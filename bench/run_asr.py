#!/usr/bin/env python3
"""
Isolated single-model ASR benchmark runner.
Runs one model per invocation with 4 CPU threads so model load time and peak RSS are uncontaminated.
"""
import argparse
import glob
import json
import os
import resource
import shutil
import subprocess
import tarfile
import time
from pathlib import Path

import numpy as np
import psutil
import sherpa_onnx
import soundfile as sf
from faster_whisper import WhisperModel

BENCH_DIR = Path("/home/computebox/voice-input/bench")
MODELS_DIR = BENCH_DIR / "models"
RESULTS_DIR = BENCH_DIR / "results"
SHERPA_BASE = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"

SHERPA_SPECS = {
    "parakeet-tdt-0.6b-v2-int8": {
        "archive": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
        "folder": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        "kind": "transducer_nemo",
        "langs": {"en"},
        "keep": True,
    },
    "parakeet-tdt-0.6b-v3-int8": {
        "archive": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
        "folder": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        "kind": "transducer_nemo",
        "langs": {"en", "es"},
        "keep": False,
    },
    "moonshine-tiny-en-int8": {
        "archive": "sherpa-onnx-moonshine-tiny-en-int8.tar.bz2",
        "folder": "sherpa-onnx-moonshine-tiny-en-int8",
        "kind": "moonshine",
        "langs": {"en"},
        "keep": True,
    },
    "moonshine-base-en-int8": {
        "archive": "sherpa-onnx-moonshine-base-en-int8.tar.bz2",
        "folder": "sherpa-onnx-moonshine-base-en-int8",
        "kind": "moonshine",
        "langs": {"en"},
        "keep": True,
    },
    "sense-voice-small-int8": {
        "archive": "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2",
        "folder": "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
        "kind": "sense_voice",
        "langs": {"en"},
        "keep": False,
    },
    "sherpa-whisper-tiny.en-int8": {
        "archive": "sherpa-onnx-whisper-tiny.en.tar.bz2",
        "folder": "sherpa-onnx-whisper-tiny.en",
        "kind": "whisper",
        "langs": {"en"},
        "keep": False,
    },
    "sherpa-whisper-base.en-int8": {
        "archive": "sherpa-onnx-whisper-base.en.tar.bz2",
        "folder": "sherpa-onnx-whisper-base.en",
        "kind": "whisper",
        "langs": {"en"},
        "keep": False,
    },
    "sherpa-whisper-small.en-int8": {
        "archive": "sherpa-onnx-whisper-small.en.tar.bz2",
        "folder": "sherpa-onnx-whisper-small.en",
        "kind": "whisper",
        "langs": {"en"},
        "keep": False,
    },
}

FASTER_WHISPER_SPECS = {
    "fw-tiny.en-int8": ("tiny.en", {"en"}),
    "fw-base.en-int8": ("base.en", {"en"}),
    "fw-small.en-int8": ("small.en", {"en"}),
    "fw-distil-small.en-int8": ("distil-small.en", {"en"}),
    "fw-large-v3-turbo-int8": ("deepdml/faster-whisper-large-v3-turbo-ct2", {"en", "hi", "es"}),
}


def dir_size_mb(path: Path) -> float:
    total = 0
    for root, _, files in os.walk(path):
        for fn in files:
            p = os.path.join(root, fn)
            if not os.path.islink(p):
                total += os.path.getsize(p)
    return round(total / 1048576.0, 1)


def pick_file(folder: Path, suffixes: list[str]) -> str:
    all_files = sorted(folder.iterdir())
    for suf in suffixes:
        for p in all_files:
            if p.name == suf or p.name.endswith(suf):
                return str(p)
    raise FileNotFoundError(f"None of {suffixes} in {folder}")


def load_sherpa_model(spec: dict, threads: int):
    MODELS_DIR.mkdir(parents=True, exist_ok=True)
    folder = MODELS_DIR / spec["folder"]
    if not folder.exists():
        arc = MODELS_DIR / spec["archive"]
        url = f"{SHERPA_BASE}/{spec['archive']}"
        subprocess.run(["curl", "-sSL", url, "-o", str(arc)], check=True)
        with tarfile.open(arc, "r:bz2") as tf:
            tf.extractall(MODELS_DIR)
        arc.unlink(missing_ok=True)

    disk_mb = dir_size_mb(folder)
    t0 = time.perf_counter()
    kind = spec["kind"]
    if kind == "transducer_nemo":
        rec = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=pick_file(folder, ["encoder.int8.onnx", "encoder.onnx"]),
            decoder=pick_file(folder, ["decoder.int8.onnx", "decoder.onnx"]),
            joiner=pick_file(folder, ["joiner.int8.onnx", "joiner.onnx"]),
            tokens=pick_file(folder, ["tokens.txt"]),
            num_threads=threads,
            provider="cpu",
            model_type="nemo_transducer",
        )
    elif kind == "moonshine":
        rec = sherpa_onnx.OfflineRecognizer.from_moonshine(
            preprocessor=pick_file(folder, ["preprocess.onnx"]),
            encoder=pick_file(folder, ["encode.int8.onnx", "encode.onnx"]),
            uncached_decoder=pick_file(folder, ["uncached_decode.int8.onnx", "uncached_decode.onnx"]),
            cached_decoder=pick_file(folder, ["cached_decode.int8.onnx", "cached_decode.onnx"]),
            tokens=pick_file(folder, ["tokens.txt"]),
            num_threads=threads,
            provider="cpu",
        )
    elif kind == "sense_voice":
        rec = sherpa_onnx.OfflineRecognizer.from_sense_voice(
            model=pick_file(folder, ["model.int8.onnx", "model.onnx"]),
            tokens=pick_file(folder, ["tokens.txt"]),
            num_threads=threads,
            use_itn=True,
            provider="cpu",
        )
    elif kind == "whisper":
        rec = sherpa_onnx.OfflineRecognizer.from_whisper(
            encoder=pick_file(folder, ["-encoder.int8.onnx", "-encoder.onnx"]),
            decoder=pick_file(folder, ["-decoder.int8.onnx", "-decoder.onnx"]),
            tokens=pick_file(folder, ["-tokens.txt", "tokens.txt"]),
            language="en",
            task="transcribe",
            num_threads=threads,
            provider="cpu",
        )
    else:
        raise ValueError(kind)
    load_ms = (time.perf_counter() - t0) * 1000.0
    return rec, folder, disk_mb, load_ms


def run_one(model_id: str, manifest_path: Path, threads: int, turbo_subset: bool):
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    rows = [json.loads(l) for l in open(manifest_path, encoding="utf-8")]
    proc = psutil.Process(os.getpid())

    if model_id in SHERPA_SPECS:
        spec = SHERPA_SPECS[model_id]
        rec, folder, disk_mb, load_ms = load_sherpa_model(spec, threads)
        rss_after_load = proc.memory_info().rss / 1048576.0
        out_records = []
        for r in rows:
            lang = r.get("lang", "en")
            if lang not in spec["langs"] and r["category"].startswith("multilingual_"):
                continue
            samples, sr = sf.read(r["path"], dtype="float32")
            # Sherpa Whisper handles <=30s per stream; chunk long_form into 25s windows if whisper/moonshine
            t0 = time.perf_counter()
            if spec["kind"] in ("whisper", "moonshine") and len(samples) > 16000 * 28:
                pieces = []
                step = 16000 * 25
                for off in range(0, len(samples), step):
                    st = rec.create_stream()
                    st.accept_waveform(sr, samples[off : off + step])
                    rec.decode_stream(st)
                    pieces.append(st.result.text.strip())
                hyp = " ".join(p for p in pieces if p)
            else:
                st = rec.create_stream()
                st.accept_waveform(sr, samples)
                rec.decode_stream(st)
                hyp = st.result.text.strip()
            wall_s = time.perf_counter() - t0
            out_records.append({
                "id": r["id"],
                "category": r["category"],
                "duration_s": r["duration_s"],
                "wall_s": round(wall_s, 4),
                "rtf": round(wall_s / max(0.1, r["duration_s"]), 4),
                "ref": r["ref"],
                "ref_written": r.get("ref_written"),
                "entities": r.get("entities"),
                "hyp": hyp,
            })
        peak_rss_mb = max(
            proc.memory_info().rss / 1048576.0,
            resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024.0,
        )
        del rec
        if not spec["keep"]:
            shutil.rmtree(folder, ignore_errors=True)
    elif model_id in FASTER_WHISPER_SPECS:
        fw_repo, langs = FASTER_WHISPER_SPECS[model_id]
        dl_root = MODELS_DIR / f"ct2_{model_id}"
        dl_root.mkdir(parents=True, exist_ok=True)
        t0 = time.perf_counter()
        model = WhisperModel(fw_repo, device="cpu", compute_type="int8", cpu_threads=threads, download_root=str(dl_root))
        load_ms = (time.perf_counter() - t0) * 1000.0
        disk_mb = dir_size_mb(dl_root)
        rss_after_load = proc.memory_info().rss / 1048576.0
        out_records = []
        per_cat_seen = {}
        for r in rows:
            lang = r.get("lang", "en")
            if lang not in langs and r["category"].startswith("multilingual_"):
                continue
            if turbo_subset and model_id == "fw-large-v3-turbo-int8":
                c = per_cat_seen.get(r["category"], 0)
                limit = 2 if r["category"] == "long_form" else 8
                if c >= limit:
                    continue
                per_cat_seen[r["category"]] = c + 1
            # Pass float32 numpy array directly to bypass PyAV 19!
            samples, _ = sf.read(r["path"], dtype="float32")
            t1 = time.perf_counter()
            lang_arg = lang if r["category"].startswith("multilingual_") else "en"
            segs, _ = model.transcribe(samples, language=lang_arg if ".en" not in fw_repo else None, beam_size=1, vad_filter=False)
            hyp = " ".join(s.text.strip() for s in segs).strip()
            wall_s = time.perf_counter() - t1
            out_records.append({
                "id": r["id"],
                "category": r["category"],
                "duration_s": r["duration_s"],
                "wall_s": round(wall_s, 4),
                "rtf": round(wall_s / max(0.1, r["duration_s"]), 4),
                "ref": r["ref"],
                "ref_written": r.get("ref_written"),
                "entities": r.get("entities"),
                "hyp": hyp,
            })
        peak_rss_mb = max(
            proc.memory_info().rss / 1048576.0,
            resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024.0,
        )
        del model
        shutil.rmtree(dl_root, ignore_errors=True)
    else:
        raise SystemExit(f"Unknown model {model_id}")

    out_file = RESULTS_DIR / f"{model_id}.json"
    summary = {
        "model_id": model_id,
        "threads": threads,
        "disk_mb": round(disk_mb, 1),
        "load_ms": round(load_ms, 1),
        "loaded_rss_mb": round(rss_after_load, 1),
        "peak_rss_mb": round(peak_rss_mb, 1),
        "utterances_evaluated": len(out_records),
        "records": out_records,
    }
    with open(out_file, "w", encoding="utf-8") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)
    print(
        f"[DONE] {model_id:28s} | clips={len(out_records):3d} | disk={disk_mb:5.1f}MB | load={load_ms:6.0f}ms | peakRSS={peak_rss_mb:6.1f}MB"
    )


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--manifest", default=str(BENCH_DIR / "corpus" / "manifest.jsonl"))
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--turbo-subset", action="store_true", default=True)
    args = ap.parse_args()
    run_one(args.model, Path(args.manifest), args.threads, args.turbo_subset)
