"""Reference vectors for clio-memory's Granite embedder (tests/fixtures/granite_reference.json):
sentence-transformers in float32, CLS pooling, first 384 components, L2-normalized.

Run: uv run --python 3.12 --no-project --with sentence-transformers --with numpy bench/embeddings/reference.py
"""

import json
from pathlib import Path

import numpy as np
import torch
from sentence_transformers import SentenceTransformer

MODEL = "ibm-granite/granite-embedding-311m-multilingual-r2"
REVISION = "44399559930365213510b1ee2eb15ded83374f0e"
TEXTS = [
    "Qué: el proyecto usa PostgreSQL 16 para guardar los pedidos · Por qué: soporte de JSON",
    "¿qué base de datos guarda las órdenes de compra?",
    "la persona prefiere respuestas cortas, en español",
]

m = SentenceTransformer(MODEL, revision=REVISION, device="cpu", model_kwargs={"torch_dtype": torch.float32})
v = np.asarray(m.encode(TEXTS, normalize_embeddings=False))[:, :384]
v = v / np.linalg.norm(v, axis=1, keepdims=True)
out = Path(__file__).parents[2] / "tests/fixtures/granite_reference.json"
out.write_text(json.dumps([{"text": t, "vector": [round(float(x), 7) for x in row]} for t, row in zip(TEXTS, v)]))
print("wrote", out)
