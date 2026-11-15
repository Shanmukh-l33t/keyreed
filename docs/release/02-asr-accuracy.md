# Speech recognition accuracy

This audit re-scored stored hypotheses with the same rules as `bench/score.py` (`ref`, English normalizer, jiwer). It did not re-run the models. Hypothesis file: `bench/results/parakeet-tdt-0.6b-v2-int8.json`, written 2026-11-15 12:55 UTC. Wavs on disk: 244. English scored clips for the default model: 208. Re-score matched `bench/results/score_summary.json` for every model on overall English WER, Indian English WER, and technical WER.

Platform for these WER numbers: Linux bench host, not the Windows exe. The Windows exe was checked on one clip, repeated. That clip is not a WER study.

## Corpus (244 wavs)

| Category | Clips | In English WER |
| --- | --- | --- |
| clean_english | 32 | yes |
| indian_english | 32 | yes |
| other_accents | 24 | yes |
| conversational | 24 | yes |
| noisy_10db | 24 | yes |
| noisy_5db | 24 | yes |
| long_form | 6 | yes |
| technical_synthetic | 30 | yes |
| multilingual_hi_in | 18 | no, unscored |
| multilingual_es_419 | 18 | no for v2 |
| non_speech_hallucination | 12 | hallucination rate only |

Sources include LibriSpeech test-clean, EDACC (Indian English, Hindi, Nigerian, Kenyan, Ghanaian, and others), and edge-tts synthetic voices. Noise is mixed at about 10 dB and 5 dB. This is not a matrix of physical microphones attached to Keyreed. Microphone hardware variety on the Windows capture path was not tested.

## Default model, re-scored this audit

`parakeet-tdt-0.6b-v2-int8`

| Slice | Clips | WER % |
| --- | --- | --- |
| clean_english | 32 | 2.05 |
| indian_english | 32 | 10.14 |
| other_accents | 24 | 17.23 |
| conversational | 24 | 16.84 |
| noisy_10db | 24 | 10.34 |
| noisy_5db | 24 | 17.77 |
| long_form | 6 | 9.38 |
| technical_synthetic | 30 | 13.24 |
| overall English | 208 | 11.50 |

Non-speech hallucination: 0.0% (empty hypothesis after normalize on all 12 non-speech clips, from the stored summary; this audit confirmed the English WER path, not a fresh hallucination recount).

Technical entity survival: 92.3% of 65 entities, case-insensitive substring of the hypothesis. That metric is generous. Example from the stored record: reference "kube-system" / spoken "kube system", hypothesis "Cube system". Reference "PostgreSQL", hypothesis "post-grey SQL". Substring survival does not mean the identifier survived.

`ref_written` is the punctuated written form (hyphenated `kube-system`). Scoring against `ref_written` instead of spoken `ref` raised technical WER from 13.24 to 17.42 on this model. The published 13.24 is the spoken reference. Do not quote 13.24 as written-form accuracy.

Hindi: `hi_wer` is null. No Hindi accuracy claim.

Spanish: v2 has no `es_wer`. v3 stored summary has `es_wer` 3.25. Not re-heard in this audit. Not a Windows result.

## Other models (stored summary, re-score matched)

Overall English WER, lowest first. Hallucination is percent of 12 non-speech clips with a non-empty normalized hypothesis.

| Model | EN WER | IndEN WER | Tech WER | Halluc % | Median RTF |
| --- | --- | --- | --- | --- | --- |
| parakeet-tdt-0.6b-v3-int8 | 10.70 | 11.74 | 15.29 | 0.0 | 0.0392 |
| parakeet-tdt-0.6b-v2-int8 | 11.50 | 10.14 | 13.24 | 0.0 | 0.0401 |
| fw-small.en-int8 | 12.62 | 15.48 | 10.59 | 33.3 | 0.1471 |
| sense-voice-small-int8 | 14.08 | 16.01 | 26.18 | 50.0 | 0.0194 |
| moonshine-base-en-int8 | 16.24 | 21.53 | 17.35 | 0.0 | 0.0311 |
| fw-base.en-int8 | 16.26 | 18.15 | 17.65 | 50.0 | 0.0636 |
| sherpa-whisper-base.en-int8 | 17.52 | 19.93 | 19.71 | 8.3 | 0.0568 |
| moonshine-tiny-en-int8 | 20.32 | 26.69 | 19.12 | 0.0 | 0.0214 |
| fw-tiny.en-int8 | 21.02 | 24.02 | 19.71 | 8.3 | 0.0382 |
| fw-distil-small.en-int8 | 23.45 | 13.88 | 21.76 | 50.0 | 0.1248 |
| sherpa-whisper-tiny.en-int8 | 23.59 | 29.00 | 23.82 | 0.0 | 0.0372 |

v2 stays the English default because Indian English and technical entity survival beat v3, and hallucination is 0%. v3 is the multilingual local option in the catalog. These comparisons are Linux bench numbers.

Numbers, dates, URLs, and IP addresses are not a separate ASR category in this corpus. They are in the correction set, which is text-only.

## Grammar correction (separate from ASR)

Set: `bench/correction_eval.jsonl`, 88 cases, 14 categories. Run 2026-11-15 13:35 UTC on Linux, not re-executed in this audit. File on disk was read.

| Rewriter | Cases | Must-keep % | Anti-hallucination % | Combined % | Median ms | P90 ms |
| --- | --- | --- | --- | --- | --- | --- |
| none (Tier 0 only) | 88 | 100.0 | 100.0 | 100.0 | 3.02 | 3.25 |
| Qwen3.5-0.8B-Q4_K_M | 88 | 100.0 | 100.0 | 100.0 | 324.91 | 813.43 |
| Qwen3-0.6B-Q4_K_M | 88 | 95.5 | 97.7 | 93.2 | 235.44 | 581.81 |

Must-keep and must-not-contain are invariant checks. They are not a human rating that meaning was preserved on every case. A pass means the required spans survived and the forbidden spans were absent. It does not prove no subtle meaning drift outside those spans.

Categories: indian_english_grammar 8, terminal_commands 8, dictated_questions_and_instructions 8, fillers_repetition 7, self_correction 7, technical_terms 7, conversational 6, numbers_dates 6, email_url 6, code_identifiers 6, long_paragraph 5, already_correct 5, multilingual_hinglish 5, lists 4.

Windows observation, one clip, this audit: smart mode removed fillers and added punctuation. Exact mode plus Tier 0 did not. That is behavior on one file, not an accuracy rate.

## What this does not support

- A Windows word error rate.
- Hindi accuracy.
- Physical microphone comparisons.
- A claim that correction never invents information, beyond the 88-case invariant file.
- URL and IP address ASR accuracy as a scored slice.
