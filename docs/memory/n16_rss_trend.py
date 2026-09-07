#!/usr/bin/env python3
"""Read-only temporal summary of the two archived N16 observe populations."""
import hashlib
import json
from pathlib import Path
from statistics import median


def summarize(path):
    raw = path.read_bytes()
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    rows = [row for row in rows if row.get("phase") == "observe"]
    if len(rows) < 3:
        raise ValueError(f"too few observe rows: {path}")
    times = [row["elapsed_s"] for row in rows]
    if any(b <= a for a, b in zip(times, times[1:])):
        raise ValueError(f"non-increasing observe timestamps: {path}")
    fields = {}
    for key in ("resident_bytes", "v8_used_bytes", "v8_total_bytes", "gpu_tracked_bytes"):
        values = [row[key] for row in rows]
        if any(type(value) is not int or value < 0 for value in values):
            raise ValueError(f"invalid {key}: {path}")
        fields[key] = {
            "first": values[0], "last": values[-1],
            "minimum": min(values), "maximum": max(values),
            "median": median(values), "last_minus_first": values[-1] - values[0],
            "consecutive_row_thirds_medians": [
                median(values[i * len(values) // 3:(i + 1) * len(values) // 3])
                for i in range(3)
            ],
        }
    return {
        "source": str(path), "sha256": hashlib.sha256(raw).hexdigest(),
        "rows": len(rows), "first_elapsed_s": times[0], "last_elapsed_s": times[-1],
        "third_partition": "consecutive row indices [i*N//3:(i+1)*N//3], i=0,1,2",
        "fields": fields,
        "performance_acceptance": False,
        "stationary_plateau_proven": False,
    }


if __name__ == "__main__":
    root = Path(__file__).parent / "diagnostics/windows-panel-n16-20260907"
    result = {
        mode: summarize(root / mode / "raw-run-01/samples.jsonl")
        for mode in ("n16-focused-one-console-intel", "n16-focused-plus-background-console-intel")
    }
    print(json.dumps(result, indent=2, allow_nan=False))
