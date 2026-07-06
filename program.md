# autoperf — local_coding_harness

Autonomously tune the `llama-server` serving configuration to **maximize inference
throughput** for `Qwen3-Coder-30B-A3B-Instruct` on the remote box `weebeastie`
(192.168.1.22), without degrading model quality or breaking tool-calling.

## Setup

Read `CLAUDE.md` first (hardware, stack, how to operate the server, prior findings).

You operate across two machines:
- **laptop (here):** run `qwen` to verify tool-calling; edit these docs; hold the results log.
- **remote (`ssh filip@192.168.1.22`):** kill/restart `llama-server`, run `llama-bench`,
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

## Scoring rule

**Primary metric: `tg@16384`** — generation t/s at 16k KV depth. This is what the agent
actually feels per turn once the cache is warm.

A config **beats baseline iff `tg@16384` is strictly higher**, subject to ALL guardrails:

- **VRAM:** the *server* launched on this config must load with `nvidia-smi` memory.used
  ≤ 3900 MiB (leave headroom; OOM = automatic reject).
- **Prefill floor:** `pp2048` must not drop below **60 t/s** (first-turn latency guard).
- **Quality floor:** model quant ≥ **IQ4_XS / Q4_0** equivalent. Never go to Q3/Q2 — they
  wreck coding ability. Any quant change is a *provisional* win until human-verified.
- **Tool-calling intact:** after keeping any config that changes quant, KV type, or the
  offload split, relaunch the server + tunnel and run the `qwen` artichoke test
  (`CLAUDE.md`). It must still make a `read_file` call and answer "artichoke". If it breaks,
  revert regardless of tok/s.

Tie-break equal `tg@16384` by higher `pp2048`, then lower VRAM.

Record every run (win or loss) to `results.tsv` (create it; columns:
`n	config	pp2048	tg128@16k	vram_mib	kept	notes`).

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

LOOP until the human interrupts:

1. Pick a hypothesis (priors above, highest-leverage untried first).
2. Stop the server. Run `llama-bench` with the config under test (`-r 3` for stable numbers).
3. Append the row to `results.tsv`.
4. If `tg@16384` strictly beats the current best AND all guardrails hold → provisional keep.
   Else → discard, next hypothesis.
5. On a provisional keep that changed quant / KV / offload: launch the **server** on it,
   check VRAM ≤ 3900 MiB, open the tunnel, run the `qwen` artichoke test. Pass → confirmed
   best. Fail (VRAM/quality/tool-calling) → revert to prior best.
6. Every 5th experiment, try a **simplification** (remove a flag: drop `-fa`, drop KV quant,
   fewer threads) — if tok/s holds, keep the simpler config.
7. Repeat.

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
