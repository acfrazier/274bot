#!/usr/bin/env python3
"""Read-only miner for VialFiller east/west B1 catalog logs."""
from __future__ import annotations

import hashlib
import json
import os
import re
from collections import Counter

BASE = os.path.join(
    os.path.dirname(__file__),
    "..",
    "catalog-harness",
    "live",
)
OUT = os.path.dirname(__file__)

FILES = [
    "r274-vial-filler-east-100adccc-b1cff8a7.log",
    "r289-vial-filler-east-100adccc-b1cff8a7.log",
    "r274-vial-filler-100adccc-b1cff8a7.log",
    "r289-vial-filler-100adccc-b1cff8a7.log",
]

INTEREST = re.compile(
    r"FAIL|fail|timeout|Timeout|could not open|Bank booth|Use-quickly|"
    r"open-booth|open_booth|openBooth|walk-near|walk_near|Interact|"
    r"InvalidAction|Invalid|has_item|ItemId|deadline|scene_state|"
    r"filled |Fountain|Vial|water|deposit|withdraw|generation|"
    r"3010|3011|3013|3352|3354|3355|2946|3369|2949|3381|"
    r"banking|retry|approach|Loc |loc id|opcode|op=|"
    r"here=|tile=|at \[|WalkTo|walk to|Traversal|"
    r"door|Door|booth|Booth|nearest|script|assert|criterion",
    re.I,
)

TILE_RE = re.compile(r"\[(\d+),\s*(\d+),\s*(\d+)\]")
XYZ_RE = re.compile(r"\((\d+),\s*(\d+)(?:,\s*(\d+))?\)")
INV_RE = re.compile(r"(?:inv|held|inventory)[^\n]{0,80}", re.I)


def summarize(path: str) -> dict:
    raw = open(path, "rb").read()
    text = raw.decode("utf-8", errors="replace")
    lines = text.splitlines()
    hits = []
    tiles = Counter()
    for i, line in enumerate(lines, 1):
        if INTEREST.search(line):
            hits.append({"n": i, "line": line[:400]})
        for m in TILE_RE.finditer(line):
            tiles[(int(m.group(1)), int(m.group(2)), int(m.group(3)))] += 1
    head = lines[:80]
    tail = lines[-80:]
    return {
        "path": os.path.abspath(path),
        "bytes": len(raw),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "line_count": len(lines),
        "tile_mentions": [
            {"tile": list(k), "n": v} for k, v in tiles.most_common(30)
        ],
        "head": head,
        "tail": tail,
        "hits": hits,
        "hit_count": len(hits),
    }


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    index = []
    for fn in FILES:
        p = os.path.join(BASE, fn)
        if not os.path.exists(p):
            index.append({"file": fn, "missing": True})
            continue
        rec = summarize(p)
        outp = os.path.join(OUT, fn.replace(".log", "-mined.json"))
        with open(outp, "w") as f:
            json.dump(rec, f, indent=2)
        index.append(
            {
                "file": fn,
                "bytes": rec["bytes"],
                "sha256": rec["sha256"],
                "line_count": rec["line_count"],
                "hit_count": rec["hit_count"],
                "tiles": rec["tile_mentions"][:10],
                "mined": outp,
            }
        )
        print(fn, rec["line_count"], rec["hit_count"], rec["sha256"][:16])
    with open(os.path.join(OUT, "log-index.json"), "w") as f:
        json.dump(index, f, indent=2)


if __name__ == "__main__":
    main()
