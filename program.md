# autoperf — local_coding_harness

Autonomously tune the `llama-server` serving configuration to **maximize inference
throughput** for `Qwen3-Coder-30B-A3B-Instruct` on the remote box `weebeastie`
(192.168.1.22), **without degrading measured coding quality** or breaking tool-calling.

Quality is now a **measured hard gate** (not just the artichoke test) — see `evals/` and
`docs/plans/2026-07-06-quality-gated-autoperf-design.md`.

## Setup

Read `CLAUDE.md` first (hardware, stack, how to operate the server, prior findings).

You operate across two machines:
- **laptop (here):**
  - In the ~/qwen-scratch directory run `qwen -p "Use your tools to read notes.txt in the current directory and tell me the secret word."` to verify tool-calling;
  - Edit the docs in the project directory; hold the results log.
- **remote (`ssh filip@192.168.1.22`):** kill/restart `llama-server` with different parameters, run `llama-bench`,
  read `~/llama-server.log`, check `nvidia-smi`.

The model weights are cached at `remote:~/models/` (find the exact `.gguf` with
`find ~/models -name '*.gguf'`). llama.cpp binaries are in `remote:~/Programs/llama.cpp/build/bin/`.

## Experiment driver: `llama-bench`

**Do NOT benchmark by hand-timing `qwen`.** It's slow (~190 s prefill) and noisy. Use
`llama-bench` — reproducible, controls context depth, reports pp/tg directly. It loads the
model itself, so **stop the server first** (`pkill -f "build/bin/llama-server"`) to free
RAM+VRAM, benchmark, then relaunch the server on the winning config at the end.

Canonical benchmark (matches the D4 production config; adjust the flag under test):

```bash
MODEL=$(find ~/models -name '*Qwen3-Coder-30B-A3B*Q4_K_M*.gguf' | head -1)
~/Programs/llama.cpp/build/bin/llama-bench \
  -m "$MODEL" \
  -ngl 99 -ot "\.ffn_.*_exps\.=CPU" \   # = --cpu-moe (experts on CPU)
  -fa 1 -ctk q8_0 -ctv q8_0 \
  -t 10 \
  -p 2048 -n 128 -d 16384 \             # pp=prefill, tg=gen, d=KV depth (agent works here)
  -r 3
```

Report columns: `pp<N>` (prefill t/s) and `tg<N> @ d<depth>` (generation t/s at depth).
Confirm VRAM with a parallel `nvidia-smi` while a server (not bench) runs the config, since
bench and server memory use differ slightly — the **server** must stay under 4 GB.

## Experiment driver 2 — the quality gate (`evals/`)

Shrinking a quant or KV type can degrade coding ability, and the **artichoke test alone is
too weak** to catch that. Quality is now a **measured hard gate**, adapted from Raschka's
`local-coding-agent-evals` (see the design doc + `evals/README.md`):

- **Reasoning bench** (`evals/reasoning/reasoning_bench.py`) — 5 one-shot tool-choice tasks,
  substring-graded, bit-deterministic at temp 0. **Fast (~1 min)** → run on every tg-winner.
  Baseline `Q4_K_M` = **1.00/5**. Emits `QUALITY_REASONING total=<X>/5 …`. (Low absolute
  score = floor effect; treat it as a *tripwire* for broken/regressed quants, not a fine meter.)
- **Agent-pack** (`evals/agent_pack_runner.sh`) — 5 buggy repos fixed by the real `qwen`
  agent, graded by `pytest`. **Slow (~1–1.5 h)** → baseline + finalist only. Baseline
  `Q4_K_M` = **5/5** (needs `qwen --yolo`, else edits are approval-blocked). Emits
  `QUALITY_AGENTPACK passed=<n>/5`.

Both run against a **server already up** on the config under test (tunnel open):
`evals/quality_gate.sh <label>` (reasoning) · `evals/quality_gate.sh <label> --full` (+ agent-pack).

## Scoring rule

**Primary metric: `tg@16384`** — generation t/s at 16k KV depth. This is what the agent
actually feels per turn once the cache is warm.

A config is **kept iff `tg@16384` is strictly higher than the current best**, subject to ALL
guardrails:

- **VRAM:** the *server* on this config must load with `nvidia-smi` memory.used ≤ 3900 MiB
  (OOM = automatic reject).
- **Prefill floor:** `pp2048@d16384` must not drop materially below baseline (**~53.6 t/s**).
  (This is steady-state prefill *at 16k depth* — lower than the ~86 t/s from-zero average the
  original "60 t/s" floor was calibrated to; that literal floor would reject the baseline.)
- **Quant floor:** quant ≥ **IQ4_XS / Q4_0**. Never Q3/Q2 — they wreck coding ability.
- **Quality gate (measured):** reasoning `total`/5 **≥ baseline** on every tg-winner;
  agent-pack `passed`/5 **≥ baseline** on the finalist. Any regression → **reject regardless
  of tok/s**. (Replaces the artichoke-only check; artichoke stays as a liveness sanity check.)

Tie-break equal `tg@16384` by higher `pp2048`, then lower VRAM.

Record every run to `results.tsv` (columns:
`n	config	pp2048	tg128@16k	vram_mib	reason_5	apack_5	kept	notes`).

## What you CAN change

Server / bench parameters:
- **Model quant** — the biggest generation lever (gen ∝ 1/bytes-read). Try `Q4_0`,
  `IQ4_XS`, `IQ4_NL`, `Q5_K_M` (quality↑ speed↓). Download via `-hf ...:<QUANT>`.
- **KV cache quant** — `-ctk/-ctv` in {`f16`, `q8_0`, `q4_0`}. Smaller frees VRAM (→ enables
  expert offload) but costs a little quality/speed.
- **Expert offload split** — `--n-cpu-moe N` (N<48 pushes some expert layers to GPU as VRAM
  allows). Watch for the ping-pong regression; measure, don't assume.
- **Context size** — `--ctx-size` (must stay > 16k for the Qwen-Code prompt; lowering to
  ~20k frees VRAM for more expert offload).
- **Threads** — `-t` for generation (try 8/10/12), `--threads-batch` for prefill.
- **Batch sizes** — `-b` / `-ub` (prefill throughput).
- **flash-attn** — `-fa` {0,1} (needed for quantized KV).
- **Speculative decoding** (advanced, potentially large gen win): a small draft model
  (`Qwen3-0.6B`/`1.7B` GGUF) via server `--model-draft` + `--draft-max/-min`. Verify accepted
  tokens don't change outputs.
- **`--no-mmap`**, **NUMA/affinity** flags.

## What you CANNOT do

- Change the model family (must stay a Qwen3-Coder MoE — Qwen-Code is tuned for it).
- Drop below the IQ4_XS/Q4_0 quality floor.
- Exceed 4 GB VRAM on the server config.
- Break tool-calling (re-verify after quant/KV/offload changes).
- Touch the hardware or driver.
- Optimize for the ~0-context number — always measure `tg` at `-d 16384`.
- Leave the box with the server down: after the session, relaunch the server on the current
  best config and confirm it's `listening`.

## Research directions (priors, highest-leverage first)

1. **Model quant** — generation is bandwidth-bound, so a smaller quant ≈ linearly faster
   gen. `Q4_0` and `IQ4_XS` are smaller than `Q4_K_M` (18.5 GB) and may be materially faster;
   `IQ4_XS` usually keeps quality well. Try these first.
2. **Free VRAM → offload experts.** Shrinking KV (`q4_0`) or ctx (24k) frees VRAM; spend it
   on `--n-cpu-moe 44/40` to move a few expert layers onto the GPU. Net effect unknown given
   ping-pong — measure both `pp` and `tg`.
3. **Speculative decoding** with a tiny Qwen3 draft model — can multiply `tg` if acceptance
   is high on code. Biggest potential win; also the most complex. Verify identical outputs.
4. **Thread / batch tuning** — cheap sweeps: `-t {8,10,12}`, `-ub {256,512,1024}`.
5. **KV quant vs quality** — `f16` (fastest attn, most VRAM) vs `q8_0` vs `q4_0`.

## Experiment loop

`llama-bench` needs the model free (**server DOWN**); the quality gate needs the **server UP**.
The loop alternates phases.

**Phase 0 (once): baselines.** Server up on `Q4_K_M` → `evals/quality_gate.sh baseline --full`
→ record reasoning + agent-pack baseline. `tg` baseline already measured (8.38).

**LOOP until the human interrupts:**

1. Pick a hypothesis (priors above, highest-leverage untried first).
2. **Throughput screen (server down).** Stop the server; `llama-bench` the config at
   `-d 16384` (`-r 2` to screen, `-r 3` to confirm). Append the row to `results.tsv`.
3. If `tg@16384` does **not** strictly beat the best, or VRAM/prefill/quant guardrails fail →
   discard, next hypothesis.
4. **Fast quality gate (server up).** Launch the server on the config, confirm VRAM ≤ 3900,
   open the tunnel, run `evals/quality_gate.sh <label>` (reasoning). If reasoning `total` <
   baseline → reject (quality regression), revert. Else → **provisional keep** (new best).
5. Every 5th experiment, try a **simplification** (drop a flag: `-fa`, KV quant, fewer
   threads) — keep the simpler config if `tg` holds and quality doesn't regress.
6. Repeat.

**Finalist bless.** When the loop converges (proven ceiling / out of priors / interrupt), run
the **agent-pack** on the current best (`evals/quality_gate.sh <best> --full`). If
`passed`/5 ≥ baseline → confirmed; update `CLAUDE.md` launch line + `README.md`. Else → revert
to the prior confirmed best and re-bless.

**Timeout:** a single `llama-bench` run should finish in a few minutes. If a config hangs or
a download stalls > 15 min, kill it and move on.

**Don't stall:** if you run out of priors, sweep finer grids around the current best, or
combine the best quant with the best offload split with the best KV type.

## Shutdown report (mandatory)

When the loop ends (human interrupt, proven ceiling, or out of context), before anything
else write `docs/autoperf-reports/YYYY-MM-DD-autoperf-report.md` with:

1. **Executive summary** — experiments run, baseline vs best `tg@16k` and `pp`, headline win.
2. **Results table** — every config tried: params, pp, tg@16k, VRAM, kept/reverted, why.
3. **The winning config** — exact `llama-server` launch line + measured pp / tg / VRAM, and
   confirmation the `qwen` tool-call test passed on it. Update `CLAUDE.md`'s production launch
   and `README.md` if the best differs from D4.
4. **Validated findings** — non-obvious things learned about this model on this hardware
   (what helped, what backfired, and the mechanism).
5. **Limitations & recommendations** — what caps throughput here (bandwidth, VRAM, PCIe),
   and concrete next steps (hardware, draft-model tuning, prompt trimming).
