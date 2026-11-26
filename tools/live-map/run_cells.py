#!/usr/bin/env python3
"""Run live cells listed in map.toml.

The runner uses one worker by default. Pass ``--jobs N`` to run independent
cells concurrently. Each cell keeps its own evidence, cache, and account prefix.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import datetime
import hashlib
import json
import math
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
import unittest.mock
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

try:
    import live_map
except ModuleNotFoundError:  # pragma: no cover - direct path use
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import live_map

RUNNER_DIR = Path(__file__).resolve().parent
DEFAULT_ENGINE_GAME_PORT = 44594
DEFAULT_ENGINE_HTTP_PORT = 1080
DEFAULT_QUEST_GAME_PORT = 44694
DEFAULT_QUEST_HTTP_PORT = 1180
DEFAULT_PASSES = 3
_PREFIX_ALPHABET = "0123456789abcdefghijklmnopqrstuvwxyz"
_PREFIX_SPACE = len(_PREFIX_ALPHABET) ** 4
_PINNED_PREFIX_CELLS = {
    "host.grinch-cross-region-return": "gr25",
    "host.prereq-phoenix-kitchen": "npr2",
}
_EXCLUSIVE_CELLS = frozenset(
    {
        "host.upstream-e2e-adopted",
        "host.interference-377-measured-live",
    }
)
_CARGO_VALUE_OPTIONS = frozenset(
    {
        "-p",
        "--package",
        "-F",
        "--features",
        "--test",
        "--bin",
        "--example",
        "--bench",
        "--profile",
        "--target",
        "--target-dir",
        "--manifest-path",
        "--config",
        "--exclude",
        "--color",
        "--message-format",
    }
)
_CARGO_FLAG_OPTIONS = frozenset(
    {
        "--locked",
        "--frozen",
        "--offline",
        "--release",
        "--all-features",
        "--no-default-features",
        "--workspace",
        "--tests",
        "--benches",
        "--examples",
        "--bins",
        "--lib",
    }
)


@dataclass(frozen=True)
class RunResult:
    cell: str
    passed_pass: int
    exit_code: int
    duration_seconds: int
    log_path: Path
    receipt_path: Path
    command: str
    start_utc: str
    end_utc: str
    upstream_results: tuple["UpstreamCaseResult", ...] = ()


@dataclass(frozen=True)
class UpstreamCaseResult:
    id: str
    verdict: str
    gate_class: str
    reason: str
    seconds: float
    classification: str | None


def _port(value: str, label: str) -> int:
    try:
        number = int(value, 10)
    except ValueError:
        raise argparse.ArgumentTypeError(f"{label} must be a port number, got {value!r}")
    if not 1 <= number <= 65535:
        raise argparse.ArgumentTypeError(f"{label} must be 1-65535, got {value!r}")
    return number


def _prefix_type(value: str) -> str:
    if not 1 <= len(value) <= 4:
        raise argparse.ArgumentTypeError(
            f"--prefix must be 1-4 characters, got {value!r}"
        )
    return value


def _passes_type(value: str) -> int:
    try:
        number = int(value, 10)
    except ValueError:
        raise argparse.ArgumentTypeError(f"--passes must be an integer, got {value!r}")
    if number < 1:
        raise argparse.ArgumentTypeError(f"--passes must be at least 1, got {value!r}")
    return number


def _jobs_type(value: str) -> int:
    try:
        number = int(value, 10)
    except ValueError:
        raise argparse.ArgumentTypeError(f"--jobs must be an integer, got {value!r}")
    if number < 1:
        raise argparse.ArgumentTypeError(f"--jobs must be at least 1, got {value!r}")
    return number


def _literal_env_assignment(command: str, name: str) -> str | None:
    for token in shlex.split(command):
        if not token.startswith(f"{name}="):
            continue
        value = token.split("=", 1)[1]
        if "$" not in value:
            return value
    return None


def _derived_prefix(seed: str, cell_name: str, role: str, used: set[str]) -> str:
    used_keys = {value.casefold() for value in used}
    for attempt in range(_PREFIX_SPACE):
        material = f"{seed}\0{cell_name}\0{role}\0{attempt}".encode("utf-8")
        value = int.from_bytes(hashlib.sha256(material).digest()[:8], "big")
        candidate_chars = ["0"] * 4
        for index in range(3, -1, -1):
            candidate_chars[index] = _PREFIX_ALPHABET[value % 36]
            value //= 36
        candidate = "".join(candidate_chars)
        candidate_key = candidate.casefold()
        if candidate_key not in used_keys:
            used.add(candidate)
            return candidate
    raise live_map.MapError("run-cells: could not allocate a unique live name prefix")


def _cell_prefixes(
    cells: list[live_map.Cell],
    prefix: str,
    partner_prefix: str | None,
    jobs: int,
) -> list[tuple[str, str | None]]:
    partner_seed = partner_prefix or os.environ.get("BOT_LIVE_PARTNER_NAME_PREFIX")
    if jobs == 1:
        return [
            (
                _literal_env_assignment(cell.command, "BOT_LIVE_NAME_PREFIX") or prefix,
                _literal_env_assignment(
                    cell.command, "BOT_LIVE_PARTNER_NAME_PREFIX"
                )
                or partner_seed,
            )
            for cell in cells
        ]
    if len(cells) == 1:
        cell = cells[0]
        primary = _literal_env_assignment(cell.command, "BOT_LIVE_NAME_PREFIX")
        if primary is None:
            primary = _PINNED_PREFIX_CELLS.get(cell.name, prefix)
        partner = _literal_env_assignment(
            cell.command, "BOT_LIVE_PARTNER_NAME_PREFIX"
        ) or partner_seed
        return [(primary, partner)]

    used = {prefix}
    used.update(_PINNED_PREFIX_CELLS.values())
    if partner_seed:
        used.add(partner_seed)
    fixed: list[tuple[str | None, str | None]] = []
    for cell in cells:
        primary = _literal_env_assignment(cell.command, "BOT_LIVE_NAME_PREFIX")
        if primary is None:
            primary = _PINNED_PREFIX_CELLS.get(cell.name)
        partner = _literal_env_assignment(
            cell.command, "BOT_LIVE_PARTNER_NAME_PREFIX"
        )
        fixed.append((primary, partner))
        if primary is not None:
            used.add(primary)
        if partner is not None:
            used.add(partner)

    assignments = []
    for cell, (fixed_primary, fixed_partner) in zip(cells, fixed):
        if fixed_primary is not None:
            primary = fixed_primary
        else:
            primary = _derived_prefix(prefix, cell.name, "primary", used)
        if "BOT_LIVE_PARTNER_NAME_PREFIX" in cell.command and partner_seed:
            partner = fixed_partner or _derived_prefix(
                partner_seed, cell.name, "partner", used
            )
        else:
            partner = fixed_partner
        assignments.append((primary, partner))
    return assignments


def is_exclusive_cell(cell: live_map.Cell) -> bool:
    if cell.name in _EXCLUSIVE_CELLS or cell.name in _PINNED_PREFIX_CELLS:
        return True
    if _literal_env_assignment(cell.command, "BOT_LIVE_NAME_PREFIX") is not None:
        return True
    if _literal_env_assignment(
        cell.command, "BOT_LIVE_PARTNER_NAME_PREFIX"
    ) is not None:
        return True
    return bool(
        is_quest_cell(cell.command)
        and (
            os.environ.get("QUESTER_LIVE_ACCOUNT")
            or "QUESTER_LIVE_ACCOUNT=" in cell.command
        )
    )


def _cell_priority(cell: live_map.Cell) -> int:
    name = cell.name.lower()
    command = cell.command.lower()
    if is_exclusive_cell(cell):
        return 0
    if (
        is_quest_cell(cell.command)
        or "upstream" in name
        or "bot_live_partner_name_prefix" in command
        or any(marker in name for marker in ("pair", "trade", "duel", "cohort", "burst"))
        or "cargo run --release" in command
    ):
        return 1
    if "live=1" in command:
        return 2
    return 3


def _cargo_test_build_key(args: list[str]) -> tuple[str, ...]:
    options: list[str] = []
    features: set[str] = set()
    index = 0
    while index < len(args):
        argument = args[index]
        if argument == "--":
            break
        if argument in {"-F", "--features"}:
            if index + 1 < len(args):
                features.update(
                    part for part in re.split(r"[\s,]+", args[index + 1]) if part
                )
            index += 2
            continue
        if argument.startswith("--features="):
            features.update(
                part
                for part in re.split(r"[\s,]+", argument.split("=", 1)[1])
                if part
            )
            index += 1
            continue
        if argument.startswith("-F") and argument != "-F":
            features.update(
                part for part in re.split(r"[\s,]+", argument[2:]) if part
            )
            index += 1
            continue
        if argument in _CARGO_FLAG_OPTIONS:
            options.append(argument)
            index += 1
            continue
        if argument in _CARGO_VALUE_OPTIONS:
            options.append(argument)
            if index + 1 < len(args):
                options.append(args[index + 1])
            index += 2
            continue
        if argument.startswith("-"):
            options.append(argument)
        index += 1
    if features:
        options.append("features=" + ",".join(sorted(features)))
    return tuple(options)


def _cargo_test_precompile(command: str) -> tuple[tuple[str, ...], str] | None:
    if any(character in command for character in ";&|<>()'`\n\r"):
        return None
    try:
        tokens = shlex.split(command)
    except ValueError:
        return None
    prefix_index = 0
    while prefix_index < len(tokens) and re.fullmatch(
        r"[A-Za-z_][A-Za-z0-9_]*=.*", tokens[prefix_index]
    ):
        prefix_index += 1
    if prefix_index < len(tokens) and tokens[prefix_index] == "${ISOHOME:?}":
        prefix_index += 1
    if prefix_index < len(tokens) and tokens[prefix_index] == "nice":
        prefix_index += 1
        if prefix_index < len(tokens) and tokens[prefix_index] == "-n":
            if (
                prefix_index + 1 >= len(tokens)
                or re.fullmatch(r"-?\d+", tokens[prefix_index + 1]) is None
            ):
                return None
            prefix_index += 2
        elif prefix_index < len(tokens) and tokens[prefix_index].startswith("-n"):
            if re.fullmatch(r"-n-?\d+", tokens[prefix_index]) is None:
                return None
            prefix_index += 1
    if tokens[prefix_index : prefix_index + 2] != ["cargo", "test"]:
        return None
    match = re.search(r"\bcargo\s+test(?=\s|$)", command)
    if match is None:
        return None
    cargo_args = tokens[prefix_index + 2 :]
    if "--no-run" in cargo_args:
        return None
    return _cargo_test_build_key(cargo_args), (
        command[: match.end()] + " --no-run" + command[match.end() :]
    )


def _tail_file(path: Path, max_lines: int = 80) -> str:
    try:
        with path.open(encoding="utf-8", errors="replace") as stream:
            lines = collections.deque(stream, maxlen=max_lines)
    except OSError as error:
        return f"(could not read precompile log: {error})"
    return "".join(lines).rstrip()


def _precompile_failure_message(path: Path, detail: str) -> str:
    return (
        f"Cargo precompile failed: {detail}; full log: {path}\n"
        f"Last 80 precompile log lines:\n{_tail_file(path)}"
    )

class _CellExecutionGate:
    def __init__(self) -> None:
        self._condition = threading.Condition()
        self._active = 0
        self._exclusive_active = False
        self._waiting_exclusive = 0
        self._cancelled = False

    def cancel(self) -> None:
        with self._condition:
            self._cancelled = True
            self._condition.notify_all()

    def run(
        self, operation: Callable[[], list[RunResult]], *, exclusive: bool
    ) -> list[RunResult]:
        with self._condition:
            if self._cancelled:
                raise concurrent.futures.CancelledError()
            if exclusive:
                self._waiting_exclusive += 1
                try:
                    while self._active or self._exclusive_active:
                        if self._cancelled:
                            raise concurrent.futures.CancelledError()
                        self._condition.wait()
                    if self._cancelled:
                        raise concurrent.futures.CancelledError()
                    self._exclusive_active = True
                finally:
                    self._waiting_exclusive -= 1
            else:
                while self._exclusive_active or self._waiting_exclusive:
                    if self._cancelled:
                        raise concurrent.futures.CancelledError()
                    self._condition.wait()
                if self._cancelled:
                    raise concurrent.futures.CancelledError()
                self._active += 1
        try:
            return operation()
        finally:
            with self._condition:
                if exclusive:
                    self._exclusive_active = False
                else:
                    self._active -= 1
                self._condition.notify_all()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, help="Lab clone path that runs the cells.")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--cells", help="Comma-separated cell ids from map.toml.")
    group.add_argument("--diff", metavar="BASE..HEAD", help="Git range listed by live_map.")
    parser.add_argument("--prefix", required=True, type=_prefix_type,
                        help="Bot live name prefix, 1-4 characters (3 is the lab convention).")
    parser.add_argument("--evidence", required=True, help="Evidence directory that receives receipts.")
    parser.add_argument("--passes", type=_passes_type, default=DEFAULT_PASSES,
                        help=f"Passes per cell (default {DEFAULT_PASSES}).")
    parser.add_argument(
        "--jobs",
        type=_jobs_type,
        default=1,
        help="Maximum concurrent cell workers (default 1).",
    )
    parser.add_argument("--engine-game-port", type=lambda v: _port(v, "--engine-game-port"),
                        default=DEFAULT_ENGINE_GAME_PORT)
    parser.add_argument("--engine-http-port", type=lambda v: _port(v, "--engine-http-port"),
                        default=DEFAULT_ENGINE_HTTP_PORT)
    parser.add_argument("--quest-game-port", type=lambda v: _port(v, "--quest-game-port"),
                        default=DEFAULT_QUEST_GAME_PORT)
    parser.add_argument("--quest-http-port", type=lambda v: _port(v, "--quest-http-port"),
                        default=DEFAULT_QUEST_HTTP_PORT)
    parser.add_argument("--catalog-root", default=None,
                        help="Local rs2b0t checkout root. Required when a selected cell uses ${CATALOG_ROOT}.")
    parser.add_argument("--unpack-source", default=None,
                        help="Revision decoded unpack directory copied to panel-home/.274bot/unpack-289. Required when a selected cell uses ${PANEL_LIVE_HOME}.")
    parser.add_argument("--partner-prefix", default=None,
                        help="Second account's name prefix, 1-4 characters and different from --prefix. Required when a selected cell uses ${BOT_LIVE_PARTNER_NAME_PREFIX} (the BOT_LIVE_PARTNER_NAME_PREFIX variable is read when the flag is absent).")
    parser.add_argument("--self-test", action="store_true", help="Run the runner unit tests.")
    return parser


def parse_run_args(argv: list[str] | None = None) -> argparse.Namespace:
    return build_parser().parse_args(argv)


def is_quest_cell(cell_command: str) -> bool:
    return (
        "ENGINE_Q_DIR" in cell_command
        or re.search(rf"\bWORLD_GAME_PORT={DEFAULT_QUEST_GAME_PORT}\b", cell_command)
        is not None
        or re.search(rf"\bWORLD_HTTP_PORT={DEFAULT_QUEST_HTTP_PORT}\b", cell_command)
        is not None
        or re.search(rf"--port[ =]{DEFAULT_QUEST_GAME_PORT}\b", cell_command)
        is not None
        or re.search(rf"--http-port[ =]{DEFAULT_QUEST_HTTP_PORT}\b", cell_command)
        is not None
    )


def _explicit_command_port(command: str, variable: str, flag: str) -> int | None:
    match = re.search(rf"\b{variable}=(\d+)\b", command)
    if match is None:
        match = re.search(rf"--{flag}[ =](\d+)\b", command)
    return _port(match.group(1), variable) if match is not None else None


def cell_engine_ports(
    cell_command: str,
    engine_game_port: int,
    engine_http_port: int,
    quest_game_port: int,
    quest_http_port: int,
) -> tuple[bool, int, int]:
    quest = is_quest_cell(cell_command)
    game_port = quest_game_port if quest else engine_game_port
    http_port = quest_http_port if quest else engine_http_port
    if quest:
        game_port = _explicit_command_port(cell_command, "WORLD_GAME_PORT", "port") or game_port
        http_port = _explicit_command_port(cell_command, "WORLD_HTTP_PORT", "http-port") or http_port
    return quest, game_port, http_port


def reject_tick_override(cell_name: str, cell_command: str) -> None:
    if "QUESTER_TICK_MS=" not in cell_command:
        return
    raise live_map.MapError(
        f"run-cells: refusing cell {cell_name!r}: its command sets QUESTER_TICK_MS. "
        "Engine Q stays at 300 ms permanently. "
        "The quest campaign keeps that rate. "
        "::speed changes the world tick rate for every bot. "
        "Remove QUESTER_TICK_MS from the map command."
    )


def ensure_no_tick_override(cells: list[live_map.Cell]) -> None:
    for cell in cells:
        reject_tick_override(cell.name, cell.command)


def needs_catalog_root(cells: list[live_map.Cell]) -> bool:
    return any("CATALOG_ROOT" in cell.command for cell in cells)


def ensure_catalog_root(cells: list[live_map.Cell], catalog_root: str | None) -> str | None:
    if not needs_catalog_root(cells):
        return catalog_root
    catalog_root = catalog_root or os.environ.get("CATALOG_ROOT")
    if not catalog_root:
        names = ", ".join(cell.name for cell in cells if "CATALOG_ROOT" in cell.command)
        raise live_map.MapError(
            f"run-cells: refusing cells [{names}]: a selected cell uses ${{CATALOG_ROOT}}. "
            "Pass --catalog-root <path> or set CATALOG_ROOT in local lab.env."
        )
    if not Path(catalog_root).is_dir():
        raise live_map.MapError(
            f"run-cells: catalog root is not a directory: {catalog_root}"
        )
    return catalog_root


def needs_panel_cache(cells: list[live_map.Cell]) -> bool:
    return any("PANEL_LIVE_HOME" in cell.command for cell in cells)


def ensure_unpack_source(cells: list[live_map.Cell], unpack_source: str | None) -> str | None:
    if not needs_panel_cache(cells):
        return unpack_source
    unpack_source = unpack_source or os.environ.get("NAV_SNAPSHOT_ROOT_289")
    if not unpack_source:
        names = ", ".join(cell.name for cell in cells if "PANEL_LIVE_HOME" in cell.command)
        raise live_map.MapError(
            f"run-cells: refusing cells [{names}]: a selected cell stages panel-home/.274bot/unpack-289. "
            "Pass --unpack-source <path> or set NAV_SNAPSHOT_ROOT_289 in local lab.env."
        )
    if not Path(unpack_source).is_dir():
        raise live_map.MapError(
            f"run-cells: unpack source is not a directory: {unpack_source}"
        )
    return unpack_source


def needs_partner_prefix(cells: list[live_map.Cell]) -> bool:
    return any("BOT_LIVE_PARTNER_NAME_PREFIX" in cell.command for cell in cells)


def ensure_partner_prefix(
    cells: list[live_map.Cell], partner_prefix: str | None, prefix: str
) -> str | None:
    """The second account's name prefix, for cells that run two real accounts."""
    if not needs_partner_prefix(cells):
        return partner_prefix
    partner_prefix = partner_prefix or os.environ.get("BOT_LIVE_PARTNER_NAME_PREFIX")
    if not partner_prefix:
        names = ", ".join(
            cell.name for cell in cells if "BOT_LIVE_PARTNER_NAME_PREFIX" in cell.command
        )
        raise live_map.MapError(
            f"run-cells: refusing cells [{names}]: a selected cell runs two accounts. "
            "Pass --partner-prefix <1-4 characters>, different from --prefix."
        )
    if not 1 <= len(partner_prefix) <= 4:
        raise live_map.MapError(
            f"run-cells: the partner prefix must be 1-4 characters, got {partner_prefix!r}"
        )
    if partner_prefix == prefix:
        raise live_map.MapError("run-cells: the partner prefix must differ from --prefix")
    return partner_prefix


def hash_unpack_source(source: Path) -> str:
    digest = hashlib.sha256()
    names: list[str] = []
    for root, dirs, files in os.walk(source):
        dirs.sort()
        for name in sorted(files):
            full = Path(root) / name
            try:
                stat = full.stat()
            except OSError:
                continue
            relative = full.relative_to(source).as_posix()
            names.append(relative)
            digest.update(relative.encode("utf-8"))
            digest.update(str(stat.st_size).encode("utf-8"))
    digest.update(str(len(names)).encode("utf-8"))
    return digest.hexdigest()[:16]


def stage_panel_unpack(panel_home: Path, unpack_source: str) -> str:
    source = Path(unpack_source)
    if not source.is_dir():
        raise live_map.MapError(
            f"run-cells: --unpack-source is not a directory: {unpack_source}"
        )
    dest = panel_home / ".274bot" / "unpack-289"
    if dest.is_symlink() or dest.is_file():
        raise live_map.MapError(
            f"run-cells: refusing to stage over non-directory: {dest}"
        )
    if dest.is_dir() and any(dest.iterdir()):
        return f"{unpack_source}#{hash_unpack_source(source)}"
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, dest, symlinks=False, dirs_exist_ok=True)
    if dest.is_symlink():
        raise live_map.MapError(f"run-cells: staged unpack is a symlink: {dest}")
    return f"{unpack_source}#{hash_unpack_source(source)}"

_PORT_ENV_RE = re.compile(r"([A-Z][A-Z0-9_]*_GAME_PORT=)\d+")
_HTTP_ENV_RE = re.compile(r"([A-Z][A-Z0-9_]*_HTTP_PORT=)\d+")
_PORT_FLAG_RE = re.compile(r"(--port[ =])\d+")
_HTTP_FLAG_RE = re.compile(r"(--http-port[ =])\d+")


def apply_ports(command: str, game_port: int, http_port: int) -> str:
    command = _PORT_ENV_RE.sub(rf"\g<1>{game_port}", command)
    command = _HTTP_ENV_RE.sub(rf"\g<1>{http_port}", command)
    command = _PORT_FLAG_RE.sub(rf"\g<1>{game_port}", command)
    command = _HTTP_FLAG_RE.sub(rf"\g<1>{http_port}", command)
    return command


def _first_existing(candidates: list[str | None], want_file: bool) -> str | None:
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate)
        if not path.is_absolute():
            continue
        if want_file and path.is_file():
            return str(path)
        if not want_file and path.is_dir():
            return str(path)
    return None


def _local_data_nav(repo: Path, revision: str, name: str) -> str | None:
    """The pack or flags that `tools/local-data/build.py` wrote for a revision."""
    path = repo / "target" / "local-data" / "nav" / revision / name
    return str(path) if path.is_file() else None


def host_defaults(repo: Path) -> dict[str, str]:
    # The nav pack is the repository's own build output. The stored v17 and v18
    # reference packs are refused by the current reader.
    defaults: dict[str, str] = {}
    isohome = _first_existing([os.environ.get("ISOHOME")], want_file=True)
    if isohome:
        defaults["ISOHOME"] = isohome
    engine_a = _first_existing([os.environ.get("ENGINE_A_DIR")], want_file=False)
    if engine_a:
        defaults["ENGINE_A_DIR"] = engine_a
    engine_q = _first_existing([os.environ.get("ENGINE_Q_DIR")], want_file=False)
    if engine_q:
        defaults["ENGINE_Q_DIR"] = engine_q
    engine_b = _first_existing([os.environ.get("ENGINE_B_DIR")], want_file=False)
    if engine_b:
        defaults["ENGINE_B_DIR"] = engine_b
    elif engine_a:
        defaults["ENGINE_B_DIR"] = engine_a
    snapshot = _first_existing(
        [os.environ.get("NAV_SNAPSHOT_ROOT_289")],
        want_file=False,
    )
    if snapshot:
        defaults["NAV_SNAPSHOT_ROOT_289"] = snapshot
    for key, revision, name in (
        ("NAV_PACK_289", "289", "274bot.navpack"),
        ("NAV_FLAGS_289", "289", "274bot.navflags"),
        ("NAV_PACK_274", "274", "274bot.navpack"),
    ):
        found = _local_data_nav(repo, revision, name)
        if found:
            defaults[key] = found
    for key in (
        "NAV_PACK_289",
        "NAV_FLAGS_289",
        "NAV_PACK_274",
        "ENGINE_A_DIR",
        "ENGINE_Q_DIR",
    ):
        if key in os.environ and key not in defaults:
            defaults[key] = os.environ[key]
    return defaults

def require_isohome(cells: list[live_map.Cell]) -> None:
    if not any("${ISOHOME:?" in cell.command for cell in cells):
        return
    value = os.environ.get("ISOHOME")
    if not value:
        raise live_map.MapError(
            "ISOHOME is required by the selected live cells; set it in your local lab.env"
        )
    isohome = Path(value)
    if not isohome.is_absolute() or not isohome.is_file() or not os.access(isohome, os.X_OK):
        raise live_map.MapError(
            f"ISOHOME must name an absolute executable isohome wrapper: {isohome}"
        )


def require_configured_paths(cells: list[live_map.Cell]) -> None:
    required_directories = (
        "ENGINE_A_DIR",
        "ENGINE_Q_DIR",
        "ENGINE_B_DIR",
        "NAV_SNAPSHOT_ROOT_289",
    )
    for name in required_directories:
        if not any(
            f"${{{name}:?" in cell.command for cell in cells
        ):
            continue
        value = os.environ.get(name)
        if name == "ENGINE_B_DIR" and not value:
            value = os.environ.get("ENGINE_A_DIR")
        if not value:
            raise live_map.MapError(
                f"{name} is required by the selected live cells; set it in your local lab.env"
            )
        if not Path(value).is_dir():
            raise live_map.MapError(
                f"{name} must name an existing directory: {value}"
            )


def cell_names(config: dict) -> dict[str, live_map.Cell]:
    found: dict[str, live_map.Cell] = {}
    for index, row in enumerate(config["cover"]):
        for spec in row["cells"]:
            cell = live_map._cell_fields(spec, f"cover row {index + 1}")
            if cell.name in found and found[cell.name] != cell:
                raise live_map.MapError(f"cell {cell.name!r} has conflicting commands")
            found[cell.name] = cell
    return found


def resolve_cells(config: dict, names: list[str]) -> list[live_map.Cell]:
    registry = cell_names(config)
    missing = [name for name in names if name not in registry]
    if missing:
        raise live_map.MapError(f"unknown cells: {', '.join(missing)}")
    return [registry[name] for name in names]


def build_cell_env(
    *,
    cell_name: str,
    cell_command: str,
    prefix: str,
    live_evidence_dir: str,
    bot_cache_dir: str,
    client_unpack_dir: str,
    bot_unpack_dir: str,
    panel_home: str,
    repo: str,
    engine_game_port: int,
    engine_http_port: int,
    quest_game_port: int,
    quest_http_port: int,
    defaults: dict[str, str] | None = None,
    catalog_root: str | None = None,
    partner_prefix: str | None = None,
) -> tuple[dict[str, str], str]:
    reject_tick_override(cell_name, cell_command)
    quest, game_port, http_port = cell_engine_ports(
        cell_command,
        engine_game_port,
        engine_http_port,
        quest_game_port,
        quest_http_port,
    )
    command = apply_ports(cell_command, game_port, http_port)
    env = dict(os.environ)
    env.pop("QUESTER_TICK_MS", None)
    env.pop("QUESTER_SPEED_RESTORE", None)
    resolved = dict(defaults) if defaults else {}
    for key, value in resolved.items():
        if key not in env or not env[key]:
            env[key] = value
    if catalog_root is not None:
        env["CATALOG_ROOT"] = catalog_root
    env["BOT_LIVE_NAME_PREFIX"] = prefix
    if partner_prefix is not None:
        env["BOT_LIVE_PARTNER_NAME_PREFIX"] = partner_prefix
    env["BOT_NAV_BUILD"] = "skip"
    env["WORLD_GAME_PORT"] = str(game_port)
    env["WORLD_HTTP_PORT"] = str(http_port)
    env["LIVE_EVIDENCE_DIR"] = live_evidence_dir
    env["BOT_CACHE_DIR"] = bot_cache_dir
    env["CLIENT_UNPACK_DIR"] = client_unpack_dir
    env["BOT_UNPACK_DIR"] = bot_unpack_dir
    env["PANEL_LIVE_HOME"] = panel_home
    env["ISOHOME_DIR"] = str(Path(live_evidence_dir) / "home")
    env["PWD"] = str(Path(repo).resolve())
    if quest:
        env["QUESTER_SPEED_RESTORE"] = "0"
    env["CARGO_TERM_COLOR"] = "never"
    return env, command


def repo_head(repo: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise live_map.MapError(f"git rev-parse HEAD failed: {result.stderr.strip()}")
    return result.stdout.strip()


def ensure_clean(repo: Path) -> None:
    result = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain", "--untracked-files=no"],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise live_map.MapError(f"git status failed: {result.stderr.strip()}")
    if result.stdout.strip():
        raise live_map.MapError(
            f"refusing to run: {repo} has uncommitted tracked changes:\n{result.stdout.strip()}"
        )


def load_map_for_repo(repo: Path) -> dict:
    candidate = repo / "tools" / "live-map" / "map.toml"
    if candidate.is_file():
        return live_map.load_map(candidate)
    return live_map.load_map()

def load_upstream_manifest_for_repo(repo: Path) -> live_map.UpstreamManifest:
    return live_map.load_upstream_manifest(
        repo / "tools" / "live-map" / "upstream-cases.json"
    )


def upstream_cases_from_env(
    manifest: live_map.UpstreamManifest, raw_case_ids: str | None = None
) -> tuple[live_map.UpstreamCase, ...]:
    value = os.environ.get("UEP_CASES", "") if raw_case_ids is None else raw_case_ids
    case_ids = [part.strip() for part in value.split(",")]
    if not value.strip() or any(not case_id for case_id in case_ids):
        raise live_map.MapError(
            "run-cells: set UEP_CASES to one or more comma-separated adopted case ids"
        )
    return live_map.resolve_upstream_cases(manifest, case_ids)


def ensure_upstream_archive(
    archive_value: str | None, cases: tuple[live_map.UpstreamCase, ...]
) -> Path:
    if not archive_value:
        raise live_map.MapError(
            "run-cells: set UEP_UPSTREAM to the frozen upstream source archive"
        )
    archive = Path(archive_value)
    if not archive.is_absolute() or not archive.is_dir():
        raise live_map.MapError(
            f"run-cells: UEP_UPSTREAM must name an existing absolute directory: {archive}"
        )
    missing = [case.path for case in cases if not (archive / case.path).is_file()]
    if missing:
        raise live_map.MapError(
            f"run-cells: UEP_UPSTREAM is missing selected case files: {', '.join(missing)}"
        )
    for case in cases:
        if case.sha256 is None:
            continue
        try:
            actual_sha256 = hashlib.sha256(
                (archive / case.path).read_bytes()
            ).hexdigest()
        except OSError as error:
            raise live_map.MapError(
                f"run-cells: cannot read UEP_UPSTREAM case {case.id}: {error}"
            ) from error
        if actual_sha256 != case.sha256.lower():
            raise live_map.MapError(
                f"run-cells: UEP_UPSTREAM source hash does not match case {case.id}"
            )
    return archive.resolve()


def read_upstream_verdict(
    sink_path: Path,
    *,
    manifest: live_map.UpstreamManifest,
    cases: tuple[live_map.UpstreamCase, ...],
    upstream_archive: Path,
    host_commit: str,
) -> tuple[UpstreamCaseResult, ...]:
    prefix = f"invalid upstream verdict sink {sink_path}: "
    try:
        raw = json.loads(sink_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise live_map.MapError(f"{prefix}{error}") from error
    if not isinstance(raw, dict) or set(raw) != {
        "pin",
        "abi_pin",
        "host_commit",
        "results",
    }:
        raise live_map.MapError(f"{prefix}expected pin, abi_pin, host_commit, and results")
    if raw["pin"] != manifest.pin or raw["abi_pin"] != manifest.abi_pin:
        raise live_map.MapError(f"{prefix}source or ABI pin does not match metadata")
    if raw["host_commit"] != host_commit:
        raise live_map.MapError(f"{prefix}host_commit does not match the selected repository")
    if not isinstance(raw["results"], list):
        raise live_map.MapError(f"{prefix}results must be an array")

    expected = {case.id: case for case in cases}
    actual: dict[str, UpstreamCaseResult] = {}
    classifications = {
        "compat defect",
        "adapter defect",
        "upstream assumption",
        "environment",
    }
    for result in raw["results"]:
        if not isinstance(result, dict):
            raise live_map.MapError(f"{prefix}each result must be an object")
        case_id = result.get("id")
        if not isinstance(case_id, str) or case_id not in expected:
            raise live_map.MapError(f"{prefix}result has an unexpected case id")
        if case_id in actual:
            raise live_map.MapError(f"{prefix}duplicate result for {case_id}")
        case = expected[case_id]
        metadata = {
            "id": case.id,
            "path": case.path,
            "family": case.family,
            "capabilities": list(case.capabilities),
            "deadline_seconds": case.deadline_seconds,
            "gate_class": case.gate_class,
            "eligibility": case.eligibility,
            "family_paths": list(case.family_paths),
        }
        required_fields = set(metadata) | {
            "eligibility_reason",
            "reason",
            "verdict",
            "seconds",
            "sha256",
        }
        if set(result) not in (required_fields, required_fields | {"classification"}):
            raise live_map.MapError(f"{prefix}case {case_id} has unexpected result fields")
        if any(result.get(key) != value for key, value in metadata.items()):
            raise live_map.MapError(f"{prefix}metadata does not match case {case_id}")
        if result.get("eligibility_reason") != case.reason:
            raise live_map.MapError(
                f"{prefix}eligibility_reason does not match case {case_id}"
            )
        verdict = result.get("verdict")
        if verdict not in ("PASS", "FAIL", "UNSUPPORTED"):
            raise live_map.MapError(f"{prefix}case {case_id} has an invalid verdict")
        reason = result.get("reason")
        if not isinstance(reason, str) or (verdict != "PASS" and not reason.strip()):
            raise live_map.MapError(f"{prefix}case {case_id} has an invalid result reason")
        seconds = result.get("seconds")
        if isinstance(seconds, bool) or not isinstance(seconds, (int, float)):
            raise live_map.MapError(f"{prefix}case {case_id} has invalid seconds")
        try:
            seconds_value = float(seconds)
        except (OverflowError, ValueError):
            raise live_map.MapError(f"{prefix}case {case_id} has invalid seconds")
        if not math.isfinite(seconds_value) or seconds_value < 0:
            raise live_map.MapError(f"{prefix}case {case_id} has invalid seconds")
        sha256 = result.get("sha256")
        if not isinstance(sha256, str) or re.fullmatch(r"[0-9a-fA-F]{64}", sha256) is None:
            raise live_map.MapError(f"{prefix}case {case_id} has an invalid source SHA-256")
        try:
            source_sha256 = hashlib.sha256(
                (upstream_archive / case.path).read_bytes()
            ).hexdigest()
        except OSError as error:
            raise live_map.MapError(
                f"{prefix}cannot read selected source file for {case_id}: {error}"
            ) from error
        if sha256.lower() != source_sha256 or (
            case.sha256 is not None and sha256.lower() != case.sha256.lower()
        ):
            raise live_map.MapError(f"{prefix}source hash does not match case {case_id}")
        classification = result.get("classification")
        if verdict == "FAIL":
            if not isinstance(classification, str) or classification not in classifications:
                raise live_map.MapError(
                    f"{prefix}failed case {case_id} needs a supported classification"
                )
        elif "classification" in result:
            raise live_map.MapError(
                f"{prefix}case {case_id} classification is only valid for FAIL"
            )
        actual[case_id] = UpstreamCaseResult(
            id=case_id,
            verdict=verdict,
            gate_class=case.gate_class,
            reason=reason,
            seconds=seconds_value,
            classification=classification,
        )
    missing_ids = [case.id for case in cases if case.id not in actual]
    if missing_ids:
        raise live_map.MapError(
            f"{prefix}missing results for selected cases: {', '.join(missing_ids)}"
        )
    return tuple(actual[case.id] for case in cases)


def _utc_now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def run_one_cell(
    *,
    cell: live_map.Cell,
    repo: Path,
    env: dict[str, str],
    command: str,
    log_path: Path,
) -> tuple[int, str, str, int]:
    import time

    log_path.parent.mkdir(parents=True, exist_ok=True)
    start = _utc_now()
    begin = time.monotonic()
    with log_path.open("w", encoding="utf-8") as stream:
        result = subprocess.run(
            ["bash", "-lc", command],
            cwd=str(repo),
            env=env,
            stdout=stream,
            stderr=subprocess.STDOUT,
            check=False,
            text=True,
        )
    duration = int(time.monotonic() - begin)
    return result.returncode, start, _utc_now(), duration


def write_receipt(path: Path, fields: dict[str, str]) -> None:
    order = (
        "cell",
        "pass",
        "repo",
        "head",
        "prefix",
        "command",
        "start_utc",
        "end_utc",
        "duration_seconds",
        "exit_code",
        "log",
        "evidence_dir",
        "catalog_root",
        "isohome_dir",
        "unpack_source",
    )
    lines = [f"{key}={fields[key]}" for key in order if key in fields]
    for key in sorted(set(fields) - set(order)):
        lines.append(f"{key}={fields[key]}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def format_summary(
    results: list[RunResult],
    *,
    repo: str,
    head: str,
    prefix: str,
    evidence: str,
) -> str:
    lines = [
        "Live-cell run summary",
        f"repo={repo} head={head} prefix={prefix} evidence={evidence}",
        "",
        "| cell | pass | exit | seconds | start_utc | end_utc | log |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for item in results:
        lines.append(
            f"| {item.cell} | {item.passed_pass} | {item.exit_code} | "
            f"{item.duration_seconds} | {item.start_utc} | {item.end_utc} | "
            f"{item.log_path} |"
        )
    for item in results:
        lines.append("")
        lines.append(f"{item.cell} pass {item.passed_pass} command={item.command}")
        lines.append(f"{item.cell} pass {item.passed_pass} start_utc={item.start_utc}")
        lines.append(f"{item.cell} pass {item.passed_pass} end_utc={item.end_utc}")
        for case in item.upstream_results:
            reason = case.reason.replace("\n", " ").replace("\r", " ")
            classification = case.classification or ""
            lines.append(
                f"  upstream {case.id} verdict={case.verdict} "
                f"class={case.gate_class} seconds={case.seconds:g} "
                f"classification={classification} reason={reason}"
            )
    passed = sum(1 for item in results if item.exit_code == 0)
    lines.append("")
    lines.append(f"passed {passed}/{len(results)}")
    return "\n".join(lines)


def run_all(
    cells: list[live_map.Cell],
    *,
    repo: Path,
    evidence_root: Path,
    prefix: str,
    passes: int,
    engine_game_port: int,
    engine_http_port: int,
    quest_game_port: int,
    quest_http_port: int,
    defaults: dict[str, str] | None = None,
    catalog_root: str | None = None,
    unpack_source: str | None = None,
    partner_prefix: str | None = None,
    upstream_manifest: live_map.UpstreamManifest | None = None,
    upstream_cases: tuple[live_map.UpstreamCase, ...] = (),
    upstream_archive: Path | None = None,
    runner: Callable[..., tuple[int, str, str, int]] | None = None,
    jobs: int = 1,
) -> list[RunResult]:
    if jobs < 1:
        raise live_map.MapError("run-cells: jobs must be at least 1")
    if jobs > 1 and len({cell.name for cell in cells}) != len(cells):
        raise live_map.MapError("run-cells: parallel runs cannot contain duplicate cell names")
    has_upstream_runner = any(
        cell.name == live_map.UPSTREAM_RUNNER_CELL for cell in cells
    )
    upstream_ready = (
        upstream_manifest is not None
        and bool(upstream_cases)
        and upstream_archive is not None
    )
    if has_upstream_runner and runner is None and not upstream_ready:
        raise live_map.MapError(
            "run-cells: the upstream runner needs metadata, UEP_CASES, and UEP_UPSTREAM"
        )
    head = repo_head(repo)
    run_fn = runner or run_one_cell_by_args
    prefixes = _cell_prefixes(cells, prefix, partner_prefix, jobs)
    precompile_failures: dict[int, str] = {}

    if jobs > 1 and runner is None:
        precompile_jobs: dict[
            tuple[str, ...], tuple[live_map.Cell, dict[str, str], str]
        ] = {}
        cell_build_keys: dict[int, tuple[str, ...]] = {}
        for index, cell in enumerate(cells):
            cell_dir = evidence_root / cell.name
            pass_dir = cell_dir / "pass1"
            live_dir = pass_dir / "evidence"
            pass_cache_dir = live_dir / "cache-289"
            client_dir = cell_dir / "client-unpack"
            panel_home = pass_dir / "panel-home"
            env, command = build_cell_env(
                cell_name=cell.name,
                cell_command=cell.command,
                prefix=prefixes[index][0],
                live_evidence_dir=str(live_dir),
                bot_cache_dir=str(pass_cache_dir),
                client_unpack_dir=str(client_dir),
                bot_unpack_dir=str(client_dir),
                panel_home=str(panel_home),
                repo=str(repo),
                engine_game_port=engine_game_port,
                engine_http_port=engine_http_port,
                quest_game_port=quest_game_port,
                quest_http_port=quest_http_port,
                defaults=defaults,
                catalog_root=catalog_root,
                partner_prefix=prefixes[index][1],
            )
            precompile = _cargo_test_precompile(command)
            if precompile is None:
                continue
            key, precompile_command = precompile
            if cell.name == live_map.UPSTREAM_RUNNER_CELL:
                env.update(
                    {
                        "UEP_CASES": ",".join(case.id for case in upstream_cases),
                        "UEP_UPSTREAM": str(upstream_archive),
                        "UEP_OUTPUT": str(live_dir / "upstream-precompile"),
                        "UEP_HOST_COMMIT": head,
                    }
                )
            precompile_jobs.setdefault(key, (cell, env, precompile_command))
            cell_build_keys[index] = key

        for index, (key, (cell, env, command)) in enumerate(precompile_jobs.items(), 1):
            digest = hashlib.sha256(repr(key).encode("utf-8")).hexdigest()[:12]
            log_path = evidence_root / "precompile" / f"{digest}.log"
            log_path.parent.mkdir(parents=True, exist_ok=True)
            try:
                with log_path.open("w", encoding="utf-8") as stream:
                    result = subprocess.run(
                        ["bash", "-lc", command],
                        cwd=str(repo),
                        env=env,
                        stdout=stream,
                        stderr=subprocess.STDOUT,
                        check=False,
                        text=True,
                    )
                if result.returncode != 0:
                    message = _precompile_failure_message(
                        log_path, f"exit code {result.returncode}"
                    )
                    for cell_index, cell_key in cell_build_keys.items():
                        if cell_key == key:
                            precompile_failures[cell_index] = message
            except Exception as error:
                message = _precompile_failure_message(log_path, str(error))
                for cell_index, cell_key in cell_build_keys.items():
                    if cell_key == key:
                        precompile_failures[cell_index] = message

    def run_cell(index: int) -> list[RunResult]:
        cell = cells[index]
        cell_prefix, cell_partner_prefix = prefixes[index]
        cell_dir = evidence_root / cell.name
        cache_dir = cell_dir / "cache-289"
        client_dir = cell_dir / "client-unpack"
        cache_dir.mkdir(parents=True, exist_ok=True)
        client_dir.mkdir(parents=True, exist_ok=True)
        cell_passes = 1 if cell.name == live_map.UPSTREAM_RUNNER_CELL else passes
        cell_results: list[RunResult] = []
        for passed_pass in range(1, cell_passes + 1):
            pass_dir = cell_dir / f"pass{passed_pass}"
            live_dir = pass_dir / "evidence"
            panel_home = pass_dir / "panel-home"
            pass_cache_dir = live_dir / "cache-289"
            log_path = pass_dir / "cell.log"
            receipt_path = pass_dir / "receipt.txt"
            upstream_output: Path | None = None
            unpack_record: str | None = None
            upstream_results: tuple[UpstreamCaseResult, ...] = ()
            env: dict[str, str] = {}
            command = cell.command
            start = _utc_now()
            try:
                live_dir.mkdir(parents=True, exist_ok=True)
                panel_home.mkdir(parents=True, exist_ok=True)
                (live_dir / "tui").mkdir(parents=True, exist_ok=True)
                (live_dir / "home").mkdir(parents=True, exist_ok=True)
                shutil.copytree(cache_dir, pass_cache_dir, symlinks=False)
                if unpack_source is not None and "PANEL_LIVE_HOME" in cell.command:
                    unpack_record = stage_panel_unpack(panel_home, unpack_source)
                env, command = build_cell_env(
                    cell_name=cell.name,
                    cell_command=cell.command,
                    prefix=cell_prefix,
                    live_evidence_dir=str(live_dir),
                    bot_cache_dir=str(pass_cache_dir),
                    client_unpack_dir=str(client_dir),
                    bot_unpack_dir=str(client_dir),
                    panel_home=str(panel_home),
                    repo=str(repo),
                    engine_game_port=engine_game_port,
                    engine_http_port=engine_http_port,
                    quest_game_port=quest_game_port,
                    quest_http_port=quest_http_port,
                    defaults=defaults,
                    catalog_root=catalog_root,
                    partner_prefix=cell_partner_prefix,
                )
                if cell.name == live_map.UPSTREAM_RUNNER_CELL and upstream_ready:
                    if upstream_manifest is None or upstream_archive is None:
                        raise live_map.MapError(
                            "run-cells: upstream runner inputs were not resolved"
                        )
                    upstream_output = Path(
                        tempfile.mkdtemp(prefix="upstream-", dir=live_dir)
                    )
                    env.update(
                        {
                            "UEP_CASES": ",".join(case.id for case in upstream_cases),
                            "UEP_UPSTREAM": str(upstream_archive),
                            "UEP_OUTPUT": str(upstream_output.resolve()),
                            "UEP_HOST_COMMIT": head,
                        }
                    )
                if index in precompile_failures:
                    raise RuntimeError(precompile_failures[index])
                exit_code, start, end, duration = run_fn(
                    cell=cell,
                    repo=repo,
                    env=env,
                    command=command,
                    log_path=log_path,
                )
            except Exception as error:  # Keep failures local to this pass.
                exit_code, end, duration = 1, _utc_now(), 0
                log_path.parent.mkdir(parents=True, exist_ok=True)
                with log_path.open("a", encoding="utf-8") as stream:
                    stream.write(f"\nrunner error: {error}\n")
            if cell.name == live_map.UPSTREAM_RUNNER_CELL and upstream_ready:
                try:
                    if (
                        upstream_output is None
                        or upstream_manifest is None
                        or upstream_archive is None
                    ):
                        raise live_map.MapError(
                            "run-cells: upstream verdict sink inputs were not configured"
                        )
                    upstream_results = read_upstream_verdict(
                        upstream_output / "verdict.json",
                        manifest=upstream_manifest,
                        cases=upstream_cases,
                        upstream_archive=upstream_archive,
                        host_commit=head,
                    )
                    if exit_code == 0 and any(
                        case.gate_class == "gating" and case.verdict != "PASS"
                        for case in upstream_results
                    ):
                        exit_code = 1
                except live_map.MapError as error:
                    exit_code = 1
                    with log_path.open("a", encoding="utf-8") as stream:
                        stream.write(f"\nverdict sink error: {error}\n")
            receipt_fields: dict[str, str] = {
                "cell": cell.name,
                "pass": str(passed_pass),
                "repo": str(repo),
                "head": head,
                "prefix": cell_prefix,
                "command": command,
                "start_utc": start,
                "end_utc": end,
                "duration_seconds": str(duration),
                "exit_code": str(exit_code),
                "log": str(log_path),
                "evidence_dir": str(live_dir),
                "isohome_dir": env.get("ISOHOME_DIR", ""),
            }
            if "BOT_LIVE_PARTNER_NAME_PREFIX" in cell.command:
                actual_partner_prefix = env.get("BOT_LIVE_PARTNER_NAME_PREFIX")
                if actual_partner_prefix is not None:
                    receipt_fields["partner_prefix"] = actual_partner_prefix
            if catalog_root is not None:
                receipt_fields["catalog_root"] = catalog_root
            if unpack_record is not None:
                receipt_fields["unpack_source"] = unpack_record
            if upstream_output is not None:
                receipt_fields["verdict_sink"] = str(upstream_output / "verdict.json")
            write_receipt(receipt_path, receipt_fields)
            cell_results.append(
                RunResult(
                    cell=cell.name,
                    passed_pass=passed_pass,
                    exit_code=exit_code,
                    duration_seconds=duration,
                    log_path=log_path,
                    receipt_path=receipt_path,
                    command=command,
                    start_utc=start,
                    end_utc=end,
                    upstream_results=upstream_results,
                )
            )
        return cell_results

    results_by_index: list[list[RunResult] | None] = [None] * len(cells)
    if jobs == 1:
        for index in range(len(cells)):
            results_by_index[index] = run_cell(index)
    else:
        gate = _CellExecutionGate()
        order = sorted(range(len(cells)), key=lambda index: (_cell_priority(cells[index]), index))
        executor = concurrent.futures.ThreadPoolExecutor(max_workers=jobs)
        futures: dict[concurrent.futures.Future[list[RunResult]], int] = {}
        try:
            for index in order:
                future = executor.submit(
                    gate.run,
                    lambda index=index: run_cell(index),
                    exclusive=is_exclusive_cell(cells[index]),
                )
                futures[future] = index
            for future in concurrent.futures.as_completed(futures):
                index = futures[future]
                try:
                    results_by_index[index] = future.result()
                except Exception as error:
                    # A failed worker must not prevent the other futures from completing.
                    cell = cells[index]
                    cell_passes = (
                        1 if cell.name == live_map.UPSTREAM_RUNNER_CELL else passes
                    )
                    failed_results = []
                    for passed_pass in range(1, cell_passes + 1):
                        pass_dir = evidence_root / cell.name / f"pass{passed_pass}"
                        log_path = pass_dir / "cell.log"
                        receipt_path = pass_dir / "receipt.txt"
                        start = _utc_now()
                        end = _utc_now()
                        try:
                            log_path.parent.mkdir(parents=True, exist_ok=True)
                            log_path.write_text(
                                f"worker error: {error}\n", encoding="utf-8"
                            )
                            write_receipt(
                                receipt_path,
                                {
                                    "cell": cell.name,
                                    "pass": str(passed_pass),
                                    "repo": str(repo),
                                    "head": head,
                                    "prefix": prefixes[index][0],
                                    "command": cell.command,
                                    "start_utc": start,
                                    "end_utc": end,
                                    "duration_seconds": "0",
                                    "exit_code": "1",
                                    "log": str(log_path),
                                    "evidence_dir": str(pass_dir / "evidence"),
                                },
                            )
                        except Exception as artifact_error:
                            print(
                                "run-cells: cannot write worker failure artifacts "
                                f"for {cell.name}: {artifact_error}",
                                file=sys.stderr,
                            )
                        failed_results.append(
                            RunResult(
                                cell=cell.name,
                                passed_pass=passed_pass,
                                exit_code=1,
                                duration_seconds=0,
                                log_path=log_path,
                                receipt_path=receipt_path,
                                command=cell.command,
                                start_utc=start,
                                end_utc=end,
                            )
                        )
                    results_by_index[index] = failed_results
        except BaseException:
            gate.cancel()
            executor.shutdown(wait=True, cancel_futures=True)
            raise
        else:
            executor.shutdown(wait=True)
    return [
        result
        for cell_results in results_by_index
        if cell_results is not None
        for result in cell_results
    ]


def run_one_cell_by_args(**kwargs) -> tuple[int, str, str, int]:  # pragma: no cover
    return run_one_cell(**kwargs)


def main(argv: list[str] | None = None) -> int:
    raw = list(argv) if argv is not None else sys.argv[1:]
    if raw == ["--self-test"] or (len(raw) == 1 and raw[0] == "--self-test"):
        return self_test()
    args = parse_run_args(argv)
    repo = Path(args.repo)
    if not repo.is_dir():
        print(f"run-cells: --repo is not a directory: {repo}", file=sys.stderr)
        return 2
    evidence_root = Path(args.evidence)
    try:
        ensure_clean(repo)
        config = load_map_for_repo(repo)
        upstream_manifest: live_map.UpstreamManifest | None = None
        upstream_cases: tuple[live_map.UpstreamCase, ...] = ()
        upstream_archive: Path | None = None
        if args.cells:
            names = [part.strip() for part in args.cells.split(",") if part.strip()]
            if not names:
                print("run-cells: --cells is empty", file=sys.stderr)
                return 2
            cells = resolve_cells(config, names)
            if any(cell.name == live_map.UPSTREAM_RUNNER_CELL for cell in cells):
                upstream_manifest = load_upstream_manifest_for_repo(repo)
                upstream_cases = upstream_cases_from_env(upstream_manifest)
                upstream_archive = ensure_upstream_archive(
                    os.environ.get("UEP_UPSTREAM"), upstream_cases
                )
        else:
            paths = live_map.diff_paths(args.diff, repo)
            upstream_manifest = load_upstream_manifest_for_repo(repo)
            plan = live_map.make_plan(paths, config, upstream_manifest)
            cells = list(plan.cells)
            upstream_cases = plan.upstream_cases
            if upstream_cases:
                upstream_archive = ensure_upstream_archive(
                    os.environ.get("UEP_UPSTREAM"), upstream_cases
                )
            if plan.uncovered or plan.unclassified:
                print(
                    "run-cells: diff has uncovered or unclassified paths; "
                    "running covering cells only",
                    file=sys.stderr,
                )
            if not cells:
                print("run-cells: diff selects no cells", file=sys.stderr)
                return 1
        ensure_no_tick_override(cells)
        catalog_root = ensure_catalog_root(cells, args.catalog_root)
        unpack_source = ensure_unpack_source(cells, args.unpack_source)
        partner_prefix = ensure_partner_prefix(cells, args.partner_prefix, args.prefix)
        require_isohome(cells)
        require_configured_paths(cells)

        evidence_root.mkdir(parents=True, exist_ok=True)
        defaults = host_defaults(repo)
        results = run_all(
            cells,
            repo=repo,
            evidence_root=evidence_root,
            prefix=args.prefix,
            passes=args.passes,
            jobs=args.jobs,
            engine_game_port=args.engine_game_port,
            engine_http_port=args.engine_http_port,
            quest_game_port=args.quest_game_port,
            quest_http_port=args.quest_http_port,
            defaults=defaults,
            catalog_root=catalog_root,
            unpack_source=unpack_source,
            partner_prefix=partner_prefix,
            upstream_manifest=upstream_manifest,
            upstream_cases=upstream_cases,
            upstream_archive=upstream_archive,
        )
        head = repo_head(repo)
        summary = format_summary(
            results,
            repo=str(repo),
            head=head,
            prefix=args.prefix,
            evidence=str(evidence_root),
        )
        print(summary)
        (evidence_root / "summary.txt").write_text(summary + "\n", encoding="utf-8")
        return 0 if all(item.exit_code == 0 for item in results) else 1
    except live_map.MapError as error:
        print(f"run-cells: {error}", file=sys.stderr)
        return 2


class RunCellsTests(unittest.TestCase):

    def upstream_sink_fixture(self):
        case = live_map.UpstreamCase(
            id="gather-one",
            path="e2e/gather-one-live.ts",
            family="gather",
            capabilities=("inventory",),
            deadline_seconds=60,
            gate_class="gating",
            eligibility="adopted",
            reason="",
            family_paths=("crates/script-js/src/compat/gather/**",),
        )
        manifest = live_map.UpstreamManifest(
            pin="a" * 40,
            abi_pin="b" * 40,
            cases=(case,),
        )
        host_commit = "c" * 40
        payload = {
            "pin": manifest.pin,
            "abi_pin": manifest.abi_pin,
            "host_commit": host_commit,
            "results": [
                {
                    "id": case.id,
                    "path": case.path,
                    "family": case.family,
                    "capabilities": list(case.capabilities),
                    "deadline_seconds": case.deadline_seconds,
                    "gate_class": case.gate_class,
                    "family_paths": list(case.family_paths),
                    "eligibility": case.eligibility,
                    "eligibility_reason": case.reason,
                    "verdict": "PASS",
                    "reason": "",
                    "seconds": 1.25,
                    "sha256": hashlib.sha256(b"source").hexdigest(),
                }
            ],
        }
        return manifest, (case,), host_commit, payload

    def write_upstream_archive(self, parent: Path) -> Path:
        archive = parent / "upstream"
        source = archive / "e2e" / "gather-one-live.ts"
        source.parent.mkdir(parents=True)
        source.write_bytes(b"source")
        return archive

    def test_upstream_verdict_reads_and_checks_sink_fields(self) -> None:
        manifest, cases, host_commit, payload = self.upstream_sink_fixture()
        with tempfile.TemporaryDirectory(prefix="run-cells-verdict-") as temp:
            archive = self.write_upstream_archive(Path(temp))
            sink = Path(temp) / "verdict.json"
            sink.write_text(json.dumps(payload), encoding="utf-8")
            results = read_upstream_verdict(
                sink,
                manifest=manifest,
                cases=cases,
                host_commit=host_commit,
                upstream_archive=archive,
            )
        self.assertEqual(
            results,
            (UpstreamCaseResult("gather-one", "PASS", "gating", "", 1.25, None),),
        )

    def test_upstream_verdict_preserves_runtime_reason(self) -> None:
        manifest, cases, host_commit, payload = self.upstream_sink_fixture()
        result = payload["results"][0]
        result["verdict"] = "FAIL"
        result["reason"] = "the observed value did not match"
        result["classification"] = "compat defect"
        with tempfile.TemporaryDirectory(prefix="run-cells-verdict-") as temp:
            archive = self.write_upstream_archive(Path(temp))
            sink = Path(temp) / "verdict.json"
            sink.write_text(json.dumps(payload), encoding="utf-8")
            results = read_upstream_verdict(
                sink,
                manifest=manifest,
                cases=cases,
                host_commit=host_commit,
                upstream_archive=archive,
            )
        self.assertEqual(results[0].reason, "the observed value did not match")
        self.assertEqual(results[0].classification, "compat defect")

    def test_upstream_verdict_fails_closed_on_sink_errors(self) -> None:
        manifest, cases, host_commit, payload = self.upstream_sink_fixture()
        with tempfile.TemporaryDirectory(prefix="run-cells-verdict-") as temp:
            archive = self.write_upstream_archive(Path(temp))
            sink = Path(temp) / "verdict.json"
            with self.assertRaisesRegex(live_map.MapError, "invalid upstream verdict sink"):
                read_upstream_verdict(
                    sink,
                    manifest=manifest,
                    cases=cases,
                    upstream_archive=archive,
                    host_commit=host_commit,
                )
            sink.write_text("{", encoding="utf-8")
            with self.assertRaisesRegex(live_map.MapError, "invalid upstream verdict sink"):
                read_upstream_verdict(
                    sink,
                    manifest=manifest,
                    cases=cases,
                    upstream_archive=archive,
                    host_commit=host_commit,
                )
            malformed = json.loads(json.dumps(payload))
            malformed["host_commit"] = "e" * 40
            sink.write_text(json.dumps(malformed), encoding="utf-8")
            with self.assertRaisesRegex(live_map.MapError, "host_commit"):
                read_upstream_verdict(
                    sink,
                    manifest=manifest,
                    cases=cases,
                    upstream_archive=archive,
                    host_commit=host_commit,
                )

            malformed = json.loads(json.dumps(payload))
            malformed["results"][0]["unexpected"] = True
            sink.write_text(json.dumps(malformed), encoding="utf-8")
            with self.assertRaisesRegex(live_map.MapError, "unexpected result fields"):
                read_upstream_verdict(
                    sink,
                    manifest=manifest,
                    cases=cases,
                    host_commit=host_commit,
                    upstream_archive=archive,
                )
            malformed = json.loads(json.dumps(payload))
            malformed["results"][0]["verdict"] = "FAIL"
            malformed["results"][0]["reason"] = "host returned the wrong value"
            sink.write_text(json.dumps(malformed), encoding="utf-8")
            with self.assertRaisesRegex(live_map.MapError, "classification"):
                read_upstream_verdict(
                    sink,
                    manifest=manifest,
                    cases=cases,
                    host_commit=host_commit,
                    upstream_archive=archive,
                )

            malformed = json.loads(json.dumps(payload))
            malformed["results"][0]["sha256"] = "e" * 64
            sink.write_text(json.dumps(malformed), encoding="utf-8")
            with self.assertRaisesRegex(live_map.MapError, "source hash"):
                read_upstream_verdict(
                    sink,
                    manifest=manifest,
                    cases=cases,
                    upstream_archive=archive,
                    host_commit=host_commit,
                )


    def test_cells_and_diff_are_exclusive(self) -> None:
        with self.assertRaises(SystemExit):
            parse_run_args(
                [
                    "--repo",
                    "/tmp/repo",
                    "--cells",
                    "a",
                    "--diff",
                    "b..h",
                    "--prefix",
                    "abc",
                    "--evidence",
                    "/tmp/e",
                ]
            )

    def test_missing_prefix_is_rejected(self) -> None:
        with self.assertRaises(SystemExit):
            parse_run_args(
                ["--repo", "/tmp/repo", "--cells", "a", "--evidence", "/tmp/e"]
            )

    def test_prefix_length_is_checked(self) -> None:
        with self.assertRaises(SystemExit):
            parse_run_args(
                [
                    "--repo",
                    "/tmp/repo",
                    "--cells",
                    "a",
                    "--prefix",
                    "abcde",
                    "--evidence",
                    "/tmp/e",
                ]
            )
        self.assertEqual(
            parse_run_args(
                [
                    "--repo",
                    "/tmp/repo",
                    "--cells",
                    "a",
                    "--prefix",
                    "abc",
                    "--evidence",
                    "/tmp/e",
                ]
            ).prefix,
            "abc",
        )

    def test_passes_and_ports_have_defaults(self) -> None:
        args = parse_run_args(
            [
                "--repo",
                "/tmp/repo",
                "--cells",
                "a",
                "--prefix",
                "abc",
                "--evidence",
                "/tmp/e",
            ]
        )
        self.assertEqual(args.passes, DEFAULT_PASSES)
        self.assertEqual(args.engine_game_port, DEFAULT_ENGINE_GAME_PORT)
        self.assertEqual(args.engine_http_port, DEFAULT_ENGINE_HTTP_PORT)
        self.assertEqual(args.quest_game_port, DEFAULT_QUEST_GAME_PORT)
        self.assertEqual(args.quest_http_port, DEFAULT_QUEST_HTTP_PORT)
        self.assertIsNone(args.catalog_root)
        self.assertIsNone(args.unpack_source)

    def test_new_paths_parse(self) -> None:
        args = parse_run_args(
            [
                "--repo",
                "/tmp/repo",
                "--cells",
                "a",
                "--prefix",
                "abc",
                "--evidence",
                "/tmp/e",
                "--catalog-root",
                "/tmp/catalog",
                "--unpack-source",
                "/tmp/unpack-289",
            ]
        )
        self.assertEqual(args.catalog_root, "/tmp/catalog")
        self.assertEqual(args.unpack_source, "/tmp/unpack-289")

    def test_env_sets_common_values_and_unsets_tick(self) -> None:
        with unittest.mock.patch.dict(
            os.environ,
            {
                "QUESTER_TICK_MS": "300",
                "QUESTER_SPEED_RESTORE": "1",
                "BOT_LIVE_PARTNER_NAME_PREFIX": "def",
                "NIGHTLY_QUIET_LOCK": "/tmp/MAC-QUIET.lock",
            },
        ):
            env, _ = build_cell_env(
                cell_name="host.sherlock-search-live",
                cell_command="LIVE=1 WORLD_GAME_PORT=44594 ${ISOHOME:?} cargo test",
                prefix="abc",
                live_evidence_dir="/e/cell/pass1/evidence",
                bot_cache_dir="/e/cell/cache-289",
                client_unpack_dir="/e/cell/client-unpack",
                bot_unpack_dir="/e/cell/client-unpack",
                panel_home="/e/cell/pass1/panel-home",
                repo="/tmp/repo",
                engine_game_port=47594,
                engine_http_port=3080,
                quest_game_port=47694,
                quest_http_port=3180,
                defaults={"ISOHOME": "/isohome"},
            )
        self.assertEqual(env["BOT_LIVE_NAME_PREFIX"], "abc")
        self.assertEqual(env["BOT_LIVE_PARTNER_NAME_PREFIX"], "def")
        self.assertEqual(env["BOT_NAV_BUILD"], "skip")
        self.assertEqual(env["WORLD_GAME_PORT"], "47594")
        self.assertEqual(env["WORLD_HTTP_PORT"], "3080")
        self.assertNotIn("QUESTER_TICK_MS", env)
        self.assertNotIn("QUESTER_SPEED_RESTORE", env)
        self.assertEqual(env["NIGHTLY_QUIET_LOCK"], "/tmp/MAC-QUIET.lock")

    def test_engine_q_marker_uses_quest_ports_without_cell_id(self) -> None:
        env, command = build_cell_env(
            cell_name="host.not-a-quester-name",
            cell_command="LIVE=1 ENGINE_Q_DIR=${ENGINE_Q_DIR:?} ${ISOHOME:?} cargo test",
            prefix="abc",
            live_evidence_dir="/e/cell/pass1/evidence",
            bot_cache_dir="/e/cell/cache-289",
            client_unpack_dir="/e/cell/client-unpack",
            bot_unpack_dir="/e/cell/client-unpack",
            panel_home="/e/cell/pass1/panel-home",
            repo="/tmp/repo",
            engine_game_port=47594,
            engine_http_port=3080,
            quest_game_port=47694,
            quest_http_port=3180,
            defaults={},
        )
        self.assertEqual(env["WORLD_GAME_PORT"], "47694")
        self.assertEqual(env["WORLD_HTTP_PORT"], "3180")
        self.assertEqual(env["QUESTER_SPEED_RESTORE"], "0")
        self.assertNotIn("QUESTER_TICK_MS", env)
        self.assertNotIn("44594", command)
        self.assertNotIn("44694", command)

    def test_prereq_engine_q_ports_override_runner_ports(self) -> None:
        env, command = build_cell_env(
            cell_name="host.prereq-phoenix-kitchen",
            cell_command=(
                "LIVE=1 WORLD_GAME_PORT=44694 WORLD_HTTP_PORT=1180 "
                "${ISOHOME:?} cargo test -- --port 44694 --http-port 1180"
            ),
            prefix="abc",
            live_evidence_dir="/e/cell/pass1/evidence",
            bot_cache_dir="/e/cell/cache-289",
            client_unpack_dir="/e/cell/client-unpack",
            bot_unpack_dir="/e/cell/client-unpack",
            panel_home="/e/cell/pass1/panel-home",
            repo="/tmp/repo",
            engine_game_port=47594,
            engine_http_port=3080,
            quest_game_port=47694,
            quest_http_port=3180,
            defaults={},
        )
        self.assertEqual(env["WORLD_GAME_PORT"], "44694")
        self.assertEqual(env["WORLD_HTTP_PORT"], "1180")
        self.assertIn("WORLD_GAME_PORT=44694", command)
        self.assertIn("WORLD_HTTP_PORT=1180", command)
        self.assertIn("--port 44694", command)
        self.assertIn("--http-port 1180", command)
        self.assertNotIn("47594", command)
        self.assertNotIn("3080", command)
        self.assertNotIn("47694", command)
        self.assertNotIn("3180", command)

    def test_quester_in_cell_name_does_not_select_engine_q(self) -> None:
        env, _ = build_cell_env(
            cell_name="host.quester-name-engine-a",
            cell_command="LIVE=1 ${ISOHOME:?} cargo test",
            prefix="abc",
            live_evidence_dir="/e/cell/pass1/evidence",
            bot_cache_dir="/e/cell/cache-289",
            client_unpack_dir="/e/cell/client-unpack",
            bot_unpack_dir="/e/cell/client-unpack",
            panel_home="/e/cell/pass1/panel-home",
            repo="/tmp/repo",
            engine_game_port=47594,
            engine_http_port=3080,
            quest_game_port=47694,
            quest_http_port=3180,
            defaults={},
        )
        self.assertEqual(env["WORLD_GAME_PORT"], "47594")
        self.assertEqual(env["WORLD_HTTP_PORT"], "3080")
        self.assertNotIn("QUESTER_SPEED_RESTORE", env)

    def test_never_stop_loop_continues_after_failure(self) -> None:
        cells = [
            live_map.Cell("first", "true", "--locked -p a --lib"),
            live_map.Cell("second", "true", "--locked -p b --lib"),
        ]
        calls: list[str] = []

        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            cell = kwargs["cell"]
            calls.append(cell.name)
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text(f"{cell.name}\n", encoding="utf-8")
            if cell.name == "first":
                return 3, "s", "e", 0
            return 0, "s", "e", 0

        import tempfile

        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp) / "repo"
            repo.mkdir()
            evidence = Path(temp) / "evidence"
            with unittest.mock.patch(
                f"{__name__}.repo_head", return_value="deadbeef"
            ):
                results = run_all(
                    cells,
                    repo=repo,
                    evidence_root=evidence,
                    prefix="abc",
                    passes=1,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    runner=fake_runner,
                )
                self.assertEqual(calls, ["first", "second"])
                self.assertEqual([item.exit_code for item in results], [3, 0])
                self.assertTrue((evidence / "first" / "pass1" / "receipt.txt").is_file())
                self.assertTrue((evidence / "second" / "pass1" / "receipt.txt").is_file())

    def test_each_pass_copies_cache_inside_its_evidence(self) -> None:
        import tempfile

        cell = live_map.Cell(
            "host.fixture-cache",
            "LIVE=1 BOT_EVIDENCE_DIR=${LIVE_EVIDENCE_DIR:?} true",
            "--locked",
        )
        seen_caches: list[Path] = []

        def observe_cache(**kwargs) -> tuple[int, str, str, int]:
            env = kwargs["env"]
            cache = Path(env["BOT_CACHE_DIR"])
            self.assertEqual(cache.parent.resolve(), Path(env["LIVE_EVIDENCE_DIR"]).resolve())
            self.assertFalse(cache.is_symlink())
            data = cache / "nested" / "data.bin"
            self.assertFalse(data.is_symlink())
            self.assertEqual(data.read_bytes(), b"seed")
            data.write_bytes(b"changed in this pass")
            seen_caches.append(cache)
            kwargs["log_path"].write_text("ok\n", encoding="utf-8")
            return 0, "s", "e", 0

        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp) / "repo"
            repo.mkdir()
            evidence = Path(temp) / "evidence"
            seed_cache = evidence / cell.name / "cache-289"
            (seed_cache / "nested").mkdir(parents=True)
            seed_data = seed_cache / "nested" / "data.bin"
            seed_data.write_bytes(b"seed")
            with unittest.mock.patch(f"{__name__}.repo_head", return_value="deadbeef"):
                results = run_all(
                    [cell],
                    repo=repo,
                    evidence_root=evidence,
                    prefix="abc",
                    passes=2,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    runner=observe_cache,
                )
            self.assertEqual([item.exit_code for item in results], [0, 0])
            self.assertEqual(
                seen_caches,
                [
                    evidence / cell.name / "pass1" / "evidence" / "cache-289",
                    evidence / cell.name / "pass2" / "evidence" / "cache-289",
                ],
            )
            self.assertEqual(seed_data.read_bytes(), b"seed")

    def test_unknown_cell_is_rejected(self) -> None:
        config = live_map.validate_map(
            {
                "live": {"globs": ["src/**"]},
                "not_live": {"globs": ["docs/**"]},
                "cover": [
                    {
                        "globs": ["src/**"],
                        "cells": ["a|cargo test --locked -p a --lib"],
                    }
                ],
            }
        )
        with self.assertRaises(live_map.MapError):
            resolve_cells(config, ["missing"])

    def test_tick_override_in_command_is_refused(self) -> None:
        with self.assertRaises(live_map.MapError) as context:
            build_cell_env(
                cell_name="host.quester-path-engine-q",
                cell_command=(
                    "LIVE=1 QUESTER_TICK_MS=300 WORLD_GAME_PORT=44694 "
                    "WORLD_HTTP_PORT=1180 ${ISOHOME:?} cargo test"
                ),
                prefix="abc",
                live_evidence_dir="/e/cell/pass1/evidence",
                bot_cache_dir="/e/cell/cache-289",
                client_unpack_dir="/e/cell/client-unpack",
                bot_unpack_dir="/e/cell/client-unpack",
                panel_home="/e/cell/pass1/panel-home",
                repo="/tmp/repo",
                engine_game_port=47594,
                engine_http_port=3080,
                quest_game_port=47694,
                quest_http_port=3180,
                defaults={},
            )
        self.assertIn("QUESTER_TICK_MS", str(context.exception))
        self.assertIn("host.quester-path-engine-q", str(context.exception))

    def test_ensure_no_tick_override_rejects_a_list(self) -> None:
        bad = [live_map.Cell("quest", "LIVE=1 QUESTER_TICK_MS=300 cargo test", "")]
        with self.assertRaises(live_map.MapError):
            ensure_no_tick_override(bad)
        good = [live_map.Cell("quest", "LIVE=1 cargo test", "")]
        ensure_no_tick_override(good)

    def test_map_sets_no_tick_override(self) -> None:
        text = (RUNNER_DIR / "map.toml").read_text(encoding="utf-8")
        self.assertNotIn("QUESTER_TICK_MS=", text)

    def test_summary_lists_command_and_times(self) -> None:
        results = [
            RunResult(
                cell="host.example-live",
                passed_pass=1,
                exit_code=0,
                duration_seconds=7,
                log_path=Path("/e/host.example-live/pass1/cell.log"),
                receipt_path=Path("/e/host.example-live/pass1/receipt.txt"),
                command="LIVE=1 ${ISOHOME:?} cargo test",
                start_utc="2026-10-29T10:00:00Z",
                end_utc="2026-10-29T10:00:07Z",
            )
        ]
        summary = format_summary(
            results, repo="/repo", head="abc123", prefix="t", evidence="/e"
        )
        self.assertIn("LIVE=1 ${ISOHOME:?} cargo test", summary)
        self.assertIn("2026-10-29T10:00:00Z", summary)
        self.assertIn("2026-10-29T10:00:07Z", summary)

    def test_catalog_root_is_required_for_catalog_cells(self) -> None:
        cells = [live_map.Cell("nav.route-search-live", "LIVE=1 RS2B0T=${CATALOG_ROOT:?} cargo test", "")]
        self.assertTrue(needs_catalog_root(cells))
        with unittest.mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(live_map.MapError) as context:
                ensure_catalog_root(cells, None)
        self.assertIn("--catalog-root", str(context.exception))
        self.assertIn("nav.route-search-live", str(context.exception))
        clean = [live_map.Cell("plain", "LIVE=1 ${ISOHOME:?} cargo test", "")]
        self.assertFalse(needs_catalog_root(clean))
        self.assertIsNone(ensure_catalog_root(clean, None))

    def test_catalog_root_missing_dir_is_refused(self) -> None:
        cells = [live_map.Cell("nav.route-search-live", "LIVE=1 ${CATALOG_ROOT:?} cargo test", "")]
        with self.assertRaises(live_map.MapError):
            ensure_catalog_root(cells, "/no/such/catalog-root-r2")

    def test_catalog_root_uses_environment_when_flag_is_absent(self) -> None:
        import tempfile

        cells = [
            live_map.Cell(
                "nav.route-search-live",
                "LIVE=1 ${CATALOG_ROOT:?} cargo test",
                "",
            )
        ]
        with tempfile.TemporaryDirectory() as temporary:
            with unittest.mock.patch.dict(
                os.environ, {"CATALOG_ROOT": temporary}, clear=True
            ):
                self.assertEqual(ensure_catalog_root(cells, None), temporary)

    def test_partner_prefix_is_required_for_two_account_cells(self) -> None:
        pair = [
            live_map.Cell(
                "host.quester-partner-trade",
                "LIVE=1 BOT_LIVE_PARTNER_NAME_PREFIX=${BOT_LIVE_PARTNER_NAME_PREFIX:?} cargo test",
                "",
            )
        ]
        self.assertTrue(needs_partner_prefix(pair))
        with unittest.mock.patch.dict(os.environ):
            os.environ.pop("BOT_LIVE_PARTNER_NAME_PREFIX", None)
            with self.assertRaises(live_map.MapError) as context:
                ensure_partner_prefix(pair, None, "abc")
            self.assertIn("--partner-prefix", str(context.exception))
            os.environ["BOT_LIVE_PARTNER_NAME_PREFIX"] = "xyz"
            self.assertEqual(ensure_partner_prefix(pair, None, "abc"), "xyz")
        self.assertEqual(ensure_partner_prefix(pair, "def", "abc"), "def")
        with self.assertRaises(live_map.MapError):
            ensure_partner_prefix(pair, "abc", "abc")
        with self.assertRaises(live_map.MapError):
            ensure_partner_prefix(pair, "toolong", "abc")
        single = [live_map.Cell("plain", "LIVE=1 ${ISOHOME:?} cargo test", "")]
        self.assertFalse(needs_partner_prefix(single))
        self.assertIsNone(ensure_partner_prefix(single, None, "abc"))

    def test_partner_prefix_flag_reaches_cell_env(self) -> None:
        args = parse_run_args(
            ["--repo", "/r", "--cells", "a", "--prefix", "abc", "--evidence", "/e",
             "--partner-prefix", "def"]
        )
        self.assertEqual(args.partner_prefix, "def")
        env, _ = build_cell_env(
            cell_name="host.quester-partner-trade",
            cell_command="LIVE=1 ${ISOHOME:?} cargo test",
            prefix="abc",
            live_evidence_dir="/e/cell/pass1/evidence",
            bot_cache_dir="/e/cell/cache-289",
            client_unpack_dir="/e/cell/client-unpack",
            bot_unpack_dir="/e/cell/client-unpack",
            panel_home="/e/cell/pass1/panel-home",
            repo="/tmp/repo",
            engine_game_port=47594,
            engine_http_port=3080,
            quest_game_port=47694,
            quest_http_port=3180,
            partner_prefix="def",
        )
        self.assertEqual(env["BOT_LIVE_PARTNER_NAME_PREFIX"], "def")

    def test_nav_pack_defaults_come_from_the_repo_local_data(self) -> None:
        import tempfile
        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp)
            nav = repo / "target" / "local-data" / "nav"
            (nav / "289").mkdir(parents=True)
            (nav / "289" / "274bot.navpack").write_bytes(b"p")
            (nav / "289" / "274bot.navflags").write_bytes(b"f")
            with unittest.mock.patch.dict(os.environ):
                for name in ("NAV_PACK_289", "NAV_FLAGS_289", "NAV_PACK_274"):
                    os.environ.pop(name, None)
                defaults = host_defaults(repo)
            self.assertEqual(defaults["NAV_PACK_289"], str(nav / "289" / "274bot.navpack"))
            self.assertEqual(defaults["NAV_FLAGS_289"], str(nav / "289" / "274bot.navflags"))
            self.assertNotIn("NAV_PACK_274", defaults)

    def test_host_path_defaults_use_only_configured_paths(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp)
            isohome = repo / "isohome"
            isohome.write_text("", encoding="utf-8")
            engine_a = repo / "engine-a"
            engine_a.mkdir()
            engine_q = repo / "engine-q"
            engine_q.mkdir()
            snapshot = repo / "snapshot"
            snapshot.mkdir()
            with unittest.mock.patch.dict(os.environ, {}, clear=True):
                defaults = host_defaults(repo)
            for key in (
                "ISOHOME",
                "ENGINE_A_DIR",
                "ENGINE_Q_DIR",
                "ENGINE_B_DIR",
                "NAV_SNAPSHOT_ROOT_289",
            ):
                self.assertNotIn(key, defaults)

            configured = {
                "ISOHOME": str(isohome),
                "ENGINE_A_DIR": str(engine_a),
                "ENGINE_Q_DIR": str(engine_q),
                "NAV_SNAPSHOT_ROOT_289": str(snapshot),
            }
            with unittest.mock.patch.dict(os.environ, configured, clear=True):
                defaults = host_defaults(repo)
            self.assertEqual(defaults["ISOHOME"], str(isohome))
            self.assertEqual(defaults["ENGINE_A_DIR"], str(engine_a))
            self.assertEqual(defaults["ENGINE_Q_DIR"], str(engine_q))
            self.assertEqual(defaults["ENGINE_B_DIR"], str(engine_a))
            self.assertEqual(defaults["NAV_SNAPSHOT_ROOT_289"], str(snapshot))

    def test_selected_live_cells_require_isohome_before_running(self) -> None:
        cells = [live_map.Cell("live", "LIVE=1 ${ISOHOME:?} cargo test", "")]
        with unittest.mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(live_map.MapError, "ISOHOME.*lab.env"):
                require_isohome(cells)

    def test_selected_cells_require_configured_engine_paths(self) -> None:
        cells = [
            live_map.Cell("engine", "LIVE=1 WORLD_ENGINE_DIR=${ENGINE_Q_DIR:?} cargo test", "")
        ]
        with unittest.mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(live_map.MapError, "ENGINE_Q_DIR.*lab.env"):
                require_configured_paths(cells)

    def test_catalog_root_reaches_cell_env(self) -> None:
        import tempfile
        with tempfile.TemporaryDirectory() as temp:
            env, _ = build_cell_env(
                cell_name="nav.route-search-live",
                cell_command="LIVE=1 RS2B0T=${CATALOG_ROOT:?} ${ISOHOME:?} cargo test",
                prefix="abc",
                live_evidence_dir="/e/cell/pass1/evidence",
                bot_cache_dir="/e/cell/cache-289",
                client_unpack_dir="/e/cell/client-unpack",
                bot_unpack_dir="/e/cell/client-unpack",
                panel_home="/e/cell/pass1/panel-home",
                repo="/tmp/repo",
                engine_game_port=47594,
                engine_http_port=3080,
                quest_game_port=47694,
                quest_http_port=3180,
                defaults={},
                catalog_root=temp,
            )
            self.assertEqual(env["CATALOG_ROOT"], temp)

    def test_isohome_dir_stays_under_evidence(self) -> None:
        env, _ = build_cell_env(
            cell_name="host.script-runtime-load",
            cell_command="LIVE=1 ${ISOHOME:?} cargo test",
            prefix="abc",
            live_evidence_dir="/e/cell/pass1/evidence",
            bot_cache_dir="/e/cell/cache-289",
            client_unpack_dir="/e/cell/client-unpack",
            bot_unpack_dir="/e/cell/client-unpack",
            panel_home="/e/cell/pass1/panel-home",
            repo="/tmp/repo",
            engine_game_port=47594,
            engine_http_port=3080,
            quest_game_port=47694,
            quest_http_port=3180,
            defaults={},
        )
        self.assertEqual(env["ISOHOME_DIR"], "/e/cell/pass1/evidence/home")

    def test_unpack_source_is_required_for_panel_cells(self) -> None:
        cells = [live_map.Cell("panel.state-gap-stop-live", "LIVE=1 ISOHOME_DIR=${PANEL_LIVE_HOME:?} cargo test", "")]
        self.assertTrue(needs_panel_cache(cells))
        with unittest.mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(live_map.MapError) as context:
                ensure_unpack_source(cells, None)
        self.assertIn("--unpack-source", str(context.exception))
        self.assertIn("panel.state-gap-stop-live", str(context.exception))
        clean = [live_map.Cell("plain", "LIVE=1 ${ISOHOME:?} cargo test", "")]
        self.assertFalse(needs_panel_cache(clean))
        self.assertIsNone(ensure_unpack_source(clean, None))

    def test_unpack_source_missing_dir_is_refused(self) -> None:
        cells = [live_map.Cell("panel.state-gap-stop-live", "ISOHOME_DIR=${PANEL_LIVE_HOME:?} cargo test", "")]
        with self.assertRaises(live_map.MapError):
            ensure_unpack_source(cells, "/no/such/unpack-source-r2")

    def test_unpack_source_uses_nav_snapshot_environment(self) -> None:
        import tempfile

        cells = [
            live_map.Cell(
                "panel.state-gap-stop-live",
                "LIVE=1 ISOHOME_DIR=${PANEL_LIVE_HOME:?} cargo test",
                "",
            )
        ]
        with tempfile.TemporaryDirectory() as temporary:
            with unittest.mock.patch.dict(
                os.environ, {"NAV_SNAPSHOT_ROOT_289": temporary}, clear=True
            ):
                self.assertEqual(ensure_unpack_source(cells, None), temporary)

    def test_stage_copies_files_without_symlink(self) -> None:
        import tempfile
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "unpack-289"
            source.mkdir()
            (source / "config").write_text("config-bytes", encoding="utf-8")
            nested = source / "media"
            nested.mkdir()
            (nested / "sprite").write_text("sprite-bytes", encoding="utf-8")
            panel = Path(temp) / "panel-home"
            panel.mkdir()
            record = stage_panel_unpack(panel, str(source))
            self.assertIn(str(source), record)
            dest = panel / ".274bot" / "unpack-289"
            self.assertTrue((dest / "config").is_file())
            self.assertTrue((dest / "media" / "sprite").is_file())
            self.assertFalse((dest / "config").is_symlink())
            self.assertFalse(dest.is_symlink())
            again = stage_panel_unpack(panel, str(source))
            self.assertEqual(record, again)

    def test_run_all_records_catalog_and_unpack(self) -> None:
        import tempfile
        cells = [
            live_map.Cell("panel.state-gap-stop-live", "LIVE=1 ISOHOME_DIR=${PANEL_LIVE_HOME:?} true", "--locked"),
        ]
        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("ok\n", encoding="utf-8")
            return 0, "s", "e", 0
        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp) / "repo"
            repo.mkdir()
            evidence = Path(temp) / "evidence"
            source = Path(temp) / "unpack-289"
            source.mkdir()
            (source / "config").write_text("x", encoding="utf-8")
            catalog = Path(temp) / "catalog"
            catalog.mkdir()
            with unittest.mock.patch(f"{__name__}.repo_head", return_value="deadbeef"):
                run_all(
                    cells,
                    repo=repo,
                    evidence_root=evidence,
                    prefix="abc",
                    passes=1,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    catalog_root=str(catalog),
                    unpack_source=str(source),
                    runner=fake_runner,
                )
            receipt = (evidence / "panel.state-gap-stop-live" / "pass1" / "receipt.txt").read_text(encoding="utf-8")
            self.assertIn(f"catalog_root={catalog}", receipt)
            self.assertIn("unpack_source=", receipt)
            self.assertIn(str(source), receipt)
    def test_parallel_run_all_overlaps_and_uses_unique_prefixes(self) -> None:
        import threading
        import tempfile

        cells = [
            live_map.Cell("cell-primary", "LIVE=1 ${ISOHOME:?} true", "true"),
            live_map.Cell(
                "cell-pair",
                "LIVE=1 BOT_LIVE_PARTNER_NAME_PREFIX=${BOT_LIVE_PARTNER_NAME_PREFIX:?} "
                "${ISOHOME:?} true",
                "true",
            ),
        ]
        barrier = threading.Barrier(2)
        lock = threading.Lock()
        accounts: list[tuple[str, str, str | None]] = []

        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            cell = kwargs["cell"]
            env = kwargs["env"]
            primary = env["BOT_LIVE_NAME_PREFIX"]
            partner = env.get("BOT_LIVE_PARTNER_NAME_PREFIX")
            with lock:
                accounts.append((cell.name, primary, partner))
            barrier.wait(timeout=3)
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("ok\n", encoding="utf-8")
            return 0, "start", "end", 1

        with tempfile.TemporaryDirectory(prefix="run-cells-parallel-prefix-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            evidence = root / "evidence"
            with unittest.mock.patch(
                f"{__name__}.repo_head", return_value="deadbeef"
            ):
                results = run_all(
                    cells,
                    repo=repo,
                    evidence_root=evidence,
                    prefix="base",
                    passes=1,
                    jobs=2,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    partner_prefix="pair",
                    runner=fake_runner,
                )

            self.assertEqual([result.exit_code for result in results], [0, 0])
            primary_prefixes = [primary for _, primary, _ in accounts]
            self.assertEqual(len(set(primary_prefixes)), 2)
            self.assertTrue(all(len(prefix) == 4 for prefix in primary_prefixes))
            partner_prefixes = [
                partner for _, _, partner in accounts if partner is not None
            ]
            self.assertEqual(len(partner_prefixes), 1)
            all_prefixes = primary_prefixes + partner_prefixes
            self.assertEqual(len(set(all_prefixes)), len(all_prefixes))
            deterministic_names = [
                f"{prefix}{role}000123"
                for prefix in all_prefixes
                for role in ("a", "b")
            ]
            self.assertEqual(len(set(deterministic_names)), len(deterministic_names))
            for cell_name, primary, partner in accounts:
                receipt = (
                    evidence / cell_name / "pass1" / "receipt.txt"
                ).read_text(encoding="utf-8")
                self.assertIn(f"prefix={primary}\n", receipt)
                if partner is not None:
                    self.assertIn(f"partner_prefix={partner}\n", receipt)

    def test_parallel_worker_error_does_not_abort_other_cells(self) -> None:
        import tempfile

        cells = [
            live_map.Cell("cell-fails", "${ISOHOME:?} true", "true"),
            live_map.Cell("cell-succeeds", "${ISOHOME:?} true", "true"),
        ]
        completed: list[str] = []

        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            cell_name = kwargs["cell"].name
            completed.append(cell_name)
            if cell_name == "cell-fails":
                raise RuntimeError("runner failed")
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("ok\n", encoding="utf-8")
            return 0, "start", "end", 1

        with tempfile.TemporaryDirectory(prefix="run-cells-parallel-error-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            with unittest.mock.patch(
                f"{__name__}.repo_head", return_value="deadbeef"
            ):
                results = run_all(
                    cells,
                    repo=repo,
                    evidence_root=root / "evidence",
                    prefix="base",
                    passes=1,
                    jobs=2,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    runner=fake_runner,
                )

        self.assertCountEqual(completed, ["cell-fails", "cell-succeeds"])
        self.assertEqual(
            [(result.cell, result.exit_code) for result in results],
            [("cell-fails", 1), ("cell-succeeds", 0)],
        )

    def test_parallel_summary_file_keeps_selected_cell_order(self) -> None:
        import tempfile
        import time

        cells = [
            live_map.Cell("slow-first", "${ISOHOME:?} true", "true"),
            live_map.Cell("fast-second", "${ISOHOME:?} true", "true"),
        ]

        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            if kwargs["cell"].name == "slow-first":
                time.sleep(0.05)
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("ok\n", encoding="utf-8")
            return 0, "start", "end", 1

        with tempfile.TemporaryDirectory(prefix="run-cells-parallel-summary-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            evidence = root / "evidence"
            with unittest.mock.patch(
                f"{__name__}.repo_head", return_value="deadbeef"
            ):
                results = run_all(
                    cells,
                    repo=repo,
                    evidence_root=evidence,
                    prefix="base",
                    passes=1,
                    jobs=2,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    runner=fake_runner,
                )
            summary = format_summary(
                results,
                repo=str(repo),
                head="deadbeef",
                prefix="base",
                evidence=str(evidence),
            )
            summary_path = evidence / "summary.txt"
            summary_path.write_text(summary + "\n", encoding="utf-8")
            written = summary_path.read_text(encoding="utf-8")

        self.assertLess(written.index("| slow-first |"), written.index("| fast-second |"))

    def test_parallel_exclusive_cell_does_not_overlap_other_cells(self) -> None:
        import threading
        import tempfile
        import time

        exclusive = "host.upstream-e2e-adopted"
        cells = [
            live_map.Cell("ordinary-a", "${ISOHOME:?} true", "true"),
            live_map.Cell(exclusive, "${ISOHOME:?} true", "true"),
            live_map.Cell("ordinary-b", "${ISOHOME:?} true", "true"),
        ]
        lock = threading.Lock()
        active: set[str] = set()
        violations: list[str] = []

        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            name = kwargs["cell"].name
            with lock:
                if name == exclusive and active:
                    violations.extend(sorted(active))
                if name != exclusive and exclusive in active:
                    violations.append(name)
                active.add(name)
            time.sleep(0.03)
            with lock:
                active.remove(name)
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("ok\n", encoding="utf-8")
            return 0, "start", "end", 1

        with tempfile.TemporaryDirectory(prefix="run-cells-parallel-exclusive-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            with unittest.mock.patch(
                f"{__name__}.repo_head", return_value="deadbeef"
            ):
                results = run_all(
                    cells,
                    repo=repo,
                    evidence_root=root / "evidence",
                    prefix="base",
                    passes=1,
                    jobs=3,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                    runner=fake_runner,
                )

        self.assertEqual(violations, [])
        self.assertEqual([result.exit_code for result in results], [0, 0, 0])

    def test_parallel_keyboard_interrupt_stops_pending_cells(self) -> None:
        import tempfile
        import threading

        cells = [
            live_map.Cell(name, "${ISOHOME:?} true", "true")
            for name in ("first", "second", "pending-a", "pending-b")
        ]
        lock = threading.Lock()
        started: list[str] = []
        started_two = threading.Event()
        release = threading.Event()

        def fake_runner(**kwargs) -> tuple[int, str, str, int]:
            name = kwargs["cell"].name
            with lock:
                started.append(name)
                if len(started) == 2:
                    started_two.set()
            release.wait(timeout=3)
            log_path = kwargs["log_path"]
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("ok\n", encoding="utf-8")
            return 0, "start", "end", 1

        def interrupt_as_completed(futures):
            self.assertTrue(started_two.wait(timeout=3))
            release_timer = threading.Timer(0.1, release.set)
            release_timer.daemon = True
            release_timer.start()
            raise KeyboardInterrupt

        with tempfile.TemporaryDirectory(prefix="run-cells-parallel-interrupt-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            with (
                unittest.mock.patch(
                    f"{__name__}.repo_head", return_value="deadbeef"
                ),
                unittest.mock.patch(
                    f"{__name__}.concurrent.futures.as_completed",
                    side_effect=interrupt_as_completed,
                ),
            ):
                with self.assertRaises(KeyboardInterrupt):
                    run_all(
                        cells,
                        repo=repo,
                        evidence_root=root / "evidence",
                        prefix="base",
                        passes=1,
                        jobs=2,
                        engine_game_port=1,
                        engine_http_port=2,
                        quest_game_port=3,
                        quest_http_port=4,
                        defaults={},
                        runner=fake_runner,
                    )

        self.assertCountEqual(started, ["first", "second"])

    def test_precompile_failure_tail_is_written_to_cell_log(self) -> None:
        import tempfile

        cell = live_map.Cell(
            "cargo-precompile-fails",
            "${ISOHOME:?} cargo test --locked -p host-play --test sample",
            "--locked -p host-play --test sample",
        )

        def fail_precompile(*args, **kwargs):
            kwargs["stdout"].write(
                "".join(f"precompile line {line}\n" for line in range(100))
            )
            return unittest.mock.Mock(returncode=1)

        with tempfile.TemporaryDirectory(prefix="run-cells-precompile-failure-") as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            evidence = root / "evidence"
            with (
                unittest.mock.patch(
                    f"{__name__}.repo_head", return_value="deadbeef"
                ),
                unittest.mock.patch(
                    f"{__name__}.subprocess.run", side_effect=fail_precompile
                ),
            ):
                results = run_all(
                    [cell],
                    repo=repo,
                    evidence_root=evidence,
                    prefix="base",
                    passes=1,
                    jobs=2,
                    engine_game_port=1,
                    engine_http_port=2,
                    quest_game_port=3,
                    quest_http_port=4,
                    defaults={},
                )

            self.assertEqual([result.exit_code for result in results], [1])
            log = (
                evidence / cell.name / "pass1" / "cell.log"
            ).read_text(encoding="utf-8")

            precompile_logs = list((evidence / "precompile").glob("*.log"))
            self.assertEqual(len(precompile_logs), 1)
            self.assertIn(f"full log: {precompile_logs[0]}", log)
            self.assertIn("precompile line 20", log)
            self.assertIn("precompile line 99", log)
            self.assertNotIn("precompile line 19", log)

    def test_precompile_skips_compound_shell_commands(self) -> None:
        self.assertIsNone(
            _cargo_test_precompile(
                "python3 tools/check.py && cargo test --locked -p host-play"
            )
        )
        self.assertIsNone(
            _cargo_test_precompile(
                "cd vendor/fr-client-rust && cargo test --locked -p client"
            )
        )
        self.assertIsNotNone(
            _cargo_test_precompile(
                "${ISOHOME:?} nice -n 10 cargo test --locked -p host-play --test sample"
            )
        )

    def test_derived_prefixes_ignore_case_when_checking_collisions(self) -> None:
        first = _derived_prefix("base", "cell", "primary", set())
        second = _derived_prefix("base", "cell", "primary", {first.upper()})
        self.assertNotEqual(first.casefold(), second.casefold())

def self_test() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RunCellsTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
