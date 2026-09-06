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
import pathlib
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
LATENCY_BOUNDS_MS = (5, 10, 20, 25, 40, 50, 100, 250, 500, 1000)

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

        # Coverage / integrity on end snapshot (cumulative). Drops/cancels/lost
        # or incomplete/absent coverage cannot pass.
        dropped = int(erow.get("decode_dropped_n") or 0)
        canceled = int(erow.get("decode_canceled_n") or 0)
        lost = int(erow.get("decode_lost_n") or 0)
        pending = int(erow.get("decode_pending_n") or 0)
        coverage = erow.get("decode_coverage_complete")
        if dropped or canceled or lost or pending:
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
        if coverage is None:
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
        if coverage is False:
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
        bounds = p99_lower_upper_ms(delta, LATENCY_BOUNDS_MS)
        # Prefer host-emitted bounds list when present.
        bound_list = erow.get("latency_bound_ms")
        if isinstance(bound_list, list) and bound_list:
            try:
                bounds = p99_lower_upper_ms(delta, tuple(int(x) for x in bound_list))
            except (TypeError, ValueError):
                pass
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

        dropped = int(erow.get("input_dropped_n") or 0)
        canceled = int(erow.get("input_canceled_n") or 0)
        lost = int(erow.get("input_lost_n") or 0)
        pending = int(erow.get("input_pending_n") or 0)
        coverage = erow.get("input_coverage_complete")
        if dropped or canceled or lost or pending:
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
        if coverage is None:
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
        if coverage is False:
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
        bounds = p99_lower_upper_ms(delta, LATENCY_BOUNDS_MS)
        bound_list = erow.get("latency_bound_ms")
        if isinstance(bound_list, list) and bound_list:
            try:
                bounds = p99_lower_upper_ms(delta, tuple(int(x) for x in bound_list))
            except (TypeError, ValueError):
                pass
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

    verdicts = {s.get("target_verdict") for s in slots}
    if any(s.get("status") != "available" for s in slots):
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
        "resource_renderer_expanded_adapters": "not_in_this_core; next card if needed",
    }
    meta_path = run_dir / "metadata.json"
    if not meta_path.is_file():
        result["error"] = "missing metadata.json"
        result["gates"] = {
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
            "scheduling": _unavailable("bad_metadata"),
            "decode": _unavailable("bad_metadata"),
            "input": _unavailable("bad_metadata"),
            "gpu": _unavailable("bad_metadata"),
        }
        return result
    if not isinstance(meta, dict):
        result["error"] = "metadata not object"
        result["gates"] = {
            k: _unavailable("bad_metadata") for k in ("scheduling", "decode", "input", "gpu")
        }
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
            "scheduling": _unavailable("samples_unreadable"),
            "decode": _unavailable("samples_unreadable"),
            "input": _unavailable("samples_unreadable"),
            "gpu": _unavailable("samples_unreadable"),
        }
        return result

    result["observation_window"] = observe_window(
        meta, samples, contamination_from=contamination_from
    )
    result["gates"] = {
        "scheduling": evaluate_scheduling(meta, samples),
        "decode": evaluate_decode(meta, samples),
        "input": evaluate_input(meta, samples),
        "gpu": evaluate_gpu(meta, samples),
    }

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
        help="comma list: scheduling,decode,input,gpu — exit 1 if unavailable or target unproven",
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
