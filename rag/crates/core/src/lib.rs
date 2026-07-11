//! Shared types for the EP-RAG pipeline.
//!
//! The [`EmbeddingContract`] is the cross-language anchor. The Rust ingestion
//! builds the index under it; the Python drills MUST reproduce it exactly (same
//! model, CLS pooling, query prefix, L2 norm) or every query looks like a
//! mismatch. It is stamped onto every Qdrant payload and asserted at serve time.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The exact conditions under which a vector was produced.
///
/// A retriever score is `d(z)·q(x)` — a dot product between a document vector and
/// a query vector, meaningful only if BOTH were produced identically. Drift in any
/// field (model / version / pooling / normalization / the query-only instruction
/// prefix) silently corrupts the cross term while each vector still looks fine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingContract {
    pub model: String,
    pub version: String,
    pub dim: usize,
    /// `"cls"` for bge-small-en-v1.5 — it uses CLS pooling, NOT mean. A hand-rolled
    /// embedder that mean-pools would silently diverge from the index.
    pub pooling: String,
    /// L2 normalization (BGE is cosine-trained).
    pub normalized: bool,
    /// Prepended to QUERIES only (never to passages).
    pub query_instruction: String,
}

impl Default for EmbeddingContract {
    /// The one contract the whole system pins to. Bump `version` on any knowing
    /// re-embed (new model, changed prefix, changed pooling).
    fn default() -> Self {
        Self {
            model: "BAAI/bge-small-en-v1.5".into(),
            version: "v1".into(),
            dim: 384,
            pooling: "cls".into(),
            normalized: true,
            query_instruction: "Represent this sentence for searching relevant passages:".into(),
        }
    }
}

impl EmbeddingContract {
    /// Flat `contract_*` map stamped onto each chunk's Qdrant payload.
    pub fn as_payload(&self) -> BTreeMap<String, serde_json::Value> {
        use serde_json::Value;
        let mut m = BTreeMap::new();
        m.insert("contract_model".into(), Value::from(self.model.clone()));
        m.insert("contract_version".into(), Value::from(self.version.clone()));
        m.insert("contract_dim".into(), Value::from(self.dim));
        m.insert("contract_pooling".into(), Value::from(self.pooling.clone()));
        m.insert("contract_normalized".into(), Value::from(self.normalized));
        m.insert(
            "contract_query_instruction".into(),
            Value::from(self.query_instruction.clone()),
        );
        m
    }

    /// Refuse to serve if the live contract != the index's stored contract.
    pub fn assert_matches(&self, stored: &BTreeMap<String, serde_json::Value>) -> Result<(), String> {
        let mine = self.as_payload();
        let drift: Vec<String> = mine
            .iter()
            .filter(|(k, v)| stored.get(*k) != Some(*v))
            .map(|(k, v)| format!("{k}: indexed={:?} live={v}", stored.get(k)))
            .collect();
        if drift.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Embedding-contract mismatch (index vs query) — refusing to serve. Drift: {drift:?}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_contract_is_bge_small_cls() {
        let c = EmbeddingContract::default();
        assert_eq!(c.dim, 384);
        assert_eq!(c.pooling, "cls"); // NOT mean — the correction from the research
        assert!(c.normalized);
        assert!(c.query_instruction.starts_with("Represent this sentence"));
    }

    #[test]
    fn matching_contract_passes_but_dropped_prefix_is_caught() {
        let c = EmbeddingContract::default();
        // matching stored contract -> OK
        assert!(c.assert_matches(&c.as_payload()).is_ok());
        // simulate the index built with an empty query prefix -> refuse (Drill 1)
        let mut tampered = c.as_payload();
        tampered.insert("contract_query_instruction".into(), serde_json::Value::from(""));
        assert!(c.assert_matches(&tampered).is_err());
    }

    #[test]
    fn contract_round_trips_through_json() {
        let c = EmbeddingContract::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: EmbeddingContract = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
