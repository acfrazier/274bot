#!/usr/bin/env python3
"""Audit actual T1.4 routes on the reference v17 pack.

Seed every Start, resource footprint/access/return tile, bank access tile, and
production hazard-retreat stand. Query every alternative goal from each seed
and every intermediate route tile until the replan-origin set is closed. Check
the client's own BFS for loc interactions, walk aims, and Guardian travel.
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
            found.append({"tile": tile, "reason": "outside-outer-bound"})
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
    radius = int(re.search(r"const NORMAL_RADIUS: i32 = (\d+);", text)[1])
    area = re.search(r"const NORMAL_SAFE_WALK[^=]*= &(\[.*?\]);", text, re.S)[1]
    rectangles = [tuple(map(int, match)) for match in re.findall(
        r"min_x: (\d+),\s*max_x: (\d+),\s*min_z: (\d+),\s*max_z: (\d+)", area)]
    if len(starts) != 10 or len(set(starts)) != 10 or not rectangles:
        raise ValueError("fixture requires ten distinct Start anchors and explicit route areas")
    return starts, maple_start, rectangles, radius


def outer_walk_bounds(source):
    text = source.read_text()
    area = re.search(r"const OUTER_WALK_BOUNDS[^=]*= &(\[.*?\]);", text, re.S)[1]
    rectangles = [tuple(map(int, match)) for match in re.findall(
        r"min_x: (\d+),\s*max_x: (\d+),\s*min_z: (\d+),\s*max_z: (\d+)", area)]
    if not rectangles:
        raise ValueError("fixture requires an explicit outer walk bound")
    return rectangles


def census(content, game, starts, rectangles, maple_site, normal_radius):
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
    bank_rows = {
        (bank["x"], bank["z"], bank["level"]): bank
        for bank in game["bank_placements"]["rows"]
        if bank["name"] == "Seers" and bank["kind"] == "object"
    }
    bank_locs = []
    for mx in range(min(r[0] for r in rectangles) // 64, max(r[1] for r in rectangles) // 64 + 1):
        for mz in range(min(r[2] for r in rectangles) // 64, max(r[3] for r in rectangles) // 64 + 1):
            path = content / f"maps/m{mx}_{mz}.jm2"
            sources.add(path)
            for number, (level, x, z), values in map_rows(path, "LOC"):
                ident = values[0]
                name = locs.get(ident)
                x, z = mx * 64 + x, mz * 64 + z
                bank = bank_rows.get((x, z, level))
                if bank is not None and ident == bank["id"]:
                    bank_locs.append(dict(kind="bank", id=ident, origin=[x, z, level],
                                          width=bank["width"], height=bank["length"],
                                          shape=values[1], angle=values[2] if len(values) > 2 else 0))
                kind = "normal" if name in normal else "maple" if name in maple else None
                if level or kind is None:
                    continue
                if kind == "normal":
                    owners = [i for i, (sx, sz) in enumerate(starts)
                              if max(abs(x - sx), abs(z - sz)) <= normal_radius]
                else:
                    # Cover the whole Site, not only its first retained anchor.
                    r = site["region"]
                    owners = ["maple"] if r["min_x"] <= x <= r["max_x"] and r["min_z"] <= z <= r["max_z"] else []
                if not owners:
                    continue
                width, height = dimensions[name]
                if len(values) > 2 and values[2] % 2:
                    width, height = height, width
                resources.append(dict(kind=kind, id=ident, origin=[x, z, 0], width=width,
                                      height=height, owners=owners, source=f"{path}:{number}",
                                      shape=values[1], angle=values[2] if len(values) > 2 else 0))
    if not any(row["kind"] == "maple" for row in resources):
        raise ValueError("no authored maple resources")
    if len(bank_locs) != len(bank_rows):
        raise ValueError("selected-content maps do not contain every Seers bank loc")
    return resources, hunter_envelopes(game), sources, bank_locs


def hunter_envelopes(game):
    # Ignore combat-level immunity and collision shielding: audit every hunter.
    definitions = {row["id"]: row for row in game["npc_names"]["rows"]}
    hunters = []
    for placement in game["npc_placements"]["rows"]:
        definition = definitions[placement["npc_id"]]
        if definition["huntrange"] <= 0:
            continue
        hunters.append(dict(name=definition["config"], spawn=[placement["x"], placement["z"], placement["plane"]],
                            envelope=definition["wanderrange"] + definition["huntrange"] + definition["size"]))
    return hunters


def normal_patch_capacity(resources, patch_count):
    counts = [sum(row["kind"] == "normal" and index in row["owners"] for row in resources)
              for index in range(patch_count)]
    for index, count in enumerate(counts):
        if count < 2:
            raise ValueError(f"normal patch {index} requires at least two eligible trees, found {count}")
    return counts


def goals_for(resource):
    x, z, level = resource["origin"]
    width, height = resource["width"], resource["height"]
    # Superset of Reach faces and Area-r1 returns, including depleted stumps.
    return {(a, b, level) for a in range(x - 1, x + width + 1)
            for b in range(z - 1, z + height + 1)}


def client_approaches(resources):
    return [
        dict(source=source, target=resource["origin"], id=resource["id"],
             shape=resource["shape"], angle=resource["angle"])
        for resource in resources for source in sorted(goals_for(resource))
    ]


def client_walks(origins, blocked, checked, recoveries=()):
    """Cover every possible Traveller aim, including click-ahead and recovery.

    pick_aim selects the last tile within Chebyshev 20, or a tile 15 steps
    ahead. Scene clipping only shortens that aim. Query both directions in
    the closed origin set. Also reissue each accepted aim from every tile
    of its client BFS detour, including recovery distances greater than 20.
    """
    walks = []
    targets = sorted(origins - blocked)
    for source in sorted(origins):
        for target in targets:
            pair = source, target
            if pair in checked or source[2] != target[2]:
                continue
            if 0 < max(abs(source[0] - target[0]), abs(source[1] - target[1])) <= 20:
                checked.add(pair)
                walks.append(dict(source=source, target=target))
    for source, target in sorted(recoveries):
        pair = source, target
        if pair not in checked and source != target and target not in blocked:
            checked.add(pair)
            walks.append(dict(source=source, target=target))
    return walks


def guardian_requests(game, origins):
    # is_evade_type (host/random.rs). IDs with equal selected-content ranges
    # share the movement policy. Exercise one real Guardian per range group.
    groups = {}
    for npc in game["npc_names"]["rows"]:
        ident = npc["id"]
        if (391 <= ident <= 396 or ident in (408, 411)
                or 413 <= ident <= 436 or 438 <= ident <= 443):
            key = npc["maxrange"], npc["wanderrange"]
            groups.setdefault(key, []).append(ident)
    if not groups:
        raise ValueError("selected content has no Guardian evade ranges")
    ranges = [dict(maxrange=key[0], wanderrange=key[1], npc_ids=sorted(ids))
              for key, ids in sorted(groups.items())]
    # macro_events.rs2 uses map_findsquare(coord, 1, 1, lineofwalk).
    # Include the same-tile fallback and every adjacent spawn direction.
    requests = [dict(source=source, npc_id=min(ids), offset=[dx, dz])
                for source in sorted(origins) for ids in groups.values()
                for dx in range(-1, 2) for dz in range(-1, 2)]
    return requests, ranges


def guardian_hazards(origins):
    # Superset of the real Guardian's ordered HAZARD_STEP=4 compass ring.
    return [dict(source=(x, z, level), target=(x + dx * 4, z + dz * 4, level))
            for x, z, level in sorted(origins)
            for dx in range(-1, 2) for dz in range(-1, 2) if dx or dz]


def run_probe(args, name, sources, targets, retreats=(), approaches=(), walks=(),
              guardians=(), hazards=()):
    payload = [dict(name=name, sources=sorted(sources), targets=sorted(targets),
                    retreats=list(retreats), approaches=list(approaches), walks=list(walks),
                    guardians=list(guardians), hazards=list(hazards))]
    if args.replay:
        inputs = args.replay / f"{name}-batches.json"
        # JSON normalizes the generated coordinate tuples to the stored arrays.
        if json.loads(inputs.read_text()) != json.loads(json.dumps(payload)):
            raise ValueError(f"replay requests differ from the actual producer input: {inputs}")
        routes = args.replay / f"{name}-routes.jsonl"
        if not routes.is_file():
            raise FileNotFoundError(f"replay route output missing: {routes}")
        return routes
    inputs = args.output / f"{name}-batches.json"
    inputs.write_text(json.dumps(payload) + "\n")
    routes = args.output / f"{name}-routes.jsonl"
    subprocess.run([str(path.resolve()) for path in
                    (args.router, args.pack, args.flags, args.cache, inputs, routes)], check=True)
    return routes


def client_route_error(row):
    # tryMove returns false before encoding movement for either request kind.
    return (row.get("kind") in ("client-approach", "client-walk",
                                "guardian-flee", "guardian-return", "guardian-hazard")
            and row["error"] != "client-no-route")


def loc_key(row):
    return row["id"], tuple(row.get("target", row.get("origin"))), row["shape"], row["angle"]


def missing_client_approaches(locs, witnesses):
    return [f"loc has no actual client approach witness: {loc['id']} at {loc['origin']}"
            for loc in locs if loc_key(loc) not in witnesses]


def verify_replay_sources(replay, sources):
    previous = json.loads((replay / "result.json").read_text())
    if previous["sources"] != sources:
        raise ValueError("replay inputs differ from the actual producer's selected inputs")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--router", type=Path, required=True)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--content", type=Path, required=True)
    parser.add_argument("--flags", type=Path, help="raw flags paired with PACK (default: its .navflags)")
    parser.add_argument("--cache", type=Path, help="real immutable client cache (default: CONTENT/../engine/data/pack/client)")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--replay", type=Path, help="recheck retained native outputs with identical inputs and request batches")
    args = parser.parse_args()
    args.flags = args.flags or args.pack.with_suffix(".navflags")
    args.cache = args.cache or args.content.parent / "engine/data/pack/client"
    args.output.mkdir(parents=True, exist_ok=False)
    source = ROOT / "crates/host-play/tests/t14_soak_live.rs"
    catalog = ROOT / "crates/api/data/game-data/289.json"
    config = ROOT / "T1.4/soak-config.json"
    game = json.loads(catalog.read_text())
    sites = json.loads(config.read_text())["workload"]["site_names"]
    starts, maple_start, rectangles, normal_radius = fixture(source)
    outer_bounds = outer_walk_bounds(source)
    resources, hunters, maps, bank_locs = census(
        args.content, game, starts, rectangles, sites[1], normal_radius)
    patch_counts = normal_patch_capacity(resources, len(starts))
    sources = {str(p): digest(p) for p in [
        source, catalog, config, args.pack, args.flags, args.cache / "config", *sorted(maps),
        ROOT / "crates/host/src/random.rs", ROOT / "crates/host/src/random/guardian.rs",
        ROOT / "crates/host-play/examples/soak_routes.rs",
    ]}
    if args.replay:
        args.replay = args.replay.resolve(strict=True)
        verify_replay_sources(args.replay, sources)
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
    guardian_origins = set(targets)
    guardians, guardian_ranges = guardian_requests(game, guardian_origins)
    retreats = [(here, resource["origin"]) for resource in resources for here in sorted(goals_for(resource))]
    files = [run_probe(args, "interactions", (), (), retreats,
                       client_approaches([*resources, *bank_locs])),
             run_probe(args, "guardians", (), (), guardians=guardians,
                       hazards=guardian_hazards(guardian_origins))]
    checked, no_path, retreat_count, client_count, walk_count = 0, 0, 0, 0, 0
    client_errors = []
    approach_witnesses = set()
    walk_pairs, transport_routes = set(), []
    walk_recoveries = set()
    blocked, processed, bank_reachable, resource_reached = set(), set(), set(), set()
    pending = set(targets)
    guardian_counts = dict.fromkeys(("guardian-flee", "guardian-return", "guardian-hazard"), 0)
    guardian_scenarios = set()
    access = {}
    for index, resource in enumerate(resources):
        for owner in resource["owners"]:
            start = (*maple_start, 0) if owner == "maple" else (*starts[owner], 0)
            for target in goals_for(resource):
                access.setdefault((start, target), set()).add(index)
    violations, tile_cache = [], {}
    round_number, walk_rounds = 0, 0
    while files:
        routes = files.pop(0)
        for line in routes.open():
            row = json.loads(line)
            origin = tuple(row["source"])
            guardian_kind = row.get("kind", "").startswith("guardian-")
            if row.get("kind") == "guardian-flee":
                guardian_scenarios.add((origin, row["npc_id"], tuple(row["npc_start"])))
            if row.get("error") == "source-not-standable":
                blocked.add(origin)
            elif "error" in row:
                if client_route_error(row):
                    client_errors.append(row)
                no_path += 1
            else:
                checked += 1
                target = tuple(row["target"])
                if guardian_kind:
                    guardian_counts[row["kind"]] += 1
                if row["kind"] == "client-approach":
                    client_count += 1
                    approach_witnesses.add(loc_key(row))
                if row["kind"] == "client-walk":
                    walk_count += 1
                    walk_recoveries.update((tuple(point), target) for point in row["tiles"])
                if row.get("transports"):
                    transport_routes.append(dict(source=row["source"], target=row["target"]))
                if row["kind"] == "hazard-retreat":
                    retreat_count += 1
                    targets.add(target)
                    pending.add(target)
                if target in bank_goals:
                    bank_reachable.add(origin)
                resource_reached.update(access.get((origin, target), ()))
                if not guardian_kind:
                    pending.update(tuple(t) for t in row["tiles"])
                failures = route_violations(row, outer_bounds, hunters, tile_cache)
                if failures:
                    violations.append(dict(source=row["source"], target=row["target"],
                                           kind=row["kind"], failures=failures[:8]))
        pending.difference_update(processed)
        if violations:
            break
        if pending:
            origins = pending
            pending = set()
            processed.update(origins)
            files.append(run_probe(args, f"closure-{round_number}", origins, targets))
            round_number += 1
        else:
            walks = client_walks(processed, blocked, walk_pairs, walk_recoveries)
            if not walks:
                break
            files.append(run_probe(args, f"client-walk-{walk_rounds}", (), (), walks=walks))
            walk_rounds += 1
    errors = [f"client route has no verified witness: {row}" for row in client_errors[:10]]
    errors.extend(missing_client_approaches([*resources, *bank_locs], approach_witnesses))
    if not walk_count:
        errors.append("no actual client Traveller-walk witnesses")
    if len(guardian_scenarios) != len(guardians):
        errors.append(f"Guardian scenarios missing: {len(guardian_scenarios)} of {len(guardians)}")
    for kind, count in guardian_counts.items():
        if not count:
            errors.append(f"no actual {kind} witnesses")
    if transport_routes:
        errors.append(f"{len(transport_routes)} routes use unverified transport interactions")
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
                  hazard_retreat_routes=retreat_count, client_approach_routes=client_count,
                  client_walk_routes=walk_count, client_walk_pairs=len(walk_pairs),
                  replan_origins=len(processed), closure_rounds=round_number,
                  client_walk_rounds=walk_rounds, transport_routes=len(transport_routes),
                  closure_complete=not pending, rectangles=rectangles, outer_bounds=outer_bounds,
                  normal_starts=starts, guardian_routes=guardian_counts,
                  guardian_scenarios=len(guardian_scenarios), guardian_work_bank_tiles=len(guardian_origins),
                  guardian_ranges=guardian_ranges,
                  normal_radius=normal_radius, normal_patch_trees=patch_counts,
                  maple_start=maple_start, resources=resources, hunters_checked=len(hunters),
                  sources=sources, replay_source=str(args.replay) if args.replay else None,
                  violation_count=len(violations), violations=violations[:30], errors=errors,
                  scope="Production host-router replan closure plus actual client loc-approach BFS for every resource/bank access origin and scene base. Actual client ground-walk BFS covers every closure-origin pair within Chebyshev 20, including Traveller click-ahead and stalled recovery. Includes blocked-origin escape and production Gatherer hazard retreats. The real Guardian and Client exercise all content range groups and adjacent spawn directions from every worksite/bank tile, exact and nearest flee fallback, and exact returns from every accepted flee-path tile. Actual client BFS also checks every Guardian four-tile hazard-ring goal. Every accepted path must avoid all authored hunter envelopes and stay inside the wide outer bound. Worksite rectangles are census inputs, not stop conditions. Transport interactions fail closed. Random-event interiors remain separate product travel.")
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("verdict", "routes_checked", "unreachable_pairs", "nonstandable_origins", "hazard_retreat_routes", "client_approach_routes", "client_walk_routes", "client_walk_pairs", "replan_origins", "closure_rounds", "client_walk_rounds", "closure_complete", "hunters_checked", "violation_count", "errors")}))
    return 1 if violations or errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
