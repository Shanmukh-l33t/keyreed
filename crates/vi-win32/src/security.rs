//! Process hardening, single-instance guard, DPAPI secret storage, and HKCU autostart.

use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;
use zeroize::Zeroize;

/// Tag placed in `dwExtraInfo` on synthetic `SendInput` events so our own `WH_KEYBOARD_LL` hook ignores them.
pub const KEYREED_SYNTHETIC_TAG: usize = 0x4B52_4544; // "KRED"

/// Hardens Win32 DLL search order at process entry to prevent DLL planting / sideloading (MITRE T1574.001)
/// and enables Per-Monitor DPI Awareness v2 so UI and overlay render razor-sharp on all displays.
pub fn harden_and_init_process() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::LibraryLoader::{
            SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32, LOAD_LIBRARY_SEARCH_USER_DIRS,
        };
        use windows_sys::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // Restrict implicit OS DLL lookups to System32 + explicit AddDllDirectory paths (blocks CWD/PATH hijacking).
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

unsafe impl Send for SingleInstanceGuard {}

impl SingleInstanceGuard {
    pub fn acquire(mutex_name: &str) -> Result<Option<Self>> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;

            let wide: Vec<u16> = mutex_name.encode_utf16().chain(std::iter::once(0)).collect();
            let h = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
            if h.is_null() {
                bail!("CreateMutexW failed");
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                windows_sys::Win32::Foundation::CloseHandle(h);
                return Ok(None);
            }
            Ok(Some(Self { handle: h }))
        }
        #[cfg(not(windows))]
        {
            let _ = mutex_name;
            Ok(Some(Self {}))
        }
    }
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

/// Encrypts a sensitive secret (such as a BYOK Cloud API key) using Windows DPAPI (`CryptProtectData`)
/// bound to the current Windows user logon credential.
pub fn dpapi_encrypt_to_b64(plaintext: &str) -> Result<String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let mut bytes = plaintext.as_bytes().to_vec();
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_mut_ptr(),
        };
        let mut out_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = CryptProtectData(
            &in_blob,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        );
        bytes.zeroize();
        if ok == 0 || out_blob.pbData.is_null() {
            bail!("CryptProtectData failed");
        }
        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let encoded = B64.encode(slice);
        LocalFree(out_blob.pbData as _);
        Ok(encoded)
    }
    #[cfg(not(windows))]
    {
        // SECURITY (threat model T10): on non-Windows hosts there is no DPAPI, so we REFUSE to
        // store a secret in plaintext by default. A release build that somehow reaches this path
        // must not silently downgrade to base64. Opt in explicitly for local dev/test only.
        if std::env::var_os("KEYREED_DEV_INSECURE_SECRETS").is_none() {
            bail!(
                "secret storage unavailable: DPAPI is Windows-only; set KEYREED_DEV_INSECURE_SECRETS=1 \
                 to allow an insecure base64 fallback for local development"
            );
        }
        log::warn!("KEYREED_DEV_INSECURE_SECRETS=1: storing secret as plaintext base64 (dev only)");
        Ok(B64.encode(plaintext.as_bytes()))
    }
}

/// Decrypts a DPAPI Base64 blob back into memory. Caller should zeroize the returned String after use.
pub fn dpapi_decrypt_from_b64(encoded_b64: &str) -> Result<String> {
    if encoded_b64.trim().is_empty() {
        return Ok(String::new());
    }
    let mut raw = B64.decode(encoded_b64.trim())?;
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: raw.len() as u32,
            pbData: raw.as_mut_ptr(),
        };
        let mut out_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = CryptUnprotectData(
            &in_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        );
        raw.zeroize();
        if ok == 0 || out_blob.pbData.is_null() {
            bail!("CryptUnprotectData failed");
        }
        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let mut vec_copy = slice.to_vec();
        LocalFree(out_blob.pbData as _);
        let s = String::from_utf8(vec_copy.clone()).unwrap_or_default();
        vec_copy.zeroize();
        Ok(s)
    }
    #[cfg(not(windows))]
    {
        let s = String::from_utf8(raw.clone())?;
        raw.zeroize();
        Ok(s)
    }
}

/// Configures per-user login autostart in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// using a strictly quoted absolute path to prevent unquoted path execution (MITRE T1574.009).
pub fn set_autostart_enabled(value_name: &str, enabled: bool, exe_path: &Path) -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_SET_VALUE, REG_SZ,
        };

        let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
            .encode_utf16()
            .collect();
        let val_w: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hkey: HKEY = std::ptr::null_mut();
        let st = RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_SET_VALUE, &mut hkey);
        if st != 0 {
            bail!("RegOpenKeyExW failed with status {}", st);
        }
        let res = if enabled {
            let quoted = format!("\"{}\" --minimized", exe_path.display());
            let data_w: Vec<u16> = quoted.encode_utf16().chain(std::iter::once(0)).collect();
            let st2 = RegSetValueExW(
                hkey,
                val_w.as_ptr(),
                0,
                REG_SZ,
                data_w.as_ptr() as *const u8,
                (data_w.len() * 2) as u32,
            );
            if st2 != 0 {
                Err(anyhow::anyhow!("RegSetValueExW failed: {}", st2))
            } else {
                Ok(())
            }
        } else {
            let _ = RegDeleteValueW(hkey, val_w.as_ptr());
            Ok(())
        };
        RegCloseKey(hkey);
        res
    }
    #[cfg(not(windows))]
    {
        let _ = (value_name, enabled, exe_path);
        Ok(())
    }
}

/// Appends `unix_ms event` when `KEYREED_TIMING_LOG` is set. No-op otherwise.
pub fn timing_mark(event: &str) {
    let Ok(path) = std::env::var("KEYREED_TIMING_LOG") else {
        return;
    };
    let ms = unix_now_ms();
    write_timing_line(&path, &format!("{ms} {event}"));
}

fn unix_now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn write_timing_line(path: &str, line: &str) {
    use std::io::Write;
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| writeln!(f, "{line}"));
}

/// One monotonic base for a whole dictation: the keyboard hook stamps t0 and the coordinator stamps t1..t5,
/// so both must read the same clock. Nanoseconds since first use, because an `Instant` cannot cross the
/// hook callback without a lock or an allocation. `None` (and nothing read) unless the log is on.
static STAGE_BASE: OnceLock<Option<Instant>> = OnceLock::new();
const STAGE_NOT_SET: u64 = u64::MAX;
static STAGE_PTT_UP: AtomicU64 = AtomicU64::new(STAGE_NOT_SET);
/// A key-up older than this belongs to an earlier press (tap/toggle, cancel, Composer stop), not to the
/// dictation being released now: the coordinator's own instant is then the honest t0.
const HOOK_UP_MAX_AGE_NS: u64 = 1_000_000_000;

/// Read once: the hook calls this on every press and must not touch the environment each time.
fn stage_base() -> Option<Instant> {
    *STAGE_BASE.get_or_init(|| {
        std::env::var("KEYREED_TIMING_LOG").is_ok_and(|p| !p.trim().is_empty()).then(Instant::now)
    })
}

fn stage_now() -> Option<u64> {
    stage_base().map(|b| b.elapsed().as_nanos() as u64)
}

/// Stamps the key-up the hook saw: that release is this dictation's t0.
pub fn stage_hook_ptt_up() {
    if let Some(ns) = stage_now() {
        STAGE_PTT_UP.store(ns, Ordering::Relaxed);
    }
}

/// Forgets any stamped key-up at a new press, so a press that never releases cannot seed a later t0.
pub fn stage_hook_ptt_clear() {
    STAGE_PTT_UP.store(STAGE_NOT_SET, Ordering::Relaxed);
}

/// One dictation's stages in milliseconds from t0; `None` = not reached. t0 is the hook's key-up when it is
/// fresh, else the coordinator's own release instant (labelled `coordinator`). Every mark comes off the same
/// `Instant`, so the series is monotonic even if the wall clock jumps. Dropping it writes the single
/// `dictation_stages` line. t5 is delivery *start*: `SendInput` returning only means the keystrokes were
/// queued, not that the target app has them, so insertion is never claimed as a stage.
pub struct DictationStages {
    id: String,
    t0_ns: u64,
    from_hook: bool,
    ms: [Option<f32>; 6],
}

impl DictationStages {
    /// `None` when the timing log is off, so the caller's field simply stays empty.
    pub fn start() -> Option<Self> {
        let now = stage_now()?;
        let hook = STAGE_PTT_UP.swap(STAGE_NOT_SET, Ordering::Relaxed);
        let hook = (hook != STAGE_NOT_SET && now.saturating_sub(hook) <= HOOK_UP_MAX_AGE_NS).then_some(hook);
        let t0_ns = hook.unwrap_or(now);
        Some(Self {
            // Joins this line to its metrics row: one monotonic instant, one process, one dictation.
            id: format!("{}-{}", std::process::id(), t0_ns),
            t0_ns,
            from_hook: hook.is_some(),
            ms: [Some(0.0), None, None, None, None, None],
        })
    }

    /// The identifier also carried by `PipelineMetrics::dictation_id`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Marks stage `1..=5`: t1 final audio ready, t2 handed to ASR, t3 ASR done, t4 cleanup done,
    /// t5 delivery starting.
    pub fn mark(&mut self, stage: usize) {
        if let (1..=5, Some(now)) = (stage, stage_now()) {
            self.ms[stage] = Some(now.saturating_sub(self.t0_ns) as f32 / 1_000_000.0);
        }
    }
}

impl Drop for DictationStages {
    fn drop(&mut self) {
        // A press that never got a capture is not a dictation: nothing to measure.
        if self.ms[1..].iter().all(Option::is_none) {
            return;
        }
        if let Ok(path) = std::env::var("KEYREED_TIMING_LOG") {
            let src = if self.from_hook { "hook" } else { "coordinator" };
            let line = stages_line(&self.id, &self.ms, src);
            write_timing_line(&path, &format!("{} {line}", unix_now_ms()));
        }
    }
}

fn stages_line(id: &str, ms: &[Option<f32>; 6], source: &str) -> String {
    let mut line = format!("dictation_stages id={id} t0_src={source}");
    for (i, v) in ms.iter().enumerate() {
        match v {
            Some(v) => line.push_str(&format!(" t{i}={v:.1}")),
            None => line.push_str(&format!(" t{i}=na")),
        }
    }
    line
}

/// Like `timing_mark` but safe to call from the keyboard hook callback: the time is taken here and the
/// file write happens on a writer thread, so the hook never blocks on disk. No-op without the env var.
pub fn timing_mark_async(event: &'static str) {
    static TX: std::sync::OnceLock<Option<std::sync::mpsc::Sender<(u128, &'static str)>>> = std::sync::OnceLock::new();
    let tx = TX.get_or_init(|| {
        let path = std::env::var("KEYREED_TIMING_LOG").ok()?;
        let (tx, rx) = std::sync::mpsc::channel::<(u128, &'static str)>();
        std::thread::Builder::new()
            .name("keyreed-timing-log".into())
            .spawn(move || {
                use std::io::Write;
                for (ms, ev) in rx {
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .and_then(|mut f| writeln!(f, "{ms} {ev}"));
                }
            })
            .ok()?;
        Some(tx)
    });
    if let Some(tx) = tx {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = tx.send((ms, event));
    }
}

pub fn read_autostart_value(value_name: &str) -> Result<Option<String>> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
            REG_SZ,
        };
        let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
            .encode_utf16()
            .collect();
        let val_w: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hkey: HKEY = std::ptr::null_mut();
        let st = RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey);
        if st != 0 {
            bail!("RegOpenKeyExW query failed with status {}", st);
        }
        let mut kind = 0u32;
        let mut cb = 0u32;
        let q = RegQueryValueExW(hkey, val_w.as_ptr(), std::ptr::null_mut(), &mut kind, std::ptr::null_mut(), &mut cb);
        if q == 2 {
            RegCloseKey(hkey);
            return Ok(None);
        }
        if q != 0 || kind != REG_SZ || cb < 2 {
            RegCloseKey(hkey);
            bail!("RegQueryValueExW failed status {} kind {} bytes {}", q, kind, cb);
        }
        let mut buf = vec![0u16; (cb as usize) / 2];
        let q2 = RegQueryValueExW(
            hkey,
            val_w.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buf.as_mut_ptr() as *mut u8,
            &mut cb,
        );
        RegCloseKey(hkey);
        if q2 != 0 {
            bail!("RegQueryValueExW read failed {}", q2);
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        Ok(Some(String::from_utf16_lossy(&buf)))
    }
    #[cfg(not(windows))]
    {
        let _ = value_name;
        Ok(None)
    }
}

/// Queries current process Working Set and Private Commit in MiB for live benchmark reporting.
pub fn current_process_memory_mb() -> (f64, f64) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut pmc as *mut _ as *mut _,
            pmc.cb,
        ) != 0
        {
            let ws_mb = (pmc.WorkingSetSize as f64) / (1024.0 * 1024.0);
            let priv_mb = (pmc.PrivateUsage as f64) / (1024.0 * 1024.0);
            return (ws_mb, priv_mb);
        }
        (0.0, 0.0)
    }
    #[cfg(not(windows))]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/status") {
            let mut rss_kb = 0.0f64;
            for line in statm.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    rss_kb = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                }
            }
            return (rss_kb / 1024.0, rss_kb / 1024.0);
        }
        (0.0, 0.0)
    }
}

/// Returns cumulative CPU time (kernel + user) of the current process in milliseconds.
pub fn current_process_cpu_ms() -> f64 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut c, mut e, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) = std::mem::zeroed();
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) != 0 {
            let kt = ((k.dwHighDateTime as u64) << 32) | (k.dwLowDateTime as u64);
            let ut = ((u.dwHighDateTime as u64) << 32) | (u.dwLowDateTime as u64);
            return ((kt + ut) as f64) / 10_000.0;
        }
        0.0
    }
    #[cfg(not(windows))]
    {
        0.0
    }
}

/// Returns available physical system RAM in MiB (`GlobalMemoryStatusEx`).
pub fn system_available_ram_mb() -> u64 {
    #[cfg(windows)]
    unsafe {
        #[repr(C)]
        struct MemoryStatusEx {
            dw_length: u32,
            dw_memory_load: u32,
            ull_total_phys: u64,
            ull_avail_phys: u64,
            ull_total_page_file: u64,
            ull_avail_page_file: u64,
            ull_total_virtual: u64,
            ull_avail_virtual: u64,
            ull_avail_extended_virtual: u64,
        }
        extern "system" {
            fn GlobalMemoryStatusEx(lpBuffer: *mut MemoryStatusEx) -> i32;
        }
        let mut ms: MemoryStatusEx = std::mem::zeroed();
        ms.dw_length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if GlobalMemoryStatusEx(&mut ms) != 0 {
            return ms.ull_avail_phys / (1024 * 1024);
        }
        8_192
    }
    #[cfg(not(windows))]
    {
        8_192
    }
}

/// Appends one JSONL performance trace record when `KEYREED_PERF_TRACE` is set. Never logs transcript text.
pub fn perf_trace_record(json_line: &str) {
    let Ok(path) = std::env::var("KEYREED_PERF_TRACE") else {
        return;
    };
    write_timing_line(&path, json_line);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_stage_line_names_every_stage_and_says_which_were_not_reached() {
        let ms = [Some(0.0), Some(12.5), None, Some(900.04), None, None];
        assert_eq!(
            super::stages_line("4242-1000", &ms, "hook"),
            "dictation_stages id=4242-1000 t0_src=hook t0=0.0 t1=12.5 t2=na t3=900.0 t4=na t5=na"
        );
        assert_eq!(
            super::stages_line("4242-1000", &[Some(0.0); 6], "coordinator"),
            "dictation_stages id=4242-1000 t0_src=coordinator t0=0.0 t1=0.0 t2=0.0 t3=0.0 t4=0.0 t5=0.0"
        );
    }
}
