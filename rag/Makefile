# EP-RAG — ingestion pipeline commands. Run from rag/ (the cargo workspace root).
# One target per pipeline step (in order), the two acceptance gates, and
# `make pipeline` to run the whole chain. Bare `make` prints the target list.
#
#   fetch ─► ingest (parse+chunk) ─► index (embed+upsert) ─► Qdrant
#   gates:  parse-gate · embed-gate · parity (cross-language embed check)

COMPOSE   := docker compose -f ../deploy/qdrant/docker-compose.yml
LIMIT     ?=
# Release: candle's numerical kernels run 10-50x slower unoptimized (debug).
# One profile for every run target so `make pipeline` never recompiles mid-chain.
CARGO_RUN := cargo run --release

.DEFAULT_GOAL := help
.PHONY: help qdrant-up qdrant-down qdrant-logs qdrant-status \
	fetch ingest index parse-gate embed-gate parity pipeline serve serve-mcp test

help: ## list targets
	@grep -hE '^[a-zA-Z0-9_-]+:.*##' $(MAKEFILE_LIST) | sort | awk 'BEGIN{FS=":.*## "}{printf "  \033[36m%-13s\033[0m %s\n", $$1, $$2}'

# --- infra ------------------------------------------------------------------
qdrant-up: ## start Qdrant (Docker, detached)
	$(COMPOSE) up -d

qdrant-down: ## stop Qdrant
	$(COMPOSE) down

qdrant-logs: ## follow Qdrant logs
	$(COMPOSE) logs -f

qdrant-status: ## list Qdrant collections (is anything indexed yet?)
	curl -s http://localhost:6333/collections | python3 -m json.tool

# --- pipeline steps (in order) ----------------------------------------------
fetch: ## 1. download EN PDFs + manifest.jsonl   (LIMIT=2 → 2 docs/type; empty = full)
	$(CARGO_RUN) -p ep-rag-fetch --bin fetch -- $(LIMIT)

ingest: ## 2. parse + chunk every manifest doc → data/chunks.jsonl
	$(CARGO_RUN) -p ep-rag-ingest --bin ingest

index: ## 3. embed chunks + upsert to Qdrant with the contract-stamped payload (Drill-1 surface)
	$(CARGO_RUN) -p ep-rag-index --bin index

# --- gates / verification ---------------------------------------------------
parse-gate: ## pdf-extract vs pymupdf parity gate
	$(CARGO_RUN) -p ep-rag-parse --bin parse-gate

embed-gate: ## candle embed gate → data/parity/rust.json
	$(CARGO_RUN) -p ep-rag-embed --bin embed-gate

parity: embed-gate ## cross-language embed parity: candle vs sentence-transformers
	uv run --project drills python drills/parity_gate.py

# --- joint ------------------------------------------------------------------
pipeline: qdrant-up fetch ingest index ## run the whole ingestion pipeline end to end
	@echo "pipeline complete → Qdrant collection populated"

serve: ## run rag-server (OpenAI-compatible RAG endpoint on :8081)
	$(CARGO_RUN) -p ep-rag-server --bin rag-server

serve-mcp: ## run ep-rag-mcp (retrieval + /route + MCP tool) on loopback :8082
	$(CARGO_RUN) -p ep-rag-mcp

test: ## cargo test across the workspace
	cargo test
