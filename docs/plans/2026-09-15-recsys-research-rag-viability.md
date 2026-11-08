# Recsys Research Corpus — Viability

**Date:** 2026-09-15 (revised 2026-09-16)
**Status:** High-level brainstorm. Nothing built. **"Not needed" is an acceptable outcome.**

## The question

Is it worth building a curated corpus of recommender-systems research — books, papers, industry
write-ups, user studies — to give a frontier model better context when brainstorming the design of
recommendation systems?

**Scope is problems, not one field.** Recommender systems share their hard problems with finance,
behavioral science and economics, which often solved them first under different names. The corpus
includes those fields wherever they answer a recsys problem:

| Recsys problem                             | Where adjacent fields work on it                                                                   |
|--------------------------------------------|----------------------------------------------------------------------------------------------------|
| Exploration vs. exploitation               | Economics / operations research: bandits, optimal experimentation                                  |
| Cold start for new items                   | Finance: price discovery for new assets, information aggregation                                   |
| Recommendations change what they recommend | Finance: market impact · Economics: performative prediction                                        |
| User decision biases                       | Behavioral finance: attention-driven trading · Behavioral economics: defaults, choice architecture |
| Platform and multi-sided incentives        | Economics: matching markets, mechanism design, platform economics                                  |
| Fake or manipulated signals                | Finance: market manipulation · Economics: information economics                                    |
| Engagement vs. long-term user welfare      | Welfare economics · Finance: suitability rules                                                     |
| Diversity of recommendations               | Finance: portfolio theory                                                                          |

**Reader:** Claude Opus (1M context) in Claude Code. Opus drives every design choice here.

## Why it might not be needed

Opus already holds the canonical recsys literature in its weights and can search and fetch open
papers itself. A corpus has to beat **Opus + web search**, not Opus alone — and it costs curation
time plus ongoing upkeep to stay current.

## Where it could still win

1. **Material Opus can't fetch** — books, paywalled journals (finance, economics). Not the ACM
   Digital Library: it has been fully open access since January 2026, RecSys proceedings included.
2. **Verification** — pinning a claim to a specific paper, instead of half-remembered numbers.
3. **Curation** — a deliberate selection that includes the reproducibility and harms literature
   and domain-specific work a generic search won't surface.
4. **Cross-field connections** — probably the strongest case. A search phrased in recsys terms
   won't surface a finance or economics paper, because the vocabulary differs; Opus knows those
   fields but won't bridge them unless prompted. A bibliography that maps each document to the
   recsys problem it addresses makes the bridge explicit.

The corpus is a **verification and curation layer, not a knowledge layer**.

## What

An **annotated bibliography**: one record per document (title, authors, year, venue, field, URL,
access = `open | paywalled | book`, the recsys problem(s) it addresses, short summary in our own
words, summary source = `abstract | full_text`), kept in **Zotero** and exported to JSONL for Opus.
The problem field is what links a finance or economics paper to a recsys design question. Full
text only later, and only for documents Opus can't fetch.

**Paywalled work: abstract first, pay only when relevant.** Paywalled papers enter the bibliography
from their abstract; the summary source field tells Opus the summary is abstract-deep and may miss
caveats. A paper is bought only when brainstorming keeps needing its full text. Before paying, look
for a legal free copy — finance and economics papers often exist as SSRN or NBER working papers or
author preprints.

## How

- **Bibliography first.** Opus loads the whole bibliography into context (~30k tokens for 200
  entries) and fetches open papers itself when it needs detail. No embeddings, no index.
- **Full-text index only if the gates below say so,** and only for the documents that need it,
  using the existing `rag` pipeline behind a second `rag-mcp` tool.

## Steps and ordering

| # | Step                                                                                                                                                                                   | Output                                                  | Gate                                                                                      |
|---|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------|-------------------------------------------------------------------------------------------|
| 1 | **Write the eval set** — 10–15 real design questions, each with the documents a good answer should draw on; include questions best answered from finance, behavioral or economics work | Test questions + expected sources                       | —                                                                                         |
| 2 | **Sourcing** — pick seeds and build the bibliography (1–2 books, canonical + industry papers, user studies, adjacent-field work per problem)                                           | Bibliography, ~100–200 entries                          | —                                                                                         |
| 3 | **Compare** — run the eval set three ways: Opus alone · Opus + web search · Opus + bibliography                                                                                        | Side-by-side answers                                    | **A:** bibliography must clearly beat Opus + web search, or **stop**                      |
| 4 | **Use it** for a few weeks of real brainstorming; log every "I'd need the text of X"                                                                                                   | Miss log — doubles as the buy list for paywalled papers | **B:** misses must pile up on documents Opus can't fetch, or **stop at the bibliography** |
| 5 | **Parse + embed design** for that subset only (formats, chunking, embedder)                                                                                                            | Design doc                                              | —                                                                                         |
| 6 | **Implement** — `rag-mcp` gets a second collection and tool                                                                                                                            | Working tool                                            | —                                                                                         |
| 7 | **Small test** — index a subset, re-run the eval set                                                                                                                                   | Recall + answer comparison                              | **C:** must beat step 3's result, or **stop**                                             |
| 8 | **Full run**, then grow toward the cutting edge                                                                                                                                        | Live corpus                                             | —                                                                                         |

**Possible outcomes:**

| Stop at | Outcome                                                                           |
|---------|-----------------------------------------------------------------------------------|
| Gate A  | Not needed; Opus + web search is enough. Keep the bibliography as a reading list. |
| Gate B  | The bibliography is the product.                                                  |
| Gate C  | Full text doesn't pay; the bibliography is the product.                           |
| Step 8  | Full corpus.                                                                      |

The cheap steps (1–4) come first and double as the viability test. The expensive work (5–8) runs
only if they justify it.

## Carried decisions

- **One corpus across fields** — recsys, finance, behavioral, economics — tagged by field and
  problem, not split into separate collections.
- **Nothing copyrighted is committed** — metadata and our own summaries only; the repo is public.
  Journal abstracts are usually copyrighted (arXiv's are CC0), so summarize them rather than copy.
- **The bibliography doubles as the manifest** `rag-mcp` needs at startup, if an index is built.
- **Self-stored.** The library and any full text live on our own machines. Hosted research
  libraries were considered and rejected — e.g. Anara (2026-09-16): would have covered steps 5–8
  with passage-cited search over MCP for $20/mo, but it keeps the library in US data centers and
  exports only on request to support.
- **Zotero is the library** (decided 2026-09-16): browser capture, PDF storage, dedup. Our fields
  map onto it as **tags** (field, problem, access) and a **child note** (summary, tagged with its
  source). An export script turns it into the JSONL Opus loads. Sync stays off, or files go to our
  own WebDAV — zotero.org sync would put the library on a third-party server. The Zotero data
  directory must not live in a CloudStation-synced folder: file sync corrupts its SQLite database.

## TODO / deferred

- **Qwen (local model)** — can't hold the whole bibliography in context; would need the bibliography
  embedded and served through routing (`/route` or an MCP tool). Revisit after step 4.
- **Seeds for step 2:** `grahamjenson/list_of_recommender_systems` (landscape map, products and
  datasets), `eugeneyan/applied-ml` (industry papers and blogs), `hongleizhang/RSPapers` (academic).
