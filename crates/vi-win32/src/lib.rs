#[cfg(windows)]
pub mod canvas;
#[cfg(windows)]
pub mod composer;
pub mod clipboard;
pub mod context;
pub mod hotkey;
pub mod icon_data;
pub mod overlay;
pub mod security;
#[cfg(windows)]
pub mod selection;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod spell;
pub mod typing;
#[cfg(windows)]
pub mod uia;
pub mod ui;

pub use clipboard::{input_gate, normalize_windows_newlines, read_clipboard_unicode, write_clipboard_unicode};
pub use context::{capture_foreground_context, classify_application, is_private_browsing_title};
pub use hotkey::{is_spec_physically_down, ChordAction, GlobalHotkeyEngine, HotkeyEvent, ModifierState, MouseButton};
pub use overlay::{render_pill, PillFrame, PillKind};
pub use security::{
    current_process_cpu_ms, current_process_memory_mb, dpapi_decrypt_from_b64,
    dpapi_encrypt_to_b64, harden_and_init_process, perf_trace_record, read_autostart_value,
    set_autostart_enabled, system_available_ram_mb, timing_mark, SingleInstanceGuard,
    KEYREED_SYNTHETIC_TAG,
};
pub use typing::{
    ask_before_inserting_decision, deliver_direct_or_clipboard_fallback, force_foreground_window,
    is_current_process_elevated, is_unsupported_shell_target,
    prompt_and_deliver_ask_before_inserting, sanitize_for_injection, AskInsertDecision,
    DirectTypeOutcome,
};
pub use ui::{SharedUiModel, UiAction};
