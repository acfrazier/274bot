#!/usr/bin/env python3
"""Print live cells that cover changed paths in a Git diff."""
from __future__ import annotations

import argparse
import ast
import fnmatch
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from unittest.mock import patch
from dataclasses import dataclass
from functools import lru_cache
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
MAP_PATH = Path(__file__).with_name("map.toml")
_CELL_NAME = re.compile(r"[A-Za-z0-9_.:-]+\Z")
CLIENT_SUBMODULE_PATH = "vendor/fr-client-rust"


class MapError(ValueError):
    """The mapping file does not match the supported format."""


@dataclass(frozen=True)
class Cell:
    name: str
    command: str
    selector: str


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

def _cell_fields(cell_spec: str, label: str) -> Cell:
    parts = [part.strip() for part in cell_spec.split("|", 2)]
    if len(parts) not in (2, 3):
        raise MapError(f"{label} has a cell without name|command[|selector]")
    name, command = parts[:2]
    if not _CELL_NAME.fullmatch(name) or not command:
        raise MapError(f"{label} has an invalid cell: {cell_spec!r}")
    selector = parts[2] if len(parts) == 3 else _test_selector_from_command(command)
    if not selector:
        raise MapError(f"cell {name!r} needs a cargo-test selector")
    try:
        selector_args = shlex.split(selector)
    except ValueError as error:
        raise MapError(f"cell {name!r} has an invalid selector: {error}") from error
    if not selector_args or selector_args[:2] == ["cargo", "test"]:
        raise MapError(f"cell {name!r} selector must contain cargo test arguments only")
    return Cell(name, command, selector)


def _test_selector_from_command(command: str) -> str:
    try:
        tokens = shlex.split(command)
    except ValueError as error:
        raise MapError(f"invalid cell command: {error}") from error
    for index in range(len(tokens) - 1):
        if tokens[index : index + 2] != ["cargo", "test"]:
            continue
        args = tokens[index + 2 :]
        if "--" in args:
            split = args.index("--")
            cargo_args = args[:split]
            test_args = [
                arg
                for arg in args[split + 1 :]
                if arg != "--nocapture" and not arg.startswith("--test-threads=")
            ]
            return shlex.join([*cargo_args, "--", *test_args])
        return shlex.join(args)
    return ""


def _cell_registry(config: dict[str, Any]) -> dict[str, Cell]:
    cells: dict[str, Cell] = {}
    names: dict[str, Cell] = {}
    for index, row in enumerate(config["cover"]):
        for cell_spec in row["cells"]:
            cell = _cell_fields(cell_spec, f"cover row {index + 1}")
            if cell.name in names and names[cell.name] != cell:
                raise MapError(f"cell {cell.name!r} has conflicting commands or selectors")
            names[cell.name] = cell
            previous = cells.get(cell.command)
            if previous is not None and previous.selector != cell.selector:
                raise MapError(
                    f"command {cell.command!r} has conflicting test selectors"
                )
            if previous is None or cell.name < previous.name:
                cells[cell.command] = cell
    return cells


def verify_cells(config: dict[str, Any]) -> int:
    isohome = Path(
        os.environ.get(
            "ISOHOME",
            "/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome",
        )
    )
    if not isohome.is_absolute() or not isohome.is_file():
        print(f"live-map: ISOHOME must name the absolute isohome executable: {isohome}", file=sys.stderr)
        return 2
    failures = 0
    cells = _cell_registry(config)
    print(f"Verifying test selectors for {len(cells)} unique commands")
    for cell in sorted(cells.values(), key=lambda item: item.name):
        selector_args = shlex.split(cell.selector)
        if "--" in selector_args:
            split = selector_args.index("--")
            command_args = [
                *selector_args[:split],
                "--",
                *selector_args[split + 1 :],
                "--list",
            ]
        else:
            command_args = [*selector_args, "--", "--list"]
        command = [str(isohome), "nice", "-n", "10", "cargo", "test", *command_args]
        print(f"\n[{cell.name}] {shlex.join(command)}")
        result = subprocess.run(command, cwd=ROOT, check=False, capture_output=True, text=True)
        output = f"{result.stdout}{result.stderr}"
        selected = [
            line.split(": ", 1)[0].strip()
            for line in output.splitlines()
            if ": test" in line and line.split(": ", 1)[1].strip().startswith("test")
        ]
        for name in selected:
            print(f"  {name}")
        print(f"  selected tests: {len(selected)}; exit: {result.returncode}")
        if result.returncode or not selected:
            failures += 1
            print(output, file=sys.stderr, end="" if output.endswith("\n") else "\n")
    print(f"\nSelector result: {len(cells) - failures}/{len(cells)} cells select at least one test")
    return 1 if failures else 0


def validate_map(raw: dict[str, Any]) -> dict[str, Any]:
    if set(raw) != {"live", "not_live", "cover"}:
        raise MapError("map.toml must contain only live, not_live, and cover")
    for section_name in ("live", "not_live"):
        section = raw[section_name]
        allowed = {"globs"} if section_name == "live" else {"globs", "exceptions"}
        if not isinstance(section, dict) or "globs" not in section or set(section) - allowed:
            raise MapError(f"[{section_name}] has invalid fields")
        _validate_globs(section["globs"], section_name)
        if "exceptions" in section:
            _validate_globs(section["exceptions"], f"{section_name}.exceptions")
    if not isinstance(raw["cover"], list):
        raise MapError("cover must be an array of tables")
    for index, row in enumerate(raw["cover"]):
        if not isinstance(row, dict) or set(row) != {"globs", "cells"}:
            raise MapError(f"cover row {index + 1} must define globs and cells")
        _validate_globs(row["globs"], f"cover row {index + 1}")
        if not isinstance(row["cells"], list) or not row["cells"]:
            raise MapError(f"cover row {index + 1} must define at least one cell")
        for cell_spec in row["cells"]:
            if not isinstance(cell_spec, str):
                raise MapError(f"cover row {index + 1} has a cell that is not a string")
            _cell_fields(cell_spec, f"cover row {index + 1}")
    _cell_registry(raw)
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
    live_exceptions = config["not_live"].get("exceptions", [])
    live_paths: list[str] = []
    not_live: list[str] = []
    unclassified: list[str] = []
    for path in unique_paths:
        excluded = matches_any(path, excluded_globs) and not matches_any(path, live_exceptions)
        is_live = matches_any(path, live_globs)
        if excluded:
            not_live.append(path)
        elif is_live:
            live_paths.append(path)
        else:
            unclassified.append(path)

    selected_cells: dict[str, Cell] = {}
    uncovered: list[str] = []
    for path in live_paths:
        rows = [row for row in config["cover"] if matches_any(path, row["globs"])]
        if not rows:
            uncovered.append(path)
            continue
        for row in rows:
            for cell_spec in row["cells"]:
                cell = _cell_fields(cell_spec, "cover row")
                previous = selected_cells.get(cell.command)
                if previous is None or cell.name < previous.name:
                    selected_cells[cell.command] = cell
    ordered_cells = tuple(sorted(selected_cells.values(), key=lambda cell: cell.name))
    return Plan(ordered_cells, tuple(uncovered), tuple(unclassified), tuple(not_live))


def _git_output(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=repo,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        detail = result.stderr.strip() or f"git exited {result.returncode}"
        raise MapError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def _gitlink_commit(repo: Path, revision: str) -> str | None:
    output = _git_output(repo, "ls-tree", "-z", revision, "--", CLIENT_SUBMODULE_PATH)
    for entry in output.split("\0"):
        if not entry:
            continue
        metadata, path = entry.split("\t", 1)
        mode, kind, oid = metadata.split()
        if path == CLIENT_SUBMODULE_PATH and mode == "160000" and kind == "commit":
            return oid
    return None


def _submodule_commit_paths(repo: Path, base: str, head: str) -> list[str]:
    old = _gitlink_commit(repo, base)
    new = _gitlink_commit(repo, head)
    if old is None and new is None:
        return []

    submodule = (repo / CLIENT_SUBMODULE_PATH).resolve()
    if not submodule.is_dir() or not (submodule / ".git").exists():
        raise MapError(
            f"client submodule is not initialized; run: "
            f"git submodule update --init {CLIENT_SUBMODULE_PATH}"
        )
    for oid in (old, new):
        if oid is None:
            continue
        result = subprocess.run(
            ["git", "-C", str(submodule), "cat-file", "-e", f"{oid}^{{commit}}"],
            cwd=repo,
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode:
            raise MapError(
                f"client submodule commit {oid} is missing locally; fetch it with: "
                f"git -C {CLIENT_SUBMODULE_PATH} fetch origin {oid}"
            )

    if old is not None and new is not None:
        if old == new:
            return []
        changed = _git_output(
            repo,
            "-C",
            str(submodule),
            "diff",
            "--name-only",
            "--no-renames",
            old,
            new,
        )
    else:
        changed = _git_output(
            repo,
            "-C",
            str(submodule),
            "ls-tree",
            "-r",
            "--name-only",
            old or new or "",
        )
    return [
        f"{CLIENT_SUBMODULE_PATH}/{path}"
        for path in changed.splitlines()
        if path
    ]


def diff_paths(diff_range: str, repo: Path = ROOT) -> list[str]:
    if diff_range.count("..") != 1 or diff_range.startswith("..") or diff_range.endswith("..") or "..." in diff_range:
        raise MapError("--diff needs one BASE..HEAD range")
    result = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", diff_range],
        cwd=repo,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        detail = result.stderr.strip() or f"git diff exited {result.returncode}"
        raise MapError(detail)
    base, head = diff_range.split("..")
    paths: list[str] = []
    for path in result.stdout.splitlines():
        paths.append(path)
        if path == CLIENT_SUBMODULE_PATH:
            paths.extend(_submodule_commit_paths(repo, base, head))
    return list(dict.fromkeys(paths))


def format_plan(plan: Plan) -> str:
    lines = [f"Required cells ({len(plan.cells)}; run each 3 times):"]
    if plan.cells:
        lines.extend(
            line
            for cell in plan.cells
            for line in (
                f"- {cell.name}",
                f"  Command: `{cell.command}`",
                f"  Test selector: `{cell.selector}`",
            )
        )
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
                    {"globs": ["crates/nav/src/router.rs"], "cells": ["nav.routes|cargo test --locked -p nav --lib router::tests::"]},
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

    def test_explicit_live_test_exception_overrides_not_live_glob(self) -> None:
        path = "crates/host-play/tests/survivable_admission_live.rs"
        config = validate_map(
            {
                "live": {"globs": [path]},
                "not_live": {"globs": ["**/tests/**"], "exceptions": [path]},
                "cover": [{"globs": [path], "cells": ["host.admission|cargo test --locked -p host-play --test survivable_admission_live real_compat_v1_crossing_and_own_sustain_at_45_and_14_hp"]}],
            }
        )
        plan = make_plan([path], config)
        self.assertEqual([cell.name for cell in plan.cells], ["host.admission"])
        self.assertEqual(plan.not_live, ())
        self.assertEqual(plan.uncovered, ())

    def test_commands_deduplicate_by_command(self) -> None:
        config = validate_map(
            {
                "live": {"globs": ["src/**"]},
                "not_live": {"globs": ["docs/**"]},
                "cover": [
                    {"globs": ["src/a.rs"], "cells": ["nav.z-route|cargo test --locked -p nav --lib router::tests::"]},
                    {"globs": ["src/b.rs"], "cells": ["nav.a-route|cargo test --locked -p nav --lib router::tests::"]},
                ],
            }
        )
        plan = make_plan(["src/a.rs", "src/b.rs"], config)
        self.assertEqual([cell.name for cell in plan.cells], ["nav.a-route"])

    def test_duplicate_commands_need_the_same_selector(self) -> None:
        raw = {
            "live": {"globs": ["src/**"]},
            "not_live": {"globs": ["docs/**"]},
            "cover": [
                {"globs": ["src/a.rs"], "cells": ["nav.a|panel-play --live nav_a|--locked -p nav --lib router::tests::route"]},
                {"globs": ["src/b.rs"], "cells": ["nav.b|panel-play --live nav_a|--locked -p nav --lib router::tests::search"]},
            ],
        }
        with self.assertRaises(MapError):
            validate_map(raw)

    def test_selector_is_derived_from_the_cell_test_command(self) -> None:
        selector = _test_selector_from_command(
            "ISOHOME nice -n 10 cargo test --locked -p host-play "
            "--features live-harness,memory-profile --lib "
            "-- --ignored --exact api_gather_live_tests::live_receipt --nocapture --test-threads=1"
        )
        self.assertEqual(
            selector,
            "--locked -p host-play --features live-harness,memory-profile --lib "
            "-- --ignored --exact api_gather_live_tests::live_receipt",
        )

    def test_non_test_cell_command_requires_a_selector(self) -> None:
        raw = {
            "live": {"globs": ["src/**"]},
            "not_live": {"globs": ["docs/**"]},
            "cover": [{"globs": ["src/**"], "cells": ["panel.nav|panel-play --live nav_full"]}],
        }
        with self.assertRaises(MapError):
            validate_map(raw)

    def test_uncovered_and_unclassified_paths_are_findings(self) -> None:
        plan = make_plan(["crates/nav/src/new_area.rs", "mystery/file.rs"], self.config)
        self.assertEqual(plan.uncovered, ("crates/nav/src/new_area.rs",))
        self.assertEqual(plan.unclassified, ("mystery/file.rs",))

    def test_conflicting_cell_commands_are_rejected(self) -> None:
        raw = {
            "live": {"globs": ["src/**"]},
            "not_live": {"globs": ["docs/**"]},
            "cover": [
                {"globs": ["src/**"], "cells": ["one|cargo test --locked -p nav --lib router::tests::route"]},
                {"globs": ["src/**"], "cells": ["one|cargo test --locked -p nav --lib router::tests::other"]},
            ],
        }
        with self.assertRaises(MapError):
            validate_map(raw)

    def test_verify_cells_rejects_an_empty_test_list(self) -> None:
        config = validate_map(
            {
                "live": {"globs": ["src/**"]},
                "not_live": {"globs": ["docs/**"]},
                "cover": [
                    {"globs": ["src/**"], "cells": ["nav.routes|cargo test --locked -p nav --lib router::tests::"]}
                ],
            }
        )
        result = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="0 tests, 0 benchmarks\n",
            stderr="",
        )
        output = StringIO()
        with patch.dict(os.environ, {"ISOHOME": sys.executable}), patch(
            "subprocess.run", return_value=result
        ), redirect_stdout(output):
            self.assertEqual(verify_cells(config), 1)
        self.assertIn("selected tests: 0", output.getvalue())

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

    def test_diff_expands_real_submodule_paths_and_requires_missing_commit_fetch(self) -> None:
        raw_evidence = os.environ.get("LIVE_MAP_TEST_TMPDIR")
        self.assertTrue(raw_evidence, "set LIVE_MAP_TEST_TMPDIR to a path under 274bot-evidence")
        evidence = Path(raw_evidence).resolve()
        self.assertIn("274bot-evidence", evidence.parts)
        try:
            evidence.relative_to(Path(tempfile.gettempdir()).resolve())
        except ValueError:
            pass
        else:
            self.fail("LIVE_MAP_TEST_TMPDIR must not be under the system temporary directory")
        evidence.mkdir(parents=True, exist_ok=True)

        with tempfile.TemporaryDirectory(prefix="live-map-submodule-", dir=evidence) as temp:
            root = Path(temp)
            client = root / "client"
            parent = root / "parent"
            client.mkdir()
            parent.mkdir()
            for repo in (client, parent):
                _git(repo, "init", "--quiet")
                _git(repo, "config", "user.name", "Live Map Test")
                _git(repo, "config", "user.email", "live-map-test@example.invalid")

            client_file = client / "crates/client/src/client/client.rs"
            client_file.parent.mkdir(parents=True)
            client_file.write_text("pub fn login() { /* before */ }\n", encoding="utf-8")
            _git(client, "add", "crates/client/src/client/client.rs")
            _git(client, "commit", "--quiet", "-m", "client baseline")

            _git(
                parent,
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                str(client),
                CLIENT_SUBMODULE_PATH,
            )
            _git(parent, "config", "user.name", "Live Map Test")
            _git(parent, "config", "user.email", "live-map-test@example.invalid")
            _git(parent, "commit", "--quiet", "-m", "baseline")
            base = _git_output(parent, "rev-parse", "HEAD").strip()

            client_file.write_text("pub fn login() { /* after */ }\n", encoding="utf-8")
            _git(client, "add", "crates/client/src/client/client.rs")
            _git(client, "commit", "--quiet", "-m", "change login")
            login_commit = _git_output(client, "rev-parse", "HEAD").strip()
            submodule = parent / CLIENT_SUBMODULE_PATH
            _git(submodule, "fetch", "origin", login_commit)
            _git(submodule, "checkout", "--detach", login_commit)
            _git(parent, "add", CLIENT_SUBMODULE_PATH)
            _git(parent, "commit", "--quiet", "-m", "update client login")
            login_update = _git_output(parent, "rev-parse", "HEAD").strip()

            config = load_map()
            login_plan = make_plan(diff_paths(f"{base}..{login_update}", parent), config)
            self.assertEqual(
                {cell.name for cell in login_plan.cells},
                {
                    "host.transport-dead-server",
                    "host.transport-relog",
                    "panel.headed-nav-script",
                    "panel.headed-render",
                },
            )
            self.assertEqual(login_plan.uncovered, ())
            self.assertEqual(login_plan.unclassified, ())

            test_file = client / "crates/client/src/client/client_tests.rs"
            test_file.write_text("#[test] fn test_only_bump() {}\n", encoding="utf-8")
            _git(client, "add", "crates/client/src/client/client_tests.rs")
            _git(client, "commit", "--quiet", "-m", "test-only change")
            test_commit = _git_output(client, "rev-parse", "HEAD").strip()
            _git(submodule, "fetch", "origin", test_commit)
            _git(submodule, "checkout", "--detach", test_commit)
            _git(parent, "add", CLIENT_SUBMODULE_PATH)
            _git(parent, "commit", "--quiet", "-m", "update client tests")
            test_update = _git_output(parent, "rev-parse", "HEAD").strip()

            test_plan = make_plan(diff_paths(f"{login_update}..{test_update}", parent), config)
            self.assertEqual([cell.name for cell in test_plan.cells], ["panel.headed-render"])
            self.assertEqual(test_plan.not_live, (f"{CLIENT_SUBMODULE_PATH}/crates/client/src/client/client_tests.rs",))
            self.assertEqual(test_plan.uncovered, ())
            self.assertEqual(test_plan.unclassified, ())
            moved_old = client / "crates/client/src/client/client.rs"
            moved_new = client / "crates/client/tests/client.rs"
            moved_new.parent.mkdir(parents=True, exist_ok=True)
            moved_old.rename(moved_new)
            _git(client, "add", "-A")
            _git(client, "commit", "--quiet", "-m", "move client source")
            moved_commit = _git_output(client, "rev-parse", "HEAD").strip()
            _git(submodule, "fetch", "origin", moved_commit)
            _git(submodule, "checkout", "--detach", moved_commit)
            _git(parent, "add", CLIENT_SUBMODULE_PATH)
            _git(parent, "commit", "--quiet", "-m", "update client rename")
            moved_update = _git_output(parent, "rev-parse", "HEAD").strip()

            moved_paths = diff_paths(f"{test_update}..{moved_update}", parent)
            old_path = f"{CLIENT_SUBMODULE_PATH}/crates/client/src/client/client.rs"
            new_path = f"{CLIENT_SUBMODULE_PATH}/crates/client/tests/client.rs"
            self.assertIn(old_path, moved_paths)
            self.assertIn(new_path, moved_paths)
            moved_plan = make_plan(moved_paths, config)
            self.assertEqual(
                {cell.name for cell in moved_plan.cells},
                {
                    "host.transport-dead-server",
                    "host.transport-relog",
                    "panel.headed-nav-script",
                    "panel.headed-render",
                },
            )
            self.assertEqual(moved_plan.uncovered, ())
            self.assertEqual(moved_plan.unclassified, ())
            self.assertEqual(moved_plan.not_live, (new_path,))

            missing = "deadbeef" * 5
            _git(
                parent,
                "update-index",
                "--add",
                "--cacheinfo",
                f"160000,{missing},{CLIENT_SUBMODULE_PATH}",
            )
            _git(parent, "commit", "--quiet", "-m", "missing old submodule object")
            missing_base = _git_output(parent, "rev-parse", "HEAD").strip()
            _git(
                parent,
                "update-index",
                "--cacheinfo",
                f"160000,{test_commit},{CLIENT_SUBMODULE_PATH}",
            )
            _git(parent, "commit", "--quiet", "-m", "restore real submodule object")
            missing_head = _git_output(parent, "rev-parse", "HEAD").strip()
            with self.assertRaises(MapError) as error:
                diff_paths(f"{missing_base}..{missing_head}", parent)
            self.assertIn(
                f"git -C {CLIENT_SUBMODULE_PATH} fetch origin {missing}",
                str(error.exception),
            )

    def test_diff_lists_removed_paths_for_renames(self) -> None:
        with tempfile.TemporaryDirectory(prefix="live-map-rename-") as temp:
            repo = Path(temp)
            _git(repo, "init", "--quiet")
            _git(repo, "config", "user.name", "Live Map Test")
            _git(repo, "config", "user.email", "live-map-test@example.invalid")
            old = repo / "crates/nav/src/world_state.rs"
            old.parent.mkdir(parents=True, exist_ok=True)
            old.write_text("base\n", encoding="utf-8")
            _git(repo, "add", "crates/nav/src/world_state.rs")
            _git(repo, "commit", "--quiet", "-m", "base")
            _git(repo, "branch", "baseline")
            new = repo / "crates/nav/src/world_state_tests.rs"
            old.rename(new)
            _git(repo, "add", "-A")
            _git(repo, "commit", "--quiet", "-m", "rename")
            self.assertEqual(
                diff_paths("baseline..HEAD", repo),
                ["crates/nav/src/world_state.rs", "crates/nav/src/world_state_tests.rs"],
            )

    def test_python_310_subset_parser_reads_the_map_shape(self) -> None:
        with tempfile.TemporaryDirectory(prefix="live-map-toml-") as temp:
            path = Path(temp) / "map.toml"
            path.write_text(
                '[live]\nglobs = ["src/**"]\n'
                '[not_live]\nglobs = ["docs/**"]\n'
                '[[cover]]\nglobs = ["src/**"]\n'
                'cells = [\n "one|cargo test --locked -p nav --lib router::tests::",\n]\n',
                encoding="utf-8",
            )
            parsed = validate_map(_load_toml_310(path))
            self.assertEqual(
                parsed["cover"][0]["cells"],
                ["one|cargo test --locked -p nav --lib router::tests::"],
            )


def self_test() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(LiveMapTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--diff", metavar="BASE..HEAD")
    group.add_argument("--self-test", action="store_true")
    group.add_argument("--verify-cells", action="store_true")
    args = parser.parse_args(argv)
    if args.verify_cells:
        try:
            return verify_cells(load_map())
        except (MapError, OSError) as error:
            print(f"live-map: {error}", file=sys.stderr)
            return 2
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
