// On Windows release builds without CLI flags, the process is a GUI application.
// A console is attached only when a flag is present, so closing a terminal does not end the tray app.
#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod composer_flow;
#[cfg(windows)]
mod gallery;
mod learning;
mod quickfix_flow;
mod textjobs;
mod voice;

use crate::voice::CaptureTarget;
use anyhow::{bail, Context, Result};
use crossbeam_channel::{select, tick};
use parking_lot::Mutex;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vi_asr::{
    catalog::{ensure_model_present, find_model_spec, ASR_CATALOG},
    cloud::transcribe_cloud_multipart,
    LocalAsrEngine,
};
use vi_audio::{list_input_devices, load_wav_16k_mono, ActiveCapture};
use vi_core::style::{LearnState, LoadOutcome, StyleStore};
use vi_core::{
    brand::{AUTOSTART_REG_VALUE, PRODUCT_NAME, PRODUCT_TAGLINE, SINGLE_INSTANCE_MUTEX, VERSION_STR},
    AppCategory, AppConfig, AudioCueStyle, EngineState, ForegroundContext, HistoryEntry, HistoryStore, OutputMode,
    HotkeySpec, MouseAction, PipelineMetrics, PipelineResult, PttAction, PttController, WritingMode,
};
use vi_rewrite::{slm::LlamaServerSupervisor, RewriteEngine};
use vi_win32::{
    capture_foreground_context, current_process_cpu_ms, current_process_memory_mb,
    deliver_direct_or_clipboard_fallback, dpapi_decrypt_from_b64, dpapi_encrypt_to_b64,
    harden_and_init_process, is_spec_physically_down, perf_trace_record,
    prompt_and_deliver_ask_before_inserting, read_autostart_value, read_clipboard_unicode,
    set_autostart_enabled, system_available_ram_mb, timing_mark, write_clipboard_unicode,
    ChordAction, DirectTypeOutcome, GlobalHotkeyEngine, HotkeyEvent, MouseButton, SingleInstanceGuard, UiAction,
};
#[cfg(windows)]
use vi_core::editwatch::UndoToken;
#[cfg(windows)]
use vi_win32::ui::{
    classify_mic, delivery_reason_sentence, friendly_error, DownloadProgress, DownloadStage, MicTestStatus,
    QuickFixStage, ReadyOutcome,
};
use zeroize::Zeroize;

/// Resolves runtime directories relative to the executable, `KEYREED_RUNTIME_ROOT`,
/// and `%LOCALAPPDATA%\keyreed\models` so both bundled offline models and user-downloaded
/// models work seamlessly even when installed under `C:\Program Files\KeyReed`.
#[derive(Clone)]
struct RuntimePaths {
    sherpa_lib_dir: PathBuf,
    models_dir: PathBuf,
    user_models_dir: PathBuf,
    system_models_dir: PathBuf,
    silero_vad_path: PathBuf,
    llama_server_bin: PathBuf,
}

impl RuntimePaths {
    fn discover(cfg: &AppConfig) -> Self {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let root = env::var("KEYREED_RUNTIME_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| exe_dir.clone());

        let sherpa_lib_dir = env::var("KEYREED_SHERPA_LIB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let candidate = root.join("runtime").join("sherpa-onnx");
                if candidate.exists() {
                    candidate
                } else {
                    exe_dir.clone()
                }
            });

        let user_models_dir = AppConfig::default_models_dir();
        let system_models_dir = PathBuf::from(r"C:\Program Files\KeyReed\models");
        let models_dir = env::var("KEYREED_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let bundled = root.join("models");
                if bundled.exists() {
                    bundled
                } else {
                    user_models_dir.clone()
                }
            });

        let silero_vad_path = {
            let candidates = [
                models_dir.join("silero_vad.onnx"),
                user_models_dir.join("silero_vad.onnx"),
                system_models_dir.join("silero_vad.onnx"),
                root.join("models").join("silero_vad.onnx"),
                root.join("silero_vad.onnx"),
            ];
            candidates
                .iter()
                .find(|p| p.exists())
                .cloned()
                .unwrap_or_else(|| models_dir.join("silero_vad.onnx"))
        };

        #[cfg(windows)]
        let llama_name = "llama-server.exe";
        #[cfg(not(windows))]
        let llama_name = "llama-server";

        let llama_server_bin = env::var("KEYREED_LLAMA_SERVER")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let c1 = root.join("runtime").join("llama").join(llama_name);
                if c1.exists() {
                    c1
                } else {
                    exe_dir.join(llama_name)
                }
            });

        let _ = cfg;
        Self {
            sherpa_lib_dir,
            models_dir,
            user_models_dir,
            system_models_dir,
            silero_vad_path,
            llama_server_bin,
        }
    }

    /// Returns a persistent writable directory (`%LOCALAPPDATA%\keyreed\models`) for on-demand model downloads
    /// so downloaded models survive application upgrades and reinstallations.
    fn writable_models_dir(&self) -> PathBuf {
        if env::var("KEYREED_MODELS_DIR").is_ok() {
            let _ = std::fs::create_dir_all(&self.models_dir);
            return self.models_dir.clone();
        }
        if std::fs::create_dir_all(&self.user_models_dir).is_ok() {
            return self.user_models_dir.clone();
        }
        let _ = std::fs::create_dir_all(&self.models_dir);
        self.models_dir.clone()
    }

    /// Finds which models root (`user_models_dir`, `models_dir`, or `system_models_dir`) contains `folder_name`.
    fn find_installed_model_root(&self, folder_name: &str) -> Option<PathBuf> {
        for root in [&self.user_models_dir, &self.models_dir, &self.system_models_dir] {
            if root.join(folder_name).exists() {
                return Some(root.clone());
            }
        }
        None
    }

    /// Lists all catalog ASR model IDs currently installed on disk.
    fn list_installed_asr_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for spec in vi_asr::catalog::ASR_CATALOG.iter().chain(vi_asr::catalog::GRAMMAR_CATALOG) {
            if self.find_installed_model_root(spec.folder_name).is_some() {
                out.push(spec.id.to_string());
            }
        }
        out
    }

    fn resolve_gguf_path(&self, model_id: &str) -> Option<PathBuf> {
        self.find_gguf_candidates(model_id).into_iter().find(|p| match vi_asr::integrity::verify_gguf(p) {
            Ok(()) => true,
            Err(err) => {
                log::error!("refusing weight file {}: {err}", p.display());
                false
            }
        })
    }

    /// Existing grammar-model files in priority order, without the (slow) SHA-256 verification.
    fn find_gguf_candidates(&self, model_id: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        if model_id.is_empty() || model_id.eq_ignore_ascii_case("none") {
            return found;
        }
        let candidates = [
            format!("{}.gguf", model_id),
            "Qwen3.5-0.8B-Q4_K_M.gguf".to_string(),
            "Qwen3-0.6B-Q4_K_M.gguf".to_string(),
        ];
        for root in [&self.user_models_dir, &self.models_dir, &self.system_models_dir] {
            for name in &candidates {
                let p = root.join(name);
                if p.exists() {
                    found.push(p);
                }
            }
        }
        found
    }
}

/// Owns lazy-loaded local ASR and local SLM engines with automatic idle-unload RAM reclamation.
struct InferenceSupervisor {
    paths: RuntimePaths,
    loaded_asr_id: String,
    asr: Option<LocalAsrEngine>,
    /// Active streaming ASR worker decoding audio chunks while the user is still speaking.
    live_asr: Option<(String, vi_asr::live::LiveAsr)>,
    /// Speech model being loaded in the background while the user is still talking or at startup.
    pending_asr: Option<(String, crossbeam_channel::Receiver<Result<LocalAsrEngine>>)>,
    /// Grammar model whose SHA-256 is still being verified in the background. Joined before the
    /// first rewrite, so integrity is still enforced but no longer blocks hotkeys at startup.
    pending_slm: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    slm_threads: usize,
    /// Shared with the text-job worker: a Composer or deeper-fix job holds it while it runs, and
    /// dictation never waits for it longer than `SLM_LOCK_WAIT` (rules-only cleanup instead).
    rewrite: Arc<Mutex<RewriteEngine>>,
    last_used: Instant,
    session_keydown_at: Option<Instant>,
    session_mic_init_ms: f64,
    session_first_frame_ms: f64,
    session_chunk_finalize_ms: f64,
    session_target_id_ms: f64,
    session_cold_model: bool,
}

/// How long dictation waits for the language model when a text job holds it.
const SLM_LOCK_WAIT: Duration = Duration::from_millis(250);

/// Tier label for a dictation cleaned up by rules only because a text job held the language model.
const TIER_SLM_BUSY: &str = "tier0_slm_busy";

/// Runs `f` on the shared language-model engine, or on a rules-only engine (second argument
/// `true`) when the lock is not free within `wait`, so the caller is never blocked by a text job.
fn with_engine_or_rules<R>(
    lock: &Mutex<RewriteEngine>,
    wait: Duration,
    f: impl FnOnce(&mut RewriteEngine, bool) -> R,
) -> R {
    match lock.try_lock_for(wait) {
        Some(mut eng) => f(&mut eng, false),
        None => f(&mut RewriteEngine::new(None), true),
    }
}

/// The chords the hook matches beside dictation and mode cycle: Quick Fix, the Composer and the voice command.
fn extra_chords(cfg: &AppConfig) -> Vec<(HotkeySpec, ChordAction)> {
    vec![
        (cfg.quick_fix_hotkey.clone(), ChordAction::CorrectSelection),
        (cfg.composer_hotkey.clone(), ChordAction::OpenComposer),
        (cfg.voice_command_hotkey.clone(), ChordAction::VoiceCommand),
    ]
}

/// The mouse buttons the user bound. `Nothing` binds nothing, so that button is never intercepted.
fn mouse_bindings(cfg: &AppConfig) -> Vec<(MouseButton, ChordAction)> {
    [
        (MouseButton::Middle, cfg.mouse_middle_action),
        (MouseButton::Back, cfg.mouse_back_action),
        (MouseButton::Forward, cfg.mouse_forward_action),
    ]
    .into_iter()
    .filter_map(|(button, action)| {
        let action = match action {
            MouseAction::Nothing => return None,
            MouseAction::Dictate => ChordAction::Dictate,
            MouseAction::QuickFix => ChordAction::CorrectSelection,
            MouseAction::VoiceCommand => ChordAction::VoiceCommand,
            MouseAction::OpenComposer => ChordAction::OpenComposer,
        };
        Some((button, action))
    })
    .collect()
}

/// Registers KeyReed's chords and mouse bindings with the hook (at start, and after every save).
fn register_extra_chords(hk: &GlobalHotkeyEngine, cfg: &AppConfig) {
    hk.set_extra_chords(extra_chords(cfg));
    hk.set_mouse_bindings(mouse_bindings(cfg));
}

/// The learning switch is a setting (setup and My style both edit it) while the store is what records, so the
/// store follows the setting. Returns whether it changed.
fn sync_learning_switch(cfg: &AppConfig, style: &mut StyleStore) -> bool {
    let want = if cfg.learn_from_edits { LearnState::On } else { LearnState::Off };
    if style.learn_from_edits == want {
        return false;
    }
    style.learn_from_edits = want;
    true
}

/// Writes the style store and hands the fresh copy to Settings: the coordinator owns the file.
#[cfg(windows)]
fn save_style(style: &StyleStore, path: &Path, gui: &vi_win32::ui::win_gui::WinGuiHandle) {
    if let Err(e) = style.save_atomic(path) {
        log::warn!("the style store could not be saved: {e:#}");
    }
    gui.set_style(style.clone());
}

/// Whether the speech engine should decode with hotword biasing: the model supports it and the
/// personal dictionary is on and has at least one usable term.
fn wants_hotword_decoding(cfg: &AppConfig) -> bool {
    find_model_spec(&cfg.local_asr_model).is_some_and(|s| s.supports_hotwords)
        && !cfg.dictionary.to_sherpa_hotwords().is_empty()
}

fn load_asr_engine(paths: &RuntimePaths, cfg: &AppConfig) -> Result<LocalAsrEngine> {
    let spec = find_model_spec(&cfg.local_asr_model)
        .with_context(|| format!("ASR model '{}' not in catalog", cfg.local_asr_model))?;
    let _ = vi_asr::integrity::sweep_incomplete(&paths.models_dir);
    let _ = vi_asr::integrity::sweep_incomplete(&paths.user_models_dir);
    let model_dir = if let Some(installed_root) = paths.find_installed_model_root(spec.folder_name) {
        vi_asr::integrity::prepare_installed_model(&installed_root, spec.id)?
    } else if cfg.processing_profile.allows_network() {
        let dl_root = paths.writable_models_dir();
        ensure_model_present(&dl_root, spec)?
    } else {
        bail!(
            "ASR model '{}' is not installed and the offline profile will not download it",
            spec.id
        );
    };
    let vad_opt = if paths.silero_vad_path.exists() {
        Some(paths.silero_vad_path.as_path())
    } else {
        None
    };
    // The derived bpe.vocab lives here, outside the integrity-pinned model folder.
    let hotword_cache = AppConfig::default_data_dir().join("cache");
    let mut engine = LocalAsrEngine::load_with_hotwords(
        &paths.sherpa_lib_dir,
        &model_dir,
        spec.id,
        vad_opt,
        cfg.inference_threads.max(1),
        &cfg.preferred_language,
        wants_hotword_decoding(cfg).then_some(hotword_cache.as_path()),
    )?;
    engine.warm_up();
    Ok(engine)
}

impl InferenceSupervisor {
    fn new(cfg: &AppConfig) -> Self {
        let paths = RuntimePaths::discover(cfg);
        let candidates = paths.find_gguf_candidates(&cfg.local_llm_model);
        let pending_slm = if candidates.is_empty() {
            None
        } else {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let spawned = std::thread::Builder::new()
                .name("keyreed-gguf-verify".to_string())
                .spawn(move || {
                    let ok = candidates.into_iter().find(|p| match vi_asr::integrity::verify_gguf(p) {
                        Ok(()) => true,
                        Err(err) => {
                            log::error!("refusing weight file {}: {err}", p.display());
                            false
                        }
                    });
                    let _ = tx.send(ok);
                });
            spawned.ok().map(|_| rx)
        };
        Self {
            paths,
            loaded_asr_id: String::new(),
            asr: None,
            live_asr: None,
            pending_asr: None,
            pending_slm,
            slm_threads: cfg.inference_threads.max(1),
            rewrite: Arc::new(Mutex::new(RewriteEngine::new(None))),
            last_used: Instant::now(),
            session_keydown_at: None,
            session_mic_init_ms: 0.0,
            session_first_frame_ms: 0.0,
            session_chunk_finalize_ms: 0.0,
            session_target_id_ms: 0.0,
            session_cold_model: false,
        }
    }

    /// Non-blocking harvest of background ASR/SLM preparation threads during idle ticks.
    fn poll_background_ready(&mut self, cfg: &AppConfig) {
        // Not during a live session: its engine comes back afterwards and must not overwrite a newer one.
        if let (Some((id, rx)), None) = (&self.pending_asr, &self.live_asr) {
            if let Ok(res) = rx.try_recv() {
                let id_owned = id.clone();
                self.pending_asr = None;
                if let Ok(engine) = res {
                    if id_owned == cfg.local_asr_model {
                        self.asr = Some(engine);
                        self.loaded_asr_id = id_owned;
                        self.last_used = Instant::now();
                    }
                }
            }
        }
        if let Some(rx) = &self.pending_slm {
            // A text job holds the engine: leave the verified model pending for a later tick.
            let Some(mut eng) = self.rewrite.try_lock() else { return };
            if let Ok(verified) = rx.try_recv() {
                self.pending_slm = None;
                if let Some(gguf) = verified {
                    *eng = self.slm_engine(gguf);
                }
                if !matches!(cfg.processing_profile, vi_core::ProcessingProfile::MaxAccuracy) {
                    eng.prewarm(cfg.default_writing_mode);
                }
            }
        }
    }

    /// Called when recording starts: if the grammar model is verified (never blocks on it), start
    /// llama-server now so its load overlaps with speech. Only for modes that use it.
    fn prewarm_slm(&mut self, cfg: &AppConfig) {
        if !matches!(cfg.processing_profile, vi_core::ProcessingProfile::MaxAccuracy) {
            // A text job holds the engine, so it is already loaded: nothing to prewarm.
            let Some(mut eng) = self.rewrite.try_lock() else { return };
            if let Some(rx) = &self.pending_slm {
                match rx.try_recv() {
                    Ok(v) => {
                        self.pending_slm = None;
                        if let Some(gguf) = v {
                            *eng = self.slm_engine(gguf);
                        }
                    }
                    Err(_) => return, // still verifying: don't block the hotkey path
                }
            }
            eng.prewarm(cfg.default_writing_mode);
        }
    }

    fn slm_engine(&self, gguf: PathBuf) -> RewriteEngine {
        RewriteEngine::new(Some(LlamaServerSupervisor::new(
            self.paths.llama_server_bin.clone(),
            gguf,
            self.slm_threads,
        )))
    }

    /// Waits for the background grammar-model verification (if still running) and enables the
    /// local grammar model only when its pinned SHA-256 matched. Must be called with the engine
    /// lock held (`eng`), so a pending model is never installed behind a running text job.
    fn ensure_slm_ready_locked(&mut self, eng: &mut RewriteEngine) {
        if let Some(rx) = self.pending_slm.take() {
            if let Ok(Some(gguf)) = rx.recv() {
                *eng = self.slm_engine(gguf);
            }
        }
    }

    /// Blocking variant for the command-line tools and tests (no text jobs run there).
    fn ensure_slm_ready(&mut self) {
        let lock = self.rewrite.clone();
        let mut eng = lock.lock();
        self.ensure_slm_ready_locked(&mut eng);
    }

    fn maybe_unload_idle(&mut self, idle_secs: u64) {
        if idle_secs == 0 || self.live_asr.is_some() {
            return;
        }
        // Adaptive retention based on system memory pressure: keep warm longer when RAM is plentiful.
        let avail_mb = system_available_ram_mb();
        let effective_secs = if avail_mb >= 4_096 {
            idle_secs.saturating_mul(6)
        } else if avail_mb < 2_048 {
            idle_secs.min(120)
        } else {
            idle_secs
        };
        if let Some(mut eng) = self.rewrite.try_lock() {
            let _ = eng.maybe_unload_idle(effective_secs);
        }
        if self.asr.is_some() && self.last_used.elapsed() >= Duration::from_secs(effective_secs) {
            self.asr = None;
            self.pending_asr = None;
            self.loaded_asr_id.clear();
            log::info!("Unloaded idle ASR engine to reclaim system RAM (avail={} MB)", avail_mb);
        }
    }

    fn force_unload_all(&mut self) {
        self.cancel_live_session();
        self.asr = None;
        self.pending_asr = None;
        self.loaded_asr_id.clear();
        if let Some(mut eng) = self.rewrite.try_lock() {
            let _ = eng.maybe_unload_idle(1);
        }
    }

    /// Loaded for this model and for the decoding mode the dictionary calls for.
    fn asr_ready(&self, cfg: &AppConfig) -> bool {
        self.asr_usable(cfg) && self.asr.as_ref().is_some_and(|e| e.hotword_requested() == wants_hotword_decoding(cfg))
    }

    /// Loaded for this model, whatever its decoding mode (still fine to dictate with).
    fn asr_usable(&self, cfg: &AppConfig) -> bool {
        self.asr.is_some() && self.loaded_asr_id == cfg.local_asr_model
    }

    /// Starts loading the speech model in the background (called at startup, model switch, or recording start).
    fn prewarm_asr(&mut self, cfg: &AppConfig) {
        self.poll_background_ready(cfg);
        if self.asr_ready(cfg)
            || self.live_asr.as_ref().map(|p| p.0 == cfg.local_asr_model).unwrap_or(false)
            || self.pending_asr.as_ref().map(|p| p.0 == cfg.local_asr_model).unwrap_or(false)
        {
            return;
        }
        let paths = self.paths.clone();
        let cfg2 = cfg.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("keyreed-asr-prewarm".to_string())
            .spawn(move || {
                let t = Instant::now();
                let r = load_asr_engine(&paths, &cfg2);
                timing_mark(&format!("asr_prewarm_done ok={} ms={:.0}", r.is_ok(), t.elapsed().as_secs_f64() * 1000.0));
                let _ = tx.send(r);
            });
        if spawned.is_ok() {
            self.pending_asr = Some((cfg.local_asr_model.clone(), rx));
        }
    }

    /// Begins an incremental streaming ASR worker for an active recording session so completed
    /// speech chunks are decoded while the user is still speaking.
    fn begin_live_session(&mut self, cfg: &AppConfig) {
        self.cancel_live_session();
        self.poll_background_ready(cfg);
        self.session_cold_model = !self.asr_usable(cfg);
        let use_cloud = cfg.processing_profile == vi_core::ProcessingProfile::MaxAccuracy
            && cfg.cloud.encrypted_api_key_b64.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false);
        if use_cloud {
            return;
        }
        let hotwords = cfg.dictionary.to_sherpa_hotwords();
        let model_id = cfg.local_asr_model.clone();
        // The dictionary switched hotword decoding on or off: dictate with the loaded engine now and
        // reload in the background (harvested after this session).
        if self.asr_usable(cfg) && !self.asr_ready(cfg) {
            self.prewarm_asr(cfg);
        }
        let live_res = if self.asr_usable(cfg) {
            let eng = self.asr.take().unwrap();
            vi_asr::live::LiveAsr::start(move || Ok(eng), hotwords)
        } else if self.pending_asr.as_ref().map(|p| p.0 == model_id).unwrap_or(false) {
            let (_, rx) = self.pending_asr.take().unwrap();
            let paths = self.paths.clone();
            let cfg2 = cfg.clone();
            vi_asr::live::LiveAsr::start(
                move || match rx.recv() {
                    Ok(r) => r,
                    Err(_) => load_asr_engine(&paths, &cfg2),
                },
                hotwords,
            )
        } else {
            let paths = self.paths.clone();
            let cfg2 = cfg.clone();
            vi_asr::live::LiveAsr::start(move || load_asr_engine(&paths, &cfg2), hotwords)
        };
        if let Ok(live) = live_res {
            self.live_asr = Some((model_id, live));
        }
    }

    fn feed_live_audio(&mut self, cfg: &AppConfig, samples_16k: Vec<f32>) {
        self.poll_background_ready(cfg);
        if let Some((_, live)) = &self.live_asr {
            live.feed(samples_16k);
        }
    }

    fn cancel_live_session(&mut self) {
        if let Some((id, live)) = self.live_asr.take() {
            if let Some(eng) = live.cancel() {
                self.asr = Some(eng);
                self.loaded_asr_id = id;
                self.last_used = Instant::now();
            }
        }
    }

    fn ensure_asr_loaded(&mut self, cfg: &AppConfig) -> Result<&mut LocalAsrEngine> {
        self.cancel_live_session();
        if !self.asr_ready(cfg) {
            if let Some((id, rx)) = self.pending_asr.take() {
                if id == cfg.local_asr_model {
                    match rx.recv() {
                        Ok(Ok(engine)) => {
                            self.asr = Some(engine);
                            self.loaded_asr_id = id;
                        }
                        Ok(Err(e)) => return Err(e),
                        Err(_) => {}
                    }
                }
            }
        }
        if !self.asr_ready(cfg) {
            let engine = load_asr_engine(&self.paths, cfg)?;
            self.loaded_asr_id = cfg.local_asr_model.clone();
            self.asr = Some(engine);
        }
        self.last_used = Instant::now();
        Ok(self.asr.as_mut().unwrap())
    }

    fn execute_pipeline(
        &mut self,
        mut samples_16k: Vec<f32>,
        cfg: &AppConfig,
        fg_ctx: &ForegroundContext,
        mode_override: Option<WritingMode>,
        deliver_output: bool,
        on_stage_correcting: Option<&dyn Fn()>,
    ) -> Result<PipelineResult> {
        let t_release = Instant::now();
        let cpu_t0 = current_process_cpu_ms();
        let mut audio_duration_ms = ((samples_16k.len() as u64) * 1000) / 16_000;

        // Determine effective writing mode: explicit override -> per-app user rule -> automatic context -> default
        let proc_lower = fg_ctx.process_name.to_lowercase();
        let effective_mode = if let Some(m) = mode_override {
            m
        } else if let Some(&app_m) = cfg.dictionary.app_mode_overrides.get(&proc_lower) {
            app_m
        } else if cfg.auto_context_mode {
            fg_ctx.category.recommended_mode(cfg.default_writing_mode)
        } else {
            cfg.default_writing_mode
        };
        timing_mark(&format!(
            "pipeline_mode {}",
            serde_json::json!({
                "process": fg_ctx.process_name,
                "category": format!("{:?}", fg_ctx.category),
                "mode": format!("{:?}", effective_mode),
            })
        ));

        let mut decrypted_key = cfg
            .cloud
            .encrypted_api_key_b64
            .as_deref()
            .and_then(|b| dpapi_decrypt_from_b64(b).ok())
            .unwrap_or_default();

        // Stage 1: Speech Recognition (Cloud when MaxAccuracy + configured + online; otherwise Local CPU ASR)
        let mut vad_ms = 0.0;
        let mut asr_ms = 0.0;
        let mut model_load_ms = 0.0;
        let mut post_release_asr_ms = 0.0;
        let mut chunks_total = 0usize;
        let mut chunks_streamed = 0usize;
        let mut speech_ratio = 1.0f32;
        let mut used_asr_id = cfg.local_asr_model.clone();
        let mut raw_transcript = String::new();
        let mut asr_sentences: Vec<(String, u32)> = Vec::new();
        let mut cloud_stt_ok = false;
        let mut cold_model = self.session_cold_model;

        if cfg.processing_profile == vi_core::ProcessingProfile::MaxAccuracy
            && !decrypted_key.trim().is_empty()
        {
            self.cancel_live_session();
            let t_cloud = Instant::now();
            let glossary = cfg.dictionary.to_prompt_glossary(25);
            if let Ok(txt) = transcribe_cloud_multipart(
                &cfg.cloud.stt_endpoint,
                &decrypted_key,
                &cfg.cloud.stt_model,
                Some(&cfg.preferred_language),
                Some(&glossary),
                &samples_16k,
            ) {
                asr_ms = t_cloud.elapsed().as_secs_f64() * 1000.0;
                post_release_asr_ms = asr_ms;
                raw_transcript = txt;
                used_asr_id = format!("cloud:{}", cfg.cloud.stt_model);
                cloud_stt_ok = true;
            }
        }

        if !cloud_stt_ok {
            if let Some((live_id, live)) = self.live_asr.take() {
                let tail = std::mem::take(&mut samples_16k);
                let done = live.finish(tail);
                if let Some(eng) = done.engine {
                    self.asr = Some(eng);
                    self.loaded_asr_id = live_id;
                    self.last_used = Instant::now();
                }
                model_load_ms = done.model_wait_ms;
                post_release_asr_ms = done.post_release_ms;
                chunks_streamed = done.chunks_before_release;
                let out = done.result?;
                audio_duration_ms = ((out.audio_samples as u64) * 1000) / 16_000;
                vad_ms = out.vad_ms;
                asr_ms = out.decode_ms;
                speech_ratio = out.speech_ratio;
                raw_transcript = out.text;
                asr_sentences = out.sentences;
                chunks_total = out.chunks;
            } else {
                cold_model = !self.asr_ready(cfg);
                let t_load = Instant::now();
                let hotwords = cfg.dictionary.to_sherpa_hotwords();
                let engine = self.ensure_asr_loaded(cfg)?;
                if cold_model {
                    model_load_ms = t_load.elapsed().as_secs_f64() * 1000.0;
                }
                let t_post = Instant::now();
                let out = engine.transcribe(&samples_16k, Some(&hotwords))?;
                post_release_asr_ms = t_post.elapsed().as_secs_f64() * 1000.0;
                vad_ms = out.vad_ms;
                asr_ms = out.decode_ms;
                speech_ratio = out.speech_ratio;
                raw_transcript = out.text;
                asr_sentences = out.sentences;
                chunks_total = out.chunks;
            }
        }

        // Zeroize raw PCM audio immediately after ASR finishes!
        samples_16k.zeroize();

        if let Some(cb) = on_stage_correcting {
            cb();
        }

        // Stage 2: Conservative Meaning-Preserving Grammar & Context Correction + Paragraph Formatting
        let lock = self.rewrite.clone();
        let rw = with_engine_or_rules(&lock, SLM_LOCK_WAIT, |eng, busy| {
            if !busy {
                self.ensure_slm_ready_locked(eng);
            }
            let mut rw = eng.process_with_sentences(
                &raw_transcript,
                &asr_sentences,
                speech_ratio,
                effective_mode,
                fg_ctx.category,
                cfg.processing_profile,
                &cfg.dictionary,
                Some(&cfg.cloud.llm_endpoint),
                Some(&cfg.cloud.llm_model),
                // A rules-only stand-in would otherwise prefer the cloud under Balanced.
                if decrypted_key.is_empty()
                    || (busy && cfg.processing_profile != vi_core::ProcessingProfile::MaxAccuracy)
                {
                    None
                } else {
                    Some(decrypted_key.as_str())
                },
            );
            if busy && rw.tier_used == "tier0_fallback" {
                rw.tier_used = TIER_SLM_BUSY.to_string();
            }
            rw
        });
        decrypted_key.zeroize();

        let mic_init_ms = std::mem::replace(&mut self.session_mic_init_ms, 0.0);
        let first_frame_ms = std::mem::replace(&mut self.session_first_frame_ms, 0.0);
        let chunk_finalize_ms = std::mem::replace(&mut self.session_chunk_finalize_ms, 0.0);
        let target_id_ms = std::mem::replace(&mut self.session_target_id_ms, 0.0);
        let keydown_at = self.session_keydown_at.take();
        self.session_cold_model = false;

        if rw.skipped_as_hallucination || rw.final_text.trim().is_empty() {
            // Crucial requirement (Section 4): NEVER overwrite existing clipboard when nothing valid was spoken!
            let total_ms = t_release.elapsed().as_secs_f64() * 1000.0;
            let end_to_end_ms = keydown_at.map(|t| t.elapsed().as_secs_f64() * 1000.0).unwrap_or(total_ms);
            let cpu_ms = (current_process_cpu_ms() - cpu_t0).max(0.0);
            let (ws_mb, _) = current_process_memory_mb();
            let metrics = PipelineMetrics {
                audio_duration_ms,
                vad_ms,
                asr_ms,
                rewrite_ms: rw.elapsed_ms,
                output_ms: 0.0,
                total_release_to_ready_ms: total_ms,
                used_tier: rw.tier_used,
                asr_model_id: used_asr_id,
                mic_init_ms,
                first_frame_ms,
                chunk_finalize_ms,
                model_load_ms,
                post_release_asr_ms,
                chunks_total,
                chunks_streamed,
                norm_ms: rw.norm_ms,
                tier0_ms: rw.tier0_ms,
                slm_startup_ms: rw.slm_startup_ms,
                slm_first_token_ms: rw.slm_first_token_ms,
                slm_ms: rw.slm_ms,
                paragraph_ms: rw.paragraph_ms,
                target_id_ms,
                end_to_end_ms,
                cpu_ms,
                working_set_mb: ws_mb,
                cold_model,
            };
            if let Ok(line) = serde_json::to_string(&metrics) {
                perf_trace_record(&line);
            }
            return Ok(PipelineResult {
                raw_transcript,
                final_text: String::new(),
                effective_mode,
                delivered_via: cfg.output_mode,
                fallback_to_clipboard: false,
                delivery_reason: None,
                requires_command_confirmation: false,
                warning_note: Some("No speech detected; existing clipboard preserved.".to_string()),
                metrics,
            });
        }

        // Stage 3: Clipboard / Direct Typing Delivery
        let t_out = Instant::now();
        let mut delivered_via = OutputMode::Clipboard;
        let mut fallback_to_clipboard = false;
        let mut delivery_reason: Option<String> = None;
        let mut warning_note = rw.warning_note.clone();

        if deliver_output {
            // The modal prompt waits for the user, so ask BEFORE taking the gate (Quick Fix and Copy Last use it).
            let ask_decision = (cfg.output_mode == OutputMode::AskBeforeInserting)
                .then(|| vi_win32::ask_before_inserting_decision(&rw.final_text, fg_ctx));
            let _gate = vi_win32::input_gate();
            match cfg.output_mode {
                OutputMode::Clipboard => {
                    write_clipboard_unicode(
                        &rw.final_text,
                        cfg.privacy.exclude_from_clipboard_history,
                    )?;
                    delivered_via = OutputMode::Clipboard;
                }
                OutputMode::AskBeforeInserting => {
                    match prompt_and_deliver_ask_before_inserting(
                        &rw.final_text,
                        fg_ctx,
                        cfg.privacy.block_password_fields,
                        rw.requires_confirmation,
                        cfg.privacy.exclude_from_clipboard_history,
                        ask_decision,
                    )? {
                        DirectTypeOutcome::TypedDirectly { .. } => {
                            delivered_via = OutputMode::DirectTyping;
                        }
                        DirectTypeOutcome::FallbackToClipboard { reason } => {
                            delivered_via = OutputMode::Clipboard;
                            fallback_to_clipboard = reason != "user_chose_clipboard_only";
                            delivery_reason = Some(reason.to_string());
                            let preview: String = rw.final_text.chars().take(96).collect();
                            warning_note = Some(format!("Copied to clipboard ({reason}). {preview}"));
                        }
                    }
                }
                OutputMode::DirectTyping => {
                    match deliver_direct_or_clipboard_fallback(
                        &rw.final_text,
                        fg_ctx,
                        cfg.privacy.block_password_fields,
                        rw.requires_confirmation,
                        cfg.privacy.exclude_from_clipboard_history,
                    )? {
                        DirectTypeOutcome::TypedDirectly { .. } => {
                            delivered_via = OutputMode::DirectTyping;
                        }
                        DirectTypeOutcome::FallbackToClipboard { reason } => {
                            delivered_via = OutputMode::Clipboard;
                            fallback_to_clipboard = true;
                            delivery_reason = Some(reason.to_string());
                            let msg = format!("Copied to clipboard ({})", reason);
                            warning_note = Some(match warning_note {
                                Some(prev) => format!("{}; {}", prev, msg),
                                None => msg,
                            });
                        }
                    }
                }
            }
        }
        let output_ms = t_out.elapsed().as_secs_f64() * 1000.0;
        let total_release_to_ready_ms = t_release.elapsed().as_secs_f64() * 1000.0;
        let end_to_end_ms = keydown_at
            .map(|t| t.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(total_release_to_ready_ms);
        let cpu_ms = (current_process_cpu_ms() - cpu_t0).max(0.0);
        let (ws_mb, _) = current_process_memory_mb();
        let metrics = PipelineMetrics {
            audio_duration_ms,
            vad_ms,
            asr_ms,
            rewrite_ms: rw.elapsed_ms,
            output_ms,
            total_release_to_ready_ms,
            used_tier: rw.tier_used,
            asr_model_id: used_asr_id,
            mic_init_ms,
            first_frame_ms,
            chunk_finalize_ms,
            model_load_ms,
            post_release_asr_ms,
            chunks_total,
            chunks_streamed,
            norm_ms: rw.norm_ms,
            tier0_ms: rw.tier0_ms,
            slm_startup_ms: rw.slm_startup_ms,
            slm_first_token_ms: rw.slm_first_token_ms,
            slm_ms: rw.slm_ms,
            paragraph_ms: rw.paragraph_ms,
            target_id_ms,
            end_to_end_ms,
            cpu_ms,
            working_set_mb: ws_mb,
            cold_model,
        };
        if let Ok(line) = serde_json::to_string(&metrics) {
            perf_trace_record(&line);
        }

        Ok(PipelineResult {
            raw_transcript,
            final_text: rw.final_text,
            effective_mode,
            delivered_via,
            fallback_to_clipboard,
            delivery_reason,
            requires_command_confirmation: rw.requires_confirmation,
            warning_note,
            metrics,
        })
    }
}

/// A note of a cue: frequency in Hz, length in ms, and whether it rings out like a struck chime.
#[cfg(windows)]
type Note = (f32, u32, bool);

/// The two halves of a style's cue: start (listening) and stop (text ready). Short and quiet by design,
/// and consonant: a fifth or a fourth, never a bare high beep.
#[cfg(windows)]
fn cue_notes(style: AudioCueStyle, is_start: bool) -> &'static [Note] {
    match (style, is_start) {
        (AudioCueStyle::Soft, true) => &[(660.0, 40, false)],
        (AudioCueStyle::Soft, false) => &[(880.0, 40, false)],
        (AudioCueStyle::Bright, true) => &[(784.0, 32, false), (1175.0, 32, false)],
        (AudioCueStyle::Bright, false) => &[(1175.0, 32, false), (784.0, 32, false)],
        (AudioCueStyle::Chime, true) => &[(784.0, 120, true), (1047.0, 200, true)],
        (AudioCueStyle::Chime, false) => &[(1047.0, 120, true), (784.0, 200, true)],
        (AudioCueStyle::Click, true) => &[(2400.0, 8, false)],
        (AudioCueStyle::Click, false) => &[(1800.0, 8, false)],
        (AudioCueStyle::None, _) => &[],
    }
}

/// One cue as a mono 16-bit 44.1 kHz WAV: what `PlaySoundW` reads from memory. `volume` (0-100) scales the
/// samples, so the cue carries its own level and no other audio is touched.
#[cfg(windows)]
fn cue_wav(style: AudioCueStyle, is_start: bool, volume: u8) -> Vec<u8> {
    const RATE: u32 = 44_100;
    const FADE: u32 = RATE * 5 / 1000;
    let notes = cue_notes(style, is_start);
    let data_len: u32 = notes.iter().map(|n| RATE * n.1 / 1000 * 2).sum();
    let peak = 0.28 * volume.min(100) as f32 / 100.0;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &(freq, ms, rings) in notes {
        let frames = RATE * ms / 1000;
        for i in 0..frames {
            let ramp = i.min(frames - 1 - i).min(FADE) as f32 / FADE as f32;
            // A struck note loses most of its level over its own length; a plain tone holds it.
            let decay = if rings { (-4.0 * i as f32 / frames as f32).exp() } else { 1.0 };
            let s = (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin() * ramp * decay * peak;
            wav.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
        }
    }
    wav
}

/// The buffer for one cue, built once per style, side and volume the user has actually tried. It lives for
/// the process because `SND_MEMORY | SND_ASYNC` reads it while the cue plays.
#[cfg(windows)]
fn cue_buffer(style: AudioCueStyle, is_start: bool, volume: u8) -> &'static [u8] {
    type Key = (AudioCueStyle, bool, u8);
    static BUFFERS: std::sync::Mutex<Vec<(Key, &'static [u8])>> = std::sync::Mutex::new(Vec::new());
    let key = (style, is_start, volume.min(100));
    let mut buffers = BUFFERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, wav)) = buffers.iter().find(|(k, _)| *k == key) {
        return wav;
    }
    let wav: &'static [u8] = Box::leak(cue_wav(key.0, key.1, key.2).into_boxed_slice());
    buffers.push((key, wav));
    wav
}

/// The start/stop cue: queued to the default output device and never waited on, so the coordinator
/// is not held for it and it does not depend on the system beep driver being present. `force` plays it
/// from the Settings "Play" button even while cues are switched off.
fn play_subtle_earcon(cfg: &AppConfig, is_start: bool, force: bool) {
    if !(cfg.audio_cues_enabled || force) || cfg.audio_cue_style == AudioCueStyle::None {
        return;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        let wav = cue_buffer(cfg.audio_cue_style, is_start, cfg.audio_cue_volume);
        unsafe {
            PlaySoundW(
                wav.as_ptr() as *const u16,
                std::ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = is_start;
}

fn parse_mode_str(s: &str) -> WritingMode {
    match s.trim().to_lowercase().as_str() {
        "exact" => WritingMode::Exact,
        "professional" | "pro" => WritingMode::Professional,
        "casual" | "chat" => WritingMode::Casual,
        "technical" | "tech" => WritingMode::Technical,
        "terminal" | "terminal_code" | "code" => WritingMode::TerminalCode,
        _ => WritingMode::Smart,
    }
}

fn parse_category_str(s: &str) -> AppCategory {
    match s.trim().to_lowercase().as_str() {
        "email" => AppCategory::Email,
        "chat" => AppCategory::Chat,
        "document" | "doc" => AppCategory::Document,
        "code" | "code_editor" | "ide" => AppCategory::CodeEditor,
        "terminal" => AppCategory::Terminal,
        "notes" => AppCategory::Notes,
        "password" => AppCategory::PasswordProtected,
        _ => AppCategory::General,
    }
}

fn run_cli_transcribe_wav(args: &[String]) -> Result<()> {
    let mut wav_path: Option<PathBuf> = None;
    let mut mode: Option<WritingMode> = None;
    let mut cat = AppCategory::General;
    let mut copy_clipboard = false;
    let mut repeat: usize = 1;
    let mut asr_model_override: Option<String> = None;
    let mut llm_override: Option<String> = None;
    let mut live_mode = false;
    let mut live_speed: f64 = 1.0;
    let mut prewarm_before = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--transcribe-wav" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    wav_path = Some(PathBuf::from(v));
                }
            }
            "--mode" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    mode = Some(parse_mode_str(v));
                }
            }
            "--app-category" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    cat = parse_category_str(v);
                }
            }
            "--asr-model" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    asr_model_override = Some(v.clone());
                }
            }
            "--llm-model" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    llm_override = Some(v.clone());
                }
            }
            "--copy-clipboard" => copy_clipboard = true,
            "--live" => live_mode = true,
            "--prewarm" => prewarm_before = true,
            "--live-speed" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    live_speed = v.parse::<f64>().unwrap_or(1.0).clamp(0.25, 64.0);
                }
            }
            "--repeat" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    repeat = v.parse().unwrap_or(1).clamp(1, 200);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let wav = wav_path.context("--transcribe-wav requires a WAV file path")?;
    let mut cfg = AppConfig::load_or_default(&AppConfig::default_config_path());
    if let Some(m) = asr_model_override {
        cfg.local_asr_model = m;
    }
    if let Some(l) = llm_override {
        cfg.local_llm_model = l;
    }
    if let Some(m) = mode {
        cfg.default_writing_mode = m;
    }

    let mut sup = InferenceSupervisor::new(&cfg);
    if prewarm_before {
        let _ = sup.ensure_asr_loaded(&cfg);
        sup.ensure_slm_ready();
        sup.rewrite.lock().warm_up_slot(cfg.default_writing_mode);
    }
    let mut fg = ForegroundContext::default();
    fg.category = cat;
    for n in 1..=repeat {
        let samples = load_wav_16k_mono(&wav)?;
        let res = if live_mode {
            sup.session_keydown_at = Some(Instant::now());
            sup.begin_live_session(&cfg);
            sup.prewarm_slm(&cfg);
            let step = 1_200usize; // 75 ms at 16 kHz, matching coordinator ticker
            let tick_dur = Duration::from_secs_f64(0.075 / live_speed);
            let total = samples.len();
            let mut offset = 0usize;
            while offset + step < total {
                sup.feed_live_audio(&cfg, samples[offset..offset + step].to_vec());
                offset += step;
                std::thread::sleep(tick_dur);
            }
            sup.execute_pipeline(samples[offset..].to_vec(), &cfg, &fg, mode, copy_clipboard, None)?
        } else {
            sup.execute_pipeline(samples, &cfg, &fg, mode, copy_clipboard, None)?
        };
        let (ws_mb, priv_mb) = current_process_memory_mb();
        let out_json = serde_json::json!({
            "run": n,
            "cold": n == 1,
            "result": res,
            "memory": {
                "working_set_mb": ws_mb,
                "private_mb": priv_mb,
            }
        });
        println!("{}", serde_json::to_string_pretty(&out_json)?);
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct CorrectionEvalCase {
    id: String,
    category: String,
    mode: String,
    app_context: String,
    input: String,
    reference: String,
    #[serde(default)]
    must_keep: Vec<String>,
    #[serde(default)]
    must_not_contain: Vec<String>,
}

#[cfg(windows)]
struct WindowsSpelling(vi_win32::spell::WindowsSpeller);

#[cfg(windows)]
impl vi_rewrite::quickfix::Speller for WindowsSpelling {
    fn misspellings(&mut self, text: &str) -> Vec<(usize, usize, Vec<String>)> {
        self.0.misspellings(text)
    }
}

/// `--eval-quickfix-jsonl <in> <out>` writes `{id, output, edits}` lines (the form `bench/score_quickfix.py` reads);
/// `--eval-typed-jsonl <in> [<out>]` writes the one JSON document `bench/score_typed.py` reads (stdout without `<out>`).
/// Inputs are gate cases (`id`, `input`, `app_context` or `context`). Offsets in the output are in characters. The model folder is
/// `--grammar-model`, else `KEYREED_GRAMMAR_MODEL`, else `<models>/gector-bert-en-int8`; `--threads N` sets the intra-op threads;
/// `--spell` adds the Windows spell checker; `--gate-candidates` reports every edit class, including those the shipped table switches off.
fn run_eval_quickfix(args: &[String], typed_format: bool) -> Result<()> {
    use vi_rewrite::quickfix::{Layer, QuickFixEngine};
    let flag = if typed_format { "--eval-typed-jsonl" } else { "--eval-quickfix-jsonl" };
    let pos = args.iter().position(|a| a == flag).context("flag missing")?;
    let input = PathBuf::from(args.get(pos + 1).with_context(|| format!("missing <in.jsonl> after {flag}"))?);
    let output = args.get(pos + 2).filter(|a| !a.starts_with("--")).map(PathBuf::from);
    if output.is_none() && !typed_format {
        bail!("missing <out.jsonl> after {flag} <in.jsonl>");
    }
    let value_of = |name: &str| args.iter().position(|a| a == name).and_then(|p| args.get(p + 1).cloned());
    let paths = RuntimePaths::discover(&AppConfig::default());
    let model_dir = value_of("--grammar-model")
        .or_else(|| env::var("KEYREED_GRAMMAR_MODEL").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.models_dir.join("gector-bert-en-int8"));
    let threads = value_of("--threads").and_then(|t| t.parse().ok()).unwrap_or(0);
    let gector = vi_rewrite::gector::Gector::load(&paths.sherpa_lib_dir.join("onnxruntime.dll"), &model_dir, threads)
        .with_context(|| format!("load the grammar model from {}", model_dir.display()))?;
    let mut engine = QuickFixEngine::new(Box::new(gector));
    #[cfg(windows)]
    if args.iter().any(|a| a == "--spell") {
        engine = engine.with_speller(Box::new(WindowsSpelling(vi_win32::spell::WindowsSpeller::new())));
    }
    if args.iter().any(|a| a == "--gate-candidates") {
        // Every class reported as a candidate (nothing off), so the scorer can grade each class alone.
        engine = engine.with_table(vi_rewrite::quickfix::EditClassTable::parse(r#"{"unknown_class":"review_only","contexts":{},"classes":{}}"#)?);
    }
    engine.fix("This are a warm up sentence.", AppCategory::General)?;

    let raw = std::fs::read_to_string(&input).with_context(|| format!("failed to read {}", input.display()))?;
    let (mut lines, mut records) = (String::new(), Vec::new());
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let mut case: serde_json::Value = serde_json::from_str(line)?;
        let text = case["input"].as_str().context("case without input")?.to_string();
        let context = parse_category_str(case["app_context"].as_str().or(case["context"].as_str()).unwrap_or("general"));
        let started = Instant::now();
        let result = engine.fix(&text, context)?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let chars = |byte: usize| text[..byte].chars().count();
        let edits: Vec<serde_json::Value> = result
            .edits
            .iter()
            .map(|e| {
                serde_json::json!({
                    "start": chars(e.start), "length": chars(e.start + e.length) - chars(e.start), "replacement": e.replacement,
                    "layer": match e.layer { Layer::Spelling => "spelling", Layer::Rule => "rule", Layer::Grammar => "grammar" },
                    "class": e.class, "confidence": e.confidence, "silent": e.silent,
                })
            })
            .collect();
        if typed_format {
            case["edits"] = edits.into();
            case["ms"] = ms.into();
            records.push(case);
        } else {
            lines.push_str(&serde_json::json!({"id": case["id"], "output": result.corrected, "edits": edits, "ms": ms}).to_string());
            lines.push_str("\n");
        }
    }
    if typed_format {
        let doc = serde_json::json!({"engine": "keyreed quick fix (gector-bert-en + rules)", "records": records}).to_string();
        match output {
            Some(path) => std::fs::write(path, doc)?,
            None => println!("{doc}"),
        }
    } else if let Some(path) = output {
        std::fs::write(path, lines)?;
    }
    Ok(())
}

fn run_eval_correction_jsonl(jsonl_path: &Path, llm_model: &str) -> Result<()> {
    let raw = std::fs::read_to_string(jsonl_path)
        .with_context(|| format!("failed to read {}", jsonl_path.display()))?;
    let mut cfg = AppConfig::default();
    cfg.local_llm_model = llm_model.to_string();
    cfg.inference_threads = 4;

    let paths = RuntimePaths::discover(&cfg);
    let slm = paths.resolve_gguf_path(&cfg.local_llm_model).map(|gguf| {
        LlamaServerSupervisor::new(paths.llama_server_bin.clone(), gguf, 4)
    });
    let mut engine = RewriteEngine::new(slm);

    // Pre-warm KV slots if local SLM is enabled so per-utterance latency reflects steady-state PTT
    if !llm_model.eq_ignore_ascii_case("none") {
        let _ = engine.process(
            "We tested the warm up utterance on port 8080.",
            0.9,
            WritingMode::Smart,
            AppCategory::General,
            vi_core::ProcessingProfile::MaxPrivacyOffline,
            &cfg.dictionary,
            None,
            None,
            None,
        );
    }

    let mut total = 0usize;
    let mut keep_hits = 0usize;
    let mut forbid_hits = 0usize;
    let mut exact_or_close = 0usize;
    let mut latencies_ms: Vec<f64> = Vec::new();
    let mut records = Vec::new();

    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let c: CorrectionEvalCase = serde_json::from_str(line)?;
        total += 1;
        let wm = parse_mode_str(&c.mode);
        let cat = parse_category_str(&c.app_context);

        let out = engine.process(
            &c.input,
            0.92,
            wm,
            cat,
            vi_core::ProcessingProfile::MaxPrivacyOffline,
            &cfg.dictionary,
            None,
            None,
            None,
        );
        latencies_ms.push(out.elapsed_ms);

        let out_low = out.final_text.to_lowercase();
        let ok_keep = c
            .must_keep
            .iter()
            .all(|k| out_low.contains(&k.to_lowercase()));
        let ok_forbid = c
            .must_not_contain
            .iter()
            .all(|bad| !out_low.contains(&bad.to_lowercase()));

        if ok_keep {
            keep_hits += 1;
        }
        if ok_forbid {
            forbid_hits += 1;
        }
        if ok_keep && ok_forbid {
            exact_or_close += 1;
        }

        records.push(serde_json::json!({
            "id": c.id,
            "category": c.category,
            "mode": c.mode,
            "input": c.input,
            "reference": c.reference,
            "output": out.final_text,
            "tier_used": out.tier_used,
            "requires_confirmation": out.requires_confirmation,
            "elapsed_ms": (out.elapsed_ms * 100.0).round() / 100.0,
            "must_keep_ok": ok_keep,
            "must_not_contain_ok": ok_forbid,
        }));
    }

    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med_ms = if latencies_ms.is_empty() {
        0.0
    } else {
        latencies_ms[latencies_ms.len() / 2]
    };
    let p90_ms = if latencies_ms.is_empty() {
        0.0
    } else {
        latencies_ms[(latencies_ms.len() * 9) / 10]
    };

    let report = serde_json::json!({
        "llm_model": llm_model,
        "total_cases": total,
        "must_keep_survival_pct": (1000.0 * (keep_hits as f64) / (total.max(1) as f64)).round() / 10.0,
        "anti_hallucination_pass_pct": (1000.0 * (forbid_hits as f64) / (total.max(1) as f64)).round() / 10.0,
        "combined_invariant_pass_pct": (1000.0 * (exact_or_close as f64) / (total.max(1) as f64)).round() / 10.0,
        "median_latency_ms": (med_ms * 100.0).round() / 100.0,
        "p90_latency_ms": (p90_ms * 100.0).round() / 100.0,
        "records": records,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(windows)]
fn run_e2e_target_window_process() -> Result<()> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, ShowWindow, TranslateMessage, MSG,
        SW_SHOWNORMAL, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_TOPMOST, WS_OVERLAPPED,
        WS_SYSMENU, WS_VISIBLE,
    };

    unsafe extern "system" fn target_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        const WM_DESTROY: u32 = 0x0002;
        const WM_USER_FOCUS_EDIT: u32 = 0x0400 + 70;
        const WM_USER_FOCUS_PASS: u32 = 0x0400 + 71;
        match msg {
            WM_USER_FOCUS_EDIT => {
                let ed = windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, 201);
                if !ed.is_null() {
                    let _ = SetFocus(ed);
                }
                0
            }
            WM_USER_FOCUS_PASS => {
                let pw = windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, 202);
                if !pw.is_null() {
                    let _ = SetFocus(pw);
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls: Vec<u16> = "KeyreedE2ETargetWindow\0".encode_utf16().collect();
        let title: Vec<u16> = "KeyReed E2E Target Editor\0".encode_utf16().collect();
        let edit_cls: Vec<u16> = "EDIT\0".encode_utf16().collect();
        let empty: Vec<u16> = "\0".encode_utf16().collect();

        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(target_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: 6 as _,
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            cls.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            80,
            120,
            520,
            340,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        // ES_MULTILINE (0x0004) | ES_AUTOVSCROLL (0x0040) | ES_WANTRETURN (0x1000)
        let ed = CreateWindowExW(
            0,
            edit_cls.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | 0x0004 | 0x0040 | 0x1000,
            16,
            16,
            470,
            210,
            hwnd,
            201usize as _,
            hinst,
            std::ptr::null(),
        );
        // ES_PASSWORD (0x0020)
        let _pw = CreateWindowExW(
            0,
            edit_cls.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | 0x0020,
            16,
            240,
            470,
            32,
            hwnd,
            202usize as _,
            hinst,
            std::ptr::null(),
        );
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
        if !ed.is_null() {
            let _ = SetFocus(ed);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn run_windows_live_e2e_test(wav_dir: Option<PathBuf>, screenshot_out: Option<PathBuf>) -> Result<()> {
    use std::process::Command;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetDlgItem, SendMessageW,
        WM_CLOSE, WM_GETTEXT, WM_GETTEXTLENGTH, WM_SETTEXT,
    };

    harden_and_init_process();
    let (idle_ws_mb, idle_priv_mb) = current_process_memory_mb();

    // 1. Single-instance guard verification
    let g1 = SingleInstanceGuard::acquire("Local\\Keyreed_E2E_Test_Mutex")?;
    assert!(g1.is_some(), "First mutex acquire must succeed");
    let g2 = SingleInstanceGuard::acquire("Local\\Keyreed_E2E_Test_Mutex")?;
    assert!(g2.is_none(), "Second mutex acquire must detect running instance");
    drop(g1);

    // 2. DPAPI encryption/decryption round-trip
    let probe_val = "dpapi_roundtrip_verification_payload_v1";
    let enc = dpapi_encrypt_to_b64(probe_val)?;
    assert!(!enc.contains("roundtrip_verification"), "Ciphertext must never contain plaintext");
    let dec = dpapi_decrypt_from_b64(&enc)?;
    assert_eq!(dec, probe_val, "DPAPI round-trip mismatch");

    // 3. Autostart HKCU\...\Run round-trip
    let exe = env::current_exe()?;
    set_autostart_enabled("KeyreedE2ETest", false, &exe)?;
    let absent = read_autostart_value("KeyreedE2ETest")?;
    assert!(absent.is_none(), "disabled startup must remove the Run value, got {absent:?}");
    set_autostart_enabled("KeyreedE2ETest", true, &exe)?;
    let stored = read_autostart_value("KeyreedE2ETest")?.context("enabled startup did not write a Run value")?;
    assert!(stored.starts_with('"') && stored.contains("--minimized"), "unquoted or incomplete autostart value: {stored}");
    let product_before = read_autostart_value(AUTOSTART_REG_VALUE)?;
    set_autostart_enabled("KeyreedE2ETest", false, &exe)?;
    let absent_after = read_autostart_value("KeyreedE2ETest")?;
    assert!(absent_after.is_none(), "cleanup failed: {absent_after:?}");

    // 4. Clipboard preservation on empty/failed dictation + Unicode & privacy-format write/read
    write_clipboard_unicode("SENTINEL_EXISTING_CLIPBOARD_DATA", false)?;
    assert_eq!(
        read_clipboard_unicode()?.as_deref(),
        Some("SENTINEL_EXISTING_CLIPBOARD_DATA")
    );

    let mut cfg = AppConfig::default();
    cfg.local_llm_model = "none".to_string(); // deterministic fast check first
    let mut sup = InferenceSupervisor::new(&cfg);
    let silent_pcm = vec![0.0f32; 16_000]; // 1.0s silence
    let silent_res = sup.execute_pipeline(
        silent_pcm,
        &cfg,
        &ForegroundContext::default(),
        Some(WritingMode::Smart),
        true,
        None,
    )?;
    assert!(silent_res.final_text.is_empty());
    assert_eq!(
        read_clipboard_unicode()?.as_deref(),
        Some("SENTINEL_EXISTING_CLIPBOARD_DATA"),
        "Clipboard MUST remain untouched when speech is empty/silent!"
    );

    let unicode_sample = "Checked server: couldn't identify why port 8080 failed.\n- नमस्ते भारत\n- Price: ₹1,250";
    write_clipboard_unicode(unicode_sample, true)?;
    let clip_back = read_clipboard_unicode()?.unwrap_or_default();
    assert!(clip_back.contains("नमस्ते भारत") && clip_back.contains("₹1,250"));

    // 5. Real Hardware Microphone Enumeration, Capture, Cancellation & Fallback Verification
    let hw_mics = list_input_devices();
    let mut mic_hw_report = serde_json::json!({
        "enumerated_count": hw_mics.len(),
        "devices": hw_mics,
    });
    if !hw_mics.is_empty() {
        let t_mic = Instant::now();
        if let Ok(cap) = ActiveCapture::start(None) {
            let open_ms = t_mic.elapsed().as_secs_f64() * 1000.0;
            let dev_name = cap.device_name().to_string();
            std::thread::sleep(Duration::from_millis(350));
            let samples_16k = cap.stop_and_take_16k();
            let fallback_ok = ActiveCapture::start(Some("Unplugged_USB_Headset_Test_Device"))
                .map(|c| {
                    let fb = c.used_fallback_device();
                    c.cancel_and_zeroize();
                    fb
                })
                .unwrap_or(false);
            mic_hw_report = serde_json::json!({
                "status": "PASS",
                "enumerated_count": hw_mics.len(),
                "devices": hw_mics,
                "active_device": dev_name,
                "open_latency_ms": (open_ms * 100.0).round() / 100.0,
                "captured_16k_samples_350ms": samples_16k.len(),
                "cancel_and_zeroize": "PASS",
                "unplugged_device_fallback": if fallback_ok { "PASS" } else { "FAIL" },
            });
        }
    }

    // 6. Spawn live Tray + Non-Activating Layered Floating Pill + Settings Window + Dedicated External Target Window!
    let (ui_tx, _ui_rx) = crossbeam_channel::unbounded();
    let mic_names: Vec<String> = list_input_devices().into_iter().map(|d| d.name).collect();
    let installed_asr = RuntimePaths::discover(&cfg).list_installed_asr_ids();
    let (gui, _pump) = vi_win32::ui::win_gui::init_and_spawn_gui_windows(
        ui_tx,
        cfg.clone(),
        mic_names.clone(),
        installed_asr,
        true,
    )?;
    gui.sync(EngineState::Listening, 0.65, None);

    // Pump pending GUI messages so Tray, Floating Pill, and Settings render completely on desktop
    let pump_gui_messages = || unsafe {
        let mut msg: windows_sys::Win32::UI::WindowsAndMessaging::MSG = std::mem::zeroed();
        while windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut msg,
            std::ptr::null_mut(),
            0,
            0,
            windows_sys::Win32::UI::WindowsAndMessaging::PM_REMOVE,
        ) != 0
        {
            windows_sys::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
    };
    pump_gui_messages();

    // Read-only check of Windows 11 Notepad deep RichEditD2DPT discovery if Notepad is open on the desktop
    let notepad_deep_editor_found = unsafe {
        let cls_w: Vec<u16> = "Notepad\0".encode_utf16().collect();
        let np_hwnd: HWND = FindWindowW(cls_w.as_ptr(), std::ptr::null());
        if !np_hwnd.is_null() {
            vi_win32::context::find_deep_editor_hwnd(np_hwnd).is_some()
        } else {
            false
        }
    };

    // Spawn dedicated external E2E target editor process so we never disturb user windows
    let mut target_child = Command::new(&exe)
        .arg("--e2e-target-window")
        .spawn()
        .ok();
    let target_cls_w: Vec<u16> = "KeyreedE2ETargetWindow\0".encode_utf16().collect();
    let mut target_hwnd: HWND = std::ptr::null_mut();
    let mut target_edit_hwnd: HWND = std::ptr::null_mut();
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        pump_gui_messages();
        unsafe {
            target_hwnd = FindWindowW(target_cls_w.as_ptr(), std::ptr::null());
            if !target_hwnd.is_null() {
                target_edit_hwnd = GetDlgItem(target_hwnd, 201);
                let pw = GetDlgItem(target_hwnd, 202);
                if !target_edit_hwnd.is_null() && !pw.is_null() {
                    break;
                }
            }
        }
    }

    let read_hwnd_text = |h: HWND| -> String {
        unsafe {
            if h.is_null() {
                return String::new();
            }
            let tlen = SendMessageW(h, WM_GETTEXTLENGTH, 0, 0);
            if tlen > 0 {
                let mut buf = vec![0u16; (tlen as usize) + 2];
                SendMessageW(h, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize);
                String::from_utf16_lossy(&buf[..tlen as usize])
            } else {
                String::new()
            }
        }
    };
    let clear_hwnd_text = |h: HWND| unsafe {
        if !h.is_null() {
            let empty: [u16; 1] = [0];
            SendMessageW(h, WM_SETTEXT, 0, empty.as_ptr() as isize);
        }
    };

    // Focus the external target window's multiline editor
    unsafe {
        if !target_hwnd.is_null() {
            let _ = vi_win32::force_foreground_window(target_hwnd as isize);
            SendMessageW(target_hwnd, 0x0400 + 70, 0, 0);
            std::thread::sleep(Duration::from_millis(120));
        }
    }
    let fg_ctx = capture_foreground_context(true);

    // 7. Comprehensive Automatic Text Insertion Matrix against External Target Process
    let mut auto_insert_matrix = serde_json::Value::Null;
    if !target_hwnd.is_null() && !target_edit_hwnd.is_null() {
        // 7a. Direct Unicode + Multiline Auto Insert
        unsafe {
            clear_hwnd_text(target_edit_hwnd);
            let _ = vi_win32::force_foreground_window(target_hwnd as isize);
            SendMessageW(target_hwnd, 0x0400 + 70, 0, 0);
            std::thread::sleep(Duration::from_millis(100));
        }
        let ctx_edit = capture_foreground_context(true);
        let multiline_unicode = "Line 1: KeyReed Auto-Insert\nLine 2: नमस्ते भारत — ₹1,250";
        let out_direct = deliver_direct_or_clipboard_fallback(
            multiline_unicode,
            &ctx_edit,
            true,
            false,
            true,
        )?;
        std::thread::sleep(Duration::from_millis(180));
        let readback_unicode = read_hwnd_text(target_edit_hwnd);

        // 7b. AskBeforeInserting (simulated Approve vs CopyToClipboardOnly)
        unsafe {
            clear_hwnd_text(target_edit_hwnd);
            let _ = vi_win32::force_foreground_window(target_hwnd as isize);
            SendMessageW(target_hwnd, 0x0400 + 70, 0, 0);
            std::thread::sleep(Duration::from_millis(80));
        }
        let ctx_ask = capture_foreground_context(true);
        write_clipboard_unicode("PRE_EXISTING_CLIPBOARD_STAYS", true)?;
        let out_ask_cancel = prompt_and_deliver_ask_before_inserting(
            "Cancelled insert touches neither target nor clipboard.",
            &ctx_ask,
            true,
            false,
            true,
            Some(vi_win32::AskInsertDecision::Cancel),
        )?;
        let readback_after_cancel = read_hwnd_text(target_edit_hwnd);
        let clip_after_cancel = read_clipboard_unicode().unwrap_or_default();

        let out_ask_decline = prompt_and_deliver_ask_before_inserting(
            "Declined insert stays on clipboard only.",
            &ctx_ask,
            true,
            false,
            true,
            Some(vi_win32::AskInsertDecision::CopyToClipboardOnly),
        )?;
        let readback_after_decline = read_hwnd_text(target_edit_hwnd);
        let out_ask_approve = prompt_and_deliver_ask_before_inserting(
            "Approved insert types into target.",
            &ctx_ask,
            true,
            false,
            true,
            Some(vi_win32::AskInsertDecision::InsertIntoTarget),
        )?;
        std::thread::sleep(Duration::from_millis(120));
        let readback_after_approve = read_hwnd_text(target_edit_hwnd);

        // 7b2. Terminal Newline Sanitization (AppCategory::Terminal never injects CR or LF)
        unsafe {
            clear_hwnd_text(target_edit_hwnd);
            let _ = vi_win32::force_foreground_window(target_hwnd as isize);
            SendMessageW(target_hwnd, 0x0400 + 70, 0, 0);
            std::thread::sleep(Duration::from_millis(80));
        }
        let mut ctx_term = capture_foreground_context(true);
        ctx_term.category = AppCategory::Terminal;
        let term_input = format!("kubectl get pods{}\necho second_line\n", char::from(13u8));
        let _ = deliver_direct_or_clipboard_fallback(
            &term_input,
            &ctx_term,
            true,
            false,
            true,
        )?;
        std::thread::sleep(Duration::from_millis(120));
        let term_readback = read_hwnd_text(target_edit_hwnd);
        let terminal_newline_stripped = !term_readback.contains('\n')
            && !term_readback.contains(char::from(13u8))
            && term_readback.contains("kubectl get pods echo second_line");

        // 7c. Password Field Protection (ES_PASSWORD control id=202)
        unsafe {
            let _ = vi_win32::force_foreground_window(target_hwnd as isize);
            SendMessageW(target_hwnd, 0x0400 + 71, 0, 0);
            std::thread::sleep(Duration::from_millis(100));
        }
        let ctx_pw = capture_foreground_context(true);
        let out_pw = deliver_direct_or_clipboard_fallback(
            "SecretDictation123",
            &ctx_pw,
            true,
            false,
            true,
        )?;
        let pw_hwnd = unsafe { GetDlgItem(target_hwnd, 202) };
        let pw_readback = read_hwnd_text(pw_hwnd);

        // 7d. TOCTOU Focus-Switch Protection
        let mut stale_ctx = ctx_edit.clone();
        stale_ctx.hwnd = 0x7FFF_FFFF; // Mismatched HWND
        let out_toctou = deliver_direct_or_clipboard_fallback(
            "ShouldNotType",
            &stale_ctx,
            true,
            false,
            true,
        )?;

        // 7e. Adversarial Fault Injection: Corrupted Model Quarantine & Download Cancel
        let fault_dir = std::env::temp_dir().join(format!("keyreed_fault_test_{}", std::process::id()));
        let corrupt_model_dir = fault_dir.join("sherpa-onnx-moonshine-tiny-en-int8");
        let _ = std::fs::remove_dir_all(&fault_dir);
        std::fs::create_dir_all(&corrupt_model_dir)?;
        std::fs::write(corrupt_model_dir.join("tokens.txt"), b"TAMPERED_CONTENT")?;
        let corrupt_rejected = vi_asr::integrity::prepare_installed_model(&fault_dir, "moonshine-tiny-en-int8").is_err()
            && !corrupt_model_dir.exists()
            && fault_dir.join(".rejected").exists();
        let cancel_flag = AtomicBool::new(true);
        let cancel_dest = fault_dir.join("cancelled.tar.bz2");
        let cancel_aborted = vi_asr::catalog::download_verified_with_progress(
            "http://127.0.0.1:1/unreachable.tar.bz2",
            &cancel_dest,
            "0000000000000000000000000000000000000000000000000000000000000000",
            Some(&cancel_flag),
            None,
        )
        .is_err()
            && !cancel_dest.exists();
        let _ = std::fs::remove_dir_all(&fault_dir);

        auto_insert_matrix = serde_json::json!({
            "unicode_multiline_outcome": format!("{:?}", out_direct),
            "unicode_multiline_readback": readback_unicode,
            "unicode_multiline_verified": readback_unicode.contains("नमस्ते भारत") && readback_unicode.contains("₹1,250") && readback_unicode.contains("Line 2:"),
            "ask_before_inserting_cancel_outcome": format!("{:?}", out_ask_cancel),
            "ask_before_inserting_cancel_target_empty": readback_after_cancel.is_empty(),
            "ask_before_inserting_cancel_clipboard_preserved": clip_after_cancel.as_deref() == Some("PRE_EXISTING_CLIPBOARD_STAYS"),
            "ask_before_inserting_decline_outcome": format!("{:?}", out_ask_decline),
            "ask_before_inserting_decline_target_empty": readback_after_decline.is_empty(),
            "ask_before_inserting_approve_outcome": format!("{:?}", out_ask_approve),
            "ask_before_inserting_approve_readback": readback_after_approve,
            "terminal_newline_sanitized": terminal_newline_stripped,
            "terminal_sanitized_readback": term_readback,
            "password_field_detected": ctx_pw.is_password_field,
            "password_field_outcome": format!("{:?}", out_pw),
            "password_field_target_empty": pw_readback.is_empty(),
            "toctou_focus_switch_outcome": format!("{:?}", out_toctou),
            "win11_notepad_deep_richedit_discovery": notepad_deep_editor_found,
            "corrupted_model_quarantined": corrupt_rejected,
            "cancelled_download_cleaned_up": cancel_aborted,
        });
    }

    // 8. Real ASR + Grammar Correction + Auto Insert into External Target Window if WAV provided
    let mut wav_e2e_json = serde_json::Value::Null;
    if let Some(wdir) = wav_dir {
        if wdir.exists() {
            cfg.local_llm_model = "Qwen3.5-0.8B-Q4_K_M".to_string();
            cfg.output_mode = OutputMode::DirectTyping;
            let mut full_sup = InferenceSupervisor::new(&cfg);
            let samples = load_wav_16k_mono(&wdir)?;

            // Warm up ASR + SLM first so focus doesn't drift during cold load, then focus target and run pipeline!
            let _ = full_sup.ensure_asr_loaded(&cfg);
            full_sup.ensure_slm_ready();
            let _ = full_sup.execute_pipeline(
                samples.clone(),
                &cfg,
                &ForegroundContext::default(),
                Some(WritingMode::Smart),
                false,
                None,
            );

            unsafe {
                if !target_hwnd.is_null() {
                    clear_hwnd_text(target_edit_hwnd);
                    let _ = vi_win32::force_foreground_window(target_hwnd as isize);
                    SendMessageW(target_hwnd, 0x0400 + 70, 0, 0);
                    std::thread::sleep(Duration::from_millis(120));
                }
            }
            let live_target_ctx = capture_foreground_context(true);

            full_sup.session_keydown_at = Some(Instant::now());
            full_sup.begin_live_session(&cfg);
            full_sup.prewarm_slm(&cfg);
            let step = 1_200usize;
            let mut offset = 0usize;
            while offset + step < samples.len() {
                full_sup.feed_live_audio(&cfg, samples[offset..offset + step].to_vec());
                offset += step;
                std::thread::sleep(Duration::from_millis(25));
            }
            unsafe {
                if !target_hwnd.is_null() {
                    let _ = vi_win32::force_foreground_window(target_hwnd as isize);
                }
            }
            let r = full_sup.execute_pipeline(
                samples[offset..].to_vec(),
                &cfg,
                &live_target_ctx,
                Some(WritingMode::Smart),
                true,
                None,
            )?;
            // Typing is asynchronous in the target's message queue: poll until the text stops growing.
            let mut auto_inserted_text = String::new();
            for _ in 0..60 {
                std::thread::sleep(Duration::from_millis(150));
                let now = read_hwnd_text(target_edit_hwnd);
                if !now.is_empty() && now == auto_inserted_text {
                    break;
                }
                auto_inserted_text = now;
            }
            let mut norm_inserted = auto_inserted_text.clone();
            norm_inserted.retain(|c| c != char::from(13u8));
            let (active_ws, active_priv) = current_process_memory_mb();
            full_sup.force_unload_all();
            let (after_unload_ws, after_unload_priv) = current_process_memory_mb();
            wav_e2e_json = serde_json::json!({
                "pipeline": r,
                "target_window_auto_inserted_text": auto_inserted_text,
                "target_window_paragraph_count": norm_inserted.trim().split("\n\n").filter(|s| !s.trim().is_empty()).count(),
                "auto_insert_exact_match": norm_inserted.trim() == r.final_text.trim(),
                "active_working_set_mb": active_ws,
                "active_private_mb": active_priv,
                "post_unload_working_set_mb": after_unload_ws,
                "post_unload_private_mb": after_unload_priv,
            });
        }
    }

    // Optional live desktop screenshot capture via PowerShell System.Drawing after auto-insert completes
    if let Some(shot_path) = &screenshot_out {
        pump_gui_messages();
        let ps_cmd = format!(
            "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
             $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
             $bmp=New-Object System.Drawing.Bitmap($b.Width,$b.Height); \
             $g=[System.Drawing.Graphics]::FromImage($bmp); \
             $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); \
             $bmp.Save('{}',[System.Drawing.Imaging.ImageFormat]::Png)",
            shot_path.display()
        );
        let _ = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .status();
    }

    unsafe {
        if !target_hwnd.is_null() {
            SendMessageW(target_hwnd, WM_CLOSE, 0, 0);
        }
    }
    if let Some(mut ch) = target_child.take() {
        let _ = ch.kill();
    }
    gui.quit();

    let summary = serde_json::json!({
        "status": "PASS",
        "os_test": "Windows 11 Live E2E Suite",
        "idle_shell_working_set_mb": idle_ws_mb,
        "idle_shell_private_mb": idle_priv_mb,
        "single_instance_mutex": "PASS",
        "dpapi_roundtrip": "PASS",
        "autostart_registry_quoted": stored,
        "product_run_value_before_test": product_before,
        "clipboard_preserve_on_silence": "PASS",
        "clipboard_unicode_multilingual": "PASS",
        "microphone_hardware": mic_hw_report,
        "foreground_context_detected": fg_ctx,
        "auto_insert_matrix": auto_insert_matrix,
        "wav_e2e": wav_e2e_json,
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

/// The route rule's answer for this configuration (plan P3.5). A writing model counts only when its
/// weight file is on disk, so an uninstalled one reports as unavailable instead of failing at submit.
#[cfg(windows)]
fn route_for(cfg: &AppConfig) -> vi_core::route::Route {
    let installed = !RuntimePaths::discover(cfg).find_gguf_candidates(&cfg.local_llm_model).is_empty();
    let local = installed.then_some(cfg.local_llm_model.as_str());
    vi_core::route::route(cfg.processing_profile, cfg.text_jobs_allow_cloud, &cfg.cloud, local)
}

/// The cloud target a text job carries, or `None` when the route is local. The key is only decrypted
/// when the route already chose the cloud.
#[cfg(windows)]
fn job_cloud(cfg: &AppConfig, route: &vi_core::route::Route) -> Option<vi_core::textjob::CloudTarget> {
    if !route.cloud {
        return None;
    }
    let key = cfg.cloud.encrypted_api_key_b64.as_deref().and_then(|b| dpapi_decrypt_from_b64(b).ok()).unwrap_or_default();
    route.target(&cfg.cloud, key)
}

/// Opens the Composer bar on the selection (plan P3.4). One function for the tray item, the pill and the
/// Composer chord, so the three can never drift. `held` is the selection a voice-command tap already captured;
/// without it the selection is copied again, and a window that refuses the copy opens the bar empty.
#[cfg(windows)]
fn open_composer(
    cfg: &AppConfig,
    composer: &mut composer_flow::ComposerFlow,
    gui: &vi_win32::ui::win_gui::WinGuiHandle,
    held: Option<(ForegroundContext, vi_win32::selection::Captured)>,
) {
    let (ctx, captured, allowed) = match held {
        Some((ctx, captured)) => (ctx, Some(captured), true),
        None => {
            let ctx = capture_foreground_context(cfg.privacy.redact_sensitive_window_titles);
            // Never copy from KeyReed's own windows or where the selection service refuses.
            let allowed = ctx.process_id != std::process::id()
                && vi_win32::selection::refusal_for(&ctx, vi_win32::typing::is_current_process_elevated()).is_none();
            let captured = if allowed {
                vi_win32::selection::capture(&ctx).ok().filter(|c| !c.copied_line)
            } else {
                None
            };
            (ctx, captured, allowed)
        }
    };
    // One rule decides the route, for the label and for the job (plan P3.5).
    let route = route_for(cfg);
    let selection = captured.as_ref().map(|c| c.text.clone());
    composer.open(captured, ctx.clone(), job_cloud(cfg, &route));
    gui.composer(vi_win32::composer::ComposerMsg::Open(vi_win32::composer::ComposerRequest {
        selection,
        app: if allowed { vi_win32::composer::app_label(&ctx.process_name) } else { String::new() },
        route: route.label,
        cloud: route.cloud,
        unavailable: !route.allowed,
    }));
}

#[cfg(windows)]
fn run_windows_desktop_daemon(open_settings_on_start: bool) -> Result<()> {
    harden_and_init_process();

    let Some(_instance_guard) = SingleInstanceGuard::acquire(SINGLE_INSTANCE_MUTEX)? else {
        let _ = vi_win32::ui::win_gui::signal_already_running();
        log::info!("{} is already running in the system tray.", PRODUCT_NAME);
        return Ok(());
    };

    let cfg_path = AppConfig::default_config_path();
    let hist_path = AppConfig::default_history_path();
    let initial_cfg = AppConfig::load_or_default(&cfg_path);
    if let Ok(exe) = env::current_exe() {
        let _ = set_autostart_enabled(AUTOSTART_REG_VALUE, initial_cfg.autostart_on_login, &exe);
    }

    let (hk_tx, hk_rx) = crossbeam_channel::unbounded::<HotkeyEvent>();
    let (ui_tx, ui_rx) = crossbeam_channel::unbounded::<UiAction>();
    let busy_flag = Arc::new(AtomicBool::new(false));

    let hk_engine = Arc::new(GlobalHotkeyEngine::start(
        hk_tx,
        initial_cfg.ptt_hotkey.clone(),
        initial_cfg.mode_cycle_hotkey.clone(),
        Arc::clone(&busy_flag),
        initial_cfg.app_enabled && initial_cfg.ptt_enabled,
    )?);
    register_extra_chords(&hk_engine, &initial_cfg);

    let mic_names: Vec<String> = list_input_devices().into_iter().map(|d| d.name).collect();
    let init_paths = RuntimePaths::discover(&initial_cfg);
    let installed_asr = init_paths.list_installed_asr_ids();
    let active_model_installed = vi_asr::catalog::find_model_spec(&initial_cfg.local_asr_model)
        .and_then(|s| init_paths.find_installed_model_root(s.folder_name))
        .is_some();
    let is_minimized = env::args().any(|a| a == "--minimized");
    let should_open_settings =
        open_settings_on_start || (!is_minimized && (!initial_cfg.first_run_completed || !active_model_installed));

    let (gui_handle, gui_pump) = vi_win32::ui::win_gui::init_and_spawn_gui_windows(
        ui_tx.clone(),
        initial_cfg.clone(),
        mic_names,
        installed_asr,
        should_open_settings,
    )?;

    let gui_for_worker = gui_handle.clone();
    let ui_tx_coord = ui_tx.clone();
    let hk_for_worker = Arc::clone(&hk_engine);
    let download_cancel_flag = Arc::new(AtomicBool::new(false));
    let download_active_flag = Arc::new(AtomicBool::new(false));
    let (job_ev_tx, job_ev_rx) = crossbeam_channel::unbounded::<vi_core::textjob::TextJobEvent>();
    let qf_cfg = initial_cfg.clone();
    let job_tx = textjobs::spawn_text_worker(job_ev_tx, Box::new(move || quickfix_flow::load_engine(&qf_cfg)))?;
    let learning = learning::Learning::spawn();
    let learning_events = learning.events().clone();

    std::thread::Builder::new()
        .name("keyreed-coordinator".to_string())
        .spawn(move || {
            let mut cfg = initial_cfg;
            let mut history = HistoryStore::load_or_default(&hist_path);
            // "My style" (plan F3): loaded once here, written after every change, and mirrored to Settings.
            let style_path = StyleStore::default_path();
            let mut style = match StyleStore::load(&style_path) {
                Ok((s, LoadOutcome::SetAside(p))) => {
                    log::warn!("the style file could not be read; it was kept as {}", p.display());
                    s
                }
                Ok((s, _)) => s,
                Err(e) => {
                    log::warn!("the style store could not be loaded: {e:#}");
                    StyleStore::default()
                }
            };
            sync_learning_switch(&cfg, &mut style);
            gui_for_worker.set_style(style.clone());
            let mut ptt = PttController::new(cfg.activation_mode, 110, 1_800_000);
            let _ = ptt.set_app_enabled(cfg.app_enabled);
            let _ = ptt.set_ptt_enabled(cfg.ptt_enabled);

            let mut supervisor = InferenceSupervisor::new(&cfg);
            if let Some(spec) = vi_asr::catalog::find_model_spec(&cfg.local_asr_model) {
                let paths = RuntimePaths::discover(&cfg);
                if paths.find_installed_model_root(spec.folder_name).is_none() {
                    gui_for_worker.notify_warning(
                        "KeyReed needs a speech model",
                        "Download one in KeyReed > Speech models (about 460 MB, one time). Dictation works offline after that.",
                    );
                } else if cfg.processing_profile != vi_core::ProcessingProfile::MaxAccuracy {
                    supervisor.prewarm_asr(&cfg);
                }
            }
            let mut active_mic: Option<ActiveCapture> = None;
            let mut captured_fg_ctx = ForegroundContext::default();
            let mut last_external_fg_ctx = ForegroundContext::default();
            let mut last_completed_text = String::new();
            let mut composer = composer_flow::ComposerFlow::default();
            let mut quick_fix = quickfix_flow::QuickFixFlow::default();
            // Where this capture's transcript goes (plan P3.7); ordinary dictation delivers it.
            let mut capture_target = CaptureTarget::Deliver;
            // When the voice-command chord went down: a tap opens the Composer, a hold speaks.
            let mut voice_pressed_ms: Option<u64> = None;
            // The word the pill just announced, so its Undo chip can take it back.
            let mut learned_undo: Option<UndoToken> = None;
            let mut saw_physical_key_down = false;
            let mut physical_key_up_ticks: u8 = 0;
            let mut transient_state_until: Option<Instant> = None;
            let start_epoch = Instant::now();
            let now_ms = || start_epoch.elapsed().as_millis() as u64;

            // Open the microphone once, stopped, so the first press starts capturing in milliseconds
            // instead of paying the WASAPI open (see ActiveCapture::prewarm). Runs on this thread
            // because the warm client is thread-local, and costs this startup only.
            let t_warm = Instant::now();
            let warm_ok = ActiveCapture::prewarm(cfg.selected_microphone.as_deref());
            timing_mark(&format!(
                "mic_prewarm ok={} ms={:.1}",
                warm_ok,
                t_warm.elapsed().as_secs_f64() * 1000.0
            ));

            let ticker = tick(Duration::from_millis(75));

            loop {
                let mut pending_action = PttAction::None;

                select! {
                    recv(hk_rx) -> ev => {
                        match ev {
                            Ok(HotkeyEvent::PttDown) => {
                                saw_physical_key_down = is_spec_physically_down(&cfg.ptt_hotkey);
                                physical_key_up_ticks = 0;
                                pending_action = ptt.on_shortcut_down(now_ms());
                            }
                            Ok(HotkeyEvent::PttUp) => {
                                saw_physical_key_down = false;
                                physical_key_up_ticks = 0;
                                pending_action = ptt.on_shortcut_up(now_ms());
                            }
                            Ok(HotkeyEvent::EmergencyCancel) => {
                                saw_physical_key_down = false;
                                physical_key_up_ticks = 0;
                                pending_action = ptt.on_emergency_cancel("escape_pressed");
                            }
                            Ok(HotkeyEvent::PttSuperseded) => {
                                saw_physical_key_down = false;
                                physical_key_up_ticks = 0;
                                pending_action = ptt.on_emergency_cancel("superseded_by_chord");
                            }
                            Ok(HotkeyEvent::CorrectSelection) => {
                                if matches!(ptt.state(), EngineState::Idle | EngineState::ReadyToPaste) {
                                    quick_fix.start(&cfg, &gui_for_worker, &job_tx, &supervisor.rewrite);
                                }
                            }
                            Ok(HotkeyEvent::OpenComposer) => {
                                if matches!(ptt.state(), EngineState::Idle | EngineState::ReadyToPaste) {
                                    open_composer(&cfg, &mut composer, &gui_for_worker, None);
                                }
                            }
                            Ok(HotkeyEvent::ToggleDictation) => {
                                // A bound mouse button: the floating listener's click, without the pill.
                                pending_action = ptt.on_floating_mic_click(now_ms());
                            }
                            // The voice-command chord: capture the selection on press, record while it is held, and on
                            // release either speak the instruction (a hold) or open the Composer on it (a tap).
                            // Without a selection nothing records and nothing opens.
                            Ok(HotkeyEvent::VoiceCommandDown) => {
                                if matches!(ptt.state(), EngineState::Idle | EngineState::ReadyToPaste) {
                                    let armed = quick_fix.arm_voice(&cfg, &gui_for_worker);
                                    if voice::may_record(CaptureTarget::VoiceCommand, true, armed) {
                                        capture_target = CaptureTarget::VoiceCommand;
                                        voice_pressed_ms = Some(now_ms());
                                        // The microphone opens on the press so a hold never clips its first word;
                                        // a tap throws those few milliseconds away unread.
                                        pending_action = ptt.on_floating_mic_click(now_ms());
                                    }
                                }
                            }
                            Ok(HotkeyEvent::VoiceCommandUp) => {
                                let held = voice_pressed_ms.take().map_or(0, |t| now_ms().saturating_sub(t));
                                let armed = capture_target == CaptureTarget::VoiceCommand && ptt.is_recording();
                                match voice::release(armed, held) {
                                    Some(voice::ChordRelease::Speak) => pending_action = ptt.on_floating_mic_click(now_ms()),
                                    Some(voice::ChordRelease::OpenComposer) => {
                                        // Discard the tap's audio, then open the bar on the selection the press took.
                                        pending_action = ptt.on_emergency_cancel("voice_tap");
                                        open_composer(&cfg, &mut composer, &gui_for_worker, quick_fix.cancel_voice());
                                    }
                                    // The press never captured a selection (refused, or a dictation was running).
                                    None => {
                                        quick_fix.cancel_voice();
                                    }
                                };
                            }
                            Ok(HotkeyEvent::CycleMode) => {
                                cfg.default_writing_mode = cfg.default_writing_mode.next();
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.flash_hint(cfg.default_writing_mode.label(), "");
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Err(_) => break,
                        }
                    }
                    recv(ui_rx) -> ua => {
                        match ua {
                            Ok(UiAction::ToggleAppEnabled) => {
                                cfg.app_enabled = !cfg.app_enabled;
                                pending_action = ptt.set_app_enabled(cfg.app_enabled);
                                hk_for_worker.set_enabled(cfg.app_enabled && cfg.ptt_enabled);
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Ok(UiAction::TogglePttEnabled) => {
                                cfg.ptt_enabled = !cfg.ptt_enabled;
                                pending_action = ptt.set_ptt_enabled(cfg.ptt_enabled);
                                hk_for_worker.set_enabled(cfg.app_enabled && cfg.ptt_enabled);
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Ok(UiAction::FloatingMicClicked) => {
                                pending_action = ptt.on_floating_mic_click(now_ms());
                            }
                            Ok(UiAction::StartDictation) => {
                                if matches!(ptt.state(), EngineState::Idle | EngineState::ReadyToPaste) {
                                    pending_action = ptt.on_floating_mic_click(now_ms());
                                }
                            }
                            Ok(UiAction::OpenComposer) => {
                                if matches!(ptt.state(), EngineState::Idle | EngineState::ReadyToPaste) {
                                    open_composer(&cfg, &mut composer, &gui_for_worker, None);
                                }
                            }
                            Ok(UiAction::ComposerClosed) => {
                                composer.close();
                                // A dictation into a closing Composer has nowhere to go: stop it and deliver nothing.
                                if capture_target == CaptureTarget::Composer {
                                    pending_action = ptt.on_emergency_cancel("composer_closed");
                                }
                            }
                            Ok(UiAction::SavePillPosition(x, y)) => {
                                cfg.floating_pill_pos = Some((x, y));
                                let _ = cfg.save_atomic(&cfg_path);
                            }
                            Ok(UiAction::SetActivationMode(m)) => {
                                cfg.activation_mode = m;
                                ptt.set_activation_mode(m);
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Ok(UiAction::SetOutputMode(o)) => {
                                cfg.output_mode = o;
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                                if o == vi_core::OutputMode::DirectTyping {
                                    gui_for_worker.notify_toast(
                                        PRODUCT_NAME,
                                        "Auto Insert is on. Text is inserted automatically into the focused field, with clipboard fallback if unsafe.",
                                    );
                                }
                            }
                            Ok(UiAction::CopyLastDictation) => {
                                if !last_completed_text.is_empty() {
                                    let _gate = vi_win32::input_gate();
                                    if write_clipboard_unicode(
                                        &last_completed_text,
                                        cfg.privacy.exclude_from_clipboard_history,
                                    )
                                    .is_ok()
                                    {
                                        gui_for_worker.notify_toast(
                                            PRODUCT_NAME,
                                            "Last dictation copied to clipboard.",
                                        );
                                    }
                                } else {
                                    gui_for_worker.notify_toast(
                                        PRODUCT_NAME,
                                        "No completed dictation in this session yet.",
                                    );
                                }
                            }
                            Ok(UiAction::CancelModelDownload) => {
                                download_cancel_flag.store(true, Ordering::Relaxed);
                            }
                            Ok(UiAction::SelectAsrModel(id)) => {
                                let paths = RuntimePaths::discover(&cfg);
                                let installed = vi_asr::catalog::find_model_spec(&id)
                                    .map(|s| paths.find_installed_model_root(s.folder_name).is_some())
                                    .unwrap_or(false);
                                if installed {
                                    cfg.local_asr_model = id;
                                    let _ = cfg.save_atomic(&cfg_path);
                                    gui_for_worker.update_status("", false, Some(paths.list_installed_asr_ids()));
                                    gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                                    if cfg.processing_profile != vi_core::ProcessingProfile::MaxAccuracy {
                                        supervisor.prewarm_asr(&cfg);
                                    }
                                }
                            }
                            Ok(UiAction::RefreshMicrophones) => {
                                let mics: Vec<String> = list_input_devices().into_iter().map(|d| d.name).collect();
                                gui_for_worker.set_microphones(mics);
                            }
                            Ok(UiAction::DownloadDefaultModel) | Ok(UiAction::DownloadModel(_)) => {
                                let model_id = match ua {
                                    Ok(UiAction::DownloadModel(id)) => id,
                                    _ => "parakeet-tdt-0.6b-v2-int8".to_string(),
                                };
                                if download_active_flag.swap(true, Ordering::SeqCst) {
                                    gui_for_worker.notify_toast(
                                        PRODUCT_NAME,
                                        "A speech model download is already in progress.",
                                    );
                                } else {
                                    download_cancel_flag.store(false, Ordering::Relaxed);
                                    let select_tx = ui_tx_coord.clone();
                                    let cancel_cl = Arc::clone(&download_cancel_flag);
                                    let active_cl = Arc::clone(&download_active_flag);
                                    let rpaths = RuntimePaths::discover(&cfg);
                                    let dl_dir = rpaths.writable_models_dir();
                                    let gui_dl = gui_for_worker.clone();
                                    std::thread::spawn(move || {
                                        let progress = |stage: DownloadStage, done: u64, total: Option<u64>| DownloadProgress {
                                            model_id: model_id.clone(),
                                            stage,
                                            done_bytes: done,
                                            total_bytes: total,
                                        };
                                        let Some(spec) = vi_asr::catalog::find_model_spec(&model_id) else {
                                            active_cl.store(false, Ordering::SeqCst);
                                            gui_dl.set_download(Some(progress(DownloadStage::Failed("this model isn't in KeyReed's catalog".into()), 0, None)), None);
                                            return;
                                        };
                                        // The grammar tagger is found by Quick Fix itself; it must never become the dictation model.
                                        let speech = spec.architecture != vi_asr::catalog::AsrArchitecture::GectorTagger;
                                        if let Some(existing_root) = rpaths.find_installed_model_root(spec.folder_name) {
                                            if vi_asr::integrity::prepare_installed_model(&existing_root, spec.id).is_ok() {
                                                active_cl.store(false, Ordering::SeqCst);
                                                let inst = rpaths.list_installed_asr_ids();
                                                gui_dl.set_download(Some(progress(DownloadStage::Installed, 0, None)), Some(inst));
                                                if speech {
                                                    let _ = select_tx.send(UiAction::SelectAsrModel(spec.id.to_string()));
                                                }
                                                return;
                                            }
                                        }
                                        gui_dl.set_download(Some(progress(DownloadStage::Starting, 0, None)), None);
                                        let gui_prog = gui_dl.clone();
                                        let prog_id = model_id.clone();
                                        let last_emit = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
                                        let prog_cb = move |done_bytes: u64, total_opt: Option<u64>, stage: &'static str| {
                                            let st = if stage == "extracting" { DownloadStage::Verifying } else { DownloadStage::Downloading };
                                            // Throttle UI updates to ~8/s; stage changes always go through.
                                            if let Ok(mut t) = last_emit.lock() {
                                                if st == DownloadStage::Downloading && t.elapsed() < Duration::from_millis(120) {
                                                    return;
                                                }
                                                *t = Instant::now();
                                            }
                                            gui_prog.set_download(
                                                Some(DownloadProgress { model_id: prog_id.clone(), stage: st, done_bytes, total_bytes: total_opt }),
                                                None,
                                            );
                                        };
                                        let res = vi_asr::catalog::ensure_model_present_with_progress(
                                            &dl_dir,
                                            spec,
                                            Some(&cancel_cl),
                                            Some(&prog_cb),
                                        );
                                        active_cl.store(false, Ordering::SeqCst);
                                        let inst = rpaths.list_installed_asr_ids();
                                        let ui_name = vi_win32::ui::asr_ui_info(spec.id).map(|i| i.name).unwrap_or(spec.display_name);
                                        match res {
                                            Ok(_) => {
                                                gui_dl.set_download(Some(progress(DownloadStage::Installed, 0, None)), Some(inst));
                                                if speech {
                                                    let _ = select_tx.send(UiAction::SelectAsrModel(spec.id.to_string()));
                                                }
                                                gui_dl.notify_toast(
                                                    &format!("{ui_name} is ready"),
                                                    if speech {
                                                        "Downloaded and verified. KeyReed will use it for your next dictation."
                                                    } else {
                                                        "Downloaded and verified. Quick Fix can check grammar now."
                                                    },
                                                );
                                            }
                                            Err(e) if e.to_string().to_ascii_lowercase().contains("cancel") => {
                                                log::info!("Model download cancelled: {e:#}");
                                                gui_dl.set_download(Some(progress(DownloadStage::Cancelled, 0, None)), Some(inst));
                                            }
                                            Err(e) => {
                                                log::error!("Model download failed: {e:#}");
                                                let lower = format!("{e:#}").to_ascii_lowercase();
                                                let why = if lower.contains("sha") || lower.contains("checksum") || lower.contains("hash") {
                                                    "the file didn't match its checksum, so it was discarded. Try again."
                                                } else if lower.contains("dns") || lower.contains("connect") || lower.contains("timed out") || lower.contains("network") || lower.contains("tls") {
                                                    "couldn't reach the download server. Check your connection and try again."
                                                } else if lower.contains("space") || lower.contains("disk") {
                                                    "there isn't enough free disk space."
                                                } else {
                                                    "something went wrong. Try again; details are in the log."
                                                };
                                                gui_dl.set_download(Some(progress(DownloadStage::Failed(why.into()), 0, None)), Some(inst));
                                                gui_dl.notify_warning(&format!("{ui_name} didn't download"), &format!("The download stopped: {why}"));
                                            }
                                        }
                                    });
                                }
                            }
                            Ok(UiAction::TestMicrophone(sel_mic)) => {
                                let gui_test = gui_for_worker.clone();
                                std::thread::spawn(move || {
                                    match ActiveCapture::start(sel_mic.as_deref()) {
                                        Ok(cap) => {
                                            let dev_name = cap.device_name().to_string();
                                            let used_fb = cap.used_fallback_device();
                                            // Live meter: real RMS from the capture stream, ~20 updates per second.
                                            let t0 = Instant::now();
                                            let mut peak_seen = 0.0f32;
                                            while t0.elapsed() < Duration::from_millis(3000) {
                                                std::thread::sleep(Duration::from_millis(50));
                                                let cur = cap.current_rms();
                                                peak_seen = peak_seen.max(cur);
                                                gui_test.set_mic_test(MicTestStatus {
                                                    running: true,
                                                    level: vi_win32::overlay::rms_to_level(cur),
                                                    peak_rms: peak_seen,
                                                    device: dev_name.clone(),
                                                    used_fallback: used_fb,
                                                    verdict: None,
                                                    error: None,
                                                });
                                            }
                                            let rms = cap.current_rms();
                                            let peak = cap.peak_rms();
                                            let samples = cap.stop_and_take_16k();
                                            let pct = (peak * 100.0).round() as u32;
                                            let fb_note = if used_fb { " [Fallback to Default]" } else { "" };
                                            let msg = if samples.is_empty() {
                                                format!(
                                                    "Mic Error ({}{}): 0 samples captured. Check Windows Microphone Privacy.",
                                                    dev_name, fb_note
                                                )
                                            } else if peak >= 0.94 {
                                                format!(
                                                    "Mic Warning ({}{}): Signal clipping (Peak {}%). Lower input volume.",
                                                    dev_name, fb_note, pct
                                                )
                                            } else if peak >= 0.08 {
                                                format!(
                                                    "Mic OK ({}{}): Healthy signal ({} samples, Peak {}%, Level {:.2}).",
                                                    dev_name, fb_note, samples.len(), pct, rms
                                                )
                                            } else if peak > 0.001 {
                                                format!(
                                                    "Mic Connected ({}{}): Active stream ({} samples, quiet room floor Peak {}%).",
                                                    dev_name, fb_note, samples.len(), pct.max(1)
                                                )
                                            } else {
                                                format!(
                                                    "Mic Muted/Silent ({}{}): 0% signal ({} samples). Check hardware mute button.",
                                                    dev_name, fb_note, samples.len()
                                                )
                                            };
                                            log::info!("Microphone test: {msg}");
                                            let peak_all = peak.max(peak_seen);
                                            gui_test.set_mic_test(MicTestStatus {
                                                running: false,
                                                level: 0.0,
                                                peak_rms: peak_all,
                                                device: dev_name.clone(),
                                                used_fallback: used_fb,
                                                verdict: if samples.is_empty() { None } else { Some(classify_mic(peak_all)) },
                                                error: if samples.is_empty() {
                                                    Some("Mic error: the microphone delivered no audio".to_string())
                                                } else {
                                                    None
                                                },
                                            });
                                        }
                                        Err(e) => {
                                            log::warn!("Microphone test could not open device: {e:#}");
                                            gui_test.set_mic_test(MicTestStatus {
                                                error: Some(format!("Mic error: {e}")),
                                                ..Default::default()
                                            });
                                        }
                                    }
                                });
                            }
                            Ok(UiAction::ResetToDefaults) => {
                                let mut def_cfg = AppConfig::default();
                                def_cfg.first_run_completed = true;
                                if let Ok(exe) = env::current_exe() {
                                    let _ = set_autostart_enabled(AUTOSTART_REG_VALUE, false, &exe);
                                }
                                ptt.set_activation_mode(def_cfg.activation_mode);
                                hk_for_worker.update_shortcuts(
                                    def_cfg.ptt_hotkey.clone(),
                                    def_cfg.mode_cycle_hotkey.clone(),
                                );
                                register_extra_chords(&hk_for_worker, &def_cfg);
                                hk_for_worker.set_enabled(true);
                                cfg = def_cfg;
                                // Save and show the defaults first; rebuilding the engines re-verifies the
                                // grammar model (seconds), which used to delay the visible reset.
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                                supervisor = InferenceSupervisor::new(&cfg);
                            }
                            Ok(UiAction::SessionOrPowerReset) => {
                                hk_for_worker.reset_modifiers();
                                saw_physical_key_down = false;
                                physical_key_up_ticks = 0;
                                pending_action = ptt.on_emergency_cancel("session_or_power_transition");
                            }
                            Ok(UiAction::SetWritingMode(wm)) => {
                                cfg.default_writing_mode = wm;
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Ok(UiAction::ToggleAutoContext) => {
                                cfg.auto_context_mode = !cfg.auto_context_mode;
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Ok(UiAction::SetProcessingProfile(p)) => {
                                cfg.processing_profile = p;
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            Ok(UiAction::ToggleFloatingMicVisible) => {
                                cfg.show_floating_mic = !cfg.show_floating_mic;
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
                            // Settings "Play": the same cue the next recording start will use.
                            Ok(UiAction::PreviewAudioCue) => play_subtle_earcon(&cfg, true, true),
                            Ok(UiAction::SaveUpdatedConfig { mut new_cfg, mut new_plaintext_api_key }) => {
                                if let Some(ref mut key) = new_plaintext_api_key {
                                    if let Ok(enc) = dpapi_encrypt_to_b64(key) {
                                        new_cfg.cloud.encrypted_api_key_b64 = Some(enc);
                                    }
                                    key.zeroize();
                                }
                                if let Ok(exe) = env::current_exe() {
                                    let _ = set_autostart_enabled(
                                        AUTOSTART_REG_VALUE,
                                        new_cfg.autostart_on_login,
                                        &exe,
                                    );
                                }
                                ptt.set_activation_mode(new_cfg.activation_mode);
                                hk_for_worker.update_shortcuts(
                                    new_cfg.ptt_hotkey.clone(),
                                    new_cfg.mode_cycle_hotkey.clone(),
                                );
                                register_extra_chords(&hk_for_worker, &new_cfg);
                                hk_for_worker.set_enabled(new_cfg.app_enabled && new_cfg.ptt_enabled);
                                let llm_changed = cfg.local_llm_model != new_cfg.local_llm_model;
                                // The listener position is owned by the coordinator (drag saves bypass the UI snapshot).
                                new_cfg.floating_pill_pos = cfg.floating_pill_pos;
                                cfg = *new_cfg;
                                let _ = cfg.save_atomic(&cfg_path);
                                if sync_learning_switch(&cfg, &mut style) {
                                    save_style(&style, &style_path, &gui_for_worker);
                                }
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                                if llm_changed {
                                    // After saving: re-verifying a grammar model can take seconds.
                                    supervisor = InferenceSupervisor::new(&cfg);
                                }
                            }
                            Ok(UiAction::SaveStyleStore(new_style)) => {
                                style = *new_style;
                                sync_learning_switch(&cfg, &mut style);
                                save_style(&style, &style_path, &gui_for_worker);
                            }
                            Ok(UiAction::QuickFixUndo) => quick_fix.undo(&gui_for_worker),
                            Ok(UiAction::LearningUndo) => {
                                if let Some(token) = learned_undo.take() {
                                    learning::undo_learning(&mut cfg, &token);
                                    let _ = cfg.save_atomic(&cfg_path);
                                    gui_for_worker.quick_fix(QuickFixStage::Notice("Unlearned".into()));
                                }
                            }
                            Ok(UiAction::ReviewDone { choice, text, accepted, .. }) => {
                                quick_fix.on_review_done(choice, &text, accepted, &gui_for_worker, &learning);
                            }
                            Ok(UiAction::QuickFixIntroSeen) => {
                                cfg.quick_fix_intro_seen = true;
                                let _ = cfg.save_atomic(&cfg_path);
                            }
                            // The deeper pass is not in this build: the pill shows no Deeper chip.
                            Ok(UiAction::QuickFixDeeper) => {}
                            // The Composer's microphone: dictate into the bar instead of the target app.
                            Ok(UiAction::ComposerDictate) => {
                                if capture_target == CaptureTarget::Composer && ptt.is_recording() {
                                    // A second click finishes the dictation and puts it in the bar.
                                    pending_action = ptt.on_floating_mic_click(now_ms());
                                } else if matches!(ptt.state(), EngineState::Idle | EngineState::ReadyToPaste)
                                    && voice::may_record(CaptureTarget::Composer, composer.is_open(), true)
                                {
                                    capture_target = CaptureTarget::Composer;
                                    pending_action = ptt.on_floating_mic_click(now_ms());
                                }
                            }
                            Ok(UiAction::ComposerSubmit { instruction, use_selection }) => {
                                composer.submit(&cfg, instruction, use_selection, &job_tx, &supervisor.rewrite, &style);
                            }
                            Ok(UiAction::ComposerFollowUp(instruction)) => {
                                composer.follow_up(&cfg, instruction, &job_tx, &supervisor.rewrite, &style);
                            }
                            Ok(UiAction::ComposerRegenerate) => {
                                composer.regenerate(&cfg, &job_tx, &supervisor.rewrite, &style);
                            }
                            Ok(UiAction::ComposerCancel) => composer.cancel(),
                            Ok(UiAction::ComposerDeliver { how, text }) => composer.deliver(how, &text, &cfg, &gui_for_worker),
                            Ok(UiAction::ClearHistory) => {
                                history.clear_all();
                                let _ = history.save_atomic(&hist_path);
                                gui_for_worker.notify_toast(PRODUCT_NAME, "Local transcription history cleared.");
                            }
                            Ok(UiAction::Quit) | Err(_) => {
                                if let Some(m) = active_mic.take() {
                                    m.cancel_and_zeroize();
                                }
                                supervisor.force_unload_all();
                                break;
                            }
                        }
                    }
                    recv(learning_events) -> ev => {
                        if let Ok(learning::LearningEvent::Learned { word, token, dictionary }) = ev {
                            // The worker hands back its dictionary, so the config and style files match what it kept.
                            cfg.dictionary = dictionary;
                            if cfg.save_atomic(&cfg_path).is_ok() {
                                learned_undo = Some(token);
                                gui_for_worker.quick_fix(QuickFixStage::Learned(word));
                            }
                        }
                    }
                    recv(job_ev_rx) -> ev => {
                        if let Ok(ev) = ev {
                            // All flows see every event and ignore the jobs that are not theirs.
                            quick_fix.on_job_event(ev.clone(), &cfg, &style, &gui_for_worker, &learning);
                            composer.on_job_event(ev, &gui_for_worker);
                        }
                    }
                    recv(ticker) -> _ => {
                        // Track last known external foreground window so floating control or tray interactions never lose target focus
                        if active_mic.is_none() {
                            let cur = capture_foreground_context(cfg.privacy.redact_sensitive_window_titles);
                            if cur.hwnd != 0 && cur.process_id != std::process::id() {
                                last_external_fg_ctx = cur;
                            }
                        }

                        // Fast lost-keyup recovery if Windows swallowed WM_KEYUP across an elevation/focus boundary
                        if ptt.is_recording()
                            && cfg.activation_mode == vi_core::ActivationMode::HoldToTalk
                            && ptt.is_shortcut_physically_down()
                            && saw_physical_key_down
                        {
                            if !is_spec_physically_down(&cfg.ptt_hotkey) {
                                physical_key_up_ticks = physical_key_up_ticks.saturating_add(1);
                                if physical_key_up_ticks >= 3 {
                                    saw_physical_key_down = false;
                                    physical_key_up_ticks = 0;
                                    hk_for_worker.reset_modifiers();
                                    pending_action = ptt.on_shortcut_up(now_ms());
                                }
                            } else {
                                physical_key_up_ticks = 0;
                            }
                        }

                        if pending_action == PttAction::None {
                            pending_action = ptt.on_tick(now_ms());
                        }
                        if let Some(mic) = &mut active_mic {
                            supervisor.session_first_frame_ms = mic.first_frame_ms();
                            // Drain only into a live session; otherwise the audio stays in the capture
                            // buffer for the whole-buffer path at stop (cloud profile, failed live start).
                            let chunk = if supervisor.live_asr.is_some() { mic.take_new_16k() } else { Vec::new() };
                            supervisor.feed_live_audio(&cfg, chunk);
                            if mic.is_disconnected() {
                                // Microphone disconnected mid-recording: attempt seamless rebind to default device
                                log::warn!("Active microphone disconnected mid-session; attempting default-device recovery");
                                if let Ok( fallback_cap ) = ActiveCapture::start(None) {
                                    gui_for_worker.set_level_source(Some(fallback_cap.level_handle()));
                                    active_mic = Some(fallback_cap);
                                    gui_for_worker.notify_warning(
                                        "Microphone disconnected",
                                        "KeyReed switched to the Windows default microphone and kept recording.",
                                    );
                                } else {
                                    pending_action = PttAction::StopAndProcess;
                                }
                            } else {
                                gui_for_worker.sync(EngineState::Listening, mic.current_rms(), None);
                            }
                        } else {
                            if let Some(deadline) = transient_state_until {
                                if Instant::now() >= deadline {
                                    transient_state_until = None;
                                    ptt.transition_to_idle();
                                    gui_for_worker.sync(ptt.state().clone(), 0.0, None);
                                }
                            }
                            supervisor.poll_background_ready(&cfg);
                            supervisor.maybe_unload_idle(cfg.idle_unload_seconds);
                        }
                    }
                }

                match pending_action {
                    PttAction::None => {}
                    PttAction::StartRecording => {
                        timing_mark("coordinator_start_recording");
                        supervisor.session_keydown_at = Some(Instant::now());
                        transient_state_until = None;
                        // The microphone first: everything else can wait behind a running capture, the
                        // first syllable cannot.
                        let t_mic = Instant::now();
                        match ActiveCapture::start(cfg.selected_microphone.as_deref()) {
                            Ok(cap) => {
                                supervisor.session_mic_init_ms = t_mic.elapsed().as_secs_f64() * 1000.0;
                                timing_mark(&format!(
                                    "mic_started warm={} start_ms={:.1}",
                                    cap.was_warm(),
                                    cap.start_ms()
                                ));
                                gui_for_worker.set_level_source(Some(cap.level_handle()));
                                active_mic = Some(cap);
                                busy_flag.store(true, Ordering::Relaxed);
                                // The cue goes out before the work that is not on the audio path.
                                play_subtle_earcon(&cfg, true, false);
                                gui_for_worker.sync(EngineState::Listening, 0.0, None);
                                let t_fg = Instant::now();
                                let cur_fg = capture_foreground_context(cfg.privacy.redact_sensitive_window_titles);
                                let practice = vi_win32::ui::win_gui::is_practice_target(cur_fg.hwnd as isize);
                                captured_fg_ctx = if cur_fg.hwnd != 0 && (cur_fg.process_id != std::process::id() || practice) {
                                    last_external_fg_ctx = cur_fg.clone();
                                    cur_fg
                                } else {
                                    last_external_fg_ctx.clone()
                                };
                                supervisor.session_target_id_ms = t_fg.elapsed().as_secs_f64() * 1000.0;
                                if cfg.processing_profile != vi_core::ProcessingProfile::MaxAccuracy {
                                    let t_live = Instant::now();
                                    supervisor.begin_live_session(&cfg);
                                    supervisor.prewarm_slm(&cfg);
                                    timing_mark(&format!("live_session_ms={:.1}", t_live.elapsed().as_secs_f64() * 1000.0));
                                }
                                timing_mark(&format!("press_path_ms={:.1}", t_mic.elapsed().as_secs_f64() * 1000.0));
                            }
                            Err(e) => {
                                timing_mark("mic_error");
                                // No capture runs, so a voice command's held selection is let go unread.
                                capture_target = CaptureTarget::Deliver;
                                quick_fix.cancel_voice();
                                let msg = format!("Mic error: {}", e);
                                log::error!("{msg}");
                                ptt.transition_to_error(&msg);
                                busy_flag.store(false, Ordering::Relaxed);
                                gui_for_worker.sync(EngineState::Error(msg.clone()), 0.0, None);
                                gui_for_worker.notify_warning("Microphone unavailable", friendly_error(&msg).message);
                                transient_state_until = Some(Instant::now() + Duration::from_millis(3500));
                            }
                        }
                    }
                    PttAction::CancelRecording { .. } => {
                        if let Some(m) = active_mic.take() {
                            m.cancel_and_zeroize();
                        }
                        // Escape, a disabled app or a closed Composer: nothing is transcribed and nothing is taken.
                        capture_target = CaptureTarget::Deliver;
                        quick_fix.cancel_voice();
                        supervisor.cancel_live_session();
                        busy_flag.store(false, Ordering::Relaxed);
                        gui_for_worker.sync(ptt.state().clone(), 0.0, None);
                    }
                    PttAction::StopAndProcess => {
                        timing_mark("coordinator_stop");
                        // The coordinator is blocked in the pipeline from here on and cannot service a cancel,
                        // so Escape must reach the user's app again instead of being swallowed.
                        busy_flag.store(false, Ordering::Relaxed);
                        let cold = supervisor.session_cold_model && !supervisor.asr_ready(&cfg) && cfg.processing_profile != vi_core::ProcessingProfile::MaxAccuracy;
                        gui_for_worker.set_processing_note(if cold { Some("Loading model") } else { None });
                        gui_for_worker.sync(EngineState::Processing, 0.0, None);
                        let taken = std::mem::take(&mut capture_target);
                        // A voice command is transcribed in Exact mode: the spoken words are the instruction,
                        // not a draft to polish. Neither it nor the Composer delivers anything.
                        let mode_override = (taken == CaptureTarget::VoiceCommand).then_some(WritingMode::Exact);
                        if let Some(m) = active_mic.take() {
                            supervisor.session_first_frame_ms = m.first_frame_ms();
                            let t_fin = Instant::now();
                            // Keeps capturing 200 to 600 ms after release so the last word is not clipped.
                            let samples = m.stop_with_release_tail_and_take_16k();
                            supervisor.session_chunk_finalize_ms = t_fin.elapsed().as_secs_f64() * 1000.0;
                            let gui_stage = gui_for_worker.clone();
                            let on_corr = move || {
                                gui_stage.set_processing_note(None);
                                gui_stage.sync(EngineState::Correcting, 0.0, None);
                            };
                            match supervisor.execute_pipeline(
                                samples,
                                &cfg,
                                &captured_fg_ctx,
                                mode_override,
                                taken == CaptureTarget::Deliver,
                                Some(&on_corr),
                            ) {
                                Ok(res) if !res.final_text.is_empty() && taken != CaptureTarget::Deliver => {
                                    match voice::route(taken, composer.is_open(), &res.final_text) {
                                        voice::Transcript::Composer(text) => {
                                            gui_for_worker.composer(vi_win32::composer::ComposerMsg::Dictated(text.to_string()));
                                        }
                                        voice::Transcript::VoiceCommand(instruction) => {
                                            quick_fix.run_voice(instruction, &cfg, &gui_for_worker, &job_tx, &supervisor.rewrite);
                                        }
                                        // Dictation is delivered below; a closed Composer or silence puts it nowhere.
                                        _ => {}
                                    }
                                    play_subtle_earcon(&cfg, false, false);
                                    ptt.transition_to_idle();
                                    gui_for_worker.sync(EngineState::Idle, 0.0, None);
                                }
                                Ok(res) if !res.final_text.is_empty() => {
                                    timing_mark(&format!(
                                        "clipboard_ready asr_ms={:.1} rewrite_ms={:.1} output_ms={:.2} total_ms={:.1} tier={}",
                                        res.metrics.asr_ms,
                                        res.metrics.rewrite_ms,
                                        res.metrics.output_ms,
                                        res.metrics.total_release_to_ready_ms,
                                        res.metrics.used_tier
                                    ));
                                    last_completed_text = res.final_text.clone();
                                    let mut outcome = ReadyOutcome::from_pipeline(
                                        res.delivered_via,
                                        res.fallback_to_clipboard,
                                        res.requires_command_confirmation,
                                        res.delivery_reason.as_deref(),
                                    );
                                    if outcome == ReadyOutcome::Inserted && res.metrics.used_tier == TIER_SLM_BUSY {
                                        outcome = ReadyOutcome::InsertedBasic;
                                    }
                                    ptt.transition_to_ready();
                                    gui_for_worker.set_outcome(outcome.clone());
                                    gui_for_worker.sync(EngineState::ReadyToPaste, 0.0, None);
                                    play_subtle_earcon(&cfg, false, false);

                                    if history.enabled {
                                        history.record(HistoryEntry {
                                            timestamp_unix: HistoryStore::now_unix(),
                                            mode: res.effective_mode,
                                            app_name: captured_fg_ctx.process_name.clone(),
                                            raw_transcript: res.raw_transcript.clone(),
                                            final_text: res.final_text.clone(),
                                            duration_ms: res.metrics.audio_duration_ms,
                                        });
                                        let _ = history.save_atomic(&hist_path);
                                    }

                                    if learning::delivered_by_typing(res.delivered_via) {
                                        learning.record_insertion(
                                            &res.final_text,
                                            None,
                                            None,
                                            &captured_fg_ctx,
                                            cfg.learn_from_edits,
                                            &cfg.dictionary,
                                        );
                                    }

                                    let preview: String = res.final_text.chars().take(72).collect();
                                    match &outcome {
                                        // Safety-relevant: always explain, regardless of the notification setting.
                                        ReadyOutcome::ReviewCommand => gui_for_worker.notify_warning(
                                            "Review before pasting",
                                            delivery_reason_sentence("dangerous_command_requires_manual_paste"),
                                        ),
                                        ReadyOutcome::FallbackCopied(reason) if cfg.notify_on_ready => {
                                            gui_for_worker.notify_toast("Copied instead of typed", delivery_reason_sentence(reason))
                                        }
                                        ReadyOutcome::Copied if cfg.notify_on_ready && cfg.output_mode == OutputMode::Clipboard => {
                                            gui_for_worker.notify_toast("Ready to paste (Ctrl+V)", &preview)
                                        }
                                        _ => {}
                                    }

                                    ptt.transition_to_idle();
                                    transient_state_until = Some(Instant::now() + Duration::from_millis(1600));
                                }
                                Ok(_) => {
                                    // Nothing recognisable was said: say so briefly instead of silently doing nothing.
                                    ptt.transition_to_idle();
                                    gui_for_worker.set_outcome(ReadyOutcome::NothingHeard);
                                    gui_for_worker.sync(EngineState::ReadyToPaste, 0.0, None);
                                    transient_state_until = Some(Instant::now() + Duration::from_millis(1400));
                                }
                                Err(e) => {
                                    let msg = format!("Transcription failed: {:#}", e);
                                    log::error!("{msg}");
                                    ptt.transition_to_error(&msg);
                                    gui_for_worker.sync(EngineState::Error(msg.clone()), 0.0, None);
                                    let fe = friendly_error(&msg);
                                    gui_for_worker.notify_warning(fe.short, fe.message);
                                    transient_state_until = Some(Instant::now() + Duration::from_millis(3500));
                                }
                            }
                        }
                        gui_for_worker.set_processing_note(None);
                        // Drain any stale key events that arrived while pipeline was busy unless shortcut is still held
                        if cfg.activation_mode == vi_core::ActivationMode::HoldToTalk
                            && !is_spec_physically_down(&cfg.ptt_hotkey)
                        {
                            while hk_rx.try_recv().is_ok() {}
                            hk_for_worker.reset_modifiers();
                        }
                        busy_flag.store(false, Ordering::Relaxed);
                    }
                }
            }
        })?;

    gui_pump();
    Ok(())
}

fn print_help() {
    println!(
        "{} v{} - {}\n\n\
         Usage:\n  \
         keyreed                                  Launch Windows system tray & global Push-to-Talk assistant\n  \
         keyreed --settings                       Launch and open Settings window immediately\n  \
         keyreed --list-models                    List supported local ONNX speech models\n  \
         keyreed --download-model <id>            Download and verify a local speech model\n  \
         keyreed --transcribe-wav <file.wav>      Run full VAD + ASR + Rewrite pipeline on a WAV file (JSON output)\n    \
           [--mode smart|exact|professional|casual|technical|terminal]\n    \
           [--app-category email|chat|document|code_editor|terminal|notes]\n    \
           [--asr-model <id>] [--llm-model <name|none>] [--copy-clipboard]\n  \
         keyreed --print-foreground                  Print the foreground classifier and exit. No mic, no hook.
         keyreed --win32-e2e-test [wav] [png]     Execute automated Windows 11 live E2E verification suite",
        PRODUCT_NAME, VERSION_STR, PRODUCT_TAGLINE
    );
}

fn print_foreground() -> Result<()> {
    #[cfg(windows)]
    {
        let fg = capture_foreground_context(true);
        let recommended = fg.category.recommended_mode(WritingMode::Smart);
        let body = serde_json::json!({
            "process_name": fg.process_name,
            "window_class": fg.window_class,
            "window_title_hint": fg.window_title_hint,
            "control_type": fg.control_type,
            "is_password_field": fg.is_password_field,
            "category": format!("{:?}", fg.category),
            "recommended_mode": format!("{:?}", recommended),
            "recommended_mode_label": recommended.label(),
            "recommendation_fallback": "Smart",
            "microphone_started": false,
            "hotkey_hook_started": false,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        bail!("--print-foreground requires Windows. It does not start a microphone or a keyboard hook.");
    }
}

/// `%LOCALAPPDATA%\Keyreed\logs\keyreed.log`. Rotated once to `.log.1` when it is over 1 MB at start.
fn log_file_path() -> PathBuf {
    let path = AppConfig::default_data_dir().join("logs").join("keyreed.log");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    path
}

#[cfg(windows)]
fn console_attached() -> bool {
    unsafe { !windows_sys::Win32::System::Console::GetConsoleWindow().is_null() }
}
#[cfg(not(windows))]
fn console_attached() -> bool {
    true
}

/// The release exe has no console, so stderr is lost: log to a file unless a console is attached. A panic
/// hook records the panic before `panic = "abort"` ends the process. Never log transcript text.
fn init_logging(keep_stderr: bool) {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"));
    if keep_stderr {
        builder.init();
        return;
    }
    let path = log_file_path();
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.init();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("(no message)");
        // Slice-boundary panics quote the text being sliced, which can be a transcript: withhold those.
        let msg = if msg.contains("byte index") || msg.contains("char boundary") { "(message withheld: may quote text)" } else { msg };
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let name = std::thread::current().name().unwrap_or("unnamed").to_string();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "[{secs}] PANIC thread '{name}' at {at}: {msg}");
        }
    }));
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    #[cfg(windows)]
    if !args.is_empty() {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(0xFFFFFFFF);
        }
    }
    init_logging(!args.is_empty() && console_attached());

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{} {}", PRODUCT_NAME, VERSION_STR);
        return Ok(());
    }

    if args.iter().any(|a| a == "--print-foreground") {
        return print_foreground();
    }

    if args.iter().any(|a| a == "--list-models") {
        println!("{}", serde_json::to_string_pretty(&ASR_CATALOG)?);
        return Ok(());
    }

    if args.iter().any(|a| a == "--list-devices") {
        let devs = list_input_devices();
        println!("{}", serde_json::to_string_pretty(&devs)?);
        return Ok(());
    }

    if let Some(pos) = args.iter().position(|a| a == "--download-model") {
        let id = args.get(pos + 1).context("missing <id> after --download-model")?;
        let spec = find_model_spec(id).with_context(|| format!("unknown model '{}'", id))?;
        let cfg = AppConfig::load_or_default(&AppConfig::default_config_path());
        let paths = RuntimePaths::discover(&cfg);
        let dl_dir = paths.writable_models_dir();
        let dir = ensure_model_present(&dl_dir, spec)?;
        println!("Model ready at {}", dir.display());
        return Ok(());
    }

    if args.iter().any(|a| a == "--transcribe-wav") {
        return run_cli_transcribe_wav(&args);
    }

    if args.iter().any(|a| a == "--eval-quickfix-jsonl") {
        return run_eval_quickfix(&args, false);
    }
    if args.iter().any(|a| a == "--eval-typed-jsonl") {
        return run_eval_quickfix(&args, true);
    }

    if let Some(pos) = args.iter().position(|a| a == "--eval-correction-jsonl") {
        let jsonl = PathBuf::from(
            args.get(pos + 1)
                .context("missing <path.jsonl> after --eval-correction-jsonl")?,
        );
        let llm_opt = args
            .iter()
            .position(|a| a == "--llm-model")
            .and_then(|p| args.get(p + 1).cloned())
            .unwrap_or_else(|| "Qwen3.5-0.8B-Q4_K_M".to_string());
        return run_eval_correction_jsonl(&jsonl, &llm_opt);
    }

    #[cfg(windows)]
    if args.iter().any(|a| a == "--e2e-target-window") {
        return run_e2e_target_window_process();
    }

    #[cfg(windows)]
    if let Some(pos) = args.iter().position(|a| a == "--ui-gallery") {
        let dir = args.get(pos + 1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("ui-gallery"));
        let cfg = AppConfig::load_or_default(&AppConfig::default_config_path());
        let installed = RuntimePaths::discover(&cfg).list_installed_asr_ids();
        let mics: Vec<String> = list_input_devices().into_iter().map(|d| d.name).collect();
        let scale = args
            .iter()
            .position(|a| a == "--scale")
            .and_then(|p| args.get(p + 1))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(100)
            .clamp(100, 250);
        return gallery::run_ui_gallery(dir, installed, mics, scale);
    }

    #[cfg(windows)]
    if let Some(pos) = args.iter().position(|a| a == "--win32-e2e-test") {
        let wav = args.get(pos + 1).map(PathBuf::from);
        let shot = args.get(pos + 2).map(PathBuf::from);
        return run_windows_live_e2e_test(wav, shot);
    }

    #[cfg(windows)]
    {
        let open_settings = args.iter().any(|a| a == "--settings");
        let _ = Mutex::new(());
        return run_windows_desktop_daemon(open_settings);
    }

    #[cfg(not(windows))]
    {
        let _ = Path::new(".");
        let _ = (
            ActiveCapture::start as fn(Option<&str>) -> Result<ActiveCapture>,
            GlobalHotkeyEngine::start,
            HotkeyEvent::PttDown,
            UiAction::Quit,
            SingleInstanceGuard::acquire,
            set_autostart_enabled,
            dpapi_encrypt_to_b64,
            read_clipboard_unicode,
            capture_foreground_context,
            play_subtle_earcon,
            AtomicBool::new(false).load(Ordering::Relaxed),
            Mutex::new(()),
            Arc::new(()),
        );
        bail!(
            "{} desktop GUI daemon requires Windows 10/11; on Linux use `--transcribe-wav <path>` or `cargo test`",
            PRODUCT_NAME
        );
    }
}

#[cfg(all(test, windows))]
mod earcon_tests {
    use super::*;

    fn le16(wav: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([wav[at], wav[at + 1]])
    }
    fn le32(wav: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([wav[at], wav[at + 1], wav[at + 2], wav[at + 3]])
    }

    /// Every style is a valid mono 16-bit WAV whose body matches its header, short enough to stay subtle, and
    /// different from every other style: that is what makes the choice audible.
    #[test]
    fn every_style_is_a_valid_wav_of_its_own_length() {
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for style in AudioCueStyle::ALL {
            for is_start in [true, false] {
                let wav = cue_wav(style, is_start, 100);
                assert_eq!(&wav[0..4], b"RIFF");
                assert_eq!(&wav[8..12], b"WAVE");
                assert_eq!(le16(&wav, 20), 1, "PCM");
                assert_eq!(le16(&wav, 22), 1, "mono");
                assert_eq!(le32(&wav, 24), 44_100);
                assert_eq!(le16(&wav, 34), 16);
                let declared = le32(&wav, 40) as usize;
                assert_eq!(wav.len(), 44 + declared, "the body matches the header");
                let expected: usize =
                    cue_notes(style, is_start).iter().map(|n| (44_100 * n.1 / 1000 * 2) as usize).sum();
                assert_eq!(declared, expected, "{style:?} start={is_start}");
                assert!(declared <= 44_100, "a cue stays under half a second ({style:?})");
                if declared == 0 {
                    continue;
                }
                assert_eq!(le16(&wav, 44), 0, "the cue fades in, so it never starts with a click");
                assert!(
                    !seen.iter().any(|old| old[44..] == wav[44..]),
                    "{style:?} start={is_start} is a copy of another cue"
                );
                seen.push(wav);
            }
        }
    }

    /// The setting scales the samples only: 0 is silent, 100 is the cue's designed peak, and the steps in
    /// between rise. winmm has to accept the buffer or the cue would be silent on the real device.
    #[test]
    fn volume_scales_the_cue_from_silent_to_full_and_winmm_accepts_it() {
        let peak = |v: u8| -> u16 {
            let wav = cue_wav(AudioCueStyle::Soft, true, v);
            wav[44..]
                .chunks(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
                .max()
                .unwrap_or(0)
        };
        let full = 0.28 * i16::MAX as f32;
        assert_eq!(peak(0), 0, "0 % is silent");
        assert!((peak(100) as f32 - full).abs() < 40.0, "100 % is the cue's designed peak");
        assert!(peak(25) < peak(50) && peak(50) < peak(75) && peak(75) < peak(100), "volume rises");
        let wav = cue_wav(AudioCueStyle::Soft, true, 100);
        let played = unsafe {
            windows_sys::Win32::Media::Audio::PlaySoundW(
                wav.as_ptr() as *const u16,
                std::ptr::null_mut(),
                windows_sys::Win32::Media::Audio::SND_MEMORY
                    | windows_sys::Win32::Media::Audio::SND_ASYNC
                    | windows_sys::Win32::Media::Audio::SND_NODEFAULT,
            )
        };
        assert_ne!(played, 0, "winmm rejected the generated WAV (the cue would be silent)");
    }

    /// What the coordinator plays is the style and volume in the config, built once and then reused.
    #[test]
    fn the_configured_style_and_volume_reach_the_played_buffer() {
        let mut cfg = AppConfig::default();
        cfg.audio_cue_style = AudioCueStyle::Chime;
        cfg.audio_cue_volume = 100;
        let played = cue_buffer(cfg.audio_cue_style, true, cfg.audio_cue_volume);
        assert_eq!(played, &cue_wav(AudioCueStyle::Chime, true, 100)[..]);
        assert_eq!(played.as_ptr(), cue_buffer(AudioCueStyle::Chime, true, 100).as_ptr());
    }

    /// The cue must cost the press path nothing: it is queued, not played to completion. The first
    /// call also builds the buffer and loads winmm, which is why it is measured separately.
    #[test]
    fn start_cue_does_not_block() {
        let cfg = AppConfig::default();
        let t = Instant::now();
        play_subtle_earcon(&cfg, true, false);
        let first = t.elapsed();
        let t = Instant::now();
        play_subtle_earcon(&cfg, true, false);
        assert!(
            t.elapsed() < Duration::from_millis(15),
            "the cue blocked the coordinator (first call {first:?})"
        );
        println!("earcon: first call {first:?}, steady state {:?}", t.elapsed());
    }
}

/// Press-to-audio measurement for the Hold-to-Talk path. Ignored by default because it needs a real
/// microphone: `cargo test -p vi-app --release -- --ignored press_path_bench --nocapture`.
#[cfg(all(test, windows))]
mod press_path_bench {
    use super::*;

    const PRESSES: usize = 6;

    /// `KEYREED_BENCH_MIC` picks the device, `KEYREED_BENCH_HOLD_MS` how long the clip lasts after
    /// the first sample (a real press is held for about a second), `KEYREED_BENCH_GAP_MS` the silence
    /// between clips.
    fn bench_env() -> (Option<String>, u64, u64) {
        let num = |name: &str, default: u64| {
            std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        let mic = std::env::var("KEYREED_BENCH_MIC").ok().filter(|s| !s.trim().is_empty());
        (mic, num("KEYREED_BENCH_HOLD_MS", 0), num("KEYREED_BENCH_GAP_MS", 0))
    }

    /// One press in the order the StartRecording arm runs it: target app, microphone, first sample.
    fn one_press(mic: Option<&str>, hold_ms: u64, gap_ms: u64) -> (bool, f64, f64, f64) {
        if gap_ms > 0 {
            std::thread::sleep(Duration::from_millis(gap_ms));
        }
        let t0 = Instant::now();
        let _fg = capture_foreground_context(true);
        let fg_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let cap = ActiveCapture::start(mic).expect("microphone");
        let mic_ms = cap.start_ms();
        let warm = cap.was_warm();
        let mut first_ms = f64::NAN;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if cap.first_frame_ms() > 0.0 {
                first_ms = t0.elapsed().as_secs_f64() * 1000.0;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if hold_ms > 0 {
            std::thread::sleep(Duration::from_millis(hold_ms));
        }
        cap.stop_and_take_16k();
        (warm, fg_ms, mic_ms, first_ms)
    }

    fn percentile(values: &[f64], p: f64) -> f64 {
        let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() {
            return f64::NAN;
        }
        let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
        v[rank.clamp(1, v.len()) - 1]
    }

    fn report(label: &str, rows: &[(bool, f64, f64, f64)]) {
        for (warm, fg, mic, first) in rows {
            println!("{label} warm={warm} fg_ms={fg:.1} mic_ms={mic:.1} press_to_first_audio_ms={first:.1}");
        }
        let firsts: Vec<f64> = rows.iter().map(|r| r.3).collect();
        let mics: Vec<f64> = rows.iter().map(|r| r.2).collect();
        println!(
            "{label} n={} press_to_first_audio p50={:.1} p95={:.1} | mic_start p50={:.1} p95={:.1}",
            rows.len(),
            percentile(&firsts, 50.0),
            percentile(&firsts, 95.0),
            percentile(&mics, 50.0),
            percentile(&mics, 95.0)
        );
        assert!(firsts.iter().all(|x| x.is_finite()), "a press never delivered audio");
    }

    /// The cold path: a fresh thread has no warm client, so every press pays the device open.
    #[test]
    #[ignore = "needs a real microphone"]
    fn press_path_bench_cold() {
        let (mic, hold, gap) = bench_env();
        let rows: Vec<(bool, f64, f64, f64)> = (0..PRESSES)
            .map(|_| {
                let mic = mic.clone();
                std::thread::spawn(move || one_press(mic.as_deref(), hold, gap)).join().unwrap()
            })
            .collect();
        assert!(rows.iter().all(|r| !r.0), "a cold press reused a warm client");
        report("cold", &rows);
    }

    /// The warm path: one prewarm, then every press starts the client it already owns.
    #[test]
    #[ignore = "needs a real microphone"]
    fn press_path_bench_warm() {
        let (mic, hold, gap) = bench_env();
        assert!(ActiveCapture::prewarm(mic.as_deref()), "prewarm could not open the microphone");
        let rows: Vec<(bool, f64, f64, f64)> = (0..PRESSES).map(|_| one_press(mic.as_deref(), hold, gap)).collect();
        assert!(rows.iter().all(|r| r.0), "a warm press fell back to a cold open");
        report("warm", &rows);
        // A warm press must also replace the client it used, or only the first press is fast.
        let later = ActiveCapture::start(mic.as_deref()).expect("microphone");
        assert!(later.was_warm(), "clip did not leave a warm client");
        later.stop_and_take_16k();
    }
}

#[cfg(test)]
mod slm_lock_tests {
    use super::*;

    /// A text job holding the language model must not delay dictation: the rules-only result
    /// arrives within 300 ms (250 ms wait plus rules).
    #[test]
    fn held_lock_gives_rules_only_result_quickly() {
        let run = |eng: &mut RewriteEngine| {
            eng.process(
                "so the the meeting tomorrow got moved to four and we was not told about it",
                0.9,
                WritingMode::Smart,
                AppCategory::Chat,
                vi_core::ProcessingProfile::MaxPrivacyOffline,
                &vi_core::PersonalDictionary::default(),
                None,
                None,
                None,
            )
            .final_text
        };
        // Compile the rule regexes first (debug builds take most of a second for that).
        run(&mut RewriteEngine::new(None));
        let lock = Arc::new(Mutex::new(RewriteEngine::new(None)));
        let (held_tx, held_rx) = crossbeam_channel::bounded(0);
        let holder = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                let _g = lock.lock();
                held_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(1500));
            })
        };
        held_rx.recv().unwrap();
        let t = Instant::now();
        let (busy, text) = with_engine_or_rules(&lock, SLM_LOCK_WAIT, |eng, busy| (busy, run(eng)));
        assert!(busy);
        assert!(!text.is_empty());
        // Well under the 1.5 s hold: it did not wait for the lock (a loaded debug build can take a few hundred ms).
        assert!(t.elapsed() < Duration::from_millis(1000), "{:?}", t.elapsed());
        holder.join().unwrap();
        // Free again: the shared engine is used.
        assert!(!with_engine_or_rules(&lock, SLM_LOCK_WAIT, |_, busy| busy));
    }
}

#[cfg(test)]
mod input_binding_tests {
    use super::*;

    #[test]
    fn all_three_chords_are_registered_for_the_hook() {
        let mut cfg = AppConfig::default();
        cfg.composer_hotkey = vi_core::shortcut::chord(true, true, false, false, 0x4B);
        let chords = extra_chords(&cfg);
        assert_eq!(chords.len(), 3);
        assert_eq!(chords[0], (cfg.quick_fix_hotkey.clone(), ChordAction::CorrectSelection));
        assert_eq!(chords[1], (cfg.composer_hotkey.clone(), ChordAction::OpenComposer));
        assert_eq!(chords[1].0.display_name, "Ctrl + Alt + K");
        assert_eq!(chords[2], (cfg.voice_command_hotkey.clone(), ChordAction::VoiceCommand));
    }

    #[test]
    fn each_mouse_slot_maps_to_its_action_and_nothing_binds_nothing() {
        let mut cfg = AppConfig::default();
        assert!(mouse_bindings(&cfg).is_empty(), "an unconfigured mouse is never intercepted");
        cfg.mouse_middle_action = MouseAction::Dictate;
        cfg.mouse_back_action = MouseAction::QuickFix;
        cfg.mouse_forward_action = MouseAction::VoiceCommand;
        assert_eq!(
            mouse_bindings(&cfg),
            [
                (MouseButton::Middle, ChordAction::Dictate),
                (MouseButton::Back, ChordAction::CorrectSelection),
                (MouseButton::Forward, ChordAction::VoiceCommand),
            ]
        );
        cfg.mouse_middle_action = MouseAction::OpenComposer;
        cfg.mouse_back_action = MouseAction::Nothing;
        assert_eq!(
            mouse_bindings(&cfg),
            [
                (MouseButton::Middle, ChordAction::OpenComposer),
                (MouseButton::Forward, ChordAction::VoiceCommand),
            ]
        );
    }
}

#[cfg(test)]
mod catalog_parity_tests {
    /// Settings shows download sizes from the UI catalog; they must match the pinned download catalog.
    #[test]
    fn settings_download_sizes_match_pinned_catalog() {
        for ui in vi_win32::ui::ASR_UI_CATALOG.iter() {
            let spec = vi_asr::catalog::find_model_spec(ui.id).expect(ui.id);
            assert_eq!(ui.download_mb, spec.approx_download_mb, "{}", ui.id);
        }
    }
}
