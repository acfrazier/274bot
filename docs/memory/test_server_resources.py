#!/usr/bin/env python3
"""Bounded tests for the standalone game-server resource sampler."""
from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import server_resources as sr  # noqa: E402


class LinuxParserTests(unittest.TestCase):
    def test_proc_stat_handles_parenthesized_command(self):
        text = "123 (server worker) S " + " ".join(str(i) for i in range(4, 53))
        self.assertEqual(sr.parse_proc_stat(text)["state"], "S")
        self.assertEqual(sr.parse_proc_stat(text)["start_ticks"], 22)
        self.assertEqual(sr.parse_proc_stat(text)["utime_ticks"], 14)
        self.assertEqual(sr.parse_proc_stat(text)["stime_ticks"], 15)
        self.assertEqual(sr.parse_proc_stat(text)["rss_pages"], 24)

    def test_pressure_parser_preserves_psi_semantics(self):
        parsed = sr.parse_psi("some avg10=1.25 avg60=2.50 avg300=3.75 total=99\n")
        self.assertEqual(parsed["avg10_percent"], 1.25)
        self.assertEqual(parsed["total_us"], 99)
        self.assertEqual(parsed["semantics"], "fraction_of_wall_time_stalled")

    def test_pressure_parser_rejects_missing_fields(self):
        with self.assertRaises(sr.SampleError):
            sr.parse_psi("some avg10=1.0\n")


class ValidationTests(unittest.TestCase):
    def test_invalid_pid_rejected(self):
        with self.assertRaises(sr.SampleError):
            sr.validate_pid(0)
        with self.assertRaises(sr.SampleError):
            sr.validate_pid(-1)

    def test_identity_change_rejected(self):
        with self.assertRaises(sr.SampleError):
            sr.require_same_identity({"start_identity": "a"}, {"start_identity": "b"})

    def test_counter_reset_rejected(self):
        with self.assertRaises(sr.SampleError):
            sr.cpu_delta_seconds({"user_s": 2, "system_s": 1}, {"user_s": 1, "system_s": 2})

    def test_missing_process_sample_rejected(self):
        with self.assertRaises(sr.SampleError):
            sr.sample_process(999_999_991)

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "samples.jsonl"
            out.write_text("sentinel\n")
            with self.assertRaises(sr.SampleError):
                sr.open_output(out, force=False)
            self.assertEqual(out.read_text(), "sentinel\n")

    def test_symlink_output_is_rejected_even_with_force(self):
        with tempfile.TemporaryDirectory() as td:
            target = pathlib.Path(td) / "target"
            target.write_text("sentinel\n")
            out = pathlib.Path(td) / "samples.jsonl"
            out.symlink_to(target)
            with self.assertRaises(sr.SampleError):
                sr.open_output(out, force=True)
            self.assertEqual(target.read_text(), "sentinel\n")

    def test_macos_ps_timeout_is_forwarded_and_reported(self):
        with mock.patch.object(sr.subprocess, "run", side_effect=subprocess.TimeoutExpired("ps", 0.2)) as run:
            with self.assertRaises(sr.SampleError):
                sr._mac_sample(123, timeout=0.2)
        self.assertEqual(run.call_args.kwargs["timeout"], 0.2)

    def test_stalled_acquisition_fails_cadence_without_sleeping(self):
        class Clock:
            now = 0.0

            def monotonic(self):
                return self.now

            def sleep(self, seconds):
                self.now += seconds

        clock = Clock()

        def stalled_sample(_pid, timeout=None):
            clock.now += 0.4
            return {"start_identity": "test", "resident_bytes": 1, "user_s": 0.0, "system_s": 0.0}

        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "samples.jsonl"
            status = sr.run(123, out, 0.1, 0.25, sample_fn=stalled_sample,
                            pressure_fn=lambda: {"status": "unavailable"},
                            monotonic_fn=clock.monotonic, sleep_fn=clock.sleep)
            self.assertEqual(status, 1)
            rows = [json.loads(line) for line in out.read_text().splitlines()]
            self.assertIn("acquisition_start_monotonic_s", rows[1])
            self.assertIn("acquisition_end_monotonic_s", rows[1])
            self.assertIn("lateness_s", rows[1])
            self.assertEqual(rows[-1]["type"], "error")
            self.assertIn("cadence", rows[-1]["reason"])


class CliSmokeTests(unittest.TestCase):
    def test_required_mid_run_failure_prints_fail(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "samples.jsonl"
            cmd = [sys.executable, str(ROOT / "server_resources.py"), "999999992", str(out), "--duration", "0.1"]
            completed = subprocess.run(cmd, text=True, capture_output=True)
            self.assertEqual(completed.returncode, 1)
            self.assertIn("FAIL:", completed.stderr)
            rows = [json.loads(line) for line in out.read_text().splitlines()]
            self.assertEqual(rows[-1]["type"], "error")

    def test_short_controlled_self_sample(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "samples.jsonl"
            cmd = [sys.executable, str(ROOT / "server_resources.py"), str(os.getpid()), str(out), "--duration", "0.15", "--interval", "0.05"]
            completed = subprocess.run(cmd, text=True, capture_output=True)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            rows = [json.loads(line) for line in out.read_text().splitlines()]
            self.assertEqual(rows[0]["type"], "metadata")
            self.assertGreaterEqual(sum(row.get("type") == "sample" for row in rows), 2)
            self.assertEqual(rows[-1]["type"], "summary")
            self.assertEqual(rows[-1]["status"], "ok")


if __name__ == "__main__":
    unittest.main()
