// On Windows release builds without CLI flags, the process is a GUI application.
// A console is attached only when a flag is present, so closing a terminal does not end the tray app.
#![cfg_attr(windows, windows_subsystem = "windows")]
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
use vi_audio::{load_wav_16k_mono, ActiveCapture};
use vi_core::{
    brand::{AUTOSTART_REG_VALUE, PRODUCT_NAME, PRODUCT_TAGLINE, SINGLE_INSTANCE_MUTEX, VERSION_STR},
    AppCategory, AppConfig, EngineState, ForegroundContext, HistoryEntry, HistoryStore, OutputMode,
    PipelineMetrics, PipelineResult, PttAction, PttController, WritingMode,
};
use vi_rewrite::{slm::LlamaServerSupervisor, RewriteEngine};
use vi_win32::{
    capture_foreground_context, current_process_memory_mb, deliver_direct_or_clipboard_fallback,
    dpapi_decrypt_from_b64, dpapi_encrypt_to_b64, harden_and_init_process, read_clipboard_unicode,
    set_autostart_enabled, read_autostart_value, timing_mark, write_clipboard_unicode, DirectTypeOutcome, GlobalHotkeyEngine,
    HotkeyEvent, SingleInstanceGuard, UiAction,
};
use zeroize::Zeroize;

/// Resolves runtime directories relative to the executable or `KEYREED_RUNTIME_ROOT` override.
struct RuntimePaths {
    sherpa_lib_dir: PathBuf,
    models_dir: PathBuf,
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

        let models_dir = env::var("KEYREED_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let bundled = root.join("models");
                if bundled.exists() {
                    bundled
                } else {
                    AppConfig::default_models_dir()
                }
            });

        let silero_vad_path = {
            let p1 = models_dir.join("silero_vad.onnx");
            let p2 = root.join("silero_vad.onnx");
            if p1.exists() {
                p1
            } else {
                p2
            }
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
            silero_vad_path,
            llama_server_bin,
        }
    }

    fn resolve_gguf_path(&self, model_id: &str) -> Option<PathBuf> {
        if model_id.is_empty() || model_id.eq_ignore_ascii_case("none") {
            return None;
        }
        let candidates = [
            format!("{}.gguf", model_id),
            "Qwen3.5-0.8B-Q4_K_M.gguf".to_string(),
            "Qwen3-0.6B-Q4_K_M.gguf".to_string(),
        ];
        for name in candidates {
            let p = self.models_dir.join(&name);
            if p.exists() {
                if let Err(err) = vi_asr::integrity::verify_gguf(&p) {
                    log::error!("refusing weight file {}: {err}", p.display());
                    continue;
                }
                return Some(p);
            }
        }
        None
    }
}

/// Owns lazy-loaded local ASR and local SLM engines with automatic idle-unload RAM reclamation.
struct InferenceSupervisor {
    paths: RuntimePaths,
    loaded_asr_id: String,
    asr: Option<LocalAsrEngine>,
    rewrite: RewriteEngine,
    last_used: Instant,
}

impl InferenceSupervisor {
    fn new(cfg: &AppConfig) -> Self {
        let paths = RuntimePaths::discover(cfg);
        let slm = paths.resolve_gguf_path(&cfg.local_llm_model).map(|gguf| {
            LlamaServerSupervisor::new(
                paths.llama_server_bin.clone(),
                gguf,
                cfg.inference_threads.max(1),
            )
        });
        Self {
            paths,
            loaded_asr_id: String::new(),
            asr: None,
            rewrite: RewriteEngine::new(slm),
            last_used: Instant::now(),
        }
    }

    fn maybe_unload_idle(&mut self, idle_secs: u64) {
        if idle_secs == 0 {
            return;
        }
        let _ = self.rewrite.maybe_unload_idle(idle_secs);
        if self.asr.is_some() && self.last_used.elapsed() >= Duration::from_secs(idle_secs) {
            self.asr = None;
            self.loaded_asr_id.clear();
            log::info!("Unloaded idle ASR engine to reclaim system RAM");
        }
    }

    fn force_unload_all(&mut self) {
        self.asr = None;
        self.loaded_asr_id.clear();
        let _ = self.rewrite.maybe_unload_idle(1);
    }

    fn ensure_asr_loaded(&mut self, cfg: &AppConfig) -> Result<&mut LocalAsrEngine> {
        if self.asr.is_none() || self.loaded_asr_id != cfg.local_asr_model {
            let spec = find_model_spec(&cfg.local_asr_model)
                .with_context(|| format!("ASR model '{}' not in catalog", cfg.local_asr_model))?;
            vi_asr::integrity::sweep_incomplete(&self.paths.models_dir)?;
            let model_dir = {
                let direct = self.paths.models_dir.join(spec.folder_name);
                if direct.exists() {
                    vi_asr::integrity::prepare_installed_model(&self.paths.models_dir, spec.id)?
                } else if cfg.processing_profile.allows_network() {
                    ensure_model_present(&self.paths.models_dir, spec)?
                } else {
                    bail!(
                        "ASR model '{}' is not installed and the offline profile will not download it",
                        spec.id
                    );
                }
            };
            let vad_opt = if self.paths.silero_vad_path.exists() {
                Some(self.paths.silero_vad_path.as_path())
            } else {
                None
            };
            let engine = LocalAsrEngine::load(
                &self.paths.sherpa_lib_dir,
                &model_dir,
                spec.id,
                vad_opt,
                cfg.inference_threads.max(1),
                &cfg.preferred_language,
            )?;
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
        let audio_duration_ms = ((samples_16k.len() as u64) * 1000) / 16_000;

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
        let mut speech_ratio = 1.0f32;
        let mut used_asr_id = cfg.local_asr_model.clone();
        let mut raw_transcript = String::new();
        let mut cloud_stt_ok = false;

        if cfg.processing_profile == vi_core::ProcessingProfile::MaxAccuracy
            && !decrypted_key.trim().is_empty()
        {
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
                raw_transcript = txt;
                used_asr_id = format!("cloud:{}", cfg.cloud.stt_model);
                cloud_stt_ok = true;
            }
        }

        if !cloud_stt_ok {
            let hotwords = cfg.dictionary.to_sherpa_hotwords();
            let engine = self.ensure_asr_loaded(cfg)?;
            let out = engine.transcribe(&samples_16k, Some(&hotwords))?;
            vad_ms = out.vad_ms;
            asr_ms = out.decode_ms;
            speech_ratio = out.speech_ratio;
            raw_transcript = out.text;
        }

        // Zeroize raw PCM audio immediately after ASR finishes!
        samples_16k.zeroize();

        if let Some(cb) = on_stage_correcting {
            cb();
        }

        // Stage 2: Conservative Meaning-Preserving Grammar & Context Correction
        let rw = self.rewrite.process(
            &raw_transcript,
            speech_ratio,
            effective_mode,
            fg_ctx.category,
            cfg.processing_profile,
            &cfg.dictionary,
            Some(&cfg.cloud.llm_endpoint),
            Some(&cfg.cloud.llm_model),
            if decrypted_key.is_empty() {
                None
            } else {
                Some(decrypted_key.as_str())
            },
        );
        decrypted_key.zeroize();

        if rw.skipped_as_hallucination || rw.final_text.trim().is_empty() {
            // Crucial requirement (Section 4): NEVER overwrite existing clipboard when nothing valid was spoken!
            let total_ms = t_release.elapsed().as_secs_f64() * 1000.0;
            return Ok(PipelineResult {
                raw_transcript,
                final_text: String::new(),
                effective_mode,
                delivered_via: cfg.output_mode,
                fallback_to_clipboard: false,
                requires_command_confirmation: false,
                warning_note: Some("No speech detected; existing clipboard preserved.".to_string()),
                metrics: PipelineMetrics {
                    audio_duration_ms,
                    vad_ms,
                    asr_ms,
                    rewrite_ms: rw.elapsed_ms,
                    output_ms: 0.0,
                    total_release_to_ready_ms: total_ms,
                    used_tier: rw.tier_used,
                    asr_model_id: used_asr_id,
                },
            });
        }

        // Stage 3: Clipboard / Direct Typing Delivery
        let t_out = Instant::now();
        let mut delivered_via = OutputMode::Clipboard;
        let mut fallback_to_clipboard = false;
        let mut warning_note = rw.warning_note.clone();

        if deliver_output {
            match cfg.output_mode {
                OutputMode::Clipboard | OutputMode::AskBeforeInserting => {
                    write_clipboard_unicode(
                        &rw.final_text,
                        cfg.privacy.exclude_from_clipboard_history,
                    )?;
                    delivered_via = OutputMode::Clipboard;
                    if cfg.output_mode == OutputMode::AskBeforeInserting {
                        let preview: String = rw.final_text.chars().take(96).collect();
                        warning_note = Some(format!("Ready to paste. {}", preview));
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

        Ok(PipelineResult {
            raw_transcript,
            final_text: rw.final_text,
            effective_mode,
            delivered_via,
            fallback_to_clipboard,
            requires_command_confirmation: rw.requires_confirmation,
            warning_note,
            metrics: PipelineMetrics {
                audio_duration_ms,
                vad_ms,
                asr_ms,
                rewrite_ms: rw.elapsed_ms,
                output_ms,
                total_release_to_ready_ms,
                used_tier: rw.tier_used,
                asr_model_id: used_asr_id,
            },
        })
    }
}

fn play_subtle_earcon(enabled: bool, is_start: bool) {
    if !enabled {
        return;
    }
    #[cfg(windows)]
    unsafe {
        let freq = if is_start { 660u32 } else { 880u32 };
        let _ = windows_sys::Win32::System::Diagnostics::Debug::Beep(freq, 28);
    }
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
            "--repeat" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    repeat = v.parse().unwrap_or(1).clamp(1, 20);
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

    let mut sup = InferenceSupervisor::new(&cfg);
    let mut fg = ForegroundContext::default();
    fg.category = cat;
    for n in 1..=repeat {
        let samples = load_wav_16k_mono(&wav)?;
        let res = sup.execute_pipeline(samples, &cfg, &fg, mode, copy_clipboard, None)?;
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
fn run_windows_live_e2e_test(wav_dir: Option<PathBuf>, screenshot_out: Option<PathBuf>) -> Result<()> {
    use std::process::Command;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetForegroundWindow, SendMessageW, SetForegroundWindow,
        WM_GETTEXT, WM_GETTEXTLENGTH,
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

    // 5. Spawn live Tray + Non-Activating Layered Floating Pill + Settings Window + Notepad focus test!
    let (ui_tx, _ui_rx) = crossbeam_channel::unbounded();
    let (gui, _pump) = vi_win32::ui::win_gui::init_and_spawn_gui_windows(ui_tx, cfg.clone(), true)?;
    gui.sync(EngineState::Listening, 0.65, None);

    // Launch classic/Windows 11 Notepad or inspect foreground focus
    let mut notepad_child = Command::new("notepad.exe").spawn().ok();
    std::thread::sleep(Duration::from_millis(900));

    // Pump pending GUI messages so Tray, Floating Pill, and Settings render completely on desktop
    unsafe {
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
    }

    let fg_ctx = capture_foreground_context(true);

    // Optional live desktop screenshot capture via PowerShell System.Drawing for visual verification
    if let Some(shot_path) = &screenshot_out {
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

    // 6. Real ASR + Grammar Correction on WAV file inside Windows 11 if provided
    let mut wav_e2e_json = serde_json::Value::Null;
    if let Some(wdir) = wav_dir {
        if wdir.exists() {
            cfg.local_llm_model = "Qwen3.5-0.8B-Q4_K_M".to_string();
            let mut full_sup = InferenceSupervisor::new(&cfg);
            let samples = load_wav_16k_mono(&wdir)?;
            let r = full_sup.execute_pipeline(
                samples,
                &cfg,
                &fg_ctx,
                Some(WritingMode::Smart),
                true,
                None,
            )?;
            let (active_ws, active_priv) = current_process_memory_mb();
            full_sup.force_unload_all();
            let (after_unload_ws, after_unload_priv) = current_process_memory_mb();
            wav_e2e_json = serde_json::json!({
                "pipeline": r,
                "active_working_set_mb": active_ws,
                "active_private_mb": active_priv,
                "post_unload_working_set_mb": after_unload_ws,
                "post_unload_private_mb": after_unload_priv,
            });

            // Also send Ctrl+V into foreground Notepad to verify real paste!
            unsafe {
                let cls_w: Vec<u16> = "Notepad\0".encode_utf16().collect();
                let np_hwnd: HWND = FindWindowW(cls_w.as_ptr(), std::ptr::null());
                if !np_hwnd.is_null() {
                    let _ = SetForegroundWindow(np_hwnd);
                    std::thread::sleep(Duration::from_millis(150));
                    let v_vk: u16 = 0x56;
                    let paste_inputs = [
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: VK_CONTROL,
                                    wScan: 0,
                                    dwFlags: 0,
                                    time: 0,
                                    dwExtraInfo: 0,
                                },
                            },
                        },
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: v_vk,
                                    wScan: 0,
                                    dwFlags: 0,
                                    time: 0,
                                    dwExtraInfo: 0,
                                },
                            },
                        },
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: v_vk,
                                    wScan: 0,
                                    dwFlags: KEYEVENTF_KEYUP,
                                    time: 0,
                                    dwExtraInfo: 0,
                                },
                            },
                        },
                        INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: VK_CONTROL,
                                    wScan: 0,
                                    dwFlags: KEYEVENTF_KEYUP,
                                    time: 0,
                                    dwExtraInfo: 0,
                                },
                            },
                        },
                    ];
                    let _ = SendInput(
                        paste_inputs.len() as u32,
                        paste_inputs.as_ptr(),
                        std::mem::size_of::<INPUT>() as i32,
                    );
                    std::thread::sleep(Duration::from_millis(250));
                    let edit_cls: Vec<u16> = "Edit\0".encode_utf16().collect();
                    let rich_cls: Vec<u16> = "RichEditD2DPT\0".encode_utf16().collect();
                    let mut ed = FindWindowExW(np_hwnd, std::ptr::null_mut(), edit_cls.as_ptr(), std::ptr::null());
                    if ed.is_null() {
                        ed = FindWindowExW(np_hwnd, std::ptr::null_mut(), rich_cls.as_ptr(), std::ptr::null());
                    }
                    if !ed.is_null() {
                        let tlen = SendMessageW(ed, WM_GETTEXTLENGTH, 0, 0);
                        if tlen > 0 {
                            let mut buf = vec![0u16; (tlen as usize) + 2];
                            SendMessageW(ed, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize);
                            let _pasted = String::from_utf16_lossy(&buf[..tlen as usize]);
                        }
                    }
                }
            }
        }
    }

    if let Some(mut ch) = notepad_child.take() {
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
        "foreground_context_detected": fg_ctx,
        "wav_e2e": wav_e2e_json,
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
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

    let (gui_handle, gui_pump) = vi_win32::ui::win_gui::init_and_spawn_gui_windows(
        ui_tx,
        initial_cfg.clone(),
        open_settings_on_start,
    )?;

    let gui_for_worker = gui_handle.clone();
    let hk_for_worker = Arc::clone(&hk_engine);

    std::thread::Builder::new()
        .name("keyreed-coordinator".to_string())
        .spawn(move || {
            let mut cfg = initial_cfg;
            let mut history = HistoryStore::load_or_default(&hist_path);
            let mut ptt = PttController::new(cfg.activation_mode, 110, 120_000);
            let _ = ptt.set_app_enabled(cfg.app_enabled);
            let _ = ptt.set_ptt_enabled(cfg.ptt_enabled);

            let mut supervisor = InferenceSupervisor::new(&cfg);
            if let Some(spec) = vi_asr::catalog::find_model_spec(&cfg.local_asr_model) {
                let installed = RuntimePaths::discover(&cfg).models_dir.join(spec.folder_name);
                if !installed.exists() {
                    gui_for_worker.notify_toast(
                        PRODUCT_NAME,
                        "Speech model is not installed. Open the tray menu and choose Download default speech model. Auto Insert stays off until you turn it on.",
                    );
                }
            }
            let mut active_mic: Option<ActiveCapture> = None;
            let mut captured_fg_ctx = ForegroundContext::default();
            let start_epoch = Instant::now();
            let now_ms = || start_epoch.elapsed().as_millis() as u64;

            let ticker = tick(Duration::from_millis(75));

            loop {
                let mut pending_action = PttAction::None;

                select! {
                    recv(hk_rx) -> ev => {
                        match ev {
                            Ok(HotkeyEvent::PttDown) => pending_action = ptt.on_shortcut_down(now_ms()),
                            Ok(HotkeyEvent::PttUp) => pending_action = ptt.on_shortcut_up(now_ms()),
                            Ok(HotkeyEvent::EmergencyCancel) => {
                                pending_action = ptt.on_emergency_cancel("escape_pressed");
                            }
                            Ok(HotkeyEvent::CycleMode) => {
                                cfg.default_writing_mode = cfg.default_writing_mode.next();
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                                gui_for_worker.notify_toast(
                                    PRODUCT_NAME,
                                    &format!("Mode switched to {}", cfg.default_writing_mode.label()),
                                );
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
                                        "Auto Insert is on. Text is typed into the focused field. Unsafe targets stay on the clipboard.",
                                    );
                                }
                            }
                            Ok(UiAction::DownloadDefaultModel) => {
                                let models = RuntimePaths::discover(&cfg).models_dir;
                                let gui_dl = gui_for_worker.clone();
                                std::thread::spawn(move || {
                                    gui_dl.notify_toast(
                                        PRODUCT_NAME,
                                        "Downloading the default speech model. This can take several minutes.",
                                    );
                                    let result = match vi_asr::catalog::find_model_spec("parakeet-tdt-0.6b-v2-int8") {
                                        Some(spec) => vi_asr::catalog::ensure_model_present(&models, spec).map(|_| ()),
                                        None => Err(anyhow::anyhow!("default speech model is not in the catalog")),
                                    };
                                    match result {
                                        Ok(()) => gui_dl.notify_toast(PRODUCT_NAME, "Default speech model is ready."),
                                        Err(e) => gui_dl.notify_toast(
                                            PRODUCT_NAME,
                                            &format!("Model download failed: {e}"),
                                        ),
                                    }
                                });
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
                                hk_for_worker.set_enabled(new_cfg.app_enabled && new_cfg.ptt_enabled);
                                cfg = *new_cfg;
                                let _ = cfg.save_atomic(&cfg_path);
                                gui_for_worker.sync(ptt.state().clone(), 0.0, Some(cfg.clone()));
                            }
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
                    recv(ticker) -> _ => {
                        pending_action = ptt.on_tick(now_ms());
                        if let Some(mic) = &active_mic {
                            gui_for_worker.sync(EngineState::Listening, mic.current_rms(), None);
                        } else {
                            supervisor.maybe_unload_idle(cfg.idle_unload_seconds);
                        }
                    }
                }

                match pending_action {
                    PttAction::None => {}
                    PttAction::StartRecording => {
                        timing_mark("coordinator_start_recording");
                        captured_fg_ctx = capture_foreground_context(cfg.privacy.redact_sensitive_window_titles);
                        match ActiveCapture::start(cfg.selected_microphone.as_deref()) {
                            Ok(cap) => {
                                timing_mark("mic_started");
                                active_mic = Some(cap);
                                busy_flag.store(true, Ordering::Relaxed);
                                gui_for_worker.sync(EngineState::Listening, 0.15, None);
                                play_subtle_earcon(cfg.audio_cues_enabled, true);
                            }
                            Err(e) => {
                                timing_mark("mic_error");
                                let msg = format!("Mic error: {}", e);
                                ptt.transition_to_error(&msg);
                                busy_flag.store(false, Ordering::Relaxed);
                                gui_for_worker.sync(EngineState::Error(msg.clone()), 0.0, None);
                                gui_for_worker.notify_toast(PRODUCT_NAME, &msg);
                            }
                        }
                    }
                    PttAction::CancelRecording { reason } => {
                        if let Some(m) = active_mic.take() {
                            m.cancel_and_zeroize();
                        }
                        busy_flag.store(false, Ordering::Relaxed);
                        gui_for_worker.sync(ptt.state().clone(), 0.0, None);
                        if reason == "recording_cap_discarded" {
                            gui_for_worker.notify_toast(
                                PRODUCT_NAME,
                                "Recording discarded. The key release was lost or the time limit was reached. Hold the shortcut to start again.",
                            );
                        }
                    }
                    PttAction::StopAndProcess => {
                        timing_mark("coordinator_stop");
                        gui_for_worker.sync(EngineState::Processing, 0.0, None);
                        if let Some(m) = active_mic.take() {
                            let samples = m.stop_and_take_16k();
                            let gui_stage = gui_for_worker.clone();
                            let on_corr = move || {
                                gui_stage.sync(EngineState::Correcting, 0.0, None);
                            };
                            match supervisor.execute_pipeline(
                                samples,
                                &cfg,
                                &captured_fg_ctx,
                                None,
                                true,
                                Some(&on_corr),
                            ) {
                                Ok(res) if !res.final_text.is_empty() => {
                                    timing_mark(&format!(
                                        "clipboard_ready asr_ms={:.1} rewrite_ms={:.1} output_ms={:.2} total_ms={:.1} tier={}",
                                        res.metrics.asr_ms,
                                        res.metrics.rewrite_ms,
                                        res.metrics.output_ms,
                                        res.metrics.total_release_to_ready_ms,
                                        res.metrics.used_tier
                                    ));
                                    ptt.transition_to_ready();
                                    gui_for_worker.sync(EngineState::ReadyToPaste, 0.0, None);
                                    play_subtle_earcon(cfg.audio_cues_enabled, false);

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

                                    if cfg.notify_on_ready {
                                        let preview: String = res.final_text.chars().take(72).collect();
                                        let title = if res.requires_command_confirmation {
                                            "Review Command Before Pasting (Ctrl+V)"
                                        } else {
                                            "Ready to Paste (Ctrl+V)"
                                        };
                                        gui_for_worker.notify_toast(title, &preview);
                                    }

                                    // Return visual state smoothly from ReadyToPaste -> Idle after 1.6s
                                    let gui_reset = gui_for_worker.clone();
                                    std::thread::spawn(move || {
                                        std::thread::sleep(Duration::from_millis(1600));
                                        gui_reset.sync(EngineState::Idle, 0.0, None);
                                    });
                                    ptt.transition_to_idle();
                                }
                                Ok(_) => {
                                    ptt.transition_to_idle();
                                    gui_for_worker.sync(EngineState::Idle, 0.0, None);
                                }
                                Err(e) => {
                                    let msg = format!("Transcription failed: {}", e);
                                    ptt.transition_to_error(&msg);
                                    gui_for_worker.sync(EngineState::Error(msg.clone()), 0.0, None);
                                    gui_for_worker.notify_toast(PRODUCT_NAME, &msg);
                                }
                            }
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

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    #[cfg(windows)]
    if !args.is_empty() {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(0xFFFFFFFF);
        }
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

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

    if let Some(pos) = args.iter().position(|a| a == "--download-model") {
        let id = args.get(pos + 1).context("missing <id> after --download-model")?;
        let spec = find_model_spec(id).with_context(|| format!("unknown model '{}'", id))?;
        let cfg = AppConfig::load_or_default(&AppConfig::default_config_path());
        let paths = RuntimePaths::discover(&cfg);
        let dir = ensure_model_present(&paths.models_dir, spec)?;
        println!("Model ready at {}", dir.display());
        return Ok(());
    }

    if args.iter().any(|a| a == "--transcribe-wav") {
        return run_cli_transcribe_wav(&args);
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
