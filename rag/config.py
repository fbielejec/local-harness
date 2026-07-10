"""Single source of truth for the EP-RAG pipeline.

Everything environment-tunable lives here, so "run on my laptop" vs "run on
weebeastie" is a change to `.env`, never a change to code. Import `settings` and
`CONTRACT` from here; do not read `os.environ` anywhere else.
"""
from __future__ import annotations

import os
from dataclasses import asdict, dataclass
from pathlib import Path

from dotenv import load_dotenv

# Load a .env sitting next to this file, if present. Overrides + secrets live
# there (gitignored); `.env.example` documents every key. Real env vars already
# set in the shell win over .env (override=False is the default).
load_dotenv(Path(__file__).parent / ".env")


# --------------------------------------------------------------------------- #
# The embedding contract — the heart of the retrieval half (Drill 1).
# --------------------------------------------------------------------------- #
@dataclass(frozen=True)
class EmbeddingContract:
    """The exact conditions under which a vector was produced.

    A retriever score is ``d(z) · q(x)`` — a dot product between a DOCUMENT
    vector and a QUERY vector. It is only meaningful if BOTH were produced
    identically. Any drift here — model, version, normalization, or the BGE
    query-only instruction prefix — leaves each vector looking individually fine
    (right shape, right norm) while silently corrupting the cross-term ``d·q``.
    That is the classic "confidently wrong retrieval" bug.

    We stamp this onto every Qdrant payload at ingest, and assert the live query
    contract equals the stored one at serve time — turning an invisible bug into
    a loud, caught one.
    """

    model: str              # e.g. "BAAI/bge-small-en-v1.5"
    version: str            # our own pin; bump whenever we knowingly re-embed
    dim: int                # 384 for bge-small
    normalized: bool        # BGE is cosine-trained -> we L2-normalize
    query_instruction: str  # prepended to QUERIES only (never to passages)

    def as_payload(self) -> dict:
        """Flat ``contract_*`` dict stamped onto each chunk's Qdrant payload."""
        return {f"contract_{k}": v for k, v in asdict(self).items()}

    def assert_matches(self, stored: dict) -> None:
        """Refuse to serve if the live contract != the index's stored contract."""
        mine = self.as_payload()
        drift = {k: {"indexed": stored.get(k), "live": v}
                 for k, v in mine.items() if stored.get(k) != v}
        if drift:
            raise RuntimeError(
                "Embedding-contract mismatch (index vs query) — refusing to serve. "
                f"Drifted fields: {drift}"
            )


# --------------------------------------------------------------------------- #
# Runtime settings (env-driven; safe local defaults).
# --------------------------------------------------------------------------- #
def _env(key: str, default: str) -> str:
    return os.environ.get(key, default)


@dataclass(frozen=True)
class Settings:
    # -- Vector DB (Qdrant) --
    qdrant_url: str = _env("QDRANT_URL", "http://localhost:6333")
    collection: str = _env("QDRANT_COLLECTION", "ep_committee_docs")

    # -- Embeddings (BGE) --
    embed_model: str = _env("EMBED_MODEL", "BAAI/bge-small-en-v1.5")

    # -- Generator (OpenAI-compatible; defaults to the tunneled Qwen) --
    # Project-namespaced (GEN_*) ON PURPOSE: do NOT read an ambient OPENAI_API_KEY
    # from the shell — that would silently pull in an unrelated real key.
    gen_base_url: str = _env("GEN_BASE_URL", "http://localhost:8080/v1")
    gen_api_key: str = _env("GEN_API_KEY", "dummy")  # llama-server ignores it
    gen_model: str = _env(
        "GEN_MODEL", "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:IQ4_XS"
    )

    # -- Retrieval --
    top_k: int = int(_env("TOP_K", "5"))

    # -- Corpus fetch --
    data_dir: str = _env("DATA_DIR", "data")


settings = Settings()

# The one contract the whole system pins to. If you change `embed_model`, you must
# also update `dim` and bump `version` — they travel together.
CONTRACT = EmbeddingContract(
    model=settings.embed_model,
    version="v1",
    dim=384,
    normalized=True,
    query_instruction="Represent this sentence for searching relevant passages:",
)


if __name__ == "__main__":
    import json

    print("Settings (env-driven):")
    for k, v in asdict(settings).items():
        shown = "***masked***" if (("key" in k or "secret" in k) and v) else v
        print(f"  {k:16s} = {shown}")

    print("\nEmbeddingContract — stamped into every chunk payload:")
    print(json.dumps(CONTRACT.as_payload(), indent=2))

    # Self-test the guard: a matching contract passes; a dropped query prefix
    # (the most common real mismatch) is caught. This is Drill 1 in miniature.
    print("\nContract self-check (matching) ->", end=" ")
    CONTRACT.assert_matches(CONTRACT.as_payload())
    print("OK")

    tampered = dict(CONTRACT.as_payload())
    tampered["contract_query_instruction"] = ""  # simulate forgetting the prefix
    try:
        CONTRACT.assert_matches(tampered)
    except RuntimeError as e:
        print("Contract self-check (dropped query prefix) -> CAUGHT:")
        print("  ", str(e))
