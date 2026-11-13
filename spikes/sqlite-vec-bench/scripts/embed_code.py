#!/usr/bin/env python3
"""Embed a subset of the corpus with nomic-embed-code (GGUF via local Ollama).

Picks NQ queries and a doc subset of ND docs that contains every picked
query's target, then writes into EMB_DIR:
  docs.code.f32 / queries.code.f32   u32 n, u32 dim, n*dim f32 LE (normalized)
  docs.code.ids / queries.code.ids   corpus ids, one per line, row order

usage: embed_code.py CORPUS EMB_DIR [nd] [nq]
"""
import json
import math
import pathlib
import random
import struct
import sys
import time
import urllib.request

MODEL = "hf.co/nomic-ai/nomic-embed-code-GGUF:Q4_K_M"
QUERY_PREFIX = "Represent this query for searching relevant code: "
URL = "http://127.0.0.1:11434/api/embed"


def embed(texts):
    body = json.dumps({"model": MODEL, "input": texts, "truncate": True}).encode()
    req = urllib.request.Request(URL, body, {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as r:
        return json.load(r)["embeddings"]


def normalized(v):
    n = math.sqrt(sum(x * x for x in v)) or 1.0
    return [x / n for x in v]


def write(path, rows):
    with open(path, "wb") as fh:
        fh.write(struct.pack("<II", len(rows), len(rows[0])))
        for r in rows:
            fh.write(struct.pack(f"<{len(r)}f", *r))


def run(texts, label, batch=16):
    out, start = [], time.time()
    for i in range(0, len(texts), batch):
        out.extend(normalized(v) for v in embed(texts[i : i + batch]))
        rate = len(out) / (time.time() - start)
        print(f"{label}: {len(out)}/{len(texts)} ({rate:.1f}/s)", file=sys.stderr, flush=True)
    secs = time.time() - start
    print(json.dumps({"embedded": label, "n": len(out), "secs": round(secs, 1), "per_sec": round(len(out) / secs, 2)}))
    return out


def main():
    corpus, emb = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    nd = int(sys.argv[3]) if len(sys.argv) > 3 else 5000
    nq = int(sys.argv[4]) if len(sys.argv) > 4 else 300
    emb.mkdir(parents=True, exist_ok=True)
    docs = [json.loads(l) for l in (corpus / "docs.jsonl").open()]
    queries = [json.loads(l) for l in (corpus / "queries.jsonl").open()][:nq]
    targets = {q["target"] for q in queries}
    rest = [d["id"] for d in docs if d["id"] not in targets]
    random.Random(3).shuffle(rest)
    dids = sorted(targets | set(rest[: nd - len(targets)]))

    qv = run([QUERY_PREFIX + q["text"] for q in queries], "queries.code")
    dv = run([docs[i]["text"] for i in dids], "docs.code")
    write(emb / "queries.code.f32", qv)
    write(emb / "docs.code.f32", dv)
    (emb / "queries.code.ids").write_text("".join(f"{q['id']}\n" for q in queries))
    (emb / "docs.code.ids").write_text("".join(f"{i}\n" for i in dids))


if __name__ == "__main__":
    main()
