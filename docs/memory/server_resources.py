#!/usr/bin/env python3
"""Bounded, non-invasive game-server process and host-pressure sampler.

The target is an explicit PID. This module never starts, finds, resets, or
stops a process and never records argv or environment data. Output is JSONL so
partial diagnostic records survive a required-sample failure.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import platform
import subprocess
import sys
import time
from datetime import datetime, timezone
from typing import Any, Dict, Optional, TextIO


class SampleError(RuntimeError):
    """A required identity/counter/sample invariant was not met."""


def validate_pid(pid: int) -> int:
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        raise SampleError("PID must be a positive integer")
    return pid


def parse_proc_stat(text: str) -> Dict[str, Any]:
    """Parse Linux /proc/PID/stat without splitting the command name naively."""
    close = text.rfind(")")
    if close < 0 or close + 2 >= len(text):
        raise SampleError("malformed /proc/PID/stat")
    fields = text[close + 2 :].split()
    # fields[0] is stat field 3 (state); starttime is field 22, utime 14, stime 15.
    if len(fields) < 20:
        raise SampleError("short /proc/PID/stat")
    try:
        return {
            "state": fields[0],
            "utime_ticks": int(fields[11]),
            "stime_ticks": int(fields[12]),
            "start_ticks": int(fields[19]),
        }
    except (TypeError, ValueError) as exc:
        raise SampleError("non-numeric /proc/PID/stat counter") from exc


def parse_psi(text: str) -> Dict[str, Any]:
    """Parse one Linux PSI line; averages are percentages, total is usec."""
    parts = text.strip().split()
    if not parts or parts[0] not in ("some", "full"):
        raise SampleError("malformed PSI memory line")
    values: Dict[str, Any] = {"stall_scope": parts[0]}
    for item in parts[1:]:
        key, sep, value = item.partition("=")
        if not sep:
            continue
        try:
            values[{"avg10": "avg10_percent", "avg60": "avg60_percent", "avg300": "avg300_percent", "total": "total_us"}[key]] = float(value) if key != "total" else int(value)
        except (KeyError, ValueError) as exc:
            raise SampleError("malformed PSI value") from exc
    if not all(k in values for k in ("avg10_percent", "avg60_percent", "avg300_percent", "total_us")):
        raise SampleError("incomplete PSI memory line")
    values["semantics"] = "fraction_of_wall_time_stalled"
    return values


def _parse_ps_time(value: str) -> float:
    value = value.strip()
    bits = value.split(":")
    try:
        if len(bits) == 2:
            minutes, seconds = bits
            return float(minutes) * 60.0 + float(seconds)
        if len(bits) == 3:
            hours, minutes, seconds = bits
            return float(hours) * 3600.0 + float(minutes) * 60.0 + float(seconds)
        return float(value)
    except ValueError as exc:
        raise SampleError("malformed ps CPU time") from exc


def require_same_identity(previous: Dict[str, Any], current: Dict[str, Any]) -> None:
    if previous.get("start_identity") != current.get("start_identity"):
        raise SampleError("PID identity changed (process exited and was reused)")


def cpu_delta_seconds(previous: Dict[str, Any], current: Dict[str, Any]) -> Dict[str, float]:
    user = float(current["user_s"]) - float(previous["user_s"])
    system = float(current["system_s"]) - float(previous["system_s"])
    if user < 0 or system < 0:
        raise SampleError("process CPU counter reset")
    return {"user_s": user, "system_s": system, "total_s": user + system}


def open_output(path: pathlib.Path, *, force: bool) -> TextIO:
    if not path.parent.is_dir():
        raise SampleError("output parent directory does not exist")
    if path.is_symlink():
        raise SampleError("refusing to write through a symlink output path")
    try:
        return path.open("w" if force else "x", encoding="utf-8")
    except FileExistsError as exc:
        raise SampleError("refusing to overwrite existing output; use --force") from exc


def _linux_sample(pid: int) -> Dict[str, Any]:
    proc = pathlib.Path("/proc") / str(pid)
    try:
        stat = parse_proc_stat((proc / "stat").read_text())
        page_size = os.sysconf("SC_PAGE_SIZE")
        rss_pages = int((proc / "statm").read_text().split()[1])
    except (OSError, IndexError, ValueError) as exc:
        raise SampleError(f"required Linux process sample unreadable: {exc}") from exc
    ticks = os.sysconf("SC_CLK_TCK")
    return {
        "start_identity": f"linux_proc_start_ticks:{stat['start_ticks']}",
        "state": stat["state"],
        "resident_bytes": rss_pages * page_size,
        "user_s": stat["utime_ticks"] / ticks,
        "system_s": stat["stime_ticks"] / ticks,
        "provenance": {
            "os": "linux",
            "rss": f"/proc/{pid}/statm resident pages × SC_PAGE_SIZE (current resident set)",
            "cpu": f"/proc/{pid}/stat utime/stime ÷ SC_CLK_TCK (cumulative process CPU)",
            "identity": f"/proc/{pid}/stat field 22 starttime ticks",
        },
    }


def _mac_sample(pid: int) -> Dict[str, Any]:
    # ps is queried only for the explicit PID; no process discovery is done.
    command = ["/bin/ps", "-o", "rss=", "-o", "utime=", "-o", "stime=", "-o", "lstart=", "-p", str(pid)]
    try:
        result = subprocess.run(command, check=True, capture_output=True, text=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        raise SampleError(f"required macOS ps sample unreadable: {exc}") from exc
    line = result.stdout.strip()
    if not line:
        raise SampleError("macOS ps reported no such PID")
    bits = line.split(None, 5)
    if len(bits) < 6:
        raise SampleError("malformed macOS ps output")
    try:
        rss_kib = int(bits[0])
        user_s = _parse_ps_time(bits[1])
        system_s = _parse_ps_time(bits[2])
    except ValueError as exc:
        raise SampleError("malformed macOS ps counters") from exc
    return {
        "start_identity": "macos_lstart:" + bits[3] + " " + bits[4] + " " + bits[5],
        "resident_bytes": rss_kib * 1024,
        "user_s": user_s,
        "system_s": system_s,
        "provenance": {
            "os": "macos",
            "rss": f"/bin/ps -o rss= -p {pid}, KiB (current resident set)",
            "cpu": f"/bin/ps -o utime=,stime= -p {pid} (cumulative process CPU)",
            "identity": f"/bin/ps -o lstart= -p {pid} (process start wall time)",
        },
    }


def sample_process(pid: int) -> Dict[str, Any]:
    validate_pid(pid)
    if sys.platform.startswith("linux"):
        return _linux_sample(pid)
    if sys.platform == "darwin":
        return _mac_sample(pid)
    raise SampleError(f"unsupported OS: {platform.system()}")


def sample_pressure() -> Dict[str, Any]:
    if sys.platform.startswith("linux"):
        path = pathlib.Path("/proc/pressure/memory")
        try:
            lines = path.read_text().splitlines()
            parsed = [parse_psi(line) for line in lines if line.strip()]
        except (OSError, SampleError) as exc:
            return {"status": "unavailable", "reason": "required_pressure_counter_unreadable", "provenance": str(path), "detail": str(exc)}
        return {"status": "available", "source": str(path), "semantics": "Linux PSI memory stall time; avg fields are percent over windows, total is microseconds", "lines": parsed}
    return {"status": "unavailable", "reason": "unsupported_pressure_counter", "semantics": "no portable macOS pressure counter selected; unavailable is not zero or healthy", "provenance": "none"}


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def run(pid: int, output: pathlib.Path, interval: float, duration: float, *, force: bool = False) -> int:
    validate_pid(pid)
    if interval <= 0 or duration <= 0 or not math.isfinite(interval) or not math.isfinite(duration):
        raise SampleError("interval and duration must be finite positive numbers")
    out = open_output(output, force=force)
    monotonic_start = time.monotonic()
    previous: Optional[Dict[str, Any]] = None
    count = 0
    try:
        def write(row: Dict[str, Any]) -> None:
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()

        write({"type": "metadata", "schema": 1, "pid": pid, "interval_s": interval, "duration_s": duration, "started_utc": _utc_now(), "clock": "time.monotonic() for elapsed/delta; datetime UTC for wall timestamp", "scope": "explicit root PID only; children are not included", "pressure": "host-wide and separate from process records"})
        deadline = monotonic_start + duration
        while True:
            now = time.monotonic()
            if now > deadline and count:
                break
            current = sample_process(pid)
            if previous is not None:
                require_same_identity(previous, current)
                delta = cpu_delta_seconds(previous, current)
                elapsed = now - previous["_monotonic"]
                if elapsed <= 0:
                    raise SampleError("non-positive monotonic sample interval")
                cores = delta["total_s"] / elapsed
            else:
                delta = {"user_s": 0.0, "system_s": 0.0, "total_s": 0.0}
                cores = None
            row = {"type": "sample", "utc": _utc_now(), "monotonic_s": now, "elapsed_s": now - monotonic_start, "resident_bytes": current["resident_bytes"], "cpu": {"cumulative_user_s": current["user_s"], "cumulative_system_s": current["system_s"], "delta_user_s": delta["user_s"], "delta_system_s": delta["system_s"], "delta_total_s": delta["total_s"], "cores_delta": cores}, "process": {k: v for k, v in current.items() if k not in ("user_s", "system_s")}, "host_pressure": sample_pressure()}
            write(row)
            previous = dict(current, _monotonic=now)
            count += 1
            if time.monotonic() >= deadline:
                break
            time.sleep(min(interval, max(0.0, deadline - time.monotonic())))
        write({"type": "summary", "status": "ok", "sample_count": count, "ended_utc": _utc_now(), "sampling_overhead": "unmeasured; no matched overhead proof"})
        return 0
    except (SampleError, OSError) as exc:
        write({"type": "error", "status": "fail", "reason": str(exc), "partial_sample_count": count, "ended_utc": _utc_now()})
        return 1
    finally:
        out.close()


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", type=int, help="explicit game-server PID; never discovered")
    parser.add_argument("output", type=pathlib.Path, help="new JSONL output path")
    parser.add_argument("--interval", type=float, default=1.0)
    parser.add_argument("--duration", type=float, required=True, help="finite sampling duration in seconds")
    parser.add_argument("--force", action="store_true", help="explicitly permit replacing an existing output")
    args = parser.parse_args(argv)
    try:
        return run(args.pid, args.output, args.interval, args.duration, force=args.force)
    except SampleError as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
