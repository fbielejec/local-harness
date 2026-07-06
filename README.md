# Local Agentic Coding Harness

Setup log for running an open-weights coding agent locally.
Design: [`docs/plans/2026-07-06-local-coding-harness-design.md`](docs/plans/2026-07-06-local-coding-harness-design.md).

Stack: **Qwen3-Coder-30B-A3B-Instruct** (GGUF Q4_K_M) → **llama-server** on the remote →
SSH tunnel → **Qwen-Code** harness on the laptop.

## Access to the remote machine

```bash
ssh filip@192.168.1.22
```

---

# Runbook (commands, in order)

Commands are logged here as we run them. `[remote]` = run on `weebeastie` (192.168.1.22);
`[laptop]` = run on the dev machine.

## Phase A — Remote model server

### A0 · Recon: what toolchain already exists  `[remote]`

```bash
echo "=== git ==="; git --version 2>/dev/null || echo MISSING
echo "=== cmake ==="; cmake --version 2>/dev/null | head -1 || echo MISSING
echo "=== gcc ==="; gcc --version 2>/dev/null | head -1 || echo MISSING
echo "=== make ==="; make --version 2>/dev/null | head -1 || echo MISSING
echo "=== nvcc (CUDA toolkit) ==="; nvcc --version 2>/dev/null | tail -1 || echo MISSING
echo "=== CUDA libs ==="; ls -d /usr/local/cuda* 2>/dev/null || echo "no /usr/local/cuda"
echo "=== ccache ==="; ccache --version 2>/dev/null | head -1 || echo MISSING
echo "=== curl dev ==="; dpkg -l | grep -E "libcurl4|libcurl.*dev" | awk '{print $2}' || echo none
```

_Result: git 2.43, cmake 3.28.3, gcc 13.3.0, make 4.3, ccache 4.9.1, libcurl4-gnutls-dev
present. **CUDA toolkit MISSING** (driver 535 is installed, but no `nvcc`). → install it (A1)._

### A1 · Install CUDA toolkit 12.6 (toolkit only, no driver)  `[remote]`

gcc 13.3 requires CUDA ≥ 12.4, so we use NVIDIA's repo (Ubuntu's `nvidia-cuda-toolkit` is
12.0 and rejects gcc 13). Installs to `/usr/local/cuda-12.6`; driver 535 untouched.

```bash
cd /tmp
wget https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2404/x86_64/cuda-keyring_1.1-1_all.deb
sudo dpkg -i cuda-keyring_1.1-1_all.deb
sudo apt update
sudo apt install -y cuda-toolkit-12-6

nvcc --version   # expect: release 12.6
```

Persist the env for future interactive shells (systemd services get it explicitly later, in E):

```bash
grep -q 'cuda-12.6/bin' ~/.bashrc || cat >> ~/.bashrc <<'EOF'

# CUDA 12.6 toolkit
export PATH=/usr/local/cuda-12.6/bin:$PATH
export LD_LIBRARY_PATH=/usr/local/cuda-12.6/lib64:$LD_LIBRARY_PATH
EOF

source ~/.bashrc
which nvcc          # expect: /usr/local/cuda-12.6/bin/nvcc
nvcc --version      # expect: release 12.6
```

_Result: **CUDA 12.6.85 installed** and on PATH (`nvcc release 12.6, V12.6.85`)._

### A2 · Build llama.cpp with CUDA (Pascal sm_61)  `[remote]`

CUDA on, targeting the 1050 Ti's `sm_61`; FA-all-quants so quantized KV cache works;
libcurl on for URL model pulls.

```bash
cd ~/Programs
git clone https://github.com/ggml-org/llama.cpp
cd llama.cpp

cmake -B build \
  -DGGML_CUDA=ON \
  -DCMAKE_CUDA_ARCHITECTURES=61 \
  -DGGML_CUDA_FA_ALL_QUANTS=ON \
  -DLLAMA_CURL=ON

cmake --build build --config Release -j$(nproc)   # ~15–25 min on 10 cores

ls -lh ~/Programs/llama.cpp/build/bin/llama-server
~/Programs/llama.cpp/build/bin/llama-server --version
```

_Result: **built OK** — llama-server version 9886 (20a04b220), GNU 13.3.0. (CUDA-active
check happens at first launch in A3.) Binary at `~/Programs/llama.cpp/build/bin/`._

### A3a · Download model + first-token smoke test  `[remote]`

Model: `unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M` (confirmed best-available GGUF,
~18 GB). llama.cpp downloads it via `-hf` (LLAMA_CURL); cache pointed at `~/models`.
This run is CPU-only (no `-ngl`) — just proves the model loads, generates, and the CUDA
build sees the GPU.

```bash
mkdir -p ~/models
export LLAMA_CACHE=$HOME/models

# NOTE: build 9886 dropped `-no-cnv` from llama-cli; use `llama-completion` for one-shot.
~/Programs/llama.cpp/build/bin/llama-completion \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  -p "Say hello in exactly one word." -n 8
```

Expect: download bars → `ggml_cuda_init: found 1 CUDA devices: ... GTX 1050 Ti` → a word.

_Result: **works** — downloaded, generated "Hello!". **Generation 23.4 t/s, prompt 51.7 t/s
on pure CPU** (no `-ngl`) — beats the 8–15 t/s estimate. GPU offload deferred to Phase D.
(Ran via llama-cli's interactive UI since `-no-cnv` is gone in this build; `llama-completion`
is the correct one-shot binary.)_

### A3b · Launch llama-server (CPU-first) + validate on remote  `[remote]`

CPU-first to reach a working pipeline fast; GPU offload is Phase D. Loopback-only bind
(reached via SSH tunnel), `--jinja` for Qwen-Code tool-calling. No api-key yet (Phase E).

```bash
export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  --host 127.0.0.1 --port 8080 --threads 10 --ctx-size 32768 --jinja \
  > ~/llama-server.log 2>&1 &

tail -f ~/llama-server.log     # Ctrl-C once you see "server is listening"
```

Validate on the remote (same terminal):

```bash
curl -s http://127.0.0.1:8080/v1/models
echo
curl -s http://127.0.0.1:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen","messages":[{"role":"user","content":"Reply with the single word: pong"}],"max_tokens":10}'
```

_Result: **Phase A ✅** — `/v1/models` lists the model (n_ctx served 32768, n_ctx_train
**262144** = 256k native); chat completion returned "pong" at **46.8 t/s prompt /
29.3 t/s gen**. Server running in background on the remote (`~/llama-server.log`)._

## Phase B — SSH tunnel from the laptop + validate  `[laptop]`

Backgrounded tunnel (`-fN` = no shell, background after auth). Maps laptop `localhost:8080`
→ remote `127.0.0.1:8080`. Then curl from the laptop to prove the round-trip.

```bash
ssh -fN -L 8080:127.0.0.1:8080 filip@192.168.1.22

curl -s http://localhost:8080/v1/models | head -c 200
echo
curl -s http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen","messages":[{"role":"user","content":"Reply with: tunnel-ok"}],"max_tokens":10}'
```

To stop the tunnel later: `pkill -f "ssh -fN -L 8080"`.

_Result: **Phase B ✅** — laptop `localhost:8080` reaches the remote model; returned
"tunnel-ok". Full laptop→remote pipeline live._

## Phase C — Qwen-Code harness  `[laptop]`

Node CLI, needs Node ≥ 20. Model id must match what `/v1/models` reports.

### C1 · Install

```bash
node --version     # need v20+
npm --version
npm install -g @qwen-code/qwen-code
qwen --version
```

_Result: **installed** — Node v23.9.0, npm 10.9.2, qwen-code **0.19.6**._

### C2 · Configure + tool-call milestone

Scoped scratch workspace (not `$HOME`). Prove the full loop incl. tool-calling.

```bash
mkdir -p ~/qwen-scratch && cd ~/qwen-scratch
printf 'The secret word is: artichoke.\n' > notes.txt

export OPENAI_BASE_URL="http://localhost:8080/v1"
export OPENAI_API_KEY="dummy"     # llama-server has no key; any value works
export OPENAI_MODEL="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M"

qwen -p "Use your tools to read notes.txt in the current directory and tell me the secret word."
```

Success = a visible tool call (read_file) + answer "artichoke". This is the viability
checkpoint (full loop incl. `--jinja` tool templating).

_Result: **connects, but too slow to be usable on the A3b (CPU, 4-slot) server.** Server log
showed the real problem: Qwen-Code sends a **~14,800-token** system+tools prompt, and (a)
CPU prefill degrades 151→72 t/s over that length (~3 min to first token), and (b) with
`--parallel 4` each turn landed on a **different cold slot → re-prefilled the whole 15k
every turn**. → Bring Phase D forward: `--parallel 1` (warm cache) + GPU offload of
attention/KV._

## Phase D — Performance tuning (brought forward — required for usability)

### D1 · Relaunch: single slot + MoE-aware GPU offload  `[remote]`

`--parallel 1` = one warm slot so the 15k system prompt is prefilled once and reused across
turns (the biggest win). `-ngl 99 --cpu-moe` = attention/KV/router on the 4 GB GPU, expert
FFNs stay in RAM. `q8_0` KV cache + `-fa on` so 32k context fits in 4 GB. `--no-mmap` per
the earlier perf warning.

```bash
pkill -f "build/bin/llama-server"
sleep 2

export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  --host 127.0.0.1 --port 8080 \
  --threads 10 \
  --parallel 1 \
  --ctx-size 32768 \
  --n-gpu-layers 99 \
  --cpu-moe \
  -fa on \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap \
  --jinja \
  > ~/llama-server.log 2>&1 &

tail -f ~/llama-server.log
nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu --format=csv
```

Target: `nvidia-smi` memory.used < ~3.9 GB. OOM → drop to `--ctx-size 24576` or `q4_0` KV.

_Result: **fits** — `n_slots = 1`, VRAM **2861 / 4096 MiB** (~1.2 GB headroom), loaded in
6.6 s. GPU offload live. (Headroom = room to push a few experts to GPU later via
`--n-cpu-moe N<48`.) Re-testing harness in D2._

### D2 · Re-test harness on the tuned server

```bash
cd ~/qwen-scratch
qwen -p "Use your tools to read notes.txt in the current directory and tell me the secret word."
# then a follow-up turn in the TUI to confirm warm-cache reuse:
#   "Now tell me how many characters are in that word."
```

Watch `tail -f ~/llama-server.log`: compare prefill t/s vs the CPU run (151→72), and confirm
the follow-up turn prefills only a few new tokens (not 15k).

_Result: **mixed — two findings.** (1) ✅ `--parallel 1` cache reuse **works**: follow-up
turn matched `sim 0.997`, prefilled only **55 tokens**, total **4.5s**. (2) ❌ GPU offload
**backfired**: generation **29 → 8.7 t/s** (3.3× slower) from CPU↔GPU PCIe ping-pong (attn
on GPU, MoE experts on CPU, every layer/token) — while prefill barely moved (~86 t/s).
First-turn prefill of the 16k prompt still ~190s. **Conclusion: CPU-only wins for this MoE;
the 4 GB Pascal can't hold experts, so offload is a net loss.** → D3 reverts GPU, keeps
`--parallel 1`, adds `--threads-batch 20` to speed prefill._

### D3 · CPU-only + single slot + fast-prefill threads  `[remote]`

No GPU (offload hurts this MoE). `--threads-batch 20` = all logical cores for compute-bound
prefill; `--threads 10` = physical cores for memory-bound generation. f16 KV cache (plenty
of RAM). Keep `--parallel 1` for warm-cache reuse.

```bash
pkill -f "build/bin/llama-server"
sleep 2

export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  --host 127.0.0.1 --port 8080 \
  --threads 10 \
  --threads-batch 20 \
  --parallel 1 \
  --ctx-size 32768 \
  --no-mmap \
  --jinja \
  > ~/llama-server.log 2>&1 &
```

Compare: generation ~29 t/s (vs 8.7), prefill hopefully > 90 t/s, follow-ups reuse cache.

_Result: **REVERSED the D2 conclusion — GPU offload actually wins.** CPU-only gen @16k ctx
was **3.07 t/s** vs GPU offload's **8.7 t/s** — the earlier "29 t/s" was at ~0 context, not
comparable. At real (16k) context the bottleneck is attention-over-KV, which the GPU
accelerates. `--threads-batch 20` did NOT help prefill (85.9 vs 86.5). → Revert to the GPU
config (D1) as production; see D4._

### D4 · Production config = GPU offload (corrected)  `[remote]`

At working context the GPU offload gives ~2.8× generation vs CPU-only. This is the config
to keep. Remaining issue: ~190s **one-time** first-turn prefill of Qwen-Code's ~16k prompt
(cache then persists across sessions until evicted).

```bash
pkill -f "build/bin/llama-server"
sleep 2

export LLAMA_CACHE=$HOME/models
nohup ~/Programs/llama.cpp/build/bin/llama-server \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  --host 127.0.0.1 --port 8080 \
  --threads 10 \
  --parallel 1 \
  --ctx-size 32768 \
  --n-gpu-layers 99 --cpu-moe \
  -fa on --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap --jinja \
  > ~/llama-server.log 2>&1 &
```

Perf @16k ctx: prefill ~86 t/s (first turn ~190s, one-time), gen ~9 t/s, follow-ups reuse
cache (~seconds). VRAM ~2.9 GB.

_Result: (pending — confirm qwen returned "artichoke" + tool call)_

---

## Phase F — Household chat frontend (Open WebUI) · DEFERRED until harness done

LAN-accessible ChatGPT-like UI for household (e.g. spouse), reusing the same loopback
`llama-server`. Docker, host-network, fixed port 3000, login wall. `llama-server` stays
loopback-only. See design doc "Household chat frontend". Draft command (finalize when we
get here):

```bash
docker run -d --name open-webui --network host \
  -e PORT=3000 \
  -e OPENAI_API_BASE_URL=http://127.0.0.1:8080/v1 \
  -e OPENAI_API_KEY=dummy \
  -v open-webui:/app/backend/data --restart always \
  ghcr.io/open-webui/open-webui:main
```
