#!/usr/bin/env python3
"""What Temis says of the pairs of bench/recall: for each saved-again text and the memory it
repeats, paraphrases or only resembles, whether one keeps everything the other says
(`memory_preserved`, both ways) and how they stand to each other (`memory_relation`).
See results.md.

usage:  CLIO_MEMORY_TEMIS_URL=https://host/temis CLIO_MEMORY_TEMIS_TOKEN=... python3 bench/temis/run.py

The token goes to curl through its config on stdin, never on the command line. The texts sent
are the made-up ones of bench/recall.
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
URL = os.environ.get("CLIO_MEMORY_TEMIS_URL", "").rstrip("/")
TOKEN = os.environ.get("CLIO_MEMORY_TEMIS_TOKEN", "")

PRESERVED = {"se_conserva": {
    "type": "noul",
    "instructions": "Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras."}}
RELATION = {"relacion": {
    "type": "choice",
    "instructions": "Decide cómo se relaciona `propuesta` con `otra`.",
    "criteria": {
        "conflicts_with": "Se contradicen: no pueden ser ciertas a la vez.",
        "scoped": "`propuesta` vale en un caso más concreto que `otra` (este proyecto, este caso).",
        "compatible": "Hablan de lo mismo y pueden ser ciertas a la vez.",
        "related": "Tratan un tema cercano, sin más.",
        "ninguna": "No tienen que ver."}}}


def esc(s):
    return s.replace("\\", "\\\\").replace('"', '\\"')


def decide(state, questions):
    body = json.dumps({"model": "temis-latest", "state": state, "questions": questions}, ensure_ascii=False)
    config = "\n".join([
        f'url = "{esc(URL)}/v1/decide"', 'request = "POST"', "max-time = 20", "silent",
        'header = "Content-Type: application/json"', f'header = "Authorization: Bearer {esc(TOKEN)}"',
        f'data-binary = "{esc(body)}"', 'write-out = "\\n%{http_code}"', ""])
    out = subprocess.run(["curl", "--config", "-"], input=config, capture_output=True, text=True).stdout
    text, _, code = out.rpartition("\n")
    if code != "200":
        sys.exit(f"Temis answered {code}: {text[:200]}")
    return json.loads(text)["answers"]


def rows(name):
    with open(os.path.join(HERE, "..", "recall", name)) as f:
        return [json.loads(line) for line in f if line.strip()]


def main():
    if not URL or not TOKEN:
        sys.exit(__doc__)
    memories = {m["id"]: m["text"] for m in rows("memories.jsonl")}
    out = []
    for d in rows("duplicates.jsonl"):
        new, old = d["text"], memories[d["twin"]]
        in_old = decide({"original": new, "unido": old}, PRESERVED)["se_conserva"]["noul"]
        time.sleep(0.6)  # the server takes 120 requests a minute
        in_new = decide({"original": old, "unido": new}, PRESERVED)["se_conserva"]["noul"]
        time.sleep(0.6)
        rel = decide({"propuesta": new, "otra": old}, RELATION)["relacion"]
        time.sleep(0.6)
        out.append((d["type"], d["twin"], in_old, in_new, rel["choice"], rel["confidence"]))
    print("| tipo | gemela | lo nuevo está en lo viejo | lo viejo está en lo nuevo | relación (confianza) |")
    print("|---|---|---|---|---|")
    for kind, twin, a, b, choice, conf in out:
        print(f"| {kind} | m{twin} | {a:.2f} | {b:.2f} | {choice} ({conf:.2f}) |")
    print()
    print("| se dobla si algún sentido ≥ | exactas | paráfrasis | solo parecidas |")
    print("|---|---|---|---|")
    for floor in (0.5, 0.7, 0.8, 0.9, 0.95):
        cells = []
        for kind in ("exacta", "parafrasis", "relacionada"):
            of_kind = [r for r in out if r[0] == kind]
            cells.append(f"{sum(1 for r in of_kind if max(r[2], r[3]) >= floor)} de {len(of_kind)}")
        print(f"| {floor} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
