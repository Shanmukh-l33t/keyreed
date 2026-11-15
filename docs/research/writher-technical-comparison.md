# WritHer technical comparison

Inspected snapshot: https://github.com/benmaster82/writher at `e1e343706f6b36a4d8f74260f6633fc1241da5fe` (2026-10-27). Shallow clone. Source was read. The README was used only as a map.

No KeyReed code was changed for this audit.

## 1. Executive assessment

WritHer is a working Windows tray dictation app. Its best ideas are about failure recovery: a lost key-up must not leave the microphone open, a shared recorder must not leak audio across sessions, and a paste must not destroy a screenshot that was already on the clipboard.

Those ideas are worth adapting. The implementation is not worth copying.

WritHer is Python, pynput, sounddevice, and faster-whisper. It downloads a Whisper model from Hugging Face and treats a cache directory as trust. It always writes dictated text to a recovery file, then synthesizes Ctrl+V. KeyReed is a Rust tray app with a low-level keyboard hook, WASAPI capture, pinned local models, and a grammar-correction pipeline whose default output is the clipboard, not an injected paste.

Adopting WritHer's stack would drop the accuracy work already measured (Parakeet v2, Indian English WER 10.14 on the stored bench, 0 percent silence hallucination) and would weaken the model-integrity pins. Do not do that.

The useful transfer is three behaviors: discard a hold-mode overrun instead of pasting it, notify on a second instance, and, only if auto-paste is ever enabled, snapshot every clipboard format and restore it only when the sequence number still matches.

## 2. Architecture comparison

| | WritHer | KeyReed |
| --- | --- | --- |
| Language | Python 3.11 | Rust workspace |
| Hotkey | pynput `keyboard.Listener` | `WH_KEYBOARD_LL` on its own thread |
| Audio | sounddevice, WASAPI preferred | cpal WASAPI |
| ASR | faster-whisper, Hugging Face cache | sherpa-onnx, six pinned models |
| Rewrite | regex vocabulary and optional symbol mode | Tier 0 rules, local SLM, optional cloud, guards |
| Output | clipboard write, then Ctrl+V, then restore | clipboard by default. User pastes. Direct typing is optional |
| Packaging | PyInstaller spec, UPX on, no signing identity | `scripts/package-win.sh`, NSIS stub, sign gate that fails closed |
| License of the app | MIT, copyright 2026 Writher Contributors | MIT OR Apache-2.0 in the crate metadata |

WritHer also has an assistant mode (notes, reminders, voice delete) that KeyReed does not have and should not grow in order to match it.

## 3. Module comparison

### Global keyboard handling

WritHer `hotkey.py`. Hold mode fires press once and release once. Toggle mode ignores the release and flips on each press, with a 300 ms debounce so key-repeat does not stop the recording. Modifier state is a set of canonical names. A bare modifier can be the hotkey (default dictation key is AltGr). After an Alt press it injects a dummy VK `0xE8` so Windows does not open the menu bar. If a release is lost, `reset_dictation_state` clears the stuck flag without firing a callback. Combo hotkeys require an exact modifier set. A single-key hotkey matches even if other modifiers are held.

KeyReed `hotkey.rs` and `ptt.rs`. The hook runs above normal priority so a slow UI thread cannot trip `LowLevelHooksTimeout`. It inspects only modifiers, Escape, and the configured trigger. Synthetic keys tagged `KRED` are ignored. Right Ctrl is the default hold key. Repeat while the key is already down is ignored. Toggle is a separate activation mode. Minimum hold is 110 ms. Escape cancels. The hook suppresses a non-modifier trigger so it does not leak into the focused app. A pure modifier trigger is passed through.

WritHer is better at naming the lost-release failure and resetting the latch without processing audio. KeyReed already clears `shortcut_physically_down` when the 120 s cap fires, so the next press is not swallowed. The difference is what happens to the audio. KeyReed `on_tick` returns `StopAndProcess`. WritHer hold-mode timeout discards the buffer and does not paste. For a lost key-up, discard is the safer behavior. For a user who held the key on purpose for two minutes, discard throws away real speech. Adopt the discard only when the cap fired while the key was still logically down, and say so in the overlay.

WritHer's 300 ms toggle debounce is not automatically better. KeyReed already ignores OS auto-repeat via the physically-down flag. Copying 300 ms would make a deliberate double-tap fail.

WritHer's "single key matches even with extra modifiers" rule is a conflict source. KeyReed's exact match is stricter and better.

Both inject VK `0xE8` to mask Alt or Win. KeyReed already does this when the spec includes Alt or Win. No change required there.

### Microphone handling

WritHer `recorder.py`. On each start it re-inits PortAudio through the private `sd._terminate` and `sd._initialize`, then prefers a WASAPI device by exact name, then substring, then any host API. It tries 16 kHz first and falls back to the device rate with linear interpolation. A failure sets `recording` false and calls `on_mic_error`. There is no hot-plug callback. The next start re-queries, which is the hot-plug support. The shared recorder has one owner and a generation counter (`main.py`). A stale stop is ignored. A new press discards the previous buffer.

KeyReed `vi-audio`. `list_input_devices` enumerates cpal inputs. `ActiveCapture::start` resolves a preferred name or the default, opens WASAPI, and resamples with an anti-aliased boxcar filter. Drop or `stop_and_take_16k` closes the stream and zeroizes the raw buffer. A start error moves the engine to an error state and does not leave `recording` true. There is one capture path, not a dictation-plus-assistant share, so the generation lock is less urgent. There is no PortAudio re-init.

WritHer's re-query on start is a good idea and KeyReed already resolves the device at start. Do not copy `sd._terminate`. That private call can disrupt other audio clients. KeyReed's resampler is the better one. Linear interpolation is a weakness, not a model to follow.

Device-change notifications while idle are absent in both. A settings refresh that calls `list_input_devices` again is enough. Do not add a background WASAPI watcher unless a test shows the list goes stale.

### Clipboard handling

WritHer `injector.py` is the strongest module in the repo. It does not use pyperclip. It opens the clipboard with retries, duplicates every format with `OleDuplicateData` (enhanced metafiles via `CopyEnhMetaFile`), refuses `CF_OWNERDISPLAY` rather than pretending it was saved, swaps in `CF_UNICODETEXT`, records `GetClipboardSequenceNumber` after `CloseClipboard`, synthesizes Ctrl+V, waits `CLIPBOARD_RESTORE_DELAY` (default 0.5 s), and restores only if the sequence number and the text still match. If the user copied something else during the wait, the new copy wins. If the full snapshot fails, it degrades to text-only save and restore. Every transcription is also appended to `recovery_notes.txt`, rotated at 500 KB.

KeyReed `clipboard.rs` writes `CF_UNICODETEXT` only. It allocates the buffer before `EmptyClipboard`, so a memory failure does not wipe the user clipboard. It retries `OpenClipboard`. It refuses an empty write. It can set the official exclude-from-history formats. It does not snapshot images or files. Default mode does not send Ctrl+V. Direct typing uses Unicode `SendInput` and falls back to the clipboard. A failed recording on this guest left a sentinel string in place. That was measured. Paste into Notepad was not confirmed.

WritHer is better if the product injects the paste. It is worse as a default for KeyReed. Restoring the clipboard 0.5 s after the write would remove the text the user is about to paste with Ctrl+V. That fights the product. The snapshot and sequence check should be adopted only behind an explicit auto-paste option. The default must keep the corrected text on the clipboard.

`recovery_notes.txt` always on disk is a privacy regression. KeyReed history is off by default, and audio is zeroized. Do not adopt an always-on transcript file.

WritHer does not verify the paste landed in the target. Neither does KeyReed, except a harness that tried Notepad and read back empty text. Neither project has a compatibility matrix that passed.

### Speech recognition

WritHer loads faster-whisper with `beam_size=5` and `vad_filter=True`. Vocabulary terms become `initial_prompt`. Symbol mode is a regex layer, off by default, that turns "forward slash" into `/` and glues spelled letters. It does not rewrite grammar. Model trust is "the Hugging Face snapshot has `model.bin` and no `.incomplete` blob". There is no SHA-256 pin.

KeyReed uses sherpa-onnx. Six models have compile-time file pins. Load hashes those files before ONNX parse. A directory that merely exists is not trusted. Hotwords already come from the personal dictionary. Tier 0 already reassembles some paths and flags. The SLM rewrite is guarded. Exact mode skips creative rewrite. Terminal and code modes refuse to "improve" commands.

WritHer's priming idea is already present as hotwords. The symbol layer is a useful opt-in under Terminal or Code mode, not a replacement for Tier 0. Do not switch the default ASR to Whisper. The stored bench showed Whisper-family hallucination rates up to 50 percent. Parakeet v2 was 0 percent on the non-speech clips.

WritHer's incomplete-blob check is the weak version of a problem KeyReed already solved more strictly. Keep the pins.

### Application lifecycle

WritHer uses a `Local\WritHerSingleInstance` mutex and toasts "already running" instead of exiting silently. Quit has a watchdog so a ghost process cannot hold the mutex. Data lives under `%APPDATA%\WritHer` when packaged. PyInstaller collects the app, enables UPX, and sets `codesign_identity=None`. First launch can sit on a Hugging Face download. The widget shows that state, which is better UX than a silent hang. The download itself is not hash-pinned.

KeyReed uses `Local\Keyreed_SingleInstance_v1`. A second process exits. The console-session test showed the second process gone and the first still running. There was no toast. Startup is opt-in, quoted, and applied from the saved preference. The installer is a 31 MB stub. Models are not embedded. The sign gate fails closed. Quit behavior was not compared on a stuck UI thread.

The toast is worth adding. UPX and an unsigned PyInstaller layout are not. KeyReed's installer and pin path stay.

## 4. Ideas worth adopting

1. Hold-mode cap discards audio and resets the latch, with an overlay line that says the recording was dropped because the key release was lost. Benefit: a stuck hook cannot paste two minutes of room noise into the focused window. Test: simulate a down without an up, advance the clock past `max_record_ms`, assert no clipboard write and that the next down starts a new recording.

2. Optional auto-paste with full-format snapshot and sequence-number restore. Default stays "leave the text on the clipboard". Benefit: a user who opts in does not lose a copied image. Test: put a bitmap and a file list on the clipboard, auto-paste a short string, assert the bitmap returns if nothing else changed, and assert a user copy during the wait is kept.

3. Second-instance toast. Benefit: the silent exit stops looking like a crash. Test: start two processes in one session, assert one remains and a notification was requested.

4. Opt-in spoken-symbol substitutions inside Terminal and Code modes, after Tier 0, never instead of it. Benefit: "forward slash" can become `/` when the user asked for that, without turning prose "dash" into a hyphen in Smart mode. Test: the existing correction set, plus a small symbol-mode set, must keep commands intact and must not fire in Smart mode.

Do not adopt these as a bundle. Each one needs its own test before it lands.

## 5. Problems in WritHer

- Model trust is a cache directory, not a hash. An attacker who can write the Hugging Face cache can feed the recognizer. KeyReed must not copy this.
- `recovery_notes.txt` stores every dictated string. That is a transcript log with a 500 KB rotation, not a privacy control.
- Default path injects Ctrl+V into whatever has focus. A focus change during the 0.5 s wait pastes into the wrong window. KeyReed's default avoids that.
- `sd._terminate()` is a private API.
- Resampling is linear interpolation.
- Single-key hotkeys ignore extra modifiers, which widens conflicts.
- pynput is a global listener in the Python process. KeyReed's dedicated hook thread is the more careful design for `LowLevelHooksTimeout`.
- The PyInstaller spec turns UPX on and names no signing identity. UPX increases SmartScreen and antivirus friction.
- The assistant can send notes to a local LLM. That is a separate product. It is not a reason to add a notes database to KeyReed.

## 6. Differentiators to keep

Do not remove or weaken:

- Grammar correction with guards, and the Tier 0 floor when the SLM fails.
- Writing modes: Smart, Exact, Professional, Casual, Technical, Terminal and Code.
- Context from the foreground process, with a fallback when the control is not exposed.
- Processing profiles, including MaxPrivacyOffline refusing the network.
- Compile-time SHA-256 pins, staging install, and quarantine of a bad model directory.
- History off by default, audio zeroized, password fields blocked.
- Clipboard as the default output. The user pastes.

WritHer's vocabulary and symbol layer can sit beside those. They must not replace them.

## 7. Licensing

WritHer's `LICENSE` is the MIT license. Copyright line: "Copyright (c) 2026 Writher Contributors". The notice and permission text must be included in any copy or substantial portion of that software.

This audit copies no source. Describing behavior is not a copy.

`requirements.txt` names faster-whisper, sounddevice, numpy, pynput, pystray, Pillow, requests, winotify, and customtkinter. Those packages are not vendored in the clone. Their licenses were not re-read from installed wheels in this pass. Before any file is copied, read that file's license and the license of every import it needs. Do not link an LGPL listener into the KeyReed binary just to reuse a hotkey helper. Reimplement the behavior in the existing Rust hook if a test says the behavior is worth having.

## 8. Prioritized recommendations

1. Change the hold-mode safety cap so a lost release discards audio and resets the latch. Do not paste it. Highest reliability gain. Small change in `ptt.rs` and the coordinator. No new dependency.

2. Add a second-instance notification. Small UX fix. No new dependency.

3. Design an opt-in auto-paste that snapshots clipboard formats and checks `GetClipboardSequenceNumber` before restore. Leave the default alone. This is the only recommendation that is a real feature, and it should wait until paste-into-Notepad is confirmed on a machine where the test can read the text back.

4. Add an opt-in symbol layer for Terminal and Code only, using the existing dictionary and Tier 0. Do not import `replacements.py`.

5. Do not change the ASR default, the pin table, the installer layout, or the privacy defaults because WritHer does something else.

## 9. Proposed tests

- Lost key-up: down without up, cross the cap, clipboard unchanged, next down starts clean.
- Stale stop: a late release after cancel does not process a new buffer.
- Toggle repeat: key-repeat within 110 ms does not stop a toggle recording. Do not add a 300 ms debounce unless this test fails.
- Clipboard sentinel: a mic-open failure leaves the previous clipboard text. Already observed once on the guest. Repeat on a machine with a microphone.
- Auto-paste, if built: image clipboard survives, and a user copy during the restore delay wins.
- Symbol mode off: the 88-case correction set does not change. Symbol mode on: a slash phrase becomes a slash, and a prose "dash" in Smart mode does not.
- Second instance: one process remains, and the user is told.

## What this comparison does not claim

WritHer was not built or run on the KeyReed test guest. The comparison is from source at the commit above, not from a side-by-side latency measurement. No number here is a new benchmark.
