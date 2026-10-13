#!/usr/bin/env python3
"""Trusted local admission and detective CI for architecture changes.

This program reads product trees as data. It loads check.py and its helper
modules only from the separately pinned enforcement checkout.
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import secrets
import struct
from pathlib import Path
from typing import Any

FORMAT = "f0-architecture-gate-v1"
GATE_SCHEMA = 1
SHA_RE = re.compile(r"\A[0-9a-f]{40}\Z")
DIGEST_RE = re.compile(r"\A[0-9a-f]{64}\Z")
RUST_ANALYZER_SHA256 = "fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49"
RUST_ANALYZER_VERSION = "rust-analyzer 0.3.3081-standalone"
BWRAP_SHA256 = "e318903862396f96de3df57264e0158682b952fd3fb53ac23d876413e7b30f71"
ARCH_DIR = Path("tools/architecture")
POLICY_PATH = ARCH_DIR / "policy.toml"
WORKFLOW_PATH = Path(".github/workflows/ci.yml")
SUBMODULE_PATH = "vendor/fr-client-rust"
SUBMODULE_URL = "https://github.com/acfrazier/FR-client-bothost.git"
SUBMODULE_BRANCH = "multirev-bh-modular"
SUBMODULE_CONFIG = (
    ("submodule.vendor/fr-client-rust.branch", SUBMODULE_BRANCH),
    ("submodule.vendor/fr-client-rust.path", SUBMODULE_PATH),
    ("submodule.vendor/fr-client-rust.url", SUBMODULE_URL),
)


class GateError(Exception):
    """A trusted gate refusal."""


def _fail(message: str) -> None:
    raise GateError(message)


def _canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _digest(value: Any) -> str:
    return hashlib.sha256(_canonical(value)).hexdigest()


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _exact_fields(value: Any, fields: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != fields:
        _fail(f"{label} must contain exactly: {', '.join(sorted(fields))}")
    return value


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as exc:
        _fail(f"cannot read trusted {label} {path}: {exc}")
    if not isinstance(value, dict):
        _fail(f"trusted {label} {path} must be a JSON object")
    return value


def _hex(value: Any, pattern: re.Pattern[str], label: str) -> str:
    length = 40 if pattern is SHA_RE else 64
    if not isinstance(value, str) or pattern.fullmatch(value) is None:
        _fail(f"{label} must be lowercase hexadecimal with length {length}")
    if pattern is DIGEST_RE and value == "0" * 64:
        _fail(f"{label} cannot be an all-zero digest")
    return value


def _nonblank(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        _fail(f"{label} must be a non-empty string")
    return value


def _string_list(value: Any, label: str, *, nonempty: bool = False) -> list[str]:
    if not isinstance(value, list) or (nonempty and not value):
        _fail(f"{label} must be {'a non-empty' if nonempty else 'a'} string array")
    if any(not isinstance(item, str) or not item.strip() for item in value):
        _fail(f"{label} must contain only non-empty strings")
    if len(value) != len(set(value)):
        _fail(f"{label} must not contain duplicates")
    return value


def _path(path: Path, label: str, *, directory: bool = True) -> Path:
    try:
        resolved = path.expanduser().resolve(strict=True)
    except OSError as exc:
        _fail(f"{label} does not exist: {exc}")
    if directory and not resolved.is_dir():
        _fail(f"{label} is not a directory: {resolved}")
    return resolved


def _overlap(left: Path, right: Path) -> bool:
    return left == right or left in right.parents or right in left.parents


def _require_separate(candidate: Path, *trusted: Path) -> None:
    for path in trusted:
        if _overlap(candidate, path):
            _fail(f"candidate tree and trusted store must be disjoint: {candidate} / {path}")


def _git(root: Path, *args: str) -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), *args],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1"},
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", "")
        _fail(f"git {' '.join(args)} failed for {root}: {detail.strip() or exc}")
    return result.stdout.strip()


def verify_checkout(
    root: Path,
    expected_sha: str,
    label: str,
    *,
    clean: bool = True,
    check_submodules: bool = True,
) -> Path:
    root = _path(root, label)
    expected_sha = _hex(expected_sha, SHA_RE, f"{label} commit")
    actual = _git(root, "rev-parse", "--verify", "HEAD")
    if actual != expected_sha:
        _fail(f"{label} HEAD is {actual}, expected exact commit {expected_sha}")
    if clean:
        status_args = ["status", "--porcelain=v1", "--untracked-files=all"]
        status_args.append("--ignore-submodules=none" if check_submodules else "--ignore-submodules=all")
        status = _git(root, *status_args)
        if status:
            _fail(f"{label} checkout is not clean: {status.splitlines()[0]}")
    return root


def _source_files(root: Path) -> list[tuple[Path, str | None]]:
    """Return committed blobs and submodule commit identities, not ignored local files."""
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-tree", "-r", "-z", "--full-tree", "HEAD"],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1"},
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", b"")
        if isinstance(detail, bytes):
            detail = detail.decode("utf-8", errors="replace")
        _fail(f"cannot list committed input tree for {root}: {detail.strip() or exc}")
    files: list[tuple[Path, str | None]] = []
    excluded = {".git", "target", "node_modules", "__pycache__"}
    for entry in result.stdout.split(b"\0"):
        if not entry:
            continue
        try:
            metadata, raw_name = entry.split(b"\t", 1)
            mode, kind, object_id = metadata.decode("ascii").split(" ")
            path = root / Path(os.fsdecode(raw_name))
        except (ValueError, UnicodeError) as exc:
            _fail(f"invalid Git tree entry for {root}: {exc}")
        if any(part in excluded for part in path.relative_to(root).parts):
            continue
        if mode == "160000" and kind == "commit":
            files.append((path, object_id))
            continue
        if mode == "120000":
            _fail(f"committed source tree contains a symlink: {path.relative_to(root).as_posix()}")
        if kind != "blob" or mode not in {"100644", "100755"}:
            _fail(f"unsupported committed input object {mode} {kind}: {path.relative_to(root).as_posix()}")
        if path.is_symlink() or not path.is_file():
            _fail(f"committed input is missing or not a regular file: {path.relative_to(root).as_posix()}")
        files.append((path, None))
    return sorted(files, key=lambda item: item[0].relative_to(root).as_posix())


def input_digest(root: Path) -> str:
    root = _path(root, "input root")
    digest = hashlib.sha256(b"f0-architecture-input-v2\0")
    for path, gitlink_sha in _source_files(root):
        name = os.fsencode(path.relative_to(root).as_posix())
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        if gitlink_sha is not None:
            digest.update(b"gitlink\0")
            digest.update(gitlink_sha.encode("ascii"))
            continue
        digest.update(b"blob\0")
        digest.update(path.stat().st_size.to_bytes(8, "big"))
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    return digest.hexdigest()


def committed_input_digest(repo_root: Path, sha: str) -> str:
    """Recompute an accepted receipt's source digest from its Git tree, not its claim."""
    sha = _hex(sha, SHA_RE, "accepted product commit")
    git_env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1"}
    try:
        listing = subprocess.run(
            ["git", "-C", str(repo_root), "ls-tree", "-r", "-z", "--full-tree", sha],
            check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=git_env,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as exc:
        _fail(f"cannot replay accepted product tree {sha}: {exc}")
    entries: list[tuple[bytes, str, str, str]] = []
    for entry in listing.split(b"\0"):
        if not entry:
            continue
        try:
            metadata, name = entry.split(b"\t", 1)
            mode, kind, oid = metadata.decode("ascii").split(" ")
            parts = Path(os.fsdecode(name)).parts
        except (ValueError, UnicodeError) as exc:
            _fail(f"invalid accepted product tree entry: {exc}")
        if any(part in {".git", "target", "node_modules", "__pycache__"} for part in parts):
            continue
        if (mode, kind) not in {("160000", "commit"), ("100644", "blob"), ("100755", "blob")}:
            _fail(f"unsupported accepted product object {mode} {kind}: {os.fsdecode(name)}")
        entries.append((name, mode, kind, oid))
    digest = hashlib.sha256(b"f0-architecture-input-v2\0")
    try:
        process = subprocess.Popen(
            ["git", "-C", str(repo_root), "cat-file", "--batch"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=git_env,
        )
    except OSError as exc:
        _fail(f"cannot replay accepted source blobs: {exc}")
    assert process.stdin is not None and process.stdout is not None and process.stderr is not None
    try:
        for name, mode, _, oid in entries:
            digest.update(len(name).to_bytes(8, "big"))
            digest.update(name)
            if mode == "160000":
                digest.update(b"gitlink\0")
                digest.update(oid.encode("ascii"))
                continue
            process.stdin.write(oid.encode("ascii") + b"\n")
            process.stdin.flush()
            header = process.stdout.readline().split()
            if len(header) != 3 or header[0] != oid.encode("ascii") or header[1] != b"blob":
                _fail(f"cannot replay accepted source blob {oid}")
            size = int(header[2])
            contents = process.stdout.read(size)
            if len(contents) != size or process.stdout.read(1) != b"\n":
                _fail(f"truncated accepted source blob {oid}")
            digest.update(b"blob\0")
            digest.update(size.to_bytes(8, "big"))
            digest.update(contents)
        process.stdin.close()
        if process.wait(timeout=30) != 0:
            _fail(f"cannot replay accepted source blobs: {process.stderr.read().decode(errors='replace')}")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        process.stdin.close()
        process.stdout.close()
        process.stderr.close()
    return digest.hexdigest()


def _submodule_manifest(root: Path) -> tuple[tuple[str, str], ...]:
    path = root / ".gitmodules"
    if path.is_symlink():
        _fail("source submodule manifest must not be a symlink")
    if not path.exists():
        return ()
    if not path.is_file():
        _fail("source submodule manifest is not a regular file")
    command = ["git", "-C", str(root), "config", "--no-includes", "--file", ".gitmodules"]
    git_env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
    }
    try:
        names_result = subprocess.run(
            [*command, "--name-only", "--list"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=git_env,
        )
    except OSError as exc:
        _fail(f"cannot read source submodule manifest: {exc}")
    if names_result.returncode != 0:
        _fail(f"cannot parse source submodule manifest: {names_result.stderr.strip()}")
    names = names_result.stdout.splitlines()
    expected = dict(SUBMODULE_CONFIG)
    if len(names) != len(expected) or set(names) != set(expected):
        _fail("source submodule manifest must contain only the pinned client module path, URL, and branch")
    values: dict[str, str] = {}
    for name, expected_value in expected.items():
        try:
            result = subprocess.run(
                [*command, "--get-all", name],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=git_env,
            )
        except OSError as exc:
            _fail(f"cannot read source submodule identity: {exc}")
        if result.returncode != 0 or result.stdout.rstrip("\n") != expected_value:
            _fail(f"source submodule identity {name} does not match its protected value")
        values[name] = expected_value
    return tuple(sorted(values.items()))


def _gitlinks(root: Path) -> dict[str, str]:
    return {
        path.relative_to(root).as_posix(): gitlink_sha
        for path, gitlink_sha in _source_files(root)
        if gitlink_sha is not None
    }


def _gitlink_findings(base_root: Path, candidate_root: Path) -> list[str]:
    """A moved client gitlink changes I-wire/T1 custody; it is an approval delta."""
    before = _gitlinks(base_root)
    after = _gitlinks(candidate_root)
    return [
        f"client gitlink moved: {path} {before[path]} -> {after[path]}"
        for path in sorted(before.keys() & after.keys())
        if before[path] != after[path]
    ]


def _submodule_paths(root: Path) -> tuple[str, ...]:
    return tuple(
        path.relative_to(root).as_posix()
        for path, gitlink_sha in _source_files(root)
        if gitlink_sha is not None
    )


def _validate_submodule_layout(base_root: Path, candidate_root: Path) -> None:
    expected_manifest = SUBMODULE_CONFIG
    base_manifest = _submodule_manifest(base_root)
    candidate_manifest = _submodule_manifest(candidate_root)
    base_paths = _submodule_paths(base_root)
    candidate_paths = _submodule_paths(candidate_root)
    if base_manifest not in ((), expected_manifest) or base_paths not in ((), (SUBMODULE_PATH,)):
        _fail("base B has an unsupported source submodule layout")
    if bool(base_manifest) != bool(base_paths):
        _fail("base B source submodule manifest does not match its gitlinks")
    if candidate_manifest != base_manifest or candidate_paths != base_paths:
        _fail("candidate changes the protected source submodule layout")


def _bundle_paths(root: Path) -> list[Path]:
    arch = root / ARCH_DIR
    workflow = root / WORKFLOW_PATH
    if not arch.is_dir() or not workflow.is_file():
        _fail(f"enforcement bundle lacks {ARCH_DIR} or {WORKFLOW_PATH}")
    paths: list[Path] = []
    for current, dirs, names in os.walk(arch, followlinks=False):
        here = Path(current)
        dirs[:] = sorted(name for name in dirs if name not in {"__pycache__", ".git"})
        for name in sorted(names):
            path = here / name
            rel = path.relative_to(root)
            if name.endswith((".pyc", ".pyo")) or rel in {POLICY_PATH, ARCH_DIR / "README.md", ARCH_DIR / "ownership.json", ARCH_DIR / "inventories.json"}:
                continue
            if path.is_symlink() or not path.is_file():
                _fail(f"enforcement bundle contains a non-regular file: {rel.as_posix()}")
            paths.append(path)
    if workflow.is_symlink() or not workflow.is_file():
        _fail(f"enforcement bundle workflow is not a regular file: {WORKFLOW_PATH}")
    paths.append(workflow)
    return sorted(paths, key=lambda path: path.relative_to(root).as_posix())


def bundle_digest(root: Path) -> str:
    root = _path(root, "enforcement root")
    digest = hashlib.sha256(b"f0-architecture-enforcement-bundle-v1\0")
    for path in _bundle_paths(root):
        name = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        digest.update(path.stat().st_size.to_bytes(8, "big"))
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    return digest.hexdigest()


def _bundle_difference(enforcement_root: Path, candidate_root: Path) -> list[str]:
    trusted_paths = _bundle_paths(enforcement_root)
    candidate_paths = _bundle_paths(candidate_root)
    trusted = {path.relative_to(enforcement_root).as_posix(): path for path in trusted_paths}
    candidate = {path.relative_to(candidate_root).as_posix(): path for path in candidate_paths}
    differences: list[str] = []
    for name in sorted(trusted.keys() | candidate.keys()):
        if name not in trusted:
            differences.append(f"added {name}")
        elif name not in candidate:
            differences.append(f"removed {name}")
        elif _file_sha256(trusted[name]) != _file_sha256(candidate[name]):
            differences.append(f"changed {name}")
    return differences


def _runtime_parts(runtime_pin: str) -> tuple[str, str]:
    pieces = runtime_pin.split(";")
    if len(pieces) != 2 or not pieces[0].startswith("python=") or not pieces[1].startswith("rust-analyzer-sha256="):
        _fail("runtime pin must be python=<exact-version>;rust-analyzer-sha256=<64 lowercase hex>")
    python_version = pieces[0][len("python="):]
    parser_digest = pieces[1][len("rust-analyzer-sha256="):]
    if re.fullmatch(r"\d+\.\d+\.\d+", python_version) is None or DIGEST_RE.fullmatch(parser_digest) is None:
        _fail("runtime pin has an invalid Python version or parser digest")
    return python_version, parser_digest


def verify_runtime(runtime_pin: str) -> None:
    python_pin, parser_pin = _runtime_parts(runtime_pin)
    actual_python = ".".join(str(part) for part in sys.version_info[:3])
    if actual_python != python_pin:
        _fail(f"Python runtime is {actual_python}, expected exact pin {python_pin}")
    if parser_pin != RUST_ANALYZER_SHA256:
        _fail("runtime pin selects a Rust parser identity outside the protected gate bundle")
    parser = shutil.which("rust-analyzer")
    if parser is None:
        _fail("pinned rust-analyzer executable is unavailable")
    try:
        parser_path = Path(parser).resolve(strict=True)
        actual_parser = _file_sha256(parser_path)
    except OSError as exc:
        _fail(f"cannot read pinned rust-analyzer executable: {exc}")
    if actual_parser != parser_pin:
        _fail(f"rust-analyzer SHA-256 is {actual_parser}, expected {parser_pin}")
    try:
        version = subprocess.run(
            [str(parser_path), "--version"],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=parser_path.parent,
            env={"PATH": os.pathsep.join((str(parser_path.parent), "/usr/bin", "/bin"))},
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", "")
        _fail(f"cannot verify pinned rust-analyzer version: {detail.strip() or exc}")
    if version.stdout.strip() != RUST_ANALYZER_VERSION:
        _fail(f"rust-analyzer version is {version.stdout.strip()!r}, expected {RUST_ANALYZER_VERSION!r}")


def _schema_versions(root: Path) -> dict[str, int]:
    try:
        policy = tomllib.loads((root / POLICY_PATH).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
        _fail(f"cannot read policy schema version: {exc}")
    policy_version = policy.get("schema_version")
    if not isinstance(policy_version, int) or isinstance(policy_version, bool):
        _fail("policy schema_version must be an integer")
    result = {"trusted_gate": GATE_SCHEMA, "policy": policy_version}
    for path in sorted((root / ARCH_DIR).glob("*.json")):
        try:
            value = _read_json(path, f"schema file {path.name}")
        except GateError:
            raise
        version = value.get("schema_version", value.get("version"))
        if isinstance(version, int) and not isinstance(version, bool):
            result[path.relative_to(root).as_posix()] = version
    return result


_CHECKER_CODE = r'''import json, sys
from pathlib import Path
arch = Path(sys.argv[1]).resolve(strict=True)
operation = sys.argv[2]
args = json.loads(sys.stdin.read())
sys.path.insert(0, str(arch))
try:
    import check
    if operation == "tree":
        result = check.check_tree(Path(args["root"]))
    elif operation == "transition":
        result = check.check_transition(Path(args["base"]), Path(args["candidate"]))
    else:
        raise ValueError("unknown checker operation")
    if not isinstance(result, list) or any(not isinstance(item, str) for item in result):
        raise TypeError("checker API must return list[str]")
    sys.stdout.write(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
except BaseException as exc:
    sys.stderr.write(f"trusted checker failed: {type(exc).__name__}: {exc}\n")
    raise SystemExit(3)
'''


def checker_findings(enforcement_root: Path, operation: str, **arguments: Path) -> list[str]:
    arch = _path(enforcement_root / ARCH_DIR, "trusted checker directory")
    payload = {key: str(value.resolve(strict=True)) for key, value in arguments.items()}
    parser = shutil.which("rust-analyzer")
    if parser is None:
        _fail("pinned rust-analyzer executable is unavailable")
    parser_dir = str(Path(parser).resolve(strict=True).parent)
    path = os.pathsep.join(dict.fromkeys((parser_dir, "/usr/bin", "/bin")))
    env = {"PATH": path, "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1"}
    try:
        result = subprocess.run(
            [sys.executable, "-B", "-I", "-S", "-c", _CHECKER_CODE, str(arch), operation],
            input=json.dumps(payload),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=arch,
            env=env,
        )
    except OSError as exc:
        _fail(f"cannot start pinned trusted checker: {exc}")
    if result.returncode != 0:
        _fail(result.stderr.strip() or f"trusted checker exited {result.returncode}")
    try:
        findings = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        _fail(f"trusted checker returned invalid output: {exc}")
    if not isinstance(findings, list) or any(not isinstance(item, str) for item in findings):
        _fail("trusted checker API did not return list[str]")
    return findings


def _policy(root: Path) -> dict[str, Any]:
    try:
        value = tomllib.loads((root / POLICY_PATH).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
        _fail(f"cannot read candidate policy data: {exc}")
    if not isinstance(value, dict):
        _fail("policy data is not a TOML table")
    return value


def _normal(value: Any) -> bytes:
    return _canonical(value)


def _strict_policy_removal(base_root: Path, candidate_root: Path) -> bool:
    """Return true only for a semantic strict subset with unchanged retained rows."""
    before = _policy(base_root)
    after = _policy(candidate_root)
    for key in ("allow", "migration"):
        before.setdefault(key, [])
        after.setdefault(key, [])
    if set(before) != set(after):
        return False
    if before.get("schema_version") != after.get("schema_version"):
        return False
    changed = False
    before_ws = before.get("workspace")
    after_ws = after.get("workspace")
    if not isinstance(before_ws, dict) or not isinstance(after_ws, dict) or set(before_ws) != set(after_ws):
        return False
    for key in before_ws:
        old = before_ws[key]
        new = after_ws[key]
        if not isinstance(old, list) or not isinstance(new, list):
            return False
        if not set(new) <= set(old):
            return False
        changed |= set(new) != set(old)
    for key in ("allow", "migration"):
        old_rows = before.get(key, [])
        new_rows = after.get(key, [])
        if not isinstance(old_rows, list) or not isinstance(new_rows, list):
            return False
        old_by_id: dict[str, dict[str, Any]] = {}
        if key == "allow":
            old_by_id = {f"{row.get('from')}->{row.get('to')}": row for row in old_rows if isinstance(row, dict)}
            new_by_id = {f"{row.get('from')}->{row.get('to')}": row for row in new_rows if isinstance(row, dict)}
            if len(old_by_id) != len(old_rows) or len(new_by_id) != len(new_rows):
                return False
            for identity, new_row in new_by_id.items():
                old_row = old_by_id.get(identity)
                if old_row is None:
                    return False
                old_fixed = {k: v for k, v in old_row.items() if k != "kinds"}
                new_fixed = {k: v for k, v in new_row.items() if k != "kinds"}
                if old_fixed != new_fixed:
                    return False
                old_kinds, new_kinds = old_row.get("kinds"), new_row.get("kinds")
                if not isinstance(old_kinds, list) or not isinstance(new_kinds, list) or not set(new_kinds) <= set(old_kinds):
                    return False
                changed |= set(new_kinds) != set(old_kinds)
            changed |= set(new_by_id) != set(old_by_id)
        else:
            old_by_id = {str(row.get("id")): row for row in old_rows if isinstance(row, dict)}
            new_by_id = {str(row.get("id")): row for row in new_rows if isinstance(row, dict)}
            if len(old_by_id) != len(old_rows) or len(new_by_id) != len(new_rows):
                return False
            if any(identity not in old_by_id or old_by_id[identity] != row for identity, row in new_by_id.items()):
                return False
            changed |= set(new_by_id) != set(old_by_id)
    for key in before.keys() - {"workspace", "allow", "migration"}:
        if _normal(before[key]) != _normal(after[key]):
            return False
    return changed


def _validate_target_ref(value: Any) -> str:
    target = _nonblank(value, "target_ref")
    if not target.startswith("refs/heads/") or ".." in target or target.endswith("/"):
        _fail("target_ref must be a valid refs/heads/... name")
    return target


def _validate_pin(pin: dict[str, Any], label: str) -> dict[str, Any]:
    expected = {"accepted_sha", "contract_epoch", "enforcement_sha", "bundle_digest", "runtime_pin", "schema_versions", "input_digest", "orchestrator_commit", "reason", "operator_inline_approval", "owner_acknowledgements"}
    _exact_fields(pin, expected, label)
    accepted = _hex(pin["accepted_sha"], SHA_RE, f"{label}.accepted_sha")
    epoch = pin["contract_epoch"]
    if not isinstance(epoch, int) or isinstance(epoch, bool) or epoch < 0:
        _fail(f"{label}.contract_epoch must be a non-negative integer")
    enforcement = _hex(pin["enforcement_sha"], SHA_RE, f"{label}.enforcement_sha")
    digest = _hex(pin["bundle_digest"], DIGEST_RE, f"{label}.bundle_digest")
    runtime = _nonblank(pin["runtime_pin"], f"{label}.runtime_pin")
    _runtime_parts(runtime)
    schemas = pin["schema_versions"]
    if not isinstance(schemas, dict) or any(not isinstance(k, str) or not isinstance(v, int) or isinstance(v, bool) for k, v in schemas.items()):
        _fail(f"{label}.schema_versions must map names to integer versions")
    input_hash = _hex(pin["input_digest"], DIGEST_RE, f"{label}.input_digest")
    orchestrator = _hex(pin["orchestrator_commit"], SHA_RE, f"{label}.orchestrator_commit")
    if orchestrator != accepted:
        _fail(f"{label}.orchestrator_commit must equal its accepted bootstrap SHA")
    _nonblank(pin["reason"], f"{label}.reason")
    _nonblank(pin["operator_inline_approval"], f"{label}.operator_inline_approval")
    _string_list(pin["owner_acknowledgements"], f"{label}.owner_acknowledgements", nonempty=True)
    return {"head_sha": accepted, "contract_epoch": epoch, "enforcement_sha": enforcement, "bundle_digest": digest, "runtime_pin": runtime, "schema_versions": schemas, "input_digest": input_hash}


def _receipt_fields() -> set[str]:
    return {"schema_version", "sequence", "target_ref", "head_sha", "base_sha", "contract_epoch", "enforcement_sha", "bundle_digest", "runtime_pin", "activated_enforcement_sha", "activated_bundle_digest", "activated_runtime_pin", "schema_versions", "input_digest", "orchestrator_commit", "transition_kind", "delta", "approval_digest", "pending_digest", "previous_digest", "receipt_digest"}


def _read_record_store(record_root: Path, target_ref: str) -> tuple[dict[str, Any], list[dict[str, Any]], dict[str, Any]]:
    state_path = record_root / "gate.json"
    state = _read_json(state_path, "gate record")
    _exact_fields(state, {"format", "schema_version", "target_ref", "bootstrap"}, "gate record")
    if state["format"] != FORMAT or not isinstance(state["schema_version"], int) or isinstance(state["schema_version"], bool) or state["schema_version"] != GATE_SCHEMA:
        _fail("unsupported trusted gate record format/schema")
    stored_target = _validate_target_ref(state["target_ref"])
    if stored_target != target_ref:
        _fail(f"records are for {stored_target}, not {target_ref}")
    bootstrap = _validate_pin(state["bootstrap"], "bootstrap")
    if bootstrap["head_sha"] != state["bootstrap"]["accepted_sha"]:
        _fail("bootstrap accepted SHA mismatch")
    raw_lines = (record_root / "receipts.jsonl").read_text(encoding="utf-8").splitlines()
    receipts: list[dict[str, Any]] = []
    current = dict(bootstrap)
    previous = _digest(state["bootstrap"])
    seen_heads = {current["head_sha"]}
    for index, line in enumerate(raw_lines, start=1):
        if not line:
            _fail(f"receipt chain has an empty line at {index}")
        try:
            receipt = json.loads(line, object_pairs_hook=_unique_object)
        except (json.JSONDecodeError, ValueError) as exc:
            _fail(f"invalid receipt line {index}: {exc}")
        _exact_fields(receipt, _receipt_fields(), f"receipt {index}")
        if not isinstance(receipt["schema_version"], int) or isinstance(receipt["schema_version"], bool) or receipt["schema_version"] != GATE_SCHEMA:
            _fail(f"receipt {index} has an invalid schema or sequence")
        if not isinstance(receipt["sequence"], int) or isinstance(receipt["sequence"], bool) or receipt["sequence"] != index:
            _fail(f"receipt {index} has an invalid schema or sequence")
        if receipt["target_ref"] != target_ref:
            _fail(f"receipt {index} targets another ref")
        base = _hex(receipt["base_sha"], SHA_RE, f"receipt {index}.base_sha")
        head = _hex(receipt["head_sha"], SHA_RE, f"receipt {index}.head_sha")
        if base != current["head_sha"] or head in seen_heads:
            _fail(f"receipt {index} does not extend the accepted target chain")
        if _hex(receipt["orchestrator_commit"], SHA_RE, f"receipt {index}.orchestrator_commit") != head:
            _fail(f"receipt {index} was not recorded as an orchestrator-created commit")
        if receipt["previous_digest"] != previous:
            _fail(f"receipt {index} breaks the receipt hash chain")
        claimed_digest = _hex(receipt["receipt_digest"], DIGEST_RE, f"receipt {index}.receipt_digest")
        body = {key: value for key, value in receipt.items() if key != "receipt_digest"}
        if _digest(body) != claimed_digest:
            _fail(f"receipt {index} digest does not match its contents")
        epoch = receipt["contract_epoch"]
        if not isinstance(epoch, int) or isinstance(epoch, bool):
            _fail(f"receipt {index} contract epoch is invalid")
        kind = receipt["transition_kind"]
        if kind not in (None, "policy", "enforcement"):
            _fail(f"receipt {index} transition kind is invalid")
        delta = _string_list(receipt["delta"], f"receipt {index}.delta")
        if delta != sorted(delta):
            _fail(f"receipt {index}.delta must be sorted")
        approval_digest = receipt["approval_digest"]
        pending_digest = receipt["pending_digest"]
        if approval_digest is not None:
            _hex(approval_digest, DIGEST_RE, f"receipt {index}.approval_digest")
        if pending_digest is not None:
            _hex(pending_digest, DIGEST_RE, f"receipt {index}.pending_digest")
        checked_enforcement = _hex(receipt["enforcement_sha"], SHA_RE, f"receipt {index}.enforcement_sha")
        checked_bundle = _hex(receipt["bundle_digest"], DIGEST_RE, f"receipt {index}.bundle_digest")
        checked_runtime = _nonblank(receipt["runtime_pin"], f"receipt {index}.runtime_pin")
        _runtime_parts(checked_runtime)
        if checked_enforcement != current["enforcement_sha"] or checked_bundle != current["bundle_digest"] or checked_runtime != current["runtime_pin"]:
            _fail(f"receipt {index} was not checked under the active enforcement identity")
        activated_enforcement = _hex(receipt["activated_enforcement_sha"], SHA_RE, f"receipt {index}.activated_enforcement_sha")
        activated_bundle = _hex(receipt["activated_bundle_digest"], DIGEST_RE, f"receipt {index}.activated_bundle_digest")
        activated_runtime = _nonblank(receipt["activated_runtime_pin"], f"receipt {index}.activated_runtime_pin")
        _runtime_parts(activated_runtime)
        schemas = receipt["schema_versions"]
        if not isinstance(schemas, dict) or any(not isinstance(k, str) or not isinstance(v, int) or isinstance(v, bool) for k, v in schemas.items()):
            _fail(f"receipt {index}.schema_versions is invalid")
        _hex(receipt["input_digest"], DIGEST_RE, f"receipt {index}.input_digest")
        if kind is None:
            if epoch != current["contract_epoch"] or activated_enforcement != current["enforcement_sha"] or activated_bundle != current["bundle_digest"] or activated_runtime != current["runtime_pin"] or schemas != current["schema_versions"] or approval_digest is not None or pending_digest is not None or delta:
                _fail(f"ordinary receipt {index} changes a protected identity or contract epoch")
        else:
            if epoch != current["contract_epoch"] + 1 or approval_digest is None:
                _fail(f"approved transition receipt {index} must advance the epoch exactly once")
            if kind == "policy" and (activated_enforcement != current["enforcement_sha"] or activated_bundle != current["bundle_digest"] or activated_runtime != current["runtime_pin"]):
                _fail(f"policy receipt {index} changes enforcement identity")
            if kind == "enforcement" and activated_enforcement == current["enforcement_sha"] and activated_bundle == current["bundle_digest"] and activated_runtime == current["runtime_pin"]:
                _fail(f"enforcement receipt {index} does not change any protected identity")
            if kind == "enforcement" and pending_digest is None:
                _fail(f"enforcement receipt {index} has no pending-transition digest")
            if pending_digest is not None and kind != "enforcement":
                _fail(f"only enforcement receipt {index} may refer to a pending transition")
        current = {"head_sha": head, "contract_epoch": epoch, "enforcement_sha": activated_enforcement, "bundle_digest": activated_bundle, "runtime_pin": activated_runtime, "schema_versions": schemas, "input_digest": receipt["input_digest"], "receipt_digest": claimed_digest}
        previous = claimed_digest
        seen_heads.add(head)
        receipts.append(receipt)
    pending_path = record_root / "pending.json"
    pending = _read_json(pending_path, "pending transition") if pending_path.exists() else {}
    if pending_path.exists():
        _validate_pending(pending)
        if pending["target_ref"] != target_ref:
            _fail("pending transition targets another protected ref")
    return current, receipts, pending


def _validate_pending(pending: dict[str, Any]) -> None:
    fields = {"schema_version", "target_ref", "base_sha", "head_sha", "contract_epoch", "next_contract_epoch", "active_enforcement_sha", "active_bundle_digest", "proposed_enforcement_sha", "proposed_bundle_digest", "runtime_pin", "proposed_runtime_pin", "input_digest", "approval_digest", "delta", "reason", "owner_acknowledgements", "orchestrator_commit", "qualification_digest"}
    _exact_fields(pending, fields, "pending transition")
    if not isinstance(pending["schema_version"], int) or isinstance(pending["schema_version"], bool) or pending["schema_version"] != GATE_SCHEMA:
        _fail("pending transition has an invalid schema or epoch")
    if not isinstance(pending["contract_epoch"], int) or isinstance(pending["contract_epoch"], bool):
        _fail("pending transition has an invalid schema or epoch")
    if not isinstance(pending["next_contract_epoch"], int) or isinstance(pending["next_contract_epoch"], bool) or pending["next_contract_epoch"] != pending["contract_epoch"] + 1:
        _fail("pending transition has an invalid schema or epoch")
    for name in ("base_sha", "head_sha", "orchestrator_commit", "active_enforcement_sha", "proposed_enforcement_sha"):
        _hex(pending[name], SHA_RE, f"pending.{name}")
    for name in ("active_bundle_digest", "proposed_bundle_digest", "input_digest", "approval_digest", "qualification_digest"):
        _hex(pending[name], DIGEST_RE, f"pending.{name}")
    if pending["head_sha"] != pending["orchestrator_commit"]:
        _fail("pending orchestrator commit must equal H")
    _runtime_parts(_nonblank(pending["runtime_pin"], "pending.runtime_pin"))
    _runtime_parts(_nonblank(pending["proposed_runtime_pin"], "pending.proposed_runtime_pin"))
    _string_list(pending["delta"], "pending.delta")
    _string_list(pending["owner_acknowledgements"], "pending.owner_acknowledgements", nonempty=True)
    _validate_target_ref(pending["target_ref"])
    _nonblank(pending["reason"], "pending.reason")


def _read_approval(record_root: Path, head_sha: str) -> tuple[dict[str, Any] | None, str | None]:
    path = record_root / "approvals" / f"{head_sha}.json"
    if not path.exists():
        return None, None
    approval = _read_json(path, "approval")
    expected = {"schema_version", "kind", "target_ref", "base_sha", "head_sha", "contract_epoch", "next_contract_epoch", "active_enforcement_sha", "active_bundle_digest", "proposed_enforcement_sha", "proposed_bundle_digest", "runtime_pin", "proposed_runtime_pin", "input_digest", "delta", "reason", "owner_acknowledgements", "orchestrator_commit", "operator_inline_approval", "source_message"}
    _exact_fields(approval, expected, "approval")
    if not isinstance(approval["schema_version"], int) or isinstance(approval["schema_version"], bool) or approval["schema_version"] != GATE_SCHEMA or approval["kind"] not in ("policy", "enforcement"):
        _fail("approval schema or kind is invalid")
    _validate_target_ref(approval["target_ref"])
    for name in ("base_sha", "head_sha", "orchestrator_commit", "active_enforcement_sha", "proposed_enforcement_sha"):
        _hex(approval[name], SHA_RE, f"approval.{name}")
    for name in ("active_bundle_digest", "proposed_bundle_digest", "input_digest"):
        _hex(approval[name], DIGEST_RE, f"approval.{name}")
    _runtime_parts(_nonblank(approval["runtime_pin"], "approval.runtime_pin"))
    _runtime_parts(_nonblank(approval["proposed_runtime_pin"], "approval.proposed_runtime_pin"))
    delta = _string_list(approval["delta"], "approval.delta")
    if delta != sorted(delta):
        _fail("approval.delta must be sorted")
    if not isinstance(approval["contract_epoch"], int) or isinstance(approval["contract_epoch"], bool) or not isinstance(approval["next_contract_epoch"], int) or isinstance(approval["next_contract_epoch"], bool) or approval["next_contract_epoch"] != approval["contract_epoch"] + 1:
        _fail("approval must advance the contract epoch exactly once")
    _nonblank(approval["reason"], "approval.reason")
    _nonblank(approval["operator_inline_approval"], "approval.operator_inline_approval")
    _nonblank(approval["source_message"], "approval.source_message")
    approval_digest = _digest(approval)
    return approval, approval_digest


def _validate_approval(approval: dict[str, Any] | None, *, kind: str, target_ref: str, base_sha: str, head_sha: str, current: dict[str, Any], candidate_digest: str, findings: list[str]) -> str:
    if approval is None:
        _fail(f"{kind} transition requires an external approval record for exact H")
    expected = {
        "kind": kind,
        "target_ref": target_ref,
        "base_sha": base_sha,
        "head_sha": head_sha,
        "contract_epoch": current["contract_epoch"],
        "next_contract_epoch": current["contract_epoch"] + 1,
        "active_enforcement_sha": current["enforcement_sha"],
        "active_bundle_digest": current["bundle_digest"],
        "runtime_pin": current["runtime_pin"],
        "input_digest": candidate_digest,
        "delta": findings,
        "orchestrator_commit": head_sha,
    }
    for key, value in expected.items():
        if approval.get(key) != value:
            _fail(f"approval.{key} does not match the exact admission identity")
    return _digest(approval)


def _check_tree_clean(enforcement_root: Path, root: Path, label: str) -> None:
    findings = checker_findings(enforcement_root, "tree", root=root)
    if findings:
        _fail(f"{label} fails trusted check_tree: " + " | ".join(findings))


def _evaluate_transition(enforcement_root: Path, base_root: Path, candidate_root: Path) -> list[str]:
    _check_tree_clean(enforcement_root, base_root, "B")
    _check_tree_clean(enforcement_root, candidate_root, "H")
    findings = checker_findings(enforcement_root, "transition", base=base_root, candidate=candidate_root)
    findings += _gitlink_findings(base_root, candidate_root)
    if len(findings) != len(set(findings)):
        _fail("trusted check_transition returned duplicate findings")
    return sorted(findings)


def _load_current_inputs(args: argparse.Namespace, *, local: bool) -> tuple[Path, Path, Path, Path, dict[str, Any], list[dict[str, Any]], dict[str, Any], dict[str, Any]]:
    target_ref = _validate_target_ref(args.target_ref)
    records_root = _path(args.records_root, "records root")
    enforcement_root = _path(args.enforcement_root, "enforcement root")
    candidate_root = _path(args.candidate_root, "candidate root")
    base_root = _path(args.base_root, "base root")
    repo_root = _path(args.repo_root, "repository root")
    _require_separate(candidate_root, records_root, enforcement_root, base_root)
    if _overlap(records_root, enforcement_root) or _overlap(records_root, base_root) or _overlap(enforcement_root, base_root):
        _fail("candidate, base, enforcement, and records roots must be separate checkouts")
    record_sha = _hex(args.records_commit, SHA_RE, "records commit")
    if args.records_ref_sha != record_sha:
        _fail("protected records ref tip does not match the exact records commit")
    verify_checkout(records_root, record_sha, "records checkout")
    current, receipts, pending = _read_record_store(records_root, target_ref)
    for index, receipt in enumerate(receipts, start=1):
        if receipt["input_digest"] != committed_input_digest(repo_root, receipt["head_sha"]):
            _fail(f"receipt {index}.input_digest does not match accepted product bytes")
    external_e = _hex(args.enforcement_sha, SHA_RE, "external enforcement SHA")
    external_digest = _hex(args.bundle_digest, DIGEST_RE, "external bundle digest")
    external_runtime = _nonblank(args.runtime_pin, "external runtime pin")
    if current["enforcement_sha"] != external_e or current["bundle_digest"] != external_digest or current["runtime_pin"] != external_runtime:
        _fail("external pin does not match the accepted record-chain enforcement SHA/digest/runtime")
    verify_runtime(external_runtime)
    verify_checkout(enforcement_root, external_e, "enforcement checkout", check_submodules=False)
    actual_bundle = bundle_digest(enforcement_root)
    if actual_bundle != external_digest:
        _fail(f"enforcement bundle digest is {actual_bundle}, expected {external_digest}")
    _hex(args.enforcement_ref_sha, SHA_RE, "enforcement ref tip")
    if local:
        current_target = _git(repo_root, "rev-parse", "--verify", target_ref)
        if current_target != current["head_sha"]:
            _fail(f"local target {target_ref} is {current_target}, but accepted B is {current['head_sha']}")
    return records_root, enforcement_root, base_root, candidate_root, current, receipts, pending, {"repo_root": repo_root, "target_ref": target_ref, "record_sha": record_sha}


def _verify_roots(base_root: Path, candidate_root: Path, base_sha: str, head_sha: str, current: dict[str, Any]) -> str:
    verify_checkout(base_root, base_sha, "base B")
    verify_checkout(candidate_root, head_sha, "candidate H")
    _validate_submodule_layout(base_root, candidate_root)
    if base_sha != current["head_sha"]:
        _fail(f"candidate base B={base_sha} is not the accepted target {current['head_sha']}")
    base_digest = input_digest(base_root)
    if base_digest != current["input_digest"]:
        _fail("B source data does not match its accepted input digest")
    return input_digest(candidate_root)


def _approval_state(record_root: Path, base_root: Path, candidate_root: Path, head_sha: str, base_sha: str, target_ref: str, current: dict[str, Any], candidate_digest: str, findings: list[str]) -> tuple[dict[str, Any] | None, str | None, str | None]:
    approval, approval_digest = _read_approval(record_root, head_sha)
    policy_changed = (base_root / POLICY_PATH).read_bytes() != (candidate_root / POLICY_PATH).read_bytes()
    strict_removal = policy_changed and _strict_policy_removal(base_root, candidate_root)
    if strict_removal:
        if findings:
            _fail("strict-subset policy removal has transition findings and cannot use the removal exception")
        if approval is not None:
            _fail("strict-subset policy removal must not reuse an approval record")
        return None, None, None
    if approval is None:
        if findings:
            _fail("transition findings require exact external approval: " + " | ".join(findings))
        if policy_changed:
            _fail("policy change is not an E-verified strict-subset removal and has no approval")
        return None, None, None
    if findings != approval.get("delta"):
        _fail("approval.delta must equal the exact sorted unique check_transition findings")
    kind = approval.get("kind")
    if kind not in ("policy", "enforcement"):
        _fail("approval.kind is invalid")
    digest = _validate_approval(approval, kind=kind, target_ref=target_ref, base_sha=base_sha, head_sha=head_sha, current=current, candidate_digest=candidate_digest, findings=findings)
    return approval, digest, kind


def _require_candidate_surface(enforcement_root: Path, candidate_root: Path, proposed_root: Path | None) -> list[str]:
    differences = _bundle_difference(enforcement_root, candidate_root)
    if not differences:
        if proposed_root is not None:
            _fail("proposed enforcement update has no executable-surface change")
        return []
    if proposed_root is None:
        _fail("candidate changes protected enforcement bytes without a prepared E' transition: " + ", ".join(differences))
    if _bundle_difference(proposed_root, candidate_root):
        _fail("candidate protected bytes do not exactly match E': " + ", ".join(_bundle_difference(proposed_root, candidate_root)))
    return differences


def _check_approval_proposal(approval: dict[str, Any], proposed_sha: str, proposed_digest: str, proposed_runtime: str) -> None:
    expected = {
        "proposed_enforcement_sha": proposed_sha,
        "proposed_bundle_digest": proposed_digest,
        "proposed_runtime_pin": proposed_runtime,
    }
    for key, value in expected.items():
        if approval.get(key) != value:
            _fail(f"approval.{key} does not match the exact proposed enforcement identity")


def _run_child(root: Path, *, evidence_root: Path, checker_root: Path | None = None) -> None:
    source = _path(root, "trusted qualification source")
    scratch = _path(evidence_root, "qualification evidence root")
    _require_separate(source, scratch)
    with tempfile.TemporaryDirectory(dir=scratch) as temp_name:
        copied = Path(temp_name) / "bundle"
        copied.mkdir()
        shutil.copytree(source / ARCH_DIR, copied / ARCH_DIR, symlinks=False)
        (copied / WORKFLOW_PATH).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / WORKFLOW_PATH, copied / WORKFLOW_PATH)
        parser = shutil.which("rust-analyzer")
        if parser is None:
            _fail("pinned rust-analyzer executable is unavailable during qualification")
        parser_dir = str(Path(parser).resolve(strict=True).parent)
        bwrap = shutil.which("bwrap")
        if bwrap is None or _file_sha256(Path(bwrap).resolve(strict=True)) != BWRAP_SHA256:
            _fail("pinned bubblewrap executable is unavailable during qualification")
        sandbox_dir = str(Path(bwrap).resolve(strict=True).parent)
        system_path = os.pathsep.join(dict.fromkeys((parser_dir, sandbox_dir, "/usr/bin", "/bin")))
        env = {"PATH": system_path, "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1", "TMPDIR": str(scratch), "F0_ARCH_TEST_ROOT": str(scratch)}
        if checker_root is not None:
            env["F0_ARCH_TEST_CHECKER_ROOT"] = str(_path(checker_root, "qualification checker root"))
        commands = [
            [sys.executable, "-B", "-I", "-S", str(copied / ARCH_DIR / "check.py"), "--self-test"],
            [sys.executable, "-B", "-I", "-S", "-m", "unittest", "discover", "-s", str(copied / ARCH_DIR), "-p", "test_*.py"],
        ]
        for command in commands:
            try:
                result = subprocess.run(command, cwd=copied, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            except OSError as exc:
                _fail(f"cannot run protected qualification command: {exc}")
            if result.returncode != 0:
                _fail("protected fixture qualification failed:\n" + result.stdout)


def _qualification_digest(active_root: Path, proposed_root: Path, base_sha: str, head_sha: str, candidate_digest: str, approval_digest: str, runtime_pin: str) -> str:
    return _digest({
        "active_bundle": bundle_digest(active_root),
        "proposed_bundle": bundle_digest(proposed_root),
        "base_sha": base_sha,
        "head_sha": head_sha,
        "candidate_input_digest": candidate_digest,
        "approval_digest": approval_digest,
        "runtime_pin": runtime_pin,
        "fixture_corpus": "check.py --self-test + tools/architecture/test_*.py",
        "admission_probes": [label for label, _, _ in ADMISSION_PROBES],
        "sequential_probe": f"{SEQUENTIAL_CONTROLS} random-comment controls then every bad candidate in fresh stages carrying only the checked receipt chain",
        "probe_isolation": "read-only host inputs, private tmpfs staging, records-only copyback, new session/keyring, default-deny x86-64 seccomp",
        "prepare_probe": "one CLI prepare-enforcement against a random-shape neutered checker; verdict exit 1 and no pending.json",
    })


SEQUENTIAL_CONTROLS = 3


def _record_files(root: Path) -> dict[str, bytes]:
    files: dict[str, bytes] = {}
    for directory, dirs, names in os.walk(root, followlinks=False):
        here = Path(directory)
        dirs[:] = sorted(name for name in dirs if name != ".git")
        for name in dirs:
            path = here / name
            if path.is_symlink():
                _fail(f"probe records contain a symlinked directory: {path}")
            files[path.relative_to(root).as_posix() + "/"] = b""
        for name in names:
            path = here / name
            if not path.is_file() or path.is_symlink():
                _fail(f"probe records contain a non-regular path: {path}")
            files[path.relative_to(root).as_posix()] = path.read_bytes()
    return files


def _check_probe_records(records: Path, before: dict[str, bytes], admitted: bool) -> None:
    after = _record_files(records)
    if not admitted:
        if after != before:
            changed = sorted(name for name in before.keys() | after.keys() if before.get(name) != after.get(name))
            _fail(f"qualification probe modified trusted records on refusal: {', '.join(changed)}")
        return
    key = "receipts.jsonl"
    if set(after) != set(before) or any(after[name] != value for name, value in before.items() if name != key):
        _fail("qualification control changed records outside its one receipt")
    previous = before[key]
    if not after[key].startswith(previous) or len(after[key].splitlines()) != len(previous.splitlines()) + 1:
        _fail("qualification control did not append exactly one receipt")


def _check_probe_receipt(records: Path, target_ref: str, base_sha: str, head_sha: str, digest: str, enforcement_sha: str) -> None:
    current, receipts, pending = _read_record_store(records, target_ref)
    receipt = receipts[-1]
    if (pending or current["head_sha"] != head_sha or current["enforcement_sha"] != enforcement_sha
            or receipt["base_sha"] != base_sha or receipt["transition_kind"] is not None
            or receipt["input_digest"] != digest):
        _fail("qualification control did not bind its exact accepted source and enforcement")


# `main()` exits 1 on every GateError denial. A probe killed by a signal or
# seccomp, or exiting with any other status, has not refused anything.
_PROBE_REFUSAL_EXIT = 1


def _seed_probe_records(records_top: Path, records_sha: str, records_subpath: Path, work: Path,
                        activation_receipt: dict[str, Any]) -> tuple[Path, Path]:
    """Private records clone whose receipt chain ends with the simulated E' activation (uncommitted)."""
    records_clone = _clone_at(records_top, work / secrets.token_hex(16), records_sha, depth=1)
    records = records_clone / records_subpath
    receipts_path = records / "receipts.jsonl"
    old = receipts_path.read_bytes()
    if old and not old.endswith(b"\n"):
        _fail("receipt file must end with a newline")
    receipts_path.write_bytes(old + _canonical(activation_receipt) + b"\n")
    return records_clone, records


def _probe_admit_command(
    enforcement: Path, records: Path, record_sha: str, proposed_sha: str,
    proposed_digest: str, runtime_pin: str, target_ref: str, base: Path,
    candidate: Path, head_sha: str,
) -> list[str]:
    return [
        sys.executable, str(enforcement / ARCH_DIR / "trusted_gate.py"), "admit",
        "--records-root", str(records), "--records-commit", record_sha,
        "--records-ref-sha", record_sha, "--enforcement-root", str(enforcement),
        "--enforcement-sha", proposed_sha, "--bundle-digest", proposed_digest,
        "--runtime-pin", runtime_pin, "--enforcement-ref-sha", proposed_sha,
        "--target-ref", target_ref, "--repo-root", str(base),
        "--base-root", str(base), "--candidate-root", str(candidate),
        "--head-sha", head_sha,
    ]


# The suites import E', but these probes run its real CLI with an exact
# post-activation records chain. Every invocation receives fresh private
# writable staging; only checked receipt bytes carry across the sequential
# universe. The honest control must admit; each bad candidate must be refused
# without a receipt. Ambient read-only fingerprints still require operator
# approval and cross-vendor diff review.
_PROBE_EMPTY_FINDINGS = (
    "\n\n"
    "def check_tree(root):\n"
    "    return []\n"
    "\n\n"
    "def check_transition(base, candidate):\n"
    "    return []\n"
)
_PROBE_MARKER = "checker-executed"
_PROBE_SEAM_PATH = Path("crates/frontend-core/src/views.rs")


_PROBE_COMMENT_WORDS = (
    "build", "cargo", "check", "crate", "default", "dependency", "edition", "feature", "keep",
    "layout", "lint", "local", "manifest", "member", "note", "order", "package", "path",
    "profile", "release", "review", "shared", "target", "test", "tooling", "version", "workspace",
)


def _probe_control(root: Path) -> None:
    """Insert one comment line of per-run random words into a random workspace manifest.

    The manifest, the words, and the position (before a random table header or
    at the end) are drawn per call, so no fixed control text marks a probe base.
    """
    random = secrets.SystemRandom()
    manifests = [root / "Cargo.toml"]
    try:
        members = tomllib.loads(manifests[0].read_text(encoding="utf-8")).get("workspace", {}).get("members", [])
    except (OSError, UnicodeError, tomllib.TOMLDecodeError, AttributeError) as exc:
        _fail(f"admission qualification cannot read the workspace manifest: {exc}")
    manifests.extend(path for member in members if (path := root / str(member) / "Cargo.toml").is_file())
    manifest = random.choice(manifests)
    lines = manifest.read_text(encoding="utf-8").splitlines(keepends=True)
    if lines and not lines[-1].endswith("\n"):
        lines[-1] += "\n"
    words = [random.choice(_PROBE_COMMENT_WORDS) for _ in range(3 + secrets.randbelow(6))]
    comment = "# " + " ".join(words).capitalize() + random.choice((".", "", ":")) + "\n"
    positions = [len(lines)] + [index for index, line in enumerate(lines) if line.startswith("[")]
    position = random.choice(positions)
    lines.insert(position, comment)
    manifest.write_text("".join(lines), encoding="utf-8")


def _probe_always_success(root: Path) -> None:
    marker = root.parent / _PROBE_MARKER
    (root / ARCH_DIR / "check.py").write_text(
        f"from pathlib import Path\nPath({str(marker)!r}).touch()\n" + _PROBE_EMPTY_FINDINGS,
        encoding="utf-8",
    )


def _probe_omitted_inventory(root: Path) -> None:
    path = root / ARCH_DIR / "inventories.py"
    path.write_text(path.read_text(encoding="utf-8") + _PROBE_EMPTY_FINDINGS, encoding="utf-8")


def _probe_workflow(root: Path) -> None:
    (root / WORKFLOW_PATH).write_text(
        "name: CI\non:\n  push:\njobs:\n  architecture:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: \"true\"\n",
        encoding="utf-8",
    )


def _dependency_names(manifest: dict[str, Any]) -> set[str]:
    tables: list[Any] = [manifest.get(key) for key in ("dependencies", "dev-dependencies", "build-dependencies")]
    targets = manifest.get("target")
    if isinstance(targets, dict):
        for table in targets.values():
            if isinstance(table, dict):
                tables.extend(table.get(key) for key in ("dependencies", "dev-dependencies", "build-dependencies"))
    names: set[str] = set()
    for table in tables:
        if not isinstance(table, dict):
            continue
        for key, spec in table.items():
            names.add(key)
            if isinstance(spec, dict) and isinstance(spec.get("package"), str):
                names.add(spec["package"])
    return names


def _probe_added_edge(root: Path, *, rotate: bool = False) -> None:
    """Add a normal path dependency that policy does not permit (restored or new debt)."""
    try:
        workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
        members = workspace.get("workspace", {}).get("members", [])
        packages: dict[str, Path] = {}
        manifests: dict[str, dict[str, Any]] = {}
        for member in members:
            manifest = tomllib.loads((root / str(member) / "Cargo.toml").read_text(encoding="utf-8"))
            name = manifest.get("package", {}).get("name")
            if isinstance(name, str):
                packages[name] = root / str(member)
                manifests[name] = manifest
    except (OSError, UnicodeError, tomllib.TOMLDecodeError, AttributeError) as exc:
        _fail(f"admission qualification cannot read workspace manifests: {exc}")
    policy = _policy(root)
    permitted = {
        (row.get("from"), row.get("to"))
        for row in policy.get("allow", [])
        if isinstance(row, dict) and "normal" in (row.get("kinds") or [])
    } | {
        (row.get("from"), row.get("to"))
        for row in policy.get("migration", [])
        if isinstance(row, dict) and row.get("kind") == "normal"
    }
    choices: list[tuple[str, str]] = []
    for source in sorted(packages):
        present = _dependency_names(manifests[source])
        for target in sorted(packages):
            if target != source and target not in present and (source, target) not in permitted:
                choices.append((source, target))
    if not choices:
        _fail("admission qualification cannot construct a dependency edge outside policy")
    source, target = choices[secrets.randbelow(len(choices) - 1) + 1] if rotate and len(choices) > 1 else choices[0]
    relative = os.path.relpath(packages[target], packages[source])
    manifest = packages[source] / "Cargo.toml"
    manifest.write_text(
        manifest.read_text(encoding="utf-8") + f'\n[dependencies.{json.dumps(target)}]\npath = {json.dumps(relative)}\n',
        encoding="utf-8",
    )


def _probe_rotated_edge(root: Path) -> None:
    _probe_added_edge(root, rotate=True)


def _probe_restored_vault_api(root: Path) -> None:
    manifest = root / "crates/vault/Cargo.toml"
    if not manifest.is_file() or not (root / "crates/api/Cargo.toml").is_file():
        _fail("admission qualification requires the vault and api crate manifests")
    if "api" in _dependency_names(tomllib.loads(manifest.read_text(encoding="utf-8"))):
        _fail("admission qualification requires the removed vault to api edge")
    manifest.write_text(manifest.read_text(encoding="utf-8") + '\n[dependencies.api]\npath = "../api"\n', encoding="utf-8")
    policy = root / POLICY_PATH
    policy.write_text(
        policy.read_text(encoding="utf-8")
        + '\n[[migration]]\nid = "VAULT-API-TEMP"\nfrom = "vault"\nto = "api"\nkind = "normal"\n'
        + 'owner = "T3"\nconsumers = ["vault:lib:vault"]\nreason = "Temporary fixture debt"\n'
        + 'removal_endpoint = "T3.3"\n',
        encoding="utf-8",
    )


def _probe_unregistered_seam(root: Path) -> None:
    path = root / _PROBE_SEAM_PATH
    if not path.is_file():
        _fail(f"admission qualification requires the frontend seam file {_PROBE_SEAM_PATH.as_posix()}")
    path.write_text(
        path.read_text(encoding="utf-8") + "\n#[derive(Clone)]\npub struct OperatorNoteView {\n    pub text: String,\n}\n",
        encoding="utf-8",
    )


ADMISSION_PROBES = (
    ("control: comment-only workspace manifest change", _probe_control, True),
    ("always-success candidate check.py", _probe_always_success, False),
    ("omitted-inventory candidate inventories.py", _probe_omitted_inventory, False),
    ("candidate CI workflow replacement", _probe_workflow, False),
    ("restored or new dependency edge outside policy", _probe_added_edge, False),
    ("rotated dependency edge outside policy", _probe_rotated_edge, False),
    ("restored vault to api edge and allowance", _probe_restored_vault_api, False),
    ("owning seam with public field and Clone, inventory row omitted", _probe_unregistered_seam, False),
)


def _require_independent_objects(clone: Path) -> None:
    """A probe repository must own its objects: no alternates file, no hard links to the source."""
    git_dir = clone / ".git"
    for path in sorted(git_dir.glob("**/objects/info/alternates")):
        _fail(f"probe repository borrows objects through an alternates file: {path}")
    for path in git_dir.glob("**/objects/**/*"):
        if path.is_file() and not path.is_symlink() and path.stat().st_nlink != 1:
            _fail(f"probe repository shares a hard-linked object file with its source: {path}")


def _clone_at(source: Path, destination: Path, sha: str, *, depth: int | None = None) -> Path:
    """Check out exact SHA in an independent copy with private objects.

    `--no-local` over `file://` copies objects through the transport: no
    alternates file pointing at the source and no hard links into its object
    store. Only external product sources are depth-limited; their full history
    is too large to copy per probe.
    """
    command = ["clone", "--quiet", "--no-local", "--no-checkout"]
    if depth is not None:
        command.extend(("--depth", str(depth)))
    _git(destination.parent, *command, source.resolve().as_uri(), str(destination))
    _git(destination, "checkout", "--quiet", "--detach", sha)
    _require_independent_objects(destination)
    return destination


def _populate_gitlinks(source_root: Path, clone: Path) -> None:
    """Check out each gitlink from the source checkout's initialized submodule, without network.

    Admission requires B/H checkouts with the client submodule initialized, so
    the probe trees must have it too. A source without it stays unpopulated,
    and the honest control then fails closed. A `file://` URL makes Git copy
    the submodule objects instead of hard-linking the source's object files.
    """
    for path in sorted(_gitlinks(clone)):
        local = source_root / path
        if not (local / ".git").exists():
            continue
        _git(clone, "config", f"submodule.{path}.url", local.resolve().as_uri())
        _git(clone, "-c", "protocol.file.allow=always", "submodule", "update", "--init", "--quiet", "--", path)
    _require_independent_objects(clone)


class _ProbeIdentities:
    """Author/subject pairs drawn at random from real history for probe commits.

    Each run shuffles the distinct pairs of the most recent commits and hands
    them out without repetition until the pool is exhausted, so a probe chain
    does not repeat one subject unless the history has too few distinct ones.
    """

    def __init__(self, root: Path, sha: str) -> None:
        rows: dict[tuple[str, str, str], None] = {}
        for line in _git(root, "log", "-n", "512", "--format=%an%x00%ae%x00%s", sha).splitlines():
            fields = tuple(line.split("\0"))
            if len(fields) == 3 and all(fields):
                rows[fields] = None
        if not rows:
            _fail(f"cannot read commit identities from {root} at {sha}")
        self._pool = list(rows)
        self._unused: list[tuple[str, str, str]] = []

    def draw(self) -> tuple[str, str, str]:
        if not self._unused:
            self._unused = list(self._pool)
            secrets.SystemRandom().shuffle(self._unused)
        return self._unused.pop()


def _commit_as(root: Path, identities: _ProbeIdentities) -> str:
    name, email, subject = identities.draw()
    # Probes only modify tracked files; `--update` leaves unpopulated gitlinks alone.
    _git(root, "add", "--update")
    _git(root, "-c", f"user.name={name}", "-c", f"user.email={email}", "commit", "--quiet", "--no-verify", "-m", subject)
    return _git(root, "rev-parse", "--verify", "HEAD")


_SECCOMP_KILL = 0x80000000
_SECCOMP_ALLOW = 0x7FFF0000
_AUDIT_ARCH_X86_64 = 0xC000003E
_X32_SYSCALL_BIT = 0x40000000
# Linux x86-64 numbers measured with `strace -f -c` over the complete honest
# e2e qualification (both real-suite cases, all admit/sequential/bad/prepare
# probes, Python, Git, pinned official rust-analyzer and nested bubblewrap).
# Full final trace: F0-CORE/checker/r6/final-honest-syscalls-summary.txt;
# initial trace: r6/honest-syscalls-summary.txt. The tmpfs-copy `sendfile`
# fallback (40) was first isolated in r6/denied-honest.trace. These traces
# include host harness calls and deliberately denied attack syscalls; neither
# those nor keyctl, bpf, perf, ptrace, or process_vm belong in the allowlist.
# A previously unmeasured call kills the probe, including honest runs.
_PROBE_SYSCALLS = (
    0, 1, 2, 3, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 21, 25, 28,
    32, 33, 37, 39, 40, 44, 45, 47, 49, 50, 51, 54, 56, 58, 59, 61, 63,
    72, 73, 74, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 95,
    102, 104, 107, 108, 110, 111, 112, 118, 120, 125, 126, 131, 137,
    155, 157, 158, 165, 166, 186, 194, 202, 204, 217, 218, 219,
    230, 231, 257, 262, 263, 272, 273, 280, 289, 290, 291, 293,
    302, 318, 319, 326, 334, 435, 436, 439,
)
_SYS_SOCKET = 41
_SYS_CONNECT = 42
_AF_UNIX = 1
_AF_INET = 2


def _probe_filter(*, deny_sockets: bool) -> int:
    """Default-deny measured syscalls; a killed probe never counts as refusal."""
    if sys.platform != "linux" or os.uname().machine != "x86_64":
        _fail("probe seccomp filter requires the pinned Linux x86-64 runtime")
    kill = (0x06, 0, 0, _SECCOMP_KILL)
    allow = (0x06, 0, 0, _SECCOMP_ALLOW)
    # seccomp_data: nr at offset 0, arch at 4, args[0] low word at 16.
    instructions: list[tuple[int, int, int, int]] = [
        (0x20, 0, 0, 4), (0x15, 1, 0, _AUDIT_ARCH_X86_64), kill,
        (0x20, 0, 0, 0), (0x35, 0, 1, _X32_SYSCALL_BIT), kill,
    ]
    for number in _PROBE_SYSCALLS:
        instructions.extend(((0x15, 0, 1, number), allow))
    if not deny_sockets:
        instructions.extend(((0x15, 0, 1, _SYS_CONNECT), allow))
        # Only AF_UNIX and AF_INET may create sockets. The network namespace
        # has no host interfaces; AF_INET's only usable peer is loopback.
        instructions.extend((
            (0x15, 0, 5, _SYS_SOCKET), (0x20, 0, 0, 16),
            (0x15, 2, 0, _AF_UNIX), (0x15, 1, 0, _AF_INET),
            kill, allow,
        ))
    instructions.append(kill)
    fd = os.memfd_create("architecture-probe-seccomp", flags=0)
    os.write(fd, struct.pack("<" + "HBBI" * len(instructions), *(
        value for instruction in instructions for value in instruction
    )))
    os.lseek(fd, 0, os.SEEK_SET)
    return fd


_STAGED_PROBE = r'''
import base64, json, os, shutil, subprocess, sys
from pathlib import Path

plan = json.loads(sys.argv[1])
work = Path(plan["work"])
source = Path("/probe-source")
for name in plan["inputs"]:
    original, staged = source / name, work / name
    if original.is_dir():
        shutil.copytree(original, staged, symlinks=True)
    else:
        shutil.copy2(original, staged)

def files(root, omit_git=False):
    result = {}
    for directory, dirs, names in os.walk(root, followlinks=False):
        here = Path(directory)
        if omit_git:
            dirs[:] = [name for name in dirs if name != ".git"]
        for name in sorted(dirs + names):
            path = here / name
            if path.is_symlink():
                raise ValueError("symlink in staged records")
            relative = path.relative_to(root).as_posix()
            result[relative + "/" if path.is_dir() else relative] = b"" if path.is_dir() else path.read_bytes()
    return result

records = Path(plan["records"]) if plan["records"] else None
clone = Path(plan["clone"]) if plan["clone"] else None
git_before = files(clone / ".git") if clone else None
# The source bind is absent from the executable's mount namespace. Every
# bind made writable below originates in this outer sandbox's private tmpfs.
child = [plan["bwrap"], "--unshare-user", "--unshare-pid", "--new-session", "--die-with-parent",
         "--clearenv", "--setenv", "PATH", os.environ["PATH"],
         "--setenv", "HOME", os.environ["HOME"], "--setenv", "TMPDIR", os.environ["TMPDIR"],
         "--ro-bind", "/", "/", "--proc", "/proc", "--dev", "/dev",
         "--tmpfs", "/tmp", "--tmpfs", "/probe-source", "--bind", str(work), str(work)]
for name in plan["inputs"]:
    path = work / name
    if path != clone:
        child.extend(("--ro-bind", str(path), str(path)))
for name in (plan["clone"], plan["evidence"], plan["home"], plan["temporary"]):
    if name:
        child.extend(("--bind", name, name))
if clone:
    child.extend(("--ro-bind", str(clone / ".git"), str(clone / ".git")))
child.extend(("--chdir", str(work), "--", *plan["command"]))
env = dict(os.environ)
if plan["library_dir"]:
    env["LD_LIBRARY_PATH"] = plan["library_dir"]
result = subprocess.run(child, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
snapshot = files(records, omit_git=True) if records else None
print(json.dumps({
    "returncode": result.returncode, "stdout": result.stdout,
    "git_unchanged": files(clone / ".git") == git_before if clone else True,
    "marker_written": (work / plan["marker"]).exists(),
    "records": {name: base64.b64encode(data).decode("ascii") for name, data in snapshot.items()} if snapshot else None,
}, separators=(",", ":")))
'''


_JOIN_PROBE_SESSION = r'''
import ctypes, os, sys
libc = ctypes.CDLL(None, use_errno=True)
if libc.syscall(250, 1, 0) < 0:  # keyctl(KEYCTL_JOIN_SESSION_KEYRING, NULL)
    raise OSError(ctypes.get_errno(), "cannot create a private probe session keyring")
os.execv(sys.argv[1], sys.argv[1:])
'''


def _isolated_probe(command: list[str], work: Path, *, deny_sockets: bool = True,
                    records: Path | None = None) -> subprocess.CompletedProcess[str]:
    """Stage inputs in inaccessible tmpfs; mount only staged copies into E'."""
    import base64

    home = work / secrets.token_hex(16)
    temporary = work / secrets.token_hex(16)
    home.mkdir()
    temporary.mkdir()
    bwrap = shutil.which("bwrap")
    parser = shutil.which("rust-analyzer")
    if bwrap is None or _file_sha256(Path(bwrap).resolve(strict=True)) != BWRAP_SHA256:
        _fail("pinned bubblewrap executable is missing or does not match its protected SHA-256")
    if parser is None or _file_sha256(Path(parser).resolve(strict=True)) != RUST_ANALYZER_SHA256:
        _fail("pinned rust-analyzer executable is missing or does not match its protected SHA-256")
    executable_paths = [Path(sys.executable), Path("/usr/bin/git"), Path("/usr/bin/sh"), Path(bwrap), Path(parser)]
    sandbox = [
        str(bwrap), "--unshare-user", "--unshare-pid", "--unshare-ipc", "--unshare-net",
        "--new-session", "--die-with-parent", "--clearenv",
        "--tmpfs", "/", "--dir", "/usr", "--dir", "/usr/bin",
        "--ro-bind", "/usr/lib", "/usr/lib",
        "--symlink", "usr/lib", "/lib", "--symlink", "usr/lib", "/lib64",
        "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
    ]
    runtime_dirs = [Path("/usr/share/git-core")]
    library_dir = Path(bwrap).resolve().parent.parent / "lib/x86_64-linux-gnu"
    if library_dir.is_dir():
        runtime_dirs.append(library_dir)
    mounted_dirs = {Path("/"), Path("/usr"), Path("/usr/bin"), Path("/usr/lib")}
    for path in [work, *runtime_dirs, *(item.parent for item in executable_paths)]:
        for parent in reversed((path, *path.parents)):
            if parent not in mounted_dirs:
                sandbox.extend(("--dir", str(parent)))
                mounted_dirs.add(parent)
    for directory in runtime_dirs:
        if directory.is_dir():
            sandbox.extend(("--ro-bind", str(directory), str(directory)))
    for path in executable_paths:
        sandbox.extend(("--ro-bind", str(path.resolve(strict=True)), str(path)))
    sandbox.extend(("--ro-bind", str(work), "/probe-source", "--tmpfs", str(work)))
    records_clone = None
    if records is not None:
        records_clone = _path(Path(_git(records, "rev-parse", "--show-toplevel")), "probe records repository")
        if not records.is_relative_to(records_clone) or records_clone.parent != work:
            _fail("probe records must be inside a private direct child of its work directory")
    evidence = None
    if "--evidence-root" in command:
        evidence = Path(command[command.index("--evidence-root") + 1])
        if evidence.parent != work:
            _fail("prepare evidence must be a private direct child of its work directory")
    inputs: list[str] = []
    for child in work.iterdir():
        if child in (home, temporary, evidence):
            continue
        mode = child.lstat().st_mode
        if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
            _fail(f"probe input is not a regular file or directory: {child}")
        if stat.S_ISDIR(mode):
            for directory, dirs, names in os.walk(child, followlinks=False):
                for name in dirs + names:
                    path = Path(directory) / name
                    kind = path.lstat().st_mode
                    if not (stat.S_ISREG(kind) or stat.S_ISDIR(kind) or stat.S_ISLNK(kind)):
                        _fail(f"probe input contains a host special file: {path}")
        inputs.append(child.name)
    if evidence is not None:
        sandbox.extend(("--tmpfs", str(evidence)))
    sandbox.extend(("--tmpfs", str(home), "--tmpfs", str(temporary), "--remount-ro", "/"))
    path = os.pathsep.join(dict.fromkeys((str(Path(parser).parent), str(Path(bwrap).parent), "/usr/bin")))
    sandbox.extend(("--setenv", "PATH", path, "--setenv", "HOME", str(home),
                    "--setenv", "TMPDIR", str(temporary), "--chdir", str(work)))
    environment = {"PATH": path}
    if library_dir.is_dir():
        environment["LD_LIBRARY_PATH"] = str(library_dir)
    filter_fd = _probe_filter(deny_sockets=deny_sockets)
    sandbox.extend(("--seccomp", str(filter_fd), "--"))
    plan = {
        "bwrap": str(bwrap), "work": str(work), "inputs": inputs, "command": command,
        "clone": str(records_clone) if records_clone else None,
        "records": str(records) if records else None,
        "evidence": str(evidence) if evidence else None,
        "home": str(home), "temporary": str(temporary), "marker": _PROBE_MARKER,
        "library_dir": str(library_dir) if library_dir.is_dir() else None,
    }
    staged = [sys.executable, "-I", "-S", "-c", _STAGED_PROBE, json.dumps(plan)]
    # A top-level probe joins a fresh session keyring before seccomp. Nested
    # qualification already inherits that inaccessible ring and cannot call
    # keyctl again: the enclosing probe's default-deny filter remains active.
    uid_map = Path("/proc/self/uid_map").read_text(encoding="ascii").strip().split()
    in_user_namespace = uid_map != ["0", "0", "4294967295"]
    launch = sandbox if in_user_namespace else [sys.executable, "-I", "-S", "-c", _JOIN_PROBE_SESSION, *sandbox]
    try:
        result = subprocess.run(launch + staged, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                env=environment, pass_fds=(filter_fd,), timeout=300)
    except subprocess.TimeoutExpired:
        _fail("proposed E' probe exceeded the 300-second child deadline")
    except OSError as exc:
        _fail(f"cannot isolate proposed E' probe in a private mount/network namespace: {exc}")
    finally:
        os.close(filter_fd)
    if result.returncode:
        _fail(f"staged probe launcher failed (exit {result.returncode}): {result.stdout[-4000:]}")
    try:
        packet = json.loads(result.stdout)
        if records_clone is None:
            return subprocess.CompletedProcess(command, packet["returncode"], packet["stdout"])
        after = {name: base64.b64decode(value, validate=True) for name, value in packet["records"].items()}
    except (ValueError, KeyError, TypeError) as exc:
        _fail(f"staged probe returned an invalid records packet: {exc}")
    if packet.get("marker_written"):
        _fail("admission-mode qualification failed: proposed E' executed candidate checker code")
    before = _record_files(records)
    allowed_changes = {"receipts.jsonl", "pending.json"}
    changed = {name for name in set(before) | set(after) if before.get(name) != after.get(name)}
    if packet["git_unchanged"] is not True or not changed <= allowed_changes:
        _fail(f"staged probe modified files outside the records protocol: "
              f"git_unchanged={packet['git_unchanged']!r}, changed={sorted(changed)!r}")
    for name in changed:
        destination = records / name
        if name in after:
            destination.write_bytes(after[name])
        else:
            destination.unlink()
    return subprocess.CompletedProcess(command, packet["returncode"], packet["stdout"])


def _qualify_admission_mode(
    *,
    scratch: Path,
    record_root: Path,
    target_ref: str,
    candidate_root: Path,
    head_sha: str,
    proposed_root: Path,
    proposed_sha: str,
    proposed_digest: str,
    runtime_pin: str,
    activation_receipt: dict[str, Any],
) -> None:
    records_top = _path(Path(_git(record_root, "rev-parse", "--show-toplevel")), "records repository")
    records_sha = _git(record_root, "rev-parse", "--verify", "HEAD")
    records_subpath = record_root.relative_to(records_top)
    bootstrap_sha = _read_json(record_root / "gate.json", "gate record")["bootstrap"]["accepted_sha"]
    product_depth = int(_git(candidate_root, "rev-list", "--count", head_sha, f"^{bootstrap_sha}")) + 2

    def universe(work: Path, *, chain: bytes | None = None,
                 product_source: Path | None = None, product_sha: str | None = None
                 ) -> tuple[Path, Path, str, Path, Path, _ProbeIdentities]:
        record_identities = _ProbeIdentities(records_top, records_sha)
        records_clone, records = _seed_probe_records(records_top, records_sha, records_subpath, work, activation_receipt)
        if chain is not None:
            (records / "receipts.jsonl").write_bytes(chain)
        simulated_records_sha = _commit_as(records_clone, record_identities)
        enforcement = _clone_at(proposed_root, work / secrets.token_hex(16), proposed_sha, depth=1)
        if bundle_digest(enforcement) != proposed_digest:
            _fail("proposed E' clone does not match its exact bundle digest")
        source = product_source or candidate_root
        base_sha = product_sha or head_sha
        base = _clone_at(source, work / secrets.token_hex(16), base_sha,
                         depth=product_depth if product_source is None else None)
        _populate_gitlinks(source, base)
        return records_clone, records, simulated_records_sha, enforcement, base, _ProbeIdentities(candidate_root, head_sha)

    def probe(work: Path, label: str, mutate: Any, admitted: bool, *, base: Path, base_sha: str, records: Path,
              records_commit: str, enforcement: Path, identities: _ProbeIdentities) -> tuple[Path, str]:
        _git(base, "update-ref", target_ref, base_sha)
        candidate = _clone_at(base, work / secrets.token_hex(16), base_sha)
        _populate_gitlinks(base, candidate)
        mutate(candidate)
        probe_sha = _commit_as(candidate, identities)
        records_before = _record_files(records)
        result = _isolated_probe(_probe_admit_command(
            enforcement, records, records_commit, proposed_sha,
            proposed_digest, runtime_pin, target_ref, base, candidate, probe_sha,
        ), work, records=records)
        output = result.stdout.strip()[-4000:]
        if admitted:
            if result.returncode != 0:
                _fail(f"admission-mode qualification failed: proposed E' refused the {label} (exit {result.returncode})\n{output}")
            _check_probe_records(records, records_before, True)
            _check_probe_receipt(records, target_ref, base_sha, probe_sha, input_digest(candidate), proposed_sha)
        else:
            if result.returncode == 0:
                _fail(f"admission-mode qualification failed: proposed E' admitted the known-bad {label} (exit 0)\n{output}")
            if result.returncode != _PROBE_REFUSAL_EXIT:
                _fail(f"admission-mode qualification failed: proposed E' did not deny the known-bad {label} with the gate's "
                      f"denial exit status {_PROBE_REFUSAL_EXIT} (exit {result.returncode})\n{output}")
            _check_probe_records(records, records_before, False)
        return candidate, probe_sha

    for label, mutate, admitted in ADMISSION_PROBES:
        with tempfile.TemporaryDirectory(dir=scratch) as temp_name:
            work = Path(temp_name)
            _, records, records_commit, enforcement, base, identities = universe(work)
            probe(work, label, mutate, admitted, base=base, base_sha=head_sha, records=records,
                  records_commit=records_commit, enforcement=enforcement, identities=identities)

    # Each step owns a new host clone and a new private tmpfs stage. The only
    # record state carried to the next step is the checked receipt-chain bytes;
    # product history is cloned afresh from the last accepted control commit.
    chain: bytes | None = None
    base_sha = head_sha
    last_control: Path | None = None
    with contextlib.ExitStack() as sources:
        for index in range(SEQUENTIAL_CONTROLS):
            work = Path(sources.enter_context(tempfile.TemporaryDirectory(dir=scratch)))
            _, records, records_commit, enforcement, base, identities = universe(
                work, chain=chain, product_source=last_control, product_sha=base_sha,
            )
            last_control, base_sha = probe(work, f"sequential control {index + 1}", _probe_control, True,
                                           base=base, base_sha=base_sha, records=records,
                                           records_commit=records_commit, enforcement=enforcement, identities=identities)
            chain = (records / "receipts.jsonl").read_bytes()
        bad = [(label, mutate) for label, mutate, admitted in ADMISSION_PROBES if not admitted]
        secrets.SystemRandom().shuffle(bad)
        for label, mutate in bad:
            work = Path(sources.enter_context(tempfile.TemporaryDirectory(dir=scratch)))
            _, records, records_commit, enforcement, base, identities = universe(
                work, chain=chain, product_source=last_control, product_sha=base_sha,
            )
            probe(work, f"{label} after {SEQUENTIAL_CONTROLS} sequential controls", mutate, False, base=base,
                  base_sha=base_sha, records=records, records_commit=records_commit,
                  enforcement=enforcement, identities=identities)


def _neuter_name(random: secrets.SystemRandom) -> str:
    return "_" + "_".join(random.sample(_PROBE_COMMENT_WORDS, 1 + random.randrange(3)))


def _neutered_admit(random: secrets.SystemRandom) -> str:
    """One of several syntactic shapes of an `admit` that records nothing."""
    first, second = random.sample([_neuter_name(random) + str(index) for index in range(2)], 2)
    words = " ".join(random.choice(_PROBE_COMMENT_WORDS) for _ in range(3 + random.randrange(4)))
    return random.choice((
        f"def admit({first}):\n    {second} = {first}\n    return {second} and None\n",
        f"admit = lambda *{first}, **{second}: None\n",
        f"def admit(*{first}, **{second}):\n    pass\n",
        f"class {first.title()}:\n    def __call__(self, *{second}):\n        return None\n\n\nadmit = {first.title()}()\n",
        f'def admit({first}):\n    """{words.capitalize()}."""\n',
        f"def admit({first}):\n    del {first}\n",
    ))


def _neutered_checks(random: secrets.SystemRandom) -> str:
    """Empty-result check API functions in a random order and syntactic shape."""
    functions = [("check_tree", 1), ("check_transition", 2), ("check_graph", 2)]
    random.shuffle(functions)
    blocks = []
    for name, arity in functions:
        params = ", ".join(_neuter_name(random) + str(index) for index in range(arity))
        blocks.append(random.choice((
            f"def {name}({params}):\n    return []\n",
            f"{name} = lambda {params}: []\n",
            f"def {name}(*{_neuter_name(random)}):\n    return list()\n",
        )))
    return "\n\n" + "\n\n".join(blocks)


def _qualify_prepare_mode(
    *,
    scratch: Path,
    record_root: Path,
    target_ref: str,
    candidate_root: Path,
    head_sha: str,
    proposed_root: Path,
    proposed_sha: str,
    proposed_digest: str,
    runtime_pin: str,
    activation_receipt: dict[str, Any],
    source_approval: dict[str, Any],
) -> None:
    """Require E' as a CLI to refuse one always-success E'' transition.

    E'' fails the E' suite before any further prepare probe can run. This is
    intentionally one level, with no marker or recursion bypass in E''. The
    verdict is the E' process exit status and the records alone; printed text
    proves nothing. E'' shapes are drawn per run.
    """
    records_top = _path(Path(_git(record_root, "rev-parse", "--show-toplevel")), "records repository")
    records_sha = _git(record_root, "rev-parse", "--verify", "HEAD")
    bootstrap_sha = _read_json(record_root / "gate.json", "gate record")["bootstrap"]["accepted_sha"]
    product_depth = int(_git(candidate_root, "rev-list", "--count", head_sha, f"^{bootstrap_sha}")) + 2
    random = secrets.SystemRandom()
    with tempfile.TemporaryDirectory(dir=scratch) as temp_name:
        work = Path(temp_name)
        def name() -> str:
            return secrets.token_hex(16)
        records_clone, records = _seed_probe_records(
            records_top, records_sha, record_root.relative_to(records_top), work, activation_receipt,
        )
        enforcement = _clone_at(proposed_root, work / name(), proposed_sha, depth=1)
        if bundle_digest(enforcement) != proposed_digest:
            _fail("proposed E' clone does not match its exact bundle digest")
        grandchild = _clone_at(enforcement, work / name(), proposed_sha)
        check = grandchild / ARCH_DIR / "check.py"
        check.write_text(check.read_text(encoding="utf-8") + _neutered_checks(random), encoding="utf-8")
        gate = grandchild / ARCH_DIR / "trusted_gate.py"
        source = gate.read_text(encoding="utf-8")
        entry = 'if __name__' + ' == "__main__":'
        if entry not in source:
            _fail("proposed E' has no CLI entry point")
        gate.write_text(source.replace(entry, _neutered_admit(random) + "\n\n" + entry, 1), encoding="utf-8")
        grandchild_sha = _commit_as(grandchild, _ProbeIdentities(proposed_root, proposed_sha))
        grandchild_digest = bundle_digest(grandchild)
        base = _clone_at(candidate_root, work / name(), head_sha, depth=product_depth)
        _populate_gitlinks(candidate_root, base)
        _git(base, "update-ref", target_ref, head_sha)
        candidate = _clone_at(base, work / name(), head_sha)
        _populate_gitlinks(base, candidate)
        for relative in (ARCH_DIR / "check.py", ARCH_DIR / "trusted_gate.py"):
            shutil.copyfile(grandchild / relative, candidate / relative)
        candidate_sha = _commit_as(candidate, _ProbeIdentities(candidate_root, head_sha))
        approval = {
            "schema_version": GATE_SCHEMA, "kind": "enforcement",
            "target_ref": target_ref, "base_sha": head_sha, "head_sha": candidate_sha,
            "contract_epoch": activation_receipt["contract_epoch"],
            "next_contract_epoch": activation_receipt["contract_epoch"] + 1,
            "active_enforcement_sha": proposed_sha, "active_bundle_digest": proposed_digest,
            "proposed_enforcement_sha": grandchild_sha, "proposed_bundle_digest": grandchild_digest,
            "runtime_pin": runtime_pin, "proposed_runtime_pin": runtime_pin,
            "input_digest": input_digest(candidate), "delta": [],
            "reason": source_approval["reason"],
            "owner_acknowledgements": source_approval["owner_acknowledgements"],
            "orchestrator_commit": candidate_sha,
            "operator_inline_approval": source_approval["operator_inline_approval"],
            "source_message": source_approval["source_message"],
        }
        approvals = records / "approvals"
        approvals.mkdir(exist_ok=True)
        (approvals / f"{candidate_sha}.json").write_bytes(_canonical(approval) + b"\n")
        _git(records_clone, "add", "--", (approvals / f"{candidate_sha}.json").relative_to(records_clone).as_posix())
        simulated_records_sha = _commit_as(records_clone, _ProbeIdentities(records_top, records_sha))
        records_before = _record_files(records)
        evidence = work / name()
        evidence.mkdir()
        command = [
            sys.executable, str(enforcement / ARCH_DIR / "trusted_gate.py"), "prepare-enforcement",
            "--records-root", str(records), "--records-commit", simulated_records_sha,
            "--records-ref-sha", simulated_records_sha,
            "--enforcement-root", str(enforcement), "--enforcement-sha", proposed_sha,
            "--bundle-digest", proposed_digest, "--runtime-pin", runtime_pin,
            "--enforcement-ref-sha", proposed_sha, "--target-ref", target_ref,
            "--repo-root", str(base), "--base-root", str(base),
            "--candidate-root", str(candidate), "--head-sha", candidate_sha,
            "--proposed-enforcement-root", str(grandchild),
            "--proposed-enforcement-sha", grandchild_sha,
            "--proposed-bundle-digest", grandchild_digest,
            "--evidence-root", str(evidence),
        ]
        # Nested suites use only private namespace-local sockets. This probe
        # stages its records and evidence in private tmpfs mounts.
        result = _isolated_probe(command, work, deny_sockets=False, records=records)
        output = result.stdout.strip()[-4000:]
        if _record_files(records) != records_before:
            _fail("prepare-mode qualification failed: proposed E' changed trusted records while probing the neutered checker\n"
                  + output)
        if result.returncode != _PROBE_REFUSAL_EXIT or (records / "pending.json").exists():
            _fail("prepare-mode qualification failed: proposed E' did not deny the neutered checker with the gate's "
                  f"denial exit status {_PROBE_REFUSAL_EXIT} and no pending.json (exit {result.returncode})\n{output}")


def _write_json_atomic(path: Path, value: dict[str, Any]) -> None:
    temporary = path.with_name(path.name + ".new")
    payload = _canonical(value) + b"\n"
    try:
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        dirfd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(dirfd)
        finally:
            os.close(dirfd)
    except OSError as exc:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        _fail(f"cannot atomically write trusted record {path}: {exc}")


def _write_pending(records_root: Path, pending: dict[str, Any]) -> None:
    if (records_root / "pending.json").exists():
        _fail("a protected enforcement transition is already pending")
    _write_json_atomic(records_root / "pending.json", pending)


def _make_receipt(*, sequence: int, target_ref: str, head_sha: str, base_sha: str, epoch: int, enforcement_sha: str, bundle_hash: str, runtime: str, activated_enforcement_sha: str, activated_bundle_hash: str, activated_runtime: str, schemas: dict[str, int], input_hash: str, transition_kind: str | None, delta: list[str], approval_digest: str | None, pending_digest: str | None, previous_digest: str) -> dict[str, Any]:
    receipt = {
        "schema_version": GATE_SCHEMA,
        "sequence": sequence,
        "target_ref": target_ref,
        "head_sha": head_sha,
        "base_sha": base_sha,
        "contract_epoch": epoch,
        "enforcement_sha": enforcement_sha,
        "bundle_digest": bundle_hash,
        "runtime_pin": runtime,
        "activated_enforcement_sha": activated_enforcement_sha,
        "activated_bundle_digest": activated_bundle_hash,
        "activated_runtime_pin": activated_runtime,
        "schema_versions": schemas,
        "input_digest": input_hash,
        "orchestrator_commit": head_sha,
        "transition_kind": transition_kind,
        "delta": delta,
        "approval_digest": approval_digest,
        "pending_digest": pending_digest,
        "previous_digest": previous_digest,
    }
    receipt["receipt_digest"] = _digest(receipt)
    return receipt


def _record_prev_digest(record_root: Path, state: dict[str, Any], receipts: list[dict[str, Any]]) -> str:
    if receipts:
        return receipts[-1]["receipt_digest"]
    return _digest(state["bootstrap"])


def _append_receipt(records_root: Path, state: dict[str, Any], receipts: list[dict[str, Any]], receipt: dict[str, Any], *, expected_pending_digest: str | None) -> None:
    directory_fd = os.open(records_root, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        fcntl.flock(directory_fd, fcntl.LOCK_EX)
        current, current_receipts, pending = _read_record_store(records_root, state["target_ref"])
        if len(current_receipts) != len(receipts) or (current_receipts and current_receipts[-1]["receipt_digest"] != receipts[-1]["receipt_digest"]):
            _fail("trusted receipt chain changed during admission")
        if bool(pending) != (expected_pending_digest is not None):
            _fail("pending transition state changed during admission")
        if pending and _digest(pending) != expected_pending_digest:
            _fail("pending transition changed during admission")
        if not current_receipts and current["head_sha"] != state["bootstrap"]["accepted_sha"]:
            _fail("trusted bootstrap changed during admission")
        path = records_root / "receipts.jsonl"
        old = path.read_bytes() if path.exists() else b""
        if old and not old.endswith(b"\n"):
            _fail("receipt file must end with a newline")
        payload = old + _canonical(receipt) + b"\n"
        temporary = records_root / "receipts.jsonl.new"
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        dirfd = os.open(records_root, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(dirfd)
        finally:
            os.close(dirfd)
    except OSError as exc:
        _fail(f"cannot append trusted admission receipt: {exc}")
    finally:
        os.close(directory_fd)


def _remove_pending(records_root: Path) -> None:
    path = records_root / "pending.json"
    try:
        path.unlink()
        fd = os.open(records_root, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    except OSError as exc:
        _fail(f"cannot clear completed pending transition: {exc}")


def _check_enforcement_ref(actual_sha: str, current: dict[str, Any], pending: dict[str, Any]) -> None:
    actual_sha = _hex(actual_sha, SHA_RE, "enforcement ref tip")
    expected = pending.get("proposed_enforcement_sha") if pending else current["enforcement_sha"]
    if actual_sha != expected:
        _fail(f"protected enforcement ref tip is {actual_sha}, expected {expected}")


def admit(args: argparse.Namespace) -> None:
    record_root, active_root, base_root, candidate_root, current, receipts, pending, meta = _load_current_inputs(args, local=True)
    base_sha = current["head_sha"]
    head_sha = _hex(args.head_sha, SHA_RE, "candidate H")
    candidate_digest = _verify_roots(base_root, candidate_root, base_sha, head_sha, current)
    _check_enforcement_ref(args.enforcement_ref_sha, current, pending)
    if pending:
        proposed_root = _path(args.proposed_enforcement_root, "proposed enforcement root")
        _require_separate(candidate_root, proposed_root, record_root, active_root, base_root)
        proposed_sha = _hex(args.proposed_enforcement_sha, SHA_RE, "proposed enforcement SHA")
        proposed_digest = _hex(args.proposed_bundle_digest, DIGEST_RE, "proposed enforcement digest")
        verify_checkout(proposed_root, proposed_sha, "proposed enforcement checkout", check_submodules=False)
        actual_proposed_digest = bundle_digest(proposed_root)
        if actual_proposed_digest != proposed_digest:
            _fail("proposed enforcement bundle digest does not match its exact pin")
        if pending["target_ref"] != meta["target_ref"] or pending["contract_epoch"] != current["contract_epoch"] or pending["next_contract_epoch"] != current["contract_epoch"] + 1:
            _fail("pending transition does not match the active target and epoch")
        if pending["base_sha"] != base_sha or pending["head_sha"] != head_sha or pending["input_digest"] != candidate_digest:
            _fail("candidate does not match the pending protected transition")
        if pending["active_enforcement_sha"] != current["enforcement_sha"] or pending["active_bundle_digest"] != current["bundle_digest"]:
            _fail("pending transition does not start at the active enforcement pin")
        if pending["proposed_enforcement_sha"] != proposed_sha or pending["proposed_bundle_digest"] != proposed_digest:
            _fail("proposed E' does not match the pending exact pin")
        if pending["runtime_pin"] != current["runtime_pin"] or pending["proposed_runtime_pin"] != args.runtime_pin:
            _fail("pending transition runtime pin mismatch")
        approval, approval_hash = _read_approval(record_root, head_sha)
        if approval is None or approval_hash != pending["approval_digest"]:
            _fail("pending approval is absent or changed")
        findings = _evaluate_transition(active_root, base_root, candidate_root)
        if findings != pending["delta"]:
            _fail("active checker findings changed after pending qualification")
        _validate_approval(approval, kind="enforcement", target_ref=meta["target_ref"], base_sha=base_sha, head_sha=head_sha, current=current, candidate_digest=candidate_digest, findings=findings)
        _check_approval_proposal(approval, proposed_sha, proposed_digest, args.runtime_pin)
        _require_candidate_surface(active_root, candidate_root, proposed_root)
        expected_qualification = _qualification_digest(active_root, proposed_root, base_sha, head_sha, candidate_digest, approval_hash, args.runtime_pin)
        if pending["qualification_digest"] != expected_qualification:
            _fail("pending enforcement qualification no longer matches its exact inputs")
        new_e, new_digest = proposed_sha, proposed_digest
        epoch = current["contract_epoch"] + 1
        runtime = args.runtime_pin
        kind = "enforcement"
        delta = findings
        pending_hash = _digest(pending)
        approval_digest = approval_hash
        schemas = _schema_versions(candidate_root)
    else:
        if args.proposed_enforcement_root is not None or args.proposed_enforcement_sha is not None or args.proposed_bundle_digest is not None:
            _fail("proposed E' arguments require an orchestrator-prepared pending transition")
        findings = _evaluate_transition(active_root, base_root, candidate_root)
        approval, approval_hash, approval_kind = _approval_state(record_root, base_root, candidate_root, head_sha, base_sha, meta["target_ref"], current, candidate_digest, findings)
        differences = _require_candidate_surface(active_root, candidate_root, None)
        if approval_kind == "enforcement":
            _fail("enforcement approval requires the prepare-enforcement transition")
        if approval_kind == "policy":
            if differences:
                _fail("policy approval cannot change protected enforcement bytes")
            _check_approval_proposal(approval, current["enforcement_sha"], current["bundle_digest"], current["runtime_pin"])
            kind = "policy"
            epoch = current["contract_epoch"] + 1
            delta = findings
        else:
            kind = None
            epoch = current["contract_epoch"]
            delta = []
        new_e, new_digest, runtime = current["enforcement_sha"], current["bundle_digest"], current["runtime_pin"]
        pending_hash = None
        approval_digest = approval_hash
        schemas = _schema_versions(candidate_root)
        if schemas != current["schema_versions"] and kind is None:
            _fail("schema version changes require an approved contract transition")
        if kind is None and approval is not None:
            _fail("an approval cannot authorize an ordinary admission")
    if head_sha == base_sha:
        _fail("H must differ from accepted B")
    sequence = len(receipts) + 1
    receipt = _make_receipt(
        sequence=sequence,
        target_ref=meta["target_ref"],
        head_sha=head_sha,
        base_sha=base_sha,
        epoch=epoch,
        enforcement_sha=current["enforcement_sha"],
        bundle_hash=current["bundle_digest"],
        runtime=current["runtime_pin"],
        activated_enforcement_sha=new_e,
        activated_bundle_hash=new_digest,
        activated_runtime=runtime,
        schemas=schemas,
        input_hash=candidate_digest,
        transition_kind=kind,
        delta=delta,
        approval_digest=approval_digest,
        pending_digest=pending_hash,
        previous_digest=_record_prev_digest(record_root, _read_json(record_root / "gate.json", "gate record"), receipts),
    )
    _append_receipt(
        record_root,
        _read_json(record_root / "gate.json", "gate record"),
        receipts,
        receipt,
        expected_pending_digest=_digest(pending) if pending else None,
    )
    if pending:
        _remove_pending(record_root)
    print(f"architecture admission recorded: {meta['target_ref']} {base_sha} -> {head_sha}; receipt {receipt['receipt_digest']}")


def prepare_enforcement(args: argparse.Namespace) -> None:
    record_root, active_root, base_root, candidate_root, current, receipts, pending, meta = _load_current_inputs(args, local=True)
    if pending:
        _fail("another protected enforcement transition is already pending")
    _check_enforcement_ref(args.enforcement_ref_sha, current, {})
    base_sha = current["head_sha"]
    head_sha = _hex(args.head_sha, SHA_RE, "candidate H")
    candidate_digest = _verify_roots(base_root, candidate_root, base_sha, head_sha, current)
    proposed_root = _path(args.proposed_enforcement_root, "proposed enforcement root")
    _require_separate(candidate_root, proposed_root, record_root, active_root, base_root)
    proposed_sha = _hex(args.proposed_enforcement_sha, SHA_RE, "proposed enforcement SHA")
    proposed_digest = _hex(args.proposed_bundle_digest, DIGEST_RE, "proposed enforcement digest")
    verify_checkout(proposed_root, proposed_sha, "proposed enforcement checkout", check_submodules=False)
    actual_proposed_digest = bundle_digest(proposed_root)
    if actual_proposed_digest != proposed_digest:
        _fail("proposed enforcement bundle digest does not match its exact pin")
    approval, approval_hash = _read_approval(record_root, head_sha)
    findings = _evaluate_transition(active_root, base_root, candidate_root)
    if approval is None:
        _fail("protected enforcement update requires an external exact-H approval")
    candidate_diff = _require_candidate_surface(active_root, candidate_root, proposed_root)
    if not candidate_diff:
        _fail("proposed E' must change protected executable bytes")
    if approval.get("kind") != "enforcement":
        _fail("protected byte changes require an enforcement approval")
    _validate_approval(approval, kind="enforcement", target_ref=meta["target_ref"], base_sha=base_sha, head_sha=head_sha, current=current, candidate_digest=candidate_digest, findings=findings)
    _check_approval_proposal(approval, proposed_sha, proposed_digest, args.runtime_pin)
    if args.runtime_pin != current["runtime_pin"]:
        _fail("runtime changes require the new exact runtime to be provisioned before qualification")
    evidence_root = _path(args.evidence_root, "qualification evidence root")
    _require_separate(evidence_root, record_root, active_root, base_root, candidate_root, proposed_root)
    _run_child(active_root, evidence_root=evidence_root, checker_root=proposed_root)
    _run_child(proposed_root, evidence_root=evidence_root, checker_root=proposed_root)
    qualification = _qualification_digest(active_root, proposed_root, base_sha, head_sha, candidate_digest, approval_hash, args.runtime_pin)
    pending_record = {
        "schema_version": GATE_SCHEMA,
        "target_ref": meta["target_ref"],
        "base_sha": base_sha,
        "head_sha": head_sha,
        "contract_epoch": current["contract_epoch"],
        "next_contract_epoch": current["contract_epoch"] + 1,
        "active_enforcement_sha": current["enforcement_sha"],
        "active_bundle_digest": current["bundle_digest"],
        "proposed_enforcement_sha": proposed_sha,
        "proposed_bundle_digest": proposed_digest,
        "runtime_pin": current["runtime_pin"],
        "proposed_runtime_pin": args.runtime_pin,
        "input_digest": candidate_digest,
        "approval_digest": approval_hash,
        "delta": findings,
        "reason": approval["reason"],
        "owner_acknowledgements": approval["owner_acknowledgements"],
        "orchestrator_commit": head_sha,
        "qualification_digest": qualification,
    }
    activation_receipt = _make_receipt(
        sequence=len(receipts) + 1,
        target_ref=meta["target_ref"],
        head_sha=head_sha,
        base_sha=base_sha,
        epoch=current["contract_epoch"] + 1,
        enforcement_sha=current["enforcement_sha"],
        bundle_hash=current["bundle_digest"],
        runtime=current["runtime_pin"],
        activated_enforcement_sha=proposed_sha,
        activated_bundle_hash=proposed_digest,
        activated_runtime=args.runtime_pin,
        schemas=_schema_versions(candidate_root),
        input_hash=candidate_digest,
        transition_kind="enforcement",
        delta=findings,
        approval_digest=approval_hash,
        pending_digest=_digest(pending_record),
        previous_digest=_record_prev_digest(record_root, _read_json(record_root / "gate.json", "gate record"), receipts),
    )
    _qualify_admission_mode(
        scratch=evidence_root,
        record_root=record_root,
        target_ref=meta["target_ref"],
        candidate_root=candidate_root,
        head_sha=head_sha,
        proposed_root=proposed_root,
        proposed_sha=proposed_sha,
        proposed_digest=proposed_digest,
        runtime_pin=args.runtime_pin,
        activation_receipt=activation_receipt,
    )
    _qualify_prepare_mode(
        scratch=evidence_root,
        record_root=record_root,
        target_ref=meta["target_ref"],
        candidate_root=candidate_root,
        head_sha=head_sha,
        proposed_root=proposed_root,
        proposed_sha=proposed_sha,
        proposed_digest=proposed_digest,
        runtime_pin=args.runtime_pin,
        activation_receipt=activation_receipt,
        source_approval=approval,
    )
    _write_pending(record_root, pending_record)
    print(f"protected transition prepared: H={head_sha} E'={proposed_sha}; publish E' only after committing pending.json")


def _run_bundle_suite(enforcement_root: Path, evidence_root: Path) -> None:
    _run_child(enforcement_root, evidence_root=evidence_root)


def ci_check(args: argparse.Namespace) -> None:
    record_root, enforcement_root, base_root, candidate_root, current, receipts, pending, meta = _load_current_inputs(args, local=False)
    if pending:
        _fail("protected enforcement transition is pending; CI cannot accept or diagnose a new baseline")
    target_ref = meta["target_ref"]
    base_sha = _hex(args.base_sha, SHA_RE, "event base B")
    head_sha = _hex(args.head_sha, SHA_RE, "event head H")
    target_head_sha = _hex(args.target_head_sha, SHA_RE, "current target head")
    expected_target = base_sha if args.mode == "pull_request" else head_sha
    if target_head_sha != expected_target:
        _fail(f"event target moved: expected {expected_target}, current target is {target_head_sha}")
    _check_enforcement_ref(args.enforcement_ref_sha, current, {})
    _run_bundle_suite(enforcement_root, _path(args.evidence_root, "CI evidence root"))
    if args.mode == "pull_request":
        if base_sha != current["head_sha"]:
            _fail("PR base SHA is not the current accepted target baseline")
        candidate_digest = _verify_roots(base_root, candidate_root, base_sha, head_sha, current)
        findings = _evaluate_transition(enforcement_root, base_root, candidate_root)
        approval, approval_hash, approval_kind = _approval_state(record_root, base_root, candidate_root, head_sha, base_sha, target_ref, current, candidate_digest, findings)
        differences = _require_candidate_surface(enforcement_root, candidate_root, None)
        if approval_kind == "enforcement" or differences:
            _fail("PR contains a protected enforcement change without a completed local E' transition")
        if approval_kind == "policy":
            _check_approval_proposal(approval, current["enforcement_sha"], current["bundle_digest"], current["runtime_pin"])
        print(f"architecture CI pass: candidate {head_sha} against accepted B={base_sha}")
    elif args.mode == "push":
        if not receipts or current["head_sha"] != head_sha:
            _fail("pushed H has no current admission receipt")
        receipt = receipts[-1]
        if receipt["head_sha"] != head_sha or receipt["base_sha"] != base_sha:
            _fail("push event before/head do not match the latest admission receipt")
        candidate_digest = _verify_roots(base_root, candidate_root, base_sha, head_sha, {**current, "head_sha": base_sha, "input_digest": _accepted_input_for_base(base_sha, receipts, _read_json(record_root / "gate.json", "gate record"))})
        if candidate_digest != receipt["input_digest"]:
            _fail("pushed H source data does not match its admission receipt")
        findings = _evaluate_transition(enforcement_root, base_root, candidate_root)
        approval, approval_hash, approval_kind = _approval_state(record_root, base_root, candidate_root, head_sha, base_sha, target_ref, _prior_pin(receipts, head_sha, _read_json(record_root / "gate.json", "gate record")), candidate_digest, findings)
        expected_kind = None if approval_kind == "removal" else approval_kind
        if receipt["approval_digest"] != approval_hash or receipt["transition_kind"] != expected_kind:
            _fail("push approval record does not match its receipt")
        if receipt["activated_enforcement_sha"] != current["enforcement_sha"] or receipt["activated_bundle_digest"] != current["bundle_digest"] or receipt["activated_runtime_pin"] != current["runtime_pin"]:
            _fail("push receipt does not activate the currently pinned enforcement identity")
        differences = _require_candidate_surface(enforcement_root, candidate_root, None)
        if differences:
            _fail("pushed H differs from active enforcement bundle")
        print(f"architecture CI pass: admitted push {head_sha} against B={base_sha}")
    else:
        _fail("CI mode must be pull_request or push")


def _accepted_input_for_base(base_sha: str, receipts: list[dict[str, Any]], state: dict[str, Any]) -> str:
    if base_sha == state["bootstrap"]["accepted_sha"]:
        return state["bootstrap"]["input_digest"]
    for receipt in receipts:
        if receipt["head_sha"] == base_sha:
            return receipt["input_digest"]
    _fail(f"event B={base_sha} is not in the accepted receipt chain")


def _prior_pin(receipts: list[dict[str, Any]], head_sha: str, state: dict[str, Any]) -> dict[str, Any]:
    previous = {"head_sha": state["bootstrap"]["accepted_sha"], "contract_epoch": state["bootstrap"]["contract_epoch"], "enforcement_sha": state["bootstrap"]["enforcement_sha"], "bundle_digest": state["bootstrap"]["bundle_digest"], "runtime_pin": state["bootstrap"]["runtime_pin"], "schema_versions": state["bootstrap"]["schema_versions"], "input_digest": state["bootstrap"]["input_digest"]}
    for receipt in receipts:
        if receipt["head_sha"] == head_sha:
            return previous
        previous = {"head_sha": receipt["head_sha"], "contract_epoch": receipt["contract_epoch"], "enforcement_sha": receipt["activated_enforcement_sha"], "bundle_digest": receipt["activated_bundle_digest"], "runtime_pin": receipt["activated_runtime_pin"], "schema_versions": receipt["schema_versions"], "input_digest": receipt["input_digest"]}
    _fail(f"head {head_sha} is not in the accepted receipt chain")


def _add_common(parser: argparse.ArgumentParser, *, include_proposal: bool = False, evidence: bool = False) -> None:
    parser.add_argument("--records-root", type=Path, required=True)
    parser.add_argument("--records-commit", required=True)
    parser.add_argument("--records-ref-sha", required=True)
    parser.add_argument("--enforcement-root", type=Path, required=True)
    parser.add_argument("--enforcement-sha", required=True)
    parser.add_argument("--bundle-digest", required=True)
    parser.add_argument("--runtime-pin", required=True)
    parser.add_argument("--enforcement-ref-sha", required=True)
    parser.add_argument("--target-ref", required=True)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--base-root", type=Path, required=True)
    parser.add_argument("--candidate-root", type=Path, required=True)
    parser.add_argument("--head-sha", required=True)
    if include_proposal:
        parser.add_argument("--proposed-enforcement-root", type=Path, required=True)
        parser.add_argument("--proposed-enforcement-sha", required=True)
        parser.add_argument("--proposed-bundle-digest", required=True)
    else:
        parser.add_argument("--proposed-enforcement-root", type=Path)
        parser.add_argument("--proposed-enforcement-sha")
        parser.add_argument("--proposed-bundle-digest")
    if evidence:
        parser.add_argument("--evidence-root", type=Path, required=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    admit_parser = commands.add_parser("admit", help="admit one exact merge into a trusted local target")
    _add_common(admit_parser)
    admit_parser.set_defaults(run=admit)
    prepare_parser = commands.add_parser("prepare-enforcement", help="qualify E' and record a pending protected transition")
    _add_common(prepare_parser, include_proposal=True, evidence=True)
    prepare_parser.set_defaults(run=prepare_enforcement)
    ci_parser = commands.add_parser("ci", help="run the detective CI check")
    _add_common(ci_parser, evidence=True)
    ci_parser.add_argument("--mode", choices=("pull_request", "push"), required=True)
    ci_parser.add_argument("--base-sha", required=True)
    ci_parser.add_argument("--target-head-sha", required=True)
    ci_parser.set_defaults(run=ci_check)
    args = parser.parse_args(argv)
    try:
        args.run(args)
    except GateError as exc:
        sys.stderr.write(f"architecture gate denied: {exc}\n")
        return 1
    except (OSError, ValueError, KeyError, TypeError, tomllib.TOMLDecodeError) as exc:
        sys.stderr.write(f"architecture gate denied: malformed trusted input: {exc}\n")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
