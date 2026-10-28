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

    def test_chosen_bank_resource_routes_pass_and_envelope_edge_fails(self):
        areas = [(2680, 2740, 3448, 3506)]
        highwayman = {"name": "highwayman", "spawn": [2735, 3521, 0], "envelope": 14}
        route = {"tiles": [[2691, 3475, 0], [2700, 3480, 0], [2724, 3493, 0], [2727, 3501, 0]]}
        cache = {}
        self.assertEqual(audit.route_violations(route, areas, [highwayman], cache), [])
        self.assertEqual(audit.route_violations(route, areas, [highwayman], cache), [])
        self.assertTrue(audit.route_violations({"tiles": [[2735, 3507, 0]]}, areas, [highwayman]))

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
