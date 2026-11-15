//! Optional OpenAI/Groq-compatible Cloud Speech-to-Text client.
//! Strictly gated: never invoked in `MaxPrivacyOffline` mode.
//! Builds an in-memory WAV buffer (`Zeroize`-backed) so raw audio never touches disk!

use anyhow::{bail, Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use serde::Deserialize;
use std::io::{Cursor, Read};
use std::time::Duration;
use zeroize::Zeroize;

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

pub fn encode_wav_in_memory(samples_16k: &[f32]) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(samples_16k.len() * 2 + 64));
    {
        let mut writer = WavWriter::new(&mut cursor, spec)?;
        for &s in samples_16k {
            let clamped = s.clamp(-1.0, 1.0);
            let pcm16 = (clamped * 32767.0) as i16;
            writer.write_sample(pcm16)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

pub fn transcribe_cloud_multipart(
    endpoint: &str,
    api_key: &str,
    model: &str,
    language: Option<&str>,
    prompt_hint: Option<&str>,
    samples_16k: &[f32],
) -> Result<String> {
    if !endpoint.starts_with("https://") {
        bail!("cloud STT endpoint must use HTTPS");
    }
    if api_key.trim().is_empty() {
        bail!("cloud STT API key is empty");
    }

    let mut wav_bytes = encode_wav_in_memory(samples_16k)?;
    let boundary = "----KeyreedAudioBoundary9f8e7d6c5b4a";
    let mut body: Vec<u8> = Vec::with_capacity(wav_bytes.len() + 1024);

    let mut add_field = |name: &str, val: &str| {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", name, val).as_bytes(),
        );
    };

    add_field("model", model);
    add_field("response_format", "json");
    if let Some(lang) = language {
        if !lang.is_empty() && lang != "auto" {
            add_field("language", lang);
        }
    }
    if let Some(hint) = prompt_hint {
        if !hint.is_empty() {
            add_field("prompt", hint);
        }
    }

    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"utterance.wav\"\r\nContent-Type: audio/wav\r\n\r\n",
    );
    body.extend_from_slice(&wav_bytes);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    wav_bytes.zeroize();

    let content_type = format!("multipart/form-data; boundary={}", boundary);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(4))
        .timeout_read(Duration::from_secs(12))
        .build();

    let res = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {}", api_key.trim()))
        .set("Content-Type", &content_type)
        .send_bytes(&body);

    body.zeroize();

    let resp = res.context("cloud STT HTTP request failed")?;
    let mut resp_str = String::new();
    resp.into_reader().read_to_string(&mut resp_str)?;
    let parsed: TranscriptionResponse =
        serde_json::from_str(&resp_str).context("invalid JSON from cloud STT")?;
    Ok(parsed.text.trim().to_string())
}
