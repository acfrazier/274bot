import json
from collections import Counter
from pathlib import Path

files = [
    Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/docs/compat/evidence/catalog-headed/r289-gnome-course-8e7d965b-6401d3a4/shots/2026-09-11T03-33-46_12953/2026-09-11T03-34-45_gnome_course.json"),
    Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/docs/compat/evidence/catalog-headed/r274-gnome-course-8e7d965b-3be68eaf/shots/2026-09-11T05-14-17_15699/2026-09-11T05-15-24_gnome_course.json"),
]
needles = ("magic tree", "stair", "ladder", "bank booth", "coal truck", "rocks")
for path in files:
    print("===", path.name)
    data = json.loads(path.read_text())
    locs = data.get("locs") or data.get("world") or []
    if isinstance(data, dict):
        for k, v in data.items():
            if isinstance(v, list) and v and isinstance(v[0], dict) and ("name" in v[0] or "id" in v[0]):
                if any(str(x.get("name") or "").lower() in ("magic tree", "bank booth") or "stair" in str(x.get("name") or "").lower() for x in v[:4000]):
                    locs = v
                    print("using key", k, "len", len(v))
                    break
    counts = Counter()
    samples = {}
    for loc in locs:
        if not isinstance(loc, dict):
            continue
        name = str(loc.get("name") or "")
        low = name.lower()
        if any(n in low for n in needles):
            key = (loc.get("id"), name)
            counts[key] += 1
            tile = loc.get("tile") or {}
            samples.setdefault(key, []).append((tile.get("x"), tile.get("z"), tile.get("level"), loc.get("actions")))
    for key, n in counts.most_common():
        print(n, key, "samples", samples[key][:4])
