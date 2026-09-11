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
    "src/bot/scripts/CookBot/CookBot.ts",
    "src/bot/scripts/CookBot/CookBotLogic.ts",
    "src/bot/scripts/SmelterBot/SmelterBot.ts",
    "src/bot/scripts/SmelterBot/SmelterBotLogic.ts",
    "src/bot/scripts/FlaxSpinner/FlaxSpinner.ts",
]
print("=== catalog hashes ===")
for c in cats:
    print("===", c[:8])
    for f in files:
        p = base / f"rs2b0t-{c}" / f
        print(hashlib.sha256(p.read_bytes()).hexdigest(), f, p.stat().st_size)

needles = (
    "raw salmon",
    "salmon",
    "raw lobster",
    "lobster",
    "burnt",
    "copper ore",
    "tin ore",
    "iron ore",
    "coal",
    "bronze bar",
    "steel bar",
    "iron bar",
    "flax",
    "bow string",
    "ball of wool",
    "wool",
)
wanted = {
    331,
    329,
    377,
    379,
    343,
    381,
    436,
    438,
    440,
    453,
    2349,
    2351,
    2353,
    1779,
    1777,
    1759,
    1737,
}
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    print(f"\n=== schema {rev} keys", list(data.keys())[:40])
    items = data.get("items") or []
    rows = items.values() if isinstance(items, dict) else items
    print(
        f"=== items {rev} type={type(items).__name__} n={len(items) if hasattr(items, '__len__') else None}"
    )
    hits = []
    for item in rows:
        if not isinstance(item, dict):
            continue
        iid = item.get("id")
        name = str(item.get("name") or "")
        alias = str(item.get("alias") or "")
        low = name.lower()
        alow = alias.lower()
        if iid in wanted or any(
            n in low or n.replace(" ", "_") in alow or n in alow for n in needles
        ):
            hits.append((iid, alias, name))
    for row in sorted(hits, key=lambda r: (r[0] is None, r[0] or 0)):
        print(row)

    loc_like = [
        k
        for k in data.keys()
        if "loc" in k.lower() or "obj" in k.lower() or "com" in k.lower()
    ]
    print("loc-like keys", loc_like)
    for key in ("locs", "objects", "loc_types", "locTypes"):
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
            if any(n in low for n in ("range", "furnace", "spinning", "ladder", "stair")):
                print(
                    "LOC",
                    lid,
                    name,
                    loc.get("actions") or loc.get("ops") or loc.get("op"),
                )
        break
