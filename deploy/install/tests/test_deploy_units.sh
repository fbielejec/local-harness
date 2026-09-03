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
