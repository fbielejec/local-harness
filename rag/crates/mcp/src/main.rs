//! ep-rag-mcp: one loopback service, three faces — MCP `search` tool,
//! POST /retrieve, POST /route (Mode-A tree-classify → retrieve). See
//! docs/plans/2026-07-12-ep-rag-mcp-design.md.
mod config;
mod context;
mod mcp;
mod route;
mod router;
mod tree;

use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router as AxumRouter};
use ep_rag_generate::{client::GenClient, provenance::Manifest};
use ep_rag_retrieve::Retriever;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use route::Route;
use router::Router;
use std::sync::Arc;

type Shared = Arc<Router>;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config::Config::from_env();
    eprintln!("ep-rag-mcp config: {cfg:?}");

    // Fail-fast on bad config paths BEFORE the slow network/warm path.
    let tree = tree::Tree::load(&cfg.tree_path).context("load tree")?;
    let manifest = Manifest::load(&cfg.manifest_path).context("load manifest")?;
    eprintln!("route tree v{} loaded", tree.version);

    let retriever = Retriever::new(&cfg.qdrant_url).context("init retriever")?;

    // The one hard constraint: live embedder contract == index's stored contract.
    let stored = retriever.any_stored_payload().await.context("read stored contract")?;
    let stored_map: std::collections::BTreeMap<_, _> = stored.into_iter().collect();
    retriever
        .contract()
        .assert_matches(&stored_map)
        .map_err(|e| anyhow::anyhow!(e))
        .context("startup contract check")?;
    eprintln!("contract OK — live embedder matches the index");

    let generator = GenClient::new(&cfg.gen_base_url);
    let gen_model = generator.model_id().await.context("resolve llama-server model")?;
    if let Err(e) = generator.warm(&gen_model).await {
        eprintln!("warm ping failed (non-fatal): {e}");
    }

    let state: Shared = Arc::new(Router {
        retriever,
        generator,
        gen_model,
        top_k: cfg.top_k,
        tree,
        manifest,
    });

    // MCP `search` face: mount the rmcp Streamable-HTTP service at `/mcp`. The
    // factory clones the shared `Router` into a fresh `EpTools` per session.
    let mcp_router = state.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(mcp::EpTools::new(mcp_router.clone())),
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );

    let app = AxumRouter::new()
        .route("/retrieve", post(retrieve_handler))
        .route("/route", post(route_handler))
        .nest_service("/mcp", mcp_service)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .with_context(|| format!("bind {}", cfg.bind_addr))?;
    eprintln!("ep-rag-mcp listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// 502 — an upstream (classify/retrieve) dependency failed. Mirrors
/// `rag-server/src/handlers.rs`'s error envelope.
fn error_response(msg: &str) -> Response {
    status_error(StatusCode::BAD_GATEWAY, msg)
}

fn status_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": {"message": msg}}))).into_response()
}

/// The MVP uses the configured `top_k`; a per-request `k` is a later refinement (YAGNI).
/// serde ignores unknown fields, so a client that sends `k` is accepted harmlessly.
#[derive(serde::Deserialize)]
struct RetrieveReq {
    query: String,
}

async fn retrieve_handler(State(s): State<Shared>, Json(req): Json<RetrieveReq>) -> Response {
    match s.retrieve_grounded(&req.query).await {
        Ok((context, hits)) => {
            Json(serde_json::json!({ "context": context, "hits": hits })).into_response()
        }
        Err(e) => error_response(&e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct RouteReq {
    message: String,
}

async fn route_handler(State(s): State<Shared>, Json(req): Json<RouteReq>) -> Response {
    match s.route(&req.message).await {
        Ok(o) => {
            let route_id = match o.route {
                Route::UseEpRag => "R_USE_EP_RAG",
                Route::Unclassified => "R_UNCLASSIFIED",
            };
            Json(serde_json::json!({
                "should_ground": o.route.should_ground(),
                "route": route_id,
                "context": o.context,
            }))
            .into_response()
        }
        Err(e) => error_response(&e.to_string()),
    }
}
