# KeyReed design v3: "a reed that breathes"

Branch `design/v3` (worktree `keyreed-design-v3`), from `release/2.1` @ `3431728`. Design, UI, UX and motion
only: speech, grammar, insertion, config and every behaviour stay as they are. Status: **Phase 0 done, Phase 1
(spec + concept) awaiting approval.** Choices and their alternatives are in [decision-log.md](decision-log.md).

## 0. Baseline (Phase 0)

### Build and tests at `3431728`

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 351 passed, 0 failed, 6 ignored (hardware/UIA) |
| `cargo build --release -p vi-app` | green (1 existing warning in `vi-app`) |
| Before gallery | `docs/design/audit/v3-before/{light,dark}-s{100,150,200}/` (85 shots each, PNG) |

### Surface inventory

| Surface | Owner | States / pages |
| --- | --- | --- |
| Listener pill | `overlay.rs` (pure renderer), `ui.rs` (window, timers, tray) | 11 `PillKind`s, 16+ gallery states incl. Quick Fix (Fixing, Fixed, Undone, Looks good, Refused, Learned) |
| Settings | `settings.rs` | 11 pages (Get started, General, Dictation, Writing, My style, Output, Microphone, Speech models, Processing, Privacy, About), 4 setup steps, in-app modal, frameless header |
| Review panel | `composer.rs` | opening, settled, show original, rejected/focus, note, long text, first use |
| Composer bar | `composer/bar.rs` | **connected in 2.1** (Ctrl + Win + K, tray, pill click); restyle only |
| Canvas | `canvas.rs` | SDF shapes + GDI text, opaque |
| Tray icons | `ui.rs` `create_state_hicon` | static |

### File classification (functional lock)

| Class | Files |
| --- | --- |
| A (presentation, free) | `overlay.rs`, `canvas.rs`, `tokens.rs` (new), `motion.rs` (new), `perf.rs` (new), `frameclock.rs` (planned), `scripts/design/shots.py` (new), docs |
| B (paint/layout/motion/microcopy only) | `settings.rs`, `ui.rs`, `composer.rs`, `composer/bar.rs`, `vi-app/src/gallery.rs`, `vi-win32/src/lib.rs` (module list) |
| C (never touched) | `vi-asr`, `vi-audio`, `vi-rewrite`, `vi-core`; `hotkey.rs`, `typing.rs`, `clipboard.rs`, `selection.rs`, `uia.rs`, `context.rs`, `security.rs`, `spell.rs`; `vi-app/src/main.rs`; NSIS, CI, release scripts |

### Baseline performance (release build, this machine, `KEYREED_UI_PERF=1`, dark theme)

Raw reports: `docs/design/audit/v3-before/perf-dark-s{100,200}.json`. CPU figures include the gallery's own
driving work, so they are an upper bound and only comparable before/after.

| Metric (budget) | 100 % | 200 % | Verdict |
| --- | --- | --- | --- |
| Pill frame p95 while listening (<= 2.0 ms @ 200 %) | 0.70 ms | **4.58 ms** | over at 200 % |
| Pill longest frame while listening (< 16.7 ms) | 2.15 ms | 5.18 ms | ok |
| Pill frame interval p50 / p95 while listening (p95 <= 20 ms) | 17.9 / **34.5 ms** | 19.6 / **37.0 ms** | `WM_TIMER` jitter (15.6 ms quantum) |
| Pill frame p95 across transitions | 1.53 ms | **13.4 ms** (max 25.6) | over |
| Settings full repaint p50 / p95 (one animated frame <= 6 ms) | 12.0 / 21.0 ms | **48.5 / 111 ms** | full-window DIB every paint |
| Settings switch slide | 33 frames, interval p50 26 / p95 35 ms | **4 frames** (the 47 ms full repaint for "Saved" ate the 150 ms slide) | over |
| Review panel frame p95 | 45.7 ms | 70.5 ms | over |
| Composer frame p95 | 10.9 ms | 35.0 ms | over |
| Idle 2 s, pill | 0 frames, timer off | 0 frames, timer off | ok |
| Idle 2 s, Settings open | 0 frames | 0 frames | ok |
| CPU, idle (gallery process) | 0.0 % | 1.6 % | |
| CPU while listening (pill animating) | 4.6 % | 19.2 % | high at 200 % |

Found while measuring: the pill painted 55 frames (about every 61 ms) during the Settings switch test although
nothing on the pill changed. To be traced in Phase 2; it breaks "silent at rest" while Settings is in use.

## 1. Principles

Taken from the brief, in order: never in the way; honest motion; silent at rest; physical, not decorative;
brand-literal playfulness (the keycap gets pressed, voice becomes text, the check draws itself); editorial calm;
accessible by construction.

## 2. Tokens (`crates/vi-win32/src/tokens.rs`)

### Colour: Settings, Review panel, Composer

| Token | Light | CSS | Dark | CSS | Role |
| --- | --- | --- | --- | --- | --- |
| `sidebar` | `#EEF4EF` | `--bg-2` | `#060908` | `--bg` | header + sidebar, the deepest tone |
| `bg` | `#F6F9F6` | `--bg` | `#0A110E` | between `--bg-2` and `--surface` | the raised content panel |
| `surface` | `#FFFFFF` | `--surface` | `#101915` | `--surface` + one step | cards, groups |
| `border` | `#E3EAE6` | `--line` on white | `#1E2C26` | `--line` on the card | hairlines |
| `divider` | `#ECF1EE` | | `#1A2621` | | row separators |
| `text` / `text2` | `#0C1A14` / `#3F5248` | `--text` / `--text-2` | `#E8F1EC` / `#A5B8AF` | `--text` / `--text-2` | |
| `text3` | `#5B6B63` | darker than `--text-3` for AA | `#8DA398` | lighter than `--text-3` for AA | metadata, eyebrows |
| `accent` | `#065F46` | `--serif-a` | `#6EE7B7` | `--serif-c` | links, selected icon |
| `accent_fill` / `press` | `#047857` / `#064E3B` | `--em-2` | `#34D399` / `#10B981` | `--em-2` / `--em` | primary, switch on |
| `accent_soft` | `#E1F3EA` | | `#103025` | | selected tint |
| `control_border` | `#7F8F87` | | `#6F8479` | | >= 3:1 boundaries |
| `working` | `#0F5A3F` | `--brand` | `#A7F3D0` | `--mint` | downloading, transcribing (was teal) |
| warn / err / ok | unchanged | | unchanged | | amber = needs you |
| `shadows` | yes | `--card-shadow` | no | `--card-shadow: none` | depth strategy |

Measured contrast (both enforced by `both_themes_meet_wcag_aa`): light text/surface 17.9, text2 8.4, text3 5.0-5.6,
accent on tint 6.7, label on primary 5.5, control boundary 3.2-3.4; dark text 15.6, text2 8.6, text3 6.7-7.4,
accent on tint 9.4, label on primary 8.6, control boundary 4.5-4.8.

### Colour: listener (always dark, `tokens::listener`)

Unchanged brand values (surface `#0C1411`, emerald `#10B981` / `#34D399`, mint `#A7F3D0`, amber `#F5C26B`, rose
`#FB7185`) except **working**: teal `#2DD4BF` (outside the brand families) becomes a mint edge with neutral text
and a mint sheen. Contrast on the capsule: text 16.2, text2 9.0, text3 4.7, mint 14.6, amber text 15.0, rose text
13.3; glyphs emerald 9.7, amber 11.4, rose 6.9.

### Shape, elevation, type

- 4 dp grid. Radii: cards, groups, panel, choice cards **12**; buttons, inputs, drop-downs, nav items **10**; chips
  and the pill full. (Was 8 / 6.)
- Light: the panel carries `--card-shadow` (two layers, tinted `#0B3B2C`); cards carry only its first layer and no
  border. Dark: no shadows; tone steps plus one hairline. High contrast: system colours and outlines only.
- Type: Segoe UI Variable stays for body and all user text. New **Eyebrow** role for section labels: 11 px
  semibold, uppercase at paint time, +1 px tracking, `text3`. Geist / Instrument Serif are gated (decision D9).
- Focus: 2 dp ring, instant, never animated (unchanged).

### Motion (`crates/vi-win32/src/motion.rs`, tested)

| Token | Value | Use |
| --- | --- | --- |
| `STANDARD` | cubic-bezier(0.2, 0.8, 0.2, 1) | fades, colour, small moves (website `--ease`) |
| `EXIT` | cubic-bezier(0.4, 0, 1, 1) | leaving |
| hover | instant | hover fills, both ways |
| `PRESS_MS` / `FADE_MS` / `PAGE_MS` / `HERO_MAX_MS` | 70 / 140 / 220 / 450 ms | |
| `MORPH` | 0.30 s, bounce 0.15 (overshoot 0.6 %) | the pill family's one spring |
| `SNAPPY` | 0.20 s, bounce 0.10 | indicators, nav, hover-intent width, scroll, snap |
| `ACCENT` | 0.28 s, bounce 0.25 (overshoot 2.8 %) | toggle thumb, radio dot, success pop |
| `SETTLE` | 0.36 s, bounce 0 | back to rest, progress |
| `REDUCED_FADE_MS` | 160 ms opacity only | everything under Reduce Motion |
| stagger | 14 ms per item, at most 6 | bars, chips, groups |
| rest | within 0.25 px and slower than 4 px/s | the clock stops |

Springs are closed-form (any timestamp, any frame rate), retargeting keeps position and velocity, Reduce Motion
puts every spring at its target and shortens long fades to 160 ms, and a time override lets the gallery render
any instant.

## 3. Storyboards

### Listener (P0-P13)

| # | State | v3 |
| --- | --- | --- |
| P0 | Idle | 36 dp circle, mark in its well, mint top lip (8 %), soft shadow. Zero frames. *(lip done)* |
| P1 | Hover intent (380 ms) | width on `SNAPPY`; labels fade in 40 ms later, rising 4 dp; out: labels first (80 ms), width on `SETTLE`. |
| P2 | Key down | the capsule **sinks to 60 %** (the existing `LISTENING_SCALE`) on `MORPH` while the keycap presses 1.5 dp and its well darkens 6 % (`PRESS_MS`); bars stagger out from the centre. *(press done, spring pending)* |
| P2b | Listening | **centre-out history** (newest in the middle), 3 % phosphor tail per step, silence rests as dots at 40 %; edge light = 0.30 + 0.55 x smoothed level; auto-level unchanged (peak/room tracking), fixed-step 240 Hz with a 140 ms release (pending). *(centre-out, tail, dots, edge done)* |
| P2c | Esc | bars sink and fade 140 ms, keycap springs up, width collapses on `MORPH`. |
| P3 | Key up | capsule springs back to full height (`ACCENT`), keycap rises; bars **converge into three text lines** (100 / 76 / 52 % of a 10 dp slot, +16 dp exactly); label fades in; a 1.1 s mint sheen runs over lines and label while the work is real. *(lines + sheen done, morph pending)* |
| P3b | Polishing | same lines; sheen leans emerald; label cross-fades. *(done)* |
| P4 | Inserted | lines collapse to a point, the check draws itself (200 ms), pop 1.00 -> 1.04 -> 1.00 (`ACCENT`), one emerald edge + margin glow fading 0.45 -> 0 over 320 ms. Under 450 ms in total. *(flash done)* |
| P5 | Copied | as P4 with no flash and no pop. *(done)* |
| P6-P9 | Attention, Notice, Error, Paused | unchanged meaning; nod / two-shake on the motion core; Paused mark at 45 %. |
| P10 | Quick Fix | single softer band (14 %, wider), tick and count-up on the core, chips get hover/press. *(band done)* |
| P11 | Mode hint | label slides from the icon side (`SNAPPY`); rapid cycling cross-fades the name in place, width retargets without breathing. |
| P12 | Drag | lift: shadow +40 %, scale 1.02 (`SNAPPY`); release near an edge springs into the snap. |
| P13 | Show / hide | scale 0.92 -> 1 plus `FADE_MS`; never delays dictation. |

Width budgets (to be enforced by an extended `pill_labels_stay_compact`): idle 36 dp, listening 60 % of 104 dp,
every text state at most +16 dp over 2.1.

### Settings (S0-S14) and setup

| # | Element | v3 |
| --- | --- | --- |
| S0 | Layout | deep header + sidebar, page in one raised rounded panel (radius 12, 8 dp from the right and bottom; under the 52 dp header). Cards quieter. `X0` / `CW` unchanged, so no page newly scrolls. *(done)* |
| S1 | Open | window instant; content fades and rises 6 dp over 180 ms. |
| S2 | Nav | instant hover; the selection is **one** fill + marker that slides on `SNAPPY` (each nav child paints its slice of the shared indicator). |
| S3 | Page change | out 90 ms, in rises 8 dp with up to 4 groups staggered, <= 220 ms; native children shown when it ends; any input finishes it at once. |
| S4 | Toggle | thumb on `ACCENT` with a 20 % squash mid-travel; track and On/Off cross-fade. |
| S5 | Choice cards | instant hover (light: one lift step, dark: brighter hairline); ring fades in 160 ms, dot scales on `ACCENT`. |
| S6 | Buttons | instant hover; press 1 dp inset + darker (`PRESS_MS`); one Primary per page; Primary gets a 1 px top highlight. |
| S7 | Progress | fill springs to the real fraction (`SETTLE`), never ahead; verifying shows a slow band; completion morphs into the badge. |
| S8 | Mic test | the pill's waveform renderer composited into the canvas with the target band; absolute level, never auto-level. |
| S9 | Writing example | the "writes" line cross-fades (90 ms out, 180 ms in, rising 6 dp). |
| S10 | Saved | check + "Saved" fades in; 1.8 s timeout unchanged. |
| S11 | Status chip | static; the dot pulses only while really listening. |
| S12 | Scroll | wheel delta animates on `SNAPPY`; focus still scrolls into view. |
| S13 | Headers | Title role, Eyebrow sections, serif accent only where allowed. *(eyebrows done)* |
| S14 | Native pieces | menus, tooltips stay native and themed. |
| Setup | | segments fill on `SNAPPY`; directional step change; one 1.2 s intro on Welcome; the drawn keycap sinks when the real shortcut is pressed; Microphone uses S8; Practice framed as a compose card; finish ring draws once. |

### Review panel and Composer

Review: scale 0.96 -> 1 + fade on `MORPH` from the pill's side, closes in 120 ms, underlines draw in, accept/reject
cross-fades, keyboard model unchanged. Composer: v3 tokens (already inherited from `theme()`), bloom and fold on
the motion core, behaviour untouched.

## 4. What comes from Hex (reference B), and what does not

| Hex idea | Where it lands in KeyReed |
| --- | --- |
| Duration + bounce springs, one `MORPH` family spring | `motion.rs` presets, pill geometry |
| Reduce Motion keeps a 0.16 s crossfade | `REDUCED_FADE_MS`, `Tween` |
| Hover instant, only on single targets; rows never hover | S2, S5, S6 |
| One accent per screen; states are dots, not buttons | S6, S11 |
| Continuity: shared objects move, never duplicate | S2 sliding selection, P3 bars -> lines |
| Deep sidebar + raised rounded panel | S0 |
| Dot scope engineering: fixed step, release time, centre-out history, phosphor, auto-level, shimmer pass | P2b, P3 |
| Windows pacing: fixed 60 Hz, one frame in flight, never disarm on the frame that just drew | `frameclock.rs` (Phase 2) |
| A live keycap that sinks | P2, setup keycap |
| Goldens in light and dark within a tolerance | `crates/vi-win32/tests/golden/` (Phase 2/3) |
| Copy discipline | microcopy pass, keeping On/Off by switches (KeyReed's audit choice) |

Not taken: Hex's colours, fonts, layouts, Dot scope's dot field and braided traces, sounds, code or assets.

## 5. Kept vs evolved

| Kept | Evolved |
| --- | --- |
| Logo, untouched, in its well | the well presses with the key |
| Emerald on evergreen-black, green-tinted neutrals | surfaces aligned to the website tones; teal working hue removed |
| Dark capsule, icon first, 36 dp idle, 60 % listening | centre-out honest bars, text lines, edge light tied to level |
| 11 pages, 4 steps, names and order, 940 x 660 dp | raised panel, quieter cards, 12/10 radii, eyebrows |
| On/Off words, focus ring, high contrast, reduced motion | springs with retargeting, one frame clock |
| Taglines, product voice, every string's meaning | sentence-case and ellipsis polish only |

## 6. Concept frames (real canvas, real windows)

`docs/design/v3-concept/`: `pill-before-dark-s150.png` vs `pill-v3-dark-s150.png`;
`writing-{light,dark}-before-vs-v3.png`, `welcome-{light,dark}-before-vs-v3.png`, and single v3 frames of Writing,
Welcome and General in both themes, all from `keyreed --ui-gallery --scale 150`.
