# deploy/install/tests/assert.sh — three asserts, no framework. bats is not
# installed on either machine and this is not worth a dependency.
#
# Initialised conditionally, never reset: a test file may source this again so it
# can be run standalone, and a plain `FAILED=0` there would erase every failure
# recorded before it and report green.
: "${FAILED:=0}"; : "${PASSED:=0}"
assert_eq() { # want, got, label
  if [ "$1" = "$2" ]; then PASSED=$((PASSED+1)); else
    FAILED=$((FAILED+1)); printf 'FAIL %s\n  want: %s\n  got:  %s\n' "$3" "$1" "$2" >&2
  fi
}
assert_contains() { # haystack, needle, label
  case "$1" in *"$2"*) PASSED=$((PASSED+1));; *)
    FAILED=$((FAILED+1)); printf 'FAIL %s\n  %s does not contain %s\n' "$3" "$1" "$2" >&2;; esac
}
assert_status() { # want_rc, label, cmd...
  local want="$1" label="$2"; shift 2
  if [ "$#" -eq 0 ]; then
    FAILED=$((FAILED+1)); printf 'FAIL %s\n  no command given to assert_status\n' "$label" >&2
    return
  fi
  # Subshell so the command under test — install-script code that may `exit`, as
  # lib.sh's die() does — cannot kill the runner. `|| got=$?` puts the call in a
  # condition context, which is what keeps a *leaked* errexit (a test file that
  # sources client.sh inherits its `set -euo pipefail`) from killing the runner on
  # every expected-nonzero assert. Both halves are load-bearing; neither alone is
  # enough. The cost of the subshell: a function under test cannot propagate
  # variable assignments back to the test file — assert on its output or its
  # status, not on what it set.
  local got=0
  ( "$@" ) >/dev/null 2>&1 || got=$?
  assert_eq "$want" "$got" "$label"
}
report() { printf '%s passed, %s failed\n' "$PASSED" "$FAILED"; [ "$FAILED" -eq 0 ]; }
