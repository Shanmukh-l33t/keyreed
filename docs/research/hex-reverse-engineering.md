# Hex reverse engineering report

Target: Hex 3.4.0 (`anomalyco/hex`, source at HEAD 94cbe24, release `v3.4.0` published 2026-11-11). Purpose: learn what to adapt for KeyReed, not to copy it.

Labels: **[REA]** observed in the shipped binary with REA; **[SRC]** observed in source with a file and line; **[DOC]** stated in Hex's own docs (the authors' measurement, not reproduced here); **[INF]** inference from the above; **[UNV]** not verified. Nothing was run: no Hex binary was executed, nothing was benchmarked, and Hex says its own Rune Windows app "has never run on Windows" in `docs/windows.md:284`, so treat it as a reference design, not proof of behaviour.

## 1. Method and REA use

Hex is a large Rust program (about 400k lines), so source analysis was the main channel. REA was used for the shipped Windows artifact, to check what the source claims against what actually ships. Official `rea-agents` 6.9.0 needs Node 22 or newer and this host has Node 20.19.5, so the REA CLI could not start; the REA MCP server already configured in this environment was used instead. Artifact: `hex-3.4.0-windows-x64.exe`, 87,241,648 bytes, SHA-256 `90bf9543dee8c3c799de95cbb4ccd1c05bcb29863c7212c1bd641979e287af9e`, downloaded from the official GitHub release.

| REA operation | Evidence id (prefix) | What it established |
| --- | --- | --- |
| `inspect_pe_imports` (raised `max_file_bytes` to 128 MiB) | `ev_c4cd8470` | 35 import descriptors plus delay-loaded `vulkan-1.dll`; the Win32 surface in section 2 |
| `inspect_binary_hardening` | `ev_3975d0ee` | ASLR, high-entropy VA, NX on; CFG instrumented flag set but `guard_cf` bit clear; Authenticode table present |
| `inspect_pe_resources` | `ev_9ae0166f` | One icon group of eight images (256, 64, 48, 40, 32, 24, 20, 16 px) and a version resource; no manifest resource in the exe (consistent with `hex/build.rs` using `manifest_optional`) |
| `inspect_pe_certificates` | `ev_4905059f` | PKCS#7 signature by Anomaly Innovations, Inc (Delaware), chain Microsoft ID Verified CS AOC CA 04 under Microsoft Identity Verification Root CA 2020; leaf valid 2026-11-09 to 2026-11-12 (a short-lived Azure Artifact Signing certificate) |
| `inspect_rust_binary` (summary) | `ev_c2d91db3` | No cargo-auditable `.dep-v0` section, so the dependency list could not be read from the binary; rustc commit is inferred only |

Limits: REA reads headers and tables, it does not decompile the 87 MB binary here and nothing was executed, so no runtime behaviour is claimed. Signature validity was not verified, only structure. The dependency graph comes from `Cargo.toml` and `Cargo.lock` instead.

## 2. What the binary confirms about the Windows layer [REA, with the matching source]

| Import (user32 unless noted) | Source evidence | Meaning |
| --- | --- | --- |
| `SetWindowsHookExW`, `CallNextHookEx`, `UnhookWindowsHookEx`, `GetAsyncKeyState`, `GetKeyState` | `src/windows/input.rs:613-735` | Low-level hooks on a dedicated thread, modifiers re-read from `GetAsyncKeyState` each event |
| `SendInput`, `MapVirtualKeyW/ExW`, `VkKeyScanW`, `GetKeyboardLayout`, `ToUnicodeEx` | `src/windows/paste.rs:323-358` | Ctrl+V with layout-correct scan codes; `ToUnicodeEx` is probably the shortcut recorder |
| `OpenClipboard`, `SetClipboardData`, `GetClipboardData`, `GetClipboardSequenceNumber`, `RegisterClipboardFormatW` | `paste.rs:147-235` | Clipboard paste with sequence-number race check and history-exclusion formats |
| `ActivateAudioInterfaceAsync` (mmdevapi) | `src/audio/wasapi_capture.rs:1-98` | Direct WASAPI fallback when cpal cannot open the device |
| `mfplat`, `mfreadwrite` | file decode for imports | Media Foundation, used for file transcription, not live dictation [INF] |
| `uiautomationcore` provider functions | accessibility tree for the custom UI | Hex exposes its own UI to screen readers |
| `RegisterRawInputDevices`, `GetRawInputData`, `RegisterHotKey` | source says the shortcut path uses neither (`input.rs`) | Present in the binary but not from Hex's shortcut code; most likely the windowing layer. Flagged as an unexplained import [UNV] |
| `d3d11`, `dxgi`, delay-loaded `vulkan-1.dll`, `DwmSetWindowAttribute` | `src/windows/vulkan.rs:43-54`, `rune.rs:760-850` | GPU UI and optional GPU inference; the app runs without Vulkan |
| `PowerCreateRequest/SetRequest`, Job objects, `SetThreadPriority` | n/a | Keep-awake while working, child process control |
| `winhttp`, `crypt32`, `secur32` | updater, downloads | Proxy discovery and TLS |

There is no `winmm` import, which matches cpal on WASAPI. No `KEYEVENTF_UNICODE` appears anywhere in Hex source, so Hex inserts text only by clipboard paste on Windows [SRC `paste.rs`].

## 3. Architecture [SRC]

- One crate with a Rune GPU UI (`src/rune_ui*`, `src/rune_hud*`), custom to Anomaly, pinned from `anomalyco/rune`. Engine and UI are separate: a dictation worker, an engine thread for transcription (`src/transcription/engine.rs`), a hook thread, a tray thread, and an authenticated local API with a TypeScript SDK.
- Speech runs through `transcribe.cpp` (GGUF, ggml, Vulkan or CPU), pinned to a fork with Parakeet vocabulary rescoring. ONNX exists only behind a benchmark feature (`Cargo.toml:31`).
- State is per-feature: an `Indicator` state machine drives the HUD (`src/dictation/indicator.rs`), `CaptureState` drives hold, tap, toggle and double-tap lock (`input.rs:295-400`), settings persist as JSON. Features that are not certain ship as off-by-default Labs (`AGENTS.md`).

## 4. What matters for speech quality [SRC unless noted]

Full detail and numbers are in `stt-analysis.md`. Short version:

1. **Tail and pre-roll protect the edges.** A 450 ms pre-roll ring and a 200 ms post-release tail (growing to 600 ms while speech continues) (`src/dictation.rs:54-130, 1395-1440`). Hex's own replay study [DOC] kept the last word 10 of 10 at 0 to +200 ms but 7 of 10 at -100 ms and 2 of 10 at -200 ms.
2. **A level gate that respects quiet microphones.** Loudest 30 ms DC-removed window at -50 dBFS, or a sustained run 12 dB over the clip's own floor (`dictation.rs:519-650`); then Silero VAD to avoid hallucinated "thank you" on silence.
3. **A real resampler.** `rubato` FFT, Blackman-Harris 2, delay compensated and truncated to exact length (`dictation.rs:675-840`).
4. **Vocabulary has a decode-time path.** Parakeet terms are rescored on the same audio against the greedy hypothesis with a boost per piece, capped at 16 candidate spans and 64 terms; Whisper and Qwen take a text prompt; a separate exact "sounds like" replacement pass runs after (`docs/features/vocabulary.md`, `src/dictation/vocabulary.rs`). Hex measured a proper noun right 12 of 19 times versus 4 [DOC].
5. **Streaming is conditional.** Parakeet Unified streams only on a GPU, because a CPU stream "re-encodes each overlapping window and would cost several times the offline compute" (`gguf_session.rs:244-247`). Every stream result is checked and the whole clip is decoded again if the stream lost its opening, dropped a phrase, or ended without closing punctuation (`streaming.rs:158-409`, commit 94cbe24).
6. **Processing is opt-in.** Spoken formatting, spoken symbols and filler removal are Labs, off by default; a separate "dictate without processing" chord pastes the raw transcript (`src/dictation/processor.rs:1195-1226`).

## 5. What matters for the interface [SRC, theme read from the pinned Rune revision]

- Neutral dark and light palettes, 13 px UI type, 8 px cards, hairline dividers inset 16, 28 px nav rows, a 220 px sidebar and an inset content panel.
- Hover is instant by rule; only controls react, rows do not.
- The dictation HUD smooths measured loudness: attack 35 ms, decay 95 ms, an auto-level that follows the speaker (peak attack 40 ms, release 3 s, room floor tracking), bar height as the square root of level, wall-clock time step clamped to 1/240..1/20 s, drawn about 60 times per second and only while visible (`scope.rs:112-127`, `dot_scope.rs:400-470`).
- Working, done and failed have separate hues. Processing stays visible at least 200 ms so it does not flash.
- First run starts the model download when the tour opens, auto-advances 700 ms after ready, has a practice step with specific hints, a persistent "your voice never leaves this computer" line, and on Windows a direct link to the microphone privacy setting (`desktop/onboarding.rs:1075-1100`).
- Vocabulary is one sheet of Term and Sounds-like rows with limits shown inline.

## 6. Windows integration notes [SRC]

Hook re-arm on every foreground change because some apps' raw-input hooks silence low-level hooks (`input/rearm.rs`, issue 1901); trigger repeats and key-up swallowed; modifier-only chords with exact side matching and an Alt/Win release mask; clipboard paste with snapshot, sequence-number race check and generation-safe restore (`paste.rs:48-303`); remote-desktop and Java special cases; bounded microphone recovery with a 4 s stall watchdog; per-user Inno Setup installer, signed exe, signed installer, self-contained runtimes, ed25519 and Authenticode-pinned updater.

## 7. Not copied, and why

- The Rune GPU UI, the GGUF runtime and Vulkan stack: a different technology base; KeyReed stays a CPU, ONNX, GDI app.
- Parakeet Unified streaming: the GPU-only rule above.
- Always-open microphone: Hex keeps it open for pre-roll; KeyReed promises the microphone is open only while the shortcut is held.
- Clipboard-only insertion: KeyReed's Unicode `SendInput` path with clipboard fallback is safer for the user's clipboard.
- Hex has no password-field or elevation handling on Windows; KeyReed already has both.

## 8. Unverified

How the native chunked decoder inside `transcribe.cpp` overlaps windows (not in the tree). Whether the Win32 hook, paste and HUD paths behave as written on a real Windows machine (Hex says the HUD was never run on Windows, `docs/windows.md:333`). Anything about Hex's runtime performance on Windows. The source of the raw-input and `RegisterHotKey` imports. Narrator behaviour of the HUD.
