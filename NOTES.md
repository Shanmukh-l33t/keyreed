# KeyReed 2.2.0 — fast where the time actually went

Dictation was taking 4–6 seconds between letting go of the key and seeing text. Measured on this
machine, most of that was not transcription, and none of it was the polish step.

## What was wrong

- **The speech model was being thrown away.** After 30 minutes idle KeyReed released the recogniser,
  so the next dictation loaded it again *inside* the dictation: `model_load_ms` 8.0 s typical and
  13.2 s worst, in 5 of 7 real dictations. A dictation with the model already loaded took 0.711 s.
- **Nothing was transcribed until you let go.** A chunk only closed at a pause of 550 ms or more,
  longer than a breath inside a sentence, so continuous speech decoded in one blob after release.
  On 44 real clips only 34 of 82 chunks had decoded before key-up.
- **The cleanup step could stall for 12 s.** A single real dictation waited 12.01 s for the optional
  local model and then returned the rule-based text anyway.
- **Inserting the text cost 0.5–1.1 s.** Per stage: foreground restore 1–4 ms, clipboard 0.3–1.4 ms,
  modifier drain 0.7–0.9 ms, and **508–1,051 ms of simulated keystrokes** — 62 characters meant 124
  key events, which is 99% of `output_ms`.

## What changed

- The recogniser stays resident; it is only released below 1 GiB of free memory. The cleanup model
  still unloads on idle as before.
- A 250 ms pause closes a chunk and decodes it while you are still talking.
- A warm cleanup call is bounded at 4 s on a GPU and 8 s on a CPU; the first call after the model
  starts keeps its longer allowance for shader compilation.
- The transcript is pasted with Ctrl+V over the clipboard copy KeyReed already writes, so it lands in
  one edit instead of growing character by character. Terminals and any transcript containing a
  newline keep the typed path; a failed clipboard write falls back to typing.

## Measured, 44 real recordings (read English + Indian, British and US accents)

| | 2.1.1 | 2.2.0 |
|---|---:|---:|
| Release → transcript, p50 / p95 | 1,009 / 4,171 ms | **523 / 1,038 ms** |
| Release → text ready, p50 / p95 | 997 / 4,178 ms | **505 / 1,041 ms** |
| Insertion (`output_ms`) | 508–1,051 ms | **15–21 ms** |
| Word error rate (1,179 words) | 10.18% | **10.18%** |

42 of the 44 transcripts are byte-identical to 2.1.1; the two that differ swap a comma for a period.
The Parakeet 110M model was also measured — 4× faster to decode but 11.62% WER, i.e. +1.44 points —
and was not adopted.

Edge cases measured on a simulated microphone: 60 s monologue 2.39 s to text, 100 s clip 585 ms after
release, silence filtered in 1 ms with nothing inserted, taps under 0.8 s filtered and 0.9 s or more
transcribed.

## Known issue

Pressing the dictation key again within about 0.2 s of the previous release transcribes every clip
but delivers only one to the app. Normal back-to-back dictation (a second or more apart) works.
Rapid-repeat presses are the next thing to fix.

## Verified / not verified

Verified: 373 workspace tests, the word-error-rate gate before and after, release-to-text measured
end to end through the real insertion path, and the edge cases above.

Not verified: a by-hand dictation pass into Notepad, a browser text box and a chat app; sleep/resume;
and a live-microphone pass on this exact build. The Windows build is unsigned.

## Install

`KeyReed-Setup-x64.exe` (24.4 MB) — SHA-256 `39ed2427e25f76715b8fe814deb289d3744dc574b51ca2cc9beb594e6655e613`

Portable: `keyreed-2.2.0-windows-x86_64-portable.zip` (46.2 MB) — SHA-256 `25ca5d85a6be560ca582c3c9a081a4c3e23ba2cbaf79f52025afd55146b5847f`

Verify with `SHA256SUMS.txt` before running.