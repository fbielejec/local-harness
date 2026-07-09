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
| Model | `unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS` (~15.25 GiB, MoE 30B / ~3B active) — autoperf winner (2026-07-07); Q4_K_M was the prior default |
| Server log | remote `~/llama-server.log` |
| Qwen-Code scratch workspace | laptop `~/qwen-scratch` |
| Chat frontend (Open WebUI, Docker) | remote `~/openwebui/` (compose + gitignored `.env`); UI at `http://192.168.1.22:3000`; config in `deploy/openwebui/` |

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

- [ ] RAG. Embedding model, retrieval index split per user.
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

## Docs

- `README.md` — chronological runbook (every command run, with results).
- `docs/plans/2026-07-06-local-coding-harness-design.md` — full design + rationale.
- `docs/plans/2026-07-06-quality-gated-autoperf-design.md` — the quality-gated tuning loop.
- `docs/autoperf-reports/2026-07-06-autoperf-report.md` — throughput + quality tuning results.
- `evals/` — coding-quality gate (reasoning + agent-pack; methodology adapted from Raschka).
- `program.md` — the autonomous, quality-gated tok/s optimization loop.
