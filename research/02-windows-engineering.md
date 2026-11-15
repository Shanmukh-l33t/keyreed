# 02: Windows 10/11 Implementation Engineering for Keyreed

Status: engineering reference. Written 2026-11-15.
Every recommendation below is either implemented in `crates/vi-win32/` and validated on a real Windows 11 VM, or is a documented pitfall we hit and worked around. API citations are Microsoft Learn URLs verified reachable 2026-11-15.

## Target matrix

- Windows 10 22H2 and Windows 11 (22H2 → 24H2+), x64 and ARM64.
- Rust, `windows-sys` 0.59 bindings (not the full `windows` crate, smaller surface, fewer surprise features). Release profile: `opt-level=3`, `lto=thin`, `panic=abort`, stripped.
- The app is a tray/overlay background process with no console. It must survive Secure Desktop-adjacent conditions (UAC prompts), RDP sessions, and sleep/resume.

## 1. Global push-to-talk hotkey detection

Decision: `WH_KEYBOARD_LL` low-level keyboard hook, not `RegisterHotKey`.

Why: `RegisterHotKey` (winuser) is the "correct" API for a fixed chord, but it cannot express (a) hold-to-talk semantics with a modifier-only chord, (b) dynamic install/uninstall when the user toggles PTT, or (c) suppression of the Start-Menu pop that happens when a Win/Alt chord is released. A dictation app needs all three. `RegisterHotKey` is used only as a secondary path for the mode-cycle shortcut where fire-and-forget is fine.

Implementation rules that matter (all in `hotkey.rs`):

- The hook runs on a dedicated message-pump thread at `THREAD_PRIORITY_ABOVE_NORMAL`. `LowLevelKeyboardProc` has a system-imposed budget (`LowLevelHooksTimeout`, ~300 ms default); if the callback blocks longer than that, Windows silently drops the hook. So the callback must never touch audio, models, or IPC that can block, it only updates a `ModifierState` and pushes an event onto a `crossbeam_channel`. This is the single most common cause of "PTT randomly stops working" in competitor apps.
- Inspect ONLY modifier VKs, `VK_ESCAPE`, and the configured shortcut VK. Zero keystroke buffering, zero logging. This is both a privacy guarantee and a perf guarantee (see `06-security-privacy.md`; MITRE T1056.001 is the technique we must not resemble).
- Inject an unassigned sentinel VK (`VK_MENU_MASK = 0xE8`) on Win/Alt chord trigger so that releasing Win or Alt does not open the Start Menu or an app's menu bar. Without this, every "Ctrl+Win to dictate" leaves a Start Menu open over the user's text.
- Track left/right modifiers separately (`lctrl`/`rctrl`, etc.) so "right Ctrl only" bindings work and so a stuck-key from a dropped key-up can be detected and reset.
- Install/uninstall the hook (`SetWindowsHookExW` / `UnhookWindowsHookEx`) when PTT is toggled, rather than leaving it hooked and ignoring events, an idle hook still costs a syscall per keystroke system-wide.

Pitfalls we hit:
- Synthetic injection detection. `SendInput`-generated events (our own paste, and other apps' automation) carry no distinguishing bit in `KBDLLHOOKSTRUCT`. We tag our own injected input with a private `dwExtraInfo` value (`KEYREED_SYNTHETIC_TAG`) and ignore hook events bearing it, so our paste never re-triggers PTT. See `security.rs`.
- Session changes. On RDP connect/disconnect and on fast-user-switch, the hook's thread can be orphaned. Handle `WTS_SESSION_CHANGE` (termserv) and `PBT_APMRESUMEAFTERSUSPEND` (power) to re-establish the hook.

## 2. Audio capture: WASAPI via `cpal`

Decision: `cpal` 0.15 (shared-mode WASAPI) rather than raw `IAudioClient`.

- Shared mode is correct for a dictation app: it does not grab an exclusive device handle, so it coexists with Teams/Zoom/Discord already holding the device. Exclusive mode buys ~10 ms latency we do not need and breaks every conferencing scenario.
- We open the default render/comm device with `IAudioClient::Initialize` + `InitializeSharedAudioStream` (audclnt), pull frames in a capture callback, resample to 16 kHz mono float, and hand PCM to the ASR pipeline. `cpal` wraps all of this; we only touch raw WASAPI for device-role change notifications.
- Device hot-plug: register an `IMMNotificationClient` (mmdeviceapi) so that plugging in a headset or switching from laptop mic to USB mic re-binds the capture stream mid-session instead of capturing silence. Recovering from `AUDCLNT_E_DEVICE_INVALIDATED` is the documented pattern (see "recovering from an invalid device error").
- Mic privacy: Windows "let desktop apps access the microphone" can be off. We detect the denial (capture returns nothing / device unavailable) and surface a actionable prompt that deep-links to the right Settings page (manage-desktop-app-privacy-access), rather than failing silently.

Pitfall: `cpal`'s default stream is event-driven but some Realtek drivers deliver irregular callback sizes; we buffer to a fixed 16 kHz ring and never assume a callback equals a fixed frame count.

## 3. Voice activity + utterance boundaries

PTT gives us the outer boundary (button down → button up). Inside that we still run Silero VAD (`silero_vad.onnx`, loaded through sherpa-onnx) to trim leading/trailing silence and to compute a speech-ratio. The speech ratio feeds the hallucination filter in the rewrite layer: if a held button produced <~10 % actual speech (user coughed, room was quiet), we emit nothing rather than letting the ASR invent text. This is measured: it is why our non-speech hallucination rate is 0 % on the default model even when the user taps PTT on silence.

## 4. Text delivery: two explicit modes (`Clipboard` default, `DirectTyping` with clipboard fallback)

Decision (`vi-core::OutputMode`, `main.rs`, `typing.rs`): we ship two distinct output modes rather than forcing one compromise onto every target.

- **`OutputMode::Clipboard` (default):** writes `CF_UNICODETEXT` to the clipboard only, no synthetic `Ctrl+V` is injected (`main.rs` L330-335; the automated `Ctrl+V` paste lives only in the `--win32-e2e-test` harness). Predictable everywhere, never races focus, and never triggers unintended shortcuts in an unfamiliar window; the user presses `Ctrl+V` where they want the text.
- **`OutputMode::DirectTyping` (`typing.rs::deliver_direct_or_clipboard_fallback`):** injects UTF-16 code units directly at the cursor via `SendInput` with `KEYEVENTF_UNICODE` (`wVk = 0`, `wScan = utf16_unit`, `dwExtraInfo = KEYREED_SYNTHETIC_TAG`), batched in 64-event chunks with a 2 ms breather on long texts, handling surrogate pairs (emoji, CJK) and layout-independent Unicode without touching VK-code layout tables. It ALSO writes the sanitized text to the clipboard so `Ctrl+V` remains available if a custom game or remote canvas ignores `KEYEVENTF_UNICODE`, and automatically falls back to clipboard-only if any safety gate trips: dangerous command requiring manual confirmation, password field/manager, elevated target window (UIPI), focus change mid-inference (TOCTOU), or `SendInput` returning 0 (`sendinput_blocked_by_target`).
- **Why both modes matter for enterprise Windows (RDP / Citrix / VMware Horizon):** locked-down VDI and jump-box sessions frequently disable host-to-guest clipboard redirection by group policy (`fDisableClip`), breaking pure clipboard workflows, whereas local `KEYEVENTF_UNICODE` keystrokes sent into the focused remote-viewer window are forwarded over the input channel cleanly. Keeping `Clipboard` as the zero-surprise default and `DirectTyping` one toggle away covers both worlds cleanly.

`clipboard.rs` guarantees, each learned from a real failure:
- Allocate and populate the `GlobalAlloc` buffer BEFORE `EmptyClipboard`. If allocation fails, the user's existing clipboard is untouched. The naive order (empty-then-fill) wipes user data on OOM.
- Retry `OpenClipboard` with backoff (up to ~180 ms) because RDP, Office, and clipboard managers hold transient locks. A single `OpenClipboard` that fails and gives up is the classic "paste sometimes does nothing" bug.
- Normalise lone `\n` → `\r\n`. Classic Win32 edit controls and Notepad collapse `\n`-only text into one line; Office and browsers are fine. Normalising is safe for all targets and preserves lists/paragraphs.
- Never clear or modify the clipboard on failure, cancellation, or silence. If dictation produces nothing, the user's copied text must still be there.
- Optional privacy clipboard formats: `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory=false`, `CanUploadToCloudClipboard=false`, these tell Windows (and clipboard history / cloud-sync) not to propagate the dictated snippet. Off by default for usability, on for MaxPrivacy.
- Paste into the foreground window (`DirectTyping` mode) requires that window to still be foreground. We capture the foreground `HWND` at PTT-down (`GetForegroundWindow`), and right before `SendInput` verify `GetForegroundWindow() == target_ctx.hwnd` (`typing.rs` L108-114). If the user Alt-Tabbed away mid-utterance, we abort direct typing, fall back to clipboard-only (`foreground_window_changed_during_processing`), and set `fallback_to_clipboard=true` in metrics.

UAC/elevation boundary: a non-elevated process cannot send input to or read the foreground window of an elevated process (e.g. Task Manager, an admin Terminal). We detect elevation via `TokenElevation` (see `context.rs`) and, when the target is elevated but we are not, we do NOT silently fail, we surface "can't type into an admin window" and offer to relaunch elevated. Shipping the app elevated-by-default is rejected: it would trip UAC on every launch and broaden our attack surface. The trade-off is documented in `06-security-privacy.md`.

## 5. Foreground app context engine (privacy-controlled)

`context.rs` classifies the foreground app into a category (General / Email / Chat / CodeEditor / Terminal / Notes / DocEditor / Browser / PasswordManager) using ONLY: process image name (`QueryFullProcessImageNameW`), window class, focused-child control style (`ES_PASSWORD` via `GetWindowLongPtr`), and UWP host remapping (`ApplicationFrameHost.exe` → the real packaged app). It never reads document contents, never screenshots, never scrapes the UI tree text. UI Automation is used only for the boolean `CurrentIsPassword` check (uiautomation), enough to refuse dictation into a password field, and nothing more.

Why this matters: the category selects the writing mode (code editor → Technical/TerminalCode, email → Smart with email formatting) which changes the rewrite prompt. But the whole engine is designed so that a security reviewer can confirm from the source that no content exfiltration path exists. Hard-coded deny-lists: password managers and password fields abort dictation outright (never type a secret through a third-party clipboard).

## 6. Overlay / status UI

Decision: a layered, topmost, click-through status pill window (`overlay.rs`, GDI, per-monitor-v2 DPI aware) rather than a full framework (no WinUI/WPF/Qt).

- `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` at startup, before any window. Multi-monitor mixed-DPI is the #1 source of "overlay in the wrong corner / blurry" bugs.
- The overlay is drawn with `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW` (not in taskbar/alt-tab, click-through, never steals focus from the dictation target). We render the BGRA premultiplied bitmap ourselves (`render_pill_bgra_premultiplied`), no D3D, no composition dependency, works on a Basic Display Driver / RDP session.
- States: Idle / Listening / Correcting / Delivered / Error, driven by an `EngineState` machine. The "Correcting" state exists specifically because local SLM rewrite takes 200-800 ms and the user needs to know the app is working, not frozen.

## 7. Process model, single instance, autostart

- Single instance via a named mutex (`Local\Keyreed...`), `SingleInstanceGuard`. A second launch signals the first and exits.
- Autostart through `HKCU\...\Run` (per-user, no elevation, quoted path). Verified round-trip on the VM.
- The inference engine runs in-process (sherpa-onnx C API via `libloading`) EXCEPT the local SLM, which is a supervised child `llama-server` on loopback (`127.0.0.1:<ephemeral>`). Rationale: llama.cpp's scheduler has crash modes we do not want taking down the whole app; a supervisor can respawn it, and idle-unload returns RAM to ~0 MB when the user stops dictating. See `04-performance.md` and `06-security-privacy.md` for the `-np 1` / `-sm none` scheduler fix.

## 8. Build / distribution engineering

- Cross-compile from Linux with `x86_64-pc-windows-msvc` + `rust-lld` (linker config in `.cargo/config.toml`). We ship a dynamically-linked exe against the sherpa-onnx/ONNX Runtime DLLs placed beside it and located via `AddDllDirectory` + `SetDefaultDllDirectories(SYSTEM32|USER_DIRS)`, never via PATH or CWD (DLL-hijacking mitigation, `06-security-privacy.md`).
- VC runtime: the MSVC CRT (`VCRUNTIME140.dll`, `MSVCP140.dll`) is a hard dependency. On a clean Win11 VM without the Visual C++ Redistributable the exe fails to start with `0xC0000135` (STATUS_DLL_NOT_FOUND). Mitigation: installer bundles `vc_redist.x64.exe` and runs it, or we ship the CRT DLLs side-by-side (the "app-local" deployment). We validated that installing the redistributable resolves the launch failure on the test VM.
- Static CRT (`crt-static`) was tried to remove the redistributable dependency; it works for the shell but the ONNX/sherpa DLLs still pull the dynamic CRT, so it does not fully de-risk first launch. Verdict: bundle the redistributable; do not rely on crt-static for the whole graph.

## Pitfall index (quick reference)

1. LowLevelHooksTimeout drops your hook if the callback blocks → keep the callback allocation-free and non-blocking.
2. Releasing a Win/Alt PTT chord opens the Start Menu → inject a sentinel modifier.
3. `OpenClipboard` fails under RDP/Office contention → retry with backoff.
4. `\n`-only text collapses in classic edit controls → normalise to `\r\n`.
5. Non-elevated app cannot paste into elevated window → detect `TokenElevation`, degrade with a message.
6. Mixed-DPI multi-monitor puts the overlay in the wrong place → per-monitor-v2 awareness before window creation.
7. First launch on clean Win11 dies with `0xC0000135` → ship the VC++ redistributable.
8. Implicit DLL load from CWD/PATH is a hijack vector → `SetDefaultDllDirectories` + `AddDllDirectory` (absolute).
9. Headset plug mid-utterance captures silence → `IMMNotificationClient` device-role rebind.
10. llama.cpp multi-slot scheduler asserts on the Windows build → `-np 1 -sm none -ngl 0` (see 04/06).

## Citations (Microsoft Learn, verified reachable 2026-11-15)

- Keyboard hooks: https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowshookexw ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-kbdllhookstruct ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
- Input injection: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-keybdinput
- Raw input (alternative to hooks): https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-rawinputdevice
- Audio: https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize ; https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream ; https://learn.microsoft.com/en-us/windows/win32/coreaudio/capturing-a-stream ; https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient ; https://learn.microsoft.com/en-us/windows/win32/coreaudio/recovering-from-an-invalid-device-error
- Clipboard: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard ; https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats ; https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-operations ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-addclipboardformatlistener
- Foreground / process context: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getforegroundwindow ; https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguithreadinfo
- UI Automation (password-field check): https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview ; https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_currentispassword
- Session/power: https://learn.microsoft.com/en-us/windows/win32/termserv/wm-wtssession-change ; https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmsuspend
- DPI / manifests: https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests
- Mic privacy: https://learn.microsoft.com/en-us/windows/client-management/manage-desktop-app-privacy-access
