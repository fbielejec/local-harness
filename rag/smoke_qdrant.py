"""Brick 2 smoke test: prove we can talk to Qdrant end-to-end.

The vector-DB mental model in ~40 lines:
    collection  ->  points (id + vector + payload)  ->  cosine search  ->  ranked scores

We reuse the org-doc toy: Paris/Rome/Berlin as one-hot 'embeddings', then a
"Paris-ish" query. Cosine similarity ignores magnitude, so the query leans hard
to Paris. Nothing here is real yet — it's the plumbing check before real BGE
vectors go in (Brick 5).
"""
from __future__ import annotations

from qdrant_client import QdrantClient, models

from config import settings

COLL = "smoke_test"


def main() -> None:
    client = QdrantClient(url=settings.qdrant_url)
    print(f"connected to Qdrant at {settings.qdrant_url}")

    # A collection is a named table of same-dim vectors + payloads. Fresh each run.
    if client.collection_exists(COLL):
        client.delete_collection(COLL)
    client.create_collection(
        COLL,
        vectors_config=models.VectorParams(size=3, distance=models.Distance.COSINE),
    )
    print(f"created collection '{COLL}' (dim=3, cosine)")

    cities = {"Paris":  [1.0, 0.0, 0.0],
              "Rome":   [0.0, 1.0, 0.0],
              "Berlin": [0.0, 0.0, 1.0]}
    client.upsert(
        COLL,
        points=[
            models.PointStruct(id=i, vector=vec, payload={"city": name})
            for i, (name, vec) in enumerate(cities.items())
        ],
    )
    print(f"upserted {len(cities)} points")

    query = [0.9, 0.1, 0.0]  # a 'Paris-ish' query vector
    hits = client.query_points(COLL, query=query, limit=3).points
    print(f"\nquery {query} -> ranked hits (cosine score in [-1, 1]):")
    for rank, h in enumerate(hits, 1):
        print(f"  #{rank}  score={h.score:.3f}  {h.payload}")

    client.delete_collection(COLL)  # tidy up; comment out to browse in the dashboard
    print("\ncleaned up. (comment out the delete to inspect at "
          "http://localhost:6333/dashboard)")


if __name__ == "__main__":
    main()
