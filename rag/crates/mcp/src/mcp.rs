//! MCP `search` face — one Streamable-HTTP tool, `search_ep_committee_docs`.
//!
//! A thin `rmcp` `ServerHandler` over the shared [`Router`]: the tool calls
//! [`Router::retrieve_grounded`] and returns the grounded-context block as a
//! text content block. Mounted at `/mcp` on the same axum listener as the HTTP
//! faces (see `main.rs`). This is a pure network seam — no unit tests; the live
//! MCP handshake is a manual deploy-time step (needs Qdrant + llama-server).
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};

use crate::router::Router;

/// Arguments for `search_ep_committee_docs`. The JSON Schema is derived by
/// `schemars` and advertised to the MCP host.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    #[schemars(description = "The user's information need (a natural-language question).")]
    pub query: String,
}

/// The MCP tool server. Holds the shared [`Router`] (retriever + manifest) plus
/// the macro-generated tool router. Cloned per session by the service factory.
#[derive(Clone)]
pub struct EpTools {
    router: Arc<Router>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EpTools {
    /// Build the tool server around a shared [`Router`].
    pub fn new(router: Arc<Router>) -> Self {
        Self {
            router,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search European Parliament committee documents (EMPL, REGI, IMCO) for \
grounded, cited passages. Call ONLY for substantive questions about EP committee document \
content/positions/deadlines. Returns labelled passages, a grounding instruction, and Sources."
    )]
    // Returns `Result<String, String>` so a retrieval failure (Qdrant/llama-server down)
    // surfaces as a protocol-level tool error (`is_error: true`) the host can handle, rather
    // than a success payload the model might quote as if it were retrieved content.
    // NOTE: the returned block carries the grounding instructions ("cite [id]", "I don't know").
    // Over MCP those land inside a *tool result* fed to a third-party host model, which is under
    // no obligation to obey them — grounding fidelity here depends on the external host.
    async fn search_ep_committee_docs(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<String, String> {
        self.router
            .retrieve_grounded(&args.query)
            .await
            .map(|(context, _hits)| context)
            .map_err(|e| format!("retrieval error: {e}"))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EpTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("EP committee documents — grounded, cited retrieval.")
    }
}
