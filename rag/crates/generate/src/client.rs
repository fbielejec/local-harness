//! llama-server generation client (streaming + non-streaming).

use anyhow::{Context, Result};
use futures::stream::Stream;
use serde_json::json;

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

        let mut buf = String::new();
        let stream = resp.bytes_stream().flat_map(move |chunk| {
            let mut out: Vec<Result<String>> = Vec::new();
            match chunk {
                Err(e) => out.push(Err(anyhow::anyhow!(e))),
                Ok(bytes) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(nl) = buf.find('\n') {
                        let line = buf[..nl].trim().to_string();
                        buf.drain(..=nl);
                        if let Some(data) = line.strip_prefix("data:") {
                            let data = data.trim();
                            if data == "[DONE]" || data.is_empty() { continue; }
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(s) = v["choices"][0]["delta"]["content"].as_str() {
                                    if !s.is_empty() { out.push(Ok(s.to_string())); }
                                }
                            }
                        }
                    }
                }
            }
            futures::stream::iter(out)
        });
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
