# 01: Speech Recognition (ASR) Selection for Keyreed

Status: decision record. Written 2026-11-15.
Authoritative numbers in this doc come from our own benchmark (`bench/`, see `07-asr-benchmark.md`), not from vendor claims. Landscape citations are listed at the end and were verified reachable.

## Product constraint recap

Keyreed is a Windows push-to-talk dictation app that must run on ordinary PCs, offline by default, with a sub-second "release button → text on screen" feel for a typical 5-15 s utterance. That pins the ASR choice to:

- CPU-only inference (no GPU requirement; the app must work on a locked-down office laptop).
- Non-streaming, per-utterance decoding (PTT gives us a clean audio boundary; we do not need continuous streaming and the extra complexity it brings).
- Native punctuation and casing, because dictation output goes straight into emails, chats and documents.
- Robustness to Indian English and to noisy home-office audio, both are core to our market, and both are where cheap models fall over.
- A licence that permits commercial redistribution of the weights inside our installer.

## Recommended shortlist

### (a) Local CPU-only default: NVIDIA Parakeet TDT 0.6B v2 (INT8, sherpa-onnx)

Measured on our corpus (208 English clips, 4 threads): overall EN WER 11.50 %, clean read-speech 2.05 %, Indian English 10.14 %, noisy-10 dB 10.34 %, technical-entity recall 92.3 %, non-speech hallucination 0 %. Median RTF 0.0401 → a 10 s utterance decodes in ~401 ms on 4 threads. Peak RSS 1.77 GB, download 631 MB.

Why v2 over v3 as the default: v2 has the best technical-entity survival (92.3 %) and the best clean/read-speech WER of anything we tested, it is the most noise-robust of the transducers at 10 dB, and it is the model we have validated end-to-end on the real Windows 11 VM. v3 wins aggregate EN WER (10.70 %) and adds 25-language coverage plus Spanish (ES WER 3.25 %), but it costs ~70 MB more RAM and drops Indian-English and noisy performance slightly. Ship v2 as the English default; make v3 the selectable "multilingual" model.

TDT (token-duration-transducer) decoding is also why latency is predictable: it emits tokens with durations rather than running a second CTC pass, so there is no beam-search blow-up on long utterances.

### (b) Tiny fallback for weak/old hardware: Moonshine Tiny (INT8) → Moonshine Base (INT8)

Moonshine Tiny: EN WER 20.32 %, but 0 % hallucination, RTF 0.0214 (fastest usable model we have), peak RSS 517 MB, download only 119 MB. This is the model for a 4 GB-RAM laptop or an older dual-core. It degrades gracefully: it gets words wrong but it does not invent sentences, which is the failure mode users actually notice.

Moonshine Base is the better fallback if the machine has ~1 GB to spare: EN WER 16.24 %, clean 3.54 %, 0 % hallucination, RTF 0.0311, 741 MB peak, 274 MB download. Recommendation: auto-select Tiny below 6 GB system RAM, Base above that, Parakeet above 8 GB.

We rejected faster-whisper tiny.en as the fallback despite a similar footprint: it hallucinated on 8.3 % of our non-speech clips (silence/keyboard-only) and its Indian-English WER (24.02 %) is worse than Moonshine's at the same speed class. Hallucination on silence is disqualifying for a dictation tool, it types words the user never said.

### (c) Multilingual local: Parakeet v3 (European languages) + SenseVoice Small (zh/ja/ko/yue), with a caveat

Parakeet v3 covers 25 European languages with auto-detect and scored ES-419 WER 3.25 % on our FLEURS-derived Spanish subset. For an India-facing product the Hindi path matters more, and here the honest result is: none of our English-optimised models handle Hindi well, and SenseVoice, the multilingual model with the widest Asian coverage, is unsafe for dictation. It posted 50 % hallucination on non-speech and only 73.8 % technical-entity recall, the worst of the field, despite being the fastest model tested (RTF 0.0194). Fast and wrong is worse than slow and right for dictation.

Recommendation: ship Parakeet v3 as the multilingual option today (European + Spanish). Treat Hindi/SenseVoice as a research item, not a shipping default, see §Open questions. If we need Indic languages at production quality, the right tool is a dedicated Indic model (AI4Bharat Svarah / a Monsoon-family checkpoint), which is a different integration than the sherpa-onnx transducer path and needs its own benchmark pass before we promise it.

### (d) Optional cloud STT for maximum accuracy: OpenAI `gpt-4o-transcribe` / Groq Whisper, BYOK, gated off in MaxPrivacyOffline

Our `vi-asr/src/cloud.rs` already implements an OpenAI/Groq-compatible multipart client that builds the WAV in memory (zeroize-backed, never touches disk) and refuses non-HTTPS endpoints. The cloud tier is only reachable when the processing profile allows network AND the user supplied their own key. That design is correct; keep it.

Which provider: for pure accuracy on clean-ish dictation audio the 2026 frontier cloud STT (OpenAI's current transcribe line, Groq's Whisper-large-v3-turbo at very low $/token and high tokens/s) both beat any local 0.6B model by roughly 2-4 WER points on read speech. The gap is smallest on Indian English and noise, which is exactly where local Parakeet is already competitive, so cloud is a "maximum accuracy on good audio for users who opted in" feature, not a correctness requirement. We deliberately do not depend on it.

Do not put specific $/M-token figures in customer-facing material without re-checking the provider pricing page the day of publication; those numbers move monthly. The pricing pages we pulled (see citations) were captured 2026-11-15 and are not guaranteed current.

## Rejected candidates and why

- whisper.cpp / faster-whisper small.en as default: RTF 0.1471 (3.7× slower than Parakeet) and 33 % non-speech hallucination. Worse on every axis that matters for PTT.
- Distil-Whisper variants: our fw-distil-small.en measured 23.45 % EN WER and 50 % hallucination, distillation helped read-speech latency but hurt robustness on our accents/noise mix. Not competitive for dictation.
- Vosk / nerd-dictation lineage: legacy, much higher WER, no punctuation. Only relevant as a compatibility story for Linux (nerd-dictation), not our Windows target.
- Whisper large-v3-turbo local: quality is excellent but ~1.5 GB+ INT8 and far above our RAM ceiling for "ordinary PCs"; belongs in the cloud tier, not the local default.

## Integration notes (match the shipped code)

- Runtime is sherpa-onnx via the C API, loaded dynamically (`vi-asr/src/ffi.rs`), so onnxruntime.dll and sherpa-onnx-c-api.dll are shipped beside the exe and located through `AddDllDirectory`, never through PATH/CWD (see `06-security-privacy.md`).
- Model archives are `.tar.bz2` from the k2-fsa GitHub release CDN, extracted with path-traversal/symlink guards (`catalog.rs::validate_safe_relative_path`), this is a deliberate mitigation for the ONNX/tar CVE family.
- Hotwords: Parakeet v2/v3 support sherpa-onnx hotword biasing (`supports_hotwords: true` in the catalog); Moonshine and Whisper do not. Our personal dictionary maps to hotwords for the transducer models and to a post-rewrite substitution for the rest. This asymmetry is why the dictionary feature is stronger on the default model.
- Punctuation: Parakeet emits punctuation natively; the Tier-0 rewrite layer must therefore be punctuation-aware and not double-punctuate (handled in `tier0.rs`).

## Ranked shortlist (final)

| Rank | Model | Role | EN WER | IndEN | Tech-ent | Halluc | 10 s latency | Peak RSS |
|---|----|---|----|----|-----|----|-------|-----|
| 1 | Parakeet TDT 0.6B v2 INT8 | Default EN | 11.50 | 10.14 | 92.3 | 0 % | 401 ms | 1770 MB |
| 2 | Parakeet TDT 0.6B v3 INT8 | Multilingual | 10.70 | 11.74 | 86.2 | 0 % | 392 ms | 1799 MB |
| 3 | Moonshine Base INT8 | Fallback (mid) | 16.24 | 21.53 | 84.6 | 0 % | 311 ms | 741 MB |
| 4 | Moonshine Tiny INT8 | Fallback (low) | 20.32 | 26.69 | 84.6 | 0 % | 214 ms | 517 MB |
| 5 | Cloud gpt-4o-transcribe / Groq Whisper | Opt-in accuracy |, |, |, |, | network-bound |, |

SenseVoice Small is parked behind a flag for Asian-language research only; it is not in the ranked shipping list because of its hallucination rate.

## Open questions

1. Indic languages. We have no production-quality Hindi local model yet. Needs a Svarah/Monsoon-class checkpoint integrated and benchmarked on our hi_in subset before we advertise it.
2. Long-form drift. Our long_form category is only 6 clips (LibriSpeech concatenated). Parakeet v3 scored 2.09 % there but the sample is too thin to trust, expand before we claim "handles multi-paragraph dictation".
3. Streaming. If we ever move from PTT to continuous mode, Parakeet's non-streaming transducer is the wrong shape and we would re-evaluate nemotron-streaming / moonshine-streaming (both in the citation list).

## Citations (verified reachable 2026-11-15)

- sherpa-onnx runtime & models: https://github.com/k2-fsa/sherpa-onnx ; https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html ; hotwords: https://k2-fsa.github.io/sherpa/onnx/hotwords/index.html
- Parakeet: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2 ; https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3 ; NeMo Speech https://github.com/NVIDIA-NeMo/Speech ; NVIDIA Open Model License https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license
- Moonshine: https://github.com/moonshine-ai/moonshine ; https://huggingface.co/UsefulSensors/moonshine-base ; https://huggingface.co/UsefulSensors/moonshine-tiny
- Whisper lineage: https://github.com/SYSTRAN/faster-whisper ; https://github.com/ggml-org/whisper.cpp ; https://huggingface.co/openai/whisper-large-v3-turbo ; distil: https://github.com/huggingface/distil-whisper
- SenseVoice / FunASR: via sherpa-onnx model index (above)
- Leaderboards (context only, not our numbers): https://huggingface.co/spaces/hf-audio/open_asr_leaderboard ; https://artificialanalysis.ai/speech-to-text/non-streaming
- Indian English / Indic: https://github.com/AI4Bharat/Svarah ; https://huggingface.co/datasets/VoiceArena/Monsoon_en_IN_test ; EdAcc https://www.research.ed.ac.uk/en/publications/the-edinburgh-international-accents-of-english-corpus-towards-the
- Cloud STT pricing pages (re-check before quoting): https://artificialanalysis.ai/providers/groq ; https://developers.openai.com/api/docs/pricing.md
