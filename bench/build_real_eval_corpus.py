#!/usr/bin/env python3
"""Builds a small REAL-speech evaluation corpus (no TTS) for KeyReed, stdlib + ffmpeg only.

  python3 bench/build_real_eval_corpus.py            # -> bench/corpus/real_v1/ (gitignored)

Sources are the ones the existing bench builders already use:
  - LibriSpeech test-clean (openslr/librispeech_asr, CC-BY-4.0): en-US read speech, short + concatenated long.
  - CMU ARCTIC ksp (festvox.org, permissive): en-IN read speech, same prompts/URLs as make_librispeech_corpus.py.
  - EdAcc (edinburghcstr/edacc, CC-BY-SA-4.0): real spontaneous accented English, references are the dataset's own
    transcripts. Selected by fixed row offsets, so the same rows are picked on every run.

Every manifest row carries the source utterance id + wav sha256, so a re-run is verifiable, and both the
`wav`/`paragraphs` keys (bench/run_bench.py + score_bench.py) and `path`/`ref` keys (legacy run_asr.py).
"""
import hashlib
import json
import subprocess
import sys
import time
import urllib.request
import wave
from pathlib import Path

BENCH = Path(__file__).resolve().parent
OUT = BENCH / "corpus" / "real_v1"
WAVS = OUT / "wavs"
SR = 16000
API = "https://datasets-server.huggingface.co/rows"

# Fixed page offsets: EdAcc rows are ordered by accent block, so these pages give accent spread.
EDACC_PAGES = [0, 1800, 3600, 5400, 7200, 9000]
LS_PAGES = [0, 200, 400]  # LibriSpeech test.clean has 2620 rows
ARCTIC = "http://festvox.org/cmu_arctic/cmu_arctic/cmu_us_ksp_arctic"
ARCTIC_UTTS = ["arctic_a0001", "arctic_a0002", "arctic_a0003", "arctic_a0004"]


def http_get(url, dest=None, tries=4):
    last = None
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "keyreed-bench/1.0"})
            with urllib.request.urlopen(req, timeout=90) as r:
                data = r.read()
            if dest is None:
                return data
            Path(dest).write_bytes(data)
            return data
        except Exception as e:  # transient HF 5xx / expired asset URL
            last = e
            time.sleep(1.5 * (i + 1))
    raise RuntimeError(f"GET failed {url[:120]}: {last}")


def hf_rows(dataset, config, split, offset, length=100):
    url = f"{API}?dataset={dataset}&config={config}&split={split}&offset={offset}&length={length}"
    return json.loads(http_get(url).decode())["rows"]


def to_wav(src_bytes: bytes, suffix: str, dest: Path) -> float:
    """Decode any container to 16 kHz mono s16 PCM; returns duration in seconds."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_name(dest.stem + ".src" + suffix)
    tmp.write_bytes(src_bytes)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(tmp), "-ac", "1", "-ar", str(SR),
                    "-c:a", "pcm_s16le", str(dest)], check=True)
    tmp.unlink(missing_ok=True)
    return wav_duration(dest)


def wav_duration(path: Path) -> float:
    with wave.open(str(path), "rb") as w:
        return round(w.getnframes() / w.getframerate(), 3)


def wav_pcm(path: Path):
    with wave.open(str(path), "rb") as w:
        assert w.getframerate() == SR and w.getnchannels() == 1 and w.getsampwidth() == 2, path
        return w.readframes(w.getnframes())


def write_concat(dest: Path, parts, pause_s: float = 0.35) -> float:
    gap = b"\x00\x00" * int(pause_s * SR)
    with wave.open(str(dest), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        for i, (pcm, _) in enumerate(parts):
            if i:
                w.writeframes(gap)
            w.writeframes(pcm)
    return wav_duration(dest)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def row(rid, path, ref, paragraphs, category, source, license_, accent, variant, dur, extra=None):
    r = {
        "id": rid,
        "wav": str(path.relative_to(BENCH / "corpus")).replace("\\", "/"),
        "path": str(path),
        "ref": " ".join(paragraphs),
        "reference": " ".join(paragraphs),
        "paragraphs": paragraphs,
        "category": category,
        "source": source,
        "license": license_,
        "accent": accent,
        "variant": variant,
        "duration_s": dur,
        "sample_rate": SR,
        "sha256": sha256(path),
    }
    r.update(extra or {})
    return r


def clean_edacc_text(t: str) -> str:
    t = " ".join(t.split())
    if t and t[-1] not in ".?!":
        t += "."
    return t


def build_librispeech(manifest):
    ls_dir = WAVS / "librispeech"
    picked, chapters, downloads = [], {}, 0
    for off in LS_PAGES:
        for r in hf_rows("openslr/librispeech_asr", "all", "test.clean", off):
            r = r["row"]
            if len(picked) >= 12 and any(sum(u[2] for u in v) >= 55 for v in chapters.values()):
                break
            if downloads >= 40:  # cap: selection order is fixed, so the cap is deterministic too
                break
            uid = r["id"]
            try:
                dur = to_wav(http_get(f"{r['audio'][0]['src']}"), ".flac", ls_dir / f"{uid}.wav")
            except Exception as e:
                print(f"  skip {uid}: {e}")
                continue
            downloads += 1
            text = r["text"].strip()
            chapters.setdefault((r["speaker_id"], r["chapter_id"]), []).append((uid, text, dur))
            if 3.0 <= dur <= 12.0 and len(picked) < 12:
                p = ls_dir / f"{uid}.wav"
                picked.append((uid, text, dur))
                manifest.append(row(f"ls_{uid}", p, [text], [text], "clean_english",
                                    f"openslr/librispeech_asr:test-clean:{uid}", "CC-BY-4.0",
                                    "en-US", "read", dur, {"row_speaker": r["speaker_id"]}))
    # long form: consecutive utterances concatenated in chapter order (same idea as make_librispeech_corpus.py,
    # which also spans chapters for its 300/600 s clips); a chapter change is a paragraph boundary.
    order = [(spk, chap, uid, text, dur) for (spk, chap), utts in chapters.items() for uid, text, dur in utts]
    idx = 0
    for n, target in enumerate((45.0, 120.0)):
        parts, paras, acc = [], [], 0.0
        while idx < len(order) and acc < target:
            spk, chap, uid, text, dur = order[idx]
            parts.append((wav_pcm(ls_dir / f"{uid}.wav"), text))
            if not paras or paras[-1][0] != (spk, chap):
                paras.append(((spk, chap), [text]))
            else:
                paras[-1][1].append(text)
            acc += dur + 0.35
            idx += 1
        if acc < 40:
            break
        cid = f"ls_long_{n:02d}"
        p = ls_dir / f"{cid}.wav"
        d = write_concat(p, parts)
        manifest.append(row(cid, p, [" ".join(t) for _, t in paras], [" ".join(t) for _, t in paras],
                            "long_form", f"openslr/librispeech_asr:concat:{order[idx-1][0]}_{order[idx-1][1]}",
                            "CC-BY-4.0", "en-US", "read_concat", d))
    keep = {Path(m["path"]).name for m in manifest}
    for f in ls_dir.glob("*.wav"):
        if f.name not in keep:
            f.unlink()
    return len(picked)


def build_arctic(manifest):
    txt = http_get(f"{ARCTIC}/etc/txt.done.data").decode("utf-8", "replace")
    prompts = {}
    for line in txt.splitlines():
        s = line.strip()
        if s.startswith('(') and '"' in s:
            key = s[1:].split()[0]
            prompts[key] = s.split('"')[1].strip()
    d = WAVS / "cmu_arctic_ksp"
    for uid in ARCTIC_UTTS:
        raw = http_get(f"{ARCTIC}/wav/{uid}.wav")
        p = d / f"ksp_{uid}.wav"
        dur = to_wav(raw, ".wav", p)
        manifest.append(row(f"in_ksp_{uid}", p, [prompts[uid]], [prompts[uid]], "indian_english",
                            f"cmu_arctic:cmu_us_ksp_arctic:{uid}", "CMU-ARCTIC-Permissive",
                            "en-IN", "read", dur))


def build_edacc(manifest):
    pages = []
    for off in EDACC_PAGES:
        pages.append([(r["row_idx"], r["row"]) for r in hf_rows("edinburghcstr/edacc", "default", "test", off)])
    # round-robin the pages so accent blocks from different pages interleave; page order alone is one accent.
    cands = []
    for i in range(max(len(p) for p in pages)):
        cands += [p[i] for p in pages if i < len(p)]
    ed_dir = WAVS / "edacc"
    quotas = {"indian_english": 10, "other_accents": 10, "conversational": 6}
    taken = {k: 0 for k in quotas}
    speakers, per_accent = set(), {}
    for row_idx, rr in cands:
        if all(taken[k] >= quotas[k] for k in quotas):
            break
        accent = rr["accent"]
        text = clean_edacc_text(rr["text"])
        tmp = ed_dir / f"probe_{row_idx}.wav"
        try:
            dur = to_wav(http_get(rr["audio"][0]["src"]), ".wav", tmp)
        except Exception as e:
            print(f"  skip edacc {row_idx}: {e}")
            continue
        if not (3.5 <= dur <= 22.0):
            tmp.unlink(missing_ok=True)
            continue
        indian = "indian" in accent.lower() or "india" in rr.get("l1", "").lower()
        if indian and taken["indian_english"] < quotas["indian_english"]:
            cat, cid = "indian_english", f"in_edacc_{taken['indian_english']:02d}"
            taken["indian_english"] += 1
        elif not indian and dur >= 9.0 and taken["conversational"] < quotas["conversational"] and rr["speaker"] not in speakers:
            cat, cid = "conversational", f"conv_edacc_{taken['conversational']:02d}"
            taken["conversational"] += 1
        elif not indian and taken["other_accents"] < quotas["other_accents"] and per_accent.get(accent, 0) < 3:
            cat, cid = "other_accents", f"acc_edacc_{taken['other_accents']:02d}"
            taken["other_accents"] += 1
        else:
            tmp.unlink(missing_ok=True)
            continue
        p = ed_dir / f"{cid}.wav"
        tmp.replace(p)
        speakers.add(rr["speaker"])
        per_accent[accent] = per_accent.get(accent, 0) + 1
        manifest.append(row(cid, p, [text], [text], cat,
                            f"edinburghcstr/edacc:test:{row_idx}:{rr['speaker']}", "CC-BY-SA-4.0",
                            accent.lower(), "spontaneous", dur,
                            {"row_idx": row_idx, "speaker": rr["speaker"], "l1": rr["l1"],
                             "gender": rr["gender"]}))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    manifest = []
    print("LibriSpeech test-clean (short + concatenated long)...")
    n = build_librispeech(manifest)
    print(f"  {n} short + {sum(1 for m in manifest if m['category'] == 'long_form')} long")
    print("CMU ARCTIC ksp (en-IN read)...")
    build_arctic(manifest)
    print("EdAcc (spontaneous Indian + other accents)...")
    build_edacc(manifest)

    manifest.sort(key=lambda m: (m["category"], m["id"]))
    ids = [m["id"] for m in manifest]
    assert len(ids) == len(set(ids)), "duplicate ids"
    assert 30 <= len(manifest) <= 50, f"clip count {len(manifest)} outside 30-50"
    for m in manifest:
        assert m["paragraphs"] and m["reference"].strip(), m["id"]
        assert Path(m["path"]).exists()
        assert abs(wav_duration(Path(m["path"])) - m["duration_s"]) < 0.01, m["id"]

    with open(OUT / "manifest.jsonl", "w", encoding="utf-8") as f:
        for m in manifest:
            f.write(json.dumps(m, ensure_ascii=False) + "\n")

    from collections import Counter
    cnt = Counter(m["category"] for m in manifest)
    total = sum(m["duration_s"] for m in manifest)
    print(f"\n{len(manifest)} clips, {total/60:.1f} min -> {OUT/'manifest.jsonl'}")
    for k in sorted(cnt):
        g = [m for m in manifest if m["category"] == k]
        print(f"  {k:18s} {cnt[k]:2d} clips  {sum(m['duration_s'] for m in g)/60:.1f} min  "
              f"{min(m['duration_s'] for m in g):5.1f}-{max(m['duration_s'] for m in g):5.1f}s")


if __name__ == "__main__":
    sys.exit(main())