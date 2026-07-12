mod config;
mod openai;
mod handlers;

use anyhow::{Context, Result};
use axum::{routing::{get, post}, Router, Json};
use ep_rag_generate::{client::GenClient, provenance::Manifest};
use ep_rag_retrieve::Retriever;
use std::sync::Arc;

pub struct AppState {
    pub retriever: Retriever,
    pub generator: GenClient,
    pub manifest: Manifest,
    pub upstream_model: String,
    pub cfg: config::Config,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config::Config::from_env();
    eprintln!("rag-server config: {cfg:?}");

    let retriever = Retriever::new(&cfg.qdrant_url).context("init retriever")?;

    // CONTRACT ASSERT (the one hard constraint): the LIVE embedder recipe == the index's
    // stored recipe. Assert against `retriever.contract()` (the live contract), not
    // `default()`, so the check means "live == stored" rather than "default == stored".
    let stored = retriever.any_stored_payload().await.context("read stored contract")?;
    let stored_map: std::collections::BTreeMap<_, _> = stored.into_iter().collect();
    retriever
        .contract()
        .assert_matches(&stored_map)
        .map_err(|e| anyhow::anyhow!(e))
        .context("startup contract check")?;
    eprintln!("contract OK — live embedder matches the index");

    let generator = GenClient::new(&cfg.gen_base_url);
    let upstream_model = generator.model_id().await.context("resolve llama-server model")?;
    eprintln!("generator model: {upstream_model}");
    if let Err(e) = generator.warm(&upstream_model).await {
        eprintln!("warm ping failed (non-fatal): {e}");
    }

    let manifest = Manifest::load(&cfg.manifest_path).context("load manifest")?;

    let state = Arc::new(AppState { retriever, generator, manifest, upstream_model, cfg: cfg.clone() });

    let app = Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(handlers::chat_completions))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await
        .with_context(|| format!("bind {}", cfg.bind_addr))?;
    eprintln!("rag-server listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// OpenAI-compatible model list — advertises OUR model id (not the gguf) to OWUI.
async fn models(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "object": "list",
        "data": [{"id": state.cfg.model_id, "object": "model", "owned_by": "ep-rag"}]
    }))
}
