import hashlib
import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
cats = [
    "100adccc037d9f6898080e1cad58fcfc43364775",
    "8e7d965be2071d6ec65c3265e12af797082d720a",
]
base = root / ".superpowers/platform-preparation/platform-isolated-caf6b809/inputs"
files = [
    "src/bot/scripts/ArdyCakes/ArdyCakes.ts",
    "src/bot/scripts/ArdyThiever/ArdyThiever.ts",
    "src/bot/api/thieving/cakeStallData.ts",
    "src/bot/api/thieving/CakeStall.ts",
    "src/bot/api/thieving/targets.ts",
    "src/bot/data/pickpocketTargets.ts",
]
for c in cats:
    print("===", c[:8])
    for f in files:
        p = base / f"rs2b0t-{c}" / f
        print(hashlib.sha256(p.read_bytes()).hexdigest(), f)

wanted = {1891, 1892, 1897, 1901, 1902, 2309, 2310, 995}
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    print(f"=== items {rev}")
    for item in data["items"]:
        if item.get("id") in wanted:
            print(item["id"], item.get("alias"), item.get("name"))
    pp = data.get("pickpocket")
    print("pickpocket type", type(pp).__name__, "len", len(pp) if hasattr(pp, "__len__") else None)
    if isinstance(pp, dict):
        print("pickpocket keys", list(pp.keys())[:20])
        for k, v in list(pp.items())[:8]:
            print(" ", k, v if not isinstance(v, (list, dict)) else (type(v).__name__, len(v)))
    elif isinstance(pp, list) and pp:
        for row in pp:
            npcs = row.get("npcs") or []
            names = ",".join(str(n.get("name") or "") for n in npcs)
            if "Guard" in names or "Knight" in names:
                loot = row.get("loot")
                print(row.get("group"), "level", row.get("level"), "xp", row.get("experience"), "stun", row.get("stun_ticks"), "npcs", names, "loot", loot)
