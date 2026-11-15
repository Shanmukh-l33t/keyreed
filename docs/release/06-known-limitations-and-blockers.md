# Known limitations and production blockers

## Blockers (public ship stops here)

1. No Authenticode signature on `keyreed.exe` or the inference DLLs. Measured. `scripts/sign-win.sh` is ready and refuses to fake a certificate. No PFX is installed.
2. SmartScreen behavior was not observed.
3. No installer, no uninstaller, no updater. Fresh install, upgrade, uninstall, and Windows restart were not tested as product flows.
4. Existing model folders are loaded without a hash re-check. Interrupted extract can leave a folder that the next launch will trust.
5. Windows latency is only measured on a 3-core, 4 GB VM. Rewrite cold start on that VM was 3483 ms, 5316 ms, and 7766 ms. A 4-core, 8 GB machine was not measured. No latency claim for that class is allowed.
6. PE checksum of `keyreed.exe` is zero. Signing should repair it. Until then, treat it as another unsigned-binary smell.
7. No `LICENSE` file in the repo or the bundle. Crate metadata says `MIT OR Apache-2.0`. The file the bundle expects to copy is missing.

## Limitations (true, not blockers by themselves)

- Hindi ASR accuracy is unmeasured (`hi_wer` null, 18 clips unscored).
- Windows accuracy evidence is one 6.67 s clip, three passes. The 208-clip English WER is a Linux bench rescore of stored hypotheses, not the Windows exe.
- Push-to-talk latency and microphone open latency were not measured. Tray process was alive at 2 s with a 9 MB working set, then killed.
- Physical microphones were not compared. The corpus uses public speech sets, synthetic voices, and mixed noise.
- URL, IP, and email accuracy is a correction-set category, not an ASR WER slice.
- Correction "meaning preserved" is an 88-case invariant score (must-keep, must-not-contain), not a human listen-back of every rewrite. Qwen3.5 and Tier 0 scored 100% on that file. Qwen3-0.6B scored 93.2% and is not the default.
- Guest GPU is virtual. Integrated graphics and dedicated GPUs were not measured. Inference is CPU-only.
- App-local MSVC CRT DLLs come from a Wine system32 image. Windows called them Valid. osslsigncode found no embedded signature. Ship the Microsoft redistributable instead.
- Balanced profile can call a cloud LLM if no local SLM is configured and a key is stored. Default profile is MaxPrivacyOffline, which the unit test holds off the network. No packet capture was taken.
- `bench/score.py` now crashes if `bench/results/` contains `win11_live_e2e_report.json`, because that file has no `records` key. The published summary was re-checked by a filtered rescore. The script itself is not a clean one-command repro until that glob is fixed. This audit did not change the script.

## Release matrix (this audit)

| Test | Result |
| --- | --- |
| CLI version launch | PASS, 22 to 36 ms, five runs |
| Tray process appears | PASS at 2 s, 9 MB, then killed. Not a full session. |
| Tier 0 pipeline | PASS, one clip |
| Tier 1 local pipeline | PASS, one clip, three timings |
| Clipboard write | PASS, 0.48 ms, one run |
| 4 core / 8 GB | NOT RUN |
| iGPU / dGPU | NOT RUN |
| Push-to-talk latency | NOT RUN |
| Microphone privacy deny | NOT RUN |
| 50-clip Windows WER | NOT RUN. Linux stored-hyp rescore PASS against the summary file. |
| Fresh installer | NOT RUN. No installer. |
| First model download to "ready" | PARTIAL. Archive and folder appeared. Process killed. Result deleted. |
| Upgrade | NOT RUN |
| Uninstall | NOT RUN |
| Interrupted download corruption | NOT PROVEN. Activation short-circuit CONFIRMED in source. |
| Network failure | NOT RUN |
| Offline packet capture | NOT RUN. Local tier observed. |
| Crash recovery | NOT RUN |
| Windows restart / autostart survival | NOT RUN. Run key absent. |
| Code signature | FAIL. Unsigned. |
| Unit tests | PASS, 23. |
