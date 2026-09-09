#!/usr/bin/env python3
import json
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parent
records = [json.loads(line) for line in (ROOT / "samples.jsonl").read_text().splitlines() if line]
assert len(records) == 8
assert {r["schema"] for r in records} == {"nav-coordinate-lookup-v1"}
by_arm = {arm: [r for r in records if r["arm"] == arm] for arm in ("before", "after")}
assert {arm: len(rows) for arm, rows in by_arm.items()} == {"before": 4, "after": 4}

for key in ("read", "route"):
    checksums = {r[key]["checksum"] for r in records}
    warm_checksums = {r[key]["warm_checksum"] for r in records}
    assert len(checksums) == 1, (key, checksums)
    assert len(warm_checksums) == 1, (key, warm_checksums)
assert len({json.dumps(r["layout"], sort_keys=True) for r in records}) == 1
assert len({(r["read"]["width"], r["read"]["height"], r["read"]["planes"], r["read"]["probes"], r["read"]["passes"]) for r in records}) == 1
assert len({(r["route"]["width"], r["route"]["height"], r["route"]["routes"]) for r in records}) == 1

summary = {
    "schema": "nav-coordinate-lookup-analysis-v1",
    "record_count": len(records),
    "processes_per_arm": 4,
    "samples_per_process": 7,
    "checksums_equal": True,
    "layout_equal": True,
    "layout": records[0]["layout"],
    "input": {
        "read": {k: records[0]["read"][k] for k in ("width", "height", "planes", "probes", "passes")},
        "route": {k: records[0]["route"][k] for k in ("width", "height", "routes")},
    },
    "metrics": {},
}
for key in ("read", "route"):
    flat = {arm: [value for r in rows for value in r[key]["samples_ns"]] for arm, rows in by_arm.items()}
    process_medians = {arm: [statistics.median(r[key]["samples_ns"]) for r in rows] for arm, rows in by_arm.items()}
    before = statistics.median(flat["before"])
    after = statistics.median(flat["after"])
    summary["metrics"][key] = {
        "before_flat_median_ns": before,
        "after_flat_median_ns": after,
        "change_percent": (after / before - 1.0) * 100.0,
        "before_min_ns": min(flat["before"]),
        "before_max_ns": max(flat["before"]),
        "after_min_ns": min(flat["after"]),
        "after_max_ns": max(flat["after"]),
        "before_process_medians_ns": process_medians["before"],
        "after_process_medians_ns": process_medians["after"],
        "before_process_median_ns": statistics.median(process_medians["before"]),
        "after_process_median_ns": statistics.median(process_medians["after"]),
    }

(ROOT / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
print(json.dumps(summary, indent=2, sort_keys=True))
