//! Retrieve brick: embed the query under the contract PREFIX, then Qdrant top-k.
//! The productization of `rag_query.org`'s `embed_query` + `search` blocks.

use qdrant_client::qdrant::{value::Kind, Value as QValue};
use serde::Serialize;
use std::collections::HashMap;

/// Convert a single Qdrant value to serde_json (the kinds our payload uses).
fn qval_to_json(v: &QValue) -> serde_json::Value {
    use serde_json::Value as J;
    match &v.kind {
        Some(Kind::StringValue(s)) => J::from(s.clone()),
        Some(Kind::IntegerValue(i)) => J::from(*i),
        Some(Kind::DoubleValue(d)) => J::from(*d),
        Some(Kind::BoolValue(b)) => J::from(*b),
        Some(Kind::ListValue(l)) => J::Array(l.values.iter().map(qval_to_json).collect()),
        Some(Kind::StructValue(s)) => {
            J::Object(s.fields.iter().map(|(k, v)| (k.clone(), qval_to_json(v))).collect())
        }
        _ => J::Null,
    }
}

/// Convert a Qdrant payload map to a serde_json object map.
pub fn payload_to_json(p: &HashMap<String, QValue>) -> serde_json::Map<String, serde_json::Value> {
    p.iter().map(|(k, v)| (k.clone(), qval_to_json(v))).collect()
}

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

    #[test]
    fn qval_conversion_covers_string_int_bool() {
        use qdrant_client::qdrant::{value::Kind, Value};
        let mk = |kind| Value { kind: Some(kind) };
        let mut m = std::collections::HashMap::new();
        m.insert("citation_id".to_string(), mk(Kind::StringValue("X:1".into())));
        m.insert("chunk_index".to_string(), mk(Kind::IntegerValue(1)));
        m.insert("contract_normalized".to_string(), mk(Kind::BoolValue(true)));
        let j = payload_to_json(&m);
        assert_eq!(j["citation_id"], serde_json::json!("X:1"));
        assert_eq!(j["chunk_index"], serde_json::json!(1));
        assert_eq!(j["contract_normalized"], serde_json::json!(true));
    }
}
