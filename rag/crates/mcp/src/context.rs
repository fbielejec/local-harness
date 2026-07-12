use ep_rag_generate::prompt::{assemble, SYSTEM};
use ep_rag_generate::provenance::Manifest;
use ep_rag_retrieve::Hit;

/// The single host-agnostic grounded-context block: discipline + labelled passages + Sources.
/// This is what both the MCP `search` tool and `/retrieve` return, and what `/route` injects.
pub fn grounded_context(question: &str, hits: &[Hit], manifest: &Manifest) -> String {
    // `assemble` returns (SYSTEM, "Context:\n\n[cid]\ntext...\n---\nQuestion: ...").
    let (_system, user) = assemble(question, hits);
    let cited: Vec<&str> = hits.iter().map(|h| h.citation_id.as_str()).collect();
    let sources = manifest.sources_block(&cited);
    let mut block = format!("{SYSTEM}\n\n{user}");
    if !sources.is_empty() {
        block.push_str("\n\n");
        block.push_str(&sources);
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits() -> Vec<Hit> {
        vec![Hit {
            citation_id: "EMPL-PR-785214:1".into(), doc_id: "EMPL-PR-785214".into(),
            score: 0.73, text: "an offer within four months".into(), title: "Youth Guarantee".into(),
        }]
    }
    fn manifest() -> Manifest {
        Manifest::from_jsonl(r#"{"doc_id":"EMPL-PR-785214","title":"Youth Guarantee","pdf_url":"https://ep/YG.pdf"}"#).unwrap()
    }

    #[test]
    fn builds_block_with_discipline_passages_and_sources() {
        let block = grounded_context("What is the deadline?", &hits(), &manifest());
        assert!(block.contains("USING ONLY"));                 // grounding discipline (from SYSTEM)
        assert!(block.contains("[EMPL-PR-785214:1]"));          // labelled passage
        assert!(block.contains("four months"));
        assert!(block.contains("Sources:"));                    // provenance
        assert!(block.contains("https://ep/YG.pdf"));
    }

    #[test]
    fn empty_hits_still_returns_discipline() {
        let block = grounded_context("anything", &[], &manifest());
        assert!(block.contains("I don't know"));                // model can honestly decline
        assert!(!block.contains("Sources:"));                   // nothing cited → no sources
    }
}
