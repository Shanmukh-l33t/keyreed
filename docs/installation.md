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
| `KeyReed-Setup-x64.exe` | You want the normal install. **Start here.** | Lightweight `24.4 MB` user-scoped installer (`%LOCALAPPDATA%\Programs\KeyReed`). Downloads speech models on demand in Settings with SHA-256 verification. |
| `keyreed-2.1.1-windows-x86_64-portable.zip` | You want a portable no-installer folder. | `46.2 MB` archive containing `keyreed.exe`, native speech/grammar runtimes, and `silero_vad.onnx`. |
| `gector-bert-en-int8.tar.bz2` | Settings downloads it for you (Quick Fix grammar model). | `73.1 MB` archive of the Apache-2.0 `gector-bert-en` model (int8) with its LICENSE and NOTICE, pinned by SHA-256 in the app. |
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

For release 2.1.1:

```text
6abae134861325cf84f99e927081e5b8dfebb47a67193179923c0fe17d516ca5  KeyReed-Setup-x64.exe
dd4bc184afdb228a86374be709cbed4fa4067aba351b761b50eb4adaea81ce09  keyreed-2.1.1-windows-x86_64-portable.zip
5654cefc549c938bbac8770d9753a8be50e6d66f9afdc6d7ec746f3d51590ae0  gector-bert-en-int8.tar.bz2
```

A SmartScreen prompt on an unsigned file is expected. It is not a verdict on the file, but it is a good reason to compare the hash or build from source ([development guide](development.md)).

## Other speech models

Five more models are in the catalog and none download unless you ask:

```powershell
keyreed.exe --list-models
keyreed.exe --download-model parakeet-tdt-0.6b-v3-int8
```

Available ids: `parakeet-tdt-0.6b-v2-int8` (default), `parakeet-tdt-0.6b-v3-int8`, `parakeet-tdt-0.6b-ultra-int8`, `parakeet-tdt-110m-en-int8`, `moonshine-base-en-int8`, `moonshine-tiny-en-int8`, `sense-voice-small-int8`, `whisper-small.en-int8`.

## Uninstall

Use Settings, Apps, KeyReed, or the Start Menu entry "Uninstall KeyReed". Uninstall removes the program files, including models the installer placed in the install folder. It does not delete `%LOCALAPPDATA%\Keyreed`, which holds your settings.
