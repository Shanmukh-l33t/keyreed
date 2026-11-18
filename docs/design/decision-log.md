# Design v3 decision log

Each entry: the choice, the alternatives, why. "Hex" = Hex by Anomaly (reference B), studied from a shallow clone
outside this repo; nothing of its code or assets is vendored.

**D1. The listening pill stays at 60 % (`LISTENING_SCALE`).** The brief allows growth to about 112 dp. 2.1 shrank
the open-mic pill on purpose so it covers less of the user's work, so v3 keeps it and turns the shrink into the
brand metaphor instead: on key-down the capsule *sinks* to 60 % on `MORPH` while the keycap presses in, and on
key-up it springs back to full height for the text lines. Alternative (grow to 112 dp for bigger bars) rejected:
it undoes a recent owner decision for a little more waveform.

**D2. Working is mint and neutral, not teal.** The brief says Transcribing reads amber; it has been teal
(`#2DD4BF`) since 2.0. Teal is outside the brand's hue families, so the working edge is mint at 35 %, the label is
text2, and the sheen is mint (emerald while polishing). In Settings `working` becomes brand `#0F5A3F` (light) and
mint (dark). Amber stays "needs you". The pill test was renamed `working_is_mint_and_distinct_from_attention`
and now checks the hue the design means (amber is far redder); the old blue-channel check would tie at 35 % mint.

**D3. Text lines take a 10 dp slot.** Three strokes (100 / 76 / 52 %, 1.75 dp, 3.5 dp pitch) plus the 6 dp gap add
exactly the +16 dp the brief allows to Processing. A 12 dp slot (the check's size) would exceed the budget by 2 dp.

**D4. Centre-out history (Hex Dot scope).** The newest level is the middle bar and older ones travel outwards on
both sides, so 11 bars show the last 6 samples mirrored (about 200 ms at the 30 Hz push). It reads as voice
coming *out* of the reed. Kept the old left-to-right? No: it reads like a recorder, and centre-out keeps the
waveform symmetric around the mark's axis. Phosphor tail 3 % per step (outermost 85 %): stronger dimming broke
`listening_bars_stay_separate_at_sixty_percent`, whose ink threshold guards against merged bars at 60 %.

**D5. The panel starts under the header.** Hex/OpenCode insets the panel ~8 dp from the window top; KeyReed's
52 dp header carries the brand and the window buttons, so the panel starts at the header's bottom edge and is
inset 8 dp on the right and bottom only. `X0` / `CW` are unchanged, so the layout tests and scroll extents are
unchanged; the focused-control reveal accounts for the 8 dp inset.

**D6. Depth strategy per theme.** Light: the panel gets the website's `--card-shadow`, cards only its first layer
and no border (tone + a whisper of lift). Dark: no shadows (`--card-shadow: none` on the site), tone steps plus a
hairline equal to `--line` composited. High contrast: system colours, outlines, no shadows.

**D7. Eyebrow section labels.** 11 px semibold Segoe UI Variable, uppercase at paint time, +1 px tracking,
`text3` (5.0-7.4:1). The words in the layout model (and anything a test or screen reader reads) stay sentence case.
GDI tracking is whole pixels, so +1 px is the smallest step at 100 %.

**D8. No negative display tracking yet.** GDI's `SetTextCharacterExtra` is integer pixels: -0.4 px rounds to 0 at
100 % and -1 at 200 %, which would make titles differ by scale. Revisit with the font gate (D9).

**D9. Fonts gated, not shipped yet.** Geist / Geist Mono / Instrument Serif need static TTFs embedded with
`AddFontMemResourceEx` and a ClearType comparison at 12-28 px x 100-200 %. Not run in Phase 1. Until then:
Segoe UI Variable everywhere, no serif accent.

**D10. Accent text in dark is `#6EE7B7`.** The old `#5EE0B0` is not in the brand palette; emerald-300
(`--serif-c` on the site) is, and passes 9.4:1 on the selected tint.

**D11. Panel shadow is analytic and paints only outside the panel.** A CSS-style layer `(y, blur, spread, alpha)`
becomes a smoothstep falloff over +-0.75 blur from the spread-adjusted rounded rect; pixels inside the shape are
skipped because the panel covers them. Cost scales with the visible band, not the panel area.

**D12. Motion core is closed-form.** Springs are solved analytically per sample (under-, critically and
over-damped), so a value is a pure function of `(x0, v0, target, t)`, frame drops cost nothing, and the gallery can
render any instant. Same mapping as Hex (SwiftUI duration + bounce); the derivation is KeyReed's own.

**D13. Composer is connected in 2.1.** The brief says it is not; it is (Ctrl + Win + K, tray, pill click). Scope
stays "restyle and retune motion only".

**D14. Baseline perf is over budget at 200 %, and that sets Phase 2.** Pill listening frame p95 4.6 ms (budget
2.0), frame interval p95 37 ms (budget 20), Settings full repaint p95 111 ms, Review panel p95 70 ms. Causes seen in
code: a new DIB and DC per pill frame (`present_layered`), a full-window `Canvas::new` per Settings paint, a
`px.clone()` per pill frame, text masks cloned per frame, and `WM_TIMER` pacing. Phase 2 order: frame clock,
persistent buffers, dirty rects, then measure again.

**D15. Unexplained pill repaints while Settings is used.** 55 pill frames at about 61 ms intervals during the
Settings switch test, with no pill change. To trace in Phase 2 (silent-at-rest rule).
