#!/usr/bin/env python3
"""274bot nightly live run: build the frontends and run the selected cells.

The runner fetches each branch anonymously, builds with `--locked`, runs the
selected catalog cells, and writes `summary.json`, `summary.md`, per-cell
receipts and a source-side `SHA256SUMS`. `DONE` is written last.

Linux (malbec) runs two sequential jobs: the main job runs non-quest cells on
its private Engine A copy; the quest job runs only `quester_*` cells on its
separate local Engine Q copy. Both use `tui-play`.

macOS (merlot) runs main cells through `lanes,panel,tui` against already-running
Engine A, then `quester_*` cells through `panel,tui` against Engine Q. Remote
engines are checked but never started, stopped, or changed. `MAC-QUIET.lock`
prevents startup, stops active live cells promptly, and marks those cells
UNAVAILABLE.

Windows runs the main job's CI lanes and cells through `panel-play` and
`tui-play` on ConPTY, then a separate quest job on Engine B. Engine B remains
at its existing tick rate; the quest job does not set `QUESTER_TICK_MS`.

The revision-274 lane uses this same script with `NIGHTLY_REVISION=274`, a
single branch (`dev`) and its exact cell list. It reports only; it never gates
a merge or release.

The latency lane is off unless `NIGHTLY_LATENCY_BANDS` or `--latency-bands`
names RTT bands (for example `50,100,150`). When it is on, after the tui
step, the runner starts one `latency_proxy.py` per band, reruns the
`cells-latency.conf` cells through it on the latency branch only, and
classifies each band row against the same night's zero-latency result. It
reports only; it never gates a merge or release.

Standard library only (Python 3.10+); on Windows also `pywinpty` for ConPTY.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import socket
import stat
import statistics
import subprocess
import sys
import threading
import time
import urllib.request
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Callable, Iterator
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

IS_MAC = sys.platform == "darwin"
IS_WINDOWS = os.name == "nt"
if IS_WINDOWS:
    import msvcrt
else:
    import fcntl
    import pty
    import struct
    import termios


def env_flag(name: str, default: bool) -> bool:
    return os.environ.get(name, "1" if default else "0") not in ("", "0", "false", "no")


HERE = Path(__file__).resolve().parent
DEFAULT_ROOT = r"C:\274bot-nightly" if IS_WINDOWS else (
    "/Volumes/dev-scratch/274bot-nightly-mac" if IS_MAC else "/srv/nightly")
ROOT = Path(os.environ.get("NIGHTLY_ROOT", DEFAULT_ROOT))
REPO_URL = os.environ.get("NIGHTLY_REPO", "https://github.com/acfrazier/274bot")
RUNNER_REPO = HERE.parent.parent
NAV_274_REQUIRED_CONTENT = RUNNER_REPO / "crates" / "nav-planner" / "src" / "required-content-274.tsv"


# Engine A is the non-Windows revision-289 pack. Engine B has its own
# verified revision-289 identity after content patch 0002.
ENGINE_A_289_CRC_SHA256 = (
    "687190c8ef08bb94a1422f4262d62bb7a76a55dfe0634ea43f0e63e0235dc623"
)
ENGINE_B_289_CRC_SHA256 = (
    "5f8a00094e8eb02dd00d703e99a073071d0349a8731f43dffa0bdaae36fa6c69"
)


def revision_defaults(revision: str, windows: bool) -> dict:
    """The settings a game revision implies, before any specific override.

    289 is the production revision. Non-Windows jobs keep the established
    289 pack identity. Windows jobs use Engine B's rebuilt identity. Revision
    274 has a separate launch profile, cache directory (`unpack` instead of
    `unpack-289`), and run-folder suffix. It has no pinned CRC identity.
    """
    if revision not in ("274", "289"):
        raise ValueError(f"revision must be 274 or 289, got {revision!r}")
    suffix = "-windows" if windows else ""
    if revision != "289":
        suffix = f"-{revision}{suffix}"
    return {
        "profile": f"local-{revision}",
        "unpack_dir": "unpack-289" if revision == "289" else "unpack",
        "run_suffix": suffix,
        "engine_crc_sha256": (
            (ENGINE_B_289_CRC_SHA256 if windows else ENGINE_A_289_CRC_SHA256)
            if revision == "289" else ""),
    }


def job_defaults(job: str, windows: bool, revision: str, root: Path, mac: bool = False) -> dict:
    """Return engine and identity settings for the selected nightly job."""
    if job not in ("main", "quest"):
        raise ValueError(f"nightly job must be main or quest, got {job!r}")
    revision_config = revision_defaults(revision, windows)
    if windows:
        engine = {
            "host": "127.0.0.1",
            "game_port": 47594,
            "http_port": 3080,
            "label": "Engine B",
            "profile": revision_config["profile"],
            "pristine": root / "engine-pristine",
            "run": root / "engine-run",
        }
    elif job == "quest":
        engine = {
            "host": "127.0.0.1",
            "game_port": 44694,
            "http_port": 1180,
            "label": "Engine Q",
            "profile": "local-289q" if revision == "289" else revision_config["profile"],
            "pristine": root / "engine-q-pristine",
            "run": root / "engine-q-run",
        }
    else:
        engine = {
            "host": "127.0.0.1",
            "game_port": 44594,
            "http_port": 1080,
            "label": "Engine A",
            "profile": revision_config["profile"],
            "pristine": root / "engine-pristine",
            "run": root / "engine-run",
        }
    quest = job == "quest"
    platform_suffix = "-mac" if mac and not windows else ""
    return {
        **engine,
        "run_suffix": revision_config["run_suffix"] + platform_suffix + ("-quest" if quest else ""),
        "name_prefix": ("wq" if windows else "nq") if quest else ("nw" if windows else "nt"),
        "panel_name_prefix": ("wqp" if windows else "nqp") if quest else "np",
        "tick_ms": 300 if quest and not windows and revision == "289" else None,
    }


# The nightly job controls which catalog cells run. Quest cells need a separate
# Engine Q because `::speed` changes the whole world.
JOB = os.environ.get("NIGHTLY_JOB", "main")
if JOB not in ("main", "quest"):
    raise SystemExit(f"NIGHTLY_JOB must be main or quest, got {JOB!r}")
REVISION = os.environ.get("NIGHTLY_REVISION", "289")
try:
    DEFAULTS = revision_defaults(REVISION, IS_WINDOWS)
    JOB_DEFAULTS = job_defaults(JOB, IS_WINDOWS, REVISION, ROOT, IS_MAC)
except ValueError as error:
    raise SystemExit(f"NIGHTLY_REVISION/NIGHTLY_JOB: {error}") from error
PROFILE = os.environ.get("NIGHTLY_PROFILE", JOB_DEFAULTS["profile"])
UNPACK_DIR = DEFAULTS["unpack_dir"]


def load_timezone() -> ZoneInfo | None:
    """America/New_York. Windows CPython ships no tz database (no `tzdata`
    package in the nightly venv), so there, unless NIGHTLY_TZ is set, the
    host's own clock zone is used; the nightly host is set to Eastern time."""
    try:
        return ZoneInfo(os.environ.get("NIGHTLY_TZ", "America/New_York"))
    except ZoneInfoNotFoundError:
        if IS_WINDOWS and "NIGHTLY_TZ" not in os.environ:
            return None
        raise


TIMEZONE = load_timezone()
NAME_PREFIX = os.environ.get("NIGHTLY_NAME_PREFIX", JOB_DEFAULTS["name_prefix"])
PANEL_NAME_PREFIX = os.environ.get("NIGHTLY_PANEL_NAME_PREFIX", JOB_DEFAULTS["panel_name_prefix"])
KEEP_DAYS = int(os.environ.get("NIGHTLY_KEEP_DAYS", "14"))
# Every cell enforces its own scenario deadline inside the frontend; this outer
# cap only stops a process that never reaches a terminal state.
CELL_CAP_S = int(os.environ.get("NIGHTLY_CELL_CAP_S", "3600"))
# Two cells need more than the Windows host's 1200 s outer cap. The proven
# walls come from the 2026-10-21 night (summary.json in each run folder).
# jive_kbd reached a verdict at 2543.0 s on Linux and 2529.6 s on Mac TUI.
# Its scenario deadline is 2700 s. quester_queue passed at 1987.9 s on Linux
# and at 1022.0 s on Windows TUI. A cap of 3600 s covers each proven wall.
# It also keeps the runner default for the other platforms.
CELL_CAPS_DEFAULT_S = {"jive_kbd": 3600, "quester_queue": 3600}


def parse_cell_caps(text: str, defaults: dict) -> dict:
    """Parse a `cell=seconds,cell=seconds` override over the default caps."""
    caps = dict(defaults)
    for entry in text.split(","):
        entry = entry.strip()
        if not entry:
            continue
        name, sep, seconds = entry.partition("=")
        if not sep or not name.strip():
            raise ValueError(f"cell cap must be cell=seconds, got {entry!r}")
        try:
            value = int(seconds)
        except ValueError:
            raise ValueError(f"cell cap for {name.strip()!r} is not a whole number: {seconds!r}") from None
        if value <= 0:
            raise ValueError(f"cell cap for {name.strip()!r} must be positive, got {value}")
        caps[name.strip()] = value
    return caps


try:
    CELL_CAPS_S = parse_cell_caps(os.environ.get("NIGHTLY_CELL_CAPS_S", ""), CELL_CAPS_DEFAULT_S)
except ValueError as error:
    raise SystemExit(f"NIGHTLY_CELL_CAPS_S: {error}") from error


def parse_latency_bands(text: str) -> list[int]:
    """Parse a `50,100,150` list of RTT bands in ms. Empty text: lane off.

    A band is a round-trip time. The proxy delays each direction by half
    of it. The 999 ms cap keeps the band digits within the account-prefix
    budget (`latency_prefix`).
    """
    bands: list[int] = []
    for entry in text.split(","):
        entry = entry.strip()
        if not entry:
            continue
        try:
            value = int(entry)
        except ValueError:
            raise ValueError(f"latency band is not a whole number of ms: {entry!r}") from None
        if not 1 <= value <= 999:
            raise ValueError(f"latency band must be between 1 and 999 ms, got {value}")
        if value in bands:
            raise ValueError(f"latency band listed twice: {value}")
        bands.append(value)
    return bands


try:
    LATENCY_BANDS = parse_latency_bands(os.environ.get("NIGHTLY_LATENCY_BANDS", ""))
except ValueError as error:
    raise SystemExit(f"NIGHTLY_LATENCY_BANDS: {error}") from error
LATENCY_BRANCH = os.environ.get("NIGHTLY_LATENCY_BRANCH", "dev")
LATENCY_CELLS_FILE = os.environ.get("NIGHTLY_LATENCY_CELLS_FILE", "cells-latency.conf")
LATENCY_PROXY = HERE / "latency_proxy.py"
# A started band cell whose measured median queue delay misses the
# configured one-way delay by more than this is rewritten UNAVAILABLE:
# the proxy, not the product, failed the band contract.
LATENCY_HEALTH_TOLERANCE_MS = 10.0
LATENCY_CLASS_ORDER = ["latency-defect", "proxy-unavailable", "also-fails-at-0",
                       "no-baseline", "unavailable", "not-run", "pass"]


def cell_cap_s(cell: str) -> int:
    """The outer cap for one cell: its own cap, else the night's cap."""
    return CELL_CAPS_S.get(cell, CELL_CAP_S)


EXE = ".exe" if IS_WINDOWS else ""
# Runs of one commit on two platforms and jobs must not collide on the archive.
RUN_SUFFIX = os.environ.get("NIGHTLY_RUN_SUFFIX", JOB_DEFAULTS["run_suffix"])
# "local": a private engine copy this runner starts and stops (malbec).
# "remote": an engine that is already running; never started, stopped or changed.
ENGINE_MODE = os.environ.get("NIGHTLY_ENGINE", "remote" if IS_WINDOWS or IS_MAC else "local")
ENGINE_SCRIPT_REVISION = os.environ.get(
    "NIGHTLY_ENGINE_SCRIPT_REVISION", f"{REVISION}q" if JOB == "quest" and not IS_WINDOWS else REVISION)
ENGINE_HOST = os.environ.get("NIGHTLY_ENGINE_HOST", JOB_DEFAULTS["host"])
GAME_PORT = int(os.environ.get("NIGHTLY_GAME_PORT", str(JOB_DEFAULTS["game_port"])))
HTTP_PORT = int(os.environ.get("NIGHTLY_HTTP_PORT", str(JOB_DEFAULTS["http_port"])))
# The 289 engine's 40-byte /crc identity (ENGINE.md); empty skips the comparison.
ENGINE_CRC_SHA256 = os.environ.get("NIGHTLY_ENGINE_CRC_SHA256", DEFAULTS["engine_crc_sha256"])
ENGINE_LABEL = os.environ.get("NIGHTLY_ENGINE_LABEL", JOB_DEFAULTS["label"])
# Fresh exact-commit clone per run from local bare mirrors (Windows) into the
# branch's one stable tree path, instead of one reused checkout per branch
# (malbec). The stable path means Cargo overwrites last night's artefacts
# instead of adding a new set for every new source path.
FRESH_TREES = env_flag("NIGHTLY_FRESH_TREES", IS_WINDOWS)
# Disk guard: a Cargo target directory bigger than the cap is removed before
# the lanes and before the build, and the night is UNAVAILABLE when the drive
# still has less free space than the floor after that. 0 turns a check off.
# The Linux VM's disk is 100 GB in total and its tree path is already stable,
# so both checks are off there unless the variables are set.
TARGET_CAP_GB = int(os.environ.get("NIGHTLY_TARGET_CAP_GB", "40" if IS_WINDOWS else "0"))
MIN_FREE_GB = int(os.environ.get("NIGHTLY_MIN_FREE_GB", "100" if IS_WINDOWS else "0"))
# The BotTest console session is required for the GPU and the panel windows.
REQUIRE_CONSOLE = env_flag("NIGHTLY_REQUIRE_CONSOLE", IS_WINDOWS)
STEPS = [s for s in os.environ.get("NIGHTLY_STEPS", "lanes,panel,tui" if IS_WINDOWS else "tui").split(",") if s]
STEP_DEADLINE_S = {
    "lanes": int(os.environ.get("NIGHTLY_LANES_DEADLINE_S", "10800")),
    "build": int(os.environ.get("NIGHTLY_BUILD_DEADLINE_S", "5400")),
    "panel": int(os.environ.get("NIGHTLY_PANEL_DEADLINE_S", "12600")),
    "tui": int(os.environ.get("NIGHTLY_TUI_DEADLINE_S", "21600")),
    "latency": int(os.environ.get("NIGHTLY_LATENCY_DEADLINE_S", "3600")),
}
# The whole night: no step runs past it, so the summary and DONE are always written
# before the scheduled task's own limit (11 h). 0 = no bound (the Linux VM).
RUN_DEADLINE_S = int(os.environ.get("NIGHTLY_RUN_DEADLINE_S", "30600" if IS_WINDOWS else "0"))
_run_deadline = float("inf")


def step_deadline(name: str, start: float) -> float:
    return min(start + STEP_DEADLINE_S[name], _run_deadline)


def deadline_text(name: str) -> str:
    night = f", night {RUN_DEADLINE_S} s" if RUN_DEADLINE_S else ""
    return f"deadline (step {name} {STEP_DEADLINE_S[name]} s{night})"


PANEL_PARALLEL = int(os.environ.get("NIGHTLY_PANEL_PARALLEL", "3"))
# Cells whose name matches draw with the GPU renderer; every other cell gets BOT_CPU=1.
GPU_CELLS = re.compile(os.environ.get("NIGHTLY_GPU_CELLS", "^render_" if IS_WINDOWS else "(?!)"))
NIGHT_START = os.environ.get("NIGHTLY_START", "23:00")
ENGINE_PRISTINE = Path(os.environ.get("NIGHTLY_ENGINE_PRISTINE", str(JOB_DEFAULTS["pristine"])))
ENGINE_RUN = Path(os.environ.get("NIGHTLY_ENGINE_RUN", str(JOB_DEFAULTS["run"])))
CATALOG = Path(os.environ.get("NIGHTLY_CATALOG", str(ROOT / "rs2b0t")))
NAV_SNAPSHOT = Path(os.environ.get("NIGHTLY_NAV_SNAPSHOT", str(ROOT / "nav-snapshot")))
NAV_FIXTURE = Path(os.environ.get(
    "NIGHTLY_NAV_FIXTURE",
    str(ROOT / "fixtures" / "274bot-289-v17.navpack"),
))
# The old inspection capture the host-play route-inspect test compares
# against. The test's built-in default path exists only on merlot, so each
# other host keeps a copy with its read-only fixtures.
INSPECT_BASELINE = Path(os.environ.get(
    "NIGHTLY_INSPECT_BASELINE",
    str(ROOT / "fixtures" / "inspect.fb"),
))
RUNS_ROOT_OVERRIDE = os.environ.get("NIGHTLY_RUNS_ROOT")
def runs_root() -> Path:
    return Path(RUNS_ROOT_OVERRIDE) if RUNS_ROOT_OVERRIDE else ROOT / "runs"
QUIET_LOCK = Path(os.environ["NIGHTLY_QUIET_LOCK"]) if os.environ.get("NIGHTLY_QUIET_LOCK") else None
ISOHOME = os.environ.get("NIGHTLY_ISOHOME")
CATALOG_RS = Path("crates/scenario/src/catalog.rs")
RECEIPT_START = re.compile(r'\{"scenario":"')
ANSI = re.compile(r"\x1b(\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[()][0-9A-Za-z]|[=>78DEHMc])")
FAILURE_HINT = re.compile(r"(FAIL|panicked|error|Error|refus|timed out|deadline)")
PAIR_GATE_REFUSAL = "only qualifies under the pair core gate"
TEST_FAILED = re.compile(r"^test (\S+) \.\.\. FAILED\s*$", re.M)
TEST_RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", re.M)
# Lane features shared with the Windows platform gate (gate.ps1).
GATE_FEATURES = "host-play/live-harness,host-play/test-support,script/path-schema"
WINDOWS_SYSTEM_ENV = (
    "SYSTEMROOT", "SYSTEMDRIVE", "WINDIR", "COMSPEC", "PATHEXT", "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE", "PROCESSOR_IDENTIFIER", "PROCESSOR_LEVEL", "PROCESSOR_REVISION", "OS",
    "PROGRAMDATA", "PROGRAMFILES", "PROGRAMFILES(X86)", "PROGRAMW6432", "COMMONPROGRAMFILES",
    "COMMONPROGRAMFILES(X86)", "COMMONPROGRAMW6432", "PUBLIC", "DRIVERDATA", "USERNAME", "USERDOMAIN",
    "COMPUTERNAME", "ALLUSERSPROFILE", "PSMODULEPATH",
)
CREATE_NO_WINDOW = 0x08000000
CREATE_NEW_PROCESS_GROUP = 0x00000200

_stop = threading.Event()
_quiet_lock_reason: str | None = None


class QuietLockError(RuntimeError):
    """A MAC-QUIET lock stopped non-cell work before it could overlap a measurement."""


def note_quiet_lock() -> str | None:
    global _quiet_lock_reason
    if QUIET_LOCK is not None and QUIET_LOCK.exists() and _quiet_lock_reason is None:
        _quiet_lock_reason = f"MAC-QUIET.lock appeared at {QUIET_LOCK}"
        log(f"{_quiet_lock_reason}; no new cells will start and active cells will stop")
    return _quiet_lock_reason


def select_job_cells(cells: list[str], only: list[str] | None, job: str) -> list[str]:
    """Select the job's quest or non-quest cells without changing catalog order."""
    if job not in ("main", "quest"):
        raise ValueError(f"nightly job must be main or quest, got {job!r}")
    selected = cells if only is None else [cell for cell in cells if cell in only]
    quest_job = job == "quest"
    return [cell for cell in selected if cell.startswith("quester_") == quest_job]


def isolated_command(cmd: list[str]) -> list[str]:
    """Run Cargo and frontend commands through the configured isolated-HOME wrapper."""
    if ISOHOME:
        return [ISOHOME, *cmd]
    return cmd
_children: dict[int, Callable[[], None]] = {}
_children_lock = threading.Lock()


def log(message: str) -> None:
    print(f"{now().isoformat(timespec='seconds')} {message}", flush=True)


def now() -> dt.datetime:
    return dt.datetime.now(TIMEZONE) if TIMEZONE else dt.datetime.now().astimezone()


def now_iso() -> str:
    return now().isoformat(timespec="seconds")


def rtext(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def wtext(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")


def platform_id() -> dict:
    os_name = platform.system().lower()
    machine = {"amd64": "x86_64", "arm64": "aarch64"}.get(platform.machine().lower(), platform.machine())
    pretty = os_name
    os_release = Path("/etc/os-release")
    if IS_WINDOWS:
        pretty = f"Windows {platform.release()} ({platform.version()})"
    elif os_release.is_file():
        for line in os_release.read_text().splitlines():
            if line.startswith("PRETTY_NAME="):
                pretty = line.split("=", 1)[1].strip('"')
    return {
        "platform": f"{os_name}-{machine}",
        "os": pretty,
        "host": platform.node(),
    }


def kill_tree(pid: int, proc: subprocess.Popen | None = None) -> None:
    """Stop a recorded child and everything it started (POSIX: its process group)."""
    if IS_WINDOWS:
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=False)
        return
    try:
        os.killpg(pid, signal.SIGTERM)
        try:
            if proc is not None:
                proc.wait(15)
        except subprocess.TimeoutExpired:
            os.killpg(pid, signal.SIGKILL)
            proc.wait()
    except ProcessLookupError:
        pass



def stop_cell_process(pid: int, proc: subprocess.Popen | None = None) -> None:
    """Stop only a recorded live-cell process group after MAC-QUIET appears."""
    with _children_lock:
        recorded = pid in _children
    if not recorded:
        return
    if IS_WINDOWS:
        kill_tree(pid, proc)
        return
    try:
        os.killpg(pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    if proc is not None:
        try:
            proc.wait(timeout=1)
        except subprocess.TimeoutExpired:
            pass
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    if proc is not None:
        proc.wait()


def popen_group(cmd: list[str], **kwargs) -> subprocess.Popen:
    """Start a child in its own process group so kill_tree reaches its descendants."""
    if IS_WINDOWS:
        kwargs["creationflags"] = kwargs.get("creationflags", 0) | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
    else:
        kwargs["start_new_session"] = True
    return subprocess.Popen(cmd, **kwargs)


def track(pid: int, kill: Callable[[], None]) -> None:
    with _children_lock:
        _children[pid] = kill


def untrack(pid: int) -> None:
    with _children_lock:
        _children.pop(pid, None)


def run(cmd: list[str], *, cwd: Path | None = None, env: dict | None = None,
        log_to: Path | None = None, check: bool = True, timeout: float | None = None,
        quiet_sensitive: bool = True) -> subprocess.CompletedProcess:
    """Run a command; stop non-cell work if a Mac timing lock appears."""
    if cmd and Path(cmd[0]).name == "cargo":
        cmd = isolated_command(cmd)
    if log_to is None:
        return subprocess.run(cmd, cwd=cwd, env=env, check=check, text=True, encoding="utf-8",
                              errors="replace", stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              timeout=timeout)
    if quiet_sensitive and note_quiet_lock():
        raise QuietLockError(_quiet_lock_reason)
    with log_to.open("a", encoding="utf-8") as out:
        out.write(f"$ {' '.join(cmd)}\n")
        out.flush()
        proc = popen_group(cmd, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=out,
                           stderr=subprocess.STDOUT)
        track(proc.pid, lambda: kill_tree(proc.pid, proc))
        started = time.monotonic()
        try:
            while True:
                wait_s = 5.0 if timeout is None else min(5.0, timeout - (time.monotonic() - started))
                if wait_s <= 0:
                    kill_tree(proc.pid, proc)
                    proc.wait()
                    raise subprocess.TimeoutExpired(cmd, timeout)
                try:
                    proc.wait(timeout=wait_s)
                    break
                except subprocess.TimeoutExpired:
                    if quiet_sensitive and note_quiet_lock():
                        kill_tree(proc.pid, proc)
                        proc.wait()
                        raise QuietLockError(_quiet_lock_reason)
                    if timeout is not None and time.monotonic() - started >= timeout:
                        kill_tree(proc.pid, proc)
                        proc.wait()
                        raise subprocess.TimeoutExpired(cmd, timeout)
        finally:
            untrack(proc.pid)
    if check and proc.returncode:
        raise subprocess.CalledProcessError(proc.returncode, cmd)
    return subprocess.CompletedProcess(cmd, proc.returncode)


def git_cmd(*args: str) -> list[str]:
    # Anonymous only: never consult or store credentials.
    return ["git", "-c", "credential.helper=", *args]


def git_env() -> dict:
    return dict(os.environ, GIT_TERMINAL_PROMPT="0", GCM_INTERACTIVE="never")


# A Windows stat result has this bit in `st_file_attributes` for a junction,
# a directory symlink or any other reparse point.
_REPARSE_POINT = stat.FILE_ATTRIBUTE_REPARSE_POINT


def _kind(info: os.stat_result, link: bool) -> str:
    """"link", "dir" or "file" for one `lstat` result.

    `link` is the caller's symlink and junction check. A directory with the
    Windows reparse-point bit is a link too. The disk code never walks into
    a link, never measures what it points to and never changes its
    permissions. It removes the link itself.
    """
    if link or stat.S_ISLNK(info.st_mode):
        return "link"
    if stat.S_ISDIR(info.st_mode):
        if getattr(info, "st_file_attributes", 0) & _REPARSE_POINT:
            return "link"
        return "dir"
    return "file"


def _path_kind(path: Path) -> tuple[str, os.stat_result]:
    isjunction = getattr(os.path, "isjunction", None)
    info = os.lstat(path)
    return _kind(info, os.path.islink(path) or bool(isjunction and isjunction(path))), info


def _entry_kind(entry: os.DirEntry) -> tuple[str, os.stat_result]:
    is_junction = getattr(entry, "is_junction", None)
    info = entry.stat(follow_symlinks=False)
    return _kind(info, entry.is_symlink() or bool(is_junction and is_junction())), info


def _walk(top: Path, top_info: os.stat_result, *, prepare: Callable[[Path], None] | None = None,
          onerror: Callable[[OSError], None] | None = None
          ) -> Iterator[tuple[Path, str, os.stat_result]]:
    """Yield each entry under the real directory `top`, then `top` itself.

    This is the one traversal that `dir_size` and `rmtree` share. Each item
    is `(path, kind, lstat result)`, with kind "link", "dir" or "file" (see
    `_kind`). A directory comes after everything in it, so the caller can
    remove each item when it arrives. The walk goes only into real
    directories. A symlink or a junction comes out as one "link" item and
    the walk never goes into it, so every item is inside `top`.

    `prepare(folder)` runs on each real directory before the walk lists it.
    An error from `prepare`, from a listing or from an `lstat` goes to
    `onerror`, and the walk continues without that part. With no `onerror`
    the error is raised. An entry that is gone before the walk reaches it
    is skipped.
    """
    def listing(folder: Path) -> Iterator[os.DirEntry]:
        try:
            if prepare is not None:
                prepare(folder)
            with os.scandir(folder) as entries:
                return iter(list(entries))
        except FileNotFoundError:
            return iter(())
        except OSError as error:
            if onerror is None:
                raise
            onerror(error)
            return iter(())

    # An explicit stack, so a deep tree cannot raise RecursionError.
    stack = [(top, top_info, listing(top))]
    while stack:
        folder, info, entries = stack[-1]
        entry = next(entries, None)
        if entry is None:
            stack.pop()
            yield folder, "dir", info
            continue
        path = Path(entry.path)
        try:
            kind, child_info = _entry_kind(entry)
        except FileNotFoundError:
            continue
        except OSError as error:
            if onerror is None:
                raise
            onerror(error)
            continue
        if kind == "dir":
            stack.append((path, child_info, listing(path)))
        else:
            yield path, kind, child_info


def _add_permissions(path: Path, bits: int) -> None:
    """Add `bits` to the mode of one real file or directory.

    `lstat` decides what `path` is. A link is refused, so a fix never goes
    through a symlink or a junction to the thing that it points to. The
    callers pass only `rmtree`'s own path or paths that `_walk` reached, so
    every fix is inside the tree that is being removed. Bits are only
    added, never taken away.
    """
    kind, info = _path_kind(path)
    if kind == "link":
        raise OSError(f"refusing to change permissions through a link: {path}")
    mode = stat.S_IMODE(info.st_mode)
    if mode & bits == bits:
        return
    if os.chmod in os.supports_follow_symlinks:
        os.chmod(path, mode | bits, follow_symlinks=False)
    else:
        os.chmod(path, mode | bits)


def _make_listable(folder: Path) -> None:
    # Owner read, write and search: list the folder and remove its entries.
    _add_permissions(folder, stat.S_IRWXU)


def _remove_link(path: Path, info: os.stat_result) -> None:
    """Remove a symlink or a junction itself, never the thing it points to."""
    if stat.S_ISDIR(info.st_mode):
        # A Windows junction or another directory reparse point. rmdir
        # removes only the link. It never removes the contents of a folder.
        os.rmdir(path)
    else:
        os.unlink(path)


def _remove_file(path: Path) -> None:
    try:
        os.unlink(path)
    except FileNotFoundError:
        return
    except OSError:
        # Windows refuses to unlink a read-only file (git's object files).
        # Add owner write and try once more.
        _add_permissions(path, stat.S_IWRITE)
        os.unlink(path)


def rmtree(path: Path) -> None:
    """Remove a file, a link or a directory tree.

    The removal uses `_walk`, so it never goes into a symlink or a junction,
    at the top or nested. Each link is removed as a link, and nothing
    outside `path` is changed or removed. Each real directory gets owner
    read, write and search before the walk lists it. A file that cannot be
    unlinked gets owner write and one more try. The removal continues past
    a failure and removes what it can. Raise one OSError when any part
    could not be removed. Return only when `path` no longer exists.
    """
    try:
        kind, info = _path_kind(path)
    except FileNotFoundError:
        return
    failures: list[OSError] = []
    if kind == "dir":
        items = _walk(path, info, prepare=_make_listable, onerror=failures.append)
    else:
        items = iter([(path, kind, info)])
    for item, item_kind, item_info in items:
        try:
            if item_kind == "link":
                _remove_link(item, item_info)
            elif item_kind == "file":
                _remove_file(item)
            else:
                os.rmdir(item)
        except FileNotFoundError:
            pass
        except OSError as error:
            failures.append(error)
    if failures:
        raise OSError(f"could not remove {path}: {failures[0]}")
    if os.path.lexists(path):
        raise OSError(f"could not remove {path}: the path still exists")


def read_list(path: Path) -> list[str]:
    rows = []
    for line in rtext(path).splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            rows.append(line)
    return rows


def cells_file_path(arg: Path | None, env: str | None) -> Path | None:
    """The cells file named by `--cells-file` or NIGHTLY_CELLS_FILE, if any.

    A relative path is read beside this script, so a guest's settings file
    can name a committed list (`cells-274.conf`) without a guest path.
    """
    raw = arg if arg is not None else (Path(env) if env else None)
    if raw is None:
        return None
    return raw if raw.is_absolute() else HERE / raw


def read_skips(path: Path) -> dict[str, str]:
    skips = {}
    for row in read_list(path):
        name, _, reason = row.partition(" ") if " " in row else row.partition("\t")
        skips[name.strip()] = reason.strip() or "listed in skip.conf"
    return skips


def slug(branch: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]", "-", branch)


def catalog_cells(tree: Path) -> list[str]:
    """Scenario names from the checked-out catalog's REGISTRY table, in order."""
    text = rtext(tree / CATALOG_RS)
    start = text.index("const REGISTRY: &[Entry] = &[")
    end = text.index("\n];", start)
    block = text[start:end]
    names = re.findall(r'Entry::(?:new|fallible)\(\s*"([A-Za-z0-9_]+)"', block)
    entries = len(re.findall(r"Entry::(?:new|fallible)\(", block))
    if not names or len(names) != entries:
        raise RuntimeError(
            f"{CATALOG_RS}: parsed {len(names)} names from {entries} REGISTRY entries; "
            "the registry format changed, update catalog_cells()")
    return names


# ---------------------------------------------------------------- disk guard

GB = 1024 ** 3


def fmt_gb(nbytes: int) -> str:
    return f"{nbytes / GB:.1f}"


def dir_size(path: Path) -> int:
    """Total size of the real files under `path` in bytes; 0 when it does not exist.

    The size comes from `_walk`, the same traversal that `rmtree` uses. It
    never follows a symlink or a junction. A link counts as 0 bytes, at the
    top or nested, and the thing that it points to is not measured. Raise
    OSError when any part of the tree cannot be listed or measured. A
    partial total is never returned as if it were the true size.
    """
    try:
        kind, info = _path_kind(path)
    except FileNotFoundError:
        return 0
    if kind == "link":
        return 0
    if kind == "file":
        return info.st_size
    return sum(item_info.st_size for _item, item_kind, item_info in _walk(path, info)
               if item_kind == "file")


def free_bytes(path: Path) -> int:
    """Free space on the drive that holds `path` (or its nearest existing parent)."""
    probe = path
    while not probe.exists() and probe.parent != probe:
        probe = probe.parent
    return shutil.disk_usage(probe).free


def disk_guard(targets: list[Path], cap_bytes: int | None = None, min_free_bytes: int | None = None,
               *, size_of=None, free_of=None, remove=None) -> str | None:
    """Keep the Cargo target directories from filling the disk.

    Remove a target that is bigger than the cap (`NIGHTLY_TARGET_CAP_GB`).
    A target whose size cannot be read completely counts as bigger than
    the cap and is removed too. The target folders are disposable Cargo
    output, and a partial size must never keep an over-cap folder alive.
    If the drive then still has less free space than the floor
    (`NIGHTLY_MIN_FREE_GB`), remove the remaining targets too, the first one
    first. Return the reason the night is UNAVAILABLE when free space is
    still below the floor after that, else None. A cap or floor of 0 turns
    that check off (the Linux defaults, so nothing is measured or removed).
    Raise OSError when a removal fails or leaves the target in place.
    `guard_disk` turns that error into an UNAVAILABLE night.
    """
    if cap_bytes is None:
        cap_bytes = TARGET_CAP_GB * GB
    if min_free_bytes is None:
        min_free_bytes = MIN_FREE_GB * GB
    if cap_bytes <= 0 and min_free_bytes <= 0:
        return None
    size_of = size_of or dir_size
    free_of = free_of or free_bytes
    remove = remove or rmtree
    free = free_of(targets[0])
    removed: set[Path] = set()

    cap_text = f"{fmt_gb(cap_bytes)} GB" if cap_bytes > 0 else "off"
    floor_text = f"{fmt_gb(min_free_bytes)} GB" if min_free_bytes > 0 else "off"

    def drop(target: Path, size: int | None, why: str) -> None:
        nonlocal free
        before = free
        remove(target)
        if os.path.lexists(target):
            raise OSError(f"disk guard could not remove {target}: the path still exists")
        removed.add(target)
        free = free_of(target)
        size_text = f"{fmt_gb(size)} GB" if size is not None else "unknown"
        log(f"disk guard: removed {target} ({why}: size {size_text}, cap {cap_text}, "
            f"free {fmt_gb(before)} -> {fmt_gb(free)} GB, floor {floor_text})")

    for target in targets:
        if cap_bytes > 0 and os.path.lexists(target):
            try:
                size = size_of(target)
            except OSError:
                drop(target, None, "size could not be read, counted as over the cap")
                continue
            if size > cap_bytes:
                drop(target, size, "over the cap")
    for target in targets:
        if min_free_bytes <= 0 or free >= min_free_bytes:
            break
        if target not in removed and os.path.lexists(target):
            try:
                size = size_of(target)
            except OSError:
                size = None
            drop(target, size, "free space below the floor")
    if min_free_bytes > 0 and free < min_free_bytes:
        return (f"not enough free disk space: {fmt_gb(free)} GB free on the drive that holds "
                f"{targets[0]}, the floor is {fmt_gb(min_free_bytes)} GB (NIGHTLY_MIN_FREE_GB); "
                "nothing was built")
    return None


def tree_path(branch: str) -> Path:
    """The branch's one stable tree path, the same every night."""
    return ROOT / "trees" / slug(branch)


def branch_targets(branch: str) -> list[Path]:
    """The Cargo target directories of one branch: CI lanes, release build, nav bootstrap."""
    targets = ROOT / "target"
    return [targets / f"{slug(branch)}-ci", targets / slug(branch), targets / "navskip"]


def guard_disk(branch: str, primary: Path) -> str | None:
    """`disk_guard` over the branch's targets, the next step's target (`primary`) first.

    A guard error (for example a target folder that cannot be removed)
    fails closed: the night is UNAVAILABLE with that reason instead of
    building into a disk the guard could not measure or clear.
    """
    try:
        return disk_guard([primary] + [target for target in branch_targets(branch) if target != primary])
    except OSError as error:
        return f"disk guard failed: {error}; nothing was built"


# ---------------------------------------------------------------- sources


def update_tree(branch: str, tree: Path, build_log: Path) -> tuple[str, str]:
    env = git_env()
    if not (tree / ".git").exists():
        tree.parent.mkdir(parents=True, exist_ok=True)
        run(git_cmd("clone", "--no-checkout", REPO_URL, str(tree)), env=env, log_to=build_log)
    git = git_cmd("-C", str(tree))
    run(git + ["remote", "set-url", "origin", REPO_URL], env=env, log_to=build_log)
    run(git + ["fetch", "--prune", "origin", f"+refs/heads/{branch}:refs/remotes/origin/{branch}"],
        env=env, log_to=build_log)
    run(git + ["checkout", "--force", "--detach", f"origin/{branch}"], env=env, log_to=build_log)
    run(git + ["submodule", "sync", "--recursive"], env=env, log_to=build_log)
    run(git + ["submodule", "update", "--init", "--recursive", "--force"], env=env, log_to=build_log)
    run(git + ["clean", "-ffdxq"], env=env, log_to=build_log)
    run(git + ["submodule", "foreach", "--recursive", "git clean -ffdxq"], env=env, log_to=build_log)
    full = run(git + ["rev-parse", "HEAD"]).stdout.strip()
    return full, full[:9]


def mirror_fetch(url: str, mirror: Path, refspecs: list[str], build_log: Path) -> None:
    env = git_env()
    if not (mirror / "HEAD").exists():
        mirror.parent.mkdir(parents=True, exist_ok=True)
        run(git_cmd("init", "--bare", "-q", str(mirror)), env=env, log_to=build_log)
    run(git_cmd("-C", str(mirror), "fetch", "-q", "--no-tags", url, *refspecs), env=env, log_to=build_log)


def has_commit(repo: Path, sha: str) -> bool:
    return run(git_cmd("-C", str(repo), "cat-file", "-e", f"{sha}^{{commit}}"), check=False).returncode == 0

# ---------------------------------------------------------------- rs2b0t API drift


# The compat contract is generated from upstream's rs2b0t-api package at the
# frozen revision named in the tested tree's abi-contract.json. A change in
# either file on upstream main is the signal that a refreeze is due. The
# check is informational: it never changes a cell's outcome or the run's
# exit status.
RS2B0T_API_FILES = ("packages/rs2b0t-api/index.d.ts", "packages/rs2b0t-api/index.js")
RS2B0T_UPSTREAM_REF = "refs/nightly/upstream-main"
# One anonymous fetch of a single branch tip. 120 s bounds a stalled network
# without consuming a step deadline; there is no retry. A miss reports
# UNAVAILABLE and the next night tries again.
RS2B0T_FETCH_TIMEOUT_S = 120
RS2B0T_MAX_COMMITS = 10
ABI_CONTRACT_REL = Path("crates/script-js/compat-js/abi-contract.json")
# One fetch per nightly run, not per branch: branches in one process share
# the fetched tip through this cache (success and failure alike).
_api_upstream: dict[tuple[str, str], tuple[str | None, str]] = {}


def read_api_freeze(tree: Path) -> tuple[str, str]:
    """The frozen rs2b0t repository URL and revision from the tested tree.

    Both come from the tree's `abi-contract.json`, never from a constant,
    so the check follows the freeze the compat contract was built from.
    """
    try:
        frozen = json.loads(rtext(tree / ABI_CONTRACT_REL))["frozen_source"]
        return frozen["repository"], frozen["revision"]
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise RuntimeError(f"freeze revision not read from {ABI_CONTRACT_REL}: {error}") from error


def _short_line(text: object, limit: int = 200) -> str:
    """One line of at most `limit` characters for a summary reason."""
    return " ".join(str(text).split())[:limit] or "unknown reason"


def _run_fetch_group(cmd: list[str], env: dict | None, timeout: float | None) -> subprocess.CompletedProcess:
    """Run one fetch in its own process group and capture its output.

    POSIX (`start_new_session=True` in `popen_group`) puts the fetch in a new
    session, so `kill_tree` (`killpg`) reaches `git-remote-https` and any other
    child. This is safe because the new group holds only this fetch subtree;
    the runner keeps its own group. Windows (`CREATE_NEW_PROCESS_GROUP` in
    `popen_group`) isolates the fetch the same way and `kill_tree`
    (`taskkill /T /F`) ends the fetch tree by PID, so only that subtree stops.
    The caller polls `note_quiet_lock` (the runner's existing lock check)
    while the fetch runs; a lock or the timeout kills the whole group.
    """
    proc = popen_group(cmd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    track(proc.pid, lambda: kill_tree(proc.pid, proc))
    started = time.monotonic()
    out = ""
    try:
        while True:
            remaining = None if timeout is None else timeout - (time.monotonic() - started)
            if remaining is not None and remaining <= 0:
                kill_tree(proc.pid, proc)
                proc.wait()
                out, _ = proc.communicate()
                raise subprocess.TimeoutExpired(cmd, timeout, output=out)
            wait_s = 5.0 if remaining is None else min(5.0, remaining)
            try:
                out, _ = proc.communicate(timeout=wait_s)
                break
            except subprocess.TimeoutExpired:
                if note_quiet_lock():
                    kill_tree(proc.pid, proc)
                    proc.wait()
                    out, _ = proc.communicate()
                    raise QuietLockError(f"quiet lock held: {_quiet_lock_reason}") from None
                if timeout is not None and time.monotonic() - started >= timeout:
                    kill_tree(proc.pid, proc)
                    proc.wait()
                    out, _ = proc.communicate()
                    raise subprocess.TimeoutExpired(cmd, timeout, output=out)
    finally:
        untrack(proc.pid)
    return subprocess.CompletedProcess(cmd, proc.returncode, out, None)


def fetch_upstream_main(catalog: Path, repository: str,
                        timeout: float = RS2B0T_FETCH_TIMEOUT_S) -> str:
    """Fetch upstream `main` into `refs/nightly/upstream-main`; return the tip.

    The fetch moves no branch and no checkout: it only adds objects and that
    one namespaced ref, so the catalog's checked-out commit never changes.
    One fetch per process: later branches reuse the cached tip or failure.
    A held quiet lock stops the fetch before it starts; the timeout and a lock
    that appears during the fetch stop the whole process group.
    """
    key = (str(catalog), repository)
    if key not in _api_upstream:
        try:
            locked = note_quiet_lock()
            if locked:
                raise QuietLockError(f"quiet lock held: {locked}")
            fetched = _run_fetch_group(git_cmd("-C", str(catalog), "fetch", "--no-tags", repository,
                                              f"+refs/heads/main:{RS2B0T_UPSTREAM_REF}"),
                                      env=git_env(), timeout=timeout)
            if fetched.returncode != 0:
                detail = _short_line(fetched.stdout)
                suffix = f": {detail}" if detail != "unknown reason" else ""
                raise RuntimeError(f"fetch failed (exit {fetched.returncode}){suffix}")
            tip = run(git_cmd("-C", str(catalog), "rev-parse", f"{RS2B0T_UPSTREAM_REF}^{{commit}}"),
                      env=git_env(), check=False).stdout.strip()
            if not tip:
                raise RuntimeError("fetch ok but the upstream ref is empty")
        except subprocess.TimeoutExpired:
            _api_upstream[key] = (None, f"fetch timed out after {timeout:g} s")
        except (OSError, RuntimeError) as error:
            _api_upstream[key] = (None, _short_line(error))
        else:
            _api_upstream[key] = (tip, "")
    tip, reason = _api_upstream[key]
    if tip is None:
        raise RuntimeError(reason)
    return tip


def _api_unavailable(repository: str | None, freeze_rev: str | None, reason: object) -> dict:
    """The `rs2b0t_api` metadata for a check that did not run to a diff."""
    return {
        "status": "unavailable",
        "repository": repository,
        "freeze_revision": freeze_rev,
        "freeze_short": (freeze_rev or "")[:8] or None,
        "upstream_revision": None,
        "upstream_short": None,
        "files": {},
        "commits": [],
        "commit_count": 0,
        "reason": _short_line(reason),
    }


def check_rs2b0t_api_drift(tree: Path | None, catalog: Path,
                           timeout: float = RS2B0T_FETCH_TIMEOUT_S) -> dict:
    """Compare the frozen rs2b0t API files against upstream `main`.

    The freeze revision comes from the tested tree's `abi-contract.json`.
    Upstream `main` is fetched once per process into the catalog checkout
    (see `fetch_upstream_main`). The result feeds `summary.json`
    (`rs2b0t_api`) and one `summary.md` line. It never raises: every
    failure reports `unavailable`, never `unchanged`.
    """
    try:
        return _check_rs2b0t_api_drift(tree, catalog, timeout)
    except Exception as error:  # informational only; the night runs on
        return _api_unavailable(None, None, f"check failed: {error}")


def _check_rs2b0t_api_drift(tree: Path | None, catalog: Path, timeout: float) -> dict:
    if tree is None:
        return _api_unavailable(None, None, "no tested tree: the sources step did not produce one")
    try:
        repository, freeze_rev = read_api_freeze(tree)
    except RuntimeError as error:
        return _api_unavailable(None, None, str(error))
    if not repository or not freeze_rev:
        return _api_unavailable(repository or None, freeze_rev or None,
                                "abi-contract.json names no frozen repository or revision")
    try:
        tip = fetch_upstream_main(catalog, repository, timeout)
    except RuntimeError as error:
        return _api_unavailable(repository, freeze_rev, str(error))
    base = {
        "repository": repository,
        "freeze_revision": freeze_rev,
        "freeze_short": freeze_rev[:8],
        "upstream_revision": tip,
        "upstream_short": tip[:8],
    }
    if not has_commit(catalog, freeze_rev):
        return {**base, "status": "unavailable", "files": {}, "commits": [], "commit_count": 0,
                "reason": f"freeze revision {freeze_rev[:8]} is not in the fetched history"}
    diff = run(git_cmd("-C", str(catalog), "diff", "--numstat", freeze_rev, tip,
                       "--", *RS2B0T_API_FILES), env=git_env(), check=False)
    if diff.returncode != 0:
        return {**base, "status": "unavailable", "files": {}, "commits": [], "commit_count": 0,
                "reason": f"comparison failed (exit {diff.returncode})"}
    files: dict[str, dict[str, int]] = {}
    for line in diff.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) != 3:
            continue
        try:
            added = int(parts[0])
        except ValueError:
            added = 0
        try:
            deleted = int(parts[1])
        except ValueError:
            deleted = 0
        files[parts[2]] = {"added": added, "deleted": deleted}
    if not files:
        return {**base, "status": "unchanged", "files": {}, "commits": [], "commit_count": 0}
    logged = run(git_cmd("-C", str(catalog), "log", "--format=%H%x00%s", f"{freeze_rev}..{tip}",
                         "--", *RS2B0T_API_FILES), env=git_env(), check=False)
    if logged.returncode != 0:
        detail = _short_line(logged.stdout)
        suffix = f": {detail}" if detail != "unknown reason" else ""
        return {**base, "status": "unavailable", "files": {}, "commits": [], "commit_count": 0,
                "reason": f"commit list failed (exit {logged.returncode}){suffix}"}
    entries = []
    for line in logged.stdout.splitlines():
        sha, _, subject = line.partition("\x00")
        if sha:
            entries.append({"sha": sha[:8], "subject": subject})
    return {**base, "status": "changed", "files": files,
            "commits": entries[:RS2B0T_MAX_COMMITS], "commit_count": len(entries)}


def api_drift_md_line(info: dict) -> str:
    """The one `summary.md` line for an `rs2b0t_api` metadata dict."""
    if info.get("status") == "unchanged":
        return (f"rs2b0t API: unchanged since {info['freeze_short']} "
                f"(upstream main {info['upstream_short']})")
    if info.get("status") == "changed":
        stats = ", ".join(f"{Path(path).name} +{counts['added']} -{counts['deleted']}"
                          for path, counts in info.get("files", {}).items())
        line = (f"rs2b0t API: CHANGED since freeze {info['freeze_short']} "
                f"(upstream main {info['upstream_short']}): {stats}")
        commits = info.get("commits") or []
        if commits:
            listed = ", ".join(f"{entry['sha']} {entry['subject']}" for entry in commits)
            extra = (info.get("commit_count") or len(commits)) - len(commits)
            if extra > 0:
                listed += f" (+{extra} more)"
            line += f"; commits: {listed}"
        return line
    return f"rs2b0t API: UNAVAILABLE ({info.get('reason') or 'unknown reason'})"


def fresh_tree(branch: str, build_log: Path) -> tuple[Path, str, str]:
    """Fetch the branch tip into the bare mirrors and clone it into the tree.

    The tree holds exactly that commit and the pinned submodule commits.
    Last night's tree is removed and the clone is made again at the same
    stable path (`trees/<branch>`), so no stale file can survive and Cargo
    sees the same package paths every night and overwrites its artefacts
    instead of adding a new set per source path. Only the mirrors and the
    one tree per branch persist between nights.
    """
    env = git_env()
    mirrors = ROOT / "mirror"
    root_mirror = mirrors / "274bot.git"
    mirror_fetch(REPO_URL, root_mirror, [f"+refs/heads/{branch}:refs/heads/{branch}"], build_log)
    full = run(git_cmd("-C", str(root_mirror), "rev-parse", f"refs/heads/{branch}^{{commit}}")).stdout.strip()
    tree = tree_path(branch)
    tree.parent.mkdir(parents=True, exist_ok=True)
    rmtree(tree)
    run(git_cmd("clone", "-q", "--no-checkout", str(root_mirror), str(tree)), env=env, log_to=build_log)
    git = git_cmd("-C", str(tree))
    run(git + ["checkout", "-q", "--detach", full], env=env, log_to=build_log)
    modules = run(git + ["config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.url$"],
                  check=False).stdout.splitlines()
    for line in modules:
        key, _, url = line.partition(" ")
        name = key[len("submodule."):-len(".url")]
        path = run(git + ["config", "-f", ".gitmodules", f"submodule.{name}.path"]).stdout.strip()
        pinned = run(git + ["ls-tree", full, path]).stdout.split()[2]
        sub_mirror = mirrors / f"{slug(name)}.git"
        if not (sub_mirror / "HEAD").exists() or not has_commit(sub_mirror, pinned):
            mirror_fetch(url, sub_mirror, ["+refs/heads/*:refs/heads/*"], build_log)
        if not has_commit(sub_mirror, pinned):
            mirror_fetch(url, sub_mirror, [pinned], build_log)
        run(git + ["submodule", "init", "--", path], env=env, log_to=build_log)
        run(git + ["config", f"submodule.{name}.url", str(sub_mirror)], env=env, log_to=build_log)
        run(git_cmd("-c", "protocol.file.allow=always", "-C", str(tree), "submodule", "update", "--force",
                    "--", path), env=env, log_to=build_log)
        got = run(git_cmd("-C", str(tree / path), "rev-parse", "HEAD")).stdout.strip()
        if got != pinned:
            raise RuntimeError(f"submodule {path}: HEAD {got} is not the pinned {pinned}")
    unset = [line for line in run(git + ["submodule", "status", "--recursive"]).stdout.splitlines()
             if line.startswith("-")]
    if unset:
        raise RuntimeError(f"uninitialised nested submodules: {unset}")
    if run(git + ["rev-parse", "HEAD"]).stdout.strip() != full:
        raise RuntimeError("tree HEAD is not the fetched commit")
    return tree, full, full[:9]


def prune_trees(branches: list[str]) -> None:
    """Remove every tree that is not a branch's stable tree path.

    This removes the per-commit trees (`trees/<branch>-<commit>-<stamp>`) the
    old fresh-tree scheme left behind, and the stable tree of a branch that
    is no longer tested. A removed tree is cloned again on its next night.
    """
    keep = {slug(branch) for branch in branches}
    trees = ROOT / "trees"
    for entry in trees.iterdir() if trees.is_dir() else []:
        if entry.is_dir() and entry.name not in keep:
            rmtree(entry)
            log(f"removed tree {entry.name} (not a branch's stable tree path)")


def build_env(target: Path, **extra: str) -> dict:
    # host-play's build script bakes the bundled navigation from the engine's content
    # and a decoded client-cache snapshot (BOT_NAV_SNAPSHOT_ROOT). The bake
    # defaults to revision 289; name the revision only for the other lane, so
    # the 289 build environment stays exactly what it was.
    # Keep persistent sccache writes outside the disposable build HOME.
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), ENGINE_DIR=str(ENGINE_PRISTINE / "engine"),
               BOT_NAV_SNAPSHOT_ROOT=str(NAV_SNAPSHOT), SCCACHE_DIR=str(ROOT / "sccache"), **extra)
    if REVISION != "289":
        env["BOT_NAV_REVISION"] = REVISION
    return env


def frontend_bins() -> list[str]:
    return ["tui-play", "panel-play"] if "panel" in STEPS else ["tui-play"]


def cargo_frontends(tree: Path, env: dict, build_log: Path, bins: list[str], timeout: float | None) -> None:
    cmd = ["cargo", "build", "--locked", "--release"]
    for name in bins:
        cmd += ["-p", "tui" if name == "tui-play" else "panel", "--bin", name]
    run(cmd, cwd=tree, env=env, log_to=build_log, timeout=timeout)


def snapshot_ready() -> bool:
    return any(NAV_SNAPSHOT.glob("*/versionlist"))


def bootstrap_nav_snapshot(tree: Path, run_dir: Path, build_log: Path, timeout: float | None) -> None:
    """Create the decoded client-cache snapshot the navigation bake reads.

    The snapshot is what the client itself decodes from the engine's cache on
    first login. Build a tui-play without bundled navigation, let one cell log
    in against a fresh engine, and keep the snapshot it decoded.
    """
    log("nav snapshot missing or stale: bootstrapping it from a navigation-free build")
    skip_target = ROOT / "target" / "navskip"
    cargo_frontends(tree, build_env(skip_target, BOT_NAV_BUILD="skip"), build_log, ["tui-play"], timeout)
    work = ROOT / "work" / "nav-snapshot"
    rmtree(work)
    home, tmp = work / "home", work / "tmp"
    home.mkdir(parents=True)
    tmp.mkdir(parents=True)
    ok, detail = engine_start(run_dir)
    if not ok:
        raise RuntimeError(f"nav snapshot bootstrap: engine unavailable: {detail}")
    try:
        run_tui(isolated_command(tui_cmd(skip_target / "release" / f"tui-play{EXE}", "walk")),
                cell_env(home, tmp, work / "evidence", "walk", NAME_PREFIX), work, work / "stderr.log", 600)
    finally:
        engine_stop(None)
    decoded = [path.parent for path in
               (home / ".274bot" / UNPACK_DIR).glob(f"revision-{REVISION}/*/*/*/versionlist")]
    if not decoded:
        raise RuntimeError("nav snapshot bootstrap: the client decoded no snapshot; "
                           f"see {work / 'stderr.log'}")
    staging = NAV_SNAPSHOT.with_name(NAV_SNAPSHOT.name + ".new")
    rmtree(staging)
    staging.mkdir(parents=True)
    for path in decoded:
        shutil.copytree(path, staging / path.name)
    rmtree(NAV_SNAPSHOT)
    staging.rename(NAV_SNAPSHOT)
    rmtree(work)
    log(f"nav snapshot ready: {[path.name for path in decoded]}")


def apply_274_inventory_overlay(tree: Path) -> None:
    """Use the pinned-content inventory for the revision-274 nav bake.

    The tested branch's manifest includes a test-only script absent from the
    pinned 274 content commit. Keep the lane-specific correction in the runner
    checkout and overlay it only on revision-274 build trees.
    """
    if REVISION != "274":
        return
    target = tree / "crates" / "nav-planner" / "src" / "required-content-274.tsv"
    if not NAV_274_REQUIRED_CONTENT.is_file():
        raise RuntimeError(f"revision-274 nav inventory missing: {NAV_274_REQUIRED_CONTENT}")
    if not target.is_file():
        raise RuntimeError(f"tested tree's revision-274 nav inventory missing: {target}")
    if NAV_274_REQUIRED_CONTENT.read_bytes() != target.read_bytes():
        shutil.copyfile(NAV_274_REQUIRED_CONTENT, target)
        log("274: applied pinned-content nav inventory overlay")
    else:
        log("274: pinned-content nav inventory overlay already current")


def build(tree: Path, target: Path, run_dir: Path, build_log: Path) -> dict[str, Path]:
    apply_274_inventory_overlay(tree)
    deadline = step_deadline("build", time.monotonic())
    remaining = lambda: max(60.0, deadline - time.monotonic())  # noqa: E731
    bins = frontend_bins()
    if not snapshot_ready():
        bootstrap_nav_snapshot(tree, run_dir, build_log, remaining())
    try:
        cargo_frontends(tree, build_env(target), build_log, bins, remaining())
    except subprocess.CalledProcessError:
        # A changed engine cache invalidates the snapshot; refresh it once and retry.
        if "decoded navigation cache" not in rtext(build_log)[-20000:]:
            raise
        bootstrap_nav_snapshot(tree, run_dir, build_log, remaining())
        cargo_frontends(tree, build_env(target), build_log, bins, remaining())
    return {name: target / "release" / f"{name}{EXE}" for name in bins}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_source_manifest(run_dir: Path) -> None:
    """Write hashes for every regular run file before publishing DONE."""
    files = sorted(
        (path for path in run_dir.rglob("*") if path.is_file() and not path.is_symlink()
         and path.name != "SHA256SUMS"),
        key=lambda path: path.relative_to(run_dir).as_posix(),
    )
    rows = [f"{sha256(path)}  {path.relative_to(run_dir).as_posix()}" for path in files]
    wtext(run_dir / "SHA256SUMS", "\n".join(rows) + "\n")


# ---------------------------------------------------------------- engine and session


def engine_check() -> tuple[bool, str]:
    """A running engine answers /crc with its identity and accepts the game port."""
    url = f"http://{ENGINE_HOST}:{HTTP_PORT}/crc"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(url, timeout=15) as response:
            body = response.read()
    except OSError as error:
        return False, f"{url}: {error}"
    digest = hashlib.sha256(body).hexdigest()
    if ENGINE_CRC_SHA256 and (len(body) != 40 or digest != ENGINE_CRC_SHA256):
        return False, (f"identity mismatch: {url}: {len(body)} bytes, sha256 {digest}, "
                       f"expected 40 bytes {ENGINE_CRC_SHA256}")
    try:
        socket.create_connection((ENGINE_HOST, GAME_PORT), timeout=15).close()
    except OSError as error:
        return False, f"{ENGINE_HOST}:{GAME_PORT}: {error}"
    return True, f"{ENGINE_LABEL} at {ENGINE_HOST}:{GAME_PORT}/{HTTP_PORT}, /crc sha256 {digest[:16]}"


def remote_engine_failure(detail: str) -> str:
    """Keep an answered HTTP identity mismatch separate from an unreachable engine."""
    if detail.startswith("identity mismatch:"):
        return f"{ENGINE_LABEL} {detail}"
    return f"{ENGINE_LABEL} not reachable: {detail}"


def copy_engine(run_dir: Path | None) -> None:
    if IS_WINDOWS:
        rmtree(ENGINE_RUN)
        shutil.copytree(ENGINE_PRISTINE, ENGINE_RUN)
    else:
        run(["rsync", "-a", "--delete", f"{ENGINE_PRISTINE}/", f"{ENGINE_RUN}/"],
            log_to=run_dir / "engine-control.log" if run_dir else None)


def engine_start(run_dir: Path) -> tuple[bool, str]:
    """Local: a fresh engine from the pristine copy (empty saves and account DB).

    Remote: a fresh copy of the engine data the clients read, and a check that
    the running engine answers; it is never started, stopped or changed.
    """
    if ENGINE_MODE == "remote":
        copy_engine(run_dir)
        return engine_check()
    engine_stop(None)
    try:
        copy_engine(run_dir)
        run([str(ENGINE_RUN / f"start-{ENGINE_SCRIPT_REVISION}.sh")], cwd=ENGINE_RUN,
            log_to=run_dir / "engine-control.log")
    except subprocess.CalledProcessError as error:
        return False, f"failed to start: {error}"
    return True, "fresh copy started, stopped after the run"


def engine_stop(run_dir: Path | None) -> None:
    if ENGINE_MODE == "remote" or not (ENGINE_RUN / "runtime" / "pids" / "engine.pid").exists():
        return
    control = run_dir / "engine-control.log" if run_dir else None
    run([str(ENGINE_RUN / f"stop-{ENGINE_SCRIPT_REVISION}.sh")], cwd=ENGINE_RUN, log_to=control, check=False,
        quiet_sensitive=False)
    if run_dir:
        engine_log = ENGINE_RUN / "runtime" / "logs" / "engine.log"
        if engine_log.is_file():
            shutil.copyfile(engine_log, run_dir / "engine.log")


CONSOLE_PS = r"""
$ErrorActionPreference = 'Stop'
$consoleUser = [string](Get-CimInstance Win32_ComputerSystem).UserName
$sessions = (query user 2>&1 | Out-String)
$pattern = '(?im)^\s*>?' + [regex]::Escape($env:USERNAME) + '\s+console\s+(\d+)\s+Active'
$m = [regex]::Match($sessions, $pattern)
[ordered]@{
  console_user = $consoleUser
  expected_user = "$env:COMPUTERNAME\$env:USERNAME"
  console_session_id = $(if ($m.Success) { [int]$m.Groups[1].Value } else { $null })
  own_session_id = [int](Get-Process -Id __PID__).SessionId
  query_user = $sessions.Trim()
} | ConvertTo-Json -Compress
"""


def console_session() -> tuple[bool, str, dict]:
    """The scenario driver's checks: this user is the console user, its console
    session is Active (the ID is read, not pinned), and this process runs in it."""
    if not REQUIRE_CONSOLE:
        return True, "console session not required on this platform", {}
    script = CONSOLE_PS.replace("__PID__", str(os.getpid()))
    try:
        out = run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], timeout=60).stdout
        facts = json.loads(out.strip().splitlines()[-1])
    except (subprocess.SubprocessError, OSError, ValueError, IndexError) as error:
        return False, f"BotTest console session not active (check failed: {error})", {}
    if str(facts.get("console_user", "")).lower() != str(facts.get("expected_user", "")).lower():
        return False, f"BotTest console session not active (console user is {facts.get('console_user')!r})", facts
    if facts.get("console_session_id") is None:
        return False, "BotTest console session not active (query user shows no Active console session)", facts
    if facts.get("own_session_id") != facts.get("console_session_id"):
        return False, (f"BotTest console session not active (runner is in session {facts.get('own_session_id')}, "
                       f"console is {facts.get('console_session_id')})"), facts
    return True, f"console session {facts['console_session_id']} active", facts


# ---------------------------------------------------------------- results


@dataclass
class CellResult:
    cell: str
    result: str  # PASS, FAIL, TIMEOUT, ERROR, UNAVAILABLE, NOT_RUN, SKIP
    branch: str
    commit: str
    platform: str
    frontend: str = "tui"
    renderer: str | None = None  # "cpu" (BOT_CPU=1) or "gpu"
    ticks: int | None = None
    wall_s: float | None = None
    first_failure: str | None = None
    exit_code: int | None = None
    reason: str | None = None
    gate: str | None = None  # "--pair-core" when the cell ran under the pair witness
    started: str | None = None
    latency_band: str | None = None  # "rtt50" on latency-lane rows, else None
    latency_rtt_ms: int | None = None
    latency_class: str | None = None  # script-computed; see classify_latency
    receipt: dict | None = field(default=None, repr=False)


@dataclass
class LaneResult:
    lane: str
    result: str  # PASS, FAIL, TIMEOUT, UNAVAILABLE, NOT_RUN
    branch: str
    commit: str
    platform: str
    command: str = ""
    exit_code: int | None = None
    wall_s: float | None = None
    tests: dict | None = None  # summed `test result:` lines
    failed_tests: list[str] = field(default_factory=list)
    first_failure: str | None = None
    reason: str | None = None
    started: str | None = None


def find_receipt(text: str, cell: str) -> dict | None:
    decoder = json.JSONDecoder()
    found = None
    for match in RECEIPT_START.finditer(text):
        try:
            value, _ = decoder.raw_decode(text, match.start())
        except json.JSONDecodeError:
            continue
        if value.get("scenario") == cell and value.get("outcome") in ("PASS", "FAIL", "UNAVAILABLE"):
            found = value
    return found


def record_receipt(result: CellResult, receipt: dict) -> None:
    result.receipt = receipt
    result.result = receipt["outcome"]
    result.ticks = receipt.get("ticks")
    if result.result == "FAIL":
        result.first_failure = (receipt.get("message") or receipt.get("predicate") or "")[:400]
    elif result.result == "UNAVAILABLE":
        result.reason = (receipt.get("message") or receipt.get("predicate") or "")[:400]


# A frontend with an UNAVAILABLE outcome still prints its plain FAIL line.
# It also prints the outcome on its own line (`FAIL: UNAVAILABLE: ...`).
# The JSON record is the authority. This line is the fallback when the
# record does not parse, so the outcome stays UNAVAILABLE in each frontend.
UNAVAILABLE_LINE = re.compile(r"^FAIL: (UNAVAILABLE: .+)$", re.M)


def verdict_line(text: str, cell: str) -> re.Match | None:
    """The plain PASS/FAIL verdict line for one cell.

    The panel prints the live name with its `script_` prefix. The TUI
    prints the bare cell name. Accept each form.
    """
    return re.search(rf"\b(PASS|FAIL): live (?:script_)?{re.escape(cell)}\b", text)


def apply_cell_outcome(result: CellResult, cell: str, frontend: str, combined: str, stderr_text: str,
                       out_text: str, timed_out: bool, cut_by_deadline: bool, cap_s: int) -> dict | None:
    """Set one finished cell's result from its output. Return its receipt.

    The parsed JSON record wins. Without it, a printed UNAVAILABLE line
    wins over the plain FAIL verdict line. The other fallbacks keep the
    runner's own states: TIMEOUT, NOT_RUN, the panel pair-gate
    UNAVAILABLE, and ERROR when no witness printed at all.
    """
    receipt = find_receipt(combined, cell)
    if receipt is not None:
        record_receipt(result, receipt)
        return receipt
    unavailable = UNAVAILABLE_LINE.search(combined)
    verdict = verdict_line(combined, cell)
    if unavailable is not None:
        result.result = "UNAVAILABLE"
        result.reason = unavailable.group(1).strip()[:400]
    elif verdict is not None:
        # The verdict line printed but its JSON record did not parse.
        result.result = verdict.group(1)
        result.reason = "verdict line without a parsable JSON record"
        if result.result == "FAIL":
            result.first_failure = first_failure_line(stderr_text) or verdict.group(0)
    elif timed_out:
        result.result = "TIMEOUT"
        bound = f"{frontend} step deadline reached" if cut_by_deadline else f"runner cap {cap_s}s reached"
        result.first_failure = (f"{bound} without a PASS/FAIL receipt; "
                                f"last: {first_failure_line(stderr_text) or '-'}")[:400]
    elif _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted while this cell ran"
    elif frontend == "panel" and PAIR_GATE_REFUSAL in stderr_text:
        result.result = "UNAVAILABLE"
        result.reason = ("paired proof: panel-play has no --pair-core gate; the tui step runs it under "
                         "--pair-core")
        result.first_failure = first_failure_line(stderr_text)
    else:
        result.result = "ERROR"
        result.first_failure = first_failure_line(stderr_text) or first_failure_line(out_text)
    return None


def first_failure_line(text: str) -> str | None:
    for line in text.splitlines():
        line = line.strip()
        if line and FAILURE_HINT.search(line):
            return line[:400]
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    return lines[-1][:400] if lines else None


# ---------------------------------------------------------------- cells


def private_dirs(home: Path, tmp: Path) -> dict:
    env = {"HOME": str(home), "TMPDIR": str(tmp)}
    if IS_WINDOWS:
        roaming, local = home / "AppData" / "Roaming", home / "AppData" / "Local"
        roaming.mkdir(parents=True, exist_ok=True)
        local.mkdir(parents=True, exist_ok=True)
        env.update(USERPROFILE=str(home), APPDATA=str(roaming), LOCALAPPDATA=str(local),
                   TEMP=str(tmp), TMP=str(tmp))
    return env


def base_env() -> dict:
    env = {"PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin")}
    if IS_WINDOWS:
        upper = {key.upper(): value for key, value in os.environ.items()}
        env.update({key: upper[key] for key in WINDOWS_SYSTEM_ENV if key in upper})
    return env


def write_server_profile(home: Path, *, name: str, host: str, game_port: int, vault: str) -> None:
    """Write one private `servers.json` server into a cell HOME.

    The world names `host` and `game_port` for the game connection. The
    asset host and port stay on the real engine, so the HTTP cache path
    stays direct.
    """
    revision = int(REVISION)
    server = {
        "name": name,
        "revision": revision,
        "transport": "tcp",
        "worlds": [{
            "number": revision,
            "host": host,
            "port": game_port,
            "node_id": revision,
            "asset_host": ENGINE_HOST,
            "asset_port": HTTP_PORT,
        }],
        "login_key": {"engine_dir": None},
        "vault": vault,
        "members": None,
        "allow_plaintext_offhost": False,
    }
    servers = home / ".274bot" / "servers.json"
    servers.parent.mkdir(parents=True, exist_ok=True)
    wtext(servers, json.dumps({"schema_version": 1, "servers": [server]}, indent=1) + "\n")


def write_quest_server_profile(home: Path) -> None:
    if IS_WINDOWS or JOB != "quest" or PROFILE != "local-289q":
        return
    write_server_profile(home, name=PROFILE, host=ENGINE_HOST, game_port=GAME_PORT,
                         vault="vault-289q")


def write_latency_server_profile(home: Path, game_port: int) -> None:
    """Local engine mode: point this HOME's profile at the latency proxy.

    The proxy listens on loopback on this host, so the world host is
    `127.0.0.1` and the world port is the proxy port. The vault matches
    the built-in profile for the revision (`servers.rs`, `builtins()`):
    `vault-289` for 289, `vault` for 274.
    """
    vault = "vault-289" if REVISION == "289" else "vault"
    write_server_profile(home, name=PROFILE, host="127.0.0.1", game_port=game_port, vault=vault)


def cell_env(home: Path, tmp: Path, evidence: Path, cell: str, prefix: str,
             game_port: int | None = None) -> dict:
    env = base_env()
    env.update(private_dirs(home, tmp))
    write_quest_server_profile(home)
    if game_port is not None and ENGINE_MODE != "remote":
        # Local mode passes no endpoint flags, so the cell HOME's
        # servers.json carries the proxy endpoint instead.
        write_latency_server_profile(home, game_port)
    env.update({
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "BOT_LIVE_NAME_PREFIX": prefix,
        "LIVE_EVIDENCE_DIR": str(evidence),
        "RS2B0T": str(CATALOG),
        "ENGINE_DIR": str(ENGINE_RUN / "engine"),
    })
    env.pop("QUESTER_TICK_MS", None)
    if JOB_DEFAULTS["tick_ms"] is not None:
        env["QUESTER_TICK_MS"] = str(JOB_DEFAULTS["tick_ms"])
    if ISOHOME:
        env["ISOHOME_DIR"] = str(home)
        if os.environ.get("ISOHOME_ROOT"):
            env["ISOHOME_ROOT"] = os.environ["ISOHOME_ROOT"]
    if not GPU_CELLS.search(cell):
        env["BOT_CPU"] = "1"
    return env


def engine_flags(game_port: int | None = None) -> list[str]:
    if ENGINE_MODE != "remote":
        return []
    if game_port is not None:
        # The latency proxy listens on loopback on this host. The asset
        # endpoint stays on the real engine, so the HTTP cache path and
        # the OnDemand HTTP fetch stay direct.
        return ["--host", "127.0.0.1", "--port", str(game_port), "--asset-host", ENGINE_HOST,
                "--http-port", str(HTTP_PORT)]
    return ["--host", ENGINE_HOST, "--port", str(GAME_PORT), "--asset-host", ENGINE_HOST,
            "--http-port", str(HTTP_PORT)]


def tui_cmd(binary: Path, cell: str, gate: str | None = None,
            game_port: int | None = None) -> list[str]:
    cmd = [str(binary), "--profile", PROFILE, *engine_flags(game_port),
           "--engine", str(ENGINE_RUN / "engine"),
           "--catalog", str(CATALOG), "--live", f"script_{cell}"]
    return cmd + [gate] if gate else cmd


def run_tui(cmd: list[str], env: dict, cwd: Path, stderr_path: Path, cap_s: float
            ) -> tuple[int, bool, str, bool]:
    """Run tui-play on a pseudo-terminal and keep stderr apart.

    Returns the exit code, cap status, cleaned terminal transcript, and quiet-lock status.
    """
    if IS_WINDOWS:
        return run_tui_conpty(cmd, env, cwd, stderr_path, cap_s)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 200, 0, 0))
    tty_raw = bytearray()

    def drain() -> None:
        while True:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                return
            if not chunk:
                return
            if len(tty_raw) < 64 << 20:
                tty_raw.extend(chunk)

    started = time.monotonic()
    timed_out = False
    quiet_stopped = False
    with stderr_path.open("wb") as stderr:
        proc = popen_group(cmd, env=env, cwd=cwd, stdin=slave, stdout=slave, stderr=stderr)
        os.close(slave)
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        track(proc.pid, lambda: kill_tree(proc.pid, proc))
        try:
            while True:
                try:
                    proc.wait(timeout=0.25)
                    if note_quiet_lock():
                        quiet_stopped = True
                        stop_cell_process(proc.pid, proc)
                    break
                except subprocess.TimeoutExpired:
                    if _stop.is_set():
                        kill_tree(proc.pid, proc)
                        break
                    if note_quiet_lock():
                        quiet_stopped = True
                        stop_cell_process(proc.pid, proc)
                        break
                    if time.monotonic() - started > cap_s:
                        timed_out = True
                        kill_tree(proc.pid, proc)
                        break
        finally:
            untrack(proc.pid)
        proc.wait()
        reader.join(timeout=10)
        os.close(master)
    tty_text = ANSI.sub("", tty_raw.decode("utf-8", "replace")).replace("\r", "")
    return proc.returncode, timed_out, tty_text, quiet_stopped


def run_tui_conpty(cmd: list[str], env: dict, cwd: Path, stderr_path: Path, cap_s: float
                   ) -> tuple[int, bool, str, bool]:
    """Run the Windows TUI on ConPTY and keep stderr in its own file."""
    from winpty import PTY, Backend

    comspec = os.environ.get("COMSPEC", r"C:\Windows\System32\cmd.exe")
    line = subprocess.list2cmdline(cmd) + f' 2>"{stderr_path}"'
    console = PTY(200, 50, backend=Backend.ConPTY)
    block = "\0".join(f"{key}={value}" for key, value in env.items()) + "\0"
    if not console.spawn(comspec, cmdline=f' /d /s /c "{line}"', cwd=str(cwd), env=block):
        raise OSError(f"ConPTY spawn failed: {line}")
    pid = console.pid
    track(pid, lambda: kill_tree(pid))
    chunks: list[str] = []
    size = 0
    started = time.monotonic()
    timed_out = False
    quiet_stopped = False
    try:
        while True:
            chunk = console.read(blocking=False)
            if chunk:
                if size < 64 << 20:
                    chunks.append(chunk)
                    size += len(chunk)
                continue
            if not console.isalive():
                time.sleep(0.5)
                tail = console.read(blocking=False)
                if tail:
                    chunks.append(tail)
                    continue
                break
            if _stop.is_set():
                kill_tree(pid)
                break
            if note_quiet_lock():
                quiet_stopped = True
                stop_cell_process(pid)
                break
            if time.monotonic() - started > cap_s:
                timed_out = True
                kill_tree(pid)
                break
            time.sleep(0.05)
    finally:
        untrack(pid)
    code = console.get_exitstatus()
    tty_text = ANSI.sub("", "".join(chunks)).replace("\r", "")
    return (code if code is not None else -1), timed_out, tty_text, quiet_stopped


def panel_cmd(binary: Path, cell: str, game_port: int | None = None) -> list[str]:
    return [str(binary), "--profile", PROFILE, *engine_flags(game_port),
            "--engine", str(ENGINE_RUN / "engine"),
            "--catalog", str(CATALOG), "--live", f"script_{cell}"]


def run_panel(cmd: list[str], env: dict, cwd: Path, stdout_path: Path, stderr_path: Path,
              cap_s: float) -> tuple[int, bool, bool]:
    """Run panel-play and return the exit code, cap status, and quiet-lock status."""
    started = time.monotonic()
    timed_out = False
    quiet_stopped = False
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        proc = popen_group(cmd, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        track(proc.pid, lambda: kill_tree(proc.pid, proc))
        try:
            while True:
                try:
                    proc.wait(timeout=0.25)
                    if note_quiet_lock():
                        quiet_stopped = True
                        stop_cell_process(proc.pid, proc)
                    break
                except subprocess.TimeoutExpired:
                    if _stop.is_set():
                        kill_tree(proc.pid, proc)
                        break
                    if note_quiet_lock():
                        quiet_stopped = True
                        stop_cell_process(proc.pid, proc)
                        break
                    if time.monotonic() - started > cap_s:
                        timed_out = True
                        kill_tree(proc.pid, proc)
                        break
        finally:
            untrack(proc.pid)
        proc.wait()
    return proc.returncode, timed_out, quiet_stopped


def cells_root(run_dir: Path, frontend: str) -> Path:
    return run_dir / ("cells" if frontend == "tui" else f"{frontend}-cells")


def run_cell(cell: str, frontend: str, binary: Path, run_dir: Path, work: Path, base: CellResult,
             deadline: float, game_port: int | None = None, name_prefix: str | None = None,
             root_name: str | None = None, step: str | None = None) -> CellResult:
    result = CellResult(**{**asdict(base), "cell": cell, "frontend": frontend,
                           "renderer": "gpu" if GPU_CELLS.search(cell) else "cpu"})
    cell_dir = (run_dir / root_name / cell) if root_name else cells_root(run_dir, frontend) / cell
    cell_dir.mkdir(parents=True, exist_ok=True)
    if note_quiet_lock():
        result.result, result.reason = "UNAVAILABLE", f"MAC-QUIET: {_quiet_lock_reason}"
        return write_cell(cell_dir, result)
    if time.monotonic() >= deadline:
        result.result = "NOT_RUN"
        result.reason = f"{deadline_text(step or frontend)} reached before this cell started"
        return write_cell(cell_dir, result)
    scratch = work / frontend / cell
    home = scratch / "home"
    tmp = scratch / "tmp"
    prefix = name_prefix or (PANEL_NAME_PREFIX if frontend == "panel" else NAME_PREFIX)
    started = time.monotonic()
    result.started = now_iso()
    gate = None
    stdout_name = "tty.log" if frontend == "tui" else "stdout.log"
    quiet_stopped = False
    while True:
        rmtree(scratch)
        home.mkdir(parents=True)
        tmp.mkdir(parents=True)
        cap = min(cell_cap_s(cell), max(1.0, deadline - time.monotonic()))
        cut_by_deadline = cap < cell_cap_s(cell)
        env = cell_env(home, tmp, cell_dir / "evidence", cell, prefix, game_port=game_port)
        if frontend == "tui":
            cmd = isolated_command(tui_cmd(binary, cell, gate, game_port=game_port))
            wtext(cell_dir / "command.txt", " ".join(cmd) + "\n")
            exit_code, timed_out, out_text, quiet_stopped = run_tui(
                cmd, env, scratch, cell_dir / "stderr.log", cap)
        else:
            cmd = isolated_command(panel_cmd(binary, cell, game_port=game_port))
            wtext(cell_dir / "command.txt", " ".join(cmd) + "\n")
            exit_code, timed_out, quiet_stopped = run_panel(
                cmd, env, scratch, cell_dir / stdout_name, cell_dir / "stderr.log", cap)
            out_text = rtext(cell_dir / stdout_name)
        stderr_text = rtext(cell_dir / "stderr.log")
        if (not quiet_stopped and frontend == "tui" and gate is None and exit_code != 0
                and PAIR_GATE_REFUSAL in stderr_text):
            gate = "--pair-core"
            continue
        break
    result.gate = gate
    result.wall_s = round(time.monotonic() - started, 1)
    result.exit_code = exit_code
    if frontend == "tui":
        wtext(cell_dir / stdout_name, out_text)
    # Logs the frontend writes under its (throwaway) HOME.
    session_text = ""
    bot_dir = home / ".274bot"
    for path in sorted(bot_dir.glob("*.log")) + sorted((bot_dir / "logs").rglob("*")):
        if path.is_file():
            target = cell_dir / "session-logs" / path.relative_to(bot_dir)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            session_text += rtext(target) + "\n"
    combined = stderr_text + "\n" + session_text + "\n" + out_text
    if quiet_stopped:
        result.result, result.reason = "UNAVAILABLE", f"MAC-QUIET: {_quiet_lock_reason}"
    else:
        receipt = apply_cell_outcome(result, cell, frontend, combined, stderr_text, out_text,
                                     timed_out, cut_by_deadline, cell_cap_s(cell))
        if receipt is not None:
            wtext(cell_dir / "receipt.json", json.dumps(receipt, indent=1) + "\n")
    rmtree(scratch)
    return write_cell(cell_dir, result)


def cell_row(result: CellResult) -> dict:
    """One cell row as JSON, without the receipt.

    The latency fields join only on latency rows. A zero-latency row keeps
    the exact keys it had before the lane existed, so a night with the lane
    off writes the same summary files as the runner without the lane.
    """
    row = asdict(result)
    row.pop("receipt")
    if row["latency_band"] is None:
        for key in ("latency_band", "latency_rtt_ms", "latency_class"):
            row.pop(key)
    return row


def write_cell(cell_dir: Path, result: CellResult) -> CellResult:
    wtext(cell_dir / "result.json", json.dumps(cell_row(result), indent=1) + "\n")
    return result


def cell_rows(frontend: str, cells: list[str], run_dir: Path, base: CellResult, skips: dict[str, str],
              unavailable: str | None, not_run: str | None,
              root_name: str | None = None) -> tuple[list[CellResult], list[str]]:
    """Rows for cells that do not start (skip, unavailable, not run) and the runnable rest."""
    rows, runnable = [], []
    for cell in cells:
        fixed = {"cell": cell, "frontend": frontend, "renderer": "gpu" if GPU_CELLS.search(cell) else "cpu"}
        if cell in skips:
            row = CellResult(**{**asdict(base), **fixed, "result": "SKIP", "reason": skips[cell]})
        elif unavailable:
            row = CellResult(**{**asdict(base), **fixed, "result": "UNAVAILABLE", "reason": unavailable})
        elif not_run:
            row = CellResult(**{**asdict(base), **fixed, "result": "NOT_RUN", "reason": not_run})
        else:
            runnable.append(cell)
            continue
        cell_dir = (run_dir / root_name / cell) if root_name else cells_root(run_dir, frontend) / cell
        cell_dir.mkdir(parents=True, exist_ok=True)
        rows.append(write_cell(cell_dir, row))
    return rows, runnable


def run_cells(frontend: str, binary: Path, cells: list[str], run_dir: Path, work: Path, base: CellResult,
              parallel: int, deadline: float, branch: str, label: str | None = None,
              game_port: int | None = None, name_prefix: str | None = None,
              root_name: str | None = None, step: str | None = None) -> list[CellResult]:
    results = []
    label = label or frontend
    log(f"{branch}: {label}: running {len(cells)} cells, {parallel} in parallel")
    with concurrent.futures.ThreadPoolExecutor(max_workers=parallel) as pool:
        futures = {pool.submit(run_cell, cell, frontend, binary, run_dir, work, base, deadline,
                               game_port=game_port, name_prefix=name_prefix,
                               root_name=root_name, step=step): cell
                   for cell in cells}
        for future in concurrent.futures.as_completed(futures):
            outcome = future.result()
            results.append(outcome)
            log(f"{branch}: {label}: {outcome.cell} {outcome.result} ({fmt_s(outcome.wall_s)})")
    return results


# ---------------------------------------------------------------- latency lane


def band_name(rtt_ms: int) -> str:
    return f"rtt{rtt_ms}"


def latency_prefix(base: str, rtt_ms: int) -> str:
    """The account name prefix for one band.

    The mint accepts a prefix of 1 to 4 characters
    (`crates/host-play/src/play_bootstrap.rs`, `parse_live_name_prefix`),
    so the job prefix keeps only the characters the band digits leave:
    `nt` at rtt50 gives `nt50`; `nt` at rtt100 gives `n100`.
    """
    suffix = str(rtt_ms)
    keep = 4 - len(suffix)
    if not base or keep < 1:
        raise ValueError(f"cannot build a latency prefix from {base!r} and band {rtt_ms}")
    return base[:keep] + suffix


def classify_latency(zero: str | None, band: str, *, proxy_unavailable: bool = False) -> str:
    """The script-computed class of one band row against its zero row.

    `proxy_unavailable` wins over every other rule: the lane failed, not
    the product. A latency defect needs a zero PASS on the same night,
    commit, platform and frontend, and a band FAIL, TIMEOUT or ERROR.
    """
    if proxy_unavailable:
        return "proxy-unavailable"
    if zero is None or zero in ("UNAVAILABLE", "NOT_RUN", "SKIP"):
        return "no-baseline"
    if zero in ("FAIL", "TIMEOUT", "ERROR"):
        return "also-fails-at-0"
    if band == "PASS":
        return "pass"
    if band in ("FAIL", "TIMEOUT", "ERROR"):
        return "latency-defect"
    if band in ("NOT_RUN", "SKIP"):
        return "not-run"
    return "unavailable"  # band UNAVAILABLE for a non-proxy reason


def read_proxy_stats(path: Path) -> list[dict]:
    """Proxy stats records (JSON Lines). A torn last line is skipped."""
    if not path.is_file():
        return []
    records = []
    for line in rtext(path).splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            records.append(json.loads(line))
        except ValueError:
            continue
    return records


def _weighted_median(pairs: list[tuple[float, int]]) -> float | None:
    ordered = sorted(pairs)
    total = sum(weight for _value, weight in ordered)
    if total <= 0:
        return None
    upto = 0
    for value, weight in ordered:
        upto += weight
        if upto >= total / 2:
            return value
    return ordered[-1][0]


def aggregate_proxy_stats(records: list[dict], start_epoch: float | None = None,
                          end_epoch: float | None = None) -> dict:
    """Aggregate the records that opened inside one time window.

    Per-direction p50 and p95 are sample-weighted medians of the
    per-connection values of delayed records only. Direct (byte 15)
    records count toward connections and bytes, never toward queue
    delay. The login value is the median of the byte-14 records.
    """
    selected = [record for record in records
                if (start_epoch is None or record.get("opened_at_epoch", 0) >= start_epoch)
                and (end_epoch is None or record.get("opened_at_epoch", 0) <= end_epoch)]
    connections = {"game_14": 0, "ondemand_15": 0, "other": 0, "none": 0}
    for record in selected:
        first = record.get("first_byte")
        key = "game_14" if first == 14 else "ondemand_15" if first == 15 \
            else "none" if first is None else "other"
        connections[key] += 1

    def direction(field: str) -> dict:
        p50_pairs, p95_pairs, samples = [], [], 0
        for record in selected:
            if record.get("path") != "delayed":
                continue
            summary = record.get(field)
            if not summary:
                continue
            count = int(summary.get("samples", 0))
            samples += count
            p50_pairs.append((float(summary["p50"]), count))
            p95_pairs.append((float(summary["p95"]), count))
        p50, p95 = _weighted_median(p50_pairs), _weighted_median(p95_pairs)
        return {"p50_ms": round(p50, 3) if p50 is not None else None,
                "p95_ms": round(p95, 3) if p95 is not None else None,
                "samples": samples}

    logins = [record["login_first_response_ms"] for record in selected
              if record.get("first_byte") == 14 and record.get("login_first_response_ms") is not None]
    return {
        "records": len(selected),
        "connections": connections,
        "bytes_client_to_server": sum(record.get("bytes_client_to_server", 0) for record in selected),
        "bytes_server_to_client": sum(record.get("bytes_server_to_client", 0) for record in selected),
        "client_to_server": direction("queue_delay_ms_client_to_server"),
        "server_to_client": direction("queue_delay_ms_server_to_client"),
        "login_first_response_ms": round(statistics.median(logins), 3) if logins else None,
    }


def _cell_window(result: CellResult) -> tuple[float, float] | None:
    """The epoch window a started cell ran in.

    `started` carries whole seconds, so the parsed start sits at or
    before the true start. No start slack is needed. The end keeps one
    second for a connection that opens in the cell's last second.
    """
    if result.started is None or result.wall_s is None:
        return None
    try:
        start = dt.datetime.fromisoformat(result.started).timestamp()
    except ValueError:
        return None
    return start, start + result.wall_s + 1.0


def probe_latency_proxy(port: int, timeout: float = 5.0) -> tuple[bool, str]:
    """One probe connection through the proxy, before any cell starts.

    The probe sends the game first byte (14). The proxy then connects to
    the engine. A dead engine makes the proxy close the probe at once. A
    live engine holds the connection while it waits for the rest of the
    login, or answers bytes. Both prove the path works.
    """
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=timeout) as sock:
            sock.sendall(bytes([14]))
            sock.settimeout(1.0)
            try:
                data = sock.recv(1)
            except socket.timeout:
                return True, "the engine holds the probe connection"
            if data == b"":
                return False, "the proxy closed the probe connection; the upstream is unreachable"
            return True, "the engine answered the probe connection"
    except OSError as error:
        return False, f"probe connection failed: {error}"


def stop_latency_proxy(proc: subprocess.Popen) -> None:
    """Stop one recorded proxy process: stdin close, then the tree kill."""
    try:
        if proc.stdin is not None:
            proc.stdin.close()
    except (OSError, ValueError):
        pass
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        kill_tree(proc.pid, proc)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
    untrack(proc.pid)


def start_latency_proxy(band_dir: Path, one_way_ms: float
                        ) -> tuple[subprocess.Popen | None, int | None, str | None]:
    """Start one band proxy on a free loopback port.

    Return `(process, listen port, error)`. On an error the process is
    already stopped. Health rule part one: no ready file within 10 s, or
    an exited process, means the band never starts a cell.
    """
    band_dir.mkdir(parents=True, exist_ok=True)
    ready = band_dir / "proxy-ready.txt"
    if ready.exists():
        ready.unlink()
    cmd = [sys.executable, str(LATENCY_PROXY), "--listen", "127.0.0.1:0",
           "--upstream", f"{ENGINE_HOST}:{GAME_PORT}", "--delay-ms", str(one_way_ms),
           "--stats", str(band_dir / "proxy-stats.jsonl"), "--ready-file", str(ready)]
    with (band_dir / "proxy.log").open("wb") as proxy_log:
        proc = popen_group(cmd, stdin=subprocess.PIPE, stdout=proxy_log, stderr=proxy_log)
    track(proc.pid, lambda: kill_tree(proc.pid, proc))
    deadline = time.monotonic() + 10.0
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            untrack(proc.pid)
            return None, None, f"the proxy exited with code {proc.returncode}; see proxy.log"
        if ready.is_file():
            try:
                return proc, int(rtext(ready).strip()), None
            except ValueError:
                pass  # a torn ready read; poll again
        time.sleep(0.05)
    stop_latency_proxy(proc)
    return None, None, "the proxy ready file did not appear within 10 s"


def write_latency_json(cell_dir: Path, result: CellResult, *, rtt_ms: int, proxy_info: dict,
                       measured: dict, frontend_sha256: str | None, account_prefix: str) -> None:
    doc = {
        "cell": result.cell,
        "band": result.latency_band,
        "configured_rtt_ms": rtt_ms,
        "configured_one_way_ms": rtt_ms / 2,
        "result": result.result,
        "latency_class": result.latency_class,
        "ticks": result.ticks,
        "proxy": proxy_info,
        "measured": measured,
        "login_first_response_ms": measured.get("login_first_response_ms"),
        "connections": measured.get("connections"),
        "frontend_sha256": frontend_sha256,
        "account_prefix": account_prefix,
    }
    wtext(cell_dir / "latency.json", json.dumps(doc, indent=1) + "\n")


def count_latency_classes(rows: list[CellResult]) -> dict:
    counts = {name: 0 for name in LATENCY_CLASS_ORDER}
    for row in rows:
        counts[row.latency_class] = counts.get(row.latency_class, 0) + 1
    return counts


def latency_counts_text(counts: dict) -> str:
    return ", ".join(f"{name} {counts[name]}" for name in LATENCY_CLASS_ORDER if counts.get(name))


def run_latency_pass(branch: str, binary: Path | None, cells: list[str], bands: list[int],
                     run_dir: Path, work: Path, base: CellResult, skips: dict[str, str],
                     parallel: int, zero: dict[str, CellResult], unavailable: str | None,
                     build_ok: bool) -> tuple[list[CellResult], dict]:
    """Run the latency cells once per band through one proxy per band.

    Return the band rows and the summary `latency` metadata. The pass
    fails closed: a proxy that does not start, does not answer the probe,
    dies during a band, or misses its measured delay band turns the
    affected rows UNAVAILABLE with class `proxy-unavailable`. It never
    turns a proxy failure into a product verdict.
    """
    deadline = step_deadline("latency", time.monotonic())
    proxy_sha = sha256(LATENCY_PROXY) if LATENCY_PROXY.is_file() else None
    frontend_sha = sha256(binary) if binary is not None and binary.is_file() else None
    rows_all: list[CellResult] = []
    band_meta: list[dict] = []

    block_unavailable, block_not_run = unavailable, None
    if note_quiet_lock():
        block_unavailable = f"MAC-QUIET: {_quiet_lock_reason}"
    if not block_unavailable and (not build_ok or binary is None):
        block_not_run = "the build failed; see build.log"
    if not block_unavailable and not block_not_run and REQUIRE_CONSOLE:
        ok, detail, _facts = console_session()
        if not ok:
            block_unavailable = detail
    engine_detail: str | None = None
    engine_started = False
    if not block_unavailable and not block_not_run:
        try:
            ok, detail = engine_start(run_dir)
        except QuietLockError as error:
            ok, detail = False, str(error)
        engine_detail = detail
        if ok:
            engine_started = True
        elif note_quiet_lock():
            block_unavailable = f"MAC-QUIET: {_quiet_lock_reason}"
        elif ENGINE_MODE == "remote":
            block_unavailable = remote_engine_failure(detail)
        else:
            block_not_run = f"engine {detail}"

    try:
        for rtt_ms in bands:
            band = band_name(rtt_ms)
            one_way_ms = rtt_ms / 2
            prefix = latency_prefix(NAME_PREFIX, rtt_ms)
            band_base = CellResult(**{**asdict(base), "latency_band": band, "latency_rtt_ms": rtt_ms})
            root_name = f"cells-latency/{band}"
            band_dir = run_dir / "latency" / band
            proxy_info: dict = {"file": LATENCY_PROXY.name, "sha256": proxy_sha,
                                "listen_port": None, "upstream": f"{ENGINE_HOST}:{GAME_PORT}",
                                "pid": None, "stats": f"latency/{band}/proxy-stats.jsonl"}
            records: list[dict] = []
            cells_started_epoch: float | None = None
            proxy_failed_rows: set[str] = set()
            band_unavailable, band_not_run = block_unavailable, block_not_run
            if not band_unavailable and not band_not_run:
                if note_quiet_lock():
                    band_unavailable = f"MAC-QUIET: {_quiet_lock_reason}"
                elif time.monotonic() >= deadline:
                    band_not_run = f"{deadline_text('latency')} reached before this band started"
            if band_unavailable or band_not_run:
                rows, _runnable = cell_rows("tui", cells, run_dir, band_base, skips,
                                            band_unavailable, band_not_run, root_name=root_name)
            else:
                log(f"{branch}: latency {band}: one-way {one_way_ms:g} ms, prefix {prefix}")
                proc, port, error = start_latency_proxy(band_dir, one_way_ms)
                if error is not None:
                    reason = f"latency proxy unavailable: {error}"
                    rows, _runnable = cell_rows("tui", cells, run_dir, band_base, skips,
                                                reason, None, root_name=root_name)
                    proxy_failed_rows = {row.cell for row in rows}
                else:
                    assert proc is not None and port is not None
                    proxy_info.update(listen_port=port, pid=proc.pid)
                    probe_ok, probe_detail = probe_latency_proxy(port)
                    if not probe_ok:
                        stop_latency_proxy(proc)
                        reason = f"latency proxy unavailable: probe failed: {probe_detail}"
                        rows, _runnable = cell_rows("tui", cells, run_dir, band_base, skips,
                                                    reason, None, root_name=root_name)
                        proxy_failed_rows = {row.cell for row in rows}
                    else:
                        assert binary is not None
                        rows, runnable = cell_rows("tui", cells, run_dir, band_base, skips,
                                                   None, None, root_name=root_name)
                        # Records from this epoch on belong to cells. The
                        # health probe record, opened before it, never
                        # counts toward a cell's measured delay.
                        cells_started_epoch = time.time()
                        if runnable:
                            rows += run_cells("tui", binary, runnable, run_dir,
                                              work / "latency" / band, band_base, parallel,
                                              deadline, branch, label=f"tui {band}",
                                              game_port=port, name_prefix=prefix,
                                              root_name=root_name, step="latency")
                        proxy_died = proc.poll() is not None
                        stop_latency_proxy(proc)
                        records = read_proxy_stats(band_dir / "proxy-stats.jsonl")
                        if proxy_died:
                            for row in rows:
                                if row.result in ("FAIL", "TIMEOUT", "ERROR"):
                                    row.result = "UNAVAILABLE"
                                    row.reason = ("latency proxy died during the band "
                                                  f"(exit {proc.returncode})")
                                    proxy_failed_rows.add(row.cell)
            order = {cell: index for index, cell in enumerate(cells)}
            rows.sort(key=lambda row: order.get(row.cell, len(order)))
            for row in rows:
                window = _cell_window(row)
                if window is not None and cells_started_epoch is not None:
                    window = (max(window[0], cells_started_epoch), window[1])
                measured = (aggregate_proxy_stats(records, window[0], window[1])
                            if window else aggregate_proxy_stats([]))
                if row.started is not None and row.cell not in proxy_failed_rows:
                    # Health rule part two: the measured median queue delay
                    # must sit inside the band. A PASS with no delayed
                    # connection at all cannot prove the band applied.
                    samples = (measured["client_to_server"]["samples"]
                               + measured["server_to_client"]["samples"])
                    p50s = [measured[direction]["p50_ms"]
                            for direction in ("client_to_server", "server_to_client")
                            if measured[direction]["p50_ms"] is not None]
                    off_band = [p50 for p50 in p50s
                                if abs(p50 - one_way_ms) > LATENCY_HEALTH_TOLERANCE_MS]
                    if off_band:
                        row.result = "UNAVAILABLE"
                        row.reason = (f"latency proxy health: measured queue delay p50 "
                                      f"{off_band} ms, configured one-way {one_way_ms:g} ms")
                        proxy_failed_rows.add(row.cell)
                    elif row.result == "PASS" and samples == 0:
                        row.result = "UNAVAILABLE"
                        row.reason = ("latency proxy health: no delayed connection "
                                      "was recorded for this cell")
                        proxy_failed_rows.add(row.cell)
                zero_row = zero.get(row.cell)
                row.latency_class = classify_latency(
                    zero_row.result if zero_row is not None else None, row.result,
                    proxy_unavailable=row.cell in proxy_failed_rows)
                cell_dir = run_dir / root_name / row.cell
                cell_dir.mkdir(parents=True, exist_ok=True)
                write_cell(cell_dir, row)
                write_latency_json(cell_dir, row, rtt_ms=rtt_ms, proxy_info=proxy_info,
                                   measured=measured, frontend_sha256=frontend_sha,
                                   account_prefix=prefix)
            rows_all += rows
            band_meta.append({"band": band, "rtt_ms": rtt_ms, "one_way_ms": one_way_ms,
                              "account_prefix": prefix, "proxy": proxy_info,
                              "measured": aggregate_proxy_stats(records, cells_started_epoch)})
            log(f"{branch}: latency {band}: "
                + latency_counts_text(count_latency_classes(rows)))
    finally:
        if engine_started and ENGINE_MODE == "local":
            engine_stop(run_dir)
    meta = {"proxy": {"file": LATENCY_PROXY.name, "sha256": proxy_sha},
            "branch": LATENCY_BRANCH, "cells_file": LATENCY_CELLS_FILE,
            "report_only": True, "engine": engine_detail, "bands": band_meta}
    return rows_all, meta



# ---------------------------------------------------------------- CI lanes


def lane_asset_env() -> dict:
    """The local test inputs each test lane gets, as on Linux CI.

    GitHub CI and the merge-check script set the three nav pack names to
    the one v17 reference pack for every test lane. The engine content
    comes from the pristine copy. It exists before the lanes step. The
    per-step engine-run copy does not exist yet at that point. The
    snapshot root and the inspection baseline join only when the host
    holds them. A test then keeps its own loud failure on a host that
    lacks the file, and it names the local path to supply.
    """
    pack = str(NAV_FIXTURE)
    env = {
        "NAV_PACK": pack,
        "WORLD_NAV_PACK": pack,
        "NAV_ARRIVAL_PACK": pack,
        "ENGINE_DIR": str(ENGINE_PRISTINE / "engine"),
        "WORLD_ENGINE_DIR": str(ENGINE_PRISTINE / "engine"),
    }
    if snapshot_ready():
        env["BOT_NAV_SNAPSHOT_ROOT"] = str(NAV_SNAPSHOT)
    if INSPECT_BASELINE.is_file():
        env["T5W_INSPECT_CAPTURE_BASELINE"] = str(INSPECT_BASELINE)
    return env


def seed_lane_home(home: Path, snapshot_root: Path | None = None, fallback_root: Path | None = None) -> int:
    """Copy decoded cache snapshots into one lane HOME. Return the count.

    The client tests read `$HOME/.274bot/unpack/<cache hash>`. The first
    source is the nightly snapshot folder. The build step creates that
    folder after the lanes step on a new host. The fallback then copies
    the 16-hex folders from the runner user's own `unpack-289`, as the
    Linux merge-check script seeds its test homes. Each copy is private.
    """
    if snapshot_root is None:
        snapshot_root = NAV_SNAPSHOT
    if fallback_root is None:
        fallback_root = Path.home() / ".274bot" / "unpack-289"
    unpack = home / ".274bot" / "unpack"
    copied = 0
    for snapshot in sorted(snapshot_root.glob("*/versionlist")):
        shutil.copytree(snapshot.parent, unpack / snapshot.parent.name)
        copied += 1
    if copied == 0 and fallback_root.is_dir():
        for snapshot in sorted(fallback_root.iterdir()):
            if snapshot.is_dir() and re.fullmatch(r"[0-9a-f]{16}", snapshot.name):
                shutil.copytree(snapshot, unpack / snapshot.name)
                copied += 1
    return copied


def lane_list(tree: Path | None) -> list[tuple[str, list[str], dict]]:
    """The Windows platform gate's lane set (gate.ps1), run with the GPU on.

    `test-panel-memory` runs every panel target, including the `ime_frame`
    integration test that needs an interactive desktop (the console session).
    `cargo fmt` runs per crate: a workspace-root fmt hits OS error 206 on Windows.
    """
    vendor = ["--manifest-path", "vendor/fr-client-rust/Cargo.toml"]
    assets = lane_asset_env()
    lanes = [
        ("build", ["build", "--locked", "--workspace", "--all-targets", "--features", GATE_FEATURES], {}),
        ("build-vendor", ["build", "--locked", *vendor, "--workspace"], {}),
        ("clippy", ["clippy", "--locked", "--workspace", "--all-targets", "--features", GATE_FEATURES,
                    "--", "-D", "warnings"], {}),
        ("clippy-vendor", ["clippy", "--locked", *vendor, "--workspace", "--all-targets", "--", "-D", "warnings"], {}),
        ("test-workspace", ["test", "--locked", "--workspace", "--no-fail-fast"], assets),
        ("test-vendor", ["test", "--locked", *vendor, "--workspace", "--no-fail-fast"], assets),
        ("test-script-load", ["test", "--locked", "-p", "script-js", "--tests",
                              "--no-fail-fast"], assets),
        ("test-script-no-default", ["test", "--locked", "-p", "script", "--no-default-features",
                                    "--no-fail-fast"], assets),
        ("test-host-play-memory", ["test", "--locked", "-p", "host-play", "--features", "memory-profile",
                                   "--no-fail-fast"], assets),
        ("test-host-play-memory-no-alloc", ["test", "--locked", "-p", "host-play", "--features",
                                            "memory-profile-no-alloc", "--lib", "--no-fail-fast"], assets),
        ("test-host-performance", ["test", "--locked", "-p", "host", "--features", "performance-profile",
                                   "--no-fail-fast"], assets),
        ("test-panel-memory", ["test", "--locked", "-p", "panel", "--features", "memory-profile",
                               "--no-fail-fast"], assets),
        ("test-panel-render-diagnostics", ["test", "--locked", "-p", "panel", "--features",
                                           "render-diagnostics", "--lib", "--no-fail-fast"], assets),
        ("test-tui-memory", ["test", "--locked", "-p", "tui", "--features", "memory-profile", "--no-fail-fast"], assets),
    ]
    if tree is not None:
        for manifest in sorted(tree.glob("crates/*/Cargo.toml")):
            lanes.append((f"fmt-{manifest.parent.name}", ["fmt", "--manifest-path", str(manifest), "--", "--check"], {}))
        for manifest in sorted(tree.glob("vendor/fr-client-rust/crates/*/Cargo.toml")):
            lanes.append((f"fmt-vendor-{manifest.parent.name}",
                          ["fmt", "--manifest-path", str(manifest), "--", "--check"], {}))
    return lanes


def lane_env(home: Path, tmp: Path, target: Path, extra: dict) -> dict:
    env = dict(os.environ)
    for name in ("NAV_PACK", "WORLD_NAV_PACK", "NAV_ARRIVAL_PACK", "ENGINE_DIR", "WORLD_ENGINE_DIR", "LIVE",
                 "BOT_NAV_SNAPSHOT_ROOT", "T5W_INSPECT_CAPTURE_BASELINE", "T5W_INSPECT_CAPTURE_OUT", "SKIP_GPU",
                 "BOT_CPU", "CARGO_TARGET_DIR", "QUESTER_TICK_MS", "ISOHOME_DIR"):
        env.pop(name, None)
    env.update(private_dirs(home, tmp))
    env.update(CARGO_TARGET_DIR=str(target), SCCACHE_DIR=str(ROOT / "sccache"),
               BOT_NAV_BUILD="skip", CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0",
               CARGO_TERM_COLOR="never", **extra)
    if ISOHOME:
        env["ISOHOME_DIR"] = str(home)
    return env


def run_lane(name: str, args: list[str], extra: dict, tree: Path, target: Path, run_dir: Path, work: Path,
             base: LaneResult, deadline: float) -> LaneResult:
    result = LaneResult(**{**asdict(base), "lane": name, "command": "cargo " + " ".join(args)})
    lane_dir = run_dir / "lanes" / name
    lane_dir.mkdir(parents=True, exist_ok=True)
    if _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted before this lane started"
        return write_lane(lane_dir, result)
    if note_quiet_lock():
        result.result, result.reason = "NOT_RUN", _quiet_lock_reason
        return write_lane(lane_dir, result)
    if time.monotonic() >= deadline:
        result.result = "NOT_RUN"
        result.reason = f"{deadline_text('lanes')} reached before this lane started"
        return write_lane(lane_dir, result)
    scratch = work / "lanes" / name
    rmtree(scratch)
    home, tmp = scratch / "home", scratch / "tmp"
    tmp.mkdir(parents=True)
    # The decoded snapshot the client tests unpack against, as a private copy.
    seed_lane_home(home)
    home.mkdir(parents=True, exist_ok=True)
    cargo_log = lane_dir / "cargo.log"
    result.started = now_iso()
    started = time.monotonic()
    timed_out = False
    exit_code = None
    try:
        exit_code = run(["cargo", *args], cwd=tree, env=lane_env(home, tmp, target, extra), log_to=cargo_log,
                        check=False, timeout=max(1.0, deadline - time.monotonic())).returncode
    except QuietLockError as error:
        result.result, result.reason = "NOT_RUN", str(error)
    except subprocess.TimeoutExpired:
        timed_out = True
    result.wall_s = round(time.monotonic() - started, 1)
    result.exit_code = exit_code
    text = rtext(cargo_log)
    sums = [tuple(int(n) for n in match) for match in TEST_RESULT.findall(text)]
    if sums:
        result.tests = {"passed": sum(s[0] for s in sums), "failed": sum(s[1] for s in sums),
                        "ignored": sum(s[2] for s in sums)}
    result.failed_tests = TEST_FAILED.findall(text)[:100]
    if timed_out:
        result.result = "TIMEOUT"
        result.first_failure = "lanes step deadline reached while this lane ran"
    elif note_quiet_lock():
        result.result, result.reason = "NOT_RUN", _quiet_lock_reason
    elif _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted while this lane ran"
    else:
        result.result = "PASS" if exit_code == 0 else "FAIL"
        if result.result == "FAIL":
            errors = [line for line in text.splitlines() if line.startswith(("error", "Error"))]
            result.first_failure = (f"{len(result.failed_tests)} failed test(s): {', '.join(result.failed_tests[:5])}"
                                    if result.failed_tests else (errors[0][:400] if errors else first_failure_line(text)))
    rmtree(scratch)
    return write_lane(lane_dir, result)


def write_lane(lane_dir: Path, result: LaneResult) -> LaneResult:
    wtext(lane_dir / "result.json", json.dumps(asdict(result), indent=1) + "\n")
    return result


def run_lanes(tree: Path, run_dir: Path, work: Path, base: LaneResult, deadline: float, branch: str,
              only: list[str] | None) -> list[LaneResult]:
    target = ROOT / "target" / f"{slug(branch)}-ci"
    results = []
    for name, args, extra in lane_list(tree):
        if only and name not in only:
            continue
        outcome = run_lane(name, args, extra, tree, target, run_dir, work, base, deadline)
        results.append(outcome)
        log(f"{branch}: lanes: {name} {outcome.result} ({fmt_s(outcome.wall_s)})")
    return results


# ---------------------------------------------------------------- summary


ORDER = ["FAIL", "TIMEOUT", "ERROR", "UNAVAILABLE", "NOT_RUN", "SKIP", "PASS"]


def count(rows) -> dict:
    counts = {key: 0 for key in ORDER}
    for row in rows:
        counts[row.result] = counts.get(row.result, 0) + 1
    return counts


def counts_text(counts: dict) -> str:
    return ", ".join(f"{key} {counts[key]}" for key in ORDER if counts.get(key))


def md_cell_table(results: list[CellResult]) -> list[str]:
    md = ["| Cell | Result | Ticks | Wall | First failure line / reason | Commit |",
          "|---|---|---:|---:|---|---|"]
    for result in sorted(results, key=lambda r: (ORDER.index(r.result), r.cell)):
        note = result.first_failure or result.reason or ""
        if result.gate:
            note = f"[{result.gate}] {note}".strip()
        if result.renderer == "gpu":
            note = f"[gpu] {note}".strip()
        note = note.replace("|", "\\|").replace("\n", " ")
        md.append(f"| `{result.cell}` | {result.result} | {result.ticks if result.ticks is not None else ''}"
                  f" | {fmt_s(result.wall_s) if result.wall_s is not None else ''} | {note} | `{result.commit}` |")
    return md


def md_latency_matrix(results: list[CellResult], latency_rows: list[CellResult]) -> list[str]:
    """The cell-by-band matrix for `summary.md`.

    One row per cell. `rtt0` is the main run of the same night. The class
    is the worst class across the bands, in `LATENCY_CLASS_ORDER`. Rows
    sort with `latency-defect` first, then `also-fails-at-0`, then the
    rest by cell name.
    """
    zero = {row.cell: row for row in results}
    bands = sorted({row.latency_rtt_ms for row in latency_rows if row.latency_rtt_ms is not None})
    by_cell: dict[str, dict[int, CellResult]] = {}
    for row in latency_rows:
        if row.latency_rtt_ms is not None:
            by_cell.setdefault(row.cell, {})[row.latency_rtt_ms] = row

    def worst_class(cell: str) -> str:
        classes = [row.latency_class or "" for row in by_cell[cell].values()]
        return min(classes, key=lambda name: LATENCY_CLASS_ORDER.index(name)
                   if name in LATENCY_CLASS_ORDER else len(LATENCY_CLASS_ORDER))

    def sort_key(cell: str) -> tuple[int, str]:
        rank = {"latency-defect": 0, "also-fails-at-0": 1}.get(worst_class(cell), 2)
        return rank, cell

    top = bands[-1] if bands else 0
    md = ["| Cell | rtt0 | " + " | ".join(f"rtt{band}" for band in bands)
          + f" | Class | Ticks 0→{top} |",
          "|---|---|" + "---|" * len(bands) + "---|---|"]
    for cell in sorted(by_cell, key=sort_key):
        zero_row = zero.get(cell)
        top_row = by_cell[cell].get(top)
        ticks = (f"{zero_row.ticks if zero_row is not None and zero_row.ticks is not None else '-'}"
                 f"→{top_row.ticks if top_row is not None and top_row.ticks is not None else '-'}")
        band_cells = " | ".join(by_cell[cell][band].result if band in by_cell[cell] else "-"
                                 for band in bands)
        md.append(f"| `{cell}` | {zero_row.result if zero_row is not None else '-'} | {band_cells} "
                  f"| {worst_class(cell)} | {ticks} |")
    return md


def write_summary(run_dir: Path, meta: dict, lanes: list[LaneResult], panel: list[CellResult],
                  results: list[CellResult], latency_rows: list[CellResult] | None = None) -> None:
    counts = count(results)
    summary = dict(meta, counts=counts, cells=[cell_row(row) for row in results])
    if "lanes" in meta["steps"]:
        summary["lane_counts"] = count(lanes)
        summary["lanes"] = [asdict(row) for row in lanes]
    if "panel" in meta["steps"]:
        summary["panel_counts"] = count(panel)
        summary["panel_cells"] = [cell_row(row) for row in panel]
    if latency_rows:
        # The main counts describe the zero run only. Band rows never
        # join them, so night-to-night readers keep their comparison.
        summary["latency_counts"] = count_latency_classes(latency_rows)
        summary["latency_cells"] = [cell_row(row) for row in latency_rows]
    wtext(run_dir / "summary.json", json.dumps(summary, indent=1) + "\n")

    md = [
        f"# Nightly {meta['run']}",
        "",
        f"- Job `{meta['job']}`",
        f"- Branch `{meta['branch']}` at `{meta['commit_full']}` (revision {meta['revision']}"
        + (f", cells from `{meta['cells_file']}`" if meta.get("cells_file") else "") + ")",
        f"- Platform {meta['platform']} ({meta['os']}, host {meta['host']})",
        f"- Started {meta['started']}, finished {meta['finished']}, wall {fmt_s(meta['wall_s'])}",
        f"- Build: {'FAILED' if not meta['build']['ok'] else 'ok' if meta['build'].get('ran', True) else 'not run'}"
        f" ({fmt_s(meta['build']['wall_s'])})"
        + (f": {meta['build']['error']}" if meta['build'].get('error') else ""),
        f"- Engine: {meta['engine']['status']}; catalog `{meta['catalog_commit']}`; "
        f"{meta['parallel']} cells in parallel; outer cap {meta['cell_cap_s']} s",
    ]
    md.append("- " + api_drift_md_line(
        meta.get("rs2b0t_api") or {"status": "unavailable", "reason": "not checked"}))
    if meta.get("quiet_lock"):
        md.append(f"- Quiet lock stopped further work: `{meta['quiet_lock']}`")
    if meta.get("session"):
        md.append(f"- Session: {meta['session']['status']}")
    for name, step in meta["steps"].items():
        if name == "tui" and len(meta["steps"]) == 1:
            continue
        md.append(f"- Step `{name}`: {step['status']}, deadline {step['deadline_s']} s"
                  + (f", wall {fmt_s(step.get('wall_s'))}" if step.get("wall_s") is not None else ""))
    if "lanes" in meta["steps"]:
        md.append("- Lane counts: " + counts_text(summary["lane_counts"]))
    if "panel" in meta["steps"]:
        md.append("- Panel cell counts: " + counts_text(summary["panel_counts"]))
    md += [("- Counts: " if len(meta["steps"]) == 1 else "- TUI cell counts: ") + counts_text(counts)]
    if latency_rows:
        md.append("- Latency counts: " + latency_counts_text(summary["latency_counts"]))
    md.append("")
    if meta.get("interrupted"):
        md += ["**Run interrupted** before every cell finished.", ""]
    if "lanes" in meta["steps"]:
        md += ["## CI lanes", "", "| Lane | Result | Tests (pass/fail/ignored) | Wall | First failure / reason |",
               "|---|---|---|---:|---|"]
        for lane in lanes:
            tests = (f"{lane.tests['passed']}/{lane.tests['failed']}/{lane.tests['ignored']}"
                     if lane.tests else "")
            note = (lane.first_failure or lane.reason or "").replace("|", "\\|").replace("\n", " ")
            md.append(f"| `{lane.lane}` | {lane.result} | {tests} | "
                      f"{fmt_s(lane.wall_s) if lane.wall_s is not None else ''} | {note} |")
        md.append("")
    if "panel" in meta["steps"]:
        md += ["## panel-play cells", ""] + md_cell_table(panel) + [""]
    if len(meta["steps"]) > 1:
        md += ["## tui-play cells", ""]
    md += md_cell_table(results)
    if latency_rows:
        proxy_sha = (meta.get("latency") or {}).get("proxy", {}).get("sha256")
        md += ["", "## Latency lane (tui, dev only, report-only)", "",
               f"Proxy `latency_proxy.py` sha256 `{proxy_sha}`. Bands are RTT in ms. "
               "Zero (`rtt0`) is the main run above. The class is the worst band class. "
               "Per-cell detail sits in `cells-latency/<band>/<cell>/latency.json`.", ""]
        md += md_latency_matrix(results, latency_rows)
    wtext(run_dir / "summary.md", "\n".join(md) + "\n")


def fmt_s(seconds: float | None) -> str:
    if seconds is None:
        return "-"
    seconds = int(round(seconds))
    hours, rest = divmod(seconds, 3600)
    minutes, secs = divmod(rest, 60)
    return f"{hours}h{minutes:02d}m{secs:02d}s" if hours else f"{minutes}m{secs:02d}s"


# ---------------------------------------------------------------- one branch


def run_dir_for(branch: str, commit: str, date: dt.date | None = None) -> Path:
    day = date or now().date()
    base = f"{day:%Y-%m-%d}-{slug(branch)}-{commit}{RUN_SUFFIX}"
    candidate = runs_root() / base
    attempt = 2
    while candidate.exists():
        candidate = runs_root() / f"{base}-r{attempt}"
        attempt += 1
    return candidate


def run_branch(branch: str, parallel: int, only: list[str] | None, only_lanes: list[str] | None,
               unavailable: str | None = None, date: dt.date | None = None,
               summary_only: bool = False, only_exact: bool = False,
               cells_file: str | None = None, latency_bands: list[int] | None = None,
               latency_cells: list[str] | None = None) -> Path:
    """One branch: sources, the steps in order, summary, DONE.

    `unavailable` writes a complete run where every lane and cell is
    UNAVAILABLE with that reason (nothing is built or started).
    `summary_only` is the logon catch-up for a missed night. It writes
    that same UNAVAILABLE run and does no other work. In particular it
    does not run the disk guard. See the guard call below.
    `only_exact` (a cells file) fails the run when a named cell is not in
    the catalog, instead of running the rest without a word: a committed
    cell list must not shrink silently when a cell is renamed or removed.
    `cells_file` only names that list in the summary.
    `latency_bands` overrides `NIGHTLY_LATENCY_BANDS` for this call; an
    explicit empty list turns the latency lane off. `latency_cells`
    overrides the cells file (`NIGHTLY_LATENCY_CELLS_FILE`).
    """
    ident = platform_id()
    target = ROOT / "target" / slug(branch)
    staging = runs_root() / f".building-{slug(branch)}"
    rmtree(staging)
    staging.mkdir(parents=True)
    started_wall = time.monotonic()
    started = now_iso()
    build_log = staging / "build.log"
    commit_full = commit = "unknown"
    tree: Path | None = None
    build_ok, build_error, binaries = True, None, {}
    build_started = time.monotonic()
    steps: dict[str, dict] = {name: {"status": "not started", "deadline_s": STEP_DEADLINE_S[name]}
                              for name in STEPS}
    session = None
    if REQUIRE_CONSOLE and unavailable is None:
        ok, detail, facts = console_session()
        session = {"status": detail, **facts}
        if not ok:
            unavailable = detail
    # The disk guard runs before the sources too, so a full disk cannot fail
    # the clone that the UNAVAILABLE summary needs for its cell list. A
    # summary-only catch-up skips the guard. Catch-up runs in a 30-minute
    # task (windows/install-tasks.ps1). A large removal there could be
    # killed at that limit before the summary exists, and the missed night
    # would leave no record at all. Catch-up builds nothing, so it needs no
    # cleared disk. The next scheduled night runs the guard again, inside
    # its 11-hour task, and removes any over-cap target then.
    if summary_only:
        log(f"{branch}: summary only; the disk guard does not run")
        disk_reason = None
    else:
        disk_reason = guard_disk(branch, ROOT / "target" / f"{slug(branch)}-ci" if "lanes" in STEPS else target)
    if unavailable is None:
        unavailable = disk_reason
    try:
        if FRESH_TREES:
            tree, commit_full, commit = fresh_tree(branch, build_log)
        else:
            tree = tree_path(branch)
            commit_full, commit = update_tree(branch, tree, build_log)
    except (subprocess.SubprocessError, OSError, RuntimeError, IndexError) as error:
        build_ok, build_error = False, f"sources: {error}"
    run_dir = run_dir_for(branch, commit, date)
    staging.rename(run_dir)
    build_log = run_dir / "build.log"
    log(f"{branch}: {commit} -> {run_dir.name}")

    skips = read_skips(HERE / "skip.conf")
    cells: list[str] = []
    try:
        if tree is not None:
            cells = catalog_cells(tree)
    except (OSError, ValueError, RuntimeError) as error:
        build_ok = False
        build_error = (build_error or "") + f"; cell list: {error}"
    if only and only_exact and cells:
        unknown = [cell for cell in only if cell not in cells]
        if unknown:
            build_ok = False
            build_error = (build_error or "") + (
                f"; cell list: {cells_file or 'cells file'} names not in the catalog: "
                + ", ".join(unknown))
    bands_wanted = LATENCY_BANDS if latency_bands is None else latency_bands
    latency_enabled = bool(bands_wanted) and branch == LATENCY_BRANCH and JOB == "main" and "tui" in STEPS
    if bands_wanted and not latency_enabled:
        log(f"{branch}: latency lane does not run here (branch {branch!r} vs {LATENCY_BRANCH!r}, "
            f"job {JOB}, steps {STEPS}); bands {bands_wanted} ignored for this branch")
    latency_names: list[str] = []
    if latency_enabled:
        if latency_cells is not None:
            latency_names = list(latency_cells)
        else:
            latency_path = cells_file_path(None, LATENCY_CELLS_FILE) or (HERE / "cells-latency.conf")
            try:
                latency_names = read_list(latency_path)
            except OSError as error:
                build_ok = False
                build_error = (build_error or "") + f"; latency cells file: {error}"
        if cells and latency_names:
            unknown = [cell for cell in latency_names if cell not in cells]
            if unknown:
                build_ok = False
                build_error = (build_error or "") + (
                    f"; latency cell list: names not in the catalog: " + ", ".join(unknown))
            quest_names = [cell for cell in latency_names if cell.startswith("quester_")]
            if quest_names:
                build_ok = False
                build_error = (build_error or "") + (
                    f"; latency cell list: quest cells stay out of the latency lane: "
                    + ", ".join(quest_names))
        if only:
            # A hand run with --cells narrows the latency set the same way,
            # so the zero baseline and the band rows name the same cells.
            latency_names = [cell for cell in latency_names if cell in only]
    latency_active = latency_enabled and bool(latency_names)
    if latency_enabled and not latency_names:
        log(f"{branch}: latency lane names no cells for this run; it writes no band rows")
    cells = select_job_cells(cells, only, JOB)
    cell_base = CellResult(cell="", result="NOT_RUN", branch=branch, commit=commit, platform=ident["platform"])
    lane_base = LaneResult(lane="", result="NOT_RUN", branch=branch, commit=commit, platform=ident["platform"])
    work = ROOT / "work" / run_dir.name
    lanes: list[LaneResult] = []
    panel: list[CellResult] = []
    results: list[CellResult] = []
    engine_status = "not started"

    def lane_rows(result: str, reason: str) -> list[LaneResult]:
        rows = []
        for name, args, _ in lane_list(tree):
            if only_lanes and name not in only_lanes:
                continue
            lane_dir = run_dir / "lanes" / name
            lane_dir.mkdir(parents=True, exist_ok=True)
            rows.append(write_lane(lane_dir, LaneResult(**{**asdict(lane_base), "lane": name, "result": result,
                                                          "command": "cargo " + " ".join(args), "reason": reason})))
        return rows

    if "lanes" in STEPS:
        step = steps["lanes"]
        step_start = time.monotonic()
        if note_quiet_lock():
            lanes, step["status"] = lane_rows("NOT_RUN", _quiet_lock_reason), f"not run: {_quiet_lock_reason}"
        elif unavailable:
            lanes, step["status"] = lane_rows("UNAVAILABLE", unavailable), f"unavailable: {unavailable}"
        elif tree is None or not build_ok:
            lanes, step["status"] = lane_rows("NOT_RUN", "sources failed; see build.log"), "not run: sources failed"
        else:
            disk_reason = guard_disk(branch, ROOT / "target" / f"{slug(branch)}-ci")
            if disk_reason:
                unavailable = disk_reason
                lanes, step["status"] = lane_rows("UNAVAILABLE", disk_reason), f"unavailable: {disk_reason}"
            else:
                ok, detail, _ = console_session()
                if not ok:
                    lanes, step["status"] = lane_rows("UNAVAILABLE", detail), f"unavailable: {detail}"
                elif note_quiet_lock():
                    lanes, step["status"] = lane_rows("NOT_RUN", _quiet_lock_reason), f"not run: {_quiet_lock_reason}"
                else:
                    try:
                        copy_engine(run_dir)
                    except QuietLockError as error:
                        lanes, step["status"] = lane_rows("NOT_RUN", str(error)), f"not run: {error}"
                    else:
                        lanes = run_lanes(tree, run_dir, work, lane_base, step_deadline("lanes", step_start),
                                          branch, only_lanes)
                        step["status"] = "ran: " + counts_text(count(lanes))
        step["wall_s"] = round(time.monotonic() - step_start, 1)

    cell_steps = [name for name in STEPS if name in ("panel", "tui")]
    build_ran = bool(cell_steps and not unavailable and not note_quiet_lock() and tree is not None and build_ok)
    if build_ran:
        disk_reason = guard_disk(branch, target)
        if disk_reason:
            unavailable, build_ran = disk_reason, False
    if build_ran:
        build_started = time.monotonic()
        try:
            binaries = build(tree, target, run_dir, build_log)
        except subprocess.TimeoutExpired:
            build_ok, build_error = False, f"build {deadline_text('build')} reached; see build.log"
        except (subprocess.CalledProcessError, OSError, RuntimeError) as error:
            build_ok, build_error = False, f"{error}"
        log(f"{branch}: build {'ok' if build_ok else 'FAILED'}")
    build_wall = round(time.monotonic() - build_started, 1)

    for frontend in cell_steps:
        step = steps[frontend]
        step_start = time.monotonic()
        reason_unavailable, reason_not_run = unavailable, None
        binary = binaries.get(f"{frontend}-play")
        if note_quiet_lock():
            reason_unavailable = f"MAC-QUIET: {_quiet_lock_reason}"
        if not reason_unavailable and not reason_not_run and REQUIRE_CONSOLE:
            ok, detail, _ = console_session()
            if not ok:
                reason_unavailable = detail
        if not reason_unavailable and not reason_not_run:
            try:
                ok, detail = engine_start(run_dir)
            except QuietLockError as error:
                ok, detail = False, str(error)
            engine_status = detail
            if not ok:
                if note_quiet_lock():
                    reason_unavailable = f"MAC-QUIET: {_quiet_lock_reason}"
                elif ENGINE_MODE == "remote":
                    reason_unavailable = remote_engine_failure(detail)
                else:
                    reason_not_run = f"engine {detail}"
        rows, runnable = cell_rows(frontend, cells, run_dir, cell_base, skips, reason_unavailable, reason_not_run)
        if runnable:
            assert binary is not None
            rows += run_cells(frontend, binary, runnable, run_dir, work, cell_base,
                              parallel if frontend == "tui" else PANEL_PARALLEL,
                              step_deadline(frontend, step_start), branch)
            step["status"] = "ran: " + counts_text(count(rows))
        elif reason_unavailable:
            step["status"] = f"unavailable: {reason_unavailable}"
        else:
            step["status"] = f"not run: {reason_not_run}"
        if ENGINE_MODE == "local":
            engine_stop(run_dir)
        order = {cell: index for index, cell in enumerate(cells)}
        rows.sort(key=lambda r: order.get(r.cell, len(order)))
        if frontend == "panel":
            panel = rows
        else:
            results = rows
        step["wall_s"] = round(time.monotonic() - step_start, 1)
    latency_rows: list[CellResult] = []
    latency_meta: dict | None = None
    if latency_active:
        step = steps["latency"] = {"status": "not started", "deadline_s": STEP_DEADLINE_S["latency"]}
        step_start = time.monotonic()
        latency_rows, latency_meta = run_latency_pass(
            branch, binaries.get("tui-play"), latency_names, bands_wanted, run_dir, work,
            cell_base, skips, parallel, {row.cell: row for row in results}, unavailable, build_ok)
        step["status"] = "ran: " + latency_counts_text(count_latency_classes(latency_rows))
        step["wall_s"] = round(time.monotonic() - step_start, 1)
    rmtree(work)
    if not build_ok and engine_status == "not started":
        engine_status = "not started (build failed)"
    meta = dict(
        job=JOB, run=run_dir.name, branch=branch, commit=commit, commit_full=commit_full, revision=int(REVISION),
        cells_file=cells_file,
        **ident, started=started, finished=now_iso(),
        wall_s=round(time.monotonic() - started_wall, 1), parallel=parallel,
        cell_cap_s=CELL_CAP_S, cell_caps_s=CELL_CAPS_S, run_deadline_s=RUN_DEADLINE_S, profile=PROFILE,
        interrupted=_stop.is_set() or _quiet_lock_reason is not None, quiet_lock=_quiet_lock_reason,
        steps=steps, session=session, unavailable=unavailable,
        build=dict(ok=build_ok, ran=build_ran, wall_s=build_wall, error=build_error,
                   tui_play_sha256=sha256(binaries["tui-play"]) if binaries.get("tui-play", Path()).is_file() else None,
                   panel_play_sha256=sha256(binaries["panel-play"]) if binaries.get("panel-play", Path()).is_file() else None),
        engine=dict(status=engine_status, mode=ENGINE_MODE,
                    source=(f"{ENGINE_PRISTINE} ({REVISION} engine copy, fresh DB)" if ENGINE_MODE == "local"
                            else f"{ENGINE_LABEL} (running, shared; never started or stopped), "
                                 f"{ENGINE_HOST}:{GAME_PORT}/{HTTP_PORT}; client data from {ENGINE_PRISTINE}")),
        catalog_commit=git_head(CATALOG), runner_commit=git_head(RUNNER_REPO),
        tree=str(tree) if tree else None,
    )
    # Informational only: a failed check reports UNAVAILABLE and never
    # changes a cell's outcome or the run's exit status.
    meta["rs2b0t_api"] = check_rs2b0t_api_drift(tree, CATALOG)
    if latency_meta is not None:
        meta["latency"] = latency_meta
    write_summary(run_dir, meta, lanes, panel, results, latency_rows)
    write_source_manifest(run_dir)
    wtext(run_dir / "DONE", meta["finished"] + "\n")
    sections = {"lanes": lanes, "panel": panel, "tui": results}
    if STEPS == ["tui"]:
        line = counts_text(count(results))
    else:
        line = "; ".join(f"{name} {counts_text(count(sections[name]))}" for name in STEPS if name in sections)
    if latency_rows:
        line += "; latency " + latency_counts_text(count_latency_classes(latency_rows))
    log(f"{branch}: summary {meta['run']}: {line}")
    return run_dir


def git_head(path: Path) -> str:
    try:
        return run(git_cmd("-C", str(path), "rev-parse", "--short=9", "HEAD")).stdout.strip() or "unknown"
    except (subprocess.CalledProcessError, OSError):
        return "unknown"


def prune_local() -> None:
    cutoff = time.time() - KEEP_DAYS * 86400
    runs = runs_root()
    for entry in runs.iterdir() if runs.is_dir() else []:
        if (entry.is_dir() and not entry.name.startswith(".")
                and entry.name.endswith(RUN_SUFFIX) and entry.stat().st_mtime < cutoff):
            rmtree(entry)
            log(f"pruned local run {entry.name} (older than {KEEP_DAYS} days; the archive keeps it)")


# ---------------------------------------------------------------- nights


def night_of(moment: dt.datetime) -> dt.date:
    """The date of the most recent scheduled start at or before `moment`."""
    hour, minute = (int(part) for part in NIGHT_START.split(":"))
    start = moment.replace(hour=hour, minute=minute, second=0, microsecond=0)
    return (start if moment >= start else start - dt.timedelta(days=1)).date()


def started_marker(day: dt.date) -> Path:
    suffix = "-quest" if JOB == "quest" else ""
    return ROOT / "state" / f"started-{day:%Y-%m-%d}{suffix}"


def on_signal(signum, _frame) -> None:
    log(f"signal {signum}: stopping cells")
    _stop.set()
    with _children_lock:
        kills = list(_children.values())
    for kill in kills:
        kill()


def take_lock():
    lock = open(ROOT / ".nightly.lock", "w")
    try:
        if IS_WINDOWS:
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return None
    return lock


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--branch", action="append", help="branch to test (repeatable; default branches.conf)")
    parser.add_argument("--cells", help="comma-separated cell names (default: every catalog cell)")
    parser.add_argument("--cells-file", type=Path,
                        help="file with one cell name per line (default NIGHTLY_CELLS_FILE; "
                             "a relative path is read beside this script; --cells wins)")
    parser.add_argument("--lanes", help="comma-separated CI lane names (default: every lane)")
    parser.add_argument("--parallel", type=int, default=int(os.environ.get("NIGHTLY_PARALLEL", "4")))
    parser.add_argument("--latency-bands",
                        help="comma-separated RTT bands in ms for the latency lane, "
                             "for example 50,100,150 (default NIGHTLY_LATENCY_BANDS; "
                             "an empty value turns the lane off)")
    parser.add_argument("--list", action="store_true", help="print the cell list of a checked-out tree and exit")
    parser.add_argument("--tree", type=Path, help="tree for --list")
    parser.add_argument("--catch-up", action="store_true",
                        help="at logon: if the last scheduled night never started, write it as UNAVAILABLE")
    args = parser.parse_args()
    latency_bands: list[int] | None = None
    if args.latency_bands is not None:
        try:
            latency_bands = parse_latency_bands(args.latency_bands)
        except ValueError as error:
            parser.error(f"--latency-bands: {error}")
    if args.list:
        skips = read_skips(HERE / "skip.conf")
        listed = catalog_cells(args.tree or Path.cwd())
        for cell in listed:
            print(f"{cell}\tSKIP: {skips[cell]}" if cell in skips else cell)
        bands = LATENCY_BANDS if latency_bands is None else latency_bands
        if bands:
            latency_path = cells_file_path(None, LATENCY_CELLS_FILE) or (HERE / "cells-latency.conf")
            try:
                latency_names = read_list(latency_path)
            except OSError as error:
                print(f"# latency lane: cannot read {latency_path}: {error}")
                return 1
            unknown = [cell for cell in latency_names if cell not in listed]
            if unknown:
                print(f"# latency lane: names not in the catalog: {', '.join(unknown)}")
                return 1
            print(f"# latency lane: branch {LATENCY_BRANCH}, bands "
                  f"{','.join(str(band) for band in bands)} ms RTT, {len(latency_names)} cells, "
                  f"list {latency_path.name}")
            for cell in latency_names:
                for band in bands:
                    print(f"# latency\t{cell}\t{band_name(band)}")
        return 0
    branches = args.branch or os.environ.get("NIGHTLY_BRANCHES", "").split() or read_list(HERE / "branches.conf")
    only = [c.strip() for c in args.cells.split(",") if c.strip()] if args.cells else None
    only_exact, cells_file = False, None
    if only is None:
        cells_path = cells_file_path(args.cells_file, os.environ.get("NIGHTLY_CELLS_FILE"))
        if cells_path is not None:
            only, only_exact, cells_file = read_list(cells_path), True, cells_path.name
            if not only:
                log(f"cells file {cells_path} names no cells; exiting")
                return 1
    only_lanes = [c.strip() for c in args.lanes.split(",") if c.strip()] if args.lanes else None
    if note_quiet_lock():
        log("nightly skipped before start because MAC-QUIET.lock is present")
        return 0
    runs_root().mkdir(parents=True, exist_ok=True)
    (ROOT / "state").mkdir(parents=True, exist_ok=True)
    lock = take_lock()
    if lock is None:
        log("another nightly run holds the lock; exiting")
        return 1
    for name in ("SIGTERM", "SIGINT", "SIGBREAK"):
        if hasattr(signal, name):
            signal.signal(getattr(signal, name), on_signal)
    moment = now()
    if args.catch_up:
        night = night_of(moment)
        if started_marker(night).exists():
            log(f"catch-up: the night of {night} started; nothing to record")
            return 0
        reason = (f"BotTest console session not active at the scheduled {NIGHT_START} start on {night} "
                  f"(the interactive task cannot run without it; recorded at logon {moment.isoformat(timespec='seconds')})")
        wtext(started_marker(night), moment.isoformat(timespec="seconds") + " catch-up\n")
        for branch in branches:
            run_branch(branch, args.parallel, only, only_lanes, unavailable=reason, date=night,
                       summary_only=True, only_exact=only_exact, cells_file=cells_file,
                       latency_bands=latency_bands)
        return 0
    wtext(started_marker(night_of(moment)), moment.isoformat(timespec="seconds") + "\n")
    global _run_deadline
    if RUN_DEADLINE_S:
        _run_deadline = time.monotonic() + RUN_DEADLINE_S
    log(f"nightly start: branches {branches}, steps {STEPS}, parallel {args.parallel}"
        + (f", latency bands {LATENCY_BANDS if latency_bands is None else latency_bands}"
           if (LATENCY_BANDS if latency_bands is None else latency_bands) else ""))
    for branch in branches:
        if _stop.is_set() or note_quiet_lock():
            break
        run_branch(branch, args.parallel, only, only_lanes, only_exact=only_exact, cells_file=cells_file,
                   latency_bands=latency_bands)
    if FRESH_TREES:
        prune_trees(branches)
    prune_local()
    log("nightly done")
    return 0


if __name__ == "__main__":
    sys.exit(main())
