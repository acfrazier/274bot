#!/usr/bin/env python3
import hashlib
import json
import statistics
from pathlib import Path

BASE = Path(__file__).resolve().parents[2]
ROOT = BASE / "diagnostics/windows-lazy-upload-20260908"
CELLS = {
    "baseline-focused-one-20260908-0102": ("baseline", "focused-one"),
    "baseline-focused-plus-background-20260908-0111": ("baseline", "focused-plus-background"),
    "candidate-focused-one-20260908-0118": ("candidate", "focused-one"),
    "candidate-focused-plus-background-20260908-0126": ("candidate", "focused-plus-background"),
}
EXPECTED_TAR = {
    "baseline-focused-one-20260908-0102": "637b594951873aa01f05977bc4d654282cd0630313db868e573347f6688a5694",
    "baseline-focused-plus-background-20260908-0111": "306d937d513eaad0136ad59d737672115202609c414c3092c94c4058d8e26e46",
    "candidate-focused-one-20260908-0118": "6ac66be419307c9cba1f02d43a07734a6dd682ae1dd0d16b36b5ec9dcaf9ca8a",
    "candidate-focused-plus-background-20260908-0126": "e10791a380f8404030c194662bd0f50270dda4210152fe7dc83b1181e696bd79",
}


def lines(path):
    return [json.loads(x) for x in path.read_text(encoding="utf-8-sig").splitlines() if x.strip()]


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def mib(value):
    return value / 1048576 if value is not None else None


def vals(rows, key):
    return [r[key] for r in rows if isinstance(r.get(key), (int, float))]


def first(obj, *path):
    for key in path:
        if not isinstance(obj, dict):
            return None
        obj = obj.get(key)
    return obj


def nested(obj, key):
    if isinstance(obj, dict):
        if key in obj:
            yield obj[key]
        for value in obj.values():
            yield from nested(value, key)
    elif isinstance(obj, list):
        for value in obj:
            yield from nested(value, key)


def all_keys(obj, output):
    if isinstance(obj, dict):
        output.update(obj)
        for value in obj.values():
            all_keys(value, output)
    elif isinstance(obj, list):
        for value in obj:
            all_keys(value, output)


def safe_range(items):
    items = [x for x in items if isinstance(x, (int, float))]
    return [min(items), max(items)] if items else None


def rss_samples(rows):
    return [{"elapsed_s": r["elapsed_s"], "rss_mib": mib(r["resident_bytes"])}
            for r in rows if isinstance(r.get("elapsed_s"), (int, float))
            and isinstance(r.get("resident_bytes"), (int, float))]


def aligned_rss(baseline, candidate):
    """Compare samples at the common harness-elapsed interval."""
    bs, cs = baseline["rss_samples"], candidate["rss_samples"]
    start = max(bs[0]["elapsed_s"], cs[0]["elapsed_s"])
    end = min(bs[-1]["elapsed_s"], cs[-1]["elapsed_s"])
    b = [x["rss_mib"] for x in bs if start <= x["elapsed_s"] <= end]
    c = [x["rss_mib"] for x in cs if start <= x["elapsed_s"] <= end]
    return {
        "elapsed_s": [start, end],
        "sample_counts": {"baseline": len(b), "candidate": len(c)},
        "rss_median_mib": {"baseline": statistics.median(b), "candidate": statistics.median(c),
                            "delta": statistics.median(c) - statistics.median(b)},
        "limit": "common harness elapsed since each process start; samples are not simultaneous wall-clock observations",
    }


def relative_bins(cell, width=30.0):
    start = cell["rss_samples"][0]["elapsed_s"]
    groups = {}
    for sample in cell["rss_samples"]:
        index = int((sample["elapsed_s"] - start) // width)
        groups.setdefault(index, []).append(sample["rss_mib"])
    return [{"bin_start_s": index * width, "bin_end_s": (index + 1) * width,
             "sample_count": len(values), "rss_median_mib": statistics.median(values)}
            for index, values in sorted(groups.items())]


def condition_summary(path):
    obj = json.loads(path.read_text(encoding="utf-8-sig"))
    power = first(obj, "native_preflight", "powerState") or {}
    battery = (power.get("battery") or [{}])[0]
    brightness = (power.get("brightness") or [{}])[0]
    return {
        "ready_scene_active_fields": {
            "ready_observed": None,
            "scene_state_observed": None,
            "active_observed": None,
            "limit": "host-conditions has no per-slot ready/scene/active sample; raw rows/captures are used instead",
        },
        "cache": {
            "cache_dir": first(obj, "native_preflight", "cache", "cache_dir") or first(obj, "cache_dir"),
            "snapshot_version": first(obj, "native_preflight", "cache", "snapshot_version") or first(obj, "snapshot_version"),
            "content_hash": None,
            "content_hash_limit": "native qualification records null_at_boundary / not_hashed_at_boundary",
        },
        "temperature": {"value": None, "limit": "no temperature field in archived host-conditions"},
        "power": {
            "ac_line_status": power.get("acLineStatus"),
            "computer_system_power_state": power.get("computerSystemPowerState"),
            "battery_status": battery.get("BatteryStatus"),
            "brightness": brightness.get("CurrentBrightness"),
            "limit": "computerSystemPowerState is not AC-line status; battery/brightness are preflight observations",
        },
        "provenance": {
            "allocator": first(obj, "native_preflight", "allocator_provenance") or first(obj, "allocator_provenance"),
            "host_conditions_sha256": sha(path),
        },
    }


def capture_summary(path):
    result = {}
    for p in sorted(path.glob("raw-run-01/captures/**/*.json")):
        obj = json.loads(p.read_text(encoding="utf-8-sig"))
        event = obj.get("event") or {}
        diagnostics = event.get("diagnostics") or {}
        client = diagnostics.get("client") or {}
        result[event.get("checkpoint", p.name)] = {
            "file": str(p.relative_to(path)),
            "slot": event.get("slot"),
            "scene_state": client.get("scene_state"),
            "ingame": client.get("ingame"),
            "drawing": client.get("drawing"),
            "position": client.get("position"),
            "event_tick": event.get("event_tick"),
            "capture_delay_ms": obj.get("capture_delay_ms"),
            "snapshot_age_ms": diagnostics.get("snapshot_age_ms"),
            "bank_loaded": (event.get("snapshot") or {}).get("bank_loaded"),
            "inventory_count": len((event.get("snapshot") or {}).get("inventory") or []),
        }
    return result


def interval_summary(bind, section, key):
    slots = first(bind, "analysis", "gates", section, key) or []
    available = [s for s in slots if s.get("status") == "available"]
    p99 = [s.get("p99") for s in available if isinstance(s.get("p99"), dict)]
    return {
        "slot_count": len(slots),
        "available_count": len(available),
        "status_counts": {str(status): sum(1 for s in slots if s.get("status") == status) for status in sorted({s.get("status") for s in slots}, key=str)},
        "p99_bounds_ms": safe_range([p.get("lower_ms") for p in p99] + [p.get("upper_ms") for p in p99]),
        "p99_ranges_ms": [[p.get("lower_ms"), p.get("upper_ms")] for p in p99],
        "scene_transition_separation": sorted({s.get("scene_transition_separation") for s in slots}, key=str),
        "target_verdicts": sorted({s.get("target_verdict") for s in slots if s.get("target_verdict") is not None}, key=str),
        "limit": "offline observation-window histogram; not physical scanout/product cadence; JSONL row time is not interval endpoint",
    }


def main():
    table = {
        "schema": "windows-lazy-upload-comparison-v2",
        "protocol_sha256": sha(BASE / "docs/memory/windows-lazy-upload-comparison-protocol.md"),
        "cells": {},
        "pairs": {},
        "cross_cell": {},
    }
    for name, (role, mode) in CELLS.items():
        root = ROOT / name
        raw = lines(root / "raw-run-01/samples.jsonl")
        qual = lines(root / "raw-run-01/samples.qualification.jsonl")
        bind = json.loads((ROOT / f"{name}-binding.json").read_text(encoding="utf-8-sig"))
        qstart = next((r.get("elapsed_s") for r in qual if r.get("phase") == "observe-start"), None)
        qend = next((r.get("elapsed_s") for r in qual if r.get("phase") == "observe-end"), None)
        obs = [r for r in raw if qstart is not None and qend is not None and qstart <= r.get("elapsed_s", -1) <= qend] or raw
        rss = vals(obs, "resident_bytes")
        elapsed = vals(obs, "elapsed_s")
        user = vals(obs, "process_cpu_user_s")
        system = vals(obs, "process_cpu_system_s")
        gpu = vals(obs, "gpu_tracked_bytes")
        renders = [x for r in obs for x in (r.get("renderer_profile") or [])]
        render_keys = set()
        all_keys(renders, render_keys)
        statuses = {}
        counters = {}
        for renderer in renders:
            for key in ("renderer_present", "backend", "draw", "full_rate", "completed", "gpu_completed", "paint_count", "callback_count", "render_completed"):
                if key in renderer:
                    statuses.setdefault(key, {})[str(renderer[key])] = statuses.setdefault(key, {}).get(str(renderer[key]), 0) + 1
            for key, value in renderer.items():
                if isinstance(value, (int, float)) and any(s in key for s in ("completed", "paint", "callback", "completion")):
                    counters[key] = max(counters.get(key, 0), value)
        qualification = first(bind, "qualification", "qualification") or first(bind, "analysis", "workload_qualification", "qualification") or {}
        wall_span = first(bind, "native_qualification", "observation_wall_span")
        env = condition_summary(root / "managed-run/host-conditions.json")
        env["ready_scene_active_fields"].update({
            "ready_observed": safe_range(vals(obs, "ready")),
            "scene_state_observed": sorted({c.get("scene_state") for c in capture_summary(root).values()}),
            "active_observed": safe_range(vals(obs, "active")),
        })
        config = first(bind, "native_qualification", "match_keys", "qualification_settings") or {}
        render_config = first(bind, "native_qualification", "match_keys", "renderer_config_by_ordinal") or []
        runtime_config = first(bind, "native_qualification", "match_keys", "slot_runtime_settings_by_ordinal") or []
        env["cache"].update({
            "cache_dir": config.get("cache_dir_canonical"),
            "snapshot_version": first(bind, "match_keys", "cache_settings", "snapshot_version"),
        })
        env["provenance"].update({
            "allocator": first(bind, "match_keys", "allocator_provenance"),
            "role": first(bind, "side_provenance", "role"),
            "host_commit": first(bind, "side_provenance", "manifest_build_commit"),
            "client_commit": first(bind, "side_provenance", "client_commit"),
            "binary_sha256": first(bind, "side_provenance", "binary_sha256"),
            "sources_stable": first(bind, "side_provenance", "manifest_sources_stable"),
        })
        scheduling = interval_summary(bind, "scheduling", "slots")
        gpu_completion = interval_summary(bind, "gpu", "diagnostic_completion_latency",) if False else None
        gpu_slots = first(bind, "analysis", "gates", "gpu", "diagnostic_completion_latency", "slots") or []
        gpu_completion = {
            "slot_count": len(gpu_slots),
            "available_count": sum(1 for s in gpu_slots if s.get("status") == "available"),
            "p99_ranges_ms": [[s.get("p99", {}).get("lower_ms"), s.get("p99", {}).get("upper_ms")] for s in gpu_slots if s.get("status") == "available" and isinstance(s.get("p99"), dict)],
            "completion_coverage_complete_count": sum(1 for s in gpu_slots if s.get("completion_coverage_complete") is True),
            "target_verdicts": sorted({s.get("target_verdict") for s in gpu_slots if s.get("target_verdict") is not None}),
            "limit": "completed GPU diagnostic only; no physical scanout and not product frame cadence",
        }
        tar = root.with_suffix(".tar.gz")
        manifest = json.loads((root / "archive-manifest.json").read_text(encoding="utf-8-sig"))
        file_checks = []
        for item in manifest.get("files", []):
            relative = item["path"].replace("\\", "/")
            file_path = root / relative
            actual_sha = sha(file_path) if file_path.exists() else None
            file_checks.append({
                "path": relative,
                "listed_sha256": item["sha256"],
                "actual_sha256": actual_sha,
                "listed_length": item.get("length"),
                "actual_length": file_path.stat().st_size if file_path.exists() else None,
                "ok": file_path.exists() and actual_sha == item["sha256"] and file_path.stat().st_size == item.get("length"),
            })
        table["cells"][name] = {
            "role": role, "mode": mode,
            "archive_tar_sha256": sha(tar) if tar.exists() else None,
            "archive_tar_expected": EXPECTED_TAR[name],
            "archive_tar_ok": tar.exists() and sha(tar) == EXPECTED_TAR[name],
            "archive_manifest_file_count": len(file_checks),
            "archive_manifest_files_ok": all(item["ok"] for item in file_checks),
            "archive_manifest_file_checks": file_checks,
            "qualification_binding": {k: bind.get(k) for k in ("status", "binding_ok", "pair_eligible", "final_acceptance_claim", "performance_acceptance") if k in bind},
            "raw_rows": len(raw), "qualification_rows": len(qual), "observation_rows": len(obs),
            "observe_bounds_s": [qstart, qend],
            "qualification_observation_s": qualification.get("observation_s"),
            "elapsed_observation_s": elapsed[-1] - elapsed[0] if len(elapsed) > 1 else None,
            "observation_wall_span_unix_s": wall_span,
            "rss_median_mib": mib(statistics.median(rss)) if rss else None,
            "rss_first_mib": mib(rss[0]) if rss else None, "rss_last_mib": mib(rss[-1]) if rss else None,
            "rss_min_mib": mib(min(rss)) if rss else None, "rss_max_mib": mib(max(rss)) if rss else None,
            "rss_samples": rss_samples(obs),
            "peak_max_mib": mib(max(vals(raw, "peak_resident_bytes"))) if vals(raw, "peak_resident_bytes") else None,
            "cpu_core_estimate": ((user[-1] - user[0]) + (system[-1] - system[0])) / (elapsed[-1] - elapsed[0]) if len(user) > 1 and len(system) > 1 and len(elapsed) > 1 else None,
            "ready_min_max": safe_range(vals(obs, "ready")), "active_min_max": safe_range(vals(obs, "active")),
            "client_tick_delta": vals(obs, "client_tick_count")[-1] - vals(obs, "client_tick_count")[0] if len(vals(obs, "client_tick_count")) > 1 else None,
            "ui_draw_delta": vals(obs, "ui_draw_count")[-1] - vals(obs, "ui_draw_count")[0] if len(vals(obs, "ui_draw_count")) > 1 else None,
            "ui_frame_delta": vals(obs, "ui_frame_count")[-1] - vals(obs, "ui_frame_count")[0] if len(vals(obs, "ui_frame_count")) > 1 else None,
            "gpu_tracked_mib_median": mib(statistics.median(gpu)) if gpu else None, "gpu_tracked_mib_max": mib(max(gpu)) if gpu else None,
            "renderer_observation_rows": len(renders), "renderer_status_counts": statuses, "renderer_key_inventory": sorted(render_keys), "renderer_counter_max": counters,
            "raw_phase_counts": {phase: sum(1 for r in raw if r.get("phase") == phase) for phase in sorted({r.get("phase") for r in raw})},
            "renderer_configuration_counts": {"records": len(render_config), "present": sum(1 for x in render_config if x.get("renderer_present")), "full_rate": sum(1 for x in render_config if x.get("full_rate")), "backend_counts": {b: sum(1 for x in render_config if x.get("backend") == b) for b in sorted({x.get("backend") for x in render_config})}},
            "runtime_configuration_counts": {"records": len(runtime_config), "draw_true": sum(1 for x in runtime_config if x.get("draw")), "draw_false": sum(1 for x in runtime_config if x.get("draw") is False)},
            "scheduling_intervals": scheduling,
            "gpu_completion_latency": gpu_completion,
            "qualification_selected": {k: first(bind, "qualification", "qualification", k) for k in ("client_ticks_per_slot_s", "steal_gains", "observe_sample_n", "simulation_intervals", "simulation_ticks", "render_completed_gpu", "background_paint_cadence_hz", "focused_paint_cadence_hz") if first(bind, "qualification", "qualification", k) is not None},
            "match_configuration": {k: config.get(k) for k in ("n", "workload", "cache_dir_canonical", "cache_dir_canonical_available", "render_policy_requested", "lowmem_requested", "port")},
            "environment": env,
            "captures": capture_summary(root),
        }
    for mode in ("focused-one", "focused-plus-background"):
        baseline = table["cells"][next(n for n, (r, m) in CELLS.items() if r == "baseline" and m == mode)]
        candidate = table["cells"][next(n for n, (r, m) in CELLS.items() if r == "candidate" and m == mode)]
        def delta(key):
            return candidate[key] - baseline[key] if isinstance(candidate.get(key), (int, float)) and isinstance(baseline.get(key), (int, float)) else None
        table["pairs"][mode] = {k: delta(k) for k in ("rss_median_mib", "rss_first_mib", "rss_last_mib", "rss_min_mib", "rss_max_mib", "peak_max_mib", "cpu_core_estimate", "gpu_tracked_mib_max", "client_tick_delta", "ui_draw_delta", "ui_frame_delta")}
        spans = [baseline.get("observation_wall_span_unix_s"), candidate.get("observation_wall_span_unix_s")]
        overlap = max(0.0, min(s[1] for s in spans if s) - max(s[0] for s in spans if s)) if all(spans) else None
        relative = {k: {"baseline": baseline.get(k), "candidate": candidate.get(k), "delta": delta(k)} for k in ("rss_first_mib", "rss_last_mib", "rss_min_mib", "rss_max_mib")}
        table["pairs"][mode].update({
            "absolute_wall_overlap_s": overlap,
            "harness_elapsed_common_window": aligned_rss(baseline, candidate),
            "observation_relative": {
                "baseline": relative_bins(baseline), "candidate": relative_bins(candidate),
                "width_s": 30.0,
                "rss_median_delta_by_bin_mib": [
                    {"bin_start_s": b["bin_start_s"], "bin_end_s": b["bin_end_s"],
                     "baseline": b["rss_median_mib"], "candidate": c["rss_median_mib"],
                     "delta": c["rss_median_mib"] - b["rss_median_mib"]}
                    for b, c in zip(relative_bins(baseline), relative_bins(candidate))
                ],
            },
            "absolute_elapsed_limit": "sequential cells have no simultaneous wall-clock control; common harness elapsed and relative bins retain startup/phase confounding",
        })
    names = list(CELLS)
    for field in ("cache", "temperature", "power", "provenance"):
        values = {n: table["cells"][n]["environment"][field] for n in names}
        table["cross_cell"][field] = {"values": values, "all_equal": len({json.dumps(v, sort_keys=True) for v in values.values()}) == 1, "limit": "comparison is descriptive; equality is only claimed for recorded fields"}
    Path(__file__).with_name("table.json").write_text(json.dumps(table, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(table, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
