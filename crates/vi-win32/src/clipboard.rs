//! Resilient Windows Clipboard integration (Master Prompt Section 4 & 12).
//!
//! Guarantees:
//! - Retries `OpenClipboard` with backoff when another process (RDP, Office, clipboard manager) briefly locks it.
//! - Allocates and populates `CF_UNICODETEXT` BEFORE calling `EmptyClipboard`, so a memory failure never wipes user data.
//! - Never clears or modifies the clipboard on transcription failure, cancellation, or silence!
//! - Normalizes lone `\n` to `\r\n` so all classic Win32 edit controls, Notepad, Office, and browsers preserve paragraphs and lists.
//! - Supports Microsoft's official clipboard privacy exclusion formats (`ExcludeClipboardContentFromMonitorProcessing`,
//!   `CanIncludeInClipboardHistory`, `CanUploadToCloudClipboard`) when configured.

use anyhow::{bail, Result};
use std::time::Duration;

pub fn normalize_windows_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev_cr = false;
    for ch in text.chars() {
        if ch == '\n' && !prev_cr {
            out.push('\r');
            out.push('\n');
        } else {
            out.push(ch);
        }
        prev_cr = ch == '\r';
    }
    out
}

pub fn write_clipboard_unicode(text: &str, exclude_from_history: bool) -> Result<()> {
    if text.is_empty() {
        bail!("refusing to overwrite clipboard with empty text");
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::GlobalFree;
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
            SetClipboardData,
        };
        use windows_sys::Win32::System::Memory::{
            GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
        };

        const CF_UNICODETEXT: u32 = 13;

        let normalized = normalize_windows_newlines(text);
        let utf16: Vec<u16> = normalized.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_len = utf16.len() * std::mem::size_of::<u16>();

        let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if hmem.is_null() {
            bail!("GlobalAlloc failed for clipboard buffer");
        }
        let locked = GlobalLock(hmem) as *mut u16;
        if locked.is_null() {
            GlobalFree(hmem);
            bail!("GlobalLock failed for clipboard buffer");
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), locked, utf16.len());
        GlobalUnlock(hmem);

        // Retry OpenClipboard across transient contention (up to ~180 ms total)
        let mut opened = false;
        for attempt in 0..12u64 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(2 + attempt * 3));
        }
        if !opened {
            GlobalFree(hmem);
            bail!("OpenClipboard busy after 12 retries");
        }

        if EmptyClipboard() == 0 {
            CloseClipboard();
            GlobalFree(hmem);
            bail!("EmptyClipboard failed");
        }

        if SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
            CloseClipboard();
            GlobalFree(hmem);
            bail!("SetClipboardData(CF_UNICODETEXT) failed");
        }
        // Ownership of `hmem` now belongs to the OS clipboard!

        if exclude_from_history {
            for fmt_name in [
                "CanIncludeInClipboardHistory\0",
                "CanUploadToCloudClipboard\0",
                "ExcludeClipboardContentFromMonitorProcessing\0",
            ] {
                let wide: Vec<u16> = fmt_name.encode_utf16().collect();
                let fmt_id = RegisterClipboardFormatW(wide.as_ptr());
                if fmt_id != 0 {
                    let h_dword = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, 4);
                    if !h_dword.is_null() {
                        if SetClipboardData(fmt_id, h_dword).is_null() {
                            GlobalFree(h_dword);
                        }
                    }
                }
            }
        }

        CloseClipboard();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = exclude_from_history;
        Ok(())
    }
}

pub fn read_clipboard_unicode() -> Result<Option<String>> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        };
        use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

        const CF_UNICODETEXT: u32 = 13;
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
            return Ok(None);
        }
        let mut opened = false;
        for attempt in 0..8u64 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(2 + attempt * 3));
        }
        if !opened {
            bail!("OpenClipboard busy while reading");
        }
        let hdata = GetClipboardData(CF_UNICODETEXT);
        if hdata.is_null() {
            CloseClipboard();
            return Ok(None);
        }
        let ptr = GlobalLock(hdata) as *const u16;
        if ptr.is_null() {
            CloseClipboard();
            return Ok(None);
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf16_lossy(slice);
        GlobalUnlock(hdata);
        CloseClipboard();
        Ok(Some(text))
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_lf_without_doubling_crlf() {
        assert_eq!(
            normalize_windows_newlines("Line 1\nLine 2\r\nLine 3"),
            "Line 1\r\nLine 2\r\nLine 3"
        );
    }
}
