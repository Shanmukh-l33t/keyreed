# KeyReed v0.5.0 end-to-end speed and accuracy: worklog and handoff

Branch: `perf/e2e-speed-accuracy-v0.5.0` (from `main` @ 3d9b5c5, v0.4.0). Never force-push.
This file is the single source of truth for whoever continues the work (human or agent).
Update the "Status" and "Next steps" sections whenever a step lands.

## Machine

Intel Core i7-8850H (6 cores / 12 threads, AVX2, no AVX-512), 64 GB RAM, Windows 11,
NVIDIA Quadro P2000 + Intel UHD 630 (unused: inference is CPU only).
Toolchain: `stable-x86_64-pc-windows-gnu`, cargo 1.100. Build: `cargo build --release -p vi-app`.
Baseline binary + runtime + models: `dist/keyreed-win-x64/keyreed.exe` (byte-identical to the
installed v0.4.0 in `%LOCALAPPDATA%\Programs\KeyReed`, which is running as the user's real app).
To run a dev build against the bundled runtime/models: set `KEYREED_RUNTIME_ROOT=<repo>/dist/keyreed-win-x64`.
User config: `%LOCALAPPDATA%\keyreed\config.json` (Parakeet v2, mode technical, `local_llm_model: none`,
hold-to-talk on Right Ctrl, direct typing). Extra models are in `%LOCALAPPDATA%\keyreed\models`.

## Baseline measured on this machine (v0.4.0, Parakeet TDT 0.6B v2 int8, Exact mode, no SLM)

`python3 bench/run_bench.py --exe dist/keyreed-win-x64/keyreed.exe --tag v0.4.0-baseline ...`
Raw data: `bench/results/v0.4.0-baseline/` (gitignored). Synthetic TTS clips.
CAVEAT: three research subagents and a 346 MB download were running, so absolute numbers are
pessimistic. Re-run on a quiet machine before quoting them (same command).

| Clip (speech) | Cold: release to text | Warm: release to text |
|---|---:|---:|
| 4 s | 14.9 s | 0.8 s |
| 15 s | 22.0 s | 3.6 s |
| 23 s | 26.2 s | 5.1 s |
| 47 s | 26.3 s | 9.8 to 19.9 s |
| 81 s | 55.7 s | 34.9 to 41.7 s |
| 215 s | 152.9 s | 68 to 77 s |
| 446 s | 116.1 s | 85 to 103 s |

Peak working set 0.8 to 1.3 GB. Output of every long clip is one single paragraph.

## Root causes found by reading the code (crates/vi-app/src/main.rs unless noted)

1. All recognition happens after key release. `execute_pipeline` gets the whole recording and
   decodes every VAD segment one after another. Nothing overlaps with speaking.
2. The speech model is loaded on the first key-down, not at startup, and is unloaded after
   `idle_unload_seconds` (default 300). Every load re-hashes about 660 MB (SHA-256) and builds the ONNX session.
3. `PttController::new(.., 110, 120_000)`: a recording longer than 120 s is DISCARDED
   (`recording_cap_discarded`). Two-minute dictations are lost entirely.
4. Chunks are joined with a single space (`pieces.join(" ")`): no paragraphs anywhere in the pipeline.
5. Grammar model (vi-rewrite/src/slm.rs): `-c 1024` context, `max_tokens` capped at 384, 20 s read
   timeout, whole transcript in one request. A one-minute dictation overflows or times out and then
   falls back to Tier 0 after the wait. llama-server is started on key-down (about 12 s cold).
6. Microphone disconnect mid-recording replaces `active_mic`, dropping the audio captured so far.
7. `tier0.rs` compiles about 30 regexes on every call (measure before touching).

## Design being implemented

A. Streaming chunked recognition (vi-asr/src/lib.rs). DONE in source, not compiled yet:
   - `LocalAsrEngine::stream_begin / stream_feed / stream_finish / stream_reset / warm_up`;
     `transcribe()` is now `stream_begin + stream_push + stream_finish` (one code path).
   - Silero VAD runs incrementally. Finished speech spans are grouped by `ready_groups()`
     (pure function, unit tested): spans closer than `ChunkPolicy.merge_gap` (default 0.55 s, the
     v0.4.0-equivalent value) are decoded together, capped at `max_chunk` (24 s). A group is decoded as
     soon as no later speech can merge into it. Dev knobs: `KEYREED_ASR_MERGE_GAP_MS`, `KEYREED_ASR_MAX_CHUNK_S`.
   - Result carries `sentences: Vec<(String, pause_before_ms)>`. Pauses come from VAD gaps between
     chunks and from the model's token timestamps inside a chunk (`sentence_pauses`).
   - Decoded audio is zeroized and dropped as it is consumed.
   STILL TO DO in this file: add fields `stream: StreamState, policy: ChunkPolicy` to
   `LocalAsrEngine` (+ init in `load`), make `decode_single_chunk` return
   `(text, lang, tokens, timestamps)`, `use zeroize::Zeroize`, add `vad_detected`
   (`SherpaOnnxVoiceActivityDetectorDetected`) to `ffi.rs`, then a `LiveAsr` worker
   (std::sync::mpsc; owns the engine during a recording; messages Audio(Vec<f32>) / Finish; dropping
   the sender = cancel; returns the engine plus the result and timings).
B. Capture (vi-audio): add `ActiveCapture::take_new_16k()` that drains and resamples the audio
   captured since the last call (48 kHz: multiples of 3; other rates: whole gcd blocks), with a test
   that chunked output equals one-shot output. The coordinator calls it on every 75 ms tick and sends
   the audio to `LiveAsr`. `stop_and_take_16k()` then returns only the tail. This also fixes cause 6.
   When cloud STT is selected (MaxAccuracy + key) keep the old whole-buffer path.
C. Model lifecycle (main.rs `InferenceSupervisor`): load the selected speech model in the
   background right after startup and after a model switch, run `warm_up()` in that thread, poll the
   pending load on the tick so the UI can show readiness, and only idle-unload when free RAM is short
   (GlobalMemoryStatusEx). Keep SHA-256 verification before every load. Measure hash vs session-build time first.
D. Paragraphs (new vi-rewrite/src/paragraphs.rs): deterministic, from sentence pauses plus length
   limits plus discourse markers; no paragraphs for Terminal/Code or short dictations. `RewriteEngine::process`
   then runs Tier 0 / SLM per paragraph and joins with a blank line. Calibrate thresholds on real speech.
E. Recording cap: raise 120 s to 30 min.
F. Grammar model: `-c 2048`+, rewrite per paragraph (40 to 60 words) with per-chunk guard and
   fallback, prime the few-shot prefix at prewarm (`n_predict: 0`), start the server at app start,
   timeouts by length, benchmark `-t 6`. Research brief with verified llama.cpp b12412 flags:
   `C:\Users\ImMandatory\keyreed_cleanup_stage_brief.md` (copy the useful parts into docs/research).
G. Instrumentation: extend `PipelineMetrics` (serde defaults) with mic open, model wait, chunks,
   audio decoded before release, post-release ASR, tier0, slm (prompt_ms/predicted_ms from
   llama-server `timings`), paragraph, output, key-down to delivered, CPU time, working set. Write one
   JSON line per dictation to the file named by `KEYREED_PERF_TRACE` (timings only, never text).
H. CLI: `--transcribe-wav <f> --live [--live-speed N]` feeds the WAV through `LiveAsr` at real-time
   pace and reports post-release latency. `bench/run_bench.py --live` already passes the flag.
I. Real-app E2E: a simulated microphone (env `KEYREED_TEST_AUDIO_WAV`, WAV fed in real time) so the
   daemon, hotkey, overlay and insertion can be timed unattended. Label such runs "simulated capture".
   The user's installed v0.4.0 holds the single-instance mutex: stop it for daemon tests and restart it after.

## Benchmark assets

- `bench/corpus_text/passages.json`: reference text with paragraph ground truth (5 documents x 4
  paragraphs, 14 short items with `must_keep` tokens).
- `bench/tts_synth.ps1` + `bench/make_tts_corpus.py` (stdlib): 57 synthetic clips, 38 min, voices
  David/Zira/Mark (en-US) and Heera/Ravi (en-IN), fast/slow/noisy/quiet/long-pause variants, nominal
  5 s to 600 s. Output `bench/corpus/tts/` + `manifest.jsonl` (gitignored; regenerate with the script).
- `bench/run_bench.py` (stdlib): one process per clip, run 1 cold, runs 2..N warm; records the
  binary's own metrics plus process CPU seconds and peak working set.
- `bench/cache/test-clean.tar.gz`: LibriSpeech test-clean (CC BY 4.0, 346 MB) downloaded, not yet
  extracted. Plan: `bench/make_librispeech_corpus.py` that concatenates consecutive utterances of one
  chapter into real-speech clips of 60/120/300/600 s with references (flac to wav via ffmpeg).
- TO WRITE: `bench/score_bench.py` (stdlib): normalised WER/CER, insertions/deletions (hallucination
  and omission rates), repeated-segment check, `must_keep` hits, sentence-boundary and paragraph-boundary
  precision/recall against the manifest, latency p50/p90/p95, real-time factor.
- Legacy `bench/build_corpus.py`, `run_asr.py`, `score.py` need Linux paths and pip packages; leave them.

## Research in flight

Three desk-research briefs were commissioned: (1) ASR models/runtimes (sherpa-onnx candidates beyond
the six, streaming vs chunked, DirectML, Indian English), (2) grammar/punctuation/paragraph stage
(done, file above), (3) licensed speech test data with direct URLs. Fold results into
`docs/research/09-model-and-runtime-candidates.md` with licences and a rejected list. No candidate
model becomes a product dependency before it is benchmarked here.

## Status

- [x] Baseline reproduced, causes identified.
- [x] TTS corpus + runner.
- [x] A streaming recognition in `vi-asr` (`lib.rs` stream_* API, `live.rs` LiveAsr worker): compiles,
      `cargo test -p vi-asr` = 18 passed. NOT yet verified on real audio: the first
      `cargo build --release -p vi-app` + one-shot sanity run on `tts_060s_david.wav` was started but its
      result was not read. Re-run it first and compare the text with `bench/results/v0.4.0-baseline`.
- [ ] Wire LiveAsr into `main.rs` (nothing in vi-app uses it yet), then B, C, D, E, G, H, then re-run the bench with `--live`.
- [ ] score_bench.py + LibriSpeech corpus + all six models.
- [ ] F grammar model, I real-app E2E, candidate models, docs, version bump 0.5.0, package, release.

## Next steps, in order

0. BLOCKER FIRST: `cargo build --release -p vi-app` from a plain shell "finishes" but the exe
   segfaults on start (even `--version`). Cause: no `windres`/gcc/clang on PATH, so rustc links with its
   bundled linker instead of the llvm-mingw UCRT toolchain the working v0.4.0 exe was built with.
   The toolchain is probably under `C:\Users\ImMandatory\.local\toolchain` (look for `windres.exe`);
   put its `bin` first on PATH (and check the `libgcc.a`/`libgcc_eh.a` aliases described in the
   `windows-desktop-app-dev` skill), rebuild, confirm `target/release/keyreed.exe --version` runs.
   The vi-asr code itself is NOT known to be at fault: it has never executed.
   Run dev builds with `KEYREED_RUNTIME_ROOT=C:/Users/ImMandatory/keyreed/dist/keyreed-win-x64`.
   Then sanity: `--transcribe-wav bench/corpus/tts/tts_060s_david.wav --llm-model none --mode exact --repeat 2`.

New since the first commit: `crates/vi-rewrite/src/paragraphs.rs` (written with tests, NOT yet added to
`vi-rewrite/src/lib.rs` as `pub mod paragraphs;`, never compiled) and `bench/score_bench.py` (works).
Scored baseline (7 TTS clips, 13.9 min, loaded machine): WER 0.44 %, sentence P/R 1.0/0.994,
paragraph recall 0.0, warm release-to-text p50 14.8 s, p95 93.9 s, cold p50 26.3 s.
Hermes delegation is now `backend: default`, `model: gemini-4-carbon`, `provider: surplus` (was
`profile_routes`, which ignored the model pin). Two research briefs (ASR models, datasets) finished
on the old setting; their text arrives as a completion message, and the grammar brief is the file named in F.

1. Finish A, make `cargo test --workspace` pass.
2. B + LiveAsr + coordinator wiring + CLI `--live`; compare text of live vs one-shot on the corpus.
3. G metrics, then run `bench/run_bench.py --exe target/release/keyreed.exe --tag v0.5.0-r1 --live`.
4. D paragraphs + scorer; E cap; C lifecycle.
5. Commit after each step. Rules: every claim in docs must come from a measured result file;
   mark items Verified / Partially verified / Not verified / Failed.

## Working notes

- Repo files use CRLF. Shell heredocs containing Rust code break in this terminal: write files with
  the file tool or a script file instead.
- Acceptance rule: live output text must not lose words relative to v0.4.0 one-shot output.
- Do not change `SileroVad` thresholds without a WER comparison; they were tuned in live mic tests.
