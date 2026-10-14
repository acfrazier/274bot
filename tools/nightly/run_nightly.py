#!/usr/bin/env python3
"""274bot nightly live run: build `tui-play` per branch and run every live cell.

Runs on the nightly VM from `274bot-nightly.service` (see README.md). For each
branch in `branches.conf` (or `--branch`), it fetches the branch tip anonymously,
builds `tui-play` (`--release --locked`), starts a private engine from a clean
copy, runs every cell in the checked-out scenario catalog with a few in parallel,
stops the engine and writes `summary.json`, `summary.md` and per-cell receipts
into `<root>/runs/<date>-<branch>-<commit>/`. `DONE` is written last; the archive
host only pulls runs that have it.

Standard library only (Python 3.10+).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import fcntl
import hashlib
import json
import os
import platform
import pty
import re
import shutil
import signal
import struct
import subprocess
import sys
import termios
import threading
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from zoneinfo import ZoneInfo

HERE = Path(__file__).resolve().parent
ROOT = Path(os.environ.get("NIGHTLY_ROOT", "/srv/nightly"))
REPO_URL = os.environ.get("NIGHTLY_REPO", "https://github.com/acfrazier/274bot")
PROFILE = os.environ.get("NIGHTLY_PROFILE", "local-289")
TIMEZONE = ZoneInfo(os.environ.get("NIGHTLY_TZ", "America/New_York"))
NAME_PREFIX = os.environ.get("NIGHTLY_NAME_PREFIX", "nt")
KEEP_DAYS = int(os.environ.get("NIGHTLY_KEEP_DAYS", "14"))
# Every cell enforces its own scenario deadline inside tui-play; this outer cap
# only stops a process that never reaches a terminal state.
CELL_CAP_S = int(os.environ.get("NIGHTLY_CELL_CAP_S", "3600"))
ENGINE_PRISTINE = ROOT / "engine-pristine"
ENGINE_RUN = ROOT / "engine-run"
CATALOG = ROOT / "rs2b0t"
NAV_SNAPSHOT = ROOT / "nav-snapshot"
CATALOG_RS = Path("crates/scenario/src/catalog.rs")
RECEIPT_START = re.compile(r'\{"scenario":"')
ANSI = re.compile(r"\x1b(\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[()][0-9A-Za-z]|[=>78DEHMc])")
FAILURE_HINT = re.compile(r"(FAIL|panicked|error|Error|refus|timed out|deadline)")
PAIR_GATE_REFUSAL = "only qualifies under the pair core gate"

_stop = threading.Event()
_children: set[subprocess.Popen] = set()
_children_lock = threading.Lock()


def log(message: str) -> None:
    print(f"{dt.datetime.now(TIMEZONE).isoformat(timespec='seconds')} {message}", flush=True)


def platform_id() -> dict:
    os_name = platform.system().lower()
    pretty = os_name
    os_release = Path("/etc/os-release")
    if os_release.is_file():
        for line in os_release.read_text().splitlines():
            if line.startswith("PRETTY_NAME="):
                pretty = line.split("=", 1)[1].strip('"')
    return {
        "platform": f"{os_name}-{platform.machine()}",
        "os": pretty,
        "host": platform.node(),
    }


def run(cmd: list[str], *, cwd: Path | None = None, env: dict | None = None,
        log_to: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    if log_to is None:
        return subprocess.run(cmd, cwd=cwd, env=env, check=check, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    with log_to.open("a") as out:
        out.write(f"$ {' '.join(cmd)}\n")
        out.flush()
        return subprocess.run(cmd, cwd=cwd, env=env, check=check, text=True,
                              stdout=out, stderr=subprocess.STDOUT)


def read_list(path: Path) -> list[str]:
    rows = []
    for line in path.read_text().splitlines():
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
    text = (tree / CATALOG_RS).read_text()
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
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
    if not (tree / ".git").exists():
        tree.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", "--no-checkout", REPO_URL, str(tree)], env=env, log_to=build_log)
    git = ["git", "-C", str(tree)]
    run(git + ["fetch", "--prune", "origin", f"+refs/heads/{branch}:refs/remotes/origin/{branch}"],
        env=env, log_to=build_log)
    run(git + ["checkout", "--force", "--detach", f"origin/{branch}"], env=env, log_to=build_log)
    run(git + ["submodule", "sync", "--recursive"], env=env, log_to=build_log)
    run(git + ["submodule", "update", "--init", "--recursive", "--force"], env=env, log_to=build_log)
    run(git + ["clean", "-ffdxq"], env=env, log_to=build_log)
    run(git + ["submodule", "foreach", "--recursive", "git clean -ffdxq"], env=env, log_to=build_log)
    full = run(git + ["rev-parse", "HEAD"]).stdout.strip()
    return full, full[:9]


def build_env(target: Path, **extra: str) -> dict:
    # host-play's build script bakes the bundled navigation from the engine's content
    # and a decoded client-cache snapshot (BOT_NAV_SNAPSHOT_ROOT).
    return dict(os.environ, CARGO_TARGET_DIR=str(target), ENGINE_DIR=str(ENGINE_PRISTINE / "engine"),
                BOT_NAV_SNAPSHOT_ROOT=str(NAV_SNAPSHOT), **extra)


def cargo_tui_play(tree: Path, env: dict, build_log: Path) -> None:
    run(["cargo", "build", "--locked", "--release", "-p", "tui", "--bin", "tui-play"],
        cwd=tree, env=env, log_to=build_log)


def snapshot_ready() -> bool:
    return any(NAV_SNAPSHOT.glob("*/versionlist"))


def bootstrap_nav_snapshot(tree: Path, run_dir: Path, build_log: Path) -> None:
    """Create the decoded client-cache snapshot the navigation bake reads.

    The snapshot is what the client itself decodes from the engine's cache on
    first login. Build a tui-play without bundled navigation, let one cell log
    in against a fresh engine, and keep the snapshot it decoded.
    """
    log("nav snapshot missing or stale: bootstrapping it from a navigation-free build")
    skip_target = ROOT / "target" / "navskip"
    cargo_tui_play(tree, build_env(skip_target, BOT_NAV_BUILD="skip"), build_log)
    work = ROOT / "work" / "nav-snapshot"
    shutil.rmtree(work, ignore_errors=True)
    home, tmp = work / "home", work / "tmp"
    home.mkdir(parents=True)
    tmp.mkdir(parents=True)
    engine_start(run_dir)
    try:
        run_tui(tui_cmd(skip_target / "release" / "tui-play", "walk"),
                tui_env(home, tmp, work / "evidence"), work, work / "stderr.log", 600)
    finally:
        engine_stop(None)
    decoded = [path.parent for path in
               (home / ".274bot" / "unpack-289").glob("revision-289/*/*/*/versionlist")]
    if not decoded:
        raise RuntimeError("nav snapshot bootstrap: the client decoded no snapshot; "
                           f"see {work / 'stderr.log'}")
    staging = NAV_SNAPSHOT.with_name(NAV_SNAPSHOT.name + ".new")
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    for path in decoded:
        shutil.copytree(path, staging / path.name)
    shutil.rmtree(NAV_SNAPSHOT, ignore_errors=True)
    staging.rename(NAV_SNAPSHOT)
    shutil.rmtree(work, ignore_errors=True)
    log(f"nav snapshot ready: {[path.name for path in decoded]}")


def build(tree: Path, target: Path, run_dir: Path, build_log: Path) -> Path:
    if not snapshot_ready():
        bootstrap_nav_snapshot(tree, run_dir, build_log)
    try:
        cargo_tui_play(tree, build_env(target), build_log)
    except subprocess.CalledProcessError:
        # A changed engine cache invalidates the snapshot; refresh it once and retry.
        if "decoded navigation cache" not in build_log.read_text(errors="replace")[-20000:]:
            raise
        bootstrap_nav_snapshot(tree, run_dir, build_log)
        cargo_tui_play(tree, build_env(target), build_log)
    return target / "release" / "tui-play"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


# ---------------------------------------------------------------- engine


def engine_start(run_dir: Path) -> None:
    """Fresh engine from the pristine copy: empty player saves, empty account DB."""
    engine_stop(None)
    run(["rsync", "-a", "--delete", f"{ENGINE_PRISTINE}/", f"{ENGINE_RUN}/"],
        log_to=run_dir / "engine-control.log")
    run([str(ENGINE_RUN / "start-289.sh")], cwd=ENGINE_RUN, log_to=run_dir / "engine-control.log")


def engine_stop(run_dir: Path | None) -> None:
    if not (ENGINE_RUN / "runtime" / "pids" / "engine.pid").exists():
        return
    control = run_dir / "engine-control.log" if run_dir else None
    run([str(ENGINE_RUN / "stop-289.sh")], cwd=ENGINE_RUN, log_to=control, check=False)
    if run_dir:
        engine_log = ENGINE_RUN / "runtime" / "logs" / "engine.log"
        if engine_log.is_file():
            shutil.copyfile(engine_log, run_dir / "engine.log")


# ---------------------------------------------------------------- cells


@dataclass
class CellResult:
    cell: str
    result: str  # PASS, FAIL, TIMEOUT, ERROR, SKIP, NOT_RUN
    branch: str
    commit: str
    platform: str
    ticks: int | None = None
    wall_s: float | None = None
    first_failure: str | None = None
    exit_code: int | None = None
    reason: str | None = None
    gate: str | None = None  # "--pair-core" when the cell ran under the pair witness
    started: str | None = None
    receipt: dict | None = field(default=None, repr=False)


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


def kill_group(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(15)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
    except ProcessLookupError:
        pass


def tui_env(home: Path, tmp: Path, evidence: Path) -> dict:
    return {
        "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"),
        "HOME": str(home),
        "TMPDIR": str(tmp),
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "BOT_CPU": "1",
        "BOT_LIVE_NAME_PREFIX": NAME_PREFIX,
        "LIVE_EVIDENCE_DIR": str(evidence),
        "RS2B0T": str(CATALOG),
        "ENGINE_DIR": str(ENGINE_RUN / "engine"),
    }


def tui_cmd(tui_play: Path, cell: str, gate: str | None = None) -> list[str]:
    cmd = [str(tui_play), "--profile", PROFILE, "--engine", str(ENGINE_RUN / "engine"),
           "--catalog", str(CATALOG), "--live", f"script_{cell}"]
    return cmd + [gate] if gate else cmd


def run_tui(cmd: list[str], env: dict, cwd: Path, stderr_path: Path, cap_s: int) -> tuple[int, bool, str]:
    """Run tui-play on a pseudo-terminal (it draws a terminal UI) with stderr kept apart.

    Returns (exit code, hit the cap, terminal transcript without escape codes).
    """
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
        proc = subprocess.Popen(cmd, env=env, cwd=cwd, stdin=slave, stdout=slave,
                                stderr=stderr, start_new_session=True)
        os.close(slave)
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        with _children_lock:
            _children.add(proc)
        try:
            while True:
                try:
                    proc.wait(timeout=5)
                    break
                except subprocess.TimeoutExpired:
                    if _stop.is_set():
                        kill_group(proc)
                        break
                    if time.monotonic() - started > cap_s:
                        timed_out = True
                        kill_group(proc)
                        break
        finally:
            with _children_lock:
                _children.discard(proc)
        reader.join(timeout=10)
        os.close(master)
    tty_text = ANSI.sub("", tty_raw.decode("utf-8", "replace")).replace("\r", "")
    return proc.returncode, timed_out, tty_text


def run_cell(cell: str, tui_play: Path, run_dir: Path, work: Path, base: CellResult) -> CellResult:
    result = CellResult(**{**asdict(base), "cell": cell})
    cell_dir = run_dir / "cells" / cell
    cell_dir.mkdir(parents=True, exist_ok=True)
    if _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted before this cell started"
        return write_cell(cell_dir, result)
    home = work / cell / "home"
    tmp = work / cell / "tmp"
    started = time.monotonic()
    result.started = dt.datetime.now(TIMEZONE).isoformat(timespec="seconds")
    gate = None
    while True:
        shutil.rmtree(work / cell, ignore_errors=True)
        home.mkdir(parents=True)
        tmp.mkdir(parents=True)
        cmd = tui_cmd(tui_play, cell, gate)
        (cell_dir / "command.txt").write_text(" ".join(cmd) + "\n")
        exit_code, timed_out, tty_text = run_tui(
            cmd, tui_env(home, tmp, cell_dir / "evidence"), work / cell, cell_dir / "stderr.log",
            CELL_CAP_S)
        stderr_text = (cell_dir / "stderr.log").read_text(errors="replace")
        # A paired proof refuses to start without the pair witness and says so;
        # run it again under that gate (the same flag a person would pass).
        if gate is None and exit_code != 0 and PAIR_GATE_REFUSAL in stderr_text:
            gate = "--pair-core"
            continue
        break
    result.gate = gate
    result.wall_s = round(time.monotonic() - started, 1)
    result.exit_code = exit_code
    (cell_dir / "tty.log").write_text(tty_text)
    # Logs the frontend writes under its (throwaway) HOME.
    session_text = ""
    bot_dir = home / ".274bot"
    for path in sorted(bot_dir.glob("*.log")) + sorted((bot_dir / "logs").rglob("*")):
        if path.is_file():
            target = cell_dir / "session-logs" / path.relative_to(bot_dir)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            session_text += target.read_text(errors="replace") + "\n"
    combined = stderr_text + "\n" + session_text + "\n" + tty_text
    receipt = find_receipt(combined, cell)
    verdict = re.search(rf"\b(PASS|FAIL): live {re.escape(cell)}\b", combined)
    if receipt is not None:
        result.receipt = receipt
        result.result = receipt["outcome"]
        result.ticks = receipt.get("ticks")
        (cell_dir / "receipt.json").write_text(json.dumps(receipt, indent=1) + "\n")
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
        result.first_failure = (f"runner cap {CELL_CAP_S}s reached without a PASS/FAIL receipt; "
                                f"last: {first_failure_line(stderr_text) or '-'}")[:400]
    elif _stop.is_set():
        result.result, result.reason = "NOT_RUN", "run interrupted while this cell ran"
    else:
        result.result = "ERROR"
        result.first_failure = first_failure_line(stderr_text) or first_failure_line(tty_text)
    shutil.rmtree(work / cell, ignore_errors=True)
    return write_cell(cell_dir, result)


def write_cell(cell_dir: Path, result: CellResult) -> CellResult:
    row = asdict(result)
    row.pop("receipt")
    (cell_dir / "result.json").write_text(json.dumps(row, indent=1) + "\n")
    return result


# ---------------------------------------------------------------- summary


ORDER = ["FAIL", "TIMEOUT", "ERROR", "NOT_RUN", "SKIP", "PASS"]


def write_summary(run_dir: Path, meta: dict, results: list[CellResult]) -> None:
    counts = {key: 0 for key in ORDER}
    for result in results:
        counts[result.result] = counts.get(result.result, 0) + 1
    rows = []
    for result in results:
        row = asdict(result)
        row.pop("receipt")
        rows.append(row)
    summary = dict(meta, counts=counts, cells=rows)
    (run_dir / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")

    md = [
        f"# Nightly {meta['run']}",
        "",
        f"- Branch `{meta['branch']}` at `{meta['commit_full']}`",
        f"- Platform {meta['platform']} ({meta['os']}, host {meta['host']})",
        f"- Started {meta['started']}, finished {meta['finished']}, wall {fmt_s(meta['wall_s'])}",
        f"- Build: {'ok' if meta['build']['ok'] else 'FAILED'} ({fmt_s(meta['build']['wall_s'])})"
        + (f": {meta['build']['error']}" if meta['build'].get('error') else ""),
        f"- Engine: {meta['engine']['status']}; catalog `{meta['catalog_commit']}`; "
        f"{meta['parallel']} cells in parallel; outer cap {meta['cell_cap_s']} s",
        "- Counts: " + ", ".join(f"{key} {counts[key]}" for key in ORDER if counts[key]),
        "",
    ]
    if meta.get("interrupted"):
        md += ["**Run interrupted** before every cell finished.", ""]
    md += ["| Cell | Result | Ticks | Wall | First failure line / reason | Commit |",
           "|---|---|---:|---:|---|---|"]
    for result in sorted(results, key=lambda r: (ORDER.index(r.result), r.cell)):
        note = result.first_failure or result.reason or ""
        if result.gate:
            note = f"[{result.gate}] {note}".strip()
        note = note.replace("|", "\\|").replace("\n", " ")
        md.append(f"| `{result.cell}` | {result.result} | {result.ticks if result.ticks is not None else ''}"
                  f" | {fmt_s(result.wall_s) if result.wall_s is not None else ''} | {note} | `{result.commit}` |")
    (run_dir / "summary.md").write_text("\n".join(md) + "\n")


def fmt_s(seconds: float | None) -> str:
    if seconds is None:
        return "-"
    seconds = int(round(seconds))
    hours, rest = divmod(seconds, 3600)
    minutes, secs = divmod(rest, 60)
    return f"{hours}h{minutes:02d}m{secs:02d}s" if hours else f"{minutes}m{secs:02d}s"


# ---------------------------------------------------------------- one branch


def run_dir_for(branch: str, commit: str) -> Path:
    base = f"{dt.datetime.now(TIMEZONE):%Y-%m-%d}-{slug(branch)}-{commit}"
    candidate = ROOT / "runs" / base
    attempt = 2
    while candidate.exists():
        candidate = ROOT / "runs" / f"{base}-r{attempt}"
        attempt += 1
    return candidate


def run_branch(branch: str, parallel: int, only: list[str] | None) -> Path:
    ident = platform_id()
    tree = ROOT / "trees" / slug(branch)
    target = ROOT / "target" / slug(branch)
    staging = ROOT / "runs" / f".building-{slug(branch)}"
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    started_wall = time.monotonic()
    started = dt.datetime.now(TIMEZONE).isoformat(timespec="seconds")
    build_log = staging / "build.log"
    commit_full = commit = "unknown"
    build_ok, build_error, tui_play = True, None, None
    build_started = time.monotonic()
    try:
        commit_full, commit = update_tree(branch, tree, build_log)
        tui_play = build(tree, target, staging, build_log)
    except (subprocess.CalledProcessError, OSError, RuntimeError) as error:
        build_ok, build_error = False, f"{error}"
    build_wall = round(time.monotonic() - build_started, 1)
    run_dir = run_dir_for(branch, commit)
    staging.rename(run_dir)
    log(f"{branch}: {commit} -> {run_dir.name} (build {'ok' if build_ok else 'FAILED'})")

    skips = read_skips(HERE / "skip.conf")
    try:
        cells = catalog_cells(tree)
    except (OSError, ValueError, RuntimeError) as error:
        cells, build_ok = [], False
        build_error = (build_error or "") + f"; cell list: {error}"
    if only:
        cells = [cell for cell in cells if cell in only]
    base = CellResult(cell="", result="NOT_RUN", branch=branch, commit=commit,
                      platform=ident["platform"])
    engine_status = "not started"
    results: list[CellResult] = []
    work = ROOT / "work" / run_dir.name
    if build_ok:
        try:
            engine_start(run_dir)
            engine_status = "fresh copy started, stopped after the run"
        except subprocess.CalledProcessError as error:
            engine_status = f"failed to start: {error}"
    runnable = []
    for cell in cells:
        if cell in skips:
            cell_dir = run_dir / "cells" / cell
            cell_dir.mkdir(parents=True, exist_ok=True)
            results.append(write_cell(cell_dir, CellResult(
                **{**asdict(base), "cell": cell, "result": "SKIP", "reason": skips[cell]})))
        elif not build_ok or tui_play is None:
            results.append(CellResult(**{**asdict(base), "cell": cell,
                                         "reason": "build failed; see build.log"}))
        elif not engine_status.startswith("fresh"):
            results.append(CellResult(**{**asdict(base), "cell": cell,
                                         "reason": f"engine {engine_status}"}))
        else:
            runnable.append(cell)
    if runnable:
        log(f"{branch}: running {len(runnable)} cells, {parallel} in parallel")
        with concurrent.futures.ThreadPoolExecutor(max_workers=parallel) as pool:
            futures = {pool.submit(run_cell, cell, tui_play, run_dir, work, base): cell
                       for cell in runnable}
            for future in concurrent.futures.as_completed(futures):
                outcome = future.result()
                results.append(outcome)
                log(f"{branch}: {outcome.cell} {outcome.result} ({fmt_s(outcome.wall_s)})")
    engine_stop(run_dir)
    shutil.rmtree(work, ignore_errors=True)
    order = {cell: index for index, cell in enumerate(cells)}
    results.sort(key=lambda r: order.get(r.cell, len(order)))
    meta = dict(
        run=run_dir.name, branch=branch, commit=commit, commit_full=commit_full,
        **ident, started=started,
        finished=dt.datetime.now(TIMEZONE).isoformat(timespec="seconds"),
        wall_s=round(time.monotonic() - started_wall, 1), parallel=parallel,
        cell_cap_s=CELL_CAP_S, profile=PROFILE, interrupted=_stop.is_set(),
        build=dict(ok=build_ok, wall_s=build_wall, error=build_error,
                   tui_play_sha256=sha256(tui_play) if tui_play and tui_play.is_file() else None),
        engine=dict(status=engine_status, source="engine-pristine (289 engine copy, fresh DB)"),
        catalog_commit=git_head(CATALOG), runner_commit=git_head(HERE.parent.parent),
    )
    write_summary(run_dir, meta, results)
    (run_dir / "DONE").write_text(meta["finished"] + "\n")
    log(f"{branch}: summary {meta['run']}: "
        + ", ".join(f"{k} {v}" for k, v in json.loads((run_dir / 'summary.json').read_text())['counts'].items() if v))
    return run_dir


def git_head(path: Path) -> str:
    try:
        return run(["git", "-C", str(path), "rev-parse", "--short=9", "HEAD"]).stdout.strip()
    except (subprocess.CalledProcessError, OSError):
        return "unknown"


def prune_local() -> None:
    cutoff = time.time() - KEEP_DAYS * 86400
    runs = ROOT / "runs"
    for entry in runs.iterdir() if runs.is_dir() else []:
        if entry.is_dir() and not entry.name.startswith(".") and entry.stat().st_mtime < cutoff:
            shutil.rmtree(entry, ignore_errors=True)
            log(f"pruned local run {entry.name} (older than {KEEP_DAYS} days; the archive keeps it)")


def on_signal(signum, _frame) -> None:
    log(f"signal {signum}: stopping cells")
    _stop.set()
    with _children_lock:
        children = list(_children)
    for proc in children:
        kill_group(proc)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--branch", action="append", help="branch to test (repeatable; default branches.conf)")
    parser.add_argument("--cells", help="comma-separated cell names (default: every catalog cell)")
    parser.add_argument("--parallel", type=int, default=int(os.environ.get("NIGHTLY_PARALLEL", "4")))
    parser.add_argument("--list", action="store_true", help="print the cell list of a checked-out tree and exit")
    parser.add_argument("--tree", type=Path, help="tree for --list")
    args = parser.parse_args()
    if args.list:
        skips = read_skips(HERE / "skip.conf")
        for cell in catalog_cells(args.tree or Path.cwd()):
            print(f"{cell}\tSKIP: {skips[cell]}" if cell in skips else cell)
        return 0
    branches = args.branch or os.environ.get("NIGHTLY_BRANCHES", "").split() or read_list(HERE / "branches.conf")
    only = [c.strip() for c in args.cells.split(",") if c.strip()] if args.cells else None
    (ROOT / "runs").mkdir(parents=True, exist_ok=True)
    lock = open(ROOT / ".nightly.lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        log("another nightly run holds the lock; exiting")
        return 1
    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    log(f"nightly start: branches {branches}, parallel {args.parallel}")
    for branch in branches:
        if _stop.is_set():
            break
        run_branch(branch, args.parallel, only)
    prune_local()
    log("nightly done")
    return 0


if __name__ == "__main__":
    sys.exit(main())
