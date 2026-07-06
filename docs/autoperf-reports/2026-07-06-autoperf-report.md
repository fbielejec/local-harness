# autoperf report — 2026-07-06

Autonomous tuning of `llama-server` serving config for `Qwen3-Coder-30B-A3B-Instruct`
on `weebeastie` (i9-10850K, 62 GB DDR4, GTX 1050 Ti 4 GB). Primary metric: **generation
throughput `tg@16384`** (t/s at 16k KV depth). Guardrails: server VRAM ≤ 3900 MiB,
prefill not materially below baseline, quant ≥ IQ4_XS/Q4_0, and — new — a **measured
coding-quality gate** (reasoning + agent-pack ≥ Q4_K_M baseline) replacing the artichoke-only
check. See `evals/` and `docs/plans/2026-07-06-quality-gated-autoperf-design.md`.

_Status: IN PROGRESS — this file is updated as experiments land._

## 1. Executive summary

_(filled at shutdown)_

- Experiments run: TBD (throughput sweep running under the new quality gate)
- Baseline `tg@16k` (D4, Q4_K_M): **8.38 t/s** · `pp2048@16k`: **53.58 t/s** · VRAM **2861 MiB**
- Baseline **coding quality** (Q4_K_M): reasoning **1.00/5**, agent-pack **5/5**
- Best quality-passing `tg@16k`: TBD
- Headline: TBD — the loop now maximizes `tg` subject to a *measured* quality floor, not just
  a one-word tool-call smoke test

## 2. Results table

Measured with `llama-bench` (`-p 2048 -n 128 -d 16384`), CUDA build 9886 (20a04b220).
`pp`/`tg` columns are `@ d16384`. Full data in `results.tsv`.

| n | config | pp2048 | tg128@16k | VRAM | reason | apack | kept? | notes |
|---|--------|-------:|----------:|-----:|:------:|:-----:|-------|-------|
| 0 | D4 baseline: Q4_K_M, ngl99 `--cpu-moe`, KV q8_0, t10 | 53.58 | **8.38** | 2861 MiB | 1.00/5 | **5/5** | baseline | reference; r3; VRAM confirmed on server |

_(rows appended as experiments complete)_

## 3. Winning config

_(filled at shutdown — exact `llama-server` launch line, measured pp/tg/VRAM, qwen artichoke tool-call confirmation)_

## 4. Validated findings

_(non-obvious things learned about this model on this hardware — what helped, what
backfired, and the mechanism)_

- Baseline reproduced with `llama-bench`: `tg128@d16384 = 8.38 ± 0.00 t/s`, `pp2048@d16384
  = 53.58 ± 0.05 t/s` — both extremely stable (±0.00), so screening at `-r 2` is reliable.
- Note: `llama-bench pp2048@d16384` (steady-state prefill at 16k depth) reads ~53.6 t/s,
  materially below CLAUDE.md's "86 t/s" (which is the from-zero average over the whole 16k
  prompt). The program's "60 t/s prefill floor" is interpreted as *don't regress prefill
  below baseline*, since the literal floor would reject the baseline itself.
- **Coding-quality gate built** (`evals/`, methodology adapted from Raschka's
  `local-coding-agent-evals`): a fast reasoning bench (5 tool-choice tasks, substring-graded,
  **bit-deterministic at temp 0**) + an agent-pack (5 buggy repos fixed by the real `qwen`
  agent, graded by `pytest`). Q4_K_M baseline = reasoning **1.00/5**, agent-pack **5/5**. The
  agent-pack is the discriminating signal (high headroom); the reasoning bench is a coarse
  *tripwire* (floor effect — the model often makes a defensible-but-not-expected tool choice,
  e.g. `edit_file` instead of `final_answer`).
- **`qwen --yolo` is mandatory for headless agent evals** (root cause of an initial
  53-turns/0-edits loop): headless `qwen -p` has no TTY to approve tool calls, so write/edit
  tools are silently blocked and the model loops forever. `--yolo` auto-approves; read-only
  tools (the artichoke test) never needed it, which hid the issue. This model also doesn't
  self-terminate → bound it with `--max-session-turns`. Security: `--yolo` auto-executes
  shell/write *unsandboxed* → motivates the `~/.qwen/settings.json` sandbox TODO.

## 5. Limitations & recommendations

_(what caps throughput here — bandwidth, VRAM, PCIe — and concrete next steps)_

- Remote HF download speed measured ≈ 16 MB/s (128 Mbps) → each new ~16 GB quant costs
  ~17 min to fetch before it can be benched.
