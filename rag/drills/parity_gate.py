"""Gate 2 (Python half) — embedding parity: sentence-transformers vs fastembed-rs.

The Rust ingestion builds the index with fastembed (ONNX, CLS pooling, L2-norm);
the Python drills embed queries with sentence-transformers. They MUST land in the
same vector space (cosine >= 0.999) or the drills' "matched" baseline is itself
broken. Run the Rust gate first (writes data/parity/rust.json), then this.

Run from the rag/ dir:
    uv run --project drills python drills/parity_gate.py
"""
import json
import pathlib

import numpy as np
from sentence_transformers import SentenceTransformer

QI = "Represent this sentence for searching relevant passages:"
samples = json.loads(pathlib.Path("parity_samples.json").read_text())
model = SentenceTransformer("BAAI/bge-small-en-v1.5")


def embed(texts, is_query):
    inp = [f"{QI} {t}" for t in texts] if is_query else texts
    return model.encode(inp, normalize_embeddings=True)


py = {"passages": {}, "queries": {}}
for kind, is_q in (("passages", False), ("queries", True)):
    for t, v in zip(samples[kind], embed(samples[kind], is_q)):
        py[kind][t] = v.tolist()
pathlib.Path("data/parity").mkdir(parents=True, exist_ok=True)
pathlib.Path("data/parity/py.json").write_text(json.dumps(py))

rust = json.loads(pathlib.Path("data/parity/rust.json").read_text())


def cos(a, b):
    a, b = np.array(a), np.array(b)
    return float(a @ b / (np.linalg.norm(a) * np.linalg.norm(b)))


print(f"{'kind':<9}{'cosine(py,rust)':>17}  text")
worst = 1.0
for kind in ("passages", "queries"):
    for t in samples[kind]:
        c = cos(py[kind][t], rust[kind][t])
        worst = min(worst, c)
        print(f"{kind:<9}{c:>17.5f}  {t[:44]}")
print(f"\nworst cosine = {worst:.5f}  ->  "
      f"{'PASS (>= 0.999): same vector space' if worst >= 0.999 else 'FAIL: spaces differ'}")
