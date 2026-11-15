# Code signing status

Unsigned. Not production-ready for public distribution.

## Measured

Linux `osslsigncode verify` on `dist/keyreed-win-x64` (this audit, `scripts/sign-win.sh status`):

- 39 exe/dll files reported UNSIGNED.
- 0 reported VALID.
- Includes `keyreed.exe`, `llama-server.exe`, `llama.dll`, `onnxruntime.dll`, `sherpa-onnx-c-api.dll`.
- `keyreed.exe` PE checksum field is `00000000`. Calculated checksum `00401C33`. osslsigncode warned `invalid PE checksum`.

Windows guest `Get-AuthenticodeSignature` on `C:\Keyreed` (this audit):

- `keyreed.exe`: NotSigned.
- sherpa and llama binaries: NotSigned.
- `msvcp140.dll`, `msvcp140_1.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`: Status Valid.

Those four CRT files in the dist tree are byte-identical to `/root/.wine/drive_c/windows/system32/msvcp140.dll` (sha256 `0a60aa77f04c6c0c88073e4ab0aa0092e794a074887b0c32aa7d391fe4da4129` for `msvcp140.dll`). osslsigncode finds no embedded signature in those bytes either. Windows reporting Valid is consistent with catalog trust of a known Microsoft binary, not with a signature this project embedded. Do not describe the CRT DLLs as "we signed them."

`scripts/sign-win.sh sign` with no `KEYREED_SIGN_PFX`: exit 1. Message: refusing to sign. No self-signed certificate was created. No file in the bundle changed.

## SmartScreen

Not observed. This audit did not download the exe through a browser, so there was no Mark of the Web and no SmartScreen dialog.

What is known from the signature check, not from a SmartScreen screenshot: the main executable is NotSigned. An unsigned executable published on the internet is not a finished trust story. This document does not claim a specific SmartScreen string because that UI was not shown.

## What a production signature requires

Not implemented, because no certificate is on this machine.

1. A code-signing certificate from a public CA. An EV certificate is what Microsoft treats as having immediate SmartScreen reputation. A standard OV certificate can sign, but reputation builds after real downloads. This audit did not buy or install either.
2. Sign, with an RFC3161 timestamp, in this order: `keyreed.exe`, every non-Microsoft DLL under `runtime/`, the installer once one exists, the updater once one exists.
3. Do not strip a real Microsoft signature off CRT binaries. Prefer `vc_redist.x64.exe` from Microsoft over app-local copies taken from a Wine system32.
4. Re-run `scripts/sign-win.sh status` and `Get-AuthenticodeSignature`. Both must report a valid embedded signature on `keyreed.exe` before any public ship claim.
5. Then, and only then, download the signed installer in a clean Windows profile via a browser and record the SmartScreen result. Until that screenshot exists, SmartScreen is untested.

`scripts/sign-win.sh` is the signer. It will not invent a certificate. Timestamp URL default is `http://timestamp.digicert.com`. Override with `KEYREED_TIMESTAMP_URL`.

## Installer and updater

No installer binary exists. No updater binary exists. Signing requirements for those files are documented so they are not forgotten. They cannot be signed until they exist.
