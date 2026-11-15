# Real Windows validation

Date: 2026-11-15
Verdict: Not ready for private beta

The acceptance kit exists. It has not been run on a physical Windows desktop. No microphone session, no physical key press, and no read-back paste from spoken dictation was completed. Building the kit is not a pass.

## Lanes

Lane A is this Linux host and the Windows VM. It can compile, hash, and run checks that do not need a microphone. Lane B is the physical Windows desktop. Only Lane B can produce the dictation evidence. VM rows are not physical results.

## Kit

Package: `dist/KeyreedAcceptance.zip`
Size: 3,215,448 bytes
SHA-256: `fdd912d31ee45f804df9f32a40b2f5a4668ea2b3745f6ff5e2d2223385353be9`

The zip contains the launcher, twenty spoken prompts, the development executable, and the VC++ runtime DLLs that sit beside that executable. It does not contain the speech models or the sherpa and llama runtimes. Those stay in the full Keyreed folder. The launcher does not install a compiler and does not upload audio.

Command, on the physical PC, in a normal PowerShell window:

powershell -NoProfile -ExecutionPolicy Bypass -File .\Start-KeyreedAcceptance.ps1 -KeyreedRoot C:\Keyreed

Logs stay in `%LOCALAPPDATA%\KeyreedAcceptance`. Copy that folder back only if you want the results recorded here. The kit does not send them.

## What the kit checks

| Phase | What it does | Who must do it |
| --- | --- | --- |
| Hardware | Windows version, CPU, RAM, capture endpoints, keyboard, microphone consent | The kit, on the target PC |
| Paste | Known text, clipboard read-back, Notepad focus, Ctrl+V, save, file read-back | The kit, in an interactive desktop session |
| Microphone | waveIn open, init time, short buffer, peak level, then the buffer is wiped | The kit, only if a capture device exists |
| Context | You focus each app. The kit runs `keyreed --print-foreground` | You, at the keyboard |
| Physical shortcut | You hold Right Ctrl. The kit does not inject a key | You |
| Lost key-up | You hold until the discard toast and type YES | You |
| 20 sessions | You speak each prompt. The kit records clipboard, saved file, and your yes or no | You |

Injected keys are not a physical pass. A text rewrite eval is not speech. A clipboard API write is not a paste. The paste row passes only when the saved Notepad file matches.

## Smoke test on the VM, this pass

This is Lane A. It is not Lane B.

`keyreed.exe --version` printed `Keyreed 0.1.0`.

`keyreed.exe --print-foreground` returned category General, empty process name, recommended mode Smart. `microphone_started` was false. `hotkey_hook_started` was false. The SSH session has no foreground window, so this does not validate Notepad, Edge, PowerShell, or VS Code.

The hardware phase, run from SSH, measured Windows 11 Pro build 26200, AMD EPYC 9655, 3 logical cores, 4 GB RAM, 2014.3 MB free. Capture endpoints failed: none. Keyboard row was Standard PS/2 Keyboard. Microphone consent was Allow. The interactive-session row failed, which is correct for SSH. Physical rows must not pass from that session.

## Context mapping the kit will check

This is the current classifier, not a wish.

| Focused process | Category | Recommended mode | Spoken symbols |
| --- | --- | --- | --- |
| powershell.exe, WindowsTerminal.exe, cmd.exe | Terminal | TerminalCode | Yes, if a later spoken session uses that mode |
| Code.exe and the other listed editors | CodeEditor | Technical | No |
| notepad.exe, WINWORD.EXE | Document | Smart | No |
| msedge.exe, chrome.exe, unless the title matches a mail or chat hint | Browser | Smart, the fallback | No |

Code editors do not enter Terminal / Code under automatic context. The earlier symbol check used a manually selected mode. That still does not prove the focused window selected the mode. The kit's context phase is the check. It has not been run on the physical desktop.

## Clipboard failure from the previous VM pass

Notepad was focused and Ctrl+V was sent from a scheduled task. UI Automation read back an empty document. That is not a confirmed paste. The kit does not trust that read-back. It saves the Notepad file and compares the file. That method has not been run on the physical desktop, so paste remains not confirmed.

## Results by class

| Item | Class |
| --- | --- |
| Kit built, hashed, and launched for a hardware inventory over SSH | Measured on the Windows VM |
| `--print-foreground` starts no mic and no hook | Measured on the Windows VM, empty foreground |
| Physical hold, release, toggle, cancel, lost key-up, mic disconnect | Not tested |
| 20 spoken sessions with file read-back | Not tested. Zero of 20 |
| Notepad, Edge, Chrome, PowerShell, VS Code, Word context rows | Not tested |
| Shortcut-to-microphone latency, transcription latency, correction latency, clipboard-ready latency | Not tested on physical hardware |
| Parent and child memory during a spoken session | Not tested |
| Code signing and SmartScreen | Still unsigned. Still a release blocker |
| Installer upgrade and login after restart | Not repeated in this pass |

## Blockers

- No physical dictation loop.
- No confirmed paste from a saved Notepad file on the physical desktop.
- Unsigned executable, runtimes, and installer.
- SmartScreen not observed.
- 4-core, 8 GB Windows not measured. The VM figure above is 3 cores and 4 GB.
- Kill during directory rename was not achieved in the earlier security pass.

## Verdict

Not ready for private beta.
