use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};
use ep_rag_generate::prompt::{assemble, strip_think};
use futures::stream::{self, StreamExt};
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use crate::{openai::*, AppState};

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Response {
    let question = match req.latest_user_message() {
        Some(q) if !q.trim().is_empty() => q.to_string(),
        // Client error: the request carried no usable user turn (not an upstream fault).
        _ => return bad_request("no user message in request"),
    };

    let hits = match state.retriever.retrieve(&question, state.cfg.top_k).await {
        Ok(h) => h,
        Err(e) => return error_response(&format!("retrieve failed: {e}")),
    };
    let (system, user) = assemble(&question, &hits);
    let cited: Vec<String> = hits.iter().map(|h| h.citation_id.clone()).collect();

    // Streaming only when NOT stripping think (delta-wise strip needs a straddling buffer;
    // STRIP_THINK falls back to buffered non-streaming). See Task 3.5.
    // CAVEAT: a `stream:true` client under STRIP_THINK=1 gets a JSON body, not SSE — a
    // strict OpenAI client that requested text/event-stream may mishandle it. Off by
    // default (current model emits no <think>); revisit if a ThinkingCap model lands.
    let want_stream = req.stream && !state.cfg.strip_think;
    if want_stream {
        return stream_answer(state, system, user, cited).await;
    }

    let mut answer = match state.generator.complete(&state.upstream_model, &system, &user).await {
        Ok(a) => a,
        Err(e) => return error_response(&format!("generate failed: {e}")),
    };
    if state.cfg.strip_think {
        answer = strip_think(&answer);
    }
    // Attach Sources only for real, grounded answers — never on the "I don't know"
    // non-answer (a Sources block there would falsely imply it was sourced).
    let cited_refs: Vec<&str> = cited.iter().map(String::as_str).collect();
    let sources = state.manifest.sources_block(&cited_refs);
    if !sources.is_empty() && !is_no_answer(&answer) {
        answer = format!("{answer}\n\n{sources}");
    }
    Json(completion_response(&state.cfg.model_id, &answer)).into_response()
}

/// The grounding SYSTEM prompt makes the model reply exactly `I don't know` when the
/// context does not support an answer. Detect that non-answer (case-insensitive, with a
/// trailing period tolerated) so callers can suppress the Sources block.
fn is_no_answer(answer: &str) -> bool {
    answer
        .trim()
        .trim_end_matches('.')
        .trim()
        .eq_ignore_ascii_case("i don't know")
}

/// 502 — an upstream (retrieve/generate) dependency failed.
fn error_response(msg: &str) -> Response {
    status_error(axum::http::StatusCode::BAD_GATEWAY, msg)
}

/// 400 — the client's request was malformed (e.g. no user message).
fn bad_request(msg: &str) -> Response {
    status_error(axum::http::StatusCode::BAD_REQUEST, msg)
}

fn status_error(status: axum::http::StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": {"message": msg}}))).into_response()
}

async fn stream_answer(
    state: Arc<AppState>,
    system: String,
    user: String,
    cited: Vec<String>,
) -> Response {
    let model = state.cfg.model_id.clone();

    let upstream = match state.generator.complete_stream(&state.upstream_model, &system, &user).await {
        Ok(s) => s,
        Err(e) => return error_response(&format!("generate stream failed: {e}")),
    };

    // Shared outcome threaded out of the delta stream: `errored` marks a mid-stream
    // upstream failure; `accumulated` collects the answer text so the lazy tail can tell
    // an "I don't know" non-answer from a real one. Both are observed AFTER the deltas
    // are exhausted (the tail runs via `stream::once`, chained after the delta stream).
    let errored = Arc::new(AtomicBool::new(false));
    let accumulated = Arc::new(Mutex::new(String::new()));

    let model_d = model.clone();
    let errored_d = errored.clone();
    let acc_d = accumulated.clone();
    let deltas = upstream.filter_map(move |item| {
        let m = model_d.clone();
        let errored_d = errored_d.clone();
        let acc_d = acc_d.clone();
        async move {
            // Once interrupted, stop forwarding any further deltas.
            if errored_d.load(Ordering::SeqCst) {
                return None;
            }
            match item {
                Ok(text) => {
                    acc_d.lock().unwrap().push_str(&text);
                    Some(Ok::<Event, Infallible>(
                        Event::default().data(stream_chunk(&m, &text).to_string()),
                    ))
                }
                Err(_) => {
                    // Mark the truncation and emit ONE in-band notice, then go quiet.
                    errored_d.store(true, Ordering::SeqCst);
                    Some(Ok(Event::default().data(
                        stream_chunk(&m, "\n\n_[generation interrupted — answer may be incomplete]_")
                            .to_string(),
                    )))
                }
            }
        }
    });

    // Lazy tail: runs only once the delta stream is exhausted, so it sees the final flag
    // + accumulated state. Interrupted ⇒ just close (no Sources, no normal stop chunk).
    let model_t = model.clone();
    let state_t = state.clone();
    let cited_t = cited.clone();
    let errored_t = errored.clone();
    let acc_t = accumulated.clone();
    let tail = stream::once(async move {
        let mut items: Vec<Result<Event, Infallible>> = Vec::new();
        if errored_t.load(Ordering::SeqCst) {
            items.push(Ok(Event::default().data("[DONE]")));
        } else {
            let answer = acc_t.lock().unwrap();
            let cited_refs: Vec<&str> = cited_t.iter().map(String::as_str).collect();
            let sources = state_t.manifest.sources_block(&cited_refs);
            if !sources.is_empty() && !is_no_answer(answer.as_str()) {
                items.push(Ok(Event::default().data(
                    stream_chunk(&model_t, &format!("\n\n{sources}")).to_string(),
                )));
            }
            items.push(Ok(Event::default().data(stream_final(&model_t).to_string())));
            items.push(Ok(Event::default().data("[DONE]")));
        }
        stream::iter(items)
    })
    .flatten();

    let body = deltas.chain(tail);
    Sse::new(body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_no_answer_matches_the_grounding_refusal() {
        assert!(is_no_answer("I don't know"));
        assert!(is_no_answer("I don't know."));
        assert!(is_no_answer("  i don't know  "));
        assert!(!is_no_answer("The deadline is four months [EMPL-PR-785214:1]."));
    }
}
