#!/usr/bin/env python3
"""Two-way oracle tests. Real-pack route coverage runs through t14_soak_routes.py."""

import argparse
import json
import tempfile
from pathlib import Path
import unittest

import t14_soak_routes as audit


class RouteSafetyTests(unittest.TestCase):
    def test_safe_endpoints_do_not_hide_an_unsafe_route(self):
        areas = [(2788, 2834, 3436, 3468), (2788, 2811, 3436, 3476)]
        wolf = {"name": "pack_wolf", "spawn": [2836, 3495, 0], "envelope": 24}
        route = {"tiles": [[2805, 3469, 0], [2813, 3477, 0], [2806, 3470, 0]]}
        self.assertTrue(audit.contains(areas, route["tiles"][0]))
        self.assertTrue(audit.contains(areas, route["tiles"][-1]))
        failures = audit.route_violations(route, areas, [wolf])
        self.assertEqual({f["reason"] for f in failures}, {"outside-approved-area", "hunter-envelope"})

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
        self.assertEqual({f["reason"] for f in failures}, {"outside-approved-area", "hunter-envelope"})

    def test_chosen_bank_resource_routes_pass_and_envelope_edge_fails(self):
        areas = [(2680, 2753, 3440, 3506)]
        highwayman = {"name": "highwayman", "spawn": [2735, 3521, 0], "envelope": 14}
        # Complete v17 route from the maple Start to a Seers bank stand.
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
            routes = root / "example-routes.jsonl"
            inputs.write_text(json.dumps([dict(
                name="example", sources=[[1, 2, 0]], targets=[[2, 2, 0]],
                retreats=[], approaches=[], walks=[],
            )]))
            routes.write_text('{"kind":"walk","tiles":[[1,2,0],[2,2,0]]}\n')
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
