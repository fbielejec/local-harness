# Root Makefile — [client] laptop-side helpers for reaching weebeastie and
# driving the harness. The RAG ingestion pipeline has its own Makefile in rag/.

REMOTE ?= filip@192.168.1.22
MODEL  ?= unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS

.PHONY: help # Show help for each of the Makefile recipes
help:
	@grep -E '^\.PHONY: .+ #' Makefile | sort | while read -r l; do printf "\033[1;32m%s\033[00m:%s\n" "$$(echo "$$l" | sed -E 's/^\.PHONY: ([^ ]+).*/\1/')" "$$(echo "$$l" | cut -f 2- -d'#')"; done

.PHONY: tunnel-llama # [client] SSH-forward the remote llama-server to localhost:8080
tunnel-llama:
	ssh -fN -L 8080:127.0.0.1:8080 $(REMOTE)
	@echo "llama-server → http://localhost:8080/v1   (stop: make tunnel-stop  ·  or pkill -f 'ssh -fN -L 8080')"

.PHONY: tunnel-qdrant # [client] SSH-forward the remote Qdrant to localhost:16333 (REST) + 16334 (gRPC)
tunnel-qdrant:
	ssh -fN -L 16333:127.0.0.1:6333 -L 16334:127.0.0.1:6334 $(REMOTE)
	@echo "Qdrant → http://localhost:16333/dashboard  ·  gRPC localhost:16334"

.PHONY: tunnel-mcp # [client] SSH-forward the remote ep-rag-mcp to localhost:8082
tunnel-mcp:
	ssh -fN -L 8082:127.0.0.1:8082 $(REMOTE)
	@echo "ep-rag-mcp → http://localhost:8082/route   (stop: make tunnel-stop  ·  or pkill -f 'ssh -fN -L 8082')"

.PHONY: tunnels # [client] open both the llama-server and Qdrant SSH tunnels
tunnels: tunnel-llama tunnel-qdrant tunnel-mcp

.PHONY: tunnels-stop # [client] close the llama-server + Qdrant SSH tunnels
tunnels-stop:
	@# [s]sh regex matches the real ssh process but not pkill's own recipe-shell argv.
	-pkill -f "[s]sh -fN -L 8080:127.0.0.1:8080 $(REMOTE)"
	-pkill -f "[s]sh -fN -L 16333:127.0.0.1:6333"
	@echo "tunnels closed"

.PHONY: qwen # [client] export OPENAI_* env and boot qwen against the tunnelled llama-server (P="prompt" optional; needs tunnel-llama)
qwen:
	OPENAI_BASE_URL="http://localhost:8080/v1" \
	OPENAI_API_KEY="dummy" \
	OPENAI_MODEL="$(MODEL)" \
	qwen $(if $(P),-p "$(P)",)
