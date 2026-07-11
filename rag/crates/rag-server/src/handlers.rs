use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};
use ep_rag_generate::prompt::{assemble, strip_think};
use futures::stream::{self, StreamExt};
use std::convert::Infallible;
use std::sync::Arc;
use crate::{openai::*, AppState};

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Response {
    let question = match req.latest_user_message() {
        Some(q) if !q.trim().is_empty() => q.to_string(),
        _ => return error_response("no user message in request"),
    };

    let hits = match state.retriever.retrieve(&question, state.cfg.top_k).await {
        Ok(h) => h,
        Err(e) => return error_response(&format!("retrieve failed: {e}")),
    };
    let (system, user) = assemble(&question, &hits);
    let cited: Vec<String> = hits.iter().map(|h| h.citation_id.clone()).collect();

    // Streaming only when NOT stripping think (delta-wise strip needs a straddling buffer;
    // STRIP_THINK falls back to buffered non-streaming). See Task 3.5.
    let want_stream = req.stream && !state.cfg.strip_think;
    if want_stream {
        return stream_answer(state, system, user, cited).await;
    }

    let mut answer = match state.gen.complete(&state.upstream_model, &system, &user).await {
        Ok(a) => a,
        Err(e) => return error_response(&format!("generate failed: {e}")),
    };
    if state.cfg.strip_think {
        answer = strip_think(&answer);
    }
    let cited_refs: Vec<&str> = cited.iter().map(String::as_str).collect();
    let sources = state.manifest.sources_block(&cited_refs);
    if !sources.is_empty() {
        answer = format!("{answer}\n\n{sources}");
    }
    Json(completion_response(&state.cfg.model_id, &answer)).into_response()
}

fn error_response(msg: &str) -> Response {
    (axum::http::StatusCode::BAD_GATEWAY,
     Json(serde_json::json!({"error": {"message": msg}}))).into_response()
}

// A mid-stream upstream error currently ends the answer (deltas after the error are
// dropped); the final `stop` chunk + `[DONE]` still close the SSE stream cleanly.
async fn stream_answer(
    state: Arc<AppState>,
    system: String,
    user: String,
    cited: Vec<String>,
) -> Response {
    let model = state.cfg.model_id.clone();

    let upstream = match state.gen.complete_stream(&state.upstream_model, &system, &user).await {
        Ok(s) => s,
        Err(e) => return error_response(&format!("generate stream failed: {e}")),
    };

    let model_for_deltas = model.clone();
    let deltas = upstream.filter_map(move |item| {
        let m = model_for_deltas.clone();
        async move {
            match item {
                Ok(text) => Some(Ok::<Event, Infallible>(
                    Event::default().data(stream_chunk(&m, &text).to_string()),
                )),
                Err(_) => None,
            }
        }
    });

    let cited_refs: Vec<&str> = cited.iter().map(String::as_str).collect();
    let sources = state.manifest.sources_block(&cited_refs);
    let mut tail: Vec<Result<Event, Infallible>> = Vec::new();
    if !sources.is_empty() {
        tail.push(Ok(Event::default().data(
            stream_chunk(&model, &format!("\n\n{sources}")).to_string(),
        )));
    }
    tail.push(Ok(Event::default().data(stream_final(&model).to_string())));
    tail.push(Ok(Event::default().data("[DONE]")));

    let body = deltas.chain(stream::iter(tail));
    Sse::new(body).into_response()
}
