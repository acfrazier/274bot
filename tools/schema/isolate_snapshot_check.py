#!/usr/bin/env python3
"""Snapshot field-slot inventory: isolate.fbs declaration order vs VT_SNAP_*.

Coverage: Snapshot table field names and wire slot indices only — not FlatBuffer
types, defaults, nested tables, or full-schema / flatc conformance. No flatc
required.

    python3 tools/schema/isolate_snapshot_check.py
    python3 tools/schema/isolate_snapshot_check.py --self-test
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

# Auditable schema snake_case names for VT_SNAP_* suffixes that do not match
# isolate.fbs field names (naive snake_case would miss these).
VT_SNAP_FIELD_ALIASES: dict[str, str] = {
    "RETALIATE": "retaliate_enabled",
    "MAIN_MODAL": "main_modal_id",
    "CHAT_MODAL": "chat_modal_id",
}

VT_SNAP_RE = re.compile(
    r"^const\s+VT_SNAP_([A-Z0-9_]+):\s+VOffsetT\s*=\s*(\d+)\s*;",
    re.MULTILINE,
)

# Field line inside table Snapshot { ... } — name: type[; optional = default]
FBS_FIELD_RE = re.compile(
    r"^\s*([a-z][a-z0-9_]*)\s*:\s*([^;/]+?)(?:\s*=\s*[^;]+)?\s*;\s*(?://.*)?$"
)


@dataclass(frozen=True)
class SnapField:
    index: int
    name: str


@dataclass(frozen=True)
class RustSlot:
    index: int
    vt_name: str
    schema_name: str
    voffset: int


def repo_root_from_script() -> Path:
    return Path(__file__).resolve().parents[2]


def vt_suffix_to_schema_name(suffix: str) -> str:
    if suffix in VT_SNAP_FIELD_ALIASES:
        return VT_SNAP_FIELD_ALIASES[suffix]
    return suffix.lower()


def expected_voffset_for_index(index: int) -> int:
    return (index + 2) * 2


def validate_wire_voffset(voffset: int, *, vt_name: str) -> None:
    if voffset < 4:
        raise ValueError(
            f"isolate_fb.rs: {vt_name} voffset={voffset} invalid (need even integer >= 4)"
        )
    if voffset % 2 != 0:
        raise ValueError(
            f"isolate_fb.rs: {vt_name} voffset={voffset} invalid (need even integer >= 4)"
        )


def parse_snapshot_fields(fbs_text: str) -> list[str]:
    start = fbs_text.find("table Snapshot {")
    if start < 0:
        raise ValueError("isolate.fbs: table Snapshot { not found")
    depth = 0
    i = start + len("table Snapshot")
    while i < len(fbs_text) and fbs_text[i] != "{":
        i += 1
    if i >= len(fbs_text):
        raise ValueError("isolate.fbs: Snapshot table missing opening brace")
    body_start = i + 1
    depth = 1
    i = body_start
    while i < len(fbs_text) and depth:
        ch = fbs_text[i]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                body = fbs_text[body_start:i]
                break
        i += 1
    else:
        raise ValueError("isolate.fbs: unclosed Snapshot table")

    names: list[str] = []
    for raw_line in body.splitlines():
        line = raw_line.split("//", 1)[0].strip()
        if not line or line.startswith("///"):
            continue
        if line.startswith("/*") or "*/" in line:
            raise ValueError(
                f"isolate.fbs Snapshot: unsupported field syntax (block comment): {raw_line!r}"
            )
        if "(" in line:
            raise ValueError(
                f"isolate.fbs Snapshot: unsupported field attributes (parenthetical / id): "
                f"{raw_line!r}"
            )
        m = FBS_FIELD_RE.match(line)
        if not m:
            if re.match(r"^\s*$", raw_line):
                continue
            raise ValueError(
                f"isolate.fbs Snapshot: unsupported or unparsed field line: {raw_line!r}"
            )
        names.append(m.group(1))
    if not names:
        raise ValueError("isolate.fbs Snapshot: no fields parsed")
    return names


def parse_vt_snap_slots(rs_text: str) -> list[RustSlot]:
    matches = list(VT_SNAP_RE.finditer(rs_text))
    if not matches:
        raise ValueError("isolate_fb.rs: no VT_SNAP_* constants found")

    by_voffset: dict[int, RustSlot] = {}
    seen_vt: set[str] = set()
    unsorted: list[RustSlot] = []

    for m in matches:
        suffix = m.group(1)
        vt_name = f"VT_SNAP_{suffix}"
        voffset = int(m.group(2))
        if suffix in seen_vt:
            raise ValueError(f"isolate_fb.rs: duplicate {vt_name}")
        seen_vt.add(suffix)
        validate_wire_voffset(voffset, vt_name=vt_name)
        schema_name = vt_suffix_to_schema_name(suffix)
        slot = RustSlot(
            index=-1,
            vt_name=vt_name,
            schema_name=schema_name,
            voffset=voffset,
        )
        unsorted.append(slot)
        if voffset in by_voffset:
            other = by_voffset[voffset]
            raise ValueError(
                f"isolate_fb.rs: duplicate voffset {voffset} on {vt_name} and {other.vt_name}"
            )
        by_voffset[voffset] = slot

    ordered = sorted(unsorted, key=lambda s: s.voffset)
    slots: list[RustSlot] = []
    for idx, slot in enumerate(ordered):
        expected_v = expected_voffset_for_index(idx)
        if slot.voffset != expected_v:
            raise ValueError(
                f"isolate_fb.rs: {slot.vt_name} voffset={slot.voffset} != "
                f"wire slot {idx} expected {expected_v}"
            )
        slots.append(
            RustSlot(
                index=idx,
                vt_name=slot.vt_name,
                schema_name=slot.schema_name,
                voffset=slot.voffset,
            )
        )
    return slots


def compare_snapshot_slots(
    schema_names: list[str], rust_slots: list[RustSlot]
) -> list[str]:
    errors: list[str] = []
    schema_set = set(schema_names)
    rust_names = [s.schema_name for s in rust_slots]

    if len(schema_names) != len(set(schema_names)):
        dupes = sorted({n for n in schema_names if schema_names.count(n) > 1})
        errors.append(f"duplicate schema field name(s): {', '.join(dupes)}")

    rust_set = set(rust_names)
    if len(rust_names) != len(rust_set):
        dupes = sorted({n for n in rust_names if rust_names.count(n) > 1})
        errors.append(f"duplicate rust-mapped field name(s): {', '.join(dupes)}")

    missing_in_schema = sorted(rust_set - schema_set)
    extra_in_schema = sorted(schema_set - rust_set)
    if missing_in_schema:
        errors.append(
            "schema missing field(s) expected from VT_SNAP_*: "
            + ", ".join(missing_in_schema)
        )
    if extra_in_schema:
        errors.append(
            "schema extra field(s) not in VT_SNAP_* inventory: "
            + ", ".join(extra_in_schema)
        )

    limit = min(len(schema_names), len(rust_slots))
    for i in range(limit):
        s_name = schema_names[i]
        r_slot = rust_slots[i]
        if s_name != r_slot.schema_name:
            errors.append(
                f"index {i}: schema {s_name!r} != rust {r_slot.schema_name!r} "
                f"({r_slot.vt_name} voffset={r_slot.voffset})"
            )

    if len(schema_names) != len(rust_slots):
        errors.append(
            f"field count mismatch: schema={len(schema_names)} rust VT_SNAP={len(rust_slots)}"
        )

    return errors


def run_check(fbs_path: Path, rs_path: Path) -> list[str]:
    fbs_text = fbs_path.read_text(encoding="utf-8")
    rs_text = rs_path.read_text(encoding="utf-8")
    try:
        schema_names = parse_snapshot_fields(fbs_text)
        rust_slots = parse_vt_snap_slots(rs_text)
    except ValueError as exc:
        return [str(exc)]
    return compare_snapshot_slots(schema_names, rust_slots)


def _script_path() -> Path:
    return Path(__file__).resolve()


def _run_public_check(
    fbs_path: Path, rs_path: Path, *, expect_fail: bool
) -> tuple[int, str]:
    """Invoke this checker as a subprocess (public CLI entry), not run_check()."""
    proc = subprocess.run(
        [
            sys.executable,
            str(_script_path()),
            "--fbs",
            str(fbs_path),
            "--rust",
            str(rs_path),
        ],
        capture_output=True,
        text=True,
    )
    combined = (proc.stdout or "") + (proc.stderr or "")
    if expect_fail:
        if proc.returncode == 0:
            return proc.returncode, combined
        return proc.returncode, combined
    if proc.returncode != 0:
        return proc.returncode, combined
    return proc.returncode, combined


def run_self_test() -> int:
    root = repo_root_from_script()
    fbs = root / "crates/script/schema/isolate.fbs"
    rs = root / "crates/script/src/isolate_fb.rs"

    code, out = _run_public_check(fbs, rs, expect_fail=False)
    if code != 0:
        sys.stderr.write(
            f"self-test: product snapshot public check failed (expected pass):\n{out}\n"
        )
        return 1

    with tempfile.TemporaryDirectory(prefix="snap-slot-check-") as tmp:
        tmp_path = Path(tmp)
        bad_fbs = tmp_path / "isolate.fbs"
        bad_rs = tmp_path / "isolate_fb.rs"
        shutil.copy(fbs, bad_fbs)
        shutil.copy(rs, bad_rs)
        text = bad_fbs.read_text(encoding="utf-8")
        rs_text = bad_rs.read_text(encoding="utf-8")

        # Reintroduce confirmed drift: nearest_booth after booths instead of chat_lines.
        if "  booths: [Booth];\n  banks:" not in text:
            sys.stderr.write("self-test: unexpected isolate.fbs layout\n")
            return 1
        drifted = text.replace(
            "  booths: [Booth];\n  banks:",
            "  booths: [Booth];\n  nearest_booth: NearestBooth;\n  banks:",
        )
        drifted = drifted.replace(
            "  chat_lines: [ChatLine];\n  nearest_booth: NearestBooth;\n  bank_note_on:",
            "  chat_lines: [ChatLine];\n  bank_note_on:",
        )
        bad_fbs.write_text(drifted, encoding="utf-8")
        code, out = _run_public_check(bad_fbs, rs, expect_fail=True)
        if code == 0 or not ("nearest_booth" in out or "index 7" in out):
            sys.stderr.write(
                f"self-test: drift fixture should fail on nearest_booth shift; "
                f"code={code} out={out!r}\n"
            )
            return 1

        dup_fbs = tmp_path / "dup.fbs"
        dup_fbs.write_text(
            text.replace("  tick: ulong;\n", "  tick: ulong;\n  tick: ulong;\n"),
            encoding="utf-8",
        )
        code, out = _run_public_check(dup_fbs, rs, expect_fail=True)
        if code == 0 or "duplicate" not in out.lower():
            sys.stderr.write(
                f"self-test: duplicate field should fail; code={code} out={out!r}\n"
            )
            return 1

        extra_fbs = tmp_path / "extra.fbs"
        extra_fbs.write_text(
            text.replace(
                "  combat_level: int;\n}",
                "  combat_level: int;\n  phantom_field: int;\n}",
            ),
            encoding="utf-8",
        )
        code, out = _run_public_check(extra_fbs, rs, expect_fail=True)
        if code == 0 or "extra field" not in out:
            sys.stderr.write(
                f"self-test: extra field should fail; code={code} out={out!r}\n"
            )
            return 1

        alias_fbs = tmp_path / "alias.fbs"
        alias_fbs.write_text(
            text.replace("  main_modal_id: int;", "  main_modal: int;"),
            encoding="utf-8",
        )
        code, out = _run_public_check(alias_fbs, rs, expect_fail=True)
        if code == 0 or "main_modal" not in out:
            sys.stderr.write(
                f"self-test: MAIN_MODAL alias (main_modal_id) mismatch should fail; "
                f"code={code} out={out!r}\n"
            )
            return 1

        # Odd / wrong voffset: nearest_booth 80 -> 81 must reject (public CLI).
        bad_voff_rs = tmp_path / "voff81.rs"
        bad_voff_rs.write_text(
            rs_text.replace(
                "VT_SNAP_NEAREST_BOOTH: VOffsetT = 80",
                "VT_SNAP_NEAREST_BOOTH: VOffsetT = 81",
            ),
            encoding="utf-8",
        )
        code, out = _run_public_check(fbs, bad_voff_rs, expect_fail=True)
        if code == 0 or "81" not in out:
            sys.stderr.write(
                f"self-test: VT_SNAP_NEAREST_BOOTH voffset 81 should fail; "
                f"code={code} out={out!r}\n"
            )
            return 1

        # FlatBuffers field id / parenthetical attributes must not be swallowed.
        id_attr_fbs = tmp_path / "id_attr.fbs"
        id_attr_fbs.write_text(
            text.replace("  tick: ulong;", "  tick: ulong (id: 0);"),
            encoding="utf-8",
        )
        code, out = _run_public_check(id_attr_fbs, rs, expect_fail=True)
        if code == 0 or "parenthetical" not in out.lower():
            sys.stderr.write(
                f"self-test: Snapshot (id:) attribute should fail; "
                f"code={code} out={out!r}\n"
            )
            return 1

        # Harmless Rust declaration reorder: sort by voffset, not source order.
        tick_line = "const VT_SNAP_TICK: VOffsetT = 4;"
        here_line = "const VT_SNAP_HERE: VOffsetT = 6;"
        if tick_line not in rs_text or here_line not in rs_text:
            sys.stderr.write("self-test: unexpected isolate_fb.rs VT_SNAP layout\n")
            return 1
        reordered_rs = tmp_path / "reordered.rs"
        reordered_rs.write_text(
            rs_text.replace(
                f"{tick_line}\n{here_line}",
                f"{here_line}\n{tick_line}",
            ),
            encoding="utf-8",
        )
        code, out = _run_public_check(fbs, reordered_rs, expect_fail=False)
        if code != 0:
            sys.stderr.write(
                f"self-test: harmless VT_SNAP declaration reorder should pass; "
                f"code={code} out={out!r}\n"
            )
            return 1

    print(
        "isolate_snapshot_check self-test: pass on product tree; "
        "detects nearest_booth drift, duplicate, extra, alias mismatch, "
        "invalid voffset, fbs id attributes; allows rust VT reorder"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fbs",
        type=Path,
        help="path to isolate.fbs (default: crates/script/schema/isolate.fbs)",
    )
    parser.add_argument(
        "--rust",
        type=Path,
        help="path to isolate_fb.rs (default: crates/script/src/isolate_fb.rs)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run fixture checks; still validates product tree first",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    root = repo_root_from_script()
    fbs_path = (args.fbs or root / "crates/script/schema/isolate.fbs").resolve()
    rs_path = (args.rust or root / "crates/script/src/isolate_fb.rs").resolve()
    errors = run_check(fbs_path, rs_path)
    if errors:
        sys.stderr.write(
            "isolate_snapshot_check failed "
            "(Snapshot field inventory/order vs VT_SNAP_* only):\n"
        )
        for err in errors:
            sys.stderr.write(f"- {err}\n")
        return 1
    n = len(parse_snapshot_fields(fbs_path.read_text(encoding="utf-8")))
    print(
        f"isolate_snapshot_check: {n} Snapshot fields match VT_SNAP_* slot order "
        f"({fbs_path.name} ↔ {rs_path.name})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
