//! Centralized product identity constants.
//! Kept in a single module so packaging, Win32 class names, mutexes, and UI strings stay consistent.

pub const PRODUCT_NAME: &str = "Keyreed";
pub const PRODUCT_SLUG: &str = "keyreed";
pub const PRODUCT_TAGLINE: &str = "Voice it. Everywhere.";
pub const SINGLE_INSTANCE_MUTEX: &str = "Local\\Keyreed_SingleInstance_v1";
pub const TRAY_WINDOW_CLASS: &str = "KeyreedTrayHostClass";
pub const OVERLAY_WINDOW_CLASS: &str = "KeyreedOverlayPillClass";
pub const SETTINGS_WINDOW_CLASS: &str = "KeyreedSettingsClass";
pub const AUTOSTART_REG_VALUE: &str = "Keyreed";
/// `WM_USER + 43`. Posted to the tray window by a second process. No string crosses the process boundary.
pub const ALREADY_RUNNING_MSG: u32 = 0x0400 + 43;
pub const VERSION_STR: &str = env!("CARGO_PKG_VERSION");
