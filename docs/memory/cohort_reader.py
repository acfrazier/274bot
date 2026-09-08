"""Fail-closed reader for the frozen responsiveness cohort sidecar envelope."""
from __future__ import annotations

import json
import math
import pathlib
from typing import Any, Optional

SCHEMA_VERSION = 1
CLOCK_DOMAIN = "responsiveness_process_mono"
TAIL_NAME = "DEFAULT_TAIL_NS"
COARSE_BOUNDS_MS = (5, 10, 20, 25, 40, 50, 100, 250, 500, 1000)
FINE_BOUNDS_MS = tuple(range(1, 101))
OUTCOMES = {"Completed", "Canceled", "Lost", "Dropped"}
LOSS_REASONS = {"Capacity", "LateEvent", "GenerationMismatch", "MissingIdentity", "Incomplete", "MalformedTimestamp"}
SURFACES = {"Decode", "Panel", "Tui"}


def unavailable(reason: str, **extra: Any) -> dict:
    out = {"status": "unavailable", "reason": reason, "target_verdict": "unavailable", "gate": "cohort"}
    out.update(extra)
    return out


def _uint(v: Any) -> bool:
    return type(v) is int and v >= 0


def _finite(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(float(v))


def _same(a: Any, b: Any) -> bool:
    return type(a) is type(b) and a == b


def _bound_sidecar_ref(recorded: Any, run_dir: pathlib.Path, expected: pathlib.Path) -> bool:
    """Accept the archived path or its native-Windows spelling, never basename-only."""
    if not isinstance(recorded, str):
        return False
    if recorded == str(expected):
        return True
    normalized = recorded.replace("\\", "/").rstrip("/")
    return normalized.endswith("/" + run_dir.name + "/" + expected.name)


def _bounds(v: Any, expected: tuple[int, ...]) -> bool:
    return isinstance(v, list) and len(v) == len(expected) and all(_same(x, e) for x, e in zip(v, expected))


def p99_ns(durations: list[int], bounds: tuple[int, ...] = COARSE_BOUNDS_MS) -> dict:
    if not durations:
        return unavailable("empty_cohort")
    if any(not _uint(x) for x in durations):
        return unavailable("malformed_duration")
    buckets = [0] * (len(bounds) + 1)
    for ns in durations:
        placed = False
        for i, ms in enumerate(bounds):
            if ns <= ms * 1_000_000:
                buckets[i] += 1
                placed = True
                break
        if not placed:
            buckets[-1] += 1
    n = len(durations)
    cum = 0
    for i, count in enumerate(buckets):
        cum += count
        if cum * 100 >= n * 99:
            if i == len(bounds):
                return unavailable("overflow_bucket", sample_n=n, overflow_count=count,
                                   lower_ms=bounds[-1], upper_ms=None, buckets=buckets)
            return {"status": "available", "sample_n": n, "bucket_index": i,
                    "lower_ms": 0 if i == 0 else bounds[i - 1], "upper_ms": bounds[i],
                    "precise_percentile": False, "buckets": buckets}
    return unavailable("histogram_exhausted")


def _validate_boundaries(value: Any) -> Optional[tuple[int, int, int]]:
    if not isinstance(value, dict):
        return None
    s, e, t = value.get("start_mono_ns"), value.get("end_mono_ns"), value.get("tail_ns")
    if not all(_uint(x) for x in (s, e, t)) or s >= e or t == 0:
        return None
    if e > (1 << 64) - 1 - t:
        return None
    return s, e, t


def _event(obj: Any, start: int, end: int, tail: int) -> tuple[dict | None, str | None]:
    if not isinstance(obj, dict) or set(obj) != {"id", "complete_mono_ns", "outcome"}:
        return None, "malformed_event_record"
    ident = obj["id"]
    required = {"slot_id", "generation", "sequence", "start_mono_ns", "surface"}
    if not isinstance(ident, dict) or set(ident) != required:
        return None, "missing_event_identity"
    if not all(_uint(ident.get(k)) for k in ("slot_id", "generation", "sequence", "start_mono_ns")):
        return None, "malformed_event_identity"
    if ident["surface"] not in SURFACES or obj["outcome"] not in OUTCOMES:
        return None, "unknown_event_schema"
    st, complete = ident["start_mono_ns"], obj["complete_mono_ns"]
    if not (start <= st < end):
        return None, "event_start_out_of_window"
    if obj["outcome"] == "Completed":
        if not _uint(complete) or complete < st or complete > end + tail:
            return None, "malformed_completion_timestamp"
        obj = dict(obj)
        obj["duration_ns"] = complete - st
    elif complete is not None:
        return None, "malformed_terminal_timestamp"
    return obj, None


def _loss(obj: Any) -> str | None:
    if not isinstance(obj, dict):
        return "malformed_loss_receipt"
    required = {"sequence", "slot_id", "generation", "start_mono_ns", "surface", "outcome", "reason"}
    if set(obj) != required:
        return "malformed_loss_receipt"
    if not _uint(obj["sequence"]) or not _uint(obj["slot_id"]):
        return "malformed_loss_identity"
    if obj["generation"] is not None and not _uint(obj["generation"]):
        return "malformed_loss_identity"
    if obj["start_mono_ns"] is not None and not _uint(obj["start_mono_ns"]):
        return "malformed_loss_identity"
    if obj["surface"] not in SURFACES or obj["outcome"] not in OUTCOMES or obj["reason"] not in LOSS_REASONS:
        return "unknown_loss_schema"
    return None


def read_cohort(run_dir: pathlib.Path, meta: dict, samples: list[dict], gate: str) -> dict:
    """Read one endpoint cohort (``decode`` or ``input``) from a bound run."""
    path = pathlib.Path(run_dir) / "samples.cohort.jsonl"
    if not path.is_file():
        return unavailable("cohort_schema_missing", gate=gate)
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
        objects = [json.loads(line) for line in lines if line.strip()]
    except (OSError, UnicodeError, json.JSONDecodeError):
        return unavailable("cohort_sidecar_unreadable", gate=gate)
    if not objects:
        return unavailable("cohort_header_missing", gate=gate)
    header = objects[0]
    if not isinstance(header, dict) or header.get("record") != "cohort-header":
        return unavailable("cohort_header_missing", gate=gate)
    if (header.get("schema_version") != SCHEMA_VERSION or header.get("clock_domain") != CLOCK_DOMAIN
            or not _uint(header.get("observe_ns")) or not _uint(header.get("capacity"))
            or header.get("capacity") == 0 or not isinstance(header.get("frontend"), str)):
        return unavailable("unknown_cohort_schema", gate=gate)
    boundaries = _validate_boundaries(header.get("boundaries"))
    if boundaries is None or header.get("tail_name") != TAIL_NAME or header.get("tail_ns") != boundaries[2]:
        return unavailable("malformed_cohort_boundaries", gate=gate)
    start, end, tail = boundaries
    if header.get("observe_ns") != end - start:
        return unavailable("observe_duration_mismatch", gate=gate)
    if not _bound_sidecar_ref(header.get("sidecar_path"), pathlib.Path(run_dir), path):
        return unavailable("sidecar_provenance_mismatch", gate=gate)
    if header.get("frontend") != meta.get("frontend"):
        return unavailable("cohort_frontend_mismatch", gate=gate)
    population = header.get(gate + "_population")
    if gate == "decode":
        if not isinstance(population, dict) or population.get("kind") != "all-run-slots" or not _uint(population.get("n")) or population["n"] != meta.get("n"):
            return unavailable("decode_population_mismatch", gate=gate)
        expected = None
    else:
        if not isinstance(population, dict):
            return unavailable("input_population_missing", gate=gate)
        if population.get("kind") == "focused-one":
            expected = {(x, None) for x in population.get("slots", [])} if isinstance(population.get("slots"), list) else None
            if expected is None or not expected or any(not _uint(x) for x, _ in expected):
                return unavailable("input_population_malformed", gate=gate)
        elif population.get("kind") == "tui-endpoint":
            expected = None
            if meta.get("frontend") != "tui":
                return unavailable("input_endpoint_mismatch", gate=gate)
        else:
            return unavailable("unsupported_input_population", gate=gate)
    records: list[dict] = []
    losses: list[dict] = []
    cursor = 0
    last_loss = 0
    last_overflow = 0
    terminal = None
    seen_sequences: set[tuple[Any, ...]] = set()
    for obj in objects[1:]:
        if not isinstance(obj, dict):
            return unavailable("malformed_cohort_record", gate=gate)
        kind = obj.get("record")
        if kind == "cohort-batch":
            if obj.get("schema_version") != SCHEMA_VERSION or type(obj.get("complete")) is not bool or _validate_boundaries(obj.get("boundaries")) != boundaries:
                return unavailable("batch_provenance_mismatch", gate=gate)
            if not _uint(obj.get("next_cursor")) or obj["next_cursor"] < cursor:
                return unavailable("cursor_regression", gate=gate)
            if not _uint(obj.get("loss_count")) or not _uint(obj.get("journal_overflow_n")) or obj["loss_count"] < last_loss or obj["journal_overflow_n"] < last_overflow:
                return unavailable("loss_counter_regression", gate=gate)
            batch_records, batch_losses = obj.get("records"), obj.get("losses")
            if not isinstance(batch_records, list) or not isinstance(batch_losses, list):
                return unavailable("malformed_batch", gate=gate)
            for raw in batch_records:
                rec, err = _event(raw, start, end, tail)
                if err:
                    return unavailable(err, gate=gate)
                key = (rec["id"]["slot_id"], rec["id"]["generation"], rec["id"]["sequence"], rec["id"]["surface"])
                if key in seen_sequences:
                    return unavailable("duplicate_event_identity", gate=gate)
                seen_sequences.add(key); records.append(rec)
            for raw in batch_losses:
                err = _loss(raw)
                if err: return unavailable(err, gate=gate)
                losses.append(raw)
            cursor, last_loss, last_overflow = obj["next_cursor"], obj["loss_count"], obj["journal_overflow_n"]
        elif kind == "cohort-terminal":
            if terminal is not None or obj.get("schema_version") != SCHEMA_VERSION or _validate_boundaries(obj.get("boundaries")) != boundaries:
                return unavailable("malformed_terminal", gate=gate)
            terminal = obj
        else:
            return unavailable("unknown_cohort_record", gate=gate)
    if terminal is None or terminal.get("terminal") is not True or terminal.get("frontend") != header.get("frontend"):
        return unavailable("cohort_terminal_missing", gate=gate)
    if not all(_uint(terminal.get(k)) for k in ("records_n", "losses_n", "pending_n")):
        return unavailable("malformed_terminal_counts", gate=gate)
    if terminal["records_n"] != len(records) or terminal["losses_n"] < len(losses) or terminal["pending_n"] != 0:
        return unavailable("terminal_count_mismatch", gate=gate)
    if terminal.get("available") is not True or losses or last_overflow:
        return unavailable("cohort_accounting_unavailable", gate=gate, records_n=len(records), losses_n=len(losses))
    selected = [r for r in records if r["id"]["surface"] == ("Decode" if gate == "decode" else ("Tui" if header.get("frontend") == "tui" else "Panel"))]
    if expected is not None:
        actual_slots = {(r["id"]["slot_id"], r["id"]["generation"]) for r in selected}
        actual_ids = {x for x, _ in actual_slots}
        if gate == "decode":
            if len(actual_ids) != meta.get("n"):
                return unavailable("declared_population_incomplete", gate=gate)
        elif actual_ids != {x for x, _ in expected}:
            return unavailable("declared_population_incomplete", gate=gate)
    # Additive references on samples/qualification are provenance, not a second
    # source of boundaries. If present, every reference must agree exactly.
    for sample in samples:
        ref = sample.get("cohort") if isinstance(sample, dict) else None
        if ref is None:
            continue
        if not isinstance(ref, dict) or ref.get("present") is not True or ref.get("schema_version") != SCHEMA_VERSION:
            return unavailable("cohort_sample_ref_malformed", gate=gate)
        if (not _bound_sidecar_ref(ref.get("sidecar_path"), pathlib.Path(run_dir), path)
                or _validate_boundaries(ref.get("boundaries")) != boundaries):
            return unavailable("cohort_sample_ref_mismatch", gate=gate)
    if not selected:
        return unavailable("empty_declared_population", gate=gate)
    if any(r["outcome"] != "Completed" for r in selected):
        return unavailable("noncompleted_member", gate=gate)
    durations = [r["duration_ns"] for r in selected]
    p99 = p99_ns(durations)
    fine = p99_ns(durations, FINE_BOUNDS_MS)
    verdict = "meet" if fine.get("status") == "available" and fine.get("upper_ms") <= 100 else ("unavailable" if fine.get("status") != "available" else "miss")
    return {"status": "available" if verdict != "unavailable" else "unavailable", "gate": gate + "_cohort", "target_verdict": verdict, "reason": None if verdict != "unavailable" else fine.get("reason"), "population_n": len(selected), "records_n": len(records), "losses_n": len(losses), "boundaries": {"start_mono_ns": start, "end_mono_ns": end, "tail_ns": tail}, "p99": p99, "fine_p99": fine, "events": selected}
