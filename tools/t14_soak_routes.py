#!/usr/bin/env python3
"""Audit scoped T1.4 workload routes on the reference v17 pack.

Seed each worksite Start, resource access/return tile, and bank access tile.
Close workload routes over every intermediate tile. Check Gatherer
retreats/re-approaches, native loc approaches, and client BFS walks for
fault-phase recovery. Random-event movement, including Guardian flee/return/
hazard routes and ordinary re-plans from those tiles, is excluded and not
proved safe.
"""

import argparse
import ast
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
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


def ordinary_gate_route(row):
    """Admit only identified, requirement-free, one-tile physical door edges."""
    edges = row.get("transport_edges")
    if not isinstance(edges, list) or len(edges) != row.get("transports"):
        return False
    for edge in edges:
        if edge.get("kind") != "Door" or edge.get("has_requirements") is not False:
            return False
        start, end = edge.get("at"), edge.get("to")
        if not isinstance(start, list) or not isinstance(end, list) or len(start) != 3 or len(end) != 3:
            return False
        if start[2] != end[2] or abs(start[0] - end[0]) + abs(start[1] - end[1]) != 1:
            return False
    return True


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


def rust_constant(text, name, pattern):
    match = re.search(rf"const {name}\b[^=]*= {pattern};", text, re.S)
    if not match:
        raise ValueError(f"fixture is missing {name}")
    return match[1]


def fixture(source):
    text = source.read_text()
    starts = ast.literal_eval(rust_constant(text, "NORMAL_STARTS", r"(\[.*?\])"))
    second_start = ast.literal_eval(rust_constant(text, "SECOND_TIER_START", r"(\(.*?\))"))
    normal_radius = int(rust_constant(text, "NORMAL_RADIUS", r"(\d+)"))
    second_radius = int(rust_constant(text, "SECOND_TIER_RADIUS", r"(\d+)"))
    area = re.search(r"const NORMAL_SAFE_WALK[^=]*= &(\[.*?\]);", text, re.S)[1]
    rectangles = [tuple(map(int, match)) for match in re.findall(
        r"min_x: (\d+),\s*max_x: (\d+),\s*min_z: (\d+),\s*max_z: (\d+)", area)]
    if len(starts) != 5 or len(set(starts)) != 5 or not rectangles:
        raise ValueError("fixture requires five distinct Start anchors and explicit route areas")
    if normal_radius < 0 or second_radius < 0:
        raise ValueError("fixture radii must be nonnegative")
    return starts, second_start, rectangles, normal_radius, second_radius


def second_tier_key(source):
    key = rust_constant(source.read_text(), "SECOND_TIER_KEY", r'"([^"]+)"')
    if key not in {"maple", "willow", "oak"}:
        raise ValueError(f"unsupported second-tier tree key: {key}")
    return key


def contains_xy(region, tile):
    x, z = tile
    return region["min_x"] <= x <= region["max_x"] and region["min_z"] <= z <= region["max_z"]


def census(content, game, starts, second_start, normal_radius, second_radius,
           second_key, normal_site_id, second_site_id, bank_name):
    locs = {row["id"]: row["config"] for row in game["loc_names"]["rows"]}
    tables = blocks(content / "scripts/skill_woodcutting/configs/trees.dbrow")
    normal_table = tables["normal_tree_table"]
    second_table_name = f"{second_key}_tree_table"
    if second_table_name not in tables:
        raise ValueError(f"selected content has no {second_table_name}")
    normal = set(re.findall(r"^data=tree,(.+)$", normal_table, re.M))
    second_tier = set(re.findall(r"^data=tree,(.+)$", tables[second_table_name], re.M))
    dimensions = {}
    for path in (content / "scripts/skill_woodcutting/configs/trees").glob("*.loc"):
        for name, body in blocks(path).items():
            values = dict(re.findall(r"^(width|length)=(\d+)$", body, re.M))
            dimensions[name] = (int(values.get("width", 1)), int(values.get("length", 1)))
    sites = {site["id"]: site for site in game["gather_sites"]}
    normal_site = sites[normal_site_id]
    second_site = sites[second_site_id]
    if not any(row["key"].casefold() == "normal" for row in normal_site["keys"]):
        raise ValueError(f"authored normal site {normal_site_id} does not support normal")
    if not any(row["key"].casefold() == second_key for row in second_site["keys"]):
        raise ValueError(f"authored second-tier site {second_site_id} does not support {second_key}")
    if any(not contains_xy(normal_site["region"], start) for start in starts):
        raise ValueError(f"authored normal site {normal_site_id} does not contain every Start")
    if not contains_xy(second_site["region"], second_start):
        raise ValueError(f"authored second-tier site {second_site_id} does not contain its Start")
    resources = []
    sources = set()
    bank_rows = {
        (bank["x"], bank["z"], bank["level"]): bank
        for bank in game["bank_placements"]["rows"]
        if bank["name"] == bank_name and bank["kind"] == "object"
    }
    bank_locs = []
    anchors = [(*start, 0) for start in starts] + [(*second_start, 0)]
    min_x = min(x - (normal_radius if index < len(starts) else second_radius)
                for index, (x, _, _) in enumerate(anchors))
    max_x = max(x + (normal_radius if index < len(starts) else second_radius)
                for index, (x, _, _) in enumerate(anchors))
    min_z = min(z - (normal_radius if index < len(starts) else second_radius)
                for index, (_, z, _) in enumerate(anchors))
    max_z = max(z + (normal_radius if index < len(starts) else second_radius)
                for index, (_, z, _) in enumerate(anchors))
    if bank_rows:
        min_x = min(min_x, min(x for x, _, _ in bank_rows))
        max_x = max(max_x, max(x for x, _, _ in bank_rows))
        min_z = min(min_z, min(z for _, z, _ in bank_rows))
        max_z = max(max_z, max(z for _, z, _ in bank_rows))
    for mx in range(min_x // 64, max_x // 64 + 1):
        for mz in range(min_z // 64, max_z // 64 + 1):
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
                kind = "normal" if name in normal else "second-tier" if name in second_tier else None
                if level or kind is None:
                    continue
                if kind == "normal":
                    owners = [i for i, (sx, sz) in enumerate(starts)
                              if max(abs(x - sx), abs(z - sz)) <= normal_radius]
                else:
                    owners = ["second-tier"] if max(
                        abs(x - second_start[0]), abs(z - second_start[1])) <= second_radius else []
                if not owners:
                    continue
                width, height = dimensions[name]
                if len(values) > 2 and values[2] % 2:
                    width, height = height, width
                resources.append(dict(kind=kind, id=ident, origin=[x, z, 0], width=width,
                                      height=height, owners=owners, source=f"{path}:{number}",
                                      shape=values[1], angle=values[2] if len(values) > 2 else 0))
    if sum(row["kind"] == "second-tier" for row in resources) < 2:
        raise ValueError(f"fewer than two authored second-tier {second_key} resources within its Start radius")
    if len(bank_locs) != len(bank_rows):
        raise ValueError(f"selected-content maps do not contain every {bank_name} bank loc")
    return resources, hunter_envelopes(game), sources, bank_locs

def outer_walk_bounds(source):
    text = source.read_text()
    area = re.search(r"const OUTER_WALK_BOUNDS[^=]*= &(\[.*?\]);", text, re.S)[1]
    rectangles = [tuple(map(int, match)) for match in re.findall(
        r"min_x: (\d+),\s*max_x: (\d+),\s*min_z: (\d+),\s*max_z: (\d+)", area)]
    if not rectangles:
        raise ValueError("fixture requires an explicit outer walk bound")
    return rectangles


def fixture_bank(source):
    return re.search(r'const SOAK_BANK: &str = "([^"]+)";', source.read_text())[1]




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


def normal_reachable_matching(reachable):
    """Return a capacity witness, not a product tree assignment."""
    for owner, trees in enumerate(reachable):
        if len(trees) < 2:
            raise ValueError(
                f"normal Start {owner} requires at least two reachable trees, found {len(trees)}")
    matched = {}

    def augment(owner, seen):
        for tree in sorted(reachable[owner]):
            if tree in seen:
                continue
            seen.add(tree)
            if tree not in matched or augment(matched[tree], seen):
                matched[tree] = owner
                return True
        return False

    for owner in range(len(reachable)):
        if not augment(owner, set()):
            raise ValueError("normal Starts have no full distinct-tree reachability matching")
    witness = [None] * len(reachable)
    for tree, owner in matched.items():
        witness[owner] = tree
    return witness


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


def run_probe(args, name, sources, targets, retreats=(), approaches=(), walks=()):
    payload = [dict(name=name, sources=sorted(sources), targets=sorted(targets),
                    retreats=list(retreats), approaches=list(approaches), walks=list(walks),
                    guardians=[], hazards=[])]
    if args.replay:
        inputs = args.replay / f"{name}-batches.json"
        # JSON normalizes the generated coordinate tuples to the stored arrays.
        if json.loads(inputs.read_text()) != json.loads(json.dumps(payload)):
            raise ValueError(f"replay requests differ from the actual producer input: {inputs}")
        routes = args.replay / f"{name}-routes.jsonl.gz"
        if not routes.is_file():
            raise FileNotFoundError(f"replay route output missing: {routes}")
        return routes
    inputs = args.output / f"{name}-batches.json"
    inputs.write_text(json.dumps(payload) + "\n")
    routes = args.output / f"{name}-routes.jsonl.gz"
    command = [str(path.resolve()) for path in
               (args.router, args.pack, args.flags, args.cache, inputs)]
    command.append("/dev/stdout")
    # Retain every native row losslessly without duplicating enormous raw
    # route files. The producer still owns all routing and path extraction.
    with subprocess.Popen(command, stdout=subprocess.PIPE) as producer:
        with gzip.open(routes, "wb", compresslevel=1) as output:
            shutil.copyfileobj(producer.stdout, output)
        returncode = producer.wait()
        if returncode:
            raise subprocess.CalledProcessError(returncode, command)
    return routes


def client_route_error(row):
    # tryMove returns false before encoding movement for either request kind.
    return (row.get("kind") in ("client-approach", "client-walk")
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
    starts, second_start, rectangles, normal_radius, second_radius = fixture(source)
    second_key = second_tier_key(source)
    outer_bounds = outer_walk_bounds(source)
    bank_name = fixture_bank(source)
    resources, hunters, maps, bank_locs = census(
        args.content, game, starts, second_start, normal_radius, second_radius,
        second_key, sites[0], sites[1], bank_name)
    patch_counts = normal_patch_capacity(resources, len(starts))
    sources = {str(p): digest(p) for p in [
        source, catalog, config, args.pack, args.flags, args.cache / "config", *sorted(maps),
        ROOT / "crates/host/src/random.rs", ROOT / "crates/host/src/random/guardian.rs",
        ROOT / "crates/host-play/examples/soak_routes.rs",
        args.router.resolve(strict=True),
        args.content / "scripts/general_use/configs/gates.loc",
        args.content / "scripts/general_use/scripts/gates.rs2",
    ]}
    if args.replay:
        args.replay = args.replay.resolve(strict=True)
        verify_replay_sources(args.replay, sources)
    anchors = {(*start, 0) for start in starts} | {(*second_start, 0)}
    targets = set(anchors)
    for resource in resources:
        targets.update(goals_for(resource))
    bank_goals = set()
    for bank in game["bank_placements"]["rows"]:
        if bank["name"] == bank_name:
            bank_goals.update(goals_for(dict(origin=[bank["x"], bank["z"], bank["level"]],
                                            width=bank["width"], height=bank["length"])))
    targets.update(bank_goals)
    # The unchanged native producer includes random-event code in its source
    # identity, but this workload proof never invokes that movement code.
    retreats = [(here, resource["origin"]) for resource in resources for here in sorted(goals_for(resource))]
    files = [run_probe(args, "interactions", (), (), retreats,
                       client_approaches([*resources, *bank_locs]))]
    checked, no_path, retreat_count, client_count, walk_count = 0, 0, 0, 0, 0
    client_errors = []
    approach_witnesses = set()
    walk_pairs, unverified_transport_routes = set(), []
    ordinary_gate_routes = 0
    ordinary_gate_locs = set()
    walk_recoveries = set()
    blocked, processed, bank_reachable, resource_reached = set(), set(), set(), set()
    normal_reachable = [set() for _ in starts]
    pending = set(targets)
    access = {}
    for index, resource in enumerate(resources):
        for owner in resource["owners"]:
            start = (*second_start, 0) if owner == "second-tier" else (*starts[owner], 0)
            for target in goals_for(resource):
                access.setdefault((start, target), set()).add((index, owner))
    violations, tile_cache = [], {}
    round_number, walk_rounds = 0, 0
    while files:
        routes = files.pop(0)
        with gzip.open(routes, "rt") as route_file:
            for line in route_file:
                row = json.loads(line)
                origin = tuple(row["source"])
                if row.get("error") == "source-not-standable":
                    blocked.add(origin)
                elif "error" in row:
                    if client_route_error(row):
                        client_errors.append(row)
                    no_path += 1
                else:
                    checked += 1
                    target = tuple(row["target"])
                    if row["kind"] == "client-approach":
                        client_count += 1
                        approach_witnesses.add(loc_key(row))
                    if row["kind"] == "client-walk":
                        walk_count += 1
                        walk_recoveries.update((tuple(point), target) for point in row["tiles"])
                    if row.get("transports"):
                        if ordinary_gate_route(row):
                            ordinary_gate_routes += 1
                            ordinary_gate_locs.update(edge["loc_id"] for edge in row["transport_edges"])
                        else:
                            unverified_transport_routes.append(dict(
                                source=row["source"], target=row["target"],
                                transport_edges=row.get("transport_edges")))
                    if row["kind"] == "hazard-retreat":
                        retreat_count += 1
                        targets.add(target)
                        pending.add(target)
                    if row["kind"] == "route":
                        if target in bank_goals:
                            bank_reachable.add(origin)
                        for index, owner in access.get((origin, target), ()):
                            resource_reached.add(index)
                            if owner != "second-tier":
                                normal_reachable[owner].add(index)
                    pending.update(tuple(t) for t in row["tiles"])
                    failures = route_violations(row, outer_bounds, hunters, tile_cache)
                    if failures:
                        violations.append(dict(source=row["source"], target=row["target"],
                                               kind=row["kind"], failures=failures[:8]))
                if violations or client_errors or unverified_transport_routes:
                    break
        pending.difference_update(processed)
        if violations or client_errors or unverified_transport_routes:
            break
        if files:
            continue
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
    if unverified_transport_routes:
        errors.append(f"{len(unverified_transport_routes)} routes use unverified transport interactions")
    if not checked:
        errors.append("no actual route witnesses")
    for start in sorted(anchors):
        if start in blocked or start not in bank_reachable:
            errors.append(f"Start is not standable with a bank route: {start}")
    for index, resource in enumerate(resources):
        if index not in resource_reached:
            errors.append(f"resource has no access route from its Start: {resource['origin']}")
    normal_matching = []
    try:
        normal_matching = normal_reachable_matching(normal_reachable)
    except ValueError as error:
        errors.append(str(error))
    isolated = processed - blocked - bank_reachable
    if isolated:
        errors.append(f"{len(isolated)} standable origins have no bank-access route")
    audit_aborted = bool(violations or client_errors or unverified_transport_routes)
    result = dict(
        request_id="T14-SOAK-ROUTE",
        verdict="FAIL" if violations or errors else "PASS",
        scope=(
            "Worksite-to-bank routes and ordinary replan closure over every intermediate tile, "
            "Gatherer retreats/re-approaches, native loc approaches, and client BFS walks for "
            "fault-phase recovery. Random-event movement is excluded, not proved safe."
        ),
        random_event_movement="excluded",
        random_event_movement_proved_safe=False,
        routes_checked=checked,
        unreachable_pairs=no_path,
        nonstandable_origins=len(blocked),
        hazard_retreat_routes=retreat_count,
        client_approach_routes=client_count,
        client_walk_routes=walk_count,
        client_walk_pairs=len(walk_pairs),
        replan_origins=len(processed),
        closure_rounds=round_number,
        client_walk_rounds=walk_rounds,
        ordinary_gate_routes=ordinary_gate_routes,
        unverified_transport_routes=len(unverified_transport_routes),
        ordinary_gate_loc_ids=sorted(ordinary_gate_locs),
        unverified_transport_examples=unverified_transport_routes[:30],
        closure_complete=not pending and not audit_aborted,
        audit_aborted=audit_aborted,
        rectangles=rectangles,
        outer_bounds=outer_bounds,
        normal_starts=starts,
        normal_radius=normal_radius,
        normal_patch_trees=patch_counts,
        normal_patch_reachable_trees=[len(trees) for trees in normal_reachable],
        normal_capacity_witness=normal_matching,
        second_tier_key=second_key,
        second_tier_start=second_start,
        second_tier_radius=second_radius,
        bank=bank_name,
        resources=resources,
        hunters_checked=len(hunters),
        sources=sources,
        replay_source=str(args.replay) if args.replay else None,
        checker_source=str(Path(__file__).resolve()),
        checker_sha256=digest(Path(__file__).resolve()),
        violation_count=len(violations),
        violations=violations[:30],
        errors=errors,
        residual_risk=(
            "Random-event movement is excluded and not proved safe. The audit also does not "
            "prove a worst-case 104-by-104 client scene-box detour."
        ),
    )
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    summary_fields = (
        "verdict", "scope", "random_event_movement", "random_event_movement_proved_safe",
        "routes_checked", "unreachable_pairs", "nonstandable_origins", "hazard_retreat_routes",
        "client_approach_routes", "client_walk_routes", "client_walk_pairs", "replan_origins",
        "closure_rounds", "client_walk_rounds", "closure_complete", "audit_aborted",
        "hunters_checked", "violation_count", "errors",
    )
    print(json.dumps({key: result[key] for key in summary_fields}))
    return 1 if violations or errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
