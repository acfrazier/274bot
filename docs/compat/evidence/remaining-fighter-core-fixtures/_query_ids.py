import hashlib
import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
cats = [
    "100adccc037d9f6898080e1cad58fcfc43364775",
    "8e7d965be2071d6ec65c3265e12af797082d720a",
]
files = [
    "src/bot/scripts/RockCrab/RockCrab.ts",
    "src/bot/scripts/RockCrab/RockCrabSpots.ts",
    "src/bot/scripts/RockCrab/AmmoLogic.ts",
    "src/bot/scripts/GreenDragon/GreenDragon.ts",
    "src/bot/scripts/GreenDragon/GreenDragonLogic.ts",
    "src/bot/scripts/FireGiant/FireGiant.ts",
    "src/bot/scripts/FireGiant/FireGiantLogic.ts",
    "src/bot/scripts/ArdyFighter/ArdyFighter.ts",
]
print("=== hashes ===")
for catalog in cats:
    print("===", catalog[:8])
    for rel in files:
        path = root / ".superpowers/inputs" / f"rs2b0t-{catalog}" / rel
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        print(digest, path.name, path.stat().st_size)

wanted = {
    1540,
    1541,
    536,
    537,
    1753,
    1754,
    1751,
    1747,
    1749,
    1333,
    1334,
    1331,
    295,
    296,
    954,
    955,
    405,
    406,
    1623,
    1891,
    2309,
    1901,
    1897,
    379,
    532,
}
needles = (
    "dragonfire",
    "anti-dragon",
    "antidragon",
    "dragon bones",
    "dragonhide",
    "rune scimitar",
    "glarial",
    "rope",
    "casket",
    "uncut sapphire",
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
    for row in sorted(hits, key=lambda item: (item[0] is None, item[0] or 0)):
        print(row)
