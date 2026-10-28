#!/usr/bin/env python3
"""Audit T1.4 routes with the production router, not endpoint rectangles alone.

Run the compiled host-play example `soak_routes` on the reference v17 pack.
Every standable tile in the approved area is a possible replan origin. Every
resource perimeter tile, return observation tile, Start, and bank access tile
is an alternative goal. Check the complete route for each reachable pair.
"""

import argparse
import ast
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def contains(rectangles, tile):
    x, z, level = tile
    return level == 0 and any(a <= x <= b and c <= z <= d for a, b, c, d in rectangles)


def route_violations(row, rectangles, hunters, cache=None):
    failures = []
    for tile in row.get("tiles", []):
        key = tuple(tile)
        if cache is not None and key in cache:
            failures.extend(cache[key])
            continue
        found = []
        if not contains(rectangles, tile):
            found.append({"tile": tile, "reason": "outside-approved-area"})
        for hunter in hunters:
            x, z, level = hunter["spawn"]
            if tile[2] == level and max(abs(tile[0] - x), abs(tile[1] - z)) <= hunter["envelope"]:
                found.append({"tile": tile, "reason": "hunter-envelope", "hunter": hunter})
        if cache is not None:
            cache[key] = found
        failures.extend(found)
    return failures


def map_rows(path, section):
    active = False
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if line.startswith("===="):
            active = line == f"==== {section} ===="
        elif active and line.strip():
            position, values = line.split(":", 1)
            yield number, tuple(map(int, position.split())), tuple(map(int, values.split()))


def blocks(path):
    return dict(re.findall(r"^\[([^]]+)\]\n(.*?)(?=^\[|\Z)", path.read_text(), re.M | re.S))


def fixture(source):
    text = source.read_text()
    starts = ast.literal_eval(re.search(r"const NORMAL_STARTS[^=]*= (\[.*?\]);", text, re.S)[1])
    area = re.search(r"const NORMAL_SAFE_WALK[^=]*= &(\[.*?\]);", text, re.S)[1]
    rectangles = [tuple(map(int, match)) for match in re.findall(
        r"min_x: (\d+),\s*max_x: (\d+),\s*min_z: (\d+),\s*max_z: (\d+)", area)]
    if len(starts) != 10 or len(set(starts)) != 10 or not rectangles:
        raise ValueError("fixture requires ten distinct Start anchors and explicit route areas")
    return starts, rectangles


def census(content, game, starts, rectangles):
    locs = {row["id"]: row["config"] for row in game["loc_names"]["rows"]}
    tables = blocks(content / "scripts/skill_woodcutting/configs/trees.dbrow")
    normal = set(re.findall(r"^data=tree,(.+)$", tables["normal_tree_table"], re.M))
    maple = set(re.findall(r"^data=tree,(.+)$", tables["maple_tree_table"], re.M))
    dimensions = {}
    for path in (content / "scripts/skill_woodcutting/configs/trees").glob("*.loc"):
        for name, body in blocks(path).items():
            values = dict(re.findall(r"^(width|length)=(\d+)$", body, re.M))
            dimensions[name] = (int(values.get("width", 1)), int(values.get("length", 1)))
    resources = []
    sources = set()
    for mx in range(min(r[0] for r in rectangles) // 64, max(r[1] for r in rectangles) // 64 + 1):
        for mz in range(min(r[2] for r in rectangles) // 64, max(r[3] for r in rectangles) // 64 + 1):
            path = content / f"maps/m{mx}_{mz}.jm2"
            sources.add(path)
            for number, (level, x, z), values in map_rows(path, "LOC"):
                ident = values[0]
                name = locs.get(ident)
                x, z = mx * 64 + x, mz * 64 + z
                kind = "normal" if name in normal else "maple" if name in maple else None
                if level or kind is None:
                    continue
                if kind == "normal":
                    owners = [i for i, (sx, sz) in enumerate(starts) if max(abs(x - sx), abs(z - sz)) <= 2]
                else:
                    # Cover all maple origins in the selected Site, including
                    # every possible retained Site anchor and its radius-12 area.
                    site = next(s for s in game["gather_sites"] if s["id"] == "woodcutting.seers")
                    r = site["region"]
                    owners = ["maple"] if r["min_x"] <= x <= r["max_x"] and r["min_z"] <= z <= r["max_z"] else []
                if not owners:
                    continue
                if kind == "normal" and len(owners) != 1:
                    raise ValueError(f"normal resource shared by patches: {(x, z)}")
                width, height = dimensions[name]
                if len(values) > 2 and values[2] % 2:
                    width, height = height, width
                resources.append(dict(kind=kind, id=ident, origin=[x, z, 0], width=width,
                                      height=height, owners=owners, source=f"{path}:{number}"))
    for index in range(10):
        if not any(index in row["owners"] for row in resources):
            raise ValueError(f"normal patch {index} contains no authored resource")
    if not any(row["kind"] == "maple" for row in resources):
        raise ValueError("no authored maple resources")
    # Include every authored hunter, not only the five mountain types.
    # This deliberately ignores combat-level immunity and collision shielding.
    definitions = {row["id"]: row for row in game["npc_names"]["rows"]}
    hunters = []
    for placement in game["npc_placements"]["rows"]:
        definition = definitions[placement["npc_id"]]
        if definition["huntrange"] <= 0:
            continue
        hunters.append(dict(name=definition["config"], spawn=[placement["x"], placement["z"], placement["plane"]],
                            envelope=definition["wanderrange"] + definition["huntrange"] + definition["size"]))
    return resources, hunters, sources


def goals_for(resource):
    x, z, level = resource["origin"]
    width, height = resource["width"], resource["height"]
    # The full footprint plus one-tile ring is a conservative superset of
    # Reach faces and Area-r1 return observations, including depleted stumps.
    return {(a, b, level) for a in range(x - 1, x + width + 1)
            for b in range(z - 1, z + height + 1)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--router", type=Path, required=True)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--content", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = ROOT / "crates/host-play/tests/t14_soak_live.rs"
    catalog = ROOT / "crates/api/data/game-data/289.json"
    game = json.loads(catalog.read_text())
    starts, rectangles = fixture(source)
    resources, hunters, maps = census(args.content, game, starts, rectangles)
    origins = {(x, z, 0) for a, b, c, d in rectangles for x in range(a, b + 1) for z in range(c, d + 1)}
    targets = {(x, z, 0) for x, z in starts} | {(2727, 3501, 0)}
    for resource in resources:
        targets.update(goals_for(resource))
    for bank in game["bank_placements"]["rows"]:
        if bank["name"] == "Seers":
            targets.update((bank["x"] + dx, bank["z"] + dz, bank["level"])
                           for dx, dz in ((-1, 0), (1, 0), (0, -1), (0, 1)))
    inputs = args.output / "batches.json"
    inputs.write_text(json.dumps([dict(name="chosen-fixture", sources=sorted(origins), targets=sorted(targets))]) + "\n")
    routes = args.output / "routes.jsonl"
    subprocess.run([str(args.router.resolve()), str(args.pack.resolve()), str(inputs.resolve()), str(routes.resolve())], check=True)
    checked, no_path, blocked = 0, 0, set()
    violations = []
    reached = set()
    tile_cache = {}
    for line in routes.open():
        row = json.loads(line)
        if row.get("error") == "source-not-standable":
            blocked.add(tuple(row["source"]))
        elif "error" in row:
            no_path += 1
        else:
            checked += 1
            reached.add(tuple(row["target"]))
            failures = route_violations(row, rectangles, hunters, tile_cache)
            if failures:
                violations.append(dict(source=row["source"], target=row["target"], failures=failures[:8]))
    errors = []
    if not checked:
        errors.append("no actual route witnesses")
    for start in starts:
        if tuple((*start, 0)) in blocked or tuple((*start, 0)) not in reached:
            errors.append(f"Start is not reachable and standable: {start}")
    for resource in resources:
        if not goals_for(resource).intersection(reached):
            errors.append(f"resource has no reachable access goal: {resource['origin']}")
    result = dict(request_id="T14-SOAK-ROUTE", verdict="FAIL" if violations or errors else "PASS",
                  routes_checked=checked, unreachable_pairs=no_path, nonstandable_origins=len(blocked),
                  rectangles=rectangles, normal_starts=starts, resources=resources, hunters_checked=len(hunters),
                  sources={str(p): digest(p) for p in [source, catalog, args.pack, *sorted(maps)]},
                  violations=violations[:30], errors=errors,
                  scope="All production-router routes from every approved tile, including blocked-origin escape, to every standable alternative resource/return/bank goal. No rectangle-only inference. Random-event interiors remain separate product travel.")
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("verdict", "routes_checked", "unreachable_pairs", "nonstandable_origins", "hunters_checked", "errors")}))
    return 1 if violations or errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
