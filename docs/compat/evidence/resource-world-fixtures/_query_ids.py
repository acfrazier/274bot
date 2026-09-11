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
    "src/bot/scripts/GnomeMagicChopper/GnomeMagicChopper.ts",
    "src/bot/scripts/CoalTrucks/CoalTrucks.ts",
    "src/bot/scripts/CoalTrucks/CoalTrucksLogic.ts",
]
for c in cats:
    print("===", c[:8])
    for f in files:
        p = base / f"rs2b0t-{c}" / f
        print(hashlib.sha256(p.read_bytes()).hexdigest(), f)

wanted = {1513, 72, 70, 861, 859, 946, 1353, 453, 1269, 1511, 60, 1777, 2096, 2097}
name_needles = (
    "magic logs",
    "magic shortbow",
    "magic longbow",
    "knife",
    "steel axe",
    "coal",
    "steel pickaxe",
)
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    print(f"=== items {rev} keys", sorted(data.keys())[:30])
    items = data.get("items") or []
    print("items type", type(items).__name__, "len", len(items) if hasattr(items, "__len__") else None)
    rows = items.values() if isinstance(items, dict) else items
    print(f"=== items {rev}")
    for item in rows:
        if not isinstance(item, dict):
            continue
        iid = item.get("id")
        name = str(item.get("name") or "")
        alias = str(item.get("alias") or "")
        low = name.lower()
        if iid in wanted or any(n in low for n in name_needles):
            print(iid, alias, name)

    for key in ("locs", "objects", "loc_types", "locTypes"):
        if key in data:
            locs = data[key]
            print(f"=== {key} {rev}", type(locs).__name__, len(locs) if hasattr(locs, "__len__") else None)
            break
    else:
        locs = []
        print("no loc key", [k for k in data.keys() if "loc" in k.lower() or "obj" in k.lower()])
    loc_rows = locs.values() if isinstance(locs, dict) else locs
    print(f"=== locs {rev}")
    hits = 0
    for loc in loc_rows:
        if not isinstance(loc, dict):
            continue
        name = str(loc.get("name") or loc.get("alias") or "")
        low = name.lower()
        lid = loc.get("id")
        if lid in (2096, 2097) or "magic tree" in low or "coal truck" in low:
            print("LOC", lid, name, loc.get("actions") or loc.get("ops") or loc.get("op"))
            hits += 1
    print("named loc hits", hits)
