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

    def test_nav_captures_allows_tui_data_only(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["tui", "16", "active", "--nav-captures", "--headless"])
        rd.validate_args(a, p)
        self.assertTrue(a.nav_captures)
        self.assertEqual(a.frontend, "tui")
        env = rd.build_child_env(a, "/tmp/nav-tui-run", base_env={"PATH": "/usr/bin", "HOME": "/tmp"})
        self.assertEqual(env.get("BOT_NAV_CAPTURES"), "1")
        self.assertIn("captures", env.get("274BOT_SMOKE_DIR", ""))
        help_proc = run_cli("--help")
        self.assertIn("TUI", help_proc.stdout)
        self.assertIn("data-only", help_proc.stdout.lower())

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

    def test_gpu_completion_profile_requires_panel_and_render_profile(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(
            ["panel", "1", "idle", "--render-profile", "--gpu-completion-profile"]
        )
        rd.validate_args(a, p)
        self.assertTrue(a.gpu_completion_profile)
        self.assertTrue(a.render_profile)
        bad_tui = run_cli("tui", "1", "idle", "--render-profile", "--gpu-completion-profile")
        self.assertNotEqual(bad_tui.returncode, 0)
        bad_no_rp = run_cli("panel", "1", "idle", "--gpu-completion-profile")
        self.assertNotEqual(bad_no_rp.returncode, 0)
        help_proc = run_cli("--help")
        self.assertIn("--gpu-completion-profile", help_proc.stdout)

    def test_responsiveness_profile_flag_independent_and_scrubbed(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "1", "idle", "--responsiveness-profile"])
        rd.validate_args(a, p)
        self.assertTrue(a.responsiveness_profile)
        self.assertFalse(a.responsiveness_fine)
        a2 = p.parse_args(["tui", "1", "idle", "--responsiveness-profile"])
        rd.validate_args(a2, p)
        self.assertTrue(a2.responsiveness_profile)
        a3 = p.parse_args(["panel", "1", "idle"])
        self.assertFalse(a3.responsiveness_profile)
        help_proc = run_cli("--help")
        self.assertIn("--responsiveness-profile", help_proc.stdout)
        self.assertIn("--responsiveness-fine", help_proc.stdout)
        fine = p.parse_args(["panel", "1", "idle", "--responsiveness-profile", "--responsiveness-fine"])
        rd.validate_args(fine, p)
        self.assertTrue(fine.responsiveness_fine)
        bad = run_cli("panel", "1", "idle", "--responsiveness-fine")
        self.assertNotEqual(bad.returncode, 0)

    def test_failure_capture_flag_independent_of_diagnostics(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        only = p.parse_args(["panel", "1", "idle", "--no-diagnostics", "--failure-capture"])
        rd.validate_args(only, p)
        self.assertTrue(only.failure_capture)
        self.assertTrue(only.no_diagnostics)
        both = p.parse_args(["panel", "16", "active", "--failure-capture"])
        rd.validate_args(both, p)
        self.assertTrue(both.failure_capture)
        self.assertFalse(both.no_diagnostics)
        off = p.parse_args(["tui", "1", "idle"])
        self.assertFalse(off.failure_capture)
        help_proc = run_cli("--help")
        self.assertIn("--failure-capture", help_proc.stdout)
        self.assertIn("--no-diagnostics", help_proc.stdout)
        self.assertIn("failure-only", help_proc.stdout)

    def test_failure_capture_env_scrubbed_unless_flag(self):
        """Inherited BOT_MEMORY_FAILURE_CAPTURE must not leak when CLI flag is off."""
        import run_diagnostic as rd

        p = rd.build_parser()
        a_off = p.parse_args(["panel", "1", "idle", "--no-diagnostics"])
        rd.validate_args(a_off, p)
        a_on = p.parse_args(["panel", "1", "idle", "--no-diagnostics", "--failure-capture"])
        rd.validate_args(a_on, p)
        polluted = {
            "BOT_MEMORY_FAILURE_CAPTURE": "1",
            "BOT_RESPONSIVENESS_FINE": "1",
            "PATH": "/usr/bin",
            "HOME": "/tmp",
        }
        env_off = rd.build_child_env(a_off, "/tmp/fc-off-run", base_env=polluted)
        self.assertNotIn("BOT_MEMORY_FAILURE_CAPTURE", env_off)
        self.assertNotIn("BOT_RESPONSIVENESS_FINE", env_off)
        self.assertEqual(env_off["BOT_MEMORY_DIAGNOSTICS"], "0")
        env_on = rd.build_child_env(a_on, "/tmp/fc-on-run", base_env=polluted)
        self.assertEqual(env_on.get("BOT_MEMORY_FAILURE_CAPTURE"), "1")
        self.assertNotIn("BOT_RESPONSIVENESS_FINE", env_on)
        self.assertEqual(env_on["BOT_MEMORY_DIAGNOSTICS"], "0")
        # Flag alone (diagnostics default on) remains legal.
        a_both = p.parse_args(["panel", "1", "idle", "--failure-capture"])
        rd.validate_args(a_both, p)
        env_both = rd.build_child_env(a_both, "/tmp/fc-both-run", base_env=polluted)
        self.assertEqual(env_both.get("BOT_MEMORY_FAILURE_CAPTURE"), "1")
        self.assertEqual(env_both["BOT_MEMORY_DIAGNOSTICS"], "1")


if __name__ == "__main__":
    unittest.main()
