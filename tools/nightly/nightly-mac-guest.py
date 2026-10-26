#!/usr/bin/env python3
"""Run the Mac nightly in a disposable Tart clone and publish its run folders."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import uuid
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence, Tuple

DEFAULT_VM_TOOL = "/Users/acfrazier/experiments/274bot/.superpowers/release-0.1.9/tools/headed/mac_vm.py"
DEFAULT_SOURCE_REPO = "https://github.com/acfrazier/274bot"
DEFAULT_BASE = "274bot-mac-base"
GUEST_SHARE = "/Volumes/My Shared Files/sess"
VM_CPUS = 8
VM_MEMORY_MB = 24576
VM_DISK_GB = 160
MIN_FREE_GIB = 120
GIB = 1024 ** 3
NODE_VERSION = "26.12.1"
RUST_VERSION = "1.98.1"
PYTHON_VERSION = "3.13"


class NightlyError(RuntimeError):
    """A required nightly operation failed."""


class DiskFloorError(NightlyError):
    """The host scratch volume is at or below its protected free-space floor."""


def log(message: str) -> None:
    print(f"nightly-mac: {message}", flush=True)


def shell_quote(value: str) -> str:
    import shlex
    return shlex.quote(value)


def run_command(args: Sequence[str], *, cwd: Optional[Path] = None,
                env: Optional[Dict[str, str]] = None, check: bool = True,
                timeout: Optional[float] = None) -> subprocess.CompletedProcess:
    result = subprocess.run(
        list(args), cwd=str(cwd) if cwd else None, env=env,
        text=True, capture_output=True, check=False, timeout=timeout,
    )
    if check and result.returncode:
        detail = (result.stderr or result.stdout or "").strip()
        raise NightlyError(f"command failed ({result.returncode}): {args[0]}: {detail}")
    return result


def git_env() -> Dict[str, str]:
    env = os.environ.copy()
    env["GIT_TERMINAL_PROMPT"] = "0"
    env["GCM_INTERACTIVE"] = "never"
    return env


def git(args: Sequence[str], *, check: bool = True,
        timeout: Optional[float] = None) -> subprocess.CompletedProcess:
    return run_command(["git", "-c", "credential.helper=", *args], env=git_env(), check=check,
                       timeout=timeout)


# ------------------------------------------------- branch list and self-update
#
# At the start of a night the controller takes its branch list from GitHub: it
# fetches the runner checkout's origin and reads tools/nightly/branches.conf
# from origin/dev. The precedence stays: --branch, then NIGHTLY_BRANCHES, then
# the GitHub list. A failed fetch falls back to the local file and records a
# note in the status file, so a night is never lost to a fetch failure.
#
# The controller also updates its own checkout (tools/nightly/ under the
# runner directory) to origin/dev before a night and re-executes itself, after
# the candidate passes a self-check (py_compile of the runner files plus its
# own unit tests, about a minute at most). The update is opt-in: only
# NIGHTLY_SELF_UPDATE=1 (set on the runner machines) updates. Anything else,
# including unset, keeps the old code. Only the runner checkout is ever
# touched: every git command below runs with -C at `repo`, the candidate is
# extracted under a temporary directory, and the re-exec runs the same script
# path again.
BRANCHES_REF = "origin/dev"
BRANCHES_REL = "tools/nightly/branches.conf"
SELF_UPDATE_ENV = "NIGHTLY_SELF_UPDATE"
SELF_UPDATE_DONE_ENV = "NIGHTLY_SELF_UPDATE_DONE"
SELF_UPDATE_FROM_ENV = "NIGHTLY_SELF_UPDATE_FROM"
# Set in the self-check subprocess: the updater's own tests skip when it is
# present, so a check never re-tests the updater recursively.
SELF_CHECK_ACTIVE_ENV = "NIGHTLY_SELF_CHECK_ACTIVE"
# The whole guard (py_compile plus the controller's own unit tests) fits in
# about a minute; a slower or hung check keeps the old code instead of the night.
SELF_UPDATE_TIMEOUT_S = 55
FETCH_TIMEOUT_S = 120
RUNNER_PY_FILES = ("run_nightly.py", "nightly-mac-guest.py", "latency_proxy.py")
SELF_TEST_FILE = "test_nightly_mac_guest.py"


def self_update_enabled() -> bool:
    """NIGHTLY_SELF_UPDATE=1 opts in. Off by default: the update force-checks
    out origin/dev in the runner checkout, so a hand run from a developer
    checkout must never update unless the runner machines opted in."""
    return os.environ.get(SELF_UPDATE_ENV, "0") in ("1", "true", "yes")


def runner_head(repo: Path) -> str:
    """The runner checkout's HEAD, or "unknown" when it cannot be read."""
    try:
        result = git(["-C", str(repo), "rev-parse", "HEAD"], check=False)
    except OSError:
        return "unknown"
    return result.stdout.strip() or "unknown"


def fetch_branch_tip(repo: Path, timeout: Optional[float] = FETCH_TIMEOUT_S) -> str:
    """Fetch origin's dev into the runner checkout; return the tip SHA."""
    git(["-C", str(repo), "fetch", "--prune", "origin",
         f"+refs/heads/dev:refs/remotes/{BRANCHES_REF}"], timeout=timeout)
    return git(["-C", str(repo), "rev-parse", BRANCHES_REF]).stdout.strip()


def read_remote_branches(repo: Path) -> Tuple[List[str], str]:
    """The branch list from origin/dev: fetch, then show the file. Return the
    list and the tip SHA. Raise on a fetch failure or an empty list."""
    tip = fetch_branch_tip(repo)
    text = git(["-C", str(repo), "show", f"{BRANCHES_REF}:{BRANCHES_REL}"]).stdout
    branches = _validate_branches([line.split("#", 1)[0].strip() for line in text.splitlines()
                                   if line.split("#", 1)[0].strip()])
    if not branches:
        raise NightlyError(f"{BRANCHES_REL} on {BRANCHES_REF} names no branches")
    return branches, tip


def resolve_branches(requested: Optional[Sequence[str]], repo: Path) -> Tuple[List[str], Dict[str, str]]:
    """The night's branches and where they came from.

    Precedence: --branch, then NIGHTLY_BRANCHES, then the GitHub list on
    origin/dev. When the fetch fails, the local file is used and the note
    says so, so a night is never lost to a fetch failure.
    """
    if requested:
        return _validate_branches(list(requested)), {"source": "cli", "commit": "",
                                                     "note": "from --branch"}
    env_branches = os.environ.get("NIGHTLY_BRANCHES", "").split()
    if env_branches:
        return _validate_branches(env_branches), {"source": "env", "commit": "",
                                                  "note": "from NIGHTLY_BRANCHES"}
    try:
        branches, tip = read_remote_branches(repo)
    except (NightlyError, subprocess.TimeoutExpired, OSError) as error:
        branches = read_branches(repo / BRANCHES_REL, None)
        head = runner_head(repo)
        return branches, {"source": "local", "commit": head,
                          "note": (f"fetch of {BRANCHES_REF} failed ({error}); "
                                   f"using local {BRANCHES_REL}")}
    return branches, {"source": "github", "commit": tip,
                      "note": f"from {BRANCHES_REF} at {tip}"}


def extract_runner_candidate(repo: Path, dest: Path, ref: str = BRANCHES_REF) -> None:
    """Materialize origin/dev's tools/nightly/ under `dest` for the self-check.
    The runner checkout itself is never touched here."""
    names = git(["-C", str(repo), "ls-tree", "-r", "--name-only", ref, "tools/nightly"]).stdout.split()
    for name in names:
        parts = Path(name).parts
        if not name.startswith("tools/nightly/") or ".." in parts or Path(name).is_absolute():
            raise NightlyError(f"refusing to extract unsafe path from {ref}: {name!r}")
        target = dest / name
        target.parent.mkdir(parents=True, exist_ok=True)
        data = subprocess.run(["git", "-c", "credential.helper=", "-C", str(repo),
                               "show", f"{ref}:{name}"],
                              env=git_env(), check=True, stdout=subprocess.PIPE).stdout
        target.write_bytes(data)


def self_check_candidate(candidate: Path, test_file: str = SELF_TEST_FILE,
                         timeout: float = SELF_UPDATE_TIMEOUT_S) -> Tuple[bool, str]:
    """py_compile the candidate runner files, then run its own unit tests.
    Return (True, detail) or (False, reason)."""
    nightly_dir = candidate / "tools" / "nightly"
    sources = sorted(str(path) for name in RUNNER_PY_FILES if (path := nightly_dir / name).is_file())
    if not sources:
        return False, "candidate has no runner files"
    compiled = subprocess.run([sys.executable, "-m", "py_compile", *sources],
                              capture_output=True, text=True)
    if compiled.returncode:
        reason = (compiled.stderr or compiled.stdout).strip().splitlines()
        detail = reason[-1][:300] if reason else "unknown error"
        return False, f"py_compile failed: {detail}"
    test_path = nightly_dir / test_file
    if not test_path.is_file():
        return False, f"candidate has no {test_file}"
    env = dict(os.environ, NIGHTLY_SELF_CHECK_ACTIVE="1")
    try:
        tested = subprocess.run([sys.executable, "-B", str(test_path)], cwd=str(candidate),
                                env=env, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False, f"{test_file} did not finish within {timeout} s"
    if tested.returncode:
        tail = (tested.stderr or tested.stdout).strip().splitlines()
        detail = tail[-1][:300] if tail else "unknown error"
        return False, f"{test_file} failed: {detail}"
    return True, f"py_compile ok; {test_file} passed"


def maybe_self_update(repo: Path, script: Path, argv: Optional[Sequence[str]] = None,
                      _exec: Any = os.execvpe) -> Optional[Dict[str, str]]:
    """Update the runner checkout to origin/dev and re-execute, after the
    candidate passes its self-check. Return the status info, or None when this
    process re-executes (the new process reports status "updated"). Only the
    runner checkout (`git -C repo`) is ever touched."""
    head = runner_head(repo)
    if not self_update_enabled():
        return {"status": "disabled", "from": head, "to": head,
                "detail": f"{SELF_UPDATE_ENV} is not 1; running {head}"}
    if os.environ.get(SELF_UPDATE_DONE_ENV):
        previous = os.environ.get(SELF_UPDATE_FROM_ENV, "")
        detail = f"re-executed after self-update to {head}"
        if previous:
            detail += f" (was {previous})"
        return {"status": "updated", "from": previous or head, "to": head, "detail": detail}
    try:
        tip = fetch_branch_tip(repo)
    except (NightlyError, subprocess.TimeoutExpired, OSError) as error:
        return {"status": "fetch-failed", "from": head, "to": head,
                "detail": f"fetch of {BRANCHES_REF} failed ({error}); running {head}"}
    if tip == head:
        return {"status": "current", "from": head, "to": tip,
                "detail": f"already at {BRANCHES_REF} {tip}"}
    with tempfile.TemporaryDirectory(prefix="nightly-self-update-") as tmp:
        candidate = Path(tmp) / "candidate"
        try:
            extract_runner_candidate(repo, candidate)
        except (NightlyError, subprocess.SubprocessError, OSError) as error:
            return {"status": "extract-failed", "from": head, "to": tip,
                    "detail": (f"could not extract {BRANCHES_REF} {tip} "
                               f"({error}); running {head}")}
        ok, check_detail = self_check_candidate(candidate)
    if not ok:
        return {"status": "check-failed", "from": head, "to": tip,
                "detail": (f"candidate {BRANCHES_REF} {tip} failed the self-check "
                           f"({check_detail}); running {head}")}
    git(["-C", str(repo), "checkout", "--force", "--detach", BRANCHES_REF])
    env = dict(os.environ, **{SELF_UPDATE_DONE_ENV: "1", SELF_UPDATE_FROM_ENV: head})
    _exec(sys.executable, [sys.executable, str(script),
                           *(list(argv) if argv is not None else sys.argv[1:])], env)
    return None


def branch_slug(branch: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]", "-", branch)


def _validate_branches(branches: Sequence[str]) -> List[str]:
    ordered: List[str] = []
    for branch in branches:
        if not branch or branch.startswith("-"):
            raise NightlyError(f"invalid branch name: {branch!r}")
        if git(["check-ref-format", "--branch", branch], check=False).returncode:
            raise NightlyError(f"invalid branch name: {branch!r}")
        if branch not in ordered:
            ordered.append(branch)
    if not ordered:
        raise NightlyError("the nightly branch list is empty")
    return ordered


def read_branches(path: Path, requested: Optional[Sequence[str]]) -> List[str]:
    if requested:
        branches = list(requested)
    else:
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except OSError as error:
            raise NightlyError(f"cannot read branch list {path}: {error}") from error
        branches = [line.strip() for line in lines
                    if line.strip() and not line.lstrip().startswith("#")]
    return _validate_branches(branches)


def is_within(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def should_skip(kind: str, relative: Path, name: str) -> bool:
    parts = relative.parts + (name,)
    common = {".git", ".DS_Store", "__pycache__", ".pytest_cache", ".mypy_cache",
              ".ruff_cache", ".cache", "node_modules", "target", ".worktrees"}
    if kind == "runner":
        return (name in common or name == ".venv" or name.endswith(".pyc"))
    if kind == "catalog":
        return (name in common or name == ".claude" or
                (len(parts) >= 2 and parts[-2:] == (".claude", "worktrees")))
    if kind == "engine":
        if name in {".git", ".DS_Store", "runtime", "node_modules"}:
            return True
        # The engine reads packed server maps from this runtime artifact directory.
        if name == ".cache" and parts != ("engine", "data", "pack", ".cache"):
            return True
        if name.startswith("db.sqlite"):
            return True
        if len(parts) >= 3 and parts[:3] == ("engine", "data", "players"):
            return True
        return False
    if kind == "snapshot":
        return name in {".DS_Store", ".git", "__pycache__"}
    if kind == "fixtures":
        return name in {".DS_Store", ".git", "__pycache__"}
    return False


def copy_tree(source: Path, destination: Path, *, kind: str) -> None:
    """Copy one input tree without following links outside its source root."""
    source = source.resolve(strict=True)
    if not source.is_dir():
        raise NightlyError(f"input is not a directory: {source}")
    if destination.exists() or destination.is_symlink():
        raise NightlyError(f"staging destination already exists: {destination}")
    destination.mkdir(parents=True)

    def copy_entry(src: Path, dst: Path, relative: Path) -> None:
        name = src.name
        if should_skip(kind, relative.parent, name):
            return
        info = os.lstat(str(src))
        if stat.S_ISLNK(info.st_mode):
            raw_target = os.readlink(str(src))
            target = (src.parent / raw_target).resolve(strict=False)
            if not is_within(target, source):
                raise NightlyError(f"input symlink escapes {source}: {src} -> {raw_target}")
            staged_target = destination / target.relative_to(source)
            link_target = os.path.relpath(str(staged_target), str(dst.parent))
            os.symlink(link_target, str(dst))
            return
        if stat.S_ISDIR(info.st_mode):
            dst.mkdir()
            for child in sorted(src.iterdir(), key=lambda item: item.name):
                copy_entry(child, dst / child.name, relative / child.name)
            shutil.copystat(str(src), str(dst), follow_symlinks=False)
            return
        if stat.S_ISREG(info.st_mode):
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(str(src), str(dst), follow_symlinks=False)
            return
        raise NightlyError(f"unsupported file type in input: {src}")

    for entry in sorted(source.iterdir(), key=lambda item: item.name):
        copy_entry(entry, destination / entry.name, Path(entry.name))


def make_readonly(root: Path) -> None:
    for directory, names, files in os.walk(str(root), topdown=True, followlinks=False):
        base = Path(directory)
        for name in list(names):
            path = base / name
            if path.is_symlink():
                names.remove(name)
                continue
            mode = stat.S_IMODE(os.lstat(str(path)).st_mode)
            os.chmod(str(path), (mode & 0o555) | 0o555)
        for name in files:
            path = base / name
            if path.is_symlink():
                continue
            mode = stat.S_IMODE(os.lstat(str(path)).st_mode)
            os.chmod(str(path), (mode & 0o555) | 0o444)
    if not root.is_symlink():
        mode = stat.S_IMODE(os.lstat(str(root)).st_mode)
        os.chmod(str(root), (mode & 0o555) | 0o555)


def make_writable_and_remove(root: Path) -> None:
    if not root.exists() and not root.is_symlink():
        return
    if root.is_symlink() or not root.is_dir():
        root.unlink(missing_ok=True)
        return
    for directory, names, files in os.walk(str(root), topdown=True, followlinks=False):
        base = Path(directory)
        for name in names + files:
            path = base / name
            if path.is_symlink():
                continue
            try:
                os.chmod(str(path), stat.S_IMODE(os.lstat(str(path)).st_mode) | 0o700)
            except OSError:
                pass
    mode = stat.S_IMODE(os.lstat(str(root)).st_mode)
    os.chmod(str(root), mode | 0o700)
    shutil.rmtree(str(root), ignore_errors=False)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_sha256sums(root: Path) -> None:
    rows: List[str] = []
    files = sorted(
        (path for path in root.rglob("*") if path.is_file() and not path.is_symlink()
         and path not in (root / "SHA256SUMS", root / "DONE")),
        key=lambda path: path.relative_to(root).as_posix(),
    )
    for path in files:
        relative = path.relative_to(root).as_posix()
        if "\n" in relative or "\r" in relative:
            raise NightlyError(f"run file name cannot be represented in SHA256SUMS: {relative!r}")
        rows.append(f"{sha256_file(path)}  {relative}")
    (root / "SHA256SUMS").write_text("\n".join(rows) + "\n", encoding="utf-8")


def verify_sha256sums(root: Path) -> None:
    manifest = root / "SHA256SUMS"
    if not manifest.is_file() or manifest.is_symlink():
        raise NightlyError(f"run folder has no regular SHA256SUMS: {root.name}")
    expected: Dict[str, str] = {}
    for line_number, line in enumerate(manifest.read_text(encoding="utf-8").splitlines(), 1):
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if not match:
            raise NightlyError(f"invalid SHA256SUMS line {line_number} in {root.name}")
        relative = PurePosixPath(match.group(2))
        if relative.is_absolute() or not relative.parts or ".." in relative.parts:
            raise NightlyError(f"unsafe SHA256SUMS path in {root.name}: {relative}")
        key = relative.as_posix()
        if key == "SHA256SUMS" or key in expected:
            raise NightlyError(f"duplicate or recursive SHA256SUMS entry in {root.name}: {key}")
        expected[key] = match.group(1)
    actual: Dict[str, Path] = {}
    for path in root.rglob("*"):
        if path.is_symlink():
            continue
        if path.is_file():
            if path not in (manifest, root / "DONE"):
                actual[path.relative_to(root).as_posix()] = path
        elif not path.is_dir():
            raise NightlyError(f"unsupported run output type: {path}")
    if set(expected) != set(actual):
        missing = sorted(set(actual) - set(expected))
        extra = sorted(set(expected) - set(actual))
        raise NightlyError(f"SHA256SUMS coverage mismatch in {root.name}: unlisted={missing[:3]} absent={extra[:3]}")
    for relative, expected_digest in expected.items():
        actual_digest = sha256_file(actual[relative])
        if actual_digest != expected_digest:
            raise NightlyError(f"SHA256SUMS mismatch in {root.name}: {relative}")


def safe_extract_archive(archive: Path, destination: Path, *, allow_links: bool) -> List[tarfile.TarInfo]:
    destination.mkdir(parents=True, exist_ok=True)
    root_real = destination.resolve()
    with tarfile.open(str(archive), mode="r:") as bundle:
        members = bundle.getmembers()
        for member in members:
            relative = PurePosixPath(member.name)
            if relative.is_absolute() or not relative.parts or ".." in relative.parts:
                raise NightlyError(f"unsafe archive member: {member.name!r}")
            target = destination.joinpath(*relative.parts)
            if not is_within(target.resolve(strict=False), root_real):
                raise NightlyError(f"archive member escapes destination: {member.name!r}")
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            if member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                source = bundle.extractfile(member)
                if source is None:
                    raise NightlyError(f"archive member has no data: {member.name}")
                with source, target.open("wb") as output:
                    shutil.copyfileobj(source, output, length=1024 * 1024)
                os.chmod(str(target), member.mode & 0o777)
                continue
            if allow_links and (member.issym() or member.islnk()):
                link = PurePosixPath(member.linkname)
                if link.is_absolute() or ".." in link.parts:
                    raise NightlyError(f"unsafe archive link: {member.name!r} -> {member.linkname!r}")
                target.parent.mkdir(parents=True, exist_ok=True)
                link_target = target.parent.joinpath(*link.parts)
                if not is_within(link_target.resolve(strict=False), root_real):
                    raise NightlyError(f"archive link escapes destination: {member.name!r}")
                if member.issym():
                    os.symlink(member.linkname, str(target))
                else:
                    os.link(str(destination.joinpath(*link.parts)), str(target))
                continue
            raise NightlyError(f"unsupported archive member type: {member.name!r}")
    return members


def validate_archive_members(members: Sequence[tarfile.TarInfo], archive_name: str) -> int:
    roots = set()
    total = 0
    for member in members:
        relative = PurePosixPath(member.name)
        if relative.is_absolute() or not relative.parts or ".." in relative.parts:
            raise NightlyError(f"unsafe archive member in {archive_name}: {member.name!r}")
        roots.add(relative.parts[0])
        if member.isfile():
            if member.size < 0:
                raise NightlyError(f"negative archive size in {archive_name}: {member.name}")
            total += member.size
        elif not member.isdir():
            raise NightlyError(f"unsupported result archive entry in {archive_name}: {member.name}")
    if len(roots) != 1:
        raise NightlyError(f"result archive must contain one run folder: {archive_name}")
    root = next(iter(roots))
    if root != Path(archive_name).stem:
        raise NightlyError(f"archive root does not match its name: {archive_name}")
    return total


def unique_run_destination(runs_root: Path, source_name: str) -> Path:
    first = runs_root / source_name
    if not first.exists() and not first.is_symlink():
        return first
    match = re.fullmatch(r"(.+)-r([2-9][0-9]*)", source_name)
    if match:
        base, start = match.group(1), int(match.group(2)) + 1
    else:
        base, start = source_name, 2
    attempt = start
    while True:
        candidate = runs_root / f"{base}-r{attempt}"
        if not candidate.exists() and not candidate.is_symlink():
            return candidate
        attempt += 1


def publish_archives(outbox: Path, runs_root: Path, disk_check: Callable[[], int],
                     min_free_bytes: int) -> List[Path]:
    published: List[Path] = []
    if not outbox.is_dir():
        return published
    runs_root.mkdir(parents=True, exist_ok=True)
    if runs_root.is_symlink():
        raise NightlyError(f"NIGHTLY result root must not be a symlink: {runs_root}")
    archives = sorted(
        (path for path in outbox.iterdir() if path.suffix == ".tar" and path.is_file()
         and not path.is_symlink()),
        key=lambda path: path.name,
    )
    for archive in archives:
        members: List[tarfile.TarInfo]
        with tarfile.open(str(archive), mode="r:") as bundle:
            members = bundle.getmembers()
        total_bytes = validate_archive_members(members, archive.name)
        free = disk_check()
        if free - total_bytes <= min_free_bytes:
            raise DiskFloorError(
                f"copying {archive.name} needs {total_bytes / GIB:.1f} GiB, "
                f"but scratch must retain {min_free_bytes / GIB:.0f} GiB free"
            )
        temporary = Path(tempfile.mkdtemp(prefix=".incoming-nightly-", dir=str(runs_root)))
        try:
            safe_extract_archive(archive, temporary, allow_links=False)
            source_name = Path(archive.name).stem
            staged = temporary / source_name
            if not staged.is_dir() or staged.is_symlink():
                raise NightlyError(f"result archive has no run directory: {archive.name}")
            if not (staged / "SHA256SUMS").is_file():
                write_sha256sums(staged)
            verify_sha256sums(staged)
            destination = unique_run_destination(runs_root, source_name)
            os.replace(str(staged), str(destination))
            published.append(destination)
            log(f"published {destination.name} ({total_bytes / GIB:.2f} GiB)")
        finally:
            shutil.rmtree(str(temporary), ignore_errors=True)
        if disk_check() <= min_free_bytes:
            raise DiskFloorError(
                f"scratch reached the {min_free_bytes / GIB:.0f} GiB floor while publishing results"
            )
    return published


def git_has_commit(repository: Path, commit: str) -> bool:
    return git(["-C", str(repository), "cat-file", "-e", f"{commit}^{{commit}}"], check=False).returncode == 0


def config_value(config_file: Path, key: str) -> Optional[str]:
    result = git(["config", "--file", str(config_file), "--get", key], check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def submodule_specs(config_file: Path) -> List[Tuple[str, str, str, Optional[str]]]:
    urls = git(["config", "--file", str(config_file), "--get-regexp",
                r"^submodule\..*\.url$"], check=False)
    if urls.returncode not in (0, 1):
        raise NightlyError(f"cannot read submodule configuration from {config_file}")
    specs: List[Tuple[str, str, str, Optional[str]]] = []
    for line in urls.stdout.splitlines():
        key, _, url = line.partition(" ")
        name = key[len("submodule."):-len(".url")]
        path = config_value(config_file, f"submodule.{name}.path")
        branch = config_value(config_file, f"submodule.{name}.branch")
        if not path or not url:
            raise NightlyError(f"incomplete submodule configuration for {name}")
        specs.append((name, path, url, branch))
    return specs


def pin_sources(source_url: str, branches: Sequence[str], input_root: Path) -> Dict[str, str]:
    """Fetch each branch once into a readonly local bare source, then pin its modules."""
    source = input_root / "source.git"
    git(["init", "--bare", "-q", str(source)])
    commits: Dict[str, str] = {}
    for branch in branches:
        refspec = f"+refs/heads/{branch}:refs/heads/{branch}"
        git(["-C", str(source), "fetch", "--no-tags", "--force", source_url, refspec])
        commits[branch] = git([
            "-C", str(source), "rev-parse", "--verify", f"refs/heads/{branch}^{{commit}}"
        ]).stdout.strip()
        log(f"pinned {branch} at {commits[branch]}")

    submodule_mirrors: Dict[str, Path] = {}
    submodule_urls: Dict[str, List[Tuple[str, Optional[str]]]] = {}
    for branch, commit in commits.items():
        config_text = git(["-C", str(source), "show", f"{commit}:.gitmodules"], check=False)
        if config_text.returncode:
            continue
        config_file = input_root / f"gitmodules-{branch_slug(branch)}.tmp"
        config_file.write_text(config_text.stdout, encoding="utf-8")
        try:
            specs = submodule_specs(config_file)
            for name, path, url, sub_branch in specs:
                tree = git(["-C", str(source), "ls-tree", commit, "--", path]).stdout.strip()
                fields = tree.split()
                if len(fields) < 3 or fields[0] != "160000" or fields[1] != "commit":
                    raise NightlyError(f"{branch} has no pinned submodule at {path}")
                pinned = fields[2]
                mirror = submodule_mirrors.setdefault(
                    name, input_root / "submodules" / f"{branch_slug(name)}.git")
                if not mirror.exists():
                    mirror.parent.mkdir(parents=True, exist_ok=True)
                    git(["init", "--bare", "-q", str(mirror)])
                if not git_has_commit(mirror, pinned):
                    submodule_urls.setdefault(name, []).append((url, sub_branch))
                    fetched = False
                    for candidate_url, candidate_branch in submodule_urls[name]:
                        if candidate_branch:
                            branch_ref = f"+refs/heads/{candidate_branch}:refs/heads/{candidate_branch}"
                            git(["-C", str(mirror), "fetch", "--no-tags", candidate_url, branch_ref], check=False)
                        if git_has_commit(mirror, pinned):
                            fetched = True
                            break
                        git(["-C", str(mirror), "fetch", "--no-tags", candidate_url, pinned], check=False)
                        if git_has_commit(mirror, pinned):
                            fetched = True
                            break
                    if not fetched:
                        raise NightlyError(
                            f"could not stage pinned submodule commit {pinned} for {branch}:{path}"
                        )
        finally:
            config_file.unlink(missing_ok=True)
    metadata = {
        "branches": commits,
        "source": source_url,
        "submodules": {
            name: str(path.relative_to(input_root)) for name, path in submodule_mirrors.items()
        },
    }
    (input_root / "branch-pins.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    return commits


class Settings:
    REQUIRED = (
        "NIGHTLY_MAC_RUNNER", "NIGHTLY_MAC_ROOT", "NIGHTLY_MAC_RUNS",
        "NIGHTLY_MAC_CATALOG", "NIGHTLY_MAC_ENGINE_A", "NIGHTLY_MAC_ENGINE_Q",
        "NIGHTLY_MAC_QUIET_LOCK", "NIGHTLY_MAC_ISOHOME", "NIGHTLY_MAC_PYTHON",
        "NIGHTLY_MAC_VM_TOOL",
    )

    def __init__(self, values: Dict[str, str]):
        missing = [name for name in self.REQUIRED if not values.get(name)]
        if missing:
            raise NightlyError("missing host configuration: " + ", ".join(missing))
        self.runner = Path(values["NIGHTLY_MAC_RUNNER"]).expanduser().resolve()
        self.root = Path(values["NIGHTLY_MAC_ROOT"]).expanduser()
        self.runs = Path(values["NIGHTLY_MAC_RUNS"]).expanduser()
        self.catalog = Path(values["NIGHTLY_MAC_CATALOG"]).expanduser().resolve()
        self.engine_a = Path(values["NIGHTLY_MAC_ENGINE_A"]).expanduser().resolve()
        self.engine_q = Path(values["NIGHTLY_MAC_ENGINE_Q"]).expanduser().resolve()
        self.quiet_lock = Path(values["NIGHTLY_MAC_QUIET_LOCK"]).expanduser()
        self.isohome = Path(values["NIGHTLY_MAC_ISOHOME"]).expanduser().resolve()
        self.python = values["NIGHTLY_MAC_PYTHON"]
        self.vm_tool = Path(values["NIGHTLY_MAC_VM_TOOL"]).expanduser().resolve()
        self.source_repo = values.get("NIGHTLY_MAC_SOURCE_REPO", DEFAULT_SOURCE_REPO)
        self.base = values.get("NIGHTLY_MAC_BASE", DEFAULT_BASE)
        self.status = Path(values.get("NIGHTLY_MAC_STATUS", str(self.runs / ".mac-nightly-running"))).expanduser()
        self.disk_root = Path(values.get("NIGHTLY_MAC_DISK_ROOT", "/Volumes/dev-scratch")).expanduser()
        try:
            self.min_free_gib = int(values.get("NIGHTLY_MAC_MIN_FREE_GIB", str(MIN_FREE_GIB)))
        except ValueError as error:
            raise NightlyError("NIGHTLY_MAC_MIN_FREE_GIB must be an integer") from error
        if self.min_free_gib < MIN_FREE_GIB:
            raise NightlyError(f"NIGHTLY_MAC_MIN_FREE_GIB cannot be below {MIN_FREE_GIB}")
        self.min_free_bytes = self.min_free_gib * GIB
        self.lock_dir = self.root / ".mac-nightly.lock"

    @classmethod
    def from_environment(cls, env: Dict[str, str]) -> "Settings":
        return cls(env)

    def validate(self) -> None:
        for path, label in (
            (self.runner / "tools/nightly/run_nightly.py", "nightly runner"),
            (self.runner / "tools/nightly/nightly-mac-guest.py", "Mac nightly controller"),
            (self.runner / "tools/nightly/branches.conf", "branch list"),
            (self.catalog, "catalog"),
            (self.engine_a / "engine/package-lock.json", "Engine A package lock"),
            (self.engine_a / "start-289.sh", "Engine A start script"),
            (self.engine_q / "engine/package-lock.json", "Engine Q package lock"),
            (self.engine_q / "start-289q.sh", "Engine Q start script"),
            (self.root / "fixtures/274bot-289-v17.navpack", "nightly nav fixture"),
            (self.root / "fixtures/inspect.fb", "nightly inspect fixture"),
            (self.isohome, "isohome helper"),
            (self.vm_tool, "mac_vm.py"),
        ):
            if not path.exists():
                raise NightlyError(f"missing {label}: {path}")
        if not self.isohome.is_file() or not os.access(str(self.isohome), os.X_OK):
            raise NightlyError(f"isohome helper is not an executable file: {self.isohome}")
        if not self.disk_root.is_dir():
            raise NightlyError(f"scratch volume is not mounted: {self.disk_root}")
        python_path = Path(self.python).expanduser()
        if python_path.is_absolute() and not python_path.is_file():
            raise NightlyError(f"host Python does not exist: {python_path}")
        if not python_path.is_absolute() and shutil.which(self.python) is None:
            raise NightlyError(f"host Python is not on PATH: {self.python}")


class TartVM:
    def __init__(self, settings: Settings, disk_check: Callable[[], int]):
        self.settings = settings
        self.disk_check = disk_check
        self.module = None
        self.session: Optional[Dict[str, Any]] = None
        self.session_id: Optional[str] = None
        self.vm_name: Optional[str] = None
        self.start_process: Optional[subprocess.Popen] = None
        self.active_pid_file: Optional[str] = None
        self.active_status_file: Optional[str] = None
        self.supervisor_path: Optional[Path] = None

    def start(
        self, quiet_lock_check: Optional[Callable[[], bool]] = None
    ) -> Optional[Dict[str, Any]]:
        module = load_vm_module(self.settings.vm_tool)
        self.module = module
        session_id = f"nightly-{uuid.uuid4().hex[:12]}"
        self.session_id = session_id
        self.vm_name = module.VM_PREFIX + session_id
        command = [
            self.settings.python, str(self.settings.vm_tool), "start", "--name", "nightly",
            "--session-id", session_id,
            "--cpu", str(VM_CPUS), "--memory", str(VM_MEMORY_MB), "--disk-size", str(VM_DISK_GB),
            "--base", self.settings.base, "--skip-tcc-bootstrap",
        ]
        log(f"starting Tart clone from {self.settings.base} ({VM_CPUS} CPU, "
            f"{VM_MEMORY_MB // 1024} GiB RAM, {VM_DISK_GB} GB disk)")
        blocked_signals = {signal.SIGTERM, signal.SIGINT, signal.SIGHUP}
        old_mask = signal.pthread_sigmask(signal.SIG_BLOCK, blocked_signals)
        try:
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
                # Exec preserves the mask, so clear it for the helper and its children.
                preexec_fn=lambda: signal.pthread_sigmask(
                    signal.SIG_SETMASK, set()
                ),
            )
            self.start_process = process
        except BaseException:
            self.session_id = None
            self.vm_name = None
            raise
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, old_mask)
        floor_error: Optional[str] = None
        while True:
            try:
                stdout, stderr = process.communicate(timeout=5)
                break
            except subprocess.TimeoutExpired:
                if quiet_lock_check is not None and quiet_lock_check():
                    log("stopping mac_vm.py start because MAC-QUIET.lock appeared")
                    if not self.terminate_start():
                        raise NightlyError(
                            "mac_vm.py start process did not stop after MAC-QUIET.lock appeared"
                        )
                    return None
                try:
                    self.disk_check()
                except DiskFloorError as error:
                    floor_error = str(error)
                    log(floor_error + "; waiting for Tart start to return so the clone can be stopped")
        self.start_process = None
        if process.returncode:
            detail = (stderr or stdout or "").strip()
            raise NightlyError(f"mac_vm.py start failed ({process.returncode}): {detail}")
        try:
            result = json.loads(stdout)
            if not isinstance(result, dict):
                raise TypeError("start result is not an object")
            if result.get("id") != session_id:
                raise ValueError("start result has a different session ID")
            self.session = module.load_session(session_id)
        except (ValueError, TypeError, KeyError, AttributeError, OSError) as error:
            if floor_error:
                raise DiskFloorError(floor_error) from error
            raise NightlyError(f"mac_vm.py returned an invalid session: {error}") from error
        if not isinstance(self.session, dict):
            raise NightlyError("mac_vm.py session has an invalid type")
        required = ("id", "vm", "ip", "guest_home", "share")
        absent = [name for name in required if not self.session.get(name)]
        if absent:
            raise NightlyError("mac_vm.py session is missing: " + ", ".join(absent))
        if self.session["vm"] != self.vm_name:
            raise NightlyError("mac_vm.py returned a different clone name")
        if floor_error:
            raise DiskFloorError(floor_error)
        self.disk_check()
        log(f"started {result['vm']} ({result['id']}) at {result['ip']}")
        return self.session

    def terminate_start(self) -> bool:
        process = self.start_process
        if process is None:
            return True
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        except OSError as error:
            log(f"could not signal mac_vm.py start process group: {error}")
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except OSError as error:
                log(f"could not kill mac_vm.py start process group: {error}")
                try:
                    process.kill()
                except OSError:
                    pass
            try:
                process.wait(timeout=15)
            except (OSError, subprocess.TimeoutExpired) as error:
                log(f"mac_vm.py start process did not stop: {error}")
                return False
        except OSError as error:
            log(f"could not wait for mac_vm.py start process: {error}")
            return False
        self.start_process = None
        log("stopped mac_vm.py start process group")
        return True

    def guest_sh(self, command: str, *, timeout: float = 45) -> str:
        if self.module is None or self.session is None:
            raise NightlyError("Tart session is not available")
        return self.module.guest_sh(self.session, command, timeout=timeout, check=True)

    def run_supervised(self, command: Sequence[str], key: str,
                       progress: Callable[[], bool]) -> int:
        if self.session is None or self.supervisor_path is None:
            raise NightlyError("Tart session is not ready for a guest command")
        guest_home = str(self.session["guest_home"])
        run_root = f"{guest_home}/nightly-control"
        logs = f"{GUEST_SHARE}/logs"
        statuses = f"{run_root}/status"
        pidfiles = f"{run_root}/pids"
        log_path = f"{logs}/{key}.log"
        status_path = f"{statuses}/{key}.status"
        pid_path = f"{pidfiles}/{key}.pid"
        guest_supervisor = f"{GUEST_SHARE}/control/supervisor.sh"
        self.guest_sh(
            "mkdir -p " + " ".join(shell_quote(path) for path in (logs, statuses, pidfiles)) +
            " && rm -f " + " ".join(shell_quote(path) for path in (status_path, pid_path)) +
            " && (nohup /bin/bash " + shell_quote(guest_supervisor) + " " +
            " ".join(shell_quote(path) for path in (log_path, status_path)) + " " +
            " ".join(shell_quote(arg) for arg in command) +
            " </dev/null >/dev/null 2>&1 & echo $! > " + shell_quote(pid_path) + ")"
        )
        self.active_pid_file = pid_path
        self.active_status_file = status_path
        last_output = ""
        disk_abort: Optional[str] = None
        while True:
            if disk_abort is None:
                try:
                    self.disk_check()
                except DiskFloorError as error:
                    disk_abort = str(error)
                    log(disk_abort + "; stopping the active guest task")
                    terminated = self.terminate_active()
                    if not terminated:
                        raise DiskFloorError(disk_abort)
            progress()
            state = self.guest_sh(
                "if [ -f " + shell_quote(status_path) + " ]; then printf '__DONE__'; "
                "cat " + shell_quote(status_path) + "; printf '\\n'; "
                "tail -n 12 " + shell_quote(log_path) + " 2>/dev/null || true; "
                "else printf '__RUNNING__\\n'; "
                "tail -n 12 " + shell_quote(log_path) + " 2>/dev/null || true; fi"
            )
            if state.startswith("__DONE__"):
                result_lines = state[len("__DONE__"):].strip().splitlines()
                try:
                    code = int(result_lines[0])
                except (ValueError, IndexError) as error:
                    raise NightlyError(f"guest task wrote invalid status: {state!r}") from error
                output = "\n".join(result_lines[1:]).strip()
                if output and output != last_output:
                    log(output)
                self.active_pid_file = None
                self.active_status_file = None
                if disk_abort:
                    raise DiskFloorError(disk_abort)
                return code
            output = state[len("__RUNNING__"):].strip()
            if output and output != last_output:
                log(output)
                last_output = output
            time.sleep(5)

    def terminate_active(self) -> bool:
        if self.session is None or not self.active_pid_file:
            return True
        pid_file = self.active_pid_file
        completed = False
        try:
            self.guest_sh(
                "if [ -f " + shell_quote(pid_file) + " ]; then kill -TERM "
                "$(cat " + shell_quote(pid_file) + ") 2>/dev/null || true; fi"
            )
            if self.active_status_file:
                deadline = time.monotonic() + 120
                while time.monotonic() < deadline:
                    state = self.guest_sh(
                        "if [ -f " + shell_quote(self.active_status_file) + " ]; then echo done; "
                        "else echo running; fi"
                    )
                    if state.strip() == "done":
                        completed = True
                        break
                    time.sleep(5)
        except Exception as error:
            log(f"could not stop the guest supervisor cleanly: {error}")
        if completed:
            self.active_pid_file = None
            self.active_status_file = None
            return True
        log("guest supervisor did not stop within 120 seconds")
        self.active_pid_file = None
        self.active_status_file = None
        return False

    def stop(self) -> None:
        if not self.session_id:
            return
        command = [self.settings.python, str(self.settings.vm_tool), "stop", self.session_id]
        result = run_command(command, check=False, timeout=150)
        if result.returncode:
            detail = (result.stderr or result.stdout or "").strip()
            raise NightlyError(f"mac_vm.py stop failed for {self.session_id}: {detail}")
        log(f"stopped and deleted Tart clone {self.session_id}")
        self.session = None
        self.session_id = None
        self.vm_name = None
        self.start_process = None


def load_vm_module(path: Path):
    spec = importlib.util.spec_from_file_location("nightly_mac_vm", str(path))
    if spec is None or spec.loader is None:
        raise NightlyError(f"cannot import mac_vm.py: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_supervisor(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "#!/bin/bash\n"
        "set -uo pipefail\n"
        "log=$1\nstatus=$2\nshift 2\nchild=\n"
        "write_status() { local temporary=\"$status.tmp.$$\"; "
        "printf '%s\\n' \"$1\" >\"$temporary\"; mv -f \"$temporary\" \"$status\"; }\n"
        "stop_child() {\n"
        "  trap - HUP INT TERM\n"
        "  local code=143\n"
        "  if [ -n \"$child\" ]; then\n"
        "    kill -TERM \"$child\" 2>/dev/null || true\n"
        "    wait \"$child\"; code=$?\n"
        "  fi\n"
        "  write_status \"$code\"\n"
        "  exit 0\n"
        "}\n"
        "trap stop_child HUP INT TERM\n"
        "\"$@\" >\"$log\" 2>&1 &\n"
        "child=$!\n"
        "wait \"$child\"; code=$?\n"
        "write_status \"$code\"\n"
        "exit 0\n",
        encoding="utf-8",
    )
    path.chmod(0o444)


def download(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(url, headers={"User-Agent": "274bot-nightly"})
    with urllib.request.urlopen(request, timeout=60) as response, destination.open("wb") as output:
        shutil.copyfileobj(response, output, length=1024 * 1024)


def safe_extract_node(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    root = destination.resolve()
    with tarfile.open(str(archive), mode="r:gz") as bundle:
        for member in bundle:
            relative = PurePosixPath(member.name)
            if relative.is_absolute() or not relative.parts or ".." in relative.parts:
                raise NightlyError(f"unsafe Node archive member: {member.name!r}")
            target = destination.joinpath(*relative.parts)
            if not is_within(target.resolve(strict=False), root):
                raise NightlyError(f"Node archive member escapes tool directory: {member.name!r}")
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                os.chmod(str(target), member.mode & 0o777)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                source = bundle.extractfile(member)
                if source is None:
                    raise NightlyError(f"Node archive member has no data: {member.name}")
                with source, target.open("wb") as output:
                    shutil.copyfileobj(source, output, length=1024 * 1024)
                os.chmod(str(target), member.mode & 0o777)
            elif member.issym() or member.islnk():
                link = PurePosixPath(member.linkname)
                if link.is_absolute():
                    raise NightlyError(f"absolute link in Node archive: {member.name!r}")
                link_target = target.parent.joinpath(*link.parts)
                if not is_within(link_target.resolve(strict=False), root):
                    raise NightlyError(f"Node archive link escapes tool directory: {member.name!r}")
                target.parent.mkdir(parents=True, exist_ok=True)
                if member.issym():
                    os.symlink(member.linkname, str(target))
                else:
                    hard_target = destination.joinpath(*link.parts)
                    os.link(str(hard_target), str(target))
            else:
                raise NightlyError(f"unsupported Node archive member: {member.name!r}")


def install_python(tools_root: Path, home: Path) -> Path:
    if sys.version_info >= (3, 10):
        return Path(sys.executable)
    bin_dir = tools_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    uv = bin_dir / "uv"
    if not uv.is_file():
        install_script = tools_root / "uv-install.sh"
        download("https://astral.sh/uv/install.sh", install_script)
        env = os.environ.copy()
        env.update({
            "HOME": str(home),
            "UV_INSTALL_DIR": str(bin_dir),
            "UV_NO_MODIFY_PATH": "1",
            "UV_PYTHON_INSTALL_DIR": str(tools_root / "python"),
            "UV_CACHE_DIR": str(tools_root / "uv-cache"),
        })
        run_command(["/bin/sh", str(install_script)], env=env)
    env = os.environ.copy()
    env.update({
        "HOME": str(home),
        "UV_PYTHON_INSTALL_DIR": str(tools_root / "python"),
        "UV_CACHE_DIR": str(tools_root / "uv-cache"),
    })
    run_command([str(uv), "python", "install", PYTHON_VERSION], env=env)
    result = run_command([str(uv), "python", "find", PYTHON_VERSION], env=env)
    python_path = Path(result.stdout.strip().splitlines()[-1])
    if not python_path.is_file():
        raise NightlyError(f"uv did not install Python {PYTHON_VERSION}: {python_path}")
    return python_path


def install_node(tools_root: Path, home: Path) -> Tuple[Path, str]:
    if platform.machine().lower() not in ("arm64", "aarch64"):
        raise NightlyError(f"Tart guest architecture is not Apple Silicon: {platform.machine()}")
    version = NODE_VERSION
    filename = f"node-v{version}-darwin-arm64.tar.gz"
    base_url = f"https://nodejs.org/dist/v{version}"
    checksums = tools_root / "node-SHASUMS256.txt"
    archive = tools_root / filename
    download(base_url + "/SHASUMS256.txt", checksums)
    expected = None
    for line in checksums.read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[1] == filename:
            expected = fields[0]
            break
    if not expected or not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise NightlyError(f"Node {version} checksums do not list {filename}")
    download(base_url + "/" + filename, archive)
    if sha256_file(archive) != expected:
        raise NightlyError(f"Node {version} archive checksum mismatch")
    extract_root = tools_root / "node-extract"
    if extract_root.exists():
        shutil.rmtree(str(extract_root))
    safe_extract_node(archive, extract_root)
    node_root = extract_root / f"node-v{version}-darwin-arm64"
    node = node_root / "bin/node"
    npm = node_root / "bin/npm"
    if not node.is_file() or not npm.exists():
        raise NightlyError(f"Node {version} archive has no node/npm binaries")
    os.chmod(str(node), os.stat(str(node)).st_mode | stat.S_IXUSR)
    os.chmod(str(npm), os.stat(str(npm)).st_mode | stat.S_IXUSR)
    shutil.rmtree(str(tools_root / f"node-v{version}-darwin-arm64"), ignore_errors=True)
    node_root.rename(tools_root / f"node-v{version}-darwin-arm64")
    shutil.rmtree(str(extract_root))
    archive.unlink(missing_ok=True)
    checksums.unlink(missing_ok=True)
    return tools_root / f"node-v{version}-darwin-arm64" / "bin", version


def install_rust(guest_home: Path, home: Path) -> Tuple[Path, Path]:
    cargo_home = guest_home / ".cargo"
    rustup_home = guest_home / ".rustup"
    cargo_home.mkdir(parents=True, exist_ok=True)
    rustup_home.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update({
        "HOME": str(home),
        "CARGO_HOME": str(cargo_home),
        "RUSTUP_HOME": str(rustup_home),
        "RUSTUP_INIT_SKIP_PATH_CHECK": "yes",
    })
    rustup = cargo_home / "bin/rustup"
    if not rustup.exists():
        init_path = guest_home / "rustup-init"
        download("https://static.rust-lang.org/rustup/dist/aarch64-apple-darwin/rustup-init", init_path)
        os.chmod(str(init_path), 0o755)
        run_command([
            str(init_path), "-y", "--no-modify-path", "--profile", "minimal",
            "--default-toolchain", "none",
        ], env=env)
        init_path.unlink(missing_ok=True)
    run_command([
        str(rustup), "toolchain", "install", RUST_VERSION, "--profile", "minimal",
        "--component", "clippy", "--component", "rustfmt",
    ], env=env)
    return cargo_home, rustup_home


def copy_guest_input(source: Path, destination: Path) -> None:
    if destination.exists() or destination.is_symlink():
        raise NightlyError(f"guest input already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(str(source), str(destination), symlinks=True)
    # Staged host inputs stay read-only, but each private guest copy must be writable.
    for base, _directories, files in os.walk(str(destination), followlinks=False):
        directory = Path(base)
        directory.chmod(stat.S_IMODE(directory.stat().st_mode) | stat.S_IWUSR)
        for name in files:
            path = directory / name
            if not path.is_symlink():
                path.chmod(stat.S_IMODE(path.stat().st_mode) | stat.S_IWUSR)


def guest_bootstrap(input_root: Path, guest_root: Path, guest_home: Path) -> Dict[str, str]:
    """Install pinned guest tools and prepare the private Engine Q tree."""
    input_root = input_root.resolve(strict=True)
    guest_home.mkdir(parents=True, exist_ok=True)
    home = guest_home / "home"
    tools_root = guest_home / "toolchains"
    temp_root = guest_home / "tmp"
    for path in (home, tools_root, temp_root, guest_root):
        path.mkdir(parents=True, exist_ok=True)
    staged_isohome = input_root / "isohome"
    isohome = tools_root / "bin/isohome"
    if not staged_isohome.is_file():
        raise NightlyError(f"staged isohome helper is missing: {staged_isohome}")
    isohome.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(str(staged_isohome), str(isohome))
    os.chmod(str(isohome), 0o555)
    if platform.system() != "Darwin":
        raise NightlyError("the nightly guest is not macOS")
    if platform.machine().lower() not in ("arm64", "aarch64"):
        raise NightlyError(f"the nightly guest is not Apple Silicon: {platform.machine()}")

    input_engine_a = input_root / "engine-a"
    input_engine_q = input_root / "engine-q"
    copy_guest_input(input_engine_a, guest_root / "engine-pristine")
    copy_guest_input(input_engine_q, guest_root / "engine-q-pristine")
    input_snapshot = input_root / "nav-snapshot"
    if input_snapshot.is_dir():
        copy_guest_input(input_snapshot, guest_root / "nav-snapshot")
    guest_mirrors = guest_root / "mirror"
    staged_mirrors = input_root / "submodules"
    if staged_mirrors.is_dir():
        guest_mirrors.mkdir(parents=True, exist_ok=True)
        for mirror in sorted(staged_mirrors.iterdir(), key=lambda path: path.name):
            if mirror.is_dir() and not mirror.is_symlink():
                copy_guest_input(mirror, guest_mirrors / mirror.name)

    python_path = install_python(tools_root, home)
    node_bin, node_version = install_node(tools_root, home)
    cargo_home, rustup_home = install_rust(guest_home, home)
    path = os.pathsep.join((str(node_bin), str(cargo_home / "bin"), str(tools_root / "bin"),
                            "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"))
    env = os.environ.copy()
    env.update({
        "HOME": str(home), "TMPDIR": str(temp_root), "PATH": path,
        "CARGO_HOME": str(cargo_home), "RUSTUP_HOME": str(rustup_home),
        "NPM_CONFIG_CACHE": str(guest_home / "npm-cache"),
    })
    run_command(["lsof", "-v"], env=env)

    engine = guest_root / "engine-q-pristine"
    engine_dir = engine / "engine"
    if not (engine_dir / "package-lock.json").is_file():
        raise NightlyError(f"Engine Q package-lock.json is missing: {engine_dir}")
    run_command([str(node_bin / "npm"), "ci", "--ignore-scripts"], cwd=engine_dir, env=env)
    run_command([str(node_bin / "npm"), "run", "sqlite:migrate"], cwd=engine_dir, env=env)

    rustc = run_command([str(cargo_home / "bin/rustc"), "+" + RUST_VERSION, "--version"], env=env).stdout.strip()
    cargo = run_command([str(cargo_home / "bin/cargo"), "+" + RUST_VERSION, "--version"], env=env).stdout.strip()
    node = run_command([str(node_bin / "node"), "--version"], env=env).stdout.strip()
    npm = run_command([str(node_bin / "npm"), "--version"], env=env).stdout.strip()
    python_version = run_command([str(python_path), "--version"], env=env).stdout.strip()
    tools = {
        "python": str(python_path), "python_version": python_version,
        "rustc": rustc, "cargo": cargo, "cargo_home": str(cargo_home),
        "rustup_home": str(rustup_home), "node": node, "node_version": node_version,
        "npm": npm, "node_bin": str(node_bin), "home": str(home),
        "tmp": str(temp_root), "path": path, "isohome": str(isohome),
    }
    tool_manifest = guest_root / "toolchains.json"
    tool_manifest.write_text(json.dumps(tools, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(tools, sort_keys=True), flush=True)
    return tools


def add_archive_tree(bundle: tarfile.TarFile, root: Path) -> None:
    root_info = os.lstat(str(root))
    if not stat.S_ISDIR(root_info.st_mode) or stat.S_ISLNK(root_info.st_mode):
        raise NightlyError(f"run output is not a real directory: {root}")
    root_member = tarfile.TarInfo(root.name)
    root_member.type = tarfile.DIRTYPE
    root_member.mode = stat.S_IMODE(root_info.st_mode)
    root_member.mtime = int(root_info.st_mtime)
    bundle.addfile(root_member)
    for directory, names, files in os.walk(str(root), topdown=True, followlinks=False):
        base = Path(directory)
        names[:] = sorted(name for name in names if not (base / name).is_symlink())
        for name in names:
            path = base / name
            info = os.lstat(str(path))
            relative = path.relative_to(root).as_posix()
            member = tarfile.TarInfo(f"{root.name}/{relative}")
            member.type = tarfile.DIRTYPE
            member.mode = stat.S_IMODE(info.st_mode)
            member.mtime = int(info.st_mtime)
            bundle.addfile(member)
        for name in sorted(files):
            path = base / name
            info = os.lstat(str(path))
            if stat.S_ISLNK(info.st_mode):
                continue
            if not stat.S_ISREG(info.st_mode):
                raise NightlyError(f"unsupported run output type: {path}")
            relative = path.relative_to(root).as_posix()
            member = tarfile.TarInfo(f"{root.name}/{relative}")
            member.size = info.st_size
            member.mode = stat.S_IMODE(info.st_mode)
            member.mtime = int(info.st_mtime)
            with path.open("rb") as source:
                bundle.addfile(member, source)


def export_run_archives(runs_root: Path, outbox: Path) -> List[Path]:
    """Copy real run files to per-session archives on the host virtiofs share."""
    outbox.mkdir(parents=True, exist_ok=True)
    exported: List[Path] = []
    for run_dir in sorted(runs_root.iterdir(), key=lambda path: path.name):
        if run_dir.is_symlink() or not run_dir.is_dir():
            continue
        if not (run_dir / "SHA256SUMS").is_file():
            write_sha256sums(run_dir)
        archive = outbox / f"{run_dir.name}.tar"
        if archive.exists():
            continue
        temporary = archive.with_name(archive.name + ".tmp")
        temporary.unlink(missing_ok=True)
        with tarfile.open(str(temporary), mode="w:") as bundle:
            add_archive_tree(bundle, run_dir)
        temporary.replace(archive)
        exported.append(archive)
        print(f"exported {run_dir.name}", flush=True)
    return exported


def guest_cli(argv: Sequence[str]) -> Optional[int]:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--guest-bootstrap", nargs=3, metavar=("INPUT", "NIGHTLY_ROOT", "GUEST_HOME"))
    parser.add_argument("--export-runs", nargs=2, metavar=("RUNS_ROOT", "OUTBOX"))
    args, remaining = parser.parse_known_args(list(argv))
    if args.guest_bootstrap:
        guest_bootstrap(*(Path(value) for value in args.guest_bootstrap))
        return 0
    if args.export_runs:
        export_run_archives(Path(args.export_runs[0]), Path(args.export_runs[1]))
        return 0
    if remaining:
        return None
    return None


class NightlyOrchestrator:
    def __init__(self, settings: Settings, branches: Sequence[str],
                 vm_factory: Callable[..., TartVM] = TartVM,
                 branch_list_info: Optional[Dict[str, str]] = None,
                 self_update_info: Optional[Dict[str, str]] = None):
        self.settings = settings
        self.branches = list(branches)
        self.branch_list_info = branch_list_info
        self.self_update_info = self_update_info
        self.vm = vm_factory(settings, self.check_disk)
        self.session: Optional[Dict[str, Any]] = None
        self.input_root: Optional[Path] = None
        self.control_root: Optional[Path] = None
        self.pins: Dict[str, str] = {}
        self.owned_lock = False
        self.quiet_stopped = False
        self.published_ok = False
        self.stop_ok = False
        self.run_failures: List[str] = []
        self.active_job = "starting"
        self.started = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())

    def check_disk(self) -> int:
        try:
            free = shutil.disk_usage(str(self.settings.disk_root)).free
        except OSError as error:
            raise DiskFloorError(f"cannot read scratch free space: {error}") from error
        if free <= self.settings.min_free_bytes:
            free_gib = free / GIB
            raise DiskFloorError(
                f"scratch has {free_gib:.1f} GiB free, at or below the "
                f"{self.settings.min_free_gib} GiB floor"
            )
        return free

    def acquire_lock(self) -> bool:
        self.settings.root.mkdir(parents=True, exist_ok=True)
        self.settings.runs.mkdir(parents=True, exist_ok=True)
        if self.settings.quiet_lock.exists():
            log(f"skipped because MAC-QUIET.lock exists: {self.settings.quiet_lock}")
            return False
        try:
            self.settings.lock_dir.mkdir()
        except FileExistsError:
            log(f"another Mac nightly owns {self.settings.lock_dir}")
            return False
        self.owned_lock = True
        try:
            self.write_status("starting")
        except Exception:
            self.settings.lock_dir.rmdir()
            self.owned_lock = False
            raise
        return True

    def write_status(self, state: str) -> None:
        status = self.settings.status
        status.parent.mkdir(parents=True, exist_ok=True)
        lines = [
            f"pid={os.getpid()}", f"job={self.active_job}", f"state={state}",
            f"started={self.started}",
            f"updated={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}",
            f"quiet_lock={self.settings.quiet_lock}",
            f"dev_commit={self.pins.get('dev', '')}",
            f"branches={','.join(self.branches)}",
        ]
        if self.branch_list_info is not None:
            lines.append(f"branch_list_source={self.branch_list_info.get('source', '')}")
            lines.append(f"branch_list_commit={self.branch_list_info.get('commit', '')}")
            if self.branch_list_info.get("note"):
                lines.append(f"branch_list_note={self.branch_list_info['note']}")
        if self.self_update_info is not None:
            lines.append(f"self_update={self.self_update_info.get('status', '')}")
            if self.self_update_info.get("detail"):
                lines.append(f"self_update_detail={self.self_update_info['detail']}")
        if self.vm.session_id:
            lines.append(f"vm_id={self.vm.session_id}")
        for branch, commit in self.pins.items():
            lines.append(f"commit_{branch_slug(branch)}={commit}")
        temporary = status.with_name(status.name + f".tmp.{os.getpid()}")
        temporary.write_text("\n".join(lines) + "\n", encoding="utf-8")
        os.replace(str(temporary), str(status))

    def stage_inputs(self) -> None:
        if self.vm.session is None:
            raise NightlyError("Tart session is not available for input staging")
        share = Path(self.vm.session["share"])
        self.input_root = share / "input"
        self.control_root = share / "control"
        self.input_root.mkdir(parents=True, exist_ok=False)
        self.control_root.mkdir(parents=True, exist_ok=False)
        copy_tree(self.settings.runner, self.input_root / "runner", kind="runner")
        copy_tree(self.settings.catalog, self.input_root / "catalog", kind="catalog")
        copy_tree(self.settings.engine_a, self.input_root / "engine-a", kind="engine")
        copy_tree(self.settings.engine_q, self.input_root / "engine-q", kind="engine")
        shutil.copy2(str(self.settings.isohome), str(self.input_root / "isohome"))
        fixture_root = self.settings.root / "fixtures"
        copy_tree(fixture_root, self.input_root / "fixtures", kind="fixtures")
        snapshot = self.settings.root / "nav-snapshot"
        if snapshot.is_dir():
            copy_tree(snapshot, self.input_root / "nav-snapshot", kind="snapshot")
        self.pins = pin_sources(self.settings.source_repo, self.branches, self.input_root)
        for path in sorted(self.input_root.iterdir(), key=lambda item: item.name):
            make_readonly(path)
        make_readonly(self.input_root)
        write_supervisor(self.control_root / "supervisor.sh")
        make_readonly(self.control_root)
        self.vm.supervisor_path = self.control_root / "supervisor.sh"
        (share / "results").mkdir(exist_ok=True)
        (share / "quiet").mkdir(exist_ok=True)
        self.write_status("inputs staged")
        log("staged runner source, vendor pins, catalog, engine source trees, fixtures, and nav snapshot")
        self.check_disk()

    def input_paths(self) -> Dict[str, str]:
        if self.input_root is None or self.vm.session is None:
            raise NightlyError("nightly inputs are not staged")
        share_input = f"{GUEST_SHARE}/input"
        guest_home = Path(str(self.vm.session["guest_home"]))
        guest_root = guest_home / "nightly"
        tools_root = guest_home / "toolchains"
        return {
            "guest_home": str(guest_home),
            "guest_root": str(guest_root),
            "tools_root": str(tools_root),
            "input": share_input,
            "runner": f"{share_input}/runner",
            "script": f"{share_input}/runner/tools/nightly/nightly-mac-guest.py",
            "source": f"{share_input}/source.git",
            "catalog": f"{share_input}/catalog",
            "fixtures": f"{share_input}/fixtures",
            "nav_snapshot": str(guest_root / "nav-snapshot"),
            "quiet_lock": f"{GUEST_SHARE}/quiet/MAC-QUIET.lock",
            "outbox": f"{GUEST_SHARE}/results",
        }

    def sync_quiet_lock(self) -> bool:
        if self.settings.quiet_lock.exists():
            if not self.quiet_stopped:
                if self.vm.session is not None:
                    quiet_path = f"{GUEST_SHARE}/quiet/MAC-QUIET.lock"
                    self.vm.guest_sh("mkdir -p " + shell_quote(f"{GUEST_SHARE}/quiet") +
                                     " && touch " + shell_quote(quiet_path))
                    log("MAC-QUIET.lock appeared; guest live cells will stop at the next boundary")
                else:
                    log("MAC-QUIET.lock appeared during guest start; stopping before staging")
            self.quiet_stopped = True
        return self.quiet_stopped

    def run_bootstrap(self) -> Dict[str, str]:
        paths = self.input_paths()
        self.active_job = "guest bootstrap"
        self.write_status("bootstrapping guest dependencies")
        command = [
            "/usr/bin/python3", paths["script"], "--guest-bootstrap", paths["input"],
            paths["guest_root"], paths["guest_home"],
        ]
        code = self.vm.run_supervised(command, "bootstrap", self.sync_quiet_lock)
        if code:
            raise NightlyError(f"guest dependency bootstrap failed with exit {code}")
        raw = self.vm.guest_sh("cat " + shell_quote(paths["guest_root"] + "/toolchains.json"))
        try:
            tools = json.loads(raw)
        except ValueError as error:
            raise NightlyError(f"guest bootstrap returned invalid toolchain metadata: {error}") from error
        for key in ("python", "cargo_home", "rustup_home", "node_bin", "home", "tmp", "isohome"):
            if key not in tools:
                raise NightlyError(f"guest bootstrap omitted {key}")
        log("guest dependencies ready: " + ", ".join(
            tools[key] for key in ("python_version", "rustc", "node", "npm") if key in tools))
        return tools

    def job_environment(self, branch: str, job: str, tools: Dict[str, str]) -> Dict[str, str]:
        paths = self.input_paths()
        root = paths["guest_root"]
        main = job == "main"
        # mac_vm.py reserves 44594/1080 for its host Engine A loopback forward.
        env = {
            "HOME": tools["home"],
            "TMPDIR": tools["tmp"],
            "PATH": os.pathsep.join((tools["node_bin"], str(Path(tools["cargo_home"]) / "bin"),
                                     str(Path(paths["tools_root"]) / "bin"),
                                     "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin")),
            "CARGO_HOME": tools["cargo_home"],
            "RUSTUP_HOME": tools["rustup_home"],
            "NIGHTLY_ROOT": root,
            "NIGHTLY_RUNS_ROOT": f"{root}/runs",
            "NIGHTLY_REPO": paths["source"],
            "NIGHTLY_FRESH_TREES": "1",
            "NIGHTLY_CATALOG": paths["catalog"],
            "NIGHTLY_NAV_SNAPSHOT": paths["nav_snapshot"],
            "NIGHTLY_NAV_FIXTURE": f"{paths['fixtures']}/274bot-289-v17.navpack",
            "NIGHTLY_INSPECT_BASELINE": f"{paths['fixtures']}/inspect.fb",
            "NIGHTLY_QUIET_LOCK": paths["quiet_lock"],
            "NIGHTLY_ISOHOME": tools["isohome"],
            "ISOHOME_ROOT": f"{paths['guest_home']}/isohome",
            "NIGHTLY_PYTHON": tools["python"],
            "NIGHTLY_REVISION": "289",
            "NIGHTLY_ENGINE": "remote" if main else "local",
            "NIGHTLY_ENGINE_HOST": "127.0.0.1",
            "NIGHTLY_TZ": "America/New_York",
            "NIGHTLY_PANEL_PARALLEL": "3" if main else "1",
            "NIGHTLY_PARALLEL": "4" if main else "1",
            "NIGHTLY_TARGET_CAP_GB": "32",
            "NIGHTLY_MIN_FREE_GB": "35",
            "NIGHTLY_JOB": job,
            "NIGHTLY_STEPS": "lanes,panel,tui" if main else "panel,tui",
            "NIGHTLY_PROFILE": "local-289" if main else "local-289q",
            "NIGHTLY_ENGINE_LABEL": "Engine A" if main else "Engine Q",
            "NIGHTLY_GAME_PORT": "44594" if main else "44694",
            "NIGHTLY_HTTP_PORT": "1080" if main else "1180",
            "NIGHTLY_ENGINE_PRISTINE": f"{root}/engine-pristine" if main else f"{root}/engine-q-pristine",
            "NIGHTLY_ENGINE_RUN": f"{root}/engine-run" if main else f"{root}/engine-q-run",
            "NIGHTLY_ENGINE_SCRIPT_REVISION": "289" if main else "289q",
            "NIGHTLY_RUN_SUFFIX": "-mac" if main else "-mac-quest",
            "NIGHTLY_NAME_PREFIX": "nm" if main else "nq",
            "NIGHTLY_PANEL_NAME_PREFIX": "nmp" if main else "nqp",
        }
        return env

    def guest_command(self, branch: str, job: str, tools: Dict[str, str]) -> List[str]:
        paths = self.input_paths()
        env = self.job_environment(branch, job, tools)
        command = ["/usr/bin/env"]
        command.extend(f"{key}={value}" for key, value in env.items())
        command.extend([tools["python"], "-u", f"{paths['runner']}/tools/nightly/run_nightly.py",
                        "--branch", branch])
        return command

    def export_runs(self) -> None:
        if self.vm.session is None or self.input_root is None:
            # Nothing was staged (for example a quiet-lock skip right after the
            # start), so the guest ran nothing and has no run folders to export.
            return
        paths = self.input_paths()
        python = "/usr/bin/python3"
        toolchains = f"{paths['guest_root']}/toolchains.json"
        try:
            raw = self.vm.guest_sh("cat " + shell_quote(toolchains))
            tools = json.loads(raw)
            python = tools.get("python", python)
        except Exception:
            pass
        self.vm.guest_sh(
            "mkdir -p " + shell_quote(paths["outbox"]) + " && if [ -d " +
            shell_quote(f"{paths['guest_root']}/runs") + " ]; then " + shell_quote(python) + " " +
            shell_quote(paths["script"]) + " --export-runs " +
            shell_quote(f"{paths['guest_root']}/runs") + " " + shell_quote(paths["outbox"]) + "; fi"
        )

    def clean_branch_targets(self, branch: str) -> None:
        if self.vm.session is None:
            return
        root = Path(str(self.vm.session["guest_home"])) / "nightly"
        slug = branch_slug(branch)
        targets = [root / "target" / slug, root / "target" / f"{slug}-ci",
                   root / "trees" / slug]
        command = "rm -rf -- " + " ".join(shell_quote(str(path)) for path in targets)
        self.vm.guest_sh(command)

    def publish(self) -> None:
        if self.session is None:
            return
        outbox = Path(str(self.session["share"])) / "results"
        self.check_disk()
        published = publish_archives(
            outbox, self.settings.runs, self.check_disk, self.settings.min_free_bytes
        )
        self.published_ok = True
        if not published:
            log("no completed run folders were available to copy")

    def run(self) -> int:
        self.settings.validate()
        self.check_disk()
        if not self.acquire_lock():
            return 0
        result = 0
        error: Optional[BaseException] = None
        try:
            if self.sync_quiet_lock():
                log("skipped before guest start because MAC-QUIET.lock exists")
            else:
                self.vm.start(self.sync_quiet_lock)
                self.session = self.vm.session
                self.check_disk()
                tools: Optional[Dict[str, str]] = None
                if self.sync_quiet_lock():
                    log("stopped before guest staging because MAC-QUIET.lock appeared")
                else:
                    self.stage_inputs()
                    if self.sync_quiet_lock():
                        log("stopped before guest bootstrap because MAC-QUIET.lock appeared")
                    else:
                        tools = self.run_bootstrap()
                if tools is not None:
                    for branch in self.branches:
                        if self.sync_quiet_lock():
                            log(f"skipped {branch}; MAC-QUIET.lock is present")
                            break
                        self.active_job = branch
                        self.write_status(f"running branch {branch}")
                        log(f"starting exact commit {self.pins[branch]} for {branch}")
                        for job in ("main", "quest"):
                            if self.sync_quiet_lock():
                                log(f"skipped {job} for {branch}; MAC-QUIET.lock is present")
                                break
                            self.active_job = f"{branch} {job}"
                            self.write_status("running")
                            log(f"starting {job} for {branch}")
                            command = self.guest_command(branch, job, tools)
                            if self.sync_quiet_lock():
                                log(f"skipped {job} for {branch}; MAC-QUIET.lock is present before cell batch")
                                break
                            code = self.vm.run_supervised(
                                command,
                                f"{branch_slug(branch)}-{job}", self.sync_quiet_lock,
                            )
                            if code:
                                self.run_failures.append(f"{branch} {job}: exit {code}")
                                log(f"{branch} {job} exited {code}; the other job remains required")
                            try:
                                self.export_runs()
                            except Exception as export_error:
                                self.run_failures.append(f"{branch} {job}: result export failed")
                                log(f"could not export {branch} {job} results yet: {export_error}")
                            if self.sync_quiet_lock():
                                log("not starting another job because MAC-QUIET.lock is present")
                                break
                        self.clean_branch_targets(branch)
                        if self.quiet_stopped:
                            break
            if self.run_failures:
                result = 1
        except BaseException as caught:
            error = caught
            result = 1
        finally:
            blocked_signals = {signal.SIGTERM, signal.SIGINT, signal.SIGHUP}
            old_mask = signal.pthread_sigmask(signal.SIG_BLOCK, blocked_signals)
            try:
                if not self.vm.terminate_start():
                    log("could not terminate mac_vm.py start process before clone teardown")
                    if error is None:
                        error = NightlyError("mac_vm.py start process did not stop")
                    result = 1
                self.vm.terminate_active()
                if self.vm.session is not None:
                    self.session = self.vm.session
                    try:
                        self.export_runs()
                    except Exception as export_error:
                        log(f"could not export guest run folders before stop: {export_error}")
                        if error is None:
                            error = export_error
                            result = 1
                if self.vm.session_id:
                    try:
                        self.vm.stop()
                        self.stop_ok = True
                    except Exception as stop_error:
                        log(f"failed to stop/delete Tart clone: {stop_error}")
                        if error is None:
                            error = stop_error
                        result = 1
                if self.stop_ok:
                    try:
                        self.publish()
                    except Exception as copy_error:
                        log(f"could not publish guest run folders: {copy_error}")
                        if error is None:
                            error = copy_error
                            result = 1
                try:
                    self.write_status("failed" if result else "completed")
                except Exception as status_error:
                    log(f"could not update nightly status: {status_error}")
                    if error is None:
                        error = status_error
                    result = 1
                finally:
                    if self.owned_lock and (self.stop_ok or not self.vm.session_id):
                        self.settings.status.unlink(missing_ok=True)
                        try:
                            self.settings.lock_dir.rmdir()
                        except OSError:
                            pass
                if self.stop_ok and self.published_ok and self.session is not None:
                    share = Path(str(self.session["share"]))
                    for name in ("input", "control", "results", "quiet"):
                        try:
                            make_writable_and_remove(share / name)
                        except OSError as cleanup_error:
                            log(f"could not remove session {name} data: {cleanup_error}")
            finally:
                signal.pthread_sigmask(signal.SIG_SETMASK, old_mask)
        if error is not None:
            if isinstance(error, KeyboardInterrupt):
                raise error
            if isinstance(error, SystemExit):
                raise error
            if isinstance(error, Exception):
                raise error
            raise NightlyError(str(error))
        if self.run_failures:
            log("required job failures: " + "; ".join(self.run_failures))
        return result


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--branch", action="append", help="branch to run (repeatable; default branches.conf)")
    return parser.parse_args(argv)


def handle_signal(signum: int, _frame: Any) -> None:
    raise SystemExit(128 + signum)


def host_main(argv: Optional[Sequence[str]] = None) -> int:
    args = parse_args(argv)
    settings = Settings.from_environment(dict(os.environ))
    settings.validate()
    # The self-update re-executes this same script path, so it runs before the
    # branch list is read: the new code decides the night's branches.
    self_update_info = maybe_self_update(settings.runner, Path(__file__),
                                         list(argv) if argv is not None else None)
    branches, branch_info = resolve_branches(args.branch, settings.runner)
    log(f"branch list: {branch_info['source']} ({branch_info['note']})")
    log(f"self-update: {self_update_info['status']} ({self_update_info['detail']})")
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, handle_signal)
    return NightlyOrchestrator(settings, branches,
                               branch_list_info=branch_info,
                               self_update_info=self_update_info).run()


def main(argv: Optional[Sequence[str]] = None) -> int:
    guest_result = guest_cli(list(argv if argv is not None else sys.argv[1:]))
    if guest_result is not None:
        return guest_result
    try:
        return host_main(argv)
    except NightlyError as error:
        print(f"nightly-mac: {error}", file=sys.stderr, flush=True)
        return 1
    except subprocess.TimeoutExpired as error:
        print(f"nightly-mac: command timed out: {error.cmd}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
