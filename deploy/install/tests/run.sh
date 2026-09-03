#!/usr/bin/env bash
# deploy/install/tests/run.sh — source every test_*.sh and report once.
#
# Two constraints on test files, enforced here:
#   1. Do not call `exit`. Files are sourced, so an `exit` ends this process too:
#      failures print, no tally follows, and make sees success. Use `return`.
#   2. Do not install an `EXIT` trap. Bash does not stack traps, so yours would
#      silently replace the abort trap below and reopen the same hole.
# Both are conventions, not guarantees — running each file in its own subshell
# would enforce them, at the price of the shared counters this harness is built on.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/assert.sh"
# _RUN_LOOP_DONE is set by the runner alone, never by a function a test file can
# reach: `report` is public, and a standalone-runnable test file ending in a call
# to it would otherwise disarm this trap mid-run.
trap '[ -n "${_RUN_LOOP_DONE:-}" ] || { printf "HARNESS ABORTED before report — result is not trustworthy\n" >&2; exit 1; }' EXIT
# The file count is reported alongside the tally, deliberately WITHOUT a minimum
# threshold (Task 1 requires this suite to be green at zero tests). It exists to
# make a vanished test file visible: a sourced script that clobbers HERE makes
# every later test_*.sh resolve `$HERE/../lib.sh` to nothing and source an empty
# file, so its asserts disappear from the tally with no abort and no failure —
# just a smaller green number that nobody is comparing against anything.
_FILES=0
for t in "$HERE"/test_*.sh; do
  [ -e "$t" ] || continue
  echo "--- $(basename "$t")"; . "$t"; _FILES=$((_FILES+1))
done
_RUN_LOOP_DONE=1
printf -- '--- %s file(s)\n' "$_FILES"
report
