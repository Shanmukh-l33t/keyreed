//! Quick Fix engine: text in, a list of edits out. Layers: protect, spell, rules, tag, guard (plan C2 and E4).
//!
//! * protect: spans that must never change (URLs, e-mail addresses, paths, numbers, identifiers, capitalised names,
//!   personal-dictionary terms) and sentences that look like shell commands
//! * spell: a [`Speller`] (no-op unless one is supplied)
//! * rules: KeyReed's deterministic rules from `tier0` (pronoun "I", Indian-English patterns)
//! * tag: the grammar tagger behind [`SentenceTagger`], run on the text the first two layers produced
//! * guard: the tagger's published guards (inside `gector`) and KeyReed's own (a change touching or directly following
//!   a protected span, a name changed, a negation added or removed is dropped)
//!
//! Every edit carries a class, and the edit-class table (`data/quickfix-classes.json`) says per class and app
//! category whether it may be applied silently, only after review, or not at all.

use crate::gector::{is_known_word, SentenceTagger, WORD};
use crate::tier0::{RE_ESL_RULES, RE_PRONOUN_I, RE_PRONOUN_IM};
use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use vi_core::AppCategory;

/// URLs, e-mail addresses, paths, numbers, identifiers, acronyms, dotted names (the evaluation kit's pattern).
static PROTECT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://\S+|www\.\S+|\S+@\S+|[A-Za-z]:[\\/]\S+|\S*[\\/]\S+|\b\w*\d\w*\b|\b\w+_\w+\b|\b[a-z]+[A-Z]\w*\b|\b[A-Z]{2,}\w*\b|\b\w+(?:\.\w+)+\b").unwrap()
});
/// A sentence that looks like a shell command is left alone.
static COMMAND: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:^|\s)-{1,2}[A-Za-z][\w-]*|\s\|\s|&&|^\s*(?:sudo|git|cargo|npm|pip|docker|kubectl|curl|cd|ls)\s").unwrap());
const NEG: &[&str] = &["not", "never", "no", "without", "cannot", "none", "nothing", "nobody"];
const ARTICLES: &[&str] = &["a", "an", "the"];
const AGREE: &[&str] = &["is", "are", "was", "were", "am", "be", "been", "has", "have", "had", "does", "do", "did"];
/// A noun changes number only next to one of these; a verb gains or loses "-s" only after a subject pronoun (plan C2, layer 4).
const INFLECTION_CUES: &[&str] = &["two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "many", "several", "few", "both", "these", "those", "multiple", "various", "one", "a", "an", "each", "every", "this", "that", "he", "she", "it", "they", "we", "i", "you"];
/// Command words that are never typos ("sudo", "nginx").
const SHELL_WORDS: &[&str] = &["sudo", "systemctl", "nginx", "docker", "kubectl", "npm", "cargo", "git", "pip", "curl", "ssh", "chmod", "chown", "grep", "sed", "awk", "pwsh", "powershell", "rustc", "gcc", "apt", "yum", "brew"];
/// The only answers accepted for a three-letter typo ("teh", "adn", "hte"): the rest are acronyms more often than typos.
const COMMON_SHORT: &[&str] = &["the", "and", "for", "are", "you", "but", "not", "was", "has", "had", "its", "can", "out"];
const CONFUSABLE: &[&str] = &["their", "there", "they're", "your", "you're", "its", "it's", "then", "than", "to", "too", "two", "lose", "loose", "affect", "effect", "whose", "who's"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Spelling,
    Rule,
    Grammar,
}

/// One change. `start` and `length` are byte offsets into the text given to [`QuickFixEngine::fix`].
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub start: usize,
    pub length: usize,
    pub replacement: String,
    pub layer: Layer,
    pub class: String,
    pub confidence: f32,
    /// The edit-class table lets this edit be applied without review in this app category.
    pub silent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuickFix {
    /// The text with every edit applied.
    pub corrected: String,
    pub edits: Vec<Edit>,
}

/// What may happen to an edit. Ordered: `Off` < `ReviewOnly` < `Silent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Policy {
    Off,
    ReviewOnly,
    Silent,
}

impl Policy {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "off" => Ok(Self::Off),
            "review_only" => Ok(Self::ReviewOnly),
            "silent" => Ok(Self::Silent),
            other => Err(anyhow!("edit-class table: unknown policy {other:?}")),
        }
    }
}

pub struct EditClassTable {
    unknown: Policy,
    contexts: HashMap<String, Policy>,
    /// Per class: its policy, and the policies that replace it in single app categories.
    classes: HashMap<String, (Policy, HashMap<String, Policy>)>,
}

impl EditClassTable {
    /// The table that ships with the engine.
    pub fn builtin() -> Self {
        Self::parse(include_str!("../data/quickfix-classes.json")).expect("built-in edit-class table")
    }

    pub fn parse(json: &str) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_str(json).context("edit-class table is not JSON")?;
        let named = |what: &str, p: &serde_json::Value| -> Result<Policy> { Policy::parse(p.as_str().ok_or_else(|| anyhow!("edit-class table: {what} has no policy"))?) };
        let contexts = v["contexts"].as_object().ok_or_else(|| anyhow!("edit-class table: no contexts"))?;
        let classes = v["classes"].as_object().ok_or_else(|| anyhow!("edit-class table: no classes"))?;
        Ok(Self {
            unknown: Policy::parse(v["unknown_class"].as_str().unwrap_or("review_only"))?,
            contexts: contexts.iter().map(|(k, p)| Ok((k.clone(), named(k, p)?))).collect::<Result<_>>()?,
            classes: classes
                .iter()
                .map(|(k, c)| {
                    let overrides = match c["contexts"].as_object() {
                        Some(o) => o.iter().map(|(ctx, p)| Ok((ctx.clone(), named(ctx, p)?))).collect::<Result<_>>()?,
                        None => HashMap::new(),
                    };
                    Ok((k.clone(), (named(k, &c["policy"])?, overrides)))
                })
                .collect::<Result<_>>()?,
        })
    }

    pub fn policy(&self, class: &str, context: AppCategory) -> Policy {
        let cap = self.contexts.get(context_name(context)).copied().unwrap_or(Policy::Silent);
        let own = match self.classes.get(class) {
            Some((base, overrides)) => overrides.get(context_name(context)).copied().unwrap_or(*base),
            None => self.unknown,
        };
        own.min(cap)
    }
}

pub fn context_name(c: AppCategory) -> &'static str {
    match c {
        AppCategory::Email => "email",
        AppCategory::Chat => "chat",
        AppCategory::Document => "document",
        AppCategory::CodeEditor => "code_editor",
        AppCategory::Terminal => "terminal",
        AppCategory::Notes => "notes",
        AppCategory::Browser => "browser",
        AppCategory::PasswordProtected => "password_protected",
        AppCategory::General => "general",
    }
}

/// The spelling layer. `misspellings` returns `(start, end, suggestions)` byte ranges in `text` for words that are
/// wrong, best suggestion first. The default checks nothing; a Windows implementation can be supplied by the application.
pub trait Speller: Send {
    fn misspellings(&mut self, text: &str) -> Vec<(usize, usize, Vec<String>)>;
}

pub struct NoSpeller;

impl Speller for NoSpeller {
    fn misspellings(&mut self, _text: &str) -> Vec<(usize, usize, Vec<String>)> {
        Vec::new()
    }
}

/// A candidate edit inside one sentence, in byte offsets of that sentence.
struct Cand {
    start: usize,
    end: usize,
    rep: String,
    layer: Layer,
    class: String,
    conf: f32,
}

pub struct QuickFixEngine {
    tagger: Box<dyn SentenceTagger>,
    speller: Box<dyn Speller>,
    table: EditClassTable,
    terms: Option<Regex>,
}

impl QuickFixEngine {
    pub fn new(tagger: Box<dyn SentenceTagger>) -> Self {
        Self { tagger, speller: Box::new(NoSpeller), table: EditClassTable::builtin(), terms: None }
    }

    pub fn with_speller(mut self, speller: Box<dyn Speller>) -> Self {
        self.speller = speller;
        self
    }

    pub fn with_table(mut self, table: EditClassTable) -> Self {
        self.table = table;
        self
    }

    /// Personal-dictionary terms: never changed, whatever the layer.
    pub fn with_terms(mut self, terms: &[String]) -> Self {
        let words: Vec<String> = terms.iter().filter(|t| !t.trim().is_empty()).map(|t| regex::escape(t.trim())).collect();
        self.terms = (!words.is_empty()).then(|| Regex::new(&format!(r"(?i)\b(?:{})\b", words.join("|"))).expect("escaped terms"));
        self
    }

    pub fn fix(&mut self, text: &str, context: AppCategory) -> Result<QuickFix> {
        let mut edits = Vec::new();
        // Where capitalisation edits are off (chat, notes, code), lower case is the writer's style: an edit that fixes
        // something else keeps the lower-case start of the words it replaces.
        let keep_lower = self.table.policy("capitalisation", context) == Policy::Off;
        for (a, b) in sentence_ranges(text) {
            for mut c in self.fix_sentence(&text[a..b])? {
                if keep_lower {
                    lower_start(&text[a + c.start..a + c.end], &mut c.rep);
                    if c.rep == text[a + c.start..a + c.end] {
                        continue;
                    }
                }
                let policy = self.table.policy(&c.class, context);
                if policy != Policy::Off {
                    edits.push(Edit { start: a + c.start, length: c.end - c.start, replacement: c.rep, layer: c.layer, class: c.class, confidence: c.conf, silent: policy == Policy::Silent });
                }
            }
        }
        let corrected = apply(text, edits.iter().map(|e| (e.start, e.start + e.length, e.replacement.as_str())));
        Ok(QuickFix { corrected, edits })
    }

    fn fix_sentence(&mut self, s: &str) -> Result<Vec<Cand>> {
        if COMMAND.is_match(s) {
            return Ok(Vec::new());
        }
        let protected = self.protected(s);
        let mut base: Vec<Cand> = Vec::new();
        let flagged = self.speller.misspellings(s);
        // Three or more unknown words in one sentence: another language or technical text, not typos.
        if flagged.len() < 3 {
            for (start, end, suggestions) in flagged {
                if let Some(rep) = s.get(start..end).and_then(|word| spelling_fix(word, &suggestions)) {
                    base.push(Cand { start, end, rep, layer: Layer::Spelling, class: "spelling".into(), conf: 1.0 });
                }
            }
        }
        base.extend(rule_edits(s));
        base.retain(|c| c.rep != s[c.start..c.end] && !vetoed(s, &protected, c.start, c.end, &c.rep));
        base.sort_by_key(|c| c.start);
        drop_overlaps(&mut base, |c| (c.start, c.end));
        let t1 = apply(s, base.iter().map(|c| (c.start, c.end, c.rep.as_str())));
        let later = self.grammar_edits(&t1)?;
        Ok(compose(s, &t1, base, later))
    }

    /// The tag and guard layers on one sentence: `(start, end, replacement, confidence)` in bytes of `sentence`.
    pub(crate) fn grammar_edits(&mut self, sentence: &str) -> Result<Vec<(usize, usize, String, f32)>> {
        let protected = self.protected(sentence);
        Ok(self
            .tagger
            .tag_sentence(sentence)?
            .into_iter()
            .map(|e| (e.start, e.start + e.length, e.replacement, e.confidence))
            .filter(|(a, b, rep, _)| !vetoed(sentence, &protected, *a, *b, rep))
            .collect())
    }

    /// Protected spans as character ranges: patterns, plus the personal-dictionary terms.
    fn protected(&self, s: &str) -> Vec<(usize, usize)> {
        let to_char = |b: usize| s[..b].chars().count();
        PROTECT.find_iter(s).chain(self.terms.iter().flat_map(|re| re.find_iter(s))).map(|m| (to_char(m.start()), to_char(m.end()))).collect()
    }
}

/// Sentence byte ranges: split after `.`, `!` or `?` followed by white space; surrounding white space is dropped.
fn sentence_ranges(text: &str) -> Vec<(usize, usize)> {
    let lead = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    if lead >= end {
        return Vec::new();
    }
    let (mut out, mut start, mut prev) = (Vec::new(), lead, None);
    let mut chars = text[lead..end].char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let at = lead + i;
        if c.is_whitespace() && matches!(prev, Some('.' | '!' | '?')) {
            let mut next = at + c.len_utf8();
            while let Some(&(k, d)) = chars.peek().filter(|(_, d)| d.is_whitespace()) {
                next = lead + k + d.len_utf8();
                chars.next();
            }
            out.push((start, at));
            start = next;
        }
        prev = Some(c);
    }
    out.push((start, end));
    out
}

/// The spelling fix for a flagged word, or `None` when it is not plainly a typo: a lower-case word with a vowel that
/// is not in the word list and not a shell command, with exactly one suggestion that is in the word list and one edit
/// away (for a three-letter word only a very common short word), or the same word with its apostrophe put back
/// ("didnt"). Several candidates mean the writer's intent is unclear, so nothing is taken. Names, acronyms and
/// technical terms the word list lacks still get through sometimes, which is why spelling is review-only unless the
/// gate says otherwise.
fn spelling_fix(word: &str, suggestions: &[String]) -> Option<String> {
    let plain = |t: &str| t.chars().all(|c| c.is_ascii_alphabetic() || c == '\'') && t.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    let len = word.chars().count();
    if len < 3 || word.contains('\'') || !plain(word) || !word.chars().any(|c| "aeiouy".contains(c)) || SHELL_WORDS.contains(&word) {
        return None;
    }
    // "didnt": the word list holds the contractions without their apostrophe, so this comes before the list check.
    if let Some(fix) = suggestions.iter().find(|fix| len >= 4 && fix.ends_with("n't") && fix.replace('\'', "") == word) {
        return Some(fix.clone());
    }
    if is_known_word(word) {
        return None;
    }
    let mut candidates = suggestions.iter().filter(|fix| plain(fix) && !fix.contains('\'') && word.strip_suffix('s') != Some(fix.as_str()) && is_known_word(fix) && edit_distance(word, fix) == 1 && (len > 3 || COMMON_SHORT.contains(&fix.as_str())));
    let first = candidates.next()?;
    candidates.next().is_none().then(|| first.clone())
}

/// Gives `rep` a lower-case first letter when the text it replaces starts lower case (the pronoun "I" excepted).
fn lower_start(orig: &str, rep: &mut String) {
    let mut chars = rep.chars();
    let Some(first) = chars.next() else { return };
    let pronoun_i = rep == "I" || rep.starts_with("I ") || rep.starts_with("I'");
    if orig.chars().next().is_some_and(char::is_lowercase) && first.is_uppercase() && !pronoun_i {
        let lowered: String = first.to_lowercase().chain(chars).collect();
        *rep = lowered;
    }
}

/// Optimal string alignment distance: insertions, deletions, substitutions and swapped neighbours cost one each.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 0..=a.len() {
        d[i][0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// KeyReed's rules on typed text: the pronoun "I" and the Indian-English patterns with one safe fix.
fn rule_edits(s: &str) -> Vec<Cand> {
    let mut found: Vec<Cand> = Vec::new();
    let mut add = |re: &Regex, rep: &str, class: &str| {
        for caps in re.captures_iter(s) {
            let m = caps.get(0).expect("whole match");
            let mut text = String::new();
            caps.expand(rep, &mut text);
            found.push(Cand { start: m.start(), end: m.end(), rep: text, layer: Layer::Rule, class: class.into(), conf: 1.0 });
        }
    };
    add(&RE_PRONOUN_IM, "I'$1", "rule_pronoun_i");
    add(&RE_PRONOUN_I, "I", "rule_pronoun_i");
    for (re, rep) in RE_ESL_RULES.iter() {
        add(re, rep, "rule_esl");
    }
    found.sort_by_key(|c| (c.start, std::cmp::Reverse(c.end)));
    drop_overlaps(&mut found, |c| (c.start, c.end));
    found
}

/// Keeps the first of any overlapping ranges (the list is sorted by start).
fn drop_overlaps<T>(list: &mut Vec<T>, range: impl Fn(&T) -> (usize, usize)) {
    let mut end = 0;
    list.retain(|item| {
        let (a, b) = range(item);
        let keep = a >= end;
        if keep {
            end = b.max(a);
        }
        keep
    });
}

/// KeyReed's own checks on one change: inside or right after a protected span, a name changed, a negation added or removed.
fn vetoed(s: &str, protected: &[(usize, usize)], a: usize, b: usize, rep: &str) -> bool {
    let orig = &s[a..b];
    let ac = s[..a].chars().count();
    let bc = ac + orig.chars().count();
    if protected.iter().any(|&(ms, me)| ms < bc.max(ac + 1) && ac <= me + 1) {
        return true;
    }
    let words = |t: &str| WORD.find_iter(t).map(|m| m.as_str().to_lowercase()).collect::<Vec<_>>();
    if ac > 0 && WORD.find_iter(orig).count() == 1 && orig.chars().next().is_some_and(char::is_uppercase) && orig != "I" && rep.to_lowercase() != orig.to_lowercase() {
        return true; // a capitalised word inside a sentence is treated as a name
    }
    let negations = |t: &str| words(t).iter().filter(|w| NEG.contains(&w.as_str()) || w.ends_with("n't")).count();
    negations(orig) != negations(rep)
}

fn apply<'a>(text: &str, edits: impl Iterator<Item = (usize, usize, &'a str)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (a, b, rep) in edits {
        out.push_str(&text[at..a]);
        out.push_str(rep);
        at = b;
    }
    out.push_str(&text[at..]);
    out
}

/// Turns the tagger's edits (on `t1`, the text after the spelling and rule edits `base`) into edits on the original
/// sentence `s`. A tagger edit that overlaps a base edit swallows it: the merged edit is `merged` class.
fn compose(s: &str, t1: &str, base: Vec<Cand>, later: Vec<(usize, usize, String, f32)>) -> Vec<Cand> {
    // Where each base edit sits in t1.
    let mut shift = 0isize;
    let placed: Vec<(usize, usize)> = base
        .iter()
        .map(|c| {
            let out_start = (c.start as isize + shift) as usize;
            shift += c.rep.len() as isize - (c.end - c.start) as isize;
            (out_start, out_start + c.rep.len())
        })
        .collect();
    // A position in t1 that is not inside a base edit's output, back in s.
    let to_s = |p: usize| -> usize {
        let delta: isize = base.iter().zip(&placed).filter(|(_, o)| o.1 <= p).map(|(c, _)| c.rep.len() as isize - (c.end - c.start) as isize).sum();
        (p as isize - delta) as usize
    };
    let mut swallowed = vec![false; base.len()];
    let mut out: Vec<Cand> = Vec::new();
    for (a, b, rep, conf) in later {
        let touched: Vec<usize> = (0..base.len()).filter(|&k| placed[k].0 < b.max(a + 1) && a < placed[k].1 && (a < b || placed[k].0 < a)).collect();
        let (class, layer) = if touched.is_empty() { (classify(t1, a, b, &rep), Layer::Grammar) } else { ("merged".to_string(), Layer::Grammar) };
        let (mut lo, mut hi, mut text) = (a, b, rep);
        if let (Some(&first), Some(&last)) = (touched.first(), touched.last()) {
            lo = a.min(placed[first].0);
            hi = b.max(placed[last].1);
            text = format!("{}{}{}", &t1[lo..a], text, &t1[b..hi]);
            for k in touched {
                swallowed[k] = true;
            }
        }
        out.push(Cand { start: to_s(lo), end: to_s(hi), rep: text, layer, class, conf });
    }
    out.extend(base.into_iter().zip(swallowed).filter(|(_, gone)| !gone).map(|(c, _)| c));
    out.sort_by_key(|c| (c.start, c.end));
    drop_overlaps(&mut out, |c| (c.start, c.end));
    debug_assert!(out.iter().all(|c| c.end <= s.len()));
    out
}

/// The edit class of a change from the tagger, read off the words it touches.
fn classify(text: &str, a: usize, b: usize, rep: &str) -> String {
    let orig = text[a..b].trim();
    let rep = rep.trim();
    let words = |t: &str| WORD.find_iter(t).map(|m| m.as_str().to_lowercase()).collect::<Vec<_>>();
    let (ow, rw) = (words(orig), words(rep));
    let punctuation = |ws: &[String]| !ws.is_empty() && ws.iter().all(|w| !w.chars().any(char::is_alphanumeric));
    let all_in = |ws: &[String], set: &[&str]| !ws.is_empty() && ws.iter().all(|w| set.contains(&w.as_str()));
    let class = if !orig.is_empty() && orig.to_lowercase() == rep.to_lowercase() {
        if rep.chars().count() > 1 && rep == rep.to_uppercase() {
            "capitalisation_upper"
        } else {
            "capitalisation"
        }
    } else if rep.is_empty() {
        let prev = words(&text[..a]).pop();
        let next = words(&text[b..]).into_iter().next();
        if ow.len() == 1 && (prev.as_ref() == ow.first() || next.as_ref() == ow.first()) {
            "doubled_word"
        } else if all_in(&ow, ARTICLES) {
            "article"
        } else if punctuation(&ow) {
            "punctuation"
        } else {
            "deletion"
        }
    } else if orig.is_empty() {
        if all_in(&rw, ARTICLES) {
            "article"
        } else if punctuation(&rw) {
            "punctuation"
        } else {
            "insertion"
        }
    } else if ow.len() == 1 && rw.len() == 1 && all_in(&ow, AGREE) && all_in(&rw, AGREE) {
        "agreement"
    } else if ow.len() == 1 && rw.len() == 1 && all_in(&ow, CONFUSABLE) && all_in(&rw, CONFUSABLE) {
        "confusable"
    } else if ow.len() == 1 && rw.len() == 1 && all_in(&ow, ARTICLES) && all_in(&rw, ARTICLES) {
        "article"
    } else if ow.len() == 1 && rw.len() == 1 && (ow[0].starts_with(&rw[0]) || rw[0].starts_with(&ow[0])) {
        let cue = words(&text[..a]).pop();
        if cue.is_some_and(|c| INFLECTION_CUES.contains(&c.as_str()) || (!c.is_empty() && c.chars().all(|d| d.is_ascii_digit()))) {
            "inflection_cued"
        } else {
            "inflection"
        }
    } else if punctuation(&ow) && punctuation(&rw) {
        "punctuation"
    } else {
        "replacement"
    };
    class.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gector::{test_model_dir, test_ort_dll, Gector, RawEdit};

    /// Returns what it is told, for sentences that contain the needle ("" matches all).
    struct Script(Vec<RawEdit>, &'static str);
    impl SentenceTagger for Script {
        fn tag_sentence(&mut self, s: &str) -> Result<Vec<RawEdit>> {
            Ok(if s.contains(self.1) { self.0.clone() } else { Vec::new() })
        }
    }

    fn edit(start: usize, length: usize, rep: &str) -> RawEdit {
        RawEdit { start, length, replacement: rep.into(), confidence: 0.9 }
    }

    fn engine(edits: Vec<RawEdit>) -> QuickFixEngine {
        QuickFixEngine::new(Box::new(Script(edits, ""))).with_table(EditClassTable::parse(r#"{"contexts":{},"classes":{}}"#).unwrap())
    }

    #[test]
    fn sentences_split_after_a_stop_and_white_space() {
        let t = "  Hello there. How are you?  Fine! v1.2 is out.\n";
        let parts: Vec<&str> = sentence_ranges(t).into_iter().map(|(a, b)| &t[a..b]).collect();
        assert_eq!(parts, ["Hello there.", "How are you?", "Fine!", "v1.2 is out."]);
        assert!(sentence_ranges("   ").is_empty());
    }

    #[test]
    fn grammar_edit_is_reported_with_text_offsets_and_class() {
        let mut e = QuickFixEngine::new(Box::new(Script(vec![edit(5, 3, "is")], "This")));
        let r = e.fix("Hi. This are fine.", AppCategory::General).unwrap();
        assert_eq!(r.edits.len(), 1);
        let ed = &r.edits[0];
        assert_eq!((ed.start, ed.length, ed.replacement.as_str(), ed.layer), (3 + 1 + 5, 3, "is", Layer::Grammar));
        assert_eq!(ed.class, "agreement");
        assert!(!ed.silent, "built-in table is review-only until the gate says otherwise");
        assert_eq!(r.corrected, "Hi. This is fine.");
    }

    #[test]
    fn protected_tokens_names_and_negations_are_never_changed() {
        // "C:\a\b.txt" protected; "Karthik" is a name; "not" removal flips a negation; an edit right after a number.
        for (text, ed) in [
            ("Open C:\\a\\b.txt now.", edit(5, 10, "c:\\a\\b.TXT")),
            ("Ask Karthik today.", edit(4, 7, "Karthick")),
            ("It is not done.", edit(6, 4, "")),
            ("Wait 10 minutes.", edit(8, 7, "minute")),
            ("See www.example.com now", edit(4, 15, "www.example.org")),
        ] {
            let r = engine(vec![ed]).fix(text, AppCategory::General).unwrap();
            assert!(r.edits.is_empty(), "{text}: {:?}", r.edits);
        }
    }

    #[test]
    fn dictionary_terms_and_command_sentences_are_left_alone() {
        let mut e = engine(vec![edit(0, 4, "Kubernetes")]).with_terms(&["kube".to_string()]);
        assert!(e.fix("kube is here", AppCategory::General).unwrap().edits.is_empty());
        let mut e = engine(vec![edit(0, 3, "Run")]);
        assert!(e.fix("git commit --amend now", AppCategory::General).unwrap().edits.is_empty());
    }

    #[test]
    fn rules_fix_i_and_esl_patterns_and_the_tagger_sees_the_result() {
        let mut e = engine(vec![]);
        let r = e.fix("i think he don't know.", AppCategory::General).unwrap();
        assert_eq!(r.corrected, "I think he doesn't know.");
        assert!(r.edits.iter().all(|x| x.layer == Layer::Rule));
        // The tagger edits the text the rules produced, and its edit is reported against the original.
        let mut e = engine(vec![edit(2, 5, "believe")]); // "I think he doesn't know." -> t1 offset 2 is "think"
        let r = e.fix("i think he don't know.", AppCategory::General).unwrap();
        assert_eq!(r.corrected, "I believe he doesn't know.");
    }

    /// Suggests whatever it is told for any word that appears in its list.
    struct Fixed(Vec<(&'static str, Vec<&'static str>)>);
    impl Speller for Fixed {
        fn misspellings(&mut self, text: &str) -> Vec<(usize, usize, Vec<String>)> {
            self.0.iter().filter_map(|(w, fixes)| text.find(w).map(|a| (a, a + w.len(), fixes.iter().map(|f| f.to_string()).collect()))).collect()
        }
    }

    #[test]
    fn spelling_layer_fixes_typos_but_not_names_or_unclear_ones() {
        let speller = Fixed(vec![("recieve", vec!["receive"]), ("Karthik", vec!["Karthick"])]);
        let mut e = engine(vec![]).with_speller(Box::new(speller));
        let r = e.fix("I recieve mail from Karthik today.", AppCategory::General).unwrap();
        assert_eq!(r.corrected, "I receive mail from Karthik today.");
        assert_eq!((r.edits[0].layer, r.edits[0].class.as_str()), (Layer::Spelling, "spelling"));
        let one = |word: &str, fixes: &[&str]| spelling_fix(word, &fixes.iter().map(|f| f.to_string()).collect::<Vec<_>>());
        assert_eq!(one("teh", &["the", "tea"]).as_deref(), Some("the"));
        assert_eq!(one("sening", &["sensing", "sending"]), None, "two candidates: intent unclear");
        assert_eq!(one("usb", &["sub"]), None);
        assert_eq!(one("src", &["sac"]), None, "no vowel: an abbreviation");
        assert_eq!(one("behaviour", &["behavior"]), None, "in the word list");
        assert_eq!(one("teh", &["a lot"]), None);
        assert_eq!(one("didnt", &["didn't"]).as_deref(), Some("didn't"));
        assert_eq!(one("informations", &["information's"]), None, "only contractions get an apostrophe");
        assert_eq!(one("sudo", &["sudor"]), None);
        assert_eq!(one("dont", &["don't"]).as_deref(), Some("don't"));
        assert_eq!(one("i'll", &["ill"]), None, "dropping an apostrophe is not a typo fix");
        assert_eq!(one("companys", &["company"]), None, "a dropped plural or possessive s is not a typo fix");
        // Three unknown words in one sentence: not typos.
        let mut e = engine(vec![]).with_speller(Box::new(Fixed(vec![("recieve", vec!["receive"]), ("hte", vec!["the"]), ("adn", vec!["and"])])));
        assert!(e.fix("recieve hte adn", AppCategory::General).unwrap().edits.is_empty());
        let mut e = engine(vec![]).with_speller(Box::new(Fixed(vec![("recieve", vec!["receive"]), ("hte", vec!["the"])])));
        assert_eq!(e.fix("recieve hte", AppCategory::General).unwrap().corrected, "receive the");
    }

    #[test]
    fn edit_distance_counts_a_swap_as_one() {
        assert_eq!(edit_distance("teh", "the"), 1);
        assert_eq!(edit_distance("recieve", "receive"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn lower_case_style_is_kept_where_capitalisation_is_off() {
        // "your" -> "You're": in chat the fix keeps the writer's lower case; in e-mail it stands as the model made it.
        let one = |ctx| engine(vec![edit(0, 4, "You're")]).fix("your going to love it", ctx).unwrap().corrected;
        let strict = EditClassTable::parse(r#"{"contexts":{},"classes":{"capitalisation":{"policy":"off"}}}"#).unwrap();
        let chat = engine(vec![edit(0, 4, "You're")]).with_table(strict).fix("your going to love it", AppCategory::Chat).unwrap();
        assert_eq!(chat.corrected, "you're going to love it");
        assert_eq!(one(AppCategory::Email), "You're going to love it");
        let mut s = String::from("I'm");
        lower_start("i'm", &mut s);
        assert_eq!(s, "I'm", "the pronoun I stays");
    }

    #[test]
    fn a_tagger_edit_over_a_rule_edit_swallows_it() {
        // t1 = "I am here"; tagger rewrites "I am" -> "We are", which covers the rule's "i" -> "I".
        let mut e = engine(vec![edit(0, 4, "We are")]);
        let r = e.fix("i am here", AppCategory::General).unwrap();
        assert_eq!(r.edits.len(), 1);
        assert_eq!((r.edits[0].class.as_str(), r.edits[0].silent), ("merged", false));
        assert_eq!(r.corrected, "We are here");
    }

    #[test]
    fn class_table_caps_by_context_and_unknown_classes_are_review_only() {
        let t = EditClassTable::parse(
            r#"{"unknown_class":"review_only","contexts":{"terminal":"off","code_editor":"review_only"},
                "classes":{"spelling":{"policy":"silent"},"article":{"policy":"off"}}}"#,
        )
        .unwrap();
        assert_eq!(t.policy("spelling", AppCategory::Email), Policy::Silent);
        assert_eq!(t.policy("spelling", AppCategory::CodeEditor), Policy::ReviewOnly);
        assert_eq!(t.policy("spelling", AppCategory::Terminal), Policy::Off);
        let t2 = EditClassTable::parse(r#"{"contexts":{},"classes":{"capitalisation":{"policy":"review_only","contexts":{"chat":"off","email":"silent"}}}}"#).unwrap();
        assert_eq!((t2.policy("capitalisation", AppCategory::Chat), t2.policy("capitalisation", AppCategory::Email), t2.policy("capitalisation", AppCategory::Notes)), (Policy::Off, Policy::Silent, Policy::ReviewOnly));
        assert_eq!(t.policy("article", AppCategory::Email), Policy::Off);
        assert_eq!(t.policy("whatever", AppCategory::Email), Policy::ReviewOnly);
        assert!(EditClassTable::parse(r#"{"classes":{"a":{"policy":"loud"}},"contexts":{}}"#).is_err());
        let b = EditClassTable::builtin();
        assert_eq!(b.policy("agreement", AppCategory::General), Policy::ReviewOnly);
        assert_eq!((b.policy("doubled_word", AppCategory::Email), b.policy("capitalisation", AppCategory::Chat), b.policy("capitalisation", AppCategory::Email)), (Policy::Silent, Policy::Off, Policy::Silent));
        assert_eq!((b.policy("doubled_word", AppCategory::Terminal), b.policy("doubled_word", AppCategory::CodeEditor)), (Policy::Off, Policy::ReviewOnly));
    }

    #[test]
    fn classes_are_read_off_the_words() {
        let c = |text: &str, a: usize, b: usize, rep: &str| classify(text, a, b, rep);
        assert_eq!(c("This are it", 5, 8, "is"), "agreement");
        assert_eq!(c("their going", 0, 5, "they're"), "confusable");
        assert_eq!(c("the the cat", 4, 8, ""), "doubled_word");
        assert_eq!(c("I have book", 7, 7, "a "), "article");
        assert_eq!(c("hello world", 0, 5, "Hello"), "capitalisation");
        assert_eq!(c("im here", 0, 2, "IM"), "capitalisation_upper");
        assert_eq!(c("a cat", 1, 1, ","), "punctuation");
        assert_eq!(c("two cat", 4, 7, "cats"), "inflection_cued");
        assert_eq!(c("he go home", 3, 5, "goes"), "inflection_cued");
        assert_eq!(c("the cat sat", 4, 7, "cats"), "inflection");
    }

    /// The Python reference and this engine make the same edits on every sentence of tests/data/quickfix_reference.json
    /// (made by tests/data/make_fixtures.py from the Python reference with the same model). `KEYREED_QF_REFERENCE` names
    /// another fixture file, for a model other than the shipped one.
    #[test]
    fn grammar_edits_equal_the_python_reference() {
        let Some(dir) = test_model_dir() else { return };
        let mut e = QuickFixEngine::new(Box::new(Gector::load(&test_ort_dll(), &dir, 2).unwrap()));
        let raw = match std::env::var_os("KEYREED_QF_REFERENCE") {
            Some(p) => std::fs::read_to_string(p).unwrap(),
            None => include_str!("../tests/data/quickfix_reference.json").to_string(),
        };
        let fixture: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let sentences = fixture["sentences"].as_array().unwrap();
        assert!(sentences.len() >= 200);
        let edits_of = |v: &serde_json::Value| -> Vec<(usize, usize, String)> { v.as_array().unwrap().iter().map(|x| (x[0].as_u64().unwrap() as usize, x[1].as_u64().unwrap() as usize, x[2].as_str().unwrap().to_string())).collect() };
        // Borderline labels that the shipped ONNX Runtime and the reference's give differently (see the file).
        let drift: serde_json::Value = serde_json::from_str(include_str!("../tests/data/quickfix_runtime_drift.json")).unwrap();
        let drift: HashMap<&str, Vec<(usize, usize, String)>> = drift["cases"].as_array().unwrap().iter().map(|c| (c["input"].as_str().unwrap(), edits_of(&c["edits"]))).collect();
        let (mut with_edits, mut differ, mut drifted) = (0, Vec::new(), 0);
        for case in sentences {
            let s = case["input"].as_str().unwrap();
            let mut want = edits_of(&case["edits"]);
            if std::env::var_os("KEYREED_QF_REFERENCE").is_none() {
                if let Some(here) = drift.get(s) {
                    assert_ne!(*here, want, "stale drift entry: {s}");
                    want = here.clone();
                    drifted += 1;
                }
            }
            let got: Vec<(usize, usize, String)> = if COMMAND.is_match(s) {
                Vec::new()
            } else {
                let chars = |b: usize| s[..b].chars().count();
                e.grammar_edits(s).unwrap().into_iter().map(|(a, b, rep, _)| (chars(a), chars(b) - chars(a), rep)).collect()
            };
            with_edits += !want.is_empty() as usize;
            if got != want {
                differ.push(format!("{s}
  reference {want:?}
  here      {got:?}"));
            }
        }
        assert!(with_edits >= 50, "the fixture should exercise real edits");
        assert!(std::env::var_os("KEYREED_QF_REFERENCE").is_some() || drifted == drift.len(), "a drift entry is not in the fixture");
        assert!(differ.is_empty(), "{} of {} sentences differ:
{}", differ.len(), sentences.len(), differ.join("
"));
    }
}
