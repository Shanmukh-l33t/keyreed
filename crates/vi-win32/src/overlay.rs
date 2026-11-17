//! Floating listener renderer (pure software, premultiplied BGRA, unit-testable).
//!
//! Design (v0.4): an icon-first, compact capsule that only grows when it has something to say.
//!
//! | State        | Content                                          | Logical size @100% |
//! |--------------|--------------------------------------------------|--------------------|
//! | Idle         | KeyReed mark only                                | 36 x 36            |
//! | Idle + hover | mark + writing mode + shortcut                   | text-sized         |
//! | Listening    | mark + 11-bar waveform of *measured* mic levels  | 63 x 22 (60 % of 104 x 36) |
//! | Processing   | mark + "Transcribing" (teal) with a moving sheen | text-sized         |
//! | Done         | mark + check + "Inserted" / "Copied"             | text-sized         |
//! | Error        | mark + alert + short plain-language reason       | text-sized         |
//! | Fixing       | mark + "Fixing", one band of light sweeping      | text-sized         |
//! | Fixed        | mark + tick + "Fixed · N changes" + Undo/Deeper  | text-sized         |
//!
//! The window itself is `WS_EX_LAYERED | WS_EX_NOACTIVATE`; this module only produces pixels.
//! Text is rasterised by the caller (GDI, grayscale AA) and passed in as a coverage mask so the
//! final alpha is exact (GDI cannot write alpha into layered windows).

use crate::icon_data::{KEYREED_ICON_64, KEYREED_ICON_SIZE};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Logical (96-DPI) capsule height. Every other dimension derives from it.
pub const PILL_HEIGHT: f32 = 36.0;
/// Transparent margin around the capsule that holds the soft shadow.
pub const PILL_MARGIN: f32 = 4.0;
/// Number of waveform bars while listening.
pub const WAVE_BARS: usize = 11;
/// While listening the pill draws at 60 percent of every metric, so the open mic covers less of the user's work.
pub const LISTENING_SCALE: f32 = 0.60;

/// Visual state of the listener. Mapped from `EngineState` by the window code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillKind {
    Idle,
    IdleHover,
    Disabled,
    Listening,
    Processing,
    Done,
    /// Neutral outcome (discarded, nothing heard): label only, no success glyph.
    Notice,
    /// Needs the user's attention but is not an error (e.g. review a command before pasting).
    Attention,
    Error,
    /// Quick Fix is working: one band of light sweeps across the capsule (plan F4).
    Fixing,
    /// Quick Fix replaced text: tick, change count and the Undo and Deeper chips.
    Fixed,
}

/// Brand colours (from website/styles.css dark theme) as straight RGB.
pub mod palette {
    pub const SURFACE: (u8, u8, u8) = (0x0C, 0x14, 0x11);
    pub const SURFACE_HOVER: (u8, u8, u8) = (0x12, 0x1D, 0x18);
    pub const TEXT: (u8, u8, u8) = (0xE8, 0xF1, 0xEC);
    pub const TEXT_2: (u8, u8, u8) = (0xA5, 0xB8, 0xAF);
    pub const TEXT_3: (u8, u8, u8) = (0x6F, 0x85, 0x7B);
    pub const EMERALD: (u8, u8, u8) = (0x10, 0xB9, 0x81);
    pub const EMERALD_2: (u8, u8, u8) = (0x34, 0xD3, 0x99);
    pub const MINT: (u8, u8, u8) = (0xA7, 0xF3, 0xD0);
    pub const WARN: (u8, u8, u8) = (0xF5, 0xC2, 0x6B);
    pub const ROSE: (u8, u8, u8) = (0xFB, 0x71, 0x85);
    pub const ROSE_TEXT: (u8, u8, u8) = (0xFE, 0xCD, 0xD3);
    pub const AMBER_TEXT: (u8, u8, u8) = (0xFD, 0xE6, 0x8A);
    /// "Working" (Processing, Fixing): a cool teal, so amber stays reserved for "needs your attention".
    pub const WORKING: (u8, u8, u8) = (0x2D, 0xD4, 0xBF);
    pub const WORKING_TEXT: (u8, u8, u8) = (0x99, 0xF6, 0xE4);
    /// Slightly lifted circle behind the mark so the keycap's dark body keeps its silhouette.
    pub const WELL: (u8, u8, u8) = (0x1B, 0x29, 0x23);
}

/// Time curves for the Quick Fix pill (plan F4). Each is a pure function of the milliseconds since the stage began;
/// pass `f32::INFINITY` for reduced motion and every curve returns its resting value.
pub mod motion {
    use std::f32::consts::PI;

    /// Longest motion, the pop and the tick (the plan's budget is 250 ms per transition).
    pub const TICK_MS: f32 = 200.0;
    const COUNT_MS: f32 = 180.0;
    const NOD_MS: f32 = 160.0;
    const SHAKE_MS: f32 = 200.0;
    /// After this the pill is at rest.
    pub const SETTLE_MS: f32 = 250.0;

    fn ease_out(t: f32) -> f32 {
        1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
    }

    /// Capsule pulse 0..1..0 (scale to 108 percent and back).
    pub fn pop(ms: f32) -> f32 {
        if ms >= TICK_MS { 0.0 } else { (PI * ms.max(0.0) / TICK_MS).sin() }
    }

    /// How much of the tick is drawn while it draws itself.
    pub fn tick(ms: f32) -> f32 {
        ease_out(ms / TICK_MS)
    }

    /// How much of the tick remains while it unwinds (undo).
    pub fn tick_unwind(ms: f32) -> f32 {
        1.0 - ease_out(ms / TICK_MS)
    }

    /// The change count rolling up from zero to `n`.
    pub fn count_up(ms: f32, n: u32) -> u32 {
        (n as f32 * ease_out(ms / COUNT_MS)).round() as u32
    }

    /// The change count rolling back down from `n` to zero.
    pub fn count_down(ms: f32, n: u32) -> u32 {
        n - count_up(ms, n)
    }

    /// Vertical offset in logical pixels: a 2 px dip and back ("Looks good" nods).
    pub fn nod(ms: f32) -> f32 {
        if ms >= NOD_MS { 0.0 } else { 2.0 * (PI * ms.max(0.0) / NOD_MS).sin() }
    }

    /// Horizontal offset in logical pixels: two short shakes of 6 px in 200 ms (errors and refusals).
    pub fn shake(ms: f32) -> f32 {
        if ms >= SHAKE_MS { 0.0 } else { 6.0 * (1.0 - ms.max(0.0) / SHAKE_MS) * (4.0 * PI * ms.max(0.0) / SHAKE_MS).sin() }
    }
}

/// Grayscale text coverage mask (0..=255), row-major.
#[derive(Debug, Clone, Copy)]
pub struct TextMask<'a> {
    pub width: usize,
    pub height: usize,
    pub coverage: &'a [u8],
}

/// One frame of the listener.
#[derive(Debug, Clone, Copy)]
pub struct PillFrame<'a> {
    /// DPI scale (1.0 = 96 DPI).
    pub scale: f32,
    pub kind: PillKind,
    /// Bar heights, oldest first, each in `0.0..=1.0` and already shaped (see `WaveSim::bars`). Only used while listening.
    pub levels: &'a [f32],
    /// Seconds since the state began; drives the processing sheen only.
    pub phase: f32,
    /// Primary label (mode name, status word, error reason).
    pub label: Option<TextMask<'a>>,
    /// Optional secondary label shown after the primary one in a dimmer colour (e.g. shortcut).
    /// On `Fixed` it is the first action chip ("Undo").
    pub secondary: Option<TextMask<'a>>,
    /// On `Fixed`, the second action chip ("Deeper").
    pub tertiary: Option<TextMask<'a>>,
    /// Capsule pulse 0..=1 (the "pop" after a fix); 0 for every other state.
    pub pop: f32,
    /// How much of the tick is drawn, 0..=1; 1 for every state that is not animating a tick.
    pub tick: f32,
    /// Opacity multiplier for the whole frame (0..=1).
    pub opacity: f32,
    /// Opacity of state content (labels, glyphs, bars) so it can fade in after a change.
    pub content_opacity: f32,
}

/// Physical geometry derived from scale.
#[derive(Debug, Clone, Copy)]
pub struct PillMetrics {
    pub margin: usize,
    pub cap_h: usize,
    pub icon: usize,
    pub content_x: usize,
    pub pad_right: usize,
    pub gap: usize,
    pub glyph: usize,
    pub bar_w: f32,
    pub bar_gap: f32,
}

impl PillMetrics {
    pub fn for_scale(scale: f32) -> Self {
        let s = scale.max(0.5);
        let cap_h = (PILL_HEIGHT * s).round() as usize;
        let margin = (PILL_MARGIN * s).round() as usize;
        let icon = ((22.0 * s).round() as usize).max(12);
        PillMetrics {
            margin,
            cap_h,
            icon,
            // Content starts 5 dp right of the icon well (well edge = margin + cap_h - 3 dp), so text never touches it.
            content_x: margin + cap_h + (2.0 * s).round() as usize,
            pad_right: (13.0 * s).round() as usize,
            gap: (6.0 * s).round() as usize,
            glyph: (12.0 * s).round() as usize,
            bar_w: (2.5 * s).max(1.5),
            bar_gap: (2.5 * s).max(1.5),
        }
    }

    pub fn window_height(&self) -> usize {
        self.cap_h + 2 * self.margin
    }

    /// Metrics for one pill state: Listening draws at `LISTENING_SCALE`, every other state at `scale`.
    pub fn for_pill(scale: f32, kind: PillKind) -> Self {
        Self::for_scale(if kind == PillKind::Listening { scale * LISTENING_SCALE } else { scale })
    }

    fn wave_width(&self) -> usize {
        (WAVE_BARS as f32 * self.bar_w + (WAVE_BARS as f32 - 1.0) * self.bar_gap).ceil() as usize
    }
}

/// Width of the window (including shadow margin) required for `kind` with the given label widths.
pub fn pill_window_width(scale: f32, kind: PillKind, label_w: usize, secondary_w: usize, tertiary_w: usize) -> usize {
    let m = PillMetrics::for_pill(scale, kind);
    let content = match kind {
        PillKind::Idle | PillKind::Disabled => return m.cap_h + 2 * m.margin,
        PillKind::Listening => m.wave_width(),
        PillKind::Done | PillKind::Attention => m.glyph + m.gap + label_w,
        // An error pill carries an action chip only when the failure has a fix (e.g. the missing grammar model).
        PillKind::Error => {
            m.glyph + m.gap + label_w + if secondary_w > 0 { (m.gap as f32 + chip_width(scale, secondary_w)).ceil() as usize } else { 0 }
        }
        PillKind::IdleHover => label_w + if secondary_w > 0 { m.gap + secondary_w } else { 0 },
        PillKind::Processing | PillKind::Notice | PillKind::Fixing => label_w,
        PillKind::Fixed => {
            let chips: f32 = [secondary_w, tertiary_w]
                .iter()
                .filter(|w| **w > 0)
                .map(|w| m.gap as f32 + chip_width(scale, *w))
                .sum();
            m.glyph + m.gap + label_w + chips.ceil() as usize
        }
    };
    let tail = if matches!(kind, PillKind::Fixed | PillKind::Error) && secondary_w + tertiary_w > 0 {
        chip_inset(scale).round() as usize
    } else {
        m.pad_right
    };
    m.content_x + content + tail + m.margin
}

fn chip_height(scale: f32) -> f32 {
    22.0 * scale
}

fn chip_width(scale: f32, text_w: usize) -> f32 {
    text_w as f32 + 16.0 * scale
}

/// Space above, below and after the chips so they sit inside the capsule's curve.
fn chip_inset(scale: f32) -> f32 {
    ((PILL_HEIGHT * scale) - chip_height(scale)) / 2.0
}

/// Window-relative `(x0, x1)` of each action chip on a `Fixed` pill, for the given text widths (zero = absent).
/// The renderer and the click handler both use this, so a click lands on what is drawn.
pub fn fixed_chips(scale: f32, label_w: usize, texts: [usize; 2]) -> Vec<(f32, f32)> {
    let m = PillMetrics::for_scale(scale);
    let mut x = (m.content_x + m.glyph + m.gap + label_w) as f32;
    let mut out = Vec::new();
    for w in texts.into_iter().filter(|w| *w > 0) {
        x += m.gap as f32;
        let cw = chip_width(scale, w);
        out.push((x, x + cw));
        x += cw;
    }
    out
}

/// Maps an RMS value (linear, 0..1) to a perceptual 0..1 level using a -55 dBFS .. -15 dBFS window.
pub fn rms_to_level(rms: f32) -> f32 {
    if !rms.is_finite() || rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 55.0) / 40.0).clamp(0.0, 1.0)
}

/// Meter ballistics for the listening waveform: pure and unit-testable, fed with the loudest RMS seen since the
/// previous step. The constants follow a studied reference (35 ms attack, 95 ms decay) and the level is auto-ranged
/// to the speaker (peak and room tracking), so a quiet laptop microphone still lights the bars. It reacts to
/// measured audio only: with no input the bars rest at the floor.
#[derive(Debug, Clone)]
pub struct WaveSim {
    peak: f32,
    room: f32,
    level: f32,
    hist: [f32; WAVE_BARS],
    acc: f32,
}

impl Default for WaveSim {
    fn default() -> Self {
        Self::new()
    }
}

impl WaveSim {
    pub const fn new() -> Self {
        Self { peak: -30.0, room: -55.0, level: 0.0, hist: [0.0; WAVE_BARS], acc: 0.0 }
    }

    fn follow(v: &mut f32, target: f32, tau: f32, dt: f32) {
        *v += (target - *v) * (1.0 - (-dt / tau.max(1e-3)).exp());
    }

    /// Clears the bars and the smoothed level but keeps `peak` and `room`, so the auto level remembers the speaker.
    pub fn restart(&mut self) {
        self.level = 0.0;
        self.hist = [0.0; WAVE_BARS];
        self.acc = 0.0;
    }

    /// `rms`: hold-max RMS (linear, 0..1) since the previous call. `dt`: wall-clock seconds since the previous call.
    pub fn step(&mut self, rms: f32, dt: f32) {
        let dt = dt.clamp(1.0 / 240.0, 1.0 / 20.0);
        let rms = if rms.is_finite() { rms.clamp(0.0, 1.0) } else { 0.0 };
        let db = 20.0 * (rms + 1e-6).log10();
        let room_tau = if db < self.room { 0.3 } else { 6.0 };
        Self::follow(&mut self.room, db, room_tau, dt);
        let floor = self.room.max(-60.0);
        let lowest = floor + 16.0;
        if db > self.peak {
            Self::follow(&mut self.peak, db, 0.04, dt);
        } else {
            Self::follow(&mut self.peak, lowest, 3.0, dt);
        }
        self.peak = self.peak.max(lowest);
        let low = (floor + 5.0).max(self.peak - 24.0);
        let raw = ((db - low) / (self.peak - low).max(1.0)).clamp(0.0, 1.0);
        let level_tau = if raw > self.level { 0.035 } else { 0.095 };
        Self::follow(&mut self.level, raw, level_tau, dt);
        self.acc += dt;
        while self.acc >= 1.0 / 30.0 {
            self.acc -= 1.0 / 30.0;
            self.hist.rotate_left(1);
            self.hist[WAVE_BARS - 1] = self.level;
        }
    }

    /// Bar heights, oldest first. Loudness reads as its square root, so ordinary speech visibly moves the bars.
    pub fn bars(&self) -> [f32; WAVE_BARS] {
        self.hist.map(f32::sqrt)
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let m = |x: u8, y: u8| lerp(x as f32, y as f32, t).round().clamp(0.0, 255.0) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Resting grey to emerald as a bar rises, with a bright mint core on loud peaks (no hard threshold).
fn bar_color(v: f32) -> (u8, u8, u8) {
    let base = mix_rgb(palette::TEXT_3, palette::EMERALD_2, smoothstep(0.03, 0.14, v));
    if v > 0.7 {
        mix_rgb(base, palette::MINT, (0.6 * (v - 0.7) / 0.3).min(0.6))
    } else {
        base
    }
}

thread_local! {
    static ICON_CACHE: RefCell<HashMap<usize, Rc<Vec<u32>>>> = RefCell::new(HashMap::new());
}

/// `scaled_icon` memoised per size: the listener repaints every 16 ms while listening and the mark never changes.
fn cached_icon(n: usize) -> Rc<Vec<u32>> {
    ICON_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 8 {
            c.clear();
        }
        c.entry(n).or_insert_with(|| Rc::new(scaled_icon(n))).clone()
    })
}

/// Box-filters the 64px brand mark to `n` px (premultiplied BGRA).
pub fn scaled_icon(n: usize) -> Vec<u32> {
    let src_n = KEYREED_ICON_SIZE;
    let n = n.clamp(1, 256);
    let mut out = vec![0u32; n * n];
    let ratio = src_n as f32 / n as f32;
    for dy in 0..n {
        let sy0 = dy as f32 * ratio;
        let sy1 = sy0 + ratio;
        for dx in 0..n {
            let sx0 = dx as f32 * ratio;
            let sx1 = sx0 + ratio;
            let (mut a, mut r, mut g, mut b, mut wsum) = (0f32, 0f32, 0f32, 0f32, 0f32);
            let mut sy = sy0.floor() as usize;
            while (sy as f32) < sy1 && sy < src_n {
                let wy = ((sy + 1) as f32).min(sy1) - (sy as f32).max(sy0);
                let mut sx = sx0.floor() as usize;
                while (sx as f32) < sx1 && sx < src_n {
                    let wx = ((sx + 1) as f32).min(sx1) - (sx as f32).max(sx0);
                    let w = wx * wy;
                    let p = KEYREED_ICON_64[sy * src_n + sx];
                    a += ((p >> 24) & 0xFF) as f32 * w;
                    r += ((p >> 16) & 0xFF) as f32 * w;
                    g += ((p >> 8) & 0xFF) as f32 * w;
                    b += (p & 0xFF) as f32 * w;
                    wsum += w;
                    sx += 1;
                }
                sy += 1;
            }
            if wsum > 0.0 {
                let f = |v: f32| ((v / wsum).round().clamp(0.0, 255.0)) as u32;
                out[dy * n + dx] = (f(a) << 24) | (f(r) << 16) | (f(g) << 8) | f(b);
            }
        }
    }
    out
}

/// Renders a frame into a premultiplied BGRA buffer of `width x height` (window size incl. margin).
pub fn render_pill(width: usize, height: usize, f: &PillFrame) -> Vec<u32> {
    let mut px = vec![0u32; width * height];
    if width < 8 || height < 8 {
        return px;
    }
    let m = PillMetrics::for_pill(f.scale, f.kind);
    // The pop grows the capsule into its shadow margin (about 4 percent on every side at the peak).
    let margin = m.margin as f32 - f.pop.clamp(0.0, 1.0) * 0.04 * m.cap_h as f32;
    let cap_h = (height as f32 - 2.0 * margin).max(4.0);
    let cap_w = (width as f32 - 2.0 * margin).max(cap_h);
    let radius = cap_h * 0.5;
    let cy = margin + radius;
    let left_cx = margin + radius;
    let right_cx = margin + cap_w - radius;

    let (accent, border_a, surface) = match f.kind {
        PillKind::Idle => (palette::MINT, 0.16, palette::SURFACE),
        PillKind::IdleHover => (palette::MINT, 0.30, palette::SURFACE_HOVER),
        PillKind::Disabled => (palette::TEXT_3, 0.18, palette::SURFACE),
        PillKind::Listening => (palette::EMERALD, 0.85, palette::SURFACE),
        PillKind::Processing => (palette::WORKING, 0.55, palette::SURFACE),
        PillKind::Done => (palette::EMERALD_2, 0.70, palette::SURFACE),
        PillKind::Notice => (palette::TEXT_2, 0.35, palette::SURFACE),
        PillKind::Attention => (palette::WARN, 0.75, palette::SURFACE),
        PillKind::Error => (palette::ROSE, 0.80, palette::SURFACE),
        PillKind::Fixing => (palette::WORKING, 0.55, palette::SURFACE),
        PillKind::Fixed => (palette::EMERALD_2, 0.70, palette::SURFACE),
    };
    let body_alpha = 0.96f32;
    let opacity = f.opacity.clamp(0.0, 1.0);
    let border_w = (1.0 * f.scale).max(1.0);
    let shadow_r = margin.max(1.0);

    // 1) Capsule body + hairline border + soft shadow (signed distance field).
    for y in 0..height {
        let py = y as f32 + 0.5;
        for x in 0..width {
            let pxf = x as f32 + 0.5;
            let cx = pxf.clamp(left_cx, right_cx);
            let d = ((pxf - cx).powi(2) + (py - cy).powi(2)).sqrt() - radius; // <0 inside
            let mut col = (0f32, 0f32, 0f32, 0f32); // straight rgb, alpha
            if d < 0.75 {
                let cov = (0.5 - d).clamp(0.0, 1.0);
                let border_t = ((d + border_w + 0.5).clamp(0.0, 1.0)) * border_a;
                let r = lerp(surface.0 as f32, accent.0 as f32, border_t);
                let g = lerp(surface.1 as f32, accent.1 as f32, border_t);
                let b = lerp(surface.2 as f32, accent.2 as f32, border_t);
                // One band of light, once per 600 ms, only while Quick Fix is working.
                let lift = if f.kind == PillKind::Fixing {
                    let band_x = margin + cap_w * ((f.phase / 0.6).fract() * 1.5 - 0.25);
                    0.22 * (1.0 - ((pxf - band_x).abs() / (cap_h * 0.9))).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                col = (
                    lerp(r, palette::MINT.0 as f32, lift),
                    lerp(g, palette::MINT.1 as f32, lift),
                    lerp(b, palette::MINT.2 as f32, lift),
                    body_alpha * cov,
                );
            }
            // Shadow outside the body (black, quadratic fall-off over the margin).
            if d > -0.5 && d < shadow_r {
                let t = 1.0 - (d.max(0.0) / shadow_r);
                let sa = 0.22 * t * t;
                let (r, g, b, a) = col;
                let out_a = a + sa * (1.0 - a);
                if out_a > 0.0 {
                    col = (r * a / out_a, g * a / out_a, b * a / out_a, out_a);
                }
            }
            let a = col.3 * opacity;
            if a > 0.0 {
                px[y * width + x] = pack(col.0, col.1, col.2, a);
            }
        }
    }

    // 2) Brand mark in the left well (centred in a circle of diameter cap_h).
    let icon_n = m.icon.min(cap_h as usize);
    let icon = cached_icon(icon_n);
    let icon_alpha = if f.kind == PillKind::Disabled { 0.45 } else { 1.0 } * opacity;
    fill_circle(&mut px, width, height, left_cx, cy, radius - 3.0 * f.scale, palette::WELL, opacity);
    let ix0 = (left_cx - icon_n as f32 / 2.0).round() as isize;
    let iy0 = (cy - icon_n as f32 / 2.0).round() as isize;
    blit_premul(&mut px, width, height, icon.as_slice(), icon_n, icon_n, ix0, iy0, icon_alpha);


    if matches!(f.kind, PillKind::Idle | PillKind::Disabled) {
        return px;
    }

    let base = px.clone();
    let content_x = m.content_x as f32;
    let opacity = opacity * f.content_opacity.clamp(0.0, 1.0);
    match f.kind {
        PillKind::Listening => {
            let max_half = cap_h * 0.30;
            let min_half = 1.2 * f.scale;
            let n = f.levels.len();
            // Newest sample on the right; missing history renders as the floor.
            let bar = |i: usize| if n >= WAVE_BARS - i { f.levels[n - (WAVE_BARS - i)].clamp(0.0, 1.0) } else { 0.0 };
            // One soft capsule behind the bars, a little brighter the louder it is (no blur needed).
            let mean = (0..WAVE_BARS).map(bar).sum::<f32>() / WAVE_BARS as f32;
            let pad = 2.0 * f.scale;
            let (gx, gy) = (content_x - pad, cy - max_half - pad);
            fill_hcapsule(&mut px, width, height, gx, gy, m.wave_width() as f32 + 2.0 * pad, 2.0 * (max_half + pad), palette::EMERALD, (0.04 + 0.12 * mean) * opacity);
            for i in 0..WAVE_BARS {
                let lvl = bar(i);
                let half = min_half + (max_half - min_half) * lvl;
                let bx = content_x + i as f32 * (m.bar_w + m.bar_gap);
                fill_round_rect(&mut px, width, height, bx, cy - half, m.bar_w, half * 2.0, bar_color(lvl), opacity);
            }
        }
        PillKind::Done | PillKind::Error | PillKind::Attention | PillKind::Fixed => {
            let g = m.glyph as f32;
            let gx = content_x;
            let gy = cy - g / 2.0;
            match f.kind {
                PillKind::Done | PillKind::Fixed => {
                    draw_check(&mut px, width, height, gx, gy, g, palette::EMERALD_2, f.scale, f.tick, opacity)
                }
                PillKind::Attention => draw_alert(&mut px, width, height, gx, gy, g, palette::WARN, f.scale, opacity),
                _ => draw_alert(&mut px, width, height, gx, gy, g, palette::ROSE, f.scale, opacity),
            }
            if let Some(t) = f.label {
                let c = match f.kind {
                    PillKind::Done | PillKind::Fixed => palette::MINT,
                    PillKind::Attention => palette::AMBER_TEXT,
                    _ => palette::ROSE_TEXT,
                };
                let tx = content_x + g + m.gap as f32;
                blend_text(&mut px, width, height, &t, tx, cy, c, None, opacity);
            }
            if matches!(f.kind, PillKind::Fixed | PillKind::Error) {
                let label_w = f.label.map_or(0, |t| t.width);
                let texts = [f.secondary, f.tertiary];
                let widths = [texts[0].map_or(0, |t| t.width), texts[1].map_or(0, |t| t.width)];
                let ch = chip_height(f.scale);
                let (tint, ink) = if f.kind == PillKind::Fixed {
                    (palette::EMERALD, palette::MINT)
                } else {
                    (palette::ROSE, palette::ROSE_TEXT)
                };
                let mut spans = fixed_chips(f.scale, label_w, widths).into_iter();
                for t in texts.into_iter().flatten().filter(|t| t.width > 0) {
                    let Some((x0, x1)) = spans.next() else { break };
                    fill_hcapsule(&mut px, width, height, x0, cy - ch / 2.0, x1 - x0, ch, tint, 0.18 * opacity);
                    blend_text(&mut px, width, height, &t, x0 + (x1 - x0 - t.width as f32) / 2.0, cy, ink, None, opacity);
                }
            }
        }
        PillKind::Fixing => {
            if let Some(t) = f.label {
                blend_text(&mut px, width, height, &t, content_x, cy, palette::WORKING_TEXT, None, opacity);
            }
        }
        PillKind::Processing => {
            if let Some(t) = f.label {
                // Sheen sweeps left->right every 1.4 s; it is the only motion and stops with the state.
                let sweep = (f.phase / 1.4).fract();
                let band_x = content_x + (t.width as f32 + 40.0 * f.scale) * sweep - 20.0 * f.scale;
                blend_text(
                    &mut px,
                    width,
                    height,
                    &t,
                    content_x,
                    cy,
                    palette::WORKING_TEXT,
                    Some((band_x, 18.0 * f.scale)),
                    opacity,
                );
            }
        }
        PillKind::Notice => {
            if let Some(t) = f.label {
                blend_text(&mut px, width, height, &t, content_x, cy, palette::TEXT_2, None, opacity);
            }
        }
        PillKind::IdleHover => {
            let mut tx = content_x;
            if let Some(t) = f.label {
                blend_text(&mut px, width, height, &t, tx, cy, palette::TEXT, None, opacity);
                tx += (t.width + m.gap) as f32;
            }
            if let Some(t2) = f.secondary {
                blend_text(&mut px, width, height, &t2, tx, cy, palette::TEXT_3, None, opacity);
            }
        }
        _ => {}
    }
    // Content never paints over the border or outside the capsule, even while the width is still tweening
    // towards a wider state (labels and chips would otherwise hang past the right edge for a few frames).
    let inner_r = radius - border_w;
    for y in 0..height {
        let py = y as f32 + 0.5;
        for x in (right_cx.floor().max(0.0) as usize)..width {
            let pxf = x as f32 + 0.5;
            let d = ((pxf - right_cx).powi(2) + (py - cy).powi(2)).sqrt() - inner_r;
            let keep = (0.5 - d).clamp(0.0, 1.0);
            if keep < 1.0 {
                let i = y * width + x;
                px[i] = mix_px(base[i], px[i], keep);
            }
        }
    }
    px
}

/// Per-channel blend of two premultiplied pixels: `a` at `t = 0`, `b` at `t = 1`.
fn mix_px(a: u32, b: u32, t: f32) -> u32 {
    (0..4).fold(0, |out, k| {
        let (ca, cb) = (((a >> (k * 8)) & 0xFF) as f32, ((b >> (k * 8)) & 0xFF) as f32);
        out | ((lerp(ca, cb, t).round() as u32) << (k * 8))
    })
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn pack(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let a8 = (a * 255.0).round().clamp(0.0, 255.0);
    let p = |c: f32| ((c * a).round().clamp(0.0, a8)) as u32;
    ((a8 as u32) << 24) | (p(r) << 16) | (p(g) << 8) | p(b)
}

/// Source-over blend of a straight colour with coverage `cov` onto premultiplied `dst`.
#[inline]
fn over(dst: u32, c: (u8, u8, u8), cov: f32) -> u32 {
    if cov <= 0.0 {
        return dst;
    }
    let cov = cov.min(1.0);
    let inv = 1.0 - cov;
    let da = ((dst >> 24) & 0xFF) as f32;
    let dr = ((dst >> 16) & 0xFF) as f32;
    let dg = ((dst >> 8) & 0xFF) as f32;
    let db = (dst & 0xFF) as f32;
    let a = cov * 255.0 + da * inv;
    let r = c.0 as f32 * cov + dr * inv;
    let g = c.1 as f32 * cov + dg * inv;
    let b = c.2 as f32 * cov + db * inv;
    let q = |v: f32| v.round().clamp(0.0, 255.0) as u32;
    (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)
}

#[allow(clippy::too_many_arguments)]
fn blit_premul(dst: &mut [u32], w: usize, h: usize, src: &[u32], sw: usize, sh: usize, x0: isize, y0: isize, alpha: f32) {
    for sy in 0..sh {
        let dy = y0 + sy as isize;
        if dy < 0 || dy >= h as isize {
            continue;
        }
        for sx in 0..sw {
            let dx = x0 + sx as isize;
            if dx < 0 || dx >= w as isize {
                continue;
            }
            let s = src[sy * sw + sx];
            let sa = (((s >> 24) & 0xFF) as f32 * alpha) / 255.0;
            if sa <= 0.0 {
                continue;
            }
            let d = &mut dst[dy as usize * w + dx as usize];
            let inv = 1.0 - sa;
            let dv = *d;
            let ch = |shift: u32| -> u32 {
                let sv = ((s >> shift) & 0xFF) as f32 * alpha;
                let dc = ((dv >> shift) & 0xFF) as f32;
                (sv + dc * inv).round().clamp(0.0, 255.0) as u32
            };
            *d = (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_circle(px: &mut [u32], w: usize, h: usize, cx: f32, cy: f32, r: f32, c: (u8, u8, u8), op: f32) {
    let x0 = (cx - r - 1.0).floor().max(0.0) as usize;
    let x1 = ((cx + r + 1.0).ceil() as usize).min(w);
    let y0 = (cy - r - 1.0).floor().max(0.0) as usize;
    let y1 = ((cy + r + 1.0).ceil() as usize).min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0) * op;
            if cov > 0.0 && (px[y * w + x] >> 24) > 0 {
                px[y * w + x] = over(px[y * w + x], c, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_round_rect(px: &mut [u32], w: usize, h: usize, x: f32, y: f32, rw: f32, rh: f32, c: (u8, u8, u8), op: f32) {
    let r = rw / 2.0;
    let cx = x + r;
    let top = y + r;
    let bot = (y + rh - r).max(top);
    let x0 = x.floor().max(0.0) as usize;
    let x1 = ((x + rw).ceil() as usize).min(w);
    let y0 = y.floor().max(0.0) as usize;
    let y1 = ((y + rh).ceil() as usize).min(h);
    for yy in y0..y1 {
        let py = yy as f32 + 0.5;
        let ny = py.clamp(top, bot);
        for xx in x0..x1 {
            let d = ((xx as f32 + 0.5 - cx).powi(2) + (py - ny).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0) * op;
            if cov > 0.0 {
                px[yy * w + xx] = over(px[yy * w + xx], c, cov);
            }
        }
    }
}

/// Distance from point to segment.
fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (vx, vy) = (bx - ax, by - ay);
    let t = (((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy).max(1e-6)).clamp(0.0, 1.0);
    ((px - (ax + vx * t)).powi(2) + (py - (ay + vy * t)).powi(2)).sqrt()
}

#[allow(clippy::too_many_arguments)]
fn stroke_segments(px: &mut [u32], w: usize, h: usize, segs: &[(f32, f32, f32, f32)], half: f32, c: (u8, u8, u8), op: f32) {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(ax, ay, bx, by) in segs {
        minx = minx.min(ax.min(bx));
        miny = miny.min(ay.min(by));
        maxx = maxx.max(ax.max(bx));
        maxy = maxy.max(ay.max(by));
    }
    let x0 = (minx - half - 1.0).floor().max(0.0) as usize;
    let x1 = ((maxx + half + 1.0).ceil() as usize).min(w);
    let y0 = (miny - half - 1.0).floor().max(0.0) as usize;
    let y1 = ((maxy + half + 1.0).ceil() as usize).min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = segs
                .iter()
                .map(|&(ax, ay, bx, by)| seg_dist(fx, fy, ax, ay, bx, by))
                .fold(f32::MAX, f32::min);
            let cov = (half - d + 0.5).clamp(0.0, 1.0) * op;
            if cov > 0.0 {
                px[y * w + x] = over(px[y * w + x], c, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_check(px: &mut [u32], w: usize, h: usize, x: f32, y: f32, s: f32, c: (u8, u8, u8), scale: f32, progress: f32, op: f32) {
    let (a, b, c3) = ((x + s * 0.12, y + s * 0.55), (x + s * 0.40, y + s * 0.82), (x + s * 0.90, y + s * 0.22));
    // The tick draws itself: the first stroke takes the first third of the progress, the second the rest.
    let p = progress.clamp(0.0, 1.0);
    let first = (p * 3.0).min(1.0);
    let second = ((p - 1.0 / 3.0) * 1.5).clamp(0.0, 1.0);
    let at = |from: (f32, f32), to: (f32, f32), t: f32| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
    let mut segs = Vec::with_capacity(2);
    if first > 0.0 {
        let e = at(a, b, first);
        segs.push((a.0, a.1, e.0, e.1));
    }
    if second > 0.0 {
        let e = at(b, c3, second);
        segs.push((b.0, b.1, e.0, e.1));
    }
    if !segs.is_empty() {
        stroke_segments(px, w, h, &segs, 0.85 * scale.max(1.0), c, op);
    }
}

/// Anti-aliased horizontal capsule (radius = half the height) with straight colour `c` at `alpha`.
#[allow(clippy::too_many_arguments)]
fn fill_hcapsule(px: &mut [u32], w: usize, h: usize, x: f32, y: f32, cw: f32, ch: f32, c: (u8, u8, u8), alpha: f32) {
    let r = ch / 2.0;
    let (left, right, cy) = (x + r, (x + cw - r).max(x + r), y + r);
    for yy in (y.floor().max(0.0) as usize)..((y + ch).ceil() as usize).min(h) {
        for xx in (x.floor().max(0.0) as usize)..((x + cw).ceil() as usize).min(w) {
            let (fx, fy) = (xx as f32 + 0.5, yy as f32 + 0.5);
            let d = ((fx - fx.clamp(left, right)).powi(2) + (fy - cy).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0) * alpha;
            if cov > 0.0 {
                px[yy * w + xx] = over(px[yy * w + xx], c, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_alert(px: &mut [u32], w: usize, h: usize, x: f32, y: f32, s: f32, c: (u8, u8, u8), scale: f32, op: f32) {
    fill_circle(px, w, h, x + s / 2.0, y + s / 2.0, s / 2.0, c, op);
    let dark = palette::SURFACE;
    let segs = [(x + s / 2.0, y + s * 0.25, x + s / 2.0, y + s * 0.58)];
    stroke_segments(px, w, h, &segs, 0.8 * scale.max(1.0), dark, op);
    fill_circle(px, w, h, x + s / 2.0, y + s * 0.76, 0.95 * scale.max(1.0), dark, op);
}

/// Blends a coverage mask vertically centred on `cy`. Optional sheen `(centre_x, half_width)` brightens a band.
#[allow(clippy::too_many_arguments)]
fn blend_text(
    px: &mut [u32],
    w: usize,
    h: usize,
    t: &TextMask,
    x: f32,
    cy: f32,
    c: (u8, u8, u8),
    sheen: Option<(f32, f32)>,
    op: f32,
) {
    let x0 = x.round() as isize;
    let y0 = (cy - t.height as f32 / 2.0).round() as isize;
    for ty in 0..t.height {
        let dy = y0 + ty as isize;
        if dy < 0 || dy >= h as isize {
            continue;
        }
        for tx in 0..t.width {
            let dx = x0 + tx as isize;
            if dx < 0 || dx >= w as isize {
                continue;
            }
            let cov = t.coverage[ty * t.width + tx] as f32 / 255.0;
            if cov <= 0.0 {
                continue;
            }
            let mut col = c;
            let mut a = cov;
            if let Some((bx, bw)) = sheen {
                let k = (1.0 - ((dx as f32 - bx).abs() / bw)).clamp(0.0, 1.0);
                // Base text at 72%, the sheen lifts toward a cool white.
                a = cov * (0.72 + 0.28 * k);
                col = (
                    lerp(c.0 as f32, 236.0, 0.35 * k) as u8,
                    lerp(c.1 as f32, 253.0, 0.35 * k) as u8,
                    lerp(c.2 as f32, 245.0, 0.35 * k) as u8,
                );
            }
            let i = dy as usize * w + dx as usize;
            px[i] = over(px[i], col, a * op);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame<'a>(kind: PillKind, levels: &'a [f32], label: Option<TextMask<'a>>) -> PillFrame<'a> {
        PillFrame {
            scale: 1.0,
            kind,
            levels,
            phase: 0.3,
            label,
            secondary: None,
            tertiary: None,
            pop: 0.0,
            tick: 1.0,
            opacity: 1.0,
            content_opacity: 1.0,
        }
    }

    #[test]
    fn idle_is_a_compact_circle_much_smaller_than_v03() {
        let m = PillMetrics::for_scale(1.0);
        let w = pill_window_width(1.0, PillKind::Idle, 0, 0, 0);
        let visible = (w - 2 * m.margin) * m.cap_h;
        // v0.3 pill was 196 x 40 = 7840 logical px.
        assert_eq!(w - 2 * m.margin, 36);
        assert!(visible * 100 / 7840 < 20, "idle footprint {visible} px");
    }

    #[test]
    fn listening_footprint_is_at_least_25_percent_smaller_than_v03() {
        let m = PillMetrics::for_pill(1.0, PillKind::Listening);
        let w = pill_window_width(1.0, PillKind::Listening, 0, 0, 0) - 2 * m.margin;
        let area = w * m.cap_h;
        assert!(w <= 140, "listening width {w}");
        assert!(area * 100 / 7840 <= 75, "listening area {area}");
    }

    #[test]
    fn listening_pill_draws_at_sixty_percent_and_other_kinds_do_not_move() {
        // Window sizes before the change, then the listening sizes at the same DPI.
        for (scale, old, now) in [(1.0f32, (112usize, 44usize), (67usize, 26usize)), (1.5, (168, 66), (102, 40))] {
            let m = PillMetrics::for_pill(scale, PillKind::Listening);
            assert_eq!((pill_window_width(scale, PillKind::Listening, 0, 0, 0), m.window_height()), now);
            // 60 % exactly, up to the whole pixel that rounding the 1.5 dp bar pitch costs.
            assert!((now.0 as f32 / old.0 as f32 - LISTENING_SCALE).abs() < 0.02, "width {now:?} of {old:?} at {scale}");
            assert!((now.1 as f32 / old.1 as f32 - LISTENING_SCALE).abs() < 0.02, "height {now:?} of {old:?} at {scale}");
        }
        let m = PillMetrics::for_pill(1.0, PillKind::Listening);
        assert_eq!((m.cap_h, m.margin, m.icon), (22, 2, 13));
        // Every other state keeps the full-size metrics.
        assert_eq!(PillMetrics::for_pill(1.0, PillKind::Done).cap_h, 36);
        assert_eq!(pill_window_width(1.0, PillKind::Idle, 0, 0, 0), 44);
        assert_eq!(pill_window_width(1.0, PillKind::Error, 40, 0, 0), 117);
    }

    /// The bars are 1.5 dp wide with a 1.5 dp gap at 60 %: each still draws its own ink, none merge.
    #[test]
    fn listening_bars_stay_separate_at_sixty_percent() {
        for scale in [1.0f32, 1.5] {
            let m = PillMetrics::for_pill(scale, PillKind::Listening);
            let (w, h) = (pill_window_width(scale, PillKind::Listening, 0, 0, 0), m.window_height());
            let mut f = frame(PillKind::Listening, &[1.0; WAVE_BARS], None);
            f.scale = scale;
            let buf = render_pill(w, h, &f);
            // Each bar's slot must hold both ink and a gap, or the waveform has merged into a smear.
            let green = |x: usize| (buf[(h / 2) * w + x] >> 8) & 0xFF;
            let pitch = (m.bar_w + m.bar_gap).round() as usize;
            for i in 0..WAVE_BARS {
                let x0 = m.content_x + (i as f32 * (m.bar_w + m.bar_gap)).round() as usize;
                let slot = x0..(x0 + pitch).min(w);
                assert_eq!(slot.clone().count(), pitch, "bar {i} inside {w} x {h} at {scale}");
                assert!(slot.clone().map(green).max().unwrap() >= 0xC8, "bar {i} has no ink at {scale}");
                assert!(slot.map(green).min().unwrap() <= 0x40, "no gap after bar {i} at {scale}");
            }
        }
    }

    #[test]
    fn capsule_has_transparent_corners_and_opaque_body() {
        let w = pill_window_width(1.0, PillKind::Listening, 0, 0, 0);
        let h = PillMetrics::for_pill(1.0, PillKind::Listening).window_height();
        let buf = render_pill(w, h, &frame(PillKind::Listening, &[0.5; 11], None));
        assert_eq!(buf[0], 0, "corner must be transparent");
        assert!((buf[(h / 2) * w + w / 2] >> 24) > 200, "body must be opaque");
    }

    #[test]
    fn waveform_follows_measured_levels_not_a_canned_animation() {
        let w = pill_window_width(1.0, PillKind::Listening, 0, 0, 0);
        let h = PillMetrics::for_pill(1.0, PillKind::Listening).window_height();
        let silent = render_pill(w, h, &frame(PillKind::Listening, &[0.0; 11], None));
        let loud = render_pill(w, h, &frame(PillKind::Listening, &[0.9; 11], None));
        assert_ne!(silent, loud);
        // Same levels at a different animation phase must render identically (no fake motion).
        let mut f2 = frame(PillKind::Listening, &[0.4; 11], None);
        let a = render_pill(w, h, &f2);
        f2.phase = 9.7;
        assert_eq!(a, render_pill(w, h, &f2));
    }

    /// Feeds a constant level for `secs` seconds in steps of `dt`.
    fn run(sim: &mut WaveSim, db: f32, secs: f32, dt: f32) {
        let rms = 10f32.powf(db / 20.0);
        for _ in 0..(secs / dt).round() as usize {
            sim.step(rms, dt);
        }
    }

    #[test]
    fn wave_sim_silence_stays_at_the_floor() {
        let mut sim = WaveSim::new();
        for _ in 0..300 {
            sim.step(0.0, 1.0 / 60.0);
        }
        assert_eq!(sim.level, 0.0);
        assert_eq!(sim.bars().len(), WAVE_BARS);
        assert!(sim.bars().iter().all(|b| *b == 0.0));
    }

    #[test]
    fn wave_sim_burst_rises_fast_and_decays() {
        let mut sim = WaveSim::new();
        run(&mut sim, -55.0, 5.0, 1.0 / 60.0);
        run(&mut sim, -30.0, 0.1, 1.0 / 60.0);
        assert!(sim.level > 0.8, "level after 100 ms burst {}", sim.level);
        run(&mut sim, -120.0, 0.6, 1.0 / 60.0);
        assert!(sim.level < 0.2, "level 600 ms after the burst {}", sim.level);
    }

    #[test]
    fn wave_sim_adapts_to_a_constant_hum() {
        let mut sim = WaveSim::new();
        run(&mut sim, -20.0, 15.0, 1.0 / 60.0);
        assert!(sim.level < 0.1, "a constant hum must not read as speech, got {}", sim.level);
    }

    #[test]
    fn wave_sim_lights_up_for_a_quiet_speaker() {
        let mut sim = WaveSim::new();
        run(&mut sim, -60.0, 10.0, 1.0 / 60.0);
        run(&mut sim, -45.0, 0.3, 1.0 / 60.0);
        assert!(sim.level > 0.7, "quiet speech should still fill the bars, got {}", sim.level);
    }

    #[test]
    fn wave_sim_is_frame_rate_independent() {
        let (mut fast, mut slow) = (WaveSim::new(), WaveSim::new());
        run(&mut fast, -40.0, 1.0, 1.0 / 240.0);
        run(&mut slow, -40.0, 1.0, 1.0 / 20.0);
        assert!((fast.level - slow.level).abs() <= 0.03, "{} vs {}", fast.level, slow.level);
    }

    #[test]
    fn wave_sim_bars_are_square_rooted_and_restart_keeps_the_room() {
        let mut sim = WaveSim::new();
        sim.hist[WAVE_BARS - 1] = 0.25;
        assert_eq!(sim.bars()[WAVE_BARS - 1], 0.5);
        run(&mut sim, -40.0, 2.0, 1.0 / 60.0);
        let (peak, room) = (sim.peak, sim.room);
        sim.restart();
        assert!(sim.bars().iter().all(|b| *b == 0.0) && sim.level == 0.0);
        assert_eq!((sim.peak, sim.room), (peak, room));
    }

    #[test]
    fn bar_colour_blends_continuously_from_grey_to_emerald_to_mint() {
        assert_eq!(bar_color(0.0), palette::TEXT_3);
        assert_eq!(bar_color(0.14), palette::EMERALD_2);
        assert_ne!(bar_color(0.08), bar_color(0.0));
        assert_ne!(bar_color(0.08), bar_color(0.14));
        assert_ne!(bar_color(1.0), palette::EMERALD_2);
    }

    #[test]
    fn working_is_teal_and_distinct_from_attention() {
        let mask = vec![255u8; 20 * 10];
        let t = TextMask { width: 20, height: 10, coverage: &mask };
        let h = PillMetrics::for_scale(1.0).window_height();
        // A fixable failure gets one chip, so its pill is wider than a bare error pill.
        let plain = pill_window_width(1.0, PillKind::Error, 20, 0, 0);
        assert!(pill_window_width(1.0, PillKind::Error, 20, 60, 0) > plain);
        let w = pill_window_width(1.0, PillKind::Error, 20, 0, 0);
        let working = render_pill(w, h, &frame(PillKind::Processing, &[], Some(t)));
        let attention = render_pill(w, h, &frame(PillKind::Attention, &[], Some(t)));
        assert_ne!(working, attention);
        // The capsule border is teal: its blue channel is higher than the amber border's.
        let at = (h / 2) * w + w - PillMetrics::for_scale(1.0).margin - 1;
        assert!((working[at] & 0xFF) > (attention[at] & 0xFF));
    }

    #[test]
    fn states_render_distinctly() {
        let mask = vec![255u8; 20 * 10];
        let t = TextMask { width: 20, height: 10, coverage: &mask };
        let h = PillMetrics::for_scale(1.0).window_height();
        let w = pill_window_width(1.0, PillKind::Error, 20, 0, 0);
        let done = render_pill(w, h, &frame(PillKind::Done, &[], Some(t)));
        let err = render_pill(w, h, &frame(PillKind::Error, &[], Some(t)));
        let proc_s = render_pill(w, h, &frame(PillKind::Processing, &[], Some(t)));
        assert_ne!(done, err);
        assert_ne!(done, proc_s);
        let notice = render_pill(w, h, &frame(PillKind::Notice, &[], Some(t)));
        let attention = render_pill(w, h, &frame(PillKind::Attention, &[], Some(t)));
        assert_ne!(attention, done);
        assert_ne!(attention, err);
        assert_ne!(notice, done);
        assert_ne!(notice, err);
        let wi = pill_window_width(1.0, PillKind::Idle, 0, 0, 0);
        assert_ne!(
            render_pill(wi, h, &frame(PillKind::Idle, &[], None)),
            render_pill(wi, h, &frame(PillKind::Disabled, &[], None))
        );
    }

    #[test]
    fn scales_with_dpi() {
        let h100 = PillMetrics::for_scale(1.0).window_height();
        let h200 = PillMetrics::for_scale(2.0).window_height();
        assert_eq!(h200, h100 * 2);
    }

    #[test]
    fn icon_downscale_keeps_mark_opaque_at_centre() {
        let ic = scaled_icon(22);
        assert!((ic[11 * 22 + 11] >> 24) > 200);
        assert_eq!(ic[0] >> 24, 0);
    }

    #[test]
    fn level_mapping_is_monotonic_and_bounded() {
        assert_eq!(rms_to_level(0.0), 0.0);
        assert!(rms_to_level(0.01) < rms_to_level(0.1));
        assert_eq!(rms_to_level(1.0), 1.0);
    }

    #[test]
    fn motion_curves_start_neutral_and_come_to_rest() {
        use motion::*;
        assert_eq!((pop(0.0), tick(0.0), count_up(0.0, 4), nod(0.0), shake(0.0)), (0.0, 0.0, 0, 0.0, 0.0));
        assert!(pop(TICK_MS / 2.0) > 0.99);
        assert_eq!((pop(TICK_MS), tick(TICK_MS), count_up(180.0, 4), nod(160.0), shake(200.0)), (0.0, 1.0, 4, 0.0, 0.0));
        assert_eq!((tick_unwind(0.0), tick_unwind(TICK_MS), count_down(0.0, 4), count_down(180.0, 4)), (1.0, 0.0, 4, 0));
        // Reduced motion passes infinity and gets the resting values.
        let rest = f32::INFINITY;
        assert_eq!((pop(rest), tick(rest), count_up(rest, 7), nod(rest), shake(rest)), (0.0, 1.0, 7, 0.0, 0.0));
        // The shake stays within 6 px and really moves both ways.
        let xs: Vec<f32> = (0..200).map(|i| shake(i as f32)).collect();
        assert!(xs.iter().all(|x| x.abs() <= 6.0) && xs.iter().any(|x| *x > 1.0) && xs.iter().any(|x| *x < -1.0));
    }

    #[test]
    fn fixed_pill_has_chips_inside_the_window_and_the_tick_draws_itself() {
        let (label_w, undo_w, deeper_w) = (92, 28, 40);
        let w = pill_window_width(1.0, PillKind::Fixed, label_w, undo_w, deeper_w);
        let chips = fixed_chips(1.0, label_w, [undo_w, deeper_w]);
        assert_eq!(chips.len(), 2);
        assert!(chips[0].1 < chips[1].0, "chips must not overlap");
        assert!(chips[1].1 < w as f32 - PillMetrics::for_scale(1.0).margin as f32, "chips stay inside the capsule");
        assert_eq!(fixed_chips(1.0, label_w, [undo_w, 0]).len(), 1);
        let h = PillMetrics::for_scale(1.0).window_height();
        let mask = vec![255u8; 100 * 10];
        let t = |n: usize| TextMask { width: n, height: 10, coverage: &mask[..n * 10] };
        let mut f = frame(PillKind::Fixed, &[], Some(t(label_w)));
        f.secondary = Some(t(undo_w));
        f.tertiary = Some(t(deeper_w));
        let full = render_pill(w, h, &f);
        f.tick = 0.0;
        let none = render_pill(w, h, &f);
        f.tick = 0.4;
        let part = render_pill(w, h, &f);
        assert!(full != none && part != none && part != full, "tick progress must change the pixels");
        f.tick = 1.0;
        f.pop = 1.0;
        assert_ne!(render_pill(w, h, &f), full, "the pop must change the capsule");
        // Mid-tween the window is narrower than the content: nothing may paint over the border or outside.
        f.pop = 0.0;
        let narrow = w / 2;
        let with = render_pill(narrow, h, &f);
        let without = render_pill(narrow, h, &frame(PillKind::Fixed, &[], None));
        let m = PillMetrics::for_scale(1.0);
        for y in 0..h {
            for x in narrow - m.margin - 1..narrow {
                assert_eq!(with[y * narrow + x], without[y * narrow + x], "content leaked at ({x}, {y})");
            }
        }
    }

    #[test]
    fn fixing_pill_sweeps_one_band_that_moves_with_the_phase() {
        let mask = vec![255u8; 40 * 10];
        let t = TextMask { width: 40, height: 10, coverage: &mask };
        let w = pill_window_width(1.0, PillKind::Fixing, 40, 0, 0);
        let h = PillMetrics::for_scale(1.0).window_height();
        let mut f = frame(PillKind::Fixing, &[], Some(t));
        f.phase = 0.0;
        let a = render_pill(w, h, &f);
        f.phase = 0.3;
        let b = render_pill(w, h, &f);
        f.phase = 0.6; // one full sweep later: back where it began
        let c = render_pill(w, h, &f);
        assert_ne!(a, b);
        assert_eq!(a, c);
    }
}
