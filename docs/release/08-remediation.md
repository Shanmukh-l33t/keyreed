# Remediation after the production audit

Date: 2026-11-15
Verdict: Not Yet Production Ready

Unsigned binaries and an untested SmartScreen prompt still block a public download. Push-to-talk latency was not measured. A 4-core, 8 GB Windows PC was not measured.

## What changed

Model load no longer treats a directory name as trust.

- Required files for all six catalog models have compile-time size and SHA-256 pins in `crates/vi-asr/src/integrity.rs`. A sidecar manifest is not read.
- `LocalAsrEngine::load` hashes those files before ONNX parse, and opens only the pinned relative paths. A sibling file with a similar suffix is not selected.
- Install extracts into `.staging`, verifies, then renames. The active folder appears only after that check. A second hash runs on the promoted folder.
- A failed check quarantines the directory under `.rejected` and returns an error. Offline mode does not download a replacement.
- Startup sweep deletes `.staging` and `*.part` when the recorded install pid is dead. `abort_incomplete_install` signals a pid only when its command line contains `keyreed`.
- Silero VAD and `Qwen3.5-0.8B-Q4_K_M.gguf` are pinned. Any other GGUF name is refused.
- The hash buffer is heap-allocated. A 1 MB stack buffer overflowed the Windows main thread. That bug was found on the guest and fixed before the passing end-to-end run.

Autostart discrepancy, explained and fixed in the harness:

- The old harness wrote `KeyreedE2ETest`, deleted it, and hardcoded `PASS`. It never read the value back. The product value is `Keyreed`, and the default `autostart_on_login` is false, so an absent `Keyreed` Run value is the intended disabled state.
- The harness now reads the value. On this guest the stored string was `"C:\Keyreed\keyreed.exe" --minimized`. After disable, the test value was absent. `product_run_value_before_test` was null.
- The tray process now applies the saved preference to the `Keyreed` Run value at startup.

Signing:

- `scripts/sign-win.sh gate` fails the release if any non-CRT exe or DLL lacks an embedded signature. This run exited 1. No production certificate is installed. No self-signed certificate was created.

Installer:

- `scripts/keyreed.nsi` builds `KeyreedSetup.exe`. This build is 31 MB. It contains the exe, runtime, and Silero VAD. It does not embed the six speech models. Those stay in the catalog and install only through `--download-model`, which now refuses to activate an unverified tree.
- Uninstall removes the exe and `runtime`. It does not delete `models\` or an unrelated directory.

## Tests executed this pass

Linux `cargo test --workspace --offline`: 33 passed, 0 failed. That includes the new integrity tests (modified file, deleted file, truncated file, empty pin list, corrupt archive does not activate, sweep after a dead pid, quarantine of an unverified directory, all six ids have pins, extracted v3 / SenseVoice / Whisper / Moonshine tiny match the pins).

Windows guest, 3 cores, 4 GB, build 26200, new exe:

- Bad Parakeet folder: exit 1, active directory removed, one quarantine entry. Error text named the missing `decoder.int8.onnx`.
- End-to-end after the stack fix: status PASS. Autostart value quoted as above. ASR 911.8 ms, rewrite 6182.0 ms, total 11980.4 ms, tier `tier1_local_slm`. This run hashed the real model before load. It did not overflow.
- Ten pipeline runs in one process, same 6.67 s clip, clipboard on. Wall clock 24097 ms.

| Run | Cold | ASR ms | Rewrite ms | Output ms | Release to ready ms | Parent working set MB |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | yes | 1107.8 | 3163.1 | 0.39 | 7663.2 | 754.9 |
| 2 | no | 776.1 | 1077.1 | 0.46 | 1885.1 | 793.6 |
| 3 | no | 850.6 | 815.3 | 0.39 | 1696.1 | 794.2 |
| 4 | no | 824.0 | 898.2 | 0.46 | 1755.6 | 794.2 |
| 5 | no | 862.4 | 880.8 | 0.31 | 1772.3 | 794.2 |
| 6 | no | 783.5 | 851.6 | 0.29 | 1663.9 | 794.6 |
| 7 | no | 752.6 | 908.9 | 0.36 | 1694.9 | 794.6 |
| 8 | no | 878.3 | 847.9 | 0.40 | 1756.1 | 794.6 |
| 9 | no | 845.7 | 921.8 | 0.30 | 1795.6 | 794.6 |
| 10 | no | 668.7 | 880.8 | 0.47 | 1580.7 | 794.6 |

Warm rewrite (runs 2 to 10): median 880.8 ms, p95 1077.1 ms. Cold rewrite is one sample, 3163.1 ms. These are not 8 GB numbers. Parent working set excludes `llama-server.exe`. Output ms is the clipboard write.

Installer on the same guest: silent install exit 0 to `C:\KeyreedInstTest`. Exe, uninstaller, runtime DLL, and Start Menu shortcut present. `--version` printed `Keyreed 0.1.0`. Silent uninstall exit 0. Exe and runtime gone. An unrelated file `C:\KeyreedKeep\user-note.txt` still read `do-not-delete`.

Bundle checksums after repackaging the new exe: 47 OK, 0 failed. Exe size 4215808 bytes. `KEYREED_DEV_INSECURE_SECRETS` is absent from that exe.

## Not measured, not passed

- Push-to-talk shortcut latency, microphone-open latency, and Ctrl+V in other applications. SSH has no interactive desktop.
- 4-core, 8 GB Windows. Integrated graphics. Dedicated GPU.
- SmartScreen dialog.
- Embedded signature on the shipping binaries. Gate failed.
- Windows login after restart.
- Upgrade of an existing install.
- A live interrupted download on Windows after the staging change. The unit test covers a dead pid and a corrupt archive. It is not a guest kill during HTTPS transfer.
- Hindi word error rate.
- A new 244-clip inference pass. Stored-hypothesis rescore from the earlier audit still stands. It was not repeated as a model reload.

## Verdict

Not Yet Production Ready.

The integrity hole from the audit is closed in code and checked on the guest for a bad directory and a good load. The installer and uninstaller ran. Warm rewrite on this 4 GB guest is under about 1.1 s at p95, after a cold start of 3.2 s on the one cold sample. None of that signs the binaries or proves the global hotkey. Do not publish the setup exe until `scripts/sign-win.sh gate` passes and a clean profile records the SmartScreen result.
