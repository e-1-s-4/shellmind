#!/usr/bin/env python3
"""Batch-embed the shellmind history index from Python.

The Rust daemon embeds history in the background automatically; this
script exists for one-off jobs (re-embedding after switching models,
benchmarking, CI smoke tests).

Usage:
    python3 embeddings.py                    # embed missing rows for the default model
    python3 embeddings.py --model nomic-embed-text --limit 500 --db ~/.config/shellmind/history.db
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from pathlib import Path

from ollama import Ollama

DEFAULT_DB = Path.home() / ".config" / "shellmind" / "history.db"
DEFAULT_MODEL = "nomic-embed-text"
BATCH = 64


def encode(vector: list[float]) -> bytes:
    import struct

    return struct.pack(f"<{len(vector)}f", *vector)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", type=Path, default=DEFAULT_DB)
    ap.add_argument("--model", default=DEFAULT_MODEL)
    ap.add_argument("--host", default="http://localhost:11434")
    ap.add_argument("--limit", type=int, default=0, help="max rows to embed (0 = all)")
    args = ap.parse_args()

    if not args.db.exists():
        print(f"no database at {args.db} — run `sm index` first", file=sys.stderr)
        return 1

    client = Ollama(host=args.host)
    if not client.ping():
        print("ollama is not reachable — start it first (ollama serve)", file=sys.stderr)
        return 1

    conn = sqlite3.connect(args.db)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS embeddings ("
        "history_id INTEGER PRIMARY KEY, model TEXT NOT NULL, "
        "dim INTEGER NOT NULL, vector BLOB NOT NULL)"
    )

    sql = (
        "SELECT h.id, h.command FROM history h "
        "LEFT JOIN embeddings e ON e.history_id = h.id AND e.model = ? "
        "WHERE h.secret = 0 AND e.history_id IS NULL "
        "ORDER BY h.uses DESC, h.ts DESC"
    )
    if args.limit:
        sql += f" LIMIT {int(args.limit)}"

    rows = conn.execute(sql, (args.model,)).fetchall()
    if not rows:
        print("nothing to embed — index is up to date")
        return 0

    print(f"embedding {len(rows)} commands with {args.model} …")
    done = 0
    for start in range(0, len(rows), BATCH):
        batch = rows[start : start + BATCH]
        vectors = client.embed(args.model, [cmd for _, cmd in batch])
        for (row_id, _), vector in zip(batch, vectors):
            conn.execute(
                "INSERT INTO embeddings(history_id, model, dim, vector) VALUES(?,?,?,?) "
                "ON CONFLICT(history_id) DO UPDATE SET "
                "model=excluded.model, dim=excluded.dim, vector=excluded.vector",
                (row_id, args.model, len(vector), encode(vector)),
            )
        done += len(batch)
        print(f"  {done}/{len(rows)}")

    conn.commit()
    conn.close()
    print("done — hybrid history search is now active for these commands")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
