#!/usr/bin/env python3
"""Scores the Quick Fix gate (plan targets G1, G2, G3, G5) from `keyreed --eval-typed-jsonl` output.
Standard library only.

  python3 bench/score_typed.py out.json [--json scores.json]

Every record carries the engine's candidate edits with their class and whether the edit-class table lets it
through silently in that context. The scorer applies edits itself, so it can report each class alone, all
classes together, and "as shipped" (silent edits only):

  G1  correct sentences changed (any edit on a clean case is wrong), separately for everyday and technical text
  G2  sentences with an error that gained a wrong edit (an edit that does not move the text toward the reference)
  G3  fix rate on must-fix classes: output equals the reference
  G5  cases where a protected token (must_keep) did not survive byte for byte
Targets: G1 <= 0.3 %, G2 <= 1 %, G3 >= 70 % (goal 85 %), G5 = 0.
"""
import json
import re
import sys
from collections import defaultdict

WORD = re.compile(r"\w+(?:'\w+)?|[^\w\s]")
MUST_FIX = {"spelling", "spelling_apostrophe", "agreement_confusable", "doubled_word", "capitalisation"}


def dist(a, b):
    a, b = WORD.findall(a), WORD.findall(b)
    d = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        p, d[0] = d[0], i
        for j, y in enumerate(b, 1):
            p, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, p + (x != y))
    return d[-1]


def apply(text, edits):
    for e in sorted(edits, key=lambda e: -e["start"]):
        text = text[:e["start"]] + e["replacement"] + text[e["start"] + e["length"]:]
    return text


def wrong(rec, e):
    if rec["input"] == rec["reference"]:
        return True
    return dist(apply(rec["input"], [e]), rec["reference"]) >= dist(rec["input"], rec["reference"])


def score(recs, keep):
    """keep(rec, edit) -> bool selects the edits applied. Returns the gate numbers and counts."""
    n = defaultdict(int)
    for r in recs:
        es = [e for e in r["edits"] if keep(r, e)]
        out = apply(r["input"], es)
        g = r["group"]
        if g in ("clean_everyday", "clean_technical") or (g == "protected" and r["input"] == r["reference"]):
            key = "technical" if g == "clean_technical" else "everyday"
            n[f"clean_{key}"] += 1
            n[f"changed_{key}"] += out != r["input"]
        else:
            n["errors"] += 1
            n["gained_wrong"] += any(wrong(r, e) for e in es)
            if r["category"] in MUST_FIX:
                n["must_fix"] += 1
                n["fixed"] += out == r["reference"]
            n["all_fixed"] += out == r["reference"]
        n["cases"] += 1
        n["protected_lost"] += any(t not in out for t in r.get("must_keep", []) if t in r["input"])
        n["edits"] += len(es)
    pct = lambda a, b: round(100.0 * n[a] / n[b], 2) if n[b] else None
    return {"G1_everyday": pct("changed_everyday", "clean_everyday"), "G1_technical": pct("changed_technical", "clean_technical"),
            "G2": pct("gained_wrong", "errors"), "G3": pct("fixed", "must_fix"), "fixed_all_pct": pct("all_fixed", "errors"),
            "G5_cases": n["protected_lost"], "edits": n["edits"], "cases": n["cases"]}


def passes(s):
    return (s["G1_everyday"] or 0) <= 0.3 and (s["G1_technical"] or 0) <= 0.3 and (s["G2"] or 0) <= 1.0 and s["G5_cases"] == 0


def main():
    path = sys.argv[1]
    data = json.load(open(path, encoding="utf-8"))
    recs = data["records"]
    classes = sorted({e["class"] for r in recs for e in r["edits"]})
    contexts = sorted({r["app_context"] for r in recs})
    report = {"engine": data.get("engine"), "as_shipped": score(recs, lambda r, e: e.get("silent")),
              "all_candidates": score(recs, lambda r, e: True), "per_class": {}, "per_class_context": {}}
    for c in classes:
        report["per_class"][c] = score(recs, lambda r, e, c=c: e["class"] == c)
        for ctx in contexts:
            sub = [r for r in recs if r["app_context"] == ctx]
            report["per_class_context"][f"{c} @ {ctx}"] = score(sub, lambda r, e, c=c: e["class"] == c)
    ms = sorted(r.get("ms", 0.0) for r in recs)
    report["timing_ms"] = {"p50": ms[len(ms) // 2], "p95": ms[min(len(ms) - 1, int(0.95 * len(ms)))], "max": ms[-1]} if ms else {}

    cols = ["G1_everyday", "G1_technical", "G2", "G3", "fixed_all_pct", "G5_cases", "edits"]
    print(f"Quick Fix gate: {len(recs)} cases, engine {report['engine']}")
    print(f"{'set':42s} " + " ".join(f"{c:>13s}" for c in cols) + "  pass(G1,G2,G5)")
    rows = [("as shipped (silent edits)", report["as_shipped"]), ("all candidate edits", report["all_candidates"])]
    rows += [(f"class {c}", s) for c, s in report["per_class"].items()]
    for name, s in rows:
        print(f"{name:42s} " + " ".join(f"{str(s[c]):>13s}" for c in cols) + f"  {'yes' if passes(s) else 'no'}")
    print("timing (ms per case):", report["timing_ms"])
    if "--json" in sys.argv:
        json.dump(report, open(sys.argv[sys.argv.index("--json") + 1], "w", encoding="utf-8"), indent=1)


if __name__ == "__main__":
    main()
