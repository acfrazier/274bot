#!/usr/bin/env python3
"""Scan ArdyCakes 21446414 old-catalog FAIL logs."""
import json
import os
import re

BASE = "docs/compat/evidence/catalog-harness/live"
OUT = "docs/compat/evidence/native-sequencing-observation-cost"
SLOW = re.compile(r"slow tick (\d+): ([0-9.]+)ms")
INTERESTING = re.compile(
    r"slow tick|steal|cake|WalkTo|walk-to|loc |OpenBooth|terminated|Unknown error|FAIL|PASS|interact |error|abort|combat|stocked|no-progress",
    re.I,
)


def scan(path):
    slow = []
    notes = []
    interacts = []
    errors = []
    for i, line in enumerate(open(path, errors="replace"), 1):
        s = line.rstrip()
        m = SLOW.search(s)
        if m:
            slow.append({"line": i, "tick": int(m.group(1)), "ms": float(m.group(2))})
        if INTERESTING.search(s):
            if len(notes) < 250:
                notes.append({"line": i, "text": s[:320]})
        if "interact " in s:
            interacts.append({"line": i, "text": s[:280]})
        if re.search(r"terminated|Unknown error|FAIL|panic|error", s, re.I):
            errors.append({"line": i, "text": s[:400]})
    with open(path, errors="replace") as fh:
        fh.seek(0, os.SEEK_END)
        size = fh.tell()
        fh.seek(max(0, size - 6000))
        tail = fh.read().splitlines()[-15:]
    return {
        "path": path,
        "size": os.path.getsize(path),
        "slow_n": len(slow),
        "slow_ms": [x["ms"] for x in slow],
        "slow_ticks": [x["tick"] for x in slow],
        "slow_min": min((x["ms"] for x in slow), default=None),
        "slow_max": max((x["ms"] for x in slow), default=None),
        "slow_median": (sorted(x["ms"] for x in slow)[len(slow) // 2] if slow else None),
        "slow_ge50": sum(1 for x in slow if x["ms"] >= 50),
        "slow_ge200": sum(1 for x in slow if x["ms"] >= 200),
        "slow_ge230": sum(1 for x in slow if x["ms"] >= 230),
        "first_slow": slow[0] if slow else None,
        "last_slow": slow[-1] if slow else None,
        "interacts": interacts[:80],
        "errors": errors,
        "notes": notes,
        "tail": tail,
    }


def summarize_json(path):
    data = json.load(open(path))
    keep = {}
    for k in (
        "host_commit",
        "client_commit",
        "binary_sha256",
        "catalog_commit",
        "revision",
        "case",
        "elapsed_seconds",
        "elapsed_is_not_performance_measurement",
        "exit_code",
        "log_sha256",
        "started_at",
        "finished_at",
        "pid",
    ):
        if k in data:
            keep[k] = data[k]
    return keep


def main():
    out = {"receipts": {}, "logs": {}}
    for name in sorted(os.listdir(BASE)):
        if "ardy-cakes" not in name or "21446414" not in name:
            continue
        path = os.path.join(BASE, name)
        if name.endswith(".json") and not name.endswith(".process.json"):
            out["receipts"][name] = summarize_json(path)
        elif name.endswith(".log"):
            rec = scan(path)
            out["logs"][name] = rec
            print(
                name,
                "slow",
                rec["slow_n"],
                "ge200",
                rec["slow_ge200"],
                "median",
                rec["slow_median"],
                "min",
                rec["slow_min"],
                "max",
                rec["slow_max"],
                "errors",
                len(rec["errors"]),
            )
    with open(os.path.join(OUT, "ardy-cakes-21446414-scan.json"), "w") as fh:
        json.dump(out, fh, indent=2)
    print("wrote ardy-cakes-21446414-scan.json")


if __name__ == "__main__":
    main()
