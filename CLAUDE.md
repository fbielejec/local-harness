# CLAUDE.md — local_coding_harness

Orientation for agents. Read this, then `program.md` (the optimization loop) and
`docs/plans/2026-07-06-local-coding-harness-design.md` (the full design).

## ⚠️ THIS REPO IS PUBLIC — github.com/fbielejec/local-harness

**Nothing identifying the household goes in a commit.** This bit the project once
(2026-07-16: a design doc recorded the home public IPv4 + IPv6 prefix, and it reached public
`main` before anyone noticed). The repo is tied to a real name and email, so a home IP here is
a **deanonymisation** vector — name → ISP → geolocation — which matters more than any port,
given the MEP-office documents this project handles. Redacted in place; history rewritten.

**Never commit:** home public IPs (v4/v6, prefixes), **the DDNS hostname** (it *resolves* to
the home IP — publishing it undoes the redaction), MAC addresses, WireGuard/SSH **private**
keys, API tokens (deSEC, Exa), `.env` files, or router credentials. Use placeholders —
`<HOME_IPV4>`, `<HOME_V6_PREFIX>`, `<DDNS_HOST>` — and keep the real values on the boxes.

**Safe to commit:** LAN addresses (`192.168.1.22`, `10.10.0.x`) — RFC1918, meaningless
off-network. ISP names and RIPE allocation ranges — they identify a company across thousands
of customers, not a house.

**Before writing any measured network fact into a doc, run `git remote -v` first.**

## What this project is

A self-hosted agentic coding setup: an open-weights coding model served locally, driven by
the Qwen-Code CLI. Motivation is security/self-sovereignty (Vitalik's *Secure LLMs*) —
nothing leaves the local network.

```
laptop (this machine)                         remote: weebeastie 192.168.1.22
  Qwen-Code CLI  ──ssh tunnel──▶ localhost:8080 ──▶ llama-server (llama.cpp)
  local git repos                                     Qwen3-Coder-30B-A3B GGUF
```

## Hardware (remote `weebeastie`)

- CPU: Intel i9-10850K — 10 physical cores / 20 threads. **The real inference engine.**
- RAM: 62 GB DDR4 (~45 GB/s dual-channel) — generation is memory-bandwidth-bound here.
- GPU: GTX 1050 Ti, **4 GB VRAM**, Pascal (sm_61). Too small for experts; used only for
  attention + KV cache offload.
- OS: Linux Mint 22.1 (Ubuntu 24.04 base), kernel 6.8.

## Stack / where things live

| Thing                                 | Location                                                                                                                                           |
|---------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------|
| llama.cpp (built, CUDA 12.6, sm_61)   | remote `~/Programs/llama.cpp`, binaries in `build/bin/`                                                                                            |
| Model weights (GGUF, via `-hf` cache) | remote `~/models/` (`export LLAMA_CACHE=$HOME/models`)                                                                                             |
| Model                                 | `unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS` (~15.25 GiB, MoE 30B / ~3B active) — autoperf winner (2026-07-07); Q4_K_M was the prior default |
| Server log                            | remote `~/llama-server.log`                                                                                                                        |
| Qwen-Code scratch workspace           | laptop `~/qwen-scratch`                                                                                                                            |
| Chat frontend (Open WebUI, Docker)    | remote `~/openwebui/` (compose + gitignored `.env`); UI at `http://192.168.1.22:3000`; config in `deploy/openwebui/`                               |

## Operating the server

SSH: `ssh filip@192.168.1.22`

**Production server = systemd unit (config "IQ4_XS", 2026-07-08).** The autoperf winner
(CPU experts + GPU attention/KV, single warm slot; **+5% tg** vs the prior Q4_K_M "D4" at
identical coding quality (agent-pack 5/5), VRAM ~2777 MiB) now runs as a **system-level
service** on weebeastie — auto-starts at boot, `Restart=always` on crash. This is Phase E
(server half). Unit file is version-controlled at `deploy/llama-server.service` → installed
at `/etc/systemd/system/llama-server.service` (**edit one, re-sync the other**). It points at
the **local GGUF path** (not `-hf`) so boot has zero network dependency.

```bash
sudo systemctl restart llama-server        # restart (also: stop / start / status — all need sudo except status)
systemctl status llama-server              # state (no sudo)
journalctl -fu llama-server                # live logs (replaces `tail -f ~/llama-server.log`)
systemctl is-enabled llama-server          # 'enabled' = survives reboot
nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu --format=csv   # VRAM
```

⚠️ **Benchmarking gotcha:** the service has `Restart=always`, so a bare
`pkill -f build/bin/llama-server` (as the autoperf loop does) makes systemd **respawn it**
and re-grab port 8080. Before any manual/benchmark launch, first
`sudo systemctl stop llama-server`; re-enable production with `sudo systemctl start llama-server`.

**Manual launch (reference / one-off benchmarking only** — stop the service first, see above**):**

```bash
export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS \
  --host 127.0.0.1 --port 8080 \
  --threads 10 --parallel 1 --ctx-size 32768 \
  --n-gpu-layers 99 --cpu-moe \
  -fa on --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap --jinja \
  > ~/llama-server.log 2>&1 &
```

**Tunnel (laptop):** `ssh -fN -L 8080:127.0.0.1:8080 filip@192.168.1.22` (kill: `pkill -f "ssh -fN -L 8080"`)

**Run the harness (laptop):**
```bash
cd ~/qwen-scratch
export OPENAI_BASE_URL="http://localhost:8080/v1" OPENAI_API_KEY=dummy \
       OPENAI_MODEL="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS"
qwen -p "Use your tools to read notes.txt and tell me the secret word."   # expect: artichoke
```

## Operating the chat frontend (Open WebUI)

LAN web chat UI at **`http://192.168.1.22:3000`** (any device on the home LAN). Runs as a Docker
Compose service on weebeastie; `restart: unless-stopped` + docker `enabled` at boot ⇒ survives
reboots (no systemd unit needed). Config is version-controlled at `deploy/openwebui/`
(compose + `.env.example`); secrets in a gitignored `deploy/openwebui/.env`. Design + rationale:
`docs/plans/2026-07-08-lan-chat-frontend-openwebui-design.md`.

```bash
cd ~/openwebui                          # on weebeastie
docker compose ps | logs -f | restart | down
docker compose pull && docker compose up -d   # update after bumping the pinned image tag
```

- **Host networking is required** — llama-server is loopback-only, so the container reaches it
  via `localhost:8080` and exposes port 3000 to the LAN. The model stays private; only the UI
  is on the network.
- **Document RAG is off** (no embedder downloaded — `RAG_EMBEDDING_ENGINE=openai` + bypass).
  Enable later via the Admin Panel (Documents). Web search (Exa) is on, with per-query consent.
- **Per-user memory** is on; separate accounts ⇒ separate memories (agentic memory is
  model-dependent and may be flaky on Qwen; manual memory is reliable).
- ⚠️ **ConfigVar caveat:** the compose env only *seeds first boot*. After that the `open-webui`
  volume is authoritative — change settings in the Admin Panel (or wipe the volume to re-seed).

## Key findings so far (don't relearn these)

- **`--parallel 1` is essential.** With multiple slots each agent turn lands on a cold slot
  and re-prefills the entire ~16k Qwen-Code system prompt. One slot keeps the KV cache warm
  (`sim ~0.997`), so follow-up turns (and even new `qwen` sessions) prefill only new tokens.
- **GPU offload helps generation *at depth*.** At 16k context, gen was 3.1 t/s CPU-only vs
  **8.7 t/s** with attention/KV on GPU — the bottleneck at depth is attention-over-KV, which
  the GPU accelerates. (At ~0 context CPU looks faster — misleading; always measure at depth.)
- **Experts don't fit the 4 GB card** — `--cpu-moe` keeps expert FFNs in RAM. Naively
  offloading experts causes per-token PCIe ping-pong and tanks throughput.
- **First-turn prefill of the ~16k prompt ≈ 190 s** (CPU experts, ~86 t/s). This is a
  **one-time cost per server restart**; the warm cache persists across sessions.
- `--threads-batch 20` did **not** help prefill. `-no-cnv` was removed from `llama-cli` in
  this build (use `llama-completion` for one-shot).
- **Model quant is the ONLY throughput lever that moved tg** (autoperf 2026-07-07): IQ4_XS
  (4.25 bpw, 15.25 GiB) is +5% over Q4_K_M at identical coding quality (agent-pack 5/5).
  gen ∝ 1/expert-bytes, as expected for memory-bandwidth-bound expert reads. Q4_0 was +4.7%.
- **What did NOT help tg@16k** (all measured at `-d 16384`, all ties within noise): KV quant
  (q8_0→q4_0), partial expert offload (`--n-cpu-moe 44/46` — confirms the ping-pong), more
  threads (t20), and **speculative decoding** (Qwen3-0.6B draft, vocab-compatible, on CPU:
  8.76 vs 8.80 — MoE routing gives weak batch-amortization + only moderate general-draft
  acceptance). The bottleneck is the irreducible per-token CPU expert read over ~45 GB/s RAM.

## Current goal

**Improve tokens/s** without degrading measured coding quality (see `program.md` + `evals/`).
Primary metric: generation t/s at 16k context (`llama-bench -d 16384`).
**Best: IQ4_XS** — tg@16k **8.80** (+5% vs Q4_K_M's 8.38), pp 54.95, VRAM 2777 MiB; quality
preserved (reasoning 1.50/5 ≥ 1.00 baseline; agent-pack **5/5** = baseline). Full results in
`results.tsv` + `docs/autoperf-reports/2026-07-06-autoperf-report.md`.

## TODOs

Running list of follow-ups (check off as done; newest at the bottom).

- [~] **Remote access to the local harness model** — reach `:8080/v1` from a travel laptop.
  **WORKING as of 2026-07-16** (verified from outside); two items pending, see *Left* below.
  Design: `docs/plans/2026-07-16-remote-model-access-design.md`; step-by-step:
  `docs/plans/2026-07-16-remote-model-access-implementation-plan.md`. Runbook: `README.md`
  §3 + §Remote access.
  - **Use it:** `make away` (off-LAN: wg up + all tunnels over it) · `make tunnels` (at home) ·
    `make wg-status` · `make away-stop`. `make away` **only works from OUTSIDE the house** —
    the endpoint is the home public IP and the b-box **doesn't hairpin**, so it fails fast (3 s)
    telling you to use `make tunnels`.
  - **Decided:** self-hosted **WireGuard** on weebeastie (one router rule: `UDP 51820 →
    192.168.1.22`; **`:22` deliberately NOT forwarded**) + **SSH over the tunnel**.
    `llama-server` binding is **UNCHANGED** — stays `127.0.0.1:8080`, never bound to any
    interface; the tunnel command is byte-identical to today's, only the host changes
    (`filip@10.10.0.1`).
  - **[x] Done (2026-07-16):** DHCP reservation · wireguard both ends (weebeastie
    `wg-quick@wg0` **enabled at boot**, laptop **deliberately not** — see below) · b-box
    UDP-51820 forward · deSEC DDNS + 5-min updater timer on weebeastie (A only, **AAAA deleted
    on purpose**, TTL 60) · `~/.ssh/config` with `weebeastie` (LAN) + `weebeastie-remote`
    (tunnel) aliases · root `Makefile` targets. **Verified from outside** via a NordVPN exit:
    handshake + `/v1/models` over both the raw IP and the DDNS hostname.
  - **[x] sshd hardened (2026-07-16):** `PasswordAuthentication no` · `PermitRootLogin no` ·
    `KbdInteractiveAuthentication no` (`/etc/ssh/sshd_config.d/99-hardening.conf`). The user key
    is **RSA-4096** (`~/.ssh/id_rsa`, sigs via `rsa-sha2-512`) — *not* ed25519; that's the
    **host** key. To verify hardening, probe behaviour rather than reading the file:
    `ssh -o PreferredAuthentications=password -o PubkeyAuthentication=no filip@192.168.1.22`
    → `Permission denied (publickey)`; that parenthesised list is the only method accepted.
  - **[ ] Left:** Stage-2 verification from a **phone hotspot** (carrier CGNAT = a harder, more
    realistic test than a clean Nord datacenter exit) · prove DDNS **recovery** by forcing a new
    lease (today only proves the updater *runs*, not that it *heals*) · **GitHub support request**
    to GC the pre-redaction commits, still fetchable by SHA.
  - **Don't relearn:** public IPv4 `<HOME_IPV4>` is **not CGNAT** ⇒ port forwarding works.
    But RIPE marks it `Proximus … xDSL customers (dynamic)` — the IP is **sticky, not static**
    ⇒ DDNS (deSEC.io) is required, else you get **locked out** when the line resyncs while
    you're away (can't learn the new IP without access to the house). `wg-quick` resolves
    `Endpoint` **once at start and never re-resolves** — but the fix is just to bounce the
    tunnel (`make away-stop && make away`), which re-resolves. A `reresolve-dns` timer was
    considered and **rejected as YAGNI**: an IP change kills the SSH session anyway, so
    auto-healing the WireGuard layer rescues nothing, and the script's 135 s staleness
    threshold + 2 min timer make it *slower* (~4 min) than the manual bounce (~10 s). It would
    only earn its place for something unattended that must stay connected. Native IPv6 exists
    (`<HOME_V6_PREFIX>::/64`) but is a
    **fallback only**: café wifi is often IPv4-only, and both v6 addrs are dynamic.
  - **Rejected:** NordVPN Meshnet (closed-source **root daemon** on the box holding the models
    + RAG index; iptables conflict with Docker/Open WebUI; its only real win — NAT traversal —
    is unneeded given a public IP). Proton/Nord *as VPNs* are a category error: they solve
    egress privacy, this needs ingress. claude-code Remote Control is unusable — it hard-requires
    `api.anthropic.com` (disables itself if `ANTHROPIC_BASE_URL` differs) and persists the full
    transcript server-side. Its one good idea is the **dial-out topology**, only worth it if the
    scope ever widens to driving an agent at home from a phone.
  - **Gotchas paid for the hard way (don't relearn):**
    - **Never test the forward from the LAN** — the b-box has no NAT loopback, so an inside test
      against the public IP is a **false negative**. Use NordVPN (as a *test client* on the
      laptop — that's not a contradiction of rejecting it as *transport* on weebeastie) or a
      hotspot. A handshake from outside is also the carrier-NAT falsification test.
    - **Don't `systemctl enable wg-quick@wg0` on the laptop.** An auto-started tunnel is
      permanently broken *at home*, which is where the laptop mostly is: live `wg0`, dead
      `10.10.0.1`, `ssh` hanging for minutes on the one network where all is well. Hence the
      explicit `make away` + its 3-s reachability check. (The **server-side** enable is correct.)
    - **`pkill -f "ssh -fN …"` matches its own argv** and kills the shell running it. The root
      `Makefile` already uses the `[s]sh` regex trick — copy it.
    - Debian ships `reresolve-dns` under `/usr/share/doc/wireguard-tools/examples/`, not the
      upstream `/usr/share/wireguard-tools/` path.
    - **No `ip_forward`, no MASQUERADE.** Most WireGuard guides add them; they're for gateway
      use. `AllowedIPs = 10.10.0.1/32` terminates at the interface — blast radius stays one host.
- [~] **RAG(s)** — EP-committee RAG, maximal-Rust. Full design + status:
  `docs/plans/2026-07-10-ep-committee-rag-design.md`. Code: `rag/` (cargo workspace, `rag/Makefile`).
  - **Goal:** grounded, cited Q&A over EMPL/REGI/IMCO EP committee PDFs (for spouse's MEP
    office) + a diagnostic rig for the RAG-debugging interview (Beat 3 / two drills).
  - **Stack:** Qdrant v1.18 (`deploy/qdrant/`, telemetry off) · BGE-small embeddings ·
    generator = the existing `llama-server` (OpenAI-compatible `:8080/v1`). Orchestrator-mediated
    RAG; Open WebUI later as a chat UI *in front of* the Rust pipeline, not as the retriever
    (integration design: `docs/plans/2026-07-11-openwebui-rag-integration-design.md`).
  - **[x] Ingestion (all Rust):** `core`·`fetch`·`parse`·`chunk`·`ingest` → `chunks.jsonl`.
    12 PDFs → 490 chunks, all ≤512 tok, 0 boilerplate leak. Both acceptance gates pass
    (pdf-extract≈pymupdf; candle==sentence-transformers **cosine 1.0** → one vector space).
  - **[x] `index` (2026-07-11):** candle passage-embed → Qdrant upsert with the **contract-stamped
    payload**. Collection `ep_committee_docs`, 490 points, single dense vector (384, cosine),
    deterministic UUIDv5 ids (idempotent re-index). Pure core unit-tested (`point_id`, `chunk_payload`).
  - **[x] `retrieve`+`generate` — hand-stitched (2026-07-11):** `rag/notebooks/rag_query.org`
    (org-babel, `llms_kernel`). Full loop works: embed_query(prefix) → Qdrant top-k → grounded
    prompt (cite `citation_id`, "I don't know") → Qwen → cited answer (~15 s warm). This notebook
    is the **executable spec** for the Rust `retrieve`/`generate` crates.
  - **[x] Drill 0 — index health (2026-07-11):** `rag/drills/drill0_index_health.org`. Integrity +
    separation/anisotropy + near-dups + self-retrieval probe. Index healthy.
  - **[x] `retrieve`/`generate` Rust crates (2026-07-11):** productized the notebook into
    workspace crates — `rag-retrieve` (bge query-prefix embed → Qdrant gRPC top-k) and
    `rag-generate` (grounded `assemble`, `<think>` strip, serve-time provenance/Sources join,
    UTF-8-safe streaming + non-streaming llama-server client). These are the substrate
    `rag-mcp` is built on.
  - **[x] `ep-rag-server` removed (2026-09-03):** an axum OpenAI-compatible RAG face on `:8081`
    (`/v1/models` + `/v1/chat/completions`, startup live-vs-stored contract assert, warm-on-boot,
    30 tests green). Built, reviewed, and **never deployed** — its one job was to be Open WebUI's
    second model, and the `/route` filter against `rag-mcp` shipped that instead. Keeping a
    second, unexercised front door to the same index was the worse trade, so the crate and its
    unit file are deleted. Recoverable from git; the build plan stays in `docs/plans/` as record.
  - **Don't relearn (key decisions + findings):**
    - **Naming: `rag-*` is generic, `ep-*` is corpus-specific** (renamed 2026-09-03, ahead of the
      papers and org-docs corpora below). The pipeline crates — `rag-core`, `chunk`, `parse`,
      `embed`, `retrieve`, `generate`, `ingest`, `index` — and the `rag-mcp` service know nothing
      about the European Parliament, so they no longer say so. What *is* EP-specific keeps the
      name and should: `ep-rag-fetch` (an ODP API client), the `ep_committee_docs` collection,
      the `search_ep_committee_docs` MCP tool, and the `R_USE_EP_RAG` route. A second corpus adds
      its own collection, tool and route beside them rather than renaming these. The routing tree
      is already opaque JSON loaded from `TREE_PATH`, so the router itself needs no change.
      Not yet parameterized: `COLLECTION` is a `const` in `index.rs` — that is the work the
      second corpus will force, and it is deliberately deferred until there is one.
    - Embed = **candle** (pure Rust), NOT fastembed — its ONNX prebuilt needs glibc ≥2.38, this
      box is 2.35 (`__isoc23_*` link error). candle links cleanly, no C++.
    - bge-small uses **CLS pooling** (not mean); query gets the instruction prefix, passages don't.
    - Parse = pure-Rust `pdf-extract` (no pdfium native dep).
    - **`make` runs use `--release`** — candle in debug is 10–50× slower (the 490-chunk index went
      44 min debug → ~1–2 min release).
    - **Self-retrieval probe detects *encoder* drift (pooling/model), NOT the dropped prefix** —
      with query=passage-text, dropping the prefix makes `q`≈`d` (self-query *more* perfect). The
      prefix bug shows only in **recall@k on real (different-text) queries**.
    - **Index is anisotropic** (mean pairwise cosine 0.68, single-domain; participation ratio
      ~45/384) → retrieval works but **don't threshold raw cosine** (relevant hits sit 0.68–0.73).
    - **Faithfulness has two axes:** content-groundedness vs citation-accuracy — observed a *correct
      but mis-attributed* citation (grounded in `:1`, model cited `:12`). A lexical check must
      **normalize whitespace** (pdf-extract emits `"four  months"`).
    - **Drill-2 substrate located:** `EMPL-PR-612058:1 ~ IMCO-PR-773060:1` at **cosine 1.000** —
      byte-identical draft-report preamble across two committees.
  - **[~] Deploy — reshaped (2026-09-03):** the index snapshot-migration to weebeastie still
    stands, but the rest of Phase 4 is void: there is no `rag-server` to run as a unit and no
    second Open WebUI model to register. What runs there is `rag-mcp` (`:8082`). Still open
    from that plan: the eval harness (recall@k/MRR **vs** groundedness+citation-accuracy).
  - **[x] Conditional/agentic retrieval — delivered by `rag-mcp`:** the problem was
    always-on RAG. RAG-*as-a-model* ran the retrieve loop **unconditionally** on every query with
    no relevance gate (by design: the index is anisotropic, so we "don't threshold raw cosine"),
    so off-topic questions still paid embed+search+~2k-token prefill before the grounding prompt
    made the model answer "I don't know" — mitigated only by the model picker. `rag-mcp` is
    the router that closes this: `/route` tree-classifies each turn before retrieving, and the
    same service exposes retrieval as an MCP tool an agent calls only when it needs it — the
    design's "tool-calling later" evolution, arrived at.
  - **Run:** `cd rag && make pipeline` (qdrant-up → fetch → ingest → index) · `make parity` ·
    `make serve-mcp` (rag-mcp: MCP tool + `/retrieve` + `/route` on `:8082`) · drills/notebook in Emacs
    (`llms_kernel`) or `uv run --project drills python drills/…`.
  - [ ] org documents (my knowledge db) — later, same pipeline.
  - [ ] LLM papers db
- [ ] Base model change. Hardware is too weak for coding specific model, a generalist geared
  towards working with text, translations, information retrieval (web search and RAG).
  - [ ] Evaluate qwen-family "ThinkingCap models" tweaked towards lower token consumption per task:
  https://paperswithcode.co/paper/102599
- [~] **Basic security via `~/.qwen/settings.json`** — harden the Qwen-Code client for the
  self-sovereignty goal. Schema note (verified against installed **qwen 0.19.8**): the file
  **exists** now and uses the `"$version": 4` nested schema — a `permissions` allow/deny model
  (not the old `coreTools`/`excludeTools`), plus `privacy.*` and `telemetry.*` blocks.
    - [x] **Telemetry locked out (2026-07-08).** Set `privacy.usageStatisticsEnabled: false`
      (the anonymized usage phone-home — tool-call names, API-request metadata, session config)
      and `telemetry.enabled: false` (OTLP exporter). Local usage recording
      (`~/.qwen/usage_record.jsonl`) stays — it never leaves the LAN. Backup:
      `~/.qwen/settings.json.bak-20260708`. Env overrides exist too (`QWEN_TELEMETRY_ENABLED`).
    - [ ] **Remaining (real hardening, deferred):** tool-execution **sandbox** on
      (docker/podman/`sandbox-exec`); **no blanket auto-approve** of shell commands (require
      approval or confine to `~/qwen-scratch`); tool allow/deny list (via the `permissions`
      block) + MCP server allowlist. `llama-server` stays loopback-only.
    - Context for the sandbox item: the coding-quality gate (`evals/agent_pack_runner.sh`) runs
      `qwen --yolo` — edits/shell auto-execute *unsandboxed* at the user's privilege (qwen even
      warns about it). Wrap those eval runs in a sandbox (`qwen --sandbox` / `QWEN_SANDBOX`,
      docker/podman) so the agent can't touch the host. (Root cause found 2026-07-06: headless
      `qwen -p` silently blocks every write/edit tool without `--yolo`, because there's no TTY
      to approve — so `--yolo` is mandatory for the eval, which is exactly why the sandbox
      matters.)
- [ ] **Chat frontend (Open WebUI) reachable from anywhere, under a stable name.**
  **NO DECISION YET — needs its own planning session.** Options below are scoped + costed;
  pick one there, then write the design doc. Today the UI is `http://192.168.1.22:3000` —
  LAN-only, IP-typed, no name, no TLS.
  - **Requirement (firm, 2026-07-16):** a **non-technical user (spouse)** opens a URL from
    **anywhere** (phone, café, hotel) and logs in. **No VPN client, no config, no install.**
    Preserve the self-sovereign posture *as far as that requirement allows*. Latency matters
    subjectively (see "latency is mostly a non-issue" below).
  - **⚠️ Public-repo rule:** a hostname that resolves to the **home** IP is a deanonymisation
    vector and **must never be committed** — placeholder `<CHAT_HOST>`, real value on the box.
    Same reason `<DDNS_HOST>` is redacted. Note the *domain* `nodrama.io` is safe to name here
    (it's already tied to the same identity and points at CloudFront, not the house) — what
    must never land in git is a **record pointing home**, in *either* repo. The sibling repo
    `github.com/fbielejec/nodrama.io` (AWS control plane: CloudFormation, Route53 zone,
    profile `nodrama`, region **us-east-1**) is **also public** — a redirect/CNAME target
    baked into a CFN template there leaks exactly like a doc would.
  - **Option A — WireGuard + static name → `10.10.0.1`.** Not DDNS at all; the name is fixed.
    - *Pros:* zero new infra/exposure/cost; reuses `wg0`; nothing public; strongest posture.
    - *Cons:* **fails the firm requirement** — needs a VPN client + config on her phone.
      Also the two-networks problem (below). Keep documented as the cheapest option and the
      right answer if the requirement ever softens to "me, from my laptop".
  - **Option B — forward `:443` on the b-box → caddy on weebeastie → Open WebUI.**
    - *Pros:* no third party at all; lowest latency (direct); one more router rule; cert via
      ACME. Meets the requirement.
    - *Cons:* **publishes the home IP** in public DNS — the deanonymisation vector the whole
      redaction policy exists to prevent, and *worse* under `nodrama.io` (WHOIS + MEP site tie
      the house to a real name). Needs DDNS (sticky-not-static IP). Public login page.
  - **Option C — S3 redirect (`chat.<domain>` → 301 → target).** *Investigated 2026-07-16;*
    *documented dead end — don't re-derive.*
    - **A signpost, not a road.** S3 answers 301 and nothing else; the browser then connects
      **directly** to the target, so S3 carries no traffic, hides no IP (target lands in the
      address bar and gets bookmarked), and can't proxy. The existing apex→`blog.nodrama.io`
      redirect (`cloudformation/nodrama-io.yml`) is the *correct* use of this pattern.
    - Can't carry it even in principle: that distribution is `AllowedMethods: GET, HEAD` —
      Open WebUI needs POST (login/chat) + WebSockets (streaming).
    - Steelman ("pretty name over a dynamic IP, no EC2") collapses: a plain
      `CNAME chat.<domain> → <DDNS_HOST>` does it better — no redirect hop, no method limit,
      pretty URL stays. So: **willing to publish the home IP ⇒ CNAME beats the redirect;
      unwilling ⇒ neither works.** No configuration makes S3 the right tool here.
  - **Option D — CloudFront with a custom origin (real proxy, not a redirect).**
    - *Pros:* genuinely proxies; WebSockets supported; ACM certs; stable name; reuses Route53.
    - *Cons:* **terminates TLS ⇒ Amazon sees all plaintext** (the posture violation the
      passthrough relay exists to avoid); origin must still be publicly reachable ⇒ **home IP
      still exposed** (mitigable via CF prefix-list firewall + secret header, but fiddly and
      **fails open** if misconfigured). Loses to Option E on both axes; CF traffic likely
      costs more than the nano.
  - **Option E — minimal EC2 relay, WireGuard dial-out, TLS passthrough.** *(leading candidate
    on the merits so far — NOT a decision)*
    - Shape: `chat.<domain>` → Elastic IP of a **`t4g.nano` in eu-west-3 (Paris)** ·
      weebeastie **dials out** over WG (`PersistentKeepalive`) · EC2 **DNATs `:443`** into the
      tunnel, **TLS passthrough (no keys, no plaintext on AWS)** · home caddy terminates with
      a real cert via **Route53 DNS-01** (zone + creds already exist; use a **scoped IAM user**
      limited to TXT on that zone) → Open WebUI.
    - *Pros:* **static A record — no DDNS, no updater, no deSEC** (dissolves the original DNS
      problem); **home IP never in public DNS** ← *the actual win here, not NAT traversal*;
      no new inbound rule at home; same URL home + away; survives IP churn; AWS sees only
      ciphertext + metadata (no keys, no chat history, no RAG index).
    - *Cons:* ~**$5–8/mo** (nano + IPv4 charge — *reconfirm current pricing*); a box to patch;
      home chat then depends on the line being up (today the LAN UI works regardless);
      trombone routing; public login page (see below).
    - ⚠️ **Do NOT reuse the existing us-east-1 instance.** Two independent reasons: (1) wrong
      region — Brussels↔us-east-1 ≈ 90 ms each way ⇒ ~360 ms RTT tromboned; (2) **blast
      radius** — that box is the **public WordPress MEP site**, a WP
      compromise would land an attacker inside a tunnel next to the model + Qdrant index.
      The relay must be **separate, minimal, single-purpose**.
  - **Option F — managed tunnel (Tailscale Funnel / cloudflared).** Not yet analysed.
    - *Pros:* least to build/patch; solves NAT, TLS, and naming in one.
    - *Cons:* third-party control plane — the same category as the **already-rejected**
      NordVPN Meshnet; cloudflared **terminates TLS** (plaintext at Cloudflare = Option D's
      flaw); custom-domain support + whether Funnel terminates or passes through **needs
      verification, don't assume**.
  - **Cross-cutting (established 2026-07-16, don't re-derive):**
    - **Latency is mostly a non-issue.** Model gen is **8.8 t/s ≈ 114 ms/token**; a 300-token
      answer ≈ 34 s. Tokens stream over one TCP connection, so added RTT is a **constant
      offset on TTFT, not a per-token tax** — even a transatlantic hop costs ~1% of response
      time. Where it *is* felt is **UI interactivity** (login, page loads) at ~360 ms/RTT =
      "slow site" to a non-technical user. An **EU relay (~25–30 ms trombone) makes it
      imperceptible**; Route53 is global so region choice costs nothing elsewhere.
    - **The relay's win is IP-hiding, NOT NAT traversal.** NAT traversal is genuinely unneeded
      (public non-CGNAT IP + working forward) — that's what killed Meshnet. But *that reasoning
      does not carry over*: no direct-forward design can hide the house, and a relay can.
    - **A public login page is irreducible** for "non-technical, no VPN client, anywhere" — it
      becomes the whole security boundary. No option avoids it (except A, which fails the
      requirement). Mitigations to spec at planning time: signup disabled, strong unique
      password, rate-limit/fail2ban at the home caddy, pinned image kept current.
    - **Two-networks problem** (applies to A/B): home → `192.168.1.22`, away → `10.10.0.1`;
      one public A record can't be both (**the b-box doesn't hairpin** — same trap as
      `make away`). Options: two names, local DNS override, or WG at home too (rejected — see
      the "don't enable `wg-quick` on the laptop" gotcha). **E and D make this moot.**
  - Context: `deploy/openwebui/` · `docs/plans/2026-07-08-lan-chat-frontend-openwebui-design.md`
    · remote-access work above (`wg0`, deSEC updater) · `~/CloudStation/DevOps/nodrama/`
    (AWS footprint; its `docs/BACKLOG-INFRA.md` should get a pointer once this is decided).

## Docs

- `README.md` — chronological runbook (every command run, with results).
- `docs/plans/2026-07-06-local-coding-harness-design.md` — full design + rationale.
- `docs/plans/2026-07-06-quality-gated-autoperf-design.md` — the quality-gated tuning loop.
- `docs/autoperf-reports/2026-07-06-autoperf-report.md` — throughput + quality tuning results.
- `evals/` — coding-quality gate (reasoning + agent-pack; methodology adapted from Raschka).
- `program.md` — the autonomous, quality-gated tok/s optimization loop.
