# deploy/install/lib.sh — shared helpers. Sourced by every install script and by
# the tests, so it must have NO side effects at source time.

log()  { printf '[INFO] %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*" >&2; }
err()  { printf '[ERROR] %s\n' "$*" >&2; }
die()  { err "$*"; exit 1; }
skip() { printf '[SKIP] %s\n' "$*"; }

# Resolve cargo without assuming PATH. `ssh host make ...` is non-interactive, so
# ~/.bashrc returns early and ~/.cargo/bin is absent — the single most likely
# failure of this whole component.
resolve_cargo() {
  if [ -n "${CARGO:-}" ] && [ -x "${CARGO}" ]; then printf '%s\n' "$CARGO"; return 0; fi
  if command -v cargo >/dev/null 2>&1;  then command -v cargo; return 0; fi
  # ${HOME:-} not $HOME: under `set -u` an unset HOME would abort with
  # "HOME: unbound variable" on the exact path where the die message below is
  # the thing the operator needs to read.
  if [ -x "${HOME:-}/.cargo/bin/cargo" ]; then printf '%s\n' "${HOME:-}/.cargo/bin/cargo"; return 0; fi
  die "cargo not found. Tried \$CARGO, PATH, and ~/.cargo/bin/cargo."
}

# "6.1" -> "61". nvidia-smi reports compute capability dotted; CMake wants it flat.
arch_from_compute_cap() { printf '%s\n' "${1//./}"; }

detect_cuda_arch() {
  command -v nvidia-smi >/dev/null 2>&1 || die "nvidia-smi not found — is the NVIDIA driver installed?"
  local cap; cap="$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d ' ')"
  [ -n "$cap" ] || die "nvidia-smi returned no compute_cap"
  arch_from_compute_cap "$cap"
}

# Render @PLACEHOLDER@ from the environment. Fails if any placeholder survives —
# the redacted units carry <deployed-rag-path>, and silently shipping an
# unsubstituted unit is exactly the failure this must not allow.
#
# FAILURE SEMANTICS ARE MIXED, deliberately. Know which you are catching:
#   RETURNS 1 (catchable with `if` or `||`) — an unset or set-but-empty
#     placeholder, or a surviving <redaction-marker>. These are operator errors:
#     export the variable and re-run.
#   CALLS die(), WHICH EXITS THE WHOLE SCRIPT (`if` and `||` CANNOT catch it) —
#     a missing template, or empty output (usually: envsubst not installed,
#     package gettext-base). Unrecoverable by design: there is no sane way to
#     continue an install without envsubst.
#
# NEVER write `render "$src" > "$dest"`. The shell truncates the redirect target
# BEFORE render is ever invoked, so on EVERY failure path an existing $dest is
# already destroyed — a live 23-byte unit becomes 0 bytes — and no guard inside
# render can prevent that. `render ... > "$f" || die ...` does not fix it either:
# on the die() paths the `||` branch never runs. The sanctioned idiom, safe
# against both failure modes and leaving a pre-existing target intact:
#     content="$(render "$src")" || die "render failed for $src"
#     printf '%s\n' "$content" > "$tmp"
# render deliberately does NOT take a destination argument: returning the
# document on stdout is what makes it testable through $(...).
render() {
  local src="$1" out line name
  [ -f "$src" ] || die "template not found: $src"
  # envsubst expands an UNSET ${VAR} to the empty string, so an unsubstituted
  # @VAR@ can never survive to be caught by the post-render grep below — it is
  # silently DELETED instead. The unset case must therefore be caught in the
  # SOURCE, by name, before substitution runs.
  for name in $(grep -oE '@[A-Z_][A-Z0-9_]*@' "$src" | tr -d '@' | sort -u); do
    # Rejects unset AND set-but-empty — the test `${!name:+x}` makes, split into
    # two branches solely so the diagnostic can say which of the two it was.
    # An empty value renders `WorkingDirectory=` exactly as an unset one does but
    # is harder to spot, so systemd would fail confusingly at start rather than
    # clearly at install. Every placeholder here is a path, user or port; none
    # has a legitimate empty value.
    if [ -z "${!name+x}" ]; then
      err "unsubstituted placeholder: @${name}@ is not set in the environment ($src)"
      return 1
    elif [ -z "${!name:+x}" ]; then
      err "unsubstituted placeholder: @${name}@ is set but EMPTY ($src)"
      return 1
    fi
  done
  out="$(sed -E 's/@([A-Z_][A-Z0-9_]*)@/${\1}/g' "$src" | envsubst)"
  # An empty result passes BOTH guards — they pattern-match on the output and
  # empty output matches neither — so it would return 0 and write a 0-byte unit.
  # That is worse than an unsubstituted one. Most likely cause: no envsubst.
  [ -n "$out" ] || die "render produced no output for $src (is envsubst installed?)"
  # Still needed after substitution: the redaction markers (<deployed-rag-path>)
  # are not @VAR@-shaped, so envsubst leaves them untouched and only this catches
  # them.
  if printf '%s' "$out" | grep -qE '@[A-Z_][A-Z0-9_]*@|<[a-z-]+>'; then
    line="$(printf '%s' "$out" | grep -nE '@[A-Z_][A-Z0-9_]*@|<[a-z-]+>' | head -1)"
    err "unsubstituted placeholder: $line"
    return 1
  fi
  printf '%s\n' "$out"
}

# Back up a destination that differs, then write. Mirrors setup-desktop's
# deploy_config: conditional on difference, so re-runs do not litter backups.
#
# RETURNS 1 WHEN NOTHING WAS WRITTEN (destination already current) — that is a
# SIGNAL, NOT AN ERROR. Genuine failures die() instead, so install_file never
# *returns* nonzero for an error: 1 unambiguously means "nothing to do".
# Consumers run under `set -euo pipefail`, so A BARE CALL WILL ABORT THE SCRIPT
# on the happy path of every re-run — the idempotent case. Always write:
#     if install_file "$src" "$dest"; then systemctl daemon-reload; fi
#   or  install_file "$src" "$dest" && changed=1
#
# MODE: an EXISTING destination keeps the mode it already has; a NEW one is
# created 0644. Not "always 0644" — that silently WIDENS a 0600 file, and this
# helper installs systemd units, where one carrying an inline credential or
# pointing at an EnvironmentFile would be exposed by the widening. Not "always
# preserve" either: a fresh systemd unit must land 0644, and there is no
# existing mode to preserve on first install.
install_file() { # src_content_file dest [sudo]
  local src="$1" dest="$2" use_sudo="${3:-}" ts mode
  ts="$(date +%Y%m%d-%H%M%S)"
  # Read the mode BEFORE anything writes to $dest. `|| echo 0644` also covers a
  # stat that cannot read the destination, so the fresh-install default is the
  # fallback for every uncertain case rather than an empty -m argument.
  mode=0644
  [ -e "$dest" ] && mode="$(stat -c '%a' "$dest" 2>/dev/null || echo 0644)"
  if [ -f "$dest" ] && ! cmp -s "$src" "$dest"; then
    ${use_sudo} cp -a "$dest" "${dest}.bak-${ts}" || die "backup failed: $dest"
    log "backed up $dest -> ${dest}.bak-${ts}"
  elif [ -f "$dest" ]; then
    skip "$dest already current"; return 1
  fi
  ${use_sudo} install -m "$mode" "$src" "$dest" || die "install failed: $dest"
  log "installed $dest"; return 0
}
