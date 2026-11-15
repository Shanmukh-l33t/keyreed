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

    /// Exports sherpa-onnx hotwords lines (uppercase tokens for BPE transducers).
    pub fn to_sherpa_hotwords(&self) -> String {
        if !self.enabled {
            return String::new();
        }
        self.vocabulary
            .iter()
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().to_uppercase())
            .collect::<Vec<_>>()
            .join("\n")
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
