use axum::{extract::State, Json};
use std::sync::Arc;
use crate::AppState;

pub async fn chat_completions(
    State(_state): State<Arc<AppState>>,
    Json(_req): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"error": "not implemented"}))
}
