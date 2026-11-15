# Security

KeyReed is a Windows desktop application. It listens for a global shortcut and opens the microphone only while a recording is active.

## Reporting

Report a vulnerability privately. Do not open a public issue for an unfixed security bug.

Use GitHub private vulnerability reporting: https://github.com/Shanmukh-l33t/keyreed/security/advisories/new. If that is unavailable to you, contact the repository owner through the profile at https://github.com/Shanmukh-l33t. No security email address is published.

Include the version, the file you ran, and what you observed. Do not include microphone recordings of other people, passwords, or clipboard contents from a real document.

## What this project already enforces

- Default processing does not call a cloud endpoint.
- Model directories are hashed before load. A partial extract is not treated as installed.
- A cancelled or timed-out recording is not transcribed and does not write the clipboard.
- Development-only insecure secret storage is not part of the Windows release path.

## What this release does not claim

The Windows executable, installer, and bundled runtimes are unsigned. An unsigned file is not a statement that the bytes were tampered with, and a signature would not by itself prove SmartScreen reputation.
