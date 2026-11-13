#!/usr/bin/env python3
"""Build the benchmark corpus from local Claude Code transcripts.

Writes, into OUT_DIR (keep it outside the repo: it holds private text):
  docs.jsonl     {"id", "project", "text"}      assistant text chunks
  queries.jsonl  {"id", "project", "text", "target"}  a user prompt and the
                 doc id of the first chunk of the assistant's reply to it

The retrieval task is "given the prompt, find the reply", a proxy for memory
recall that needs no hand labels. Queries are never in docs.

usage: build_corpus.py PROJECTS_DIR OUT_DIR [max_docs] [max_queries]
"""
import hashlib
import json
import pathlib
import random
import sys

CHUNK = 1200  # characters, roughly 250-300 tokens
MIN_LEN = 80


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(
            b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"
        )
    return ""


def is_prompt(content):
    # A real prompt, not a tool result or a harness-injected block.
    if isinstance(content, list) and any(
        isinstance(b, dict) and b.get("type") == "tool_result" for b in content
    ):
        return False
    t = text_of(content).strip()
    return MIN_LEN <= len(t) <= 2000 and not t.startswith("<")


def chunks(text):
    text = text.strip()
    out, cur = [], ""
    for para in text.split("\n\n"):
        if len(cur) + len(para) + 2 > CHUNK and cur:
            out.append(cur)
            cur = ""
        while len(para) > CHUNK:
            out.append(para[:CHUNK])
            para = para[CHUNK:]
        cur = f"{cur}\n\n{para}" if cur else para
    if cur:
        out.append(cur)
    return [c for c in out if len(c) >= MIN_LEN]


def main():
    src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    max_docs = int(sys.argv[3]) if len(sys.argv) > 3 else 50_000
    max_queries = int(sys.argv[4]) if len(sys.argv) > 4 else 1_000
    out.mkdir(parents=True, exist_ok=True)

    docs, queries, seen = [], [], set()
    files = sorted(src.glob("**/*.jsonl"))  # subagent transcripts sit deeper
    random.Random(7).shuffle(files)
    for f in files:
        project = hashlib.sha1(f.relative_to(src).parts[0].encode()).hexdigest()[:8]
        pending = None  # prompt waiting for its reply
        for line in f.open(errors="ignore"):
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            kind, msg = rec.get("type"), rec.get("message") or {}
            content = msg.get("content")
            if kind == "user":
                pending = text_of(content).strip() if is_prompt(content) else pending
            elif kind == "assistant":
                for c in chunks(text_of(content)):
                    h = hashlib.sha1(c.encode()).hexdigest()
                    if h in seen:
                        continue
                    seen.add(h)
                    doc_id = len(docs)
                    docs.append({"id": doc_id, "project": project, "text": c})
                    if pending:
                        queries.append(
                            {"id": len(queries), "project": project, "text": pending, "target": doc_id}
                        )
                        pending = None
            if len(docs) >= max_docs:
                break
        if len(docs) >= max_docs:
            break

    random.Random(11).shuffle(queries)
    queries = queries[:max_queries]
    for i, q in enumerate(queries):
        q["id"] = i
    with (out / "docs.jsonl").open("w") as fh:
        fh.writelines(json.dumps(d) + "\n" for d in docs)
    with (out / "queries.jsonl").open("w") as fh:
        fh.writelines(json.dumps(q) + "\n" for q in queries)
    projects = len({d["project"] for d in docs})
    print(f"docs={len(docs)} queries={len(queries)} projects={projects} files_read<= {len(files)}")


if __name__ == "__main__":
    main()
