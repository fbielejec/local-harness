//! llama-server generation client (streaming + non-streaming).

use anyhow::{Context, Result};
use futures::stream::Stream;
use serde_json::json;
use std::collections::VecDeque;

/// Incremental SSE frame parser. Owns a **raw-byte** buffer so that a multi-byte
/// UTF-8 codepoint (é, ü, ç, —, “ ” — dense in the EP corpus) whose bytes straddle a
/// network-chunk boundary is NOT corrupted: decoding each chunk with
/// `from_utf8_lossy` independently would emit U+FFFD for the split codepoint's two
/// halves and destroy the character before it reached the line buffer. Instead we
/// buffer bytes, split on the `\n` byte, and lossy-decode each *complete* line — a
/// full SSE line never splits a codepoint, so per-line decode is safe.
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    /// Feed one raw network chunk; return the content deltas of any now-complete lines.
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            if let Some(delta) = parse_sse_line(&line) {
                out.push(delta);
            }
        }
        out
    }

    /// End of stream: parse any residual complete line the server left without a
    /// trailing newline (a final `data:` delta would otherwise be silently dropped).
    fn flush(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            if let Some(delta) = parse_sse_line(&self.buf) {
                out.push(delta);
            }
            self.buf.clear();
        }
        out
    }
}

/// Parse one SSE line (`data: {json}`) into its `choices[0].delta.content`, if present.
/// Non-`data:` lines, `[DONE]`, and unparseable/contentless frames yield `None`
/// (silently skipped — a `debug!` trace would go here if a logger were pinned).
fn parse_sse_line(bytes: &[u8]) -> Option<String> {
    let line = String::from_utf8_lossy(bytes);
    let data = line.trim().strip_prefix("data:")?.trim();
    if data == "[DONE]" || data.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    let s = v["choices"][0]["delta"]["content"].as_str()?;
    (!s.is_empty()).then(|| s.to_string())
}

/// `unfold` state for `complete_stream`: the byte stream, the frame parser, a queue of
/// deltas ready to yield, and an end-of-stream flag (so the terminal flush runs once).
struct StreamState<S> {
    inner: S,
    parser: SseParser,
    pending: VecDeque<Result<String>>,
    done: bool,
}

/// Client for the OpenAI-compatible llama-server (the generator).
pub struct GenClient {
    base_url: String, // e.g. http://localhost:8080/v1
    http: reqwest::Client,
}

impl GenClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), http: reqwest::Client::new() }
    }

    /// The gguf model id llama-server advertises (notebook: MODEL = /models[0].id).
    pub async fn model_id(&self) -> Result<String> {
        let v: serde_json::Value = self
            .http
            .get(format!("{}/models", self.base_url))
            .send().await?.error_for_status()?.json().await?;
        v["data"][0]["id"].as_str().map(str::to_owned).context("no model id from llama-server")
    }

    fn body(&self, model: &str, system: &str, user: &str, stream: bool) -> serde_json::Value {
        json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0.0,
            "max_tokens": 400,
            "stream": stream
        })
    }

    /// Non-streaming completion → the answer text.
    pub async fn complete(&self, model: &str, system: &str, user: &str) -> Result<String> {
        let v: serde_json::Value = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&self.body(model, system, user, false))
            .send().await?.error_for_status()?.json().await?;
        v["choices"][0]["message"]["content"]
            .as_str().map(str::to_owned).context("no content in completion")
    }

    /// Warm-on-boot ping (design §latency): a tiny request to fault the model in.
    pub async fn warm(&self, model: &str) -> Result<()> {
        let _ = self.complete(model, "ping", "ping").await?;
        Ok(())
    }

    /// Streaming completion → a stream of answer-text deltas (already extracted from
    /// the upstream SSE). Errors in the stream surface as `Err`.
    pub async fn complete_stream(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<impl Stream<Item = Result<String>>> {
        use futures::StreamExt;
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&self.body(model, system, user, true))
            .send().await?.error_for_status()?;

        // Box::pin the byte stream so the `unfold` state is `Unpin` (we call `.next()`).
        let state = StreamState {
            inner: Box::pin(resp.bytes_stream()),
            parser: SseParser::default(),
            pending: VecDeque::new(),
            done: false,
        };

        let stream = futures::stream::unfold(state, |mut st| async move {
            loop {
                if let Some(item) = st.pending.pop_front() {
                    return Some((item, st));
                }
                if st.done {
                    return None;
                }
                match st.inner.next().await {
                    Some(Ok(bytes)) => {
                        st.pending.extend(st.parser.feed(&bytes).into_iter().map(Ok));
                    }
                    Some(Err(e)) => st.pending.push_back(Err(anyhow::anyhow!(e))),
                    None => {
                        // Stream closed: flush a trailing newline-less `data:` line, then stop.
                        st.pending.extend(st.parser.flush().into_iter().map(Ok));
                        st.done = true;
                    }
                }
            }
        });
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_preserves_utf8_split_across_chunks() {
        // The em-dash — is U+2014 = bytes E2 80 94. Split the byte stream inside it so
        // the old per-chunk `from_utf8_lossy` would have emitted U+FFFD for both halves.
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"a—b\"}}]}\n";
        let bytes = line.as_bytes();
        let split = line.find('—').unwrap() + 1; // land between the em-dash's bytes
        let mut p = SseParser::default();
        let mut out = p.feed(&bytes[..split]);
        assert!(out.is_empty()); // no newline yet -> nothing emitted mid-codepoint
        out.extend(p.feed(&bytes[split..]));
        out.extend(p.flush());
        assert_eq!(out, vec!["a—b".to_string()]);
    }

    #[test]
    fn framing_flushes_trailing_line_without_newline() {
        let mut p = SseParser::default();
        // A final delta the server sent without a trailing '\n' before closing.
        let mid = p.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}");
        assert!(mid.is_empty());
        assert_eq!(p.flush(), vec!["hi".to_string()]);
    }

    #[test]
    fn framing_skips_done_and_emits_multiple_deltas() {
        let mut p = SseParser::default();
        let out = p.feed(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\
              data: {\"choices\":[{\"delta\":{\"content\":\"y\"}}]}\n\
              data: [DONE]\n",
        );
        assert_eq!(out, vec!["x".to_string(), "y".to_string()]);
        assert!(p.flush().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires the llama-server tunnel at :8080"]
    async fn complete_answers_from_context() {
        let c = GenClient::new("http://localhost:8080/v1");
        let model = c.model_id().await.unwrap();
        let out = c.complete(&model, crate::prompt::SYSTEM,
            "Context:\n\n[X:1]\nThe deadline is four months.\n\n---\nQuestion: What is the deadline?")
            .await.unwrap();
        assert!(out.to_lowercase().contains("four months"));
    }

    #[tokio::test]
    #[ignore = "requires the llama-server tunnel at :8080"]
    async fn complete_stream_yields_deltas() {
        use futures::StreamExt;
        let c = GenClient::new("http://localhost:8080/v1");
        let model = c.model_id().await.unwrap();
        let mut s = Box::pin(c.complete_stream(&model, "You are terse.", "Say: four months.").await.unwrap());
        let mut acc = String::new();
        while let Some(item) = s.next().await { acc.push_str(&item.unwrap()); }
        assert!(!acc.is_empty());
    }
}
