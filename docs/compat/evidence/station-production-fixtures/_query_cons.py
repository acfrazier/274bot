import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
for rev in (274, 289):
    data = json.loads((root / f"crates/api/data/game-data/{rev}.json").read_text())
    cons = data.get("consumption")
    print(f"=== consumption {rev} type={type(cons).__name__}", end=" ")
    if isinstance(cons, dict):
        print("keys", list(cons.keys())[:20], "n", len(cons))
        sample = next(iter(cons.values())) if cons else None
        print("sample", sample if not isinstance(sample, (list, dict)) else str(sample)[:400])
    elif isinstance(cons, list):
        print("n", len(cons))
        if cons:
            print("sample0", json.dumps(cons[0])[:500])
            print("sample1", json.dumps(cons[1])[:500] if len(cons) > 1 else None)
    else:
        print(cons)

    # find salmon/lobster/bronze/steel/flax rows
    rows = cons.values() if isinstance(cons, dict) else cons or []
    needles = (331, 329, 377, 379, 436, 438, 2349, 440, 453, 2353, 1779, 1777)
    hits = 0
    for row in rows:
        if not isinstance(row, dict):
            continue
        blob = json.dumps(row)
        if any(str(n) in blob for n in needles) or any(
            s in blob.lower()
            for s in ("salmon", "lobster", "bronze", "steel", "flax", "bow string", "copper")
        ):
            print("CONS", blob[:400])
            hits += 1
            if hits > 40:
                break
    print("hits", hits)
