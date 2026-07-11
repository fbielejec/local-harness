//! Serve-time provenance join: citation_id -> doc_id -> manifest pdf_url.

use anyhow::Result;
use ep_rag_retrieve::doc_id_of;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestEntry {
    pub doc_id: String,
    pub title: String,
    pub pdf_url: String,
}

/// doc_id -> (title, pdf_url), loaded from data/manifest.jsonl. The serve-time
/// provenance source (design option (b) — no re-index needed). Note: real manifest
/// lines carry extra fields; serde ignores unknown fields by default.
pub struct Manifest {
    by_doc: HashMap<String, ManifestEntry>,
}

impl Manifest {
    pub fn from_jsonl(s: &str) -> Result<Self> {
        let mut by_doc = HashMap::new();
        for line in s.lines().filter(|l| !l.trim().is_empty()) {
            let e: ManifestEntry = serde_json::from_str(line)?;
            by_doc.insert(e.doc_id.clone(), e);
        }
        Ok(Self { by_doc })
    }

    pub fn load(path: &str) -> Result<Self> {
        Self::from_jsonl(&std::fs::read_to_string(path)?)
    }

    /// Join a citation_id (doc_id:idx) to its manifest entry.
    pub fn lookup(&self, citation_id: &str) -> Option<&ManifestEntry> {
        self.by_doc.get(doc_id_of(citation_id))
    }

    /// Human-facing Sources block: one line per distinct source document, in first-seen
    /// order. Empty string if nothing was cited.
    ///
    /// MVP contract: the caller passes ALL retrieved citation ids (not just the ones the
    /// model actually cited). Parsing the `[cid]` markers the model emitted and listing
    /// only those is a deferred refinement.
    pub fn sources_block(&self, cited: &[&str]) -> String {
        let mut seen = Vec::new();
        for cid in cited {
            let doc = doc_id_of(cid);
            if !seen.iter().any(|d| d == &doc) {
                seen.push(doc);
            }
        }
        let lines: Vec<String> = seen
            .iter()
            .filter_map(|doc| self.by_doc.get(*doc))
            .map(|e| format!("- {} — {}", e.title, e.pdf_url))
            .collect();
        if lines.is_empty() {
            String::new()
        } else {
            format!("Sources:\n{}", lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        let jsonl = r#"{"doc_id":"EMPL-PR-785214","title":"Youth Guarantee","pdf_url":"https://ep/EMPL-PR-785214_en.pdf"}
{"doc_id":"IMCO-PR-773060","title":"EV supply equipment","pdf_url":"https://ep/IMCO-PR-773060_en.pdf"}"#;
        Manifest::from_jsonl(jsonl).unwrap()
    }

    #[test]
    fn manifest_joins_citation_to_pdf_url() {
        let m = manifest();
        let e = m.lookup("EMPL-PR-785214:3").unwrap();
        assert_eq!(e.title, "Youth Guarantee");
        assert_eq!(e.pdf_url, "https://ep/EMPL-PR-785214_en.pdf");
        assert!(m.lookup("NOPE-XX-000:1").is_none());
    }

    #[test]
    fn sources_block_dedupes_by_doc_and_lists_urls() {
        let m = manifest();
        let cited = ["EMPL-PR-785214:1", "EMPL-PR-785214:12", "IMCO-PR-773060:2"];
        let block = m.sources_block(&cited);
        assert!(block.starts_with("Sources:"));
        assert_eq!(block.matches("https://ep/EMPL-PR-785214_en.pdf").count(), 1);
        assert!(block.contains("https://ep/IMCO-PR-773060_en.pdf"));
        assert!(block.contains("Youth Guarantee"));
    }

    #[test]
    fn sources_block_empty_when_nothing_cited() {
        assert_eq!(manifest().sources_block(&[]), "");
    }
}
