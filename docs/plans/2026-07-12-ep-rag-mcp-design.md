# EP-Committee RAG as an MCP Server — Design

**Date:** 2026-07-12
**Status:** **Design, validated in brainstorming + one live spike.** Supersedes the household half of
`2026-07-11-openwebui-rag-integration-design.md` (Option A, "RAG-as-a-model"). The Rust `retrieve`/
`generate` crates it builds on already exist (branch `rag-server`, 30 tests green). Not yet built.
**Topic:** re-cast the EP-committee RAG from a monolithic OpenAI-compatible *model* into a **shared
MCP retrieval server** that any host on the LAN can attach — the household chat (Open WebUI) *and*
the coding agent (qwen-code) — so the **generator model stays a swappable dropdown** and retrieval
is reused, not re-implemented, per host.

## Why change from Option A

Option A (`rag-server`, an OpenAI-compatible endpoint) owns the *whole* loop: embed → Qdrant →
ground → **call llama-server** → cited answer, presented to Open WebUI as one model, "EP Committees,
grounded." It works, but it **welds retrieval to a specific generator** (`GEN_BASE_URL`) and it is
**not reusable** — an OpenAI `/v1` model is not a tool qwen-code can call mid-task.

The goal here is **loose coupling**: change the generator (in llama-server or the Open WebUI picker)
without touching the RAG, and expose retrieval *once* to every agentic host. That points at MCP.

## The topology that makes it work (and a common MCP misconception)

**MCP connects to the *host* — the thing running the agent loop — never to the model server.**
llama.cpp's `llama-server` is a dumb text generator with no MCP client. The decide-to-call → dispatch
→ feed-results-back cycle lives *above* the model, in the host. So the two hosts (Open WebUI,
qwen-code) each hold their own MCP client, both point at the **same** MCP server, and both
independently use llama-server as their generator. The MCP server and llama-server are **siblings**,
not stacked.

```
                          ┌──────────────────────────────────────────────┐
  qwen-code (CLI) ──MCP──▶│  ep-rag-mcp   (NEW, loopback :8082)           │
   │  generation          │   ├─ MCP Streamable HTTP → tool               │──▶ Qdrant :6334
   └──► llama-server:8080  │   │    search_ep_committee_docs(query, k)     │   (embed query INSIDE
                          │   ├─ POST /retrieve   (plain, grounded ctx)    │    the server → the
  Open WebUI ──┬─native──▶│   └─ POST /route      (classify + retrieve)    │    embedding contract
   (household) │          │         │ reuses ep-rag-generate::client       │    is preserved)
               └─filter──▶│         └─► classify call → llama-server:8080  │
   generation: each host ──► llama-server :8080  (swappable, untouched)    │
                          └──────────────────────────────────────────────┘
```

**One shared MCP server serves both hosts** — this reuse is the whole point, and Option A cannot
give it. Only `:3000` (Open WebUI) faces the LAN; `:8082`, `:6334`, `:8080` stay loopback.

## Decision table

| Option | What | Verdict |
|---|---|---|
| **A. `rag-server` (OpenAI model)** | full loop behind `/v1/chat/completions`; OWUI second model | prior design; **superseded** for the household — welds generator, not reusable |
| **B. MCP retrieval server (this doc)** | retrieval as a tool + `/route`; hosts own generation | **chosen** — generator is a dropdown, retrieval reused by chat *and* qwen-code |
| C. OWUI native RAG (`VECTOR_DB=qdrant`) | OWUI embeds/searches itself | **rejected** (unchanged) — its own embedder ⇒ query/index mismatch (Drill 1) |

## The one hard constraint (unchanged)

The index was built under a pinned embedding contract (bge-small-en-v1.5, query-only instruction
**prefix**, CLS pooling, L2-norm, `citation_id` payload). Retrieval is valid **only** if the query
is embedded under the identical recipe. Therefore embedding happens **inside** `ep-rag-mcp` (reusing
`ep-rag-embed::Embedder`), and the server **asserts live contract == stored contract on boot**
(`EmbeddingContract::assert_matches`). Open WebUI is never the retriever.

## Component: `ep-rag-mcp` (new bin crate, loopback :8082)

A thin new shell over the crates already built — little new logic:

- reuses `ep-rag-retrieve` (embed-under-contract + Qdrant top-k → `Hit`s),
- reuses `ep-rag-core` (the `EmbeddingContract`),
- reuses `ep-rag-generate::provenance::Manifest` (join `citation_id → doc_id → pdf_url`, serve-time,
  no re-index) and `prompt::{SYSTEM, assemble}` (the grounding discipline),
- **repurposes** `ep-rag-generate::client` (the llama-server client) — *not* to generate the answer
  (that is now the host's job) but to run the **classify** call for `/route` (below).

Three faces on the one port:

1. **MCP Streamable HTTP** → tool `search_ep_committee_docs(query, k?=5)`. Returns the grounded
   text block (below) for the model, plus `structuredContent.hits`. For agentic hosts (qwen-code;
   Open WebUI native, if used). Rust MCP SDK (`rmcp`), Streamable HTTP transport (the only transport
   Open WebUI's native MCP supports).
2. **`POST /retrieve`** `{query, k?}` → `{context, hits[], }`. The same grounded block as plain HTTP,
   for any dumb caller.
3. **`POST /route`** `{message}` → `{route, should_ground, context?}`. The household brain: classify
   the message against the decision tree (one llama-server call, Mode-A structured walk), and if the
   route says ground, retrieve + assemble and return the context inline. **All routing logic stays
   in Rust**, unit-tested; the Open WebUI filter becomes trivial.

### The grounded-context payload (the "discipline rides in the result" decision)

Both `search` and `/retrieve` return the **same** block, so the grounding rule lives in exactly one
place and is host-agnostic (an LLM tool-call and the filter consume it identically):

```
Answer USING ONLY the passages below. Cite the passage id in [brackets] after
each claim, e.g. [EMPL-PR-785214:3]. If the answer is not in the passages,
reply exactly: I don't know.

[EMPL-PR-785214:1]
young person under the age of 30 … within four months …

[EMPL-PR-785214:3]
The evaluation of the reinforced Youth Guarantee …

Sources:
- Youth Guarantee — https://ep/EMPL-PR-785214_en.pdf
```

## The decision tree (versioned config, open-ended)

Routing is expressed as **declarative JSON** the server loads — auditable, versioned, and extensible
to more RAG tools by adding nodes/subtrees (no code change). Its node questions are natural language,
evaluated by the model. Skeleton:

```json
{
  "version": "1.0",
  "root": "Q_EP_COMMITTEE",
  "nodes": {
    "Q_EP_COMMITTEE": {
      "type": "question",
      "question": "Is the user asking a substantive question about the content/positions/deadlines of EP committee documents (EMPL, REGI, IMCO)?",
      "help": "YES: 'Youth Guarantee deadline', 'IMCO EV charging', 'REGI cohesion funding'. NO: chit-chat, coding, math, translation, generic EU facts not tied to committee documents.",
      "yes": "R_USE_EP_RAG",
      "no": "R_UNCLASSIFIED"
    },
    "R_USE_EP_RAG":   { "type": "result", "tool": "search_ep_committee_docs" },
    "R_UNCLASSIFIED": { "type": "result", "tool": null,
      "description": "Answer from general knowledge; mark facts unverified; recommend authoritative sources." }
  }
}
```

**The tree is not a rival to a gate — it is the routing structure; the node-evaluator is pluggable.**
In the household path the evaluator is a Mode-A classify call (below); in qwen-code the model walks it
natively.

## The household routing flow (`/route`, deterministic)

```
household msg ─► (1) CLASSIFY  (llama-server, tree in prompt, Mode-A) ─► {"route": …}
                     │
             route == R_USE_EP_RAG ? ─yes─► retrieve + assemble grounded context ─┐
                     │                                                            ▼
                     └─no──────────────────────────────────────────► (2) GENERATE (grounded | normal)
                                                                         answer [+ Sources if grounded]
```

The model **never emits a tool_call** here — the *code* decides from parsed JSON, so it is fully
deterministic with the classifier's proven accuracy. Step (1) is Rust (`/route`); step (2) is the
host's ordinary generation with the returned `context` injected. **The generator is a swappable
dropdown** — the loose-coupling goal, achieved.

## Evidence — the spike (2026-07-12, live against IQ4_XS on weebeastie)

Ran the tree + 10 labelled queries (4 in-domain EP, 4 clearly off-topic, 2 hard "generic-EU-fact"
borderline) in two modes. `scratchpad/tree_spike.py`.

| Mode | Routing | Format | Note |
|---|---|---|---|
| **A — walk tree → emit JSON decision** | **10/10** | 10/10 valid JSON | correct even on both hard borderline cases (~2.6 s/q warm) |
| **B — native tool-calling** | **8/10** | 10/10 valid tool args | 0 missed EP Qs; the 2 errors were **over-firing** on borderline generic-EU questions |

**Read:** Qwen *can obey the tree flawlessly* (A). The weak spot is that **native tool-choice
short-circuits the tree walk** (B) — it pattern-matches "EU institution → call the EU tool" and skips
the reasoning, over-firing on borderline questions. An over-fire is costly: it retrieves irrelevant
chunks, and the grounding prompt then forces "I don't know" to a question the model actually knows.
**Conclusion:** the household path uses the **Mode-A classify** evaluator (10/10), not native
tool-calling. n=10 — directional, judged sufficient to choose the architecture.

## Host integration

- **qwen-code** — add an `mcpServers` entry in `~/.qwen/settings.json` pointing at
  `http://localhost:8082` (Streamable HTTP). Native tool-calling; Mode B's borderline over-fire is
  acceptable for a coding agent with a human in the loop. (Verify the transport key against 0.19.8.)
- **Open WebUI (household)** — a thin inlet **Filter Function**: on `inlet`, `POST /route`; if
  `should_ground`, prepend the returned `context` as a system message into `body["messages"]`; else
  pass through. Generation proceeds normally on whatever model the persona uses. One self-routing
  model — no picker. (A native-MCP variant is possible — OWUI ≥ v0.6.31, admin-added Streamable HTTP,
  Native/Agentic mode — but the spike showed native over-fires on borderline, so the filter path is
  the production choice.)

## Fate of the existing `rag-server`

Superseded for the household by `/route` + filter. **Retire it, or keep on ice** as a deterministic
guaranteed-grounding fallback. Its crates are reused, not discarded — `ep-rag-mcp` is largely the
same parts re-wired.

## Deployment

Mirror `llama-server`: a **loopback systemd unit** on weebeastie (`:8082`, `Restart=always`,
`enabled` at boot), config via env (`QDRANT_URL`, `GEN_BASE_URL`, `TREE_PATH`, `MANIFEST_PATH`,
`TOP_K`, `RAG_BIND_ADDR`). Index already lives in Qdrant; snapshot-migrate per the Option-A plan's
Phase 4 (contract travels in the payload). Open WebUI reaches `:8082` over host networking
(`localhost:8082`; else `host.docker.internal:8082`).

## Out of scope / later

- Multi-tool trees (more RAG MCP servers as new leaf subtrees) — the tree is built for it; not now.
- Reranker / MMR / dedup / `p_eta`-weighted fusion — the Drill-2 track, unchanged.
- The two named debugging drills + eval harness (recall@k/MRR vs groundedness+citation-accuracy).
- An embedding-only gate (one bge embed, no classify turn) — cheaper, but unproven on the anisotropic
  borderline and needs a calibrated dev set; the Mode-A classify won on accuracy. Revisit if the
  per-message classify latency (~2.6 s + some KV churn under `--parallel 1`) hurts.

## Don't relearn (key decisions)

- **MCP attaches to the host, not llama-server.** llama-server has no MCP client; hosts orchestrate.
- **One shared Streamable-HTTP MCP server** (not stdio) so both hosts connect to one running process.
  Open WebUI's native MCP is **Streamable-HTTP-only, admin-added** (Admin → External Tools, v0.6.31+).
- **OWUI tool-calling collapsed to one supported mode: Native/Agentic** (v0.10.0; old "Default" →
  "Legacy", unsupported). There is **no "always-call this tool" switch** — hence the deterministic
  `/route` for guaranteed grounding. OWUI's own docs warn small local models are unreliable at native
  tool-calling; our spike quantified it (over-fire on borderline).
- **Discipline rides in the tool result** (host-agnostic), grounding logic lives once in Rust.
- **Contract-assert on boot** stays — the one hard constraint.
- **The tree is structure; the evaluator is pluggable** (Mode-A classify in the household path, native
  in qwen-code).

## References

- Prior (superseded-for-household) integration design: `docs/plans/2026-07-11-openwebui-rag-integration-design.md`
- `rag-server` build plan (crates reused here): `docs/plans/2026-07-11-rag-server-implementation-plan.md`
- EP-committee RAG design + status: `docs/plans/2026-07-10-ep-committee-rag-design.md`
- Executable spec for retrieval/grounding: `rag/notebooks/rag_query.org`
- Spike script + results: `scratchpad/tree_spike.py` (Mode A 10/10, Mode B 8/10)
- Open WebUI docs: MCP (`/features/extensibility/mcp/`), Tools & tool-calling modes
  (`/features/extensibility/plugin/tools/`), Filter Functions
  (`/features/extensibility/plugin/functions/filter/`)
