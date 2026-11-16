# KeyReed design audit and change record (branch `design/premium-ui-v2`)

Design-only mission. Baseline = `main` @ `3d9b5c5` (v0.4.0), built unmodified and captured
through the app's own `--ui-gallery` harness (real pixels, real window, real fonts) at
100/125/150/175/200 % (`docs/design/audit/before/`). Performance work lives on
`perf/e2e-speed-accuracy-v0.5.0` and is not mixed into this branch.

## 1. What the UI actually is

No web view, no framework. A native Win32 app (Rust, `windows-sys`) with a software
canvas (`vi-win32/src/canvas.rs`, premultiplied BGRA) for all painting.

| Surface | Source | Purpose |
|---|---|---|
| Floating listener (16 states) | `vi-win32/src/overlay.rs`, `pill.rs`, `ui.rs` | idle, hover/mode hint, listening (+silence), transcribing, polishing, inserted, copied, review, discarded, nothing heard, error x2, paused |
| Settings window, 9 pages + first-run (4 steps) | `vi-win32/src/settings.rs` | Get started, General, Dictation, Writing (6 modes), Output, Microphone, Speech models, Processing, Privacy, About |
| Model states | `settings.rs` | installed / in use / downloading / verifying / failed |
| Mic test states | `settings.rs` | testing, good, low, silent |
| Tray menu, tooltips, native combo/edit controls | `tray.rs`, `settings.rs` | system-drawn, intentionally left native |

Existing accessibility already present: high-contrast theme branch, reduced-motion check
(`SPI_GETCLIENTAREAANIMATION`), UI Automation names on controls, zero idle repaints
(measured by the gallery: `idle_frames_painted_in_2s = 0`, timer off).

## 2. Verdict on the existing design

It already follows the brief's own principles: restrained (no glass, glow, gradients or
blobs), one real icon family, hierarchy through type and dividers rather than nested
cards, native window chrome, a distinct dark listener capsule carrying the real logo,
toggles that print On/Off (not colour-only), an unmistakable selected writing mode
(green border + filled radio + tint).

A wholesale redesign would add risk and no value. The defects found are measurable and
specific, so the work is targeted.

## 3. Measured defects (WCAG 2.2 contrast computed from the actual token values)

| # | Defect | Measured | Target |
|---|---|---|---|
| 1 | `text3` (metadata, taglines, disabled labels, model specs) on bg / surface / sidebar / selected tint | 3.72 / 3.94 / 3.51 / 3.38 | 4.5 |
| 2 | White label on primary button and toggle fill `#059669` | 3.77 | 4.5 |
| 3 | Secondary / danger button outline `#DDE5E1` on surface | 1.21 | 3.0 (1.4.11) |
| 4 | Sidebar status chip overlaps the last nav item when client height < ~524 dp (200 % on a 1080p panel: clipped half-chip drawn over "About") | visual | none |

Not defects (checked): body / secondary text 16.9 / 6.9; warn, error, ok pairs 6.2-6.7;
listener capsule text 8.97-16.2, listener glyphs 6.9-11.4; card borders are decorative
(the cards are also separated by fill and spacing), so they stay quiet on purpose.

## 4. File classification (functional lock)

| Category | Files | Reason |
|---|---|---|
| A presentation | `crates/vi-win32/src/settings.rs` -- `Theme` token values and `control_border` field | colour tokens only |
| B mixed | `settings.rs` paint function: status-chip draw condition, secondary/danger button stroke colour | paint-only; no handler, binding, control id, hit-test or layout rectangle changed |
| C (not touched) | audio, hotkeys, ASR, SLM/rewrite, model download/verify, config, tray, installer, `vi-app`, `vi-core` | functional |

## 5. Changes (iteration 1: tokens and small-screen paint)

| Token | Before | After | Contrast after |
|---|---|---|---|
| `text3` | `#74847C` | `#5B6B63` | 4.65-5.63 on all surfaces |
| `accent_fill` (primary button, toggle ON, progress, nav marker) | `#059669` | `#047857` | white 5.48 |
| `accent` (hover fill, accent text) | `#047857` | `#065F46` | 6.6-7.3 |
| `accent_press` | `#047857` | `#064E3B` | white 9.7 |
| `control_border` (new; secondary/danger button outline) | -- (`#DDE5E1`) | `#7F8F87` | 3.14-3.40 |
| Sidebar status chip | always drawn | drawn only if it clears the nav | -- |

Hover and pressed states previously shared one colour on primary buttons; they are now
three distinct steps. The brand emerald `#10B981` is unchanged on the listener and logo;
only the light-theme fills moved to the darker emerald that already was the accent text
colour. High-contrast mode still maps to system colours (`control_border` = window text).

## 6. Not attempted, and why

- Typography, spacing scale, radii, motion: reviewed in the screenshots and found
  consistent; changing them without a defect would be decoration. The motion timings are
  already inside the brief's ranges and honour reduced motion; unchanged.
- Dark theme for settings: does not exist today; adding one is a new feature.
- Native combo boxes / edit boxes / tray menu: system-drawn on purpose (native comfort,
  keyboard and UI Automation behaviour come with them).
