#!/usr/bin/env python3
"""Read-only harvest of flax/gnome LIVE logs for t_26092c52."""
from pathlib import Path
import hashlib
import json
import re

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
LIVE = ROOT / "docs/compat/evidence/catalog-harness/live"
OUT = ROOT / "docs/compat/evidence/flax-course-ownership"
OUT.mkdir(parents=True, exist_ok=True)

GLOBS = [
    "*flax-picker*-b1cff8a7.log",
    "*gnome-course*-b1cff8a7.log",
    "*flax-picker*-3be68eaf.log",
    "*gnome-course*-3be68eaf.log",
    "*gnome-course-radius*-50f2be8a.log",
    "*gnome-course-radius*-3be68eaf.log",
]
KEYS = (
    "FAIL",
    "failed",
    "timeout",
    "not impl",
    "re-sync",
    "resync",
    "lap ",
    "Squeeze",
    "Pick",
    "1779",
    "2737",
    "2738",
    "2484",
    "2474",
    "walkable",
    "canReach",
    "full pack",
    "deposit",
    "deadline",
    "panic",
    "assertion",
    "XP",
    "pipe",
    "log balance",
    "obstacle",
    "inventory",
    "Flax",
    "flax",
    "course",
    "passed",
    "failed:",
    "thread",
    "dest",
    "loc",
)

files = []
for g in GLOBS:
    files.extend(sorted(LIVE.glob(g)))
# unique, keep b1cff8a7 first
seen = set()
ordered = []
for p in files:
    if p.name in seen:
        continue
    seen.add(p.name)
    ordered.append(p)

index = []
for p in ordered:
    text = p.read_text(errors="replace")
    sha = hashlib.sha256(text.encode("utf-8", errors="replace")).hexdigest()
    lines = text.splitlines()
    interesting = []
    for i, line in enumerate(lines, 1):
        if any(k.lower() in line.lower() for k in KEYS):
            interesting.append(f"{i}|{line[:400]}")
    rec = {
        "file": str(p.relative_to(ROOT)),
        "bytes": p.stat().st_size,
        "sha256": sha,
        "lines": len(lines),
        "interesting_count": len(interesting),
        "tail": lines[-40:],
        "interesting_head": interesting[:80],
        "interesting_tail": interesting[-40:] if len(interesting) > 80 else [],
    }
    index.append(rec)
    outp = OUT / (p.stem + ".extract.txt")
    with outp.open("w") as f:
        f.write(f"file={p}\nbytes={rec['bytes']}\nsha256={sha}\nlines={len(lines)}\n")
        f.write("===INTERESTING===\n")
        for row in interesting:
            f.write(row + "\n")
        f.write("===TAIL===\n")
        start = max(1, len(lines) - 39)
        for i, line in enumerate(lines[-40:], start=start):
            f.write(f"{i}|{line}\n")

(OUT / "log-index.json").write_text(json.dumps(index, indent=2) + "\n")
print(f"wrote {len(index)} extracts to {OUT}")
for rec in index:
    print(rec["file"], rec["bytes"], rec["interesting_count"], rec["sha256"][:12])
