import hashlib
import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
cats = [
    "100adccc037d9f6898080e1cad58fcfc43364775",
    "8e7d965be2071d6ec65c3265e12af797082d720a",
]
files = [
    "src/bot/scripts/ChaosDruidKiller/ChaosDruidKiller.ts",
    "src/bot/scripts/ChaosDruidKiller/ChaosDruidLogic.ts",
    "src/bot/scripts/MossGiant/MossGiant.ts",
    "src/bot/scripts/HillGiant/HillGiant.ts",
    "src/bot/scripts/HillGiant/HillGiantLogic.ts",
    "src/bot/scripts/AutoFighter/AutoFighter.ts",
    "src/bot/scripts/AutoFighter/AutoFighterData.ts",
]
print("=== hashes ===")
for c in cats:
    print("===", c[:8])
    for f in files:
        p = root / ".superpowers/inputs" / f"rs2b0t-{c}" / f
        print(hashlib.sha256(p.read_bytes()).hexdigest(), f.split("/")[-1], p.stat().st_size)

wanted = {
    379,
    380,
    333,
    334,
    532,
    533,
    225,
    226,
    199,
    200,
    561,
    562,
    563,
    564,
    1331,
    1332,
    526,
    527,
    995,
}
needles = (
    "lobster",
    "trout",
    "big bones",
    "limpwurt",
    "herb",
    "guam",
    "law rune",
    "nature rune",
    "adamant scimitar",
    "bones",
    "coins",
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
        if iid in wanted or any(n in low or n.replace(" ", "_") in alow for n in needles):
            hits.append((iid, alias, name))
    print(f"\n=== items {rev} n={len(hits)} ===")
    for row in sorted(hits, key=lambda r: (r[0] is None, r[0] or 0))[:90]:
        print(row)
    print("top keys", sorted(data.keys())[:40])
    for key in ("npcs", "npc", "npc_types", "pickpocket_npcs"):
        if key in data:
            v = data[key]
            print(key, type(v).__name__, len(v) if hasattr(v, "__len__") else None)
