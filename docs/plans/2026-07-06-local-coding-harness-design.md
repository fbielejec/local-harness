# Local Agentic Coding Harness — Design

**Date:** 2026-07-06
**Status:** Approved design, ready for implementation

## Motivation

Run an agentic coding assistant entirely on local/self-hosted hardware, driven by
open-weights models — no code or prompts leaving the network. Inspired by:

- Vitalik Buterin, *Secure LLMs* (https://vitalik.eth.limo/general/2026/04/02/secure_llms.html) — local-first inference, open weights,
  and "sandbox everything" with human-approval gates as non-negotiable principles.
- Sebastian Raschka, *Using local coding agents* (https://magazine.sebastianraschka.com/p/using-local-coding-agents) — validates small-active-parameter
  MoE models (Qwen3.x **A3B** class) as the sweet spot, and notes harnesses differ sharply in token appetite (Codex fewest → Claude Code most).

## Hardware (remote box: `weebeastie`, 192.168.1.22)

| Component | Spec | Implication |
|-----------|------|-------------|
| GPU | GTX 1050 Ti, **4 GB VRAM**, Pascal | Marginal; offload only a few layers. Not the engine. |
| CPU | i9-10850K, 10c / 20t | The actual inference engine. |
| RAM | 62 GB (~58 free) | The real model budget: ~30–45 GB on-disk models with headroom. |
| Disk | 661 GB free | Plenty for weights. |
| OS | Linux Mint 22.1 (kernel 6.8) | — |

**Consequence:** 355B-class models (GLM-4.6 / GLM-5.2) and dense 70B are out of reach.
Token generation is **memory-bandwidth-bound** (~45 GB/s dual-channel DDR4), so a small
**MoE with ~3B active params** is the only path to a usable agent loop.

## Key decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Model | **Qwen3-Coder-30B-A3B-Instruct**, GGUF, **Q4_K_M** (~18–19 GB) | Coder-specialized MoE; ~3B active = fast on CPU; Q4_K_M is the bandwidth-friendly sweet spot. Confirm best-available build at setup. |
| Serving | **`llama-server`** (llama.cpp) | Single static binary, no daemon, OpenAI `/v1` endpoint, full CPU/GPU tuning knobs; transparent (fits security posture). |
| Harness | **Qwen-Code** | OpenAI-native (no Anthropic proxy shim), tuned for Qwen, lighter token appetite than Claude Code. |
| Topology (now) | **Split** — model+server on remote, SSH tunnel to laptop `localhost:8080`, Qwen-Code + repos local | Heavy inference on strong box; edit where you work; port never on the LAN. |
| Topology (later) | All-on-remote sandbox | Remote becomes a genuine agent sandbox, not the daily driver. |

Benchmarking is treated as a **repeatable** step for future hardware/software upgrades,
not a gate on the initial build.

**Expected performance:** ~8–15 tok/s generation (well below Raschka's 30–40 tok/s on
Apple Silicon / DGX Spark, which have much higher memory bandwidth). Usable for agents.

## Architecture

```
┌─────────────────────────────┐         SSH tunnel          ┌──────────────────────────────┐
│  This laptop (dev machine)  │   localhost:8080  ◀────────▶│  weebeastie  192.168.1.22    │
│                             │   (encrypted, LAN-only)     │  i9-10850K · 62 GB · 1050 Ti │
│  • Qwen-Code (the harness)  │                             │                              │
│  • your local git repos     │                             │  • llama-server (llama.cpp)  │
│  • you edit here            │                             │  • Qwen3 Coder MoE (GGUF)    │
└─────────────────────────────┘                             │  • OpenAI /v1 endpoint        │
                                                            └──────────────────────────────┘
```

## Serving config

`llama-server` on the remote, bound to loopback only:

```bash
llama-server \
  --model ~/models/qwen3-coder-30b-a3b-instruct-Q4_K_M.gguf \
  --host 127.0.0.1 --port 8080 \
  --threads 10 \                # physical cores; hyperthreads hurt
  --n-gpu-layers 8 \            # offload a few layers to the 4 GB card; tune down if OOM
  --ctx-size 32768 \            # KV cache; size against free RAM
  --cache-type-k q8_0 --cache-type-v q8_0 \  # quantized KV cache = more context per GB
  --flash-attn \
  --jinja \                     # model chat/tool-call template (required for Qwen-Code tools)
  --api-key <local-secret>
```

Tuning levers for the benchmark loop: `--threads`, `--n-gpu-layers`, `--ctx-size`,
KV-cache quant. 32 K of KV cache costs real RAM on top of the ~19 GB weights; agent
runs consume context quickly, so size `--ctx-size` against actual free RAM.

Tunnel from the laptop:

```bash
ssh -N -L 8080:127.0.0.1:8080 filip@192.168.1.22
```

`http://localhost:8080/v1` on the laptop is then the remote model — encrypted, LAN-only.

## Harness wiring

```bash
export OPENAI_BASE_URL="http://localhost:8080/v1"
export OPENAI_API_KEY="<local-secret>"        # matches llama-server --api-key
export OPENAI_MODEL="qwen3-coder-30b-a3b-instruct"
```

## Security / sandbox gates

- **Approval gates:** run Qwen-Code in confirmation mode — shell commands and file
  writes require explicit human OK. The single most important control in topology-1.
- **Scope the workspace:** launch the harness from inside the target repo, never `$HOME`.
- **Treat fetched web content as hostile** (prompt-injection surface): the model can't
  exfiltrate, but agent-executed commands can.
- **Later:** the all-on-remote topology makes the remote a real sandbox.

## Out of scope (YAGNI)

No multi-model router, no Ollama registry, no web UI, no fine-tuning, no containerized
agent (yet). One model, one endpoint, one harness; manual first, then systemd.

## Setup runbook

**Phase A — Remote model server**
1. SSH in; install build deps + CUDA toolkit; clone and build `llama.cpp` with `-DGGML_CUDA=ON`.
2. Download `Qwen3-Coder-30B-A3B-Instruct` Q4_K_M GGUF into `~/models/` (confirm best-available build).
3. Launch `llama-server` manually with the tuned flags, bound to `127.0.0.1:8080`.
4. Validate on the remote: `curl 127.0.0.1:8080/v1/models` and a tiny `/v1/chat/completions` call.

**Phase B — Tunnel + first token from the laptop**
5. Open `ssh -N -L 8080:127.0.0.1:8080 filip@192.168.1.22`.
6. From the laptop: `curl localhost:8080/v1/models` → confirm round-trip.

**Phase C — Harness**
7. Install Qwen-Code; set the three env vars; smoke-test a trivial prompt end-to-end.
8. Confirm **tool-calling** works (read/list a file) — where template/`--jinja` issues surface.

**Phase D — Real run + tuning**
9. Run one small real coding task in a scratch repo, in **approval mode**.
10. Record baseline tok/s + quality gaps → decide on Q5_K_M vs `--threads`/`--n-gpu-layers`/`--ctx-size`.

**Phase E — Make it durable**
11. systemd user service for `llama-server` (auto-restart/boot); `autossh` service for the tunnel.

**First checkpoint to prove viability:** Phase C step 8 — a working tool call
end-to-end. Everything before is plumbing; that is the moment it becomes a real agent.
