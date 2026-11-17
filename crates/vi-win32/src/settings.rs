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
    asr_ui_info, friendly_error, hotkey_conflict, hotkey_presets, DownloadStage, MicVerdict,
    SharedUiModel, UiAction, ASR_UI_CATALOG,
};
use std::cell::RefCell;
use std::time::Instant;
use vi_core::brand::{PRODUCT_NAME, PRODUCT_TAGLINE, SETTINGS_WINDOW_CLASS, VERSION_STR};
use vi_core::{ActivationMode, AppConfig, EngineState, HotkeySpec, OutputMode, ProcessingProfile, WritingMode};
use vi_core::shortcut::{CaptureStep, ShortcutCapture, ShortcutRole};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, GetMonitorInfoW, InvalidateRect,
    MonitorFromPoint, SetBkColor, SetTextColor, HDC, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const PAGE_START: usize = 0;
pub const PAGE_GENERAL: usize = 1;
pub const PAGE_DICTATION: usize = 2;
pub const PAGE_WRITING: usize = 3;
pub const PAGE_OUTPUT: usize = 4;
pub const PAGE_AUDIO: usize = 5;
pub const PAGE_MODELS: usize = 6;
pub const PAGE_PROCESSING: usize = 7;
pub const PAGE_PRIVACY: usize = 8;
pub const PAGE_ABOUT: usize = 9;

/// (label, Segoe Fluent/MDL2 glyph)
const NAV: [(&str, char); 10] = [
    ("Get started", '\u{E80F}'),
    ("General", '\u{E713}'),
    ("Dictation", '\u{E765}'),
    ("Writing", '\u{E70F}'),
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
const SIDEBAR_W: i32 = 236;
const X0: i32 = SIDEBAR_W + 40;
const CW: i32 = WIN_W - X0 - 40;
const ROW_H: i32 = 56;
const ROW_H_SHORT: i32 = 46;
const PAD: i32 = 16;

// Control ids.
const ID_NAV: i32 = 100;
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
const W_MODE: i32 = 400;
const W_AUTOCTX: i32 = 410;
const W_VOCAB: i32 = 411;
const O_TYPE: i32 = 501;
const O_CLIP: i32 = 502;
const O_ASK: i32 = 503;
const O_COPYLAST: i32 = 505;
const A_MIC: i32 = 601;
const A_TEST: i32 = 602;
const M_ACT: i32 = 700;
const P_PROF: i32 = 800;
const P_KEY: i32 = 810;
const P_KEY_SAVE: i32 = 811;
const P_KEY_REMOVE: i32 = 812;
const P_LLM: i32 = 813;
const V_PWD: i32 = 850;
const V_REDACT: i32 = 851;
const V_CLIPHIST: i32 = 852;
const V_CLEAR_HIST: i32 = 853;
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

const IDT_SAVED: usize = 9001;
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
        Builder { items: Vec::new(), ctrls: Vec::new(), y: 28, group_start: None, rows_in_group: 0 }
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
        let w = 96;
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
    /// Logical scroll offset of the content column (non-zero only when the work area is too short).
    scroll: i32,
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
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
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
        // Size for the DPI of the monitor the window landed on, then centre it.
        let dpi = window_dpi(hwnd);
        let mut rc = RECT { left: 0, top: 0, right: sc(WIN_W, dpi), bottom: sc(WIN_H, dpi) };
        AdjustWindowRectExForDpi(&mut rc, style, 0, WS_EX_CONTROLPARENT, dpi);
        let (w, h) = (rc.right - rc.left, (rc.bottom - rc.top).min(wa.bottom - wa.top));
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
            return IsDialogMessageW(h, msg) != 0;
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
    // Navigation
    for (i, (label, _)) in NAV.iter().enumerate() {
        let y = 92 + i as i32 * 38;
        b.ctrl(ID_NAV + i as i32, Kind::Nav { page: i, selected: i == page }, r(12, y, SIDEBAR_W - 24, 36), Bg::Sidebar, label);
    }
    match page {
        PAGE_START => page_start(&mut b, step, m),
        PAGE_GENERAL => page_general(&mut b, m),
        PAGE_DICTATION => page_dictation(&mut b, m),
        PAGE_WRITING => page_writing(&mut b, m),
        PAGE_OUTPUT => page_output(&mut b, m),
        PAGE_AUDIO => page_audio(&mut b, m),
        PAGE_MODELS => page_models(&mut b, m),
        PAGE_PROCESSING => page_processing(&mut b, m),
        PAGE_PRIVACY => page_privacy(&mut b, m),
        _ => page_about(&mut b, m),
    }
    if page != PAGE_START {
        let (s, c) = if saved { ("Saved", C::Accent) } else { ("Changes save automatically", C::Text3) };
        b.text(r(X0, WIN_H - 34, CW, 18), s, F::Caption, c, DT_SINGLELINE | DT_RIGHT);
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
    if let Some(e) = &t.error {
        b.items.push(Item::Banner {
            r: r(X0 + PAD, y + 104, CW - 2 * PAD, 52),
            tone: Tone::Err,
            title: "Couldn't open the microphone".into(),
            body: friendly_error(e).message.into(),
        });
        h = 168;
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
    }
    b.y = y + h;
    b.end_group();
}

fn page_start(b: &mut Builder, step: usize, m: &SharedUiModel) {
    let hk = m.config.ptt_hotkey.display_name.clone();
    b.text(r(X0, b.y, CW, 18), format!("Setup · step {} of 4", step + 1), F::CaptionSemi, C::Text2, DT_SINGLELINE);
    b.items.push(Item::Steps { r: r(X0, b.y + 24, CW, 4), current: step, total: 4 });
    b.y += 44;
    match step {
        0 => {
            b.title("Welcome to KeyReed", "Talk instead of type, in any app. Speech is turned into text on this PC.");
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
            b.section("Your shortcut");
            b.group();
            let (items, sel, note, nc) = hotkey_items(m);
            b.combo_row(S_HOTKEY, "Push-to-talk shortcut", &note, nc, items, sel, 220);
            b.end_group();
            activation_choices(b, m, S_HOLD, S_TOGGLE);
            b.ctrl(S_SKIP, Kind::Button { style: BtnStyle::Subtle, label: "Skip setup".into() }, r(X0, WIN_H - 56, 110, 32), Bg::Content, "Skip setup");
            b.ctrl(S_NEXT, Kind::Button { style: BtnStyle::Primary, label: "Next".into() }, r(X0 + CW - 120, WIN_H - 56, 120, 32), Bg::Content, "Next");
        }
        1 => {
            b.title("Check your microphone", "Pick the microphone you'll speak into, then run a quick test.");
            b.group();
            let (items, sel) = mic_items(m);
            b.combo_row(S_MIC, "Microphone", "If it's unplugged, KeyReed uses the Windows default.", C::Text2, items, sel, 300);
            b.end_group();
            mic_test_block(b, m, S_TEST);
            nav_buttons(b, "Next");
        }
        2 => {
            b.title("Get the speech engine", "KeyReed turns speech into text on your PC. It's a one-time download.");
            let id = if m.active_model_ready() { m.config.local_asr_model.clone() } else { ASR_UI_CATALOG[0].id.to_string() };
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
            nav_buttons(b, "Next");
        }
        _ => {
            b.title("Try it", &format!("Click in the box, hold {hk}, say a sentence, then let go."));
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
            b.y += 120;
            b.group();
            let mode = m.config.default_writing_mode;
            b.row(&format!("Writing mode: {}", mode.label()), mode.description(), C::Text2, 0);
            let om = m.config.output_mode;
            let short = match om {
                OutputMode::DirectTyping => "Types straight into the app you're using.",
                OutputMode::Clipboard => "Copies the text; you paste it with Ctrl+V.",
                OutputMode::AskBeforeInserting => "Shows the text first and asks before inserting.",
            };
            b.row(&format!("Output: {}", om.title()), short, C::Text2, 0);
            b.end_group();
            b.text(r(X0, b.y, CW, 18), "You can change these any time under Writing and Output.", F::Caption, C::Text3, DT_SINGLELINE);
            nav_buttons(b, "Finish");
        }
    }
}

fn nav_buttons(b: &mut Builder, next: &str) {
    b.ctrl(S_BACK, Kind::Button { style: BtnStyle::Secondary, label: "Back".into() }, r(X0 + CW - 248, WIN_H - 56, 120, 32), Bg::Content, "Back");
    b.ctrl(S_NEXT, Kind::Button { style: BtnStyle::Primary, label: next.into() }, r(X0 + CW - 120, WIN_H - 56, 120, 32), Bg::Content, next);
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
    b.toggle_row(D_QF_REVIEW, "Review Quick Fix changes first", "Shows every change before anything is replaced. Turning this off lets KeyReed replace spelling fixes it is sure about, with Undo.", c.quick_fix_always_review, true);
    b.toggle_row(D_PTT, "Listen for the shortcuts", "When off, start dictation from the floating listener instead (its right-click menu, or a click if you set that in General).", c.ptt_enabled, true);
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
    let (y0, h) = b.row("Personal dictionary", "Names and terms to spell your way, separated by commas.", C::Text2, 250);
    b.ctrl(
        W_VOCAB,
        Kind::Edit { text: c.dictionary.vocabulary.join(", "), password: false, multiline: false, cue: "e.g. KeyReed, Kubernetes".into() },
        r(X0 + CW - PAD - 250, y0 + (h - 30) / 2, 250, 30),
        Bg::Surface,
        "Personal dictionary",
    );
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
    b.section("Input device");
    b.group();
    let (items, sel) = mic_items(m);
    b.combo_row(A_MIC, "Microphone", "If it's unplugged, KeyReed switches to the Windows default.", C::Text2, items, sel, 300);
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
        Some(DownloadStage::Failed(_)) if !installed => ("Retry", BtnStyle::Primary, !other_active),
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
    let dy = if matches!(c.kind, Kind::Nav { .. }) { 0 } else { cx.scroll };
    let cr = child_rect(c);
    let (x, y, w, h) = (sc(cr.x, d), sc(cr.y - dy, d), sc(cr.w, d), sc(cr.h, d));
    let (cls, style, ex, text): (&str, u32, u32, String) = match &c.kind {
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
    }
    match &c.kind {
        Kind::Edit { cue, .. } => {
            if !cue.is_empty() {
                let w = to_wide(cue);
                SendMessageW(hwnd, EM_SETCUEBANNER, 1, w.as_ptr() as LPARAM);
            }
        }
        _ => {
            // Reference data: 1 = drop-down field, 2 = shortcut capture field.
            let kind = match c.kind {
                Kind::Combo { .. } => 1,
                Kind::Shortcut { .. } => 2,
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

/// Copies edit contents into `St` (practice text, unsaved key) and returns the dictionary text,
/// which the caller commits *after* releasing the borrow. Reading child text only messages the
/// child controls, never `wnd_proc`.
fn harvest_edits(st: &mut St) -> Option<String> {
    let mut vocab = None;
    for (c, h) in &st.ctrls {
        match c.id {
            S_PRACTICE => st.practice_text = read_text(*h),
            P_KEY => st.key_draft = read_text(*h),
            W_VOCAB => vocab = Some(read_text(*h)),
            _ => {}
        }
    }
    vocab
}

fn harvest_and_commit() {
    let vocab = ST.with(|s| s.borrow_mut().as_mut().and_then(harvest_edits));
    if let Some(v) = vocab {
        save_vocab(&v);
    }
}

fn parse_vocab(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn save_vocab(v: &str) {
    let list = parse_vocab(v);
    let (current, enabled) = snapshot()
        .map(|m| (m.config.dictionary.vocabulary, m.config.dictionary.enabled))
        .unwrap_or_default();
    // Words typed here must actually be used: the dictionary is on whenever it has entries
    // (it shipped disabled by default, so entries were silently ignored before v0.4.0).
    let want_enabled = !list.is_empty();
    if list != current || enabled != want_enabled {
        commit(|c| {
            c.dictionary.vocabulary = list;
            c.dictionary.enabled = want_enabled;
        });
    }
}

/// Full rebuild of page controls (page switch / DPI change / structure change).
fn rebuild(focus_first: bool) {
    let Some(m) = snapshot() else { return };
    // Phase 1: compute under the borrow and take ownership of the old controls.
    let prep = ST.with(|s| {
        let mut sb = s.borrow_mut();
        let st = sb.as_mut()?;
        let saved = st.saved_at.map(|t| t.elapsed().as_millis() < 1800).unwrap_or(false);
        let (items, ctrls) = build(st.page, st.step, &m, saved);
        let old: Vec<(Ctrl, HWND)> = st.ctrls.drain(..).collect();
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
        let target = if focus_first {
            created.iter().find(|(c, _)| c.id == ID_NAV + page as i32).map(|(_, h)| *h)
        } else {
            focused_id.and_then(|id| created.iter().find(|(c, _)| c.id == id).map(|(_, h)| *h))
        };
        // Phase 3: store.
        ST.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.ctrls = created;
            }
        });
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
        return;
    }
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

fn status_chip(m: &SharedUiModel) -> (String, u32) {
    match &m.state {
        EngineState::Error(e) => (friendly_error(e).short.to_string(), rgb(0xE1, 0x1D, 0x48)),
        EngineState::Disabled => ("Paused".into(), rgb(0x8A, 0x94, 0x90)),
        EngineState::Listening => ("Listening…".into(), rgb(0x10, 0xB9, 0x81)),
        EngineState::Processing | EngineState::Correcting => ("Transcribing…".into(), rgb(0xF5, 0x9E, 0x0B)),
        _ if !m.active_model_ready() => ("Speech model needed".into(), rgb(0xF5, 0x9E, 0x0B)),
        _ => (format!("Ready · hold {}", m.config.ptt_hotkey.display_name), rgb(0x10, 0xB9, 0x81)),
    }
}

unsafe fn paint(st: &St, hdc: HDC) {
    let t = theme();
    let d = st.dpi;
    let s = |v: i32| sc(v, d);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(st.hwnd, &mut rc);
    let Some(mut cv) = Canvas::new(rc.right, rc.bottom) else { return };
    cv.fill(t.bg);
    cv.rect(0, 0, s(SIDEBAR_W), rc.bottom, t.sidebar);
    cv.rect(s(SIDEBAR_W), 0, 1.max(s(1)), rc.bottom, t.border);

    // Brand block
    let icon_n = s(30) as usize;
    blit_icon(&mut cv, s(22), s(26), icon_n);
    cv.text(PRODUCT_NAME, s(62), s(24), s(160), s(22), st.fonts.get(F::Brand), t.text, DT_SINGLELINE);
    cv.text(PRODUCT_TAGLINE, s(62), s(46), s(170), s(18), st.fonts.get(F::Caption), t.text3, DT_SINGLELINE);

    // Status chip at the bottom of the sidebar
    // Skipped when the window is so short (e.g. 200% scaling on a 1080p display) that the chip would overlap the last nav item.
    let nav_bottom = s(92 + NAV.len() as i32 * 38);
    if let Some(m) = snapshot().filter(|_| rc.bottom - s(52) >= nav_bottom) {
        let (label, dot) = status_chip(&m);
        let y = rc.bottom - s(52);
        cv.round_rect(s(12) as f32, y as f32, s(SIDEBAR_W - 24) as f32, s(36) as f32, s(8) as f32, t.surface, 1.0);
        cv.round_rect_stroke(s(12) as f32, y as f32, s(SIDEBAR_W - 24) as f32, s(36) as f32, s(8) as f32, 1.0, t.border, 1.0);
        cv.circle(s(28) as f32, (y + s(18)) as f32, s(4) as f32, dot, 1.0);
        cv.text(&label, s(40), y, s(SIDEBAR_W - 64), s(36), st.fonts.get(F::Caption), t.text2, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }

    let off = st.scroll;
    let s = |v: i32| sc(v, d);
    let sy = |v: i32| sc(v - off, d);
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
    if off > 0 {
        // Content scrolled under the top edge: keep the sidebar crisp and on top.
        cv.rect(0, 0, s(SIDEBAR_W), rc.bottom, t.sidebar);
        cv.rect(s(SIDEBAR_W), 0, 1.max(s(1)), rc.bottom, t.border);
        blit_icon(&mut cv, s(22), s(26), icon_n);
        cv.text(PRODUCT_NAME, s(62), s(24), s(160), s(22), st.fonts.get(F::Brand), t.text, DT_SINGLELINE);
        cv.text(PRODUCT_TAGLINE, s(62), s(46), s(170), s(18), st.fonts.get(F::Caption), t.text3, DT_SINGLELINE);
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
            let label = if *on { "On" } else { "Off" };
            cv.text(label, 0, 0, (sx as i32) - s(10), h, st.fonts.get(F::Body), if disabled { t.text3 } else { t.text2 }, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
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
        _ => {}
    }
    if focus {
        let (x, y, fw, fh, fr) = focus_rect;
        cv.round_rect_stroke(x, y, fw, fh, fr, 2.0_f32.max(s(2) as f32), t.focus, 1.0);
    }
    cv.blit_to(di.hdc, di.rc.left, di.rc.top);
}

// ------------------------------------------------------------------------------------------------
// Commands
// ------------------------------------------------------------------------------------------------

/// Scrolls the content column (only possible when the window is shorter than the layout).
fn scroll_by(delta: i32) {
    let moved = ST.with(|s| {
        let mut sb = s.borrow_mut();
        let st = sb.as_mut()?;
        let max = (WIN_H - st.view_h).max(0);
        let new = (st.scroll + delta).clamp(0, max);
        if new == st.scroll {
            return None;
        }
        st.scroll = new;
        let moves: Vec<(HWND, i32, i32)> = st
            .ctrls
            .iter()
            .filter(|(c, _)| !matches!(c.kind, Kind::Nav { .. }))
            .map(|(c, h)| {
                let cr = child_rect(c);
                (*h, sc(cr.x, st.dpi), sc(cr.y - new, st.dpi))
            })
            .collect();
        Some((st.hwnd, moves))
    });
    if let Some((hwnd, moves)) = moved {
        unsafe {
            for (h, x, y) in moves {
                SetWindowPos(h, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
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
    let click = code == BN_CLICKED || code == BN_DOUBLECLICKED;
    if code == EN_SETFOCUS || code == EN_KILLFOCUS {
        unsafe { InvalidateRect(hwnd(), std::ptr::null(), 0) };
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
    if (M_ACT..M_ACT + 6).contains(&id) && click {
        model_action(&m, ASR_UI_CATALOG[(id - M_ACT) as usize].id);
        return;
    }
    if (P_PROF..P_PROF + 3).contains(&id) && click {
        let p = [ProcessingProfile::MaxPrivacyOffline, ProcessingProfile::Balanced, ProcessingProfile::MaxAccuracy][(id - P_PROF) as usize];
        commit(|c| c.processing_profile = p);
        return;
    }

    match (id, click, code) {
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
        (G_PILL_RESET, true, _) => unsafe { crate::ui::win_gui::reset_pill_position() },
        (G_SOUNDS, true, _) => {
            let v = !c.audio_cues_enabled;
            commit(|c| c.audio_cues_enabled = v)
        }
        (G_NOTIFY, true, _) => {
            let v = !c.notify_on_ready;
            commit(|c| c.notify_on_ready = v)
        }
        (D_HOTKEY, true, _) | (D_QF_KEY, true, _) => start_capture(id),
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
        (W_VOCAB, _, EN_KILLFOCUS) => {
            let h = unsafe { GetDlgItem(hwnd(), W_VOCAB) };
            save_vocab(&read_text(h));
        }
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
        (AB_RESET, true, _) => unsafe {
            let title = to_wide("Reset all settings?");
            let body = to_wide("KeyReed will restore its default settings. Downloaded speech models are kept.\n\nReset now?");
            let r = MessageBoxW(hwnd(), body.as_ptr(), title.as_ptr(), MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2);
            if r == IDOK {
                send(UiAction::ResetToDefaults);
                flash_saved();
            }
        },
        (S_DL, true, _) => {
            let id = if m.active_model_ready() { c.local_asr_model.clone() } else { ASR_UI_CATALOG[0].id.to_string() };
            model_action(&m, &id);
        }
        (S_ALLMODELS, true, _) => navigate(PAGE_MODELS),
        (S_SKIP, true, _) => finish_setup(false),
        (S_BACK, true, _) => {
            let step = ST.with(|s| s.borrow().as_ref().map(|x| x.step).unwrap_or(0));
            set_step(step.saturating_sub(1));
        }
        (S_NEXT, true, _) => {
            let step = ST.with(|s| s.borrow().as_ref().map(|x| x.step).unwrap_or(0));
            if step >= 3 {
                finish_setup(true);
            } else {
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

unsafe extern "system" fn btn_subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, is_select: usize) -> LRESULT {
    const VK_UP: usize = 0x26;
    const VK_DOWN: usize = 0x28;
    const VK_F4: usize = 0x73;
    if is_select == 2 {
        if let Some(r) = shortcut_field_msg(hwnd, msg, wparam, lparam) {
            return r;
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
    // Windows 11: tint the caption to match the sidebar and use the brand border. Ignored on Windows 10.
    let caption: u32 = t.sidebar;
    let border: u32 = t.border;
    let dark = DARK.load(std::sync::atomic::Ordering::Relaxed) as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as *const _, 4); // DWMWA_USE_IMMERSIVE_DARK_MODE
    DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as *const _, 4);
    DwmSetWindowAttribute(hwnd, 34, &border as *const u32 as *const _, 4);
    let big = brand_hicon(32);
    let small = brand_hicon(20);
    SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as LPARAM);
    SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as LPARAM);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            refresh_dark_mode();
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
                    view_h: WIN_H,
                    anims: Vec::new(),
                })
            });
            apply_window_chrome(hwnd);
            rebuild(true);
            0
        }
        WM_ERASEBKGND => 1,
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
            if id == IDCANCEL as i32 {
                // Esc closes Settings (but not while the user is dictating into the practice box).
                let listening = snapshot().map(|m| matches!(m.state, EngineState::Listening | EngineState::Processing | EngineState::Correcting)).unwrap_or(false);
                if !listening {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
                return 0;
            }
            if id == IDOK as i32 {
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
                    st.scroll = st.scroll.clamp(0, (WIN_H - st.view_h).max(0));
                }
            });
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            scroll_by(-(delta * 56) / 120);
            0
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
        }
    }

    fn rect_ok(rr: &R) -> bool {
        rr.x >= 0 && rr.y >= 0 && rr.x + rr.w <= WIN_W && rr.y + rr.h <= WIN_H
    }

    #[test]
    fn every_page_and_step_fits_the_window_without_overlap() {
        let mut m = model();
        for variant in 0..3 {
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
        assert_eq!(ctrls.iter().filter(|c| (M_ACT..M_ACT + 6).contains(&c.id)).count(), 6);
        let (_, ctrls) = build(PAGE_OUTPUT, 0, &m, false);
        assert_eq!(ctrls.iter().filter(|c| [O_TYPE, O_CLIP, O_ASK].contains(&c.id)).count(), 3);
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
        assert_eq!(label(M_ACT), "In use");
        assert_eq!(label(M_ACT + 2), "Use");
        assert_eq!(label(M_ACT + 5), "Cancel");
        assert_eq!(label(M_ACT + 3), "Download");
        assert!(!ctrls.iter().find(|c| c.id == M_ACT + 3).unwrap().enabled, "one download at a time");
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
            ];
            for (name, fg, bg) in text_pairs {
                let r = contrast(fg, bg);
                assert!(r >= 4.5, "{} theme: {name} is {r:.2}:1 (< 4.5)", if dark { "dark" } else { "light" });
            }
            let ui_pairs = [
                ("control_border/surface", t.control_border, t.surface), ("control_border/bg", t.control_border, t.bg),
                ("accent_fill/surface (switch ON)", t.accent_fill, t.surface),
            ];
            for (name, fg, bg) in ui_pairs {
                let r = contrast(fg, bg);
                assert!(r >= 3.0, "{} theme: {name} is {r:.2}:1 (< 3.0)", if dark { "dark" } else { "light" });
            }
        }
        DARK.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn vocabulary_parsing_trims_and_drops_empties() {
        assert_eq!(parse_vocab(" KeyReed, ,kubectl ,"), vec!["KeyReed".to_string(), "kubectl".to_string()]);
    }
}
