# KeyReed

Voice it. Everywhere.

KeyReed is a system-wide Windows voice input application. Hold a shortcut, speak, release, and paste the corrected text into the application you were using.

The Windows build in this release is unsigned. Windows SmartScreen may warn before you run it. Signing does not by itself create SmartScreen reputation, and this release has not been observed in SmartScreen.

## Download

| File | Size | SHA-256 |
| --- | --- | --- |
| [KeyReed-Setup-x64.exe](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.1.1/KeyReed-Setup-x64.exe) | 32,600,120 bytes | `cba942e7e6e7236f6201ca9e8f77fd9e9798bfb66d8de3c235bb4355fa46ab10` |
| [KeyReed-Full-x64.zip](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.1.1/KeyReed-Full-x64.zip) | 1,041,930,575 bytes | `63767c3cf0a46561ae84ad5a49785a29852ec7d00af1d2fbc7f75e2bc872fcc5` |

The setup file is the small installer. It does not include speech models. After launch, use the tray menu item Download default speech model. The zip is the full offline package, with the default speech model and the default grammar model. Checksums are also in [SHA256SUMS.txt](https://github.com/Shanmukh-l33t/keyreed/releases/download/v0.1.1/SHA256SUMS.txt).

## Install

1. Download `KeyReed-Setup-x64.exe`.
2. Double-click it. Windows may show an unknown-publisher prompt because the file is unsigned.
3. Choose the install folder. The default is `C:\Program Files\KeyReed`.
4. Finish. Leave "Launch KeyReed" checked if you want it to start now.
5. KeyReed stays in the system tray. A Start Menu shortcut is created. A desktop shortcut is optional.

You do not need Rust, Python, Visual Studio, or a command line.

## Uninstall

Use Settings, Apps, KeyReed, or the Start Menu entry "Uninstall KeyReed".

Uninstall removes the program files, including the models that the installer placed under the install folder. It does not delete `%LOCALAPPDATA%\Keyreed`, which holds your settings.

## First use

1. Launch KeyReed from the Start Menu. It runs from the tray.
2. Open Settings from the tray icon.
3. Choose a microphone and confirm the shortcut. The default shortcut is Right Ctrl, hold to talk.
4. Hold the shortcut, speak, and release.
5. When the tray says the text is ready, click the place you want it and press Ctrl+V.

The installer does not include the speech models. Open the tray menu and choose Download default speech model. That download checks the compile-time size and SHA-256 pins before the model is activated. The other five speech models stay available through `keyreed.exe --download-model <id>`. None of them download unless you ask.

Startup with Windows is off unless you turn it on in Settings.

## What it does

- Global hold-to-talk, and toggle-to-talk if you switch the activation mode.
- Six writing modes: Smart, Exact, Professional, Casual, Technical, and Terminal / Code.
- Grammar correction after transcription. Tier 0 is local and deterministic. A local model can run after that when it is installed.
- Clipboard is the default output. You paste with Ctrl+V. Direct insertion is a separate output setting.
- Local processing is the default. Cloud processing is optional and is not used by the offline profile.
- Microphone selection, custom vocabulary, and configurable shortcuts are in Settings.
- A second launch does not start another listener. It asks the running copy to show a tray notice.

Spoken punctuation such as "forward slash" is applied in Terminal / Code mode, not in Smart or Professional mode. Focusing a code editor does not by itself select Terminal / Code. Code editors are classified as Technical.

## Hardware

Packaged and launched on 64-bit Windows 11. Windows 10 is an intended target and was not re-tested for this package.

The packaging machine's Windows guest had 3 logical cores and 4 GB of RAM. That is a measurement of the build guest, not a claim that 4 GB is comfortable, and not a measurement of an 8 GB computer. A machine with more free memory than that guest is a safer choice. No dedicated GPU is required for the packaged CPU build.

## Privacy

- The microphone is opened when a recording starts, and closed when the recording stops, is cancelled, or hits the time limit.
- A lost key release, or the 120 second cap, discards the audio. That path does not transcribe and does not change the clipboard.
- Audio is kept in memory for the recording. It is not written to a file by the normal dictation path.
- The default profile does not send audio or text to a cloud service.
- A cloud profile uses the endpoint you configure, and only if you have stored a key. Do not describe the product as "nothing ever leaves the machine" if that profile is on.
- Model files are checked against compile-time sizes and SHA-256 values before they are loaded. A partial download is not activated.
- The clipboard receives the finished text and is left there for you to paste. KeyReed does not restore the previous clipboard contents after dictation.

## Troubleshooting

Microphone missing. Windows is reporting no capture device, or the app has no permission. Settings, Privacy, Microphone, allow desktop apps. Then pick the device in KeyReed Settings.

Shortcut does nothing. Another program may own Right Ctrl. Change the shortcut in Settings.

Model will not load. Do not copy a half-extracted model folder into place. Use the installer models, or `keyreed.exe --download-model`. A failed install is quarantined and is not loaded.

Paste is empty. KeyReed copies the finished text. Click the target application first, then Ctrl+V. If the target ignores the clipboard, that is a target-application issue, not proof that every Windows app accepts the paste.

Offline. The default models are already in the install folder. Cloud options stay off unless you switch the processing profile and add a key.

## Licence

Source is dual licensed under MIT or Apache-2.0, as declared in `Cargo.toml`. See `LICENSE-MIT` and `LICENSE-APACHE`.

## Known limits

- The Windows executable, bundled runtimes, and installer are unsigned.
- SmartScreen behaviour for this download has not been observed.
- A physical microphone dictation loop on a desktop outside the packaging guest has not been recorded in this repository.
- Hindi transcription accuracy has not been scored.
- Do not treat one clip, or a text-only rewrite check, as a word error rate.
