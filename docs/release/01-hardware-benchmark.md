# Hardware benchmark

Measured 2026-11-15. Estimates are labeled. Unmeasured cells stay unmeasured.

## Machines

| Machine | Status | CPU | RAM | GPU |
| --- | --- | --- | --- | --- |
| Windows guest `vi-win` | MEASURED | AMD EPYC 9655, 3 cores exposed | 4.00 GB | Red Hat VirtIO GPU DOD controller |
| Linux bench host | MEASURED for the 2026-11-15 ASR bench only | AMD EPYC 9655, 6 cores visible (`nproc`) | 15 GB host, 8 GB available at audit time | not used for ASR (CPU) |
| 4 cores, 8 GB Windows | NOT MEASURED |  |  |  |
| Integrated graphics laptop | NOT MEASURED |  |  |  |
| Dedicated GPU | NOT PRESENT in the guest. No passthrough. llama.cpp is started with GPU layers off. |  |  |  |

The 4 GB guest is the low-memory case. It is not a stand-in for an 8 GB laptop.

## Windows guest, this audit

Clip: `C:\Keyreed\ind_000.wav`, 6670 ms audio. Same file every run. One recording, three pipeline passes. Not a latency distribution.

CLI `--version` (process start and print, not tray ready):

| Run | ms |
| --- | --- |
| 1 | 36 |
| 2 | 28 |
| 3 | 23 |
| 4 | 24 |
| 5 | 22 |

Tray process with no arguments, sampled at 2 seconds, then killed: working set 9.0 MB. That is pre-model idle. It is not "ready to dictate."

Pipeline. `output_ms` is the clipboard write. It was measured only on the third run, which passed `--copy-clipboard`.

| Run | Mode | Tier | ASR ms | Rewrite ms | Output ms | Release to ready ms | keyreed working set MB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Tier 0, exact | exact | tier0_deterministic | 997.1 | 1.8 | not requested | 3660.4 | 753.7 |
| Tier 1 cold | smart | tier1_local_slm | 1029.3 | 5315.6 | not requested | 8555.9 | 754.3 |
| Tier 1 plus clipboard | smart | tier1_local_slm | 869.0 | 3483.2 | 0.48 | 6419.2 | 754.6 |

Earlier same-day end-to-end on this guest, after the single-slot llama.cpp flags: rewrite 7766 ms, release to ready 12023.5 ms, active working set 1092.4 MB, after unload 378.3 MB. That figure is from `vm/shared/win11_e2e_report.json` (16:03 UTC). This audit did not repeat that harness. The three rows above were run in this audit.

Clipboard write itself is sub-millisecond when it runs (0.48 ms). The wait the user feels is model load plus rewrite, not the clipboard API.

Text from the smart runs: "I know that my parents bought me a lot of books, so I used to always read a lot."
Exact plus Tier 0 kept fillers: "I know like Um like my parents bought me a lot of books so I used to always read a lot."

## Memory and CPU during the clipboard run

Samples every 700 ms while the process lived. CPU column is process CPU time in seconds, not a utilization percentage. Do not convert it to a percent. This audit did not record a performance-counter percentage.

| Sample | keyreed WS MB | keyreed CPU s | llama-server WS MB | llama-server CPU s |
| --- | --- | --- | --- | --- |
| 0 | 681.1 | 0.91 | absent |  |
| 1 | 681.0 | 1.70 | absent |  |
| 2 | 721.5 | 3.14 | absent |  |
| 3 | 754.7 | 4.86 | 62.3 | 0.66 |
| 4 | 754.7 | 4.88 | 802.6 | 1.67 |
| 5 | 754.5 | 4.88 | 840.9 | 3.06 |
| 6 | 754.5 | 4.88 | 882.4 | 4.66 |

At sample 6 both processes were alive. Sum of those working sets: 1636.9 MB. That is one sample, not a traced peak. On a 4 GB guest this completed. It does not prove an 8 GB machine, and it does not prove a 4 GB machine stays comfortable once Windows services are busy.

keyreed CPU time stopped rising once llama-server started. Rewrite CPU sat in the child process.

Model unload: not re-measured in this audit. The 16:03 harness reported 1092.4 MB down to 378.3 MB after unload. Treat that as a prior same-day measurement, not a new one.

## Not measured, and not estimated

- Microphone device open latency.
- Push-to-talk hook to capture start.
- Time until the tray is interactive and the shortcut is live.
- Warm rewrite with llama-server already resident across utterances. Each CLI run is a new process, so each rewrite includes server start. The drop from 7.8 s to 3.5 s shows variance, not a warm steady state.
- 4-core, 8 GB Windows. No number. No estimate is offered as a measurement.

## Linux ASR bench (different platform)

Stored in `bench/results/`, scored 2026-11-15 13:23 UTC, hypotheses from 12:55 UTC. Host: EPYC 9655, 4 inference threads in the bench harness. This is not the Windows exe.

Default model `parakeet-tdt-0.6b-v2-int8`: load 1127 ms, loaded RSS 875 MB, peak RSS 1771 MB, median RTF 0.0401, estimated 401 ms for 10 s of audio. That estimate is `median_rtf * 10000`. It is a bench extrapolation, not a stopwatch on Windows.
