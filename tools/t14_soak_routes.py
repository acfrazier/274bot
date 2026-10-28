#!/usr/bin/env python3
"""Audit actual T1.4 routes on the reference v17 pack.

Seed every Start, resource footprint/access/return tile, bank access tile, and
production hazard-retreat stand. Query every alternative goal from each seed
and every intermediate route tile until the replan-origin set is closed.
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
    maple_start = ast.literal_eval(re.search(r"const MAPLE_START[^=]*= (\(.*?\));", text)[1])
    area = re.search(r"const NORMAL_SAFE_WALK[^=]*= &(\[.*?\]);", text, re.S)[1]
    rectangles = [tuple(map(int, match)) for match in re.findall(
        r"min_x: (\d+),\s*max_x: (\d+),\s*min_z: (\d+),\s*max_z: (\d+)", area)]
    if len(starts) != 10 or len(set(starts)) != 10 or not rectangles:
        raise ValueError("fixture requires ten distinct Start anchors and explicit route areas")
    return starts, maple_start, rectangles


def census(content, game, starts, rectangles, maple_site):
    locs = {row["id"]: row["config"] for row in game["loc_names"]["rows"]}
    tables = blocks(content / "scripts/skill_woodcutting/configs/trees.dbrow")
    normal = set(re.findall(r"^data=tree,(.+)$", tables["normal_tree_table"], re.M))
    maple = set(re.findall(r"^data=tree,(.+)$", tables["maple_tree_table"], re.M))
    dimensions = {}
    for path in (content / "scripts/skill_woodcutting/configs/trees").glob("*.loc"):
        for name, body in blocks(path).items():
            values = dict(re.findall(r"^(width|length)=(\d+)$", body, re.M))
            dimensions[name] = (int(values.get("width", 1)), int(values.get("length", 1)))
    site = next(s for s in game["gather_sites"] if s["id"] == maple_site)
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
                    # Cover the whole Site, not only its first retained anchor.
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
    # Ignore combat-level immunity and collision shielding: audit every hunter.
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
    # Superset of Reach faces and Area-r1 returns, including depleted stumps.
    return {(a, b, level) for a in range(x - 1, x + width + 1)
            for b in range(z - 1, z + height + 1)}


def run_probe(args, name, sources, targets, retreats=()):
    inputs = args.output / f"{name}-batches.json"
    inputs.write_text(json.dumps([dict(name=name, sources=sorted(sources),
                                      targets=sorted(targets), retreats=list(retreats))]) + "\n")
    routes = args.output / f"{name}-routes.jsonl"
    subprocess.run([str(args.router.resolve()), str(args.pack.resolve()), str(inputs.resolve()), str(routes.resolve())], check=True)
    return routes


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
    config = ROOT / "T1.4/soak-config.json"
    game = json.loads(catalog.read_text())
    sites = json.loads(config.read_text())["workload"]["site_names"]
    starts, maple_start, rectangles = fixture(source)
    resources, hunters, maps = census(args.content, game, starts, rectangles, sites[1])
    anchors = {(*start, 0) for start in starts} | {(*maple_start, 0)}
    targets = set(anchors)
    for resource in resources:
        targets.update(goals_for(resource))
    bank_goals = set()
    for bank in game["bank_placements"]["rows"]:
        if bank["name"] == "Seers":
            bank_goals.update(goals_for(dict(origin=[bank["x"], bank["z"], bank["level"]],
                                            width=bank["width"], height=bank["length"])))
    targets.update(bank_goals)
    retreats = [(here, resource["origin"]) for resource in resources for here in sorted(goals_for(resource))]
    files = [run_probe(args, "retreat", (), (), retreats)]
    checked, no_path, retreat_count = 0, 0, 0
    blocked, processed, bank_reachable, resource_reached = set(), set(), set(), set()
    pending = set(targets)
    access = {}
    for index, resource in enumerate(resources):
        for owner in resource["owners"]:
            start = (*maple_start, 0) if owner == "maple" else (*starts[owner], 0)
            for target in goals_for(resource):
                access.setdefault((start, target), set()).add(index)
    violations, tile_cache = [], {}
    round_number = 0
    while files:
        routes = files.pop(0)
        for line in routes.open():
            row = json.loads(line)
            origin = tuple(row["source"])
            if row.get("error") == "source-not-standable":
                blocked.add(origin)
            elif "error" in row:
                no_path += 1
            else:
                checked += 1
                target = tuple(row["target"])
                if row["kind"] == "hazard-retreat":
                    retreat_count += 1
                    targets.add(target)
                    pending.add(target)
                if target in bank_goals:
                    bank_reachable.add(origin)
                resource_reached.update(access.get((origin, target), ()))
                pending.update(tuple(t) for t in row["tiles"])
                failures = route_violations(row, rectangles, hunters, tile_cache)
                if failures:
                    violations.append(dict(source=row["source"], target=row["target"],
                                           kind=row["kind"], failures=failures[:8]))
        pending.difference_update(processed)
        if violations or not pending:
            break
        origins = pending
        pending = set()
        processed.update(origins)
        files.append(run_probe(args, f"closure-{round_number}", origins, targets))
        round_number += 1
    errors = []
    for hunter in hunters:
        x, z, level = hunter["spawn"]
        radius = hunter["envelope"]
        if level == 0 and any(a - radius <= x <= b + radius and c - radius <= z <= d + radius
                              for a, b, c, d in rectangles):
            errors.append(f"approved area intersects hunter envelope: {hunter}")
    if not checked:
        errors.append("no actual route witnesses")
    for start in sorted(anchors):
        if start in blocked or start not in bank_reachable:
            errors.append(f"Start is not standable with a bank route: {start}")
    for index, resource in enumerate(resources):
        if index not in resource_reached:
            errors.append(f"resource has no access route from its Start: {resource['origin']}")
    isolated = processed - blocked - bank_reachable
    if isolated:
        errors.append(f"{len(isolated)} standable origins have no bank-access route")
    result = dict(request_id="T14-SOAK-ROUTE", verdict="FAIL" if violations or errors else "PASS",
                  routes_checked=checked, unreachable_pairs=no_path, nonstandable_origins=len(blocked),
                  hazard_retreat_routes=retreat_count, replan_origins=len(processed), closure_rounds=round_number,
                  closure_complete=not pending, rectangles=rectangles, normal_starts=starts,
                  maple_start=maple_start, resources=resources, hunters_checked=len(hunters),
                  sources={str(p): digest(p) for p in [source, catalog, config, args.pack, *sorted(maps)]},
                  violation_count=len(violations), violations=violations[:30], errors=errors,
                  scope="Production-router replan closure over all alternative resource/return/bank goals, including blocked-origin escape and production hazard retreats. Random-event interiors remain separate product travel.")
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("verdict", "routes_checked", "unreachable_pairs", "nonstandable_origins", "hazard_retreat_routes", "replan_origins", "closure_rounds", "closure_complete", "hunters_checked", "violation_count", "errors")}))
    return 1 if violations or errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
