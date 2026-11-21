#!/usr/bin/env python3
"""Print live cells that cover changed paths in a Git diff."""
from __future__ import annotations

import argparse
import ast
import fnmatch
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from unittest.mock import patch
from dataclasses import asdict, dataclass
from functools import lru_cache
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
MAP_PATH = Path(__file__).with_name("map.toml")
_CELL_NAME = re.compile(r"[A-Za-z0-9_.:-]+\Z")
UPSTREAM_CASES_PATH = Path(__file__).with_name("upstream-cases.json")
UPSTREAM_CASES_RELATIVE_PATH = "tools/live-map/upstream-cases.json"
UPSTREAM_RUNNER_CELL = "host.upstream-e2e-adopted"
CLIENT_SUBMODULE_PATH = "vendor/fr-client-rust"

# These paths affect the shared ABI, Load runtime, or upstream test harness.
# Family-specific behavior remains mapped by upstream-cases.json.
UPSTREAM_SHARED_PATH_GLOBS = (
    "crates/script-js/compat-js/abi-contract.json",
    "crates/script-js/compat-js/abi-contract.cjs",
    "crates/script-js/compat-js/generate.cjs",
    "crates/script-js/schema/isolate.fbs",
    "crates/script-js/src/library/**",
    "crates/script-js/src/materialize/**",
    "crates/script-js/src/runtime/**",
    "crates/script-js/src/shim/**",
    "crates/script-js/src/wire/**",
    "crates/script-js/src/surface.rs",
    "crates/script/src/native/abi.rs",
    "crates/script/src/native/request.rs",
    "crates/script/src/slot.rs",
    "crates/script/src/slot/**",
    "crates/host-play/src/play_scripts.rs",
    "crates/host-play/src/script_runtime.rs",
    "crates/host-play/tests/upstream_e2e.rs",
    "crates/host-play/tests/upstream_e2e/**",
)


class MapError(ValueError):
    """The mapping file does not match the supported format."""


@dataclass(frozen=True)
class Cell:
    name: str
    command: str
    selector: str


@dataclass(frozen=True)
class UpstreamCase:
    id: str
    path: str
    family: str
    capabilities: tuple[str, ...]
    deadline_seconds: int
    gate_class: str
    eligibility: str
    reason: str
    family_paths: tuple[str, ...]
    sha256: str | None = None


@dataclass(frozen=True)
class UpstreamManifest:
    pin: str
    abi_pin: str
    cases: tuple[UpstreamCase, ...]


@dataclass(frozen=True)
class Plan:
    cells: tuple[Cell, ...]
    uncovered: tuple[str, ...]
    unclassified: tuple[str, ...]
    not_live: tuple[str, ...]
    upstream_cases: tuple[UpstreamCase, ...] = ()
    upstream_runner: Cell | None = None

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
    isohome_value = os.environ.get("ISOHOME")
    if not isohome_value:
        print(
            "live-map: set ISOHOME in your local lab.env to the absolute isohome executable",
            file=sys.stderr,
        )
        return 2
    isohome = Path(isohome_value)
    if (
        not isohome.is_absolute()
        or not isohome.is_file()
        or not os.access(isohome, os.X_OK)
    ):
        print(
            f"live-map: ISOHOME must name the absolute executable isohome wrapper: {isohome}",
            file=sys.stderr,
        )
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


def load_upstream_manifest(
    path: Path = UPSTREAM_CASES_PATH,
) -> UpstreamManifest:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise MapError(f"cannot read upstream case metadata {path}: {error}") from error
    if not isinstance(raw, dict) or set(raw) != {"pin", "abi_pin", "cases"}:
        raise MapError(
            "upstream case metadata must contain only pin, abi_pin, and cases"
        )
    pin = raw["pin"]
    abi_pin = raw["abi_pin"]
    if not isinstance(pin, str) or re.fullmatch(r"[0-9a-fA-F]{40}", pin) is None:
        raise MapError("upstream case metadata pin must be a full 40-character commit")
    if not isinstance(abi_pin, str) or re.fullmatch(r"[0-9a-fA-F]{40}", abi_pin) is None:
        raise MapError("upstream case metadata abi_pin must be a full 40-character SHA")
    if not isinstance(raw["cases"], list):
        raise MapError("upstream case metadata cases must be an array")

    required = {
        "id",
        "path",
        "family",
        "capabilities",
        "deadline_seconds",
        "gate_class",
        "eligibility",
        "reason",
        "family_paths",
    }
    cases: list[UpstreamCase] = []
    seen_ids: set[str] = set()
    seen_paths: set[str] = set()
    for index, entry in enumerate(raw["cases"], start=1):
        label = f"upstream case {index}"
        if not isinstance(entry, dict) or set(entry) - (required | {"sha256"}) or not required.issubset(entry):
            raise MapError(f"{label} has invalid fields")
        case_id = entry["id"]
        path_value = entry["path"]
        family = entry["family"]
        capabilities = entry["capabilities"]
        deadline = entry["deadline_seconds"]
        gate_class = entry["gate_class"]
        eligibility = entry["eligibility"]
        reason = entry["reason"]
        family_paths = entry["family_paths"]
        source_sha256 = entry.get("sha256")
        if source_sha256 is not None and (
            not isinstance(source_sha256, str)
            or re.fullmatch(r"[0-9a-fA-F]{64}", source_sha256) is None
        ):
            raise MapError(f"{label} sha256 must be a 64-character digest")
        if not isinstance(case_id, str) or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*", case_id) is None:
            raise MapError(f"{label} id must be a safe case basename")
        if case_id in seen_ids:
            raise MapError(f"duplicate upstream case id: {case_id}")
        if (
            not isinstance(path_value, str)
            or not path_value.startswith("e2e/")
            or not path_value.endswith(".ts")
            or "\\" in path_value
            or any(part in ("", ".", "..") for part in path_value.split("/"))
        ):
            raise MapError(f"{label} path must be a relative e2e/*.ts path")
        if path_value in seen_paths:
            raise MapError(f"duplicate upstream case path: {path_value}")
        if not isinstance(family, str) or not family.strip():
            raise MapError(f"{label} family must be a non-empty string")
        if not isinstance(capabilities, list) or any(
            not isinstance(value, str) or not value for value in capabilities
        ):
            raise MapError(f"{label} capabilities must be an array of non-empty strings")
        if type(deadline) is not int or deadline < 1:
            raise MapError(f"{label} deadline_seconds must be a positive integer")
        if gate_class not in ("gating", "non-gating"):
            raise MapError(f"{label} gate_class must be gating or non-gating")
        if eligibility not in ("adopted", "excluded"):
            raise MapError(f"{label} eligibility must be adopted or excluded")
        if eligibility == "excluded" and gate_class != "non-gating":
            raise MapError(f"{label} excluded cases must be non-gating")
        if not isinstance(reason, str) or (eligibility == "excluded" and not reason.strip()):
            raise MapError(f"{label} reason must explain each exclusion")
        if not isinstance(family_paths, list) or any(
            not isinstance(pattern, str)
            or not pattern
            or pattern.startswith("/")
            or "\\" in pattern
            for pattern in family_paths
        ):
            raise MapError(f"{label} family_paths must be relative glob strings")
        cases.append(
            UpstreamCase(
                id=case_id,
                path=path_value,
                family=family,
                capabilities=tuple(capabilities),
                deadline_seconds=deadline,
                gate_class=gate_class,
                eligibility=eligibility,
                reason=reason,
                family_paths=tuple(family_paths),
                sha256=source_sha256,
            )
        )
        seen_ids.add(case_id)
        seen_paths.add(path_value)
    if not any(case.eligibility == "adopted" for case in cases):
        raise MapError("upstream case metadata must include an adopted case")
    return UpstreamManifest(pin=pin, abi_pin=abi_pin, cases=tuple(cases))


def select_upstream_cases(
    paths: list[str], manifest: UpstreamManifest
) -> tuple[UpstreamCase, ...]:
    adopted = tuple(case for case in manifest.cases if case.eligibility == "adopted")
    if not adopted:
        return ()
    full_corpus = any(
        matches(path, UPSTREAM_CASES_RELATIVE_PATH)
        or matches_any(path, list(UPSTREAM_SHARED_PATH_GLOBS))
        for path in paths
    )
    if full_corpus:
        return adopted

    selected_ids: set[str] = set()
    selected_families: set[str] = set()
    for path in paths:
        for case in adopted:
            if matches(path, case.path):
                selected_ids.add(case.id)
            if matches_any(path, list(case.family_paths)):
                selected_families.add(case.family)
    return tuple(
        case
        for case in adopted
        if case.id in selected_ids or case.family in selected_families
    )


def resolve_upstream_cases(
    manifest: UpstreamManifest, case_ids: list[str]
) -> tuple[UpstreamCase, ...]:
    if not case_ids or any(not case_id for case_id in case_ids):
        raise MapError("UEP_CASES must contain one or more comma-separated case ids")
    if len(set(case_ids)) != len(case_ids):
        raise MapError("UEP_CASES contains duplicate case ids")
    cases_by_id = {case.id: case for case in manifest.cases}
    missing = [case_id for case_id in case_ids if case_id not in cases_by_id]
    if missing:
        raise MapError(f"unknown upstream case ids: {', '.join(missing)}")
    excluded = [
        case_id
        for case_id in case_ids
        if cases_by_id[case_id].eligibility != "adopted"
    ]
    if excluded:
        raise MapError(f"UEP_CASES cannot select excluded cases: {', '.join(excluded)}")
    return tuple(cases_by_id[case_id] for case_id in case_ids)


def _is_exact_test_source_cover(path: str, config: dict[str, Any]) -> bool:
    return any(
        pattern.endswith(".rs")
        and "/tests/" in f"/{pattern}"
        and not any(character in pattern for character in "*?[]")
        and matches(path, pattern)
        for row in config["cover"]
        for pattern in row["globs"]
    )


def make_plan(
    paths: list[str],
    config: dict[str, Any],
    upstream_manifest: UpstreamManifest | None = None,
) -> Plan:
    unique_paths = sorted(set(paths))
    live_globs = config["live"]["globs"]
    excluded_globs = config["not_live"]["globs"]
    live_exceptions = config["not_live"].get("exceptions", [])
    live_paths: list[str] = []
    not_live: list[str] = []
    unclassified: list[str] = []
    for path in unique_paths:
        covered = any(matches_any(path, row["globs"]) for row in config["cover"])
        exact_test_source = _is_exact_test_source_cover(path, config)
        excluded = matches_any(path, excluded_globs) and not matches_any(
            path, live_exceptions
        )
        is_live = matches_any(path, live_globs)
        if exact_test_source:
            live_paths.append(path)
        elif excluded:
            not_live.append(path)
        elif covered or is_live:
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

    upstream_cases = (
        select_upstream_cases(unique_paths, upstream_manifest)
        if upstream_manifest is not None
        else ()
    )
    upstream_runner: Cell | None = None
    if upstream_cases:
        for index, row in enumerate(config["cover"]):
            for cell_spec in row["cells"]:
                candidate = _cell_fields(cell_spec, f"cover row {index + 1}")
                if candidate.name == UPSTREAM_RUNNER_CELL:
                    upstream_runner = candidate
                    break
            if upstream_runner is not None:
                break
        if upstream_runner is None:
            raise MapError(
                f"map.toml must define the {UPSTREAM_RUNNER_CELL!r} cell"
            )
        selected_cells[upstream_runner.command] = upstream_runner

    ordered_cells = tuple(sorted(selected_cells.values(), key=lambda cell: cell.name))
    return Plan(
        ordered_cells,
        tuple(uncovered),
        tuple(unclassified),
        tuple(not_live),
        upstream_cases=upstream_cases,
        upstream_runner=upstream_runner,
    )


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
    native_cells = tuple(
        cell for cell in plan.cells if cell.name != UPSTREAM_RUNNER_CELL
    )
    lines = [f"Required native cells ({len(native_cells)}; run each 3 times):"]
    if native_cells:
        lines.extend(
            line
            for cell in native_cells
            for line in (
                f"- {cell.name}",
                f"  Command: `{cell.command}`",
                f"  Test selector: `{cell.selector}`",
            )
        )
    else:
        lines.append("- None")
    lines.append(f"Adopted upstream cases ({len(plan.upstream_cases)}):")
    lines.extend(
        f"- {case.id} [{case.family}; {case.gate_class}]"
        for case in plan.upstream_cases
    )
    if not plan.upstream_cases:
        lines.append("- None")
    if plan.upstream_runner is not None:
        runner = plan.upstream_runner
        lines.extend(
            (
                f"Upstream runner (one grouped invocation): {runner.name}",
                f"  Command: `{runner.command}`",
                f"  Test selector: `{runner.selector}`",
            )
        )
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


    def upstream_manifest(self) -> UpstreamManifest:
        shared = ("crates/script-js/src/runtime/**",)
        gather_paths = ("crates/script-js/src/compat/gather/**", *shared)
        return UpstreamManifest(
            pin="a" * 40,
            abi_pin="d" * 40,
            cases=(
                UpstreamCase(
                    id="gather-one",
                    path="e2e/gather-one-live.ts",
                    family="gather",
                    capabilities=("inventory",),
                    deadline_seconds=60,
                    gate_class="gating",
                    eligibility="adopted",
                    reason="",
                    family_paths=gather_paths,
                ),
                UpstreamCase(
                    id="gather-two",
                    path="e2e/gather-two-live.ts",
                    family="gather",
                    capabilities=("inventory",),
                    deadline_seconds=60,
                    gate_class="non-gating",
                    eligibility="adopted",
                    reason="",
                    family_paths=gather_paths,
                ),
                UpstreamCase(
                    id="combat-one",
                    path="e2e/combat-one-live.ts",
                    family="combat",
                    capabilities=("combat",),
                    deadline_seconds=90,
                    gate_class="gating",
                    eligibility="adopted",
                    reason="",
                    family_paths=("crates/script-js/src/compat/combat/**", *shared),
                ),
                UpstreamCase(
                    id="gather-excluded",
                    path="e2e/gather-excluded-live.ts",
                    family="gather",
                    capabilities=("two-accounts",),
                    deadline_seconds=60,
                    gate_class="non-gating",
                    eligibility="excluded",
                    reason="requires two accounts",
                    family_paths=gather_paths,
                ),
            ),
        )

    def test_excluded_upstream_case_cannot_be_a_gate(self) -> None:
        raw = json.loads(json.dumps(asdict(self.upstream_manifest())))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "cases.json"
            path.write_text(json.dumps(raw), encoding="utf-8")
            load_upstream_manifest(path)
            raw["cases"][-1]["gate_class"] = "gating"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(MapError, "excluded cases must be non-gating"):
                load_upstream_manifest(path)

    def test_upstream_family_change_selects_adopted_family_as_one_group(self) -> None:
        config = validate_map(
            {
                "live": {"globs": ["crates/script-js/src/compat/gather/**"]},
                "not_live": {"globs": ["tools/**"]},
                "cover": [
                    {
                        "globs": ["tools/live-map/upstream-cases.json"],
                        "cells": [
                            "host.upstream-e2e-adopted|${ISOHOME:?} cargo test --locked -p host-play --features test-support,js --test upstream_e2e upstream_e2e_corpus -- --ignored --nocapture|--locked -p host-play --features test-support,js --test upstream_e2e upstream_e2e_corpus -- --ignored --nocapture"
                        ],
                    }
                ],
            }
        )
        plan = make_plan(
            ["crates/script-js/src/compat/gather/worker.rs"], config, self.upstream_manifest()
        )
        self.assertEqual(
            [case.id for case in plan.upstream_cases],
            ["gather-one", "gather-two"],
        )
        self.assertEqual(
            [cell.name for cell in plan.cells],
            ["host.upstream-e2e-adopted"],
        )
        self.assertEqual(plan.upstream_runner, plan.cells[0])

    def test_upstream_shared_paths_and_metadata_changes_select_full_adopted_set(self) -> None:
        manifest = self.upstream_manifest()
        adopted_ids = ["gather-one", "gather-two", "combat-one"]
        for path in (
            "crates/script-js/src/runtime/bindings.rs",
            "crates/script-js/src/materialize/skills_v8.rs",
            "crates/script-js/src/library/prepare.rs",
            "crates/script-js/schema/isolate.fbs",
            UPSTREAM_CASES_RELATIVE_PATH,
        ):
            with self.subTest(path=path):
                self.assertEqual(
                    [case.id for case in select_upstream_cases([path], manifest)],
                    adopted_ids,
                )

    def test_upstream_exact_case_and_native_or_ui_changes_stay_narrow(self) -> None:
        manifest = self.upstream_manifest()
        self.assertEqual(
            [case.id for case in select_upstream_cases(["e2e/gather-one-live.ts"], manifest)],
            ["gather-one"],
        )
        self.assertEqual(select_upstream_cases(["crates/nav/src/router.rs"], manifest), ())
        native_plan = make_plan(
            ["crates/nav/src/router.rs"], self.config, manifest
        )
        self.assertEqual([cell.name for cell in native_plan.cells], ["nav.routes"])
        self.assertEqual(native_plan.upstream_cases, ())
        ui_plan = make_plan(["crates/panel/src/views.rs"], self.config, manifest)
        self.assertEqual(ui_plan.upstream_cases, ())
        self.assertIsNone(ui_plan.upstream_runner)

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

    def test_exact_test_source_cover_overrides_not_live_glob(self) -> None:
        path = "crates/host-play/tests/revision_377_live/menu.rs"
        config = validate_map(
            {
                "live": {"globs": ["src/**"]},
                "not_live": {"globs": ["**/tests/**"]},
                "cover": [
                    {
                        "globs": [path],
                        "cells": [
                            "host.menu-377-trade-duel-loc-live|cargo test --locked -p host-play --test revision_377_live -- --ignored --exact menu::trade_duel_loc_menu_377"
                        ],
                    }
                ],
            }
        )
        plan = make_plan([path], config)
        self.assertEqual(
            [cell.name for cell in plan.cells],
            ["host.menu-377-trade-duel-loc-live"],
        )
        self.assertEqual(plan.not_live, ())
        self.assertEqual(plan.uncovered, ())
        self.assertEqual(plan.unclassified, ())

    def test_wildcard_covers_and_non_test_exact_covers_stay_not_live(self) -> None:
        config = load_map()
        for path in (
            "crates/script/src/quester/inventory_tests.rs",
            "Cargo.lock",
        ):
            with self.subTest(path=path):
                plan = make_plan([path], config)
                self.assertEqual(plan.cells, ())
                self.assertEqual(plan.uncovered, ())
                self.assertEqual(plan.unclassified, ())
                self.assertEqual(plan.not_live, (path,))

    def test_diff_of_live_test_sources_selects_their_cells(self) -> None:
        config = load_map()
        with tempfile.TemporaryDirectory(prefix="live-map-test-sources-") as temp:
            repo = Path(temp)
            _git(repo, "init", "--quiet")
            _git(repo, "config", "user.name", "Live Map Test")
            _git(repo, "config", "user.email", "live-map-test@example.invalid")
            (repo / "README.md").write_text("base\n", encoding="utf-8")
            _git(repo, "add", "README.md")
            _git(repo, "commit", "--quiet", "-m", "base")
            _git(repo, "branch", "baseline")
            changed = [
                "crates/host-play/tests/revision_377_live/menu.rs",
                "crates/host-play/tests/quester_path_live.rs",
                "crates/host-play/tests/revision_377_live/cook.rs",
                "crates/host-play/tests/revision_377_live/quantity.rs",
                "crates/host-play/tests/revision_377_live/varbit.rs",
            ]
            for relative in changed:
                target = repo / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("changed\n", encoding="utf-8")
            _git(repo, "add", *changed)
            _git(repo, "commit", "--quiet", "-m", "change test sources")
            paths = diff_paths("baseline..HEAD", repo)
            plan = make_plan(paths, config)
        selected = {cell.name for cell in plan.cells}
        self.assertIn("host.menu-377-trade-duel-loc-live", selected)
        self.assertIn("host.quester-path-engine-q", selected)
        self.assertIn("host.receipt-377-cook-raster-live", selected)
        self.assertIn("host.quantity-377-make-x-live", selected)
        self.assertIn("host.varbit-377-known-missing-live", selected)
        self.assertEqual(plan.not_live, ())
        self.assertEqual(plan.uncovered, ())
        self.assertEqual(plan.unclassified, ())
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

    def test_verify_cells_requires_configured_isohome(self) -> None:
        output = StringIO()
        with patch.dict(os.environ, {}, clear=True), redirect_stderr(output):
            self.assertEqual(verify_cells(self.config), 2)
        self.assertIn("ISOHOME", output.getvalue())
        self.assertIn("lab.env", output.getvalue())

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

    def test_shared_protocol_paths_select_headless_without_377_live_cells(self) -> None:
        config = load_map()
        for path in (
            "vendor/fr-client-rust/crates/client/src/client/client.rs",
            "vendor/fr-client-rust/crates/client/src/io/client_stream.rs",
            "vendor/fr-client-rust/crates/client/src/io/packet.rs",
        ):
            with self.subTest(path=path):
                plan = make_plan([path], config)
                self.assertEqual(
                    {cell.name for cell in plan.cells if "377" in cell.name},
                    {"client.protocol-377-wire-headless"},
                )
                self.assertEqual(plan.uncovered, ())
                self.assertEqual(plan.unclassified, ())

    def test_ondemand_path_selects_377_live_receipt(self) -> None:
        config = load_map()
        plan = make_plan(
            ["vendor/fr-client-rust/crates/client/src/io/ondemand.rs"], config
        )
        self.assertEqual(
            {cell.name for cell in plan.cells if "377" in cell.name},
            {"client.protocol-377-wire-headless", "host.receipt-377-client-live"},
        )
        self.assertEqual(plan.uncovered, ())
        self.assertEqual(plan.unclassified, ())

    def test_377_protocol_paths_select_both_live_cells(self) -> None:
        config = load_map()
        required = {"host.receipt-377-client-live", "host.menu-377-hans-live"}
        for path in (
            "vendor/fr-client-rust/crates/client/src/client/outbound_377.rs",
            "vendor/fr-client-rust/crates/client/src/io/client_prot_377.rs",
        ):
            with self.subTest(path=path):
                plan = make_plan([path], config)
                self.assertTrue(required.issubset({cell.name for cell in plan.cells}))
                self.assertEqual(plan.uncovered, ())
                self.assertEqual(plan.unclassified, ())

    def test_377_outbound_login_path_selects_headless_wire_fixture(self) -> None:
        config = load_map()
        plan = make_plan(
            ["vendor/fr-client-rust/crates/client/src/client/outbound_377.rs"], config
        )
        self.assertIn("client.protocol-377-wire-headless", {cell.name for cell in plan.cells})
        self.assertEqual(plan.uncovered, ())
        self.assertEqual(plan.unclassified, ())

    def test_diff_expands_real_submodule_paths_and_requires_missing_commit_fetch(self) -> None:
        raw_evidence = os.environ.get("LIVE_MAP_TEST_TMPDIR")
        raw_root = os.environ.get("LIVE_EVIDENCE_DIR")
        self.assertTrue(raw_root, "set LIVE_EVIDENCE_DIR to the configured evidence root")
        self.assertTrue(
            raw_evidence,
            "set LIVE_MAP_TEST_TMPDIR to a path under LIVE_EVIDENCE_DIR",
        )
        evidence_root = Path(raw_root).resolve()
        evidence = Path(raw_evidence).resolve()
        try:
            evidence.relative_to(evidence_root)
        except ValueError:
            self.fail("LIVE_MAP_TEST_TMPDIR must be under LIVE_EVIDENCE_DIR")
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
                    "client.protocol-377-wire-headless",
                    "host.scriptless-idle-held",
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
                    "client.protocol-377-wire-headless",
                    "host.scriptless-idle-held",
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
    with tempfile.TemporaryDirectory(
        prefix=".live-map-self-test-", dir=ROOT
    ) as temp:
        evidence_root = Path(temp) / "evidence"
        test_tmpdir = evidence_root / "live-map-test"
        test_tmpdir.mkdir(parents=True)
        with patch.dict(
            os.environ,
            {
                "LIVE_EVIDENCE_DIR": str(evidence_root),
                "LIVE_MAP_TEST_TMPDIR": str(test_tmpdir),
            },
        ):
            suite = unittest.defaultTestLoader.loadTestsFromTestCase(LiveMapTests)
            result = unittest.TextTestRunner(verbosity=2).run(suite)
            return 0 if result.wasSuccessful() else 1


def main(argv: list[str] | None = None) -> int:
    raw = list(argv) if argv is not None else sys.argv[1:]
    if raw and raw[0] == "run":
        try:
            import run_cells
        except ModuleNotFoundError:
            sys.path.insert(0, str(Path(__file__).resolve().parent))
            import run_cells
        return run_cells.main(raw[1:])
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
        manifest = load_upstream_manifest()
        paths = diff_paths(args.diff)
        plan = make_plan(paths, config, manifest)
    except (MapError, OSError) as error:
        print(f"live-map: {error}", file=sys.stderr)
        return 2
    print(format_plan(plan))
    return 1 if plan.uncovered or plan.unclassified else 0


if __name__ == "__main__":
    raise SystemExit(main())
