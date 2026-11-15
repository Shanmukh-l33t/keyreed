//! Safe optional Direct Typing engine via `SendInput` (`KEYEVENTF_UNICODE`) with mandatory
//! security and reliability fallbacks to Clipboard (Master Prompt Section 4, 6 & 12).

use crate::clipboard::write_clipboard_unicode;
use crate::security::KEYREED_SYNTHETIC_TAG;
use anyhow::Result;
use vi_core::{AppCategory, ForegroundContext};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectTypeOutcome {
    TypedDirectly { code_units_sent: usize },
    FallbackToClipboard { reason: &'static str },
}

/// Strips dangerous Unicode bidirectional override characters and non-printable control characters.
/// In `Terminal` contexts, also strips `\r` and `\n` so dictated text can NEVER auto-execute a shell command!
pub fn sanitize_for_injection(text: &str, is_terminal: bool) -> String {
    text.chars()
        .filter(|&c| {
            // Strip bidi Trojan-Source overrides
            if matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}') {
                return false;
            }
            if is_terminal && (c == '\r' || c == '\n') {
                return false;
            }
            if c.is_control() && c != '\n' && c != '\r' && c != '\t' {
                return false;
            }
            true
        })
        .collect()
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

/// Attempts direct Unicode insertion into `target_ctx`, falling back to Clipboard automatically
/// whenever UIPI, password protection, focus change (TOCTOU), or dangerous-command policy applies.
pub fn deliver_direct_or_clipboard_fallback(
    text: &str,
    target_ctx: &ForegroundContext,
    block_passwords: bool,
    requires_command_confirmation: bool,
    exclude_from_clipboard_history: bool,
) -> Result<DirectTypeOutcome> {
    // Always copy to clipboard as well or on fallback; check safety gates first!
    if requires_command_confirmation {
        write_clipboard_unicode(text, exclude_from_clipboard_history)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "dangerous_command_requires_manual_paste",
        });
    }

    if block_passwords
        && (target_ctx.is_password_field || target_ctx.category == AppCategory::PasswordProtected)
    {
        write_clipboard_unicode(text, true)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "password_field_protected",
        });
    }

    if target_ctx.is_elevated && !is_current_process_elevated() {
        write_clipboard_unicode(text, exclude_from_clipboard_history)?;
        return Ok(DirectTypeOutcome::FallbackToClipboard {
            reason: "target_window_elevated_uipi",
        });
    }

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, VK_RETURN,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        // TOCTOU focus verification: if user switched windows while inference ran, do NOT type into wrong window!
        let current_fg = GetForegroundWindow() as usize as u64;
        if target_ctx.hwnd != 0 && current_fg != target_ctx.hwnd {
            write_clipboard_unicode(text, exclude_from_clipboard_history)?;
            return Ok(DirectTypeOutcome::FallbackToClipboard {
                reason: "foreground_window_changed_during_processing",
            });
        }

        let is_term = target_ctx.category == AppCategory::Terminal;
        let sanitized = sanitize_for_injection(text, is_term);
        if sanitized.is_empty() {
            return Ok(DirectTypeOutcome::TypedDirectly { code_units_sent: 0 });
        }

        // Also keep on clipboard so user still has Ctrl+V if custom game/remote control ignores KEYEVENTF_UNICODE
        let _ = write_clipboard_unicode(&sanitized, exclude_from_clipboard_history);

        let mut inputs: Vec<INPUT> = Vec::with_capacity(sanitized.len() * 2);
        let mut prev_cr = false;
        for ch in sanitized.chars() {
            if ch == '\r' {
                prev_cr = true;
                continue;
            }
            if ch == '\n' {
                prev_cr = false;
                // Emit VK_RETURN for multi-line prose in non-terminal fields
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
                continue;
            }
            prev_cr = false;
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
        let _ = prev_cr;

        let mut total_sent = 0usize;
        for chunk in inputs.chunks(64) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_sanitization_strips_newlines_and_bidi() {
        let s = sanitize_for_injection("kubectl get pods\u{202E}\r\nrm -rf /", true);
        assert!(!s.contains('\n'));
        assert!(!s.contains('\r'));
        assert!(!s.contains('\u{202E}'));
    }
}
