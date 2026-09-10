#!/usr/bin/env python3
"""Compare selected host content identities against the pinned local sources.

This is source evidence only. Packed-cache shape and live behavior require
their own checks. No source files, caches, accounts or world state are changed.
"""

import hashlib
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path


ROOTS = {
    274: (Path("/Users/acfrazier/experiments/Server/content"),
          "000c19997e07206131bcb3c884265840efce416d"),
    289: (Path("/Users/acfrazier/experiments/lostcity-289/content"),
          "92649430fcbc83538d8c4367ecb96cee1a67a944"),
}
PACK_IDS = {
    "interface.pack": [147, 2808, 2812, 2831, 3322, 3323, 3415, 3416,
                       3417, 3443, 3824, 3900, 6543, *range(6546, 6554),
                       6554, 6555, 6557, 6559, 6561, 6562, 6563, 6564],
    "spotanim.pack": [245],
    "obj.pack": [3062, 2528, 1113],
    "loc.pack": [3628],
}
INTERFACES = {"controls", "xplamp", "macro_cube", "macro_mime_emotes",
              "shop_template", "bank_main", "trademain", "tradeconfirm"}


def identity(path, root):
    data = path.read_bytes()
    return {"path": str(path.relative_to(root)), "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest()}


def audit(revision, root, expected):
    commit = subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    if commit != expected:
        raise RuntimeError(f"revision {revision}: content commit changed: {commit}")
    files = []
    selected = {}
    for name, ids in PACK_IDS.items():
        path = root / "pack" / name
        files.append(identity(path, root))
        rows = {}
        for line in path.read_text().splitlines():
            key, sep, value = line.partition("=")
            if sep and key.isdigit():
                rows[int(key)] = value
        selected[name] = {str(key): rows.get(key) for key in ids}
        if any(value is None for value in selected[name].values()):
            raise RuntimeError(f"revision {revision}: missing selected {name} row")
    files.append(identity(root / "pack/interface.order", root))
    found = set()
    for path in sorted((root / "scripts").rglob("*.if")):
        if path.stem in INTERFACES:
            found.add(path.stem)
            files.append(identity(path, root))
    if found != INTERFACES:
        raise RuntimeError(f"revision {revision}: missing interfaces {INTERFACES - found}")
    thieving = root / "scripts/skill_thieving/scripts/thieving.rs2"
    files.append(identity(thieving, root))
    if "spotanim_pl(stunned_thieving, 124, 0)" not in thieving.read_text():
        raise RuntimeError(f"revision {revision}: thieving onset contract changed")
    return {"revision": revision, "content_path": str(root),
            "content_commit": commit, "selected_ids": selected, "files": files}


if __name__ == "__main__":
    worlds = [audit(revision, root, commit)
              for revision, (root, commit) in ROOTS.items()]
    print(json.dumps({
        "captured_at": datetime.now(timezone.utc).isoformat(),
        "scope": "selected source IDs and interface definitions; no packed-cache or live acceptance",
        "selected_ids_match": worlds[0]["selected_ids"] == worlds[1]["selected_ids"],
        "worlds": worlds,
    }, indent=2))
