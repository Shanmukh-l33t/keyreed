//! First-class Automatic Text Insertion engine via `SendInput` (`KEYEVENTF_UNICODE`) with mandatory
//! security, focus-retention, and reliability fallbacks to Clipboard (Master Directive Section 6).

use crate::clipboard::write_clipboard_unicode;
use crate::security::KEYREED_SYNTHETIC_TAG;
use anyhow::Result;
use vi_core::{AppCategory, ForegroundContext};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectTypeOutcome {
    TypedDirectly { code_units_sent: usize },
    FallbackToClipboard { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskInsertDecision {
    InsertIntoTarget,
    CopyToClipboardOnly,
    Cancel,
}

#[cfg(windows)]
pub fn force_foreground_window(target_hwnd: isize) -> bool {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    unsafe {
        let hwnd = target_hwnd as HWND;
        if hwnd.is_null() || IsWindow(hwnd) == 0 {
            return false;
        }
        let fg = GetForegroundWindow();
        if fg == hwnd {
            return true;
        }
        let fg_tid = if !fg.is_null() {
            GetWindowThreadProcessId(fg, std::ptr::null_mut())
        } else {
            0
        };
        let cur_tid = GetCurrentThreadId();
        let target_tid = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());

        if fg_tid != 0 && fg_tid != cur_tid {
            let _ = AttachThreadInput(fg_tid, cur_tid, 1);
        }
        if target_tid != 0 && target_tid != cur_tid {
            let _ = AttachThreadInput(target_tid, cur_tid, 1);
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);

        if target_tid != 0 && target_tid != cur_tid {
            let _ = AttachThreadInput(target_tid, cur_tid, 0);
        }
        if fg_tid != 0 && fg_tid != cur_tid {
            let _ = AttachThreadInput(fg_tid, cur_tid, 0);
        }

        for _ in 0..8 {
            if GetForegroundWindow() == hwnd {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        GetForegroundWindow() == hwnd
    }
}

#[cfg(not(windows))]
pub fn force_foreground_window(_target_hwnd: isize) -> bool {
    true
}

/// Strips dangerous Unicode bidirectional override characters and non-printable control characters.
/// In `Terminal` contexts, replaces CR/LF sequences with a single space so dictated text can NEVER auto-execute a shell command!
pub fn sanitize_for_injection(text: &str, is_terminal: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = false;
    for c in text.chars() {
        // Strip bidi Trojan-Source overrides
        if matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}') {
            continue;
        }
        if is_terminal && (c == '\x0d' || c == '\n') {
            if !out.is_empty() && !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        if c.is_control() && c != '\n' && c != '\x0d' && c != '\t' {
            continue;
        }
        last_was_space = c == ' ';
        out.push(c);
    }
    if is_terminal {
        out.trim().to_string()
    } else {
        out
    }
}

pub fn is_current_process_elevated() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut htok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut htok) != 0 && !htok.is_null() {
            let mut elev: TOKEN_ELEVATION = std::mem::zeroed();
            let mut ret: u32 = 0;
            let ok = GetTokenInformation(
                htok,
                TokenElevation,
                &mut elev as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret,
            );
            CloseHandle(htok);
            return ok != 0 && elev.TokenIsElevated != 0;
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn is_unsupported_shell_target(window_class: &str, process_name: &str) -> bool {
    let cls = window_class.trim().to_lowercase();
    let proc_l = process_name.trim().to_lowercase();
    matches!(
        cls.as_str(),
        "progman" | "workerw" | "shell_traywnd" | " shell_secondarytraywnd" | "dv2controlhost"
    ) || proc_l == "lockapp.exe"
}

#[cfg(windows)]
unsafe fn release_active_modifiers_cleanly() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };

    // Wait up to ~90ms if user is still lifting finger off Right Ctrl / Alt / Win
    for _ in 0..9 {
        let any_down = [0x10i32, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]
            .iter()
            .any(|&vk| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0);
        if !any_down {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Ensure logical modifier state is clean so Unicode characters are never interpreted as Ctrl+<key>
    let mod_vks: [u16; 8] = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C];
    let mut release_inputs: Vec<INPUT> = Vec::new();
    for &vk in &mod_vks {
        if (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 {
            release_inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                    },
                },
            });
        }
    }
    if !release_inputs.is_empty() {
        let _ = SendInput(
            release_inputs.len() as u32,
            release_inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Gives keyboard focus to KeyReed's own confirmation dialog when Windows' foreground lock refuses
/// `SetForegroundWindow`. A tagged Alt tap makes this process the source of the last input event,
/// which is the documented condition for being allowed to set the foreground window. Alt alone is
/// ignored by the dialog; the tag keeps our hotkey hook from reacting to it.
#[cfg(windows)]
fn focus_dialog_with_alt_unlock(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VK_MENU, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: KEYREED_SYNTHETIC_TAG },
        },
    };
    unsafe {
        let down = [key(0)];
        let up = [key(KEYEVENTF_KEYUP)];
        let _ = SendInput(1, down.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(hwnd as _);
        let _ = SendInput(1, up.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(std::time::Duration::from_millis(30));
        GetForegroundWindow() as isize == hwnd
    }
}

/// Attempts direct Unicode insertion into `target_ctx`, falling back to Clipboard automatically
/// whenever UIPI, password protection, focus change (TOCTOU), unsupported target, or dangerous-command policy applies.
pub fn deliver_direct_or_clipboard_fallback(
    text: &str,
    target_ctx: &ForegroundContext,
    block_passwords: bool,
    requires_command_confirmation: bool,
    exclude_from_clipboard_history: bool,
) -> Result<DirectTypeOutcome> {
    if text.is_empty() {
        return Ok(DirectTypeOutcome::TypedDirectly { code_units_sent: 0 });
    }

    // 1. Dangerous shell command gate
    if requires_command_confirmation {
        write_clipboard_unicode(text, exclude_from_clipboard_history)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "dangerous_command_requires_manual_paste",
        });
    }

    // 2. Password field / password manager protection gate
    if block_passwords
        && (target_ctx.is_password_field || target_ctx.category == AppCategory::PasswordProtected)
    {
        write_clipboard_unicode(text, true)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "password_field_protected",
        });
    }

    // 3. Elevated target process (UIPI) gate
    if target_ctx.is_elevated && !is_current_process_elevated() {
        write_clipboard_unicode(text, exclude_from_clipboard_history)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "target_window_elevated_uipi",
        });
    }

    // 4. Desktop / shell background / missing target gate
    if target_ctx.hwnd == 0
        || is_unsupported_shell_target(&target_ctx.window_class, &target_ctx.process_name)
    {
        write_clipboard_unicode(text, exclude_from_clipboard_history)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "unsupported_target_window",
        });
    }

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, VK_RETURN, VK_SHIFT, VK_TAB,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
        };

        let target_hwnd = target_ctx.hwnd as usize as HWND;
        if IsWindow(target_hwnd) == 0 {
            write_clipboard_unicode(text, exclude_from_clipboard_history)?;
            return Ok(DirectTypeOutcome::FallbackToClipboard {
                reason: "target_window_closed",
            });
        }

        // 5. TOCTOU focus verification & safe self-focus restoration:
        // If our own window (e.g. AskBeforeInserting dialog or tray menu) briefly held focus,
        // restore focus to `target_hwnd`. If the user switched to a DIFFERENT third-party window,
        // NEVER type into the unintended window!
        let mut current_fg = GetForegroundWindow();
        if current_fg as usize as u64 != target_ctx.hwnd {
            let mut fg_pid: u32 = 0;
            if !current_fg.is_null() {
                GetWindowThreadProcessId(current_fg, &mut fg_pid);
            }
            if current_fg.is_null() || fg_pid == std::process::id() {
                let _ = force_foreground_window(target_hwnd as isize);
                current_fg = GetForegroundWindow();
            }
            if current_fg as usize as u64 != target_ctx.hwnd {
                write_clipboard_unicode(text, exclude_from_clipboard_history)?;
                return Ok(DirectTypeOutcome::FallbackToClipboard {
                    reason: "foreground_window_changed_during_processing",
                });
            }
        }

        // If target is a modern container like Windows 11 Notepad (`NotepadTextBox` -> `RichEditD2DPT`),
        // ensure keyboard focus is inside the editor child across process threads via AttachThreadInput.
        if let Some(editor_child) = crate::context::find_deep_editor_hwnd(target_hwnd) {
            use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
            let target_tid = GetWindowThreadProcessId(target_hwnd, std::ptr::null_mut());
            let cur_tid = GetCurrentThreadId();
            if target_tid != 0 && target_tid != cur_tid {
                let _ = AttachThreadInput(target_tid, cur_tid, 1);
                let _ = SetFocus(editor_child);
                let _ = AttachThreadInput(target_tid, cur_tid, 0);
            } else {
                let _ = SetFocus(editor_child);
            }
        }

        let is_term = target_ctx.category == AppCategory::Terminal;
        let is_chat = target_ctx.category == AppCategory::Chat;
        let sanitized = sanitize_for_injection(text, is_term);
        if sanitized.is_empty() {
            return Ok(DirectTypeOutcome::TypedDirectly { code_units_sent: 0 });
        }

        // Keep completed text on clipboard as backup so transcription is never lost
        let _ = write_clipboard_unicode(&sanitized, exclude_from_clipboard_history);

        release_active_modifiers_cleanly();

        let mut inputs: Vec<INPUT> = Vec::with_capacity(sanitized.len() * 2 + 8);
        for ch in sanitized.chars() {
            if ch == '\x0d' {
                continue;
            }
            if ch == '\n' {
                if is_chat {
                    // In Chat apps (Slack, Teams, Discord, WhatsApp), Shift+Enter inserts a newline without sending!
                    for (vk, flags) in [
                        (VK_SHIFT, 0u32),
                        (VK_RETURN, 0u32),
                        (VK_RETURN, KEYEVENTF_KEYUP),
                        (VK_SHIFT, KEYEVENTF_KEYUP),
                    ] {
                        inputs.push(INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: vk,
                                    wScan: 0,
                                    dwFlags: flags,
                                    time: 0,
                                    dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                                },
                            },
                        });
                    }
                } else {
                    for flags in [0u32, KEYEVENTF_KEYUP] {
                        inputs.push(INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: VK_RETURN,
                                    wScan: 0,
                                    dwFlags: flags,
                                    time: 0,
                                    dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                                },
                            },
                        });
                    }
                }
                continue;
            }
            if ch == '\t' {
                for flags in [0u32, KEYEVENTF_KEYUP] {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_TAB,
                                wScan: 0,
                                dwFlags: flags,
                                time: 0,
                                dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                            },
                        },
                    });
                }
                continue;
            }
            let mut u16_buf = [0u16; 2];
            for &unit in ch.encode_utf16(&mut u16_buf).iter() {
                for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: 0,
                                wScan: unit,
                                dwFlags: flags,
                                time: 0,
                                dwExtraInfo: KEYREED_SYNTHETIC_TAG,
                            },
                        },
                    });
                }
            }
        }

        let mut total_sent = 0usize;
        for chunk in inputs.chunks(64) {
            // Re-verify foreground window before each chunk on long multiline dictations
            if GetForegroundWindow() as usize as u64 != target_ctx.hwnd {
                return Ok(DirectTypeOutcome::FallbackToClipboard {
                    reason: "foreground_window_changed_during_typing",
                });
            }
            let sent = SendInput(
                chunk.len() as u32,
                chunk.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
            if sent == 0 {
                return Ok(DirectTypeOutcome::FallbackToClipboard {
                    reason: "sendinput_blocked_by_target",
                });
            }
            total_sent += sent as usize;
            if inputs.len() > 64 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        Ok(DirectTypeOutcome::TypedDirectly {
            code_units_sent: total_sent / 2,
        })
    }
    #[cfg(not(windows))]
    {
        write_clipboard_unicode(text, exclude_from_clipboard_history)?;
        Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "non_windows_host",
        })
    }
}

/// Prompts the user before inserting into `target_ctx` (`OutputMode::AskBeforeInserting`).
/// Always copies `text` to the clipboard first so the transcription is never lost.
pub fn prompt_and_deliver_ask_before_inserting(
    text: &str,
    target_ctx: &ForegroundContext,
    block_passwords: bool,
    requires_command_confirmation: bool,
    exclude_from_clipboard_history: bool,
    override_decision: Option<AskInsertDecision>,
) -> Result<DirectTypeOutcome> {
    let decision = if let Some(d) = override_decision {
        d
    } else {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, IDNO, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST,
                MB_YESNOCANCEL,
            };
            let target_name = if target_ctx.process_name.is_empty() {
                "the active window".to_string()
            } else {
                target_ctx.process_name.clone()
            };
            let preview: String = text.chars().take(280).collect();
            let prompt = format!(
                "Insert dictated text into {}?\n\n\"{}\"\n\n• Click Yes to auto-insert into {}.\n• Click No to keep on clipboard only.\n• Click Cancel to dismiss without inserting.",
                target_name, preview, target_name
            );
            let title = "KeyReed — Ask Before Inserting\0"
                .encode_utf16()
                .collect::<Vec<u16>>();
            let body: Vec<u16> = prompt.encode_utf16().chain(std::iter::once(0)).collect();
            // Windows' foreground lock usually stops a background process from activating its own
            // MessageBox, which left the dialog visible but without keyboard focus (Enter/Esc went to
            // the user's app). Give it focus explicitly once it exists.
            let focus_helper = std::thread::spawn(|| {
                use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindowVisible};
                let cls: Vec<u16> = "#32770\0".encode_utf16().collect();
                let ttl: Vec<u16> = "KeyReed — Ask Before Inserting\0".encode_utf16().collect();
                for _ in 0..60 {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    let h = FindWindowW(cls.as_ptr(), ttl.as_ptr());
                    if !h.is_null() && IsWindowVisible(h) != 0 {
                        let first = force_foreground_window(h as isize);
                        let second = first || focus_dialog_with_alt_unlock(h as isize);
                        crate::security::timing_mark(&format!("ask_dialog_focus attach={first} alt={second}"));
                        return;
                    }
                }
            });
            let ans = MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_YESNOCANCEL | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
            );
            let _ = focus_helper.join();
            if ans == IDYES {
                AskInsertDecision::InsertIntoTarget
            } else if ans == IDNO {
                AskInsertDecision::CopyToClipboardOnly
            } else {
                AskInsertDecision::Cancel
            }
        }
        #[cfg(not(windows))]
        {
            AskInsertDecision::CopyToClipboardOnly
        }
    };

    match decision {
        AskInsertDecision::InsertIntoTarget => {
            write_clipboard_unicode(text, exclude_from_clipboard_history)?;
            if target_ctx.hwnd != 0 {
                let _ = force_foreground_window(target_ctx.hwnd as isize);
                std::thread::sleep(std::time::Duration::from_millis(35));
            }
            deliver_direct_or_clipboard_fallback(
                text,
                target_ctx,
                block_passwords,
                requires_command_confirmation,
                exclude_from_clipboard_history,
            )
        }
        AskInsertDecision::CopyToClipboardOnly => {
            write_clipboard_unicode(text, exclude_from_clipboard_history)?;
            Ok(DirectTypeOutcome::FallbackToClipboard {
                reason: "user_chose_clipboard_only",
            })
        }
        AskInsertDecision::Cancel => Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "user_cancelled_insert",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_sanitization_strips_newlines_and_bidi() {
        let s = sanitize_for_injection("kubectl get pods\u{202E}\x0d\nrm -rf /", true);
        assert!(!s.contains('\n'));
        assert!(!s.contains('\x0d'));
        assert!(!s.contains('\u{202E}'));
    }

    #[test]
    fn detects_unsupported_shell_targets() {
        assert!(is_unsupported_shell_target("Progman", "explorer.exe"));
        assert!(is_unsupported_shell_target("Shell_TrayWnd", "explorer.exe"));
        assert!(!is_unsupported_shell_target("Notepad", "notepad.exe"));
    }

    #[test]
    fn blocks_password_and_dangerous_command_targets() {
        let mut ctx = ForegroundContext::default();
        ctx.hwnd = 12345;
        ctx.process_name = "notepad.exe".to_string();
        ctx.window_class = "Notepad".to_string();
        ctx.is_password_field = true;

        let res = deliver_direct_or_clipboard_fallback("secret123", &ctx, true, false, true).unwrap();
        assert_eq!(
            res,
            DirectTypeOutcome::FallbackToClipboard {
                reason: "password_field_protected"
            }
        );

        ctx.is_password_field = false;
        let res_cmd = deliver_direct_or_clipboard_fallback("sudo rm -rf /", &ctx, true, true, false).unwrap();
        assert_eq!(
            res_cmd,
            DirectTypeOutcome::FallbackToClipboard {
                reason: "dangerous_command_requires_manual_paste"
            }
        );
    }
}
