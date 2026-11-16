#!/usr/bin/env python3
"""Scores KeyReed benchmark runs (bench/results/<tag>/*.json) against the corpus manifests.
Standard library only.

  python3 bench/score_bench.py --tag v0.5.0 [--tag v0.4.0-baseline ...] [--field final_text|raw_transcript]

Per clip: normalised WER, deletions (omitted words) and insertions (possible hallucinations),
CER for short clips, repeated-segment count, must-keep tokens, sentence-boundary and
paragraph-boundary precision/recall against the reference, latency and real-time factor.
Writes bench/results/<tag>/score.json and prints a Markdown summary.
"""
import argparse
import json
import re
import statistics
from collections import defaultdict
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ONES = "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen".split()
TENS = "x x twenty thirty forty fifty sixty seventy eighty ninety".split()


def num_words(n):
    if n < 20:
        return [ONES[n]]
    if n < 100:
        return [TENS[n // 10]] + ([ONES[n % 10]] if n % 10 else [])
    if n < 1000:
        return [ONES[n // 100], "hundred"] + (num_words(n % 100) if n % 100 else [])
    for size, name in ((10 ** 9, "billion"), (10 ** 6, "million"), (1000, "thousand")):
        if n >= size:
            return num_words(n // size) + [name] + (num_words(n % size) if n % size else [])
    return []


def norm_token(tok):
    """One whitespace token -> list of normalised words (numbers spelled out, punctuation dropped)."""
    t = tok.lower().replace("%", " percent ").replace("$", " dollars ").replace("\u20b9", " rupees ")
    t = re.sub(r"(?<=\d),(?=\d{3})", "", t)
    t = re.sub(r"[\u2018\u2019]", "'", t)
    out = []
    for part in re.split(r"[^a-z0-9'.:]+", t):
        part = part.strip(".:'")
        if not part:
            continue
        if re.fullmatch(r"\d+(?:[.:]\d+)*", part):
            pieces = re.split(r"([.:])", part)
            for p in pieces:
                if p == ".":
                    out.append("point")
                elif p.isdigit():
                    out += num_words(int(p)) if len(p) < 10 else list(p)
        else:
            out += [w for w in re.split(r"[.:]+", part.replace("'", "")) if w]
    # "and" inside spoken numbers ("two hundred and fifty") is optional: drop it everywhere on both sides
    return [w for w in out if w != "and"]


def tokens_with_flags(paragraphs):
    """-> (words, sentence_end_flags, paragraph_end_flags) for a list of paragraph strings."""
    words, sent, para = [], [], []
    for p in paragraphs:
        for tok in p.split():
            ws = norm_token(tok)
            if not ws:
                continue
            words += ws
            sent += [False] * len(ws)
            para += [False] * len(ws)
            if re.search(r"[.?!][\"')\]]*$", tok):
                sent[-1] = True
        if words:
            para[-1] = True
            sent[-1] = True
    return words, sent, para


def align(ref, hyp):
    """Word Levenshtein with backtrace -> (subs, dels, ins, [(ref_idx|None, hyp_idx|None)])"""
    n, m = len(ref), len(hyp)
    prev = list(range(m + 1))
    back = [bytearray(m + 1) for _ in range(n + 1)]  # 0 diag, 1 up (deletion), 2 left (insertion)
    for j in range(1, m + 1):
        back[0][j] = 2
    for i in range(1, n + 1):
        cur = [i] + [0] * m
        back[i][0] = 1
        ri, bi = ref[i - 1], back[i]
        for j in range(1, m + 1):
            d = prev[j - 1] + (ri != hyp[j - 1])
            u = prev[j] + 1
            l = cur[j - 1] + 1
            if d <= u and d <= l:
                cur[j] = d
            elif u <= l:
                cur[j] = u; bi[j] = 1
            else:
                cur[j] = l; bi[j] = 2
        prev = cur
    i, j, pairs, s, d, ins = n, m, [], 0, 0, 0
    while i or j:
        b = back[i][j]
        if b == 0:
            s += ref[i - 1] != hyp[j - 1]
            pairs.append((i - 1, j - 1)); i -= 1; j -= 1
        elif b == 1:
            d += 1; pairs.append((i - 1, None)); i -= 1
        else:
            ins += 1; pairs.append((None, j - 1)); j -= 1
    return s, d, ins, pairs[::-1]


def char_distance(a, b):
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def prf(ref_flags, hyp_flags, pairs, skip_last=True):
    """Boundary precision/recall: a reference boundary is found if the aligned hypothesis word has one."""
    r2h = {r: h for r, h in pairs if r is not None and h is not None}
    h2r = {h: r for r, h in r2h.items()}
    ref_b = [i for i, f in enumerate(ref_flags) if f and not (skip_last and i == len(ref_flags) - 1)]
    hyp_b = [i for i, f in enumerate(hyp_flags) if f and not (skip_last and i == len(hyp_flags) - 1)]
    hit_r = sum(1 for i in ref_b if i in r2h and hyp_flags[r2h[i]])
    hit_h = sum(1 for i in hyp_b if i in h2r and ref_flags[h2r[i]])
    rec = hit_r / len(ref_b) if ref_b else None
    prec = hit_h / len(hyp_b) if hyp_b else None
    return prec, rec, len(ref_b), len(hyp_b)


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))] if xs else None


def score_clip(clip, text):
    ref_w, ref_s, ref_p = tokens_with_flags(clip["paragraphs"])
    hyp_paras = [p for p in re.split(r"\n\s*\n", text) if p.strip()]
    hyp_w, hyp_s, hyp_p = tokens_with_flags(hyp_paras)
    s, d, i, pairs = align(ref_w, hyp_w)
    out = {"ref_words": len(ref_w), "hyp_words": len(hyp_w), "sub": s, "del": d, "ins": i,
           "wer": round(100 * (s + d + i) / max(1, len(ref_w)), 2)}
    a, b = " ".join(ref_w), " ".join(hyp_w)
    if len(a) <= 2500:
        out["cer"] = round(100 * char_distance(a, b) / max(1, len(a)), 2)
    out["repeated_segments"] = sum(1 for k in range(len(hyp_w) - 9) if hyp_w[k:k + 5] == hyp_w[k + 5:k + 10])
    if clip.get("must_keep"):
        low = text.lower()
        out["must_keep_hit"] = sum(1 for k in clip["must_keep"] if k.lower() in low)
        out["must_keep_total"] = len(clip["must_keep"])
    sp, sr, _, _ = prf(ref_s, hyp_s, pairs)
    out["sentence_precision"], out["sentence_recall"] = sp, sr
    pp, pr, n_ref_b, n_hyp_b = prf(ref_p, hyp_p, pairs)
    out.update({"paragraph_precision": pp, "paragraph_recall": pr, "ref_paragraphs": n_ref_b + 1,
                "hyp_paragraphs": len(hyp_paras), "longest_paragraph_words": max((len(p.split()) for p in hyp_paras), default=0)})
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", action="append", required=True)
    ap.add_argument("--field", default="final_text")
    a = ap.parse_args()
    clips = {}
    for mf in sorted((BENCH / "corpus").glob("*/manifest.jsonl")):
        for l in mf.read_text(encoding="utf-8").splitlines():
            if l.strip():
                c = json.loads(l); clips[c["id"]] = c

    for tag in a.tag:
        tagdir = BENCH / "results" / tag
        summary = {}
        for f in sorted(tagdir.glob("*.json")):
            if f.name == "score.json":
                continue
            data = json.loads(f.read_text(encoding="utf-8"))
            rows = []
            for rec in data["records"]:
                if not rec["runs"] or rec["id"] not in clips:
                    continue
                clip, runs = clips[rec["id"]], rec["runs"]
                res = runs[-1]["result"]
                row = {"id": rec["id"], "duration_s": rec["duration_s"], "source": clip.get("source"),
                       "accent": clip.get("accent"), "variant": clip.get("variant")}
                row.update(score_clip(clip, res[a.field]))
                m0 = runs[0]["result"]["metrics"]
                warm = [r["result"]["metrics"] for r in runs[1:]] or [m0]
                row["cold_release_to_ready_ms"] = round(m0["total_release_to_ready_ms"])
                row["warm_release_to_ready_ms"] = round(statistics.median(m["total_release_to_ready_ms"] for m in warm))
                row["warm_asr_ms"] = round(statistics.median(m["asr_ms"] for m in warm))
                row["warm_decode_rtf"] = round(row["warm_asr_ms"] / 1000 / rec["duration_s"], 3)
                for k in ("post_release_asr_ms", "model_load_ms", "rewrite_ms", "output_ms"):
                    if k in warm[-1]:
                        row["warm_" + k] = round(statistics.median(m[k] for m in warm), 1)
                row["peak_working_set_mb"] = rec["peak_working_set_mb"]
                row["process_cpu_s"] = rec["process_cpu_s"]
                rows.append(row)
            if not rows:
                continue

            def agg(sel):
                rs = [r for r in rows if sel(r)]
                if not rs:
                    return None
                refw = sum(r["ref_words"] for r in rs)
                mk_t = sum(r.get("must_keep_total", 0) for r in rs)

                def mean(key):
                    v = [r[key] for r in rs if r.get(key) is not None]
                    return round(sum(v) / len(v), 3) if v else None
                lat = [r["warm_release_to_ready_ms"] for r in rs]
                return {"clips": len(rs), "minutes": round(sum(r["duration_s"] for r in rs) / 60, 1),
                        "wer": round(100 * sum(r["sub"] + r["del"] + r["ins"] for r in rs) / max(1, refw), 2),
                        "omission_pct": round(100 * sum(r["del"] for r in rs) / max(1, refw), 2),
                        "insertion_pct": round(100 * sum(r["ins"] for r in rs) / max(1, refw), 2),
                        "repeated_segments": sum(r["repeated_segments"] for r in rs),
                        "must_keep_pct": round(100 * sum(r.get("must_keep_hit", 0) for r in rs) / mk_t, 1) if mk_t else None,
                        "sentence_precision": mean("sentence_precision"), "sentence_recall": mean("sentence_recall"),
                        "paragraph_precision": mean("paragraph_precision"), "paragraph_recall": mean("paragraph_recall"),
                        "warm_ms_p50": pct(lat, 50), "warm_ms_p90": pct(lat, 90), "warm_ms_p95": pct(lat, 95),
                        "cold_ms_p50": pct([r["cold_release_to_ready_ms"] for r in rs], 50),
                        "decode_rtf_p50": pct([r["warm_decode_rtf"] for r in rs], 50)}

            groups = {"all": agg(lambda r: True)}
            for lo, hi, name in ((0, 10, "under_10s"), (10, 35, "10_35s"), (35, 70, "35_70s"), (70, 150, "70_150s"), (150, 1e9, "over_150s")):
                groups[name] = agg(lambda r: lo <= r["duration_s"] < hi)
            for key in ("source", "accent", "variant"):
                for val in sorted({r[key] for r in rows if r.get(key)}):
                    groups[f"{key}={val}"] = agg(lambda r: r.get(key) == val)
            summary[f.stem] = {"model": data["model"], "live": data.get("live"), "cpu": data.get("cpu"), "date": data.get("date"),
                               "field": a.field, "groups": {k: v for k, v in groups.items() if v}, "clips": rows}

            print(f"\n### {tag} / {f.stem} ({a.field}; {data.get('cpu')}; {data.get('date')})\n")
            print("| group | clips | min | WER % | omitted % | inserted % | repeats | must-keep % | sent P/R | para P/R | warm p50 / p95 ms | cold p50 ms | decode RTF |")
            print("|---|---:|---:|---:|---:|---:|---:|---:|---|---|---:|---:|---:|")
            for k, g in summary[f.stem]["groups"].items():
                fmt = lambda x: "-" if x is None else x
                print(f"| {k} | {g['clips']} | {g['minutes']} | {g['wer']} | {g['omission_pct']} | {g['insertion_pct']} | {g['repeated_segments']} | {fmt(g['must_keep_pct'])} | "
                      f"{fmt(g['sentence_precision'])} / {fmt(g['sentence_recall'])} | {fmt(g['paragraph_precision'])} / {fmt(g['paragraph_recall'])} | "
                      f"{g['warm_ms_p50']} / {g['warm_ms_p95']} | {g['cold_ms_p50']} | {g['decode_rtf_p50']} |")
        (tagdir / "score.json").write_text(json.dumps(summary, indent=1, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    main()
