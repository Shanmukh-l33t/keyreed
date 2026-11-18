//! KeyReed design tokens, v3: every colour, radius and elevation the native UI draws with lives here.
//! The values follow `website/styles.css` (`:root` and `[data-theme="dark"]`); docs/design/03-design-v3.md maps
//! each token to its CSS variable. Neutrals are green-tinted, never pure grey; emerald means action or live.

/// COLORREF (0x00BBGGRR), the form GDI and the Settings canvas take.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// The Settings window, the Review panel and the Composer. High contrast builds its own from system colours.
#[derive(Clone, Copy)]
pub(crate) struct Theme {
    /// The deepest tone: the header and the sidebar, around the raised content panel.
    pub(crate) sidebar: u32,
    /// The raised content panel (the page behind the cards).
    pub(crate) bg: u32,
    /// Cards and groups inside the panel.
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
    /// "Working" (transcribing, downloading): brand green or mint, so amber stays reserved for "needs attention".
    pub(crate) working: u32,
    /// Light theme lifts the panel and cards with soft shadows; dark theme uses tone steps and hairlines instead.
    pub(crate) shadows: bool,
    pub(crate) hc: bool,
}

/// Light: page `--bg #F6F9F6`, deep `--bg-2 #EEF4EF`, cards `--surface #FFF`, ink `--text #0C1A14`.
pub(crate) const LIGHT: Theme = Theme {
    sidebar: rgb(0xEE, 0xF4, 0xEF),
    bg: rgb(0xF6, 0xF9, 0xF6),
    surface: rgb(0xFF, 0xFF, 0xFF),
    border: rgb(0xE3, 0xEA, 0xE6),
    control_border: rgb(0x7F, 0x8F, 0x87),
    divider: rgb(0xEC, 0xF1, 0xEE),
    text: rgb(0x0C, 0x1A, 0x14),
    text2: rgb(0x3F, 0x52, 0x48),
    // text3 >= 4.5:1 on every surface it is drawn on; the website's #66786E is for large type only.
    text3: rgb(0x5B, 0x6B, 0x63),
    accent: rgb(0x06, 0x5F, 0x46),
    accent_fill: rgb(0x04, 0x78, 0x57),
    accent_press: rgb(0x06, 0x4E, 0x3B),
    accent_soft: rgb(0xE1, 0xF3, 0xEA),
    hover: rgb(0xE4, 0xEC, 0xE7),
    hover_card: rgb(0xF2, 0xF7, 0xF4),
    on_accent: rgb(0xFF, 0xFF, 0xFF),
    warn_fg: rgb(0x8A, 0x4B, 0x08),
    warn_bg: rgb(0xFE, 0xF4, 0xD7),
    err_fg: rgb(0xA3, 0x14, 0x3C),
    err_bg: rgb(0xFF, 0xE8, 0xEB),
    ok_fg: rgb(0x06, 0x5F, 0x46),
    ok_bg: rgb(0xDC, 0xF5, 0xE8),
    track: rgb(0xD8, 0xE1, 0xDC),
    disabled_bg: rgb(0xEF, 0xF2, 0xF0),
    focus: rgb(0x0C, 0x1A, 0x14),
    focus_ring: rgb(0x04, 0x78, 0x57),
    working: rgb(0x0F, 0x5A, 0x3F),
    shadows: true,
    hc: false,
};

/// Dark: deep `--bg #060908`, page between `--bg-2 #08100D` and `--surface #0C1411`, hairlines `--line`.
pub(crate) const DARK: Theme = Theme {
    sidebar: rgb(0x06, 0x09, 0x08),
    bg: rgb(0x0A, 0x11, 0x0E),
    surface: rgb(0x10, 0x19, 0x15),
    // rgba(176,236,208,.09) over the card, as a solid colour.
    border: rgb(0x1E, 0x2C, 0x26),
    control_border: rgb(0x6F, 0x84, 0x79),
    divider: rgb(0x1A, 0x26, 0x21),
    text: rgb(0xE8, 0xF1, 0xEC),
    text2: rgb(0xA5, 0xB8, 0xAF),
    text3: rgb(0x8D, 0xA3, 0x98),
    accent: rgb(0x6E, 0xE7, 0xB7),
    accent_fill: rgb(0x34, 0xD3, 0x99),
    accent_press: rgb(0x10, 0xB9, 0x81),
    accent_soft: rgb(0x10, 0x30, 0x25),
    hover: rgb(0x19, 0x25, 0x20),
    hover_card: rgb(0x15, 0x21, 0x1C),
    on_accent: rgb(0x04, 0x24, 0x1A),
    warn_fg: rgb(0xF8, 0xCE, 0x85),
    warn_bg: rgb(0x3B, 0x2C, 0x10),
    err_fg: rgb(0xFD, 0xA4, 0xAF),
    err_bg: rgb(0x3E, 0x17, 0x22),
    ok_fg: rgb(0x7B, 0xE8, 0xBC),
    ok_bg: rgb(0x10, 0x34, 0x26),
    track: rgb(0x33, 0x43, 0x3B),
    disabled_bg: rgb(0x11, 0x1A, 0x16),
    focus: rgb(0xE8, 0xF1, 0xEC),
    focus_ring: rgb(0x34, 0xD3, 0x99),
    working: rgb(0xA7, 0xF3, 0xD0),
    shadows: false,
    hc: false,
};

/// Corner radii in logical pixels (4 dp grid).
pub mod radius {
    /// Cards, groups, the content panel, choice cards, the modal card.
    pub const CARD: i32 = 12;
    /// Buttons, inputs, drop-downs, navigation items.
    pub const CONTROL: i32 = 10;
    /// The raised content panel and modal cards.
    pub const PANEL: i32 = 14;
}

/// The light theme's elevation: CSS box-shadow layers `(y offset, blur, spread, alpha)` in the tint #0B3B2C.
pub mod elevation {
    pub type Layer = (f32, f32, f32, f32);
    /// The content panel: the website's `--card-shadow`, `0 1px 2px rgba(11,59,44,.06), 0 18px 40px -22px rgba(11,59,44,.22)`.
    pub const PANEL: [Layer; 2] = [(1.0, 2.0, 0.0, 0.06), (18.0, 40.0, -22.0, 0.22)];
    /// Cards inside the panel: the first layer only, so they separate by tone and a whisper of lift.
    pub const CARD: [Layer; 1] = [(1.0, 2.0, 0.0, 0.06)];
    pub const TINT: u32 = super::rgb(0x0B, 0x3B, 0x2C);
}

/// The listener (pill, tray states): dark in both Windows themes, as straight RGB.
pub mod listener {
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
    /// "Working" (Transcribing, Polishing, Fixing): a quiet mint edge and neutral text. Amber means "needs you".
    pub const WORKING: (u8, u8, u8) = MINT;
    pub const WORKING_TEXT: (u8, u8, u8) = TEXT_2;
    /// Slightly lifted circle behind the mark so the keycap's dark body keeps its silhouette.
    pub const WELL: (u8, u8, u8) = (0x1B, 0x29, 0x23);
}
