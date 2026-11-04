"""Compares embedding models on Spanish memories searched by meaning, not by words.

Each query is a paraphrase of one memory (sometimes two). For every model it measures: hit@1,
hit@5 and MRR (how high the right memory comes), the time to embed one memory (what a save
costs) on CPU and, when there is one, on the GPU, and the time to load the model.

Run: uv run --python 3.12 --with sentence-transformers --with model2vec bench/embeddings/run.py
"""

import json
import os
import statistics
import sys
import time
from pathlib import Path

import numpy as np

HERE = Path(__file__).parent
MEMORIES = [json.loads(l) for l in (HERE / "memories.jsonl").read_text().splitlines() if l.strip()]
QUERIES = [json.loads(l) for l in (HERE / "queries.jsonl").read_text().splitlines() if l.strip()]
POTION = Path.home() / ".cache/clio-models/potion-multilingual-128M"

# (label, loader kind, model id, dimensions to keep or None, query prompt name or None)
MODELS = [
    ("potion-multilingual-128M (actual)", "model2vec", str(POTION), None, None),
    ("granite-97m-multilingual-r2", "st", "ibm-granite/granite-embedding-97m-multilingual-r2", None, None),
    ("granite-311m-multilingual-r2 · 768", "st", "ibm-granite/granite-embedding-311m-multilingual-r2", None, None),
    ("granite-311m-multilingual-r2 · 384", "st", "ibm-granite/granite-embedding-311m-multilingual-r2", 384, None),
    ("granite-311m-multilingual-r2 · 256", "st", "ibm-granite/granite-embedding-311m-multilingual-r2", 256, None),
    ("Qwen3-Embedding-0.6B (referencia)", "st", "Qwen/Qwen3-Embedding-0.6B", None, "query"),
]


def load(kind, name, device):
    t = time.perf_counter()
    if kind == "model2vec":
        from model2vec import StaticModel

        m = StaticModel.from_pretrained(name)
        enc = lambda texts, query=False: np.asarray(m.encode(texts))
    else:
        from sentence_transformers import SentenceTransformer

        m = SentenceTransformer(name, device=device)
        def enc(texts, query=False, _m=m):
            kw = {"prompt_name": PROMPT} if (query and PROMPT) else {}
            return np.asarray(_m.encode(texts, normalize_embeddings=True, **kw))
    return enc, time.perf_counter() - t


def evaluate(enc, dims):
    docs = enc([m["text"] for m in MEMORIES])
    qs = enc([q["q"] for q in QUERIES], query=True)
    if dims:
        docs, qs = docs[:, :dims], qs[:, :dims]
    docs = docs / np.linalg.norm(docs, axis=1, keepdims=True)
    qs = qs / np.linalg.norm(qs, axis=1, keepdims=True)
    ids = [m["id"] for m in MEMORIES]
    hit1 = hit5 = rr = 0.0
    misses = []
    for q, v in zip(QUERIES, qs):
        order = [ids[i] for i in np.argsort(-(docs @ v))]
        rank = min(order.index(g) for g in q["gold"]) + 1
        hit1 += rank == 1
        hit5 += rank <= 5
        rr += 1 / rank
        if rank > 1:
            misses.append((q["q"], rank, order[0]))
    n = len(QUERIES)
    return hit1 / n, hit5 / n, rr / n, docs.shape[1], misses


def per_memory_ms(enc):
    text = MEMORIES[8]["text"]
    enc([text])  # warm up
    times = []
    for _ in range(30):
        t = time.perf_counter()
        enc([text])
        times.append((time.perf_counter() - t) * 1000)
    return statistics.median(times)


def main():
    try:
        import torch

        gpu = torch.cuda.is_available()
        gpu_name = torch.cuda.get_device_name(0) if gpu else None
    except ImportError:
        gpu, gpu_name = False, None
    print(f"{len(MEMORIES)} recuerdos, {len(QUERIES)} preguntas · GPU: {gpu_name or 'no'}\n")
    rows, misses_by_model = [], {}
    global PROMPT
    for label, kind, name, dims, prompt in MODELS:
        PROMPT = prompt
        enc_cpu, load_cpu = load(kind, name, "cpu")
        h1, h5, mrr, d, misses = evaluate(enc_cpu, dims)
        cpu_ms = per_memory_ms(enc_cpu)
        gpu_ms = load_gpu = None
        if gpu and kind == "st":
            enc_gpu, load_gpu = load(kind, name, "cuda")
            gpu_ms = per_memory_ms(enc_gpu)
            del enc_gpu
            torch.cuda.empty_cache()
        rows.append((label, d, h1, h5, mrr, cpu_ms, gpu_ms, load_cpu))
        misses_by_model[label] = misses
        print(f"  {label}: hit@1 {h1:.0%}, hit@5 {h5:.0%}, MRR {mrr:.3f}, CPU {cpu_ms:.1f} ms"
              + (f", GPU {gpu_ms:.1f} ms" if gpu_ms else ""), file=sys.stderr)
    print("| Modelo | Dim. | Acierto 1.º | Entre los 5 primeros | MRR | ms por recuerdo (CPU) | ms por recuerdo (GPU) | Carga (s) |")
    print("|---|---|---|---|---|---|---|---|")
    for label, d, h1, h5, mrr, cpu_ms, gpu_ms, load_s in rows:
        g = f"{gpu_ms:.1f}" if gpu_ms else "—"
        print(f"| {label} | {d} | {h1:.0%} | {h5:.0%} | {mrr:.3f} | {cpu_ms:.1f} | {g} | {load_s:.1f} |")
    print("\nFallos (pregunta · puesto de la respuesta correcta · lo que salió primero):")
    for label, misses in misses_by_model.items():
        print(f"\n{label}: {len(misses)}")
        for q, rank, top in misses:
            print(f"  - «{q}» · puesto {rank} · salió m{top}")


PROMPT = None
if __name__ == "__main__":
    main()
