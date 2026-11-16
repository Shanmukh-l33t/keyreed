# Market & Competitor Research: KeyReed

**Prepared:** 17 November 2026 · **Subject:** KeyReed v0.5.0 (open-source, Windows-only, local-first voice dictation)
**Method:** first-party pages, GitHub API, vendor pricing pages, Hacker News threads, secondary comparison blogs. Every number is dated; anything not confirmable on a primary source is marked `[unverified]`.

---

## 1. Summary

- **Wispr Flow is the category reference point and it is cloud-only by design.** Its own docs say "an internet connection is required for transcription" and "transcription always occurs on the cloud" — there is no on-device mode at any price.[13][11]
- **Its "Privacy Mode" does not mean local processing.** Wispr's data-controls page states the setting "was previously called 'Privacy Mode'" and only governs whether data is used for model training.[11] This is the single most exploitable misunderstanding in the market.
- **Pricing:** Free (2,000 words/week), Pro $15/user/mo monthly or $12 annual, Growth $23/$18 (adds SSO, enforced HIPAA with BAA), Enterprise custom.[2]
- **Platforms:** Mac, Windows, iPhone, Android.[1][12] Windows is x64; ARM/Snapdragon is reported unsupported by secondary sources `[unverified]` on Wispr's own requirements page.[26]
- **The open-source field is crowded but shallow on Windows.** Handy (MIT, 38,990★) owns "free + offline + cross-platform"; OpenWhispr (MIT, 11,694★) owns "local + cloud + meetings"; Amical (MIT, 1,622★) owns "app-aware formatting, free".[7][31][33] Voicetypr (AGPL, 746★) owns "polished Windows with AI cleanup at $69 lifetime".[34]
- **Nobody currently owns "offline by default *and* polished text *and* Windows-native"** — Handy is deliberately plain transcription, and the HN complaint about it is exactly that it "can't have it clean up your grammar mistakes".[23][25]
- **Demand is proven and recurring:** a 277-point HN thread for a free Wispr alternative, a dedicated "Anyone using a Wispr Flow alternative that is non-cloud?" thread, and repeated Reddit "is it worth $144/year" threads.[24][23]
- **Distribution, not features, is the binding constraint.** A comparable open-source Windows dictation app (VoxScribe, MIT) reported ~80 downloads and one paying user after a month of Reddit posting.[29] KeyReed's repo currently has 0 stars and is not indexed for any of the four target queries.[28]
- **KeyReed's defensible wedge:** 23 MB Win32-native installer, six local ASR engines on CPU via sherpa-onnx, six writing modes plus per-app adaptation, and clipboard fallback for password/elevated fields — a combination no competitor ships together.
- **KeyReed's honest gaps:** Windows x64 only, no mobile, no AI command mode, no team/admin/SSO, unsigned binaries, English-centric model set, and effectively zero community. Any positioning that implies parity with Wispr Flow will be contradicted on first install.

---

## 2. Wispr Flow deep dive

### 2.1 Verified facts

| Dimension | Fact | Source |
|---|---|---|
| Hero headline | "Don't type, *just speak.*" — sub: "The voice-to-text AI that turns speech into clear, polished writing in every app." | [1] |
| Platforms | Mac, Windows, iPhone, Android; desktop + mobile clients, web admin portal | [1][12] |
| Connectivity | "An internet connection is required for transcription" | [13] |
| Processing model | "Transcription always occurs on the cloud." No offline mode. | [11] |
| Privacy Mode | Setting renamed; legacy name for the model-training opt-out, **not** local processing. Training defaults off and locked on Growth/Enterprise. | [11][5] |
| Data path | Audio captured on device → TLS 1.2+ → transcribed → "Discarded"; AES-256 at rest; dictation cloud storage is a separate toggle | [5] |
| Certifications | SOC 2 Type II, ISO 27001; HIPAA with signed BAA on Growth and Enterprise (turns off Scratchpad/MCP) | [2][5] |
| Free tier | $0, 2,000 words/week, no card, no trial countdown; limited Notetaker | [1][2] |
| Pro | $15/user/mo monthly, $12 annual — unlimited dictation, unlimited Notetaker history, shared dictionary/snippets, team workspace, usage analytics | [2] |
| Growth | $23/user/mo monthly, $18 annual — SSO/SAML, org-wide HIPAA enforcement, multi-domain, admin model-training control | [2] |
| Enterprise | Custom, annual — SCIM, audit logs, MDM, domain capture, invoicing, SLAs | [2] |
| Features | 100+ languages, personal dictionary, snippets, Command Mode (edit selected text by voice), context awareness, app-aware tone | [2][11] |
| Claimed performance | "4x faster than typing" — keyboard 45 wpm vs Flow 220 wpm; "90% zero edit accuracy" | [1][3] |
| Social proof | Logo wall ("Used by professionals at"), named-company case study (Clay: 20% more customer calls/day, $3.08m/yr estimated savings), press quotes (WSJ, Fast Company), investor quote | [1][4] |
| Company signal | Homepage banner: "Wispr raises $81M to build the Voice OS" | [1] |
| Restriction worth noting | Flow "does not insert into password or credit-card fields" and does not appear in banking apps or password managers | [12] |

### 2.2 Marketing psychology (principles, not copy)

1. **Negate the verb, not the competitor.** "Don't type" attacks the behaviour rather than a rival product, so the ad stays true even after the user picks a competitor. It reframes the category from "dictation software" (a 1990s chore) to "how writing happens now".
2. **Make speed concrete and asymmetric.** 45 vs 220 wpm is a comparison a reader can verify against their own typing, which converts an abstract AI claim into a personal calculation.
3. **Kill the "only works in some apps" objection in the first screen** — "in every app" appears in the hero sub-line, before any feature detail.
4. **Name the prior failure before the user does.** "You've tried dictation. This is *different*" pre-empts the memory of Windows Speech Recognition and turns scepticism into a reason to keep reading.
5. **Pre-empt the technical dismissal.** A dedicated FAQ answers "Is this just a wrapper around OpenAI's Whisper?" — the exact objection a developer audience raises.[3]
6. **Trust is a hero-level asset, not a footer link.** SOC 2 II / HIPAA / ISO 27001 sit on the homepage next to "your data is never sold".[1] Compliance is used as a *desire* driver, not just a checkbox.
7. **Proof ladder: authority → peers → money.** Press quotes establish credibility, named testimonials establish relatability, and the Clay case study converts to a dollar figure ($3.08m/yr).[1][4] Each rung answers a different level of doubt.
8. **Loss aversion via calculator.** "22 hours saved per month" and "$13,056 estimated annual savings" against a $12 cost makes the subscription feel like a rounding error.[2]
9. **Free tier as a habit-formation device.** 2,000 words/week is enough for a first week of real work and not enough to finish one — the paywall arrives exactly when the behaviour is established.[1][26]
10. **The mobile apps are a moat, not a feature.** One subscription across four platforms raises switching cost and makes "just use a free desktop tool" feel like a downgrade.
11. **Tier laddering sells compliance, not features.** Pro → Growth (SSO, HIPAA BAA) → Enterprise (SCIM, audit, MDM) means each upgrade is triggered by an org requirement, which is far more durable than feature envy.[2]
12. **Zero-friction entry.** "No credit card required", "no trial countdown", one-click download, a browser demo, and a mobile handoff link — the first session costs nothing but a permission prompt.[1][3]
13. **Bundle a second habit.** Notetaker is included free on every plan, creating a second daily use case and doubling the cost of leaving.[1][2]
14. **Soften urgency.** The only deadline on the site is a freebie expiry ("Full access through November 30"), which creates motion without the used-car feel of a purchase countdown.[4]
15. **Answer in the user's words.** The FAQ is written as questions people actually type ("Is Flow free?", "Does Flow work in other languages?"), which also functions as SEO surface for long-tail queries.[1]

**What Wispr avoids saying:** it never claims local processing, and it never claims a privacy advantage over offline tools — because it cannot. Its privacy story is custody ("we never sell", "you choose what we keep"), not architecture. That is the seam KeyReed can open.

---

## 3. Competitor matrix

Star counts pulled from the GitHub REST API on **17 November 2026**; treat them as a point-in-time snapshot, not a trend.[7][31][32][33][34][35][36][17][19][30]

| Tool | Platforms | Licence / price | Local vs cloud | Stars | Main pitch |
|---|---|---|---|---|---|
| **Wispr Flow** | Mac, Win, iOS, Android | Proprietary; Free 2k words/wk, Pro $15/mo ($12 annual) | Cloud only[11][13] | — | "Don't type, just speak" — polished text everywhere[1] |
| **Handy** | Win, macOS, Linux | MIT; free[6][7] | Local only[15] | 38,990 | Free, open-source, offline push-to-talk[6] |
| **OpenWhispr** | Win, macOS, Linux, iOS | MIT; local free, cloud from $6.67/user/mo[14] | Local + BYOK cloud[8] | 11,694 | "Voice-to-text that makes your security team happy"[8] |
| **VoiceInk** | macOS only | GPL-3.0; $25–$49 one-time[9][16] | Local (+optional cloud enhancement)[9] | 7,142 | Mac-native, "buy once, own forever"[9] |
| **Superwhisper** | Mac, Win, iOS, Android | Proprietary; Free, Pro $8.49/mo, $84.99/yr, $249.99 lifetime[10] | Local Whisper + cloud models[10] | — | "Just speak. Write faster."[10] |
| **Voicetypr** | Win 10+, macOS 14+ | AGPL-3.0; $69 lifetime / 2 devices[21] | Local default + optional cloud[21] | 746 | Offline dictation with AI formatting, pay once[21] |
| **Amical** | Win, macOS, Linux | MIT; free[22][25] | Local (+cloud option)[22] | 1,622 | "Type 4x faster" with app-aware formatting[22] |
| **Whispering (fork)** | Win, macOS, Linux | MIT; free[19] | Local Whisper/Parakeet + cloud providers[19] | 1 | Community continuation of the abandoned 7.x dictation app[19] |
| **VoiceGecko** | Windows (Tauri)[17] | MIT; free[17] | Local[17] | 35 | "Open source, local voice-to-text"[17] |
| **WritHer** | Windows 10/11 | MIT; free[18] | Local (faster-whisper + Ollama)[18] | 94 | Offline dictation + symbols/code + local AI assistant[18] |
| **VoxScribe** | Windows, Android | MIT; free, $14 Pro[29] | Local[29] | 41 | Hold key, talk, text appears; file transcription[29] |
| **Windows Win+H** | Windows 10/11 | Bundled | **Cloud** — Azure Speech[20] | — | Built-in voice typing, zero install[20] |
| **KeyReed** | Windows 10/11 x64 | MIT OR Apache-2.0; free[28] | Local by default, optional BYOK cloud[28] | 0 | Local dictation with six writing modes[28] |

### Per-competitor notes

**Handy — the incumbent to beat, and the honest benchmark.** 38,990 stars, MIT, Rust + Tauri, cross-platform (Win/macOS/Linux), installed via winget and Homebrew, actively released (v0.10.1 on 14 Nov 2026).[7][25] Its pitch is deliberately narrow: "One tool, one job."[6] The gap is stated by its own users — an HN commenter says "I use Handy, it's fine, my complaint is that you can't have it clean up your grammar mistakes, it just records your words exactly as is".[23] A comparison site puts it more precisely: "It transcribes rather than rewrites … although an optional post-processing mode exists."[25] Handy also has no Windows Store presence and no signed installer story beyond minisign artifacts.[15]

**OpenWhispr — the closest thing to a funded OSS competitor.** 11,694 stars, MIT, claims "Downloaded by 150k+ users", backed by a16z Speedrun, with SOC 2 II / ISO 27001 / HIPAA / GDPR badges on the homepage.[8] Free tier includes *unlimited local* dictation; Pro ($6.67/user/mo annual) sells cloud transcription, agent mode, and device sync; Business $13.33.[14] Its weakness for KeyReed's audience: JavaScript/Electron-class footprint, an account-shaped product surface, and a pitch aimed at security teams rather than at the "I want it to stop sending my voice to a server" individual.

**VoiceInk — proof that "buy once" works, on the wrong OS.** GPL-3.0, 7,142 stars, macOS 14.4+ on Apple Silicon only, $25/$39/$49 one-time with lifetime updates, 200k+ downloads claimed.[9][16] Its testimonial wall is a pricing-attack archive: users describe migrating "wisperflow $15/mo > superwhisper $8.49/mo > voiceink ($25 one time)".[9] Nothing about this exists on Windows at the same polish level — that is Voicetypr's and KeyReed's opening.

**Superwhisper — the paid cross-platform benchmark.** Closed source, Mac/Win/iOS/Android, Free tier with unlimited local Whisper, Pro $8.49/mo or $249.99 lifetime, Enterprise with SOC 2 Type II.[10] It proves Windows users will pay for dictation, and its "Intel Macs work best with Cloud models" FAQ shows the same CPU-constraint problem KeyReed solves with Parakeet.[10]

**Voicetypr — the direct Windows threat.** AGPL-3.0, 746 stars, Rust, Windows 10+ and macOS 14+, $69 lifetime for two devices after a 3-day trial, local by default with optional Groq/Deepgram/OpenAI, AI formatting, CLI/Agent API, Microsoft Store listing.[21][25] This is the tool that most closely overlaps KeyReed's "offline + polished text + Windows" claim, and it is ahead on signed builds, Store distribution, and agent tooling.

**Amical — free, MIT, and format-aware.** 1,622 stars, Windows/macOS/Linux, "auto-formats your dictation for any app", free and open source.[22][25] Its community quotes show the appetite KeyReed is chasing: users switch from Aqua Voice and Wispr because "it lets me run everything locally so nothing leaves my machine".[22] It is the strongest free competitor on the formatting axis.

**Whispering — a cautionary tale about OSS dictation.** The original Show HN hit 591 points.[37] Upstream Epicenter then rebuilt it as a file-based voice-notes app "without global shortcuts, push-to-talk, voice-activated recording, auto-paste, or on-device transcription", and a community fork created 15 October 2026 now carries the 7.x behaviour with 1 star.[19] Lesson: an OSS dictation project can lose its users through a single product-direction decision, and a fork with one star does not recover them.

**VoiceGecko — effectively dormant.** MIT, 35 stars, one contributor, last commit 15 September 2026, Tauri/TypeScript with a bundled Whisper sidecar.[17] `voicegecko.io` did not resolve to a reachable product page from this network — the domain surfaced as parked/blocked, and the bare domain `voicegecko.com` is listed for sale.[17] Treat as inactive.

**WritHer — the closest feature analogue on Windows.** MIT, 94 stars, Python, faster-whisper + Ollama, Windows-only, free, offline, and it already does code/symbol dictation plus a local assistant.[18][36] It is KeyReed's most direct functional rival on "developer dictation", but its Python/Ollama stack implies a heavier install and setup than a 23 MB native binary.

**VoxScribe — the distribution reality check.** MIT, 41 stars, Windows + Android, free with a $14 Pro tier; the author reports "about 80 downloads, around 20 people who have actually opened it and one person paying for Pro" after a month of posting, and lists the unsigned-installer warning as a top adoption objection.[29][30]

**Windows built-in (Win+H) — the real default, and it is cloud.** Microsoft documents voice typing as using "online speech recognition, which is powered by Azure Speech services", requiring an internet connection, with a fixed language list and no custom vocabulary.[20] Every offline vendor's pitch is built against this: free, zero-install, and quietly uploading.

---

## 4. Market gaps KeyReed can credibly own

### 4.1 The four gaps that hold up under scrutiny

**Gap 1 — "Offline" and "polished" are still mutually exclusive on Windows.**
Handy is offline but explicitly does not rewrite.[15][23] Voicetypr and Amical rewrite but are Tauri/Electron-class and, in Voicetypr's case, paid.[21][25] Wispr rewrites and cannot work offline at all.[13] KeyReed ships six writing modes plus per-app adaptation *on CPU, offline, in a 23 MB installer*.[28] This is the one claim no competitor can copy in a sentence.

**Gap 2 — The "Privacy Mode" misconception.**
Users and even comparison blogs treat Wispr's privacy settings as a local-processing switch; Wispr's own page says transcription "always occurs on the cloud" and that Privacy Mode was only a training opt-out.[11][26] A plain-language page — *"Privacy Mode is not offline. Here is what actually leaves your machine"* — is a legitimate, factual, high-intent asset that no OSS vendor currently owns.

**Gap 3 — Insertion reliability in hostile Windows contexts.**
Wispr explicitly refuses password and credit-card fields and stays out of password managers and banking apps.[12] Most OSS tools paste and hope. KeyReed's Auto Insert falls back to the clipboard for password fields, elevated windows and focus changes, and offers Ask-Before-Insert.[28] For IT-administered, UAC-heavy or password-manager-heavy Windows environments this is a practical differentiator, not a marketing one.

**Gap 4 — Old and cheap hardware.**
Comparisons repeatedly note that offline quality depends on Apple Silicon or a GPU, and Superwhisper tells Intel Mac users to use cloud models.[10][27] KeyReed's entire model set runs on CPU via sherpa-onnx, and its measured latency (47 s of dictation ready ~0.5–0.7 s after key release on an i7-8850H, synthetic audio) is a claim aimed exactly at the 2019-era corporate laptop.[28] "Works on the laptop you already have, with no GPU and no account" is defensible and testable.

**Adjacent opening (weaker, but real):** auditability. Pinned SHA-256 model hashes, no telemetry, no account, MIT/Apache dual licence.[28] This matters to the HN/GitHub audience that reads the code before installing — the same audience that pushed Handy to 38,990 stars.[7]

### 4.2 Where KeyReed is weaker — say it before a competitor does

| Weakness | Why it matters | Honest framing |
|---|---|---|
| **Windows x64 only** | Wispr, Superwhisper, OpenWhispr and Handy all cover Mac; three cover mobile[1][10][8][7] | "Built for Windows first" — not "cross-platform" |
| **No mobile** | Wispr's four-platform footprint is its stickiness[1] | Out of scope; do not imply a roadmap date |
| **No AI command mode** | Wispr's Command Mode edits selected text by voice; OpenWhispr has "agent mode"[11][14] | Writing modes are style transforms, not arbitrary instructions — describe them as such |
| **No team/admin/SSO** | Growth/Enterprise revenue and the compliance story live there[2] | Individual-product positioning only |
| **Unsigned binaries** | SmartScreen warning is a documented adoption blocker for comparable apps[29] | Disclose it on the download page; code-signing is the highest-ROI trust spend |
| **English-centric models** | Parakeet v2 is English; v3 covers 25 European languages; SenseVoice adds CJK; no 100+ language parity[28] | Claim the languages the models actually cover, per model |
| **Zero community** | 0 stars vs Handy's 38,990; not indexed for any target query[28][7] | Distribution is the work, not the product |
| **Benchmark is synthetic** | The 0.5–0.7 s figure is a harness measurement on synthetic audio, not field data[28] | Publish the method with the number |

**Positioning consequence:** KeyReed cannot win "the best Wispr Flow alternative" head-on — that SERP is owned by listicles that rank on cross-platform coverage and star counts.[25][27] It can win *"the Windows dictation app that never sends your voice anywhere, and still cleans up your text."*

---

## 5. Audience prioritisation

| Rank | Audience | Why they fit KeyReed's verified abilities | Evidence |
|---|---|---|---|
| **1** | **Windows developers and AI-tool power users** (Claude Code, Cursor, VS Code, terminal) | Terminal/Code mode exists; CPU-only performance matters because dev laptops are often corporate-specced; they read licences and hashes; they are already searching for non-cloud options | HN: "Anyone using a Wispr Flow alternative that is non-cloud?"[23]; HN FreeFlow thread asks for Parakeet for offline latency[24]; OpenWhispr testimonials cite dictating into Claude Code[8] |
| **2** | **Privacy-constrained Windows professionals** (health, legal, finance, public sector, defence-adjacent) | No cloud, no account, no telemetry; clipboard fallback for password/elevated windows; no BAA needed because nothing leaves the device | Wispr gates HIPAA behind Growth/Enterprise with a signed BAA and turns off features when accepted[2][5]; r/WisprFlow "Privacy questions" / "Security Concerns on Wispr" threads `[unverified — titles only, Reddit blocked from this network]` |
| **3** | **RSI / accessibility users and heavy typists** who need dependable insertion more than AI cleverness | Small native footprint, hold-or-toggle, Ask-Before-Insert, clipboard fallback; accessibility framing is Handy's own stated mission[6] | Handy's "Accessibility tooling belongs in everyone's hands, not behind a paywall"[6]; Talon's RSI user base in comparison roundups[27] |
| **4** | **Writers, students, journalists on Windows laptops** (travel, offline, battery) | Offline after model download; 1.4 GB full-offline bundle for air-gapped setups; free | VoiceInk's "13,469+ writers, students, coders" positioning shows the segment buys on price + privacy[9]; Laxis: "legal wants audio kept on the device"[26] |
| **5** | **Cost-driven Wispr subscribers** ($144–180/yr) | Free, no subscription, no word cap | VoiceInk testimonials are a migration log away from $15/mo[9]; r/ProductivityApps "Before you pay $144/year for Wispr Flow…" `[unverified — title only]` |

**Who to deprioritise:** Mac users (no product), mobile-first users (no product), enterprise IT buyers (no SSO/audit/signed builds), and non-English-first markets beyond the 25 European languages Parakeet v3 and SenseVoice cover.[28]

**Why #1 before #2:** developers convert without a sales cycle, tolerate an unsigned installer, and generate the GitHub/HN evidence that later makes #2 credible. Regulated buyers will ask for code signing and a security page — both cheaper to produce after there is public traction.

---

## 6. Search intent & communities

### 6.1 What ranks, and what is missing

| Query | What currently ranks | Gap KeyReed can fill |
|---|---|---|
| `open source wispr flow alternative` | Nearly all listicle blogs from adjacent SaaS vendors: Gladia, Laxis, GetVoibe, AssemblyAI, DevToolLab, OpenAppScout, Archestack, Toolfinder[26][27][25] | No *first-party* comparison from an OSS vendor with a real feature matrix, honest weaknesses, and per-tool licences. Listicles are also commercially biased (Laxis and Voibe both disclose they sell a competitor)[26][27] |
| `wispr flow alternative windows` | Same listicles plus Windows-specific vendor posts (SpeakoFlow, DictationDaddy, Amical)[25][22] | No page that answers the Windows-specific questions: x64 vs ARM, UAC/elevated windows, password fields, SmartScreen warning, winget/Store availability |
| `offline voice dictation windows` | Microsoft Store listings (Murmur, Voxmelt, Bepo, Local Whisper) and vendor blogs[22][27] | Store listings rank because they are transactional; an OSS project with no Store listing is invisible. A credible "download without the Store, here is the hash" page is missing |
| `local whisper dictation windows` | GitHub repos (WhisperDictation, whisper-type, local-dictation) and PyPI packages — thin, developer-only pages | No polished explainer that compares *engines* (Whisper vs Parakeet vs SenseVoice vs Moonshine) on CPU latency and install size, with numbers |
| `voice typing for developers` | Vendor landing pages (FluidVox, Infina, Yaps, use.fo, OpenTypeless, Laxis) — content marketing, not OSS | No OSS Windows page that demonstrates Terminal/Code mode with actual symbol and identifier examples, and no page targeting "dictate into Claude Code on Windows" |

**Structural observations**

- The SERPs are **commercial-investigation dominated** (comparison, "best", "alternatives") and **vendor-authored**. Independent, testable data (latency on named hardware, install size, licence, offline behaviour) is the scarce input.
- **KeyReed has no organic footprint.** It does not appear for any of the four queries; the repo is 0 stars and created 15 November 2026.[28] Search is a 6–12 month project here, not a launch channel.
- **Nobody owns the "Privacy Mode ≠ offline" query.** This is the highest-leverage content bet because it is factual, defensible, and directly precedes a purchase decision.[11]

### 6.2 Communities worth engaging

| Community | Why | Note |
|---|---|---|
| Hacker News (Show HN) | The single highest-yield launch surface for this category: Handy hit 247 and 237 points; Whispering hit 591; a free Wispr alternative hit 277 with 132 comments[37][24] | HN rewards a technical angle (Rust, Win32, CPU latency, model hashes), not a growth angle |
| r/software, r/SideProject | Where Windows dictation launches actually happen; VoxScribe's own posts live here and drew substantive feedback[29] | Expect "how is this different from Handy?" as the first comment — answer with the grammar-cleanup gap[23] |
| r/ClaudeCode, r/LocalLLaMA | Developer-adjacent, already discussing offline dictation for prompting[24] | Tool promotion is tolerated when framed as a build log, not an ad |
| r/WisprFlow, r/ProductivityApps | Where churn is discussed: privacy, $144/year, cloud dependency[26] | Do not astroturf; answer privacy questions factually |
| r/ADHD_Programmers, r/freelanceWriters | Adjacent demand for dictation as a working method | Lower volume, higher intent |
| GitHub topics (`offline-speech-recognition`, `speech-to-text`, `dictation`) | Passive discovery; Handy and OpenWhispr both surface here[7][31] | Cheap, slow, compounding |

**Posting rules:** Reddit blocked automated access from this network, so subreddit-specific self-promotion rules could not be read and remain `[unverified]`. Working assumption: most of these subs enforce a contribution-to-promotion ratio and ban bare link drops — lead with a build log or a benchmark, disclose authorship in the first line, and answer comments for 48 hours. HN's Show HN norms (no landing-page-only submissions, be present in the thread) apply.

---

## 7. Not verified / limits of this research

- Wispr Flow's Windows x64-only restriction and Snapdragon rejection are reported by a secondary comparison[26]; Wispr's own requirements article was retrieved but its device table did not render in text extraction.[12] Treat as `[unverified]` on primary evidence.
- Reddit thread content could not be fetched (network-level block); Reddit is cited only at title/snippet level and subreddit rules are `[unverified]`.
- GitHub star counts are a single API snapshot on 17 November 2026, not growth rates.
- `voicegecko.io` was unreachable from this network; its status is inferred from the GitHub repo's last commit (15 September 2026) and the parked `.com` domain.[17]
- VoxScribe's "~80 downloads" is self-reported by its author in a Reddit post.[29]
- KeyReed's latency figure is a harness measurement on synthetic audio on an i7-8850H, per project documentation.[28] It is not an independent benchmark and not a field measurement.
- No pricing, user count, or revenue figure in this document is estimated; where a vendor does not publish a number, none is given.

## Sources

[1] https://wisprflow.ai
[2] https://wisprflow.ai/pricing
[3] https://wisprflow.ai/why-flow
[4] https://wisprflow.ai/business
[5] https://wisprflow.ai/privacy
[6] https://handy.computer
[7] https://github.com/cjpais/Handy
[8] https://openwhispr.com
[9] https://tryvoiceink.com
[10] https://superwhisper.com
[11] https://wisprflow.ai/data-controls
[12] https://docs.wisprflow.ai/articles/1036674442-supported-devices-and-system-requirements
[13] https://docs.wisprflow.ai/articles/2772472373-what-is-flow
[14] https://openwhispr.com/pricing
[15] https://raw.githubusercontent.com/cjpais/Handy/main/README.md
[16] https://raw.githubusercontent.com/Beingpax/VoiceInk/main/LICENSE
[17] https://github.com/Lukem121/voicegecko
[18] https://getwrither.com
[19] https://github.com/samiralibabic/whispering
[20] https://support.microsoft.com/en-us/windows/use-voice-typing-to-talk-instead-of-type-on-your-pc-fec94565-c4bd-329d-e59a-af033fa5689f
[21] https://voicetypr.com
[22] https://amical.ai
[23] https://news.ycombinator.com/item?id=49809670
[24] https://news.ycombinator.com/item?id=47040375
[25] https://openappscout.com/collections/open-source-wispr-flow-alternatives
[26] https://www.laxis.com/blog/wispr-flow-alternatives
[27] https://www.getvoibe.com/resources/best-open-source-wispr-flow-alternatives
[28] https://github.com/Shanmukh-l33t/keyreed
[29] https://www.reddit.com/r/software/comments/1wxsdbj/i_built_a_free_open_source_offline_dictation_app
[30] https://github.com/ahmedhmam1994/voxscribe-ai-voice-dictation
[31] https://github.com/OpenWhispr/openwhispr
[32] https://github.com/Beingpax/VoiceInk
[33] https://github.com/amicalhq/amical
[34] https://github.com/ideaplexa/voicetypr
[35] https://github.com/zachlatta/freeflow
[36] https://github.com/benmaster82/writher
[37] https://news.ycombinator.com/item?id=44942731
