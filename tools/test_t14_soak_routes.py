#!/usr/bin/env python3
"""Two-way oracle tests. Real-pack route coverage runs through t14_soak_routes.py."""

import argparse
import gzip
import json
import tempfile
from pathlib import Path
import unittest

import t14_soak_routes as audit


class RouteSafetyTests(unittest.TestCase):
    def test_adjacent_ordinary_gate_passes_but_other_transports_fail_closed(self):
        edge = dict(kind="Door", at=[2766, 3444, 0], to=[2767, 3444, 0],
                    loc_id=1551, open_loc_id=1552, has_requirements=False)
        route = dict(transports=1, transport_edges=[edge],
                     tiles=[[2765, 3444, 0], edge["at"], edge["to"], [2768, 3444, 0]])
        self.assertTrue(audit.ordinary_gate_route(route))
        self.assertEqual(audit.route_violations(route, [(2624, 2879, 3392, 3583)], []), [])
        for replacement in [
            dict(to=[2766, 3444, 0]), dict(to=[2768, 3444, 0]), dict(to=[2767, 3444, 1]),
            dict(kind="Stairs"), dict(kind="AgilityShortcut"),
            dict(kind="Teleport"), dict(has_requirements=True),
        ]:
            with self.subTest(replacement=replacement):
                self.assertFalse(audit.ordinary_gate_route(
                    route | {"transport_edges": [edge | replacement]}))
        self.assertFalse(audit.ordinary_gate_route({"transports": 1}))
        self.assertFalse(audit.ordinary_gate_route(route | {"transports": 2}))
        for tile in [edge["at"], edge["to"]]:
            hunter = dict(name="test-hunter", spawn=tile, envelope=0)
            self.assertTrue(audit.route_violations(route, [(2624, 2879, 3392, 3583)], [hunter]))

    def test_safe_endpoints_do_not_hide_an_unsafe_route(self):
        areas = [(2788, 2834, 3436, 3468), (2788, 2811, 3436, 3476)]
        wolf = {"name": "pack_wolf", "spawn": [2836, 3495, 0], "envelope": 24}
        route = {"tiles": [[2805, 3469, 0], [2813, 3477, 0], [2806, 3470, 0]]}
        self.assertTrue(audit.contains(areas, route["tiles"][0]))
        self.assertTrue(audit.contains(areas, route["tiles"][-1]))
        failures = audit.route_violations(route, areas, [wolf])
        self.assertEqual({f["reason"] for f in failures}, {"outside-outer-bound", "hunter-envelope"})

    def test_actual_v17_route_to_unsafe_observed_tile_is_rejected(self):
        # This is a router witness, not attribution of the old movement.
        points = [
            (2805, 3469), (2805, 3469), (2805, 3470), (2806, 3471),
            (2807, 3471), (2808, 3472), (2808, 3473), (2809, 3474),
            (2809, 3475), (2809, 3476), (2810, 3476), (2811, 3477),
            (2812, 3478), (2813, 3477),
        ]
        wolf = {"name": "pack_wolf", "spawn": [2836, 3495, 0], "envelope": 24}
        route = {"tiles": [[x, z, 0] for x, z in points]}
        failures = audit.route_violations(
            route, [(2788, 2834, 3436, 3468), (2788, 2811, 3436, 3476)], [wolf])
        self.assertEqual({f["reason"] for f in failures}, {"outside-outer-bound", "hunter-envelope"})

    def test_historical_bank_resource_route_passes_and_envelope_edge_fails(self):
        areas = [(2680, 2753, 3440, 3506)]
        highwayman = {"name": "highwayman", "spawn": [2735, 3521, 0], "envelope": 14}
        # Historical route from the second-tier Start to a Seers bank stand.
        points = [
            (2726, 3500), (2726, 3500), (2725, 3501), (2724, 3501),
            (2723, 3500), (2722, 3500), (2721, 3500), (2720, 3499),
            (2719, 3498), (2718, 3497), (2718, 3496), (2718, 3495),
            (2718, 3494), (2718, 3493), (2719, 3492), (2720, 3491),
            (2720, 3490), (2720, 3489), (2721, 3488), (2722, 3487),
            (2723, 3486), (2724, 3486), (2725, 3486), (2726, 3487),
            (2726, 3488), (2726, 3489), (2726, 3490), (2726, 3491),
            (2725, 3492), (2724, 3493),
        ]
        route = {"tiles": [[x, z, 0] for x, z in points]}
        cache = {}
        self.assertEqual(audit.route_violations(route, areas, [highwayman], cache), [])
        self.assertEqual(audit.route_violations(route, areas, [highwayman], cache), [])
        self.assertTrue(audit.route_violations({"tiles": [[2735, 3507, 0]]}, areas, [highwayman]))

    def test_geometry_is_not_exempted_by_request_kind(self):
        row = {"kind": "guardian-flee", "tiles": [[2700, 3480, 0]]}
        hunter = {"name": "test-hunter", "spawn": [2700, 3480, 0], "envelope": 0}
        failures = audit.route_violations(row, [(2624, 2879, 3392, 3583)], [hunter])
        self.assertEqual({failure["reason"] for failure in failures}, {"hunter-envelope"})

    def test_client_approaches_cover_every_access_origin_and_keep_loc_geometry(self):
        resource = dict(origin=[2695, 3474, 0], width=2, height=2,
                        id=1276, shape=10, angle=2)
        approaches = audit.client_approaches([resource])
        self.assertEqual({row["source"] for row in approaches}, audit.goals_for(resource))
        self.assertEqual(len(approaches), 16)
        for row in approaches:
            self.assertEqual(row["target"], resource["origin"])
            self.assertEqual((row["id"], row["shape"], row["angle"]), (1276, 10, 2))

    def test_client_walks_cover_short_aims_both_directions_and_blocked_escape(self):
        start, edge, far, upstairs = (0, 0, 0), (20, 0, 0), (21, 0, 0), (0, 0, 1)
        checked = set()
        walks = audit.client_walks({start, edge, far, upstairs}, {edge}, checked)
        pairs = {(row["source"], row["target"]) for row in walks}
        self.assertIn((edge, start), pairs)
        self.assertNotIn((start, edge), pairs)
        self.assertNotIn((start, far), pairs)
        self.assertNotIn((start, upstairs), pairs)
        self.assertNotIn((start, start), pairs)
        self.assertEqual(pairs, checked)
        self.assertEqual(audit.client_walks({start, edge, far, upstairs}, {edge}, checked), [])

    def test_client_walk_closure_adds_both_directions_for_new_origins(self):
        left, right, middle = (0, 0, 0), (20, 0, 0), (10, 10, 0)
        checked = set()
        audit.client_walks({left, right}, set(), checked)
        walks = audit.client_walks({left, right, middle}, set(), checked)
        self.assertEqual({(row["source"], row["target"]) for row in walks}, {
            (left, middle), (right, middle), (middle, left), (middle, right),
        })

    def test_stalled_walk_recovery_can_reissue_an_aim_beyond_twenty_tiles(self):
        detour, aim = (0, 30, 0), (20, 0, 0)
        checked = set()
        self.assertEqual(audit.client_walks({detour, aim}, set(), checked), [])
        self.assertEqual(
            audit.client_walks({detour, aim}, set(), checked, {(detour, aim), (aim, aim)}),
            [dict(source=detour, target=aim)],
        )

    def test_normal_patch_capacity_rejects_single_tree_and_counts_shared_alternatives(self):
        trees = [dict(kind="normal", owners=[0, 1]), dict(kind="second-tier", owners=["second-tier"])]
        with self.assertRaisesRegex(ValueError, "at least two eligible trees, found 1"):
            audit.normal_patch_capacity(trees, 2)
        trees.append(dict(kind="normal", owners=[0, 1]))
        self.assertEqual(audit.normal_patch_capacity(trees, 2), [2, 2])

    def test_normal_reachable_capacity_requires_two_alternatives_per_start(self):
        with self.assertRaisesRegex(ValueError, "two reachable trees, found 1"):
            audit.normal_reachable_matching([{0}, {1, 2}])

    def test_normal_reachable_capacity_rejects_shared_pool_too_small_for_starts(self):
        with self.assertRaisesRegex(ValueError, "no full distinct-tree reachability matching"):
            audit.normal_reachable_matching([{0, 1}, {0, 1}, {0, 1}])

    def test_normal_reachable_capacity_rematches_instead_of_fixing_product_selection(self):
        reachable = [{0, 1}, {0, 2}, {2, 3}]
        witness = audit.normal_reachable_matching(reachable)
        self.assertEqual(len(set(witness)), len(reachable))
        for owner, tree in enumerate(witness):
            self.assertIn(tree, reachable[owner])

    def test_fixture_reads_second_tier_constants_and_key(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "fixture.rs"
            starts = [(100 + index, 200) for index in range(5)]
            source.write_text(
                "const NORMAL_STARTS: [(i32, i32); 5] = [\n"
                f"    {', '.join(map(str, starts))},\n];\n"
                "const SECOND_TIER_START: (i32, i32) = (120, 220);\n"
                "const SECOND_TIER_KEY: &str = \"willow\";\n"
                "const NORMAL_RADIUS: i32 = 5;\n"
                "const SECOND_TIER_RADIUS: i32 = 12;\n"
                "const NORMAL_SAFE_WALK: &[Rect] = &[Rect { min_x: 1, max_x: 2, "
                "min_z: 3, max_z: 4 }];\n"
            )
            self.assertEqual(audit.fixture(source),
                             (starts, (120, 220), [(1, 2, 3, 4)], 5, 12))
            self.assertEqual(audit.second_tier_key(source), "willow")

    def test_second_tier_census_uses_radius_and_selected_oak_or_willow_table(self):
        starts = [(10 + index, 10) for index in range(10)]
        second_start = (40, 40)
        with tempfile.TemporaryDirectory() as folder:
            content = Path(folder)
            configs = content / "scripts/skill_woodcutting/configs"
            trees = configs / "trees"
            trees.mkdir(parents=True)
            tables = ["[normal_tree_table]\ndata=tree,NormalTree"]
            for key in ("oak", "willow"):
                tables.append(f"[{key}_tree_table]\ndata=tree,{key.title()}Tree")
                (trees / f"{key}.loc").write_text(
                    f"[{key.title()}Tree]\nwidth=1\nlength=1\n")
            (configs / "trees.dbrow").write_text("\n".join(tables) + "\n")
            (trees / "normal.loc").write_text("[NormalTree]\nwidth=1\nlength=1\n")
            (content / "maps").mkdir()
            (content / "maps/m0_0.jm2").write_text(
                "==== LOC ====\n"
                "0 40 40: 2 10\n"
                "0 52 40: 2 10\n"
                "0 53 40: 2 10\n"
                "0 40 40: 3 10\n"
                "0 52 40: 3 10\n"
                "0 53 40: 3 10\n"
                "0 10 10: 1 10\n"
                "==== NPC ====\n"
            )
            game = {
                "loc_names": {"rows": [
                    {"id": 1, "config": "NormalTree"},
                    {"id": 2, "config": "OakTree"},
                    {"id": 3, "config": "WillowTree"},
                ]},
                "gather_sites": [
                    {"id": "normal-site", "region": dict(min_x=0, min_z=0, max_x=100, max_z=100),
                     "keys": [{"key": "normal"}]},
                    {"id": "second-site", "region": dict(min_x=40, min_z=40, max_x=40, max_z=40),
                     "keys": [{"key": "oak"}, {"key": "willow"}]},
                ],
                "bank_placements": {"rows": []},
                "npc_names": {"rows": []},
                "npc_placements": {"rows": []},
            }
            for key in ("oak", "willow"):
                with self.subTest(key=key):
                    resources, _, _, _ = audit.census(
                        content, game, starts, second_start, 5, 12,
                        key, "normal-site", "second-site", "Test bank")
                    selected = [row for row in resources if row["kind"] == "second-tier"]
                    self.assertEqual({tuple(row["origin"]) for row in selected},
                                     {(40, 40, 0), (52, 40, 0)})
                    self.assertTrue(all(row["owners"] == ["second-tier"] for row in selected))
                    self.assertEqual({row["id"] for row in selected},
                                     {2 if key == "oak" else 3})
                    with self.assertRaisesRegex(ValueError, "fewer than two authored second-tier"):
                        audit.census(
                            content, game, starts, second_start, 5, 0,
                            key, "normal-site", "second-site", "Test bank")

    def test_route_audit_reads_normal_and_second_tier_radii(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "fixture.rs"
            starts = [(100 + index, 200) for index in range(5)]
            source.write_text(
                f"const NORMAL_STARTS: [(i32, i32); 5] = {starts!r};\n"
                "const SECOND_TIER_START: (i32, i32) = (120, 220);\n"
                "const SECOND_TIER_KEY: &str = \"oak\";\n"
                "const NORMAL_RADIUS: i32 = 5;\n"
                "const SECOND_TIER_RADIUS: i32 = 12;\n"
                "const NORMAL_SAFE_WALK: &[Rect] = &[Rect { min_x: 1, max_x: 2, "
                "min_z: 3, max_z: 4 }];\n"
            )
            self.assertEqual(audit.fixture(source),
                             (starts, (120, 220), [(1, 2, 3, 4)], 5, 12))
            self.assertEqual(audit.second_tier_key(source), "oak")

    def test_unroutable_client_alternatives_encode_no_movement(self):
        for kind in ("client-approach", "client-walk"):
            self.assertFalse(audit.client_route_error(dict(kind=kind, error="client-no-route")))
            self.assertTrue(audit.client_route_error(dict(kind=kind, error="loc-id-not-in-cache")))
            self.assertTrue(audit.client_route_error(dict(kind=kind, error="client-route-path-empty")))
        self.assertFalse(audit.client_route_error(dict(kind="walk", error="NoPath")))

    def test_each_resource_and_bank_loc_requires_an_actual_client_witness(self):
        tree = dict(id=1276, origin=[2695, 3474, 0], shape=10, angle=2)
        bank = dict(id=2213, origin=[2721, 3494, 0], shape=10, angle=2)
        tree_route = dict(id=1276, target=[2695, 3474, 0], shape=10, angle=2)
        witnessed = {audit.loc_key(tree_route)}
        self.assertEqual(audit.missing_client_approaches([tree, bank], witnessed), [
            "loc has no actual client approach witness: 2213 at [2721, 3494, 0]",
        ])
        witnessed.add(audit.loc_key(bank))
        self.assertEqual(audit.missing_client_approaches([tree, bank], witnessed), [])
        self.assertTrue(audit.missing_client_approaches([dict(tree, angle=1)], witnessed))

    def test_replay_requires_identical_requests_and_retained_output(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            args = argparse.Namespace(replay=root, output=root / "fresh")
            inputs = root / "example-batches.json"
            routes = root / "example-routes.jsonl.gz"
            inputs.write_text(json.dumps([dict(
                name="example", sources=[[1, 2, 0]], targets=[[2, 2, 0]],
                retreats=[], approaches=[], walks=[], guardians=[], hazards=[],
            )]))
            routes.write_bytes(gzip.compress(b'{"kind":"walk","tiles":[[1,2,0],[2,2,0]]}\n'))
            self.assertEqual(
                audit.run_probe(args, "example", {(1, 2, 0)}, {(2, 2, 0)}), routes)
            self.assertFalse(args.output.exists())
            with self.assertRaisesRegex(ValueError, "replay requests differ"):
                audit.run_probe(args, "example", {(1, 3, 0)}, {(2, 2, 0)})
            routes.unlink()
            with self.assertRaisesRegex(FileNotFoundError, "replay route output missing"):
                audit.run_probe(args, "example", {(1, 2, 0)}, {(2, 2, 0)})

    def test_replay_rechecks_failed_verdicts_but_rejects_changed_inputs(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            sources = {"actual-selected-config": "original-input"}
            (root / "result.json").write_text(json.dumps(dict(verdict="FAIL", sources=sources)))
            audit.verify_replay_sources(root, sources)
            with self.assertRaisesRegex(ValueError, "replay inputs differ"):
                audit.verify_replay_sources(root, {"actual-selected-config": "changed-input"})

    def test_authored_map_rotation_is_optional(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "m42_54.jm2"
            path.write_text("==== LOC ====\n0 33 45: 1307 10\n0 41 25: 1307 10 2\n==== NPC ====\n0 1 2: 3\n")
            self.assertEqual(list(audit.map_rows(path, "LOC")), [
                (2, (0, 33, 45), (1307, 10)),
                (3, (0, 41, 25), (1307, 10, 2)),
            ])


if __name__ == "__main__":
    unittest.main()
