#!/usr/bin/env bash
# autoperf QUALITY GATE — run against the server that is CURRENTLY UP on the config under test.
#
# The caller is responsible for launching llama-server on the config and opening the tunnel
# BEFORE calling this. This script only measures quality; it does not start/stop the server.
#
# Usage:
#   evals/quality_gate.sh <label>          # fast gate: reasoning bench only (~1 min)
#   evals/quality_gate.sh <label> --full   # + agent-pack (real qwen file-editing, ~10-20 min)
#
# Prints the machine-readable lines the loop compares against baseline:
#   QUALITY_REASONING total=<X.XX>/5 mean=<..> strict_passed=<n>/5 runs=<R>
#   QUALITY_AGENTPACK passed=<n>/5 run=<..>   (only with --full)
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
LABEL="${1:?usage: quality_gate.sh <label> [--full]}"
FULL=0; [ "${2:-}" = "--full" ] && FULL=1

export OPENAI_BASE_URL="${OPENAI_BASE_URL:-http://localhost:8080/v1}"
export OPENAI_MODEL="${OPENAI_MODEL:-unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M}"
export OPENAI_API_KEY="${OPENAI_API_KEY:-dummy}"
REPEATS="${REASONING_REPEATS:-3}"
mkdir -p "$HERE/results"

echo "=== QUALITY GATE: $LABEL (base_url=$OPENAI_BASE_URL) ==="
echo "--- reasoning bench (repeats=$REPEATS) ---"
python3 "$HERE/reasoning/reasoning_bench.py" \
  --repeats "$REPEATS" \
  --csv "$HERE/results/reasoning-$LABEL.csv"
r_rc=$?

if [ "$FULL" = "1" ]; then
  echo "--- agent-pack (qwen) ---"
  bash "$HERE/agent_pack_runner.sh" "$LABEL"
fi

exit "$r_rc"
