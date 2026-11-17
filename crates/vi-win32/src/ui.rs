//! KeyReed desktop shell: system tray, compact floating listener, and the bridge to the Settings window.
//!
//! Architecture: pure Win32 + a tiny software renderer (no WebView, no GPU runtime).
//! - The floating listener is a `WS_EX_LAYERED | WS_EX_NOACTIVATE` popup drawn by `crate::overlay`.
//! - Its animation timer runs only while something is actually moving; at idle the GUI thread sleeps.
//! - Settings (`crate::settings`) creates its controls on demand and frees them on close.

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::Arc;
use vi_core::{
    ActivationMode, AppConfig, EngineState, HotkeySpec, OutputMode, ProcessingProfile, WritingMode,
};

#[derive(Debug, Clone)]
pub enum UiAction {
    ToggleAppEnabled,
    TogglePttEnabled,
    FloatingMicClicked,
    SetActivationMode(ActivationMode),
    SetOutputMode(OutputMode),
    SetWritingMode(WritingMode),
    ToggleAutoContext,
    SetProcessingProfile(ProcessingProfile),
    ToggleFloatingMicVisible,
    SavePillPosition(i32, i32),
    SaveUpdatedConfig {
        new_cfg: Box<AppConfig>,
        new_plaintext_api_key: Option<String>,
    },
    ClearHistory,
    CopyLastDictation,
    DownloadDefaultModel,
    DownloadModel(String),
    CancelModelDownload,
    /// Switch the active speech model (only sent for installed models).
    SelectAsrModel(String),
    TestMicrophone(Option<String>),
    /// Re-enumerate input devices (sent when Settings opens).
    RefreshMicrophones,
    ResetToDefaults,
    SessionOrPowerReset,
    Quit,
}

/// Stage of a speech-model download, reported by the download worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadStage {
    Starting,
    Downloading,
    /// SHA-256 verification and extraction of the pinned files.
    Verifying,
    Installed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub model_id: String,
    pub stage: DownloadStage,
    pub done_bytes: u64,
    pub total_bytes: Option<u64>,
}

impl DownloadProgress {
    pub fn is_active(&self) -> bool {
        matches!(
            self.stage,
            DownloadStage::Starting | DownloadStage::Downloading | DownloadStage::Verifying
        )
    }

    /// Real fraction when the server reported a size; `None` means "size unknown" (no fake progress).
    pub fn fraction(&self) -> Option<f32> {
        match self.stage {
            DownloadStage::Downloading => self
                .total_bytes
                .filter(|t| *t > 0)
                .map(|t| (self.done_bytes as f64 / t as f64).clamp(0.0, 1.0) as f32),
            DownloadStage::Verifying | DownloadStage::Installed => Some(1.0),
            _ => None,
        }
    }

    pub fn detail_text(&self) -> String {
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        match &self.stage {
            DownloadStage::Starting => "Connecting…".to_string(),
            DownloadStage::Downloading => match self.total_bytes {
                Some(t) if t > 0 => format!(
                    "{:.0} of {:.0} MB · {}%",
                    mb(self.done_bytes),
                    mb(t),
                    ((self.done_bytes as f64 / t as f64) * 100.0).floor().min(100.0) as u32
                ),
                _ => format!("{:.0} MB downloaded", mb(self.done_bytes)),
            },
            DownloadStage::Verifying => "Verifying checksums and unpacking…".to_string(),
            DownloadStage::Installed => "Installed and verified".to_string(),
            DownloadStage::Cancelled => "Download cancelled. Nothing was changed.".to_string(),
            DownloadStage::Failed(e) => format!("Download failed: {e}"),
        }
    }
}

/// Plain-language result of a microphone test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicVerdict {
    /// The device opened but delivered no signal (muted, privacy-blocked, or wrong device).
    Silent,
    Low,
    Good,
    /// Loud enough that speech may distort.
    TooLoud,
}

impl MicVerdict {
    pub fn headline(self) -> &'static str {
        match self {
            MicVerdict::Silent => "No sound detected",
            MicVerdict::Low => "Very quiet",
            MicVerdict::Good => "Sounds good",
            MicVerdict::TooLoud => "Too loud",
        }
    }

    pub fn advice(self) -> &'static str {
        match self {
            MicVerdict::Silent => "Check the mute switch and Windows Settings › Privacy › Microphone, or pick another device.",
            MicVerdict::Low => "KeyReed can hear you. Move closer or raise the input volume for best accuracy.",
            MicVerdict::Good => "Your microphone is ready for dictation.",
            MicVerdict::TooLoud => "Lower the input volume in Windows Sound settings to avoid distortion.",
        }
    }
}

/// Classifies a test from the loudest RMS window that was observed.
pub fn classify_mic(peak_rms: f32) -> MicVerdict {
    if !peak_rms.is_finite() || peak_rms < 0.004 {
        MicVerdict::Silent
    } else if peak_rms < 0.035 {
        MicVerdict::Low
    } else if peak_rms < 0.5 {
        MicVerdict::Good
    } else {
        MicVerdict::TooLoud
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MicTestStatus {
    pub running: bool,
    /// Live level 0..1 (perceptual), updated ~20x/s while running.
    pub level: f32,
    pub peak_rms: f32,
    pub device: String,
    pub used_fallback: bool,
    pub verdict: Option<MicVerdict>,
    pub error: Option<String>,
}

/// What actually happened to the last dictation. Drives the listener's completion state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyOutcome {
    Inserted,
    /// Typed, but cleaned up by rules only because a text job held the language model.
    InsertedBasic,
    Copied,
    /// Auto Insert fell back to the clipboard (reason code from the typing layer).
    FallbackCopied(String),
    ReviewCommand,
    Discarded,
    NothingHeard,
}

impl ReadyOutcome {
    pub fn from_pipeline(
        delivered_via: OutputMode,
        fallback_to_clipboard: bool,
        requires_command_confirmation: bool,
        reason: Option<&str>,
    ) -> Self {
        match reason {
            Some("user_cancelled_insert") => return ReadyOutcome::Discarded,
            Some("dangerous_command_requires_manual_paste") => return ReadyOutcome::ReviewCommand,
            _ => {}
        }
        if requires_command_confirmation && delivered_via != OutputMode::DirectTyping {
            return ReadyOutcome::ReviewCommand;
        }
        match (delivered_via, fallback_to_clipboard) {
            (OutputMode::DirectTyping, _) => ReadyOutcome::Inserted,
            (_, true) => ReadyOutcome::FallbackCopied(reason.unwrap_or("unknown").to_string()),
            _ => ReadyOutcome::Copied,
        }
    }

    /// Short label for the floating listener (must stay compact).
    pub fn pill_label(&self) -> &'static str {
        match self {
            ReadyOutcome::Inserted => "Inserted",
            ReadyOutcome::InsertedBasic => "Basic cleanup",
            ReadyOutcome::Copied | ReadyOutcome::FallbackCopied(_) => "Copied · Ctrl+V",
            ReadyOutcome::ReviewCommand => "Review · Ctrl+V",
            ReadyOutcome::Discarded => "Discarded",
            ReadyOutcome::NothingHeard => "No speech heard",
        }
    }

    /// Success = text reached the user (typed or on the clipboard).
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            ReadyOutcome::Inserted
                | ReadyOutcome::InsertedBasic
                | ReadyOutcome::Copied
                | ReadyOutcome::FallbackCopied(_)
                | ReadyOutcome::ReviewCommand
        )
    }
}

/// One-sentence explanation of why text was copied instead of typed.
pub fn delivery_reason_sentence(reason: &str) -> &'static str {
    match reason {
        "password_field_protected" => "This looks like a password field, so KeyReed didn't type into it. The text is on your clipboard.",
        "target_window_elevated_uipi" => "That window runs as administrator, so Windows blocked typing. Press Ctrl+V to paste.",
        "foreground_window_changed_during_processing" | "foreground_window_changed_during_typing" => {
            "You switched windows before KeyReed finished, so it copied the text instead. Press Ctrl+V to paste."
        }
        "target_window_closed" => "The window you dictated into was closed. The text is on your clipboard.",
        "unsupported_target_window" | "sendinput_blocked_by_target" => {
            "KeyReed couldn't type into that window. Press Ctrl+V to paste."
        }
        "dangerous_command_requires_manual_paste" => {
            "This looks like a command that could change your system. Review it, then press Ctrl+V."
        }
        "user_chose_clipboard_only" => "Copied to your clipboard. Press Ctrl+V to paste.",
        "user_cancelled_insert" => "Discarded. Nothing was inserted or copied.",
        _ => "The text is on your clipboard. Press Ctrl+V to paste.",
    }
}

/// Plain-language error for the UI. Technical details stay in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendlyError {
    /// Fits the floating listener.
    pub short: &'static str,
    pub message: &'static str,
}

pub fn friendly_error(raw: &str) -> FriendlyError {
    let l = raw.to_ascii_lowercase();
    let has = |k: &str| l.contains(k);
    if has("mic") || has("microphone") || has("audio endpoint") || has("wasapi") || has("capture device") {
        FriendlyError {
            short: "Mic unavailable",
            message: "KeyReed couldn't open your microphone. Check that it's connected and that Windows allows microphone access. Nothing was recorded.",
        }
    } else if has("not installed")
        || has("not in catalog")
        || has("onnx")
        || (has("model") && (has("missing") || has("load") || has("not found") || has("no such")))
    {
        FriendlyError {
            short: "Model missing",
            message: "The speech model isn't installed or couldn't start. Open Settings › Speech models to download or switch it. This recording was discarded.",
        }
    } else if has("clipboard") {
        FriendlyError {
            short: "Couldn't copy",
            message: "Windows didn't let KeyReed use the clipboard. Try again in a moment.",
        }
    } else {
        FriendlyError {
            short: "Couldn't transcribe",
            message: "Something went wrong while transcribing, and this recording was discarded. Try again. Details are in the KeyReed log.",
        }
    }
}

/// Plain-language catalog presentation. Speeds are measured, see docs/release/15-v0.4.0-design-and-ux.md.
#[derive(Debug, Clone, Copy)]
pub struct AsrUiInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub languages: &'static str,
    pub download_mb: u32,
    /// Median warm decode time for 10 s of speech on the KeyReed test laptop (CPU only), in seconds.
    /// Source: bench/model_speed_v0.4.0.json (9 warm runs per model over 3 clips).
    pub measured_s_per_10s: Option<f32>,
    pub recommended: bool,
}

pub const ASR_UI_CATALOG: [AsrUiInfo; 6] = [
    AsrUiInfo {
        id: "parakeet-tdt-0.6b-v2-int8",
        name: "Parakeet v2",
        summary: "Most accurate for English dictation.",
        languages: "English",
        download_mb: 460,
        measured_s_per_10s: Some(1.5),
        recommended: true,
    },
    AsrUiInfo {
        id: "parakeet-tdt-0.6b-v3-int8",
        name: "Parakeet v3 Multilingual",
        summary: "English plus 24 European languages.",
        languages: "25 languages",
        download_mb: 465,
        measured_s_per_10s: Some(1.7),
        recommended: false,
    },
    AsrUiInfo {
        id: "moonshine-base-en-int8",
        name: "Moonshine Base",
        summary: "Smaller download for lower-end PCs.",
        languages: "English",
        download_mb: 239,
        measured_s_per_10s: Some(0.7),
        recommended: false,
    },
    AsrUiInfo {
        id: "moonshine-tiny-en-int8",
        name: "Moonshine Tiny",
        summary: "Smallest download. Lower accuracy.",
        languages: "English",
        download_mb: 103,
        measured_s_per_10s: Some(0.5),
        recommended: false,
    },
    AsrUiInfo {
        id: "sense-voice-small-int8",
        name: "SenseVoice Small",
        summary: "Chinese, Cantonese, English, Japanese, Korean.",
        languages: "5 languages",
        download_mb: 999,
        measured_s_per_10s: Some(0.7),
        recommended: false,
    },
    AsrUiInfo {
        id: "whisper-small.en-int8",
        name: "Whisper Small",
        summary: "OpenAI Whisper, English only. Slowest on CPU.",
        languages: "English",
        download_mb: 606,
        measured_s_per_10s: Some(7.9),
        recommended: false,
    },
];

pub fn asr_ui_info(id: &str) -> Option<&'static AsrUiInfo> {
    ASR_UI_CATALOG.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// Push-to-talk presets offered in Settings, each with an honest compatibility note.
pub fn hotkey_presets() -> Vec<(HotkeySpec, &'static str)> {
    vec![
        (HotkeySpec::right_ctrl(), "Recommended. Rarely used on its own by other apps."),
        (HotkeySpec::default(), "Two-key combination; safe in most apps."),
        (
            HotkeySpec {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
                vk: 0xA5, // VK_RMENU
                display_name: "Right Alt".to_string(),
            },
            "Acts as AltGr on many non-US keyboards.",
        ),
        (
            HotkeySpec {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
                vk: 0x77, // VK_F8
                display_name: "F8".to_string(),
            },
            "Some apps (debuggers, games) also use F8.",
        ),
        (
            HotkeySpec {
                ctrl: false,
                alt: true,
                shift: false,
                win: false,
                vk: 0x20,
                display_name: "Alt + Space".to_string(),
            },
            "Normally opens the window menu in most apps.",
        ),
        (HotkeySpec::ctrl_shift_space(), "Also the shortcut that cycles writing modes."),
    ]
}

/// Real conflicts with KeyReed's own shortcuts.
pub fn hotkey_conflict(cfg: &AppConfig, candidate: &HotkeySpec) -> Option<String> {
    let same = |a: &HotkeySpec, b: &HotkeySpec| {
        a.vk == b.vk && a.ctrl == b.ctrl && a.alt == b.alt && a.shift == b.shift && a.win == b.win
    };
    if same(candidate, &cfg.mode_cycle_hotkey) {
        Some(format!(
            "{} is also the mode-cycle shortcut. Pick another one to avoid switching modes.",
            candidate.display_name
        ))
    } else if same(candidate, &cfg.cancel_hotkey) {
        Some(format!("{} is the cancel shortcut.", candidate.display_name))
    } else {
        None
    }
}

#[derive(Clone)]
pub struct SharedUiModel {
    pub config: AppConfig,
    pub state: EngineState,
    pub rms_level: f32,
    /// Legacy one-line status (kept for the CLI/E2E report and tray tooltip).
    pub status_detail: String,
    pub available_microphones: Vec<String>,
    pub installed_asr_models: Vec<String>,
    pub download_in_progress: bool,
    pub download: Option<DownloadProgress>,
    pub mic_test: MicTestStatus,
    pub last_outcome: Option<ReadyOutcome>,
    /// Brief, quiet message on the listener (e.g. after switching writing mode by shortcut).
    pub hint: Option<(String, String, std::time::Instant)>,
    /// Overrides the processing label (e.g. "Loading model" on a cold start).
    pub processing_note: Option<String>,
}

/// How long a listener hint stays visible.
pub const HINT_MS: u128 = 1600;

impl SharedUiModel {
    pub fn is_installed(&self, id: &str) -> bool {
        self.installed_asr_models.iter().any(|m| m.eq_ignore_ascii_case(id))
    }

    pub fn active_model_ready(&self) -> bool {
        self.is_installed(&self.config.local_asr_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_ask_before_inserting_is_discarded_not_copied() {
        let o = ReadyOutcome::from_pipeline(OutputMode::Clipboard, true, false, Some("user_cancelled_insert"));
        assert_eq!(o, ReadyOutcome::Discarded);
        assert!(!o.is_success());
        assert_eq!(o.pill_label(), "Discarded");
    }

    #[test]
    fn outcomes_map_from_real_delivery_results() {
        assert_eq!(ReadyOutcome::from_pipeline(OutputMode::DirectTyping, false, false, None), ReadyOutcome::Inserted);
        assert_eq!(ReadyOutcome::from_pipeline(OutputMode::Clipboard, false, false, None), ReadyOutcome::Copied);
        assert_eq!(
            ReadyOutcome::from_pipeline(OutputMode::Clipboard, true, false, Some("password_field_protected")),
            ReadyOutcome::FallbackCopied("password_field_protected".into())
        );
        assert_eq!(
            ReadyOutcome::from_pipeline(OutputMode::Clipboard, true, true, Some("dangerous_command_requires_manual_paste")),
            ReadyOutcome::ReviewCommand
        );
        assert_eq!(
            ReadyOutcome::from_pipeline(OutputMode::Clipboard, false, false, Some("user_chose_clipboard_only")),
            ReadyOutcome::Copied
        );
    }

    #[test]
    fn pill_labels_stay_compact() {
        for o in [
            ReadyOutcome::Inserted,
            ReadyOutcome::InsertedBasic,
            ReadyOutcome::Copied,
            ReadyOutcome::ReviewCommand,
            ReadyOutcome::Discarded,
            ReadyOutcome::NothingHeard,
        ] {
            assert!(o.pill_label().chars().count() <= 16, "{o:?}");
        }
        for raw in ["Mic error: x", "ASR model 'x' not in catalog", "Transcription failed: boom", "clipboard busy"] {
            assert!(friendly_error(raw).short.chars().count() <= 20);
        }
    }

    #[test]
    fn friendly_errors_hide_internal_text() {
        let e = friendly_error("Mic error: WASAPI IAudioClient::Initialize failed 0x88890008");
        assert_eq!(e.short, "Mic unavailable");
        assert!(!e.message.contains("0x"));
        assert_eq!(friendly_error("ASR model 'foo' not in catalog").short, "Model missing");
        assert_eq!(friendly_error("Transcription failed: decoder panic").short, "Couldn't transcribe");
    }

    #[test]
    fn mic_verdicts_cover_silence_low_good_and_loud() {
        assert_eq!(classify_mic(0.0), MicVerdict::Silent);
        assert_eq!(classify_mic(0.01), MicVerdict::Low);
        assert_eq!(classify_mic(0.12), MicVerdict::Good);
        assert_eq!(classify_mic(0.8), MicVerdict::TooLoud);
        assert_eq!(classify_mic(f32::NAN), MicVerdict::Silent);
    }

    #[test]
    fn download_progress_never_fakes_a_percentage() {
        let mut p = DownloadProgress {
            model_id: "m".into(),
            stage: DownloadStage::Downloading,
            done_bytes: 50 * 1024 * 1024,
            total_bytes: None,
        };
        assert_eq!(p.fraction(), None);
        assert_eq!(p.detail_text(), "50 MB downloaded");
        p.total_bytes = Some(200 * 1024 * 1024);
        assert_eq!(p.fraction(), Some(0.25));
        assert_eq!(p.detail_text(), "50 of 200 MB · 25%");
    }

    #[test]
    fn asr_catalog_ui_has_six_models_and_one_recommendation() {
        assert_eq!(ASR_UI_CATALOG.len(), 6);
        assert_eq!(ASR_UI_CATALOG.iter().filter(|m| m.recommended).count(), 1);
        assert!(asr_ui_info("PARAKEET-TDT-0.6B-V2-INT8").is_some());
    }

    #[test]
    fn detects_conflict_with_mode_cycle_shortcut() {
        let cfg = AppConfig::default();
        assert!(hotkey_conflict(&cfg, &HotkeySpec::ctrl_shift_space()).is_some());
        assert!(hotkey_conflict(&cfg, &HotkeySpec::right_ctrl()).is_none());
    }
}

#[cfg(windows)]
pub mod win_gui {
    use super::*;
    use crate::overlay::{
        pill_window_width, render_pill, rms_to_level, scaled_icon, PillFrame, PillKind, PillMetrics, TextMask,
    };
    use once_cell::sync::OnceCell;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::time::Instant;
    use vi_core::brand::{OVERLAY_WINDOW_CLASS, PRODUCT_NAME, SETTINGS_WINDOW_CLASS, TRAY_WINDOW_CLASS};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
        GetTextExtentPoint32W, GetTextFaceW, MonitorFromPoint, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
        TextOutW, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
        HBITMAP, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForMonitor, GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        ReleaseCapture, SetCapture, TrackMouseEvent, TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT,
    };
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_NOSOUND, NIIF_WARNING, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub const WM_TRAY_CALLBACK: u32 = WM_USER + 40;
    pub const WM_SYNC_UI: u32 = WM_USER + 41;
    pub const WM_SHOW_TOAST: u32 = WM_USER + 42;
    pub const WM_ALREADY_RUNNING: u32 = vi_core::brand::ALREADY_RUNNING_MSG;
    const WM_MOUSEHOVER_U: u32 = 0x02A1;
    const WM_MOUSELEAVE_U: u32 = 0x02A3;

    // Tray menu ids (IDM_EXIT and IDM_OPEN_SETTINGS are used by automation; keep stable).
    const IDM_APP_ENABLE: usize = 1001;
    const IDM_OUT_CLIP: usize = 1005;
    const IDM_OUT_TYPE: usize = 1006;
    const IDM_AUTO_CTX: usize = 1007;
    const IDM_SHOW_PILL: usize = 1008;
    pub const IDM_OPEN_SETTINGS: usize = 1009;
    pub const IDM_EXIT: usize = 1011;
    const IDM_OUT_ASK: usize = 1012;
    const IDM_COPY_LAST: usize = 1014;
    const IDM_SETUP_MODEL: usize = 1015;
    const IDM_MODE_BASE: usize = 1100;

    const IDT_PILL_ANIM: usize = 7002;
    const IDT_HOVER_COLLAPSE: usize = 7003;
    /// Hover must rest this long before the listener expands (avoids accidental expansion).
    const HOVER_INTENT_MS: u32 = 380;

    pub(crate) struct WinGuiGlobals {
        pub(crate) tx: Sender<UiAction>,
        pub(crate) model: Arc<Mutex<SharedUiModel>>,
        pub(crate) tray_hwnd: Mutex<isize>,
        pub(crate) pill_hwnd: Mutex<isize>,
        pub(crate) toast_buf: Mutex<Option<(String, String, bool)>>,
    }

    static GUI_GLOBALS: OnceCell<WinGuiGlobals> = OnceCell::new();

    pub(crate) fn globals() -> Option<&'static WinGuiGlobals> {
        GUI_GLOBALS.get()
    }

    pub(crate) fn send(action: UiAction) {
        if let Some(g) = GUI_GLOBALS.get() {
            let _ = g.tx.send(action);
        }
    }

    pub(crate) fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// True for KeyReed's own practice field (Settings › Get started), which may receive dictation.
    pub fn is_practice_target(hwnd: isize) -> bool {
        if hwnd == 0 {
            return false;
        }
        unsafe {
            let mut buf = [0u16; 64];
            let n = GetClassNameW(hwnd as HWND, buf.as_mut_ptr(), buf.len() as i32);
            n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == SETTINGS_WINDOW_CLASS
        }
    }

    pub(crate) fn reduced_motion() -> bool {
        unsafe {
            let mut on: i32 = 1;
            let ok = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on as *mut i32 as *mut _, 0);
            ok != 0 && on == 0
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Text rasterisation (grayscale coverage masks for the layered listener)
    // ---------------------------------------------------------------------------------------------

    pub(crate) fn ui_font_face() -> &'static str {
        static FACE: OnceCell<&'static str> = OnceCell::new();
        FACE.get_or_init(|| unsafe {
            // Segoe UI Variable ships with Windows 11; Windows 10 falls back to Segoe UI.
            let want = "Segoe UI Variable Text";
            let name = to_wide(want);
            let f = CreateFontW(-14, 0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 0, 0, name.as_ptr());
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let old = SelectObject(dc, f as _);
            let mut buf = [0u16; 64];
            let n = GetTextFaceW(dc, buf.len() as i32, buf.as_mut_ptr());
            SelectObject(dc, old);
            DeleteObject(f as _);
            DeleteDC(dc);
            let end = buf.iter().position(|c| *c == 0).unwrap_or(n.max(0) as usize);
            let got = String::from_utf16_lossy(&buf[..end]);
            if got.eq_ignore_ascii_case(want) {
                "Segoe UI Variable Text"
            } else {
                "Segoe UI"
            }
        })
    }

    struct MaskEntry {
        w: usize,
        h: usize,
        cov: Vec<u8>,
    }

    thread_local! {
        static MASK_CACHE: RefCell<HashMap<(String, i32, i32), MaskEntry>> = RefCell::new(HashMap::new());
    }

    unsafe fn rasterize_text(text: &str, px: i32, weight: i32) -> MaskEntry {
        let face = to_wide(ui_font_face());
        // ANTIALIASED_QUALITY (4): grayscale AA so coverage maps cleanly onto alpha.
        let font = CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, 0, 0, 0, 4, 0, face.as_ptr());
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let oldf = SelectObject(dc, font as _);
        let w16: Vec<u16> = text.encode_utf16().collect();
        let mut sz = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(dc, w16.as_ptr(), w16.len() as i32, &mut sz);
        let w = (sz.cx + 2).max(1) as usize;
        let h = sz.cy.max(1) as usize;
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w as i32;
        bmi.bmiHeader.biHeight = -(h as i32);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let mut cov = vec![0u8; w * h];
        if !hbm.is_null() && !bits.is_null() {
            let oldb = SelectObject(dc, hbm as _);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00FFFFFF);
            TextOutW(dc, 1, 0, w16.as_ptr(), w16.len() as i32);
            let src = std::slice::from_raw_parts(bits as *const u32, w * h);
            for (c, p) in cov.iter_mut().zip(src.iter()) {
                *c = ((p >> 8) & 0xFF) as u8;
            }
            SelectObject(dc, oldb);
            DeleteObject(hbm as _);
        }
        SelectObject(dc, oldf);
        DeleteObject(font as _);
        DeleteDC(dc);
        MaskEntry { w, h, cov }
    }

    fn with_mask<R>(text: &str, px: i32, weight: i32, f: impl FnOnce(&MaskEntry) -> R) -> R {
        MASK_CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 48 {
                c.clear();
            }
            let key = (text.to_string(), px, weight);
            let e = c.entry(key).or_insert_with(|| unsafe { rasterize_text(text, px, weight) });
            f(e)
        })
    }

    // ---------------------------------------------------------------------------------------------
    // Icons
    // ---------------------------------------------------------------------------------------------

    unsafe fn create_state_hicon(state: &EngineState, size: usize, needs_setup: bool) -> HICON {
        let n = size.clamp(16, 64);
        let mut px = scaled_icon(n);
        let badge = match state {
            EngineState::Disabled => Some((0x8Au8, 0x94u8, 0x90u8)),
            EngineState::Listening => Some((0x10, 0xB9, 0x81)),
            EngineState::Processing | EngineState::Correcting => Some((0xF5, 0x9E, 0x0B)),
            EngineState::Error(_) => Some((0xE1, 0x1D, 0x48)),
            _ if needs_setup => Some((0xF5, 0x9E, 0x0B)),
            _ => None,
        };
        if let Some((r, g, b)) = badge {
            let rad = n as f32 * 0.19;
            let c = n as f32 - rad - 0.5;
            for y in 0..n {
                for x in 0..n {
                    let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
                    let ring = (rad + 1.3 - d + 0.5).clamp(0.0, 1.0);
                    let dot = (rad - d + 0.5).clamp(0.0, 1.0);
                    if ring <= 0.0 {
                        continue;
                    }
                    // White ring then coloured dot, composited over the mark (premultiplied).
                    let mut p = px[y * n + x];
                    for (cov, (cr, cg, cb)) in [(ring, (255u8, 255u8, 255u8)), (dot, (r, g, b))] {
                        let inv = 1.0 - cov;
                        let cur = p;
                        let ch = |sh: u32, v: u8| ((v as f32 * cov) + ((cur >> sh) & 0xFF) as f32 * inv) as u32;
                        p = (ch(24, 255) << 24) | (ch(16, cr) << 16) | (ch(8, cg) << 8) | ch(0, cb);
                    }
                    px[y * n + x] = p;
                }
            }
        }
        // Icons take straight (non-premultiplied) alpha.
        let straight: Vec<u32> = px
            .iter()
            .map(|&p| {
                let a = (p >> 24) & 0xFF;
                if a == 0 {
                    0
                } else {
                    let un = |sh: u32| ((((p >> sh) & 0xFF) * 255 + a / 2) / a).min(255);
                    (a << 24) | (un(16) << 16) | (un(8) << 8) | un(0)
                }
            })
            .collect();
        let mask = vec![0u8; n.div_ceil(16) * 2 * n];
        let hbm_color =
            windows_sys::Win32::Graphics::Gdi::CreateBitmap(n as i32, n as i32, 1, 32, straight.as_ptr() as *const _);
        let hbm_mask =
            windows_sys::Win32::Graphics::Gdi::CreateBitmap(n as i32, n as i32, 1, 1, mask.as_ptr() as *const _);
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: hbm_mask, hbmColor: hbm_color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(hbm_color as _);
        DeleteObject(hbm_mask as _);
        icon
    }

    /// Window icons for Settings (title bar + Alt-Tab), rendered from the brand mark.
    pub(crate) unsafe fn brand_hicon(size: usize) -> HICON {
        create_state_hicon(&EngineState::Idle, size, false)
    }

    unsafe fn update_tray_icon(hwnd: HWND, action: u32, state: &EngineState, needs_setup: bool, tip_text: &str) {
        let dpi = GetDpiForSystem().max(96);
        let sz = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16) as usize;
        let hicon = create_state_hicon(state, sz, needs_setup);
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;
        nid.hIcon = hicon;
        let tip_w = to_wide(tip_text);
        let copy_len = tip_w.len().min(nid.szTip.len() - 1);
        nid.szTip[..copy_len].copy_from_slice(&tip_w[..copy_len]);
        let _ = Shell_NotifyIconW(action, &nid);
        if !hicon.is_null() {
            let _ = DestroyIcon(hicon);
        }
    }

    unsafe fn show_tray_balloon(hwnd: HWND, title: &str, body: &str, warning: bool) {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = (if warning { NIIF_WARNING } else { NIIF_INFO }) | NIIF_NOSOUND;
        let tw = to_wide(title);
        let tlen = tw.len().min(nid.szInfoTitle.len() - 1);
        nid.szInfoTitle[..tlen].copy_from_slice(&tw[..tlen]);
        let bw = to_wide(body);
        let blen = bw.len().min(nid.szInfo.len() - 1);
        nid.szInfo[..blen].copy_from_slice(&bw[..blen]);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }

    // ---------------------------------------------------------------------------------------------
    // Floating listener
    // ---------------------------------------------------------------------------------------------

    /// GUI-thread state of the listener (never touched by other threads).
    struct Pill {
        /// Physical centre-x and top of the listener; expansion grows symmetrically around `cx`.
        cx: i32,
        top: i32,
        /// DPI scale the geometry above was last laid out for.
        last_scale: f32,
        cur_w: f32,
        from_w: f32,
        to_w: f32,
        tween_start: Instant,
        content_since: Instant,
        state_since: Instant,
        last_kind: PillKind,
        last_label: String,
        levels: VecDeque<f32>,
        tick: u32,
        hovered: bool,
        hover_tracking: bool,
        drag: Option<(i32, i32, bool)>,
        timer_on: bool,
        visible: bool,
        frames_painted: u64,
    }

    thread_local! {
        static PILL: RefCell<Option<Pill>> = const { RefCell::new(None) };
    }

    const TWEEN_MS: f32 = 160.0;
    const FADE_MS: f32 = 140.0;

    fn ease_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    static DPI_OVERRIDE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// Test hook (`--ui-gallery --scale N`): render as if the monitor were at `dpi`. 0 = off.
    pub fn set_dpi_override(dpi: u32) {
        DPI_OVERRIDE.store(dpi, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn dpi_override() -> u32 {
        DPI_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed)
    }

    unsafe fn dpi_scale(hwnd: HWND) -> f32 {
        let o = dpi_override();
        if o != 0 {
            return o as f32 / 96.0;
        }
        let d = GetDpiForWindow(hwnd);
        (if d == 0 { 96 } else { d }) as f32 / 96.0
    }

    unsafe fn work_area_at(x: i32, y: i32) -> RECT {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) != 0 {
            mi.rcWork
        } else {
            RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
        }
    }

    /// Default home: bottom centre of the primary work area, just above the taskbar.
    unsafe fn default_anchor(scale: f32) -> (i32, i32) {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let wa = if GetMonitorInfoW(mon, &mut mi) != 0 {
            mi.rcWork
        } else {
            RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
        };
        let h = PillMetrics::for_scale(scale).window_height() as i32;
        ((wa.left + wa.right) / 2, wa.bottom - h - (14.0 * scale).round() as i32)
    }

    /// What the listener should show for the current model.
    struct PillView {
        kind: PillKind,
        label: String,
        secondary: String,
    }

    fn pill_view(snap: &SharedUiModel, hovered: bool) -> PillView {
        let none = |kind| PillView { kind, label: String::new(), secondary: String::new() };
        match &snap.state {
            EngineState::Disabled => {
                if hovered {
                    PillView { kind: PillKind::IdleHover, label: "Paused".into(), secondary: "Right-click".into() }
                } else {
                    none(PillKind::Disabled)
                }
            }
            EngineState::Idle => {
                if let Some((label, sub, at)) = snap.hint.as_ref().filter(|h| h.2.elapsed().as_millis() < HINT_MS) {
                    let _ = at;
                    PillView { kind: PillKind::IdleHover, label: label.clone(), secondary: sub.clone() }
                } else if !hovered {
                    none(PillKind::Idle)
                } else if !snap.active_model_ready() {
                    PillView { kind: PillKind::IdleHover, label: "Set up speech".into(), secondary: "Click".into() }
                } else {
                    PillView {
                        kind: PillKind::IdleHover,
                        label: snap.config.default_writing_mode.short_label().to_string(),
                        secondary: snap.config.ptt_hotkey.display_name.clone(),
                    }
                }
            }
            EngineState::Listening => none(PillKind::Listening),
            EngineState::Processing => PillView {
                kind: PillKind::Processing,
                label: snap.processing_note.clone().unwrap_or_else(|| "Transcribing".into()),
                secondary: String::new(),
            },
            EngineState::Correcting => {
                PillView { kind: PillKind::Processing, label: "Polishing".into(), secondary: String::new() }
            }
            EngineState::ReadyToPaste => {
                let o = snap.last_outcome.clone().unwrap_or(ReadyOutcome::Copied);
                let kind = match o {
                    ReadyOutcome::ReviewCommand => PillKind::Attention,
                    _ if o.is_success() => PillKind::Done,
                    _ => PillKind::Notice,
                };
                PillView { kind, label: o.pill_label().to_string(), secondary: String::new() }
            }
            EngineState::Error(msg) => {
                PillView { kind: PillKind::Error, label: friendly_error(msg).short.to_string(), secondary: String::new() }
            }
        }
    }

    fn label_px(scale: f32) -> i32 {
        (12.5 * scale).round() as i32
    }

    fn needs_frames(kind: PillKind) -> bool {
        matches!(kind, PillKind::Listening | PillKind::Processing)
    }

    /// Recomputes the target, (re)starts or stops the animation timer and paints one frame.
    ///
    /// Re-entrancy safe: moving the layered window onto a monitor with a different DPI makes Windows
    /// send WM_DPICHANGED synchronously from inside SetWindowPos/UpdateLayeredWindow. A nested call is
    /// recorded and replayed once the outer refresh has released its borrow (this used to panic with
    /// "RefCell already borrowed" and quit KeyReed when the listener was dragged across monitors).
    unsafe fn pill_refresh(hwnd: HWND, from_timer: bool) {
        thread_local! {
            static IN_REFRESH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
            static PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        if IN_REFRESH.with(|f| f.replace(true)) {
            PENDING.with(|f| f.set(true));
            return;
        }
        pill_refresh_inner(hwnd, from_timer);
        for _ in 0..4 {
            if !PENDING.with(|f| f.replace(false)) {
                break;
            }
            pill_refresh_inner(hwnd, false);
        }
        IN_REFRESH.with(|f| f.set(false));
    }

    unsafe fn pill_refresh_inner(hwnd: HWND, from_timer: bool) {
        let Some(g) = GUI_GLOBALS.get() else { return };
        let snap = g.model.lock().clone();
        let should_show = snap.config.show_floating_mic;
        PILL.with(|p| {
            let mut p = p.borrow_mut();
            let Some(pill) = p.as_mut() else { return };
            if !should_show {
                if pill.visible {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    pill.visible = false;
                }
                if pill.timer_on {
                    KillTimer(hwnd, IDT_PILL_ANIM);
                    pill.timer_on = false;
                }
                return;
            }
            let scale = dpi_scale(hwnd);
            if (scale - pill.last_scale).abs() > 0.01 {
                // Moved onto a monitor with another DPI: keep the same vertical centre.
                let old_h = PillMetrics::for_scale(pill.last_scale).window_height() as i32;
                let new_h = PillMetrics::for_scale(scale).window_height() as i32;
                pill.top += (old_h - new_h) / 2;
                pill.last_scale = scale;
            }
            let reduced = reduced_motion();
            let view = pill_view(&snap, pill.hovered && pill.drag.is_none());
            let now = Instant::now();

            if view.kind != pill.last_kind || view.label != pill.last_label {
                if view.kind != pill.last_kind {
                    pill.state_since = now;
                    if view.kind == PillKind::Listening {
                        pill.levels.clear();
                    }
                }
                pill.content_since = now;
                pill.last_kind = view.kind;
                pill.last_label = view.label.clone();
            }
            if view.kind == PillKind::Listening && from_timer {
                pill.tick = pill.tick.wrapping_add(1);
                // ~15 bars/s: one new measured level every second 33 ms frame.
                if pill.tick % 2 == 0 {
                    pill.levels.push_back(rms_to_level(snap.rms_level));
                    while pill.levels.len() > crate::overlay::WAVE_BARS {
                        pill.levels.pop_front();
                    }
                }
            }

            let px = label_px(scale);
            let weight = if matches!(view.kind, PillKind::IdleHover) { 600 } else { 500 };
            let (lw, lh, lcov) = if view.label.is_empty() {
                (0, 0, Vec::new())
            } else {
                with_mask(&view.label, px, weight, |m| (m.w, m.h, m.cov.clone()))
            };
            let (sw, sh, scov) = if view.secondary.is_empty() {
                (0, 0, Vec::new())
            } else {
                with_mask(&view.secondary, px, 400, |m| (m.w, m.h, m.cov.clone()))
            };
            let target_w = pill_window_width(scale, view.kind, lw, sw) as f32;
            if (target_w - pill.to_w).abs() > 0.5 {
                pill.from_w = pill.cur_w;
                pill.to_w = target_w;
                pill.tween_start = now;
            }
            let tween_t =
                if reduced { 1.0 } else { now.duration_since(pill.tween_start).as_secs_f32() * 1000.0 / TWEEN_MS };
            pill.cur_w = if tween_t >= 1.0 {
                pill.to_w
            } else {
                pill.from_w + (pill.to_w - pill.from_w) * ease_out_cubic(tween_t)
            };
            let fade_t =
                if reduced { 1.0 } else { now.duration_since(pill.content_since).as_secs_f32() * 1000.0 / FADE_MS };
            let tweening = tween_t < 1.0 || fade_t < 1.0;

            let w = pill.cur_w.round().max(8.0) as usize;
            let h = PillMetrics::for_scale(scale).window_height();
            let lbl = (lw > 0).then_some(TextMask { width: lw, height: lh, coverage: &lcov });
            let sec = (sw > 0).then_some(TextMask { width: sw, height: sh, coverage: &scov });
            let levels: Vec<f32> = pill.levels.iter().copied().collect();
            let phase = if reduced { 0.35 } else { now.duration_since(pill.state_since).as_secs_f32() };
            let frame = PillFrame {
                scale,
                kind: view.kind,
                levels: &levels,
                phase,
                label: lbl,
                secondary: sec,
                opacity: 1.0,
                content_opacity: ease_out_cubic(fade_t),
            };
            let bgra = render_pill(w, h, &frame);

            // Position: centred on the anchor, clamped into the monitor's work area.
            let wa = work_area_at(pill.cx, pill.top + h as i32 / 2);
            let x = (pill.cx - w as i32 / 2).clamp(wa.left, (wa.right - w as i32).max(wa.left));
            let y = pill.top.clamp(wa.top, (wa.bottom - h as i32).max(wa.top));
            present_layered(hwnd, &bgra, w as i32, h as i32, x, y);
            pill.frames_painted += 1;
            if !pill.visible {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                pill.visible = true;
            }

            if let Some((_, _, at)) = snap.hint.as_ref() {
                let left = HINT_MS.saturating_sub(at.elapsed().as_millis());
                if left > 0 {
                    SetTimer(hwnd, IDT_HOVER_COLLAPSE, (left as u32).max(30) + 20, None);
                }
            }
            let animate = tweening || (needs_frames(view.kind) && !(reduced && view.kind == PillKind::Processing));
            if animate && !pill.timer_on {
                SetTimer(hwnd, IDT_PILL_ANIM, 33, None);
                pill.timer_on = true;
            } else if !animate && pill.timer_on {
                KillTimer(hwnd, IDT_PILL_ANIM);
                pill.timer_on = false;
            }
        });
    }

    unsafe fn present_layered(hwnd: HWND, bgra: &[u32], w: i32, h: i32, x: i32, y: i32) {
        let hdc_screen = GetDC(std::ptr::null_mut());
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbm: HBITMAP = CreateDIBSection(hdc_mem, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if !hbm.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u32, bgra.len().min((w * h) as usize));
            let old = SelectObject(hdc_mem, hbm as _);
            let pt_src = POINT { x: 0, y: 0 };
            let pt_dst = POINT { x, y };
            let sz = SIZE { cx: w, cy: h };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(hwnd, hdc_screen, &pt_dst, &sz, hdc_mem, &pt_src, 0, &blend, ULW_ALPHA);
            SelectObject(hdc_mem, old);
            DeleteObject(hbm as _);
        }
        DeleteDC(hdc_mem);
        ReleaseDC(std::ptr::null_mut(), hdc_screen);
    }

    /// Persisted position = top-left of the idle circle (stable across DPI and state changes).
    unsafe fn save_pill_anchor(hwnd: HWND) {
        let scale = dpi_scale(hwnd);
        let idle_w = pill_window_width(scale, PillKind::Idle, 0, 0) as i32;
        let pos = PILL.with(|p| p.borrow().as_ref().map(|pill| (pill.cx - idle_w / 2, pill.top)));
        if let Some((x, y)) = pos {
            send(UiAction::SavePillPosition(x, y));
        }
    }

    /// Snaps to a comfortable margin when released near a work-area edge.
    unsafe fn snap_to_edges(hwnd: HWND) {
        let scale = dpi_scale(hwnd);
        let h = PillMetrics::for_scale(scale).window_height() as i32;
        let idle_w = pill_window_width(scale, PillKind::Idle, 0, 0) as i32;
        let snap = (18.0 * scale) as i32;
        let margin = (8.0 * scale) as i32;
        PILL.with(|p| {
            if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                let wa = work_area_at(pill.cx, pill.top + h / 2);
                let left = pill.cx - idle_w / 2;
                if left - wa.left < snap {
                    pill.cx = wa.left + margin + idle_w / 2;
                }
                if wa.right - (left + idle_w) < snap {
                    pill.cx = wa.right - margin - idle_w / 2;
                }
                if pill.top - wa.top < snap {
                    pill.top = wa.top + margin;
                }
                if wa.bottom - (pill.top + h) < snap {
                    pill.top = wa.bottom - margin - h;
                }
            }
        });
    }

    /// After display changes, bring a listener that ended up off-screen back home.
    unsafe fn revalidate_pill_position(hwnd: HWND) {
        let scale = dpi_scale(hwnd);
        PILL.with(|p| {
            if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                let mon = MonitorFromPoint(POINT { x: pill.cx, y: pill.top + 4 }, MONITOR_DEFAULTTONULL);
                if mon.is_null() {
                    let (cx, top) = default_anchor(scale);
                    pill.cx = cx;
                    pill.top = top;
                }
            }
        });
    }

    /// Moves the listener back to its default home (Settings › General).
    pub(crate) unsafe fn reset_pill_position() {
        let Some(g) = GUI_GLOBALS.get() else { return };
        let ph = *g.pill_hwnd.lock() as HWND;
        if ph.is_null() {
            return;
        }
        let scale = dpi_scale(ph);
        let (cx, top) = default_anchor(scale);
        PILL.with(|p| {
            if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                pill.cx = cx;
                pill.top = top;
            }
        });
        save_pill_anchor(ph);
        pill_refresh(ph, false);
    }

    /// Diagnostics for the UI gallery / performance report.
    pub fn pill_debug_info() -> Option<(i32, i32, f32, bool, u64)> {
        PILL.with(|p| p.borrow().as_ref().map(|x| (x.cx, x.top, x.cur_w, x.timer_on, x.frames_painted)))
    }

    /// Test hook: force the hover-expanded state (the gallery cannot move the user's mouse).
    pub fn set_pill_hover_for_test(hovered: bool) {
        PILL.with(|p| {
            if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                pill.hovered = hovered;
            }
        });
        if let Some(g) = GUI_GLOBALS.get() {
            let ph = *g.pill_hwnd.lock() as HWND;
            if !ph.is_null() {
                unsafe { pill_refresh(ph, false) };
            }
        }
    }

    unsafe extern "system" fn pill_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT, // Never steal focus from the dictation target.
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
                1
            }
            WM_LBUTTONDOWN => {
                // Use where the press happened (lParam), not where the pointer is by the time we
                // process it: a quick press-and-drag otherwise lost its first movement.
                let mut pt = POINT {
                    x: (lparam & 0xFFFF) as u16 as i16 as i32,
                    y: ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
                };
                let _ = ClientToScreen(hwnd, &mut pt);
                PILL.with(|p| {
                    if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                        pill.drag = Some((pt.x, pt.y, false));
                    }
                });
                SetCapture(hwnd);
                0
            }
            WM_MOUSEMOVE => {
                let mut moved_now = false;
                let mut arm_tracking = false;
                let threshold = (4.0 * dpi_scale(hwnd)) as i32;
                PILL.with(|p| {
                    if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                        if !pill.hover_tracking {
                            pill.hover_tracking = true;
                            arm_tracking = true;
                        }
                        if let Some((sx, sy, moved)) = pill.drag {
                            let mut pt = POINT { x: 0, y: 0 };
                            let _ = GetCursorPos(&mut pt);
                            let (dx, dy) = (pt.x - sx, pt.y - sy);
                            if moved || dx.abs() > threshold || dy.abs() > threshold {
                                pill.cx += dx;
                                pill.top += dy;
                                pill.drag = Some((pt.x, pt.y, true));
                                pill.hovered = false;
                                moved_now = true;
                            }
                        }
                    }
                });
                if arm_tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_HOVER | TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: HOVER_INTENT_MS,
                    };
                    TrackMouseEvent(&mut tme);
                }
                if moved_now {
                    pill_refresh(hwnd, false);
                }
                0
            }
            WM_MOUSEHOVER_U => {
                let dragging = PILL.with(|p| p.borrow().as_ref().map(|x| x.drag.is_some()).unwrap_or(false));
                if !dragging {
                    KillTimer(hwnd, IDT_HOVER_COLLAPSE);
                    PILL.with(|p| {
                        if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                            pill.hovered = true;
                        }
                    });
                    pill_refresh(hwnd, false);
                }
                0
            }
            WM_MOUSELEAVE_U => {
                PILL.with(|p| {
                    if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                        pill.hover_tracking = false;
                    }
                });
                // Small grace period so a pointer brushing the edge does not make it flicker.
                SetTimer(hwnd, IDT_HOVER_COLLAPSE, 220, None);
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                // Apply the pointer's final position first: on a fast flick the button-up can be
                // processed before the last WM_MOUSEMOVE, which used to leave the listener short.
                let mut pt = POINT { x: 0, y: 0 };
                let _ = GetCursorPos(&mut pt);
                let threshold = (4.0 * dpi_scale(hwnd)) as i32;
                let was_drag = PILL.with(|p| {
                    let Ok(mut b) = p.try_borrow_mut() else { return false };
                    let Some(pill) = b.as_mut() else { return false };
                    let Some((sx, sy, moved)) = pill.drag.take() else { return false };
                    let (dx, dy) = (pt.x - sx, pt.y - sy);
                    if moved || dx.abs() > threshold || dy.abs() > threshold {
                        pill.cx += dx;
                        pill.top += dy;
                        true
                    } else {
                        false
                    }
                });
                if was_drag {
                    snap_to_edges(hwnd);
                    save_pill_anchor(hwnd);
                    pill_refresh(hwnd, false);
                } else if let Some(g) = GUI_GLOBALS.get() {
                    let (clickable, ready, is_error, disabled) = {
                        let m = g.model.lock();
                        (
                            m.config.floating_mic_clickable,
                            m.active_model_ready(),
                            matches!(m.state, EngineState::Error(_)),
                            matches!(m.state, EngineState::Disabled),
                        )
                    };
                    if !ready {
                        // Recovery action: take the user straight to the thing that needs fixing.
                        crate::settings::open(Some(crate::settings::PAGE_MODELS));
                    } else if is_error {
                        crate::settings::open(None);
                    } else if clickable && !disabled {
                        send(UiAction::FloatingMicClicked);
                    }
                }
                0
            }
            WM_RBUTTONUP => {
                if let Some(g) = GUI_GLOBALS.get() {
                    let th = *g.tray_hwnd.lock() as HWND;
                    if !th.is_null() {
                        show_context_menu(th);
                    }
                }
                0
            }
            WM_TIMER => {
                if wparam == IDT_HOVER_COLLAPSE {
                    KillTimer(hwnd, IDT_HOVER_COLLAPSE);
                    let mut pt = POINT { x: 0, y: 0 };
                    let mut rc: RECT = std::mem::zeroed();
                    let inside = GetCursorPos(&mut pt) != 0
                        && GetWindowRect(hwnd, &mut rc) != 0
                        && pt.x >= rc.left
                        && pt.x < rc.right
                        && pt.y >= rc.top
                        && pt.y < rc.bottom;
                    if !inside {
                        PILL.with(|p| {
                            if let Some(pill) = p.try_borrow_mut().ok().as_mut().and_then(|b| b.as_mut()) {
                                pill.hovered = false;
                            }
                        });
                        pill_refresh(hwnd, false);
                    }
                } else if wparam == IDT_PILL_ANIM {
                    pill_refresh(hwnd, true);
                }
                0
            }
            WM_DPICHANGED => {
                // pill_refresh re-centres vertically when the scale changes.
                pill_refresh(hwnd, false);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Tray menu
    // ---------------------------------------------------------------------------------------------

    unsafe fn show_context_menu(hwnd: HWND) {
        let Some(g) = GUI_GLOBALS.get() else { return };
        let snap = g.model.lock().clone();
        let hmenu = CreatePopupMenu();
        let append = |menu: HMENU, flags: u32, id: usize, text: &str| {
            let w = to_wide(text);
            let _ = AppendMenuW(menu, flags, id, w.as_ptr());
        };
        let check = |b: bool| if b { MF_CHECKED } else { MF_UNCHECKED };

        let status = match &snap.state {
            EngineState::Error(m) => friendly_error(m).short.to_string(),
            EngineState::Idle if !snap.active_model_ready() => "Speech model needed".to_string(),
            EngineState::Idle => format!("Ready · hold {}", snap.config.ptt_hotkey.display_name),
            s => s.status_label().to_string(),
        };
        append(hmenu, MF_STRING | MF_GRAYED, 0, &format!("{PRODUCT_NAME}  ·  {status}"));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        if !snap.active_model_ready() {
            append(hmenu, MF_STRING, IDM_SETUP_MODEL, "Set up speech model…");
        }

        let hmode = CreatePopupMenu();
        for (i, m) in WritingMode::ALL.iter().enumerate() {
            append(hmode, MF_STRING, IDM_MODE_BASE + i, m.label());
        }
        if let Some(sel) = WritingMode::ALL.iter().position(|m| *m == snap.config.default_writing_mode) {
            let _ = CheckMenuRadioItem(
                hmode,
                IDM_MODE_BASE as u32,
                (IDM_MODE_BASE + WritingMode::ALL.len() - 1) as u32,
                (IDM_MODE_BASE + sel) as u32,
                MF_BYCOMMAND,
            );
        }
        let _ = AppendMenuW(hmode, MF_SEPARATOR, 0, std::ptr::null());
        append(hmode, MF_STRING | check(snap.config.auto_context_mode), IDM_AUTO_CTX, "Adapt to the app I'm using");
        let wm = to_wide(&format!("Writing mode: {}", snap.config.default_writing_mode.short_label()));
        let _ = AppendMenuW(hmenu, MF_POPUP, hmode as usize, wm.as_ptr());

        let hout = CreatePopupMenu();
        append(hout, MF_STRING, IDM_OUT_TYPE, OutputMode::DirectTyping.title());
        append(hout, MF_STRING, IDM_OUT_CLIP, OutputMode::Clipboard.title());
        append(hout, MF_STRING, IDM_OUT_ASK, OutputMode::AskBeforeInserting.title());
        let out_sel = match snap.config.output_mode {
            OutputMode::DirectTyping => IDM_OUT_TYPE,
            OutputMode::Clipboard => IDM_OUT_CLIP,
            OutputMode::AskBeforeInserting => IDM_OUT_ASK,
        };
        let _ = CheckMenuRadioItem(hout, IDM_OUT_CLIP as u32, IDM_OUT_ASK as u32, out_sel as u32, MF_BYCOMMAND);
        let ow = to_wide(&format!("Output: {}", snap.config.output_mode.title()));
        let _ = AppendMenuW(hmenu, MF_POPUP, hout as usize, ow.as_ptr());
        append(hmenu, MF_STRING, IDM_COPY_LAST, "Copy last dictation");

        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        append(hmenu, MF_STRING, IDM_OPEN_SETTINGS, "Open KeyReed…");
        append(hmenu, MF_STRING | check(snap.config.show_floating_mic), IDM_SHOW_PILL, "Show floating listener");
        append(
            hmenu,
            MF_STRING,
            IDM_APP_ENABLE,
            if snap.config.app_enabled { "Pause KeyReed" } else { "Resume KeyReed" },
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        append(hmenu, MF_STRING, IDM_EXIT, "Quit KeyReed");
        let _ = SetMenuDefaultItem(hmenu, IDM_OPEN_SETTINGS as u32, 0);

        let mut pt = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(hmenu, TPM_BOTTOMALIGN | TPM_RIGHTALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null());
        let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
        let _ = DestroyMenu(hmenu);
    }

    unsafe extern "system" fn tray_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY_CALLBACK => {
                let ev = (lparam & 0xFFFF) as u32;
                if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                    show_context_menu(hwnd);
                } else if ev == WM_LBUTTONUP || ev == WM_LBUTTONDBLCLK {
                    crate::settings::open(None);
                }
                0
            }
            WM_SYNC_UI => {
                if let Some(g) = GUI_GLOBALS.get() {
                    let snap = g.model.lock().clone();
                    let status = match &snap.state {
                        EngineState::Error(m) => friendly_error(m).short.to_string(),
                        EngineState::Idle if !snap.active_model_ready() => "Speech model needed".into(),
                        EngineState::Idle => "Ready".into(),
                        s => s.status_label().to_string(),
                    };
                    let tip = format!(
                        "{PRODUCT_NAME}: {status}\n{} · hold {}",
                        snap.config.default_writing_mode.short_label(),
                        snap.config.ptt_hotkey.display_name
                    );
                    update_tray_icon(hwnd, NIM_MODIFY, &snap.state, !snap.active_model_ready(), &tip);
                    let ph = *g.pill_hwnd.lock() as HWND;
                    if !ph.is_null() {
                        pill_refresh(ph, false);
                    }
                    crate::settings::on_model_changed();
                }
                0
            }
            WM_SHOW_TOAST => {
                if let Some(g) = GUI_GLOBALS.get() {
                    if let Some((title, body, warn)) = g.toast_buf.lock().take() {
                        show_tray_balloon(hwnd, &title, &body, warn);
                    }
                }
                0
            }
            WM_ALREADY_RUNNING => {
                crate::settings::open(None);
                0
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                if let Some(g) = GUI_GLOBALS.get() {
                    let ph = *g.pill_hwnd.lock() as HWND;
                    if !ph.is_null() {
                        revalidate_pill_position(ph);
                        pill_refresh(ph, false);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_COMMAND => {
                let id = wparam & 0xFFFF;
                match id {
                    IDM_APP_ENABLE => send(UiAction::ToggleAppEnabled),
                    IDM_OUT_CLIP => send(UiAction::SetOutputMode(OutputMode::Clipboard)),
                    IDM_OUT_TYPE => send(UiAction::SetOutputMode(OutputMode::DirectTyping)),
                    IDM_OUT_ASK => send(UiAction::SetOutputMode(OutputMode::AskBeforeInserting)),
                    IDM_COPY_LAST => send(UiAction::CopyLastDictation),
                    IDM_AUTO_CTX => send(UiAction::ToggleAutoContext),
                    IDM_SHOW_PILL => send(UiAction::ToggleFloatingMicVisible),
                    IDM_OPEN_SETTINGS => crate::settings::open(None),
                    IDM_SETUP_MODEL => crate::settings::open(Some(crate::settings::PAGE_MODELS)),
                    IDM_EXIT => {
                        send(UiAction::Quit);
                        PostQuitMessage(0);
                    }
                    m if (IDM_MODE_BASE..IDM_MODE_BASE + WritingMode::ALL.len()).contains(&m) => {
                        send(UiAction::SetWritingMode(WritingMode::ALL[m - IDM_MODE_BASE]));
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1;
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Cross-thread handle
    // ---------------------------------------------------------------------------------------------

    #[derive(Clone)]
    pub struct WinGuiHandle {
        tray_hwnd: isize,
        model: Arc<Mutex<SharedUiModel>>,
    }

    impl WinGuiHandle {
        fn post_sync(&self) {
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_SYNC_UI, 0, 0);
            }
        }

        pub fn sync(&self, state: EngineState, rms: f32, config: Option<AppConfig>) {
            {
                let mut g = self.model.lock();
                g.state = state;
                g.rms_level = rms;
                if let Some(c) = config {
                    g.config = c;
                }
            }
            self.post_sync();
        }

        /// Replaces "Transcribing" with a more specific, truthful label (or clears it).
        pub fn set_processing_note(&self, note: Option<&str>) {
            self.model.lock().processing_note = note.map(|s| s.to_string());
        }

        /// Shows a short message on the listener for ~1.6 s without a notification.
        pub fn flash_hint(&self, label: &str, secondary: &str) {
            self.model.lock().hint = Some((label.to_string(), secondary.to_string(), std::time::Instant::now()));
            self.post_sync();
        }

        /// Records what happened to the dictation; call before `sync(ReadyToPaste)`.
        pub fn set_outcome(&self, outcome: ReadyOutcome) {
            self.model.lock().last_outcome = Some(outcome);
        }

        pub fn update_status(&self, status_detail: &str, download_in_progress: bool, installed_models: Option<Vec<String>>) {
            {
                let mut g = self.model.lock();
                g.status_detail = status_detail.to_string();
                g.download_in_progress = download_in_progress;
                if let Some(inst) = installed_models {
                    g.installed_asr_models = inst;
                }
            }
            self.post_sync();
        }

        pub fn set_download(&self, progress: Option<DownloadProgress>, installed_models: Option<Vec<String>>) {
            {
                let mut g = self.model.lock();
                g.download_in_progress = progress.as_ref().map(|p| p.is_active()).unwrap_or(false);
                g.status_detail = progress.as_ref().map(|p| p.detail_text()).unwrap_or_default();
                g.download = progress;
                if let Some(inst) = installed_models {
                    g.installed_asr_models = inst;
                }
            }
            self.post_sync();
        }

        pub fn set_mic_test(&self, status: MicTestStatus) {
            self.model.lock().mic_test = status;
            self.post_sync();
        }

        pub fn set_microphones(&self, mics: Vec<String>) {
            self.model.lock().available_microphones = mics;
            self.post_sync();
        }

        pub fn notify_toast(&self, title: &str, body: &str) {
            self.toast(title, body, false);
        }

        pub fn notify_warning(&self, title: &str, body: &str) {
            self.toast(title, body, true);
        }

        fn toast(&self, title: &str, body: &str, warning: bool) {
            if let Some(g) = GUI_GLOBALS.get() {
                *g.toast_buf.lock() = Some((title.to_string(), body.to_string(), warning));
            }
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_SHOW_TOAST, 0, 0);
            }
        }

        pub fn open_settings(&self) {
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_COMMAND, IDM_OPEN_SETTINGS, 0);
            }
        }

        pub fn quit(&self) {
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_DESTROY, 0, 0);
            }
        }

        pub fn pill_hwnd(&self) -> isize {
            GUI_GLOBALS.get().map(|g| *g.pill_hwnd.lock()).unwrap_or(0)
        }
    }

    pub fn init_and_spawn_gui_windows(
        tx: Sender<UiAction>,
        initial_cfg: AppConfig,
        available_microphones: Vec<String>,
        installed_asr_models: Vec<String>,
        open_settings_immediately: bool,
    ) -> anyhow::Result<(WinGuiHandle, Box<dyn FnOnce()>)> {
        let model = Arc::new(Mutex::new(SharedUiModel {
            config: initial_cfg.clone(),
            state: if initial_cfg.app_enabled { EngineState::Idle } else { EngineState::Disabled },
            rms_level: 0.0,
            status_detail: String::new(),
            available_microphones,
            installed_asr_models,
            download_in_progress: false,
            download: None,
            mic_test: MicTestStatus::default(),
            last_outcome: None,
            hint: None,
            processing_note: None,
        }));

        let _ = GUI_GLOBALS.set(WinGuiGlobals {
            tx,
            model: Arc::clone(&model),
            tray_hwnd: Mutex::new(0),
            pill_hwnd: Mutex::new(0),
            toast_buf: Mutex::new(None),
        });

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let reg_class = |name: &str, proc_fn: WNDPROC| {
                let nw = to_wide(name);
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_DBLCLKS,
                    lpfnWndProc: proc_fn,
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinst,
                    hIcon: std::ptr::null_mut(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: nw.as_ptr(),
                    hIconSm: std::ptr::null_mut(),
                };
                RegisterClassExW(&wc);
            };
            reg_class(TRAY_WINDOW_CLASS, Some(tray_wnd_proc));
            reg_class(OVERLAY_WINDOW_CLASS, Some(pill_wnd_proc));
            crate::settings::register_class();

            let tray_cls = to_wide(TRAY_WINDOW_CLASS);
            let tray_hwnd = CreateWindowExW(
                0,
                tray_cls.as_ptr(),
                tray_cls.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if tray_hwnd.is_null() {
                anyhow::bail!("Failed to create tray host window");
            }

            let pill_cls = to_wide(OVERLAY_WINDOW_CLASS);
            let pill_title = to_wide("KeyReed listener");
            let pill_hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                pill_cls.as_ptr(),
                pill_title.as_ptr(),
                WS_POPUP,
                0,
                0,
                44,
                44,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );

            if !pill_hwnd.is_null() {
                // The saved anchor was written in the DPI of the monitor it is on, so convert it with
                // that monitor's DPI (the new window still has the primary monitor's DPI here).
                let scale0 = match initial_cfg.floating_pill_pos {
                    Some((sx, sy)) if dpi_override() == 0 => {
                        let mon = MonitorFromPoint(POINT { x: sx + 4, y: sy + 4 }, MONITOR_DEFAULTTONULL);
                        let (mut dx, mut dy) = (0u32, 0u32);
                        if !mon.is_null() && GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) == 0 && dx > 0 {
                            dx as f32 / 96.0
                        } else {
                            dpi_scale(pill_hwnd)
                        }
                    }
                    _ => dpi_scale(pill_hwnd),
                };
                let idle_w0 = pill_window_width(scale0, PillKind::Idle, 0, 0) as i32;
                let (cx, top) = match initial_cfg.floating_pill_pos {
                    Some((sx, sy))
                        if !MonitorFromPoint(POINT { x: sx + idle_w0 / 2, y: sy + 4 }, MONITOR_DEFAULTTONULL)
                            .is_null() =>
                    {
                        (sx + idle_w0 / 2, sy)
                    }
                    _ => default_anchor(scale0),
                };
                let now = Instant::now();
                PILL.with(|p| {
                    *p.borrow_mut() = Some(Pill {
                        cx,
                        top,
                        last_scale: scale0,
                        cur_w: idle_w0 as f32,
                        from_w: idle_w0 as f32,
                        to_w: idle_w0 as f32,
                        tween_start: now,
                        content_since: now,
                        state_since: now,
                        last_kind: PillKind::Idle,
                        last_label: String::new(),
                        levels: VecDeque::with_capacity(16),
                        tick: 0,
                        hovered: false,
                        hover_tracking: false,
                        drag: None,
                        timer_on: false,
                        visible: false,
                        frames_painted: 0,
                    })
                });
            }

            if let Some(g) = GUI_GLOBALS.get() {
                *g.tray_hwnd.lock() = tray_hwnd as isize;
                *g.pill_hwnd.lock() = pill_hwnd as isize;
            }

            let ready = model.lock().active_model_ready();
            update_tray_icon(
                tray_hwnd,
                NIM_ADD,
                &EngineState::Idle,
                !ready,
                &format!("{PRODUCT_NAME}: {}", if ready { "Ready" } else { "Speech model needed" }),
            );
            if !pill_hwnd.is_null() {
                pill_refresh(pill_hwnd, false);
            }
            if open_settings_immediately {
                crate::settings::open(None);
            }

            let handle = WinGuiHandle { tray_hwnd: tray_hwnd as isize, model };
            let pump = Box::new(move || {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if crate::settings::pre_translate(&msg) {
                        continue;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            });
            Ok((handle, pump))
        }
    }

    pub fn signal_already_running() -> bool {
        let class: Vec<u16> = to_wide(TRAY_WINDOW_CLASS);
        unsafe {
            let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
            if hwnd.is_null() {
                return false;
            }
            PostMessageW(hwnd, WM_ALREADY_RUNNING, 0, 0) != 0
        }
    }
}
