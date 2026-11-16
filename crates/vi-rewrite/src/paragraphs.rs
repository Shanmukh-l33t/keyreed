//! Deterministic paragraph formatting for long dictations (no model, microseconds).
//!
//! Input is the transcript as sentences, each with the silence the speaker left before it.
//! A new paragraph starts where the speaker paused clearly longer than between ordinary
//! sentences, where a sentence opens a new point ("Finally, ..."), or, as a last resort,
//! when a paragraph has simply grown too long. Short dictations stay one paragraph.

/// Dictations shorter than this stay a single paragraph.
const MIN_WORDS_TO_SPLIT: usize = 40;
/// A paragraph needs at least this many words before a pause may end it (no one-line orphans).
const MIN_PARAGRAPH_WORDS: usize = 24;
/// Absolute floor for a "long" pause; the real threshold adapts to the speaker (see `long_pause_ms`).
const LONG_PAUSE_FLOOR_MS: u32 = 1_250;
/// Used when too few pauses are known to adapt.
const LONG_PAUSE_DEFAULT_MS: u32 = 1_350;
/// From here on, a moderately long pause is enough to break.
const SOFT_CAP_WORDS: usize = 75;
/// From here on, break at the next sentence end whatever the pause.
const HARD_CAP_WORDS: usize = 120;

/// Sentence openers that announce a new point.
const TOPIC_OPENERS: &[&str] = &[
    "finally,", "lastly,", "anyway,", "moving on", "on the other hand", "in conclusion",
    "to summarize", "to summarise", "to sum up", "regarding ", "as for ", "separately,",
    "in addition,", "apart from that", "coming to ", "now coming to", "that said,", "having said that",
];

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// A pause counts as long when it is at least twice this speaker's typical sentence pause.
fn long_pause_ms(sentences: &[(String, u32)]) -> u32 {
    let mut known: Vec<u32> = sentences.iter().skip(1).map(|s| s.1).filter(|p| *p > 0).collect();
    if known.len() < 4 {
        return LONG_PAUSE_DEFAULT_MS;
    }
    known.sort_unstable();
    (known[known.len() / 2] * 2).max(LONG_PAUSE_FLOOR_MS)
}

fn opens_new_topic(sentence: &str) -> bool {
    let lower = sentence.trim_start().to_lowercase();
    TOPIC_OPENERS.iter().any(|o| lower.starts_with(o))
}

/// Splits plain punctuated text into `(sentence, 0)` pairs when acoustic pauses are unavailable.
pub fn sentences_from_text(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if matches!(bytes[i], b'.' | b'?' | b'!') {
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b'"' | b'\'' | b')') {
                j += 1;
            }
            let rest = &text[j..];
            let trimmed = rest.trim_start();
            let next_ok = trimmed
                .chars()
                .next()
                .map(|c| c.is_uppercase() || c.is_ascii_digit() || c == '"' || c == '\'')
                .unwrap_or(false);
            if trimmed.len() < rest.len() && next_ok {
                out.push((text[start..j].trim().to_string(), 0));
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push((tail.to_string(), 0));
    }
    out
}

/// Groups sentences into paragraphs. Never drops, reorders or edits a sentence.
pub fn group_paragraphs(sentences: &[(String, u32)]) -> Vec<String> {
    let total: usize = sentences.iter().map(|s| word_count(&s.0)).sum();
    if sentences.len() < 3 || total < MIN_WORDS_TO_SPLIT {
        let all = sentences.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(" ");
        return if all.is_empty() { Vec::new() } else { vec![all] };
    }
    let long = long_pause_ms(sentences);
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut words = 0usize;
    for (text, pause) in sentences {
        let topic_brk = opens_new_topic(text) && (*pause == 0 || *pause >= 750 || words >= 50);
        let brk = words >= HARD_CAP_WORDS
            || (words >= SOFT_CAP_WORDS && *pause >= long * 6 / 10)
            || (words >= MIN_PARAGRAPH_WORDS && (*pause >= long || topic_brk));
        if brk && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            words = 0;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(text);
        words += word_count(text);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str, pause: u32) -> (String, u32) {
        (text.to_string(), pause)
    }

    const A: &str = "We agreed that the mobile app is the top priority for the next quarter.";
    const B: &str = "The web dashboard will only receive bug fixes until the app reaches parity.";
    const C: &str = "Everyone accepted that this means some customer requests will have to wait.";

    #[test]
    fn short_dictation_stays_one_paragraph() {
        let p = group_paragraphs(&[s("Send the report today.", 0), s("Thanks.", 3_000)]);
        assert_eq!(p, vec!["Send the report today. Thanks."]);
        assert!(group_paragraphs(&[]).is_empty());
    }

    #[test]
    fn long_pause_starts_a_paragraph_and_ordinary_pauses_do_not() {
        let p = group_paragraphs(&[s(A, 0), s(B, 400), s(C, 450), s(A, 1_900), s(B, 380), s(C, 500)]);
        assert_eq!(p.len(), 2, "{p:?}");
        assert_eq!(p[0], format!("{A} {B} {C}"));
        assert_eq!(p[1], format!("{A} {B} {C}"));
    }

    #[test]
    fn slow_speaker_is_not_split_after_every_sentence() {
        // every pause is 1.2 s: that is this speaker's normal rhythm, not a paragraph signal
        let v: Vec<_> = [A, B, C, A, B, C].iter().enumerate().map(|(i, t)| s(t, if i == 0 { 0 } else { 1_200 })).collect();
        assert_eq!(group_paragraphs(&v).len(), 1);
    }

    #[test]
    fn topic_opener_breaks_without_a_pause() {
        let p = group_paragraphs(&[s(A, 0), s(B, 0), s("Finally, let us move the weekly review to Tuesday.", 0), s(C, 0)]);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[1].starts_with("Finally,"));
    }

    #[test]
    fn wall_of_text_is_capped_and_nothing_is_lost() {
        // no pause information at all (e.g. a model without timestamps)
        let v: Vec<_> = (0..30).map(|i| s(if i % 2 == 0 { A } else { B }, 0)).collect();
        let p = group_paragraphs(&v);
        assert!(p.len() >= 3, "{}", p.len());
        assert!(p.iter().all(|x| word_count(x) <= HARD_CAP_WORDS + 20));
        let joined = p.join(" ");
        let original = v.iter().map(|x| x.0.as_str()).collect::<Vec<_>>().join(" ");
        assert_eq!(joined, original);
    }
}
