# Changelog

## 2.0.0-beta.1 (2026-11-17)

Beta. First public build of the 2.0 line. Quick Fix is the new feature; the dictation path is the 0.5.1 path plus the changes listed below. No dictation benchmark was run for this beta.

### What works

- **Quick Fix (Ctrl + Win + G).** Select text in another application, press the shortcut, and KeyReed shows the proposed corrections in a Review panel. Nothing is replaced until you confirm (`quick_fix_always_review` is on by default; a Settings switch turns on silent replace). Spelling uses the Windows spell checker; grammar uses the Apache-2.0 `gector-bert-en` model (int8, 73 MB download, runs locally through the ONNX Runtime that ships with the app). Undo restores the original text.
- **Safer text handling.** Selection capture and replacement restore your clipboard; a busy language model never blocks dictation (it falls back to basic cleanup); streaming client with limits; the shortcut table supports several chords.
- **Shortcut capture.** Press the keys you want in Settings instead of picking a preset; conflicting or reserved chords are refused.
- **New pill states and Review panel** with short motion that respects reduced-motion.

### Not in this beta

- The Composer (the window that rewrites a selection with a preset such as Shorter or Summarise): the window exists but is not connected, so its shortcut and the pill click do nothing beyond starting dictation.
- Voice commands, learning from your edits, and the "My style" Settings page.
- The deeper grammar pass.
- Quick Fix in Chromium-based apps, Outlook and Word has not been checked by hand; the automated check ran in a plain Windows edit control. Quick Fix grammar edits stay Review-only.
- The default shortcuts (Ctrl + Win + G) have not been checked against every app's own shortcuts; change them in Settings if they clash.
- The build is unsigned.

The Quick Fix grammar model is downloaded from this release (`gector-bert-en-int8.tar.bz2`, SHA-256 pinned in the app). The full offline archive already contains it.

| File | Size | SHA-256 |
| --- | --- | --- |
| `KeyReed-Setup-x64.exe` | 24.2 MB | `2c11bde55ee52bf91d2c762f10709825dd2af553212f51f726acd7d3f0021631` |
| `keyreed-2.0.0-beta.1-windows-x86_64-portable.zip` | 47.0 MB | `4936c20ed3286bb6309e1a47caaad6ba2df6def7e6cd3c3f68c9edec3279c732` |
| `keyreed-2.0.0-beta.1-windows-x64-full-offline.zip` | 1.10 GB | `b87365519ba2662d11debbeb263d01ad5f8ef9717b2e6d77ad5e77473d3f5c5f` |
| `gector-bert-en-int8.tar.bz2` | 73.1 MB | `5654cefc549c938bbac8770d9753a8be50e6d66f9afdc6d7ec746f3d51590ae0` |

## 0.5.1 (2026-11-17)

Design release: new installer, redesigned Settings controls. Speech, grammar and insertion code are unchanged from 0.5.0.

- **Installer**: a frameless KeyReed window replaces the standard setup wizard. Welcome screen with install location and a desktop-shortcut switch; while installing, a waveform turns into lines of text as the real progress advances, with tips about KeyReed underneath; a "KeyReed is ready" screen with a launch switch. The uninstaller uses the same look and says what it keeps (settings, dictionary, downloaded models). Drawn by a small native plugin (`scripts/installer/kr_setup.c`, plain GDI); NSIS still does all the installing, so paths, shortcuts, upgrade and uninstall behave as in 0.5.0. Animation stops when Windows animations are off.
- **Settings controls**: drop-downs and text boxes are drawn by KeyReed instead of the 3D system controls. A drop-down opens the native Windows menu under the field (Up/Down change the value, Alt+Down or F4 open it). Popup menus follow the dark or light theme. Toggles slide (150 ms; instant when Windows animations are off).
- **Speech models page**: SenseVoice and Whisper showed 240 MB and 375 MB downloads; the real archives are 999 MB and 606 MB. A test now keeps the page in line with the pinned catalog. The model in use is marked once, and badges are spaced from the measured name.
- **Listener**: 5 dp gap between the logo and the label or waveform.
- **Installer**: version info is compiled as UTF-8 (the description's em dash was garbled).
- **CI**: Linux build fixed (`zeroize` import), website excerpts regenerated.

## 0.5.0 (2026-11-17)

Speed and accuracy work on the whole dictation path. Measured on an i7-8850H laptop (CPU) with a Quadro P2000; see `docs/research/08-e2e-speed-accuracy-worklog.md` for method, raw numbers and what is not verified.

- **Streaming recognition**: speech is decoded while you are still talking, so a 47 s dictation is ready 0.5-0.7 s after release (v0.4.0: 9.8-26 s). A 7.4 min synthetic dictation finishes in under 2.4 s after release.
- **Paragraphs**: long dictation is split into paragraphs from pauses and topic openers instead of one block of text.
- **Dictation limit** raised from 2 to 30 minutes (longer recordings were silently discarded before).
- **Grammar model**: runs on a discrete GPU (Vulkan) when one is present, measured 2.7-6x faster than CPU, falls back to CPU automatically; skipped for mixed-language text and already-clean sentences. `KEYREED_SLM_DEVICE=cpu` disables the GPU.
- **Tier-0 rules** for common Indian-English constructions; precompiled regexes (about 0.3 ms per dictation).
- **Model readiness**: selected speech model prepared in the background at startup; integrity results cached per process; idle unload lengthened when RAM allows. SHA-256 checks are unchanged.
- **Build**: `scripts/build-release.sh` (the build now fails if `windres` is missing instead of producing a broken exe).
- **Settings dark and light themes**: follows Windows *Choose your default app mode* (`AppsUseLightTheme`), including dark title bar and dark native drop-downs/text boxes; high-contrast mode still uses system colours. WCAG 2.2 AA contrast enforced by unit test across both palettes; sidebar status-chip overlap fixed on short windows.
- **Branded Windows installer**: official KeyReed icon artwork on welcome/header/finish pages and a lightweight native GDI waveform progress view (`KeyReedWave.dll`) driven by real NSIS progress; aborts cleanly with a non-zero exit code if file extraction fails.


## 0.4.0 (2026-11-16)

- **Compact Icon-First Floating Listener (`overlay.rs`, `ui.rs`, `icon_data.rs`)**: Replaced the permanent `196×40` text banner with a quiet, non-activating (`WS_EX_LAYERED | WS_EX_NOACTIVATE`) icon-first capsule derived from the brand palette. At 100% scaling the idle listener is a `36×36` logical (`44×44 px` window) circle showing only the cropped, premultiplied KeyReed mark over a subtle lifted well (`palette::WELL`). It expands smoothly around its horizontal center only when communicating state: `98×36` logical (`107×44 px`) during recording with an 11-bar waveform driven by measured microphone RMS levels, `157×44 px` on deliberate 380 ms hover dwell (showing active writing mode and shortcut), `128×44 px` during transcription (`Loading model` on cold start, `Transcribing` when warm, `Polishing` during Tier-1 grammar rewrite), and distinct semantic completion/attention/notice/error capsules (`Inserted`, `Copied`, amber `Review · Ctrl+V`, neutral `Discarded` / `Nothing heard`, and plain-language rose error badges).
- **Zero-Idle-Timer Animation Architecture & Lower Idle Footprint**: The listener animation timer (`IDT_PILL_ANIM`) now runs strictly while a state transition or waveform/sheen animation is active and turns off completely at idle (`idle_frames_painted_in_2s = 0`). Over a 60-second idle benchmark on Windows 11, CPU time dropped by **38%** (`0.203 s` vs `0.328 s` in v0.3.0, or `0.34%` of one core) and working set dropped by **15.7%** (`16.29 MB` vs `19.32 MB` with GUI initialized; `9.25 MB` shell baseline).
- **10-Page Instant-Apply Settings Interface (`settings.rs`, `canvas.rs`, `keyreed.manifest`)**: Replaced the cramped single-scroll `640×680` dialog with a `942×692` (`1418×1025` at 150% DPI) two-pane native Win32 window featuring a 32-bit DIB immediate-mode canvas, ClearType typography, Common-Controls v6 + Per-Monitor-v2 manifest (`crates/vi-app/keyreed.manifest`), keyboard navigation, screen-reader accessible names, and instant persistence (no manual Save button required). Organised into ten pages: `Get started`, `General`, `Dictation`, `Writing`, `Output`, `Microphone`, `Speech models`, `Processing`, `Privacy`, and `About`.
- **Guided 4-Step First-Run Setup**: Added an interactive onboarding flow (`Welcome & shortcut` → `Microphone & live level test` → `Speech engine download/selection` → `Try it live practice box`) that opens automatically on first launch (`first_run_completed == false`) and lets users test dictation directly inside KeyReed before switching to another app.
- **Measured Speech Model Catalog & Transparent Downloads (`bench/model_speed_v0.4.0.json`)**: Benchmarked all six ASR models across warm runs on this Windows 11 CPU (`Moonshine Tiny` `0.5s`, `Moonshine Base` `0.7s`, `SenseVoice Small` `0.7s`, `Parakeet v2` `1.5s`, `Parakeet v3` `1.7s`, `Whisper Small.en` `7.9s` per 10 s of speech) and surfaced measured speeds, download sizes, offline readiness badges, live MB/% progress bars, cancellation, checksum verification states, and one-click model switching in `Settings › Speech models`.
- **UI Honesty Test & Writing-Mode Examples (`vi-core/src/types.rs`, `vi-rewrite/src/tier0.rs`)**: Added plain-language descriptions and real `You say` → `KeyReed writes` previews for all six writing modes, enforced by unit test `writing_mode_ui_examples_match_real_tier0_output` so UI examples can never drift from real Tier-0 pipeline output. Mode switches via `Ctrl+Shift+Space` now display a quiet 1.6 s pill hint instead of a disruptive tray balloon.
- **Reliability & Latency Fixes Discovered During Live Acoustic & Multi-Monitor Testing**:
  - **Cross-DPI Drag Re-Entrancy Fix (`ui.rs`)**: Made `pill_refresh` re-entrancy safe (`IN_REFRESH` / `PENDING` cell queue) and switched `PILL` handlers to `try_borrow_mut()`, fixing a `RefCell already borrowed` panic when `UpdateLayeredWindow` / `SetWindowPos` synchronously dispatched `WM_DPICHANGED` while dragging across monitors with different DPI scaling (`100%` ↔ `125%`).
  - **Multi-Monitor DPI Anchor & Drag Precision (`ui.rs`)**: Used `lParam` + `ClientToScreen` on `WM_LBUTTONDOWN` and applied final cursor delta on `WM_LBUTTONUP` so fast drags land to the exact pixel; resolved the target monitor's DPI via `GetDpiForMonitor` when restoring saved listener positions on secondary/portrait displays.
  - **Background Model Pre-Warming & Non-Blocking Startup (`main.rs`, `slm.rs`)**: Moved the 500 MB GGUF SHA-256 verification onto a background thread (`keyreed-gguf-verify`) and pre-warmed both ONNX ASR and `llama-server` as soon as push-to-talk recording starts, eliminating the 2.4 s startup hotkey delay and cutting cold first-dictation wait after key release by ~40%.
  - **Silero VAD Soft-Onset Pre-Roll Padding (`vi-asr/src/lib.rs`)**: Widened Silero VAD speech spans by 300 ms pre-roll and 250 ms post-roll (`pad_and_merge_spans`) and read `seg.start` as `u32`, preventing soft sentence-initial words from being clipped during live microphone capture.
  - **Ask Before Inserting Keyboard Focus & Escape Handling (`typing.rs`, `hotkey.rs`)**: Added foreground-lock recovery (`force_foreground_window` + tagged `VK_MENU` unlock) for the `Ask Before Inserting` dialog and exempted KeyReed's own `#32770` dialogs from the global `Esc` emergency-cancel hook so `Enter` (`Yes`) and `Esc` (`Cancel`) work reliably from the keyboard.
  - **Lenient Config Loading & Auto-Enabled Personal Dictionary (`config.rs`, `settings.rs`)**: Added `AppConfig::parse_lenient` so a single invalid field falls back to its default while preserving all other user settings (backing up the raw file to `config.invalid.json`), and automatically set `dictionary.enabled = !vocabulary.is_empty()` when saving personal dictionary terms in Settings.

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
