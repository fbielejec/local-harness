# Local Agentic Coding Harness

A self-hosted agentic coding setup: an open-weights coding model served locally by
**llama.cpp**, driven by the **Qwen-Code** CLI over an SSH tunnel. Motivation is
security / self-sovereignty — nothing leaves the local network.

**Current best on this box:** `Qwen3-Coder-30B-A3B-Instruct` **IQ4_XS** → `tg@16k` ≈ 8.8 t/s,
VRAM ≈ 2.8 GB on a **GTX 1050 Ti (4 GB) + i9-10850K**. How that was tuned:
`docs/autoperf-reports/`; design + rationale: `docs/plans/`.

```
client machine (laptop)                    server machine (e.g. weebeastie, 192.168.1.22)
  Qwen-Code CLI ──ssh tunnel──▶ localhost:8080 ──▶ llama-server (llama.cpp, CUDA)
  your local git repos                              Qwen3-Coder-30B-A3B GGUF (IQ4_XS)
```

| machine    | role                                                                  |
|------------|-----------------------------------------------------------------------|
| **server** | the GPU box — builds & runs `llama-server`, bound to `127.0.0.1` only |
| **client** | your dev laptop — SSH tunnel + Qwen-Code + your git repos             |

The server never listens on the LAN; it is reached exclusively through the SSH tunnel.

---

# Reproduce this setup

Commands are tagged `[server]` / `[client]`. The example host is `filip@192.168.1.22` —
substitute your own throughout.

## 0. Prerequisites

**Server** (the GPU box):
- Linux with an NVIDIA GPU + working driver (`nvidia-smi` runs).
- Build tools: `git`, `cmake` (≥3.18), a C/C++ compiler, `make`, `libcurl` dev headers.
- **CUDA toolkit** (`nvcc`) ≥ 12.4 — install in step 1 if missing.
- RAM ≥ ~20 GB (the MoE experts live in RAM) and ~16 GB free disk for the weights.

**Client** (your laptop): SSH access to the server + **Node ≥ 20** with npm, and **`jq`**
(`make install-client` merges into `~/.qwen/settings.json` rather than overwriting it).

> Sizing intuition: generation here is **memory-bandwidth-bound on the CPU-resident experts**;
> the small GPU only accelerates attention + the KV cache. See *Adapting to different hardware*.

## 1. [server] Build llama.cpp with CUDA

```bash
make build-llama        # skipped when llama-server already builds and runs
```

Auto-detects the GPU's compute capability from `nvidia-smi` and passes it as
`CMAKE_CUDA_ARCHITECTURES` (6.1 Pascal → `61`, 7.5 Turing, 8.6 Ampere, 8.9 Ada, 9.0 Hopper).
That value is the single most likely thing to get wrong on a new box, and it fails *after* a
20-minute compile — override with `CUDA_ARCH=` only if the detection is wrong. `FORCE=1`
rebuilds an existing tree; `LLAMA_DIR=` moves it off `~/Programs/llama.cpp`.

The build itself is `-DGGML_CUDA=ON -DGGML_CUDA_FA_ALL_QUANTS=ON -DLLAMA_CURL=ON` —
flash-attention for quantized KV, and `-hf` model pulls. ~15–25 min.

**The CUDA toolkit is a precondition, not a step.** `make build-llama` fails pointing here if
`nvcc` is missing rather than installing 3 GB unasked. This box is Ubuntu 24.04 → CUDA 12.6
(adjust for your OS; ≥ 12.4 is needed for gcc 13):

```bash
cd /tmp
wget https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2404/x86_64/cuda-keyring_1.1-1_all.deb
sudo dpkg -i cuda-keyring_1.1-1_all.deb
sudo apt update && sudo apt install -y cuda-toolkit-12-6
nvcc --version      # expect release >= 12.4
```

If `nvcc` is not on `PATH` afterwards, put it there durably — a `~/.bashrc.d/cuda.sh` snippet
if you use `setup-desktop`, whose `setup-bash.sh` overwrites `~/.bashrc` wholesale and would
eat lines appended to it:

```sh
export PATH=/usr/local/cuda-12.6/bin:$PATH
export LD_LIBRARY_PATH=/usr/local/cuda-12.6/lib64:$LD_LIBRARY_PATH
```

## 2. [server] Launch the model server (winning config)

```bash
make fetch-model        # ~16 GiB into $LLAMA_CACHE; skipped when already there
```

Making the pull its own step is the point: it used to happen implicitly on the first
`llama-server` start via `-hf`, so a first boot silently took as long as a download and looked
like a hang. **IQ4_XS** is the quality-gated throughput winner here (Q4_K_M also works, ~5 %
slower); `MODEL=` overrides, `LLAMA_CACHE=` moves the cache off `~/models`.

In production the server runs under systemd (§2a). Launch it by hand only for benchmarking —
stop the unit first, or the two contend for `:8080`:

```bash
sudo systemctl stop llama-server
export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS \
  --host 127.0.0.1 --port 8080 \
  --threads 10 --parallel 1 --ctx-size 32768 \
  --n-gpu-layers 99 --cpu-moe \
  -fa on --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap --jinja \
  > ~/llama-server.log 2>&1 &

tail -f ~/llama-server.log      # wait for "listening on http://127.0.0.1:8080" (~17 s)
nvidia-smi --query-gpu=memory.used,memory.total --format=csv   # expect < 3900 MiB
```

What the flags do (why → *Why these choices*):

| flag | purpose |
|---|---|
| `--parallel 1` | **essential** — one warm KV slot; else every agent turn re-prefills the ~16k prompt |
| `-ngl 99 --cpu-moe` | attention/KV/router on the GPU; expert FFNs stay in RAM (won't fit 4 GB) |
| `-fa on` + `-ctk/-ctv q8_0` | flash-attn + quantized KV so 32k context fits in ~2.8 GB VRAM |
| `--ctx-size 32768` | must stay **> 16k** for the Qwen-Code system prompt |
| `--no-mmap` | load weights into RAM up front (avoids paging stalls) |
| `--jinja` | chat template required for Qwen-Code tool-calling |
| `--threads 10` | = physical cores (generation is memory-bandwidth-bound) |

Verify on the server, then restart with `pkill -9 -x llama-server; sleep 2; <relaunch>`:

```bash
curl -s http://127.0.0.1:8080/v1/models
curl -s http://127.0.0.1:8080/v1/chat/completions -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Reply with the single word: pong"}],"max_tokens":10}'
```

## 2a. [server] Run it as a systemd service (survives reboots)

In production both services run **system-level** so they auto-start on boot and restart on
crash: `llama-server` and `rag-mcp` (§*EP-Committee RAG*).

```bash
make install-server     # build-llama + fetch-model + build-rag + deploy-units
make restart-server     # the only step that takes downtime — never implicit
```

`deploy/*.service` are **templates**, rendered with this machine's user, paths and the resolved
GGUF snapshot path — they cannot be copied into place as-is. `deploy-units` renders all of them,
runs `systemd-analyze verify` on the results, and only then writes to `/etc/systemd/system`,
backing up any differing live file as `*.service.bak-<ts>`. Every tier is separately runnable
(`make build-rag`, `make deploy-units`) and skips when its work is already done.

**A deploy is inert.** Writing a unit file changes nothing until a restart, so `install-server`
installs, `daemon-reload`s and *reports* what changed — it never bounces a live service. Taking
the downtime is `make restart-server`, which drops the warm KV cache (the next agent turn
re-prefills the ~16k prompt, ~190 s). Preview a run that touches nothing with `DRY_RUN=1`:

```bash
DRY_RUN=1 make deploy-units
```

`install-tools` separately puts the pipeline binaries (`ingest`, `index`, `parse-gate`,
`embed-gate`, `fetch`) on `PATH`. They are not services and `install-server` does not build them.

> **Nothing from `~/.bashrc.d/` is on the non-interactive `PATH`.** `ssh box 'make install-server'`
> runs a non-interactive shell, `~/.bashrc` returns early at its `case $- in *i*` guard, and neither
> `~/.cargo/bin` nor nvm is loaded — so `cargo` and `node` both look uninstalled when they are not.
> The install scripts resolve cargo explicitly (`$CARGO` → `PATH` → `~/.cargo/bin/cargo`) and source
> nvm before using node; anything else you run over `ssh` needs `bash -lc`.

**Check if it's running** (no sudo):

**Check if it's running** (no sudo):

```bash
systemctl is-active  llama-server     # -> active
systemctl is-enabled llama-server     # -> enabled   (will start on boot)
systemctl status     llama-server     # full state + last log lines
curl -s http://127.0.0.1:8080/health  # -> {"status":"ok"}   (once weights finish loading)
```

**View logs** — journald replaces the old `~/llama-server.log` (no sudo):

```bash
journalctl -fu llama-server                 # follow live (Ctrl-C to stop) — the everyday one
journalctl -u llama-server -n 100            # last 100 lines
journalctl -u llama-server -b                # everything since the last boot
journalctl -u llama-server --since "10 min ago"
journalctl -u llama-server -p err            # errors only
journalctl -u llama-server -o cat            # raw lines, no systemd timestamp prefix
```

**Stop it** (needs sudo):

```bash
sudo systemctl stop llama-server           # stop now; stays down until started or next boot
sudo systemctl disable --now llama-server  # stop AND don't auto-start at boot
```
⚠️ `pkill llama-server` will **not** stop it — `Restart=always` makes systemd respawn it.
Always `sudo systemctl stop llama-server` first before a manual/benchmark launch (§2).

**Restart / reboot it** (needs sudo):

```bash
sudo systemctl restart llama-server   # restart the service (drops warm KV cache; first
                                      #   request after re-prefills the ~16k prompt, ~190 s)
sudo systemctl start   llama-server   # bring it back after a stop
sudo reboot                           # full host reboot — service auto-starts (it's enabled);
                                      #   re-run the "Check if it's running" block once back up
```

## 3. [client] Reach the server — on the LAN, or from anywhere

`llama-server` is **loopback-only** on weebeastie, so every client path is an SSH forward
mapping the laptop's `localhost:8080` to the server's `127.0.0.1:8080`. Two ways in, one
command each. Everything downstream (§4's env vars, §5's eval gate) is identical either way —
from the client's point of view `localhost:8080` is `localhost:8080`.

**On the home LAN:**
```bash
make tunnels                                             # llama :8080 · Qdrant :16333/:16334 · mcp :8082
curl -s http://localhost:8080/v1/models | head -c 200     # round-trip check
make tunnels-stop
```
Just the model: `make tunnel-llama`. Raw equivalent:
`ssh -fN -L 8080:127.0.0.1:8080 filip@192.168.1.22` (stop: `pkill -f "ssh -fN -L 8080"`).

**From an external network** (café, hotspot, anywhere):
```bash
make away                                                # wg up (+ reachability check) → same tunnels over WireGuard
curl -s http://localhost:8080/v1/models | head -c 200     # identical check, identical result
make wg-status                                           # endpoint / handshake / transfer
make away-stop                                           # tunnels down + WireGuard down
```
`make away` re-points `REMOTE` at the WireGuard peer (`10.10.0.1`), so Qdrant and the MCP
server come along for free. Tunnel died mid-session? `make away-stop && make away` — it
re-resolves DNS on every up.

⚠️ **`make away` works only from OUTSIDE the house.** The endpoint is the home *public* IP and
the router doesn't hairpin, so at home the packets die at the router. Run it there and it fails
in 3 s pointing you at `make tunnels` (rather than hanging `ssh` for minutes). Router rule,
DDNS, and the rest: *Remote access (WireGuard)* below.

## 4. [client] Qwen-Code harness

```bash
make install-client     # node ≥ 20 gate · qwen CLI · ~/.qwen/settings.json · smoke fixture
```

Idempotent — a second run skips everything already in place and downloads nothing. It **merges**
into an existing `~/.qwen/settings.json` rather than overwriting it, so anything you added by hand
(extra MCP servers, permissions) survives; a `.bak-<timestamp>` is written only when the merge
actually changes something. The three steps it performs, if you would rather do them by hand:

| step | guard |
|---|---|
| `npm install -g @qwen-code/qwen-code` | skipped when `qwen --version` succeeds |
| deploy `deploy/qwen/settings.json` — the `ep-rag` MCP server, telemetry and usage stats off | skipped when the merged result is already current |
| seed `~/qwen-scratch/notes.txt` with the artichoke fixture | skipped when the fixture is already there |

`make test-install` unit-tests the install scripts without installing anything.

Then point the CLI at the tunnelled server and verify the whole loop:

```bash
cd ~/qwen-scratch
export OPENAI_BASE_URL="http://localhost:8080/v1"
export OPENAI_API_KEY="dummy"           # llama-server has no key; any value works
export OPENAI_MODEL="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS"   # match /v1/models

qwen -p "Use your tools to read notes.txt and tell me the secret word."  # expect: artichoke
```

✅ A visible `read_file` tool call **and** the answer **"artichoke"** = the full loop works.
The first turn takes ~190 s (one-time prefill of the ~16k prompt); the warm cache then persists
across turns and sessions.

## 5. [client] Coding-quality gate (recommended before any config change)

Prove coding ability didn't regress when you change quant/KV/offload (`evals/`, methodology
adapted from Sebastian Raschka's `local-coding-agent-evals`). Server up + tunnel open:

```bash
# one-time: clone the agent-pack (unlicensed → used from a local clone, not vendored)
git clone https://github.com/rasbt/local-coding-agent-evals ~/qwen-scratch/raschka-evals

evals/quality_gate.sh <label>           # fast: 5 tool-reasoning tasks (~1 min, deterministic)
evals/quality_gate.sh <label> --full    # + agent-pack: 5 buggy repos fixed by qwen, graded by pytest
```
Baselines on this box: reasoning **1.00/5**, agent-pack **5/5**. Keep a config only if quality
≥ baseline. (The agent-pack runs `qwen --yolo` — see `evals/README.md`.)

---

# Why these choices (findings)

- **`--parallel 1` is non-negotiable.** Qwen-Code sends a ~16k-token system+tools prompt each
  session. One warm slot prefills it once (`sim ~0.997`) and reuses it across turns *and* new
  sessions; multiple slots re-prefill the whole 16k on every turn (~3 min to first token).
- **GPU offload of attention/KV wins at depth.** At 16k context gen is ~3 t/s CPU-only vs
  ~8.8 t/s with attention/KV on the GPU — the depth bottleneck is attention-over-KV, which the
  GPU accelerates. (At ~0 context CPU *looks* faster — misleading; always measure at 16k depth.)
- **Experts don't fit a 4 GB GPU** → `--cpu-moe` keeps them in RAM. Naively offloading experts
  (`--n-cpu-moe N<48`) causes per-token CPU↔GPU PCIe ping-pong and gives **no** net speedup.
- **IQ4_XS is the one throughput win**: +5 % `tg@16k` over Q4_K_M at identical coding quality
  (agent-pack 5/5). gen ∝ 1/expert-bytes, so a smaller quant ≈ linearly faster generation.
- **What did NOT help** (all measured at 16k depth): KV-cache quant (q4_0), expert offload,
  thread count, and speculative decoding (a general 0.6B draft — MoE routing gives weak
  batch-amortization). The ceiling is the per-token CPU expert read over ~45 GB/s RAM.
- **`qwen --yolo`** is required for headless *file-editing* agents (write/edit tools are
  approval-blocked with no TTY → the model loops); read-only tools work without it. `--yolo`
  auto-executes unsandboxed — sandbox it for untrusted work.

Full blow-by-blow (the original chronological build log, incl. the CPU-vs-GPU reversal) lives in
`git log` and `docs/`.

# Adapting to different hardware

The config assumes a **small GPU that can't hold the model + a fast CPU/RAM**. Retarget by VRAM:

- **GPU holds the whole model** (≥ ~18 GB free): drop `--cpu-moe` — `-ngl 99` puts everything on
  the GPU, far faster (no CPU expert read). Consider a higher quant (Q5/Q6) if VRAM allows.
- **Mid GPU (8-16 GB):** keep `--cpu-moe` but push some expert layers onto the GPU
  (`--n-cpu-moe N<48`) using the VRAM headroom — **measure**, ping-pong can cancel the gain.
- **Tiny GPU (this box, 4 GB):** the config as written. If VRAM OOMs: `--ctx-size 24576` (keep
  > 16k), or `q4_0` KV, or drop GPU offload entirely.
- **No GPU:** remove `-ngl / --cpu-moe / -fa / -ctk / -ctv`; expect much slower gen at depth.

Also: set `CMAKE_CUDA_ARCHITECTURES` for your GPU (step 1); re-pick the quant with the quality
gate (step 5); keep the model a **Qwen3-Coder MoE** (Qwen-Code is tuned for it). RAM must hold
the model + KV cache (~20 GB for IQ4_XS).

# LAN chat frontend (Open WebUI) — 2026-07-08

A ChatGPT-like UI for the household at **http://192.168.1.22:3000**, reusing the loopback
llama-server (which stays loopback-only; Open WebUI adds its own login wall). Runs as a Docker
Compose service on weebeastie — `restart: unless-stopped` + docker `enabled` at boot ⇒ survives
reboots. Full rationale: `docs/plans/2026-07-08-lan-chat-frontend-openwebui-design.md`.

**Version-controlled config:** `deploy/openwebui/docker-compose.yml` (+ `.env.example`). Secrets
live in a gitignored `deploy/openwebui/.env` (`WEBUI_SECRET_KEY`, `EXA_API_KEY`). Pinned image
`ghcr.io/open-webui/open-webui:v0.10.2`. Key config: host networking (to reach loopback model),
`PORT=3000`, fixed accounts w/ signup locked, per-user memory on, document-RAG **off** (no
embedder downloaded — `RAG_EMBEDDING_ENGINE=openai` + bypass), Exa web search on (with
per-query confirmation), telemetry/version-check/community-sharing off, `cap_drop: [ALL]` +
`no-new-privileges`.

**Deploy / ops (on weebeastie, from `~/openwebui/`):**
```bash
docker compose up -d          # start (also pulls); down / restart / ps as usual
docker compose logs -f        # live logs
docker compose pull && docker compose up -d   # update after bumping the image tag
```

**First-run browser steps (once):** (signup is already locked — `enable_signup=false`,
`onboarding=true` — so only the first admin can self-create.)
1. Open `http://192.168.1.22:3000`, register **filip** → auto-admin.
2. Admin Panel → Users: add **spouse** and **guest** (shared password).
3. Admin Panel → Settings → Web Search → paste the **Exa API key** (this is authoritative —
   `EXA_API_KEY` is a ConfigVar already seeded empty, so editing `.env` + re-up won't override
   it). Record the key in `~/openwebui/.env` too, so a volume-wipe re-seed still has it.

No need to set "Function Calling = Native": in v0.10.0+ **Native (Agentic) mode is the default**
for all models (the old "Default" was renamed "Legacy" and is the opt-out). `search_web` and the
memory tools work out of the box. The mode lives under a model's *Advanced Params → Function
Calling* if you ever need to inspect it.

⚠️ Most settings are Open WebUI "ConfigVar" — the compose env only **seeds first boot**;
afterwards the volume is authoritative, so change settings in the Admin Panel (or wipe the
`open-webui` volume to re-seed).

# EP-Committee RAG — 2026-07-11

A grounded, cited Q&A assistant over EMPL/REGI/IMCO committee PDFs. 
Rust ingestion + index; org-babel notebooks for the debugging drills 
and the hand-stitched query path. Full design: `docs/plans/2026-07-10-ep-committee-rag-design.md`.

**Stack:** Qdrant v1.18 (`deploy/qdrant/`) · `BAAI/bge-small-en-v1.5` embeddings (candle, pure
Rust) · generator = the loopback `llama-server` (`:8080/v1`). One code path, two deploys via env.

**Run (from `rag/`; all run targets are `--release` — candle in debug is 10–50× slower):**
```bash
make pipeline        # qdrant-up → fetch → ingest → index   (12 PDFs → 490 chunks → Qdrant)
make parity          # candle vs sentence-transformers embed parity gate (cosine 1.0)
make serve-mcp       # rag-mcp: MCP search tool + /retrieve + /route on loopback :8082
make qdrant-status   # collection point count · make help for all targets
```

**Reach the remote (weebeastie) Qdrant from the laptop (SSH tunnel).** On weebeastie Qdrant binds
**loopback-only** (only Open WebUI's `:3000` faces the LAN), so tunnel to inspect the production
index / dashboard. The laptop's own Qdrant already owns `6333/6334`, so map to spare local ports:
```bash
ssh -fN -L 16333:127.0.0.1:6333 -L 16334:127.0.0.1:6334 filip@192.168.1.22
curl -s http://localhost:16333/collections/ep_committee_docs | grep -o '"points_count":[0-9]*'  # → 490
# REST + dashboard: http://localhost:16333/dashboard   ·   gRPC (qdrant-client): localhost:16334
```
Stop later: `pkill -f "ssh -fN -L 16333"`.

**State (2026-07-11):**
- **Index built** — collection `ep_committee_docs`, **490 points**; each carries full provenance +
  `text` + the six `contract_*` fields (the embedding contract stamped per point, so a mismatched
  serve can be refused).
- **Query path hand-stitched** — `rag/notebooks/rag_query.org`: embed_query → Qdrant top-k →
  grounded prompt → Qwen → cited answer (~15 s warm). The executable spec for the Rust
  `retrieve`/`generate` crates.
- **Drill 0 (index health)** — `rag/drills/drill0_index_health.org`: integrity, separation
  (anisotropic cone, mean cosine 0.68), near-dups, self-retrieval probe.
- **`retrieve`/`generate` crates built (2026-07-11)** — the notebook productized into workspace
  crates: `rag-retrieve` (bge query-prefix embed → Qdrant gRPC top-k) and `rag-generate`
  (grounded `assemble`, `<think>` strip, serve-time provenance/Sources join, UTF-8-safe streaming +
  non-streaming llama-server client). Both are what `rag-mcp` is built on.
- **`ep-rag-server` removed (2026-09-03)** — an OpenAI-compatible RAG face on `:8081`, built but
  never deployed. Its purpose was to be Open WebUI's second model; what actually shipped instead
  was the `/route` filter calling `rag-mcp`, which reaches the same goal through the service
  that was already running. Rather than keep a second, unproven front door to the same index, the
  crate and its unit file are deleted — recoverable from git, and the plan that produced it stays
  in `docs/plans/` as the record.
- **Next** — the two named debugging drills (embedding-mismatch, confidence-blind fusion) + eval
  harness.

# Remote access (WireGuard) — 2026-07-16

Reach the model from outside the house — a travel laptop points `OPENAI_BASE_URL` at the home
box and `qwen` works exactly as at home. Design + rationale:
`docs/plans/2026-07-16-remote-model-access-design.md` · steps:
`docs/plans/2026-07-16-remote-model-access-implementation-plan.md`.

**The invariant: `llama-server` is unchanged.** It stays bound to `127.0.0.1:8080` and never
learns any of this happened. WireGuard authenticates the tunnel (X25519); SSH authenticates
the user (existing RSA-4096 key); the SSH forward is byte-identical to the LAN one — only the
host changes. The model is never bound to a network interface, so there is no new auth surface
and no `--api-key` bearer string to leak.

```
travel laptop                                weebeastie (home)
  make away                                    wg0 10.10.0.1 ◀── UDP 51820 forwarded
   ├─ wg-quick up wg0  ═══ WireGuard ═══▶      sshd (LAN + wg0 only; :22 NOT forwarded)
   └─ ssh -fN -L 8080 filip@10.10.0.1  ───▶    127.0.0.1:8080 llama-server
```

**Use it:**
```bash
make away         # off-LAN: wg up (+ reachability check) → llama/qdrant/mcp tunnels over it
make tunnels      # at home: straight over the LAN (make away will NOT work here — see below)
make wg-status    # endpoint / handshake / transfer
make away-stop    # tear it all down
```

**Router:** one rule — `UDP 51820 → 192.168.1.22`. Port **22 is deliberately not forwarded**:
WireGuard is the front door, SSH lives behind it. An unauthenticated WireGuard packet draws *no
reply at all*, so the box stays invisible to internet-wide scanners.

**`make away` only works from OUTSIDE the house.** The endpoint is the home public IP and the
b-box doesn't do NAT loopback (hairpinning), so from the LAN the packets die at the router. This
is also why the tunnel is **not** a boot-time service — an auto-started tunnel would be
permanently broken at home, which is where the laptop mostly is. `wg-up` pings the peer and
tears the interface back down on failure, so this fails in 3 s with an explanation instead of
hanging `ssh` for minutes.

**DDNS is load-bearing, not a nicety.** RIPE marks the ISP range `xDSL customers (dynamic)` —
the home IP is *sticky*, not static. Without DDNS, a line resync while you're abroad is an
**unrecoverable lockout**: fixing the hardcoded endpoint needs the new IP, and learning the new
IP needs access to the house. deSEC (`<DDNS_HOST>`, TTL 60) + a 5-min updater timer on
weebeastie closes it.

**Tunnel died mid-session? Bounce it:** `make away-stop && make away`. `wg-quick` resolves
`Endpoint` **once at interface start and never re-resolves**, so a live tunnel sits on a stale
address — but a bounce re-resolves, and every `make away` starts fresh anyway. A `reresolve-dns`
timer was considered and **rejected as YAGNI**: an IP change kills the SSH session regardless, so
healing the WireGuard layer rescues nothing, and it'd be *slower* (~4 min) than bouncing by hand
(~10 s). See the design doc for the full reasoning.

⚠️ **The updater deletes the AAAA record on purpose** (`curl -4` + `myipv6=`). weebeastie has
native IPv6, and a published AAAA would let `wg-quick` resolve the endpoint to v6 — which works
beautifully at home and then fails on IPv4-only café wifi, i.e. exactly when you can't debug it.

**Verified (2026-07-16):** handshake + model response from outside via a NordVPN exit, over both
the raw IP and the DDNS hostname. That test also settled the one fact unfalsifiable from inside
the LAN — the public IPv4 is **not** behind carrier NAT. Testing the forward *from* the LAN is a
false negative (hairpinning); use NordVPN or a phone hotspot.

**sshd is hardened** (2026-07-16): key-only, no root login. Verify by *behaviour*, not by
reading the config — `ssh -o PreferredAuthentications=password -o PubkeyAuthentication=no
filip@192.168.1.22` must answer `Permission denied (publickey)`; that list is the only method
the server accepts.

**Pending:** Stage-2 verification from a phone hotspot (carrier CGNAT — a harder, more realistic
test than a clean Nord datacenter exit) · proving DDNS *recovery* by forcing a new lease (today
only proves the updater runs, not that it heals).

⚠️ **Secrets:** `/etc/wireguard/*` and `/etc/desec-updater.env` are root-only and live outside
the repo. This repo is **public** — never commit the home IP, the DDNS hostname, or private
keys. See the warning at the top of `CLAUDE.md`.

# Docs

- `CLAUDE.md` — orientation for agents (hardware, ops, findings, TODOs).
- `program.md` — the autonomous, quality-gated throughput-tuning loop.
- `docs/plans/2026-07-06-local-coding-harness-design.md` — full design + rationale.
- `docs/plans/2026-07-06-quality-gated-autoperf-design.md` — the quality-gate loop design.
- `docs/autoperf-reports/2026-07-06-autoperf-report.md` — tuning results (every config tried).
- `evals/` — the coding-quality gate · `results.tsv` — raw per-config numbers.
- `docs/plans/2026-07-10-ep-committee-rag-design.md` — EP-committee RAG design + status.
- `docs/plans/2026-07-11-openwebui-rag-integration-design.md` — wiring the RAG behind Open WebUI (superseded: shipped as the `/route` filter against `rag-mcp`).
- `docs/plans/2026-07-11-rag-server-implementation-plan.md` — the `rag-server` build plan (historical; the service was removed 2026-09-03).
- `docs/plans/2026-07-08-lan-chat-frontend-openwebui-design.md` — the Open WebUI chat frontend.
- `docs/plans/2026-07-16-remote-model-access-design.md` — remote access via WireGuard (design + rejected alternatives).
- `docs/plans/2026-07-16-remote-model-access-implementation-plan.md` — the 13-task WireGuard build plan.
