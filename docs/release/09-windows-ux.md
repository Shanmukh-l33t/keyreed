# Windows user-experience validation

Date: 2026-11-15
Verdict: not ready for distribution.

A private beta is not recommended yet. Hold-to-talk reaches the hook, then stops because this guest has no microphone. Paste into applications was not confirmed. The binaries and the installer are unsigned.

## What was run

Guest: Windows 11 Pro build 26200, console session 1, user tester, 3 cores, 4 GB. No capture endpoints (`capture_keys=0`). Word, Chrome, and VS Code are not installed. Edge is installed and was not paste-tested. Reboot is blocked on this host, so login-after-restart was not run.

The hook test used `SendInput` of Right Ctrl from a scheduled task in the console session. That is an injected key, not a physical keyboard. The hook ignores only events tagged with the app's synthetic tag. These events were untagged. `SendInput` returned 1 after the struct size was corrected to 40 bytes.

## Push-to-talk

Hook install was logged. Three hold cycles and two toggle presses were seen by `WH_KEYBOARD_LL`.

Shortcut detection, inject timestamp to `hook_ptt_down`:

| Cycle | Delta ms |
| --- | --- |
| hold 1 | 16 |
| hold 2 | 1 |
| hold 3 | 1 |
| toggle press 1 | 4 |
| toggle press 2 | 1 |

Release, inject up to `hook_ptt_up`: 3 ms, 1 ms, 1 ms on the three hold cycles.

Microphone: `mic_started` never appeared. `mic_error` appeared 46 ms, 2 ms, and 5 ms after the hook on the three hold cycles. Toggle press 1 reached `mic_error` 32 ms after the hook. There is no capture device, so microphone-open latency, recording start, recording stop, and speech-from-the-mic were not measured. Do not treat the error time as a successful open.

Because the microphone failed immediately, toggle-to-talk never had a live recording to stop. The second press started another attempt. Toggle stop is not validated.

Grammar correction, clipboard update, and Ctrl+V did not run on the push-to-talk path. Total push-to-talk time to clipboard readiness was not measured. The chain stopped at the microphone.

Cancellation during a live recording was not tested. There was no recording to cancel.

Repeated activation: three holds and two toggle presses all reached the hook. No missed inject after the struct fix (`sent=1`).

A second `keyreed.exe` exited while the first stayed up. One instance remained. That was measured in the console session.

Failed recording did not wipe the clipboard. Value before the failed hold: `SENTINEL_BEFORE_FAILED_PTT`. Value after: the same string. `sentinel_kept=True`.

## Clipboard and applications

In the console session, `keyreed.exe --transcribe-wav` with `--copy-clipboard` and exact mode put this on the clipboard:

`I know like Um like my parents bought me a lot of books so I used to always read a lot.`

That is one English sentence from `ind_000.wav`. It is not a long dictation, not a unicode sample, and not a technical-term sample.

| Target | Result |
| --- | --- |
| Windows clipboard, session 1 | Text present. Measured. |
| Notepad | Window found (hwnd 524882). UI Automation text after Ctrl+V was empty. Paste not confirmed. |
| WordPad | No window handle. Not confirmed. |
| Microsoft Word | Not installed. Not tested. |
| Chrome | Not installed. Not tested. |
| Edge | Installed. Not tested. |
| VS Code | Not installed. Not tested. |
| PowerShell | Not paste-tested. |
| Windows Terminal | Present (`wt.exe`). Not paste-tested. |
| Direct typing | Not tested. |

No universal compatibility claim.

## Model interruption

Kill of `keyreed.exe --download-model moonshine-tiny-en-int8` while `.staging` existed. The archive on disk was already 107,600,538 bytes, so the kill was during extraction, not during the network read. A partial `cached_decode.int8.onnx` was in staging (26,992,045 bytes, then 3,599,656 bytes on a second kill). `final_exists=False` both times. The incomplete tree was not activated.

First restart, before a sweep fix, left `.staging` in place and still did not activate it. `ensure_asr_loaded` returned before sweep on the offline missing-model path. That path now calls `sweep_incomplete` first.

After that fix, same kill, then restart: `staging_after_restart=False`, `incoming_part=False`, `final_after=False`, exit 1, error that the default model is not installed and the offline profile will not download it. Cleanup on restart held. Verification and promotion were not hit by a kill. Those windows are shorter than the poll. Not passed.

The six catalog pins were not changed. Integrity unit tests: 10 passed in this pass, including the real extracted-archive hash check.

## Installation and startup

Silent install and uninstall were measured in the previous pass, not repeated here. This pass confirmed `KeyreedSetup.exe` has no embedded signature (`osslsigncode`: no signature, PE checksum field zero).

Interactive installation was not completed. Upgrade was not run. Windows restart was not run. The host blocks reboot from this agent.

Startup disabled remains the default. The product Run value was absent in the earlier same-day readback. Startup enabled across a reboot was not tested.

## Memory

One smart-mode transcribe, parent and `llama-server` sampled together. Working set and private bytes are both below. CPU column is process CPU time in seconds, not a utilization percent.

| Sample | keyreed WS MB | keyreed private MB | llama-server WS MB | llama-server private MB |
| --- | --- | --- | --- | --- |
| 0 | 2.2 | 0.4 | absent |  |
| 1 | 11.1 | 3.7 | absent |  |
| 5 | 752.7 | 758.3 | absent |  |
| 9 | 754.4 | 758.8 | 882.2 | 615.9 |

At sample 9 both were alive. Combined working set 1636.6 MB. Combined private bytes 1374.7 MB. That is one sample, not a traced peak. It is a 4 GB guest. It is not an 8 GB result.

Idle tray before model load was about 9 to 11 MB in earlier samples this session. Recording memory was not measured. No microphone.

The ten-run latency table in `08-remediation.md` is still the latency evidence. This pass did not repeat those ten runs. Warm rewrite median 880.8 ms and p95 1077.1 ms remain that prior measurement on this same guest.

## Accuracy

No new inference pass. Do not treat the earlier Linux rescore as a new Windows accuracy result. Hindi remains unmeasured. Technical errors in the stored hypotheses (for example `PostgreSQL` heard as `post-grey SQL`) are unchanged. No model was swapped to buy speed.

## Signing

No production certificate. No self-signed certificate. `scripts/sign-win.sh gate` still fails closed. The installer is unsigned. SmartScreen was not shown. Signature verification, which has not succeeded, would still not prove SmartScreen reputation.

## Known limits

- This guest cannot record. Push-to-talk cannot be called complete here.
- Paste into Notepad, Word, Chrome, Edge, VS Code, PowerShell, and Windows Terminal is not confirmed.
- 4-core, 8 GB Windows was not measured.
- Login after restart was not measured.
- Kill during hash verification and during folder rename was not achieved.
- Public distribution of the unsigned setup is not acceptable.
