#!/usr/bin/env python3
"""Builds the synthetic (TTS) half of the KeyReed speech benchmark. Standard library only.

  python3 bench/make_tts_corpus.py            # writes bench/corpus/tts/*.wav + bench/corpus/tts/manifest.jsonl

Every clip is SYNTHETIC SPEECH (Windows OneCore TTS voices) and is labelled `"source": "tts"` in
the manifest. Sentences are synthesised one by one and joined with seeded pauses, so the manifest
knows the exact reference text, the paragraph boundaries and where every pause is.
Needs: Windows, PowerShell, ffmpeg on PATH. Voices that are not installed are skipped.
"""
import array
import hashlib
import json
import random
import subprocess
import sys
import wave
from pathlib import Path

BENCH = Path(__file__).resolve().parent
OUT = BENCH / "corpus" / "tts"
CACHE = BENCH / "cache" / "tts_sentences"
SR = 16000
US_VOICES = ["David", "Zira", "Mark"]
IN_VOICES = ["Heera", "Ravi"]


def synth_all(jobs):
    """jobs: {(voice, rate, text)} -> {key: wav path (16 kHz mono s16)}"""
    CACHE.mkdir(parents=True, exist_ok=True)
    out, todo = {}, []
    for voice, rate, text in sorted(jobs):
        h = hashlib.sha1(f"{voice}|{rate}|{text}".encode()).hexdigest()[:16]
        raw, final = CACHE / f"{h}.raw.wav", CACHE / f"{h}.wav"
        out[(voice, rate, text)] = final
        if not final.exists():
            todo.append({"voice": voice, "rate": rate, "text": text, "out": str(raw), "final": str(final)})
    if todo:
        jf = CACHE / "jobs.json"
        jf.write_text(json.dumps(todo), encoding="utf-8")
        r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                            str(BENCH / "tts_synth.ps1"), "-Jobs", str(jf)], capture_output=True, text=True)
        print(r.stdout.strip(), r.stderr.strip()[:2000])
        if r.returncode != 0:
            sys.exit("TTS synthesis failed")
        for j in todo:
            subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", j["out"], "-ac", "1", "-ar", str(SR),
                            "-c:a", "pcm_s16le", j["final"]], check=True)
            Path(j["out"]).unlink()
    return out


def read_pcm(path):
    with wave.open(str(path), "rb") as w:
        assert w.getframerate() == SR and w.getnchannels() == 1 and w.getsampwidth() == 2, path
        a = array.array("h")
        a.frombytes(w.readframes(w.getnframes()))
        return a


def trim(a, thresh=200, keep=int(0.05 * SR)):
    """Strip the TTS engine's own leading/trailing silence so the manifest pauses are the real ones."""
    n = len(a)
    i = next((k for k in range(n) if abs(a[k]) > thresh), 0)
    j = next((k for k in range(n - 1, -1, -1) if abs(a[k]) > thresh), n - 1)
    return a[max(0, i - keep): min(n, j + keep)]


def silence(sec):
    return array.array("h", [0]) * int(sec * SR)


def noise(n, rng, level):
    """Synthetic steady background noise (low-passed white noise). Labelled as synthetic in the manifest."""
    out, prev = array.array("h"), 0.0
    for _ in range(n):
        prev = 0.85 * prev + 0.15 * rng.uniform(-1.0, 1.0)
        out.append(int(prev * level))
    return out


def rms(a):
    return (sum(x * x for x in a) / max(1, len(a))) ** 0.5


def write_wav(path, a):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(a.tobytes())


def main():
    data = json.loads((BENCH / "corpus_text" / "passages.json").read_text(encoding="utf-8"))
    docs = {d["id"]: d for d in data["documents"]}
    order = [d["id"] for d in data["documents"]]
    all_paras = [(d, i) for d in order for i in range(len(docs[d]["paragraphs"]))]

    # (clip id, voice, rate, [(doc, para index)], variant)
    clips = []
    for vi, v in enumerate(US_VOICES + IN_VOICES):
        d = order[vi % len(order)]
        clips.append((f"tts_015s_{v.lower()}", v, 1.0, [(d, 1)], "normal", 3))  # first 3 sentences
        clips.append((f"tts_030s_{v.lower()}", v, 1.0, [(d, 0)], "normal", None))
        clips.append((f"tts_060s_{v.lower()}", v, 1.0, [(d, 0), (d, 1)], "normal", None))
        d2 = order[(vi + 2) % len(order)]
        clips.append((f"tts_120s_{v.lower()}", v, 1.0, [(d2, i) for i in range(4)], "normal", None))
    clips.append(("tts_060s_fast_zira", "Zira", 1.4, [("planning_notes", 0), ("planning_notes", 1), ("planning_notes", 2)], "fast", None))
    clips.append(("tts_060s_slow_david", "David", 0.75, [("howto_backup", 0), ("howto_backup", 1)], "slow", None))
    clips.append(("tts_060s_fast_ravi", "Ravi", 1.3, [("incident_note", 0), ("incident_note", 1), ("incident_note", 2)], "fast", None))
    clips.append(("tts_060s_longpause_heera", "Heera", 1.0, [("update_email", 2), ("update_email", 3)], "long_pauses", None))
    clips.append(("tts_060s_noisy_david", "David", 1.0, [("incident_note", 2), ("incident_note", 3)], "noisy_10db", None))
    clips.append(("tts_060s_quiet_zira", "Zira", 1.0, [("weekend_story", 0), ("weekend_story", 1)], "quiet", None))
    clips.append(("tts_300s_heera", "Heera", 1.0, all_paras[:10], "normal", None))
    clips.append(("tts_300s_zira", "Zira", 1.0, all_paras[10:], "normal", None))
    clips.append(("tts_600s_david", "David", 1.0, all_paras, "normal", None))
    shorts = [(f"tts_005s_{s['id']}_{v.lower()}", v, s) for s in data["short_items"] for v in ("David", "Heera")]

    jobs = set()
    for _, v, rate, paras, _, limit in clips:
        for d, pi in paras:
            for s in docs[d]["paragraphs"][pi][:limit]:
                jobs.add((v, rate, s))
    for _, v, s in shorts:
        jobs.add((v, 1.0, s["text"]))
    wavs = synth_all(jobs)

    OUT.mkdir(parents=True, exist_ok=True)
    manifest = []
    for cid, v, rate, paras, variant, limit in clips:
        rng = random.Random(cid)
        audio = silence(0.4)
        ref_paras, para_ends = [], []
        for k, (d, pi) in enumerate(paras):
            sents = docs[d]["paragraphs"][pi][:limit]
            # a new document is always a new thought, so it is a paragraph boundary as well
            if k:
                audio += silence(rng.uniform(1.5, 2.4))
            for si, s in enumerate(sents):
                if si:
                    gap = rng.uniform(0.3, 0.6)
                    if variant == "long_pauses" and si % 2 == 0:
                        gap = rng.uniform(0.9, 1.2)  # thinking pause inside a paragraph
                    audio += silence(gap)
                audio += trim(read_pcm(wavs[(v, rate, s)]))
            ref_paras.append(" ".join(sents))
            para_ends.append(round(len(audio) / SR, 2))
        audio += silence(0.5)
        if variant == "noisy_10db":
            level = rms(audio) / (10 ** (10 / 20)) / 0.33  # low-passed uniform noise has rms ~0.33*level
            nz = noise(len(audio), rng, level)
            audio = array.array("h", (max(-32768, min(32767, a + b)) for a, b in zip(audio, nz)))
        if variant == "quiet":
            audio = array.array("h", (int(x * 0.08) for x in audio))
        write_wav(OUT / f"{cid}.wav", audio)
        manifest.append({
            "id": cid, "wav": f"tts/{cid}.wav", "source": "tts", "voice": v,
            "accent": "en-IN" if v in IN_VOICES else "en-US", "rate": rate, "variant": variant,
            "duration_s": round(len(audio) / SR, 2), "paragraphs": ref_paras,
            "reference": " ".join(ref_paras), "paragraph_end_s": para_ends,
        })
    for cid, v, s in shorts:
        audio = silence(0.3) + trim(read_pcm(wavs[(v, 1.0, s["text"])])) + silence(0.4)
        write_wav(OUT / f"{cid}.wav", audio)
        manifest.append({
            "id": cid, "wav": f"tts/{cid}.wav", "source": "tts", "voice": v,
            "accent": "en-IN" if v in IN_VOICES else "en-US", "rate": 1.0, "variant": s["category"],
            "duration_s": round(len(audio) / SR, 2), "paragraphs": [s["text"]], "reference": s["text"],
            "must_keep": s["must_keep"],
        })
    with open(OUT / "manifest.jsonl", "w", encoding="utf-8") as f:
        for m in manifest:
            f.write(json.dumps(m, ensure_ascii=False) + "\n")
    total = sum(m["duration_s"] for m in manifest)
    print(f"{len(manifest)} clips, {total / 60:.1f} min -> {OUT}")


if __name__ == "__main__":
    main()
