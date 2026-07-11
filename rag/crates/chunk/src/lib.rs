//! Structure-aware, token-bounded chunking for EP committee PDFs.
//!
//! Two steps:
//!   1. `clean_boilerplate` — strip the EP page-header/footer artifacts that
//!      pdf-extract interleaves (PE-refs, *.docx tags, page N/M, the "United in
//!      diversity" motto, lone "EN").
//!   2. token-aware recursive splitting via `text-splitter` (respects paragraph/
//!      sentence boundaries), sized in TOKENS so every chunk stays under bge-small's
//!      512-token limit (else the embedder silently truncates).
//!
//! TODO (tomorrow / refinement): pre-split on `Article N` / recital markers for
//! cleaner legislative boundaries; this is currently generic recursive splitting.
use anyhow::{anyhow, Result};
use hf_hub::{api::sync::Api, Repo, RepoType};
use regex::Regex;
use serde::Serialize;
use text_splitter::{ChunkConfig, TextSplitter};
use tokenizers::Tokenizer;

#[derive(Debug, Clone, Serialize)]
pub struct Chunk {
    pub citation_id: String, // doc_id:chunk_index — what the generator cites
    pub doc_id: String,
    pub committee: String,
    pub doc_type: String,
    pub title: String,
    pub chunk_index: usize,
    pub n_chunks: usize,
    pub text: String,
    pub char_len: usize,
    pub token_len: usize,
}

/// Strip EP page-header/footer boilerplate that pdf-extract interleaves.
pub fn clean_boilerplate(text: &str) -> String {
    let re_ref = Regex::new(r"PE\s?\d+\.\d+(?:\s?v\d+-\d+)?").unwrap();
    let re_docx = Regex::new(r"\S*\.docx").unwrap();
    let re_page = Regex::new(r"\b\d+/\d+\b").unwrap();
    let mut out = Vec::new();
    for line in text.lines() {
        let footer = re_ref.is_match(line) || re_docx.is_match(line);
        let mut s = re_ref.replace_all(line, "").to_string();
        s = re_docx.replace_all(&s, "").to_string();
        if footer {
            // page numbers stripped only on footer lines, so body numbers survive
            s = re_page.replace_all(&s, "").to_string();
        }
        let t = s.trim();
        if t.is_empty() || t == "EN" || t.contains("United in diversity") {
            continue;
        }
        out.push(t.to_string());
    }
    out.join("\n")
}

pub struct Chunker {
    splitter: TextSplitter<Tokenizer>,
    tokenizer: Tokenizer,
}

impl Chunker {
    pub fn new() -> Result<Self> {
        let api = Api::new()?;
        let repo = api.repo(Repo::new("BAAI/bge-small-en-v1.5".to_string(), RepoType::Model));
        let tokenizer =
            Tokenizer::from_file(repo.get("tokenizer.json")?).map_err(|e| anyhow!("{e}"))?;
        // Sizing in TOKENS. bge-small max seq = 512, so 256..500 keeps every chunk
        // (+2 special tokens at embed time) safely under truncation; ~50 overlap.
        let cfg = ChunkConfig::new(256..500)
            .with_sizer(tokenizer.clone())
            .with_overlap(50)?;
        Ok(Self { splitter: TextSplitter::new(cfg), tokenizer })
    }

    fn token_len(&self, s: &str) -> usize {
        self.tokenizer.encode(s, true).map(|e| e.len()).unwrap_or(0)
    }

    /// Clean boilerplate, then recursively split into token-bounded chunks.
    pub fn chunk_document(
        &self,
        doc_id: &str,
        committee: &str,
        doc_type: &str,
        title: &str,
        raw_text: &str,
    ) -> Vec<Chunk> {
        let clean = clean_boilerplate(raw_text);
        let pieces: Vec<String> = self.splitter.chunks(&clean).map(str::to_string).collect();
        let n = pieces.len();
        pieces
            .into_iter()
            .enumerate()
            .map(|(i, text)| Chunk {
                citation_id: format!("{doc_id}:{i}"),
                doc_id: doc_id.to_string(),
                committee: committee.to_string(),
                doc_type: doc_type.to_string(),
                title: title.to_string(),
                chunk_index: i,
                n_chunks: n,
                char_len: text.chars().count(),
                token_len: self.token_len(&text),
                text,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::clean_boilerplate;

    #[test]
    fn strips_ep_boilerplate_keeps_body() {
        let raw = "PR\\1338336EN.docx PE785.214v01-00\n\
                   EN\n\
                   United in diversity EN\n\
                   This is real body text about the Youth Guarantee.\n\
                   PE785.214v01-00 2/11 PR\\1338336EN.docx";
        let out = clean_boilerplate(raw);
        assert!(out.contains("This is real body text about the Youth Guarantee."));
        assert!(!out.contains("docx"));
        assert!(!out.contains("United in diversity"));
        assert!(!out.contains("v01-00"));
    }

    #[test]
    fn keeps_body_numbers_like_fractions() {
        // a body "3/4" is NOT on a footer line -> must survive
        let out = clean_boilerplate("Roughly 3/4 of Member States screen investments.");
        assert!(out.contains("3/4"));
    }
}
