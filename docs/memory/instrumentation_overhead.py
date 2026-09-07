#!/usr/bin/env python3
"""Offline four-cell instrumentation overhead reader (artifact-backed).

Consumes an explicit ordered OFF/ON/ON/OFF receipt quartet plus shared
manifest, re-binds each side through ``matched_evidence_adapter.bind_side``,
and reports host/server/helper resource deltas separately.

Does not authorize final performance acceptance, RSS savings, or pair
eligibility. Continuous helper accounting measures helper resources only —
not sampler causal perturbation of the host workload.
"""
from __future__ import annotations

import math
import pathlib
import sys
from typing import Any, Mapping, Optional, Sequence

ROOT = pathlib.Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import matched_evidence_adapter as mea  # noqa: E402

PASS_MEANS = (
    "offline four-cell instrumentation overhead reader only; "
    "not final performance, RSS saving, or pair eligibility"
)

# Predeclared same-binary order from the approved batch protocol.
EXPECTED_SETTING_ORDER = ("off", "on", "on", "off")
EXPECTED_CELL_LABELS = ("off_a", "on_a", "on_b", "off_b")
EXPECTED_PROTOCOL_N = 16

# Toggled as a group for the overhead experiment (GPU stays disabled on TUI).
PROFILE_GROUP_KEYS = (
    "scheduling_profile",
    "render_profile",
    "responsiveness_profile",
    "responsiveness_fine",
)
GPU_PROFILE_KEY = "gpu_completion_profile"

# Match-key / native-key fields that may differ solely because profiles toggle.
ALLOWED_MATCH_KEY_DIFFS = frozenset(
    PROFILE_GROUP_KEYS
    + (
        GPU_PROFILE_KEY,  # must stay False, still listed so mismatches surface cleanly
        "renderer_settings",  # instrumentation availability differs when profile off
    )
)

# Workload / probe flags must match across the quartet. OFF/ON toggles only the
# instrumentation profile group — never tui_input_probes or other workload knobs.
# (Root may emit tui_input_probes=false for legacy metadata; still exact-match.)
EXACT_MATCH_WORKLOAD_KEYS = frozenset(
    {
        "tui_input_probes",
        "frontend",
        "n",
        "workload",
        "failure_capture",
        "nav_captures",
    }
)

ALLOWED_NATIVE_KEY_DIFFS = frozenset(
    {
        "scheduling_profile_enabled",
        "render_profile_enabled",
        "responsiveness_profile_enabled",
        "responsiveness_fine_enabled",
        "gpu_completion_profile_enabled",
        "env_flags_requested",
        "renderer_config_by_ordinal",
    }
)

HOST_ROLE = "rust_host"
SERVER_ROLE = "game_server"
OWNED_ROLES_PID_MAY_DIFFER = frozenset({"controller", "launcher", "collector"})
# Server + ambient identities must be equal across the quartet.
AMBIENT_IDENTITY_ROLES = frozenset({"game_server", "gateway", "server_supervisor"})

CPU_SCREEN_RATIO = 1.05
CPU_SPREAD_LIMIT = 0.05


def _unavailable(reason: str, **extra: Any) -> dict:
    out = {
        "status": "unavailable",
        "reason": reason,
        "pass_means": PASS_MEANS,
        "final_acceptance_claim": False,
        "pair_eligible": False,
        "accepted_rss_saving": False,
        "instrumentation_overhead_measured": False,
        "cpu_5pct_screen": None,
        "latency_overhead": {
            "status": "unmeasured",
            "reason": "not_evaluated",
        },
        "remaining_conditions": [reason],
        "partial": {},
    }
    out.update(extra)
    return out


def _finite_nonneg(value: Any) -> Optional[float]:
    if type(value) not in (int, float) or isinstance(value, bool):
        return None
    f = float(value)
    if not math.isfinite(f) or f < 0:
        return None
    return f


def _profile_setting(match_keys: Mapping[str, Any]) -> Optional[str]:
    """Return 'off' / 'on' when the four profile flags form a legal group."""
    if not isinstance(match_keys, Mapping):
        return None
    flags = []
    for key in PROFILE_GROUP_KEYS:
        val = match_keys.get(key)
        if type(val) is not bool:
            return None
        flags.append(val)
    gpu = match_keys.get(GPU_PROFILE_KEY)
    if gpu is not False:
        # TUI overhead protocol keeps GPU completion profiling disabled.
        return None
    if flags == [False, False, False, False]:
        return "off"
    if flags == [True, True, True, True]:
        return "on"
    return None


def _wall_span(side: Mapping[str, Any]) -> Optional[tuple[float, float]]:
    span = side.get("observation_wall_span")
    if not isinstance(span, (list, tuple)) or len(span) != 2:
        return None
    a, b = _finite_nonneg(span[0]), _finite_nonneg(span[1])
    if a is None or b is None or not (b > a):
        return None
    return (a, b)


def _windows_overlap(a: tuple[float, float], b: tuple[float, float]) -> bool:
    return not (a[1] <= b[0] or b[1] <= a[0])


def _run_identity(side: Mapping[str, Any]) -> Optional[str]:
    ident = side.get("run_identity")
    if not isinstance(ident, Mapping):
        return None
    value = ident.get("identity_sha256")
    return value if isinstance(value, str) and len(value) == 64 else None


def _binary_sha(side: Mapping[str, Any]) -> Optional[str]:
    sha = side.get("binary_sha256")
    if isinstance(sha, str) and len(sha) == 64:
        return sha
    prov = side.get("side_provenance")
    if isinstance(prov, Mapping):
        sha = prov.get("binary_sha256")
        if isinstance(sha, str) and len(sha) == 64:
            return sha
    return None


def _managed_process(side: Mapping[str, Any]) -> Optional[dict]:
    managed = side.get("managed_resources")
    if not isinstance(managed, dict) or managed.get("status") != "available":
        return None
    process = managed.get("process")
    if not isinstance(process, dict) or process.get("status") != "available":
        return None
    roles = process.get("roles")
    if not isinstance(roles, dict) or not roles:
        return None
    return process


def _role_metrics(role_row: Mapping[str, Any]) -> Optional[dict]:
    if not isinstance(role_row, Mapping):
        return None
    cpu = _finite_nonneg(role_row.get("cpu_s_enclosing_observation"))
    rss = _finite_nonneg(role_row.get("resident_median_bytes"))
    peak = _finite_nonneg(role_row.get("sampled_resident_peak_bytes"))
    cores = role_row.get("cpu_cores_interval")
    cores_lo = cores_hi = None
    if isinstance(cores, (list, tuple)) and len(cores) == 2:
        cores_lo = _finite_nonneg(cores[0])
        cores_hi = _finite_nonneg(cores[1])
    if cpu is None or rss is None:
        return None
    pid = role_row.get("pid")
    start = role_row.get("start_identity")
    if type(pid) is not int or pid <= 0:
        return None
    if not isinstance(start, str) or not start:
        return None
    return {
        "pid": pid,
        "start_identity": start,
        "cpu_s_enclosing_observation": cpu,
        "resident_median_bytes": rss,
        "sampled_resident_peak_bytes": peak,
        "cpu_cores_interval": [cores_lo, cores_hi]
        if cores_lo is not None and cores_hi is not None
        else None,
        # Prefer upper core bound (conservative) when screening ratios.
        "cpu_cores_comparison_value": cores_hi
        if cores_hi is not None
        else None,
    }


def _extract_cell_resources(side: Mapping[str, Any]) -> Optional[dict]:
    process = _managed_process(side)
    if process is None:
        return None
    roles_in = process["roles"]
    roles_out: dict[str, dict] = {}
    for name, row in roles_in.items():
        if not isinstance(name, str) or not name:
            return None
        metrics = _role_metrics(row)
        if metrics is None:
            return None
        roles_out[name] = metrics
    if not {'controller', 'launcher', 'collector', SERVER_ROLE}.issubset(roles_out):
        return None
    resources = side.get('analysis', {}).get('gates', {}).get('resources', {})
    # The old resource gate stays unavailable until overhead is measured. Its
    # numeric host counters are usable here only after independent side/native/
    # cache/process qualification; do not replace this owner with a helper.
    required = ('cpu_cores', 'cpu_seconds', 'observation_s', 'resident_median_bytes',
                'resident_max_bytes', 'peak_resident_bytes')
    numbers = {key: _finite_nonneg(resources.get(key)) for key in required}
    if (any(value is None for value in numbers.values())
            or numbers['observation_s'] <= 0 or resources.get('contaminated') is not False):
        return None
    host = {
        'source': 'samples.jsonl_via_independently_bound_analysis',
        **numbers,
        'cpu_s_enclosing_observation': numbers['cpu_seconds'],
        'cpu_cores_comparison_value': numbers['cpu_cores'],
        'cpu_endpoint_semantics': 'host sample counter delta / monotonic sampled span; point estimate',
        'sampled_resident_peak_bytes': numbers['resident_max_bytes'],
    }
    return {
        'host': host,
        'server': {'role': SERVER_ROLE, **roles_out[SERVER_ROLE]},
        'helpers': {name: value for name, value in roles_out.items() if name != SERVER_ROLE},
        'roles': roles_out,
        'metric_owners': {**roles_out, HOST_ROLE: host},
        'process_backend': process.get('process_backend'),
        'continuous_coverage': process.get('continuous_coverage') is True,
    }


def _spread_ratio(values: Sequence[float]) -> Optional[float]:
    """Relative spread (max-min)/min across replicated observations."""
    if len(values) < 2:
        return None
    lo = min(values)
    hi = max(values)
    if lo <= 0:
        return None
    return (hi - lo) / lo


def _paired_delta(on_val: float, off_val: float) -> dict:
    """ON-minus-OFF oriented delta for one adjacent pair."""
    return {
        "on_minus_off": on_val - off_val,
        "on": on_val,
        "off": off_val,
        "ratio_on_over_off": (on_val / off_val) if off_val > 0 else None,
    }


def _cpu_screen(off_vals: Sequence[float], on_vals: Sequence[float]) -> dict:
    """Empirical 5% CPU screen — not a confidence interval, not acceptance.

    Reports within-screen only when:
      conservative ONmax/OFFmin <= 1.05
      AND both OFF and ON replicated spreads <= 5%.
    Otherwise regression (ratio exceeds) or inconclusive (spread too wide /
    incomplete numbers).
    """
    out: dict[str, Any] = {
        "rule": (
            f"within_screen only if ONmax/OFFmin <= {CPU_SCREEN_RATIO} "
            f"and OFF/ON spreads (max-min)/min <= {CPU_SPREAD_LIMIT}; "
            "never a confidence interval or final acceptance"
        ),
        "off_values": list(off_vals),
        "on_values": list(on_vals),
        "off_range": [min(off_vals), max(off_vals)] if off_vals else None,
        "on_range": [min(on_vals), max(on_vals)] if on_vals else None,
        "off_spread": _spread_ratio(off_vals) if len(off_vals) >= 2 else None,
        "on_spread": _spread_ratio(on_vals) if len(on_vals) >= 2 else None,
        "status": "inconclusive",
        "reason": None,
    }
    if len(off_vals) < 2 or len(on_vals) < 2:
        out["reason"] = "need_two_off_and_two_on_observations"
        return out
    if any(v <= 0 for v in list(off_vals) + list(on_vals)):
        out["reason"] = "non_positive_cpu_observation"
        return out
    off_min = min(off_vals)
    on_max = max(on_vals)
    conservative_ratio = on_max / off_min
    out["conservative_on_max_over_off_min"] = conservative_ratio
    off_spread = out["off_spread"]
    on_spread = out["on_spread"]
    assert off_spread is not None and on_spread is not None
    if off_spread > CPU_SPREAD_LIMIT or on_spread > CPU_SPREAD_LIMIT:
        out["status"] = "inconclusive"
        out["reason"] = (
            f"replicated_spread_exceeds_{CPU_SPREAD_LIMIT:g}: "
            f"off_spread={off_spread:.6f} on_spread={on_spread:.6f}"
        )
        return out
    if conservative_ratio > CPU_SCREEN_RATIO:
        best_ratio = min(on_vals) / max(off_vals)
        out['on_min_over_off_max'] = best_ratio
        out['status'] = 'regression' if best_ratio > CPU_SCREEN_RATIO else 'inconclusive'
        out['reason'] = ('entire_observed_ratio_range_exceeds_margin' if best_ratio > CPU_SCREEN_RATIO
                         else 'observed_ratio_range_crosses_margin')
        return out
    out["status"] = "within_5pct_empirical_screen"
    out["reason"] = (
        f"conservative_ONmax/OFFmin={conservative_ratio:.6f} <= {CPU_SCREEN_RATIO} "
        f"and spreads <= {CPU_SPREAD_LIMIT}"
    )
    return out


def _latency_overhead_status(sides: Sequence[Mapping[str, Any]]) -> dict:
    """Missing OFF histograms ⇒ latency overhead unmeasured, not zero."""
    off_sides = [sides[0], sides[3]]
    on_sides = [sides[1], sides[2]]

    def hist_ok(side: Mapping[str, Any]) -> tuple[bool, str]:
        notes = side.get("endpoint_notes") if isinstance(side.get("endpoint_notes"), dict) else {}
        # Prefer scheduling / responsiveness evidence; fine p99 when present.
        scheduling = notes.get("scheduling") if isinstance(notes.get("scheduling"), dict) else {}
        if isinstance(scheduling, dict) and scheduling.get("status") == "available":
            return True, "scheduling_available"
        analysis = side.get("analysis") if isinstance(side.get("analysis"), dict) else {}
        gates = analysis.get("gates") if isinstance(analysis.get("gates"), dict) else {}
        sched_gate = gates.get("scheduling") if isinstance(gates.get("scheduling"), dict) else {}
        if isinstance(sched_gate, dict) and sched_gate.get("status") == "available":
            return True, "analysis_scheduling_available"
        fine = side.get("match_keys") if isinstance(side.get("match_keys"), dict) else {}
        if isinstance(fine, dict) and fine.get("responsiveness_fine") is True:
            # Fine requested but histogram status unknown/unavailable.
            return False, "fine_requested_but_histogram_status_unavailable"
        if isinstance(fine, dict) and fine.get("scheduling_profile") is not True and fine.get("responsiveness_profile") is not True:
            return False, "profiles_off_no_histogram_counters"
        return False, "histogram_unavailable"

    off_states = [hist_ok(s) for s in off_sides]
    on_states = [hist_ok(s) for s in on_sides]
    if not all(ok for ok, _ in off_states):
        reasons = [r for ok, r in off_states if not ok]
        return {
            "status": "unmeasured",
            "reason": "missing_off_histograms",
            "detail": (
                "OFF cells lack available latency histograms, so instrumentation "
                "p99 perturbation cannot be proven and must not be reported as zero. "
                + "; ".join(reasons)
            ),
            "off": [{"ok": ok, "reason": r} for ok, r in off_states],
            "on": [{"ok": ok, "reason": r} for ok, r in on_states],
        }
    if not all(ok for ok, _ in on_states):
        return {
            "status": "unmeasured",
            "reason": "missing_on_histograms",
            "detail": "ON cells lack available latency histograms",
            "off": [{"ok": ok, "reason": r} for ok, r in off_states],
            "on": [{"ok": ok, "reason": r} for ok, r in on_states],
        }
    return {
        "status": "histograms_present_comparison_not_implemented",
        "reason": (
            "OFF and ON histogram endpoints are present; this reader reports "
            "resource overhead only and does not invent a p99 overhead number"
        ),
        "off": [{"ok": ok, "reason": r} for ok, r in off_states],
        "on": [{"ok": ok, "reason": r} for ok, r in on_states],
    }


def _renderer_comparison_note(sides: Sequence[Mapping[str, Any]]) -> dict:
    """Unprofiled renderer rows stay unavailable — do not fabricate backend."""
    notes = []
    blocked = False
    for label, side in zip(EXPECTED_CELL_LABELS, sides):
        keys = side.get("match_keys") if isinstance(side.get("match_keys"), dict) else {}
        setting = _profile_setting(keys) if keys else None
        renderer = keys.get("renderer_settings") if isinstance(keys, dict) else None
        native = side.get("native_qualification") if isinstance(side.get("native_qualification"), dict) else {}
        native_keys = native.get("match_keys") if isinstance(native.get("match_keys"), dict) else {}
        native_renderer = (
            native_keys.get("renderer_config_by_ordinal") if isinstance(native_keys, dict) else None
        )
        entry = {
            "label": label,
            "profile_setting": setting,
            "renderer_settings": renderer,
            "native_renderer_config_by_ordinal": native_renderer,
        }
        if setting == "off":
            # Profile-off must not invent an actual backend from draw=false alone.
            fabricated = False
            if isinstance(renderer, Mapping):
                by = renderer.get("by_ordinal")
                if isinstance(by, list):
                    for row in by:
                        if isinstance(row, Mapping) and row.get("status") == "enabled":
                            fabricated = True
            if fabricated:
                blocked = True
                entry["issue"] = "off_cell_reports_enabled_renderer_row"
            else:
                entry["note"] = (
                    "unprofiled renderer row unavailable; draw=false alone is not "
                    "an observed backend"
                )
        notes.append(entry)
    return {
        "blocked": blocked,
        "cells": notes,
        "comparison_capability_needed": (
            None
            if not blocked
            else (
                "OFF cells must expose explicit disabled_profile_off renderer "
                "rows (or omit live renderer evidence). Do not fabricate backend "
                "from runtime draw=false when render_profile is off."
            )
        ),
    }


def _diff_keys(left: Mapping[str, Any], right: Mapping[str, Any], allowed: frozenset) -> list[str]:
    names = sorted(set(left) | set(right))
    bad = []
    for key in names:
        if key in allowed:
            continue
        if not mea._typed_equal(left.get(key), right.get(key)):
            bad.append(key)
    return bad


def _native_invariant(side, setting):
    keys = side['native_qualification']['match_keys']
    if not isinstance(keys, dict) or not isinstance(keys.get('qualification_settings'), dict):
        raise ValueError('nested qualification_settings required')
    normalized = dict(keys)
    config = dict(keys['qualification_settings'])
    env = config.get('env_flags_requested')
    if not isinstance(env, dict):
        raise ValueError('requested profile environment flags missing')
    env = dict(env)
    for key in PROFILE_GROUP_KEYS + (GPU_PROFILE_KEY,):
        expected = setting == 'on' and key != GPU_PROFILE_KEY
        if config.pop(key + '_enabled', None) is not expected:
            raise ValueError('actual profile flag disagrees: ' + key)
        if env.pop('BOT_' + key.upper(), None) is not expected:
            raise ValueError('requested profile flag disagrees: ' + key)
    config['env_flags_requested'] = env  # all other flags remain exact-match
    normalized['qualification_settings'] = config
    renderers = normalized.pop('renderer_config_by_ordinal', None)
    expected_status = 'enabled' if setting == 'on' else 'disabled_profile_off'
    if (not isinstance(renderers, list) or len(renderers) != side['n']
            or any(not isinstance(row, dict) or row.get('ordinal') != i
                   or row.get('status') != expected_status for i, row in enumerate(renderers))):
        raise ValueError('renderer ordinal or instrumentation availability invalid')
    return normalized


def _identity_check(role_sets: Sequence[Mapping[str, Mapping[str, Any]]]) -> Optional[str]:
    """Server/ambient identities equal; owned controller/launcher/collector may differ."""
    base_roles = set(role_sets[0])
    for roles in role_sets[1:]:
        if set(roles) != base_roles:
            return "role_set_mismatch"
    for name in sorted(base_roles):
        base = role_sets[0][name]
        for roles in role_sets[1:]:
            other = roles[name]
            if name in OWNED_ROLES_PID_MAY_DIFFER:
                # Only require the role to exist with valid metrics (already checked).
                continue
            if name in AMBIENT_IDENTITY_ROLES or name == SERVER_ROLE:
                if base.get("pid") != other.get("pid") or base.get("start_identity") != other.get(
                    "start_identity"
                ):
                    return f"ambient_or_server_identity_mismatch:{name}"
            else:
                # Unknown role: require identity equality (conservative).
                if base.get("pid") != other.get("pid") or base.get("start_identity") != other.get(
                    "start_identity"
                ):
                    return f"role_identity_mismatch:{name}"
    return None


def _bind_one(
    receipt_path: pathlib.Path | str,
    *,
    manifest_path: pathlib.Path | str,
    role: str,
    counting: bool,
    diagnostics: bool,
    server_identity_path: Optional[pathlib.Path | str],
    host_conditions_path: Optional[pathlib.Path | str],
) -> dict:
    return mea.bind_side(
        receipt_path,
        role=role,
        manifest_path=manifest_path,
        counting=counting,
        diagnostics=diagnostics,
        server_identity_path=server_identity_path,
        host_conditions_path=host_conditions_path,
    )


def analyze_instrumentation_overhead(
    receipt_paths: Sequence[pathlib.Path | str],
    *,
    manifest_path: pathlib.Path | str,
    role: str = "candidate",
    counting: bool = False,
    diagnostics: bool = False,
    server_identity_path: Optional[pathlib.Path | str] = None,
    host_conditions_path: Optional[pathlib.Path | str] = None,
    expected_n: int = EXPECTED_PROTOCOL_N,
) -> dict:
    """Analyze the predeclared OFF/ON/ON/OFF same-binary overhead quartet.

    ``receipt_paths`` must be exactly four paths in order off_a, on_a, on_b,
    off_b. Each side is bound by invoking ``matched_evidence_adapter.bind_side``. Caller-supplied bound dicts or
    overhead booleans are never trusted as evidence.
    """
    if not isinstance(receipt_paths, (list, tuple)) or len(receipt_paths) != 4:
        return _unavailable(
            "require_exactly_four_ordered_receipt_paths",
            got=len(receipt_paths) if isinstance(receipt_paths, (list, tuple)) else None,
        )

    # Bind each side independently — never accept pre-bound caller payloads.
    sides: list[dict] = []
    for idx, path in enumerate(receipt_paths):
        side = _bind_one(
            path,
            manifest_path=manifest_path,
            role=role,
            counting=counting,
            diagnostics=diagnostics,
            server_identity_path=server_identity_path,
            host_conditions_path=host_conditions_path,
        )
        if not isinstance(side, dict):
            return _unavailable("bind_side_returned_non_object", index=idx, path=str(path))
        sides.append(side)

    partial: dict[str, Any] = {
        "labels": list(EXPECTED_CELL_LABELS),
        "receipt_paths": [str(p) for p in receipt_paths],
        "side_status": [
            {
                "label": EXPECTED_CELL_LABELS[i],
                "status": s.get("status"),
                "reason": s.get("reason"),
                "binding_ok": s.get("binding_ok"),
                "qualified": s.get("qualified"),
                "receipt_id": s.get("receipt_id"),
                "run_dir": s.get("run_dir"),
            }
            for i, s in enumerate(sides)
        ],
    }

    remaining: list[str] = []

    # Successful completed qualified native windows + managed resources.
    for i, side in enumerate(sides):
        label = EXPECTED_CELL_LABELS[i]
        if side.get("binding_ok") is not True or side.get("status") != "bound":
            remaining.append(f"{label}:side_not_bound:{side.get('reason')}")
        if side.get("qualified") is not True:
            remaining.append(f"{label}:not_qualified")
        if side.get("exit_code") != 0:
            remaining.append(f"{label}:nonzero_exit")
        native = side.get("native_qualification")
        if not isinstance(native, Mapping) or native.get("status") != "available":
            remaining.append(f"{label}:native_qualification_unavailable")
        managed = side.get("managed_resources")
        if not isinstance(managed, Mapping) or managed.get("status") != "available":
            remaining.append(f"{label}:managed_resources_unavailable")
        elif not isinstance(managed.get("process"), Mapping) or managed["process"].get("status") != "available":
            remaining.append(f"{label}:process_evidence_unavailable")

    if remaining:
        # Still extract whatever resource rows we can for partial reporting.
        resources_partial = []
        for i, side in enumerate(sides):
            res = _extract_cell_resources(side)
            resources_partial.append({"label": EXPECTED_CELL_LABELS[i], "resources": res})
        partial["resources"] = resources_partial
        partial["latency_overhead"] = _latency_overhead_status(sides)
        partial["renderer_notes"] = _renderer_comparison_note(sides)
        return _unavailable(
            "sides_incomplete",
            remaining_conditions=remaining,
            partial=partial,
            sides=partial["side_status"],
        )

    # Distinct non-overlapping chronological identities in declared order.
    identities = []
    spans = []
    for i, side in enumerate(sides):
        ident = _run_identity(side)
        span = _wall_span(side)
        if ident is None:
            remaining.append(f"{EXPECTED_CELL_LABELS[i]}:missing_run_identity")
        if span is None:
            remaining.append(f"{EXPECTED_CELL_LABELS[i]}:missing_observation_wall_span")
        identities.append(ident)
        spans.append(span)
    if any(x is None for x in identities + spans):  # type: ignore[list-item]
        return _unavailable(
            "identity_or_span_missing",
            remaining_conditions=remaining,
            partial=partial,
        )
    if len(set(identities)) != 4:
        return _unavailable(
            "duplicate_run_identity",
            identities=identities,
            remaining_conditions=["distinct_run_identities_required"],
            partial=partial,
        )
    run_dirs = [side.get("run_dir") for side in sides]
    if len(set(run_dirs)) != 4:
        return _unavailable(
            "duplicate_run_dir",
            run_dirs=run_dirs,
            remaining_conditions=["distinct_run_dirs_required"],
            partial=partial,
        )
    for i in range(4):
        for j in range(i + 1, 4):
            if _windows_overlap(spans[i], spans[j]):  # type: ignore[arg-type]
                return _unavailable(
                    "overlapping_observation_windows",
                    a=EXPECTED_CELL_LABELS[i],
                    b=EXPECTED_CELL_LABELS[j],
                    spans=spans,
                    remaining_conditions=["nonoverlapping_observation_windows"],
                    partial=partial,
                )
    # Chronological order: each next span starts at/after previous end.
    for i in range(3):
        if not (spans[i][1] <= spans[i + 1][0]):  # type: ignore[index]
            return _unavailable(
                "observation_windows_not_chronological",
                order_break_after=EXPECTED_CELL_LABELS[i],
                spans=spans,
                remaining_conditions=["chronological_off_on_on_off_order"],
                partial=partial,
            )

    # Same frozen binary.
    binaries = [_binary_sha(side) for side in sides]
    if any(b is None for b in binaries):
        return _unavailable("missing_binary_sha256", partial=partial)
    if len(set(binaries)) != 1:
        return _unavailable(
            "binary_sha256_mismatch_across_cells",
            binaries=binaries,
            remaining_conditions=["same_frozen_binary_required"],
            partial=partial,
        )
    partial["binary_sha256"] = binaries[0]

    # Profile settings must match OFF/ON/ON/OFF group protocol.
    settings = []
    for i, side in enumerate(sides):
        keys = side.get("match_keys")
        setting = _profile_setting(keys) if isinstance(keys, Mapping) else None
        if setting is None:
            return _unavailable(
                "invalid_or_mixed_profile_group",
                label=EXPECTED_CELL_LABELS[i],
                match_keys_profiles={
                    k: (keys or {}).get(k) for k in PROFILE_GROUP_KEYS + (GPU_PROFILE_KEY,)
                },
                remaining_conditions=[
                    "profile_group_must_be_all_off_or_all_on_with_gpu_false"
                ],
                partial=partial,
            )
        settings.append(setting)
    if tuple(settings) != EXPECTED_SETTING_ORDER:
        return _unavailable(
            "profile_setting_order_mismatch",
            expected=list(EXPECTED_SETTING_ORDER),
            actual=settings,
            remaining_conditions=["exact_off_on_on_off_order"],
            partial=partial,
        )
    partial["profile_settings"] = settings

    # Match keys equal except allowed profile/renderer instrumentation diffs.
    base_keys = sides[0].get("match_keys")
    if not isinstance(base_keys, Mapping):
        return _unavailable("match_keys_missing", partial=partial)
    for i, side in enumerate(sides[1:], start=1):
        keys = side.get("match_keys")
        if not isinstance(keys, Mapping):
            return _unavailable("match_keys_missing", label=EXPECTED_CELL_LABELS[i], partial=partial)
        bad = _diff_keys(base_keys, keys, ALLOWED_MATCH_KEY_DIFFS)
        if bad:
            return _unavailable(
                "disallowed_match_key_mismatch",
                label=EXPECTED_CELL_LABELS[i],
                differing_keys=bad,
                remaining_conditions=["match_keys_equal_except_allowed_profile_toggles"],
                partial=partial,
            )

    if any(side.get('match_keys', {}).get('frontend') != 'tui' for side in sides):
        return _unavailable('protocol_requires_tui', partial=partial)

    # Native keys are nested. Strip only the exact profile toggles, after
    # checking that actual flags and requested flags agree with each cell.
    native_invariants = []
    for i, side in enumerate(sides):
        try:
            native_invariants.append(_native_invariant(side, settings[i]))
        except (ValueError, TypeError, KeyError) as error:
            return _unavailable('native_profile_contract_invalid', label=EXPECTED_CELL_LABELS[i], detail=str(error), partial=partial)
    for i, value in enumerate(native_invariants[1:], 1):
        if not mea._typed_equal(native_invariants[0], value):
            return _unavailable('disallowed_native_match_key_mismatch', label=EXPECTED_CELL_LABELS[i], partial=partial)
    # ON observations must report the same actual renderer configuration.
    on_renderers = [sides[i]['native_qualification']['match_keys']['renderer_config_by_ordinal'] for i in (1, 2)]
    if not mea._typed_equal(*on_renderers):
        return _unavailable('on_renderer_settings_mismatch', partial=partial)

    # n equality + protocol expectation.
    ns = [side.get("n") for side in sides]
    if any(type(n) is not int or n < 1 for n in ns) or len(set(ns)) != 1:
        return _unavailable("n_mismatch_or_invalid", n_values=ns, partial=partial)
    n_value = ns[0]
    partial["n"] = n_value
    if n_value != expected_n:
        remaining.append(f"protocol_expects_n={expected_n}_got_{n_value}")

    # Resources per cell.
    cell_resources = []
    for i, side in enumerate(sides):
        res = _extract_cell_resources(side)
        if res is None:
            return _unavailable(
                "process_role_metrics_unavailable",
                label=EXPECTED_CELL_LABELS[i],
                remaining_conditions=["real_process_evidence_per_role"],
                partial=partial,
            )
        if not res.get("continuous_coverage"):
            remaining.append(f"{EXPECTED_CELL_LABELS[i]}:continuous_coverage_false")
        cell_resources.append(res)
    partial["resources_by_cell"] = {
        EXPECTED_CELL_LABELS[i]: cell_resources[i] for i in range(4)
    }

    id_err = _identity_check([c["roles"] for c in cell_resources])
    if id_err is not None:
        return _unavailable(
            id_err,
            remaining_conditions=["server_ambient_identities_equal_owned_pids_may_differ"],
            partial=partial,
        )

    renderer_notes = _renderer_comparison_note(sides)
    partial["renderer_notes"] = renderer_notes
    if renderer_notes.get("blocked"):
        remaining.append("renderer_comparison_blocked_fabricated_off_backend")

    latency = _latency_overhead_status(sides)
    partial["latency_overhead"] = latency

    # Per-role observed values across OFF/ON replicates.
    role_names = sorted(cell_resources[0]["metric_owners"])
    per_role: dict[str, Any] = {}
    for name in role_names:
        off_cpu = [
            cell_resources[0]["metric_owners"][name]["cpu_s_enclosing_observation"],
            cell_resources[3]["metric_owners"][name]["cpu_s_enclosing_observation"],
        ]
        on_cpu = [
            cell_resources[1]["metric_owners"][name]["cpu_s_enclosing_observation"],
            cell_resources[2]["metric_owners"][name]["cpu_s_enclosing_observation"],
        ]
        off_rss = [
            cell_resources[0]["metric_owners"][name]["resident_median_bytes"],
            cell_resources[3]["metric_owners"][name]["resident_median_bytes"],
        ]
        on_rss = [
            cell_resources[1]["metric_owners"][name]["resident_median_bytes"],
            cell_resources[2]["metric_owners"][name]["resident_median_bytes"],
        ]
        off_cores = []
        on_cores = []
        for idx in (0, 3):
            c = cell_resources[idx]["metric_owners"][name].get("cpu_cores_comparison_value")
            if c is not None:
                off_cores.append(c)
        for idx in (1, 2):
            c = cell_resources[idx]["metric_owners"][name].get("cpu_cores_comparison_value")
            if c is not None:
                on_cores.append(c)
        category = (
            "host"
            if name == HOST_ROLE
            else ("server" if name == SERVER_ROLE else "helper")
        )
        per_role[name] = {
            "category": category,
            "cpu_s": {
                "off_values": off_cpu,
                "on_values": on_cpu,
                "off_range": [min(off_cpu), max(off_cpu)],
                "on_range": [min(on_cpu), max(on_cpu)],
                "off_spread": _spread_ratio(off_cpu),
                "on_spread": _spread_ratio(on_cpu),
                "paired_deltas_on_minus_off": {
                    "off_a_to_on_a": _paired_delta(on_cpu[0], off_cpu[0]),
                    "on_b_to_off_b": _paired_delta(on_cpu[1], off_cpu[1]),
                },
                "note": "paired deltas are adjacent ON-minus-OFF observations, not confidence intervals",
            },
            "resident_median_bytes": {
                "off_values": off_rss,
                "on_values": on_rss,
                "off_range": [min(off_rss), max(off_rss)],
                "on_range": [min(on_rss), max(on_rss)],
                "off_spread": _spread_ratio(off_rss),
                "on_spread": _spread_ratio(on_rss),
                "paired_deltas_on_minus_off": {
                    "off_a_to_on_a": _paired_delta(on_rss[0], off_rss[0]),
                    "on_b_to_off_b": _paired_delta(on_rss[1], off_rss[1]),
                },
                "note": (
                    "RSS deltas are diagnostic only; allocation removal is not "
                    "an accepted RSS saving"
                ),
            },
            "cpu_cores_comparison_value": {
                "off_values": off_cores,
                "on_values": on_cores,
                "endpoint_semantics": ('host sampled-span point estimates' if name == HOST_ROLE
                                       else 'helper/server acquisition-interval upper endpoints; full intervals retained per cell'),
                "paired_deltas_on_minus_off": (
                    {
                        "off_a_to_on_a": _paired_delta(on_cores[0], off_cores[0]),
                        "on_b_to_off_b": _paired_delta(on_cores[1], off_cores[1]),
                    }
                    if len(off_cores) == 2 and len(on_cores) == 2
                    else None
                ),
            },
        }

    # Only the independently bound Rust process counters drive the host screen.
    host = per_role[HOST_ROLE]
    cpu_screen = _cpu_screen(host['cpu_cores_comparison_value']['off_values'],
                             host['cpu_cores_comparison_value']['on_values'])
    cpu_screen['metric'] = 'rust_host_cpu_cores_from_samples_jsonl'
    cpu_screen['endpoint_semantics'] = 'host monotonic sampled spans; not helper acquisition intervals'

    helpers_report = {
        name: per_role[name]
        for name in role_names
        if per_role[name]["category"] == "helper"
    }

    # Overall measured only when every structural condition is satisfied and
    # we are not blocked on renderer fabrication; still never final acceptance.
    structural_ok = not remaining and not renderer_notes.get("blocked")
    overall_measured = structural_ok and all(
        cell_resources[i].get("continuous_coverage") for i in range(4)
    )

    if not overall_measured and not remaining:
        remaining.append("unspecified_incomplete_condition")

    result = {
        "status": "partial" if not overall_measured else "resource_deltas_available",
        "reason": None if overall_measured else "remaining_conditions",
        "pass_means": PASS_MEANS,
        "final_acceptance_claim": False,
        "pair_eligible": False,
        "accepted_rss_saving": False,
        "resource_deltas_available": overall_measured,
        "instrumentation_overhead_measured": False,
        "resource_deltas_available_means": (
            "four qualified same-binary OFF/ON/ON/OFF cells produced separable "
            "host/server/helper resource deltas under continuous process evidence; "
            "does not prove sampler causal perturbation, latency p99 overhead, "
            "RSS saving, or final acceptance"
        ),
        "protocol": {
            "order": list(EXPECTED_CELL_LABELS),
            "settings": list(EXPECTED_SETTING_ORDER),
            "expected_n": expected_n,
            "actual_n": n_value,
            "profile_group": list(PROFILE_GROUP_KEYS),
            "gpu_completion_profile": False,
            "same_binary_sha256": binaries[0],
        },
        "cells": [
            {
                "label": EXPECTED_CELL_LABELS[i],
                "setting": settings[i],
                "receipt_path": str(receipt_paths[i]),
                "receipt_id": sides[i].get("receipt_id"),
                "run_dir": sides[i].get("run_dir"),
                "run_identity": identities[i],
                "observation_wall_span": list(spans[i]),  # type: ignore[arg-type]
                "resources": cell_resources[i],
            }
            for i in range(4)
        ],
        "host": per_role[HOST_ROLE],
        "server": per_role[SERVER_ROLE],
        "helpers": helpers_report,
        "per_role": per_role,
        "cpu_5pct_screen": cpu_screen,
                "latency_overhead": latency,
        "renderer_notes": renderer_notes,
        "remaining_conditions": remaining + ["latency_overhead_unmeasured", "sampler_causal_perturbation_unmeasured"],
        "partial": partial,
        "notes": [
            "No subtraction of helper CPU/RSS or malloc bytes from host.",
            "Paired deltas are adjacent ON-minus-OFF observations (off_a→on_a, on_b→off_b), not confidence intervals.",
            "Continuous helper accounting measures helper resources; it does not measure sampler causal perturbation of the host.",
            "Missing OFF histograms mean latency overhead is unmeasured, not zero.",
            "CPU 5% empirical screen is not final acceptance.",
        ],
    }
    if remaining:
        result["status"] = "partial"
        result["reason"] = "remaining_conditions"
        result["instrumentation_overhead_measured"] = False
    return result


__all__ = [
    "ALLOWED_MATCH_KEY_DIFFS",
    "ALLOWED_NATIVE_KEY_DIFFS",
    "EXACT_MATCH_WORKLOAD_KEYS",
    "EXPECTED_CELL_LABELS",
    "EXPECTED_PROTOCOL_N",
    "EXPECTED_SETTING_ORDER",
    "PASS_MEANS",
    "PROFILE_GROUP_KEYS",
    "analyze_instrumentation_overhead",
]
