#!/usr/bin/env python3
"""Independently recompute the native boxed-tile N16 focused-one pair."""
from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ARCHIVE_ROOT = ROOT / "diagnostics/windows-tile-boxed-clean-20260908"
OUT = Path(__file__).with_name("windows-tile-boxed-focused-screen-table.json")
CELLS = [
    ("baseline", "baseline-focused-one-nativecheck-20260908-0103", "reference"),
    ("candidate", "candidate-focused-one-nativecheck-20260908-0103", "candidate"),
]


def module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


base = module("boxed_background_analysis", Path(__file__).with_name("windows-tile-boxed-background-screen-analysis.py"))
metrics = base.reviewed_metrics()


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def jsonl(path: Path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8-sig").splitlines() if line.strip()]


def compact_gate(gate):
    """Retain per-slot counters/p99 while avoiding raw histogram duplication."""
    if not isinstance(gate, dict):
        return gate
    out = {k: v for k, v in gate.items() if k not in ("slots", "process_wide_groups")}
    list_fields = {
        "sample_ages_ms", "buckets_delta", "fine_buckets_delta",
        "stable_completion_interval_buckets_delta", "interval_excess_buckets_delta",
        "sleep_excess_buckets_delta",
    }
    if "slots" in gate:
        slots = []
        for slot in gate["slots"]:
            if not isinstance(slot, dict):
                slots.append(slot)
                continue
            item = {}
            for key, value in slot.items():
                if key in list_fields:
                    if isinstance(value, list):
                        item[key + "_count"] = sum(value)
                        item[key + "_bucket_count"] = len(value)
                    continue
                item[key] = value
            slots.append(item)
        out["slots"] = slots
    if "process_wide_groups" in gate:
        out["process_wide_groups"] = gate["process_wide_groups"]
    return out


def exact_settings(cell: Path, meta: dict, bound: dict):
    spec = load(cell / "managed-run/spec.json")
    conditions = load(cell / "managed-run/host-conditions.json")
    server = load(cell / "managed-run/server-identity.json")
    cache = None
    cache_paths = list((cell / "managed-run/cells").glob("*/cache-provenance.json"))
    if cache_paths:
        cache = load(cache_paths[0])
    return {
        "metadata": {key: meta.get(key) for key in (
            "frontend", "n", "workload", "warmup_s", "observe_s", "nav_pack",
            "nav_pack_sha256", "nav_flags", "diagnostic_only", "scheduling_profile",
            "render_profile", "gpu_completion_profile", "responsiveness_profile",
            "responsiveness_fine", "diagnostic_sidecar", "failure_capture",
            "nav_captures", "single_renderer", "render_policy", "render_policy_requested",
            "terminal", "terminal_size", "binary", "binary_sha256", "host_commit",
            "client_commit", "host_sources_sha256", "client_sources_sha256",
        )},
        "spec": spec,
        "host_conditions": conditions,
        "server_identity": server,
        "cache_provenance": cache,
        "native_bound_side_provenance": bound.get("side_provenance"),
        "native_bound_match_keys": bound.get("match_keys"),
    }


def aggregate(rows):
    return base.aggregate(rows)


def selected_gate(meta, rows):
    return {
        name: compact_gate(getattr(metrics, name)(meta, rows))
        for name in (
            "evaluate_scheduling", "evaluate_scheduling_slots", "evaluate_decode",
            "evaluate_input", "evaluate_gpu",
        )
    }


def main():
    cells = {}
    for label, archive, role in CELLS:
        cell = ARCHIVE_ROOT / archive
        raw = jsonl(cell / "raw-run-01/samples.jsonl")
        qualification = base.qualification(cell)
        start = qualification["elapsed_s"]["start"]
        end = qualification["elapsed_s"]["end"]
        observed = [row for row in raw if start <= row.get("elapsed_s", -1) <= end]
        meta = load(cell / "raw-run-01/metadata.json")
        bound = load(cell / "managed-run/native-bound.json")
        cells[label] = {
            "archive": archive,
            "role": role,
            "raw_row_count": len(raw),
            "qualification": {
                "original": qualification,
                "qualify_control": base.qualify_control(cell),
            },
            "native_bound": base.bound_summary(cell),
            "exact_settings_and_provenance": exact_settings(cell, meta, bound),
            "whole_observation": aggregate(observed),
            "archive_verification": base.manifest_check(cell),
            "raw_input_files": {
                name: base.sha(cell / "raw-run-01" / name)
                for name in ("metadata.json", "samples.jsonl", "samples.qualification.jsonl")
            },
            "startup_render_inspection": base.startup_render_inspection(cell, meta, bound),
            "reviewed_gates": selected_gate(meta, observed),
            "raw_rows": raw,
        }

    common = [
        max(cells[label]["whole_observation"]["elapsed_endpoint_s"][0] for label, _, _ in CELLS),
        min(cells[label]["whole_observation"]["elapsed_endpoint_s"][1] for label, _, _ in CELLS),
    ]
    for label, cell in cells.items():
        selected = [row for row in cell["raw_rows"] if common[0] <= row.get("elapsed_s", -1) <= common[1]]
        meta = load(ARCHIVE_ROOT / cell["archive"] / "raw-run-01/metadata.json")
        cell["common_window"] = {
            "selected_observation": aggregate(selected),
            "selected_elapsed_s": [selected[0].get("elapsed_s"), selected[-1].get("elapsed_s")] if selected else None,
            "sample_count": len(selected),
            "reviewed_gates": selected_gate(meta, selected),
        }
        del cell["raw_rows"]

    reference = cells["baseline"]["common_window"]["selected_observation"]
    candidate = cells["candidate"]["common_window"]["selected_observation"]
    delta = {
        key: (candidate[key] - reference[key])
        if isinstance(candidate.get(key), (int, float)) and isinstance(reference.get(key), (int, float))
        else None
        for key in (
            "rss_median_mib", "rss_max_mib", "rss_peak_field_max_mib",
            "private_commit_median_mib", "private_commit_max_mib",
            "gpu_tracked_median_mib", "gpu_tracked_max_mib", "cpu_process_cores",
            "client_tick_delta", "ui_draw_delta", "ui_frame_delta",
        )
    }
    out = {
        "schema": "windows-tile-boxed-focused-screen-recompute-v1",
        "reproducible_command": "python3 docs/memory/windows-tile-boxed-focused-screen-analysis.py",
        "scope": "focused-one N16 screening evidence only; no retention, budget, lifecycle, visual, or final acceptance claim",
        "archives_read_exactly": [archive for _, archive, _ in CELLS],
        "cells": cells,
        "common_window": {
            "elapsed_s": common,
            "duration_s": common[1] - common[0],
            "per_cell": {label: cells[label]["common_window"] for label, _, _ in CELLS},
            "candidate_minus_baseline": delta,
        },
        "classification": {
            "intended_rss_reduction": "inconclusive",
            "cpu_5_percent_non_regression": "inconclusive",
            "p99_2ms_non_regression": "unavailable",
            "reason": "Focused common RSS is lower for the candidate, but both sides descend from different startup ages and this single pair cannot separate retained boxed-field effect from readiness/resident decay. CPU delta is within 5% descriptively, while complete paired fine p99 and input proof are unavailable.",
            "absolute_budget": "not evaluated as passed",
            "final_acceptance": False,
        },
        "confirmation_boundary": {
            "recommendation": "one_predeclared_longer_confirmation_stage",
            "justification": "The focused pair is directionally lower in common RSS and CPU is within the 5% screening margin, so one bounded stage is justified; the evidence remains insufficient for retention.",
            "stage": {
                "duration": "one 600 s observation per side",
                "warmup": "same 30 s warmup",
                "modes": "focused-one only, baseline then candidate",
                "order": "fresh preflight before each side; no reverse rerun or additional short pairs",
                "stable_age_question": "whether candidate and baseline converge to distinct stationary RSS/private-commit levels after startup decay under identical focused-one load",
                "stop_rule": "stop and reject confirmation if either cell fails exit0, native binding, 16/16 qualification, exact fixture/geometry/Intel Vulkan conditions, or if the selected overlap is absent; stop with no retention if candidate RSS is not lower in common median, CPU exceeds baseline by >5%, or required fine-gate coverage remains unavailable",
            },
            "still_pending": ["CPU functional proof", "live visual proof", "lifecycle", "input", "final target/resource provenance gates"],
        },
        "limitations": [
            "Windows original paths are preserved as literal evidence and are not reopened on macOS; copied native-bound records are proof of the archived native run, not local path binding.",
            "Managed process/cache accounting is available; instrumentation-overhead calibration and the standalone missing-resource-provenance gate remain unavailable.",
            "Input has no actual samples. Decode and GPU fine readers retain contained per-slot counters, dropped/lost/pending fields, role/ordinal mappings and p99 bucket bounds, but coverage is not complete for all slots.",
            "GPU completion is callback timing, not physical presentation or rendered-image proof; one actual GPU renderer with 16 active slots is not a frame-cadence acceptance claim.",
            "Private commit, peak RSS and GPU tracking are reported separately and never subtracted from RSS.",
        ],
    }
    OUT.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({
        "output": str(OUT),
        "archive_ok": {label: cells[label]["archive_verification"]["all_files_ok"] for label, _, _ in CELLS},
        "common_window": common,
        "delta": delta,
        "qualification": {label: cells[label]["qualification"]["qualify_control"] for label, _, _ in CELLS},
    }, indent=2))


if __name__ == "__main__":
    main()
