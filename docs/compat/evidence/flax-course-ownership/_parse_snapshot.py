#!/usr/bin/env python3
"""Parse the native flax diagnostic snapshot for loc identity at the stuck tiles."""
from pathlib import Path
import json
import hashlib

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
SHOT = ROOT / "docs/compat/evidence/catalog-headed/r274-flax-picker-100adccc-b1cff8a7-gpu-diagnostic/shots/2026-09-11T06-21-11_29226/2026-09-11T06-22-46_flax_picker.json"
PNG = SHOT.with_suffix(".png")
OUT = ROOT / "docs/compat/evidence/flax-course-ownership"

raw = SHOT.read_bytes()
data = json.loads(raw)
player_tile = data.get("player", {}).get("player", {}).get("actor", {}).get("tile")
locs = data.get("loc") or data.get("locs") or []
# try common keys
if not locs:
    for k, v in data.items():
        if isinstance(v, list) and v and isinstance(v[0], dict) and ("tile" in v[0] or "layer" in v[0] or "actions" in v[0]):
            if "name" in v[0] or "id" in v[0] or "layer" in v[0]:
                print("candidate key", k, "n", len(v), "sample keys", list(v[0].keys())[:12])

print("top keys", list(data.keys())[:40])
print("player_tile", player_tile)
print("png exists", PNG.exists(), PNG.stat().st_size if PNG.exists() else 0)
print("json sha256", hashlib.sha256(raw).hexdigest())
print("json bytes", len(raw))

# inventory
inv = data.get("inv") or data.get("inventory") or data.get("items")
print("inv type", type(inv).__name__, "len" if hasattr(inv, "__len__") else "", len(inv) if hasattr(inv, "__len__") else None)

targets = {(2737, 3440, 0), (2738, 3441, 0), (2738, 3440, 0), (2739, 3442, 0), (2739, 3444, 0)}

def tile_of(row):
    t = row.get("tile") if isinstance(row, dict) else None
    if isinstance(t, dict):
        return (t.get("x"), t.get("z"), t.get("level", 0))
    return None

loc_key = None
for k, v in data.items():
    if not isinstance(v, list) or not v or not isinstance(v[0], dict):
        continue
    sample = v[0]
    if "tile" in sample and ("layer" in sample or "actions" in sample or "name" in sample):
        loc_key = k
        locs = v
        break

print("loc_key", loc_key, "n", len(locs) if locs else 0)
if locs:
    print("sample", json.dumps(locs[0])[:500])

hits = []
flax = []
for i, row in enumerate(locs or []):
    t = tile_of(row)
    name = (row.get("name") or "")
    if t in targets or (isinstance(name, str) and "flax" in name.lower()):
        rec = {
            "i": i,
            "tile": t,
            "id": row.get("id") or row.get("typecode") or row.get("def_id"),
            "name": name,
            "layer": row.get("layer"),
            "actions": row.get("actions"),
            "distance": row.get("distance"),
            "reachable": row.get("reachable"),
            "reachable_adj": row.get("reachable_adj"),
        }
        hits.append(rec)
        if isinstance(name, str) and "flax" in name.lower():
            flax.append(rec)

summary = {
    "snapshot": str(SHOT.relative_to(ROOT)),
    "json_sha256": hashlib.sha256(raw).hexdigest(),
    "png_sha256": hashlib.sha256(PNG.read_bytes()).hexdigest() if PNG.exists() else None,
    "png_bytes": PNG.stat().st_size if PNG.exists() else None,
    "player_tile": player_tile,
    "loc_key": loc_key,
    "loc_count": len(locs) if locs else 0,
    "flax_count": len(flax),
    "target_hits": [h for h in hits if h["tile"] in targets],
    "nearby_flax": flax[:40],
    "top_keys": list(data.keys()),
}
(OUT / "native-flax-snapshot-locs.json").write_text(json.dumps(summary, indent=2) + "\n")
print("flax_count", len(flax))
print("target_hits", json.dumps(summary["target_hits"], indent=2))
print("first 8 flax", json.dumps(flax[:8], indent=2))
