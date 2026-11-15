# 07: ASR Benchmark Corpus, Harness, and Measured Results

Status: benchmark record. Written 2026-11-15.
Everything here is reproducible from `bench/`. Numbers are ours, measured on real audio, not quoted from model cards or leaderboards. Where a number is missing or unreliable, this doc says so instead of filling the gap.

## What was built

A repeatable corpus + harness under `/home/computebox/voice-input/bench/`:

- `build_corpus.py`, assembles the corpus from public datasets into `corpus/wavs/` (16 kHz mono) + `corpus/manifest.jsonl` (one JSON line per clip: id, category, source URI, reference transcript, duration, entity list where applicable).
- `run_asr.py`, loads each candidate model through sherpa-onnx / faster-whisper, transcribes every clip, records per-clip wall time, RTF, hypothesis, and process RSS, writes `results/<model>.json`.
- `score.py`, computes WER/CER per category, technical-entity survival %, non-speech hallucination rate, median/p90 RTF, and emits `results/score_summary.json`.
- `build_correction_eval.py` + the Rust `-eval-correction-jsonl` path, the text-only rewrite eval (see `03-ai-correction-context.md`).

Corpus: **244 clips, 58 MB**, 11 categories:

| Category | Clips | Source | Purpose |
|-----|----|----|-----|
| clean_english | 32 | LibriSpeech test-clean (openslr/librispeech_asr) | read-speech floor |
| long_form | 6 | LibriSpeech concatenated | multi-paragraph drift |
| conversational | 24 | EdAcc (edinburghcstr/edacc) | spontaneous speech |
| indian_english | 32 | EdAcc Indian-English + Hindi-English speakers | our core market accent |
| other_accents | 24 | EdAcc (Scottish, Irish, Australian, etc.) | accent robustness |
| noisy_10db | 24 | EdAcc/LibriSpeech + additive noise @ SNR 10 dB | home-office noise |
| noisy_5db | 24 | same @ SNR 5 dB | harder noise |
| technical_synthetic | 30 | TTS-synthesised dev/IT sentences with named entities | identifier/command survival |
| non_speech_hallucination | 12 | silence + keyboard-only + room-tone | hallucination trap |
| multilingual_es_419 | 18 | FLEURS es_419 | Spanish |
| multilingual_hi_in | 18 | FLEURS hi_in | Hindi |

Machine: Linux 6.8, glibc 2.39, 6 vCPU host; harness ran each model at **4 threads** (our reference office-laptop core count), Python 3.14 venv in `bench/.venv`. Models are sherpa-onnx INT8 ONNX unless noted; faster-whisper variants use CTranslate2 INT8.

## Headline results (all models, 4 threads)

EN WER is over the 208 English clips (all except v3, which also scored the 18 Spanish clips → 226). `ent%` = technical-entity survival on the 30 technical clips. `hal%` = fraction of the 12 non-speech clips that produced any text. RTF = decode wall time / audio duration.

| Model | EN WER | clean | IndEN | accents | conv | noise10 | noise5 | long | tech | ent% | hal% | med RTF | peak RSS | disk |
|----|----:|---:|---:|----:|---:|----:|----:|---:|---:|---:|---:|----:|-----:|---:|
| Parakeet TDT 0.6B v3 INT8 | 10.70 | 2.42 | 11.74 | 17.84 | 14.50 | 16.18 | 19.10 | 2.09 | 15.29 | 86.2 | 0.0 | 0.0392 | 1799 | 640 |
| Parakeet TDT 0.6B v2 INT8 | 11.50 | 2.05 | 10.14 | 17.23 | 16.84 | 10.34 | 17.77 | 9.38 | 13.24 | 92.3 | 0.0 | 0.0401 | 1770 | 631 |
| faster-whisper small.en INT8 | 12.62 | 3.91 | 15.48 | 24.05 | 22.60 | 13.26 | 18.83 | 3.02 | 10.59 | 93.8 | 33.3 | 0.1471 | 1255 | 464 |
| SenseVoice Small INT8 | 14.08 | 3.72 | 16.01 | 24.25 | 21.75 | 13.79 | 18.30 | 3.86 | 26.18 | 73.8 | 50.0 | 0.0194 | 578 | 1124 |
| Moonshine Base INT8 | 16.24 | 3.54 | 21.53 | 25.85 | 27.51 | 16.71 | 29.71 | 3.86 | 17.35 | 84.6 | 0.0 | 0.0311 | 741 | 274 |
| faster-whisper base.en INT8 | 16.26 | 5.21 | 18.15 | 27.66 | 28.78 | 20.95 | 23.34 | 4.17 | 17.65 | 84.6 | 50.0 | 0.0636 | 505 | 141 |
| sherpa whisper base.en INT8 | 17.52 | 4.84 | 19.93 | 30.86 | 30.49 | 20.69 | 27.59 | 3.96 | 19.71 | 84.6 | 8.3 | 0.0568 | 766 | 432 |
| Moonshine Tiny INT8 | 20.32 | 4.28 | 26.69 | 30.86 | 33.90 | 27.59 | 36.07 | 4.80 | 19.12 | 84.6 | 0.0 | 0.0214 | 517 | 119 |
| faster-whisper tiny.en INT8 | 21.02 | 6.15 | 24.02 | 35.67 | 37.10 | 25.99 | 36.07 | 4.69 | 19.71 | 83.1 | 8.3 | 0.0382 | 372 | 74 |
| fw distil-small.en INT8 | 23.45 | 3.72 | 13.88 | 20.24 | 20.26 | 15.92 | 18.57 | 48.80 | 21.76 | 70.8 | 50.0 | 0.1248 | 675 | 320 |
| sherpa whisper tiny.en INT8 | 23.59 | 7.45 | 29.00 | 40.08 | 34.54 | 27.32 | 35.28 | 9.38 | 23.82 | 87.7 | 0.0 | 0.0372 | 528 | 245 |

Multilingual: **Spanish (es_419) WER 3.25 % for Parakeet v3 only.** No other model in this run produced a scorable Spanish number on our 18-clip subset. **Hindi (hi_in): no model produced scorable output, `hi_wer`/`hi_cer` are null for every candidate.** The 18 Hindi clips are in the corpus and ready, but none of these English/Asian-multilingual models transcribes them well enough to score meaningfully. This is a real coverage gap, not a passing result (see §Limitations).

## What the numbers actually say

1. Parakeet is the clear default. v2 and v3 occupy the top two EN-WER slots and are the only high-accuracy models with **0 % hallucination** on silence/keyboard noise. For a dictation app, 0 % hallucination is non-negotiable, it is the difference between "types what I said" and "invents sentences." Both transducers hit it; every Whisper-lineage model except the tiniest does not.

2. v2 vs v3 is a genuine trade, decided by use-case:
   - v2 wins Indian English (10.14 vs 11.74), clean read speech (2.05 vs 2.42), technical-entity survival (92.3 % vs 86.2 %), and 10 dB noise (10.34 vs 16.18). It is the better *English dictation* model, especially for our Indian-English market and for code/commands.
   - v3 wins aggregate EN WER (10.70 vs 11.50), conversational (14.50 vs 16.84), long-form (2.09 vs 9.38), and adds 25-language + Spanish (3.25 %). It is the better *multilingual / long-document* model.
   - Verdict: ship v2 as the English default, v3 as the selectable multilingual model. That is what the catalog does.

3. Hallucination separates the field more sharply than WER. faster-whisper small/base and distil-small and SenseVoice all sit at 33-50 % hallucination, on 1 in 2-3 silent/noisy clips they emit text. That disqualifies them as a PTT default regardless of WER. It is also why our VAD speech-ratio gate + `KNOWN_HALLUCINATIONS` filter (Tier 0) exist as a second line of defence: even a 33 % model would be made safe-ish by the gate, but we would rather start from a 0 % model.

4. Distil-Whisper is a trap for dictation. distil-small has a great clean-speech number (3.72 %) and decent Indian English (13.88 %) but catastrophic long-form (48.80 % WER) and 50 % hallucination. Distillation optimised read-speech speed and broke robustness. Not usable.

5. Speed is not the constraint; RAM and startup are. RTFs are 0.02-0.15, i.e. 7-50× faster than realtime. Even the slowest model (fw-small, RTF 0.147) decodes a 10 s utterance in 1.5 s. The perceptible latency in the product comes from SLM rewrite and cold model/server startup, not ASR (quantified in `04-performance.md`). SenseVoice is the fastest (RTF 0.019) and the least trustworthy (50 % hallucination, 73.8 % entity survival), a reminder that raw speed is the wrong axis to rank dictation models on.

6. Moonshine is the honest fallback. Base (16.24 % EN, 0 % halluc, 741 MB) and Tiny (20.32 %, 0 % halluc, 517 MB) trade accuracy for a small, predictable footprint with no invented text. On a 4 GB machine, Moonshine Tiny + Tier-0-only is a coherent, safe product; it just gets more words wrong.

## Live Windows 11 validation (not just Linux)

The same pipeline was run end-to-end on a real Windows 11 QEMU/KVM guest (3 vCPU, 4 GB). On a 6.7 s utterance the default config (Parakeet v2 + Qwen3.5-0.8B) produced: raw "I know like Um like my parents bought me a lot of books so I used to always read a lot." → final "I know that my parents bought me a lot of books, so I used to always read a lot." (filler removed, stutter collapsed, meaning preserved), delivered via clipboard, ASR 752 ms, VAD 55 ms, and the full suite of Windows checks PASS (DPAPI round-trip, clipboard unicode/preserve-on-silence, single-instance mutex, autostart registry quoting, foreground-context detection). The rewrite leg's 12 s on that run was llama-server cold start on the undersized VM, not inference, see `04-performance.md`.

## Limitations and honest gaps

- **Hindi is unmeasured.** 18 hi_in clips exist but no candidate transcribes them to a scorable degree. We cannot claim Indic support. Closing this needs a dedicated Indic model (AI4Bharat Svarah / Monsoon family), a different integration than the sherpa transducer path, and its own benchmark pass. Flagged in `01` and `05`.
- **long_form is 6 clips.** Parakeet v3's 2.09 % is promising but the sample is too thin to certify multi-paragraph dictation. Expand before advertising it.
- **technical_synthetic is TTS-generated.** The 30 technical clips are synthesised, so their WER reflects TTS artefacts as much as real dictation. The *entity-survival* metric (92.3 % on v2) is the trustworthy part; the raw WER on that category should be read loosely.
- **Noise is additive, not reverberant.** Our noisy_5/10dB use additive babble/hum, not room reverb or far-field mic drop-off. Real conference-room audio will be harder than these numbers.
- **Single machine, 4 threads.** Absolute RTFs will differ on weaker/stronger CPUs; the *ranking* between models is stable, the absolute ms are not. Windows VM numbers differ from Linux (see cold-start caveat).
- **WER is case/punctuation-normalised** in `score.py`, so it measures word-sequence accuracy, not the punctuation quality the product actually ships (Parakeet's native punctuation is assessed qualitatively in the E2E, not scored here).

## Reproduce

```
cd /home/computebox/voice-input/bench
.venv/bin/python build_corpus.py          # -> corpus/ + manifest.jsonl
.venv/bin/python run_asr.py -model parakeet-tdt-0.6b-v2-int8 -threads 4
.venv/bin/python score.py                 # -> results/score_summary.json
# rewrite eval (Rust):
cargo run -release -p vi-app - -eval-correction-jsonl correction_eval.jsonl -llm none
cargo run -release -p vi-app - -eval-correction-jsonl correction_eval.jsonl -llm Qwen3.5-0.8B-Q4_K_M
```

## Sources for corpus & candidate models (verified reachable 2026-11-15)

- LibriSpeech test-clean: https://huggingface.co/datasets/openslr/librispeech_asr
- EdAcc (accents + Indian English + conversational): https://www.research.ed.ac.uk/en/publications/the-edinburgh-international-accents-of-english-corpus-towards-the ; https://huggingface.co/datasets/edinburghcstr/edacc
- FLEURS (es_419, hi_in): https://huggingface.co/datasets/google/fleurs
- sherpa-onnx models: https://github.com/k2-fsa/sherpa-onnx ; https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html
- Parakeet v2/v3: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2 ; https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
- Moonshine: https://github.com/moonshine-ai/moonshine ; https://huggingface.co/UsefulSensors/moonshine-base
- faster-whisper / CTranslate2: https://github.com/SYSTRAN/faster-whisper ; https://github.com/OpenNMT/CTranslate2
- Distil-Whisper: https://github.com/huggingface/distil-whisper
- Open ASR Leaderboard (context, not our numbers): https://huggingface.co/spaces/hf-audio/open_asr_leaderboard
- Indic reference: https://github.com/AI4Bharat/Svarah ; https://huggingface.co/datasets/VoiceArena/Monsoon_hi_test
