#!/usr/bin/env python3
"""Tests for continuous explicit-PID multi-role process accounting."""
from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import unittest
from typing import Any, Dict, List, Optional
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import process_accounting as pa  # noqa: E402
import server_resources as sr  # noqa: E402


class Clock:
    """Injectable monotonic clock with cooperative sleep."""

    def __init__(self, start: float = 0.0) -> None:
        self.now = start

    def monotonic(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += max(0.0, float(seconds))


def _sample(identity: str, rss: int = 1000, user: float = 1.0, system: float = 0.5) -> Dict[str, Any]:
    return {
        "start_identity": identity,
        "resident_bytes": rss,
        "user_s": user,
        "system_s": system,
        "provenance": {"os": "test"},
    }


class ConfigTests(unittest.TestCase):
    def test_parse_roles_ok(self):
        roles = pa.parse_role_specs(["game_server=123", "launcher=456"])
        self.assertEqual(roles, {"game_server": 123, "launcher": 456})

    def test_duplicate_pid_rejected(self):
        with self.assertRaises(pa.AccountingError) as ctx:
            pa.parse_role_specs(["a=10", "b=10"])
        self.assertIn("duplicate PID", str(ctx.exception))

    def test_duplicate_name_rejected(self):
        with self.assertRaises(pa.AccountingError):
            pa.parse_role_specs(["a=1", "a=2"])

    def test_malformed_spec_rejected(self):
        with self.assertRaises(pa.AccountingError):
            pa.parse_role_specs(["nope"])
        with self.assertRaises(pa.AccountingError):
            pa.parse_role_specs(["=1"])
        with self.assertRaises(pa.AccountingError):
            pa.parse_role_specs(["x=notint"])

    def test_normalize_injects_collector_without_double_count(self):
        roles = pa.normalize_roles({"game": 99}, include_collector_self=True, getpid_fn=lambda: 7)
        self.assertEqual(roles["game"], 99)
        self.assertEqual(roles["collector"], 7)

    def test_normalize_skips_collector_when_pid_already_listed(self):
        roles = pa.normalize_roles({"sampler": 7}, include_collector_self=True, getpid_fn=lambda: 7)
        self.assertEqual(roles, {"sampler": 7})
        self.assertNotIn("collector", roles)

    def test_normalize_rejects_wrong_collector_pid(self):
        with self.assertRaises(pa.AccountingError):
            pa.normalize_roles(
                {"collector": 1}, include_collector_self=True, getpid_fn=lambda: 2
            )


class InjectedCollectorTests(unittest.TestCase):
    def _run(
        self,
        roles: Dict[str, int],
        *,
        interval: float = 0.1,
        duration: Optional[float] = 0.35,
        sample_fn=None,
        stop_event=None,
        force: bool = False,
        include_collector_self: bool = False,
        clock: Optional[Clock] = None,
        install_signal_handlers: bool = False,
    ):
        clock = clock or Clock()
        pressure = lambda: {"status": "unavailable", "reason": "test"}
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            status = pa.run(
                roles,
                out,
                interval,
                duration,
                force=force,
                include_collector_self=include_collector_self,
                sample_fn=sample_fn,
                pressure_fn=pressure,
                monotonic_fn=clock.monotonic,
                sleep_fn=clock.sleep,
                utc_fn=lambda: "1970-01-01T00:00:00Z",
                getpid_fn=lambda: 4242,
                stop_event=stop_event,
                install_signal_handlers=install_signal_handlers,
            )
            text = out.read_text() if out.exists() else ""
            rows = [json.loads(line) for line in text.splitlines() if line.strip()]
            return status, rows, out

    def test_happy_path_multi_role(self):
        state = {"n": 0}

        def sample(pid, timeout=None):
            n = state["n"]
            state["n"] = n + 1
            # Alternate roles share call order by sorted name: a then b
            base_u = 1.0 + 0.1 * n
            return _sample(f"id-{pid}", rss=1000 + pid, user=base_u, system=0.5)

        status, rows, _ = self._run({"a": 10, "b": 20}, sample_fn=sample, duration=0.35, interval=0.1)
        self.assertEqual(status, 0, rows)
        self.assertEqual(rows[0]["type"], "metadata")
        self.assertEqual(rows[0]["schema"], 1)
        self.assertIn("scope", rows[0])
        self.assertTrue(rows[0]["no_discovery"])
        samples = [r for r in rows if r["type"] == "sample"]
        self.assertGreaterEqual(len(samples), 2)
        self.assertIn("a", samples[0]["roles"])
        self.assertIn("b", samples[0]["roles"])
        self.assertEqual(samples[0]["roles"]["a"]["pid"], 10)
        self.assertEqual(samples[0]["ps_child_cost"]["status"], "unaccounted")
        summary = rows[-1]
        self.assertEqual(summary["type"], "summary")
        self.assertEqual(summary["status"], "ok")
        self.assertEqual(summary["completion"], "full_duration")
        self.assertTrue(summary["role_lifetimes"]["a"]["complete_for_configured_duration"])

    def test_pid_reuse_identity_change_fails(self):
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            # First tick ok, second tick identity flips for role a (pid 10)
            if calls["n"] <= 2:  # a and b first tick
                return _sample(f"id-{pid}-v1", user=1.0, system=0.1)
            if pid == 10:
                return _sample("id-10-REUSED", user=1.2, system=0.1)
            return _sample(f"id-{pid}-v1", user=1.2, system=0.1)

        status, rows, _ = self._run({"a": 10, "b": 20}, sample_fn=sample, duration=0.35, interval=0.1)
        self.assertEqual(status, 1)
        self.assertEqual(rows[-1]["type"], "summary")
        self.assertEqual(rows[-1]["status"], "fail")
        self.assertIn("identity", rows[-1]["fail_reason"].lower())
        self.assertGreaterEqual(rows[-1]["sample_count"], 1)

    def test_decreasing_cpu_counters_fail(self):
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            if calls["n"] <= 2:
                return _sample(f"id-{pid}", user=5.0, system=2.0)
            return _sample(f"id-{pid}", user=1.0, system=2.0)  # decrease user

        status, rows, _ = self._run({"a": 10, "b": 20}, sample_fn=sample, duration=0.35, interval=0.1)
        self.assertEqual(status, 1)
        self.assertEqual(rows[-1]["status"], "fail")
        self.assertIn("reset", rows[-1]["fail_reason"].lower())

    def test_missing_role_fails_and_retains_partial(self):
        def sample(pid, timeout=None):
            if pid == 20:
                raise sr.SampleError("required process sample unreadable: gone")
            return _sample(f"id-{pid}")

        status, rows, _ = self._run({"a": 10, "b": 20}, sample_fn=sample, duration=0.2, interval=0.1)
        self.assertEqual(status, 1)
        samples = [r for r in rows if r["type"] == "sample"]
        self.assertGreaterEqual(len(samples), 1)
        self.assertEqual(samples[0]["roles"]["b"]["status"], "error")
        self.assertEqual(rows[-1]["status"], "fail")
        self.assertTrue(rows[-1]["role_errors"])

    def test_cadence_gap_fails(self):
        clock = Clock()

        def stalled(pid, timeout=None):
            clock.now += 0.5
            return _sample(f"id-{pid}", user=1.0 + clock.now, system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=stalled,
            duration=0.3,
            interval=0.1,
            clock=clock,
        )
        self.assertEqual(status, 1)
        self.assertEqual(rows[-1]["status"], "fail")
        self.assertIn("cadence", rows[-1]["fail_reason"].lower())

    def test_orderly_early_stop_is_incomplete_not_full_ok(self):
        clock = Clock()
        stop = threading.Event()
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            # After first full tick (one role), request stop before duration ends.
            if calls["n"] >= 1:
                stop.set()
            return _sample(f"id-{pid}", user=1.0 + 0.01 * calls["n"], system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample,
            duration=5.0,
            interval=0.1,
            clock=clock,
            stop_event=stop,
        )
        self.assertEqual(status, 1)
        summary = rows[-1]
        self.assertEqual(summary["status"], "incomplete")
        self.assertEqual(summary["completion"], "partial_orchestrator_stop")
        self.assertEqual(summary["stop_reason"], "orchestrator_stop")
        self.assertFalse(summary["role_lifetimes"]["a"]["complete_for_configured_duration"])
        self.assertIsNotNone(summary["role_lifetimes"]["a"]["first_ok_monotonic_s"])

    def test_duplicate_pid_in_run_rejected_before_write(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            with self.assertRaises(pa.AccountingError):
                pa.run(
                    {"a": 5, "b": 5},
                    out,
                    0.1,
                    0.2,
                    include_collector_self=False,
                    install_signal_handlers=False,
                    sample_fn=lambda pid, timeout=None: _sample("x"),
                    pressure_fn=lambda: {},
                    monotonic_fn=Clock().monotonic,
                    sleep_fn=Clock().sleep,
                )
            self.assertFalse(out.exists())

    def test_malformed_interval_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            with self.assertRaises(pa.AccountingError):
                pa.run(
                    {"a": 1},
                    out,
                    0.0,
                    1.0,
                    include_collector_self=False,
                    install_signal_handlers=False,
                )

    def test_write_failure_existing_output(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            out.write_text("sentinel\n")
            with self.assertRaises(sr.SampleError):
                pa.open_output(out, force=False)
            status = pa.main(["a=1", str(out), "--duration", "0.1", "--no-collector-self"])
            self.assertEqual(status, 1)
            self.assertEqual(out.read_text(), "sentinel\n")

    def test_streaming_bounded_sample_count(self):
        """Configured duration/interval bounds the number of sample rows."""
        clock = Clock()
        n_calls = {"c": 0}

        def sample(pid, timeout=None):
            n_calls["c"] += 1
            return _sample(f"id-{pid}", user=1.0 + 0.01 * n_calls["c"], system=0.0)

        status, rows, _ = self._run(
            {"only": 3},
            sample_fn=sample,
            duration=0.45,
            interval=0.1,
            clock=clock,
        )
        self.assertEqual(status, 0)
        samples = [r for r in rows if r["type"] == "sample"]
        # scheduled 0.0, 0.1, 0.2, 0.3, 0.4 — five slots before 0.45 deadline
        self.assertLessEqual(len(samples), 5)
        self.assertGreaterEqual(len(samples), 3)

    def test_collector_self_auto_added(self):
        seen_pids = []

        def sample(pid, timeout=None):
            seen_pids.append(pid)
            return _sample(f"id-{pid}", user=1.0 + 0.01 * len(seen_pids), system=0.0)

        status, rows, _ = self._run(
            {"game": 99},
            sample_fn=sample,
            duration=0.15,
            interval=0.1,
            include_collector_self=True,
        )
        self.assertEqual(status, 0)
        self.assertIn(4242, seen_pids)
        self.assertIn(99, seen_pids)
        self.assertIn("collector", rows[0]["roles"])


class CliAndSmokeTests(unittest.TestCase):
    def test_cli_malformed_config_exit1(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "out.jsonl"
            completed = subprocess.run(
                [sys.executable, str(ROOT / "process_accounting.py"), "badspec", str(out), "--duration", "0.1"],
                text=True,
                capture_output=True,
            )
            self.assertEqual(completed.returncode, 1)
            self.assertIn("FAIL:", completed.stderr)

    def test_cli_duplicate_pid_exit1(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "out.jsonl"
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "process_accounting.py"),
                    "a=1",
                    "b=1",
                    str(out),
                    "--duration",
                    "0.1",
                ],
                text=True,
                capture_output=True,
            )
            self.assertEqual(completed.returncode, 1)
            self.assertIn("duplicate", completed.stderr.lower())

    def test_optional_dummy_child_smoke(self):
        """Tiny local child process only — no bot/game-server launch."""
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(2.0)"],
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                out = pathlib.Path(td) / "smoke.jsonl"
                cmd = [
                    sys.executable,
                    str(ROOT / "process_accounting.py"),
                    f"dummy_child={child.pid}",
                    str(out),
                    "--interval",
                    "0.05",
                    "--duration",
                    "0.2",
                ]
                completed = subprocess.run(cmd, text=True, capture_output=True)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                rows = [json.loads(line) for line in out.read_text().splitlines()]
                self.assertEqual(rows[0]["type"], "metadata")
                self.assertIn("dummy_child", rows[0]["roles"])
                self.assertIn("collector", rows[0]["roles"])
                samples = [r for r in rows if r["type"] == "sample"]
                self.assertGreaterEqual(len(samples), 2)
                self.assertEqual(rows[-1]["status"], "ok")
                # Collector and child remain separate series
                self.assertIn("dummy_child", samples[0]["roles"])
                self.assertEqual(samples[0]["roles"]["dummy_child"]["status"], "ok")
                self.assertGreater(samples[0]["roles"]["dummy_child"]["resident_bytes"], 0)
        finally:
            child.terminate()
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=2)


if __name__ == "__main__":
    unittest.main()
