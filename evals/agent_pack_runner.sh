#!/usr/bin/env bash
# Drive Raschka's agent-problem-pack through the qwen harness against our local server.
#
# Methodology: rasbt/local-coding-agent-evals (agent-problem-pack). That repo carries no
# license, so we do NOT vendor it — clone it locally and point AGENT_PACK_DIR at it:
#   git clone https://github.com/rasbt/local-coding-agent-evals ~/qwen-scratch/raschka-evals
#
# For each of the 5 problems: prepare an isolated git workspace -> run qwen headless in it
# -> capture (runs `uv run pytest`; exit_code==0 is the pass signal) -> tally pass/N.
# Emits one machine-readable line the autoperf loop parses:
#   QUALITY_AGENTPACK passed=<n>/<N> run=<label>
#
# Requires: uv, qwen, and OPENAI_BASE_URL/OPENAI_MODEL/OPENAI_API_KEY pointing at the server
# (via the SSH tunnel). Run with the server UP on the config under test.
set -u

PACK_DIR="${AGENT_PACK_DIR:-$HOME/qwen-scratch/raschka-evals/agent-problem-pack}"
LABEL="${1:?usage: agent_pack_runner.sh <run-label>}"
QWEN_TIMEOUT="${QWEN_TIMEOUT:-900}"      # per-problem hard wall-clock cap (s), external safety net
QWEN_MAX_TURNS="${QWEN_MAX_TURNS:-25}"   # bound the agent loop (this model tends not to self-terminate)
export QWEN_CODE_SUPPRESS_YOLO_WARNING=1
: "${OPENAI_BASE_URL:?set OPENAI_BASE_URL (e.g. http://localhost:8080/v1)}"
: "${OPENAI_MODEL:?set OPENAI_MODEL}"
export OPENAI_API_KEY="${OPENAI_API_KEY:-dummy}"
export OPENAI_BASE_URL OPENAI_MODEL   # qwen reads the model/endpoint from env

if [ ! -d "$PACK_DIR" ]; then
  echo "error: agent pack not found at $PACK_DIR" >&2
  echo "clone it: git clone https://github.com/rasbt/local-coding-agent-evals ~/qwen-scratch/raschka-evals" >&2
  exit 2
fi
cd "$PACK_DIR" || exit 2

PROBLEMS=$(uv run python scripts/pack_tools.py list | cut -d: -f1)
pass=0; total=0
for prob in $PROBLEMS; do
  total=$((total+1))
  rundir="$PACK_DIR/runs/$prob/$LABEL"
  rm -rf "$rundir"
  uv run python scripts/pack_tools.py prepare "$prob" "$LABEL" >/dev/null || { echo "  $prob: PREPARE_FAIL"; continue; }
  ws="$rundir/workspace"
  prompt="$(cat "$rundir/artifacts/task-prompt.txt")"

  # --yolo: auto-approve edits/writes — REQUIRED, else headless (no TTY) blocks every edit tool
  #   call and the model loops forever without landing a fix. SECURITY: this auto-executes shell/
  #   write unsandboxed at this user's privilege; runs only in the isolated per-run workspace copy.
  #   Harden with a sandbox (the ~/.qwen/settings.json security TODO) before trusting untrusted tasks.
  # --max-session-turns: this model does not cleanly self-terminate; bound the loop. Grading is via
  #   pytest in capture() below, independent of qwen's exit, so the cap does not bias the verdict.
  ( cd "$ws" && timeout "$QWEN_TIMEOUT" \
      qwen --yolo --max-session-turns "$QWEN_MAX_TURNS" -o json -p "$prompt" \
      >"$rundir/artifacts/qwen-output.json" 2>"$rundir/artifacts/qwen-stderr.txt" )
  qrc=$?

  # best-effort token usage -> usage.json (result.usage from qwen json output)
  python3 - "$rundir" <<'PY' 2>/dev/null || true
import json, sys, pathlib
rd = pathlib.Path(sys.argv[1]); art = rd/"artifacts"
try:
    data = json.loads((art/"qwen-output.json").read_text())
except Exception:
    sys.exit(0)
u = (data.get("result") or {}).get("usage") or data.get("usage") or {}
if u:
    rec = {"schema_version":1,"harness":"qwen","source":"cli_json","exact":True,
           "input_tokens":u.get("input_tokens") or u.get("prompt_tokens"),
           "output_tokens":u.get("output_tokens") or u.get("completion_tokens"),
           "total_tokens":u.get("total_tokens"),"raw_usage":u,"notes":""}
    (art/"usage.json").write_text(json.dumps(rec, indent=2)+"\n")
PY

  uv run python scripts/pack_tools.py capture "runs/$prob/$LABEL" >/dev/null 2>&1
  if grep -q "^exit_code=0" "$rundir/artifacts/verification.txt" 2>/dev/null; then
    pass=$((pass+1)); status=PASS
  else
    status=FAIL; [ "$qrc" -eq 124 ] && status="FAIL(timeout)"
  fi
  echo "  $prob: $status"
done

echo "QUALITY_AGENTPACK passed=$pass/$total run=$LABEL"
[ "$pass" -eq "$total" ]
