#!/usr/bin/env python3
"""Continuous explicit-PID multi-role process resource accounting.

Named roles map to caller-supplied PIDs only. This module never discovers
processes by name/argv/env, never starts or signals foreign processes, and
never records secrets. It reuses server_resources sampling helpers.

Output is streaming JSONL (exclusive create only — no overwrite). Missing
identity, counter reset, cadence loss, required-grid miss, or required-role
sample failure yields exit 1 with partial records retained.

Modes:
  fixed duration: full required-grid coverage → exit 0 / status ok /
    completion required_grid_complete. Actual duration coverage is separate.
    Early orchestrator stop → incomplete / exit 1.
  stop_controlled (duration=None): requested stop → exit 0 / status closed /
    completion controlled_stop with honest first/last spans and no
    full-duration flag. Future readers must independently require coverage.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import resource
import signal
import sys
import threading
import time
from datetime import datetime, timezone
from typing import Any, Callable, Dict, List, Mapping, Optional, Sequence, TextIO, Tuple

# Adjacent helper module (same directory).
_ROOT = pathlib.Path(__file__).resolve().parent
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

import server_resources as sr  # noqa: E402

SCHEMA = 2
COLLECTOR_ROLE = "collector"
# Default per-role sample timeout when stop-controlled (duration=None) or when
# fixed-duration runs do not pass an explicit bound (still recomputed vs remaining).
DEFAULT_SAMPLE_TIMEOUT_S = 5.0


class AccountingError(sr.SampleError):
    """Config or accounting invariant failure (inherits SampleError for reuse)."""


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _is_finite_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(float(value))


def _require_finite_nonnegative(name: str, value: Any) -> float:
    if not _is_finite_number(value):
        raise AccountingError(f"{name} must be a finite number, got {value!r}")
    f = float(value)
    if f < 0:
        raise AccountingError(f"{name} must be nonnegative, got {f}")
    return f


def validate_sample_dict(sample: Mapping[str, Any], *, role: str) -> Dict[str, Any]:
    """Strict-type a process sample: identity present; counters finite nonnegative."""
    if not isinstance(sample, Mapping):
        raise AccountingError(f"role {role!r}: sample must be a mapping")
    identity = sample.get("start_identity")
    if not isinstance(identity, str) or not identity.strip():
        raise AccountingError("start_identity must be a non-empty string")
    rss = _require_finite_nonnegative("resident_bytes", sample.get("resident_bytes"))
    user = _require_finite_nonnegative("user_s", sample.get("user_s"))
    system = _require_finite_nonnegative("system_s", sample.get("system_s"))
    out = dict(sample)
    out["start_identity"] = identity
    out["resident_bytes"] = int(rss) if float(rss) == int(rss) else rss
    out["user_s"] = user
    out["system_s"] = system
    return out


def parse_role_specs(specs: Sequence[str]) -> Dict[str, int]:
    """Parse ROLE=PID specs. Rejects empty names, bad PIDs, and duplicate PIDs."""
    roles: Dict[str, int] = {}
    pid_to_role: Dict[int, str] = {}
    for raw in specs:
        if "=" not in raw:
            raise AccountingError(f"malformed role spec (expected NAME=PID): {raw!r}")
        name, _, pid_s = raw.partition("=")
        name = name.strip()
        pid_s = pid_s.strip()
        if not name:
            raise AccountingError("role name must be non-empty")
        if name in roles:
            raise AccountingError(f"duplicate role name: {name}")
        try:
            pid = int(pid_s, 10)
        except ValueError as exc:
            raise AccountingError(f"role {name!r} PID must be an integer") from exc
        # Reject bool via validate_pid; int("...") never yields bool.
        sr.validate_pid(pid)
        if pid in pid_to_role:
            raise AccountingError(
                f"duplicate PID {pid} for roles {pid_to_role[pid]!r} and {name!r}; "
                "do not double-count — map each PID to one role"
            )
        roles[name] = pid
        pid_to_role[pid] = name
    if not roles:
        raise AccountingError("at least one ROLE=PID is required")
    return roles


def normalize_roles(
    roles: Mapping[str, int],
    *,
    include_collector_self: bool = True,
    getpid_fn: Optional[Callable[[], int]] = None,
) -> Dict[str, int]:
    """Copy roles, optionally inject collector=self PID without double-counting."""
    getpid_fn = getpid_fn or os.getpid
    out: Dict[str, int] = {}
    if not roles:
        raise AccountingError("at least one role is required")
    seen: Dict[int, str] = {}
    for name, pid in roles.items():
        if not isinstance(name, str) or not name.strip():
            raise AccountingError("role name must be a non-empty string")
        if name != name.strip():
            raise AccountingError(f"role name must not have surrounding whitespace: {name!r}")
        # bool is a subclass of int — validate_pid rejects it.
        if isinstance(pid, bool) or not isinstance(pid, int):
            raise AccountingError(f"role {name!r} PID must be a positive int, got {pid!r}")
        sr.validate_pid(pid)
        if pid in seen:
            raise AccountingError(
                f"duplicate PID {pid} for roles {seen[pid]!r} and {name!r}; "
                "do not double-count — map each PID to one role"
            )
        out[name] = pid
        seen[pid] = name
    if include_collector_self:
        self_pid = getpid_fn()
        if isinstance(self_pid, bool) or not isinstance(self_pid, int):
            raise AccountingError(f"collector PID must be a positive int, got {self_pid!r}")
        self_pid = int(self_pid)
        sr.validate_pid(self_pid)
        if COLLECTOR_ROLE in out:
            if out[COLLECTOR_ROLE] != self_pid:
                raise AccountingError(
                    f"role {COLLECTOR_ROLE!r} must be this collector PID {self_pid}, "
                    f"got {out[COLLECTOR_ROLE]}"
                )
        elif self_pid in seen:
            # Already accounted under another role name — do not double-count.
            pass
        else:
            out[COLLECTOR_ROLE] = self_pid
            seen[self_pid] = COLLECTOR_ROLE
    return out


def required_grid_count(duration: float, interval: float) -> int:
    """Number of scheduled ticks with k*interval < duration (k = 0,1,...).

    Matches the loop condition ``scheduled >= deadline → break``. Examples:
    duration=0.35 interval=0.1 → slots 0.0,0.1,0.2,0.3 → 4;
    duration=1.0 interval=1.0 → slot 0.0 only → 1.
    """
    if duration <= 0 or interval <= 0 or not (math.isfinite(duration) and math.isfinite(interval)):
        raise AccountingError("duration and interval must be finite positive for grid count")
    n = 0
    # Guard against float drift: stop when k*interval >= duration.
    while n * interval < duration - 1e-12:
        n += 1
        if n > 10_000_000:
            raise AccountingError("required grid count overflow")
    if n < 1:
        raise AccountingError(
            f"duration {duration} with interval {interval} yields no required sample slots"
        )
    return n


def scope_notes(roles: Mapping[str, int]) -> Dict[str, Any]:
    """Document measured scope and known unaccounted gaps (not a full tree)."""
    return {
        "measured": (
            "per-role current RSS and cumulative user/system CPU for explicit root PIDs only; "
            "collector self included when configured; host pressure via sample_pressure; "
            "waited-child cumulative/delta CPU via resource.getrusage(RUSAGE_CHILDREN) when available"
        ),
        "not_measured": [
            "descendant/child processes of any role (not a process tree walk)",
            "current RSS of short-lived sample helper subprocesses (macOS ps child RSS is "
            "high-water/maxrss semantics — not emitted as current RSS)",
            "kernel threads, other ambient host processes not listed as roles",
            "instrumentation perturbation (profile on/off); that is a separate protocol",
        ],
        "identity_policy": (
            "start_identity must remain stable per role; change means exit/reuse — "
            "no silent PID replacement"
        ),
        "double_count_policy": "each PID maps to at most one role name",
        "role_count": len(roles),
        "role_names": sorted(roles.keys()),
        "child_rss_policy": (
            "waited-child current RSS is unavailable; ru_maxrss is high-water and is not "
            "reported as current resident set"
        ),
    }


def open_output(path: pathlib.Path) -> TextIO:
    """Exclusive create only. No force/overwrite path on this collector."""
    return sr.open_output(path, force=False)


def _children_rusage_snapshot(
    rusage_children_fn: Optional[Callable[[], Any]] = None,
) -> Dict[str, Any]:
    """Snapshot waited-child cumulative CPU; RSS current is unavailable."""
    try:
        if rusage_children_fn is not None:
            ru = rusage_children_fn()
        else:
            ru = resource.getrusage(resource.RUSAGE_CHILDREN)
        user = float(ru.ru_utime)
        system = float(ru.ru_stime)
        if not (math.isfinite(user) and math.isfinite(system)) or user < 0 or system < 0:
            return {
                "status": "unavailable",
                "reason": "non_finite_or_negative_rusage_children",
                "current_rss_bytes": None,
                "current_rss_status": "unavailable",
                "current_rss_note": "not fabricated; ru_maxrss is high-water not current",
            }
        return {
            "status": "available",
            "source": "resource.getrusage(RUSAGE_CHILDREN)",
            "cumulative_user_s": user,
            "cumulative_system_s": system,
            "cumulative_total_s": user + system,
            "current_rss_bytes": None,
            "current_rss_status": "unavailable",
            "current_rss_note": (
                "waited-child current RSS not available portably; "
                "ru_maxrss is high-water and is not reported as current"
            ),
        }
    except (AttributeError, OSError, ValueError, TypeError) as exc:
        return {
            "status": "unavailable",
            "reason": str(exc),
            "current_rss_bytes": None,
            "current_rss_status": "unavailable",
            "current_rss_note": "not fabricated",
        }


def _children_cost_delta(before: Mapping[str, Any], after: Mapping[str, Any]) -> Dict[str, Any]:
    """Per-sweep waited-child CPU delta; never fabricates current RSS."""
    base = {
        "current_rss_bytes": None,
        "current_rss_status": "unavailable",
        "current_rss_note": (
            "waited-child current RSS not available; high-water maxrss is not current RSS"
        ),
    }
    if before.get("status") != "available" or after.get("status") != "available":
        return {
            **base,
            "status": "unavailable",
            "reason": before.get("reason") or after.get("reason") or "rusage_children_unavailable",
            "source": "resource.getrusage(RUSAGE_CHILDREN)",
            "before": before,
            "after": after,
        }
    du = float(after["cumulative_user_s"]) - float(before["cumulative_user_s"])
    ds = float(after["cumulative_system_s"]) - float(before["cumulative_system_s"])
    if not (math.isfinite(du) and math.isfinite(ds)) or du < 0 or ds < 0:
        return {
            **base,
            "status": "unavailable",
            "reason": "non_monotonic_or_non_finite_children_cpu",
            "source": "resource.getrusage(RUSAGE_CHILDREN)",
            "before": before,
            "after": after,
        }
    return {
        **base,
        "status": "available",
        "source": "resource.getrusage(RUSAGE_CHILDREN)",
        "delta_user_s": du,
        "delta_system_s": ds,
        "delta_total_s": du + ds,
        "cumulative_user_s": float(after["cumulative_user_s"]),
        "cumulative_system_s": float(after["cumulative_system_s"]),
        "cumulative_total_s": float(after["cumulative_total_s"]),
        "note": (
            "waited children only (reaped); separate from collector self and role PIDs; "
            "does not include still-running children"
        ),
    }


def _role_sample_row(
    name: str,
    pid: int,
    current: Dict[str, Any],
    previous: Optional[Dict[str, Any]],
    role_acq_start: float,
    role_acq_end: float,
) -> Dict[str, Any]:
    if previous is not None:
        sr.require_same_identity(previous, current)
        delta = sr.cpu_delta_seconds(previous, current)
        # Per-role CPU interval uses this role's prior acquisition start → this start.
        elapsed = role_acq_start - float(previous["_monotonic"])
        if elapsed <= 0 or not math.isfinite(elapsed):
            raise AccountingError(f"role {name!r}: non-positive monotonic sample interval")
        cores = delta["total_s"] / elapsed
    else:
        delta = {"user_s": 0.0, "system_s": 0.0, "total_s": 0.0}
        cores = None
    return {
        "role": name,
        "pid": pid,
        "status": "ok",
        "start_identity": current["start_identity"],
        "resident_bytes": current["resident_bytes"],
        "acquisition_start_monotonic_s": role_acq_start,
        "acquisition_end_monotonic_s": role_acq_end,
        "acquisition_duration_s": role_acq_end - role_acq_start,
        "cpu": {
            "cumulative_user_s": current["user_s"],
            "cumulative_system_s": current["system_s"],
            "delta_user_s": delta["user_s"],
            "delta_system_s": delta["system_s"],
            "delta_total_s": delta["total_s"],
            "cores_delta": cores,
            "interval_basis": "per_role_acquisition_start",
        },
        "process": {k: v for k, v in current.items() if k not in ("user_s", "system_s")},
    }


def _resolve_sample_timeout(
    *,
    duration: Optional[float],
    sample_timeout: Optional[float],
) -> float:
    """Finite positive per-role timeout. Default when stop-controlled or unspecified."""
    if sample_timeout is None:
        return float(DEFAULT_SAMPLE_TIMEOUT_S)
    if not _is_finite_number(sample_timeout) or float(sample_timeout) <= 0:
        raise AccountingError("sample_timeout must be a finite positive number")
    return float(sample_timeout)


def run(
    roles: Mapping[str, int],
    output: pathlib.Path,
    interval: float,
    duration: Optional[float] = None,
    *,
    include_collector_self: bool = True,
    sample_fn: Optional[Callable[..., Dict[str, Any]]] = None,
    pressure_fn: Optional[Callable[[], Dict[str, Any]]] = None,
    monotonic_fn: Optional[Callable[[], float]] = None,
    sleep_fn: Optional[Callable[[float], None]] = None,
    utc_fn: Optional[Callable[[], str]] = None,
    getpid_fn: Optional[Callable[[], int]] = None,
    stop_event: Optional[threading.Event] = None,
    install_signal_handlers: bool = True,
    sample_timeout: Optional[float] = None,
    rusage_children_fn: Optional[Callable[[], Any]] = None,
) -> int:
    """Run continuous multi-role accounting.

    duration: finite seconds, or None for stop-event/signal controlled only.
    stop_event: when set, ends the loop.
      - fixed duration + early stop → incomplete / exit 1
      - stop_controlled (duration=None) + requested stop → closed / controlled_stop / exit 0
    Returns 0 on clean full-duration completion or intentional controlled stop;
    1 on failure or incomplete early stop under fixed duration.
    Exclusive output create only (no force).
    """
    if interval <= 0 or not math.isfinite(interval):
        raise AccountingError("interval must be a finite positive number")
    if duration is not None and (duration <= 0 or not math.isfinite(duration)):
        raise AccountingError("duration must be finite and positive when provided")

    resolved_timeout = _resolve_sample_timeout(duration=duration, sample_timeout=sample_timeout)

    # Stop control: reject hang if neither duration, stop_event, nor working signals.
    local_stop = stop_event or threading.Event()
    previous_handlers: Dict[Any, Any] = {}
    signals_installed = 0

    def _request_stop(signum=None, frame=None) -> None:  # noqa: ARG001
        local_stop.set()

    if install_signal_handlers:
        for sig in (signal.SIGINT, signal.SIGTERM):
            try:
                previous_handlers[sig] = signal.signal(sig, _request_stop)
                signals_installed += 1
            except (ValueError, OSError):
                # Not in main thread or unsupported.
                pass

    if duration is None and stop_event is None and signals_installed == 0:
        raise AccountingError(
            "stop-controlled run requires a finite duration, an external stop_event, "
            "or installable signal handlers; refusing to hang"
        )

    grid_required: Optional[int] = None
    if duration is not None:
        grid_required = required_grid_count(duration, interval)

    resolved = normalize_roles(
        roles, include_collector_self=include_collector_self, getpid_fn=getpid_fn
    )
    out = open_output(output)
    monotonic_fn = monotonic_fn or time.monotonic
    sleep_fn = sleep_fn or time.sleep
    sample_fn = sample_fn or sr.sample_process
    pressure_fn = pressure_fn or sr.sample_pressure
    utc_fn = utc_fn or _utc_now

    monotonic_start = monotonic_fn()
    previous: Dict[str, Dict[str, Any]] = {}
    count = 0
    status = "ok"
    fail_reason: Optional[str] = None
    stop_reason: Optional[str] = None
    cadence_tolerance = min(0.5, max(0.05, interval * 0.25))
    role_first_ok: Dict[str, float] = {}
    role_last_ok: Dict[str, float] = {}
    role_errors: List[Dict[str, Any]] = []
    samples_completed_ok = 0

    def write(row: Dict[str, Any]) -> None:
        out.write(json.dumps(row, sort_keys=True) + "\n")
        out.flush()

    try:
        write(
            {
                "type": "metadata",
                "schema": SCHEMA,
                "roles": {name: {"pid": pid} for name, pid in sorted(resolved.items())},
                "interval_s": interval,
                "duration_s": duration,
                "duration_mode": "fixed" if duration is not None else "stop_controlled",
                "required_grid_sample_count": grid_required,
                "sample_timeout_s": resolved_timeout,
                "cadence_tolerance_s": cadence_tolerance,
                "started_utc": utc_fn(),
                "clock": "time.monotonic() for elapsed/delta; datetime UTC for wall timestamp",
                "scope": scope_notes(resolved),
                "pressure": "host-wide and separate from per-role process records",
                "sampling_overhead": "unmeasured protocol; collector role series is diagnostic only",
                "no_discovery": True,
                "no_foreign_signaling": True,
                "exclusive_create": True,
                "overwrite_permitted": False,
                "bracketing": {
                    "tick": [
                        "scheduled_monotonic_s",
                        "acquisition_start_monotonic_s",
                        "acquisition_end_monotonic_s",
                        "acquisition_duration_s",
                        "lateness_s",
                    ],
                    "per_role": [
                        "acquisition_start_monotonic_s",
                        "acquisition_end_monotonic_s",
                        "acquisition_duration_s",
                    ],
                    "cpu_interval_basis": "per_role_acquisition_start",
                    "run": ["monotonic_start", "first_ok / last_ok per role", "observed_span_s"],
                },
                "children_cpu": "resource.getrusage(RUSAGE_CHILDREN) cumulative+delta per sweep when available",
                "children_rss": "current unavailable; maxrss high-water not emitted as current",
            }
        )

        deadline = (monotonic_start + duration) if duration is not None else None

        while True:
            if local_stop.is_set():
                stop_reason = "orchestrator_stop"
                if duration is None:
                    status = "closed"
                else:
                    status = "incomplete"
                break

            scheduled = monotonic_start + count * interval
            if deadline is not None and scheduled >= deadline:
                # Past last grid slot — exit loop; coverage checked after.
                break

            now = monotonic_fn()
            if now < scheduled:
                # Sleep in small slices so stop_event/signals remain responsive.
                while True:
                    if local_stop.is_set():
                        stop_reason = "orchestrator_stop"
                        if duration is None:
                            status = "closed"
                        else:
                            status = "incomplete"
                        break
                    now = monotonic_fn()
                    if now >= scheduled:
                        break
                    if deadline is not None and now >= deadline:
                        break
                    slice_s = min(0.05, scheduled - now)
                    if deadline is not None:
                        slice_s = min(slice_s, max(0.0, deadline - now))
                    if slice_s <= 0:
                        break
                    sleep_fn(slice_s)
                if status in ("incomplete", "closed"):
                    break
                # Duration exhausted while waiting for a still-required scheduled tick.
                if deadline is not None and monotonic_fn() >= deadline:
                    if scheduled < deadline:
                        status = "fail"
                        fail_reason = (
                            f"duration exhausted before required grid sample index {count} "
                            f"(scheduled_monotonic_s={scheduled})"
                        )
                    break

            # Late clock / jump: already past the slot window for this required tick.
            acquisition_tick_start = monotonic_fn()
            if deadline is not None and acquisition_tick_start >= deadline and scheduled < deadline:
                status = "fail"
                fail_reason = (
                    f"duration exhausted before required grid sample index {count} "
                    f"(scheduled_monotonic_s={scheduled})"
                )
                break

            lateness = max(0.0, acquisition_tick_start - scheduled)
            if lateness > cadence_tolerance:
                status = "fail"
                fail_reason = (
                    f"sample cadence missed required deadline at index {count} "
                    f"(lateness_s={lateness})"
                )
                # Still attempt? No — fail closed without claiming the tick.
                break

            tick_roles: Dict[str, Any] = {}
            tick_failed = False
            tick_reason: Optional[str] = None
            children_before = _children_rusage_snapshot(rusage_children_fn)
            tick_acq_start = monotonic_fn()

            # Deterministic role order; per-role real brackets + remaining timeout recompute.
            for name, pid in sorted(resolved.items()):
                role_start = monotonic_fn()
                if deadline is not None:
                    remaining = deadline - role_start
                    if remaining <= 0:
                        tick_failed = True
                        tick_reason = (
                            f"role {name!r}: duration exhausted before per-role acquisition "
                            f"at sample index {count}"
                        )
                        tick_roles[name] = {
                            "role": name,
                            "pid": pid,
                            "status": "error",
                            "reason": tick_reason,
                            "acquisition_start_monotonic_s": role_start,
                            "acquisition_end_monotonic_s": role_start,
                            "acquisition_duration_s": 0.0,
                        }
                        role_errors.append(
                            {
                                "role": name,
                                "pid": pid,
                                "utc": utc_fn(),
                                "monotonic_s": role_start,
                                "reason": tick_reason,
                                "sample_index": count,
                            }
                        )
                        break
                    per_role_timeout = min(remaining, resolved_timeout)
                else:
                    per_role_timeout = resolved_timeout
                if per_role_timeout <= 0 or not math.isfinite(per_role_timeout):
                    tick_failed = True
                    tick_reason = f"role {name!r}: non-positive sample timeout"
                    break
                try:
                    raw = sample_fn(pid, timeout=per_role_timeout)
                    current = validate_sample_dict(raw, role=name)
                    role_end = monotonic_fn()
                    row = _role_sample_row(
                        name, pid, current, previous.get(name), role_start, role_end
                    )
                    tick_roles[name] = row
                    previous[name] = dict(current, _monotonic=role_start)
                    if name not in role_first_ok:
                        role_first_ok[name] = role_start
                    role_last_ok[name] = role_end
                except (sr.SampleError, OSError, TypeError, KeyError, ValueError) as exc:
                    role_end = monotonic_fn()
                    tick_failed = True
                    tick_reason = f"role {name!r}: {exc}"
                    err = {
                        "role": name,
                        "pid": pid,
                        "status": "error",
                        "reason": str(exc),
                        "acquisition_start_monotonic_s": role_start,
                        "acquisition_end_monotonic_s": role_end,
                        "acquisition_duration_s": role_end - role_start,
                    }
                    tick_roles[name] = err
                    role_errors.append(
                        {
                            "role": name,
                            "pid": pid,
                            "utc": utc_fn(),
                            "monotonic_s": role_start,
                            "reason": str(exc),
                            "sample_index": count,
                        }
                    )
                    # Fail closed: do not continue remaining roles as if ok.
                    break

            tick_acq_end = monotonic_fn()
            children_after = _children_rusage_snapshot(rusage_children_fn)
            children_cost = _children_cost_delta(children_before, children_after)

            try:
                host_pressure = pressure_fn()
            except Exception as exc:  # noqa: BLE001 — fail closed on malformed pressure
                status = "fail"
                fail_reason = f"host_pressure failed: {exc}"
                write(
                    {
                        "type": "sample",
                        "utc": utc_fn(),
                        "monotonic_s": tick_acq_start,
                        "elapsed_s": tick_acq_start - monotonic_start,
                        "scheduled_monotonic_s": scheduled,
                        "acquisition_start_monotonic_s": tick_acq_start,
                        "acquisition_end_monotonic_s": tick_acq_end,
                        "acquisition_duration_s": tick_acq_end - tick_acq_start,
                        "lateness_s": lateness,
                        "sample_index": count,
                        "roles": tick_roles,
                        "host_pressure": {
                            "status": "error",
                            "reason": str(exc),
                        },
                        "ps_child_cost": children_cost,
                        "waited_children_cpu": children_cost,
                    }
                )
                count += 1
                break

            write(
                {
                    "type": "sample",
                    "utc": utc_fn(),
                    "monotonic_s": tick_acq_start,
                    "elapsed_s": tick_acq_start - monotonic_start,
                    "scheduled_monotonic_s": scheduled,
                    "acquisition_start_monotonic_s": tick_acq_start,
                    "acquisition_end_monotonic_s": tick_acq_end,
                    "acquisition_duration_s": tick_acq_end - tick_acq_start,
                    "lateness_s": lateness,
                    "sample_index": count,
                    "roles": tick_roles,
                    "host_pressure": host_pressure,
                    # Historical key retained; value is waited-children cost (CPU when available).
                    "ps_child_cost": children_cost,
                    "waited_children_cpu": children_cost,
                }
            )
            count += 1

            if tick_failed:
                status = "fail"
                fail_reason = tick_reason
                break

            samples_completed_ok += 1

            # Acquisition overran the cadence window for this tick.
            if tick_acq_end > scheduled + interval + cadence_tolerance:
                status = "fail"
                fail_reason = "sample cadence missed required deadline"
                break

            if local_stop.is_set():
                stop_reason = "orchestrator_stop"
                if duration is None:
                    status = "closed"
                else:
                    status = "incomplete"
                break

        # Fixed-duration required-grid coverage (honest; no false full_duration).
        if duration is not None and status == "ok":
            assert grid_required is not None
            if samples_completed_ok < grid_required:
                status = "fail"
                fail_reason = (
                    f"required grid incomplete: got {samples_completed_ok} ok samples, "
                    f"required {grid_required} "
                    f"(duration_s={duration}, interval_s={interval})"
                )

        end_mono = monotonic_fn()
        lifetimes = {}
        for name, pid in sorted(resolved.items()):
            first = role_first_ok.get(name)
            last = role_last_ok.get(name)
            observed = None if first is None or last is None else max(0.0, last - first)
            complete = bool(
                duration is not None
                and status == "ok"
                and grid_required is not None
                and samples_completed_ok >= grid_required
                and first is not None
                and last is not None
            )
            lifetimes[name] = {
                "pid": pid,
                "first_ok_monotonic_s": first,
                "last_ok_monotonic_s": last,
                # Honest wall of samples actually observed — not configured duration.
                "observed_span_s": observed,
                "run_elapsed_s": end_mono - monotonic_start,
                "complete_for_required_grid": complete,
                "complete_for_configured_duration": bool(complete and observed is not None and observed >= duration),
                "full_duration_claim": bool(complete and observed is not None and observed >= duration),
            }

        if status == "ok" and duration is not None:
            completion = "required_grid_complete"
        elif status == "closed" and duration is None and stop_reason == "orchestrator_stop":
            completion = "controlled_stop"
        elif status == "incomplete":
            completion = "partial_orchestrator_stop"
        else:
            completion = "failed_or_incomplete"

        covered_span = None
        if len(role_first_ok) == len(resolved) and len(role_last_ok) == len(resolved):
            # Coverage shared by every role, not the union of staggered ranges.
            covered_span = max(0.0, min(role_last_ok.values()) - max(role_first_ok.values()))

        summary: Dict[str, Any] = {
            "type": "summary",
            "status": status,
            "sample_count": count,
            "ok_sample_count": samples_completed_ok,
            "required_grid_sample_count": grid_required,
            "grid_complete": bool(
                duration is not None
                and grid_required is not None
                and samples_completed_ok >= grid_required
                and status == "ok"
            ),
            "ended_utc": utc_fn(),
            "elapsed_s": end_mono - monotonic_start,
            "configured_duration_s": duration,
            "observed_covered_span_s": covered_span,
            "stop_reason": stop_reason,
            "fail_reason": fail_reason,
            "completion": completion,
            "role_lifetimes": lifetimes,
            "role_errors": role_errors,
            "sampling_overhead": "unmeasured; no matched overhead proof",
            "scope_gaps": scope_notes(resolved)["not_measured"],
            "bracketing": {
                "monotonic_start_s": monotonic_start,
                "monotonic_end_s": end_mono,
                "per_role_first_last_ok": {
                    name: {
                        "first_ok_monotonic_s": lifetimes[name]["first_ok_monotonic_s"],
                        "last_ok_monotonic_s": lifetimes[name]["last_ok_monotonic_s"],
                        "observed_span_s": lifetimes[name]["observed_span_s"],
                    }
                    for name in lifetimes
                },
            },
            "reader_note": (
                "stop_controlled controlled_stop is not full observation coverage; "
                "readers must independently require required_grid / observed span"
            ),
        }
        write(summary)

        if status == "ok":
            return 0
        if status == "closed" and completion == "controlled_stop":
            return 0
        if fail_reason:
            print(f"FAIL: {fail_reason}", file=sys.stderr)
        elif stop_reason:
            print(f"INCOMPLETE: {stop_reason}", file=sys.stderr)
        return 1
    except (AccountingError, sr.SampleError, OSError) as exc:
        try:
            write(
                {
                    "type": "error",
                    "status": "fail",
                    "reason": str(exc),
                    "partial_sample_count": count,
                    "ended_utc": utc_fn(),
                }
            )
        except OSError:
            pass
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1
    finally:
        for sig, handler in previous_handlers.items():
            try:
                signal.signal(sig, handler)
            except (ValueError, OSError):
                pass
        out.close()


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "roles",
        nargs="+",
        metavar="ROLE=PID",
        help="explicit role to PID mapping (no discovery); unique PIDs only",
    )
    parser.add_argument(
        "output",
        type=pathlib.Path,
        help="new JSONL output path (exclusive create; no overwrite)",
    )
    parser.add_argument("--interval", type=float, default=1.0, help="sample cadence seconds")
    parser.add_argument(
        "--duration",
        type=float,
        default=None,
        help="finite capture seconds; omit for signal/stop-controlled only",
    )
    parser.add_argument(
        "--no-collector-self",
        action="store_true",
        help="do not auto-add collector=self PID (still allowed if passed explicitly)",
    )
    parser.add_argument(
        "--sample-timeout",
        type=float,
        default=None,
        help=(
            f"per-role sample timeout seconds (default {DEFAULT_SAMPLE_TIMEOUT_S}; "
            "always finite positive; recomputed vs remaining duration each role)"
        ),
    )
    args = parser.parse_args(argv)
    try:
        role_map = parse_role_specs(args.roles)
        return run(
            role_map,
            args.output,
            args.interval,
            args.duration,
            include_collector_self=not args.no_collector_self,
            sample_timeout=args.sample_timeout,
        )
    except (AccountingError, sr.SampleError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
