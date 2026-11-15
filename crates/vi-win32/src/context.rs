//! Minimal, privacy-controlled Windows Application Context Engine (Master Prompt Section 6 & 12).
//!
//! Identifies active process, UWP child process (`ApplicationFrameHost.exe`), window class,
//! focused child control style (`ES_PASSWORD`), and process elevation (`TokenElevation`)
//! without ever scraping document contents or screenshots.

use vi_core::{AppCategory, ForegroundContext};

const PASSWORD_MANAGERS: &[&str] = &[
    "keepass.exe",
    "keepassxc.exe",
    "1password.exe",
    "bitwarden.exe",
    "dashlane.exe",
    "nordpass.exe",
];

const TERMINALS: &[&str] = &[
    "windowsterminal.exe",
    "wt.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "kitty.exe",
    "mintty.exe",
    "putty.exe",
    "mobaxterm.exe",
];

const CODE_EDITORS: &[&str] = &[
    "code.exe",
    "cursor.exe",
    "windsurf.exe",
    "zed.exe",
    "devenv.exe",
    "idea64.exe",
    "pycharm64.exe",
    "clion64.exe",
    "goland64.exe",
    "rustrover64.exe",
    "rider64.exe",
    "webstorm64.exe",
    "sublime_text.exe",
    "notepad++.exe",
    "nvim-qt.exe",
];

const EMAIL_CLIENTS: &[&str] = &[
    "outlook.exe",
    "olk.exe",
    "thunderbird.exe",
    "mailspring.exe",
    "spark desktop.exe",
];

const CHAT_APPS: &[&str] = &[
    "slack.exe",
    "teams.exe",
    "ms-teams.exe",
    "discord.exe",
    "whatsapp.exe",
    "telegram.exe",
    "signal.exe",
    "mattermost.exe",
    "element.exe",
    "zoom.exe",
];

const NOTES_APPS: &[&str] = &[
    "obsidian.exe",
    "notion.exe",
    "onenote.exe",
    "logseq.exe",
    "joplin.exe",
    "evernote.exe",
];

const DOC_EDITORS: &[&str] = &[
    "winword.exe",
    "wordpad.exe",
    "soffice.bin",
    "swriter.exe",
    "notepad.exe",
    "acrobat.exe",
];

const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "vivaldi.exe",
    "arc.exe",
    "opera.exe",
];

/// Pure classification logic (unit-testable on any OS).
pub fn classify_application(
    process_name: &str,
    window_class: &str,
    window_title: &str,
    is_password_field: bool,
) -> AppCategory {
    if is_password_field {
        return AppCategory::PasswordProtected;
    }
    let proc_l = process_name.trim().to_lowercase();
    let cls_l = window_class.trim().to_lowercase();
    let title_l = window_title.trim().to_lowercase();

    if PASSWORD_MANAGERS.iter().any(|p| proc_l == *p) {
        return AppCategory::PasswordProtected;
    }
    if TERMINALS.iter().any(|p| proc_l == *p)
        || cls_l.contains("cascadia_hosting_window_class")
        || cls_l == "consolewindowclass"
    {
        return AppCategory::Terminal;
    }
    if CODE_EDITORS.iter().any(|p| proc_l == *p) {
        return AppCategory::CodeEditor;
    }
    if EMAIL_CLIENTS.iter().any(|p| proc_l == *p) {
        return AppCategory::Email;
    }
    if CHAT_APPS.iter().any(|p| proc_l == *p) {
        return AppCategory::Chat;
    }
    if NOTES_APPS.iter().any(|p| proc_l == *p) {
        return AppCategory::Notes;
    }
    if DOC_EDITORS.iter().any(|p| proc_l == *p) {
        return AppCategory::Document;
    }
    if BROWSERS.iter().any(|p| proc_l == *p) {
        // Lightweight title hints when permitted (e.g. Gmail / Outlook Web / Slack / GitHub / Docs)
        if title_l.contains("gmail") || title_l.contains("outlook") || title_l.contains("proton mail") {
            return AppCategory::Email;
        }
        if title_l.contains("slack")
            || title_l.contains("discord")
            || title_l.contains("whatsapp")
            || title_l.contains("microsoft teams")
        {
            return AppCategory::Chat;
        }
        if title_l.contains("google docs") || title_l.contains("word") || title_l.contains("overleaf") {
            return AppCategory::Document;
        }
        if title_l.contains("notion") || title_l.contains("confluence") {
            return AppCategory::Notes;
        }
        return AppCategory::Browser;
    }
    AppCategory::General
}

pub fn is_private_browsing_title(title: &str) -> bool {
    let tl = title.to_lowercase();
    tl.contains("inprivate") || tl.contains("incognito") || tl.contains("private browsing")
}

/// Captures a minimal, privacy-safe snapshot of the current foreground window on Windows.
pub fn capture_foreground_context(redact_sensitive_titles: bool) -> ForegroundContext {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM};
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumChildWindows, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
            GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, GUITHREADINFO, GWL_STYLE,
        };

        const ES_PASSWORD: i32 = 0x0020;

        let hwnd: HWND = GetForegroundWindow();
        if hwnd.is_null() {
            return ForegroundContext::default();
        }

        let mut pid: u32 = 0;
        let tid = GetWindowThreadProcessId(hwnd, &mut pid);

        let query_exe_name = |target_pid: u32| -> (String, bool) {
            if target_pid == 0 {
                return (String::new(), false);
            }
            let hproc: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, target_pid);
            if hproc.is_null() {
                // Access denied on OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) typically indicates
                // a protected/system/elevated process boundary.
                return (String::new(), true);
            }
            let mut buf = [0u16; 512];
            let mut sz = buf.len() as u32;
            let name = if QueryFullProcessImageNameW(hproc, 0, buf.as_mut_ptr(), &mut sz) != 0 {
                let full = String::from_utf16_lossy(&buf[..sz as usize]);
                full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string()
            } else {
                String::new()
            };

            let mut elevated = false;
            let mut htok: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(hproc, TOKEN_QUERY, &mut htok) != 0 && !htok.is_null() {
                let mut elev: TOKEN_ELEVATION = std::mem::zeroed();
                let mut ret_len: u32 = 0;
                if GetTokenInformation(
                    htok,
                    TokenElevation,
                    &mut elev as *mut _ as *mut _,
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut ret_len,
                ) != 0
                {
                    elevated = elev.TokenIsElevated != 0;
                }
                CloseHandle(htok);
            }
            CloseHandle(hproc);
            (name, elevated)
        };

        let (mut proc_name, mut is_elevated) = query_exe_name(pid);

        // Resolve real child UWP process hosted inside `ApplicationFrameHost.exe`
        if proc_name.eq_ignore_ascii_case("ApplicationFrameHost.exe") {
            struct ChildSearch {
                host_pid: u32,
                found_pid: u32,
            }
            unsafe extern "system" fn enum_cb(child: HWND, lparam: LPARAM) -> BOOL {
                let st = &mut *(lparam as *mut ChildSearch);
                let mut cpid: u32 = 0;
                GetWindowThreadProcessId(child, &mut cpid);
                if cpid != 0 && cpid != st.host_pid {
                    st.found_pid = cpid;
                    return 0;
                }
                1
            }
            let mut search = ChildSearch {
                host_pid: pid,
                found_pid: 0,
            };
            EnumChildWindows(hwnd, Some(enum_cb), &mut search as *mut _ as LPARAM);
            if search.found_pid != 0 {
                pid = search.found_pid;
                let (uwp_name, uwp_elev) = query_exe_name(pid);
                if !uwp_name.is_empty() {
                    proc_name = uwp_name;
                    is_elevated = uwp_elev;
                }
            }
        }

        let mut cls_buf = [0u16; 128];
        let cls_len = GetClassNameW(hwnd, cls_buf.as_mut_ptr(), cls_buf.len() as i32);
        let window_class = if cls_len > 0 {
            String::from_utf16_lossy(&cls_buf[..cls_len as usize])
        } else {
            String::new()
        };

        let mut title_buf = [0u16; 256];
        let title_len = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32);
        let raw_title = if title_len > 0 {
            String::from_utf16_lossy(&title_buf[..title_len as usize])
        } else {
            String::new()
        };

        // Check focused child control class and ES_PASSWORD style via GetGUIThreadInfo (fast, non-blocking!)
        let mut gti: GUITHREADINFO = std::mem::zeroed();
        gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let mut control_type = String::new();
        let mut is_password_field = false;
        let mut focus_hwnd: HWND = std::ptr::null_mut();
        if tid != 0 && GetGUIThreadInfo(tid, &mut gti) != 0 && !gti.hwndFocus.is_null() {
            focus_hwnd = gti.hwndFocus;
        }
        // In Windows 11 XAML Islands apps (e.g. modern Notepad), the actual editor child (`RichEditD2DPT`)
        // may live inside a nested child container (`NotepadTextBox`).
        if focus_hwnd.is_null() || focus_hwnd == hwnd {
            if let Some(deep) = find_deep_editor_hwnd(hwnd) {
                focus_hwnd = deep;
            }
        }
        if !focus_hwnd.is_null() {
            let mut fcls = [0u16; 96];
            let flen = GetClassNameW(focus_hwnd, fcls.as_mut_ptr(), fcls.len() as i32);
            if flen > 0 {
                control_type = String::from_utf16_lossy(&fcls[..flen as usize]);
            }
            let style = GetWindowLongW(focus_hwnd, GWL_STYLE);
            if (style & ES_PASSWORD) != 0 || control_type.to_lowercase().contains("password") {
                is_password_field = true;
            }
        }
        let raw_title_low = raw_title.to_lowercase();
        if raw_title_low.contains("password")
            || raw_title_low.contains("1password")
            || raw_title_low.contains("keepass")
            || raw_title_low.contains("bitwarden")
        {
            is_password_field = true;
        }

        let category = classify_application(&proc_name, &window_class, &raw_title, is_password_field);

        let window_title_hint = if redact_sensitive_titles
            && (category == AppCategory::PasswordProtected || is_private_browsing_title(&raw_title))
        {
            "[Redacted Private Window]".to_string()
        } else {
            raw_title.chars().take(80).collect()
        };

        ForegroundContext {
            hwnd: hwnd as usize as u64,
            process_id: pid,
            process_name: proc_name,
            window_class,
            window_title_hint,
            control_type,
            is_password_field,
            is_elevated,
            category,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = redact_sensitive_titles;
        ForegroundContext::default()
    }
}

/// Recursively searches child windows of `parent` for a known text editor surface
/// (`RichEditD2DPT`, `Edit`, `RICHEDIT50W`, `Scintilla`). Used for Windows 11 tabbed Notepad
/// (`Notepad` -> `NotepadTextBox` -> `RichEditD2DPT`) and classic Win32 editors.
#[cfg(windows)]
pub fn find_deep_editor_hwnd(
    parent: windows_sys::Win32::Foundation::HWND,
) -> Option<windows_sys::Win32::Foundation::HWND> {
    unsafe {
        use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW, IsWindowVisible};

        struct EditorSearch {
            found: HWND,
        }
        unsafe extern "system" fn enum_editor_cb(child: HWND, lparam: LPARAM) -> BOOL {
            let st = &mut *(lparam as *mut EditorSearch);
            if IsWindowVisible(child) == 0 {
                return 1;
            }
            let mut cls_buf = [0u16; 96];
            let len = GetClassNameW(child, cls_buf.as_mut_ptr(), cls_buf.len() as i32);
            if len > 0 {
                let cls = String::from_utf16_lossy(&cls_buf[..len as usize]);
                let low = cls.to_lowercase();
                if low == "richeditd2dpt"
                    || low == "edit"
                    || low.starts_with("richedit")
                    || low == "scintilla"
                {
                    st.found = child;
                    return 0;
                }
            }
            1
        }
        let mut search = EditorSearch {
            found: std::ptr::null_mut(),
        };
        EnumChildWindows(parent, Some(enum_editor_cb), &mut search as *mut _ as LPARAM);
        if search.found.is_null() {
            None
        } else {
            Some(search.found)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_windows_apps_accurately() {
        assert_eq!(
            classify_application("WindowsTerminal.exe", "CASCADIA_HOSTING_WINDOW_CLASS", "PowerShell", false),
            AppCategory::Terminal
        );
        assert_eq!(
            classify_application("Code.exe", "Chrome_WidgetWin_1", "main.rs - Visual Studio Code", false),
            AppCategory::CodeEditor
        );
        assert_eq!(
            classify_application("OUTLOOK.EXE", "rctrl_renwnd32", "Inbox", false),
            AppCategory::Email
        );
        assert_eq!(
            classify_application("msedge.exe", "Chrome_WidgetWin_1", "Inbox - Gmail - Microsoft Edge", false),
            AppCategory::Email
        );
        assert_eq!(
            classify_application("KeePassXC.exe", "Qt653QWindowIcon", "Passwords", false),
            AppCategory::PasswordProtected
        );
        assert_eq!(
            classify_application("notepad.exe", "Notepad", "Untitled", true),
            AppCategory::PasswordProtected
        );
    }
}
