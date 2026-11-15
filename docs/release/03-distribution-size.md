# Distribution size

Measured on `dist/keyreed-win-x64` after `scripts/package-win.sh`, checksum manifest 47 of 47 OK.

Full tree: 1217.1 MB, 48 files.

| Piece | Size | Required for first launch with defaults |
| --- | --- | --- |
| keyreed.exe | 4.0 MB | yes |
| runtime/sherpa-onnx | 20 MB | yes for local ASR |
| runtime/llama | 55 MB (du) | yes for Tier 1 rewrite |
| silero_vad.onnx | 0.6 MB | yes if VAD is used |
| Parakeet v2 folder | 631 MB | yes for the default ASR model |
| encoder.int8.onnx inside that folder | 622 MB | yes, it is the model |
| Qwen3.5-0.8B-Q4_K_M.gguf | 508 MB | yes for default smart rewrite |
| README plus checksums | under 6 KB | yes for the bundle contract |

Byte sum of exe + runtime + VAD + README + checksums, excluding ASR and GGUF: 78.6 MB.

The 1.2 GB package is two models. Runtime and the executable are not the problem. Sharing the MSVC CRT with a system `vc_redist` install would remove four DLLs (about 6 MB). That does not change the download the user notices.

## On-demand models

All six ASR ids stay in `ASR_CATALOG`. The default bundle ships one of them (Parakeet v2) plus the default GGUF. The other five are not in the 1.2 GB tree:

- parakeet-tdt-0.6b-v3-int8
- moonshine-base-en-int8
- moonshine-tiny-en-int8
- sense-voice-small-int8
- whisper-small.en-int8

`keyreed --download-model <id>` calls `ensure_model_present`, which downloads over HTTPS and checks the SHA-256 pin before rename. Pins are 64 hex for every catalog row (unit test `every_catalog_entry_is_https_and_sha256_pinned`).

Settings UI was not shown to start a download in this audit. The CLI path exists. A settings button, if present, was not exercised.

A first-download probe on the guest: `--download-model moonshine-tiny-en-int8` into an empty directory. At 8 seconds the process was still running. On disk: `sherpa-onnx-moonshine-tiny-en-int8.tar.bz2` at 107,600,538 bytes (catalog approx 103 MB) and an extracted folder with the expected Moonshine filenames. The process had not exited, so archive cleanup had not finished. The directory was deleted. This audit did not see the "Model ready" line and did not transcribe with that model. First download is partial, not a pass.

## Smaller initial installer

Possible without removing a capability:

- Ship the 78.6 MB runtime stub.
- Download Parakeet v2 and the GGUF on first launch through the existing pinned downloader.
- Keep the other four ASR models as explicit `--download-model` choices.

That preserves the six catalog options. It does not exist as an installer today. `scripts/package-win.sh` builds the full default tree. There is no MSI, no WiX, no Inno script in the repo.

Do not ship the stub until interrupted-download behavior is fixed. See security report. A folder that exists is treated as ready even if extract was killed mid-way. A stub installer that downloads on first launch inherits that hole.

## Hash before activation

Download: verified, fail closed, unit tested.
Activation of an existing folder: not verified. `ensure_asr_loaded` returns the directory if it exists and does not re-hash the ONNX files.

## Reproducible package

```
scripts/package-win.sh default
```

Output: `dist/keyreed-win-x64/`. `CHECKSUMS.sha256` checked 47 of 47 OK in this audit. No `LICENSE` file is in the repo, so the bundle has no license file even though the crate license is `MIT OR Apache-2.0`.
