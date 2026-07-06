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

~/Programs/llama.cpp/build/bin/llama-cli \
  -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M \
  -p "Say hello in exactly one word." -n 8 -no-cnv
```

Expect: download bars → `ggml_cuda_init: found 1 CUDA devices: ... GTX 1050 Ti` → a word.

_Result: (pending — paste output)_
