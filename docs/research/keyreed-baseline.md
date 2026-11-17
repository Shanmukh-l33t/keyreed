# KeyReed baseline (v2.0.0-beta.1, commit c6c5792)

Written before any change was made in this round. It records how the app works today, read from source. Nothing here was run: this round was built on a Linux host and the owner tests on Windows. Evidence tags: [O] observed in code at the cited place, [I] inferred from code, [U] not verified.

## 1. Shape of the code

A Rust workspace of six crates, Windows only, about 30k lines.

| Crate | Job |
| --- | --- |
| `vi-core` | Config, push-to-talk state machine (`ptt.rs`), shortcut model, dictionary, history, writing styles, shared types |
| `vi-audio` | Microphone capture through cpal (WASAPI), mono mix, resample to 16 kHz |
| `vi-asr` | sherpa-onnx through `libloading` (offline recognizer plus Silero VAD), model catalog, SHA-256 pins, downloads, the live chunk worker |
| `vi-rewrite` | Tier 0 deterministic cleanup, paragraphs, optional local grammar model through llama-server, guards, Quick Fix tagger |
| `vi-win32` | Everything that touches Windows: low-level keyboard hook, SendInput typing, clipboard, UI Automation, foreground context, the software-rendered listener pill, the GDI settings window |
| `vi-app` | The daemon: one coordinator thread that owns hotkey events, UI actions, a 75 ms ticker and the whole dictation pipeline |

The UI is hand drawn: the pill (`overlay.rs`) is a premultiplied BGRA software renderer into a `WS_EX_LAYERED | WS_EX_NOACTIVATE` window, and Settings (`settings.rs`, 3.1k lines) is a custom-painted window with its own widget set. There is no UI framework. [O]

## 2. The pipeline as it runs

```
Right Ctrl down
  vi-win32/hotkey.rs   WH_KEYBOARD_LL on its own thread; only modifiers, cancel and trigger keys are inspected
  vi-core/ptt.rs       Idle -> Recording after min hold 110 ms; cap 30 min
  vi-audio             cpal stream opened at key-down, closed at key-up (no pre-roll, no tail)
  main.rs tick (75 ms) drain 16 kHz samples -> LiveAsr worker thread
  vi-asr/lib.rs        Silero VAD (512-sample windows) -> speech spans -> groups (merge gap 550 ms, cap 24 s)
                       a group is decoded offline (greedy_search) as soon as no later speech can merge into it
Key up
  last group decoded, sentences with pause times returned
  vi-rewrite           Tier 0 -> paragraphs -> optional Qwen3.5 0.8B grammar pass -> guards (fallback to Tier 0)
  vi-win32/typing.rs   SendInput Unicode in 64-input chunks, clipboard fallback for password, elevated, shell targets
```

Facts worth knowing, with locations:

- Streaming is VAD-driven chunking, not model streaming. A group is decoded while the user is still talking, so after release only the final group remains. `ready_groups`, `PAD_PRE` 300 ms, `PAD_POST` 250 ms (`vi-asr/src/lib.rs:143-216`). [O]
- The recognizer is created with `decoding_method = "greedy_search"` (`lib.rs:438`). Personal vocabulary is passed as uppercased per-stream hotwords (`vi-core/src/dictionary.rs:75`). In sherpa-onnx the NeMo transducer applies hotwords only inside the `modified_beam_search` branch, so this path is inert [I, strong; confirmed against upstream source, not run].
- Before decoding, the whole clip's mean RMS is compared with 0.0012 (about -58 dBFS) and the clip is dropped below that (`lib.rs:623-638`). A quiet laptop microphone with long pauses can fall under it [I].
- The microphone opens at key-down and closes at key-up. Opening is cold, there is no pre-roll, and the audio is cut at the instant of release (`main.rs` `stop_and_take_16k`). [O]
- Resampling is a 3-tap box average for 48 kHz and plain linear interpolation for everything else, with no anti-alias filter (`vi-audio/src/lib.rs:474-506`). Incremental draining is block aligned and tested. [O]
- Level meter: one instantaneous RMS per capture callback is stored in an atomic, read about every 66 to 75 ms, no smoothing, fixed -55 to -15 dBFS window (`vi-audio/src/lib.rs:440`, `overlay.rs:249`). [O]
- Direct typing writes a clipboard backup copy of the text before typing; `exclude_from_clipboard_history` defaults to false, so the backup lands in Win+V history (`typing.rs:337`, `config.rs:24`). [O]
- A key-up that never arrives is recovered after 3 ticks in Hold mode, but a trigger key's auto-repeat and key-up leak to the focused app, and modifier state is event-only (`hotkey.rs:99-153`). [O]
- The 75 ms ticker drains the capture buffer into `feed_live_audio`, which discards the chunk when there is no live ASR session (cloud profile, or a failed live start) (`main.rs:2427`, `:503`). [O]
- The 30 minute cap discards the recording instead of processing it (`ptt.rs:237`). [O]
- Logging goes to stderr of a GUI-subsystem process and the release profile aborts on panic with no hook, so field failures leave nothing behind (`main.rs:3`, `Cargo.toml`). [O]
- `vi-app/build.rs` embeds the manifest and icon only for the GNU target; an MSVC build would ship without DPI and common-controls manifest. [O]

## 3. Answers to the baseline questions

1. Implemented correctly and kept: low-level hook with a privacy filter, hold and toggle PTT, Escape cancel, SendInput Unicode typing with TOCTOU checks, UIA service with bounded workers, clipboard snapshot code, SHA-256 and size pins before every model load, download safety, zeroized audio, per-monitor DPI, reduced motion and high contrast, zero idle timers in the pill, Tier 0 with guards and an Exact path, history with raw and final text kept apart.
2. Where latency comes from. After release: the last VAD group decode (Parakeet v2 int8, measured real-time factor 0.04 in KeyReed's own benchmark, `research/07-asr-benchmark.md`), Tier 0 (regex), the optional grammar model, then typing. Before release: cold microphone open. Model load is already background and warmed up. [O for structure, figures are KeyReed's own earlier measurements]
3. Accuracy: three outside the model. The inert hotwords, the level gate that can discard quiet speech, and the resampler aliasing. [I, none measured]
4. Text processing that can damage accurate transcription: the filler regex eats a following full stop, the stutter rule deletes legitimate repeats ("had had", "bye bye"), phrase rewrites change wording in Smart mode, and the tech casing table recases ordinary words such as "march" (`tier0.rs:21-52, 191-300`). [I from reading regexes]
5. Redundant or heavy: `main.rs` is 2.9k lines and the coordinator thread runs the pipeline synchronously, so a second hotkey event waits (kept, see the plan). `apply_spoken_symbols` recompiles regexes on every call.
6. Model versus application: punctuation and capitalization come from Parakeet itself; chunk boundaries, hotword biasing, level gating and audio edges are application concerns.
7. Compatible with offline and Windows: everything in the plan keeps CPU-only, local-first operation.

## 4. Test and benchmark assets already in the repo

`bench/` holds a 244-clip accuracy corpus runner, correction eval sets and a TTS corpus generator; `research/07-asr-benchmark.md` has the model comparison (Parakeet v2 INT8 EN WER 11.50 aggregate on that mixed hard corpus, 2.05 clean). They were read to understand the architecture and not executed in this round.
