#!/usr/bin/env python3
"""Stage and verify the frozen TUI test-only spawn guard overlay.

The immutable frozen archive is the sole input.  A fresh extracted tree is
created outside the checkout, the hash-bound patch is applied to exactly one
source file, and every archive member is rehashed before optional bounded
offline/locked TUI tests.  Tests may construct synthetic temporary vault
fixtures, but this helper never accesses an operator/live vault, account,
cache, server, or network, and never edits the source archive or checkout.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import signal
import shutil
import subprocess
import sys
import tarfile
import tempfile
from typing import Any

ARCHIVE_ROOT = "direct-owner-source"
TARGET = "crates/tui/src/bin.rs"
ORIGINAL_SHA256 = "a859ca06b33066cbb55641facb7eea449317c9d3e7a6f93ff11c5787d01499d2"
DERIVED_SHA256 = "aee97345c86cff0eb3253ef56b3e64fd123991f57bebbe5fcb750d899839e28c"
PATCH_SHA256 = "3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4"
TEST_FILE_MARKER = b"#[cfg(test)]\nmod tests {"
EXPECTED_MEMBER_COUNT = 1124


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_path(name: str) -> PurePosixPath:
    pure = PurePosixPath(name)
    if pure.is_absolute() or ".." in pure.parts or not pure.parts:
        raise RuntimeError(f"unsafe archive member path: {name}")
    if pure.parts[0] != ARCHIVE_ROOT:
        raise RuntimeError(f"unexpected archive root: {name}")
    return pure


def member_bytes(path: Path) -> bytes:
    return os.readlink(path).encode() if path.is_symlink() else path.read_bytes()


def mode(path: Path) -> str:
    if path.is_symlink():
        return "120000"
    if path.is_file():
        return "100755" if path.stat().st_mode & 0o111 else "100644"
    raise RuntimeError(f"unsupported staged member: {path}")


def extract_and_verify(archive: Path, manifest: dict[str, Any], destination: Path) -> tuple[Path, dict[str, Any]]:
    expected_archive = manifest["archive"]
    actual_archive = {"bytes": archive.stat().st_size, "sha256": sha256_file(archive)}
    if actual_archive != {"bytes": expected_archive["bytes"], "sha256": expected_archive["sha256"]}:
        raise RuntimeError(f"immutable archive identity mismatch: {actual_archive}")
    source = destination / ARCHIVE_ROOT
    source.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:gz") as stream:
        for item in stream:
            relative = safe_path(item.name)
            path = destination / relative
            if item.isdir():
                path.mkdir(parents=True, exist_ok=True)
            elif item.isfile():
                path.parent.mkdir(parents=True, exist_ok=True)
                extracted = stream.extractfile(item)
                if extracted is None:
                    raise RuntimeError(f"cannot read {item.name}")
                with path.open("xb") as output:
                    shutil.copyfileobj(extracted, output)
                path.chmod(item.mode)
            elif item.issym():
                target = PurePosixPath(item.linkname)
                if target.is_absolute() or ".." in target.parts:
                    raise RuntimeError(f"unsafe symlink target: {item.name}")
                path.parent.mkdir(parents=True, exist_ok=True)
                path.symlink_to(item.linkname)
            else:
                raise RuntimeError(f"unsupported archive member: {item.name}")

    expected = {row["path"]: row for row in manifest["source_members"]}
    actual_paths = sorted(
        path.relative_to(source).as_posix()
        for path in source.rglob("*")
        if path.is_file() or path.is_symlink()
    )
    if len(expected) != EXPECTED_MEMBER_COUNT or len(expected) != len(actual_paths):
        raise RuntimeError(f"expected {EXPECTED_MEMBER_COUNT} source files, got {len(expected)} manifest/{len(actual_paths)} staged")
    if set(actual_paths) != set(expected):
        raise RuntimeError("staged source member set differs from immutable manifest")
    rows = []
    for relative in actual_paths:
        path = source / relative
        data = member_bytes(path)
        wanted = expected[relative]
        actual = {"bytes": len(data), "sha256": sha256_bytes(data), "mode": mode(path)}
        if actual != {"bytes": wanted["bytes"], "sha256": wanted["sha256"], "mode": wanted["mode"]}:
            raise RuntimeError(f"immutable member mismatch: {relative}")
        rows.append({"path": relative, **actual})
    return source, {"archive": actual_archive, "source_member_count": len(rows), "members": rows}


def apply_patch(source: Path, patch: Path) -> dict[str, Any]:
    patch_hash = sha256_file(patch)
    if PATCH_SHA256 and patch_hash != PATCH_SHA256:
        raise RuntimeError(f"overlay patch identity mismatch: {patch_hash}")
    before = (source / TARGET).read_bytes()
    if sha256_bytes(before) != ORIGINAL_SHA256:
        raise RuntimeError("TUI source is not the frozen original before patch")
    command = ["patch", "--batch", "--forward", "--strip=1", "--input", str(patch)]
    result = subprocess.run(command, cwd=source, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"test-only patch failed ({result.returncode}): {result.stdout}")
    after = (source / TARGET).read_bytes()
    if sha256_bytes(after) != DERIVED_SHA256:
        raise RuntimeError(f"derived TUI source hash mismatch: {sha256_bytes(after)}")
    if before == after:
        raise RuntimeError("test-only patch made no change")
    return {
        "command": command,
        "patch_sha256": patch_hash,
        "original_sha256": sha256_bytes(before),
        "derived_sha256": sha256_bytes(after),
        "changed_paths": [TARGET],
        "patch_output": result.stdout,
    }


def cfg_test_contract(source: Path, patch: Path) -> dict[str, Any]:
    data = (source / TARGET).read_bytes()
    if data.count(TEST_FILE_MARKER) != 1:
        raise RuntimeError("expected exactly one top-level cfg(test) module")
    if b"#[cfg(test)]\n    suppress_slot_spawn: bool" not in data:
        raise RuntimeError("spawn guard field is not cfg(test)-excluded")
    if b"#[cfg(test)]\n        if self.suppress_slot_spawn" not in data:
        raise RuntimeError("spawn guard branch is not cfg(test)-excluded")
    if b"#[cfg(test)]\nimpl TuiSession" not in data or b"fn suppress_slot_spawn(&mut self)" not in data:
        raise RuntimeError("test helper impl is not cfg(test)-only")
    patch_text = patch.read_text()
    forbidden = ["Cargo.toml", "Cargo.lock", "vendor/", "src/lib.rs"]
    if any(token in patch_text for token in forbidden):
        raise RuntimeError("overlay patch mentions an out-of-scope production path")
    return {
        "verified": True,
        "source_path": TARGET,
        "guard_field_cfg_test": True,
        "guard_branch_cfg_test": True,
        "helper_impl_cfg_test": True,
        "production_source_hash": ORIGINAL_SHA256,
        "derived_source_hash": DERIVED_SHA256,
        "non_test_source_modified": False,
        "basis": "exact hash-bound c0709aba..f9675b66 diff; only cfg(test)-excluded guard and fixture assertions",
    }


def verify_locks(source: Path, manifest: dict[str, Any], phase: str) -> dict[str, Any]:
    verified: dict[str, Any] = {}
    for relative, expected in manifest["locks"].items():
        path = source / relative
        if not path.is_file() or path.is_symlink():
            raise RuntimeError(f"{phase}: manifest lock is not a regular file: {relative}")
        data = path.read_bytes()
        actual = {"bytes": len(data), "sha256": sha256_bytes(data)}
        wanted = {"bytes": expected["bytes"], "sha256": expected["sha256"]}
        if actual != wanted:
            raise RuntimeError(f"{phase}: lock identity mismatch: {relative}: {actual}")
        verified[relative] = actual
    return {"phase": phase, "verified": verified}


def run_tests(source: Path, output: Path, timeout: int) -> dict[str, Any]:
    env = dict(os.environ)
    for name in ("LIVE", "BOT_VAULT_PASS", "BOT_VAULT", "BOT_CACHE", "BOT_CACHE_DIR", "BOT_SERVER", "BOT_HOST"):
        env.pop(name, None)
    env["BOT_MEMORY_OWNER_CAPTURE"] = "0"
    env["BOT_CPU"] = "1"
    env["CARGO_TARGET_DIR"] = str(output / "target")
    command = [
        "cargo", "test", "--offline", "--locked", "-p", "tui",
        "--features", "memory-profile-no-alloc,memory-owner-capture",
        "--", "--test-threads=1",
    ]
    log = output / "tui-tests.log"
    timed_out = False
    with log.open("w", encoding="utf-8") as stream:
        process = subprocess.Popen(
            command,
            cwd=source,
            env=env,
            stdout=stream,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    return {
        "command": command,
        "exit": process.returncode,
        "timed_out": timed_out,
        "timeout_s": timeout,
        "log": str(log),
        "log_sha256": sha256_file(log),
        "log_bytes": log.stat().st_size,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--patch", type=Path, default=Path(__file__).with_name("tui-test-only.patch"))
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--run-tests", action="store_true")
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    if output.exists():
        print(json.dumps({"prepared": False, "error": "output-dir must be fresh"}), file=sys.stderr)
        return 2
    output.mkdir(parents=True)
    try:
        manifest = json.loads(args.manifest.read_text())
        with tempfile.TemporaryDirectory(prefix="tui-test-overlay-") as temporary:
            source, identity = extract_and_verify(args.archive.resolve(), manifest, Path(temporary))
            patch_receipt = apply_patch(source, args.patch.resolve())
            contract = cfg_test_contract(source, args.patch.resolve())
            lock_receipts = [verify_locks(source, manifest, "after-stage")]
            test_receipt = run_tests(source, output, args.timeout) if args.run_tests else None
            if test_receipt is not None:
                lock_receipts.append(verify_locks(source, manifest, "after-tests"))
            failed = test_receipt is not None and (test_receipt["exit"] != 0 or test_receipt["timed_out"])
            # Retain failures for inspection; success is retained only on request.
            if failed or os.environ.get("KEEP_TUI_OVERLAY_TREE") == "1":
                kept = output / "derived-source"
                shutil.copytree(source, kept, symlinks=True)
                identity["derived_tree"] = str(kept)
            receipt = {
                "schema": "direct-owner-tui-test-overlay-v1",
                "prepared": True,
                "archive_is_immutable_input": True,
                "archive": identity["archive"],
                "source_member_count": identity["source_member_count"],
                "original_member_hashes_verified": True,
                "patch": patch_receipt,
                "cfg_test_contract": contract,
                "locks": lock_receipts,
                "tests": test_receipt,
                "linux_qualified": False,
                "live_qualified": False,
                "limitations": [
                    "This is a test-only derived tree, never a production archive replacement.",
                    "No native Linux or live qualification is claimed.",
                    "The original frozen archive and manifest are not modified.",
                ],
            }
        (output / "overlay-receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
        print(json.dumps(receipt, indent=2, sort_keys=True))
        return 0 if test_receipt is None or test_receipt["exit"] == 0 else 1
    except (OSError, RuntimeError, KeyError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        (output / "overlay-receipt.json").write_text(json.dumps({"prepared": False, "error": str(error)}, indent=2) + "\n")
        print(json.dumps({"prepared": False, "error": str(error)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
