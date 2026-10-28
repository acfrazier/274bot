#!/usr/bin/env python3
"""Check diagnostic configuration before any engine or process can start."""

import copy
import contextlib
import io
import json
from pathlib import Path
import sys
import unittest
from unittest import mock

import t14_soak_run as runner


class PreflightReached(Exception):
    pass


class RunModeTests(unittest.TestCase):
    def arguments(self, *extra):
        return ["t14_soak_run.py", "--mode", "candidate", "--seed", "1027",
                "--output", str(runner.ROOT / "unused-test-output"), *extra]

    def capture_config(self, *extra):
        captured = []
        deepcopy = copy.deepcopy

        def capture(value):
            captured.append(deepcopy(value))
            return captured[-1]

        with mock.patch.object(sys, "argv", self.arguments(*extra)), \
                mock.patch.object(runner.copy, "deepcopy", side_effect=capture), \
                mock.patch.dict(runner.os.environ, {"BOT_NAV_SNAPSHOT_ROOT": str(runner.ROOT)}), \
                mock.patch.object(Path, "is_dir", side_effect=PreflightReached):
            with self.assertRaises(PreflightReached):
                runner.main()
        return captured[0]

    def test_diagnostic_retains_workload_faults_limits_and_full_cohort(self):
        original = json.loads(runner.SOURCE_CONFIG.read_text())
        config = self.capture_config("--diagnostic-seconds", "1800", "--name-prefix", "srt",
                                     "--engine-dir", str(runner.ROOT), "--upstream-port", "3080")
        self.assertEqual(config["run"]["duration_seconds"], 1800)
        self.assertEqual(config["run"]["slots"], 20)
        self.assertEqual(config["run"]["warmup_min_seconds"], original["run"]["warmup_min_seconds"])
        self.assertEqual(config["run"]["name_prefix"], "srt")
        self.assertEqual(config["proxy"]["upstream_port"], 3080)
        self.assertEqual(config["workload"], original["workload"])
        self.assertEqual(config["limits"], original["limits"])
        self.assertEqual(config["fault"] | {"seed": None}, original["fault"])

    def test_qualification_keeps_full_duration(self):
        config = self.capture_config()
        self.assertEqual(config["run"]["duration_seconds"], 7200)
        self.assertEqual(config["run"]["slots"], 20)
        self.assertEqual(config["proxy"]["upstream_port"], 1080)
        self.assertNotIn("diagnostic_override", config["_sources"])

    def test_qualification_rejects_engine_override(self):
        with mock.patch.object(sys, "argv", self.arguments("--upstream-port", "3080")):
            with self.assertRaisesRegex(ValueError, "diagnostic/smoke-only"):
                runner.main()

    def test_diagnostic_requires_positive_duration(self):
        with mock.patch.object(sys, "argv", self.arguments("--diagnostic-seconds", "0")):
            with self.assertRaisesRegex(ValueError, "must be positive"):
                runner.main()

    def test_diagnostic_cannot_disable_faults(self):
        with mock.patch.object(sys, "argv", self.arguments("--diagnostic-seconds", "1800",
                                                         "--diagnostic-no-ws-faults")):
            with self.assertRaisesRegex(ValueError, "smoke-only"):
                runner.main()

    def test_smoke_and_diagnostic_are_mutually_exclusive(self):
        with mock.patch.object(sys, "argv", self.arguments("--smoke", "--diagnostic-seconds", "1800")), \
                contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as error:
                runner.main()
        self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
