#!/usr/bin/env python3
"""Real `clio-memory mcp` processes, as agents start them, against a TEMPORARY data folder:
what several sessions at once cost in memory and time. See results.md.

usage:
  run.py conc N [K]   N sessions at once, K searches each (default 15)
  run.py cold N       N sessions that never used the model save at the same moment
  run.py idle         one session: memory before, after a search, 110 s without use, then until the daemon leaves

CLIO_BENCH_BIN picks the binary (default: ~/.local/bin/clio-memory). The model is the one
installed in ~/.cache/clio-models; the memories and questions are those of bench/recall.
"""
import json
import os
import subprocess
import sys
import tempfile
import threading
import time

BIN = os.path.abspath(os.environ.get("CLIO_BENCH_BIN") or os.path.expanduser("~/.local/bin/clio-memory"))
RECALL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "recall")


def status(pid):
    out = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            k, _, v = line.partition(":")
            if k in ("VmRSS", "VmHWM", "Threads"):
                out[k] = int(v.split()[0])
    return out


def daemons(data):
    """The pids of the daemons serving `data` (none before v0.10)."""
    out = subprocess.run(["pgrep", "-f", "--", f"--db {data}/memory.db daemon"], capture_output=True, text=True).stdout
    return [int(p) for p in out.split()]


class Mcp:
    def __init__(self, data, cwd, name):
        env = dict(os.environ, CLIO_MEMORY_DATA=data)
        self.p = subprocess.Popen([BIN, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, cwd=cwd, env=env, text=True)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": name, "version": "0"}})

    def rpc(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def call(self, tool, args):
        t = time.perf_counter()
        r = self.rpc("tools/call", {"name": tool, "arguments": args})
        ms = (time.perf_counter() - t) * 1000
        res = r.get("result", {})
        return ms, res.get("isError", True), (res.get("content") or [{}])[0].get("text", str(r))

    def close(self):
        self.p.stdin.close()
        self.p.wait()


def rows(name):
    with open(os.path.join(RECALL, name)) as f:
        return [json.loads(line) for line in f if line.strip()]


def folders():
    return tempfile.mkdtemp(prefix="cm-data-"), tempfile.mkdtemp(prefix="cm-proj-")


def seed(data, cwd, n):
    mems = rows("memories.jsonl")[:n]
    for i in range(0, len(mems), 30):  # a session saves 30 times
        m = Mcp(data, cwd, "seed")
        for r in mems[i:i + 30]:
            m.call("memory_save", {"text": r["text"], "kind": "semantic", "title": r.get("title", "")[:120]})
        m.close()


def pct(v, p):
    v = sorted(v)
    return v[min(len(v) - 1, int(len(v) * p))]


def memory_line(data, sessions):
    per = [status(s.p.pid) for s in sessions]
    line = f"mcp processes: {sum(p['VmRSS'] for p in per) // 1024} MB in all (high-water {max(p['VmHWM'] for p in per) // 1024} MB each)"
    for pid in daemons(data):
        try:
            d = status(pid)
        except OSError:  # it left between pgrep and the read
            continue
        line += f" · daemon {pid}: {d['VmRSS'] // 1024} MB (high-water {d['VmHWM'] // 1024} MB)"
    return line


def conc(n, k):
    data, cwd = folders()
    seed(data, cwd, 90)
    qs = [q["q"] for q in rows("queries.jsonl")]
    sessions = [Mcp(data, cwd, f"s{i}") for i in range(n)]
    first, rest, errs = [], [], []

    def work(i, s):
        for j in range(k):
            ms, err, text = s.call("memory_search", {"query": qs[(i * 31 + j) % len(qs)][:200]})
            (first if j == 0 else rest).append(ms)
            if err:
                errs.append(text[:120])

    t = time.perf_counter()
    threads = [threading.Thread(target=work, args=(i, s)) for i, s in enumerate(sessions)]
    [x.start() for x in threads]
    [x.join() for x in threads]
    wall = time.perf_counter() - t
    print(f"{n} sessions x {k} searches: {wall:.1f} s, {n * k / wall:.1f} searches/s")
    print(f"  first search: p50 {pct(first, .5):.0f} ms, max {max(first):.0f} ms")
    print(f"  later searches: p50 {pct(rest, .5):.0f} ms, p95 {pct(rest, .95):.0f} ms, max {max(rest):.0f} ms")
    print(f"  {memory_line(data, sessions)}")
    print(f"  errors: {len(errs)} {errs[:2]}")
    [s.close() for s in sessions]


def cold(n):
    data, cwd = folders()
    mems = rows("memories.jsonl")
    seed(data, cwd, 1)
    sessions = [Mcp(data, cwd, f"s{i}") for i in range(n)]
    out = [None] * n
    go = threading.Event()

    def work(i):
        go.wait()
        out[i] = sessions[i].call("memory_save", {"text": mems[10 + i]["text"], "kind": "semantic"})

    threads = [threading.Thread(target=work, args=(i,)) for i in range(n)]
    [x.start() for x in threads]
    go.set()
    [x.join() for x in threads]
    refused = [o for o in out if o[1]]
    print(f"{n} cold sessions saving at once: {n - len(refused)} saved, {len(refused)} refused")
    print(f"  ms: {sorted(round(o[0]) for o in out)}")
    for o in refused[:1]:
        print(f"  refused with: {o[2][:120]}")
    print(f"  {memory_line(data, sessions)}")
    [s.close() for s in sessions]


def idle():
    data, cwd = folders()
    seed(data, cwd, 30)
    m = Mcp(data, cwd, "idle")

    def show(what):
        print(f"{what:22} {memory_line(data, [m])}")

    show("after initialize")
    ms, _, _ = m.call("memory_search", {"query": "¿qué base de datos usa la tienda?"})
    show(f"1st search {ms:5.0f} ms")
    ms, _, _ = m.call("memory_search", {"query": "¿cómo se despliega la aplicación?"})
    show(f"2nd search {ms:5.0f} ms")
    t0 = time.time()
    for wait in (30, 60, 75, 110):
        time.sleep(max(0, wait - (time.time() - t0)))
        show(f"idle +{wait} s")
    ms, _, _ = m.call("memory_search", {"query": "¿quién es la persona?"})
    show(f"search after {ms:5.0f} ms")
    m.close()
    # v0.10: the daemon leaves by itself after 600 s without requests; watch it go (12 min at most)
    t0 = time.time()
    while daemons(data) and time.time() - t0 < 720:
        time.sleep(15)
        for pid in daemons(data):
            try:
                print(f"session closed +{time.time() - t0:3.0f} s: daemon {pid}: {status(pid)['VmRSS'] // 1024} MB", flush=True)
            except OSError:  # it left between pgrep and the read
                pass
    print(f"daemons left after {time.time() - t0:.0f} s without sessions: {len(daemons(data))}")


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "conc":
        conc(int(sys.argv[2]), int(sys.argv[3]) if len(sys.argv) > 3 else 15)
    elif mode == "cold":
        cold(int(sys.argv[2]))
    elif mode == "idle":
        idle()
    else:
        sys.exit(__doc__)
