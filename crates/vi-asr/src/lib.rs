pub mod catalog;
pub mod cloud;
pub mod ffi;
pub mod integrity;
pub mod live;

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
use zeroize::Zeroize;

/// One sentence of a transcript and the silence (ms) the speaker left before it.
/// The pause drives paragraph formatting; 0 means "unknown or none".
pub type Sentence = (String, u32);

#[derive(Debug, Clone, Default)]
pub struct TranscriptionOutput {
    pub text: String,
    pub detected_lang: Option<String>,
    pub vad_ms: f64,
    pub decode_ms: f64,
    pub speech_ratio: f32,
    /// `text` split into sentences, in order, each with the pause before it.
    pub sentences: Vec<Sentence>,
    /// Number of separately decoded audio chunks.
    pub chunks: usize,
    /// Total 16 kHz samples fed into this transcription.
    pub audio_samples: usize,
}

#[derive(Deserialize)]
struct SherpaResultJson {
    #[serde(default)]
    text: String,
    #[serde(default)]
    lang: String,
    #[serde(default)]
    tokens: Vec<String>,
    #[serde(default)]
    timestamps: Vec<f32>,
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
        cfg.silero_vad.max_speech_duration = 14.0;
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

    fn reset(&mut self) {
        unsafe { (self.api.vad_reset)(self.ptr) }
    }

    /// Feeds 16 kHz audio (any length) and appends every speech span that has finished to `out`
    /// as absolute sample positions `(start, end)`.
    fn accept(&mut self, samples_16k: &[f32], out: &mut Vec<(usize, usize)>) {
        unsafe {
            // 512-sample windows, exactly like a live stream, so results do not depend on how the caller batches.
            for w in samples_16k.chunks(512) {
                (self.api.vad_accept_waveform)(self.ptr, w.as_ptr(), w.len() as i32);
            }
        }
        self.pop_spans(out);
    }

    /// Ends the stream: closes a span that is still open.
    fn flush(&mut self, out: &mut Vec<(usize, usize)>) {
        unsafe { (self.api.vad_flush)(self.ptr) };
        self.pop_spans(out);
    }

    /// True while the detector is inside speech that has not finished yet.
    fn speech_active(&self) -> bool {
        unsafe { (self.api.vad_detected)(self.ptr) != 0 }
    }

    fn pop_spans(&mut self, out: &mut Vec<(usize, usize)>) {
        unsafe {
            while (self.api.vad_empty)(self.ptr) == 0 {
                let seg_ptr = (self.api.vad_front)(self.ptr);
                if !seg_ptr.is_null() {
                    let seg = &*seg_ptr;
                    if seg.n > 0 && !seg.samples.is_null() {
                        // The C struct's `start` is int32_t; only the low 32 bits are defined.
                        let start = (seg.start as u64 & 0xFFFF_FFFF) as u32 as usize;
                        out.push((start, start + seg.n as usize));
                    }
                    (self.api.destroy_speech_segment)(seg_ptr);
                }
                (self.api.vad_pop)(self.ptr);
            }
        }
    }
}

/// Real audio kept around each decoded group: Silero often fires a little late on soft onsets
/// ("um, I checked…" said quietly), which dropped the first word(s) in live microphone tests.
const PAD_PRE: usize = 4_800;
const PAD_POST: usize = 4_000;
/// How long after a short span ended we wait before deciding nothing will be merged into it.
const ONSET_MARGIN: usize = 6_400;
/// Once a group has at least 5 s of speech, any Silero VAD sentence pause (>= 450 ms = 7,200 samples)
/// closes and decodes it immediately without waiting for `merge_gap + ONSET_MARGIN`.
const READY_SPAN_LEN: usize = 5 * 16_000;
const VAD_MIN_SILENCE: usize = 7_000;

/// Chunking policy for decoding speech spans.
#[derive(Debug, Clone, Copy)]
pub struct ChunkPolicy {
    /// Spans separated by less silence than this (samples) are decoded together, so a short
    /// breath inside a sentence does not become a chunk boundary.
    pub merge_gap: usize,
    /// Longest audio (samples) decoded in one go.
    pub max_chunk: usize,
}

impl Default for ChunkPolicy {
    fn default() -> Self {
        // Dev knobs for the chunking benchmark: KEYREED_ASR_MERGE_GAP_MS, KEYREED_ASR_MAX_CHUNK_S.
        let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok());
        Self {
            // 250 ms, not 550: a normal breath inside a sentence used to keep the whole utterance
            // waiting for release. Closing chunks at shorter pauses decodes them while the user is
            // still talking — real-clip p50 1009 -> 532 ms, p95 4171 -> 1073 ms, WER unchanged (10.18%).
            merge_gap: env("KEYREED_ASR_MERGE_GAP_MS").map(|ms| ms * 16).unwrap_or(4_000),
            max_chunk: env("KEYREED_ASR_MAX_CHUNK_S").map(|s| s * 16_000).unwrap_or(24 * 16_000),
        }
    }
}

/// Groups finished speech spans into decode chunks and says how many leading spans can be
/// decoded now. Returns the size of each ready group, in order.
///
/// `speech_since` is the stream position at which the detector reported new speech that is
/// still running (None while it hears silence). `fin` = the recording is over.
fn ready_groups(
    pending: &[(usize, usize)],
    fed: usize,
    speech_since: Option<usize>,
    fin: bool,
    policy: ChunkPolicy,
) -> Vec<usize> {
    let mut groups = Vec::new();
    let mut g_start = 0usize;
    for i in 1..=pending.len() {
        let span_len = pending[i - 1].1.saturating_sub(pending[g_start].0);
        let closes = if i == pending.len() {
            true
        } else {
            let gap = pending[i].0.saturating_sub(pending[i - 1].1);
            gap >= policy.merge_gap
                || (span_len >= READY_SPAN_LEN && gap >= VAD_MIN_SILENCE)
                || pending[i].1.saturating_sub(pending[g_start].0) > policy.max_chunk
        };
        if !closes {
            continue;
        }
        if i == pending.len() && !fin {
            let last_end = pending[i - 1].1;
            if span_len >= READY_SPAN_LEN && fed >= last_end + PAD_POST {
                // Group is already >= 5 s and has its post-padding; next speech will start a new chunk.
            } else {
                // Short group: only ready once we know the next speech cannot be merged into it.
                let quiet_until = speech_since.unwrap_or(fed);
                if quiet_until.saturating_sub(last_end) < policy.merge_gap + ONSET_MARGIN {
                    break;
                }
            }
        }
        groups.push(i - g_start);
        g_start = i;
    }
    groups
}

fn starts_sentence(rest: &str) -> bool {
    rest.chars().next().map(|c| c.is_uppercase() || c.is_ascii_digit() || c == '"' || c == '\'').unwrap_or(false)
}

/// Splits punctuated text into sentences (a `.`, `?` or `!` followed by whitespace and then an
/// uppercase letter, digit or quote, so "3.5" and "e.g. this" stay intact).
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if matches!(bytes[i], b'.' | b'?' | b'!') {
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b'"' | b'\'' | b')') {
                j += 1;
            }
            let rest = &text[j..];
            let trimmed = rest.trim_start();
            if trimmed.len() < rest.len() && starts_sentence(trimmed) {
                out.push(text[start..j].trim());
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Pause (ms) before each sentence after the first, read from the model's token timestamps.
/// Returns None when the tokens do not line up with the sentence split (then pauses are unknown).
fn sentence_pauses(n_sentences: usize, tokens: &[String], timestamps: &[f32]) -> Option<Vec<u32>> {
    if n_sentences < 2 || tokens.len() != timestamps.len() {
        return None;
    }
    let mut pauses = Vec::new();
    for i in 0..tokens.len().saturating_sub(1) {
        let t = tokens[i].trim_end();
        let next = &tokens[i + 1];
        let next_word = next.trim_start_matches([' ', '\u{2581}']);
        if (t.ends_with('.') || t.ends_with('?') || t.ends_with('!')) && next_word.len() < next.len() && starts_sentence(next_word) {
            pauses.push(((timestamps[i + 1] - timestamps[i]).max(0.0) * 1000.0) as u32);
        }
    }
    (pauses.len() == n_sentences - 1).then_some(pauses)
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    const P: ChunkPolicy = ChunkPolicy { merge_gap: 8_800, max_chunk: 24 * 16_000 };

    #[test]
    fn groups_merge_short_gaps_and_wait_for_the_last() {
        // spans 0.4 s apart are one group; a 2 s gap starts a new one
        let pending = [(0, 16_000), (22_400, 40_000), (72_000, 90_000)];
        // just after the third span closed: the first group is ready, the last must wait
        assert_eq!(ready_groups(&pending, 97_200, None, false, P), vec![2]);
        // long silence after it: ready
        assert_eq!(ready_groups(&pending, 90_000 + 8_800 + ONSET_MARGIN, None, false, P), vec![2, 1]);
        // new speech began right after it: might merge, keep waiting
        assert_eq!(ready_groups(&pending, 200_000, Some(92_000), false, P), vec![2]);
        // new speech began long after it: cannot merge, ready
        assert_eq!(ready_groups(&pending, 200_000, Some(120_000), false, P), vec![2, 1]);
        // recording over: everything goes
        assert_eq!(ready_groups(&pending, 97_200, None, true, P), vec![2, 1]);
        assert!(ready_groups(&[], 10, None, true, P).is_empty());
    }

    #[test]
    fn groups_respect_the_chunk_cap() {
        // three 10 s spans 0.2 s apart: the third would exceed 24 s, so it starts a new group
        let pending = [(0, 160_000), (163_200, 323_200), (326_400, 486_400)];
        assert_eq!(ready_groups(&pending, 486_400, None, true, P), vec![2, 1]);
    }

    #[test]
    fn sentences_split_only_at_real_boundaries() {
        let s = split_sentences("Version 3.5 is out. Did it work? Yes! We use e.g. this, and so on.");
        assert_eq!(s, vec!["Version 3.5 is out.", "Did it work?", "Yes!", "We use e.g. this, and so on."]);
        assert_eq!(split_sentences("no punctuation at all"), vec!["no punctuation at all"]);
    }

    #[test]
    fn runtime_version_gate_for_hotword_decoding() {
        assert!(version_at_least("2.0.1", MIN_HOTWORD_RUNTIME));
        assert!(version_at_least("v1.12.24", MIN_HOTWORD_RUNTIME));
        assert!(version_at_least("1.13.0", MIN_HOTWORD_RUNTIME));
        assert!(!version_at_least("1.12.23", MIN_HOTWORD_RUNTIME));
        assert!(!version_at_least("1.9.30", MIN_HOTWORD_RUNTIME));
        assert!(!version_at_least("unknown", MIN_HOTWORD_RUNTIME));
        assert!(!version_at_least("", MIN_HOTWORD_RUNTIME));
        assert!(version_at_least("2.0.1-dev", MIN_HOTWORD_RUNTIME));
    }

    #[test]
    fn bpe_vocab_uses_the_first_field_of_each_token_line() {
        let tokens = "<unk> 0\n\u{2581}t 1\n\n\u{2581}th 2\n  \n<blk> 3\n";
        assert_eq!(bpe_vocab_from_tokens(tokens), "<unk>\t-1.0\n\u{2581}t\t-1.0\n\u{2581}th\t-1.0\n<blk>\t-1.0\n");
        assert_eq!(bpe_vocab_from_tokens(""), "");
        // deterministic
        assert_eq!(bpe_vocab_from_tokens(tokens), bpe_vocab_from_tokens(tokens));
    }

    fn tone(amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (i as f32 * 0.3).sin()).collect()
    }

    fn gate(samples: &[f32]) -> bool {
        let mut m = LevelMeter::default();
        // fed in odd pieces: windows must not depend on how the audio is batched
        for piece in samples.chunks(777) {
            m.push(piece);
        }
        m.passes()
    }

    #[test]
    fn level_gate_keeps_quiet_speech_and_drops_silence() {
        assert!(!gate(&[]));
        assert!(!gate(&vec![0.0; 32_000]));
        // a constant offset has no energy once the DC is removed
        assert!(!gate(&vec![0.2; 32_000]));
        // loud enough outright (-30 dBFS peak tone)
        assert!(gate(&tone(0.03, 16_000)));
        // quiet mic: speech about -60 dBFS RMS over a -80 dBFS floor, 70% silence; the old clip average fell under 0.0012
        let mut clip = tone(0.0001, 16_000 * 2);
        clip.extend(tone(0.0014, 16_000 / 2));
        clip.extend(tone(0.0001, 16_000 * 2));
        assert!(gate(&clip));
        // steady hiss with no stretch above its own floor
        assert!(!gate(&tone(0.0006, 16_000 * 3)));
    }

    #[test]
    fn level_gate_run_bridges_one_quiet_window_only() {
        let (q, l) = (-80.0f32, -62.0f32);
        let mut levels = vec![q; 20];
        levels.extend([l, l, l, q, l, l, l]);
        assert!(level_gate_passes(&levels));
        let mut levels = vec![q; 20];
        levels.extend([l, l, l, q, q, l, l, l]);
        assert!(!level_gate_passes(&levels));
        assert!(level_gate_passes(&[-40.0]));
    }

    #[test]
    fn pauses_come_from_token_times() {
        let toks: Vec<String> = [" It", " works", ".", " Next", " one", "."].iter().map(|s| s.to_string()).collect();
        let ts = [0.1, 0.4, 0.7, 2.2, 2.5, 2.8];
        assert_eq!(sentence_pauses(2, &toks, &ts), Some(vec![1500]));
        // token/sentence mismatch: unknown rather than wrong
        assert_eq!(sentence_pauses(3, &toks, &ts), None);
    }
}

impl Drop for SileroVad {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.api.destroy_vad)(self.ptr) };
        }
    }
}

/// One level window: 30 ms at 16 kHz.
const LEVEL_WINDOW: usize = 480;
/// Digital silence or a constant offset reads as this level.
const LEVEL_FLOOR_DB: f32 = -120.0;
/// A clip whose loudest window reaches this level is speech enough for the detector.
const GATE_LOUDEST_DB: f32 = -50.0;
/// A quiet clip also passes on a run this far above its own noise floor ...
const GATE_MARGIN_DB: f32 = 12.0;
/// ... when that run is at least this loud ...
const GATE_RELATIVE_FLOOR_DB: f32 = -66.0;
/// ... and this many windows long (180 ms).
const GATE_RUN_WINDOWS: usize = 6;

/// Measures a recording as it arrives: the DC-removed RMS level (dBFS) of each 30 ms window.
/// Thirty minutes is 60,000 floats, so memory stays bounded.
#[derive(Default)]
struct LevelMeter {
    levels: Vec<f32>,
    n: usize,
    sum: f64,
    sum_sq: f64,
}

impl LevelMeter {
    fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            let s = s as f64;
            self.sum += s;
            self.sum_sq += s * s;
            self.n += 1;
            if self.n == LEVEL_WINDOW {
                self.close_window();
            }
        }
    }

    fn close_window(&mut self) {
        let n = self.n as f64;
        let mean = self.sum / n;
        let var = (self.sum_sq / n - mean * mean).max(0.0);
        self.levels.push(((20.0 * var.sqrt().max(1e-6).log10()) as f32).max(LEVEL_FLOOR_DB));
        (self.n, self.sum, self.sum_sq) = (0, 0.0, 0.0);
    }

    /// Whether the clip is worth the speech detector's attention (a short last window counts).
    fn passes(&mut self) -> bool {
        if self.n > 0 {
            self.close_window();
        }
        level_gate_passes(&self.levels)
    }
}

/// Hex's level gate: the loudest window reaches -50 dBFS, or a run of 6 windows sits 12 dB over
/// the clip's own noise floor (10th percentile window, ignoring digital silence) and above -66 dBFS.
fn level_gate_passes(levels: &[f32]) -> bool {
    if levels.iter().copied().fold(LEVEL_FLOOR_DB, f32::max) >= GATE_LOUDEST_DB {
        return true;
    }
    let mut audible: Vec<f32> = levels.iter().copied().filter(|&l| l > LEVEL_FLOOR_DB).collect();
    if audible.is_empty() {
        return false;
    }
    audible.sort_by(f32::total_cmp);
    let threshold = (audible[(audible.len() - 1) / 10] + GATE_MARGIN_DB).max(GATE_RELATIVE_FLOOR_DB);
    // One quiet window between loud ones is bridged but not counted; two in a row end the run.
    let (mut run, mut bridged) = (0usize, false);
    for &l in levels {
        if l >= threshold {
            run += 1;
            bridged = false;
            if run >= GATE_RUN_WINDOWS {
                return true;
            }
        } else if run == 0 {
            bridged = false;
        } else if bridged {
            run = 0;
            bridged = false;
        } else {
            bridged = true;
        }
    }
    false
}

/// Oldest sherpa-onnx whose NeMo transducer supports `modified_beam_search` with hotwords. An older
/// runtime would call `exit(-1)` on the unknown decoding method, so it is never asked.
const MIN_HOTWORD_RUNTIME: (u32, u32, u32) = (1, 12, 24);
const HOTWORDS_SCORE: f32 = 2.0;
const HOTWORD_BEAM_PATHS: i32 = 4;
/// Decode chunks shorter than this get trailing zeros on Parakeet-style models.
const MIN_DECODE_SAMPLES: usize = 16_000;

/// `true` when a version string such as "2.0.1" or "v1.12.24" is at least `min`.
pub fn version_at_least(version: &str, min: (u32, u32, u32)) -> bool {
    let mut parts = version.trim().trim_start_matches('v').split('.').map(|p| {
        let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u32>().ok()
    });
    match (parts.next().flatten(), parts.next().flatten(), parts.next().flatten()) {
        (Some(a), Some(b), Some(c)) => (a, b, c) >= min,
        _ => false,
    }
}

/// The `bpe.vocab` text sherpa needs to encode hotwords: the first field of every non-blank
/// `tokens.txt` line, then a tab and `-1.0` (equal scores give longest-match encoding). Same
/// derivation as the official Parakeet hotwords example, which the release does not ship.
pub fn bpe_vocab_from_tokens(tokens_txt: &str) -> String {
    let mut out = String::with_capacity(tokens_txt.len());
    for line in tokens_txt.split('\n').filter(|l| !l.trim().is_empty()) {
        out.push_str(line.split(' ').next().unwrap_or(""));
        out.push_str("\t-1.0\n");
    }
    out
}

/// Writes the derived vocab to `<cache_dir>/<model_id>.bpe.vocab` unless it is already identical.
/// It lives outside the pinned model directory, so integrity checks and installs never see it.
fn write_bpe_vocab(cache_dir: &Path, model_id: &str, tokens_path: &Path) -> Result<PathBuf> {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let text = bpe_vocab_from_tokens(&fs::read_to_string(tokens_path)?);
    let dest = cache_dir.join(format!("{model_id}.bpe.vocab"));
    if fs::read_to_string(&dest).map(|t| t == text).unwrap_or(false) {
        return Ok(dest);
    }
    fs::create_dir_all(cache_dir)?;
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = cache_dir.join(format!("{model_id}.bpe.vocab.{}-{seq}.tmp", std::process::id()));
    fs::write(&tmp, &text)?;
    fs::rename(&tmp, &dest).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(dest)
}

/// State of one recording being recognised incrementally.
#[derive(Default)]
struct StreamState {
    /// 16 kHz audio not decoded yet; `audio[0]` is absolute sample `base`.
    audio: Vec<f32>,
    base: usize,
    fed: usize,
    levels: LevelMeter,
    pending: Vec<(usize, usize)>,
    /// End of the last decoded speech span (absolute), for pauses and padding.
    last_end: Option<usize>,
    speech_since: Option<usize>,
    speech_samples: usize,
    out: TranscriptionOutput,
}

/// Local CPU/ONNX offline speech recognizer supporting Parakeet TDT v2/v3, Moonshine, SenseVoice, and Whisper.
pub struct LocalAsrEngine {
    api: Arc<SherpaApi>,
    recognizer: *const SherpaOnnxOfflineRecognizer,
    vad: Option<SileroVad>,
    spec: &'static AsrModelSpec,
    /// Beam search with hotwords is active (otherwise greedy, and hotwords are ignored).
    hotword_decoding: bool,
    /// Hotword decoding was asked for at load (it may still be off, e.g. an old runtime).
    hotword_requested: bool,
    _owned_cstrings: Vec<CString>,
    stream: StreamState,
    policy: ChunkPolicy,
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
        Self::load_with_hotwords(sherpa_lib_dir, model_dir, model_id, silero_vad_path, num_threads, language, None)
    }

    /// Like `load`. With `hotword_cache_dir` the recognizer uses `modified_beam_search` so personal
    /// vocabulary biases decoding; this only happens for models that support hotwords and a runtime
    /// that is new enough, otherwise it stays on greedy search. See `hotword_decoding`.
    pub fn load_with_hotwords(
        sherpa_lib_dir: &Path,
        model_dir: &Path,
        model_id: &str,
        silero_vad_path: Option<&Path>,
        num_threads: usize,
        language: &str,
        hotword_cache_dir: Option<&Path>,
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

        let tokens_path = pin_file(model_dir, spec.id, |n| n == "tokens.txt" || n.ends_with("-tokens.txt"))?;
        let hotword_requested = hotword_cache_dir.is_some()
            && spec.supports_hotwords
            && spec.architecture == AsrArchitecture::NemoTransducer;
        let mut hotword_vocab: Option<PathBuf> = None;
        if let (true, Some(dir)) = (hotword_requested, hotword_cache_dir) {
            let version = unsafe {
                let p = (api.get_version)();
                if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
            };
            if !version_at_least(&version, MIN_HOTWORD_RUNTIME) {
                log::warn!("sherpa-onnx '{version}' is too old for hotword decoding; using greedy search");
            } else {
                match write_bpe_vocab(dir, spec.id, &tokens_path) {
                    Ok(p) => hotword_vocab = Some(p),
                    Err(e) => log::warn!("hotword vocab could not be written ({e:#}); using greedy search"),
                }
            }
        }

        let mut cstrings: Vec<CString> = Vec::with_capacity(14);
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
        let thr = std::env::var("KEYREED_ASR_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(num_threads);
        cfg.model_config.num_threads = thr.clamp(1, 16) as i32;
        cfg.model_config.provider = push_c("cpu")?;
        cfg.model_config.debug = 0;
        cfg.decoding_method = push_c("greedy_search")?;

        cfg.model_config.tokens = push_c(&tokens_path.to_string_lossy())?;
        if let Some(vocab) = &hotword_vocab {
            cfg.decoding_method = push_c("modified_beam_search")?;
            cfg.max_active_paths = HOTWORD_BEAM_PATHS;
            cfg.hotwords_score = HOTWORDS_SCORE;
            cfg.model_config.modeling_unit = push_c("bpe")?;
            cfg.model_config.bpe_vocab = push_c(&vocab.to_string_lossy())?;
        }

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
            AsrArchitecture::GectorTagger => bail!("'{}' is a grammar model, not a speech model", spec.id),
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
            hotword_decoding: hotword_vocab.is_some(),
            hotword_requested,
            _owned_cstrings: cstrings,
            stream: StreamState::default(),
            policy: ChunkPolicy::default(),
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

    /// True when hotwords really bias this engine (beam search is on).
    pub fn hotword_decoding(&self) -> bool {
        self.hotword_decoding
    }

    /// True when the engine was loaded with hotword decoding requested, whether or not it took effect.
    pub fn hotword_requested(&self) -> bool {
        self.hotword_requested
    }

    pub fn model_spec(&self) -> &'static AsrModelSpec {
        self.spec
    }

    /// Decodes one chunk: (text, detected language, tokens, token start times in seconds).
    fn decode_single_chunk(
        &self,
        chunk_16k: &[f32],
        hotwords: Option<&str>,
    ) -> Result<(String, Option<String>, Vec<String>, Vec<f32>)> {
        unsafe {
            let hw_c = match hotwords {
                Some(h) if !h.trim().is_empty() && self.hotword_decoding => {
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
            let (mut tokens, mut times) = (Vec::new(), Vec::new());
            if !json_ptr.is_null() {
                let raw_json = CStr::from_ptr(json_ptr).to_string_lossy();
                if let Ok(parsed) = serde_json::from_str::<SherpaResultJson>(&raw_json) {
                    text = parsed.text.trim().to_string();
                    if !parsed.lang.is_empty() {
                        lang = Some(parsed.lang);
                    }
                    tokens = parsed.tokens;
                    times = parsed.timestamps;
                }
                (self.api.destroy_offline_stream_result_json)(json_ptr);
            }
            (self.api.destroy_offline_stream)(stream);
            Ok((text, lang, tokens, times))
        }
    }

    /// Starts recognising a new recording. Audio then arrives through `stream_feed`.
    pub fn stream_begin(&mut self) {
        self.stream_reset();
        if let Some(vad) = &mut self.vad {
            vad.reset();
        }
    }

    /// Chunks decoded so far in the current recording.
    pub fn stream_chunks(&self) -> usize {
        self.stream.out.chunks
    }

    /// Wipes the audio of the current recording (cancel, or after it finished).
    pub fn stream_reset(&mut self) {
        self.stream.audio.zeroize();
        self.stream = StreamState::default();
    }

    /// Feeds the next piece of 16 kHz mono audio (call it with small pieces as they are captured)
    /// and decodes every speech chunk that is complete, so the work overlaps with the recording.
    pub fn stream_feed(&mut self, samples_16k: &[f32], hotwords: Option<&str>) -> Result<()> {
        self.stream_push(samples_16k);
        self.decode_ready(false, hotwords)
    }

    fn stream_push(&mut self, samples_16k: &[f32]) {
        let st = &mut self.stream;
        st.audio.extend_from_slice(samples_16k);
        st.fed += samples_16k.len();
        st.levels.push(samples_16k);
        if let Some(vad) = &mut self.vad {
            let t = Instant::now();
            vad.accept(samples_16k, &mut st.pending);
            st.speech_since = match (vad.speech_active(), st.speech_since) {
                (false, _) => None,
                (true, None) => Some(st.fed),
                (true, since) => since,
            };
            st.out.vad_ms += t.elapsed().as_secs_f64() * 1000.0;
        }
    }

    /// Ends the recording: decodes whatever is left and returns the whole transcript.
    pub fn stream_finish(&mut self, hotwords: Option<&str>) -> Result<TranscriptionOutput> {
        let fed = self.stream.fed;
        // Level gate: digital silence or a clip with no louder stretch than its own floor is never speech.
        if self.stream.levels.passes() {
            match &mut self.vad {
                Some(vad) => {
                    let t = Instant::now();
                    vad.flush(&mut self.stream.pending);
                    self.stream.out.vad_ms += t.elapsed().as_secs_f64() * 1000.0;
                }
                None => self.stream.pending.push((0, fed)),
            }
            if let Err(e) = self.decode_ready(true, hotwords) {
                self.stream_reset();
                return Err(e);
            }
        }
        let mut out = std::mem::take(&mut self.stream.out);
        out.audio_samples = fed;
        out.speech_ratio = if fed == 0 { 0.0 } else { self.stream.speech_samples as f32 / fed as f32 };
        out.text = out.sentences.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(" ");
        self.stream_reset();
        Ok(out)
    }

    fn decode_ready(&mut self, fin: bool, hotwords: Option<&str>) -> Result<()> {
        let st = &self.stream;
        let groups = ready_groups(&st.pending, st.fed, st.speech_since, fin, self.policy);
        for n in groups {
            let st = &self.stream;
            let (first, last) = (st.pending[0], st.pending[n - 1]);
            // Padding is real audio, but never audio that an earlier chunk already decoded.
            let from = first.0.saturating_sub(PAD_PRE).max(st.last_end.unwrap_or(0)).max(st.base);
            let to = (last.1 + PAD_POST).min(st.fed);
            let t = Instant::now();
            let audio = &st.audio[from - st.base..to - st.base];
            // Parakeet is unreliable on under a second of audio: end the chunk with silence.
            let mut padded = Vec::new();
            if audio.len() < MIN_DECODE_SAMPLES && self.spec.architecture == AsrArchitecture::NemoTransducer {
                padded.extend_from_slice(audio);
                padded.resize(MIN_DECODE_SAMPLES, 0.0);
            }
            let res = self.decode_single_chunk(if padded.is_empty() { audio } else { &padded }, hotwords);
            padded.zeroize();
            let (text, lang, tokens, times) = res?;
            let st = &mut self.stream;
            st.out.decode_ms += t.elapsed().as_secs_f64() * 1000.0;
            st.out.chunks += 1;
            if st.out.detected_lang.is_none() {
                st.out.detected_lang = lang;
            }
            let sentences = split_sentences(&text);
            let pauses = sentence_pauses(sentences.len(), &tokens, &times);
            let gap_ms = st
                .last_end
                .map(|e| {
                    let vad_gap = (first.0.saturating_sub(e) / 16) as u32;
                    let tok_gap = times
                        .first()
                        .map(|&t| ((from as f32 / 16.0 + t * 1000.0) - (e as f32 / 16.0)).max(0.0) as u32)
                        .unwrap_or(0);
                    vad_gap.max(tok_gap)
                })
                .unwrap_or(0);
            for (i, s) in sentences.iter().enumerate() {
                let pause = if i == 0 { gap_ms } else { pauses.as_ref().map(|p| p[i - 1]).unwrap_or(0) };
                st.out.sentences.push((s.to_string(), pause));
            }
            st.speech_samples += st.pending[..n].iter().map(|(a, b)| b - a).sum::<usize>();
            st.last_end = Some(last.1);
            st.pending.drain(..n);
            // Decoded audio is not needed again: wipe it and keep only what follows.
            let cut = last.1.min(st.fed) - st.base;
            st.audio[..cut].zeroize();
            st.audio.drain(..cut);
            st.base += cut;
        }
        Ok(())
    }

    /// Runs one tiny decode so the first real dictation does not pay ONNX Runtime's first-run cost.
    pub fn warm_up(&mut self) {
        let noise: Vec<f32> = (0..16_000u32).map(|i| ((i.wrapping_mul(2_654_435_761) >> 16) as f32 / 65_536.0 - 0.5) * 0.02).collect();
        let _ = self.decode_single_chunk(&noise, None);
        if let Some(vad) = &mut self.vad {
            vad.accept(&noise, &mut Vec::new());
            vad.reset();
        }
    }

    /// Transcribes a complete 16 kHz mono recording in one call (files, tests, cloud fallback).
    /// Uses Silero VAD when loaded to reject non-speech noise/silence and to cut long dictations
    /// at pauses. Same chunking as the live path, decided from the real gaps between spans.
    pub fn transcribe(&mut self, samples_16k: &[f32], hotwords: Option<&str>) -> Result<TranscriptionOutput> {
        self.stream_begin();
        self.stream_push(samples_16k);
        self.stream_finish(hotwords)
    }
}

impl Drop for LocalAsrEngine {
    fn drop(&mut self) {
        if !self.recognizer.is_null() {
            unsafe { (self.api.destroy_offline_recognizer)(self.recognizer) };
        }
    }
}
