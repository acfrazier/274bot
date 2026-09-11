import json
from pathlib import Path

root = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
headed = root / "docs/compat/evidence/catalog-headed"
needles = ("range", "furnace", "spinning", "ladder", "stair")
tiles = {
    (2817, 3444, 0),
    (2817, 3443, 0),
    (3275, 3185, 0),
    (2711, 3471, 1),
    (2714, 3471, 0),
    (2809, 3441, 0),
    (2722, 3493, 0),
    (3269, 3167, 0),
}
hits = []
for path in headed.rglob("*.json"):
    try:
        data = json.loads(path.read_text())
    except Exception:
        continue
    locs = None
    if isinstance(data, dict):
        if isinstance(data.get("locs"), list):
            locs = data["locs"]
        else:
            for k, v in data.items():
                if (
                    isinstance(v, list)
                    and v
                    and isinstance(v[0], dict)
                    and ("name" in v[0] or "id" in v[0])
                ):
                    name0 = str(v[0].get("name") or "").lower()
                    if any(n in name0 for n in needles) or "bank" in name0:
                        locs = v
                        break
    if not locs:
        continue
    for loc in locs:
        if not isinstance(loc, dict):
            continue
        name = str(loc.get("name") or "")
        low = name.lower()
        tile = loc.get("tile") or {}
        if isinstance(tile, dict):
            t = (tile.get("x"), tile.get("z"), tile.get("level"))
        elif isinstance(tile, (list, tuple)) and len(tile) >= 2:
            t = (tile[0], tile[1], tile[2] if len(tile) > 2 else 0)
        else:
            t = (loc.get("x"), loc.get("z"), loc.get("level"))
        if any(n in low for n in needles) or t in tiles:
            hits.append(
                (
                    str(path.relative_to(root)),
                    loc.get("id"),
                    name,
                    t,
                    loc.get("actions") or loc.get("ops") or loc.get("op"),
                )
            )

out = Path(
    "/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/docs/compat/evidence/station-production-fixtures/loc-dump.txt"
)
lines = [f"{len(hits)} hits"]
for row in hits[:200]:
    lines.append(repr(row))
out.write_text("\n".join(lines) + "\n")
print(out, "hits", len(hits))
