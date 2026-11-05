"""Relations between memories without a judge: are they right, and do they help to explore?

Memories: the 60 of bench/embeddings plus 4 that contradict or narrow some of them (extra.jsonl).
Gold: pairs.jsonl, the pairs a person would link (with their kind), and tasks.jsonl, questions whose
answer needs several memories at once.

For each way of linking (k nearest, a similarity floor with the nearest always kept) it measures:
- precision and recall of the links against the gold pairs, and how many memories stay loose;
- whether each contradicting pair is among the candidates a judge would be shown;
- for the tasks, how much of what is needed a reader reaches with a budget of memories read:
  plain search (the K nearest to the question) against search of 2 and then following links.

Run: uv run --python 3.12 --with sentence-transformers bench/relations/run.py
"""

import json
from pathlib import Path

import numpy as np

HERE = Path(__file__).parent
EMB = HERE.parent / "embeddings"


def jsonl(p):
    return [json.loads(l) for l in p.read_text().splitlines() if l.strip()]


MEMS = jsonl(EMB / "memories.jsonl") + jsonl(HERE / "extra.jsonl")
PAIRS = jsonl(HERE / "pairs.jsonl")
TASKS = jsonl(HERE / "tasks.jsonl")
IDS = [m["id"] for m in MEMS]
IX = {i: n for n, i in enumerate(IDS)}
GOLD = {frozenset((p["a"], p["b"])) for p in PAIRS}
CONFLICTS = [p for p in PAIRS if p["rel"] == "conflicts_with"]


def embed():
    from sentence_transformers import SentenceTransformer

    m = SentenceTransformer("ibm-granite/granite-embedding-311m-multilingual-r2", device="cpu")

    def enc(texts):
        v = np.asarray(m.encode(texts, normalize_embeddings=True))[:, :384]
        return v / np.linalg.norm(v, axis=1, keepdims=True)

    return enc


def knn_edges(S, k):
    edges = {}
    for i in range(len(IDS)):
        order = [j for j in np.argsort(-S[i]) if j != i][:k]
        for j in order:
            edges[frozenset((IDS[i], IDS[j]))] = float(S[i, j])
    return edges


def floor_edges(S, tau, cap):
    """Every pair at or above tau (at most `cap` per memory), plus each memory's nearest."""
    edges = {}
    for i in range(len(IDS)):
        order = [j for j in np.argsort(-S[i]) if j != i]
        keep = [j for j in order[:cap] if S[i, j] >= tau] or order[:1]
        for j in keep:
            edges[frozenset((IDS[i], IDS[j]))] = float(S[i, j])
    return edges


def csls(S, k=5):
    """Similarity minus how close each memory is to everything (its mean to its k nearest): a
    memory near all others stops being everyone's neighbor."""
    r = np.sort(S - np.eye(len(S)) * 9, axis=1)[:, -k:].mean(axis=1)
    return 2 * S - r[:, None] - r[None, :]


def mutual_edges(S, k):
    """Links both sides choose among their k nearest, plus each memory's nearest."""
    top = [set([j for j in np.argsort(-S[i]) if j != i][:k]) for i in range(len(IDS))]
    edges = {}
    for i in range(len(IDS)):
        near = [j for j in np.argsort(-S[i]) if j != i]
        keep = [j for j in top[i] if i in top[j]] or near[:1]
        for j in keep:
            edges[frozenset((IDS[i], IDS[j]))] = float(S[i, j])
    return edges


STOP = set("""a al algo ante antes como con cada de del donde el ella ellas en entre era es esa ese
eso esta este esto la las le lo los más mas no nos o para pero por porque que qué se si sin sobre
su sus también un una uno y ya ser está están hace hacer debe quiere usa usar va van vez cuando
antes después qué cómo dónde por arreglo causa error aprendido decisión configuración preferencia
procedimiento quién""".split())


def words(t):
    import unicodedata
    t = unicodedata.normalize("NFKD", t.lower())
    t = "".join(c for c in t if not unicodedata.combining(c))
    toks = "".join(c if c.isalnum() else " " for c in t).split()
    return {w[:6] for w in toks if len(w) > 2 and w not in STOP}


def word_sim():
    """TF-IDF cosine over word stems: shared rare words (restic, pedidos, webhooks)."""
    W = [words(m["text"]) for m in MEMS]
    df = {}
    for ws in W:
        for w in ws:
            df[w] = df.get(w, 0) + 1
    n = len(W)
    vec = [{w: np.log(n / df[w]) for w in ws} for ws in W]
    norm = [np.sqrt(sum(v * v for v in d.values())) or 1.0 for d in vec]
    S = np.zeros((n, n))
    for i in range(n):
        for j in range(n):
            if i != j:
                S[i, j] = sum(vec[i][w] * vec[j].get(w, 0) for w in vec[i]) / (norm[i] * norm[j])
    return S


def union(*es):
    out = {}
    for e in es:
        for k, v in e.items():
            out[k] = max(out.get(k, 0), v)
    return out


def candidates(S, k=5):
    """What a judge is shown for each memory: its k nearest."""
    out = {}
    for i in range(len(IDS)):
        for j in [j for j in np.argsort(-S[i]) if j != i][:k]:
            out[frozenset((IDS[i], IDS[j]))] = float(S[i, j])
    return out


def judged(cands, score, keep, nearest_of):
    """Links the judge keeps (score >= keep), plus each memory's nearest so none is loose."""
    edges = {e: score[e] for e in cands if score[e] >= keep}
    linked = {i for e in edges for i in e}
    for i, e in nearest_of.items():
        if i not in linked:
            edges[e] = score.get(e, 0.0)
    return edges


def reranker_scores(pairs):
    from sentence_transformers import CrossEncoder

    ce = CrossEncoder("BAAI/bge-reranker-v2-m3", device="cpu")
    text = {m["id"]: m["text"] for m in MEMS}
    ps = list(pairs)
    a = ce.predict([(text[min(e)], text[max(e)]) for e in ps], batch_size=16)
    b = ce.predict([(text[max(e)], text[min(e)]) for e in ps], batch_size=16)
    return {e: float(max(x, y)) for e, x, y in zip(ps, a, b)}


def edge_report(edges):
    got = set(edges)
    tp = len(got & GOLD)
    linked = {i for e in got for i in e}
    return {
        "links": len(got),
        "precision": tp / len(got) if got else 0.0,
        "recall": tp / len(GOLD),
        "loose": len(IDS) - len(linked),
    }


def neighbors(edges):
    nb = {i: [] for i in IDS}
    for e, w in edges.items():
        a, b = tuple(e)
        nb[a].append((w, b))
        nb[b].append((w, a))
    for i in nb:
        nb[i].sort(reverse=True)
    return nb


def explore(qv, D, nb, budget, seeds=2):
    """Search `seeds`, then read the strongest links of what was read, breadth first, until the
    budget; when the links run out, the next search results."""
    by_sim = [IDS[j] for j in np.argsort(-(D @ qv))]
    read, frontier = [], by_sim[:seeds]
    read.extend(frontier)
    while len(read) < budget:
        nxt = [b for i in frontier for _, b in nb[i] if b not in read]
        if not nxt:
            nxt = [i for i in by_sim if i not in read][:1]
        frontier = []
        for b in nxt:
            if len(read) >= budget:
                break
            if b not in read:
                read.append(b)
                frontier.append(b)
    return read


def reader_report(D, Q, S, budget, fanout, seeds=2, smart=True):
    """A reader that sees, with each memory it opens, the titles of its `fanout` nearest, and
    opens next what serves its question (smart: what the task needs; else the most similar);
    when nothing it sees serves, the next search result."""
    cover = full = 0.0
    for t, qv in zip(TASKS, Q):
        need = set(t["need"])
        by_sim = [IDS[j] for j in np.argsort(-(D @ qv))]
        read = by_sim[:seeds]
        while len(read) < budget:
            seen = [IDS[j] for i in read for j in np.argsort(-S[IX[i]])[1:fanout + 1]]
            seen = [i for i in dict.fromkeys(seen) if i not in read]
            if smart:
                pick = [i for i in seen if i in need]
            else:  # the neighbor most like the question, without understanding it
                pick = sorted(seen, key=lambda i: -float(D[IX[i]] @ qv))[:1]
            nxt = pick[0] if pick else next(i for i in by_sim if i not in read)
            read.append(nxt)
        got = len(set(read) & need)
        cover += got / len(need)
        full += got == len(need)
    return cover / len(TASKS), full / len(TASKS)


def task_report(D, Q, nb, budget):
    cover = full = 0.0
    for t, qv in zip(TASKS, Q):
        if nb is None:
            read = [IDS[j] for j in np.argsort(-(D @ qv))[:budget]]
        else:
            read = explore(qv, D, nb, budget)
        got = len(set(read) & set(t["need"]))
        cover += got / len(t["need"])
        full += got == len(t["need"])
    return cover / len(TASKS), full / len(TASKS)


def main():
    enc = embed()
    D = enc([m["text"] for m in MEMS])
    Q = enc([t["q"] for t in TASKS])
    S = D @ D.T
    gold_sims = sorted(float(S[IX[p["a"]], IX[p["b"]]]) for p in PAIRS)
    off = [float(S[i, j]) for i in range(len(IDS)) for j in range(i + 1, len(IDS))
           if frozenset((IDS[i], IDS[j])) not in GOLD]
    print(f"similitud de los pares buenos: mediana {np.median(gold_sims):.3f}, "
          f"mínimo {gold_sims[0]:.3f}; del resto: mediana {np.median(off):.3f}, "
          f"p95 {np.percentile(off, 95):.3f}, p99 {np.percentile(off, 99):.3f}\n")

    ways = {f"{k} más cercanos": knn_edges(S, k) for k in (1, 2, 3)}
    for tau in (0.82, 0.85, 0.88):
        ways[f"piso {tau:.2f} (máx 3) + el más cercano"] = floor_edges(S, tau, 3)
    C = csls(S)
    for k in (1, 2):
        ways[f"CSLS {k} más cercanos"] = knn_edges(C, k)
    for k in (2, 3):
        ways[f"mutuos entre {k} + el más cercano"] = mutual_edges(S, k)
        ways[f"CSLS mutuos entre {k} + el más cercano"] = mutual_edges(C, k)
    Wd = word_sim()
    ways["palabras 1 más cercano"] = knn_edges(Wd, 1)
    ways["vectores 1 + palabras 1"] = union(knn_edges(S, 1), knn_edges(Wd, 1))
    ways["vectores 2 + palabras 1"] = union(knn_edges(S, 2), knn_edges(Wd, 1))
    ways["CSLS mutuos 3 + palabras 1"] = union(mutual_edges(C, 3), knn_edges(Wd, 1))
    cands = candidates(S, 5)
    nearest_of = {IDS[i]: frozenset((IDS[i], IDS[[j for j in np.argsort(-S[i]) if j != i][0]]))
                  for i in range(len(IDS))}
    print(f"candidatos (5 vecinos): {len(cands)}, de ellos buenos "
          f"{len(set(cands) & GOLD)}/{len(GOLD)}")
    oracle = {e: (1.0 if e in GOLD else 0.0) for e in cands}
    ways["juez perfecto sobre 5 vecinos"] = judged(cands, oracle, 0.5, nearest_of)
    rr = reranker_scores(cands) if "--reranker" in __import__("sys").argv else {e: 0.0 for e in cands}
    good = sorted(rr[e] for e in cands if e in GOLD)
    bad = sorted(rr[e] for e in cands if e not in GOLD)
    print(f"reranker: buenos mediana {np.median(good):.3f}; malos mediana {np.median(bad):.3f}, "
          f"p90 {np.percentile(bad, 90):.3f}")
    for keep in (0.001, 0.003, 0.01):
        ways[f"reranker ≥ {keep} sobre 5 vecinos"] = judged(cands, rr, keep, nearest_of)
    for k in (1, 2):
        edges = {}
        for i in IDS:
            mine = sorted((rr[e], e) for e in cands if i in e)[::-1][:k]
            for sc, e in mine:
                edges[e] = sc
        ways[f"reranker: los {k} mejores de 5 vecinos"] = edges
    ways["oro (a mano)"] = {e: float(S[IX[min(e)], IX[max(e)]]) for e in GOLD}

    print("| enlaces | cuántos | precisión | cobertura | sueltas | conflictos a la vista | "
          "tareas K=4 | K=6 | completas K=6 |")
    print("|---|---|---|---|---|---|---|---|---|")
    base4, _ = task_report(D, Q, None, 4)
    base6, full6 = task_report(D, Q, None, 6)
    print(f"| solo búsqueda | – | – | – | – | – | {base4:.0%} | {base6:.0%} | {full6:.0%} |")
    for name, edges in ways.items():
        r = edge_report(edges)
        seen = sum(frozenset((c["a"], c["b"])) in edges for c in CONFLICTS)
        nb = neighbors(edges)
        c4, _ = task_report(D, Q, nb, 4)
        c6, f6 = task_report(D, Q, nb, 6)
        print(f"| {name} | {r['links']} | {r['precision']:.0%} | {r['recall']:.0%} | "
              f"{r['loose']} | {seen}/{len(CONFLICTS)} | {c4:.0%} | {c6:.0%} | {f6:.0%} |")

    print("\nlector que ve los vecinos de lo que abre (sin guardar relaciones):")
    print("| vecinos a la vista | tareas K=4 | K=6 | completas K=6 |")
    print("|---|---|---|---|")
    for f in (3, 5, 8):
        c4, _ = reader_report(D, Q, S, 4, f)
        c6, f6 = reader_report(D, Q, S, 6, f)
        print(f"| {f} | {c4:.0%} | {c6:.0%} | {f6:.0%} |")
    for f in (5, 8):
        c4, _ = reader_report(D, Q, S, 4, f, smart=False)
        c6, f6 = reader_report(D, Q, S, 6, f, smart=False)
        print(f"| {f}, lector torpe (el más parecido a la pregunta) | {c4:.0%} | {c6:.0%} | {f6:.0%} |")

    print("\nconflictos: rango del otro entre los vecinos de cada uno")
    for c in CONFLICTS:
        a, b = IX[c["a"]], IX[c["b"]]
        rank = [IDS[j] for j in np.argsort(-S[a]) if j != a].index(c["b"]) + 1
        print(f"  m{c['a']} ↔ m{c['b']}: similitud {S[a, b]:.3f}, puesto {rank}")


if __name__ == "__main__":
    main()
