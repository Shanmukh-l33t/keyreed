#!/usr/bin/env python3
"""
Deterministic, low-memory multi-category ASR benchmark corpus builder.
Builds 16 kHz mono 16-bit PCM WAVs + manifest.jsonl under /home/computebox/voice-input/bench/corpus/.
"""
import asyncio
import io
import json
import os
import random
import subprocess
import tarfile
import urllib.request
from pathlib import Path

import edge_tts
import numpy as np
import pyarrow.parquet as pq
import soundfile as sf
import soxr
from huggingface_hub import hf_hub_download

BENCH_DIR = Path("/home/computebox/voice-input/bench")
CORPUS_DIR = BENCH_DIR / "corpus"
WAV_DIR = CORPUS_DIR / "wavs"
CACHE_TMP = BENCH_DIR / "cache" / "tmp_dl"
FFMPEG = "/root/.hermes/tools/ffmpeg-9.0.1-linux-x64/bin/ffmpeg"
SEED = 42


def to_16k_mono(audio: np.ndarray, sr: int) -> np.ndarray:
    if audio.ndim > 1:
        audio = np.mean(audio, axis=1)
    audio = audio.astype(np.float32)
    if sr != 16000:
        audio = soxr.resample(audio, sr, 16000)
    peak = float(np.max(np.abs(audio))) if len(audio) else 0.0
    if peak > 0.99:
        audio = audio / (peak * 1.01)
    return audio.astype(np.float32)


def decode_audio_bytes(raw_bytes: bytes) -> np.ndarray:
    data, sr = sf.read(io.BytesIO(raw_bytes), dtype="float32")
    return to_16k_mono(data, sr)


def save_wav(path: Path, samples_16k: np.ndarray) -> float:
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), samples_16k, 16000, subtype="PCM_16")
    return round(len(samples_16k) / 16000.0, 3)


def mix_at_snr(clean: np.ndarray, noise: np.ndarray, snr_db: float) -> np.ndarray:
    if len(noise) < len(clean):
        reps = (len(clean) // len(noise)) + 2
        noise = np.tile(noise, reps)
    start = 0 if len(noise) == len(clean) else random.randint(0, len(noise) - len(clean))
    n_seg = noise[start : start + len(clean)].astype(np.float32)
    rms_s = float(np.sqrt(np.mean(clean**2)) + 1e-8)
    rms_n = float(np.sqrt(np.mean(n_seg**2)) + 1e-8)
    target_rms_n = rms_s / (10.0 ** (snr_db / 20.0))
    mixed = clean + n_seg * (target_rms_n / rms_n)
    peak = float(np.max(np.abs(mixed)))
    if peak > 0.99:
        mixed = mixed / (peak * 1.02)
    return mixed.astype(np.float32)


TECHNICAL_ITEMS = [
    ("Deploy the Kubernetes ingress controller in namespace kube system on port 8443.", "Deploy the Kubernetes ingress controller in namespace kube-system on port 8443.", ["Kubernetes", "8443"]),
    ("The PostgreSQL connection pool timed out after 30 seconds on staging.", "The PostgreSQL connection pool timed out after 30 seconds on staging.", ["PostgreSQL", "30"]),
    ("Update the OAuth2 callback handler to validate the JWT audience claim.", "Update the OAuth2 callback handler to validate the JWT audience claim.", ["OAuth2", "JWT"]),
    ("Check nginx error logs under slash var slash log slash nginx.", "Check nginx error logs under /var/log/nginx.", ["nginx"]),
    ("Please email the security report to priya dot sharma at example dot com.", "Please email the security report to priya.sharma@example.com.", ["priya", "example"]),
    ("Send the invoice copy to ops dash alerts at healthyheartcrm dot com by Friday.", "Send the invoice copy to ops-alerts@healthyheartcrm.com by Friday.", ["healthyheartcrm", "Friday"]),
    ("Our Q3 infrastructure spend dropped from 14,500 dollars to 9,250 dollars.", "Our Q3 infrastructure spend dropped from $14,500 to $9,250.", ["14", "9"]),
    ("Schedule the production cutover for November 24, 2026 at 11:30 PM UTC.", "Schedule the production cutover for November 24, 2026 at 11:30 PM UTC.", ["November", "24", "2026", "11", "30"]),
    ("Run kubectl get pods dash n production dash o wide.", "kubectl get pods -n production -o wide", ["kubectl", "pods", "production"]),
    ("Execute git checkout dash b hotfix slash auth token refresh.", "git checkout -b hotfix/auth-token-refresh", ["git", "checkout", "hotfix"]),
    ("Open docs dot hermes dash agent dot io to verify the webhook schema.", "Open docs.hermes-agent.io to verify the webhook schema.", ["webhook", "schema"]),
    ("The Redis cluster replication lag spiked to 420 milliseconds during backup.", "The Redis cluster replication lag spiked to 420 milliseconds during backup.", ["Redis", "420"]),
    ("Verify SHA 256 checksums before loading ONNX Runtime version 1.32.", "Verify SHA-256 checksums before loading ONNX Runtime version 1.32.", ["256", "ONNX", "1"]),
    ("Set HTTP status code 429 when the client exceeds 120 requests per minute.", "Set HTTP status code 429 when the client exceeds 120 requests per minute.", ["HTTP", "429", "120"]),
    ("In Rust, use parking lot Mutex to avoid poison errors across worker threads.", "In Rust, use parking_lot::Mutex to avoid poison errors across worker threads.", ["Rust", "Mutex"]),
    ("Windows 11 build 26200 enforces User Interface Privilege Isolation on SendInput.", "Windows 11 build 26200 enforces User Interface Privilege Isolation on SendInput.", ["Windows", "11", "26200"]),
    ("Configure WebSocket keepalive pings every 25 seconds behind Cloudflare.", "Configure WebSocket keepalive pings every 25 seconds behind Cloudflare.", ["WebSocket", "25", "Cloudflare"]),
    ("The gRPC service listens on localhost port 50051 with mutual TLS enabled.", "The gRPC service listens on localhost port 50051 with mutual TLS enabled.", ["50051", "TLS"]),
    ("Run cargo test double dash workspace double dash release before tagging.", "cargo test --workspace --release", ["cargo", "test", "workspace"]),
    ("Total latency from key release to clipboard ready measured 385 milliseconds.", "Total latency from key release to clipboard ready measured 385 milliseconds.", ["385", "clipboard"]),
    ("Migrate the SQLite write ahead log mode to synchronous normal.", "Migrate the SQLite write-ahead log mode to synchronous NORMAL.", ["SQLite"]),
    ("Check DNS CAA records for api dot staff4u dot ai using dig.", "Check DNS CAA records for api.staff4u.ai using dig.", ["DNS"]),
    ("Monthly recurring cost per active user is 0 rupees in offline mode.", "Monthly recurring cost per active user is ₹0 in offline mode.", ["0", "offline"]),
    ("Add Terraform state locking with DynamoDB table prod locks.", "Add Terraform state locking with DynamoDB table prod-locks.", ["Terraform", "DynamoDB"]),
    ("The Docker image compressed size decreased from 680 megabytes to 42 megabytes.", "The Docker image compressed size decreased from 680 MB to 42 MB.", ["Docker", "680", "42"]),
    ("Inspect process memory counters via GetProcessMemoryInfo in Win32.", "Inspect process memory counters via GetProcessMemoryInfo in Win32.", ["Win32"]),
    ("Enable Content Security Policy headers on login dot homecaretoolbox dot com.", "Enable Content Security Policy headers on login.homecaretoolbox.com.", ["Policy"]),
    ("Rotate the Ed25519 SSH deploy key every 90 days.", "Rotate the Ed25519 SSH deploy key every 90 days.", ["SSH", "90"]),
    ("Quantizing Parakeet TDT to INT8 cuts RAM consumption below 700 megabytes.", "Quantizing Parakeet TDT to INT8 cuts RAM consumption below 700 MB.", ["700"]),
    ("Set thread priority above normal for the low level keyboard hook.", "Set thread priority above normal for the low-level keyboard hook.", ["keyboard", "hook"]),
]

TTS_VOICES = [
    "en-IN-NeerjaNeural",
    "en-US-AndrewNeural",
    "en-GB-SoniaNeural",
    "en-US-AriaNeural",
]


async def synth_technical_wavs(manifest: list):
    print("Synthesizing technical_synthetic clips via multi-voice neural TTS...")
    CACHE_TMP.mkdir(parents=True, exist_ok=True)
    (WAV_DIR / "technical_synthetic").mkdir(parents=True, exist_ok=True)
    for i, (spoken, written, entities) in enumerate(TECHNICAL_ITEMS):
        voice = TTS_VOICES[i % len(TTS_VOICES)]
        mp3_tmp = CACHE_TMP / f"tts_{i:03d}.mp3"
        wav_out = WAV_DIR / "technical_synthetic" / f"tech_{i:03d}.wav"
        if not wav_out.exists() or wav_out.stat().st_size < 1000:
            for attempt in range(6):
                try:
                    use_v = voice if attempt < 2 else "en-US-AriaNeural"
                    mp3_tmp.unlink(missing_ok=True)
                    comm = edge_tts.Communicate(spoken, use_v)
                    await comm.save(str(mp3_tmp))
                    if mp3_tmp.exists() and mp3_tmp.stat().st_size > 800:
                        break
                except Exception:
                    pass
                await asyncio.sleep(1.2 * (attempt + 1))
            subprocess.run(
                [FFMPEG, "-y", "-loglevel", "error", "-i", str(mp3_tmp), "-ar", "16000", "-ac", "1", "-sample_fmt", "s16", str(wav_out)],
                check=True,
            )
            mp3_tmp.unlink(missing_ok=True)
            await asyncio.sleep(0.15)
        info = sf.info(str(wav_out))
        manifest.append({
            "id": f"tech_{i:03d}",
            "path": str(wav_out),
            "ref": spoken,
            "ref_written": written,
            "entities": entities,
            "category": "technical_synthetic",
            "synthetic": True,
            "voice": voice,
            "source": f"SYNTHETIC:edge-tts:{voice}",
            "duration_s": round(info.duration, 3),
            "license": "Synthetic benchmark test fixture",
        })


def main():
    random.seed(SEED)
    np.random.seed(SEED)
    WAV_DIR.mkdir(parents=True, exist_ok=True)
    CACHE_TMP.mkdir(parents=True, exist_ok=True)
    manifest = []

    # 1. LibriSpeech test-clean (35 short clips + 6 long-form 45-70s concatenated clips)
    print("Fetching LibriSpeech test-clean shard...")
    ls_pq = Path(hf_hub_download("openslr/librispeech_asr", "all/test.clean/0000.parquet", repo_type="dataset", local_dir=str(CACHE_TMP)))
    pf = pq.ParquetFile(ls_pq)
    clean_pool = []
    by_chapter = {}
    for batch in pf.iter_batches(batch_size=32, columns=["audio", "text", "speaker_id", "chapter_id", "id"]):
        cols = batch.to_pydict()
        for aud, txt, spk, chp, uid in zip(cols["audio"], cols["text"], cols["speaker_id"], cols["chapter_id"], cols["id"]):
            raw_b = aud["bytes"]
            arr = decode_audio_bytes(raw_b)
            dur = len(arr) / 16000.0
            if 2.5 <= dur <= 14.0 and len(clean_pool) < 60:
                clean_pool.append((uid, arr, txt, dur))
            key = f"{spk}_{chp}"
            if len(by_chapter.get(key, [])) < 12:
                by_chapter.setdefault(key, []).append((uid, arr, txt, dur))
        if len(clean_pool) >= 45 and len(by_chapter) >= 12:
            break
    ls_pq.unlink(missing_ok=True)

    clean_saved = []
    for idx, (uid, arr, txt, _) in enumerate(clean_pool[:32]):
        out_p = WAV_DIR / "clean_english" / f"clean_{idx:03d}.wav"
        dur = save_wav(out_p, arr)
        clean_saved.append((arr, txt))
        manifest.append({
            "id": f"clean_{idx:03d}",
            "path": str(out_p),
            "ref": txt,
            "category": "clean_english",
            "source": f"openslr/librispeech_asr:test-clean:{uid}",
            "duration_s": dur,
            "license": "CC-BY-4.0",
        })

    # Long-form 45-75s clips from consecutive chapter utterances
    pause = np.zeros(int(16000 * 0.35), dtype=np.float32)
    lf_count = 0
    for key, segs in by_chapter.items():
        total_d = sum(x[3] for x in segs)
        if total_d >= 46.0 and lf_count < 6:
            acc_audio = []
            acc_txt = []
            acc_d = 0.0
            for _, arr, txt, d in segs:
                acc_audio.append(arr)
                acc_audio.append(pause)
                acc_txt.append(txt)
                acc_d += d + 0.35
                if acc_d >= 52.0:
                    break
            full_arr = np.concatenate(acc_audio)
            out_p = WAV_DIR / "long_form" / f"long_{lf_count:03d}.wav"
            dur = save_wav(out_p, full_arr)
            manifest.append({
                "id": f"long_{lf_count:03d}",
                "path": str(out_p),
                "ref": " ".join(acc_txt),
                "category": "long_form",
                "source": f"openslr/librispeech_asr:concat:{key}",
                "duration_s": dur,
                "license": "CC-BY-4.0",
            })
            lf_count += 1

    # 2. EdAcc: real spontaneous Indian-accented English + non-US accents + conversational speech
    print("Extracting EdAcc Indian English, non-US accents, and spontaneous conversational turns...")
    with open(BENCH_DIR / "cache" / "meta" / "edacc.json") as f:
        edacc_meta = json.load(f)

    # Shards test-00001 and test-00005 contain rich Indian English + diverse accents
    target_shards = [
        "data/test-00001-of-00010-856b016d9d438ff3.parquet",
        "data/test-00005-of-00010-9c97c4c4c8d01f82.parquet",
    ]
    indian_saved = []
    ind_cnt = 0
    oth_cnt = 0
    conv_cnt = 0

    for shard in target_shards:
        want_indian = [m for m in edacc_meta if m["file"] == shard and "indian" in m["accent"].lower() and len(m["text"].split()) >= 6]
        want_other = [m for m in edacc_meta if m["file"] == shard and "indian" not in m["accent"].lower() and "us " not in m["accent"].lower() and len(m["text"].split()) >= 6]
        random.shuffle(want_indian)
        random.shuffle(want_other)
        pick_indices = {}
        for m in want_indian[:22]:
            pick_indices[(m["rg"], m["idx"])] = ("indian_english", m)
        for m in want_other[:16]:
            pick_indices[(m["rg"], m["idx"])] = ("other_accents", m)
        for m in want_other[16:34]:
            pick_indices[(m["rg"], m["idx"])] = ("conversational", m)

        loc_pq = Path(hf_hub_download("edinburghcstr/edacc", shard, repo_type="dataset", local_dir=str(CACHE_TMP)))
        pf_ed = pq.ParquetFile(loc_pq)
        for rg_i in range(pf_ed.num_row_groups):
            needed_in_rg = {idx: v for (r, idx), v in pick_indices.items() if r == rg_i}
            if not needed_in_rg:
                continue
            tbl = pf_ed.read_row_group(rg_i, columns=["audio", "text"])
            aud_col = tbl["audio"].to_pylist()
            txt_col = tbl["text"].to_pylist()
            for row_idx, (cat_name, meta_row) in needed_in_rg.items():
                if row_idx >= len(aud_col):
                    continue
                arr = decode_audio_bytes(aud_col[row_idx]["bytes"])
                dur_s = len(arr) / 16000.0
                if not (2.0 <= dur_s <= 18.0):
                    continue
                ref_txt = txt_col[row_idx].strip()
                if cat_name == "indian_english" and ind_cnt < 32:
                    out_p = WAV_DIR / "indian_english" / f"ind_{ind_cnt:03d}.wav"
                    d = save_wav(out_p, arr)
                    indian_saved.append((arr, ref_txt))
                    manifest.append({
                        "id": f"ind_{ind_cnt:03d}",
                        "path": str(out_p),
                        "ref": ref_txt,
                        "category": "indian_english",
                        "source": f"edinburghcstr/edacc:{meta_row['speaker']}:{meta_row['l1']}",
                        "duration_s": d,
                        "license": "CC-BY-SA-4.0",
                    })
                    ind_cnt += 1
                elif cat_name == "other_accents" and oth_cnt < 24:
                    out_p = WAV_DIR / "other_accents" / f"acc_{oth_cnt:03d}.wav"
                    d = save_wav(out_p, arr)
                    manifest.append({
                        "id": f"acc_{oth_cnt:03d}",
                        "path": str(out_p),
                        "ref": ref_txt,
                        "category": "other_accents",
                        "source": f"edinburghcstr/edacc:{meta_row['accent']}",
                        "duration_s": d,
                        "license": "CC-BY-SA-4.0",
                    })
                    oth_cnt += 1
                elif cat_name == "conversational" and conv_cnt < 26:
                    out_p = WAV_DIR / "conversational" / f"conv_{conv_cnt:03d}.wav"
                    d = save_wav(out_p, arr)
                    manifest.append({
                        "id": f"conv_{conv_cnt:03d}",
                        "path": str(out_p),
                        "ref": ref_txt,
                        "category": "conversational",
                        "source": f"edinburghcstr/edacc:{meta_row['speaker']}",
                        "duration_s": d,
                        "license": "CC-BY-SA-4.0",
                    })
                    conv_cnt += 1
        loc_pq.unlink(missing_ok=True)

    # 3. Real environmental noise from ESC-50 for noisy_10db, noisy_5db, and non_speech_hallucination
    print("Downloading real ESC-50 office/typing/ambient noise clips...")
    esc_files = [
        ("keyboard_typing", "1-137-A-32.wav"),
        ("keyboard_typing", "1-53501-A-32.wav"),
        ("mouse_click", "1-118206-A-31.wav"),
        ("wind", "1-137296-A-16.wav"),
        ("rain", "1-17367-A-10.wav"),
        ("vacuum", "1-100210-A-36.wav"),
        ("washing_machine", "1-21896-A-35.wav"),
        ("footsteps", "1-155858-A-25.wav"),
    ]
    noise_clips = []
    for label, fname in esc_files:
        url = f"https://github.com/karolpiczak/ESC-50/raw/master/audio/{fname}"
        raw_n = urllib.request.urlopen(url, timeout=20).read()
        n_arr = decode_audio_bytes(raw_n)
        noise_clips.append((label, fname, n_arr))

    base_for_noise = clean_saved[:12] + indian_saved[:12]
    for snr_db, cat_label in [(10.0, "noisy_10db"), (5.0, "noisy_5db")]:
        for i, (s_arr, s_ref) in enumerate(base_for_noise):
            n_lbl, n_fn, n_arr = noise_clips[i % len(noise_clips)]
            mixed = mix_at_snr(s_arr, n_arr, snr_db)
            out_p = WAV_DIR / cat_label / f"{cat_label}_{i:03d}.wav"
            d = save_wav(out_p, mixed)
            manifest.append({
                "id": f"{cat_label}_{i:03d}",
                "path": str(out_p),
                "ref": s_ref,
                "category": cat_label,
                "source": f"mixed:{snr_db}dB:ESC-50:{n_fn}",
                "duration_s": d,
                "license": "CC-BY-4.0 / ESC-50",
            })

    # Non-speech hallucination clips (12 clips: digital silence, room floor, typing, clicks, hum)
    non_speech_specs = [
        ("digital_silence_3s", np.zeros(16000 * 3, dtype=np.float32)),
        ("digital_silence_8s", np.zeros(16000 * 8, dtype=np.float32)),
        ("low_room_floor_4s", (np.random.randn(16000 * 4) * 0.0004).astype(np.float32)),
        ("low_room_floor_7s", (np.random.randn(16000 * 7) * 0.0006).astype(np.float32)),
    ]
    for idx_n, (n_lbl, n_fn, n_arr) in enumerate(noise_clips):
        non_speech_specs.append((f"esc50_{n_lbl}_{idx_n}", n_arr * 0.35))

    for i, (lbl, arr) in enumerate(non_speech_specs[:12]):
        out_p = WAV_DIR / "non_speech_hallucination" / f"nonspeech_{i:03d}.wav"
        d = save_wav(out_p, arr)
        manifest.append({
            "id": f"nonspeech_{i:03d}",
            "path": str(out_p),
            "ref": "",
            "category": "non_speech_hallucination",
            "source": f"nonspeech:{lbl}",
            "duration_s": d,
            "license": "ESC-50 / Generated Silence",
        })

    # 4. Multilingual: real Hindi (hi_in) + Spanish (es_419) from google/fleurs
    for lang_code in ["hi_in", "es_419"]:
        print(f"Extracting FLEURS {lang_code} test clips...")
        tsv_p = Path(hf_hub_download("google/fleurs", f"data/{lang_code}/test.tsv", repo_type="dataset", local_dir=str(CACHE_TMP)))
        tar_p = Path(hf_hub_download("google/fleurs", f"data/{lang_code}/audio/test.tar.gz", repo_type="dataset", local_dir=str(CACHE_TMP)))
        tsv_map = {}
        with open(tsv_p, encoding="utf-8") as f:
            for line in f:
                parts = line.rstrip("\n").split("\t")
                if len(parts) >= 3:
                    tsv_map[parts[1]] = parts[2]
        saved_l = 0
        with tarfile.open(tar_p, "r:gz") as tf:
            for member in tf:
                if not member.isfile() or not member.name.endswith(".wav"):
                    continue
                leaf = os.path.basename(member.name)
                if leaf not in tsv_map:
                    continue
                raw_b = tf.extractfile(member).read()
                arr = decode_audio_bytes(raw_b)
                dur_s = len(arr) / 16000.0
                if 2.5 <= dur_s <= 14.0:
                    cat_name = f"multilingual_{lang_code}"
                    out_p = WAV_DIR / cat_name / f"{lang_code}_{saved_l:03d}.wav"
                    d = save_wav(out_p, arr)
                    manifest.append({
                        "id": f"{lang_code}_{saved_l:03d}",
                        "path": str(out_p),
                        "ref": tsv_map[leaf],
                        "lang": lang_code.split("_")[0],
                        "category": cat_name,
                        "source": f"google/fleurs:{lang_code}:{leaf}",
                        "duration_s": d,
                        "license": "CC-BY-4.0",
                    })
                    saved_l += 1
                    if saved_l >= 18:
                        break
        tar_p.unlink(missing_ok=True)
        tsv_p.unlink(missing_ok=True)

    # 5. Technical synthetic (30 clips across 4 voices)
    asyncio.run(synth_technical_wavs(manifest))

    manifest_path = CORPUS_DIR / "manifest.jsonl"
    with open(manifest_path, "w", encoding="utf-8") as f:
        for row in manifest:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    from collections import Counter
    counts = Counter(r["category"] for r in manifest)
    total_dur = sum(r["duration_s"] for r in manifest)
    print(f"\nCorpus built: {len(manifest)} clips ({total_dur/60:.1f} min total) -> {manifest_path}")
    for k, v in sorted(counts.items()):
        print(f"  {k:28s}: {v:3d} clips")


if __name__ == "__main__":
    main()
