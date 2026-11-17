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
use std::time::{Duration, Instant};

/// The input and clipboard gate: one lock serialises every clipboard transaction and every burst of
/// synthetic input across threads, so a dictation delivered during a Quick Fix restore cannot
/// collide with it. Not re-entrant: take it at the outermost call site only.
static INPUT_GATE: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub fn input_gate() -> parking_lot::MutexGuard<'static, ()> {
    INPUT_GATE.lock()
}

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

// ---------------------------------------------------------------------------------------------------------------
// Snapshot and restore (plan P2.3): copy every restorable format before KeyReed borrows the clipboard, put it back
// afterwards in one pass. A clipboard that cannot be copied completely is never touched: `restore` refuses an
// incomplete snapshot, and the caller checks `is_complete` before it overwrites anything.
// Decisions: a format whose data handle is empty or unreadable makes the snapshot incomplete (a password manager's
// exclusion marker is such a format, so a copy from one is never overwritten or kept in memory). Formats Windows
// derives on demand are not stored (plain text from Unicode text, a bitmap from a DIB, a metafile picture from an
// enhanced metafile); enhanced metafiles are saved by their bits.
// Reading a format another program renders on request can block until that program answers; the time limit is
// checked between formats, not inside one read.
// ---------------------------------------------------------------------------------------------------------------

/// Limits for one snapshot (plan H1: 150 ms, 64 MB in total, 8 MB per format).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotLimits {
    pub total_bytes: usize,
    pub format_bytes: usize,
    pub time: Duration,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            total_bytes: 64 << 20,
            format_bytes: 8 << 20,
            time: Duration::from_millis(150),
        }
    }
}

/// Why a format on the clipboard was not copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// A GDI handle, owner-display or private format that cannot be copied as bytes.
    Unsupported,
    /// The data handle was empty or could not be read.
    Unreadable,
    TooLarge,
    TotalLimit,
    TimeLimit,
}

enum Disposition {
    Copy,
    Emf,
    Implied,
    Unsupported,
}

const CF_TEXT: u32 = 1;
const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_OEMTEXT: u32 = 7;
const CF_DIB: u32 = 8;
const CF_PALETTE: u32 = 9;
const CF_UNICODETEXT: u32 = 13;
const CF_ENHMETAFILE: u32 = 14;
const CF_DIBV5: u32 = 17;

/// How a format is handled; `present` is every format on the clipboard.
fn classify(format: u32, present: &[u32]) -> Disposition {
    let has = |f: u32| present.contains(&f);
    match format {
        CF_TEXT | CF_OEMTEXT if has(CF_UNICODETEXT) => Disposition::Implied,
        CF_BITMAP if has(CF_DIB) || has(CF_DIBV5) => Disposition::Implied,
        CF_METAFILEPICT if has(CF_ENHMETAFILE) => Disposition::Implied,
        CF_ENHMETAFILE => Disposition::Emf,
        // Other GDI handles, CF_OWNERDISPLAY and the display variants, and the private and GDI-object ranges.
        CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | 0x80..=0x8E | 0x200..=0x3FF => Disposition::Unsupported,
        _ => Disposition::Copy,
    }
}

/// Whether `size` more bytes may be copied after `used` bytes and `elapsed` time.
fn admit(limits: &SnapshotLimits, used: usize, size: usize, elapsed: Duration) -> Result<(), SkipReason> {
    if elapsed > limits.time {
        Err(SkipReason::TimeLimit)
    } else if size > limits.format_bytes {
        Err(SkipReason::TooLarge)
    } else if used.saturating_add(size) > limits.total_bytes {
        Err(SkipReason::TotalLimit)
    } else {
        Ok(())
    }
}

/// Every copyable format that was on the clipboard, in its original order. The data is wiped from memory on drop.
pub struct ClipboardSnapshot {
    formats: Vec<(u32, Vec<u8>)>,
    skipped: Vec<(u32, SkipReason)>,
    /// `GetClipboardSequenceNumber` at capture time.
    pub sequence: u32,
}

impl Drop for ClipboardSnapshot {
    fn drop(&mut self) {
        for (_, data) in &mut self.formats {
            zeroize::Zeroize::zeroize(data);
        }
    }
}

impl ClipboardSnapshot {
    /// True when no format was left out; only then is it safe to overwrite the clipboard and restore it later.
    pub fn is_complete(&self) -> bool {
        self.skipped.is_empty()
    }

    /// Formats that were not copied, with the reason (format numbers only, never content).
    pub fn skipped(&self) -> &[(u32, SkipReason)] {
        &self.skipped
    }

    pub fn format_ids(&self) -> Vec<u32> {
        self.formats.iter().map(|(f, _)| *f).collect()
    }

    pub fn data(&self, format: u32) -> Option<&[u8]> {
        self.formats.iter().find(|(f, _)| *f == format).map(|(_, d)| d.as_slice())
    }

    /// Copies the clipboard within `limits`. Errors only when the clipboard cannot be opened or listed; a format
    /// that is skipped is reported through `skipped` and makes the snapshot incomplete.
    pub fn capture(limits: SnapshotLimits) -> Result<Self> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::DataExchange::CloseClipboard;

            let start = Instant::now();
            if !open_with_retry(8) {
                bail!("OpenClipboard busy while taking a snapshot");
            }
            let snap = read_open_clipboard(&limits, start);
            CloseClipboard();
            snap
        }
        #[cfg(not(windows))]
        {
            let _ = limits;
            Ok(Self { formats: Vec::new(), skipped: Vec::new(), sequence: 0 })
        }
    }

    /// Puts the clipboard back as captured, in one pass: every handle is built before the clipboard is emptied, so
    /// an allocation failure leaves it alone. Returns the number of formats restored. Refuses an incomplete snapshot.
    pub fn restore(&self) -> Result<usize> {
        if !self.is_complete() {
            bail!(
                "snapshot is incomplete ({} formats not copied); clipboard left untouched",
                self.skipped.len()
            );
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::GlobalFree;
            use windows_sys::Win32::Graphics::Gdi::{DeleteEnhMetaFile, SetEnhMetaFileBits};
            use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, SetClipboardData};
            use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

            let free = |format: u32, handle: *mut std::ffi::c_void| {
                if format == CF_ENHMETAFILE {
                    DeleteEnhMetaFile(handle);
                } else {
                    GlobalFree(handle);
                }
            };
            let mut handles: Vec<(u32, *mut std::ffi::c_void)> = Vec::with_capacity(self.formats.len());
            for (format, data) in &self.formats {
                let handle = if *format == CF_ENHMETAFILE {
                    SetEnhMetaFileBits(data.len() as u32, data.as_ptr())
                } else {
                    let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
                    let p = if h.is_null() { std::ptr::null_mut() } else { GlobalLock(h) as *mut u8 };
                    if p.is_null() {
                        if !h.is_null() {
                            GlobalFree(h);
                        }
                        std::ptr::null_mut()
                    } else {
                        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
                        GlobalUnlock(h);
                        h
                    }
                };
                if handle.is_null() {
                    for (f, h) in handles {
                        free(f, h);
                    }
                    bail!("could not allocate memory to restore the clipboard; clipboard left untouched");
                }
                handles.push((*format, handle));
            }
            if !open_with_retry(12) {
                for (f, h) in handles {
                    free(f, h);
                }
                bail!("OpenClipboard busy while restoring");
            }
            if EmptyClipboard() == 0 {
                CloseClipboard();
                for (f, h) in handles {
                    free(f, h);
                }
                bail!("EmptyClipboard failed while restoring");
            }
            let total = handles.len();
            let mut restored = 0;
            for (format, handle) in handles {
                if SetClipboardData(format, handle).is_null() {
                    free(format, handle);
                } else {
                    restored += 1; // the clipboard owns the handle now
                }
            }
            CloseClipboard();
            if restored != total {
                bail!("restored {restored} of {total} clipboard formats");
            }
            Ok(restored)
        }
        #[cfg(not(windows))]
        {
            Ok(0)
        }
    }
}

#[cfg(windows)]
unsafe fn open_with_retry(attempts: u64) -> bool {
    for attempt in 0..attempts {
        if windows_sys::Win32::System::DataExchange::OpenClipboard(std::ptr::null_mut()) != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2 + attempt * 3));
    }
    false
}

/// Reads every format from the already-open clipboard.
#[cfg(windows)]
unsafe fn read_open_clipboard(limits: &SnapshotLimits, start: Instant) -> Result<ClipboardSnapshot> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::Graphics::Gdi::GetEnhMetaFileBits;
    use windows_sys::Win32::System::DataExchange::{
        EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    let sequence = GetClipboardSequenceNumber();
    let mut present = Vec::new();
    SetLastError(0);
    let mut format = EnumClipboardFormats(0);
    while format != 0 {
        present.push(format);
        format = EnumClipboardFormats(format);
    }
    if GetLastError() != 0 {
        bail!("EnumClipboardFormats failed");
    }

    let mut snap = ClipboardSnapshot { formats: Vec::new(), skipped: Vec::new(), sequence };
    let mut used = 0usize;
    for &format in &present {
        let emf = match classify(format, &present) {
            Disposition::Implied => continue,
            Disposition::Unsupported => {
                snap.skipped.push((format, SkipReason::Unsupported));
                continue;
            }
            Disposition::Emf => true,
            Disposition::Copy => false,
        };
        let read = (|| -> Result<Vec<u8>, SkipReason> {
            admit(limits, used, 0, start.elapsed())?; // do not start another read after the time is up
            let handle = GetClipboardData(format);
            if handle.is_null() {
                return Err(SkipReason::Unreadable);
            }
            let size = if emf { GetEnhMetaFileBits(handle, 0, std::ptr::null_mut()) as usize } else { GlobalSize(handle) };
            if size == 0 {
                return Err(SkipReason::Unreadable);
            }
            admit(limits, used, size, start.elapsed())?;
            let mut buf = vec![0u8; size];
            if emf {
                if GetEnhMetaFileBits(handle, size as u32, buf.as_mut_ptr()) as usize != size {
                    return Err(SkipReason::Unreadable);
                }
            } else {
                let p = GlobalLock(handle) as *const u8;
                if p.is_null() {
                    return Err(SkipReason::Unreadable);
                }
                std::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), size);
                GlobalUnlock(handle);
            }
            Ok(buf)
        })();
        match read {
            Ok(buf) => {
                used += buf.len();
                snap.formats.push((format, buf));
            }
            Err(reason) => snap.skipped.push((format, reason)),
        }
    }
    Ok(snap)
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

    fn limits() -> SnapshotLimits {
        SnapshotLimits { total_bytes: 100, format_bytes: 40, time: Duration::from_millis(150) }
    }

    #[test]
    fn default_limits_match_the_plan() {
        let d = SnapshotLimits::default();
        assert_eq!((d.total_bytes, d.format_bytes, d.time), (64 << 20, 8 << 20, Duration::from_millis(150)));
    }

    #[test]
    fn admit_enforces_each_limit_and_allows_the_boundary() {
        let l = limits();
        let t = Duration::from_millis(10);
        assert_eq!(admit(&l, 0, 40, t), Ok(()));
        assert_eq!(admit(&l, 60, 40, t), Ok(()));
        assert_eq!(admit(&l, 0, 41, t), Err(SkipReason::TooLarge));
        assert_eq!(admit(&l, 61, 40, t), Err(SkipReason::TotalLimit));
        assert_eq!(admit(&l, 0, 1, Duration::from_millis(150)), Ok(()));
        assert_eq!(admit(&l, 0, 1, Duration::from_millis(151)), Err(SkipReason::TimeLimit));
        // time is reported first, then size, then the total
        assert_eq!(admit(&l, 99, 500, Duration::from_secs(1)), Err(SkipReason::TimeLimit));
        assert_eq!(admit(&l, 99, 500, t), Err(SkipReason::TooLarge));
        assert_eq!(admit(&l, usize::MAX, 1, t), Err(SkipReason::TotalLimit));
    }

    #[test]
    fn derived_formats_are_not_stored_and_handles_are_refused() {
        let is = |d: Disposition, want: &str| {
            let got = match d {
                Disposition::Copy => "copy",
                Disposition::Emf => "emf",
                Disposition::Implied => "implied",
                Disposition::Unsupported => "unsupported",
            };
            assert_eq!(got, want);
        };
        is(classify(CF_TEXT, &[CF_TEXT, CF_UNICODETEXT]), "implied");
        is(classify(CF_OEMTEXT, &[CF_OEMTEXT, CF_UNICODETEXT]), "implied");
        is(classify(CF_TEXT, &[CF_TEXT]), "copy");
        is(classify(CF_UNICODETEXT, &[CF_TEXT, CF_UNICODETEXT]), "copy");
        is(classify(CF_BITMAP, &[CF_BITMAP, CF_DIB]), "implied");
        is(classify(CF_BITMAP, &[CF_BITMAP, CF_DIBV5]), "implied");
        is(classify(CF_BITMAP, &[CF_BITMAP]), "unsupported");
        is(classify(CF_DIB, &[CF_BITMAP, CF_DIB, CF_DIBV5]), "copy");
        is(classify(CF_METAFILEPICT, &[CF_METAFILEPICT, CF_ENHMETAFILE]), "implied");
        is(classify(CF_METAFILEPICT, &[CF_METAFILEPICT]), "unsupported");
        is(classify(CF_ENHMETAFILE, &[CF_ENHMETAFILE]), "emf");
        is(classify(CF_PALETTE, &[CF_PALETTE]), "unsupported");
        is(classify(0x80, &[0x80]), "unsupported"); // CF_OWNERDISPLAY
        is(classify(0x200, &[0x200]), "unsupported"); // private range
        is(classify(0x3FF, &[0x3FF]), "unsupported"); // GDI object range
        is(classify(15, &[15]), "copy"); // CF_HDROP
        is(classify(0xC001, &[0xC001]), "copy"); // a registered format
    }

    #[test]
    fn incomplete_snapshot_is_reported_and_never_restored() {
        let mut s = ClipboardSnapshot { formats: vec![(CF_UNICODETEXT, vec![1, 2])], skipped: Vec::new(), sequence: 7 };
        assert!(s.is_complete());
        assert_eq!(s.format_ids(), vec![CF_UNICODETEXT]);
        assert_eq!(s.data(CF_UNICODETEXT), Some(&[1u8, 2][..]));
        assert_eq!(s.data(CF_DIB), None);
        s.skipped.push((0x200, SkipReason::Unsupported));
        assert!(!s.is_complete());
        assert_eq!(s.skipped(), &[(0x200, SkipReason::Unsupported)]);
        // refuses before it opens the clipboard, so this does not touch the real one
        assert!(s.restore().is_err());
    }

    /// Manual harness: writes text, HTML, RTF, a DIB bitmap, a file list and an enhanced metafile to the real
    /// clipboard, snapshots, overwrites, restores and compares byte for byte, then puts the user's own clipboard
    /// back. Run with `cargo test -p vi-win32 clipboard_round_trip -- --ignored` while the PC is otherwise idle.
    #[cfg(windows)]
    #[test]
    #[ignore = "uses the real clipboard"]
    fn clipboard_round_trip_restores_every_format() {
        use windows_sys::Win32::Foundation::GlobalFree;
        use windows_sys::Win32::Graphics::Gdi::{CloseEnhMetaFile, CreateEnhMetaFileW, Rectangle};
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        };
        use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

        struct PutBack(Option<ClipboardSnapshot>);
        impl Drop for PutBack {
            fn drop(&mut self) {
                if let Some(s) = self.0.take() {
                    let _ = s.restore();
                }
            }
        }

        let limits = SnapshotLimits { time: Duration::from_secs(3), ..SnapshotLimits::default() };
        let original = ClipboardSnapshot::capture(limits).expect("snapshot of the user's clipboard");
        if !original.is_complete() {
            eprintln!("skipped: the current clipboard cannot be copied completely, so it is left alone");
            return;
        }
        let _guard = PutBack(Some(original));

        let reg = |name: &str| unsafe {
            let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            RegisterClipboardFormatW(w.as_ptr())
        };
        let text: Vec<u8> = "Quick Fix round trip\0".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let html = b"Version:0.9\r\nStartHTML:0000000105\r\n<html><body><b>round trip</b></body></html>\0".to_vec();
        let rtf = b"{\\rtf1\\ansi round trip}\0".to_vec();
        let mut dib = vec![0u8; 40 + 16];
        dib[0] = 40; // BITMAPINFOHEADER.biSize
        dib[4] = 2; // biWidth
        dib[8] = 2; // biHeight
        dib[12] = 1; // biPlanes
        dib[14] = 32; // biBitCount
        dib[20] = 16; // biSizeImage
        for (i, b) in dib[40..].iter_mut().enumerate() {
            *b = 0x10 + i as u8;
        }
        let mut hdrop = vec![0u8; 20];
        hdrop[0] = 20; // DROPFILES.pFiles
        hdrop[16] = 1; // fWide
        hdrop.extend("C:\\Temp\\a.txt\0\0".encode_utf16().flat_map(|u| u.to_le_bytes()));
        let formats: Vec<(u32, Vec<u8>)> = vec![
            (CF_UNICODETEXT, text),
            (reg("HTML Format"), html),
            (reg("Rich Text Format"), rtf),
            (CF_DIB, dib),
            (15, hdrop),
        ];

        unsafe {
            assert!(OpenClipboard(std::ptr::null_mut()) != 0);
            assert!(EmptyClipboard() != 0);
            for (format, data) in &formats {
                let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
                std::ptr::copy_nonoverlapping(data.as_ptr(), GlobalLock(h) as *mut u8, data.len());
                GlobalUnlock(h);
                if SetClipboardData(*format, h).is_null() {
                    GlobalFree(h);
                    panic!("SetClipboardData({format}) failed");
                }
            }
            let dc = CreateEnhMetaFileW(std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null());
            Rectangle(dc, 1, 1, 40, 30);
            assert!(!SetClipboardData(CF_ENHMETAFILE, CloseEnhMetaFile(dc)).is_null());
            CloseClipboard();
        }

        let before = ClipboardSnapshot::capture(limits).unwrap();
        assert!(before.is_complete(), "skipped: {:?}", before.skipped());
        for (format, data) in &formats {
            assert_eq!(before.data(*format), Some(data.as_slice()), "format {format} was not captured as written");
        }
        assert!(before.data(CF_ENHMETAFILE).is_some_and(|d| !d.is_empty()));
        assert!(before.data(CF_TEXT).is_none() && before.data(CF_BITMAP).is_none(), "derived formats must not be stored");

        write_clipboard_unicode("overwritten by the test", true).unwrap();
        assert_eq!(read_clipboard_unicode().unwrap().as_deref(), Some("overwritten by the test"));

        assert_eq!(before.restore().unwrap(), before.format_ids().len());
        let after = ClipboardSnapshot::capture(limits).unwrap();
        assert!(after.is_complete());
        assert_eq!(after.format_ids(), before.format_ids());
        for id in before.format_ids() {
            assert_eq!(after.data(id), before.data(id), "format {id} differs after the restore");
        }
    }
}
