#!/usr/bin/env bash
# deploy/install/restart-server.sh — the ONLY sanctioned downtime. Separate from
# deploy-units on purpose: a deploy step that silently drops the model server
# mid-session is not one anybody runs twice.
#
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"

UNITS="${UNITS:-llama-server rag-mcp}"
SUDO="${SUDO-sudo}"
SUPERSEDES="${SUPERSEDES-rag-mcp:ep-rag-mcp}"

main() {
  set -euo pipefail

  # Starting rag-mcp while the POC unit still holds 127.0.0.1:8082 gives a unit
  # that fails, retries every 3 s and never comes up — with the *old* service
  # still answering, so the box looks healthy and the deploy looks done.
  local pair new old
  for pair in $SUPERSEDES; do
    new="${pair%%:*}"; old="${pair#*:}"
    case " $UNITS " in *" $new "*) ;; *) continue;; esac
    if systemctl is-active "$old" >/dev/null 2>&1; then
      die "$old is still active and binds the same port as $new. Cut over first:
  sudo systemctl disable --now $old
then re-run 'make restart-server'."
    fi
  done

  warn "restarting: $UNITS — drops the warm KV cache; the next agent turn re-prefills (~190 s)"
  for u in $UNITS; do $SUDO systemctl restart "$u"; done
  sleep 3
  local rc=0
  for u in $UNITS; do
    printf '%-16s %s\n' "$u" "$(systemctl is-active "$u" 2>&1)"
    systemctl is-active --quiet "$u" || rc=1
  done
  # A restart that left something dead must not exit 0: `make install-server &&
  # make restart-server` would otherwise read as a clean deploy.
  [ "$rc" -eq 0 ] || die "at least one unit is not active — check 'journalctl -u <unit> -n 50'"
}

[ "${1:-}" = "--source-only" ] || main "$@"
