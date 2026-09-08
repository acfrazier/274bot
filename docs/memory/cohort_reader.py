"""Fail-closed reader for the frozen responsiveness cohort sidecar envelope.

Source contract: host::responsiveness_cohort + host-play CohortPublisher
(serialize_qualification_boundary, attach_cohort_* , arm/drain/finish).

Never opens files outside the bound run directory. Malformed inputs yield
status=unavailable — never exceptions, never quiet-window inference.
"""
from __future__ import annotations

import json
import math
import pathlib
import re
from typing import Any, Optional

SCHEMA_VERSION = 1
CLOCK_DOMAIN = "responsiveness_process_mono"
TAIL_NAME = "DEFAULT_TAIL_NS"
COARSE_BOUNDS_MS = (5, 10, 20, 25, 40, 50, 100, 250, 500, 1000)
FINE_BOUNDS_MS = tuple(range(1, 101))
OUTCOMES = frozenset({"Completed", "Canceled", "Lost", "Dropped"})
LOSS_REASONS = frozenset({
    "Capacity", "LateEvent", "GenerationMismatch", "MissingIdentity",
    "Incomplete", "MalformedTimestamp",
})
SURFACES = frozenset({"Decode", "Panel", "Tui"})
TARGET_MS = 100

# Windows drive path: C:\... or C:/...
_WIN_ABS = re.compile(r"^[A-Za-z]:[\\/]")


def unavailable(reason: str, **extra: Any) -> dict:
    out = {
        "status": "unavailable",
        "reason": reason,
        "target_verdict": "unavailable",
        "gate": "cohort",
    }
    out.update(extra)
    return out


def _uint(v: Any) -> bool:
    """Strict non-negative integer; bool is not an integer."""
    return type(v) is int and v >= 0


def _u16_schema(v: Any) -> bool:
    return type(v) is int and 0 <= v <= 0xFFFF


def _finite(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(float(v))


def _norm_sep(path: str) -> str:
    return path.replace("\\", "/").rstrip("/")


def slot_id_for(username: str) -> int:
    """FNV-1a 64 — matches host::responsiveness_profile::slot_id_for."""
    h = 0xCBF29CE484222325
    for b in username.encode("utf-8"):
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def p99_ns(durations: list[int], bounds: tuple[int, ...] = COARSE_BOUNDS_MS) -> dict:
    """Bucket ns durations against inclusive ms upper edges (same rule as host)."""
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
                return unavailable(
                    "overflow_bucket",
                    sample_n=n,
                    overflow_count=count,
                    lower_ms=bounds[-1],
                    upper_ms=None,
                    buckets=buckets,
                )
            return {
                "status": "available",
                "sample_n": n,
                "bucket_index": i,
                "lower_ms": 0 if i == 0 else bounds[i - 1],
                "upper_ms": bounds[i],
                "precise_percentile": False,
                "buckets": buckets,
            }
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


def _is_absolute_path_str(recorded: str) -> bool:
    if recorded.startswith("/") or recorded.startswith("//"):
        return True
    return bool(_WIN_ABS.match(recorded))


def _bound_sidecar_ref(
    recorded: Any,
    run_dir: pathlib.Path,
    expected: pathlib.Path,
    *,
    canonical_recorded: Optional[str] = None,
) -> tuple[bool, Optional[str]]:
    """Map a native/archive sidecar claim onto the bound run file only.

    Never opens ``recorded``. Basename-only and paths that do not name this
    run directory immediately above the expected leaf are rejected. When
    ``canonical_recorded`` is set, every later claim must match it exactly
    after separator normalization (consistent provenance).
    """
    if not isinstance(recorded, str) or not recorded.strip():
        return False, None
    if not _is_absolute_path_str(recorded):
        return False, None
    norm = _norm_sep(recorded)
    if canonical_recorded is not None and norm != canonical_recorded:
        return False, norm
    exp_name = expected.name
    run_name = run_dir.name
    parts = [p for p in norm.split("/") if p not in ("", ".")]
    if len(parts) < 2:
        return False, norm
    if parts[-1] != exp_name:
        return False, norm
    if parts[-2] != run_name:
        return False, norm
    # Archive file must exist at the bound location; never chase recorded.
    if not expected.is_file():
        return False, norm
    # Exact archive path is always acceptable when the leaf binding holds.
    return True, norm


def _event(obj: Any, start: int, end: int, tail: int) -> tuple[Optional[dict], Optional[str]]:
    if not isinstance(obj, dict):
        return None, "malformed_event_record"
    if set(obj.keys()) != {"id", "complete_mono_ns", "outcome"}:
        return None, "malformed_event_record"
    ident = obj["id"]
    required = {"slot_id", "generation", "sequence", "start_mono_ns", "surface"}
    if not isinstance(ident, dict) or set(ident.keys()) != required:
        return None, "missing_event_identity"
    if not all(_uint(ident.get(k)) for k in ("slot_id", "generation", "sequence", "start_mono_ns")):
        return None, "malformed_event_identity"
    surface = ident["surface"]
    if not isinstance(surface, str) or surface not in SURFACES:
        return None, "unknown_event_schema"
    outcome = obj["outcome"]
    if not isinstance(outcome, str) or outcome not in OUTCOMES:
        return None, "unknown_event_schema"
    st = ident["start_mono_ns"]
    complete = obj["complete_mono_ns"]
    if not (start <= st < end):
        return None, "event_start_out_of_window"
    if outcome == "Completed":
        if not _uint(complete) or complete < st or complete > end + tail:
            return None, "malformed_completion_timestamp"
        out = dict(obj)
        out["duration_ns"] = complete - st
        return out, None
    if complete is not None:
        return None, "malformed_terminal_timestamp"
    return dict(obj), None


def _loss(obj: Any) -> Optional[str]:
    if not isinstance(obj, dict):
        return "malformed_loss_receipt"
    required = {
        "sequence", "slot_id", "generation", "start_mono_ns",
        "surface", "outcome", "reason",
    }
    if set(obj.keys()) != required:
        return "malformed_loss_receipt"
    if not _uint(obj["sequence"]) or not _uint(obj["slot_id"]):
        return "malformed_loss_identity"
    gen = obj["generation"]
    if gen is not None and not _uint(gen):
        return "malformed_loss_identity"
    start = obj["start_mono_ns"]
    if start is not None and not _uint(start):
        return "malformed_loss_identity"
    if (not isinstance(obj["surface"], str) or obj["surface"] not in SURFACES
            or not isinstance(obj["outcome"], str) or obj["outcome"] not in OUTCOMES
            or not isinstance(obj["reason"], str) or obj["reason"] not in LOSS_REASONS):
        return "unknown_loss_schema"
    return None


def _load_jsonl(path: pathlib.Path) -> tuple[Optional[list], Optional[str]]:
    if not path.is_file():
        return None, "missing"
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        return None, "unreadable"
    rows: list = []
    for line in text.splitlines():
        if not line.strip():
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            return None, "truncated_or_invalid_json"
    return rows, None


def _load_qualification(run_dir: pathlib.Path) -> tuple[Optional[list[dict]], Optional[str]]:
    path = pathlib.Path(run_dir) / "samples.qualification.jsonl"
    rows, err = _load_jsonl(path)
    if err == "missing":
        return None, "qualification_missing"
    if err:
        return None, f"qualification_{err}"
    if not rows or any(not isinstance(row, dict) for row in rows):
        return None, "qualification_malformed"
    return rows, None  # type: ignore[return-value]


def _qual_settings_list(rows: list[dict]) -> tuple[Optional[list[dict]], Optional[str]]:
    """Collect every qualification settings object; require critical-key agreement."""
    settings_list: list[dict] = []
    for row in rows:
        if not isinstance(row, dict):
            return None, "qualification_malformed"
        settings = row.get("settings")
        if settings is None:
            continue
        if not isinstance(settings, dict):
            return None, "qualification_settings_malformed"
        settings_list.append(settings)
    if not settings_list:
        return None, "qualification_settings_missing"
    agree_keys = (
        "responsiveness_profile_enabled",
        "responsiveness_fine_enabled",
        "frontend",
        "n",
        "render_policy_requested",
        "workload",
    )
    first = settings_list[0]
    for other in settings_list[1:]:
        for key in agree_keys:
            if first.get(key) != other.get(key):
                return None, "qualification_settings_disagree"
    return settings_list, None


def _qual_slot_table(rows: list[dict]) -> tuple[Optional[list[dict]], Optional[str]]:
    """Return authoritative slots[] from qualification (not qualification_slot_rows)."""
    tables: list[list[dict]] = []
    for row in rows:
        slots = row.get("slots")
        if slots is None:
            continue
        if not isinstance(slots, list) or not slots:
            return None, "qualification_slots_malformed"
        cleaned: list[dict] = []
        for i, slot in enumerate(slots):
            if not isinstance(slot, dict):
                return None, "qualification_slots_malformed"
            if type(slot.get("ordinal")) is not int or slot["ordinal"] != i:
                return None, "qualification_ordinal_mismatch"
            if not isinstance(slot.get("name"), str) or not slot["name"]:
                return None, "qualification_slot_name_missing"
            sid = slot.get("responsiveness_slot_id")
            if not _uint(sid):
                return None, "qualification_slot_id_malformed"
            # Generation is NOT on qualification rows (source: qualification_slot_rows).
            if "generation" in slot:
                return None, "qualification_unexpected_generation"
            cleaned.append(slot)
        tables.append(cleaned)
    if not tables:
        return None, "qualification_slots_missing"
    first = tables[0]
    for other in tables[1:]:
        if len(other) != len(first):
            return None, "qualification_slots_disagree"
        for a, b in zip(first, other):
            if (a.get("ordinal"), a.get("name"), a.get("responsiveness_slot_id")) != (
                b.get("ordinal"), b.get("name"), b.get("responsiveness_slot_id")
            ):
                return None, "qualification_slots_disagree"
    return first, None


def _qual_boundary_pair(
    rows: list[dict],
) -> tuple[Optional[dict], Optional[dict], Optional[str]]:
    """Require observe-start + observe-end qualification boundaries with refs."""
    starts: list[dict] = []
    ends: list[dict] = []
    for row in rows:
        if not isinstance(row, dict):
            return None, None, "qualification_malformed"
        phase = row.get("phase")
        cohort = row.get("cohort")
        phase_tag = cohort.get("phase_tag") if isinstance(cohort, dict) else None
        tag = phase if isinstance(phase, str) else phase_tag
        if tag == "observe-start":
            starts.append(row)
        elif tag == "observe-end":
            ends.append(row)
    if not starts:
        return None, None, "qualification_observe_start_missing"
    if not ends:
        return None, None, "qualification_observe_end_missing"
    if len(starts) != 1 or len(ends) != 1:
        return None, None, "qualification_boundary_duplicate"
    start_row, end_row = starts[0], ends[0]
    for label, row in (("start", start_row), ("end", end_row)):
        if "cohort" not in row or not isinstance(row.get("cohort"), dict):
            return None, None, f"qualification_{label}_ref_missing"
        if not isinstance(row.get("settings"), dict):
            return None, None, f"qualification_{label}_settings_missing"
        if not _finite(row.get("elapsed_s")):
            return None, None, f"qualification_{label}_elapsed_malformed"
    start_elapsed = float(start_row["elapsed_s"])
    end_elapsed = float(end_row["elapsed_s"])
    if not (end_elapsed > start_elapsed):
        return None, None, "qualification_elapsed_order"
    # Cohort ref phase tags must match boundary roles when present.
    sc, ec = start_row["cohort"], end_row["cohort"]
    if sc.get("phase_tag") not in (None, "observe-start"):
        return None, None, "qualification_start_phase_tag"
    if ec.get("phase_tag") not in (None, "observe-end"):
        return None, None, "qualification_end_phase_tag"
    oee = ec.get("observe_end_elapsed_s")
    if oee is not None and _finite(oee) and float(oee) != end_elapsed:
        # Harness clock self-consistency: end boundary elapsed vs declared end mark.
        if abs(float(oee) - end_elapsed) > 1e-9:
            return None, None, "qualification_observe_end_elapsed_mismatch"
    return start_row, end_row, None


def _sample_generations(samples: list[dict]) -> tuple[Optional[dict[int, int]], Optional[str]]:
    """Map responsiveness slot_id → single generation from cohort-bearing samples.

    Only samples with ``cohort.present is True`` contribute. Pre-arm seed/warmup
    generations (outside the publisher-declared cohort span) are ignored — a
    legitimate seed restart before observe must not look like an in-cohort reset.
    Multiple generations per slot *within* the cohort-bearing span fail closed.
    Missing generation is not invented. ``ended=True`` or empty/disappeared
    profiles on a cohort-bearing sample fail closed. Interior observe/drain rows
    without a present cohort ref cannot hide a reset by exclusion.
    """
    seen: dict[int, set[int]] = {}
    found_any = False
    slot_sets: list[set[int]] = []
    for sample in samples:
        if not isinstance(sample, dict):
            continue
        phase = sample.get("phase")
        cohort = sample.get("cohort")
        cohort_present = isinstance(cohort, dict) and cohort.get("present") is True
        # Once armed, observe/drain rows must carry cohort refs — missing refs
        # must not exclude a row that would reveal disappearance/reset.
        if phase in ("observe", "drain") and not cohort_present:
            return None, "cohort_sample_ref_missing_interior"
        if not cohort_present:
            # Outside publisher cohort span (seed/warmup/teardown without attach).
            continue
        rows = sample.get("responsiveness_profile")
        if rows is None or (isinstance(rows, list) and len(rows) == 0):
            return None, "sample_slot_disappeared"
        if not isinstance(rows, list):
            return None, "sample_responsiveness_malformed"
        present_ids: set[int] = set()
        for row in rows:
            if not isinstance(row, dict):
                return None, "sample_responsiveness_malformed"
            if row.get("ended") is True:
                return None, "sample_slot_ended"
            sid = row.get("slot_id")
            gen = row.get("generation")
            if not _uint(sid) or not _uint(gen):
                return None, "sample_slot_generation_malformed"
            found_any = True
            sid_i = int(sid)
            gen_i = int(gen)
            if sid_i in present_ids:
                return None, "sample_duplicate_slot_identity"
            present_ids.add(sid_i)
            seen.setdefault(sid_i, set()).add(gen_i)
        slot_sets.append(present_ids)
    if not found_any:
        return None, "sample_generations_missing"
    # Lifetime: every cohort-bearing sample must expose the same slot set.
    # One good sample cannot prove population lifetime alone.
    base = slot_sets[0]
    for other in slot_sets[1:]:
        if other != base:
            return None, "sample_slot_disappeared"
    out: dict[int, int] = {}
    for sid, gens in seen.items():
        if len(gens) != 1:
            return None, "sample_generation_reset"
        out[sid] = next(iter(gens))
    return out, None


def _cohort_ref_ok(
    ref: Any,
    run_dir: pathlib.Path,
    sidecar: pathlib.Path,
    boundaries: tuple[int, int, int],
    *,
    canonical_path: Optional[str],
    require_qual_fields: bool,
) -> tuple[bool, Optional[str], Optional[str]]:
    if not isinstance(ref, dict):
        return False, None, "cohort_ref_malformed"
    if ref.get("present") is not True:
        return False, None, "cohort_ref_not_present"
    if ref.get("schema_version") != SCHEMA_VERSION:
        return False, None, "cohort_ref_schema"
    if ref.get("tail_name") != TAIL_NAME:
        return False, None, "cohort_ref_tail_name"
    if _validate_boundaries(ref.get("boundaries")) != boundaries:
        return False, None, "cohort_ref_boundaries"
    ok, norm = _bound_sidecar_ref(
        ref.get("sidecar_path"), run_dir, sidecar, canonical_recorded=canonical_path
    )
    if not ok:
        return False, norm, "cohort_ref_sidecar_path"
    if require_qual_fields:
        if ref.get("qualification_elapsed_s_is_harness_not_cohort_mono") is not True:
            return False, norm, "cohort_ref_clock_flag"
        if "phase_tag" not in ref or not isinstance(ref.get("phase_tag"), str):
            return False, norm, "cohort_ref_phase_tag"
        oee = ref.get("observe_end_elapsed_s")
        if oee is not None and not _finite(oee):
            return False, norm, "cohort_ref_observe_end_elapsed"
    return True, norm, None


def _surface_for_gate(gate: str, frontend: str) -> str:
    if gate == "decode":
        return "Decode"
    if frontend == "tui":
        return "Tui"
    return "Panel"


def _meta_profile_ok(
    meta: dict,
    settings_list: list[dict],
    header_frontend: str,
) -> Optional[str]:
    """Require profile/fine/frontend/N agreement across meta and EVERY qual settings."""
    if meta.get("frontend") != header_frontend:
        return "cohort_frontend_mismatch"
    if meta.get("responsiveness_profile") is not True:
        return "responsiveness_profile_disabled"
    # Fine buckets drive the <=100ms cohort verdict; meta alone is not enough —
    # every qualification settings object must also enable fine (no meta-True
    # override of silent qual False).
    if meta.get("responsiveness_fine") is not True:
        return "responsiveness_fine_disabled"
    for settings in settings_list:
        if settings.get("responsiveness_profile_enabled") is not True:
            return "qualification_profile_disabled"
        if settings.get("responsiveness_fine_enabled") is not True:
            return "qualification_fine_disabled"
        frontend = settings.get("frontend")
        if frontend is not None and frontend != header_frontend:
            return "qualification_frontend_mismatch"
        n_set = settings.get("n")
        if n_set is not None and n_set != meta.get("n"):
            return "qualification_n_mismatch"
    return None


def _expected_population(
    gate: str,
    header: dict,
    meta: dict,
    qual_slots: list[dict],
    generations: dict[int, int],
    settings: Optional[dict],
) -> tuple[Optional[set[tuple[int, int]]], Optional[str]]:
    """Build declared (slot_id, generation) population from qual ordinals + sample gens."""
    n = meta.get("n")
    if not _uint(n) or n == 0:
        return None, "meta_n_invalid"
    if len(qual_slots) != n:
        return None, "qualification_population_n_mismatch"
    # Decode always all-run-slots with n.
    if gate == "decode":
        pop = header.get("decode_population")
        if (not isinstance(pop, dict) or pop.get("kind") != "all-run-slots"
                or pop.get("n") != n):
            return None, "decode_population_mismatch"
        expected: set[tuple[int, int]] = set()
        for slot in qual_slots:
            sid = slot["responsiveness_slot_id"]
            if sid not in generations:
                return None, "missing_generation_for_slot"
            expected.add((sid, generations[sid]))
        if len(expected) != n:
            return None, "decode_population_identity_collision"
        return expected, None

    pop = header.get("input_population")
    if not isinstance(pop, dict) or not isinstance(pop.get("kind"), str):
        return None, "input_population_missing"
    kind = pop["kind"]
    frontend = header.get("frontend")

    if kind == "focused-one":
        slots = pop.get("slots")
        if not isinstance(slots, list) or not slots or any(not _uint(x) for x in slots):
            return None, "input_population_malformed"
        # slots are fixture ordinals, not EventId.slot_id.
        policy = None
        if isinstance(settings, dict):
            policy = settings.get("render_policy_requested")
        if policy is None:
            policy = meta.get("render_policy")
        # Header kind is focused-one; when policy is present it must agree.
        if policy is not None and policy != "focused-one":
            return None, "input_render_policy_mismatch"
        expected = set()
        for ordinal in slots:
            if ordinal >= len(qual_slots):
                return None, "input_ordinal_out_of_range"
            sid = qual_slots[ordinal]["responsiveness_slot_id"]
            if sid not in generations:
                return None, "missing_generation_for_slot"
            expected.add((sid, generations[sid]))
        if not expected:
            return None, "input_population_empty"
        return expected, None

    if kind == "all-run-slots":
        if pop.get("n") != n:
            return None, "input_population_n_mismatch"
        expected = set()
        for slot in qual_slots:
            sid = slot["responsiveness_slot_id"]
            if sid not in generations:
                return None, "missing_generation_for_slot"
            expected.add((sid, generations[sid]))
        if len(expected) != n:
            return None, "input_population_identity_collision"
        return expected, None

    if kind == "tui-endpoint":
        if frontend != "tui" or meta.get("frontend") != "tui":
            return None, "input_endpoint_mismatch"
        # TUI is a distinct endpoint population — all run slots' Tui surface.
        expected = set()
        for slot in qual_slots:
            sid = slot["responsiveness_slot_id"]
            if sid not in generations:
                return None, "missing_generation_for_slot"
            expected.add((sid, generations[sid]))
        return expected, None

    return None, "unsupported_input_population"


def _verdict_from_fine(fine: dict) -> str:
    if fine.get("status") != "available":
        return "unavailable"
    upper = fine.get("upper_ms")
    if type(upper) is not int:
        return "unavailable"
    if upper <= TARGET_MS:
        return "meet"
    lower = fine.get("lower_ms")
    if type(lower) is int and lower > TARGET_MS:
        return "miss"
    return "miss"


def _read_cohort_impl(run_dir: pathlib.Path, meta: dict, samples: list[dict], gate: str) -> dict:
    if gate not in ("decode", "input"):
        return unavailable("unknown_gate", gate=gate)
    if not isinstance(meta, dict):
        return unavailable("bad_metadata", gate=gate)
    if not isinstance(samples, list):
        return unavailable("bad_samples", gate=gate)

    run_dir = pathlib.Path(run_dir)
    path = run_dir / "samples.cohort.jsonl"
    if not path.is_file():
        return unavailable("cohort_schema_missing", gate=gate)

    objects, load_err = _load_jsonl(path)
    if load_err:
        return unavailable("cohort_sidecar_unreadable", gate=gate, detail=load_err)
    assert objects is not None
    if not objects:
        return unavailable("cohort_header_missing", gate=gate)

    header = objects[0]
    if not isinstance(header, dict) or header.get("record") != "cohort-header":
        return unavailable("cohort_header_missing", gate=gate)
    if objects.count(header) and sum(1 for o in objects if isinstance(o, dict) and o.get("record") == "cohort-header") != 1:
        return unavailable("multiple_cohort_headers", gate=gate)
    if sum(1 for o in objects if isinstance(o, dict) and o.get("record") == "cohort-header") != 1:
        return unavailable("multiple_cohort_headers", gate=gate)

    if (not _u16_schema(header.get("schema_version")) or header.get("schema_version") != SCHEMA_VERSION
            or header.get("clock_domain") != CLOCK_DOMAIN
            or not _uint(header.get("observe_ns")) or not _uint(header.get("capacity"))
            or header.get("capacity") == 0
            or not isinstance(header.get("frontend"), str)):
        return unavailable("unknown_cohort_schema", gate=gate)

    boundaries = _validate_boundaries(header.get("boundaries"))
    if boundaries is None or header.get("tail_name") != TAIL_NAME or header.get("tail_ns") != boundaries[2]:
        return unavailable("malformed_cohort_boundaries", gate=gate)
    start, end, tail = boundaries
    if header.get("observe_ns") != end - start:
        return unavailable("observe_duration_mismatch", gate=gate)

    ok, header_path_norm = _bound_sidecar_ref(header.get("sidecar_path"), run_dir, path)
    if not ok or header_path_norm is None:
        return unavailable("sidecar_provenance_mismatch", gate=gate)
    canonical_path = header_path_norm

    if header.get("frontend") != meta.get("frontend"):
        return unavailable("cohort_frontend_mismatch", gate=gate)

    qualification, qerr = _load_qualification(run_dir)
    if qerr:
        return unavailable(qerr, gate=gate)
    assert qualification is not None

    start_qual, end_qual, berr = _qual_boundary_pair(qualification)
    if berr:
        return unavailable(berr, gate=gate)
    assert start_qual is not None and end_qual is not None
    harness_start_elapsed = float(start_qual["elapsed_s"])
    harness_end_elapsed = float(end_qual["elapsed_s"])

    settings_list, serr = _qual_settings_list(qualification)
    if serr:
        return unavailable(serr, gate=gate)
    assert settings_list is not None
    settings = settings_list[0]

    profile_err = _meta_profile_ok(meta, settings_list, header["frontend"])
    if profile_err:
        return unavailable(profile_err, gate=gate)

    qual_slots, qs_err = _qual_slot_table(qualification)
    if qs_err:
        return unavailable(qs_err, gate=gate)
    assert qual_slots is not None

    generations, gen_err = _sample_generations(samples)
    if gen_err:
        return unavailable(gen_err, gate=gate)
    assert generations is not None

    expected, pop_err = _expected_population(
        gate, header, meta, qual_slots, generations, settings
    )
    if pop_err:
        return unavailable(pop_err, gate=gate)
    assert expected is not None

    # Declared population identities must appear in every cohort-bearing sample.
    expected_sids = {sid for sid, _gen in expected}
    for sample in samples:
        if not isinstance(sample, dict):
            continue
        cohort = sample.get("cohort")
        if not (isinstance(cohort, dict) and cohort.get("present") is True):
            continue
        rows = sample.get("responsiveness_profile")
        if not isinstance(rows, list):
            return unavailable("sample_responsiveness_malformed", gate=gate)
        present = set()
        for row in rows:
            if not isinstance(row, dict) or not _uint(row.get("slot_id")):
                return unavailable("sample_responsiveness_malformed", gate=gate)
            present.add(int(row["slot_id"]))
        if not expected_sids.issubset(present):
            return unavailable("sample_population_incomplete", gate=gate)

    # Provenance refs on samples + qualification must agree with sidecar.
    sample_refs = []
    sample_cursors: list[int] = []
    for sample in samples:
        if not isinstance(sample, dict) or "cohort" not in sample:
            continue
        ref = sample["cohort"]
        sample_refs.append(ref)
        if isinstance(ref, dict) and ref.get("present") is True:
            # Harness elapsed on cohort-bearing samples must sit in the
            # qualification observe interval (distinct clock, self-consistent).
            elapsed = sample.get("elapsed_s")
            if elapsed is not None:
                if not _finite(elapsed):
                    return unavailable("sample_elapsed_malformed", gate=gate)
                ev = float(elapsed)
                # Drain may land at/after observe-end mark; reject before start.
                if ev < harness_start_elapsed:
                    return unavailable("sample_elapsed_before_observe", gate=gate)
            cur = ref.get("cursor")
            if not _uint(cur):
                return unavailable("sample_cursor_malformed", gate=gate)
            sample_cursors.append(int(cur))
    qual_refs = []
    for row in qualification:
        if "cohort" in row:
            qual_refs.append(row["cohort"])
    if not sample_refs:
        return unavailable("cohort_sample_ref_missing", gate=gate)
    if not qual_refs:
        return unavailable("cohort_qualification_ref_missing", gate=gate)
    for ref in sample_refs:
        ok, _norm, reason = _cohort_ref_ok(
            ref, run_dir, path, boundaries,
            canonical_path=canonical_path, require_qual_fields=False,
        )
        if not ok:
            return unavailable(reason or "cohort_sample_ref_mismatch", gate=gate)
    for ref in qual_refs:
        ok, _norm, reason = _cohort_ref_ok(
            ref, run_dir, path, boundaries,
            canonical_path=canonical_path, require_qual_fields=True,
        )
        if not ok:
            return unavailable(reason or "cohort_qualification_ref_mismatch", gate=gate)

    records: list[dict] = []
    losses: list[dict] = []
    cursor = 0
    last_loss = 0
    last_overflow = 0
    terminal = None
    last_batch_complete = False
    seen_sequences: set[tuple[Any, ...]] = set()
    batch_count = 0
    emitted_cursors: set[int] = {0}

    for obj in objects[1:]:
        if not isinstance(obj, dict):
            return unavailable("malformed_cohort_record", gate=gate)
        kind = obj.get("record")
        if terminal is not None:
            return unavailable("record_after_terminal", gate=gate)

        if kind == "cohort-batch":
            batch_count += 1
            if (not _u16_schema(obj.get("schema_version")) or obj.get("schema_version") != SCHEMA_VERSION
                    or type(obj.get("complete")) is not bool
                    or _validate_boundaries(obj.get("boundaries")) != boundaries):
                return unavailable("batch_provenance_mismatch", gate=gate)
            next_cursor_v = obj.get("next_cursor")
            loss_count_v = obj.get("loss_count")
            overflow_n_v = obj.get("journal_overflow_n")
            if not _uint(next_cursor_v) or not _uint(loss_count_v) or not _uint(overflow_n_v):
                return unavailable("batch_counters_malformed", gate=gate)
            next_cursor = int(next_cursor_v)
            loss_count = int(loss_count_v)
            overflow_n = int(overflow_n_v)
            if next_cursor < cursor:
                return unavailable("cursor_regression", gate=gate)
            if loss_count < last_loss or overflow_n < last_overflow:
                return unavailable("loss_counter_regression", gate=gate)

            batch_records, batch_losses = obj.get("records"), obj.get("losses")
            if not isinstance(batch_records, list) or not isinstance(batch_losses, list):
                return unavailable("malformed_batch", gate=gate)

            for raw in batch_records:
                rec, err = _event(raw, start, end, tail)
                if err:
                    return unavailable(err, gate=gate)
                assert rec is not None
                key = (
                    rec["id"]["slot_id"], rec["id"]["generation"],
                    rec["id"]["sequence"], rec["id"]["surface"],
                )
                if key in seen_sequences:
                    return unavailable("duplicate_event_identity", gate=gate)
                seen_sequences.add(key)
                records.append(rec)

            for raw in batch_losses:
                err = _loss(raw)
                if err:
                    return unavailable(err, gate=gate)
                losses.append(raw)

            retained = len(batch_records) + len(batch_losses)
            # Inclusive high-water: empty batch keeps cursor; non-empty advances
            # contiguously by retained journal entries (source extract_since).
            if retained == 0:
                if next_cursor != cursor:
                    return unavailable("cursor_accounting_mismatch", gate=gate)
            else:
                if next_cursor != cursor + retained:
                    return unavailable("cursor_accounting_mismatch", gate=gate)

            loss_delta = loss_count - last_loss
            if loss_delta < len(batch_losses):
                return unavailable("loss_receipt_count_mismatch", gate=gate)
            # Counter-only gaps: loss_count advanced without receipts.
            # Unlocalizable capacity/publication gap — fail at terminal accounting.

            cursor = next_cursor
            emitted_cursors.add(cursor)
            last_loss = loss_count
            last_overflow = overflow_n
            last_batch_complete = obj["complete"] is True

        elif kind == "cohort-terminal":
            if (not _u16_schema(obj.get("schema_version")) or obj.get("schema_version") != SCHEMA_VERSION
                    or _validate_boundaries(obj.get("boundaries")) != boundaries):
                return unavailable("malformed_terminal", gate=gate)
            terminal = obj
        else:
            return unavailable("unknown_cohort_record", gate=gate)

    if terminal is None:
        return unavailable("cohort_terminal_missing", gate=gate)
    if terminal.get("terminal") is not True:
        return unavailable("cohort_terminal_flag", gate=gate)
    if terminal.get("frontend") != header.get("frontend"):
        return unavailable("terminal_frontend_mismatch", gate=gate)
    if terminal.get("tail_name") != TAIL_NAME:
        return unavailable("terminal_tail_name_mismatch", gate=gate)
    if terminal.get("producers_joined") is not True:
        return unavailable("producers_not_joined", gate=gate)
    oee = terminal.get("observe_end_elapsed_s")
    if not _finite(oee):
        return unavailable("observe_end_elapsed_missing", gate=gate)
    # Terminal harness end mark must agree with qualification observe-end.
    if abs(float(oee) - harness_end_elapsed) > 1e-9:
        return unavailable("observe_end_elapsed_mismatch", gate=gate)
    if batch_count == 0 or not last_batch_complete:
        return unavailable("final_batch_incomplete", gate=gate)

    # Sample cohort.cursor must match emitted batch progress (publisher HWM).
    if not sample_cursors:
        return unavailable("sample_cursor_missing", gate=gate)
    for sc in sample_cursors:
        if sc not in emitted_cursors:
            return unavailable("sample_cursor_mismatch", gate=gate)
    if max(sample_cursors) != cursor:
        return unavailable("sample_cursor_hwm_mismatch", gate=gate)

    if not all(_uint(terminal.get(k)) for k in ("records_n", "losses_n", "pending_n")):
        return unavailable("malformed_terminal_counts", gate=gate)
    if terminal["records_n"] != len(records):
        return unavailable("terminal_records_mismatch", gate=gate)
    # losses_n may exceed serialized receipts (capacity gaps).
    if terminal["losses_n"] < len(losses):
        return unavailable("terminal_losses_underflow", gate=gate)
    if terminal["losses_n"] != last_loss:
        return unavailable("terminal_loss_counter_mismatch", gate=gate)
    # pending_n is AFTER finalize converted pending → Incomplete losses; often 0.
    if terminal["pending_n"] != 0:
        return unavailable("terminal_pending_nonzero", gate=gate)

    # Global terminal unavailability + any loss/overflow/counter-only gap.
    if (terminal.get("available") is not True
            or losses
            or last_overflow
            or last_loss
            or terminal["losses_n"] != 0):
        return unavailable(
            "cohort_accounting_unavailable",
            gate=gate,
            records_n=len(records),
            losses_n=int(terminal["losses_n"]),
            loss_receipts_n=len(losses),
            journal_overflow_n=last_overflow,
            terminal_available=terminal.get("available"),
        )

    surface = _surface_for_gate(gate, str(header["frontend"]))
    selected = [
        r for r in records
        if r["id"]["surface"] == surface and r["outcome"] == "Completed"
    ]
    # Non-completed outcomes for this surface make the cohort unavailable.
    noncompleted = [
        r for r in records
        if r["id"]["surface"] == surface and r["outcome"] != "Completed"
    ]
    if noncompleted:
        return unavailable("noncompleted_member", gate=gate, count=len(noncompleted))

    # Identity must match declared population generations exactly.
    by_slot: dict[tuple[int, int], list[int]] = {key: [] for key in expected}
    for r in selected:
        key = (r["id"]["slot_id"], r["id"]["generation"])
        if key not in expected:
            return unavailable("unexpected_slot_identity", gate=gate,
                               slot_id=r["id"]["slot_id"], generation=r["id"]["generation"])
        by_slot[key].append(r["duration_ns"])

    missing = [key for key, durs in by_slot.items() if not durs]
    if missing:
        return unavailable(
            "declared_population_incomplete",
            gate=gate,
            missing_n=len(missing),
            expected_n=len(expected),
        )

    # Per-slot p99 — do not pool a slow slot into a fleet average.
    slot_results = []
    worst_verdict = "meet"
    for key, durs in sorted(by_slot.items()):
        coarse = p99_ns(durs, COARSE_BOUNDS_MS)
        fine = p99_ns(durs, FINE_BOUNDS_MS)
        verdict = _verdict_from_fine(fine)
        slot_results.append({
            "slot_id": key[0],
            "generation": key[1],
            "sample_n": len(durs),
            "p99": coarse,
            "fine_p99": fine,
            "target_verdict": verdict,
        })
        if verdict == "unavailable":
            worst_verdict = "unavailable"
        elif verdict == "miss" and worst_verdict == "meet":
            worst_verdict = "miss"

    if worst_verdict == "unavailable":
        reason = next(
            (s["fine_p99"].get("reason") for s in slot_results
             if s["target_verdict"] == "unavailable"),
            "slot_p99_unavailable",
        )
        return unavailable(
            reason or "slot_p99_unavailable",
            gate=gate + "_cohort",
            slots=slot_results,
            population_n=len(expected),
            boundaries={"start_mono_ns": start, "end_mono_ns": end, "tail_ns": tail},
        )

    all_durs = [d for durs in by_slot.values() for d in durs]
    return {
        "status": "available",
        "gate": gate + "_cohort",
        "target_verdict": worst_verdict,
        "reason": None,
        "population_n": len(expected),
        "event_n": len(all_durs),
        "records_n": len(records),
        "losses_n": 0,
        "boundaries": {
            "start_mono_ns": start,
            "end_mono_ns": end,
            "tail_ns": tail,
        },
        "p99": p99_ns(all_durs, COARSE_BOUNDS_MS),
        "fine_p99": p99_ns(all_durs, FINE_BOUNDS_MS),
        "slots": slot_results,
        "events": selected,
    }


def read_cohort(run_dir: pathlib.Path, meta: dict, samples: list[dict], gate: str) -> dict:
    """Public fail-closed boundary: malformed archives never raise."""
    try:
        return _read_cohort_impl(run_dir, meta, samples, gate)
    except Exception as exc:  # noqa: BLE001 — fail closed on any unexpected fault
        return unavailable(
            "malformed_cohort_input",
            gate=gate,
            detail=type(exc).__name__,
        )
