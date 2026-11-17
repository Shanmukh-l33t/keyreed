pub mod cloud_llm;
pub mod compose;
pub mod gector;
pub mod guards;
pub mod paragraphs;
pub mod quickfix;
pub mod slm;
pub mod stream;
pub mod symbols;
pub mod tier0;
pub mod wordpiece;

use guards::{verify_rewrite_candidate, GuardVerdict};
use slm::LlamaServerSupervisor;
use std::time::Instant;
use tier0::run_tier0;
use vi_core::{AppCategory, PersonalDictionary, ProcessingProfile, WritingMode};

#[derive(Debug, Clone, Default)]
pub struct RewriteOutcome {
    pub final_text: String,
    pub tier_used: String,
    pub requires_confirmation: bool,
    pub warning_note: Option<String>,
    pub skipped_as_hallucination: bool,
    pub elapsed_ms: f64,
    pub norm_ms: f64,
    pub tier0_ms: f64,
    pub slm_startup_ms: f64,
    pub slm_first_token_ms: f64,
    pub slm_ms: f64,
    pub paragraph_ms: f64,
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
            s.prewarm_mode(mode);
        }
    }

    pub fn warm_up_slot(&mut self, mode: WritingMode) {
        if !matches!(mode, WritingMode::Exact | WritingMode::TerminalCode) {
            if let Some(s) = &mut self.slm {
                s.warm_up_slot(mode);
            }
        }
    }

    /// Chat URL and per-session key of the local language model, starting it if needed;
    /// `None` when no local model is installed.
    pub fn chat_endpoint(&mut self) -> Option<anyhow::Result<(String, String)>> {
        self.slm.as_mut().map(|s| s.chat_endpoint())
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
        self.process_with_sentences(
            raw_transcript,
            &[],
            speech_ratio,
            mode,
            category,
            profile,
            dict,
            cloud_endpoint,
            cloud_model,
            cloud_api_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_with_sentences(
        &mut self,
        raw_transcript: &str,
        sentences: &[(String, u32)],
        speech_ratio: f32,
        mode: WritingMode,
        category: AppCategory,
        profile: ProcessingProfile,
        dict: &PersonalDictionary,
        cloud_endpoint: Option<&str>,
        cloud_model: Option<&str>,
        cloud_api_key: Option<&str>,
    ) -> RewriteOutcome {
        let t_all = Instant::now();
        if mode == WritingMode::TerminalCode {
            let t_norm = Instant::now();
            let prepared = symbols::apply_spoken_symbols(raw_transcript);
            let norm_ms = t_norm.elapsed().as_secs_f64() * 1000.0;
            let t_t0 = Instant::now();
            let mut t0 = run_tier0(&prepared, speech_ratio, mode, category, dict);
            t0.text = symbols::apply_spoken_symbols(&t0.text);
            let tier0_ms = t_t0.elapsed().as_secs_f64() * 1000.0;
            let empty = t0.skipped_as_hallucination || t0.text.is_empty();
            return RewriteOutcome {
                final_text: if empty { String::new() } else { t0.text },
                tier_used: if empty { "tier0_vad_filtered" } else { "tier0_deterministic" }.to_string(),
                requires_confirmation: !empty && t0.requires_confirmation,
                warning_note: if empty { None } else { t0.warning_note },
                skipped_as_hallucination: empty,
                elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
                norm_ms,
                tier0_ms,
                ..Default::default()
            };
        }

        if tier0::is_likely_silence_hallucination(raw_transcript, speech_ratio) {
            return RewriteOutcome {
                tier_used: "tier0_vad_filtered".to_string(),
                skipped_as_hallucination: true,
                elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
                ..Default::default()
            };
        }

        let t_para = Instant::now();
        let raw_paras = if sentences.len() >= 3 {
            paragraphs::group_paragraphs(sentences)
        } else if raw_transcript.split_whitespace().count() >= 40
            && !raw_transcript.to_ascii_lowercase().contains("new paragraph")
            && !raw_transcript.to_ascii_lowercase().contains("next paragraph")
        {
            paragraphs::group_paragraphs(&paragraphs::sentences_from_text(raw_transcript))
        } else {
            vec![raw_transcript.to_string()]
        };
        let mut paragraph_ms = t_para.elapsed().as_secs_f64() * 1000.0;

        let t_t0 = Instant::now();
        let mut t0_paras: Vec<(String, String)> = Vec::with_capacity(raw_paras.len());
        let mut requires_confirmation = false;
        let mut warning_note = None;
        for rp in &raw_paras {
            let t0 = run_tier0(rp, speech_ratio, mode, category, dict);
            if t0.skipped_as_hallucination || t0.text.is_empty() {
                continue;
            }
            requires_confirmation |= t0.requires_confirmation;
            if warning_note.is_none() {
                warning_note = t0.warning_note;
            }
            let subs: Vec<&str> = t0.text.split("\n\n").map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            for (si, st) in subs.iter().enumerate() {
                let raw_sub = if subs.len() == 1 { rp.as_str() } else if si == 0 { "" } else { "" };
                t0_paras.push((raw_sub.to_string(), (*st).to_string()));
            }
        }
        let tier0_ms = t_t0.elapsed().as_secs_f64() * 1000.0;

        if t0_paras.is_empty() {
            return RewriteOutcome {
                tier_used: "tier0_vad_filtered".to_string(),
                skipped_as_hallucination: true,
                elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
                tier0_ms,
                paragraph_ms,
                ..Default::default()
            };
        }

        let total_words: usize = t0_paras.iter().map(|(_, p)| p.split_whitespace().count()).sum();
        let all_clean = t0_paras
            .iter()
            .all(|(rp, tp)| is_already_clean_paragraph(rp, tp, mode));
        if mode == WritingMode::Exact || total_words <= 3 || all_clean {
            let t_join = Instant::now();
            let final_text = t0_paras.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>().join("\n\n");
            paragraph_ms += t_join.elapsed().as_secs_f64() * 1000.0;
            return RewriteOutcome {
                final_text,
                tier_used: "tier0_deterministic".to_string(),
                requires_confirmation,
                warning_note,
                skipped_as_hallucination: false,
                elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
                tier0_ms,
                paragraph_ms,
                ..Default::default()
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
                        let joined_t0 = t0_paras.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>().join("\n\n");
                        if let Ok(cand) = cloud_llm::rewrite_via_cloud(
                            ep, key, mdl, &joined_t0, mode, category, &glossary,
                        ) {
                            if let GuardVerdict::Accepted(clean) =
                                verify_rewrite_candidate(&joined_t0, &cand)
                            {
                                let clean = apply_post_dict(clean, dict);
                                return RewriteOutcome {
                                    final_text: clean,
                                    tier_used: "tier2_cloud_llm".to_string(),
                                    requires_confirmation,
                                    warning_note,
                                    skipped_as_hallucination: false,
                                    elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
                                    tier0_ms,
                                    paragraph_ms,
                                    ..Default::default()
                                };
                            }
                        }
                    }
                }
            }
        }

        // Tier 1: Local CPU SLM via supervised llama-server, rewritten per paragraph so long dictations
        // never overflow context and paragraph structure is strictly preserved.
        if let Some(slm) = &mut self.slm {
            let mut out_paras = Vec::with_capacity(t0_paras.len());
            let mut any_slm_accepted = false;
            let mut slm_startup_ms = 0.0;
            let mut slm_first_token_ms = 0.0;
            let mut slm_ms = 0.0;

            for (raw_p, p_t0) in &t0_paras {
                if p_t0.split_whitespace().count() <= 3 || is_already_clean_paragraph(raw_p, p_t0, mode) {
                    out_paras.push(p_t0.clone());
                    continue;
                }
                match slm.rewrite_timed(p_t0, mode, category, &glossary) {
                    Ok((cand, m)) => {
                        slm_startup_ms += m.startup_ms;
                        if slm_first_token_ms == 0.0 {
                            slm_first_token_ms = m.first_token_ms;
                        }
                        slm_ms += m.total_ms;
                        match verify_rewrite_candidate(p_t0, &cand) {
                            GuardVerdict::Accepted(clean) => {
                                let clean = apply_post_dict(clean, dict);
                                out_paras.push(clean);
                                any_slm_accepted = true;
                            }
                            GuardVerdict::Rejected(reason) => {
                                log::warn!("SLM candidate rejected by post-guard ({}); keeping Tier 0 paragraph", reason);
                                out_paras.push(p_t0.clone());
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Local SLM unavailable ({}); falling back to Tier 0", e);
                        out_paras.push(p_t0.clone());
                    }
                }
            }

            if any_slm_accepted {
                let t_join = Instant::now();
                let final_text = out_paras.join("\n\n");
                paragraph_ms += t_join.elapsed().as_secs_f64() * 1000.0;
                return RewriteOutcome {
                    final_text,
                    tier_used: "tier1_local_slm".to_string(),
                    requires_confirmation,
                    warning_note,
                    skipped_as_hallucination: false,
                    elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
                    tier0_ms,
                    slm_startup_ms,
                    slm_first_token_ms,
                    slm_ms,
                    paragraph_ms,
                    ..Default::default()
                };
            }
        }

        let t_join = Instant::now();
        let final_text = t0_paras.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>().join("\n\n");
        paragraph_ms += t_join.elapsed().as_secs_f64() * 1000.0;
        RewriteOutcome {
            final_text,
            tier_used: "tier0_fallback".to_string(),
            requires_confirmation,
            warning_note,
            skipped_as_hallucination: false,
            elapsed_ms: t_all.elapsed().as_secs_f64() * 1000.0,
            tier0_ms,
            paragraph_ms,
            ..Default::default()
        }
    }
}

/// Non-English letters, or romanized-Hindi function words (Hinglish). shortcut: fixed marker list, extend
/// with a real language-ID model if other code-mixed languages matter.
fn is_mixed_language(s: &str) -> bool {
    if s.chars().any(|c| c.is_alphabetic() && !c.is_ascii()) {
        return true;
    }
    const HINGLISH: &[&str] = &[
        "yaar", "bhai", "kar", "karo", "karna", "dena", "denge", "nahi", "nahin", "hai", "hain", "kya", "baje",
        "phir", "subah", "shaam", "theek", "accha", "chahiye", "abhi", "bata", "wala", "gaya", "hoga", "rakhte",
        "mein", "aur", "toh", "kal", "aaj", "hola", "por", "favor", "equipo",
    ];
    s.split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| !w.is_empty() && HINGLISH.contains(&w.to_ascii_lowercase().as_str()))
}

/// True when a Tier-0 paragraph is already cleanly punctuated, capitalized, and grammatical,
/// so invoking the neural SLM would only add latency without improving the text.
fn is_already_clean_paragraph(_raw: &str, t0: &str, mode: WritingMode) -> bool {
    // Benchmark switch: force the SLM for every eligible paragraph to measure what the gate saves/loses.
    if std::env::var_os("KEYREED_SLM_ALWAYS").is_some() {
        return false;
    }
    let t0_t = t0.trim();
    if t0_t.is_empty() {
        return false;
    }
    if is_mixed_language(t0_t) {
        return true; // a 0.8B English-only editor mangles mixed-language text (drops/translates words)
    }
    if t0_t.contains("\n- ") {
        return true;
    }
    if mode == WritingMode::Professional && t0_t.contains('\'') {
        return false;
    }
    let low = t0_t.to_ascii_lowercase();
    // General non-native-English / grammar-risk markers that deterministic rules cannot fix.
    for bad in [
        "not able to", "why is the", "what is the", "where is the", "how is the", "where is ", "how is ",
        "we was", "they was", "you was", "he don't", "she don't", "it don't",
        "kindly ", "myself ", "revert back", "do the needful", "having a doubt", "is having ",
        "am having", "are having",
    ] {
        if low.contains(bad) {
            return false;
        }
    }
    let sents = paragraphs::sentences_from_text(t0_t);
    if sents.is_empty() {
        return false;
    }
    for (s, _) in &sents {
        let st = s.trim();
        let first_ok = st
            .chars()
            .next()
            .map(|c| c.is_uppercase() || c.is_ascii_digit())
            .unwrap_or(false);
        let last_ok = st.ends_with('.') || st.ends_with('?') || st.ends_with('!');
        if !first_ok || !last_ok {
            return false;
        }
        let wc = st.split_whitespace().count();
        let has_internal_punct = st.contains(',') || st.contains(';') || st.contains(':');
        if wc > 18 && !has_internal_punct {
            return false;
        }
        let s_low = st.to_ascii_lowercase();
        if wc >= 12
            && !has_internal_punct
            && (s_low.contains(" and ")
                || s_low.contains(" but ")
                || s_low.contains(" whereas ")
                || s_low.contains(" while "))
        {
            return false;
        }
    }
    true
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
    fn esl_rules_fix_known_constructions_but_exact_mode_is_untouched() {
        let mut eng = RewriteEngine::new(None);
        let d = PersonalDictionary::default();
        let p = ProcessingProfile::MaxPrivacyOffline;
        let run = |e: &mut RewriteEngine, m, t: &str| {
            e.process(t, 0.9, m, AppCategory::General, p, &d, None, None, None).final_text
        };
        let o = run(&mut eng, WritingMode::Smart, "we was discussing about the informations since two hours");
        assert_eq!(o, "We were discussing the information for two hours.");
        let o = run(&mut eng, WritingMode::Smart, "he don't know and she can explain me");
        assert!(o.contains("doesn't") && o.contains("explain to me"), "{o}");
        let ex = run(&mut eng, WritingMode::Exact, "we was discussing about the informations");
        assert!(ex.contains("informations") && ex.contains("we was") || ex.contains("We was"), "{ex}");
    }

    /// Sanity: the offline profile is the one that disallows network, and it is the app default.
    #[test]
    fn offline_profile_disallows_network() {
        assert!(!ProcessingProfile::MaxPrivacyOffline.allows_network());
    }
}
