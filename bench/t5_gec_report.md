# T5-small GEC int8 vs the shipped Quick Fix (measurement only)

Probe: `crates/vi-rewrite/examples/t5_gec_probe.rs` (measurement lane; nothing here is wired into the app).
Corpus: `bench/quickfix_gate.jsonl`, all 401 cases, scored with the repo's own `bench/score_quickfix.py`.
Machine: this box (12 logical cores, Windows 11). Build: `release`, `CARGO_TARGET_DIR=kr-targets/t5`.

## What was run

| row | artifact | revision | quant | how it was run |
|---|---|---|---|---|
| GECToR | `proofbird/gector-bert-en` v1.1.0, the installed `KeyReed/models/gector-bert-en-int8` | run `2026-11-06-0801` | fp32 ONNX (112 MB) | `Gector::load(dll, model, 2)` + `QuickFixEngine::fix`, i.e. the shipping path, 2 intra-op threads |
| T5 | `Xenova/grammar-synthesis-small` ONNX of `pszemraj/grammar-synthesis-small` | `573e5a3a5a75113ec8e1250ea29233923301deff` | int8 dynamic (`*_int8.onnx`) | `ort` 2.0.0-rc.15, greedy (no beam), encoder + decoder(step 0) + decoder_with_past |
| Qwen | `Qwen3.5-0.8B-Q4_K_M.gguf` (507.8 MiB) | local file | Q4_K_M | the app's own `LlamaServerSupervisor::rewrite`, `WritingMode::Smart`, same prompt as the fallback |
| Harper | `harper-core` 3.2.0 | — | — | **owner's own measurement, not re-run here** (see note) |

T5 files used: `onnx/encoder_model_int8.onnx` 35,759,168 B, `onnx/decoder_model_int8.onnx` 58,943,404 B,
`onnx/decoder_with_past_model_int8.onnx` 55,729,398 B → 143.5 MiB of weights, 146.5 MiB with `tokenizer.json` + `spiece.model`.

What did **not** work: `onnx/decoder_model_merged_int8.onnx` (the single-file export) fails inside ONNX Runtime on
step 0 with `If node 'optimum::if' … FusedMatMul right operand cannot broadcast on dim 0`, so the probe uses the
three-file export instead. No Python ML stack on this box, so no conversion was attempted.

## Numbers (same corpus, same scorer)

| engine | fix rate, must-fix classes (G3 ≥70% target) | false edits on already-correct (G1 ≤0.3%) | clean sentences left untouched | p50 / p95 ms per sentence | model size on disk | weights licence | training data licence |
|---|---|---|---|---|---|---|---|
| GECToR (shipped) | **34.7%** (51/147) | **0/130 (0.00%)** | **151/151** | **37 / 98** (max 165) | 112.4 MiB | Apache-2.0 | wikimedia/wikipedia (CC BY-SA) |
| T5-small int8 | 19.0% (28/147) | 104/130 (80.0%) — 44/64 everyday, 60/66 technical | 27/151 | 254 / 516 (max 898) | 146.5 MiB | **CC-BY-NC-SA-4.0** | **JFLEG (CC-BY-NC-SA-4.0)** |
| Qwen 0.8B (fallback) | 64.6% (95/147) | 15/130 (11.5%) | 131/151 | 621 / 1674 | 507.8 MiB | Apache-2.0 | n/a (pretrained) |
| Harper 3.2.0 *(owner's run, not mine)* | n/a on this gate | n/a on this gate | 1/1 on his sample | 3–7 (after ~60 ms init) | +28 MB of binary | Apache-2.0 (crate metadata) | n/a (rules + curated dictionary) |

Worst-edit rate (G2, error sentences that gained a wrong edit): GECToR 2.4% (6/250), T5 75.6% (189/250), Qwen 30.8% (77/250).
Protected tokens (G5): GECToR 0/74 destroyed, T5 62/74, Qwen 4/74. T5 kept `must_keep` in 342/401 cases, GECToR 401/401.

Text T5 changed that was already correct (verbatim from `qf_t5`):

- `Attached is the revised schedule, and I have highlighted the changes in yellow.` → `As mentioned, the schedule is changed, …`
- `Best regards, Ananya` → `Thanks for all the great advice.`
- `I will be out of the office from Monday and back on the 14th.` → `… from Monday to Thursday.` (the date is gone)
- `Hi Priya, …` → `Hi Piya, …`

T5 also produced no output at all for one case (`pr_37`, left unchanged and counted as a miss).
Per class it is behind GECToR on every counted one (agreement 37% vs 67%, doubled_word 35% vs 40%, confusable 19%
vs 44%, capitalisation 8% vs 38%): it paraphrases the sentence instead of making the local edit.

## Verdict

1. **No.** T5-small as published is worse on every gate that matters: 19% vs 35% fix rate, it rewrites 80% of already-correct sentences (GECToR: 0%), and it destroys 62 of 74 protected tokens; latency is 254/516 ms vs 37/98 ms.
2. Size is not the ~80 MB guess either: the int8 set is 146.5 MiB against GECToR's 112.4 MiB, so the real cost of this swap is ~+34 MiB of model plus a much slower path — and it would be a quality regression.
3. **Licence forbids shipping anyway**: weights are CC-BY-NC-SA-4.0 and the training set is JFLEG, also CC-BY-NC-SA-4.0 — non-commercial and share-alike, so it cannot be bundled in a commercial (or Apache-2.0) app. Its accuracy is reported above only as a data point.

## Reproduce

```
cargo build --release -p vi-rewrite --example t5_gec_probe
t5_gec_probe t5     <dir with onnx/ + tokenizer.json> bench/quickfix_gate.jsonl out_t5.jsonl
t5_gec_probe gector <KeyReed/models/gector-bert-en-int8> bench/quickfix_gate.jsonl out_gector.jsonl
t5_gec_probe qwen   <Qwen3.5-0.8B-Q4_K_M.gguf>          bench/quickfix_gate.jsonl out_qwen.jsonl
python3 bench/score_quickfix.py t5=out_t5.jsonl gector=out_gector.jsonl qwen=out_qwen.jsonl
```

Caveats: T5 tokenisation here is greedy longest-match over the Unigram vocab with the `▁` prefix and no NFKC
charsmap (exact for the ASCII gate, needs the real tokenizer for other scripts); T5 ran at ORT's default thread
count (12 cores) while GECToR ran at the app's 2, so T5's latency is its best case. Harper's row is cited from the
owner's run on six sentences — it was not scored on this gate.