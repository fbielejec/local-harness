#!/usr/bin/env bash
# deploy/install/build-rag.sh — build and install the rag-mcp binary.
#
# Into ~/.cargo/bin, NOT target/release: `cargo clean` can delete a binary a
# systemd unit points at, and the live POC unit already hit that (its ExecStart
# was a hand-copied binary in a flat directory, for exactly this reason).
#
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"

RAG_BIN="${RAG_BIN:-$HOME/.cargo/bin/rag-mcp}"

main() {
  set -euo pipefail

  # Resolved, never assumed: `ssh host make install-server` is non-interactive, so
  # ~/.bashrc returns early and ~/.cargo/bin is not on PATH.
  local cargo
  cargo="$(resolve_cargo)"

  # `-x` ALONE, deliberately — do NOT probe this binary the way build-llama probes
  # llama-server. rag-mcp parses no arguments at all: `rag-mcp --help` does not
  # print usage, it BOOTS THE SERVICE — loads the route tree, dials Qdrant, and
  # binds 127.0.0.1:8082, which on the deploy target is the port the live unit is
  # already holding. A liveness probe here would fail for the wrong reason on a
  # healthy box, and start a second server on an idle one.
  #
  # `-x` is the right property for this artifact anyway: cargo installs by atomic
  # rename into ~/.cargo/bin, so unlike a cmake build tree there is no half-written
  # executable to catch. The cost is that a source change does not retrigger a
  # build — that is what FORCE=1 is for.
  if [ -z "${FORCE:-}" ] && [ -x "$RAG_BIN" ]; then
    skip "rag-mcp already installed at $RAG_BIN"
    return 0
  fi

  # --locked: a deploy builds the lockfile's versions rather than silently
  # resolving newer ones. cargo install refuses to overwrite by default, which is
  # the second guard for free; FORCE=1 adds --force.
  log "cargo install rag-mcp (this builds the workspace; candle is slow)"
  "$cargo" install --path "$REPO/rag/crates/mcp" --locked ${FORCE:+--force}

  [ -x "$RAG_BIN" ] || die "cargo install reported success but $RAG_BIN is missing — is CARGO_INSTALL_ROOT set?"
  log "rag-mcp installed at $RAG_BIN"
}

[ "${1:-}" = "--source-only" ] || main "$@"
