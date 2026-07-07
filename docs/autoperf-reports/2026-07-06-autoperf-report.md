# autoperf report — 2026-07-06 → 07

Autonomous throughput tuning of `llama-server` for `Qwen3-Coder-30B-A3B-Instruct` on
`weebeastie` (i9-10850K, 62 GB DDR4 ~45 GB/s, GTX 1050 Ti 4 GB), run under a **measured
coding-quality gate** (added mid-run at the human's request). Primary metric: generation
throughput **`tg@16384`**. Guardrails: server VRAM ≤ 3900 MiB, prefill not below baseline,
quant ≥ IQ4_XS/Q4_0, and coding quality ≥ Q4_K_M baseline (reasoning + agent-pack, `evals/`).

## 1. Executive summary

- **Headline:** switching the served quant **Q4_K_M → IQ4_XS** raises generation throughput
  **+5.0 %** (`tg@16k` 8.38 → **8.80 t/s**) at **identical measured coding quality**
  (agent-pack **5/5** on both) and **lower VRAM** (2861 → **2777 MiB**). IQ4_XS is now the
  production config.
- **Every other throughput lever tested gave no gain** (all within ~±0.5 % noise): KV-cache
  quant, partial expert offload, thread count, and speculative decoding. The generation
  bottleneck is the **irreducible per-token CPU expert read** over ~45 GB/s RAM — not KV-read
  bandwidth, not PCIe, not thread count.
- **New capability:** a measured coding-quality gate (`evals/`, methodology adapted from
  Sebastian Raschka's `local-coding-agent-evals`) — a fast, temp-0-deterministic reasoning
  bench + a real `qwen` agent-pack graded by `pytest` — replaces the one-word artichoke smoke
  test. It is what let us *prove* the quant change preserves coding ability.

| metric | baseline Q4_K_M ("D4") | **best: IQ4_XS** |
|---|---|---|
| `tg@16k` | 8.38 t/s | **8.80 t/s (+5.0 %)** |
| `pp2048@16k` | 53.58 t/s | 54.95 t/s |
| server VRAM | 2861 MiB | 2777 MiB |
| reasoning gate | 1.00/5 | 1.50/5 |
| agent-pack gate | 5/5 | 5/5 |
| size | 17.28 GiB | 15.25 GiB |

## 2. Results table

Measured with `llama-bench -p 2048 -n 128 -d 16384` (CUDA build 9886, `20a04b220`); `pp`/`tg`
are `@ d16384`. Quality via `evals/` (reasoning `total`/5 · agent-pack `pass`/5). Raw in
`results.tsv`.

| n | config | pp | tg@16k | VRAM | reason | apack | kept | note |
|---|--------|---:|-------:|-----:|:------:|:-----:|------|------|
| 0 | Q4_K_M `--cpu-moe` KV q8_0 t10 (D4) | 53.58 | 8.38 | 2861 | 1.00/5 | 5/5 | baseline | reference |
| 1 | Q4_K_M + KV **q4_0** | 53.90 | 8.40 | – | – | – | no | tie — KV type doesn't move tg |
| 2 | **IQ4_XS** `--cpu-moe` KV q8_0 | 54.95 | **8.80** | 2777 | 1.50/5 | 5/5 | **BEST** | +5 %, quality intact |
| 3 | Q4_0 `--cpu-moe` KV q8_0 | 54.96 | 8.77 | – | – | – | no | +4.7 % but dominated by IQ4_XS |
| 4 | IQ4_XS `--n-cpu-moe 46` (offload 2) | 55.29 | 8.81 | – | – | – | no | offload no gain |
| 5 | IQ4_XS `--n-cpu-moe 44` (offload 4) | 55.51 | 8.85 | – | – | – | no | offload no gain |
| 6 | IQ4_XS t20 | 55.17 | 8.82 | – | – | – | no | more threads no gain |
| 7 | IQ4_XS + spec-decode (Qwen3-0.6B) | – | 8.76 | 2777 | – | – | no | spec no gain (MoE) |

IQ4_XS re-measured 8.80–8.86 across runs (~0.7 % run-to-run variance). `-r 2` screening is
safe: within a run every number was ±0.00.

## 3. Winning config

Production `llama-server` launch (`weebeastie`, `export LLAMA_CACHE=$HOME/models`):

```bash
llama-server -hf unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS \
  --host 127.0.0.1 --port 8080 --threads 10 --parallel 1 --ctx-size 32768 \
  --n-gpu-layers 99 --cpu-moe -fa on --cache-type-k q8_0 --cache-type-v q8_0 \
  --no-mmap --jinja
```

Measured on this config: `pp2048@d16384` **54.95 t/s** · `tg128@d16384` **8.80 t/s** · server
VRAM **2777 MiB** (loads ~17 s). Quality gate: reasoning **1.50/5** (≥ 1.00 baseline),
agent-pack **5/5** (= baseline). Tool-calling: artichoke `read_file` test → **PASS** — a plain
`qwen -p` run on IQ4_XS read `notes.txt` and answered "artichoke" (rc=0; read-only tools work
without `--yolo`). `CLAUDE.md` production launch, the harness `OPENAI_MODEL`, and this report updated.

## 4. Validated findings

- **Model quant is the only software lever that moved `tg@16k`.** IQ4_XS (4.25 bpw) is +5 %
  over Q4_K_M and Q4_0 (4.5 bpw) is +4.7 %; gen ∝ 1/expert-bytes, exactly as expected for
  memory-bandwidth-bound expert reads. IQ4_XS's slightly heavier dequant did **not** eat the
  bandwidth win on this CPU.
- **What did NOT help `tg@16k`** (all measured `-d 16384`): KV-cache quant q8_0→q4_0 (8.40 vs
  8.38 → the attention-KV *read* is not the bottleneck); partial expert offload
  `--n-cpu-moe 44/46` (8.81–8.85 → confirms the D2 CPU↔GPU ping-pong — pushing experts to the
  GPU trades a CPU read for a PCIe transfer at no net gain); more threads t20 (8.82 → gen is
  bandwidth-bound, not thread-bound); **speculative decoding** with a vocab-compatible
  Qwen3-0.6B draft (8.76 @16k, 17.5 @0-depth — no gain at either depth). Spec decoding
  underperforms here because (a) MoE routing gives weak batch-amortization — verifying K
  drafted tokens still reads ~K different expert sets — and (b) a *general* 0.6B draft only
  moderately predicts a *Coder* target's tokens.
- **Coding-quality gate** (`evals/`): the reasoning bench is **bit-deterministic at temp 0**
  (identical across 3 reps) — so any score change across quants is real signal — but has a
  **floor effect** (Q4_K_M scores 1.00/5 because on several tasks the model makes a
  defensible-but-not-expected tool choice). Treat it as a fast tripwire. The **agent-pack
  (pytest) is the discriminating signal** — Q4_K_M and IQ4_XS both 5/5, giving clean headroom.
- **`qwen --yolo` is mandatory for the headless agent-pack.** Root cause of an initial
  53-turn/0-edit loop: headless `qwen -p` has no TTY to approve tool calls, so write/edit
  tools are silently blocked and the model loops forever. `--yolo` auto-approves; read-only
  tools (the artichoke test) never needed it, which hid the issue. This model also does not
  cleanly self-terminate → bound it with `--max-session-turns`. Security: `--yolo`
  auto-executes shell/write *unsandboxed* → see the `~/.qwen` sandbox TODO in `CLAUDE.md`.
- `llama-bench pp2048@d16384` (steady-state prefill at 16k depth) reads ~53.6 t/s — materially
  below CLAUDE.md's "86 t/s" from-zero average; the original "60 t/s prefill floor" was
  calibrated to the latter, so it's interpreted as *don't regress prefill vs baseline*.

## 5. Limitations & recommendations

- **The ceiling is CPU memory bandwidth on the active-expert read.** At q4 (~0.5 byte/param)
  × ~3 B active ≈ 1.5 GB/token over ~45 GB/s ≈ ~30 t/s for expert reads alone; measured ~9 t/s
  at 16k is further limited by attention-over-KV on the GPU and per-layer CPU↔GPU sync. Smaller
  quant helps only ~linearly with expert bytes, and we're near the IQ4_XS/Q4_0 quality floor.
- **Highest-leverage next steps** (roughly in order): (a) **faster / more RAM channels** — the
  real bottleneck (dual→quad channel or DDR5 would move `tg` more than any flag did); (b) **a
  GPU that can hold the experts** (removes the CPU read entirely — the current 4 GB card can't);
  (c) **trim the ~16k Qwen-Code system prompt** — cuts the one-time ~190 s first-turn prefill,
  the biggest *latency* pain even though it doesn't change steady-state `tg`; (d) a
  **coder-specific or larger draft** for spec decoding — general 0.6B acceptance was too low;
  worth one more try with Qwen3-1.7B or a code-tuned small model before abandoning spec decode.
- **Quality-gate hardening:** the agent-pack is single-shot and slow (~35 min/run); for a
  production regression gate, run N≥3 and diff distributions, and consider extending the task
  set beyond Raschka's 5.
- **Download cost:** remote HF pull ≈ 16 MB/s → each new ~16 GB quant is ~17 min to fetch
  before it can be benched (dominated the quant sweep wall-clock).
