# Drill 1 — Embedding Mismatch (query/index encoder drift) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build `rag/drills/drill1_embedding_mismatch.org` — a hand-holding, runnable org-mode
notebook that reproduces the canonical RAG "confidently wrong facts" failure caused by a
**query/index embedding mismatch**, on the REAL 490-chunk EP-committee index, and bisects it the
way the toy example in `~/CloudStation/TeXProjects/CV/interview_prep_aiinfra.org` (Beat 3, Drill 1)
does — but on real vectors.

**Architecture:** A read-only diagnostic notebook against the **remote** Qdrant (over the SSH
tunnel, `:16333`), collection `ep_committee_docs`. It embeds golden-set queries in Python
`transformers` (bge-small) under a *parameterized* recipe, so we can flip ONE fault — **mean pooling
instead of bge's required CLS pooling** — and watch retrieval quality collapse, the self-retrieval
probe catch it, and the embedding contract refuse the serve. Then restore CLS and recall returns.
The notebook IS the deliverable (self-documenting, like `drill0_index_health.org`); there is no
separate design doc.

**Tech Stack:** Emacs org-babel (`jupyter-python`, `:session llms :kernel llms_kernel :async yes
:exports both`) · Python `transformers` + `torch` + `numpy` · Qdrant REST over the tunnel · the
existing `rag/drills/` uv project.

---

## Context the executor needs (read before starting)

- **The toy this mirrors:** `~/CloudStation/TeXProjects/CV/interview_prep_aiinfra.org`, Beat 3, the
  first diagnostic exercise ("confidently wrong factual answers" → Drill 1). Its talking points
  (lines ~239–264) are the prose spine: *bisect before touching anything · the retriever score is a
  product `d(z)·q(x)` · clear the index by reading stored vectors · the fault is the query encoder ·
  confirm label-free with self-retrieval · fix = mitigate→migrate→prevent (store the embedding
  contract, refuse mismatched serves) · eval separates plumbing (recall@k/MRR) from quality
  (faithfulness).* Cross-reference these explicitly in the notebook prose so the drill reads as "the
  toy, on real vectors."
- **The pattern to match:** `rag/drills/drill0_index_health.org` — same header, same Qdrant REST
  helpers (`_post`, `fetch_all`), same `embed_query` built from `AutoTokenizer`/`AutoModel`. Drill 0
  even ends teasing this drill ("break it (drop the prefix) to preview Drill 1"). **Reuse its
  helpers verbatim** where possible; do not reinvent.
- **The one fault we inject (LOCKED decision):** **mean pooling instead of CLS.** bge-small-en-v1.5
  requires **CLS pooling**; the index was built with CLS (`contract_pooling="cls"`). Mean pooling is
  a *genuine encoder drift* — the SAME text produces a DIFFERENT vector — so unlike a dropped prefix
  it IS caught by the self-retrieval probe (maps 1:1 to the toy's climax where the drifted query
  tower gives `self-retrieval cos≈0.01`). Do NOT use the dropped-prefix fault here (self-retrieval
  is blind to it on real data — that is a different, richer drill deliberately out of scope).
- **Same vector space (why Python queries can search the Rust index):** the parity gate proved
  candle (Rust index) == `sentence-transformers`/`transformers` (Python) at **cosine 1.0**. So a
  Python `transformers` query embedding lands in the same space as the stored vectors. CLS pooling +
  L2-norm + the query instruction prefix is the contract; keep the prefix CORRECT throughout this
  drill (we are breaking *pooling*, not the prefix).
- **Anisotropy caveat (from `drill0`):** the index is a tight cone (mean pairwise cosine ~0.68);
  **do not threshold raw cosine** for relevance. This drill uses *ranking* metrics (recall@k / MRR)
  and *relative* collapse (correct ≫ broken), never an absolute cosine cutoff.
- **Contract fields in every payload:** `contract_model` (`BAAI/bge-small-en-v1.5`),
  `contract_version` (`v1`), `contract_dim` (`384`), `contract_normalized` (`true`),
  `contract_pooling` (`cls`), `contract_query_instruction`
  (`Represent this sentence for searching relevant passages:`). The contract check reads these and
  mirrors the Rust `EmbeddingContract::assert_matches`.
- **Remote, read-only:** all Qdrant calls are `search` / `scroll` — no writes. The index is
  production; never upsert/delete from this notebook.

## Prerequisites (do once, before any task)

1. **Open the Qdrant tunnel to the remote** (from the repo root on the laptop):
   ```bash
   make tunnel-qdrant     # ssh -fN -L 16333:127.0.0.1:6333 -L 16334:127.0.0.1:6334 filip@192.168.1.22
   curl -s http://localhost:16333/collections/ep_committee_docs | grep -o '"points_count":[0-9]*'
   # expect: "points_count":490
   ```
   Close it at the end with `make tunnels-stop`.
2. **Kernel/deps:** the notebook uses `:kernel llms_kernel`. Confirm `transformers`, `torch`, `numpy`
   are importable in that kernel (they are what `drill0` already uses). If running headless instead
   of Emacs: `cd rag && uv run --project drills python -c "import transformers, torch, numpy"`.
3. **First run downloads bge-small** (~130 MB) into the HF cache — one-time, needs network.

## How to "test" an org notebook (adapt TDD to this deliverable)

Each task adds ONE `#+BEGIN_SRC jupyter-python … #+END_SRC` block plus its prose. The "test" is:
**run the block and confirm its `#+RESULTS:` shows the claimed behavior** (e.g. recall collapses,
self-retrieval misses, contract refuses). Paste the REAL output into a `#+RESULTS:` block beneath
the source (exactly as `drill0` does). If a golden query does not retrieve its gold doc even under
the CORRECT recipe (Task 4), swap it for a more distinctive question — the baseline must be strong
for the collapse to be meaningful. Commit after each task.

---

## Task 1: Notebook skeleton + remote-Qdrant setup block

**Files:**
- Create: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the title + intro prose**

```org
#+TITLE: Drill 1 — Embedding Mismatch (query/index encoder drift)

* Drill 1 — query/index embedding mismatch

Symptom (support tickets): the assistant answers a policy question fluently, cites something
that looks real — but the fact is *wrong*. Same surface as a hallucination; different cause.

Toy analog (=interview_prep_aiinfra.org=, Beat 3, Drill 1): the retriever score is a product
=d(z)·q(x)= — two encoders writing into one shared space. A bad score never says WHICH factor
broke. We bisect: is the gold chunk even retrieved? If no → the *retrieval* half. Then, since
Drill 0 already cleared the index =d(z)= (healthy, well-separated), the fault must be the *query
encoder* =q(x)=. Here we inject a realistic query-side drift — **mean pooling instead of bge's
required CLS** — and watch recall collapse, the self-retrieval probe catch it, and the stored
embedding contract refuse the serve.

We run READ-ONLY against the REMOTE index over the tunnel (=:16333=).
```

**Step 2: Write the setup block** (reuses `drill0`'s helpers + pulls a sample of points for the
probe):

```org
** Setup — connect to the remote index

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  import json, urllib.request
  import numpy as np

  # 16333 = SSH-tunnelled remote Qdrant (make tunnel-qdrant). 6333 if running locally.
  QDRANT, COLLECTION = "http://localhost:16333", "ep_committee_docs"

  def _post(path, body):
      req = urllib.request.Request(QDRANT + path, data=json.dumps(body).encode(),
                                   headers={"content-type": "application/json"})
      return json.load(urllib.request.urlopen(req))["result"]

  def search(vec, k=5):
      """Top-k against the remote index. Returns [(citation_id, doc_id, score), ...]."""
      r = _post(f"/collections/{COLLECTION}/points/search",
                {"vector": vec.tolist(), "limit": k, "with_payload": True})
      out = []
      for h in r:
          cid = h["payload"]["citation_id"]
          out.append((cid, cid.split(":")[0], h["score"]))
      return out

  # a few stored points for the self-retrieval probe (Task 6)
  sample = _post(f"/collections/{COLLECTION}/points/scroll",
                 {"limit": 8, "with_vector": True, "with_payload": True})["points"]
  print("collection reachable — sample points:", len(sample))
#+END_SRC
```

**Step 3: Run it**

Expected `#+RESULTS:`: `collection reachable — sample points: 8`. If it errors with a URL/connection
error, the tunnel is down — run `make tunnel-qdrant`.

**Step 4: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: notebook skeleton + remote-Qdrant setup"
```

---

## Task 2: Parameterized query embedder (CLS vs mean)

The single lever of the whole drill: one `embed_query` with a `pooling` switch. CLS is correct
(matches the index); `mean` is the injected fault. The query prefix stays CORRECT in both.

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the embedder block**

```org
** The query encoder =q(x)= — one recipe, one fault switch

bge-small needs **CLS pooling**; the index was built that way (=contract_pooling = "cls"=).
=pooling="mean"= is the drift we inject — the SAME text, a DIFFERENT vector.

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  import torch, torch.nn.functional as F
  from transformers import AutoTokenizer, AutoModel

  _M       = "BAAI/bge-small-en-v1.5"
  _QPREFIX = "Represent this sentence for searching relevant passages:"
  _tok     = AutoTokenizer.from_pretrained(_M)
  _model   = AutoModel.from_pretrained(_M).eval()

  @torch.no_grad()
  def embed_query(text, pooling="cls", prefix=_QPREFIX):
      enc = _tok([f"{prefix} {text}".strip()], padding=True, truncation=True,
                 max_length=512, return_tensors="pt")
      out = _model(**enc).last_hidden_state          # [1, T, 384]
      if pooling == "cls":
          vec = out[:, 0]                            # CLS token — bge's trained pooling
      elif pooling == "mean":
          m   = enc["attention_mask"].unsqueeze(-1).float()
          vec = (out * m).sum(1) / m.sum(1)          # masked mean — the WRONG pooling
      else:
          raise ValueError(pooling)
      return F.normalize(vec, p=2, dim=1)[0].numpy()

  # sanity: same text, two poolings => two different vectors
  a = embed_query("Youth Guarantee deadline", "cls")
  b = embed_query("Youth Guarantee deadline", "mean")
  print("dim", a.shape[0], " cos(cls, mean) =", round(float(a @ b), 3))
#+END_SRC
```

**Step 2: Run it**

Expected: `dim 384  cos(cls, mean) = 0.6xx` (clearly < 1 — the two recipes disagree even on the
same text; that gap is the whole bug). Record the real number in `#+RESULTS:`.

**Step 3: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: parameterized query embedder (CLS vs mean pooling)"
```

---

## Task 3: The golden set + recall@k / MRR helpers

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the golden-set + metrics block**

```org
** Golden set + retrieval metrics (plumbing, not quality)

~7 hand-authored policy questions, each with its gold source doc_id (edit freely). Metrics are
RANKING-based (recall@k, MRR) — never an absolute cosine cutoff (the index is an anisotropic
cone; see Drill 0).

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  GOLDEN = [
      ("What is the deadline for the Youth Guarantee quality offer?",      "EMPL-PR-785214"),
      ("How should public procurement rules support SMEs?",                "IMCO-PR-767975"),
      ("What challenges do the EU's eastern border regions face in cohesion policy?", "REGI-PR-789932"),
      ("Minimum number of Member States for an EDIP flagship defence project?",       "IMCO-AD-778358"),
      ("Future challenges for cross-border cooperation between regions?",  "REGI-PR-751733"),
      ("EMPL position on the Multiannual Financial Framework?",            "EMPL-AD-781335"),
      ("Findings on the Recovery and Resilience Facility's employment impact?", "EMPL-AD-768112"),
  ]

  def metrics(pooling, k=5):
      recall, rr = 0, 0.0
      rows = []
      for q, gold in GOLDEN:
          hits = search(embed_query(q, pooling), k)         # [(cid, doc_id, score)]
          doc_ranks = [i + 1 for i, (_, d, _) in enumerate(hits) if d == gold]
          got = bool(doc_ranks)
          recall += got
          rr     += (1.0 / doc_ranks[0]) if got else 0.0
          rows.append((gold, hits[0][1], "HIT" if got else "MISS"))
      n = len(GOLDEN)
      return recall / n, rr / n, rows
#+END_SRC
```

**Step 2: Run it** (defines functions only)

Expected: no output / no error.

**Step 3: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: golden set + recall@k/MRR helpers"
```

---

## Task 4: Baseline — correct recipe (CLS) retrieves the gold docs

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the baseline block**

```org
** Baseline — the CORRECT recipe (CLS) works

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  r, mrr, rows = metrics("cls", k=5)
  print(f"CLS (correct):  recall@5 = {r:.2f}   MRR = {mrr:.2f}")
  for gold, top, tag in rows:
      print(f"  gold {gold:<16} top1 {top:<16} {tag}")
#+END_SRC
```

**Step 2: Run it**

Expected: high recall (aim `recall@5 ≥ 0.85`) — most golden queries retrieve their gold doc.
**If any query MISSes under CLS, replace it** with a more distinctive question (the baseline must be
strong, or the collapse in Task 5 is not meaningful). Record real numbers in `#+RESULTS:`.

**Step 3: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: baseline recall@k/MRR under correct CLS recipe"
```

---

## Task 5: Inject the fault — mean pooling → recall collapses

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the collapse block**

```org
** Inject the drift — mean pooling → retrieval collapses

Same corpus, same index, same prefix — only the query POOLING changed. Every query vector still
has the right dim and unit norm (looks individually fine); only the cross-term =d·q= breaks.

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  rc, mc, _  = metrics("cls",  k=5)
  rb, mb, rows = metrics("mean", k=5)
  print(f"CLS  (correct):  recall@5 = {rc:.2f}   MRR = {mc:.2f}")
  print(f"mean (drift)  :  recall@5 = {rb:.2f}   MRR = {mb:.2f}   <- collapse")
  for gold, top, tag in rows:
      print(f"  gold {gold:<16} top1 {top:<16} {tag}")
#+END_SRC
```

**Step 2: Run it**

Expected: `mean` recall ≪ `CLS` recall (e.g. 1.00 → ~0.1–0.3). Record real numbers. Prose after the
block: this is the ticket reproduced — the gold chunk is NOT retrieved, so the fault is the
retrieval half, specifically `q(x)`.

**Step 3: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: mean-pool drift collapses recall (the reproduced ticket)"
```

---

## Task 6: Self-retrieval probe (label-free) catches the drift

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the probe block**

```org
** Confirm label-free — the self-retrieval probe

No labels, no prod logs: a passage re-embedded AS A QUERY must return its OWN point (cos ~ 1)
IFF the query and index recipes share a space. Because mean-pool changes the vector for the SAME
text, the probe CATCHES this drift (unlike a dropped prefix, which it is blind to).

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  import numpy as np
  for p in sample[:5]:
      cid  = p["payload"]["citation_id"]
      d    = np.array(p["vector"], dtype=np.float32)   # stored (CLS) doc vector
      txt  = p["payload"]["text"]
      for pooling in ("cls", "mean"):
          q      = embed_query(txt, pooling)
          selfcos = float(q @ d)
          top    = search(q, 1)[0][0]
          hit    = "self" if top == cid else f"-> {top}"
          print(f"{cid:<20} {pooling:<4} self-cos={selfcos:5.2f}  rank1 {hit}")
      print()
#+END_SRC
```

**Step 2: Run it**

Expected: `cls` → `self-cos ≈ 0.95+`, `rank1 self`; `mean` → `self-cos` drops (≈0.4–0.7) and often
`rank1` is NOT self. This is the toy's `self-retrieval [prod-bug] cos≈0.01 → retrieves Rome`, on
real vectors. Record real numbers.

**Step 3: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: self-retrieval probe catches the pooling drift"
```

---

## Task 7: Contract check — refuse the mismatched serve

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the contract block** (mirrors Rust `EmbeddingContract::assert_matches`):

```org
** Prevent — the embedding contract refuses a mismatched serve

The real fix is structural: every point stores the contract =(model, version, dim, normalized,
pooling, query_instruction)=. On serve, assert the LIVE query recipe equals the STORED contract;
mismatch → refuse loudly (never silently return wrong chunks). This is what =rag-server= does at
startup (=EmbeddingContract::assert_matches=).

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  stored = sample[0]["payload"]
  STORED = {k[len("contract_"):]: stored[k] for k in stored if k.startswith("contract_")}

  def assert_matches(live_pooling):
      live = {"model": "BAAI/bge-small-en-v1.5", "version": "v1", "dim": 384,
              "normalized": True, "pooling": live_pooling,
              "query_instruction": "Represent this sentence for searching relevant passages:"}
      bad = {k: (STORED.get(k), live[k]) for k in live if STORED.get(k) != live[k]}
      return (not bad), bad

  for pooling in ("cls", "mean"):
      ok, bad = assert_matches(pooling)
      print(f"live pooling={pooling:<4} -> {'OK (serve)' if ok else 'REFUSE  mismatch=' + str(bad)}")
#+END_SRC
```

**Step 2: Run it**

Expected: `cls → OK (serve)`; `mean → REFUSE  mismatch={'pooling': ('cls', 'mean')}`. Record real
output.

**Step 3: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: embedding-contract check refuses the mismatched serve"
```

---

## Task 8: Fix + restore + closing talking points

**Files:**
- Modify: `rag/drills/drill1_embedding_mismatch.org`

**Step 1: Write the restore block + closing prose**

```org
** Fix — mitigate → migrate → prevent; restore CLS → recall returns

#+BEGIN_SRC jupyter-python :session llms :kernel llms_kernel :async yes :exports both
  r, mrr, _ = metrics("cls", k=5)   # pin the query path back to the index-time recipe
  print(f"restored (CLS): recall@5 = {r:.2f}   MRR = {mrr:.2f}  <- recall returns")
#+END_SRC
```

Then the closing prose (adapt the toy's talking points, lines ~239–264):

```org
*Talking points — Drill 1 (retrieval-side "confidently wrong facts"):*

- *Bisect before touching anything.* On a known-gold failing query, are the gold chunks retrieved?
  No → retrieval half; yes → generation half. One probe halves the pipeline.
- *The retriever score is a product =d(z)·q(x)=.* Two encoders, one shared space; a bad score never
  says which factor broke. Drill 0 cleared the index =d(z)=, so the fault is =q(x)=.
- *Root cause: query/index embedding mismatch* — here the query used mean pooling; the index is CLS.
  Vicious because every vector looks individually fine (right dim, unit norm); only the cross-term
  =d·q= breaks. (Sibling faults: wrong model/version/normalization, or a dropped instruction prefix
  — the last is invisible to the self-retrieval probe, so the contract check is the real backstop.)
- *Confirm label-free with self-retrieval* — a passage re-embedded as a query stops returning itself
  under mean pooling (cos collapses). No labels, no prod logs needed.
- *Fix = mitigate → migrate → prevent.* Mitigate (minutes): pin the query path back to CLS. Migrate
  (planned): re-embed only if the INDEX recipe changed + atomic/blue-green swap. Prevent: store the
  embedding contract and refuse mismatched serves — exactly =rag-server='s startup assert.
- *Eval separates plumbing from quality.* Self-retrieval + recall@k/MRR prove the retrieval PLUMBING
  (same space, right chunks); faithfulness/groundedness prove GENERATION quality. Keep them distinct
  — the separation IS the bisection.
```

**Step 2: Run the restore block**

Expected: recall back to the baseline (≈ Task 4). Record real output.

**Step 3: Final full-notebook sanity run**

Re-run the whole notebook top-to-bottom (Emacs: `org-babel-execute-buffer`; or headless via
`llms_kernel`) so every `#+RESULTS:` reflects one consistent session against the live remote index.

**Step 4: Commit**

```bash
git add rag/drills/drill1_embedding_mismatch.org
git commit -m "drill1: fix/restore + closing talking points; full-notebook run"
```

**Step 5: Close the tunnel**

```bash
make tunnels-stop
```

---

## Definition of done

- [ ] `rag/drills/drill1_embedding_mismatch.org` exists, runs top-to-bottom against the REMOTE index
      (`:16333`), every block has a real `#+RESULTS:`.
- [ ] Baseline CLS recall@5 is strong (≥ 0.85); mean-pool recall collapses well below it.
- [ ] Self-retrieval probe: `self-cos ≈ 0.95+`/rank-1-self under CLS; drops and mis-ranks under mean.
- [ ] Contract check: `cls → OK`, `mean → REFUSE` with `pooling: ('cls','mean')`.
- [ ] Restore CLS → recall returns to baseline.
- [ ] Prose cross-references the toy (`interview_prep_aiinfra.org` Drill 1) throughout; reads as
      "the toy, on real vectors."
- [ ] No writes to the production index (search/scroll only).

## Explicitly out of scope (do NOT build here)

- The **dropped-prefix** fault and the "self-retrieval blind spot" two-mode version — deliberately a
  different, richer drill; not this one.
- Drill 2 (confidence-blind fusion) and Drill 3 (out-of-scope/abstain) — separate notebooks.
- Any change to `rag-server`, the index, or the contract types.
- LLM-judged faithfulness / generation-side metrics — this drill is retrieval-half only.

## Update the CLAUDE.md TODO after done

Check off Drill 1 in the RAG TODO block (it currently lists "Drill 1 — embedding mismatch" as NEXT),
noting the injected fault was **mean-vs-CLS pooling** (self-retrieval-catchable) and that the
dropped-prefix variant remains a future richer drill.
