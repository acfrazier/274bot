#!/usr/bin/env python3
"""Run live cells listed in map.toml, one at a time.

This module imports live_map for cell names and commands. It never
duplicates a per-cell command. It sets the environment each cell needs,
runs each cell in its own pass, writes one receipt per cell and pass,
and never stops the loop on a failure.

Use through live_map.py so one tool lists and runs cells:

    python3 -B tools/live-map/live_map.py run --repo <clone> \\
        --cells frontend-core.driver-command-headless,host-play.local-state-headless \\
        --prefix lcr --evidence /path/to/evidence --passes 1

Or call this file directly (the leading ``run`` verb is then absent):

    python3 -B tools/live-map/run_cells.py --repo <clone> --cells <id,...> \\
        --prefix lcr --evidence /path/to/evidence --passes 1
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import os
import re
import shutil
import subprocess
import sys
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


def is_quest_cell(cell_name: str, cell_command: str) -> bool:
    return (
        "quester" in cell_name
        or "QUESTER_TICK_MS" in cell_command
        or "ENGINE_Q_DIR" in cell_command
    )


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
    if not catalog_root:
        names = ", ".join(cell.name for cell in cells if "CATALOG_ROOT" in cell.command)
        raise live_map.MapError(
            f"run-cells: refusing cells [{names}]: a selected cell uses ${{CATALOG_ROOT}}. "
            "Pass --catalog-root <path> with the local rs2b0t checkout root."
        )
    if not Path(catalog_root).is_dir():
        raise live_map.MapError(
            f"run-cells: --catalog-root is not a directory: {catalog_root}"
        )
    return catalog_root


def needs_panel_cache(cells: list[live_map.Cell]) -> bool:
    return any("PANEL_LIVE_HOME" in cell.command for cell in cells)


def ensure_unpack_source(cells: list[live_map.Cell], unpack_source: str | None) -> str | None:
    if not needs_panel_cache(cells):
        return unpack_source
    if not unpack_source:
        names = ", ".join(cell.name for cell in cells if "PANEL_LIVE_HOME" in cell.command)
        raise live_map.MapError(
            f"run-cells: refusing cells [{names}]: a selected cell stages panel-home/.274bot/unpack-289. "
            "Pass --unpack-source <path> with the revision decoded unpack directory."
        )
    if not Path(unpack_source).is_dir():
        raise live_map.MapError(
            f"run-cells: --unpack-source is not a directory: {unpack_source}"
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
    home = str(Path.home())
    # The nav pack is the repository's own build output. The stored v17 and v18
    # reference packs are refused by the current reader.
    defaults: dict[str, str] = {}
    isohome = _first_existing(
        [
            os.environ.get("ISOHOME"),
            "/home/acfrazier/experiments/274bot/.superpowers/bin/isohome",
            "/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome",
            f"{home}/experiments/274bot/.superpowers/bin/isohome",
        ],
        want_file=True,
    )
    if isohome:
        defaults["ISOHOME"] = isohome
    engine_a = _first_existing(
        [
            os.environ.get("ENGINE_A_DIR"),
            "/home/acfrazier/experiments/lostcity-289/engine",
            "/Users/acfrazier/experiments/lostcity-289/engine",
            "/srv/dev/experiments/lostcity-289/engine",
        ],
        want_file=False,
    )
    if engine_a:
        defaults["ENGINE_A_DIR"] = engine_a
    engine_q = _first_existing(
        [
            os.environ.get("ENGINE_Q_DIR"),
            "/home/acfrazier/experiments/lostcity-289-q/engine",
            "/Users/acfrazier/experiments/lostcity-289-q/engine",
        ],
        want_file=False,
    )
    if engine_q:
        defaults["ENGINE_Q_DIR"] = engine_q
    engine_b = _first_existing(
        [os.environ.get("ENGINE_B_DIR")],
        want_file=False,
    )
    if engine_b:
        defaults["ENGINE_B_DIR"] = engine_b
    elif engine_a:
        defaults["ENGINE_B_DIR"] = engine_a
    snapshot = _first_existing(
        [
            os.environ.get("NAV_SNAPSHOT_ROOT_289"),
            "/home/acfrazier/.274bot/unpack-289",
            "/Users/acfrazier/.274bot/unpack-289",
        ],
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
    quest = is_quest_cell(cell_name, cell_command)
    game_port = quest_game_port if quest else engine_game_port
    http_port = quest_http_port if quest else engine_http_port
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
    runner: Callable[..., tuple[int, str, str, int]] | None = None,
) -> list[RunResult]:
    head = repo_head(repo)
    results: list[RunResult] = []
    run_fn = runner or run_one_cell_by_args
    for cell in cells:
        quest = is_quest_cell(cell.name, cell.command)
        game_port = quest_game_port if quest else engine_game_port
        http_port = quest_http_port if quest else engine_http_port
        cell_dir = evidence_root / cell.name
        cache_dir = cell_dir / "cache-289"
        client_dir = cell_dir / "client-unpack"
        cache_dir.mkdir(parents=True, exist_ok=True)
        client_dir.mkdir(parents=True, exist_ok=True)
        for passed_pass in range(1, passes + 1):
            pass_dir = cell_dir / f"pass{passed_pass}"
            live_dir = pass_dir / "evidence"
            panel_home = pass_dir / "panel-home"
            live_dir.mkdir(parents=True, exist_ok=True)
            panel_home.mkdir(parents=True, exist_ok=True)
            (live_dir / "tui").mkdir(parents=True, exist_ok=True)
            (live_dir / "home").mkdir(parents=True, exist_ok=True)
            pass_cache_dir = live_dir / "cache-289"
            shutil.copytree(cache_dir, pass_cache_dir, symlinks=False)
            log_path = pass_dir / "cell.log"
            receipt_path = pass_dir / "receipt.txt"
            unpack_record: str | None = None
            if unpack_source is not None and "PANEL_LIVE_HOME" in cell.command:
                unpack_record = stage_panel_unpack(panel_home, unpack_source)
            env, command = build_cell_env(
                cell_name=cell.name,
                cell_command=cell.command,
                prefix=prefix,
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
                partner_prefix=partner_prefix,
            )
            try:
                if runner is not None:
                    exit_code, start, end, duration = runner(
                        cell=cell,
                        repo=repo,
                        env=env,
                        command=command,
                        log_path=log_path,
                    )
                else:
                    exit_code, start, end, duration = run_fn(
                        cell=cell,
                        repo=repo,
                        env=env,
                        command=command,
                        log_path=log_path,
                    )
            except Exception as error:  # never stop the loop on a failure
                exit_code, start, end, duration = (
                    1,
                    _utc_now(),
                    _utc_now(),
                    0,
                )
                log_path.parent.mkdir(parents=True, exist_ok=True)
                with log_path.open("a", encoding="utf-8") as stream:
                    stream.write(f"\nrunner error: {error}\n")
            receipt_fields: dict[str, str] = {
                "cell": cell.name,
                "pass": str(passed_pass),
                "repo": str(repo),
                "head": head,
                "prefix": prefix,
                "command": command,
                "start_utc": start,
                "end_utc": end,
                "duration_seconds": str(duration),
                "exit_code": str(exit_code),
                "log": str(log_path),
                "evidence_dir": str(live_dir),
                "isohome_dir": env.get("ISOHOME_DIR", ""),
            }
            if catalog_root is not None:
                receipt_fields["catalog_root"] = catalog_root
            if unpack_record is not None:
                receipt_fields["unpack_source"] = unpack_record
            write_receipt(receipt_path, receipt_fields)
            results.append(
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
                )
            )
    return results


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
        if args.cells:
            names = [part.strip() for part in args.cells.split(",") if part.strip()]
            if not names:
                print("run-cells: --cells is empty", file=sys.stderr)
                return 2
            cells = resolve_cells(config, names)
        else:
            paths = live_map.diff_paths(args.diff, repo)
            plan = live_map.make_plan(paths, config)
            cells = list(plan.cells)
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
        evidence_root.mkdir(parents=True, exist_ok=True)
        defaults = host_defaults(repo)
        results = run_all(
            cells,
            repo=repo,
            evidence_root=evidence_root,
            prefix=args.prefix,
            passes=args.passes,
            engine_game_port=args.engine_game_port,
            engine_http_port=args.engine_http_port,
            quest_game_port=args.quest_game_port,
            quest_http_port=args.quest_http_port,
            defaults=defaults,
            catalog_root=catalog_root,
            unpack_source=unpack_source,
            partner_prefix=partner_prefix,
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

    def test_quest_cells_use_quest_ports_and_speed_restore(self) -> None:
        env, command = build_cell_env(
            cell_name="host.quester-path-engine-q",
            cell_command=(
                "LIVE=1 WORLD_GAME_PORT=44694 "
                "WORLD_HTTP_PORT=1180 ${ISOHOME:?} cargo test -- --port 44694 --http-port 1180"
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
        self.assertEqual(env["WORLD_GAME_PORT"], "47694")
        self.assertEqual(env["WORLD_HTTP_PORT"], "3180")
        self.assertEqual(env["QUESTER_SPEED_RESTORE"], "0")
        self.assertNotIn("QUESTER_TICK_MS", env)
        self.assertIn("47694", command)
        self.assertIn("3180", command)
        self.assertNotIn("44594", command)
        self.assertNotIn("44694", command)

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

def self_test() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RunCellsTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
