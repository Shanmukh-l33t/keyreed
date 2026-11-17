//! Dedicated `WH_KEYBOARD_LL` global Push-to-Talk thread (Master Prompt Section 3 & 12).
//!
//! Architecture & Security rules:
//! - Runs on its own high-priority message-pump thread (`THREAD_PRIORITY_ABOVE_NORMAL`) so UI rendering or
//!   model loading NEVER delays `LowLevelKeyboardProc` past Windows' `LowLevelHooksTimeout` (~300 ms).
//! - Inspects ONLY modifier VK codes, `VK_ESCAPE`, and the configured shortcut VKs: ZERO keystroke logging or buffering.
//!   One exception: while a modifier-only dictation chord is held, any key-down is seen once as "another key joined"
//!   (so Right Ctrl + C cancels the recording). Its identity is never stored or logged.
//! - Injects unassigned `VK_MENU_MASK` (`0xE8`) when `Win` or `Alt` chord triggers so releasing `Win`/`Alt`
//!   never pops open the Windows Start Menu or app menu bar!
//! - Dynamically installs and removes (`UnhookWindowsHookEx`) the hook when Push-to-Talk is independently toggled!
//! - Runs a second `WH_MOUSE_LL` hook on the same thread for the three bindable buttons (middle, back, forward).
//!   Moves, the wheel and left/right clicks return before any state is read, so a mouse never fires an action
//!   it was not bound to.

use crate::security::KEYREED_SYNTHETIC_TAG;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use vi_core::HotkeySpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    PttDown,
    PttUp,
    EmergencyCancel,
    CycleMode,
    CorrectSelection,
    OpenComposer,
    /// A bound mouse button: start or stop dictation, as a click on the floating listener does.
    ToggleDictation,
    VoiceCommandDown,
    VoiceCommandUp,
    /// A longer chord fired while a modifier-only dictation chord (its prefix) had already
    /// started: the recording that began must be dropped silently.
    PttSuperseded,
}

/// What an extra chord in the shortcut table does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordAction {
    CorrectSelection,
    OpenComposer,
    /// Held like dictation: `VoiceCommandDown` on press, `VoiceCommandUp` on release.
    VoiceCommand,
    /// Only a mouse binding asks for this: a press toggles dictation.
    Dictate,
}

/// The buttons KeyReed can bind. Left and right are never intercepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Middle,
    Back,
    Forward,
}

/// At most this many extra chords (one latch bit each).
pub const MAX_EXTRA_CHORDS: usize = 8;

/// Set while the Settings shortcut-capture field is recording a chord: the hook then matches nothing.
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Pauses (true) or resumes shortcut matching for the duration of a capture in Settings.
pub fn set_capture_active(on: bool) {
    CAPTURE_ACTIVE.store(on, Ordering::Relaxed);
}

#[derive(Debug, Clone)]
struct HookSharedConfig {
    ptt_spec: HotkeySpec,
    mode_cycle_spec: HotkeySpec,
    /// Extra chords; only entries with a trigger key (`vk != 0`) are used.
    chords: Vec<(HotkeySpec, ChordAction)>,
    /// Bound mouse buttons; an unbound button is not in here and is never swallowed.
    mouse: Vec<(MouseButton, ChordAction)>,
    cancel_vk: u16,
    active_recording_or_busy: Arc<AtomicBool>,
}

fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

impl HookSharedConfig {
    /// Privacy / anti-keylogger invariant: only modifier keys, the cancel key and configured
    /// trigger keys are ever looked at.
    fn is_relevant(&self, vk: u16) -> bool {
        is_modifier_vk(vk)
            || vk == self.cancel_vk
            || (self.ptt_spec.vk != 0 && vk == self.ptt_spec.vk)
            || (self.mode_cycle_spec.vk != 0 && vk == self.mode_cycle_spec.vk)
            || self.chords.iter().any(|(c, _)| c.vk != 0 && c.vk == vk)
    }

    /// Dictation is a modifier chord (Right Ctrl, Ctrl+Win): a trigger key joining it means a shortcut for another app.
    fn ptt_modifier_only(&self) -> bool {
        self.ptt_spec.vk == 0 || is_modifier_vk(self.ptt_spec.vk)
    }
}

/// What the hook does with one relevant key event.
#[derive(Debug, Default, PartialEq, Eq)]
struct KeyDecision {
    events: Vec<HotkeyEvent>,
    /// Do not pass the key on to the focused application.
    swallow: bool,
    /// Inject the menu mask so releasing Win/Alt does not open Start or a menu bar.
    menu_mask: bool,
}

/// The shortcut matcher (everything but Escape-cancel), pure so it can be unit-tested.
fn decide(cfg: &HookSharedConfig, mods: &mut ModifierState, vk: u16, is_down: bool) -> KeyDecision {
    let mut d = KeyDecision::default();
    if !mods.update_modifier(vk, is_down) {
        if is_down {
            mods.main_vk_down = vk;
        } else if mods.main_vk_down == vk {
            mods.main_vk_down = 0;
        }
    }
    if !(mods.any_ctrl() || mods.any_alt() || mods.any_shift() || mods.any_win()) {
        mods.ptt_blocked_by_chord = false;
    }

    // A trigger key swallowed on press: swallow its auto-repeats and its release too, so the app never
    // sees a lone key-up or repeats. Repeats fire nothing new; the release still runs the logic below.
    if mods.swallowed.contains(&vk) {
        d.swallow = true;
        if is_down {
            return d;
        }
        mods.swallowed.retain(|&k| k != vk);
    }

    // Mode cycle chord
    if is_down && !mods.ptt_combo_latched && mods.matches_spec(&cfg.mode_cycle_spec, vk, true) {
        d.events.push(HotkeyEvent::CycleMode);
        d.swallow = true;
        mods.latch_swallow(vk);
        return d;
    }

    // Extra chords, before dictation so a longer chord wins over a modifier-only prefix.
    for (i, (spec, action)) in cfg.chords.iter().take(MAX_EXTRA_CHORDS).enumerate() {
        let bit = 1u8 << i;
        if spec.vk == 0 {
            continue;
        }
        if is_down && mods.chord_latch & bit == 0 && mods.matches_spec(spec, vk, true) {
            mods.chord_latch |= bit;
            mods.ptt_blocked_by_chord = true;
            if mods.ptt_combo_latched {
                mods.ptt_combo_latched = false;
                d.events.push(HotkeyEvent::PttSuperseded);
            }
            d.events.push(match action {
                ChordAction::CorrectSelection => HotkeyEvent::CorrectSelection,
                ChordAction::OpenComposer => HotkeyEvent::OpenComposer,
                ChordAction::VoiceCommand => HotkeyEvent::VoiceCommandDown,
                ChordAction::Dictate => HotkeyEvent::ToggleDictation,
            });
            d.menu_mask = spec.win || spec.alt;
            d.swallow = !is_modifier_vk(spec.vk);
            mods.latch_swallow(vk);
            return d;
        }
        if mods.chord_latch & bit != 0 && !mods.still_holding_spec(spec) {
            mods.chord_latch &= !bit;
            if *action == ChordAction::VoiceCommand {
                d.events.push(HotkeyEvent::VoiceCommandUp);
            }
        }
    }

    // Another key joined a held modifier-only dictation chord (Right Ctrl + C): it is a shortcut for
    // another app, so drop the recording and let the key through. No restart until all modifiers are up.
    if is_down
        && mods.ptt_combo_latched
        && cfg.ptt_modifier_only()
        && !is_modifier_vk(vk)
        && vk != cfg.cancel_vk
    {
        mods.ptt_combo_latched = false;
        mods.ptt_blocked_by_chord = true;
        d.events.push(HotkeyEvent::PttSuperseded);
        return d;
    }

    // Push-to-Talk trigger
    if is_down && !mods.ptt_combo_latched && !mods.ptt_blocked_by_chord && mods.matches_spec(&cfg.ptt_spec, vk, true) {
        mods.ptt_combo_latched = true;
        d.menu_mask = cfg.ptt_spec.win || cfg.ptt_spec.alt;
        d.events.push(HotkeyEvent::PttDown);
        // Suppress non-modifier trigger key so Space/F8 doesn't leak into active document;
        // pass pure modifiers through so standard Ctrl+C / Win+D keep working if pressed differently.
        d.swallow = cfg.ptt_spec.vk != 0 && !matches!(cfg.ptt_spec.vk, 0xA0..=0xA5 | 0x5B | 0x5C);
        if d.swallow {
            mods.latch_swallow(vk);
        }
    } else if mods.ptt_combo_latched && !mods.still_holding_spec(&cfg.ptt_spec) {
        mods.ptt_combo_latched = false;
        d.events.push(HotkeyEvent::PttUp);
    }
    d
}

/// What the mouse hook does with one press or release of a bindable button.
#[derive(Debug, Default, PartialEq, Eq)]
struct MouseDecision {
    events: Vec<HotkeyEvent>,
    swallow: bool,
}

/// The mouse matcher (pure so it can be unit-tested): a bound button sends the same events its keyboard chord
/// would, so the coordinator handles it identically. Only a bound button is swallowed — an unbound one, and
/// every left/right click, is never seen here.
fn decide_mouse(cfg: &HookSharedConfig, button: MouseButton, is_down: bool) -> MouseDecision {
    let Some(action) = cfg.mouse.iter().find(|(b, _)| *b == button).map(|(_, a)| *a) else {
        return MouseDecision::default();
    };
    let event = match (action, is_down) {
        (ChordAction::VoiceCommand, true) => Some(HotkeyEvent::VoiceCommandDown),
        (ChordAction::VoiceCommand, false) => Some(HotkeyEvent::VoiceCommandUp),
        (_, false) => None,
        (ChordAction::CorrectSelection, true) => Some(HotkeyEvent::CorrectSelection),
        (ChordAction::OpenComposer, true) => Some(HotkeyEvent::OpenComposer),
        (ChordAction::Dictate, true) => Some(HotkeyEvent::ToggleDictation),
    };
    // The release is swallowed too: the application must never see a lone button-up.
    MouseDecision { events: event.into_iter().collect(), swallow: true }
}

/// Pure VK-matching state tracker (unit-testable across platforms).
#[derive(Debug, Default, Clone)]
pub struct ModifierState {
    pub lctrl: bool,
    pub rctrl: bool,
    pub lalt: bool,
    pub ralt: bool,
    pub lshift: bool,
    pub rshift: bool,
    pub lwin: bool,
    pub rwin: bool,
    pub main_vk_down: u16,
    pub ptt_combo_latched: bool,
    /// One bit per extra chord that is currently held.
    pub chord_latch: u8,
    /// An extra chord fired: dictation cannot start until every modifier is released, so a held
    /// Ctrl+Win left over from Ctrl+Win+G never starts a recording on key repeat.
    pub ptt_blocked_by_chord: bool,
    /// Non-modifier trigger keys whose key-down was swallowed and whose release is still pending.
    pub swallowed: Vec<u16>,
}

impl ModifierState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Re-reads the eight modifier keys (and held trigger keys) from the OS before each event, because the
    /// hook never sees the release after Win+L, Ctrl+Alt+Del or a UAC prompt. `current_vk` is the key of the
    /// event about to be applied, whose own state is left to the event.
    pub fn resync(&mut self, down: impl Fn(u16) -> bool, current_vk: u16) {
        self.lctrl = down(0xA2);
        self.rctrl = down(0xA3);
        self.lalt = down(0xA4);
        self.ralt = down(0xA5);
        self.lshift = down(0xA0);
        self.rshift = down(0xA1);
        self.lwin = down(0x5B);
        self.rwin = down(0x5C);
        if self.main_vk_down != 0 && self.main_vk_down != current_vk && !down(self.main_vk_down) {
            self.main_vk_down = 0;
        }
        self.swallowed.retain(|&k| k == current_vk || down(k));
    }

    fn latch_swallow(&mut self, vk: u16) {
        if !is_modifier_vk(vk) && !self.swallowed.contains(&vk) {
            self.swallowed.push(vk);
        }
    }

    pub fn any_ctrl(&self) -> bool {
        self.lctrl || self.rctrl
    }
    pub fn any_alt(&self) -> bool {
        self.lalt || self.ralt
    }
    pub fn any_shift(&self) -> bool {
        self.lshift || self.rshift
    }
    pub fn any_win(&self) -> bool {
        self.lwin || self.rwin
    }

    pub fn update_modifier(&mut self, vk: u16, is_down: bool) -> bool {
        match vk {
            0xA2 | 0x11 => self.lctrl = is_down,
            0xA3 => self.rctrl = is_down,
            0xA4 | 0x12 => self.lalt = is_down,
            0xA5 => self.ralt = is_down,
            0xA0 | 0x10 => self.lshift = is_down,
            0xA1 => self.rshift = is_down,
            0x5B => self.lwin = is_down,
            0x5C => self.rwin = is_down,
            _ => return false,
        }
        true
    }

    pub fn matches_spec(&self, spec: &HotkeySpec, current_vk: u16, is_down: bool) -> bool {
        if spec.ctrl && !self.any_ctrl() {
            return false;
        }
        if spec.alt && !self.any_alt() {
            return false;
        }
        if spec.shift && !self.any_shift() {
            return false;
        }
        if spec.win && !self.any_win() {
            return false;
        }
        if spec.vk == 0 {
            // Modifier-only chord (e.g. Ctrl + Win)
            spec.ctrl || spec.alt || spec.shift || spec.win
        } else if matches!(spec.vk, 0xA0..=0xA5 | 0x5B | 0x5C) {
            // Single specific modifier key (e.g. Right Ctrl = 0xA3)
            is_down && current_vk == spec.vk
        } else {
            // Key chord: exact modifiers, so Ctrl+Shift+Win+G does not fire Ctrl+Win+G.
            is_down
                && current_vk == spec.vk
                && (spec.ctrl || !self.any_ctrl())
                && (spec.alt || !self.any_alt())
                && (spec.shift || !self.any_shift())
                && (spec.win || !self.any_win())
        }
    }

    pub fn still_holding_spec(&self, spec: &HotkeySpec) -> bool {
        if spec.ctrl && !self.any_ctrl() {
            return false;
        }
        if spec.alt && !self.any_alt() {
            return false;
        }
        if spec.shift && !self.any_shift() {
            return false;
        }
        if spec.win && !self.any_win() {
            return false;
        }
        match spec.vk {
            0 => spec.ctrl || spec.alt || spec.shift || spec.win,
            0xA2 => self.lctrl,
            0xA3 => self.rctrl,
            0xA4 => self.lalt,
            0xA5 => self.ralt,
            0xA0 => self.lshift,
            0xA1 => self.rshift,
            0x5B => self.lwin,
            0x5C => self.rwin,
            vk => self.main_vk_down == vk,
        }
    }
}

pub struct GlobalHotkeyEngine {
    #[cfg(windows)]
    hook_thread_id: u32,
    shared: Arc<Mutex<HookSharedConfig>>,
}

#[cfg(windows)]
mod win_impl {
    use super::*;
    use once_cell::sync::OnceCell;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
        SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG,
        MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN,
        WM_MBUTTONUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN, WM_XBUTTONUP,
    };

    pub const WM_ENABLE_HOOK: u32 = WM_USER + 10;
    pub const WM_DISABLE_HOOK: u32 = WM_USER + 11;
    pub const WM_RESET_MODS: u32 = WM_USER + 12;
    const VK_MENU_MASK: u16 = 0xE8;

    struct GlobalHookCtx {
        tx: Sender<HotkeyEvent>,
        shared: Arc<Mutex<HookSharedConfig>>,
        mods: Mutex<ModifierState>,
    }

    static HOOK_CTX: OnceCell<GlobalHookCtx> = OnceCell::new();

    fn key_physically_down(vk: u16) -> bool {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
    }

    unsafe fn send_menu_mask() {
        let mut inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_MENU_MASK,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_MENU_MASK,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                    },
                },
            },
        ];
        let _ = SendInput(2, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
    }

    /// True when the foreground window is a standard dialog (`#32770`) owned by KeyReed itself.
    fn own_dialog_in_foreground() -> bool {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
        };
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, &mut pid);
            if pid != GetCurrentProcessId() {
                return false;
            }
            let mut buf = [0u16; 16];
            let n = GetClassNameW(fg, buf.as_mut_ptr(), buf.len() as i32);
            n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "#32770"
        }
    }

    unsafe extern "system" fn ll_keyboard_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 && lparam != 0 {
            let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
            // Ignore our own synthetic SendInput keystrokes
            if kb.dwExtraInfo == KEYREED_SYNTHETIC_TAG {
                return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
            }
            if let Some(ctx) = HOOK_CTX.get() {
                let msg = wparam as u32;
                let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                let vk = kb.vkCode as u16;

                let cfg = ctx.shared.lock();
                let mut mods = ctx.mods.lock();

                // While a modifier-only dictation chord is held every key is relevant (see header).
                let relevant = cfg.is_relevant(vk) || (mods.ptt_combo_latched && cfg.ptt_modifier_only());
                if relevant && (is_down || is_up) {
                    mods.resync(key_physically_down, vk);

                    // Emergency Escape cancel while active — except when KeyReed's own dialog
                    // (e.g. "Ask before inserting") is in front: Escape must reach it as Cancel.
                    if is_down
                        && vk == cfg.cancel_vk
                        && cfg.active_recording_or_busy.load(Ordering::Relaxed)
                        && !own_dialog_in_foreground()
                    {
                        mods.ptt_combo_latched = false;
                        let _ = ctx.tx.try_send(HotkeyEvent::EmergencyCancel);
                        return 1; // Suppress Escape from closing user's foreground dialog during cancel!
                    }

                    // Shortcut capture in Settings: every key goes to the capture field untouched. Only the modifier
                    // state is kept current, and releasing Win must not open Start.
                    if CAPTURE_ACTIVE.load(Ordering::Relaxed) {
                        mods.update_modifier(vk, is_down);
                        if is_up && matches!(vk, 0x5B | 0x5C) {
                            send_menu_mask();
                        }
                        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
                    }

                    let d = decide(&cfg, &mut mods, vk, is_down);
                    if d.menu_mask {
                        send_menu_mask();
                    }
                    for ev in d.events {
                        match ev {
                            HotkeyEvent::PttDown => crate::security::timing_mark_async("hook_ptt_down"),
                            HotkeyEvent::PttUp => crate::security::timing_mark_async("hook_ptt_up"),
                            _ => {}
                        }
                        let _ = ctx.tx.try_send(ev);
                    }
                    if d.swallow {
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// The bindable button of one hook message, and whether it went down. Everything else (moves, the wheel,
    /// left and right clicks) is not ours.
    pub(super) fn mouse_button(msg: u32, mouse_data: u32) -> Option<(MouseButton, bool)> {
        let down = msg == WM_MBUTTONDOWN || msg == WM_XBUTTONDOWN;
        match msg {
            WM_MBUTTONDOWN | WM_MBUTTONUP => Some((MouseButton::Middle, down)),
            WM_XBUTTONDOWN | WM_XBUTTONUP => match (mouse_data >> 16) as u16 {
                1 => Some((MouseButton::Back, down)),
                2 => Some((MouseButton::Forward, down)),
                _ => None,
            },
            _ => None,
        }
    }

    unsafe extern "system" fn ll_mouse_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // The early test keeps moves and the wheel off every lock and every keystroke path.
        if code >= 0 && lparam != 0 && matches!(wparam as u32, WM_MBUTTONDOWN | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP) {
            let ms = &*(lparam as *const MSLLHOOKSTRUCT);
            if let Some((button, is_down)) = mouse_button(wparam as u32, ms.mouseData) {
                if let Some(ctx) = HOOK_CTX.get().filter(|_| !CAPTURE_ACTIVE.load(Ordering::Relaxed)) {
                    let d = decide_mouse(&ctx.shared.lock(), button, is_down);
                    for ev in d.events {
                        let _ = ctx.tx.try_send(ev);
                    }
                    if d.swallow {
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    pub fn spawn_hook_thread(
        tx: Sender<HotkeyEvent>,
        shared: Arc<Mutex<HookSharedConfig>>,
        initially_enabled: bool,
    ) -> anyhow::Result<u32> {
        let _ = HOOK_CTX.set(GlobalHookCtx {
            tx,
            shared,
            mods: Mutex::new(ModifierState::default()),
        });

        let (tid_tx, tid_rx) = crossbeam_channel::bounded::<u32>(1);
        std::thread::Builder::new()
            .name("keyreed-ll-hook".to_string())
            .spawn(move || unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
                let mut dummy: MSG = std::mem::zeroed();
                // Force creation of thread message queue before signaling thread ID
                let _ = PeekMessageW(&mut dummy, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                let tid = GetCurrentThreadId();
                let _ = tid_tx.send(tid);

                let mut hhook: HHOOK = if initially_enabled {
                    let h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), std::ptr::null_mut(), 0);
                    crate::security::timing_mark_async(if h.is_null() { "hook_install_failed" } else { "hook_installed" });
                    h
                } else {
                    crate::security::timing_mark_async("hook_initially_disabled");
                    std::ptr::null_mut()
                };
                let mut hmouse: HHOOK = if initially_enabled {
                    SetWindowsHookExW(WH_MOUSE_LL, Some(ll_mouse_proc), std::ptr::null_mut(), 0)
                } else {
                    std::ptr::null_mut()
                };

                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    match msg.message {
                        WM_ENABLE_HOOK => {
                            if hhook.is_null() {
                                hhook = SetWindowsHookExW(
                                    WH_KEYBOARD_LL,
                                    Some(ll_keyboard_proc),
                                    std::ptr::null_mut(),
                                    0,
                                );
                            }
                            if hmouse.is_null() {
                                hmouse = SetWindowsHookExW(WH_MOUSE_LL, Some(ll_mouse_proc), std::ptr::null_mut(), 0);
                            }
                        }
                        WM_DISABLE_HOOK => {
                            if !hhook.is_null() {
                                let _ = UnhookWindowsHookEx(hhook);
                                hhook = std::ptr::null_mut();
                            }
                            if !hmouse.is_null() {
                                let _ = UnhookWindowsHookEx(hmouse);
                                hmouse = std::ptr::null_mut();
                            }
                            if let Some(ctx) = HOOK_CTX.get() {
                                ctx.mods.lock().reset();
                            }
                        }
                        WM_RESET_MODS => {
                            if let Some(ctx) = HOOK_CTX.get() {
                                ctx.mods.lock().reset();
                            }
                        }
                        WM_QUIT => break,
                        _ => {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }
                if !hhook.is_null() {
                    let _ = UnhookWindowsHookEx(hhook);
                }
                if !hmouse.is_null() {
                    let _ = UnhookWindowsHookEx(hmouse);
                }
            })?;

        let tid = tid_rx.recv()?;
        Ok(tid)
    }

    pub fn post_hook_cmd(tid: u32, msg: u32) {
        unsafe {
            let _ = PostThreadMessageW(tid, msg, 0, 0);
        }
    }
}

impl GlobalHotkeyEngine {
    pub fn start(
        tx: Sender<HotkeyEvent>,
        ptt_spec: HotkeySpec,
        mode_cycle_spec: HotkeySpec,
        active_recording_or_busy: Arc<AtomicBool>,
        initially_enabled: bool,
    ) -> anyhow::Result<Self> {
        let shared = Arc::new(Mutex::new(HookSharedConfig {
            ptt_spec,
            mode_cycle_spec,
            chords: Vec::new(),
            mouse: Vec::new(),
            cancel_vk: 0x1B, // VK_ESCAPE
            active_recording_or_busy,
        }));

        #[cfg(windows)]
        {
            let tid = win_impl::spawn_hook_thread(tx, Arc::clone(&shared), initially_enabled)?;
            Ok(Self {
                hook_thread_id: tid,
                shared,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (tx, initially_enabled);
            Ok(Self { shared })
        }
    }

    pub fn update_shortcuts(&self, ptt_spec: HotkeySpec, mode_cycle_spec: HotkeySpec) {
        let mut guard = self.shared.lock();
        guard.ptt_spec = ptt_spec;
        guard.mode_cycle_spec = mode_cycle_spec;
    }

    /// Replaces the extra chords (Quick Fix, Composer, voice command); at most `MAX_EXTRA_CHORDS`.
    pub fn set_extra_chords(&self, mut chords: Vec<(HotkeySpec, ChordAction)>) {
        chords.truncate(MAX_EXTRA_CHORDS);
        self.shared.lock().chords = chords;
    }

    /// Replaces the mouse bindings: only bound buttons are matched, everything else is left alone.
    pub fn set_mouse_bindings(&self, bindings: Vec<(MouseButton, ChordAction)>) {
        self.shared.lock().mouse = bindings;
    }

    pub fn set_enabled(&self, enabled: bool) {
        #[cfg(windows)]
        {
            let cmd = if enabled {
                win_impl::WM_ENABLE_HOOK
            } else {
                win_impl::WM_DISABLE_HOOK
            };
            win_impl::post_hook_cmd(self.hook_thread_id, cmd);
        }
        #[cfg(not(windows))]
        let _ = enabled;
    }

    pub fn reset_modifiers(&self) {
        #[cfg(windows)]
        win_impl::post_hook_cmd(self.hook_thread_id, win_impl::WM_RESET_MODS);
    }
}

/// Checks via `GetAsyncKeyState` whether the physical key(s) in `spec` are currently held down.
/// Used for fast lost-keyup recovery when Windows swallows a `WM_KEYUP` across focus/elevation boundaries.
pub fn is_spec_physically_down(spec: &HotkeySpec) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let down = |vk: i32| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0;

        if spec.ctrl && !(down(0xA2) || down(0xA3) || down(0x11)) {
            return false;
        }
        if spec.alt && !(down(0xA4) || down(0xA5) || down(0x12)) {
            return false;
        }
        if spec.shift && !(down(0xA0) || down(0xA1) || down(0x10)) {
            return false;
        }
        if spec.win && !(down(0x5B) || down(0x5C)) {
            return false;
        }
        if spec.vk != 0 {
            return down(spec.vk as i32);
        }
        spec.ctrl || spec.alt || spec.shift || spec.win
    }
    #[cfg(not(windows))]
    {
        let _ = spec;
        false
    }
}

#[cfg(windows)]
impl Drop for GlobalHotkeyEngine {
    fn drop(&mut self) {
        win_impl::post_hook_cmd(self.hook_thread_id, windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_ctrl_and_ctrl_win_match_and_release_cleanly() {
        let mut m = ModifierState::default();
        let rctrl = HotkeySpec::right_ctrl();
        m.update_modifier(0xA3, true);
        assert!(m.matches_spec(&rctrl, 0xA3, true));
        assert!(m.still_holding_spec(&rctrl));
        m.update_modifier(0xA3, false);
        assert!(!m.still_holding_spec(&rctrl));

        let cw = HotkeySpec::default(); // Ctrl + Win
        m.update_modifier(0xA2, true);
        assert!(!m.matches_spec(&cw, 0xA2, true));
        m.update_modifier(0x5B, true);
        assert!(m.matches_spec(&cw, 0x5B, true));
        assert!(m.still_holding_spec(&cw));
        m.update_modifier(0x5B, false);
        assert!(!m.still_holding_spec(&cw));
    }

    const G: u16 = 0x47;
    const K: u16 = 0x4B;
    const V: u16 = 0x56;

    fn chord(vk: u16) -> HotkeySpec {
        HotkeySpec { ctrl: true, alt: false, shift: false, win: true, vk, display_name: String::new() }
    }

    fn cfg_with_chords() -> HookSharedConfig {
        HookSharedConfig {
            ptt_spec: HotkeySpec::default(), // Ctrl + Win, modifier-only
            mode_cycle_spec: HotkeySpec { ctrl: true, alt: true, shift: false, win: false, vk: 0x4D, display_name: String::new() },
            chords: vec![
                (chord(G), ChordAction::CorrectSelection),
                (chord(K), ChordAction::OpenComposer),
                (chord(V), ChordAction::VoiceCommand),
            ],
            mouse: Vec::new(),
            cancel_vk: 0x1B,
            active_recording_or_busy: Arc::new(AtomicBool::new(false)),
        }
    }

    fn press(cfg: &HookSharedConfig, m: &mut ModifierState, keys: &[(u16, bool)]) -> Vec<HotkeyEvent> {
        keys.iter().flat_map(|&(vk, down)| decide(cfg, m, vk, down).events).collect()
    }

    #[test]
    fn extra_chord_supersedes_modifier_only_dictation_prefix() {
        let cfg = cfg_with_chords();
        let mut m = ModifierState::default();
        // Ctrl, Win (dictation starts: its chord is a prefix), then G.
        let ev = press(&cfg, &mut m, &[(0xA2, true), (0x5B, true)]);
        assert_eq!(ev, [HotkeyEvent::PttDown]);
        let d = decide(&cfg, &mut m, G, true);
        assert_eq!(d.events, [HotkeyEvent::PttSuperseded, HotkeyEvent::CorrectSelection]);
        assert!(d.swallow && d.menu_mask);
        // G released, Ctrl+Win still held and repeating: dictation must not start again.
        assert!(press(&cfg, &mut m, &[(G, false), (0x5B, true), (0xA2, true)]).is_empty());
        // Everything released, then Ctrl+Win alone: dictation works again.
        assert!(press(&cfg, &mut m, &[(0x5B, false), (0xA2, false)]).is_empty());
        assert_eq!(press(&cfg, &mut m, &[(0xA2, true), (0x5B, true)]), [HotkeyEvent::PttDown]);
    }

    #[test]
    fn each_chord_has_its_own_latch_and_voice_command_is_held() {
        let cfg = cfg_with_chords();
        let mut m = ModifierState::default();
        press(&cfg, &mut m, &[(0xA2, true), (0x5B, true)]);
        assert_eq!(press(&cfg, &mut m, &[(K, true)]), [HotkeyEvent::PttSuperseded, HotkeyEvent::OpenComposer]);
        // Key repeat of K while held fires nothing new.
        assert!(press(&cfg, &mut m, &[(K, true)]).is_empty());
        assert!(press(&cfg, &mut m, &[(K, false)]).is_empty());
        assert_eq!(press(&cfg, &mut m, &[(V, true)]), [HotkeyEvent::VoiceCommandDown]);
        assert_eq!(press(&cfg, &mut m, &[(V, false)]), [HotkeyEvent::VoiceCommandUp]);
    }

    #[test]
    fn unconfigured_keys_are_not_relevant() {
        let cfg = cfg_with_chords();
        for vk in [G, K, V, 0x1B, 0x4D, 0xA2, 0x5B] {
            assert!(cfg.is_relevant(vk), "{vk:#x}");
        }
        for vk in [0x41u16, 0x20, 0x0D, 0x70, 0x31] {
            assert!(!cfg.is_relevant(vk), "{vk:#x}");
        }
        // A chord without a trigger key adds nothing.
        let mut c = cfg_with_chords();
        c.chords = vec![(HotkeySpec { vk: 0, ..chord(0) }, ChordAction::OpenComposer)];
        assert!(!c.is_relevant(0));
    }

    #[test]
    fn dictation_chord_still_works_without_extra_chords() {
        let mut cfg = cfg_with_chords();
        cfg.chords.clear();
        let mut m = ModifierState::default();
        assert_eq!(press(&cfg, &mut m, &[(0xA2, true), (0x5B, true)]), [HotkeyEvent::PttDown]);
        assert_eq!(press(&cfg, &mut m, &[(0x5B, false)]), [HotkeyEvent::PttUp]);
    }

    fn key_cfg(ptt: HotkeySpec) -> HookSharedConfig {
        let mut cfg = cfg_with_chords();
        cfg.chords.clear();
        cfg.ptt_spec = ptt;
        cfg
    }

    const F8: u16 = 0x77;

    #[test]
    fn key_based_trigger_swallows_repeats_and_release() {
        let f8 = HotkeySpec { ctrl: false, alt: false, shift: false, win: false, vk: F8, display_name: String::new() };
        let cfg = key_cfg(f8);
        let mut m = ModifierState::default();
        let d = decide(&cfg, &mut m, F8, true);
        assert_eq!(d.events, [HotkeyEvent::PttDown]);
        assert!(d.swallow);
        let rep = decide(&cfg, &mut m, F8, true);
        assert!(rep.swallow && rep.events.is_empty());
        let up = decide(&cfg, &mut m, F8, false);
        assert!(up.swallow);
        assert_eq!(up.events, [HotkeyEvent::PttUp]);
        // Latch is gone: an unrelated later release of F8 is not swallowed.
        assert!(!decide(&cfg, &mut m, F8, false).swallow);
    }

    #[test]
    fn chord_key_up_and_repeats_are_swallowed_but_modifiers_are_not() {
        let cfg = cfg_with_chords();
        let mut m = ModifierState::default();
        press(&cfg, &mut m, &[(0xA2, true), (0x5B, true)]);
        assert!(decide(&cfg, &mut m, K, true).swallow);
        assert!(decide(&cfg, &mut m, K, true).swallow);
        assert!(decide(&cfg, &mut m, K, false).swallow);
        assert!(!decide(&cfg, &mut m, 0xA2, false).swallow);
    }

    #[test]
    fn key_chords_need_exactly_their_modifiers() {
        let cfg = cfg_with_chords();
        let mut m = ModifierState::default();
        // Ctrl+Shift+Win+G must not fire Ctrl+Win+G.
        let ev = press(&cfg, &mut m, &[(0xA2, true), (0xA0, true), (0x5B, true), (G, true)]);
        assert!(!ev.contains(&HotkeyEvent::CorrectSelection));
        m.reset();
        // Ctrl+Alt+M (mode cycle) is exact too: with Shift added it does nothing.
        assert!(press(&cfg, &mut m, &[(0xA2, true), (0xA4, true), (0xA0, true), (0x4D, true)]).is_empty());
    }

    #[test]
    fn mode_cycle_does_not_repeat_while_held() {
        let cfg = cfg_with_chords();
        let mut m = ModifierState::default();
        assert_eq!(press(&cfg, &mut m, &[(0xA2, true), (0xA4, true), (0x4D, true)]), [HotkeyEvent::CycleMode]);
        assert!(press(&cfg, &mut m, &[(0x4D, true), (0x4D, true)]).is_empty());
    }

    #[test]
    fn another_key_with_modifier_only_dictation_drops_the_recording() {
        let mut cfg = key_cfg(HotkeySpec::right_ctrl());
        assert!(cfg.ptt_modifier_only());
        let mut m = ModifierState::default();
        assert_eq!(press(&cfg, &mut m, &[(0xA3, true)]), [HotkeyEvent::PttDown]);
        // Right Ctrl + C: dictation is dropped and C is not swallowed.
        let d = decide(&cfg, &mut m, 0x43, true);
        assert_eq!(d.events, [HotkeyEvent::PttSuperseded]);
        assert!(!d.swallow);
        // Right Ctrl auto-repeat must not restart it until everything is released.
        assert!(press(&cfg, &mut m, &[(0x43, false), (0xA3, true)]).is_empty());
        assert!(press(&cfg, &mut m, &[(0xA3, false)]).is_empty());
        assert_eq!(press(&cfg, &mut m, &[(0xA3, true)]), [HotkeyEvent::PttDown]);
        // Escape is the cancel key, not a chord partner.
        assert!(decide(&cfg, &mut m, 0x1B, true).events.is_empty());
        // A key-based dictation shortcut has no such interrupt.
        cfg.ptt_spec = HotkeySpec { ctrl: false, alt: false, shift: false, win: false, vk: F8, display_name: String::new() };
        assert!(!cfg.ptt_modifier_only());
    }

    /// The owner's setup end to end, matcher and controller: hold Right Ctrl, speak, let go.
    /// A recording the pill started must also end on that release instead of recording until the cap.
    #[test]
    fn right_ctrl_drives_the_controller_and_ends_a_pill_session() {
        use vi_core::ptt::{PttAction, PttController};
        use vi_core::types::ActivationMode;
        let cfg = key_cfg(HotkeySpec::right_ctrl());
        let mut m = ModifierState::default();
        let mut ptt = PttController::new(ActivationMode::HoldToTalk, 110, 1_800_000);

        // A pill click starts the session; the dictation key then has to end it.
        assert_eq!(ptt.on_floating_mic_click(0), PttAction::StartRecording);
        assert_eq!(press(&cfg, &mut m, &[(0xA3, true)]), [HotkeyEvent::PttDown]);
        assert_eq!(ptt.on_shortcut_down(1_000), PttAction::None);
        assert_eq!(press(&cfg, &mut m, &[(0xA3, false)]), [HotkeyEvent::PttUp]);
        assert_eq!(ptt.on_shortcut_up(1_500), PttAction::StopAndProcess);
        ptt.transition_to_idle();

        // And the plain hold still starts and stops one recording.
        assert_eq!(press(&cfg, &mut m, &[(0xA3, true)]), [HotkeyEvent::PttDown]);
        assert_eq!(ptt.on_shortcut_down(2_000), PttAction::StartRecording);
        assert_eq!(press(&cfg, &mut m, &[(0xA3, false)]), [HotkeyEvent::PttUp]);
        assert_eq!(ptt.on_shortcut_up(2_500), PttAction::StopAndProcess);
    }

    #[test]
    fn resync_clears_stale_modifiers_and_trigger_latches() {
        let mut m = ModifierState::default();
        m.update_modifier(0x5B, true);
        m.update_modifier(0xA2, true);
        m.main_vk_down = F8;
        m.swallowed = vec![F8, G];
        // Win+L swallowed the releases: only Ctrl is physically down now.
        m.resync(|vk| vk == 0xA2, 0x41);
        assert!(m.lctrl && !m.lwin && !m.any_win());
        assert_eq!(m.main_vk_down, 0);
        assert!(m.swallowed.is_empty());
        // The current event's own key is left to the event.
        m.swallowed = vec![G];
        m.resync(|_| false, G);
        assert_eq!(m.swallowed, [G]);
    }

    fn cfg_with_mouse(bindings: &[(MouseButton, ChordAction)]) -> HookSharedConfig {
        let mut cfg = cfg_with_chords();
        cfg.mouse = bindings.to_vec();
        cfg
    }

    #[test]
    fn a_bound_mouse_button_fires_what_its_chord_would() {
        let cfg = cfg_with_mouse(&[
            (MouseButton::Middle, ChordAction::Dictate),
            (MouseButton::Back, ChordAction::CorrectSelection),
            (MouseButton::Forward, ChordAction::VoiceCommand),
        ]);
        assert_eq!(decide_mouse(&cfg, MouseButton::Middle, true).events, [HotkeyEvent::ToggleDictation]);
        assert_eq!(decide_mouse(&cfg, MouseButton::Back, true).events, [HotkeyEvent::CorrectSelection]);
        // A voice command is held like its chord: the release ends it.
        assert_eq!(decide_mouse(&cfg, MouseButton::Forward, true).events, [HotkeyEvent::VoiceCommandDown]);
        assert_eq!(decide_mouse(&cfg, MouseButton::Forward, false).events, [HotkeyEvent::VoiceCommandUp]);
        // The press-and-release actions fire once, on the press.
        assert!(decide_mouse(&cfg, MouseButton::Middle, false).events.is_empty());
        assert!(decide_mouse(&cfg, MouseButton::Back, false).events.is_empty());
    }

    #[test]
    fn only_a_bound_button_is_swallowed() {
        let cfg = cfg_with_mouse(&[(MouseButton::Back, ChordAction::CorrectSelection)]);
        assert!(decide_mouse(&cfg, MouseButton::Back, true).swallow, "it fires, so the app must not get the click");
        assert!(decide_mouse(&cfg, MouseButton::Back, false).swallow, "nor a lone release");
        assert!(!decide_mouse(&cfg, MouseButton::Middle, true).swallow, "unbound");
        assert!(!decide_mouse(&cfg, MouseButton::Forward, false).swallow);
        // Nothing bound at all: an old config with no mouse keys leaves every button alone.
        assert!(!decide_mouse(&cfg_with_mouse(&[]), MouseButton::Middle, true).swallow);
    }

    /// Left and right clicks, moves and the wheel never reach the matcher.
    #[test]
    #[cfg(windows)]
    fn other_mouse_events_are_not_ours() {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        for msg in [WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_MOUSEWHEEL, WM_MOUSEHWHEEL] {
            assert!(super::win_impl::mouse_button(msg, 0).is_none(), "{msg:#x}");
        }
        assert_eq!(super::win_impl::mouse_button(WM_MBUTTONDOWN, 0), Some((MouseButton::Middle, true)));
        assert_eq!(super::win_impl::mouse_button(WM_XBUTTONDOWN, 1 << 16), Some((MouseButton::Back, true)));
        assert_eq!(super::win_impl::mouse_button(WM_XBUTTONUP, 2 << 16), Some((MouseButton::Forward, false)));
        assert!(super::win_impl::mouse_button(WM_XBUTTONDOWN, 3 << 16).is_none(), "XBUTTON3 is not ours");
    }
}
