# Changelog

## Unreleased

- Website rebuilt: new design, animated dictation illustration, writing-mode explorer, Wispr Flow comparison, privacy flow, and live release information.
- README rewritten. Added installation, troubleshooting, development and architecture guides, a feature request template, and CI for the website and Rust tests.
- Added brand assets, a social preview image and genuine Windows 11 screenshots under `assets/`.
- No application code changed.

## 0.1.1

- The Windows build is a GUI application. Closing a terminal no longer ends the tray process.
- The standard installer no longer embeds speech or grammar models. The tray menu downloads the default speech model and checks the compile-time pins.
- Auto Insert is available and recommended. It stays off until the user turns it on. Clipboard only remains the stored default.
- Ask before inserting copies the text and does not type into a window.
- The tray icon loads `keyreed.ico` from the install folder when that file is present.

## 0.1.0

First public Windows package.

- Installer and portable archive include the application, speech runtime, default Parakeet speech model, and default Qwen3.5 grammar model.
- Hold to talk, toggle to talk, six writing modes, clipboard output, and optional direct insertion remain in the application.
- A lost key release or the 120 second cap discards the recording and does not paste.
- A second launch notifies the running tray copy and does not start another listener.
- Model files are checked against compile-time size and SHA-256 pins before load.
- Windows binaries in this release are unsigned.
