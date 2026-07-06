# autoperf report — 2026-07-06

Autonomous tuning of `llama-server` serving config for `Qwen3-Coder-30B-A3B-Instruct`
on `weebeastie` (i9-10850K, 62 GB DDR4, GTX 1050 Ti 4 GB). Primary metric: **generation
throughput `tg@16384`** (t/s at 16k KV depth). Guardrails: server VRAM ≤ 3900 MiB,
prefill not materially below baseline, quality floor ≥ IQ4_XS/Q4_0, tool-calling intact.

_Status: IN PROGRESS — this file is updated as experiments land._

## 1. Executive summary

_(filled at shutdown)_

- Experiments run: TBD
- Baseline `tg@16k` (D4, Q4_K_M): **8.38 t/s** · `pp2048@16k`: **53.58 t/s**
- Best `tg@16k`: TBD
- Headline: TBD

## 2. Results table

Measured with `llama-bench` (`-p 2048 -n 128 -d 16384`), CUDA build 9886 (20a04b220).
`pp`/`tg` columns are `@ d16384`. Full data in `results.tsv`.

| n | config | pp2048 | tg128@16k | VRAM (server) | kept? | notes |
|---|--------|-------:|----------:|--------------:|-------|-------|
| 0 | D4 baseline: Q4_K_M, ngl99 `--cpu-moe`, KV q8_0, t10 | 53.58 | **8.38** | ~2861 MiB* | baseline | reference; r3. *VRAM from D1 (server, 32k ctx) |

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

## 5. Limitations & recommendations

_(what caps throughput here — bandwidth, VRAM, PCIe — and concrete next steps)_

- Remote HF download speed measured ≈ 16 MB/s (128 Mbps) → each new ~16 GB quant costs
  ~17 min to fetch before it can be benched.
