# deploy/install/tests/test_client.sh — client.sh's pure logic and its merge.
#
# Sourced by run.sh: no `exit`, no EXIT trap, no `report` (see run.sh's header).
. "$HERE/../lib.sh"

# Sourcing must define functions and do nothing else. client.sh names its own
# directory variable SCRIPT_DIR precisely so that this line cannot clobber the
# runner's HERE — no save/restore needed here.
. "$HERE/../client.sh" --source-only

# --- node_version_ok: the version gate, the only pure function here -----------
assert_status 0 "node 20 ok"    node_version_ok "v20.11.0"
assert_status 0 "node 26 ok"    node_version_ok "v26.8.1"
assert_status 1 "node 18 fails" node_version_ok "v18.15.0"
assert_status 1 "garbage fails" node_version_ok "not-a-version"

# --- deploy_qwen_settings: merge semantics, against a throwaway HOME ----------
# The function reads $HOME at call time, so overriding it in a subshell reaches
# the whole code path without touching the real ~/.qwen. The subshell also
# contains lib.sh's die(), which would otherwise kill the runner.
_sandbox="$(mktemp -d)"
mkdir -p "$_sandbox/.qwen"
# A user-hand-edited file: one key the asset does not carry, one it contradicts.
printf '{"permissions":{"allow":["Bash(mkdir *)"]},"telemetry":{"enabled":true}}\n' \
  > "$_sandbox/.qwen/settings.json"

( HOME="$_sandbox" deploy_qwen_settings ) >/dev/null 2>&1
assert_eq "Bash(mkdir *)" "$(jq -r '.permissions.allow[0]' "$_sandbox/.qwen/settings.json")" \
  "merge keeps a user-only key"
assert_eq "false" "$(jq -r '.telemetry.enabled' "$_sandbox/.qwen/settings.json")" \
  "merge lets the asset win a conflict"
assert_eq "http://localhost:8082/mcp" "$(jq -r '.mcpServers["ep-rag"].httpUrl' "$_sandbox/.qwen/settings.json")" \
  "merge adds the ep-rag server"
assert_eq "1" "$(ls "$_sandbox/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "changed merge leaves exactly one backup"

# The whole point of merging into a temp and handing THAT to install_file: once
# the merge stops changing anything, a re-run must write nothing at all.
( HOME="$_sandbox" deploy_qwen_settings ) >/dev/null 2>&1
assert_eq "1" "$(ls "$_sandbox/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "re-run is a no-op: no second backup"

rm -rf "$_sandbox"

# The other branch: a fresh machine with no ~/.qwen/settings.json at all. There
# is nothing to merge, so the asset must land verbatim and leave no backup.
_sandbox="$(mktemp -d)"
mkdir -p "$_sandbox/.qwen"
( HOME="$_sandbox" deploy_qwen_settings ) >/dev/null 2>&1
assert_status 0 "fresh install writes the asset verbatim" \
  cmp -s "$REPO/deploy/qwen/settings.json" "$_sandbox/.qwen/settings.json"
assert_eq "0" "$(ls "$_sandbox/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "fresh install leaves no backup"

rm -rf "$_sandbox"; unset _sandbox

# --- main(): end to end, EXECUTED not sourced ---------------------------------
# main() is the only place `set -euo pipefail` is in effect, and nothing above
# reaches it. Asserting deploy_qwen_settings' status cannot substitute: it ends
# in `rm -f "$merged"` so it always returns 0. Specifically pinned here: the
# condition-context call to install_file. Demoting `if install_file ...; then`
# to a bare call leaves every assert above green while aborting the real script
# rc=1 on the happy path of every re-run — exactly what install_file's doc
# comment warns about in capitals.
#
# A stub qwen shadows the real one so the npm branch is never taken and the
# assert measures this script rather than the machine's node install.
_e2e="$(mktemp -d)"
mkdir -p "$_e2e/bin" "$_e2e/home"
printf '#!/bin/sh\nexit 0\n' > "$_e2e/bin/qwen"; chmod +x "$_e2e/bin/qwen"

assert_status 0 "client.sh runs clean on a fresh HOME" \
  env HOME="$_e2e/home" PATH="$_e2e/bin:$PATH" bash "$HERE/../client.sh"
# The re-run is the assert that matters: every guard is now on its skip path.
assert_status 0 "client.sh re-runs clean (install_file's 1 is handled)" \
  env HOME="$_e2e/home" PATH="$_e2e/bin:$PATH" bash "$HERE/../client.sh"

assert_eq "0" "$(ls "$_e2e/home/.qwen"/settings.json.bak-* 2>/dev/null | wc -l)" \
  "two end-to-end runs on a fresh HOME leave no backup"
assert_status 0 "end-to-end run seeds the artichoke fixture" \
  grep -q artichoke "$_e2e/home/qwen-scratch/notes.txt"

# IMP-2, branch 1: an interrupted run leaves a 0-byte notes.txt, which -f accepts.
rm -rf "$_e2e/home/qwen-scratch"; mkdir -p "$_e2e/home/qwen-scratch"
: > "$_e2e/home/qwen-scratch/notes.txt"
( env HOME="$_e2e/home" PATH="$_e2e/bin:$PATH" bash "$HERE/../client.sh" ) >/dev/null 2>&1
assert_status 0 "a truncated fixture is repaired, not skipped" \
  grep -q artichoke "$_e2e/home/qwen-scratch/notes.txt"

# IMP-2, branch 2: a user's own notes must survive the repair.
printf 'my own notes\n' > "$_e2e/home/qwen-scratch/notes.txt"
( env HOME="$_e2e/home" PATH="$_e2e/bin:$PATH" bash "$HERE/../client.sh" ) >/dev/null 2>&1
assert_status 0 "the fixture is appended to a user's notes" \
  grep -q artichoke "$_e2e/home/qwen-scratch/notes.txt"
assert_status 0 "...and the user's own content survives" \
  grep -q 'my own notes' "$_e2e/home/qwen-scratch/notes.txt"

rm -rf "$_e2e"; unset _e2e
