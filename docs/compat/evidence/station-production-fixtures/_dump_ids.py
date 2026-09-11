import hashlib
import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
wanted = {
    331,
    329,
    332,
    330,
    377,
    379,
    378,
    380,
    323,
    343,
    381,
    436,
    437,
    438,
    439,
    440,
    441,
    453,
    454,
    2349,
    2350,
    2351,
    2353,
    2354,
    1779,
    1777,
    1778,
    1780,
    1759,
}
name_needles = (
    "raw salmon",
    "salmon",
    "raw lobster",
    "lobster",
    "burnt fish",
    "burnt lobster",
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
)
out = []
cats = [
    "100adccc037d9f6898080e1cad58fcfc43364775",
    "8e7d965be2071d6ec65c3265e12af797082d720a",
]
files = [
    "src/bot/scripts/CookBot/CookBot.ts",
    "src/bot/scripts/CookBot/CookBotLogic.ts",
    "src/bot/scripts/SmelterBot/SmelterBot.ts",
    "src/bot/scripts/SmelterBot/SmelterBotLogic.ts",
    "src/bot/scripts/FlaxSpinner/FlaxSpinner.ts",
]
base = root / ".superpowers/inputs"
for c in cats:
    out.append(f"=== catalog {c[:8]}")
    for f in files:
        p = base / f"rs2b0t-{c}" / f
        out.append(f"{hashlib.sha256(p.read_bytes()).hexdigest()} {f}")
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    out.append(f"=== {rev} keys {list(data.keys())[:24]}")
    items = data.get("items") or []
    rows = items.values() if isinstance(items, dict) else items
    out.append(
        f"items type={type(items).__name__} len={len(items) if hasattr(items, '__len__') else None}"
    )
    for item in rows:
        if not isinstance(item, dict):
            continue
        iid = item.get("id")
        name = str(item.get("name") or "")
        alias = str(item.get("alias") or "")
        low = name.lower()
        if iid in wanted or any(n == low for n in name_needles):
            out.append(f"ITEM {iid} alias={alias!r} name={name!r}")
    loc_key = next((k for k in data if "loc" in k.lower() or k in ("objects", "objTypes")), None)
    out.append(f"loc_key={loc_key}")

dest = Path(__file__).with_name("id-dump.txt")
dest.write_text("\n".join(out) + "\n")
print(dest)
