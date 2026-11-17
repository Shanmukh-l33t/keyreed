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

/// COLORREF (0x00BBGGRR) helper.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

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
        let x1 = (x + w).clamp(0, self.w);
        let y1 = (y + h).clamp(0, self.h);
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
        let x0 = x.clamp(clip.0, clip.2);
        let x1 = (x + w).clamp(clip.0, clip.2);
        let y0 = y.clamp(clip.1, clip.3);
        let y1 = (y + h).clamp(clip.1, clip.3);
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
        for yy in y0..y1 {
            for xx in x0..x1 {
                let d = Self::sd_round_rect(xx as f32 + 0.5, yy as f32 + 0.5, x, y, w, h, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend((yy * self.w + xx) as usize, c, cov * alpha);
                }
            }
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
