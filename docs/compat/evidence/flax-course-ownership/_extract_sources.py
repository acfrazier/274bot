#!/usr/bin/env python3
"""Extract hashed line windows from frozen sources for t_26092c52."""
from pathlib import Path
import hashlib
import json
import re

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
OUT = ROOT / "docs/compat/evidence/flax-course-ownership"
EXPORT = ROOT / ".superpowers/review-exports/catalog-root-b1cff8a7"
CAT_OLD = ROOT / ".superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775"
CAT_NEW = ROOT / ".superpowers/inputs/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a"
CLIENT = ROOT / "vendor/fr-client-rust"

files = {
    "flax_old": CAT_OLD / "src/bot/scripts/FlaxPicker/FlaxPicker.ts",
    "flax_new": CAT_NEW / "src/bot/scripts/FlaxPicker/FlaxPicker.ts",
    "agility_old": CAT_OLD / "src/bot/scripts/AgilityBot/AgilityBot.ts",
    "agility_new": CAT_NEW / "src/bot/scripts/AgilityBot/AgilityBot.ts",
    "reachability_js": EXPORT / "crates/script/src/shim/reachability.js",
    "locs_js": EXPORT / "crates/script/src/shim/locs.js",
    "query_js": EXPORT / "crates/script/src/shim/query.js",
    "api_query": EXPORT / "crates/api/src/query.rs",
    "api_snapshot": EXPORT / "crates/api/src/snapshot.rs",
    "host_play": EXPORT / "crates/host-play/src/lib.rs",
}

# client may be in export vendor or live gitlink
for cand in [
    EXPORT / "vendor/fr-client-rust/crates/client/src/world.rs",
    CLIENT / "crates/client/src/world.rs",
]:
    if cand.exists():
        files["client_world"] = cand
        break

hashes = {}
for name, path in files.items():
    data = path.read_bytes()
    hashes[name] = {
        "path": str(path),
        "sha256": hashlib.sha256(data).hexdigest(),
        "bytes": len(data),
        "exists": True,
    }

(OUT / "source-hashes.json").write_text(json.dumps(hashes, indent=2) + "\n")
print("hashed", len(hashes))
for k, v in hashes.items():
    print(k, v["sha256"], v["bytes"])

# dump matching lines for key symbols
needles = {
    "agility_old": ["searchRadius", "resyncTo", "DoObstacle", "distance()", "find("],
    "agility_new": ["searchRadius", "resyncTo", "DoObstacle", "distance()", "find("],
    "reachability_js": ["walkable", "canReach", "canStep", "not impl"],
    "locs_js": ["distance", "query", "results", "cache"],
    "api_snapshot": ["static_loc", "rebuild_loc", "LocView", "distance"],
    "api_query": ["flood_reach", "can_reach", "walkable", "reachable_adj"],
    "host_play": ["flood_reach", "reachable", "static_loc", "loc_view"],
    "client_world": ["static_loc_generation", "del_loc", "add_scenery"],
}

for name, keys in needles.items():
    path = Path(files[name])
    lines = path.read_text(errors="replace").splitlines()
    outp = OUT / f"src-{name}.txt"
    with outp.open("w") as f:
        f.write(f"file={path}\nsha256={hashes[name]['sha256']}\n")
        for i, line in enumerate(lines, 1):
            if any(k in line for k in keys):
                f.write(f"{i}|{line}\n")
    print("wrote", outp.name)
