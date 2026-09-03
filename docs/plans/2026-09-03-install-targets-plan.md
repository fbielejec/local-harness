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
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/assert.sh"
for t in "$HERE"/test_*.sh; do [ -e "$t" ] || continue; echo "--- $(basename "$t")"; . "$t"; done
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
out="$(USER_NAME=filip BIN=/x/y render "$tmp")"
assert_contains "$out" "Exec=/x/y"    "render substitutes BIN"
assert_contains "$out" "User=filip"   "render substitutes USER_NAME"

tmp2="$(mktemp)"; printf 'Exec=@BIN@\nOops=@NOT_SET@\n' > "$tmp2"
assert_status 1 "render fails on a leftover placeholder" env BIN=/x/y render "$tmp2"
rm -f "$tmp" "$tmp2"
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
die()  { printf '[ERROR] %s\n' "$*" >&2; exit 1; }
skip() { printf '[SKIP] %s\n' "$*"; }

# Resolve cargo without assuming PATH. `ssh host make ...` is non-interactive, so
# ~/.bashrc returns early and ~/.cargo/bin is absent — the single most likely
# failure of this whole component.
resolve_cargo() {
  if [ -n "${CARGO:-}" ] && [ -x "${CARGO}" ]; then printf '%s\n' "$CARGO"; return 0; fi
  if command -v cargo >/dev/null 2>&1;  then command -v cargo; return 0; fi
  if [ -x "$HOME/.cargo/bin/cargo" ];   then printf '%s\n' "$HOME/.cargo/bin/cargo"; return 0; fi
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
render() {
  local src="$1" out line
  [ -f "$src" ] || die "template not found: $src"
  out="$(sed -E 's/@([A-Z_][A-Z0-9_]*)@/${\1}/g' "$src" | envsubst)"
  if printf '%s' "$out" | grep -qE '@[A-Z_][A-Z0-9_]*@|<[a-z-]+>'; then
    line="$(printf '%s' "$out" | grep -nE '@[A-Z_][A-Z0-9_]*@|<[a-z-]+>' | head -1)"
    warn "unsubstituted placeholder: $line"
    return 1
  fi
  printf '%s\n' "$out"
}

# Back up a destination that differs, then write. Mirrors setup-desktop's
# deploy_config: conditional on difference, so re-runs do not litter backups.
install_file() { # src_content_file dest [sudo]
  local src="$1" dest="$2" use_sudo="${3:-}" ts
  ts="$(date +%Y%m%d-%H%M%S)"
  if [ -f "$dest" ] && ! cmp -s "$src" "$dest"; then
    ${use_sudo} cp -a "$dest" "${dest}.bak-${ts}" || die "backup failed: $dest"
    log "backed up $dest -> ${dest}.bak-${ts}"
  elif [ -f "$dest" ]; then
    skip "$dest already current"; return 1
  fi
  ${use_sudo} install -m 0644 "$src" "$dest" || die "install failed: $dest"
  log "installed $dest"; return 0
}
```

**Step 4: Verify tests pass**

Run: `make test-install`
Expected: `6 passed, 0 failed`.

`render` uses `envsubst` (package `gettext-base`). **Verified present on both the laptop and
weebeastie** at `/usr/bin/envsubst`, and **neither unit file contains a literal `$`** — which is what
makes the sed-to-`${VAR}`-then-`envsubst` approach safe here. If a future unit needs a literal `$`
(e.g. a systemd specifier in an `Environment=` line), escape it as `$$` or switch to a `while read`
substitution loop.

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
  "permissions": { "allow": ["Bash(mkdir *)"] },
  "$version": 4,
  "ui": { "autoModeAcknowledged": true },
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

**Step 2: Verify it is valid JSON and the ep-rag block is byte-identical to the snippet's**

```bash
jq -e . deploy/qwen/settings.json >/dev/null && echo "valid json"
jq -S '.mcpServers' deploy/qwen/settings.json > /tmp/a.json
jq -S '.mcpServers' deploy/qwen/mcp-servers.snippet.json > /tmp/b.json
diff /tmp/a.json /tmp/b.json && echo "mcpServers identical — safe to delete the snippet"
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
# deploy/install/tests/test_client.sh
. "$HERE/../lib.sh"
. "$HERE/../client.sh" --source-only     # must define funcs and do nothing else

assert_status 0 "node 20 ok"    node_version_ok "v20.11.0"
assert_status 0 "node 26 ok"    node_version_ok "v26.8.1"
assert_status 1 "node 18 fails" node_version_ok "v18.15.0"
assert_status 1 "garbage fails" node_version_ok "not-a-version"
```

**Step 2: Run — expect failure** (`client.sh` missing).

**Step 3: Implement**

```bash
#!/usr/bin/env bash
# deploy/install/client.sh — README §4 as a script. Idempotent; every step guarded.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/lib.sh"
REPO="$(cd "$HERE/../.." && pwd)"

node_version_ok() { # v20.11.0 -> true if major >= 20
  local v="${1#v}" major="${v%%.*}"
  case "$major" in ''|*[!0-9]*) return 1;; esac
  [ "$major" -ge 20 ]
}

main() {
  command -v node >/dev/null 2>&1 || die "node not found. Install node >= 20 first (setup-desktop's node step, or brew)."
  node_version_ok "$(node --version)" || die "node $(node --version) is too old — qwen-code needs >= 20."

  if command -v qwen >/dev/null 2>&1; then skip "qwen already installed"
  else log "installing @qwen-code/qwen-code"; npm install -g @qwen-code/qwen-code; fi

  mkdir -p "$HOME/.qwen"
  install_file "$REPO/deploy/qwen/settings.json" "$HOME/.qwen/settings.json" || true

  mkdir -p "$HOME/qwen-scratch"
  if [ -f "$HOME/qwen-scratch/notes.txt" ]; then skip "smoke fixture present"
  else printf 'The secret word is: artichoke.\n' > "$HOME/qwen-scratch/notes.txt"; log "seeded ~/qwen-scratch/notes.txt"; fi

  log "client install complete"
}

[ "${1:-}" = "--source-only" ] || main "$@"
```

**Step 4: Run tests** — expect `10 passed, 0 failed`.

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

## Task 5: `build-llama`

**Files:** Create `deploy/install/build-llama.sh`; modify `Makefile`.

**Step 1: Implement** (logic already unit-tested via `detect_cuda_arch` in Task 2)

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$HERE/lib.sh"

LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
BIN="$LLAMA_DIR/build/bin/llama-server"

[ -x "$BIN" ] && [ -z "${FORCE:-}" ] && { skip "llama-server already built at $BIN"; exit 0; }

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
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$HERE/lib.sh"

MODEL="${MODEL:-unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS}"
export LLAMA_CACHE="${LLAMA_CACHE:-$HOME/models}"
LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
CACHE_KEY="models--$(printf '%s' "${MODEL%%:*}" | tr '/' '-')"

mkdir -p "$LLAMA_CACHE"
if [ -d "$LLAMA_CACHE/$CACHE_KEY" ] && [ -z "${FORCE:-}" ]; then
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
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$HERE/lib.sh"
REPO="$(cd "$HERE/../.." && pwd)"
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
assert_status 1 "render refuses a missing var" env -u RAG_BIN render "$HERE/../../rag-mcp.service"

out2="$(render "$HERE/../../llama-server.service")"
assert_contains "$out2" "User=filip" "llama-server User rendered"
```

**Step 3: Run — expect FAIL, then templatise until green.**

**Step 4: Implement `deploy-units.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$HERE/lib.sh"
REPO="$(cd "$HERE/../.." && pwd)"

export USER_NAME="${USER_NAME:-$USER}" HOME_DIR="${HOME_DIR:-$HOME}"
export RAG_BIN="${RAG_BIN:-$HOME/.cargo/bin/rag-mcp}"
export LLAMA_DIR="${LLAMA_DIR:-$HOME/Programs/llama.cpp}"
UNITS="${UNITS:-llama-server rag-mcp}"
CHANGED=""

for u in $UNITS; do
  tmp="$(mktemp)"
  render "$REPO/deploy/$u.service" > "$tmp" || die "render failed for $u"
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
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$HERE/lib.sh"
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
