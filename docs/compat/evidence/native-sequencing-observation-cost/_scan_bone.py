#!/usr/bin/env python3
"""Scan BoneBurier 21446414 logs for walk/open/timeout/slow-tick facts."""
import json
import os
import re

BASE = "docs/compat/evidence/catalog-harness/live"
OUT = "docs/compat/evidence/native-sequencing-observation-cost"
SLOW = re.compile(r"slow tick (\d+): ([0-9.]+)ms")
NAV = re.compile(
    r"\[nav-walk\] here=WorldTile \{ x: (-?\d+), z: (-?\d+), level: (-?\d+) \} "
    r"aim=WorldTile \{ x: (-?\d+), z: (-?\d+), level: (-?\d+) \} radius=(-?\d+)"
)
FOLLOW = re.compile(
    r"\[nav-follow\] here=WorldTile \{ x: (-?\d+), z: (-?\d+), level: (-?\d+) \}"
)
INTERACT = re.compile(r"interact (WalkNearestBank|OpenBooth|WalkNear|Held)")
OPEN = re.compile(r"could not open|walk-timeout|timeout|retrying|withdraw|Bury|bank_open|open-booth", re.I)


def scan(path):
    slow = []
    nav = []
    follow = []
    interacts = []
    notes = []
    ticks_seen = []
    for i, line in enumerate(open(path, errors="replace"), 1):
        m = SLOW.search(line)
        if m:
            slow.append({"line": i, "tick": int(m.group(1)), "ms": float(m.group(2)), "text": line.strip()[:240]})
        m = NAV.search(line)
        if m:
            nav.append({
                "line": i,
                "here": [int(m.group(1)), int(m.group(2)), int(m.group(3))],
                "aim": [int(m.group(4)), int(m.group(5)), int(m.group(6))],
                "radius": int(m.group(7)),
            })
        m = FOLLOW.search(line)
        if m:
            follow.append({"line": i, "here": [int(m.group(1)), int(m.group(2)), int(m.group(3))]})
        m = INTERACT.search(line)
        if m:
            interacts.append({"line": i, "kind": m.group(1), "text": line.strip()[:240]})
        if OPEN.search(line) or "WalkNearestBank" in line or "open-booth" in line or "[shim" in line:
            if len(notes) < 200:
                notes.append({"line": i, "text": line.strip()[:300]})
    return {
        "path": path,
        "slow_n": len(slow),
        "slow_ticks": [s["tick"] for s in slow],
        "slow_ms": [s["ms"] for s in slow],
        "slow_min": min((s["ms"] for s in slow), default=None),
        "slow_max": max((s["ms"] for s in slow), default=None),
        "slow_median": (sorted(s["ms"] for s in slow)[len(slow)//2] if slow else None),
        "slow_ge50": sum(1 for s in slow if s["ms"] >= 50),
        "slow_ge40_lt50": sum(1 for s in slow if 40 <= s["ms"] < 50),
        "first_slow": slow[0] if slow else None,
        "last_slow": slow[-1] if slow else None,
        "nav_n": len(nav),
        "nav_first": nav[0] if nav else None,
        "nav_last": nav[-1] if nav else None,
        "nav_radii": sorted({n["radius"] for n in nav}),
        "nav_last_radius0": next((n for n in reversed(nav) if n["radius"] == 0), None),
        "follow_first": follow[0] if follow else None,
        "follow_last": follow[-1] if follow else None,
        "interacts": interacts,
        "notes": notes,
    }


def main():
    out = {}
    for name in sorted(os.listdir(BASE)):
        if "bone" in name and name.endswith("21446414.log"):
            rec = scan(os.path.join(BASE, name))
            out[name] = rec
            print(name, "slow", rec["slow_n"], "ge50", rec["slow_ge50"], "nav", rec["nav_n"], "interacts", rec["interacts"])
    with open(os.path.join(OUT, "bone-21446414-scan.json"), "w") as fh:
        json.dump(out, fh, indent=2)
    print("wrote bone-21446414-scan.json")


if __name__ == "__main__":
    main()
