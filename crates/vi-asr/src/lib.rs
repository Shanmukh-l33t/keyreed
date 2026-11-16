pub mod catalog;
pub mod cloud;
pub mod ffi;
pub mod integrity;

use anyhow::{bail, Context, Result};
use catalog::{find_model_spec, AsrArchitecture, AsrModelSpec};
use ffi::{
    SherpaApi, SherpaOnnxOfflineRecognizer, SherpaOnnxOfflineRecognizerConfig,
    SherpaOnnxVadModelConfig, SherpaOnnxVoiceActivityDetector,
};
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::fs;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct TranscriptionOutput {
    pub text: String,
    pub detected_lang: Option<String>,
    pub vad_ms: f64,
    pub decode_ms: f64,
    pub speech_ratio: f32,
}

#[derive(Deserialize)]
struct SherpaResultJson {
    #[serde(default)]
    text: String,
    #[serde(default)]
    lang: String,
}

/// Thread-safe wrapper around `SherpaOnnxVoiceActivityDetector` (`silero_vad.onnx`).
pub struct SileroVad {
    api: Arc<SherpaApi>,
    ptr: *const SherpaOnnxVoiceActivityDetector,
    _model_c: CString,
    _provider_c: CString,
}

unsafe impl Send for SileroVad {}

impl SileroVad {
    pub fn new(api: Arc<SherpaApi>, silero_onnx_path: &Path) -> Result<Self> {
        if !silero_onnx_path.exists() {
            bail!("Silero VAD model missing at {}", silero_onnx_path.display());
        }
        let model_c = CString::new(silero_onnx_path.to_string_lossy().as_bytes())?;
        let provider_c = CString::new("cpu")?;

        let mut cfg: SherpaOnnxVadModelConfig = unsafe { MaybeUninit::zeroed().assume_init() };
        cfg.silero_vad.model = model_c.as_ptr();
        cfg.silero_vad.threshold = 0.35;
        cfg.silero_vad.min_silence_duration = 0.45;
        cfg.silero_vad.min_speech_duration = 0.18;
        cfg.silero_vad.window_size = 512;
        cfg.silero_vad.max_speech_duration = 22.0;
        cfg.sample_rate = 16_000;
        cfg.num_threads = 1;
        cfg.provider = provider_c.as_ptr();
        cfg.debug = 0;

        let ptr = unsafe { (api.create_vad)(&cfg, 120.0) };
        if ptr.is_null() {
            bail!("SherpaOnnxCreateVoiceActivityDetector returned NULL");
        }
        Ok(Self {
            api,
            ptr,
            _model_c: model_c,
            _provider_c: provider_c,
        })
    }

    /// Segments 16 kHz mono audio into active speech chunks (max ~22s per chunk).
    /// Eliminates silence/fan/keyboard noise before ASR to prevent hallucinations and speed up CPU decode.
    ///
    /// Each detected segment is widened with real audio (300 ms before, 250 ms after) and overlapping
    /// spans are merged: Silero often fires a little late on soft onsets ("um, I checked…" said quietly),
    /// which dropped the first word(s) in live microphone tests.
    pub fn extract_speech_segments(&mut self, samples_16k: &[f32]) -> Vec<Vec<f32>> {
        if samples_16k.is_empty() {
            return Vec::new();
        }
        let mut spans: Vec<(usize, usize)> = Vec::new();
        unsafe {
            (self.api.vad_reset)(self.ptr);
            let window = 512usize;
            let mut offset = 0usize;
            while offset < samples_16k.len() {
                let n = (samples_16k.len() - offset).min(window);
                (self.api.vad_accept_waveform)(
                    self.ptr,
                    samples_16k[offset..].as_ptr(),
                    n as i32,
                );
                offset += n;
            }
            (self.api.vad_flush)(self.ptr);

            while (self.api.vad_empty)(self.ptr) == 0 {
                let seg_ptr = (self.api.vad_front)(self.ptr);
                if !seg_ptr.is_null() {
                    let seg = &*seg_ptr;
                    if seg.n > 0 && !seg.samples.is_null() {
                        // The C struct's `start` is int32_t; only the low 32 bits are defined.
                        let start = (seg.start as u64 & 0xFFFF_FFFF) as u32 as usize;
                        spans.push((start, start + seg.n as usize));
                    }
                    (self.api.destroy_speech_segment)(seg_ptr);
                }
                (self.api.vad_pop)(self.ptr);
            }
        }
        pad_and_merge_spans(&spans, samples_16k.len(), 4_800, 4_000)
            .into_iter()
            .map(|(a, b)| samples_16k[a..b].to_vec())
            .collect()
    }
}

/// Widens speech spans by `pre`/`post` samples (clamped to the buffer) and merges overlaps.
fn pad_and_merge_spans(spans: &[(usize, usize)], len: usize, pre: usize, post: usize) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = spans
        .iter()
        .filter(|(a, b)| a < b && *a < len)
        .map(|&(a, b)| (a.saturating_sub(pre), (b + post).min(len)))
        .collect();
    v.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

#[cfg(test)]
mod span_tests {
    use super::pad_and_merge_spans;

    #[test]
    fn pads_clamps_and_merges() {
        // two segments 0.4 s apart merge after padding; padding never leaves the buffer
        let s = pad_and_merge_spans(&[(2_000, 10_000), (16_400, 30_000)], 32_000, 4_800, 4_000);
        assert_eq!(s, vec![(0, 32_000)]);
        // far-apart segments stay separate
        let s = pad_and_merge_spans(&[(10_000, 20_000), (60_000, 70_000)], 100_000, 4_800, 4_000);
        assert_eq!(s, vec![(5_200, 24_000), (55_200, 74_000)]);
        // garbage start beyond the buffer is ignored
        assert!(pad_and_merge_spans(&[(200_000, 200_100)], 1_000, 10, 10).is_empty());
    }
}

impl Drop for SileroVad {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.api.destroy_vad)(self.ptr) };
        }
    }
}

/// Local CPU/ONNX offline speech recognizer supporting Parakeet TDT v2/v3, Moonshine, SenseVoice, and Whisper.
pub struct LocalAsrEngine {
    api: Arc<SherpaApi>,
    recognizer: *const SherpaOnnxOfflineRecognizer,
    vad: Option<SileroVad>,
    spec: &'static AsrModelSpec,
    _owned_cstrings: Vec<CString>,
}

unsafe impl Send for LocalAsrEngine {}

fn pin_file(dir: &Path, model_id: &str, pred: impl Fn(&str) -> bool) -> Result<PathBuf> {
    let pins = crate::integrity::pins_for(model_id)
        .with_context(|| format!("no compile-time pins for '{model_id}'"))?;
    let pin = pins
        .iter()
        .find(|p| pred(p.relative))
        .context("required file is not in the compile-time pin table")?;
    Ok(dir.join(pin.relative))
}

#[allow(dead_code)]
fn find_first_existing(dir: &Path, candidates: &[&str]) -> Result<PathBuf> {
    for name in candidates {
        let p = dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    // Fallback glob scan for dynamic prefixes (e.g. tiny.en-encoder.int8.onnx)
    if let Ok(entries) = fs::read_dir(dir) {
        let files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        for pat in candidates {
            if let Some(found) = files.iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.ends_with(pat))
                    .unwrap_or(false)
            }) {
                return Ok(found.clone());
            }
        }
    }
    bail!(
        "none of {:?} found inside model directory {}",
        candidates,
        dir.display()
    )
}

impl LocalAsrEngine {
    pub fn load(
        sherpa_lib_dir: &Path,
        model_dir: &Path,
        model_id: &str,
        silero_vad_path: Option<&Path>,
        num_threads: usize,
        language: &str,
    ) -> Result<Self> {
        let api = Arc::new(
            SherpaApi::load_from_dir(sherpa_lib_dir)
                .with_context(|| format!("failed to load sherpa-onnx from {}", sherpa_lib_dir.display()))?,
        );
        let spec = find_model_spec(model_id)
            .or_else(|| {
                model_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .and_then(find_model_spec)
            })
            .with_context(|| format!("unknown ASR model id '{}'", model_id))?;

        // Hash required files against compile-time pins before any ONNX parse.
        crate::integrity::verify_model_dir(model_dir, spec.id)?;

        let mut cstrings: Vec<CString> = Vec::with_capacity(12);
        let mut push_c = |s: &str| -> Result<*const std::ffi::c_char> {
            let cs = CString::new(s)?;
            let ptr = cs.as_ptr();
            cstrings.push(cs);
            Ok(ptr)
        };

        let mut cfg: SherpaOnnxOfflineRecognizerConfig =
            unsafe { MaybeUninit::zeroed().assume_init() };
        cfg.feat_config.sample_rate = 16_000;
        cfg.feat_config.feature_dim = 80;
        cfg.model_config.num_threads = num_threads.clamp(1, 16) as i32;
        cfg.model_config.provider = push_c("cpu")?;
        cfg.model_config.debug = 0;
        cfg.decoding_method = push_c("greedy_search")?;

        let tokens_path = pin_file(model_dir, spec.id, |n| n == "tokens.txt" || n.ends_with("-tokens.txt"))?;
        cfg.model_config.tokens = push_c(&tokens_path.to_string_lossy())?;

        match spec.architecture {
            AsrArchitecture::NemoTransducer => {
                let enc = pin_file(model_dir, spec.id, |n| n == "encoder.int8.onnx")?;
                let dec = pin_file(model_dir, spec.id, |n| n == "decoder.int8.onnx")?;
                let join = pin_file(model_dir, spec.id, |n| n == "joiner.int8.onnx")?;
                cfg.model_config.transducer.encoder = push_c(&enc.to_string_lossy())?;
                cfg.model_config.transducer.decoder = push_c(&dec.to_string_lossy())?;
                cfg.model_config.transducer.joiner = push_c(&join.to_string_lossy())?;
                cfg.model_config.model_type = push_c("nemo_transducer")?;
            }
            AsrArchitecture::Moonshine | AsrArchitecture::MoonshineV2 => {
                let pre = pin_file(model_dir, spec.id, |n| n == "preprocess.onnx")?;
                let enc = pin_file(model_dir, spec.id, |n| n == "encode.int8.onnx")?;
                let uncached = pin_file(model_dir, spec.id, |n| n == "uncached_decode.int8.onnx")?;
                let cached = pin_file(model_dir, spec.id, |n| n == "cached_decode.int8.onnx")?;
                cfg.model_config.moonshine.preprocessor = push_c(&pre.to_string_lossy())?;
                cfg.model_config.moonshine.encoder = push_c(&enc.to_string_lossy())?;
                cfg.model_config.moonshine.uncached_decoder = push_c(&uncached.to_string_lossy())?;
                cfg.model_config.moonshine.cached_decoder = push_c(&cached.to_string_lossy())?;
            }
            AsrArchitecture::SenseVoice => {
                let mdl = pin_file(model_dir, spec.id, |n| n == "model.int8.onnx")?;
                let lang = if language.is_empty() { "auto" } else { language };
                cfg.model_config.sense_voice.model = push_c(&mdl.to_string_lossy())?;
                cfg.model_config.sense_voice.language = push_c(lang)?;
                cfg.model_config.sense_voice.use_itn = 1;
            }
            AsrArchitecture::Whisper => {
                let enc = pin_file(model_dir, spec.id, |n| n.ends_with("-encoder.int8.onnx"))?;
                let dec = pin_file(model_dir, spec.id, |n| n.ends_with("-decoder.int8.onnx"))?;
                let lang = if language.is_empty() || language == "auto" {
                    "en"
                } else {
                    language
                };
                cfg.model_config.whisper.encoder = push_c(&enc.to_string_lossy())?;
                cfg.model_config.whisper.decoder = push_c(&dec.to_string_lossy())?;
                cfg.model_config.whisper.language = push_c(lang)?;
                cfg.model_config.whisper.task = push_c("transcribe")?;
            }
        }

        let recognizer = unsafe { (api.create_offline_recognizer)(&cfg) };
        if recognizer.is_null() {
            bail!(
                "SherpaOnnxCreateOfflineRecognizer failed for model '{}' at {}",
                spec.id,
                model_dir.display()
            );
        }

        let vad = match silero_vad_path {
            Some(vp) if vp.exists() => {
                crate::integrity::verify_silero(vp)?;
                Some(SileroVad::new(Arc::clone(&api), vp)?)
            }
            _ => None,
        };

        Ok(Self {
            api,
            recognizer,
            vad,
            spec,
            _owned_cstrings: cstrings,
        })
    }

    pub fn version(&self) -> String {
        unsafe {
            let p = (self.api.get_version)();
            if p.is_null() {
                "unknown".to_string()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    pub fn model_spec(&self) -> &'static AsrModelSpec {
        self.spec
    }

    fn decode_single_chunk(&self, chunk_16k: &[f32], hotwords: Option<&str>) -> Result<(String, Option<String>)> {
        unsafe {
            let hw_c = match hotwords {
                Some(h) if !h.trim().is_empty() && self.spec.supports_hotwords => {
                    Some(CString::new(h)?)
                }
                _ => None,
            };
            let stream = match &hw_c {
                Some(cs) => (self.api.create_offline_stream_with_hotwords)(self.recognizer, cs.as_ptr()),
                None => (self.api.create_offline_stream)(self.recognizer),
            };
            if stream.is_null() {
                bail!("SherpaOnnxCreateOfflineStream returned NULL");
            }
            (self.api.accept_waveform_offline)(
                stream,
                16_000,
                chunk_16k.as_ptr(),
                chunk_16k.len() as i32,
            );
            (self.api.decode_offline_stream)(self.recognizer, stream);
            let json_ptr = (self.api.get_offline_stream_result_as_json)(stream);
            let mut text = String::new();
            let mut lang = None;
            if !json_ptr.is_null() {
                let raw_json = CStr::from_ptr(json_ptr).to_string_lossy();
                if let Ok(parsed) = serde_json::from_str::<SherpaResultJson>(&raw_json) {
                    text = parsed.text.trim().to_string();
                    if !parsed.lang.is_empty() {
                        lang = Some(parsed.lang);
                    }
                }
                (self.api.destroy_offline_stream_result_json)(json_ptr);
            }
            (self.api.destroy_offline_stream)(stream);
            Ok((text, lang))
        }
    }

    /// Transcribes 16 kHz mono float32 audio.
    /// Uses Silero VAD when loaded to reject non-speech noise/silence (zero hallucination!)
    /// and cleanly segment long dictations (>22s) at pause boundaries.
    pub fn transcribe(&mut self, samples_16k: &[f32], hotwords: Option<&str>) -> Result<TranscriptionOutput> {
        if samples_16k.is_empty() {
            return Ok(TranscriptionOutput {
                text: String::new(),
                detected_lang: None,
                vad_ms: 0.0,
                decode_ms: 0.0,
                speech_ratio: 0.0,
            });
        }

        // Fast RMS gate: pure digital silence or near-zero floor (< -58 dBFS) skips compute immediately.
        let energy: f32 = samples_16k.iter().map(|s| s * s).sum::<f32>() / (samples_16k.len() as f32);
        let rms = energy.sqrt();
        if rms < 0.0012 {
            return Ok(TranscriptionOutput {
                text: String::new(),
                detected_lang: None,
                vad_ms: 0.05,
                decode_ms: 0.0,
                speech_ratio: 0.0,
            });
        }

        let t_vad = Instant::now();
        let chunks: Vec<Vec<f32>> = if let Some(vad) = &mut self.vad {
            vad.extract_speech_segments(samples_16k)
        } else {
            vec![samples_16k.to_vec()]
        };
        let vad_ms = t_vad.elapsed().as_secs_f64() * 1000.0;

        if chunks.is_empty() {
            return Ok(TranscriptionOutput {
                text: String::new(),
                detected_lang: None,
                vad_ms,
                decode_ms: 0.0,
                speech_ratio: 0.0,
            });
        }

        let speech_samples: usize = chunks.iter().map(|c| c.len()).sum();
        let speech_ratio = (speech_samples as f32) / (samples_16k.len() as f32);

        let t_dec = Instant::now();
        let mut pieces = Vec::with_capacity(chunks.len());
        let mut detected_lang = None;
        for chunk in &chunks {
            let (piece, lang) = self.decode_single_chunk(chunk, hotwords)?;
            if detected_lang.is_none() {
                detected_lang = lang;
            }
            if !piece.is_empty() {
                pieces.push(piece);
            }
        }
        let decode_ms = t_dec.elapsed().as_secs_f64() * 1000.0;

        Ok(TranscriptionOutput {
            text: pieces.join(" "),
            detected_lang,
            vad_ms,
            decode_ms,
            speech_ratio,
        })
    }
}

impl Drop for LocalAsrEngine {
    fn drop(&mut self) {
        if !self.recognizer.is_null() {
            unsafe { (self.api.destroy_offline_recognizer)(self.recognizer) };
        }
    }
}
