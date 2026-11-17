//! The grammar tagger (`proofbird/gector-bert-en`, Apache-2.0) behind the one interface Quick Fix uses: a sentence in,
//! a list of edits out.
//!
//! A port of the published reference (`quickstart.py`: tag every word, apply the tags, repeat up to five times, then
//! turn the difference into edits) together with the model author's published guards (`gector-guards.json`) as
//! `evalkit.Tagger` reads them. The model runs through the shipped ONNX Runtime (`onnxruntime.dll`, loaded at run
//! time by the `ort` crate). Edits are checked against the Python reference in `tests/data/quickfix_reference.json`.

use crate::wordpiece::WordPiece;
use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use ort::{environment::Environment, session::Session, value::Tensor};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

/// A word with one apostrophe inside, or one punctuation mark.
pub(crate) static WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"\w+(?:'\w+)?|[^\w\s]").unwrap());
const OPENING: &str = "([{\u{201C}\u{AB}\"";
const CLOSING: &str = ".,;:!?)]}%\u{BB}\u{201D}";
const MAX_PASSES: usize = 5;
/// The model reads 512 positions; a sentence this long is left to the other layers.
const MAX_TOKENS: usize = 510;
const QUANT: &[&str] = &["two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "many", "several", "few", "both", "these", "those", "multiple", "various"];
/// KeyReed's own: only these may be deleted when they are not a doubled word.
const MAY_DELETE: &[&str] = &["the", "a", "an", "of", "about", "with", "for", "at", "by", "from", "as", "more", "most", "very", "been", "be", "did", "does", "will", "would", ",", "'"];
static WORDS_KNOWN: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut set: HashSet<&'static str> = include_str!("../data/words.txt").lines().collect();
    set.insert("i");
    set
});

/// Whether `word` (any case) is in the English word list the guards use.
pub(crate) fn is_known_word(word: &str) -> bool {
    WORDS_KNOWN.contains(word.to_lowercase().as_str())
}

/// One change to a sentence. `start` and `length` are byte offsets into the sentence given to the tagger.
#[derive(Debug, Clone, PartialEq)]
pub struct RawEdit {
    pub start: usize,
    pub length: usize,
    pub replacement: String,
    /// The lowest probability among the labels the model chose to change something in its first pass.
    pub confidence: f32,
}

/// Sentence in, edit list out. Quick Fix talks to the grammar model only through this, so another model can replace it.
pub trait SentenceTagger: Send {
    fn tag_sentence(&mut self, sentence: &str) -> Result<Vec<RawEdit>>;
}

pub struct Gector {
    session: Session,
    tok: WordPiece,
    labels: Vec<String>,
    keep: usize,
    verbs: HashMap<String, String>,
    guards: Guards,
}

impl Gector {
    /// `ort_dll` is the shipped `onnxruntime.dll`; `model_dir` holds `model.onnx`, `tokenizer.json`, `config.json`,
    /// `verb-form-vocab.txt` and `gector-guards.json`. `threads` is the intra-op thread count (0 lets ONNX Runtime choose).
    pub fn load(ort_dll: &Path, model_dir: &Path, threads: usize) -> Result<Self> {
        let env = ort_environment(ort_dll)?;
        let read = |name: &str| std::fs::read_to_string(model_dir.join(name)).with_context(|| format!("read {name}"));
        let config: serde_json::Value = serde_json::from_str(&read("config.json")?).context("config.json")?;
        let id2label = config["id2label"].as_object().ok_or_else(|| anyhow!("config.json: no id2label"))?;
        let mut labels = vec![String::new(); id2label.len()];
        for (k, v) in id2label {
            let slot = k.parse::<usize>().ok().and_then(|i| labels.get_mut(i)).ok_or_else(|| anyhow!("config.json: bad label id {k}"))?;
            *slot = v.as_str().unwrap_or_default().to_string();
        }
        let keep_label = config["keep_label"].as_str().unwrap_or("$KEEP");
        let keep = labels.iter().position(|l| l == keep_label).ok_or_else(|| anyhow!("config.json: no {keep_label} label"))?;
        let mut verbs = HashMap::new();
        for line in read("verb-form-vocab.txt")?.lines() {
            if let Some((words, tags)) = line.trim().split_once(':') {
                if let Some((w1, w2)) = words.split_once('_') {
                    verbs.insert(format!("{w1}_{tags}"), w2.to_string());
                }
            }
        }
        let mut builder = Session::builder(&env).map_err(oerr)?;
        if threads > 0 {
            builder = builder.with_intra_threads(threads).map_err(oerr)?;
        }
        let session = builder.commit_from_file(model_dir.join("model.onnx")).map_err(oerr)?;
        Ok(Self {
            session,
            tok: WordPiece::from_file(&model_dir.join("tokenizer.json"))?,
            labels,
            keep,
            verbs,
            guards: Guards::parse(&read("gector-guards.json")?)?,
        })
    }

    /// One pass: the model's chosen label for `$START` and for every word, with its probability.
    fn tag(&mut self, words: &[String]) -> Result<Vec<(usize, f32)>> {
        let enc = self.tok.encode_words(words);
        let n = enc.ids.len();
        let ids = Tensor::from_array(([1usize, n], enc.ids.iter().map(|&i| i as i64).collect::<Vec<_>>()))?;
        let mask = Tensor::from_array(([1usize, n], vec![1i64; n]))?;
        let out = self.session.run(ort::inputs!["input_ids" => ids, "attention_mask" => mask])?;
        let (shape, logits) = out["logits_labels"].try_extract_tensor::<f32>()?;
        let classes = *shape.last().ok_or_else(|| anyhow!("model output has no shape"))? as usize;
        if classes != self.labels.len() || logits.len() != n * classes {
            bail!("model output does not match the label list");
        }
        // The first token of each word carries its label. A word the tokenizer drops entirely keeps.
        let mut result = vec![(self.keep, 1.0f32); words.len() + 1];
        let mut seen = HashSet::new();
        for (t, w) in enc.word_ids.iter().enumerate() {
            let word = if t == 0 { Some(0) } else { w.map(|w| w + 1) };
            let Some(word) = word.filter(|w| seen.insert(*w)) else { continue };
            let row = &logits[t * classes..(t + 1) * classes];
            let mut best = 0;
            for (i, &v) in row.iter().enumerate() {
                if v > row[best] {
                    best = i;
                }
            }
            let denom: f32 = row.iter().map(|&v| (v - row[best]).exp()).sum();
            result[word] = (best, 1.0 / denom);
        }
        Ok(result)
    }

    /// Applies one tag to one word (`quickstart.py: edit`).
    fn edit(&self, word: &str, label: &str) -> String {
        if let Some(rest) = label.strip_prefix("$APPEND_") {
            return format!("{word} {rest}");
        }
        if word == "$START" || label == "$KEEP" {
            return word.to_string();
        }
        if let Some(rest) = label.strip_prefix("$REPLACE_") {
            return rest.to_string();
        }
        if label == "$DELETE" {
            return String::new();
        }
        if let Some(rest) = label.strip_prefix("$TRANSFORM_SUF_") {
            let Some((old, new)) = rest.split_once('_') else { return word.to_string() };
            return match word.strip_suffix(old) {
                Some(stem) => format!("{stem}{new}"),
                None => word.to_string(),
            };
        }
        if let Some(rest) = label.strip_prefix("$TRANSFORM_VERB_") {
            return self.verbs.get(&format!("{word}_{rest}")).cloned().unwrap_or_else(|| word.to_string());
        }
        let mut chars = word.chars();
        match label {
            "$TRANSFORM_CASE_CAPITAL" => {
                let first: String = chars.next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                format!("{first}{}", chars.as_str().to_lowercase())
            }
            "$TRANSFORM_CASE_LOWER" => word.to_lowercase(),
            "$TRANSFORM_CASE_UPPER" => word.to_uppercase(),
            "$TRANSFORM_AGREEMENT_PLURAL" => format!("{word}s"),
            "$TRANSFORM_AGREEMENT_SINGULAR" => {
                chars.next_back();
                chars.as_str().to_string()
            }
            "$TRANSFORM_SPLIT_HYPHEN" => word.replace('-', " "),
            l if l.starts_with("$MERGE_") => format!("{word} {l}"),
            _ => word.to_string(),
        }
    }

    /// The model's labels for one pass with the published guards applied (`evalkit.Tagger.tag`).
    fn guarded_tags(&mut self, words: &[String]) -> Result<Vec<(usize, f32)>> {
        let mut tags = self.tag(words)?;
        let low: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        // The reference guards words 1..n only. A word appended after `$START` (an article at the start of the text) is
        // the same insertion the append guard exists to stop, so it is guarded too (the only deviation).
        for i in 0..tags.len() {
            if tags[i].0 == self.keep {
                continue;
            }
            let label = self.labels[tags[i].0].clone();
            if i == 0 && !label.starts_with("$APPEND_") {
                continue;
            }
            let (w, lw) = if i == 0 { ("$START", "$start") } else { (words[i - 1].as_str(), low[i - 1].as_str()) };
            let lp = if i >= 2 { low[i - 2].as_str() } else { "" };
            let last = i == words.len() || !words[i].chars().next().is_some_and(char::is_alphanumeric);
            let new = self.edit(w, &label);
            if self.guards.vetoes(&label, w, lw, lp, last, &new, &low, i) {
                tags[i].0 = self.keep;
            }
        }
        Ok(tags)
    }

    /// Up to five passes of tag and apply (`quickstart.py: correct`), returning the final words and the lowest
    /// first-pass confidence among the changes.
    fn correct_words(&mut self, mut words: Vec<String>) -> Result<(Vec<String>, f32)> {
        let mut confidence = 1.0f32;
        for pass in 0..MAX_PASSES {
            let tags = self.guarded_tags(&words)?;
            if pass == 0 {
                confidence = tags.iter().filter(|t| t.0 != self.keep).map(|t| t.1).fold(1.0, f32::min);
            }
            let edited: Vec<String> = std::iter::once("$START")
                .chain(words.iter().map(String::as_str))
                .zip(&tags)
                .map(|(w, &(l, _))| self.edit(w, &self.labels[l]))
                .filter(|e| !e.is_empty())
                .collect();
            let joined = edited.join(" ").replace(" $MERGE_HYPHEN ", "-").replace(" $MERGE_SPACE ", "");
            let tokens = joined.split_whitespace().filter(|t| *t != "$START");
            let mut rejoined: Vec<String> = Vec::new();
            for t in tokens {
                match rejoined.last_mut() {
                    Some(last) if last.chars().count() > 1 && last.ends_with('\'') && t.chars().next().is_some_and(char::is_alphanumeric) => last.push_str(t),
                    _ => rejoined.push(t.to_string()),
                }
            }
            let new_words: Vec<String> = rejoined.iter().flat_map(|t| split_words(t)).collect();
            if new_words == words {
                break;
            }
            words = new_words;
        }
        Ok((words, confidence))
    }
}

impl SentenceTagger for Gector {
    fn tag_sentence(&mut self, sentence: &str) -> Result<Vec<RawEdit>> {
        // The final stop comes off before the model and stays where it was (language-rules.json: stripTrailingTerminator).
        let trimmed = sentence.trim_end();
        let core = match trimmed.chars().next_back() {
            Some(c) if ".!?".contains(c) => &trimmed[..trimmed.len() - c.len_utf8()],
            _ => sentence,
        };
        // The models were trained on straight apostrophes; the character count stays the same.
        let plain: Vec<char> = core.chars().map(|c| if c == '\u{2019}' { '\'' } else { c }).collect();
        let plain_str: String = plain.iter().collect();
        let spans = word_spans(&plain_str);
        let words: Vec<String> = spans.iter().map(|s| s.0.clone()).collect();
        if words.is_empty() || self.tok.encode_words(&words).ids.len() > MAX_TOKENS {
            return Ok(Vec::new());
        }
        let (corrected, confidence) = self.correct_words(words)?;
        let to_byte: Vec<usize> = core.char_indices().map(|(b, _)| b).chain(std::iter::once(core.len())).collect();
        Ok(suggestions(&plain, &spans, &corrected)
            .into_iter()
            .map(|(start, end, replacement)| RawEdit { start: to_byte[start], length: to_byte[end] - to_byte[start], replacement, confidence })
            .collect())
    }
}

fn split_words(text: &str) -> Vec<String> {
    WORD.find_iter(text).map(|m| m.as_str().to_string()).collect()
}

/// The words of a sentence with their character positions `(word, start, end)`.
fn word_spans(text: &str) -> Vec<(String, usize, usize)> {
    let to_char: HashMap<usize, usize> = text.char_indices().enumerate().map(|(c, (b, _))| (b, c)).chain(std::iter::once((text.len(), text.chars().count()))).collect();
    WORD.find_iter(text).map(|m| (m.as_str().to_string(), to_char[&m.start()], to_char[&m.end()])).collect()
}

/// Words back to normal text: no space before closing punctuation or after an opening mark.
fn join_words(text: &str) -> String {
    let flat: Vec<char> = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let mut closed = String::new();
    for (i, &c) in flat.iter().enumerate() {
        if c == ' ' && flat.get(i + 1).is_some_and(|n| CLOSING.contains(*n)) {
            continue;
        }
        closed.push(c);
    }
    let mut out = String::new();
    let mut skip_space = false;
    for c in closed.chars() {
        if skip_space && c == ' ' {
            skip_space = false;
            continue;
        }
        skip_space = c != '"' && OPENING.contains(c);
        out.push(c);
    }
    out
}

/// One edit per changed stretch of words, as `(start, end, replacement)` in characters of the original text
/// (`quickstart.py: suggestions`). Unchanged text keeps the writer's own spacing and characters.
fn suggestions(original: &[char], spans: &[(String, usize, usize)], corrected: &[String]) -> Vec<(usize, usize, String)> {
    let a: Vec<&str> = spans.iter().map(|s| s.0.as_str()).collect();
    let mut out = Vec::new();
    for (op, i1, i2, j1, j2) in opcodes(&a, corrected) {
        if op == Op::Equal {
            continue;
        }
        let mut new = join_words(&corrected[j1..j2].join(" "));
        let (mut start, mut end);
        if i1 == i2 {
            // A pure insertion: before a word it reads "new ", before punctuation " new".
            if i1 < a.len() {
                start = spans[i1].1;
                end = start;
                if new.chars().next_back().is_some_and(|c| OPENING.contains(c)) {
                    // an opening mark hugs the next word
                } else if i1 == 0 || a[i1].chars().next().is_some_and(char::is_alphanumeric) {
                    new.push(' ');
                } else {
                    new.insert(0, ' ');
                }
            } else {
                start = spans.last().map_or(0, |s| s.2);
                end = start;
                if new.chars().next().is_some_and(char::is_alphanumeric) {
                    new.insert(0, ' ');
                }
            }
        } else {
            start = spans[i1].1;
            end = spans[i2 - 1].2;
            if !new.is_empty() && original.get(end).is_some_and(|c| c.is_alphanumeric()) && new.chars().next_back().is_some_and(char::is_alphanumeric) {
                new.push(' ');
            }
            if new.is_empty() {
                // A pure deletion takes one neighbouring space with it.
                if original.get(end) == Some(&' ') {
                    end += 1;
                } else if start > 0 && original[start - 1] == ' ' {
                    start -= 1;
                }
            }
        }
        out.push((start, end, new));
    }
    out
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Equal,
    Change,
}

/// Python's `difflib.SequenceMatcher(autojunk=False).get_opcodes()`, which is not a shortest edit script: it keeps
/// the longest matching block, then recurses left and right of it. Equal and non-equal runs only.
fn opcodes(a: &[&str], b: &[String]) -> Vec<(Op, usize, usize, usize, usize)> {
    let mut b2j: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, w) in b.iter().enumerate() {
        b2j.entry(w.as_str()).or_default().push(j);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, word) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut new: HashMap<usize, usize> = HashMap::new();
            for &j in b2j.get(word).map_or(&[][..], |v| &v[..]) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                new.insert(j, k);
                if k > bestsize {
                    besti = i + 1 - k;
                    bestj = j + 1 - k;
                    bestsize = k;
                }
            }
            j2len = new;
        }
        (besti, bestj, bestsize)
    };
    let mut blocks = Vec::new();
    let mut queue = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(alo, ahi, blo, bhi);
        if k > 0 {
            blocks.push((i, j, k));
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    blocks.sort_unstable();
    blocks.push((a.len(), b.len(), 0));
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::new();
    for (ai, bj, size) in blocks {
        if i < ai || j < bj {
            ops.push((Op::Change, i, ai, j, bj));
        }
        i = ai + size;
        j = bj + size;
        if size > 0 {
            ops.push((Op::Equal, ai, i, bj, j));
        }
    }
    ops
}

/// The published guard list, read the way the evaluation kit reads it.
struct Guards {
    collective: HashMap<(String, String), HashSet<String>>,
    mass: HashSet<String>,
    mass_pairs: HashSet<(String, String)>,
    corr_pairs: HashSet<(String, String)>,
    compound_pairs: HashSet<(String, String)>,
    no_append: HashSet<String>,
    append_unless: HashSet<String>,
    append_cues: HashSet<String>,
    dup_only: HashSet<String>,
    boundary: HashSet<String>,
    block: HashSet<(String, String)>,
}

impl Guards {
    fn parse(json: &str) -> Result<Self> {
        let all: serde_json::Value = serde_json::from_str(json).context("gector-guards.json")?;
        let list = all["guards"].as_array().ok_or_else(|| anyhow!("gector-guards.json: no guards"))?;
        let find = |kind: &str, key: &str| -> Result<&str> {
            list.iter()
                .find(|g| g["type"] == kind && g[key].is_string())
                .and_then(|g| g[key].as_str())
                .ok_or_else(|| anyhow!("gector-guards.json: no {kind} guard with {key}"))
        };
        let set = |s: &str| -> HashSet<String> { s.split('|').map(str::to_string).collect() };
        let pairs = |s: &str| -> HashSet<(String, String)> { s.split('|').filter_map(|p| p.split_once('>')).map(|(a, b)| (a.to_string(), b.to_string())).collect() };
        let text = |g: &serde_json::Value, key: &str| g[key].as_str().unwrap_or_default().to_string();
        let mut collective = HashMap::new();
        for g in list.iter().filter(|g| g["type"] == "redirect_replace" && g["whenPrevIs"].is_string()) {
            collective.insert((text(g, "from"), text(g, "to")), set(&text(g, "whenPrevIs")));
        }
        Ok(Self {
            collective,
            mass: set(find("mass_noun_agreement", "words")?),
            mass_pairs: pairs(find("mass_noun_agreement", "pairs")?),
            corr_pairs: pairs(find("correlative_agreement", "pairs")?),
            compound_pairs: pairs(find("compound_subject_number", "pairs")?),
            no_append: set(find("suppress_append_words", "words")?),
            append_unless: set(find("suppress_append_unless", "words")?),
            append_cues: set(find("suppress_append_unless", "cues")?),
            dup_only: set(find("delete_only_duplicates", "words")?),
            boundary: set(find("keep_before_boundary", "words")?),
            block: list.iter().filter(|g| g["type"] == "block_replace").map(|g| (text(g, "from"), text(g, "to"))).collect(),
        })
    }

    /// True when the guard list rejects the model's change to word `i` (1-based, `$START` is 0).
    #[allow(clippy::too_many_arguments)]
    fn vetoes(&self, label: &str, w: &str, lw: &str, lp: &str, last: bool, new: &str, low: &[String], i: usize) -> bool {
        if let Some(appended) = label.strip_prefix("$APPEND_") {
            let a = appended.to_lowercase();
            return self.no_append.contains(&a) || (self.append_unless.contains(&a) && !self.append_cues.contains(&lw.replace('\'', "")));
        }
        if label == "$DELETE" {
            let dup = lp == lw;
            return (self.dup_only.contains(lw) && !dup) || (dup && w.chars().next().is_some_and(char::is_uppercase)) || (!dup && !MAY_DELETE.contains(&lw));
        }
        let new_low = new.to_lowercase();
        let pair = (lw.to_string(), new_low.clone());
        let has = |word: &str| low.iter().any(|x| x == word);
        self.block.contains(&(w.to_string(), new.to_string()))
            || (label == "$TRANSFORM_CASE_CAPITAL" && i == 1 && !WORDS_KNOWN.contains(lw))
            || (self.boundary.contains(lw) && last && (new_low == "to" || new_low == "than"))
            || self.collective.get(&pair).is_some_and(|prev| prev.contains(lp))
            || (self.mass_pairs.contains(&pair) && self.mass.contains(lp))
            || (self.compound_pairs.contains(&pair) && low[i.saturating_sub(6)..i - 1].iter().any(|x| x == "and"))
            || (self.corr_pairs.contains(&pair) && ((has("neither") && has("nor")) || (has("either") && has("or"))))
            || (label == "$TRANSFORM_AGREEMENT_SINGULAR" && ((!lp.is_empty() && lp.chars().all(|c| c.is_ascii_digit()) && lp.parse::<u64>().is_ok_and(|n| n > 1)) || QUANT.contains(&lp)))
            || (new != w && !label.starts_with("$TRANSFORM_CASE") && !new.is_empty() && new.chars().all(char::is_alphabetic) && !WORDS_KNOWN.contains(new_low.as_str()))
    }
}

/// ONNX Runtime errors that carry their builder are not `Send`; keep the message.
fn oerr<T>(e: ort::Error<T>) -> anyhow::Error {
    anyhow!("onnx runtime: {e}")
}

/// ONNX Runtime allows one environment per process: load the DLL and create it on the first call, share it afterwards.
fn ort_environment(dll: &Path) -> Result<Environment> {
    static ENV: Mutex<Option<Environment>> = Mutex::new(None);
    let mut slot = ENV.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(ort::init_from(dll).with_context(|| format!("load {}", dll.display()))?.with_name("keyreed").build()?);
    }
    Ok(slot.clone().expect("set above"))
}

/// Where the tests find the int8 grammar model and the shipped ONNX Runtime: `KEYREED_GRAMMAR_MODEL` and
/// `KEYREED_ORT_DLL`, else the folders this build was developed with. `None` skips the tests that need them.
#[cfg(test)]
pub(crate) fn test_model_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("KEYREED_GRAMMAR_MODEL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:/Users/ImMandatory/keyreed-2.0-plan/build/models/gector-bert-en-int8".into());
    let found = dir.join("model.onnx").exists();
    if !found {
        eprintln!("grammar model not found at {}: test skipped", dir.display());
    }
    found.then_some(dir)
}

#[cfg(test)]
pub(crate) fn test_ort_dll() -> std::path::PathBuf {
    std::env::var_os("KEYREED_ORT_DLL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:/Users/ImMandatory/keyreed/dist/keyreed-win-x64/runtime/sherpa-onnx/onnxruntime.dll".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn join_hugs_punctuation() {
        assert_eq!(join_words("Hello , ( world ) ! \u{201C} hi \u{201D}"), "Hello, (world)! \u{201C}hi\u{201D}");
    }

    #[test]
    fn opcodes_follow_difflib_not_a_shortest_script() {
        // difflib takes the earliest longest block (x, size 1) and rewrites around it, where a shortest script
        // would keep y.
        let ops = opcodes(&["x", "y", "z"], &s(&["y", "x", "z"]));
        assert_eq!(ops, vec![(Op::Change, 0, 0, 0, 1), (Op::Equal, 0, 1, 1, 2), (Op::Change, 1, 2, 2, 2), (Op::Equal, 2, 3, 2, 3)]);
    }

    #[test]
    fn suggestions_insert_replace_delete() {
        let text = "This are a test .";
        let chars: Vec<char> = text.chars().collect();
        let spans = word_spans(text);
        assert_eq!(suggestions(&chars, &spans, &s(&["This", "is", "a", "test", "."])), vec![(5, 8, "is".to_string())]);
        assert_eq!(suggestions(&chars, &spans, &s(&["This", "are", "test", "."])), vec![(9, 11, String::new())]);
        assert_eq!(suggestions(&chars, &spans, &s(&["This", "are", "a", "big", "test", "."])), vec![(11, 11, "big ".to_string())]);
    }

    #[test]
    fn guards_veto_collective_nouns_appended_articles_and_non_words() {
        let Some(dir) = test_model_dir() else { return };
        let g = Guards::parse(&std::fs::read_to_string(dir.join("gector-guards.json")).unwrap()).unwrap();
        let low = s(&["the", "team", "are", "here"]);
        assert!(g.vetoes("$REPLACE_is", "are", "are", "team", false, "is", &low, 3));
        assert!(!g.vetoes("$REPLACE_is", "are", "are", "they", false, "is", &low, 3));
        assert!(g.vetoes("$APPEND_the", "of", "of", "", false, "of the", &low, 1));
        assert!(g.vetoes("$REPLACE_prcoessing", "process", "process", "", false, "prcoessing", &low, 1));
    }

    #[test]
    fn fixes_agreement_and_leaves_empty_text_alone() {
        let Some(dir) = test_model_dir() else { return };
        let mut g = Gector::load(&test_ort_dll(), &dir, 2).unwrap();
        for (text, from, to) in [("This are a test.", "are", "is"), ("My brother have two cat.", "have", "has")] {
            let edits = g.tag_sentence(text).unwrap();
            assert!(edits.iter().any(|e| &text[e.start..e.start + e.length] == from && e.replacement == to), "{edits:?}");
        }
        assert!(g.tag_sentence("").unwrap().is_empty());
    }
}
