#!/usr/bin/env python3
"""Read-only parser for BoneBurier 21446414 receipts. Audit helper, not a test."""
import json
import os
import re
from collections import Counter, defaultdict

BASE = "docs/compat/evidence/catalog-harness/live"
OUT = "docs/compat/evidence/native-sequencing-observation-cost"


def load_json(path):
    with open(path) as fh:
        return json.load(fh)


def summarize_json(path):
    data = load_json(path)
    summary = {"path": path, "top_keys": list(data.keys()) if isinstance(data, dict) else type(data).__name__}
    if not isinstance(data, dict):
        return summary
    for k in (
        "ok",
        "pass",
        "status",
        "result",
        "outcome",
        "error",
        "scenario",
        "revision",
        "catalog",
        "name",
        "cell",
        "elapsed_ms",
        "duration_ms",
        "exit_code",
    ):
        if k in data:
            summary[k] = data[k]
    for k, v in data.items():
        if isinstance(v, dict):
            summary[f"{k}_keys"] = list(v.keys())[:30]
        elif isinstance(v, list):
            summary[f"{k}_len"] = len(v)
            if v:
                first = v[0]
                if isinstance(first, dict):
                    summary[f"{k}_0_keys"] = list(first.keys())[:20]
                else:
                    summary[f"{k}_0"] = repr(first)[:200]
        elif isinstance(v, str) and len(v) > 240:
            summary[f"{k}_len"] = len(v)
        elif k not in summary:
            summary[k] = v
    return summary


def scan_log(path, max_lines=None):
    stats = {
        "path": path,
        "size": os.path.getsize(path),
        "lines": 0,
        "walk_nearest": 0,
        "open_booth": 0,
        "timeout": 0,
        "walk_timeout": 0,
        "slow_ticks": 0,
        "slow_tick_ms": [],
        "phases": Counter(),
        "samples": [],
        "here_samples": [],
        "nearest_booth_samples": [],
        "locs_len_samples": [],
        "bank_events": [],
        "tick_ms_by_phase": defaultdict(list),
        "first_last": [],
    }
    # patterns
    slow_re = re.compile(r"slow.?tick|tick_ms|tickMs|duration_ms|elapsed", re.I)
    walk_re = re.compile(r"walk[-_ ]?nearest[-_ ]?bank|WalkNearestBank", re.I)
    timeout_re = re.compile(r"timeout|walk-timeout|open timeout", re.I)
    interesting = re.compile(
        r"WalkNearestBank|walk-nearest-bank|open-booth|openBooth|bank_open|timeout|WalkNear|nearest_booth|bury|withdraw|slow",
        re.I,
    )
    with open(path, errors="replace") as fh:
        for i, line in enumerate(fh, 1):
            stats["lines"] = i
            if interesting.search(line):
                if len(stats["samples"]) < 80:
                    stats["samples"].append((i, line[:400].rstrip()))
                if walk_re.search(line):
                    stats["walk_nearest"] += 1
                if timeout_re.search(line):
                    stats["timeout"] += 1
                    if "walk-timeout" in line or "walk_timeout" in line:
                        stats["walk_timeout"] += 1
                if "open-booth" in line or "openBooth" in line or "OpenBooth" in line:
                    stats["open_booth"] += 1
            # json-ish tick records
            if line.startswith("{") or '"tick"' in line or "tick_ms" in line:
                try:
                    rec = json.loads(line)
                except Exception:
                    rec = None
                if isinstance(rec, dict):
                    phase = rec.get("phase") or rec.get("op") or rec.get("kind") or rec.get("state")
                    ms = rec.get("tick_ms") or rec.get("tickMs") or rec.get("duration_ms") or rec.get("ms")
                    if isinstance(ms, (int, float)) and ms >= 50:
                        stats["slow_ticks"] += 1
                        stats["slow_tick_ms"].append(ms)
                        if phase:
                            stats["tick_ms_by_phase"][str(phase)].append(ms)
                    if phase:
                        stats["phases"][str(phase)] += 1
            if i == 1 or (max_lines and i >= max_lines):
                stats["first_last"].append((i, line[:300].rstrip()))
    # last 3 lines
    with open(path, errors="replace") as fh:
        fh.seek(0, os.SEEK_END)
        size = fh.tell()
        fh.seek(max(0, size - 8000))
        tail = fh.read().splitlines()[-8:]
        stats["tail"] = tail
    return stats


def main():
    os.makedirs(OUT, exist_ok=True)
    files = sorted(
        p
        for p in os.listdir(BASE)
        if "bone" in p.lower() and "21446414" in p
    )
    index = []
    for name in files:
        path = os.path.join(BASE, name)
        entry = {"name": name, "size": os.path.getsize(path)}
        if name.endswith(".json"):
            entry["summary"] = summarize_json(path)
        elif name.endswith(".log"):
            entry["scan"] = {
                k: (dict(v) if isinstance(v, Counter) else v)
                for k, v in scan_log(path).items()
                if k not in ("tick_ms_by_phase",)
            }
            # keep phase timing compact
            scan = scan_log(path)
            entry["scan"]["tick_ms_by_phase"] = {
                k: {
                    "n": len(v),
                    "min": min(v) if v else None,
                    "max": max(v) if v else None,
                    "median": sorted(v)[len(v) // 2] if v else None,
                    "ge50": sum(1 for x in v if x >= 50),
                    "ge40": sum(1 for x in v if x >= 40),
                }
                for k, v in scan["tick_ms_by_phase"].items()
            }
            entry["scan"]["slow_tick_ms_n"] = len(scan["slow_tick_ms"])
            if scan["slow_tick_ms"]:
                ms = scan["slow_tick_ms"]
                entry["scan"]["slow_tick_ms_min"] = min(ms)
                entry["scan"]["slow_tick_ms_max"] = max(ms)
                entry["scan"]["slow_tick_ms_median"] = sorted(ms)[len(ms) // 2]
            # drop bulky samples if huge
        index.append(entry)
        print(name, entry.get("size"), list(entry.keys()))
    with open(os.path.join(OUT, "log-index.json"), "w") as fh:
        json.dump(index, fh, indent=2, default=str)
    print("wrote", os.path.join(OUT, "log-index.json"))


if __name__ == "__main__":
    main()
