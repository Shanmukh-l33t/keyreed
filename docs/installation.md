# Installation

## Requirements

- 64-bit Windows 11 (verified end-to-end on Windows 11 x64; Windows 10 x64 is supported by the Win32/WASAPI architecture).
- A microphone that desktop apps are allowed to use.
- About 18 MB for the standard installer and about 460 MB for the default speech model (`Parakeet TDT 0.6B v2`).

No Rust, Python, Visual Studio or command line is needed.

## Choose a download

Both files are on the [latest release](https://github.com/Shanmukh-l33t/keyreed/releases/latest).

| File | Use it when | Notes |
| --- | --- | --- |
| `KeyReed-Setup-x64.exe` | You want the normal install. **Start here.** | Lightweight `17.9 MB` user-scoped installer (`%LOCALAPPDATA%\Programs\KeyReed`). Downloads speech models on demand in Settings with SHA-256 verification. |
| `keyreed-0.4.0-windows-x86_64-portable.zip` | You want a portable no-installer folder. | `30.9 MB` archive containing `keyreed.exe`, native speech/grammar runtimes, and `silero_vad.onnx`. |
| `keyreed-0.4.0-windows-x64-full-offline.zip` | You want everything offline in one download. | `1.56 GB` archive containing `keyreed.exe`, runtimes, `Parakeet TDT 0.6B v2`, `Moonshine Base`, `Moonshine Tiny`, and `Qwen3.5-0.8B-Q4_K_M`. Extract and run `keyreed.exe`. |
| `SHA256SUMS.txt` | You want to verify a download. | Checksums for the files above. |

## Install

1. Run `KeyReed-Setup-x64.exe`. Windows may show an unknown-publisher prompt because the file is unsigned.
2. Choose the install folder. The default is `%LOCALAPPDATA%\Programs\KeyReed`.
3. Finish. Leave "Launch KeyReed" checked to start it now. KeyReed lives in the system tray, and a Start Menu shortcut is created. A desktop shortcut is optional.
4. On first launch, KeyReed opens **Settings** automatically if the default speech model is not installed yet. Click **Download** next to **Local Speech Engine** (or choose **Download default speech model** from the tray menu). The download is HTTPS and is checked against pinned size and SHA-256 values before it is activated.
5. In **Settings**, pick your microphone, click **Test Mic** to verify your input level, and confirm your shortcut.

## First use

1. Hold **Right Ctrl**, speak, release.
2. By default (**Auto Insert**), KeyReed types the finished text directly into the focused window (falling back to the clipboard for password fields, elevated windows, or unfocused targets). You can switch to **Clipboard Only** or **Ask Before Inserting** at any time in Settings or the tray menu.

Startup with Windows is off by default.

## Verify your download

The Windows files are not code-signed, so check the hash against the one published on the release page or in `SHA256SUMS.txt`:

```powershell
Get-FileHash .\KeyReed-Setup-x64.exe -Algorithm SHA256
```

For release 0.4.0:

```text
f81da5b3530d7581d01f3f41fdda41231b4f2a68d7e22e2b4b291556d7722812  KeyReed-Setup-x64.exe
a21dc95cac2b4038ee3738f28c2875c642977160745f6a280f6b3b4ac474460f  keyreed-0.4.0-windows-x86_64-portable.zip
a75ba1b69c752b37db517689b97d1e7279bb138bfe96b905464415680aa0f97e  keyreed-0.4.0-windows-x64-full-offline.zip
```

A SmartScreen prompt on an unsigned file is expected. It is not a verdict on the file, but it is a good reason to compare the hash or build from source ([development guide](development.md)).

## Other speech models

Five more models are in the catalog and none download unless you ask:

```powershell
keyreed.exe --list-models
keyreed.exe --download-model parakeet-tdt-0.6b-v3-int8
```

Available ids: `parakeet-tdt-0.6b-v2-int8` (default), `parakeet-tdt-0.6b-v3-int8`, `moonshine-base-en-int8`, `moonshine-tiny-en-int8`, `sense-voice-small-int8`, `whisper-small.en-int8`.

## Uninstall

Use Settings, Apps, KeyReed, or the Start Menu entry "Uninstall KeyReed". Uninstall removes the program files, including models the installer placed in the install folder. It does not delete `%LOCALAPPDATA%\Keyreed`, which holds your settings.
