use crate::types::WritingMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Spoken-to-written replacement rule (e.g., "home care toolbox" -> "HomeCareToolbox").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementRule {
    pub spoken: String,
    pub written: String,
    #[serde(default)]
    pub case_sensitive: bool,
}

/// Privacy-safe personalization store (Master Prompt Section 11).
/// Never stores audio; stores only user-approved vocabulary, replacement rules,
/// and per-application writing mode preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonalDictionary {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Custom names, technical jargon, product names, abbreviations.
    /// Fed both to ASR hotword biasing (when supported) and the rewrite glossary.
    #[serde(default)]
    pub vocabulary: Vec<String>,
    /// Deterministic phrase replacements applied in Tier 0 and enforced in post-guards.
    #[serde(default)]
    pub replacements: Vec<ReplacementRule>,
    /// Per-process writing mode overrides (lowercase exe name -> WritingMode),
    /// e.g. "wt.exe" -> TerminalCode, "outlook.exe" -> Professional.
    #[serde(default)]
    pub app_mode_overrides: BTreeMap<String, WritingMode>,
}

/// Most terms handed to the decoder (Hex's cap for Parakeet).
pub const MAX_HOTWORDS: usize = 64;
const MAX_HOTWORD_CHARS: usize = 64;

/// One term in a form sherpa's hotword parser accepts, or None when it would break the format or
/// is not a spoken name: over 64 characters, a newline, `/` (the phrase separator), `:` (the score
/// marker), a path or an address (`\\`, `@`, `opencode.ai`).
pub fn sanitize_hotword(term: &str) -> Option<String> {
    if term.contains(['\n', '\r', '/', '\\', ':', '@']) || term.chars().any(char::is_control) {
        return None;
    }
    let t = term.split_whitespace().collect::<Vec<_>>().join(" ");
    let dotted_domain = t.split(' ').any(|w| {
        let c: Vec<char> = w.chars().collect();
        c.windows(3).any(|x| x[1] == '.' && x[0].is_alphabetic() && x[2].is_alphabetic())
    });
    (!t.is_empty() && t.chars().count() <= MAX_HOTWORD_CHARS && !dotted_domain).then_some(t)
}

/// Cleans a vocabulary for hotword decoding: each term through `sanitize_hotword`, duplicates
/// (any case) dropped, at most `MAX_HOTWORDS` kept in list order.
pub fn sanitize_hotwords(vocabulary: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in vocabulary.iter().filter_map(|t| sanitize_hotword(t)) {
        if out.len() == MAX_HOTWORDS {
            break;
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    out
}

fn default_enabled() -> bool {
    true
}

impl PersonalDictionary {
    pub fn add_term(&mut self, term: impl Into<String>) {
        let t = term.into().trim().to_string();
        if !t.is_empty() && !self.vocabulary.iter().any(|v| v.eq_ignore_ascii_case(&t)) {
            self.vocabulary.push(t);
        }
    }

    pub fn remove_term(&mut self, term: &str) -> bool {
        let before = self.vocabulary.len();
        self.vocabulary.retain(|v| !v.eq_ignore_ascii_case(term.trim()));
        self.vocabulary.len() != before
    }

    pub fn upsert_replacement(&mut self, spoken: impl Into<String>, written: impl Into<String>, case_sensitive: bool) {
        let s = spoken.into().trim().to_string();
        let w = written.into().trim().to_string();
        if s.is_empty() || w.is_empty() {
            return;
        }
        if let Some(existing) = self
            .replacements
            .iter_mut()
            .find(|r| r.spoken.eq_ignore_ascii_case(&s))
        {
            existing.written = w;
            existing.case_sensitive = case_sensitive;
        } else {
            self.replacements.push(ReplacementRule {
                spoken: s,
                written: w,
                case_sensitive,
            });
        }
    }

    /// Exports sherpa-onnx hotwords, one cleaned term per line in the case the user typed it
    /// (the BPE hotword encoder is case sensitive). Empty when the dictionary is off or has nothing usable.
    pub fn to_sherpa_hotwords(&self) -> String {
        if !self.enabled {
            return String::new();
        }
        sanitize_hotwords(&self.vocabulary).join("\n")
    }

    /// Compact glossary snippet injected into the rewrite prompt when enabled.
    pub fn to_prompt_glossary(&self, max_terms: usize) -> String {
        if !self.enabled || self.vocabulary.is_empty() {
            return String::new();
        }
        let terms: Vec<&str> = self
            .vocabulary
            .iter()
            .take(max_terms)
            .map(|s| s.as_str())
            .collect();
        format!("Preferred spelling glossary: {}.", terms.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotwords_keep_the_users_case_and_drop_what_breaks_the_format() {
        let d = PersonalDictionary {
            enabled: true,
            vocabulary: ["  Phoebe ", "Don   Quixote", "a/b", "C:\\x", "x:3", "line\nbreak", "me@x.io", "opencode.ai", "", "phoebe", &"z".repeat(65), &"y".repeat(64)]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Default::default()
        };
        let hw = d.to_sherpa_hotwords();
        let lines: Vec<&str> = hw.lines().collect();
        assert_eq!(lines, vec!["Phoebe", "Don Quixote", &"y".repeat(64)]);
    }

    #[test]
    fn hotwords_are_empty_when_off_and_capped_at_64() {
        let mut d = PersonalDictionary { enabled: false, vocabulary: vec!["Phoebe".into()], ..Default::default() };
        assert!(d.to_sherpa_hotwords().is_empty());
        d.enabled = true;
        d.vocabulary = (0..100).map(|i| format!("term{}", "x".repeat(i % 5) + &i.to_string())).collect();
        assert_eq!(d.to_sherpa_hotwords().lines().count(), MAX_HOTWORDS);
    }
}
