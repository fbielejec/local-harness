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

# ── hf_cache_key ────────────────────────────────────────────────────────────
assert_eq "models--unsloth--Qwen3-Coder-30B-A3B-Instruct-GGUF" \
  "$(hf_cache_key 'unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS')" \
  "hf_cache_key: org/repo:quant -> models--org--repo"
assert_eq "models--bare" "$(hf_cache_key 'bare')" "hf_cache_key: no slash, no quant"

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

# --- install_file: DRY_RUN and atomicity --------------------------------------
# The first run against /etc/systemd/system on the live box is a dry one, so the
# dry path has to return the SAME signal as the wet path — 0 "would write",
# 1 "already current" — while touching nothing. A dry run that reported "would
# change" on an already-current unit would make the real run look unnecessary.
ddir="$(mktemp -d)"
printf 'new\n' > "$ddir/src"

rc=0; ( DRY_RUN=1 install_file "$ddir/src" "$ddir/fresh" ) >/dev/null 2>&1 || rc=$?
assert_eq 0 "$rc" "DRY_RUN reports 0 (would create) for a new destination"
assert_status 1 "...and creates nothing" test -e "$ddir/fresh"

printf 'old\n' > "$ddir/dest"
rc=0; ( DRY_RUN=1 install_file "$ddir/src" "$ddir/dest" ) >/dev/null 2>&1 || rc=$?
assert_eq 0 "$rc" "DRY_RUN reports 0 (would overwrite) for a differing destination"
assert_eq "old" "$(cat "$ddir/dest")" "...leaving the destination untouched"
assert_eq 0 "$(ls "$ddir" | grep -c '\.bak-')" "...and writing no backup"

cp "$ddir/src" "$ddir/same"
rc=0; ( DRY_RUN=1 install_file "$ddir/src" "$ddir/same" ) >/dev/null 2>&1 || rc=$?
assert_eq 1 "$rc" "DRY_RUN reports 1 (already current) like the wet path"

# The rename must leave no debris: a surviving .tmp- sibling in /etc/systemd/system
# would be inert (systemd ignores the suffix) but would accumulate one per deploy.
rc=0; install_file "$ddir/src" "$ddir/dest" >/dev/null 2>&1 || rc=$?
assert_eq 0 "$rc" "the wet path still writes after a dry one"
assert_eq "new" "$(cat "$ddir/dest")" "...with the new content"
assert_eq 0 "$(ls "$ddir" | grep -c '\.tmp-')" "...leaving no temp sibling behind"
assert_eq 1 "$(ls "$ddir" | grep -c '\.bak-')" "...and exactly one backup"

# Neither sibling may end in a unit suffix, or systemd would load the backup as a
# second copy of the unit. Asserted on the real name this run produced.
assert_status 1 "backups are not named *.service" \
  bash -c "ls \"$ddir\" | grep -q '\.service\$'"
rm -rf "$ddir"

# --- resolve_model_path -------------------------------------------------------
# The unit is given --model <absolute path>, not -hf, so this is what stands
# between a deploy and a llama-server that dies at start on a path that moved.
rdir="$(mktemp -d)"
assert_status 1 "resolve_model_path dies when the cache is absent" resolve_model_path "$rdir/nope"

mkdir -p "$rdir/cache/blobs" "$rdir/cache/snapshots/abc"
truncate -s 2G "$rdir/cache/blobs/blob-a"
ln -s ../../blobs/blob-a "$rdir/cache/snapshots/abc/Model-IQ4_XS.gguf"
assert_eq "$rdir/cache/snapshots/abc/Model-IQ4_XS.gguf" \
  "$(resolve_model_path "$rdir/cache" IQ4_XS)" "resolve_model_path follows the snapshot symlink"

# Two quants in one cache must NOT resolve to whichever sorts first: the unit
# would silently load a different model, visible only as changed output quality.
truncate -s 2G "$rdir/cache/blobs/blob-b"
ln -s ../../blobs/blob-b "$rdir/cache/snapshots/abc/Model-Q8_0.gguf"
assert_eq "$rdir/cache/snapshots/abc/Model-IQ4_XS.gguf" \
  "$(resolve_model_path "$rdir/cache" IQ4_XS)" "...and the quant filter still disambiguates"
rc=0; msg="$( { resolve_model_path "$rdir/cache" >/dev/null; } 2>&1 )" || rc=$?
assert_eq 1 "$rc" "resolve_model_path refuses an ambiguous cache"
assert_contains "$msg" "ambiguous model" "...saying so"
assert_contains "$msg" "Set MODEL_PATH=" "...and naming the escape hatch"
rm -rf "$rdir"
