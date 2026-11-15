pub mod clipboard;
pub mod context;
pub mod hotkey;
pub mod overlay;
pub mod security;
pub mod typing;
pub mod ui;

pub use clipboard::{normalize_windows_newlines, read_clipboard_unicode, write_clipboard_unicode};
pub use context::{capture_foreground_context, classify_application, is_private_browsing_title};
pub use hotkey::{GlobalHotkeyEngine, HotkeyEvent, ModifierState};
pub use overlay::render_pill_bgra_premultiplied;
pub use security::{
    current_process_memory_mb, dpapi_decrypt_from_b64, dpapi_encrypt_to_b64,
    harden_and_init_process, read_autostart_value, set_autostart_enabled, SingleInstanceGuard,
    timing_mark, KEYREED_SYNTHETIC_TAG,
};
pub use typing::{
    deliver_direct_or_clipboard_fallback, is_current_process_elevated, sanitize_for_injection,
    DirectTypeOutcome,
};
pub use ui::{SharedUiModel, UiAction};
