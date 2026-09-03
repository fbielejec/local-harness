#!/usr/bin/env bash
# deploy/install/deploy-units.sh — render the systemd units from the repo templates,
# install them, bring the compose stacks up. NEVER restarts anything: writing a unit
# file changes nothing until a restart, so this target installs, backs up,
# daemon-reloads and REPORTS. Bouncing services is `make restart-server`.
#
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"

UNITS="${UNITS:-llama-server rag-mcp}"
SYSTEMD_DIR="${SYSTEMD_DIR:-/etc/systemd/system}"
SUDO="${SUDO-sudo}"
MODEL="${MODEL:-unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS}"

# Units this deploy REPLACES, as `new:old` pairs. The live box still runs
# ep-rag-mcp.service (the POC unit, ExecStart in a flat hand-copied directory);
# the repo renamed it to rag-mcp on 2026-09-03 because `ep-` now means
# corpus-specific. Both bind 127.0.0.1:8082, so they must never both be enabled —
# `enable` alone would not start the new one now, but a reboot would start both.
# Delete this table once the cutover is done and ~/ep-rag-mcp/ is gone — or
# retire the guard for one run with SUPERSEDES= (a bare `-` default, not `:-`,
# so an empty value means "no supersessions" rather than "give me the default").
SUPERSEDES="${SUPERSEDES-rag-mcp:ep-rag-mcp}"

superseded_by() { # unit -> the legacy unit name it replaces, or nothing
  local pair
  for pair in $SUPERSEDES; do
    [ "${pair%%:*}" = "$1" ] && { printf '%s\n' "${pair#*:}"; return 0; }
  done
  return 1
}

main() {
  set -euo pipefail

  export USER_NAME="${USER_NAME:-$(id -un)}"
  export GROUP_NAME="${GROUP_NAME:-$(id -gn)}"
  export RAG_BIN="${RAG_BIN:-$HOME/.cargo/bin/rag-mcp}"
  export RAG_WORKDIR="${RAG_WORKDIR:-$REPO/rag}"
  export LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
  # Resolved from the cache rather than hardcoded: the snapshot hash in the path
  # changes whenever the repo publishes a new revision, and a stale one makes
  # llama-server fail at start with a file-not-found that reads like a bad build.
  export MODEL_PATH="${MODEL_PATH:-$(resolve_model_path "${LLAMA_CACHE:-$HOME/models}/$(hf_cache_key "$MODEL")" "${MODEL##*:}")}"

  log "rendering with USER_NAME=$USER_NAME RAG_WORKDIR=$RAG_WORKDIR"
  log "  RAG_BIN=$RAG_BIN"
  log "  MODEL_PATH=$MODEL_PATH"

  # ── Phase 1: render and validate EVERY unit before writing ANY of them ──────
  # Not one render-then-write per unit. A die() midway through an interleaved loop
  # leaves the box with some new unit files on disk and every old process still
  # running — and an unrelated reboot weeks later applies half a configuration.
  local staging u tmp
  staging="$(mktemp -d)"
  for u in $UNITS; do
    tmp="$staging/$u.service"
    # NOT `render ... > "$tmp"`: the shell truncates a redirect target before
    # render runs, and render's die() paths cannot be caught by `||`.
    local content
    content="$(render "$REPO/deploy/$u.service")" || die "render failed for $u"
    printf '%s\n' "$content" > "$tmp"
  done

  # Named *.service in a temp dir precisely so systemd-analyze will parse them.
  # This catches a mistyped directive, and an ExecStart that does not exist,
  # BEFORE anything reaches /etc — which is the whole point of a staging phase.
  if [ -z "${SKIP_VERIFY:-}" ] && command -v systemd-analyze >/dev/null 2>&1; then
    for u in $UNITS; do
      systemd-analyze verify "$staging/$u.service" \
        || die "systemd-analyze rejected the rendered $u.service (nothing was installed). Most likely its ExecStart does not exist yet — run 'make build-llama build-rag' first, or re-run with SKIP_VERIFY=1."
    done
    log "rendered units pass systemd-analyze verify"
  fi

  # ── Phase 2: write ─────────────────────────────────────────────────────────
  local changed=""
  for u in $UNITS; do
    if install_file "$staging/$u.service" "$SYSTEMD_DIR/$u.service" "$SUDO"; then
      changed="$changed $u"
    fi
  done
  rm -rf "$staging"

  if [ -n "${DRY_RUN:-}" ]; then
    log "DRY_RUN: no daemon-reload, no enable, no compose. Nothing was changed."
    return 0
  fi

  # A redirected SYSTEMD_DIR means this is a rehearsal, so the phases that talk to
  # the REAL systemd must not run: `daemon-reload` would reload a manager that
  # never saw these files, and `enable` would either fail on a unit it cannot find
  # or — worse — succeed against a same-named unit already in /etc. Deriving this
  # from SYSTEMD_DIR rather than adding a flag keeps the two impossible to
  # desynchronise.
  if [ "$SYSTEMD_DIR" != "/etc/systemd/system" ]; then
    log "SYSTEMD_DIR is $SYSTEMD_DIR, not /etc/systemd/system — staging only."
    log "  Skipped: daemon-reload, enable, and the compose stacks."
    return 0
  fi

  # ── Phase 3: one reload, then enable where it is safe to ───────────────────
  if [ -n "$changed" ]; then
    $SUDO systemctl daemon-reload
    log "unit files changed:$changed"
  fi

  for u in $UNITS; do
    local old
    if old="$(superseded_by "$u")" \
       && systemctl list-unit-files "$old.service" >/dev/null 2>&1 \
       && { systemctl is-enabled "$old" >/dev/null 2>&1 || systemctl is-active "$old" >/dev/null 2>&1; }; then
      warn "NOT enabling $u: $old is still enabled/active and both bind the same port."
      warn "  Cut over deliberately:  sudo systemctl disable --now $old && make restart-server"
      continue
    fi
    $SUDO systemctl enable "$u" >/dev/null 2>&1 || warn "could not enable $u"
  done

  # ── Compose stacks. Idempotent, and a no-op when the running config matches. ─
  #
  # A stack failure WARNS, it does not abort. The units are what this target
  # exists to install, and they are already on disk by now; taking the whole
  # deploy down because a chat frontend could not start would be the wrong trade.
  # The expected failure is concrete: deploy/openwebui/.env holds WEBUI_SECRET_KEY
  # and is gitignored, so it is absent on every machine but the one that made it —
  # which is exactly the second-machine case this repo keeps meeting.
  local failed=""
  if command -v docker >/dev/null 2>&1; then
    for stack in qdrant openwebui; do
      docker compose -f "$REPO/deploy/$stack/docker-compose.yml" up -d || failed="$failed $stack"
    done
  else
    warn "docker not found — skipping the qdrant and openwebui stacks"
  fi
  if [ -n "$failed" ]; then
    warn "compose stack(s) did not come up:$failed"
    warn "  The units above ARE installed. For openwebui the usual cause is a missing"
    warn "  deploy/openwebui/.env — copy .env.example and set WEBUI_SECRET_KEY."
  fi

  if [ -n "$changed" ]; then
    log "NOT restarting. Run 'make restart-server' when you are ready to take the downtime."
  else
    skip "all units already current"
  fi
}

[ "${1:-}" = "--source-only" ] || main "$@"
