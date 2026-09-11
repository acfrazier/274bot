import hashlib
import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
cats = [
    "100adccc037d9f6898080e1cad58fcfc43364775",
    "8e7d965be2071d6ec65c3265e12af797082d720a",
]
base = root / ".superpowers/inputs"
files = [
    "src/bot/scripts/FlaxAIO/flaxaio.ts",
    "src/bot/scripts/FlaxAIO/picking.ts",
    "src/bot/scripts/FlaxAIO/spinning.ts",
    "src/bot/scripts/FlaxAIO/banking.ts",
    "src/bot/scripts/FlaxAIO/walking.ts",
    "src/bot/scripts/HerbloreSecondaries/HerbloreSecondaries.ts",
    "src/bot/scripts/HerbloreSecondaries/HerbloreSecondariesLogic.ts",
]
print("=== catalog hashes ===")
for c in cats:
    print("===", c[:8])
    for f in files:
        p = base / f"rs2b0t-{c}" / f
        print(hashlib.sha256(p.read_bytes()).hexdigest(), f.split("/")[-1], p.stat().st_size)

wanted_ids = {1779, 1777, 1780, 1778, 1759, 223, 224, 221, 222, 995, 379, 380}
needles = (
    "flax",
    "bow string",
    "ball of wool",
    "red spider",
    "eye of newt",
    "coins",
    "lobster",
)
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    items = data.get("items") or []
    rows = items.values() if isinstance(items, dict) else items
    hits = []
    for item in rows:
        if not isinstance(item, dict):
            continue
        iid = item.get("id")
        name = str(item.get("name") or "")
        alias = str(item.get("alias") or "")
        low = name.lower()
        alow = alias.lower()
        if iid in wanted_ids or any(
            n in low or n.replace(" ", "_") in alow for n in needles
        ):
            hits.append((iid, alias, name))
    print(f"\n=== items {rev} ===")
    for row in sorted(hits, key=lambda r: (r[0] is None, r[0] or 0)):
        print(row)
    loc_like = [
        k
        for k in data.keys()
        if any(x in k.lower() for x in ("loc", "obj", "com", "npc"))
    ]
    print("loc-like", loc_like)
    for key in ("locs", "objects", "loc_types", "locTypes", "npcs"):
        if key not in data:
            continue
        locs = data[key]
        loc_rows = locs.values() if isinstance(locs, dict) else locs
        print(f"=== {key} {rev} n={len(locs) if hasattr(locs, '__len__') else None}")
        for loc in loc_rows:
            if not isinstance(loc, dict):
                continue
            name = str(loc.get("name") or loc.get("alias") or "")
            low = name.lower()
            lid = loc.get("id")
            if any(
                n in low
                for n in (
                    "flax",
                    "spinning",
                    "ladder",
                    "betty",
                    "bank booth",
                )
            ):
                print(
                    "LOC",
                    lid,
                    name,
                    loc.get("actions") or loc.get("ops") or loc.get("op"),
                )
