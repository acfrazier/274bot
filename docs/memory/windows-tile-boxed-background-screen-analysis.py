#!/usr/bin/env python3
"""Independently recompute the native boxed-tile N16 ABBA screen.

Only the four immutable clean archives named below are inputs.  The script
keeps Windows paths as recorded, verifies local archive/manifests without
attempting to reopen those paths, and never treats resource provenance labels
as an acceptance gate.
"""
from __future__ import annotations

import hashlib
import json
import statistics
import tarfile
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ARCHIVE_ROOT = ROOT / "diagnostics/windows-tile-boxed-clean-20260908"
OUT = Path(__file__).with_name("windows-tile-boxed-background-screen-table.json")
CELLS = [
    ("baseline-forward", "baseline-focused-plus-background-nativecheck-20260908-0103", "reference"),
    ("candidate-forward", "candidate-focused-plus-background-nativecheck-20260908-0103", "candidate"),
    ("candidate-reverse", "candidate-focused-plus-background-reverse-20260908-0116", "candidate"),
    ("baseline-reverse", "baseline-focused-plus-background-reverse-20260908-0116", "reference"),
]

def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()

def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))

def jsonl(path: Path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8-sig").splitlines() if line.strip()]

def nums(rows, key):
    return [r[key] for r in rows if isinstance(r.get(key), (int, float))]

def mib(v):
    return None if v is None else v / 1048576.0

def range_for(rows, key):
    v = nums(rows, key)
    return [min(v), max(v)] if v else None

def aggregate(rows):
    elapsed = nums(rows, "elapsed_s")
    user = nums(rows, "process_cpu_user_s")
    system = nums(rows, "process_cpu_system_s")
    wall = elapsed[-1] - elapsed[0] if len(elapsed) > 1 else None
    cpu = None
    if wall and len(user) > 1 and len(system) > 1:
        cpu = ((user[-1] - user[0]) + (system[-1] - system[0])) / wall
    rss = nums(rows, "resident_bytes")
    peak = nums(rows, "peak_resident_bytes")
    private = nums(rows, "windows_private_commit_bytes")
    return {
        "elapsed_endpoint_s": [elapsed[0], elapsed[-1]] if elapsed else None,
        "duration_s": wall,
        "sample_count": len(rows),
        "rss_median_mib": mib(statistics.median(rss)) if rss else None,
        "rss_max_mib": mib(max(rss)) if rss else None,
        "rss_peak_field_max_mib": mib(max(peak)) if peak else None,
        "private_commit_median_mib": mib(statistics.median(private)) if private else None,
        "private_commit_max_mib": mib(max(private)) if private else None,
        "gpu_tracked_median_mib": mib(statistics.median(nums(rows, "gpu_tracked_bytes"))) if nums(rows, "gpu_tracked_bytes") else None,
        "gpu_tracked_max_mib": mib(max(nums(rows, "gpu_tracked_bytes"))) if nums(rows, "gpu_tracked_bytes") else None,
        "cpu_process_cores": cpu,
        "ready_range": range_for(rows, "ready"),
        "active_range": range_for(rows, "active"),
        "client_tick_delta": (nums(rows, "client_tick_count")[-1] - nums(rows, "client_tick_count")[0]) if len(nums(rows, "client_tick_count")) > 1 else None,
        "ui_draw_delta": (nums(rows, "ui_draw_count")[-1] - nums(rows, "ui_draw_count")[0]) if len(nums(rows, "ui_draw_count")) > 1 else None,
        "ui_frame_delta": (nums(rows, "ui_frame_count")[-1] - nums(rows, "ui_frame_count")[0]) if len(nums(rows, "ui_frame_count")) > 1 else None,
    }

def manifest_check(cell: Path):
    manifest = load(cell / "archive-manifest.json")
    checks = []
    for item in manifest.get("files", []):
        rel = item["path"].replace("\\", "/")
        p = cell / rel
        actual = sha(p) if p.is_file() else None
        checks.append({"path": rel, "listed_sha256": item.get("sha256"), "actual_sha256": actual,
                       "listed_length": item.get("length"), "actual_length": p.stat().st_size if p.is_file() else None,
                       "ok": p.is_file() and actual == item.get("sha256") and p.stat().st_size == item.get("length")})
    tar = cell.with_suffix(".tar.gz")
    return {"file_count": len(checks), "all_files_ok": all(x["ok"] for x in checks),
            "checks": checks, "tar_sha256": sha(tar) if tar.is_file() else None,
            "tar_present": tar.is_file()}

def qualification(cell: Path):
    rows = jsonl(cell / "raw-run-01/samples.qualification.jsonl")
    by = {r.get("phase"): r for r in rows}
    start, end = by.get("observe-start", {}), by.get("observe-end", {})
    return {
        "phases": [r.get("phase") for r in rows],
        "elapsed_s": {"start": start.get("elapsed_s"), "end": end.get("elapsed_s")},
        "slot_count": {"start": len(start.get("slots", [])), "end": len(end.get("slots", []))},
        "all_16_running_at_boundaries": all(s.get("state") == "Running" and not s.get("error") for s in start.get("slots", []) + end.get("slots", [])),
        "startup_age_to_observe_start_s": start.get("elapsed_s"),
        "native_rows_preserved": True,
    }

def find_keys(obj, key, out):
    if isinstance(obj, dict):
        if key in obj:
            out.append(obj[key])
        for v in obj.values():
            find_keys(v, key, out)
    elif isinstance(obj, list):
        for v in obj:
            find_keys(v, key, out)

def gate_summary(cell: Path):
    p = cell / "managed-run/cells"
    reports = list(p.glob("*/cell_report.json"))
    if not reports:
        return {"status": "unavailable", "reason": "cell_report_missing"}
    obj = load(reports[0])
    result = {}
    for key in ("scheduling", "decode", "input", "gpu"):
        found = []
        find_keys(obj.get("analysis", obj), key, found)
        result[key] = {"occurrences": len(found), "statuses": sorted({str(x.get("status")) for x in found if isinstance(x, dict)})}
    return result

def bound_summary(cell: Path):
    bound = load(cell / "managed-run/native-bound.json")
    side = bound.get("side_provenance", {})
    qual = bound.get("qualification", {})
    q = qual.get("qualification", {}) if isinstance(qual, dict) else {}
    return {
        "status": bound.get("status"),
        "binding_ok": bound.get("binding_ok"),
        "qualified": bound.get("qualified"),
        "exit_code": bound.get("exit_code"),
        "pair_eligible": bound.get("pair_eligible"),
        "final_acceptance_claim": bound.get("final_acceptance_claim"),
        "raw_hash_status": bound.get("raw_hash_status"),
        "binary_sha256": bound.get("binary_sha256"),
        "manifest_binary_key": bound.get("manifest_binary_key"),
        "manifest_build_commit": side.get("manifest_build_commit"),
        "manifest_sources_sha256": side.get("manifest_sources_sha256"),
        "client_commit": side.get("client_commit"),
        "client_sources_sha256": side.get("client_sources_sha256"),
        "qualification": {k: q.get(k) for k in ("qualified", "observation_s", "cpu_cores", "client_ticks_per_slot_s", "steal_gains")},
        "native_observation_wall_span": bound.get("observation_wall_span"),
        "native_wall_span_source": bound.get("observation_wall_span_source"),
        "endpoint_notes": bound.get("endpoint_notes"),
        "managed_resource_status": (bound.get("managed_resources") or {}).get("status"),
        "managed_resource_reason": (bound.get("managed_resources") or {}).get("reason"),
        "managed_resources": bound.get("managed_resources"),
        "overhead": bound.get("overhead"),
    }

def reviewed_metrics():
    path = Path(__file__).with_name("reference_metrics.py")
    spec = importlib.util.spec_from_file_location("reference_metrics", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module

def qualify_control(cell: Path):
    path = Path(__file__).with_name("qualify_control.py")
    spec = importlib.util.spec_from_file_location("qualify_control", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module.qualify(cell / "raw-run-01", counting=False, diagnostics=False)

def compact_gate(gate):
    """Keep reviewed per-slot results without copying raw histograms."""
    if not isinstance(gate, dict):
        return gate
    out = {k: v for k, v in gate.items() if k not in ("slots", "process_wide_groups")}
    slots = []
    for slot in gate.get("slots", []):
        if not isinstance(slot, dict):
            slots.append(slot)
            continue
        item = {}
        for key, value in slot.items():
            if key in ("sample_ages_ms", "buckets_delta", "fine_buckets_delta",
                       "stable_completion_interval_buckets_delta",
                       "interval_excess_buckets_delta", "sleep_excess_buckets_delta"):
                if isinstance(value, list):
                    item[key + "_count"] = sum(value)
                    item[key + "_bucket_count"] = len(value)
                continue
            item[key] = value
        slots.append(item)
    if "slots" in gate:
        out["slots"] = slots
    if "process_wide_groups" in gate:
        out["process_wide_groups"] = gate["process_wide_groups"]
    return out

def startup_render_inspection(cell: Path, meta: dict, bound: dict):
    """Extract actual startup/render records, never infer image proof."""
    records = [meta, bound]
    for rel in ("managed-run/completion.json", "managed-run/started.json",
                "managed-run/host-conditions.json", "managed-run/server-identity.json"):
        p = cell / rel
        if p.is_file():
            records.append(load(p))
    wanted = {"geometry", "scale", "visible", "focused", "focus", "minimized",
              "offscreen", "window", "requested_adapter", "actual_adapter",
              "adapter", "backend", "renderer", "renderer_settings",
              "render_policy", "render_policy_requested", "count", "n",
              "drawing", "draw", "full_rate", "renderer_present"}
    found = []
    def walk(obj, path="$"):
        if isinstance(obj, dict):
            for key, value in obj.items():
                if key in wanted:
                    if isinstance(value, (str, int, float, bool)) or value is None:
                        found.append({"path": path + "." + key, "value": value})
                    elif key in {"geometry", "window", "renderer_settings"}:
                        found.append({"path": path + "." + key, "value": value})
                walk(value, path + "." + key)
        elif isinstance(obj, list):
            for i, value in enumerate(obj):
                walk(value, path + "[" + str(i) + "]")
    for record in records:
        walk(record)
    return found

def main():
    rm = reviewed_metrics()
    cells = {}
    for label, name, role in CELLS:
        cell = ARCHIVE_ROOT / name
        raw = jsonl(cell / "raw-run-01/samples.jsonl")
        q = qualification(cell)
        lo, hi = q["elapsed_s"]["start"], q["elapsed_s"]["end"]
        obs = [r for r in raw if lo <= r.get("elapsed_s", -1) <= hi]
        meta = load(cell / "raw-run-01/metadata.json")
        bound = load(cell / "managed-run/native-bound.json")
        cells[label] = {
            "archive": name, "role": role, "raw_row_count": len(raw),
            "qualification": {"original": q, "qualify_control": qualify_control(cell)},
            "native_bound": bound_summary(cell),
            "metadata": meta, "whole_observation": aggregate(obs),
            "archive_verification": manifest_check(cell),
            "raw_input_files": {x: sha(cell / "raw-run-01" / x) for x in ("metadata.json", "samples.jsonl", "samples.qualification.jsonl")},
            "startup_render_inspection": startup_render_inspection(cell, meta, bound),
            "raw_rows": raw,
        }
    ranges = {}
    def intersection(labels):
        start = max(cells[x]["whole_observation"]["elapsed_endpoint_s"][0] for x in labels)
        end = min(cells[x]["whole_observation"]["elapsed_endpoint_s"][1] for x in labels)
        return [start, end]
    ranges["forward_AB"] = intersection(["baseline-forward", "candidate-forward"])
    ranges["reverse_BA"] = intersection(["baseline-reverse", "candidate-reverse"])
    ranges["common_all_four"] = intersection([x[0] for x in CELLS])
    gate_functions = ("evaluate_scheduling", "evaluate_scheduling_slots", "evaluate_decode", "evaluate_input", "evaluate_gpu")
    for name, pair_range in ranges.items():
        for label, cell in cells.items():
            selected = [r for r in cell["raw_rows"] if pair_range[0] <= r.get("elapsed_s", -1) <= pair_range[1]]
            cell.setdefault("ranges", {})[name] = {
                "selected_observation": aggregate(selected),
                "selected_elapsed_s": [selected[0].get("elapsed_s"), selected[-1].get("elapsed_s")] if selected else None,
                "sample_count": len(selected),
                "reviewed_gates": {fn: compact_gate(getattr(rm, fn)(cell["metadata"], selected)) for fn in gate_functions},
            }
    for cell in cells.values():
        del cell["raw_rows"]
    def pair(a, b, key):
        ra, rb = cells[a]["ranges"][key], cells[b]["ranges"][key]
        x, y = ra["selected_observation"], rb["selected_observation"]
        return {"reference": a, "candidate": b, "common_elapsed_s": ranges[key],
                "duration_s": ranges[key][1] - ranges[key][0],
                "selected_endpoints": {"reference": ra["selected_elapsed_s"], "candidate": rb["selected_elapsed_s"]},
                "sample_counts": {"reference": x["sample_count"], "candidate": y["sample_count"]},
                "delta_candidate_minus_reference": {k: (y[k] - x[k]) if isinstance(x.get(k), (int, float)) and isinstance(y.get(k), (int, float)) else None
                    for k in ("rss_median_mib", "rss_max_mib", "rss_peak_field_max_mib", "private_commit_median_mib", "gpu_tracked_median_mib", "cpu_process_cores", "client_tick_delta", "ui_draw_delta", "ui_frame_delta")}}
    out = {
        "schema": "windows-tile-boxed-background-screen-recompute-v2",
        "reproducible_command": "python3 docs/memory/windows-tile-boxed-background-screen-analysis.py",
        "scope": "screening evidence only; no retention, budget, lifecycle, visual, or final acceptance claim",
        "archives_read_exactly": [x[1] for x in CELLS], "cells": cells,
        "forward_AB": pair("baseline-forward", "candidate-forward", "forward_AB"),
        "reverse_BA": pair("baseline-reverse", "candidate-reverse", "reverse_BA"),
        "common_all_four": {"elapsed_s": ranges["common_all_four"], "duration_s": ranges["common_all_four"][1] - ranges["common_all_four"][0],
                            "per_cell": {x: cells[x]["ranges"]["common_all_four"] for x, _, _ in CELLS}},
        "classification": {"intended_rss_reduction": "inconclusive", "cpu_5_percent_non_regression": "inconclusive", "p99_2ms_non_regression": "unavailable",
            "reason": "two repeats expose order/readiness variation but are not a noise distribution; process/cache resource evidence is available, while overhead calibration and complete paired latency proof are unavailable",
            "next_action": "focused-one regression pair; consider at most one longer confirmation only if focused pair supports retention"},
        "limitations": ["Windows original paths are intentionally not reopened on macOS; native-bound status remains artifact proof.",
            "Source/runtime checkout labels are not binary identity; exact build provenance and startup records are retained.",
            "No rendered-image proof is inferred from logs. GPU evaluation is callback-completion diagnostic, not presentation/scanout.",
            "Missing input samples and incomplete decode/transition coverage remain explicit unavailable gate results.",
            "Private commit, GPU tracking, and peak RSS are reported separately and never subtracted from RSS."],
    }
    OUT.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(OUT), "ranges": ranges,
        "archive_ok": {k: v["archive_verification"]["all_files_ok"] for k, v in cells.items()},
        "gate_status": {cell: {window: {gate: value["reviewed_gates"][gate].get("status") + ":" + str(value["reviewed_gates"][gate].get("reason"))
            for gate in gate_functions} for window, value in data["ranges"].items()} for cell, data in cells.items()}}, indent=2))

if __name__ == "__main__":
    main()
