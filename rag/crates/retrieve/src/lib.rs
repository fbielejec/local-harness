//! Retrieve brick: embed the query under the contract PREFIX, then Qdrant top-k.
//! The productization of `rag_query.org`'s `embed_query` + `search` blocks.

use serde::Serialize;

/// One retrieved chunk: what the generator cites and grounds on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub citation_id: String,
    pub doc_id: String,
    pub score: f32,
    pub text: String,
    pub title: String,
}

/// `doc_id` is the `citation_id` up to the `:` (chunk index). See `Chunk::citation_id`.
pub fn doc_id_of(citation_id: &str) -> &str {
    citation_id.split(':').next().unwrap_or(citation_id)
}

/// Build a `Hit` from a Qdrant payload (already converted to a serde_json map) + score.
/// Returns `None` if the two load-bearing fields (citation_id, text) are absent.
pub fn hit_from_payload(
    score: f32,
    payload: &serde_json::Map<String, serde_json::Value>,
) -> Option<Hit> {
    let s = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let citation_id = s("citation_id")?;
    let text = s("text")?;
    let doc_id = s("doc_id").unwrap_or_else(|| doc_id_of(&citation_id).to_owned());
    let title = s("title").unwrap_or_default();
    Some(Hit { citation_id, doc_id, score, text, title })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> serde_json::Map<String, serde_json::Value> {
        json!({
            "citation_id": "EMPL-PR-785214:3",
            "doc_id": "EMPL-PR-785214",
            "text": "Member States shall ensure a quality offer within four months.",
            "title": "DRAFT REPORT on the Youth Guarantee"
        }).as_object().unwrap().clone()
    }

    #[test]
    fn hit_from_payload_extracts_fields() {
        let h = hit_from_payload(0.728, &payload()).expect("well-formed payload -> Hit");
        assert_eq!(h.citation_id, "EMPL-PR-785214:3");
        assert_eq!(h.doc_id, "EMPL-PR-785214");
        assert!(h.text.starts_with("Member States"));
        assert!((h.score - 0.728).abs() < 1e-6);
    }

    #[test]
    fn hit_from_payload_derives_doc_id_when_missing() {
        let mut p = payload();
        p.remove("doc_id");
        let h = hit_from_payload(0.5, &p).unwrap();
        assert_eq!(h.doc_id, "EMPL-PR-785214");
    }

    #[test]
    fn hit_from_payload_rejects_missing_citation_or_text() {
        let mut p = payload();
        p.remove("text");
        assert!(hit_from_payload(0.5, &p).is_none());
    }
}
