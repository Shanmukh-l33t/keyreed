# KeyReed

**Voice it. Everywhere.**

KeyReed is a native, offline-first Windows 10/11 voice dictation and AI writing assistant. Hold a shortcut (`Right Ctrl` by default), speak naturally, release, and KeyReed transcribes your speech, applies your selected writing mode, and **automatically inserts the finished text directly into the focused application** (with automatic clipboard preservation and fallback when a target field is protected, elevated, or unfocused).

The Windows build in this release is unsigned. Windows SmartScreen may warn before you run it. Signing does not by itself create SmartScreen reputation, and this release has not been observed in SmartScreen.

## Download (`v0.2.0` — Windows x64)

| File | Size | SHA-256 | Description |
| --- | --- | --- | --- |
| [`KeyReed-Setup-x64.exe`](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.2.0/KeyReed-Setup-x64.exe) | 18,589,388 bytes (~17.7 MB) | `1190bc30a7d5220825cdfc1868cb563d8580d80f6a616c7ba6274c1ed9067b21` | **Standard Lightweight Installer** (includes `keyreed.exe`, `sherpa-onnx` & `llama-server` runtimes, and `silero_vad.onnx`; downloads your chosen speech model on demand in Settings or tray menu) |
| [`keyreed-0.2.0-windows-x86_64-portable.zip`](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.2.0/keyreed-0.2.0-windows-x86_64-portable.zip) | 32,262,155 bytes (~30.8 MB) | `ac4b6e1bd4b6f3821de9cb04ae634adee5c94165912415e76fa40af7c919a5f4` | **Standard Portable Archive** (no installation required) |
| [`keyreed-0.2.0-windows-x64-full-offline.zip`](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.2.0/keyreed-0.2.0-windows-x64-full-offline.zip) | 1,245,412,519 bytes (~1.16 GB) | `ea9cc2513faba3baa79a33795c051ed9cc6ec82662f5d77d14b267edbde6da62` | **Full Offline Package** (bundles `Parakeet TDT 0.6B v2 INT8` speech model + `Qwen3.5-0.8B-Q4_K_M` grammar model + runtimes for 100% air-gapped offline operation) |
| [`SHA256SUMS.txt`](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.2.0/SHA256SUMS.txt) | 414 bytes | — | SHA-256 manifest for all release artifacts |

## Install

1. Download `KeyReed-Setup-x64.exe`.
2. Double-click it. Windows may show an unknown-publisher prompt because the file is unsigned.
3. Choose the install folder (defaults to `%LOCALAPPDATA%\Programs\KeyReed` so no Administrator elevation is required).
4. Finish. Leave **Launch KeyReed** checked to start immediately in the system tray and floating control.
5. Start Menu shortcuts (`KeyReed`, `KeyReed Settings`, and `Uninstall KeyReed`) are created automatically.

You do not need Rust, Python, Visual Studio, or an open command prompt/PowerShell window.

## Uninstall

Use **Windows Settings → Apps → Installed apps → KeyReed**, or the Start Menu entry **Uninstall KeyReed**.

Uninstallation cleanly terminates any running `keyreed.exe` instance, removes the installed program files, runtime DLLs, and shortcuts, and clears the `HKCU\...\Run` startup entry while leaving user preferences in `%LOCALAPPDATA%\keyreed` intact.

## Workflow & First Use

1. Launch **KeyReed** from the Start Menu. It runs independently in the background with a system tray icon and a compact, non-activating floating control pill.
2. Open **Settings** by double-clicking the tray icon or right-clicking the tray icon / floating pill.
3. If you installed the standard lightweight setup (`KeyReed-Setup-x64.exe`), select your preferred speech model in Settings (default: **Parakeet TDT 0.6B v2**) and click **Download** (or choose **Download Default Speech Model** from the tray menu). Every downloaded model is verified against compile-time file-size and SHA-256 pins before activation.
4. Click inside any text field (Notepad, browser, chat client, code editor, email, or document).
5. Hold **Right Ctrl** (or your configured shortcut), speak naturally, and release.
6. KeyReed transcribes your speech, applies your selected Writing Mode, and **automatically inserts the text into the focused field** (or copies to clipboard with a notification if the field is a password box, elevated across UIPI, or lost focus).

## Core Capabilities

- **Three Output Delivery Modes**:
  1. **Auto Insert into Active App (Default)**: Automatically types Unicode and multiline text into the focused window (`SendInput` `KEYEVENTF_UNICODE` with deep child `RichEditD2DPT` / `EDIT` focus resolution, terminal newline sanitization, and automatic clipboard backup).
  2. **Clipboard Only (`Ctrl+V`)**: Places the completed dictation on the clipboard without synthesizing keystrokes.
  3. **Ask Before Inserting**: Displays a confirmation preview showing the target application and transcribed text before inserting or copying to clipboard.
- **Six Writing Modes**:
  1. **Smart Writing**: Removes speech fillers (`um`, `uh`, false starts), repairs grammar, and formats punctuation naturally.
  2. **Exact Dictation**: Preserves your exact wording while adding clean capitalization and punctuation.
  3. **Professional Writing**: Expands casual contractions and formats polished business prose.
  4. **Casual Writing**: Keeps warm, conversational contractions (`I'm`, `don't`, `can't`) for chat and messaging.
  5. **Technical**: Preserves technical identifiers (`OAuth2`, `gRPC`, `PostgreSQL`, `auth_middleware.rs`, `10.0.0.1`, `v1.2.0`) and removes conversational hedging.
  6. **Terminal / Code**: Translates spoken symbols (`dash n`, `pipe`, `double dash`, `dot rs`) into exact CLI syntax (`kubectl get pods -n prod -o wide`) and strips trailing newlines so commands are never auto-executed in terminals.
- **Six Verified Local ONNX ASR Models**:
  - `parakeet-tdt-0.6b-v2-int8` (Default English, 652 MB)
  - `parakeet-tdt-0.6b-v3-int8` (Multilingual 25 languages, 660 MB)
  - `moonshine-base-en-int8` (Ultra-light English, 290 MB)
  - `moonshine-tiny-en-int8` (Low-RAM English, 120 MB)
  - `sense-voice-small-int8` (Multilingual + Asian/Indian English ITN, 238 MB)
  - `whisper-small.en-int8` (Whisper Small.en INT8, 370 MB)
- **Global Shortcuts & Hardware Audio Resilience**:
  - Supports **Hold-to-Talk** and **Toggle-to-Talk** across 7 configurable shortcut presets (`Right Ctrl`, `Right Alt`, `Ctrl+Shift+Space`, `Alt+Space`, `Ctrl+Win`, `F8`, `F9`), plus `Esc` emergency cancellation and `Ctrl+Shift+M` mode cycling.
  - Includes a physical `GetAsyncKeyState` watchdog that recovers from swallowed `WM_KEYUP` events within ~225 ms, plus automatic fallback to the system default microphone if a preferred USB/Bluetooth headset is unplugged.

## Measured Windows 11 Performance

Measured natively on Windows 11 x64 (`KeyReed v0.2.0`, CPU-only inference, 6.84 s speech audio fixture):

- **Idle Daemon Memory**: `8.75 MB` Working Set (`1.49 MB` Private Bytes) when models are unloaded.
- **Microphone Open Latency**: `50.1 ms` (`Microphone (OWL 4K PRO Audio)` at 48 kHz stereo → 16 kHz mono).
- **Warm Speech-to-Text (`Parakeet TDT 0.6B v2 INT8`)**: `700 ms – 1,359 ms` for 6.84 s audio (`~0.12x – 0.20x` RTF) + `48 ms – 120 ms` Silero VAD.
- **Grammar Polish Latency**:
  - **Tier 0 Deterministic Engine**: `20.8 ms` median (`30.1 ms` p90) across the 88-case evaluation benchmark (`100.0%` invariant & anti-hallucination pass rate).
  - **Tier 1 Local SLM (`Qwen3.5-0.8B-Q4_K_M` via `llama-server`)**: `1,515.5 ms` median across the 88-case evaluation benchmark (`100.0%` invariant & anti-hallucination pass rate, protected by the 6-point anti-hallucination guard).

## Security & Privacy

- **Microphone Lifecycle**: The microphone stream opens only when recording begins and is dropped and zeroized immediately upon release, cancellation (`Esc`), or the 120-second safety cap.
- **No Silent Cloud Transmission**: `Maximum Privacy (100% Offline)` is the default profile. Local `llama-server` binds strictly to `127.0.0.1` on an ephemeral port. Cloud endpoints are only contacted if the user explicitly switches profiles and saves an API key (encrypted at rest via Windows DPAPI `CryptProtectData`).
- **Cryptographic Model Verification**: Every local ASR model is verified against compile-time file size bounds and SHA-256 digests (`crates/vi-asr/src/catalog.rs`) before `sherpa-onnx` parses a single byte. Corrupted files are quarantined automatically.
- **Process Hardening**: Enables `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR)` and `HeapEnableTerminationOnCorruption`.

## Known Limitations

- The Windows executable, bundled runtimes, and NSIS installer are unsigned (`docs/release/05-code-signing.md`); Windows SmartScreen may prompt on first run.
- UIPI prevents a non-elevated `keyreed.exe` from synthesizing `SendInput` keystrokes into an elevated (`Run as Administrator`) window; KeyReed detects elevation mismatch automatically, preserves the text on the clipboard, and notifies the user to press `Ctrl+V`.
- Automated E2E verification uses controlled speech WAV fixtures and real Win32 hardware microphone capture; live human speech across diverse room acoustics and Hindi WER are documented as open field benchmarks.

## License

Dual-licensed under MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
