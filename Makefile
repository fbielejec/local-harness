# Root Makefile — [client] laptop-side helpers for reaching weebeastie and
# driving the harness. The RAG ingestion pipeline has its own Makefile in rag/.

# REMOTE defaults to the home LAN. `make away` re-points it at the WireGuard tunnel —
# target-specific variables propagate to prerequisites in GNU Make, so every tunnel-*
# target follows without duplicating a single recipe.
REMOTE   ?= filip@192.168.1.22
WG_PEER  := filip@10.10.0.1
WG_ADDR  := 10.10.0.1
WG_IFACE ?= wg0
MODEL    ?= unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS

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

.PHONY: tunnel-mcp # [client] SSH-forward the remote rag-mcp to localhost:8082
tunnel-mcp:
	ssh -fN -L 8082:127.0.0.1:8082 $(REMOTE)
	@echo "rag-mcp → http://localhost:8082/route   (stop: make tunnel-stop  ·  or pkill -f 'ssh -fN -L 8082')"

.PHONY: tunnels # [client] open both the llama-server and Qdrant SSH tunnels
tunnels: tunnel-llama tunnel-qdrant tunnel-mcp

.PHONY: tunnels-stop # [client] close the llama-server + Qdrant SSH tunnels
tunnels-stop:
	@# [s]sh regex matches the real ssh process but not pkill's own recipe-shell argv.
	@# Host deliberately omitted: matches whether the tunnel went over the LAN or WireGuard.
	-pkill -f "[s]sh -fN -L 8080:127.0.0.1:8080"
	-pkill -f "[s]sh -fN -L 16333:127.0.0.1:6333"
	-pkill -f "[s]sh -fN -L 8082:127.0.0.1:8082"
	@echo "tunnels closed"

# ── Off the home LAN ────────────────────────────────────────────────────────────
# The WireGuard endpoint is the home PUBLIC IP, and the router does NOT hairpin, so
# `make away` only works from OUTSIDE the house. At home use `make tunnels` directly.
# Deliberately not a boot-time service: an auto-started tunnel would be permanently
# broken at home, which is where you are most of the time.

.PHONY: wg-up # [client] bring up the WireGuard tunnel — off-LAN only (needs sudo)
wg-up:
	sudo wg-quick up $(WG_IFACE)
	@# Fail fast and leave no trace: a dangling wg0 would blackhole $(WG_ADDR) and make
	@# the next ssh hang for minutes instead of erroring.
	@ping -c1 -W3 $(WG_ADDR) >/dev/null 2>&1 || { \
	  sudo wg-quick down $(WG_IFACE) >/dev/null 2>&1; \
	  echo ""; \
	  echo "!! wg0 came up but $(WG_ADDR) is unreachable — tunnel torn back down."; \
	  echo "   Most likely: you are ON the home LAN. The endpoint is the home public IP"; \
	  echo "   and the router does not hairpin, so the packets die at the router."; \
	  echo "   At home:  make tunnels     (direct over the LAN)"; \
	  echo "   Away:     make wg-up && make wg-status   (look for a handshake)"; \
	  echo ""; \
	  exit 1; }
	@echo "WireGuard up · $(WG_ADDR) reachable"

.PHONY: wg-down # [client] tear down the WireGuard tunnel
wg-down:
	-sudo wg-quick down $(WG_IFACE)

.PHONY: wg-status # [client] WireGuard peer state (endpoint, handshake, transfer)
wg-status:
	@sudo wg show $(WG_IFACE) 2>/dev/null || echo "$(WG_IFACE) is down"

.PHONY: away # [client] off the home LAN: WireGuard up, then all tunnels over it
away: REMOTE := $(WG_PEER)
away: wg-up tunnels

.PHONY: away-stop # [client] close the tunnels and drop WireGuard
away-stop: tunnels-stop wg-down

.PHONY: qwen # [client] export OPENAI_* env and boot qwen against the tunnelled llama-server (P="prompt" optional; needs tunnel-llama)
qwen:
	OPENAI_BASE_URL="http://localhost:8080/v1" \
	OPENAI_API_KEY="dummy" \
	OPENAI_MODEL="$(MODEL)" \
	qwen $(if $(P),-p "$(P)",)

.PHONY: remote-qdrant-status # list remote Qdrant collections (is anything indexed yet?)
remote-qdrant-status:
	curl -s http://localhost:16333/collections | python3 -m json.tool
