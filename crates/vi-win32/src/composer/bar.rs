//! The Composer (plan C3, F2): a small borderless bar that blooms out of the pill. Row 1 holds the context chip
//! and the route label, row 2 the input (a native edit control, so input methods and screen readers work) with
//! the microphone button, row 3 the preset chips. A result streams in under the input word by word and becomes a
//! native edit box when the user clicks it.
//!
//! Keyboard (F2): Enter submits, or with a result replaces the selection (inserts when there was none);
//! Ctrl+Enter submits, or copies a result; Ctrl+R regenerates; Tab moves through the chips; Esc closes, or stops a
//! running request and keeps the prompt.
//!
//! A click outside the bar (another app, the desktop, the pill) closes it only when nothing would be lost: no
//! request running and nothing typed or shown. Otherwise Esc closes it (stopping a request first).
//!
//! Drag: row 1 outside the context chip moves the bar (HTCAPTION); the resting rectangle follows every move, so
//! painting, the native children and later resizes stay on the new spot instead of snapping back.
//!
//! Motion (F4): opening, the window grows out of the pill to the bar's size while the content stays put (it is
//! revealed, not squeezed), the chips arrive 20 ms apart and the caret appears when the bloom ends; closing folds
//! back into the pill faster; streamed words rise 4 px and fade in. Reduced motion: a short cross-fade only.
//! Nothing animates while the bar is idle. Text lives in this window only and is never logged.

use super::{ease_out, layout, push_text, sc, Placed, Tok};
use crate::canvas::{text_width, Canvas, DT_CENTER, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER};
use crate::settings::{refresh_dark_mode, theme, Fonts, F};
use crate::ui::win_gui::{dpi_override, globals, reduced_motion, send, to_wide};
use crate::ui::{DeliverHow, UiAction};
use std::cell::RefCell;
use std::time::Instant;
use vi_core::textjob::Instruction;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint,
    SetBkColor, SetTextColor, HBRUSH, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_TAB};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const COMPOSER_WINDOW_CLASS: &str = "KeyreedComposerClass";

// Logical (96-DPI) geometry.
const W: i32 = 520;
const MAX_H: i32 = 360;
const PAD: i32 = 16;
const ROW1_Y: i32 = 12;
const ROW1_H: i32 = 24;
const INPUT_Y: i32 = 44;
const INPUT_H: i32 = 44;
const GAP: i32 = 10;
const CHIP_H: i32 = 30;
const ACTION_H: i32 = 34;
const LINE_H: i32 = 22;
const CARD_PAD: i32 = 12;
const RESULT_MAX: i32 = 150;

const BLOOM_MS: f32 = 200.0;
const FOLD_MS: f32 = 140.0;
const FADE_MS: f32 = 100.0;
const CHIP_STAGGER_MS: f32 = 20.0;
const CHIP_IN_MS: f32 = 120.0;
const WORD_MS: f32 = 160.0;

const IDT_BAR_ANIM: usize = 9201;
const ID_INPUT: i32 = 10;
const ID_RESULT: i32 = 11;
const EM_SETREADONLY: u32 = 0x00CF;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_SETSEL: u32 = 0x00B1;

/// The presets (C3: at most six; the sixth, Custom, is whatever is typed into the input).
pub const PRESETS: [&str; 5] = ["Fix", "Improve", "Shorter", "Change tone", "Summarise"];

fn preset(i: usize) -> Instruction {
    match i {
        0 => Instruction::Fix,
        1 => Instruction::Improve,
        2 => Instruction::Shorter,
        3 => Instruction::Tone,
        _ => Instruction::Summarise,
    }
}

/// What the Composer opens with.
#[derive(Clone, Debug, Default)]
pub struct ComposerRequest {
    /// The selection captured when the Composer opened (memory only), or `None`.
    pub selection: Option<String>,
    /// Where it came from, for the chip ("Outlook").
    pub app: String,
    /// Where the request will run: "On this PC · Qwen3.5 2B" or "Cloud · api.example.com · model".
    pub route: String,
    /// The route is the user's cloud endpoint (the label gets the cloud colour).
    pub cloud: bool,
    /// No writing model is installed and the cloud is off: requests cannot run; the bar says so.
    pub unavailable: bool,
}

/// Messages from the coordinator, applied in order on the GUI thread.
#[derive(Clone, Debug)]
pub enum ComposerMsg {
    Open(ComposerRequest),
    Token(String),
    Done(String),
    /// A dictated transcript: it goes into the input, ready to edit or send. The bar stays open.
    Dictated(String),
    /// The microphone is recording into the input (the mic chip shows the listening state).
    Listening(bool),
    /// One plain sentence; `settings` adds an "Open Settings" action (missing writing model).
    Failed { message: String, settings: bool },
}

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Empty,
    /// The text so far and when each word arrived (for the rise-and-fade).
    Streaming { text: String, arrived: Vec<Instant> },
    Result { text: String },
    Failed { message: String, settings: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Chip(usize),
    RemoveContext,
    Mic,
    Deliver,
    Copy,
    Regenerate,
    Settings,
    ResultText,
}

struct Bar {
    hwnd: HWND,
    input: HWND,
    /// The native edit that replaces the canvas result once the user clicks it.
    result_edit: HWND,
    req: ComposerRequest,
    use_selection: bool,
    stage: Stage,
    /// Keyboard focus on a chip (None: the input has it).
    chip_focus: Option<usize>,
    hover: Option<Hit>,
    /// The microphone is recording into the input.
    listening: bool,
    opened: Instant,
    closing: Option<Instant>,
    /// Screen rectangles: where the bloom starts (the pill) and the bar's resting place.
    from: RECT,
    to: RECT,
    dpi: u32,
    fonts: Fonts,
    brush: HBRUSH,
    /// The last word layout of the result and the text it was made for (measuring words is the costly part of a
    /// frame, and the text only changes when a token arrives).
    words: RefCell<(String, std::rc::Rc<(Vec<(Placed, String)>, i32)>)>,
}

thread_local! {
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
    /// The click-away watch's hook, while the bar is open.
    static CLICK_HOOK: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

/// A low-level mouse hook: the bar is layered and does not own another app's windows or the desktop, so a press is
/// the only reliable sign of a click outside it (a lost activation can happen with nobody clicking).
unsafe extern "system" fn click_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam != 0 && wparam as u32 == WM_LBUTTONDOWN {
        let pt = (*(lparam as *const MSLLHOOKSTRUCT)).pt;
        let outside = peek(|b| {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(b.hwnd, &mut r);
            b.closing.is_none() && !inside(&r, pt.x, pt.y)
        })
        .unwrap_or(false);
        if outside {
            outside_click();
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

fn watch_clicks() {
    CLICK_HOOK.with(|c| {
        if c.get() == 0 {
            c.set(unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(click_proc), std::ptr::null_mut(), 0) } as isize);
        }
    });
}

fn unwatch_clicks() {
    CLICK_HOOK.with(|c| {
        let h = c.replace(0);
        if h != 0 {
            unsafe { UnhookWindowsHookEx(h as HHOOK) };
        }
    });
}

/// Reads the bar without risking a second borrow (window messages can arrive while it is being changed).
fn peek<R>(f: impl FnOnce(&Bar) -> R) -> Option<R> {
    BAR.with(|b| b.try_borrow().ok().and_then(|bb| bb.as_ref().map(f)))
}

// ---------------------------------------------------------------------------------------------------------------
// Pure helpers (tested below)
// ---------------------------------------------------------------------------------------------------------------

/// "Using selection, 412 characters from Outlook".
pub fn context_label(selection: &str, app: &str) -> String {
    let n = selection.chars().count();
    let from = if app.is_empty() { String::new() } else { format!(" from {app}") };
    format!("Using selection, {n} character{}{from}", if n == 1 { "" } else { "s" })
}

/// "OUTLOOK.EXE" -> "Outlook", "msedge.exe" -> "Msedge": a short name for the context chip.
pub fn app_label(process_name: &str) -> String {
    let base = process_name.rsplit(['/', std::path::MAIN_SEPARATOR]).next().unwrap_or("");
    let stem = match base.len().checked_sub(4) {
        Some(i) if base.is_char_boundary(i) && base[i..].eq_ignore_ascii_case(".exe") => &base[..i],
        _ => base,
    };
    let mut chars = stem.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
        None => String::new(),
    }
}

/// How many whole words of `text` have arrived; the arrival list grows to match, stamped `now`.
fn note_arrivals(arrived: &mut Vec<Instant>, text: &str, now: Instant) {
    let words = text.split_whitespace().count();
    while arrived.len() < words {
        arrived.push(now);
    }
}

/// Linear interpolation of two COLORREFs.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| -> u32 {
        let (x, y) = (((a >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(0) | ch(8) | ch(16)
}

/// The window rectangle `t` (0..1) of the way from `a` to `b`.
fn lerp_rect(a: &RECT, b: &RECT, t: f32) -> RECT {
    let l = |x: i32, y: i32| x + ((y - x) as f32 * t).round() as i32;
    RECT { left: l(a.left, b.left), top: l(a.top, b.top), right: l(a.right, b.right), bottom: l(a.bottom, b.bottom) }
}

/// Where the bar rests: centred on the pill, on whichever side of it has room, kept inside the work area. The
/// edge nearest the pill stays fixed when the bar grows.
fn place(pill: &RECT, work: &RECT, w: i32, h: i32, gap: i32) -> RECT {
    let cx = (pill.left + pill.right) / 2;
    let left = (cx - w / 2).clamp(work.left, (work.right - w).max(work.left));
    let below = pill.bottom + gap + h <= work.bottom;
    let above = pill.top - gap - h >= work.top;
    let top = if below && (!above || pill.top - work.top < work.bottom - pill.bottom) {
        pill.bottom + gap
    } else if above {
        pill.top - gap - h
    } else {
        (work.bottom - h).max(work.top)
    };
    RECT { left, top, right: left + w, bottom: top + h }
}

/// A click outside the bar closes it only when nothing would be lost: nothing is on screen (no request running, no
/// result, no failure to read) and nothing is typed. Otherwise Esc closes it, so typed text is never thrown away.
pub fn outside_click_closes(has_content: bool, typed: &str) -> bool {
    !has_content && typed.trim().is_empty()
}

/// A preset works on text: the kept selection, the latest result, or the draft typed into the input. It is dead only
/// while a request runs (the bar dims it then) or when there is nothing at all to work on.
fn presets_ready(has_selection: bool, has_result: bool, draft: &str, streaming: bool) -> bool {
    !streaming && (has_selection || has_result || !draft.trim().is_empty())
}

// ---------------------------------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------------------------------

struct Geo {
    w: i32,
    h: i32,
    input: RECT,
    mic: RECT,
    card: Option<RECT>,
    chips: Vec<RECT>,
    actions: Vec<(Hit, RECT, String, &'static str)>,
    context_x: Option<RECT>,
}

fn rc(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

impl Bar {
    fn s(&self, v: i32) -> i32 {
        sc(v, self.dpi)
    }

    fn tw(&self, s: &str, f: F) -> i32 {
        text_width(s, self.fonts.get(f))
    }

    fn result_text(&self) -> Option<&str> {
        match &self.stage {
            Stage::Streaming { text, .. } | Stage::Result { text } => Some(text),
            _ => None,
        }
    }

    /// Height of the streamed text inside the card, in physical pixels.
    fn result_content_h(&self) -> i32 {
        let Some(text) = self.result_text() else { return self.s(LINE_H) };
        let laid = self.lay_words(text);
        if laid.0.is_empty() { self.s(LINE_H) } else { laid.1 }
    }

    fn lay_words(&self, text: &str) -> std::rc::Rc<(Vec<(Placed, String)>, i32)> {
        if let Ok(c) = self.words.try_borrow() {
            if c.0 == text {
                return c.1.clone();
            }
        }
        let laid = std::rc::Rc::new(self.measure_words(text));
        if let Ok(mut c) = self.words.try_borrow_mut() {
            *c = (text.to_string(), laid.clone());
        }
        laid
    }

    fn measure_words(&self, text: &str) -> (Vec<(Placed, String)>, i32) {
        let mut toks = Vec::new();
        push_text(&mut toks, text);
        let widths: Vec<i32> = toks.iter().map(|t| if let Tok::Word(w) = t { self.tw(w, F::Body) } else { 0 }).collect();
        let avail = self.s(W - 2 * PAD - 2 * CARD_PAD - 8);
        let (placed, h) = layout(&toks, &widths, self.tw(" ", F::Body).max(self.s(4)), avail, self.s(LINE_H));
        let words = placed.into_iter().filter_map(|p| if let Tok::Word(w) = &toks[p.tok] { Some((p, w.clone())) } else { None }).collect();
        (words, h)
    }

    fn geo(&self) -> Geo {
        let s = |v| self.s(v);
        let w = s(W);
        let input = rc(s(PAD), s(INPUT_Y), w - 2 * s(PAD), s(INPUT_H));
        let mic = rc(input.right - s(40), input.top + (s(INPUT_H) - s(32)) / 2, s(32), s(32));
        let mut y = input.bottom + s(GAP);
        let card = if self.stage == Stage::Empty {
            None
        } else {
            let inner = self.result_content_h().min(s(RESULT_MAX)).max(s(LINE_H));
            let r = rc(s(PAD), y, w - 2 * s(PAD), inner + 2 * s(CARD_PAD));
            y = r.bottom + s(GAP);
            Some(r)
        };
        let mut chips = Vec::new();
        let mut x = s(PAD);
        for label in PRESETS {
            let cw = self.tw(label, F::CaptionSemi) + s(26);
            chips.push(rc(x, y, cw, s(CHIP_H)));
            x += cw + s(8);
        }
        y += s(CHIP_H);
        let mut actions = Vec::new();
        let action_labels: Vec<(Hit, String, &'static str)> = match &self.stage {
            Stage::Result { .. } => {
                let deliver = if self.replaces() { "Replace" } else { "Insert" };
                vec![(Hit::Deliver, deliver.into(), "Enter"), (Hit::Copy, "Copy".into(), "Ctrl+Enter"), (Hit::Regenerate, "Again".into(), "Ctrl+R")]
            }
            Stage::Failed { settings: true, .. } => vec![(Hit::Settings, "Open Settings".into(), "")],
            _ => Vec::new(),
        };
        if !action_labels.is_empty() || matches!(self.stage, Stage::Streaming { .. }) {
            y += s(GAP);
            let mut right = w - s(PAD);
            for (hit, label, key) in action_labels {
                let bw = self.tw(&label, F::BodySemi) + if key.is_empty() { 0 } else { self.tw(key, F::Caption) + s(10) } + s(30);
                right -= bw;
                actions.push((hit, rc(right, y, bw, s(ACTION_H)), label, key));
                right -= s(8);
            }
            y += s(ACTION_H);
        }
        let h = (y + s(14)).min(s(MAX_H));
        let context_x = match (&self.req.selection, self.use_selection) {
            (Some(_), true) => {
                let label = context_label(self.req.selection.as_deref().unwrap_or(""), &self.req.app);
                let cw = self.tw(&label, F::Caption) + s(44);
                Some(rc(s(PAD) + cw - s(26), s(ROW1_Y) + (s(ROW1_H) - s(18)) / 2, s(18), s(18)))
            }
            _ => None,
        };
        Geo { w, h, input, mic, card, chips, actions, context_x }
    }

    fn replaces(&self) -> bool {
        self.use_selection && self.req.selection.is_some()
    }

    fn presets_enabled(&self) -> bool {
        presets_ready(self.replaces(), self.result_text().is_some(), &input_text(self), matches!(self.stage, Stage::Streaming { .. }))
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        let g = self.geo();
        if g.context_x.as_ref().is_some_and(|r| inside(r, x, y)) {
            return Some(Hit::RemoveContext);
        }
        if inside(&g.mic, x, y) {
            return Some(Hit::Mic);
        }
        for (hit, r, _, _) in &g.actions {
            if inside(r, x, y) {
                return Some(*hit);
            }
        }
        if !matches!(self.stage, Stage::Streaming { .. }) {
            if let Some(i) = g.chips.iter().position(|r| inside(r, x, y)) {
                return Some(Hit::Chip(i));
            }
        }
        if matches!(self.stage, Stage::Result { .. }) && g.card.as_ref().is_some_and(|r| inside(r, x, y)) {
            return Some(Hit::ResultText);
        }
        None
    }

    /// Motion progress at this instant: (window rect 0..1, alpha 0..1, ms since open), honouring reduced motion.
    fn motion(&self) -> (f32, f32, f32) {
        let reduced = reduced_motion();
        if let Some(c) = self.closing {
            let ms = c.elapsed().as_secs_f32() * 1000.0;
            let total = if reduced { FADE_MS } else { FOLD_MS };
            let t = (ms / total).clamp(0.0, 1.0);
            let shape = if reduced { 0.0 } else { 1.0 - ease_out(1.0 - t).max(0.0) };
            return (1.0 - shape, 1.0 - t, f32::INFINITY);
        }
        let ms = self.opened.elapsed().as_secs_f32() * 1000.0;
        if reduced {
            return (1.0, (ms / FADE_MS).clamp(0.0, 1.0), f32::INFINITY);
        }
        (ease_out(ms / BLOOM_MS), (ms / 90.0).clamp(0.0, 1.0), ms)
    }

    fn animating(&self) -> bool {
        if self.closing.is_some() {
            return true;
        }
        let (_, _, ms) = self.motion();
        let chips_done = BLOOM_MS * 0.5 + CHIP_STAGGER_MS * PRESETS.len() as f32 + CHIP_IN_MS;
        let words = match &self.stage {
            Stage::Streaming { arrived, .. } => arrived.last().is_some_and(|t| t.elapsed().as_secs_f32() * 1000.0 < WORD_MS),
            _ => false,
        };
        (ms.is_finite() && ms < chips_done.max(BLOOM_MS)) || (!reduced_motion() && words) || (reduced_motion() && self.opened.elapsed().as_secs_f32() * 1000.0 < FADE_MS)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------------------------------------

unsafe fn paint(b: &Bar, hdc: HDC, client_w: i32, client_h: i32, ox: i32, oy: i32) {
    let t = theme();
    let g = b.geo();
    let Some(mut cv) = Canvas::new(client_w.max(1), client_h.max(1)) else { return };
    cv.fill(t.bg);
    let s = |v| b.s(v);
    let f = |x: F| b.fonts.get(x);
    let one = DT_LEFT | DT_SINGLELINE | DT_VCENTER;
    let (_, _, ms) = b.motion();
    let at = |r: &RECT| (r.left + ox, r.top + oy, r.right - r.left, r.bottom - r.top);

    // Row 1: the context chip (removable) and the route label.
    let y1 = s(ROW1_Y) + oy;
    match (&b.req.selection, b.use_selection) {
        (Some(sel), true) => {
            let label = context_label(sel, &b.req.app);
            let cw = b.tw(&label, F::Caption) + s(44);
            cv.round_rect((s(PAD) + ox) as f32, y1 as f32, cw as f32, s(ROW1_H) as f32, s(ROW1_H) as f32 / 2.0, t.accent_soft, 1.0);
            cv.text(&label, s(PAD + 12) + ox, y1, cw, s(ROW1_H), f(F::Caption), t.accent, one);
            if let Some(xr) = &g.context_x {
                let (x, y, w, h) = at(xr);
                if b.hover == Some(Hit::RemoveContext) {
                    cv.circle((x + w / 2) as f32, (y + h / 2) as f32, w as f32 / 2.0, t.hover, 1.0);
                }
                cv.text("\u{E711}", x, y, w, h, f(F::IconSmall), t.accent, DT_CENTER | DT_SINGLELINE | DT_VCENTER);
            }
        }
        _ => {
            let label = if b.req.selection.is_some() { "Selection removed: KeyReed writes something new" } else { "Nothing selected: KeyReed writes something new" };
            cv.text(label, s(PAD + 2) + ox, y1, s(300), s(ROW1_H), f(F::Caption), t.text3, one);
        }
    }
    let route_w = b.tw(&b.req.route, F::Caption);
    let rx = g.w - s(PAD) - route_w + ox;
    cv.circle((rx - s(9)) as f32, (y1 + s(ROW1_H) / 2) as f32, s(3) as f32, if b.req.cloud { t.warn_fg } else { t.ok_fg }, 1.0);
    cv.text(&b.req.route, rx, y1, route_w + s(4), s(ROW1_H), f(F::Caption), t.text2, one);

    // Row 2: the input card (the edit control sits inside it) and the microphone.
    let (ix, iy, iw, ih) = at(&g.input);
    let focused = GetFocus() == b.input;
    cv.round_rect(ix as f32, iy as f32, iw as f32, ih as f32, s(10) as f32, t.surface, 1.0);
    let (bc, bw) = if focused { (t.accent_fill, (1.5 * b.dpi as f32 / 96.0).max(1.0)) } else { (t.control_border, 1.0) };
    cv.round_rect_stroke(ix as f32, iy as f32, iw as f32, ih as f32, s(10) as f32, bw, bc, 1.0);
    let (mx, my, mw, mh) = at(&g.mic);
    if b.listening {
        // Recording: the mic chip is the active control, and a second click cancels.
        cv.circle((mx + mw / 2) as f32, (my + mh / 2) as f32, mw as f32 / 2.0, t.accent_fill, 1.0);
    } else if b.hover == Some(Hit::Mic) {
        cv.circle((mx + mw / 2) as f32, (my + mh / 2) as f32, mw as f32 / 2.0, t.hover, 1.0);
    }
    let mic_fg = if b.listening { t.on_accent } else { t.text2 };
    cv.text("\u{E720}", mx, my, mw, mh, f(F::Icon), mic_fg, DT_CENTER | DT_SINGLELINE | DT_VCENTER);

    // The result card: streamed words rise 4 px and fade in; a failure is one plain sentence.
    if let Some(card) = &g.card {
        let (cx, cy, cw, ch) = at(card);
        let failed = matches!(b.stage, Stage::Failed { .. });
        cv.round_rect(cx as f32, cy as f32, cw as f32, ch as f32, s(10) as f32, if failed { t.err_bg } else { t.surface }, 1.0);
        if !failed {
            let editing = !b.result_edit.is_null() && GetFocus() == b.result_edit;
            let (bc, bw) = if editing {
                (t.accent_fill, (1.5 * b.dpi as f32 / 96.0).max(1.0))
            } else {
                (if b.hover == Some(Hit::ResultText) { t.control_border } else { t.border }, 1.0)
            };
            cv.round_rect_stroke(cx as f32, cy as f32, cw as f32, ch as f32, s(10) as f32, bw, bc, 1.0);
        }
        match &b.stage {
            Stage::Failed { message, .. } => {
                cv.text(message, cx + s(CARD_PAD), cy + s(CARD_PAD), cw - 2 * s(CARD_PAD), ch - 2 * s(CARD_PAD), f(F::Body), t.err_fg, crate::canvas::DT_WORDBREAK);
            }
            _ if b.result_edit.is_null() => {
                let text = b.result_text().unwrap_or("");
                let laid = b.lay_words(text);
                let words = &laid.0;
                let arrived = match &b.stage { Stage::Streaming { arrived, .. } => Some(arrived), _ => None };
                let (x0, y0) = (cx + s(CARD_PAD), cy + s(CARD_PAD));
                let bottom = cy + ch - s(CARD_PAD) + s(2);
                // Long results keep their newest lines in view.
                let shift = (words.last().map_or(0, |(p, _)| p.y + s(LINE_H)) - (ch - 2 * s(CARD_PAD))).max(0);
                for (i, (p, word)) in words.iter().enumerate() {
                    let y = y0 + p.y - shift;
                    if y < cy + s(4) || y + s(LINE_H) > bottom {
                        continue;
                    }
                    let k = match arrived.and_then(|a| a.get(i)) {
                        Some(t0) if !reduced_motion() => ease_out(t0.elapsed().as_secs_f32() * 1000.0 / WORD_MS),
                        _ => 1.0,
                    };
                    let dy = (s(4) as f32 * (1.0 - k)).round() as i32;
                    cv.text(word, x0 + p.x, y + dy, p.w + s(4), s(LINE_H), f(F::Body), mix(t.surface, t.text, k), one);
                }
                if text.trim().is_empty() {
                    cv.text("Writing", x0, y0, s(200), s(LINE_H), f(F::Body), t.text3, one);
                }
            }
            _ => {}
        }
    }

    // Row 3: preset chips, arriving one after another while the bar blooms.
    let enabled = b.presets_enabled();
    for (i, r) in g.chips.iter().enumerate() {
        let k = if ms.is_finite() { ease_out((ms - BLOOM_MS * 0.5 - CHIP_STAGGER_MS * i as f32) / CHIP_IN_MS) } else { 1.0 };
        if k <= 0.0 {
            continue;
        }
        let (x, y, w, h) = at(r);
        let y = y + (s(6) as f32 * (1.0 - k)).round() as i32;
        let hot = b.hover == Some(Hit::Chip(i)) && enabled;
        let fill = if hot { t.hover } else { t.bg };
        cv.round_rect(x as f32, y as f32, w as f32, h as f32, h as f32 / 2.0, mix(t.bg, fill, k), 1.0);
        cv.round_rect_stroke(x as f32, y as f32, w as f32, h as f32, h as f32 / 2.0, 1.0, mix(t.bg, if enabled { t.control_border } else { t.border }, k), 1.0);
        let fg = if enabled { t.text } else { t.text3 };
        cv.text(PRESETS[i], x, y, w, h, f(F::CaptionSemi), mix(t.bg, fg, k), DT_CENTER | DT_SINGLELINE | DT_VCENTER);
        if b.chip_focus == Some(i) {
            cv.round_rect_stroke(x as f32 - 2.0, y as f32 - 2.0, w as f32 + 4.0, h as f32 + 4.0, (h + 4) as f32 / 2.0, (2.0 * b.dpi as f32 / 96.0).max(2.0), t.focus, 1.0);
        }
    }

    // Actions, right-aligned; while streaming the row says what is happening and how to stop it.
    if matches!(b.stage, Stage::Streaming { .. }) {
        let y = g.chips[0].bottom + s(GAP) + oy;
        cv.text("Writing...", s(PAD) + ox, y, s(200), s(ACTION_H), f(F::BodySemi), t.accent, one);
        cv.text("Esc stops", 0, y, g.w - s(PAD) + ox, s(ACTION_H), f(F::Caption), t.text3, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
    }
    for (i, (hit, r, label, key)) in g.actions.iter().enumerate() {
        let (x, y, w, h) = at(r);
        let primary = i == 0;
        let hot = b.hover == Some(*hit);
        let (bg, fg, hint) = if primary {
            (if hot { t.accent_press } else { t.accent_fill }, t.on_accent, t.on_accent)
        } else {
            (if hot { t.hover } else { t.surface }, t.text, t.text3)
        };
        cv.round_rect(x as f32, y as f32, w as f32, h as f32, s(8) as f32, bg, 1.0);
        if !primary {
            cv.round_rect_stroke(x as f32, y as f32, w as f32, h as f32, s(8) as f32, 1.0, t.control_border, 0.7);
        }
        let lw = b.tw(label, F::BodySemi);
        let kw = if key.is_empty() { 0 } else { b.tw(key, F::Caption) + s(10) };
        let start = x + (w - lw - kw) / 2;
        cv.text(label, start, y, lw + 4, h, f(F::BodySemi), fg, one);
        if !key.is_empty() {
            cv.text(key, start + lw + s(10), y, kw, h, f(F::Caption), hint, one);
        }
    }
    cv.round_rect_stroke(0.0, 0.0, client_w as f32, client_h as f32, 0.0, 1.0, t.border, 1.0);
    cv.blit_to(hdc, 0, 0);
}

// ---------------------------------------------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------------------------------------------

fn window_dpi(hwnd: HWND) -> u32 {
    let o = dpi_override();
    if o != 0 {
        return o;
    }
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 { 96 } else { d }
}

pub unsafe fn register_class() {
    let cls = to_wide(COMPOSER_WINDOW_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DROPSHADOW,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: GetModuleHandleW(std::ptr::null()),
        hIcon: std::ptr::null_mut(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: cls.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    RegisterClassExW(&wc);
}

pub fn hwnd() -> HWND {
    BAR.with(|b| b.borrow().as_ref().map(|b| b.hwnd).unwrap_or(std::ptr::null_mut()))
}

/// Applies one coordinator message (GUI thread).
pub fn apply(msg: ComposerMsg) {
    match msg {
        ComposerMsg::Open(req) => open(req),
        ComposerMsg::Token(tok) => update(|b| {
            if let Stage::Streaming { text, arrived } = &mut b.stage {
                text.push_str(&tok);
                note_arrivals(arrived, text, Instant::now());
            }
        }),
        ComposerMsg::Done(text) => update(|b| {
            if !matches!(b.stage, Stage::Empty) {
                b.stage = Stage::Result { text };
                // The input now takes a follow-up; Enter on an empty input delivers the result.
                let cue = to_wide(if b.replaces() { "Ask for a change, or press Enter to replace" } else { "Ask for a change, or press Enter to insert" });
                unsafe {
                    SetWindowTextW(b.input, to_wide("").as_ptr());
                    SendMessageW(b.input, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
                    SendMessageW(b.input, EM_SETREADONLY, 0, 0);
                }
            }
        }),
        ComposerMsg::Dictated(text) => update(|b| {
            // A single-line input: the transcript's line breaks become spaces.
            let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let wide = to_wide(&words);
            unsafe {
                SetWindowTextW(b.input, wide.as_ptr());
                // The caret sits after the last word, so the user can keep talking or type on.
                let end = wide.len().saturating_sub(1);
                SendMessageW(b.input, EM_SETSEL, end, end as LPARAM);
                SendMessageW(b.input, EM_SETREADONLY, 0, 0);
            }
        }),
        ComposerMsg::Listening(on) => update(|b| b.listening = on),
        ComposerMsg::Failed { message, settings } => update(|b| b.stage = Stage::Failed { message, settings }),
    }
}

/// Changes the state, then resizes (keeping the edge nearest the pill), repaints and runs the timer if needed.
fn update(f: impl FnOnce(&mut Bar)) {
    let h = BAR.with(|b| {
        let mut bb = b.borrow_mut();
        let bar = bb.as_mut()?;
        f(bar);
        resize(bar);
        Some(bar.hwnd)
    });
    if let Some(h) = h {
        unsafe {
            InvalidateRect(h, std::ptr::null(), 0);
            SetTimer(h, IDT_BAR_ANIM, 16, None);
        }
    }
}

unsafe fn work_area(pt: POINT) -> RECT {
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(mon, &mut mi);
    mi.rcWork
}

fn resize(b: &mut Bar) {
    let g = b.geo();
    let below = b.to.top >= b.from.bottom;
    let new = if below {
        RECT { bottom: b.to.top + g.h, ..b.to }
    } else {
        RECT { top: b.to.bottom - g.h, ..b.to }
    };
    b.to = new;
    if b.closing.is_none() && b.motion().0 >= 1.0 {
        unsafe {
            SetWindowPos(b.hwnd, std::ptr::null_mut(), new.left, new.top, new.right - new.left, new.bottom - new.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        place_children(b, 0, 0);
    }
}

/// Positions the input (and the result editor) at their resting place, offset by the bloom's reveal.
fn place_children(b: &Bar, ox: i32, oy: i32) {
    let g = b.geo();
    unsafe {
        let r = g.input;
        let (x, y) = (r.left + b.s(12) + ox, r.top + (r.bottom - r.top - b.s(22)) / 2 + oy);
        SetWindowPos(b.input, std::ptr::null_mut(), x, y, g.mic.left - b.s(8) - (r.left + b.s(12)), b.s(22), SWP_NOZORDER | SWP_NOACTIVATE);
        if let (false, Some(c)) = (b.result_edit.is_null(), g.card) {
            let p = b.s(CARD_PAD);
            SetWindowPos(b.result_edit, std::ptr::null_mut(), c.left + p + ox, c.top + p + oy, c.right - c.left - 2 * p, c.bottom - c.top - 2 * p, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

fn open(req: ComposerRequest) {
    close_now();
    unsafe {
        refresh_dark_mode();
        let cls = to_wide(COMPOSER_WINDOW_CLASS);
        let title = to_wide("KeyReed Composer");
        let hwnd = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED, cls.as_ptr(), title.as_ptr(), WS_POPUP | WS_CLIPCHILDREN, 0, 0, 10, 10, std::ptr::null_mut(), std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null());
        if hwnd.is_null() {
            return;
        }
        // The pill is where the bloom starts; without one, the bar opens near the top of the screen.
        let pill = globals().map(|g| *g.pill_hwnd.lock() as HWND).filter(|h| !h.is_null() && IsWindowVisible(*h) != 0);
        let mut from: RECT = std::mem::zeroed();
        let mut pt = POINT { x: 0, y: 0 };
        if let Some(p) = pill {
            GetWindowRect(p, &mut from);
            pt = POINT { x: (from.left + from.right) / 2, y: (from.top + from.bottom) / 2 };
        } else {
            GetCursorPos(&mut pt);
        }
        let work = work_area(pt);
        if pill.is_none() {
            let cx = (work.left + work.right) / 2;
            let y = work.top + (work.bottom - work.top) / 5;
            from = RECT { left: cx - 20, top: y - 20, right: cx + 20, bottom: y };
        }
        let dpi = window_dpi(hwnd);
        let input = CreateWindowExW(0, to_wide("EDIT").as_ptr(), std::ptr::null(), WS_CHILD | WS_VISIBLE | WS_TABSTOP | 0x0080 /* ES_AUTOHSCROLL */, 0, 0, 10, 10, hwnd, ID_INPUT as isize as HMENU, GetModuleHandleW(std::ptr::null()), std::ptr::null());
        let mut bar = Bar {
            hwnd,
            input,
            result_edit: std::ptr::null_mut(),
            use_selection: req.selection.is_some(),
            req,
            stage: Stage::Empty,
            chip_focus: None,
            hover: None,
            listening: false,
            opened: Instant::now(),
            closing: None,
            from,
            to: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            dpi,
            fonts: Fonts::new(dpi),
            brush: CreateSolidBrush(theme().surface),
            words: RefCell::new((String::new(), std::rc::Rc::new((Vec::new(), 0)))),
        };
        let g = bar.geo();
        bar.to = place(&from, &work, g.w, g.h, sc(10, dpi));
        SendMessageW(input, WM_SETFONT, bar.fonts.get(F::Body) as WPARAM, 1);
        let cue = to_wide(if bar.req.unavailable { "Add a writing model in Settings to use the Composer" } else { "Ask KeyReed to write or change something" });
        SendMessageW(input, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
        SetWindowSubclass(input, Some(edit_subclass), 1, 0);
        let round: u32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const _, 4);
        let reduced = reduced_motion();
        let start = if reduced { bar.to } else { from };
        SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        SetWindowPos(hwnd, HWND_TOPMOST, start.left, start.top, start.right - start.left, start.bottom - start.top, SWP_NOACTIVATE);
        let (ox, oy) = (bar.to.left - start.left, bar.to.top - start.top);
        place_children(&bar, ox, oy);
        bar.opened = Instant::now();
        BAR.with(|b| *b.borrow_mut() = Some(bar));
        ShowWindow(hwnd, SW_SHOW);
        let _ = crate::typing::force_foreground_window(hwnd as isize);
        watch_clicks();
        SetTimer(hwnd, IDT_BAR_ANIM, 16, None);
    }
}

/// Starts the fold back into the pill; the window is destroyed when it ends.
pub fn close() {
    let h = BAR.with(|b| {
        let mut bb = b.borrow_mut();
        let bar = bb.as_mut()?;
        if bar.closing.is_none() {
            bar.closing = Some(Instant::now());
            // Fold from where the bar is now.
            let mut r: RECT = unsafe { std::mem::zeroed() };
            unsafe { GetWindowRect(bar.hwnd, &mut r) };
            bar.to = r;
        }
        Some(bar.hwnd)
    });
    if let Some(h) = h {
        unsafe { SetTimer(h, IDT_BAR_ANIM, 16, None) };
    }
}

fn close_now() {
    let h = hwnd();
    if !h.is_null() {
        unsafe { DestroyWindow(h) };
    }
}

fn input_text(b: &Bar) -> String {
    read_text(b.input)
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

/// The text the user will get: the editor's text once they have edited it, otherwise the result.
fn delivered_text(b: &Bar) -> Option<String> {
    if !b.result_edit.is_null() {
        return Some(read_text(b.result_edit).replace("\r\n", "\n"));
    }
    match &b.stage {
        Stage::Result { text } => Some(text.clone()),
        _ => None,
    }
}

fn submit(b: &mut Bar, instruction: Instruction, draft: Option<String>) {
    if b.req.unavailable {
        b.stage = Stage::Failed { message: "No writing model is installed and cloud text jobs are off.".into(), settings: true };
        return;
    }
    let follow_up = matches!(b.stage, Stage::Result { .. });
    drop_result_editor(b);
    // The prompt stays visible while the request runs (a preset shows its name).
    if let Some(i) = (0..PRESETS.len()).find(|i| preset(*i) == instruction) {
        unsafe { SetWindowTextW(b.input, to_wide(PRESETS[i]).as_ptr()) };
    }
    b.stage = Stage::Streaming { text: String::new(), arrived: Vec::new() };
    b.chip_focus = None;
    unsafe { SendMessageW(b.input, EM_SETREADONLY, 1, 0) };
    send(if follow_up { UiAction::ComposerFollowUp(instruction) } else { UiAction::ComposerSubmit { instruction, use_selection: b.use_selection, draft } });
}

fn drop_result_editor(b: &mut Bar) {
    if !b.result_edit.is_null() {
        unsafe { DestroyWindow(b.result_edit) };
        b.result_edit = std::ptr::null_mut();
    }
}

/// One action from a key, a click or a chip.
fn act(hit: Hit) {
    enum After {
        Nothing,
        Close,
        Settings,
    }
    let after = BAR.with(|cell| {
        let mut bb = cell.borrow_mut();
        let Some(b) = bb.as_mut() else { return After::Nothing };
        match hit {
            Hit::Chip(i) if b.presets_enabled() => {
                // A preset works on the selection, the result, or the draft typed into the input.
                let draft = (!b.replaces() && b.result_text().is_none()).then(|| input_text(b)).filter(|t| !t.trim().is_empty());
                submit(b, preset(i), draft);
                After::Nothing
            }
            // Nothing to work on yet: the caret goes where the text would be, so the click is never silently dead.
            Hit::Chip(_) => {
                unsafe { SetFocus(b.input) };
                After::Nothing
            }
            Hit::RemoveContext => {
                b.use_selection = false;
                After::Nothing
            }
            Hit::Mic => {
                send(UiAction::ComposerDictate);
                After::Nothing
            }
            Hit::Deliver | Hit::Copy => match delivered_text(b) {
                Some(text) => {
                    let how = if hit == Hit::Copy { DeliverHow::Copy } else if b.replaces() { DeliverHow::Replace } else { DeliverHow::Insert };
                    send(UiAction::ComposerDeliver { how, text });
                    After::Close
                }
                None => After::Nothing,
            },
            Hit::Regenerate if matches!(b.stage, Stage::Result { .. }) => {
                drop_result_editor(b);
                b.stage = Stage::Streaming { text: String::new(), arrived: Vec::new() };
                send(UiAction::ComposerRegenerate);
                After::Nothing
            }
            Hit::Settings => After::Settings,
            Hit::ResultText => {
                open_result_editor(b);
                After::Nothing
            }
            _ => After::Nothing,
        }
    });
    match after {
        After::Close => close(),
        After::Settings => {
            close();
            crate::settings::open(Some(crate::settings::PAGE_MODELS));
        }
        After::Nothing => update(|_| {}),
    }
}

/// The result becomes a native multi-line edit box (selection, input methods, screen readers).
fn open_result_editor(b: &mut Bar) {
    let Stage::Result { text } = &b.stage else { return };
    if !b.result_edit.is_null() {
        return;
    }
    unsafe {
        let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | 0x0004 /* ES_MULTILINE */ | 0x0040 /* ES_AUTOVSCROLL */ | 0x1000 /* ES_WANTRETURN */;
        let body = to_wide(&text.replace('\n', "\r\n"));
        let h = CreateWindowExW(0, to_wide("EDIT").as_ptr(), body.as_ptr(), style, 0, 0, 10, 10, b.hwnd, ID_RESULT as isize as HMENU, GetModuleHandleW(std::ptr::null()), std::ptr::null());
        if h.is_null() {
            return;
        }
        SendMessageW(h, WM_SETFONT, b.fonts.get(F::Body) as WPARAM, 1);
        SetWindowSubclass(h, Some(edit_subclass), 2, 0);
        b.result_edit = h;
        place_children(b, 0, 0);
        SetFocus(h);
        SendMessageW(h, EM_SETSEL, usize::MAX, -1);
    }
}

/// Esc: stop a running request (the prompt stays), otherwise close.
fn escape() {
    let streaming = peek(|b| matches!(b.stage, Stage::Streaming { .. })).unwrap_or(false);
    if streaming {
        send(UiAction::ComposerCancel);
        update(|b| {
            b.stage = Stage::Empty;
            unsafe { SendMessageW(b.input, EM_SETREADONLY, 0, 0) };
        });
    } else {
        dismiss();
    }
}

/// The bar goes away: the coordinator forgets the session, and the window folds back into the pill.
fn dismiss() {
    send(UiAction::ComposerClosed);
    close();
}

/// A click outside the bar (another app, the desktop, the pill) closes it only when nothing would be lost.
/// Returns whether it closed; otherwise the bar stays and Esc still closes it.
pub fn outside_click() -> bool {
    let closes = peek(|b| outside_click_closes(!matches!(b.stage, Stage::Empty), &input_text(b))).unwrap_or(false);
    if closes {
        dismiss();
    }
    closes
}

/// Keys that mean something to the bar, wherever the focus is inside it.
fn key(vk: u16) -> bool {
    let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) < 0 };
    let shift = unsafe { GetKeyState(VK_SHIFT as i32) < 0 };
    let Some((stage_result, stage_empty, chip, typed, editing)) = peek(|b| {
        (matches!(b.stage, Stage::Result { .. }), matches!(b.stage, Stage::Empty | Stage::Failed { .. }), b.chip_focus, input_text(b), !b.result_edit.is_null())
    }) else {
        return false;
    };
    match vk {
        VK_ESCAPE => escape(),
        VK_RETURN if editing && !ctrl => return false, // a new line in the result editor
        VK_RETURN if stage_result && !ctrl && chip.is_none() && !typed.trim().is_empty() => {
            BAR.with(|b| {
                if let Some(b) = b.borrow_mut().as_mut() {
                    submit(b, Instruction::Custom(typed.trim().to_string()), None);
                }
            });
            update(|_| {});
        }
        VK_RETURN if stage_result => match chip {
            Some(i) if !ctrl => act(Hit::Chip(i)),
            _ => act(if ctrl { Hit::Copy } else { Hit::Deliver }),
        },
        VK_RETURN if stage_empty => match chip {
            Some(i) => act(Hit::Chip(i)),
            None if !typed.trim().is_empty() => {
                BAR.with(|b| {
                    if let Some(b) = b.borrow_mut().as_mut() {
                        submit(b, Instruction::Custom(typed.trim().to_string()), None);
                    }
                });
                update(|_| {});
            }
            None => {}
        },
        0x52 if ctrl && stage_result => act(Hit::Regenerate), // Ctrl+R
        VK_TAB if !editing => update(|b| {
            if !b.presets_enabled() {
                b.chip_focus = None;
                return;
            }
            let n = PRESETS.len();
            b.chip_focus = match (b.chip_focus, shift) {
                (None, false) => Some(0),
                (None, true) => Some(n - 1),
                (Some(i), false) if i + 1 < n => Some(i + 1),
                (Some(i), true) if i > 0 => Some(i - 1),
                _ => None,
            };
            unsafe { SetFocus(if b.chip_focus.is_some() { b.hwnd } else { b.input }) };
        }),
        _ => return false,
    }
    true
}

unsafe extern "system" fn edit_subclass(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        WM_GETDLGCODE => return DefSubclassProc(h, msg, wparam, lparam) | (DLGC_WANTALLKEYS | DLGC_WANTTAB) as LRESULT,
        WM_KEYDOWN if key(wparam as u16) => return 0,
        // Enter, Esc and Tab would otherwise beep in a single-line edit.
        WM_CHAR if matches!(wparam as u16, 0x0D | 0x1B | 0x09) && GetDlgCtrlID(h) == ID_INPUT => return 0,
        WM_SETFOCUS | WM_KILLFOCUS => {
            InvalidateRect(GetParent(h), std::ptr::null(), 0);
        }
        _ => {}
    }
    DefSubclassProc(h, msg, wparam, lparam)
}

/// The window was moved: the resting rectangle becomes the window's, and the children and pixels follow it.
fn follow_move(hwnd: HWND) {
    // Only at rest: the bloom moves the window itself, and its own rectangle must survive that.
    if !peek(|b| b.closing.is_none() && b.motion().0 >= 1.0).unwrap_or(false) {
        return;
    }
    BAR.with(|b| {
        if let Ok(mut bb) = b.try_borrow_mut() {
            if let Some(bar) = bb.as_mut() {
                let mut r: RECT = unsafe { std::mem::zeroed() };
                unsafe { GetWindowRect(hwnd, &mut r) };
                // `from` stays the pill, so closing still folds back into it.
                bar.to = r;
                place_children(bar, 0, 0);
            }
        }
    });
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

fn client_xy(lparam: LPARAM) -> (i32, i32) {
    ((lparam & 0xFFFF) as u16 as i16 as i32, ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32)
}

/// The reveal offset: where the resting layout's origin sits inside the current (blooming) window.
fn reveal_offset(b: &Bar) -> (i32, i32) {
    let mut r: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetWindowRect(b.hwnd, &mut r) };
    (b.to.left - r.left, b.to.top - r.top)
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut cr: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut cr);
            peek(|bar| {
                let (ox, oy) = reveal_offset(bar);
                paint(bar, hdc, cr.right, cr.bottom, ox, oy);
            });
            EndPaint(hwnd, &ps);
            0
        }
        // A read-only edit (the prompt while a request runs) asks for the static colours.
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let t = theme();
            SetTextColor(wparam as HDC, t.text);
            SetBkColor(wparam as HDC, t.surface);
            peek(|b| b.brush as LRESULT).unwrap_or(0)
        }
        WM_NCHITTEST => {
            // Row 1 outside the chip and the route label drags the bar.
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let (x, y) = (((lparam & 0xFFFF) as u16 as i16) as i32 - r.left, (((lparam >> 16) & 0xFFFF) as u16 as i16) as i32 - r.top);
            let drag = peek(|b| y < b.s(INPUT_Y - 4) && b.hit(x, y).is_none()).unwrap_or(false);
            if drag { HTCAPTION as LRESULT } else { HTCLIENT as LRESULT }
        }
        // Moved (a drag, or Windows moving it): the resting rectangle follows, so the reveal offset stays zero and
        // the next resize keeps the new spot instead of snapping the bar back to where it opened.
        WM_MOVE | WM_EXITSIZEMOVE => {
            follow_move(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_xy(lparam);
            let changed = BAR.with(|b| {
                let Ok(mut bb) = b.try_borrow_mut() else { return false };
                let Some(bar) = bb.as_mut() else { return false };
                let h = bar.hit(x, y);
                let changed = h != bar.hover;
                bar.hover = h;
                changed
            });
            if changed {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_SETCURSOR => {
            let hot = peek(|b| b.hover.is_some()).unwrap_or(false);
            if hot && (lparam & 0xFFFF) as u32 == HTCLIENT {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONUP => {
            let (x, y) = client_xy(lparam);
            let hit = peek(|b| b.hit(x, y)).flatten();
            if let Some(h) = hit {
                act(h);
            } else {
                if let Some(i) = peek(|b| b.input) {
                    SetFocus(i);
                }
            }
            0
        }
        WM_KEYDOWN => {
            // The bar itself has the focus while a chip is focused.
            if !key(wparam as u16) && wparam as u16 == 0x20 {
                if let Some(i) = peek(|b| b.chip_focus).flatten() {
                    act(Hit::Chip(i));
                }
            }
            0
        }
        WM_TIMER if wparam == IDT_BAR_ANIM => {
            let state = peek(|bar| {
                let (shape, alpha, _) = bar.motion();
                (shape, alpha, bar.animating(), bar.closing.is_some(), bar.from, bar.to)
            });
            let Some((shape, alpha, animating, closing, from, to)) = state else { return 0 };
            let r = lerp_rect(&from, &to, shape);
            SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE);
            SetLayeredWindowAttributes(hwnd, 0, (255.0 * alpha) as u8, LWA_ALPHA);
            peek(|bar| place_children(bar, to.left - r.left, to.top - r.top));
            InvalidateRect(hwnd, std::ptr::null(), 0);
            if closing && alpha <= 0.0 {
                KillTimer(hwnd, IDT_BAR_ANIM);
                DestroyWindow(hwnd);
            } else if !animating {
                KillTimer(hwnd, IDT_BAR_ANIM);
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                // The caret appears when the bloom has ended.
                let input = peek(|b| (b.result_edit.is_null() && b.chip_focus.is_none()).then_some(b.input)).flatten();
                if let Some(i) = input {
                    if GetForegroundWindow() == hwnd && GetFocus() != i {
                        SetFocus(i);
                    }
                }
            }
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xFFFF) != 0 {
                if let Some(i) = peek(|b| b.input) {
                    SetFocus(i);
                }
            }
            0
        }
        WM_CLOSE => {
            send(UiAction::ComposerClosed);
            close();
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, IDT_BAR_ANIM);
            unwatch_clicks();
            BAR.with(|b| {
                if let Some(bar) = b.borrow_mut().take() {
                    DeleteObject(bar.brush as _);
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Gallery hook: types `text` into the input, or focuses chip `chip`, without the keyboard.
pub fn set_state_for_test(text: Option<&str>, chip: Option<usize>, edit_result: bool) {
    BAR.with(|b| {
        if let Some(bar) = b.borrow_mut().as_mut() {
            if let Some(t) = text {
                let w = to_wide(t);
                unsafe { SetWindowTextW(bar.input, w.as_ptr()) };
            }
            bar.chip_focus = chip;
            if edit_result {
                open_result_editor(bar);
            }
        }
    });
    update(|_| {});
}

/// Gallery hook: a screen point a simulated click aims at: preset chip `chip`, or the input when `None`.
pub fn click_point(chip: Option<usize>) -> Option<(i32, i32)> {
    let (r, wr) = BAR.with(|b| {
        let b = b.borrow();
        let bar = b.as_ref()?;
        let g = bar.geo();
        let r = match chip {
            Some(i) => *g.chips.get(i)?,
            None => g.input,
        };
        let mut wr: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetWindowRect(bar.hwnd, &mut wr) };
        Some((r, wr))
    })?;
    Some((wr.left + (r.left + r.right) / 2, wr.top + (r.top + r.bottom) / 2))
}

/// Gallery hook: submits preset `i`, or the typed instruction for any other `i`, as Enter or a click would.
pub fn submit_for_test(i: usize) {
    BAR.with(|b| {
        if let Some(bar) = b.borrow_mut().as_mut() {
            let instruction = if i < PRESETS.len() { preset(i) } else { Instruction::Custom(input_text(bar)) };
            submit(bar, instruction, None);
        }
    });
    update(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_label_counts_characters_not_bytes() {
        assert_eq!(context_label("héllo", "Outlook"), "Using selection, 5 characters from Outlook");
        assert_eq!(context_label("a", ""), "Using selection, 1 character");
    }

    #[test]
    fn app_label_is_a_short_name() {
        assert_eq!(app_label("OUTLOOK.EXE"), "Outlook");
        assert_eq!(app_label("C:\\Program Files\\notepad.exe"), "Notepad");
        assert_eq!(app_label("winword"), "Winword");
        assert_eq!(app_label(""), "");
        assert_eq!(app_label("é.exe"), "É");
    }

    #[test]
    fn arrivals_follow_whole_words() {
        let now = Instant::now();
        let mut a = Vec::new();
        note_arrivals(&mut a, "Hello", now);
        note_arrivals(&mut a, "Hello wor", now);
        assert_eq!(a.len(), 2);
        note_arrivals(&mut a, "Hello world", now);
        assert_eq!(a.len(), 2, "a word that grows is still one word");
        note_arrivals(&mut a, "", now);
        assert_eq!(a.len(), 2, "never shrinks");
    }

    #[test]
    fn colours_and_rects_interpolate() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 1.0), 0xFFFFFF);
        assert_eq!(mix(0x0000FF, 0x00FF00, 0.5), 0x008080);
        let a = RECT { left: 0, top: 0, right: 10, bottom: 10 };
        let b = RECT { left: 100, top: 50, right: 300, bottom: 150 };
        let m = lerp_rect(&a, &b, 0.5);
        assert_eq!((m.left, m.top, m.right, m.bottom), (50, 25, 155, 80));
        assert_eq!(lerp_rect(&a, &b, 1.0).right, 300);
    }

    #[test]
    fn a_click_outside_closes_only_when_nothing_would_be_lost() {
        assert!(outside_click_closes(false, ""), "an empty bar just folds away");
        assert!(outside_click_closes(false, "   "), "whitespace is not text");
        assert!(!outside_click_closes(false, "a draft"), "typed text is never thrown away");
        assert!(!outside_click_closes(true, ""), "a result, a failure or a running request stays until Esc");
        assert!(!outside_click_closes(true, "a draft"));
    }

    #[test]
    fn presets_need_text_and_stop_while_a_request_runs() {
        assert!(presets_ready(true, false, "", false), "the kept selection");
        assert!(presets_ready(false, true, "", false), "the latest result");
        assert!(presets_ready(false, false, "a draft", false), "the draft typed into the input");
        assert!(!presets_ready(false, false, "  ", false), "an empty bar has nothing to work on");
        assert!(!presets_ready(true, true, "a draft", true), "dimmed while streaming, with text or without");
    }

    #[test]
    fn bar_opens_on_the_side_of_the_pill_with_room_and_stays_on_screen() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let near_bottom = RECT { left: 940, top: 980, right: 980, bottom: 1020 };
        let r = place(&near_bottom, &work, 520, 144, 10);
        assert_eq!((r.bottom, r.right - r.left), (970, 520), "above a pill at the bottom");
        let near_top = RECT { left: 10, top: 20, right: 50, bottom: 60 };
        let r = place(&near_top, &work, 520, 144, 10);
        assert_eq!((r.top, r.left), (70, 0), "below a pill at the top, clamped to the left edge");
        let tiny = RECT { left: 0, top: 0, right: 600, bottom: 150 };
        let r = place(&RECT { left: 280, top: 50, right: 320, bottom: 90 }, &tiny, 520, 144, 10);
        assert!(r.top >= tiny.top && r.bottom <= tiny.bottom + 1, "no room on either side: inside the work area");
    }
}
