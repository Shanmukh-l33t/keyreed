//! `keyreed --ui-gallery <dir>`: drives the real Win32 listener and Settings window through every
//! visual state and captures the actual on-screen pixels (BitBlt + CAPTUREBLT) as BMP files, plus a
//! JSON report with measured window sizes and timer activity.
//!
//! This exercises rendering and layout only. States are injected through the same `WinGuiHandle`
//! API the coordinator uses; listening levels are synthetic here (the real audio path is covered by
//! the live dictation tests). No coordinator runs, so nothing is saved to the user's config.

#![cfg(windows)]

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vi_core::{AppConfig, EngineState};
use vi_win32::ui::win_gui::{self, WinGuiHandle};
use vi_win32::ui::{DownloadProgress, DownloadStage, MicTestStatus, MicVerdict, ReadyOutcome};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetWindowRect, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

const CAPTUREBLT: u32 = 0x4000_0000;

fn pump_for(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while Instant::now() < until {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if vi_win32::settings::pre_translate(&msg) {
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

fn window_rect(h: HWND) -> RECT {
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetWindowRect(h, &mut rc);
    }
    rc
}

/// Visible frame (excludes Windows 11's invisible resize borders and shadow).
fn frame_rect(h: HWND) -> RECT {
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    let hr = unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            h,
            9, // DWMWA_EXTENDED_FRAME_BOUNDS
            &mut rc as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if hr != 0 {
        return window_rect(h);
    }
    rc
}

/// Captures a screen rectangle (including layered windows) to a 32-bit BMP.
fn capture(x: i32, y: i32, w: i32, h: i32, path: &Path) -> Result<()> {
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old = SelectObject(mem, bmp as _);
        BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        SelectObject(mem, old);
        DeleteObject(bmp as _);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);

        let mut out = Vec::with_capacity(54 + pixels.len());
        let file_size = 54 + pixels.len() as u32;
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&(-h).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&pixels);
        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

struct Gallery {
    dir: PathBuf,
    gui: WinGuiHandle,
    report: Vec<serde_json::Value>,
}

impl Gallery {
    fn pill(&self) -> HWND {
        self.gui.pill_hwnd() as HWND
    }

    fn shot_pill(&mut self, name: &str, note: &str) -> Result<()> {
        let rc = window_rect(self.pill());
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        // A generous crop around the listener so the screenshot shows it in desktop context.
        let cx = (rc.left + rc.right) / 2;
        let cy = (rc.top + rc.bottom) / 2;
        capture(cx - 170, cy - 50, 340, 100, &self.dir.join(format!("{name}.bmp")))?;
        let dbg = win_gui::pill_debug_info();
        self.report.push(serde_json::json!({
            "shot": name,
            "kind": "listener",
            "note": note,
            "window_px": {"w": w, "h": h},
            "animation_timer_running": dbg.map(|d| d.3),
            "frames_painted_total": dbg.map(|d| d.4),
        }));
        Ok(())
    }

    fn shot_settings(&mut self, name: &str, note: &str) -> Result<()> {
        let cls: Vec<u16> = "KeyreedSettingsClass\0".encode_utf16().collect();
        let h = unsafe { FindWindowW(cls.as_ptr(), std::ptr::null()) };
        anyhow::ensure!(!h.is_null(), "Settings window not found");
        let rc = frame_rect(h);
        capture(rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, &self.dir.join(format!("{name}.bmp")))?;
        self.report.push(serde_json::json!({
            "shot": name,
            "kind": "settings",
            "note": note,
            "window_px": {"w": rc.right - rc.left, "h": rc.bottom - rc.top},
        }));
        Ok(())
    }
}

pub fn run_ui_gallery(dir: PathBuf, installed: Vec<String>, mics: Vec<String>, scale_pct: u32) -> Result<()> {
    std::fs::create_dir_all(&dir)?;
    if scale_pct != 100 {
        win_gui::set_dpi_override(96 * scale_pct / 100);
    }
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut cfg = AppConfig::default();
    cfg.first_run_completed = true;
    cfg.floating_pill_pos = None;
    let (gui, _pump) = win_gui::init_and_spawn_gui_windows(tx, cfg, mics.clone(), installed.clone(), false)?;
    let mut g = Gallery { dir: dir.clone(), gui, report: Vec::new() };
    pump_for(600);

    // ---- Listener states ----
    g.gui.sync(EngineState::Idle, 0.0, None);
    pump_for(500);
    g.shot_pill("pill_01_idle", "Idle: icon only")?;
    let idle_timer = win_gui::pill_debug_info().map(|d| d.3).unwrap_or(true);
    let idle_frames_before = win_gui::pill_debug_info().map(|d| d.4).unwrap_or(0);
    pump_for(2000);
    let idle_frames_after = win_gui::pill_debug_info().map(|d| d.4).unwrap_or(0);

    win_gui::set_pill_hover_for_test(true);
    pump_for(450);
    g.shot_pill("pill_02_hover", "Intentional hover: writing mode + shortcut")?;
    win_gui::set_pill_hover_for_test(false);
    pump_for(400);

    // Synthetic speech-like envelope fed through the same sync() path as live capture.
    let env = [0.002f32, 0.01, 0.05, 0.12, 0.2, 0.09, 0.03, 0.15, 0.26, 0.18, 0.07, 0.02, 0.11, 0.22, 0.14, 0.05];
    let t0 = Instant::now();
    let mut i = 0;
    while t0.elapsed() < Duration::from_millis(1400) {
        g.gui.sync(EngineState::Listening, env[i % env.len()], None);
        i += 1;
        pump_for(70);
    }
    g.shot_pill("pill_03_listening", "Listening: bars follow the RMS values passed to sync() (synthetic levels in this gallery)")?;
    g.gui.sync(EngineState::Listening, 0.0, None);
    pump_for(900);
    g.shot_pill("pill_04_listening_silence", "Listening with silence: bars rest at the floor")?;

    g.gui.sync(EngineState::Processing, 0.0, None);
    pump_for(450);
    g.shot_pill("pill_05_processing", "Processing (ASR)")?;
    g.gui.sync(EngineState::Correcting, 0.0, None);
    pump_for(450);
    g.shot_pill("pill_06_polishing", "Processing (grammar polish)")?;

    for (name, outcome) in [
        ("pill_07_done_inserted", ReadyOutcome::Inserted),
        ("pill_07b_done_basic_cleanup", ReadyOutcome::InsertedBasic),
        ("pill_08_done_copied", ReadyOutcome::Copied),
        ("pill_09_review_command", ReadyOutcome::ReviewCommand),
        ("pill_10_discarded", ReadyOutcome::Discarded),
        ("pill_11_nothing_heard", ReadyOutcome::NothingHeard),
    ] {
        g.gui.set_outcome(outcome);
        g.gui.sync(EngineState::ReadyToPaste, 0.0, None);
        pump_for(400);
        g.shot_pill(name, "Completion state")?;
    }
    g.gui.sync(EngineState::Error("Mic error: WASAPI device not found (0x88890004)".into()), 0.0, None);
    pump_for(400);
    g.shot_pill("pill_12_error_mic", "Error: raw WASAPI text is hidden; plain message shown")?;
    g.gui.sync(EngineState::Error("Transcription failed: decoder returned NaN".into()), 0.0, None);
    pump_for(400);
    g.shot_pill("pill_13_error_transcribe", "Error: transcription failure")?;
    g.gui.sync(EngineState::Disabled, 0.0, None);
    pump_for(400);
    g.shot_pill("pill_14_paused", "Paused (app disabled)")?;
    g.gui.sync(EngineState::Idle, 0.0, None);
    g.gui.flash_hint("Professional Writing", "");
    pump_for(400);
    g.shot_pill("pill_15_mode_hint", "Mode switched by shortcut: brief hint instead of a notification")?;
    pump_for(1800);
    g.shot_pill("pill_16_hint_expired", "Hint collapses back to idle by itself")?;

    // ---- Settings pages ----
    vi_win32::settings::open(Some(vi_win32::settings::PAGE_GENERAL));
    pump_for(900);
    let pages = [
        (vi_win32::settings::PAGE_GENERAL, "settings_01_general"),
        (vi_win32::settings::PAGE_DICTATION, "settings_02_dictation"),
        (vi_win32::settings::PAGE_WRITING, "settings_03_writing"),
        (vi_win32::settings::PAGE_OUTPUT, "settings_04_output"),
        (vi_win32::settings::PAGE_AUDIO, "settings_05_microphone"),
        (vi_win32::settings::PAGE_MODELS, "settings_06_models"),
        (vi_win32::settings::PAGE_PROCESSING, "settings_07_processing"),
        (vi_win32::settings::PAGE_PRIVACY, "settings_08_privacy"),
        (vi_win32::settings::PAGE_ABOUT, "settings_09_about"),
    ];
    for (p, name) in pages {
        vi_win32::settings::show_page_for_test(p, 0);
        pump_for(500);
        g.shot_settings(name, "Settings page")?;
    }
    for step in 0..4 {
        vi_win32::settings::show_page_for_test(vi_win32::settings::PAGE_START, step);
        pump_for(500);
        g.shot_settings(&format!("onboarding_{}", step + 1), "First-run setup step")?;
    }

    // Model download states (injected through the same set_download() path the downloader uses).
    vi_win32::settings::show_page_for_test(vi_win32::settings::PAGE_MODELS, 0);
    let target = "whisper-small.en-int8";
    // Show the download states on a model that is genuinely "not installed" from the UI's view.
    let without_target: Vec<String> = installed.iter().filter(|m| !m.eq_ignore_ascii_case(target)).cloned().collect();
    g.gui.update_status("", false, Some(without_target));
    g.gui.set_download(
        Some(DownloadProgress {
            model_id: target.into(),
            stage: DownloadStage::Downloading,
            done_bytes: 141 * 1024 * 1024,
            total_bytes: vi_asr::catalog::find_model_spec(target).map(|m| m.approx_download_mb as u64 * 1024 * 1024),
        }),
        None,
    );
    pump_for(500);
    g.shot_settings("models_downloading", "Download in progress (injected state)")?;
    g.gui.set_download(Some(DownloadProgress { model_id: target.into(), stage: DownloadStage::Verifying, done_bytes: 0, total_bytes: None }), None);
    pump_for(400);
    g.shot_settings("models_verifying", "Checksum verification + extraction (injected state)")?;
    g.gui.set_download(
        Some(DownloadProgress {
            model_id: target.into(),
            stage: DownloadStage::Failed("couldn't reach the download server. Check your connection and try again.".into()),
            done_bytes: 0,
            total_bytes: None,
        }),
        None,
    );
    pump_for(400);
    g.shot_settings("models_failed", "Download failure (injected state)")?;
    g.gui.set_download(None, Some(installed.clone()));

    // Microphone test states.
    vi_win32::settings::show_page_for_test(vi_win32::settings::PAGE_AUDIO, 0);
    g.gui.set_mic_test(MicTestStatus { running: true, level: 0.62, peak_rms: 0.08, device: mics.first().cloned().unwrap_or_default(), ..Default::default() });
    pump_for(400);
    g.shot_settings("mic_testing", "Mic test running (injected level)")?;
    for (name, v, peak) in [("mic_result_good", MicVerdict::Good, 0.11f32), ("mic_result_low", MicVerdict::Low, 0.012), ("mic_result_silent", MicVerdict::Silent, 0.0)] {
        g.gui.set_mic_test(MicTestStatus { running: false, level: 0.0, peak_rms: peak, device: mics.first().cloned().unwrap_or_default(), used_fallback: false, verdict: Some(v), error: None });
        pump_for(400);
        g.shot_settings(name, "Mic test verdict (injected)")?;
    }

    let report = serde_json::json!({
        "generated_by": "keyreed --ui-gallery",
        "scale_percent": scale_pct,
        "idle_animation_timer_running": idle_timer,
        "idle_frames_painted_in_2s": idle_frames_after.saturating_sub(idle_frames_before),
        "shots": g.report,
    });
    std::fs::write(dir.join("gallery_report.json"), serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    g.gui.quit();
    pump_for(200);
    Ok(())
}
