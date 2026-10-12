#!/usr/bin/env python3
"""Trusted local admission and detective CI for architecture changes.

This program reads product trees as data. It loads check.py and its helper
modules only from the separately pinned enforcement checkout.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Any

FORMAT = "f0-architecture-gate-v1"
GATE_SCHEMA = 1
SHA_RE = re.compile(r"\A[0-9a-f]{40}\Z")
DIGEST_RE = re.compile(r"\A[0-9a-f]{64}\Z")
RUST_ANALYZER_SHA256 = "88ae4c54c68d6899010c18074b9e415f2dbee09527fb703fd4536b665571db0a"
RUST_ANALYZER_VERSION = "rust-analyzer 1 (9074e9b4c6 2026-09-06)"
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
    with tempfile.TemporaryDirectory(prefix="arch-qualification-", dir=scratch) as temp_name:
        copied = Path(temp_name) / "bundle"
        copied.mkdir()
        shutil.copytree(source / ARCH_DIR, copied / ARCH_DIR, symlinks=False)
        (copied / WORKFLOW_PATH).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / WORKFLOW_PATH, copied / WORKFLOW_PATH)
        parser = shutil.which("rust-analyzer")
        if parser is None:
            _fail("pinned rust-analyzer executable is unavailable during qualification")
        parser_dir = str(Path(parser).resolve(strict=True).parent)
        system_path = os.pathsep.join(dict.fromkeys((parser_dir, "/usr/bin", "/bin")))
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
    })


# Admission-mode qualification. The suites above import E' as a module, so
# E' can tell that it is under test. These probes instead run E' exactly as
# an operator runs an ordinary admission once E' is active: the same
# interpreter, environment, working directory, and `admit` command line,
# against a records chain identical to the post-activation chain. Nothing
# in that invocation marks it as qualification. The control must be
# admitted (the simulated store is valid); every known-bad candidate must
# be refused without a receipt.
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


def _probe_control(root: Path) -> None:
    manifest = root / "Cargo.toml"
    manifest.write_text(manifest.read_text(encoding="utf-8") + "\n# Workspace notes are tracked in the README.\n", encoding="utf-8")


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


def _probe_added_edge(root: Path) -> None:
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
    for source in sorted(packages):
        present = _dependency_names(manifests[source])
        for target in sorted(packages):
            if target == source or target in present or (source, target) in permitted:
                continue
            relative = os.path.relpath(packages[target], packages[source])
            manifest = packages[source] / "Cargo.toml"
            manifest.write_text(
                manifest.read_text(encoding="utf-8") + f'\n[dependencies.{json.dumps(target)}]\npath = {json.dumps(relative)}\n',
                encoding="utf-8",
            )
            return
    _fail("admission qualification cannot construct a dependency edge outside policy")


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
    ("owning seam with public field and Clone, inventory row omitted", _probe_unregistered_seam, False),
)


def _clone_at(source: Path, destination: Path, sha: str) -> Path:
    """Check out exact SHA in a new clone that borrows the source object store."""
    _git(destination.parent, "clone", "--quiet", "--shared", "--no-checkout", str(source), str(destination))
    _git(destination, "checkout", "--quiet", "--detach", sha)
    return destination


def _populate_gitlinks(source_root: Path, clone: Path) -> None:
    """Check out each gitlink from the source checkout's initialized submodule, without network.

    Admission requires B/H checkouts with the client submodule initialized, so
    the probe trees must have it too. A source without it stays unpopulated,
    and the honest control then fails closed.
    """
    for path in sorted(_gitlinks(clone)):
        local = source_root / path
        if not (local / ".git").exists():
            continue
        _git(clone, "config", f"submodule.{path}.url", str(local))
        _git(clone, "-c", "protocol.file.allow=always", "submodule", "update", "--init", "--quiet", "--", path)


def _commit_as(root: Path, identity_root: Path, identity_sha: str, message: str) -> str:
    author = _git(identity_root, "log", "-1", "--format=%an%x00%ae", identity_sha).split("\0")
    if len(author) != 2:
        _fail(f"cannot read commit identity of {identity_sha}")
    # Probes only modify tracked files; `--update` leaves unpopulated gitlinks alone.
    _git(root, "add", "--update")
    _git(root, "-c", f"user.name={author[0]}", "-c", f"user.email={author[1]}", "commit", "--quiet", "--no-verify", "-m", message)
    return _git(root, "rev-parse", "--verify", "HEAD")


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
    for label, mutate, admitted in ADMISSION_PROBES:
        with tempfile.TemporaryDirectory(prefix="arch-", dir=scratch) as temp_name:
            work = Path(temp_name)
            records_clone = _clone_at(records_top, work / "records", records_sha)
            records = records_clone / records_subpath
            receipts_path = records / "receipts.jsonl"
            old = receipts_path.read_bytes()
            if old and not old.endswith(b"\n"):
                _fail("receipt file must end with a newline")
            receipts_path.write_bytes(old + _canonical(activation_receipt) + b"\n")
            simulated_records_sha = _commit_as(records_clone, records_top, records_sha, "Record architecture admission receipt")
            enforcement = _clone_at(proposed_root, work / "enforcement", proposed_sha)
            if bundle_digest(enforcement) != proposed_digest:
                _fail("proposed E' clone does not match its exact bundle digest")
            base = _clone_at(candidate_root, work / "base", head_sha)
            _populate_gitlinks(candidate_root, base)
            _git(base, "update-ref", target_ref, head_sha)
            candidate = _clone_at(candidate_root, work / "candidate", head_sha)
            _populate_gitlinks(candidate_root, candidate)
            mutate(candidate)
            probe_sha = _commit_as(candidate, candidate_root, head_sha, "Update workspace")
            receipts_before = receipts_path.read_bytes()
            command = [
                sys.executable, str(enforcement / ARCH_DIR / "trusted_gate.py"), "admit",
                "--records-root", str(records),
                "--records-commit", simulated_records_sha,
                "--records-ref-sha", simulated_records_sha,
                "--enforcement-root", str(enforcement),
                "--enforcement-sha", proposed_sha,
                "--bundle-digest", proposed_digest,
                "--runtime-pin", runtime_pin,
                "--enforcement-ref-sha", proposed_sha,
                "--target-ref", target_ref,
                "--repo-root", str(base),
                "--base-root", str(base),
                "--candidate-root", str(candidate),
                "--head-sha", probe_sha,
            ]
            try:
                result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=dict(os.environ))
            except OSError as exc:
                _fail(f"cannot run proposed E' in admission mode: {exc}")
            output = result.stdout.strip()[-4000:]
            if (work / _PROBE_MARKER).exists():
                _fail(f"admission-mode qualification failed: proposed E' executed candidate checker code ({label})\n{output}")
            if admitted:
                if result.returncode != 0:
                    _fail(f"admission-mode qualification failed: proposed E' refused the {label} (exit {result.returncode})\n{output}")
                current, receipts, pending = _read_record_store(records, target_ref)
                if pending or current["head_sha"] != probe_sha or receipts[-1]["transition_kind"] is not None or current["enforcement_sha"] != proposed_sha:
                    _fail(f"admission-mode qualification failed: proposed E' did not record an ordinary receipt for the {label}\n{output}")
            elif result.returncode == 0 or receipts_path.read_bytes() != receipts_before or (records / "pending.json").exists():
                _fail(f"admission-mode qualification failed: proposed E' admitted the known-bad {label} (exit {result.returncode})\n{output}")


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
