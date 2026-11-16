# Changelog

## 0.3.0 (2026-11-16)

- **First-Run Onboarding & Live Model Status (`DEF-01`–`DEF-04`)**: Automatically opens the **Settings** window on first launch when no speech model is installed yet, displays live `[Installed Offline]` vs `Click Download` status in the header banner, updates the action button between `Installed`, `Download`, and `Cancel`, and streams real-time MB/% download and extraction progress with cancellation support.
- **Safe Concurrent Model Installation & Pinned-File Extraction (`DEF-01`, `DEF-02`)**: Added `InProcessInstallGuard` (`ACTIVE_IN_PROCESS_INSTALLS`) so `sweep_incomplete()` on the dictation thread never deletes `.staging` while an in-process UI download thread is extracting a model. Updated `extract_tar_bz2_filtered` and `install_verified_archive` to extract only pinned files and prune unpinned archive extras (`test_wavs/`, unused FP32 `.onnx` weights), ensuring all 6 catalog ASR models pass strict `verify_model_dir` checks on every load.
- **All 6 ASR Models Verified End-to-End on Windows 11**: Downloaded, SHA-256 verified, loaded, and benchmarked all 6 speech models (`Parakeet TDT 0.6B v2`, `Parakeet TDT 0.6B v3`, `Moonshine Base`, `Moonshine Tiny`, `SenseVoice Small`, and `Whisper Small.en INT8`), and wired `PersonalDictionary.vocabulary` into Sherpa-ONNX transducer `hotwords_file` (`DEF-05`).
- **Settings Diagnostics & Restoration (`DEF-04`, `DEF-05`)**: Added a **Test Mic** button in Settings that samples the selected microphone for 1.2 seconds and reports peak/RMS signal health, clipping, mute state, or default-device fallback; added a **Reset Defaults** button; widened Settings to `640x680` to eliminate status and combobox label truncation; and added a continuous 50 ms `IDT_PILL_TIMER` for smooth floating-pill animations.
- **Technical & Identifier Preservation Across Writing Modes (`DEF-06`, `DEF-07`, `DEF-015`–`DEF-017`)**: Preserved sentence-initial lowercase on version tags (`v0.2.0`), filenames (`auth_middleware.rs`, `tsconfig.json`, `config.json`), and domains (`github.com`, `support@keyreed.dev`); added spoken Windows path conversion (`C colon backslash Dev backslash keyreed` -> `C:\Dev\keyreed`); stripped stray leading BPE quote artifacts from Moonshine; normalized spoken port/HTTP numbers (`port eighty eighty` -> `port 8080`, `HTTP401` -> `HTTP 401`, `for$1,250` -> `for $1,250`); and hardened `guards.rs` to protect Windows paths, `snake_case` identifiers, and currency symbols (`₹`, `$`, `€`, `£`).
- **Ask Before Inserting & Terminal Safety (`DEF-08`, `DEF-019`)**: Upgraded `Ask Before Inserting` to a 3-button `MB_YESNOCANCEL` prompt (`Yes` = auto-insert, `No` = copy to clipboard only, `Cancel` = dismiss without touching target or pre-existing clipboard), added cross-thread `AttachThreadInput` when focusing deep child editors, and replaced terminal CR/LF sequences with spaces instead of concatenating adjacent lines.
- **In-Place Installer Upgrade & Website Asset Parity (`DEF-020`)**: Updated `scripts/keyreed.nsi` to stop running `keyreed.exe` and `llama-server.exe` instances before upgrading files in place, and synchronized `website/index.html`, `website/site.js`, `docs/installation.md`, and `docs/troubleshooting.md` with `v0.3.0` asset names and SHA-256 checksums.

## 0.2.0 (2026-11-16)

- **Automatic Text Insertion (`Auto Insert`) as Default**: Completed dictations are now automatically typed into the focused target window via batched Win32 `SendInput` (`KEYEVENTF_UNICODE`), with deep child HWND resolution (`RichEditD2DPT` for Windows 11 Tabbed Notepad and nested editors), focus restoration, terminal newline stripping (preventing accidental command execution), and automatic clipboard fallback on password fields, elevated windows (UIPI), or focus changes (TOCTOU).
- **Interactive Ask Before Inserting Mode**: Added a native confirmation dialog displaying the target application and dictated text preview so users can approve direct insertion or copy to clipboard on a per-dictation basis.
- **Critical Local SLM Fix on Windows 11**: Replaced Wine stub MSVC CRT DLLs in `runtime/llama/` with genuine Microsoft-signed VC++ runtime DLLs (`msvcp140.dll`, `msvcp140_1.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) and added fast process-exit detection in `LlamaServerSupervisor`, resolving the `0xC0000142` (`STATUS_DLL_INIT_FAILED`) startup crash and eliminating the 15-second health-check timeout.
- **Polished Brand Assets & Floating Control**: Rebuilt `assets/keyreed-logo.png`, `assets/keyreed-icon.png`, `assets/keyreed.ico`, and embedded `KEYREED_ICON_28_BGRA` with true alpha transparency (removing the baked-in checkerboard background and fixing bottom-left keycap clipping). Added 5-bar live voice waveform animation, hover state, and persistent drag positioning to the non-activating (`WS_EX_NOACTIVATE`) floating pill.
- **Expanded Settings & Tray Experience**: Added Microphone Device selector, Local Speech Engine selector with in-UI **Download** button, Local Grammar Model selector, **Copy Last Dictation** tray command, state-colored tray icon badge, and writable `%LOCALAPPDATA%\keyreed\models` fallback for non-elevated model downloads. Fixed ampersand mnemonic rendering and label widths in Settings.
- **Six Writing Modes & Spoken Symbol Precision**: Improved `Exact`, `Smart`, `Professional`, `Casual`, `Technical`, and `Terminal / Code` differentiation; fixed sentence-period capitalization so filenames (`auth_middleware.rs`) and domains (`github.com`) are never miscapitalized; preserved identifier casing during spoken symbol expansion; added spoken IPv4/semver formatting (`10 dot 0 dot 0 dot 1` -> `10.0.0.1`).
- **Audio & Shortcut Resilience**: Added physical `GetAsyncKeyState` watchdog for fast lost `WM_KEYUP` recovery (~225 ms), automatic fallback to the system default microphone when a preferred USB/Bluetooth microphone is unplugged, peak/clipping diagnostics, and shortcut conflict checks.
- **Lightweight User-Scoped Installer (`17.7 MB`)**: Switched `KeyReed-Setup-x64.exe` to LZMA solid compression and user-scoped default install directory (`%LOCALAPPDATA%\Programs\KeyReed`) with clean Start Menu shortcuts and automated uninstallation.
- **Website & Documentation Rebuilt**: Added light/dark theme website with animated dictation illustration, writing-mode explorer, Wispr Flow comparison, live code excerpts (`scripts/gen-excerpts.py`), and comprehensive architecture, installation, and troubleshooting guides.

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
