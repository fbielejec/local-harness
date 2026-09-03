//! index bin: `data/chunks.jsonl` -> candle BGE embed (passages, NO prefix) ->
//! upsert to Qdrant under the contract-stamped payload.
//!
//! This is the embed->index boundary = Drill-1's surface. Single default (dense)
//! vector, cosine, HNSW ANN. Idempotent: deterministic point ids mean a re-run
//! overwrites in place. Config via env (QDRANT_URL) — one code path, two deploys.

use anyhow::{Context, Result};
use rag_chunk::Chunk;
use rag_embed::Embedder;
use rag_index::{chunk_payload, point_id};
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::{Payload, Qdrant};
use std::fs;
use std::path::Path;

const COLLECTION: &str = "ep_committee_docs";
const DIM: u64 = 384;
const BATCH: usize = 64;

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".into());

    // 1. load chunks (Chunk is Deserialize now)
    let path = Path::new("data/chunks.jsonl");
    let raw = fs::read_to_string(path).with_context(|| {
        format!("missing {} — run: cargo run -p rag-ingest --bin ingest", path.display())
    })?;
    let chunks: Vec<Chunk> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    println!("loaded {} chunks", chunks.len());

    // 2. embedder + its contract (stamped onto every point)
    let embedder = Embedder::new()?;
    let contract = embedder.contract().clone();

    // 3. qdrant client + ensure collection (create-if-absent)
    let client = Qdrant::from_url(&url).build()?;
    if client.collection_exists(COLLECTION).await? {
        println!("collection {COLLECTION} exists — upserting (idempotent)");
    } else {
        client
            .create_collection(
                CreateCollectionBuilder::new(COLLECTION)
                    .vectors_config(VectorParamsBuilder::new(DIM, Distance::Cosine)),
            )
            .await?;
        println!("created collection {COLLECTION} (dim {DIM}, cosine)");
    }

    // 4. embed passages + upsert, in batches
    let total = chunks.len();
    let mut done = 0;
    for batch in chunks.chunks(BATCH) {
        let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
        let vecs = embedder.embed_passages(texts)?; // passages: NO instruction prefix
        let points: Vec<PointStruct> = batch
            .iter()
            .zip(vecs)
            .map(|(c, v)| {
                let json = serde_json::Value::Object(
                    chunk_payload(c, &contract).into_iter().collect(),
                );
                let payload = Payload::try_from(json).expect("payload is a json object");
                PointStruct::new(point_id(&c.citation_id).to_string(), v, payload)
            })
            .collect();
        client
            .upsert_points(UpsertPointsBuilder::new(COLLECTION, points))
            .await?;
        done += batch.len();
        println!("  upserted {done}/{total}");
    }

    // 5. verify count round-trips
    let info = client.collection_info(COLLECTION).await?;
    let count = info.result.and_then(|r| r.points_count);
    println!("done — {COLLECTION} points_count = {count:?} (expected {total})");
    Ok(())
}
