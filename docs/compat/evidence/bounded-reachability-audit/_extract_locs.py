#!/usr/bin/env python3
"""Extract flax_aio_pick loc facts from frozen resource2d589 logs."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

ROOT = Path("/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision")
LIVE = ROOT / "docs/compat/evidence/catalog-harness/live"


def extract_obj(text: str, key: str) -> dict | None:
    needle = f'"{key}":'
    i = text.find(needle)
    if i < 0:
        return None
    start = text.find("{", i)
    if start < 0:
        return None
    depth = 0
    for j, ch in enumerate(text[start:], start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return json.loads(text[start : j + 1])
    return None


def summarize(name: str) -> dict:
    path = LIVE / name
    text = path.read_text(errors="replace")
    facts = extract_obj(text, "flax_aio_pick")
    walkto = [ln.strip() for ln in text.splitlines() if "WalkTo" in ln or "OpenBooth" in ln]
    pick_ids = re.findall(r"You pick some flax", text)
    identity = extract_obj(text, "phase") if '"phase":"identity"' in text.replace(" ", "") else None
    # identity is nested; grab first JSON object on line 3-ish
    ident = None
    for ln in text.splitlines():
        if '"phase":"identity"' in ln.replace(" ", "") or '"phase": "identity"' in ln:
            try:
                ident = json.loads(ln)
            except json.JSONDecodeError:
                ident = {"raw_prefix": ln[:400]}
            break
    locs = facts.get("relevant_locs", []) if facts else []
    dists = [loc["player_distance"] for loc in locs]
    field = [loc["field_distance"] for loc in locs]
    adj_true = [loc for loc in locs if loc.get("reachable_adj")]
    reach_true = [loc for loc in locs if loc.get("reachable")]
    return {
        "file": name,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "bytes": path.stat().st_size,
        "identity_keys": sorted(ident.keys()) if isinstance(ident, dict) else None,
        "catalog_commit": ident.get("catalog_commit") if isinstance(ident, dict) else None,
        "revision": ident.get("revision") if isinstance(ident, dict) else None,
        "card": ident.get("card") if isinstance(ident, dict) else None,
        "pick_chat_count": len(pick_ids),
        "walk_open_lines": walkto,
        "facts_meta": {
            k: facts.get(k)
            for k in [
                "player_tile",
                "field_center",
                "field_scope",
                "at_field",
                "bank_open",
                "bank_loaded",
                "bank_generation",
                "reachability_source",
                "adapter_query",
                "reachability_available",
                "nearest_reachable",
            ]
        }
        if facts
        else None,
        "loc_count": len(locs),
        "player_distance_min": min(dists) if dists else None,
        "player_distance_max": max(dists) if dists else None,
        "field_distance_min": min(field) if field else None,
        "field_distance_max": max(field) if field else None,
        "reachable_true": len(reach_true),
        "reachable_adj_true": len(adj_true),
        "all_adj_true": bool(locs) and all(loc.get("reachable_adj") for loc in locs),
        "nearest_adj": adj_true[0] if adj_true else None,
        "farthest_adj": adj_true[-1] if adj_true else None,
        "sample_first3": locs[:3],
        "sample_last3": locs[-3:],
    }


def main() -> None:
    out = {
        "r289": summarize("r289-flax-aio-pick-100adccc-resource2d589.log"),
        "r274": summarize("r274-flax-aio-pick-100adccc-resource2d589.log"),
    }
    dest = ROOT / "docs/compat/evidence/bounded-reachability-audit/flax-loc-facts.json"
    dest.write_text(json.dumps(out, indent=2) + "\n")
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
