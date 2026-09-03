#!/usr/bin/env bash
# deploy/install/build-llama.sh — README §1 as a script: build llama.cpp with CUDA.
#
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
# SCRIPT_DIR, never HERE: tests/run.sh owns HERE.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"

LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
LLAMA_BIN="$LLAMA_DIR/build/bin/llama-server"

# Clone-or-fetch, so a re-run cannot hard-fail on an existing directory.
sync_source() {
  mkdir -p "$(dirname "$LLAMA_DIR")"
  if [ -d "$LLAMA_DIR/.git" ]; then
    log "updating existing clone at $LLAMA_DIR"
    git -C "$LLAMA_DIR" fetch --depth 1 origin || die "git fetch failed in $LLAMA_DIR"
    # origin/HEAD is NOT guaranteed to exist: a clone made with --depth or an older
    # git leaves the symref unset, and `reset --hard origin/HEAD` then fails with a
    # bad-revision error that reads like a corrupt repo. Fall back to the branch the
    # remote actually tracks; llama.cpp's is master.
    local ref
    ref="$(git -C "$LLAMA_DIR" symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null || true)"
    [ -n "$ref" ] || ref="origin/master"
    git -C "$LLAMA_DIR" reset --hard "$ref" || die "git reset --hard $ref failed in $LLAMA_DIR"
  else
    log "cloning llama.cpp into $LLAMA_DIR"
    git clone https://github.com/ggml-org/llama.cpp "$LLAMA_DIR" || die "git clone failed"
  fi
}

main() {
  set -euo pipefail

  # `-x` proves the file bit, not that it runs — a half-finished build leaves an
  # executable that dies on a missing shared library. Ask it, as client.sh asks qwen.
  if [ -z "${FORCE:-}" ] && [ -x "$LLAMA_BIN" ] && "$LLAMA_BIN" --version >/dev/null 2>&1; then
    skip "llama-server already built at $LLAMA_BIN"
    return 0
  fi

  # The CUDA toolkit is a PRECONDITION, not a step: installing 3 GB and appending to
  # ~/.bashrc unasked is not this script's business — and setup-desktop's setup-bash.sh
  # would overwrite those appended lines anyway.
  command -v nvcc >/dev/null 2>&1 || die "nvcc not found. Install the CUDA toolkit (>= 12.4) — see README §1. This script will not install a 3 GB toolkit for you."

  local arch
  arch="${CUDA_ARCH:-$(detect_cuda_arch)}"
  log "building llama.cpp for CUDA arch $arch (override with CUDA_ARCH=)"

  sync_source

  cmake -B "$LLAMA_DIR/build" -S "$LLAMA_DIR" \
    -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES="$arch" \
    -DGGML_CUDA_FA_ALL_QUANTS=ON -DLLAMA_CURL=ON
  cmake --build "$LLAMA_DIR/build" --config Release -j"$(nproc)"

  "$LLAMA_BIN" --version
  log "llama.cpp built at $LLAMA_BIN"
}

[ "${1:-}" = "--source-only" ] || main "$@"
