use anyhow::{Context, Result};
use ep_rag_generate::client::GenClient;
use ep_rag_generate::provenance::Manifest;
use ep_rag_retrieve::{Hit, Retriever};
use crate::{context::grounded_context, route::Route, tree::Tree};

pub struct Router {
    pub retriever: Retriever,
    pub generator: GenClient,
    pub tree: Tree,
    pub manifest: Manifest,
    pub gen_model: String,   // resolved llama-server gguf id
    pub top_k: u64,
}

/// The outcome `/route` returns and the OWUI filter consumes. The `/route` handler surfaces
/// only `{should_ground, route, context}`, so the grounded `hits` (returned by
/// `retrieve_grounded`, which `/retrieve` uses) are intentionally not carried here.
pub struct RouteOutcome {
    pub route: Route,
    pub context: Option<String>, // grounded block iff should_ground
}

impl Router {
    /// Classify the message (Mode-A), and if the route says ground, retrieve + assemble.
    pub async fn route(&self, message: &str) -> Result<RouteOutcome> {
        let (system, user) = self.tree.classify_prompt(message);
        let raw = self.generator.complete(&self.gen_model, &system, &user).await
            .context("classify call to llama-server")?;
        let route = Route::from_completion(&raw);
        if !route.should_ground() {
            return Ok(RouteOutcome { route, context: None });
        }
        // Delegate to the retrieval-only path so the two grounding sites can never diverge.
        let (context, _hits) = self.retrieve_grounded(message).await?;
        Ok(RouteOutcome { route, context: Some(context) })
    }

    /// Retrieval-only (for the MCP tool + /retrieve): always retrieve, always ground.
    pub async fn retrieve_grounded(&self, question: &str) -> Result<(String, Vec<Hit>)> {
        let hits = self.retriever.retrieve(question, self.top_k).await.context("qdrant retrieve")?;
        let context = grounded_context(question, &hits, &self.manifest);
        Ok((context, hits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn router() -> Router {
        let qurl = std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".into());
        let gurl = std::env::var("GEN_BASE_URL").unwrap_or_else(|_| "http://localhost:8080/v1".into());
        let generator = GenClient::new(&gurl);
        let gen_model = generator.model_id().await.unwrap();
        Router {
            retriever: Retriever::new(&qurl).unwrap(),
            generator, tree: Tree::load("data/route_tree.json").unwrap(),
            manifest: Manifest::load("data/manifest.jsonl").unwrap(),
            gen_model, top_k: 5,
        }
    }

    #[tokio::test]
    #[ignore = "requires live Qdrant + llama-server"]
    async fn ep_question_grounds_general_question_passes_through() {
        let r = router().await;
        let ep = r.route("What is the deadline under the Youth Guarantee?").await.unwrap();
        assert_eq!(ep.route, Route::UseEpRag);
        assert!(ep.context.as_deref().unwrap().contains("Sources:"));

        let chit = r.route("What's 2 + 2?").await.unwrap();
        assert_eq!(chit.route, Route::Unclassified);
        assert!(chit.context.is_none());
    }
}
