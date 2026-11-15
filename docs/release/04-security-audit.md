# Security audit

Read the code. Ran the unit tests. Ran the Windows checks listed below. A code read is not a pass of a runtime test. Each row says which one happened.

## Results

| Control | Result | Evidence |
| --- | --- | --- |
| DPAPI on Windows | Present in the shipping exe. Roundtrip not repeated this pass. | `CryptProtectData` found in `dist/keyreed-win-x64/keyreed.exe` (file offset 3788947). `KEYREED_DEV_INSECURE_SECRETS` not in the exe (search returned -1). Same-day harness file `vm/shared/win11_e2e_report.json` records `dpapi_roundtrip: PASS`. This audit did not call CryptProtectData again. |
| Dev insecure secret store | Cannot activate in the Windows binary. The branch is `cfg(not(windows))` and the env string is absent from the exe. | `crates/vi-win32/src/security.rs` lines 112 to 125. Binary search above. |
| Non-Windows decrypt | Still accepts base64 without the env var. Linux only. Not a Windows ship bug. | `dpapi_decrypt_from_b64` non-Windows arm decodes base64 unconditionally. |
| Clipboard empty overwrite | Refused in code. | `write_clipboard_unicode` bails on empty text before `OpenClipboard`. |
| Clipboard preserve on silence | Code path sets `output_ms` 0 and does not write. Functional PASS is from the 16:03 harness, not repeated here. | `main.rs` silence return. Harness field `clipboard_preserve_on_silence: PASS`. |
| Clipboard write latency | MEASURED 0.48 ms on one run with `--copy-clipboard`. | `vm/shared/audit_clip.json`. |
| Unicode clipboard | Harness field `clipboard_unicode_multilingual: PASS` at 16:03. Not repeated. | report file |
| Microphone permission | WASAPI open errors tell the user to check Windows Microphone privacy. No runtime denial test. | `crates/vi-audio/src/lib.rs` error strings. Capture is in memory. No `File::create` in that crate. |
| Audio retention | PCM buffer zeroized after ASR in `execute_pipeline`. | `samples_16k.zeroize()` after transcribe. Cloud WAV is built in a `Cursor`, not a temp file (`cloud.rs`). |
| Global hook | `WH_KEYBOARD_LL`, removed when push-to-talk is toggled off. Synthetic keys tagged so the hook ignores them. | `hotkey.rs`, `KEYREED_SYNTHETIC_TAG`. Not timed. Not tested against an elevated window in this audit. |
| Model download pin | HTTPS plus 64-hex SHA-256 required before any socket. Unit test passed this audit. | `download_verified` and `download_verified_fails_closed_before_network_on_bad_pin`. |
| Model activation pin | FAIL. Existing folder skips the hash. | `ensure_asr_loaded`: if `direct.exists()` return that path. |
| Interrupted download | PARTIAL. Kill at 8 s left a tar.bz2 (107,600,538 bytes) and an extracted folder while the process was still alive. Files were deleted. Truncation was not proven. The activation hole means a partial folder would be loaded next time. | guest script `audit_extra.ps1` |
| Offline enforcement | Unit test `max_privacy_offline_never_uses_cloud_tier` passed. Windows runs reported `tier0_deterministic` or `tier1_local_slm`. No packet capture. | test output; audit JSON `used_tier` |
| Cloud STT gate | Only when profile is MaxAccuracy and the key is non-empty. Endpoint must be HTTPS. | `main.rs` around the cloud branch; `cloud.rs` rejects non-HTTPS. |
| Cloud rewrite gate | `allows_network()` is false for MaxPrivacyOffline. Balanced can still call cloud if no local SLM is configured and a key exists. | `vi-rewrite/src/lib.rs` lines 92 to 94. Default profile is MaxPrivacyOffline. |
| Cloud key storage | Encrypted blob in config, decrypted into a zeroizing string for the call. | `dpapi_decrypt_from_b64` then `decrypted_key` cleared by `Zeroize` import. This audit did not inspect a live config file for a key. |
| Autostart | Quoted path in code (`"path" --minimized`). HKCU value `Keyreed` was ABSENT during this audit. Restart not tested. | `set_autostart_enabled`. Guest query. |
| Single instance | Harness `single_instance_mutex: PASS` at 16:03. Not repeated. | report file |
| DLL search order | `SetDefaultDllDirectories` to System32 plus user dirs at process start. Not independently verified with a planted DLL. | `harden_and_init_process` |
| Update integrity | No updater in the crate tree. Nothing to sign, nothing to verify. | search for updater symbols returned no matches. |
| Insecure mode in production | Windows exe cannot take the dev base64 path. Confirmed by string absence, not by a negative runtime trial of the env var. | binary search |

## Confirmed gap

If `models/<folder>` exists, Keyreed loads it. A killed download can leave that folder. The archive pin never runs again. This is a local integrity hole, not a remote exploit by itself. It becomes a supply-chain hole if a stub installer downloads on first launch and the user, or a crash, interrupts extract.

Tar extract refuses symlinks, hard links, and absolute paths (`rejects_traversal_paths` passed). That does not fix the exists-short-circuit.

## Not claimed

- No finding of a working clipboard theft bug.
- No finding of an always-on microphone. Capture starts with the session object. This audit did not watch the Windows microphone indicator.
- No SmartScreen result. See the signing report.
