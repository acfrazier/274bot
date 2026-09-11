#!/usr/bin/env python3
"""Field-local loc composition from the native flax diagnostic snapshot."""
from pathlib import Path
import json
import math

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
SHOT = ROOT / "docs/compat/evidence/catalog-headed/r274-flax-picker-100adccc-b1cff8a7-gpu-diagnostic/shots/2026-09-11T06-21-11_29226/2026-09-11T06-22-46_flax_picker.json"
OUT = ROOT / "docs/compat/evidence/flax-course-ownership"
data = json.loads(SHOT.read_text())
locs = data["loc"]
player = data["player"]["player"]["actor"]["tile"]
field = (2741, 3444)
SCOPE = 12

def cheb(a, b):
    return max(abs(a[0]-b[0]), abs(a[1]-b[1]))

field_locs = []
for i, row in enumerate(locs):
    t = row["tile"]
    tile = (t["x"], t["z"], t.get("level", 0))
    if tile[2] != 0:
        continue
    if cheb((tile[0], tile[1]), field) > SCOPE:
        continue
    field_locs.append({
        "i": i,
        "tile": [tile[0], tile[1], tile[2]],
        "id": row.get("id"),
        "name": row.get("name"),
        "layer": row.get("layer"),
        "actions": row.get("actions"),
        "distance": row.get("distance"),
        "block_walk": row.get("block_walk"),
        "block_range": row.get("block_range"),
        "width": row.get("width"),
        "length": row.get("length"),
        "shape": row.get("shape"),
    })

by_tile = {}
for row in field_locs:
    key = tuple(row["tile"])
    by_tile.setdefault(key, []).append(row)

colocated = []
for tile, rows in sorted(by_tile.items()):
    layers = [r["layer"] for r in rows]
    names = [r["name"] for r in rows]
    ids = [r["id"] for r in rows]
    if len(rows) > 1:
        colocated.append({"tile": list(tile), "layers": layers, "ids": ids, "names": names, "rows": rows})

flax_tiles = [t for t, rows in by_tile.items() if any((r.get("name") or "").lower() == "flax" for r in rows)]
flax_with_wall = [c for c in colocated if "Wall" in c["layers"] and any((n or "").lower() == "flax" for n in c["names"])]
player_tile = (player["x"], player["z"], player.get("level", 0))

summary = {
    "player_tile": player,
    "inv_len": len(data.get("inv") or []),
    "inv": data.get("inv"),
    "field_loc_count": len(field_locs),
    "field_tiles": len(by_tile),
    "flax_tiles_in_scope": len(flax_tiles),
    "colocated_tiles": len(colocated),
    "flax_with_wall_tiles": flax_with_wall,
    "player_tile_rows": by_tile.get(player_tile, []),
    "tile_2737_3440": by_tile.get((2737, 3440, 0), []),
    "tile_2738_3441": by_tile.get((2738, 3441, 0), []),
    "tile_2738_3440": by_tile.get((2738, 3440, 0), []),
    "tile_2739_3442": by_tile.get((2739, 3442, 0), []),
    "chat_head": (data.get("chat") or [])[:12],
}
(OUT / "native-flax-field-locs.json").write_text(json.dumps(summary, indent=2) + "\n")
print("field_locs", len(field_locs), "tiles", len(by_tile), "flax_tiles", len(flax_tiles), "colocated", len(colocated), "flax+wall", len(flax_with_wall))
print("player rows", json.dumps(summary["player_tile_rows"], indent=2))
print("2737,3440", json.dumps(summary["tile_2737_3440"], indent=2))
print("2738,3441", json.dumps(summary["tile_2738_3441"], indent=2))
print("flax+wall tiles", json.dumps(flax_with_wall, indent=2)[:4000])
print("inv", summary["inv"])
