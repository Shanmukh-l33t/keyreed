# Keyreed research & engineering docs

Voice-input (push-to-talk dictation) for Windows 10/11, offline-first, with conservative AI rewriting.
Written 2026-11-15. All performance/accuracy numbers are measured on our own corpus and a real Windows 11 VM, not quoted from vendors.

| Doc | Topic |
|---|----|
| 01-speech-recognition.md | ASR model selection + ranked shortlist (default, fallback, multilingual, cloud) |
| 02-windows-engineering.md | Win32 implementation: keyboard hook, WASAPI, clipboard/SendInput delivery, overlay, pitfalls |
| 03-ai-correction-context.md | 3-tier rewrite architecture, prompts, meaning-preservation guards, eval results |
| 04-performance.md | Measured latency/RTF/RAM, cold-start analysis, realistic targets, tuning decisions |
| 05-product-ux-competitive.md | Competitor landscape, user complaints, exploitable gaps, recommended UX, name candidates |
| 06-security-privacy.md | Threat model (T1-T14), implementable mitigations, prioritised testable checklist |
| 07-asr-benchmark.md | Corpus + harness, full measured results, honest limitations |

Reproduce the measurements: `bench/` (corpus, run_asr.py, score.py, correction eval). Implementation lives in `crates/` (vi-core, vi-asr, vi-rewrite, vi-audio, vi-win32, vi-app).

Top outstanding security gaps (see 06): model-archive SHA-256 pinning (T9) and Authenticode code-signing (T12).
