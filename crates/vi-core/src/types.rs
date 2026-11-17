use serde::{Deserialize, Serialize};
use std::fmt;

/// Writing and transcription modes (Master Prompt Section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WritingMode {
    #[default]
    Smart,
    Exact,
    Professional,
    Casual,
    Technical,
    TerminalCode,
}

impl WritingMode {
    pub const ALL: [WritingMode; 6] = [
        WritingMode::Smart,
        WritingMode::Exact,
        WritingMode::Professional,
        WritingMode::Casual,
        WritingMode::Technical,
        WritingMode::TerminalCode,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WritingMode::Smart => "Smart Writing",
            WritingMode::Exact => "Exact Dictation",
            WritingMode::Professional => "Professional Writing",
            WritingMode::Casual => "Casual Writing",
            WritingMode::Technical => "Technical Mode",
            WritingMode::TerminalCode => "Terminal / Code Mode",
        }
    }

    /// Compact label used where space is tight (floating listener, menus).
    pub fn short_label(self) -> &'static str {
        match self {
            WritingMode::Smart => "Smart",
            WritingMode::Exact => "Exact",
            WritingMode::Professional => "Professional",
            WritingMode::Casual => "Casual",
            WritingMode::Technical => "Technical",
            WritingMode::TerminalCode => "Code",
        }
    }

    /// One-sentence, plain-language explanation shown in Settings and onboarding.
    pub fn description(self) -> &'static str {
        match self {
            WritingMode::Smart => "Removes filler words and false starts, fixes punctuation and casing.",
            WritingMode::Exact => "Keeps every word you said. Only adds basic casing and punctuation.",
            WritingMode::Professional => "Clear, workplace-ready phrasing for email and documents.",
            WritingMode::Casual => "Relaxed tone for chat. Keeps contractions and your phrasing.",
            WritingMode::Technical => "Preserves file names, identifiers, versions, paths and numbers.",
            WritingMode::TerminalCode => "Turns spoken flags and symbols into commands. Never presses Enter.",
        }
    }

    /// A real before/after pair produced by the deterministic Tier-0 engine.
    /// `vi-rewrite` has a regression test that asserts each pair, so the UI never shows invented output.
    pub fn example(self) -> (&'static str, &'static str) {
        match self {
            WritingMode::Smart => (
                "um i i think the the build pipeline is stuck on step 3 uh again",
                "I think the build pipeline is stuck on step 3 again.",
            ),
            WritingMode::Exact => (
                "um so the build is stuck on step 3",
                "Um so the build is stuck on step 3.",
            ),
            WritingMode::Professional => (
                "please revert back once the deployment is completed on staging",
                "Please reply once the deployment is completed on staging.",
            ),
            WritingMode::Casual => (
                "i will be 10 mins late stuck in traffic",
                "I'll be 10 mins late stuck in traffic",
            ),
            WritingMode::Technical => (
                "update auth underscore middleware dot rs to return http 401",
                "Update auth_middleware.rs to return HTTP 401.",
            ),
            WritingMode::TerminalCode => (
                "git checkout dash b fix slash login",
                "git checkout -b fix/login",
            ),
        }
    }

    pub fn next(self) -> Self {
        match self {
            WritingMode::Smart => WritingMode::Exact,
            WritingMode::Exact => WritingMode::Professional,
            WritingMode::Professional => WritingMode::Casual,
            WritingMode::Casual => WritingMode::Technical,
            WritingMode::Technical => WritingMode::TerminalCode,
            WritingMode::TerminalCode => WritingMode::Smart,
        }
    }
}

impl fmt::Display for WritingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Push-to-Talk activation behaviour (Master Prompt Section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    #[default]
    HoldToTalk,
    ToggleToTalk,
}

impl ActivationMode {
    pub fn label(self) -> &'static str {
        match self {
            ActivationMode::HoldToTalk => "Hold to Talk",
            ActivationMode::ToggleToTalk => "Toggle to Talk",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ActivationMode::HoldToTalk => "Hold the shortcut while you speak. Release to finish.",
            ActivationMode::ToggleToTalk => "Press once to start, press again to finish.",
        }
    }
}

/// Output delivery mode. Auto Insert (`DirectTyping`) is the default primary experience.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Copies the finished text and waits for Ctrl+V.
    Clipboard,
    /// Automatically inserts text into the focused field; falls back to the clipboard if unsafe.
    #[default]
    DirectTyping,
    /// Shows a confirmation prompt before inserting into the focused target window.
    AskBeforeInserting,
}

impl OutputMode {
    pub fn label(self) -> &'static str {
        match self {
            OutputMode::DirectTyping => "Auto Insert (Recommended)",
            OutputMode::Clipboard => "Clipboard Only (Ctrl+V)",
            OutputMode::AskBeforeInserting => "Ask Before Inserting",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            OutputMode::DirectTyping => "Auto Insert",
            OutputMode::Clipboard => "Clipboard only",
            OutputMode::AskBeforeInserting => "Ask before inserting",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            OutputMode::DirectTyping => "Types the text into the app you were using. Password boxes, admin windows and focus changes fall back to the clipboard.",
            OutputMode::Clipboard => "Copies the text. You paste it yourself with Ctrl+V.",
            OutputMode::AskBeforeInserting => "Shows a preview first. Yes inserts, No copies, Cancel discards.",
        }
    }
}

/// Processing & privacy preference tier (Master Prompt Section 8 & 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingProfile {
    /// 100% local processing. Network calls for audio or text are strictly forbidden.
    #[default]
    MaxPrivacyOffline,
    /// Local ASR + local SLM by default; optional cloud fallback on long/low-confidence clips when configured.
    Balanced,
    /// Uses configured cloud STT/LLM when online for highest accuracy; seamlessly falls back to local offline.
    MaxAccuracy,
}

impl ProcessingProfile {
    pub fn label(self) -> &'static str {
        match self {
            ProcessingProfile::MaxPrivacyOffline => "Maximum Privacy (100% Offline)",
            ProcessingProfile::Balanced => "Balanced (Local First)",
            ProcessingProfile::MaxAccuracy => "Maximum Accuracy (Hybrid / Cloud)",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ProcessingProfile::MaxPrivacyOffline => "On this PC only",
            ProcessingProfile::Balanced => "Local first",
            ProcessingProfile::MaxAccuracy => "Cloud when available",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ProcessingProfile::MaxPrivacyOffline => "Audio and text never leave this computer. Recommended.",
            ProcessingProfile::Balanced => "Runs locally. May use your cloud key for long or unclear recordings.",
            ProcessingProfile::MaxAccuracy => "Uses your cloud key when online, falls back to local offline.",
        }
    }

    pub fn allows_network(self) -> bool {
        !matches!(self, ProcessingProfile::MaxPrivacyOffline)
    }
}

/// Foreground application semantic category (Master Prompt Section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    Email,
    Chat,
    Document,
    CodeEditor,
    Terminal,
    Notes,
    Browser,
    PasswordProtected,
    #[default]
    General,
}

impl AppCategory {
    /// Suggests the default writing mode when automatic context adaptation is enabled.
    pub fn recommended_mode(self, fallback: WritingMode) -> WritingMode {
        match self {
            AppCategory::Email => WritingMode::Professional,
            AppCategory::Chat => WritingMode::Casual,
            AppCategory::Document => WritingMode::Smart,
            AppCategory::CodeEditor => WritingMode::Technical,
            AppCategory::Terminal => WritingMode::TerminalCode,
            AppCategory::Notes => WritingMode::Smart,
            AppCategory::PasswordProtected => WritingMode::Exact,
            AppCategory::Browser | AppCategory::General => fallback,
        }
    }
}

/// Minimal, privacy-preserving snapshot of the user's active focus target.
/// Never captures full document text or screenshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ForegroundContext {
    pub hwnd: u64,
    pub process_id: u32,
    pub process_name: String,
    pub window_class: String,
    /// Redacted automatically for InPrivate/Incognito or blocklisted password managers.
    pub window_title_hint: String,
    pub control_type: String,
    pub is_password_field: bool,
    pub is_elevated: bool,
    pub category: AppCategory,
}

/// Visible application lifecycle state (Master Prompt Section 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Disabled,
    #[default]
    Idle,
    Listening,
    Processing,
    Correcting,
    ReadyToPaste,
    Error(String),
}

impl EngineState {
    pub fn status_label(&self) -> &str {
        match self {
            EngineState::Disabled => "Disabled",
            EngineState::Idle => "Idle (Ready)",
            EngineState::Listening => "Listening...",
            EngineState::Processing => "Transcribing...",
            EngineState::Correcting => "Correcting Grammar...",
            EngineState::ReadyToPaste => "Ready to Paste (Ctrl+V)",
            EngineState::Error(_) => "Error",
        }
    }
}

/// Global shortcut specification using Windows Virtual-Key (VK) codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeySpec {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Primary Win32 virtual-key code (e.g., 0x20 = VK_SPACE, 0xA3 = VK_RCONTROL).
    /// May be a modifier key itself for single-key modifier hold-to-talk (e.g. Right Ctrl).
    pub vk: u16,
    pub display_name: String,
}

impl Default for HotkeySpec {
    fn default() -> Self {
        // Default: Ctrl + Win (or Right Ctrl): ergonomics without conflicting with Ctrl+Space IDE autocomplete.
        Self {
            ctrl: true,
            alt: false,
            shift: false,
            win: true,
            vk: 0, // Modifier-chord hold: Ctrl + Win
            display_name: "Ctrl + Win".to_string(),
        }
    }
}

impl HotkeySpec {
    pub fn right_ctrl() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk: 0xA3, // VK_RCONTROL
            display_name: "Right Ctrl".to_string(),
        }
    }

    pub fn ctrl_shift_space() -> Self {
        Self {
            ctrl: true,
            alt: false,
            shift: true,
            win: false,
            vk: 0x20, // VK_SPACE
            display_name: "Ctrl + Shift + Space".to_string(),
        }
    }

    pub fn cancel_escape() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk: 0x1B, // VK_ESCAPE
            display_name: "Esc".to_string(),
        }
    }

    /// Checks whether this shortcut conflicts with reserved OS chords or another configured shortcut.
    pub fn check_conflict(&self, other: Option<&HotkeySpec>) -> Option<String> {
        if let Some(o) = other {
            if self.ctrl == o.ctrl
                && self.alt == o.alt
                && self.shift == o.shift
                && self.win == o.win
                && self.vk == o.vk
            {
                return Some(format!(
                    "Shortcut '{}' conflicts with another configured KeyReed shortcut.",
                    self.display_name
                ));
            }
        }
        // Reserved Windows system / standard editing chords that must not be hijacked
        if self.ctrl && self.alt && self.vk == 0x2E {
            return Some("Ctrl + Alt + Delete is reserved by the Windows security desktop.".to_string());
        }
        if self.win && !self.ctrl && !self.alt && !self.shift && self.vk == 0x4C {
            return Some("Win + L is reserved by Windows Lock Screen.".to_string());
        }
        if self.alt && !self.ctrl && !self.shift && !self.win && (self.vk == 0x09 || self.vk == 0x73) {
            return Some("Alt + Tab and Alt + F4 are reserved Windows window-management shortcuts.".to_string());
        }
        if self.ctrl && !self.alt && !self.shift && !self.win && matches!(self.vk, 0x43 | 0x56 | 0x58 | 0x5A) {
            return Some("Ctrl + C, V, X and Z are the editing shortcuts every app needs. Pick another chord.".to_string());
        }
        None
    }
}

/// Measured stage-by-stage latency and resource telemetry for a single dictation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineMetrics {
    pub audio_duration_ms: u64,
    pub vad_ms: f64,
    pub asr_ms: f64,
    pub rewrite_ms: f64,
    pub output_ms: f64,
    pub total_release_to_ready_ms: f64,
    pub used_tier: String,
    pub asr_model_id: String,
    #[serde(default)]
    pub mic_init_ms: f64,
    #[serde(default)]
    pub first_frame_ms: f64,
    #[serde(default)]
    pub chunk_finalize_ms: f64,
    #[serde(default)]
    pub model_load_ms: f64,
    #[serde(default)]
    pub post_release_asr_ms: f64,
    #[serde(default)]
    pub chunks_total: usize,
    #[serde(default)]
    pub chunks_streamed: usize,
    #[serde(default)]
    pub norm_ms: f64,
    #[serde(default)]
    pub tier0_ms: f64,
    #[serde(default)]
    pub slm_startup_ms: f64,
    #[serde(default)]
    pub slm_first_token_ms: f64,
    #[serde(default)]
    pub slm_ms: f64,
    #[serde(default)]
    pub paragraph_ms: f64,
    #[serde(default)]
    pub target_id_ms: f64,
    #[serde(default)]
    pub end_to_end_ms: f64,
    #[serde(default)]
    pub cpu_ms: f64,
    #[serde(default)]
    pub working_set_mb: f64,
    #[serde(default)]
    pub cold_model: bool,
}

/// Complete output of one transcription + correction cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub raw_transcript: String,
    pub final_text: String,
    pub effective_mode: WritingMode,
    pub delivered_via: OutputMode,
    pub fallback_to_clipboard: bool,
    pub requires_command_confirmation: bool,
    pub warning_note: Option<String>,
    /// Machine-readable reason when text was not typed directly (e.g. `password_field_protected`,
    /// `user_cancelled_insert`). `None` when typed directly or in Clipboard-only mode.
    #[serde(default)]
    pub delivery_reason: Option<String>,
    pub metrics: PipelineMetrics,
}
