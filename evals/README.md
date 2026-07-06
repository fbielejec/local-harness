# evals — coding-quality gate for autoperf

A lightweight, deterministic check that **coding ability is preserved** when we change the
serving config (quant, KV type, offload split, …). This is the measured replacement for the
one-word "artichoke" tool-call smoke test.

**Methodology** is adapted from Sebastian Raschka's
[`rasbt/local-coding-agent-evals`](https://github.com/rasbt/local-coding-agent-evals)
(the *hard-tool-reasoning-benchmark* and *agent-problem-pack*). That repo carries **no
license**, so:
- The **reasoning bench** here is our own reimplementation of the *method* (transport
  repointed from Ollama → our llama.cpp OpenAI `/v1` endpoint; grading follows his rubric).
- The **agent-pack** is his code, used from a **local clone** (not vendored here):
  ```bash
  git clone https://github.com/rasbt/local-coding-agent-evals ~/qwen-scratch/raschka-evals
  ```

## Two layers

| Layer | Script | What it does | Cost | Role |
|-------|--------|--------------|------|------|
| Reasoning | `reasoning/reasoning_bench.py` | 5 one-shot tool-choice tasks → `/v1/chat/completions`, substring-graded 1.0/0.5/0.0 | ~30–60 s | **per-config gate** |
| Agent-pack | `agent_pack_runner.sh` | 5 buggy workspaces fixed by the real `qwen` agent, graded by `pytest` | ~10–20 min | **bless the finalist** |

Grading targets and tasks mirror Raschka's. Deviation: we add `read_file` to the reasoning
tool catalog (his omits it, but task 04 requires it — adding it makes all 5 winnable, which
is what a *relative* quality gate needs).

## Running

Both layers run against a **server that is already up** on the config under test (with the
SSH tunnel open). They do not start/stop the server.

```bash
# fast per-config gate (reasoning only)
evals/quality_gate.sh <label>

# full gate (reasoning + agent-pack) — for the finalist and the baseline
evals/quality_gate.sh <label> --full
```

Env (defaults shown): `OPENAI_BASE_URL=http://localhost:8080/v1`,
`OPENAI_MODEL=unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M`, `OPENAI_API_KEY=dummy`,
`REASONING_REPEATS=3`, `AGENT_PACK_DIR=~/qwen-scratch/raschka-evals/agent-problem-pack`,
`QWEN_TIMEOUT=900`, `QWEN_MAX_TURNS=25`.

### Gotchas (learned the hard way)

- **`qwen --yolo` is mandatory for the agent-pack.** Headless `qwen -p` has no TTY to approve
  tool calls, so **write/edit tools are silently blocked** — the model then loops forever
  making edits that never land (observed: 53 turns, 0 file changes). `--yolo` auto-approves.
  Read-only tools (the artichoke test) work without it, which is why this wasn't obvious.
  ⚠️ `--yolo` auto-executes shell/write **unsandboxed** — see the sandbox TODO in `CLAUDE.md`.
- **This model doesn't cleanly self-terminate**, so the runner caps the loop with
  `--max-session-turns`. Grading is via `pytest` in `capture` (independent of qwen's exit), so
  the cap doesn't bias the verdict; it just bounds wall-clock.

## Gate rule (see `../program.md`)

A config is kept only if `tg@16384` **strictly beats** baseline AND the quality numbers do
**not regress** below the Q4_K_M baseline: reasoning `total`/5 ≥ baseline, and (finalist)
agent-pack `passed`/5 ≥ baseline. Any quality regression → reject regardless of throughput.

CSV output and per-run artifacts land in `results/` (gitignored).
