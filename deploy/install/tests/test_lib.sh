# deploy/install/tests/test_lib.sh — unit tests for deploy/install/lib.sh.
#
# Sourced by run.sh: no `exit`, no EXIT trap, no `report` call here. $HERE comes
# from the runner and is this directory. die() is contained by assert_status and
# by $(...), but NOT at bare file scope — so every call that can die is wrapped
# in one of the two.
. "$HERE/../lib.sh"

# ── arch_from_compute_cap ───────────────────────────────────────────────────
# The whole point is 6.1 -> 61. detect_cuda_arch is deliberately NOT tested — it
# shells out to nvidia-smi and die()s when absent, which is every run on the laptop.
assert_eq "61" "$(arch_from_compute_cap "6.1")"      "arch 6.1"
assert_eq "89" "$(arch_from_compute_cap "8.9")"      "arch 8.9"
assert_eq "120" "$(arch_from_compute_cap "12.0")"    "arch 12.0 (two digits)"

# ── render ──────────────────────────────────────────────────────────────────
tmp="$(mktemp)"; printf 'Exec=@BIN@\nUser=@USER_NAME@\n' > "$tmp"
out="$(export USER_NAME=filip BIN=/x/y; render "$tmp")"
assert_contains "$out" "Exec=/x/y"    "render substitutes BIN"
assert_contains "$out" "User=filip"   "render substitutes USER_NAME"

# NOTE: do NOT write `env BIN=/x/y render ...` — env(1) execs a *binary* and
# render is a shell function, so it would fail with 127 for the wrong reason.
# Exports stay scoped inside the command substitution, like the assert above, so
# an abort between asserts cannot leak BIN or EMPTY_VAR into a later test file.
#
# Status alone is too weak to assert here: render returns 1 for several distinct
# reasons and the number cannot tell them apart, so each case also pins the
# diagnostic that names it.
tmp2="$(mktemp)"; printf 'Exec=@BIN@\nOops=@NOT_SET@\n' > "$tmp2"
rc=0; msg="$( export BIN=/x/y; { render "$tmp2" >/dev/null; } 2>&1 )" || rc=$?
assert_eq 1 "$rc" "render rejects a leftover placeholder"
assert_contains "$msg" "@NOT_SET@ is not set" "...and names the offending placeholder"

# Set-but-EMPTY is rejected too, and is the nastier of the two: it renders
# `WorkingDirectory=` with nothing obviously missing, so systemd fails at start
# time instead of install time. Distinct case from the unset assert above.
tmp3="$(mktemp)"; printf 'WorkingDirectory=@EMPTY_VAR@\n' > "$tmp3"
rc=0; msg="$( export EMPTY_VAR=; { render "$tmp3" >/dev/null; } 2>&1 )" || rc=$?
assert_eq 1 "$rc" "render rejects a set-but-empty placeholder"
assert_contains "$msg" "@EMPTY_VAR@ is set but EMPTY" "...and says EMPTY, not unset"

# An empty result passes BOTH placeholder guards (they pattern-match on the
# output, and empty output matches neither), so it needs its own guard. An empty
# template reaches it honestly — no envsubst stub required.
tmp4="$(mktemp)"   # deliberately left empty
rc=0; msg="$( { render "$tmp4" >/dev/null; } 2>&1 )" || rc=$?
assert_eq 1 "$rc" "render refuses to emit an empty document"
assert_contains "$msg" "produced no output" "...saying why it refused"
# The post-render grep is the ONLY thing standing between the redacted units and
# production: <deployed-rag-path> is not @VAR@-shaped, so the source-side loop
# above never sees it and envsubst leaves it untouched. Without this assert the
# whole grep can be deleted and the suite still reports green.
tmp5="$(mktemp)"; printf 'WorkingDirectory=/home/filip/<deployed-rag-path>\n' > "$tmp5"
rc=0; msg="$( { render "$tmp5" >/dev/null; } 2>&1 )" || rc=$?
assert_eq 1 "$rc" "render rejects a surviving redaction marker"
assert_contains "$msg" "deployed-rag-path" "...naming the marker"
rm -f "$tmp" "$tmp2" "$tmp3" "$tmp4" "$tmp5"

# ── install_file ────────────────────────────────────────────────────────────
# Called through `|| rc=$?` rather than assert_status: that keeps a leaked
# errexit from seeing the bare 1 at file scope, AND keeps the filesystem side
# effects in this shell, which assert_status's subshell would discard.
d="$(mktemp -d)"; src="$d/src"; dest="$d/dest"

printf 'v1\n' > "$src"
rc=0; install_file "$src" "$dest" >/dev/null 2>&1 || rc=$?
assert_eq 0 "$rc"               "install_file writes a new destination"
assert_eq "v1" "$(cat "$dest")" "...with the source content"

printf 'v2\n' > "$src"
rc=0; install_file "$src" "$dest" >/dev/null 2>&1 || rc=$?
assert_eq 0 "$rc"               "install_file overwrites a differing destination"
assert_eq "v2" "$(cat "$dest")" "...with the new content"
assert_eq 1 "$(ls "$d" | grep -c '\.bak-')" "...and leaves exactly one backup"
assert_eq "v1" "$(cat "$d"/dest.bak-*)"     "...holding the previous content"

# The idempotent case, and the reason for the doc comment on the definition.
rc=0; install_file "$src" "$dest" >/dev/null 2>&1 || rc=$?
assert_eq 1 "$rc"               "install_file returns 1 (signal) when already current"
assert_eq "v2" "$(cat "$dest")" "...leaving the destination untouched"
assert_eq 1 "$(ls "$d" | grep -c '\.bak-')" "...and creating no second backup"
rm -rf "$d"

# ── resolve_cargo ───────────────────────────────────────────────────────────
# The cargo trap: `ssh host make ...` is non-interactive, ~/.bashrc returns early
# and ~/.cargo/bin is off PATH. Every branch runs against a stub tree so the
# result never depends on whether the machine running the tests has cargo.
# resolve_cargo uses only builtins, so PATH=/nonexistent is safe here.
cdir="$(mktemp -d)"; mkdir -p "$cdir/bin" "$cdir/home/.cargo/bin"
printf '#!/bin/sh\n' > "$cdir/bin/cargo";             chmod +x "$cdir/bin/cargo"
printf '#!/bin/sh\n' > "$cdir/home/.cargo/bin/cargo"; chmod +x "$cdir/home/.cargo/bin/cargo"
noexec="$cdir/not-executable"; : > "$noexec"

assert_eq "$cdir/bin/cargo" "$( CARGO="$cdir/bin/cargo" resolve_cargo )" \
  "resolve_cargo honours an executable \$CARGO"
assert_eq "$cdir/bin/cargo" "$( CARGO="$noexec" PATH="$cdir/bin:$PATH" resolve_cargo )" \
  "resolve_cargo falls through when \$CARGO is set but not executable"
assert_eq "$cdir/home/.cargo/bin/cargo" "$( CARGO= HOME="$cdir/home" PATH=/nonexistent resolve_cargo )" \
  "resolve_cargo finds ~/.cargo/bin when PATH has no cargo"

rc=0; msg="$( { CARGO= HOME="$cdir/nowhere" PATH=/nonexistent resolve_cargo >/dev/null; } 2>&1 )" || rc=$?
assert_eq 1 "$rc" "resolve_cargo dies when cargo resolves nowhere"
assert_contains "$msg" "cargo not found" "...naming the three places it tried"
rm -rf "$cdir"

# --- install_file: destination mode -------------------------------------------
# "Always 0644" WIDENS an existing 0600 file. That is harmless for a settings
# file but not for a systemd unit carrying a credential or an EnvironmentFile,
# which this same helper installs. Both branches are pinned: preserve when the
# destination exists, 0644 when it does not.
mdir="$(mktemp -d)"
printf 'old\n' > "$mdir/dest"; chmod 0600 "$mdir/dest"
printf 'new\n' > "$mdir/src"
( install_file "$mdir/src" "$mdir/dest" ) >/dev/null 2>&1
assert_eq "600" "$(stat -c '%a' "$mdir/dest")" "install_file preserves an existing 0600 mode"

( install_file "$mdir/src" "$mdir/fresh" ) >/dev/null 2>&1
assert_eq "644" "$(stat -c '%a' "$mdir/fresh")" "install_file creates a new file 0644"
rm -rf "$mdir"
