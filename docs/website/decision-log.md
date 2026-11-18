# KeyReed Website v3 — Decision Log & Claim Ledger

Date: 2026-11-18  
Branch: `site/v3-premium`  
Canonical domain verified: `https://www.keyreed.com/` (`https://keyreed.com/` returns `HTTP/2 308` → `https://www.keyreed.com/`)

---

## 1. Baseline Audit (Before Rebuild)

Measured locally against `main` (`9e3b20b`) via Playwright Chromium and Lighthouse:

| Metric | Before (`main` @ `9e3b20b`) | v3 Target |
| --- | --- | --- |
| Homepage `<main>` sections | 13 | 8 |
| Visible words (collapsed FAQ excluded) | 2,276 (2,406 `innerText`) | ≤ 700 |
| Scroll height @ `1440×900` | 16,177 px (~18 screens) | ~6,800–8,200 px (incl. 300 vh pin) |
| Scroll height @ `390×844` | 25,574 px | Compact, no pin |
| Lighthouse Mobile (Perf / A11y / BP / SEO) | 100 / 95 / 100 / 100 (LCP 1,056 ms, CLS 0, TBT 0 ms) | ≥ 90 / ≥ 98 / 100 / 100 |
| Lighthouse Desktop (Perf / A11y / BP / SEO) | 100 / 95 / 100 / 100 (LCP 303 ms, CLS 0, TBT 0 ms) | ≥ 97 / ≥ 98 / 100 / 100 |

**Baseline accessibility defects identified and resolved in v3:**
1. Light-theme `--text-3` (`#66786e`) had a `4.2:1` contrast ratio on `--bg-2` (`#eef4ef`), failing WCAG AA `4.5:1`. Updated light `--text-3` to `#485c52` (`6.6:1`) and dark `--text-3` to `#8da59a` (`6.7:1`).
2. Footer column headings used `<h4>` directly after `<h2>`, triggering `heading-order`. Replaced with styled `<p class="foot-heading">` labels.

---

## 2. Reference Study (Live Browser Inspection, 2026-11-18)

### Studied references
1. **`https://anoma.ly/hex/` (Hex by Anomaly)** — Inspected live in Chromium (`1440×900`). Single-screen dark canvas, two `<canvas>` layers, a centered tactile 3D beveled keycap with specular top edge and `"Click to play"` prompt, H1 `"Speak. It appears."`, one two-line subhead, and a clean copyable command block.
   - *Rejected:* Mascot illustration, orange burst palette, pure-black-only theme, Inter typography.
2. **`https://wisprflow.ai/` + `/pricing` + `/privacy` + `/data-controls` (Wispr Flow)** — Inspected live in Chromium (`1440×900`). Editorial serif italics paired with clean geometric sans; raw spoken transcript curving into a centered waveform pill and emerging as clean formatted prose; full-bleed rounded colour slabs that scale/settle on scroll; pinned comparison storytelling. Also observed 15+ third-party advertising/tracking scripts (`tiktok`, `fbevents`, `applovin`, `linkedin`, `twitter`, `axon`) — reinforcing KeyReed's zero-tracker, strict-CSP stance.
   - *Rejected:* Cream/lavender palette, Figtree/EB Garamond, invented WPM multipliers (`4x faster`), customer logos, testimonials.
3. **Linear / Raycast / Vercel / Apple craft patterns** — Raycast physical 3D keycap lighting and keyboard-first ergonomics; Linear cursor-tracked border luminescence (`.lit`) and dark surface layering; Vercel hairline grid/beam precision and technical receipts; Apple shared-element scroll-scrubbed window transformation.

### 10 Adopted Qualities
1. **One tactile hero toy (`Right Ctrl` 3D keycap)** — Press-and-hold via pointer, `Space`/`Enter`, or the user's real physical `ControlRight` key (> 250 ms solo hold; ignored if another key is pressed so shortcuts like `Ctrl+C` never trigger it).
2. **Living brand metaphor ("Reeds that hear you")** — Hand-written WebGL2 instanced emerald reed blades along the base of the dark hero stage, swaying in breeze, parting away from pointer wind, and rippling outward with the voice envelope.
3. **Raw-to-clean curved flight** — Spoken words including `"um"` and `"uh"` stream in monospace, filler words strike through and collapse via `SplitText`, waveform bars settle into three text lines (*voice becomes text*), and the clean sentence arcs into a floating mail window.
4. **Full-bleed rounded dark stages (`40px` desktop / `28px` mobile)** — Rhythm of dark hero slab → airy light proof & product scenes → cinematic dark privacy slab (scaling `0.94 → 1.0` on entry) → light open-source & FAQ → calm dark finale slab. Light remains the default page theme.
5. **Shared-element scrubbed app window** — One window frame pinned for ~`300vh` on desktop morphs cleanly across Mail (`Professional`), Chat (`Casual`), Code editor (`Technical`), and Terminal (`Terminal / Code`) without cuts; degrades to a swipeable snap carousel on `< 768px` and stacked frames under `prefers-reduced-motion`.
6. **Interactive mini-demos backed by real product captures** — Quick Fix (`Ctrl+Win+G`) animates selection + chord + underline fix into the real `quickfix-review.webp` capture; Composer (`Ctrl+Win+K`) lets visitors click `Shorter` · `Warmer` · `Clearer` chips over the real `composer-result.webp` capture.
7. **Opt-in synthesized WebAudio** — Zero audio asset weight; muted by default with a persistent nav speaker button (`keyreed-sound` in `localStorage`); synthesizes a soft mechanical key thock, low reed breeze whoosh, and a gentle harmonic chime on insertion.
8. **Code-as-receipts** — Instead of marketing promises, the privacy slab highlights the real generated `allows_network()` Rust excerpt (`crates/vi-core/src/types.rs` via `excerpts.js`) line-by-line on scroll.
9. **Honesty without hero clutter** — Unsigned SmartScreen disclosure, SHA-256 verification (`Get-FileHash`), hardware caveats, and deep model/pipeline specs move out of the hero into download disclosures, concise FAQs, and the new `/details` reference page.
10. **Strict zero-tracker performance** — `script-src 'self'`, self-hosted variable fonts, vendored motion stack, `requestIdleCallback` WebGL boot after first paint, and full static readability with JS disabled.

---

## 3. Motion Stack & Licensing Decision

| Need | Choice | Version & Licence | Verification |
| --- | --- | --- | --- |
| Timelines, pinned scroll scenes, word/char splitting | `gsap.min.js`, `ScrollTrigger.min.js`, `SplitText.min.js` | `3.15.1` — Webflow/GreenSock Standard "No Charge" License (`https://gsap.com/standard-license`, effective 2025-04-30, modified 2025-05-30) | Verified live license text: Section II grants a worldwide non-exclusive license to *"use, reproduce, display, and implement GSAP Products solely for Permitted Uses"* (any website or web app not competing with Webflow's visual no-code animation builder), with all plugins including `SplitText` and `ScrollTrigger` 100% free and proprietary headers preserved. `LICENSE.txt` included in `website/vendor/gsap/`. |
| Smooth scroll (fine-pointer desktop only) | `lenis.min.js` (+ `lenis.min.js.map`) | `1.3.27` — MIT License (`darkroom.engineering`) | `LICENSE` included in `website/vendor/lenis/`. Source map included so Lighthouse `valid-source-maps` passes with HTTP 200 while normal browsers download 0 extra bytes. |
| Hero reed field | `website/js/reeds.js` | Hand-written WebGL2 (`MIT OR Apache-2.0`) | Instanced emerald blades (~1,600 desktop / ~600 mobile), DPR capped at 1.75, paused offscreen/hidden, falls back to generated `assets/reeds-poster.webp`. |
| Sound | `website/js/hero.js` | Runtime WebAudio synthesis (`MIT OR Apache-2.0`) | Off by default; zero audio files. |

Note on `.gitignore`: Root `.gitignore` contains an unanchored `vendor/` rule intended for Rust/C++ build dependencies. Vendored web libraries in `website/vendor/` are tracked explicitly (`git add -f website/vendor/...`) without modifying non-website repository rules, and `scripts/check-site.mjs` verifies every file in `website/vendor/` is documented in `website/vendor/README.md`.

---

## 4. Messaging Hierarchy & Homepage Copy Budget (≤ 700 Visible Words)

Collapsed `<details>` disclosures, collapsed FAQ answers, `aria-only`/`.sr` labels, and decorative illustration chrome (`aria-hidden="true"`) are excluded per §5.3. Target visible word count: **~560 words** (well inside the `≤ 700` ceiling).

| Block | Section | Purpose | Word Budget | Target |
| --- | --- | --- | --- | --- |
| 0 | Nav | Brand, 4 section links + Details, sound, theme, live GitHub stars, Download | ~15 | 14 |
| 1 | Hero ("Reeds that hear you") | Release pill, H1 (`Your voice. Your words. Your machine.`), 1-line sub, Windows `.exe` + GitHub CTAs, interactive `Right Ctrl` toy | ≤ 40 (copy) + ~35 (toy) | ~68 |
| 2 | Proof band | `0.5 s` · `0 bytes` · `$0` count-ups in Geist Mono with honest footnotes & method link | ≤ 30 (+ footnotes) | ~48 |
| 3 | "Every app. The right voice." | Pinned 4-app morph (`Mail`, `Chat`, `Code editor`, `Terminal`) + all 6 mode chips | ≤ 60 | ~58 |
| 4 | "More than dictation." | Quick Fix (`Ctrl+Win+G`) & Composer (`Ctrl+Win+K`) interactive mini-demos + real 2.1.0 captures | ≤ 80 | ~74 |
| 5 | "Private by design." | Dark slab scale-in, canvas/SVG particle flow + dim disconnected cloud, 3 proof chips, real `types.rs` `allows_network` excerpt | ≤ 70 | ~66 |
| 6 | "Free. Open. Yours." | 4-row KeyReed vs Wispr Flow table + sourcing footnote + live GitHub open-source card | ≤ 70 | ~78 |
| 7 | FAQ (6 questions) | 6 concise Q&As (answers collapsed by default, ≤ 60 words each, linked to `/details`, 1:1 `FAQPage` JSON-LD parity) | ~55 visible | ~52 |
| 8 | Finale + Footer | Calm reeds, giant keycap download button, collapsed SHA-256 & SmartScreen disclosures, 4-column footer | ≤ 50 + footer | ~85 |
| **Total** | | | **≤ 700** | **~543** |

**Moved to `/details` ("Everything, in detail"):**
- Full 8-model speech catalog table (`Parakeet v2`, `v3`, `Ultra`, `110M`, `Whisper small.en`, `SenseVoice Small`, `Moonshine Base`, `Moonshine Tiny`) + grammar tagger (`gector-bert-en-int8`) & local writing models (`Qwen3.5 0.8B/2B/4B`).
- Full 5-stage dictation & cleanup pipeline (`Hotkey` → `Capture` → `sherpa-onnx` → `Tier 0` → `Optional Qwen3.5 + Guards` → `Auto Insert / Clipboard`).
- Complete privacy architecture (RAM-only audio zeroization, DPAPI-encrypted cloud keys, clipboard history flag, processing profiles, generated code excerpts).
- Full keyboard shortcuts & CLI flags table.
- Benchmark methodology & raw hardware numbers (`i7-8850H`, `Quadro P2000`, synthetic TTS vs LibriSpeech/CMU Arctic).
- System requirements & full transparent Known Limitations list from `README.md` / `CHANGELOG.md`.

---

## 5. Truth & Copy Claim Ledger

Every factual statement on `/`, `/details`, and `/vs-wispr-flow` traces to an exact source in this repository or a dated external check:

| Claim on Site | Exact Value | Source |
| --- | --- | --- |
| Latest version & release date | `2.1.0` (18 November 2026) | `README.md:32`, `CHANGELOG.md:3`, GitHub tag `v2.1.0` |
| Installer filename, size & SHA-256 | `KeyReed-Setup-x64.exe`, `24.4 MB` (`24,427,697` bytes), `e83ccc84791ccece3e00524b9c7fb2f4e76b876409b8867cce84475a2f0e18dd` | `gh release view v2.1.0`, `docs/installation.md:17,48` |
| Portable archive filename, size & SHA-256 | `keyreed-2.1.0-windows-x86_64-portable.zip`, `47.3 MB` (`47,320,361` bytes), `ed19f1f265134db2968205bbf51248b36fd5d6ea8149ec28f8322ed50e8bb35c` | `gh release view v2.1.0`, `docs/installation.md:18,49` |
| Quick Fix grammar archive & SHA-256 | `gector-bert-en-int8.tar.bz2`, `73.1 MB` (`~70 MB` catalog), `5654cefc549c938bbac8770d9753a8be50e6d66f9afdc6d7ec746f3d51590ae0`, Apache-2.0 | `crates/vi-asr/src/catalog.rs:151-163`, `docs/installation.md:19,50` |
| Supported OS | Windows 10 & 11 x64 (built & tested on 64-bit Windows 11; Windows 10 x64 supported by code, not re-tested for 2.1.0) | `README.md:149,177`, `docs/installation.md:5` |
| Licence | Dual licensed MIT or Apache-2.0 | `Cargo.toml`, `README.md:202` |
| Streaming speed figure | `0.5 s` (`0.5–0.7 s`) after releasing a 47-second dictation on a 2018 Intel Core i7-8850H laptop CPU, Exact mode, synthetic speech | `README.md:62`, `CHANGELOG.md:107`, `docs/research/08-e2e-speed-accuracy-worklog.md:171,183` |
| Long dictation with grammar rewrite | ~`7.2 s` release-to-typed on a 60 s dictation (`3.5 s` local SLM rewrite), ~`600 chars/s` typing | `docs/research/08-e2e-speed-accuracy-worklog.md:174,195`, `website/index.html:672` |
| Default network behaviour | `0 bytes` of audio or text sent anywhere in `Maximum Privacy (100% Offline)` default profile; enforced by `allows_network()` & unit test; one-time HTTPS model download on setup | `crates/vi-core/src/types.rs:178-216`, `README.md:56,134-136` |
| Speech models count & specs | 8 INT8 CPU models (`Parakeet TDT 0.6B v2` 460 MB default, `v3` 465 MB, `Ultra` 465 MB, `110M` 103 MB, `Whisper small.en` 606 MB, `SenseVoice Small` 999 MB, `Moonshine Base` 239 MB, `Moonshine Tiny` 103 MB) | `crates/vi-asr/src/catalog.rs:39-146`, `docs/architecture.md:39-51` |
| Writing modes & examples | 6 modes (`Smart`, `Exact`, `Professional`, `Casual`, `Technical`, `Terminal / Code`) + deterministic `WritingMode::example()` pairs | `crates/vi-core/src/types.rs:7-91`, `README.md:94-105` |
| Quick Fix & Composer | Quick Fix (`Ctrl+Win+G`): Windows spell checker + local `gector-bert-en` tagger, Undo on pill, Review panel for uncertain/formatted edits. Composer (`Ctrl+Win+K`): local `Qwen3.5 0.8B` streaming rewrite/presets | `README.md:38-39,87-88`, `CHANGELOG.md:16-18,35-36` |
| Memory & GPU | No GPU needed (ASR on CPU; optional Vulkan GPU speeds up grammar `2.7–6×` on Quadro P2000). ~`1.6 GB` working set with grammar runtime on a 4 GB test VM | `README.md:61,150,152`, `CHANGELOG.md:110` |
| Unsigned installer & SmartScreen | Executables and installer are not code-signed yet; SmartScreen may warn; SHA-256 verification documented | `README.md:32,45,169`, `docs/installation.md:37-53` |
| Wispr Flow comparison facts | Free tier limited words/week (`2,000 words/week` on Mac/Windows per Flow docs); Pro `$15/mo` (`$12/mo` or `€12/mo` billed annually); Mac/Windows/iOS/Android; 100+ languages; *"Transcription always occurs on the cloud"*; Command Mode, snippets, Notetaker, SOC 2 Type II, ISO 27001, HIPAA on Growth/Enterprise | Re-verified live on `https://wisprflow.ai/pricing`, `https://wisprflow.ai/privacy`, and `https://wisprflow.ai/data-controls` on **18 November 2026** |
