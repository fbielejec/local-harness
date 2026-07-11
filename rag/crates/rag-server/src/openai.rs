//! OpenAI-compatible chat request/response shapes + the latest-user extraction.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    #[serde(default)]
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
}

impl ChatRequest {
    /// The question = the content of the last `user` turn.
    pub fn latest_user_message(&self) -> Option<&str> {
        self.messages.iter().rev().find(|m| m.role == "user").map(|m| m.content.as_str())
    }
}

/// Build a non-streaming response body carrying `content`.
pub fn completion_response(model: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "ragcmpl-0",
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }]
    })
}

/// One streaming SSE chunk carrying a content delta.
pub fn stream_chunk(model: &str, delta: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "ragcmpl-0",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{"index": 0, "delta": {"content": delta}, "finish_reason": serde_json::Value::Null}]
    })
}

/// The final SSE chunk (finish_reason=stop, empty delta).
pub fn stream_final(model: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "ragcmpl-0",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_user_message_picks_last_user_turn() {
        let req: ChatRequest = serde_json::from_str(r#"{
            "model": "ep-committees-grounded",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": "What is the Youth Guarantee deadline?"}
            ],
            "stream": true
        }"#).unwrap();
        assert_eq!(req.latest_user_message().unwrap(), "What is the Youth Guarantee deadline?");
        assert!(req.stream);
    }

    #[test]
    fn latest_user_message_none_when_no_user_turn() {
        let req = ChatRequest { model: "m".into(), messages: vec![], stream: false };
        assert!(req.latest_user_message().is_none());
    }

    #[test]
    fn stream_chunk_has_delta_content() {
        let c = stream_chunk("m", "four");
        assert_eq!(c["choices"][0]["delta"]["content"], serde_json::json!("four"));
        assert_eq!(c["object"], serde_json::json!("chat.completion.chunk"));
    }
}
