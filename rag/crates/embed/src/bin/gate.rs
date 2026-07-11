//! Rust half of the embedding parity gate: embed the shared samples (passages
//! without prefix, queries with prefix) and write vectors to data/parity/rust.json.
//! The Python half (drills/parity_gate.py) embeds the same samples with
//! sentence-transformers and compares cosine per string.
use anyhow::Result;
use ep_rag_embed::Embedder;
use serde_json::{json, Map, Value};
use std::fs;

fn strings(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn main() -> Result<()> {
    let samples: Value = serde_json::from_str(&fs::read_to_string("parity_samples.json")?)?;
    let passages = strings(&samples, "passages");
    let queries = strings(&samples, "queries");

    let embedder = Embedder::new()?; // downloads bge-small from HF on first run
    let pv = embedder.embed_passages(passages.clone())?;
    let qv = embedder.embed_queries(queries.clone())?;

    let mut pass = Map::new();
    for (t, v) in passages.iter().zip(&pv) {
        pass.insert(t.clone(), json!(v));
    }
    let mut qry = Map::new();
    for (t, v) in queries.iter().zip(&qv) {
        qry.insert(t.clone(), json!(v));
    }

    fs::create_dir_all("data/parity")?;
    fs::write(
        "data/parity/rust.json",
        serde_json::to_string(&json!({"passages": pass, "queries": qry, "dim": pv[0].len()}))?,
    )?;
    println!(
        "wrote data/parity/rust.json ({} passages, {} queries, dim {})",
        passages.len(),
        queries.len(),
        pv[0].len()
    );
    Ok(())
}
