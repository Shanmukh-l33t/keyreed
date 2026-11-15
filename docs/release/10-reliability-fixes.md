# Reliability fixes after the WritHer comparison

Date: 2026-11-15

Clipboard snapshot and sequence-number restore was not implemented. KeyReed still copies the corrected text and leaves it for Ctrl+V. WritHer's restore path is a possible future option for an explicit auto-paste mode only. It is not in this release.

## Lost key-up

`PttController::on_tick` no longer returns `StopAndProcess` when the recording cap is reached. It returns `CancelRecording` with reason `recording_cap_discarded`, clears the hold latch, and sets the state to idle. The coordinator drops the microphone buffer with `cancel_and_zeroize` and does not call speech recognition, grammar correction, or the clipboard. A tray balloon says the recording was discarded and that the shortcut can be held again.

A late key-up after that discard is ignored. A floating-mic session started after the discard is not stopped by that late key-up. OS key-repeat does not start a second session. Hold and toggle both discard at the cap. A normal release before the cap still processes.

This is a unit-tested state machine. It was not replayed as a lost physical key-up on Windows in this pass.

## Second instance

A second process finds `KeyreedTrayHostClass` and posts `WM_USER+43`. The running instance shows a fixed balloon: "Keyreed is already running. Use the tray icon. A second copy was not started." No string is sent across processes. Settings are not forced open. The second process does not start a hook or a microphone. It exits.

The balloon was not visually confirmed. A console-session test started one instance, started a second, and observed the second exit 0 with one process left. `FindWindow` of `KeyreedTrayHostClass` returned 0 in that same test, so the post to the tray window was not proven.

## Spoken symbols

Only `WritingMode::TerminalCode` runs the symbol map. That is the Terminal / Code profile. Smart, Exact, Professional, Casual, and Technical do not.

Phrases: "forward slash", "backslash", "back slash", "underscore", "open bracket", "close bracket", "open square bracket", "close square bracket". Bare "slash", "dash", and "one" are not replaced.

The map runs before Tier 0 as well as after it, only in that mode. Tier 0 already treats the single word "slash" as a path token, so "forward slash" would be split before an after-only pass could see it. The after pass still runs for phrases Tier 0 leaves intact.

## Tests

`cargo test --workspace --offline`: 40 passed, 0 failed. New cases cover the cap discard, toggle cap discard, stale key-up versus a newer floating-mic session, duplicate key-down, symbol phrases, and TerminalCode versus Smart.

Windows integration of a physical lost key-up was not run. A second-instance test on the guest showed one process remaining and the second exiting 0. The tray-window notification was not confirmed.
