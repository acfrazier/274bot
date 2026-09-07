#!/usr/bin/env python3
"""Continuous explicit-PID multi-role process resource accounting.

Named roles map to caller-supplied PIDs only. This module never discovers
processes by name/argv/env, never starts or signals foreign processes, and
never records secrets. It reuses server_resources sampling helpers.

Output is streaming JSONL (exclusive create). Missing identity, counter reset,
cadence loss, or required-role sample failure yields exit 1 with partial
records retained. Orchestrator stop (signal or stop event) records honest
partial lifetimes — never a false full-duration ok.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import signal
import sys
import threading
import time
from datetime import datetime, timezone
from typing import Any, Callable, Dict, List, Mapping, MutableMapping, Optional, Sequence, TextIO, Tuple

# Adjacent helper module (same directory).
_ROOT = pathlib.Path(__file__).resolve().parent
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

import server_resources as sr  # noqa: E402

SCHEMA = 1
COLLECTOR_ROLE = "collector"


class AccountingError(sr.SampleError):
    """Config or accounting invariant failure (inherits SampleError for reuse)."""


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


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
    out = dict(roles)
    if not out:
        raise AccountingError("at least one role is required")
    seen: Dict[int, str] = {}
    for name, pid in out.items():
        if not isinstance(name, str) or not name.strip():
            raise AccountingError("role name must be a non-empty string")
        if name != name.strip():
            raise AccountingError(f"role name must not have surrounding whitespace: {name!r}")
        sr.validate_pid(pid)
        if pid in seen:
            raise AccountingError(
                f"duplicate PID {pid} for roles {seen[pid]!r} and {name!r}; "
                "do not double-count — map each PID to one role"
            )
        seen[pid] = name
    if include_collector_self:
        self_pid = int(getpid_fn())
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


def scope_notes(roles: Mapping[str, int]) -> Dict[str, Any]:
    """Document measured scope and known unaccounted gaps (not a full tree)."""
    return {
        "measured": (
            "per-role current RSS and cumulative user/system CPU for explicit root PIDs only; "
            "collector self included when configured; host pressure via sample_pressure"
        ),
        "not_measured": [
            "descendant/child processes of any role (not a process tree walk)",
            "short-lived sample helper subprocess CPU/RSS when OS path uses ps "
            "(macOS); acquisition wall time is recorded, child CPU is unaccounted",
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
    }


def open_output(path: pathlib.Path, *, force: bool = False) -> TextIO:
    return sr.open_output(path, force=force)


def _role_sample_row(
    name: str,
    pid: int,
    current: Dict[str, Any],
    previous: Optional[Dict[str, Any]],
    acquisition_start: float,
) -> Dict[str, Any]:
    if previous is not None:
        sr.require_same_identity(previous, current)
        delta = sr.cpu_delta_seconds(previous, current)
        elapsed = acquisition_start - float(previous["_monotonic"])
        if elapsed <= 0:
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
        "cpu": {
            "cumulative_user_s": current["user_s"],
            "cumulative_system_s": current["system_s"],
            "delta_user_s": delta["user_s"],
            "delta_system_s": delta["system_s"],
            "delta_total_s": delta["total_s"],
            "cores_delta": cores,
        },
        "process": {k: v for k, v in current.items() if k not in ("user_s", "system_s")},
    }


def run(
    roles: Mapping[str, int],
    output: pathlib.Path,
    interval: float,
    duration: Optional[float] = None,
    *,
    force: bool = False,
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
) -> int:
    """Run continuous multi-role accounting.

    duration: finite seconds, or None for stop-event/signal controlled only.
    stop_event: when set, ends the loop with status incomplete/stopped.
    Returns 0 on clean full-duration completion; 1 on failure or incomplete stop.
    """
    if interval <= 0 or not math.isfinite(interval):
        raise AccountingError("interval must be a finite positive number")
    if duration is not None and (duration <= 0 or not math.isfinite(duration)):
        raise AccountingError("duration must be finite and positive when provided")
    if duration is None and stop_event is None and not install_signal_handlers:
        raise AccountingError("duration is required unless stop_event or signals can stop the run")

    resolved = normalize_roles(
        roles, include_collector_self=include_collector_self, getpid_fn=getpid_fn
    )
    out = open_output(output, force=force)
    monotonic_fn = monotonic_fn or time.monotonic
    sleep_fn = sleep_fn or time.sleep
    sample_fn = sample_fn or sr.sample_process
    pressure_fn = pressure_fn or sr.sample_pressure
    utc_fn = utc_fn or _utc_now

    local_stop = stop_event or threading.Event()
    previous_handlers: Dict[Any, Any] = {}

    def _request_stop(signum=None, frame=None) -> None:  # noqa: ARG001
        local_stop.set()

    if install_signal_handlers:
        for sig in (signal.SIGINT, signal.SIGTERM):
            try:
                previous_handlers[sig] = signal.signal(sig, _request_stop)
            except (ValueError, OSError):
                # Not in main thread or unsupported — rely on stop_event/duration.
                pass

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
                "cadence_tolerance_s": cadence_tolerance,
                "started_utc": utc_fn(),
                "clock": "time.monotonic() for elapsed/delta; datetime UTC for wall timestamp",
                "scope": scope_notes(resolved),
                "pressure": "host-wide and separate from per-role process records",
                "sampling_overhead": "unmeasured protocol; collector role series is diagnostic only",
                "no_discovery": True,
                "no_foreign_signaling": True,
            }
        )

        deadline = (monotonic_start + duration) if duration is not None else None

        while True:
            if local_stop.is_set():
                stop_reason = "orchestrator_stop"
                status = "incomplete"
                break

            scheduled = monotonic_start + count * interval
            if deadline is not None and scheduled >= deadline:
                break

            now = monotonic_fn()
            if now < scheduled:
                # Sleep in small slices so stop_event/signals remain responsive.
                while True:
                    if local_stop.is_set():
                        stop_reason = "orchestrator_stop"
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
                if status == "incomplete":
                    break
                if deadline is not None and monotonic_fn() >= deadline and monotonic_fn() < scheduled:
                    break

            acquisition_start = monotonic_fn()
            if deadline is not None:
                remaining = deadline - acquisition_start
                if remaining <= 0:
                    # Duration exhausted before this sample slot.
                    break
                per_sample_timeout = remaining if sample_timeout is None else min(remaining, sample_timeout)
            else:
                per_sample_timeout = sample_timeout

            tick_roles: Dict[str, Any] = {}
            tick_failed = False
            tick_reason: Optional[str] = None

            # Deterministic role order.
            for name, pid in sorted(resolved.items()):
                try:
                    current = sample_fn(pid, timeout=per_sample_timeout)
                    row = _role_sample_row(
                        name, pid, current, previous.get(name), acquisition_start
                    )
                    tick_roles[name] = row
                    previous[name] = dict(current, _monotonic=acquisition_start)
                    if name not in role_first_ok:
                        role_first_ok[name] = acquisition_start
                    role_last_ok[name] = acquisition_start
                except (sr.SampleError, OSError, TypeError, KeyError, ValueError) as exc:
                    tick_failed = True
                    tick_reason = f"role {name!r}: {exc}"
                    err = {
                        "role": name,
                        "pid": pid,
                        "status": "error",
                        "reason": str(exc),
                    }
                    tick_roles[name] = err
                    role_errors.append(
                        {
                            "role": name,
                            "pid": pid,
                            "utc": utc_fn(),
                            "monotonic_s": acquisition_start,
                            "reason": str(exc),
                            "sample_index": count,
                        }
                    )
                    # Fail closed: do not continue sampling remaining roles as if ok.
                    # Still record whatever was collected this tick.
                    break

            acquisition_end = monotonic_fn()
            acquisition_duration = acquisition_end - acquisition_start
            lateness = max(0.0, acquisition_start - scheduled)

            write(
                {
                    "type": "sample",
                    "utc": utc_fn(),
                    "monotonic_s": acquisition_start,
                    "elapsed_s": acquisition_start - monotonic_start,
                    "scheduled_monotonic_s": scheduled,
                    "acquisition_start_monotonic_s": acquisition_start,
                    "acquisition_end_monotonic_s": acquisition_end,
                    "acquisition_duration_s": acquisition_duration,
                    "lateness_s": lateness,
                    "sample_index": count,
                    "roles": tick_roles,
                    "host_pressure": pressure_fn(),
                    "ps_child_cost": {
                        "status": "unaccounted",
                        "note": (
                            "macOS path may spawn short-lived ps; CPU/RSS of that child is not "
                            "attributed; acquisition_duration_s is wall time only"
                        ),
                    },
                }
            )
            count += 1

            if tick_failed:
                status = "fail"
                fail_reason = tick_reason
                break

            if lateness > cadence_tolerance or acquisition_end > scheduled + interval + cadence_tolerance:
                status = "fail"
                fail_reason = "sample cadence missed required deadline"
                break

            if local_stop.is_set():
                stop_reason = "orchestrator_stop"
                status = "incomplete"
                break

        # Lifetime brackets per role (honest partials on early stop).
        lifetimes = {}
        end_mono = monotonic_fn()
        for name, pid in sorted(resolved.items()):
            first = role_first_ok.get(name)
            last = role_last_ok.get(name)
            lifetimes[name] = {
                "pid": pid,
                "first_ok_monotonic_s": first,
                "last_ok_monotonic_s": last,
                "observed_span_s": (None if first is None or last is None else max(0.0, last - first)),
                "full_run_span_s": end_mono - monotonic_start,
                "complete_for_configured_duration": bool(
                    duration is not None
                    and status == "ok"
                    and first is not None
                    and last is not None
                ),
            }

        summary: Dict[str, Any] = {
            "type": "summary",
            "status": status,
            "sample_count": count,
            "ended_utc": utc_fn(),
            "elapsed_s": end_mono - monotonic_start,
            "configured_duration_s": duration,
            "stop_reason": stop_reason,
            "fail_reason": fail_reason,
            "role_lifetimes": lifetimes,
            "role_errors": role_errors,
            "sampling_overhead": "unmeasured; no matched overhead proof",
            "scope_gaps": scope_notes(resolved)["not_measured"],
        }
        if status == "ok" and duration is not None:
            summary["completion"] = "full_duration"
        elif status == "incomplete":
            summary["completion"] = "partial_orchestrator_stop"
        else:
            summary["completion"] = "failed_or_incomplete"
        write(summary)

        if status == "ok":
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
    parser.add_argument("output", type=pathlib.Path, help="new JSONL output path (exclusive create)")
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
    parser.add_argument("--force", action="store_true", help="permit replacing existing output")
    parser.add_argument(
        "--sample-timeout",
        type=float,
        default=None,
        help="optional per-role sample timeout seconds (also bounded by remaining duration)",
    )
    args = parser.parse_args(argv)
    try:
        role_map = parse_role_specs(args.roles)
        return run(
            role_map,
            args.output,
            args.interval,
            args.duration,
            force=args.force,
            include_collector_self=not args.no_collector_self,
            sample_timeout=args.sample_timeout,
        )
    except (AccountingError, sr.SampleError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
