use crate::slm::build_mode_prefix;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;
use vi_core::{AppCategory, WritingMode};

#[derive(Deserialize)]
struct CloudChatResp {
    choices: Vec<CloudChoice>,
}

#[derive(Deserialize)]
struct CloudChoice {
    message: CloudMsg,
}

#[derive(Deserialize)]
struct CloudMsg {
    content: String,
}

/// Optional BYOK Cloud LLM rewriter (OpenAI-compatible `/v1/chat/completions` such as Groq, Cerebras, Gemini, OpenAI).
/// Strictly skipped in `MaxPrivacyOffline` mode.
pub fn rewrite_via_cloud(
    endpoint: &str,
    api_key: &str,
    model: &str,
    tier0_text: &str,
    mode: WritingMode,
    category: AppCategory,
    glossary: &str,
) -> Result<String> {
    if !endpoint.starts_with("https://") {
        bail!("cloud LLM endpoint must use HTTPS");
    }
    if api_key.trim().is_empty() {
        bail!("cloud LLM API key is empty");
    }

    let mut messages = build_mode_prefix(mode, category, glossary);
    messages.push(crate::slm::ChatMessage {
        role: "user",
        content: format!("<transcript>{}</transcript>", tier0_text),
    });

    let max_tokens = (tier0_text.split_whitespace().count() * 2 + 24).clamp(32, 512);
    let payload = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": 0.0,
        "max_tokens": max_tokens,
    });

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(6))
        .build();

    let resp: CloudChatResp = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {}", api_key.trim()))
        .set("Content-Type", "application/json")
        .send_json(payload)
        .context("cloud LLM HTTP call failed")?
        .into_json()?;

    Ok(resp
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .unwrap_or_default())
}
