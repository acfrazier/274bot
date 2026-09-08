#!/usr/bin/env python3
"""Bounded offline histogram + observation-window gate core.

Workload qualification is delegated to qualify_control (not reimplemented).
This module never claims final performance acceptance. Metrics that cannot be
proven from the input return status=unavailable explicitly — never a silent
pass, never a paint-proxy for GPU completion, never process-wide scheduling as
a per-slot proof.

Own only this file, test_reference_metrics.py, and reference-metrics-report.md.
"""
from __future__ import annotations

import argparse
import json
import math
import pathlib
import statistics
import sys
from datetime import datetime, timezone
from typing import Any, Iterable, Optional

ROOT = pathlib.Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

# Host serializers (cadence / render_profile / responsiveness_profile).
SCHED_EXCESS_BOUNDS_MS = (1, 2, 5, 10, 20)  # + overflow bucket
SCHED_BASELINE_MS = 20  # client-loop budget; interval = baseline + excess
INTERVAL_BOUNDS_MS = (10, 20, 25, 40, 50, 100, 250, 500, 1000, 2000)
# Host cadence per-slot histogram.  These are inclusive upper edges; the final
# bucket is overflow and never supplies a finite p99 upper bound.
SLOT_INTERVAL_BOUNDS_MS = (
    5, 10, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 42, 45, 50, 100, 250,
    500, 1000,
)
LATENCY_BOUNDS_MS = (5, 10, 20, 25, 40, 50, 100, 250, 500, 1000)
# Fine sibling: 1..100 ms inclusive upper edges + overflow (matches host FINE_LATENCY_BOUNDS_MS).
FINE_LATENCY_BOUNDS_MS = tuple(range(1, 101))
RESPONSIVENESS_CLOCK_DOMAIN = "responsiveness_process_mono"
# Paired clean-run non-regression margin (plan §5): candidate upper vs reference lower.
PAIRED_LATENCY_MARGIN_MS = 2.0

PASS_MEANS = (
    "offline histogram / observation-window core only; "
    "not final performance or lifecycle acceptance"
)


def _unavailable(reason: str, **extra: Any) -> dict:
    out = {"status": "unavailable", "reason": reason, "pass_means": PASS_MEANS}
    out.update(extra)
    return out


def _ok(**fields: Any) -> dict:
    out = {"status": "available", "pass_means": PASS_MEANS}
    out.update(fields)
    return out


def _sample_age_reason(row: Optional[dict]) -> Optional[str]:
    """Fail-closed freshness: host emits sample_age_ms on profile slot rows.

    Absent/null/non-finite age cannot prove the observe sample is live → reason
    string; None means present and finite (including 0).
    """
    if not isinstance(row, dict):
        return "sample_age_ms_absent"
    if "sample_age_ms" not in row:
        return "sample_age_ms_absent"
    age = row.get("sample_age_ms")
    if age is None:
        return "sample_age_ms_absent"
    if isinstance(age, bool) or not isinstance(age, (int, float)):
        return "sample_age_ms_invalid"
    if age != age or age in (float("inf"), float("-inf")):  # NaN / inf
        return "sample_age_ms_invalid"
    if age < 0:
        return "sample_age_ms_invalid"
    return None


def load_json(path: pathlib.Path) -> Any:
    return json.loads(path.read_text())


def iter_jsonl(path: pathlib.Path) -> Iterable[dict]:
    """Bounded line iterator — does not slurp schema dumps."""
    with path.open() as fh:
        for i, line in enumerate(fh, 1):
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"malformed JSONL {path.name} line {i}: {exc}") from exc
            if not isinstance(obj, dict):
                raise ValueError(f"malformed JSONL {path.name} line {i}: expected object")
            yield obj


def p99_lower_upper_ms(
    buckets: Optional[list],
    bounds_ms: tuple[int, ...],
) -> dict:
    """Conservative p99 from cumulative fixed buckets.

    Returns lower/upper edges of the bucket that first reaches cumulative count
    ≥ ceil-equivalent of 0.99*n (same rule as host p99_upper_bound_ms:
    cum*100 >= n*99). Overflow or empty → unavailable (no false precise p99).
    """
    if buckets is None:
        return _unavailable("missing_histogram")
    if not isinstance(buckets, (list, tuple)):
        return _unavailable("malformed_histogram")
    expected = len(bounds_ms) + 1  # finite bounds + overflow
    if len(buckets) != expected:
        return _unavailable(
            "histogram_width_mismatch",
            expected_buckets=expected,
            got_buckets=len(buckets),
        )
    try:
        counts = [int(c) for c in buckets]
    except (TypeError, ValueError):
        return _unavailable("malformed_histogram_counts")
    if any(c < 0 for c in counts):
        return _unavailable("negative_histogram_count")
    n = sum(counts)
    if n == 0:
        return _unavailable("empty_histogram")

    cum = 0
    for i, c in enumerate(counts):
        cum += c
        if cum * 100 >= n * 99:
            if i >= len(bounds_ms):
                return _unavailable(
                    "overflow_bucket",
                    sample_n=n,
                    overflow_count=counts[-1],
                    lower_ms=bounds_ms[-1] if bounds_ms else None,
                    upper_ms=None,
                )
            lower = 0 if i == 0 else int(bounds_ms[i - 1])
            upper = int(bounds_ms[i])
            return _ok(
                sample_n=n,
                bucket_index=i,
                lower_ms=lower,
                upper_ms=upper,
                precise_percentile=False,
            )
    return _unavailable("histogram_exhausted", sample_n=n)


def target_verdict(bounds: dict, target_ms: Optional[float]) -> str:
    """meet | miss | unproven | unavailable — never invent a pass."""
    if bounds.get("status") != "available":
        return "unavailable"
    if target_ms is None:
        return "unproven"
    lo = bounds.get("lower_ms")
    hi = bounds.get("upper_ms")
    if hi is None or lo is None:
        return "unproven"
    if hi <= target_ms:
        return "meet"
    if lo > target_ms:
        return "miss"
    return "unproven"


def subtract_counts(end: Any, start: Any) -> Optional[list]:
    """Cumulative end-start; None on length mismatch or counter reset (any drop)."""
    if end is None or start is None:
        return None
    if not isinstance(end, (list, tuple)) or not isinstance(start, (list, tuple)):
        return None
    if len(end) != len(start):
        return None
    out = []
    for e, s in zip(end, start):
        try:
            ev, sv = int(e), int(s)
        except (TypeError, ValueError):
            return None
        if ev < sv:
            return None  # counter reset / generation mismatch residue
        out.append(ev - sv)
    return out


def scheduling_interval_bounds_from_excess(excess_p99: dict) -> dict:
    """Map excess-over-budget p99 bucket to absolute interval lower/upper.

    Host cadence records excess = interval.saturating_sub(budget) with a fixed
    20ms budget. The first excess bucket therefore also holds intervals *below*
    the budget (excess 0), so absolute lower must stay 0 for bucket 0 — it
    cannot prove ≥20ms. Absolute upper = baseline + excess_upper is sound.

    For excess bucket i>0, samples have excess > bounds[i-1], so
    interval > baseline + bounds[i-1].
    """
    if excess_p99.get("status") != "available":
        out = dict(excess_p99)
        out["baseline_ms"] = SCHED_BASELINE_MS
        out["excess_bounds_ms"] = list(SCHED_EXCESS_BOUNDS_MS)
        out["observation_coverage"] = "inconclusive_process_wide_batch_lag"
        return out
    i = int(excess_p99["bucket_index"])
    excess_lo = int(excess_p99["lower_ms"])
    excess_hi = int(excess_p99["upper_ms"])
    if i == 0:
        # Excess ≤1ms includes sub-budget intervals (saturating_sub → 0).
        abs_lo = 0
    else:
        abs_lo = SCHED_BASELINE_MS + excess_lo
    abs_hi = SCHED_BASELINE_MS + excess_hi
    return _ok(
        sample_n=excess_p99["sample_n"],
        bucket_index=i,
        lower_ms=abs_lo,
        upper_ms=abs_hi,
        precise_percentile=False,
        baseline_ms=SCHED_BASELINE_MS,
        excess_lower_ms=excess_lo,
        excess_upper_ms=excess_hi,
        excess_bounds_ms=list(SCHED_EXCESS_BOUNDS_MS),
        observation_coverage="inconclusive_process_wide_batch_lag",
        note=(
            "process-wide drawing/non-drawing groups; per-thread publish lag "
            "up to 49 cycles/slot; cannot prove per-slot or exact observe coverage"
        ),
    )


def subtract_scalar(end: Any, start: Any) -> Optional[int]:
    if end is None or start is None:
        return None
    try:
        ev, sv = int(end), int(start)
    except (TypeError, ValueError):
        return None
    if ev < sv:
        return None
    return ev - sv


def compare_paired_p99_bounds(
    candidate: dict, reference: dict, *, margin_ms: float = 2.0
) -> dict:
    """Conservative paired non-regression check using only bucket bounds.

    The worst defensible candidate-vs-reference difference is candidate upper
    minus reference lower.  No exact percentile or interpolation is inferred.
    """
    if not isinstance(candidate, dict) or not isinstance(reference, dict):
        return {"status": "inconclusive", "reason": "missing_p99_bounds", "margin_ms": margin_ms}
    if candidate.get("status") != "available" or reference.get("status") != "available":
        return {"status": "inconclusive", "reason": "missing_p99_bounds", "margin_ms": margin_ms}
    try:
        difference = float(candidate["upper_ms"]) - float(reference["lower_ms"])
    except (KeyError, TypeError, ValueError):
        return {"status": "inconclusive", "reason": "missing_p99_bounds", "margin_ms": margin_ms}
    result = {
        "status": "available" if difference <= margin_ms else "inconclusive",
        "target_verdict": "meet" if difference <= margin_ms else "unproven",
        "candidate_upper_ms": candidate["upper_ms"],
        "reference_lower_ms": reference["lower_ms"],
        "worst_case_difference_ms": difference,
        "margin_ms": margin_ms,
        "precise_percentile": False,
    }
    if difference > margin_ms:
        result["reason"] = "candidate_upper_minus_reference_lower_exceeds_margin"
    return result


def evaluate_scheduling_slots(
    meta: dict,
    samples: list[dict],
    *,
    target_interval_ms: float = 40.0,
    min_fps: float = 40.0,
) -> dict:
    """Qualify per-slot cadence from endpoint-stamped cumulative snapshots.

    The duration is the closing ``last_cycle_mono_ms`` minus the opening one.
    It is computed independently within each slot's Local origin; absolute
    monotonic values are never compared across slots or with global elapsed_s.
    """
    unavailable = lambda reason, **extra: _unavailable(  # noqa: E731
        reason, gate="scheduling", target_verdict="unavailable", **extra
    )
    if not meta.get("scheduling_profile"):
        return unavailable("profile_disabled", satisfies_per_slot_requirement=False)
    start, end = _observe_pair(samples)
    if start is None or end is None:
        return unavailable("no_observe_samples", satisfies_per_slot_requirement=False)
    s_map, e_map, err, disappeared = _pair_slot_maps(
        start.get("scheduling_slots"), end.get("scheduling_slots")
    )
    if err:
        return unavailable(err, disappeared=[[a, b] for a, b in disappeared])
    assert s_map is not None and e_map is not None
    if set(s_map) != set(e_map):
        start_slots = {key[0] for key in s_map}
        end_slots = {key[0] for key in e_map}
        if start_slots == end_slots and start_slots:
            return unavailable(
                "generation_mismatch", expected_slot_set=sorted(s_map), observed_slot_set=sorted(e_map)
            )
        return unavailable(
            "slot_set_changed", expected_slot_set=sorted(s_map), observed_slot_set=sorted(e_map)
        )
    if not e_map:
        return unavailable("no_slot_rows", expected_slot_set=[])

    slots = []
    for key in sorted(e_map):
        srow, erow = s_map[key], e_map[key]
        base = {"slot_id": key[0], "generation": key[1], "expected_slot": True}
        age_err = _sample_age_reason(srow) or _sample_age_reason(erow)
        if age_err:
            slots.append(_unavailable(age_err, **base, freshness_field="sample_age_ms"))
            continue
        for fresh_row in (srow, erow):
            age = fresh_row.get("sample_age_ms")
            lag_limit = fresh_row.get("flush_lag_max_ms", 1000)
            if isinstance(age, (int, float)) and isinstance(lag_limit, (int, float)) and age > lag_limit:
                age_err = "stale_snapshot"
                break
        if age_err:
            slots.append(_unavailable(age_err, **base, freshness_field="sample_age_ms"))
            continue
        if erow.get("ended"):
            slots.append(_unavailable("slot_ended", **base))
            continue
        if int(erow.get("ended_lost_n") or 0) or int(srow.get("ended_lost_n") or 0):
            slots.append(_unavailable("ended_rows_lost", **base, ended_lost_n=erow.get("ended_lost_n")))
            continue
        try:
            duration_ms = float(erow["last_cycle_mono_ms"]) - float(srow["last_cycle_mono_ms"])
        except (KeyError, TypeError, ValueError):
            slots.append(_unavailable("missing_last_cycle_endpoint", **base))
            continue
        if duration_ms <= 0:
            slots.append(_unavailable("invalid_last_cycle_endpoint_span", **base, measured_duration_ms=duration_ms))
            continue
        cycle_delta = subtract_scalar(erow.get("cycle_n"), srow.get("cycle_n"))
        interval_delta = subtract_scalar(erow.get("interval_n"), srow.get("interval_n"))
        mode_delta = subtract_scalar(erow.get("mode_break_n"), srow.get("mode_break_n"))
        park_delta = subtract_scalar(erow.get("park_n"), srow.get("park_n"))
        anchor_delta = subtract_scalar(erow.get("anchor_miss_n"), srow.get("anchor_miss_n"))
        if None in (cycle_delta, interval_delta, mode_delta, park_delta, anchor_delta):
            slots.append(_unavailable("counter_reset", **base))
            continue
        if cycle_delta != interval_delta + mode_delta + anchor_delta:
            slots.append(_unavailable("interval_coverage_mismatch", **base,
                                       cycle_delta=cycle_delta, interval_n_delta=interval_delta,
                                       mode_break_n_delta=mode_delta, park_n_delta=park_delta,
                                       anchor_miss_n_delta=anchor_delta))
            continue
        if erow.get("interval_coverage_complete") is not True:
            slots.append(_unavailable("coverage_incomplete", **base))
            continue
        delta, hist_err = _slot_hist_delta(srow, erow, "interval_buckets")
        if hist_err:
            slots.append(_unavailable(hist_err, **base))
            continue
        try:
            bound_tuple = tuple(int(x) for x in (erow.get("interval_bound_ms") or SLOT_INTERVAL_BOUNDS_MS))
        except (TypeError, ValueError):
            slots.append(_unavailable("malformed_interval_bounds", **base))
            continue
        bounds = p99_lower_upper_ms(delta, bound_tuple)
        if bounds.get("status") != "available":
            slots.append(_unavailable(bounds.get("reason", "interval_unavailable"), **base, p99=bounds))
            continue
        fps = cycle_delta / (duration_ms / 1000.0)
        verdict = "meet" if fps >= min_fps and bounds["upper_ms"] <= target_interval_ms else "miss"
        slots.append({**base, "status": "available", "measured_duration_ms": duration_ms,
                      "interval_n_delta": interval_delta, "cycle_n_delta": cycle_delta,
                      "mode_break_n_delta": mode_delta, "park_n_delta": park_delta,
                      "anchor_miss_n_delta": anchor_delta, "p99": bounds,
                      "observed_fps": fps, "observed_iterations_per_s": fps,
                      "target_interval_ms": target_interval_ms,
                      "target_verdict": verdict,
                      "coverage_math": "cycle_delta = interval_delta + mode_break_delta + anchor_miss_delta",
                      "scene_transition_separation": erow.get("scene_transition_separation", "unavailable"),
                      "global_mono_alignment_claim": False,
                      "jsonl_row_time_is_not_interval_endpoint": True})
    if not slots or any(s.get("status") != "available" for s in slots):
        return unavailable("no_complete_qualified_per_slot", slots=slots,
                           expected_slot_set=sorted(e_map), full_fleet_coverage=False)
    verdicts = {s["target_verdict"] for s in slots}
    return {"status": "available", "gate": "scheduling", "slots": slots,
            "expected_slot_set": sorted(e_map), "qualified_slot_n": len(slots),
            "full_fleet_coverage": True, "worst_qualified_slot": min(
                slots, key=lambda s: (s["observed_fps"], -s["p99"]["upper_ms"]))["slot_id"],
            "target_verdict": "meet" if verdicts == {"meet"} else "miss",
            "satisfies_per_slot_requirement": verdicts == {"meet"},
            "global_mono_alignment_claim": False, "pass_means": PASS_MEANS}


def _parse_utc(value: Any) -> Optional[float]:
    if value is None:
        return None
    if isinstance(value, (int, float)):
        return float(value)
    if not isinstance(value, str):
        return None
    s = value.strip()
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    try:
        return datetime.fromisoformat(s).timestamp()
    except ValueError:
        return None


def observe_window(
    meta: dict,
    samples: list[dict],
    *,
    contamination_from: Any = None,
) -> dict:
    """Observation boundary timestamps and contamination intersection."""
    if not meta:
        return _unavailable("missing_metadata")
    started = meta.get("started_unix")
    if not isinstance(started, (int, float)):
        return _unavailable("missing_started_unix")

    observe = [s for s in samples if s.get("phase") == "observe"]
    if not observe:
        return _unavailable("no_observe_samples")

    first, last = observe[0], observe[-1]
    e0, e1 = first.get("elapsed_s"), last.get("elapsed_s")
    if not isinstance(e0, (int, float)) or not isinstance(e1, (int, float)):
        return _unavailable("missing_elapsed_s")
    if e1 < e0:
        return _unavailable("elapsed_s_reset")

    obs_start = float(started) + float(e0)
    obs_end = float(started) + float(e1)
    warmup_s = meta.get("warmup_s")
    observe_s = meta.get("observe_s")

    contam_ts = _parse_utc(contamination_from)
    contaminated = False
    intersection = None
    if contam_ts is not None:
        # Intersection of [obs_start, obs_end] with [contam_ts, +inf)
        if obs_end >= contam_ts and obs_start <= obs_end:
            left = max(obs_start, contam_ts)
            right = obs_end
            if left <= right:
                contaminated = True
                intersection = {"start_unix": left, "end_unix": right}

    return _ok(
        started_unix=float(started),
        obs_start_unix=obs_start,
        obs_end_unix=obs_end,
        first_elapsed_s=float(e0),
        last_elapsed_s=float(e1),
        observe_sample_n=len(observe),
        warmup_s=warmup_s,
        observe_s=observe_s,
        contamination_from_unix=contam_ts,
        contaminated=contaminated,
        contamination_intersection=intersection,
    )


def _observe_pair(samples: list[dict]) -> tuple[Optional[dict], Optional[dict]]:
    observe = [s for s in samples if s.get("phase") == "observe"]
    if not observe:
        return None, None
    return observe[0], observe[-1]


def evaluate_scheduling(
    meta: dict,
    samples: list[dict],
    *,
    target_interval_ms: float = 40.0,
) -> dict:
    """Process-wide drawing/non-drawing cadence histograms.

    Explicitly cannot satisfy a per-slot scheduling requirement — status for the
    gate is unavailable even when process-wide diagnostic bounds exist.
    """
    if not meta.get("scheduling_profile"):
        return _unavailable(
            "profile_disabled",
            profile="scheduling_profile",
            satisfies_per_slot_requirement=False,
        )

    start, end = _observe_pair(samples)
    if start is None or end is None:
        return _unavailable("no_observe_samples", satisfies_per_slot_requirement=False)

    s_sched, e_sched = start.get("scheduling"), end.get("scheduling")
    if s_sched is None and e_sched is None:
        return _unavailable("no_input", field="scheduling", satisfies_per_slot_requirement=False)
    if not isinstance(s_sched, list) or not isinstance(e_sched, list):
        return _unavailable("malformed_scheduling", satisfies_per_slot_requirement=False)
    if len(s_sched) != len(e_sched):
        return _unavailable("scheduling_group_mismatch", satisfies_per_slot_requirement=False)

    groups = []
    for sg, eg in zip(s_sched, e_sched):
        if not isinstance(sg, dict) or not isinstance(eg, dict):
            return _unavailable("malformed_scheduling_group", satisfies_per_slot_requirement=False)
        drawing = bool(eg.get("drawing", sg.get("drawing")))
        excess_delta = subtract_counts(
            eg.get("interval_excess_buckets"),
            sg.get("interval_excess_buckets"),
        )
        sleep_delta = subtract_counts(
            eg.get("sleep_excess_buckets"),
            sg.get("sleep_excess_buckets"),
        )
        intervals = subtract_scalar(eg.get("intervals"), sg.get("intervals"))
        cycles = subtract_scalar(eg.get("cycles"), sg.get("cycles"))
        if excess_delta is None:
            group = _unavailable(
                "counter_reset_or_missing_interval_excess",
                drawing=drawing,
                cycles_delta=cycles,
                intervals_delta=intervals,
            )
        else:
            excess_bounds = p99_lower_upper_ms(excess_delta, SCHED_EXCESS_BOUNDS_MS)
            interval_bounds = scheduling_interval_bounds_from_excess(excess_bounds)
            # Process-wide + batch lag → never a per-slot meet; diagnostic only.
            group = {
                "drawing": drawing,
                "cycles_delta": cycles,
                "intervals_delta": intervals,
                "interval_excess_buckets_delta": excess_delta,
                "sleep_excess_buckets_delta": sleep_delta,
                "interval_excess_p99": excess_bounds,
                "interval_p99": interval_bounds,
                "target_interval_ms": target_interval_ms,
                "target_verdict": "unavailable",  # not a per-slot proof
                "scope": "process_wide",
                "satisfies_per_slot_requirement": False,
                "exact_observation_coverage": "inconclusive",
            }
        groups.append(group)

    # Per-slot requirement cannot be met by process-wide groups.
    return _unavailable(
        "process_wide_only_cannot_satisfy_per_slot",
        profile="scheduling_profile",
        satisfies_per_slot_requirement=False,
        process_wide_groups=groups,
        gate="scheduling",
        target_interval_ms=target_interval_ms,
        target_verdict="unavailable",
    )


def _index_slots(rows: Any) -> tuple[Optional[dict[tuple[Any, Any], dict]], Optional[str]]:
    """Index rows by (slot_id, generation). Fail closed on duplicates/malformed."""
    if rows is None:
        return {}, None
    if not isinstance(rows, list):
        return None, "malformed_slot_rows"
    out: dict[tuple[Any, Any], dict] = {}
    for row in rows:
        if not isinstance(row, dict):
            return None, "malformed_slot_row"
        key = (row.get("slot_id"), row.get("generation"))
        if key[0] is None or key[1] is None:
            return None, "missing_slot_id_or_generation"
        if key in out:
            return None, "duplicate_slot_generation"
        out[key] = row
    return out, None


def _pair_slot_maps(
    start_rows: Any,
    end_rows: Any,
) -> tuple[Optional[dict[tuple[Any, Any], dict]], Optional[dict[tuple[Any, Any], dict]], Optional[str], list]:
    """Build start/end maps; reject duplicates and start-only disappearing slots."""
    s_map, err = _index_slots(start_rows)
    if err:
        return None, None, err, []
    e_map, err = _index_slots(end_rows)
    if err:
        return None, None, err, []
    assert s_map is not None and e_map is not None
    disappeared = sorted(k for k in s_map if k not in e_map)
    if disappeared:
        return None, None, "disappearing_expected_slots", disappeared
    return s_map, e_map, None, []


def _slot_hist_delta(
    start_row: dict,
    end_row: dict,
    buckets_key: str,
) -> tuple[Optional[list], Optional[str]]:
    if start_row.get("slot_id") != end_row.get("slot_id"):
        return None, "slot_id_mismatch"
    if start_row.get("generation") != end_row.get("generation"):
        return None, "generation_mismatch"
    delta = subtract_counts(end_row.get(buckets_key), start_row.get(buckets_key))
    if delta is None:
        # Distinguish missing vs reset
        if end_row.get(buckets_key) is None or start_row.get(buckets_key) is None:
            return None, "missing_histogram"
        return None, "counter_reset"
    return delta, None


def _hist_sum(buckets: Optional[list]) -> Optional[int]:
    if not isinstance(buckets, (list, tuple)):
        return None
    total = 0
    for v in buckets:
        if not _finite(v) or float(v) != int(v) or int(v) < 0:
            return None
        total += int(v)
    return total


def _exact_int_bounds_tuple(bound_list: Any, expected: tuple[int, ...]) -> bool:
    """True only when bound_list is exact finite nonnegative ints matching expected.

    Rejects bools, fractional floats (5.9), NaN/inf (no OverflowError via int()),
    strings, and wrong length/order. No lossy int() coercion.
    """
    if not isinstance(bound_list, (list, tuple)):
        return False
    if len(bound_list) != len(expected):
        return False
    for x, exp in zip(bound_list, expected):
        # bool is a subclass of int — reject explicitly.
        if isinstance(x, bool) or not isinstance(x, (int, float)):
            return False
        if isinstance(x, float):
            if not math.isfinite(x) or x < 0:
                return False
            # Exact integer-valued float only (5.0 ok; 5.9 reject). No bare int(inf).
            as_int = int(x)
            if x != as_int or as_int != exp:
                return False
        else:
            # pure int
            if x < 0 or x != exp:
                return False
    return True


def _int_counter_delta(start_row: dict, end_row: dict, key: str) -> tuple[Optional[int], Optional[str]]:
    """Non-negative integer delta for a cumulative counter field when present on both rows."""
    if key not in start_row and key not in end_row:
        return None, None
    if key not in start_row or key not in end_row:
        return None, "latency_counter_incomplete"
    s, e = start_row.get(key), end_row.get(key)
    if not _finite(s) or not _finite(e) or float(s) != int(s) or float(e) != int(e):
        return None, "latency_counter_malformed"
    si, ei = int(s), int(e)
    if si < 0 or ei < 0 or ei < si:
        return None, "counter_reset"
    return ei - si, None


def fine_to_coarse_rollup(
    fine_delta: list,
    coarse_bounds: tuple[int, ...] = LATENCY_BOUNDS_MS,
    fine_bounds: tuple[int, ...] = FINE_LATENCY_BOUNDS_MS,
) -> tuple[Optional[list], Optional[str]]:
    """Roll fine 1 ms bins into coarse bins. Fine overflow maps to all coarse >100 + overflow."""
    if len(fine_delta) != len(fine_bounds) + 1:
        return None, "fine_histogram_width_mismatch"
    expected_coarse = len(coarse_bounds) + 1
    rolled = [0] * expected_coarse
    # Map each finite fine upper edge into the first coarse bound that covers it.
    for i, upper in enumerate(fine_bounds):
        count = int(fine_delta[i])
        placed = False
        for ci, cb in enumerate(coarse_bounds):
            if upper <= cb:
                rolled[ci] += count
                placed = True
                break
        if not placed:
            # Fine finite bin above all coarse bounds → coarse overflow.
            rolled[-1] += count
    # Fine overflow must equal sum of coarse bins whose lower edge is > last fine bound
    # (coarse bounds strictly greater than 100) plus coarse overflow.
    fine_overflow = int(fine_delta[-1])
    last_fine = fine_bounds[-1]
    tail = 0
    for ci, cb in enumerate(coarse_bounds):
        prev = 0 if ci == 0 else coarse_bounds[ci - 1]
        # Coarse bin covers (prev, cb]; contributes to fine-overflow tail iff prev >= last_fine.
        if prev >= last_fine:
            tail += rolled[ci]  # still 0 here; tail is only fine_overflow placement target
    # Place fine overflow entirely into the first coarse bin with prev >= last_fine, else overflow.
    placed_overflow = False
    for ci, cb in enumerate(coarse_bounds):
        prev = 0 if ci == 0 else coarse_bounds[ci - 1]
        if prev >= last_fine:
            rolled[ci] += fine_overflow
            placed_overflow = True
            # Remaining coarser bins stay 0 from fine (events >100 all share fine overflow).
            # We cannot split fine overflow across 250/500/1000; require coarse tail+overflow
            # equality against fine overflow in the conservation check instead.
            break
    if not placed_overflow:
        rolled[-1] += fine_overflow
    return rolled, None


def _validate_fine_coarse_conservation(
    coarse_delta: list,
    fine_delta: list,
    coarse_bounds: tuple[int, ...] = LATENCY_BOUNDS_MS,
    fine_bounds: tuple[int, ...] = FINE_LATENCY_BOUNDS_MS,
) -> Optional[str]:
    """When both hists count the same events: totals match; fine rolls into ≤100 coarse bins."""
    c_sum = _hist_sum(coarse_delta)
    f_sum = _hist_sum(fine_delta)
    if c_sum is None or f_sum is None:
        return "histogram_malformed"
    if c_sum != f_sum:
        return "fine_coarse_count_mismatch"
    if len(coarse_delta) != len(coarse_bounds) + 1:
        return "coarse_histogram_width_mismatch"
    if len(fine_delta) != len(fine_bounds) + 1:
        return "fine_histogram_width_mismatch"
    # Roll finite fine bins into coarse bins with bound <= last_fine.
    last_fine = fine_bounds[-1]
    expected = [0] * len(coarse_delta)
    for i, upper in enumerate(fine_bounds):
        count = int(fine_delta[i])
        for ci, cb in enumerate(coarse_bounds):
            if upper <= cb:
                expected[ci] += count
                break
        else:
            expected[-1] += count
    # Finite coarse bins with bound <= last_fine must match rolled fine.
    for ci, cb in enumerate(coarse_bounds):
        if cb <= last_fine:
            if int(coarse_delta[ci]) != expected[ci]:
                return "fine_coarse_rollup_mismatch"
        else:
            # Coarse bins above fine range must be empty on the rolled side.
            if expected[ci] != 0:
                return "fine_coarse_rollup_mismatch"
    # Fine overflow + any fine beyond last coarse-≤100 must equal coarse tail (bounds > last_fine) + overflow.
    fine_overflow = int(fine_delta[-1])
    coarse_tail = sum(int(coarse_delta[ci]) for ci, cb in enumerate(coarse_bounds) if cb > last_fine)
    coarse_tail += int(coarse_delta[-1])
    if fine_overflow != coarse_tail:
        return "fine_coarse_overflow_mismatch"
    return None


def _validate_latency_hist_conservation(
    *,
    prefix: str,
    srow: dict,
    erow: dict,
    coarse_delta: list,
    fine_delta: Optional[list] = None,
    counter_deltas: Optional[dict] = None,
) -> Optional[str]:
    """Require hist totals match latency_n and event counters; fine/coarse when both present."""
    coarse_sum = _hist_sum(coarse_delta)
    if coarse_sum is None:
        return "histogram_malformed"
    lat_key = f"{prefix}_latency_n"
    lat_delta, lat_err = _int_counter_delta(srow, erow, lat_key)
    if lat_err:
        return lat_err
    if lat_delta is not None and lat_delta != coarse_sum:
        return "histogram_latency_n_mismatch"
    # Event counter: decode uses dispatch; input uses complete.
    if prefix == "decode":
        event_delta = None
        if counter_deltas is not None and "dispatch" in counter_deltas:
            event_delta = int(counter_deltas["dispatch"])
        else:
            event_delta, err = _int_counter_delta(srow, erow, "dispatch_n")
            if err:
                return err
        if event_delta is not None and event_delta != coarse_sum:
            return "histogram_dispatch_mismatch"
        if lat_delta is not None and event_delta is not None and lat_delta != event_delta:
            return "latency_n_dispatch_mismatch"
    elif prefix == "input":
        event_delta = None
        if counter_deltas is not None and "complete" in counter_deltas:
            event_delta = int(counter_deltas["complete"])
        else:
            event_delta, err = _int_counter_delta(srow, erow, "input_complete_n")
            if err:
                return err
        if event_delta is not None and event_delta != coarse_sum:
            return "histogram_complete_mismatch"
        if lat_delta is not None and event_delta is not None and lat_delta != event_delta:
            return "latency_n_complete_mismatch"
    if fine_delta is not None:
        fine_sum = _hist_sum(fine_delta)
        if fine_sum is None:
            return "histogram_malformed"
        if lat_delta is not None and fine_sum != lat_delta:
            return "fine_histogram_latency_n_mismatch"
        if fine_sum != coarse_sum:
            return "fine_coarse_count_mismatch"
        roll_err = _validate_fine_coarse_conservation(coarse_delta, fine_delta)
        if roll_err:
            return roll_err
    return None


def _finite_nonneg_ms(value: Any) -> Optional[float]:
    if value is None or isinstance(value, bool):
        return None
    try:
        v = float(value)
    except (TypeError, ValueError):
        return None
    if not math.isfinite(v) or v < 0.0:
        return None
    return v


def paired_fine_p99_margin(
    candidate_slot: dict,
    reference_slot: dict,
    *,
    candidate_run: Optional[dict] = None,
    reference_run: Optional[dict] = None,
    margin_ms: float = PAIRED_LATENCY_MARGIN_MS,
) -> dict:
    """Diagnostic fine p99 bound difference — pairing eligibility stays unavailable.

    Single-run evaluation must never claim paired ≤2 ms. This helper does **not**
    implement an independently-qualified matched-run reader (no artifact binding,
    slot→run binding, gate/endpoint match, or qualification loader). Callers may
    supply optional run labels for diagnostics only; they never unlock a pair pass.

    When both slots carry available fine_p99 bounds that are finite, nonnegative,
    and lower ≤ upper, the result includes a diagnostic arithmetic difference
    (candidate upper − reference lower) but **never** ``paired_within_margin`` or
    an available/pass status. True paired ≤2 ms remains a remaining gap until a
    real matched-run evidence adapter exists.
    """
    del candidate_run, reference_run  # labels ignored; not proof
    out: dict[str, Any] = {
        "status": "unavailable",
        "reason": "paired_matched_run_evidence_pending",
        "gate": "paired_fine_p99",
        "means": (
            "diagnostic bound difference only when finite bounds present; "
            "paired eligibility unavailable until a real matched-run evidence "
            "adapter binds distinct qualified runs (artifact/slot/gate/endpoint); "
            "not same-run coarse/fine and not caller metadata labels"
        ),
        "margin_ms": None,
        "diagnostic_margin_ms": None,
    }
    m_lim = _finite_nonneg_ms(margin_ms)
    if m_lim is not None:
        out["margin_ms"] = m_lim
    if not isinstance(candidate_slot, dict) or not isinstance(reference_slot, dict):
        out["reason"] = "paired_slot_missing"
        return out
    c_fine = candidate_slot.get("fine_p99")
    r_fine = reference_slot.get("fine_p99")
    if not isinstance(c_fine, dict) or not isinstance(r_fine, dict):
        return out
    if c_fine.get("status") != "available" or r_fine.get("status") != "available":
        return out
    c_lo = _finite_nonneg_ms(c_fine.get("lower_ms"))
    c_up = _finite_nonneg_ms(c_fine.get("upper_ms"))
    r_lo = _finite_nonneg_ms(r_fine.get("lower_ms"))
    r_up = _finite_nonneg_ms(r_fine.get("upper_ms"))
    if c_lo is None or c_up is None or r_lo is None or r_up is None:
        out["reason"] = "paired_fine_bounds_malformed"
        return out
    if c_lo > c_up or r_lo > r_up:
        out["reason"] = "paired_fine_bounds_inverted"
        return out
    margin = c_up - r_lo
    if not math.isfinite(margin):
        out["reason"] = "paired_fine_bounds_malformed"
        return out
    out.update(
        {
            "reason": "paired_matched_run_evidence_pending",
            "candidate_fine_lower_ms": c_lo,
            "candidate_fine_upper_ms": c_up,
            "reference_fine_lower_ms": r_lo,
            "reference_fine_upper_ms": r_up,
            "diagnostic_margin_ms": margin,
            # Explicit: no eligibility boolean — do not reintroduce false pair pass.
        }
    )
    return out


# Removed ad-hoc _PAIRED_RUN_MATCH_KEYS / _paired_run_evidence: caller labels are
# not independently-qualified matched-run proof. Keep arithmetic helper above.


def _validate_row_hist_conservation(row: dict, prefix: str) -> Optional[dict]:
    """Per selected native row: hist sum == latency_n == dispatch/complete (+ fine).

    Required under the responsiveness profile: coarse hist, latency_n, and the
    event counter the publisher always emits. Fine is optional but when present
    must conserve against the same row's coarse hist and latency_n. Does not
    search for a favorable sub-window — callers audit every selected row.
    """
    if prefix == "decode":
        coarse_key = "decode_latency_buckets"
        fine_key = "decode_fine_latency_buckets"
        lat_key = "decode_latency_n"
        event_key = "dispatch_n"
        event_hist_reason = "histogram_dispatch_mismatch"
        lat_event_reason = "latency_n_dispatch_mismatch"
    elif prefix == "input":
        coarse_key = "input_latency_buckets"
        fine_key = "input_fine_latency_buckets"
        lat_key = "input_latency_n"
        event_key = "input_complete_n"
        event_hist_reason = "histogram_complete_mismatch"
        lat_event_reason = "latency_n_complete_mismatch"
    else:
        return {"reason": "unknown_prefix", "prefix": prefix}

    if coarse_key not in row or row.get(coarse_key) is None:
        return {"reason": "histogram_missing", "field": coarse_key}
    if lat_key not in row or row.get(lat_key) is None:
        return {"reason": "latency_counter_incomplete", "field": lat_key}
    if event_key not in row or row.get(event_key) is None:
        return {"reason": "counter_missing_or_malformed", "field": event_key}

    coarse = row.get(coarse_key)
    if not isinstance(coarse, list):
        return {"reason": "histogram_malformed", "field": coarse_key}
    expected_coarse_w = len(LATENCY_BOUNDS_MS) + 1
    if len(coarse) != expected_coarse_w:
        return {"reason": "coarse_histogram_width_mismatch", "field": coarse_key}

    bound_list = row.get("latency_bound_ms")
    if bound_list is None:
        return {"reason": "latency_bounds_missing", "field": "latency_bound_ms"}
    if not _exact_int_bounds_tuple(bound_list, LATENCY_BOUNDS_MS):
        return {"reason": "latency_bounds_schema_mismatch", "field": "latency_bound_ms"}

    coarse_sum = _hist_sum(coarse)
    if coarse_sum is None:
        return {"reason": "histogram_malformed", "field": coarse_key}

    lat_v = row.get(lat_key)
    if not _finite(lat_v) or float(lat_v) != int(lat_v) or int(lat_v) < 0:
        return {"reason": "latency_counter_malformed", "field": lat_key}
    lat_n = int(lat_v)

    ev_v = row.get(event_key)
    if not _finite(ev_v) or float(ev_v) != int(ev_v) or int(ev_v) < 0:
        return {"reason": "counter_missing_or_malformed", "field": event_key}
    event_n = int(ev_v)

    if coarse_sum != lat_n:
        return {"reason": "histogram_latency_n_mismatch", "field": coarse_key,
                "hist_sum": coarse_sum, "latency_n": lat_n}
    if coarse_sum != event_n:
        return {"reason": event_hist_reason, "field": coarse_key,
                "hist_sum": coarse_sum, "event_n": event_n}
    if lat_n != event_n:
        return {"reason": lat_event_reason, "latency_n": lat_n, "event_n": event_n}

    fine = row.get(fine_key)
    if fine is not None:
        if not isinstance(fine, list):
            return {"reason": "histogram_malformed", "field": fine_key}
        expected_fine_w = len(FINE_LATENCY_BOUNDS_MS) + 1
        if len(fine) != expected_fine_w:
            return {"reason": "fine_histogram_width_mismatch", "field": fine_key}
        fine_bound_list = row.get("fine_latency_bound_ms")
        if fine_bound_list is None:
            return {"reason": "fine_latency_bounds_missing", "field": "fine_latency_bound_ms"}
        if not _exact_int_bounds_tuple(fine_bound_list, FINE_LATENCY_BOUNDS_MS):
            return {"reason": "fine_latency_bounds_schema_mismatch",
                    "field": "fine_latency_bound_ms"}
        fine_sum = _hist_sum(fine)
        if fine_sum is None:
            return {"reason": "histogram_malformed", "field": fine_key}
        if fine_sum != lat_n:
            return {"reason": "fine_histogram_latency_n_mismatch", "field": fine_key,
                    "hist_sum": fine_sum, "latency_n": lat_n}
        if fine_sum != coarse_sum:
            return {"reason": "fine_coarse_count_mismatch",
                    "fine_sum": fine_sum, "coarse_sum": coarse_sum}
        roll_err = _validate_fine_coarse_conservation(list(coarse), list(fine))
        if roll_err:
            return {"reason": roll_err, "field": fine_key}
    return None


def _validate_interior_histograms(selected_items: list, prefix: str) -> Optional[dict]:
    """Interior coarse+fine histograms and latency_n must be non-decreasing.

    Shared by decode and input counter families — input must not return before this.
    Also rejects recovered resets where counters climb but hist counts drop then recover.
    Every selected row (endpoints + interior) must conserve hist sum == latency_n ==
    dispatch/complete; missing required native fields reject. Maximal span is kept —
    no favorable inner window around a bad middle.
    """
    if prefix == "decode":
        coarse_key = "decode_latency_buckets"
        fine_key = "decode_fine_latency_buckets"
        lat_key = "decode_latency_n"
        hist_keys = [coarse_key, fine_key]
    elif prefix == "input":
        coarse_key = "input_latency_buckets"
        fine_key = "input_fine_latency_buckets"
        lat_key = "input_latency_n"
        hist_keys = [coarse_key, fine_key]
    else:
        return {"reason": "unknown_prefix", "prefix": prefix}

    # Fine presence must be uniform across the selected span (all or none).
    fine_flags = [item["row"].get(fine_key) is not None for item in selected_items]
    if any(fine_flags) and not all(fine_flags):
        return {"reason": "histogram_missing_interior", "field": fine_key}

    # Coarse hist required on every selected row (native always emits under profile).
    for item in selected_items:
        if item["row"].get(coarse_key) is None:
            return {"reason": "histogram_missing", "field": coarse_key}
        if lat_key not in item["row"] or item["row"].get(lat_key) is None:
            return {"reason": "latency_counter_incomplete", "field": lat_key}

    # Monotonicity first so recovered resets still surface as counter_reset.
    for hkey in hist_keys:
        prev = selected_items[0]["row"].get(hkey)
        if prev is None:
            # Fine optional when entirely absent; coarse already required above.
            continue
        if not isinstance(prev, list):
            return {"reason": "histogram_malformed", "field": hkey}
        for item in selected_items[1:]:
            cur = item["row"].get(hkey)
            if cur is None:
                return {"reason": "histogram_missing_interior", "field": hkey}
            if not isinstance(cur, list) or len(cur) != len(prev):
                return {"reason": "histogram_malformed", "field": hkey}
            if any(
                (not _finite(a) or not _finite(b) or int(b) < int(a)
                 or float(a) != int(a) or float(b) != int(b))
                for a, b in zip(prev, cur)
            ):
                return {"reason": "counter_reset", "field": hkey}
            prev = cur
    prev_lat = None
    for item in selected_items:
        v = item["row"].get(lat_key)
        if not _finite(v) or float(v) != int(v) or int(v) < 0:
            return {"reason": "latency_counter_malformed", "field": lat_key}
        vi = int(v)
        if prev_lat is not None and vi < prev_lat:
            return {"reason": "counter_reset", "field": lat_key}
        prev_lat = vi

    # Every selected row (endpoints + interior): hist sum == latency_n == event.
    for item in selected_items:
        row_err = _validate_row_hist_conservation(item["row"], prefix)
        if row_err is not None:
            return row_err
    return None


def _mono_ns_pair(obj: dict, lo_key: str, hi_key: str) -> tuple[Optional[int], Optional[int], Optional[str]]:
    """Parse an inclusive mono-ns bracket. Returns (lo, hi, err_reason)."""
    lo, hi = obj.get(lo_key), obj.get(hi_key)
    if lo is None and hi is None:
        return None, None, "missing"
    if not _finite(lo) or not _finite(hi):
        return None, None, "malformed"
    lo_i, hi_i = int(lo), int(hi)
    if float(lo) != lo_i or float(hi) != hi_i or lo_i < 0 or hi_i < 0:
        return None, None, "malformed"
    if hi_i < lo_i:
        return None, None, "inverted"
    if lo_i == 0 and hi_i == 0:
        return None, None, "unset"
    return lo_i, hi_i, None


def _input_row_preactivity_ok(row: dict) -> Optional[str]:
    """Return None when input counters/hists prove genuine pre-activity (all zero).

    Missing or malformed required counters, any non-zero counter, or a present
    non-zero/malformed coarse/fine hist means the row is not trim-eligible.
    """
    counter_fields = (
        "input_start_n",
        "input_complete_n",
        "input_canceled_n",
        "input_lost_n",
        "input_dropped_n",
        "input_pending_n",
    )
    for field in counter_fields:
        if field not in row:
            return f"preactivity_counter_missing:{field}"
        value = row.get(field)
        if not _finite(value) or float(value) != int(value) or int(value) < 0:
            return f"preactivity_counter_malformed:{field}"
        if int(value) != 0:
            return f"preactivity_counter_nonzero:{field}"
    for field in ("input_latency_n", "input_latency_ns"):
        if field not in row:
            continue
        value = row.get(field)
        if value is None:
            return f"preactivity_counter_null:{field}"
        if not _finite(value) or float(value) != int(value) or int(value) < 0:
            return f"preactivity_counter_malformed:{field}"
        if int(value) != 0:
            return f"preactivity_counter_nonzero:{field}"
    for hkey in ("input_latency_buckets", "input_fine_latency_buckets"):
        hist = row.get(hkey)
        if hist is None:
            continue
        if not isinstance(hist, list):
            return f"preactivity_hist_malformed:{hkey}"
        for item in hist:
            if not _finite(item) or float(item) != int(item) or int(item) < 0:
                return f"preactivity_hist_malformed:{hkey}"
            if int(item) != 0:
                return f"preactivity_hist_nonzero:{hkey}"
    return None


def _contained_responsiveness_pair(
    meta: dict, samples: list[dict], key: tuple[Any, Any], prefix: str,
) -> tuple[Optional[dict], Optional[dict], dict]:
    """Audit one deterministic maximal publisher-contained counter span.

    Uses native process-local mono brackets (decode/input capture + sample/read)
    when present. Never invents a clock from age alone. Never selects a
    favorable inner segment after outcome validation fails.

    For prefix==\"input\" only: leading rows with unset/missing input capture and
    genuine all-zero input counters/hists are omitted after sample/read clock
    validation (pre-activity trim). Observe mono bounds still come from the full
    observe series of validated sample clocks. Unset capture after any activity
    or set capture still fails closed.
    """
    observed = [s for s in samples if isinstance(s, dict) and s.get("phase") == "observe"]
    if len(observed) < 2:
        return None, None, {"reason": "publisher_timestamp_missing"}

    cap_lo_k = f"{prefix}_capture_mono_ns_lower"
    cap_hi_k = f"{prefix}_capture_mono_ns_upper"
    rows = []
    clocks_present = 0
    clocks_absent = 0
    # Full-observe mono bounds (includes input pre-activity rows that were trimmed).
    observe_elapsed_ns: list[int] = []
    input_preactivity_rows_trimmed = 0
    input_activity_or_capture_seen = False
    prev_elapsed: Optional[float] = None
    for sample_index, sample in enumerate(observed):
        elapsed = sample.get("elapsed_s")
        if not _finite(elapsed):
            return None, None, {"reason": "invalid_observation_elapsed"}
        elapsed_f = float(elapsed)
        if prev_elapsed is not None and elapsed_f <= prev_elapsed:
            return None, None, {"reason": "non_increasing_observation_elapsed"}
        prev_elapsed = elapsed_f
        matches = [r for r in (sample.get("responsiveness_profile") or [])
                   if isinstance(r, dict) and (r.get("slot_id"), r.get("generation")) == key]
        if len(matches) != 1:
            return None, None, {"reason": "publisher_slot_row_missing_or_duplicate",
                                "sample_index": sample_index}
        row = matches[0]
        age = row.get("sample_age_ms")
        updated = row.get("updated_ms")
        if not _finite(age) or float(age) < 0:
            return None, None, {"reason": "publisher_age_invalid"}
        if not _finite(updated):
            return None, None, {"reason": "publisher_timestamp_missing"}

        clock = sample.get("responsiveness_clock")
        if not isinstance(clock, dict):
            clocks_absent += 1
            rows.append({
                "i": sample_index, "elapsed": elapsed_f, "row": row,
                "age": float(age), "clock": None,
            })
            continue

        # Domain is required exactly when any mono evidence is present.
        domain = clock.get("domain")
        if domain != "responsiveness_process_mono":
            if domain is None and not any(
                k in clock for k in (
                    "sample_mono_ns_lower", "read_mono_ns_lower", "elapsed_mono_ns",
                )
            ):
                clocks_absent += 1
                rows.append({
                    "i": sample_index, "elapsed": elapsed_f, "row": row,
                    "age": float(age), "clock": None,
                })
                continue
            return None, None, {"reason": "clock_domain_mismatch", "domain": domain}

        sample_lo, sample_hi, s_err = _mono_ns_pair(
            clock, "sample_mono_ns_lower", "sample_mono_ns_upper")
        read_lo, read_hi, r_err = _mono_ns_pair(
            clock, "read_mono_ns_lower", "read_mono_ns_upper")
        cap_lo, cap_hi, c_err = _mono_ns_pair(row, cap_lo_k, cap_hi_k)
        elapsed_ns = clock.get("elapsed_mono_ns")
        if s_err == "missing" and r_err == "missing" and c_err in (None, "missing", "unset") and elapsed_ns is None:
            clocks_absent += 1
            rows.append({
                "i": sample_index, "elapsed": elapsed_f, "row": row,
                "age": float(age), "clock": None,
            })
            continue
        if s_err or r_err:
            return None, None, {
                "reason": "publisher_clock_bracket_malformed",
                "sample_err": s_err, "read_err": r_err, "sample_index": sample_index,
            }
        if not _finite(elapsed_ns) or float(elapsed_ns) != int(elapsed_ns) or int(elapsed_ns) < 0:
            return None, None, {
                "reason": "elapsed_mono_ns_malformed", "sample_index": sample_index,
            }
        elapsed_ns_i = int(elapsed_ns)
        # Sample/read already validated: elapsed must lie in sample before trim.
        assert sample_lo is not None and sample_hi is not None
        assert read_lo is not None and read_hi is not None
        if read_lo < sample_lo or read_hi > sample_hi:
            return None, None, {
                "reason": "read_outside_sample_bracket",
                "sample_index": sample_index,
            }
        if elapsed_ns_i < sample_lo or elapsed_ns_i > sample_hi:
            return None, None, {
                "reason": "elapsed_mono_outside_sample_bracket",
                "sample_index": sample_index,
            }
        if c_err == "unset" or c_err == "missing":
            if prefix == "input":
                pre_err = _input_row_preactivity_ok(row)
                if pre_err is None and not input_activity_or_capture_seen:
                    # Genuine leading pre-activity: keep observe mono bounds, omit row.
                    observe_elapsed_ns.append(elapsed_ns_i)
                    input_preactivity_rows_trimmed += 1
                    continue
                return None, None, {
                    "reason": "publisher_clock_bracket_incomplete",
                    "missing_capability": "native_sample_publisher_bracket",
                    "sample_index": sample_index,
                    "prefix": prefix,
                    "capture_err": c_err,
                    "preactivity_detail": (
                        "input_capture_unset_after_activity"
                        if pre_err is None
                        else pre_err
                    ),
                }
            clocks_absent += 1
            rows.append({
                "i": sample_index, "elapsed": elapsed_f, "row": row,
                "age": float(age), "clock": None,
            })
            continue
        if c_err:
            return None, None, {
                "reason": "capture_bracket_malformed",
                "capture_err": c_err, "sample_index": sample_index, "prefix": prefix,
            }
        assert cap_lo is not None and cap_hi is not None
        # Capture ends at/before read upper; capture sits at/before sample upper.
        if cap_hi > read_hi or cap_hi > sample_hi or cap_lo > sample_hi:
            return None, None, {
                "reason": "capture_outside_sample_bracket",
                "sample_index": sample_index,
            }
        if prefix == "input":
            input_activity_or_capture_seen = True
        clocks_present += 1
        observe_elapsed_ns.append(elapsed_ns_i)
        rows.append({
            "i": sample_index, "elapsed": elapsed_f, "row": row,
            "age": float(age), "clock": clock,
            "sample_lo": sample_lo, "sample_hi": sample_hi,
            "read_lo": read_lo, "read_hi": read_hi,
            "cap_lo": cap_lo, "cap_hi": cap_hi,
            "elapsed_ns": elapsed_ns_i,
        })

    if any(b["elapsed"] <= a["elapsed"] for a, b in zip(rows, rows[1:])):
        return None, None, {"reason": "non_increasing_observation_elapsed"}

    def _validate_counters(selected_items: list) -> tuple[Optional[dict], Optional[dict], dict]:
        """Shared counter/outcome checks on a pre-selected maximal span."""
        if not selected_items:
            return None, None, {"reason": "publisher_window_too_short"}
        srow, erow = selected_items[0]["row"], selected_items[-1]["row"]
        # Any ended row in the span (including interior) rejects the window.
        for item in selected_items:
            if item["row"].get("ended") is True:
                return None, None, {"reason": "boundary_ended" if item in (selected_items[0], selected_items[-1]) else "interior_ended"}
        if prefix == "input":
            mono_names = ("start", "complete", "canceled", "lost", "dropped")
            fields = {name: f"input_{name}_n" for name in mono_names}
            fields["pending"] = "input_pending_n"
            parsed = []
            for item in selected_items:
                values = [item["row"].get(field) for field in fields.values()]
                if not all(_finite(v) and float(v) >= 0 and float(v) == int(v) for v in values):
                    return None, None, {"reason": "counter_missing_or_malformed"}
                parsed.append({name: int(item["row"][field]) for name, field in fields.items()})
            # Pending is a gauge — exclude from monotonic/reset checks.
            for before, after in zip(parsed, parsed[1:]):
                if any(after[name] < before[name] for name in mono_names):
                    return None, None, {"reason": "counter_reset"}
            deltas = {name: parsed[-1][name] - parsed[0][name] for name in fields}
            if any(deltas[name] < 0 for name in mono_names):
                return None, None, {"reason": "counter_reset"}
            if any(deltas[name] for name in ("canceled", "lost", "dropped")):
                return None, None, {"reason": "coverage_lost_or_incomplete"}
            if parsed[0]["pending"] != 0 or parsed[-1]["pending"] != 0:
                return None, None, {"reason": "boundary_pending_incomplete"}
            # Per-row identity: start - complete - canceled - lost - dropped == pending
            for row in parsed:
                open_n = (row["start"] - row["complete"] - row["canceled"]
                          - row["lost"] - row["dropped"])
                if open_n != row["pending"]:
                    return None, None, {
                        "reason": "input_accounting_identity_mismatch",
                        "open_n": open_n, "pending": row["pending"],
                    }
            if deltas["complete"] > deltas["start"]:
                return None, None, {"reason": "input_accounting_identity_mismatch"}
            # Span identity with pending gauge: Δstart = Δcomplete + Δcanceled + Δlost + Δdropped + Δpending
            if (deltas["start"] != deltas["complete"] + deltas["canceled"]
                    + deltas["lost"] + deltas["dropped"] + deltas["pending"]):
                return None, None, {"reason": "input_accounting_identity_mismatch"}
            hist_err = _validate_interior_histograms(selected_items, prefix)
            if hist_err is not None:
                return None, None, hist_err
            return srow, erow, {"counter_deltas": deltas}
        mono_names = ("edge", "dispatch", "canceled", "lost", "dropped")
        names = {
            "edge": f"{prefix}_edge_n",
            "dispatch": "dispatch_n",
            "canceled": f"{prefix}_canceled_n",
            "lost": f"{prefix}_lost_n",
            "dropped": f"{prefix}_dropped_n",
            "pending": f"{prefix}_pending_n",
        }
        unmatched_key = f"{prefix}_unmatched_canceled_n"
        parsed = []
        for item in selected_items:
            values = [item["row"].get(name) for name in names.values()]
            if not all(_finite(v) and float(v) >= 0 and float(v) == int(v) for v in values):
                return None, None, {"reason": "counter_missing_or_malformed"}
            row = {name: int(item["row"][field]) for name, field in names.items()}
            if prefix == "decode":
                um = item["row"].get(unmatched_key)
                if um is None:
                    # Legacy rows without unmatched sibling cannot prove identity
                    # under native unmatched-cancel semantics.
                    return None, None, {
                        "reason": "unmatched_canceled_counter_missing",
                        "missing_capability": "decode_unmatched_canceled_n",
                    }
                if not _finite(um) or float(um) != int(um) or int(um) < 0:
                    return None, None, {"reason": "counter_missing_or_malformed",
                                        "field": unmatched_key}
                row["unmatched_canceled"] = int(um)
                if row["unmatched_canceled"] > row["canceled"]:
                    return None, None, {"reason": "accounting_identity_mismatch",
                                        "detail": "unmatched_gt_canceled"}
            else:
                # Input has no unmatched sibling yet; identity is
                # edge(=start) = complete + canceled + pending + lost + dropped.
                row["unmatched_canceled"] = 0
            parsed.append(row)
        for before, after in zip(parsed, parsed[1:]):
            if any(after[name] < before[name] for name in mono_names):
                return None, None, {"reason": "counter_reset"}
            if after["unmatched_canceled"] < before["unmatched_canceled"]:
                return None, None, {"reason": "counter_reset", "field": unmatched_key}
        deltas = {name: parsed[-1][name] - parsed[0][name] for name in list(names) + ["unmatched_canceled"]}
        if any(deltas[name] < 0 for name in mono_names):
            return None, None, {"reason": "counter_reset"}
        if any(deltas[name] for name in ("canceled", "lost", "dropped")):
            # canceled delta may include unmatched; still a coverage cancel in-span
            if deltas["canceled"] or deltas["lost"] or deltas["dropped"]:
                # Matched cancels in-span are coverage_lost; unmatched-only may
                # still break product coverage intent — reject any cancel delta.
                return None, None, {"reason": "coverage_lost_or_incomplete"}
        if parsed[0]["pending"] != 0 or parsed[-1]["pending"] != 0:
            return None, None, {"reason": "boundary_pending_incomplete"}
        # edge = dispatch + canceled - unmatched_canceled + pending + dropped + lost
        for row in parsed:
            rhs = (row["dispatch"] + row["canceled"] - row["unmatched_canceled"]
                   + row["pending"] + row["dropped"] + row["lost"])
            if row["edge"] != rhs:
                return None, None, {
                    "reason": "accounting_identity_mismatch",
                    "edge": row["edge"], "rhs": rhs,
                }
        if (deltas["edge"] != deltas["dispatch"] + deltas["canceled"] - deltas["unmatched_canceled"]
                + deltas["pending"] + deltas["dropped"] + deltas["lost"]):
            return None, None, {"reason": "accounting_identity_mismatch"}
        hist_err = _validate_interior_histograms(selected_items, prefix)
        if hist_err is not None:
            return None, None, hist_err
        return srow, erow, {"counter_deltas": deltas}

    # No native mono: age-based maximal span still audits counters, then fails
    # closed on missing clock (never a contained pass without mono proof).
    if clocks_present == 0:
        observe_lb_e, observe_ub_e = rows[0]["elapsed"], rows[-1]["elapsed"]
        for r in rows:
            r["pub_lo"] = r["elapsed"] - r["age"] / 1000.0
            r["pub_hi"] = r["elapsed"]
        start_c = [r for r in rows if r["pub_lo"] >= observe_lb_e]
        end_c = [r for r in rows if r["pub_hi"] <= observe_ub_e]
        if not start_c or not end_c:
            return None, None, {
                "reason": "publisher_window_not_contained",
                "missing_capability": "native_sample_publisher_bracket",
            }
        si, ei = start_c[0], end_c[-1]
        if si["i"] >= ei["i"]:
            return None, None, {"reason": "publisher_window_too_short"}
        selected = rows[si["i"]:ei["i"] + 1]
        if len(selected) != ei["i"] - si["i"] + 1:
            return None, None, {"reason": "publisher_slot_row_missing_or_duplicate"}
        _s, _e, ctr = _validate_counters(selected)
        if _s is None:
            return None, None, ctr
        return None, None, {
            "reason": "publisher_clock_bracket_missing",
            "missing_capability": "native_sample_publisher_bracket",
            "selected_start_elapsed_s": si["elapsed"],
            "selected_end_elapsed_s": ei["elapsed"],
            "sample_ages_ms": [item["age"] for item in selected],
            "coverage_excluded_edges": {
                "before_start_samples": si["i"],
                "after_end_samples": len(observed) - 1 - ei["i"],
            },
        }
    if clocks_absent:
        return None, None, {
            "reason": "publisher_clock_bracket_incomplete",
            "missing_capability": "native_sample_publisher_bracket",
            "clocks_present": clocks_present,
            "clocks_absent": clocks_absent,
        }

    # Conservative observe mono window from first/last elapsed_mono_ns across
    # the full observe series of validated sample clocks (including any input
    # pre-activity rows that were trimmed out of the selection list). Not
    # sample_hi — that includes later registry-read/serialization.
    if len(observe_elapsed_ns) < 2:
        # Fallback for paths that only populated rows with clocks (e.g. decode).
        clocked = [r for r in rows if r.get("elapsed_ns") is not None]
        if len(clocked) < 2:
            return None, None, {
                "reason": "publisher_clock_bracket_incomplete",
                "missing_capability": "native_sample_publisher_bracket",
                "clocks_present": clocks_present,
                "clocks_absent": clocks_absent,
            }
        observe_elapsed_ns = [int(r["elapsed_ns"]) for r in clocked]
    observe_lb = observe_elapsed_ns[0]
    observe_ub = observe_elapsed_ns[-1]
    if observe_ub < observe_lb:
        return None, None, {"reason": "observe_mono_window_inverted"}

    # Maximal contained span over the (possibly pre-activity-trimmed) row list:
    # first list entry whose capture lower ≥ observe_lb, last whose capture
    # upper ≤ observe_ub. No favorable inner search. List positions — not
    # raw sample indices — because leading input pre-activity rows may be gone.
    start_pos = next((idx for idx, r in enumerate(rows) if r.get("cap_lo") is not None and r["cap_lo"] >= observe_lb), None)
    end_pos = next(
        (idx for idx in range(len(rows) - 1, -1, -1)
         if rows[idx].get("cap_hi") is not None and rows[idx]["cap_hi"] <= observe_ub),
        None,
    )
    if start_pos is None or end_pos is None:
        return None, None, {
            "reason": "publisher_window_not_contained",
            "missing_capability": "native_sample_publisher_bracket",
        }
    if start_pos >= end_pos:
        return None, None, {"reason": "publisher_window_too_short"}
    selected = rows[start_pos:end_pos + 1]
    start_item, end_item = selected[0], selected[-1]
    # Contiguous original sample indices inside the selected mono span.
    for a, b in zip(selected, selected[1:]):
        if b["i"] != a["i"] + 1:
            return None, None, {"reason": "publisher_slot_row_missing_or_duplicate"}
        if b["cap_lo"] < a["cap_lo"] or b["cap_hi"] < a["cap_hi"]:
            return None, None, {"reason": "capture_bracket_order_regression"}
        if b["elapsed_ns"] < a["elapsed_ns"]:
            return None, None, {"reason": "elapsed_mono_order_regression"}

    srow, erow, ctr = _validate_counters(selected)
    if srow is None:
        return None, None, ctr
    meta_out = {
        "contained_window": True,
        "clock_domain": "responsiveness_process_mono",
        "selected_start_elapsed_s": start_item["elapsed"],
        "selected_end_elapsed_s": end_item["elapsed"],
        "sample_ages_ms": [item["age"] for item in selected],
        "observe_mono_ns_lower": observe_lb,
        "observe_mono_ns_upper": observe_ub,
        "start_capture_mono_ns_lower": start_item["cap_lo"],
        "end_capture_mono_ns_upper": end_item["cap_hi"],
        "counter_deltas": ctr["counter_deltas"],
        "coverage_excluded_edges": {
            "before_start_samples": start_item["i"],
            "after_end_samples": len(observed) - 1 - end_item["i"],
        },
    }
    if prefix == "input":
        meta_out["input_preactivity_rows_trimmed"] = input_preactivity_rows_trimmed
    return srow, erow, meta_out

def evaluate_decode(
    meta: dict,
    samples: list[dict],
    *,
    target_ms: float = 100.0,
) -> dict:
    if not meta.get("responsiveness_profile"):
        return _unavailable("profile_disabled", profile="responsiveness_profile", gate="decode")

    start, end = _observe_pair(samples)
    if start is None or end is None:
        return _unavailable("no_observe_samples", gate="decode")

    s_rows, e_rows = start.get("responsiveness_profile"), end.get("responsiveness_profile")
    if s_rows is None and e_rows is None:
        return _unavailable("no_input", field="responsiveness_profile", gate="decode")
    s_map, e_map, idx_err, disappeared = _pair_slot_maps(s_rows, e_rows)
    if idx_err:
        return _unavailable(
            idx_err,
            gate="decode",
            disappeared=[[a, b] for a, b in disappeared],
        )
    assert s_map is not None and e_map is not None
    if not e_map:
        return _unavailable("no_slot_rows", gate="decode")

    slots = []
    any_available = False
    for key, erow in e_map.items():
        srow = s_map.get(key)
        if srow is None:
            slots.append(_unavailable("missing_start_slot", slot_id=key[0], generation=key[1], gate="decode"))
            continue

        raw_fields = {"decode_edge_n", "dispatch_n"}
        contained = {}
        if any(field in srow or field in erow for field in raw_fields):
            contained_start, contained_end, contained = _contained_responsiveness_pair(meta, samples, key, "decode")
            if contained_start is None or contained_end is None:
                slots.append(_unavailable(contained.get("reason", "contained_window_unavailable"),
                                          slot_id=key[0], generation=key[1], gate="decode"))
                continue
            srow, erow = contained_start, contained_end

        # Coverage / integrity on end snapshot (cumulative). Drops/cancels/lost
        # or incomplete/absent coverage cannot pass.
        dropped = int(erow.get("decode_dropped_n") or 0)
        canceled = int(erow.get("decode_canceled_n") or 0)
        lost = int(erow.get("decode_lost_n") or 0)
        pending = int(erow.get("decode_pending_n") or 0)
        coverage = erow.get("decode_coverage_complete")
        if not contained and (dropped or canceled or lost or pending):
            slots.append(
                _unavailable(
                    "coverage_lost_or_incomplete",
                    slot_id=key[0],
                    generation=key[1],
                    gate="decode",
                    decode_dropped_n=dropped,
                    decode_canceled_n=canceled,
                    decode_lost_n=lost,
                    decode_pending_n=pending,
                    decode_coverage_complete=coverage,
                )
            )
            continue
        if not contained and coverage is None:
            slots.append(
                _unavailable(
                    "coverage_flag_absent",
                    slot_id=key[0],
                    generation=key[1],
                    gate="decode",
                    decode_coverage_complete=coverage,
                )
            )
            continue
        if not contained and coverage is False:
            slots.append(
                _unavailable(
                    "coverage_incomplete",
                    slot_id=key[0],
                    generation=key[1],
                    gate="decode",
                    decode_coverage_complete=coverage,
                )
            )
            continue

        # Freshness: sample_age_ms on end responsiveness row (host publish age).
        age_err = _sample_age_reason(erow)
        if age_err:
            slots.append(
                _unavailable(
                    age_err,
                    slot_id=key[0],
                    generation=key[1],
                    gate="decode",
                    sample_age_ms=erow.get("sample_age_ms") if isinstance(erow, dict) else None,
                    freshness_field="sample_age_ms",
                )
            )
            continue

        delta, err = _slot_hist_delta(srow, erow, "decode_latency_buckets")
        if err:
            slots.append(
                _unavailable(err, slot_id=key[0], generation=key[1], gate="decode")
            )
            continue
        # Fine hist optional sibling — only when both endpoints carry the array.
        fine_delta = None
        fine_err = None
        if (
            srow.get("decode_fine_latency_buckets") is not None
            or erow.get("decode_fine_latency_buckets") is not None
        ):
            fine_delta, fine_err = _slot_hist_delta(srow, erow, "decode_fine_latency_buckets")
            if fine_err:
                slots.append(
                    _unavailable(fine_err, slot_id=key[0], generation=key[1], gate="decode",
                                 field="decode_fine_latency_buckets")
                )
                continue
        cons_err = _validate_latency_hist_conservation(
            prefix="decode",
            srow=srow,
            erow=erow,
            coarse_delta=delta,
            fine_delta=fine_delta,
            counter_deltas=contained.get("counter_deltas") if contained else None,
        )
        if cons_err:
            slots.append(
                _unavailable(cons_err, slot_id=key[0], generation=key[1], gate="decode")
            )
            continue
        bounds = p99_lower_upper_ms(delta, LATENCY_BOUNDS_MS)
        # Prefer host-emitted bounds list when present.
        bound_list = erow.get("latency_bound_ms")
        if isinstance(bound_list, list) and bound_list:
            try:
                bounds = p99_lower_upper_ms(delta, tuple(int(x) for x in bound_list))
            except (TypeError, ValueError):
                pass
        fine_bounds = None
        if fine_delta is not None:
            fine_bound_list = erow.get("fine_latency_bound_ms")
            fine_bounds_tuple = FINE_LATENCY_BOUNDS_MS
            if isinstance(fine_bound_list, list) and fine_bound_list:
                try:
                    fine_bounds_tuple = tuple(int(x) for x in fine_bound_list)
                except (TypeError, ValueError):
                    pass
            fine_bounds = p99_lower_upper_ms(fine_delta, fine_bounds_tuple)
        verdict = target_verdict(bounds, target_ms)
        row = {
            "slot_id": key[0],
            "generation": key[1],
            "gate": "decode",
            "buckets_delta": delta,
            "p99": bounds,
            "target_ms": target_ms,
            "target_verdict": verdict,
            "decode_coverage_complete": coverage,
            "sample_age_ms": erow.get("sample_age_ms"),
            "freshness_field": "sample_age_ms",
        }
        if fine_bounds is not None:
            row["fine_buckets_delta"] = fine_delta
            row["fine_p99"] = fine_bounds
            # Same-run coarse/fine is NOT a paired 2ms proof. Paired margin
            # requires paired_fine_p99_margin(candidate_slot, reference_slot).
        if contained:
            row.update(contained)
            row["decode_canceled_n_delta"] = contained["counter_deltas"]["canceled"]
        if bounds.get("status") == "available" and verdict == "meet":
            row["status"] = "available"
            any_available = True
        elif bounds.get("status") == "available":
            row["status"] = "available"
            row["reason"] = f"target_{verdict}"
            any_available = True
        else:
            row.update(bounds)
        slots.append(row)

    if not slots:
        return _unavailable("no_matched_slots", gate="decode")
    if not any_available:
        return {
            "status": "unavailable",
            "reason": "no_slot_with_available_decode_p99",
            "gate": "decode",
            "slots": slots,
            "pass_means": PASS_MEANS,
            "target_verdict": "unavailable",
        }
    # Aggregate: require all slots meet for gate meet; else unproven/miss/unavailable
    verdicts = {s.get("target_verdict") for s in slots}
    if verdicts == {"meet"}:
        agg = "meet"
        status = "available"
    elif "unavailable" in verdicts and not (verdicts - {"unavailable"}):
        agg = "unavailable"
        status = "unavailable"
    elif "miss" in verdicts:
        agg = "miss"
        status = "available"
    else:
        agg = "unproven"
        status = "available"
    return {
        "status": status,
        "gate": "decode",
        "slots": slots,
        "target_ms": target_ms,
        "target_verdict": agg,
        "pass_means": PASS_MEANS,
        "reason": None if status == "available" else "no_slot_with_available_decode_p99",
    }


def evaluate_input(
    meta: dict,
    samples: list[dict],
    *,
    target_ms: float = 100.0,
) -> dict:
    if not meta.get("responsiveness_profile"):
        return _unavailable("profile_disabled", profile="responsiveness_profile", gate="input")

    start, end = _observe_pair(samples)
    if start is None or end is None:
        return _unavailable("no_observe_samples", gate="input")

    s_rows, e_rows = start.get("responsiveness_profile"), end.get("responsiveness_profile")
    if s_rows is None and e_rows is None:
        return _unavailable("no_input", field="responsiveness_profile", gate="input")
    s_map, e_map, idx_err, disappeared = _pair_slot_maps(s_rows, e_rows)
    if idx_err:
        return _unavailable(
            idx_err,
            gate="input",
            disappeared=[[a, b] for a, b in disappeared],
        )
    assert s_map is not None and e_map is not None
    if not e_map:
        return _unavailable("no_slot_rows", gate="input")

    slots = []
    any_available = False
    for key, erow in e_map.items():
        srow = s_map.get(key)
        if srow is None:
            slots.append(_unavailable("missing_start_slot", slot_id=key[0], generation=key[1], gate="input"))
            continue

        # No input samples at all → unavailable (not a pass).
        start_n = int(erow.get("input_start_n") or 0)
        if start_n == 0 and int(srow.get("input_start_n") or 0) == 0:
            slots.append(
                _unavailable(
                    "no_input_samples",
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                )
            )
            continue

        raw_fields = {"input_start_n", "input_complete_n"}
        contained = {}
        if all(field in srow and field in erow for field in raw_fields):
            contained_start, contained_end, contained = _contained_responsiveness_pair(meta, samples, key, "input")
            if contained_start is None or contained_end is None:
                slots.append(_unavailable(contained.get("reason", "contained_window_unavailable"),
                                          slot_id=key[0], generation=key[1], gate="input"))
                continue
            srow, erow = contained_start, contained_end

        dropped = int(erow.get("input_dropped_n") or 0)
        canceled = int(erow.get("input_canceled_n") or 0)
        lost = int(erow.get("input_lost_n") or 0)
        pending = int(erow.get("input_pending_n") or 0)
        coverage = erow.get("input_coverage_complete")
        if not contained and (dropped or canceled or lost or pending):
            slots.append(
                _unavailable(
                    "coverage_lost_or_incomplete",
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                    input_dropped_n=dropped,
                    input_canceled_n=canceled,
                    input_lost_n=lost,
                    input_pending_n=pending,
                    input_coverage_complete=coverage,
                )
            )
            continue
        if not contained and coverage is None:
            slots.append(
                _unavailable(
                    "coverage_flag_absent",
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                    input_coverage_complete=coverage,
                )
            )
            continue
        if not contained and coverage is False:
            slots.append(
                _unavailable(
                    "coverage_incomplete",
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                    input_coverage_complete=coverage,
                )
            )
            continue

        # visible_ack: require explicit end-row dict with available is True.
        # Absent/malformed/False must not false-pass (serializer always emits
        # the object when profile on; core still fail-closed if missing).
        ack = erow.get("visible_ack")
        if not isinstance(ack, dict):
            slots.append(
                _unavailable(
                    "visible_ack_absent",
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                    visible_ack=ack,
                )
            )
            continue
        if ack.get("available") is not True:
            slots.append(
                _unavailable(
                    "visible_ack_unavailable",
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                    visible_ack=ack,
                )
            )
            continue

        # Freshness: sample_age_ms on end responsiveness row.
        age_err = _sample_age_reason(erow)
        if age_err:
            slots.append(
                _unavailable(
                    age_err,
                    slot_id=key[0],
                    generation=key[1],
                    gate="input",
                    sample_age_ms=erow.get("sample_age_ms"),
                    freshness_field="sample_age_ms",
                )
            )
            continue

        delta, err = _slot_hist_delta(srow, erow, "input_latency_buckets")
        if err:
            slots.append(_unavailable(err, slot_id=key[0], generation=key[1], gate="input"))
            continue
        fine_delta = None
        if (
            srow.get("input_fine_latency_buckets") is not None
            or erow.get("input_fine_latency_buckets") is not None
        ):
            fine_delta, fine_err = _slot_hist_delta(srow, erow, "input_fine_latency_buckets")
            if fine_err:
                slots.append(
                    _unavailable(fine_err, slot_id=key[0], generation=key[1], gate="input",
                                 field="input_fine_latency_buckets")
                )
                continue
        cons_err = _validate_latency_hist_conservation(
            prefix="input",
            srow=srow,
            erow=erow,
            coarse_delta=delta,
            fine_delta=fine_delta,
            counter_deltas=contained.get("counter_deltas") if contained else None,
        )
        if cons_err:
            slots.append(
                _unavailable(cons_err, slot_id=key[0], generation=key[1], gate="input")
            )
            continue
        bounds = p99_lower_upper_ms(delta, LATENCY_BOUNDS_MS)
        bound_list = erow.get("latency_bound_ms")
        if isinstance(bound_list, list) and bound_list:
            try:
                bounds = p99_lower_upper_ms(delta, tuple(int(x) for x in bound_list))
            except (TypeError, ValueError):
                pass
        fine_bounds = None
        if fine_delta is not None:
            fine_bound_list = erow.get("fine_latency_bound_ms")
            fine_bounds_tuple = FINE_LATENCY_BOUNDS_MS
            if isinstance(fine_bound_list, list) and fine_bound_list:
                try:
                    fine_bounds_tuple = tuple(int(x) for x in fine_bound_list)
                except (TypeError, ValueError):
                    pass
            fine_bounds = p99_lower_upper_ms(fine_delta, fine_bounds_tuple)
        verdict = target_verdict(bounds, target_ms)
        row = {
            "slot_id": key[0],
            "generation": key[1],
            "gate": "input",
            "buckets_delta": delta,
            "p99": bounds,
            "target_ms": target_ms,
            "target_verdict": verdict,
            "input_coverage_complete": coverage,
            "visible_ack_available": True,
            "sample_age_ms": erow.get("sample_age_ms"),
            "freshness_field": "sample_age_ms",
        }
        if fine_bounds is not None:
            row["fine_buckets_delta"] = fine_delta
            row["fine_p99"] = fine_bounds
        if contained:
            row.update(contained)
        if bounds.get("status") == "available":
            row["status"] = "available"
            if verdict != "meet":
                row["reason"] = f"target_{verdict}"
            any_available = True
        else:
            row.update(bounds)
        slots.append(row)

    if not slots:
        return _unavailable("no_matched_slots", gate="input")
    if not any_available:
        return {
            "status": "unavailable",
            "reason": "no_slot_with_available_input_p99",
            "gate": "input",
            "slots": slots,
            "pass_means": PASS_MEANS,
            "target_verdict": "unavailable",
        }
    verdicts = {s.get("target_verdict") for s in slots}
    if verdicts == {"meet"}:
        agg, status = "meet", "available"
    elif "unavailable" in verdicts and not (verdicts - {"unavailable"}):
        agg, status = "unavailable", "unavailable"
    elif "miss" in verdicts:
        agg, status = "miss", "available"
    else:
        agg, status = "unproven", "available"
    return {
        "status": status,
        "gate": "input",
        "slots": slots,
        "target_ms": target_ms,
        "target_verdict": agg,
        "pass_means": PASS_MEANS,
        "reason": None if status == "available" else "no_slot_with_available_input_p99",
    }


GPU_COUNTER_KEYS = (
    "registered_n",
    "completed_n",
    "dropped_n",
    "lost_n",
    "stable_completed_n",
    "stable_completion_intervals",
)


def _gpu_role(row: dict) -> Optional[str]:
    """Infer an observed role from identity and renderer state, not policy flags."""
    if (
        row.get("renderer_present") is True
        and row.get("ingame") is True
        and row.get("scene_state") == 2
        and row.get("draw") is True
        and row.get("full_rate") is True
    ):
        return "focused_full_rate"
    if (
        row.get("renderer_present") is True
        and row.get("ingame") is True
        and row.get("scene_state") == 2
        and row.get("full_rate") is False
    ):
        return "background"
    return None


def _gpu_backend_present(row: dict) -> bool:
    backend = row.get("backend_kind")
    return row.get("renderer_present") is True and isinstance(backend, str) and backend.lower() not in {
        "",
        "absent",
        "cpu",
        "cpu_fallback",
        "pixmap",
    }


def _gpu_headless_row_valid(row: dict) -> bool:
    """Validate an explicitly expected, non-rendering panel slot."""
    if (row.get("renderer_present") is not False
            or row.get("backend_kind") not in (None, "")
            or row.get("draw") is not False
            or row.get("full_rate") is not False):
        return False
    completion = row.get("gpu_completion")
    if not isinstance(completion, dict) or completion.get("enabled") is not True:
        return False
    if completion.get("pending_n") != 0:
        return False
    if any(completion.get(key) != 0 for key in GPU_COUNTER_KEYS):
        return False
    for key in ("stable_completion_interval_buckets", "completion_latency_buckets"):
        values = completion.get(key)
        if not isinstance(values, list) or any(value != 0 for value in values):
            return False
    return True


def _gpu_required_row_epoch_error(row: dict, previous: Optional[dict]) -> Optional[str]:
    """Validate one required GPU row, including every contained epoch."""
    completion = row.get("gpu_completion")
    if not isinstance(completion, dict) or completion.get("enabled") is not True:
        return "gpu_backend_or_completion_unavailable"
    if not _gpu_backend_present(row):
        return "gpu_backend_or_completion_unavailable"
    # A pending callback in an interior observation is normal in-flight work;
    # endpoint pending is rejected by the interval adapter below. Drops and
    # losses are terminal coverage failures at any epoch.
    if completion.get("dropped_n") != 0 or completion.get("lost_n") != 0:
        return "coverage_lost_or_incomplete"
    if completion.get("registration_complete") is not True:
        return "coverage_incomplete"
    # The serializer can publish a normal in-flight callback with coverage
    # incomplete between endpoints.  The endpoint adapter still requires both
    # flags complete, while an interior pending epoch remains admissible.
    pending_n = completion.get("pending_n")
    if (completion.get("completion_coverage_complete") is not True
            and not (completion.get("completion_coverage_complete") is False
                     and isinstance(pending_n, int) and not isinstance(pending_n, bool)
                     and pending_n > 0)):
        return "coverage_incomplete"
    for key in GPU_COUNTER_KEYS:
        value = completion.get(key)
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            return "missing_or_malformed_gpu_counter"
    for key in ("stable_completion_interval_buckets", "completion_latency_buckets"):
        values = completion.get(key)
        if not isinstance(values, list) or any(
            isinstance(value, bool) or not isinstance(value, int) or value < 0 for value in values
        ):
            return "missing_or_malformed_gpu_histogram"
    if previous is None:
        return None
    previous_completion = previous.get("gpu_completion")
    if not isinstance(previous_completion, dict):
        return "missing_gpu_completion"
    if row.get("backend_kind") != previous.get("backend_kind"):
        return "renderer_backend_changed"
    for key in ("attach_n", "detach_n", "backend_change_n"):
        current_value, previous_value = row.get(key), previous.get(key)
        if current_value is None and previous_value is None:
            continue
        if (isinstance(current_value, bool) or not isinstance(current_value, int)
                or isinstance(previous_value, bool) or not isinstance(previous_value, int)
                or current_value < previous_value):
            return "renderer_epoch_invalid"
        if current_value != previous_value or key == "backend_change_n" and current_value > previous_value:
            return "renderer_epoch_changed"
    for key in GPU_COUNTER_KEYS:
        if completion[key] < previous_completion.get(key, -1):
            return "counter_reset"
    for key in ("stable_completion_interval_buckets", "completion_latency_buckets"):
        current_values = completion[key]
        previous_values = previous_completion.get(key)
        if not isinstance(previous_values, list) or len(current_values) != len(previous_values):
            return "missing_or_reset_gpu_histogram"
        if any(current < prior for current, prior in zip(current_values, previous_values)):
            return "missing_or_reset_gpu_histogram"
    return None


def _gpu_role_contract(meta: dict, observed: list[dict], n: Any) -> tuple[Optional[str], Optional[str]]:
    """Check declared policy and every contained renderer-profile epoch."""
    declared = "render_policy" in meta or "render_policy_requested" in meta
    if not declared:
        return None, None  # historical all-slots-must-render behavior
    if (meta.get("render_policy_requested") is not True
            or meta.get("render_policy") not in {"focused-one", "focused-plus-background"}):
        return "invalid_render_policy", None
    if isinstance(n, bool) or not isinstance(n, int) or n < 1:
        return "invalid_declared_slot_count", None
    expected_set = None
    focused_key = None
    previous_rows = {}
    for sample in observed:
        row_map, error = _index_slots(sample.get("renderer_profile"))
        if error:
            return error, None
        assert row_map is not None
        if len(row_map) != n:
            return "renderer_slot_count_mismatch", None
        keys = set(row_map)
        if expected_set is None:
            expected_set = keys
        elif keys != expected_set:
            return "renderer_slot_set_changed", None
        for row in row_map.values():
            age_error = _sample_age_reason(row)
            if age_error:
                return age_error, None
        roles = [_gpu_role(row) for row in row_map.values()]
        current_focused = [key for key, row in row_map.items() if _gpu_role(row) == "focused_full_rate"]
        if focused_key is None:
            focused_key = current_focused[0] if len(current_focused) == 1 else None
        elif current_focused != [focused_key]:
            return "focused_renderer_identity_changed", None
        if meta["render_policy"] == "focused-one":
            if roles.count("focused_full_rate") != 1:
                return "focused_one_role_contract_failed", None
            if sum(_gpu_headless_row_valid(row) for row in row_map.values()) != n - 1:
                return "focused_one_role_contract_failed", None
            if any(role not in ("focused_full_rate", None) for role in roles):
                return "unexpected_renderer_role", None
        elif roles.count("focused_full_rate") != 1 or roles.count("background") != n - 1:
            return "focused_plus_background_role_contract_failed", None
        for key, row, role in zip(row_map, row_map.values(), roles):
            if meta["render_policy"] == "focused-one" and role is None:
                continue
            epoch_error = _gpu_required_row_epoch_error(row, previous_rows.get(key))
            if epoch_error:
                return epoch_error, None
        previous_rows = row_map
    return None, meta["render_policy"]


def evaluate_gpu_completion_intervals(
    meta: dict,
    samples: list[dict],
    *,
    target_ms: float = 40.0,
    background_fps_tolerance: float = 0.25,
) -> dict:
    """Adapt stable callback-completion intervals into a bounded product gate.

    The endpoint is still CPU callback delivery after a prior submit, not GPU
    hardware timing or scanout. Every cumulative counter is differenced over
    the exact observe endpoints; non-zero boundary pending is unavailable.
    """
    unavailable = lambda reason, **extra: _unavailable(  # noqa: E731
        reason,
        gate="gpu",
        target_verdict="unavailable",
        paint_proxy_forbidden=True,
        paint_proxy_used=False,
        **extra,
    )
    if not meta.get("gpu_completion_profile"):
        return unavailable("gpu_completion_profile_disabled")
    if not meta.get("render_profile"):
        return unavailable("render_profile_disabled")
    if not isinstance(meta, dict) or not isinstance(samples, list):
        return unavailable("malformed_gpu_input")
    observed = [sample for sample in samples if isinstance(sample, dict) and sample.get("phase") == "observe"]
    if len(observed) < 2:
        return unavailable("no_observe_samples")
    contract_error, declared_policy = _gpu_role_contract(meta, observed, meta.get("n"))
    if contract_error:
        return unavailable(contract_error, declared_render_policy=meta.get("render_policy"))
    start, end = _observe_pair(samples)
    if start is None or end is None:
        return unavailable("no_observe_samples")
    e0, e1 = start.get("elapsed_s"), end.get("elapsed_s")
    if not isinstance(e0, (int, float)) or not isinstance(e1, (int, float)) or e1 <= e0:
        return unavailable("invalid_observation_endpoints")
    duration_s = float(e1 - e0)
    s_map, e_map, idx_err, disappeared = _pair_slot_maps(
        start.get("renderer_profile"), end.get("renderer_profile")
    )
    if idx_err:
        return unavailable(
            idx_err,
            disappeared=[[a, b] for a, b in disappeared],
            expected_slot_set=sorted(s_map or {}),
        )
    assert s_map is not None and e_map is not None
    if set(s_map) != set(e_map):
        return unavailable(
            "renderer_slot_set_changed",
            expected_slot_set=sorted(s_map),
            observed_slot_set=sorted(e_map),
        )
    if not e_map:
        return unavailable("no_slot_rows", expected_slot_set=[])

    slots = []
    for key in sorted(e_map):
        srow, erow = s_map[key], e_map[key]
        if declared_policy == "focused-one" and _gpu_headless_row_valid(erow):
            slots.append({
                "slot_id": key[0], "generation": key[1], "expected_slot": True,
                "status": "available", "role": "expected_headless",
                "target_verdict": "not_applicable", "gpu_completion_required": False,
                "freshness_field": "sample_age_ms",
            })
            continue
        sg, eg = srow.get("gpu_completion"), erow.get("gpu_completion")
        base = {"slot_id": key[0], "generation": key[1], "expected_slot": True}
        age_err = _sample_age_reason(erow) or _sample_age_reason(srow)
        if age_err:
            slots.append(_unavailable(age_err, **base, freshness_field="sample_age_ms"))
            continue
        if not isinstance(sg, dict) or not isinstance(eg, dict):
            slots.append(_unavailable("missing_gpu_completion", **base))
            continue
        if not eg.get("enabled") or not _gpu_backend_present(erow):
            slots.append(_unavailable("gpu_backend_or_completion_unavailable", **base))
            continue
        role_s, role_e = _gpu_role(srow), _gpu_role(erow)
        if role_s is None or role_e is None or role_s != role_e:
            slots.append(_unavailable("unstable_or_unclassified_renderer_role", **base, role_start=role_s, role_end=role_e))
            continue
        if not all(sg.get(k) is not None and eg.get(k) is not None for k in GPU_COUNTER_KEYS):
            slots.append(_unavailable("missing_gpu_counter", **base))
            continue
        deltas = {}
        reset = False
        for counter in GPU_COUNTER_KEYS:
            delta = subtract_scalar(eg.get(counter), sg.get(counter))
            if delta is None:
                reset = True
                break
            deltas[counter] = delta
        if reset:
            slots.append(_unavailable("counter_reset", **base))
            continue
        pending_start, pending_end = sg.get("pending_n"), eg.get("pending_n")
        if pending_start != 0 or pending_end != 0:
            slots.append(_unavailable(
                "boundary_pending_incomplete",
                **base,
                pending_start=pending_start,
                pending_end=pending_end,
                boundary_accounted=False,
            ))
            continue
        if deltas["dropped_n"] or deltas["lost_n"]:
            slots.append(_unavailable(
                "coverage_lost_or_incomplete", **base,
                dropped_n=deltas["dropped_n"], lost_n=deltas["lost_n"],
            ))
            continue
        if not all(sg.get(k) is True and eg.get(k) is True for k in ("registration_complete", "completion_coverage_complete")):
            slots.append(_unavailable("coverage_incomplete", **base))
            continue
        buckets = subtract_counts(
            eg.get("stable_completion_interval_buckets"),
            sg.get("stable_completion_interval_buckets"),
        )
        if buckets is None:
            slots.append(_unavailable("missing_or_reset_stable_interval_histogram", **base))
            continue
        try:
            interval_bounds = tuple(
                int(x) for x in (eg.get("interval_bound_ms") or INTERVAL_BOUNDS_MS)
            )
        except (TypeError, ValueError):
            slots.append(_unavailable("malformed_interval_bounds", **base))
            continue
        bounds = p99_lower_upper_ms(buckets, interval_bounds)
        if bounds.get("status") != "available":
            slots.append(_unavailable(bounds.get("reason", "interval_unavailable"), **base, p99=bounds))
            continue
        stable_n = deltas["stable_completed_n"]
        interval_n = deltas["stable_completion_intervals"]
        if stable_n <= 0 or interval_n <= 0:
            slots.append(_unavailable("no_stable_completion_samples", **base, stable_completed_n=stable_n, interval_n=interval_n))
            continue
        observed_fps = stable_n / duration_s
        if role_e == "focused_full_rate":
            p99_upper = bounds.get("upper_ms")
            verdict = "meet" if observed_fps >= 40.0 and p99_upper is not None and p99_upper <= target_ms else "miss"
            target = {"min_fps": 40.0, "p99_max_ms": target_ms}
        else:
            low, high = 1.0 - background_fps_tolerance, 1.0 + background_fps_tolerance
            verdict = "meet" if low <= observed_fps <= high else "miss"
            target = {"expected_fps": 1.0, "fps_tolerance": background_fps_tolerance}
        slots.append({
            **base,
            "status": "available",
            "role": role_e,
            "backend_kind": erow.get("backend_kind"),
            "drawing": erow.get("draw"),
            "full_rate": erow.get("full_rate"),
            "stable_completion_interval_buckets_delta": buckets,
            "stable_completed_n_delta": stable_n,
            "gpu_counter_deltas": deltas,
            "pending_delta": pending_end - pending_start,
            "p99": bounds,
            "observed_fps": observed_fps,
            "target": target,
            "target_verdict": verdict,
            "pending_start": pending_start,
            "pending_end": pending_end,
            "boundary_accounted": True,
            "freshness_field": "sample_age_ms",
            "timestamp_semantics": "callback_delivery_cpu_after_prior_submit_not_hw_gpu_or_scanout",
        })

    required_slots = [s for s in slots if s.get("gpu_completion_required", True)]
    verdicts = {s.get("target_verdict") for s in required_slots}
    if any(s.get("status") != "available" for s in required_slots):
        status, aggregate = "unavailable", "unavailable"
    elif verdicts == {"meet"}:
        status, aggregate = "available", "meet"
    elif "miss" in verdicts:
        status, aggregate = "available", "miss"
    else:
        status, aggregate = "available", "unproven"
    return {
        "status": status,
        "reason": None if status == "available" else "no_complete_qualified_stable_interval",
        "gate": "gpu",
        "slots": slots,
        "expected_slot_set": sorted(s_map),
        "declared_render_policy": declared_policy,
        "required_gpu_slot_n": len(required_slots),
        "target_ms": target_ms,
        "target_verdict": aggregate,
        "paint_proxy_forbidden": True,
        "paint_proxy_used": False,
        "presentation_endpoint": "unavailable; callback delivery is not hardware/scanout",
        "full_coverage": status == "available",
        "adapter_implemented": {"completion_latency_diagnostic": True, "stable_completion_interval": True},
    }


def evaluate_gpu(
    meta: dict,
    samples: list[dict],
    *,
    target_ms: float = 40.0,
) -> dict:
    """Product GPU frame/cadence gate (narrow core).

    Never uses host paint as a GPU proxy. Product `--require gpu` requires
    stable completion *interval*/FPS proof. completion_latency_buckets measure
    mainredraw→callback delivery only — scored under diagnostic_completion_latency
    (metric=gpu_completion_latency), never as product meet. Serializer also
    emits stable_completion_interval_buckets; the adapter consumes those raw
    cumulative buckets without treating them as scanout.
    """
    product_note = (
        "completion_latency is mainredraw→callback delivery, not frame/scanout cadence; "
        "fast callback latency does not prove 40fps. "
        "serializer gpu_completion.stable_completion_interval_buckets are adapted "
        "as callback-delivery cadence, not hardware presentation"
    )

    if not meta.get("gpu_completion_profile"):
        return _unavailable(
            "gpu_completion_profile_disabled",
            gate="gpu",
            target_verdict="unavailable",
            paint_proxy_forbidden=True,
            paint_proxy_used=False,
            note=product_note,
            raw_gpu_interval_histograms_in_serializer=True,
            adapter_implemented={
                "completion_latency_diagnostic": False,
                "stable_completion_interval": False,
            },
        )
    if not meta.get("render_profile"):
        return _unavailable(
            "render_profile_disabled",
            gate="gpu",
            target_verdict="unavailable",
            paint_proxy_forbidden=True,
            paint_proxy_used=False,
        )

    start, end = _observe_pair(samples)
    if start is None or end is None:
        return _unavailable(
            "no_observe_samples",
            gate="gpu",
            target_verdict="unavailable",
            paint_proxy_forbidden=True,
            paint_proxy_used=False,
        )

    s_rows, e_rows = start.get("renderer_profile"), end.get("renderer_profile")
    if s_rows is None and e_rows is None:
        return _unavailable(
            "no_input",
            field="renderer_profile",
            gate="gpu",
            target_verdict="unavailable",
            paint_proxy_forbidden=True,
            paint_proxy_used=False,
        )
    s_map, e_map, idx_err, disappeared = _pair_slot_maps(s_rows, e_rows)
    if idx_err:
        return _unavailable(
            idx_err,
            gate="gpu",
            target_verdict="unavailable",
            paint_proxy_forbidden=True,
            paint_proxy_used=False,
            disappeared=[[a, b] for a, b in disappeared],
        )
    assert s_map is not None and e_map is not None
    if not e_map:
        return _unavailable(
            "no_slot_rows",
            gate="gpu",
            target_verdict="unavailable",
            paint_proxy_forbidden=True,
            paint_proxy_used=False,
        )

    slots = []
    any_available = False
    for key, erow in e_map.items():
        srow = s_map.get(key)
        if srow is None:
            slots.append(
                _unavailable("missing_start_slot", slot_id=key[0], generation=key[1], gate="gpu")
            )
            continue
        eg = erow.get("gpu_completion")
        sg = (srow or {}).get("gpu_completion")
        if not isinstance(eg, dict):
            slots.append(
                _unavailable("missing_gpu_completion", slot_id=key[0], generation=key[1], gate="gpu")
            )
            continue
        if not eg.get("enabled", False):
            slots.append(
                _unavailable(
                    "gpu_completion_not_enabled_on_slot",
                    slot_id=key[0],
                    generation=key[1],
                    gate="gpu",
                    paint_proxy_forbidden=True,
                )
            )
            continue

        dropped = int(eg.get("dropped_n") or 0)
        lost = int(eg.get("lost_n") or 0)
        pending = int(eg.get("pending_n") or 0)
        reg_ok = eg.get("registration_complete")
        cov_ok = eg.get("completion_coverage_complete")
        if dropped or lost or pending:
            slots.append(
                _unavailable(
                    "coverage_lost_or_incomplete",
                    slot_id=key[0],
                    generation=key[1],
                    gate="gpu",
                    dropped_n=dropped,
                    lost_n=lost,
                    pending_n=pending,
                    registration_complete=reg_ok,
                    completion_coverage_complete=cov_ok,
                )
            )
            continue
        if reg_ok is None or cov_ok is None:
            slots.append(
                _unavailable(
                    "coverage_flag_absent",
                    slot_id=key[0],
                    generation=key[1],
                    gate="gpu",
                    registration_complete=reg_ok,
                    completion_coverage_complete=cov_ok,
                )
            )
            continue
        if reg_ok is False or cov_ok is False:
            slots.append(
                _unavailable(
                    "coverage_incomplete",
                    slot_id=key[0],
                    generation=key[1],
                    gate="gpu",
                    registration_complete=reg_ok,
                    completion_coverage_complete=cov_ok,
                )
            )
            continue

        # Freshness on renderer_profile row (gpu_completion has no sample_age_ms).
        age_err = _sample_age_reason(erow)
        if age_err:
            slots.append(
                _unavailable(
                    age_err,
                    slot_id=key[0],
                    generation=key[1],
                    gate="gpu",
                    sample_age_ms=erow.get("sample_age_ms") if isinstance(erow, dict) else None,
                    freshness_field="sample_age_ms",
                )
            )
            continue

        if not isinstance(sg, dict):
            slots.append(
                _unavailable(
                    "missing_start_gpu_completion",
                    slot_id=key[0],
                    generation=key[1],
                    gate="gpu",
                )
            )
            continue
        if erow.get("generation") != srow.get("generation"):
            slots.append(
                _unavailable("generation_mismatch", slot_id=key[0], generation=key[1], gate="gpu")
            )
            continue

        delta = subtract_counts(
            eg.get("completion_latency_buckets"),
            sg.get("completion_latency_buckets"),
        )
        if delta is None:
            if eg.get("completion_latency_buckets") is None:
                slots.append(
                    _unavailable("missing_histogram", slot_id=key[0], generation=key[1], gate="gpu")
                )
            else:
                slots.append(
                    _unavailable("counter_reset", slot_id=key[0], generation=key[1], gate="gpu")
                )
            continue

        bound_list = eg.get("interval_bound_ms") or INTERVAL_BOUNDS_MS
        try:
            bounds_t = tuple(int(x) for x in bound_list)
        except (TypeError, ValueError):
            bounds_t = INTERVAL_BOUNDS_MS
        bounds = p99_lower_upper_ms(delta, bounds_t)
        verdict = target_verdict(bounds, target_ms)
        row = {
            "slot_id": key[0],
            "generation": key[1],
            "metric": "gpu_completion_latency",
            "buckets_delta": delta,
            "p99": bounds,
            "target_ms": target_ms,
            "target_verdict": verdict,
            "registration_complete": reg_ok,
            "completion_coverage_complete": cov_ok,
            "sample_age_ms": erow.get("sample_age_ms"),
            "freshness_field": "sample_age_ms",
            "paint_proxy_used": False,
            "not_product_frame_cadence": True,
        }
        if bounds.get("status") == "available":
            row["status"] = "available"
            if verdict != "meet":
                row["reason"] = f"target_{verdict}"
            any_available = True
        else:
            row.update(bounds)
        slots.append(row)

    if not slots:
        diagnostic: dict[str, Any] = {
            "metric": "gpu_completion_latency",
            "status": "unavailable",
            "reason": "no_matched_slots",
            "target_verdict": "unavailable",
            "pass_means": PASS_MEANS,
            "not_product_frame_cadence": True,
        }
    elif not any_available:
        diagnostic = {
            "metric": "gpu_completion_latency",
            "status": "unavailable",
            "reason": "no_slot_with_available_gpu_completion_latency_p99",
            "slots": slots,
            "pass_means": PASS_MEANS,
            "target_verdict": "unavailable",
            "not_product_frame_cadence": True,
        }
    else:
        verdicts = {s.get("target_verdict") for s in slots}
        if verdicts == {"meet"}:
            d_agg, d_status = "meet", "available"
        elif "unavailable" in verdicts and not (verdicts - {"unavailable"}):
            d_agg, d_status = "unavailable", "unavailable"
        elif "miss" in verdicts:
            d_agg, d_status = "miss", "available"
        else:
            d_agg, d_status = "unproven", "available"
        diagnostic = {
            "metric": "gpu_completion_latency",
            "status": d_status,
            "slots": slots,
            "target_ms": target_ms,
            "target_verdict": d_agg,
            "pass_means": PASS_MEANS,
            "not_product_frame_cadence": True,
            "reason": None
            if d_status == "available"
            else "no_slot_with_available_gpu_completion_latency_p99",
        }

    product = evaluate_gpu_completion_intervals(meta, samples, target_ms=target_ms)
    product.update(
        {
            "pass_means": PASS_MEANS,
            "note": product_note,
            "raw_gpu_interval_histograms_in_serializer": True,
            "diagnostic_completion_latency": diagnostic,
        }
    )
    return product


# Approved finish-line resource budgets. Values are bytes and process cores;
# they apply only to active N=1/N=16 cells, not to arbitrary matrix rows.
RESOURCE_BUDGETS = {
    ("tui", 1, "none"): {"median_rss_bytes": 256 * 1024**2,
                           "peak_rss_bytes": 384 * 1024**2, "cpu_cores": None},
    ("tui", 16, "none"): {"median_rss_bytes": 512 * 1024**2,
                            "peak_rss_bytes": 768 * 1024**2, "cpu_cores": 0.5},
    ("panel", 1, "focused-one"): {"median_rss_bytes": 384 * 1024**2,
                                    "peak_rss_bytes": 512 * 1024**2, "cpu_cores": None},
    ("panel", 16, "focused-one"): {"median_rss_bytes": 768 * 1024**2,
                                     "peak_rss_bytes": 1024 * 1024**2, "cpu_cores": 1.0},
    ("panel", 1, "focused-plus-background"): {"median_rss_bytes": 384 * 1024**2,
                                                "peak_rss_bytes": 512 * 1024**2, "cpu_cores": None},
    ("panel", 16, "focused-plus-background"): {"median_rss_bytes": 768 * 1024**2,
                                                 "peak_rss_bytes": 1024 * 1024**2, "cpu_cores": 1.0},
}


def _finite(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and value == value and value not in (float("inf"), float("-inf"))


# Match keys must be present and equal across paired sides. Binary/source build
# digests are side provenance: present and role-correct, but allowed to differ.
# Include scheduling/failure/nav/fine instrumentation — absent must not drop out
# of equality (compare ALL non-side keys; never silently ignore supplied flags).
RESOURCE_MATCH_KEY_FIELDS = (
    "frontend", "n", "workload", "render_policy", "render_policy_requested",
    "single_renderer", "diagnostic_sidecar", "allocation_counting",
    "scheduling_profile", "responsiveness_profile", "responsiveness_fine",
    "render_profile", "gpu_completion_profile", "stack_logging",
    "sustain", "terminal", "terminal_size",
    "nav_pack_sha256", "nav_flags_sha256", "renderer_settings",
    "cache_settings", "catalog_sha256", "feature_flags", "allocator_provenance",
    "client_sources_sha256", "failure_capture", "nav_captures",
    "cpu_fallback", "requested_backend",
)

RESOURCE_SIDE_PROVENANCE_FIELDS = (
    "binary_sha256", "host_sources_sha256",
)

# Single-run resource gate still requires full provenance present on meta.
RESOURCE_PROVENANCE_FIELDS = (
    "nav_pack_sha256", "nav_flags_sha256", "renderer_settings", "cache_settings",
    "catalog_sha256", "feature_flags", "allocator_provenance",
    "host_sources_sha256", "client_sources_sha256", "binary_sha256",
)

VALID_REQUESTED_BACKENDS = frozenset({"cpu_fallback", "gpu", "none"})


def backend_match_fields_from_meta(meta: dict) -> tuple[Any, Any]:
    """Resolve ``cpu_fallback`` + ``requested_backend`` for match keys.

    Legacy defaults apply ONLY when both fields are absent: ``False`` and
    frontend-derived backend (``panel``→``gpu``, otherwise ``none``). Missing
    frontend is not invented as panel.

    Any explicit presence (including null/partial) must be a coherent bool plus
    known backend with matching intent; otherwise ``(None, None)`` fail-closed —
    never coerce non-bool/unknown/contradictory values to GPU.
    """
    if not isinstance(meta, dict):
        return None, None
    has_cpu = "cpu_fallback" in meta
    has_rb = "requested_backend" in meta
    if not has_cpu and not has_rb:
        if meta.get("frontend") == "panel":
            return False, "gpu"
        return False, "none"
    if not has_cpu or not has_rb:
        return None, None
    cpu_fb = meta.get("cpu_fallback")
    rb = meta.get("requested_backend")
    if type(cpu_fb) is not bool:
        return None, None
    if type(rb) is not str or rb not in VALID_REQUESTED_BACKENDS:
        return None, None
    if cpu_fb and rb != "cpu_fallback":
        return None, None
    if (not cpu_fb) and rb == "cpu_fallback":
        return None, None
    return cpu_fb, rb


def resource_match_keys_from_meta(meta: dict) -> dict:
    """Equality-checked match keys only (no binary/host source side digests).

    Never default TUI null render_policy to \"none\" — absent stays None/unavailable.
    """
    out = {key: meta.get(key) for key in RESOURCE_MATCH_KEY_FIELDS}
    cpu_fb, rb = backend_match_fields_from_meta(meta if isinstance(meta, dict) else {})
    out["cpu_fallback"] = cpu_fb
    out["requested_backend"] = rb
    return out


def resource_side_provenance_from_meta(meta: dict) -> dict:
    """Role-specific binary/source digests; may differ across paired sides."""
    return {key: meta.get(key) for key in RESOURCE_SIDE_PROVENANCE_FIELDS}


def _resource_match_metadata(meta: dict) -> dict:
    return resource_match_keys_from_meta(meta)


def evaluate_resources(
    meta: dict,
    samples: list[dict],
    *,
    workload_qualification: Optional[dict] = None,
    contaminated: bool = False,
) -> dict:
    """Evaluate bounded RSS/CPU observations without confusing peak and RSS.

    CPU is process user+system counter delta divided by the elapsed monotonic
    sample span. The JSONL ``elapsed_s`` field is the host's monotonic-relative
    timestamp; no machine-wide percentage or nominal observe duration is used.
    """
    base = {"gate": "resources", "final_acceptance_claim": False,
            "pass_means": PASS_MEANS, "accepted_saving": False,
            "cpu_units": "process_cpu_seconds / sampled_monotonic_wall_seconds",
            "match_metadata": _resource_match_metadata(meta),
            "side_provenance": resource_side_provenance_from_meta(meta)
            if isinstance(meta, dict) else {},
            # A label is not evidence; overhead attribution is not implemented.
            "overhead": "unknown"}
    if not isinstance(meta, dict) or not isinstance(samples, list):
        return {**base, "status": "unavailable", "reason": "malformed_resource_input"}
    match_md = base.get("match_metadata") or {}
    if match_md.get("cpu_fallback") is None or match_md.get("requested_backend") is None:
        return {**base, "status": "unavailable", "reason": "invalid_backend_match_fields"}
    qualification_reason = None
    if workload_qualification is None:
        qualification_reason = "missing_workload_qualification"
    elif workload_qualification.get("qualified") is not True:
        qualification_reason = "workload_not_qualified"
    contamination_reason = "contaminated_observation" if contaminated else None
    observed = [row for row in samples if isinstance(row, dict) and row.get("phase") == "observe"]
    if len(observed) < 2:
        return {**base, "status": "unavailable", "reason": "insufficient_observation_boundaries"}
    times = [row.get("elapsed_s") for row in observed]
    if not all(_finite(value) for value in times):
        return {**base, "status": "unavailable", "reason": "invalid_observation_wall_time"}
    if any(float(b) <= float(a) for a, b in zip(times, times[1:])):
        return {**base, "status": "unavailable", "reason": "non_increasing_observation_wall_time"}
    duration = float(times[-1]) - float(times[0])
    if duration <= 0:
        return {**base, "status": "unavailable", "reason": "invalid_observation_wall_span"}
    for field in ("process_cpu_user_s", "process_cpu_system_s"):
        values = [row.get(field) for row in observed]
        if not all(_finite(value) for value in values):
            return {**base, "status": "unavailable", "reason": "invalid_cpu_counter", "field": field}
        if any(float(b) < float(a) for a, b in zip(values, values[1:])):
            return {**base, "status": "unavailable", "reason": "cpu_counter_reset", "field": field}
        if any(float(value) < 0 for value in values):
            return {**base, "status": "unavailable", "reason": "negative_cpu_counter", "field": field}
    rss = [row.get("resident_bytes") for row in observed]
    if not all(_finite(value) for value in rss):
        return {**base, "status": "unavailable", "reason": "invalid_resident_rss"}
    if any(float(value) < 0 for value in rss):
        return {**base, "status": "unavailable", "reason": "negative_resident_rss"}
    # Lifetime peak is intentionally sourced from every phase; it is not the
    # maximum of the steady observation RSS samples.
    peaks = [row.get("peak_resident_bytes") for row in samples if isinstance(row, dict)]
    if not all(_finite(value) for value in peaks):
        return {**base, "status": "unavailable", "reason": "invalid_peak_rss"}
    if any(float(value) < 0 for value in peaks):
        return {**base, "status": "unavailable", "reason": "negative_peak_rss"}
    cpu_seconds = ((float(observed[-1]["process_cpu_user_s"]) - float(observed[0]["process_cpu_user_s"])) +
                   (float(observed[-1]["process_cpu_system_s"]) - float(observed[0]["process_cpu_system_s"])))
    policy = meta.get("render_policy")
    if policy is None and meta.get("frontend") == "tui":
        policy = "none"
    if meta.get("frontend") == "panel" and policy not in {"focused-one", "focused-plus-background"}:
        return {**base, "status": "unavailable", "reason": "unsupported_panel_render_policy",
                "profile": [meta.get("frontend"), meta.get("n"), policy]}
    key = (meta.get("frontend"), meta.get("n"), policy)
    budget = RESOURCE_BUDGETS.get(key)
    if budget is None:
        return {**base, "status": "unavailable", "reason": "unsupported_resource_profile",
                "profile": list(key)}
    if meta.get("workload") != "active":
        return {**base, "status": "unavailable", "reason": "unsupported_resource_workload",
                "profile": list(key)}
    median_rss = statistics.median(rss)
    max_rss = max(rss)
    peak_rss = max(peaks)
    cpu_cores = cpu_seconds / duration
    metrics = {"median_rss": {"value_bytes": median_rss, "budget_bytes": budget["median_rss_bytes"],
                               "target_verdict": "meet" if median_rss <= budget["median_rss_bytes"] else "miss"},
               "peak_rss": {"value_bytes": peak_rss, "budget_bytes": budget["peak_rss_bytes"],
                            "target_verdict": "meet" if peak_rss <= budget["peak_rss_bytes"] else "miss"}}
    if budget["cpu_cores"] is not None:
        metrics["cpu_cores"] = {"value": cpu_cores, "budget": budget["cpu_cores"],
                                 "target_verdict": "meet" if cpu_cores <= budget["cpu_cores"] else "miss"}
    verdict = "miss" if any(m["target_verdict"] == "miss" for m in metrics.values()) else "meet"
    blocked_reason = qualification_reason or contamination_reason
    if blocked_reason is None:
        missing = [field for field in RESOURCE_PROVENANCE_FIELDS if meta.get(field) is None]
        if missing:
            blocked_reason = "missing_resource_provenance"
    if blocked_reason is None:
        blocked_reason = "overhead_unknown"
    return {**base, "status": "unavailable" if blocked_reason else "available",
            "reason": blocked_reason, "target_verdict": "unavailable" if blocked_reason else verdict,
            "observation_s": duration,
            "sample_n": len(observed), "cpu_seconds": cpu_seconds, "cpu_cores": cpu_cores,
            "resident_median_bytes": median_rss, "resident_max_bytes": max_rss,
            "peak_resident_bytes": peak_rss, "metrics": metrics, "budget": budget,
            "contaminated": contaminated}


def compare_matched_runs(candidate: dict, reference: dict, *, cpu_margin: float = 0.05) -> dict:
    """Return diagnostic paired deltas; one pair never establishes significance."""
    out = {"status": "inconclusive", "accepted_saving": False,
           "pass_means": PASS_MEANS}
    if not isinstance(candidate, dict) or not isinstance(reference, dict):
        out["reason"] = "missing_matched_run"
        return out
    if candidate.get("status") != "available" or reference.get("status") != "available":
        out["reason"] = "unavailable_matched_run"
        return out
    def contaminated(run: dict) -> bool:
        window = run.get("observation_window")
        return bool(run.get("contaminated") or (window or {}).get("contaminated"))
    if contaminated(candidate) or contaminated(reference):
        out["reason"] = "contaminated_matched_run"
        return out
    def _side_prov(run: dict) -> dict:
        side = run.get("side_provenance")
        if isinstance(side, dict):
            return side
        # Backward-compatible: older callers left digests inside match_metadata.
        metadata = run.get("match_metadata")
        if isinstance(metadata, dict):
            return {field: metadata.get(field) for field in RESOURCE_SIDE_PROVENANCE_FIELDS}
        return {}

    # Core resource fixture keys must be present (not side digests).
    match_prov_keys = (
        "nav_pack_sha256", "nav_flags_sha256", "renderer_settings",
        "cache_settings", "catalog_sha256", "feature_flags",
        "allocator_provenance", "client_sources_sha256",
    )
    for run in (candidate, reference):
        metadata = run.get("match_metadata")
        if (not isinstance(metadata, dict) or
                any(metadata.get(field) in (None, "", {}, [])
                    for field in match_prov_keys)):
            out["reason"] = "missing_match_provenance"
            return out
        side = _side_prov(run)
        if any(side.get(field) in (None, "", {}, [])
               for field in RESOURCE_SIDE_PROVENANCE_FIELDS):
            out["reason"] = "missing_side_provenance"
            return out

    def _match_only(md: Any) -> dict:
        """All non-side keys — full equality guard minus binary/source digests."""
        if not isinstance(md, dict):
            return {}
        side = set(RESOURCE_SIDE_PROVENANCE_FIELDS)
        return {k: v for k, v in md.items() if k not in side}

    for run in (candidate, reference):
        md = run.get("match_metadata")
        if not isinstance(md, dict):
            continue
        # Synthetics may omit both keys (pre-backend tooling). Once either key is
        # present, values must be coherent — never treat None/malformed as GPU.
        if "cpu_fallback" not in md and "requested_backend" not in md:
            continue
        cpu_fb = md.get("cpu_fallback")
        rb = md.get("requested_backend")
        if cpu_fb is None or rb is None or type(cpu_fb) is not bool or (
                type(rb) is not str or rb not in VALID_REQUESTED_BACKENDS):
            out["reason"] = "invalid_backend_match_fields"
            return out
        if cpu_fb and rb != "cpu_fallback":
            out["reason"] = "invalid_backend_match_fields"
            return out
        if (not cpu_fb) and rb == "cpu_fallback":
            out["reason"] = "invalid_backend_match_fields"
            return out

    if _match_only(candidate.get("match_metadata")) != _match_only(reference.get("match_metadata")):
        out["reason"] = "mismatched_provenance_or_settings"
        return out
    if (candidate.get("overhead", "unknown") != "measured" or
            reference.get("overhead", "unknown") != "measured"):
        out["reason"] = "overhead_unknown"
        return out
    cpu_change = candidate["cpu_cores"] - reference["cpu_cores"]
    out.update({"intended_rss_change_bytes": candidate["resident_median_bytes"] - reference["resident_median_bytes"],
                "cpu_change": cpu_change, "cpu_margin": cpu_margin,
                "cpu_non_regression": candidate["cpu_cores"] <= reference["cpu_cores"] * (1.0 + cpu_margin),
                "reason": "single_pair_no_variation"})
    return out


def load_run_samples(run_dir: pathlib.Path, *, max_samples: Optional[int] = None) -> list[dict]:
    path = run_dir / "samples.jsonl"
    if not path.is_file():
        raise FileNotFoundError(f"missing {path}")
    out = []
    for obj in iter_jsonl(path):
        out.append(obj)
        if max_samples is not None and len(out) >= max_samples:
            break
    return out


def analyze_run(
    run_dir: pathlib.Path,
    *,
    contamination_from: Any = None,
    qualify: bool = False,
    counting: bool = False,
    diagnostics: bool = False,
) -> dict:
    run_dir = pathlib.Path(run_dir)
    result: dict[str, Any] = {
        "run": str(run_dir),
        "pass_means": PASS_MEANS,
        "final_acceptance_claim": False,
        "resource_renderer_expanded_adapters": "resource RSS/CPU adapters are diagnostic; no final acceptance",
    }
    meta_path = run_dir / "metadata.json"
    if not meta_path.is_file():
        result["error"] = "missing metadata.json"
        result["gates"] = {
            "resources": _unavailable("missing_metadata"),
            "scheduling": _unavailable("missing_metadata"),
            "decode": _unavailable("missing_metadata"),
            "input": _unavailable("missing_metadata"),
            "gpu": _unavailable("missing_metadata"),
        }
        return result

    try:
        meta = load_json(meta_path)
    except (OSError, json.JSONDecodeError) as exc:
        result["error"] = f"metadata unreadable: {exc}"
        result["gates"] = {
            "resources": _unavailable("bad_metadata"),
            "scheduling": _unavailable("bad_metadata"),
            "decode": _unavailable("bad_metadata"),
            "input": _unavailable("bad_metadata"),
            "gpu": _unavailable("bad_metadata"),
        }
        return result
    if not isinstance(meta, dict):
        result["error"] = "metadata not object"
        result["gates"] = {"resources": _unavailable("bad_metadata")}
        result["gates"].update({
            k: _unavailable("bad_metadata") for k in ("scheduling", "decode", "input", "gpu")
        })
        return result

    result["metadata_flags"] = {
        "scheduling_profile": bool(meta.get("scheduling_profile")),
        "responsiveness_profile": bool(meta.get("responsiveness_profile")),
        "render_profile": bool(meta.get("render_profile")),
        "gpu_completion_profile": bool(meta.get("gpu_completion_profile")),
        "n": meta.get("n"),
        "workload": meta.get("workload"),
        "frontend": meta.get("frontend"),
    }

    try:
        samples = load_run_samples(run_dir)
    except (OSError, ValueError, FileNotFoundError) as exc:
        result["error"] = str(exc)
        result["observation_window"] = _unavailable("samples_unreadable")
        result["gates"] = {
            "resources": _unavailable("samples_unreadable"),
            "scheduling": _unavailable("samples_unreadable"),
            "decode": _unavailable("samples_unreadable"),
            "input": _unavailable("samples_unreadable"),
            "gpu": _unavailable("samples_unreadable"),
        }
        return result

    result["observation_window"] = observe_window(
        meta, samples, contamination_from=contamination_from
    )
    if qualify:
        try:
            import qualify_control as qc

            result["workload_qualification"] = qc.qualify(
                run_dir, counting=counting, diagnostics=diagnostics
            )
        except Exception as exc:  # noqa: BLE001 — surface adapter failure honestly
            result["workload_qualification"] = {
                "qualified": False,
                "errors": [f"qualify_control adapter error: {exc}"],
                "pass_means": "workload qualification only; not performance acceptance",
            }

    result["gates"] = {
        "resources": evaluate_resources(
            meta, samples,
            workload_qualification=result.get("workload_qualification"),
            contaminated=bool(result["observation_window"].get("contaminated")),
        ),
        # The scheduling gate is per-slot only. Keep legacy process-wide groups
        # as a separately named diagnostic; they must not satisfy this gate.
        "scheduling": evaluate_scheduling_slots(meta, samples),
        "scheduling_process_wide": evaluate_scheduling(meta, samples),
        "decode": evaluate_decode(meta, samples),
        "input": evaluate_input(meta, samples),
        "gpu": evaluate_gpu(meta, samples),
    }

    return result


def gate_satisfies_require(gate_result: dict) -> bool:
    """Require path: available AND target meet. Fail closed otherwise."""
    if not isinstance(gate_result, dict):
        return False
    if gate_result.get("status") != "available":
        return False
    return gate_result.get("target_verdict") == "meet"


def main(argv: Optional[list[str]] = None) -> int:
    p = argparse.ArgumentParser(
        description=(
            "Bounded offline histogram / observation-window core. "
            "Never claims final performance acceptance."
        )
    )
    p.add_argument("run_dir", type=pathlib.Path, help="completed diagnostic run directory")
    p.add_argument(
        "--inspect",
        action="store_true",
        help="print partial analysis; exit 0 even when gates are unavailable",
    )
    p.add_argument(
        "--require",
        default="",
        help="comma list: resources,scheduling,decode,input,gpu — exit 1 if unavailable or target unproven",
    )
    p.add_argument(
        "--contamination-from",
        default=None,
        help="UTC timestamp (ISO or unix) for contamination intersection",
    )
    p.add_argument(
        "--qualify",
        action="store_true",
        help="also run qualify_control workload qualification adapter",
    )
    p.add_argument("--counting", action="store_true", help="qualify_control --counting")
    p.add_argument("--diagnostics", action="store_true", help="qualify_control --diagnostics")
    p.add_argument("-o", "--output", type=pathlib.Path, default=None)
    args = p.parse_args(argv)

    result = analyze_run(
        args.run_dir,
        contamination_from=args.contamination_from,
        qualify=args.qualify,
        counting=args.counting,
        diagnostics=args.diagnostics,
    )
    text = json.dumps(result, indent=2, sort_keys=True)
    if args.output:
        try:
            # Immutable evidence: exclusive create; never overwrite.
            with args.output.open("x", encoding="utf-8") as fh:
                fh.write(text + "\n")
        except FileExistsError:
            print(
                f"refusing to overwrite existing output: {args.output}",
                file=sys.stderr,
            )
            return 1
        except OSError as exc:
            print(f"output write failed: {exc}", file=sys.stderr)
            return 1
    else:
        print(text)

    require = [x.strip() for x in args.require.split(",") if x.strip()]
    if require:
        gates = result.get("gates") or {}
        failures = []
        for name in require:
            gr = gates.get(name)
            if gr is None:
                failures.append(f"{name}: missing_gate")
            elif not gate_satisfies_require(gr):
                failures.append(
                    f"{name}: status={gr.get('status')} "
                    f"verdict={gr.get('target_verdict')} reason={gr.get('reason')}"
                )
        result_req = {
            "required": require,
            "failures": failures,
            "final_acceptance_claim": False,
            "pass_means": PASS_MEANS,
        }
        if failures:
            # Always exit 1 on require failure; still printed full result above.
            print(json.dumps(result_req, indent=2, sort_keys=True), file=sys.stderr)
            return 1
        # Even if all meet, this core does not claim final acceptance.
        print(json.dumps(result_req, indent=2, sort_keys=True), file=sys.stderr)
        return 0

    # Default / --inspect: partial OK
    return 0


if __name__ == "__main__":
    sys.exit(main())
