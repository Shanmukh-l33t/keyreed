//! Per-Monitor DPI v2 Non-Activating Layered Floating Microphone Pill (Master Prompt Section 10).
//!
//! Uses `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` and `UpdateLayeredWindow`
//! with a smooth anti-aliased premultiplied-ARGB pill buffer so it NEVER steals keyboard focus from
//! the user's active window even when clicked!

use vi_core::EngineState;

/// Pure software premultiplied-ARGB pill renderer (unit-testable and screenshot-verifiable!).
/// Produces a `width * height` BGRA32 premultiplied buffer for Win32 `CreateDIBSection` + `UpdateLayeredWindow`.
pub fn render_pill_bgra_premultiplied(
    width: usize,
    height: usize,
    state: &EngineState,
    rms_level: f32,
) -> Vec<u32> {
    let mut pixels = vec![0u32; width * height];
    if width < 8 || height < 8 {
        return pixels;
    }

    // State accent color (R, G, B)
    let (acc_r, acc_g, acc_b) = match state {
        EngineState::Disabled => (108u8, 117u8, 125u8),     // Muted gray
        EngineState::Idle => (31u8, 107u8, 74u8),
        EngineState::Listening => (31u8, 107u8, 74u8),
        EngineState::Processing => (246u8, 173u8, 85u8),    // Warm amber (transcribing)
        EngineState::Correcting => (56u8, 189u8, 248u8),    // Cyan (AI grammar polish)
        EngineState::ReadyToPaste => (52u8, 211u8, 153u8),  // Emerald green (Ctrl+V ready)
        EngineState::Error(_) => (251u8, 113u8, 133u8),     // Rose alert
    };

    let radius = (height as f32) * 0.5;
    let left_cx = radius;
    let right_cx = (width as f32) - radius;
    let cy = radius;

    let dot_cx = height as f32 * 0.52;
    let dot_r = height as f32 * 0.20;

    // Waveform bar region on the right side of the pill
    let bar_start_x = (height as f32 * 0.95) as usize;
    let bar_end_x = width.saturating_sub((height as f32 * 0.35) as usize);
    let active_bar_w = match state {
        EngineState::Listening => {
            let lvl = (rms_level * 3.2).clamp(0.18, 1.0);
            ((bar_end_x.saturating_sub(bar_start_x)) as f32 * lvl) as usize
        }
        EngineState::Processing => ((bar_end_x.saturating_sub(bar_start_x)) * 55) / 100,
        EngineState::Correcting => ((bar_end_x.saturating_sub(bar_start_x)) * 80) / 100,
        EngineState::ReadyToPaste => bar_end_x.saturating_sub(bar_start_x),
        _ => ((bar_end_x.saturating_sub(bar_start_x)) * 28) / 100,
    };

    for y in 0..height {
        let py = (y as f32) + 0.5;
        for x in 0..width {
            let px = (x as f32) + 0.5;
            // Signed distance to capsule (stadium) shape for smooth 1px anti-aliasing
            let clamped_x = px.clamp(left_cx, right_cx);
            let dist = ((px - clamped_x).powi(2) + (py - cy).powi(2)).sqrt();
            let edge_cov = (radius - dist + 0.5).clamp(0.0, 1.0);
            if edge_cov <= 0.0 {
                continue;
            }

            // Base dark obsidian glass background + subtle 1px border highlight
            let is_border = (radius - dist) < 1.25;
            let (mut r, mut g, mut b, mut a) = if is_border {
                (
                    ((acc_r as u16 + 40).min(255)) as u8,
                    ((acc_g as u16 + 40).min(255)) as u8,
                    ((acc_b as u16 + 40).min(255)) as u8,
                    225u8,
                )
            } else {
                (20u8, 24u8, 33u8, 236u8)
            };

            // Status orb on left
            let d_orb = ((px - dot_cx).powi(2) + (py - cy).powi(2)).sqrt();
            if d_orb <= dot_r + 0.6 {
                let orb_cov = (dot_r - d_orb + 0.5).clamp(0.0, 1.0);
                r = lerp_u8(r, acc_r, orb_cov);
                g = lerp_u8(g, acc_g, orb_cov);
                b = lerp_u8(b, acc_b, orb_cov);
                a = 250;
            }

            // Progress / live voice bar in center-right
            let bar_half_h = (height as f32 * 0.11).max(2.0);
            if x >= bar_start_x && x < bar_end_x && (py - cy).abs() <= bar_half_h {
                if x < bar_start_x + active_bar_w {
                    r = acc_r;
                    g = acc_g;
                    b = acc_b;
                } else {
                    r = 52;
                    g = 60;
                    b = 76;
                }
            }

            let final_a = ((a as f32) * edge_cov).round() as u32;
            let pr = (((r as u32) * final_a) / 255) & 0xFF;
            let pg = (((g as u32) * final_a) / 255) & 0xFF;
            let pb = (((b as u32) * final_a) / 255) & 0xFF;
            pixels[y * width + x] = (final_a << 24) | (pr << 16) | (pg << 8) | pb;
        }
    }
    pixels
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    ((a as f32) + ((b as f32) - (a as f32)) * t).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_non_empty_anti_aliased_capsule() {
        let buf = render_pill_bgra_premultiplied(156, 36, &EngineState::Listening, 0.5);
        assert_eq!(buf.len(), 156 * 36);
        // Corner pixel outside capsule radius must be transparent
        assert_eq!(buf[0], 0);
        // Center pixel must be opaque premultiplied ARGB
        let center = buf[18 * 156 + 78];
        assert!((center >> 24) > 200);
    }
}
