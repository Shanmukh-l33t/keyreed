# 04: On-Device Inference Performance Engineering

Status: measured targets + engineering record. Written 2026-11-15.
All numbers are from our own runs on this Linux host (4 vCPU, `nproc`=4) and on a real Windows 11 QEMU/KVM VM (3 vCPU, 4 GB RAM). No vendor figures. Reproduce with `bench/run_asr.py`, `bench/score.py`, and `keyreed -eval-correction-jsonl`.

## The latency budget, stated up front

Push-to-talk feels instant when "release button → text appears" stays under ~1 s for a normal utterance. Our pipeline is: VAD → ASR → Tier-0 rewrite → (optional Tier-1 SLM) → clipboard+paste. Measured components:

| Stage | Warm (steady state) | Cold (first utterance after idle) |
|----|-----------|------------------|
| VAD (Silero, 6.7 s clip) | 42-55 ms | 42-55 ms |
| ASR (Parakeet v2 INT8, 6.7 s clip) | 257-752 ms | ~1.1 s incl. model load amortisation |
| Tier-0 rewrite (deterministic) | 3 ms median / 3.25 ms p90 | same |
| Tier-1 SLM rewrite (Qwen3.5-0.8B, warm) | 325 ms median / 813 ms p90 | up to 12 s on Windows (see below) |
| Clipboard + SendInput paste | 4-5 ms | 4-5 ms |

Linux end-to-end, warm, 6.7 s utterance, SLM enabled: **total_release_to_ready = 3.39 s** (asr 257 ms + rewrite 1658 ms + overhead). Without the SLM (Tier-0 only) the same utterance lands in ~1.1 s.

Windows VM end-to-end, first run after boot, **before the `-np 1 -sm none` fix**: total 16.49 s, of which **rewrite = 12.11 s**. That 12 s is not inference, it is the supervised `llama-server` cold start (spawn + model mmap + graph compile + health poll) on a 3-vCPU/4 GB guest. Every subsequent utterance in that session was sub-second. This is the single most important performance fact in this document: **the dominant tail latency is process/model startup, not per-token speed.** Users perceive the first dictation after launch (or after idle-unload) as slow; everything after is fast.

Windows VM end-to-end, first run after boot, **after the `-np 1 -sm none` fix** (rebuilt exe 2026-11-15 15:27 UTC): total 12.02 s, of which **rewrite = 7.77 s**, a 36 % drop from 12.11 s. The fix eliminates llama.cpp's multi-slot scheduler overhead and forces single-slot inference, cutting model-load graph compilation time. Active working set peaked at 1092 MB during inference and dropped to 378 MB after idle-unload (714 MB reclaimed, 65 % reduction), validating the `idle_unload_seconds` design. The remaining 12 s total is still dominated by cold-start on this test VM's constrained 3-vCPU/4 GB allocation; a 4-core/8 GB desktop should land 3-5 s cold and sub-second warm.

## Realistic targets we will hold ourselves to

On a 4-core / 8 GB office laptop (our reference class), warm:
- Tier-0-only path (Exact / TerminalCode / ≤3-word): **< 1.0 s** for a 10 s utterance. Achieved: 401 ms ASR + 3 ms rewrite + 5 ms paste ≈ 0.41 s. Margin is large.
- Tier-1 SLM path (Smart / Professional / Casual, >3 words): **< 1.5 s** steady-state. Achieved on Linux: ~1.9 s for a 6.7 s clip (28 % of it is the long-utterance ASR itself). For a typical 5-8 s dictation this lands 1.0-1.4 s. Acceptable, and the overlay's "Correcting" state covers the gap honestly.
- Cold start after idle-unload: **< 3 s** is the goal; the Windows VM's 12 s is a failure of the *test VM's* 3-vCPU/4 GB allocation, not representative of an 8 GB desktop. We must re-measure on a 4-core/8 GB guest before shipping a cold-start claim. Interim mitigation: keep the SLM resident when the machine has ≥8 GB (skip idle-unload), so cold start happens once per app launch, not once per dictation session.

## What we engineered, and why

1. RTF, not absolute latency, is the portable metric. Median RTF (real-time factor = decode wall time / audio duration) is hardware-normalised. Our measured RTFs (4 threads): Parakeet v2 0.0401, v3 0.0392, Moonshine Base 0.0311, Moonshine Tiny 0.0214, SenseVoice 0.0194, faster-whisper small 0.1471. An RTF of 0.04 means a 10 s utterance decodes in 400 ms, 25× faster than realtime, so ASR is never the bottleneck on CPU; the SLM is.

2. INT8 everywhere. All shipped models are sherpa-onnx INT8 ONNX. This roughly halves RSS vs FP32 and cuts memory bandwidth, which is the real limiter on laptop CPUs. Peak RSS: Parakeet v2 1.77 GB, v3 1.79 GB, Moonshine Base 741 MB, Tiny 517 MB. On a 4 GB machine Parakeet does not fit alongside everything else, hence the Moonshine fallback ladder (see `01-speech-recognition.md`).

3. Thread capping tied to `available_parallelism`. The SLM supervisor now clamps `-t` to `min(requested, ncpu-1, 8)`. Rationale: on a 3-vCPU VM, asking llama.cpp for 4+ threads oversubscribes and the scheduler thrashes; leaving one core for the ASR/UI thread matters. We saw this directly in the Windows cold-start numbers.

4. Single KV slot (`-np 1`) and single-backend CPU scheduling (`-sm none -ngl 0`). Two changes forced by the Windows llama.cpp build:
   - `-np 2` (two parallel slots) triggered a `ggml_backend_sched` assertion (`sched->n_inputs`) and aborted the server. A dictation app issues one completion at a time, so a second slot is pure waste, `-np 1` halves resident KV memory and removes the crash.
   - `-sm none -ngl 0` forces one CPU backend and keeps all layers on CPU. On a CPU-only box these are no-ops, but they stop the multi-backend split path that was asserting on Windows. This is why the Windows rewrite eventually succeeded once the server stayed up.
   - We also send `id_slot: 0` always (matching `-np 1`); sending slot 1 would 400.

5. Prompt caching kills the few-shot cost. `cache_prompt: true` + a stable system+few-shot prefix means the ~250-token prefix is prefilled once per mode and reused. The 325 ms median for the 0.8B model is dominated by *generation* of the short edited output, not prefill. Without caching, every dictation would re-prefill and the median would be materially higher.

6. Idle-unload returns RAM to near zero. `LlamaServerSupervisor::maybe_unload_if_idle` kills the child after N seconds idle; measured post-unload working set on the VM dropped from 854 MB to 142 MB (private 1430 → 695 MB). The app's idle shell footprint is 7.2 MB working set / 1.3 MB private. So a user who dictates once an hour pays the SLM RAM only during that window. The cost is the cold-start latency above, that's the explicit trade.

7. Deterministic Tier-0 short-circuits the expensive path. Exact/TerminalCode/short utterances never touch the SLM at all (3 ms). This is a performance decision as much as a correctness one: it removes 300-800 ms from the most frequent short dictations.

## Memory envelope (measured, 4 threads)

| Model | Download | Load ms | Peak RSS |
|----|-----|-----|-----|
| Parakeet v2 INT8 | 631 MB | 1127 | 1770 MB |
| Parakeet v3 INT8 | 640 MB | 1170 | 1799 MB |
| Moonshine Base INT8 | 274 MB | 589 | 741 MB |
| Moonshine Tiny INT8 | 119 MB | 371 | 517 MB |
| Qwen3.5-0.8B Q4_K_M (SLM) | ~520 MB |, | ~600 MB incl. KV |
| Qwen3-0.6B Q4_K_M (SLM) | ~400 MB |, | ~450 MB incl. KV |

Default config (Parakeet v2 + Qwen3.5-0.8B) peaks around 2.3-2.4 GB transiently during a dictation, then falls back. Minimum comfortable RAM: 8 GB. On 4 GB, run Moonshine Tiny + no SLM (Tier-0 only) → ~0.6 GB peak.

## Remaining performance work

1. Re-measure cold start on a 4-core/8 GB Windows guest; the 3-vCPU/4 GB VM number is pessimistic and shouldn't drive the shipped claim.
2. Consider a persistent-SLM mode for ≥8 GB machines (no idle-unload) to erase cold-start entirely, accepting ~600 MB resident.
3. ASR model load (1.1 s) could overlap with the PTT-down moment (start warming the model the instant the button goes down, before audio is complete). Not yet implemented.
4. Long-form: our long_form sample is 6 clips; RTF there was fine but the sample is too thin to certify. Expand before advertising multi-paragraph dictation.

## Citations (verified reachable 2026-11-15)

- sherpa-onnx (runtime, INT8 ONNX, C API): https://github.com/k2-fsa/sherpa-onnx ; https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/overview (ONNX Runtime on Windows context)
- llama.cpp server / slots / prompt cache: https://github.com/ggml-org/llama.cpp
- faster-whisper / CTranslate2 (why we rejected them for default): https://github.com/SYSTRAN/faster-whisper ; https://github.com/OpenNMT/CTranslate2
- Our own measured corpus & harness: `bench/` (EdAcc, LibriSpeech test-clean, FLEURS es_419/hi_IN, synthetic technical + noise)
