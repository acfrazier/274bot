#!/usr/bin/env python3
"""CLI validation for run_diagnostic (no live launch)."""
from __future__ import annotations

import pathlib
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
SCRIPT = ROOT / "run_diagnostic.py"


def run_cli(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )


class RunDiagnosticCli(unittest.TestCase):
    def test_accepts_n16_in_help_choices(self):
        proc = run_cli("--help")
        self.assertEqual(proc.returncode, 0)
        self.assertIn("{1,16,32,128}", proc.stdout)

    def test_rejects_invalid_n_without_launch(self):
        proc = run_cli("panel", "2", "idle")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("invalid choice", (proc.stderr + proc.stdout).lower())

    def test_rejects_panel_flags_on_tui(self):
        for flag in ("--single-renderer", "--focused-one", "--focused-background"):
            proc = run_cli("tui", "16", "idle", flag)
            self.assertNotEqual(proc.returncode, 0, flag)
            err = proc.stderr + proc.stdout
            self.assertIn("panel", err.lower(), err)

    def test_rejects_conflicting_render_flags(self):
        proc = run_cli("panel", "16", "idle", "--single-renderer", "--focused-one")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("mutually exclusive", (proc.stderr + proc.stdout).lower())

    def test_nav_captures_requires_one_draw_panel_mode(self):
        proc = run_cli("panel", "16", "idle", "--nav-captures")
        self.assertNotEqual(proc.returncode, 0)
        proc = run_cli("panel", "16", "idle", "--nav-captures", "--focused-background")
        self.assertNotEqual(proc.returncode, 0)
        # Valid combos still need a binary and would launch — only check parser path via import.
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "16", "idle", "--nav-captures", "--single-renderer"])
        rd.validate_args(a, p)
        a = p.parse_args(["panel", "16", "idle", "--nav-captures", "--focused-one"])
        rd.validate_args(a, p)

    def test_requested_render_policy_metadata_names(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        cases = [
            (["panel", "1", "idle"], "rotating-all", False),
            (["panel", "16", "idle", "--single-renderer"], "fixed-one", True),
            (["panel", "16", "idle", "--focused-one"], "focused-one", False),
            (["panel", "16", "idle", "--focused-background"], "focused-plus-background", False),
            (["tui", "16", "idle"], "none", False),
        ]
        for args, policy, single in cases:
            a = p.parse_args(args)
            rd.validate_args(a, p)
            self.assertEqual(rd.requested_render_policy(a), policy, args)
            self.assertEqual(a.single_renderer, single, args)

    def test_render_profile_flag_independent_and_scrubbed(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "1", "idle", "--render-profile"])
        rd.validate_args(a, p)
        self.assertTrue(a.render_profile)
        a2 = p.parse_args(["panel", "1", "idle"])
        self.assertFalse(a2.render_profile)
        help_proc = run_cli("--help")
        self.assertIn("--render-profile", help_proc.stdout)


if __name__ == "__main__":
    unittest.main()
