#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import run_current_tui_calibration as runner


class CurrentTuiCalibrationControls(unittest.TestCase):
    def test_diagnostic_argv_is_exact_resource_only_contract(self):
        argv = runner.diagnostic_argv(pathlib.Path("/bin/tui-play"), pathlib.Path("/tmp/build.json"), "candidate")
        self.assertEqual(argv[:3], ["tui", "16", "active"])
        self.assertIn("--no-diagnostics", argv)
        self.assertIn("--sustain", argv)
        self.assertEqual(argv[argv.index("--warmup") + 1], "120")
        self.assertEqual(argv[argv.index("--observe") + 1], "600")
        for forbidden in ("--debug", "--tui-input-probes", "--stack-logging", "--scheduling-profile", "--render-profile", "--responsiveness-profile"):
            self.assertNotIn(forbidden, argv)

    def test_clean_environment_forces_all_hot_flags_off(self):
        env = {key: "1" for key in ("BOT_DEBUG", "BOT_CPU", "BOT_SCHEDULING_PROFILE", "BOT_RENDER_PROFILE", "BOT_RESPONSIVENESS_PROFILE", "MallocStackLogging")}
        clean = runner.clean_environment(env)
        self.assertEqual(clean["BOT_MEMORY_N"], "16")
        self.assertEqual(clean["BOT_MEMORY_WORKLOAD"], "active")
        self.assertEqual(clean["BOT_MEMORY_DIAGNOSTICS"], "0")
        for key in ("BOT_DEBUG", "BOT_CPU", "MallocStackLogging"):
            self.assertNotIn(key, clean)
        for key in ("BOT_SCHEDULING_PROFILE", "BOT_RENDER_PROFILE", "BOT_RESPONSIVENESS_PROFILE", "BOT_RESPONSIVENESS_FINE", "BOT_GPU_COMPLETION_PROFILE"):
            self.assertEqual(clean[key], "0")

    def test_server_identity_requires_fresh_declared_server_and_real_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "server.json"
            valid = {"pid": 42, "start_identity": "start:42", "port_listen": 43594,
                     "config_sha256": "a" * 64, "fixture_manifest_sha256": "b" * 64,
                     "public_key_sha256": "c" * 64, "configuration": {
                         "bind_host": "127.0.0.1", "world_json_sha256": "a" * 64,
                         "fixture_manifest_sha256": "b" * 64,
                     }}
            path.write_text(json.dumps(valid))
            self.assertEqual(runner.validate_server_identity(path, 42, "start:42")["pid"], 42)
            valid["configuration"]["fixture_manifest_sha256"] = "bad"
            path.write_text(json.dumps(valid))
            with self.assertRaises(runner.CalibrationError):
                runner.validate_server_identity(path, 42, "start:42")

    def test_memory_guard_triggers_cleanup_callback(self):
        seen = []
        values = iter([None, runner.MEM_AVAILABLE_GUARD_BYTES - 1])
        guard = runner.MemoryGuard(seen.append, reader=lambda: next(values))
        guard.start()
        guard.thread.join(timeout=2)
        guard.close()
        self.assertEqual(len(seen), 1)
        self.assertIn("MemAvailable", seen[0])

    def test_preflight_only_has_no_launch_or_output_reservation(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = mock.Mock(output=pathlib.Path(tmp) / "future.json")
            fake_spec = {"id": "future", "launcher_argv": ["never"]}
            with mock.patch.object(runner, "validate_inputs", return_value={"host_commit": runner.EXPECTED_HOST}), \
                 mock.patch.object(runner, "build_spec", return_value=fake_spec), \
                 mock.patch.object(runner.rmc, "run_managed_cell") as managed:
                args.preflight_only = True
                with mock.patch.object(runner, "parser", return_value=mock.Mock(parse_args=lambda _: args)):
                    self.assertEqual(runner.main([]), 0)
                managed.assert_not_called()
            self.assertFalse(args.output.exists())
            self.assertFalse(args.output.with_suffix(args.output.suffix + ".spec.json").exists())

    def test_incomplete_managed_state_is_failed_and_never_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / "result.json"
            args = mock.Mock(output=output)
            spec = {"id": "result"}
            with mock.patch.object(runner.rmc, "run_managed_cell", return_value={"status": "failed_or_unavailable", "launched": True, "attempts": 1}):
                self.assertEqual(runner.run(args, spec), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "failed_or_unavailable")
            self.assertFalse(report["performance_acceptance"])
            self.assertEqual(report["memory_guard"]["status"], "not_triggered")

    def test_source_mismatch_fails_before_build_or_launch(self):
        with tempfile.TemporaryDirectory() as tmp:
            host = pathlib.Path(tmp) / "host"
            client = host / "vendor" / "fr-client-rust"
            client.mkdir(parents=True)
            (host / ".gitmodules").write_text("submodule")
            with mock.patch.object(runner, "_git", side_effect=["wrong-host", runner.EXPECTED_CLIENT]):
                with self.assertRaisesRegex(runner.CalibrationError, "host commit"):
                    runner.check_source(host, runner.EXPECTED_HOST, runner.EXPECTED_CLIENT)


if __name__ == "__main__":
    unittest.main()
