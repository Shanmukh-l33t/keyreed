# Changelog

## 0.2.0 (2026-11-16)

- **Automatic Text Insertion (`Auto Insert`) as Default**: Completed dictations are now automatically typed into the focused target window via batched Win32 `SendInput` (`KEYEVENTF_UNICODE`), with deep child HWND resolution (`RichEditD2DPT` for Windows 11 Tabbed Notepad and nested editors), focus restoration, terminal newline stripping (preventing accidental command execution), and automatic clipboard fallback on password fields, elevated windows (UIPI), or focus changes (TOCTOU).
- **Interactive Ask Before Inserting Mode**: Added a native confirmation dialog displaying the target application and dictated text preview so users can approve direct insertion or copy to clipboard on a per-dictation basis.
- **Critical Local SLM Fix on Windows 11**: Replaced Wine stub MSVC CRT DLLs in `runtime/llama/` with genuine Microsoft-signed VC++ runtime DLLs (`msvcp140.dll`, `msvcp140_1.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) and added fast process-exit detection in `LlamaServerSupervisor`, resolving the `0xC0000142` (`STATUS_DLL_INIT_FAILED`) startup crash and eliminating the 15-second health-check timeout.
- **Polished Brand Assets & Floating Control**: Rebuilt `assets/keyreed-logo.png`, `assets/keyreed-icon.png`, `assets/keyreed.ico`, and embedded `KEYREED_ICON_28_BGRA` with true alpha transparency (removing the baked-in checkerboard background and fixing bottom-left keycap clipping). Added 5-bar live voice waveform animation, hover state, and persistent drag positioning to the non-activating (`WS_EX_NOACTIVATE`) floating pill.
- **Expanded Settings & Tray Experience**: Added Microphone Device selector, Local Speech Engine selector with in-UI **Download** button, Local Grammar Model selector, **Copy Last Dictation** tray command, state-colored tray icon badge, and writable `%LOCALAPPDATA%\keyreed\models` fallback for non-elevated model downloads.
- **Six Writing Modes & Spoken Symbol Precision**: Improved `Exact`, `Smart`, `Professional`, `Casual`, `Technical`, and `Terminal / Code` differentiation; fixed sentence-period capitalization so filenames (`auth_middleware.rs`) and domains (`github.com`) are never miscapitalized; preserved identifier casing during spoken symbol expansion; added spoken IPv4/semver formatting (`10 dot 0 dot 0 dot 1` -> `10.0.0.1`).
- **Audio & Shortcut Resilience**: Added physical `GetAsyncKeyState` watchdog for fast lost `WM_KEYUP` recovery (~225 ms), automatic fallback to the system default microphone when a preferred USB/Bluetooth microphone is unplugged, peak/clipping diagnostics, and shortcut conflict checks.
- **Lightweight User-Scoped Installer (`17.7 MB`)**: Switched `KeyReed-Setup-x64.exe` to LZMA solid compression and user-scoped default install directory (`%LOCALAPPDATA%\Programs\KeyReed`) with clean Start Menu shortcuts and automated uninstallation.

## 0.1.1

- The Windows build is a GUI application. Closing a terminal no longer ends the tray process.
- The standard installer no longer embeds speech or grammar models. The tray menu downloads the default speech model and checks the compile-time pins.
- Auto Insert is available and recommended. It stays off until the user turns it on. Clipboard only remains the stored default.
- Ask before inserting copies the text and does not type into a window.
- The tray icon loads `keyreed.ico` from the install folder when that file is present.

## 0.1.0

First public Windows package.

- Installer and portable archive include the application, speech runtime, default Parakeet speech model, and default Qwen3.5 grammar model.
- Hold to talk, toggle to talk, six writing modes, clipboard output, and optional direct insertion remain in the application.
- A lost key release or the 120 second cap discards the recording and does not paste.
- A second launch notifies the running tray copy and does not start another listener.
- Model files are checked against compile-time size and SHA-256 pins before load.
- Windows binaries in this release are unsigned.
