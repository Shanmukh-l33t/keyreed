# 05: Product, UX & Competitive Research

Status: market + UX record. Written 2026-11-15.
Competitor data is GitHub API snapshots pulled 2026-11-15 (stars/issues/license/last-push) plus issue-title mining. Where I cite a complaint, it is from a real issue thread in `bench`-adjacent research cache, not hearsay. Pricing is directional only, re-check the vendor page the day you quote it.

## Landscape (GitHub snapshot, 2026-11-15)

| Project | Stars | Issues | License | Position |
|-----|----|----|-----|-----|
| cjpais/Handy | 38,815 | 152 | MIT | The reference open-source dictation app; offline, extensible, cross-platform. homepage handy.computer |
| EpicenterHQ/epicenter (absorbed braden-w/whispering) | 4,839 |, | (repo license NOASSERT) | Local-first app suite; whispering migrated into it |
| thewh1teagle/vibe |, |, |, | Whisper transcription GUI (bulk audio, not PTT dictation) |
| chidiwilliams/buzz |, |, |, | Offline Whisper transcriber (audio files, not live dictation) |
| OpenWhispr/openwhispr |, |, |, | Voice-to-text, local (Parakeet/Whisper) + cloud BYOK, privacy-first |
| Beingpax/VoiceInk |, |, |, | "Best open-source alternative to Superwhisper & Wispr Flow", macOS only |
| amicalhq/amical |, |, |, | AI dictation, open source, local-first, "3× faster" |
| josiahsrc/voquill |, |, |, | Open dictation, voquill.com |
| zachlatta/freeflow |, |, |, | "Free & fast alternative to Wispr Flow" |
| savbell/whisper-writer |, |, |, | Small Whisper dictation app |
| ideasman42/nerd-dictation |, |, |, | Linux, hackable, VOSK, not Windows, legacy WER |
| mkiol/dsnote |, |, |, | Linux speech note app |
| kitlangton/Hex → anomalyco/hex | (archived) |, |, | Swift/macOS, archived/relocated |

Closed-source leaders (not on this list because they are proprietary): Wispr Flow, Superwhisper, Dragon/Motivity, macOS/Windows built-in dictation, and the big-vendor "voice typing" (Google, Microsoft). Their pricing clusters around $12-30/user/month for the cloud products; the local-first ones compete on "no subscription."

Key structural read: the open-source field is **macOS-heavy and audio-file-transcription-heavy**. Handy is the one cross-platform PTT dictation app with real momentum (38.8k stars). Windows-first, offline, PTT dictation with conservative LLM rewriting is a genuinely under-served niche, most Windows options are either cloud (Wispr Flow) or bulk-transcription (Vibe, Buzz), not live dictation into the focused field.

## What competitors do well

- Handy: truly offline, cross-platform, extensible plugin model, huge community. Sets the UX bar for "hold a key, talk, get text."
- VoiceInk / Superwhisper: polished per-app writing modes and dictionary support; strong "it adapts to where I'm typing."
- Wispr Flow: best-in-class cloud rewrite quality and formatting (emails, lists, code), the accuracy bar users now expect.
- OpenWhispr: local + BYOK cloud in one app, honest about privacy modes. Closest architectural cousin to ours.

## What users actually complain about (mined from issue trackers)

From Handy's issue list and comparable trackers, the recurring themes:
1. **It types into the wrong window / loses focus.** Alt-Tab or a popup mid-utterance and the text lands somewhere unexpected. (Our fix: capture foreground HWND at PTT-down, clipboard-only fallback if focus changed, `02-windows-engineering.md` §4.)
2. **Hallucinated text on silence / background noise.** The app invents sentences. (Our fix: speech-ratio VAD gate + `KNOWN_HALLUCINATION` filter + 0 %-hallucination default model, measured.)
3. **Numbers, names and code get mangled or dropped.** "Invoice for 450" becomes "four hundred fifty" or loses the figure. (Our fix: numeric/identifier survival guards, measured 100 % keep on the shipped path.)
4. **It answers a question I dictated.** "What's the ETA?" comes back as an answer. (Our fix: dictated-question guard + few-shot, measured.)
5. **Subscription fatigue / privacy anxiety.** The single biggest driver of people toward Handy/open-source is "I don't want my voice going to the cloud." (Our posture: offline by default, BYOK cloud only if opted in.)
6. **First-launch and dependency pain.** Windows users hit missing runtimes, model-download walls, AV/SmartScreen flags. (Our fix: bundled redistributable + guarded model extraction, `02` §8, `06`.)
7. **Latency feels frozen with no feedback.** No indication the app is working. (Our fix: explicit "Correcting" overlay state.)
8. **Doesn't survive UAC/admin windows, RDP, sleep/resume.** Power-user complaints. (Our fix: elevation detection + session/power re-hook, `02` §1, §4.)

## Gaps we can exploit

1. **Windows-first, offline, PTT with a conservative rewrite that is provably meaning-preserving.** Nobody ships an invariant-guarded rewrite; competitors rely on the prompt and occasionally eat your numbers. Our guard+fallback floor (100 % invariant even when the LLM fails) is a differentiator we can state in plain language: "It will never change your numbers, names, or code, and if the AI would, it doesn't use the AI."
2. **Indian English as a first-class target, not an afterthought.** Deterministic Tier-0 normalisation for Indian-English constructions + a benchmark that measures Indian-English WER (10.14 % on the default model). Competitors benchmark on US/UK read speech and quietly degrade on Indian accents. This is a real, defensible moat for the home-care/SaaS market the owner serves.
3. **Honest resource behaviour.** Idle-unload to ~7 MB, a clear RAM ladder (Tiny on 4 GB → Parakeet on 8 GB), and truthful cold-start messaging. Power users notice and reward this.
4. **Safety for destructive commands.** TerminalCode mode flags `rm -rf`, `git push -force`, `terraform destroy` for confirmation. No consumer dictation app does this; for developers and ops it is a trust feature.

## Recommended UX (what we ship)

- One primary gesture: hold PTT, speak, release. Modifier-chord configurable, install/uninstall on toggle, never hijack the Start Menu.
- A status pill that always tells the truth: Listening → Correcting → Delivered. The "Correcting" state is load-bearing because the SLM takes 300-800 ms.
- Per-app writing modes auto-selected by foreground context (email/chat/code/notes), overridable by a cycle hotkey.
- An "Exact" escape hatch always one keystroke away, when in doubt the user wants verbatim, and we honour that with zero LLM.
- Confirmation gate before pasting a detected-destructive command.
- Privacy at the centre of the story, not buried: offline default, a visible indicator when audio would leave the machine, and clipboard-propagation exclusions on MaxPrivacy.
- Failures never destroy user state: empty/failed dictation leaves the clipboard and the target field untouched.

## Product name candidates (conflict-checked, 2026-11-15)

The codebase currently brands as **"Keyreed"** (PRODUCT_NAME in `vi-core/src/brand.rs`, mutex `Local\Keyreed...`, clipboard boundary `KeyreedAudioBoundary`). Treat Keyreed as the working name. Alternatives, each checked against the obvious collision (an existing dictation/voice product or a crowded GitHub org):

1. **Keyreed** (current), distinctive, "key + reed (as in a reed instrument/voice)". No direct voice-product collision found. Domain keyreed.dev is referenced in our own support strings, implying it's ours. Low risk.
2. **Dicta** / **Dictamen**, clean but crowded (many "Dicta"-branded STT tools). Medium-high collision risk.
3. **Quillvoice**, evokes writing+speech; check trademark, moderate risk.
4. **Reedwrite**, combines the reed motif with output; low collision, slightly clunky.
5. **Talkscribe**, descriptive; "Scribe" is heavily used in AI-note products (high collision).
6. **Utterly**, playful "utter"+typing; low tech collision, brandable.
7. **Saytype**, blunt and memorable; low collision.
8. **Voquill**, taken (josiahsrc/voquill exists). Do not use.
9. **Flowtype**, collides conceptually with Wispr *Flow*; avoid, invites confusion/UDRP.
10. **Parlour** / **Parley**, speech-adjacent, consumer-friendly; medium collision.

Recommendation: keep **Keyreed**. It is already wired through the codebase (mutex, clipboard boundary, brand constants), it is distinctive, and I found no voice-product collision in this pass. Before launch, run a proper trademark search (USPTO/EUIPO/IPO India) and domain/app-store availability check, this pass is a quick collision scan, not legal clearance.

## Open questions

- Pricing: the market splits at "free/open-source" (Handy) vs "$12-30/mo cloud" (Wispr/Superwhisper). Our offline-first stance argues for a one-time or per-seat licence with optional cloud BYOK, not a voice-minute subscription. Needs a willingness-to-pay test in the Indian SMB/home-care segment specifically.
- Do we open-source any of it? Handy's 38.8k stars show the community-distribution flywheel is real, but our moats (Indian-English Tier-0, guard layer, context engine) are the differentiators, open-sourcing them erodes the edge. Likely: closed app, maybe an open benchmark/eval harness for credibility.

## Citations (verified reachable 2026-11-15)

- Handy: https://github.com/cjpais/Handy ; https://handy.computer ; issue miner e.g. https://github.com/cjpais/Handy/issues/261 , /issues/96
- Epicenter/whispering: https://github.com/EpicenterHQ/epicenter (whispering redirected)
- OpenWhispr: https://github.com/OpenWhispr/openwhispr
- VoiceInk: https://github.com/Beingpax/VoiceInk ; amical: https://github.com/amicalhq/amical ; freeflow: https://github.com/zachlatta/freeflow ; voquill: https://github.com/josiahsrc/voquill
- Vibe: https://github.com/thewh1teagle/vibe ; Buzz: https://github.com/chidiwilliams/buzz ; whisper-writer: https://github.com/savbell/whisper-writer
- nerd-dictation (Linux): https://github.com/ideasman42/nerd-dictation
- Closed-source comparators (pricing pages, re-check before quoting): Wispr Flow, Superwhisper, Motivity/Dragon.
