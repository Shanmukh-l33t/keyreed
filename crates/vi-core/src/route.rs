//! The route rule (plan P3.5): one function decides where text jobs run, from the processing
//! profile, the "Let text jobs use the cloud" switch and the cloud settings. Pure, so the offline
//! guarantee can be tested without a network or a window.

use crate::config::CloudSettings;
use crate::textjob::CloudTarget;
use crate::types::ProcessingProfile;

/// Where text jobs run: the bar's route label, and whether a job may carry a cloud target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// "Cloud · api.example.com · model" or "On this PC · Qwen3.5 2B".
    pub label: String,
    /// The route is the user's cloud endpoint (the bar colours the label).
    pub cloud: bool,
    /// Somewhere to run: false when no local model is installed and the cloud is not allowed.
    pub allowed: bool,
}

/// The one rule. Maximum Privacy never routes to the cloud, whatever the switch says.
pub fn route(profile: ProcessingProfile, allow_cloud: bool, cloud: &CloudSettings, local_model: Option<&str>) -> Route {
    if cloud_ready(profile, allow_cloud, cloud) {
        let label = format!("Cloud · {} · {}", host_of(&cloud.llm_endpoint), cloud.llm_model.trim());
        return Route { label, cloud: true, allowed: true };
    }
    let local = local_model.map(str::trim).filter(|m| !m.is_empty());
    let label = match local {
        Some(model) => format!("On this PC · {model}"),
        None => "On this PC · built-in model".to_string(),
    };
    Route { label, cloud: false, allowed: local.is_some() }
}

impl Route {
    /// The cloud target a text job carries, or `None` when it runs locally. The only builder of
    /// `CloudTarget`, so the offline profile cannot hand the worker an endpoint.
    pub fn target(&self, cloud: &CloudSettings, api_key: String) -> Option<CloudTarget> {
        self.cloud.then(|| CloudTarget {
            endpoint: cloud.llm_endpoint.trim().to_string(),
            model: cloud.llm_model.trim().to_string(),
            api_key,
        })
    }
}

fn cloud_ready(profile: ProcessingProfile, allow_cloud: bool, cloud: &CloudSettings) -> bool {
    profile.allows_network()
        && allow_cloud
        // The same rule the stream enforces: text leaves the PC over HTTPS, or stays on the loopback.
        && (cloud.llm_endpoint.trim().starts_with("https://") || cloud.llm_endpoint.trim().starts_with("http://127.0.0.1:"))
        && !cloud.llm_model.trim().is_empty()
        && cloud.encrypted_api_key_b64.as_deref().is_some_and(|key| !key.trim().is_empty())
}

/// "https://api.groq.com/openai/v1/chat/completions" -> "api.groq.com".
fn host_of(endpoint: &str) -> &str {
    let rest = endpoint.trim().split_once("://").map_or(endpoint.trim(), |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textjob::{Instruction, TextJob, TextJobKind};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn fully_configured() -> CloudSettings {
        CloudSettings {
            llm_endpoint: "https://api.groq.com/openai/v1/chat/completions".into(),
            llm_model: "openai/gpt-oss-20b".into(),
            encrypted_api_key_b64: Some("c2stbGl2ZQ==".into()),
            ..CloudSettings::default()
        }
    }

    #[test]
    fn the_offline_profile_stays_local_with_a_fully_configured_cloud() {
        let r = route(ProcessingProfile::MaxPrivacyOffline, true, &fully_configured(), Some("Qwen3.5 2B"));
        assert!(!r.cloud);
        assert!(r.allowed);
        assert_eq!(r.label, "On this PC · Qwen3.5 2B");
    }

    #[test]
    fn no_job_kind_carries_a_cloud_target_under_the_offline_profile() {
        let cloud = fully_configured();
        let r = route(ProcessingProfile::MaxPrivacyOffline, true, &cloud, Some("Qwen3.5 2B"));
        for kind in [TextJobKind::QuickFix, TextJobKind::DeepFix, TextJobKind::Compose, TextJobKind::VoiceCommand] {
            let job = TextJob {
                id: 1,
                kind,
                instruction: Instruction::Fix,
                source: String::new(),
                rich: false,
                target: Default::default(),
                max_output_tokens: 64,
                terms: Vec::new(),
                cancel: Arc::new(AtomicBool::new(false)),
                cloud: r.target(&cloud, "sk-live".into()),
                style_block: None,
            };
            assert!(job.cloud.is_none(), "{kind:?} must never run on the cloud under Maximum Privacy");
        }
    }

    #[test]
    fn a_configured_cloud_is_used_when_the_switch_is_on() {
        let cloud = fully_configured();
        let r = route(ProcessingProfile::Balanced, true, &cloud, Some("Qwen3.5 2B"));
        assert!(r.cloud && r.allowed);
        assert_eq!(r.label, "Cloud · api.groq.com · openai/gpt-oss-20b");
        let t = r.target(&cloud, "sk-live".into()).expect("cloud target");
        assert_eq!((t.endpoint.as_str(), t.model.as_str(), t.api_key.as_str()), (cloud.llm_endpoint.as_str(), "openai/gpt-oss-20b", "sk-live"));
        assert!(!format!("{t:?}").contains("sk-live"), "the key never shows in Debug");
    }

    #[test]
    fn a_plain_http_endpoint_keeps_the_job_local() {
        let mut cloud = fully_configured();
        cloud.llm_endpoint = "http://192.168.1.4/v1/chat/completions".into();
        assert!(!route(ProcessingProfile::Balanced, true, &cloud, Some("Qwen3.5 2B")).cloud);
        cloud.llm_endpoint = "http://127.0.0.1:8080/v1/chat/completions".into();
        assert!(route(ProcessingProfile::Balanced, true, &cloud, Some("Qwen3.5 2B")).cloud, "the loopback is allowed");
    }

    #[test]
    fn the_cloud_needs_the_switch_and_a_key() {
        let cloud = fully_configured();
        for (allow, key) in [(false, Some("c2stbGl2ZQ==")), (true, None), (true, Some("  "))] {
            let mut c = cloud.clone();
            c.encrypted_api_key_b64 = key.map(str::to_string);
            let r = route(ProcessingProfile::Balanced, allow, &c, Some("Qwen3.5 2B"));
            assert!(!r.cloud && r.allowed, "allow_cloud={allow}");
        }
    }

    #[test]
    fn nothing_to_run_with_reports_not_allowed() {
        let r = route(ProcessingProfile::MaxPrivacyOffline, false, &CloudSettings::default(), None);
        assert!(!r.allowed && !r.cloud);
        assert_eq!(r.label, "On this PC · built-in model");
    }

    #[test]
    fn host_comes_from_the_endpoint() {
        assert_eq!(host_of("https://api.groq.com/openai/v1/chat/completions"), "api.groq.com");
        assert_eq!(host_of("api.example.com/v1"), "api.example.com");
        assert_eq!(host_of("  "), "");
    }
}