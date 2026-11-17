#!/usr/bin/env python3
"""Regenerates the two Quick Fix test fixtures from the Python reference. Not run by `cargo test`.

  wordpiece_golden.json    ids and word ids that the Python `tokenizers` package gives for
                           encode(words, is_pretokenized=True), from the model's own tokenizer.json
  quickfix_reference.json  the edits the Python reference (evalkit.Engine with the published guards and KeyReed's
                           protections, run as in evalkit.fix) makes on single sentences: [start, length, replacement]
                           in code points of the sentence

Sentences are the inputs of bench/typed_gate.jsonl (written for KeyReed; no third-party text). Run with the scratch
Python packages on the path:

  PYTHONPATH=<kr_bench>/site python crates/vi-rewrite/tests/data/make_fixtures.py <model_dir> <kr_bench>/eval

<model_dir> is the int8 grammar model folder (gector-bert-en-int8), the second argument the folder holding
evalkit.py and data/words_alpha.txt.
"""
import json
import os
import re
import sys

model_dir, eval_dir = sys.argv[1], sys.argv[2]
here = os.path.dirname(os.path.abspath(__file__))
repo = os.path.abspath(os.path.join(here, "..", "..", "..", ".."))
sys.path.insert(0, eval_dir)
import evalkit  # noqa: E402
from tokenizers import Tokenizer  # noqa: E402

SENT = evalkit.SENT
sentences = []
for line in open(os.path.join(repo, "bench", "typed_gate.jsonl"), encoding="utf-8"):
    if line.strip():
        for s in SENT.split(json.loads(line)["input"].strip()):
            if s and s not in sentences:
                sentences.append(s)

# ---- tokenizer golden vectors
tok = Tokenizer.from_file(os.path.join(model_dir, "tokenizer.json"))
odd = ["café", "naïve", "\u201cquoted\u201d", "\u2019", "\u2014", "\u2026", "😀", "日本語", "a\u200bb", "ß", "İstanbul", "e.g.",
       "x" * 150, "$100", "C++", "50%", "100°C", "×", "½", "\u00a0", "\t", "O'Brien", "rock'n'roll", "ﬁne", "Ǆ", "a\u0301",
       "Привет", "مرحبا", "‹x›", "¿", "§5", "\ufeffword", "tab\tinside", "un\u00adbreakable", "x" + "y" * 99, "x" + "y" * 100]
cases = []
for s in sentences:
    words = evalkit.WORD.findall(s.replace("\u2019", "'"))
    cases.append(words)
for w in odd:
    cases.append([w])
    cases.append(["see", w, "now"])
cases.append([])
out = []
for words in cases:
    e = tok.encode(words, is_pretokenized=True)
    out.append({"words": words, "ids": e.ids, "word_ids": e.word_ids})
json.dump({"made_by": "Python tokenizers %s, tokenizer.json of gector-bert-en-int8, encode(words, is_pretokenized=True)"
           % __import__("tokenizers").__version__, "cases": out}, open(os.path.join(here, "wordpiece_golden.json"), "w", encoding="utf-8"),
          ensure_ascii=False, indent=0)

# ---- engine reference edits
eng = evalkit.Engine(model_dir, guards=True, protect=True)
ref = []
for s in sentences:
    if evalkit.COMMAND.search(s):
        edits = []
    else:
        _, found = eng.g.correct(s)
        edits = [[f["start"], f["length"], f["replacement"]] for f in found if not eng.veto(s, f)]
    ref.append({"input": s, "edits": edits})
import onnxruntime  # noqa: E402
json.dump({"made_by": "Python reference evalkit.Engine(guards=True, protect=True), int8 model, onnxruntime %s, tokenizers %s; "
           "sentences from bench/typed_gate.jsonl" % (onnxruntime.__version__, __import__("tokenizers").__version__),
           "sentences": ref}, open(os.path.join(here, "quickfix_reference.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=0)
print(len(cases), "tokenizer cases;", len(ref), "sentences;", sum(1 for r in ref if r["edits"]), "with edits")
