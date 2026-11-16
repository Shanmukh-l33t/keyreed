#!/usr/bin/env python3
"""Builds the real-speech (LibriSpeech test-clean, CC BY 4.0) part of the KeyReed benchmark.
Standard library only; uses ffmpeg on PATH to decode FLAC to 16 kHz mono s16 PCM WAV.

Reads bench/cache/test-clean.tar.gz and writes bench/corpus/librispeech/*.wav + manifest.jsonl
with consecutive utterances from the same speaker/chapter concatenated into 5s, 15s, 30s,
60s, 120s, 300s, and 600s recordings with seeded inter-utterance pauses and paragraph breaks
between chapters/sections.
"""
import array
import json
import random
import re
import subprocess
import tarfile
import urllib.request
import wave
from collections import defaultdict
from pathlib import Path

BENCH = Path(__file__).resolve().parent
TAR = BENCH / "cache" / "test-clean.tar.gz"
OUT = BENCH / "corpus" / "librispeech"
SR = 16000


def flac_bytes_to_pcm(raw_flac: bytes) -> array.array:
    p = subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-i", "pipe:0", "-ac", "1", "-ar", str(SR), "-f", "s16le", "pipe:1"],
        input=raw_flac,
        capture_output=True,
        check=True,
    )
    a = array.array("h")
    a.frombytes(p.stdout)
    return a


def silence(sec: float) -> array.array:
    return array.array("h", [0]) * int(sec * SR)


def write_wav(path: Path, a: array.array):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(a.tobytes())


def sentence_case(raw_upper: str) -> str:
    s = raw_upper.strip().lower()
    if not s:
        return ""
    s = s[0].upper() + s[1:]
    if s[-1] not in ".?!":
        s += "."
    return s


def main():
    assert TAR.exists(), f"Missing {TAR}"
    OUT.mkdir(parents=True, exist_ok=True)

    # First pass: read all .trans.txt files from the tarball
    trans_by_chap = defaultdict(list)
    needed_flacs = {}
    with tarfile.open(TAR, "r:gz") as tf:
        members = tf.getmembers()
        by_name = {m.name: m for m in members}
        for m in members:
            if m.name.endswith(".trans.txt"):
                f = tf.extractfile(m)
                if not f:
                    continue
                prefix = m.name.rsplit("/", 1)[0]
                parts = prefix.split("/")
                spk, chap = parts[-2], parts[-1]
                for line in f.read().decode("utf-8").splitlines():
                    toks = line.strip().split(" ", 1)
                    if len(toks) == 2:
                        uid, txt = toks
                        flac_name = f"{prefix}/{uid}.flac"
                        if flac_name in by_name:
                            trans_by_chap[(spk, chap)].append((uid, sentence_case(txt), flac_name))

        # Sort chapters deterministically so every run produces the exact same clips
        chapters = sorted(trans_by_chap.keys())
        # Targets: (clip_id, target_seconds, start_chapter_idx, utterances_per_paragraph)
        specs = [
            ("ls_005s_clean_1", 5.0, 0, 99),
            ("ls_005s_clean_2", 5.0, 1, 99),
            ("ls_015s_clean_1", 15.0, 2, 99),
            ("ls_030s_clean_1", 30.0, 3, 99),
            ("ls_060s_clean_1", 60.0, 4, 4),
            ("ls_060s_clean_2", 60.0, 5, 4),
            ("ls_120s_clean_1", 120.0, 6, 4),
            ("ls_300s_clean_1", 300.0, 7, 5),
            ("ls_600s_clean_1", 600.0, 10, 5),
        ]

        # Collect which FLACs we actually need to extract (fast!)
        clip_utts = {}
        for cid, target_s, ch_idx, _ in specs:
            utts = []
            est_s = 0.0
            ci = ch_idx
            while est_s < target_s and ci < len(chapters):
                for u in trans_by_chap[chapters[ci]]:
                    utts.append(u)
                    needed_flacs[u[2]] = None
                    # average word ~0.38s
                    est_s += len(u[1].split()) * 0.38 + 0.5
                    if est_s >= target_s * 1.15:
                        break
                ci += 1
            clip_utts[cid] = utts

        for flac_name in sorted(needed_flacs.keys()):
            f = tf.extractfile(by_name[flac_name])
            assert f is not None
            needed_flacs[flac_name] = flac_bytes_to_pcm(f.read())

    manifest = []
    for cid, target_s, _, para_len in specs:
        rng = random.Random(cid)
        audio = silence(0.35)
        paras = []
        cur_para = []
        para_ends = []
        for idx, (_, sent, flac_name) in enumerate(clip_utts[cid]):
            pcm = needed_flacs[flac_name]
            if idx > 0:
                if len(cur_para) >= para_len:
                    paras.append(" ".join(cur_para))
                    para_ends.append(round(len(audio) / SR, 2))
                    cur_para = []
                    audio += silence(rng.uniform(1.6, 2.2))
                else:
                    audio += silence(rng.uniform(0.35, 0.55))
            audio += pcm
            cur_para.append(sent)
            if len(audio) / SR >= target_s and len(cur_para) >= min(2, para_len):
                break
        if cur_para:
            paras.append(" ".join(cur_para))
            para_ends.append(round(len(audio) / SR, 2))
        audio += silence(0.45)
        write_wav(OUT / f"{cid}.wav", audio)
        manifest.append({
            "id": cid,
            "wav": f"librispeech/{cid}.wav",
            "source": "librispeech_test_clean",
            "license": "CC-BY-4.0",
            "accent": "en-US",
            "variant": "real_read",
            "duration_s": round(len(audio) / SR, 2),
            "paragraphs": paras,
            "reference": " ".join(paras),
            "paragraph_end_s": para_ends,
        })

    with open(OUT / "manifest.jsonl", "w", encoding="utf-8") as f:
        for m in manifest:
            f.write(json.dumps(m, ensure_ascii=False) + "\n")
    total_min = sum(m["duration_s"] for m in manifest) / 60.0
    print(f"Built {len(manifest)} LibriSpeech real-speech clips ({total_min:.1f} min) -> {OUT}")
    build_cmu_arctic_indian()


def fetch_file(url: str, dest: Path):
    if not dest.exists():
        subprocess.run(["curl", "-4", "-fsSL", "-o", str(dest), url], check=True)


def build_cmu_arctic_indian():
    out_dir = BENCH / "corpus" / "cmu_arctic_indian"
    cache_dir = BENCH / "cache" / "cmu_arctic"
    out_dir.mkdir(parents=True, exist_ok=True)
    cache_dir.mkdir(parents=True, exist_ok=True)
    base = "http://festvox.org/cmu_arctic/cmu_arctic"
    txt_url = f"{base}/cmu_us_ksp_arctic/etc/txt.done.data"
    txt_cache = cache_dir / "txt.done.data"
    fetch_file(txt_url, txt_cache)
    prompts = {}
    for line in txt_cache.read_text(encoding="utf-8", errors="replace").splitlines():
        m = re.match(r'\(\s*(\S+)\s+"(.+)"\s*\)', line.strip())
        if m:
            prompts[m.group(1)] = m.group(2)

    specs = [
        ("in_real_005s_ksp", "ksp", ["arctic_a0001"], 99),
        ("in_real_015s_ksp", "ksp", ["arctic_a0002", "arctic_a0003", "arctic_a0004", "arctic_a0005"], 99),
        ("in_real_030s_ksp", "ksp", [f"arctic_a00{i:02d}" for i in range(6, 14)], 99),
        ("in_real_060s_ksp", "ksp", [f"arctic_a00{i:02d}" for i in range(14, 30)], 8),
    ]
    manifest = []
    for cid, spk, uids, para_len in specs:
        rng = random.Random(cid)
        audio = silence(0.35)
        paras, cur_para, para_ends = [], [], []
        for idx, uid in enumerate(uids):
            wav_cache = cache_dir / f"{spk}_{uid}.wav"
            fetch_file(f"{base}/cmu_us_{spk}_arctic/wav/{uid}.wav", wav_cache)
            pcm = flac_bytes_to_pcm(wav_cache.read_bytes())
            if idx > 0:
                if len(cur_para) >= para_len:
                    paras.append(" ".join(cur_para))
                    para_ends.append(round(len(audio) / SR, 2))
                    cur_para = []
                    audio += silence(rng.uniform(1.6, 2.1))
                else:
                    audio += silence(rng.uniform(0.35, 0.55))
            audio += pcm
            cur_para.append(prompts[uid])
        if cur_para:
            paras.append(" ".join(cur_para))
            para_ends.append(round(len(audio) / SR, 2))
        audio += silence(0.45)
        write_wav(out_dir / f"{cid}.wav", audio)
        manifest.append({
            "id": cid,
            "wav": f"cmu_arctic_indian/{cid}.wav",
            "source": "cmu_arctic_indian",
            "license": "CMU-ARCTIC-Permissive",
            "accent": "en-IN",
            "variant": "real_read",
            "duration_s": round(len(audio) / SR, 2),
            "paragraphs": paras,
            "reference": " ".join(paras),
            "paragraph_end_s": para_ends,
        })
    with open(out_dir / "manifest.jsonl", "w", encoding="utf-8") as f:
        for m in manifest:
            f.write(json.dumps(m, ensure_ascii=False) + "\n")
    print(f"Built {len(manifest)} CMU ARCTIC real Indian-English clips -> {out_dir}")


if __name__ == "__main__":
    main()
