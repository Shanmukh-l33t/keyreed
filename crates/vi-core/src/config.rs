use crate::brand::PRODUCT_SLUG;
use crate::dictionary::PersonalDictionary;
use crate::types::{ActivationMode, HotkeySpec, OutputMode, ProcessingProfile, WritingMode};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettings {
    /// Exclude dictated clipboard writes from Windows Clipboard History (Win+V) and Cloud Sync.
    /// On by default; only a config that does not mention the key gets the default, saved choices stay.
    #[serde(default = "default_true")]
    pub exclude_from_clipboard_history: bool,
    /// Block direct typing into password fields (always recommended true).
    pub block_password_fields: bool,
    /// Skip reading window titles in private/incognito browsers and password managers.
    pub redact_sensitive_window_titles: bool,
    /// Never write temporary WAV files to disk; keep audio strictly in RAM and zeroize after decode.
    pub ram_only_audio: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            exclude_from_clipboard_history: true,
            block_password_fields: true,
            redact_sensitive_window_titles: true,
            ram_only_audio: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSettings {
    /// Optional OpenAI-compatible STT endpoint (e.g. Groq Whisper or OpenAI).
    pub stt_endpoint: String,
    pub stt_model: String,
    /// Optional OpenAI-compatible LLM rewrite endpoint (e.g. Groq, Cerebras, Gemini OpenAI endpoint).
    pub llm_endpoint: String,
    pub llm_model: String,
    /// DPAPI-encrypted Base64 blob on Windows (never plaintext API key!).
    pub encrypted_api_key_b64: Option<String>,
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            stt_endpoint: "https://api.groq.com/openai/v1/audio/transcriptions".to_string(),
            stt_model: "whisper-large-v3-turbo".to_string(),
            llm_endpoint: "https://api.groq.com/openai/v1/chat/completions".to_string(),
            llm_model: "openai/gpt-oss-20b".to_string(),
            encrypted_api_key_b64: None,
        }
    }
}

/// What a click on the floating pill does (plan C3). While a recording runs, a click never opens anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PillClickAction {
    /// Not offered in 2.0 beta 1: the Composer is not wired to a writing model yet (the setting is kept for later).
    OpenComposer,
    #[default]
    Dictate,
    Nothing,
}

impl PillClickAction {
    pub const ALL: [PillClickAction; 2] = [PillClickAction::Dictate, PillClickAction::Nothing];

    pub fn label(self) -> &'static str {
        match self {
            PillClickAction::OpenComposer => "Open the Composer",
            PillClickAction::Dictate => "Start or stop dictation",
            PillClickAction::Nothing => "Nothing",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub schema_version: u32,
    pub app_enabled: bool,
    pub ptt_enabled: bool,
    pub activation_mode: ActivationMode,
    pub ptt_hotkey: HotkeySpec,
    pub cancel_hotkey: HotkeySpec,
    pub mode_cycle_hotkey: HotkeySpec,
    /// Fix the selected text in any application (plan F1). Proposed default, not yet checked against every app.
    #[serde(default = "default_quick_fix_hotkey")]
    pub quick_fix_hotkey: HotkeySpec,
    /// Quick Fix shows every change in the Review panel before anything is replaced. Turning this off lets edits
    /// the edit-class table marks silent be applied without asking.
    #[serde(default = "default_true")]
    pub quick_fix_always_review: bool,
    /// The one-time explanation in the Review panel was dismissed.
    #[serde(default)]
    pub quick_fix_intro_seen: bool,
    /// Open the Composer (plan F2).
    #[serde(default = "default_composer_hotkey")]
    pub composer_hotkey: HotkeySpec,
    pub output_mode: OutputMode,
    pub default_writing_mode: WritingMode,
    pub auto_context_mode: bool,
    pub processing_profile: ProcessingProfile,
    /// Selected local ASR model ID from catalog (e.g., "parakeet-tdt-0.6b-v2-int8").
    pub local_asr_model: String,
    /// Selected local GGUF rewrite model ID (e.g., "qwen3.5-0.8b-q4_k_m" or "none" for Tier-0-only low-RAM PCs).
    pub local_llm_model: String,
    /// Preferred language ("auto", "en", "hi", "es", etc.).
    pub preferred_language: String,
    /// Specific microphone device ID/name (None = resolved system default input device).
    pub selected_microphone: Option<String>,
    /// Show floating microphone pill on desktop.
    pub show_floating_mic: bool,
    /// What a click on the floating pill does (replaces `floating_mic_clickable`; see `migrate`).
    #[serde(default)]
    pub pill_click_action: PillClickAction,
    /// Play subtle non-intrusive earcon tones on start/ready/error.
    pub audio_cues_enabled: bool,
    /// Show subtle completion notification when clipboard text is ready.
    pub notify_on_ready: bool,
    /// Unload heavy local models after N seconds of inactivity (0 = keep warm in RAM).
    pub idle_unload_seconds: u64,
    /// Number of CPU threads for local inference (0 = auto physical-core cap).
    pub inference_threads: usize,
    pub autostart_on_login: bool,
    /// Saved floating microphone pill screen position `(x, y)`.
    #[serde(default)]
    pub floating_pill_pos: Option<(i32, i32)>,
    /// Tracks whether first-run setup/onboarding has been shown.
    #[serde(default)]
    pub first_run_completed: bool,
    pub privacy: PrivacySettings,
    pub cloud: CloudSettings,
    pub dictionary: PersonalDictionary,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            app_enabled: true,
            ptt_enabled: true,
            activation_mode: ActivationMode::HoldToTalk,
            ptt_hotkey: HotkeySpec::right_ctrl(),
            cancel_hotkey: HotkeySpec::cancel_escape(),
            mode_cycle_hotkey: HotkeySpec::ctrl_shift_space(),
            quick_fix_hotkey: default_quick_fix_hotkey(),
            quick_fix_always_review: true,
            quick_fix_intro_seen: false,
            composer_hotkey: default_composer_hotkey(),
            output_mode: OutputMode::DirectTyping,
            default_writing_mode: WritingMode::Smart,
            auto_context_mode: true,
            processing_profile: ProcessingProfile::MaxPrivacyOffline,
            local_asr_model: "parakeet-tdt-0.6b-v2-int8".to_string(),
            local_llm_model: "qwen3.5-0.8b-q4_k_m".to_string(),
            preferred_language: "en".to_string(),
            selected_microphone: None,
            show_floating_mic: true,
            pill_click_action: PillClickAction::Dictate,
            audio_cues_enabled: true,
            notify_on_ready: true,
            idle_unload_seconds: 300,
            inference_threads: 4,
            autostart_on_login: false,
            floating_pill_pos: None,
            first_run_completed: false,
            privacy: PrivacySettings::default(),
            cloud: CloudSettings::default(),
            dictionary: PersonalDictionary::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_quick_fix_hotkey() -> HotkeySpec {
    crate::shortcut::chord(true, false, false, true, 0x47) // Ctrl + Win + G
}

fn default_composer_hotkey() -> HotkeySpec {
    crate::shortcut::chord(true, false, false, true, 0x4B) // Ctrl + Win + K
}

impl AppConfig {
    pub fn default_data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(PRODUCT_SLUG)
    }

    pub fn default_config_path() -> PathBuf {
        Self::default_data_dir().join("config.json")
    }

    pub fn default_history_path() -> PathBuf {
        Self::default_data_dir().join("history.json")
    }

    pub fn default_models_dir() -> PathBuf {
        Self::default_data_dir().join("models")
    }

    pub fn load_or_default(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => {
                let (cfg, dropped) = Self::parse_lenient(&raw);
                if !dropped.is_empty() {
                    // Keep the user's original file so nothing is silently lost.
                    let _ = fs::write(path.with_file_name("config.invalid.json"), &raw);
                    eprintln!("KeyReed: ignored invalid settings {dropped:?}; original saved as config.invalid.json");
                }
                cfg
            }
            Err(_) => Self::default(),
        }
    }

    /// Parses a config file, keeping every valid top-level setting and falling back to the
    /// default only for settings that are missing or invalid. Returns the names of dropped keys.
    pub fn parse_lenient(raw: &str) -> (Self, Vec<String>) {
        let (mut cfg, dropped) = Self::parse_fields(raw);
        if let Ok(serde_json::Value::Object(user)) = serde_json::from_str::<serde_json::Value>(raw) {
            cfg.migrate(&user);
        }
        (cfg, dropped)
    }

    /// Settings renamed since an older version: `floating_mic_clickable` false becomes "Nothing", true becomes
    /// "Open the Composer". A value under the new name always wins.
    fn migrate(&mut self, user: &serde_json::Map<String, serde_json::Value>) {
        if !user.contains_key("pill_click_action") {
            if let Some(clickable) = user.get("floating_mic_clickable").and_then(|v| v.as_bool()) {
                self.pill_click_action = if clickable { PillClickAction::Dictate } else { PillClickAction::Nothing };
            }
        }
    }

    fn parse_fields(raw: &str) -> (Self, Vec<String>) {
        if let Ok(cfg) = serde_json::from_str::<Self>(raw) {
            return (cfg, Vec::new());
        }
        let Ok(serde_json::Value::Object(user)) = serde_json::from_str::<serde_json::Value>(raw) else {
            return (Self::default(), vec!["<unreadable file>".to_string()]);
        };
        let Ok(mut merged) = serde_json::to_value(Self::default()) else {
            return (Self::default(), vec!["<internal>".to_string()]);
        };
        let mut dropped = Vec::new();
        for (k, v) in user {
            let mut trial = merged.clone();
            if let Some(obj) = trial.as_object_mut() {
                if !obj.contains_key(&k) {
                    continue; // unknown key from another version: ignore quietly
                }
                obj.insert(k.clone(), v);
            }
            if serde_json::from_value::<Self>(trial.clone()).is_ok() {
                merged = trial;
            } else {
                dropped.push(k);
            }
        }
        (serde_json::from_value(merged).unwrap_or_default(), dropped)
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config directory {}", parent.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_invalid_setting_does_not_wipe_the_others() {
        let mut cfg = AppConfig::default();
        cfg.default_writing_mode = WritingMode::Technical;
        cfg.local_asr_model = "moonshine-base-en-int8".into();
        let mut v = serde_json::to_value(&cfg).unwrap();
        v["output_mode"] = serde_json::json!("DirectTyping"); // wrong case: invalid
        v["future_setting"] = serde_json::json!(true); // unknown key: ignored
        let (loaded, dropped) = AppConfig::parse_lenient(&v.to_string());
        assert_eq!(dropped, vec!["output_mode".to_string()]);
        assert_eq!(loaded.default_writing_mode, WritingMode::Technical);
        assert_eq!(loaded.local_asr_model, "moonshine-base-en-int8");
        assert_eq!(loaded.output_mode, OutputMode::DirectTyping); // default for the dropped key

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, v.to_string()).unwrap();
        let reloaded = AppConfig::load_or_default(&path);
        assert_eq!(reloaded.local_asr_model, "moonshine-base-en-int8");
        assert!(dir.path().join("config.invalid.json").exists(), "original kept for the user");
    }

    #[test]
    fn default_config_uses_auto_insert_and_offline_privacy() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.output_mode, OutputMode::DirectTyping);
        assert_eq!(cfg.processing_profile, ProcessingProfile::MaxPrivacyOffline);
        assert_eq!(cfg.local_asr_model, "parakeet-tdt-0.6b-v2-int8");
    }

    #[test]
    fn clipboard_history_exclusion_defaults_on_but_keeps_a_saved_choice() {
        assert!(AppConfig::default().privacy.exclude_from_clipboard_history);
        let (c, _) = AppConfig::parse_lenient("{}");
        assert!(c.privacy.exclude_from_clipboard_history);
        // An older file whose privacy block predates the key.
        let old = r#"{"privacy": {"block_password_fields": true, "redact_sensitive_window_titles": true, "ram_only_audio": true}}"#;
        let (c, dropped) = AppConfig::parse_lenient(old);
        assert!(dropped.is_empty() && c.privacy.exclude_from_clipboard_history);
        let saved = r#"{"privacy": {"exclude_from_clipboard_history": false, "block_password_fields": true, "redact_sensitive_window_titles": true, "ram_only_audio": true}}"#;
        let (c, _) = AppConfig::parse_lenient(saved);
        assert!(!c.privacy.exclude_from_clipboard_history);
    }

    #[test]
    fn config_persists_and_reloads_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = AppConfig::default();
        cfg.selected_microphone = Some("Microphone (OWL 4K PRO Audio)".to_string());
        cfg.floating_pill_pos = Some((420, 880));
        cfg.first_run_completed = true;
        cfg.save_atomic(&path).unwrap();

        let loaded = AppConfig::load_or_default(&path);
        assert_eq!(
            loaded.selected_microphone.as_deref(),
            Some("Microphone (OWL 4K PRO Audio)")
        );
        assert_eq!(loaded.floating_pill_pos, Some((420, 880)));
        assert!(loaded.first_run_completed);
    }

    #[test]
    fn quick_fix_asks_before_replacing_by_default() {
        let (c, _) = AppConfig::parse_lenient("{}");
        assert!(c.quick_fix_always_review);
        assert!(!c.quick_fix_intro_seen);
        let (c, _) = AppConfig::parse_lenient(r#"{"quick_fix_always_review": false}"#);
        assert!(!c.quick_fix_always_review);
    }

    #[test]
    fn floating_mic_clickable_migrates_to_pill_click_action() {
        let (c, _) = AppConfig::parse_lenient(r#"{"floating_mic_clickable": false}"#);
        assert_eq!(c.pill_click_action, PillClickAction::Nothing);
        let (c, _) = AppConfig::parse_lenient(r#"{"floating_mic_clickable": true}"#);
        assert_eq!(c.pill_click_action, PillClickAction::Dictate);
        let (c, _) = AppConfig::parse_lenient(r#"{"floating_mic_clickable": false, "pill_click_action": "dictate"}"#);
        assert_eq!(c.pill_click_action, PillClickAction::Dictate, "the new setting wins");
        let (c, _) = AppConfig::parse_lenient("{}");
        assert_eq!(c.pill_click_action, PillClickAction::Dictate);
        // A full old file (every other setting valid) takes the fast path and is migrated too.
        let mut old = serde_json::to_value(AppConfig::default()).unwrap();
        let o = old.as_object_mut().unwrap();
        o.remove("pill_click_action");
        o.insert("floating_mic_clickable".into(), serde_json::Value::Bool(false));
        let (c, dropped) = AppConfig::parse_lenient(&old.to_string());
        assert!(dropped.is_empty());
        assert_eq!(c.pill_click_action, PillClickAction::Nothing);
    }

    #[test]
    fn hotkey_conflict_detection_blocks_reserved_chords() {
        let ctrl_c = HotkeySpec {
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
            vk: 0x43,
            display_name: "Ctrl + C".to_string(),
        };
        assert!(ctrl_c.check_conflict(None).is_some());

        let rctrl = HotkeySpec::right_ctrl();
        assert!(rctrl.check_conflict(None).is_none());
        assert!(rctrl.check_conflict(Some(&rctrl)).is_some());
    }
}
