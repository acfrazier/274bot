#!/usr/bin/env python3
"""Independent recomputation of the single longer focused-one N16 pair."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ARCHIVE_ROOT = ROOT / "diagnostics/windows-tile-boxed-long-20260908"
OUT = Path(__file__).with_name("windows-tile-boxed-long-screen-table.json")
CELLS = [
    ("baseline", "baseline-focused-one-long-native-20260908-0250", "reference"),
    ("candidate", "candidate-focused-one-long-native-20260908-0250", "candidate"),
]
EXPECTED_TARS = {
    "baseline": "b580284cd6ef1eb5766877e8d4543b853c0e6c55a4c8daa3612932856f5907cf",
    "candidate": "f651fbda0fe19539512bf81cc2ad2b665aa686844c011af124c76bf50a3374ad",
}
MIB = 1024.0 * 1024.0


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def jsonl(path):
    return [json.loads(line) for line in Path(path).read_text(encoding="utf-8-sig").splitlines() if line.strip()]


def sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as fh:
        for block in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def nums(rows, key):
    return [r[key] for r in rows if isinstance(r.get(key), (int, float))]


def mib(value):
    return None if value is None else value / MIB


def rng(rows, key):
    values = nums(rows, key)
    return [min(values), max(values)] if values else None


def aggregate(rows):
    elapsed = nums(rows, "elapsed_s")
    user = nums(rows, "process_cpu_user_s")
    system = nums(rows, "process_cpu_system_s")
    rss = nums(rows, "resident_bytes")
    peak = nums(rows, "peak_resident_bytes")
    private = nums(rows, "windows_private_commit_bytes")
    gpu = nums(rows, "gpu_tracked_bytes")
    duration = elapsed[-1] - elapsed[0] if len(elapsed) > 1 else None
    cpu = ((user[-1] - user[0]) + (system[-1] - system[0])) / duration if duration and len(user) > 1 and len(system) > 1 else None
    def delta(key):
        values = nums(rows, key)
        return values[-1] - values[0] if len(values) > 1 else None
    return {
        "sample_n": len(rows),
        "start_elapsed_s": elapsed[0] if elapsed else None,
        "end_elapsed_s": elapsed[-1] if elapsed else None,
        "duration_s": duration,
        "median_rss_mib": mib(statistics.median(rss)) if rss else None,
        "max_current_rss_mib": mib(max(rss)) if rss else None,
        "peak_field_max_mib": mib(max(peak)) if peak else None,
        "median_private_commit_mib": mib(statistics.median(private)) if private else None,
        "max_private_commit_mib": mib(max(private)) if private else None,
        "median_gpu_tracked_mib": mib(statistics.median(gpu)) if gpu else None,
        "max_gpu_tracked_mib": mib(max(gpu)) if gpu else None,
        "cpu_user_delta_s": user[-1] - user[0] if len(user) > 1 else None,
        "cpu_system_delta_s": system[-1] - system[0] if len(system) > 1 else None,
        "cpu_cores": cpu,
        "ready_range": rng(rows, "ready"),
        "active_range": rng(rows, "active"),
        "client_tick_delta": delta("client_tick_count"),
        "ui_draw_delta": delta("ui_draw_count"),
        "ui_frame_delta": delta("ui_frame_count"),
    }


def reviewed_module():
    path = Path(__file__).with_name("windows-tile-boxed-background-screen-analysis.py")
    spec = importlib.util.spec_from_file_location("boxed_reviewed_metrics", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def compact_gate(gate):
    if not isinstance(gate, dict):
        return gate
    out = {k: v for k, v in gate.items() if k not in ("slots", "process_wide_groups")}
    list_fields = {"sample_ages_ms", "buckets_delta", "fine_buckets_delta", "stable_completion_interval_buckets_delta", "interval_excess_buckets_delta", "sleep_excess_buckets_delta"}
    if "slots" in gate:
        out["slots"] = []
        for slot in gate["slots"]:
            item = {}
            for key, value in slot.items():
                if key in list_fields and isinstance(value, list):
                    item[key + "_count"] = sum(value)
                    item[key + "_bucket_count"] = len(value)
                else:
                    item[key] = value
            out["slots"].append(item)
    return out


def gate_results(rm, meta, rows):
    return {
        name: compact_gate(getattr(rm, name)(meta, rows))
        for name in ("evaluate_scheduling", "evaluate_scheduling_slots", "evaluate_decode", "evaluate_input", "evaluate_gpu")
    }


def qualification(cell):
    rows = jsonl(cell / "raw-run-01/samples.qualification.jsonl")
    points = {r.get("phase"): r for r in rows}
    start, end = points["observe-start"], points["observe-end"]
    return {
        "start_elapsed_s": start["elapsed_s"],
        "end_elapsed_s": end["elapsed_s"],
        "start_slot_n": len(start.get("slots", [])),
        "end_slot_n": len(end.get("slots", [])),
        "all_running_no_error": all(s.get("state") == "Running" and not s.get("error") for s in start.get("slots", []) + end.get("slots", [])),
        "rows": rows,
    }


def manifest_check(cell, expected_tar):
    manifest = load(cell / "archive-manifest.json")
    checks = []
    for item in manifest.get("files", []):
        rel = item["path"].replace("\\", "/")
        path = cell / rel
        actual = sha(path) if path.is_file() else None
        length = path.stat().st_size if path.is_file() else None
        checks.append({"path": rel, "listed_sha256": item.get("sha256"), "actual_sha256": actual, "listed_length": item.get("length"), "actual_length": length, "ok": path.is_file() and actual == item.get("sha256") and length == item.get("length")})
    tar = cell.with_suffix(".tar.gz")
    return {"manifest_file_n": len(checks), "manifest_all_ok": all(x["ok"] for x in checks), "tar_sha256": sha(tar), "tar_expected": expected_tar, "tar_ok": sha(tar) == expected_tar, "checks": checks}


def per_slot_summary(gates):
    result = {}
    for name, gate in gates.items():
        if not isinstance(gate, dict):
            result[name] = gate
            continue
        slots = gate.get("slots") or []
        result[name] = {"status": gate.get("status"), "reason": gate.get("reason"), "slot_n": len(slots), "available_n": sum(s.get("status") == "available" for s in slots if isinstance(s, dict)), "slots": slots, "process_wide_groups": gate.get("process_wide_groups")}
    return result


def main():
    helper = reviewed_module()
    rm = helper.reviewed_metrics()
    cells = {}
    for label, dirname, role in CELLS:
        cell = ARCHIVE_ROOT / dirname
        meta = load(cell / "raw-run-01/metadata.json")
        bound = load(cell / "managed-run/native-bound.json")
        q = qualification(cell)
        raw = jsonl(cell / "raw-run-01/samples.jsonl")
        observed = [r for r in raw if r.get("phase") == "observe" and q["start_elapsed_s"] <= r.get("elapsed_s", -1) <= q["end_elapsed_s"]]
        qual_control = helper.qualify_control(cell)
        gates = gate_results(rm, meta, observed)
        cells[label] = {
            "archive": dirname,
            "role": role,
            "raw_row_n": len(raw),
            "whole": aggregate(observed),
            "qualification": {k: v for k, v in q.items() if k != "rows"},
            "qualify_control_no_write": qual_control,
            "native_binding": {k: bound.get(k) for k in ("status", "binding_ok", "qualified", "exit_code", "pair_eligible", "match_keys_missing", "final_acceptance_claim", "reason")},
            "match_keys": bound.get("match_keys"),
            "side_provenance": bound.get("side_provenance"),
            "observation_wall_span": bound.get("observation_wall_span"),
            "managed_resources": {k: (bound.get("managed_resources") or {}).get(k) for k in ("status", "reason", "continuous_coverage")},
            "archive_verification": manifest_check(cell, EXPECTED_TARS[label]),
            "reviewed_gates_whole": per_slot_summary(gates),
            "raw_rows": raw,
        }
    common_start = max(cells[x]["whole"]["start_elapsed_s"] for x in cells)
    common_end = min(cells[x]["whole"]["end_elapsed_s"] for x in cells)
    for label, cell in cells.items():
        selected = [r for r in cell["raw_rows"] if r.get("phase") == "observe" and common_start <= r.get("elapsed_s", -1) <= common_end]
        meta = load(ARCHIVE_ROOT / cell["archive"] / "raw-run-01/metadata.json")
        cell["common"] = {"window": aggregate(selected), "reviewed_gates": per_slot_summary(gate_results(rm, meta, selected))}
        windows = {}
        for index in range(5):
            start = common_start + index * 100.0
            end = start + 100.0
            rows = [r for r in cell["raw_rows"] if r.get("phase") == "observe" and start <= r.get("elapsed_s", -1) <= end]
            windows["bin%d" % (index + 1)] = {"requested_elapsed_s": [start, end], "metrics": aggregate(rows)}
        final_start = common_end - 300.0
        rows = [r for r in cell["raw_rows"] if r.get("phase") == "observe" and final_start <= r.get("elapsed_s", -1) <= common_end]
        windows["final300"] = {"requested_elapsed_s": [final_start, common_end], "metrics": aggregate(rows)}
        cell["predeclared_windows"] = windows
        del cell["raw_rows"]
    def delta(section, key):
        a, b = cells["baseline"][section][key], cells["candidate"][section][key]
        return b - a if isinstance(a, (int, float)) and isinstance(b, (int, float)) else None
    common_delta = {key: cells["candidate"]["common"]["window"][key] - cells["baseline"]["common"]["window"][key] for key in ("median_rss_mib", "max_current_rss_mib", "peak_field_max_mib", "median_private_commit_mib", "max_private_commit_mib", "median_gpu_tracked_mib", "max_gpu_tracked_mib", "cpu_cores")}
    whole_delta = {key: delta("whole", key) for key in ("median_rss_mib", "max_current_rss_mib", "peak_field_max_mib", "median_private_commit_mib", "max_private_commit_mib", "median_gpu_tracked_mib", "max_gpu_tracked_mib", "cpu_cores")}
    final_delta = {key: cells["candidate"]["predeclared_windows"]["final300"]["metrics"][key] - cells["baseline"]["predeclared_windows"]["final300"]["metrics"][key] for key in ("median_rss_mib", "max_current_rss_mib", "peak_field_max_mib", "median_private_commit_mib", "max_private_commit_mib", "median_gpu_tracked_mib", "max_gpu_tracked_mib", "cpu_cores")}
    out = {
        "schema": "windows-tile-boxed-long-focused-recompute-v1",
        "reproducible_command": "python3 docs/memory/windows-tile-boxed-long-screen-analysis.py",
        "scope": "one predeclared focused-one N16 longer pair; descriptive only",
        "archives_read_exactly": [x[1] for x in CELLS],
        "common_window": {"elapsed_s": [common_start, common_end], "duration_s": common_end - common_start, "common_delta_candidate_minus_baseline": common_delta},
        "whole_delta_candidate_minus_baseline": whole_delta,
        "final300_delta_candidate_minus_baseline": final_delta,
        "cells": cells,
        "classification": {
            "candidate_recommendation": "park",
            "intended_rss_saving": "not_proven",
            "cpu_non_regression_5_percent": abs(common_delta["cpu_cores"] / cells["baseline"]["common"]["window"]["cpu_cores"] * 100) < 5,
            "fine_p99": "unavailable_or_incomplete",
            "input": "unavailable",
            "reason": "The common median RSS is slightly higher for the candidate, while the final300 median is about 8.02% higher. Both traces show substantial startup decay and this single pair has no focused RSS win; per-slot scheduling/decode/GPU readers remain descriptive and input is unavailable.",
            "accepted_rss_saving": False,
            "performance_acceptance": False,
            "final_acceptance": False,
        },
    }
    OUT.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(OUT), "common_window": out["common_window"], "whole_delta": whole_delta, "final300_delta": final_delta, "archive_ok": {k: v["archive_verification"]["manifest_all_ok"] and v["archive_verification"]["tar_ok"] for k, v in cells.items()}, "qualification": {k: v["qualify_control_no_write"] for k, v in cells.items()}}, indent=2, default=str))


if __name__ == "__main__":
    main()