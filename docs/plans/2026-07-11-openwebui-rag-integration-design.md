# Open WebUI ↔ EP-Committee RAG — Integration Design

**Date:** 2026-07-11
**Status:** **design only — NO implementation yet.** Blocks on the Rust `retrieve`/`generate`
crates (currently hand-stitched in `rag/notebooks/rag_query.org`) and the Qdrant→weebeastie move.
**Topic:** wire the EP-committee RAG (Qdrant + our bge retriever + grounded Qwen) *behind* the
Open WebUI chat at http://192.168.1.22:3000, so any household user gets grounded, cited answers.
This is the concrete follow-through on the openwebui plan's "Document RAG: off for now — enable
later."

## Goal

A household member opens the chat, picks the **"EP Committees (grounded)"** model, asks a policy
question in plain language, and gets an answer **grounded in and cited to** the committee
documents — or an honest **"I don't know."** No uploads, no config, no knowledge of embeddings.
Everything runs on weebeastie; only Open WebUI's `:3000` faces the LAN.

## The one hard constraint

The index was built under a pinned **embedding contract** (bge-small-en-v1.5, query-only
instruction prefix, CLS pooling, L2-norm, `citation_id` payload). Retrieval is valid *only* if the
**query** is embedded under the identical recipe. Therefore **Open WebUI must NOT be the
retriever** — its built-in RAG uses its own embedder/chunker and payload, which is exactly the
query/index mismatch we built Drill 1 to expose. Open WebUI is the **chat UI in front of** our
pipeline; our pipeline owns retrieval end to end.

## Decision: RAG-as-a-model (an OpenAI-compatible `rag-server`)

| Option | What | Verdict |
|---|---|---|
| **A. `rag-server` — OpenAI-compatible endpoint** | our Rust `retrieve`+`generate` behind `/v1/chat/completions`; registered in Open WebUI as a second model connection | **chosen** — cleanest separation, preserves the contract, zero OWUI RAG config |
| B. Open WebUI **Pipe Function** | a Python function inside OWUI intercepts the chat, calls our retriever + llama-server | rejected as primary — couples logic to OWUI's runtime + Python; re-implements or just calls our service anyway |
| C. Open WebUI **native RAG** (`VECTOR_DB=qdrant`) | OWUI embeds/chunks/searches itself | **rejected** — its own embedder ⇒ query/index mismatch (Drill 1 *by construction*), no citation contract, aimed at user uploads not a curated index |

Option A is the design's stated intent — "Open WebUI sits in front of the Rust pipeline as the
chat UI, not as the retriever" — and it is literally the **productization of `rag_query.org`**:
the notebook's `retrieve → assemble → generate` becomes a small HTTP service.

## Architecture (all on weebeastie; loopback between services)

```
LAN device ──http://192.168.1.22:3000──► Open WebUI ─┬─► llama-server 127.0.0.1:8080/v1   ("Qwen", bare)
   (household)                            (chat UI)   └─► rag-server   127.0.0.1:8081/v1   ("EP Committees, grounded")
                                                                          │
      rag-server:  question ─► bge embed_query (PREFIX) ─► Qdrant 127.0.0.1:6333 top-k
                            ─► assemble grounded prompt ─► llama-server 127.0.0.1:8080 ─► stream cited answer
```

Open WebUI gains a **second** OpenAI connection (it already points at llama-server). The
`rag-server` is itself a client of both Qdrant and llama-server. Only `:3000` faces the LAN;
`:6333` / `:8080` / `:8081` stay loopback.

## Components (to build later — this doc is design only)

1. **`rag-server`** — the `generate` crate wrapped as an OpenAI-compatible HTTP service
   (`GET /v1/models`, `POST /v1/chat/completions` incl. **streaming** SSE). Per request: take the
   latest user turn as the question → `embed_query` (bge, prefix) → Qdrant top-k → `assemble`
   grounded prompt (cite `citation_id`; "I don't know" if absent) → call llama-server → stream the
   cited answer. Env-driven (`QDRANT_URL`, `GEN_BASE_URL`, `EMBED_MODEL`, `TOP_K`) — one binary,
   laptop↔weebeastie a config swap. If a **ThinkingCap** reasoning model is loaded, strip
   `<think>…</think>` before returning (and note it inflates tokens under `--parallel 1`).
2. **Qdrant on weebeastie** — `deploy/qdrant/` compose is already portable; run it beside
   llama-server + Open WebUI. Migrate the index (below).
3. **Open WebUI config** — admin adds the `rag-server` base URL as a **second** OpenAI connection
   (Admin → Connections, or `OPENAI_API_BASE_URLS`). Name the model. Keep
   `BYPASS_EMBEDDING_AND_RETRIEVAL=true` (we never use OWUI's RAG). `ENABLE_DIRECT_CONNECTIONS=false`
   stays — admin-only config; users just select the model.

## Migrating the index to weebeastie

Prefer **snapshot transfer** (no re-embed, deterministic; the contract travels in the payload so
the restored collection is self-describing):

- **Snapshot (preferred):** `POST /collections/ep_committee_docs/snapshots` on the laptop →
  download → copy to weebeastie → restore via the snapshot upload API. Avoids rebuilding candle +
  re-embedding on the remote.
- **Re-ingest:** run `make fetch ingest index` on weebeastie (needs the candle build + bge cache
  there). Slower, but the natural path once ingestion also lives on the remote.

## Citations & provenance UX (household-facing)

The answer carries inline `[committee-type-num:idx]` markers. For non-technical users the
`rag-server` should also append a **Sources** block: each cited `citation_id` → document title
(payload) → the source PDF URL. **Gap to decide:** `source_url` is *not* in the current payload
(only `doc_id`, `committee`, `title`, `doc_type`, `chunk_index`, `n_chunks`, lengths, contract).
Options: (a) stamp `source_url` at index time (needs a re-index) or (b) join
`citation_id → doc_id → manifest.pdf_url` at serve time in `rag-server`. (b) avoids a re-index.

## Access, trust, latency

- **Users:** the fixed household accounts already exist; they just pick the grounded model. No new
  auth, no new LAN surface (`rag-server` is loopback-only).
- **Trust:** the grounding prompt yields honest **"I don't know"** over hallucination, and inline
  citations make every claim checkable — the whole point of a household knowledge base. (Note the
  observed faithfulness nuance: content can be grounded while the *citation* is mis-attributed —
  see the RAG design doc's eval section.)
- **Latency:** cold-start ~168 s (model fault-in) + multi-chunk prefill (~2k tokens ≈ 30–40 s) ⇒
  **stream** so first tokens appear early, and **warm on boot** (a startup ping). `--parallel 1`
  serializes household requests (they queue); revisit if concurrency hurts.

## Out of scope / later

- The two named debugging drills + eval harness (separate track — the immediate next work).
- Reranker, multilingual, hybrid BM25 (the collection is dense-only by decision).
- Per-user document upload (this is a curated *shared* index, not user RAG).
- Auto-refresh of the index from the ODP change feeds.

## References

- EP-committee RAG design + status: `docs/plans/2026-07-10-ep-committee-rag-design.md`
- Open WebUI deploy + "RAG off for now": `docs/plans/2026-07-08-lan-chat-frontend-openwebui-design.md`, `deploy/openwebui/`
- Hand-stitched query path (the executable spec for `rag-server`): `rag/notebooks/rag_query.org`
- Index-health drill: `rag/drills/drill0_index_health.org`
