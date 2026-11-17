# STT analysis and decisions

Labels: **[OBS]** read in code, docs or an API response; **[DOC]** the Hex authors' own measurement, not reproduced; **[INF]** inference; **[UNV]** not verified. No audio was transcribed, no benchmark was run and no WER was measured in this round. Every accuracy or speed statement below is either from a cited source or marked as a hypothesis for the owner's testing.

## 1. KeyReed before this round

Capture, 16 kHz mono, Silero VAD spans grouped and decoded offline with `greedy_search` while the user is still talking, tail group on release, Tier 0 text rules, optional local grammar pass. Parakeet TDT 0.6B v2 INT8 is the default (KeyReed's own corpus: 11.50 aggregate WER on a deliberately hard mixed set, 2.05 clean, real-time factor 0.04, 0% hallucination on non-speech; `research/07-asr-benchmark.md`). Details in `keyreed-baseline.md`.

## 2. What Hex does that matters, and what was adopted

| Topic | Hex [OBS unless noted] | KeyReed decision |
| --- | --- | --- |
| Where streaming happens | Parakeet Unified streams only on GPU: "a CPU stream re-encodes each overlapping window and would cost several times the offline compute" (`gguf_session.rs:244-247`) | Not adopted. KeyReed is CPU only. Its VAD-chunked offline decode already overlaps work with speaking |
| Finish after release | 200 ms tail, up to 600 ms while speech continues, then finalize; 1.5 s deadline then whole-clip fallback | Tail adopted (`vi-audio`). Whole-clip fallback not needed because KeyReed's last group is already an offline decode |
| Edge words | 450 ms pre-roll plus tail. Hex's replay study [DOC]: last word kept 10/10 at 0 to +200 ms, 7/10 at -100 ms, 2/10 at -200 ms | Tail yes. Pre-roll no: it needs an always-open microphone, which breaks KeyReed's "open only while the key is held" promise |
| Silence gate | Loudest 30 ms DC-removed window at -50 dBFS, or a run 12 dB over the clip floor; then Silero | Adopted in `vi-asr` (windowed meter, bounded memory). Old gate was the whole-clip mean RMS at -58 dBFS |
| Resampling | rubato FFT, Blackman-Harris 2, exact length | In-tree polyphase windowed-sinc (Blackman-Harris, 7.6 kHz cutoff, 16 zero crossings per side), same property that incremental equals one-shot; no new dependency. Plus a 20 Hz DC blocker |
| Vocabulary | Parakeet: rescoring of heard spans, boost per piece, 64 terms; Whisper and Qwen: text prompt; separate exact "sounds like" pass | KeyReed's route is sherpa-onnx hotwords (section 3). A UI for Term and Sounds like rows now maps to vocabulary and replacements |
| Minimum clip length | Every clip padded to 1.5 s | Chunks under 1.0 s are zero-padded for Parakeet-family models only. Whether sherpa needs it is [UNV]; it is cheap and was kept low-risk |
| Raw versus processed | Processing is opt-in Labs; a raw chord pastes the transcript | KeyReed keeps Exact mode as its raw path. Tier 0 was narrowed (section 5). A per-press raw chord was not added |

## 3. Hotwords: the one real defect found

[OBS] sherpa-onnx `OfflineRecognizerTransducerNeMoImpl` builds the hotword context graph and BPE encoder only inside its `modified_beam_search` branch (upstream `offline-recognizer-transducer-nemo-impl.h`). The branch was added in sherpa-onnx PR 3077 (2026-02-05). The official Parakeet v2 hotwords example sets `modified_beam_search`, `modeling_unit=bpe`, a `bpe_vocab` derived from `tokens.txt` (the release archive ships none), and `hotwords_score` 2.0.

[OBS] KeyReed 2.0.0-beta.1 sets `greedy_search`, no modeling unit, no BPE vocab, and passes uppercased terms (`lib.rs:438`, `dictionary.rs:75`). [INF, strong] so the biasing did nothing. This was not run.

Implemented: when the dictionary is enabled and non-empty and the model is a hotword-capable NeMo transducer, the recognizer is created with `modified_beam_search`, `max_active_paths` 4, `hotwords_score` 2.0, `modeling_unit=bpe` and a `bpe.vocab` generated from the pinned `tokens.txt` into `%LOCALAPPDATA%\Keyreed\cache` (outside the pinned model folder, so integrity checks never see it). Otherwise the greedy path is unchanged. Guards: the runtime version string must be at least 1.12.24 (an older library would call `exit(-1)` on the method), and the C struct field names and order in `ffi.rs` were compared with the upstream v2.0.1 header. The engine reloads in the background when the dictionary changes between empty and non-empty. Terms are passed in natural case; entries with `/`, `\`, `:`, `@`, newlines, control characters, domain-like words, or over 64 characters are dropped (Hex skips address and path-like terms for the same reason), capped at 64 terms.

Open questions for the owner's test: speed of beam search on the target CPU [UNV], whether TDT token timestamps (used for paragraph pauses) are still produced under beam search (if not, pauses fall back to VAD gaps) [UNV], false replacements on rhymes. Hex's own rejected-design notes warn about over-firing; the score of 2.0 is the upstream example's value, not a tuned one.

## 4. Model decision

Choice: keep Parakeet TDT 0.6B v2 INT8 as the default and add two selectable catalog entries. Nothing available as a ready sherpa INT8 archive clearly beats v2 on accuracy and non-speech safety together.

| Candidate | Size (archive) | Licence | Evidence | Decision |
| --- | --- | --- | --- | --- |
| Parakeet TDT 0.6B v2 (current) | 482,440,461 B | CC-BY-4.0 | NVIDIA 8-set average WER 6.05 [OBS, averaged from the model card]; KeyReed corpus above | Default, unchanged |
| Parakeet TDT 0.6B v3 | 487,211,443 B | CC-BY-4.0 | KeyReed corpus: better aggregate, worse Indian English (11.74), 10 dB noise (16.18), entities (86.2%) | Stays as the multilingual option |
| Parakeet Ultra 0.6B | 487,134,290 B, sha256 `d216b71d` | CC-BY-4.0 | Moondream's own comparison: 5.80 versus v3's 6.26 on 7 English sets; sherpa package README FLEURS-English 8.51 versus v3 11.16 [OBS, publisher numbers, different harnesses] | Added as selectable. Hypothesis to test: beats v3, maybe v2. Same decoder, joiner and tokens as v3, only the encoder differs [OBS from the extracted archive] |
| Parakeet TDT 110M (transducer) | 108,035,095 B, sha256 `f628312e` | CC-BY-4.0 | NVIDIA card 8-set average 7.50 [OBS, averaged from the card] versus Moonshine base 16.24 on KeyReed's harness (not comparable) | Added as the low-RAM tier. About 190 MB RAM (approximate) |
| Parakeet Unified EN streaming | 501 MB | NVIDIA Open Model License | NVIDIA 6.29 at 1.12 s latency; sherpa-onnx PR 4133 reports 14.8% WER without endpointing and 39% with it on FLEURS versus 8.1% for NeMo's loop, merge status [UNV] | Rejected: buffered CPU stream cost and runtime accuracy dispute |
| Nemotron Speech Streaming EN | 464 MB | NVIDIA Open Model License | 8-set average 6.94; greedy only in the published C++ | Rejected for now: needs online-recognizer FFI, loses on average WER, no hotwords |
| Whisper large-v3-turbo, Qwen3-ASR, Cohere Transcribe | 564 MB, 879 MB, 1.7 GB | MIT, Apache-2.0, Apache-2.0 | KeyReed's own corpus shows Whisper-family models hallucinate on 8 to 50% of non-speech clips; CPU latency of the others [UNV] | Not added |

Both new entries were verified, not trusted: the archives were downloaded and the archive SHA-256 matched the value published by the sherpa-onnx release API; every extracted file has its own pinned size and SHA-256 in `integrity.rs`, using the same compile-time pin mechanism as the existing models. Nothing is bundled; weights download from the sherpa-onnx release when selected. CC-BY-4.0 requires attribution, and the catalog carries the licence field shown in the app.

Not verified: `feature_dim` of the 110M export versus the hard-coded 80 in `LocalAsrEngine::load` (smoke test required), WER on KeyReed's corpus for either new model.

Suggested A/B for the owner, using the existing harness: v2 versus Ultra versus v3 on `bench/` clips with and without a vocabulary list; watch Indian English, 10 dB noise, entity survival and hallucination on non-speech. Hex's acceptance gate for vocabulary biasing is a useful bar: at most +0.3 WER points and at most 0.5% false replacements on term-free clips.

## 5. Text processing: raw, deterministic, vocabulary, optional grammar, final

Layers kept apart as before: raw transcript (stored in History), Tier 0 deterministic cleanup, personal replacements and casing, optional local grammar model behind guards, final formatting. Changes:

- Fillers: a token-based remover keeps a following full stop, hands over the capital, no longer removes "hmm" and "ah" (they can mean something), and no longer eats "err".
- Stutters: only exactly doubled function words collapse ("the the", "I I"). "had had", "that that", "no no", "bye bye" survive. Exact and Terminal modes are untouched.
- Wording rewrites ("not able to find", "revert back", "having a doubt", "myself X") run only in Professional mode.
- "march" and "august" are no longer recased; weekday and other month names stay.
- Not changed: guard logic (the grammar model still cannot change numbers or identifiers), Exact mode guarantees, Terminal compilation. A shared rule table `RE_ESL_RULES` still rewrites some phrases in every mode and in Quick Fix; flagged, not changed.

## 6. Latency

What the code now does between key release and text, in order: tail capture 200 ms (up to 600 ms while speech continues; the coordinator thread waits), decode of the last VAD group, Tier 0, optional grammar model, typing. Net effect on end-of-speech latency is plus 200 ms by construction, in exchange for not losing the last word [DOC support from Hex's replay study; not measured on KeyReed]. Beam search may add decode time when a vocabulary exists [UNV]. No claim is made that KeyReed matches Hex's latency or accuracy; that needs the owner's side-by-side test.

## 7. Privacy

Unchanged: local by default, no network call for audio or text on the default profile, mic open only while the key is held (plus the 0.2 to 0.6 s tail), no telemetry added. The only new network use is the optional model download the user already triggers. The new log file never contains transcript text (existing log calls were checked; panic messages that could quote slices are withheld).
