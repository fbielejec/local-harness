# EP-Committee RAG — Design

**Date:** 2026-07-10
**Status:** `index` built (490 pts) + query path hand-stitched (`rag/notebooks/rag_query.org`)
+ Drill 0 index-health done — 2026-07-11. The two named drills (embedding-mismatch,
confidence-blind fusion) are next; then Rust `retrieve`/`generate` crates + eval + Open WebUI
wiring (`docs/plans/2026-07-11-openwebui-rag-integration-design.md`). Live checklist: `CLAUDE.md` TODOs.
**Topic:** an "almost production" retrieval-augmented-generation system over European
Parliament committee documents (EMPL / REGI / IMCO), reusing the local Qwen `llama-server`
as the generator — built to double as a hands-on RAG-debugging rig.

## Goal

Two heads on one system:

1. **Something useful.** A RAG assistant over the working documents of three EP committees —
   **EMPL** (Employment & Social Affairs), **REGI** (Regional Development), **IMCO** (Internal
   Market & Consumer Protection). Ask a policy question → get a grounded, **cited** answer from
   the local Qwen. Eventual home: `weebeastie`, beside `llama-server` + Open WebUI, wired into
   the chat (the "Document RAG: off for now — enable later" left open in the Open WebUI plan).
   Real end user: an MEP office covering these committees.
2. **A diagnostic rig.** Every stage is observable, and two deliberate failure switches let us
   reproduce and *bisect* the canonical RAG failures on **real** documents (not a 3×3 toy),
   backed by an eval harness that separates retrieval quality from generation quality. This is
   direct prep for the interview's live RAG-debugging exercise (Beat 3).

## Scope now / non-goals

**In scope:** the ingestion pipeline, retrieval + grounded generation, the two debugging
drills, and the eval harness. **Later (see Out of scope):** Open WebUI wiring, remote deploy to
`weebeastie`, a cross-encoder reranker, multilingual ingestion.

## Implementation status — maximal-Rust pivot (2026-07-10)

The pipeline is a **Rust cargo workspace** (`rag/`), per the decision to maximize Rust. The two
technical risks were retired with acceptance gates on real data:

| Gate | Result | Decision |
|------|--------|----------|
| PDF parse — `pdf-extract` vs `pymupdf` (incl. 2-col amendment tables) | equivalent, <5% char delta, reading order preserved | pure-Rust **`pdf-extract`** (no native dep) |
| Embed parity — `candle` vs `sentence-transformers` | **cosine = 1.00000** on all samples (passages + queries) | pure-Rust **`candle`** |

**Embed pivot (important):** `fastembed` (ONNX Runtime) **cannot link on this box** — glibc 2.35,
but its ORT prebuilt needs ≥2.38 (`__isoc23_*` undefined symbols). So we use **`candle`** (pure
Rust, no C++). candle owns CLS pooling + L2-norm; the parity gate proves it matches
`sentence-transformers`, so the Rust index and the Python drills share **one vector space**.

**Contract correction:** bge-small uses **CLS pooling** (not mean) — pinned into the contract.

### Rust workspace (`rag/crates/`)
- `core` — `EmbeddingContract` (cross-language anchor; pinned `pooling=cls`), unit-tested.
- `fetch` — ODP list/select/resolve/download → `manifest.jsonl` (`reqwest` blocking + retry).
- `parse` — `pdf-extract` text extraction (+ `parse-gate` bin).
- `chunk` — boilerplate strip + token-bounded recursive split (`text-splitter`+`tokenizers`), tested.
- `embed` — candle BGE: CLS pool, L2-norm, query-prefix (+ `embed-gate` bin).
- `ingest` — bin: `manifest → parse → chunk → chunks.jsonl`.
- **TODO** `index`, `retrieve`, `generate`, `eval`.

### Python floor (`rag/drills/`)
Deliberately Python (interview-prep, PyTorch): the two debugging drills + `parity_gate.py`.
Nothing else is Python — the recon `config.py`/`fetch.py`/`smoke_qdrant.py` were removed.

### Done vs TODO
- **DONE (boilerplate):** `core`, `fetch`, `parse`, `chunk`, `ingest`→`chunks.jsonl`; both gates.
  12 EP PDFs → 490 chunks, all ≤512 tokens, zero boilerplate leakage.
- **TODO (tomorrow, drill-connected):** embed chunks + **upsert to Qdrant with the
  contract-stamped payload** (Drill-1 surface); `retrieve` + the assembly/**fusion switch**
  (Drill-2 surface); grounded `generate`; the retriever↔generator contract; the two Python
  drills; the eval harness.

### Interface (from the Rust/Qwen↔Qdrant research)
Orchestrator-mediated now (Qwen = pure generator over `/v1/chat/completions`); tool-calling later
(both pure Rust via `async-openai` + `qdrant-client`). Open WebUI eventually sits **in front** of
the Rust pipeline as the chat UI, not as the retriever. Avoid `mcp-server-qdrant` on the hot path
(its FastEmbed/own-payload design breaks our prefix + hybrid + citation contract).

## Decisions (what & why)

| Question | Decision | Why |
|----------|----------|-----|
| RAG pattern | **Industry "retrieve-then-concatenate"** (no marginalization) | What real systems + Open WebUI actually do; a single conditional `p(y \| x, fuse(z))`. The retriever/generator gradient decoupling is *why* we debug the halves independently. |
| Vector DB | **Qdrant** (Docker) | Mirrors the real compliance-pipeline experience; metadata filtering **and** native sparse vectors for BM25 hybrid (exact IDs like `2024/0123(COD)`). |
| Embeddings | **`BAAI/bge-small-en-v1.5`** (384-dim, cosine) | Small, CPU-friendly, versionable. Its **query-only instruction prefix** is a built-in, realistic trigger for the embedding-mismatch drill. |
| Generator | **Remote Qwen3-Coder-30B** via `llama-server` OpenAI-compatible API (`localhost:8080/v1` over SSH tunnel) | Already running, already tuned; treated as a black box behind a configurable `base_url` (swap to local Ollama in one env var). |
| Corpus | EMPL/REGI/IMCO **2024–2025 draft reports (`-PR-`) + adopted opinions (`-AD-`)**, EN | Substantive text; ~20–40 docs/committee, ~100–150 PDFs, ~20–30 MB — trivially local. |
| Fetch source | **EP Open Data Portal API** (backbone) + **OEIL XML** (committee index) | Both no-auth, both verified 2026-07-10; ODP yields the real PDFs, OEIL is the authoritative lead-committee filter. |
| Distance | **Cosine** (vectors L2-normalized) | BGE is cosine-trained. |
| Chunking | **Structure-aware** (~400–600 tok, ~10–15% overlap) | EP docs are numbered (recitals/articles); "too big = diluted, too small = fragmented." |
| Config | **Env-driven** (`.env`, gitignored) | One code path, two deploys (laptop ↔ weebeastie) — a config swap, never a code change. Mirrors `deploy/openwebui`. |
| Embedding contract | **Pinned in config AND stamped into every Qdrant payload** | Makes query/index mismatch detectable *by construction* — refuse mismatched serves. |

## Architecture

Industry-RAG shape — retrieve-then-concatenate, the retriever's scores used only to select
top-k and then discarded (no `∑_z` marginalization):

```
INGEST
  ODP API ─► fetch EN PDFs ─► parse (structure-aware) ─► chunk ─► BGE embed ─► Qdrant upsert
  OEIL XML ─► committee→procedure index (provenance + golden-set metadata)

QUERY
  question ─► BGE embed (SAME model+version+prefix!) ─► Qdrant top-k (± BM25 hybrid)
           ─► assemble context ──[fusion switch]──► grounded prompt ─► Qwen ─► answer + citations
```

**Topology — one code path, two configs:**

```
  LOCAL (dev, now)                          REMOTE (weebeastie, later — TODO)
  ┌───────────────────────────┐            ┌──────────────────────────────────┐
  │ Qdrant (Docker, laptop)   │            │ Qdrant (Docker) ─┐                │
  │ BGE embed (local)         │            │ ingestion run here                │
  │ generator ──ssh tunnel──► │            │ Open WebUI ──► our retriever/pipe │
  │   localhost:8080/v1 (Qwen)│            │ llama-server (Qwen, loopback)     │
  └───────────────────────────┘            └──────────────────────────────────┘
       swap = QDRANT_URL / OPENAI_BASE_URL / EMBED_MODEL in .env
```

## Data model (Qdrant)

One **point per chunk**. Named vectors so hybrid is available: `dense` (BGE) and optional
`sparse` (BM25). Distance = cosine.

Payload on every chunk:

- **Provenance:** `committee` (EMPL/REGI/IMCO), `doc_type` (`PR`/`AD`/…), `title`, `date`,
  `procedure_ref`, `source_url`, `doc_id`, `chunk_index`, `n_chunks`.
- **Citation id:** `doc_id:chunk_index` (stable — the generator cites this).
- **Embedding contract:** `embed_model`, `embed_version`, `dim`, `normalized`,
  `query_instruction`. On serve, the query path asserts its live contract equals the stored
  one; mismatch → refuse (loud), don't silently return wrong chunks.

## Ingestion / fetch (verified sources, 2026-07-10)

Two no-auth sources, used together.

**Backbone — EP Open Data Portal** (`https://data.europarl.europa.eu/api/v2`, JSON-LD via
`Accept: application/ld+json`):

- *List:* `/committee-documents?year=YYYY&limit=&offset=`. **Gotcha:** the API *silently
  ignores unknown query params* — there is **no server-side committee filter** (`author=org/EMPL`
  returns bytes-identical to unfiltered). Scope **client-side by ID prefix** (`EMPL-`, `REGI-`,
  `IMCO-`) + `work_type`. Results are alphabetical by identifier → committees form contiguous
  blocks → offset-seek, don't full-scan.
- *Detail:* `/documents/{ID}?language=en` → walk the manifestation chain
  `is_realized_by[] → is_embodied_by[] → is_exemplified_by[]` with sibling `media_type`
  `.../application/pdf` to get the **distribution path** (not guessable from the ID).
- *Download:* `GET https://data.europarl.europa.eu/<distribution-path>` → `200 application/pdf`.
  **Avoid** `www.europarl.europa.eu/doceo/…` — returns HTTP 202 (async render gateway) to
  scripts.

**Committee index — OEIL** (`https://oeil.europarl.europa.eu/oeil`, needs a browser
`User-Agent`): `…/search/export/XML?committeeResponsible=EMPL` → authoritative lead-committee
procedure list (`reference`, `title`, `rapporteur`, `lastpubdate`). Verified all-time counts:
EMPL 464, IMCO 456, REGI 312. Doubles as free **golden-set metadata** for evals.

`work_type` → `doc_type` map: `REPORT_PARLIAMENTARY_COMMITTEE_DRAFT`→`PR`,
`OPINION_PARLIAMENTARY_COMMITTEE`→`AD`, `AMENDMENT_LIST`→`AM`, `REPORT_PLENARY`→`A`.

## Chunking

Structure-aware first: split on document structure (headings, numbered recitals/articles),
recursive-character split with overlap only as fallback. Target ~400–600 tokens, ~10–15%
overlap. Each chunk keeps parent metadata + its `doc_id:chunk_index` citation id. EP docs share
heavy standard boilerplate (legal citations, stock recitals) → **natural near-duplicates**, the
real substrate for Drill 2.

## Retrieval & the assembly seam

Query → BGE embed (with the **query instruction prefix**) → Qdrant top-k (dense, optional
BM25 hybrid for exact refs). Then the **fusion switch** — the retriever↔generator seam:

- `mean-pool` (industry default): concat/average chunks equally, `p_η` discarded.
- `p_eta_weighted`: weight chunk contributions by retriever confidence (marginalized-RAG flavor).

This single toggle *is* Drill 2.

## Generation (grounded)

OpenAI-compatible client, `base_url` configurable. **Grounding system prompt:** answer *only*
from the provided context, cite chunk ids, say **"I don't know"** if the answer isn't present.
Two constraints from the harness: it's a **coding** model (grounding discipline matters more,
not less), and `--parallel 1` **serializes** requests (evals run sequentially; keep the golden
set modest).

## The two drills (first-class, run on the real index)

- **`drill1_embedding_mismatch.py`** — index with the correct BGE contract, then query with the
  **query instruction prefix dropped** (or a different model/version). Watch recall@k collapse;
  run the **self-retrieval probe** (a doc re-embedded as a query must return itself — label-free
  detector); show the **contract check** catching it; restore the prefix → recall returns.
- **`drill2_confidence_blind_fusion.py`** — pick a query whose top-k pulls natural boilerplate
  near-duplicates; contrast `mean-pool` vs `p_eta_weighted`; watch dupes outvote the gold chunk;
  show **rerank / MMR / dedup-at-ingestion** restoring it, with faithfulness moving in step.

## Eval harness

Golden set of ~15–30 items: `question`, `gold_answer`, `gold_doc_ids`, `committee`
(seeded from OEIL metadata + hand-authored Qs). Two metric families, kept **strictly separate**
(the separation *is* the bisection):

- **Retrieval** (no LLM): **recall@k**, **MRR** vs `gold_doc_ids`.
- **Generation** (needs output): **faithfulness/groundedness** + answer-relevance via
  LLM-judge (same Qwen endpoint) with a lexical-overlap backstop.

## Repo layout (in `local_coding_harness`)

```
rag/
  ingest/     fetch → parse → chunk → embed → upsert
  retrieve/   embed query → Qdrant search (±hybrid) → assemble context
  generate/   OpenAI-compatible client → grounded prompt → answer+citations
  drills/     drill1_embedding_mismatch.py · drill2_confidence_blind_fusion.py
  eval/       golden set · recall@k/MRR · faithfulness/groundedness
  config.py   env-driven single source of truth (+ embedding contract)
deploy/qdrant/docker-compose.yml
docs/plans/2026-07-10-ep-committee-rag-design.md   (this file)
```

## Error handling (production-shaped)

- Empty / low-score retrieval → generator answers **"I don't know"**, never hallucinates.
- Embedding-contract mismatch → **refuse the serve**, loud error.
- PDF parse failure / empty chunk → skip + logged warning; never index an empty vector.
- Generator timeout / `--parallel 1` queue backpressure → bounded retry, clear surfacing.

## Verification (evidence before "done")

1. `docker compose up` → Qdrant healthy; a smoke upsert + search round-trips.
2. Ingest one committee → collection count > 0; a spot-checked chunk carries full provenance +
   the embedding contract.
3. A known-gold question retrieves the gold `doc_id` in top-k (recall@k > 0), and Qwen returns a
   grounded, cited answer.
4. `drill1` reproduces the recall collapse **and** the self-retrieval probe flags it; the fix
   restores recall@k.
5. `drill2` shows mean-pool losing to the dupes and `p_eta_weighted`/rerank restoring the gold
   answer; faithfulness moves with it.
6. Eval harness prints separate retrieval vs generation numbers on the golden set.

## Debugging (Beat 3)

| Talking point | Artifact that rehearses it |
|---|---|
| Bisect retrieval vs generation by logging retrieved chunks | Per-query chunk/score logging |
| Retriever score is a product `d(z)·q(x)`; isolate each factor | `drill1` (index clean, query encoder/prefix broken) |
| Self-retrieval as label-free probe | `drill1` self-retrieval check |
| Store the embedding contract; refuse mismatched serves | Payload contract + serve-time assert |
| Confidence-blind fusion; dupes outvote gold | `drill2` fusion switch |
| Symptom patch (shrink-k/dedup) vs root-cause (rerank/MMR/weight) | `drill2` fix ladder |
| "How would we know it's fixed?" — split metrics | Eval harness (recall@k/MRR vs faithfulness) |

## Out of scope / later

- **Open WebUI wiring** on weebeastie (either `VECTOR_DB=qdrant` or expose our retriever as a
  tool/pipe) — the concrete "enable Document RAG later" follow-through.
- **Remote deploy** of Qdrant + ingestion to weebeastie.
- **Cross-encoder reranker** as a permanent stage (drills demonstrate it; productionizing later).
- **Multilingual** ingestion (all 24 languages) and Think-Tank studies/briefings.
- **Incremental ingestion** via the ODP `/{collection}/feed` change feeds.

## References

- Org doc RAG section: `~/CloudStation/LLMs/research/LLMs_README.org` (Lewis et al. 2020
  marginalization; industry retrieve-then-concatenate; gradient decoupling).
- Prep doc RAG debugging drills: `~/CloudStation/TeXProjects/CV/interview_prep_aiinfra.org`
- Verified EP source endpoints: research pass 2026-07-10 (this design's §Ingestion).
