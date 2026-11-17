//! Writing-habit analysers (plan P4.4): deterministic statistics from text the user wrote, and counters for the
//! suggestions they accept or reject. Nothing but counts leaves this module: the text is read once, never stored,
//! logged or returned, and `StyleStore` only ever receives (habit, value, count) triples.
//!
//! Habits (keys) and values, observed per sentence unless noted:
//! - `dialect`: `british` / `american` (spelling markers; a sentence that mixes both says nothing)
//! - `contractions`: `uses` / `avoids` (a sentence with both says nothing)
//! - `sentence_length`: `short` (up to 12 words), `medium`, `long` (23 words or more)
//! - `serial_comma`: `yes` / `no` (lists of three single words: "a, b, and c" or "a, b and c")
//! - `dash_style`: `em_dash` / `spaced_en_dash` / `spaced_hyphen` (a sentence using two styles says nothing)
//! - `emoji`: `uses` / `none`
//! - `final_full_stop`, once per text: `yes` / `no` for a single-line, one-sentence message of at most 12 words
//!   that ends in a letter, digit or full stop (a question mark, exclamation mark, ellipsis or emoji says nothing)
//! URLs, e-mail addresses, paths and `code` spans are removed before any counting.

use crate::style::{LearnState, Scope, Source, StyleStore};
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

pub const SHORT_MESSAGE_WORDS: usize = 12;
const SHORT_SENTENCE_WORDS: usize = 12;
const LONG_SENTENCE_WORDS: usize = 23;

/// `n` sentences (or messages) that showed `value` for habit `key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Observation {
    pub key: &'static str,
    pub value: &'static str,
    pub n: u32,
}

/// (British, American) spellings of words whose spelling differs by dialect and has no second common meaning.
const SPELLINGS: &[(&str, &str)] = &[
    ("colour", "color"), ("colours", "colors"), ("coloured", "colored"), ("favour", "favor"), ("favours", "favors"),
    ("favourite", "favorite"), ("favourites", "favorites"), ("honour", "honor"), ("behaviour", "behavior"),
    ("neighbour", "neighbor"), ("neighbours", "neighbors"), ("labour", "labor"), ("humour", "humor"),
    ("flavour", "flavor"), ("organise", "organize"), ("organised", "organized"), ("organising", "organizing"),
    ("organisation", "organization"), ("organisations", "organizations"), ("realise", "realize"),
    ("realised", "realized"), ("recognise", "recognize"), ("recognised", "recognized"), ("apologise", "apologize"),
    ("summarise", "summarize"), ("prioritise", "prioritize"), ("minimise", "minimize"), ("optimise", "optimize"),
    ("optimised", "optimized"), ("analyse", "analyze"), ("analysed", "analyzed"), ("centre", "center"),
    ("centres", "centers"), ("theatre", "theater"), ("litre", "liter"), ("fibre", "fiber"),
    ("travelling", "traveling"), ("travelled", "traveled"), ("traveller", "traveler"), ("cancelled", "canceled"),
    ("cancelling", "canceling"), ("modelling", "modeling"), ("labelled", "labeled"), ("catalogue", "catalog"),
    ("grey", "gray"), ("defence", "defense"), ("offence", "offense"), ("aluminium", "aluminum"), ("mum", "mom"),
    ("jewellery", "jewelry"), ("ageing", "aging"), ("enrol", "enroll"), ("fulfil", "fulfill"),
    ("instalment", "installment"), ("sceptical", "skeptical"), ("authorise", "authorize"),
    ("authorised", "authorized"), ("customise", "customize"), ("customised", "customized"),
    ("standardise", "standardize"), ("emphasise", "emphasize"), ("utilise", "utilize"), ("licence", "license"),
];

const CONTRACTIONS: &[&str] = &[
    "i'm", "i've", "i'll", "i'd", "you're", "you've", "you'll", "you'd", "he's", "she's", "it's", "that's",
    "there's", "what's", "let's", "we're", "we've", "we'll", "we'd", "they're", "they've", "they'll", "they'd",
    "who's", "how's", "here's", "where's",
];

/// Word pairs that a contraction would have shortened.
const EXPANDED: &[(&str, &str)] = &[
    ("do", "not"), ("does", "not"), ("did", "not"), ("is", "not"), ("are", "not"), ("was", "not"), ("were", "not"),
    ("have", "not"), ("has", "not"), ("had", "not"), ("will", "not"), ("would", "not"), ("should", "not"),
    ("could", "not"), ("i", "am"), ("i", "have"), ("i", "will"), ("i", "would"), ("i", "had"), ("you", "are"),
    ("you", "have"), ("you", "will"), ("we", "are"), ("we", "have"), ("we", "will"), ("they", "are"),
    ("they", "have"), ("they", "will"), ("it", "is"), ("that", "is"), ("there", "is"), ("what", "is"),
    ("he", "is"), ("she", "is"), ("let", "us"),
];

const ABBREVIATIONS: &[&str] = &["mr.", "mrs.", "ms.", "dr.", "prof.", "e.g.", "i.e.", "vs.", "st.", "fig."];

const EMOTICONS: &[&str] = &[":)", ":-)", ":(", ":-(", ";)", ";-)", ":D", ":P", ":p", "xD", "XD"];

pub(crate) fn dialect_words() -> &'static (HashSet<&'static str>, HashSet<&'static str>) {
    static SETS: OnceLock<(HashSet<&'static str>, HashSet<&'static str>)> = OnceLock::new();
    SETS.get_or_init(|| (SPELLINGS.iter().map(|p| p.0).collect(), SPELLINGS.iter().map(|p| p.1).collect()))
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1F2FF | 0x1F300..=0x1FAFF | 0x2600..=0x26FF | 0x2705 | 0x2728 | 0x274C | 0x2764 | 0x2B50)
}

/// Removes `code` spans and, token by token, URLs, e-mail addresses and paths; keeps line breaks. A removed token's
/// closing punctuation stays, so a sentence that ended in a link still ends.
fn strip_protected(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let prose: String = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
        let mut kept = Vec::new();
        for token in prose.split_whitespace() {
            let protected = token.contains("://")
                || token.starts_with("www.")
                || (token.contains('@') && token.contains('.'))
                || token.contains('\\')
                || token.starts_with('/')
                || token.starts_with("~/");
            if protected {
                let tail: String = token.chars().rev().take_while(|c| matches!(c, '.' | '!' | '?')).collect();
                if !tail.is_empty() {
                    kept.push(tail); // all one kind of mark, so the reversed order is the same
                }
            } else {
                kept.push(token.to_string());
            }
        }
        lines.push(kept.join(" "));
    }
    lines.join("\n")
}

fn is_abbreviation(segment: &str) -> bool {
    let word = segment.rsplit(char::is_whitespace).next().unwrap_or("");
    let lower = word.to_lowercase();
    if ABBREVIATIONS.contains(&lower.as_str()) {
        return true;
    }
    let mut chars = word.chars();
    matches!((chars.next(), chars.next(), chars.next()), (Some(c), Some('.'), None) if c.is_uppercase() && c != 'I')
}

fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut push = |s| {
        let t = str::trim(s);
        if t.chars().any(char::is_alphanumeric) {
            out.push(t);
        }
    };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let (mut start, mut i) = (0, 0);
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c == '\n' {
            push(&text[start..pos]);
            start = pos + 1;
            i += 1;
        } else if matches!(c, '.' | '!' | '?') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?' | '"' | '\'' | ')' | ']' | '\u{201D}' | '\u{2019}') {
                j += 1;
            }
            let end = chars.get(j).map_or(text.len(), |p| p.0);
            let boundary = j >= chars.len() || chars[j].1.is_whitespace();
            if boundary && !(c == '.' && j == i + 1 && is_abbreviation(&text[start..pos + 1])) {
                push(&text[start..end]);
                start = end;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    push(&text[start..]);
    out
}

/// Lower-case words with their inner apostrophes ("don't"); digits count as words.
fn words(sentence: &str) -> Vec<String> {
    sentence
        .replace('\u{2019}', "'")
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\'').to_lowercase())
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .collect()
}

fn dialect(words: &[String]) -> Option<&'static str> {
    let (british, american) = dialect_words();
    let b = words.iter().any(|w| british.contains(w.as_str()));
    let a = words.iter().any(|w| american.contains(w.as_str()));
    match (b, a) {
        (true, false) => Some("british"),
        (false, true) => Some("american"),
        _ => None,
    }
}

fn contractions(words: &[String]) -> Option<&'static str> {
    let used = words.iter().any(|w| (w.ends_with("n't") && w.len() > 3) || CONTRACTIONS.contains(&w.as_str()));
    let expanded = words.iter().any(|w| w == "cannot")
        || words.windows(2).any(|p| EXPANDED.iter().any(|(a, b)| p[0] == *a && p[1] == *b));
    match (used, expanded) {
        (true, false) => Some("uses"),
        (false, true) => Some("avoids"),
        _ => None,
    }
}

fn sentence_length(word_count: usize) -> &'static str {
    match word_count {
        0..=SHORT_SENTENCE_WORDS => "short",
        n if n < LONG_SENTENCE_WORDS => "medium",
        _ => "long",
    }
}

fn serial_comma(sentence: &str) -> Option<&'static str> {
    let item = |t: &str| {
        let w = t.trim_end_matches(',');
        !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '-')
    };
    let tokens: Vec<&str> = sentence.split_whitespace().collect();
    for k in 2..tokens.len().saturating_sub(1) {
        if !matches!(tokens[k], "and" | "or") || !tokens[k - 2].ends_with(',') || !item(tokens[k - 2]) || !item(tokens[k - 1]) {
            continue;
        }
        if tokens[k - 1].ends_with(',') {
            return Some("yes");
        }
        if item(tokens[k + 1].trim_end_matches(|c: char| matches!(c, '.' | '!' | '?' | ';' | ':'))) {
            return Some("no");
        }
    }
    None
}

fn dash_style(sentence: &str) -> Option<&'static str> {
    let chars: Vec<char> = sentence.chars().collect();
    let spaced_hyphen = chars
        .windows(5)
        .any(|w| w[0].is_alphabetic() && w[1] == ' ' && w[2] == '-' && w[3] == ' ' && w[4].is_alphabetic());
    let found: Vec<&'static str> = [
        (sentence.contains('\u{2014}'), "em_dash"),
        (sentence.contains(" \u{2013} "), "spaced_en_dash"),
        (spaced_hyphen, "spaced_hyphen"),
    ]
    .into_iter()
    .filter_map(|(hit, name)| hit.then_some(name))
    .collect();
    if found.len() == 1 { Some(found[0]) } else { None }
}

fn has_emoji(sentence: &str) -> bool {
    sentence.chars().any(is_emoji) || sentence.split_whitespace().any(|t| EMOTICONS.contains(&t))
}

fn final_full_stop(text: &str, sentences: &[&str]) -> Option<&'static str> {
    let t = text.trim();
    if t.contains('\n') || sentences.len() != 1 {
        return None;
    }
    let word_count = words(t).len();
    if word_count == 0 || word_count > SHORT_MESSAGE_WORDS {
        return None;
    }
    let t = t.trim_end_matches(|c: char| matches!(c, '"' | '\'' | ')' | ']' | '\u{201D}' | '\u{2019}'));
    if t.ends_with("..") || t.ends_with('\u{2026}') {
        return None;
    }
    match t.chars().last()? {
        '.' => Some("yes"),
        c if c.is_alphanumeric() => Some("no"),
        _ => None,
    }
}

/// The habit observations in `text`, counted and sorted. The text is not kept.
pub fn analyse(text: &str) -> Vec<Observation> {
    let clean = strip_protected(text);
    let sentences = split_sentences(&clean);
    let mut counts: BTreeMap<(&'static str, &'static str), u32> = BTreeMap::new();
    let mut add = |key, value| *counts.entry((key, value)).or_insert(0) += 1;
    for sentence in &sentences {
        let w = words(sentence);
        if w.is_empty() {
            continue;
        }
        add("sentence_length", sentence_length(w.len()));
        add("emoji", if has_emoji(sentence) { "uses" } else { "none" });
        for (key, value) in [
            ("dialect", dialect(&w)),
            ("contractions", contractions(&w)),
            ("serial_comma", serial_comma(sentence)),
            ("dash_style", dash_style(sentence)),
        ] {
            if let Some(value) = value {
                add(key, value);
            }
        }
    }
    if let Some(value) = final_full_stop(&clean, &sentences) {
        add("final_full_stop", value);
    }
    counts.into_iter().map(|((key, value), n)| Observation { key, value, n }).collect()
}

/// Analyses `text` and records the counts in `store` for `scope` (and everywhere). Returns how many observations
/// were recorded; 0 when the learning switch for `source` is not on. The text is dropped when this returns.
pub fn learn_from_text(store: &mut StyleStore, source: Source, scope: Scope, text: &str, now: u64) -> usize {
    analyse(text)
        .into_iter()
        .filter(|o| store.observe(source, o.key, scope, o.value, o.n, now))
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accepted,
    Rejected,
}

impl Decision {
    fn name(self) -> &'static str {
        match self {
            Decision::Accepted => "accepted",
            Decision::Rejected => "rejected",
        }
    }
}

/// `kind` names what was offered, from a fixed vocabulary of the caller's such as `preset:fix` or `edit:spelling`:
/// letters, digits, `_` and `:` only, so no text can end up in a counter.
fn valid_kind(kind: &str) -> bool {
    !kind.is_empty() && kind.len() <= 40 && kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// Counts a suggestion the user accepted or rejected in the Review panel or the Composer. Choices are about drafts,
/// so the drafts switch governs them. Returns whether it was counted.
pub fn record_decision(store: &mut StyleStore, kind: &str, decision: Decision) -> bool {
    if store.learn_from_drafts != LearnState::On || !valid_kind(kind) {
        return false;
    }
    store.bump_chip(&format!("{kind}:{}", decision.name()));
    true
}

/// (accepted, rejected) so far for `kind`.
pub fn decision_counts(store: &StyleStore, kind: &str) -> (u32, u32) {
    let get = |d: Decision| store.chip_counts.get(&format!("{kind}:{}", d.name())).copied().unwrap_or(0);
    (get(Decision::Accepted), get(Decision::Rejected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(text: &str, key: &str) -> Vec<(&'static str, u32)> {
        analyse(text).into_iter().filter(|o| o.key == key).map(|o| (o.value, o.n)).collect()
    }

    #[test]
    fn dialect_counts_sentences_with_one_dialect_only() {
        let british = "I organise the colour samples. The centre is closed. We met at noon.";
        assert_eq!(obs(british, "dialect"), vec![("british", 2)]);
        assert_eq!(obs("My favorite color is gray. Please analyze it.", "dialect"), vec![("american", 2)]);
        assert_eq!(obs("The colour and the color differ.", "dialect"), vec![], "a mixed sentence says nothing");
        assert_eq!(obs("Nothing to see here.", "dialect"), vec![]);
        assert_eq!(obs("Visit https://colour.example.com/colour now.", "dialect"), vec![], "links are ignored");
        assert_eq!(obs("Rename `colour` to `color` today.", "dialect"), vec![], "code spans are ignored");
    }

    #[test]
    fn contractions_are_told_from_possessives_and_expanded_forms() {
        assert_eq!(obs("I don't know. It's fine. We'll see.", "contractions"), vec![("uses", 3)]);
        assert_eq!(obs("I do not know. It is fine. We cannot go.", "contractions"), vec![("avoids", 3)]);
        assert_eq!(obs("John's car is red. The dog's bowl is empty.", "contractions"), vec![], "possessives are not contractions");
        assert_eq!(obs("It's fine, but I do not like it.", "contractions"), vec![], "a mixed sentence says nothing");
        assert_eq!(obs("She didn\u{2019}t come.", "contractions"), vec![("uses", 1)], "curly apostrophes count");
    }

    #[test]
    fn sentence_length_buckets() {
        let twelve = "one two three four five six seven eight nine ten eleven twelve";
        let thirteen = format!("{twelve} thirteen");
        let twenty_two = format!("{twelve} a b c d e f g h i j");
        let twenty_three = format!("{twenty_two} k");
        let text = format!("Short one. {twelve}. {thirteen}. {twenty_two}. {twenty_three}.");
        assert_eq!(obs(&text, "sentence_length"), vec![("long", 1), ("medium", 2), ("short", 2)]);
    }

    #[test]
    fn serial_comma_is_detected_in_simple_lists() {
        assert_eq!(obs("We bought apples, oranges, and pears.", "serial_comma"), vec![("yes", 1)]);
        assert_eq!(obs("We bought apples, oranges and pears.", "serial_comma"), vec![("no", 1)]);
        assert_eq!(obs("Pick red or blue, green, or black.", "serial_comma"), vec![("yes", 1)]);
        assert_eq!(obs("Yesterday, John and Mary left.", "serial_comma"), vec![], "a capitalised opener is not a list");
        assert_eq!(obs("We met Tom and Jerry.", "serial_comma"), vec![]);
    }

    #[test]
    fn dash_styles_and_the_one_style_rule() {
        assert_eq!(obs("It works\u{2014}mostly.", "dash_style"), vec![("em_dash", 1)]);
        assert_eq!(obs("It works \u{2013} mostly.", "dash_style"), vec![("spaced_en_dash", 1)]);
        assert_eq!(obs("It works - mostly.", "dash_style"), vec![("spaced_hyphen", 1)]);
        assert_eq!(obs("Pages 3\u{2013}5 and a well-known fix.", "dash_style"), vec![], "ranges and compounds are not dashes");
        assert_eq!(obs("A - 3 - B items.", "dash_style"), vec![], "a hyphen needs letters on both sides");
        assert_eq!(obs("One\u{2014}two \u{2013} three.", "dash_style"), vec![], "two styles in one sentence say nothing");
    }

    #[test]
    fn emoji_and_emoticons() {
        assert_eq!(obs("Great news \u{1F600}. See you soon.", "emoji"), vec![("none", 1), ("uses", 1)]);
        assert_eq!(obs("Thanks :) see you.", "emoji"), vec![("uses", 1)]);
        assert_eq!(obs("A heart \u{2764}\u{FE0F} here.", "emoji"), vec![("uses", 1)]);
        assert_eq!(obs("Price is 5 \u{20AC} \u{00A9} 2026.", "emoji"), vec![("none", 1)], "currency and copyright signs are not emoji");
    }

    #[test]
    fn final_full_stop_only_for_short_single_line_messages() {
        assert_eq!(obs("See you at six.", "final_full_stop"), vec![("yes", 1)]);
        assert_eq!(obs("See you at six", "final_full_stop"), vec![("no", 1)]);
        assert_eq!(obs("See you at six!", "final_full_stop"), vec![]);
        assert_eq!(obs("Are you coming?", "final_full_stop"), vec![]);
        assert_eq!(obs("Hmm, maybe...", "final_full_stop"), vec![]);
        assert_eq!(obs("On my way \u{1F44D}", "final_full_stop"), vec![]);
        assert_eq!(obs("He said \"see you at six.\"", "final_full_stop"), vec![("yes", 1)]);
        assert_eq!(obs("ok. thanks", "final_full_stop"), vec![], "two sentences");
        assert_eq!(obs("ok\nthanks", "final_full_stop"), vec![], "two lines");
        assert_eq!(obs("one two three four five six seven eight nine ten eleven twelve thirteen.", "final_full_stop"), vec![]);
        assert_eq!(obs("Details are at https://example.com/a/b.", "final_full_stop"), vec![("yes", 1)], "a link does not hide the full stop");
        assert_eq!(obs("", "final_full_stop"), vec![]);
    }

    #[test]
    fn sentence_splitting_respects_abbreviations_decimals_and_lines() {
        let n = |t: &str| obs(t, "sentence_length").iter().map(|o| o.1).sum::<u32>();
        assert_eq!(n("Dr. Rao met Mr. Iyer. They left."), 2);
        assert_eq!(n("Use e.g. a spoon. Then stir."), 2);
        assert_eq!(n("Version 4.2.1 is out. Update now!"), 2);
        assert_eq!(n("J. Smith wrote it. Really?"), 2);
        assert_eq!(n("First line\nSecond line\n\nThird line."), 3);
        assert_eq!(n("Wait... what? OK."), 3);
        assert_eq!(n("   \n  "), 0);
        assert!(analyse("   ").is_empty());
    }

    #[test]
    fn learning_records_counts_for_the_scope_and_everywhere() {
        let mut store = StyleStore::default();
        let n = learn_from_text(&mut store, Source::Drafts, Scope::Email, "I organise the files. It's done.", 50);
        assert!(n >= 4, "sentence length, emoji, dialect and contractions were recorded, got {n}");
        let find = |scope, key: &str, value: &str| store.items.iter().find(|i| i.scope == scope && i.key == key && i.value == value).map(|i| (i.seen, i.agree));
        assert_eq!(find(Scope::Email, "dialect", "british"), Some((1, 1)));
        assert_eq!(find(Scope::Everywhere, "dialect", "british"), Some((1, 1)));
        assert_eq!(find(Scope::Email, "contractions", "uses"), Some((1, 1)));
        assert_eq!(find(Scope::Chat, "dialect", "british"), None);
    }

    #[test]
    fn a_paused_or_off_switch_records_nothing_but_samples_always_do() {
        let mut store = StyleStore::default();
        store.learn_from_drafts = LearnState::Paused;
        assert_eq!(learn_from_text(&mut store, Source::Drafts, Scope::Chat, "I organise it.", 1), 0);
        assert!(store.items.is_empty());
        store.learn_from_drafts = LearnState::Off;
        assert_eq!(learn_from_text(&mut store, Source::Drafts, Scope::Chat, "I organise it.", 1), 0);
        assert!(learn_from_text(&mut store, Source::Samples, Scope::Chat, "I organise it.", 1) > 0);
    }

    #[test]
    fn no_text_is_retained() {
        let mut store = StyleStore::default();
        let secret = "Quarterly zeppelin budget for Zanzibar: I organise the colour plan, and we don't mention hunter2.";
        learn_from_text(&mut store, Source::Samples, Scope::Documents, secret, 9);
        record_decision(&mut store, "preset:fix", Decision::Accepted);
        let json = serde_json::to_string(&store).unwrap();
        for word in ["zeppelin", "Zanzibar", "hunter2", "budget", "Quarterly", "colour plan"] {
            assert!(!json.contains(word), "{word} must not be stored");
        }
        assert!(json.contains("british"), "but the habit is");
    }

    #[test]
    fn acceptance_counters() {
        let mut store = StyleStore::default();
        assert!(record_decision(&mut store, "preset:fix", Decision::Accepted));
        assert!(record_decision(&mut store, "preset:fix", Decision::Accepted));
        assert!(record_decision(&mut store, "preset:fix", Decision::Rejected));
        assert!(record_decision(&mut store, "edit:spelling", Decision::Rejected));
        assert_eq!(decision_counts(&store, "preset:fix"), (2, 1));
        assert_eq!(decision_counts(&store, "edit:spelling"), (0, 1));
        assert_eq!(decision_counts(&store, "preset:shorter"), (0, 0));
        assert!(!record_decision(&mut store, "my secret sentence", Decision::Accepted), "text is not a kind");
        assert!(!record_decision(&mut store, "", Decision::Accepted));
        assert!(!record_decision(&mut store, &"a".repeat(41), Decision::Accepted));
        store.learn_from_drafts = LearnState::Paused;
        assert!(!record_decision(&mut store, "preset:fix", Decision::Accepted));
        assert_eq!(decision_counts(&store, "preset:fix"), (2, 1));
    }

    #[test]
    fn a_habit_emerges_after_enough_text_and_applies() {
        let mut store = StyleStore::default();
        for i in 0..40 {
            learn_from_text(&mut store, Source::Drafts, Scope::Chat, "I can't make it. See you there.", i);
        }
        assert_eq!(store.applied("contractions", Scope::Chat), Some("uses"));
        assert_eq!(store.applied("contractions", Scope::Email), Some("uses"), "falls back to the everywhere habit");
        assert_eq!(store.applied("dialect", Scope::Chat), None, "no dialect evidence");
    }
}
