//! llama-server generation client (streaming + non-streaming).

use anyhow::{Context, Result};
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
}
