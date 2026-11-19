# KeyReed latency on the i7-8850H / Quadro P2000

**Decision: leave eight configured inference threads alone.** Four ASR threads improved the median but did not establish a reliable p95 improvement. No model, decoder, streaming architecture, or user setting was changed. Instrumentation and replay: `e5194bc` on `perf/latency`. See [the pipeline map](perf-pipeline.md).

## Release-to-ready, not release-to-visible

One KeyReed process per configuration; live-speed WAV replay; local Parakeet v2 INT8, Smart cleanup, and no clipboard or audio dump. First run excluded from warm percentiles. p50 = median; p95 = nearest rank. The 21-clip mixed set had 4 real recordings and 17 TTS clips. Percentages use sums of the baseline stage times rather than sums of percentiles.

| 20 warm clips, milliseconds | 8 threads p50 / p95 | 4 threads p50 / p95 | Baseline time |
|---|---:|---:|---:|
| Final handoff through ASR | 1,924 / 4,115 | 1,641 / 3,157 | 69.9% |
| Cleanup | 0.37 / 6,377 | 0.46 / 6,678 | 29.3% |
| Total to ready | 1,938 / 10,519 | 1,642 / 9,172 | 100% |
| First process-cold clip (one sample each) | 17,242 | 21,788 | — |

One baseline cleanup call took 12.01 s and fell back to rules. Cold starts and cleanup outliers are separate from warm ASR. Warm median main-process working set: 850.27 vs 849.19 MiB, excluding the sidecar. In a separate approximately 17-second sample of the installed app, KeyReed + llama-server used 2,037 MiB combined working set and about 0.51% of the 12-logical-processor machine's capacity. Not a battery-life measurement.

## Accuracy and real speech

The repo scorer normalized 1,179 reference words across 44 checksum-verified human speech clips (LibriSpeech, ARCTIC ksp, EdAcc). Final-text WER was **120/1,179 = 10.18% for both thread counts**, with identical final text on every clip. On the separate mixed set: 20/393 = 5.09% for both. The 44-clip set includes Indian, Scottish and US speech but is not a dedicated technical-dictation or noise gate.

Among 36 warm human-speech clips in the 3–15 s range, replay-to-ready p50/p95 was 997/4,178 ms at eight threads and 816/5,787 ms at four. ASR accounted for 987/4,171 and 816/5,765 ms respectively. A build/test job overlapped part of the four-thread run: its WER is usable, its p95 is **not** a controlled estimate of a regression. The single 47.12 s clip was ready after 1,810 vs 1,088 ms; one clip is not a long-form percentile.

## Actual Windows delivery

Speaker playback of one 4.035 s WAV → OWL microphone → running installed app → synthetic Right Ctrl release → disposable Win32 EDIT length readback. A standalone trial observed the first character at 3,144 ms and the last length change at 3,526 ms. In a subsequent series, 10 successful readbacks had medians of 630 ms to first change and 1,020 ms to last change (nearest-rank p95s: 755 and 1,150 ms). The eleventh attempt lost focus; the harness aborted rather than redirect input. These runs used the **existing installed daemon**; the sandbox binary hosted only the target. Readback is not an on-screen paint timestamp, and text length is not transcript verification.

## Verification and limits

371 workspace tests passed; 7 ignored. The normal installer was rebuilt at `dist/KeyReed-Setup-x64.exe` (24,454,277 bytes, SHA-256 `66c3546b7d6bbb3bab98e0301a43cac44fc425cc0a0cd831f24f5d36fbcb8cd1`). Installed the sandbox variant and checked that its executable SHA-256 matches the release executable and the slim payload. The working-tree build includes the six pre-existing dirty files, not just the instrumentation commit. The user's regular installation was not replaced.

No verified after-gain to ship. Real release-to-visible percentiles in Notepad/browser/chat, rapid dictations, a 60 s microphone capture, silence/tap and sleep/resume on the new binary remain open. The optional audio dump is enabled in the user's existing config; benchmark config disabled it without modifying that file. No claim that the target is impossible on this hardware. Raw results: `bench/results/perf-local/` (machine-local, not committed).

## Insertion and edge cases (19 Nov)

Delivery was timed by stage with `KEYREED_PERF_TRACE` (62-character clip, 4 runs):

| stage | ms |
|---|---:|
| foreground restore + editor focus | 1–4 |
| clipboard backup | 0.3–1.4 |
| modifier drain | 0.7–0.9 |
| **SendInput keystrokes** | **508–1,051** |

The cost was the per-character `SendInput`, not its overhead. Delivering the same text with
Ctrl+V on the clipboard copy the path already writes: 15–21 ms, and the text appears in one
change instead of growing character by character. Warm release-to-text on the clip 2,278 → 894 ms.
Terminals and transcripts containing a newline keep the typed path; a failed clipboard write
falls back to typing.

Edge cases, simulated microphone:

| case | result |
|---|---|
| 60 s monologue, normal pauses | total 2.39 s, 8/9 chunks decoded during the hold |
| 100 s / 47 s clip | post-release 585 / 654 ms |
| silence only | filtered in 1 ms, nothing inserted |
| tap 0.7 s | filtered (deliberate minimum); 0.9 s and 1.2 s transcribe |
| continuous speech with no 250 ms pause | worst case one 24 s chunk ≈ 3.9 s; a 10 s cap made p50 3x worse, so 24 s stays |
| rapid back-to-back (0.2 s apart) | all dictations transcribe, but only one of four reached the target: open defect |
