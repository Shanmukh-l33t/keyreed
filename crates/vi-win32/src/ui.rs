//! Native Win32 System Tray, Non-Activating Layered Floating Pill Window, and Lightweight Settings Window.
//!
//! Zero WebView2 / Electron / GPU bloat:
//! - Idle private RAM of the entire UI shell is ~4-6 MiB!
//! - Settings dialog creates standard Win32 controls only while open and frees them on close.

use crate::overlay::render_pill_bgra_premultiplied;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::Arc;
use vi_core::{
    ActivationMode, AppConfig, EngineState, HotkeySpec, OutputMode, ProcessingProfile, WritingMode,
};

#[derive(Debug, Clone)]
pub enum UiAction {
    ToggleAppEnabled,
    TogglePttEnabled,
    FloatingMicClicked,
    SetActivationMode(ActivationMode),
    SetOutputMode(OutputMode),
    SetWritingMode(WritingMode),
    ToggleAutoContext,
    SetProcessingProfile(ProcessingProfile),
    ToggleFloatingMicVisible,
    SaveUpdatedConfig {
        new_cfg: Box<AppConfig>,
        new_plaintext_api_key: Option<String>,
    },
    ClearHistory,
    Quit,
}

#[derive(Clone)]
pub struct SharedUiModel {
    pub config: AppConfig,
    pub state: EngineState,
    pub rms_level: f32,
    pub status_detail: String,
}

#[cfg(windows)]
pub mod win_gui {
    use super::*;
    use once_cell::sync::OnceCell;
    use vi_core::brand::{
        OVERLAY_WINDOW_CLASS, PRODUCT_NAME, SETTINGS_WINDOW_CLASS, TRAY_WINDOW_CLASS,
    };
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, BLENDFUNCTION, COLOR_WINDOW, DIB_RGB_COLORS, HBITMAP,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_NOSOUND,
        NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub const WM_TRAY_CALLBACK: u32 = WM_USER + 40;
    pub const WM_SYNC_UI: u32 = WM_USER + 41;
    pub const WM_SHOW_TOAST: u32 = WM_USER + 42;
    pub const WM_ALREADY_RUNNING: u32 = vi_core::brand::ALREADY_RUNNING_MSG;

    const IDM_APP_ENABLE: usize = 1001;
    const IDM_PTT_ENABLE: usize = 1002;
    const IDM_HOLD_MODE: usize = 1003;
    const IDM_TOGGLE_MODE: usize = 1004;
    const IDM_OUT_CLIP: usize = 1005;
    const IDM_OUT_TYPE: usize = 1006;
    const IDM_AUTO_CTX: usize = 1007;
    const IDM_SHOW_PILL: usize = 1008;
    const IDM_OPEN_SETTINGS: usize = 1009;
    const IDM_CLEAR_HISTORY: usize = 1010;
    const IDM_EXIT: usize = 1011;

    const IDM_MODE_BASE: usize = 1100;
    const IDM_PROF_BASE: usize = 1200;

    // Settings dialog control IDs
    const IDC_CMB_HOTKEY: i32 = 2001;
    const IDC_CMB_ACT: i32 = 2002;
    const IDC_CMB_OUT: i32 = 2003;
    const IDC_CMB_MODE: i32 = 2004;
    const IDC_CMB_PROF: i32 = 2005;
    const IDC_CMB_ASR: i32 = 2006;
    const IDC_CHK_AUTO_CTX: i32 = 2007;
    const IDC_CHK_PILL: i32 = 2008;
    const IDC_CHK_CLIP_EXCL: i32 = 2009;
    const IDC_CHK_AUTOSTART: i32 = 2010;
    const IDC_EDT_VOCAB: i32 = 2011;
    const IDC_EDT_APIKEY: i32 = 2012;
    const IDC_BTN_SAVE: i32 = 2013;
    const IDC_BTN_CLOSE: i32 = 2014;

    struct WinGuiGlobals {
        tx: Sender<UiAction>,
        model: Arc<Mutex<SharedUiModel>>,
        tray_hwnd: Mutex<isize>,
        pill_hwnd: Mutex<isize>,
        settings_hwnd: Mutex<isize>,
        drag_offset: Mutex<Option<(i32, i32, bool)>>,
        toast_buf: Mutex<Option<(String, String)>>,
    }

    static GUI_GLOBALS: OnceCell<WinGuiGlobals> = OnceCell::new();

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn create_state_hicon(state: &EngineState) -> HICON {
        let (r, g, b) = match state {
            EngineState::Disabled => (120u8, 126u8, 135u8),
            EngineState::Idle => (72u8, 149u8, 239u8),
            EngineState::Listening => (239u8, 68u8, 68u8),
            EngineState::Processing => (245u8, 158u8, 11u8),
            EngineState::Correcting => (14u8, 165u8, 233u8),
            EngineState::ReadyToPaste => (16u8, 185u8, 129u8),
            EngineState::Error(_) => (244u8, 63u8, 94u8),
        };
        let mut color_bits = [0u32; 32 * 32];
        let mut mask_bits = [0u8; (32 * 32) / 8];
        for y in 0..32usize {
            for x in 0..32usize {
                let dx = (x as f32) - 15.5;
                let dy = (y as f32) - 15.5;
                let d = (dx * dx + dy * dy).sqrt();
                if d <= 13.5 {
                    let a = ((13.5 - d + 0.5).clamp(0.0, 1.0) * 255.0) as u32;
                    let (cr, cg, cb) = if d <= 5.5 {
                        (255u32, 255u32, 255u32)
                    } else {
                        (r as u32, g as u32, b as u32)
                    };
                    color_bits[y * 32 + x] = (a << 24) | (cr << 16) | (cg << 8) | cb;
                } else {
                    mask_bits[(y * 32 + x) / 8] |= 1 << (7 - (x % 8));
                }
            }
        }
        let hbm_color = windows_sys::Win32::Graphics::Gdi::CreateBitmap(
            32,
            32,
            1,
            32,
            color_bits.as_ptr() as *const _,
        );
        let hbm_mask = windows_sys::Win32::Graphics::Gdi::CreateBitmap(
            32,
            32,
            1,
            1,
            mask_bits.as_ptr() as *const _,
        );
        let ii = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(hbm_color as _);
        DeleteObject(hbm_mask as _);
        icon
    }

    unsafe fn update_tray_icon(hwnd: HWND, action: u32, state: &EngineState, tip_text: &str) {
        let hicon = create_state_hicon(state);
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;
        nid.hIcon = hicon;

        let tip_w = to_wide(tip_text);
        let copy_len = tip_w.len().min(nid.szTip.len() - 1);
        nid.szTip[..copy_len].copy_from_slice(&tip_w[..copy_len]);

        let _ = Shell_NotifyIconW(action, &nid);
        if !hicon.is_null() {
            let _ = DestroyIcon(hicon);
        }
    }

    unsafe fn show_tray_balloon(hwnd: HWND, title: &str, body: &str) {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;

        let tw = to_wide(title);
        let tlen = tw.len().min(nid.szInfoTitle.len() - 1);
        nid.szInfoTitle[..tlen].copy_from_slice(&tw[..tlen]);

        let bw = to_wide(body);
        let blen = bw.len().min(nid.szInfo.len() - 1);
        nid.szInfo[..blen].copy_from_slice(&bw[..blen]);

        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }

    unsafe fn paint_layered_pill(hwnd: HWND, state: &EngineState, rms: f32) {
        let dpi = {
            let d = GetDpiForWindow(hwnd);
            if d == 0 { 96 } else { d }
        };
        let scale = (dpi as f32) / 96.0;
        let width = ((164.0 * scale).round() as i32).max(96);
        let height = ((38.0 * scale).round() as i32).max(28);

        let bgra = render_pill_bgra_premultiplied(width as usize, height as usize, state, rms);

        let hdc_screen = GetDC(std::ptr::null_mut());
        let hdc_mem = CreateCompatibleDC(hdc_screen);

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height; // top-down DIB
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbm: HBITMAP = CreateDIBSection(
            hdc_mem,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            std::ptr::null_mut(),
            0,
        );
        if !hbm.is_null() && !bits_ptr.is_null() {
            std::ptr::copy_nonoverlapping(
                bgra.as_ptr(),
                bits_ptr as *mut u32,
                bgra.len(),
            );
            let old_bm = SelectObject(hdc_mem, hbm as _);
            let pt_src = POINT { x: 0, y: 0 };
            let sz = SIZE {
                cx: width,
                cy: height,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                hdc_screen,
                std::ptr::null(),
                &sz,
                hdc_mem,
                &pt_src,
                0,
                &blend,
                ULW_ALPHA,
            );
            SelectObject(hdc_mem, old_bm);
            DeleteObject(hbm as _);
        }
        DeleteDC(hdc_mem);
        ReleaseDC(std::ptr::null_mut(), hdc_screen);
    }

    unsafe fn show_context_menu(hwnd: HWND) {
        let Some(g) = GUI_GLOBALS.get() else { return };
        let snap = g.model.lock().clone();

        let hmenu = CreatePopupMenu();
        let append = |menu: HMENU, flags: u32, id: usize, text: &str| {
            let w = to_wide(text);
            let _ = AppendMenuW(menu, flags, id, w.as_ptr());
        };

        let status_hdr = format!("{} - {}", PRODUCT_NAME, snap.state.status_label());
        append(hmenu, MF_STRING | MF_GRAYED, 0, &status_hdr);
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());

        append(
            hmenu,
            MF_STRING | if snap.config.app_enabled { MF_CHECKED } else { 0 },
            IDM_APP_ENABLE,
            "Enable Application",
        );
        append(
            hmenu,
            MF_STRING | if snap.config.ptt_enabled { MF_CHECKED } else { 0 },
            IDM_PTT_ENABLE,
            &format!("Push-to-Talk ({})", snap.config.ptt_hotkey.display_name),
        );

        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        append(
            hmenu,
            MF_STRING
                | if snap.config.activation_mode == ActivationMode::HoldToTalk {
                    MF_CHECKED
                } else {
                    0
                },
            IDM_HOLD_MODE,
            "Activation: Hold to Talk",
        );
        append(
            hmenu,
            MF_STRING
                | if snap.config.activation_mode == ActivationMode::ToggleToTalk {
                    MF_CHECKED
                } else {
                    0
                },
            IDM_TOGGLE_MODE,
            "Activation: Toggle to Talk",
        );

        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        append(
            hmenu,
            MF_STRING
                | if snap.config.output_mode == OutputMode::Clipboard {
                    MF_CHECKED
                } else {
                    0
                },
            IDM_OUT_CLIP,
            "Output: Clipboard (Ctrl+V Default)",
        );
        append(
            hmenu,
            MF_STRING
                | if snap.config.output_mode == OutputMode::DirectTyping {
                    MF_CHECKED
                } else {
                    0
                },
            IDM_OUT_TYPE,
            "Output: Direct Typing",
        );

        // Writing Mode submenu
        let hmode = CreatePopupMenu();
        for (i, m) in WritingMode::ALL.iter().enumerate() {
            let chk = if snap.config.default_writing_mode == *m {
                MF_CHECKED
            } else {
                0
            };
            append(hmode, MF_STRING | chk, IDM_MODE_BASE + i, m.label());
        }
        let _ = AppendMenuW(hmode, MF_SEPARATOR, 0, std::ptr::null());
        append(
            hmode,
            MF_STRING | if snap.config.auto_context_mode { MF_CHECKED } else { 0 },
            IDM_AUTO_CTX,
            "Automatic App-Context Adaptation",
        );
        let w_mode = to_wide("Writing Mode");
        let _ = AppendMenuW(hmenu, MF_POPUP, hmode as usize, w_mode.as_ptr());

        // Processing & Privacy submenu
        let hprof = CreatePopupMenu();
        for (i, p) in [
            ProcessingProfile::MaxPrivacyOffline,
            ProcessingProfile::Balanced,
            ProcessingProfile::MaxAccuracy,
        ]
        .iter()
        .enumerate()
        {
            let chk = if snap.config.processing_profile == *p {
                MF_CHECKED
            } else {
                0
            };
            append(hprof, MF_STRING | chk, IDM_PROF_BASE + i, p.label());
        }
        let w_prof = to_wide("Processing & Privacy");
        let _ = AppendMenuW(hmenu, MF_POPUP, hprof as usize, w_prof.as_ptr());

        append(
            hmenu,
            MF_STRING | if snap.config.show_floating_mic { MF_CHECKED } else { 0 },
            IDM_SHOW_PILL,
            "Show Floating Microphone",
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        append(hmenu, MF_STRING, IDM_OPEN_SETTINGS, "Settings...");
        append(hmenu, MF_STRING, IDM_CLEAR_HISTORY, "Clear Transcription History");
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        append(hmenu, MF_STRING, IDM_EXIT, "Exit");

        let mut pt = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(hmenu, TPM_BOTTOMALIGN | TPM_LEFTALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null());
        let _ = DestroyMenu(hmenu);
    }

    fn hotkey_presets() -> [HotkeySpec; 4] {
        [
            HotkeySpec::right_ctrl(),
            HotkeySpec::default(), // Ctrl + Win
            HotkeySpec::ctrl_shift_space(),
            HotkeySpec {
                ctrl: false,
                alt: true,
                shift: false,
                win: false,
                vk: 0x20,
                display_name: "Alt + Space".to_string(),
            },
        ]
    }

    unsafe fn open_settings_window() {
        let Some(g) = GUI_GLOBALS.get() else { return };
        let existing = *g.settings_hwnd.lock() as HWND;
        if !existing.is_null() && IsWindow(existing) != 0 {
            let _ = ShowWindow(existing, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(existing);
            return;
        }

        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = to_wide(SETTINGS_WINDOW_CLASS);
        let title = to_wide(&format!("{} Settings", PRODUCT_NAME));
        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            520,
            540,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if !hwnd.is_null() {
            *g.settings_hwnd.lock() = hwnd as isize;
        }
    }

    unsafe extern "system" fn settings_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        const CBS_DROPDOWNLIST: u32 = 0x0003;
        const BS_AUTOCHECKBOX: u32 = 0x0003;
        const BS_PUSHBUTTON: u32 = 0x0000;
        const ES_AUTOHSCROLL: u32 = 0x0080;
        const ES_PASSWORD_U32: u32 = 0x0020;
        const CB_ADDSTRING: u32 = 0x0143;
        const CB_SETCURSEL: u32 = 0x014E;
        const CB_GETCURSEL: u32 = 0x0147;
        const BM_SETCHECK: u32 = 0x00F1;
        const BM_GETCHECK: u32 = 0x00F0;

        match msg {
            WM_CREATE => {
                let Some(g) = GUI_GLOBALS.get() else { return 0 };
                let cfg = g.model.lock().config.clone();
                let hinst = GetModuleHandleW(std::ptr::null());
                let font_name = to_wide("Segoe UI");
                let hfont = CreateFontW(
                    -15, 0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 0, 0, font_name.as_ptr(),
                );

                let mk_ctrl = |cls: &str, txt: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32| -> HWND {
                    let cw = to_wide(cls);
                    let tw = to_wide(txt);
                    let ch = CreateWindowExW(
                        0,
                        cw.as_ptr(),
                        tw.as_ptr(),
                        WS_CHILD | WS_VISIBLE | style,
                        x,
                        y,
                        w,
                        h,
                        hwnd,
                        id as isize as HMENU,
                        hinst,
                        std::ptr::null(),
                    );
                    if !ch.is_null() && !hfont.is_null() {
                        SendMessageW(ch, WM_SETFONT, hfont as WPARAM, 1);
                    }
                    ch
                };

                mk_ctrl("STATIC", "Push-to-Talk Shortcut:", 0, 20, 20, 185, 22, 0);
                let cmb_hk = mk_ctrl("COMBOBOX", "", CBS_DROPDOWNLIST | WS_VSCROLL, 215, 16, 265, 160, IDC_CMB_HOTKEY);
                let presets = hotkey_presets();
                let mut hk_idx = 0usize;
                for (i, p) in presets.iter().enumerate() {
                    let s = to_wide(&p.display_name);
                    SendMessageW(cmb_hk, CB_ADDSTRING, 0, s.as_ptr() as LPARAM);
                    if p.display_name == cfg.ptt_hotkey.display_name {
                        hk_idx = i;
                    }
                }
                SendMessageW(cmb_hk, CB_SETCURSEL, hk_idx, 0);

                mk_ctrl("STATIC", "Activation Behaviour:", 0, 20, 56, 185, 22, 0);
                let cmb_act = mk_ctrl("COMBOBOX", "", CBS_DROPDOWNLIST, 215, 52, 265, 120, IDC_CMB_ACT);
                for lbl in ["Hold to Talk (Release to Finish)", "Toggle to Talk (Press Start/Stop)"] {
                    let s = to_wide(lbl);
                    SendMessageW(cmb_act, CB_ADDSTRING, 0, s.as_ptr() as LPARAM);
                }
                SendMessageW(
                    cmb_act,
                    CB_SETCURSEL,
                    if cfg.activation_mode == ActivationMode::HoldToTalk { 0 } else { 1 },
                    0,
                );

                mk_ctrl("STATIC", "Output Delivery:", 0, 20, 92, 185, 22, 0);
                let cmb_out = mk_ctrl("COMBOBOX", "", CBS_DROPDOWNLIST, 215, 88, 265, 120, IDC_CMB_OUT);
                for lbl in ["Clipboard (Ctrl+V Default)", "Direct Typing (Auto Fallback)"] {
                    let s = to_wide(lbl);
                    SendMessageW(cmb_out, CB_ADDSTRING, 0, s.as_ptr() as LPARAM);
                }
                SendMessageW(
                    cmb_out,
                    CB_SETCURSEL,
                    if cfg.output_mode == OutputMode::Clipboard { 0 } else { 1 },
                    0,
                );

                mk_ctrl("STATIC", "Default Writing Mode:", 0, 20, 128, 185, 22, 0);
                let cmb_mode = mk_ctrl("COMBOBOX", "", CBS_DROPDOWNLIST | WS_VSCROLL, 215, 124, 265, 200, IDC_CMB_MODE);
                let mut m_idx = 0usize;
                for (i, m) in WritingMode::ALL.iter().enumerate() {
                    let s = to_wide(m.label());
                    SendMessageW(cmb_mode, CB_ADDSTRING, 0, s.as_ptr() as LPARAM);
                    if *m == cfg.default_writing_mode {
                        m_idx = i;
                    }
                }
                SendMessageW(cmb_mode, CB_SETCURSEL, m_idx, 0);

                mk_ctrl("STATIC", "Privacy & Processing:", 0, 20, 164, 185, 22, 0);
                let cmb_prof = mk_ctrl("COMBOBOX", "", CBS_DROPDOWNLIST, 215, 160, 265, 140, IDC_CMB_PROF);
                let profs = [
                    ProcessingProfile::MaxPrivacyOffline,
                    ProcessingProfile::Balanced,
                    ProcessingProfile::MaxAccuracy,
                ];
                let mut p_idx = 0usize;
                for (i, p) in profs.iter().enumerate() {
                    let s = to_wide(p.label());
                    SendMessageW(cmb_prof, CB_ADDSTRING, 0, s.as_ptr() as LPARAM);
                    if *p == cfg.processing_profile {
                        p_idx = i;
                    }
                }
                SendMessageW(cmb_prof, CB_SETCURSEL, p_idx, 0);

                mk_ctrl("STATIC", "Local Speech Engine:", 0, 20, 200, 185, 22, 0);
                let cmb_asr = mk_ctrl("COMBOBOX", "", CBS_DROPDOWNLIST | WS_VSCROLL, 215, 196, 265, 180, IDC_CMB_ASR);
                let asr_ids = [
                    ("parakeet-tdt-0.6b-v2-int8", "Parakeet TDT 0.6B v2 INT8 (Default)"),
                    ("parakeet-tdt-0.6b-v3-int8", "Parakeet TDT 0.6B v3 INT8 (25 Langs)"),
                    ("moonshine-base-en-int8", "Moonshine Base INT8 (Light 65 MB)"),
                    ("moonshine-tiny-en-int8", "Moonshine Tiny INT8 (Legacy 30 MB)"),
                    ("sense-voice-small-int8", "SenseVoice Small INT8 (Multilingual)"),
                ];
                let mut a_idx = 0usize;
                for (i, (id, lbl)) in asr_ids.iter().enumerate() {
                    let s = to_wide(lbl);
                    SendMessageW(cmb_asr, CB_ADDSTRING, 0, s.as_ptr() as LPARAM);
                    if *id == cfg.local_asr_model {
                        a_idx = i;
                    }
                }
                SendMessageW(cmb_asr, CB_SETCURSEL, a_idx, 0);

                let chk_ctx = mk_ctrl("BUTTON", "Automatic App-Context Writing Adaptation", BS_AUTOCHECKBOX, 20, 238, 450, 22, IDC_CHK_AUTO_CTX);
                SendMessageW(chk_ctx, BM_SETCHECK, usize::from(cfg.auto_context_mode), 0);

                let chk_pill = mk_ctrl("BUTTON", "Show Non-Activating Floating Microphone Pill", BS_AUTOCHECKBOX, 20, 266, 450, 22, IDC_CHK_PILL);
                SendMessageW(chk_pill, BM_SETCHECK, usize::from(cfg.show_floating_mic), 0);

                let chk_excl = mk_ctrl("BUTTON", "Exclude Dictation from Windows Clipboard History & Cloud Sync", BS_AUTOCHECKBOX, 20, 294, 460, 22, IDC_CHK_CLIP_EXCL);
                SendMessageW(chk_excl, BM_SETCHECK, usize::from(cfg.privacy.exclude_from_clipboard_history), 0);

                let chk_auto = mk_ctrl("BUTTON", "Launch Quietly in System Tray at Windows Login", BS_AUTOCHECKBOX, 20, 322, 450, 22, IDC_CHK_AUTOSTART);
                SendMessageW(chk_auto, BM_SETCHECK, usize::from(cfg.autostart_on_login), 0);

                mk_ctrl("STATIC", "Personal Dictionary (comma-separated):", 0, 20, 358, 260, 20, 0);
                let vocab_str = cfg.dictionary.vocabulary.join(", ");
                mk_ctrl("EDIT", &vocab_str, WS_BORDER | ES_AUTOHSCROLL, 20, 380, 460, 26, IDC_EDT_VOCAB);

                mk_ctrl("STATIC", "Optional Cloud API Key (DPAPI Encrypted):", 0, 20, 414, 280, 20, 0);
                mk_ctrl("EDIT", "", WS_BORDER | ES_AUTOHSCROLL | ES_PASSWORD_U32, 295, 412, 185, 24, IDC_EDT_APIKEY);

                mk_ctrl("BUTTON", "Save & Apply", BS_PUSHBUTTON, 270, 455, 105, 32, IDC_BTN_SAVE);
                mk_ctrl("BUTTON", "Cancel", BS_PUSHBUTTON, 385, 455, 95, 32, IDC_BTN_CLOSE);
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as i32;
                if id == IDC_BTN_CLOSE {
                    let _ = DestroyWindow(hwnd);
                    return 0;
                }
                if id == IDC_BTN_SAVE {
                    if let Some(g) = GUI_GLOBALS.get() {
                        let mut new_cfg = g.model.lock().config.clone();
                        let presets = hotkey_presets();
                        let hk_sel = SendMessageW(GetDlgItem(hwnd, IDC_CMB_HOTKEY), CB_GETCURSEL, 0, 0) as usize;
                        if let Some(p) = presets.get(hk_sel) {
                            new_cfg.ptt_hotkey = p.clone();
                        }
                        let act_sel = SendMessageW(GetDlgItem(hwnd, IDC_CMB_ACT), CB_GETCURSEL, 0, 0);
                        new_cfg.activation_mode = if act_sel == 1 {
                            ActivationMode::ToggleToTalk
                        } else {
                            ActivationMode::HoldToTalk
                        };
                        let out_sel = SendMessageW(GetDlgItem(hwnd, IDC_CMB_OUT), CB_GETCURSEL, 0, 0);
                        new_cfg.output_mode = if out_sel == 1 {
                            OutputMode::DirectTyping
                        } else {
                            OutputMode::Clipboard
                        };
                        let mode_sel = SendMessageW(GetDlgItem(hwnd, IDC_CMB_MODE), CB_GETCURSEL, 0, 0) as usize;
                        if let Some(m) = WritingMode::ALL.get(mode_sel) {
                            new_cfg.default_writing_mode = *m;
                        }
                        let prof_sel = SendMessageW(GetDlgItem(hwnd, IDC_CMB_PROF), CB_GETCURSEL, 0, 0);
                        new_cfg.processing_profile = match prof_sel {
                            1 => ProcessingProfile::Balanced,
                            2 => ProcessingProfile::MaxAccuracy,
                            _ => ProcessingProfile::MaxPrivacyOffline,
                        };
                        let asr_ids = [
                            "parakeet-tdt-0.6b-v2-int8",
                            "parakeet-tdt-0.6b-v3-int8",
                            "moonshine-base-en-int8",
                            "moonshine-tiny-en-int8",
                            "sense-voice-small-int8",
                        ];
                        let asr_sel = SendMessageW(GetDlgItem(hwnd, IDC_CMB_ASR), CB_GETCURSEL, 0, 0) as usize;
                        if let Some(aid) = asr_ids.get(asr_sel) {
                            new_cfg.local_asr_model = (*aid).to_string();
                        }

                        new_cfg.auto_context_mode =
                            SendMessageW(GetDlgItem(hwnd, IDC_CHK_AUTO_CTX), BM_GETCHECK, 0, 0) == 1;
                        new_cfg.show_floating_mic =
                            SendMessageW(GetDlgItem(hwnd, IDC_CHK_PILL), BM_GETCHECK, 0, 0) == 1;
                        new_cfg.privacy.exclude_from_clipboard_history =
                            SendMessageW(GetDlgItem(hwnd, IDC_CHK_CLIP_EXCL), BM_GETCHECK, 0, 0) == 1;
                        new_cfg.autostart_on_login =
                            SendMessageW(GetDlgItem(hwnd, IDC_CHK_AUTOSTART), BM_GETCHECK, 0, 0) == 1;

                        let read_edit = |cid: i32| -> String {
                            let he = GetDlgItem(hwnd, cid);
                            let mut buf = [0u16; 1024];
                            let n = GetWindowTextW(he, buf.as_mut_ptr(), buf.len() as i32);
                            if n > 0 {
                                String::from_utf16_lossy(&buf[..n as usize])
                            } else {
                                String::new()
                            }
                        };

                        let vocab_raw = read_edit(IDC_EDT_VOCAB);
                        new_cfg.dictionary.vocabulary = vocab_raw
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();

                        let api_raw = read_edit(IDC_EDT_APIKEY);
                        let new_key = if api_raw.trim().is_empty() {
                            None
                        } else {
                            Some(api_raw)
                        };

                        let _ = g.tx.send(UiAction::SaveUpdatedConfig {
                            new_cfg: Box::new(new_cfg),
                            new_plaintext_api_key: new_key,
                        });
                    }
                    let _ = DestroyWindow(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DESTROY => {
                if let Some(g) = GUI_GLOBALS.get() {
                    *g.settings_hwnd.lock() = 0;
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe extern "system" fn pill_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT, // CRITICAL: Never steal focus from user's target field!
            WM_LBUTTONDOWN => {
                let mut pt = POINT { x: 0, y: 0 };
                let _ = GetCursorPos(&mut pt);
                if let Some(g) = GUI_GLOBALS.get() {
                    *g.drag_offset.lock() = Some((pt.x, pt.y, false));
                }
                SetCapture(hwnd);
                0
            }
            WM_MOUSEMOVE => {
                if let Some(g) = GUI_GLOBALS.get() {
                    let mut drag = g.drag_offset.lock();
                    if let Some((sx, sy, ref mut moved)) = *drag {
                        let mut pt = POINT { x: 0, y: 0 };
                        let _ = GetCursorPos(&mut pt);
                        let dx = pt.x - sx;
                        let dy = pt.y - sy;
                        if dx.abs() > 4 || dy.abs() > 4 || *moved {
                            *moved = true;
                            let mut rc = std::mem::zeroed();
                            if GetWindowRect(hwnd, &mut rc) != 0 {
                                let _ = SetWindowPos(
                                    hwnd,
                                    HWND_TOPMOST,
                                    rc.left + dx,
                                    rc.top + dy,
                                    0,
                                    0,
                                    SWP_NOSIZE | SWP_NOACTIVATE,
                                );
                            }
                            *drag = Some((pt.x, pt.y, true));
                        }
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                if let Some(g) = GUI_GLOBALS.get() {
                    let was_drag = g.drag_offset.lock().take().map(|t| t.2).unwrap_or(false);
                    if !was_drag && g.model.lock().config.floating_mic_clickable {
                        let _ = g.tx.send(UiAction::FloatingMicClicked);
                    }
                }
                0
            }
            WM_RBUTTONUP => {
                if let Some(g) = GUI_GLOBALS.get() {
                    let th = *g.tray_hwnd.lock() as HWND;
                    if !th.is_null() {
                        show_context_menu(th);
                    }
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TRAY_CALLBACK => {
                let ev = (lparam & 0xFFFF) as u32;
                if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                    show_context_menu(hwnd);
                } else if ev == WM_LBUTTONDBLCLK {
                    open_settings_window();
                }
                0
            }
            WM_SYNC_UI => {
                if let Some(g) = GUI_GLOBALS.get() {
                    let snap = g.model.lock().clone();
                    let tip = format!(
                        "{}: {} ({})",
                        PRODUCT_NAME,
                        snap.state.status_label(),
                        snap.config.default_writing_mode.label()
                    );
                    update_tray_icon(hwnd, NIM_MODIFY, &snap.state, &tip);
                    let ph = *g.pill_hwnd.lock() as HWND;
                    if !ph.is_null() {
                        if snap.config.show_floating_mic && snap.config.app_enabled {
                            let _ = ShowWindow(ph, SW_SHOWNOACTIVATE);
                            paint_layered_pill(ph, &snap.state, snap.rms_level);
                        } else {
                            let _ = ShowWindow(ph, SW_HIDE);
                        }
                    }
                }
                0
            }
            WM_SHOW_TOAST => {
                if let Some(g) = GUI_GLOBALS.get() {
                    if let Some((title, body)) = g.toast_buf.lock().take() {
                        show_tray_balloon(hwnd, &title, &body);
                    }
                }
                0
            }
            WM_ALREADY_RUNNING => {
                show_tray_balloon(
                    hwnd,
                    PRODUCT_NAME,
                    "Keyreed is already running. Use the tray icon. A second copy was not started.",
                );
                0
            }
            WM_COMMAND => {
                let id = wparam & 0xFFFF;
                if let Some(g) = GUI_GLOBALS.get() {
                    match id {
                        IDM_APP_ENABLE => {
                            let _ = g.tx.send(UiAction::ToggleAppEnabled);
                        }
                        IDM_PTT_ENABLE => {
                            let _ = g.tx.send(UiAction::TogglePttEnabled);
                        }
                        IDM_HOLD_MODE => {
                            let _ = g.tx.send(UiAction::SetActivationMode(ActivationMode::HoldToTalk));
                        }
                        IDM_TOGGLE_MODE => {
                            let _ = g.tx.send(UiAction::SetActivationMode(ActivationMode::ToggleToTalk));
                        }
                        IDM_OUT_CLIP => {
                            let _ = g.tx.send(UiAction::SetOutputMode(OutputMode::Clipboard));
                        }
                        IDM_OUT_TYPE => {
                            let _ = g.tx.send(UiAction::SetOutputMode(OutputMode::DirectTyping));
                        }
                        IDM_AUTO_CTX => {
                            let _ = g.tx.send(UiAction::ToggleAutoContext);
                        }
                        IDM_SHOW_PILL => {
                            let _ = g.tx.send(UiAction::ToggleFloatingMicVisible);
                        }
                        IDM_OPEN_SETTINGS => open_settings_window(),
                        IDM_CLEAR_HISTORY => {
                            let _ = g.tx.send(UiAction::ClearHistory);
                        }
                        IDM_EXIT => {
                            let _ = g.tx.send(UiAction::Quit);
                            PostQuitMessage(0);
                        }
                        m if (IDM_MODE_BASE..IDM_MODE_BASE + WritingMode::ALL.len()).contains(&m) => {
                            let mode = WritingMode::ALL[m - IDM_MODE_BASE];
                            let _ = g.tx.send(UiAction::SetWritingMode(mode));
                        }
                        p if (IDM_PROF_BASE..IDM_PROF_BASE + 3).contains(&p) => {
                            let prof = match p - IDM_PROF_BASE {
                                1 => ProcessingProfile::Balanced,
                                2 => ProcessingProfile::MaxAccuracy,
                                _ => ProcessingProfile::MaxPrivacyOffline,
                            };
                            let _ = g.tx.send(UiAction::SetProcessingProfile(prof));
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_DESTROY => {
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1;
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    #[derive(Clone)]
    pub struct WinGuiHandle {
        tray_hwnd: isize,
        model: Arc<Mutex<SharedUiModel>>,
    }

    impl WinGuiHandle {
        pub fn sync(&self, state: EngineState, rms: f32, config: Option<AppConfig>) {
            {
                let mut g = self.model.lock();
                g.state = state;
                g.rms_level = rms;
                if let Some(c) = config {
                    g.config = c;
                }
            }
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_SYNC_UI, 0, 0);
            }
        }

        pub fn notify_toast(&self, title: &str, body: &str) {
            if let Some(g) = GUI_GLOBALS.get() {
                *g.toast_buf.lock() = Some((title.to_string(), body.to_string()));
            }
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_SHOW_TOAST, 0, 0);
            }
        }

        pub fn open_settings(&self) {
            unsafe {
                let _ = PostMessageW(
                    self.tray_hwnd as HWND,
                    WM_COMMAND,
                    IDM_OPEN_SETTINGS,
                    0,
                );
            }
        }

        pub fn quit(&self) {
            unsafe {
                let _ = PostMessageW(self.tray_hwnd as HWND, WM_DESTROY, 0, 0);
            }
        }
    }

    pub fn init_and_spawn_gui_windows(
        tx: Sender<UiAction>,
        initial_cfg: AppConfig,
        open_settings_immediately: bool,
    ) -> anyhow::Result<(WinGuiHandle, Box<dyn FnOnce()>)> {
        let model = Arc::new(Mutex::new(SharedUiModel {
            config: initial_cfg.clone(),
            state: if initial_cfg.app_enabled {
                EngineState::Idle
            } else {
                EngineState::Disabled
            },
            rms_level: 0.0,
            status_detail: String::new(),
        }));

        let _ = GUI_GLOBALS.set(WinGuiGlobals {
            tx,
            model: Arc::clone(&model),
            tray_hwnd: Mutex::new(0),
            pill_hwnd: Mutex::new(0),
            settings_hwnd: Mutex::new(0),
            drag_offset: Mutex::new(None),
            toast_buf: Mutex::new(None),
        });

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());

            let reg_class = |name: &str, proc_fn: WNDPROC, bg: isize| {
                let nw = to_wide(name);
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_DBLCLKS,
                    lpfnWndProc: proc_fn,
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinst,
                    hIcon: std::ptr::null_mut(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    hbrBackground: bg as _,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: nw.as_ptr(),
                    hIconSm: std::ptr::null_mut(),
                };
                RegisterClassExW(&wc);
            };

            reg_class(TRAY_WINDOW_CLASS, Some(tray_wnd_proc), 0);
            reg_class(OVERLAY_WINDOW_CLASS, Some(pill_wnd_proc), 0);
            reg_class(SETTINGS_WINDOW_CLASS, Some(settings_wnd_proc), (COLOR_WINDOW + 1) as isize);

            let tray_cls = to_wide(TRAY_WINDOW_CLASS);
            let tray_hwnd = CreateWindowExW(
                0,
                tray_cls.as_ptr(),
                tray_cls.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if tray_hwnd.is_null() {
                anyhow::bail!("Failed to create tray host window");
            }

            // Position floating pill near bottom-center of primary work area
            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let screen_h = GetSystemMetrics(SM_CYSCREEN);
            let pill_x = ((screen_w - 168) / 2).max(20);
            let pill_y = (screen_h - 108).max(40);

            let pill_cls = to_wide(OVERLAY_WINDOW_CLASS);
            let pill_hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                pill_cls.as_ptr(),
                pill_cls.as_ptr(),
                WS_POPUP,
                pill_x,
                pill_y,
                168,
                40,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );

            if let Some(g) = GUI_GLOBALS.get() {
                *g.tray_hwnd.lock() = tray_hwnd as isize;
                *g.pill_hwnd.lock() = pill_hwnd as isize;
            }

            update_tray_icon(
                tray_hwnd,
                NIM_ADD,
                &EngineState::Idle,
                &format!("{} - Ready", PRODUCT_NAME),
            );
            if !pill_hwnd.is_null() && initial_cfg.show_floating_mic && initial_cfg.app_enabled {
                let _ = ShowWindow(pill_hwnd, SW_SHOWNOACTIVATE);
                paint_layered_pill(pill_hwnd, &EngineState::Idle, 0.0);
            }
            if open_settings_immediately {
                open_settings_window();
            }

            let handle = WinGuiHandle {
                tray_hwnd: tray_hwnd as isize,
                model,
            };
            let pump = Box::new(move || {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            });
            Ok((handle, pump))
        }
    }

    pub fn signal_already_running() -> bool {
        use vi_core::brand::TRAY_WINDOW_CLASS;
        let class: Vec<u16> = TRAY_WINDOW_CLASS.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
            if hwnd.is_null() {
                return false;
            }
            PostMessageW(hwnd, WM_ALREADY_RUNNING, 0, 0) != 0
        }
    }
}
