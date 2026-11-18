//! Privacy-first microphone capture and 16 kHz mono resampling.
//!
//! Security & Privacy guarantees (Master Prompt Section 12):
//! - Nothing is captured until explicit user Push-to-Talk activation: one WASAPI client is kept
//!   initialized (and stopped, see `ActiveCapture::prewarm`) so the clip starts in milliseconds, and
//!   it is stopped immediately on release/cancel so the Windows 11 taskbar microphone indicator
//!   accurately reflects live recording!
//! - Resolves concrete device names rather than holding a stale default handle across hot-plugs.
//! - Stores captured PCM strictly in RAM and explicitly `zeroize`s sample memory when extracted or cancelled.

use anyhow::{bail, Context, Result};
#[cfg(windows)]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::SampleFormat;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::AtomicU32;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
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

/// One recording session. `start` hands out a client that is already initialized at startup and
/// after every clip (see `prewarm`), so a press only has to start it. Dropping or calling
/// `stop_and_take_16k()` stops WASAPI capture (turning off the Windows 11 microphone indicator),
/// returns resampled 16 kHz mono PCM and keeps the stopped client for the next press.
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
    hold_max_bits: Arc<AtomicU32>,
    #[cfg(windows)]
    resampler: Resampler,
    #[cfg(windows)]
    stream_disconnected: Arc<AtomicBool>,
    #[cfg(windows)]
    last_error: Arc<Mutex<Option<String>>>,
    #[cfg(windows)]
    device_name: String,
    #[cfg(windows)]
    used_fallback_device: bool,
    #[cfg(windows)]
    first_frame_us: Arc<AtomicU64>,
    #[cfg(windows)]
    sim_running: Option<Arc<AtomicBool>>,
    /// When `start` began this session, so `first_frame_ms` is measured from the press.
    #[cfg(windows)]
    started_us: u64,
    /// Wall time `start` spent (a warm client pays the stream start only), for the press-path log.
    #[cfg(windows)]
    start_ms: f64,
    /// This session reused the warm client instead of opening the device.
    #[cfg(windows)]
    warm: bool,
    /// The device request this client was opened for; a warm client is reused only for the same one.
    #[cfg(windows)]
    preferred: Option<String>,
    /// A real stream is running, so it is worth keeping once the clip ends.
    #[cfg(windows)]
    playing: bool,
}

std::thread_local! {
    /// The stopped client kept between clips. cpal's stream is not `Send`, so it lives on the thread
    /// that records (the coordinator); starting it replaces the device open on the press.
    #[cfg(windows)]
    static WARM: std::cell::RefCell<Option<ActiveCapture>> = const { std::cell::RefCell::new(None) };
}

#[cfg(windows)]
fn now_us() -> u64 {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(std::time::Instant::now).elapsed().as_micros() as u64
}

impl ActiveCapture {
    /// One recording session on the microphone. The client is kept warm between clips, so this is a
    /// stream start in milliseconds; the first call after startup, a device change or an error pays
    /// the device open.
    pub fn start(preferred_device: Option<&str>) -> Result<Self> {
        #[cfg(not(windows))]
        {
            let _ = preferred_device;
            let _ = Mutex::new(());
            bail!("Live WASAPI capture is enabled on Windows; use `--transcribe-wav` on Linux")
        }
        #[cfg(windows)]
        {
            let t0 = std::time::Instant::now();
            let wanted = preferred_device.filter(|s| !s.trim().is_empty());
            if let Some(mut cap) = take_warm(wanted) {
                cap.warm = true;
                match cap.begin(t0) {
                    Ok(cap) => return Ok(cap),
                    // A client whose device went away while it waited: open one now instead.
                    Err(e) => log::debug!("warm microphone did not start ({e:#}); opening the device"),
                }
            }
            Self::open(preferred_device)?.begin(t0)
        }
    }

    /// Opens a capture client now and leaves it stopped, so the next `start` only has to start it.
    /// No audio is captured while it waits. Returns false when the device cannot be opened; the
    /// press then does a normal open and reports the error itself.
    pub fn prewarm(preferred_device: Option<&str>) -> bool {
        #[cfg(windows)]
        {
            // The simulated microphone is a file, not a device: nothing to keep open.
            if std::env::var("KEYREED_TEST_AUDIO_WAV").map(|v| !v.trim().is_empty()).unwrap_or(false) {
                return false;
            }
            match Self::open(preferred_device) {
                Ok(cap) => keep_warm(cap),
                Err(e) => {
                    log::debug!("microphone prewarm failed: {e:#}");
                    false
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = preferred_device;
            false
        }
    }

    /// Starts the stream this client already owns and marks where the clip begins.
    #[cfg(windows)]
    fn begin(mut self, t0: std::time::Instant) -> Result<Self> {
        self.reset_for_clip();
        self.started_us = now_us();
        if let Some(s) = &self.stream {
            s.play().context("Failed to start microphone stream")?;
            self.playing = true;
        }
        self.start_ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok(self)
    }

    /// Clears what the previous clip left behind. `stream_disconnected` and `last_error` stay as the
    /// device reported them, so a failing device is never quietly reused as healthy.
    #[cfg(windows)]
    fn reset_for_clip(&mut self) {
        if self.sim_running.is_some() {
            return;
        }
        let mut guard = self.raw_mono.lock();
        if !guard.is_empty() {
            guard.zeroize();
            guard.clear();
        }
        drop(guard);
        self.latest_rms_bits.store(0.0f32.to_bits(), Ordering::Relaxed);
        self.peak_rms_bits.store(0.0f32.to_bits(), Ordering::Relaxed);
        self.hold_max_bits.store(0.0f32.to_bits(), Ordering::Relaxed);
        self.first_frame_us.store(0, Ordering::Relaxed);
        self.resampler = Resampler::new(self.input_sample_rate);
    }

    /// Wall time `start` spent, so the coordinator can log the press path without guessing.
    pub fn start_ms(&self) -> f64 {
        #[cfg(windows)]
        {
            self.start_ms
        }
        #[cfg(not(windows))]
        {
            0.0
        }
    }

    /// True when this session reused the client kept warm instead of opening the device.
    pub fn was_warm(&self) -> bool {
        #[cfg(windows)]
        {
            self.warm
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Builds and initializes the capture client without starting it.
    #[cfg(windows)]
    fn open(preferred_device: Option<&str>) -> Result<Self> {
        if let Ok(sim_wav) = std::env::var("KEYREED_TEST_AUDIO_WAV") {
            if !sim_wav.trim().is_empty() {
                let samples = load_wav_16k_mono(Path::new(sim_wav.trim()))?;
                let raw_mono = Arc::new(Mutex::new(Vec::with_capacity(samples.len())));
                let buf_ref = Arc::clone(&raw_mono);
                let latest_rms_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
                let rms_ref = Arc::clone(&latest_rms_bits);
                let peak_rms_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
                let peak_ref = Arc::clone(&peak_rms_bits);
                let hold_max_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
                let hold_ref = Arc::clone(&hold_max_bits);
                let first_frame_us = Arc::new(AtomicU64::new(0));
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
                        let mut dc = DcBlocker::new(16_000);
                        let step = 320usize; // 20 ms at 16 kHz
                        let tick = std::time::Duration::from_secs_f64(0.020 / speed);
                        for ch in samples.chunks(step) {
                            if !run_ref.load(Ordering::Relaxed) {
                                break;
                            }
                            push_interleaved_f32(ch, 1, &buf_ref, &rms_ref, &peak_ref, &hold_ref, &ff_ref, &mut dc);
                            std::thread::sleep(tick);
                        }
                    })?;
                return Ok(Self {
                    stream: None,
                    raw_mono,
                    input_sample_rate: 16_000,
                    latest_rms_bits,
                    peak_rms_bits,
                    hold_max_bits,
                    resampler: Resampler::new(16_000),
                    stream_disconnected: Arc::new(AtomicBool::new(false)),
                    last_error: Arc::new(Mutex::new(None)),
                    device_name: format!("SimulatedWAV:{}", sim_wav.trim()),
                    used_fallback_device: false,
                    first_frame_us,
                    sim_running: Some(sim_running),
                    started_us: now_us(),
                    start_ms: 0.0,
                    warm: false,
                    preferred: preferred_device.map(str::to_string),
                    playing: false,
                });
            }
        }

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
        let hold_max_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let hold_ref = Arc::clone(&hold_max_bits);
        let mut dc = DcBlocker::new(sample_rate);
        let stream_disconnected = Arc::new(AtomicBool::new(false));
        let disc_ref = Arc::clone(&stream_disconnected);
        let last_error = Arc::new(Mutex::new(None));
        let err_ref = Arc::clone(&last_error);
        let first_frame_us = Arc::new(AtomicU64::new(0));
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
                    push_interleaved_f32(data, channels, &buf_ref, &rms_ref, &peak_ref, &hold_ref, &ff_ref, &mut dc)
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _| {
                    let converted: Vec<f32> =
                        data.iter().map(|&x| (x as f32) / 32768.0).collect();
                    push_interleaved_f32(&converted, channels, &buf_ref, &rms_ref, &peak_ref, &hold_ref, &ff_ref, &mut dc);
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
                    push_interleaved_f32(&converted, channels, &buf_ref, &rms_ref, &peak_ref, &hold_ref, &ff_ref, &mut dc);
                },
                err_fn,
                None,
            )?,
            other => bail!("Unsupported microphone sample format: {:?}", other),
        };

        Ok(Self {
            stream: Some(stream),
            raw_mono,
            input_sample_rate: sample_rate,
            latest_rms_bits,
            peak_rms_bits,
            hold_max_bits,
            resampler: Resampler::new(sample_rate),
            stream_disconnected,
            last_error,
            device_name,
            used_fallback_device: used_fallback,
            first_frame_us,
            sim_running: None,
            started_us: 0,
            start_ms: 0.0,
            warm: false,
            preferred: preferred_device.map(str::to_string),
            playing: false,
        })
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

    /// Hold-max microphone level (DC-removed callback RMS, `0.0..=1.0`, stored as `f32` bits).
    ///
    /// Contract: the capture callback only raises it (`fetch_update` with max). The reader (the
    /// waveform UI) calls `swap(0, Relaxed)` once per frame, so each frame sees the loudest
    /// callback since the previous frame and brief peaks are never missed between frames.
    pub fn level_handle(&self) -> Arc<AtomicU32> {
        #[cfg(windows)]
        {
            Arc::clone(&self.hold_max_bits)
        }
        #[cfg(not(windows))]
        {
            Arc::new(AtomicU32::new(0))
        }
    }

    /// Milliseconds from this press to the first sample captured; 0 when nothing arrived yet.
    pub fn first_frame_ms(&self) -> f64 {
        #[cfg(windows)]
        {
            let first = self.first_frame_us.load(Ordering::Relaxed);
            if first == 0 || first < self.started_us {
                return 0.0;
            }
            (first - self.started_us) as f64 / 1000.0
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
            drain_to_16k(&mut guard, &mut self.resampler)
        }
        #[cfg(not(windows))]
        {
            let _ = self;
            Vec::new()
        }
    }

    /// Stops WASAPI capture immediately, resamples to 16 kHz mono, and zeroizes the raw buffer.
    /// `Drop` then keeps the stopped client for the next press.
    pub fn stop_and_take_16k(mut self) -> Vec<f32> {
        #[cfg(windows)]
        {
            if let Some(sim) = self.sim_running.take() {
                sim.store(false, Ordering::Relaxed);
            }
            if let Some(s) = &self.stream {
                let _ = s.pause();
            }
            let mut guard = self.raw_mono.lock();
            let mut out = drain_to_16k(&mut guard, &mut self.resampler);
            out.extend(self.resampler.finish());
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

    /// Like `stop_and_take_16k`, but first keeps capturing for the post-release tail so a word
    /// still being said as the key goes up is not clipped: 200 ms, extended up to 600 ms while
    /// sustained speech continues (see `tail_should_stop`). Only for a normal release: callers must
    /// use `cancel_and_zeroize` for Escape, key-lost and time-cap discards. The stream runs only
    /// while the key is held plus this bounded tail: between clips the client is initialized but
    /// stopped, so nothing is captured.
    /// Blocks the calling thread for at most `MAX_RELEASE_TAIL_MS`. Skipped for a stalled or
    /// disconnected device and for the simulated WAV microphone.
    pub fn stop_with_release_tail_and_take_16k(self) -> Vec<f32> {
        #[cfg(windows)]
        {
            if self.sim_running.is_none() && !self.is_disconnected() && self.first_frame_us.load(Ordering::Relaxed) != 0 {
                self.wait_release_tail();
            }
        }
        self.stop_and_take_16k()
    }

    #[cfg(windows)]
    fn wait_release_tail(&self) {
        let frame_len = (self.input_sample_rate as usize / 1000) * TAIL_FRAME_MS as usize;
        if frame_len == 0 {
            return;
        }
        let (release_len, floor) = {
            let g = self.raw_mono.lock();
            (g.len(), noise_floor(&frame_rms(&g, frame_len)))
        };
        let started = std::time::Instant::now();
        let max = std::time::Duration::from_millis(MAX_RELEASE_TAIL_MS);
        // The wall-clock bound also covers a stream that quietly stops delivering callbacks.
        while started.elapsed() < max && !self.is_disconnected() {
            std::thread::sleep(std::time::Duration::from_millis(15));
            let frames = {
                let g = self.raw_mono.lock();
                frame_rms(g.get(release_len..).unwrap_or(&[]), frame_len)
            };
            if tail_should_stop(&frames, floor) {
                break;
            }
        }
    }

    /// Cancels capture and wipes buffered audio from RAM immediately. `Drop` then keeps the stopped
    /// client for the next press.
    pub fn cancel_and_zeroize(mut self) {
        #[cfg(windows)]
        {
            if let Some(sim) = self.sim_running.take() {
                sim.store(false, Ordering::Relaxed);
            }
            if let Some(s) = &self.stream {
                let _ = s.pause();
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
        {
            let mut guard = self.raw_mono.lock();
            if !guard.is_empty() {
                guard.zeroize();
                guard.clear();
            }
        }
        let Some(stream) = self.stream.take() else { return };
        let _ = stream.pause();
        // A client that errored has a dead audio thread: reopening later is the only way back.
        if !self.playing || self.is_disconnected() {
            return;
        }
        // Keep the initialized client instead of closing it: the next press only starts it. A fresh
        // resampler and a wiped buffer make this indistinguishable from a cold open for the clip.
        keep_warm(ActiveCapture {
            stream: Some(stream),
            raw_mono: Arc::clone(&self.raw_mono),
            input_sample_rate: self.input_sample_rate,
            latest_rms_bits: Arc::clone(&self.latest_rms_bits),
            peak_rms_bits: Arc::clone(&self.peak_rms_bits),
            hold_max_bits: Arc::clone(&self.hold_max_bits),
            resampler: Resampler::new(self.input_sample_rate),
            stream_disconnected: Arc::clone(&self.stream_disconnected),
            last_error: Arc::clone(&self.last_error),
            device_name: self.device_name.clone(),
            used_fallback_device: self.used_fallback_device,
            first_frame_us: Arc::clone(&self.first_frame_us),
            sim_running: None,
            started_us: 0,
            start_ms: 0.0,
            warm: true,
            preferred: self.preferred.clone(),
            playing: false,
        });
    }
}

/// The warm client, when it was opened for the same device request and still looks healthy.
#[cfg(windows)]
fn take_warm(preferred: Option<&str>) -> Option<ActiveCapture> {
    WARM.try_with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        let reusable = slot
            .as_ref()
            .is_some_and(|c| c.preferred.as_deref() == preferred && !c.is_disconnected());
        if reusable {
            slot.take()
        } else {
            None
        }
    })
    .ok()
    .flatten()
}

/// Parks a stopped client for the next press. False when this thread already holds one or is
/// shutting down, in which case the client is simply closed.
#[cfg(windows)]
fn keep_warm(cap: ActiveCapture) -> bool {
    WARM.try_with(|slot| match slot.try_borrow_mut() {
        Ok(mut slot) if slot.is_none() => {
            *slot = Some(cap);
            true
        }
        _ => false,
    })
    .unwrap_or(false)
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn push_interleaved_f32(
    data: &[f32],
    channels: usize,
    dest: &Mutex<Vec<f32>>,
    rms_atomic: &AtomicU32,
    peak_atomic: &AtomicU32,
    hold_atomic: &AtomicU32,
    first_frame_us: &AtomicU64,
    dc: &mut DcBlocker,
) {
    if data.is_empty() || channels == 0 {
        return;
    }
    // Stamped against a process-wide clock so a warm client measures from the press, not from open.
    if first_frame_us.load(Ordering::Relaxed) == 0 {
        let _ = first_frame_us.compare_exchange(0, now_us(), Ordering::Relaxed, Ordering::Relaxed);
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
        let mono = dc.process(mono);
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
        // Non-negative f32 bits order like integers, so an integer max is a float max.
        hold_atomic.fetch_max(rms.to_bits(), Ordering::Relaxed);
    }
}

/// Single-pole DC blocker (about 20 Hz corner): `y[n] = x[n] - x[n-1] + r * y[n-1]`.
/// Seeded with the first sample so a constant offset does not produce a start-up step.
pub struct DcBlocker {
    r: f32,
    x1: f32,
    y1: f32,
    primed: bool,
}

impl DcBlocker {
    pub fn new(sample_rate: u32) -> Self {
        let r = 1.0 - (2.0 * std::f32::consts::PI * 20.0 / sample_rate.max(1) as f32);
        Self { r: r.clamp(0.0, 0.9999), x1: 0.0, y1: 0.0, primed: false }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        if !self.primed {
            self.primed = true;
            self.x1 = x;
        }
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
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

const TARGET_RATE: u32 = 16_000;
/// Low-pass corner of the resampler in Hz (just under the 8 kHz output Nyquist).
const RESAMPLE_CUTOFF_HZ: f32 = 7_600.0;
/// Sinc zero crossings kept on each side of the centre (about 32 taps per output phase at 16 kHz).
const RESAMPLE_ZERO_CROSSINGS: f32 = 16.0;

/// Stateful polyphase windowed-sinc (Blackman-Harris) resampler from any rate to 16 kHz.
///
/// The ratio is reduced with gcd (48000 -> 3:1, 44100 -> 441:160). Output sample `n` sits exactly at
/// input position `n * src / 16000` and the kernel is symmetric, so there is no group delay and speech
/// is not shifted in time. Input history is kept between `process` calls, so incremental drains
/// give bit-identical samples to a one-shot conversion; call `finish` once to flush the tail.
pub struct Resampler {
    passthrough: bool,
    up: u64,
    down: u64,
    half: usize,
    taps: usize,
    /// `up` rows of `taps` coefficients, each row normalised to unit DC gain.
    table: Vec<f32>,
    buf: Vec<f32>,
    /// Absolute input index of `buf[0]` (negative while the zero lead-in is buffered).
    buf_start: i64,
    real_in: u64,
    next_out: u64,
}

impl Resampler {
    pub fn new(src_rate: u32) -> Self {
        let src_rate = src_rate.max(1);
        let g = gcd_u32(src_rate, TARGET_RATE);
        let (down, up) = ((src_rate / g) as u64, (TARGET_RATE / g) as u64);
        let passthrough = src_rate == TARGET_RATE;
        let cutoff = RESAMPLE_CUTOFF_HZ.min(0.45 * src_rate as f32);
        let fcn = (cutoff / src_rate as f32) as f64; // cycles per input sample
        let half = if passthrough { 1 } else { (RESAMPLE_ZERO_CROSSINGS as f64 / (2.0 * fcn)).ceil() as usize };
        let taps = 2 * half;
        let mut table = Vec::new();
        if !passthrough {
            table.reserve(up as usize * taps);
            for phase in 0..up {
                let frac = phase as f64 / up as f64;
                let start = table.len();
                for k in 0..taps {
                    let t = (k as f64 - half as f64 + 1.0) - frac;
                    let x = 2.0 * fcn * t;
                    let sinc = if x.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
                    let w = t / half as f64;
                    let pi = std::f64::consts::PI;
                    let win = 0.35875 + 0.48829 * (pi * w).cos() + 0.14128 * (2.0 * pi * w).cos() + 0.01168 * (3.0 * pi * w).cos();
                    table.push((sinc * win) as f32);
                }
                let sum: f32 = table[start..].iter().sum();
                if sum.abs() > 1e-9 {
                    table[start..].iter_mut().for_each(|c| *c /= sum);
                }
            }
        }
        Self {
            passthrough,
            up,
            down,
            half,
            taps,
            table,
            buf: vec![0.0; half.saturating_sub(1)],
            buf_start: -(half as i64 - 1),
            real_in: 0,
            next_out: 0,
        }
    }

    /// Feeds more input and returns every output sample that is now fully determined.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.passthrough {
            return input.to_vec();
        }
        self.real_in += input.len() as u64;
        self.buf.extend_from_slice(input);
        self.emit(u64::MAX)
    }

    /// Flushes the tail with zero padding and returns the remaining samples; the total output is
    /// exactly `floor(input_len * 16000 / src_rate)`. The resampler restarts clean afterwards.
    pub fn finish(&mut self) -> Vec<f32> {
        if self.passthrough {
            return Vec::new();
        }
        let target = self.real_in * self.up / self.down;
        self.buf.resize(self.buf.len() + self.half + 1, 0.0);
        let out = self.emit(target);
        self.buf.zeroize();
        self.buf.clear();
        self.buf.resize(self.half.saturating_sub(1), 0.0);
        self.buf_start = -(self.half as i64 - 1);
        self.real_in = 0;
        self.next_out = 0;
        out
    }

    fn emit(&mut self, limit: u64) -> Vec<f32> {
        let mut out = Vec::new();
        let end = self.buf_start + self.buf.len() as i64;
        while self.next_out < limit {
            let pos = self.next_out * self.down;
            let base = (pos / self.up) as i64;
            let phase = (pos % self.up) as usize;
            if base + self.half as i64 >= end {
                break;
            }
            let off = (base - self.half as i64 + 1 - self.buf_start) as usize;
            let row = &self.table[phase * self.taps..(phase + 1) * self.taps];
            out.push(row.iter().zip(&self.buf[off..off + self.taps]).map(|(c, x)| c * x).sum());
            self.next_out += 1;
        }
        // Drop input no later output can reach, wiping the raw audio.
        let keep_from = ((self.next_out * self.down / self.up) as i64 - self.half as i64 + 1).max(self.buf_start);
        let n = ((keep_from - self.buf_start) as usize).min(self.buf.len());
        self.buf[..n].zeroize();
        self.buf.drain(..n);
        self.buf_start += n as i64;
        out
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        self.buf.zeroize();
    }
}

/// Moves all buffered raw samples through the stateful resampler and wipes them.
fn drain_to_16k(buf: &mut Vec<f32>, rs: &mut Resampler) -> Vec<f32> {
    let out = rs.process(buf);
    buf.zeroize();
    buf.clear();
    out
}

/// One-shot band-limited resample from arbitrary `src_rate` to 16 kHz mono.
pub fn resample_to_16k_mono(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if samples.is_empty() || src_rate == 0 {
        return Vec::new();
    }
    let mut rs = Resampler::new(src_rate);
    let mut out = rs.process(samples);
    out.extend(rs.finish());
    out
}

/// Release tail constants (Hex values): always keep 200 ms after release, extend up to 600 ms while
/// sustained speech continues, and stop once 8 frames (160 ms) in a row are quiet.
pub const POST_RELEASE_TAIL_MS: u64 = 200;
pub const MAX_RELEASE_TAIL_MS: u64 = 600;
pub const TAIL_FRAME_MS: u64 = 20;
const TAIL_SPEECH_FRAMES: usize = 5;
const TAIL_SPEECH_GAP_FRAMES: usize = 2;
const TAIL_QUIET_FRAMES: usize = 8;
/// A tail frame counts as speech at +12 dB over the clip noise floor and above about -66 dBFS.
const TAIL_SPEECH_OVER_FLOOR: f32 = 3.98;
const TAIL_MIN_SPEECH_RMS: f32 = 0.0005;

/// RMS of each full `frame_len`-sample frame (a trailing partial frame is ignored).
fn frame_rms(samples: &[f32], frame_len: usize) -> Vec<f32> {
    samples
        .chunks_exact(frame_len.max(1))
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

/// 10th percentile of frame levels, a robust estimate of the room noise floor.
fn noise_floor(frames: &[f32]) -> f32 {
    let mut v = frames.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 10).copied().unwrap_or(0.0)
}

/// Pure tail decision. `frame_rms` holds the 20 ms frame levels captured since the key release and
/// `floor_rms` the clip noise floor. Returns true when capture may stop: never before 200 ms; at
/// 200 ms stop unless speech was running (5 speech frames within the last frames, gaps up to 2);
/// while speech runs on, stop after 8 quiet frames in a row; always stop at 600 ms.
pub fn tail_should_stop(frame_rms: &[f32], floor_rms: f32) -> bool {
    let base = (POST_RELEASE_TAIL_MS / TAIL_FRAME_MS) as usize;
    let max = (MAX_RELEASE_TAIL_MS / TAIL_FRAME_MS) as usize;
    let n = frame_rms.len();
    if n >= max {
        return true;
    }
    if n < base {
        return false;
    }
    let is_speech = |r: f32| r >= TAIL_MIN_SPEECH_RMS && r >= floor_rms * TAIL_SPEECH_OVER_FLOOR;
    let (mut speech, mut gap, mut talking) = (0, 0, false);
    for &r in frame_rms[..base].iter().rev() {
        if is_speech(r) {
            speech += 1;
            gap = 0;
            if speech >= TAIL_SPEECH_FRAMES {
                talking = true;
                break;
            }
        } else {
            gap += 1;
            if gap > TAIL_SPEECH_GAP_FRAMES {
                break;
            }
        }
    }
    if !talking {
        return true;
    }
    let mut quiet = 0;
    for &r in &frame_rms[base..] {
        if is_speech(r) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= TAIL_QUIET_FRAMES {
                return true;
            }
        }
    }
    false
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

    fn incremental_matches_oneshot(rate: u32, len: usize, chunk: usize) {
        let full: Vec<f32> = (0..len).map(|i| ((i as f32) * 0.007).cos()).collect();
        let expected = resample_to_16k_mono(&full, rate);
        assert_eq!(expected.len(), len * 16_000 / rate as usize);
        let mut rs = Resampler::new(rate);
        let mut buf = Vec::new();
        let mut got = Vec::new();
        for ch in full.chunks(chunk) {
            buf.extend_from_slice(ch);
            got.extend(drain_to_16k(&mut buf, &mut rs));
        }
        got.extend(rs.finish());
        assert_eq!(got, expected);
    }

    #[test]
    fn incremental_drain_matches_oneshot_48k() {
        incremental_matches_oneshot(48_000, 48_005, 1_111);
    }

    #[test]
    fn incremental_drain_matches_oneshot_44k() {
        incremental_matches_oneshot(44_100, 44_103, 1_201);
    }

    #[test]
    fn resampler_keeps_alignment_and_rejects_aliasing() {
        // A 1 kHz tone must stay in phase (no group delay shift) and a 12 kHz tone must be removed.
        let tone = |f: f32, rate: f32, n: usize| -> Vec<f32> {
            (0..n).map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / rate).sin()).collect()
        };
        for rate in [48_000u32, 44_100] {
            let out = resample_to_16k_mono(&tone(1_000.0, rate as f32, rate as usize), rate);
            for i in 2_000..2_100 {
                let want = (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / 16_000.0).sin();
                assert!((out[i] - want).abs() < 0.02, "rate {rate} i {i}");
            }
            let hi = resample_to_16k_mono(&tone(12_000.0, rate as f32, rate as usize), rate);
            let peak = hi[2_000..14_000].iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak < 0.05, "rate {rate} aliased peak {peak}");
        }
    }

    #[test]
    fn dc_blocker_removes_offset() {
        let mut dc = DcBlocker::new(48_000);
        let mut last = 1.0;
        for _ in 0..48_000 {
            last = dc.process(0.3);
        }
        assert!(last.abs() < 1e-3);
    }

    #[test]
    fn tail_policy_stops_at_200ms_without_speech() {
        let quiet = vec![0.001f32; 10];
        assert!(!tail_should_stop(&quiet[..9], 0.001));
        assert!(tail_should_stop(&quiet, 0.001));
    }

    #[test]
    fn tail_policy_extends_while_speaking_then_stops() {
        let mut f = vec![0.1f32; 10];
        assert!(!tail_should_stop(&f, 0.001));
        f.extend(vec![0.1f32; 8]);
        assert!(!tail_should_stop(&f, 0.001));
        f.extend(vec![0.001f32; 7]);
        assert!(!tail_should_stop(&f, 0.001));
        f.push(0.001);
        assert!(tail_should_stop(&f, 0.001));
        // Continuous speech is capped at 600 ms.
        assert!(!tail_should_stop(&vec![0.1f32; 29], 0.001));
        assert!(tail_should_stop(&vec![0.1f32; 30], 0.001));
    }
}
