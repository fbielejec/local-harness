# rag-server Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build `rag-server` — an OpenAI-compatible HTTP service that productizes the hand-stitched
`rag/notebooks/rag_query.org` loop (embed_query → Qdrant top-k → grounded prompt → Qwen → cited
answer), so Open WebUI can register it as a second model ("EP Committees, grounded") and any
household user gets grounded, cited answers with no RAG config.

**Architecture:** Three new Rust crates in the existing `rag/` cargo workspace, matching the
one-crate-per-brick house style and the `retrieve`/`generate` names the integration design calls
for:
- **`crates/retrieve`** (lib) — reuses `ep-rag-embed::Embedder::embed_queries` (the query PREFIX
  contract) + `qdrant-client` gRPC top-k → ranked `Hit`s carrying `citation_id`, `score`, `text`,
  `doc_id`.
- **`crates/generate`** (lib) — pure grounded-prompt `assemble`, `<think>` stripping, the
  serve-time provenance join (`citation_id → doc_id → manifest.pdf_url`, the design's option (b),
  no re-index), and the llama-server generation client (streaming + non-streaming).
- **`crates/rag-server`** (bin) — axum service exposing `GET /v1/models` and
  `POST /v1/chat/completions` (SSE streaming + non-streaming). On startup it asserts the live
  embedding contract equals the index's stored contract (`EmbeddingContract::assert_matches`) and
  warms the generator with a boot ping. Env-configured so laptop↔weebeastie is a config swap.

Only `:3000` (Open WebUI) faces the LAN; `rag-server` (`:8081`), Qdrant (`:6334`), and llama-server
(`:8080`) stay loopback.

**Tech Stack:** Rust 2021 · axum + tokio (HTTP server) · reqwest (streaming client to llama-server)
· qdrant-client 1.x (gRPC) · candle (bge via `ep-rag-embed`) · serde/serde_json · anyhow.

---

## Context the executor needs (read before starting)

- **The executable spec is `rag/notebooks/rag_query.org`.** Every behavior below mirrors a block
  there: `embed_query` (with the PREFIX), `search` (Qdrant top-k, `with_payload`), `assemble`
  (SYSTEM prompt + `[cid]\ntext` concat), `generate` (llama-server `/v1/chat/completions`, temp 0).
  When in doubt, match the notebook.
- **The one hard constraint (design §"The one hard constraint"):** the query MUST be embedded under
  the identical recipe as the index. We reuse `ep-rag-embed::Embedder`, whose `embed_queries`
  already prepends the contract prefix — do **not** re-implement embedding. Open WebUI is never the
  retriever.
- **Contract types already exist** in `crates/core/src/lib.rs`: `EmbeddingContract::default()`,
  `.as_payload()`, `.assert_matches(&stored)`. The index stamps `contract_*` onto every payload
  (`crates/index/src/lib.rs::chunk_payload`). We assert live == stored at startup.
- **`Chunk`/payload fields** (`crates/chunk/src/lib.rs`, `crates/index/src/lib.rs`): payload carries
  `citation_id` (= `doc_id:chunk_index`), `doc_id`, `committee`, `doc_type`, `title`, `chunk_index`,
  `n_chunks`, `text`, `char_len`, `token_len`, and the six `contract_*` fields.
- **Manifest** `rag/data/manifest.jsonl`, one JSON object per line, has `doc_id`, `title`,
  `pdf_url` — the source for the Sources block (design §"Citations & provenance UX", option (b)).
- **qdrant-client is already a dep** of `crates/index` (gRPC, default URL `http://localhost:6334`,
  env `QDRANT_URL`). Reuse the same collection `ep_committee_docs` and the same env var.
- **Release builds are mandatory** for anything touching candle — debug is 10–50× slower
  (CLAUDE.md). All `cargo run` for `rag-server` use `--release`.
- **All commands run from `rag/`** (the workspace root) unless stated. Qwen cold-start on first call
  is minutes (model fault-in); warm thereafter. `--parallel 1` serializes requests.

## Decisions locked for this plan (from the design doc, no re-litigation)

1. **Option A — RAG-as-a-model.** `rag-server` is an OpenAI-compatible endpoint; OWUI registers it
   as a second connection. No OWUI native RAG (`BYPASS_EMBEDDING_AND_RETRIEVAL=true` stays).
2. **Provenance = serve-time join (option (b)).** Load the manifest at startup; join
   `citation_id → doc_id → pdf_url`. **No re-index.**
3. **Retrieval transport = gRPC via `qdrant-client`** (reuse `crates/index`'s approach + `QDRANT_URL`),
   not the notebook's REST — the maximal-Rust house choice.
4. **Fusion = equal concat** (the notebook's `assemble`; the industry mean-pool). `p_eta`-weighting,
   rerank/MMR/dedup are the Drill-2 track — **out of scope here.**
5. **Streaming is required** (design §"latency"): SSE so first tokens appear during the multi-chunk
   prefill; plus a warm-on-boot ping.

---

## Phase 0 — Scaffolding

### Task 0.1: Pin new workspace dependencies

**Files:**
- Modify: `rag/Cargo.toml` (the `[workspace.dependencies]` table)

**Step 1: Add the shared deps** the three crates will draw from, keeping the central-pinning
convention. Append to `[workspace.dependencies]`:

```toml
# HTTP service (rag-server) + streaming client to llama-server.
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
axum = "0.7"
reqwest = { version = "0.12", features = ["json", "stream"] }
futures = "0.3"
tokio-stream = "0.1"
qdrant-client = "1"
uuid = { version = "1", features = ["v5"] }
```

> Note: `tokio`, `qdrant-client`, `uuid` are already used ad-hoc in `crates/index`; centralizing
> them here is DRY. Leave the `crates/index` manifest as-is (it pins its own; migrating it is a
> separate cleanup, YAGNI for this plan).

**Step 2: Verify the workspace still resolves**

Run: `cargo metadata --no-deps --format-version 1 >/dev/null && echo OK`
Expected: `OK` (no manifest parse error).

**Step 3: Commit**

```bash
git add rag/Cargo.toml
git commit -m "build(rag): pin axum/reqwest/futures deps for rag-server"
```

---

## Phase 1 — `crates/retrieve` (embed query → Qdrant top-k)

### Task 1.1: Create the crate skeleton

**Files:**
- Create: `rag/crates/retrieve/Cargo.toml`
- Create: `rag/crates/retrieve/src/lib.rs`

**Step 1: Write `Cargo.toml`**

```toml
[package]
name = "ep-rag-retrieve"
version.workspace = true
edition.workspace = true

[dependencies]
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
ep-rag-core = { path = "../core" }
ep-rag-embed = { path = "../embed" }
qdrant-client = { workspace = true }
tokio = { workspace = true }
```

**Step 2: Write a stub `lib.rs`** so the workspace compiles:

```rust
//! Retrieve brick: embed the query under the contract PREFIX, then Qdrant top-k.
//! The productization of `rag_query.org`'s `embed_query` + `search` blocks.
```

**Step 3: Verify it builds**

Run: `cargo build -p ep-rag-retrieve`
Expected: compiles (empty lib).

**Step 4: Commit**

```bash
git add rag/crates/retrieve
git commit -m "feat(retrieve): scaffold ep-rag-retrieve crate"
```

### Task 1.2: `Hit` type + pure payload→Hit parsing (TDD)

The network-free core: turn a Qdrant payload (as a `serde_json` map) + score into a `Hit`. This is
the unit-testable seam — the live gRPC call converts qdrant values to json at the boundary and
hands them here.

**Files:**
- Modify: `rag/crates/retrieve/src/lib.rs`
- Test: same file (`#[cfg(test)]`)

**Step 1: Write the failing test**

```rust
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
        })
        .as_object()
        .unwrap()
        .clone()
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
        // doc_id is derivable from citation_id (doc_id:chunk_index) if the payload omits it.
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
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ep-rag-retrieve hit_from_payload`
Expected: FAIL — `cannot find function hit_from_payload` / `Hit`.

**Step 3: Write the minimal implementation**

```rust
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
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ep-rag-retrieve hit_from_payload`
Expected: PASS (3 tests).

**Step 5: Commit**

```bash
git add rag/crates/retrieve/src/lib.rs
git commit -m "feat(retrieve): Hit type + pure payload parsing (doc_id fallback)"
```

### Task 1.3: qdrant Value → serde_json conversion (TDD)

qdrant-client returns `HashMap<String, qdrant_client::qdrant::Value>`; the pure parser above wants a
`serde_json::Map`. Write and test the conversion so the boundary is trustworthy.

**Files:**
- Modify: `rag/crates/retrieve/src/lib.rs`

**Step 1: Write the failing test**

```rust
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
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ep-rag-retrieve qval_conversion`
Expected: FAIL — `cannot find function payload_to_json`.

**Step 3: Write the minimal implementation**

```rust
use std::collections::HashMap;
use qdrant_client::qdrant::{value::Kind, Value as QValue};

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
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ep-rag-retrieve qval_conversion`
Expected: PASS.

**Step 5: Commit**

```bash
git add rag/crates/retrieve/src/lib.rs
git commit -m "feat(retrieve): qdrant Value -> serde_json payload conversion"
```

### Task 1.4: `Retriever` — live embed + Qdrant search

Ties `Embedder` (query prefix) + qdrant-client top-k together. No unit test (network); an
`#[ignore]` integration test documents the contract and can be run against a live index. Mirrors the
notebook's `search`/`retrieve`.

**Files:**
- Modify: `rag/crates/retrieve/src/lib.rs`

**Step 1: Write the implementation**

```rust
use anyhow::{Context, Result};
use ep_rag_core::EmbeddingContract;
use ep_rag_embed::Embedder;
use qdrant_client::qdrant::SearchPointsBuilder;
use qdrant_client::Qdrant;

pub const COLLECTION: &str = "ep_committee_docs";

pub struct Retriever {
    embedder: Embedder,
    qdrant: Qdrant,
}

impl Retriever {
    /// `qdrant_url` e.g. `http://localhost:6334` (gRPC). Downloads/loads bge on first use.
    pub fn new(qdrant_url: &str) -> Result<Self> {
        let embedder = Embedder::new().context("load bge embedder")?;
        let qdrant = Qdrant::from_url(qdrant_url).build().context("connect qdrant")?;
        Ok(Self { embedder, qdrant })
    }

    pub fn contract(&self) -> &EmbeddingContract {
        self.embedder.contract()
    }

    /// Embed the question under the PREFIX, then Qdrant top-k with payload.
    pub async fn retrieve(&self, question: &str, k: u64) -> Result<Vec<Hit>> {
        let vec = self
            .embedder
            .embed_queries(vec![question.to_string()])? // QUERY prefix applied inside
            .into_iter()
            .next()
            .context("embedder returned no vector")?;

        let resp = self
            .qdrant
            .search_points(
                SearchPointsBuilder::new(COLLECTION, vec, k).with_payload(true),
            )
            .await
            .context("qdrant search")?;

        Ok(resp
            .result
            .into_iter()
            .filter_map(|p| hit_from_payload(p.score, &payload_to_json(&p.payload)))
            .collect())
    }

    /// Fetch ONE point's stored contract_* fields, for the startup drift assert.
    /// Returns the payload map of an arbitrary point (scroll limit 1).
    pub async fn any_stored_payload(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        use qdrant_client::qdrant::ScrollPointsBuilder;
        let resp = self
            .qdrant
            .scroll(ScrollPointsBuilder::new(COLLECTION).limit(1).with_payload(true))
            .await
            .context("qdrant scroll")?;
        let point = resp.result.into_iter().next().context("collection is empty")?;
        Ok(payload_to_json(&point.payload))
    }
}
```

**Step 2: Add an ignored integration test** (documents the live contract; run manually against a
populated Qdrant):

```rust
#[tokio::test]
#[ignore = "requires a live, populated Qdrant at QDRANT_URL"]
async fn retrieve_youth_guarantee_returns_empl_chunks() {
    let url = std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".into());
    let r = Retriever::new(&url).unwrap();
    let hits = r
        .retrieve("What is the deadline under the Youth Guarantee?", 5)
        .await
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.citation_id.contains(':')));
    // notebook ground truth: top hits are the EMPL Youth-Guarantee draft report
    assert!(hits.iter().any(|h| h.doc_id.starts_with("EMPL-PR-785214")));
}
```

**Step 3: Verify it compiles and unit tests still pass**

Run: `cargo test -p ep-rag-retrieve`
Expected: PASS (pure tests run; the `#[ignore]`d one is skipped).

**Step 4: (Optional, manual) run the live probe** with Qdrant up (`make qdrant-up` + index present):

Run: `QDRANT_URL=http://localhost:6334 cargo test -p ep-rag-retrieve --release -- --ignored`
Expected: PASS — hits include `EMPL-PR-785214`.

**Step 5: Commit**

```bash
git add rag/crates/retrieve/src/lib.rs
git commit -m "feat(retrieve): Retriever (bge query embed + qdrant top-k) + ignored live test"
```

---

## Phase 2 — `crates/generate` (assemble, think-strip, provenance, gen client)

### Task 2.1: Create the crate skeleton

**Files:**
- Create: `rag/crates/generate/Cargo.toml`
- Create: `rag/crates/generate/src/lib.rs`

**Step 1: Write `Cargo.toml`**

```toml
[package]
name = "ep-rag-generate"
version.workspace = true
edition.workspace = true

[dependencies]
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
ep-rag-retrieve = { path = "../retrieve" }
reqwest = { workspace = true }
futures = { workspace = true }
tokio = { workspace = true }
```

**Step 2: Write a stub `lib.rs`**

```rust
//! Generate brick: grounded-prompt assembly, <think> stripping, provenance join,
//! and the llama-server generation client. Productizes rag_query.org's B/C blocks.
pub mod prompt;
pub mod provenance;
pub mod client;
```

Create the three empty module files so it compiles:
- `rag/crates/generate/src/prompt.rs` (empty)
- `rag/crates/generate/src/provenance.rs` (empty)
- `rag/crates/generate/src/client.rs` (empty)

**Step 3: Verify it builds**

Run: `cargo build -p ep-rag-generate`
Expected: compiles.

**Step 4: Commit**

```bash
git add rag/crates/generate
git commit -m "feat(generate): scaffold ep-rag-generate crate"
```

### Task 2.2: `assemble` grounded prompt (TDD)

The exact SYSTEM prompt + context format from the notebook's block B.

**Files:**
- Modify: `rag/crates/generate/src/prompt.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ep_rag_retrieve::Hit;

    fn hits() -> Vec<Hit> {
        vec![
            Hit { citation_id: "EMPL-PR-785214:1".into(), doc_id: "EMPL-PR-785214".into(),
                  score: 0.73, text: "young person under the age of 30 ... within four months".into(),
                  title: "Youth Guarantee".into() },
            Hit { citation_id: "EMPL-PR-785214:3".into(), doc_id: "EMPL-PR-785214".into(),
                  score: 0.69, text: "The evaluation of the reinforced Youth Guarantee".into(),
                  title: "Youth Guarantee".into() },
        ]
    }

    #[test]
    fn assemble_fixes_grounding_and_labels_context() {
        let (system, user) = assemble("What is the deadline?", &hits());
        // grounding discipline from the notebook SYSTEM prompt
        assert!(system.contains("USING ONLY"));
        assert!(system.contains("I don't know"));
        assert!(system.contains("square brackets"));
        // each chunk is labelled with its citation_id and its text appears
        assert!(user.contains("[EMPL-PR-785214:1]"));
        assert!(user.contains("within four months"));
        assert!(user.contains("Question: What is the deadline?"));
    }

    #[test]
    fn assemble_with_no_hits_still_produces_a_prompt() {
        let (_system, user) = assemble("anything", &[]);
        assert!(user.contains("Question: anything"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ep-rag-generate assemble`
Expected: FAIL — `cannot find function assemble`.

**Step 3: Write the minimal implementation** (SYSTEM string copied verbatim from the notebook):

```rust
use ep_rag_retrieve::Hit;

/// The grounding discipline — copied verbatim from rag_query.org block B.
pub const SYSTEM: &str = "You are a research assistant for European Parliament committee documents. \
Answer the question USING ONLY the numbered context passages below. \
After each claim, cite the passage id in square brackets, e.g. [EMPL-PR-785214:3]. \
If the answer is not contained in the context, reply with exactly: I don't know.";

/// Concatenate top-k as `[cid]\ntext` (equal concat = the industry mean-pool) and
/// wrap with the question. Returns `(system, user)`.
pub fn assemble(question: &str, hits: &[Hit]) -> (String, String) {
    let context = hits
        .iter()
        .map(|h| format!("[{}]\n{}", h.citation_id, h.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let user = format!("Context:\n\n{context}\n\n---\nQuestion: {question}");
    (SYSTEM.to_string(), user)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ep-rag-generate assemble`
Expected: PASS.

**Step 5: Commit**

```bash
git add rag/crates/generate/src/prompt.rs
git commit -m "feat(generate): grounded-prompt assemble (verbatim notebook SYSTEM)"
```

### Task 2.3: `<think>` stripping (TDD)

Design §Components(1): if a ThinkingCap reasoning model is loaded, strip `<think>…</think>` before
returning.

**Files:**
- Modify: `rag/crates/generate/src/prompt.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn strip_think_removes_reasoning_block() {
    assert_eq!(strip_think("<think>ponder</think>Answer [X:1]."), "Answer [X:1].");
    // no think block -> unchanged
    assert_eq!(strip_think("Answer [X:1]."), "Answer [X:1].");
    // multiline
    assert_eq!(strip_think("<think>\na\nb\n</think>\nHi"), "Hi");
    // unterminated open tag (stream cut mid-think) -> drop the tail
    assert_eq!(strip_think("before<think>never closes"), "before");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ep-rag-generate strip_think`
Expected: FAIL — `cannot find function strip_think`.

**Step 3: Write the minimal implementation**

```rust
/// Remove `<think>…</think>` spans (ThinkingCap models). An unterminated `<think>`
/// (e.g. a truncated stream) drops everything from the tag onward. Whitespace-trims.
pub fn strip_think(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => { rest = ""; break; }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ep-rag-generate strip_think`
Expected: PASS.

**Step 5: Commit**

```bash
git add rag/crates/generate/src/prompt.rs
git commit -m "feat(generate): strip <think> blocks for ThinkingCap models"
```

### Task 2.4: Provenance — manifest load + Sources block (TDD)

Design §"Citations & provenance UX", option (b): join `citation_id → doc_id → manifest.pdf_url` at
serve time. No re-index.

**Files:**
- Modify: `rag/crates/generate/src/provenance.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        // one JSON object per line, exactly like data/manifest.jsonl
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
        // two chunks from the same doc -> ONE source line
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
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ep-rag-generate -- provenance`
Expected: FAIL — `Manifest` undefined. (Note: `cargo test` filters by test-name substring; these
live in `provenance.rs` so `manifest`/`sources_block` names match.)

**Step 3: Write the minimal implementation**

```rust
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
/// provenance source (design option (b) — no re-index needed).
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
```

> Note: `sources_block` takes `&[&str]`; the caller will pass the cited ids. For the MVP we append
> a Sources block for **all retrieved** ids (the notebook returns `sources = [c['cid'] ...]`).
> Parsing which ids the model *actually* cited from the answer text is a refinement (YAGNI now); a
> code comment should say so.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ep-rag-generate -- provenance`
Expected: PASS (3 tests).

**Step 5: Commit**

```bash
git add rag/crates/generate/src/provenance.rs
git commit -m "feat(generate): manifest provenance join + deduped Sources block"
```

### Task 2.5: Generation client (non-streaming first)

Calls llama-server `/v1/chat/completions` (temp 0), like the notebook's `generate`. Also expose the
model id (from `GET /v1/models`) so `rag-server` can label itself. No unit test (network); an
`#[ignore]`d live test documents it.

**Files:**
- Modify: `rag/crates/generate/src/client.rs`

**Step 1: Write the implementation**

```rust
use anyhow::{Context, Result};
use serde_json::json;

/// Client for the OpenAI-compatible llama-server (the generator).
pub struct GenClient {
    base_url: String, // e.g. http://localhost:8080/v1
    http: reqwest::Client,
}

impl GenClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), http: reqwest::Client::new() }
    }

    /// The gguf model id llama-server advertises (notebook: MODEL = /models[0].id).
    pub async fn model_id(&self) -> Result<String> {
        let v: serde_json::Value = self
            .http
            .get(format!("{}/models", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        v["data"][0]["id"].as_str().map(str::to_owned).context("no model id from llama-server")
    }

    fn body(&self, model: &str, system: &str, user: &str, stream: bool) -> serde_json::Value {
        json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0.0,
            "max_tokens": 400,
            "stream": stream
        })
    }

    /// Non-streaming completion → the answer text.
    pub async fn complete(&self, model: &str, system: &str, user: &str) -> Result<String> {
        let v: serde_json::Value = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&self.body(model, system, user, false))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .context("no content in completion")
    }

    /// Warm-on-boot ping (design §latency): a tiny request to fault the model in.
    pub async fn warm(&self, model: &str) -> Result<()> {
        let _ = self.complete(model, "ping", "ping").await?;
        Ok(())
    }
}
```

**Step 2: Add an ignored live test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires the llama-server tunnel at :8080"]
    async fn complete_answers_from_context() {
        let c = GenClient::new("http://localhost:8080/v1");
        let model = c.model_id().await.unwrap();
        let out = c
            .complete(&model, super::super::prompt::SYSTEM,
                "Context:\n\n[X:1]\nThe deadline is four months.\n\n---\nQuestion: What is the deadline?")
            .await
            .unwrap();
        assert!(out.to_lowercase().contains("four months"));
    }
}
```

**Step 3: Verify it compiles**

Run: `cargo test -p ep-rag-generate`
Expected: PASS (pure tests; live test skipped).

**Step 4: Commit**

```bash
git add rag/crates/generate/src/client.rs
git commit -m "feat(generate): non-streaming llama-server client + model_id + warm ping"
```

### Task 2.6: Streaming generation (SSE from llama-server → token stream)

Design §latency: stream so first tokens appear during prefill. Return a `Stream` of answer-text
deltas that `rag-server` re-frames as its own SSE.

**Files:**
- Modify: `rag/crates/generate/src/client.rs`

**Step 1: Add the streaming method** (parses upstream `data: {json}\n\n` SSE lines, yields the
`choices[0].delta.content` strings, stops on `data: [DONE]`):

```rust
use futures::stream::Stream;

impl GenClient {
    /// Streaming completion → a stream of answer-text deltas (already extracted from
    /// the upstream SSE). Errors in the stream surface as `Err`.
    pub async fn complete_stream(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<impl Stream<Item = Result<String>>> {
        use futures::StreamExt;
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&self.body(model, system, user, true))
            .send()
            .await?
            .error_for_status()?;

        // reqwest byte stream -> line-buffered SSE -> content deltas.
        let mut buf = String::new();
        let stream = resp.bytes_stream().flat_map(move |chunk| {
            let mut out: Vec<Result<String>> = Vec::new();
            match chunk {
                Err(e) => out.push(Err(anyhow::anyhow!(e))),
                Ok(bytes) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(nl) = buf.find('\n') {
                        let line = buf[..nl].trim().to_string();
                        buf.drain(..=nl);
                        if let Some(data) = line.strip_prefix("data:") {
                            let data = data.trim();
                            if data == "[DONE]" || data.is_empty() {
                                continue;
                            }
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(s) = v["choices"][0]["delta"]["content"].as_str() {
                                    if !s.is_empty() {
                                        out.push(Ok(s.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            futures::stream::iter(out)
        });
        Ok(stream)
    }
}
```

> **Streaming + `<think>`:** `strip_think` is buffer-based, so for streamed output the server should
> either (a) buffer the whole answer, strip, then emit (simplest, loses streaming for ThinkingCap),
> or (b) suppress deltas while inside a `<think>` span with a small tag-straddling buffer. MVP: do
> (a) only when a `STRIP_THINK=1` env is set; otherwise pass deltas straight through (the default
> Qwen-Coder emits no `<think>`). Document this in the server task.

**Step 2: Extend the ignored live test** to drain the stream and assert non-empty:

```rust
#[tokio::test]
#[ignore = "requires the llama-server tunnel at :8080"]
async fn complete_stream_yields_deltas() {
    use futures::StreamExt;
    let c = GenClient::new("http://localhost:8080/v1");
    let model = c.model_id().await.unwrap();
    let mut s = Box::pin(
        c.complete_stream(&model, "You are terse.", "Say: four months.").await.unwrap(),
    );
    let mut acc = String::new();
    while let Some(item) = s.next().await {
        acc.push_str(&item.unwrap());
    }
    assert!(!acc.is_empty());
}
```

**Step 3: Verify it compiles**

Run: `cargo test -p ep-rag-generate`
Expected: PASS (live tests skipped).

**Step 4: Commit**

```bash
git add rag/crates/generate/src/client.rs
git commit -m "feat(generate): streaming SSE completion (content-delta stream)"
```

---

## Phase 3 — `crates/rag-server` (OpenAI-compatible HTTP service)

### Task 3.1: Create the binary crate + config

**Files:**
- Create: `rag/crates/rag-server/Cargo.toml`
- Create: `rag/crates/rag-server/src/main.rs`
- Create: `rag/crates/rag-server/src/config.rs`

**Step 1: Write `Cargo.toml`**

```toml
[package]
name = "ep-rag-server"
version.workspace = true
edition.workspace = true

[[bin]]
name = "rag-server"
path = "src/main.rs"

[dependencies]
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
ep-rag-core = { path = "../core" }
ep-rag-retrieve = { path = "../retrieve" }
ep-rag-generate = { path = "../generate" }
axum = { workspace = true }
tokio = { workspace = true }
futures = { workspace = true }
```

**Step 2: Write `config.rs` with a TDD'd env parser**

Test first (in `config.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_env_reader_applies_defaults_and_overrides() {
        let get = |k: &str| match k {
            "QDRANT_URL" => Some("http://x:6334".to_string()),
            "TOP_K" => Some("8".to_string()),
            _ => None,
        };
        let c = Config::from_reader(get);
        assert_eq!(c.qdrant_url, "http://x:6334");
        assert_eq!(c.top_k, 8);
        // defaults
        assert_eq!(c.gen_base_url, "http://localhost:8080/v1");
        assert_eq!(c.bind_addr, "127.0.0.1:8081");
        assert_eq!(c.manifest_path, "data/manifest.jsonl");
        assert_eq!(c.model_id, "ep-committees-grounded");
    }
}
```

Then the impl:

```rust
/// rag-server config. Env-driven so laptop↔weebeastie is a config swap (design §Components 1).
#[derive(Debug, Clone)]
pub struct Config {
    pub qdrant_url: String,
    pub gen_base_url: String,
    pub bind_addr: String,
    pub manifest_path: String,
    pub top_k: u64,
    pub model_id: String,   // the name OWUI shows in its model picker
    pub strip_think: bool,
}

impl Config {
    /// Testable core: reads via a closure so tests inject env without touching the process.
    pub fn from_reader(get: impl Fn(&str) -> Option<String>) -> Self {
        let s = |k: &str, d: &str| get(k).unwrap_or_else(|| d.to_string());
        Config {
            qdrant_url: s("QDRANT_URL", "http://localhost:6334"),
            gen_base_url: s("GEN_BASE_URL", "http://localhost:8080/v1"),
            bind_addr: s("RAG_BIND_ADDR", "127.0.0.1:8081"),
            manifest_path: s("MANIFEST_PATH", "data/manifest.jsonl"),
            top_k: s("TOP_K", "5").parse().unwrap_or(5),
            model_id: s("RAG_MODEL_ID", "ep-committees-grounded"),
            strip_think: get("STRIP_THINK").as_deref() == Some("1"),
        }
    }

    pub fn from_env() -> Self {
        Self::from_reader(|k| std::env::var(k).ok())
    }
}
```

**Step 3: Write a placeholder `main.rs`** so the crate builds:

```rust
mod config;
fn main() { println!("rag-server (wiring in next tasks)"); }
```

**Step 4: Verify**

Run: `cargo test -p ep-rag-server`
Expected: PASS (config test).

**Step 5: Commit**

```bash
git add rag/crates/rag-server
git commit -m "feat(rag-server): scaffold bin + env-driven Config (TDD)"
```

### Task 3.2: OpenAI request/response types + latest-user extraction (TDD)

**Files:**
- Create: `rag/crates/rag-server/src/openai.rs`
- Modify: `rag/crates/rag-server/src/main.rs` (add `mod openai;`)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_user_message_picks_last_user_turn() {
        let req: ChatRequest = serde_json::from_str(r#"{
            "model": "ep-committees-grounded",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": "What is the Youth Guarantee deadline?"}
            ],
            "stream": true
        }"#).unwrap();
        assert_eq!(req.latest_user_message().unwrap(), "What is the Youth Guarantee deadline?");
        assert!(req.stream);
    }

    #[test]
    fn latest_user_message_none_when_no_user_turn() {
        let req = ChatRequest { model: "m".into(), messages: vec![], stream: false };
        assert!(req.latest_user_message().is_none());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ep-rag-server latest_user`
Expected: FAIL — types undefined.

**Step 3: Write the minimal implementation**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    #[serde(default)]
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
}

impl ChatRequest {
    /// The question = the content of the last `user` turn (design §Components 1).
    pub fn latest_user_message(&self) -> Option<&str> {
        self.messages.iter().rev().find(|m| m.role == "user").map(|m| m.content.as_str())
    }
}

// --- response shaping (OpenAI /v1/chat/completions) ---

/// Build a non-streaming response body carrying `content`.
pub fn completion_response(model: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "ragcmpl-0",
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }]
    })
}

/// One streaming SSE chunk carrying a content delta (OpenAI `chat.completion.chunk`).
pub fn stream_chunk(model: &str, delta: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "ragcmpl-0",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{"index": 0, "delta": {"content": delta}, "finish_reason": serde_json::Value::Null}]
    })
}

/// The final SSE chunk (finish_reason=stop, empty delta).
pub fn stream_final(model: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "ragcmpl-0",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    })
}
```

Add a test for the chunk shapes too:

```rust
#[test]
fn stream_chunk_has_delta_content() {
    let c = stream_chunk("m", "four");
    assert_eq!(c["choices"][0]["delta"]["content"], serde_json::json!("four"));
    assert_eq!(c["object"], serde_json::json!("chat.completion.chunk"));
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ep-rag-server`
Expected: PASS.

**Step 5: Commit**

```bash
git add rag/crates/rag-server/src/openai.rs rag/crates/rag-server/src/main.rs
git commit -m "feat(rag-server): OpenAI request/response types + latest-user extraction (TDD)"
```

### Task 3.3: App state, startup contract assert, warm ping, `GET /v1/models`

**Files:**
- Modify: `rag/crates/rag-server/src/main.rs`

**Step 1: Write the wiring** — build shared state, assert the contract, warm the model, serve
`/v1/models`. This is integration glue (no unit test); a manual curl verifies it.

```rust
mod config;
mod openai;
mod handlers; // added next task

use anyhow::{Context, Result};
use axum::{routing::{get, post}, Router, Json};
use ep_rag_core::EmbeddingContract;
use ep_rag_generate::{client::GenClient, provenance::Manifest};
use ep_rag_retrieve::Retriever;
use std::sync::Arc;

pub struct AppState {
    pub retriever: Retriever,
    pub gen: GenClient,
    pub manifest: Manifest,
    pub upstream_model: String, // the gguf id llama-server advertises
    pub cfg: config::Config,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config::Config::from_env();
    eprintln!("rag-server config: {cfg:?}");

    // 1. Build retriever (loads bge + connects Qdrant).
    let retriever = Retriever::new(&cfg.qdrant_url).context("init retriever")?;

    // 2. CONTRACT ASSERT (the one hard constraint): live embedder contract == index's stored.
    let stored = retriever.any_stored_payload().await.context("read stored contract")?;
    let stored_map: std::collections::BTreeMap<_, _> = stored.into_iter().collect();
    EmbeddingContract::default()
        .assert_matches(&stored_map)
        .map_err(|e| anyhow::anyhow!(e))
        .context("startup contract check")?;
    eprintln!("contract OK — live embedder matches the index");

    // 3. Generation client + resolve upstream model id + warm ping.
    let gen = GenClient::new(&cfg.gen_base_url);
    let upstream_model = gen.model_id().await.context("resolve llama-server model")?;
    eprintln!("generator model: {upstream_model}");
    if let Err(e) = gen.warm(&upstream_model).await {
        eprintln!("warm ping failed (non-fatal): {e}");
    }

    // 4. Manifest for provenance.
    let manifest = Manifest::load(&cfg.manifest_path).context("load manifest")?;

    let state = Arc::new(AppState { retriever, gen, manifest, upstream_model, cfg: cfg.clone() });

    let app = Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(handlers::chat_completions))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await
        .with_context(|| format!("bind {}", cfg.bind_addr))?;
    eprintln!("rag-server listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// OpenAI-compatible model list — advertises OUR model id (not the gguf) to OWUI.
async fn models(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "object": "list",
        "data": [{"id": state.cfg.model_id, "object": "model", "owned_by": "ep-rag"}]
    }))
}
```

**Step 2: Add a throwaway empty `handlers.rs`** with a stub so it compiles:

```rust
use axum::{extract::State, Json};
use std::sync::Arc;
use crate::AppState;

pub async fn chat_completions(
    State(_state): State<Arc<AppState>>,
    Json(_req): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"error": "not implemented"}))
}
```

**Step 3: Verify it compiles**

Run: `cargo build -p ep-rag-server`
Expected: compiles.

**Step 4: Commit**

```bash
git add rag/crates/rag-server/src/main.rs rag/crates/rag-server/src/handlers.rs
git commit -m "feat(rag-server): app state, startup contract assert + warm, /v1/models"
```

### Task 3.4: `POST /v1/chat/completions` — non-streaming path

**Files:**
- Modify: `rag/crates/rag-server/src/handlers.rs`

**Step 1: Implement the non-streaming branch** (RAG loop = notebook's `answer`): extract question →
retrieve top-k → assemble → complete → optional think-strip → append Sources block → OpenAI response.

```rust
use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};
use ep_rag_generate::prompt::{assemble, strip_think};
use std::sync::Arc;
use crate::{openai::*, AppState};

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Response {
    let question = match req.latest_user_message() {
        Some(q) if !q.trim().is_empty() => q.to_string(),
        _ => return error_response("no user message in request"),
    };

    // Retrieve → assemble (the notebook's retrieve + assemble).
    let hits = match state.retriever.retrieve(&question, state.cfg.top_k).await {
        Ok(h) => h,
        Err(e) => return error_response(&format!("retrieve failed: {e}")),
    };
    let (system, user) = assemble(&question, &hits);
    let cited: Vec<&str> = hits.iter().map(|h| h.citation_id.as_str()).collect();

    if req.stream {
        return stream_answer(state, system, user, cited).await; // Task 3.5
    }

    // Non-streaming.
    let mut answer = match state.gen.complete(&state.upstream_model, &system, &user).await {
        Ok(a) => a,
        Err(e) => return error_response(&format!("generate failed: {e}")),
    };
    if state.cfg.strip_think {
        answer = strip_think(&answer);
    }
    let sources = state.manifest.sources_block(&cited);
    if !sources.is_empty() {
        answer = format!("{answer}\n\n{sources}");
    }
    Json(completion_response(&state.cfg.model_id, &answer)).into_response()
}

fn error_response(msg: &str) -> Response {
    (axum::http::StatusCode::BAD_GATEWAY,
     Json(serde_json::json!({"error": {"message": msg}}))).into_response()
}

// placeholder until Task 3.5
async fn stream_answer(_s: Arc<AppState>, _sys: String, _usr: String, _c: Vec<&str>) -> Response {
    error_response("streaming not implemented yet")
}
```

**Step 2: Fix the borrow** — `cited` borrows `hits`; since `stream_answer` needs owned data, change
its signature to take `Vec<String>` and map before calling. Adjust:

```rust
let cited: Vec<String> = hits.iter().map(|h| h.citation_id.clone()).collect();
// ... sources uses &cited refs:
let cited_refs: Vec<&str> = cited.iter().map(String::as_str).collect();
let sources = state.manifest.sources_block(&cited_refs);
```

**Step 3: Verify it compiles**

Run: `cargo build -p ep-rag-server`
Expected: compiles.

**Step 4: Manual end-to-end check** (needs Qdrant + index + llama-server tunnel up):

```bash
# terminal A
cargo run --release -p ep-rag-server
# terminal B
curl -s http://localhost:8081/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"ep-committees-grounded","stream":false,
       "messages":[{"role":"user","content":"What is the deadline under the Youth Guarantee?"}]}' \
  | python3 -m json.tool
```

Expected: JSON with a cited answer mentioning "four months" + a trailing `Sources:` block.

**Step 5: Commit**

```bash
git add rag/crates/rag-server/src/handlers.rs
git commit -m "feat(rag-server): non-streaming RAG chat completion (retrieve→assemble→generate→sources)"
```

### Task 3.5: Streaming path (SSE) with a Sources tail

**Files:**
- Modify: `rag/crates/rag-server/src/handlers.rs`

**Step 1: Implement `stream_answer`** — re-frame the generator's content-delta stream as our own SSE
`chat.completion.chunk`s, then emit the Sources block as a final content chunk, then `[DONE]`.

```rust
use axum::response::sse::{Event, Sse};
use futures::stream::{self, Stream, StreamExt};
use std::convert::Infallible;

async fn stream_answer(
    state: Arc<AppState>,
    system: String,
    user: String,
    cited: Vec<String>,
) -> Response {
    let model = state.cfg.model_id.clone();

    let upstream = match state.gen.complete_stream(&state.upstream_model, &system, &user).await {
        Ok(s) => s,
        Err(e) => return error_response(&format!("generate stream failed: {e}")),
    };

    // token deltas -> our SSE chunks
    let model_for_deltas = model.clone();
    let deltas = upstream.filter_map(move |item| {
        let m = model_for_deltas.clone();
        async move {
            match item {
                Ok(text) => Some(Ok::<Event, Infallible>(
                    Event::default().data(stream_chunk(&m, &text).to_string()),
                )),
                Err(_) => None, // drop a mid-stream error; final chunk still closes cleanly
            }
        }
    });

    // Sources tail as a final content chunk (design: household-facing provenance).
    let cited_refs: Vec<&str> = cited.iter().map(String::as_str).collect();
    let sources = state.manifest.sources_block(&cited_refs);
    let mut tail: Vec<Result<Event, Infallible>> = Vec::new();
    if !sources.is_empty() {
        tail.push(Ok(Event::default().data(
            stream_chunk(&model, &format!("\n\n{sources}")).to_string(),
        )));
    }
    tail.push(Ok(Event::default().data(stream_final(&model).to_string())));
    tail.push(Ok(Event::default().data("[DONE]")));

    let body = deltas.chain(stream::iter(tail));
    Sse::new(body).into_response()
}
```

> **Note on `<think>` while streaming:** MVP passes deltas straight through. If `STRIP_THINK=1`, the
> honest MVP is to fall back to the **non-streaming** path for this request (buffer→strip→emit),
> since delta-wise stripping needs a tag-straddling buffer. Implement that as: in
> `chat_completions`, `if req.stream && !state.cfg.strip_think { stream } else { non-stream }`. Add a
> one-line comment explaining the fallback.

**Step 2: Apply the strip-think/stream fallback** in `chat_completions`:

```rust
let want_stream = req.stream && !state.cfg.strip_think;
if want_stream {
    return stream_answer(state, system, user, cited).await;
}
// else fall through to non-streaming (also covers STRIP_THINK + stream)
```

**Step 3: Verify it compiles**

Run: `cargo build -p ep-rag-server`
Expected: compiles.

**Step 4: Manual streaming check** (services up):

```bash
curl -N -s http://localhost:8081/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"ep-committees-grounded","stream":true,
       "messages":[{"role":"user","content":"What is the deadline under the Youth Guarantee?"}]}'
```

Expected: a sequence of `data: {"object":"chat.completion.chunk",...}` lines with incremental
content, then a Sources chunk, then `data: [DONE]`.

**Step 5: Commit**

```bash
git add rag/crates/rag-server/src/handlers.rs
git commit -m "feat(rag-server): SSE streaming completion with Sources tail + think-strip fallback"
```

### Task 3.6: Makefile target + workspace test sweep

**Files:**
- Modify: `rag/Makefile`

**Step 1: Add a `serve` target**

```makefile
serve: ## run rag-server (OpenAI-compatible RAG endpoint on :8081)
	$(CARGO_RUN) -p ep-rag-server --bin rag-server
```

Add `serve` to the `.PHONY` list.

**Step 2: Run the full workspace test suite**

Run: `cargo test`
Expected: PASS — all pure/unit tests across core, index, retrieve, generate, rag-server;
`#[ignore]`d live tests skipped.

**Step 3: Commit**

```bash
git add rag/Makefile
git commit -m "build(rag): make serve target for rag-server"
```

---

## Phase 4 — Deploy to weebeastie & wire Open WebUI

> This phase runs **on weebeastie** (or over ssh). It has no unit tests; each step is a manual verify.
> Do it only after Phases 1–3 pass locally against the laptop's Qdrant + tunnelled llama-server.

### Task 4.1: Migrate the index to weebeastie (snapshot transfer — no re-embed)

Design §"Migrating the index": snapshot on the laptop, restore on weebeastie. The contract travels
in the payload, so the restored collection is self-describing.

**Steps (manual):**

1. Laptop — ensure Qdrant is up and populated: `cd rag && make qdrant-up && make qdrant-status`
   (expect `ep_committee_docs`, 490 points).
2. Create a snapshot:
   `curl -s -X POST http://localhost:6333/collections/ep_committee_docs/snapshots | python3 -m json.tool`
   — note the returned snapshot `name`.
3. Download it:
   `curl -s http://localhost:6333/collections/ep_committee_docs/snapshots/<name> -o /tmp/ep.snapshot`
4. Copy to weebeastie: `scp /tmp/ep.snapshot filip@192.168.1.22:~/ep.snapshot`
5. On weebeastie — start Qdrant from the version-controlled compose:
   `cd ~/openwebui/../qdrant 2>/dev/null || true` → actually: sync `deploy/qdrant/` to weebeastie
   and `docker compose -f deploy/qdrant/docker-compose.yml up -d`.
6. Restore via the upload API:
   `curl -s -X POST 'http://localhost:6333/collections/ep_committee_docs/snapshots/upload?priority=snapshot' -H 'Content-Type:multipart/form-data' -F 'snapshot=@/home/filip/ep.snapshot'`
7. Verify: `curl -s http://localhost:6333/collections/ep_committee_docs | python3 -m json.tool`
   → `points_count: 490`.

**Verify (the whole point):** on weebeastie, scroll one point and confirm the six `contract_*`
fields are present — that's what `rag-server`'s startup assert reads.

No commit (data move), but record the snapshot name + date in `README.md` runbook (Task 4.4).

### Task 4.2: Run `rag-server` on weebeastie as a systemd unit

Mirror the `llama-server`/Qdrant deploy pattern (version-controlled unit → installed copy).

**Files:**
- Create: `deploy/rag-server.service`

**Step 1: Write the unit** (loopback bind; env points at local Qdrant + local llama-server; the
binary path is the release build on weebeastie):

```ini
[Unit]
Description=EP-RAG OpenAI-compatible RAG server (retrieve+generate)
After=network-online.target docker.service llama-server.service
Wants=network-online.target

[Service]
Type=simple
User=filip
WorkingDirectory=/home/filip/local_coding_harness/rag
Environment=QDRANT_URL=http://localhost:6334
Environment=GEN_BASE_URL=http://localhost:8080/v1
Environment=RAG_BIND_ADDR=127.0.0.1:8081
Environment=MANIFEST_PATH=/home/filip/local_coding_harness/rag/data/manifest.jsonl
Environment=TOP_K=5
Environment=RAG_MODEL_ID=ep-committees-grounded
ExecStart=/home/filip/local_coding_harness/rag/target/release/rag-server
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

> Adjust `WorkingDirectory`/paths to wherever the repo lives on weebeastie. `MANIFEST_PATH` must be
> absolute (the service has no guaranteed CWD-relative data). Build the release binary there first:
> `cargo build --release -p ep-rag-server` (needs the candle/bge cache on weebeastie — same
> prerequisite as running `make index` remotely).

**Step 2: Install + start** (on weebeastie):

```bash
sudo cp deploy/rag-server.service /etc/systemd/system/rag-server.service
sudo systemctl daemon-reload
sudo systemctl enable --now rag-server
systemctl status rag-server              # expect active (running); logs show "contract OK"
journalctl -u rag-server -n 40 --no-pager
```

**Verify:** `curl -s http://localhost:8081/v1/models` → our `ep-committees-grounded` id; a curl
chat-completion (as in Task 3.4/3.5) returns a cited answer.

**Step 3: Commit**

```bash
git add deploy/rag-server.service
git commit -m "deploy(rag-server): systemd unit for weebeastie (loopback :8081)"
```

### Task 4.3: Register the second model in Open WebUI

Design §Components(3): OWUI adds `rag-server` as a **second** OpenAI connection; keep
`BYPASS_EMBEDDING_AND_RETRIEVAL=true` (we never use OWUI RAG).

**Steps (manual, Admin Panel — remember the ConfigVar caveat: the volume is authoritative after
first boot, so change this in the UI, not just compose):**

1. Open WebUI → Admin → Settings → **Connections** → add an OpenAI connection:
   - Base URL: `http://localhost:8081/v1` (host networking → container reaches loopback)
   - API key: any non-empty dummy (e.g. `dummy`)
2. Confirm the model **`ep-committees-grounded`** appears in the model picker.
3. Verify `BYPASS_EMBEDDING_AND_RETRIEVAL=true` and `ENABLE_DIRECT_CONNECTIONS=false` are still set
   (Admin → Documents / Connections).
4. As a household user: pick "EP Committees (grounded)", ask the Youth-Guarantee question → grounded,
   cited answer + Sources block, streamed.

**Step 2: Reflect the setting in version control** — if a compose/env seed is used, update
`deploy/openwebui/.env.example` with a comment documenting the second connection (URL + the
`BYPASS_EMBEDDING_AND_RETRIEVAL` reminder). Commit:

```bash
git add deploy/openwebui/.env.example
git commit -m "docs(openwebui): document the rag-server second OpenAI connection"
```

### Task 4.4: Update docs (CLAUDE.md, README, design doc status)

**Files:**
- Modify: `CLAUDE.md` (the RAG TODO block + the "Operating the chat frontend" section)
- Modify: `README.md` (append a runbook entry: snapshot name/date, the curl checks, `make serve`)
- Modify: `docs/plans/2026-07-11-openwebui-rag-integration-design.md` (flip Status to "implemented",
  note the provenance option chosen = (b), and resolve the "Gap to decide")

**Steps:**

1. In `CLAUDE.md`, under the RAG TODO, check off "harden `retrieve`/`generate` into Rust crates
   (+ `rag-server`…)" and add operating notes: `make serve`, the `:8081` loopback endpoint, the
   systemd unit, and "Open WebUI second model = EP Committees (grounded)".
2. In `README.md`, add the chronological entry with exact commands + observed outputs (latency,
   sample cited answer).
3. In the integration design doc, set **Status: implemented (YYYY-MM-DD)**, record provenance =
   serve-time join (b), and link this plan.

**Step 4: Commit**

```bash
git add CLAUDE.md README.md docs/plans/2026-07-11-openwebui-rag-integration-design.md
git commit -m "docs: rag-server operating notes + integration design status=implemented"
```

---

## Definition of done

- [ ] `cargo test` green across the workspace (pure/unit); `--ignored` live tests pass with services up.
- [ ] `make serve` boots `rag-server`; startup logs **"contract OK"** (drift would refuse to serve).
- [ ] Non-streaming curl returns a grounded, cited answer + Sources block.
- [ ] Streaming curl emits incremental `chat.completion.chunk`s → Sources tail → `[DONE]`.
- [ ] Index restored on weebeastie (490 points, contract fields intact) via snapshot (no re-embed).
- [ ] `rag-server` runs as an enabled systemd unit on weebeastie, loopback `:8081`.
- [ ] Open WebUI shows "EP Committees (grounded)"; a household user gets a streamed cited answer.
- [ ] Docs updated; design doc status flipped to implemented.

## Explicitly out of scope (do NOT build here)

- The two named debugging drills (Drill 1 embedding-mismatch, Drill 2 confidence-blind fusion) and
  the eval harness (recall@k/MRR vs groundedness+citation-accuracy) — separate track, the immediate
  *next* work per CLAUDE.md.
- Reranker, MMR, dedup, `p_eta`-weighted fusion, hybrid BM25, multilingual (dense-only by decision).
- Per-user document upload; OWUI native RAG (`VECTOR_DB=qdrant`) — rejected by the design.
- Parsing which citation ids the model *actually* emitted (Sources currently lists all retrieved);
  refine later alongside the citation-accuracy eval axis.
- Auto-refresh of the index from ODP change feeds.
```
