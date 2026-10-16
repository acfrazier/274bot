#!/usr/bin/env python3
"""274bot nightly live run: build `tui-play` per branch and run every live cell.

Runs on the nightly VM from `274bot-nightly.service`, and on the Windows nightly
host from the `274bot-nightly` scheduled task (see README.md). For each branch in
`branches.conf` (or `--branch`, `NIGHTLY_BRANCHES`), it fetches the branch tip
anonymously, builds the frontends (`--release --locked`), points them at an
engine, runs every cell in the checked-out scenario catalog with a few in
parallel, and writes `summary.json`, `summary.md` and per-cell receipts into
`<root>/runs/<date>-<branch>-<commit>[-windows]/`. `DONE` is written last; the
archive host only pulls runs that have it.

Linux (malbec): one step, `tui` cells on a pty against a private engine copy it
starts and stops. Windows: three steps in order, each with its own deadline:
the CI lanes, the cells through `panel-play` in the console session, and the
cells through `tui-play` on ConPTY, against a running engine it never starts or
stops (Engine B).

Standard library only (Python 3.10+); on Windows also `pywinpty` for ConPTY
(pinned in windows/requirements.txt).
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
import subprocess
import sys
import threading
import time
import urllib.request
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Callable
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

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
ROOT = Path(os.environ.get("NIGHTLY_ROOT", r"C:\274bot-nightly" if IS_WINDOWS else "/srv/nightly"))
REPO_URL = os.environ.get("NIGHTLY_REPO", "https://github.com/acfrazier/274bot")
PROFILE = os.environ.get("NIGHTLY_PROFILE", "local-289")


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
NAME_PREFIX = os.environ.get("NIGHTLY_NAME_PREFIX", "nw" if IS_WINDOWS else "nt")
PANEL_NAME_PREFIX = os.environ.get("NIGHTLY_PANEL_NAME_PREFIX", "np")
KEEP_DAYS = int(os.environ.get("NIGHTLY_KEEP_DAYS", "14"))
# Every cell enforces its own scenario deadline inside the frontend; this outer
# cap only stops a process that never reaches a terminal state.
CELL_CAP_S = int(os.environ.get("NIGHTLY_CELL_CAP_S", "3600"))
EXE = ".exe" if IS_WINDOWS else ""
# Runs of one commit on two platforms must not collide on the archive dataset.
RUN_SUFFIX = os.environ.get("NIGHTLY_RUN_SUFFIX", "-windows" if IS_WINDOWS else "")
# "local": a private engine copy this runner starts and stops (malbec).
# "remote": an engine that is already running; never started, stopped or changed.
ENGINE_MODE = os.environ.get("NIGHTLY_ENGINE", "remote" if IS_WINDOWS else "local")
ENGINE_HOST = os.environ.get("NIGHTLY_ENGINE_HOST", "127.0.0.1")
GAME_PORT = int(os.environ.get("NIGHTLY_GAME_PORT", "47594"))
HTTP_PORT = int(os.environ.get("NIGHTLY_HTTP_PORT", "3080"))
# The 289 engine's 40-byte /crc identity (ENGINE.md); empty skips the comparison.
ENGINE_CRC_SHA256 = os.environ.get(
    "NIGHTLY_ENGINE_CRC_SHA256", "687190c8ef08bb94a1422f4262d62bb7a76a55dfe0634ea43f0e63e0235dc623")
ENGINE_LABEL = os.environ.get("NIGHTLY_ENGINE_LABEL", "Engine B")
# Fresh exact-commit clone per run from local bare mirrors (Windows), instead of
# one reused checkout per branch (malbec).
FRESH_TREES = env_flag("NIGHTLY_FRESH_TREES", IS_WINDOWS)
KEEP_TREES = int(os.environ.get("NIGHTLY_KEEP_TREES", "2"))
# The BotTest console session is required for the GPU and the panel windows.
REQUIRE_CONSOLE = env_flag("NIGHTLY_REQUIRE_CONSOLE", IS_WINDOWS)
STEPS = [s for s in os.environ.get("NIGHTLY_STEPS", "lanes,panel,tui" if IS_WINDOWS else "tui").split(",") if s]
STEP_DEADLINE_S = {
    "lanes": int(os.environ.get("NIGHTLY_LANES_DEADLINE_S", "10800")),
    "build": int(os.environ.get("NIGHTLY_BUILD_DEADLINE_S", "5400")),
    "panel": int(os.environ.get("NIGHTLY_PANEL_DEADLINE_S", "12600")),
    "tui": int(os.environ.get("NIGHTLY_TUI_DEADLINE_S", "21600")),
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
ENGINE_PRISTINE = ROOT / "engine-pristine"
ENGINE_RUN = ROOT / "engine-run"
CATALOG = ROOT / "rs2b0t"
NAV_SNAPSHOT = ROOT / "nav-snapshot"
NAV_FIXTURE = ROOT / "fixtures" / "bake-r2-a.navpack"
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
        log_to: Path | None = None, check: bool = True, timeout: float | None = None) -> subprocess.CompletedProcess:
    """Run a command to completion; with `timeout`, its whole process tree is stopped at the bound."""
    if log_to is None:
        return subprocess.run(cmd, cwd=cwd, env=env, check=check, text=True, encoding="utf-8",
                              errors="replace", stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              timeout=timeout)
    with log_to.open("a", encoding="utf-8") as out:
        out.write(f"$ {' '.join(cmd)}\n")
        out.flush()
        proc = popen_group(cmd, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=out,
                           stderr=subprocess.STDOUT)
        track(proc.pid, lambda: kill_tree(proc.pid, proc))
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            kill_tree(proc.pid, proc)
            proc.wait()
            raise
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


def rmtree(path: Path) -> None:
    """Remove a tree, including git's read-only object files on Windows."""
    def retry(func, target, _exc):
        os.chmod(target, stat.S_IWRITE)
        func(target)
    if path.exists():
        if sys.version_info >= (3, 12):
            shutil.rmtree(path, onexc=retry)
        else:
            shutil.rmtree(path, onerror=retry)


def read_list(path: Path) -> list[str]:
    rows = []
    for line in rtext(path).splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            rows.append(line)
    return rows


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


# ---------------------------------------------------------------- sources


def update_tree(branch: str, tree: Path, build_log: Path) -> tuple[str, str]:
    env = git_env()
    if not (tree / ".git").exists():
        tree.parent.mkdir(parents=True, exist_ok=True)
        run(git_cmd("clone", "--no-checkout", REPO_URL, str(tree)), env=env, log_to=build_log)
    git = git_cmd("-C", str(tree))
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


def fresh_tree(branch: str, build_log: Path) -> tuple[Path, str, str]:
    """Fetch the branch tip into the bare mirrors and clone it into a new tree.

    The tree holds exactly that commit and the pinned submodule commits; it is
    never reused. Only the mirrors persist between nights.
    """
    env = git_env()
    mirrors = ROOT / "mirror"
    root_mirror = mirrors / "274bot.git"
    mirror_fetch(REPO_URL, root_mirror, [f"+refs/heads/{branch}:refs/heads/{branch}"], build_log)
    full = run(git_cmd("-C", str(root_mirror), "rev-parse", f"refs/heads/{branch}^{{commit}}")).stdout.strip()
    stamp = now().strftime("%Y%m%dT%H%M%S")
    tree = ROOT / "trees" / f"{slug(branch)}-{full[:9]}-{stamp}"
    tree.parent.mkdir(parents=True, exist_ok=True)
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


def prune_trees() -> None:
    trees = ROOT / "trees"
    entries = sorted((p for p in trees.iterdir() if p.is_dir()), key=lambda p: p.stat().st_mtime) \
        if trees.is_dir() else []
    for entry in entries[:-KEEP_TREES] if KEEP_TREES > 0 else entries:
        rmtree(entry)
        log(f"removed tree {entry.name} (keeping the last {KEEP_TREES})")


def build_env(target: Path, **extra: str) -> dict:
    # host-play's build script bakes the bundled navigation from the engine's content
    # and a decoded client-cache snapshot (BOT_NAV_SNAPSHOT_ROOT).
    return dict(os.environ, CARGO_TARGET_DIR=str(target), ENGINE_DIR=str(ENGINE_PRISTINE / "engine"),
                BOT_NAV_SNAPSHOT_ROOT=str(NAV_SNAPSHOT), **extra)


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
        run_tui(tui_cmd(skip_target / "release" / f"tui-play{EXE}", "walk"),
                cell_env(home, tmp, work / "evidence", "walk", NAME_PREFIX), work, work / "stderr.log", 600)
    finally:
        engine_stop(None)
    decoded = [path.parent for path in
               (home / ".274bot" / "unpack-289").glob("revision-289/*/*/*/versionlist")]
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


def build(tree: Path, target: Path, run_dir: Path, build_log: Path) -> dict[str, Path]:
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
        return False, f"{url}: {len(body)} bytes, sha256 {digest}, expected 40 bytes {ENGINE_CRC_SHA256}"
    try:
        socket.create_connection((ENGINE_HOST, GAME_PORT), timeout=15).close()
    except OSError as error:
        return False, f"{ENGINE_HOST}:{GAME_PORT}: {error}"
    return True, f"{ENGINE_LABEL} at {ENGINE_HOST}:{GAME_PORT}/{HTTP_PORT}, /crc sha256 {digest[:16]}"


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
        run([str(ENGINE_RUN / "start-289.sh")], cwd=ENGINE_RUN, log_to=run_dir / "engine-control.log")
    except subprocess.CalledProcessError as error:
        return False, f"failed to start: {error}"
    return True, "fresh copy started, stopped after the run"


def engine_stop(run_dir: Path | None) -> None:
    if ENGINE_MODE == "remote" or not (ENGINE_RUN / "runtime" / "pids" / "engine.pid").exists():
        return
    control = run_dir / "engine-control.log" if run_dir else None
    run([str(ENGINE_RUN / "stop-289.sh")], cwd=ENGINE_RUN, log_to=control, check=False)
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
        if value.get("scenario") == cell and value.get("outcome") in ("PASS", "FAIL"):
            found = value
    return found


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


def cell_env(home: Path, tmp: Path, evidence: Path, cell: str, prefix: str) -> dict:
    env = base_env()
    env.update(private_dirs(home, tmp))
    env.update({
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "BOT_LIVE_NAME_PREFIX": prefix,
        "LIVE_EVIDENCE_DIR": str(evidence),
        "RS2B0T": str(CATALOG),
        "ENGINE_DIR": str(ENGINE_RUN / "engine"),
    })
    if not GPU_CELLS.search(cell):
        env["BOT_CPU"] = "1"
    return env


def engine_flags() -> list[str]:
    if ENGINE_MODE != "remote":
        return []
    return ["--host", ENGINE_HOST, "--port", str(GAME_PORT), "--asset-host", ENGINE_HOST,
            "--http-port", str(HTTP_PORT)]


def tui_cmd(binary: Path, cell: str, gate: str | None = None) -> list[str]:
    cmd = [str(binary), "--profile", PROFILE, *engine_flags(), "--engine", str(ENGINE_RUN / "engine"),
           "--catalog", str(CATALOG), "--live", f"script_{cell}"]
    return cmd + [gate] if gate else cmd


def run_tui(cmd: list[str], env: dict, cwd: Path, stderr_path: Path, cap_s: float) -> tuple[int, bool, str]:
    """Run tui-play on a pseudo-terminal (it draws a terminal UI) with stderr kept apart.

    Returns (exit code, hit the cap, terminal transcript without escape codes).
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
    with stderr_path.open("wb") as stderr:
        proc = popen_group(cmd, env=env, cwd=cwd, stdin=slave, stdout=slave, stderr=stderr)
        os.close(slave)
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        track(proc.pid, lambda: kill_tree(proc.pid, proc))
        try:
            while True:
                try:
                    proc.wait(timeout=5)
                    break
                except subprocess.TimeoutExpired:
                    if _stop.is_set():
                        kill_tree(proc.pid, proc)
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
    return proc.returncode, timed_out, tty_text


def run_tui_conpty(cmd: list[str], env: dict, cwd: Path, stderr_path: Path, cap_s: float) -> tuple[int, bool, str]:
    """Windows: the same on ConPTY (pywinpty). A `cmd /c` shim inside the console
    sends stderr to its file; the console carries stdin, stdout and the screen."""
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
            if time.monotonic() - started > cap_s:
                timed_out = True
                kill_tree(pid)
                break
            time.sleep(0.05)
    finally:
        untrack(pid)
    code = console.get_exitstatus()
    tty_text = ANSI.sub("", "".join(chunks)).replace("\r", "")
    return (code if code is not None else -1), timed_out, tty_text


def panel_cmd(binary: Path, cell: str) -> list[str]:
    return [str(binary), "--profile", PROFILE, *engine_flags(), "--engine", str(ENGINE_RUN / "engine"),
            "--catalog", str(CATALOG), "--live", f"script_{cell}"]


def run_panel(cmd: list[str], env: dict, cwd: Path, stdout_path: Path, stderr_path: Path,
              cap_s: float) -> tuple[int, bool]:
    """Run panel-play as a windowed process in this (console) session."""
    started = time.monotonic()
    timed_out = False
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        proc = popen_group(cmd, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        track(proc.pid, lambda: kill_tree(proc.pid, proc))
        try:
            while True:
                try:
                    proc.wait(timeout=5)
                    break
                except subprocess.TimeoutExpired:
                    if _stop.is_set():
                        kill_tree(proc.pid, proc)
                        break
                    if time.monotonic() - started > cap_s:
                        timed_out = True
                        kill_tree(proc.pid, proc)
                        break
        finally:
            untrack(proc.pid)
        proc.wait()
    return proc.returncode, timed_out


def cells_root(run_dir: Path, frontend: str) -> Path:
    return run_dir / ("cells" if frontend == "tui" else f"{frontend}-cells")


def run_cell(cell: str, frontend: str, binary: Path, run_dir: Path, work: Path, base: CellResult,
             deadline: float) -> CellResult:
    result = CellResult(**{**asdict(base), "cell": cell, "frontend": frontend,
                           "renderer": "gpu" if GPU_CELLS.search(cell) else "cpu"})
    cell_dir = cells_root(run_dir, frontend) / cell
    cell_dir.mkdir(parents=True, exist_ok=True)
    if _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted before this cell started"
        return write_cell(cell_dir, result)
    if time.monotonic() >= deadline:
        result.result = "NOT_RUN"
        result.reason = f"{deadline_text(frontend)} reached before this cell started"
        return write_cell(cell_dir, result)
    scratch = work / frontend / cell
    home = scratch / "home"
    tmp = scratch / "tmp"
    prefix = PANEL_NAME_PREFIX if frontend == "panel" else NAME_PREFIX
    started = time.monotonic()
    result.started = now_iso()
    gate = None
    stdout_name = "tty.log" if frontend == "tui" else "stdout.log"
    while True:
        rmtree(scratch)
        home.mkdir(parents=True)
        tmp.mkdir(parents=True)
        cap = min(CELL_CAP_S, max(1.0, deadline - time.monotonic()))
        cut_by_deadline = cap < CELL_CAP_S
        env = cell_env(home, tmp, cell_dir / "evidence", cell, prefix)
        if frontend == "tui":
            cmd = tui_cmd(binary, cell, gate)
            wtext(cell_dir / "command.txt", " ".join(cmd) + "\n")
            exit_code, timed_out, out_text = run_tui(cmd, env, scratch, cell_dir / "stderr.log", cap)
        else:
            cmd = panel_cmd(binary, cell)
            wtext(cell_dir / "command.txt", " ".join(cmd) + "\n")
            exit_code, timed_out = run_panel(cmd, env, scratch, cell_dir / stdout_name,
                                             cell_dir / "stderr.log", cap)
            out_text = rtext(cell_dir / stdout_name)
        stderr_text = rtext(cell_dir / "stderr.log")
        # A paired proof refuses to start without the pair witness and says so;
        # run it again under that gate (the same flag a person would pass).
        if frontend == "tui" and gate is None and exit_code != 0 and PAIR_GATE_REFUSAL in stderr_text:
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
    receipt = find_receipt(combined, cell)
    verdict = re.search(rf"\b(PASS|FAIL): live {re.escape(cell)}\b", combined)
    if receipt is not None:
        result.receipt = receipt
        result.result = receipt["outcome"]
        result.ticks = receipt.get("ticks")
        wtext(cell_dir / "receipt.json", json.dumps(receipt, indent=1) + "\n")
        if result.result == "FAIL":
            result.first_failure = (receipt.get("message") or receipt.get("predicate") or "")[:400]
    elif verdict is not None:
        # The verdict line printed but its JSON record did not parse.
        result.result = verdict.group(1)
        result.reason = "verdict line without a parsable JSON record"
        if result.result == "FAIL":
            result.first_failure = first_failure_line(stderr_text) or verdict.group(0)
    elif timed_out:
        result.result = "TIMEOUT"
        bound = f"{frontend} step deadline reached" if cut_by_deadline else f"runner cap {CELL_CAP_S}s reached"
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
    rmtree(scratch)
    return write_cell(cell_dir, result)


def write_cell(cell_dir: Path, result: CellResult) -> CellResult:
    row = asdict(result)
    row.pop("receipt")
    wtext(cell_dir / "result.json", json.dumps(row, indent=1) + "\n")
    return result


def cell_rows(frontend: str, cells: list[str], run_dir: Path, base: CellResult, skips: dict[str, str],
              unavailable: str | None, not_run: str | None) -> tuple[list[CellResult], list[str]]:
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
        cell_dir = cells_root(run_dir, frontend) / cell
        cell_dir.mkdir(parents=True, exist_ok=True)
        rows.append(write_cell(cell_dir, row))
    return rows, runnable


def run_cells(frontend: str, binary: Path, cells: list[str], run_dir: Path, work: Path, base: CellResult,
              parallel: int, deadline: float, branch: str) -> list[CellResult]:
    results = []
    log(f"{branch}: {frontend}: running {len(cells)} cells, {parallel} in parallel")
    with concurrent.futures.ThreadPoolExecutor(max_workers=parallel) as pool:
        futures = {pool.submit(run_cell, cell, frontend, binary, run_dir, work, base, deadline): cell
                   for cell in cells}
        for future in concurrent.futures.as_completed(futures):
            outcome = future.result()
            results.append(outcome)
            log(f"{branch}: {frontend}: {outcome.cell} {outcome.result} ({fmt_s(outcome.wall_s)})")
    return results


# ---------------------------------------------------------------- CI lanes


def lane_list(tree: Path | None) -> list[tuple[str, list[str], dict]]:
    """The Windows platform gate's lane set (gate.ps1), run with the GPU on.

    `test-panel-memory` runs every panel target, including the `ime_frame`
    integration test that needs an interactive desktop (the console session).
    `cargo fmt` runs per crate: a workspace-root fmt hits OS error 206 on Windows.
    """
    vendor = ["--manifest-path", "vendor/fr-client-rust/Cargo.toml"]
    nav = {"NAV_PACK": str(NAV_FIXTURE)}
    engine = {"ENGINE_DIR": str(ENGINE_RUN / "engine")}
    lanes = [
        ("build", ["build", "--locked", "--workspace", "--all-targets", "--features", GATE_FEATURES], {}),
        ("build-vendor", ["build", "--locked", *vendor, "--workspace"], {}),
        ("clippy", ["clippy", "--locked", "--workspace", "--all-targets", "--features", GATE_FEATURES,
                    "--", "-D", "warnings"], {}),
        ("clippy-vendor", ["clippy", "--locked", *vendor, "--workspace", "--all-targets", "--", "-D", "warnings"], {}),
        ("test-workspace", ["test", "--locked", "--workspace", "--no-fail-fast"], {}),
        ("test-vendor", ["test", "--locked", *vendor, "--workspace", "--no-fail-fast"], engine),
        ("test-script-load", ["test", "--locked", "-p", "script", "--features", "load", "--tests",
                              "--no-fail-fast"], nav),
        ("test-script-no-default", ["test", "--locked", "-p", "script", "--no-default-features",
                                    "--no-fail-fast"], nav),
        ("test-host-play-memory", ["test", "--locked", "-p", "host-play", "--features", "memory-profile",
                                   "--no-fail-fast"], {}),
        ("test-host-play-memory-no-alloc", ["test", "--locked", "-p", "host-play", "--features",
                                            "memory-profile-no-alloc", "--lib", "--no-fail-fast"], {}),
        ("test-host-performance", ["test", "--locked", "-p", "host", "--features", "performance-profile",
                                   "--no-fail-fast"], {}),
        ("test-panel-memory", ["test", "--locked", "-p", "panel", "--features", "memory-profile",
                               "--no-fail-fast"], {}),
        ("test-panel-render-diagnostics", ["test", "--locked", "-p", "panel", "--features",
                                           "render-diagnostics", "--lib", "--no-fail-fast"], {}),
        ("test-tui-memory", ["test", "--locked", "-p", "tui", "--features", "memory-profile", "--no-fail-fast"], {}),
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
    for name in ("NAV_PACK", "ENGINE_DIR", "WORLD_ENGINE_DIR", "LIVE", "BOT_NAV_SNAPSHOT_ROOT", "SKIP_GPU",
                 "BOT_CPU", "CARGO_TARGET_DIR"):
        env.pop(name, None)
    env.update(private_dirs(home, tmp))
    env.update(CARGO_TARGET_DIR=str(target), BOT_NAV_BUILD="skip", CARGO_INCREMENTAL="0",
               CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never", **extra)
    return env


def run_lane(name: str, args: list[str], extra: dict, tree: Path, target: Path, run_dir: Path, work: Path,
             base: LaneResult, deadline: float) -> LaneResult:
    result = LaneResult(**{**asdict(base), "lane": name, "command": "cargo " + " ".join(args)})
    lane_dir = run_dir / "lanes" / name
    lane_dir.mkdir(parents=True, exist_ok=True)
    if _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted before this lane started"
        return write_lane(lane_dir, result)
    if time.monotonic() >= deadline:
        result.result = "NOT_RUN"
        result.reason = f"{deadline_text('lanes')} reached before this lane started"
        return write_lane(lane_dir, result)
    scratch = work / "lanes" / name
    rmtree(scratch)
    home, tmp = scratch / "home", scratch / "tmp"
    tmp.mkdir(parents=True)
    # The decoded 289 snapshot the client tests unpack against, as a private copy.
    for snapshot in NAV_SNAPSHOT.glob("*/versionlist"):
        shutil.copytree(snapshot.parent, home / ".274bot" / "unpack" / snapshot.parent.name)
    home.mkdir(parents=True, exist_ok=True)
    cargo_log = lane_dir / "cargo.log"
    result.started = now_iso()
    started = time.monotonic()
    timed_out = False
    exit_code = None
    try:
        exit_code = run(["cargo", *args], cwd=tree, env=lane_env(home, tmp, target, extra), log_to=cargo_log,
                        check=False, timeout=max(1.0, deadline - time.monotonic())).returncode
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


def write_summary(run_dir: Path, meta: dict, lanes: list[LaneResult], panel: list[CellResult],
                  results: list[CellResult]) -> None:
    counts = count(results)
    summary = dict(meta, counts=counts, cells=[{k: v for k, v in asdict(r).items() if k != "receipt"}
                                                for r in results])
    if "lanes" in meta["steps"]:
        summary["lane_counts"] = count(lanes)
        summary["lanes"] = [asdict(r) for r in lanes]
    if "panel" in meta["steps"]:
        summary["panel_counts"] = count(panel)
        summary["panel_cells"] = [{k: v for k, v in asdict(r).items() if k != "receipt"} for r in panel]
    wtext(run_dir / "summary.json", json.dumps(summary, indent=1) + "\n")

    md = [
        f"# Nightly {meta['run']}",
        "",
        f"- Branch `{meta['branch']}` at `{meta['commit_full']}`",
        f"- Platform {meta['platform']} ({meta['os']}, host {meta['host']})",
        f"- Started {meta['started']}, finished {meta['finished']}, wall {fmt_s(meta['wall_s'])}",
        f"- Build: {'FAILED' if not meta['build']['ok'] else 'ok' if meta['build'].get('ran', True) else 'not run'}"
        f" ({fmt_s(meta['build']['wall_s'])})"
        + (f": {meta['build']['error']}" if meta['build'].get('error') else ""),
        f"- Engine: {meta['engine']['status']}; catalog `{meta['catalog_commit']}`; "
        f"{meta['parallel']} cells in parallel; outer cap {meta['cell_cap_s']} s",
    ]
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
    md += [("- Counts: " if len(meta["steps"]) == 1 else "- TUI cell counts: ") + counts_text(counts), ""]
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
    candidate = ROOT / "runs" / base
    attempt = 2
    while candidate.exists():
        candidate = ROOT / "runs" / f"{base}-r{attempt}"
        attempt += 1
    return candidate


def run_branch(branch: str, parallel: int, only: list[str] | None, only_lanes: list[str] | None,
               unavailable: str | None = None, date: dt.date | None = None) -> Path:
    """One branch: sources, the steps in order, summary, DONE.

    `unavailable` writes a complete run where every lane and cell is
    UNAVAILABLE with that reason (nothing is built or started).
    """
    ident = platform_id()
    target = ROOT / "target" / slug(branch)
    staging = ROOT / "runs" / f".building-{slug(branch)}"
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
    try:
        if FRESH_TREES:
            tree, commit_full, commit = fresh_tree(branch, build_log)
        else:
            tree = ROOT / "trees" / slug(branch)
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
    if only:
        cells = [cell for cell in cells if cell in only]
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
        if unavailable:
            lanes, step["status"] = lane_rows("UNAVAILABLE", unavailable), f"unavailable: {unavailable}"
        elif tree is None or not build_ok:
            lanes, step["status"] = lane_rows("NOT_RUN", "sources failed; see build.log"), "not run: sources failed"
        else:
            ok, detail, _ = console_session()
            if not ok:
                lanes, step["status"] = lane_rows("UNAVAILABLE", detail), f"unavailable: {detail}"
            else:
                copy_engine(run_dir)
                lanes = run_lanes(tree, run_dir, work, lane_base, step_deadline("lanes", step_start), branch,
                                  only_lanes)
                step["status"] = "ran: " + counts_text(count(lanes))
        step["wall_s"] = round(time.monotonic() - step_start, 1)

    cell_steps = [name for name in STEPS if name in ("panel", "tui")]
    build_ran = bool(cell_steps and not unavailable and tree is not None and build_ok)
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
        if not reason_unavailable and (not build_ok or binary is None):
            reason_not_run = "build failed; see build.log"
        if not reason_unavailable and not reason_not_run and REQUIRE_CONSOLE:
            ok, detail, _ = console_session()
            if not ok:
                reason_unavailable = detail
        if not reason_unavailable and not reason_not_run:
            ok, detail = engine_start(run_dir)
            engine_status = detail
            if not ok:
                if ENGINE_MODE == "remote":
                    reason_unavailable = f"{ENGINE_LABEL} not reachable: {detail}"
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
    rmtree(work)
    if not build_ok and engine_status == "not started":
        engine_status = "not started (build failed)"
    meta = dict(
        run=run_dir.name, branch=branch, commit=commit, commit_full=commit_full,
        **ident, started=started, finished=now_iso(),
        wall_s=round(time.monotonic() - started_wall, 1), parallel=parallel,
        cell_cap_s=CELL_CAP_S, run_deadline_s=RUN_DEADLINE_S, profile=PROFILE, interrupted=_stop.is_set(),
        steps=steps, session=session, unavailable=unavailable,
        build=dict(ok=build_ok, ran=build_ran, wall_s=build_wall, error=build_error,
                   tui_play_sha256=sha256(binaries["tui-play"]) if binaries.get("tui-play", Path()).is_file() else None,
                   panel_play_sha256=sha256(binaries["panel-play"]) if binaries.get("panel-play", Path()).is_file() else None),
        engine=dict(status=engine_status, mode=ENGINE_MODE,
                    source=("engine-pristine (289 engine copy, fresh DB)" if ENGINE_MODE == "local"
                            else f"{ENGINE_LABEL} (running, shared; never started or stopped), "
                                 f"{ENGINE_HOST}:{GAME_PORT}/{HTTP_PORT}; client data from engine-pristine")),
        catalog_commit=git_head(CATALOG), runner_commit=git_head(HERE.parent.parent),
        tree=str(tree) if tree else None,
    )
    write_summary(run_dir, meta, lanes, panel, results)
    wtext(run_dir / "DONE", meta["finished"] + "\n")
    sections = {"lanes": lanes, "panel": panel, "tui": results}
    if STEPS == ["tui"]:
        line = counts_text(count(results))
    else:
        line = "; ".join(f"{name} {counts_text(count(sections[name]))}" for name in STEPS if name in sections)
    log(f"{branch}: summary {meta['run']}: {line}")
    return run_dir


def git_head(path: Path) -> str:
    try:
        return run(git_cmd("-C", str(path), "rev-parse", "--short=9", "HEAD")).stdout.strip() or "unknown"
    except (subprocess.CalledProcessError, OSError):
        return "unknown"


def prune_local() -> None:
    cutoff = time.time() - KEEP_DAYS * 86400
    runs = ROOT / "runs"
    for entry in runs.iterdir() if runs.is_dir() else []:
        if entry.is_dir() and not entry.name.startswith(".") and entry.stat().st_mtime < cutoff:
            rmtree(entry)
            log(f"pruned local run {entry.name} (older than {KEEP_DAYS} days; the archive keeps it)")


# ---------------------------------------------------------------- nights


def night_of(moment: dt.datetime) -> dt.date:
    """The date of the most recent scheduled start at or before `moment`."""
    hour, minute = (int(part) for part in NIGHT_START.split(":"))
    start = moment.replace(hour=hour, minute=minute, second=0, microsecond=0)
    return (start if moment >= start else start - dt.timedelta(days=1)).date()


def started_marker(day: dt.date) -> Path:
    return ROOT / "state" / f"started-{day:%Y-%m-%d}"


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
    parser.add_argument("--lanes", help="comma-separated CI lane names (default: every lane)")
    parser.add_argument("--parallel", type=int, default=int(os.environ.get("NIGHTLY_PARALLEL", "4")))
    parser.add_argument("--list", action="store_true", help="print the cell list of a checked-out tree and exit")
    parser.add_argument("--tree", type=Path, help="tree for --list")
    parser.add_argument("--catch-up", action="store_true",
                        help="at logon: if the last scheduled night never started, write it as UNAVAILABLE")
    args = parser.parse_args()
    if args.list:
        skips = read_skips(HERE / "skip.conf")
        for cell in catalog_cells(args.tree or Path.cwd()):
            print(f"{cell}\tSKIP: {skips[cell]}" if cell in skips else cell)
        return 0
    branches = args.branch or os.environ.get("NIGHTLY_BRANCHES", "").split() or read_list(HERE / "branches.conf")
    only = [c.strip() for c in args.cells.split(",") if c.strip()] if args.cells else None
    only_lanes = [c.strip() for c in args.lanes.split(",") if c.strip()] if args.lanes else None
    (ROOT / "runs").mkdir(parents=True, exist_ok=True)
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
            run_branch(branch, args.parallel, only, only_lanes, unavailable=reason, date=night)
        return 0
    wtext(started_marker(night_of(moment)), moment.isoformat(timespec="seconds") + "\n")
    global _run_deadline
    if RUN_DEADLINE_S:
        _run_deadline = time.monotonic() + RUN_DEADLINE_S
    log(f"nightly start: branches {branches}, steps {STEPS}, parallel {args.parallel}")
    for branch in branches:
        if _stop.is_set():
            break
        run_branch(branch, args.parallel, only, only_lanes)
    if FRESH_TREES:
        prune_trees()
    prune_local()
    log("nightly done")
    return 0


if __name__ == "__main__":
    sys.exit(main())
