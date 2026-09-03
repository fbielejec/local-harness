# deploy/install/tests/test_deploy_units.sh — the migration table that keeps two
# units off the same port. Pure lookup; everything else in deploy-units.sh talks
# to systemd or docker and is not worth stubbing.
#
# Sourced by run.sh: no `exit`, no EXIT trap, no `report` (see run.sh's header).
. "$HERE/../lib.sh"
# --source-only so main() does not run; the file's top level only assigns.
. "$HERE/../deploy-units.sh" --source-only

assert_eq "ep-rag-mcp" "$(superseded_by rag-mcp)" "rag-mcp supersedes the POC unit"
assert_status 1 "llama-server supersedes nothing" superseded_by llama-server
assert_status 1 "an unknown unit supersedes nothing" superseded_by not-a-unit

# The table is overridable so the guard can be retired after the cutover without
# editing the script — and so a second rename can reuse it.
assert_eq "old-b" "$(SUPERSEDES='new-a:old-a new-b:old-b' superseded_by new-b)" \
  "SUPERSEDES accepts several pairs"
assert_status 1 "an empty SUPERSEDES disables the guard" \
  bash -c "SUPERSEDES= ; . '$HERE/../deploy-units.sh' --source-only; superseded_by rag-mcp"

# --- rag_workdir_ok: what systemd-analyze cannot see --------------------------
# rag-mcp reads these two relative to WorkingDirectory before it binds a port, so
# a unit that verifies clean can still crash-loop. Found the hard way on 2026-09-03.
_w="$(mktemp -d)"
assert_status 1 "rag_workdir_ok rejects an empty workdir" rag_workdir_ok "$_w"
mkdir -p "$_w/data"; printf '{}\n' > "$_w/data/route_tree.json"
assert_status 1 "...still rejects it with only the route tree" rag_workdir_ok "$_w"
printf '{}\n' > "$_w/data/manifest.jsonl"
assert_status 0 "...accepts it once both are present" rag_workdir_ok "$_w"
: > "$_w/data/manifest.jsonl"   # 0 bytes: an interrupted write, not a manifest
assert_status 1 "...and rejects a 0-byte manifest" rag_workdir_ok "$_w"
rm -rf "$_w"
