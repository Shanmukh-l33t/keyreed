//! The Review panel (plan F1): the first state of the Composer window. It shows the corrected text with every
//! changed word marked, lets the user accept or reject each change, and offers Replace, Copy and Dismiss.
//! Enter replaces, Esc dismisses; Tab or the arrow keys move between changes, Space accepts or rejects the one in
//! focus, O shows the original words, Ctrl+C copies.
//!
//! Drawing uses the Settings theme and the software canvas; the window is a borderless popup so it can take the
//! keyboard. The layout and the edit handling are pure functions (tested below); only the paint and the message
//! handler touch Win32. Text lives in this window's memory only and is never logged.

#![cfg(windows)]

mod bar;
pub use bar::{app_label, apply, ComposerMsg, ComposerRequest, COMPOSER_WINDOW_CLASS};
/// The Composer bar (plan F2): open, stream, deliver.
pub mod composer_bar {
    pub use super::bar::{click_point, close, hwnd, set_state_for_test, submit_for_test};
}

use crate::canvas::{text_width, Canvas, DT_LEFT, DT_SINGLELINE};
use crate::settings::{refresh_dark_mode, theme, Fonts, Theme, F};
use crate::ui::win_gui::{dpi_override, reduced_motion, send, to_wide};
use crate::ui::{ReviewChoice, UiAction};
use std::cell::RefCell;
use std::time::Instant;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const REVIEW_WINDOW_CLASS: &str = "KeyreedReviewClass";

// Logical (96-DPI) geometry.
const WIN_W: i32 = 520;
const MAX_H: i32 = 360;
const PAD: i32 = 18;
const HEADER_H: i32 = 46;
const FOOTER_H: i32 = 62;
const LINE_H: i32 = 24;
const CARD_PAD: i32 = 12;
const MARK_PAD: i32 = 2;
const BUTTON_H: i32 = 34;
const NOTE_H: i32 = 32;

const IDT_REVIEW_ANIM: usize = 9101;
const FADE_MS: f32 = 120.0;
const UNDERLINE_MS: f32 = 180.0;
const RELAX_MS: f32 = 120.0;

// ---------------------------------------------------------------------------------------------------------------
// Edits and layout: pure
// ---------------------------------------------------------------------------------------------------------------

/// One proposed change: a byte range of the original text and what replaces it (empty = delete; empty range =
/// insert). `label` names the layer ("Spelling", "Grammar", "Rule").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub label: &'static str,
}

/// What the panel shows. `note` is one plain sentence about why the result needs a look (for example, formatting in
/// the selection would be lost).
#[derive(Clone, Debug, Default)]
pub struct ReviewRequest {
    pub original: String,
    pub edits: Vec<ReviewEdit>,
    pub note: Option<String>,
}

/// Keeps the edits that can be applied: in range, on character boundaries, in order, not overlapping, and not
/// no-ops. Anything else is dropped rather than risking a wrong replacement.
pub fn normalize_edits(original: &str, edits: &[ReviewEdit]) -> Vec<ReviewEdit> {
    let mut valid: Vec<ReviewEdit> = edits
        .iter()
        .filter(|e| {
            e.start <= e.end
                && e.end <= original.len()
                && original.is_char_boundary(e.start)
                && original.is_char_boundary(e.end)
                && original[e.start..e.end] != e.replacement
        })
        .cloned()
        .map(|e| trim_shared_space(original, e))
        .collect();
    valid.sort_by_key(|e| (e.start, e.end));
    let mut out: Vec<ReviewEdit> = Vec::with_capacity(valid.len());
    let mut last = 0;
    for e in valid {
        if e.start >= last {
            last = e.end;
            out.push(e);
        }
    }
    out
}

/// Moves whitespace that the original span and the replacement share at either end out of the edit, so a change
/// is marked on its words only (" the the" to " the" marks "the the" to "the", not the space before it).
fn trim_shared_space(original: &str, mut e: ReviewEdit) -> ReviewEdit {
    let same_space = |(a, b): &(char, char)| a == b && a.is_whitespace();
    let span = &original[e.start..e.end];
    let lead: usize = span.chars().zip(e.replacement.chars()).take_while(same_space).map(|(a, _)| a.len_utf8()).sum();
    let (span, rep) = (&span[lead..], &e.replacement[lead..]);
    let trail: usize = span.chars().rev().zip(rep.chars().rev()).take_while(same_space).map(|(a, _)| a.len_utf8()).sum();
    let trail = trail.min(span.len()).min(rep.len());
    e.replacement = rep[..rep.len() - trail].to_string();
    e.start += lead;
    e.end -= trail;
    e
}

/// The original with every accepted edit applied. `edits` must come from `normalize_edits`.
pub fn final_text(original: &str, edits: &[ReviewEdit], accepted: &[bool]) -> String {
    let mut out = String::with_capacity(original.len());
    let mut pos = 0;
    for (e, ok) in edits.iter().zip(accepted) {
        out.push_str(&original[pos..e.start]);
        out.push_str(if *ok { &e.replacement } else { &original[e.start..e.end] });
        pos = e.end;
    }
    out.push_str(&original[pos..]);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Space,
    Break,
    Change(usize),
}

fn push_text(toks: &mut Vec<Tok>, text: &str) {
    let mut word = String::new();
    let flush = |toks: &mut Vec<Tok>, word: &mut String| {
        if !word.is_empty() {
            toks.push(Tok::Word(std::mem::take(word)));
        }
    };
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' => {
                flush(toks, &mut word);
                toks.push(Tok::Break);
            }
            c if c.is_whitespace() => {
                flush(toks, &mut word);
                if !matches!(toks.last(), Some(Tok::Space)) {
                    toks.push(Tok::Space);
                }
            }
            c => word.push(c),
        }
    }
    flush(toks, &mut word);
}

fn tokens(original: &str, edits: &[ReviewEdit]) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut pos = 0;
    for (i, e) in edits.iter().enumerate() {
        push_text(&mut toks, &original[pos..e.start]);
        toks.push(Tok::Change(i));
        pos = e.end;
    }
    push_text(&mut toks, &original[pos..]);
    toks
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    tok: usize,
    x: i32,
    y: i32,
    w: i32,
}

/// Word-wrap. Words and changes that touch (no space between) stay on one line. Returns the placed tokens (not
/// spaces) and the total height. A single run wider than `avail` overflows rather than breaking inside a word.
fn layout(toks: &[Tok], widths: &[i32], space_w: i32, avail: i32, line_h: i32) -> (Vec<Placed>, i32) {
    let mut placed = Vec::new();
    let (mut x, mut y, mut i) = (0, 0, 0);
    while i < toks.len() {
        match toks[i] {
            Tok::Space => {
                if x > 0 {
                    x += space_w;
                }
                i += 1;
            }
            Tok::Break => {
                x = 0;
                y += line_h;
                i += 1;
            }
            _ => {
                let mut j = i;
                while j < toks.len() && !matches!(toks[j], Tok::Space | Tok::Break) {
                    j += 1;
                }
                let run: i32 = widths[i..j].iter().sum();
                if x > 0 && x + run > avail {
                    x = 0;
                    y += line_h;
                }
                for k in i..j {
                    placed.push(Placed { tok: k, x, y, w: widths[k] });
                    x += widths[k];
                }
                i = j;
            }
        }
    }
    (placed, y + line_h)
}

// ---------------------------------------------------------------------------------------------------------------
// Panel state
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Change(usize),
    Replace,
    Copy,
    Dismiss,
    ShowOriginal,
    GotIt,
}

struct Panel {
    hwnd: HWND,
    original: String,
    edits: Vec<ReviewEdit>,
    note: Option<String>,
    accepted: Vec<bool>,
    first_use: bool,
    show_original: bool,
    focus: Option<usize>,
    hover: Option<Hit>,
    opened: Instant,
    dpi: u32,
    fonts: Fonts,
    /// Physical pixels scrolled.
    scroll: i32,
    layout: Option<Cached>,
}

struct Cached {
    toks: Vec<Tok>,
    placed: Vec<Placed>,
    content_h: i32,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

fn sc(v: i32, dpi: u32) -> i32 {
    (v as f32 * dpi as f32 / 96.0).round() as i32
}

fn rc(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn stagger_ms(n: usize) -> f32 {
    (160.0 / n.max(1) as f32).min(40.0)
}

/// Window-relative rectangles, in physical pixels.
struct Geo {
    w: i32,
    h: i32,
    card: RECT,
    buttons: Vec<(Hit, RECT, &'static str, &'static str)>,
}

impl Panel {
    fn applied(&self) -> usize {
        self.accepted.iter().filter(|a| **a).count()
    }

    fn word_w(&self, s: &str, f: F) -> i32 {
        text_width(s, self.fonts.get(f))
    }

    /// Width of a change token: the text it currently shows plus the mark's padding.
    fn change_w(&self, i: usize) -> i32 {
        let e = &self.edits[i];
        let orig = &self.original[e.start..e.end];
        let pad = 2 * sc(MARK_PAD, self.dpi);
        let gap = sc(4, self.dpi);
        let shown = if !self.accepted[i] {
            self.word_w(orig, F::Body)
        } else if self.show_original && !orig.is_empty() && !e.replacement.is_empty() {
            self.word_w(orig, F::Body) + gap + self.word_w(&e.replacement, F::Body)
        } else if e.replacement.is_empty() {
            self.word_w(orig, F::Body)
        } else {
            self.word_w(&e.replacement, F::Body)
        };
        shown + pad
    }

    fn text_area_w(&self) -> i32 {
        sc(WIN_W - 2 * PAD - 2 * CARD_PAD, self.dpi)
    }

    fn ensure_layout(&mut self) {
        if self.layout.is_some() {
            return;
        }
        let toks = tokens(&self.original, &self.edits);
        let widths: Vec<i32> = toks
            .iter()
            .map(|t| match t {
                Tok::Word(w) => self.word_w(w, F::Body),
                Tok::Change(i) => self.change_w(*i),
                _ => 0,
            })
            .collect();
        let space_w = self.word_w(" ", F::Body).max(sc(4, self.dpi));
        let (placed, content_h) = layout(&toks, &widths, space_w, self.text_area_w(), sc(LINE_H, self.dpi));
        self.layout = Some(Cached { toks, placed, content_h });
    }

    /// Height of the whole window in logical pixels for the current content.
    fn window_h(&mut self) -> i32 {
        if self.first_use {
            return first_use_h(self);
        }
        self.ensure_layout();
        let content = self.layout.as_ref().map_or(0, |l| l.content_h) as f32 * 96.0 / self.dpi as f32;
        let note = if self.note.is_some() { NOTE_H + 8 } else { 0 };
        let chrome = HEADER_H + note + FOOTER_H + 2 * CARD_PAD;
        ((content.ceil() as i32) + chrome).clamp(HEADER_H + FOOTER_H + 2 * CARD_PAD + 3 * LINE_H, MAX_H)
    }

    fn geo(&mut self) -> Geo {
        let dpi = self.dpi;
        let (w, h) = (sc(WIN_W, dpi), sc(self.window_h(), dpi));
        let note = if self.note.is_some() { sc(NOTE_H + 8, dpi) } else { 0 };
        let top = sc(HEADER_H, dpi) + note;
        let bottom = h - sc(FOOTER_H, dpi);
        let card = rc(sc(PAD, dpi), top, w - 2 * sc(PAD, dpi), (bottom - top).max(0));
        let (bh, gap, right) = (sc(BUTTON_H, dpi), sc(8, dpi), w - sc(PAD, dpi));
        let by = h - sc(FOOTER_H, dpi) + (sc(FOOTER_H, dpi) - bh) / 2;
        let mut buttons = Vec::new();
        let mut x = right;
        let specs: &[(Hit, &'static str, &'static str, i32)] = if self.first_use {
            &[(Hit::GotIt, "Got it", "Enter", 112)]
        } else {
            &[(Hit::Replace, "Replace", "Enter", 118), (Hit::Copy, "Copy", "Ctrl+C", 104), (Hit::Dismiss, "Dismiss", "Esc", 104)]
        };
        for (hit, label, key, base) in specs {
            let bw = (sc(*base, dpi)).max(self.word_w(label, F::BodySemi) + self.word_w(key, F::Caption) + sc(40, dpi));
            x -= bw;
            buttons.push((*hit, rc(x, by, bw, bh), *label, *key));
            x -= gap;
        }
        if !self.first_use && !self.edits.is_empty() {
            let label = "Show original";
            let bw = self.word_w(label, F::CaptionSemi) + sc(28, dpi);
            buttons.push((Hit::ShowOriginal, rc(sc(PAD, dpi), by + (bh - sc(28, dpi)) / 2, bw, sc(28, dpi)), label, ""));
        }
        Geo { w, h, card, buttons }
    }

    fn view_h(&mut self) -> i32 {
        let g = self.geo();
        g.card.bottom - g.card.top - 2 * sc(CARD_PAD, self.dpi)
    }

    fn max_scroll(&mut self) -> i32 {
        self.ensure_layout();
        let content = self.layout.as_ref().map_or(0, |l| l.content_h);
        (content - self.view_h()).max(0)
    }

    fn hit(&mut self, x: i32, y: i32) -> Option<Hit> {
        let geo = self.geo();
        for (hit, r, _, _) in &geo.buttons {
            if inside(r, x, y) {
                return Some(*hit);
            }
        }
        if self.first_use || !inside(&geo.card, x, y) {
            return None;
        }
        let (ox, oy) = (geo.card.left + sc(CARD_PAD, self.dpi), geo.card.top + sc(CARD_PAD, self.dpi) - self.scroll);
        let line_h = sc(LINE_H, self.dpi);
        let l = self.layout.as_ref()?;
        l.placed.iter().find_map(|p| match l.toks[p.tok] {
            Tok::Change(i) if inside(&rc(ox + p.x, oy + p.y, p.w, line_h), x, y) => Some(Hit::Change(i)),
            _ => None,
        })
    }

    fn animating(&self) -> bool {
        if reduced_motion() {
            return false;
        }
        let ms = self.opened.elapsed().as_secs_f32() * 1000.0;
        ms < FADE_MS + 60.0 + stagger_ms(self.edits.len()) * self.edits.len() as f32 + UNDERLINE_MS + RELAX_MS
    }

    /// Underline progress 0..1 and the alpha it relaxes to for change `i`.
    fn underline(&self, i: usize) -> (f32, f32) {
        if reduced_motion() {
            return (1.0, 0.5);
        }
        let ms = self.opened.elapsed().as_secs_f32() * 1000.0 - 60.0 - i as f32 * stagger_ms(self.edits.len());
        let p = ease_out(ms / UNDERLINE_MS);
        let relax = ease_out((ms - UNDERLINE_MS) / RELAX_MS);
        (p, 1.0 - 0.5 * relax)
    }

    fn finish_text(&self) -> String {
        final_text(&self.original, &self.edits, &self.accepted)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------------------------------------

fn button_fill(t: &Theme, hit: Hit, hot: bool) -> (u32, u32, u32) {
    // (background, text, hint text)
    match hit {
        Hit::Replace | Hit::GotIt => (if hot { t.accent_press } else { t.accent_fill }, t.on_accent, t.on_accent),
        _ => (if hot { t.hover } else { t.surface }, t.text, t.text3),
    }
}

unsafe fn paint(p: &mut Panel, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
    let _pf = crate::perf::frame("review");
    let t = theme();
    let geo = p.geo();
    p.ensure_layout();
    let (max_scroll, view_h) = (p.max_scroll(), p.view_h());
    let p = &*p;
    let dpi = p.dpi;
    let Some(mut cv) = Canvas::new(geo.w, geo.h) else { return };
    cv.fill(t.bg);
    let font = |f: F| p.fonts.get(f);
    let single = DT_LEFT | DT_SINGLELINE;

    if p.first_use {
        paint_first_use(p, &mut cv, &t, &geo);
    } else {
        let cardr = geo.card;
        let r = sc(10, dpi) as f32;
        cv.round_rect(cardr.left as f32, cardr.top as f32, (cardr.right - cardr.left) as f32, (cardr.bottom - cardr.top) as f32, r, t.surface, 1.0);
        let (ox, oy) = (cardr.left + sc(CARD_PAD, dpi), cardr.top + sc(CARD_PAD, dpi) - p.scroll);
        let line_h = sc(LINE_H, dpi);
        let (view_top, view_bot) = (cardr.top + sc(4, dpi), cardr.bottom - sc(4, dpi));
        let l = p.layout.as_ref().unwrap();
        for pl in &l.placed {
            let y = oy + pl.y;
            if y + line_h < view_top || y > view_bot {
                continue;
            }
            match &l.toks[pl.tok] {
                Tok::Word(w) => {
                    cv.text(w, ox + pl.x, y, pl.w + 4, line_h, font(F::Body), t.text, single | crate::canvas::DT_VCENTER);
                }
                Tok::Change(i) => {
                    let e = &p.edits[*i];
                    let orig = &p.original[e.start..e.end];
                    let (mx, my, mw, mh) = ((ox + pl.x) as f32, (y + sc(1, dpi)) as f32, pl.w as f32, (line_h - sc(2, dpi)) as f32);
                    let pad = sc(MARK_PAD, dpi);
                    let accepted = p.accepted[*i];
                    let delete = e.replacement.is_empty();
                    let vc = single | crate::canvas::DT_VCENTER;
                    if accepted {
                        let (bg, fg, line) = if delete { (t.err_bg, t.err_fg, t.err_fg) } else { (t.accent_soft, t.text, t.accent) };
                        cv.round_rect(mx, my, mw, mh, sc(5, dpi) as f32, bg, 1.0);
                        let mut tx = ox + pl.x + pad;
                        if p.show_original && !orig.is_empty() && !delete {
                            let ow = p.word_w(orig, F::Body);
                            cv.text(orig, tx, y, ow + 4, line_h, font(F::Body), t.text3, vc);
                            cv.rect(tx, y + line_h / 2, ow, sc(1, dpi).max(1), t.text3);
                            tx += ow + sc(4, dpi);
                        }
                        if delete {
                            cv.text(orig, tx, y, pl.w, line_h, font(F::Body), fg, vc);
                            cv.rect(tx, y + line_h / 2, pl.w - 2 * pad, sc(1, dpi).max(1), fg);
                        } else {
                            cv.text(&e.replacement, tx, y, pl.w, line_h, font(F::Body), fg, vc);
                        }
                        // The underline draws left to right, then relaxes to a quiet tint.
                        let (prog, alpha) = p.underline(*i);
                        let uw = (pl.w as f32 * prog) as i32;
                        if uw > 0 {
                            let uy = y + line_h - sc(5, dpi);
                            cv.round_rect((ox + pl.x) as f32, uy as f32, uw as f32, sc(2, dpi).max(2) as f32, 1.0, line, alpha);
                        }
                    } else {
                        // Rejected: the original word, marked only by a thin dotted-looking rule.
                        cv.text(orig, ox + pl.x + pad, y, pl.w, line_h, font(F::Body), t.text2, vc);
                        cv.rect(ox + pl.x + pad, y + line_h - sc(5, dpi), pl.w - 2 * pad, sc(1, dpi).max(1), t.text3);
                    }
                    if p.focus == Some(*i) {
                        cv.round_rect_stroke(mx - 1.0, my - 1.0, mw + 2.0, mh + 2.0, sc(6, dpi) as f32, sc(2, dpi).max(2) as f32, t.focus, 1.0);
                    }
                }
                _ => {}
            }
        }
        // Whatever the scroll pushed above or below the card is painted over with the window background.
        cv.rect(0, 0, geo.w, cardr.top, t.bg);
        cv.rect(0, cardr.bottom, geo.w, geo.h - cardr.bottom, t.bg);
        cv.round_rect_stroke(cardr.left as f32, cardr.top as f32, (cardr.right - cardr.left) as f32, (cardr.bottom - cardr.top) as f32, r, 1.0, t.border, 1.0);
        let max = max_scroll;
        if max > 0 {
            let track_h = (cardr.bottom - cardr.top - 2 * sc(8, dpi)) as f32;
            let content = p.layout.as_ref().map_or(1, |l| l.content_h) as f32;
            let thumb_h = (track_h * view_h as f32 / content).max(sc(24, dpi) as f32);
            let thumb_y = cardr.top as f32 + sc(8, dpi) as f32 + (track_h - thumb_h) * p.scroll as f32 / max as f32;
            cv.round_rect((cardr.right - sc(7, dpi)) as f32, thumb_y, sc(3, dpi) as f32, thumb_h, 1.5, t.control_border, 0.8);
        }

        // Header: title, count, and the optional note.
        let title = "Quick Fix";
        cv.text(title, sc(PAD, dpi), sc(13, dpi), sc(160, dpi), sc(24, dpi), font(F::BodySemi), t.text, single);
        let total = p.edits.len();
        let count = if p.applied() == total {
            format!("{total} change{}", if total == 1 { "" } else { "s" })
        } else {
            format!("{} of {total} applied", p.applied())
        };
        let (cw, chh) = (p.word_w(&count, F::CaptionSemi) + sc(20, dpi), sc(22, dpi));
        let cx = sc(PAD, dpi) + p.word_w(title, F::BodySemi) + sc(10, dpi);
        cv.round_rect(cx as f32, sc(14, dpi) as f32, cw as f32, chh as f32, chh as f32 / 2.0, t.accent_soft, 1.0);
        cv.text(&count, cx + sc(10, dpi), sc(14, dpi), cw, chh, font(F::CaptionSemi), t.accent, single | crate::canvas::DT_VCENTER);
        if let Some(note) = &p.note {
            let (ny, nh) = (sc(HEADER_H - 4, dpi), sc(NOTE_H, dpi));
            cv.round_rect(sc(PAD, dpi) as f32, ny as f32, (geo.w - 2 * sc(PAD, dpi)) as f32, nh as f32, sc(8, dpi) as f32, t.warn_bg, 1.0);
            cv.text(note, sc(PAD + 12, dpi), ny, geo.w - sc(2 * PAD + 24, dpi), nh, font(F::Caption), t.warn_fg, single | crate::canvas::DT_VCENTER | crate::canvas::DT_END_ELLIPSIS);
        }
    }

    // Footer buttons.
    for (hit, r, label, key) in &geo.buttons {
        let hot = p.hover == Some(*hit);
        let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        if *hit == Hit::ShowOriginal {
            let on = p.show_original;
            let (bg, fg) = if on { (t.accent_soft, t.accent) } else if hot { (t.hover, t.text2) } else { (t.bg, t.text2) };
            cv.round_rect(r.left as f32, r.top as f32, w, h, h / 2.0, bg, 1.0);
            if !on {
                cv.round_rect_stroke(r.left as f32, r.top as f32, w, h, h / 2.0, 1.0, t.control_border, 0.6);
            }
            cv.text(label, r.left, r.top, r.right - r.left, r.bottom - r.top, font(F::CaptionSemi), fg, crate::canvas::DT_CENTER | crate::canvas::DT_VCENTER | DT_SINGLELINE);
            continue;
        }
        let (bg, fg, hint) = button_fill(&t, *hit, hot);
        cv.round_rect(r.left as f32, r.top as f32, w, h, sc(8, dpi) as f32, bg, 1.0);
        if !matches!(hit, Hit::Replace | Hit::GotIt) {
            cv.round_rect_stroke(r.left as f32, r.top as f32, w, h, sc(8, dpi) as f32, 1.0, t.control_border, 0.7);
        }
        let lw = p.word_w(label, F::BodySemi);
        let kw = p.word_w(key, F::Caption);
        let start = r.left + ((r.right - r.left) - (lw + sc(10, dpi) + kw)) / 2;
        let vc = single | crate::canvas::DT_VCENTER;
        cv.text(label, start, r.top, lw + 4, r.bottom - r.top, font(F::BodySemi), fg, vc);
        cv.text(key, start + lw + sc(10, dpi), r.top, kw + 4, r.bottom - r.top, font(F::Caption), hint, vc);
    }
    cv.round_rect_stroke(0.0, 0.0, geo.w as f32, geo.h as f32, 0.0, 1.0, t.border, 1.0);
    cv.blit_to(hdc, 0, 0);
}

/// The one-time explanation (plan F1): what Quick Fix does, the clipboard, privacy, undo and the deeper pass.
const FIRST_USE: [(&str, F); 4] = [
    ("Fixes typos and common slips in the text you select. It does not rewrite.", F::Body),
    ("To read your selection KeyReed copies it through the Windows clipboard, so it can show up in clipboard history. Your clipboard is put back afterwards.", F::Caption),
    ("All of this happens on this PC.", F::Caption),
    ("Undo from the pill or with Ctrl+Z in your app. For a deeper pass, press the Quick Fix key again or choose Deeper.", F::Caption),
];

/// Logical height of the first-use screen.
fn first_use_h(p: &Panel) -> i32 {
    let w = sc(WIN_W - 2 * PAD, p.dpi);
    let text: i32 = FIRST_USE.iter().map(|(t, f)| crate::canvas::text_height(t, p.fonts.get(*f), w) + sc(14, p.dpi)).sum();
    let px = sc(62, p.dpi) + text + sc(FOOTER_H, p.dpi);
    (px as f32 * 96.0 / p.dpi as f32).ceil() as i32
}

unsafe fn paint_first_use(p: &Panel, cv: &mut Canvas, t: &Theme, geo: &Geo) {
    let dpi = p.dpi;
    let font = |f: F| p.fonts.get(f);
    cv.text("Quick Fix", sc(PAD, dpi), sc(16, dpi), geo.w, sc(32, dpi), font(F::Title), t.text, DT_LEFT | DT_SINGLELINE);
    let mut y = sc(62, dpi);
    let w = geo.w - 2 * sc(PAD, dpi);
    for (i, (text, f)) in FIRST_USE.iter().enumerate() {
        let h = crate::canvas::text_height(text, font(*f), w);
        cv.text(text, sc(PAD, dpi), y, w, h + 4, font(*f), if i == 0 { t.text } else { t.text2 }, crate::canvas::DT_WORDBREAK);
        y += h + sc(14, dpi);
    }
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
    bar::register_class();
    let hinst = GetModuleHandleW(std::ptr::null());
    let cls = to_wide(REVIEW_WINDOW_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DROPSHADOW,
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
}

pub fn hwnd() -> HWND {
    PANEL.with(|p| p.borrow().as_ref().map(|p| p.hwnd).unwrap_or(std::ptr::null_mut()))
}

/// Opens the panel (GUI thread only). A panel that is already open is replaced.
pub fn open_review(req: ReviewRequest, first_use: bool) {
    unsafe {
        close();
        refresh_dark_mode();
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = to_wide(REVIEW_WINDOW_CLASS);
        let edits = normalize_edits(&req.original, &req.edits);
        let title = to_wide(&format!(
            "KeyReed Quick Fix, {} change{}. Enter replaces, Escape dismisses.",
            edits.len(),
            if edits.len() == 1 { "" } else { "s" }
        ));
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            cls.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            sc(WIN_W, 96),
            sc(120, 96),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        let dpi = window_dpi(hwnd);
        let accepted = vec![true; edits.len()];
        let mut panel = Panel {
            hwnd,
            original: req.original,
            edits,
            note: req.note,
            accepted,
            first_use,
            show_original: false,
            focus: None,
            hover: None,
            opened: Instant::now(),
            dpi,
            fonts: Fonts::new(dpi),
            scroll: 0,
            layout: None,
        };
        let (w, h) = (sc(WIN_W, dpi), sc(panel.window_h(), dpi));
        // Windows 11 rounds the corners; Windows 10 keeps them square. Ignored where unsupported.
        let round: u32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const _, 4);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let x = wa.left + ((wa.right - wa.left) - w) / 2;
        let y = wa.top + ((wa.bottom - wa.top) as f32 * 0.22) as i32;
        let reduced = reduced_motion();
        SetLayeredWindowAttributes(hwnd, 0, if reduced { 255 } else { 0 }, LWA_ALPHA);
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
        panel.opened = Instant::now();
        PANEL.with(|p| *p.borrow_mut() = Some(panel));
        ShowWindow(hwnd, SW_SHOW);
        let _ = crate::typing::force_foreground_window(hwnd as isize);
        if !reduced {
            SetTimer(hwnd, IDT_REVIEW_ANIM, 16, None);
        }
    }
}

pub fn close() {
    let h = hwnd();
    if !h.is_null() {
        unsafe {
            DestroyWindow(h);
        }
    }
}

/// Test hook for the gallery: sets what a person could do with the mouse and keyboard.
pub fn set_state_for_test(show_original: bool, rejected: &[usize], focus: Option<usize>, hover: Option<usize>) {
    PANEL.with(|p| {
        let Ok(mut b) = p.try_borrow_mut() else { return };
        let Some(pn) = b.as_mut() else { return };
        pn.show_original = show_original;
        for (i, a) in pn.accepted.iter_mut().enumerate() {
            *a = !rejected.contains(&i);
        }
        pn.focus = focus;
        pn.hover = hover.map(Hit::Change);
        pn.layout = None;
        let h = pn.hwnd;
        let (w, hh) = (sc(WIN_W, pn.dpi), sc(pn.window_h(), pn.dpi));
        unsafe {
            SetWindowPos(h, std::ptr::null_mut(), 0, 0, w, hh, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            InvalidateRect(h, std::ptr::null(), 0);
        }
    });
}

fn finish(choice: ReviewChoice) {
    let (text, accepted, rejected, hwnd) = PANEL.with(|p| {
        let b = p.borrow();
        match b.as_ref() {
            Some(pn) => (pn.finish_text(), pn.applied() as u32, (pn.edits.len() - pn.applied()) as u32, pn.hwnd),
            None => (String::new(), 0, 0, std::ptr::null_mut()),
        }
    });
    if hwnd.is_null() {
        return;
    }
    send(UiAction::ReviewDone { choice, text, accepted, rejected });
    unsafe {
        DestroyWindow(hwnd);
    }
}

/// Moves the focus ring to the next or previous change and scrolls it into view.
fn move_focus(p: &mut Panel, forward: bool) {
    let n = p.edits.len();
    if n == 0 {
        return;
    }
    p.focus = Some(match (p.focus, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    });
    p.ensure_layout();
    let line_h = sc(LINE_H, p.dpi);
    let target = p.focus.and_then(|f| {
        let l = p.layout.as_ref()?;
        l.placed.iter().find(|pl| l.toks[pl.tok] == Tok::Change(f)).map(|pl| pl.y)
    });
    if let Some(y) = target {
        let view = p.view_h();
        if y < p.scroll {
            p.scroll = y;
        } else if y + line_h > p.scroll + view {
            p.scroll = y + line_h - view;
        }
        p.scroll = p.scroll.clamp(0, p.max_scroll());
    }
}

fn toggle(p: &mut Panel, i: usize) {
    if let Some(a) = p.accepted.get_mut(i) {
        *a = !*a;
        p.layout = None;
    }
}

fn relayout_window(p: &mut Panel) {
    let (w, h) = (sc(WIN_W, p.dpi), sc(p.window_h(), p.dpi));
    p.scroll = p.scroll.clamp(0, p.max_scroll());
    unsafe {
        SetWindowPos(p.hwnd, std::ptr::null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        InvalidateRect(p.hwnd, std::ptr::null(), 0);
    }
}

fn client_xy(lparam: LPARAM) -> (i32, i32) {
    ((lparam & 0xFFFF) as u16 as i16 as i32, ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32)
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            PANEL.with(|p| {
                if let Ok(mut b) = p.try_borrow_mut() {
                    if let Some(pn) = b.as_mut() {
                        paint(pn, hdc);
                    }
                }
            });
            EndPaint(hwnd, &ps);
            0
        }
        WM_NCHITTEST => {
            // The header drags the panel.
            let (mut x, mut y) = (((lparam & 0xFFFF) as u16 as i16) as i32, (((lparam >> 16) & 0xFFFF) as u16 as i16) as i32);
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            x -= r.left;
            y -= r.top;
            let in_header = PANEL.with(|p| p.try_borrow().ok().and_then(|b| b.as_ref().map(|pn| y < sc(HEADER_H, pn.dpi) && x < sc(WIN_W - 2 * PAD, pn.dpi)))).unwrap_or(false);
            if in_header { HTCAPTION as LRESULT } else { HTCLIENT as LRESULT }
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_xy(lparam);
            let changed = PANEL.with(|p| {
                let Ok(mut b) = p.try_borrow_mut() else { return false };
                let Some(pn) = b.as_mut() else { return false };
                let h = pn.hit(x, y);
                let changed = h != pn.hover;
                pn.hover = h;
                if changed {
                    SetCursor(LoadCursorW(std::ptr::null_mut(), if h.is_some() { IDC_HAND } else { IDC_ARROW }));
                }
                changed
            });
            if changed {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = client_xy(lparam);
            let hit = PANEL.with(|p| p.try_borrow_mut().ok().and_then(|mut b| b.as_mut().and_then(|pn| pn.hit(x, y))));
            match hit {
                Some(Hit::Change(i)) => {
                    PANEL.with(|p| {
                        if let Some(pn) = p.borrow_mut().as_mut() {
                            toggle(pn, i);
                            pn.focus = Some(i);
                            relayout_window(pn);
                        }
                    });
                }
                Some(Hit::ShowOriginal) => {
                    PANEL.with(|p| {
                        if let Some(pn) = p.borrow_mut().as_mut() {
                            pn.show_original = !pn.show_original;
                            pn.layout = None;
                            relayout_window(pn);
                        }
                    });
                }
                Some(Hit::Replace) => finish(ReviewChoice::Replace),
                Some(Hit::Copy) => finish(ReviewChoice::Copy),
                Some(Hit::Dismiss) => finish(ReviewChoice::Dismiss),
                Some(Hit::GotIt) => got_it(),
                None => {}
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            PANEL.with(|p| {
                if let Some(pn) = p.borrow_mut().as_mut() {
                    let max = pn.max_scroll();
                    pn.scroll = (pn.scroll - delta * sc(56, pn.dpi) / 120).clamp(0, max);
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            });
            0
        }
        WM_KEYDOWN => {
            let vk = wparam as u16;
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            let shift = GetKeyState(VK_SHIFT as i32) < 0;
            let first_use = PANEL.with(|p| p.borrow().as_ref().is_some_and(|pn| pn.first_use));
            match vk {
                VK_RETURN if first_use => got_it(),
                VK_RETURN => finish(ReviewChoice::Replace),
                VK_ESCAPE => finish(ReviewChoice::Dismiss),
                0x43 if ctrl && !first_use => finish(ReviewChoice::Copy), // Ctrl+C
                _ if first_use => {}
                VK_TAB | VK_RIGHT | VK_LEFT => {
                    let forward = if vk == VK_TAB { !shift } else { vk == VK_RIGHT };
                    PANEL.with(|p| {
                        if let Some(pn) = p.borrow_mut().as_mut() {
                            move_focus(pn, forward);
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                    });
                }
                VK_SPACE => {
                    PANEL.with(|p| {
                        if let Some(pn) = p.borrow_mut().as_mut() {
                            if let Some(i) = pn.focus {
                                toggle(pn, i);
                                relayout_window(pn);
                            }
                        }
                    });
                }
                0x4F => {
                    // O: show the original words next to the changes.
                    PANEL.with(|p| {
                        if let Some(pn) = p.borrow_mut().as_mut() {
                            pn.show_original = !pn.show_original;
                            pn.layout = None;
                            relayout_window(pn);
                        }
                    });
                }
                _ => {}
            }
            0
        }
        WM_TIMER if wparam == IDT_REVIEW_ANIM => {
            let (alive, animating, ms) = PANEL.with(|p| {
                let b = p.borrow();
                b.as_ref().map(|pn| (true, pn.animating(), pn.opened.elapsed().as_secs_f32() * 1000.0)).unwrap_or((false, false, 0.0))
            });
            if alive {
                SetLayeredWindowAttributes(hwnd, 0, (255.0 * ease_out(ms / FADE_MS)) as u8, LWA_ALPHA);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            if !animating {
                KillTimer(hwnd, IDT_REVIEW_ANIM);
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
            0
        }
        WM_CLOSE => {
            finish(ReviewChoice::Dismiss);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, IDT_REVIEW_ANIM);
            PANEL.with(|p| *p.borrow_mut() = None);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Leaves the first-use screen for the review, and tells the coordinator it was seen.
fn got_it() {
    PANEL.with(|p| {
        if let Some(pn) = p.borrow_mut().as_mut() {
            pn.first_use = false;
            pn.layout = None;
            pn.opened = Instant::now();
            relayout_window(pn);
        }
    });
    send(UiAction::QuickFixIntroSeen);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(start: usize, end: usize, to: &str) -> ReviewEdit {
        ReviewEdit { start, end, replacement: to.into(), label: "Spelling" }
    }

    #[test]
    fn normalize_keeps_valid_ordered_edits_and_drops_the_rest() {
        let text = "teh quick brown fox";
        let edits = vec![
            e(10, 15, "brown"),   // no-op
            e(4, 9, "quick!"),    // later than the next one in the list, valid
            e(0, 3, "the"),
            e(2, 6, "x"),         // overlaps the first edit
            e(50, 60, "out"),     // out of range
            e(5, 3, "reversed"),  // start after end
        ];
        let got = normalize_edits(text, &edits);
        assert_eq!(got.iter().map(|g| (g.start, g.end)).collect::<Vec<_>>(), vec![(0, 3), (4, 9)]);
        // A boundary inside a multi-byte character is refused.
        assert!(normalize_edits("café x", &[e(3, 4, "e")]).is_empty());
        assert_eq!(normalize_edits("café x", &[e(3, 5, "e")]).len(), 1);
        // Whitespace both sides share is moved out of the change, so only the words are marked.
        let t = "Thanks the the team";
        let got = normalize_edits(t, &[e(6, 14, " the")]);
        assert_eq!((got[0].start, got[0].end, got[0].replacement.as_str()), (7, 14, "the"));
        assert_eq!(final_text(t, &got, &[true]), "Thanks the team");
        let got = normalize_edits("a  b ", &[e(1, 5, " c ")]);
        assert_eq!((got[0].start, got[0].end, got[0].replacement.as_str()), (2, 4, "c"));
    }

    #[test]
    fn final_text_applies_only_accepted_edits() {
        let text = "teh quick  brwn fox";
        let edits = normalize_edits(text, &[e(0, 3, "the"), e(11, 15, "brown"), e(19, 19, "!")]);
        assert_eq!(final_text(text, &edits, &[true, true, true]), "the quick  brown fox!");
        assert_eq!(final_text(text, &edits, &[true, false, true]), "the quick  brwn fox!");
        assert_eq!(final_text(text, &edits, &[false, false, false]), text);
        assert_eq!(final_text("", &[], &[]), "");
        // A deletion and an insertion.
        let edits = normalize_edits("a the the b", &[e(2, 6, "")]);
        assert_eq!(final_text("a the the b", &edits, &[true]), "a the b");
    }

    #[test]
    fn tokens_keep_touching_words_and_changes_together() {
        let edits = normalize_edits("(teh) cat\nsat", &[e(1, 4, "the")]);
        let toks = tokens("(teh) cat\nsat", &edits);
        assert_eq!(
            toks,
            vec![
                Tok::Word("(".into()),
                Tok::Change(0),
                Tok::Word(")".into()),
                Tok::Space,
                Tok::Word("cat".into()),
                Tok::Break,
                Tok::Word("sat".into()),
            ]
        );
    }

    #[test]
    fn layout_wraps_whole_runs_and_honours_breaks() {
        let toks = vec![
            Tok::Word("aaaa".into()),
            Tok::Space,
            Tok::Word("(".into()),
            Tok::Change(0),
            Tok::Word(")".into()),
            Tok::Space,
            Tok::Word("bb".into()),
            Tok::Break,
            Tok::Word("cc".into()),
        ];
        let widths = vec![40, 0, 10, 30, 10, 0, 20, 0, 20];
        let (placed, h) = layout(&toks, &widths, 5, 80, 20);
        let at = |tok: usize| placed.iter().find(|p| p.tok == tok).map(|p| (p.x, p.y)).unwrap();
        assert_eq!(at(0), (0, 0));
        // "(", change and ")" form one 50-wide run: 40 + 5 + 50 > 80, so the run moves to the next line whole.
        assert_eq!((at(2), at(3), at(4)), ((0, 20), (10, 20), (40, 20)));
        assert_eq!(at(6), (55, 20)); // 50 + 5 gap = 55; 55 + 20 <= 80
        assert_eq!(at(8), (0, 40)); // after the break
        assert_eq!(h, 60);
        // A run wider than the line overflows instead of looping or panicking.
        let (placed, h) = layout(&[Tok::Word("x".into())], &[500], 5, 80, 20);
        assert_eq!((placed.len(), h), (1, 20));
        assert_eq!(layout(&[], &[], 5, 80, 20), (vec![], 20));
    }

    #[test]
    fn stagger_keeps_the_whole_sequence_short() {
        assert_eq!(stagger_ms(1), 40.0);
        assert!(stagger_ms(8) * 8.0 <= 160.0 + 0.001);
        assert!(stagger_ms(100) * 100.0 <= 160.0 + 0.001);
        assert_eq!(stagger_ms(0), 40.0);
    }
}
