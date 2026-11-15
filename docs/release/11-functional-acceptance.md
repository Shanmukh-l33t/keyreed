# Functional acceptance

Date: 2026-11-15
Verdict: Not Ready for Private Beta

The complete path was not run. This guest has no microphone. A virtual PS/2 keyboard is present. No physical key was pressed. Twenty dictation sessions with a read-back paste were not performed. Private beta is not justified.

## How to read this report

| Label | Meaning |
| --- | --- |
| Windows VM | Ran on the dockur Windows 11 guest, console session 1, 3 cores, 4 GB, build 26200 |
| Unit test | Linux `cargo test` from the earlier reliability pass. Not repeated as a physical session |
| Not tested | Not run |

Unsigned binaries, an unsigned installer, and SmartScreen remain release blockers from earlier passes. They were not cleared here.

## Physical push-to-talk

Not tested.

Capture endpoints: 0. Host audio devices are not passed into the container. `Win32_SoundDevice` returned no rows. A hold on a real microphone cannot start.

The keyboard row is `Standard PS/2 Keyboard`. That is the virtual keyboard. It was not pressed by hand. Injected keys from an earlier pass are not this test.

Lost key-up, timeout discard, cancellation, repeated sessions, focus changes during recording, and microphone disconnect were not run on hardware. The cap-discard behavior remains a unit test only (`recording_cap_discarded` does not return `StopAndProcess`).

## Complete workflow

Not tested. Required shape: physical shortcut, microphone, speech, release, transcription, correction, clipboard, Ctrl+V, then read the inserted text back. Zero of 20 sessions were completed.

A clipboard write from `keyreed.exe --transcribe-wav` is not that workflow. A later Notepad attempt used that clipboard text, focused hwnd 197452, sent Ctrl+V, and UI Automation returned an empty document. `notepad_match=False`. Paste is not confirmed.

| App | Result |
| --- | --- |
| Notepad | Window found. Inserted text read back empty. Not confirmed. Windows VM. |
| Edge | Installed. Not paste-tested this pass. |
| Chrome | Not installed. Not tested. |
| PowerShell | Not paste-tested this pass. |
| VS Code | Not installed. Not tested. |

## Second-instance notification

Verified on the Windows VM, by screenshot, not on a physical laptop.

One instance stayed up. The second exited 0. `EnumWindows` found `KeyreedTrayHostClass` (hidden) and `KeyreedOverlayPillClass` (visible). After the second launch, the desktop showed a balloon from `keyreed.exe`:

Keyreed
Keyreed is already running. Use the tray icon. A second copy was not started.

Screenshot: `vm/shared/accept_after.png`. The earlier `FindWindow` miss is not the result. The balloon is visible.

## Spoken symbols

Verified as a text pipeline on the Windows exe. Not spoken into a microphone. Not triggered by focusing a terminal.

Command: `keyreed.exe --eval-correction-jsonl` with `--llm-model none`. Three cases.

| Id | Mode | Output |
| --- | --- | --- |
| sym_term | terminal | `cd / var _ name [ ]` |
| sym_smart | smart | `Cd forward slash var underscore name open bracket close bracket.` |
| sym_pro | professional | `Please use a forward slash in the path.` |

Terminal mode produced slash, underscore, and brackets. Smart and professional kept the words. That does not prove the foreground app is what selected the mode. Context detection during a live recording was not tested.

## Security interruptions

Windows VM, this pass: killed `keyreed.exe --download-model moonshine-tiny-en-int8` after `.staging\...\cached_decode.int8.onnx` reached 45,264,830 bytes and before the active folder existed. `final_exists=False`. `staging_exists=True` at the kill. That is after the staged file was complete and before promotion. It is not a stack sample from inside the hash function, and it is not a kill during the rename. Promotion mid-rename was not hit. Restart after an interrupted promotion was not run as its own case.

An earlier pass killed during a partial extract and, after the sweep fix, a restart removed staging and did not activate the folder. That remains an earlier pass.

Compile-time pins were not changed.

## Performance and memory

Shortcut-to-microphone latency: not measured. No microphone.

Transcription, correction, and clipboard-ready times for a physical session: not measured.

Prior VM pipeline numbers in `08-remediation.md` and `09-windows-ux.md` are earlier runs on this same guest, not this acceptance sprint, and not a physical workflow. Do not reuse them as this sprint's result.

## Release blockers still open

- No physical dictation and paste loop.
- Unsigned `keyreed.exe`, runtimes, and `KeyreedSetup.exe`.
- SmartScreen not observed.
- 4-core, 8 GB Windows not measured.
- Installer upgrade and login-after-restart not repeated here.
- Kill during directory rename not achieved.

## Verdict

Not Ready for Private Beta.
