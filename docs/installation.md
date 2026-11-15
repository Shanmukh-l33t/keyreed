# Installation

## Requirements

- 64-bit Windows 11 (Windows 10 is an intended target and was not re-tested for 0.1.1).
- A microphone that desktop apps are allowed to use.
- About 33 MB for the installer and about 460 MB for the default speech model.

No Rust, Python, Visual Studio or command line is needed.

## Choose a download

Both files are on the [latest release](https://github.com/Shanmukh-l33t/keyreed/releases/latest).

| File | Use it when | Notes |
| --- | --- | --- |
| `KeyReed-Setup-x64.exe` | You want the normal install. **Start here.** | Small installer. It does not include speech models; you download the default one once. |
| `KeyReed-Full-x64.zip` | You want everything offline in one download. | About 1 GB. Contains the default speech model and the default grammar model. Extract and run `keyreed.exe`. |
| `SHA256SUMS.txt` | You want to verify a download. | Checksums for the files above. |

## Install

1. Run `KeyReed-Setup-x64.exe`. Windows may show an unknown-publisher prompt because the file is unsigned.
2. Choose the install folder. The default is `C:\Program Files\KeyReed`.
3. Finish. Leave "Launch KeyReed" checked to start it now. KeyReed lives in the system tray, and a Start Menu shortcut is created. A desktop shortcut is optional.
4. From the tray menu choose **Download default speech model**. The download is HTTPS and is checked against pinned size and SHA-256 values before it is activated.
5. Open **Settings** from the tray icon, pick your microphone and confirm the shortcut.

## First use

1. Hold **Right Ctrl**, speak, release.
2. When the tray says the text is ready, click where you want it and press **Ctrl+V**.

Auto Insert (typing into the focused field) is off until you turn it on in the tray menu or Settings. Startup with Windows is also off by default.

## Verify your download

The Windows files are not code-signed, so check the hash against the one published on the release page or in `SHA256SUMS.txt`:

```powershell
Get-FileHash .\KeyReed-Setup-x64.exe -Algorithm SHA256
```

For release 0.1.1:

```text
cba942e7e6e7236f6201ca9e8f77fd9e9798bfb66d8de3c235bb4355fa46ab10  KeyReed-Setup-x64.exe
63767c3cf0a46561ae84ad5a49785a29852ec7d00af1d2fbc7f75e2bc872fcc5  KeyReed-Full-x64.zip
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
