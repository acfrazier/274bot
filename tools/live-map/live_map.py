#!/usr/bin/env python3
"""Print live cells that cover changed paths in a Git diff."""
from __future__ import annotations

import argparse
import ast
import fnmatch
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from dataclasses import dataclass
from functools import lru_cache
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
MAP_PATH = Path(__file__).with_name("map.toml")
_CELL_NAME = re.compile(r"[A-Za-z0-9_.:-]+\Z")


class MapError(ValueError):
    """The mapping file does not match the supported format."""


@dataclass(frozen=True)
class Cell:
    name: str
    command: str


@dataclass(frozen=True)
class Plan:
    cells: tuple[Cell, ...]
    uncovered: tuple[str, ...]
    unclassified: tuple[str, ...]
    not_live: tuple[str, ...]


def _array_end(text: str) -> bool:
    """Return true when a possibly multiline array has closed."""
    quote = False
    escaped = False
    depth = 0
    for char in text:
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quote = False
        elif char == '"':
            quote = True
        elif char == "[":
            depth += 1
        elif char == "]":
            depth -= 1
            if depth == 0:
                return True
    return False


def _load_toml_310(path: Path) -> dict[str, Any]:
    """Read the small TOML subset used by map.toml on Python 3.10."""
    data: dict[str, Any] = {"cover": []}
    section: dict[str, Any] | None = None
    current_cover: dict[str, Any] | None = None
    lines = path.read_text(encoding="utf-8").splitlines()
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        index += 1
        if not line or line.startswith("#"):
            continue
        if line == "[[cover]]":
            current_cover = {}
            data["cover"].append(current_cover)
            section = current_cover
            continue
        if line.startswith("[") and line.endswith("]"):
            name = line[1:-1]
            if name not in ("live", "not_live"):
                raise MapError(f"unsupported TOML section: {line}")
            section = {}
            data[name] = section
            current_cover = None
            continue
        if section is None or "=" not in line:
            raise MapError(f"invalid TOML line {index}: {line}")
        key, value = (part.strip() for part in line.split("=", 1))
        if value.startswith("[") and not _array_end(value):
            parts = [value]
            while index < len(lines) and not _array_end("\n".join(parts)):
                parts.append(lines[index])
                index += 1
            if not _array_end("\n".join(parts)):
                raise MapError(f"unterminated array for {key}")
            value = "\n".join(parts)
        try:
            parsed = ast.literal_eval(value)
        except (SyntaxError, ValueError) as error:
            raise MapError(f"invalid value for {key}: {error}") from error
        section[key] = parsed
    return data


def load_map(path: Path = MAP_PATH) -> dict[str, Any]:
    try:
        import tomllib
    except ModuleNotFoundError:
        raw = _load_toml_310(path)
    else:
        with path.open("rb") as stream:
            raw = tomllib.load(stream)
    return validate_map(raw)


def validate_map(raw: dict[str, Any]) -> dict[str, Any]:
    if set(raw) != {"live", "not_live", "cover"}:
        raise MapError("map.toml must contain only live, not_live, and cover")
    for section_name in ("live", "not_live"):
        section = raw[section_name]
        if not isinstance(section, dict) or set(section) != {"globs"}:
            raise MapError(f"[{section_name}] must define only globs")
        _validate_globs(section["globs"], section_name)
    if not isinstance(raw["cover"], list):
        raise MapError("cover must be an array of tables")
    names: dict[str, str] = {}
    for index, row in enumerate(raw["cover"]):
        if not isinstance(row, dict) or set(row) != {"globs", "cells"}:
            raise MapError(f"cover row {index + 1} must define globs and cells")
        _validate_globs(row["globs"], f"cover row {index + 1}")
        if not isinstance(row["cells"], list) or not row["cells"]:
            raise MapError(f"cover row {index + 1} must define at least one cell")
        for cell_spec in row["cells"]:
            if not isinstance(cell_spec, str) or "|" not in cell_spec:
                raise MapError(f"cover row {index + 1} has a cell without name|command")
            name, command = (part.strip() for part in cell_spec.split("|", 1))
            if not _CELL_NAME.fullmatch(name) or not command:
                raise MapError(f"cover row {index + 1} has an invalid cell: {cell_spec!r}")
            if name in names and names[name] != command:
                raise MapError(f"cell {name!r} has conflicting commands")
            names[name] = command
    return raw


def _validate_globs(globs: Any, label: str) -> None:
    if not isinstance(globs, list) or not globs:
        raise MapError(f"{label} must define at least one glob")
    if any(not isinstance(pattern, str) or not pattern or pattern.startswith("/") for pattern in globs):
        raise MapError(f"{label} has an invalid relative glob")


@lru_cache(maxsize=4096)
def _match_parts(path_parts: tuple[str, ...], pattern_parts: tuple[str, ...], pi: int = 0, gi: int = 0) -> bool:
    if gi == len(pattern_parts):
        return pi == len(path_parts)
    if pattern_parts[gi] == "**":
        return _match_parts(path_parts, pattern_parts, pi, gi + 1) or (
            pi < len(path_parts) and _match_parts(path_parts, pattern_parts, pi + 1, gi)
        )
    return pi < len(path_parts) and fnmatch.fnmatchcase(path_parts[pi], pattern_parts[gi]) and _match_parts(
        path_parts, pattern_parts, pi + 1, gi + 1
    )


def matches(path: str, pattern: str) -> bool:
    normalized = Path(path).as_posix()
    if normalized.startswith("/") or normalized == ".." or normalized.startswith("../"):
        return False
    return _match_parts(tuple(normalized.split("/")), tuple(pattern.split("/")))


def matches_any(path: str, globs: list[str]) -> bool:
    return any(matches(path, pattern) for pattern in globs)


def make_plan(paths: list[str], config: dict[str, Any]) -> Plan:
    unique_paths = sorted(set(paths))
    live_globs = config["live"]["globs"]
    excluded_globs = config["not_live"]["globs"]
    live_paths: list[str] = []
    not_live: list[str] = []
    unclassified: list[str] = []
    for path in unique_paths:
        excluded = matches_any(path, excluded_globs)
        is_live = matches_any(path, live_globs)
        if excluded:
            not_live.append(path)
        elif is_live:
            live_paths.append(path)
        else:
            unclassified.append(path)

    cells: dict[str, Cell] = {}
    uncovered: list[str] = []
    for path in live_paths:
        rows = [row for row in config["cover"] if matches_any(path, row["globs"])]
        if not rows:
            uncovered.append(path)
            continue
        for row in rows:
            for cell_spec in row["cells"]:
                name, command = (part.strip() for part in cell_spec.split("|", 1))
                cells.setdefault(name, Cell(name, command))
    ordered_cells = tuple(cells[name] for name in sorted(cells))
    return Plan(ordered_cells, tuple(uncovered), tuple(unclassified), tuple(not_live))


def diff_paths(diff_range: str, repo: Path = ROOT) -> list[str]:
    if diff_range.count("..") != 1 or diff_range.startswith("..") or diff_range.endswith("..") or "..." in diff_range:
        raise MapError("--diff needs one BASE..HEAD range")
    result = subprocess.run(
        ["git", "diff", "--name-only", diff_range],
        cwd=repo,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        detail = result.stderr.strip() or f"git diff exited {result.returncode}"
        raise MapError(detail)
    return result.stdout.splitlines()


def format_plan(plan: Plan) -> str:
    lines = [f"Required cells ({len(plan.cells)}; run each 3 times):"]
    if plan.cells:
        for cell in plan.cells:
            lines.extend((f"- {cell.name}", f"  Command: `{cell.command}`"))
    else:
        lines.append("- None")
    lines.append(f"Changed live paths without a covering cell ({len(plan.uncovered)}):")
    lines.extend(f"- {path}" for path in plan.uncovered)
    if not plan.uncovered:
        lines.append("- None")
    lines.append(f"Unclassified paths ({len(plan.unclassified)}):")
    lines.extend(f"- {path}" for path in plan.unclassified)
    if not plan.unclassified:
        lines.append("- None")
    lines.append(f"Not-live paths ignored: {len(plan.not_live)}")
    return "\n".join(lines)


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True)


class LiveMapTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = validate_map(
            {
                "live": {"globs": ["crates/nav/src/**"]},
                "not_live": {"globs": ["**/*_tests.rs", "docs/**", "tools/**"]},
                "cover": [
                    {"globs": ["crates/nav/src/router.rs"], "cells": ["nav.routes|run-nav"]},
                ],
            }
        )

    def test_glob_matches_components_and_double_star(self) -> None:
        self.assertTrue(matches("crates/nav/src/router.rs", "crates/nav/src/**"))
        self.assertTrue(matches("crates/nav/src/router/heap.rs", "crates/nav/src/router/**"))
        self.assertFalse(matches("crates/nav/tests/router.rs", "crates/nav/src/**"))
        self.assertFalse(matches("../outside.rs", "**"))

    def test_not_live_overrides_live_and_cells_deduplicate(self) -> None:
        plan = make_plan(
            [
                "crates/nav/src/router.rs",
                "crates/nav/src/router_tests.rs",
                "tools/check.py",
            ],
            self.config,
        )
        self.assertEqual([cell.name for cell in plan.cells], ["nav.routes"])
        self.assertEqual(plan.uncovered, ())
        self.assertEqual(plan.unclassified, ())
        self.assertEqual(plan.not_live, ("crates/nav/src/router_tests.rs", "tools/check.py"))

    def test_uncovered_and_unclassified_paths_are_findings(self) -> None:
        plan = make_plan(["crates/nav/src/new_area.rs", "mystery/file.rs"], self.config)
        self.assertEqual(plan.uncovered, ("crates/nav/src/new_area.rs",))
        self.assertEqual(plan.unclassified, ("mystery/file.rs",))

    def test_conflicting_cell_commands_are_rejected(self) -> None:
        raw = {
            "live": {"globs": ["src/**"]},
            "not_live": {"globs": ["docs/**"]},
            "cover": [
                {"globs": ["src/**"], "cells": ["one|first"]},
                {"globs": ["src/**"], "cells": ["one|second"]},
            ],
        }
        with self.assertRaises(MapError):
            validate_map(raw)

    def test_diff_reads_a_temporary_git_repository(self) -> None:
        with tempfile.TemporaryDirectory(prefix="live-map-self-test-") as temp:
            repo = Path(temp)
            _git(repo, "init", "--quiet")
            _git(repo, "config", "user.name", "Live Map Test")
            _git(repo, "config", "user.email", "live-map-test@example.invalid")
            (repo / "README.md").write_text("base\n", encoding="utf-8")
            _git(repo, "add", "README.md")
            _git(repo, "commit", "--quiet", "-m", "base")
            _git(repo, "branch", "baseline")
            for relative in (
                "crates/nav/src/router.rs",
                "crates/nav/src/router_tests.rs",
                "tools/check.py",
                "mystery/file.rs",
            ):
                target = repo / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("changed\n", encoding="utf-8")
            _git(repo, "add", "crates/nav/src/router.rs", "crates/nav/src/router_tests.rs", "tools/check.py", "mystery/file.rs")
            _git(repo, "commit", "--quiet", "-m", "change")
            paths = diff_paths("baseline..HEAD", repo)
            plan = make_plan(paths, self.config)
            self.assertEqual([cell.name for cell in plan.cells], ["nav.routes"])
            self.assertEqual(plan.uncovered, ())
            self.assertEqual(plan.unclassified, ("mystery/file.rs",))

    def test_python_310_subset_parser_reads_the_map_shape(self) -> None:
        with tempfile.TemporaryDirectory(prefix="live-map-toml-") as temp:
            path = Path(temp) / "map.toml"
            path.write_text(
                '[live]\nglobs = ["src/**"]\n'
                '[not_live]\nglobs = ["docs/**"]\n'
                '[[cover]]\nglobs = ["src/**"]\n'
                'cells = [\n "one|run-one",\n]\n',
                encoding="utf-8",
            )
            parsed = validate_map(_load_toml_310(path))
            self.assertEqual(parsed["cover"][0]["cells"], ["one|run-one"])


def self_test() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(LiveMapTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--diff", metavar="BASE..HEAD")
    group.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    try:
        config = load_map()
        paths = diff_paths(args.diff)
        plan = make_plan(paths, config)
    except (MapError, OSError) as error:
        print(f"live-map: {error}", file=sys.stderr)
        return 2
    print(format_plan(plan))
    return 1 if plan.uncovered or plan.unclassified else 0


if __name__ == "__main__":
    raise SystemExit(main())
