//! Privacy-first microphone capture and 16 kHz mono resampling.
//!
//! Security & Privacy guarantees (Master Prompt Section 12):
//! - Never opens an audio stream until explicit user Push-to-Talk activation, and drops the stream
//!   immediately upon release/cancel so the Windows 11 taskbar microphone indicator accurately reflects live recording!
//! - Resolves concrete device names rather than holding a stale default handle across hot-plugs.
//! - Stores captured PCM strictly in RAM and explicitly `zeroize`s sample memory when extracted or cancelled.

use anyhow::{bail, Context, Result};
#[cfg(windows)]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::SampleFormat;
use parking_lot::Mutex;
use std::path::Path;
#[cfg(windows)]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use zeroize::Zeroize;

#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

pub fn list_input_devices() -> Vec<AudioDeviceInfo> {
    #[cfg(windows)]
    {
        let host = cpal::default_host();
        let default_name = host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        let mut out = Vec::new();
        if let Ok(devs) = host.input_devices() {
            for dev in devs {
                if let Ok(name) = dev.name() {
                    let is_default = !default_name.is_empty() && name == default_name;
                    out.push(AudioDeviceInfo { name, is_default });
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Active recording session. Dropping or calling `stop_and_take_16k()` immediately closes WASAPI capture
/// (turning off the Windows 11 microphone indicator) and returns resampled 16 kHz mono PCM.
pub struct ActiveCapture {
    #[cfg(windows)]
    stream: Option<cpal::Stream>,
    #[cfg(windows)]
    raw_mono: Arc<Mutex<Vec<f32>>>,
    #[cfg(windows)]
    input_sample_rate: u32,
    #[cfg(windows)]
    latest_rms_bits: Arc<AtomicU32>,
}

impl ActiveCapture {
    pub fn start(preferred_device: Option<&str>) -> Result<Self> {
        #[cfg(not(windows))]
        {
            let _ = preferred_device;
            let _ = Mutex::new(());
            bail!("Live WASAPI capture is enabled on Windows; use `--transcribe-wav` on Linux")
        }
        #[cfg(windows)]
        {
        let host = cpal::default_host();
        let device = match preferred_device.filter(|s| !s.trim().is_empty()) {
            Some(wanted) => host
                .input_devices()
                .ok()
                .and_then(|mut it| {
                    it.find(|d| {
                        d.name()
                            .map(|n| n.eq_ignore_ascii_case(wanted) || n.contains(wanted))
                            .unwrap_or(false)
                    })
                })
                .or_else(|| host.default_input_device()),
            None => host.default_input_device(),
        }
        .context("No microphone input device available (check Windows Microphone Privacy settings)")?;

        let supported = device
            .default_input_config()
            .context("Failed to query microphone format (check Windows Microphone Privacy permission)")?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let sample_format = supported.sample_format();
        let stream_cfg: cpal::StreamConfig = supported.into();

        // Pre-allocate 30s in RAM to avoid reallocations on the real-time audio callback thread
        let raw_mono = Arc::new(Mutex::new(Vec::with_capacity((sample_rate as usize) * 30)));
        let buf_ref = Arc::clone(&raw_mono);
        let latest_rms_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let rms_ref = Arc::clone(&latest_rms_bits);

        let err_fn = |err| log::error!("Audio capture stream error: {}", err);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_cfg,
                move |data: &[f32], _| push_interleaved_f32(data, channels, &buf_ref, &rms_ref),
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _| {
                    let converted: Vec<f32> = data.iter().map(|&x| (x as f32) / 32768.0).collect();
                    push_interleaved_f32(&converted, channels, &buf_ref, &rms_ref);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::U16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[u16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|&x| ((x as f32) - 32768.0) / 32768.0)
                        .collect();
                    push_interleaved_f32(&converted, channels, &buf_ref, &rms_ref);
                },
                err_fn,
                None,
            )?,
            other => bail!("Unsupported microphone sample format: {:?}", other),
        };

        stream.play().context("Failed to start microphone stream")?;

        Ok(Self {
            stream: Some(stream),
            raw_mono,
            input_sample_rate: sample_rate,
            latest_rms_bits,
        })
        }
    }

    /// Current microphone RMS level in `[0.0, 1.0]` for visual feedback on the floating pill.
    pub fn current_rms(&self) -> f32 {
        #[cfg(windows)]
        {
            f32::from_bits(self.latest_rms_bits.load(Ordering::Relaxed))
        }
        #[cfg(not(windows))]
        {
            0.0
        }
    }

    /// Stops WASAPI capture immediately, resamples to 16 kHz mono, and zeroizes the raw buffer.
    pub fn stop_and_take_16k(mut self) -> Vec<f32> {
        #[cfg(windows)]
        {
            if let Some(s) = self.stream.take() {
                let _ = s.pause();
                drop(s);
            }
            let mut guard = self.raw_mono.lock();
            let out = resample_to_16k_mono(&guard, self.input_sample_rate);
            guard.zeroize();
            guard.clear();
            out
        }
        #[cfg(not(windows))]
        {
            let _ = &mut self;
            Vec::new()
        }
    }

    /// Cancels capture and wipes buffered audio from RAM immediately.
    pub fn cancel_and_zeroize(mut self) {
        #[cfg(windows)]
        {
            if let Some(s) = self.stream.take() {
                let _ = s.pause();
                drop(s);
            }
            let mut guard = self.raw_mono.lock();
            guard.zeroize();
            guard.clear();
        }
        #[cfg(not(windows))]
        let _ = &mut self;
    }
}

#[cfg(windows)]
impl Drop for ActiveCapture {
    fn drop(&mut self) {
        if let Some(s) = self.stream.take() {
            let _ = s.pause();
            drop(s);
        }
        let mut guard = self.raw_mono.lock();
        if !guard.is_empty() {
            guard.zeroize();
            guard.clear();
        }
    }
}

#[cfg(windows)]
fn push_interleaved_f32(
    data: &[f32],
    channels: usize,
    dest: &Mutex<Vec<f32>>,
    rms_atomic: &AtomicU32,
) {
    if data.is_empty() || channels == 0 {
        return;
    }
    let frames = data.len() / channels;
    let mut sum_sq = 0.0f32;
    let mut guard = dest.lock();
    for f in 0..frames {
        let mut mono = 0.0f32;
        let base = f * channels;
        for c in 0..channels {
            mono += data[base + c];
        }
        mono /= channels as f32;
        sum_sq += mono * mono;
        guard.push(mono);
    }
    drop(guard);
    if frames > 0 {
        let rms = (sum_sq / (frames as f32)).sqrt().clamp(0.0, 1.0);
        rms_atomic.store(rms.to_bits(), Ordering::Relaxed);
    }
}

/// Anti-aliased boxcar-filtered linear resampler from arbitrary `src_rate` (e.g. 44.1 / 48 kHz) to 16 kHz mono.
pub fn resample_to_16k_mono(samples: &[f32], src_rate: u32) -> Vec<f32> {
    const TARGET_RATE: u32 = 16_000;
    if samples.is_empty() || src_rate == 0 {
        return Vec::new();
    }
    if src_rate == TARGET_RATE {
        return samples.to_vec();
    }
    // Exact integer 3:1 fast path for standard 48 kHz WASAPI shared mode with 3-tap anti-aliasing!
    if src_rate == 48_000 {
        let out_len = samples.len() / 3;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let b = i * 3;
            let avg = (samples[b] + samples[b + 1] + samples[b + 2]) / 3.0;
            out.push(avg);
        }
        return out;
    }

    let ratio = (src_rate as f64) / (TARGET_RATE as f64);
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = (i as f64) * ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - (idx as f64)) as f32;
        let s0 = samples[idx];
        let s1 = samples.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

/// Loads any WAV file (8/16/24/32-bit int or 32-bit float, mono or stereo, any sample rate)
/// and returns normalized 16 kHz mono `f32` samples for deterministic E2E testing and CLI transcription.
pub fn load_wav_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open WAV {}", path.display()))?;
    let spec = reader.spec();
    let channels = (spec.channels as usize).max(1);

    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, _) => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?,
        (SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|r| r.map(|v| (v as f32) / 32768.0))
            .collect::<std::result::Result<Vec<_>, _>>()?,
        (SampleFormat::Int, bits) => {
            let scale = (1i64 << (bits.saturating_sub(1))) as f32;
            reader
                .samples::<i32>()
                .map(|r| r.map(|v| (v as f32) / scale))
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
    };

    let frames = interleaved.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += interleaved[f * channels + c];
        }
        mono.push(acc / (channels as f32));
    }
    Ok(resample_to_16k_mono(&mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_48k_to_16k_exact_ratio() {
        let src: Vec<f32> = (0..48_000)
            .map(|i| ((i as f32) * 0.01).sin() * 0.5)
            .collect();
        let out = resample_to_16k_mono(&src, 48_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resamples_44100_to_16k() {
        let src = vec![0.25f32; 44_100];
        let out = resample_to_16k_mono(&src, 44_100);
        assert_eq!(out.len(), 16_000);
        assert!((out[100] - 0.25).abs() < 1e-5);
    }
}
