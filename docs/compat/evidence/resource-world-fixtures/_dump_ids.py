import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
wanted = {1513, 1514, 72, 70, 73, 71, 861, 859, 946, 1353, 453, 454, 1269, 2096, 2097}
name_needles = (
    "magic logs",
    "magic shortbow",
    "magic longbow",
    "knife",
    "steel axe",
    "coal",
    "steel pickaxe",
    "odd cocktail",
)
out = []
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    out.append(f"=== {rev} keys {list(data.keys())[:24]}")
    items = data.get("items") or data.get("item") or []
    rows = items.values() if isinstance(items, dict) else items
    out.append(f"items type={type(items).__name__} len={len(items) if hasattr(items, '__len__') else None}")
    for item in rows:
        if not isinstance(item, dict):
            continue
        iid = item.get("id")
        name = str(item.get("name") or "")
        alias = str(item.get("alias") or "")
        low = name.lower()
        if iid in wanted or any(n in low for n in name_needles):
            out.append(f"ITEM {iid} alias={alias!r} name={name!r}")
    loc_key = next((k for k in data if "loc" in k.lower() or k in ("objects", "objTypes")), None)
    out.append(f"loc_key={loc_key}")
    locs = data.get(loc_key) if loc_key else []
    loc_rows = locs.values() if isinstance(locs, dict) else locs or []
    hits = 0
    for loc in loc_rows:
        if not isinstance(loc, dict):
            continue
        name = str(loc.get("name") or loc.get("alias") or "")
        low = name.lower()
        lid = loc.get("id")
        if lid in (2096, 2097, 1306, 1742) or "magic tree" in low or "coal truck" in low:
            out.append(f"LOC {lid} {name!r} {loc.get('actions') or loc.get('ops') or loc.get('op')}")
            hits += 1
    out.append(f"named loc hits {hits}")

dest = Path(__file__).with_name("id-dump.txt")
dest.write_text("\n".join(out) + "\n")
print(dest)
