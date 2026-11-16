//! Floating listener renderer (pure software, premultiplied BGRA, unit-testable).
//!
//! Design (v0.4): an icon-first, compact capsule that only grows when it has something to say.
//!
//! | State        | Content                                          | Logical size @100% |
//! |--------------|--------------------------------------------------|--------------------|
//! | Idle         | KeyReed mark only                                | 36 x 36            |
//! | Idle + hover | mark + writing mode + shortcut                   | text-sized         |
//! | Listening    | mark + 11-bar waveform of *measured* mic levels  | 98 x 36            |
//! | Processing   | mark + "Transcribing" with a moving sheen        | text-sized         |
//! | Done         | mark + check + "Inserted" / "Copied"             | text-sized         |
//! | Error        | mark + alert + short plain-language reason       | text-sized         |
//!
//! The window itself is `WS_EX_LAYERED | WS_EX_NOACTIVATE`; this module only produces pixels.
//! Text is rasterised by the caller (GDI, grayscale AA) and passed in as a coverage mask so the
//! final alpha is exact (GDI cannot write alpha into layered windows).

use crate::icon_data::{KEYREED_ICON_64, KEYREED_ICON_SIZE};

/// Logical (96-DPI) capsule height. Every other dimension derives from it.
pub const PILL_HEIGHT: f32 = 36.0;
/// Transparent margin around the capsule that holds the soft shadow.
pub const PILL_MARGIN: f32 = 4.0;
/// Number of waveform bars while listening.
pub const WAVE_BARS: usize = 11;

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
    /// Slightly lifted circle behind the mark so the keycap's dark body keeps its silhouette.
    pub const WELL: (u8, u8, u8) = (0x1B, 0x29, 0x23);
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
    /// Measured input levels, oldest first, each in `0.0..=1.0`. Only used while listening.
    pub levels: &'a [f32],
    /// Seconds since the state began; drives the processing sheen only.
    pub phase: f32,
    /// Primary label (mode name, status word, error reason).
    pub label: Option<TextMask<'a>>,
    /// Optional secondary label shown after the primary one in a dimmer colour (e.g. shortcut).
    pub secondary: Option<TextMask<'a>>,
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

    fn wave_width(&self) -> usize {
        (WAVE_BARS as f32 * self.bar_w + (WAVE_BARS as f32 - 1.0) * self.bar_gap).ceil() as usize
    }
}

/// Width of the window (including shadow margin) required for `kind` with the given label widths.
pub fn pill_window_width(scale: f32, kind: PillKind, label_w: usize, secondary_w: usize) -> usize {
    let m = PillMetrics::for_scale(scale);
    let content = match kind {
        PillKind::Idle | PillKind::Disabled => return m.cap_h + 2 * m.margin,
        PillKind::Listening => m.wave_width(),
        PillKind::Done | PillKind::Error | PillKind::Attention => m.glyph + m.gap + label_w,
        PillKind::IdleHover => label_w + if secondary_w > 0 { m.gap + secondary_w } else { 0 },
        PillKind::Processing | PillKind::Notice => label_w,
    };
    m.content_x + content + m.pad_right + m.margin
}

/// Maps an RMS value (linear, 0..1) to a perceptual 0..1 level using a -55 dBFS .. -15 dBFS window.
pub fn rms_to_level(rms: f32) -> f32 {
    if !rms.is_finite() || rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 55.0) / 40.0).clamp(0.0, 1.0)
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
    let m = PillMetrics::for_scale(f.scale);
    let margin = m.margin as f32;
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
        PillKind::Processing => (palette::WARN, 0.55, palette::SURFACE),
        PillKind::Done => (palette::EMERALD_2, 0.70, palette::SURFACE),
        PillKind::Notice => (palette::TEXT_2, 0.35, palette::SURFACE),
        PillKind::Attention => (palette::WARN, 0.75, palette::SURFACE),
        PillKind::Error => (palette::ROSE, 0.80, palette::SURFACE),
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
                col = (r, g, b, body_alpha * cov);
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
    let icon = scaled_icon(icon_n);
    let icon_alpha = if f.kind == PillKind::Disabled { 0.45 } else { 1.0 } * opacity;
    fill_circle(&mut px, width, height, left_cx, cy, radius - 3.0 * f.scale, palette::WELL, opacity);
    let ix0 = (left_cx - icon_n as f32 / 2.0).round() as isize;
    let iy0 = (cy - icon_n as f32 / 2.0).round() as isize;
    blit_premul(&mut px, width, height, &icon, icon_n, icon_n, ix0, iy0, icon_alpha);


    if matches!(f.kind, PillKind::Idle | PillKind::Disabled) {
        return px;
    }

    let content_x = m.content_x as f32;
    let opacity = opacity * f.content_opacity.clamp(0.0, 1.0);
    match f.kind {
        PillKind::Listening => {
            let max_half = cap_h * 0.30;
            let min_half = 1.2 * f.scale;
            let n = f.levels.len();
            for i in 0..WAVE_BARS {
                // Newest sample on the right; missing history renders as the floor.
                let lvl = if n >= WAVE_BARS - i { f.levels[n - (WAVE_BARS - i)] } else { 0.0 };
                let half = min_half + (max_half - min_half) * lvl.clamp(0.0, 1.0);
                let bx = content_x + i as f32 * (m.bar_w + m.bar_gap);
                let c = if lvl > 0.04 { palette::EMERALD_2 } else { palette::TEXT_3 };
                fill_round_rect(&mut px, width, height, bx, cy - half, m.bar_w, half * 2.0, c, opacity);
            }
        }
        PillKind::Done | PillKind::Error | PillKind::Attention => {
            let g = m.glyph as f32;
            let gx = content_x;
            let gy = cy - g / 2.0;
            match f.kind {
                PillKind::Done => draw_check(&mut px, width, height, gx, gy, g, palette::EMERALD_2, f.scale, opacity),
                PillKind::Attention => draw_alert(&mut px, width, height, gx, gy, g, palette::WARN, f.scale, opacity),
                _ => draw_alert(&mut px, width, height, gx, gy, g, palette::ROSE, f.scale, opacity),
            }
            if let Some(t) = f.label {
                let c = match f.kind {
                    PillKind::Done => palette::MINT,
                    PillKind::Attention => palette::AMBER_TEXT,
                    _ => palette::ROSE_TEXT,
                };
                let tx = content_x + g + m.gap as f32;
                blend_text(&mut px, width, height, &t, tx, cy, c, None, opacity);
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
                    palette::WARN,
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
    px
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
fn draw_check(px: &mut [u32], w: usize, h: usize, x: f32, y: f32, s: f32, c: (u8, u8, u8), scale: f32, op: f32) {
    let segs = [
        (x + s * 0.12, y + s * 0.55, x + s * 0.40, y + s * 0.82),
        (x + s * 0.40, y + s * 0.82, x + s * 0.90, y + s * 0.22),
    ];
    stroke_segments(px, w, h, &segs, 0.85 * scale.max(1.0), c, op);
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
                // Base text at 72%, the sheen lifts toward warm white.
                a = cov * (0.72 + 0.28 * k);
                col = (
                    lerp(c.0 as f32, 255.0, 0.35 * k) as u8,
                    lerp(c.1 as f32, 245.0, 0.35 * k) as u8,
                    lerp(c.2 as f32, 225.0, 0.35 * k) as u8,
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
        PillFrame { scale: 1.0, kind, levels, phase: 0.3, label, secondary: None, opacity: 1.0, content_opacity: 1.0 }
    }

    #[test]
    fn idle_is_a_compact_circle_much_smaller_than_v03() {
        let m = PillMetrics::for_scale(1.0);
        let w = pill_window_width(1.0, PillKind::Idle, 0, 0);
        let visible = (w - 2 * m.margin) * m.cap_h;
        // v0.3 pill was 196 x 40 = 7840 logical px.
        assert_eq!(w - 2 * m.margin, 36);
        assert!(visible * 100 / 7840 < 20, "idle footprint {visible} px");
    }

    #[test]
    fn listening_footprint_is_at_least_25_percent_smaller_than_v03() {
        let m = PillMetrics::for_scale(1.0);
        let w = pill_window_width(1.0, PillKind::Listening, 0, 0) - 2 * m.margin;
        let area = w * m.cap_h;
        assert!(w <= 140, "listening width {w}");
        assert!(area * 100 / 7840 <= 75, "listening area {area}");
    }

    #[test]
    fn capsule_has_transparent_corners_and_opaque_body() {
        let w = pill_window_width(1.0, PillKind::Listening, 0, 0);
        let h = PillMetrics::for_scale(1.0).window_height();
        let buf = render_pill(w, h, &frame(PillKind::Listening, &[0.5; 11], None));
        assert_eq!(buf[0], 0, "corner must be transparent");
        assert!((buf[(h / 2) * w + w / 2] >> 24) > 200, "body must be opaque");
    }

    #[test]
    fn waveform_follows_measured_levels_not_a_canned_animation() {
        let w = pill_window_width(1.0, PillKind::Listening, 0, 0);
        let h = PillMetrics::for_scale(1.0).window_height();
        let silent = render_pill(w, h, &frame(PillKind::Listening, &[0.0; 11], None));
        let loud = render_pill(w, h, &frame(PillKind::Listening, &[0.9; 11], None));
        assert_ne!(silent, loud);
        // Same levels at a different animation phase must render identically (no fake motion).
        let mut f2 = frame(PillKind::Listening, &[0.4; 11], None);
        let a = render_pill(w, h, &f2);
        f2.phase = 9.7;
        assert_eq!(a, render_pill(w, h, &f2));
    }

    #[test]
    fn states_render_distinctly() {
        let mask = vec![255u8; 20 * 10];
        let t = TextMask { width: 20, height: 10, coverage: &mask };
        let h = PillMetrics::for_scale(1.0).window_height();
        let w = pill_window_width(1.0, PillKind::Error, 20, 0);
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
        let wi = pill_window_width(1.0, PillKind::Idle, 0, 0);
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
}
