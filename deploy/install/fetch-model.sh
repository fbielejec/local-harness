#!/usr/bin/env bash
# deploy/install/fetch-model.sh — make the ~16 GiB GGUF pull an explicit step.
#
# Today it happens implicitly on the first `llama-server` start via -hf, so the
# first service start silently takes as long as a download and looks like a hang.
#
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"

MODEL="${MODEL:-unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS}"
LLAMA_CACHE="${LLAMA_CACHE:-$HOME/models}"
LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"

# hf_cache_key lives in lib.sh: deploy-units.sh needs the same mapping to find
# the GGUF it writes into the llama-server unit.
# FITNESS, not existence. An interrupted 16 GiB fetch creates this directory
# immediately and leaves partial blobs behind; a bare [ -d ] then skips forever and
# llama-server fails to load a truncated GGUF. Same defect class as the 0-byte
# notes.txt found in Task 4, on the artifact where interruption is likeliest.
#
# `find -L`, NOT `find`. The HF cache stores the weights in blobs/ and puts a
# SYMLINK in snapshots/; without -L, find stats the link itself, `-size +1G` never
# matches a fully-downloaded model, and the guard re-downloads it. Verified against
# the real cache on weebeastie, where the only .gguf is such a symlink.
model_present() { # cache_dir
  local dir="$1"
  [ -d "$dir" ] || return 1
  find "$dir" \( -name '*.incomplete' -o -name '*.downloadInProgress' -o -name '*.part' \) \
       -print -quit 2>/dev/null | grep -q . && return 1
  find -L "$dir" -name '*.gguf' -size +1G -print -quit 2>/dev/null | grep -q .
}

main() {
  set -euo pipefail
  export LLAMA_CACHE

  local key dir
  key="$(hf_cache_key "$MODEL")"
  dir="$LLAMA_CACHE/$key"
  mkdir -p "$LLAMA_CACHE"

  if [ -z "${FORCE:-}" ] && model_present "$dir"; then
    skip "model present ($key)"
    return 0
  fi

  local cli="$LLAMA_DIR/build/bin/llama-cli"
  [ -x "$cli" ] || die "$cli not found — run 'make build-llama' first."

  log "pulling $MODEL into $LLAMA_CACHE (~16 GiB, resumable)"
  "$cli" -hf "$MODEL" -n 0 --no-warmup >/dev/null

  model_present "$dir" || die "fetch finished but $dir still has no usable .gguf — check the download."
  log "model cached at $dir"
}

[ "${1:-}" = "--source-only" ] || main "$@"
