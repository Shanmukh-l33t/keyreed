//! Streaming chat completions (server-sent events) for text jobs, local llama-server or a cloud
//! endpoint, with a cancel flag and a per-job output limit. Text is never logged.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How a stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// The server sent `[DONE]` or closed the stream.
    Finished,
    /// The caller's cancel flag was set; the connection is closed, which llama-server treats as stop.
    Cancelled,
    /// The output limit was reached; the connection is closed.
    Limit,
}

#[derive(Debug, Clone)]
pub struct StreamOutcome {
    pub text: String,
    pub end: StreamEnd,
}

/// One server-sent-event line, parsed.
#[derive(Debug, PartialEq, Eq)]
enum SseLine {
    Token(String),
    Done,
    Skip,
}

fn parse_sse_line(line: &str) -> SseLine {
    let Some(data) = line.strip_prefix("data:") else { return SseLine::Skip };
    let data = data.trim();
    if data == "[DONE]" {
        return SseLine::Done;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { return SseLine::Skip };
    match v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()) {
        Some(t) if !t.is_empty() => SseLine::Token(t.to_string()),
        _ => SseLine::Skip,
    }
}

/// Only HTTPS, or plain HTTP to the loopback llama-server, may carry text.
fn check_url(url: &str) -> Result<()> {
    if url.starts_with("https://") || url.starts_with("http://127.0.0.1:") {
        Ok(())
    } else {
        bail!("text jobs need an HTTPS endpoint (or the local model)")
    }
}

/// Posts `body` (an OpenAI-compatible chat request; `stream: true` is added) to `url` and calls
/// `on_token` for every piece of text as it arrives. Returns within about 30 ms of `cancel` being
/// set. At most `max_chars` characters are returned.
pub fn stream_chat(
    url: &str,
    bearer: Option<&str>,
    mut body: serde_json::Value,
    max_chars: usize,
    read_timeout: Duration,
    cancel: &AtomicBool,
    mut on_token: impl FnMut(&str),
) -> Result<StreamOutcome> {
    check_url(url)?;
    body["stream"] = serde_json::Value::Bool(true);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(read_timeout)
        .build();
    let mut req = agent.post(url).set("Content-Type", "application/json");
    if let Some(key) = bearer {
        req = req.set("Authorization", &format!("Bearer {}", key.trim()));
    }
    let resp = req.send_json(body).context("text job request failed")?;

    // The read blocks until the next event, so it runs on its own thread; this thread watches the
    // cancel flag. `stop` tells the reader to drop the connection when we return for any reason.
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<String>>(64);
    let reader_stop = stop.clone();
    std::thread::Builder::new()
        .name("keyreed-stream-read".to_string())
        .spawn(move || {
            let reader = BufReader::new(resp.into_reader());
            for line in reader.lines() {
                if reader_stop.load(Ordering::Relaxed) || tx.send(line).is_err() {
                    return;
                }
            }
        })
        .context("failed to start the stream reader")?;
    struct StopOnDrop(Arc<AtomicBool>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _stop_guard = StopOnDrop(stop);

    let mut text = String::new();
    let mut chars = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(StreamOutcome { text, end: StreamEnd::Cancelled });
        }
        let line = match rx.recv_timeout(Duration::from_millis(30)) {
            Ok(line) => line.context("text job stream broke off")?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(StreamOutcome { text, end: StreamEnd::Finished })
            }
        };
        match parse_sse_line(&line) {
            SseLine::Token(t) => {
                let room = max_chars.saturating_sub(chars);
                let n = t.chars().count();
                if n >= room {
                    let cut: String = t.chars().take(room).collect();
                    on_token(&cut);
                    text.push_str(&cut);
                    return Ok(StreamOutcome { text, end: StreamEnd::Limit });
                }
                chars += n;
                on_token(&t);
                text.push_str(&t);
            }
            SseLine::Done => return Ok(StreamOutcome { text, end: StreamEnd::Finished }),
            SseLine::Skip => {}
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn parses_recorded_llama_server_lines() {
        // Recorded shapes from llama-server and OpenAI-compatible endpoints.
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#),
            SseLine::Token("Hello".into())
        );
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#), SseLine::Skip);
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{"content":""},"finish_reason":"stop"}]}"#), SseLine::Skip);
        assert_eq!(parse_sse_line("data: [DONE]"), SseLine::Done);
        assert_eq!(parse_sse_line(": keep-alive"), SseLine::Skip);
        assert_eq!(parse_sse_line(""), SseLine::Skip);
        assert_eq!(parse_sse_line("data: {not json"), SseLine::Skip);
    }

    #[test]
    fn refuses_plain_http_to_other_hosts() {
        assert!(check_url("http://example.com/v1/chat/completions").is_err());
        assert!(check_url("http://localhost:8080/v1").is_err());
        assert!(check_url("https://api.example.com/v1/chat/completions").is_ok());
        assert!(check_url("http://127.0.0.1:5000/v1/chat/completions").is_ok());
    }

    /// A one-request SSE server: sends `tokens`, one every `gap`, then `[DONE]`.
    pub(crate) fn serve(tokens: Vec<&'static str>, gap: Duration) -> (String, std::thread::JoinHandle<()>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", l.local_addr().unwrap().port());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 8192];
            // Read the whole request (headers and body may arrive separately): closing a socket with unread
            // data makes Windows send a reset, which the client then sees as a broken stream.
            let mut req = Vec::new();
            while let Ok(n @ 1..) = s.read(&mut buf) {
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_ascii_lowercase();
                let Some(end) = text.find("\r\n\r\n") else { continue };
                let len = text[..end]
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if req.len() >= end + 4 + len {
                    break;
                }
            }
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
            for t in tokens {
                std::thread::sleep(gap);
                let line = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{t}\"}}}}]}}\n\n");
                if s.write_all(line.as_bytes()).is_err() {
                    return; // client closed: generation stops
                }
            }
            let _ = s.write_all(b"data: [DONE]\n\n");
        });
        (url, h)
    }

    #[test]
    fn streams_tokens_in_order() {
        let (url, h) = serve(vec!["Hi", " there", "."], Duration::from_millis(5));
        let mut seen = Vec::new();
        let out = stream_chat(&url, None, serde_json::json!({}), 1000, Duration::from_secs(5), &AtomicBool::new(false), |t| {
            seen.push(t.to_string())
        })
        .unwrap();
        assert_eq!(out.text, "Hi there.");
        assert_eq!(out.end, StreamEnd::Finished);
        assert_eq!(seen, ["Hi", " there", "."]);
        h.join().unwrap();
    }

    #[test]
    fn output_limit_cuts_the_stream() {
        let (url, h) = serve(vec!["abcd", "efgh", "ijkl"], Duration::from_millis(5));
        let out = stream_chat(&url, None, serde_json::json!({}), 6, Duration::from_secs(5), &AtomicBool::new(false), |_| {})
            .unwrap();
        assert_eq!(out.text, "abcdef");
        assert_eq!(out.end, StreamEnd::Limit);
        h.join().unwrap();
    }

    #[test]
    fn cancel_returns_within_target() {
        // A slow server: one token every 400 ms. Cancel must return well inside 150 ms.
        let (url, h) = serve(vec!["a", "b", "c", "d"], Duration::from_millis(400));
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            c2.store(true, Ordering::Relaxed);
            Instant::now()
        });
        let out = stream_chat(&url, None, serde_json::json!({}), 1000, Duration::from_secs(5), &cancel, |_| {}).unwrap();
        let returned = Instant::now();
        let set_at = setter.join().unwrap();
        assert_eq!(out.end, StreamEnd::Cancelled);
        assert!(returned.duration_since(set_at) < Duration::from_millis(150));
        h.join().unwrap();
    }
}
