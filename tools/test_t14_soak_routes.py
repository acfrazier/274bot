#!/usr/bin/env python3
"""Two-way oracle tests. Real-pack route coverage runs through t14_soak_routes.py."""

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
