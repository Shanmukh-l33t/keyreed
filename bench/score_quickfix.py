#!/usr/bin/env python3
"""Scores Quick Fix output on the typed-text gate set (bench/quickfix_gate.jsonl): plan gates G1, G2, G3 and G5.

Usage:
  score_quickfix.py [--gate bench/quickfix_gate.jsonl] label=output.jsonl [label=output.jsonl ...]
  score_quickfix.py --self-test
Each output line is {"id": ..., "output": "...", "edits": [{"start", "length", "replacement", ...}]}; "edits" is optional
(start and length index into the case's input). Every gate id must appear exactly once.

  G1  correct sentences (input == reference) the engine changed: lower is better, target <= 0.3% (everyday and technical)
  G2  sentences with an error that gained a wrong edit (a word-level edit the reference does not make): target <= 1%
  G3  fix rate on the must-fix classes (output equals the reference): target >= 70% at launch, 85% goal
  G5  cases where a protected token (must_keep) was changed or edited over: target zero
Exits 2 on malformed input; gate misses are reported, not an error.
"""
import difflib
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

DEFAULT_GATE = Path(__file__).with_name("quickfix_gate.jsonl")
MUST_FIX = {"spelling", "agreement", "confusable", "doubled_word", "capitalisation"}
TOKEN = re.compile(r"\w+(?:'\w+)*|[^\w\s]")


class Malformed(Exception):
    pass


def norm(s):
    return " ".join(s.split())


def tokens(s):
    return TOKEN.findall(s.replace("\u2019", "'"))


def word_edits(a, b):
    ta, tb = tokens(a), tokens(b)
    sm = difflib.SequenceMatcher(None, ta, tb, autojunk=False)
    return {(i1, i2, tuple(tb[j1:j2])) for op, i1, i2, j1, j2 in sm.get_opcodes() if op != "equal"}


def read_jsonl(path, what):
    rows = []
    try:
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                if not line.strip():
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError as e:
                    raise Malformed(f"{what} {path} line {n}: invalid JSON ({e.msg})")
                if not isinstance(row, dict):
                    raise Malformed(f"{what} {path} line {n}: not a JSON object")
                rows.append((n, row))
    except OSError as e:
        raise Malformed(f"cannot read {path}: {e.strerror}")
    return rows


def load_gate(path):
    gate, seen = [], set()
    for n, c in read_jsonl(path, "gate"):
        for k, t in (("id", str), ("category", str), ("context", str), ("input", str), ("reference", str),
                     ("must_keep", list), ("must_not_contain", list)):
            if not isinstance(c.get(k), t):
                raise Malformed(f"gate {path} line {n}: field '{k}' missing or not {t.__name__}")
        if c["id"] in seen:
            raise Malformed(f"gate {path} line {n}: duplicate id {c['id']}")
        seen.add(c["id"])
        gate.append(c)
    if not gate:
        raise Malformed(f"gate {path} is empty")
    return gate


def check_edits(n, path, rec, inp):
    edits = rec.get("edits")
    if edits is None:
        return None
    if not isinstance(edits, list):
        raise Malformed(f"{path} line {n}: 'edits' is not a list")
    for e in edits:
        ok = (isinstance(e, dict) and all(type(e.get(k)) is int and e[k] >= 0 for k in ("start", "length"))
              and isinstance(e.get("replacement"), str) and e["start"] + e["length"] <= len(inp))
        if not ok:
            raise Malformed(f"{path} line {n}: bad edit in id {rec['id']}: needs int start/length inside the input and a string replacement")
    return edits


def load_outputs(path, gate):
    by_id = {c["id"]: c for c in gate}
    out = {}
    for n, r in read_jsonl(path, "output"):
        if not isinstance(r.get("id"), str) or not isinstance(r.get("output"), str):
            raise Malformed(f"{path} line {n}: needs string fields 'id' and 'output'")
        if r["id"] not in by_id:
            raise Malformed(f"{path} line {n}: id {r['id']} is not in the gate set")
        if r["id"] in out:
            raise Malformed(f"{path} line {n}: duplicate id {r['id']}")
        r["edits"] = check_edits(n, path, r, by_id[r["id"]]["input"])
        out[r["id"]] = r
    missing = [i for i in by_id if i not in out]
    if missing:
        raise Malformed(f"{path}: {len(missing)} gate ids have no output (first: {', '.join(missing[:5])})")
    return out


def protected_hit(case, rec):
    """True if a must_keep token is gone from the output or an edit overlaps its place in the input."""
    if any(k not in rec["output"] for k in case["must_keep"]):
        return True
    for k in case["must_keep"]:
        for m in re.finditer(re.escape(k), case["input"]):
            for e in rec["edits"] or []:
                s, end = e["start"], e["start"] + e["length"]
                if (s < m.end() and end > m.start()) or (e["length"] == 0 and m.start() < s < m.end()):
                    return True
    return False


def judge(case, rec):
    inp, ref, out = case["input"], case["reference"], rec["output"]
    clean = inp == ref
    wrong = None if clean else bool(word_edits(inp, out) - word_edits(inp, ref))
    return {
        "category": case["category"], "context": case["context"], "clean": clean,
        "changed": clean and norm(out) != norm(inp),
        "wrong_edit": wrong,
        "fixed": (not clean) and norm(out) == norm(ref),
        "has_keep": bool(case["must_keep"]), "protected_hit": protected_hit(case, rec),
        "bad_hit": any(b.lower() in out.lower() for b in case["must_not_contain"]),
        "keep_ok": all(k in out for k in case["must_keep"]),
        "edits": rec["edits"] or [],
    }


def tally(rows):
    err = [r for r in rows if not r["clean"]]
    clean = [r for r in rows if r["clean"]]
    fix = [r for r in err if r["category"] in MUST_FIX]
    prot = [r for r in rows if r["has_keep"]]
    return {
        "n": len(rows),
        "g1": (sum(r["changed"] for r in clean), len(clean)),
        "g2": (sum(r["wrong_edit"] for r in err), len(err)),
        "g3": (sum(r["fixed"] for r in fix), len(fix)),
        "g5": (sum(r["protected_hit"] for r in prot), len(prot)),
        "fixrate": (sum(r["fixed"] for r in err), len(err)),
    }


def pct(t, digits=1):
    return "-" if t[1] == 0 else f"{100 * t[0] / t[1]:.{digits}f}%"


def row_text(name, t, flag=""):
    cells = [pct(t["g1"]), pct(t["g2"]), pct(t["g3"]) if t["g3"][1] else "-", str(t["g5"][0]) if t["g5"][1] else "-"]
    return f"   {name + flag:<18}{t['n']:>5}  {cells[0]:>7} {cells[1]:>7} {cells[2]:>7} {cells[3]:>5}   fix {pct(t['fixrate'])}"


def report(label, gate, results):
    rows = [judge(c, results[c["id"]]) for c in gate]
    total = tally(rows)
    everyday = tally([r for r in rows if r["category"] == "clean_everyday"])
    technical = tally([r for r in rows if r["category"] == "clean_technical"])
    print(f"== {label}: n={total['n']}")
    print("   gate  target                 result")
    print(f"   G1    <= 0.3% everyday       {pct(everyday['g1'], 2)} ({everyday['g1'][0]}/{everyday['g1'][1]})")
    print(f"   G1    <= 0.3% technical      {pct(technical['g1'], 2)} ({technical['g1'][0]}/{technical['g1'][1]})")
    print(f"   G2    <= 1% with an error    {pct(total['g2'], 2)} ({total['g2'][0]}/{total['g2'][1]})")
    print(f"   G3    >= 70% must-fix        {pct(total['g3'])} ({total['g3'][0]}/{total['g3'][1]})")
    print(f"   G5    zero protected changed {total['g5'][0]} of {total['g5'][1]} cases with protected tokens")
    inv = [r for r in rows if r["bad_hit"] and not r["clean"]]
    print(f"   must_keep intact {sum(r['keep_ok'] for r in rows)}/{len(rows)}; wrong form still present in {len(inv)} error cases")
    layers = defaultdict(int)
    for r in rows:
        for e in r["edits"]:
            layers[e.get("layer", "unlabelled")] += 1
    if layers:
        print("   edits reported: " + ", ".join(f"{k}={v}" for k, v in sorted(layers.items())))
    for title, key in (("category", "category"), ("context", "context")):
        print(f"   per {title:<9}{'n':>7}  {'G1':>7} {'G2':>7} {'G3':>7} {'G5':>5}   (* = must-fix class)")
        for v in sorted({r[key] for r in rows}):
            print(row_text(v, tally([r for r in rows if r[key] == v]), "*" if v in MUST_FIX else ""))
    return total


def self_test():
    import os
    import tempfile
    gate = [
        dict(id="a", category="clean_everyday", context="chat", input="Hello there.", reference="Hello there.", must_keep=[], must_not_contain=[]),
        dict(id="b", category="spelling", context="email", input="I recieve it.", reference="I receive it.", must_keep=[], must_not_contain=["recieve"]),
        dict(id="c", category="protected", context="notes", input=r"Open C:\x\y.txt plese", reference=r"Open C:\x\y.txt please",
             must_keep=[r"C:\x\y.txt"], must_not_contain=[]),
    ]
    mk = lambda outs: {c["id"]: dict(id=c["id"], output=o, edits=None) for c, o in zip(gate, outs)}
    ref = [c["reference"] for c in gate]
    perfect = tally([judge(c, mk(ref)[c["id"]]) for c in gate])
    assert perfect["g1"] == (0, 1) and perfect["g2"] == (0, 2) and perfect["g3"] == (1, 1) and perfect["g5"] == (0, 1), perfect
    same = tally([judge(c, mk([c["input"] for c in gate])[c["id"]]) for c in gate])
    assert same["g1"] == (0, 1) and same["g2"] == (0, 2) and same["g3"] == (0, 1) and same["fixrate"] == (0, 2), same
    bad = mk(["Hello there!", "I receive this.", r"Open C:\x\z.txt please"])
    t = tally([judge(c, bad[c["id"]]) for c in gate])
    assert t["g1"] == (1, 1) and t["g2"] == (2, 2) and t["g3"] == (0, 1) and t["g5"] == (1, 1), t
    overlap = dict(id="c", output=gate[2]["input"], edits=[dict(start=8, length=2, replacement="Z")])
    assert protected_hit(gate[2], overlap)
    outside = dict(id="c", output=gate[2]["input"], edits=[dict(start=18, length=5, replacement="please")])
    assert not protected_hit(gate[2], outside)
    with tempfile.TemporaryDirectory() as d:
        g = os.path.join(d, "gate.jsonl")
        o = os.path.join(d, "out.jsonl")
        open(g, "w", encoding="utf-8").write("\n".join(json.dumps(c) for c in gate) + "\n")
        full = [dict(id=c["id"], output=c["reference"]) for c in gate]
        for label, rows in (("missing id", full[1:]), ("duplicate id", full + full[:1]),
                            ("unknown id", full + [dict(id="zz", output="x")]), ("non-string output", [dict(id="a", output=3)] + full[1:]),
                            ("bad edit", [dict(id="a", output="x", edits=[dict(start=0, length=99, replacement="")])] + full[1:])):
            open(o, "w", encoding="utf-8").write("\n".join(json.dumps(r) for r in rows) + "\n")
            try:
                load_outputs(o, load_gate(g))
            except Malformed:
                continue
            raise AssertionError(f"malformed output accepted: {label}")
        open(o, "w", encoding="utf-8").write("{not json\n")
        try:
            load_outputs(o, load_gate(g))
            raise AssertionError("invalid JSON accepted")
        except Malformed:
            pass
        open(o, "w", encoding="utf-8").write("\n".join(json.dumps(r) for r in full) + "\n")
        report("self-test", load_gate(g), load_outputs(o, load_gate(g)))
    if DEFAULT_GATE.exists():
        load_gate(DEFAULT_GATE)
    print("self-test ok")


def main(argv):
    if "--self-test" in argv:
        self_test()
        return 0
    gate_path, args = DEFAULT_GATE, []
    it = iter(argv)
    for a in it:
        if a == "--gate":
            gate_path = next(it, None)
            if gate_path is None:
                print("--gate needs a path", file=sys.stderr)
                return 2
        else:
            args.append(a)
    if not args or any("=" not in a for a in args):
        print(__doc__, file=sys.stderr)
        return 2
    try:
        gate = load_gate(gate_path)
        runs = [(a.split("=", 1)[0], load_outputs(a.split("=", 1)[1], gate)) for a in args]
    except Malformed as e:
        print(f"malformed input: {e}", file=sys.stderr)
        return 2
    for label, results in runs:
        report(label, gate, results)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
