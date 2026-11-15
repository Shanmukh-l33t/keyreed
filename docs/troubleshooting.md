# Troubleshooting

**Windows says "Windows protected your PC".**
The installer is unsigned. Choose "More info" then "Run anyway" only after you have compared the SHA-256 with the release page. See [Verify your download](installation.md#verify-your-download).

**Nothing happens when I hold Right Ctrl.**
Another program may own that key. Pick a different preset under *Push-to-Talk Shortcut* in Settings. Also confirm KeyReed is running: look for its icon in the system tray. A second launch does not start a second listener; it asks the running copy to show a tray notice.

**The tray says the speech model is not installed.**
Open the tray menu and choose *Download default speech model*, or use the full offline archive.

**The model will not load.**
Do not copy a half-extracted model folder into place. Use the installer's models or `keyreed.exe --download-model <id>`. A failed install is quarantined and is not loaded. Models are checked against pinned size and SHA-256 values before use. If a download was interrupted, delete that model folder and retry.

**No microphone, or the wrong one.**
Windows may report no capture device, or desktop apps may be blocked. Check Settings, Privacy, Microphone, and allow desktop apps. Then choose the device in KeyReed Settings.

**The text did not paste.**
By default KeyReed copies the finished text and you paste it yourself. Click the target application first, then press Ctrl+V. If the target ignores clipboard paste, that is a target-application issue. Auto Insert is optional and falls back to the clipboard for password fields, elevated windows, or when the target has lost focus.

**A recording was thrown away.**
A lost key release, or the 120 second cap, discards the audio on purpose. It does not transcribe and does not change the clipboard. Escape also cancels.

**Spoken symbols like "forward slash" are not applied.**
They apply only in Terminal / Code mode. A code editor is classified as Technical, not Terminal / Code, unless you choose that mode yourself.

**Some Settings labels show an underscore instead of an ampersand.**
This is a known cosmetic issue in 0.1.1. It does not affect behaviour.

**It feels slow, or memory use is high.**
The first dictation loads the speech model, which is the slow part. On a 4 GB test VM the app plus local grammar runtime used roughly 1.6 GB. Models unload after 300 seconds idle by default. A lighter speech model (`moonshine-base-en-int8` or `moonshine-tiny-en-int8`) uses less RAM, with different accuracy.

**Does it send my audio anywhere?**
Not in the default profile, Maximum Privacy (100% Offline). Only the Balanced and Maximum Accuracy profiles can call a cloud endpoint, and only if you stored an API key.

Still stuck? [Open an issue](https://github.com/Shanmukh-l33t/keyreed/issues/new?template=bug.md) with your Windows version and steps. Do not attach recordings of other people, passwords, or clipboard contents from real documents.
