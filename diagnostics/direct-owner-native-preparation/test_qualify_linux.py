#!/usr/bin/env python3
"""Unit tests for the frozen Linux qualification contract."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


MODULE_PATH = Path(__file__).resolve().parent / "qualify_linux.py"
SPEC = importlib.util.spec_from_file_location("direct_owner_qualify_linux", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
QUALIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QUALIFY)


class QualificationContractTests(unittest.TestCase):
    def test_tool_identity_binds_invoked_and_resolved_executable(self):
        identity = QUALIFY.executable_identity(sys.executable)
        self.assertTrue(Path(identity["invoked_path"]).is_absolute())
        self.assertTrue(Path(identity["resolved_path"]).is_file())
        self.assertGreater(identity["bytes"], 0)
        self.assertEqual(len(identity["sha256"]), 64)

    def test_binary_identity_requires_all_native_inspection_commands(self):
        binary = {name: {"exit": 0} for name in ("file", "readelf", "ldd")}
        self.assertTrue(QUALIFY.binary_identity_commands_passed(binary))
        binary["readelf"]["exit"] = 1
        self.assertFalse(QUALIFY.binary_identity_commands_passed(binary))
        self.assertFalse(QUALIFY.binary_identity_commands_passed(None))

    def test_source_check_reports_only_steps_it_executes(self):
        coverage = QUALIFY.coverage_contract("source-check")
        self.assertEqual(
            set(coverage),
            {"frozen_compile", "generated_observer", "protocol_validator"},
        )
        self.assertEqual(coverage["frozen_compile"]["steps"], ["cargo-check-tui"])
        self.assertNotIn("native", coverage["frozen_compile"]["claim"].lower())

    def test_linux_contract_maps_every_required_seam_to_executed_steps(self):
        coverage = QUALIFY.coverage_contract("linux")
        self.assertEqual(
            set(coverage),
            {
                "feature_graph_and_binary",
                "generated_observer",
                "owner_budget_mailbox_output_guards",
                "stop_and_cleanup",
                "protocol_validator",
            },
        )
        mapped = {step for item in coverage.values() for step in item["steps"]}
        self.assertIn("host-play-generated-observer", mapped)
        self.assertIn("api-owner-lib", mapped)
        self.assertIn("host-owner-lib", mapped)
        self.assertIn("host-play-owner-lib", mapped)
        self.assertIn("script-stop-integration", mapped)
        self.assertIn("managed-guard-cleanup-generated", mapped)
        self.assertIn("protocol-validator-generated", mapped)
        self.assertTrue(all(item["claim"] for item in coverage.values()))


if __name__ == "__main__":
    unittest.main()
