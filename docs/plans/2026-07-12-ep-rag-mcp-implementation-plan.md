# EP-RAG MCP Server Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build `ep-rag-mcp` — one loopback Rust service that exposes EP-committee retrieval three
ways: an **MCP `search` tool** (for qwen-code and other agentic hosts), a plain **`POST /retrieve`**,
and a deterministic **`POST /route`** (Mode-A tree-classify → retrieve-if-relevant) that lets Open
WebUI be a single self-routing household model with a swappable generator.

**Architecture:** A new bin crate in the existing `rag/` cargo workspace, a thin shell over the
already-built `ep-rag-retrieve` / `ep-rag-generate` / `ep-rag-core` crates. Pure logic (route-JSON
parsing, tree-prompt building, grounded-context assembly) is TDD-unit-tested; the two network seams
(llama-server classify call, Qdrant search) reuse existing clients and are covered by `#[ignore]`d
live tests + manual curl. The MCP face uses `rmcp` 2.2 Streamable-HTTP; the HTTP faces use axum.

**Tech Stack:** Rust 2021 · `rmcp` 2.2 (`server`, `macros`, `transport-streamable-http-server`,
`schemars`) · axum 0.8 (this crate only) · tokio · serde/serde_json · anyhow · reuse:
`ep-rag-retrieve` (`Retriever`, `Hit`), `ep-rag-generate` (`prompt::{SYSTEM,assemble}`,
`provenance::Manifest`, `client::GenClient`), `ep-rag-core::EmbeddingContract`.

---

## Context the executor needs (read before starting)

- **The design is `docs/plans/2026-07-12-ep-rag-mcp-design.md`.** When in doubt, match it. The spike
  that justified the deterministic `/route` (Mode-A 10/10 vs native 8/10) is `scratchpad/tree_spike.py`.
- **Reused signatures (do NOT re-implement):**
  - `ep_rag_retrieve::Retriever::new(qdrant_url: &str) -> Result<Retriever>`,
    `.retrieve(&self, question: &str, k: u64) -> Result<Vec<Hit>>`,
    `.contract(&self) -> &EmbeddingContract`,
    `.any_stored_payload(&self) -> Result<serde_json::Map<String,Value>>`.
  - `ep_rag_retrieve::Hit { citation_id, doc_id, score, text, title }` (all pub), `doc_id_of(&str)`.
  - `ep_rag_generate::prompt::{SYSTEM: &str, assemble(question, &[Hit]) -> (String,String), strip_think(&str) -> String}`.
  - `ep_rag_generate::provenance::Manifest::{load(path)->Result<Self>, from_jsonl(&str), lookup(cid)->Option<&ManifestEntry>, sources_block(&[&str]) -> String}`.
  - `ep_rag_generate::client::GenClient::{new(base_url), model_id()->Result<String>, complete(model,system,user)->Result<String>, warm(model)->Result<()>}`.
  - `ep_rag_core::EmbeddingContract::{default(), as_payload()->BTreeMap<String,Value>, assert_matches(&BTreeMap)->Result<(),String>}`.
- **The one hard constraint:** embedding happens inside `Retriever` (query PREFIX contract); assert
  live == stored on boot. Never let a caller pass raw vectors.
- **Release builds are mandatory** for anything touching candle/`Retriever` (debug is 10–50× slower).
- **All commands run from `rag/`** unless stated. Qwen cold-start is minutes on first call, warm after.
  `--parallel 1` serializes requests (classify then answer are two serialized turns).
- **The generator (llama-server) must be reachable** at `GEN_BASE_URL` for live tests: from the laptop
  that means the tunnel (`ssh -fN -L 8080:127.0.0.1:8080 filip@192.168.1.22`) is up.

## Decisions locked (from the design — no re-litigation)

1. **One crate, three faces, one loopback port** (`:8082`): MCP `search` tool, `POST /retrieve`,
   `POST /route`. `/route` is the household brain; the OWUI filter is trivial.
2. **Household evaluator = Mode-A classify call** (proven), NOT native tool-calling. The model never
   emits a tool_call in the household path; the code decides from parsed JSON.
3. **Grounding discipline rides in the returned context block** (host-agnostic), assembled once in Rust.
4. **`rmcp` for MCP**, Streamable-HTTP server transport (the only transport OWUI native MCP supports).
   Enable only `server`+`transport-streamable-http-server` (no rmcp reqwest-client feature).
5. **axum 0.8 for this crate only** (rmcp 2.2 aligns with 0.8); the workspace stays 0.7 for `rag-server`.
6. **Provenance = serve-time join** via `Manifest` (option (b), no re-index). Unchanged.
7. **The decision tree is versioned JSON** at `rag/data/route_tree.json`, loaded at startup.

---

## Phase 0 — Scaffolding

### Task 0.1: Create the crate skeleton

**Files:**
- Create: `rag/crates/mcp/Cargo.toml`
- Create: `rag/crates/mcp/src/main.rs`

**Step 1: Write `Cargo.toml`** (note: axum/rmcp pinned locally, not via workspace):

```toml
[package]
name = "ep-rag-mcp"
version.workspace = true
edition.workspace = true

[[bin]]
name = "ep-rag-mcp"
path = "src/main.rs"

[dependencies]
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
ep-rag-core = { path = "../core" }
ep-rag-retrieve = { path = "../retrieve" }
ep-rag-generate = { path = "../generate" }
axum = "0.8"
rmcp = { version = "2.2", features = ["server", "macros", "transport-streamable-http-server", "schemars"] }
schemars = "1"
tower = "0.5"
```

**Step 2: Write a placeholder `main.rs`** so the workspace compiles:

```rust
//! ep-rag-mcp: one loopback service, three faces — MCP `search` tool,
//! POST /retrieve, POST /route (Mode-A tree-classify → retrieve). See
//! docs/plans/2026-07-12-ep-rag-mcp-design.md.
fn main() {
    println!("ep-rag-mcp (wiring in later tasks)");
}
```

**Step 3: Verify the workspace resolves and builds**

Run: `cd rag && cargo build -p ep-rag-mcp`
Expected: compiles (downloads rmcp/axum-0.8 on first build; may take a minute).

> If rmcp/axum resolution fails, this is the integration risk flagged in the design. Do **not**
> proceed to real logic until `cargo build -p ep-rag-mcp` is green — resolving version conflicts here
> (locally pinning transitive deps if needed) is cheaper than after code is written.

**Step 4: Commit**

```bash
git add rag/crates/mcp
git commit -m "feat(mcp): scaffold ep-rag-mcp crate (rmcp 2.2 + axum 0.8)"
```

### Task 0.2: Add the decision-tree config

**Files:**
- Create: `rag/data/route_tree.json`

**Step 1: Write the tree** (the spike's tree, verbatim — it scored 10/10):

```json
{
  "version": "1.0",
  "last_updated": "2026-07-12",
  "description": "Top-level task classifier. Walk this tree before answering.",
  "root": "Q_EP_COMMITTEE",
  "nodes": {
    "Q_EP_COMMITTEE": {
      "type": "question",
      "question": "Is the user asking a substantive question about the work, positions, or content of European Parliament committee documents (EMPL employment/social affairs, REGI regional development, IMCO internal market/consumer protection) — e.g. draft reports, amendments, rapporteur positions, deadlines, or policy specifics?",
      "help": "YES: 'What is the Youth Guarantee deadline?', 'IMCO position on EV charging', 'REGI cohesion funding priorities', 'what does the EMPL draft report say about traineeships'. NO: general chit-chat, coding, math, translation, generic EU facts not tied to committee documents.",
      "yes": "R_USE_EP_RAG",
      "no": "R_UNCLASSIFIED"
    },
    "R_USE_EP_RAG": {
      "type": "result", "task_type": "EP_COMMITTEE_RESEARCH", "tool": "search_ep_committee_docs",
      "description": "Ground the answer in committee documents."
    },
    "R_UNCLASSIFIED": {
      "type": "result", "task_type": "UNCLASSIFIED", "tool": null,
      "description": "Answer from general knowledge; mark facts unverified; recommend authoritative sources."
    }
  }
}
```

**Step 2: Commit**

```bash
git add rag/data/route_tree.json
git commit -m "feat(mcp): add versioned route decision tree config"
```

---

## Phase 1 — Pure logic (TDD, network-free)

All three modules live in `ep-rag-mcp` and are unit-tested with no network. Wire them into `main.rs`
with `mod route; mod tree; mod context;` as you create them.

### Task 1.1: Route-decision parsing

Parse the model's Mode-A JSON output (`{"reached","tool","reason"}`, possibly fenced) into a `Route`.

**Files:**
- Create: `rag/crates/mcp/src/route.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_use_ep_rag() {
        let r = Route::from_completion(r#"{"reached":"R_USE_EP_RAG","tool":"search_ep_committee_docs","reason":"x"}"#);
        assert_eq!(r, Route::UseEpRag);
    }
    #[test]
    fn parses_unclassified() {
        let r = Route::from_completion(r#"{"reached":"R_UNCLASSIFIED","tool":null,"reason":"chit-chat"}"#);
        assert_eq!(r, Route::Unclassified);
    }
    #[test]
    fn tolerates_code_fence_and_prose() {
        let raw = "Sure:\n```json\n{\"reached\":\"R_USE_EP_RAG\",\"tool\":\"search_ep_committee_docs\"}\n```";
        assert_eq!(Route::from_completion(raw), Route::UseEpRag);
    }
    #[test]
    fn falls_back_to_unclassified_on_garbage() {
        // A safe default: if we cannot parse, do NOT force grounding (design: precision-biased).
        assert_eq!(Route::from_completion("I couldn't decide"), Route::Unclassified);
    }
}
```

**Step 2: Run to verify it fails**

Run: `cargo test -p ep-rag-mcp route::`
Expected: FAIL — `Route` undefined.

**Step 3: Write the minimal implementation**

```rust
/// The routing outcome. `Unclassified` is the safe default (do not force grounding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    UseEpRag,
    Unclassified,
}

impl Route {
    /// Parse the model's Mode-A JSON decision. Tolerates a ```json fence / surrounding prose.
    /// Any parse failure → `Unclassified` (precision-biased: never ground on uncertainty).
    pub fn from_completion(raw: &str) -> Route {
        let slice = extract_json_object(raw);
        let reached = slice
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("reached").and_then(|r| r.as_str()).map(str::to_owned));
        match reached.as_deref() {
            Some("R_USE_EP_RAG") => Route::UseEpRag,
            _ => Route::Unclassified,
        }
    }

    pub fn should_ground(self) -> bool {
        matches!(self, Route::UseEpRag)
    }
}

/// Return the first `{...}` slice (outermost braces) from possibly-fenced/prose text.
fn extract_json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end > start { Some(&raw[start..=end]) } else { None }
}
```

**Step 4: Run to verify it passes**

Run: `cargo test -p ep-rag-mcp route::`
Expected: PASS (4 tests).

**Step 5: Commit**

```bash
git add rag/crates/mcp/src/route.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): Route parsing from Mode-A JSON (unclassified-on-failure)"
```

### Task 1.2: Decision-tree load + classify-prompt builder

**Files:**
- Create: `rag/crates/mcp/src/tree.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"{"version":"1.0","root":"Q","nodes":{"Q":{"type":"question","question":"about EP committees?","help":"YES: x NO: y","yes":"R_USE_EP_RAG","no":"R_UNCLASSIFIED"}}}"#;

    #[test]
    fn loads_tree_from_json() {
        let t = Tree::from_json(TREE).unwrap();
        assert_eq!(t.version, "1.0");
    }

    #[test]
    fn classify_prompt_embeds_tree_and_demands_json() {
        let t = Tree::from_json(TREE).unwrap();
        let (system, user) = t.classify_prompt("What is the Youth Guarantee deadline?");
        assert!(system.contains("about EP committees?"));       // the tree is in the prompt
        assert!(system.contains("\"reached\""));                 // output contract
        assert!(system.contains("R_USE_EP_RAG"));
        assert!(user.contains("Youth Guarantee"));               // the message is the user turn
    }
}
```

**Step 2: Run to verify it fails**

Run: `cargo test -p ep-rag-mcp tree::`
Expected: FAIL — `Tree` undefined.

**Step 3: Write the minimal implementation** (the SYSTEM text mirrors the spike's `SYS_A`, which
scored 10/10 — do not paraphrase it loosely):

```rust
use anyhow::Result;

/// The routing tree, loaded as opaque JSON (we only need version + the raw object to inline
/// into the classify prompt; the model walks the structure, not Rust).
pub struct Tree {
    pub version: String,
    raw: serde_json::Value,
}

impl Tree {
    pub fn from_json(s: &str) -> Result<Self> {
        let raw: serde_json::Value = serde_json::from_str(s)?;
        let version = raw.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        Ok(Self { version, raw })
    }

    pub fn load(path: &str) -> Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Build the Mode-A classify prompt: tree in the system prompt, JSON-only output contract.
    /// Returns `(system, user)`. Mirrors scratchpad/tree_spike.py::SYS_A (proven 10/10).
    pub fn classify_prompt(&self, message: &str) -> (String, String) {
        let tree = serde_json::to_string_pretty(&self.raw).unwrap_or_default();
        let system = format!(
            "You are a task router. Below is a decision tree as JSON. Walk it starting at `root`. \
For the user's message, answer the `question` node(s) and follow yes/no until you reach a `result` node.\n\n\
DECISION TREE:\n{tree}\n\n\
Respond with ONLY a compact JSON object, no prose, no markdown fence:\n\
{{\"reached\": \"<result node id>\", \"tool\": \"<tool name or null>\", \"reason\": \"<one short clause>\"}}"
        );
        (system, message.to_string())
    }
}
```

**Step 4: Run to verify it passes**

Run: `cargo test -p ep-rag-mcp tree::`
Expected: PASS (2 tests).

**Step 5: Commit**

```bash
git add rag/crates/mcp/src/tree.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): decision-tree load + Mode-A classify-prompt builder"
```

### Task 1.3: Grounded-context block builder

Assemble the single host-agnostic context block (discipline + labelled passages + Sources) from
`Hit`s, reusing `assemble` and `Manifest::sources_block`.

**Files:**
- Create: `rag/crates/mcp/src/context.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ep_rag_generate::provenance::Manifest;
    use ep_rag_retrieve::Hit;

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
```

**Step 2: Run to verify it fails**

Run: `cargo test -p ep-rag-mcp context::`
Expected: FAIL — `grounded_context` undefined.

**Step 3: Write the minimal implementation** (reuse — do not re-derive the SYSTEM text or the
passage format):

```rust
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
```

**Step 4: Run to verify it passes**

Run: `cargo test -p ep-rag-mcp context::`
Expected: PASS (2 tests).

**Step 5: Commit**

```bash
git add rag/crates/mcp/src/context.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): grounded-context block (reuses assemble + sources_block)"
```

---

## Phase 2 — The router (classify → retrieve orchestration)

### Task 2.1: `Router` state + `route()`

Ties `GenClient` (classify), `Tree`, `Retriever`, and `Manifest` together. Network → an `#[ignore]`d
live test documents it; the pure seams are already tested in Phase 1.

**Files:**
- Create: `rag/crates/mcp/src/router.rs`

**Step 1: Write the implementation**

```rust
use anyhow::{Context, Result};
use ep_rag_generate::client::GenClient;
use ep_rag_generate::provenance::Manifest;
use ep_rag_retrieve::{Hit, Retriever};
use crate::{context::grounded_context, route::Route, tree::Tree};

pub struct Router {
    pub retriever: Retriever,
    pub gen: GenClient,
    pub tree: Tree,
    pub manifest: Manifest,
    pub gen_model: String,   // resolved llama-server gguf id
    pub top_k: u64,
}

/// The outcome `/route` returns and the OWUI filter consumes.
pub struct RouteOutcome {
    pub route: Route,
    pub context: Option<String>, // grounded block iff should_ground
    pub hits: Vec<Hit>,
}

impl Router {
    /// Classify the message (Mode-A), and if the route says ground, retrieve + assemble.
    pub async fn route(&self, message: &str) -> Result<RouteOutcome> {
        let (system, user) = self.tree.classify_prompt(message);
        let raw = self.gen.complete(&self.gen_model, &system, &user).await
            .context("classify call to llama-server")?;
        let route = Route::from_completion(&raw);
        if !route.should_ground() {
            return Ok(RouteOutcome { route, context: None, hits: vec![] });
        }
        let hits = self.retriever.retrieve(message, self.top_k).await.context("qdrant retrieve")?;
        let context = grounded_context(message, &hits, &self.manifest);
        Ok(RouteOutcome { route, context: Some(context), hits })
    }

    /// Retrieval-only (for the MCP tool + /retrieve): always retrieve, always ground.
    pub async fn retrieve_grounded(&self, question: &str) -> Result<(String, Vec<Hit>)> {
        let hits = self.retriever.retrieve(question, self.top_k).await.context("qdrant retrieve")?;
        let context = grounded_context(question, &hits, &self.manifest);
        Ok((context, hits))
    }
}
```

**Step 2: Add an ignored live test** (documents the end-to-end classify+retrieve; needs Qdrant +
llama-server):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    async fn router() -> Router {
        let qurl = std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".into());
        let gurl = std::env::var("GEN_BASE_URL").unwrap_or_else(|_| "http://localhost:8080/v1".into());
        let gen = GenClient::new(&gurl);
        let gen_model = gen.model_id().await.unwrap();
        Router {
            retriever: Retriever::new(&qurl).unwrap(),
            gen, tree: Tree::load("data/route_tree.json").unwrap(),
            manifest: Manifest::load("data/manifest.jsonl").unwrap(),
            gen_model, top_k: 5,
        }
    }

    #[tokio::test]
    #[ignore = "requires live Qdrant + llama-server"]
    async fn ep_question_grounds_general_question_passes_through() {
        let r = router().await;
        let ep = r.route("What is the deadline under the Youth Guarantee?").await.unwrap();
        assert_eq!(ep.route, Route::UseEpRag);
        assert!(ep.context.as_deref().unwrap().contains("Sources:"));

        let chit = r.route("What's 2 + 2?").await.unwrap();
        assert_eq!(chit.route, Route::Unclassified);
        assert!(chit.context.is_none());
    }
}
```

**Step 3: Verify it compiles + unit tests pass**

Run: `cargo test -p ep-rag-mcp`
Expected: PASS (Phase-1 tests; the live test is skipped).

**Step 4: Commit**

```bash
git add rag/crates/mcp/src/router.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): Router — Mode-A classify then retrieve-if-relevant (+ignored live test)"
```

---

## Phase 3 — HTTP faces (`/retrieve`, `/route`) + startup

### Task 3.1: Config (env) + app assembly

**Files:**
- Create: `rag/crates/mcp/src/config.rs`
- Modify: `rag/crates/mcp/src/main.rs`

**Step 1: TDD the config parser** (in `config.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_overrides() {
        let get = |k: &str| match k { "TOP_K" => Some("8".into()), _ => None };
        let c = Config::from_reader(get);
        assert_eq!(c.top_k, 8);
        assert_eq!(c.bind_addr, "127.0.0.1:8082");
        assert_eq!(c.qdrant_url, "http://localhost:6334");
        assert_eq!(c.gen_base_url, "http://localhost:8080/v1");
        assert_eq!(c.tree_path, "data/route_tree.json");
        assert_eq!(c.manifest_path, "data/manifest.jsonl");
    }
}
```

Then the impl:

```rust
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub qdrant_url: String,
    pub gen_base_url: String,
    pub tree_path: String,
    pub manifest_path: String,
    pub top_k: u64,
}
impl Config {
    pub fn from_reader(get: impl Fn(&str) -> Option<String>) -> Self {
        let s = |k: &str, d: &str| get(k).unwrap_or_else(|| d.to_string());
        Config {
            bind_addr: s("RAG_MCP_BIND_ADDR", "127.0.0.1:8082"),
            qdrant_url: s("QDRANT_URL", "http://localhost:6334"),
            gen_base_url: s("GEN_BASE_URL", "http://localhost:8080/v1"),
            tree_path: s("TREE_PATH", "data/route_tree.json"),
            manifest_path: s("MANIFEST_PATH", "data/manifest.jsonl"),
            top_k: s("TOP_K", "5").parse().unwrap_or(5),
        }
    }
    pub fn from_env() -> Self { Self::from_reader(|k| std::env::var(k).ok()) }
}
```

**Step 2: Verify**

Run: `cargo test -p ep-rag-mcp config::`
Expected: PASS.

**Step 3: Commit**

```bash
git add rag/crates/mcp/src/config.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): env-driven Config (TDD)"
```

### Task 3.2: `main` — build state, contract-assert on boot, serve `/retrieve` + `/route`

**Files:**
- Modify: `rag/crates/mcp/src/main.rs`

**Step 1: Write the wiring** (integration glue; verified by curl in Step 3):

```rust
mod config; mod context; mod route; mod router; mod tree;

use anyhow::{Context, Result};
use axum::{extract::State, routing::post, Json, Router as AxumRouter};
use ep_rag_core::EmbeddingContract;
use ep_rag_generate::{client::GenClient, provenance::Manifest};
use ep_rag_retrieve::Retriever;
use std::sync::Arc;
use router::Router;

type Shared = Arc<Router>;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config::Config::from_env();
    eprintln!("ep-rag-mcp config: {cfg:?}");

    let retriever = Retriever::new(&cfg.qdrant_url).context("init retriever")?;

    // The one hard constraint: live embedder contract == index's stored contract.
    let stored = retriever.any_stored_payload().await.context("read stored contract")?;
    let stored_map: std::collections::BTreeMap<_, _> = stored.into_iter().collect();
    EmbeddingContract::default().assert_matches(&stored_map)
        .map_err(|e| anyhow::anyhow!(e)).context("startup contract check")?;
    eprintln!("contract OK — live embedder matches the index");

    let gen = GenClient::new(&cfg.gen_base_url);
    let gen_model = gen.model_id().await.context("resolve llama-server model")?;
    if let Err(e) = gen.warm(&gen_model).await { eprintln!("warm ping failed (non-fatal): {e}"); }

    let state: Shared = Arc::new(Router {
        retriever, gen, gen_model, top_k: cfg.top_k,
        tree: tree::Tree::load(&cfg.tree_path).context("load tree")?,
        manifest: Manifest::load(&cfg.manifest_path).context("load manifest")?,
    });

    let app = AxumRouter::new()
        .route("/retrieve", post(retrieve))
        .route("/route", post(route_handler))
        .with_state(state.clone());
    // MCP `search` face is mounted in Phase 4.

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await
        .with_context(|| format!("bind {}", cfg.bind_addr))?;
    eprintln!("ep-rag-mcp listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct RetrieveReq { query: String, #[serde(default)] k: Option<u64> }

async fn retrieve(State(s): State<Shared>, Json(req): Json<RetrieveReq>) -> Json<serde_json::Value> {
    match s.retrieve_grounded(&req.query).await {
        Ok((context, hits)) => Json(serde_json::json!({ "context": context, "hits": hits })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

#[derive(serde::Deserialize)]
struct RouteReq { message: String }

async fn route_handler(State(s): State<Shared>, Json(req): Json<RouteReq>) -> Json<serde_json::Value> {
    match s.route(&req.message).await {
        Ok(o) => Json(serde_json::json!({
            "should_ground": o.context.is_some(),
            "route": if o.context.is_some() { "R_USE_EP_RAG" } else { "R_UNCLASSIFIED" },
            "context": o.context,
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}
```

> `Hit` already derives `Serialize`, so `"hits": hits` works. `req.k` is accepted but the MVP uses the
> configured `top_k`; honoring per-request `k` is a trivial later refinement (YAGNI now — note it).

**Step 2: Verify it compiles**

Run: `cargo build -p ep-rag-mcp`
Expected: compiles.

**Step 3: Manual live check** (needs `make qdrant-up`, the index present, and the llama-server tunnel):

```bash
cd rag && QDRANT_URL=http://localhost:6334 GEN_BASE_URL=http://localhost:8080/v1 \
  cargo run --release -p ep-rag-mcp &
# then, in another shell:
curl -s localhost:8082/route -d '{"message":"What is the Youth Guarantee deadline?"}' | head -c 300
# expect: {"should_ground":true,"route":"R_USE_EP_RAG","context":"You are a research assistant...Sources:..."}
curl -s localhost:8082/route -d '{"message":"whats 2+2"}' | head -c 200
# expect: {"should_ground":false,"route":"R_UNCLASSIFIED","context":null}
curl -s localhost:8082/retrieve -d '{"query":"Youth Guarantee deadline"}' | head -c 200
# expect: {"context":"...","hits":[{"citation_id":"EMPL-PR-...",...}]}
```

**Step 4: Commit**

```bash
git add rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): serve /retrieve + /route with startup contract-assert + warm"
```

---

## Phase 4 — MCP `search` face (`rmcp` Streamable-HTTP)

### Task 4.1: Minimal `rmcp` tool spike (derisk the transport before wiring retrieval)

Prove one `#[tool]` on `StreamableHttpService` mounts into the axum 0.8 app and completes an MCP
handshake, returning a canned string. This isolates the rmcp/axum integration risk.

**Files:**
- Create: `rag/crates/mcp/src/mcp.rs`
- Modify: `rag/crates/mcp/src/main.rs` (mount the MCP service at `/mcp`)

**Step 1: Write a hello-tool service.** Follow the current rmcp streamable-http server example
(`https://github.com/modelcontextprotocol/rust-sdk` → `examples/servers`, `StreamableHttpService`).
Sketch (adapt names to the 2.2 API — the macro set is `#[tool]` / `#[tool_router]` / `#[tool_handler]`):

```rust
use rmcp::{tool, tool_router, tool_handler, ServerHandler};
// A handler holding no state yet — just returns a canned string.
#[derive(Clone)]
pub struct EpTools; // will hold Arc<Router> in Task 4.2

#[tool_router]
impl EpTools {
    #[tool(description = "Health probe for the EP RAG MCP server.")]
    async fn ping(&self) -> String { "ok".to_string() }
}
#[tool_handler]
impl ServerHandler for EpTools {}
```

**Step 2: Mount it** in `main.rs` alongside the axum routes. Use rmcp's `StreamableHttpService` as a
tower service nested under `/mcp` (exact constructor per the 2.2 example — typically
`StreamableHttpService::new(|| Ok(EpTools::new()), Default::default(), Default::default())`, then
`.nest_service("/mcp", service)` on the axum router).

**Step 3: Smoke-test the handshake.** Start the server, then verify with an MCP client. Easiest is
qwen-code (Task 5.2's config) or the rmcp streamable-http *client* example. Minimal manual probe:

```bash
# MCP Streamable HTTP: initialize handshake should return serverInfo/capabilities.
curl -s localhost:8082/mcp -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}' | head -c 300
# expect a JSON-RPC result with serverInfo (not a 404/parse error).
```

Expected: a JSON-RPC `result`. If this fails, resolve the rmcp/axum wiring here (this is the whole
point of the spike) before Task 4.2.

**Step 4: Commit**

```bash
git add rag/crates/mcp/src/mcp.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): rmcp streamable-http hello-tool mounted at /mcp (transport spike)"
```

### Task 4.2: The real `search_ep_committee_docs` tool

**Files:**
- Modify: `rag/crates/mcp/src/mcp.rs`

**Step 1: Give `EpTools` the shared `Router`** and replace `ping` with the real tool. The tool takes
`{query, k?}`, calls `Router::retrieve_grounded`, and returns the grounded-context text (rmcp maps a
returned `String` to a text content block). Include `structuredContent` with hits if the 2.2 API
exposes it easily; otherwise text-only is acceptable for the MVP (note it).

```rust
use std::sync::Arc;
use crate::router::Router;
use rmcp::{tool, tool_router, tool_handler, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone)]
pub struct EpTools { router: Arc<Router> }
impl EpTools { pub fn new(router: Arc<Router>) -> Self { Self { router } } }

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// The user's information need (a natural-language question).
    pub query: String,
}

#[tool_router]
impl EpTools {
    #[tool(description = "Search European Parliament committee documents (EMPL, REGI, IMCO) for \
grounded, cited passages. Call ONLY for substantive questions about EP committee document \
content/positions/deadlines. Returns labelled passages, a grounding instruction, and Sources.")]
    async fn search_ep_committee_docs(&self, args: SearchArgs) -> String {
        match self.router.retrieve_grounded(&args.query).await {
            Ok((context, _hits)) => context,
            Err(e) => format!("retrieval error: {e}"),
        }
    }
}
#[tool_handler]
impl ServerHandler for EpTools {}
```

> Argument extraction: rmcp's `#[tool]` derives the JSON Schema from `SearchArgs` via `schemars`.
> Match the exact 2.2 signature convention from the example (it may wrap args in `Parameters<T>`);
> adjust the fn signature accordingly. Keep the tool name **exactly** `search_ep_committee_docs` — the
> tree's `R_USE_EP_RAG` node and the design reference it.

**Step 2: Pass the shared `Router` into the service factory** in `main.rs` (the `Router` built in Task
3.2 is now shared by the axum handlers *and* the MCP service — wrap once in `Arc`, clone into both).

**Step 3: Verify it compiles + smoke-test the tool call**

Run: `cargo build -p ep-rag-mcp`, start it, then (with Qdrant + tunnel up) list/call tools via
qwen-code (Task 5.2) or a `tools/call` JSON-RPC:

```bash
curl -s localhost:8082/mcp -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_ep_committee_docs","arguments":{"query":"Youth Guarantee deadline"}}}' | head -c 400
# expect: result content containing "[EMPL-PR-..." and "Sources:"
```

**Step 4: Commit**

```bash
git add rag/crates/mcp/src/mcp.rs rag/crates/mcp/src/main.rs
git commit -m "feat(mcp): real search_ep_committee_docs tool (shared Router)"
```

---

## Phase 5 — Host integration + deploy

### Task 5.1: Open WebUI inlet Filter (the thin household shim)

**Files:**
- Create: `deploy/openwebui/functions/ep_rag_router_filter.py`

**Step 1: Write the filter.** It calls `/route`; if grounded, injects the returned context as a
system message before the LLM runs. Deterministic — no model tool-calling.

```python
"""
title: EP RAG Router
author: local_coding_harness
description: On each user turn, ask ep-rag-mcp /route whether to ground in EP committee
             documents; if so, inject the retrieved+grounded context. Deterministic, no tool-calling.
version: 0.1.0
"""
import requests
from pydantic import BaseModel, Field


class Filter:
    class Valves(BaseModel):
        route_url: str = Field(default="http://localhost:8082/route")
        timeout_s: int = Field(default=120)

    def __init__(self):
        self.valves = self.Valves()

    def inlet(self, body: dict, __user__: dict = None) -> dict:
        msgs = body.get("messages", [])
        last_user = next((m for m in reversed(msgs) if m.get("role") == "user"), None)
        if not last_user or not last_user.get("content"):
            return body
        try:
            r = requests.post(self.valves.route_url,
                              json={"message": last_user["content"]},
                              timeout=self.valves.timeout_s)
            data = r.json()
        except Exception:
            return body  # fail open: never block a turn if the router is down
        if data.get("should_ground") and data.get("context"):
            # Prepend grounded context as a system message; the persona's own model generates.
            body["messages"] = [{"role": "system", "content": data["context"]}] + msgs
        return body
```

**Step 2: Install + test in Open WebUI** (manual): Admin → Functions → import this file, enable it
globally (or attach to the household persona). Ask an EP question → grounded, cited answer + Sources;
ask "what's 2+2" → normal answer, no injection.

**Step 3: Commit**

```bash
git add deploy/openwebui/functions/ep_rag_router_filter.py
git commit -m "feat(mcp): Open WebUI inlet filter — deterministic /route injection"
```

### Task 5.2: qwen-code MCP registration

**Files:**
- Modify: `~/.qwen/settings.json` (document the snippet here; the file is machine-local, not in repo)
- Create: `deploy/qwen/mcp-servers.snippet.json` (version-controlled reference)

**Step 1: Write the reference snippet** (`deploy/qwen/mcp-servers.snippet.json`):

```json
{
  "mcpServers": {
    "ep-rag": {
      "httpUrl": "http://localhost:8082/mcp",
      "description": "EP committee documents — grounded, cited retrieval (search_ep_committee_docs)."
    }
  }
}
```

> Verify the transport key against installed qwen 0.19.8 (`httpUrl` vs `url` vs `type:"http"`);
> qwen-code inherits gemini-cli's MCP config. Merge into the existing `~/.qwen/settings.json`
> (`"$version": 4` schema) without clobbering the `permissions`/`privacy`/`telemetry` blocks.

**Step 2: Test** — with `ep-rag-mcp` up, run `qwen` and ask an EP-policy question; confirm it calls
`search_ep_committee_docs` (per the spike, native tool-calling works; borderline over-fire is
acceptable for a human-in-the-loop coding agent).

**Step 3: Commit**

```bash
git add deploy/qwen/mcp-servers.snippet.json
git commit -m "docs(mcp): qwen-code MCP registration snippet"
```

### Task 5.3: systemd unit + Makefile target

**Files:**
- Create: `deploy/ep-rag-mcp.service`
- Modify: `rag/Makefile` (add a `serve-mcp` target)

**Step 1: Write the unit** (mirror `deploy/llama-server.service`: loopback, `Restart=always`,
`enabled` at boot; env for URLs/paths). Point `WorkingDirectory` at the deployed `rag/` so
`data/route_tree.json` + `data/manifest.jsonl` resolve.

```ini
[Unit]
Description=EP-RAG MCP server (retrieval + /route)
After=network.target

[Service]
Type=simple
User=filip
WorkingDirectory=/home/filip/<deployed-rag-path>
Environment=QDRANT_URL=http://localhost:6334
Environment=GEN_BASE_URL=http://localhost:8080/v1
Environment=RAG_MCP_BIND_ADDR=127.0.0.1:8082
ExecStart=/home/filip/<deployed-rag-path>/target/release/ep-rag-mcp
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

**Step 2: Add the Makefile target**

```makefile
serve-mcp: ## Run ep-rag-mcp (retrieval + /route) on loopback :8082
	cargo run --release -p ep-rag-mcp
```

**Step 3: Verify** `make serve-mcp` starts and the curl checks from Task 3.2/4.2 pass. Install the
unit only on weebeastie at deploy time (`/etc/systemd/system/ep-rag-mcp.service`; keep the repo copy
in sync — same discipline as `llama-server.service`).

**Step 4: Commit**

```bash
git add deploy/ep-rag-mcp.service rag/Makefile
git commit -m "feat(mcp): systemd unit + make serve-mcp target"
```

### Task 5.4: End-to-end validation

**No new code.** With Qdrant, llama-server, and `ep-rag-mcp` all up, confirm the three faces:

1. **Household (OWUI filter):** EP question → grounded, cited answer + Sources; "what's 2+2" →
   normal answer (no injection); a generic-EU question ("who is the Commission President?") → normal
   answer (the deterministic path avoids the native over-fire the spike found).
2. **qwen-code (MCP tool):** an EP question triggers `search_ep_committee_docs`; the answer is grounded.
3. **Generator swap:** change `GEN_BASE_URL` (or the OWUI persona's base model) and re-run (1) — the
   grounded pipeline is unchanged, proving the loose coupling the design set out to achieve.

Record results in `README.md` (the runbook) and check off the RAG TODO in `CLAUDE.md`.

---

## Notes / deferred (YAGNI now)

- **Per-request `k`** on `/retrieve` and the MCP tool — accepted but ignored; honor later if needed.
- **`structuredContent.hits`** on the MCP tool — text-only is fine for the MVP; add typed content if a
  host wants it.
- **`<think>` stripping** — `strip_think` exists; only wire it if a ThinkingCap model is loaded (the
  default Qwen-Coder emits none). The classify call and the answer both run at temp 0.
- **Embedding-only gate** (skip the classify turn) — cheaper but unproven on the anisotropic borderline
  and needs a calibrated dev set; revisit only if per-message classify latency (~2.6 s) hurts.
- **Retire `rag-server`** — once (1)–(3) pass, remove it from the deploy story (or leave dormant).
```
