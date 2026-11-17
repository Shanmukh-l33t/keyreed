//! Selection service (plan P2.4): read the selected text of another application and replace it, for Quick Fix
//! and the Composer.
//!
//! Capture: refuse unsafe targets, snapshot the clipboard (refuse when it cannot be put back), send a tagged
//! Ctrl+C, wait for the clipboard sequence number to change (no fixed sleep), read the text, note whether the copy
//! carried structure, restore the clipboard. Replace: re-verify window and control, publish the result as a
//! delayed-render clipboard entry, send Ctrl+V and treat the render request as the receipt, restore the user's
//! clipboard after a quiet period unless somebody else wrote to it. The whole transaction runs under the input gate.
//! Text never reaches a log: refusals are a closed list.

use crate::clipboard::{
    input_gate, normalize_windows_newlines, read_clipboard_unicode, write_clipboard_unicode, ClipboardSnapshot,
    SnapshotLimits,
};
use crate::security::KEYREED_SYNTHETIC_TAG;
use crate::typing::{
    force_foreground_window, is_current_process_elevated, is_unsupported_shell_target,
    release_active_modifiers_cleanly, sanitize_for_injection,
};
use std::cell::RefCell;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vi_core::{AppCategory, ForegroundContext};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowThreadProcessId, MsgWaitForMultipleObjects, PeekMessageW, RegisterClassW, TranslateMessage, GUITHREADINFO, HWND_MESSAGE, MSG,
    PM_REMOVE, QS_ALLINPUT, WM_DESTROYCLIPBOARD, WM_RENDERFORMAT, WNDCLASSW,
};

const CF_UNICODETEXT: u32 = 13;
const CF_BITMAP: u32 = 2;
const CF_DIB: u32 = 8;
const CF_HDROP: u32 = 15;
const VK_C: u16 = 0x43;
const VK_V: u16 = 0x56;
const VK_Z: u16 = 0x5A;

/// How long the copy may take to reach the clipboard before "nothing is selected" is assumed.
const COPY_WAIT: Duration = Duration::from_millis(400);
/// A capture older than this is repeated before it is replaced, and must give the same text (plan C2).
const STALE_AFTER: Duration = Duration::from_secs(1);
const RECEIPT_WAIT: Duration = Duration::from_secs(2);
/// After the first wait, a result that was not pasted stays on the clipboard this long, then the user's clipboard returns.
const PENDING_RESTORE: Duration = Duration::from_secs(8);
const QUIET: Duration = Duration::from_millis(200);
/// Largest HTML or RTF flavour inspected for structure.
const FLAVOUR_LIMIT: usize = 1 << 20;

/// Why the selection service did nothing. A closed list so no text can leak into a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    PasswordTarget,
    Elevated,
    ShellTarget,
    Terminal,
    NothingSelected,
    ClipboardNotRestorable,
    ClipboardBusy,
    FocusMoved,
    SelectionChanged,
    Failed,
}

impl Refusal {
    /// The one sentence the pill or the Review panel shows.
    pub fn sentence(self) -> &'static str {
        match self {
            Refusal::PasswordTarget => "KeyReed doesn't work in password fields.",
            Refusal::Elevated => "KeyReed can't work in administrator windows.",
            Refusal::ShellTarget => "There's no text box here.",
            Refusal::Terminal => "KeyReed doesn't fix text in terminals.",
            Refusal::NothingSelected => "Select some text first.",
            Refusal::ClipboardNotRestorable => {
                "Your clipboard holds something KeyReed can't put back. Nothing was changed."
            }
            Refusal::ClipboardBusy => "The clipboard is busy. Nothing was changed.",
            Refusal::FocusMoved => "The window changed. Nothing was changed.",
            Refusal::SelectionChanged => "The selection changed. Nothing was replaced.",
            Refusal::Failed => "KeyReed couldn't replace the text. Nothing was changed.",
        }
    }
}

/// Targets where Ctrl+C must not be sent at all. Code editors and spreadsheet cells are allowed: the engine
/// limits what it does there.
pub fn refusal_for(ctx: &ForegroundContext, self_elevated: bool) -> Option<Refusal> {
    if ctx.is_password_field || ctx.category == AppCategory::PasswordProtected {
        Some(Refusal::PasswordTarget)
    } else if ctx.is_elevated && !self_elevated {
        Some(Refusal::Elevated)
    } else if ctx.hwnd == 0 || is_unsupported_shell_target(&ctx.window_class, &ctx.process_name) {
        Some(Refusal::ShellTarget)
    } else if ctx.category == AppCategory::Terminal {
        // With nothing selected Ctrl+C reaches the shell and interrupts the running program.
        Some(Refusal::Terminal)
    } else {
        None
    }
}

/// The signature of "copy line" in editors that copy the whole line when nothing is selected.
pub fn looks_like_copied_line(text: &str) -> bool {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .is_some_and(|rest| !rest.contains('\n'))
}

/// Links, images, tables, lists or headings in the HTML flavour of a copy.
pub fn html_is_structured(html: &str) -> bool {
    const TAGS: [&str; 12] = ["a", "img", "table", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6"];
    let lower = html.to_ascii_lowercase();
    // Browsers wrap the selection in a fragment; the rest is page scaffolding.
    let fragment = match (lower.find("<!--startfragment-->"), lower.find("<!--endfragment-->")) {
        (Some(a), Some(b)) if a < b => &lower[a..b],
        _ => &lower[..],
    };
    fragment.match_indices('<').any(|(i, _)| {
        let rest = &fragment[i + 1..];
        TAGS.iter().any(|t| {
            rest.strip_prefix(t)
                .is_some_and(|after| after.starts_with(|c: char| c == '>' || c == '/' || c.is_ascii_whitespace()))
        })
    })
}

/// Pictures, tables, list text or hyperlink fields in the RTF flavour of a copy.
pub fn rtf_is_structured(rtf: &str) -> bool {
    ["\\pict", "\\trowd", "\\listtext", "\\pntext", "\\field"].iter().any(|c| rtf.contains(c))
}

/// What a capture found. `text` is the selected text; `structured` and `copied_line` are the F1 conditions that
/// send the result to Review.
pub struct Captured {
    pub text: String,
    pub structured: bool,
    pub copied_line: bool,
    /// False when the user's clipboard could not be put back after the copy.
    pub clipboard_restored: bool,
    hwnd: u64,
    focus: u64,
    at: Instant,
}

/// What `replace` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replaced {
    /// Pasted and the target asked for the text; the user's clipboard comes back after a quiet period.
    Pasted,
    /// Pasted, but no receipt within two seconds: the result waits on the clipboard ("Press Ctrl+V to paste") and the
    /// user's clipboard returns after the next paste or eight seconds.
    NoReceipt,
    /// The window or control changed: nothing was pasted and the result is on the clipboard.
    CopiedFocusMoved,
}

fn foreground_is(hwnd: u64) -> bool {
    unsafe { GetForegroundWindow() as usize as u64 == hwnd }
}

/// The focused control of the foreground window's thread (0 when it has none).
fn focused_control() -> u64 {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return 0;
        }
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if tid != 0 && GetGUIThreadInfo(tid, &mut info) != 0 {
            info.hwndFocus as usize as u64
        } else {
            0
        }
    }
}

/// Brings the target back when one of KeyReed's own windows (the Review panel, the Composer) holds focus.
fn return_to_target(hwnd: u64) {
    unsafe {
        let fg = GetForegroundWindow();
        let mut pid = 0u32;
        if !fg.is_null() {
            GetWindowThreadProcessId(fg, &mut pid);
        }
        if fg as usize as u64 != hwnd && (fg.is_null() || pid == std::process::id()) {
            force_foreground_window(hwnd as isize);
            std::thread::sleep(Duration::from_millis(35));
        }
    }
}

/// Tagged modifier+key chord, so KeyReed's own hook ignores it.
fn send_chord(modifier: u16, key: u16) {
    let input = |vk: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: KEYREED_SYNTHETIC_TAG },
        },
    };
    let inputs = [input(modifier, 0), input(key, 0), input(key, KEYEVENTF_KEYUP), input(modifier, KEYEVENTF_KEYUP)];
    unsafe {
        release_active_modifiers_cleanly();
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

fn open_for(owner: HWND, attempts: u64) -> bool {
    for attempt in 0..attempts {
        if unsafe { OpenClipboard(owner) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2 + attempt * 3));
    }
    false
}

/// Reads a clipboard flavour the caller has open, up to `FLAVOUR_LIMIT` bytes (none if larger).
unsafe fn flavour_text(format: u32) -> Option<String> {
    let handle = GetClipboardData(format);
    if handle.is_null() {
        return None;
    }
    let size = GlobalSize(handle);
    let p = GlobalLock(handle) as *const u8;
    if p.is_null() || size > FLAVOUR_LIMIT {
        if !p.is_null() {
            GlobalUnlock(handle);
        }
        return None;
    }
    let text = String::from_utf8_lossy(std::slice::from_raw_parts(p, size)).into_owned();
    GlobalUnlock(handle);
    Some(text)
}

/// Whether the copy now on the clipboard carried links, images, tables, lists or headings.
fn clipboard_is_structured() -> bool {
    if !open_for(std::ptr::null_mut(), 8) {
        return false;
    }
    let mut structured = false;
    unsafe {
        let mut format = EnumClipboardFormats(0);
        while format != 0 && !structured {
            if matches!(format, CF_BITMAP | CF_DIB | CF_HDROP) {
                structured = true;
            } else if format >= 0xC000 {
                let mut name = [0u16; 64];
                let len = GetClipboardFormatNameW(format, name.as_mut_ptr(), name.len() as i32);
                let name = String::from_utf16_lossy(&name[..len.max(0) as usize]);
                structured = match name.as_str() {
                    "HTML Format" => flavour_text(format).is_some_and(|h| html_is_structured(&h)),
                    "Rich Text Format" => flavour_text(format).is_some_and(|r| rtf_is_structured(&r)),
                    _ => false,
                };
            }
            format = EnumClipboardFormats(format);
        }
        CloseClipboard();
    }
    structured
}

/// Copies the current selection of the verified foreground window. Caller holds the input gate. Returns the
/// text, whether it carried structure, and whether the user's clipboard was put back.
fn copy_selection(hwnd: u64) -> Result<(String, bool, bool), Refusal> {
    if !foreground_is(hwnd) {
        return Err(Refusal::FocusMoved);
    }
    let snapshot = ClipboardSnapshot::capture(SnapshotLimits::default()).map_err(|_| Refusal::ClipboardBusy)?;
    if !snapshot.is_complete() {
        return Err(Refusal::ClipboardNotRestorable);
    }
    let before = unsafe { GetClipboardSequenceNumber() };
    send_chord(VK_CONTROL, VK_C);
    let start = Instant::now();
    while unsafe { GetClipboardSequenceNumber() } == before {
        if start.elapsed() > COPY_WAIT {
            return Err(Refusal::NothingSelected); // the clipboard never changed, so there is nothing to put back
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = read_clipboard_unicode().ok().flatten();
    let structured = clipboard_is_structured();
    let restored = snapshot.restore().is_ok();
    match text {
        Some(t) if !t.trim().is_empty() => Ok((t, structured, restored)),
        _ => Err(Refusal::NothingSelected),
    }
}

/// Reads the selection of the foreground window `ctx` describes. The clipboard is put back before returning.
pub fn capture(ctx: &ForegroundContext) -> Result<Captured, Refusal> {
    if let Some(refusal) = refusal_for(ctx, is_current_process_elevated()) {
        return Err(refusal);
    }
    let _gate = input_gate();
    // KeyReed's own window can be in front: the Review panel takes the foreground when it opens, and the chord
    // can be pressed again straight after a fix. The selection is still in the window the chord was for, so put
    // it back in front first; another application's window is never taken from.
    return_to_target(ctx.hwnd);
    let (text, structured, clipboard_restored) = copy_selection(ctx.hwnd)?;
    Ok(Captured {
        copied_line: looks_like_copied_line(&text),
        text,
        structured,
        clipboard_restored,
        hwnd: ctx.hwnd,
        focus: focused_control(),
        at: Instant::now(),
    })
}

/// Replaces the captured selection with `text` by pasting. A window or control that changed, or a selection that
/// differs from the capture, is never pasted into.
pub fn replace(captured: &Captured, text: &str, ctx: &ForegroundContext) -> Result<Replaced, Refusal> {
    if let Some(refusal) = refusal_for(ctx, is_current_process_elevated()) {
        return Err(refusal);
    }
    let clean = sanitize_for_injection(text, false);
    if clean.is_empty() {
        return Err(Refusal::Failed);
    }
    let _gate = input_gate();
    return_to_target(captured.hwnd);
    if !foreground_is(captured.hwnd) || focused_control() != captured.focus {
        write_clipboard_unicode(&clean, true).map_err(|_| Refusal::ClipboardBusy)?;
        return Ok(Replaced::CopiedFocusMoved);
    }
    if captured.at.elapsed() > STALE_AFTER {
        let (again, _, _) = copy_selection(captured.hwnd)?;
        if again != captured.text {
            return Err(Refusal::SelectionChanged);
        }
    }
    let snapshot = ClipboardSnapshot::capture(SnapshotLimits::default()).map_err(|_| Refusal::ClipboardBusy)?;
    if !snapshot.is_complete() {
        return Err(Refusal::ClipboardNotRestorable);
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let (hwnd, focus) = (captured.hwnd, captured.focus);
    let units: Vec<u16> = normalize_windows_newlines(&clean).encode_utf16().chain(std::iter::once(0)).collect();
    std::thread::Builder::new()
        .name("keyreed-paste".into())
        .spawn(move || paste_thread(units, snapshot, hwnd, focus, tx))
        .map_err(|_| Refusal::Failed)?;
    match rx.recv_timeout(RECEIPT_WAIT + Duration::from_secs(1)) {
        Ok(PasteResult::Receipt) => Ok(Replaced::Pasted),
        Ok(PasteResult::NoReceipt) => Ok(Replaced::NoReceipt),
        Ok(PasteResult::FocusMoved) => Ok(Replaced::CopiedFocusMoved),
        Ok(PasteResult::Failed) | Err(_) => Err(Refusal::Failed),
    }
}

/// Sends Ctrl+Z to the window the last replace went into, after checking it is still in front.
pub fn undo(hwnd: u64) -> Result<(), Refusal> {
    let _gate = input_gate();
    return_to_target(hwnd);
    if !foreground_is(hwnd) {
        return Err(Refusal::FocusMoved);
    }
    send_chord(VK_CONTROL, VK_Z);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Delayed-render paste. The result is published without data; the target's request for it (WM_RENDERFORMAT) is the receipt. Clipboard history and cloud sync are told to skip the entry, so they
// never request it. The window that owns the entry lives on its own thread with a message loop.
// ---------------------------------------------------------------------------------------------------------------

enum PasteResult {
    Receipt,
    NoReceipt,
    FocusMoved,
    Failed,
}

struct PasteState {
    text: Vec<u16>,
    receipt: bool,
    /// Another program took the clipboard: the user's clipboard is not ours to restore any more.
    foreign_write: bool,
    restoring: bool,
}

impl Drop for PasteState {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.text);
    }
}

thread_local! {
    static PASTE: RefCell<Option<PasteState>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut PasteState) -> R) -> Option<R> {
    PASTE.with(|p| p.borrow_mut().as_mut().map(f))
}

unsafe extern "system" fn paste_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_RENDERFORMAT if wparam as u32 == CF_UNICODETEXT => {
            // The requester has the clipboard open: hand over the text. Clipboard history and cloud sync skip this
            // entry, so a request for it is the target pasting.
            with_state(|s| {
                let bytes = s.text.len() * 2;
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
                let p = if h.is_null() { std::ptr::null_mut() } else { GlobalLock(h) as *mut u16 };
                if !p.is_null() {
                    std::ptr::copy_nonoverlapping(s.text.as_ptr(), p, s.text.len());
                    GlobalUnlock(h);
                    if !SetClipboardData(CF_UNICODETEXT, h).is_null() {
                        s.receipt = true;
                    }
                }
            });
            0
        }
        WM_DESTROYCLIPBOARD => {
            with_state(|s| s.foreign_write |= !s.restoring);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Dispatches messages until `done` or `deadline`.
fn pump_until(deadline: Instant, done: impl Fn() -> bool) {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if done() || Instant::now() >= deadline {
                return;
            }
            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 10, QS_ALLINPUT);
        }
    }
}

fn paste_thread(units: Vec<u16>, snapshot: ClipboardSnapshot, hwnd: u64, focus: u64, tx: mpsc::SyncSender<PasteResult>) {
    PASTE.with(|p| {
        *p.borrow_mut() = Some(PasteState { text: units, receipt: false, foreign_write: false, restoring: false });
    });
    let owner = unsafe {
        let class = wide("KeyReedPasteOwner");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(paste_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc); // fails harmlessly when the class exists from an earlier paste
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if owner.is_null() {
        let _ = tx.send(PasteResult::Failed);
        return;
    }
    let _owner = DestroyOnDrop(owner);

    let published = open_for(owner, 12) && unsafe {
        EmptyClipboard();
        // No data yet: the target's request renders it.
        SetClipboardData(CF_UNICODETEXT, std::ptr::null_mut());
        let ok = IsClipboardFormatAvailable(CF_UNICODETEXT) != 0;
        for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard", "ExcludeClipboardContentFromMonitorProcessing"] {
            let id = RegisterClipboardFormatW(wide(name).as_ptr());
            let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, 4);
            if id != 0 && !h.is_null() && SetClipboardData(id, h).is_null() {
                windows_sys::Win32::Foundation::GlobalFree(h);
            }
        }
        CloseClipboard();
        ok
    };
    if !published {
        let _ = snapshot.restore();
        let _ = tx.send(PasteResult::Failed);
        return;
    }
    with_state(|s| s.foreign_write = false); // our own EmptyClipboard does not count

    if !foreground_is(hwnd) || focused_control() != focus {
        // Keep the result for a manual paste as ordinary clipboard text.
        let text = with_state(|s| String::from_utf16_lossy(&s.text[..s.text.len() - 1])).unwrap_or_default();
        let _ = write_clipboard_unicode(&text, true);
        let _ = tx.send(PasteResult::FocusMoved);
        return;
    }
    send_chord(VK_CONTROL, VK_V);

    let received = || with_state(|s| s.receipt || s.foreign_write).unwrap_or(true);
    pump_until(Instant::now() + RECEIPT_WAIT, received);
    let got_receipt = with_state(|s| s.receipt).unwrap_or(false);
    let _ = tx.send(if got_receipt { PasteResult::Receipt } else { PasteResult::NoReceipt });
    if !got_receipt {
        pump_until(Instant::now() + PENDING_RESTORE, received);
    }
    pump_until(Instant::now() + QUIET, || false); // late requests for the data are still answered

    let ours = with_state(|s| {
        s.restoring = true;
        !s.foreign_write
    })
    .unwrap_or(false);
    if ours {
        let _ = snapshot.restore();
    }
}

/// Destroys the owner window when the paste thread ends, on every path.
struct DestroyOnDrop(HWND);

impl Drop for DestroyOnDrop {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(category: AppCategory) -> ForegroundContext {
        ForegroundContext {
            hwnd: 1,
            process_name: "app.exe".into(),
            window_class: "Window".into(),
            category,
            ..ForegroundContext::default()
        }
    }

    #[test]
    fn refusals_follow_the_f1_table() {
        assert_eq!(refusal_for(&ctx(AppCategory::Email), false), None);
        assert_eq!(refusal_for(&ctx(AppCategory::CodeEditor), false), None);
        assert_eq!(refusal_for(&ctx(AppCategory::Terminal), false), Some(Refusal::Terminal));
        assert_eq!(refusal_for(&ctx(AppCategory::PasswordProtected), false), Some(Refusal::PasswordTarget));
        let mut c = ctx(AppCategory::General);
        c.is_password_field = true;
        assert_eq!(refusal_for(&c, false), Some(Refusal::PasswordTarget));
        let mut c = ctx(AppCategory::General);
        c.is_elevated = true;
        assert_eq!(refusal_for(&c, false), Some(Refusal::Elevated));
        assert_eq!(refusal_for(&c, true), None);
        let mut c = ctx(AppCategory::General);
        c.hwnd = 0;
        assert_eq!(refusal_for(&c, false), Some(Refusal::ShellTarget));
        let mut c = ctx(AppCategory::General);
        c.window_class = "Shell_TrayWnd".into();
        assert_eq!(refusal_for(&c, false), Some(Refusal::ShellTarget));
    }

    #[test]
    fn a_single_line_with_a_break_is_a_copied_line() {
        assert!(looks_like_copied_line("let x = 1;\r\n"));
        assert!(looks_like_copied_line("let x = 1;\n"));
        assert!(!looks_like_copied_line("let x = 1;"));
        assert!(!looks_like_copied_line("a\r\nb\r\n"));
        assert!(!looks_like_copied_line(""));
    }

    #[test]
    fn html_structure_is_judged_on_the_copied_fragment() {
        let page = |inner: &str| {
            format!("Version:0.9\r\n<html><body><nav><a href=x>menu</a></nav><!--StartFragment-->{inner}<!--EndFragment--></body></html>")
        };
        assert!(!html_is_structured(&page("<span>plain words</span>")));
        assert!(!html_is_structured(&page("<b>bold</b> and <i>italic</i> and <abbr>x</abbr>")));
        assert!(html_is_structured(&page("see <a href=\"https://x\">this</a>")));
        assert!(html_is_structured(&page("<UL><LI>one</LI></UL>")));
        assert!(html_is_structured(&page("<img src=a.png>")));
        assert!(html_is_structured(&page("<h2>Title</h2>")));
        assert!(html_is_structured("<table><tr><td>x</td></tr></table>")); // no fragment markers: whole text
    }

    #[test]
    fn rtf_structure_markers() {
        assert!(!rtf_is_structured("{\\rtf1\\ansi plain \\b bold\\b0}"));
        assert!(rtf_is_structured("{\\rtf1{\\pict\\wmetafile8 00}}"));
        assert!(rtf_is_structured("{\\rtf1\\trowd\\cell}"));
        assert!(rtf_is_structured("{\\rtf1{\\field{\\*\\fldinst HYPERLINK}}}"));
    }

    #[test]
    fn every_refusal_has_a_sentence_without_placeholders() {
        for r in [
            Refusal::PasswordTarget,
            Refusal::Elevated,
            Refusal::ShellTarget,
            Refusal::Terminal,
            Refusal::NothingSelected,
            Refusal::ClipboardNotRestorable,
            Refusal::ClipboardBusy,
            Refusal::FocusMoved,
            Refusal::SelectionChanged,
            Refusal::Failed,
        ] {
            assert!(r.sentence().ends_with('.') && !r.sentence().contains('{'));
        }
    }

    /// Manual harness: opens its own edit window, selects its text, then captures, replaces and undoes through the
    /// service and checks that the clipboard comes back. Injects keys into that window only. Run with
    /// `cargo test -p vi-win32 selection_round_trip -- --ignored --nocapture` while the PC is otherwise idle.
    #[test]
    #[ignore = "injects keys into a window the test creates and uses the real clipboard"]
    fn selection_round_trip_in_an_edit_window() {
        use windows_sys::Win32::UI::Controls::EM_SETSEL;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetWindowTextW, SendMessageW, SetWindowTextW, ES_MULTILINE,
            WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };

        struct PutBack(Option<ClipboardSnapshot>);
        impl Drop for PutBack {
            fn drop(&mut self) {
                if let Some(s) = self.0.take() {
                    let _ = s.restore();
                }
            }
        }
        let original = ClipboardSnapshot::capture(SnapshotLimits { time: Duration::from_secs(3), ..Default::default() })
            .expect("snapshot of the user's clipboard");
        if !original.is_complete() {
            eprintln!("skipped: the current clipboard cannot be copied completely, so it is left alone");
            return;
        }
        let _guard = PutBack(Some(original));
        write_clipboard_unicode("clipboard sentinel", true).unwrap();

        let (before, after) = ("teh quick brown fox", "the quick brown fox");
        let edit = unsafe {
            let hwnd = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("KeyReed selection test").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE | ES_MULTILINE as u32,
                100,
                100,
                420,
                200,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            SetWindowTextW(hwnd, wide(before).as_ptr());
            hwnd
        };
        let hwnd = edit as usize as u64;
        let text_of = || unsafe {
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(edit, buf.as_mut_ptr(), 256);
            String::from_utf16_lossy(&buf[..n as usize])
        };
        unsafe {
            if !force_foreground_window(edit as isize) {
                DestroyWindow(edit);
                eprintln!("skipped: Windows would not bring the test window to the front");
                return;
            }
            SetFocus(edit);
            SendMessageW(edit, EM_SETSEL, 0, -1);
        }

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let run = || -> Result<(String, bool), Refusal> {
                let ctx = crate::context::capture_foreground_context(false);
                assert_eq!(ctx.hwnd, hwnd, "the test window must be in front");
                let cap = capture(&ctx)?;
                assert!(!cap.structured && !cap.copied_line && cap.clipboard_restored);
                assert_eq!(read_clipboard_unicode().unwrap().as_deref(), Some("clipboard sentinel"));
                std::thread::sleep(Duration::from_millis(1200)); // forces the repeat-copy check
                let result = replace(&cap, "the quick brown fox", &ctx)?;
                Ok((cap.text, result == Replaced::Pasted))
            };
            let _ = tx.send(run());
        });
        let mut outcome = None;
        let deadline = Instant::now() + Duration::from_secs(15);
        while outcome.is_none() && Instant::now() < deadline {
            pump_until(Instant::now() + Duration::from_millis(20), || false);
            outcome = rx.try_recv().ok();
        }
        let (captured_text, pasted) = outcome.expect("service finished").expect("capture and replace");
        assert_eq!(captured_text, before);
        assert!(pasted, "no paste receipt");
        pump_until(Instant::now() + Duration::from_millis(700), || false); // the quiet period and the restore
        assert_eq!(text_of(), after);
        assert_eq!(read_clipboard_unicode().unwrap().as_deref(), Some("clipboard sentinel"), "clipboard not restored");

        let (utx, urx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = utx.send(undo(hwnd));
        });
        let mut undone = None;
        while undone.is_none() && Instant::now() < deadline {
            pump_until(Instant::now() + Duration::from_millis(20), || false);
            undone = urx.try_recv().ok();
        }
        undone.expect("undo finished").expect("undo");
        pump_until(Instant::now() + Duration::from_millis(300), || false);
        // The classic edit control keeps one undo level and undoes only the insert half of a replace, so the
        // harness checks that the text left `after`, not that it equals `before`; other applications undo in one step.
        assert_ne!(text_of(), after, "undo did not reach the window");
        unsafe {
            DestroyWindow(edit);
        }
    }
}
