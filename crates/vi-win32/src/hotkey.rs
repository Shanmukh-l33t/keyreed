//! Dedicated `WH_KEYBOARD_LL` global Push-to-Talk thread (Master Prompt Section 3 & 12).
//!
//! Architecture & Security rules:
//! - Runs on its own high-priority message-pump thread (`THREAD_PRIORITY_ABOVE_NORMAL`) so UI rendering or
//!   model loading NEVER delays `LowLevelKeyboardProc` past Windows' `LowLevelHooksTimeout` (~300 ms).
//! - Inspects ONLY modifier VK codes, `VK_ESCAPE`, and the configured shortcut VKs: ZERO keystroke logging or buffering.
//! - Injects unassigned `VK_MENU_MASK` (`0xE8`) when `Win` or `Alt` chord triggers so releasing `Win`/`Alt`
//!   never pops open the Windows Start Menu or app menu bar!
//! - Dynamically installs and removes (`UnhookWindowsHookEx`) the hook when Push-to-Talk is independently toggled!

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
}

#[derive(Debug, Clone)]
struct HookSharedConfig {
    ptt_spec: HotkeySpec,
    mode_cycle_spec: HotkeySpec,
    cancel_vk: u16,
    active_recording_or_busy: Arc<AtomicBool>,
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
}

impl ModifierState {
    pub fn reset(&mut self) {
        *self = Self::default();
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
            is_down && current_vk == spec.vk
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
        PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
        WM_USER,
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

                let cfg = ctx.shared.lock().clone();
                // Privacy / anti-keylogger invariant: ONLY inspect modifier VKs, cancel VK, and configured trigger VKs!
                let is_relevant = matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
                    || vk == cfg.cancel_vk
                    || (cfg.ptt_spec.vk != 0 && vk == cfg.ptt_spec.vk)
                    || (cfg.mode_cycle_spec.vk != 0 && vk == cfg.mode_cycle_spec.vk);

                if is_relevant && (is_down || is_up) {
                    let mut mods = ctx.mods.lock();
                    let was_mod = mods.update_modifier(vk, is_down);
                    if !was_mod {
                        if is_down {
                            mods.main_vk_down = vk;
                        } else if mods.main_vk_down == vk {
                            mods.main_vk_down = 0;
                        }
                    }

                    // Emergency Escape cancel while active
                    if is_down
                        && vk == cfg.cancel_vk
                        && cfg.active_recording_or_busy.load(Ordering::Relaxed)
                    {
                        mods.ptt_combo_latched = false;
                        let _ = ctx.tx.try_send(HotkeyEvent::EmergencyCancel);
                        return 1; // Suppress Escape from closing user's foreground dialog during cancel!
                    }

                    // Mode cycle chord
                    if is_down
                        && !mods.ptt_combo_latched
                        && mods.matches_spec(&cfg.mode_cycle_spec, vk, true)
                    {
                        let _ = ctx.tx.try_send(HotkeyEvent::CycleMode);
                        return 1;
                    }

                    // Push-to-Talk trigger
                    if is_down && !mods.ptt_combo_latched && mods.matches_spec(&cfg.ptt_spec, vk, true)
                    {
                        mods.ptt_combo_latched = true;
                        if cfg.ptt_spec.win || cfg.ptt_spec.alt {
                            send_menu_mask();
                        }
                        let _ = ctx.tx.try_send(HotkeyEvent::PttDown);
                        crate::security::timing_mark("hook_ptt_down");
                        // Suppress non-modifier trigger key so Space/F8 doesn't leak into active document;
                        // pass pure modifiers through so standard Ctrl+C / Win+D keep working if pressed differently.
                        if cfg.ptt_spec.vk != 0 && !matches!(cfg.ptt_spec.vk, 0xA0..=0xA5 | 0x5B | 0x5C) {
                            return 1;
                        }
                    } else if mods.ptt_combo_latched && !mods.still_holding_spec(&cfg.ptt_spec) {
                        mods.ptt_combo_latched = false;
                        let _ = ctx.tx.try_send(HotkeyEvent::PttUp);
                        crate::security::timing_mark("hook_ptt_up");
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
                    crate::security::timing_mark(if h.is_null() { "hook_install_failed" } else { "hook_installed" });
                    h
                } else {
                    crate::security::timing_mark("hook_initially_disabled");
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
                        }
                        WM_DISABLE_HOOK => {
                            if !hhook.is_null() {
                                let _ = UnhookWindowsHookEx(hhook);
                                hhook = std::ptr::null_mut();
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
}
