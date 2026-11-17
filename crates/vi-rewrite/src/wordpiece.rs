//! BERT WordPiece tokenizer for the grammar tagger (`bert-base-cased`: no lower-casing, no accent stripping).
//!
//! Reads the model's own `tokenizer.json` and reproduces what the Python `tokenizers` package does for
//! `encode(words, is_pretokenized=True)`: the BERT normaliser (control characters dropped, whitespace made
//! plain, spaces around CJK ideographs), the BERT pre-tokenizer (split at whitespace and punctuation), greedy
//! longest-match WordPiece, `[CLS]` and `[SEP]` around the sentence. Locked to golden vectors made by the
//! Python package (`tests/data/wordpiece_golden.json`).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub struct WordPiece {
    vocab: HashMap<String, u32>,
    unk: u32,
    cls: u32,
    sep: u32,
    prefix: String,
    max_chars: usize,
}

/// Ids with `[CLS]` first and `[SEP]` last, and for every id the index of the input word it came from
/// (`None` for the two special tokens).
pub struct Encoding {
    pub ids: Vec<u32>,
    pub word_ids: Vec<Option<usize>>,
}

impl WordPiece {
    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let json: serde_json::Value = serde_json::from_str(&raw).context("tokenizer.json is not JSON")?;
        let model = &json["model"];
        if model["type"] != "WordPiece" {
            bail!("tokenizer.json: only WordPiece models are supported");
        }
        let vocab: HashMap<String, u32> = serde_json::from_value(model["vocab"].clone()).context("tokenizer.json vocab")?;
        let id = |t: &str| vocab.get(t).copied().ok_or_else(|| anyhow!("tokenizer.json: no {t} token"));
        Ok(Self {
            unk: id(model["unk_token"].as_str().unwrap_or("[UNK]"))?,
            cls: id("[CLS]")?,
            sep: id("[SEP]")?,
            prefix: model["continuing_subword_prefix"].as_str().unwrap_or("##").to_string(),
            max_chars: model["max_input_chars_per_word"].as_u64().unwrap_or(100) as usize,
            vocab,
        })
    }

    pub fn encode_words(&self, words: &[String]) -> Encoding {
        let mut ids = vec![self.cls];
        let mut word_ids = vec![None];
        for (i, word) in words.iter().enumerate() {
            for piece in pre_tokenize(&normalize(word)) {
                let before = ids.len();
                self.word_piece(&piece, &mut ids);
                word_ids.extend(std::iter::repeat(Some(i)).take(ids.len() - before));
            }
        }
        ids.push(self.sep);
        word_ids.push(None);
        Encoding { ids, word_ids }
    }

    /// Greedy longest match first; a word with any unmatched stretch becomes one `[UNK]`.
    fn word_piece(&self, word: &str, out: &mut Vec<u32>) {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > self.max_chars {
            out.push(self.unk);
            return;
        }
        let mut pieces = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut end = chars.len();
            let mut found = None;
            while start < end {
                let mut s: String = chars[start..end].iter().collect();
                if start > 0 {
                    s.insert_str(0, &self.prefix);
                }
                if let Some(&id) = self.vocab.get(&s) {
                    found = Some(id);
                    break;
                }
                end -= 1;
            }
            match found {
                Some(id) => pieces.push(id),
                None => {
                    out.push(self.unk);
                    return;
                }
            }
            start = end;
        }
        out.extend(pieces);
    }
}

fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\0' || c == '\u{FFFD}' || (is_control(c)) {
            continue;
        }
        if c.is_whitespace() {
            out.push(' ');
        } else if is_cjk(c) {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Control characters other than tab, newline and carriage return (those count as whitespace).
fn is_control(c: char) -> bool {
    !matches!(c, '\t' | '\n' | '\r') && (c.is_control() || is_format(c))
}

/// Unicode "Cf" characters that BERT's `_is_control` also drops (zero-width joiners, direction marks, BOM, soft hyphen).
fn is_format(c: char) -> bool {
    matches!(c, '\u{AD}' | '\u{600}'..='\u{605}' | '\u{61C}' | '\u{6DD}' | '\u{70F}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}')
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF | 0x2A700..=0x2B73F | 0x2B740..=0x2B81F
        | 0x2B820..=0x2CEAF | 0xF900..=0xFAFF | 0x2F800..=0x2FA1F)
}

fn is_punct(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation();
    }
    matches!(c, '\u{A1}' | '\u{A7}' | '\u{AB}' | '\u{B6}' | '\u{B7}' | '\u{BB}' | '\u{BF}' | '\u{37E}' | '\u{387}'
        | '\u{2010}'..='\u{2027}' | '\u{2030}'..='\u{2043}' | '\u{2045}'..='\u{2051}' | '\u{2053}'..='\u{205E}'
        | '\u{2308}'..='\u{230B}' | '\u{2329}' | '\u{232A}' | '\u{3001}'..='\u{3003}' | '\u{3008}'..='\u{3011}'
        | '\u{3014}'..='\u{301F}' | '\u{FF01}'..='\u{FF03}' | '\u{FF05}'..='\u{FF0A}' | '\u{FF0C}'..='\u{FF0F}'
        | '\u{FF1A}' | '\u{FF1B}' | '\u{FF1F}' | '\u{FF20}' | '\u{FF3B}'..='\u{FF3D}' | '\u{FF3F}' | '\u{FF5B}' | '\u{FF5D}'
        | '\u{FF5F}'..='\u{FF65}')
}

/// Splits at whitespace and puts every punctuation mark on its own.
fn pre_tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if is_punct(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> WordPiece {
        let vocab: HashMap<String, u32> = ["[UNK]", "[CLS]", "[SEP]", "un", "##aff", "##able", "don", "'", "t", ".", "a"]
            .iter()
            .enumerate()
            .map(|(i, t)| (t.to_string(), i as u32))
            .collect();
        WordPiece { vocab, unk: 0, cls: 1, sep: 2, prefix: "##".into(), max_chars: 100 }
    }

    #[test]
    fn pieces_punctuation_unknown_and_word_ids() {
        let e = tiny().encode_words(&["unaffable".into(), "don't".into(), "zzz".into(), ".".into()]);
        assert_eq!(e.ids, vec![1, 3, 4, 5, 6, 7, 8, 0, 9, 2]);
        assert_eq!(
            e.word_ids,
            vec![None, Some(0), Some(0), Some(0), Some(1), Some(1), Some(1), Some(2), Some(3), None]
        );
    }

    #[test]
    fn overlong_word_is_one_unknown() {
        let e = tiny().encode_words(&["a".repeat(101)]);
        assert_eq!(e.ids, vec![1, 0, 2]);
    }

    #[test]
    fn empty_input_is_just_the_markers() {
        let e = tiny().encode_words(&[]);
        assert_eq!(e.ids, vec![1, 2]);
        assert_eq!(e.word_ids, vec![None, None]);
    }

    #[test]
    fn matches_python_tokenizers_golden_vectors() {
        let Some(dir) = crate::gector::test_model_dir() else { return };
        let wp = WordPiece::from_file(&dir.join("tokenizer.json")).unwrap();
        let golden: serde_json::Value = serde_json::from_str(include_str!("../tests/data/wordpiece_golden.json")).unwrap();
        let cases = golden["cases"].as_array().unwrap();
        assert!(cases.len() >= 200);
        for c in cases {
            let words: Vec<String> = c["words"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
            let e = wp.encode_words(&words);
            let ids: Vec<u32> = c["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
            let wids: Vec<Option<usize>> = c["word_ids"].as_array().unwrap().iter().map(|v| v.as_u64().map(|n| n as usize)).collect();
            assert_eq!((e.ids, e.word_ids), (ids, wids), "words {words:?}");
        }
    }
}
