//! Index brick: turn ingested chunks into Qdrant points under the embedding contract.
//!
//! The contract-critical, network-free core lives here and is unit-tested:
//!   * `point_id` — a DETERMINISTIC id per `citation_id`, so a re-index upserts in
//!     place instead of duplicating points.
//!   * `chunk_payload` — the stamped embedding contract (model·version·dim·pooling·
//!     normalized·query_instruction, the invariants) plus the provenance, lengths,
//!     and `text` the citation/generation path needs. Stamping the contract is what
//!     lets the serve path assert live == stored and REFUSE on drift (Drill 1).
//!
//! The live Qdrant work (collection create + batched upsert) is the bin.

use rag_chunk::Chunk;
use rag_core::EmbeddingContract;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Stable point id for a chunk: same `citation_id` -> same id across runs, so a
/// re-index overwrites rather than duplicates. Namespaced under the built-in URL
/// namespace to keep ids project-scoped.
pub fn point_id(citation_id: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("ep-rag/{citation_id}").as_bytes())
}

/// Qdrant payload for a chunk: the stamped embedding contract, plus provenance,
/// lengths, and the `text` (so retrieval needs no second lookup).
pub fn chunk_payload(chunk: &Chunk, contract: &EmbeddingContract) -> BTreeMap<String, Value> {
    // Start from the stamped contract (the six contract_* invariants), then layer
    // provenance, lengths, and the chunk text on top.
    let mut p = contract.as_payload();
    p.insert("citation_id".into(), Value::from(chunk.citation_id.clone()));
    p.insert("doc_id".into(), Value::from(chunk.doc_id.clone()));
    p.insert("committee".into(), Value::from(chunk.committee.clone()));
    p.insert("doc_type".into(), Value::from(chunk.doc_type.clone()));
    p.insert("title".into(), Value::from(chunk.title.clone()));
    p.insert("chunk_index".into(), Value::from(chunk.chunk_index));
    p.insert("n_chunks".into(), Value::from(chunk.n_chunks));
    p.insert("text".into(), Value::from(chunk.text.clone()));
    p.insert("char_len".into(), Value::from(chunk.char_len));
    p.insert("token_len".into(), Value::from(chunk.token_len));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunk() -> Chunk {
        Chunk {
            citation_id: "EMPL-PR-785214:3".into(),
            doc_id: "EMPL-PR-785214".into(),
            committee: "EMPL".into(),
            doc_type: "PR".into(),
            title: "DRAFT REPORT on the Youth Guarantee".into(),
            chunk_index: 3,
            n_chunks: 11,
            text: "Member States shall ensure a quality offer within four months.".into(),
            char_len: 61,
            token_len: 12,
        }
    }

    #[test]
    fn point_id_is_deterministic_and_distinct() {
        // same citation_id -> same id (idempotent re-index, no duplicate points)
        assert_eq!(point_id("EMPL-PR-785214:3"), point_id("EMPL-PR-785214:3"));
        // different chunk -> different id
        assert_ne!(point_id("EMPL-PR-785214:3"), point_id("EMPL-PR-785214:4"));
    }

    #[test]
    fn payload_carries_provenance_text_lengths_and_contract_stamp() {
        let c = sample_chunk();
        let p = chunk_payload(&c, &EmbeddingContract::default());

        // provenance + citation the generator needs
        assert_eq!(p["citation_id"], Value::from("EMPL-PR-785214:3"));
        assert_eq!(p["doc_id"], Value::from("EMPL-PR-785214"));
        assert_eq!(p["committee"], Value::from("EMPL"));
        assert_eq!(p["doc_type"], Value::from("PR"));
        assert_eq!(p["chunk_index"], Value::from(3usize));
        assert_eq!(p["n_chunks"], Value::from(11usize));

        // the chunk text rides in the payload (no second lookup at serve time)
        assert!(p.contains_key("text"), "chunk text must travel in the payload");
        // kept for observability
        assert_eq!(p["char_len"], Value::from(61usize));
        assert_eq!(p["token_len"], Value::from(12usize));

        // the six stamped contract_* invariants — Drill-1's serve-time assert compares these
        for k in [
            "contract_model",
            "contract_version",
            "contract_dim",
            "contract_pooling",
            "contract_normalized",
            "contract_query_instruction",
        ] {
            assert!(p.contains_key(k), "missing contract field {k}");
        }
        assert_eq!(p["contract_pooling"], Value::from("cls"));
    }
}
