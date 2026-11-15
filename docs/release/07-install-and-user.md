# Install and use

There is no installer. These steps copy a folder. They are what was used on the test guest (`C:\Keyreed`). They are not a finished setup program.

## What you need

- Windows 10 or Windows 11, 64-bit.
- A microphone the user has allowed for desktop apps (Settings, Privacy, Microphone).
- Disk: about 1.3 GB for the default bundle, or about 80 MB if you later split models out. The split installer does not exist yet.
- RAM: the 4 GB test guest completed a run while `keyreed.exe` held about 755 MB and `llama-server.exe` held about 882 MB at the same sample. That is tight. This project has not measured an 8 GB PC.

## Files

Build or take `dist/keyreed-win-x64/` from `scripts/package-win.sh`. Check `CHECKSUMS.sha256` before you copy it anywhere else.

Copy the folder to something like `C:\Keyreed`. The exe looks next to itself for `runtime\sherpa-onnx`, `runtime\llama`, and `models`.

Start `keyreed.exe`. A tray process came up in the audit (working set about 9 MB before any model load). First dictation loads the speech model. That load is the slow part.

Optional models, still in the app, not all in the default folder:

```
keyreed.exe --download-model parakeet-tdt-0.6b-v3-int8
keyreed.exe --download-model moonshine-base-en-int8
keyreed.exe --download-model moonshine-tiny-en-int8
keyreed.exe --download-model sense-voice-small-int8
keyreed.exe --download-model whisper-small.en-int8
```

Downloads are HTTPS and SHA-256 pinned. If a download is interrupted, delete the model folder before you retry. The app currently treats an existing folder as ready and will not re-check the hash.

## Defaults

- Hold Right Ctrl to talk. Release to finish.
- Esc cancels.
- Ctrl+Shift+Space cycles writing mode.
- Output is the clipboard. Paste with Ctrl+V. The app does not press Ctrl+V for you.
- Writing mode starts at Smart.
- Processing starts at Maximum Privacy (offline). Cloud is off unless you change that and store a key.
- History is off unless you turn it on.

Direct typing is a setting. Clipboard stays the default because it does not depend on the focused window accepting fake keystrokes.

## Remove

Delete the folder. Delete `%APPDATA%` data for Keyreed if a config file was written (the default data dir is under the user profile; this audit did not print that path from a live session). If a Run key named `Keyreed` exists under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, delete that value. It was absent on the test guest at audit time.

## Not done

- No Start Menu shortcut is created by a setup program.
- No Windows restart test.
- No signed binary. Windows may warn if you download this folder from the internet. That warning was not captured in this audit. Do not ship this folder as a public download until it is signed. See `docs/release/05-code-signing.md`.
