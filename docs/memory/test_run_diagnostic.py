#!/usr/bin/env python3
"""CLI validation for run_diagnostic (no live launch)."""
from __future__ import annotations

import pathlib
import os
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
SCRIPT = ROOT / "run_diagnostic.py"
sys.path.insert(0, str(ROOT))
import run_diagnostic as rd  # noqa: E402


def run_cli(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )


class RunDiagnosticCli(unittest.TestCase):
    def test_capture_frontend_bound_preserves_960_second_live_budget(self):
        self.assertEqual(rd._CAPTURE_FRONTEND_MAX_WALL_S, 960)

    def test_help_survives_windows_redirected_output_encoding(self):
        proc = subprocess.run(
            [sys.executable, str(SCRIPT), '--help'],
            cwd=ROOT, capture_output=True,
            env={**os.environ, 'PYTHONIOENCODING': 'cp1252'},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(b'--responsiveness-fine', proc.stdout)

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

    def test_import_survives_missing_unix_tty_modules(self):
        """Panel import/validate must work when pty/fcntl/termios are absent."""
        code = r"""
import importlib
import sys
import types

BLOCK = {"pty", "fcntl", "termios"}

class Blocker:
    def find_spec(self, fullname, path=None, target=None):
        if fullname in BLOCK:
            raise ModuleNotFoundError(fullname)
        return None

# Drop cached modules so the blocker is meaningful.
for name in list(sys.modules):
    if name in BLOCK or name == "run_diagnostic" or name.startswith("run_diagnostic."):
        del sys.modules[name]

sys.meta_path.insert(0, Blocker())
import run_diagnostic as rd

# Top-level import must not pull Unix TTY stack.
assert "pty" not in sys.modules
assert "fcntl" not in sys.modules
assert "termios" not in sys.modules

p = rd.build_parser()
a = p.parse_args(["panel", "1", "idle", "--focused-one"])
rd.validate_args(a, p)
env = rd.build_child_env(
    a, "/tmp/panel-no-pty", base_env={"PATH": "/usr/bin", "HOME": "/tmp"}, platform="win32"
)
assert "RS2B0T" not in env or env.get("RS2B0T") != rd._DEFAULT_RS2B0T_MAC
assert env.get("BOT_MEMORY_RENDER_POLICY") == "focused-one"

# Real TUI terminal on forced win32 fails closed when native ConPTY
# cannot bind (this host is not win32, or API missing) — never silent headless.
try:
    tag = rd.require_terminal_transport(platform="win32")
except RuntimeError as err:
    msg = str(err).lower()
    assert "windows" in msg or "conpty" in msg
    assert "headless" in msg or "panel" in msg
else:
    # On real win32 the ConPTY helper loads successfully.
    assert tag[0] == "conpty", tag

try:
    rd.require_terminal_transport(platform="linux")
except RuntimeError as err:
    assert "pty" in str(err).lower() or "termios" in str(err).lower() or "fcntl" in str(err).lower()
else:
    raise SystemExit("expected missing-module RuntimeError on linux flag")

print("ok")
"""
        proc = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            capture_output=True,
            text=True,
            env={**dict(**{k: v for k, v in __import__("os").environ.items()}), "PYTHONPATH": str(ROOT)},
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("ok", proc.stdout)

    def test_rs2b0t_default_mac_only_not_windows(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        a = p.parse_args(["panel", "1", "idle"])
        base = {"PATH": "/usr/bin", "HOME": "/tmp"}
        env_mac = rd.build_child_env(a, "/tmp/r1", base_env=base, platform="darwin")
        self.assertEqual(env_mac.get("RS2B0T"), rd._DEFAULT_RS2B0T_MAC)
        env_win = rd.build_child_env(a, "/tmp/r2", base_env=dict(base), platform="win32")
        self.assertNotIn("RS2B0T", env_win)
        env_win_explicit = rd.build_child_env(
            a,
            "/tmp/r3",
            base_env={**base, "RS2B0T": r"C:\src\rs2b0t"},
            platform="win32",
        )
        self.assertEqual(env_win_explicit.get("RS2B0T"), r"C:\src\rs2b0t")

    def test_resolve_rs2b0t_commit_fail_closed_windows(self):
        import run_diagnostic as rd

        with self.assertRaises(ValueError) as ctx:
            rd.resolve_rs2b0t_commit({}, platform="win32")
        self.assertIn("RS2B0T", str(ctx.exception))
        self.assertNotIn("deadbeef", str(ctx.exception).lower())

        calls = []

        def fake_git(args):
            calls.append(args)
            return "abc123real"

        # Non-dir path must not call git with a fabricated SHA path silently OK.
        with self.assertRaises(ValueError):
            rd.resolve_rs2b0t_commit(
                {"RS2B0T": "/no/such/rs2b0t-checkout-xyz"},
                platform="win32",
                git_check_output=fake_git,
            )
        self.assertEqual(calls, [])

    def test_resolve_rs2b0t_commit_real_git_when_path_ok(self):
        import run_diagnostic as rd
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            subprocess.run(["git", "init"], cwd=root, check=True, capture_output=True)
            subprocess.run(
                ["git", "config", "user.email", "t@example.com"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["git", "config", "user.name", "t"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            (root / "f").write_text("x\n")
            subprocess.run(["git", "add", "f"], cwd=root, check=True, capture_output=True)
            subprocess.run(
                ["git", "commit", "-m", "t"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            head = subprocess.check_output(
                ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
            ).strip()
            got = rd.resolve_rs2b0t_commit({"RS2B0T": str(root)}, platform="win32")
            self.assertEqual(got, head)

    def test_catalog_path_uses_operator_home_windows_profile(self):
        import run_diagnostic as rd

        path = rd.catalog_path_for_env(
            {"USERPROFILE": r"C:\Users\BotTest", "PATH": "x"},
            windows=True,
        )
        self.assertTrue(str(path).replace("\\", "/").endswith(".274bot/js-scripts.json"))
        # Empty explicit HOME must not silently use USERPROFILE (Rust parity).
        path_empty = rd.catalog_path_for_env(
            {"HOME": "", "USERPROFILE": r"C:\Users\BotTest"},
            windows=True,
        )
        # bot_home "." → resolve relative catalog
        self.assertTrue(str(path_empty).endswith(str(pathlib.Path(".274bot") / "js-scripts.json")) or
                        ".274bot" in str(path_empty))

    def test_unix_terminal_transport_loads_when_modules_present(self):
        import run_diagnostic as rd

        if sys.platform == "win32":
            self.skipTest("host is win32")
        tag, fcntl, pty, termios = rd.require_terminal_transport()
        self.assertEqual(tag, "unix")
        self.assertTrue(hasattr(pty, "openpty"))
        self.assertTrue(hasattr(fcntl, "ioctl"))
        self.assertTrue(hasattr(termios, "TIOCSWINSZ"))

    def test_win32_terminal_transport_selects_conpty_helper(self):
        import run_diagnostic as rd
        import windows_conpty as wcp

        if sys.platform == "win32":
            tag, mod = rd.require_terminal_transport(platform="win32")
            self.assertEqual(tag, "conpty")
            self.assertIs(mod, wcp)
        else:
            # Off Windows, forced platform still must not claim unix PTY.
            with self.assertRaises(RuntimeError) as ctx:
                rd.require_terminal_transport(platform="win32")
            msg = str(ctx.exception).lower()
            self.assertTrue("conpty" in msg or "windows" in msg)
            self.assertTrue("headless" in msg or "panel" in msg)

    def test_cpu_fallback_flag_panel_only_and_metadata_names(self):
        import run_diagnostic as rd

        p = rd.build_parser()
        help_proc = run_cli("--help")
        self.assertIn("--cpu-fallback", help_proc.stdout)

        default = p.parse_args(["panel", "1", "idle"])
        rd.validate_args(default, p)
        self.assertFalse(default.cpu_fallback)
        self.assertEqual(rd.requested_backend(default), "gpu")

        on = p.parse_args(["panel", "16", "idle", "--cpu-fallback", "--focused-one"])
        rd.validate_args(on, p)
        self.assertTrue(on.cpu_fallback)
        self.assertEqual(rd.requested_backend(on), "cpu_fallback")
        self.assertEqual(rd.requested_render_policy(on), "focused-one")

        tui = p.parse_args(["tui", "1", "idle"])
        rd.validate_args(tui, p)
        self.assertEqual(rd.requested_backend(tui), "none")

        bad_tui = run_cli("tui", "1", "idle", "--cpu-fallback")
        self.assertNotEqual(bad_tui.returncode, 0)
        self.assertIn("panel", (bad_tui.stderr + bad_tui.stdout).lower())

        bad_gpu = run_cli(
            "panel", "1", "idle",
            "--render-profile", "--gpu-completion-profile", "--cpu-fallback",
        )
        self.assertNotEqual(bad_gpu.returncode, 0)
        err = (bad_gpu.stderr + bad_gpu.stdout).lower()
        self.assertTrue("cpu-fallback" in err or "gpu-completion" in err, err)

        # Nav captures + focused-one still legal with CPU fallback.
        nav = p.parse_args(
            ["panel", "16", "idle", "--nav-captures", "--focused-one", "--cpu-fallback"]
        )
        rd.validate_args(nav, p)
        self.assertTrue(nav.cpu_fallback)
        self.assertTrue(nav.nav_captures)

    def test_cpu_fallback_env_scrub_default_and_explicit_set(self):
        """Inherited BOT_CPU is scrubbed; BOT_CPU=1 only when --cpu-fallback."""
        import run_diagnostic as rd

        p = rd.build_parser()
        a_off = p.parse_args(["panel", "1", "idle"])
        rd.validate_args(a_off, p)
        a_on = p.parse_args(["panel", "1", "idle", "--cpu-fallback"])
        rd.validate_args(a_on, p)
        polluted = {
            "BOT_CPU": "1",
            "BOT_DEBUG": "1",
            "PATH": "/usr/bin",
            "HOME": "/tmp",
        }
        env_off = rd.build_child_env(a_off, "/tmp/cpu-off-run", base_env=polluted)
        self.assertNotIn("BOT_CPU", env_off)
        self.assertNotIn("BOT_DEBUG", env_off)
        env_on = rd.build_child_env(a_on, "/tmp/cpu-on-run", base_env=polluted)
        self.assertEqual(env_on.get("BOT_CPU"), "1")
        self.assertNotIn("BOT_DEBUG", env_on)
        # Explicit flag still sets after scrub even when parent lacked BOT_CPU.
        clean = {"PATH": "/usr/bin", "HOME": "/tmp"}
        env_clean_on = rd.build_child_env(a_on, "/tmp/cpu-clean-on", base_env=clean)
        self.assertEqual(env_clean_on.get("BOT_CPU"), "1")
        env_clean_off = rd.build_child_env(a_off, "/tmp/cpu-clean-off", base_env=clean)
        self.assertNotIn("BOT_CPU", env_clean_off)

    def test_heaptrack_option_is_n1_tui_only_and_child_env_is_explicit(self):
        import run_diagnostic as rd
        p = rd.build_parser()
        valid = p.parse_args(["tui", "1", "active", "--no-diagnostics", "--sustain",
                              "--heaptrack-output", "/tmp/capture"])
        rd.validate_args(valid, p)
        valid.heaptrack_capture = {
            "output": {"raw": "/tmp/capture/alloc.raw"},
            "preload": {"path": "/usr/lib/heaptrack/libheaptrack_preload.so"},
        }
        env = rd.build_child_env(valid, "/tmp/run", base_env={"PATH": "/usr/bin"})
        self.assertEqual(env["LD_PRELOAD"], "/usr/lib/heaptrack/libheaptrack_preload.so")
        self.assertEqual(env["DUMP_HEAPTRACK_OUTPUT"], "/tmp/capture/alloc.raw")
        for args in (("tui", "16", "active"), ("panel", "1", "active"),
                     ("tui", "1", "idle")):
            bad = p.parse_args([*args, "--no-diagnostics", "--sustain", "--heaptrack-output", "/tmp/c"])
            with self.assertRaises(SystemExit):
                rd.validate_args(bad, p)

    def test_heaptrack_option_rejects_other_probes(self):
        import run_diagnostic as rd
        p = rd.build_parser()
        bad = p.parse_args(["tui", "1", "active", "--no-diagnostics", "--sustain",
                            "--heaptrack-output", "/tmp/c", "--stack-logging"])
        with self.assertRaises(SystemExit):
            rd.validate_args(bad, p)


if __name__ == "__main__":
    unittest.main()
