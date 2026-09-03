# `make install-client` / `make install-server` — Design

**Date:** 2026-09-03
**Status:** Design agreed in brainstorming. Not yet built. The `rag-server` retirement it
depends on is **already done** (crate, unit and live references deleted 2026-09-03).
**Topic:** turn this repo's bring-up prose into two Makefile targets, so that `setup-desktop`
can install either half of the harness without re-implementing — or drifting from — what
README §1–§4 describe.

## Why

`setup-desktop` grew a client-half component on 2026-09-03 (`local-harness/`). It works, but it
re-implements README §4 in bash and carries its own copy of `~/.qwen/settings.json` whose
`ep-rag` block duplicates `deploy/qwen/mcp-servers.snippet.json` in this repo. Two copies, two
repos, drifting the moment a port changes.

The server half is worse: it has no automation anywhere. README §1–§2a is a sequence of commands
you retype on a new GPU box, and the `deploy/*.service` files are redacted templates
(`<deployed-rag-path>`) that cannot be copied into place as-is.

`setup-desktop`'s design (`docs/plans/2026-09-03-second-machine-design.md` §3) deliberately
refused to absorb the server tier, on the grounds that it would create a second source of truth.
That refusal was right, and it leaves the automation gap here — where the units, the compose
files and the knowledge already live.

## Decision table

| Decision              | Choice                                   | Rationale                                                                       |
|-----------------------|------------------------------------------|---------------------------------------------------------------------------------|
| Where the logic lives | This repo                                | It already owns the units, compose files and the README                         |
| Interface             | `make install-client` / `install-server` | Symmetric; `setup-desktop` calls one line either way                            |
| Server granularity    | Tiered sub-targets                       | The CUDA build and a 16 GiB pull must be independently re-runnable              |
| Client settings owner | This repo                                | Kills the duplicated `ep-rag` block; the file names *our* MCP port              |
| `rag-server`          | Retired                                  | Closes "retire it, or keep on ice" from `2026-07-12-ep-rag-mcp-design.md` §Fate |
| Restart policy        | Never implicit                           | weebeastie serves live traffic; a deploy must be inert until asked              |
| Unit paths            | `~/.cargo/bin`                           | `cargo clean` can delete a `target/release` binary a unit points at             |

## 1. `make install-client`

Absorbs README §4 verbatim. Four guarded steps, each a no-op on re-run:

1. **Node ≥ 20 check** — fail with a readable message rather than a confusing `npm` error. This
   repo does not install node; that stays `setup-desktop`'s job (Homebrew's, on macOS).
2. **`npm install -g @qwen-code/qwen-code`**, skipped when `command -v qwen` succeeds. Version
   upgrades stay a manual act.
3. **`~/.qwen/settings.json`** from a new single-source asset, `deploy/qwen/settings.json` — the
   full file, absorbing today's `mcp-servers.snippet.json`, which is deleted. Backed up on differ
   as `settings.json.bak-<ts>`, never overwritten silently.
4. **`~/qwen-scratch/notes.txt`** seeded with the artichoke fixture, skipped if present, so the
   documented liveness check works as soon as a tunnel is up.

`setup-desktop` deletes its `local-harness/qwen-settings.json`. Both its Linux and macOS scripts
collapse to: ensure the clone, ensure node, `make -C "$SETUP_HARNESS_DIR" install-client`. The
macOS script stops reading its Linux sibling by relative path, because that asset is gone.

## 2. `make install-server`

Orchestrates four tiers, each independently runnable and individually guarded:

```
install-server: build-llama fetch-model build-rag deploy-units
```

### `build-llama`

README §1, guarded on `~/Programs/llama.cpp/build/bin/llama-server` existing. Clone-or-fetch, so
a re-run cannot hard-fail on an existing directory.

`CMAKE_CUDA_ARCHITECTURES` is **auto-detected** rather than hardcoded:

```sh
nvidia-smi --query-gpu=compute_cap --format=csv,noheader   # "6.1" → 61
```

The README's hardcoded `61` is the single most likely thing to get wrong on a second GPU box, and
it fails *after* a 20-minute compile. An override variable stays for the odd case.

The CUDA toolkit is a **precondition, not a step**: if `nvcc` is absent the target fails pointing
at README §1, rather than installing 3 GB and appending to `~/.bashrc` unasked. (Those two
`echo >> ~/.bashrc` lines in the README are also exactly what `setup-desktop`'s `setup-bash.sh`
would later overwrite — a `bashrc.d/cuda.sh` snippet there is the durable place for that PATH.)

### `fetch-model`

Makes the ~16 GiB GGUF pull explicit. Today it happens implicitly on first `llama-server` start
via `-hf`, so the first service start silently takes as long as a download and looks like a hang.
The target checks `$LLAMA_CACHE` and pulls if absent, so a failure reads as "download failed"
rather than "service won't start".

### `build-rag`

```sh
cargo install --path rag/crates/mcp --locked      # → ~/.cargo/bin/rag-mcp
```

`--locked` so a deploy builds the lockfile's versions rather than silently resolving newer ones.
`cargo install` refuses to overwrite by default, which is the guard for free; `FORCE=1` adds
`--force`.

One binary, not two — `rag-server` is retired (§4).

The other pipeline binaries (`ingest`, `index`, `parse-gate`, `embed-gate`, `fetch`) are not
services and are not on this path. `install-tools` can put them on `PATH` separately; `ingest` in
particular is worth having.

### `deploy-units`

Two systemd units and two compose stacks.

**Render, don't copy.** The units carry a redaction placeholder and hardcoded `/home/filip`
paths — they are templates whether or not they are named that way. `deploy-units` renders them
from variables (user/home, llama.cpp prefix, model spec, ports, binary path) into
`/etc/systemd/system/`, backing up any differing live file as `*.service.bak-<ts>`.

`ExecStart` resolves to `/home/<user>/.cargo/bin/rag-mcp` — an **absolute** path. Do not reach
for systemd's `%h` specifier: in a system unit under `/etc/systemd/system/` it expands to *root's*
home even with `User=` set.

**No restarts.** Writing a unit file changes nothing until a restart, so this target installs,
backs up, `daemon-reload`s and *reports* what changed. Bouncing services is a separate explicit
`restart-server`. A deploy step that silently drops the model server mid-session is not one
anybody runs twice.

**Compose stacks** (`qdrant`, `openwebui`) are `docker compose up -d` — idempotent, and a no-op
when the running config already matches.

## 3. The weebeastie reconciliation

On weebeastie the first `install-server` is a **reconciliation, not an install**:

- **The unit is still called `ep-rag-mcp.service`** and is `active`. The repo renamed it to
  `rag-mcp.service` on 2026-09-03 (`ep-` now means corpus-specific), so the box and the repo
  disagree by design until this deploy lands. `deploy-units` installs the new name; the old unit
  must be `disable --now`d in the same window, or two units will contend for `:8082`.
- Its `ExecStart` is `/home/filip/ep-rag-mcp/ep-rag-mcp` — a flat directory holding a hand-copied
  POC binary. Almost certainly copied there *because* `target/release` is unsafe to point a unit
  at, which is the same conclusion `build-rag` reaches properly.
- `llama-server.service` points its `WorkingDirectory` at the non-git `~/local_coding_harness`.

Both are what the render replaces. Because `deploy-units` never restarts, the live services keep
running on the old paths until an explicit `restart-server` — which is the moment to verify the
freshly-installed binary reads the same env as the POC one.

## 4. Fate of `rag-server` — done

`2026-07-12-ep-rag-mcp-design.md` §*Fate of the existing `rag-server`* left it at "retire it, or
keep on ice". **Retired**, 2026-09-03. It was an OpenAI-compatible RAG face on `:8081`, built and
reviewed (30 tests green) but never deployed end to end; its one purpose was to be Open WebUI's
second model, and what shipped instead was the `/route` filter against `rag-mcp` — the same
goal through the service already running. Keeping a second, unexercised front door to one index
was the worse trade.

Deleted: `rag/crates/rag-server/`, `deploy/rag-server.service`, the `serve` Makefile target, and
the live references in `README.md` and `CLAUDE.md`. The workspace resolves to 10 packages; the
lockfile change was 22 deletions and no version drift. `docs/plans/2026-07-11-*` are kept
untouched as the historical record. `rag-retrieve` and `rag-generate` are unaffected —
`rag-mcp` is built on them.

## 5. The `setup-desktop` side

```sh
# local-harness/setup-local-harness.sh          (client; existing flag)
ensure clone at $SETUP_HARNESS_DIR; source nvm; make -C "$SETUP_HARNESS_DIR" install-client

# local-harness/setup-local-harness-server.sh   (new; SETUP_ENABLE_LOCAL_HARNESS_SERVER=false)
ensure clone at $SETUP_HARNESS_DIR;             make -C "$SETUP_HARNESS_DIR" install-server
```

A second script in the existing component directory, mirroring `i3/`, which already keeps four
scripts in one dir. `run.sh` goes 26 → 27 steps; the server step is **off by default** (exactly
one machine is ever the server) and **Linux-only**, like `synology/`. Gating stays in `run.sh`
only, so both halves remain standalone-runnable — this repo's stated retry mechanism.

## Portability

The Makefile must stay usable on macOS for `install-client`: GNU make 3.81 and bash 3.2. No
associative arrays, no `${var,,}`, no `mapfile`. `install-server` is Linux-only and may assume
more, but should fail cleanly rather than obscurely elsewhere.

## Verification

The test bed is a live server, so order matters:

1. `build-rag` alone — writes only `~/.cargo/bin`, touches nothing live.
2. `deploy-units` — then diff rendered units against live ones and confirm `systemctl is-active`
   is **unchanged** for both services. This proves a deploy is inert.
3. `restart-server` deliberately; watch `rag-mcp` come back on `~/.cargo/bin` instead of the
   POC path. Confirm `/route` still answers and the qwen MCP tool still resolves.
4. Re-run `install-server` end to end — every tier must report skip.
5. `install-client` twice on the laptop; the second pass changes nothing and re-downloads nothing.

## Out of scope

CUDA toolkit installation · index snapshot-migration · the eval harness (recall@k/MRR vs
groundedness) · `install-tools` for the pipeline binaries · any macOS server story.

## References

- `README.md` §1–§2a (server bring-up), §4 (client) — the prose these targets replace
- `docs/plans/2026-07-12-ep-rag-mcp-design.md` §Fate, §Deployment
- `docs/plans/2026-07-08-systemd-llama-server.md` — the unit this mirrors
- `setup-desktop/docs/plans/2026-09-03-second-machine-design.md` §3 — why the server tier is here
