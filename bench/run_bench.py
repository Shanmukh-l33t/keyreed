#!/usr/bin/env python3
"""KeyReed speech benchmark runner. Standard library only (Windows).

Runs `keyreed.exe --transcribe-wav` over the corpus manifests and stores every run's own stage
timings plus the process CPU time and peak working set measured from outside.

  python3 bench/run_bench.py --exe target/release/keyreed.exe --tag v0.5.0 \
      --models parakeet-tdt-0.6b-v2-int8 --filter tts_060s --repeat 3 [--live] [--mode exact] [--llm none]

Each clip is one fresh process: run 1 is the cold run (model load included), runs 2..N are warm.
`--live` asks the binary to feed the audio through the streaming session in real time, exactly
like a held hotkey (new in v0.5.0; the v0.4.0 binary does not have it).
Results: bench/results/<tag>/<model>[.live].json  -> score with bench/score_bench.py
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import platform
import subprocess
import time
from pathlib import Path

BENCH = Path(__file__).resolve().parent


class PMC(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("PageFaultCount", wt.DWORD), ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t), ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t), ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t), ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t)]


def proc_stats(handle):
    """(cpu seconds, peak working set MB) of a finished child, read from its still-open handle."""
    k32, psapi = ctypes.windll.kernel32, ctypes.windll.psapi
    c, e, kt, ut = wt.FILETIME(), wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
    k32.GetProcessTimes(wt.HANDLE(handle), ctypes.byref(c), ctypes.byref(e), ctypes.byref(kt), ctypes.byref(ut))
    cpu = sum(((t.dwHighDateTime << 32) | t.dwLowDateTime) for t in (kt, ut)) / 1e7
    pmc = PMC(); pmc.cb = ctypes.sizeof(PMC)
    psapi.GetProcessMemoryInfo(wt.HANDLE(handle), ctypes.byref(pmc), pmc.cb)
    return round(cpu, 2), round(pmc.PeakWorkingSetSize / 1048576, 1)


def parse_json_stream(text):
    dec, out, i = json.JSONDecoder(), [], 0
    while True:
        i = text.find("{", i)
        if i < 0:
            return out
        try:
            obj, end = dec.raw_decode(text, i)
            out.append(obj); i = end
        except json.JSONDecodeError:
            i += 1


def cpu_name():
    try:
        r = subprocess.run(["powershell", "-NoProfile", "-Command",
                            "(Get-CimInstance Win32_Processor).Name; [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB)"],
                           capture_output=True, text=True, timeout=30)
        lines = [l.strip() for l in r.stdout.splitlines() if l.strip()]
        return lines[0], int(lines[1])
    except Exception:
        return platform.processor(), None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", required=True)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--models", default="parakeet-tdt-0.6b-v2-int8")
    ap.add_argument("--filter", default="", help="comma-separated substrings; a clip runs if its id contains any")
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--mode", default="exact")
    ap.add_argument("--llm", default="none")
    ap.add_argument("--live", action="store_true")
    ap.add_argument("--max-seconds", type=float, default=1e9, help="skip clips longer than this")
    ap.add_argument("--extra", default="", help="extra arguments appended to the keyreed command line")
    a = ap.parse_args()

    clips = []
    for mf in sorted((BENCH / "corpus").glob("*/manifest.jsonl")):
        clips += [json.loads(l) for l in mf.read_text(encoding="utf-8").splitlines() if l.strip()]
    wanted = [f for f in a.filter.split(",") if f]
    clips = [c for c in clips if (not wanted or any(f in c["id"] for f in wanted)) and c["duration_s"] <= a.max_seconds]
    cpu, ram = cpu_name()
    outdir = BENCH / "results" / a.tag
    outdir.mkdir(parents=True, exist_ok=True)

    for model in a.models.split(","):
        records = []
        for c in clips:
            cmd = [a.exe, "--transcribe-wav", str(BENCH / "corpus" / c["wav"]), "--asr-model", model,
                   "--llm-model", a.llm, "--mode", a.mode, "--repeat", str(a.repeat)]
            if a.live:
                cmd.append("--live")
            cmd += a.extra.split()
            t0 = time.perf_counter()
            p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            out, err = p.communicate()
            wall = time.perf_counter() - t0
            cpu_s, peak_mb = proc_stats(p._handle)
            runs = parse_json_stream(out.decode("utf-8", "replace"))
            rec = {"id": c["id"], "duration_s": c["duration_s"], "process_wall_s": round(wall, 2),
                   "process_cpu_s": cpu_s, "peak_working_set_mb": peak_mb, "exit_code": p.returncode, "runs": runs}
            if p.returncode != 0 or len(runs) != a.repeat:
                rec["stderr"] = err.decode("utf-8", "replace")[-800:]
            records.append(rec)
            last = runs[-1]["result"]["metrics"] if runs else {}
            print(f"{model} {c['id']:34s} {c['duration_s']:7.1f}s  cold_total={runs[0]['result']['metrics'].get('total_release_to_ready_ms', -1) if runs else -1:9.0f}ms"
                  f"  warm_total={last.get('total_release_to_ready_ms', -1):9.0f}ms  asr={last.get('asr_ms', -1):8.0f}ms  peak={peak_mb}MB", flush=True)
        name = f"{model}{'.live' if a.live else ''}.json"
        (outdir / name).write_text(json.dumps({
            "tag": a.tag, "model": model, "exe": a.exe, "live": a.live, "mode": a.mode, "llm": a.llm,
            "repeat": a.repeat, "cpu": cpu, "ram_gb": ram, "date": time.strftime("%Y-%m-%d %H:%M"),
            "records": records}, indent=1, ensure_ascii=False), encoding="utf-8")
        print("wrote", outdir / name)


if __name__ == "__main__":
    main()
