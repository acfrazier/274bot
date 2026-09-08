#!/usr/bin/env python3
"""Tests for reference_metrics bounded histogram / observation-window core."""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import reference_metrics as rm  # noqa: E402

CLI = ROOT / "reference_metrics.py"
# Reviewed low-end reference cell: scheduling + responsiveness flags OFF.
REAL_FLAGS_OFF = (
    ROOT
    / "diagnostics"
    / "low-end-reference-screen-20260906T220129Z"
    / "tui_n1_active"
)
CONTAM_UTC = "2026-09-06T22:21:17Z"


def _write_jsonl(path: pathlib.Path, rows):
    path.write_text("".join(json.dumps(r) + "\n" for r in rows))


def _meta(**flags):
    base = {
        "exit_code": 0,
        "n": 1,
        "observe_s": 120,
        "warmup_s": 30,
        "workload": "active",
        "frontend": "tui",
        "started_unix": 1_000_000.0,
        "ended_unix": 1_000_250.0,
        "scheduling_profile": False,
        "responsiveness_profile": False,
        "render_profile": False,
        "gpu_completion_profile": False,

        "nav_pack_sha256": "nav-pack",
        "nav_flags_sha256": "nav-flags",
        "renderer_settings": {"quality": "default"},
        "cache_settings": {"cache": "default"},
        "catalog_sha256": "catalog",
        "feature_flags": {"feature": True},
        "allocator_provenance": "system",
        "host_sources_sha256": "host",
        "client_sources_sha256": "client",
        "binary_sha256": "binary",
    }
    base.update(flags)
    return base


class P99BoundsTests(unittest.TestCase):
    def test_empty_unavailable(self):
        b = rm.p99_lower_upper_ms([0] * 11, rm.LATENCY_BOUNDS_MS)
        self.assertEqual(b["status"], "unavailable")
        self.assertEqual(b["reason"], "empty_histogram")

    def test_missing_unavailable(self):
        b = rm.p99_lower_upper_ms(None, rm.LATENCY_BOUNDS_MS)
        self.assertEqual(b["reason"], "missing_histogram")

    def test_overflow_unavailable(self):
        buckets = [0] * 10 + [100]
        b = rm.p99_lower_upper_ms(buckets, rm.LATENCY_BOUNDS_MS)
        self.assertEqual(b["status"], "unavailable")
        self.assertEqual(b["reason"], "overflow_bucket")
        self.assertIsNone(b.get("upper_ms"))

    def test_upper_and_lower_coarse(self):
        # 100 samples all in ≤50ms (index of 50 in LATENCY_BOUNDS = 5)
        buckets = [0] * 11
        buckets[5] = 100
        b = rm.p99_lower_upper_ms(buckets, rm.LATENCY_BOUNDS_MS)
        self.assertEqual(b["status"], "available")
        self.assertEqual(b["upper_ms"], 50)
        self.assertEqual(b["lower_ms"], 40)  # previous bound
        self.assertFalse(b["precise_percentile"])

    def test_p99_crosses_into_higher_bucket(self):
        # 98 in ≤50, 2 in ≤100 → p99 in ≤100 bucket
        buckets = [0] * 11
        buckets[5] = 98
        buckets[6] = 2
        b = rm.p99_lower_upper_ms(buckets, rm.LATENCY_BOUNDS_MS)
        self.assertEqual(b["upper_ms"], 100)
        self.assertEqual(b["lower_ms"], 50)

    def test_width_mismatch(self):
        b = rm.p99_lower_upper_ms([1, 2, 3], rm.SCHED_EXCESS_BOUNDS_MS)
        self.assertEqual(b["reason"], "histogram_width_mismatch")

    def test_target_verdict_meet_miss_unproven(self):
        meet = {"status": "available", "lower_ms": 0, "upper_ms": 40}
        miss = {"status": "available", "lower_ms": 50, "upper_ms": 100}
        straddle = {"status": "available", "lower_ms": 20, "upper_ms": 100}
        self.assertEqual(rm.target_verdict(meet, 40), "meet")
        self.assertEqual(rm.target_verdict(miss, 40), "miss")
        self.assertEqual(rm.target_verdict(straddle, 40), "unproven")
        self.assertEqual(rm.target_verdict({"status": "unavailable"}, 40), "unavailable")


class CounterResetTests(unittest.TestCase):
    def test_subtract_ok(self):
        self.assertEqual(rm.subtract_counts([5, 7], [2, 3]), [3, 4])

    def test_subtract_reset_rejected(self):
        self.assertIsNone(rm.subtract_counts([1, 2], [3, 0]))

    def test_generation_mismatch_on_slot(self):
        s = {"slot_id": 1, "generation": 1, "decode_latency_buckets": [0] * 11}
        e = {"slot_id": 1, "generation": 2, "decode_latency_buckets": [1] + [0] * 10}
        delta, err = rm._slot_hist_delta(s, e, "decode_latency_buckets")
        self.assertIsNone(delta)
        self.assertEqual(err, "generation_mismatch")


class SchedulingExcessBoundsTests(unittest.TestCase):
    def test_first_excess_bucket_lower_is_zero_not_baseline(self):
        # cadence: excess=interval.saturating_sub(20ms); bucket0 holds sub-budget too
        excess = rm.p99_lower_upper_ms([100, 0, 0, 0, 0, 0], rm.SCHED_EXCESS_BOUNDS_MS)
        abs_b = rm.scheduling_interval_bounds_from_excess(excess)
        self.assertEqual(abs_b["status"], "available")
        self.assertEqual(abs_b["lower_ms"], 0)
        self.assertEqual(abs_b["upper_ms"], 21)  # 20 + 1
        self.assertEqual(abs_b["observation_coverage"], "inconclusive_process_wide_batch_lag")

    def test_higher_excess_bucket_lower_uses_baseline(self):
        excess = rm.p99_lower_upper_ms([0, 0, 100, 0, 0, 0], rm.SCHED_EXCESS_BOUNDS_MS)
        abs_b = rm.scheduling_interval_bounds_from_excess(excess)
        # bounds [1,2,5,...] index 2 → excess lower=2 upper=5 → abs 22..25
        self.assertEqual(abs_b["lower_ms"], 22)
        self.assertEqual(abs_b["upper_ms"], 25)

    def test_process_wide_cannot_meet_per_slot(self):
        meta = _meta(scheduling_profile=True)
        # meaningful cumulative delta on drawing group
        start = {
            "phase": "observe",
            "elapsed_s": 30.0,
            "scheduling": [
                {
                    "drawing": False,
                    "cycles": 0,
                    "intervals": 0,
                    "interval_excess_buckets": [0] * 6,
                    "sleep_excess_buckets": [0] * 6,
                },
                {
                    "drawing": True,
                    "cycles": 100,
                    "intervals": 100,
                    "interval_excess_buckets": [10, 0, 0, 0, 0, 0],
                    "sleep_excess_buckets": [10, 0, 0, 0, 0, 0],
                },
            ],
        }
        end = {
            "phase": "observe",
            "elapsed_s": 150.0,
            "scheduling": [
                {
                    "drawing": False,
                    "cycles": 0,
                    "intervals": 0,
                    "interval_excess_buckets": [0] * 6,
                    "sleep_excess_buckets": [0] * 6,
                },
                {
                    "drawing": True,
                    "cycles": 5000,
                    "intervals": 5000,
                    "interval_excess_buckets": [4000, 500, 400, 50, 40, 10],
                    "sleep_excess_buckets": [4000, 500, 400, 50, 40, 10],
                },
            ],
        }
        g = rm.evaluate_scheduling(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["reason"], "process_wide_only_cannot_satisfy_per_slot")
        self.assertFalse(g["satisfies_per_slot_requirement"])
        self.assertEqual(g["target_verdict"], "unavailable")
        # diagnostic groups still present
        groups = g["process_wide_groups"]
        drawing = [x for x in groups if x.get("drawing")][0]
        self.assertEqual(drawing["exact_observation_coverage"], "inconclusive")
        self.assertFalse(drawing["satisfies_per_slot_requirement"])
        # Diagnostic absolute bounds may be available process-wide; still not a gate meet.
        self.assertIn(drawing["interval_p99"]["status"], ("available", "unavailable"))
        self.assertEqual(drawing["target_verdict"], "unavailable")
        self.assertEqual(
            drawing["interval_p99"].get("observation_coverage"),
            "inconclusive_process_wide_batch_lag",
        )

    def test_profile_disabled(self):
        g = rm.evaluate_scheduling(_meta(scheduling_profile=False), [])
        self.assertEqual(g["reason"], "profile_disabled")


class PerSlotSchedulingTests(unittest.TestCase):
    def _row(self, *, slot=1, generation=1, mono=0, cycle=0, intervals=0,
             ended=False, age=10, interval_bucket=None, mode_break=0,
             park=0, anchor_miss=0, lost=0):
        buckets = [0] * (len(rm.SLOT_INTERVAL_BOUNDS_MS) + 1)
        if interval_bucket is not None:
            buckets[interval_bucket] = intervals
        return {
            "slot_id": slot, "generation": generation, "sample_age_ms": age,
            "updated_ms": 123, "ended": ended, "cycle_n": cycle,
            "interval_n": intervals, "interval_buckets": buckets,
            "interval_bound_ms": list(rm.SLOT_INTERVAL_BOUNDS_MS),
            "interval_coverage_complete": cycle == intervals + mode_break + anchor_miss,
            "mode_break_n": mode_break, "park_n": park, "anchor_miss_n": anchor_miss,
            "ended_lost_n": lost, "last_cycle_mono_ms": mono,
            "last_cycle_ms": 1_000_000 + mono,
            "flush_lag_max_ms": 1000, "flush_lag_max_cycles": 50,
            "scene_transition_separation": "unavailable",
        }

    def _pair(self, start, end):
        return [
            {"phase": "observe", "elapsed_s": 1, "scheduling_slots": start},
            {"phase": "observe", "elapsed_s": 999, "scheduling_slots": end},
        ]

    def test_uses_last_cycle_endpoints_not_lifetime_first_interval(self):
        meta = _meta(scheduling_profile=True)
        start = self._row(mono=10_000, cycle=100, intervals=99, interval_bucket=3, anchor_miss=1)
        start.update({"first_interval_start_mono_ms": 1, "first_interval_ms": 1})
        end = self._row(mono=11_000, cycle=140, intervals=139, interval_bucket=3, anchor_miss=1)
        end.update({"first_interval_start_mono_ms": 1, "first_interval_ms": 1})
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end]))
        self.assertEqual(result["status"], "available")
        slot = result["slots"][0]
        self.assertEqual(slot["measured_duration_ms"], 1000)
        self.assertEqual(slot["interval_n_delta"], 40)
        self.assertEqual(slot["target_verdict"], "meet")
        self.assertEqual(slot["coverage_math"], "cycle_delta = interval_delta + mode_break_delta + anchor_miss_delta")

    def test_local_origins_are_not_compared_between_slots(self):
        meta = _meta(scheduling_profile=True)
        s1 = self._row(slot=1, mono=100_000, cycle=0)
        e1 = self._row(slot=1, mono=101_000, cycle=40, intervals=40, interval_bucket=3)
        s2 = self._row(slot=2, mono=7, cycle=0)
        e2 = self._row(slot=2, mono=1_007, cycle=40, intervals=40, interval_bucket=3)
        result = rm.evaluate_scheduling_slots(meta, self._pair([s1, s2], [e1, e2]))
        self.assertEqual(result["status"], "available")
        self.assertEqual({s["measured_duration_ms"] for s in result["slots"]}, {1000})
        self.assertFalse(result["global_mono_alignment_claim"])

    def test_missing_or_ended_slot_is_unavailable(self):
        meta = _meta(scheduling_profile=True)
        start = self._row(cycle=0)
        end = self._row(cycle=40, intervals=40, interval_bucket=3, ended=True)
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end]))
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["slots"][0]["reason"], "slot_ended")

    def test_flush_lag_and_insufficient_span_are_unavailable(self):
        meta = _meta(scheduling_profile=True)
        start = self._row(mono=10_000, cycle=0, age=2001)
        end = self._row(mono=11_000, cycle=40, intervals=40, interval_bucket=27)
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end]))
        self.assertEqual(result["slots"][0]["reason"], "stale_snapshot")
        start = self._row(mono=10_000, cycle=0)
        end = self._row(mono=10_000, cycle=40, intervals=40, interval_bucket=27)
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end]))
        self.assertEqual(result["slots"][0]["reason"], "invalid_last_cycle_endpoint_span")

    def test_mode_and_park_counters_are_excluded_from_interval_rate(self):
        meta = _meta(scheduling_profile=True)
        start = self._row(mono=10_000, cycle=0)
        end = self._row(mono=11_000, cycle=42, intervals=40, interval_bucket=27,
                        mode_break=1, park=1, anchor_miss=1)
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["slots"][0]["interval_n_delta"], 40)
        self.assertEqual(result["slots"][0]["mode_break_n_delta"], 1)
        self.assertEqual(result["slots"][0]["park_n_delta"], 1)
        self.assertEqual(result["slots"][0]["anchor_miss_n_delta"], 1)

    def test_end_only_expected_slot_is_rejected(self):
        meta = _meta(scheduling_profile=True)
        start = self._row(mono=10_000, cycle=0)
        end = self._row(mono=11_000, cycle=40, intervals=40, interval_bucket=27)
        extra = self._row(slot=2, mono=11_000, cycle=40, intervals=40, interval_bucket=27)
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end, extra]))
        self.assertEqual(result["reason"], "slot_set_changed")

    def test_paired_bounds_are_inconclusive_when_margin_not_proven(self):
        candidate = {"status": "available", "lower_ms": 20, "upper_ms": 22}
        reference = {"status": "available", "lower_ms": 19, "upper_ms": 21}
        result = rm.compare_paired_p99_bounds(candidate, reference)
        self.assertEqual(result["status"], "inconclusive")
        self.assertEqual(result["reason"], "candidate_upper_minus_reference_lower_exceeds_margin")

    def test_one_ms_histogram_bounds_and_counter_reset(self):
        meta = _meta(scheduling_profile=True)
        start = self._row(cycle=0, mono=10_000)
        end = self._row(cycle=40, intervals=40, interval_bucket=27, mono=11_000)
        result = rm.evaluate_scheduling_slots(meta, self._pair([start], [end]))
        self.assertEqual(result["slots"][0]["p99"]["upper_ms"], 40)
        reset = self._row(cycle=40, intervals=40, interval_bucket=3)
        start_reset = self._row(cycle=0, mono=10_000)
        reset["last_cycle_mono_ms"] = 11_000
        start_reset["interval_buckets"][0] = 2
        reset["interval_buckets"][0] = 1
        result = rm.evaluate_scheduling_slots(meta, self._pair([start_reset], [reset]))
        self.assertEqual(result["slots"][0]["reason"], "counter_reset")


class DecodeInputGpuSyntheticTests(unittest.TestCase):
    def _resp_pair(self, **end_extra):
        bounds = list(rm.LATENCY_BOUNDS_MS)
        buckets_s = [0] * 11
        buckets_e = [0] * 11
        buckets_e[4] = 100  # ≤40ms
        base_s = {
            "slot_id": 7,
            "generation": 1,
            "sample_age_ms": 12,
            "decode_latency_buckets": list(buckets_s),
            "input_latency_buckets": list(buckets_s),
            "decode_dropped_n": 0,
            "decode_canceled_n": 0,
            "decode_lost_n": 0,
            "decode_pending_n": 0,
            "decode_coverage_complete": True,
            "input_start_n": 0,
            "input_dropped_n": 0,
            "input_canceled_n": 0,
            "input_lost_n": 0,
            "input_pending_n": 0,
            "input_coverage_complete": True,
            "latency_bound_ms": bounds,
            "visible_ack": {"available": True, "endpoint": "test"},
        }
        base_e = dict(base_s)
        base_e["decode_latency_buckets"] = list(buckets_e)
        base_e["input_latency_buckets"] = list(buckets_e)
        base_e["input_start_n"] = 100
        base_e["input_complete_n"] = 100
        base_e.update(end_extra)
        start = {
            "phase": "observe",
            "elapsed_s": 30.0,
            "responsiveness_profile": [base_s],
        }
        end = {
            "phase": "observe",
            "elapsed_s": 150.0,
            "responsiveness_profile": [base_e],
        }
        return start, end

    def test_decode_available_meet(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        g = rm.evaluate_decode(meta, [start, end], target_ms=40)
        self.assertEqual(g["status"], "available")
        self.assertEqual(g["target_verdict"], "meet")
        self.assertEqual(g["slots"][0]["p99"]["upper_ms"], 40)

    def test_decode_dropped_no_false_pass(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair(decode_dropped_n=1, decode_coverage_complete=False)
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "coverage_lost_or_incomplete")

    def test_decode_canceled_no_false_pass(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair(decode_canceled_n=3)
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["slots"][0]["status"], "unavailable")

    def test_decode_overflow_unavailable(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        end["responsiveness_profile"][0]["decode_latency_buckets"] = [0] * 10 + [50]
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "overflow_bucket")

    def test_decode_counter_reset(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        start["responsiveness_profile"][0]["decode_latency_buckets"] = [50] + [0] * 10
        end["responsiveness_profile"][0]["decode_latency_buckets"] = [10] + [0] * 10
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["slots"][0]["reason"], "counter_reset")

    def test_input_no_samples_unavailable(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        end["responsiveness_profile"][0]["input_start_n"] = 0
        g = rm.evaluate_input(meta, [start, end])
        self.assertEqual(g["slots"][0]["reason"], "no_input_samples")

    def test_input_visible_ack_unavailable(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair(
            visible_ack={
                "available": False,
                "missing_capability": "no scanout",
            }
        )
        g = rm.evaluate_input(meta, [start, end])
        self.assertEqual(g["slots"][0]["reason"], "visible_ack_unavailable")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_input_visible_ack_absent_no_false_pass(self):
        """Absent/malformed visible_ack must not allow input require pass."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        del end["responsiveness_profile"][0]["visible_ack"]
        del start["responsiveness_profile"][0]["visible_ack"]
        g = rm.evaluate_input(meta, [start, end])
        self.assertEqual(g["slots"][0]["reason"], "visible_ack_absent")
        self.assertEqual(g["status"], "unavailable")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_input_visible_ack_malformed_no_false_pass(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair(visible_ack="yes")
        g = rm.evaluate_input(meta, [start, end])
        self.assertEqual(g["slots"][0]["reason"], "visible_ack_absent")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_input_visible_ack_available_missing_key_no_false_pass(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair(visible_ack={"endpoint": "test"})
        g = rm.evaluate_input(meta, [start, end])
        self.assertEqual(g["slots"][0]["reason"], "visible_ack_unavailable")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_decode_sample_age_absent_no_false_pass(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        del end["responsiveness_profile"][0]["sample_age_ms"]
        del start["responsiveness_profile"][0]["sample_age_ms"]
        g = rm.evaluate_decode(meta, [start, end], target_ms=40)
        self.assertEqual(g["slots"][0]["reason"], "sample_age_ms_absent")
        self.assertEqual(g["status"], "unavailable")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_input_sample_age_null_no_false_pass(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        end["responsiveness_profile"][0]["sample_age_ms"] = None
        g = rm.evaluate_input(meta, [start, end], target_ms=40)
        self.assertEqual(g["slots"][0]["reason"], "sample_age_ms_absent")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_gpu_never_paint_proxy_when_disabled(self):
        meta = _meta(render_profile=True, gpu_completion_profile=False)
        # Even with paint histograms present, GPU gate stays unavailable.
        start = {
            "phase": "observe",
            "elapsed_s": 30.0,
            "renderer_profile": [
                {
                    "slot_id": 1,
                    "generation": 1,
                    "paint_n": 100,
                    "stable_paint_interval_buckets": [100] + [0] * 10,
                    "gpu_completion": {"enabled": False},
                }
            ],
        }
        end = {
            "phase": "observe",
            "elapsed_s": 150.0,
            "renderer_profile": [
                {
                    "slot_id": 1,
                    "generation": 1,
                    "paint_n": 5000,
                    "stable_paint_interval_buckets": [5000] + [0] * 10,
                    "gpu_completion": {"enabled": False},
                }
            ],
        }
        g = rm.evaluate_gpu(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["reason"], "gpu_completion_profile_disabled")
        self.assertTrue(g["paint_proxy_forbidden"])
        self.assertTrue(g["raw_gpu_interval_histograms_in_serializer"])

    def test_gpu_product_gate_not_meet_on_completion_latency(self):
        """completion_latency is callback delivery — not product frame/cadence."""
        meta = _meta(render_profile=True, gpu_completion_profile=True)
        bounds = list(rm.INTERVAL_BOUNDS_MS)
        s_buckets = [0] * 11
        e_buckets = [0] * 11
        e_buckets[3] = 200  # ≤40ms completion latency

        def row(buckets, enabled=True):
            return {
                "slot_id": 3,
                "generation": 1,
                "sample_age_ms": 8,
                "gpu_completion": {
                    "enabled": enabled,
                    "dropped_n": 0,
                    "lost_n": 0,
                    "pending_n": 0,
                    "registration_complete": True,
                    "completion_coverage_complete": True,
                    "completion_latency_buckets": buckets,
                    "interval_bound_ms": bounds,
                    # raw interval hists exist on serializer; product adapter deferred
                    "stable_completion_interval_buckets": list(buckets),
                },
            }

        start = {"phase": "observe", "elapsed_s": 30.0, "renderer_profile": [row(s_buckets)]}
        end = {"phase": "observe", "elapsed_s": 150.0, "renderer_profile": [row(e_buckets)]}
        g = rm.evaluate_gpu(meta, [start, end], target_ms=40)
        # Product --require gpu must not pass on latency alone.
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["target_verdict"], "unavailable")
        self.assertEqual(g["gate"], "gpu")
        self.assertIn("interval", g["reason"])
        self.assertFalse(g["paint_proxy_used"])
        self.assertTrue(g["paint_proxy_forbidden"])
        # Distinct diagnostic may score latency; not product cadence.
        diag = g["diagnostic_completion_latency"]
        self.assertEqual(diag["metric"], "gpu_completion_latency")
        self.assertEqual(diag["status"], "available")
        self.assertEqual(diag["target_verdict"], "meet")
        self.assertFalse(rm.gate_satisfies_require(g))

    def test_gpu_stable_interval_adapter_meets_focused_observation(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True)
        def row(completed, intervals):
            gpu = {
                "enabled": True, "registered_n": completed, "completed_n": completed,
                "dropped_n": 0, "lost_n": 0, "pending_n": 0,
                "stable_completed_n": completed,
                "stable_completion_intervals": intervals,
                "stable_completion_interval_buckets": [0, 0, 0, intervals] + [0] * 7,
                "registration_complete": True, "completion_coverage_complete": True,
                "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
            }
            return {
                "slot_id": 3, "generation": 1, "sample_age_ms": 8,
                "renderer_present": True, "backend_kind": "wgpu",
                "ingame": True, "scene_state": 2, "draw": True, "full_rate": True,
                "gpu_completion": gpu,
            }
        start = {"phase": "observe", "elapsed_s": 30.0, "renderer_profile": [row(0, 0)]}
        end = {"phase": "observe", "elapsed_s": 150.0, "renderer_profile": [row(4800, 4800)]}
        g = rm.evaluate_gpu(meta, [start, end])
        self.assertEqual(g["status"], "available")
        self.assertEqual(g["target_verdict"], "meet")
        self.assertEqual(g["slots"][0]["observed_fps"], 40.0)
        self.assertTrue(g["slots"][0]["boundary_accounted"])
        self.assertEqual(g["presentation_endpoint"].split(";")[0], "unavailable")
        self.assertTrue(rm.gate_satisfies_require(g))

    def test_gpu_background_reports_expected_one_fps_without_lowering_policy(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True)
        def row(completed, intervals):
            return {
                "slot_id": 4, "generation": 2, "sample_age_ms": 8,
                "renderer_present": True, "backend_kind": "metal",
                "ingame": True, "scene_state": 2, "draw": False, "full_rate": False,
                "gpu_completion": {
                    "enabled": True, "registered_n": completed, "completed_n": completed,
                    "dropped_n": 0, "lost_n": 0, "pending_n": 0,
                    "stable_completed_n": completed, "stable_completion_intervals": intervals,
                    "stable_completion_interval_buckets": [0, 0, 0, intervals] + [0] * 7,
                    "registration_complete": True, "completion_coverage_complete": True,
                    "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
                },
            }
        g = rm.evaluate_gpu(meta, [
            {"phase": "observe", "elapsed_s": 0.0, "renderer_profile": [row(0, 0)]},
            {"phase": "observe", "elapsed_s": 120.0, "renderer_profile": [row(120, 120)]},
        ])
        self.assertEqual(g["target_verdict"], "meet")
        self.assertEqual(g["slots"][0]["role"], "background")
        self.assertEqual(g["slots"][0]["target"]["expected_fps"], 1.0)
        self.assertEqual(g["slots"][0]["observed_fps"], 1.0)

    def test_gpu_interval_adapter_rejects_boundary_pending(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True)
        row = {
            "slot_id": 9, "generation": 1, "sample_age_ms": 1,
            "renderer_present": True, "backend_kind": "wgpu",
            "ingame": True, "scene_state": 2, "draw": True, "full_rate": True,
            "gpu_completion": {
                "enabled": True, "registered_n": 1, "completed_n": 1,
                "dropped_n": 0, "lost_n": 0, "pending_n": 1,
                "stable_completed_n": 1, "stable_completion_intervals": 1,
                "stable_completion_interval_buckets": [0, 0, 0, 1] + [0] * 7,
                "registration_complete": True, "completion_coverage_complete": True,
                "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
            },
        }
        g = rm.evaluate_gpu(meta, [
            {"phase": "observe", "elapsed_s": 0.0, "renderer_profile": [row]},
            {"phase": "observe", "elapsed_s": 10.0, "renderer_profile": [row]},
        ])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "boundary_pending_incomplete")

    def test_gpu_interval_adapter_rejects_cpu_fallback_backend(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True)
        completion = {
            "enabled": True, "registered_n": 4800, "completed_n": 4800,
            "dropped_n": 0, "lost_n": 0, "pending_n": 0,
            "stable_completed_n": 4800, "stable_completion_intervals": 4800,
            "stable_completion_interval_buckets": [0, 0, 0, 4800] + [0] * 7,
            "registration_complete": True, "completion_coverage_complete": True,
            "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
        }
        row = {
            "slot_id": 9, "generation": 1, "sample_age_ms": 1,
            "renderer_present": True, "backend_kind": "cpu_fallback",
            "ingame": True, "scene_state": 2, "draw": True, "full_rate": True,
            "gpu_completion": completion,
        }
        g = rm.evaluate_gpu(meta, [
            {"phase": "observe", "elapsed_s": 0.0, "renderer_profile": [row]},
            {"phase": "observe", "elapsed_s": 120.0, "renderer_profile": [row]},
        ])
        self.assertEqual(g["status"], "unavailable")
        self.assertFalse(rm.gate_satisfies_require(g))
        self.assertEqual(g["slots"][0]["reason"], "gpu_backend_or_completion_unavailable")

    def test_gpu_lost_no_false_pass(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True)
        start = {
            "phase": "observe",
            "elapsed_s": 1.0,
            "renderer_profile": [
                {
                    "slot_id": 1,
                    "generation": 1,
                    "sample_age_ms": 5,
                    "gpu_completion": {
                        "enabled": True,
                        "dropped_n": 0,
                        "lost_n": 0,
                        "pending_n": 0,
                        "registration_complete": True,
                        "completion_coverage_complete": True,
                        "completion_latency_buckets": [0] * 11,
                        "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
                    },
                }
            ],
        }
        end = {
            "phase": "observe",
            "elapsed_s": 2.0,
            "renderer_profile": [
                {
                    "slot_id": 1,
                    "generation": 1,
                    "sample_age_ms": 5,
                    "gpu_completion": {
                        "enabled": True,
                        "dropped_n": 0,
                        "lost_n": 4,
                        "pending_n": 0,
                        "registration_complete": False,
                        "completion_coverage_complete": False,
                        "completion_latency_buckets": [10] + [0] * 10,
                        "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
                    },
                }
            ],
        }
        g = rm.evaluate_gpu(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        diag = g["diagnostic_completion_latency"]
        self.assertEqual(diag["slots"][0]["reason"], "coverage_lost_or_incomplete")

    def test_duplicate_slot_generation_rejected(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        dup = dict(end["responsiveness_profile"][0])
        end["responsiveness_profile"] = [end["responsiveness_profile"][0], dup]
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["reason"], "duplicate_slot_generation")

    def test_disappearing_start_slot_rejected(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        extra = dict(start["responsiveness_profile"][0])
        extra["slot_id"] = 99
        start["responsiveness_profile"] = [start["responsiveness_profile"][0], extra]
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["reason"], "disappearing_expected_slots")
        self.assertIn([99, 1], g.get("disappeared", []))

    def test_missing_coverage_flag_unavailable(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._resp_pair()
        del end["responsiveness_profile"][0]["decode_coverage_complete"]
        del start["responsiveness_profile"][0]["decode_coverage_complete"]
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["slots"][0]["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "coverage_flag_absent")

    def _contained_pair(self, *, canceled_end=5, ended=False, publisher=True):
        """A publisher-timestamped span with a warmup-only cancel offset."""
        bounds = list(rm.LATENCY_BOUNDS_MS)
        # Warmup unmatched cancels do not consume edges on this slot:
        # edge = dispatch + (canceled - unmatched) + pending + lost + dropped
        # With unmatched==canceled and pending 0 → edge == dispatch.
        # Hist totals must equal latency_n == dispatch (native one-sample-per-dispatch).
        start_row = {
            "slot_id": 7, "generation": 1, "sample_age_ms": 0,
            "decode_edge_n": 100, "dispatch_n": 100,
            "decode_canceled_n": 5, "decode_unmatched_canceled_n": 5,
            "decode_lost_n": 0,
            "decode_dropped_n": 0, "decode_pending_n": 0,
            "decode_latency_n": 100,
            "decode_coverage_complete": False,
            "decode_latency_buckets": [0, 0, 0, 10, 90, 0, 0, 0, 0, 0, 0],
            "latency_bound_ms": bounds, "ended": ended,
        }
        end_row = dict(start_row)
        # matched in-span = canceled_end - 5; unmatched stays 5
        matched = canceled_end - 5
        end_dispatch = 300 - matched
        end_row.update({
            "decode_edge_n": 300, "dispatch_n": end_dispatch,
            "decode_canceled_n": canceled_end,
            "decode_unmatched_canceled_n": 5,
            "decode_latency_n": end_dispatch,
            "decode_latency_buckets": [0, 0, 0, 30, end_dispatch - 30, 0, 0, 0, 0, 0, 0],
        })
        if publisher:
            for row, elapsed in ((start_row, 40.5), (end_row, 160.0)):
                row["updated_ms"] = int((_meta()["started_unix"] + elapsed - 0.3) * 1000)
        return (
            {"phase": "observe", "elapsed_s": 40.5, "responsiveness_profile": [start_row]},
            {"phase": "observe", "elapsed_s": 160.0, "responsiveness_profile": [end_row]},
        )

    def test_decode_uses_contained_delta_not_lifetime_coverage_flag(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        g = rm.evaluate_decode(meta, [start, end], target_ms=100)
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_missing")

    def _mono_clock(self, elapsed_ns, sample_lo=None, sample_hi=None, read_lo=None, read_hi=None):
        """Build a valid mono clock: elapsed in sample; read ⊆ sample."""
        sample_lo = elapsed_ns if sample_lo is None else sample_lo
        sample_hi = elapsed_ns + 50_000 if sample_hi is None else sample_hi
        read_lo = elapsed_ns + 10_000 if read_lo is None else read_lo
        read_hi = elapsed_ns + 40_000 if read_hi is None else read_hi
        return {
            "domain": "responsiveness_process_mono",
            "elapsed_mono_ns": elapsed_ns,
            "sample_mono_ns_lower": sample_lo,
            "sample_mono_ns_upper": sample_hi,
            "read_mono_ns_lower": read_lo,
            "read_mono_ns_upper": read_hi,
        }

    def test_decode_mono_clock_contained_window_passes(self):
        """Native mono brackets unlock contained deltas (warmup cancel offset OK)."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        # first.elapsed_ns is observe_lb; mid/end captures must be ≥ that.
        e0, e1, e2 = 40_000_000_000, 70_000_000_000, 100_000_000_000
        mid_row = json.loads(json.dumps(start["responsiveness_profile"][0]))
        mid_row.update({
            "decode_edge_n": 200, "dispatch_n": 200, "decode_canceled_n": 5,
            "decode_unmatched_canceled_n": 5,
            "decode_latency_n": 200,
            "decode_latency_buckets": [0, 0, 0, 20, 180, 0, 0, 0, 0, 0, 0],
            # Capture after observe_lb, before read_hi of its sample.
            "decode_capture_mono_ns_lower": e0 + 1_000_000,
            "decode_capture_mono_ns_upper": e0 + 2_000_000,
            "updated_ms": int((_meta()["started_unix"] + 100.0 - 0.3) * 1000),
        })
        start["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": 10_000_000_000,  # before observe_lb
            "decode_capture_mono_ns_upper": 10_000_100_000,
        })
        end["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e1 + 1_000_000,
            "decode_capture_mono_ns_upper": e1 + 2_000_000,
        })
        # Delayed serialization: sample_hi >> elapsed (read still ⊆ sample).
        start["responsiveness_clock"] = self._mono_clock(
            e0, sample_lo=e0, sample_hi=e0 + 5_000_000,
            read_lo=e0 + 100_000, read_hi=e0 + 200_000,
        )
        # start capture 10e9 > read_hi of start? cap checked vs THIS sample's read.
        # start cap 10e9 << start read — OK for start row; excluded as start candidate.
        middle = {
            "phase": "observe", "elapsed_s": 100.0,
            "responsiveness_profile": [mid_row],
            "responsiveness_clock": self._mono_clock(
                e1, sample_lo=e1, sample_hi=e1 + 5_000_000,
                read_lo=e1 + 100_000, read_hi=e1 + 3_000_000,
            ),
        }
        end["responsiveness_clock"] = self._mono_clock(
            e2, sample_lo=e2, sample_hi=e2 + 5_000_000,
            read_lo=e2 + 100_000, read_hi=e2 + 3_000_000,
        )
        g = rm.evaluate_decode(meta, [start, middle, end], target_ms=100)
        self.assertEqual(g["status"], "available", g)
        slot = g["slots"][0]
        self.assertTrue(slot.get("contained_window"))
        self.assertEqual(slot["status"], "available")
        # Delta mid→end: edges 100, dispatch 100, cancel 0
        self.assertEqual(slot["counter_deltas"]["edge"], 100)
        self.assertEqual(slot["counter_deltas"]["canceled"], 0)
        self.assertEqual(slot["target_verdict"], "meet")

    def test_decode_fine_hist_emits_bounds_without_same_run_paired_claim(self):
        """Fine p99 sibling is available; same-run paired ≤2ms must NOT be claimed."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        fine_bounds = list(rm.FINE_LATENCY_BOUNDS_MS)
        # Conserved with coarse: start 10@≤25 + 90@≤40; end 30@≤25 + 270@≤40
        # Fine: 10 events at 25ms (idx 24), 90 at 40ms (idx 39) → start totals 100
        fine_s = [0] * (len(fine_bounds) + 1)
        fine_s[24] = 10
        fine_s[39] = 90
        fine_m = list(fine_s)
        fine_m[24] = 20
        fine_m[39] = 180
        fine_e = list(fine_s)
        fine_e[24] = 30
        fine_e[39] = 270
        e0, e1, e2 = 40_000_000_000, 70_000_000_000, 100_000_000_000
        start["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": 10_000_000_000,
            "decode_capture_mono_ns_upper": 10_000_100_000,
            "decode_fine_latency_buckets": list(fine_s),
            "fine_latency_bound_ms": fine_bounds,
        })
        start["responsiveness_clock"] = self._mono_clock(e0)
        mid = json.loads(json.dumps(start))
        mid["elapsed_s"] = 100.0
        mr = mid["responsiveness_profile"][0]
        mr.update({
            "decode_edge_n": 200, "dispatch_n": 200, "decode_latency_n": 200,
            "decode_latency_buckets": [0, 0, 0, 20, 180, 0, 0, 0, 0, 0, 0],
            "decode_fine_latency_buckets": fine_m,
            "decode_capture_mono_ns_lower": e0 + 1_000_000,
            "decode_capture_mono_ns_upper": e0 + 2_000_000,
            "updated_ms": int((_meta()["started_unix"] + 99.7) * 1000),
        })
        mid["responsiveness_clock"] = self._mono_clock(
            e1, sample_lo=e1, sample_hi=e1 + 5_000_000,
            read_lo=e1 + 100_000, read_hi=e1 + 3_000_000,
        )
        end["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e1 + 1_000_000,
            "decode_capture_mono_ns_upper": e1 + 2_000_000,
            "decode_fine_latency_buckets": fine_e,
            "fine_latency_bound_ms": fine_bounds,
        })
        end["responsiveness_clock"] = self._mono_clock(
            e2, sample_lo=e2, sample_hi=e2 + 5_000_000,
            read_lo=e2 + 100_000, read_hi=e2 + 3_000_000,
        )
        g = rm.evaluate_decode(meta, [start, mid, end], target_ms=100)
        self.assertEqual(g["status"], "available", g)
        slot = g["slots"][0]
        self.assertIn("fine_p99", slot)
        self.assertEqual(slot["fine_p99"]["status"], "available")
        self.assertNotIn("fine_paired_within_2ms", slot)
        self.assertNotIn("fine_paired_margin_ms", slot)

    def test_decode_inconsistent_fine_coarse_is_negative(self):
        """Former false-paired fixture: coarse 25–40 vs fine ~3ms must fail closed."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        fine_bounds = list(rm.FINE_LATENCY_BOUNDS_MS)
        fine_s = [0] * (len(fine_bounds) + 1)
        fine_e = list(fine_s)
        fine_s[2] = 10
        fine_e[2] = 110  # n=100 at ~3ms while coarse stays in 25–40 bins
        e0, e1, e2 = 40_000_000_000, 70_000_000_000, 100_000_000_000
        start["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": 10_000_000_000,
            "decode_capture_mono_ns_upper": 10_000_100_000,
            "decode_fine_latency_buckets": list(fine_s),
            "fine_latency_bound_ms": fine_bounds,
        })
        start["responsiveness_clock"] = self._mono_clock(e0)
        mid = json.loads(json.dumps(start))
        mid["elapsed_s"] = 100.0
        mr = mid["responsiveness_profile"][0]
        mr.update({
            "decode_edge_n": 200, "dispatch_n": 200, "decode_latency_n": 200,
            "decode_latency_buckets": [0, 0, 0, 20, 180, 0, 0, 0, 0, 0, 0],
            "decode_fine_latency_buckets": list(fine_s),
            "decode_capture_mono_ns_lower": e0 + 1_000_000,
            "decode_capture_mono_ns_upper": e0 + 2_000_000,
            "updated_ms": int((_meta()["started_unix"] + 99.7) * 1000),
        })
        mid["responsiveness_clock"] = self._mono_clock(
            e1, sample_lo=e1, sample_hi=e1 + 5_000_000,
            read_lo=e1 + 100_000, read_hi=e1 + 3_000_000,
        )
        end["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e1 + 1_000_000,
            "decode_capture_mono_ns_upper": e1 + 2_000_000,
            "decode_fine_latency_buckets": fine_e,
            "fine_latency_bound_ms": fine_bounds,
        })
        end["responsiveness_clock"] = self._mono_clock(
            e2, sample_lo=e2, sample_hi=e2 + 5_000_000,
            read_lo=e2 + 100_000, read_hi=e2 + 3_000_000,
        )
        g = rm.evaluate_decode(meta, [start, mid, end], target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertIn(
            g["slots"][0]["reason"],
            {
                "fine_coarse_rollup_mismatch",
                "fine_coarse_count_mismatch",
                "fine_histogram_latency_n_mismatch",
            },
            g["slots"][0],
        )

    def test_paired_fine_margin_is_diagnostic_only_never_eligibility_pass(self):
        """Arithmetic delta may be reported; paired eligibility stays unavailable."""
        cand = {
            "status": "available", "contained_window": True,
            "fine_p99": {"status": "available", "lower_ms": 2, "upper_ms": 3},
        }
        ref = {
            "status": "available", "contained_window": True,
            "fine_p99": {"status": "available", "lower_ms": 2, "upper_ms": 3},
        }
        p = rm.paired_fine_p99_margin(cand, ref)
        self.assertEqual(p["status"], "unavailable")
        self.assertEqual(p["reason"], "paired_matched_run_evidence_pending")
        self.assertEqual(p["diagnostic_margin_ms"], 1.0)
        self.assertNotIn("paired_within_margin", p)
        # Caller labels must not unlock a pass
        fake_runs = (
            {"run_id": "c1", "qualified": True, "frontend": "tui", "n": 1,
             "workload": "active", "renderer": "cpu",
             "responsiveness_profile": True, "responsiveness_fine": True},
            {"run_id": "r1", "qualified": True, "frontend": "tui", "n": 1,
             "workload": "active", "renderer": "cpu",
             "responsiveness_profile": True, "responsiveness_fine": True},
        )
        p2 = rm.paired_fine_p99_margin(
            cand, ref, candidate_run=fake_runs[0], reference_run=fake_runs[1]
        )
        self.assertEqual(p2["status"], "unavailable")
        self.assertEqual(p2["reason"], "paired_matched_run_evidence_pending")
        self.assertNotIn("paired_within_margin", p2)
        # NaN / negative / inverted bounds rejected (still no pass)
        bad_nan = {
            "status": "available", "contained_window": True,
            "fine_p99": {"status": "available", "lower_ms": float("nan"), "upper_ms": 3},
        }
        self.assertEqual(
            rm.paired_fine_p99_margin(bad_nan, ref)["reason"],
            "paired_fine_bounds_malformed",
        )
        bad_neg = {
            "status": "available", "contained_window": True,
            "fine_p99": {"status": "available", "lower_ms": -1, "upper_ms": 3},
        }
        self.assertEqual(
            rm.paired_fine_p99_margin(bad_neg, ref)["reason"],
            "paired_fine_bounds_malformed",
        )
        bad_inv = {
            "status": "available", "contained_window": True,
            "fine_p99": {"status": "available", "lower_ms": 5, "upper_ms": 3},
        }
        self.assertEqual(
            rm.paired_fine_p99_margin(bad_inv, ref)["reason"],
            "paired_fine_bounds_inverted",
        )
        # -inf must not yield a true eligibility pass
        bad_ninf = {
            "status": "available", "contained_window": True,
            "fine_p99": {
                "status": "available",
                "lower_ms": float("-inf"),
                "upper_ms": 3,
            },
        }
        pn = rm.paired_fine_p99_margin(cand, bad_ninf)
        self.assertEqual(pn["status"], "unavailable")
        self.assertNotIn("paired_within_margin", pn)

    def test_interior_ended_row_rejects_mono_window(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        e0, e1, e2 = 40_000_000_000, 70_000_000_000, 100_000_000_000
        # Start is a valid candidate (cap after observe_lb) so mid is truly interior.
        start["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e0 + 100_000,
            "decode_capture_mono_ns_upper": e0 + 200_000,
        })
        start["responsiveness_clock"] = self._mono_clock(
            e0, sample_lo=e0, sample_hi=e0 + 5_000_000,
            read_lo=e0 + 100_000, read_hi=e0 + 3_000_000,
        )
        end["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e1 + 1_000_000,
            "decode_capture_mono_ns_upper": e1 + 2_000_000,
        })
        end["responsiveness_clock"] = self._mono_clock(
            e2, sample_lo=e2, sample_hi=e2 + 5_000_000,
            read_lo=e2 + 100_000, read_hi=e2 + 3_000_000,
        )
        mid = json.loads(json.dumps(start))
        mid["elapsed_s"] = 100.0
        mr = mid["responsiveness_profile"][0]
        mr.update({
            "ended": True,
            "decode_edge_n": 200, "dispatch_n": 200,
            "decode_capture_mono_ns_lower": e0 + 1_000_000,
            "decode_capture_mono_ns_upper": e0 + 2_000_000,
            "updated_ms": int((_meta()["started_unix"] + 99.7) * 1000),
        })
        mid["responsiveness_clock"] = self._mono_clock(
            e1, sample_lo=e1, sample_hi=e1 + 5_000_000,
            read_lo=e1 + 100_000, read_hi=e1 + 3_000_000,
        )
        g = rm.evaluate_decode(meta, [start, mid, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "interior_ended")

    def test_pending_gauge_mid_span_does_not_false_reject(self):
        """pending 0→1→0 is a gauge swing, not a counter reset."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        e0, e1, e2 = 40_000_000_000, 70_000_000_000, 100_000_000_000
        # Start is a valid candidate so mid stays interior with pending=1.
        start["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e0 + 100_000,
            "decode_capture_mono_ns_upper": e0 + 200_000,
        })
        start["responsiveness_clock"] = self._mono_clock(
            e0, sample_lo=e0, sample_hi=e0 + 5_000_000,
            read_lo=e0 + 100_000, read_hi=e0 + 3_000_000,
        )
        mid = json.loads(json.dumps(start))
        mid["elapsed_s"] = 100.0
        mr = mid["responsiveness_profile"][0]
        # One edge in flight: edge 201 = dispatch 200 + unmatched0 matched + pending 1
        # cancel 5 unmatched 5 → 200 + 0 + 1 = 201
        mr.update({
            "decode_edge_n": 201, "dispatch_n": 200, "decode_canceled_n": 5,
            "decode_unmatched_canceled_n": 5, "decode_pending_n": 1,
            "decode_latency_n": 200,
            "decode_latency_buckets": [0, 0, 0, 20, 180, 0, 0, 0, 0, 0, 0],
            "decode_capture_mono_ns_lower": e0 + 1_000_000,
            "decode_capture_mono_ns_upper": e0 + 2_000_000,
            "updated_ms": int((_meta()["started_unix"] + 99.7) * 1000),
        })
        mid["responsiveness_clock"] = self._mono_clock(
            e1, sample_lo=e1, sample_hi=e1 + 5_000_000,
            read_lo=e1 + 100_000, read_hi=e1 + 3_000_000,
        )
        end["responsiveness_profile"][0].update({
            "decode_edge_n": 300, "dispatch_n": 300, "decode_canceled_n": 5,
            "decode_unmatched_canceled_n": 5, "decode_pending_n": 0,
            "decode_latency_n": 300,
            "decode_latency_buckets": [0, 0, 0, 30, 270, 0, 0, 0, 0, 0, 0],
            "decode_capture_mono_ns_lower": e1 + 1_000_000,
            "decode_capture_mono_ns_upper": e1 + 2_000_000,
        })
        end["responsiveness_clock"] = self._mono_clock(
            e2, sample_lo=e2, sample_hi=e2 + 5_000_000,
            read_lo=e2 + 100_000, read_hi=e2 + 3_000_000,
        )
        g = rm.evaluate_decode(meta, [start, mid, end], target_ms=100)
        self.assertEqual(g["status"], "available", g)
        self.assertEqual(g["slots"][0]["counter_deltas"]["pending"], 0)

    def test_decode_cancellation_inside_contained_span_rejected(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair(canceled_end=6)
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "coverage_lost_or_incomplete")

    def test_decode_missing_publisher_timestamp_unavailable(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair(publisher=False)
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "publisher_timestamp_missing")

    def test_decode_middle_cancellation_cannot_be_hidden_by_inner_segment(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        middle = json.loads(json.dumps(start))
        middle["elapsed_s"] = 100.0
        row = middle["responsiveness_profile"][0]
        row["updated_ms"] = int((meta["started_unix"] + 99.7) * 1000)
        # Matched cancels in middle; unmatched stays warmup-only 5.
        row["decode_edge_n"] = 200
        row["decode_canceled_n"] = 11
        row["decode_unmatched_canceled_n"] = 5
        row["dispatch_n"] = 200 - (11 - 5)  # 194
        end["responsiveness_profile"][0].update({
            "decode_edge_n": 302,
            "decode_canceled_n": 12,
            "decode_unmatched_canceled_n": 5,
            "dispatch_n": 302 - (12 - 5),  # 295
            "updated_ms": int((meta["started_unix"] + 159.7) * 1000),
        })
        g = rm.evaluate_decode(meta, [start, middle, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "coverage_lost_or_incomplete")

    def test_observation_bound_not_warmup_duration(self):
        meta = _meta(responsiveness_profile=True, warmup_s=30)
        start, end = self._contained_pair()
        start["elapsed_s"] = 29.0
        start["responsiveness_profile"][0]["updated_ms"] = int((meta["started_unix"] + 28.7) * 1000)
        g = rm.evaluate_decode(meta, [start, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_missing")

    def test_interior_counter_reset_cannot_recover_beyond_start(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        middle = json.loads(json.dumps(start))
        middle["elapsed_s"] = 100.0
        row = middle["responsiveness_profile"][0]
        row["updated_ms"] = int((meta["started_unix"] + 99.7) * 1000)
        row["decode_edge_n"] = 10
        row["dispatch_n"] = 5
        row["decode_canceled_n"] = 5  # 10-5-5=0 pending
        g = rm.evaluate_decode(meta, [start, middle, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "counter_reset")

    def test_interior_missing_publisher_row_rejects_whole_window(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        middle = {"phase": "observe", "elapsed_s": 100.0, "responsiveness_profile": []}
        g = rm.evaluate_decode(meta, [start, middle, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "publisher_slot_row_missing_or_duplicate")

    def test_interior_duplicate_publisher_row_rejects_whole_window(self):
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        middle = json.loads(json.dumps(start))
        middle["elapsed_s"] = 100.0
        row = middle["responsiveness_profile"][0]
        row["updated_ms"] = int((meta["started_unix"] + 99.7) * 1000)
        middle["responsiveness_profile"].append(json.loads(json.dumps(row)))
        g = rm.evaluate_decode(meta, [start, middle, end])
        self.assertEqual(g["status"], "unavailable")
        self.assertEqual(g["slots"][0]["reason"], "publisher_slot_row_missing_or_duplicate")

    def test_input_interior_histogram_reset_rejects_even_when_counters_recover(self):
        """Orch false-pass: input hist 200→5→300 with counters climbing must be unavailable."""
        meta = _meta(responsiveness_profile=True)
        bounds = list(rm.LATENCY_BOUNDS_MS)
        e0, e1, e2, e3 = 40e9, 70e9, 85e9, 100e9

        def row(start_n, buckets, cap_lo, cap_hi, age_elapsed):
            return {
                "slot_id": 7, "generation": 1, "sample_age_ms": 0,
                "decode_edge_n": start_n, "dispatch_n": start_n,
                "decode_canceled_n": 5, "decode_unmatched_canceled_n": 5,
                "decode_lost_n": 0, "decode_dropped_n": 0, "decode_pending_n": 0,
                "decode_latency_n": start_n,
                "decode_latency_buckets": [0, 0, 0, 0, start_n, 0, 0, 0, 0, 0, 0],
                "decode_coverage_complete": False,
                "latency_bound_ms": bounds, "ended": False,
                "updated_ms": int((_meta()["started_unix"] + age_elapsed - 0.3) * 1000),
                "decode_capture_mono_ns_lower": int(cap_lo),
                "decode_capture_mono_ns_upper": int(cap_hi),
                "input_start_n": start_n, "input_complete_n": start_n,
                "input_canceled_n": 0, "input_lost_n": 0, "input_dropped_n": 0,
                "input_pending_n": 0, "input_latency_n": start_n,
                "input_latency_buckets": buckets,
                "input_coverage_complete": True,
                "input_surface": "tui",
                "visible_ack": {"available": True},
                "input_capture_mono_ns_lower": int(cap_lo),
                "input_capture_mono_ns_upper": int(cap_hi),
            }

        samples = []
        for elapsed, en, buckets, clo, chi in (
            (40.5, 100, [100] + [0] * 10, 10e9, 10e9 + 1e5),
            (100.0, 200, [200] + [0] * 10, e0 + 1e6, e0 + 2e6),
            (130.0, 250, [5] + [0] * 10, 60e9, 60e9 + 1e5),  # hist reset
            (160.0, 300, [300] + [0] * 10, e1 + 1e6, e1 + 2e6),
        ):
            samples.append({
                "phase": "observe", "elapsed_s": elapsed,
                "responsiveness_profile": [row(en, buckets, clo, chi, elapsed)],
                "responsiveness_clock": self._mono_clock(
                    int(en * 1e0 + elapsed * 1e9),  # unique-ish
                    sample_lo=int(elapsed * 1e9),
                    sample_hi=int(elapsed * 1e9) + 5_000_000,
                    read_lo=int(elapsed * 1e9) + 100_000,
                    read_hi=int(elapsed * 1e9) + 3_000_000,
                ),
            })
        # Fix elapsed_mono to the intended e0..e3 series
        for s, en in zip(samples, (e0, e1, e2, e3)):
            s["responsiveness_clock"] = self._mono_clock(
                int(en), sample_lo=int(en), sample_hi=int(en) + 5_000_000,
                read_lo=int(en) + 100_000, read_hi=int(en) + 3_000_000,
            )
        # First row capture before observe_lb so window starts at sample1
        samples[0]["responsiveness_profile"][0]["input_capture_mono_ns_lower"] = 10_000_000_000
        samples[0]["responsiveness_profile"][0]["input_capture_mono_ns_upper"] = 10_000_100_000
        samples[0]["responsiveness_profile"][0]["decode_capture_mono_ns_lower"] = 10_000_000_000
        samples[0]["responsiveness_profile"][0]["decode_capture_mono_ns_upper"] = 10_000_100_000
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(g["slots"][0]["reason"], "counter_reset", g["slots"][0])

    def test_input_interior_hist_conservation_mismatch_rejects_maximal_span(self):
        """Orch b157df7 hole: monotonic middle hist 201 vs complete/latency_n 250.

        Endpoints conserve; interior only was monotonic. Must stay unavailable
        without shrinking the maximal contained window around the bad middle.
        """
        path = (
            ROOT
            / "diagnostics"
            / "clock-adapter-review-20260907T022630Z"
            / "input-interior-conservation-mismatch.json"
        )
        data = json.loads(path.read_text())
        g = rm.evaluate_input(data["meta"], data["samples"], target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(
            g["slots"][0]["reason"],
            "histogram_latency_n_mismatch",
            g["slots"][0],
        )
        self.assertNotEqual(g["slots"][0].get("target_verdict"), "meet")
        self.assertNotEqual(g["slots"][0].get("contained_window"), True)

    def test_decode_interior_hist_conservation_mismatch_rejects(self):
        """Decode sibling: middle hist under-counts vs dispatch/latency_n."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        e0, e1, e2, e3 = 40e9, 70e9, 85e9, 100e9
        bounds = list(rm.LATENCY_BOUNDS_MS)

        def stamp(row, clo, chi):
            row["decode_capture_mono_ns_lower"] = int(clo)
            row["decode_capture_mono_ns_upper"] = int(chi)
            row["latency_bound_ms"] = bounds

        start_row = start["responsiveness_profile"][0]
        stamp(start_row, 10e9, 10e9 + 1e5)
        mid = json.loads(json.dumps(start))
        mid["elapsed_s"] = 100.0
        mr = mid["responsiveness_profile"][0]
        # Monotonic hist 200→201→300 but dispatch/latency_n 200→250→300.
        mr.update({
            "decode_edge_n": 250, "dispatch_n": 250, "decode_latency_n": 250,
            "decode_latency_buckets": [0, 0, 0, 20, 181, 0, 0, 0, 0, 0, 0],  # sum 201
            "decode_canceled_n": 5, "decode_unmatched_canceled_n": 5,
            "updated_ms": int((_meta()["started_unix"] + 99.7) * 1000),
        })
        stamp(mr, e0 + 1e6, e0 + 2e6)
        end_row = end["responsiveness_profile"][0]
        end_row.update({
            "decode_edge_n": 300, "dispatch_n": 300, "decode_latency_n": 300,
            "decode_latency_buckets": [0, 0, 0, 30, 270, 0, 0, 0, 0, 0, 0],
            "decode_canceled_n": 5, "decode_unmatched_canceled_n": 5,
        })
        stamp(end_row, e1 + 1e6, e1 + 2e6)
        # Extra interior after mid so maximal span includes the bad middle.
        mid2 = json.loads(json.dumps(end))
        mid2["elapsed_s"] = 130.0
        m2 = mid2["responsiveness_profile"][0]
        m2.update({
            "decode_edge_n": 280, "dispatch_n": 280, "decode_latency_n": 280,
            "decode_latency_buckets": [0, 0, 0, 28, 252, 0, 0, 0, 0, 0, 0],
            "updated_ms": int((_meta()["started_unix"] + 129.7) * 1000),
        })
        stamp(m2, 60e9, 60e9 + 1e5)
        samples = [start, mid, mid2, end]
        for s, en in zip(samples, (e0, e1, e2, e3)):
            s["responsiveness_clock"] = self._mono_clock(
                int(en), sample_lo=int(en), sample_hi=int(en) + 5_000_000,
                read_lo=int(en) + 100_000, read_hi=int(en) + 3_000_000,
            )
        g = rm.evaluate_decode(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(
            g["slots"][0]["reason"],
            "histogram_latency_n_mismatch",
            g["slots"][0],
        )

    def test_input_missing_required_latency_n_rejects(self):
        """Native always emits input_latency_n under the profile — missing rejects."""
        meta = _meta(responsiveness_profile=True)
        bounds = list(rm.LATENCY_BOUNDS_MS)
        e0, e1 = 40e9, 100e9

        def row(n, *, drop_lat=False):
            r = {
                "slot_id": 7, "generation": 1, "sample_age_ms": 0,
                "decode_edge_n": n, "dispatch_n": n,
                "decode_canceled_n": 5, "decode_unmatched_canceled_n": 5,
                "decode_lost_n": 0, "decode_dropped_n": 0, "decode_pending_n": 0,
                "decode_latency_n": n,
                "decode_latency_buckets": [0, 0, 0, 0, n, 0, 0, 0, 0, 0, 0],
                "decode_coverage_complete": False,
                "latency_bound_ms": bounds, "ended": False,
                "updated_ms": 0,
                "decode_capture_mono_ns_lower": int(e0 + 1e6 if n > 100 else 10e9),
                "decode_capture_mono_ns_upper": int(e0 + 2e6 if n > 100 else 10e9 + 1e5),
                "input_start_n": n, "input_complete_n": n,
                "input_canceled_n": 0, "input_lost_n": 0, "input_dropped_n": 0,
                "input_pending_n": 0,
                "input_latency_buckets": [n] + [0] * 10,
                "input_coverage_complete": True,
                "input_surface": "tui",
                "visible_ack": {"available": True},
                "input_capture_mono_ns_lower": int(e0 + 1e6 if n > 100 else 10e9),
                "input_capture_mono_ns_upper": int(e0 + 2e6 if n > 100 else 10e9 + 1e5),
            }
            if not drop_lat:
                r["input_latency_n"] = n
            return r

        start = {
            "phase": "observe", "elapsed_s": 40.5,
            "responsiveness_profile": [row(100)],
            "responsiveness_clock": self._mono_clock(
                int(e0), sample_lo=int(e0), sample_hi=int(e0) + 5_000_000,
                read_lo=int(e0) + 100_000, read_hi=int(e0) + 3_000_000,
            ),
        }
        end = {
            "phase": "observe", "elapsed_s": 160.0,
            "responsiveness_profile": [row(300, drop_lat=True)],
            "responsiveness_clock": self._mono_clock(
                int(e1), sample_lo=int(e1), sample_hi=int(e1) + 5_000_000,
                read_lo=int(e1) + 100_000, read_hi=int(e1) + 3_000_000,
            ),
        }
        # Start capture before observe_lb so selected span is end-only? Need two
        # selected rows: put start capture inside observe window too.
        start["responsiveness_profile"][0]["input_capture_mono_ns_lower"] = int(e0 + 1e6)
        start["responsiveness_profile"][0]["input_capture_mono_ns_upper"] = int(e0 + 2e6)
        start["responsiveness_profile"][0]["decode_capture_mono_ns_lower"] = int(e0 + 1e6)
        start["responsiveness_profile"][0]["decode_capture_mono_ns_upper"] = int(e0 + 2e6)
        end["responsiveness_profile"][0]["input_capture_mono_ns_lower"] = int(e0 + 3e6)
        end["responsiveness_profile"][0]["input_capture_mono_ns_upper"] = int(e0 + 4e6)
        g = rm.evaluate_input(meta, [start, end], target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(
            g["slots"][0]["reason"],
            "latency_counter_incomplete",
            g["slots"][0],
        )

    def test_exact_int_bounds_tuple_rejects_lossy_and_nonfinite(self):
        """Bounds schema: no lossy int() coercion; bool/frac/inf/NaN reject."""
        exp = rm.LATENCY_BOUNDS_MS
        self.assertTrue(rm._exact_int_bounds_tuple(list(exp), exp))
        self.assertTrue(rm._exact_int_bounds_tuple(tuple(float(x) for x in exp), exp))
        bad_frac = list(exp)
        bad_frac[0] = 5.9
        self.assertFalse(rm._exact_int_bounds_tuple(bad_frac, exp))
        bad_bool = list(exp)
        bad_bool[0] = True  # would become 1 under int()
        self.assertFalse(rm._exact_int_bounds_tuple(bad_bool, exp))
        bad_inf = list(exp)
        bad_inf[0] = float("inf")
        self.assertFalse(rm._exact_int_bounds_tuple(bad_inf, exp))
        bad_nan = list(exp)
        bad_nan[0] = float("nan")
        self.assertFalse(rm._exact_int_bounds_tuple(bad_nan, exp))
        bad_neg = list(exp)
        bad_neg[0] = -5
        self.assertFalse(rm._exact_int_bounds_tuple(bad_neg, exp))
        self.assertFalse(rm._exact_int_bounds_tuple(list(exp)[:-1], exp))
        mutated = list(exp)
        mutated[3] = 26  # was 25
        self.assertFalse(rm._exact_int_bounds_tuple(mutated, exp))

    def test_decode_middle_fractional_bounds_reject(self):
        """Mutated middle latency_bound_ms with fractional edge fails closed."""
        meta = _meta(responsiveness_profile=True)
        start, end = self._contained_pair()
        e0, e1, e2 = 40e9, 70e9, 100e9
        start["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": 10_000_000_000,
            "decode_capture_mono_ns_upper": 10_000_100_000,
        })
        mid = json.loads(json.dumps(start))
        mid["elapsed_s"] = 100.0
        mr = mid["responsiveness_profile"][0]
        bad_bounds = list(rm.LATENCY_BOUNDS_MS)
        bad_bounds[0] = 5.9
        mr.update({
            "decode_edge_n": 200, "dispatch_n": 200, "decode_latency_n": 200,
            "decode_latency_buckets": [0, 0, 0, 20, 180, 0, 0, 0, 0, 0, 0],
            "latency_bound_ms": bad_bounds,
            "decode_capture_mono_ns_lower": e0 + 1_000_000,
            "decode_capture_mono_ns_upper": e0 + 2_000_000,
            "updated_ms": int((_meta()["started_unix"] + 99.7) * 1000),
        })
        end["responsiveness_profile"][0].update({
            "decode_capture_mono_ns_lower": e1 + 1_000_000,
            "decode_capture_mono_ns_upper": e1 + 2_000_000,
        })
        start["responsiveness_clock"] = self._mono_clock(int(e0))
        mid["responsiveness_clock"] = self._mono_clock(
            int(e1), sample_lo=int(e1), sample_hi=int(e1) + 5_000_000,
            read_lo=int(e1) + 100_000, read_hi=int(e1) + 3_000_000,
        )
        end["responsiveness_clock"] = self._mono_clock(
            int(e2), sample_lo=int(e2), sample_hi=int(e2) + 5_000_000,
            read_lo=int(e2) + 100_000, read_hi=int(e2) + 3_000_000,
        )
        g = rm.evaluate_decode(meta, [start, mid, end], target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(
            g["slots"][0]["reason"],
            "latency_bounds_schema_mismatch",
            g["slots"][0],
        )


class ObservationWindowTests(unittest.TestCase):
    def test_boundaries_and_contamination_intersection(self):
        meta = _meta(started_unix=1_000_000.0)
        samples = [
            {"phase": "warmup", "elapsed_s": 10.0},
            {"phase": "observe", "elapsed_s": 30.0},
            {"phase": "observe", "elapsed_s": 150.0},
            {"phase": "teardown", "elapsed_s": 200.0},
        ]
        w = rm.observe_window(meta, samples, contamination_from=1_000_100.0)
        self.assertEqual(w["status"], "available")
        self.assertEqual(w["obs_start_unix"], 1_000_030.0)
        self.assertEqual(w["obs_end_unix"], 1_000_150.0)
        self.assertTrue(w["contaminated"])
        self.assertEqual(w["contamination_intersection"]["start_unix"], 1_000_100.0)
        self.assertEqual(w["contamination_intersection"]["end_unix"], 1_000_150.0)

    def test_no_contamination_when_before_window_end(self):
        meta = _meta(started_unix=1_000_000.0)
        samples = [
            {"phase": "observe", "elapsed_s": 30.0},
            {"phase": "observe", "elapsed_s": 50.0},
        ]
        w = rm.observe_window(meta, samples, contamination_from=1_000_200.0)
        self.assertFalse(w["contaminated"])
        self.assertIsNone(w["contamination_intersection"])

    def test_no_observe_samples(self):
        w = rm.observe_window(_meta(), [{"phase": "warmup", "elapsed_s": 1.0}])
        self.assertEqual(w["reason"], "no_observe_samples")


class CliAndRealCellTests(unittest.TestCase):
    def test_inspect_exit_0_partial(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td)
            (run / "metadata.json").write_text(json.dumps(_meta()))
            _write_jsonl(
                run / "samples.jsonl",
                [
                    {"phase": "observe", "elapsed_s": 1.0, "scheduling": None},
                    {"phase": "observe", "elapsed_s": 2.0, "scheduling": None},
                ],
            )
            proc = subprocess.run(
                [sys.executable, str(CLI), str(run), "--inspect"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            data = json.loads(proc.stdout)
            self.assertFalse(data["final_acceptance_claim"])
            self.assertEqual(data["gates"]["scheduling"]["status"], "unavailable")

    def test_require_exit_1_when_unavailable(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td)
            (run / "metadata.json").write_text(json.dumps(_meta()))
            _write_jsonl(
                run / "samples.jsonl",
                [
                    {"phase": "observe", "elapsed_s": 1.0},
                    {"phase": "observe", "elapsed_s": 2.0},
                ],
            )
            proc = subprocess.run(
                [
                    sys.executable,
                    str(CLI),
                    str(run),
                    "--require",
                    "scheduling,decode,input,gpu",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 1)
            self.assertIn("scheduling", proc.stderr)

    def test_real_flags_off_require_scheduling_and_responsiveness_fail(self):
        self.assertTrue(
            REAL_FLAGS_OFF.is_dir(),
            f"missing reviewed reference cell {REAL_FLAGS_OFF}",
        )
        proc = subprocess.run(
            [
                sys.executable,
                str(CLI),
                str(REAL_FLAGS_OFF),
                "--inspect",
                "--contamination-from",
                CONTAM_UTC,
                "--require",
                "scheduling,decode,input,gpu",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(proc.returncode, 1, proc.stderr)
        data = json.loads(proc.stdout)
        self.assertFalse(data["final_acceptance_claim"])
        meta_f = data["metadata_flags"]
        self.assertFalse(meta_f["scheduling_profile"])
        self.assertFalse(meta_f["responsiveness_profile"])
        self.assertFalse(meta_f["gpu_completion_profile"])
        for name in ("scheduling", "decode", "input", "gpu"):
            self.assertEqual(
                data["gates"][name]["status"],
                "unavailable",
                msg=f"{name}: {data['gates'][name]}",
            )
        # scheduling + decode (responsiveness) required gates fail
        self.assertEqual(data["gates"]["scheduling"]["reason"], "profile_disabled")
        self.assertEqual(data["gates"]["decode"]["reason"], "profile_disabled")
        self.assertEqual(data["gates"]["input"]["reason"], "profile_disabled")
        # observation window computed from real cell
        self.assertEqual(data["observation_window"]["status"], "available")
        self.assertGreater(data["observation_window"]["observe_sample_n"], 0)

    def test_require_unproven_target_exit_1(self):
        """Available p99 that straddles target is unproven → require fails."""
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td)
            meta = _meta(responsiveness_profile=True)
            (run / "metadata.json").write_text(json.dumps(meta))
            buckets_s = [0] * 11
            buckets_e = [0] * 11
            # ≤250ms bucket (index 7): lower=100 upper=250 vs default decode target 100 → unproven
            buckets_e[7] = 100
            row_s = {
                "slot_id": 1,
                "generation": 1,
                "sample_age_ms": 3,
                "decode_latency_buckets": buckets_s,
                "decode_dropped_n": 0,
                "decode_canceled_n": 0,
                "decode_lost_n": 0,
                "decode_pending_n": 0,
                "decode_coverage_complete": True,
                "latency_bound_ms": list(rm.LATENCY_BOUNDS_MS),
            }
            row_e = dict(row_s)
            row_e["decode_latency_buckets"] = buckets_e
            _write_jsonl(
                run / "samples.jsonl",
                [
                    {"phase": "observe", "elapsed_s": 1.0, "responsiveness_profile": [row_s]},
                    {"phase": "observe", "elapsed_s": 2.0, "responsiveness_profile": [row_e]},
                ],
            )
            proc = subprocess.run(
                [sys.executable, str(CLI), str(run), "--require", "decode"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 1)
            data = json.loads(proc.stdout)
            self.assertEqual(data["gates"]["decode"]["status"], "available")
            self.assertEqual(data["gates"]["decode"]["target_verdict"], "unproven")
            self.assertEqual(data["gates"]["decode"]["slots"][0]["p99"]["lower_ms"], 100)
            self.assertEqual(data["gates"]["decode"]["slots"][0]["p99"]["upper_ms"], 250)

    def test_output_rejects_existing_path(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td) / "run"
            run.mkdir()
            (run / "metadata.json").write_text(json.dumps(_meta()))
            _write_jsonl(
                run / "samples.jsonl",
                [
                    {"phase": "observe", "elapsed_s": 1.0},
                    {"phase": "observe", "elapsed_s": 2.0},
                ],
            )
            out = pathlib.Path(td) / "out.json"
            out.write_text("sentinel\n")
            proc = subprocess.run(
                [sys.executable, str(CLI), str(run), "--inspect", "-o", str(out)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(proc.returncode, 0)
            self.assertEqual(out.read_text(), "sentinel\n")
            self.assertIn("exist", proc.stderr.lower())

    def test_output_exclusive_create(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td) / "run"
            run.mkdir()
            (run / "metadata.json").write_text(json.dumps(_meta()))
            _write_jsonl(
                run / "samples.jsonl",
                [
                    {"phase": "observe", "elapsed_s": 1.0},
                    {"phase": "observe", "elapsed_s": 2.0},
                ],
            )
            out = pathlib.Path(td) / "out.json"
            proc = subprocess.run(
                [sys.executable, str(CLI), str(run), "--inspect", "-o", str(out)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            data = json.loads(out.read_text())
            self.assertFalse(data["final_acceptance_claim"])


class NoInputAndEmptyRunTests(unittest.TestCase):
    def test_missing_run_dir(self):
        r = rm.analyze_run(pathlib.Path("/nonexistent/path/for/reference_metrics"))
        self.assertIn("error", r)
        self.assertEqual(r["gates"]["scheduling"]["status"], "unavailable")


class ResourceAdapterTests(unittest.TestCase):
    def _samples(self, **changes):
        rows = []
        for t, user, system, rss, peak in ((0.0, 1.0, 0.5, 100, 120),
                                            (5.0, 2.0, 1.0, 140, 160),
                                            (10.0, 3.0, 1.5, 120, 180)):
            row = {"phase": "observe", "elapsed_s": t,
                   "process_cpu_user_s": user, "process_cpu_system_s": system,
                   "resident_bytes": rss, "peak_resident_bytes": peak}
            row.update(changes)
            rows.append(row)
        return rows

    def test_cpu_is_process_core_delta_over_sampled_wall_and_rss_is_current(self):
        result = rm.evaluate_resources(_meta(), self._samples(),
                                       workload_qualification={"qualified": True})
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["observation_s"], 10.0)
        self.assertEqual(result["cpu_seconds"], 3.0)
        self.assertEqual(result["cpu_cores"], 0.3)
        self.assertEqual(result["resident_median_bytes"], 120)
        self.assertEqual(result["resident_max_bytes"], 140)
        self.assertEqual(result["peak_resident_bytes"], 180)
        self.assertEqual(result["cpu_units"], "process_cpu_seconds / sampled_monotonic_wall_seconds")

    def test_missing_nan_and_reset_cpu_fail_closed(self):
        for index, changes, reason in ((None, {"process_cpu_system_s": None}, "invalid_cpu_counter"),
                                       (None, {"process_cpu_user_s": float("nan")}, "invalid_cpu_counter"),
                                       (1, {"process_cpu_user_s": -1.0}, "cpu_counter_reset")):
            rows = self._samples()
            if index is None:
                for row in rows:
                    row.update(changes)
            else:
                rows[index].update(changes)
            result = rm.evaluate_resources(_meta(), rows,
                                           workload_qualification={"qualified": True})
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(result["reason"], reason)

    def test_wrong_mode_and_unqualified_workload_never_pass(self):
        result = rm.evaluate_resources(_meta(frontend="panel", n=1, render_policy="none"),
                                        self._samples(), workload_qualification={"qualified": True})
        self.assertEqual(result["reason"], "unsupported_panel_render_policy")
        result = rm.evaluate_resources(_meta(), self._samples(),
                                       workload_qualification={"qualified": False})
        self.assertEqual(result["reason"], "workload_not_qualified")

    def test_budget_miss_is_honest_and_single_pair_is_not_significance(self):
        rows = self._samples()
        for row in rows:
            row["resident_bytes"] += 400 * 1024 * 1024
        result = rm.evaluate_resources(_meta(), rows,
                                       workload_qualification={"qualified": True})
        self.assertEqual(result["target_verdict"], "unavailable")
        self.assertEqual(result["metrics"]["median_rss"]["target_verdict"], "miss")
        self.assertFalse(result["accepted_saving"])
        result["status"] = "available"  # direct comparison fixture only
        result["overhead"] = "measured"
        pair = rm.compare_matched_runs(result, result)
        self.assertEqual(pair["status"], "inconclusive")
        self.assertIn("single", pair["reason"])

    def test_matched_pair_rejects_mismatch_contamination_and_unknown_overhead(self):
        base = rm.evaluate_resources(_meta(), self._samples(),
                                     workload_qualification={"qualified": True})
        base["status"] = "available"  # direct comparison fixture only
        other = dict(base)
        other["match_metadata"] = dict(base["match_metadata"])
        other["match_metadata"]["frontend"] = "panel"
        base["match_metadata"] = dict(base["match_metadata"])
        base["match_metadata"]["frontend"] = "tui"
        mismatch = rm.compare_matched_runs(other, base)
        self.assertEqual(mismatch["status"], "inconclusive")
        self.assertEqual(mismatch["reason"], "mismatched_provenance_or_settings")
        base["contaminated"] = True
        contamination = rm.compare_matched_runs(base, base)
        self.assertEqual(contamination["status"], "inconclusive")
        self.assertEqual(contamination["reason"], "contaminated_matched_run")
        base["contaminated"] = False
        base["overhead"] = "measured"
        candidate_unknown = dict(base)
        candidate_unknown["overhead"] = "unknown"
        candidate_result = rm.compare_matched_runs(candidate_unknown, base)
        self.assertEqual(candidate_result["status"], "inconclusive")
        self.assertEqual(candidate_result["reason"], "overhead_unknown")
        reference_unknown = dict(base)
        reference_unknown["overhead"] = "unknown"
        reference_result = rm.compare_matched_runs(base, reference_unknown)
        self.assertEqual(reference_result["status"], "inconclusive")
        self.assertEqual(reference_result["reason"], "overhead_unknown")
        self.assertNotIn("intended_rss_change_bytes", reference_result)
        self.assertNotIn("cpu_non_regression", reference_result)
        missing_metadata = dict(base)
        missing_metadata["match_metadata"] = dict(base["match_metadata"])
        missing_metadata["match_metadata"].pop("catalog_sha256")
        missing_result = rm.compare_matched_runs(missing_metadata, base)
        self.assertEqual(missing_result["status"], "inconclusive")
        self.assertEqual(missing_result["reason"], "missing_match_provenance")

    def test_resource_gate_rejects_negative_values_idle_and_bad_wall_samples(self):
        for field, reason in (("resident_bytes", "negative_resident_rss"),
                              ("peak_resident_bytes", "negative_peak_rss"),
                              ("process_cpu_user_s", "negative_cpu_counter")):
            rows = self._samples()
            if field == "process_cpu_user_s":
                for row in rows:
                    row[field] = -1
            else:
                rows[1][field] = -1
            result = rm.evaluate_resources(_meta(), rows,
                                           workload_qualification={"qualified": True})
            self.assertEqual(result["reason"], reason)

        idle = rm.evaluate_resources(_meta(n=16, workload="idle"), self._samples(),
                                     workload_qualification={"qualified": True})
        self.assertEqual(idle["reason"], "unsupported_resource_workload")
        rows = self._samples()
        rows[1]["elapsed_s"] = rows[0]["elapsed_s"]
        result = rm.evaluate_resources(_meta(), rows,
                                       workload_qualification={"qualified": True})
        self.assertEqual(result["reason"], "non_increasing_observation_wall_time")

    def test_resource_gate_requires_overhead_and_full_provenance(self):
        missing = _meta()
        missing.pop("catalog_sha256")
        result = rm.evaluate_resources(missing, self._samples(),
                                       workload_qualification={"qualified": True})
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["reason"], "missing_resource_provenance")
        unknown = _meta(overhead="unknown")
        result = rm.evaluate_resources(unknown, self._samples(),
                                       workload_qualification={"qualified": True})
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["reason"], "overhead_unknown")
        measured_label = _meta(overhead="measured")
        result = rm.evaluate_resources(measured_label, self._samples(),
                                       workload_qualification={"qualified": True})
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["reason"], "overhead_unknown")
        self.assertIn("metrics", result)


class InputPreactivityTrimTests(unittest.TestCase):
    """Leading input-capture unset + all-zero counters may be trimmed; fail-closed otherwise."""

    def _mono_clock(self, elapsed_ns, sample_lo=None, sample_hi=None, read_lo=None, read_hi=None):
        sample_lo = elapsed_ns if sample_lo is None else sample_lo
        sample_hi = elapsed_ns + 5_000_000 if sample_hi is None else sample_hi
        read_lo = elapsed_ns + 100_000 if read_lo is None else read_lo
        read_hi = elapsed_ns + 3_000_000 if read_hi is None else read_hi
        return {
            "domain": "responsiveness_process_mono",
            "elapsed_mono_ns": int(elapsed_ns),
            "sample_mono_ns_lower": int(sample_lo),
            "sample_mono_ns_upper": int(sample_hi),
            "read_mono_ns_lower": int(read_lo),
            "read_mono_ns_upper": int(read_hi),
        }

    def _input_row(
        self,
        *,
        start_n=0,
        complete_n=None,
        cap_lo=0,
        cap_hi=0,
        buckets=None,
        fine=None,
        pending=0,
        canceled=0,
        lost=0,
        dropped=0,
        latency_n=None,
        omit_counters=(),
        visible_ack=True,
    ):
        if complete_n is None:
            complete_n = start_n
        if latency_n is None:
            latency_n = complete_n
        if buckets is None:
            buckets = [int(latency_n)] + [0] * 10
        bounds = list(rm.LATENCY_BOUNDS_MS)
        row = {
            "slot_id": 7,
            "generation": 1,
            "sample_age_ms": 0,
            "ended": False,
            "updated_ms": 0,
            "latency_bound_ms": bounds,
            "decode_edge_n": 0,
            "dispatch_n": 0,
            "decode_canceled_n": 0,
            "decode_unmatched_canceled_n": 0,
            "decode_lost_n": 0,
            "decode_dropped_n": 0,
            "decode_pending_n": 0,
            "decode_latency_n": 0,
            "decode_latency_buckets": [0] * 11,
            "decode_coverage_complete": False,
            "decode_capture_mono_ns_lower": 0,
            "decode_capture_mono_ns_upper": 0,
            "input_start_n": start_n,
            "input_complete_n": complete_n,
            "input_canceled_n": canceled,
            "input_lost_n": lost,
            "input_dropped_n": dropped,
            "input_pending_n": pending,
            "input_latency_n": latency_n,
            "input_latency_buckets": list(buckets),
            "input_coverage_complete": True,
            "input_surface": "tui",
            "visible_ack": {"available": True} if visible_ack else {"available": False},
            "input_capture_mono_ns_lower": int(cap_lo),
            "input_capture_mono_ns_upper": int(cap_hi),
        }
        if fine is not None:
            row["input_fine_latency_buckets"] = list(fine)
            row["fine_latency_bound_ms"] = list(rm.FINE_LATENCY_BOUNDS_MS)
        for key in omit_counters:
            row.pop(key, None)
        return row

    def _sample(self, elapsed_s, elapsed_ns, row, *, clock=True):
        out = {
            "phase": "observe",
            "elapsed_s": float(elapsed_s),
            "responsiveness_profile": [row],
        }
        if clock:
            out["responsiveness_clock"] = self._mono_clock(int(elapsed_ns))
        return out

    def test_leading_unset_zero_then_input_available(self):
        """Leading unset capture + typed-zero counters trim; later input is available."""
        meta = _meta(responsiveness_profile=True)
        e = [40e9, 50e9, 60e9, 70e9, 100e9]
        # Three leading pre-activity rows: valid sample clocks, unset capture, all zero.
        samples = [
            self._sample(40.0 + i, e[i], self._input_row(start_n=0, cap_lo=0, cap_hi=0, buckets=[0] * 11))
            for i in range(3)
        ]
        # Activity: capture set inside observe mono; counters climb with conserved hist.
        samples.append(
            self._sample(
                70.0,
                e[3],
                self._input_row(
                    start_n=0,
                    complete_n=0,
                    cap_lo=int(e[3] + 1e6),
                    cap_hi=int(e[3] + 2e6),
                    buckets=[0] * 11,
                    latency_n=0,
                ),
            )
        )
        # End capture must sit at/before observe_ub (= last elapsed_mono_ns).
        samples.append(
            self._sample(
                160.0,
                e[4],
                self._input_row(
                    start_n=20,
                    complete_n=20,
                    cap_lo=int(e[3] + 3e6),
                    cap_hi=int(e[3] + 4e6),
                    buckets=[20] + [0] * 10,
                    latency_n=20,
                ),
            )
        )
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "available", g)
        slot = g["slots"][0]
        self.assertEqual(slot["status"], "available", slot)
        self.assertNotEqual(slot.get("reason"), "publisher_clock_bracket_incomplete")
        self.assertTrue(slot.get("contained_window"))
        self.assertEqual(slot["counter_deltas"]["start"], 20)
        self.assertEqual(slot["counter_deltas"]["complete"], 20)
        self.assertEqual(slot.get("input_preactivity_rows_trimmed"), 3)

    def test_unset_nonzero_counter_rejects(self):
        meta = _meta(responsiveness_profile=True)
        e0, e1 = 40e9, 100e9
        samples = [
            self._sample(
                40.0,
                e0,
                self._input_row(start_n=5, complete_n=5, cap_lo=0, cap_hi=0, buckets=[5] + [0] * 10),
            ),
            self._sample(
                160.0,
                e1,
                self._input_row(
                    start_n=20,
                    complete_n=20,
                    cap_lo=int(e1 + 1e6),
                    cap_hi=int(e1 + 2e6),
                    buckets=[20] + [0] * 10,
                ),
            ),
        ]
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_incomplete")

    def test_unset_zero_nonzero_hist_rejects(self):
        meta = _meta(responsiveness_profile=True)
        e0, e1 = 40e9, 100e9
        samples = [
            self._sample(
                40.0,
                e0,
                self._input_row(start_n=0, cap_lo=0, cap_hi=0, buckets=[3] + [0] * 10, latency_n=0),
            ),
            self._sample(
                160.0,
                e1,
                self._input_row(
                    start_n=10,
                    complete_n=10,
                    cap_lo=int(e1 + 1e6),
                    cap_hi=int(e1 + 2e6),
                    buckets=[10] + [0] * 10,
                ),
            ),
        ]
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_incomplete")

    def test_missing_global_sample_clock_on_preactivity_rejects(self):
        meta = _meta(responsiveness_profile=True)
        e0, e1, e2 = 40e9, 70e9, 100e9
        samples = [
            self._sample(40.0, e0, self._input_row(start_n=0, cap_lo=0, cap_hi=0, buckets=[0] * 11), clock=False),
            self._sample(
                100.0,
                e1,
                self._input_row(
                    start_n=0,
                    cap_lo=int(e1 + 1e6),
                    cap_hi=int(e1 + 2e6),
                    buckets=[0] * 11,
                    latency_n=0,
                ),
            ),
            self._sample(
                160.0,
                e2,
                self._input_row(
                    start_n=8,
                    complete_n=8,
                    cap_lo=int(e2 + 1e6),
                    cap_hi=int(e2 + 2e6),
                    buckets=[8] + [0] * 10,
                ),
            ),
        ]
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_incomplete")

    def test_missing_sample_clock_post_activity_rejects(self):
        meta = _meta(responsiveness_profile=True)
        e0, e1, e2 = 40e9, 70e9, 100e9
        samples = [
            self._sample(
                40.0,
                e0,
                self._input_row(
                    start_n=0,
                    cap_lo=int(e0 + 1e6),
                    cap_hi=int(e0 + 2e6),
                    buckets=[0] * 11,
                    latency_n=0,
                ),
            ),
            self._sample(
                100.0,
                e1,
                self._input_row(
                    start_n=5,
                    complete_n=5,
                    cap_lo=int(e1 + 1e6),
                    cap_hi=int(e1 + 2e6),
                    buckets=[5] + [0] * 10,
                ),
                clock=False,
            ),
            self._sample(
                160.0,
                e2,
                self._input_row(
                    start_n=10,
                    complete_n=10,
                    cap_lo=int(e2 + 1e6),
                    cap_hi=int(e2 + 2e6),
                    buckets=[10] + [0] * 10,
                ),
            ),
        ]
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_incomplete")

    def test_internal_unset_after_activity_rejects(self):
        meta = _meta(responsiveness_profile=True)
        e0, e1, e2 = 40e9, 70e9, 100e9
        samples = [
            self._sample(
                40.0,
                e0,
                self._input_row(
                    start_n=0,
                    cap_lo=int(e0 + 1e6),
                    cap_hi=int(e0 + 2e6),
                    buckets=[0] * 11,
                    latency_n=0,
                ),
            ),
            self._sample(
                100.0,
                e1,
                self._input_row(
                    start_n=5,
                    complete_n=5,
                    cap_lo=0,
                    cap_hi=0,
                    buckets=[5] + [0] * 10,
                ),
            ),
            self._sample(
                160.0,
                e2,
                self._input_row(
                    start_n=10,
                    complete_n=10,
                    cap_lo=int(e2 + 1e6),
                    cap_hi=int(e2 + 2e6),
                    buckets=[10] + [0] * 10,
                ),
            ),
        ]
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["status"], "unavailable", g)
        self.assertEqual(g["slots"][0]["reason"], "publisher_clock_bracket_incomplete")

    def test_never_input_still_no_input_samples(self):
        meta = _meta(responsiveness_profile=True)
        e0, e1 = 40e9, 100e9
        samples = [
            self._sample(40.0, e0, self._input_row(start_n=0, cap_lo=0, cap_hi=0, buckets=[0] * 11)),
            self._sample(160.0, e1, self._input_row(start_n=0, cap_lo=0, cap_hi=0, buckets=[0] * 11)),
        ]
        g = rm.evaluate_input(meta, samples, target_ms=100)
        self.assertEqual(g["slots"][0]["reason"], "no_input_samples")


class FocusedGpuRoleCoverageTests(unittest.TestCase):
    def _completion(self, count, *, headless=False):
        return {
            "enabled": True, "registered_n": count, "completed_n": count,
            "dropped_n": 0, "lost_n": 0, "pending_n": 0,
            "stable_completed_n": count, "stable_completion_intervals": count,
            "stable_completion_interval_buckets": [0, 0, 0, count] + [0] * 7,
            "completion_latency_buckets": [0] * 11,
            "registration_complete": True, "completion_coverage_complete": True,
            "interval_bound_ms": list(rm.INTERVAL_BOUNDS_MS),
        } if not headless else {
            "enabled": True, "registered_n": 0, "completed_n": 0,
            "dropped_n": 0, "lost_n": 0, "pending_n": 0,
            "stable_completed_n": 0, "stable_completion_intervals": 0,
            "stable_completion_interval_buckets": [0] * 10,
            "completion_latency_buckets": [0] * 11,
            "registration_complete": True, "completion_coverage_complete": True,
        }

    def _row(self, slot, *, focused=False, background=False, count=0):
        headless = not focused and not background
        return {
            "slot_id": slot, "generation": 1, "sample_age_ms": 3,
            "renderer_present": not headless, "backend_kind": "wgpu" if not headless else None,
            "ingame": True, "scene_state": 2, "draw": focused,
            "full_rate": focused, "gpu_completion": self._completion(count, headless=headless),
        }

    def _samples(self, n, policy, middle=None):
        def rows(focus=0, completed=False):
            return [self._row(slot, focused=slot == focus, background=policy == "focused-plus-background" and slot != focus,
                              count=(4800 if slot == focus else 120) if completed else 0) for slot in range(n)]
        start, end = rows(0), rows(0, completed=True)
        if middle is not None:
            middle_rows = rows(middle, completed=True)
            return [{"phase": "observe", "elapsed_s": 0.0, "renderer_profile": start},
                    {"phase": "observe", "elapsed_s": 60.0, "renderer_profile": middle_rows},
                    {"phase": "observe", "elapsed_s": 120.0, "renderer_profile": end}]
        return [{"phase": "observe", "elapsed_s": 0.0, "renderer_profile": start},
                {"phase": "observe", "elapsed_s": 120.0, "renderer_profile": end}]

    def test_public_evaluate_gpu_accepts_focused_one_n1_and_n16(self):
        for n in (1, 16):
            meta = _meta(render_profile=True, gpu_completion_profile=True, frontend="panel", n=n,
                         render_policy="focused-one", render_policy_requested=True)
            result = rm.evaluate_gpu(meta, self._samples(n, "focused-one"))
            self.assertEqual(result["status"], "available", result)
            self.assertEqual(result["target_verdict"], "meet", result)
            self.assertEqual(sum(s["role"] == "expected_headless" for s in result["slots"]), n - 1)

    def test_public_evaluate_gpu_background_requires_all_gpu_rows(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True, frontend="panel", n=16,
                     render_policy="focused-plus-background", render_policy_requested=True)
        result = rm.evaluate_gpu(meta, self._samples(16, "focused-plus-background"))
        self.assertEqual(result["target_verdict"], "meet", result)
        self.assertEqual(result["required_gpu_slot_n"], 16)
        bad_samples = self._samples(16, "focused-plus-background")
        bad_samples[1]["renderer_profile"][-1]["backend_kind"] = None
        bad = rm.evaluate_gpu(meta, bad_samples)
        self.assertEqual(bad["status"], "unavailable")

    def test_focused_role_switch_interior_and_malformed_policy_fail_closed(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True, frontend="panel", n=16,
                     render_policy="focused-one", render_policy_requested=True)
        switched = rm.evaluate_gpu(meta, self._samples(16, "focused-one", middle=7))
        self.assertEqual(switched["status"], "unavailable")
        malformed = dict(meta, render_policy="mystery")
        self.assertEqual(rm.evaluate_gpu(malformed, self._samples(16, "focused-one"))["status"], "unavailable")

    def test_headless_activity_or_missing_focus_never_passes(self):
        meta = _meta(render_profile=True, gpu_completion_profile=True, frontend="panel", n=16,
                     render_policy="focused-one", render_policy_requested=True)
        active_headless = self._samples(16, "focused-one")
        active_headless[1]["renderer_profile"][1]["draw"] = True
        self.assertEqual(rm.evaluate_gpu(meta, active_headless)["status"], "unavailable")
        missing_focus = self._samples(16, "focused-one")
        missing_focus[1]["renderer_profile"][0]["renderer_present"] = False
        missing_focus[1]["renderer_profile"][0]["backend_kind"] = None
        missing_focus[1]["renderer_profile"][0]["draw"] = False
        missing_focus[1]["renderer_profile"][0]["full_rate"] = False
        self.assertEqual(rm.evaluate_gpu(meta, missing_focus)["status"], "unavailable")


if __name__ == "__main__":
    unittest.main()
