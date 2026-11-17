#!/usr/bin/env python3
"""Builds bench/typed_gate.jsonl: the Quick Fix gate set (typed text, not dictation). Standard library only.

  python3 bench/build_typed_gate.py

Deterministic. Sources: the references of bench/correction_eval.jsonl and correction_heldout.jsonl (correct
everyday text), sentences from docs/ and README.md (correct technical text), and the hand-written cases at the
bottom (Indian-English typing patterns, protected tokens). Errors are seeded into correct sentences, one per
case, so every case has one known reference. Fields:

  id, category, app_context, input, reference, must_keep (tokens that must survive byte for byte), group
  group: clean_everyday | clean_technical | error (must-fix classes) | indian | protected

Score with: keyreed --eval-typed-jsonl bench/typed_gate.jsonl > out.json; python3 bench/score_typed.py out.json
"""
import glob
import json
import os
import random
import re

BENCH = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(BENCH)
SENT = re.compile(r"(?<=[.!?])\s+")
CONTEXTS = ["email", "chat", "document", "general"]

# Common real typing misspellings (wrong -> right).
MISSPELL = {
    "receive": "recieve", "definitely": "definately", "separate": "seperate", "until": "untill", "which": "wich",
    "the": "teh", "their": "thier", "occurred": "occured", "accommodate": "accomodate", "believe": "beleive",
    "because": "becuase", "business": "buisness", "calendar": "calender", "necessary": "neccessary",
    "tomorrow": "tommorow", "really": "realy", "already": "allready", "address": "adress", "government": "goverment",
    "environment": "enviroment", "beginning": "begining", "different": "diffrent", "probably": "probaly",
    "something": "somthing", "without": "wihtout", "message": "mesage", "meeting": "meating", "about": "abuot",
    "should": "shoud", "would": "woudl", "could": "coudl", "their": "thier", "people": "poeple",
    "change": "chnage", "before": "befor", "process": "proccess", "release": "relase", "version": "verison",
    "problem": "probelm", "available": "availabe", "information": "infromation", "important": "importnat",
    "with": "wiht", "from": "form", "just": "jsut", "know": "konw", "what": "waht", "when": "wehn",
}
CONTRACTION_DROP = {"don't": "dont", "can't": "cant", "didn't": "didnt", "doesn't": "doesnt", "isn't": "isnt",
                    "won't": "wont", "I'm": "Im", "I've": "Ive", "wasn't": "wasnt", "couldn't": "couldnt"}
SWAPS = [("their", "there"), ("there", "their"), ("your", "you're"), ("its", "it's"), ("it's", "its"),
         ("is", "are"), ("are", "is"), ("was", "were"), ("were", "was"), ("has", "have"), ("have", "has"),
         ("than", "then"), ("then", "than"), ("does", "do")]
DOUBLE = ["the", "a", "to", "is", "and", "of", "in", "for"]


def lines(p):
    return open(p, encoding="utf-8").read().splitlines()


def everyday():
    out = []
    for f in ("correction_eval.jsonl", "correction_heldout.jsonl"):
        for l in lines(os.path.join(BENCH, f)):
            if l.strip():
                out += [s for s in SENT.split(json.loads(l)["reference"].strip()) if 5 <= len(s.split()) <= 40]
    out = [s for s in sorted(set(out)) if s[0].isupper() and s[-1] in ".!?"]
    random.Random(5).shuffle(out)
    return out


def technical():
    out = []
    files = sorted(glob.glob(os.path.join(REPO, "docs", "**", "*.md"), recursive=True)) + [os.path.join(REPO, "README.md")]
    for f in files:
        fenced = False
        for l in open(f, encoding="utf-8", errors="replace"):
            l = l.strip()
            if l.startswith("```"):
                fenced = not fenced
                continue
            if fenced or not l or l[0] in "#|>!<" or l.startswith("---"):
                continue
            l = re.sub(r"^(?:[-*+]|\d+\.)\s+(?:\[[ x]\]\s+)?", "", l)
            l = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", l).replace("**", "").replace("`", "")
            for s in SENT.split(l):
                if 6 <= len(s.split()) <= 35 and s[0].isupper() and s[-1] in ".!?" and "*" not in s and "|" not in s \
                        and not re.search(r"[^\x00-\x7f]", s):
                    out.append(s)
    out = sorted(set(out))
    random.Random(7).shuffle(out)
    return out


def word_at(s, w):
    return [m for m in re.finditer(rf"(?<![\w'-]){re.escape(w)}(?![\w'-])", s) if m.start() > 0]


def typo(w, rng):
    """A realistic slip inside a word: two neighbouring letters swapped, or one letter dropped."""
    i = rng.randrange(1, len(w) - 2)
    if rng.random() < 0.6 and w[i] != w[i + 1]:
        return w[:i] + w[i + 1] + w[i] + w[i + 2:]
    return w[:i] + w[i + 1:]


# Seeded categories and how many of each (spelling is the largest class of real typing errors).
QUOTA = {"spelling": 70, "spelling_apostrophe": 20, "agreement_confusable": 40, "doubled_word": 25,
         "capitalisation": 25, "article_missing": 20}


def seed(s, rng, want):
    """One seeded error of category `want` in a correct sentence: the bad sentence, or None."""
    for k in [want]:
        if k == "spelling":
            hits = [(m, MISSPELL[w]) for w in MISSPELL for m in word_at(s, w)]
            if hits and rng.random() < 0.5:
                m, bad = rng.choice(hits)
                return s[:m.start()] + bad + s[m.end():], "spelling"
            prot = set(protected_tokens(s))
            words = [m for m in re.finditer(r"(?<![\w'./\\@:-])[a-z]{6,}(?![\w'./\\@:-])", s) if m.group() not in prot]
            if words:
                m = rng.choice(words)
                return s[:m.start()] + typo(m.group(), rng) + s[m.end():], "spelling"
        elif k == "spelling_apostrophe":
            hits = [(m, CONTRACTION_DROP[w]) for w in CONTRACTION_DROP for m in word_at(s, w)]
            if hits:
                m, bad = rng.choice(hits)
                return s[:m.start()] + bad + s[m.end():], "spelling_apostrophe"
        elif k == "agreement_confusable":
            hits = [(m, b) for a, b in SWAPS for m in word_at(s, a)]
            if hits:
                m, b = rng.choice(hits)
                return s[:m.start()] + b + s[m.end():], "agreement_confusable"
        elif k == "doubled_word":
            hits = [m for w in DOUBLE for m in word_at(s, w)]
            if hits:
                m = rng.choice(hits)
                return s[:m.end()] + " " + m.group() + s[m.end():], "doubled_word"
        elif k == "capitalisation":
            if s[0].isupper() and s[1:2].islower() and not s.startswith("I "):
                return s[0].lower() + s[1:], "capitalisation"
        elif k == "article_missing":
            hits = word_at(s, "the")
            if hits:
                m = rng.choice(hits)
                return s[:m.start()] + s[m.end():].lstrip(), "article_missing"
    return None


PROTECT = re.compile(r"https?://\S+|\S+@\S+\.\w+|[A-Za-z]:[\\/]\S+|\S*[\\/]\S+|\b\w*\d\w*\b|\b\w+_\w+\b|\b[a-z]+[A-Z]\w*\b|\b[A-Z]{2,}\w*\b")


def protected_tokens(s):
    return sorted({m.group().rstrip(".,;:") for m in PROTECT.finditer(s) if len(m.group().rstrip(".,;:")) > 1})


def main():
    cases = []
    rng = random.Random(13)
    ev, tech = everyday(), technical()
    for i, s in enumerate(ev[:110]):
        cases.append({"id": f"ce_{i:03d}", "category": "clean_everyday", "group": "clean_everyday",
                      "app_context": CONTEXTS[i % len(CONTEXTS)], "input": s, "reference": s, "must_keep": protected_tokens(s)})
    for i, s in enumerate(tech[:110]):
        cases.append({"id": f"ct_{i:03d}", "category": "clean_technical", "group": "clean_technical",
                      "app_context": "code" if i % 4 == 0 else "document", "input": s, "reference": s, "must_keep": protected_tokens(s)})
    n, left = 0, dict(QUOTA)
    # Everyday sentences are fewer, so the error pool reuses them (a different case: a seeded error).
    foreign = re.compile(r"(?i)\b(?:hola|por|favor|equipo|yaar|bhai|hai|kya|nahi|karo|kal|theek)\b")
    every = [(s, "everyday") for s in ev if not foreign.search(s)]
    techp = [(s, "technical") for s in tech[110:]]
    rng.shuffle(every)
    rng.shuffle(techp)
    pool = [x for pair in zip(every, techp) for x in pair]
    for s, src in pool:
        r = next((x for x in (seed(s, rng, k) for k in sorted(left, key=lambda k: -left[k]) if left[k] > 0) if x), None)
        if not r:
            continue
        bad, cat = r
        left[cat] -= 1
        cases.append({"id": f"er_{n:03d}", "category": cat, "group": "error", "source": src,
                      "app_context": (CONTEXTS[n % len(CONTEXTS)] if src == "everyday" else "document"),
                      "input": bad, "reference": s, "must_keep": [t for t in protected_tokens(s) if t in bad]})
        n += 1
        if not any(left.values()):
            break
    for i, (bad, good, ctx) in enumerate(INDIAN):
        cases.append({"id": f"in_{i:03d}", "category": "indian_english", "group": "indian", "app_context": ctx,
                      "input": bad, "reference": good, "must_keep": protected_tokens(good)})
    for i, (bad, good, keep, ctx) in enumerate(PROTECTED):
        cases.append({"id": f"pt_{i:03d}", "category": "protected", "group": "protected", "app_context": ctx,
                      "input": bad, "reference": good, "must_keep": keep})
    out = os.path.join(BENCH, "typed_gate.jsonl")
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    by = {}
    for c in cases:
        by[c["category"]] = by.get(c["category"], 0) + 1
    print(f"wrote {out}: {len(cases)} cases", by)


# Typed Indian-English patterns with one minimal fix each (input, reference, context).
INDIAN = [
    ("Kindly do the needful and share the report by evening.", "Please share the report by evening.", "email"),
    ("We were discussing about the new pricing yesterday.", "We were discussing the new pricing yesterday.", "email"),
    ("Can you explain me how the billing works?", "Can you explain to me how the billing works?", "chat"),
    ("I have been waiting since two hours for the build.", "I have been waiting for two hours for the build.", "chat"),
    ("He don't know about the outage yet.", "He doesn't know about the outage yet.", "chat"),
    ("They was not informed about the change.", "They were not informed about the change.", "email"),
    ("Please send me the informations about the trip.", "Please send me the information about the trip.", "email"),
    ("She don't have access to the shared drive.", "She doesn't have access to the shared drive.", "chat"),
    ("We was planning to deploy on Thursday.", "We were planning to deploy on Thursday.", "chat"),
    ("I am waiting since three days for a reply.", "I am waiting for three days for a reply.", "email"),
    ("Let us discuss about the budget in the call.", "Let us discuss the budget in the call.", "email"),
    ("Can you explain me the error in the logs?", "Can you explain to me the error in the logs?", "chat"),
    ("You was right about the cache issue.", "You were right about the cache issue.", "chat"),
    ("It don't work on my laptop.", "It doesn't work on my laptop.", "chat"),
    ("The team discussed about the roadmap for an hour.", "The team discussed the roadmap for an hour.", "document"),
    ("We have been working on this since six months.", "We have been working on this for six months.", "document"),
    ("Kindly do the needful and approve the leave.", "Please approve the leave.", "email"),
    ("Thanks for the informations you shared.", "Thanks for the information you shared.", "email"),
    ("i will send the file after lunch.", "I will send the file after lunch.", "chat"),
    ("Yesterday i spoke to the vendor about the delay.", "Yesterday I spoke to the vendor about the delay.", "email"),
    ("Can you check if i have the right access?", "Can you check if I have the right access?", "chat"),
    ("Please explain me the steps once more.", "Please explain to me the steps once more.", "email"),
    ("They was asking about the invoice.", "They were asking about the invoice.", "chat"),
    ("He don't reply to emails on weekends.", "He doesn't reply to emails on weekends.", "chat"),
    ("We discussed about it in the standup.", "We discussed it in the standup.", "chat"),
    ("I am stuck on this issue since two days.", "I am stuck on this issue for two days.", "chat"),
    ("Please share the informations at the earliest.", "Please share the information as soon as possible.", "email"),
    ("i think the server restarted at night.", "I think the server restarted at night.", "chat"),
    ("She was discussing about the offer with HR.", "She was discussing the offer with HR.", "email"),
    ("You was supposed to join the call.", "You were supposed to join the call.", "chat"),
]

# Protected tokens (input, reference, tokens that must survive, context). Some carry one error elsewhere.
PROTECTED = [
    ("Open https://example.com/docs/setup and follow the steps.", "Open https://example.com/docs/setup and follow the steps.", ["https://example.com/docs/setup"], "email"),
    ("The config is in C:\\Users\\dev\\app\\config.json on my machine.", "The config is in C:\\Users\\dev\\app\\config.json on my machine.", ["C:\\Users\\dev\\app\\config.json"], "chat"),
    ("Send it to priya.sharma@example.org before noon.", "Send it to priya.sharma@example.org before noon.", ["priya.sharma@example.org"], "email"),
    ("We upgraded to v2.14.3 last week.", "We upgraded to v2.14.3 last week.", ["v2.14.3"], "chat"),
    ("Call get_user_profile before you render the page.", "Call get_user_profile before you render the page.", ["get_user_profile"], "document"),
    ("The useEffect hook runs twice in strict mode.", "The useEffect hook runs twice in strict mode.", ["useEffect"], "document"),
    ("Run git rebase -i HEAD~3 to squash them.", "Run git rebase -i HEAD~3 to squash them.", ["git rebase -i HEAD~3"], "chat"),
    ("The service listens on port 8443 now.", "The service listens on port 8443 now.", ["8443"], "chat"),
    ("Set MAX_RETRIES to 5 in the env file.", "Set MAX_RETRIES to 5 in the env file.", ["MAX_RETRIES"], "document"),
    ("Ask Arjun and Meenakshi about the AWS bill.", "Ask Arjun and Meenakshi about the AWS bill.", ["Arjun", "Meenakshi", "AWS"], "email"),
    ("The file lives at /var/log/nginx/error.log on prod.", "The file lives at /var/log/nginx/error.log on prod.", ["/var/log/nginx/error.log"], "chat"),
    ("Use npm install --save-dev eslint for linting.", "Use npm install --save-dev eslint for linting.", ["npm install --save-dev eslint"], "document"),
    ("Our SLA is 99.9 percent for the API.", "Our SLA is 99.9 percent for the API.", ["SLA", "99.9", "API"], "email"),
    ("The PR #482 fixes the login bug.", "The PR #482 fixes the login bug.", ["#482"], "chat"),
    ("Meet me at 4:30 pm in room B-204.", "Meet me at 4:30 pm in room B-204.", ["4:30", "B-204"], "chat"),
    ("Open https://example.com/pricing, it has the the new plans.", "Open https://example.com/pricing, it has the new plans.", ["https://example.com/pricing"], "email"),
    ("I definately pushed fix_auth_flow to the branch.", "I definitely pushed fix_auth_flow to the branch.", ["fix_auth_flow"], "chat"),
    ("Their is a typo in README.md line 12.", "There is a typo in README.md line 12.", ["README.md", "12"], "chat"),
    ("Please recieve the parcel for Karthik on Friday.", "Please receive the parcel for Karthik on Friday.", ["Karthik"], "email"),
    ("The docker compose file uses postgres:16-alpine now.", "The docker compose file uses postgres:16-alpine now.", ["postgres:16-alpine"], "document"),
    ("Mail ops@keyreed.app if the build breaks again.", "Mail ops@keyreed.app if the build breaks again.", ["ops@keyreed.app"], "email"),
    ("The variable userId is null in the handler.", "The variable userId is null in the handler.", ["userId"], "document"),
    ("Install it with pip install requests==2.31.0 first.", "Install it with pip install requests==2.31.0 first.", ["pip install requests==2.31.0"], "document"),
    ("Version 3.2 adds IPv6 support to the agent.", "Version 3.2 adds IPv6 support to the agent.", ["3.2", "IPv6"], "document"),
    ("We moved the meeting with Dr. Rao to 11 am.", "We moved the meeting with Dr. Rao to 11 am.", ["Dr. Rao", "11"], "email"),
    ("Check D:\\Projects\\keyreed\\target\\release for the exe.", "Check D:\\Projects\\keyreed\\target\\release for the exe.", ["D:\\Projects\\keyreed\\target\\release"], "chat"),
    ("The SHA is 3f9a2c7 and the tag is v0.5.1.", "The SHA is 3f9a2c7 and the tag is v0.5.1.", ["SHA", "3f9a2c7", "v0.5.1"], "chat"),
    ("Use Ctrl+Shift+P to open the command palette.", "Use Ctrl+Shift+P to open the command palette.", ["Ctrl+Shift+P"], "document"),
    ("kubectl get pods -n staging shows two restarts.", "kubectl get pods -n staging shows two restarts.", ["kubectl get pods -n staging"], "chat"),
    ("The JSON key is created_at, not createdAt.", "The JSON key is created_at, not createdAt.", ["JSON", "created_at", "createdAt"], "document"),
    ("Order 1042 ships to Bengaluru on 12 March.", "Order 1042 ships to Bengaluru on 12 March.", ["1042", "Bengaluru", "12"], "email"),
    ("Ping Sanjay on Slack about the iOS release.", "Ping Sanjay on Slack about the iOS release.", ["Sanjay", "Slack", "iOS"], "chat"),
    ("It is defined in src/main.rs around line 400.", "It is defined in src/main.rs around line 400.", ["src/main.rs", "400"], "document"),
    ("The endpoint /api/v2/users returns 404 for me.", "The endpoint /api/v2/users returns 404 for me.", ["/api/v2/users", "404"], "chat"),
    ("Our OKR for Q3 is a 20 percent faster checkout.", "Our OKR for Q3 is a 20 percent faster checkout.", ["OKR", "Q3", "20"], "email"),
    ("Visit www.example.in for the tickets.", "Visit www.example.in for the tickets.", ["www.example.in"], "chat"),
    ("I has sent the logs from /tmp/crash.txt already.", "I have sent the logs from /tmp/crash.txt already.", ["/tmp/crash.txt"], "chat"),
    ("Neha said the the deadline is 15 June.", "Neha said the deadline is 15 June.", ["Neha", "15"], "chat"),
    ("Restart the box with sudo systemctl restart nginx please.", "Restart the box with sudo systemctl restart nginx please.", ["sudo systemctl restart nginx"], "chat"),
    ("The hash map in cache.rs grows without a limit.", "The hash map in cache.rs grows without a limit.", ["cache.rs"], "document"),
]

if __name__ == "__main__":
    main()
