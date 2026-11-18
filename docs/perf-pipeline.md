# Release-to-text performance pipeline

What happens between the push-to-talk key going up and text appearing in the focused field, in the
release build of v2.1.1, traced from the source at `1549416`. Every number below is a constant or a
setting read from the code, with its file. Nothing here is measured output; the numbers are the knobs,
not the timings.

`docs/architecture.md` describes the same pipeline as product behaviour. This page is the
engineering view: stage order, who blocks, which thread does the work, every sleep and timeout, every
file touched, and what privacy guarantees hold.

## 1. Settings that move the numbers

`crates/vi-core/src/config.rs`, `AppConfig::default()`.

| Setting | Default | Effect on this path |
| --- | --- | --- |
| `processing_profile` | `MaxPrivacyOffline` | Offline: no network at all, `allows_network()` false. `Balanced`/`MaxAccuracy` unlock the cloud tiers. |
| `local_asr_model` | `parakeet-tdt-0.6b-v2-int8` | Nemo transducer, INT8, ~460 MB download, ~690 MB RAM (`catalog.rs`). |
| `local_llm_model` | `qwen3.5-0.8b-q4_k_m` | GGUF handed to `llama-server.exe`; ~0 MB when idle-unloaded. |
| `inference_threads` | `4` | ASR threads (clamped 1..16) and the floor for `llama-server` threads on CPU. |
| `output_mode` | `DirectTyping` | Clipboard vs `SendInput` typing vs ask-first. |
| `default_writing_mode` | `Smart` | `Exact`/`TerminalCode` skip the language model entirely. |
| `auto_context_mode` | `true` | Per-app category may override the writing mode. |
| `idle_unload_seconds` | `300` | ASR and `llama-server` are freed after idle (×6 when ≥4 GB free, capped at 120 s when <2 GB). |
| `audio_cues_enabled` / `audio_cue_style` | `true` / `Soft` | Cue is played `SND_ASYNC`; it never blocks the coordinator. |
| `privacy.*` | all four on | Clipboard-history exclusion, password-field block, title redaction, RAM-only audio. |
| `history.enabled` (history file) | `false` | No transcript is persisted unless the user turns history on. |
| `debug_dump_wav` | `None` | Opt-in clip dump; the only path that writes speech to disk. |

## 2. Stages, in order

All on the `keyreed-coordinator` thread unless stated. Stage boundaries are the `t_release` /
`t_out` timers in `InferenceSupervisor::execute_pipeline` (`crates/vi-app/src/main.rs:633`).

| # | Stage | Code | Blocking | Notes |
| --- | --- | --- | --- | --- |
| 0 | Key-up → state `Processing` | `main.rs:2989` | No | `busy_flag` is cleared first so Escape reaches the user's app while the pipeline runs. |
| 1 | Release tail | `vi-audio/src/lib.rs:554` | Yes, ≤600 ms | Keeps capturing 200 ms, extended to 600 ms while speech continues. Polls every 15 ms. |
| 2 | PCM finalise | `vi-audio/src/lib.rs:523` | Yes, ms | Pause stream, drain + resample to 16 kHz, zeroize raw buffer. Whole clip = streamed part + tail. |
| 3 | ASR | `vi-asr/src/lib.rs`, `live.rs` | Yes | Streaming decode already ran during the hold, so release only finishes the last chunk. No live session under `MaxAccuracy` (whole clip decoded at release) or when a cloud STT key is configured. |
| 4 | Audio wipe | `main.rs:779` | No | `samples_16k.zeroize()` immediately after the transcript exists. |
| 5 | Tier 0 rules | `vi-rewrite/src/lib.rs:172` | Yes, sub-ms | Deterministic cleanup per paragraph; can drop the clip as a silence hallucination. |
| 6 | Tier 2 cloud, or Tier 1 local SLM | `lib.rs:228` / `lib.rs:263` | Yes | Cloud only when the profile allows network, a key exists, and (`MaxAccuracy` or no local SLM). Local SLM per paragraph, skipping ≤3-word and already-clean ones. |
| 7 | Guards | `vi-rewrite/src/guards.rs` | Yes, µs | Rejects the candidate and keeps Tier 0 text. |
| 8 | Delivery gate | `main.rs:885` | Yes | `vi_win32::input_gate()` is held for the whole delivery, so no second injection interleaves. |
| 9 | Clipboard / typing | `vi-win32/src/typing.rs:247`, `clipboard.rs:38` | Yes | Safety gates first, then `SendInput`. Skipped entirely when the capture targets the Composer or a voice command (`deliver_output = false`). |
| 10 | Bookkeeping | `main.rs:3043` | Yes | Metrics, optional history write, learning watcher, cue, pill state `ReadyToPaste` for 1.6 s. |

### Stage 1 — release tail (`vi-audio`)

* `POST_RELEASE_TAIL_MS = 200`, `MAX_RELEASE_TAIL_MS = 600`, frames of `TAIL_FRAME_MS = 20`.
* Loop sleeps 15 ms per iteration. At 200 ms it stops unless speech was running (5 of the first
  200 ms of frames above threshold, gaps up to 2 frames allowed); while speech carries on it stops
  after 8 consecutive quiet frames, and always at 600 ms. Speech = +12 dB over the clip's
  10th-percentile noise floor and above −66 dBFS.
* Skipped when the device is stalled/disconnected or for the simulated WAV microphone.
* The capture stream runs only while the key is held plus this tail; the WASAPI client stays open
  (prewarmed) but paused between clips.

### Stage 3 — speech recognition (`vi-asr`)

* Audio is 16 kHz mono f32, resampled with a windowed-sinc filter (cutoff 7.6 kHz, 16 zero
  crossings) and DC-blocked at capture time.
* Silero VAD: threshold 0.35, min silence 0.45 s, min speech 0.18 s, 512-sample windows, max speech
  14 s, `num_threads = 1`.
* Chunking (`ChunkPolicy`): spans closer than 8,800 samples (550 ms) merge; a chunk closes at 24 s
  max. Short spans wait `merge_gap + ONSET_MARGIN` (6,400 = 400 ms) of quiet; once a group holds 5 s
  of speech a 437 ms pause closes it immediately.
* Decode padding: 300 ms before (`PAD_PRE`), 250 ms after (`PAD_POST`); chunks under 1 s are
  zero-padded to 16,000 samples for Nemo transducers.
* Recogniser threads = `inference_threads` (4), hard-clamped 1..16; VAD is separate and single-threaded.
* Vocabulary non-empty + Parakeet → `modified_beam_search`, hotword score 2.0, 4 beam paths,
  requires sherpa-onnx ≥ 1.12.24. Empty vocabulary → greedy.
* Decoded audio is zeroized and dropped as each chunk completes (`lib.rs:951`).

### Stage 5–7 — cleanup

* Tier 0: fillers, stutters, self-corrections, casing, dictionary, spoken symbols in Terminal mode.
  `Exact`, ≤3 words, or an already-clean paragraph short-circuit at Tier 0.
* Local SLM: one `POST /v1/chat/completions` per paragraph, `temperature 0`,
  `max_tokens = words*2+24` clamped 32..384, `cache_prompt: true`, `--reasoning-budget 0`,
  `enable_thinking: false`. `llama-server` runs with `-c 2048 -np 1` on `127.0.0.1:<ephemeral port>`
  and a random 128-bit per-session `--api-key`.
* Threads: `-t/-tb` = `clamp(max(cfg, clamp(cores/2, 2, 6)), 1..min(cores-1, 8))` on CPU; on a
  discrete GPU `-ngl 99 -t 2 -tb 2 --device <Vulkan n>`. Integrated GPUs are never selected.
* Guards: output length 0.40–1.85× input, no assistant prefix, dictated question stays a question,
  every number / email / path / identifier / currency symbol preserved, ≤52 % novel long words.

## 3. Threads

| Thread | Owner | Role |
| --- | --- | --- |
| `keyreed-ll-hook` | `vi-win32/hotkey.rs` | Low-level keyboard/mouse hook. Posts events only. |
| main | `vi-app` | Message pump for settings, pill, Composer windows. |
| `keyreed-frameclock` | `vi-win32/frameclock.rs` | One 60 Hz (16,667 µs) waitable timer, `WM_APP_FRAME` to subscribed windows. |
| `keyreed-coordinator` | `main.rs:2202` | PTT state machine, capture drain, pipeline, delivery. **Blocks for the whole release path.** |
| `keyreed-live-asr` | `vi-asr/live.rs` | Owns the ASR engine for one recording; decodes finished chunks while the user speaks. |
| `keyreed-asr-prewarm` | `main.rs:536` | Loads the model off the hotkey path; result harvested on an idle tick. |
| `keyreed-gguf-verify` | `main.rs:367` | SHA-256 verification of the GGUF before it may be used. |
| `keyreed-gpu-detect` | `vi-rewrite/slm.rs:165` | `llama-server --list-devices`, ~1 s, off the hotkey path. |
| `keyreed-slm-slot-warmup` | `vi-rewrite/slm.rs:229` | Waits for `/health`, then evaluates the few-shot prefix into the KV slot. |
| `keyreed-learning` | `vi-app/learning.rs` | Edit watcher; created on first typed insertion. |
| `keyreed-text-jobs` | `vi-app/textjobs.rs` | Quick Fix / Composer jobs; shares the `RewriteEngine` mutex. |
| `keyreed-uia`, `keyreed-stream-read`, `keyreed-paste` | `vi-win32`, `vi-rewrite` | UIA events, streaming completion read, Quick Fix paste. Not on the dictation path. |
| `keyreed-timing-log` | `vi-win32/security.rs:251` | Only with `KEYREED_TIMING_LOG`; keeps file IO off the hook. |
| OS audio callback | cpal/WASAPI | Appends frames, updates RMS; the coordinator drains it, never the reverse. |
| `llama-server.exe` | child process | Its own inference threads per `-t`. Killed on idle unload and on exit. |

## 4. Sleeps, waits and timeouts

| Where | Value |
| --- | --- |
| Coordinator tick (`tick(75 ms)`) | 75 ms — capture drain, live feed, RMS sync, watchdogs |
| PTT min hold / session cap | 110 ms / 1,800,000 ms (`PttController::new`) |
| Release-tail poll | 15 ms, ≤600 ms total |
| Stale-state heal / no-audio cancel | 2 s / 3 s |
| Rewrite lock wait (`SLM_LOCK_WAIT`) | 250 ms, then rules-only (`tier0_slm_busy`) |
| `llama-server` health poll | 80 ms, deadline 15 s CPU / 45 s GPU |
| `llama-server` health request | connect 250 ms, read 250 ms |
| `llama-server` prewarm | health poll 60 ms, up to 60 s |
| SLM completion read timeout | 12 s, 60 s for the first GPU reply (shader compile) |
| Cloud STT | connect 4 s, read 12 s |
| Cloud LLM | connect 3 s, read 6 s |
| Clipboard open retry | 12 attempts, `2 + 3i` ms (≈180 ms worst case) |
| Modifier release wait | 9 × 10 ms (≈90 ms) |
| `SendInput` chunk pause | 2 ms between 64-event chunks |
| Foreground restore poll | 8 × 15 ms |
| Pill transient states | 1.4 s (nothing heard), 1.6 s (ready), 3.5 s (error) |

## 5. IO touched by one dictation

| When | What | Path |
| --- | --- | --- |
| Every clip | Model files read (mmap by sherpa/llama) | `%LOCALAPPDATA%\keyreed\models\…`, `C:\Program Files\KeyReed\models`, or bundled `runtime\sherpa-onnx` |
| Only if `debug_dump_wav` | One WAV + two lines in `last-clips.jsonl` | path given by the user |
| Only if `KEYREED_TIMING_LOG` | `unix_ms event` lines | env path |
| Only if `KEYREED_PERF_TRACE` | One JSON metrics line per result (no text) | env path |
| After a result, only if history on | Whole `history.json` rewritten atomically (100 entries / 7 days) | `%LOCALAPPDATA%\keyreed\history.json` |
| After a result, typed delivery | Learning worker reads the field (UIA) | in memory only |
| Settings change | Whole `config.json` rewritten atomically | `%LOCALAPPDATA%\keyreed\config.json` |
| Warnings/panics | `keyreed.log`, rotated at 1 MB | `%LOCALAPPDATA%\keyreed\logs\keyreed.log` |
| Delivery | Clipboard set once as a backup, once for clipboard mode | OS clipboard |
| SLM | Loopback HTTP per paragraph | `127.0.0.1:<ephemeral>` |

Nothing on this path writes audio to disk. The config is rewritten on settings changes and after a
learned word, not per dictation; history, when enabled, is rewritten in full per result.

## 6. Privacy on this path

* **Audio.** In RAM only, 16 kHz f32. Streamed chunks are zeroized in the worker as they are decoded;
  the raw buffer is zeroized on drain; the full clip is zeroized right after ASR. The only disk dump
  is the opt-in `debug_dump_wav`.
* **Cloud.** Gated by `ProcessingProfile::allows_network()` and HTTP is HTTPS-only (`cloud.rs`,
  `cloud_llm.rs`). Audio goes as a `multipart/form-data` WAV built in memory and zeroized after send;
  the request body is zeroized too. `MaxPrivacyOffline` makes no network call at all.
* **API key.** Stored DPAPI-encrypted (`encrypted_api_key_b64`), decrypted into a local `String` per
  pipeline run and `zeroize`d after the rewrite.
* **Local model server.** Loopback bind only, ephemeral port, per-session random Bearer key, no shell,
  `CREATE_NO_WINDOW`, single slot. Killed on idle unload, on settings change and on exit.
* **Delivery.** Password fields and password managers fall back to the clipboard with history
  exclusion forced on; elevated targets (UIPI) and shell/desktop windows fall back instead of typing;
  Terminal text has newlines collapsed so dictation can never run a command. Bidi override and
  control characters are stripped before injection.
* **Clipboard.** `CanIncludeInClipboardHistory`, `CanUploadToCloudClipboard` and
  `ExcludeClipboardContentFromMonitorProcessing` are set to 0 for every dictation write; the typed
  path's backup copy always excludes, whatever the setting says.
* **History.** Off by default; when on it stores text, never audio, and can be cleared wholesale.
* **Logs.** Level `warn` by default. Window titles are redacted for private/incognito windows and
  password managers. Panic messages that could quote a transcript are withheld.
* **Learning.** Only typed insertions are watched; field text stays inside the `keyreed-learning`
  worker, and only a learned word pair plus the dictionary cross back.

## 7. Measuring it

`KEYREED_PERF_TRACE=<file>` appends one JSON object per result (`PipelineMetrics` in
`vi-core/src/types.rs`), fields: `audio_duration_ms`, `vad_ms`, `asr_ms`, `rewrite_ms`, `output_ms`,
`total_release_to_ready_ms`, `used_tier`, `asr_model_id`, `mic_init_ms`, `first_frame_ms`,
`chunk_finalize_ms`, `model_load_ms`, `post_release_asr_ms`, `chunks_total`, `chunks_streamed`,
`norm_ms`, `tier0_ms`, `slm_startup_ms`, `slm_first_token_ms`, `slm_ms`, `paragraph_ms`,
`target_id_ms`, `end_to_end_ms`, `cpu_ms`, `working_set_mb`, `cold_model`.

`KEYREED_TIMING_LOG=<file>` adds `unix_ms event` marks: `coordinator_start_recording`,
`mic_started`, `mic_prewarm`, `live_session_ms`, `press_path_ms`, `coordinator_stop`,
`clipboard_ready`.

For microphone runs, `KEYREED_TIMING_LOG` also writes `dictation_stages` with one ID shared with
`KEYREED_PERF_TRACE`. Times are monotonic milliseconds relative to hook key-up (`t0_src=hook`)
or coordinator stop (`t0_src=coordinator`). t1 is the final buffer, t2 enters ASR, t3 is transcript
ready, t4 is cleanup complete, t5 starts delivery. A missing stage is `na`. There is no in-process t6.

`end_to_end_ms` includes the hold. `total_release_to_ready_ms` starts **after microphone finalization**,
not at key-up. With CLI replay it also excludes target delivery. Do not add total ASR time to it:
some decode runs during speech. To test delivery, `bench/acoustic_release.py` measures synthetic
key-up to the first and last observed text change in a disposable EDIT; this is not a paint timestamp.

## 8. Known gaps

* `mic_init_ms` and `press_path_ms` are wall-clock on the coordinator; a cold WASAPI open can dominate
  the first press after login.
* Cloud tiers have no retry: a timeout falls through to the next tier, so a failed cloud call is
  counted in `rewrite_ms`/`asr_ms` and looks like local slowness.
* `output_ms` includes the clipboard retry loop and the 2 ms per-chunk typing pause, so it scales with
  text length, not with frame rate.
* History is a full-file rewrite; with a large history the bookkeeping step can exceed the delivery.
* The benchmark in `crates/vi-app/src/main.rs` (`press_path_bench_*`) exercises the press path only,
  not delivery.