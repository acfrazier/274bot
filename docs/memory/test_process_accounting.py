#!/usr/bin/env python3
"""Tests for continuous explicit-PID multi-role process accounting."""
from __future__ import annotations

import json
import math
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import unittest
from types import SimpleNamespace
from typing import Any, Dict, List, Optional

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


class FakeChildrenRusage:
    def __init__(self) -> None:
        self.user = 0.0
        self.system = 0.0

    def snapshot(self):
        return SimpleNamespace(ru_utime=self.user, ru_stime=self.system, ru_maxrss=999999)

    def bump(self, u: float = 0.01, s: float = 0.002) -> None:
        self.user += u
        self.system += s


class ConfigTests(unittest.TestCase):
    def test_wall_bracket_encloses_sampling_but_not_later_pressure_work(self):
        clock = Clock()
        def sample(pid, timeout):
            clock.sleep(0.01)
            return _sample('same')
        def pressure():
            clock.sleep(0.02)
            return {'status': 'unavailable'}
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp) / 'samples.jsonl'
            pa.run({'a': 1}, out, interval=1.0, duration=0.5,
                   include_collector_self=False, sample_fn=sample,
                   pressure_fn=pressure, monotonic_fn=clock.monotonic,
                   sleep_fn=clock.sleep, utc_fn=lambda: str(clock.now))
            rows = [json.loads(line) for line in out.read_text().splitlines()]
            row = next(r for r in rows if r['type'] == 'sample')
            self.assertEqual(float(row['acquisition_before_utc']), 0.0)
            self.assertEqual(float(row['acquisition_after_utc']), 0.01)
            self.assertEqual(float(row['utc']), 0.03)

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

    def test_normalize_rejects_bool_pid(self):
        with self.assertRaises(pa.AccountingError):
            pa.normalize_roles({"a": True}, include_collector_self=False)  # type: ignore[dict-item]

    def test_required_grid_count(self):
        self.assertEqual(pa.required_grid_count(0.35, 0.1), 4)
        self.assertEqual(pa.required_grid_count(0.45, 0.1), 5)
        self.assertEqual(pa.required_grid_count(0.3, 0.1), 3)
        self.assertEqual(pa.required_grid_count(1.0, 1.0), 1)
        self.assertEqual(pa.required_grid_count(0.4, 0.5), 1)
        # t=0 is still a valid slot whenever duration > 0
        self.assertEqual(pa.required_grid_count(0.05, 0.1), 1)
        with self.assertRaises(pa.AccountingError):
            pa.required_grid_count(0.0, 0.1)
        with self.assertRaises(pa.AccountingError):
            pa.required_grid_count(-1.0, 0.1)


class InjectedCollectorTests(unittest.TestCase):
    def _run(
        self,
        roles: Dict[str, int],
        *,
        interval: float = 0.1,
        duration: Optional[float] = 0.35,
        sample_fn=None,
        stop_event=None,
        include_collector_self: bool = False,
        clock: Optional[Clock] = None,
        install_signal_handlers: bool = False,
        sample_timeout: Optional[float] = None,
        rusage_children_fn=None,
        pressure_fn=None,
    ):
        clock = clock or Clock()
        pressure = pressure_fn or (lambda: {"status": "unavailable", "reason": "test"})
        kids = FakeChildrenRusage()
        if rusage_children_fn is None:
            rusage_children_fn = kids.snapshot

        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            status = pa.run(
                roles,
                out,
                interval,
                duration,
                include_collector_self=include_collector_self,
                sample_fn=sample_fn,
                pressure_fn=pressure,
                monotonic_fn=clock.monotonic,
                sleep_fn=clock.sleep,
                utc_fn=lambda: "1970-01-01T00:00:00Z",
                getpid_fn=lambda: 4242,
                stop_event=stop_event,
                install_signal_handlers=install_signal_handlers,
                sample_timeout=sample_timeout,
                rusage_children_fn=rusage_children_fn,
            )
            text = out.read_text() if out.exists() else ""
            rows = [json.loads(line) for line in text.splitlines() if line.strip()]
            return status, rows, out

    def test_happy_path_multi_role(self):
        state = {"n": 0}
        kids = FakeChildrenRusage()

        def sample(pid, timeout=None):
            n = state["n"]
            state["n"] = n + 1
            kids.bump()
            base_u = 1.0 + 0.1 * n
            return _sample(f"id-{pid}", rss=1000 + pid, user=base_u, system=0.5)

        status, rows, _ = self._run(
            {"a": 10, "b": 20},
            sample_fn=sample,
            duration=0.35,
            interval=0.1,
            rusage_children_fn=kids.snapshot,
        )
        self.assertEqual(status, 0, rows)
        self.assertEqual(rows[0]["type"], "metadata")
        self.assertEqual(rows[0]["schema"], 2)
        self.assertEqual(rows[0]["required_grid_sample_count"], 4)
        self.assertFalse(rows[0]["overwrite_permitted"])
        self.assertTrue(rows[0]["exclusive_create"])
        self.assertIn("bracketing", rows[0])
        self.assertIn("scope", rows[0])
        self.assertTrue(rows[0]["no_discovery"])
        samples = [r for r in rows if r["type"] == "sample"]
        self.assertEqual(len(samples), 4)
        self.assertIn("a", samples[0]["roles"])
        self.assertIn("b", samples[0]["roles"])
        self.assertEqual(samples[0]["roles"]["a"]["pid"], 10)
        # Per-role brackets present and ordered for sequential roles.
        a0 = samples[0]["roles"]["a"]
        b0 = samples[0]["roles"]["b"]
        self.assertIn("acquisition_start_monotonic_s", a0)
        self.assertIn("acquisition_end_monotonic_s", a0)
        self.assertLessEqual(a0["acquisition_start_monotonic_s"], b0["acquisition_start_monotonic_s"])
        # Child CPU available from fake rusage; current RSS not fabricated.
        child = samples[0]["waited_children_cpu"]
        self.assertEqual(child["status"], "available")
        self.assertIsNone(child["current_rss_bytes"])
        self.assertEqual(child["current_rss_status"], "unavailable")
        self.assertGreaterEqual(child["delta_total_s"], 0.0)
        summary = rows[-1]
        self.assertEqual(summary["type"], "summary")
        self.assertEqual(summary["status"], "ok")
        self.assertEqual(summary["completion"], "required_grid_complete")
        self.assertTrue(summary["grid_complete"])
        self.assertEqual(summary["ok_sample_count"], 4)
        self.assertTrue(summary["role_lifetimes"]["a"]["complete_for_required_grid"])
        self.assertFalse(summary["role_lifetimes"]["a"]["complete_for_configured_duration"])
        self.assertFalse(summary["role_lifetimes"]["a"]["full_duration_claim"])
        # observed span is actual first→last, not configured duration
        obs = summary["role_lifetimes"]["a"]["observed_span_s"]
        self.assertIsNotNone(obs)
        self.assertLess(obs, 0.35)

    def test_pid_reuse_identity_change_fails(self):
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            if calls["n"] <= 2:
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
            return _sample(f"id-{pid}", user=1.0, system=2.0)

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

    def test_orderly_early_stop_fixed_duration_is_incomplete_not_full_ok(self):
        clock = Clock()
        stop = threading.Event()
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
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
        self.assertFalse(summary["role_lifetimes"]["a"]["full_duration_claim"])
        self.assertIsNotNone(summary["role_lifetimes"]["a"]["first_ok_monotonic_s"])

    def test_stop_controlled_requested_stop_is_closed_exit0(self):
        """duration=None + orchestrator stop → controlled_stop / exit 0 (clarification)."""
        clock = Clock()
        stop = threading.Event()
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            if calls["n"] >= 2:
                stop.set()
            return _sample(f"id-{pid}", user=1.0 + 0.01 * calls["n"], system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample,
            duration=None,
            interval=0.1,
            clock=clock,
            stop_event=stop,
            sample_timeout=1.0,
        )
        self.assertEqual(status, 0, rows)
        summary = rows[-1]
        self.assertEqual(summary["status"], "closed")
        self.assertEqual(summary["completion"], "controlled_stop")
        self.assertIsNone(summary["required_grid_sample_count"])
        self.assertFalse(summary["role_lifetimes"]["a"]["full_duration_claim"])
        self.assertIsNotNone(summary["observed_covered_span_s"])
        self.assertIn("reader_note", summary)

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

    def test_write_failure_existing_output_no_force(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            out.write_text("sentinel\n")
            with self.assertRaises(sr.SampleError):
                pa.open_output(out)
            # No --force on CLI; existing path must fail exclusive create.
            status = pa.main(
                [
                    "a=1",
                    str(out),
                    "--duration",
                    "0.2",
                    "--interval",
                    "0.1",
                    "--no-collector-self",
                ]
            )
            self.assertEqual(status, 1)
            self.assertEqual(out.read_text(), "sentinel\n")
            # force kwargs removed from public run/open_output surface.
            self.assertFalse(hasattr(pa.run, "__force_removed__"))
            import inspect

            self.assertNotIn("force", inspect.signature(pa.run).parameters)
            self.assertNotIn("force", inspect.signature(pa.open_output).parameters)

    def test_streaming_bounded_sample_count_matches_grid(self):
        clock = Clock()
        n_calls = {"c": 0}

        def sample(pid, timeout=None):
            n_calls["c"] += 1
            return _sample(f"id-{pid}", user=1.0 + 0.01 * n_calls["c"], system=0.0)

        # duration 0.45 / interval 0.1 → slots 0.0..0.4 → 5 required
        status, rows, _ = self._run(
            {"only": 3},
            sample_fn=sample,
            duration=0.45,
            interval=0.1,
            clock=clock,
        )
        self.assertEqual(status, 0, rows)
        samples = [r for r in rows if r["type"] == "sample"]
        self.assertEqual(len(samples), 5)
        self.assertEqual(rows[0]["required_grid_sample_count"], 5)
        self.assertEqual(rows[-1]["ok_sample_count"], 5)
        self.assertTrue(rows[-1]["grid_complete"])

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

    # --- production negatives for root corrections ---

    def test_force_flag_absent_from_cli(self):
        completed = subprocess.run(
            [sys.executable, str(ROOT / "process_accounting.py"), "--help"],
            text=True,
            capture_output=True,
        )
        self.assertEqual(completed.returncode, 0)
        self.assertNotIn("--force", completed.stdout)

    def test_sample_timeout_default_finite_stop_controlled(self):
        """Stop-controlled path always gets a finite positive timeout (default 5s)."""
        clock = Clock()
        stop = threading.Event()
        seen_timeouts: List[Optional[float]] = []
        calls = {"n": 0}

        def sample(pid, timeout=None):
            seen_timeouts.append(timeout)
            calls["n"] += 1
            if calls["n"] >= 1:
                stop.set()
            return _sample(f"id-{pid}", user=1.0 + 0.01 * calls["n"], system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample,
            duration=None,
            interval=0.1,
            clock=clock,
            stop_event=stop,
            sample_timeout=None,  # default
        )
        self.assertEqual(status, 0, rows)
        self.assertTrue(seen_timeouts)
        for t in seen_timeouts:
            self.assertIsInstance(t, (int, float))
            tf = float(t)  # type: ignore[arg-type]
            self.assertGreater(tf, 0.0)
            self.assertTrue(math.isfinite(tf))
        self.assertEqual(rows[0]["sample_timeout_s"], pa.DEFAULT_SAMPLE_TIMEOUT_S)

    def test_sample_timeout_invalid_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            with self.assertRaises(pa.AccountingError):
                pa.run(
                    {"a": 1},
                    out,
                    0.1,
                    0.2,
                    include_collector_self=False,
                    install_signal_handlers=False,
                    sample_timeout=0.0,
                    sample_fn=lambda pid, timeout=None: _sample("x"),
                    pressure_fn=lambda: {},
                    monotonic_fn=Clock().monotonic,
                    sleep_fn=Clock().sleep,
                )
            with self.assertRaises(pa.AccountingError):
                pa.run(
                    {"a": 1},
                    out,
                    0.1,
                    None,
                    include_collector_self=False,
                    install_signal_handlers=False,
                    stop_event=threading.Event(),
                    sample_timeout=float("nan"),
                    sample_fn=lambda pid, timeout=None: _sample("x"),
                    pressure_fn=lambda: {},
                    monotonic_fn=Clock().monotonic,
                    sleep_fn=Clock().sleep,
                )

    def test_per_role_timeout_recomputed_not_stale(self):
        """Each role recomputes remaining budget; slow first role shrinks second timeout."""
        clock = Clock()
        timeouts: List[float] = []

        def sample(pid, timeout=None):
            timeouts.append(float(timeout))
            if pid == 10:
                # Consume wall inside the sample (after timeout was computed for this role).
                clock.now += 0.15
            return _sample(f"id-{pid}", user=1.0 + clock.now, system=0.1)

        # duration 0.4 from t=0; roles a then b; sample_timeout large so remaining binds.
        status, rows, _ = self._run(
            {"a": 10, "b": 20},
            sample_fn=sample,
            duration=0.4,
            interval=0.5,  # single grid slot at 0.0
            clock=clock,
            sample_timeout=10.0,
        )
        # May fail cadence after slow sample; still assert timeout recompute happened.
        self.assertGreaterEqual(len(timeouts), 2)
        # First role got ~0.4 remaining; second got ~0.25 after 0.15 burn.
        self.assertAlmostEqual(float(timeouts[0]), 0.4, places=5)
        self.assertAlmostEqual(float(timeouts[1]), 0.25, places=5)

    def test_per_role_brackets_differ_for_sequential_ps(self):
        clock = Clock()

        def sample(pid, timeout=None):
            clock.now += 0.01  # sequential cost per role
            return _sample(f"id-{pid}", user=1.0 + clock.now, system=0.1)

        status, rows, _ = self._run(
            {"a": 10, "b": 20},
            sample_fn=sample,
            duration=0.15,
            interval=0.1,
            clock=clock,
            sample_timeout=1.0,
        )
        self.assertEqual(status, 0, rows)
        s0 = [r for r in rows if r["type"] == "sample"][0]
        a = s0["roles"]["a"]
        b = s0["roles"]["b"]
        self.assertLess(a["acquisition_start_monotonic_s"], b["acquisition_start_monotonic_s"])
        self.assertLess(a["acquisition_end_monotonic_s"], b["acquisition_end_monotonic_s"])
        self.assertEqual(a["cpu"]["interval_basis"], "per_role_acquisition_start")
        summary = rows[-1]
        lifetimes = summary['role_lifetimes'].values()
        intersection = min(r['last_ok_monotonic_s'] for r in lifetimes) - max(r['first_ok_monotonic_s'] for r in lifetimes)
        self.assertAlmostEqual(summary['observed_covered_span_s'], max(0, intersection))
        union = max(r['last_ok_monotonic_s'] for r in lifetimes) - min(r['first_ok_monotonic_s'] for r in lifetimes)
        self.assertLess(summary['observed_covered_span_s'], union)

    def test_clock_jump_skips_required_grid_fails(self):
        clock = Clock()
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            # After first tick, jump clock past remaining grid.
            if calls["n"] == 1:
                clock.now = 10.0
            return _sample(f"id-{pid}", user=1.0 + 0.01 * calls["n"], system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample,
            duration=0.35,
            interval=0.1,
            clock=clock,
        )
        self.assertEqual(status, 1)
        summary = rows[-1]
        self.assertEqual(summary["status"], "fail")
        self.assertNotEqual(summary.get("completion"), "full_duration")
        self.assertFalse(summary.get("grid_complete", True))
        # Either cadence miss on next schedule or required grid incomplete.
        reason = (summary.get("fail_reason") or "").lower()
        self.assertTrue("cadence" in reason or "grid" in reason or "duration exhausted" in reason)

    def test_missing_tick_late_before_deadline_fails_not_full_ok(self):
        """If remaining duration hits before a still-required scheduled sample → fail."""
        clock = Clock()

        def sample(pid, timeout=None):
            # First sample ok at t~0; then sleep loop will see jumped time.
            return _sample(f"id-{pid}", user=1.0 + clock.now, system=0.1)

        # Manually: after first sample, advance clock near deadline so second scheduled
        # tick (0.1) cannot complete before duration 0.15 with slow acquisition.
        calls = {"n": 0}

        def sample2(pid, timeout=None):
            calls["n"] += 1
            if calls["n"] == 1:
                out = _sample(f"id-{pid}", user=1.0, system=0.1)
                clock.now = 0.14  # past scheduled 0.1 with lateness beyond tolerance? 
                # interval 0.1, tolerance ~0.05; lateness 0.04 is within tolerance.
                # Jump further so duration exhausted before taking index 1.
                clock.now = 0.16
                return out
            return _sample(f"id-{pid}", user=1.1, system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample2,
            duration=0.15,
            interval=0.1,
            clock=clock,
            sample_timeout=1.0,
        )
        self.assertEqual(status, 1, rows)
        summary = rows[-1]
        self.assertEqual(summary["status"], "fail")
        self.assertNotEqual(summary["completion"], "full_duration")
        self.assertFalse(summary["role_lifetimes"]["a"]["complete_for_configured_duration"])
        self.assertFalse(summary.get("grid_complete", True))
        # observed span is actual first→last ok, never a synthetic "full duration" claim
        self.assertFalse(summary["role_lifetimes"]["a"]["full_duration_claim"])

    def test_child_counters_separate_no_fabricated_rss(self):
        kids = FakeChildrenRusage()
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            kids.bump(0.05, 0.01)
            return _sample(f"id-{pid}", user=1.0 + 0.01 * calls["n"], system=0.1)

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample,
            duration=0.15,
            interval=0.1,
            rusage_children_fn=kids.snapshot,
        )
        self.assertEqual(status, 0, rows)
        for s in rows:
            if s["type"] != "sample":
                continue
            w = s["waited_children_cpu"]
            self.assertEqual(w["status"], "available")
            self.assertIn("delta_user_s", w)
            self.assertIn("cumulative_user_s", w)
            self.assertIsNone(w["current_rss_bytes"])
            self.assertEqual(w["current_rss_status"], "unavailable")
            # maxrss must not appear as current
            self.assertNotIn("ru_maxrss", w)

    def test_malformed_sample_values_fail(self):
        def bad_identity(pid, timeout=None):
            return {"start_identity": "", "resident_bytes": 1, "user_s": 1.0, "system_s": 0.0}

        status, rows, _ = self._run({"a": 10}, sample_fn=bad_identity, duration=0.15, interval=0.1)
        self.assertEqual(status, 1)
        self.assertEqual(rows[-1]["status"], "fail")

        def neg_rss(pid, timeout=None):
            return _sample("id", rss=-5)

        status2, rows2, _ = self._run({"a": 10}, sample_fn=neg_rss, duration=0.15, interval=0.1)
        self.assertEqual(status2, 1)
        self.assertEqual(rows2[-1]["status"], "fail")

        def nan_cpu(pid, timeout=None):
            return {
                "start_identity": "x",
                "resident_bytes": 1,
                "user_s": float("nan"),
                "system_s": 0.0,
            }

        status3, rows3, _ = self._run({"a": 10}, sample_fn=nan_cpu, duration=0.15, interval=0.1)
        self.assertEqual(status3, 1)
        self.assertEqual(rows3[-1]["status"], "fail")

    def test_malformed_pressure_fails_cleanly(self):
        def sample(pid, timeout=None):
            return _sample(f"id-{pid}")

        def bad_pressure():
            raise RuntimeError("pressure boom")

        status, rows, _ = self._run(
            {"a": 10},
            sample_fn=sample,
            duration=0.15,
            interval=0.1,
            pressure_fn=bad_pressure,
        )
        self.assertEqual(status, 1)
        self.assertEqual(rows[-1]["status"], "fail")
        self.assertIn("pressure", (rows[-1].get("fail_reason") or "").lower())

    def test_stop_controlled_without_stop_or_signals_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            out = pathlib.Path(td) / "acct.jsonl"
            with self.assertRaises(pa.AccountingError) as ctx:
                pa.run(
                    {"a": 1},
                    out,
                    0.1,
                    None,
                    include_collector_self=False,
                    install_signal_handlers=False,
                    stop_event=None,
                    sample_fn=lambda pid, timeout=None: _sample("x"),
                    pressure_fn=lambda: {},
                    monotonic_fn=Clock().monotonic,
                    sleep_fn=Clock().sleep,
                )
            self.assertIn("refusing to hang", str(ctx.exception).lower())
            self.assertFalse(out.exists())

    def test_partial_output_preserved_on_fail(self):
        calls = {"n": 0}

        def sample(pid, timeout=None):
            calls["n"] += 1
            if calls["n"] > 2:
                raise sr.SampleError("gone")
            return _sample(f"id-{pid}", user=1.0 + 0.01 * calls["n"], system=0.1)

        status, rows, _out = self._run(
            {"a": 10, "b": 20},
            sample_fn=sample,
            duration=0.35,
            interval=0.1,
        )
        self.assertEqual(status, 1)
        # rows were read before temp teardown — partial JSONL retained during run
        self.assertGreaterEqual(len(rows), 2)  # metadata + at least one sample
        self.assertEqual(rows[0]["type"], "metadata")
        self.assertEqual(rows[-1]["type"], "summary")
        self.assertEqual(rows[-1]["status"], "fail")


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
                    "--sample-timeout",
                    "2.0",
                ]
                completed = subprocess.run(cmd, text=True, capture_output=True)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                rows = [json.loads(line) for line in out.read_text().splitlines()]
                self.assertEqual(rows[0]["type"], "metadata")
                self.assertEqual(rows[0]["schema"], 2)
                self.assertIn("dummy_child", rows[0]["roles"])
                self.assertIn("collector", rows[0]["roles"])
                samples = [r for r in rows if r["type"] == "sample"]
                self.assertGreaterEqual(len(samples), 2)
                self.assertEqual(rows[-1]["status"], "ok")
                self.assertTrue(rows[-1]["grid_complete"])
                self.assertIn("dummy_child", samples[0]["roles"])
                self.assertEqual(samples[0]["roles"]["dummy_child"]["status"], "ok")
                self.assertGreater(samples[0]["roles"]["dummy_child"]["resident_bytes"], 0)
                # Real rusage children path present; RSS current still unavailable.
                w = samples[0]["waited_children_cpu"]
                self.assertIn(w["status"], ("available", "unavailable"))
                self.assertEqual(w["current_rss_status"], "unavailable")
                self.assertIsNone(w["current_rss_bytes"])
        finally:
            child.terminate()
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=2)


if __name__ == "__main__":
    unittest.main()
