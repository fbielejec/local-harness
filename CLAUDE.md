# CLAUDE.md — local_coding_harness

Orientation for agents. Read this, then `program.md` (the optimization loop) and
`docs/plans/2026-07-06-local-coding-harness-design.md` (the full design).

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

| Thing | Location |
|-------|----------|
| llama.cpp (built, CUDA 12.6, sm_61) | remote `~/Programs/llama.cpp`, binaries in `build/bin/` |
| Model weights (GGUF, via `-hf` cache) | remote `~/models/` (`export LLAMA_CACHE=$HOME/models`) |
| Model | `unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M` (~18.5 GB, MoE 30B total / ~3B active) |
| Server log | remote `~/llama-server.log` |
| Qwen-Code scratch workspace | laptop `~/qwen-scratch` |

## Operating the server

SSH: `ssh filip@192.168.1.22`

**Current production launch (config "D4")** — CPU experts + GPU attention/KV, single warm slot:

```bash
export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  --host 127.0.0.1 --port 8080 \
  --threads 10 --parallel 1 --ctx-size 32768 \
  --n-gpu-layers 99 --cpu-moe \
  -fa on --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap --jinja \
  > ~/llama-server.log 2>&1 &
```

Restart: `pkill -f "build/bin/llama-server"; sleep 2; <relaunch>`
Watch:   `tail -f ~/llama-server.log`   ·   VRAM: `nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu --format=csv`

**Tunnel (laptop):** `ssh -fN -L 8080:127.0.0.1:8080 filip@192.168.1.22` (kill: `pkill -f "ssh -fN -L 8080"`)

**Run the harness (laptop):**
```bash
cd ~/qwen-scratch
export OPENAI_BASE_URL="http://localhost:8080/v1" OPENAI_API_KEY=dummy \
       OPENAI_MODEL="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M"
qwen -p "Use your tools to read notes.txt and tell me the secret word."   # expect: artichoke
```

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

## Current goal

**Improve tokens/s** (see `program.md`). Primary metric: generation t/s at ~16k context.
Baseline (D4): pp ≈ 86 t/s, tg@16k ≈ 9 t/s, VRAM ≈ 2.9 GB.

## TODOs

Running list of follow-ups (check off as done; newest at the bottom).

- [ ] **Basic security via `~/.qwen/settings.json`** — the file doesn't exist yet; create it
  to harden the Qwen-Code client for the self-sovereignty goal. Verify the exact schema
  against the installed `qwen` version first, then set at least: external telemetry/usage
  reporting **off** (local `~/.qwen/usage/` is fine — nothing should leave the LAN);
  tool-execution **sandbox** on (docker/podman/`sandbox-exec`); **no blanket auto-approve**
  of shell commands (require approval or confine to `~/qwen-scratch`); tool allow/deny list
  (`coreTools`/`excludeTools`) + MCP server allowlist. `llama-server` stays loopback-only.
  **Now concrete:** the coding-quality gate (`evals/agent_pack_runner.sh`) runs `qwen --yolo`
  — edits/shell auto-execute *unsandboxed* at the user's privilege (qwen even warns about it).
  Wrap those eval runs in a sandbox (`qwen --sandbox` / `QWEN_SANDBOX`, docker/podman) so the
  agent can't touch the host. (Root cause found 2026-07-06: headless `qwen -p` silently blocks
  every write/edit tool without `--yolo`, because there's no TTY to approve — so `--yolo` is
  mandatory for the eval, which is exactly why the sandbox matters.)

## Docs

- `README.md` — chronological runbook (every command run, with results).
- `docs/plans/2026-07-06-local-coding-harness-design.md` — full design + rationale.
- `program.md` — the autonomous tok/s optimization loop.
