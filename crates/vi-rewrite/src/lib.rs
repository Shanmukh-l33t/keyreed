pub mod cloud_llm;
pub mod guards;
pub mod slm;
pub mod symbols;
pub mod tier0;

use guards::{verify_rewrite_candidate, GuardVerdict};
use slm::LlamaServerSupervisor;
use std::time::Instant;
use tier0::run_tier0;
use vi_core::{AppCategory, PersonalDictionary, ProcessingProfile, WritingMode};

#[derive(Debug, Clone)]
pub struct RewriteOutcome {
    pub final_text: String,
    pub tier_used: String,
    pub requires_confirmation: bool,
    pub warning_note: Option<String>,
    pub skipped_as_hallucination: bool,
    pub elapsed_ms: f64,
}

/// Orchestrates the 3-tier conservative rewrite pipeline:
/// - Tier 0 (Deterministic Rust, <0.3 ms): always runs; authoritative for `Exact` and `TerminalCode` modes!
/// - Tier 1 (Local CPU SLM via `llama-server` + KV cache + strict post-guards): runs offline when configured.
/// - Tier 2 (Optional BYOK Cloud LLM + strict post-guards): runs only when `ProcessingProfile` allows network and key is present; seamlessly falls back to Tier 1 / Tier 0 offline!
pub struct RewriteEngine {
    slm: Option<LlamaServerSupervisor>,
}

impl Default for RewriteEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

impl RewriteEngine {
    pub fn new(slm: Option<LlamaServerSupervisor>) -> Self {
        Self { slm }
    }

    /// Starts the local grammar model in the background when this dictation may use it.
    pub fn prewarm(&mut self, mode: WritingMode) {
        if matches!(mode, WritingMode::Exact | WritingMode::TerminalCode) {
            return; // these modes never use the language model
        }
        if let Some(s) = &mut self.slm {
            s.prewarm();
        }
    }

    pub fn maybe_unload_idle(&mut self, idle_secs: u64) -> bool {
        if let Some(s) = &mut self.slm {
            s.maybe_unload_if_idle(idle_secs)
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        raw_transcript: &str,
        speech_ratio: f32,
        mode: WritingMode,
        category: AppCategory,
        profile: ProcessingProfile,
        dict: &PersonalDictionary,
        cloud_endpoint: Option<&str>,
        cloud_model: Option<&str>,
        cloud_api_key: Option<&str>,
    ) -> RewriteOutcome {
        let t0_clock = Instant::now();
        let prepared = if mode == WritingMode::TerminalCode {
            symbols::apply_spoken_symbols(raw_transcript)
        } else {
            raw_transcript.to_string()
        };
        let mut t0 = run_tier0(&prepared, speech_ratio, mode, category, dict);
        if mode == WritingMode::TerminalCode {
            t0.text = symbols::apply_spoken_symbols(&t0.text);
        }

        if t0.skipped_as_hallucination || t0.text.is_empty() {
            return RewriteOutcome {
                final_text: String::new(),
                tier_used: "tier0_vad_filtered".to_string(),
                requires_confirmation: false,
                warning_note: None,
                skipped_as_hallucination: true,
                elapsed_ms: t0_clock.elapsed().as_secs_f64() * 1000.0,
            };
        }

        // Master Prompt Section 5: Exact Dictation and Terminal/Code Mode NEVER apply creative LLM rewriting!
        // Also ultra-short 1-3 word utterances ("Yes.", "Thank you.", "Approved.") need zero LLM latency.
        let word_count = t0.text.split_whitespace().count();
        if matches!(mode, WritingMode::Exact | WritingMode::TerminalCode) || word_count <= 3 {
            return RewriteOutcome {
                final_text: t0.text,
                tier_used: "tier0_deterministic".to_string(),
                requires_confirmation: t0.requires_confirmation,
                warning_note: t0.warning_note,
                skipped_as_hallucination: false,
                elapsed_ms: t0_clock.elapsed().as_secs_f64() * 1000.0,
            };
        }

        let glossary = dict.to_prompt_glossary(30);

        // Tier 2: Cloud LLM when profile == MaxAccuracy (or Balanced when no local SLM is configured) and credentials exist.
        if profile.allows_network() {
            let prefer_cloud = profile == ProcessingProfile::MaxAccuracy || self.slm.is_none();
            if prefer_cloud {
                if let (Some(ep), Some(mdl), Some(key)) =
                    (cloud_endpoint, cloud_model, cloud_api_key)
                {
                    if !key.trim().is_empty() {
                        if let Ok(cand) = cloud_llm::rewrite_via_cloud(
                            ep, key, mdl, &t0.text, mode, category, &glossary,
                        ) {
                            if let GuardVerdict::Accepted(clean) =
                                verify_rewrite_candidate(&t0.text, &cand)
                            {
                                let clean = apply_post_dict(clean, dict);
                                return RewriteOutcome {
                                    final_text: clean,
                                    tier_used: "tier2_cloud_llm".to_string(),
                                    requires_confirmation: t0.requires_confirmation,
                                    warning_note: t0.warning_note,
                                    skipped_as_hallucination: false,
                                    elapsed_ms: t0_clock.elapsed().as_secs_f64() * 1000.0,
                                };
                            }
                        }
                    }
                }
            }
        }

        // Tier 1: Local CPU SLM via supervised llama-server
        if let Some(slm) = &mut self.slm {
            match slm.rewrite(&t0.text, mode, category, &glossary) {
                Ok(cand) => match verify_rewrite_candidate(&t0.text, &cand) {
                    GuardVerdict::Accepted(clean) => {
                        let clean = apply_post_dict(clean, dict);
                        return RewriteOutcome {
                            final_text: clean,
                            tier_used: "tier1_local_slm".to_string(),
                            requires_confirmation: t0.requires_confirmation,
                            warning_note: t0.warning_note,
                            skipped_as_hallucination: false,
                            elapsed_ms: t0_clock.elapsed().as_secs_f64() * 1000.0,
                        };
                    }
                    GuardVerdict::Rejected(reason) => {
                        log::warn!("SLM candidate rejected by post-guard ({}); falling back to Tier 0", reason);
                    }
                },
                Err(e) => {
                    log::warn!("Local SLM unavailable ({}); falling back to Tier 0", e);
                }
            }
        }

        RewriteOutcome {
            final_text: t0.text,
            tier_used: "tier0_fallback".to_string(),
            requires_confirmation: t0.requires_confirmation,
            warning_note: t0.warning_note,
            skipped_as_hallucination: false,
            elapsed_ms: t0_clock.elapsed().as_secs_f64() * 1000.0,
        }
    }
}

fn apply_post_dict(mut text: String, dict: &PersonalDictionary) -> String {
    if !dict.enabled {
        return text;
    }
    for term in &dict.vocabulary {
        let pat = format!(r"(?i)\b{}\b", regex::escape(term));
        if let Ok(re) = regex::Regex::new(&pat) {
            text = re.replace_all(&text, term.as_str()).to_string();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Threat model T2 invariant: MaxPrivacyOffline must NEVER attempt a network call, even when
    /// a full set of cloud credentials is supplied. If this regresses, dictated audio/text could
    /// leave the machine without user opt-in. The assertion is that the tier is a local one.
    #[test]
    fn max_privacy_offline_never_uses_cloud_tier() {
        let mut eng = RewriteEngine::new(None); // no local SLM either
        let dict = PersonalDictionary::default();
        let out = eng.process(
            "hey so the the meeting tomorrow at 3 pm got moved to 4 right",
            0.9,
            WritingMode::Smart,
            AppCategory::Chat,
            ProcessingProfile::MaxPrivacyOffline,
            &dict,
            // Fully-populated cloud creds: must be ignored under MaxPrivacyOffline.
            Some("https://127.0.0.1:9/v1/chat/completions"),
            Some("gpt-oss-20b"),
            Some("sk-should-never-be-used"),
        );
        assert!(
            !out.tier_used.contains("cloud"),
            "MaxPrivacyOffline reached the network path: {}",
            out.tier_used
        );
        assert!(out.final_text.len() > 0);
    }

    #[test]
    fn spoken_symbols_apply_only_in_terminal_code() {
        let mut eng = RewriteEngine::new(None);
        let dict = PersonalDictionary::default();
        let src = "cd forward slash var underscore name open bracket close bracket";
        let term = eng.process(
            src,
            1.0,
            WritingMode::TerminalCode,
            AppCategory::Terminal,
            ProcessingProfile::MaxPrivacyOffline,
            &dict,
            None,
            None,
            None,
        );
        assert!(term.final_text.contains('/'), "{}", term.final_text);
        assert!(term.final_text.contains('_'), "{}", term.final_text);
        assert!(term.final_text.contains('['), "{}", term.final_text);
        assert!(!term.tier_used.contains("cloud"));
        let smart = eng.process(
            src,
            1.0,
            WritingMode::Smart,
            AppCategory::Chat,
            ProcessingProfile::MaxPrivacyOffline,
            &dict,
            None,
            None,
            None,
        );
        assert!(
            !smart.final_text.contains("cd /"),
            "smart mode must not apply spoken symbols: {}",
            smart.final_text
        );
    }

    /// Sanity: the offline profile is the one that disallows network, and it is the app default.
    #[test]
    fn offline_profile_disallows_network() {
        assert!(!ProcessingProfile::MaxPrivacyOffline.allows_network());
    }
}
