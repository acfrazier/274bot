#!/usr/bin/env python3
"""Bounded-reachability audit helpers. Read-only analysis; not a product runtime."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
CATS = [
    ROOT / ".superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775",
    ROOT / ".superpowers/inputs/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a",
]
FILES = [
    "src/bot/scripts/FlaxAIO/picking.ts",
    "src/bot/scripts/FlaxAIO/flaxaio.ts",
    "src/bot/scripts/FlaxAIO/banking.ts",
    "src/bot/scripts/FlaxAIO/walking.ts",
    "src/bot/scripts/FlaxPicker/FlaxPicker.ts",
    "src/bot/scripts/FlaxRunner/FlaxRunner.ts",
    "src/bot/event/webwalk/geometry/localReach.ts",
    "src/bot/event/webwalk/geometry/Reachability.ts",
    "src/bot/scripts/BrimhavenAgility/BrimhavenAgility.ts",
]
DIRS = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def expansions_to(dest, max_steps=10**9, start=(100, 100), adjacent_ok=True):
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


def parse_log(path: Path) -> dict:
    text = path.read_text(errors="replace")
    out = {
        "path": str(path),
        "sha256": sha256(path),
        "bytes": path.stat().st_size,
        "counts": {
            k: text.count(k)
            for k in [
                "flax_aio_pick",
                "nearest_reachable",
                "relevant_locs",
                "reachable_adj",
                "bank_closed",
                "WalkTo",
                "OpenBooth",
                "depositInventory",
                "You pick some flax",
                "You can't carry any more flax",
            ]
        },
    }
    # Extract FAIL evidence JSON-ish flax_aio_pick object if present.
    m = re.search(r'"flax_aio_pick"\s*:\s*(\{.*?)\n', text, re.S)
    if not m:
        m = re.search(r"flax_aio_pick[^\n]{0,500}", text)
        out["flax_snippet"] = m.group(0)[:800] if m else None
    else:
        out["flax_snippet"] = m.group(0)[:800]
    # Try to locate a pretty-printed diagnostic dump.
    idx = text.find("relevant_locs")
    out["relevant_locs_idx"] = idx
    if idx >= 0:
        out["relevant_locs_context"] = text[max(0, idx - 400) : idx + 1200]
    fail = text.find("FAIL:")
    out["fail_line"] = None
    if fail >= 0:
        out["fail_line"] = text[fail : fail + 900]
    # Collect player_distance / reachable_adj pairs via regex on dumped facts.
    rows = re.findall(
        r"player_distance[\"']?\s*[:=]\s*(\d+).*?reachable_adj[\"']?\s*[:=]\s*(true|false)|reachable_adj[\"']?\s*[:=]\s*(true|false).*?player_distance[\"']?\s*[:=]\s*(\d+)",
        text,
        re.I | re.S,
    )
    out["row_pairs_n"] = len(rows)
    # Simpler: extract JSON object after flax_aio_pick
    jidx = text.find('"flax_aio_pick"')
    out["json_idx"] = jidx
    return out


def main() -> None:
    hashes = []
    print("=== catalog hashes ===")
    for rel in FILES:
        hs = []
        for cat in CATS:
            p = cat / rel
            h = sha256(p) if p.exists() else "MISSING"
            hs.append(h)
        print(f"{rel}\n  100adccc {hs[0]}\n  8e7d965b {hs[1]}\n  identical {hs[0] == hs[1]}")
        hashes.append({"file": rel, "old": hs[0], "new": hs[1], "identical": hs[0] == hs[1]})

    print("\n=== open-grid expansion model (match localReach dequeue+increment) ===")
    cases = {
        "flax_offset_dx20_dz49": expansions_to((100 + 20, 100 - 49)),
        "flax_offset_cap400": expansions_to((100 + 20, 100 - 49), max_steps=400),
        "cheb8": expansions_to((108, 100)),
        "cheb8_cap400": expansions_to((108, 100), max_steps=400),
        "cheb9": expansions_to((109, 100)),
        "cheb10": expansions_to((110, 100)),
        "cheb10_cap400": expansions_to((110, 100), max_steps=400),
        "cheb11_cap400": expansions_to((111, 100), max_steps=400),
        "cheb1_cap1": expansions_to((101, 100), max_steps=1),
        "cheb2_cap1": expansions_to((102, 100), max_steps=1),
        "far59_1d_cap10": expansions_to((159, 100), max_steps=10, adjacent_ok=False),
    }
    for k, v in cases.items():
        print(k, v)

    print("\nChebyshev radius node counts (2r+1)^2:")
    radii = {r: (2 * r + 1) ** 2 for r in range(0, 16)}
    print(radii)

    logs = []
    for name in [
        "r289-flax-aio-pick-100adccc-resource2d589.log",
        "r274-flax-aio-pick-100adccc-resource2d589.log",
    ]:
        p = ROOT / "docs/compat/evidence/catalog-harness/live" / name
        parsed = parse_log(p)
        logs.append(parsed)
        print(f"\n=== {name} ===")
        print("sha256", parsed["sha256"])
        print("bytes", parsed["bytes"])
        print("counts", parsed["counts"])
        print("fail_line", parsed["fail_line"][:500] if parsed["fail_line"] else None)
        print("json_idx", parsed["json_idx"])
        print("relevant_locs_idx", parsed["relevant_locs_idx"])
        if parsed.get("relevant_locs_context"):
            print("relevant_locs_context:\n", parsed["relevant_locs_context"][:1500])
        if parsed.get("flax_snippet"):
            print("flax_snippet:\n", parsed["flax_snippet"][:800])

    out = ROOT / "docs/compat/evidence/bounded-reachability-audit/parse.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps({"hashes": hashes, "expansions": cases, "radii": radii, "logs": logs}, indent=2)
        + "\n"
    )
    print("wrote", out)


if __name__ == "__main__":
    main()
