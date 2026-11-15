# Keyreed production readiness

Date: 2026-11-15
Verdict: Not Yet Production Ready

Not ready for private beta. The acceptance kit is `dist/KeyreedAcceptance.zip`. Physical dictation was not run. See `docs/release/12-real-windows-validation.md`.

## What was actually run in this audit

- `cargo test --workspace --offline`: 23 tests passed (vi-asr 3, vi-audio 2, vi-core 5, vi-rewrite 8, vi-win32 5). Zero failures.
- Windows 11 Pro build 26200 guest, 3 cores, 4 GB RAM, VirtIO GPU. No dedicated GPU. Not an integrated-graphics laptop.
- `keyreed.exe --version` five times: 36, 28, 23, 24, 22 ms.
- Three pipeline runs of the same 6.67 s clip `ind_000.wav` on that guest.
- Authenticode status via `Get-AuthenticodeSignature` on the guest and `osslsigncode verify` on the bundle.
- `scripts/sign-win.sh sign` with no certificate: exit 1, no signature written.
- Bundle checksum manifest: 47 of 47 OK.
- ASR hypotheses on disk re-scored with `bench/score.py` logic. Published English WER matched.

## What was not run

- 4 CPU cores and 8 GB RAM. Guest is fixed at 3 cores and 4 GB (`RAM_SIZE=4G`, `CPU_CORES=3`). Host had 8 GB available. Giving the guest 8 GB would leave the host with no headroom and would contaminate timings with swap. Not resized. No 8 GB number exists.
- Integrated graphics. Guest GPU is `Red Hat VirtIO GPU DOD controller`.
- Push-to-talk key latency. Microphone start latency. SSH has no interactive desktop. An earlier screenshot step failed with `CopyFromScreen` / invalid handle.
- SmartScreen prompt. No browser download, no Mark of the Web.
- Installer, upgrade, uninstall, Windows restart. No installer exists. HKCU Run value `Keyreed` was ABSENT at audit time.
- Blocked-network download. Packet capture of an offline run.
- A fresh 244-clip ASR inference pass. This audit re-scored stored hypotheses. It did not reload the models.
- Hindi word error rate. Stored `hi_wer` is null. Re-score was English `ref` only.

## Classification

Not Yet Production Ready.

Reasons that block a public ship, each measured or read in source:

1. `keyreed.exe`, `onnxruntime.dll`, `sherpa-onnx-c-api.dll`, `llama-server.exe`, and `llama.dll` have no embedded Authenticode signature.
2. There is no installer, no uninstaller, and no updater. Update integrity cannot be tested because there is no update channel.
3. A folder that already exists is loaded with no hash re-check (`ensure_asr_loaded` in `crates/vi-app/src/main.rs`). Download pins are real. Activation of a pre-existing folder is not pinned.
4. The only Windows box measured is a 3-core, 4 GB VM. Cold rewrite on that box was 3.5 s, 5.3 s, and 7.8 s across three runs. That is not a 4-core, 8 GB result, and it is not a steady-state claim.

## Deliverables

- Hardware: `docs/release/01-hardware-benchmark.md`
- Accuracy: `docs/release/02-asr-accuracy.md`
- Package size: `docs/release/03-distribution-size.md`
- Security: `docs/release/04-security-audit.md`
- Signing: `docs/release/05-code-signing.md`
- Limits and blockers: `docs/release/06-known-limitations-and-blockers.md`
- Install and use: `docs/release/07-install-and-user.md`
- Bundle: `dist/keyreed-win-x64/` built by `scripts/package-win.sh`
- Signer (fails closed): `scripts/sign-win.sh`
