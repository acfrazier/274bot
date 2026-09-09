#!/usr/bin/env python3
"""Prepare and verify the frozen direct-owner diagnostic source archive.

The preparation path reads Git objects through temporary indexes. It never
checks out a branch, touches either real index, or reads moving production
source as an archive input.
"""

from __future__ import annotations

import argparse
import difflib
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from typing import Any

HOST_ORIGINAL = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf"
CLIENT_ORIGINAL = "3456edc8dabf7b25ada78110ffa56327af9f67a4"
HOST_REVIEWED = "3cdc3e4fbeb960fe4c270eb994ce22746787eb2d"
CLIENT_REVIEWED = "5c73a4a27f3d72834c2c2a071668eb197eb39fd9"
ROOT_GITLINK_BINDING = "edc3a3e"
ARCHIVE_NAME = "frozen-direct-owner-source.tar.gz"
ARCHIVE_ROOT = "direct-owner-source"
PATCH_NAMES = ("host-initial.patch", "host-correction.patch", "client-overlay.patch")
PATCH_BINDINGS = {
    "host-initial.patch": (104314, "5229bab8d7c59046dbb8e134daca2bd9e570043c3016e3277e2db6369fdbb8a9"),
    "host-correction.patch": (104127, "d2646cc175a376a295de2bd9261c4c8d8c7a2af57bbeb52171732b73480f4917"),
    "client-overlay.patch": (19104, "0a5d43ed382c43d293e17824b1565dc4a5258d5b17971edaea55ad6a58324430"),
}
FEATURE_MANIFESTS = (
    "crates/api/Cargo.toml",
    "crates/host/Cargo.toml",
    "crates/host-play/Cargo.toml",
    "crates/script/Cargo.toml",
    "crates/tui/Cargo.toml",
    "vendor/fr-client-rust/crates/client/Cargo.toml",
)
SUPPORT_FILES = (
    "validate_direct_owner_capture.py",
    "test_validate_direct_owner_capture.py",
)
TEST_ONLY_DERIVATIONS = {
    "crates/host-play/src/lib.rs": {
        "materialized": {
            "bytes": 357877,
            "sha256": "f997f22f00bb205cdad74eec70d9dfc5750c3f8f80d9ba52c024f5ea4ea9120f",
        },
        "reviewed": {
            "bytes": 361346,
            "sha256": "096753fc31636e829058e33f54770090810688346c9be47b4ed49193813801ee",
        },
        "tests_start_line": 4270,
        "production_prefix_sha256": "37e1d14040400d818ee56aa11372f284f7f98ce008fd3d4145b16fa57d0503e5",
    },
    "crates/host-play/src/memory.rs": {
        "materialized": {
            "bytes": 161591,
            "sha256": "76e633acf9d4dcd1fd03f1a57ea722f5111025790071679119e7e5932ece2991",
        },
        "reviewed": {
            "bytes": 161447,
            "sha256": "74b7a18164f1b6861697ec8c651c2f81a400dbd41c8bf772cdab29ff4b3bb923",
        },
        "tests_start_line": 2077,
        "production_prefix_sha256": "1af24d12119b500b802e02c63f0f33055b49fda16c03a7eae54121a7af06aa69",
    },
}
TEST_MODULE_MARKER = b"#[cfg(test)]\nmod tests {"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_git(repo: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        ["git", *args], cwd=repo, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    if result.returncode:
        raise RuntimeError(
            f"git {' '.join(args)} failed in {repo}: {result.stderr.decode(errors='replace')}"
        )
    return result


def git_bytes(repo: Path, revision_path: str) -> bytes:
    return run_git(repo, "show", revision_path).stdout


def git_text(repo: Path, *args: str) -> str:
    return run_git(repo, *args).stdout.decode().strip()


def actual_repo_state(root: Path, client_root: Path) -> dict[str, Any]:
    def one(repo: Path) -> dict[str, Any]:
        index = Path(git_text(repo, "rev-parse", "--git-path", "index"))
        if not index.is_absolute():
            index = repo / index
        return {
            "head": git_text(repo, "rev-parse", "HEAD"),
            "index_path": str(index.resolve()),
            "index_sha256": file_sha256(index),
            "tracked_status": git_text(repo, "status", "--porcelain=v1", "--untracked-files=no"),
        }

    return {
        "host": one(root),
        "client": one(client_root),
        "locks": {
            "Cargo.lock": file_sha256(root / "Cargo.lock"),
            "vendor/fr-client-rust/Cargo.lock": file_sha256(client_root / "Cargo.lock"),
        },
    }


def temporary_index(repo: Path, base: str, patches: list[Path], temp: Path) -> tuple[dict[str, str], list[dict[str, Any]]]:
    env = dict(os.environ)
    env["GIT_INDEX_FILE"] = str(temp / "index")
    run_git(repo, "read-tree", base, env=env)
    results = []
    for patch in patches:
        run_git(repo, "apply", "--cached", "--check", str(patch), env=env)
        run_git(repo, "apply", "--cached", str(patch), env=env)
        results.append({"patch": patch.name, "check_exit": 0, "apply_exit": 0})
    return env, results


def index_entries(repo: Path, env: dict[str, str], prefix: str = "") -> list[dict[str, Any]]:
    raw = run_git(repo, "ls-files", "--stage", "-z", env=env).stdout
    result = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        meta, path_bytes = item.split(b"\t", 1)
        mode, blob, stage = meta.decode().split()
        if stage != "0":
            raise RuntimeError(f"unmerged temporary index entry: {path_bytes!r}")
        path = path_bytes.decode()
        result.append({"path": prefix + path, "mode": mode, "git_blob": blob})
    return result


def diff_names(repo: Path, env: dict[str, str], base: str) -> list[dict[str, str]]:
    raw = run_git(repo, "diff-index", "--cached", "--name-status", "-z", base, env=env).stdout
    fields = raw.split(b"\0")
    result = []
    index = 0
    while index < len(fields) and fields[index]:
        status_code = fields[index].decode()
        index += 1
        path = fields[index].decode()
        index += 1
        if status_code.startswith(("R", "C")):
            second = fields[index].decode()
            index += 1
            path = f"{path} -> {second}"
        result.append({"status": status_code, "path": path})
    return result


def checkout_index(repo: Path, env: dict[str, str], destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    prefix = str(destination.resolve()) + os.sep
    run_git(repo, "checkout-index", "--all", f"--prefix={prefix}", env=env)


def member_data(path: Path, mode: str) -> bytes:
    if mode == "120000":
        return os.readlink(path).encode()
    return path.read_bytes()


def production_prefix(data: bytes, path: str, expected_line: int) -> bytes:
    if data.count(TEST_MODULE_MARKER) != 1:
        raise RuntimeError(f"expected one top-level test module marker: {path}")
    offset = data.index(TEST_MODULE_MARKER)
    line = data[:offset].count(b"\n") + 1
    if line != expected_line:
        raise RuntimeError(
            f"test module boundary moved in {path}: line {line}, expected {expected_line}"
        )
    return data[:offset]


def test_only_diff(path: str, materialized: bytes, reviewed: bytes) -> str:
    return "".join(
        difflib.unified_diff(
            materialized.decode("utf-8").splitlines(keepends=True),
            reviewed.decode("utf-8").splitlines(keepends=True),
            fromfile=f"a/{path} (original-plus-reviewed-overlays)",
            tofile=f"b/{path} (reviewed-commit)",
            n=3,
        )
    )


def verify_overlay_members(
    stage: Path,
    entries: list[dict[str, Any]],
    receipt_members: list[dict[str, Any]],
    reviewed_repo: Path,
    reviewed_commit: str,
    stage_prefix: str = "",
) -> list[dict[str, Any]]:
    by_path = {entry["path"]: entry for entry in entries}
    verified = []
    for expected in receipt_members:
        archive_path = stage_prefix + expected["path"]
        entry = by_path.get(archive_path)
        if entry is None:
            raise RuntimeError(f"overlay member missing from temporary index: {archive_path}")
        data = member_data(stage / archive_path, entry["mode"])
        reviewed = git_bytes(reviewed_repo, f"{reviewed_commit}:{expected['path']}")
        actual = {"bytes": len(data), "sha256": sha256(data)}
        wanted = {"bytes": expected["bytes"], "sha256": expected["sha256"]}
        reviewed_identity = {"bytes": len(reviewed), "sha256": sha256(reviewed)}
        row = {
            "path": archive_path,
            "materialized": actual,
            "receipt": wanted,
            "reviewed_commit": reviewed_identity,
            "materialized_matches_receipt": actual == wanted,
            "materialized_matches_reviewed": data == reviewed,
            "receipt_matches_reviewed": wanted == reviewed_identity,
        }
        derivation = TEST_ONLY_DERIVATIONS.get(expected["path"])
        if derivation is None:
            row["classification"] = "exact_reviewed"
            row["verified"] = actual == wanted and data == reviewed
        else:
            materialized_prefix = production_prefix(
                data, expected["path"], derivation["tests_start_line"]
            )
            reviewed_prefix = production_prefix(
                reviewed, expected["path"], derivation["tests_start_line"]
            )
            prefix_identity = {
                "bytes": len(materialized_prefix),
                "sha256": sha256(materialized_prefix),
            }
            difference = test_only_diff(expected["path"], data, reviewed)
            row.update(
                {
                    "classification": "derived_test_only",
                    "expected_materialized": derivation["materialized"],
                    "materialized_matches_expected_derived": actual
                    == derivation["materialized"],
                    "reviewed_matches_bound_identity": reviewed_identity
                    == derivation["reviewed"],
                    "tests_start_line": derivation["tests_start_line"],
                    "production_prefix": prefix_identity,
                    "production_prefix_equal": materialized_prefix == reviewed_prefix,
                    "production_prefix_matches_bound_identity": prefix_identity["sha256"]
                    == derivation["production_prefix_sha256"],
                    "differences_confined_to_test_module": materialized_prefix
                    == reviewed_prefix,
                    "difference": {
                        "bytes": len(difference.encode()),
                        "sha256": sha256(difference.encode()),
                    },
                    "_difference_text": difference,
                }
            )
            row["verified"] = all(
                (
                    row["receipt_matches_reviewed"],
                    row["materialized_matches_expected_derived"],
                    row["reviewed_matches_bound_identity"],
                    row["production_prefix_equal"],
                    row["production_prefix_matches_bound_identity"],
                    bool(difference),
                )
            )
        verified.append(row)
    return verified


def source_members(stage: Path, entries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    members = []
    for entry in sorted(entries, key=lambda item: item["path"]):
        if entry["mode"] == "160000":
            raise RuntimeError(f"unmaterialized gitlink in source tree: {entry['path']}")
        path = stage / entry["path"]
        data = member_data(path, entry["mode"])
        members.append(
            {
                "path": entry["path"],
                "mode": entry["mode"],
                "bytes": len(data),
                "sha256": sha256(data),
                "git_blob": entry["git_blob"],
            }
        )
    return members


def deterministic_archive(stage: Path, destination: Path) -> None:
    with tempfile.NamedTemporaryFile(dir=destination.parent, suffix=".tar", delete=False) as raw:
        raw_path = Path(raw.name)
    try:
        with tarfile.open(raw_path, mode="w", format=tarfile.PAX_FORMAT) as archive:
            root_info = tarfile.TarInfo(ARCHIVE_ROOT)
            root_info.type = tarfile.DIRTYPE
            root_info.mode = 0o755
            root_info.mtime = root_info.uid = root_info.gid = 0
            root_info.uname = root_info.gname = ""
            archive.addfile(root_info)
            for path in sorted(stage.rglob("*"), key=lambda item: item.relative_to(stage).as_posix()):
                rel = path.relative_to(stage).as_posix()
                name = f"{ARCHIVE_ROOT}/{rel}"
                info = tarfile.TarInfo(name)
                status = path.lstat()
                info.mode = stat.S_IMODE(status.st_mode)
                info.mtime = info.uid = info.gid = 0
                info.uname = info.gname = ""
                if path.is_symlink():
                    info.type = tarfile.SYMTYPE
                    info.linkname = os.readlink(path)
                    archive.addfile(info)
                elif path.is_dir():
                    info.type = tarfile.DIRTYPE
                    archive.addfile(info)
                elif path.is_file():
                    info.size = status.st_size
                    with path.open("rb") as stream:
                        archive.addfile(info, stream)
                else:
                    raise RuntimeError(f"unsupported source member type: {path}")
        with raw_path.open("rb") as source, destination.open("wb") as output:
            with gzip.GzipFile(filename="", mode="wb", compresslevel=9, fileobj=output, mtime=0) as compressed:
                shutil.copyfileobj(source, compressed, length=1024 * 1024)
    finally:
        raw_path.unlink(missing_ok=True)


def parse_feature_table(path: Path) -> dict[str, list[str]]:
    table: dict[str, list[str]] = {}
    active = False
    pending = ""
    for raw in path.read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if line.startswith("["):
            active = line == "[features]"
            pending = ""
            continue
        if not active or not line:
            continue
        pending = f"{pending} {line}".strip()
        if pending.count("[") != pending.count("]"):
            continue
        key, separator, value = pending.partition("=")
        if not separator:
            raise RuntimeError(f"cannot parse feature declaration in {path}: {pending}")
        table[key.strip()] = [item for item in value.split('"')[1::2]]
        pending = ""
    if pending:
        raise RuntimeError(f"unterminated feature declaration in {path}: {pending}")
    return table


def features(stage: Path) -> dict[str, Any]:
    result = {}
    for relative in FEATURE_MANIFESTS:
        path = stage / relative
        result[relative] = {
            "bytes": path.stat().st_size,
            "sha256": file_sha256(path),
            "features": parse_feature_table(path),
        }
    return result


def verify_archive_members(archive: Path, manifest: dict[str, Any]) -> None:
    if archive.stat().st_size != manifest["archive"]["bytes"]:
        raise RuntimeError("archive byte count mismatch")
    if file_sha256(archive) != manifest["archive"]["sha256"]:
        raise RuntimeError("archive SHA-256 mismatch")
    expected = {f"{ARCHIVE_ROOT}/{row['path']}": row for row in manifest["source_members"]}
    seen: set[str] = set()
    with tarfile.open(archive, mode="r:gz") as stream:
        for member in stream:
            if member.isdir():
                continue
            row = expected.get(member.name)
            if row is None or member.name in seen:
                raise RuntimeError(f"unexpected or duplicate archive member: {member.name}")
            if member.issym():
                data = member.linkname.encode()
            elif member.isfile():
                extracted = stream.extractfile(member)
                if extracted is None:
                    raise RuntimeError(f"cannot read archive member: {member.name}")
                data = extracted.read()
            else:
                raise RuntimeError(f"unsupported archive member: {member.name}")
            if len(data) != row["bytes"] or sha256(data) != row["sha256"]:
                raise RuntimeError(f"archive member identity mismatch: {member.name}")
            archive_mode = (
                "120000"
                if member.issym()
                else "100755"
                if member.mode & 0o111
                else "100644"
            )
            if archive_mode != row["mode"]:
                raise RuntimeError(f"archive member mode mismatch: {member.name}")
            seen.add(member.name)
    missing = sorted(set(expected) - seen)
    if missing:
        raise RuntimeError(f"archive missing {len(missing)} members; first={missing[0]}")


def prepare(root: Path, output: Path) -> None:
    if output.exists():
        raise RuntimeError(f"output must be a fresh nonexistent path: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir()
    receipts = root / "diagnostics/direct-owner-capture-implementation"
    client_root = root / "vendor/fr-client-rust"
    overlay = json.loads((receipts / "overlay-manifest.json").read_text())
    binding = json.loads((receipts / "root-client-commit-binding.json").read_text())
    before = actual_repo_state(root, client_root)

    if overlay["original_host"] != HOST_ORIGINAL or overlay["original_client"] != CLIENT_ORIGINAL:
        raise RuntimeError("overlay receipt original commit mismatch")
    if binding["reviewed_host"] != HOST_REVIEWED or binding["client_commit"] != CLIENT_REVIEWED:
        raise RuntimeError("root binding receipt mismatch")
    patch_receipts = {}
    for name in PATCH_NAMES:
        path = receipts / name
        actual = (path.stat().st_size, file_sha256(path))
        if actual != PATCH_BINDINGS[name]:
            raise RuntimeError(f"patch binding mismatch: {name}: {actual}")
        if overlay["patches"][name] != {"bytes": actual[0], "sha256": actual[1]}:
            raise RuntimeError(f"patch receipt mismatch: {name}")
        patch_receipts[name] = {"bytes": actual[0], "sha256": actual[1]}
    support_receipts = {}
    for name in SUPPORT_FILES:
        local = Path(__file__).resolve().parent / "support" / name
        reviewed = git_bytes(root, f"{HOST_REVIEWED}:docs/memory/{name}")
        if local.read_bytes() != reviewed:
            raise RuntimeError(f"qualification support differs from reviewed host commit: {name}")
        support_receipts[name] = {"bytes": len(reviewed), "sha256": sha256(reviewed)}

    for repo, commit in (
        (root, HOST_ORIGINAL),
        (root, HOST_REVIEWED),
        (root, ROOT_GITLINK_BINDING),
        (client_root, CLIENT_ORIGINAL),
        (client_root, CLIENT_REVIEWED),
    ):
        if git_text(repo, "cat-file", "-t", commit) != "commit":
            raise RuntimeError(f"required commit object unavailable: {commit}")
    original_gitlink = git_text(root, "rev-parse", f"{HOST_ORIGINAL}:vendor/fr-client-rust")
    reviewed_gitlink = git_text(root, "rev-parse", f"{ROOT_GITLINK_BINDING}:vendor/fr-client-rust")
    if original_gitlink != CLIENT_ORIGINAL or reviewed_gitlink != CLIENT_REVIEWED:
        raise RuntimeError("host/client gitlink provenance mismatch")
    if git_text(client_root, "rev-parse", f"{CLIENT_REVIEWED}^") != CLIENT_ORIGINAL:
        raise RuntimeError("reviewed client commit is not a direct overlay on original client")

    archive = output / ARCHIVE_NAME
    manifest_path = output / "frozen-source-manifest.json"
    result_path = output / "preparation-result.json"
    with tempfile.TemporaryDirectory(prefix="direct-owner-prepare-") as temp_name:
        temp = Path(temp_name)
        stage = temp / ARCHIVE_ROOT
        stage.mkdir()
        host_temp = temp / "host-index"
        host_temp.mkdir()
        host_env, host_apply = temporary_index(
            root,
            HOST_ORIGINAL,
            [receipts / "host-initial.patch", receipts / "host-correction.patch"],
            host_temp,
        )
        client_temp = temp / "client-index"
        client_temp.mkdir()
        client_env, client_apply = temporary_index(
            client_root, CLIENT_ORIGINAL, [receipts / "client-overlay.patch"], client_temp
        )
        host_diff = diff_names(root, host_env, HOST_ORIGINAL)
        client_diff = diff_names(client_root, client_env, CLIENT_ORIGINAL)
        host_paths = {row["path"] for row in overlay["members"]["host"]}
        client_paths = {row["path"] for row in overlay["members"]["client"]}
        if {row["path"] for row in host_diff} != host_paths:
            raise RuntimeError("host overlay diff scope differs from receipt members")
        if {row["path"] for row in client_diff} != client_paths:
            raise RuntimeError("client overlay diff scope differs from receipt members")

        checkout_index(root, host_env, stage)
        checkout_index(client_root, client_env, stage / "vendor/fr-client-rust")
        host_entries = [
            row for row in index_entries(root, host_env) if row["mode"] != "160000"
        ]
        client_entries = index_entries(client_root, client_env, "vendor/fr-client-rust/")
        entries = host_entries + client_entries
        host_verified = verify_overlay_members(
            stage, entries, overlay["members"]["host"], root, HOST_REVIEWED
        )
        client_verified = verify_overlay_members(
            stage,
            entries,
            overlay["members"]["client"],
            client_root,
            CLIENT_REVIEWED,
            "vendor/fr-client-rust/",
        )
        difference_parts = []
        for row in host_verified:
            difference = row.pop("_difference_text", None)
            if difference is not None:
                difference_parts.append(difference)
        difference_text = "".join(difference_parts)
        difference_path = output / "derived-vs-reviewed-test-only.patch"
        difference_path.write_text(difference_text)
        derived_paths = {
            row["path"]
            for row in host_verified
            if row["classification"] == "derived_test_only"
        }
        exact_host_count = sum(
            row["classification"] == "exact_reviewed" and row["materialized_matches_reviewed"]
            for row in host_verified
        )
        exact_client_count = sum(row["materialized_matches_reviewed"] for row in client_verified)
        provenance_audit = {
            "schema": "direct-owner-frozen-provenance-audit-v2",
            "host_original": HOST_ORIGINAL,
            "client_original": CLIENT_ORIGINAL,
            "host_reviewed": HOST_REVIEWED,
            "client_reviewed": CLIENT_REVIEWED,
            "host": host_verified,
            "client": client_verified,
            "exact_reviewed_full_file_counts": {
                "host": exact_host_count,
                "client": exact_client_count,
            },
            "test_only_derivations": {
                "paths": sorted(derived_paths),
                "expected_paths": sorted(TEST_ONLY_DERIVATIONS),
                "difference_artifact": {
                    "path": difference_path.name,
                    "bytes": difference_path.stat().st_size,
                    "sha256": file_sha256(difference_path),
                },
            },
        }
        provenance_audit["verified"] = (
            all(row["verified"] for row in host_verified + client_verified)
            and derived_paths == set(TEST_ONLY_DERIVATIONS)
            and exact_host_count == 19
            and exact_client_count == 5
        )
        (output / "provenance-audit.json").write_text(
            json.dumps(provenance_audit, indent=2, sort_keys=True) + "\n"
        )
        if not provenance_audit["verified"]:
            failed = [
                row["path"] for row in host_verified + client_verified if not row["verified"]
            ]
            failure = {
                "schema": "direct-owner-preparation-result-v1",
                "prepared": False,
                "archive_verified": False,
                "production_tree_unchanged": before == actual_repo_state(root, client_root),
                "error": "reviewed overlay member mismatch",
                "failed_members": failed,
            }
            result_path.write_text(json.dumps(failure, indent=2, sort_keys=True) + "\n")
            raise RuntimeError(
                f"reviewed overlay member mismatch ({len(failed)}): {', '.join(failed)}"
            )
        members = source_members(stage, entries)
        locks = {
            row["path"]: {"bytes": row["bytes"], "sha256": row["sha256"]}
            for row in members
            if row["path"].endswith("Cargo.lock")
        }
        deterministic_archive(stage, archive)
        manifest = {
            "schema": "direct-owner-frozen-source-v1",
            "archive": {
                "name": ARCHIVE_NAME,
                "root": ARCHIVE_ROOT,
                "bytes": archive.stat().st_size,
                "sha256": file_sha256(archive),
                "determinism": "sorted PAX tar; uid/gid/mtime zero; gzip filename empty and mtime zero",
            },
            "provenance": {
                "host_original": HOST_ORIGINAL,
                "client_original": CLIENT_ORIGINAL,
                "host_reviewed": HOST_REVIEWED,
                "client_reviewed": CLIENT_REVIEWED,
                "root_gitlink_binding": ROOT_GITLINK_BINDING,
                "original_host_gitlink": original_gitlink,
                "reviewed_root_gitlink": reviewed_gitlink,
                "overlay_receipt_current_host_is_non_authoritative": overlay.get("current_host"),
                "production_input": "original host/client Git objects plus only the three bound reviewed patches",
            },
            "patches": patch_receipts,
            "patch_application": {"host": host_apply, "client": client_apply},
            "overlay_diff": {"host": host_diff, "client": client_diff},
            "reviewed_overlay_members": {"host": host_verified, "client": client_verified},
            "qualification_support": {
                "source_commit": HOST_REVIEWED,
                "source_prefix": "docs/memory",
                "members": support_receipts,
                "archive_member": False,
            },
            "features": features(stage),
            "build_contract": {
                "package": "tui",
                "binary": "tui-play",
                "features": ["memory-profile-no-alloc", "memory-owner-capture"],
                "cargo_flags": ["--offline", "--locked"],
                "runtime_capture_default": "BOT_MEMORY_OWNER_CAPTURE=0",
                "snapshot_dedup": False,
            },
            "locks": locks,
            "exclusions": [
                "Git metadata",
                "Cargo target directories",
                "runtime output directories",
                "accounts, vaults, cache contents, and server binaries",
            ],
            "source_member_count": len(members),
            "source_members": members,
        }
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        verify_archive_members(archive, manifest)

    after = actual_repo_state(root, client_root)
    unchanged = before == after
    result = {
        "schema": "direct-owner-preparation-result-v1",
        "prepared": True,
        "archive_verified": True,
        "production_tree_unchanged": unchanged,
        "before": before,
        "after": after,
        "manifest_sha256": file_sha256(manifest_path),
        "archive_sha256": file_sha256(archive),
        "archive_bytes": archive.stat().st_size,
    }
    result_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    (output / f"{ARCHIVE_NAME}.sha256").write_text(f"{result['archive_sha256']}  {ARCHIVE_NAME}\n")
    if not unchanged:
        raise RuntimeError("actual repository/index/lock state changed during preparation")
    print(json.dumps(result, indent=2, sort_keys=True))


def verify(output: Path) -> None:
    manifest = json.loads((output / "frozen-source-manifest.json").read_text())
    archive = output / manifest["archive"]["name"]
    verify_archive_members(archive, manifest)
    print(
        json.dumps(
            {
                "archive": str(archive),
                "bytes": archive.stat().st_size,
                "sha256": file_sha256(archive),
                "source_members": manifest["source_member_count"],
                "verified": True,
            },
            indent=2,
            sort_keys=True,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subparsers.add_parser("prepare")
    prepare_parser.add_argument("--output-dir", type=Path, required=True)
    verify_parser = subparsers.add_parser("verify")
    verify_parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args.root.resolve(), args.output_dir.resolve())
        else:
            verify(args.output_dir.resolve())
    except (OSError, RuntimeError, KeyError, ValueError, json.JSONDecodeError) as error:
        print(json.dumps({"prepared": False, "error": str(error)}), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
