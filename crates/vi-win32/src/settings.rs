//! KeyReed Settings window: sidebar navigation, ten focused pages, a four-step first-run setup,
//! and instant-apply (every change is saved immediately; there is no Save button to forget).
//!
//! Rendering: the window background, text and illustrations are painted with `crate::canvas`
//! (anti-aliased shapes + ClearType text, double-buffered). Interactive elements are real child
//! windows so Tab / Shift+Tab / Space / Enter / Esc and screen readers keep working:
//! owner-drawn BUTTONs for navigation, switches, choice cards and buttons; native COMBOBOX/EDIT
//! controls (Common Controls v6 visual style) for lists and text.

#![cfg(windows)]

use crate::canvas::{rgb, text_width, Canvas, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK};
use crate::overlay::scaled_icon;
use crate::ui::win_gui::{brand_hicon, globals, send, to_wide, ui_font_face};
use crate::ui::{
    asr_ui_info, friendly_error, hotkey_conflict, hotkey_presets, DownloadStage, ErrorFix, MicVerdict, ReadyOutcome,
    SharedUiModel, UiAction, ASR_UI_CATALOG,
};
use std::cell::RefCell;
use std::time::Instant;
use vi_core::brand::{PRODUCT_NAME, PRODUCT_TAGLINE, SETTINGS_WINDOW_CLASS, VERSION_STR};
use vi_core::habits::Dialect;
use vi_core::{
    ActivationMode, AppConfig, AudioCueStyle, EngineState, HotkeySpec, MouseAction, OutputMode,
    ProcessingProfile, WritingMode,
};
use vi_core::shortcut::{CaptureStep, ShortcutCapture, ShortcutRole};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRectRgn, CreateSolidBrush, DeleteObject, EndPaint, GetMonitorInfoW,
    IntersectClipRect, InvalidateRect, MonitorFromPoint, RestoreDC, SaveDC, SetBkColor, SetTextColor, SetWindowRgn,
    HDC, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const PAGE_START: usize = 0;
pub const PAGE_GENERAL: usize = 1;
pub const PAGE_DICTATION: usize = 2;
pub const PAGE_WRITING: usize = 3;
pub const PAGE_MYSTYLE: usize = 4;
pub const PAGE_OUTPUT: usize = 5;
pub const PAGE_AUDIO: usize = 6;
pub const PAGE_MODELS: usize = 7;
pub const PAGE_PROCESSING: usize = 8;
pub const PAGE_PRIVACY: usize = 9;
pub const PAGE_ABOUT: usize = 10;

/// (label, Segoe Fluent/MDL2 glyph)
const NAV: [(&str, char); 11] = [
    ("Get started", '\u{E80F}'),
    ("General", '\u{E713}'),
    ("Dictation", '\u{E765}'),
    ("Writing", '\u{E70F}'),
    ("My style", '\u{E790}'),
    ("Output", '\u{E77F}'),
    ("Microphone", '\u{E720}'),
    ("Speech models", '\u{E8F1}'),
    ("Processing", '\u{E9F5}'),
    ("Privacy", '\u{E72E}'),
    ("About", '\u{E946}'),
];

const REPO_URL: &str = "https://github.com/Shanmukh-l33t/keyreed";

// Logical (96-DPI) geometry.
const WIN_W: i32 = 940;
const WIN_H: i32 = 660;
/// The frameless header: KeyReed's own title row (mark, name, window buttons), drawn on the canvas.
const HEADER_H: i32 = 52;
/// First navigation row, below the header.
const NAV_Y: i32 = HEADER_H + 20;
const SIDEBAR_W: i32 = 236;
const X0: i32 = SIDEBAR_W + 40;
const CW: i32 = WIN_W - X0 - 40;
const ROW_H: i32 = 56;
const ROW_H_SHORT: i32 = 46;
/// One wheel notch: a whole settings row, so the page lands on row boundaries instead of between them.
const WHEEL_STEP: i32 = 56;
const PAD: i32 = 16;
/// The confirmation card (replaces the stock MessageBox).
const MODAL_W: i32 = 460;
const MODAL_H: i32 = 196;

// Control ids.
const ID_NAV: i32 = 100;
const HEADER_MIN: i32 = 90;
const HEADER_CLOSE: i32 = 91;
/// The confirmation card's two buttons (see `ask`).
const MODAL_CANCEL: i32 = 92;
const MODAL_CONFIRM: i32 = 93;
const G_ENABLED: i32 = 201;
const G_AUTOSTART: i32 = 202;
const G_PILL: i32 = 203;
const G_PILL_CLICK: i32 = 204;
const G_PILL_RESET: i32 = 205;
const G_SOUNDS: i32 = 206;
const G_NOTIFY: i32 = 207;
const D_HOTKEY: i32 = 301;
const D_HOLD: i32 = 302;
const D_TOGGLE: i32 = 303;
const D_PTT: i32 = 304;
const D_QF_KEY: i32 = 305;
const D_QF_REVIEW: i32 = 306;
const D_VOICE_KEY: i32 = 307;
const D_COMPOSER_KEY: i32 = 308;
/// The middle, back and forward button rows of the "Mouse" group, in that order.
const D_MOUSE: i32 = 320;
const W_MODE: i32 = 400;
const W_AUTOCTX: i32 = 410;
/// The "Edit" button of the personal dictionary row on the Writing page.
const W_VOCAB: i32 = 411;
// Personal dictionary editor: one Term field, one Sounds like field and one Remove button per visible row.
const V_TERM: i32 = 1100;
const V_SOUND: i32 = 1110;
const V_DEL: i32 = 1120;
const V_ADD: i32 = 1130;
const V_DONE: i32 = 1131;
const V_PREV: i32 = 1132;
const V_NEXT: i32 = 1133;
/// Visible rows per page of the dictionary editor.
const DICT_ROWS: usize = 8;
const DICT_MAX_TERMS: usize = 500;
const DICT_MAX_TERM_CHARS: usize = 64;
const DICT_MAX_SOUNDS_CHARS: usize = 400;
const O_TYPE: i32 = 501;
const O_CLIP: i32 = 502;
const O_ASK: i32 = 503;
const O_COPYLAST: i32 = 505;
const A_MIC: i32 = 601;
const A_TEST: i32 = 602;
/// The "Sounds" group on the Microphone page: cue style, its own volume, and a Play button.
const A_CUE: i32 = 605;
const A_VOL: i32 = 606;
const A_PLAY: i32 = 607;
/// "Open privacy settings" under a failed or silent microphone test (shared by the Microphone page and setup).
const MIC_PRIVACY: i32 = 604;
const M_ACT: i32 = 700;
/// The Quick Fix grammar model's row: outside the speech-model action range, and its own install path.
const M_GRAMMAR: i32 = M_ACT + 8;
const GRAMMAR_MODEL_ID: &str = "gector-bert-en-int8";
const P_PROF: i32 = 800;
const P_KEY: i32 = 810;
const P_KEY_SAVE: i32 = 811;
const P_KEY_REMOVE: i32 = 812;
const P_LLM: i32 = 813;
const V_PWD: i32 = 850;
const V_REDACT: i32 = 851;
const V_CLIPHIST: i32 = 852;
const V_CLEAR_HIST: i32 = 853;
/// Text jobs (Composer, Quick Fix, voice commands) may use the cloud key. Never under Maximum Privacy.
const V_CLOUD_JOBS: i32 = 854;
const AB_RELEASES: i32 = 901;
const AB_UPDATES: i32 = 902;
const AB_GITHUB: i32 = 903;
const AB_LICENSE: i32 = 904;
const AB_SETUP: i32 = 905;
const AB_RESET: i32 = 906;
const S_HOTKEY: i32 = 1001;
const S_HOLD: i32 = 1002;
const S_TOGGLE: i32 = 1003;
const S_MIC: i32 = 1004;
const S_TEST: i32 = 1005;
const S_DL: i32 = 1006;
const S_ALLMODELS: i32 = 1007;
const S_PRACTICE: i32 = 1008;
const S_BACK: i32 = 1010;
const S_NEXT: i32 = 1011;
const S_SKIP: i32 = 1012;
// My style page (also the setup switch, which flips the same setting).
const Y_LEARN: i32 = 1300;
const Y_DIALECT: i32 = 1301;
const Y_ITEM: i32 = 1310;
const Y_CAND: i32 = 1330;
const Y_CAND_DISMISS: i32 = Y_CAND + 16;
const Y_APP: i32 = 1400;
const Y_PREV: i32 = 1420;
const Y_NEXT: i32 = 1421;
const Y_RESET: i32 = 1422;
const Y_EXPORT: i32 = 1423;
const Y_DELETE: i32 = 1424;
/// Learned habits per page of the list.
const STYLE_ITEMS_PER_PAGE: usize = 6;
const STYLE_CANDIDATES_SHOWN: usize = 5;
const STYLE_APPS_SHOWN: usize = 12;

const IDT_SAVED: usize = 9001;
/// One-shot: how long the finished model step shows "ready" before setup moves on.
const IDT_ADVANCE: usize = 9003;
const ADVANCE_MS: u32 = 700;
/// Posted when a dictionary field loses focus; the commit runs once focus has settled.
const WM_DICT_COMMIT: u32 = WM_APP + 1;
const WM_MOUSELEAVE_U: u32 = 0x02A3;

// ------------------------------------------------------------------------------------------------
// Theme (light, derived from the KeyReed emerald brand; high-contrast uses system colours)
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) bg: u32,
    pub(crate) sidebar: u32,
    pub(crate) surface: u32,
    pub(crate) border: u32,
    /// Boundary of buttons that have no fill (needs >= 3:1 against the surface, WCAG 1.4.11).
    pub(crate) control_border: u32,
    pub(crate) divider: u32,
    pub(crate) text: u32,
    pub(crate) text2: u32,
    pub(crate) text3: u32,
    pub(crate) accent: u32,
    pub(crate) accent_fill: u32,
    pub(crate) accent_press: u32,
    pub(crate) accent_soft: u32,
    pub(crate) hover: u32,
    pub(crate) hover_card: u32,
    pub(crate) on_accent: u32,
    pub(crate) warn_fg: u32,
    pub(crate) warn_bg: u32,
    pub(crate) err_fg: u32,
    pub(crate) err_bg: u32,
    pub(crate) ok_fg: u32,
    pub(crate) ok_bg: u32,
    pub(crate) track: u32,
    pub(crate) disabled_bg: u32,
    pub(crate) focus: u32,
    /// Keyboard focus ring on controls (emerald); `focus` (the text colour) remains for high contrast.
    pub(crate) focus_ring: u32,
    /// "Working" hue (transcribing, downloading): teal, so amber stays reserved for "needs attention".
    pub(crate) working: u32,
    pub(crate) hc: bool,
}

#[repr(C)]
struct HighContrastW {
    cb_size: u32,
    dw_flags: u32,
    scheme: *mut u16,
}

pub(crate) fn high_contrast_on() -> bool {
    unsafe {
        let mut hc = HighContrastW { cb_size: std::mem::size_of::<HighContrastW>() as u32, dw_flags: 0, scheme: std::ptr::null_mut() };
        let ok = SystemParametersInfoW(0x0042, hc.cb_size, &mut hc as *mut _ as *mut _, 0);
        ok != 0 && (hc.dw_flags & 1) != 0
    }
}

static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Re-reads which colour scheme Settings should use: Windows "Choose your default app mode"
/// (HKCU\...\Themes\Personalize\AppsUseLightTheme). If the value is missing or unreadable the
/// light theme is used. `KEYREED_UI_THEME=dark|light` overrides it for screenshots and testing.
pub(crate) fn refresh_dark_mode() {
    let forced = std::env::var("KEYREED_UI_THEME").ok();
    let dark = match forced.as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => system_apps_use_dark(),
    };
    DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
}

fn system_apps_use_dark() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let key = to_wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let val = to_wide("AppsUseLightTheme");
    let mut data: u32 = 1;
    let mut len = std::mem::size_of::<u32>() as u32;
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut data as *mut u32 as *mut _, &mut len) };
    rc == 0 && data == 0
}

pub(crate) fn theme() -> Theme {
    if high_contrast_on() {
        use windows_sys::Win32::Graphics::Gdi::GetSysColor;
        unsafe {
            let win = GetSysColor(5); // COLOR_WINDOW
            let txt = GetSysColor(8); // COLOR_WINDOWTEXT
            let hl = GetSysColor(13); // COLOR_HIGHLIGHT
            let hlt = GetSysColor(14); // COLOR_HIGHLIGHTTEXT
            let gray = GetSysColor(17); // COLOR_GRAYTEXT
            let face = GetSysColor(15); // COLOR_BTNFACE
            return Theme {
                bg: win,
                sidebar: win,
                surface: win,
                border: txt,
                control_border: txt,
                divider: gray,
                text: txt,
                text2: txt,
                text3: gray,
                accent: hl,
                accent_fill: hl,
                accent_press: hl,
                accent_soft: face,
                hover: face,
                hover_card: face,
                on_accent: hlt,
                warn_fg: txt,
                warn_bg: win,
                err_fg: txt,
                err_bg: win,
                ok_fg: txt,
                ok_bg: win,
                track: gray,
                disabled_bg: win,
                focus: txt,
                focus_ring: txt,
                working: txt,
                hc: true,
            };
        }
    }
    if DARK.load(std::sync::atomic::Ordering::Relaxed) {
        // Same emerald identity on the listener's dark green-black. Every text/control pair is checked
        // against WCAG 2.2 AA (text 4.5:1, control boundaries 3:1); see docs/design/02-dark-theme.md.
        return Theme {
            bg: rgb(0x0F, 0x17, 0x13),
            sidebar: rgb(0x0B, 0x12, 0x0F),
            surface: rgb(0x17, 0x23, 0x1D),
            border: rgb(0x26, 0x35, 0x2D),
            control_border: rgb(0x6F, 0x84, 0x79),
            divider: rgb(0x1F, 0x2C, 0x25),
            text: rgb(0xE8, 0xF1, 0xEC),
            text2: rgb(0xA9, 0xBB, 0xB2),
            text3: rgb(0x8D, 0xA3, 0x98),
            accent: rgb(0x5E, 0xE0, 0xB0),
            accent_fill: rgb(0x34, 0xD3, 0x99),
            accent_press: rgb(0x10, 0xB9, 0x81),
            accent_soft: rgb(0x14, 0x3C, 0x2E),
            hover: rgb(0x22, 0x32, 0x2A),
            hover_card: rgb(0x1C, 0x2B, 0x24),
            on_accent: rgb(0x04, 0x24, 0x1A),
            warn_fg: rgb(0xF8, 0xCE, 0x85),
            warn_bg: rgb(0x3B, 0x2C, 0x10),
            err_fg: rgb(0xFD, 0xA4, 0xAF),
            err_bg: rgb(0x3E, 0x17, 0x22),
            ok_fg: rgb(0x7B, 0xE8, 0xBC),
            ok_bg: rgb(0x10, 0x34, 0x26),
            track: rgb(0x3A, 0x4B, 0x42),
            disabled_bg: rgb(0x14, 0x1E, 0x19),
            focus: rgb(0xE8, 0xF1, 0xEC),
            focus_ring: rgb(0x34, 0xD3, 0x99),
            working: rgb(0x2D, 0xD4, 0xBF),
            hc: false,
        };
    }
    Theme {
        bg: rgb(0xF7, 0xF9, 0xF8),
        sidebar: rgb(0xEE, 0xF3, 0xF0),
        surface: rgb(0xFF, 0xFF, 0xFF),
        border: rgb(0xDD, 0xE5, 0xE1),
        control_border: rgb(0x7F, 0x8F, 0x87),
        divider: rgb(0xEA, 0xEF, 0xEC),
        text: rgb(0x0E, 0x1A, 0x14),
        text2: rgb(0x4A, 0x5A, 0x52),
        // text3 >= 4.5:1 on every surface it is drawn on (was 3.4-3.9). accent_fill carries white text at 5.5:1 (was 3.8).
        text3: rgb(0x5B, 0x6B, 0x63),
        accent: rgb(0x06, 0x5F, 0x46),
        accent_fill: rgb(0x04, 0x78, 0x57),
        accent_press: rgb(0x06, 0x4E, 0x3B),
        accent_soft: rgb(0xE0, 0xF2, 0xE9),
        hover: rgb(0xE4, 0xEB, 0xE7),
        hover_card: rgb(0xF3, 0xF7, 0xF5),
        on_accent: rgb(0xFF, 0xFF, 0xFF),
        warn_fg: rgb(0x8A, 0x4B, 0x08),
        warn_bg: rgb(0xFE, 0xF4, 0xD7),
        err_fg: rgb(0xA3, 0x14, 0x3C),
        err_bg: rgb(0xFF, 0xE8, 0xEB),
        ok_fg: rgb(0x06, 0x5F, 0x46),
        ok_bg: rgb(0xDC, 0xF5, 0xE8),
        track: rgb(0xD8, 0xE1, 0xDC),
        disabled_bg: rgb(0xEF, 0xF2, 0xF0),
        focus: rgb(0x0E, 0x1A, 0x14),
        focus_ring: rgb(0x04, 0x78, 0x57),
        working: rgb(0x0F, 0x76, 0x6E),
        hc: false,
    }
}

// ------------------------------------------------------------------------------------------------
// Fonts
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum F {
    Display,
    Title,
    Body,
    BodySemi,
    Caption,
    CaptionSemi,
    Icon,
    IconSmall,
    IconLarge,
    Mono,
    Brand,
}

pub(crate) struct Fonts {
    list: Vec<(F, HFONT)>,
}

impl Fonts {
    pub(crate) unsafe fn new(dpi: u32) -> Fonts {
        let s = dpi as f32 / 96.0;
        let face = ui_font_face();
        let display_face = if face.starts_with("Segoe UI Variable") { "Segoe UI Variable Display" } else { "Segoe UI" };
        let icon_face = icon_font_face();
        let mk = |face: &str, px: f32, weight: i32| -> HFONT {
            let w = to_wide(face);
            CreateFontW(-((px * s).round() as i32), 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, w.as_ptr())
        };
        Fonts {
            list: vec![
                (F::Display, mk(display_face, 30.0, 600)),
                (F::Title, mk(display_face, 26.0, 600)),
                (F::Body, mk(face, 14.0, 400)),
                (F::BodySemi, mk(face, 14.0, 600)),
                (F::Caption, mk(face, 12.0, 400)),
                (F::CaptionSemi, mk(face, 12.0, 600)),
                (F::Icon, mk(icon_face, 16.0, 400)),
                (F::IconSmall, mk(icon_face, 12.0, 400)),
                (F::IconLarge, mk(icon_face, 24.0, 400)),
                (F::Mono, mk("Cascadia Mono", 13.0, 400)),
                (F::Brand, mk(display_face, 17.0, 600)),
            ],
        }
    }

    pub(crate) fn get(&self, f: F) -> HFONT {
        self.list.iter().find(|(k, _)| *k == f).map(|(_, h)| *h).unwrap_or(std::ptr::null_mut())
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for (_, h) in &self.list {
            unsafe {
                DeleteObject(*h as _);
            }
        }
    }
}

fn icon_font_face() -> &'static str {
    use once_cell::sync::OnceCell;
    static FACE: OnceCell<&'static str> = OnceCell::new();
    FACE.get_or_init(|| unsafe {
        use windows_sys::Win32::Graphics::Gdi::{CreateCompatibleDC, DeleteDC, GetTextFaceW, SelectObject};
        let want = "Segoe Fluent Icons"; // Windows 11; Windows 10 has Segoe MDL2 Assets (same code points).
        let n = to_wide(want);
        let f = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, n.as_ptr());
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let old = SelectObject(dc, f as _);
        let mut buf = [0u16; 64];
        GetTextFaceW(dc, buf.len() as i32, buf.as_mut_ptr());
        SelectObject(dc, old);
        DeleteObject(f as _);
        DeleteDC(dc);
        let end = buf.iter().position(|c| *c == 0).unwrap_or(0);
        if String::from_utf16_lossy(&buf[..end]).eq_ignore_ascii_case(want) {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        }
    })
}

// ------------------------------------------------------------------------------------------------
// Layout model
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
struct R {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

const fn r(x: i32, y: i32, w: i32, h: i32) -> R {
    R { x, y, w, h }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum C {
    Text,
    Text2,
    Text3,
    Accent,
    Warn,
    Err,
    Ok,
    Working,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tone {
    Info,
    Warn,
    Err,
    Ok,
}

#[derive(Clone, PartialEq, Debug)]
enum Item {
    Text { r: R, s: String, font: F, color: C, flags: u32 },
    Group { r: R },
    Card { r: R },
    Divider { x: i32, y: i32, w: i32 },
    /// Pill label. With `after`, it is placed 10 dp after that text (measured in BodySemi at paint
    /// time) instead of at `x`; badges sharing the same `after` and `y` line up one after another.
    Badge { x: i32, y: i32, s: String, tone: Tone, after: Option<String> },
    Meter { r: R, level: f32, active: bool },
    Progress { r: R, frac: Option<f32> },
    Glyph { r: R, ch: char, font: F, color: C },
    Keycap { x: i32, y: i32, s: String },
    Steps { r: R, current: usize, total: usize },
    Banner { r: R, tone: Tone, title: String, body: String },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Bg {
    Sidebar,
    Content,
    Surface,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum BtnStyle {
    Primary,
    Secondary,
    Subtle,
    Danger,
    /// Not an action: a state shown in the action column (check mark + label), e.g. the model in use.
    Status,
}

#[derive(Clone, PartialEq, Debug)]
enum Kind {
    Nav { page: usize, selected: bool },
    Toggle { on: bool },
    Choice { title: String, desc: String, selected: bool },
    Button { style: BtnStyle, label: String },
    Combo { items: Vec<String>, sel: usize },
    Edit { text: String, password: bool, multiline: bool, cue: String },
    /// "Press your shortcut" field: shows `spec` as keycaps; Space or a click starts a capture.
        Shortcut { spec: HotkeySpec, role: ShortcutRole },
        /// A window button in the frameless header: minimize, or close.
        WinBtn { close: bool },
    }

    impl Kind {
        fn tag(&self) -> u8 {
            match self {
                Kind::Nav { .. } => 0,
                Kind::Toggle { .. } => 1,
                Kind::Choice { .. } => 2,
                Kind::Button { .. } => 3,
                Kind::Combo { .. } => 4,
                Kind::Edit { .. } => 5,
                Kind::Shortcut { .. } => 6,
                Kind::WinBtn { .. } => 7,
            }
        }
    }

#[derive(Clone, PartialEq, Debug)]
struct Ctrl {
    id: i32,
    kind: Kind,
    r: R,
    bg: Bg,
    enabled: bool,
    name: String,
    tip: Option<String>,
}

struct Builder {
    items: Vec<Item>,
    ctrls: Vec<Ctrl>,
    y: i32,
    group_start: Option<(usize, i32)>,
    rows_in_group: usize,
}

impl Builder {
    fn new() -> Self {
        Builder { items: Vec::new(), ctrls: Vec::new(), y: HEADER_H + 24, group_start: None, rows_in_group: 0 }
    }

    fn text(&mut self, rr: R, s: impl Into<String>, font: F, color: C, flags: u32) {
        self.items.push(Item::Text { r: rr, s: s.into(), font, color, flags });
    }

    fn title(&mut self, title: &str, subtitle: &str) {
        self.text(r(X0, self.y, CW, 36), title, F::Title, C::Text, DT_SINGLELINE);
        self.y += 40;
        if !subtitle.is_empty() {
            self.text(r(X0, self.y, CW, 40), subtitle, F::Body, C::Text2, DT_WORDBREAK);
            self.y += 34;
        } else {
            self.y += 8;
        }
    }

    fn section(&mut self, label: &str) {
        self.y += 4;
        self.text(r(X0, self.y, CW, 18), label, F::CaptionSemi, C::Text2, DT_SINGLELINE);
        self.y += 24;
    }

    fn group(&mut self) {
        self.items.push(Item::Group { r: r(X0, self.y, CW, 0) });
        self.group_start = Some((self.items.len() - 1, self.y));
        self.rows_in_group = 0;
    }

    fn end_group(&mut self) {
        if let Some((idx, y0)) = self.group_start.take() {
            if let Item::Group { r: ref mut gr } = self.items[idx] {
                gr.h = self.y - y0;
            }
        }
        self.y += 12;
    }

    fn ctrl(&mut self, id: i32, kind: Kind, rr: R, bg: Bg, name: &str) {
        self.ctrls.push(Ctrl { id, kind, r: rr, bg, enabled: true, name: name.to_string(), tip: None });
    }

    fn last_ctrl(&mut self) -> &mut Ctrl {
        self.ctrls.last_mut().expect("ctrl")
    }

    /// A settings row inside a group: label + description on the left, a control of width
    /// `control_w` on the right (0 = no control). Text never runs underneath the control.
    fn row(&mut self, label: &str, desc: &str, desc_color: C, control_w: i32) -> (i32, i32) {
        self.row_ex(label, desc, desc_color, control_w, false)
    }

    fn row_ex(&mut self, label: &str, desc: &str, desc_color: C, control_w: i32, wrap: bool) -> (i32, i32) {
        if self.rows_in_group > 0 {
            self.items.push(Item::Divider { x: X0 + PAD, y: self.y, w: CW - 2 * PAD });
        }
        self.rows_in_group += 1;
        let text_w = if control_w > 0 { CW - 2 * PAD - control_w - 20 } else { CW - 2 * PAD };
        let h = if desc.is_empty() { ROW_H_SHORT } else if wrap { ROW_H + 16 } else { ROW_H };
        let label_y = if desc.is_empty() { self.y + 13 } else { self.y + 9 };
        self.text(r(X0 + PAD, label_y, text_w, 20), label, F::Body, C::Text, DT_SINGLELINE | DT_END_ELLIPSIS);
        if !desc.is_empty() {
            if wrap {
                self.text(r(X0 + PAD, self.y + 31, text_w, 34), desc, F::Caption, desc_color, DT_WORDBREAK | DT_END_ELLIPSIS);
            } else {
                self.text(r(X0 + PAD, self.y + 31, text_w, 18), desc, F::Caption, desc_color, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
        }
        let y0 = self.y;
        self.y += h;
        (y0, h)
    }

    fn toggle_row(&mut self, id: i32, label: &str, desc: &str, on: bool, enabled: bool) {
        // Just the switch: its position and fill show the state (the accessible name still says On or Off).
        let w = 52;
        let wrap = desc.chars().count() as i32 * 6 > CW - 2 * PAD - w - 20;
        let (y0, h) = self.row_ex(label, desc, C::Text2, w, wrap);
        self.ctrl(id, Kind::Toggle { on }, r(X0 + CW - PAD - w, y0 + (h - 32) / 2, w, 32), Bg::Surface, &format!("{label}: {}", if on { "On" } else { "Off" }));
        self.last_ctrl().enabled = enabled;
    }

    fn button_row(&mut self, id: i32, label: &str, desc: &str, button: &str, style: BtnStyle) {
        let w = button_width(button);
        let (y0, h) = self.row(label, desc, C::Text2, w);
        self.ctrl(id, Kind::Button { style, label: button.into() }, r(X0 + CW - PAD - w, y0 + (h - 32) / 2, w, 32), Bg::Surface, button);
    }

    fn combo_row(&mut self, id: i32, label: &str, desc: &str, desc_color: C, items: Vec<String>, sel: usize, w: i32) {
        let wrap = desc.chars().count() as i32 * 6 > CW - 2 * PAD - w - 20;
        let (y0, h) = self.row_ex(label, desc, desc_color, w, wrap);
        let name = format!("{label}: {}", items.get(sel).map(String::as_str).unwrap_or(""));
        self.ctrl(id, Kind::Combo { items, sel }, r(X0 + CW - PAD - w, y0 + (h - 32) / 2, w, 32), Bg::Surface, &name);
    }

    /// A shortcut row: the capture field on the right, and below the label the refusal from the last capture, the
    /// configured chord's warning, or `desc`.
    fn shortcut_row(&mut self, id: i32, label: &str, desc: &str, cfg: &AppConfig, role: ShortcutRole) {
        let w = 220;
        let spec = cfg.shortcut(role).clone();
        let refusal = SHORTCUT_REFUSAL.with(|n| n.borrow().as_ref().filter(|(i, _)| *i == id).map(|(_, m)| m.clone()));
        let (desc, color) = match (refusal, cfg.shortcut_problem(role, &spec)) {
            (Some(m), _) => (m, C::Err),
            (None, Some(p)) => (p.message, C::Warn),
            (None, None) => (desc.to_string(), C::Text2),
        };
        let wrap = desc.chars().count() as i32 * 6 > CW - 2 * PAD - w - 20;
        let (y0, h) = self.row_ex(label, &desc, color, w, wrap);
        let name = format!("{label}: {}. Press Space to change it.", spec.display_name);
        self.ctrl(id, Kind::Shortcut { spec, role }, r(X0 + CW - PAD - w, y0 + (h - 32) / 2, w, 32), Bg::Surface, &name);
    }

    fn choice(&mut self, id: i32, rr: R, title: &str, desc: &str, selected: bool) {
        self.ctrl(
            id,
            Kind::Choice { title: title.into(), desc: desc.into(), selected },
            rr,
            Bg::Content,
            &format!("{title}{}", if selected { " (selected)" } else { "" }),
        );
    }
}

fn button_width(label: &str) -> i32 {
    (label.chars().count() as i32 * 8 + 32).clamp(84, 220)
}

// ------------------------------------------------------------------------------------------------
// Window-local state (GUI thread only)
// ------------------------------------------------------------------------------------------------

struct St {
    hwnd: HWND,
    page: usize,
    step: usize,
    dpi: u32,
    fonts: Fonts,
    ctrls: Vec<(Ctrl, HWND)>,
    items: Vec<Item>,
    hot: i32,
    saved_at: Option<Instant>,
    tooltip: HWND,
    practice_text: String,
    key_draft: String,
    last_page: usize,
    /// Logical scroll offset of the content column (non-zero only when the layout is taller than the view).
    scroll: i32,
    /// Logical height of the current page's content (some pages, e.g. eight speech models, are taller than the window).
    content_h: i32,
    /// Logical client height.
    view_h: i32,
    /// Toggles sliding to a new state: (control id, start). Empty unless a switch was just flipped.
    anims: Vec<(i32, Instant)>,
}

const IDT_ANIM: usize = 9002;
const TOGGLE_MS: f32 = 150.0;

/// 0..1 progress of a toggle slide, eased out (cubic). 1.0 when no slide is running.
fn toggle_progress(st: &St, id: i32) -> f32 {
    st.anims
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, t0)| {
            let p = (t0.elapsed().as_secs_f32() * 1000.0 / TOGGLE_MS).clamp(0.0, 1.0);
            1.0 - (1.0 - p).powi(3)
        })
        .unwrap_or(1.0)
}

thread_local! {
    static ST: RefCell<Option<St>> = const { RefCell::new(None) };
    static LAST_PAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(PAGE_GENERAL) };
    /// The shortcut capture in progress: the field's control id and its state.
    static CAPTURE: RefCell<Option<(i32, ShortcutCapture)>> = const { RefCell::new(None) };
    /// Why the last captured chord was refused; shown under that row until the next capture.
    static SHORTCUT_REFUSAL: RefCell<Option<(i32, String)>> = const { RefCell::new(None) };
    /// Setup step 4: the practice box holds text (a dictation arrived).
    static PRACTICE_HEARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Setup step 4: why the last practice attempt produced nothing (0 none, 1 nothing heard, 3 failed).
    static PRACTICE_NOTE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// The step-3 "Ready" beat is already counting down to the next step.
    static ADVANCE_ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Writing page: `Some(page)` while the personal dictionary editor is open, showing that page of rows.
    static VOCAB_VIEW: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// The dictionary editor shows one blank row at the end for a new term.
    static VOCAB_DRAFT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// My style: the page of learned habits on show.
    static STYLE_PAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The confirmation card on show, if any: the window's own replacement for the stock dialog box.
    static MODAL: RefCell<Option<Modal>> = const { RefCell::new(None) };
}

/// What confirming the card does.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ModalAction {
    /// Nothing to confirm: a notice with one OK button.
    Notice,
    DeleteLearned,
    ResetAll,
}

#[derive(Clone, PartialEq, Debug)]
struct Modal {
    title: String,
    body: String,
    confirm: String,
    action: ModalAction,
}

impl Modal {
    fn destructive(&self) -> bool {
        self.action != ModalAction::Notice
    }

    /// Destructive cards get Cancel (the default button); a notice only has OK.
    fn cancel(&self) -> Option<&'static str> {
        self.destructive().then_some("Cancel")
    }
}

/// Where the card sits: centred on the design size.
fn modal_card() -> R {
    r((WIN_W - MODAL_W) / 2, (WIN_H - MODAL_H) / 2, MODAL_W, MODAL_H)
}

/// Shows the card and hands the keyboard to its default button (Cancel where the action destroys something).
fn ask(title: &str, body: &str, confirm: &str, action: ModalAction) {
    MODAL.with(|m| {
        *m.borrow_mut() = Some(Modal { title: title.into(), body: body.into(), confirm: confirm.into(), action })
    });
    rebuild(true);
}

fn ask_delete_learned() {
    ask(
        "Delete everything KeyReed learned?",
        "Every learned word and habit, the suggestions and the excluded apps go. Your switches and other settings stay.",
        "Delete",
        ModalAction::DeleteLearned,
    );
}

/// Drops the card and puts focus back on something reachable.
fn close_modal() {
    MODAL.with(|m| *m.borrow_mut() = None);
    rebuild(false);
    let page = ST.with(|s| s.borrow().as_ref().map(|st| st.page).unwrap_or(PAGE_GENERAL));
    if unsafe { GetFocus() }.is_null() {
        focus_ctrl(ID_NAV + page as i32);
    }
}

/// Controls that never scroll with the page: the navigation and the header's window buttons.
fn fixed_in_place(k: &Kind) -> bool {
    matches!(k, Kind::Nav { .. } | Kind::WinBtn { .. })
}

/// Window DPI, honouring the gallery's scale override.
fn window_dpi(h: HWND) -> u32 {
    let o = crate::ui::win_gui::dpi_override();
    if o != 0 {
        o
    } else {
        unsafe { GetDpiForWindow(h).max(96) }
    }
}

fn sc(v: i32, dpi: u32) -> i32 {
    ((v as f32) * dpi as f32 / 96.0).round() as i32
}

fn snapshot() -> Option<SharedUiModel> {
    globals().map(|g| g.model.lock().clone())
}

/// Applies one change on top of the latest config and saves it immediately.
fn commit(f: impl FnOnce(&mut AppConfig)) {
    let Some(g) = globals() else { return };
    let cfg = {
        let mut m = g.model.lock();
        let mut c = m.config.clone();
        f(&mut c);
        m.config = c.clone();
        c
    };
    send(UiAction::SaveUpdatedConfig { new_cfg: Box::new(cfg), new_plaintext_api_key: None });
    flash_saved();
}

/// Applies one change to the style store on top of the latest copy; the coordinator saves it.
fn commit_style(f: impl FnOnce(&mut vi_core::style::StyleStore)) {
    let Some(g) = globals() else { return };
    let style = {
        let mut m = g.model.lock();
        let mut s = m.style.clone();
        f(&mut s);
        m.style = s.clone();
        s
    };
    send(UiAction::SaveStyleStore(Box::new(style)));
    flash_saved();
}

fn flash_saved() {
    ST.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.saved_at = Some(Instant::now());
            unsafe {
                SetTimer(st.hwnd, IDT_SAVED, 1800, None);
            }
        }
    });
    refresh();
}

// ------------------------------------------------------------------------------------------------
// Public entry points (called from ui::win_gui on the GUI thread)
// ------------------------------------------------------------------------------------------------

pub fn hwnd() -> HWND {
    ST.with(|s| s.borrow().as_ref().map(|st| st.hwnd).unwrap_or(std::ptr::null_mut()))
}

pub unsafe fn register_class() {
    let hinst = GetModuleHandleW(std::ptr::null());
    let cls = to_wide(SETTINGS_WINDOW_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: std::ptr::null_mut(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: cls.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    RegisterClassExW(&wc);
    let icc = windows_sys::Win32::UI::Controls::INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<windows_sys::Win32::UI::Controls::INITCOMMONCONTROLSEX>() as u32,
        dwICC: 0x0000_00FF, // ICC_WIN95_CLASSES (includes tooltips)
    };
    windows_sys::Win32::UI::Controls::InitCommonControlsEx(&icc);
}

/// Opens (or focuses) Settings. `None` opens setup on first run, otherwise the last page.
pub fn open(page: Option<usize>) {
    unsafe {
        let existing = hwnd();
        if !existing.is_null() && IsWindow(existing) != 0 {
            if let Some(p) = page {
                navigate(p);
            }
            if IsIconic(existing) != 0 {
                ShowWindow(existing, SW_RESTORE);
            }
            let _ = crate::typing::force_foreground_window(existing as isize);
            return;
        }
        let first_run = snapshot().map(|m| !m.config.first_run_completed).unwrap_or(false);
        let start_page = page.unwrap_or(if first_run { PAGE_START } else { LAST_PAGE.with(|c| c.get()) });
        PENDING_PAGE.with(|c| c.set(start_page));

        // Refresh the device list every time Settings opens (devices may have been plugged in).
        send(UiAction::RefreshMicrophones);

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;

        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = to_wide(SETTINGS_WINDOW_CLASS);
        let title = to_wide(&format!("{PRODUCT_NAME} Settings"));
        // Frameless: the window is its own frame (WM_NCCALCSIZE / WM_NCHITTEST below); the edges still resize.
        let style = WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            cls.as_ptr(),
            title.as_ptr(),
            style,
            wa.left + 40,
            wa.top + 40,
            WIN_W,
            WIN_H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        // The client area is the whole window, so the design size is the window size. Centre it on the monitor.
        let dpi = window_dpi(hwnd);
        let (w, h) = (sc(WIN_W, dpi), sc(WIN_H, dpi).min(wa.bottom - wa.top));
        let x = wa.left + ((wa.right - wa.left) - w) / 2;
        let y = (wa.top + ((wa.bottom - wa.top) - h) / 2).max(wa.top);
        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = crate::typing::force_foreground_window(hwnd as isize);
    }
}

thread_local! {
    static PENDING_PAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(PAGE_GENERAL) };
}

/// Called after every model sync. Only repaints / reconciles what actually changed.
pub fn on_model_changed() {
    refresh();
}

/// Keyboard navigation (Tab, Shift+Tab, Space, Enter, Esc) for the Settings window.
pub fn pre_translate(msg: &MSG) -> bool {
    let h = hwnd();
    if h.is_null() {
        return false;
    }
    unsafe {
        if IsWindow(h) == 0 {
            return false;
        }
        if msg.hwnd == h || IsChild(h, msg.hwnd) != 0 {
            let handled = IsDialogMessageW(h, msg) != 0;
            if handled {
                // Tab can land on a control below the fold: bring it into view.
                reveal_focused();
            }
            return handled;
        }
    }
    false
}

/// Test hook for the UI gallery: navigate to a page / setup step.
pub fn show_page_for_test(page: usize, step: usize) {
    ST.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.step = step;
        }
    });
    navigate(page);
}

/// Gallery hook: (scroll offset, content height, visible height) of the page on show, in design units.
pub fn scroll_state_for_test() -> (i32, i32, i32) {
    ST.with(|s| s.borrow().as_ref().map(|st| (st.scroll, st.content_h, st.view_h)).unwrap_or((0, 0, 0)))
}

/// Gallery hook: one wheel notch sent to control `id`, as Windows sends it when the pointer is over that control.
pub fn wheel_over_control_for_test(id: i32, delta: i32) -> bool {
    unsafe {
        let h = GetDlgItem(hwnd(), id);
        !h.is_null() && SendMessageW(h, WM_MOUSEWHEEL, (((delta as i16 as u16) as usize) << 16) as WPARAM, 0) == 0
    }
}

/// Gallery hooks: the Delete-all card (the one the My style page opens), and dismissing it again.
pub fn confirm_delete_for_test() {
    ask_delete_learned();
}

/// Test hook (gallery perf): flips the Notifications switch on the General page the way a click does.
pub fn flip_toggle_for_test() {
    on_command(G_NOTIFY, 0);
}

pub fn dismiss_modal_for_test() {
    close_modal();
}

/// Gallery hook: scroll the content column of the page on show (logical offset, clamped like the wheel).
pub fn scroll_to_for_test(offset: i32) {
    let cur = ST.with(|s| s.borrow().as_ref().map(|st| st.scroll).unwrap_or(0));
    scroll_by(offset - cur);
}

/// Gallery hook for the Quick Fix shortcut field on the Dictation page: 0 = recording, nothing held yet;
/// 1 = recording with Ctrl and Win held; 2 = Ctrl + C was pressed and refused; anything else = back to normal.
pub fn shortcut_capture_for_test(step: u8) {
    match step {
        0 => start_capture(D_QF_KEY),
        1 => {
            start_capture(D_QF_KEY);
            CAPTURE.with(|c| {
                if let Some((_, cap)) = c.borrow_mut().as_mut() {
                    cap.key(0xA2, true, false);
                    cap.key(0x5B, true, false);
                }
            });
            unsafe { InvalidateRect(GetDlgItem(hwnd(), D_QF_KEY), std::ptr::null(), 0) };
        }
        2 => {
            start_capture(D_QF_KEY);
            end_capture(D_QF_KEY, Some(vi_core::shortcut::chord(true, false, false, false, 0x43)));
        }
        _ => {
            end_capture(D_QF_KEY, None);
            SHORTCUT_REFUSAL.with(|n| *n.borrow_mut() = None);
            refresh();
        }
    }
}

fn navigate(page: usize) {
    let mut changed = false;
    let cur = ST.with(|s| s.borrow().as_ref().map(|st| st.page));
    if cur.is_some() && cur != Some(page) {
        harvest_and_commit();
    }
    ST.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            if st.page != page {
                st.page = page;
                st.scroll = 0;
                VOCAB_VIEW.with(|v| v.set(None));
                VOCAB_DRAFT.with(|v| v.set(false));
                changed = true;
                if page != PAGE_START {
                    st.last_page = page;
                    LAST_PAGE.with(|c| c.set(page));
                }
            }
        }
    });
    if changed {
        rebuild(true);
    } else {
        refresh();
    }
}

// ------------------------------------------------------------------------------------------------
// Page builders
// ------------------------------------------------------------------------------------------------

fn build(page: usize, step: usize, m: &SharedUiModel, saved: bool) -> (Vec<Item>, Vec<Ctrl>) {
    let mut b = Builder::new();
    // The frameless header's window buttons: real controls, so hover, press and focus draw as everywhere else.
    b.ctrl(HEADER_MIN, Kind::WinBtn { close: false }, r(WIN_W - 92, 10, 40, 32), Bg::Content, "Minimize");
    b.ctrl(HEADER_CLOSE, Kind::WinBtn { close: true }, r(WIN_W - 48, 10, 40, 32), Bg::Content, "Close");
    // Navigation
    for (i, (label, _)) in NAV.iter().enumerate() {
        let y = NAV_Y + i as i32 * 38;
        b.ctrl(ID_NAV + i as i32, Kind::Nav { page: i, selected: i == page }, r(12, y, SIDEBAR_W - 24, 36), Bg::Sidebar, label);
    }
    match page {
        PAGE_START => page_start(&mut b, step, m),
        PAGE_GENERAL => page_general(&mut b, m),
        PAGE_DICTATION => page_dictation(&mut b, m),
        PAGE_WRITING => page_writing(&mut b, m),
        PAGE_MYSTYLE => page_mystyle(&mut b, m),
        PAGE_OUTPUT => page_output(&mut b, m),
        PAGE_AUDIO => page_audio(&mut b, m),
        PAGE_MODELS => page_models(&mut b, m),
        PAGE_PROCESSING => page_processing(&mut b, m),
        PAGE_PRIVACY => page_privacy(&mut b, m),
        _ => page_about(&mut b, m),
    }
    if page != PAGE_START {
        // Only said when something was saved; changes always apply at once, so there is nothing to remind the user of.
        if saved {
            b.text(r(X0, WIN_H - 34, CW, 18), "Saved", F::Caption, C::Accent, DT_SINGLELINE | DT_RIGHT);
        }
    }
    // The confirmation card (see `ask`): Cancel first, so Enter on the default button never deletes anything.
    if let Some(md) = MODAL.with(|m| m.borrow().clone()) {
        let card = modal_card();
        let (bh, by) = (32, card.y + card.h - 24 - 32);
        let cw = button_width(&md.confirm);
        let confirm_x = card.x + card.w - 24 - cw;
        if let Some(label) = md.cancel() {
            let nw = button_width(label);
            b.ctrl(MODAL_CANCEL, Kind::Button { style: BtnStyle::Secondary, label: label.into() }, r(confirm_x - 8 - nw, by, nw, bh), Bg::Surface, label);
        }
        let style = if md.destructive() { BtnStyle::Danger } else { BtnStyle::Primary };
        b.ctrl(MODAL_CONFIRM, Kind::Button { style, label: md.confirm.clone() }, r(confirm_x, by, cw, bh), Bg::Surface, &md.confirm);
    }
    (b.items, b.ctrls)
}

fn hotkey_items(m: &SharedUiModel) -> (Vec<String>, usize, String, C) {
    let presets = hotkey_presets();
    let mut sel = 0;
    let mut items = Vec::new();
    let mut note = String::new();
    for (i, (spec, n)) in presets.iter().enumerate() {
        items.push(spec.display_name.clone());
        if spec.vk == m.config.ptt_hotkey.vk
            && spec.ctrl == m.config.ptt_hotkey.ctrl
            && spec.alt == m.config.ptt_hotkey.alt
            && spec.shift == m.config.ptt_hotkey.shift
            && spec.win == m.config.ptt_hotkey.win
        {
            sel = i;
            note = (*n).to_string();
        }
    }
    if let Some(c) = hotkey_conflict(&m.config, &m.config.ptt_hotkey) {
        return (items, sel, c, C::Warn);
    }
    (items, sel, note, C::Text2)
}

fn mic_items(m: &SharedUiModel) -> (Vec<String>, usize) {
    let mut items = vec!["Windows default microphone".to_string()];
    let mut sel = 0;
    for (i, name) in m.available_microphones.iter().enumerate() {
        items.push(name.clone());
        if let Some(s) = &m.config.selected_microphone {
            if s.eq_ignore_ascii_case(name) {
                sel = i + 1;
            }
        }
    }
    if sel == 0 {
        if let Some(s) = &m.config.selected_microphone {
            // Preferred device is unplugged: keep it visible so the choice isn't silently lost.
            items.push(format!("{s} (not connected)"));
            sel = items.len() - 1;
        }
    }
    (items, sel)
}

fn activation_choices(b: &mut Builder, m: &SharedUiModel, id_hold: i32, id_toggle: i32) {
    let w = (CW - 12) / 2;
    let hold = m.config.activation_mode == ActivationMode::HoldToTalk;
    b.choice(id_hold, r(X0, b.y, w, 78), "Hold to talk", ActivationMode::HoldToTalk.description(), hold);
    b.choice(id_toggle, r(X0 + w + 12, b.y, w, 78), "Press to start and stop", ActivationMode::ToggleToTalk.description(), !hold);
    b.y += 90;
}

fn mic_test_block(b: &mut Builder, m: &SharedUiModel, test_id: i32) {
    b.group();
    let y = b.y;
    let t = &m.mic_test;
    b.text(r(X0 + PAD, y + 14, CW - 220, 20), "Say a few words. KeyReed listens for 3 seconds.", F::Body, C::Text, DT_SINGLELINE);
    b.text(r(X0 + PAD, y + 36, CW - 220, 18), "The bar shows the real input level from your microphone.", F::Caption, C::Text2, DT_SINGLELINE);
    let label = if t.running { "Listening…" } else if t.verdict.is_some() || t.error.is_some() { "Test again" } else { "Test microphone" };
    b.ctrl(test_id, Kind::Button { style: BtnStyle::Primary, label: label.into() }, r(X0 + CW - PAD - 156, y + 16, 156, 32), Bg::Surface, label);
    b.last_ctrl().enabled = !t.running;
    b.last_ctrl().tip = Some("Records for 3 seconds. Nothing is saved.".into());
    b.items.push(Item::Meter { r: r(X0 + PAD, y + 66, CW - 2 * PAD, 10), level: t.level, active: t.running });
    b.text(r(X0 + PAD, y + 80, 120, 16), "Quiet", F::Caption, C::Text3, DT_SINGLELINE);
    b.text(r(X0 + PAD + (CW - 2 * PAD) / 2 - 60, y + 80, 120, 16), "Good", F::Caption, C::Text3, DT_SINGLELINE | DT_CENTER);
    b.text(r(X0 + CW - PAD - 120, y + 80, 120, 16), "Too loud", F::Caption, C::Text3, DT_SINGLELINE | DT_RIGHT);
    let mut h = 106;
    // Windows can hand desktop apps silence when microphone privacy is off; one click opens that page.
    let privacy_button = |b: &mut Builder, y: i32| {
        let label = "Open privacy settings";
        b.ctrl(MIC_PRIVACY, Kind::Button { style: BtnStyle::Secondary, label: label.into() }, r(X0 + PAD, y, button_width(label), 32), Bg::Surface, "Open Windows microphone privacy settings");
        b.last_ctrl().tip = Some("Turn on \u{201C}Let desktop apps access your microphone\u{201D} in Windows Settings.".into());
    };
    if let Some(e) = &t.error {
        let fe = friendly_error(e);
        let body = if fe.fix == ErrorFix::Microphone { "Check it's connected, and turn on \u{201C}Let desktop apps access your microphone\u{201D}." } else { fe.message };
        b.items.push(Item::Banner { r: r(X0 + PAD, y + 104, CW - 2 * PAD, 52), tone: Tone::Err, title: "Couldn't open the microphone".into(), body: body.into() });
        privacy_button(b, y + 164);
        h = 208;
    } else if let Some(v) = t.verdict {
        let tone = match v {
            MicVerdict::Good => Tone::Ok,
            MicVerdict::Low | MicVerdict::TooLoud => Tone::Warn,
            MicVerdict::Silent => Tone::Err,
        };
        b.items.push(Item::Banner { r: r(X0 + PAD, y + 104, CW - 2 * PAD, 52), tone, title: v.headline().into(), body: v.advice().into() });
        let peak_db = if t.peak_rms > 0.0 { 20.0 * t.peak_rms.log10() } else { -90.0 };
        let detail = format!(
            "Device: {}{} · loudest moment {:.0} dBFS",
            if t.device.is_empty() { "unknown" } else { &t.device },
            if t.used_fallback { " (Windows default, preferred device unavailable)" } else { "" },
            peak_db
        );
        b.text(r(X0 + PAD, y + 162, CW - 2 * PAD, 16), detail, F::Caption, C::Text3, DT_SINGLELINE | DT_END_ELLIPSIS);
        h = 188;
        if v == MicVerdict::Silent {
            privacy_button(b, y + 186);
            h = 230;
        }
    }
    b.y = y + h;
    b.end_group();
}

/// The quiet line at the foot of the setup steps that handle the microphone and the model.
fn privacy_line(b: &mut Builder) {
    b.items.push(Item::Glyph { r: r(X0, WIN_H - 49, 16, 18), ch: '\u{E72E}', font: F::IconSmall, color: C::Text3 });
    b.text(r(X0 + 22, WIN_H - 49, CW - 282, 18), "Speech is processed on this PC.", F::Caption, C::Text3, DT_SINGLELINE | DT_END_ELLIPSIS);
}

fn page_start(b: &mut Builder, step: usize, m: &SharedUiModel) {
    let hk = m.config.ptt_hotkey.display_name.clone();
    b.text(r(X0, b.y, CW, 18), format!("Setup · step {} of 4", step + 1), F::CaptionSemi, C::Text2, DT_SINGLELINE);
    b.items.push(Item::Steps { r: r(X0, b.y + 24, CW, 4), current: step, total: 4 });
    b.y += 44;
    let downloading = m.download.as_ref().is_some_and(|d| d.is_active());
    let toggle = m.config.activation_mode == ActivationMode::ToggleToTalk;
    match step {
        0 => {
            let how = if toggle { format!("Press {hk}, speak, press again.") } else { format!("Hold {hk}, speak, let go.") };
            b.title("Welcome to KeyReed", &format!("{how} KeyReed types it wherever your cursor is."));
            let w = (CW - 24) / 3;
            let y = b.y;
            let cards: [(&str, &str); 3] = [
                ("Hold the shortcut", "Wherever your cursor is."),
                ("Speak naturally", "Pauses and \u{201C}um\u{201D}s are fine."),
                ("Let go", "Your words appear in the app."),
            ];
            for (i, (t, d)) in cards.iter().enumerate() {
                let x = X0 + i as i32 * (w + 12);
                b.items.push(Item::Card { r: r(x, y, w, 128) });
                b.items.push(Item::Badge { x: x + 14, y: y + 14, s: format!("{}", i + 1), tone: Tone::Info, after: None });
                match i {
                    0 => b.items.push(Item::Keycap { x: x + 14, y: y + 46, s: hk.clone() }),
                    1 => b.items.push(Item::Glyph { r: r(x + 14, y + 44, 32, 32), ch: '\u{E720}', font: F::IconLarge, color: C::Accent }),
                    _ => b.items.push(Item::Glyph { r: r(x + 14, y + 44, 32, 32), ch: '\u{E8D2}', font: F::IconLarge, color: C::Accent }),
                }
                b.text(r(x + 14, y + 84, w - 28, 20), *t, F::BodySemi, C::Text, DT_SINGLELINE);
                b.text(r(x + 14, y + 104, w - 28, 18), *d, F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
            b.y = y + 144;
            // Plan F3: one sentence saying exactly what learning reads, and the switch, on by default.
            b.group();
            b.toggle_row(
                Y_LEARN,
                "Learn my words from my corrections",
                "KeyReed learns your words from the corrections you make to its text. It checks only the text box it just typed into, on this PC, and keeps only the corrected words.",
                m.config.learn_from_edits,
                true,
            );
            b.end_group();
            b.section("Your shortcut");
            b.group();
            let (items, sel, note, nc) = hotkey_items(m);
            b.combo_row(S_HOTKEY, "Push-to-talk shortcut", &note, nc, items, sel, 220);
            b.end_group();
            activation_choices(b, m, S_HOLD, S_TOGGLE);
            // Next starts the one-time download, so say so (and how big it is) before the user presses it.
            if !m.active_model_ready() && !downloading {
                let mb = ASR_UI_CATALOG[0].download_mb;
                b.text(r(X0, b.y, CW, 18), format!("Next downloads the speech engine to this PC, once ({mb} MB)."), F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
            b.ctrl(S_SKIP, Kind::Button { style: BtnStyle::Subtle, label: "Skip setup".into() }, r(X0, WIN_H - 56, 110, 32), Bg::Content, "Skip setup");
            b.ctrl(S_NEXT, Kind::Button { style: BtnStyle::Primary, label: "Next".into() }, r(X0 + CW - 120, WIN_H - 56, 120, 32), Bg::Content, "Next");
        }
        1 => {
            let lead = if toggle { format!("KeyReed listens only between your two presses of {hk}.") } else { format!("KeyReed listens only while you hold {hk}.") };
            b.title("Let KeyReed hear you", &lead);
            b.group();
            let (items, sel) = mic_items(m);
            b.combo_row(S_MIC, "Microphone", "If it's unplugged, KeyReed uses the Windows default", C::Text2, items, sel, 300);
            b.end_group();
            mic_test_block(b, m, S_TEST);
            privacy_line(b);
            nav_buttons(b, "Next");
        }
        2 => {
            let ready = m.active_model_ready();
            let id = if ready { m.config.local_asr_model.clone() } else { ASR_UI_CATALOG[0].id.to_string() };
            let mb = asr_ui_info(&id).map_or(ASR_UI_CATALOG[0].download_mb, |i| i.download_mb);
            let lead = if ready { "The speech engine is ready.".to_string() } else { format!("Downloading the speech engine, {mb} MB, once.") };
            b.title("Setting up KeyReed", &lead);
            b.group();
            model_row(b, m, &id, S_DL, false);
            b.end_group();
            b.ctrl(S_ALLMODELS, Kind::Button { style: BtnStyle::Subtle, label: "Compare all six models".into() }, r(X0, b.y, 210, 32), Bg::Content, "Compare all six models");
            b.y += 48;
            b.items.push(Item::Banner {
                r: r(X0, b.y, CW, 56),
                tone: Tone::Info,
                title: "Private by default".into(),
                body: "Speech stays on this PC. Cloud processing is off unless you add your own key under Processing.".into(),
            });
            b.y += 68;
            privacy_line(b);
            nav_buttons(b, "Next");
        }
        _ => {
            let heard = PRACTICE_HEARD.with(|c| c.get());
            if heard {
                b.title("That's KeyReed", "It types wherever your cursor is, in any app.");
            } else {
                let lead = if toggle { format!("Press {hk}, say anything, then press it again.") } else { format!("Hold {hk} and say anything, then let go.") };
                b.title("Your turn", &lead);
            }
            if !m.active_model_ready() {
                b.items.push(Item::Banner {
                    r: r(X0, b.y, CW, 56),
                    tone: Tone::Warn,
                    title: "Speech model needed".into(),
                    body: "Go back one step and download the speech engine first.".into(),
                });
                b.y += 68;
            }
            b.text(r(X0, b.y, CW, 18), "Practice area · text appears here, nothing is sent anywhere", F::CaptionSemi, C::Text2, DT_SINGLELINE);
            b.y += 22;
            b.ctrl(S_PRACTICE, Kind::Edit { text: String::new(), password: false, multiline: true, cue: String::new() }, r(X0, b.y, CW, 104), Bg::Content, "Practice area");
            // One quiet line under the box says what is happening or what to try next.
            let (hint, color) = match (&m.state, PRACTICE_NOTE.with(|c| c.get())) {
                _ if heard => (format!("It typed what you said. Use {hk} again to try another."), C::Ok),
                (EngineState::Listening, _) => ("Listening…".to_string(), C::Text2),
                (EngineState::Processing | EngineState::Correcting, _) => ("Typing what you said…".to_string(), C::Working),
                (_, 1) => ("KeyReed didn't hear any words. Speak while it's listening, then try again.".to_string(), C::Warn),
                (_, 3) => ("That didn't work. Try again.".to_string(), C::Warn),
                _ => (String::new(), C::Text2),
            };
            if !hint.is_empty() {
                b.text(r(X0, b.y + 108, CW, 18), hint, F::Caption, color, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
            b.y += 138;
            b.group();
            let mode = m.config.default_writing_mode;
            b.row(&format!("Writing mode: {}", mode.label()), mode.description(), C::Text2, 0);
            let om = m.config.output_mode;
            let short = match om {
                OutputMode::DirectTyping => "Types straight into the app you're using",
                OutputMode::Clipboard => "Copies the text; you paste it with Ctrl+V",
                OutputMode::AskBeforeInserting => "Shows the text first and asks before inserting",
            };
            b.row(&format!("Output: {}", om.title()), short, C::Text2, 0);
            b.end_group();
            b.text(r(X0, b.y, CW, 18), "You can change these any time under Writing and Output.", F::Caption, C::Text3, DT_SINGLELINE);
            nav_buttons(b, "Start using KeyReed");
        }
    }
}

fn nav_buttons(b: &mut Builder, next: &str) {
    let nw = button_width(next).max(120);
    b.ctrl(S_BACK, Kind::Button { style: BtnStyle::Secondary, label: "Back".into() }, r(X0 + CW - nw - 128, WIN_H - 56, 120, 32), Bg::Content, "Back");
    b.ctrl(S_NEXT, Kind::Button { style: BtnStyle::Primary, label: next.into() }, r(X0 + CW - nw, WIN_H - 56, nw, 32), Bg::Content, next);
}

fn page_general(b: &mut Builder, m: &SharedUiModel) {
    let c = &m.config;
    b.title("General", "How KeyReed starts and stays out of your way.");
    b.section("Startup");
    b.group();
    b.toggle_row(G_ENABLED, "KeyReed is on", "When off, the shortcut is ignored and the microphone stays closed.", c.app_enabled, true);
    b.toggle_row(G_AUTOSTART, "Start with Windows", "Opens quietly in the system tray when you sign in.", c.autostart_on_login, true);
    b.end_group();
    b.section("Floating listener");
    b.group();
    b.toggle_row(G_PILL, "Show the floating listener", "A small KeyReed button that shows when it's listening.", c.show_floating_mic, true);
    let items: Vec<String> = vi_core::PillClickAction::ALL.iter().map(|a| a.label().to_string()).collect();
    let sel = vi_core::PillClickAction::ALL.iter().position(|a| *a == c.pill_click_action).unwrap_or(0);
    let desc = match c.pill_click_action {
        vi_core::PillClickAction::OpenComposer => "Write or rewrite text. Start dictation with your shortcut or from the right-click menu.",
        vi_core::PillClickAction::Dictate => "Click once to start and again to finish.",
        vi_core::PillClickAction::Nothing => "Use the right-click menu or your shortcut to dictate.",
    };
    b.combo_row(G_PILL_CLICK, "When you click the listener", desc, C::Text2, items, sel, 220);
    b.button_row(G_PILL_RESET, "Listener position", "Drag it anywhere. Reset puts it back above the taskbar.", "Reset position", BtnStyle::Secondary);
    b.end_group();
    b.section("Feedback");
    b.group();
    b.toggle_row(G_SOUNDS, "Sounds", "A soft tone when recording starts and when text is ready.", c.audio_cues_enabled, true);
    b.toggle_row(G_NOTIFY, "Notifications", "Tell me when text is copied instead of typed.", c.notify_on_ready, true);
    b.end_group();
}

fn page_dictation(b: &mut Builder, m: &SharedUiModel) {
    let c = &m.config;
    b.title("Dictation", "Choose how you start and stop talking.");
    b.section("Shortcut");
    b.group();
    let note = hotkey_presets()
        .into_iter()
        .find(|(p, _)| vi_core::shortcut::same_chord(p, &c.ptt_hotkey))
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| "Your own shortcut. Click the field and press new keys to change it.".into());
    b.shortcut_row(D_HOTKEY, "Push-to-talk shortcut", &note, c, ShortcutRole::Dictation);
    b.shortcut_row(D_QF_KEY, "Quick Fix shortcut", "Fixes the text you have selected, in any app.", c, ShortcutRole::QuickFix);
    b.shortcut_row(D_VOICE_KEY, "Voice command shortcut", "Tap it for the Composer on the text you have selected, or hold it and say what to do with it.", c, ShortcutRole::VoiceCommand);
    b.shortcut_row(D_COMPOSER_KEY, "Composer shortcut", "Opens the bar to write, or to rewrite the text you have selected.", c, ShortcutRole::Composer);
    b.toggle_row(D_QF_REVIEW, "Review every Quick Fix change", "Off by default: a fix KeyReed is sure about replaces the text straight away, with Undo on the listener. Turn it on to see every change first; formatting and writing-model fixes are always shown.", c.quick_fix_always_review, true);
    b.toggle_row(D_PTT, "Listen for the shortcuts", "When off, start dictation from the floating listener instead (its right-click menu, or a click if you set that in General).", c.ptt_enabled, true);
    b.end_group();
    b.section("Mouse");
    b.group();
    for (i, (label, desc, action)) in [
        ("Middle button", "Pressing the scroll wheel. Left and right clicks are never used.", c.mouse_middle_action),
        ("Back button", "The rear side button on most mice.", c.mouse_back_action),
        ("Forward button", "The front side button on most mice.", c.mouse_forward_action),
    ]
    .into_iter()
    .enumerate()
    {
        let items: Vec<String> = MouseAction::ALL.iter().map(|a| a.label().to_string()).collect();
        let sel = MouseAction::ALL.iter().position(|a| *a == action).unwrap_or(0);
        b.combo_row(D_MOUSE + i as i32, label, desc, C::Text2, items, sel, 220);
    }
    b.end_group();
    b.section("How the shortcut works");
    activation_choices(b, m, D_HOLD, D_TOGGLE);
    b.section("Other shortcuts");
    b.group();
    for (i, (key, what)) in [
        (c.cancel_hotkey.display_name.clone(), "Cancel the current recording. Nothing is typed."),
        (c.mode_cycle_hotkey.display_name.clone(), "Switch to the next writing mode."),
    ]
    .iter()
    .enumerate()
    {
        if i > 0 {
            b.items.push(Item::Divider { x: X0 + PAD, y: b.y, w: CW - 2 * PAD });
        }
        b.items.push(Item::Keycap { x: X0 + PAD, y: b.y + 9, s: key.clone() });
        b.text(r(X0 + PAD + 190, b.y + 13, CW - 230, 20), *what, F::Body, C::Text, DT_SINGLELINE);
        b.y += ROW_H_SHORT;
    }
    b.end_group();
}

fn page_writing(b: &mut Builder, m: &SharedUiModel) {
    if let Some(page) = VOCAB_VIEW.with(|v| v.get()) {
        return page_vocab(b, m, page);
    }
    let c = &m.config;
    b.title("Writing", &format!("Pick how KeyReed turns speech into text. {} switches modes any time.", c.mode_cycle_hotkey.display_name));
    let w = (CW - 10) / 2;
    let y0 = b.y;
    for (i, mode) in WritingMode::ALL.iter().enumerate() {
        let x = X0 + (i as i32 % 2) * (w + 10);
        let y = y0 + (i as i32 / 2) * 82;
        b.choice(W_MODE + i as i32, r(x, y, w, 74), mode.label(), mode.description(), *mode == c.default_writing_mode);
    }
    b.y = y0 + 3 * 82 + 4;
    let mode = c.default_writing_mode;
    let (said, wrote) = mode.example();
    b.group();
    let y = b.y;
    b.text(r(X0 + PAD, y + 12, 200, 16), "YOU SAY", F::CaptionSemi, C::Text3, DT_SINGLELINE);
    b.text(r(X0 + PAD, y + 30, CW - 2 * PAD, 20), format!("\u{201C}{said}\u{201D}"), F::Body, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
    b.text(r(X0 + PAD, y + 56, 300, 16), format!("KEYREED WRITES · {}", mode.short_label().to_uppercase()), F::CaptionSemi, C::Accent, DT_SINGLELINE);
    let mono = matches!(mode, WritingMode::TerminalCode);
    b.text(r(X0 + PAD, y + 74, CW - 2 * PAD, 20), wrote, if mono { F::Mono } else { F::Body }, C::Text, DT_SINGLELINE | DT_END_ELLIPSIS);
    b.y = y + 104;
    b.end_group();
    b.group();
    b.toggle_row(W_AUTOCTX, "Adapt to the app I'm using", "Professional in email, Casual in chat, Code in terminals.", c.auto_context_mode, true);
    b.ctrls.last_mut().unwrap().tip = Some("Email → Professional, chat → Casual, code editors → Technical, terminals → Terminal / Code. Other apps use your default mode.".into());
    let terms = dict_entries(&c.dictionary).len();
    let desc = match terms {
        0 => "Names and terms to spell your way".to_string(),
        1 => "1 term".to_string(),
        n => format!("{n} terms"),
    };
    b.button_row(W_VOCAB, "Personal dictionary", &desc, "Edit", BtnStyle::Secondary);
    b.last_ctrl().name = "Edit personal dictionary".into();
    b.end_group();
}

/// One row of the personal dictionary editor: a term and the spoken forms that should become it.
#[derive(Clone, Debug, PartialEq)]
struct DictEntry {
    term: String,
    sounds: Vec<String>,
}

fn spoken_for(d: &vi_core::PersonalDictionary, term: &str) -> Vec<String> {
    d.replacements.iter().filter(|r| r.written.eq_ignore_ascii_case(term)).map(|r| r.spoken.clone()).collect()
}

/// Vocabulary terms in order, each with the replacement rules that write it. A rule whose written form is not a
/// vocabulary term still gets its own row, so nothing stored by another route is hidden or lost.
fn dict_entries(d: &vi_core::PersonalDictionary) -> Vec<DictEntry> {
    let mut v: Vec<DictEntry> = d.vocabulary.iter().map(|t| DictEntry { term: t.clone(), sounds: spoken_for(d, t) }).collect();
    for r in &d.replacements {
        if !v.iter().any(|e| e.term.eq_ignore_ascii_case(&r.written)) {
            v.push(DictEntry { term: r.written.clone(), sounds: spoken_for(d, &r.written) });
        }
    }
    v
}

fn same_spellings(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// The dictionary is on whenever it has something to apply (it shipped disabled, so entries used to be ignored).
fn dict_sync_enabled(d: &mut vi_core::PersonalDictionary) {
    d.enabled = !(d.vocabulary.is_empty() && d.replacements.is_empty());
}

/// Applies the edited texts of one row. `index` is the row's place in `dict_entries`, `None` for the blank row that
/// adds a term. An empty term never deletes anything (use Remove), a duplicate term is refused, nothing is truncated.
/// Returns true when the dictionary changed.
fn apply_dict_row(d: &mut vi_core::PersonalDictionary, index: Option<usize>, term: &str, sounds: &str) -> bool {
    let (before_v, before_r, before_e) = (d.vocabulary.clone(), d.replacements.clone(), d.enabled);
    let term = term.trim();
    let spoken = parse_vocab(sounds);
    let entries = dict_entries(d);
    match index.and_then(|i| entries.get(i)).cloned() {
        Some(old) => {
            let taken = entries.iter().enumerate().any(|(j, e)| Some(j) != index && e.term.eq_ignore_ascii_case(term));
            let new_term = if term.is_empty() || taken { old.term.clone() } else { term.to_string() };
            if new_term != old.term {
                for v in d.vocabulary.iter_mut().filter(|v| v.eq_ignore_ascii_case(&old.term)) {
                    *v = new_term.clone();
                }
                for r in d.replacements.iter_mut().filter(|r| r.written.eq_ignore_ascii_case(&old.term)) {
                    r.written = new_term.clone();
                }
            }
            if !same_spellings(&spoken, &old.sounds) {
                let kept: Vec<_> = d.replacements.iter().filter(|r| r.written.eq_ignore_ascii_case(&new_term)).cloned().collect();
                d.replacements.retain(|r| !r.written.eq_ignore_ascii_case(&new_term));
                for sp in spoken {
                    match kept.iter().find(|r| r.spoken.eq_ignore_ascii_case(&sp)) {
                        Some(rule) => d.replacements.push(rule.clone()),
                        None => d.upsert_replacement(sp, new_term.clone(), false),
                    }
                }
            }
        }
        None => {
            if term.is_empty() || d.vocabulary.len() >= DICT_MAX_TERMS {
                return false;
            }
            d.add_term(term);
            for sp in spoken {
                d.upsert_replacement(sp, term.to_string(), false);
            }
        }
    }
    dict_sync_enabled(d);
    d.vocabulary != before_v || d.replacements != before_r || d.enabled != before_e
}

fn remove_dict_entry(d: &mut vi_core::PersonalDictionary, index: usize) {
    if let Some(e) = dict_entries(d).get(index) {
        d.vocabulary.retain(|v| !v.eq_ignore_ascii_case(&e.term));
        d.replacements.retain(|r| !r.written.eq_ignore_ascii_case(&e.term));
        dict_sync_enabled(d);
    }
}

fn is_dict_edit(id: i32) -> bool {
    (V_TERM..V_TERM + DICT_ROWS as i32).contains(&id) || (V_SOUND..V_SOUND + DICT_ROWS as i32).contains(&id)
}

/// The dictionary editor: `DICT_ROWS` rows per page, a blank row for the next term, Remove per row.
fn page_vocab(b: &mut Builder, m: &SharedUiModel, page_idx: usize) {
    let entries = dict_entries(&m.config.dictionary);
    let n = entries.len();
    let at_cap = n >= DICT_MAX_TERMS;
    let draft = !at_cap && (VOCAB_DRAFT.with(|c| c.get()) || n == 0);
    let total = n + draft as usize;
    let pages = total.div_ceil(DICT_ROWS).max(1);
    let page = page_idx.min(pages - 1);
    VOCAB_VIEW.with(|v| v.set(Some(page)));
    let first = page * DICT_ROWS;
    let rows = (total - first).min(DICT_ROWS);

    b.title("Personal dictionary", "Spell names and terms your way. Add how they sound if they get misheard.");
    b.text(r(X0 + PAD + 12, b.y, 180, 18), "Term", F::CaptionSemi, C::Text2, DT_SINGLELINE);
    b.text(r(X0 + 230, b.y, 240, 18), "Sounds like (separate with commas)", F::CaptionSemi, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
    b.y += 22;
    b.group();
    b.y += 4;
    for i in 0..rows {
        let abs = first + i;
        if i > 0 {
            b.items.push(Item::Divider { x: X0 + PAD, y: b.y, w: CW - 2 * PAD });
        }
        let (term, sounds) = entries.get(abs).map(|e| (e.term.clone(), e.sounds.join(", "))).unwrap_or_default();
        let ry = b.y;
        let (tname, sname) = if term.is_empty() { ("New term".to_string(), "Sounds like for the new term".to_string()) } else { (format!("Term {term}"), format!("Sounds like for {term}")) };
        b.ctrl(V_TERM + i as i32, Kind::Edit { text: term.clone(), password: false, multiline: false, cue: "Name or term".into() }, r(X0 + PAD, ry + 7, 190, 30), Bg::Surface, &tname);
        b.ctrl(V_SOUND + i as i32, Kind::Edit { text: sounds, password: false, multiline: false, cue: "How it sounds".into() }, r(X0 + 218, ry + 7, 250, 30), Bg::Surface, &sname);
        if abs < n {
            b.ctrl(V_DEL + i as i32, Kind::Button { style: BtnStyle::Subtle, label: "Remove".into() }, r(X0 + 480, ry + 6, 84, 32), Bg::Surface, &format!("Remove {term}"));
        }
        b.y += 44;
    }
    b.y += 4;
    let group_end = b.y;
    b.end_group();
    b.text(r(X0, group_end + 8, CW, 18), "A spelling that is also a common word is replaced whenever KeyReed hears it.", F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
    if at_cap {
        b.text(r(X0, group_end + 28, CW, 18), format!("That is the limit of {DICT_MAX_TERMS} terms. Remove one to add another."), F::Caption, C::Warn, DT_SINGLELINE | DT_END_ELLIPSIS);
    }

    // Actions sit above the "Saved" line at the foot of the window.
    let y = WIN_H - 80;
    b.ctrl(V_ADD, Kind::Button { style: BtnStyle::Secondary, label: "Add term".into() }, r(X0, y, button_width("Add term"), 32), Bg::Content, "Add term");
    b.last_ctrl().enabled = !at_cap;
    b.ctrl(V_DONE, Kind::Button { style: BtnStyle::Primary, label: "Done".into() }, r(X0 + CW - 120, y, 120, 32), Bg::Content, "Done");
    if pages > 1 {
        b.ctrl(V_NEXT, Kind::Button { style: BtnStyle::Secondary, label: "Next".into() }, r(X0 + CW - 212, y, 84, 32), Bg::Content, "Next page of terms");
        b.last_ctrl().enabled = page + 1 < pages;
        b.ctrl(V_PREV, Kind::Button { style: BtnStyle::Secondary, label: "Previous".into() }, r(X0 + CW - 304, y, 84, 32), Bg::Content, "Previous page of terms");
        b.last_ctrl().enabled = page > 0;
        b.text(r(X0 + 112, y + 7, CW - 304 - 112 - 8, 18), format!("Page {} of {pages} · {n} terms", page + 1), F::Caption, C::Text3, DT_SINGLELINE | DT_RIGHT | DT_END_ELLIPSIS);
    }
}

/// One learned habit in plain language, with its evidence: "Agreed 41 of 44 times", or "You set this".
fn style_item_lines(item: &vi_core::style::StyleItem) -> (String, String) {
    let label = vi_core::habits::habit_sentence(&item.key, &item.value)
        .unwrap_or_else(|| format!("{}: {}", item.key.replace('_', " "), item.value));
    let evidence = if item.locked {
        "You set this".to_string()
    } else {
        format!("Agreed {} of {} times", item.agree, item.seen)
    };
    (label, evidence)
}

/// "My style" (plan F3, P4.5): what is read and kept, the learning switch, the spelling choice, what has been
/// learned, the single-sighting suggestions, the excluded apps, and export / reset / delete.
fn page_mystyle(b: &mut Builder, m: &SharedUiModel) {
    let c = &m.config;
    let s = &m.style;
    b.title("My style", "What KeyReed has learned from your own corrections, and what it may change.");
    b.text(
        r(X0, b.y, CW, 34),
        "It reads only the text box it just typed into, on this PC, and keeps only the corrected words and habits \u{2014} never the text around them.",
        F::Body,
        C::Text2,
        DT_WORDBREAK,
    );
    b.y += 44;
    b.group();
    b.toggle_row(Y_LEARN, "Learn from my corrections", "Off means nothing is read and nothing is kept.", c.learn_from_edits, true);
    b.combo_row(
        Y_DIALECT,
        "Spelling",
        "What KeyReed writes, and never changes away from.",
        C::Text2,
        vec!["Auto (follow what I write)".to_string(), "British".to_string(), "American".to_string()],
        match c.dialect {
            Dialect::Auto => 0,
            Dialect::British => 1,
            Dialect::American => 2,
        },
        220,
    );
    b.end_group();

    b.section("Learned");
    b.group();
    if s.items.is_empty() {
        b.row("Nothing yet", "A word or habit you correct twice appears here, with its evidence.", C::Text2, 0);
    } else {
        let pages = s.items.len().div_ceil(STYLE_ITEMS_PER_PAGE);
        let page = STYLE_PAGE.with(|p| p.get()).min(pages - 1);
        // Keep the state on a page that exists: the Remove buttons address rows by it.
        STYLE_PAGE.with(|p| p.set(page));
        for (i, item) in s.items[page * STYLE_ITEMS_PER_PAGE..].iter().take(STYLE_ITEMS_PER_PAGE).enumerate() {
            let (label, evidence) = style_item_lines(item);
            b.button_row(Y_ITEM + i as i32, &label, &evidence, "Remove", BtnStyle::Subtle);
        }
        if pages > 1 {
            let y = b.y;
            b.ctrl(Y_PREV, Kind::Button { style: BtnStyle::Secondary, label: "Previous".into() }, r(X0 + PAD, y + 6, 84, 32), Bg::Surface, "Previous learned items");
            b.last_ctrl().enabled = page > 0;
            b.ctrl(Y_NEXT, Kind::Button { style: BtnStyle::Secondary, label: "Next".into() }, r(X0 + PAD + 92, y + 6, 84, 32), Bg::Surface, "More learned items");
            b.last_ctrl().enabled = page + 1 < pages;
            b.text(r(X0 + PAD + 188, y + 13, CW - 2 * PAD - 188, 18), format!("Page {} of {pages}", page + 1), F::Caption, C::Text3, DT_SINGLELINE);
            b.y += 44;
        }
    }
    b.end_group();

    b.section("Suggestions");
    b.group();
    if s.vocabulary_candidates.is_empty() {
        b.row("Nothing waiting", "A word you fix once waits here until you add it or dismiss it.", C::Text2, 0);
    } else {
        for (i, cand) in s.vocabulary_candidates.iter().take(STYLE_CANDIDATES_SHOWN).enumerate() {
            let evidence = format!("Corrected {} time{} in {}", cand.count, if cand.count == 1 { "" } else { "s" }, cand.context);
            let (y0, h) = b.row(&format!("{} \u{2192} {}", cand.from, cand.to), &evidence, C::Text2, 176);
            b.ctrl(Y_CAND + i as i32, Kind::Button { style: BtnStyle::Secondary, label: "Add".into() }, r(X0 + CW - PAD - 176, y0 + (h - 32) / 2, 80, 32), Bg::Surface, &format!("Add {} to my dictionary", cand.to));
            b.ctrl(Y_CAND_DISMISS + i as i32, Kind::Button { style: BtnStyle::Subtle, label: "Dismiss".into() }, r(X0 + CW - PAD - 88, y0 + (h - 32) / 2, 88, 32), Bg::Surface, &format!("Dismiss the suggestion {}", cand.to));
        }
        let more = s.vocabulary_candidates.len().saturating_sub(STYLE_CANDIDATES_SHOWN);
        if more > 0 {
            b.text(r(X0 + PAD, b.y + 8, CW - 2 * PAD, 18), format!("{more} more wait behind these."), F::Caption, C::Text3, DT_SINGLELINE);
            b.y += 30;
        }
    }
    b.end_group();

    b.section("Apps KeyReed never reads");
    b.group();
    if s.excluded_apps.is_empty() {
        b.row("None excluded", "Password managers and private windows are skipped anyway.", C::Text2, 0);
    } else {
        for (i, app) in s.excluded_apps.iter().take(STYLE_APPS_SHOWN).enumerate() {
            b.button_row(Y_APP + i as i32, app, "Nothing is read in this app.", "Remove", BtnStyle::Subtle);
        }
        let more = s.excluded_apps.len().saturating_sub(STYLE_APPS_SHOWN);
        if more > 0 {
            b.text(r(X0 + PAD, b.y + 8, CW - 2 * PAD, 18), format!("{more} more are excluded."), F::Caption, C::Text3, DT_SINGLELINE);
            b.y += 30;
        }
    }
    b.end_group();

    b.group();
    let y = b.y + 4;
    b.ctrl(Y_EXPORT, Kind::Button { style: BtnStyle::Secondary, label: "Export".into() }, r(X0 + PAD, y, 84, 32), Bg::Surface, "Export my style as a file");
    b.ctrl(Y_RESET, Kind::Button { style: BtnStyle::Secondary, label: "Reset learned".into() }, r(X0 + PAD + 92, y, 136, 32), Bg::Surface, "Reset everything learned");
    b.ctrl(Y_DELETE, Kind::Button { style: BtnStyle::Danger, label: "Delete all".into() }, r(X0 + PAD + 236, y, 112, 32), Bg::Surface, "Delete everything learned and the excluded apps");
    b.y = y + 44;
    b.end_group();
}

fn page_output(b: &mut Builder, m: &SharedUiModel) {
    b.title("Output", "Where your words go when you finish speaking.");
    b.section("Delivery");
    for (i, (id, mode)) in [(O_TYPE, OutputMode::DirectTyping), (O_CLIP, OutputMode::Clipboard), (O_ASK, OutputMode::AskBeforeInserting)]
        .iter()
        .enumerate()
    {
        let title = if i == 0 { format!("{} (recommended)", mode.title()) } else { mode.title().to_string() };
        b.choice(*id, r(X0, b.y, CW, 74), &title, mode.description(), m.config.output_mode == *mode);
        b.y += 82;
    }
    b.y += 4;
    b.section("Clipboard");
    b.group();
    b.button_row(O_COPYLAST, "Copy last dictation", "Put your most recent dictation back on the clipboard.", "Copy", BtnStyle::Secondary);
    b.end_group();
}

fn page_audio(b: &mut Builder, m: &SharedUiModel) {
    b.title("Microphone", "Choose a microphone and check that KeyReed hears you clearly.");
    let c = &m.config;
    b.section("Input device");
    b.group();
    let (items, sel) = mic_items(m);
    b.combo_row(A_MIC, "Microphone", "If it's unplugged, KeyReed switches to the Windows default.", C::Text2, items, sel, 300);
    b.end_group();
    b.section("Sounds");
    b.group();
    let styles: Vec<String> = AudioCueStyle::ALL.iter().map(|s| s.label().to_string()).collect();
    let sel = AudioCueStyle::ALL.iter().position(|s| *s == c.audio_cue_style).unwrap_or(0);
    b.combo_row(
        A_CUE,
        "Cue",
        "A short tone when listening starts, a different one when the text is ready.",
        C::Text2,
        styles,
        sel,
        220,
    );
    // Steps of 5 %: fine enough to place the cue, and no slider control exists in this window.
    let levels: Vec<String> = (0..=20).map(|i| format!("{} %", i * 5)).collect();
    b.combo_row(
        A_VOL,
        "Cue volume",
        "Moves only the cue. Your microphone and playback volume stay as they are.",
        C::Text2,
        levels,
        (c.audio_cue_volume.min(100) / 5) as usize,
        220,
    );
    b.button_row(A_PLAY, "Hear it", "Plays the cue now, at this volume.", "Play", BtnStyle::Secondary);
    b.end_group();
    b.section("Test");
    mic_test_block(b, m, A_TEST);
}

/// One speech-model row (used by Speech models and setup step 3).
fn model_row(b: &mut Builder, m: &SharedUiModel, id: &str, action_id: i32, footnote: bool) {
    let Some(info) = asr_ui_info(id) else { return };
    if b.rows_in_group > 0 {
        b.items.push(Item::Divider { x: X0 + PAD, y: b.y, w: CW - 2 * PAD });
    }
    b.rows_in_group += 1;
    let y = b.y;
    let installed = m.is_installed(info.id);
    let active = m.config.local_asr_model.eq_ignore_ascii_case(info.id);
    let dl = m.download.as_ref().filter(|d| d.model_id.eq_ignore_ascii_case(info.id));
    let other_active = m.download.as_ref().map(|d| d.is_active() && !d.model_id.eq_ignore_ascii_case(info.id)).unwrap_or(false);

    b.text(r(X0 + PAD, y + 10, 260, 20), info.name, F::BodySemi, C::Text, DT_SINGLELINE);
    // Badges follow the model name; their x is measured at paint time.
    let badge = |b: &mut Builder, s: &str, tone: Tone| {
        b.items.push(Item::Badge { x: X0 + PAD, y: y + 10, s: s.into(), tone, after: Some(info.name.to_string()) });
    };
    let downloading = dl.map(|d| d.is_active()).unwrap_or(false);
    // The model in use is marked once, in the action column (BtnStyle::Status), not with a second badge.
    if installed && !downloading && !active {
        badge(b, "Installed", Tone::Info);
    }
    if info.recommended {
        badge(b, "Recommended", Tone::Info);
    }
    b.text(r(X0 + PAD, y + 32, CW - 200, 18), info.summary, F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);

    let mut facts = format!("{} · {} MB download", info.languages, info.download_mb);
    if let Some(s) = info.measured_s_per_10s {
        facts.push_str(&format!(" · {s:.1} s to transcribe 10 s of speech{}", if footnote { "*" } else { "" }));
    }
    let (label, style, enabled) = match dl.map(|d| &d.stage) {
        Some(DownloadStage::Starting) | Some(DownloadStage::Downloading) => ("Cancel", BtnStyle::Secondary, true),
        Some(DownloadStage::Verifying) => ("Verifying…", BtnStyle::Secondary, false),
        Some(DownloadStage::Failed(_)) if !installed => ("Try again", BtnStyle::Primary, !other_active),
        _ if active && installed => ("In use", BtnStyle::Status, false),
        _ if installed => ("Use", BtnStyle::Secondary, true),
        _ => ("Download", if info.recommended { BtnStyle::Primary } else { BtnStyle::Secondary }, !other_active),
    };
    match dl {
        Some(d) if d.is_active() => {
            b.items.push(Item::Progress { r: r(X0 + PAD, y + 55, 220, 6), frac: d.fraction() });
            b.text(r(X0 + PAD + 232, y + 49, CW - 400, 18), d.detail_text(), F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        Some(d) if matches!(d.stage, DownloadStage::Failed(_)) && !installed => {
            b.text(r(X0 + PAD, y + 49, CW - 200, 18), d.detail_text(), F::Caption, C::Err, DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        Some(d) if d.stage == DownloadStage::Cancelled && !installed => {
            b.text(r(X0 + PAD, y + 49, CW - 200, 18), d.detail_text(), F::Caption, C::Text3, DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        _ => b.text(r(X0 + PAD, y + 49, CW - 200, 18), facts, F::Caption, C::Text3, DT_SINGLELINE | DT_END_ELLIPSIS),
    }
    let bw = 120;
    b.ctrl(action_id, Kind::Button { style, label: label.into() }, r(X0 + CW - PAD - bw, y + 18, bw, 32), Bg::Surface, &format!("{label} {}", info.name));
    b.last_ctrl().enabled = enabled;
    if other_active && !installed {
        b.last_ctrl().tip = Some("One download at a time. Wait for the current download to finish.".into());
    }
    b.y += 72;
}

/// The Quick Fix grammar model: the same row anatomy as a speech model, but it powers Quick Fix, so its
/// action column installs only (Quick Fix finds it by folder name; there is nothing to switch).
fn grammar_row(b: &mut Builder, m: &SharedUiModel, action_id: i32) {
    let y = b.y;
    let installed = m.is_installed(GRAMMAR_MODEL_ID);
    let dl = m.download.as_ref().filter(|d| d.model_id.eq_ignore_ascii_case(GRAMMAR_MODEL_ID));
    b.text(r(X0 + PAD, y + 10, 260, 20), "Grammar tagger", F::BodySemi, C::Text, DT_SINGLELINE);
    if installed {
        b.items.push(Item::Badge { x: X0 + PAD, y: y + 10, s: "In use by Quick Fix".into(), tone: Tone::Info, after: Some("Grammar tagger".into()) });
    }
    b.text(r(X0 + PAD, y + 32, CW - 200, 18), "Checks grammar when you run Quick Fix.", F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
    let (label, style, enabled) = match dl.map(|d| &d.stage) {
        Some(DownloadStage::Starting) | Some(DownloadStage::Downloading) => ("Cancel", BtnStyle::Secondary, true),
        Some(DownloadStage::Verifying) => ("Verifying…", BtnStyle::Secondary, false),
        Some(DownloadStage::Failed(_)) if !installed => ("Try again", BtnStyle::Primary, true),
        _ if installed => ("Installed", BtnStyle::Status, false),
        _ => ("Download", BtnStyle::Primary, true),
    };
    match dl {
        Some(d) if d.is_active() => {
            b.items.push(Item::Progress { r: r(X0 + PAD, y + 55, 220, 6), frac: d.fraction() });
            b.text(r(X0 + PAD + 232, y + 49, CW - 400, 18), d.detail_text(), F::Caption, C::Text2, DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        Some(d) if matches!(d.stage, DownloadStage::Failed(_)) && !installed => {
            b.text(r(X0 + PAD, y + 49, CW - 200, 18), d.detail_text(), F::Caption, C::Err, DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        _ => b.text(
            r(X0 + PAD, y + 49, CW - 200, 18),
            "English · about 70 MB · runs on this PC",
            F::Caption,
            C::Text3,
            DT_SINGLELINE | DT_END_ELLIPSIS,
        ),
    }
    let bw = 120;
    b.ctrl(action_id, Kind::Button { style, label: label.into() }, r(X0 + CW - PAD - bw, y + 18, bw, 32), Bg::Surface, &format!("{label} Grammar tagger"));
    b.last_ctrl().enabled = enabled;
    b.y += 72;
}

fn page_models(b: &mut Builder, m: &SharedUiModel) {
    b.title("Speech models", "Models run on this PC. Download the ones you want and choose which to use.");
    if !m.active_model_ready() && m.download.as_ref().map(|d| !d.is_active()).unwrap_or(true) {
        b.items.push(Item::Banner {
            r: r(X0, b.y, CW, 52),
            tone: Tone::Warn,
            title: "Your selected model isn't installed".into(),
            body: "Download a model below to start dictating.".into(),
        });
        b.y += 64;
    }
    b.group();
    for (i, info) in ASR_UI_CATALOG.iter().enumerate() {
        model_row(b, m, info.id, M_ACT + i as i32, true);
    }
    b.end_group();
    let note = if ASR_UI_CATALOG.iter().any(|i| i.measured_s_per_10s.is_some()) {
        "*Measured on the KeyReed test laptop (CPU only); your PC may be faster or slower. Every model is verified with SHA-256 before use."
    } else {
        "Every model is verified with SHA-256 before use."
    };
    b.text(r(X0, b.y, CW, 34), note, F::Caption, C::Text3, DT_WORDBREAK);
    // Two wrapped lines of caption, then room to separate the next section's heading.
    b.y += 42;
    b.section("Quick Fix grammar");
    b.group();
    grammar_row(b, m, M_GRAMMAR);
    b.end_group();
}

fn page_processing(b: &mut Builder, m: &SharedUiModel) {
    let c = &m.config;
    b.title("Processing", "Choose where speech is processed. On this PC is the default.");
    for (i, p) in [ProcessingProfile::MaxPrivacyOffline, ProcessingProfile::Balanced, ProcessingProfile::MaxAccuracy].iter().enumerate() {
        b.choice(P_PROF + i as i32, r(X0, b.y, CW, 62), p.title(), p.description(), c.processing_profile == *p);
        b.y += 70;
    }
    b.y += 4;
    b.section("Cloud key (optional)");
    b.group();
    let has_key = c.cloud.encrypted_api_key_b64.is_some();
    let y = b.y;
    b.text(r(X0 + PAD, y + 10, CW - 2 * PAD, 20), "API key", F::Body, C::Text, DT_SINGLELINE);
    b.text(
        r(X0 + PAD, y + 31, CW - 2 * PAD, 18),
        if has_key { "A key is saved, encrypted with Windows DPAPI. Paste a new one to replace it." } else { "Only used by the cloud options above. Stored encrypted with Windows DPAPI." },
        F::Caption,
        if has_key { C::Ok } else { C::Text2 },
        DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    b.ctrl(P_KEY, Kind::Edit { text: String::new(), password: true, multiline: false, cue: "Paste an API key".into() }, r(X0 + PAD, y + 58, 320, 30), Bg::Surface, "API key");
    b.ctrl(P_KEY_SAVE, Kind::Button { style: BtnStyle::Secondary, label: "Save key".into() }, r(X0 + PAD + 332, y + 57, 100, 32), Bg::Surface, "Save key");
    if has_key {
        b.ctrl(P_KEY_REMOVE, Kind::Button { style: BtnStyle::Danger, label: "Remove".into() }, r(X0 + PAD + 442, y + 57, 96, 32), Bg::Surface, "Remove key");
    }
    b.y = y + 102;
    b.end_group();
    b.section("Grammar polish");
    b.group();
    let llm_sel = if c.local_llm_model.eq_ignore_ascii_case("none") { 1 } else { 0 };
    b.combo_row(
        P_LLM,
        "Polish engine",
        "Local AI fixes grammar and wording. Rules only uses less memory.",
        C::Text2,
        vec!["Local AI (Qwen3.5 0.8B)".into(), "Rules only (fastest)".into()],
        llm_sel,
        230,
    );
    b.end_group();
}

fn page_privacy(b: &mut Builder, m: &SharedUiModel) {
    let c = &m.config;
    b.title("Privacy", "KeyReed works offline. Your recordings are processed in memory and never saved.");
    b.section("Protection");
    b.group();
    b.toggle_row(V_PWD, "Never type into password fields", "Password boxes get the clipboard, never keystrokes.", c.privacy.block_password_fields, true);
    b.toggle_row(V_REDACT, "Hide private window titles", "Don't read titles of private browser windows and password managers.", c.privacy.redact_sensitive_window_titles, true);
    b.toggle_row(V_CLIPHIST, "Keep dictation out of clipboard history", "Excludes KeyReed text from Win+V history and cloud clipboard.", c.privacy.exclude_from_clipboard_history, true);
    b.toggle_row(V_CLOUD_JOBS, "Let text jobs use the cloud", "Composer, Quick Fix and voice commands may send text to your cloud key. This never applies under Maximum Privacy.", c.text_jobs_allow_cloud, true);
    b.end_group();
    b.section("Your data");
    b.group();
    b.row("Audio", "Processed in memory on this PC and never written to disk.", C::Text2, 0);
    b.button_row(V_CLEAR_HIST, "Dictation history", "Off by default. Clear removes any saved history on this PC.", "Clear history", BtnStyle::Danger);
    b.end_group();
}

fn page_about(b: &mut Builder, _m: &SharedUiModel) {
    b.y = 40;
    b.items.push(Item::Glyph { r: r(X0, b.y, 64, 64), ch: '\u{0}', font: F::Display, color: C::Text }); // brand mark slot
    b.text(r(X0 + 80, b.y + 2, CW - 80, 40), PRODUCT_NAME, F::Display, C::Text, DT_SINGLELINE);
    b.text(r(X0 + 80, b.y + 42, CW - 80, 20), PRODUCT_TAGLINE, F::Body, C::Text2, DT_SINGLELINE);
    b.text(r(X0 + 80, b.y + 64, CW - 80, 18), format!("Version {VERSION_STR} · Windows 10 and 11 · x64"), F::Caption, C::Text3, DT_SINGLELINE);
    b.y += 104;
    b.section("Links");
    b.group();
    b.button_row(AB_RELEASES, "Release notes", "See what changed in each version.", "Open", BtnStyle::Secondary);
    b.button_row(AB_UPDATES, "Check for updates", "KeyReed doesn't update itself. This opens the latest release.", "Check", BtnStyle::Secondary);
    b.button_row(AB_GITHUB, "Source code", "github.com/Shanmukh-l33t/keyreed", "Open", BtnStyle::Secondary);
    b.button_row(AB_LICENSE, "License", "MIT or Apache 2.0, your choice.", "View", BtnStyle::Secondary);
    b.end_group();
    b.section("Maintenance");
    b.group();
    b.button_row(AB_SETUP, "Run setup again", "Walk through shortcut, microphone and model setup.", "Start", BtnStyle::Secondary);
    b.button_row(AB_RESET, "Reset all settings", "Restores defaults. Downloaded models are kept.", "Reset…", BtnStyle::Danger);
    b.end_group();
}

// ------------------------------------------------------------------------------------------------
// Control lifecycle
// ------------------------------------------------------------------------------------------------
//
// Re-entrancy rule: Win32 calls that create, destroy, focus, enable or retitle child windows can
// synchronously send messages back to `wnd_proc` (WM_CTLCOLOREDIT, WM_COMMAND/EN_KILLFOCUS, ...).
// So the `ST` RefCell is never borrowed across such calls: compute under the borrow, release it,
// then talk to Win32.

const EM_SETCUEBANNER: u32 = 0x1501;
const TTM_ADDTOOLW: u32 = 0x0432;
const TTM_DELTOOLW: u32 = 0x0433;
const TTM_SETMAXTIPWIDTH: u32 = 0x0418;

thread_local! {
    static EDIT_BRUSH: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

#[repr(C)]
struct ToolInfo {
    cb_size: u32,
    flags: u32,
    hwnd: HWND,
    id: usize,
    rect: RECT,
    hinst: *mut std::ffi::c_void,
    text: *mut u16,
    lparam: LPARAM,
    reserved: *mut std::ffi::c_void,
}

/// Everything control creation needs, copied out of `St` so no borrow is held.
struct Ctx {
    hwnd: HWND,
    dpi: u32,
    scroll: i32,
    body: HFONT,
    tooltip: HWND,
    practice_text: String,
    key_draft: String,
}

fn ctx_of(st: &St) -> Ctx {
    Ctx {
        hwnd: st.hwnd,
        dpi: st.dpi,
        scroll: st.scroll,
        body: st.fonts.get(F::Body),
        tooltip: st.tooltip,
        practice_text: st.practice_text.clone(),
        key_draft: st.key_draft.clone(),
    }
}

/// Where the child window goes. Text boxes are inset into the field painted around them (see `paint`),
/// so the border, hover and focus ring are drawn by KeyReed instead of the 3D system edge.
fn child_rect(c: &Ctrl) -> R {
    match &c.kind {
        Kind::Edit { multiline: true, .. } => r(c.r.x + 12, c.r.y + 10, c.r.w - 24, c.r.h - 20),
        Kind::Edit { .. } => r(c.r.x + 12, c.r.y + (c.r.h - 20) / 2, c.r.w - 24, 20),
        _ => c.r,
    }
}

unsafe fn create_ctrl(cx: &Ctx, c: &Ctrl) -> HWND {
    let hinst = GetModuleHandleW(std::ptr::null());
    let d = cx.dpi;
    let dy = if fixed_in_place(&c.kind) { 0 } else { cx.scroll };
    let cr = child_rect(c);
    let (x, y, w, h) = (sc(cr.x, d), sc(cr.y - dy, d), sc(cr.w, d), sc(cr.h, d));
    let (cls, style, ex, text): (&str, u32, u32, String) = match &c.kind {
        // Window buttons are not in the tab order: Tab belongs to the page.
        Kind::WinBtn { .. } => ("BUTTON", 0x0B /* BS_OWNERDRAW */, 0, c.name.clone()),
        Kind::Nav { .. } | Kind::Toggle { .. } | Kind::Choice { .. } | Kind::Button { .. } | Kind::Combo { .. } | Kind::Shortcut { .. } => {
            ("BUTTON", 0x0B /* BS_OWNERDRAW */ | WS_TABSTOP, 0, c.name.clone())
        }
        Kind::Edit { text, password, multiline, .. } => {
            let mut s = WS_TABSTOP | 0x0080 /* ES_AUTOHSCROLL */;
            if *password {
                s |= 0x0020;
            }
            if *multiline {
                s = WS_TABSTOP | 0x0004 /* ES_MULTILINE */ | 0x0040 /* ES_AUTOVSCROLL */ | 0x1000 /* ES_WANTRETURN */;
            }
            let initial = if *multiline { cx.practice_text.clone() } else if *password { cx.key_draft.clone() } else { text.clone() };
            ("EDIT", s, 0, initial)
        }
    };
    let clsw = to_wide(cls);
    let tw = to_wide(&text);
    // Created hidden + disabled-state applied first so no half-initialised control ever paints.
    let hwnd = CreateWindowExW(ex, clsw.as_ptr(), tw.as_ptr(), WS_CHILD | style, x, y, w, h, cx.hwnd, c.id as isize as HMENU, hinst, std::ptr::null());
    if hwnd.is_null() {
        return hwnd;
    }
    SendMessageW(hwnd, WM_SETFONT, cx.body as WPARAM, 0);
    if matches!(c.kind, Kind::Edit { .. }) {
        theme_native_control(hwnd, "Edit");
        if is_dict_edit(c.id) {
            let max = if c.id < V_SOUND { DICT_MAX_TERM_CHARS } else { DICT_MAX_SOUNDS_CHARS };
            SendMessageW(hwnd, 0x00C5 /* EM_LIMITTEXT */, max, 0);
        }
    }
    match &c.kind {
        Kind::Edit { cue, .. } => {
            SetWindowSubclass(hwnd, Some(edit_wheel_subclass), 1, 0);
            if !cue.is_empty() {
                let w = to_wide(cue);
                SendMessageW(hwnd, EM_SETCUEBANNER, 1, w.as_ptr() as LPARAM);
            }
        }
        _ => {
            // Reference data: 1 = drop-down field, 2 = shortcut capture field, 3 = navigation item.
            let kind = match c.kind {
                Kind::Combo { .. } => 1,
                Kind::Shortcut { .. } => 2,
                Kind::Nav { .. } => 3,
                _ => 0,
            };
            SetWindowSubclass(hwnd, Some(btn_subclass), 1, kind);
        }
    }
    if !c.enabled {
        EnableWindow(hwnd, 0);
    }
    if let Some(tip) = &c.tip {
        add_tooltip(cx, hwnd, tip);
    }
    ShowWindow(hwnd, SW_SHOWNA);
    hwnd
}

unsafe fn add_tooltip(cx: &Ctx, ctl: HWND, tip: &str) {
    if cx.tooltip.is_null() {
        return;
    }
    let mut text = to_wide(tip);
    let mut ti = ToolInfo {
        cb_size: std::mem::size_of::<ToolInfo>() as u32,
        flags: 0x01 | 0x10, // TTF_IDISHWND | TTF_SUBCLASS
        hwnd: cx.hwnd,
        id: ctl as usize,
        rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
        hinst: std::ptr::null_mut(),
        text: text.as_mut_ptr(),
        lparam: 0,
        reserved: std::ptr::null_mut(),
    };
    SendMessageW(cx.tooltip, TTM_ADDTOOLW, 0, &mut ti as *mut _ as LPARAM);
}

unsafe fn remove_tooltip(cx: &Ctx, ctl: HWND) {
    if cx.tooltip.is_null() {
        return;
    }
    let mut ti = ToolInfo {
        cb_size: std::mem::size_of::<ToolInfo>() as u32,
        flags: 0x01,
        hwnd: cx.hwnd,
        id: ctl as usize,
        rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
        hinst: std::ptr::null_mut(),
        text: std::ptr::null_mut(),
        lparam: 0,
        reserved: std::ptr::null_mut(),
    };
    SendMessageW(cx.tooltip, TTM_DELTOOLW, 0, &mut ti as *mut _ as LPARAM);
}

fn read_text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

/// Copies edit contents into `St` (practice text, unsaved key) and returns the texts of the dictionary fields
/// (control id, text), which the caller applies *after* releasing the borrow. Reading child text only messages the
/// child controls, never `wnd_proc`.
fn harvest_edits(st: &mut St) -> Vec<(i32, String)> {
    let mut dict = Vec::new();
    for (c, h) in &st.ctrls {
        match c.id {
            S_PRACTICE => st.practice_text = read_text(*h),
            P_KEY => st.key_draft = read_text(*h),
            id if is_dict_edit(id) => dict.push((id, read_text(*h))),
            _ => {}
        }
    }
    dict
}

fn harvest_and_commit() {
    let edits = ST.with(|s| s.borrow_mut().as_mut().map(harvest_edits)).unwrap_or_default();
    apply_dict_edits(&edits, false);
}

fn parse_vocab(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Saves the edited dictionary rows in one change. With `skip_draft`, the blank row for a new term is left alone
/// (it is saved when focus leaves the row or on Enter), so adding a row never rebuilds the page under the caret.
fn apply_dict_edits(edits: &[(i32, String)], skip_draft: bool) {
    if edits.is_empty() {
        return;
    }
    let Some(m) = snapshot() else { return };
    let page = VOCAB_VIEW.with(|v| v.get()).unwrap_or(0);
    let mut d = m.config.dictionary.clone();
    let n = dict_entries(&d).len();
    let text = |base: i32, i: usize| edits.iter().find(|(id, _)| *id == base + i as i32).map(|(_, t)| t.as_str());
    let mut changed = false;
    for i in 0..DICT_ROWS {
        let (Some(term), Some(sounds)) = (text(V_TERM, i), text(V_SOUND, i)) else { continue };
        let abs = page * DICT_ROWS + i;
        let index = (abs < n).then_some(abs);
        if index.is_none() && skip_draft {
            continue;
        }
        changed |= apply_dict_row(&mut d, index, term, sounds);
    }
    if changed {
        commit(|c| c.dictionary = d);
    }
}

fn focused_id() -> i32 {
    unsafe {
        let f = GetFocus();
        if f.is_null() {
            0
        } else {
            GetDlgCtrlID(f)
        }
    }
}

/// Runs after a dictionary field lost focus, once focus has settled on its new owner.
fn dict_commit_deferred() {
    let id = focused_id();
    let row = if (V_TERM..V_TERM + DICT_ROWS as i32).contains(&id) {
        Some((id - V_TERM) as usize)
    } else if (V_SOUND..V_SOUND + DICT_ROWS as i32).contains(&id) {
        Some((id - V_SOUND) as usize)
    } else {
        None
    };
    let page = VOCAB_VIEW.with(|v| v.get()).unwrap_or(0);
    let n = snapshot().map(|m| dict_entries(&m.config.dictionary).len()).unwrap_or(0);
    let in_draft = row.is_some_and(|r| page * DICT_ROWS + r >= n);
    let edits = ST.with(|s| s.try_borrow_mut().ok().and_then(|mut b| b.as_mut().map(harvest_edits))).unwrap_or_default();
    apply_dict_edits(&edits, in_draft);
}

fn focus_ctrl(id: i32) {
    unsafe {
        let h = GetDlgItem(hwnd(), id);
        if !h.is_null() {
            SetFocus(h);
        }
    }
}

/// Leaves the dictionary editor for the Writing page.
fn close_vocab() {
    harvest_and_commit();
    VOCAB_VIEW.with(|v| v.set(None));
    VOCAB_DRAFT.with(|v| v.set(false));
    rebuild(false);
    focus_ctrl(W_VOCAB);
}

/// Moves the editor to `page` (rows per page: `DICT_ROWS`) and focuses its first field.
fn vocab_goto(page: usize, draft: bool) {
    VOCAB_VIEW.with(|v| v.set(Some(page)));
    VOCAB_DRAFT.with(|v| v.set(draft));
    rebuild(false);
}

fn dict_len() -> usize {
    snapshot().map(|m| dict_entries(&m.config.dictionary).len()).unwrap_or(0)
}

/// Enter in a "Sounds like" field: save the row; a newly added term is followed by a fresh blank row.
fn dict_enter(row: usize) {
    let before = dict_len();
    harvest_and_commit();
    let after = dict_len();
    let page = VOCAB_VIEW.with(|v| v.get()).unwrap_or(0);
    if page * DICT_ROWS + row >= before && after > before {
        vocab_goto(after / DICT_ROWS, true);
        focus_ctrl(V_TERM + (after % DICT_ROWS) as i32);
    } else {
        unsafe { SendMessageW(hwnd(), WM_NEXTDLGCTL, 0, 0) };
    }
}

/// Return: presses the focused push button, moves Term to Sounds like, saves a Sounds like row, and otherwise
/// presses the setup step's primary button (never a destructive one).
fn on_enter() {
    let id = focused_id();
    if (V_SOUND..V_SOUND + DICT_ROWS as i32).contains(&id) {
        return dict_enter((id - V_SOUND) as usize);
    }
    if (V_TERM..V_TERM + DICT_ROWS as i32).contains(&id) {
        unsafe { SendMessageW(hwnd(), WM_NEXTDLGCTL, 0, 0) };
        return;
    }
    let (is_button, in_setup) = ST
        .with(|s| {
            let b = s.try_borrow().ok()?;
            let st = b.as_ref()?;
            let pressable = st.ctrls.iter().any(|(c, _)| c.id == id && c.enabled && matches!(c.kind, Kind::Button { .. }));
            Some((pressable, st.page == PAGE_START))
        })
        .unwrap_or((false, false));
    if is_button {
        on_command(id, 0);
    } else if in_setup {
        on_command(S_NEXT, 0);
    }
}

/// Full rebuild of page controls (page switch / DPI change / structure change).
/// The logical bottom of a page: the lowest control or item, never less than the window.
fn content_height(items: &[Item], ctrls: &[Ctrl]) -> i32 {
    let items_bottom = items.iter().map(|it| match it {
        Item::Text { r, .. }
        | Item::Meter { r, .. }
        | Item::Progress { r, .. }
        | Item::Glyph { r, .. }
        | Item::Steps { r, .. }
        | Item::Banner { r, .. }
        | Item::Group { r }
        | Item::Card { r } => r.y + r.h,
        Item::Badge { y, .. } | Item::Keycap { y, .. } => y + 22,
        Item::Divider { y, .. } => *y,
    });
    let ctrls_bottom = ctrls.iter().map(|c| c.r.y + c.r.h);
    let bottom = items_bottom.chain(ctrls_bottom).max().unwrap_or(WIN_H);
    // 24 px of breathing room under the last row, but a page that fits the window never scrolls and
    // never shows a scrollbar.
    (bottom + 24).max(WIN_H)
}

/// The largest offset a page can scroll to: the part of its content that hangs below the window.
fn max_scroll(content_h: i32, view_h: i32) -> i32 {
    (content_h - view_h).max(0)
}

/// The offset after one step: never negative, never past the end of the page.
fn next_scroll(cur: i32, delta: i32, content_h: i32, view_h: i32) -> i32 {
    (cur + delta).clamp(0, max_scroll(content_h, view_h))
}

/// How far to scroll so the control at `y` (design units) is fully in the view again, 8 px clear of the
/// edge. `None` when it is already there.
fn reveal_delta(y: i32, h: i32, scroll: i32, view_h: i32) -> Option<i32> {
    let (top, bottom) = (y - scroll, y - scroll + h);
    if bottom > view_h {
        Some(bottom - view_h + 8)
    } else if top < 0 {
        Some(top - 8)
    } else {
        None
    }
}

fn rebuild(focus_first: bool) {
    let Some(m) = snapshot() else { return };
    // Phase 1: compute under the borrow and take ownership of the old controls.
    let prep = ST.with(|s| {
        let mut sb = s.borrow_mut();
        let st = sb.as_mut()?;
        let saved = st.saved_at.map(|t| t.elapsed().as_millis() < 1800).unwrap_or(false);
        let (items, ctrls) = build(st.page, st.step, &m, saved);
        let old: Vec<(Ctrl, HWND)> = st.ctrls.drain(..).collect();
        st.content_h = content_height(&items, &ctrls);
        st.items = items;
        Some((ctrls, old, ctx_of(st), st.page))
    });
    let Some((ctrls, old, cx, page)) = prep else { return };
    unsafe {
        // Phase 2: talk to Win32 with no borrow held.
        let focus = GetFocus();
        let focused_id = old.iter().find(|(_, h)| *h == focus).map(|(c, _)| c.id);
        for (_, h) in &old {
            remove_tooltip(&cx, *h);
            DestroyWindow(*h);
        }
        let created: Vec<(Ctrl, HWND)> = ctrls.into_iter().map(|c| {
            let h = create_ctrl(&cx, &c);
            (c, h)
        }).collect();
        let target = if MODAL.with(|m| m.borrow().is_some()) {
            created.iter().find(|(c, _)| matches!(c.id, MODAL_CANCEL | MODAL_CONFIRM)).map(|(_, h)| *h)
        } else if focus_first {
            created.iter().find(|(c, _)| c.id == ID_NAV + page as i32).map(|(_, h)| *h)
        } else {
            focused_id.and_then(|id| created.iter().find(|(c, _)| c.id == id).map(|(_, h)| *h))
        };
        // Phase 3: store.
        let regions = ST.with(|s| {
            let mut sb = s.borrow_mut();
            let st = sb.as_mut()?;
            st.ctrls = created;
            // A card in front owns the screen: only its buttons show.
            show_children(st);
            Some(child_regions(st))
        });
        if let Some(regions) = regions {
            apply_regions(&regions);
        }
        if let Some(h) = target {
            if GetForegroundWindow() == cx.hwnd {
                SetFocus(h);
            }
        }
        InvalidateRect(cx.hwnd, std::ptr::null(), 0);
    }
}

enum Op {
    Enable(HWND, bool),
    Title(HWND, String),
    Animate,
    Invalidate(HWND),
}

/// Incremental update: same structure → update in place; different structure → rebuild.
fn refresh() {
    let Some(m) = snapshot() else { return };
    track_practice(&m);
    let mut ops: Vec<Op> = Vec::new();
    let mut need_rebuild = false;
    let mut parent: HWND = std::ptr::null_mut();
    ST.with(|s| {
        let Ok(mut sb) = s.try_borrow_mut() else { return };
        let Some(st) = sb.as_mut() else { return };
        parent = st.hwnd;
        let saved = st.saved_at.map(|t| t.elapsed().as_millis() < 1800).unwrap_or(false);
        let (items, ctrls) = build(st.page, st.step, &m, saved);
        let same_structure = ctrls.len() == st.ctrls.len()
            && ctrls.iter().zip(st.ctrls.iter()).all(|(a, (b, _))| a.id == b.id && a.kind.tag() == b.kind.tag() && a.r == b.r);
        if !same_structure {
            need_rebuild = true;
            // Keep practice text / unsaved key across the rebuild (no commit here).
            for (c, h) in &st.ctrls {
                match c.id {
                    S_PRACTICE => st.practice_text = read_text(*h),
                    P_KEY => st.key_draft = read_text(*h),
                    _ => {}
                }
            }
            return;
        }
        for (new, (old, h)) in ctrls.into_iter().zip(st.ctrls.iter_mut()) {
            if new == *old {
                continue;
            }
            if new.enabled != old.enabled {
                ops.push(Op::Enable(*h, new.enabled));
            }
            if let (Kind::Toggle { on: a }, Kind::Toggle { on: b }) = (&new.kind, &old.kind) {
                if a != b && !crate::ui::win_gui::reduced_motion() {
                    st.anims.retain(|(i, _)| *i != new.id);
                    st.anims.push((new.id, Instant::now()));
                    ops.push(Op::Animate);
                }
            }
            if new.name != old.name && !matches!(new.kind, Kind::Edit { .. }) {
                ops.push(Op::Title(*h, new.name.clone()));
            }
            ops.push(Op::Invalidate(*h));
            *old = new;
        }
        if items != st.items {
            st.items = items;
            ops.push(Op::Invalidate(st.hwnd));
        }
    });
    if need_rebuild {
        rebuild(false);
        maybe_arm_advance();
        return;
    }
    maybe_arm_advance();
    unsafe {
        for op in ops {
            match op {
                Op::Enable(h, e) => {
                    EnableWindow(h, e as i32);
                }
                Op::Title(h, t) => {
                    let w = to_wide(&t);
                    SetWindowTextW(h, w.as_ptr());
                }
                Op::Invalidate(h) => {
                    InvalidateRect(h, std::ptr::null(), 0);
                }
                Op::Animate => {
                    SetTimer(parent, IDT_ANIM, 16, None);
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Painting
// ------------------------------------------------------------------------------------------------

fn color(t: &Theme, c: C) -> u32 {
    match c {
        C::Text => t.text,
        C::Text2 => t.text2,
        C::Text3 => t.text3,
        C::Accent => t.accent,
        C::Warn => t.warn_fg,
        C::Err => t.err_fg,
        C::Ok => t.ok_fg,
        C::Working => t.working,
    }
}

fn tone_colors(t: &Theme, tone: Tone) -> (u32, u32, char) {
    match tone {
        Tone::Info => (t.text2, t.hover, '\u{E946}'),
        Tone::Warn => (t.warn_fg, t.warn_bg, '\u{E7BA}'),
        Tone::Err => (t.err_fg, t.err_bg, '\u{EA39}'),
        Tone::Ok => (t.ok_fg, t.ok_bg, '\u{E73E}'),
    }
}

fn status_chip(m: &SharedUiModel, t: &Theme) -> (String, u32) {
    let rose = rgb(0xE1, 0x1D, 0x48);
    match &m.state {
        EngineState::Error(e) => (friendly_error(e).short.to_string(), rose),
        EngineState::Disabled => ("Paused".into(), rgb(0x8A, 0x94, 0x90)),
        EngineState::Listening => ("Listening…".into(), rgb(0x10, 0xB9, 0x81)),
        EngineState::Processing | EngineState::Correcting => ("Transcribing…".into(), t.working),
        // The model download is shown here so it stays visible on every page, not only on the models page.
        _ if m.download.as_ref().is_some_and(|d| d.is_active()) => {
            let d = m.download.as_ref().expect("checked above");
            let label = match (&d.stage, d.fraction()) {
                (DownloadStage::Verifying, _) => "Verifying model…".to_string(),
                (_, Some(f)) => format!("Downloading model {}%", (f * 100.0).floor() as u32),
                _ => "Downloading model…".to_string(),
            };
            (label, t.working)
        }
        _ if !m.active_model_ready() && m.download.as_ref().is_some_and(|d| matches!(d.stage, DownloadStage::Failed(_))) => {
            ("Model download failed".into(), rose)
        }
        _ if !m.active_model_ready() => ("Speech model needed".into(), rgb(0xF5, 0x9E, 0x0B)),
        _ => (format!("Ready · hold {}", m.config.ptt_hotkey.display_name), rgb(0x10, 0xB9, 0x81)),
    }
}

unsafe fn paint(st: &St, hdc: HDC) {
    let _pf = crate::perf::frame("settings");
    let t = theme();
    let d = st.dpi;
    let s = |v: i32| sc(v, d);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(st.hwnd, &mut rc);
    let Some(mut cv) = Canvas::new(rc.right, rc.bottom) else { return };
    cv.fill(t.bg);
    // Frameless: KeyReed's own header row across the top; both columns start below it.
    let head = s(HEADER_H);
    cv.rect(0, head, s(SIDEBAR_W), rc.bottom - head, t.sidebar);
    cv.rect(s(SIDEBAR_W), head, 1.max(s(1)), rc.bottom - head, t.border);
    cv.rect(0, head, rc.right, 1.max(s(1)), t.border);

    // The mark lives in the header now, where the stock title bar used to be.
    blit_icon(&mut cv, s(16), (head - s(20)) / 2, s(20) as usize);
    let ty = (head - s(20)) / 2;
    cv.text(PRODUCT_NAME, s(46), ty, s(160), s(20), st.fonts.get(F::Brand), t.text, DT_SINGLELINE | DT_VCENTER);
    let dot_x = s(46) + text_width(PRODUCT_NAME, st.fonts.get(F::Brand)) + s(8);
    cv.text("\u{00B7}", dot_x, ty, s(12), s(20), st.fonts.get(F::Body), t.text3, DT_SINGLELINE | DT_VCENTER);
    cv.text("Settings", dot_x + s(14), ty, s(140), s(20), st.fonts.get(F::BodySemi), t.text2, DT_SINGLELINE | DT_VCENTER);

    // Status chip at the bottom of the sidebar
    // Skipped when the window is so short (e.g. 200% scaling on a 1080p display) that the chip would overlap the last nav item.
    let nav_bottom = s(NAV_Y + NAV.len() as i32 * 38);
    if let Some(m) = snapshot().filter(|_| rc.bottom - s(52) >= nav_bottom) {
        let (label, dot) = status_chip(&m, &t);
        let y = rc.bottom - s(52);
        cv.round_rect(s(12) as f32, y as f32, s(SIDEBAR_W - 24) as f32, s(36) as f32, s(8) as f32, t.surface, 1.0);
        cv.round_rect_stroke(s(12) as f32, y as f32, s(SIDEBAR_W - 24) as f32, s(36) as f32, s(8) as f32, 1.0, t.border, 1.0);
        cv.circle(s(28) as f32, (y + s(18)) as f32, s(4) as f32, dot, 1.0);
        cv.text(&label, s(40), y, s(SIDEBAR_W - 64), s(36), st.fonts.get(F::Caption), t.text2, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }

    let off = st.scroll;
    let s = |v: i32| sc(v, d);
    let sy = |v: i32| sc(v - off, d);
    // Only the column right of the sidebar scrolls: clip the canvas to it so no scrolled row can paint over the
    // sidebar or past the window edges.
    cv.set_clip(s(SIDEBAR_W + 1), head, rc.right - s(SIDEBAR_W + 1), rc.bottom - head);
    // Running x for badges placed after a text (key: anchor text + y), see Item::Badge.
    let mut badge_x: Option<(&str, i32, i32)> = None;
    for it in &st.items {
        match it {
            Item::Text { r: rr, s: txt, font, color: c, flags } => {
                cv.text(txt, s(rr.x), sy(rr.y), s(rr.w), s(rr.h), st.fonts.get(*font), color(&t, *c), *flags);
            }
            Item::Group { r: rr } | Item::Card { r: rr } => {
                let (x, y, w, h) = (s(rr.x) as f32, sy(rr.y) as f32, s(rr.w) as f32, s(rr.h) as f32);
                cv.round_rect(x, y, w, h, s(8) as f32, t.surface, 1.0);
                cv.round_rect_stroke(x, y, w, h, s(8) as f32, 1.0, t.border, 1.0);
            }
            Item::Divider { x, y, w } => cv.rect(s(*x), sy(*y), s(*w), 1, t.divider),
            Item::Badge { x, y, s: txt, tone, after } => {
                let (fg, bg, _) = tone_colors(&t, *tone);
                let f = st.fonts.get(F::CaptionSemi);
                let tw = text_width(txt, f);
                let bw = tw + s(16);
                let bx = match after {
                    Some(a) => {
                        let start = match badge_x {
                            Some((pa, py, nx)) if pa == a.as_str() && py == *y => nx,
                            _ => s(*x) + text_width(a, st.fonts.get(F::BodySemi)) + s(10),
                        };
                        badge_x = Some((a.as_str(), *y, start + bw + s(6)));
                        start
                    }
                    None => s(*x),
                };
                let (by, bh) = (sy(*y), s(20));
                cv.round_rect(bx as f32, by as f32, bw as f32, bh as f32, (bh / 2) as f32, bg, 1.0);
                let fgc = if *tone == Tone::Info { t.text2 } else { fg };
                cv.text(txt, bx, by, bw, bh, f, fgc, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
            }
            Item::Meter { r: rr, level, active } => {
                let (x, y, w, h) = (s(rr.x) as f32, sy(rr.y) as f32, s(rr.w) as f32, s(rr.h) as f32);
                cv.round_rect(x, y, w, h, h / 2.0, t.track, 1.0);
                // Target zone (normal speech) shown as a faint band.
                if !t.hc {
                    cv.round_rect(x + w * 0.45, y, w * 0.40, h, h / 2.0, t.accent_soft, 1.0);
                }
                if *active || *level > 0.0 {
                    let lv = level.clamp(0.0, 1.0);
                    let c = if lv > 0.92 { rgb(0xF5, 0x9E, 0x0B) } else if lv > 0.40 { t.accent_fill } else { rgb(0x9C, 0xB3, 0xA8) };
                    if lv > 0.01 {
                        cv.round_rect(x, y, (w * lv).max(h), h, h / 2.0, c, 1.0);
                    }
                }
            }
            Item::Progress { r: rr, frac } => {
                let (x, y, w, h) = (s(rr.x) as f32, sy(rr.y) as f32, s(rr.w) as f32, s(rr.h) as f32);
                cv.round_rect(x, y, w, h, h / 2.0, t.track, 1.0);
                if let Some(f) = frac {
                    if *f > 0.0 {
                        cv.round_rect(x, y, (w * f).max(h), h, h / 2.0, t.accent_fill, 1.0);
                    }
                }
            }
            Item::Glyph { r: rr, ch, font, color: c } => {
                if *ch == '\u{0}' {
                    blit_icon(&mut cv, s(rr.x), sy(rr.y), s(rr.w) as usize);
                } else {
                    let g: String = ch.to_string();
                    cv.text(&g, s(rr.x), sy(rr.y), s(rr.w), s(rr.h), st.fonts.get(*font), color(&t, *c), DT_SINGLELINE | DT_LEFT | DT_VCENTER);
                }
            }
            Item::Keycap { x, y, s: txt } => {
                let f = st.fonts.get(F::BodySemi);
                let tw = text_width(txt, f);
                let (kx, ky, kw, kh) = (s(*x), sy(*y), tw + s(24), s(28));
                cv.round_rect(kx as f32, ky as f32, kw as f32, kh as f32, s(6) as f32, t.border, 1.0);
                cv.round_rect(kx as f32, ky as f32, kw as f32, (kh - s(3)) as f32, s(6) as f32, t.surface, 1.0);
                cv.round_rect_stroke(kx as f32, ky as f32, kw as f32, kh as f32, s(6) as f32, 1.0, t.border, 1.0);
                cv.text(txt, kx, ky, kw, kh - s(2), f, t.text, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
            }
            Item::Steps { r: rr, current, total } => {
                let gap = s(6);
                let segw = (s(rr.w) - gap * (*total as i32 - 1)) / *total as i32;
                for i in 0..*total {
                    let x = s(rr.x) + i as i32 * (segw + gap);
                    let c = if i <= *current { t.accent_fill } else { t.track };
                    cv.round_rect(x as f32, sy(rr.y) as f32, segw as f32, s(rr.h) as f32, s(2) as f32, c, 1.0);
                }
            }
            Item::Banner { r: rr, tone, title, body } => {
                let (fg, bg, glyph) = tone_colors(&t, *tone);
                let (x, y, w, h) = (s(rr.x), sy(rr.y), s(rr.w), s(rr.h));
                cv.round_rect(x as f32, y as f32, w as f32, h as f32, s(8) as f32, if *tone == Tone::Info { t.accent_soft } else { bg }, 1.0);
                let gc = if *tone == Tone::Info { t.accent } else { fg };
                cv.text(&glyph.to_string(), x + s(14), y + s(10), s(20), s(20), st.fonts.get(F::Icon), gc, DT_SINGLELINE);
                cv.text(title, x + s(44), y + s(8), w - s(56), s(20), st.fonts.get(F::BodySemi), if *tone == Tone::Info { t.text } else { fg }, DT_SINGLELINE | DT_END_ELLIPSIS);
                cv.text(body, x + s(44), y + s(29), w - s(56), h - s(30), st.fonts.get(F::Caption), t.text2, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
        }
    }
    // Text-box fields: recessed fill, 3:1 boundary, 2 px accent ring while the box has keyboard focus.
    let focus = GetFocus();
    for (c, h) in &st.ctrls {
        if let Kind::Edit { .. } = c.kind {
            let (x, y, w, hh) = (s(c.r.x) as f32, sy(c.r.y) as f32, s(c.r.w) as f32, s(c.r.h) as f32);
            let rad = s(6) as f32;
            cv.round_rect(x, y, w, hh, rad, t.bg, 1.0);
            if *h == focus {
                cv.round_rect_stroke(x, y, w, hh, rad, 2.0_f32.max(s(2) as f32), t.accent_fill, 1.0);
            } else {
                cv.round_rect_stroke(x, y, w, hh, rad, 1.0, t.control_border, 1.0);
            }
        }
    }
    // Slim scrollbar on the right edge of the content column: shows the page continues, and how far down.
    let view_h = (rc.bottom as f32 * 96.0 / d as f32).round() as i32;
    let max = max_scroll(st.content_h, view_h);
    if max > 0 {
        let (bar_x, bar_w, top) = (rc.right - s(8), s(4), s(8));
        let track_h = (rc.bottom - 2 * top).max(1);
        let thumb_h = ((track_h as f32 * view_h as f32 / st.content_h as f32) as i32).clamp(s(24), track_h);
        let thumb_y = top + (track_h - thumb_h) * st.scroll / max;
        let rad = (bar_w / 2) as f32;
        cv.round_rect(bar_x as f32, top as f32, bar_w as f32, track_h as f32, rad, t.track, 1.0);
        cv.round_rect(bar_x as f32, thumb_y as f32, bar_w as f32, thumb_h as f32, rad, t.text3, 1.0);
    }
    cv.clear_clip();
    // The confirmation card: the page dims behind it, and only the card's own buttons (child windows) stay live.
    if let Some(md) = MODAL.with(|m| m.borrow().clone()) {
        cv.round_rect(0.0, 0.0, rc.right as f32, rc.bottom as f32, 0.0, rgb(0, 0, 0), 0.55);
        let card = modal_card();
        let (x, y, w, h) = (s(card.x) as f32, s(card.y) as f32, s(card.w) as f32, s(card.h) as f32);
        cv.round_rect(x - s(6) as f32, y + s(4) as f32, w + s(12) as f32, h + s(14) as f32, s(16) as f32, rgb(0, 0, 0), 0.12);
        cv.round_rect(x, y, w, h, s(12) as f32, t.surface, 1.0);
        cv.round_rect_stroke(x, y, w, h, s(12) as f32, 1.0, t.border, 1.0);
        cv.text(&md.title, s(card.x + 24), s(card.y + 22), s(card.w - 48), s(24), st.fonts.get(F::Brand), t.text, DT_SINGLELINE | DT_END_ELLIPSIS);
        cv.text(&md.body, s(card.x + 24), s(card.y + 54), s(card.w - 48), s(62), st.fonts.get(F::Body), t.text2, DT_WORDBREAK);
    }
    cv.blit_to(hdc, 0, 0);
}

fn blit_icon(cv: &mut Canvas, x: i32, y: i32, n: usize) {
    let ic = scaled_icon(n);
    for yy in 0..n {
        for xx in 0..n {
            let p = ic[yy * n + xx];
            let a = ((p >> 24) & 0xFF) as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            // Un-premultiply to a straight colour, then blend with coverage `a`.
            let un = |sh: u32| (((p >> sh) & 0xFF) as f32 / a).round().clamp(0.0, 255.0) as u8;
            let col = rgb(un(16), un(8), un(0));
            cv.round_rect((x + xx as i32) as f32, (y + yy as i32) as f32, 1.0, 1.0, 0.0, col, a);
        }
    }
}

#[repr(C)]
struct DrawItem {
    ctl_type: u32,
    ctl_id: u32,
    item_id: u32,
    item_action: u32,
    item_state: u32,
    hwnd_item: HWND,
    hdc: HDC,
    rc: RECT,
    item_data: usize,
}

const ODS_SELECTED: u32 = 0x0001;
const ODS_DISABLED: u32 = 0x0004;
const ODS_FOCUS: u32 = 0x0010;
const ODS_NOFOCUSRECT: u32 = 0x0200;

unsafe fn draw_item(st: &St, di: &DrawItem) {
    let _pf = crate::perf::frame("settings_ctrl");
    let Some((c, _)) = st.ctrls.iter().find(|(c, _)| c.id == di.ctl_id as i32) else { return };
    let t = theme();
    let d = st.dpi;
    let s = |v: i32| sc(v, d);
    let w = di.rc.right - di.rc.left;
    let h = di.rc.bottom - di.rc.top;
    let Some(mut cv) = Canvas::new(w, h) else { return };
    let bg = match c.bg {
        Bg::Sidebar => t.sidebar,
        Bg::Content => t.bg,
        Bg::Surface => t.surface,
    };
    cv.fill(bg);
    let pressed = di.item_state & ODS_SELECTED != 0;
    let disabled = di.item_state & ODS_DISABLED != 0;
    let focus = di.item_state & ODS_FOCUS != 0 && di.item_state & ODS_NOFOCUSRECT == 0;
    let hot = st.hot == c.id && !disabled;
    let (wf, hf) = (w as f32, h as f32);
    let rad = s(6) as f32;
    let mut focus_rect = (1.0, 1.0, wf - 2.0, hf - 2.0, rad);

    match &c.kind {
        Kind::Nav { page, selected } => {
            if *selected {
                cv.round_rect(0.0, 0.0, wf, hf, rad, t.accent_soft, 1.0);
                cv.round_rect(0.0, (hf - s(16) as f32) / 2.0, s(3) as f32, s(16) as f32, s(2) as f32, t.accent_fill, 1.0);
            } else if hot || pressed {
                cv.round_rect(0.0, 0.0, wf, hf, rad, t.hover, 1.0);
            }
            let (label, glyph) = NAV[*page];
            let gc = if *selected { t.accent } else { t.text2 };
            cv.text(&glyph.to_string(), s(12), 0, s(20), h, st.fonts.get(F::Icon), gc, DT_SINGLELINE | DT_VCENTER);
            let f = if *selected { F::BodySemi } else { F::Body };
            cv.text(label, s(44), 0, w - s(48), h, st.fonts.get(f), t.text, DT_SINGLELINE | DT_VCENTER);
        }
        Kind::Toggle { on } => {
            let (sw, sh) = (s(40) as f32, s(20) as f32);
            let sx = wf - sw - s(2) as f32;
            let sy = (hf - sh) / 2.0;
            // amount = 0 fully off .. 1 fully on; between the two only while a slide is running.
            let p = toggle_progress(st, c.id);
            let amount = if *on { p } else { 1.0 - p };
            let fill = if disabled { t.track } else if pressed || hot { t.accent_press } else { t.accent_fill };
            if amount < 1.0 {
                if hot {
                    cv.round_rect(sx, sy, sw, sh, sh / 2.0, t.hover_card, 1.0);
                }
                cv.round_rect_stroke(sx, sy, sw, sh, sh / 2.0, 1.0_f32.max(s(1) as f32), if disabled { t.track } else { t.text3 }, 1.0);
            }
            if amount > 0.0 {
                cv.round_rect(sx, sy, sw, sh, sh / 2.0, fill, amount);
            }
            let kx = sx + sh / 2.0 + (sw - sh) * amount;
            let kr = s(5) as f32 + (s(7) - s(5)) as f32 * amount;
            let off_knob = if disabled { t.track } else { t.text2 };
            cv.circle(kx, sy + sh / 2.0, kr, off_knob, 1.0 - amount);
            cv.circle(kx, sy + sh / 2.0, kr, t.surface, amount);
            focus_rect = (sx - s(3) as f32, sy - s(3) as f32, sw + s(6) as f32, sh + s(6) as f32, (sh + s(6) as f32) / 2.0);
        }
        Kind::Choice { title, desc, selected } => {
            let r8 = s(8) as f32;
            let fill = if *selected { t.accent_soft } else if hot || pressed { t.hover_card } else { t.surface };
            cv.round_rect(0.0, 0.0, wf, hf, r8, fill, 1.0);
            let (bc, bw) = if *selected { (t.accent_fill, 1.5_f32 * d as f32 / 96.0) } else { (t.border, 1.0) };
            cv.round_rect_stroke(0.0, 0.0, wf, hf, r8, bw, bc, 1.0);
            let (rx, ry) = (s(22) as f32, s(22) as f32);
            if *selected {
                cv.circle(rx, ry, s(9) as f32, t.accent_fill, 1.0);
                cv.circle(rx, ry, s(4) as f32, t.surface, 1.0);
            } else {
                cv.circle(rx, ry, s(9) as f32, t.text3, 1.0);
                cv.circle(rx, ry, s(9) as f32 - 1.25_f32.max(s(1) as f32), fill, 1.0);
            }
            cv.text(title, s(42), s(12), w - s(54), s(20), st.fonts.get(F::BodySemi), t.text, DT_SINGLELINE | DT_END_ELLIPSIS);
            cv.text(desc, s(42), s(34), w - s(54), h - s(36), st.fonts.get(F::Caption), t.text2, DT_WORDBREAK);
            focus_rect = (s(2) as f32, s(2) as f32, wf - s(4) as f32, hf - s(4) as f32, r8 - 1.0);
        }
        Kind::Button { style, label } => {
            let (fill, stroke, fg) = match (style, disabled) {
                (BtnStyle::Status, _) => (None, None, t.ok_fg),
                (BtnStyle::Subtle, true) => (None, None, t.text3),
                (_, true) => (Some(t.disabled_bg), None, t.text3),
                (BtnStyle::Primary, _) => (Some(if pressed { t.accent_press } else if hot { t.accent } else { t.accent_fill }), None, t.on_accent),
                (BtnStyle::Secondary, _) => (Some(if pressed { t.hover } else if hot { t.hover_card } else { t.surface }), Some(t.control_border), t.text),
                (BtnStyle::Danger, _) => (Some(if pressed { t.err_bg } else if hot { t.err_bg } else { t.surface }), Some(t.control_border), t.err_fg),
                (BtnStyle::Subtle, _) => (if hot || pressed { Some(t.hover) } else { None }, None, t.accent),
            };
            if let Some(f) = fill {
                cv.round_rect(0.0, 0.0, wf, hf, rad, f, 1.0);
            }
            if let Some(sc_) = stroke {
                cv.round_rect_stroke(0.0, 0.0, wf, hf, rad, 1.0, sc_, 1.0);
            }
            if *style == BtnStyle::Status {
                // Check mark + label, right-aligned with the buttons in the rows above and below.
                let f = st.fonts.get(F::BodySemi);
                let tw = text_width(label, f);
                let x = w - tw - s(4);
                cv.text("\u{E73E}", x - s(22), 0, s(18), h, st.fonts.get(F::Icon), fg, DT_SINGLELINE | DT_VCENTER);
                cv.text(label, x, 0, tw + s(4), h, f, fg, DT_SINGLELINE | DT_VCENTER);
            } else {
                let f = if *style == BtnStyle::Primary { F::BodySemi } else { F::Body };
                cv.text(label, 0, 0, w, h, st.fonts.get(f), fg, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
            }
        }
        Kind::Combo { items, sel } => {
            let fill = if disabled { t.disabled_bg } else if pressed { t.hover } else if hot { t.hover_card } else { t.bg };
            cv.round_rect(0.0, 0.0, wf, hf, rad, fill, 1.0);
            cv.round_rect_stroke(0.0, 0.0, wf, hf, rad, 1.0, if disabled { t.border } else { t.control_border }, 1.0);
            let fg = if disabled { t.text3 } else { t.text };
            let label = items.get(*sel).map(String::as_str).unwrap_or("");
            cv.text(label, s(12), 0, w - s(44), h, st.fonts.get(F::Body), fg, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            // Chevron (Segoe Fluent Icons / MDL2 ChevronDown), nudged down while pressed.
            let nudge = if pressed { s(1) } else { 0 };
            cv.text("\u{E70D}", w - s(32), nudge, s(20), h, st.fonts.get(F::IconSmall), if disabled { t.text3 } else { t.text2 }, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        }
        Kind::Shortcut { spec, .. } => {
            // `capture`: None when idle; Some(held modifiers, may be empty) while this field is recording.
            let capture = CAPTURE.with(|k| k.borrow().as_ref().filter(|(i, _)| *i == c.id).map(|(_, s)| s.held_label()));
            let fill = if disabled { t.disabled_bg } else if capture.is_some() { t.surface } else if pressed { t.hover } else if hot { t.hover_card } else { t.bg };
            cv.round_rect(0.0, 0.0, wf, hf, rad, fill, 1.0);
            match capture {
                Some(_) => cv.round_rect_stroke(0.0, 0.0, wf, hf, rad, 1.5 * d as f32 / 96.0, t.accent_fill, 1.0),
                None => cv.round_rect_stroke(0.0, 0.0, wf, hf, rad, 1.0, if disabled { t.border } else { t.control_border }, 1.0),
            }
            let right_w = s(capture.as_ref().map_or(28, |_| 84));
            let (text, more) = match &capture {
                None => (spec.display_name.clone(), false),
                Some(held) if held.is_empty() => (String::new(), false),
                Some(held) => (held.trim_end_matches(" + ...").to_string(), true),
            };
            let f = st.fonts.get(F::CaptionSemi);
            let parts: Vec<&str> = if text.is_empty() { Vec::new() } else { text.split(" + ").collect() };
            let (pad, gap, kh) = (s(8), s(4), s(22));
            let widths: Vec<i32> = parts.iter().map(|p| text_width(p, f) + 2 * pad).collect();
            let total: i32 = widths.iter().sum::<i32>() + gap * parts.len().saturating_sub(1) as i32;
            let mut x = s(8);
            if parts.is_empty() {
                cv.text("Press your shortcut", x, 0, w - right_w - x, h, st.fonts.get(F::Body), t.text2, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            } else if total > w - right_w - x {
                cv.text(&text, x, 0, w - right_w - x, h, st.fonts.get(F::Body), t.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            } else {
                // Each key as a small keycap, the same look as the keycaps elsewhere on the page.
                let ky = (h - kh) / 2;
                for (p, kw) in parts.iter().zip(&widths) {
                    let (fx, fy, fw, fh, r6) = (x as f32, ky as f32, *kw as f32, kh as f32, s(5) as f32);
                    cv.round_rect(fx, fy, fw, fh, r6, t.border, 1.0);
                    cv.round_rect(fx, fy, fw, fh - s(2) as f32, r6, t.surface, 1.0);
                    cv.round_rect_stroke(fx, fy, fw, fh, r6, 1.0, t.border, 1.0);
                    cv.text(p, x, ky, *kw, kh - s(2), f, if disabled { t.text3 } else { t.text }, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
                    x += kw + gap;
                }
                if more {
                    cv.text("+ …", x, 0, s(44), h, st.fonts.get(F::Body), t.text2, DT_SINGLELINE | DT_VCENTER);
                }
            }
            if capture.is_some() {
                cv.text("Esc cancels", w - right_w, 0, right_w - s(10), h, st.fonts.get(F::Caption), t.text3, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
            } else {
                // Pencil (Segoe Fluent Icons / MDL2 Edit).
                cv.text("\u{E70F}", w - right_w, 0, right_w - s(6), h, st.fonts.get(F::IconSmall), if disabled { t.text3 } else { t.text2 }, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            }
        }
        Kind::WinBtn { close } => {
            let (fill, fg) = match (*close, pressed || hot) {
                (_, false) => (None, t.text2),
                (true, _) => (Some(t.err_bg), t.err_fg),
                (false, _) => (Some(t.hover), t.text),
            };
            if let Some(f) = fill {
                cv.round_rect(0.0, 0.0, wf, hf, rad, f, 1.0);
            }
            // Segoe Fluent Icons / MDL2: ChromeMinimize, ChromeClose.
            let glyph = if *close { "\u{E8BB}" } else { "\u{E921}" };
            cv.text(glyph, 0, 0, w, h, st.fonts.get(F::Icon), fg, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
        }
        _ => {}
    }
    if focus {
        let (x, y, fw, fh, fr) = focus_rect;
        // Emerald ring; on a Primary button (filled with the same emerald) it is drawn in the label colour, one step in.
        let primary = matches!(c.kind, Kind::Button { style: BtnStyle::Primary, .. });
        let (ring, inset) = if t.hc { (t.focus, 0.0) } else if primary { (t.on_accent, s(2) as f32) } else { (t.focus_ring, 0.0) };
        cv.round_rect_stroke(x + inset, y + inset, fw - 2.0 * inset, fh - 2.0 * inset, (fr - inset).max(1.0), 2.0_f32.max(s(2) as f32), ring, 1.0);
    }
    // An owner-drawn control paints into its own window, whose coordinates start at its own top edge, so a
    // control scrolled under the header is cut off here instead of hanging over it.
    let cut = if fixed_in_place(&c.kind) { 0 } else { sc(HEADER_H + 1, d) - sc(child_rect(c).y - st.scroll, d) };
    let saved = if cut > 0 { SaveDC(di.hdc) } else { 0 };
    if cut > 0 {
        IntersectClipRect(di.hdc, 0, cut, 1 << 20, 1 << 20);
    }
    cv.blit_to(di.hdc, di.rc.left, di.rc.top);
    if cut > 0 {
        RestoreDC(di.hdc, saved);
    }
}

// ------------------------------------------------------------------------------------------------
// Commands
// ------------------------------------------------------------------------------------------------

/// The part of a control still inside the scrolling column (right of the sidebar, below the header), in the
/// control's OWN coordinates: `SetWindowRgn` takes window-relative pixels, so the control's client position
/// has to be subtracted. Passing client coordinates clipped every control away for good.
fn child_region(x: i32, y: i32, w: i32, h: i32, view: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (left, top, right, bottom) = view;
    let (x0, y0) = (x.max(left) - x, y.max(top) - y);
    let (x1, y1) = ((x + w).min(right) - x, (y + h).min(bottom) - y);
    // Scrolled out of view: nothing of the control is left, so it is neither painted nor clickable.
    if x1 <= x0 || y1 <= y0 { (0, 0, 0, 0) } else { (x0, y0, x1, y1) }
}

/// Where each content control is visible right now, in its own coordinates. Computed without calling the
/// system, so it can run while `ST` is borrowed; `apply_regions` does the Win32 calls afterwards.
fn child_regions(st: &St) -> Vec<(HWND, (i32, i32, i32, i32))> {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(st.hwnd, &mut rc) };
    let left = sc(SIDEBAR_W + 1, st.dpi);
    // The page starts under the header: a control scrolled up must not paint over it.
    let top = sc(HEADER_H + 1, st.dpi);
    st.ctrls
        .iter()
        // Navigation and the header's own buttons stay put; clipping them to the page column would hide them.
        .filter(|(c, _)| !fixed_in_place(&c.kind))
        .map(|(c, h)| {
            let cr = child_rect(c);
            let (x, y) = (sc(cr.x, st.dpi), sc(cr.y - st.scroll, st.dpi));
            (*h, child_region(x, y, sc(cr.w, st.dpi), sc(cr.h, st.dpi), (left, top, rc.right, rc.bottom)))
        })
        .collect()
}

/// A control scrolled out of view is clipped to nothing, so it is neither painted nor clickable, and one that
/// scrolls back in is whole again.
unsafe fn apply_regions(regions: &[(HWND, (i32, i32, i32, i32))]) {
    for (h, (x0, y0, x1, y1)) in regions {
        // The system owns the region after a successful SetWindowRgn and frees it when it is replaced.
        let rgn = CreateRectRgn(*x0, *y0, (*x1).max(*x0), (*y1).max(*y0));
        if SetWindowRgn(*h, rgn, 1) == 0 {
            DeleteObject(rgn as _);
        }
    }
}

/// Scrolls the page so the control with keyboard focus is visible (Tab can move focus below the fold).
fn reveal_focused() {
    let f = unsafe { GetFocus() };
    let delta = ST.with(|s| {
        let b = s.try_borrow().ok()?;
        let st = b.as_ref()?;
        let (ctrl, _) = st.ctrls.iter().find(|(_, h)| *h == f)?;
        reveal_delta(ctrl.r.y, ctrl.r.h, st.scroll, st.view_h)
    });
    if let Some(delta) = delta {
        scroll_by(delta);
    }
}

/// The wheel, wherever it lands. Windows sends the wheel to the focused control, so every control forwards
/// it here instead of swallowing it (that is why the page only scrolled with the pointer on bare canvas).
fn wheel_scroll(wparam: WPARAM) -> LRESULT {
    let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    scroll_by(-(delta * WHEEL_STEP) / 120);
    0
}

/// While a confirmation card is up, the page's controls are hidden: the card's buttons are the only ones.
unsafe fn show_children(st: &St) {
    let card = MODAL.with(|m| m.borrow().is_some());
    for (c, h) in &st.ctrls {
        let own = matches!(c.id, MODAL_CANCEL | MODAL_CONFIRM);
        // A card hides the page, not the frame: the navigation and the header's window buttons stay visible
        // (their commands are already refused while a card is up), so the window never looks broken.
        let show = if card { own || fixed_in_place(&c.kind) } else { !own };
        ShowWindow(*h, if show { SW_SHOWNA } else { SW_HIDE });
    }
}

/// Scrolls the content column (only possible when the window is shorter than the layout).
fn scroll_by(delta: i32) {
    let moved = ST.with(|s| {
        let mut sb = s.borrow_mut();
        let st = sb.as_mut()?;
        // The client height is the truth, in design units (a resize may not have reached WM_SIZE yet).
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(st.hwnd, &mut rc) };
        st.view_h = (rc.bottom as f32 * 96.0 / st.dpi as f32).round() as i32;
        let new = next_scroll(st.scroll, delta, st.content_h, st.view_h);
        if new == st.scroll {
            return None;
        }
        st.scroll = new;
        let moves: Vec<(HWND, i32, i32)> = st
            .ctrls
            .iter()
            .filter(|(c, _)| !fixed_in_place(&c.kind))
            .map(|(c, h)| {
                let cr = child_rect(c);
                (*h, sc(cr.x, st.dpi), sc(cr.y - new, st.dpi))
            })
            .collect();
        Some((st.hwnd, moves, child_regions(st)))
    });
    if let Some((hwnd, moves, regions)) = moved {
        unsafe {
            // One deferred batch: every control lands in the same frame as the repaint, so no row tears.
            let mut hdwp = BeginDeferWindowPos(moves.len() as i32);
            for (h, x, y) in moves {
                if hdwp.is_null() {
                    SetWindowPos(h, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                } else {
                    hdwp = DeferWindowPos(hdwp, h, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            if !hdwp.is_null() {
                EndDeferWindowPos(hdwp);
            }
            apply_regions(&regions);
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }
}

fn combo_of(id: i32) -> Option<(HWND, Vec<String>, usize)> {
    ST.with(|s| {
        let b = s.try_borrow().ok()?;
        b.as_ref()?.ctrls.iter().find_map(|(c, h)| match &c.kind {
            Kind::Combo { items, sel } if c.id == id => Some((*h, items.clone(), *sel)),
            _ => None,
        })
    })
}

fn combo_sel(id: i32) -> usize {
    combo_of(id).map(|(_, _, sel)| sel).unwrap_or(0)
}

/// Selects `new` in drop-down `id` and runs the same handler a system combo box would (CBN_SELCHANGE).
fn set_combo_sel(id: i32, new: usize) {
    let changed = ST.with(|s| {
        let mut b = s.try_borrow_mut().ok()?;
        let (c, h) = b.as_mut()?.ctrls.iter_mut().find(|(c, _)| c.id == id)?;
        match &mut c.kind {
            Kind::Combo { items, sel } if *sel != new && new < items.len() => {
                *sel = new;
                Some(*h)
            }
            _ => None,
        }
    });
    if let Some(h) = changed {
        unsafe { InvalidateRect(h, std::ptr::null(), 0) };
        on_command(id, 1 /* CBN_SELCHANGE */);
    }
}

/// Opens the choices of drop-down `id` as a native popup menu under the field (keyboard, screen
/// readers and the Windows 11 menu style come from the system). The current choice is checked.
fn open_select(id: i32) {
    let Some((h, items, sel)) = combo_of(id) else { return };
    let pick = unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for (i, it) in items.iter().enumerate() {
            let w = to_wide(it);
            AppendMenuW(menu, MF_STRING | if i == sel { MF_CHECKED } else { 0 }, i + 1, w.as_ptr());
        }
        let mut rc: RECT = std::mem::zeroed();
        GetWindowRect(h, &mut rc);
        let tpm = TPMPARAMS { cbSize: std::mem::size_of::<TPMPARAMS>() as u32, rcExclude: rc };
        let pick = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_VERTICAL, rc.left, rc.bottom, hwnd(), &tpm);
        DestroyMenu(menu);
        pick
    };
    if pick > 0 {
        set_combo_sel(id, pick as usize - 1);
    }
}

fn open_url(url: &str) {
    unsafe {
        let op = to_wide("open");
        let u = to_wide(url);
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), u.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

fn set_step(step: usize) {
    harvest_and_commit();
    ST.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.step = step;
        }
    });
    rebuild(false);
    // Put keyboard focus on the primary action of the new step.
    ST.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            if let Some((_, h)) = st.ctrls.iter().find(|(c, _)| c.id == if step == 3 { S_PRACTICE } else { S_NEXT }) {
                unsafe {
                    SetFocus(*h);
                }
            }
        }
    });
    maybe_arm_advance();
}

/// Setup step 3 only reports progress: once the model is ready it shows that for a beat, then moves on by itself.
fn maybe_arm_advance() {
    let ready = snapshot().is_some_and(|m| m.active_model_ready());
    let target = ST.with(|s| {
        let b = s.try_borrow().ok()?;
        let st = b.as_ref()?;
        (st.page == PAGE_START && st.step == 2).then_some(st.hwnd)
    });
    if let (true, Some(h)) = (ready, target) {
        if !ADVANCE_ARMED.with(|c| c.replace(true)) {
            unsafe { SetTimer(h, IDT_ADVANCE, ADVANCE_MS, None) };
        }
    }
}

/// Remembers why the last practice attempt produced no text, for the hint under the practice box.
fn track_practice(m: &SharedUiModel) {
    let note = match &m.state {
        EngineState::Listening => Some(0),
        EngineState::Error(_) => Some(3),
        EngineState::ReadyToPaste => match &m.last_outcome {
            Some(ReadyOutcome::NothingHeard) => Some(1),
            Some(o) if o.is_success() => Some(0),
            _ => None,
        },
        _ => None,
    };
    if let Some(n) = note {
        PRACTICE_NOTE.with(|c| c.set(n));
    }
}

fn selected_mic_from_combo(id: i32) -> Option<String> {
    let m = snapshot()?;
    let sel = combo_sel(id);
    if sel == 0 {
        None
    } else if let Some(name) = m.available_microphones.get(sel - 1) {
        Some(name.clone())
    } else {
        m.config.selected_microphone.clone() // "(not connected)" entry: keep the preference
    }
}

fn on_command(id: i32, code: u32) {
    const BN_CLICKED: u32 = 0;
    const BN_DOUBLECLICKED: u32 = 5;
    const CBN_SELCHANGE: u32 = 1;
    const EN_KILLFOCUS: u32 = 0x0200;
    const EN_SETFOCUS: u32 = 0x0100;
    const EN_CHANGE: u32 = 0x0300;
    // A confirmation card owns the window: nothing on the page behind it reacts until it is answered.
    if MODAL.with(|m| m.borrow().is_some()) && !matches!(id, MODAL_CANCEL | MODAL_CONFIRM) {
        return;
    }
    let click = code == BN_CLICKED || code == BN_DOUBLECLICKED;
    if code == EN_SETFOCUS || code == EN_KILLFOCUS {
        unsafe { InvalidateRect(hwnd(), std::ptr::null(), 0) };
    }
    if code == EN_KILLFOCUS && is_dict_edit(id) {
        unsafe { PostMessageW(hwnd(), WM_DICT_COMMIT, 0, 0) };
        return;
    }
    if code == EN_CHANGE && id == S_PRACTICE {
        // Setup step 4 turns into its success state as soon as the box holds the dictated text.
        let heard = !read_text(unsafe { GetDlgItem(hwnd(), S_PRACTICE) }).is_empty();
        if PRACTICE_HEARD.with(|c| c.replace(heard)) != heard {
            refresh();
        }
        return;
    }
    if click && combo_of(id).is_some() {
        open_select(id);
        return;
    }
    let Some(m) = snapshot() else { return };
    let c = &m.config;

    if (ID_NAV..ID_NAV + NAV.len() as i32).contains(&id) && click {
        let p = (id - ID_NAV) as usize;
        if p == PAGE_START {
            ST.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    st.step = 0;
                }
            });
        }
        navigate(p);
        return;
    }
    if (W_MODE..W_MODE + 6).contains(&id) && click {
        let mode = WritingMode::ALL[(id - W_MODE) as usize];
        commit(|c| c.default_writing_mode = mode);
        return;
    }
    // Every catalogue row, not the first six: SenseVoice and Whisper's buttons were dead.
    if (M_ACT..M_ACT + ASR_UI_CATALOG.len() as i32).contains(&id) && click {
        model_action(&m, ASR_UI_CATALOG[(id - M_ACT) as usize].id);
        return;
    }
    if id == M_GRAMMAR && click {
        let downloading = m
            .download
            .as_ref()
            .map(|d| d.is_active() && d.model_id.eq_ignore_ascii_case(GRAMMAR_MODEL_ID))
            .unwrap_or(false);
        if downloading {
            send(UiAction::CancelModelDownload);
        } else if !m.is_installed(GRAMMAR_MODEL_ID) {
            send(UiAction::DownloadModel(GRAMMAR_MODEL_ID.to_string()));
        }
        return;
    }
    if (P_PROF..P_PROF + 3).contains(&id) && click {
        let p = [ProcessingProfile::MaxPrivacyOffline, ProcessingProfile::Balanced, ProcessingProfile::MaxAccuracy][(id - P_PROF) as usize];
        commit(|c| c.processing_profile = p);
        return;
    }

    match (id, click, code) {
        (HEADER_MIN, true, _) => unsafe { ShowWindow(hwnd(), SW_MINIMIZE); },
        // The same path as the system close button.
        (HEADER_CLOSE, true, _) => unsafe { PostMessageW(hwnd(), WM_CLOSE, 0, 0); },
        (MODAL_CANCEL, true, _) => close_modal(),
        (MODAL_CONFIRM, true, _) => {
            let action = MODAL.with(|m| m.borrow_mut().take().map(|md| md.action)).unwrap_or(ModalAction::Notice);
            close_modal();
            match action {
                // A fresh store: the coordinator puts the learning switch back from the setting.
                ModalAction::DeleteLearned => commit_style(|s| *s = Default::default()),
                ModalAction::ResetAll => {
                    send(UiAction::ResetToDefaults);
                    flash_saved();
                }
                ModalAction::Notice => {}
            }
        }
        (G_ENABLED, true, _) => send(UiAction::ToggleAppEnabled),
        (G_AUTOSTART, true, _) => {
            let v = !c.autostart_on_login;
            commit(|c| c.autostart_on_login = v)
        }
        (G_PILL, true, _) => {
            let v = !c.show_floating_mic;
            commit(|c| c.show_floating_mic = v)
        }
        (G_PILL_CLICK, _, CBN_SELCHANGE) => {
            let a = vi_core::PillClickAction::ALL.get(combo_sel(G_PILL_CLICK)).copied().unwrap_or_default();
            commit(|c| c.pill_click_action = a)
        }
        (id, _, CBN_SELCHANGE) if (D_MOUSE..D_MOUSE + 3).contains(&id) => {
            let a = MouseAction::ALL.get(combo_sel(id)).copied().unwrap_or_default();
            commit(|c| match id - D_MOUSE {
                0 => c.mouse_middle_action = a,
                1 => c.mouse_back_action = a,
                _ => c.mouse_forward_action = a,
            })
        }
        (G_PILL_RESET, true, _) => unsafe { crate::ui::win_gui::reset_pill_position() },
        (G_SOUNDS, true, _) => {
            let v = !c.audio_cues_enabled;
            commit(|c| c.audio_cues_enabled = v)
        }
        (G_NOTIFY, true, _) => {
            let v = !c.notify_on_ready;
            commit(|c| c.notify_on_ready = v)
        }
        (A_CUE, _, CBN_SELCHANGE) => {
            let s = AudioCueStyle::ALL.get(combo_sel(A_CUE)).copied().unwrap_or_default();
            commit(|c| c.audio_cue_style = s)
        }
        (A_VOL, _, CBN_SELCHANGE) => {
            let v = (combo_sel(A_VOL) * 5).min(100) as u8;
            commit(|c| c.audio_cue_volume = v)
        }
        (A_PLAY, true, _) => send(UiAction::PreviewAudioCue),
        (D_HOTKEY, true, _) | (D_QF_KEY, true, _) | (D_VOICE_KEY, true, _) | (D_COMPOSER_KEY, true, _) => {
            start_capture(id)
        }
        (D_HOTKEY, _, CBN_SELCHANGE) | (S_HOTKEY, _, CBN_SELCHANGE) => {
            if let Some((spec, _)) = hotkey_presets().get(combo_sel(id)) {
                let spec = spec.clone();
                commit(|c| c.ptt_hotkey = spec);
            }
        }
        (D_QF_REVIEW, true, _) => {
            let v = !c.quick_fix_always_review;
            commit(|c| c.quick_fix_always_review = v)
        }
        (D_PTT, true, _) => send(UiAction::TogglePttEnabled),
        (D_HOLD, true, _) | (S_HOLD, true, _) => commit(|c| c.activation_mode = ActivationMode::HoldToTalk),
        (D_TOGGLE, true, _) | (S_TOGGLE, true, _) => commit(|c| c.activation_mode = ActivationMode::ToggleToTalk),
        (W_AUTOCTX, true, _) => {
            let v = !c.auto_context_mode;
            commit(|c| c.auto_context_mode = v)
        }
        (W_VOCAB, true, _) => {
            vocab_goto(0, false);
            focus_ctrl(V_TERM);
        }
        (V_DONE, true, _) => close_vocab(),
        (V_ADD, true, _) => {
            harvest_and_commit();
            let n = dict_len();
            vocab_goto(n / DICT_ROWS, true);
            focus_ctrl(V_TERM + (n % DICT_ROWS) as i32);
        }
        (V_PREV, true, _) | (V_NEXT, true, _) => {
            harvest_and_commit();
            let page = VOCAB_VIEW.with(|v| v.get()).unwrap_or(0);
            vocab_goto(if id == V_PREV { page.saturating_sub(1) } else { page + 1 }, VOCAB_DRAFT.with(|v| v.get()));
            focus_ctrl(V_TERM);
        }
        (i, true, _) if (V_DEL..V_DEL + DICT_ROWS as i32).contains(&i) => {
            harvest_and_commit();
            let abs = VOCAB_VIEW.with(|v| v.get()).unwrap_or(0) * DICT_ROWS + (i - V_DEL) as usize;
            commit(|c| remove_dict_entry(&mut c.dictionary, abs));
        }
        (Y_LEARN, true, _) => {
            let v = !c.learn_from_edits;
            commit(|c| c.learn_from_edits = v)
        }
        (Y_DIALECT, _, CBN_SELCHANGE) => {
            let d = [Dialect::Auto, Dialect::British, Dialect::American].get(combo_sel(Y_DIALECT)).copied().unwrap_or_default();
            commit(|c| c.dialect = d)
        }
        (i, true, _) if (Y_ITEM..Y_ITEM + STYLE_ITEMS_PER_PAGE as i32).contains(&i) => {
            let page = STYLE_PAGE.with(|p| p.get());
            let Some(item) = m.style.items.get(page * STYLE_ITEMS_PER_PAGE + (i - Y_ITEM) as usize).cloned() else { return };
            commit_style(|s| {
                s.remove(&item.key, item.scope, &item.value);
            });
        }
        (i, true, _) if (Y_CAND..Y_CAND + STYLE_CANDIDATES_SHOWN as i32).contains(&i) => {
            let Some(cand) = m.style.vocabulary_candidates.get((i - Y_CAND) as usize).cloned() else { return };
            commit(|c| {
                c.dictionary.add_term(cand.to.clone());
                dict_sync_enabled(&mut c.dictionary);
            });
            commit_style(|s| {
                s.remove_candidate(&cand.from, &cand.to);
            });
        }
        (i, true, _) if (Y_CAND_DISMISS..Y_CAND_DISMISS + STYLE_CANDIDATES_SHOWN as i32).contains(&i) => {
            let Some(cand) = m.style.vocabulary_candidates.get((i - Y_CAND_DISMISS) as usize).cloned() else { return };
            commit_style(|s| {
                s.remove_candidate(&cand.from, &cand.to);
            });
        }
        (i, true, _) if (Y_APP..Y_APP + STYLE_APPS_SHOWN as i32).contains(&i) => {
            let Some(app) = m.style.excluded_apps.get((i - Y_APP) as usize).cloned() else { return };
            commit_style(|s| {
                s.include_app(&app);
            });
        }
        (Y_PREV, true, _) | (Y_NEXT, true, _) => {
            let page = STYLE_PAGE.with(|p| p.get());
            let last = m.style.items.len().div_ceil(STYLE_ITEMS_PER_PAGE).saturating_sub(1);
            let next = if id == Y_PREV { page.saturating_sub(1) } else { (page + 1).min(last) };
            STYLE_PAGE.with(|p| p.set(next));
            refresh();
        }
        (Y_RESET, true, _) => commit_style(|s| s.reset_learned()),
        (Y_EXPORT, true, _) => {
            let path = vi_core::style::StyleStore::default_path().with_file_name("style-export.json");
            match m.style.save_atomic(&path) {
                Ok(()) => {
                    open_url(&path.to_string_lossy());
                    flash_saved();
                }
                Err(_) => ask(
                    "The export failed",
                    "KeyReed could not write style-export.json to its folder. Nothing was changed.",
                    "OK",
                    ModalAction::Notice,
                ),
            }
        }
        (Y_DELETE, true, _) => ask_delete_learned(),
        (MIC_PRIVACY, true, _) => open_url("ms-settings:privacy-microphone"),
        (O_TYPE, true, _) => commit(|c| c.output_mode = OutputMode::DirectTyping),
        (O_CLIP, true, _) => commit(|c| c.output_mode = OutputMode::Clipboard),
        (O_ASK, true, _) => commit(|c| c.output_mode = OutputMode::AskBeforeInserting),
        (O_COPYLAST, true, _) => send(UiAction::CopyLastDictation),
        (A_MIC, _, CBN_SELCHANGE) | (S_MIC, _, CBN_SELCHANGE) => {
            let sel = selected_mic_from_combo(id);
            commit(|c| c.selected_microphone = sel);
        }
        (A_TEST, true, _) | (S_TEST, true, _) => {
            let mic = if id == A_TEST { selected_mic_from_combo(A_MIC) } else { selected_mic_from_combo(S_MIC) };
            if let Some(g) = globals() {
                let mut mm = g.model.lock();
                mm.mic_test = crate::ui::MicTestStatus { running: true, ..Default::default() };
            }
            send(UiAction::TestMicrophone(mic));
            refresh();
        }
        (P_KEY_SAVE, true, _) => {
            let h = unsafe { GetDlgItem(hwnd(), P_KEY) };
            let key = read_text(h);
            if !key.trim().is_empty() {
                if let Some(g) = globals() {
                    let cfg = g.model.lock().config.clone();
                    send(UiAction::SaveUpdatedConfig { new_cfg: Box::new(cfg), new_plaintext_api_key: Some(key.trim().to_string()) });
                }
                unsafe {
                    let empty = to_wide("");
                    SetWindowTextW(h, empty.as_ptr());
                }
                ST.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        st.key_draft.clear();
                    }
                });
                flash_saved();
            }
        }
        (P_KEY_REMOVE, true, _) => commit(|c| c.cloud.encrypted_api_key_b64 = None),
        (P_LLM, _, CBN_SELCHANGE) => {
            let v = if combo_sel(P_LLM) == 1 { "none".to_string() } else { "qwen3.5-0.8b-q4_k_m".to_string() };
            commit(|c| c.local_llm_model = v);
        }
        (V_PWD, true, _) => {
            let v = !c.privacy.block_password_fields;
            commit(|c| c.privacy.block_password_fields = v)
        }
        (V_REDACT, true, _) => {
            let v = !c.privacy.redact_sensitive_window_titles;
            commit(|c| c.privacy.redact_sensitive_window_titles = v)
        }
        (V_CLIPHIST, true, _) => {
            let v = !c.privacy.exclude_from_clipboard_history;
            commit(|c| c.privacy.exclude_from_clipboard_history = v)
        }
        (V_CLOUD_JOBS, true, _) => {
            let v = !c.text_jobs_allow_cloud;
            commit(|c| c.text_jobs_allow_cloud = v)
        }
        (V_CLEAR_HIST, true, _) => send(UiAction::ClearHistory),
        (AB_RELEASES, true, _) => open_url(&format!("{REPO_URL}/releases")),
        (AB_UPDATES, true, _) => open_url(&format!("{REPO_URL}/releases/latest")),
        (AB_GITHUB, true, _) => open_url(REPO_URL),
        (AB_LICENSE, true, _) => open_url(&format!("{REPO_URL}/blob/main/LICENSE")),
        (AB_SETUP, true, _) => {
            ST.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    st.step = 0;
                }
            });
            navigate(PAGE_START);
        }
        (AB_RESET, true, _) => ask(
            "Reset all settings?",
            "KeyReed will restore its default settings. Downloaded speech models are kept.",
            "Reset",
            ModalAction::ResetAll,
        ),
        (S_DL, true, _) => {
            let id = if m.active_model_ready() { c.local_asr_model.clone() } else { ASR_UI_CATALOG[0].id.to_string() };
            model_action(&m, &id);
        }
        (S_ALLMODELS, true, _) => navigate(PAGE_MODELS),
        (S_SKIP, true, _) => finish_setup(false),
        (S_BACK, true, _) => {
            let step = ST.with(|s| s.borrow().as_ref().map(|x| x.step).unwrap_or(0));
            // The model step moves on by itself once the model is ready, so Back from the practice step skips it.
            set_step(if step == 3 && m.active_model_ready() { 1 } else { step.saturating_sub(1) });
        }
        (S_NEXT, true, _) => {
            let step = ST.with(|s| s.borrow().as_ref().map(|x| x.step).unwrap_or(0));
            if step >= 3 {
                finish_setup(true);
            } else {
                // Welcome: Next is the explicit consent to the one-time download of the speech engine.
                if step == 0 && !m.active_model_ready() && !m.download.as_ref().is_some_and(|d| d.is_active()) {
                    model_action(&m, ASR_UI_CATALOG[0].id);
                }
                set_step(step + 1);
            }
        }
        _ => {}
    }
}

fn model_action(m: &SharedUiModel, id: &str) {
    let downloading_this = m.download.as_ref().map(|d| d.is_active() && d.model_id.eq_ignore_ascii_case(id)).unwrap_or(false);
    if downloading_this {
        send(UiAction::CancelModelDownload);
    } else if m.is_installed(id) {
        send(UiAction::SelectAsrModel(id.to_string()));
    } else {
        send(UiAction::DownloadModel(id.to_string()));
    }
}

fn finish_setup(finished: bool) {
    commit(|c| c.first_run_completed = true);
    let ready = snapshot().map(|m| m.active_model_ready()).unwrap_or(false);
    if finished && ready {
        if let Some(g) = globals() {
            let hk = g.model.lock().config.ptt_hotkey.display_name.clone();
            *g.toast_buf.lock() = Some((
                "KeyReed is ready".into(),
                format!("Hold {hk} in any app and speak. The KeyReed listener shows when it's recording."),
                false,
            ));
            unsafe {
                PostMessageW(*g.tray_hwnd.lock() as HWND, crate::ui::win_gui::WM_SHOW_TOAST, 0, 0);
            }
        }
        unsafe {
            PostMessageW(hwnd(), WM_CLOSE, 0, 0);
        }
    } else {
        navigate(if ready { PAGE_GENERAL } else { PAGE_MODELS });
    }
}

// ------------------------------------------------------------------------------------------------
// Window procedures
// ------------------------------------------------------------------------------------------------

/// Left/right-specific virtual key of a key message: the capture keeps "Right Ctrl" apart from Ctrl.
fn specific_vk(wparam: WPARAM, lparam: LPARAM) -> u16 {
    let extended = (lparam >> 24) & 1 != 0;
    match wparam as u16 {
        0x11 => if extended { 0xA3 } else { 0xA2 },
        0x12 => if extended { 0xA5 } else { 0xA4 },
        0x10 => if (lparam >> 16) & 0xFF == 0x36 { 0xA1 } else { 0xA0 },
        vk => vk,
    }
}

fn capturing(id: i32) -> bool {
    CAPTURE.with(|c| c.borrow().as_ref().is_some_and(|(i, _)| *i == id))
}

fn shortcut_role_of(id: i32) -> Option<ShortcutRole> {
    ST.with(|s| {
        s.try_borrow().ok()?.as_ref()?.ctrls.iter().find_map(|(c, _)| match &c.kind {
            Kind::Shortcut { role, .. } if c.id == id => Some(*role),
            _ => None,
        })
    })
}

/// Starts recording a chord into the field `id`. KeyReed's own shortcuts are paused meanwhile, so pressing the
/// current dictation chord does not start a recording.
fn start_capture(id: i32) {
    SHORTCUT_REFUSAL.with(|n| *n.borrow_mut() = None);
    CAPTURE.with(|c| *c.borrow_mut() = Some((id, ShortcutCapture::default())));
    crate::hotkey::set_capture_active(true);
    unsafe {
        let h = GetDlgItem(hwnd(), id);
        SetFocus(h);
        InvalidateRect(h, std::ptr::null(), 0);
    }
    refresh();
}

/// Ends the capture in `id`: saves `chord` unless a rule refuses it (the reason then shows under the row).
fn end_capture(id: i32, chord: Option<HotkeySpec>) {
    if !capturing(id) {
        return;
    }
    CAPTURE.with(|c| *c.borrow_mut() = None);
    crate::hotkey::set_capture_active(false);
    if let (Some(spec), Some(role), Some(m)) = (chord, shortcut_role_of(id), snapshot()) {
        match m.config.shortcut_problem(role, &spec) {
            Some(p) if p.blocks => SHORTCUT_REFUSAL.with(|n| *n.borrow_mut() = Some((id, p.message))),
            _ => commit(move |c| match role {
                ShortcutRole::Dictation => c.ptt_hotkey = spec,
                ShortcutRole::QuickFix => c.quick_fix_hotkey = spec,
                ShortcutRole::Composer => c.composer_hotkey = spec,
                ShortcutRole::VoiceCommand => c.voice_command_hotkey = spec,
            }),
        }
    }
    unsafe { InvalidateRect(GetDlgItem(hwnd(), id), std::ptr::null(), 0) };
    refresh();
}

/// Key handling of a shortcut field while it records. Only messages sent to this field are read, so the capture
/// never sees keys typed anywhere else. `None` = default handling.
unsafe fn shortcut_field_msg(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let id = GetDlgCtrlID(h);
    if !capturing(id) {
        return None;
    }
    match msg {
        WM_GETDLGCODE => Some(DLGC_WANTALLKEYS as LRESULT),
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
            let dictation = shortcut_role_of(id) == Some(ShortcutRole::Dictation);
            let step = CAPTURE.with(|c| c.borrow_mut().as_mut().map(|(_, cap)| cap.key(specific_vk(wparam, lparam), down, dictation)));
            match step {
                Some(CaptureStep::Done(spec)) => end_capture(id, Some(spec)),
                Some(CaptureStep::Cancelled) => end_capture(id, None),
                _ => {
                    InvalidateRect(h, std::ptr::null(), 0);
                }
            }
            Some(0)
        }
        WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => Some(0),
        WM_KILLFOCUS => {
            end_capture(id, None);
            None
        }
        _ => None,
    }
}

/// A text box would scroll its own text on the wheel; the page scrolls instead.
unsafe extern "system" fn edit_wheel_subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _kind: usize) -> LRESULT {
    if msg == WM_MOUSEWHEEL {
        return wheel_scroll(wparam);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn btn_subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, is_select: usize) -> LRESULT {
    const VK_UP: usize = 0x26;
    const VK_DOWN: usize = 0x28;
    const VK_F4: usize = 0x73;
    if msg == WM_MOUSEWHEEL {
        return wheel_scroll(wparam);
    }
    if is_select == 2 {
        if let Some(r) = shortcut_field_msg(hwnd, msg, wparam, lparam) {
            return r;
        }
    }
    if is_select == 3 {
        // The sidebar is a source list: Up and Down move the selection (and with it the page).
        match msg {
            WM_GETDLGCODE => return DefSubclassProc(hwnd, msg, wparam, lparam) | DLGC_WANTARROWS as LRESULT,
            WM_KEYDOWN if wparam == VK_UP || wparam == VK_DOWN => {
                let cur = GetDlgCtrlID(hwnd) - ID_NAV;
                let new = if wparam == VK_UP { cur - 1 } else { cur + 1 };
                if (0..NAV.len() as i32).contains(&new) {
                    // Posted, not called: switching pages destroys this very control.
                    PostMessageW(GetParent(hwnd), WM_COMMAND, (ID_NAV + new) as WPARAM, 0);
                }
                return 0;
            }
            _ => {}
        }
    }
    if is_select == 1 {
        let id = GetDlgCtrlID(hwnd);
        match msg {
            WM_GETDLGCODE => return DefSubclassProc(hwnd, msg, wparam, lparam) | DLGC_WANTARROWS as LRESULT,
            WM_KEYDOWN if wparam == VK_UP || wparam == VK_DOWN => {
                if let Some((_, items, sel)) = combo_of(id) {
                    let new = if wparam == VK_UP { sel.saturating_sub(1) } else { (sel + 1).min(items.len().saturating_sub(1)) };
                    set_combo_sel(id, new);
                }
                return 0;
            }
            WM_KEYDOWN if wparam == VK_F4 => {
                open_select(id);
                return 0;
            }
            WM_SYSKEYDOWN if wparam == VK_DOWN => {
                open_select(id);
                return 0;
            }
            _ => {}
        }
    }
    match msg {
        WM_MOUSEMOVE => {
            let id = GetDlgCtrlID(hwnd);
            let changed = ST.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    if st.hot != id {
                        let prev = st.hot;
                        st.hot = id;
                        if prev != 0 {
                            if let Some((_, h)) = st.ctrls.iter().find(|(c, _)| c.id == prev) {
                                InvalidateRect(*h, std::ptr::null(), 0);
                            }
                        }
                        return true;
                    }
                }
                false
            });
            if changed {
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
        }
        WM_MOUSELEAVE_U => {
            let id = GetDlgCtrlID(hwnd);
            ST.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    if st.hot == id {
                        st.hot = 0;
                    }
                }
            });
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
        WM_ERASEBKGND => return 1,
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Gives the system-drawn controls (drop-downs, text boxes, tooltips) the matching light or dark
/// visual style. Unknown style names are ignored by Windows, so older builds simply keep the light look.
unsafe fn theme_native_control(ctl: HWND, class: &str) {
    use windows_sys::Win32::UI::Controls::SetWindowTheme;
    let dark = DARK.load(std::sync::atomic::Ordering::Relaxed) && !high_contrast_on();
    let name = match (dark, class) {
        (false, _) => None,
        (true, _) => Some("DarkMode_Explorer"),
    };
    match name {
        Some(n) => { let w = to_wide(n); SetWindowTheme(ctl, w.as_ptr(), std::ptr::null()); }
        None => { SetWindowTheme(ctl, std::ptr::null(), std::ptr::null()); }
    }
}

unsafe extern "system" fn retheme_child(h: HWND, _: LPARAM) -> i32 {
    let mut buf = [0u16; 32];
    let n = GetClassNameW(h, buf.as_mut_ptr(), 32) as usize;
    let class = String::from_utf16_lossy(&buf[..n]);
    if class == "Edit" || class == "tooltips_class32" {
        theme_native_control(h, &class);
    }
    1
}

/// Popup menus (drop-down lists, tray menu) follow the Settings theme. Uses uxtheme's
/// SetPreferredAppMode/FlushMenuThemes (ordinals 135/136, Windows 10 1903+); missing exports are skipped.
unsafe fn apply_menu_theme() {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let lib = to_wide("uxtheme.dll");
    let m = LoadLibraryW(lib.as_ptr());
    if m.is_null() {
        return;
    }
    let dark = DARK.load(std::sync::atomic::Ordering::Relaxed) && !high_contrast_on();
    if let Some(f) = GetProcAddress(m, 135 as *const u8) {
        let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_mode(if dark { 2 /* ForceDark */ } else { 3 /* ForceLight */ });
    }
    if let Some(f) = GetProcAddress(m, 136 as *const u8) {
        let flush: unsafe extern "system" fn() = std::mem::transmute(f);
        flush();
    }
}

unsafe fn apply_window_chrome(hwnd: HWND) {
    apply_menu_theme();
    let t = theme();
    let border: u32 = t.border;
    let dark = DARK.load(std::sync::atomic::Ordering::Relaxed) as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as *const _, 4); // DWMWA_USE_IMMERSIVE_DARK_MODE
    DwmSetWindowAttribute(hwnd, 34, &border as *const u32 as *const _, 4);
    // Frameless (Windows 11; ignored on Windows 10): round the corners, and keep a popup's drop shadow.
    let round: u32 = 2; // DWMWCP_ROUND
    DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const _, 4); // DWMWA_WINDOW_CORNER_PREFERENCE
    let shadow: u32 = 2; // DWMNCRP_ENABLED
    DwmSetWindowAttribute(hwnd, 2, &shadow as *const u32 as *const _, 4); // DWMWA_NCRENDERING_POLICY
    let big = brand_hicon(32);
    let small = brand_hicon(20);
    SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as LPARAM);
    SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as LPARAM);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            refresh_dark_mode();
            PRACTICE_HEARD.with(|c| c.set(false));
            PRACTICE_NOTE.with(|c| c.set(0));
            ADVANCE_ARMED.with(|c| c.set(false));
            VOCAB_VIEW.with(|v| v.set(None));
            VOCAB_DRAFT.with(|v| v.set(false));
            let dpi = window_dpi(hwnd);
            let tooltip = {
                let cls = to_wide("tooltips_class32");
                let h = CreateWindowExW(WS_EX_TOPMOST, cls.as_ptr(), std::ptr::null(), WS_POPUP | 0x01 | 0x02, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, hwnd, std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null());
                if !h.is_null() {
                    SendMessageW(h, TTM_SETMAXTIPWIDTH, 0, sc(360, dpi) as LPARAM);
                    theme_native_control(h, "tooltips_class32");
                }
                h
            };
            let page = PENDING_PAGE.with(|c| c.get());
            EDIT_BRUSH.with(|b| {
                if b.get() == 0 {
                    b.set(CreateSolidBrush(theme().bg) as isize);
                }
            });
            ST.with(|s| {
                *s.borrow_mut() = Some(St {
                    hwnd,
                    page,
                    step: 0,
                    dpi,
                    fonts: Fonts::new(dpi),
                    ctrls: Vec::new(),
                    items: Vec::new(),
                    hot: 0,
                    saved_at: None,
                    tooltip,
                    practice_text: String::new(),
                    key_draft: String::new(),
                    last_page: page,
                    scroll: 0,
                    content_h: WIN_H,
                    view_h: WIN_H,
                    anims: Vec::new(),
                })
            });
            apply_window_chrome(hwnd);
            rebuild(true);
            0
        }
        WM_ERASEBKGND => 1,
        // Frameless: no system frame. The client area is the whole window, so the resize grip and the caption
        // live inside it (WM_NCHITTEST below).
        WM_NCCALCSIZE if wparam != 0 => 0,
        WM_NCPAINT => 0,
        WM_NCHITTEST => {
            let (x, y) = (((lparam & 0xFFFF) as u16 as i16) as i32, (((lparam >> 16) & 0xFFFF) as u16 as i16) as i32);
            let mut rc: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rc);
            let dpi = window_dpi(hwnd);
            let g = sc(6, dpi);
            let (near_left, near_top) = (x < rc.left + g, y < rc.top + g);
            let (near_right, near_bottom) = (x >= rc.right - g, y >= rc.bottom - g);
            let edge = match (near_left, near_top, near_right, near_bottom) {
                (true, true, _, _) => HTTOPLEFT,
                (_, true, true, _) => HTTOPRIGHT,
                (true, _, _, true) => HTBOTTOMLEFT,
                (_, _, true, true) => HTBOTTOMRIGHT,
                (true, _, _, _) => HTLEFT,
                (_, true, _, _) => HTTOP,
                (_, _, true, _) => HTRIGHT,
                (_, _, _, true) => HTBOTTOM,
                _ => 0,
            };
            if edge != 0 {
                return edge as LRESULT;
            }
            // The header row drags the window; the page below it does not.
            if y - rc.top < sc(HEADER_H, dpi) { HTCAPTION as LRESULT } else { HTCLIENT as LRESULT }
        }
        // A double-click on the header must not maximise a window that has no maximise button.
        WM_NCLBUTTONDBLCLK => 0,
        WM_DICT_COMMIT => {
            dict_commit_deferred();
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            ST.with(|s| {
                if let Ok(b) = s.try_borrow() {
                    if let Some(st) = b.as_ref() {
                        paint(st, hdc);
                    }
                }
            });
            EndPaint(hwnd, &ps);
            0
        }
        WM_DRAWITEM => {
            let di = &*(lparam as *const DrawItem);
            ST.with(|s| {
                if let Ok(b) = s.try_borrow() {
                    if let Some(st) = b.as_ref() {
                        draw_item(st, di);
                    }
                }
            });
            1
        }
        WM_CTLCOLOREDIT => {
            let t = theme();
            let hdc = wparam as HDC;
            SetTextColor(hdc, t.text);
            SetBkColor(hdc, t.bg);
            EDIT_BRUSH.with(|b| b.get() as LRESULT)
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            let code = ((wparam >> 16) & 0xFFFF) as u32;
            if id == IDCANCEL as i32 && MODAL.with(|m| m.borrow().is_some()) {
                close_modal(); // Esc answers the card first
                return 0;
            }
            if id == IDCANCEL as i32 && VOCAB_VIEW.with(|v| v.get()).is_some() {
                close_vocab(); // Esc leaves the dictionary editor first, not the window
                return 0;
            }
            if id == IDCANCEL as i32 {
                // Esc closes Settings, except while the user is dictating into the practice box. Every other
                // recording must not trap the window: while one is stuck, Settings is the only way out.
                if focused_id() != S_PRACTICE {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
                return 0;
            }
            if id == IDOK as i32 {
                on_enter();
                return 0;
            }
            on_command(id, code);
            0
        }
        WM_SIZE => {
            let h_px = ((lparam >> 16) & 0xFFFF) as i32;
            ST.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    st.view_h = (h_px as f32 * 96.0 / st.dpi as f32).round() as i32;
                    st.scroll = st.scroll.clamp(0, (st.content_h - st.view_h).max(0));
                }
            });
            0
        }
        WM_MOUSEWHEEL => {
            // The card's buttons do not scroll the page behind them.
            if MODAL.with(|m| m.borrow().is_some()) {
                return 0;
            }
            wheel_scroll(wparam)
        }
        WM_TIMER => {
            if wparam == IDT_ANIM {
                let (hs, done) = ST.with(|s| {
                    let Ok(mut b) = s.try_borrow_mut() else { return (Vec::new(), false) };
                    let Some(st) = b.as_mut() else { return (Vec::new(), true) };
                    let hs: Vec<HWND> = st.anims.iter().filter_map(|(id, _)| st.ctrls.iter().find(|(c, _)| c.id == *id).map(|(_, h)| *h)).collect();
                    st.anims.retain(|(_, t0)| t0.elapsed().as_secs_f32() * 1000.0 < TOGGLE_MS);
                    (hs, st.anims.is_empty())
                });
                for h in hs {
                    InvalidateRect(h, std::ptr::null(), 0);
                }
                if done {
                    KillTimer(hwnd, IDT_ANIM);
                }
                return 0;
            }
            if wparam == IDT_ADVANCE {
                KillTimer(hwnd, IDT_ADVANCE);
                ADVANCE_ARMED.with(|c| c.set(false));
                let on_model_step = ST.with(|s| s.try_borrow().ok().and_then(|b| b.as_ref().map(|st| st.page == PAGE_START && st.step == 2))).unwrap_or(false);
                if on_model_step && snapshot().is_some_and(|m| m.active_model_ready()) {
                    set_step(3);
                }
                return 0;
            }
            if wparam == IDT_SAVED {
                KillTimer(hwnd, IDT_SAVED);
                ST.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        st.saved_at = None;
                    }
                });
                refresh();
            }
            0
        }
        WM_DPICHANGED => {
            let dpi = ((wparam >> 16) & 0xFFFF) as u32;
            let rc = &*(lparam as *const RECT);
            harvest_and_commit();
            ST.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    let _ = harvest_edits(st);
                    st.dpi = dpi.max(96);
                    st.fonts = Fonts::new(st.dpi);
                }
            });
            SetWindowPos(hwnd, std::ptr::null_mut(), rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
            rebuild(false);
            0
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
            // High-contrast toggles, colour changes and the Windows light/dark app mode repaint everything.
            refresh_dark_mode();
            EnumChildWindows(hwnd, Some(retheme_child), 0);
            EDIT_BRUSH.with(|b| {
                if b.get() != 0 {
                    DeleteObject(b.get() as _);
                }
                b.set(CreateSolidBrush(theme().bg) as isize);
            });
            apply_window_chrome(hwnd);
            rebuild(false);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            harvest_and_commit();
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            // A capture never outlives the window: KeyReed's shortcuts must work again.
            CAPTURE.with(|c| *c.borrow_mut() = None);
            crate::hotkey::set_capture_active(false);
            ST.with(|s| {
                if let Some(mut st) = s.borrow_mut().take() {
                    // Never keep a typed-but-unsaved API key around.
                    unsafe {
                        let bytes = st.key_draft.as_bytes_mut();
                        for b in bytes.iter_mut() {
                            std::ptr::write_volatile(b, 0);
                        }
                    }
                    st.key_draft.clear();
                    if !st.tooltip.is_null() {
                        DestroyWindow(st.tooltip);
                    }
                    LAST_PAGE.with(|c| c.set(st.last_page));
                }
            });
            if let Some(g) = globals() {
                g.model.lock().mic_test = crate::ui::MicTestStatus::default();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{DownloadProgress, MicTestStatus};

    #[test]
    fn capture_keeps_left_and_right_modifiers_apart() {
        const EXT: LPARAM = 1 << 24;
        assert_eq!(specific_vk(0x11, 0), 0xA2);
        assert_eq!(specific_vk(0x11, EXT), 0xA3);
        assert_eq!(specific_vk(0x12, EXT), 0xA5);
        assert_eq!(specific_vk(0x10, 0x36 << 16), 0xA1);
        assert_eq!(specific_vk(0x10, 0x2A << 16), 0xA0);
        assert_eq!(specific_vk(0x47, EXT), 0x47);
    }

    fn model() -> SharedUiModel {
        SharedUiModel {
            config: AppConfig::default(),
            state: EngineState::Idle,
            rms_level: 0.0,
            status_detail: String::new(),
            available_microphones: vec!["Microphone (USB)".into()],
            installed_asr_models: vec![],
            download_in_progress: false,
            download: None,
            mic_test: MicTestStatus::default(),
            last_outcome: None,
            hint: None,
            processing_note: None,
            quick_fix: None,
            style: Default::default(),
        }
    }

    /// Horizontally on screen and below the top. Taller pages scroll (see `content_height`), so a control
    /// may sit past the window's bottom edge.
    fn rect_ok(rr: &R) -> bool {
        rr.x >= 0 && rr.y >= 0 && rr.x + rr.w <= WIN_W && rr.y + rr.h <= WIN_H * 2
    }

    #[test]
    fn every_page_and_step_fits_the_window_without_overlap() {
        let mut m = model();
        for variant in 0..5 {
            match variant {
                1 => {
                    m.installed_asr_models = vec!["parakeet-tdt-0.6b-v2-int8".into()];
                    m.mic_test = MicTestStatus { verdict: Some(MicVerdict::Good), peak_rms: 0.2, device: "Mic".into(), ..Default::default() };
                }
                2 => {
                    m.download = Some(DownloadProgress { model_id: "moonshine-tiny-en-int8".into(), stage: DownloadStage::Downloading, done_bytes: 10, total_bytes: Some(100) });
                    m.config.cloud.encrypted_api_key_b64 = Some("x".into());
                    m.mic_test = MicTestStatus { error: Some("Mic error: x".into()), ..Default::default() };
                }
                3 => {
                    m.mic_test = MicTestStatus { verdict: Some(MicVerdict::Silent), peak_rms: 0.0, device: "Mic".into(), ..Default::default() };
                }
                4 => {
                    // "My style" with content: learned rows and their paging, suggestions, an excluded app.
                    for n in 0..7 {
                        m.style.observe(vi_core::style::Source::Samples, &format!("habit_{n}"), vi_core::style::Scope::Everywhere, "yes", 35, 1);
                    }
                    m.style.set_locked("dialect", vi_core::style::Scope::Everywhere, "british", 2);
                    for n in 0..6 {
                        m.style.record_candidate(&format!("word{n}"), &format!("Word{n}"), "chat", 1);
                    }
                    m.style.exclude_app("keepass.exe");
                }
                _ => {}
            }
            for page in 0..NAV.len() {
                for step in 0..4 {
                    let (items, ctrls) = build(page, step, &m, false);
                    for c in &ctrls {
                        assert!(rect_ok(&c.r), "page {page} step {step} ctrl {} out of bounds: {:?}", c.id, c.r);
                    }
                    for it in &items {
                        if let Item::Group { r } | Item::Card { r } | Item::Banner { r, .. } = it {
                            assert!(rect_ok(r), "page {page} step {step} item out of bounds: {r:?}");
                        }
                    }
                    // Interactive controls in the content area must not overlap each other.
                    let content: Vec<&Ctrl> = ctrls.iter().filter(|c| !matches!(c.kind, Kind::Nav { .. })).collect();
                    for (i, a) in content.iter().enumerate() {
                        for b in content.iter().skip(i + 1) {
                            let overlap = a.r.x < b.r.x + b.r.w && b.r.x < a.r.x + a.r.w && a.r.y < b.r.y + b.r.h && b.r.y < a.r.y + a.r.h;
                            assert!(!overlap, "page {page} step {step}: {} overlaps {}", a.id, b.id);
                        }
                    }
                    if page != PAGE_START {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn all_six_writing_modes_and_models_are_selectable() {
        let m = model();
        let (_, ctrls) = build(PAGE_WRITING, 0, &m, false);
        assert_eq!(ctrls.iter().filter(|c| (W_MODE..W_MODE + 6).contains(&c.id)).count(), 6);
        let (_, ctrls) = build(PAGE_MODELS, 0, &m, false);
        assert_eq!(ctrls.iter().filter(|c| (M_ACT..M_ACT + ASR_UI_CATALOG.len() as i32).contains(&c.id)).count(), ASR_UI_CATALOG.len());
        let (_, ctrls) = build(PAGE_OUTPUT, 0, &m, false);
        assert_eq!(ctrls.iter().filter(|c| [O_TYPE, O_CLIP, O_ASK].contains(&c.id)).count(), 3);
    }

    /// The frameless header owns the top of the window, everything else starts below it, and the confirmation
    /// card fits the design size with Cancel (the default button) to the left of Confirm.
    #[test]
    fn the_header_and_the_confirmation_card_fit_the_window() {
        let (items, ctrls) = build(PAGE_GENERAL, 0, &model(), false);
        let win_btn = |id: i32| ctrls.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} is missing"));
        for id in [HEADER_MIN, HEADER_CLOSE] {
            let b = win_btn(id);
            assert!(b.r.y >= 0 && b.r.y + b.r.h <= HEADER_H, "window button {id} inside the header: {:?}", b.r);
            assert!(b.r.x >= SIDEBAR_W && b.r.x + b.r.w <= WIN_W, "window button {id} on the right: {:?}", b.r);
        }
        assert!(win_btn(HEADER_MIN).r.x < win_btn(HEADER_CLOSE).r.x, "minimise left of close");
        let nav = ctrls.iter().find(|c| c.id == ID_NAV + PAGE_GENERAL as i32).expect("nav");
        assert!(nav.r.y >= HEADER_H, "navigation below the header: {:?}", nav.r);
        let top = items
            .iter()
            .filter_map(|it| match it {
                Item::Text { r, .. } => Some(r.y),
                _ => None,
            })
            .min()
            .expect("page text");
        assert!(top >= HEADER_H, "page content below the header, was {top}");

        MODAL.with(|m| {
            *m.borrow_mut() = Some(Modal {
                title: "Delete everything KeyReed learned?".into(),
                body: "Every learned word and habit go.".into(),
                confirm: "Delete".into(),
                action: ModalAction::DeleteLearned,
            })
        });
        let (_, ctrls) = build(PAGE_GENERAL, 0, &model(), false);
        MODAL.with(|m| *m.borrow_mut() = None);
        let card = modal_card();
        assert!(card.x >= 0 && card.y >= 0 && card.x + card.w <= WIN_W && card.y + card.h <= WIN_H, "{card:?}");
        let cancel = ctrls.iter().find(|c| c.id == MODAL_CANCEL).expect("cancel");
        let confirm = ctrls.iter().find(|c| c.id == MODAL_CONFIRM).expect("confirm");
        for b in [cancel, confirm] {
            assert!(b.r.x >= card.x && b.r.x + b.r.w <= card.x + card.w, "{:?} inside the card", b.r);
            assert!(b.r.y >= card.y && b.r.y + b.r.h <= card.y + card.h, "{:?} inside the card", b.r);
        }
        assert!(cancel.r.x < confirm.r.x, "Cancel on the left of Confirm");
        assert!(matches!(confirm.kind, Kind::Button { style: BtnStyle::Danger, .. }), "the destructive action is red");
        assert!(matches!(cancel.kind, Kind::Button { style: BtnStyle::Secondary, .. }));
    }

    #[test]
    fn the_grammar_row_installs_but_never_becomes_a_speech_model() {
        let label = |m: &SharedUiModel, want_enabled: bool| {
            let (_, ctrls) = build(PAGE_MODELS, 0, m, false);
            let c = ctrls.iter().find(|c| c.id == M_GRAMMAR).expect("grammar row").clone();
            assert_eq!(c.enabled, want_enabled);
            match c.kind {
                Kind::Button { label, .. } => label,
                _ => unreachable!(),
            }
        };
        let mut m = model();
        assert_eq!(label(&m, true), "Download");
        m.download = Some(DownloadProgress { model_id: GRAMMAR_MODEL_ID.into(), stage: DownloadStage::Downloading, done_bytes: 1, total_bytes: Some(2) });
        assert_eq!(label(&m, true), "Cancel");
        m.download = Some(DownloadProgress { model_id: GRAMMAR_MODEL_ID.into(), stage: DownloadStage::Verifying, done_bytes: 0, total_bytes: None });
        assert_eq!(label(&m, false), "Verifying…");
        m.download = None;
        m.installed_asr_models = vec![GRAMMAR_MODEL_ID.into()];
        assert_eq!(label(&m, false), "Installed");
        let (items, _) = build(PAGE_MODELS, 0, &m, false);
        assert!(items.iter().any(|i| matches!(i, Item::Badge { s, .. } if s == "In use by Quick Fix")));
    }

    #[test]
    fn model_buttons_reflect_real_install_and_download_state() {
        let mut m = model();
        m.installed_asr_models = vec!["parakeet-tdt-0.6b-v2-int8".into(), "moonshine-base-en-int8".into()];
        m.download = Some(DownloadProgress { model_id: "whisper-small.en-int8".into(), stage: DownloadStage::Downloading, done_bytes: 1, total_bytes: None });
        let (_, ctrls) = build(PAGE_MODELS, 0, &m, false);
        let label = |id: i32| match &ctrls.iter().find(|c| c.id == id).unwrap().kind {
            Kind::Button { label, .. } => label.clone(),
            _ => unreachable!(),
        };
        // Button ids follow the catalogue order, so look the models up instead of hardcoding an index.
        let idx = |id: &str| M_ACT + ASR_UI_CATALOG.iter().position(|m| m.id.eq_ignore_ascii_case(id)).expect("in the catalogue") as i32;
        assert_eq!(label(M_ACT), "In use");
        assert_eq!(label(idx("moonshine-base-en-int8")), "Use");
        assert_eq!(label(idx("whisper-small.en-int8")), "Cancel");
        let idle = idx("sense-voice-small-int8");
        assert_eq!(label(idle), "Download");
        assert!(!ctrls.iter().find(|c| c.id == idle).unwrap().enabled, "one download at a time");
    }

    #[test]
    fn the_dictation_page_offers_every_chord_and_the_three_mouse_buttons() {
        let m = model();
        let (_, ctrls) = build(PAGE_DICTATION, 0, &m, false);
        for id in [D_HOTKEY, D_QF_KEY, D_VOICE_KEY, D_COMPOSER_KEY] {
            let c = ctrls.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} is missing"));
            assert!(matches!(c.kind, Kind::Shortcut { .. }), "{id} is a chord field");
        }
        for i in 0..3i32 {
            let c = ctrls.iter().find(|c| c.id == D_MOUSE + i).unwrap_or_else(|| panic!("mouse row {i} is missing"));
            assert!(matches!(&c.kind, Kind::Combo { items, .. } if items.len() == MouseAction::ALL.len()));
        }
    }

    /// A tall page must reach every control, and one that fits must not scroll at all.
    #[test]
    fn tall_pages_scroll_to_their_last_control() {
        let m = model();
        let view = (SIDEBAR_W + 1, HEADER_H + 1, WIN_W, WIN_H);
        for (page, name) in [(PAGE_DICTATION, "dictation"), (PAGE_MODELS, "speech models")] {
            let (items, ctrls) = build(page, 0, &m, false);
            let h = content_height(&items, &ctrls);
            let end = max_scroll(h, WIN_H);
            assert!(h > WIN_H && end > 0, "{name}: content {h} in a {WIN_H} window scrolls ({end})");
            for c in ctrls.iter().filter(|c| !fixed_in_place(&c.kind)) {
                let off = (c.r.y + c.r.h - WIN_H).max(0).min(end);
                let (x0, y0, x1, y1) = child_region(c.r.x, c.r.y - off, c.r.w, c.r.h, view);
                assert_eq!(
                    (x0, y0, x1, y1),
                    (0, 0, c.r.w, c.r.h),
                    "{name}: control {} is not whole at offset {off}",
                    c.id
                );
            }
        }
    }

    #[test]
    fn scrolling_stops_at_both_ends_and_a_page_that_fits_does_not_move() {
        assert_eq!(WHEEL_STEP, ROW_H, "one notch lands on a row boundary");
        assert_eq!(next_scroll(0, -WHEEL_STEP, 1000, WIN_H), 0, "never negative");
        assert_eq!(next_scroll(0, WHEEL_STEP, 1000, WIN_H), WHEEL_STEP);
        let end = max_scroll(1000, WIN_H);
        assert_eq!(next_scroll(end - 10, WHEEL_STEP, 1000, WIN_H), end, "stops at the end");
        assert_eq!(next_scroll(end, WHEEL_STEP, 1000, WIN_H), end);
        assert_eq!(max_scroll(WIN_H, WIN_H), 0, "a page that fits does not scroll");
        assert_eq!(max_scroll(WIN_H - 40, WIN_H), 0);
        assert_eq!(next_scroll(0, WHEEL_STEP, WIN_H - 40, WIN_H), 0);
    }

    #[test]
    fn tabbing_to_a_control_below_the_fold_scrolls_it_into_view() {
        assert_eq!(reveal_delta(100, 32, 0, WIN_H), None, "already in view: no scroll");
        assert_eq!(reveal_delta(700, 32, 0, WIN_H), Some(700 + 32 - WIN_H + 8), "just enough to show it");
        assert_eq!(reveal_delta(300, 32, 400, WIN_H), Some(-108), "scrolled past the top: back down");
        assert!(reveal_delta(WIN_H - 10, 32, 0, WIN_H).is_some(), "half hidden at the bottom edge");
        assert!(reveal_delta(640, 32, 0, 660).is_some() && reveal_delta(620, 32, 0, 660).is_none());
    }

    /// The clip is what makes a control scrolled out of view unpainted and unclickable, and one in view whole.
    #[test]
    fn a_control_scrolled_out_of_view_is_clipped_away() {
        let view = (SIDEBAR_W + 1, HEADER_H + 1, WIN_W, WIN_H);
        assert_eq!(child_region(300, 100, 200, 32, view), (0, 0, 200, 32), "whole when in view");
        assert_eq!(child_region(300, WIN_H - 10, 200, 32, view), (0, 0, 200, 10), "half past the bottom");
        assert_eq!(child_region(300, 30, 200, 32, view), (0, HEADER_H + 1 - 30, 200, 32), "top cut off at the header");
        assert_eq!(child_region(300, 20, 200, 32, view), (0, 0, 0, 0), "scrolled under the header: nothing");
        assert_eq!(child_region(300, WIN_H + 40, 200, 32, view), (0, 0, 0, 0), "below the window: nothing to paint or click");
        assert_eq!(child_region(300, -60, 200, 32, view), (0, 0, 0, 0), "above the window");
        assert_eq!(child_region(10, 100, 200, 32, view), (0, 0, 0, 0), "left of the content column");
    }

    #[test]
    fn the_general_page_offers_the_composer_on_a_pill_click() {
        let (_, ctrls) = build(PAGE_GENERAL, 0, &model(), false);
        let c = ctrls.iter().find(|c| c.id == G_PILL_CLICK).expect("the pill click choice");
        match &c.kind {
            Kind::Combo { items, .. } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[1], "Open the Composer");
            }
            _ => panic!("a drop-down"),
        }
    }

    #[test]
    fn unplugged_preferred_microphone_stays_visible() {
        let mut m = model();
        m.config.selected_microphone = Some("Headset (BT)".into());
        let (items, sel) = mic_items(&m);
        assert_eq!(items[sel], "Headset (BT) (not connected)");
    }

    fn luminance(c: u32) -> f64 {
        let f = |v: u32| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c & 0xFF) + 0.7152 * f((c >> 8) & 0xFF) + 0.0722 * f((c >> 16) & 0xFF)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// WCAG 2.2 AA for both colour schemes: 4.5:1 for text, 3:1 for control boundaries and the ON switch.
    /// One test, because the scheme is process-global state.
    #[test]
    fn both_themes_meet_wcag_aa() {
        for dark in [false, true] {
            DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
            let t = theme();
            if t.hc {
                continue; // high-contrast mode uses the user's own system colours
            }
            let text_pairs = [
                ("text/bg", t.text, t.bg), ("text/surface", t.text, t.surface), ("text2/bg", t.text2, t.bg),
                ("text2/surface", t.text2, t.surface), ("text2/hover", t.text2, t.hover),
                ("text3/bg", t.text3, t.bg), ("text3/surface", t.text3, t.surface), ("text3/sidebar", t.text3, t.sidebar),
                ("text3/hover", t.text3, t.hover), ("text3/accent_soft", t.text3, t.accent_soft),
                ("accent text/bg", t.accent, t.bg), ("accent text/surface", t.accent, t.surface),
                ("accent text/accent_soft", t.accent, t.accent_soft), ("accent text/sidebar", t.accent, t.sidebar),
                ("on_accent/accent_fill", t.on_accent, t.accent_fill), ("on_accent/accent (hover)", t.on_accent, t.accent),
                ("on_accent/accent_press", t.on_accent, t.accent_press),
                ("ok", t.ok_fg, t.ok_bg), ("warn", t.warn_fg, t.warn_bg), ("err", t.err_fg, t.err_bg),
                ("working text/bg", t.working, t.bg), ("working text/surface", t.working, t.surface),
            ];
            for (name, fg, bg) in text_pairs {
                let r = contrast(fg, bg);
                assert!(r >= 4.5, "{} theme: {name} is {r:.2}:1 (< 4.5)", if dark { "dark" } else { "light" });
            }
            let ui_pairs = [
                ("control_border/surface", t.control_border, t.surface), ("control_border/bg", t.control_border, t.bg),
                ("accent_fill/surface (switch ON)", t.accent_fill, t.surface),
                ("working dot/sidebar", t.working, t.sidebar),
                ("focus_ring/bg", t.focus_ring, t.bg), ("focus_ring/surface", t.focus_ring, t.surface),
                ("focus_ring/sidebar", t.focus_ring, t.sidebar), ("focus_ring/accent_soft", t.focus_ring, t.accent_soft),
            ];
            for (name, fg, bg) in ui_pairs {
                let r = contrast(fg, bg);
                assert!(r >= 3.0, "{} theme: {name} is {r:.2}:1 (< 3.0)", if dark { "dark" } else { "light" });
            }
        }
        DARK.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn fits(page: usize, step: usize, m: &SharedUiModel) {
        let (items, ctrls) = build(page, step, m, false);
        for c in &ctrls {
            assert!(rect_ok(&c.r), "page {page} step {step} ctrl {} out of bounds: {:?}", c.id, c.r);
        }
        for it in &items {
            if let Item::Group { r } | Item::Card { r } | Item::Banner { r, .. } = it {
                assert!(rect_ok(r), "page {page} step {step} item out of bounds: {r:?}");
            }
        }
        let content: Vec<&Ctrl> = ctrls.iter().filter(|c| !matches!(c.kind, Kind::Nav { .. })).collect();
        for (i, a) in content.iter().enumerate() {
            for b in content.iter().skip(i + 1) {
                let overlap = a.r.x < b.r.x + b.r.w && b.r.x < a.r.x + a.r.w && a.r.y < b.r.y + b.r.h && b.r.y < a.r.y + a.r.h;
                assert!(!overlap, "page {page} step {step}: {} overlaps {}", a.id, b.id);
            }
        }
    }

    #[test]
    fn dictionary_editor_fits_at_every_size_and_shows_at_most_one_page() {
        for terms in [0usize, 1, 7, 8, 9, 17, DICT_MAX_TERMS] {
            let mut m = model();
            m.config.dictionary.vocabulary = (0..terms).map(|i| format!("Term{i}")).collect();
            for page in [0usize, 1, 99] {
                VOCAB_VIEW.with(|v| v.set(Some(page)));
                fits(PAGE_WRITING, 0, &m);
                let (_, ctrls) = build(PAGE_WRITING, 0, &m, false);
                let rows = ctrls.iter().filter(|c| (V_TERM..V_TERM + DICT_ROWS as i32).contains(&c.id)).count();
                assert!((1..=DICT_ROWS).contains(&rows), "{terms} terms, page {page}: {rows} rows");
                assert_eq!(ctrls.iter().any(|c| c.id == V_ADD && c.enabled), terms < DICT_MAX_TERMS);
            }
        }
        VOCAB_VIEW.with(|v| v.set(None));
    }

    #[test]
    fn setup_practice_step_fits_in_every_state() {
        let mut m = model();
        for ready in [false, true] {
            if ready {
                m.installed_asr_models = vec!["parakeet-tdt-0.6b-v2-int8".into()];
            }
            for state in [EngineState::Idle, EngineState::Listening, EngineState::Processing, EngineState::ReadyToPaste] {
                m.state = state;
                for (heard, note) in [(false, 0u8), (true, 0), (false, 1), (false, 3)] {
                    PRACTICE_HEARD.with(|c| c.set(heard));
                    PRACTICE_NOTE.with(|c| c.set(note));
                    fits(PAGE_START, 3, &m);
                }
            }
        }
        PRACTICE_HEARD.with(|c| c.set(false));
        PRACTICE_NOTE.with(|c| c.set(0));
    }

    #[test]
    fn dictionary_rows_map_terms_to_spellings_without_losing_data() {
        let mut d = vi_core::PersonalDictionary::default();
        // A new term with spellings.
        assert!(apply_dict_row(&mut d, None, " Kubernetes ", "koober netties, cube er nettys"));
        assert_eq!(d.vocabulary, vec!["Kubernetes".to_string()]);
        assert_eq!(dict_entries(&d), vec![DictEntry { term: "Kubernetes".into(), sounds: vec!["koober netties".into(), "cube er nettys".into()] }]);
        assert!(d.enabled);
        // The same texts again change nothing (this is what a focus change without typing does).
        assert!(!apply_dict_row(&mut d, Some(0), "Kubernetes", "koober netties, cube er nettys"));
        // Renaming carries the spellings over; clearing the term text keeps the old term.
        assert!(apply_dict_row(&mut d, Some(0), "K8s", "koober netties, cube er nettys"));
        assert!(d.replacements.iter().all(|r| r.written == "K8s"));
        assert!(!apply_dict_row(&mut d, Some(0), "", "koober netties, cube er nettys"));
        assert_eq!(d.vocabulary, vec!["K8s".to_string()]);
        // A duplicate term is refused, an empty blank row adds nothing.
        assert!(apply_dict_row(&mut d, None, "KeyReed", ""));
        assert!(!apply_dict_row(&mut d, Some(1), "k8s", ""));
        assert!(!apply_dict_row(&mut d, None, "  ", "x"));
        // Emptying the spellings removes their rules only.
        assert!(apply_dict_row(&mut d, Some(0), "K8s", ""));
        assert!(d.replacements.is_empty() && d.vocabulary.len() == 2);
        // A rule written elsewhere still gets a row, and Remove takes the term and its rules.
        d.upsert_replacement("tea cee pee", "TCP", true);
        assert_eq!(dict_entries(&d).len(), 3);
        remove_dict_entry(&mut d, 2);
        assert!(d.replacements.is_empty());
        remove_dict_entry(&mut d, 0);
        remove_dict_entry(&mut d, 0);
        assert!(d.vocabulary.is_empty() && !d.enabled);
        // The term limit holds.
        d.vocabulary = (0..DICT_MAX_TERMS).map(|i| format!("t{i}")).collect();
        assert!(!apply_dict_row(&mut d, None, "one more", ""));
    }

    #[test]
    fn status_chip_uses_the_working_colour_and_shows_the_download() {
        let t = theme();
        let mut m = model();
        m.state = EngineState::Processing;
        assert_eq!(status_chip(&m, &t), ("Transcribing…".to_string(), t.working));
        m.state = EngineState::Idle;
        assert_eq!(status_chip(&m, &t).0, "Speech model needed");
        m.download = Some(crate::ui::DownloadProgress { model_id: "m".into(), stage: DownloadStage::Downloading, done_bytes: 25, total_bytes: Some(100) });
        assert_eq!(status_chip(&m, &t), ("Downloading model 25%".to_string(), t.working));
        m.download = Some(crate::ui::DownloadProgress { model_id: "m".into(), stage: DownloadStage::Failed("x".into()), done_bytes: 0, total_bytes: None });
        assert_eq!(status_chip(&m, &t).0, "Model download failed");
    }

    #[test]
    fn vocabulary_parsing_trims_and_drops_empties() {
        assert_eq!(parse_vocab(" KeyReed, ,kubectl ,"), vec!["KeyReed".to_string(), "kubectl".to_string()]);
    }
}
