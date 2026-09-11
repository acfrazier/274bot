#!/usr/bin/env python3
"""Offline open-grid expansion model. Not a Seers collision replay."""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
DIRS = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)]


def expansions_to(dest, max_steps=10**9, start=(0, 0), adjacent_ok=True):
    sx, sz = start
    tx, tz = dest
    seen = {sx * 100000 + sz}
    q = [(sx, sz)]
    expansions = 0
    head = 0
    while head < len(q):
        cx, cz = q[head]
        head += 1
        if (cx, cz) == (tx, tz):
            return {"status": "found", "expansions": expansions, "dequeued": head}
        if adjacent_ok and abs(cx - tx) + abs(cz - tz) == 1:
            return {"status": "adj", "expansions": expansions, "dequeued": head}
        expansions += 1
        if expansions > max_steps:
            return {"status": "capped", "expansions": expansions, "dequeued": head}
        for dx, dz in DIRS:
            nx, nz = cx + dx, cz + dz
            k = nx * 100000 + nz
            if k not in seen:
                seen.add(k)
                q.append((nx, nz))
    return {"status": "miss", "expansions": expansions, "dequeued": head}


def cheb(d, **kw):
    return expansions_to((d, 0), **kw)


def corridor_to(d, max_steps=400, adjacent_ok=False):
    """1-wide infinite x-corridor: only z==0 is walkable. Opposite bound vs open grid."""
    sx, sz = 0, 0
    tx, tz = d, 0
    seen = {0}
    q = [(sx, sz)]
    expansions = 0
    head = 0
    while head < len(q):
        cx, cz = q[head]
        head += 1
        if (cx, cz) == (tx, tz):
            return {"status": "found", "expansions": expansions, "dequeued": head}
        if adjacent_ok and abs(cx - tx) + abs(cz - tz) == 1:
            return {"status": "adj", "expansions": expansions, "dequeued": head}
        expansions += 1
        if expansions > max_steps:
            return {"status": "capped", "expansions": expansions, "dequeued": head}
        for dx, dz in ((-1, 0), (1, 0)):
            nx, nz = cx + dx, cz + dz
            if nz != 0:
                continue
            k = nx * 100000 + nz
            if k not in seen:
                seen.add(k)
                q.append((nx, nz))
    return {"status": "miss", "expansions": expansions, "dequeued": head}


def main() -> None:
    distances = [1, 2, 8, 10, 11, 12, 20, 41, 49, 55]
    rows = []
    for d in distances:
        open_uncap = cheb(d)
        open_400 = cheb(d, max_steps=400)
        corridor = corridor_to(d, max_steps=400)
        rows.append(
            {
                "chebyshev": d,
                "open_adjacent_uncapped": open_uncap,
                "open_adjacent_maxSteps_400": open_400,
                "corridor_exact_maxSteps_400": corridor,
                "nodes_within_cheb": (2 * d + 1) ** 2,
            }
        )
    out = {
        "note": (
            "Standalone bounds, not a Seers collision replay. Open-grid uses 8-connected "
            "localReach DIRS. Corridor is 1-wide along x only. Foreign Reach.ts states "
            "400 expansions run out at ~11 tiles of open ground. A 1-wide corridor of "
            "length 41 is still inside 400 expansions; an open plane at 41 is not."
        ),
        "default_max_steps": 400,
        "live_nearest_player_distance": 41,
        "live_farthest_player_distance": 55,
        "rows": rows,
    }
    dest = ROOT / "docs/compat/evidence/bounded-reachability-audit/expansion-model.json"
    dest.write_text(json.dumps(out, indent=2) + "\n")
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
