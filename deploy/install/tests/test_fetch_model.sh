# deploy/install/tests/test_fetch_model.sh — the two guards that decide whether a
# 16 GiB download happens. Both were wrong in the plan's draft; both are cheap to pin.
#
# Sourced by run.sh: no `exit`, no EXIT trap, no `report` (see run.sh's header).
. "$HERE/../lib.sh"
# --source-only so main() does not run. fetch-model.sh names its own directory
# variable SCRIPT_DIR, so this cannot clobber the runner's HERE.
. "$HERE/../fetch-model.sh" --source-only

# --- model_present: fitness, not existence -----------------------------------
_t="$(mktemp -d)"

assert_status 1 "model_present: missing directory" model_present "$_t/nope"

mkdir -p "$_t/empty"
assert_status 1 "model_present: empty cache dir" model_present "$_t/empty"

# A small .gguf is a truncated download, not a model.
mkdir -p "$_t/small/snapshots/abc"
: > "$_t/small/snapshots/abc/model.gguf"
assert_status 1 "model_present: undersized .gguf" model_present "$_t/small"

# The real layout: a big blob, and a SYMLINK to it in snapshots/. Plain `find`
# cannot see through that link, which is why model_present uses `find -L`.
mkdir -p "$_t/ok/blobs" "$_t/ok/snapshots/abc"
truncate -s 2G "$_t/ok/blobs/deadbeef"
ln -s ../../blobs/deadbeef "$_t/ok/snapshots/abc/model.gguf"
assert_status 0 "model_present: symlinked blob counts as present" model_present "$_t/ok"

# A partial marker alongside a complete-looking file must still fail: an interrupted
# fetch leaves both, and skipping there loads a truncated GGUF at service start.
cp -a "$_t/ok" "$_t/partial"
: > "$_t/partial/blobs/deadbeef.incomplete"
assert_status 1 "model_present: .incomplete marker vetoes" model_present "$_t/partial"

rm -rf "$_t"
