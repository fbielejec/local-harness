# Quality-gated autoperf — design (2026-07-06)

## Motivation

The original autoperf loop (`program.md`) maximized `tg@16384` with a quality "floor"
checked only by the one-word **artichoke** tool-call smoke test. That proves the model can
still call a tool and echo a word — it cannot detect a quant that is *faster but subtly worse
at coding*. We adopt Sebastian Raschka's `local-coding-agent-evals` methodology to make
coding quality a **measured hard gate** on top of the throughput objective.

Decision (with the human): keep **`tg@16k` as the primary objective**, add a **hard quality
gate** — reject any config whose measured coding quality drops below the `Q4_K_M` baseline.

## What we measure (`evals/`)

Two signals, different cost/fidelity:

1. **Reasoning bench** — `evals/reasoning/reasoning_bench.py`. 5 one-shot tool-choice tasks
   → our llama.cpp `/v1/chat/completions`, deterministic substring grading (1.0 / 0.5 / 0.0),
   `--repeats 3`. **Fast (~1 min), bit-deterministic at temp 0** (verified: identical scores
   across 3 reps). Baseline `Q4_K_M` = **1.00 / 5** (mean 0.200).
   - *Role:* fast **per-config tripwire**. The absolute score is low (floor effect — on
     several tasks the model makes a defensible-but-not-expected tool choice), so this is not
     a fine quality meter. Its job is to catch a **broken** quant (invalid JSON, nonsense
     tools, catastrophic drop) and any regression below baseline, cheaply, on every candidate.

2. **Agent-pack** — `evals/agent_pack_runner.sh` driving Raschka's `agent-problem-pack` (local
   clone) through the **real `qwen` harness**: 5 buggy workspaces, agent edits files, graded
   by `pytest` pass/fail + git diff. **Slow (~15–20 min/problem on this hardware** — ~190 s
   Qwen-Code prefill + ~8 t/s generation + periodic context-reshuffle re-prefills). Baseline
   `Q4_K_M` = **5 / 5** (requires `qwen --yolo` — headless has no TTY to approve edits).
   - *Role:* **authoritative finalist bless** — the truest "coding ability preserved?" test,
     run only on the baseline and the final winner (not per-config).

## The gate rule

A candidate config is **kept** iff ALL hold:

- **`tg@16384` strictly beats** the current best (primary objective — unchanged), AND
- existing guardrails: server VRAM ≤ 3900 MiB, `pp2048` not materially below baseline, quant
  ≥ IQ4_XS/Q4_0 floor, AND
- **quality does not regress** below the `Q4_K_M` baseline:
  - reasoning `total`/5 ≥ baseline (checked on **every** tg-winning candidate), AND
  - agent-pack `passed`/5 ≥ baseline (checked on the **finalist** before it is committed to
    the server + `CLAUDE.md`).

Any quality regression ⇒ **reject regardless of throughput**.

## Loop structure (server up/down alternation)

`llama-bench` needs the model files free (**server DOWN**); the evals need the **server UP**.
So the loop alternates:

- **Phase 0 (once): baselines.** Server up on `Q4_K_M` → reasoning + agent-pack → record the
  quality baseline. (`tg` baseline already measured with `llama-bench`: **8.38 t/s**.)
- **Per hypothesis:**
  1. **Throughput screen (server down).** Stop server; `llama-bench` the config at `-d 16384`.
     If `tg ≤ best` or a guardrail fails → discard.
  2. **Fast quality gate (server up).** If `tg` beats best: launch server on the config, open
     tunnel, run the reasoning bench. If reasoning `total` < baseline → reject (quality
     regression). Else → **provisional keep**.
  3. **Finalist bless (server up).** When the loop converges on a best config, run the
     agent-pack on it. If `passed`/5 < baseline → reject, revert to prior best. Else →
     **confirmed best**; update `CLAUDE.md` launch line, `README.md`, `results.tsv`.

## Costs on this hardware

| Step | Cost | Cadence |
|------|------|---------|
| Throughput screen (`llama-bench -d 16384 -r 2/3`) | ~10–15 min/config | every candidate |
| Reasoning gate | ~1 min/config | every tg-winner |
| Agent-pack | ~1–1.5 h/run | baseline + finalist only |

## Determinism / repeats

Reasoning bench is bit-deterministic at temp 0 here (`--parallel 1`, single slot) — verified.
Keep `--repeats 3` as a guard for configs that could perturb determinism (different batching).
Agent-pack is single-shot (too slow to repeat); a pass/fail flip on rerun would be investigated.

## Attribution / licensing

Methodology from [`rasbt/local-coding-agent-evals`](https://github.com/rasbt/local-coding-agent-evals),
which carries **no license**. The reasoning bench is our own reimplementation of the *method*
(transport repointed Ollama→`/v1`; grader follows his rubric). The agent-pack is his code, run
from a **local clone** (`~/qwen-scratch/raschka-evals`), **not vendored** into this repo. See
`evals/README.md`.
