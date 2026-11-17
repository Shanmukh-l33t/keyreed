# KeyReed implementation plan (Hex-informed, v2.1 candidate)

Inputs: `keyreed-baseline.md`, `hex-reverse-engineering.md`, `stt-analysis.md`. Decisions are made here; nothing waited for approval. All work was done on branch `feat/hex-informed-2.1` and verified by compiling only. The owner tests on Windows.

## 1. Comparison

| Area | Hex | KeyReed before | Chosen change |
| --- | --- | --- | --- |
| Desktop design | GPU UI, neutral palette, 8 px cards, inset panel | Hand-drawn GDI UI, emerald brand, already hairline cards, dark and light | Keep brand and structure. Upgrade the pieces with real gaps: waveform, colour semantics, recovery, onboarding, focus, vocabulary |
| Dictation interaction | Smoothed, auto-levelled HUD, 200 ms minimum processing, tick on done | Pill with raw per-callback RMS, no smoothing, fixed window, amber for both working and attention | Meter ballistics (35 ms attack, 95 ms decay, auto-level), sqrt bars, teal for working, 200 ms processing dwell, tick on every done |
| Audio pipeline | rubato FFT resampler, DC removal, 450 ms pre-roll, 200 to 600 ms tail, windowed level gate | Box and linear resamplers, no DC removal, cold mic, cut at release, whole-clip RMS gate | Windowed-sinc resampler and DC blocker, release tail, windowed gate. Warm mic and pre-roll skipped on purpose (privacy promise) |
| Streaming STT | GPU-only buffered Unified stream with fallbacks | CPU, VAD-chunked offline decode while speaking | Keep. A CPU buffered stream re-encodes overlapped windows; Hex gates it to GPU for that reason |
| Model configuration | GGUF catalog, vocabulary slot per model | sherpa-onnx int8 catalog with pins | Add Parakeet 110M (light tier) and Parakeet Ultra as selectable, pinned; default stays Parakeet v2 |
| Vocabulary | Decode-time rescoring plus sounds-like pass | Hotwords passed to a greedy decoder (inert), Tier 0 recasing and replacements | Make hotwords real: `modified_beam_search` with a BPE vocab built from `tokens.txt`, only when the dictionary is non-empty; vocabulary UI gains sounds-like rows |
| Text processing | Opt-in fillers and formatting, raw chord | Always-on regex Tier 0, some rewrites of wording | Narrow fillers and stutter rules, polish rewrites only in Professional, remove ambiguous recasing |
| Windows integration | Re-arm hook, swallow repeats, GetAsyncKeyState resync, clipboard restore | Event-only modifiers, repeat leak, data-loss paths, clipboard backup in history | Fix data loss (drain, cap), swallow repeats and key-up, resync modifiers, history exclusion default on, file log and panic hook |
| Performance | Warm model, GPU | Background load plus warm-up already | Keep; add tail and gate fixes; no new threads on the hot path |
| Settings and onboarding | Download starts at tour open, auto-advance, practice hints, mic privacy link | Manual download step, text-only mic hint | Download on Welcome Next with size shown, auto-advance, practice hints, privacy line, mic privacy button |

## 2. Per change: benefit, cost, risk

| Change | Benefit | Complexity | Cost or risk | Dependency or licence |
| --- | --- | --- | --- | --- |
| Hotwords via modified_beam_search | Names and jargon bias at decode time instead of nothing | M | Beam search is slower than greedy [UNV, not measured]; engine reloads when the dictionary becomes empty or non-empty; a wrong config can make sherpa exit, so a version guard and a config cross-check against the C header were required | None |
| Windowed level gate | Quiet laptop microphones stop being dropped as "no speech" | S | More quiet clips reach the VAD, which is cheap | None |
| Windowed-sinc resampler, DC blocker | Less aliasing above 8 kHz, especially 44.1 kHz microphones | M | Filter delay must stay compensated; written to keep incremental equal to one-shot; WER gain not measured | None, in-tree |
| Release tail 200 to 600 ms | Last word survives early key release | S | Mic stays open 0.2 to 0.6 s longer after release | None |
| Tier 0 hardening | Stops silent wording changes | S to M | Existing eval fixtures that encoded the old behaviour are updated | None |
| Catalog: Parakeet 110M, Ultra | Light tier and an English A/B candidate | S | Ultra may lose to v2 on Indian English and noise (v3 did); marked selectable only | CC-BY-4.0 (attribution), pins verified from downloaded archives |
| Waveform ballistics and teal working colour | Biggest perceptible visual upgrade | M | 16 ms timer only while listening, still no idle timer; needs eyes on a real screen | None |
| Onboarding, mic recovery, focus ring, vocabulary UI | First-run success, fewer dead ends | M | GDI layout changes cannot be viewed here; geometry was checked by reading | None |
| Data-loss fixes (A1, A2), hook fixes, privacy default | Correctness | S each | Hook edits are the highest regression risk; see section 4 | None |
| CI Windows release-candidate job | Reproducible artifact on a Windows runner | M | Installer not built in CI (needs an i686 clang for the NSIS plugin) | None |

## 3. Explicitly not done

- Parakeet Unified streaming and Nemotron live captions: the first is CPU-expensive and its sherpa runtime accuracy is under dispute upstream (PR 4133), the second needs a new online-recognizer FFI and loses to v2 on average WER. Revisit with measurements.
- Always-open microphone and pre-roll: breaks the "mic open only while holding the key" guarantee. Could return as an opt-in later.
- Replacing the default model: no available sherpa int8 archive clearly beats Parakeet v2 on both accuracy and hallucination safety.
- New UI framework or page consolidation: cannot be viewed here; layout risk judged too high for an unattended change.
- Moving the pipeline off the coordinator thread: larger refactor than this round justifies.
- Updater, code signing: no certificate may be invented; the build stays unsigned.

## 4. Merge and verification order

1. Four implementation branches (`feat/asr`, `feat/audio`, `feat/win`, `feat/ui`) from one base, disjoint files by design, merged into `feat/hex-informed-2.1`; conflicts only expected in `vi-app/src/main.rs`.
2. `cargo check --workspace --tests` for `x86_64-pc-windows-gnu` after each merge.
3. Release build for `x86_64-pc-windows-gnu` with the repo's GNU path, import table check (system DLLs only), portable zip and NSIS installer assembly, SHA-256 sums.
4. GitHub Actions Windows job as a second, independent artifact.
5. No tests, benchmarks or app launches were run, by the owner's instruction.
