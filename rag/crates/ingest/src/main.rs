//! Ingestion: manifest -> parse (pdf-extract) -> chunk -> data/chunks.jsonl.
//!
//! This is the BOILERPLATE half of ingestion. It stops at chunks — the next step,
//! embed + upsert-to-Qdrant with the contract-stamped payload, is the DRILL-1
//! surface and is done in tomorrow's hand-holding session.
use anyhow::{Context, Result};
use ep_rag_chunk::{Chunk, Chunker};
use ep_rag_parse::extract_text;
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize)]
struct ManifestRow {
    doc_id: String,
    committee: String,
    doc_type: String,
    title: String,
    pdf_path: String,
}

fn main() -> Result<()> {
    let manifest = Path::new("data/manifest.jsonl");
    let raw = fs::read_to_string(manifest).with_context(|| {
        format!("missing {} — run: cargo run -p ep-rag-fetch --bin fetch", manifest.display())
    })?;
    let rows: Vec<ManifestRow> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    println!("parse + chunk over {} documents", rows.len());

    let chunker = Chunker::new()?;
    let mut all: Vec<Chunk> = Vec::new();
    for row in &rows {
        let text = match extract_text(Path::new(&row.pdf_path)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  ! {}: parse failed: {e}", row.doc_id);
                continue;
            }
        };
        let chunks =
            chunker.chunk_document(&row.doc_id, &row.committee, &row.doc_type, &row.title, &text);
        println!("  {:<18} {:>3} chunks", row.doc_id, chunks.len());
        all.extend(chunks);
    }

    let toks: Vec<usize> = all.iter().map(|c| c.token_len).collect();
    let over = toks.iter().filter(|&&t| t > 512).count();
    let (min, max) = (toks.iter().min().copied().unwrap_or(0), toks.iter().max().copied().unwrap_or(0));
    let mean = if toks.is_empty() { 0 } else { toks.iter().sum::<usize>() / toks.len() };

    let out = Path::new("data/chunks.jsonl");
    let body: String = all
        .iter()
        .map(|c| serde_json::to_string(c).map(|s| s + "\n"))
        .collect::<Result<String, _>>()?;
    fs::write(out, body)?;

    println!("\n{} chunks -> {}", all.len(), out.display());
    println!("token_len: min {min}, mean {mean}, max {max}   ({over} over 512 — must be 0)");
    println!("\nNEXT (tomorrow, hand-holding): embed chunks (candle) + upsert to Qdrant with the");
    println!("contract-stamped payload. That embed->index boundary is where Drill 1 lives.");
    Ok(())
}
