# Security

KeyReed is a Windows desktop application. It listens for a global shortcut and opens the microphone only while a recording is active.

## Reporting

Report a vulnerability privately. Do not open a public issue for an unfixed security bug.

Email is not published in this repository yet. Until a security contact is listed, open a private GitHub security advisory on this repository if your account can, or contact the repository owner directly.

Include the version, the file you ran, and what you observed. Do not include microphone recordings of other people, passwords, or clipboard contents from a real document.

## What this project already enforces

- Default processing does not call a cloud endpoint.
- Model directories are hashed before load. A partial extract is not treated as installed.
- A cancelled or timed-out recording is not transcribed and does not write the clipboard.
- Development-only insecure secret storage is not part of the Windows release path.

## What this release does not claim

The Windows executable, installer, and bundled runtimes are unsigned. An unsigned file is not a statement that the bytes were tampered with, and a signature would not by itself prove SmartScreen reputation.
