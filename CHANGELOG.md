# Changelog

## 0.1.0

First public Windows package.

- Installer and portable archive include the application, speech runtime, default Parakeet speech model, and default Qwen3.5 grammar model.
- Hold to talk, toggle to talk, six writing modes, clipboard output, and optional direct insertion remain in the application.
- A lost key release or the 120 second cap discards the recording and does not paste.
- A second launch notifies the running tray copy and does not start another listener.
- Model files are checked against compile-time size and SHA-256 pins before load.
- Windows binaries in this release are unsigned.
