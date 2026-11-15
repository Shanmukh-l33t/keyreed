//! Per-Monitor DPI v2 Non-Activating Layered Floating Microphone Control (Master Directive Section 8).
//!
//! Uses `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` and `UpdateLayeredWindow`
//! with an anti-aliased premultiplied-ARGB pill buffer featuring the official KeyReed keycap + reed-waveform
//! brand mark, 5-bar live voice equalizer, and distinct state visuals (Idle, Listening, Processing, Completed, Error).

use vi_core::EngineState;

/// Embedded 28x28 premultiplied-BGRA bitmap of the official KeyReed keycap + reed-waveform logo mark.
pub const KEYREED_ICON_28_BGRA: [u32; 784] = [
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x01000000, 0x01000000, 0x01000000, 0x01000000, 0x01000000,
    0x01000000, 0x01000000, 0x01000000, 0x01000000, 0x01000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x03000000, 0x02020202, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x03010101, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x02000000, 0x00000000, 0x00000000, 0x07000000, 0x0A000000, 0x0B000000, 0x0B000000, 0x0B000000,
    0x0C000000, 0x0C000000, 0x0A000000, 0x0B000000, 0x04000000, 0x00000000, 0x00000000, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x02000000, 0x00000000, 0x3C000000, 0xB2252B2D, 0xE0565C5E, 0xE55C6264, 0xE65A6062, 0xE65C6162, 0xE6585F60,
    0xE8606567, 0xE85F6466, 0xE65A6061, 0xE65C6163, 0xDB525B5C, 0xA3192021, 0x29000000, 0x00000000, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x03000000, 0x00000000, 0x4B000000, 0xFC6F7677, 0xFFF3F4F5, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF,
    0xFFF7FFFD, 0xFFF9FFFD, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFE8EAEA, 0xEF535B5C, 0x34000000, 0x01010101, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x01000000, 0x01010101, 0x08000000, 0xE03A4143, 0xFFFFFFFF, 0xF9F9F9F9, 0xFEFBFBFB, 0xFDFBFBFB, 0xFEF9F9F9, 0xFEFEFCFC, 0xFEABC5BD,
    0xFE10563B, 0xFEA5C2B8, 0xFEFBFBFB, 0xFEF8F9F8, 0xFDFAFAFA, 0xF8F8F8F8, 0xFFF2F4F4, 0xC61B2325, 0x00000000, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x03000000, 0x00000000, 0x45000000, 0xFF909697, 0xFCFCFCFC, 0xFFFAFAFA, 0xFFFFFFFF, 0xFFFCFDFD, 0xFFFDFEFD, 0xFFEEF4F2, 0xFF1B664F,
    0xFF004529, 0xFFB3CCC4, 0xFFFDFEFD, 0xFFFBFCFB, 0xFFFFFFFF, 0xFFFBFBFB, 0xFDFDFDFD, 0xFB60696A, 0x25000000, 0x00000000, 0x02000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x04000000, 0x00000000, 0x99060C0D, 0xFED4D7D7, 0xFBFBFBFB, 0xFFFBFCFC, 0xFFFDFDFD, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFD6E4E0, 0xFF03543A,
    0xFF296C57, 0xFFF0F5F3, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFCFDFC, 0xFFFBFBFB, 0xFBFBFBFB, 0xFFABB2B2, 0x79000202, 0x00000000, 0x04000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x01000000, 0x00000000, 0x0A000000, 0xE4374041, 0xFFFCFDFD, 0xFDFAFBFB, 0xFFFFFFFF, 0xFFFFFFFF, 0xFF8FB3A7, 0xFFAAC6BD, 0xFFE9EFED, 0xFF00422C,
    0xFF447667, 0xFFFFFFFF, 0xFF6C9B8B, 0xFFC9DAD4, 0xFFFFFFFF, 0xFFFFFFFF, 0xFCFCFCFC, 0xFFEBEDED, 0xCD152222, 0x01000000, 0x01010101, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x03000000, 0x00000000, 0x49000000, 0xFF818989, 0xFDFDFDFD, 0xFFFFFFFF, 0xFFC4D6D0, 0xFFBDD1CA, 0xFF5D907F, 0xFF74A091, 0xFFF0F3F3, 0xFF003F2A,
    0xFF407565, 0xFFFEFFFE, 0xFF1F634C, 0xFFAAC4BB, 0xFFA4BFB7, 0xFFE5EDEA, 0xFFFFFFFF, 0xFEFEFEFE, 0xFC576362, 0x2B000000, 0x00000000, 0x02000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x03000101, 0x00000000, 0x9C040B0B, 0xFED2D4D5, 0xFBEBF2F0, 0xFFB6CCC5, 0xFF7EA598, 0xFF639181, 0xFF749F91, 0xFF7CA496, 0xFFEEF2F2, 0xFF00452D,
    0xFF3F7966, 0xFFFFFFFF, 0xFF2F705A, 0xFFADC7BE, 0xFF2E6B58, 0xFFB3CAC2, 0xFFAAC1BA, 0xFBFBFBFB, 0xFFAAB3B2, 0x80000302, 0x00000000, 0x04000000, 0x00000000, 0x00000000,
    0x00000000, 0x01000000, 0x00000000, 0x0B000000, 0xE62F3A3B, 0xFFFFFFFF, 0xFEBED2CC, 0xFF357660, 0xFF8CADA2, 0xFF719B8D, 0xFF709C8F, 0xFF7CA598, 0xFFEFECEE, 0xFF00211A,
    0xFF445E59, 0xFFFFFFFF, 0xFF2D6E58, 0xFFABC5BC, 0xFF477B6A, 0xFFA3BEB5, 0xFF195D47, 0xFDF1F5F4, 0xFFF0F1F1, 0xD4142523, 0x02000000, 0x01010101, 0x00000000, 0x00000000,
    0x00000000, 0x03000000, 0x00000000, 0x4A000000, 0xFF7E8687, 0xFDFDFDFD, 0xFFE1E9E7, 0xFFB7CDC5, 0xFF7FA79A, 0xFF639484, 0xFF739D90, 0xFF7DA698, 0xFFEFECEE, 0xFF000D0F,
    0xFF465152, 0xFFFFFFFF, 0xFF2E6C58, 0xFFAEC6BE, 0xFF2E6B58, 0xFFB3C9C2, 0xFFACC4BC, 0xFFF4F7F6, 0xFDFDFDFD, 0xFE596968, 0x34000000, 0x00000000, 0x03000000, 0x00000000,
    0x00000000, 0x03000101, 0x00000000, 0x9D020809, 0xFECBCFCF, 0xFBFBFBFB, 0xFFFEFEFE, 0xFFFFFFFF, 0xFFB7CDC6, 0xFFAFC9C0, 0xFF639082, 0xFF78A194, 0xFFF0ECEF, 0xFF000F10,
    0xFF465353, 0xFFFFFFFF, 0xFF24634F, 0xFFACC4BC, 0xFF89ADA2, 0xFFDDE7E4, 0xFFFFFFFF, 0xFFFBFCFB, 0xFBFBFBFB, 0xFFACB5B4, 0x8D000303, 0x00000000, 0x04000000, 0x00000000,
    0x01000000, 0x00000000, 0x0C000000, 0xE7323D3D, 0xFFFBFBFB, 0xFDFDFDFD, 0xFFFEFEFE, 0xFFFCFDFD, 0xFFFFFFFF, 0xFFFFFFFF, 0xFF79A195, 0xFF98B9AE, 0xFFEDEAEC, 0xFF011011,
    0xFF485354, 0xFFFFFFFF, 0xFF4D7F6F, 0xFFBDCFC9, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFCFDFC, 0xFFFEFEFE, 0xFDFDFDFD, 0xFFECEFEE, 0xDD162B29, 0x06000000, 0x01010101, 0x01000000,
    0x04000000, 0x01010101, 0x4A000000, 0xFF535E5E, 0xFDFDFDFD, 0xFFFCFCFC, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFDFEFD, 0xFFFDFDFD, 0xFFFDFDFD, 0xFFFFFFFF, 0xFFD8DADB, 0xFF021011,
    0xFF455151, 0xFFFFFFFF, 0xFFFBFDFC, 0xFFFFFFFF, 0xFFFCFDFD, 0xFFFEFEFE, 0xFFFFFFFF, 0xFFFEFEFE, 0xFFFDFDFD, 0xFDFDFDFD, 0xFF2A4642, 0x40000100, 0x00000000, 0x03000000,
    0x03000101, 0x00000000, 0xA2050F10, 0xFE1F3030, 0xFBECEDED, 0xFFFFFFFF, 0xFFFCFCFC, 0xFFFCFDFD, 0xFFFCFDFD, 0xFFFCFDFD, 0xFFFBFBFB, 0xFFFCFCFC, 0xFFE4E5E6, 0xFF0A191A,
    0xFF535E5E, 0xFFFCFDFD, 0xFFFBFBFB, 0xFFFCFDFD, 0xFFFCFCFC, 0xFFFCFDFD, 0xFFFCFDFD, 0xFFFBFCFC, 0xFFFFFFFF, 0xFBCED3D3, 0xFF082621, 0x9A041512, 0x00000000, 0x04000000,
    0x00000000, 0x10000101, 0xEF0F1F1F, 0xFF061918, 0xFE5E6B6A, 0xFFE6E7E8, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFE5E7E7,
    0xFFF7F7F7, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFCFD4D4, 0xFE2C4440, 0xFF011D18, 0xE807211D, 0x0B000000, 0x00000000,
    0x00000000, 0x59050A0A, 0xFF0F2121, 0xFC172827, 0xFF324342, 0xFF3E4D4D, 0xFF52605F, 0xFF586564, 0xFF576363, 0xFF586363, 0xFF586363, 0xFF586363, 0xFF576262, 0xFF5F6A69,
    0xFF5B6666, 0xFF556161, 0xFF586364, 0xFF586464, 0xFF586464, 0xFF586463, 0xFF576363, 0xFF455554, 0xFF0B2A25, 0xFF00241C, 0xFC052420, 0xFF062520, 0x51020909, 0x00000000,
    0x00000000, 0x91081111, 0xFD0E1F1F, 0xFB223231, 0xFF455352, 0xFF283938, 0xFF071B1A, 0xFF021514, 0xFF031515, 0xFF031414, 0xFF021213, 0xFF021213, 0xFF021313, 0xFF001111,
    0xFF011213, 0xFF021314, 0xFF021314, 0xFF011213, 0xFF001213, 0xFF001213, 0xFF001213, 0xFF011716, 0xFF072722, 0xFF062E26, 0xFB04231E, 0xFD05241F, 0x8F041211, 0x00000000,
    0x00000000, 0x77070E0E, 0xFF122423, 0xF82F3E3D, 0xFE394847, 0xFE1D2E2D, 0xFE122424, 0xFE122424, 0xFE122323, 0xFE122222, 0xFE112021, 0xFE112021, 0xFE122121, 0xFE112122,
    0xFE112122, 0xFE112122, 0xFE122222, 0xFE112021, 0xFE101F20, 0xFE102021, 0xFE102121, 0xFE0F2121, 0xFE092320, 0xFF052A23, 0xF903231E, 0xFF062420, 0x7D030F0E, 0x00000000,
    0x01000000, 0x1A000202, 0xE8172526, 0xFF3A4A4A, 0xFF2D3D3C, 0xFF142625, 0xFF102121, 0xFF102020, 0xFF101F20, 0xFF101F20, 0xFF101E1F, 0xFF101E1F, 0xFF101E1F, 0xFF101E1F,
    0xFF0F1E20, 0xFF0F1E1F, 0xFF0F1E1F, 0xFF0F1E1F, 0xFF0E1D1E, 0xFF0F1E1F, 0xFF0F1F20, 0xFF0F2020, 0xFF0C2220, 0xFF062722, 0xFF032C24, 0xF405211D, 0x22000404, 0x01000000,
    0x02000000, 0x00000000, 0x28050808, 0xAD232C2C, 0xE81F2D2C, 0xF4102020, 0xFC101F1F, 0xFD0F1E1E, 0xFD101E1F, 0xFD101E1F, 0xFD0F1D1E, 0xFD0F1D1E, 0xFD0F1D1E, 0xFD0F1C1E,
    0xFD101D1F, 0xFD0F1D1E, 0xFD0F1E1E, 0xFD0F1D1E, 0xFD0F1D1F, 0xFD0F1D1F, 0xFD0F1F20, 0xFD0F1F20, 0xFC0E2020, 0xF7082320, 0xCB03211B, 0x3C010806, 0x00000000, 0x02000000,
    0x00000000, 0x01000000, 0x00000000, 0x00000000, 0x0E000101, 0x19010303, 0x24020404, 0x26020303, 0x26020404, 0x26020404, 0x26020304, 0x26020404, 0x26020304, 0x26020303,
    0x26020404, 0x26020304, 0x26020404, 0x25020404, 0x26020405, 0x26020304, 0x26020404, 0x26020404, 0x25020404, 0x20010303, 0x00000000, 0x00000000, 0x02000000, 0x00000000,
    0x00000000, 0x00000000, 0x02000000, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x02000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x01000000, 0x01000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000,
    0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x02000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
];

/// Pure software premultiplied-ARGB pill renderer (unit-testable and screenshot-verifiable).
pub fn render_pill_bgra_premultiplied(
    width: usize,
    height: usize,
    state: &EngineState,
    rms_level: f32,
) -> Vec<u32> {
    render_pill_bgra_premultiplied_ex(width, height, state, rms_level, 0, false)
}

/// Extended renderer supporting animation phase and hover state for the floating control.
pub fn render_pill_bgra_premultiplied_ex(
    width: usize,
    height: usize,
    state: &EngineState,
    rms_level: f32,
    anim_tick: u32,
    is_hovered: bool,
) -> Vec<u32> {
    let mut pixels = vec![0u32; width * height];
    if width < 8 || height < 8 {
        return pixels;
    }

    // State accent color (R, G, B) — aligned with KeyReed's forest emerald & charcoal design language
    let (acc_r, acc_g, acc_b) = match state {
        EngineState::Disabled => (108u8, 117u8, 125u8),     // Muted slate
        EngineState::Idle => (31u8, 107u8, 74u8),           // KeyReed forest emerald (#1F6B4A)
        EngineState::Listening => (42u8, 168u8, 112u8),     // Active listening emerald
        EngineState::Processing => (245u8, 158u8, 11u8),    // Warm amber (transcribing)
        EngineState::Correcting => (20u8, 184u8, 166u8),    // Teal-emerald (grammar polish)
        EngineState::ReadyToPaste => (16u8, 185u8, 129u8),  // Completed emerald
        EngineState::Error(_) => (244u8, 63u8, 94u8),       // Rose alert
    };

    let (bg_r, bg_g, bg_b) = if is_hovered {
        (28u8, 33u8, 39u8)
    } else {
        (21u8, 25u8, 30u8)
    };

    let radius = (height as f32) * 0.5;
    let left_cx = radius;
    let right_cx = (width as f32) - radius;
    let cy = radius;

    // 5-bar equalizer geometry positioned next to the KeyReed keycap logo mark
    let scale = (height as f32) / 38.0;
    let eq_start_x = (height as f32 * 1.05).round() as isize;
    let bar_w = (3.0 * scale).round().max(2.0);
    let bar_gap = (2.8 * scale).round().max(2.0);
    let phase = (anim_tick as f32) * 0.45;

    let mut bar_heights = [0.0f32; 5];
    let max_bar_half_h = (height as f32 * 0.28).max(4.0);
    for (i, bh) in bar_heights.iter_mut().enumerate() {
        let idx_f = i as f32;
        let frac = match state {
            EngineState::Listening => {
                let base_lvl = (rms_level * 3.8).clamp(0.22, 1.0);
                let wave = ((phase + idx_f * 1.1).sin() * 0.28 + 0.72).clamp(0.2, 1.0);
                let center_boost = if i == 2 { 1.0 } else if i == 1 || i == 3 { 0.82 } else { 0.58 };
                (base_lvl * wave * center_boost).clamp(0.18, 1.0)
            }
            EngineState::Processing | EngineState::Correcting => {
                let wave = ((phase * 1.3 - idx_f * 0.85).sin() * 0.5 + 0.5).clamp(0.22, 0.95);
                wave
            }
            EngineState::ReadyToPaste => {
                // Rising completion cadence
                (0.35 + (idx_f * 0.14)).clamp(0.35, 0.92)
            }
            EngineState::Error(_) => {
                if i == 2 { 0.85 } else { 0.25 }
            }
            _ => {
                // Idle: calm, symmetrical reed silhouette matching the logo
                match i {
                    0 | 4 => 0.22,
                    1 | 3 => 0.36,
                    _ => 0.54,
                }
            }
        };
        *bh = (max_bar_half_h * frac).max(1.5);
    }

    for y in 0..height {
        let py = (y as f32) + 0.5;
        for x in 0..width {
            let px = (x as f32) + 0.5;
            let clamped_x = px.clamp(left_cx, right_cx);
            let dist = ((px - clamped_x).powi(2) + (py - cy).powi(2)).sqrt();
            let edge_cov = (radius - dist + 0.5).clamp(0.0, 1.0);
            if edge_cov <= 0.0 {
                continue;
            }

            let is_border = (radius - dist) < 1.35;
            let (mut r, mut g, mut b, a) = if is_border {
                let border_mix = if matches!(state, EngineState::Idle | EngineState::Disabled) && !is_hovered {
                    0.42
                } else {
                    0.82
                };
                (
                    lerp_u8(bg_r, acc_r, border_mix),
                    lerp_u8(bg_g, acc_g, border_mix),
                    lerp_u8(bg_b, acc_b, border_mix),
                    242u8,
                )
            } else {
                (bg_r, bg_g, bg_b, 244u8)
            };

            // Render 5-bar voice equalizer in the center-left region
            let rel_x = (x as isize) - eq_start_x;
            if rel_x >= 0 {
                let stride = (bar_w + bar_gap) as isize;
                if stride > 0 {
                    let bar_idx = (rel_x / stride) as usize;
                    let in_bar_x = (rel_x % stride) as f32;
                    if bar_idx < 5 && in_bar_x < bar_w {
                        let dy = (py - cy).abs();
                        let half_h = bar_heights[bar_idx];
                        if dy <= half_h + 0.5 {
                            let cov = (half_h - dy + 0.5).clamp(0.0, 1.0);
                            let (br, bg_c, bb) = if matches!(state, EngineState::Idle) && !is_hovered {
                                (62u8, 128u8, 98u8)
                            } else {
                                (acc_r, acc_g, acc_b)
                            };
                            r = lerp_u8(r, br, cov);
                            g = lerp_u8(g, bg_c, cov);
                            b = lerp_u8(b, bb, cov);
                        }
                    }
                }
            }

            let final_a = ((a as f32) * edge_cov).round() as u32;
            let pr = (((r as u32) * final_a) / 255) & 0xFF;
            let pg = (((g as u32) * final_a) / 255) & 0xFF;
            let pb = (((b as u32) * final_a) / 255) & 0xFF;
            pixels[y * width + x] = (final_a << 24) | (pr << 16) | (pg << 8) | pb;
        }
    }

    // Composite the embedded 28x28 KeyReed logo mark onto the left side of the pill
    let icon_target_sz = ((height as f32) * 0.72).round().clamp(16.0, 48.0) as usize;
    let icon_x0 = ((height.saturating_sub(icon_target_sz)) / 2).max(4);
    let icon_y0 = (height.saturating_sub(icon_target_sz)) / 2;

    for iy in 0..icon_target_sz {
        let dst_y = icon_y0 + iy;
        if dst_y >= height {
            continue;
        }
        let src_y = ((iy * 28) / icon_target_sz).min(27);
        for ix in 0..icon_target_sz {
            let dst_x = icon_x0 + ix;
            if dst_x >= width {
                continue;
            }
            let src_x = ((ix * 28) / icon_target_sz).min(27);
            let src_px = KEYREED_ICON_28_BGRA[src_y * 28 + src_x];
            let sa = (src_px >> 24) & 0xFF;
            if sa == 0 {
                continue;
            }
            let dst_px = pixels[dst_y * width + dst_x];
            let inv_sa = 255 - sa;
            let out_a = sa + (((dst_px >> 24) & 0xFF) * inv_sa) / 255;
            let out_r = ((src_px >> 16) & 0xFF) + ((((dst_px >> 16) & 0xFF) * inv_sa) / 255);
            let out_g = ((src_px >> 8) & 0xFF) + ((((dst_px >> 8) & 0xFF) * inv_sa) / 255);
            let out_b = (src_px & 0xFF) + (((dst_px & 0xFF) * inv_sa) / 255);
            pixels[dst_y * width + dst_x] =
                (out_a.min(255) << 24) | (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255);
        }
    }

    // Small live state badge dot at bottom-right of the KeyReed icon mark
    let badge_cx = (icon_x0 + icon_target_sz.saturating_sub(2)) as f32;
    let badge_cy = (icon_y0 + icon_target_sz.saturating_sub(2)) as f32;
    let badge_r = (2.8 * scale).clamp(2.2, 4.5);
    for y in 0..height {
        let py = (y as f32) + 0.5;
        if (py - badge_cy).abs() > badge_r + 1.5 {
            continue;
        }
        for x in 0..width {
            let px = (x as f32) + 0.5;
            let d = ((px - badge_cx).powi(2) + (py - badge_cy).powi(2)).sqrt();
            if d <= badge_r + 0.6 {
                let cov = (badge_r - d + 0.5).clamp(0.0, 1.0);
                let dst_px = pixels[y * width + x];
                let da = (dst_px >> 24) & 0xFF;
                if da > 0 {
                    let dr = ((dst_px >> 16) & 0xFF) as u8;
                    let dg = ((dst_px >> 8) & 0xFF) as u8;
                    let db = (dst_px & 0xFF) as u8;
                    let nr = lerp_u8(dr, acc_r, cov) as u32;
                    let ng = lerp_u8(dg, acc_g, cov) as u32;
                    let nb = lerp_u8(db, acc_b, cov) as u32;
                    pixels[y * width + x] = (da << 24) | (nr << 16) | (ng << 8) | nb;
                }
            }
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

    #[test]
    fn renders_all_five_visual_states_distinctly() {
        let idle = render_pill_bgra_premultiplied_ex(196, 40, &EngineState::Idle, 0.0, 0, false);
        let listen = render_pill_bgra_premultiplied_ex(196, 40, &EngineState::Listening, 0.7, 2, false);
        let proc_s = render_pill_bgra_premultiplied_ex(196, 40, &EngineState::Processing, 0.0, 4, false);
        let done = render_pill_bgra_premultiplied_ex(196, 40, &EngineState::ReadyToPaste, 0.0, 0, false);
        let err = render_pill_bgra_premultiplied_ex(
            196,
            40,
            &EngineState::Error("mic".to_string()),
            0.0,
            0,
            false,
        );
        assert_ne!(idle, listen);
        assert_ne!(listen, proc_s);
        assert_ne!(proc_s, done);
        assert_ne!(done, err);
    }
}
