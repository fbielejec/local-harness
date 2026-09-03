# deploy/install/tests/test_render_units.sh — the render that matters, because a
# bad one installs a broken unit onto a live box.
#
# Sourced by run.sh: no `exit`, no EXIT trap, no `report` (see run.sh's header).
. "$HERE/../lib.sh"

_UNITS="$HERE/../.."   # deploy/

export USER_NAME=filip GROUP_NAME=filip \
       RAG_BIN=/home/filip/.cargo/bin/rag-mcp \
       RAG_WORKDIR=/home/filip/local-harness/rag \
       LLAMA_DIR=/home/filip/Programs/llama.cpp \
       MODEL_PATH=/home/filip/models/x.gguf

out="$(render "$_UNITS/rag-mcp.service")"
assert_contains "$out" "ExecStart=/home/filip/.cargo/bin/rag-mcp" "rag-mcp ExecStart rendered"
assert_contains "$out" "WorkingDirectory=/home/filip/local-harness/rag" "rag-mcp WorkingDirectory rendered"
assert_contains "$out" "User=filip" "rag-mcp User rendered"

# The unit that was redacted. If the marker ever comes back, render must refuse
# rather than install `/home/filip/<deployed-rag-path>` onto the box.
assert_status 1 "render still refuses a surviving redaction marker" \
  bash -c "grep -q '<deployed-rag-path>' \"$_UNITS/rag-mcp.service\""

# unset, not `env -u` — see the Task 2 note; render is a shell function.
unset RAG_BIN
assert_status 1 "render refuses a missing var" render "$_UNITS/rag-mcp.service"
export RAG_BIN=/home/filip/.cargo/bin/rag-mcp

out2="$(render "$_UNITS/llama-server.service")"
assert_contains "$out2" "User=filip" "llama-server User rendered"
assert_contains "$out2" "--model /home/filip/models/x.gguf" "llama-server model path rendered"
assert_contains "$out2" "ExecStart=/home/filip/Programs/llama.cpp/build/bin/llama-server" \
  "llama-server ExecStart rendered"

# The line continuations in ExecStart are what make this unit one command; a render
# that ate them would produce a unit systemd parses as several broken directives.
assert_eq "6" "$(printf '%s\n' "$out2" | grep -c ' \\$')" "llama-server keeps its 6 continuations"

unset USER_NAME GROUP_NAME RAG_BIN RAG_WORKDIR LLAMA_DIR MODEL_PATH
