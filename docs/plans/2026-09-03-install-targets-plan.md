# Harness Install Targets — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the harness bring-up prose with `make install-client` / `make install-server`, so
`setup-desktop` installs either half by calling one line instead of re-implementing it.

**Architecture:** Makefile targets are a thin interface over small bash scripts in `deploy/install/`.
Scripts hold the logic (so it is testable and readable); the Makefile holds the names and the
ordering. Every step is guarded and re-runnable; nothing restarts a live service implicitly.

**Tech Stack:** GNU make, bash, systemd, docker compose, cargo. **No new dependencies** — `bats` and
`shellcheck` are absent on both machines, so tests are plain bash assert scripts run by `make test-install`.

**Design:** `docs/plans/2026-09-03-install-targets-design.md`. Read it first; this plan does not
re-argue the decisions.

---

## Ground truth (verified 2026-09-03, do not re-probe)

| Fact | Value | Consequence |
|---|---|---|
| `nvidia-smi --query-gpu=compute_cap --format=csv,noheader` | `6.1` | strip the dot → `61` |
| `cargo` on weebeastie | installed 1.98.0, **not on non-interactive PATH** | never assume `cargo`; resolve it |
| llama.cpp on weebeastie | `~/Programs/llama.cpp/build/bin/llama-server` exists | `build-llama` must skip |
| GGUF on weebeastie | `~/models/models--unsloth--Qwen3-Coder-30B-A3B-Instruct-GGUF` | `fetch-model` must skip |
| Live MCP unit | `ep-rag-mcp.service` **active**, ExecStart `/home/filip/ep-rag-mcp/ep-rag-mcp` | rename collision, see Task 12 |
| Root Makefile help | parses `.PHONY: name # description` | new targets MUST use that exact form |

**The `cargo` trap.** `ssh host 'make install-server'` runs a non-interactive shell; `~/.bashrc`
returns early at its `case $- in *i*` guard, so `~/.cargo/bin` is not on `PATH`. Resolve cargo
explicitly, in this order: `$CARGO` → `command -v cargo` → `$HOME/.cargo/bin/cargo` → fail with a
message naming all three.

---

## Task 1: Test harness

**Files:**
- Create: `deploy/install/tests/assert.sh`
- Create: `deploy/install/tests/run.sh`
- Modify: `Makefile` (add `test-install` target)

**Step 1: Write the assertion helper**

```bash
# deploy/install/tests/assert.sh — three asserts, no framework. bats is not
# installed on either machine and this is not worth a dependency.
FAILED=0; PASSED=0
assert_eq() { # want, got, label
  if [ "$1" = "$2" ]; then PASSED=$((PASSED+1)); else
    FAILED=$((FAILED+1)); printf 'FAIL %s\n  want: %s\n  got:  %s\n' "$3" "$1" "$2" >&2
  fi
}
assert_contains() { # haystack, needle, label
  case "$1" in *"$2"*) PASSED=$((PASSED+1));; *)
    FAILED=$((FAILED+1)); printf 'FAIL %s\n  %s does not contain %s\n' "$3" "$1" "$2" >&2;; esac
}
assert_status() { # want_rc, label, cmd...
  local want="$1" label="$2"; shift 2
  "$@" >/dev/null 2>&1; local got=$?
  assert_eq "$want" "$got" "$label"
}
report() { printf '%s passed, %s failed\n' "$PASSED" "$FAILED"; [ "$FAILED" -eq 0 ]; }
```

**Step 2: Write the runner**

```bash
#!/usr/bin/env bash
# deploy/install/tests/run.sh — source every test_*.sh and report once.
#
# Two constraints on test files, enforced here:
#   1. Do not call `exit`. Files are sourced, so an `exit` ends this process too:
#      failures print, no tally follows, and make sees success. Use `return`.
#   2. Do not install an `EXIT` trap. Bash does not stack traps, so yours would
#      silently replace the abort trap below and reopen the same hole.
# Both are conventions, not guarantees — running each file in its own subshell
# would enforce them, at the price of the shared counters this harness is built on.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/assert.sh"
# _RUN_LOOP_DONE is set by the runner alone, never by a function a test file can
# reach: `report` is public, and a standalone-runnable test file ending in a call
# to it would otherwise disarm this trap mid-run.
trap '[ -n "${_RUN_LOOP_DONE:-}" ] || { printf "HARNESS ABORTED before report — result is not trustworthy\n" >&2; exit 1; }' EXIT
# The file count is reported alongside the tally, deliberately WITHOUT a minimum
# threshold (Task 1 requires this suite to be green at zero tests). It exists to
# make a vanished test file visible: a sourced script that clobbers HERE makes
# every later test_*.sh resolve `$HERE/../lib.sh` to nothing and source an empty
# file, so its asserts disappear from the tally with no abort and no failure —
# just a smaller green number that nobody is comparing against anything.
_FILES=0
for t in "$HERE"/test_*.sh; do
  [ -e "$t" ] || continue
  echo "--- $(basename "$t")"; . "$t"; _FILES=$((_FILES+1))
done
_RUN_LOOP_DONE=1
printf -- '--- %s file(s)\n' "$_FILES"
report
```

**Step 3: Wire the make target** — append to the root `Makefile`, using the help convention exactly:

```make
.PHONY: test-install # [any] unit-test the install scripts (no install performed)
test-install:
	bash deploy/install/tests/run.sh
```

**Step 4: Verify it runs green with zero tests**

Run: `make test-install`
Expected: `0 passed, 0 failed`, exit 0.

**Step 5: Commit**

```bash
git add deploy/install/tests Makefile
git commit -m "test: dependency-free assert harness for the install scripts"
```

---

## Task 2: `deploy/install/lib.sh` — shared helpers

**Files:**
- Create: `deploy/install/lib.sh`
- Create: `deploy/install/tests/test_lib.sh`

**Step 1: Write the failing tests first**

```bash
# deploy/install/tests/test_lib.sh
. "$HERE/../lib.sh"

# arch_from_compute_cap: the whole point is 6.1 -> 61
assert_eq "61" "$(arch_from_compute_cap "6.1")"      "arch 6.1"
assert_eq "89" "$(arch_from_compute_cap "8.9")"      "arch 8.9"
assert_eq "120" "$(arch_from_compute_cap "12.0")"    "arch 12.0 (two digits)"

# render: @VAR@ substitution, and an unsubstituted placeholder is an ERROR
tmp="$(mktemp)"; printf 'Exec=@BIN@\nUser=@USER_NAME@\n' > "$tmp"
out="$(export USER_NAME=filip BIN=/x/y; render "$tmp")"
assert_contains "$out" "Exec=/x/y"    "render substitutes BIN"
assert_contains "$out" "User=filip"   "render substitutes USER_NAME"

# NOTE: do NOT write `env BIN=/x/y render ...` — env(1) execs a *binary* and
# render is a shell function, so it would fail with 127 for the wrong reason.
# BIN is already exported above; NOT_SET is simply never set, which is the case
# under test.
tmp2="$(mktemp)"; printf 'Exec=@BIN@\nOops=@NOT_SET@\n' > "$tmp2"
export BIN=/x/y
assert_status 1 "render fails on a leftover placeholder" render "$tmp2"
unset BIN

tmp3="$(mktemp)"; printf 'WorkingDirectory=@EMPTY_VAR@\n' > "$tmp3"
export EMPTY_VAR=
assert_status 1 "render fails on a set-but-empty placeholder" render "$tmp3"
unset EMPTY_VAR

rm -f "$tmp" "$tmp2" "$tmp3"
```

**Step 2: Run to verify failure**

Run: `make test-install`
Expected: FAIL — `lib.sh: No such file or directory`.

**Step 3: Implement**

```bash
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
```

**Step 4: Verify tests pass**

Run: `make test-install`
Expected: `29 passed, 0 failed` (Task 2's own asserts).

**`render` was defective as first specified — corrected 2026-09-03.** The original relied solely
on a post-render grep to catch a surviving `@VAR@`. That can never fire: `envsubst` expands an unset
`${VAR}` to the empty string, so the placeholder is silently **deleted**, and the function returned 0
on a template with a missing value — producing `WorkingDirectory=` in a systemd unit. The fix scans
the *source* for `@VAR@` names first and refuses when any is unset **or set-but-empty** (an empty
value yields the identical broken line but is harder to spot). Both checks are load-bearing: the
`<deployed-rag-path>` redaction markers are not `@VAR@`-shaped, so `envsubst` ignores them and only
the post-render grep catches those. `render` requires **bash**, not sh — `${!name}` is indirect
expansion.

`render` uses `envsubst` (package `gettext-base`). **Verified present on both the laptop and
weebeastie** at `/usr/bin/envsubst`, and **neither unit file contains a literal `$`** — which is what
makes the sed-to-`${VAR}`-then-`envsubst` approach safe here. If a future unit needs a literal `$`
(e.g. a systemd specifier in an `Environment=` line), escape it as `$$` or switch to a `while read`
substitution loop.

**Consumer constraint — never redirect `render` straight to a file.** The shell truncates a redirect
target *before* `render` is invoked, so `render tpl > unit` destroys the target on **every** failure
path (verified: a pre-existing 23-byte file becomes 0 bytes), not just the empty-output one. And
`render … > "$tmp" || die …` does **not** save you: in the empty-output case `render`'s own `die`
exits the shell before the `||` branch can run. The only shape that handles both failure modes and
leaves a pre-existing file intact:

```sh
content="$(render "$src")" || die "render failed for $src"
printf '%s\n' "$content" > "$tmp"
```

`render` is a stdout filter and deliberately takes no destination argument — that keeps it testable
through `$(...)`, which the whole of `test_lib.sh` depends on. Tasks 5–8 must use the shape above.

**`render` has mixed failure semantics.** The two placeholder paths `return 1` and are catchable by
`if`/`||`; the empty-output path `die`s and exits the script outright, which neither can catch. That
is deliberate — a missing `envsubst` is unrecoverable — but do not assume every failure is catchable.

**Step 5: Commit**

```bash
git add deploy/install/lib.sh deploy/install/tests/test_lib.sh
git commit -m "feat(install): shared helpers — cargo resolution, arch detect, strict render"
```

---

## Task 3: Single-source the qwen client settings

**Files:**
- Create: `deploy/qwen/settings.json`
- Delete: `deploy/qwen/mcp-servers.snippet.json`
- Delete (other repo): `setup-desktop/local-harness/qwen-settings.json`

**Step 1: Create the merged asset.** It is `setup-desktop`'s file verbatim — that file is already a
superset, carrying the same `ep-rag` block plus the privacy/telemetry keys:

```json
{
  "privacy": { "usageStatisticsEnabled": false },
  "telemetry": { "enabled": false },
  "mcpServers": {
    "ep-rag": {
      "httpUrl": "http://localhost:8082/mcp",
      "description": "EP committee documents — grounded, cited retrieval (search_ep_committee_docs)."
    }
  }
}
```

**Three keys, not six — corrected 2026-09-03 after review of qwen-code's shipped bundle.** The asset
carries only the harness's own contribution. Dropped:

- **`"$version": 4`** — application-owned. The bundle defines `SETTINGS_VERSION = 4` (an older `= 2`
  is still present, so it has been bumped before), writes the key itself, and gates schema migrations
  on it; its docs note that a wrapped entry in an already-migrated `$version: 4` file is *silently
  skipped*. Shipping it pins the client to a stale schema and re-asserts it after every app migration.
- **`ui.autoModeAcknowledged`** — a record that a user dismissed a one-time dialog, not configuration.
- **`permissions`** — its `allow` array would be **replaced, not unioned**, by the `jq` merge in
  Task 4, silently wiping hand-added permissions. Removing it leaves the asset with **no arrays at
  all**, which is what makes that merge safe without special-casing. The lost `Bash(mkdir *)` entry
  is not needed: the documented smoke test uses `read_file`, and `client.sh` creates `~/qwen-scratch`
  itself.

**Step 2: Verify it is valid JSON and the ep-rag block is byte-identical to the snippet's**

```bash
jq -e . deploy/qwen/settings.json >/dev/null && echo "valid json"
jq -S '.mcpServers' deploy/qwen/settings.json > /tmp/a.json
jq -S '.mcpServers' deploy/qwen/mcp-servers.snippet.json > /tmp/b.json
# cmp, NOT `diff ... && echo`. This shell runs behind a filtering hook that masks
# diff's exit status to 0 even when files differ, so a `diff && echo "identical"`
# gate reports success unconditionally. cmp's status comes through intact.
cmp -s /tmp/a.json /tmp/b.json && echo "mcpServers identical — safe to delete the snippet"
```
Expected: `valid json`, then `mcpServers identical`.

**Step 3: Delete the snippet, grep for references**

```bash
git rm deploy/qwen/mcp-servers.snippet.json
grep -rn "mcp-servers.snippet" . --exclude-dir=.git   # expect: only docs/plans/2026-07-* (historical)
```

**Step 4: Commit**

```bash
git add deploy/qwen/settings.json
git commit -m "refactor(qwen): one settings asset, absorbing the mcp-servers snippet"
```

---

## Task 4: `make install-client`

**Files:**
- Create: `deploy/install/client.sh`
- Create: `deploy/install/tests/test_client.sh`
- Modify: `Makefile`

**Step 1: Write the failing test.** Only the pure logic is testable — the node version gate:

```bash
# deploy/install/tests/test_client.sh — client.sh's pure logic and its merge.
#
# Sourced by run.sh: no `exit`, no EXIT trap, no `report` (see run.sh's header).
. "$HERE/../lib.sh"

# Sourcing must define functions and do nothing else. client.sh names its own
# directory variable SCRIPT_DIR precisely so that this line cannot clobber the
# runner's HERE — no save/restore needed here.
. "$HERE/../client.sh" --source-only

# --- node_version_ok: the version gate, the only pure function here -----------
assert_status 0 "node 20 ok"    node_version_ok "v20.11.0"
assert_status 0 "node 26 ok"    node_version_ok "v26.8.1"
assert_status 1 "node 18 fails" node_version_ok "v18.15.0"
assert_status 1 "garbage fails" node_version_ok "not-a-version"

# --- deploy_qwen_settings: merge semantics, against a throwaway HOME ----------
# The function reads $HOME at call time, so overriding it in a subshell reaches
# the whole code path without touching the real ~/.qwen. The subshell also
# contains lib.sh's die(), which would otherwise kill the runner.
_sandbox="$(mktemp -d)"
mkdir -p "$_sandbox/.qwen"
# A user-hand-edited file: one key the asset does not carry, one it contradicts.
printf '{"permissions":{"allow":["Bash(mkdir *)"]},"telemetry":{"enabled":true}}\n' \
  > "$_sandbox/.qwen/settings.json"

( HOME="$_sandbox" deploy_qwen_settings ) >/dev/null 2>&1
assert_eq "Bash(mkdir *)" "$(jq -r '.permissions.allow[0]' "$_sandbox/.qwen/settings.json")" \
  "merge keeps a user-only key"
assert_eq "false" "$(jq -r '.telemetry.enabled' "$_sandbox/.qwen/settings.json")" \
  "merge lets the asset win a conflict"
assert_eq "http://localhost:8082/mcp" "$(jq -r '.mcpServers["ep-rag"].httpUrl' "$_sandbox/.qwen/settings.json")" \
  "merge adds the ep-rag server"
assert_eq "1" "$(ls "$_sandbox/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "changed merge leaves exactly one backup"

# The whole point of merging into a temp and handing THAT to install_file: once
# the merge stops changing anything, a re-run must write nothing at all.
( HOME="$_sandbox" deploy_qwen_settings ) >/dev/null 2>&1
assert_eq "1" "$(ls "$_sandbox/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "re-run is a no-op: no second backup"

rm -rf "$_sandbox"

# The other branch: a fresh machine with no ~/.qwen/settings.json at all. There
# is nothing to merge, so the asset must land verbatim and leave no backup.
_sandbox="$(mktemp -d)"
mkdir -p "$_sandbox/.qwen"
( HOME="$_sandbox" deploy_qwen_settings ) >/dev/null 2>&1
assert_status 0 "fresh install writes the asset verbatim" \
  cmp -s "$REPO/deploy/qwen/settings.json" "$_sandbox/.qwen/settings.json"
assert_eq "0" "$(ls "$_sandbox/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "fresh install leaves no backup"

rm -rf "$_sandbox"; unset _sandbox
```

**Step 2: Run — expect failure** (`client.sh` missing).

**Step 3: Implement**

```bash
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
```

**MERGE, do not replace.** The user hand-edits this file — `~/.qwen/` holds `settings.json.bak-20260708`
(134 B) and `settings.json.bak-mcp-20260712` (234 B), hand-made backups in a format no script here
produces, tracking its growth 134 → 234 → 432 bytes. qwen-code also rewrites the file itself. A
whole-file replace would overwrite both (with a `.bak`, but silently in terms of intent) and, because
the app keeps re-diverging it, would emit a fresh `.bak-<ts>` on *every* run — defeating
`install_file`'s conditional-backup design. The sibling repo already answered this question the same
way for Claude Code: `setup-claude-code.sh` merges with `jq -s '.[0] * .[1]'` and says so in a comment.


**Step 4: Run tests** — expect `47 passed, 0 failed` (29 from Task 2 + 18 here).

**Step 5: Add the target**

```make
.PHONY: install-client # [client] qwen CLI, ~/.qwen/settings.json, smoke fixture
install-client:
	bash deploy/install/client.sh
```

**Step 6: Run it for real on the laptop, twice**

```bash
make install-client && make install-client
```
Expected, second run: `[SKIP] qwen already installed`, `[SKIP] .../settings.json already current`,
`[SKIP] smoke fixture present`. **No downloads on the second pass.**

**Step 7: Commit**

```bash
git add deploy/install/client.sh deploy/install/tests/test_client.sh Makefile
git commit -m "feat(install): make install-client"
```

---

## Live-machine risks — read before Task 8

Tasks 5–8 run against a server that is **currently serving traffic**. These are risks to the machine,
distinct from code quality, carried out of the Tasks 1–4 reviews.

1. **`install_file` is not atomic.** It backs up, then `install`s over the destination in place. A
   `die` between the two leaves a truncated unit plus a `.bak-<ts>` and needs manual recovery.
   Before Task 8 writes to `/etc/systemd/system`, change it to install to a temp file in the same
   directory and `mv` into place — replacement then cannot be observed half-done. (Reassuring but
   currently true only by luck: `foo.service.bak-<ts>` does not end in `.service`, so systemd will
   not load the backups. State that in a comment rather than relying on it.)

2. **Partial application in `deploy-units`' loop.** If a `die` fires midway, the box has some new
   unit files on disk and every old process still running — and an unrelated reboot weeks later
   applies half a configuration. **Render and validate every unit first, then write, then a single
   `daemon-reload`.** Do not interleave render and write per unit.

3. **The `rag-mcp` / `ep-rag-mcp` collision is a Task 8 problem, not a Task 12 one.** The live unit is
   `ep-rag-mcp.service` (active, `ExecStart=/home/filip/ep-rag-mcp/ep-rag-mcp`); the repo asset is now
   `rag-mcp.service`. Installing the latter without retiring the former leaves two units contending
   for `:8082`. The cutover is sequenced in Task 12, but the *file* arrives in Task 8 — make sure
   nothing enables it until the old unit is `disable --now`d.

4. **No dry run.** There is no way to see what a run would change before it changes it, and backups
   accumulate as timestamped siblings under `/etc/systemd/system`. A `DRY_RUN=1` short-circuit in
   `install_file` is cheap and would make the first server-side run inspectable — worth more than any
   test here, because the target is live.

5. **The suite is not hermetic.** The Task 4 e2e asserts stub `qwen` but use the host's `node` and
   `jq`. Fine on these two machines; it would fail for environmental reasons in CI.

**The theme across every defect found in Tasks 1–4: a guard checking the wrong property** —
green-when-aborted, present-when-truncated, exists-when-fit, 0644-when-preserve. That pattern is
about to meet a 16 GiB download and a live systemd target.

## Task 5: `build-llama`

**Files:** Create `deploy/install/build-llama.sh`; modify `Makefile`.

**Step 1: Implement** (logic already unit-tested via `detect_cuda_arch` in Task 2)

```bash
#!/usr/bin/env bash
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$SCRIPT_DIR/lib.sh"

LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
BIN="$LLAMA_DIR/build/bin/llama-server"

# `-x` proves the file bit, not that it runs — a half-finished build leaves an
# executable that dies on a missing shared library. Ask it, as client.sh asks qwen.
if [ -z "${FORCE:-}" ] && [ -x "$BIN" ] && "$BIN" --version >/dev/null 2>&1; then
  skip "llama-server already built at $BIN"; exit 0
fi

command -v nvcc >/dev/null 2>&1 || die "nvcc not found. Install the CUDA toolkit (>= 12.4) — see README §1. This script will not install a 3 GB toolkit for you."

ARCH="${CUDA_ARCH:-$(detect_cuda_arch)}"
log "building llama.cpp for CUDA arch $ARCH (override with CUDA_ARCH=)"

mkdir -p "$(dirname "$LLAMA_DIR")"
if [ -d "$LLAMA_DIR/.git" ]; then
  log "updating existing clone"; git -C "$LLAMA_DIR" fetch --depth 1 origin && git -C "$LLAMA_DIR" reset --hard origin/HEAD
else
  git clone https://github.com/ggml-org/llama.cpp "$LLAMA_DIR"
fi

cmake -B "$LLAMA_DIR/build" -S "$LLAMA_DIR" \
  -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES="$ARCH" \
  -DGGML_CUDA_FA_ALL_QUANTS=ON -DLLAMA_CURL=ON
cmake --build "$LLAMA_DIR/build" --config Release -j"$(nproc)"

"$BIN" --version
log "llama.cpp built"
```

**Step 2: Target**

```make
.PHONY: build-llama # [server] build llama.cpp with CUDA (auto-detects GPU arch)
build-llama:
	bash deploy/install/build-llama.sh
```

**Step 3: Verify the guard on weebeastie without building anything**

```bash
ssh weebeastie 'cd ~/local-harness && make build-llama'
```
Expected: `[SKIP] llama-server already built at /home/filip/Programs/llama.cpp/build/bin/llama-server`,
exit 0, **no compile**.

**Step 4: Verify arch detection in isolation**

```bash
ssh weebeastie 'cd ~/local-harness && bash -c ". deploy/install/lib.sh && detect_cuda_arch"'
```
Expected: `61`

**Step 5: Commit.**

---

## Task 6: `fetch-model`

**Files:** Create `deploy/install/fetch-model.sh`; modify `Makefile`.

```bash
#!/usr/bin/env bash
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$SCRIPT_DIR/lib.sh"

MODEL="${MODEL:-unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS}"
export LLAMA_CACHE="${LLAMA_CACHE:-$HOME/models}"
LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
CACHE_KEY="models--$(printf '%s' "${MODEL%%:*}" | tr '/' '-')"

mkdir -p "$LLAMA_CACHE"
# FITNESS, not existence. An interrupted 16 GiB fetch creates this directory
# immediately and leaves partial/.incomplete blobs behind; a bare [ -d ] then skips
# forever and llama-server fails to load a truncated GGUF. Same defect class as the
# 0-byte notes.txt found in Task 4, on the artifact where interruption is likeliest.
if [ -z "${FORCE:-}" ] \
   && [ -d "$LLAMA_CACHE/$CACHE_KEY" ] \
   && ! find "$LLAMA_CACHE/$CACHE_KEY" -name '*.incomplete' -print -quit | grep -q . \
   && find "$LLAMA_CACHE/$CACHE_KEY" -name '*.gguf' -size +1G -print -quit | grep -q .; then
  skip "model present ($CACHE_KEY)"; exit 0
fi
log "pulling $MODEL into $LLAMA_CACHE (~16 GiB, resumable)"
"$LLAMA_DIR/build/bin/llama-cli" -hf "$MODEL" -n 0 --no-warmup >/dev/null
log "model cached"
```

**Verify on weebeastie:** `ssh weebeastie 'cd ~/local-harness && make fetch-model'`
Expected: `[SKIP] model present (models--unsloth--Qwen3-Coder-30B-A3B-Instruct-GGUF)`. **No download.**

If `llama-cli -n 0` turns out to still generate, substitute
`llama-server --hf-repo ... & sleep until cached; kill` — but check `-n 0` first, it is cleaner.

**Commit.**

---

## Task 7: `build-rag`

**Files:** Create `deploy/install/build-rag.sh`; modify `Makefile`.

```bash
#!/usr/bin/env bash
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$SCRIPT_DIR/lib.sh"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO="$(resolve_cargo)"

BIN="$HOME/.cargo/bin/rag-mcp"
if [ -x "$BIN" ] && [ -z "${FORCE:-}" ]; then skip "rag-mcp already installed at $BIN"; exit 0; fi

log "cargo install rag-mcp (this builds the workspace; candle is slow)"
"$CARGO" install --path "$REPO/rag/crates/mcp" --locked ${FORCE:+--force}
"$BIN" --help >/dev/null 2>&1 || true
log "rag-mcp installed at $BIN"
```

**Verify on weebeastie** — this is the first target that actually does work there:

```bash
ssh weebeastie 'cd ~/local-harness && make build-rag'
ssh weebeastie 'ls -l ~/.cargo/bin/rag-mcp'
```
Expected: a build, then the binary exists. **This proves the cargo-resolution fix** — it runs over
non-interactive ssh where `cargo` is not on `PATH`.

Then re-run: expect `[SKIP] rag-mcp already installed`.

**Commit.**

---

## Task 8: `deploy-units` and `restart-server`

**Files:**
- Modify: `deploy/llama-server.service`, `deploy/rag-mcp.service` → convert to `@VAR@` templates
- Create: `deploy/install/deploy-units.sh`, `deploy/install/restart-server.sh`
- Create: `deploy/install/tests/test_render_units.sh`
- Modify: `Makefile`

**Step 1: Templatise the units.** Replace `/home/filip` and `<deployed-rag-path>` with
`@HOME_DIR@`, `@USER_NAME@`, `@RAG_BIN@`, `@LLAMA_DIR@`, `@MODEL_PATH@`. Keep every other line byte-identical.

**Step 2: Write the failing render test** — this is the one that matters, because a bad render
installs a broken unit:

```bash
# deploy/install/tests/test_render_units.sh
. "$HERE/../lib.sh"
export HOME_DIR=/home/filip USER_NAME=filip \
       RAG_BIN=/home/filip/.cargo/bin/rag-mcp \
       LLAMA_DIR=/home/filip/Programs/llama.cpp \
       MODEL_PATH=/home/filip/models/x.gguf

out="$(render "$HERE/../../rag-mcp.service")"
assert_contains "$out" "ExecStart=/home/filip/.cargo/bin/rag-mcp" "rag-mcp ExecStart rendered"
unset RAG_BIN   # not `env -u` — see the Task 2 note; render is a shell function
assert_status 1 "render refuses a missing var" render "$HERE/../../rag-mcp.service"
export RAG_BIN=/home/filip/.cargo/bin/rag-mcp

out2="$(render "$HERE/../../llama-server.service")"
assert_contains "$out2" "User=filip" "llama-server User rendered"
```

**Step 3: Run — expect FAIL, then templatise until green.**

**Step 4: Implement `deploy-units.sh`**

```bash
#!/usr/bin/env bash
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$SCRIPT_DIR/lib.sh"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"

export USER_NAME="${USER_NAME:-$USER}" HOME_DIR="${HOME_DIR:-$HOME}"
export RAG_BIN="${RAG_BIN:-$HOME/.cargo/bin/rag-mcp}"
export LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
UNITS="${UNITS:-llama-server rag-mcp}"
CHANGED=""

for u in $UNITS; do
  tmp="$(mktemp)"
  # NOT `render ... > "$tmp"` — see the Task 2 consumer constraint: a redirect
  # truncates the target before render runs, and render's die is uncatchable.
  content="$(render "$REPO/deploy/$u.service")" || die "render failed for $u"
  printf '%s\n' "$content" > "$tmp"
  if install_file "$tmp" "/etc/systemd/system/$u.service" sudo; then CHANGED="$CHANGED $u"; fi
  rm -f "$tmp"
done

sudo systemctl daemon-reload
for u in $UNITS; do sudo systemctl enable "$u" >/dev/null 2>&1 || true; done

docker compose -f "$REPO/deploy/qdrant/docker-compose.yml" up -d
docker compose -f "$REPO/deploy/openwebui/docker-compose.yml" up -d

if [ -n "$CHANGED" ]; then
  log "unit files changed:$CHANGED"
  log "NOT restarting. Run 'make restart-server' when you are ready to take the downtime."
else
  skip "all units already current"
fi
```

**Step 5: Implement `restart-server.sh`** — explicit, and it prints the warm-cache cost:

```bash
#!/usr/bin/env bash
# set -euo pipefail goes INSIDE main(), not here — see the conventions note.
# A sourced install script must not leak errexit into the shared test runner.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$SCRIPT_DIR/lib.sh"
UNITS="${UNITS:-llama-server rag-mcp}"
warn "restarting: $UNITS — drops the warm KV cache; the next agent turn re-prefills (~190 s)"
for u in $UNITS; do sudo systemctl restart "$u"; done
sleep 3
for u in $UNITS; do printf '%-16s %s\n' "$u" "$(systemctl is-active "$u")"; done
```

**Step 6: Targets**

```make
.PHONY: deploy-units # [server] render+install systemd units and compose stacks (never restarts)
deploy-units:
	bash deploy/install/deploy-units.sh

.PHONY: restart-server # [server] restart llama-server + rag-mcp (drops the warm KV cache)
restart-server:
	bash deploy/install/restart-server.sh
```

**Step 7: Commit.**

---

## Task 9: The orchestrators

**Files:** Modify `Makefile`.

```make
.PHONY: install-server # [server] build llama.cpp, fetch the model, build+install rag-mcp, deploy units
install-server: build-llama fetch-model build-rag deploy-units
	@echo "install-server complete. Nothing was restarted — 'make restart-server' when ready."

.PHONY: install-tools # [server] put the pipeline binaries (ingest, index, gates) on PATH
install-tools:
	$(shell bash -c '. deploy/install/lib.sh && resolve_cargo') install --path rag/crates/ingest --locked
	# ... index, parse, embed, fetch likewise
```

**Verify ordering without running:** `make -n install-server` — confirm the four run in order.

**Commit.**

---

## Task 10: The `setup-desktop` callers

**Files (other repo, `~/CloudStation/DevOps/setup-desktop`):**
- Modify: `local-harness/setup-local-harness.sh`
- Create: `local-harness/setup-local-harness-server.sh`
- Delete: `local-harness/qwen-settings.json`
- Modify: `config.sh`, `run.sh`, `macos/local-harness/setup-local-harness.sh`, `macos/config.sh`

**Step 1: Rewrite the client script** — it collapses to ensure-clone + make:

```bash
#!/bin/bash
set -e
source "$(dirname "$0")/../lib/common.sh"
source "$(dirname "$0")/../config.sh"

log_info "Setting up local coding harness (client)..."

# run.sh gives each step a fresh non-interactive bash — no nvm, no ~/.bashrc.d.
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"

if [ -d "$SETUP_HARNESS_DIR" ]; then
    log_info "Harness repo present at $SETUP_HARNESS_DIR"
else
    log_info "Cloning harness repo to $SETUP_HARNESS_DIR..."
    git clone git@github.com:fbielejec/local-harness.git "$SETUP_HARNESS_DIR"
fi

# The harness owns what "client install" means; this repo only decides *when*.
make -C "$SETUP_HARNESS_DIR" install-client

log_info "Local coding harness client setup complete"
```

**Step 2: Server script** — same shape, `install-server`, plus a guard that it is not the laptop:

```bash
#!/bin/bash
set -e
source "$(dirname "$0")/../lib/common.sh"
source "$(dirname "$0")/../config.sh"

log_info "Setting up local coding harness (server)..."

if [ -d "$SETUP_HARNESS_DIR" ]; then
    log_info "Harness repo present at $SETUP_HARNESS_DIR"
else
    git clone git@github.com:fbielejec/local-harness.git "$SETUP_HARNESS_DIR"
fi

make -C "$SETUP_HARNESS_DIR" install-server

log_info "Local coding harness server setup complete"
```

**Step 3: `config.sh`** — add below the existing harness flag:

```bash
# The GPU box only. Off by default: exactly one machine serves the harness.
SETUP_ENABLE_LOCAL_HARNESS_SERVER="${SETUP_ENABLE_LOCAL_HARNESS_SERVER:-false}"
```

**Step 4: `run.sh`** — bump `TOTAL=26` to `TOTAL=27` and add after the client step:

```bash
run_step "Setting up local harness server..."  local-harness/setup-local-harness-server.sh  "$SETUP_ENABLE_LOCAL_HARNESS_SERVER"
```

**Step 5: Delete the duplicated asset and fix the macOS twin**

```bash
git rm local-harness/qwen-settings.json
grep -rn "qwen-settings" . --exclude-dir=.git     # expect: no hits after the macOS edit
```

**Step 6: Verify**

```bash
bash -n local-harness/*.sh macos/local-harness/*.sh run.sh config.sh
SETUP_ENABLE_LOCAL_HARNESS_SERVER=true bash -c 'source config.sh; echo $SETUP_ENABLE_LOCAL_HARNESS_SERVER'
```
Expected: clean, then `true`. Run `./run.sh` and confirm the banner lists 27 steps and that
`local_harness_server` appears under `Disabled:` when the flag is false.

**Step 7: Commit in `setup-desktop`.**

---

## Task 11: Documentation

- `README.md` §1–§2a: replace the retyped command blocks with `make install-server` (keep the flag
  table and the *why* — that is the value; delete only the sequences the targets now own).
- `README.md` §4: replace with `make install-client`.
- `CLAUDE.md`: note the two targets under *Operating the server*, and the `cargo`-not-on-PATH trap.
- Set the design doc's **Status:** to `Implemented <date>`.

**Commit.**

---

## Task 12: Run it on weebeastie — the reconciliation

Not a code task. Everything before this was rehearsal. **`ep-rag-mcp.service` is live and serving.**

**Step 1: Deploy without restarting**

```bash
ssh -t weebeastie
cd ~/local-harness && git pull && make install-server
```
Expected: `[SKIP]` for build-llama and fetch-model, a real build for build-rag, unit changes
reported, and **no restart**.

**Step 2: Prove the deploy was inert**

```bash
systemctl is-active ep-rag-mcp llama-server     # both still active, on the OLD paths
diff <(sudo cat /etc/systemd/system/rag-mcp.service) <(cd ~/local-harness && bash -c '...render...')
```

**Step 3: The cutover — the one step with downtime.** The old and new units both bind `:8082`, so
they cannot both run:

```bash
sudo systemctl disable --now ep-rag-mcp        # stop the POC unit FIRST
make restart-server                            # start llama-server + rag-mcp
systemctl is-active rag-mcp                    # expect: active
curl -s localhost:8082/route -H 'Content-Type: application/json' \
  -d '{"message":"What is the deadline under the Youth Guarantee?"}' | head -c 200
```

**Step 4: Prove the client still works end to end** (the MCP tool name did not change, so this must
still pass):

```bash
cd ~/qwen-scratch && qwen -p "Use your tools to read notes.txt and tell me the secret word."
```
Expected: `artichoke`.

**Step 5: Idempotency** — `make install-server` again; every tier reports skip.

**Step 6:** Delete the POC directory `~/ep-rag-mcp/` **only after** a successful cutover, and record
anything surprising in `CLAUDE.md` under *Don't relearn*.

---

## Notes for the implementer

- **Never `git add -A`** in `setup-desktop`; `docs/plans` is gitignored there on purpose.
- **The harness repo is public.** No home IPs, no DDNS hostname. `192.168.1.22` is fine (RFC1918).
- **Do not restart a live service to "check it works".** The whole design turns on deploys being
  inert; Task 12 Step 3 is the only sanctioned downtime.
- **`make` recipes need the `.PHONY: name # description` form** or they vanish from `make help`.
- Test what has logic (render, arch parsing, version gates). Do not write tests that shell out to
  `apt`, `cargo` or `systemctl` — they would test the machine, not the code.
- **`install_file` uses `stat -c '%a'`, which is GNU-only.** Both targets are Linux, so this is fine
  today — but if `lib.sh` is ever reused on the macOS tree it needs `stat -f '%Lp'`. Note the failure
  is quiet: the `|| echo 0644` fallback means BSD `stat` degrades to *widening* a destination's mode
  rather than erroring.
- **Name the script-directory variable `SCRIPT_DIR`, never `HERE`.** The runner (`tests/run.sh`)
  owns `HERE`, and every `test_*.sh` resolves siblings through it. An install script that sets a
  file-scope `HERE` and is then sourced with `--source-only` *clobbers the runner's*, so the next
  test file's `. "$HERE/../lib.sh"` silently resolves to the wrong path — dropping its whole assert
  count to zero while the suite still reports green. Found in Task 4; applies to every script in
  `deploy/install/`.
- **Put `set -euo pipefail` inside `main()`, not at file scope.** A sourced install script otherwise
  turns errexit on for the whole runner, contaminating every test file that follows it in glob order.
- **Test-file constraints (enforced by the harness, learned the hard way in Task 1).** Every
  `test_*.sh` is *sourced* into the runner's own shell, which buys shared counters and costs
  isolation. So, in a test file: **never call `exit`** (it truncates the suite — use `return`);
  **never install an `EXIT` trap** (bash does not stack them, so it would silently replace the
  runner's abort guard); and **never call `report`** (the runner owns the tally). Clean up temp
  files with a trailing `rm -f`, not a trap.
- **Beware `set -e` leaking out of a script under test.** `client.sh` and friends open with
  `set -euo pipefail`; sourcing one turns `errexit` on for the whole runner, after which any
  *expected* nonzero — which is most of what these tests assert — kills the process. `assert_status`
  defends against this by calling in a condition context; do not "simplify" that line.
- **`env VAR=x somefunc` does not work.** `env(1)` execs a binary and these are shell functions, so
  it fails with 127 for a reason unrelated to the test. Use `export`/`unset` around the call.
