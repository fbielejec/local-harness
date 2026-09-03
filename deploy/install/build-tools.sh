#!/usr/bin/env bash
# deploy/install/build-tools.sh — put the RAG pipeline binaries on PATH.
#
# These are NOT services and are not on the install-server path: nothing systemd
# runs needs them. They are the corpus-maintenance tools you want on the box when
# you next re-ingest, `ingest` above all.
#
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Crate directories under rag/crates, not binary names — they differ.
TOOL_CRATES="${TOOL_CRATES:-ingest index parse embed fetch}"

main() {
  set -euo pipefail
  local cargo crate
  cargo="$(resolve_cargo)"
  for crate in $TOOL_CRATES; do
    [ -d "$REPO/rag/crates/$crate" ] || die "no such crate: rag/crates/$crate"
    log "cargo install $crate"
    # Not guarded on the binary the way build-rag is: these are developer tools
    # rebuilt on purpose, so --force is the sane default rather than an escape
    # hatch. Nothing systemd runs points at them, so replacing one is inert.
    "$cargo" install --path "$REPO/rag/crates/$crate" --locked --force
  done
  log "pipeline tools installed into ${CARGO_INSTALL_ROOT:-$HOME/.cargo}/bin"
}

[ "${1:-}" = "--source-only" ] || main "$@"
