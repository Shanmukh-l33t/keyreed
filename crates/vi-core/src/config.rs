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
            exclude_from_clipboard_history: false,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub schema_version: u32,
    pub app_enabled: bool,
    pub ptt_enabled: bool,
    pub activation_mode: ActivationMode,
    pub ptt_hotkey: HotkeySpec,
    pub cancel_hotkey: HotkeySpec,
    pub mode_cycle_hotkey: HotkeySpec,
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
    /// Allow clicking the floating pill to start/stop dictation.
    pub floating_mic_clickable: bool,
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
            output_mode: OutputMode::DirectTyping,
            default_writing_mode: WritingMode::Smart,
            auto_context_mode: true,
            processing_profile: ProcessingProfile::MaxPrivacyOffline,
            local_asr_model: "parakeet-tdt-0.6b-v2-int8".to_string(),
            local_llm_model: "qwen3.5-0.8b-q4_k_m".to_string(),
            preferred_language: "en".to_string(),
            selected_microphone: None,
            show_floating_mic: true,
            floating_mic_clickable: true,
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
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
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
    fn default_config_uses_auto_insert_and_offline_privacy() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.output_mode, OutputMode::DirectTyping);
        assert_eq!(cfg.processing_profile, ProcessingProfile::MaxPrivacyOffline);
        assert_eq!(cfg.local_asr_model, "parakeet-tdt-0.6b-v2-int8");
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
