#!/usr/bin/env bash
# deploy/install/client.sh — README §4 as a script. Idempotent; every step guarded.
#
# `set -euo pipefail` lives INSIDE main(), not at file scope, because
# tests/test_client.sh sources this file with --source-only into the shared
# runner shell. File-scope errexit would leak into every test file sourced after
# this one. Sourcing must define functions and do nothing else: no mkdir, no
# npm, no writes.
#
# SCRIPT_DIR, not HERE: tests/run.sh owns a global named HERE (the tests dir) and
# sources this file into its own shell, so a file-scope HERE here would clobber
# it — every later test_*.sh would resolve `$HERE/../lib.sh` against the wrong
# directory and silently source nothing. Install scripts must not stomp it.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$SCRIPT_DIR/lib.sh"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"

node_version_ok() { # v20.11.0 -> true if major >= 20
  # Two statements, not `local v=... major="${v%%.*}"`. `local` declares every
  # name on the line before assigning any, so in that one-liner $v expands to the
  # fresh, still-unset local rather than the value assigned to its left — major
  # comes out EMPTY and the gate rejects every version, including good ones.
  local v="${1#v}" major
  major="${v%%.*}"
  case "$major" in ''|*[!0-9]*) return 1;; esac
  [ "$major" -ge 20 ]
}

# MERGE, do not replace. The user hand-edits ~/.qwen/settings.json and qwen-code
# rewrites it too, so a whole-file replace would both discard their edits and —
# because the app keeps re-diverging the file — emit a fresh .bak-<ts> on every
# run, defeating install_file's conditional-backup design. Same answer
# setup-desktop's setup-claude-code.sh gives for Claude Code.
#
# jq is a hard dependency here; name the package rather than failing with a bare
# 127, the same courtesy resolve_cargo and render extend for cargo and envsubst.
deploy_qwen_settings() {
  command -v jq >/dev/null 2>&1 || die "jq not found — install it (package: jq)"
  local asset="$REPO/deploy/qwen/settings.json" live="$HOME/.qwen/settings.json" merged
  merged="$(mktemp)"
  if [ -f "$live" ]; then
    # Right operand wins on conflicts, so the asset's keys are authoritative while
    # everything the user added survives. Safe because the asset contains no arrays:
    # `*` replaces arrays rather than unioning them.
    jq -s '.[0] * .[1]' "$live" "$asset" > "$merged" || die "merge failed for $live"
  else
    cp "$asset" "$merged" || die "could not stage $asset"
  fi
  # install_file is content-replace with a backup; handing it the *merged* content
  # keeps its "already current" signal meaningful — once the merge stops changing
  # anything, it correctly reports a skip and writes nothing. Its return of 1 is
  # that signal, not an error, so it must be called in a condition context.
  if install_file "$merged" "$live"; then log "qwen settings updated"; fi
  rm -f "$merged"
}

main() {
  set -euo pipefail

  command -v node >/dev/null 2>&1 || die "node not found. Install node >= 20 first (setup-desktop's node step, or brew)."
  node_version_ok "$(node --version)" || die "node $(node --version) is too old — qwen-code needs >= 20."

  # `qwen --version`, not `command -v qwen`: a partial `npm install -g` leaves the
  # bin shim on PATH pointing at a package that no longer resolves, so `command -v`
  # says installed while every invocation exits 127. Running it covers the missing
  # and the broken case with one guard, and repairs the second.
  if qwen --version >/dev/null 2>&1; then skip "qwen already installed"
  else log "installing @qwen-code/qwen-code"; npm install -g @qwen-code/qwen-code; fi

  mkdir -p "$HOME/.qwen"
  deploy_qwen_settings

  # Grep for the fixture, do not merely test -f. The file exists for exactly one
  # reason — the RAG smoke test asks for the secret word — so "present" must mean
  # "contains the string", not "the path resolves". `printf >` truncates before it
  # writes, so an interrupted run leaves a 0-byte notes.txt that -f happily accepts
  # and -s alone would not explain.
  #
  # APPEND rather than overwrite when the file exists with other content: this is a
  # scratch directory a user may well have put their own notes in, and truncating
  # them to restore a one-line fixture is a worse failure than the one being fixed.
  # Appending restores the property without destroying anything, and is idempotent
  # because the grep matches on the next run.
  local notes="$HOME/qwen-scratch/notes.txt"
  mkdir -p "$HOME/qwen-scratch"
  if [ -f "$notes" ] && grep -q 'artichoke' "$notes"; then
    skip "smoke fixture present"
  elif [ -s "$notes" ]; then
    printf 'The secret word is: artichoke.\n' >> "$notes"
    log "appended the smoke fixture to an existing ~/qwen-scratch/notes.txt"
  else
    printf 'The secret word is: artichoke.\n' > "$notes"
    log "seeded ~/qwen-scratch/notes.txt"
  fi

  log "client install complete"
}

[ "${1:-}" = "--source-only" ] || main "$@"
