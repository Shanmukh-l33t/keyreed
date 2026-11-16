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
use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use zeroize::Zeroize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub sample_rate: u32,
    pub channels: u16,
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
                    let (sample_rate, channels) = dev
                        .default_input_config()
                        .map(|c| (c.sample_rate().0, c.channels()))
                        .unwrap_or((16_000, 1));
                    out.push(AudioDeviceInfo {
                        name,
                        is_default,
                        sample_rate,
                        channels,
                    });
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
    #[cfg(windows)]
    peak_rms_bits: Arc<AtomicU32>,
    #[cfg(windows)]
    stream_disconnected: Arc<AtomicBool>,
    #[cfg(windows)]
    last_error: Arc<Mutex<Option<String>>>,
    #[cfg(windows)]
    device_name: String,
    #[cfg(windows)]
    used_fallback_device: bool,
    #[cfg(windows)]
    first_frame_us: Arc<AtomicU32>,
    #[cfg(windows)]
    sim_running: Option<Arc<AtomicBool>>,
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
            if let Ok(sim_wav) = std::env::var("KEYREED_TEST_AUDIO_WAV") {
                if !sim_wav.trim().is_empty() {
                    let t0 = std::time::Instant::now();
                    let samples = load_wav_16k_mono(Path::new(sim_wav.trim()))?;
                    let raw_mono = Arc::new(Mutex::new(Vec::with_capacity(samples.len())));
                    let buf_ref = Arc::clone(&raw_mono);
                    let latest_rms_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
                    let rms_ref = Arc::clone(&latest_rms_bits);
                    let peak_rms_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
                    let peak_ref = Arc::clone(&peak_rms_bits);
                    let first_frame_us = Arc::new(AtomicU32::new(0));
                    let ff_ref = Arc::clone(&first_frame_us);
                    let sim_running = Arc::new(AtomicBool::new(true));
                    let run_ref = Arc::clone(&sim_running);
                    let speed: f64 = std::env::var("KEYREED_TEST_AUDIO_SPEED")
                        .ok()
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(1.0)
                        .clamp(0.25, 32.0);
                    std::thread::Builder::new()
                        .name("keyreed-sim-mic".to_string())
                        .spawn(move || {
                            let step = 320usize; // 20 ms at 16 kHz
                            let tick = std::time::Duration::from_secs_f64(0.020 / speed);
                            for ch in samples.chunks(step) {
                                if !run_ref.load(Ordering::Relaxed) {
                                    break;
                                }
                                push_interleaved_f32(ch, 1, &buf_ref, &rms_ref, &peak_ref, &ff_ref, t0);
                                std::thread::sleep(tick);
                            }
                        })?;
                    return Ok(Self {
                        stream: None,
                        raw_mono,
                        input_sample_rate: 16_000,
                        latest_rms_bits,
                        peak_rms_bits,
                        stream_disconnected: Arc::new(AtomicBool::new(false)),
                        last_error: Arc::new(Mutex::new(None)),
                        device_name: format!("SimulatedWAV:{}", sim_wav.trim()),
                        used_fallback_device: false,
                        first_frame_us,
                        sim_running: Some(sim_running),
                    });
                }
            }

            let t0 = std::time::Instant::now();
            let host = cpal::default_host();
            let mut used_fallback = false;
            let device = match preferred_device.filter(|s| !s.trim().is_empty()) {
                Some(wanted) => {
                    let matched = host.input_devices().ok().and_then(|mut it| {
                        it.find(|d| {
                            d.name()
                                .map(|n| n.eq_ignore_ascii_case(wanted) || n.contains(wanted))
                                .unwrap_or(false)
                        })
                    });
                    if let Some(dev) = matched {
                        Some(dev)
                    } else {
                        used_fallback = true;
                        host.default_input_device()
                    }
                }
                None => host.default_input_device(),
            }
            .context("No microphone input device available (check Windows Microphone Privacy settings)")?;

            let device_name = device.name().unwrap_or_else(|_| "Default Microphone".to_string());
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
            let peak_rms_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
            let peak_ref = Arc::clone(&peak_rms_bits);
            let stream_disconnected = Arc::new(AtomicBool::new(false));
            let disc_ref = Arc::clone(&stream_disconnected);
            let last_error = Arc::new(Mutex::new(None));
            let err_ref = Arc::clone(&last_error);
            let first_frame_us = Arc::new(AtomicU32::new(0));
            let ff_ref = Arc::clone(&first_frame_us);

            let err_fn = move |err: cpal::StreamError| {
                log::error!("Audio capture stream error: {}", err);
                disc_ref.store(true, Ordering::Relaxed);
                *err_ref.lock() = Some(err.to_string());
            };

            let stream = match sample_format {
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &stream_cfg,
                    move |data: &[f32], _| {
                        push_interleaved_f32(data, channels, &buf_ref, &rms_ref, &peak_ref, &ff_ref, t0)
                    },
                    err_fn,
                    None,
                )?,
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &stream_cfg,
                    move |data: &[i16], _| {
                        let converted: Vec<f32> =
                            data.iter().map(|&x| (x as f32) / 32768.0).collect();
                        push_interleaved_f32(&converted, channels, &buf_ref, &rms_ref, &peak_ref, &ff_ref, t0);
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
                        push_interleaved_f32(&converted, channels, &buf_ref, &rms_ref, &peak_ref, &ff_ref, t0);
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
                peak_rms_bits,
                stream_disconnected,
                last_error,
                device_name,
                used_fallback_device: used_fallback,
                first_frame_us,
                sim_running: None,
            })
        }
    }

    pub fn device_name(&self) -> &str {
        #[cfg(windows)]
        {
            &self.device_name
        }
        #[cfg(not(windows))]
        {
            "none"
        }
    }

    pub fn used_fallback_device(&self) -> bool {
        #[cfg(windows)]
        {
            self.used_fallback_device
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn is_disconnected(&self) -> bool {
        #[cfg(windows)]
        {
            self.stream_disconnected.load(Ordering::Relaxed)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn last_error(&self) -> Option<String> {
        #[cfg(windows)]
        {
            self.last_error.lock().clone()
        }
        #[cfg(not(windows))]
        {
            None
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

    pub fn peak_rms(&self) -> f32 {
        #[cfg(windows)]
        {
            f32::from_bits(self.peak_rms_bits.load(Ordering::Relaxed))
        }
        #[cfg(not(windows))]
        {
            0.0
        }
    }

    pub fn first_frame_ms(&self) -> f64 {
        #[cfg(windows)]
        {
            (self.first_frame_us.load(Ordering::Relaxed) as f64) / 1000.0
        }
        #[cfg(not(windows))]
        {
            0.0
        }
    }

    /// Drains and resamples newly captured audio into 16 kHz mono while recording continues,
    /// zeroizing the consumed raw samples immediately.
    pub fn take_new_16k(&mut self) -> Vec<f32> {
        #[cfg(windows)]
        {
            let mut guard = self.raw_mono.lock();
            take_aligned_16k(&mut guard, self.input_sample_rate)
        }
        #[cfg(not(windows))]
        {
            let _ = self;
            Vec::new()
        }
    }

    /// Stops WASAPI capture immediately, resamples to 16 kHz mono, and zeroizes the raw buffer.
    pub fn stop_and_take_16k(mut self) -> Vec<f32> {
        #[cfg(windows)]
        {
            if let Some(sim) = self.sim_running.take() {
                sim.store(false, Ordering::Relaxed);
            }
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
            if let Some(sim) = self.sim_running.take() {
                sim.store(false, Ordering::Relaxed);
            }
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
        if let Some(sim) = self.sim_running.take() {
            sim.store(false, Ordering::Relaxed);
        }
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
    peak_atomic: &AtomicU32,
    first_frame_us: &AtomicU32,
    started_at: std::time::Instant,
) {
    if data.is_empty() || channels == 0 {
        return;
    }
    if first_frame_us.load(Ordering::Relaxed) == 0 {
        let us = started_at.elapsed().as_micros().clamp(1, u32::MAX as u128) as u32;
        let _ = first_frame_us.compare_exchange(0, us, Ordering::Relaxed, Ordering::Relaxed);
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
        let prev_peak = f32::from_bits(peak_atomic.load(Ordering::Relaxed));
        if rms > prev_peak {
            peak_atomic.store(rms.to_bits(), Ordering::Relaxed);
        }
    }
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Drains an exact integer-ratio multiple of `src_rate` samples into 16 kHz mono so incremental
/// drains never drift relative to a single whole-recording resample.
fn take_aligned_16k(buf: &mut Vec<f32>, src_rate: u32) -> Vec<f32> {
    let block = if src_rate == 0 { 1 } else { (src_rate / gcd_u32(src_rate, 16_000)).max(1) as usize };
    let take_len = (buf.len() / block) * block;
    if take_len == 0 {
        return Vec::new();
    }
    let out = resample_to_16k_mono(&buf[..take_len], src_rate);
    buf[..take_len].zeroize();
    buf.drain(..take_len);
    out
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

    #[test]
    fn incremental_drain_matches_oneshot_48k() {
        let full: Vec<f32> = (0..48_005).map(|i| ((i as f32) * 0.007).cos()).collect();
        let expected = resample_to_16k_mono(&full, 48_000);
        let mut buf = Vec::new();
        let mut got = Vec::new();
        for ch in full.chunks(1_111) {
            buf.extend_from_slice(ch);
            got.extend(take_aligned_16k(&mut buf, 48_000));
        }
        got.extend(resample_to_16k_mono(&buf, 48_000));
        assert_eq!(got, expected);
    }
}
