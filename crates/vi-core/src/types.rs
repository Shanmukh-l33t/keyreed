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
}

/// Output delivery mode. Clipboard stays the stored default until the user turns on Auto Insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Copies the finished text and waits for Ctrl+V.
    #[default]
    Clipboard,
    /// Types into the focused field. Falls back to the clipboard if that is unsafe.
    DirectTyping,
    /// Copies the text and asks the user to paste. Does not type into a window.
    AskBeforeInserting,
}

impl OutputMode {
    pub fn label(self) -> &'static str {
        match self {
            OutputMode::Clipboard => "Clipboard only (Ctrl+V)",
            OutputMode::DirectTyping => "Auto Insert (recommended)",
            OutputMode::AskBeforeInserting => "Ask before inserting",
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
    pub metrics: PipelineMetrics,
}
