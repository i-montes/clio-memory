"""A small local language model as the judge of relations (no Jev, no API).

For each memory's 5 nearest (by Granite vectors) it asks one closed question, the way Jev does:
how do these two stand? The answer is read from the probabilities of the options' letters, in one
pass, with no text generated. Then the same measures as run.py.

Run: uv run --python 3.12 --with sentence-transformers --with accelerate \
       bench/relations/llm_judge.py [Qwen/Qwen3-1.7B]
"""

import sys
import time

import numpy as np
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

import run as R

MODEL = sys.argv[1] if len(sys.argv) > 1 else "Qwen/Qwen3-1.7B"
OPTIONS = [
    ("A", "conflicts_with", "Se contradicen: no pueden ser ciertas a la vez."),
    ("B", "scoped", "Una vale en un caso más concreto que la otra (un proyecto, un caso)."),
    ("C", "compatible", "Hablan de lo mismo y pueden ser ciertas a la vez."),
    ("D", "related", "Son de un mismo asunto: quien lea una necesita o agradece la otra."),
    ("E", "none", "No tienen que ver: solo se parecen en palabras o en forma."),
]


def prompt(a, b):
    opts = "\n".join(f"{l}) {d}" for l, _, d in OPTIONS)
    return (
        "Dos recuerdos de la memoria de un asistente.\n"
        f"Recuerdo 1: {a}\nRecuerdo 2: {b}\n\n"
        f"¿Cómo se relacionan?\n{opts}\n\nResponde solo con la letra."
    )


def main():
    tok = AutoTokenizer.from_pretrained(MODEL)
    lm = AutoModelForCausalLM.from_pretrained(MODEL, dtype=torch.bfloat16, device_map="cuda")
    letters = [tok.encode(l, add_special_tokens=False)[0] for l, _, _ in OPTIONS]
    text = {m["id"]: m["text"] for m in R.MEMS}

    enc = R.embed()
    D = enc([m["text"] for m in R.MEMS])
    Q = enc([t["q"] for t in R.TASKS])
    S = D @ D.T
    cands = R.candidates(S, 5)
    nearest_of = {R.IDS[i]: frozenset((R.IDS[i], R.IDS[[j for j in np.argsort(-S[i]) if j != i][0]]))
                  for i in range(len(R.IDS))}

    probs = {}
    t0 = time.perf_counter()
    for e in cands:
        a, b = sorted(e)
        msgs = [{"role": "user", "content": prompt(text[a], text[b])}]
        chat = tok.apply_chat_template(msgs, add_generation_prompt=True, enable_thinking=False,
                                       tokenize=False)
        ids = tok(chat, return_tensors="pt").input_ids.to("cuda")
        with torch.no_grad():
            logits = lm(ids).logits[0, -1, letters].float()
        probs[e] = torch.softmax(logits, -1).cpu().numpy()
    per = (time.perf_counter() - t0) / len(cands) * 1000
    print(f"{MODEL}: {len(cands)} pares juzgados, {per:.0f} ms por par")

    related = {e: float(1 - p[4]) for e, p in probs.items()}
    kind = {e: OPTIONS[int(np.argmax(p))][1] for e, p in probs.items()}
    gold_kind = {frozenset((p["a"], p["b"])): p["rel"] for p in R.PAIRS}

    print("| enlaces | cuántos | precisión | cobertura | sueltas | tareas K=4 | K=6 | completas K=6 |")
    print("|---|---|---|---|---|---|---|---|")
    ways = {"1 más cercano (vectores)": R.knn_edges(S, 1)}
    for keep in (0.5, 0.7, 0.9):
        ways[f"modelo: relacionadas ≥ {keep}"] = R.judged(cands, related, keep, nearest_of)
    for name, edges in ways.items():
        r = R.edge_report(edges)
        nb = R.neighbors(edges)
        c4, _ = R.task_report(D, Q, nb, 4)
        c6, f6 = R.task_report(D, Q, nb, 6)
        print(f"| {name} | {r['links']} | {r['precision']:.0%} | {r['recall']:.0%} | "
              f"{r['loose']} | {c4:.0%} | {c6:.0%} | {f6:.0%} |")

    print("\ntipo dicho por el modelo para los pares buenos que vio:")
    for e, g in gold_kind.items():
        if e in kind and g != "related":
            print(f"  m{min(e)}–m{max(e)}: oro {g}, modelo {kind[e]} ({max(probs[e]):.2f})")
    said_conflict = [e for e in cands if kind[e] == "conflicts_with"]
    wrong = [e for e in said_conflict if gold_kind.get(e) != "conflicts_with"]
    print(f"\n'se contradicen' dicho {len(said_conflict)} veces; falsas: {len(wrong)}")
    for e in wrong[:8]:
        print(f"  m{min(e)}–m{max(e)}: {text[min(e)][:60]} | {text[max(e)][:60]}")


if __name__ == "__main__":
    main()
