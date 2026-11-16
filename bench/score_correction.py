#!/usr/bin/env python3
"""Scores `keyreed --eval-correction-jsonl` outputs: content WER vs reference (case/punctuation-insensitive),
exact-match rate, invariants, and latency split by SLM-invoked vs bypassed cases.
Usage: score_correction.py label=path.json [label=path.json ...]
"""
import json, re, sys
from collections import defaultdict


def toks(s):
    return re.findall(r"[a-z0-9_.@:/+'-]+", s.lower().replace("\n", " "))


def edit(a, b):
    d = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        p, d[0] = d[0], i
        for j, y in enumerate(b, 1):
            p, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, p + (x != y))
    return d[-1]


def q(a, p):
    a = sorted(a)
    return a[min(len(a) - 1, int(p * len(a)))] if a else float("nan")


for arg in sys.argv[1:]:
    label, path = arg.split("=", 1)
    recs = json.load(open(path, encoding="utf-8"))["records"]
    cat = defaultdict(lambda: [0, 0])
    tot_e = tot_n = exact = inv = 0
    slm, byp = [], []
    for r in recs:
        ref, out = toks(r["reference"]), toks(r["output"])
        e = edit(out, ref)
        c = cat[r["category"]]
        c[0] += e
        c[1] += len(ref)
        tot_e += e
        tot_n += len(ref)
        exact += r["output"].strip() == r["reference"].strip()
        inv += r["must_keep_ok"] and r["must_not_contain_ok"]
        (slm if r["tier_used"] == "tier1_local_slm" else byp).append(r["elapsed_ms"])
    print(f"== {label}: n={len(recs)} contentWER={100*tot_e/tot_n:.1f}% exact={exact} invariants={inv}/{len(recs)}")
    print("   per-category WER%: " + ", ".join(f"{k}={100*v[0]/v[1]:.1f}" for k, v in sorted(cat.items())))
    print(f"   SLM-invoked n={len(slm)} p50/p90/p95={q(slm,.5):.0f}/{q(slm,.9):.0f}/{q(slm,.95):.0f} ms | bypass n={len(byp)} p50/p95={q(byp,.5):.2f}/{q(byp,.95):.2f} ms")
