# 06: Security & Privacy Threat Model and Requirements

Status: threat model + implementable mitigations + testable checklist. Written 2026-11-15.
Each mitigation below maps to code that exists in `crates/` (file cited) or is an explicit, named gap. MITRE ATT&CK technique IDs are used to frame each threat the way a defender or auditor would. CVE references are to the real vulnerability families our input-parsing surface must resist.

## System summary (what an attacker can reach)

Keyreed is a per-user, non-elevated Windows background process that:
- holds a system-wide low-level keyboard hook (`WH_KEYBOARD_LL`);
- captures the microphone on demand (PTT);
- reads the foreground window's process name, class, and focused-control style;
- writes text into arbitrary applications via clipboard and `SendInput`;
- loads large model binaries (ONNX, GGUF) from disk and, on first run, from the network;
- optionally talks to a user-configured cloud endpoint with a user-supplied key;
- runs a loopback HTTP child (`llama-server` on `127.0.0.1:<ephemeral>`).

That combination, global input hook + mic + write-into-any-window + network model download, is exactly the profile a security reviewer should treat as high-risk. The design goal is: **every capability is justified, bounded, and observable, and no path lets dictated content or audio leave the machine without an explicit user opt-in.**

## Threat model

### T1: The app behaves like a keylogger (trust + legal risk)
Technique analogue: MITRE T1056.001 (Input Capture: Keylogging). A global keyboard hook is the same primitive malware uses.
Mitigation (implemented, `hotkey.rs`): the hook inspects ONLY modifier VKs, `VK_ESCAPE`, and the configured shortcut VK. There is no keystroke buffer, no logging sink, no persistence of key events. The callback pushes a coarse `HotkeyEvent` (PttDown/PttUp/Cancel/CycleMode) onto a channel and nothing else. This is both a privacy guarantee and an auditable one: a reviewer can read `hotkey.rs` and confirm no character data is retained.
Requirement: this must never regress. Any future feature that wants keystroke content (e.g. "type a trigger word") must be rejected or implemented via the ASR path, not the hook.

### T2: Audio or dictated text leaks to the network
Technique analogue: T1123 (Audio Capture), T1115 (Clipboard Data), exfiltration.
Mitigation (implemented): default `ProcessingProfile::MaxPrivacyOffline` (`config.rs`). In this profile the cloud STT (`cloud.rs`) and cloud LLM (`cloud_llm.rs`) paths are never reached, the engine only attempts network when `profile.allows_network()`. Audio is kept RAM-only and zeroized (`privacy.ram_only_audio = true` default; `cloud.rs::encode_wav_in_memory` builds the WAV in a `Cursor`, and both the WAV bytes and the request body are `.zeroize()`d after send). No temp WAV ever hits disk.
Requirement: HTTPS-only for any cloud endpoint (`cloud.rs` rejects non-`https://`). BYOK only, the app ships with no vendor key, so there is no server-side account that could be subpoenaed or breached to expose user audio.
Gap / requirement: add a build-time or runtime assertion that the offline profile cannot reach a socket, and a unit test that the cloud client is inert under MaxPrivacyOffline. Today it is gated by control flow, not by an enforced invariant.

### T3: Prompt injection via dictated text (the LLM obeys or answers the user's words)
Technique analogue: OWASP LLM01; also a correctness risk.
Mitigation (implemented, `guards.rs`, `slm.rs::build_mode_prefix`): dictated text is wrapped in `<transcript>` and the system prompt forbids treating it as instructions; three few-shot turns demonstrate refusing an injection, preserving a dictated question, and not answering. Crucially the guard layer is the real defence: length-ratio, numeric/identifier survival, question-preservation, chatter-prefix, and lexical-drift checks reject any candidate that looks like the model obeyed, answered, or hallucinated, and we fall back to deterministic Tier 0. Unit-tested (`guards.rs::tests::blocks_prompt_injection_collapse`).
Requirement: the guard floor must remain ≥ Tier 0 on the invariant eval (currently 100 % vs 100 %). A regression here is a security bug, not just a quality bug.

### T4: Trojan Source / bidi-override and control-character injection
Technique analogue: CVE-2021-42574 (Unicode bidi overrides). Dictated or model-produced text containing RLO/LRO/FSI characters can visually reorder code or commands in an editor, hiding malicious intent.
Mitigation (implemented, `typing.rs::sanitize_for_injection`): strips `\u202A-\u202E`, `\u2066-\u2069`, `\u200E`, `\u200F` and non-printable control chars before any `SendInput`. In Terminal contexts it also strips `\r`/`\n` so dictated text can never auto-submit a shell command.
Requirement: this sanitizer must run on the direct-typing path AND be applied to clipboard writes destined for terminals. Verify it is not bypassed by the clipboard-only fallback.

### T5: Destructive command auto-execution
Scenario: user dictates "run rm -rf slash var slash tmp" into a terminal; the app types it and it executes.
Mitigation (implemented, `tier0.rs::RE_DANGEROUS_CMD` + `typing.rs`): a catalog of destructive commands (`rm -rf`, `sudo rm`, `mkfs`, `dd if=`, `format X:`, `git push -force`, `git reset -hard`, `terraform destroy`, `kubectl delete`, `drop table`, `chmod -R 777`, `shutdown`, PowerShell `Remove-Item -Recurse`, …) sets `requires_confirmation=true`, which forces clipboard-only delivery with a manual-paste gate, the app never auto-types a destructive command. Terminal newline stripping (T4) prevents the Enter that would run it.
Requirement: keep the dangerous-command list curated and tested; a miss here is data loss.

### T6: Typing into the wrong window / TOCTOU
Scenario: user starts dictation in email, Alt-Tabs to a password field or a different app during inference; text lands in the wrong place.
Mitigation (implemented, `typing.rs::deliver_direct_or_clipboard_fallback`): the target `HWND` is captured at PTT-down; before direct typing we re-check `GetForegroundWindow() == target_ctx.hwnd` and, on mismatch, fall back to clipboard-only (`foreground_window_changed_during_processing`). Password fields and password managers are blocked outright (`block_password_fields=true` default; `context.rs` deny-list + `ES_PASSWORD`/UIA `CurrentIsPassword` check), text goes to clipboard with history-exclusion, never typed.
Requirement: the elevation check (T7) and the focus check must both pass before any `SendInput`; order matters and is currently correct.

### T7: UIPI / integrity-boundary violation
Scenario: a non-elevated app trying to send input to an elevated window (Task Manager, admin terminal), either fails silently or, if we ran elevated, would broaden our own attack surface.
Mitigation (implemented, `typing.rs`): we detect target elevation vs our own (`TokenElevation`) and, when the target is elevated and we are not, fall back to clipboard (`target_window_elevated_uipi`) with a clear message. We deliberately do NOT run elevated by default.
Requirement: never ship elevated-by-default. The correct UX is "can't type into an admin window, click to relaunch elevated" as an explicit, logged user choice.

### T8: DLL planting / sideloading
Technique analogue: MITRE T1574.001 (DLL Search Order Hijacking). We ship `onnxruntime.dll`, `sherpa-onnx-c-api.dll`, `llama-*.dll` next to the exe; a classic attack is dropping a malicious DLL with one of those names in the app folder or CWD.
Mitigation (implemented, `security.rs::harden_and_init_process`): at process entry we call `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS)` so implicit OS DLL loads only come from System32 or directories we explicitly registered; the app's own DLLs are loaded by absolute path via `AddDllDirectory` (`ffi.rs`). PATH and CWD are removed from the search.
Requirement: this call must happen before any DLL load (it does, first thing in `main`). Code-sign every shipped DLL (see T12) so a swapped DLL fails signature/pinning.

### T9: Malicious model archive extraction (path traversal / symlink / zip-slip)
Technique analogue: CVE-2022-25882, CVE-2024-27318, CVE-2024-7776 (ONNX external_data / tar traversal), CVE-2026-27489 (ONNX symlink traversal), CVE-2024-27319 / CVE-2026-14647 (ONNX/ORT out-of-bounds parse), CVE-2024-34359 / CVE-2024-42479 / CVE-2025-53630 / CVE-2026-27940 (llama.cpp GGUF / RPC integer-overflow and heap OOB). Our app parses ONNX and GGUF files; these CVEs are the concrete reason parsing untrusted model files is dangerous.
Mitigation (implemented, `catalog.rs`): `validate_safe_relative_path` rejects absolute paths, `..`, root, and prefix components; extraction rejects symlinks, hardlinks, and device nodes; we download only from the pinned k2-fsa GitHub release origin over HTTPS.
GAP (important, be honest): every catalog entry currently has `expected_sha256: ""`. We verify HTTPS origin and safe extraction, but we do NOT pin the archive hash. A compromise of the release CDN, or a MITM on a misconfigured TLS path, could substitute a malicious model that then tries to exploit an ONNX/GGUF parser CVE. Requirement: populate `expected_sha256` for all six models and enforce it before extraction. This is the single highest-value security fix outstanding. Additionally, keep sherpa-onnx / onnxruntime / llama.cpp updated to versions that contain the fixes for the CVEs above, and track them.

### T10: Secrets at rest (BYOK cloud key)
Mitigation (implemented, `security.rs`): the cloud API key is stored DPAPI-encrypted (`CryptProtectData`, `CRYPTPROTECT_UI_FORBIDDEN`) bound to the user logon; plaintext is zeroized immediately after encryption; config stores only `encrypted_api_key_b64`.
GAP: the non-Windows `dpapi_*` path returns plain Base64 (dev-only convenience). This must never be reachable in a shipped Windows build, and ideally the dev path should refuse rather than silently store plaintext. Requirement: gate the base64 fallback behind an explicit debug flag and make release builds fail if DPAPI is unavailable.

### T11: Loopback inference server exposure
Scenario: `llama-server` listens on `127.0.0.1:<ephemeral>`; another local process (or a malicious browser page via DNS-rebinding/CSRF to localhost) could hit it.
Mitigation (implemented, `slm.rs`): binds to `127.0.0.1` only (not `0.0.0.0`), uses an ephemeral port chosen by binding first, and the child is killed on idle-unload. The window of exposure is the active-dictation session.
Requirement: consider a shared secret / token on the loopback request to defeat same-machine opportunistic access; low priority on a single-user desktop but worth noting for multi-user/RDP hosts.

### T12: Unsigned binary / SmartScreen / AV false positives
Scenario: first launch on a clean machine is blocked by SmartScreen, or AV flags the keyboard-hook + mic + SendInput behaviour. This is simultaneously a security signal and the #1 adoption killer (competitor complaint #6 in `05`).
Status: GAP. We currently ship unsigned. Requirement: Authenticode code-signing (EV cert preferred to avoid SmartScreen reputation delay) on the exe, installer, and every shipped DLL; publish a checksum manifest; pre-register with AV vendors. Until signed, document the SmartScreen "More info → Run anyway" path for testers.

### T13: Telemetry / silent data collection
Status: NONE by design. There is no analytics, no crash reporter, no phone-home outside the explicit cloud BYOK path. Requirement: keep it that way; if telemetry is ever added it must be opt-in, disclosed, and off by default, and it must never carry audio or dictated text.

### T14: Config/history integrity
Mitigation (implemented, `config.rs`, `history.rs`): atomic write (temp + rename) so a crash cannot corrupt config/history; history is OFF by default, stores no audio, is capped (100 entries / 7-day retention) and pruned on load; `redact_sensitive_window_titles` skips titles in private browsers/password managers.
Requirement: set restrictive DACL on the data dir (current user only) so other local users can't read config/history; verify on multi-user machines.

## Privacy-by-design summary (the story we tell users)

- Offline by default; audio never leaves the machine unless you add your own key and switch profile.
- We do not log keystrokes, the hook only watches your PTT chord.
- No account, no telemetry, no server that holds your voice.
- History is off; if you turn it on it stays local, capped, and never stores audio.
- The app refuses to type into password fields and never auto-runs a destructive command.

## Testable checklist (map each to an automated or manual test)

1. [DONE, test it] Hook retains no character data, static review of `hotkey.rs` + a test asserting only `HotkeyEvent` crosses the channel. (T1)
2. [GAP→add] Under MaxPrivacyOffline, no socket is opened, integration test with a fake network that throws if dialed. (T2)
3. [DONE, tested] Injection/answer/hallucination rejected by guards, `guards.rs` unit tests + invariant eval ≥ Tier 0. (T3)
4. [DONE, test it] Bidi/control chars stripped; terminal `\r\n` stripped, unit test on `sanitize_for_injection`. (T4)
5. [DONE, test it] Destructive command → clipboard-only + confirmation flag, unit test over the `RE_DANGEROUS_CMD` catalog. (T5)
6. [DONE, test it] Focus change mid-inference → no direct type, VM E2E with an Alt-Tab injected. (T6)
7. [DONE] Password field / manager blocked, `context.rs` deny-list + UIA check; E2E assertion. (T6)
8. [DONE] Elevated target → clipboard fallback + message, app not elevated by default. (T7)
9. [DONE] `SetDefaultDllDirectories` before any load; DLLs via `AddDllDirectory` absolute. (T8)
10. [GAP→HIGH] Populate `expected_sha256` for all 6 models; enforce before extract; reject on mismatch. (T9)
11. [GAP] Keep sherpa-onnx/ORT/llama.cpp on patched versions for the ONNX/GGUF CVE list; add a dependency-CVE watch. (T9)
12. [GAP→MED] Gate DPAPI base64 fallback behind debug flag; release fails if DPAPI unavailable. (T10)
13. [DONE] Loopback server binds 127.0.0.1, ephemeral port, killed on idle. (T11)
14. [GAP→HIGH] Authenticode-sign exe + DLLs + installer; publish checksum manifest; AV pre-registration. (T12)
15. [DONE by design] No telemetry; assert no outbound except cloud BYOK. (T13)
16. [DONE, test it] Atomic config/history writes; history off/capped/no-audio; restrictive DACL on data dir. (T14)

Priority order for the outstanding gaps: **T9 hash-pinning (10)** and **T12 code-signing (14)** first, they are the two that a security-minded buyer or an enterprise procurement review will block on, and T9 is the one with a real exploit path (malicious model → parser CVE). Then T10 (12) and the T2 enforced-invariant test (2).

## Citations (verified reachable 2026-11-15)

- MITRE ATT&CK: T1056.001 https://attack.mitre.org/techniques/T1056/001 ; T1123 https://attack.mitre.org/techniques/T1123 ; T1115 https://attack.mitre.org/techniques/T1115 ; T1574.001/T1574.009 https://attack.mitre.org/techniques/T1574
- Trojan Source: https://nvd.nist.gov/vuln/detail/cve-2021-42574
- ONNX/ORT parse & extraction CVEs: https://access.redhat.com/security/cve/CVE-2024-27318 ; https://nvd.nist.gov/vuln/detail/cve-2024-7776 ; https://www.cve.org/CVERecord?id=CVE-2026-27489 ; https://security.snyk.io/vuln/SNYK-PYTHON-ONNXRUNTIME-15869956
- llama.cpp GGUF parser CVEs: CVE-2025-53630, CVE-2026-27940 (https://github.com/ggml-org/llama.cpp/security/advisories)
- Windows APIs used by the mitigations: DPAPI `CryptProtectData` https://learn.microsoft.com/en-us/windows/win32/api/crypt32/nf-crypt32-cryptprotectdata ; `SetDefaultDllDirectories` https://learn.microsoft.com/en-us/windows/win32/api/libraryloader/nf-libraryloader-setdefaultdlldirectories ; `AddDllDirectory` https://learn.microsoft.com/en-us/windows/win32/api/libraryloader/nf-libraryloader-adddlldirectory ; UIPI/integrity https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview
- OWASP LLM Top 10 (LLM01 injection): https://owasp.org/www-project-large-language-model-ai-top-10/
