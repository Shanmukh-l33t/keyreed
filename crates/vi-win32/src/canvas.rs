//! Tiny immediate-mode canvas for the Settings window: anti-aliased shapes into a 32-bit DIB,
//! ClearType text via GDI on top, then one BitBlt. Opaque backgrounds only (no alpha output).

#![cfg(windows)]

use windows_sys::Win32::Foundation::{RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, DrawTextW, GetTextExtentPoint32W,
    IntersectClipRect, SelectClipRgn, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HDC, HFONT, HGDIOBJ, SRCCOPY, TRANSPARENT,
};

pub const DT_LEFT: u32 = 0x0;
pub const DT_CENTER: u32 = 0x1;
pub const DT_RIGHT: u32 = 0x2;
pub const DT_VCENTER: u32 = 0x4;
pub const DT_WORDBREAK: u32 = 0x10;
pub const DT_SINGLELINE: u32 = 0x20;
pub const DT_CALCRECT: u32 = 0x400;
pub const DT_NOPREFIX: u32 = 0x800;
pub const DT_END_ELLIPSIS: u32 = 0x8000;

pub use crate::tokens::rgb;

#[inline]
fn colorref_to_px(c: u32) -> u32 {
    ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    /// (x0, y0, x1, y1) in pixels: drawing outside it is dropped, text included (see `set_clip`).
    clip: (i32, i32, i32, i32),
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}

impl Canvas {
    /// # Safety
    /// Must be called on a thread that may use GDI. The canvas owns its DC and bitmap.
    pub unsafe fn new(w: i32, h: i32) -> Option<Canvas> {
        let w = w.max(1);
        let h = h.max(1);
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bmp as _);
        SetBkMode(dc, TRANSPARENT as i32);
        Some(Canvas { w, h, clip: (0, 0, w, h), dc, bmp, old, bits: bits as *mut u32 })
    }

    /// Narrows every later draw (GDI text too) to `x, y, w, h`; `clear_clip` restores the whole canvas.
    pub fn set_clip(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let x0 = x.clamp(0, self.w);
        let y0 = y.clamp(0, self.h);
        // A layout can hand over an empty or inverted box (a window narrower than the sidebar while it is being
        // sized, a clipped pill). Normalise it: `clamp` panics when minimum > maximum, which aborted the app.
        let x1 = x.saturating_add(w.max(0)).clamp(0, self.w).max(x0);
        let y1 = y.saturating_add(h.max(0)).clamp(0, self.h).max(y0);
        self.clip = (x0, y0, x1, y1);
        unsafe {
            IntersectClipRect(self.dc, x0, y0, x1, y1);
        }
    }

    pub fn clear_clip(&mut self) {
        self.clip = (0, 0, self.w, self.h);
        unsafe {
            SelectClipRgn(self.dc, std::ptr::null_mut());
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    fn px(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }

    pub fn fill(&mut self, c: u32) {
        let p = colorref_to_px(c);
        self.px().fill(p);
    }

    /// Pixel-aligned solid rectangle.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        let p = colorref_to_px(c);
        let (cw, clip) = (self.w, self.clip);
        if clip.2 <= clip.0 || clip.3 <= clip.1 {
            return;
        }
        let x0 = x.clamp(clip.0, clip.2);
        let x1 = x.saturating_add(w.max(0)).clamp(clip.0, clip.2).max(x0);
        let y0 = y.clamp(clip.1, clip.3);
        let y1 = y.saturating_add(h.max(0)).clamp(clip.1, clip.3).max(y0);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let buf = self.px();
        for yy in y0..y1 {
            let row = (yy * cw) as usize;
            buf[row + x0 as usize..row + x1 as usize].fill(p);
        }
    }

    #[inline]
    fn blend(&mut self, idx: usize, c: u32, a: f32) {
        if a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let buf = self.px();
        let d = buf[idx];
        let mix = |s: u32, d: u32| ((s as f32) * a + (d as f32) * (1.0 - a)).round() as u32;
        let (sr, sg, sb) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF);
        let (dr, dg, db) = ((d >> 16) & 0xFF, (d >> 8) & 0xFF, d & 0xFF);
        buf[idx] = (mix(sr, dr) << 16) | (mix(sg, dg) << 8) | mix(sb, db);
    }

    /// Signed distance to a rounded rectangle (negative inside).
    fn sd_round_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        let cx = x + w / 2.0;
        let cy = y + h / 2.0;
        let qx = (px - cx).abs() - (w / 2.0 - r);
        let qy = (py - cy).abs() - (h / 2.0 - r);
        let ox = qx.max(0.0);
        let oy = qy.max(0.0);
        (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
    }

    /// Anti-aliased filled rounded rectangle (`alpha` 0..1).
    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: u32, alpha: f32) {
        let x0 = (x - 1.0).floor().max(self.clip.0 as f32) as i32;
        let y0 = (y - 1.0).floor().max(self.clip.1 as f32) as i32;
        let x1 = ((x + w + 1.0).ceil() as i32).min(self.clip.2);
        let y1 = ((y + h + 1.0).ceil() as i32).min(self.clip.3);
        let (ix0, ix1) = (x.ceil() as i32, (x + w).floor() as i32);
        let re = r.min(w / 2.0).min(h / 2.0).max(0.0);
        for yy in y0..y1 {
            // Rows clear of the corners are fully covered between the straight edges: no distance field there.
            let (a, b) = if yy as f32 >= y + re && (yy + 1) as f32 <= y + h - re { { let a = ix0.max(x0).min(x1); (a, ix1.min(x1).max(a)) } } else { (x1, x1) };
            for xx in (x0..a).chain(b..x1) {
                let d = Self::sd_round_rect(xx as f32 + 0.5, yy as f32 + 0.5, x, y, w, h, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend((yy * self.w + xx) as usize, c, cov * alpha);
                }
            }
            self.blend_span(yy, a, b, c, alpha);
        }
    }

    /// Blends colour `c` at `alpha` over pixels `x0..x1` of row `y` (already inside the clip).
    fn blend_span(&mut self, y: i32, x0: i32, x1: i32, c: u32, alpha: f32) {
        if x1 <= x0 || alpha <= 0.0 {
            return;
        }
        let row = (y * self.w) as usize;
        let p = ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF);
        let buf = &mut self.px()[row + x0 as usize..row + x1 as usize];
        if alpha >= 1.0 {
            buf.fill(p);
            return;
        }
        let a = (alpha * 256.0) as u32;
        for d in buf {
            let mix = |sh: u32| ((((p >> sh) & 0xFF) * a + ((*d >> sh) & 0xFF) * (256 - a)) >> 8) << sh;
            *d = mix(16) | mix(8) | mix(0);
        }
    }

    /// Anti-aliased rounded-rectangle outline of width `t`, drawn inside the rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn round_rect_stroke(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, t: f32, c: u32, alpha: f32) {
        let x0 = (x - 1.0).floor().max(self.clip.0 as f32) as i32;
        let y0 = (y - 1.0).floor().max(self.clip.1 as f32) as i32;
        let x1 = ((x + w + 1.0).ceil() as i32).min(self.clip.2);
        let y1 = ((y + h + 1.0).ceil() as i32).min(self.clip.3);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let d = Self::sd_round_rect(xx as f32 + 0.5, yy as f32 + 0.5, x, y, w, h, r);
                let outer = (0.5 - d).clamp(0.0, 1.0);
                let inner = (0.5 - (d + t)).clamp(0.0, 1.0);
                let cov = outer - inner;
                if cov > 0.0 {
                    self.blend((yy * self.w + xx) as usize, c, cov * alpha);
                }
            }
        }
    }

    /// Soft drop shadow of a rounded rectangle from CSS-style layers `(y offset, blur, spread, alpha)` in colour `c`.
    /// Paints only outside the rectangle itself (it is about to be covered), so a big panel costs a thin band.
    #[allow(clippy::too_many_arguments)]
    pub fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, layers: &[(f32, f32, f32, f32)], scale: f32, c: u32) {
        for &(dy, blur, spread, a) in layers {
            let (dy, blur, spread) = (dy * scale, (blur * scale).max(1.0), spread * scale);
            let (sx, sy, sw, sh) = (x - spread, y + dy - spread, (w + 2.0 * spread).max(1.0), (h + 2.0 * spread).max(1.0));
            let sr = (r + spread).max(0.0);
            // A Gaussian edge spans about +-0.75 blur; beyond that the layer adds nothing.
            let reach = 0.75 * blur;
            let x0 = (sx - reach).floor().max(self.clip.0 as f32) as i32;
            let y0 = (sy - reach).floor().max(self.clip.1 as f32) as i32;
            let x1 = ((sx + sw + reach).ceil() as i32).min(self.clip.2);
            let y1 = ((sy + sh + reach).ceil() as i32).min(self.clip.3);
            let re = r.min(w / 2.0).min(h / 2.0).max(0.0);
            for yy in y0..y1 {
                // Rows clear of the shape's corners skip its interior outright (it is about to be covered).
                let (sa, sb) = if yy as f32 >= y + re && (yy + 1) as f32 <= y + h - re { (x.ceil() as i32 + 1, (x + w).floor() as i32 - 1) } else { (x1, x1) };
                // max/min, not clamp: a shadow clipped away entirely has x1 < x0, and clamp panics on that.
                let left_end = sa.max(x0).min(x1);
                for xx in (x0..left_end).chain(sb.max(left_end).min(x1)..x1) {
                    let (px, py) = (xx as f32 + 0.5, yy as f32 + 0.5);
                    if Self::sd_round_rect(px, py, x, y, w, h, r) < -0.5 {
                        continue;
                    }
                    let d = Self::sd_round_rect(px, py, sx, sy, sw, sh, sr);
                    let t = ((d + reach) / (2.0 * reach)).clamp(0.0, 1.0);
                    let k = 1.0 - t * t * (3.0 - 2.0 * t);
                    if k > 0.0 {
                        self.blend((yy * self.w + xx) as usize, c, a * k);
                    }
                }
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: u32, alpha: f32) {
        self.round_rect(cx - r, cy - r, r * 2.0, r * 2.0, r, c, alpha);
    }

    /// Anti-aliased polyline stroke (used for check marks).
    pub fn stroke(&mut self, pts: &[(f32, f32)], half: f32, c: u32) {
        if pts.len() < 2 {
            return;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        let x0 = (minx - half - 1.0).floor().max(self.clip.0 as f32) as i32;
        let y0 = (miny - half - 1.0).floor().max(self.clip.1 as f32) as i32;
        let x1 = ((maxx + half + 1.0).ceil() as i32).min(self.clip.2);
        let y1 = ((maxy + half + 1.0).ceil() as i32).min(self.clip.3);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let (px, py) = (xx as f32 + 0.5, yy as f32 + 0.5);
                let mut d = f32::MAX;
                for s in pts.windows(2) {
                    let (ax, ay) = s[0];
                    let (bx, by) = s[1];
                    let (vx, vy) = (bx - ax, by - ay);
                    let t = (((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy).max(1e-6)).clamp(0.0, 1.0);
                    d = d.min(((px - ax - vx * t).powi(2) + (py - ay - vy * t).powi(2)).sqrt());
                }
                let cov = (half - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend((yy * self.w + xx) as usize, c, cov);
                }
            }
        }
    }

    /// Copies `src` (opaque) with its top-left at `(x, y)`, inside the current clip.
    pub fn draw_canvas(&mut self, src: &Canvas, x: i32, y: i32) {
        let clip = self.clip;
        let (x0, x1) = (x.max(clip.0), (x + src.w).min(clip.2));
        if x1 <= x0 {
            return;
        }
        let (sw, dw) = (src.w as usize, self.w as usize);
        let s = unsafe { std::slice::from_raw_parts(src.bits, (src.w * src.h) as usize) };
        let d = self.px();
        for yy in y.max(clip.1)..(y + src.h).min(clip.3) {
            let (sr, dr) = ((yy - y) as usize * sw, yy as usize * dw);
            d[dr + x0 as usize..dr + x1 as usize].copy_from_slice(&s[sr + (x0 - x) as usize..sr + (x1 - x) as usize]);
        }
    }

    /// Blends `src` (same size) over this canvas inside `(x0, y0, x1, y1)`, row by row: destination row `y` takes
    /// source row `y - dy` shifted right by `dx`, at opacity `op`, where `(op, dx, dy) = at(y)`. Page entrances use it to
    /// fade and slide a fully drawn page in without drawing it again each frame.
    pub fn blend_rows_from(&mut self, src: &Canvas, area: (i32, i32, i32, i32), at: impl Fn(i32) -> (f32, i32, i32)) {
        if src.w != self.w || src.h != self.h {
            return;
        }
        let w = self.w as usize;
        let s = unsafe { std::slice::from_raw_parts(src.bits, (src.w * src.h) as usize) };
        let (x0, y0, x1, y1) = (area.0.max(0), area.1.max(0), area.2.min(self.w), area.3.min(self.h));
        let clip = self.clip;
        let d = self.px();
        for y in y0.max(clip.1)..y1.min(clip.3) {
            let (op, dx, dy) = at(y);
            let sy = y - dy;
            if op <= 0.0 || sy < y0 || sy >= y1 {
                continue;
            }
            let a = (op.min(1.0) * 256.0) as u32;
            for x in x0.max(clip.0)..x1.min(clip.2) {
                let sx = x - dx;
                if sx < x0 || sx >= x1 {
                    continue;
                }
                let (sp, dp) = (s[sy as usize * w + sx as usize], &mut d[y as usize * w + x as usize]);
                let mix = |sh: u32| ((((sp >> sh) & 0xFF) * a + ((*dp >> sh) & 0xFF) * (256 - a)) >> 8) << sh;
                *dp = mix(16) | mix(8) | mix(0);
            }
        }
    }

    /// The colour at `(x, y)` as a COLORREF (what a fading text blends toward).
    pub fn sample(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        let p = unsafe { *self.bits.add((y * self.w + x) as usize) };
        ((p >> 16) & 0xFF) | (p & 0xFF00) | ((p & 0xFF) << 16)
    }

    /// Blends a premultiplied BGRA image (the listener's own renderer output) over this canvas at `(x, y)`.
    pub fn blend_premul(&mut self, src: &[u32], sw: i32, sh: i32, x: i32, y: i32) {
        let clip = self.clip;
        let w = self.w as usize;
        let d = self.px();
        for yy in y.max(clip.1)..(y + sh).min(clip.3) {
            for xx in x.max(clip.0)..(x + sw).min(clip.2) {
                let p = src[((yy - y) * sw + (xx - x)) as usize];
                let a = p >> 24;
                if a == 0 {
                    continue;
                }
                let dp = &mut d[yy as usize * w + xx as usize];
                let ch = |sh: u32| (((p >> sh) & 0xFF) + ((((*dp >> sh) & 0xFF) * (255 - a)) / 255)).min(255) << sh;
                *dp = ch(16) | ch(8) | ch(0);
            }
        }
    }

    /// Draws text with GDI (ClearType on the opaque background). Returns the height used.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, s: &str, x: i32, y: i32, w: i32, h: i32, font: HFONT, color: u32, flags: u32) -> i32 {
        if s.is_empty() {
            return 0;
        }
        unsafe {
            let old = SelectObject(self.dc, font as _);
            SetTextColor(self.dc, color);
            let mut wide: Vec<u16> = s.encode_utf16().collect();
            let mut rc = RECT { left: x, top: y, right: x + w, bottom: y + h };
            let used = DrawTextW(self.dc, wide.as_mut_ptr(), wide.len() as i32, &mut rc, flags | DT_NOPREFIX);
            SelectObject(self.dc, old);
            used
        }
    }

    /// The canvas size in pixels.
    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    /// The current clip `(x0, y0, x1, y1)`, to hand back to `restore_clip` after a narrower one.
    pub fn clip(&self) -> (i32, i32, i32, i32) {
        self.clip
    }

    pub fn restore_clip(&mut self, c: (i32, i32, i32, i32)) {
        self.clip = c;
    }

    /// Narrows the current clip further to `x, y, w, h`.
    pub fn narrow_clip(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let c = self.clip;
        let (x0, y0) = (x.max(c.0), y.max(c.1));
        self.clip = (x0, y0, (x + w).min(c.2).max(x0), (y + h).min(c.3).max(y0));
    }

    /// Copies only the clip rectangle to `hdc` (a frame that changed a small part repaints a small part).
    pub fn blit_clip_to(&self, hdc: HDC) {
        let (x0, y0, x1, y1) = self.clip;
        if x1 > x0 && y1 > y0 {
            unsafe {
                BitBlt(hdc, x0, y0, x1 - x0, y1 - y0, self.dc, x0, y0, SRCCOPY);
            }
        }
    }

    pub fn blit_to(&self, hdc: HDC, x: i32, y: i32) {
        unsafe {
            BitBlt(hdc, x, y, self.w, self.h, self.dc, 0, 0, SRCCOPY);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as _);
            DeleteDC(self.dc);
        }
    }
}

/// Measures single-line text width in pixels.
pub fn text_width(s: &str, font: HFONT) -> i32 {
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let old = SelectObject(dc, font as _);
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(dc, w.as_ptr(), w.len() as i32, &mut sz);
        SelectObject(dc, old);
        DeleteDC(dc);
        sz.cx
    }
}

/// Measures the height of word-wrapped text within `width`.
pub fn text_height(s: &str, font: HFONT, width: i32) -> i32 {
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let old = SelectObject(dc, font as _);
        let mut w: Vec<u16> = s.encode_utf16().collect();
        let mut rc = RECT { left: 0, top: 0, right: width, bottom: 0 };
        DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut rc, DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX);
        SelectObject(dc, old);
        DeleteDC(dc);
        rc.bottom - rc.top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(cv: &Canvas, x: i32, y: i32) -> u32 {
        unsafe { *cv.bits.add((y * cv.w + x) as usize) }
    }

    /// The Settings window is painted while it is still being sized; a box narrower than the sidebar
    /// produced an inverted clip and `clamp` aborted the process (seen in the wild, min > max).
    #[test]
    fn an_inverted_or_empty_clip_draws_nothing_instead_of_panicking() {
        let mut cv = unsafe { Canvas::new(20, 10) }.expect("canvas");
        cv.fill(rgb(0x10, 0x10, 0x10));
        cv.set_clip(1, 0, -3, 5);
        cv.rect(0, 0, 20, 10, rgb(0xFF, 0, 0));
        cv.round_rect(0.0, 0.0, 20.0, 10.0, 3.0, rgb(0, 0xFF, 0), 1.0);
        assert_eq!(px(&cv, 0, 0), colorref_to_px(rgb(0x10, 0x10, 0x10)), "nothing is painted through an empty clip");
        cv.set_clip(0, 0, 0, 0);
        cv.rect(0, 0, 20, 10, rgb(0xFF, 0, 0));
        assert_eq!(px(&cv, 5, 5), colorref_to_px(rgb(0x10, 0x10, 0x10)));
    }

    #[test]
    fn a_shadow_darkens_below_the_shape_and_never_inside_it() {
        let mut cv = unsafe { Canvas::new(60, 60) }.expect("canvas");
        let bg = rgb(0xF6, 0xF9, 0xF6);
        cv.fill(bg);
        cv.shadow(10.0, 10.0, 40.0, 30.0, 6.0, &[(4.0, 8.0, 0.0, 0.3)], 1.0, rgb(0, 0, 0));
        assert_eq!(px(&cv, 30, 20), colorref_to_px(bg), "inside the shape is left for the shape itself");
        let below = px(&cv, 30, 42) & 0xFF;
        let above = px(&cv, 30, 8) & 0xFF;
        assert!(below < 0xF6 && below < above, "the offset shadow falls below: {below:x} vs {above:x}");
        assert_eq!(px(&cv, 0, 0), colorref_to_px(bg), "nothing far from the shape");
    }

    #[test]
    fn shapes_outside_or_across_a_narrow_clip_stay_inside_it() {
        // A clip left of the shape, right of it, and empty: nothing panics and nothing lands outside the clip.
        let mut cv = unsafe { Canvas::new(60, 40) }.expect("canvas");
        for clip in [(0, 0, 10, 40), (50, 0, 10, 40), (30, 10, 0, 0), (25, 5, 4, 30)] {
            cv.fill(rgb(0, 0, 0));
            cv.set_clip(clip.0, clip.1, clip.2, clip.3);
            cv.shadow(15.0, 8.0, 30.0, 24.0, 6.0, &[(1.0, 2.0, 0.0, 0.5), (18.0, 40.0, -22.0, 0.5)], 1.0, rgb(255, 255, 255));
            cv.round_rect(15.0, 8.0, 30.0, 24.0, 6.0, rgb(255, 255, 255), 1.0);
            cv.clear_clip();
            for y in 0..40 {
                for x in 0..60 {
                    let inside = x >= clip.0 && x < clip.0 + clip.2 && y >= clip.1 && y < clip.1 + clip.3;
                    if !inside {
                        assert_eq!(cv.sample(x, y), 0, "{x},{y} outside clip {clip:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_filled_interior_matches_the_distance_field() {
        // The straight-edge fast path must paint exactly what per-pixel coverage would.
        let mut a = unsafe { Canvas::new(40, 30) }.expect("canvas");
        a.fill(rgb(10, 20, 30));
        a.round_rect(3.3, 2.6, 30.4, 24.2, 6.0, rgb(200, 150, 100), 1.0);
        a.round_rect(5.0, 4.0, 20.0, 10.0, 3.0, rgb(0, 255, 0), 0.5);
        for (x, y) in [(10, 12), (3, 14), (4, 14), (33, 14), (20, 2), (20, 26), (6, 9), (24, 9)] {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let inside_outer = Canvas::sd_round_rect(fx, fy, 3.3, 2.6, 30.4, 24.2, 6.0) < -0.5;
            let clear_of_inner = Canvas::sd_round_rect(fx, fy, 5.0, 4.0, 20.0, 10.0, 3.0) >= 0.5;
            if inside_outer && clear_of_inner {
                assert_eq!(a.sample(x, y), rgb(200, 150, 100), "at {x},{y}");
            }
        }
        let half = a.sample(10, 8);
        assert!((half >> 8 & 0xFF) > 190 && (half >> 8 & 0xFF) < 210, "half green over the fill: {half:06x}");
    }

    #[test]
    fn a_page_entrance_blends_shifted_rows_and_lands_exactly() {
        let mut base = unsafe { Canvas::new(8, 8) }.expect("canvas");
        let mut page = unsafe { Canvas::new(8, 8) }.expect("canvas");
        base.fill(rgb(0, 0, 0));
        page.fill(rgb(0, 0, 0));
        page.rect(0, 2, 8, 1, rgb(200, 100, 50));
        let mut half = unsafe { Canvas::new(8, 8) }.expect("canvas");
        half.fill(rgb(0, 0, 0));
        half.blend_rows_from(&page, (0, 0, 8, 8), |_| (0.5, 0, 2));
        assert_eq!(half.sample(3, 2), rgb(0, 0, 0), "the row moved down by two");
        let mid = half.sample(3, 4);
        assert!((mid & 0xFF) > 90 && (mid & 0xFF) < 110, "half way: {mid:06x}");
        base.blend_rows_from(&page, (0, 0, 8, 8), |_| (1.0, 0, 0));
        assert_eq!(base.sample(3, 2), rgb(200, 100, 50), "at rest it is the page itself");
    }

    #[test]
    fn a_clip_keeps_shapes_inside_it() {
        let mut cv = unsafe { Canvas::new(20, 10) }.expect("canvas");
        cv.fill(rgb(0x10, 0x10, 0x10));
        cv.set_clip(5, 2, 4, 3);
        cv.rect(0, 0, 20, 10, rgb(0xFF, 0, 0));
        assert_eq!(px(&cv, 6, 3), colorref_to_px(rgb(0xFF, 0, 0)), "inside the clip is painted");
        assert_eq!(px(&cv, 4, 3), colorref_to_px(rgb(0x10, 0x10, 0x10)), "left of the clip is untouched");
        assert_eq!(px(&cv, 6, 6), colorref_to_px(rgb(0x10, 0x10, 0x10)), "below the clip is untouched");
        cv.clear_clip();
        cv.rect(0, 0, 20, 10, rgb(0, 0xFF, 0));
        assert_eq!(px(&cv, 0, 9), colorref_to_px(rgb(0, 0xFF, 0)), "clearing the clip restores the whole canvas");
    }
}
