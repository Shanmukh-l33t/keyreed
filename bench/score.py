#!/usr/bin/env python3
"""
Scores all ASR benchmark JSON outputs under bench/results/ using jiwer and whisper-normalizer.
Outputs structured metrics JSON + Markdown summary tables.
"""
import glob
import json
from collections import defaultdict
from pathlib import Path

import jiwer
import numpy as np
from whisper_normalizer.basic import BasicTextNormalizer
from whisper_normalizer.english import EnglishTextNormalizer
from whisper_normalizer.indic import HindiNormalizer

RESULTS_DIR = Path("/home/computebox/voice-input/bench/results")
en_norm = EnglishTextNormalizer()
basic_norm = BasicTextNormalizer()
hi_norm = HindiNormalizer()

EN_CATS = [
    "clean_english",
    "indian_english",
    "other_accents",
    "conversational",
    "noisy_10db",
    "noisy_5db",
    "long_form",
    "technical_synthetic",
]


def score_pair_list(refs: list[str], hyps: list[str], norm_fn, cer=False) -> float:
    n_refs, n_hyps = [], []
    for r, h in zip(refs, hyps):
        nr = norm_fn(r).strip()
        nh = norm_fn(h).strip()
        if nr:
            n_refs.append(nr)
            n_hyps.append(nh)
    if not n_refs:
        return 0.0
    val = jiwer.cer(n_refs, n_hyps) if cer else jiwer.wer(n_refs, n_hyps)
    return round(val * 100.0, 2)


def main():
    files = sorted(glob.glob(str(RESULTS_DIR / "*.json")))
    summary_rows = []
    for fp in files:
        if fp.endswith("score_summary.json"):
            continue
        data = json.load(open(fp, encoding="utf-8"))
        by_cat = defaultdict(list)
        all_rtfs = []
        for rec in data["records"]:
            by_cat[rec["category"]].append(rec)
            if rec["category"] != "non_speech_hallucination":
                all_rtfs.append(rec["rtf"])

        cat_wer = {}
        en_all_r, en_all_h = [], []
        for c in EN_CATS:
            if c in by_cat:
                rs = [x["ref"] for x in by_cat[c]]
                hs = [x["hyp"] for x in by_cat[c]]
                cat_wer[c] = score_pair_list(rs, hs, en_norm)
                en_all_r.extend(rs)
                en_all_h.extend(hs)

        overall_en_wer = score_pair_list(en_all_r, en_all_h, en_norm)

        # Multilingual scores
        hi_wer, hi_cer, es_wer = None, None, None
        if "multilingual_hi_in" in by_cat:
            rs = [x["ref"] for x in by_cat["multilingual_hi_in"]]
            hs = [x["hyp"] for x in by_cat["multilingual_hi_in"]]
            hi_wer = score_pair_list(rs, hs, hi_norm, cer=False)
            hi_cer = score_pair_list(rs, hs, hi_norm, cer=True)
        if "multilingual_es_419" in by_cat:
            rs = [x["ref"] for x in by_cat["multilingual_es_419"]]
            hs = [x["hyp"] for x in by_cat["multilingual_es_419"]]
            es_wer = score_pair_list(rs, hs, basic_norm, cer=False)

        # Technical entity survival
        tech_hit, tech_tot = 0, 0
        for x in by_cat.get("technical_synthetic", []):
            hl = x["hyp"].lower()
            for ent in x.get("entities") or []:
                tech_tot += 1
                if ent.lower() in hl:
                    tech_hit += 1
        tech_entity_pct = round(100.0 * tech_hit / tech_tot, 1) if tech_tot else None

        # Non-speech hallucination rate
        ns = by_cat.get("non_speech_hallucination", [])
        ns_nonempty = sum(1 for x in ns if en_norm(x["hyp"]).strip() != "")
        halluc_rate = round(100.0 * ns_nonempty / len(ns), 1) if ns else 0.0

        med_rtf = round(float(np.median(all_rtfs)), 4) if all_rtfs else 0.0
        p90_rtf = round(float(np.percentile(all_rtfs, 90)), 4) if all_rtfs else 0.0

        summary_rows.append({
            "model_id": data["model_id"],
            "disk_mb": data["disk_mb"],
            "load_ms": data["load_ms"],
            "loaded_rss_mb": data["loaded_rss_mb"],
            "peak_rss_mb": data["peak_rss_mb"],
            "median_rtf": med_rtf,
            "p90_rtf": p90_rtf,
            "est_10s_latency_ms": round(med_rtf * 10000.0, 0),
            "overall_en_wer": overall_en_wer,
            "cat_wer": cat_wer,
            "hi_wer": hi_wer,
            "hi_cer": hi_cer,
            "es_wer": es_wer,
            "tech_entity_pct": tech_entity_pct,
            "nonspeech_halluc_pct": halluc_rate,
            "clips": data["utterances_evaluated"],
        })

    summary_rows.sort(key=lambda r: r["overall_en_wer"])
    out_sum = RESULTS_DIR / "score_summary.json"
    with open(out_sum, "w", encoding="utf-8") as f:
        json.dump(summary_rows, f, indent=2)

    print("\n=== MEASURED ASR BENCHMARK SUMMARY (4 CPU threads, EPYC 9655) ===")
    hdr = f"{'Model':28s} | {'AllEN':>5s} | {'Clean':>5s} | {'IndEN':>5s} | {'Conv':>5s} | {'10dB':>5s} | {'5dB':>5s} | {'Tech':>5s} | {'Hall%':>5s} | {'mRTF':>6s} | {'10sMs':>5s} | {'RSS_MB':>6s} | {'DiskMB':>6s}"
    print(hdr)
    print("-" * len(hdr))
    for r in summary_rows:
        cw = r["cat_wer"]
        print(
            f"{r['model_id']:28s} | {r['overall_en_wer']:5.2f} | "
            f"{cw.get('clean_english', -1):5.2f} | {cw.get('indian_english', -1):5.2f} | "
            f"{cw.get('conversational', -1):5.2f} | {cw.get('noisy_10db', -1):5.2f} | "
            f"{cw.get('noisy_5db', -1):5.2f} | {cw.get('technical_synthetic', -1):5.2f} | "
            f"{r['nonspeech_halluc_pct']:5.1f} | {r['median_rtf']:6.4f} | "
            f"{r['est_10s_latency_ms']:5.0f} | {r['peak_rss_mb']:6.1f} | {r['disk_mb']:6.1f}"
        )


if __name__ == "__main__":
    main()
