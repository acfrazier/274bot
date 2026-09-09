#!/usr/bin/env python3
"""Validate direct-owner-v1 owner-capture JSONL rows (feature memory-owner-capture).

Usage:
  python3 scripts/validate_owner_capture_jsonl.py path/to/owner.jsonl

Exit 0 on pass; non-zero with reason lines on failure.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

SCHEMA = "direct-owner-v1"
REQUIRED_TOP = {
    "schema",
    "slot_token",
    "request_id",
    "frame_serial",
    "phase",
    "source",
    "complete",
    "reason",
    "rows",
}
REQUIRED_ROW = {
    "owner",
    "field",
    "complete",
    "reason",
}
# Capacity fields may be null when incomplete/unknown — but when present must be int.
OPTIONAL_INT_ROW = {
    "element_len",
    "element_capacity",
    "occupied_count",
    "occupied_element_bytes",
    "capacity_bytes",
    "nested_capacity_bytes",
    "box_count",
    "box_bytes",
    "holder_count",
    "elapsed_observer_ns",
}


def fail(msg: str) -> int:
    print(f"FAIL: {msg}", file=sys.stderr)
    return 1


def validate_row(row: dict, line_no: int) -> list[str]:
    errs = []
    missing = REQUIRED_ROW - row.keys()
    if missing:
        errs.append(f"L{line_no} row missing {sorted(missing)}")
    if row.get("complete") is True:
        # complete rows must not use zero as a stand-in for unknown capacity
        for k in ("capacity_bytes", "element_capacity"):
            if k in row and row[k] is None:
                errs.append(f"L{line_no} complete row has null {k}")
    for k in OPTIONAL_INT_ROW:
        v = row.get(k)
        if v is not None and not isinstance(v, int):
            errs.append(f"L{line_no} {k} must be int|null, got {type(v).__name__}")
    return errs


def validate_record(obj: dict, line_no: int) -> list[str]:
    errs = []
    if obj.get("schema") != SCHEMA:
        errs.append(f"L{line_no} schema want {SCHEMA!r} got {obj.get('schema')!r}")
    missing = REQUIRED_TOP - obj.keys()
    if missing:
        errs.append(f"L{line_no} missing top keys {sorted(missing)}")
    rows = obj.get("rows")
    if not isinstance(rows, list):
        errs.append(f"L{line_no} rows must be list")
        return errs
    for i, row in enumerate(rows):
        if not isinstance(row, dict):
            errs.append(f"L{line_no} rows[{i}] not object")
            continue
        errs.extend(validate_row(row, line_no))
    return errs


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    path = Path(argv[1])
    if not path.is_file():
        return fail(f"not a file: {path}")
    n = 0
    errs: list[str] = []
    with path.open() as f:
        for line_no, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            n += 1
            try:
                obj = json.loads(line)
            except json.JSONDecodeError as e:
                errs.append(f"L{line_no} JSON: {e}")
                continue
            if not isinstance(obj, dict):
                errs.append(f"L{line_no} top-level must be object")
                continue
            errs.extend(validate_record(obj, line_no))
    if n == 0:
        return fail("empty JSONL")
    if errs:
        for e in errs[:50]:
            print(e, file=sys.stderr)
        if len(errs) > 50:
            print(f"... and {len(errs) - 50} more", file=sys.stderr)
        return 1
    print(f"OK: {n} records schema={SCHEMA}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
