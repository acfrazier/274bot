#!/usr/bin/env python3
"""Stage and exercise the frozen direct-owner test-only coverage overlay.

The immutable frozen archive is the sole production-source input. This helper
verifies every archived member, applies the already-reviewed TUI test guard and
this coverage-only cfg(test) patch, runs requested offline/locked commands in
owned process groups, and re-verifies source and lock identities afterward.
It never launches a frontend, live test, account, vault, cache, or server.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
from typing import Any

ARCHIVE_ROOT = "direct-owner-source"
EXPECTED_ARCHIVE = {
    "bytes": 5_075_876,
    "sha256": "2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d",
}
EXPECTED_MEMBER_COUNT = 1_124
TUI_PATCH_SHA256 = "3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4"
COVERAGE_PATCH_SHA256 = "dcb84047157f75c7ad21868b72eb7c3434cb818e8ba3e4b732f6b40d435c1b6e"
DERIVED_HASHES = {
    "crates/tui/src/bin.rs": "aee97345c86cff0eb3253ef56b3e64fd123991f57bebbe5fcb750d899839e28c",
    "crates/api/src/owner_capture.rs": "5d91a1a70dcf1b1954931cc168d0344c1b6e010dfb2016e5d01244e71a052a06",
    "crates/host/src/owner_capture.rs": "138a88380bcebc078c571e13be21f382d79e2316fc49b2bc9941ad60c4ea438e",
    "crates/host-play/src/owner_capture.rs": "b758ca4740f3210a411b0b4643d13f6589d98b6ebbadb40bb70bd0901f29ee2a",
    "crates/host-play/src/owner_capture_output.rs": "59421d88f652236c2c1aa4757c3b6c77632a47752ac82a98f8febdae19f33c3e",
    "vendor/fr-client-rust/crates/client/src/core/world_owner_capture.rs": "7b572b26ada4e8135d0513975678106285e8f2a3b3d36d7a5cfe8ca65f765599",
}
TEST_MARKER = b"#[cfg(test)]"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def member_bytes(path: Path) -> bytes:
    return os.readlink(path).encode() if path.is_symlink() else path.read_bytes()


def member_mode(path: Path) -> str:
    if path.is_symlink():
        return "120000"
    if path.is_file():
        return "100755" if path.stat().st_mode & 0o111 else "100644"
    raise RuntimeError(f"unsupported staged member: {path}")


def safe_member(name: str) -> PurePosixPath:
    relative = PurePosixPath(name)
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        raise RuntimeError(f"unsafe archive member: {name}")
    if relative.parts[0] != ARCHIVE_ROOT:
        raise RuntimeError(f"unexpected archive root: {name}")
    return relative


def extract_archive(archive: Path, destination: Path) -> Path:
    source = destination / ARCHIVE_ROOT
    with tarfile.open(archive, "r:gz") as stream:
        for item in stream:
            relative = safe_member(item.name)
            path = destination / relative
            if item.isdir():
                path.mkdir(parents=True, exist_ok=True)
            elif item.isfile():
                path.parent.mkdir(parents=True, exist_ok=True)
                input_stream = stream.extractfile(item)
                if input_stream is None:
                    raise RuntimeError(f"cannot read {item.name}")
                with path.open("xb") as output:
                    shutil.copyfileobj(input_stream, output)
                path.chmod(item.mode)
            elif item.issym():
                target = PurePosixPath(item.linkname)
                if target.is_absolute() or ".." in target.parts:
                    raise RuntimeError(f"unsafe symlink target: {item.name}")
                path.parent.mkdir(parents=True, exist_ok=True)
                path.symlink_to(item.linkname)
            else:
                raise RuntimeError(f"unsupported archive member: {item.name}")
    return source


def expected_members(manifest: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows = {row["path"]: row for row in manifest["source_members"]}
    if len(rows) != EXPECTED_MEMBER_COUNT:
        raise RuntimeError(f"expected {EXPECTED_MEMBER_COUNT} manifest members, got {len(rows)}")
    return rows


def verify_tree(
    source: Path,
    manifest: dict[str, Any],
    derived: dict[str, str] | None = None,
) -> dict[str, Any]:
    expected = expected_members(manifest)
    actual_paths = sorted(
        path.relative_to(source).as_posix()
        for path in source.rglob("*")
        if path.is_file() or path.is_symlink()
    )
    if set(actual_paths) != set(expected):
        missing = sorted(set(expected) - set(actual_paths))
        extra = sorted(set(actual_paths) - set(expected))
        raise RuntimeError(f"source member set mismatch: missing={missing[:3]} extra={extra[:3]}")
    derived = derived or {}
    for relative in actual_paths:
        path = source / relative
        data = member_bytes(path)
        row = expected[relative]
        wanted_hash = derived.get(relative, row["sha256"])
        wanted_bytes = len(data) if relative in derived else row["bytes"]
        if (
            len(data) != wanted_bytes
            or hashlib.sha256(data).hexdigest() != wanted_hash
            or member_mode(path) != row["mode"]
        ):
            raise RuntimeError(f"source member identity mismatch: {relative}")
    return {
        "source_member_count": len(actual_paths),
        "original_members_verified": len(actual_paths) - len(derived),
        "derived_members_verified": sorted(derived),
    }


def verify_locks(source: Path, manifest: dict[str, Any], phase: str) -> dict[str, Any]:
    actual = {}
    for relative, wanted in manifest["locks"].items():
        path = source / relative
        if not path.is_file() or path.is_symlink():
            raise RuntimeError(f"{phase}: lock is not a regular file: {relative}")
        row = {"bytes": path.stat().st_size, "sha256": sha256_file(path)}
        if row != wanted:
            raise RuntimeError(f"{phase}: lock identity mismatch: {relative}")
        actual[relative] = row
    return {"phase": phase, "verified": actual}


def apply_patch(source: Path, patch: Path, wanted_hash: str, name: str) -> dict[str, Any]:
    actual_hash = sha256_file(patch)
    if actual_hash != wanted_hash:
        raise RuntimeError(f"{name} patch identity mismatch: {actual_hash}")
    command = ["patch", "--batch", "--forward", "--strip=1", "--input", str(patch)]
    result = subprocess.run(
        command,
        cwd=source,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"{name} patch failed ({result.returncode}): {result.stdout}")
    return {
        "name": name,
        "path": str(patch),
        "sha256": actual_hash,
        "command": command,
        "output": result.stdout,
    }


def verify_test_only_contract(
    source: Path,
    manifest: dict[str, Any],
    coverage_patch: Path,
    tui_patch: Path,
    original_prefixes: dict[str, bytes],
) -> dict[str, Any]:
    expected = expected_members(manifest)
    for relative, derived_hash in DERIVED_HASHES.items():
        data = (source / relative).read_bytes()
        if hashlib.sha256(data).hexdigest() != derived_hash:
            raise RuntimeError(f"derived hash mismatch: {relative}")
        original = expected[relative]
        if relative == "crates/tui/src/bin.rs":
            if b"#[cfg(test)]\n    suppress_slot_spawn: bool" not in data:
                raise RuntimeError("TUI guard field is not cfg(test)-only")
            if b"#[cfg(test)]\n        if self.suppress_slot_spawn" not in data:
                raise RuntimeError("TUI spawn branch is not cfg(test)-only")
        else:
            marker = data.find(TEST_MARKER)
            if marker < 0:
                raise RuntimeError(f"missing cfg(test) marker: {relative}")
            if data[:marker] != original_prefixes[relative]:
                raise RuntimeError(f"non-test prefix changed: {relative}")
        if member_mode(source / relative) != original["mode"]:
            raise RuntimeError(f"mode changed in test overlay: {relative}")
    coverage_text = coverage_patch.read_text()
    tui_text = tui_patch.read_text()
    required = set(DERIVED_HASHES) - {"crates/tui/src/bin.rs"}
    for relative in required:
        if f"a/{relative}" not in coverage_text or f"b/{relative}" not in coverage_text:
            raise RuntimeError(f"coverage patch does not bind both labels: {relative}")
    forbidden = ["Cargo.toml", "Cargo.lock", "src/lib.rs", "src/memory.rs"]
    if any(token in coverage_text for token in forbidden):
        raise RuntimeError("coverage patch mentions a production or lock path")
    if "crates/tui/src/bin.rs" not in tui_text:
        raise RuntimeError("reviewed TUI patch path missing")
    return {
        "verified": True,
        "changed_paths": sorted(DERIVED_HASHES),
        "coverage_changes_inside_existing_cfg_test_modules": True,
        "tui_guard_cfg_test_only": True,
        "non_test_behavior_change_authorized": False,
        "derived_hashes": DERIVED_HASHES,
    }


def safe_environment(target: Path) -> dict[str, str]:
    env = dict(os.environ)
    for name in (
        "LIVE",
        "BOT_VAULT_PASS",
        "BOT_VAULT",
        "BOT_CACHE",
        "BOT_CACHE_DIR",
        "BOT_SERVER",
        "BOT_HOST",
        "BOT_MEMORY_N",
        "R274_TEST_FORCE_NO_GPU",
    ):
        env.pop(name, None)
    env["BOT_MEMORY_OWNER_CAPTURE"] = "0"
    env["BOT_CPU"] = "1"
    env["CARGO_TARGET_DIR"] = str(target)
    return env


def process_group_exists(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def stop_group(pgid: int) -> dict[str, Any]:
    actions = []
    if process_group_exists(pgid):
        os.killpg(pgid, signal.SIGTERM)
        actions.append("SIGTERM")
        deadline = time.monotonic() + 10
        while process_group_exists(pgid) and time.monotonic() < deadline:
            time.sleep(0.05)
        if process_group_exists(pgid):
            os.killpg(pgid, signal.SIGKILL)
            actions.append("SIGKILL")
            deadline = time.monotonic() + 10
            while process_group_exists(pgid) and time.monotonic() < deadline:
                time.sleep(0.05)
    return {"signals": actions, "group_absent": not process_group_exists(pgid)}


def run_step(
    name: str,
    command: list[str],
    cwd: Path,
    output: Path,
    env: dict[str, str],
    timeout_s: int,
) -> dict[str, Any]:
    log = output / "logs" / f"{name}.log"
    log.parent.mkdir(exist_ok=True)
    started = time.monotonic()
    timed_out = False
    with log.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps({"cwd": str(cwd), "command": command, "timeout_s": timeout_s}) + "\n")
        stream.flush()
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=env,
            stdout=stream,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        try:
            process.wait(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            timed_out = True
            cleanup = stop_group(process.pid)
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        else:
            cleanup = stop_group(process.pid)
    text = log.read_text(errors="replace")
    return {
        "name": name,
        "cwd": str(cwd),
        "command": command,
        "exit": process.returncode,
        "timed_out": timed_out,
        "timeout_s": timeout_s,
        "wall_s": time.monotonic() - started,
        "cleanup": cleanup,
        "environment": {
            "BOT_MEMORY_OWNER_CAPTURE": env.get("BOT_MEMORY_OWNER_CAPTURE"),
            "BOT_CPU": env.get("BOT_CPU"),
            "R274_TEST_FORCE_NO_GPU": env.get("R274_TEST_FORCE_NO_GPU"),
            "CARGO_TARGET_DIR": env.get("CARGO_TARGET_DIR"),
            "removed_live_inputs": all(
                name not in env
                for name in (
                    "LIVE",
                    "BOT_VAULT_PASS",
                    "BOT_VAULT",
                    "BOT_CACHE",
                    "BOT_CACHE_DIR",
                    "BOT_SERVER",
                    "BOT_HOST",
                    "BOT_MEMORY_N",
                )
            ),
        },
        "log": str(log),
        "log_bytes": log.stat().st_size,
        "log_sha256": sha256_file(log),
        "test_results": re.findall(r"test result: [^\n]+", text),
        "failures": re.findall(r"^    ([A-Za-z0-9_:]+)$", text, flags=re.MULTILINE),
    }


def gap_matrix(source: Path) -> list[tuple[str, list[str], Path]]:
    one = ["--", "--test-threads=1"]
    return [
        (
            "gap-api-deadline",
            ["cargo", "test", "--offline", "--locked", "-p", "api", "--features", "memory-owner-capture", "--lib", "deadline_failure_is_latched_and_suppresses_later_rows", *one],
            source,
        ),
        (
            "gap-client-linked-deadline",
            ["cargo", "test", "--offline", "--locked", "-p", "client", "--features", "memory-owner-capture", "--lib", "linked_square_walk_honors_fragment_deadline", *one],
            source / "vendor/fr-client-rust",
        ),
        (
            "gap-host-stale-slot",
            ["cargo", "test", "--offline", "--locked", "-p", "host", "--features", "memory-owner-capture", "--lib", "token_addressed_queue_does_not_deliver_stale_request_to_current_slot", *one],
            source,
        ),
        (
            "gap-host-play-output",
            ["cargo", "test", "--offline", "--locked", "-p", "host-play", "--features", "memory-profile-no-alloc,memory-owner-capture", "--lib", "owner_capture::output::tests", *one],
            source,
        ),
        (
            "gap-host-play-terminal-deadline",
            ["cargo", "test", "--offline", "--locked", "-p", "host-play", "--features", "memory-profile-no-alloc,memory-owner-capture", "--lib", "delayed_fragment_latches_failure_and_terminal_is_unsuccessful", *one],
            source,
        ),
    ]


def feature_off_matrix(source: Path) -> list[tuple[str, list[str], Path]]:
    one = ["--", "--test-threads=1"]
    return [
        ("feature-off-api", ["cargo", "test", "--offline", "--locked", "-p", "api", *one], source),
        ("feature-off-host", ["cargo", "test", "--offline", "--locked", "-p", "host", *one], source),
        ("feature-off-script-load", ["cargo", "test", "--offline", "--locked", "-p", "script", "--features", "load", *one], source),
        ("feature-off-host-play-profile", ["cargo", "test", "--offline", "--locked", "-p", "host-play", "--features", "memory-profile-no-alloc", *one], source),
        ("feature-off-tui-profile", ["cargo", "test", "--offline", "--locked", "-p", "tui", "--features", "memory-profile-no-alloc", *one], source),
        ("feature-off-client-unit", ["cargo", "test", "--offline", "--locked", "-p", "client", "--lib", *one], source / "vendor/fr-client-rust"),
        ("feature-off-client-integration", ["cargo", "test", "--offline", "--locked", "-p", "client", "--tests", *one], source / "vendor/fr-client-rust"),
    ]


def feature_on_matrix(source: Path) -> list[tuple[str, list[str], Path]]:
    one = ["--", "--test-threads=1"]
    client = source / "vendor/fr-client-rust"
    return [
        ("feature-on-api", ["cargo", "test", "--offline", "--locked", "-p", "api", "--features", "memory-owner-capture", *one], source),
        ("feature-on-host", ["cargo", "test", "--offline", "--locked", "-p", "host", "--features", "memory-owner-capture", *one], source),
        ("feature-on-script-load", ["cargo", "test", "--offline", "--locked", "-p", "script", "--features", "load,memory-owner-capture", *one], source),
        ("feature-on-host-play-profile", ["cargo", "test", "--offline", "--locked", "-p", "host-play", "--features", "memory-profile-no-alloc,memory-owner-capture", *one], source),
        ("feature-on-tui-profile", ["cargo", "test", "--offline", "--locked", "-p", "tui", "--features", "memory-profile-no-alloc,memory-owner-capture", *one], source),
        ("feature-on-client-unit", ["cargo", "test", "--offline", "--locked", "-p", "client", "--features", "memory-owner-capture", "--lib", *one], client),
        ("feature-on-client-integration", ["cargo", "test", "--offline", "--locked", "-p", "client", "--features", "memory-owner-capture", "--tests", *one], client),
    ]


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--coverage-patch", type=Path, default=Path(__file__).with_name("coverage-test-only.patch"))
    parser.add_argument("--tui-patch", type=Path, default=Path(__file__).parent.parent / "tui-test-overlay" / "tui-test-only.patch")
    parser.add_argument("--run-gap-tests", action="store_true")
    parser.add_argument("--run-feature-off", action="store_true")
    parser.add_argument("--run-feature-on", action="store_true")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--keep-source", action="store_true")
    args = parser.parse_args()

    output = args.output_dir.resolve()
    if output.exists():
        print(json.dumps({"prepared": False, "error": "output-dir must be fresh"}), file=sys.stderr)
        return 2
    output.mkdir(parents=True)
    steps: list[dict[str, Any]] = []
    try:
        archive = args.archive.resolve()
        manifest_path = args.manifest.resolve()
        coverage_patch = args.coverage_patch.resolve()
        tui_patch = args.tui_patch.resolve()
        manifest = json.loads(manifest_path.read_text())
        actual_archive = {"bytes": archive.stat().st_size, "sha256": sha256_file(archive)}
        manifest_archive = {key: manifest["archive"][key] for key in ("bytes", "sha256")}
        if actual_archive != EXPECTED_ARCHIVE or actual_archive != manifest_archive:
            raise RuntimeError(f"immutable archive identity mismatch: {actual_archive}")
        with tempfile.TemporaryDirectory(prefix="direct-owner-coverage-overlay-") as temporary:
            source = extract_archive(archive, Path(temporary))
            original_identity = verify_tree(source, manifest)
            original_prefixes = {}
            for relative in set(DERIVED_HASHES) - {"crates/tui/src/bin.rs"}:
                data = (source / relative).read_bytes()
                marker = data.find(TEST_MARKER)
                if marker < 0:
                    raise RuntimeError(f"missing original cfg(test) marker: {relative}")
                original_prefixes[relative] = data[:marker]
            patches = [
                apply_patch(source, tui_patch, TUI_PATCH_SHA256, "reviewed-tui-test-guard"),
                apply_patch(source, coverage_patch, COVERAGE_PATCH_SHA256, "coverage-gap-tests"),
            ]
            contract = verify_test_only_contract(
                source,
                manifest,
                coverage_patch,
                tui_patch,
                original_prefixes,
            )
            derived_identity = verify_tree(source, manifest, DERIVED_HASHES)
            locks = [verify_locks(source, manifest, "after-stage")]
            env = safe_environment(output / "target")
            matrix = []
            if args.run_gap_tests:
                matrix.extend(gap_matrix(source))
            if args.run_feature_off:
                matrix.extend(feature_off_matrix(source))
            if args.run_feature_on:
                matrix.extend(feature_on_matrix(source))
            for name, command, cwd in matrix:
                step_env = dict(env)
                if name in {"feature-off-host", "feature-on-host"}:
                    # This suite already forces the CPU renderer internally. A
                    # global BOT_CPU=1 collapses its false->true preference
                    # transition and creates a test-environment-only failure.
                    step_env.pop("BOT_CPU", None)
                    step_env["R274_TEST_FORCE_NO_GPU"] = "1"
                step = run_step(name, command, cwd, output, step_env, args.timeout)
                steps.append(step)
                write_json(output / "steps.json", steps)
            locks.append(verify_locks(source, manifest, "after-tests"))
            post_test_identity = verify_tree(source, manifest, DERIVED_HASHES)
            all_requested_passed = all(
                row["exit"] == 0
                and not row["timed_out"]
                and row["cleanup"]["group_absent"]
                for row in steps
            )
            if args.keep_source or not all_requested_passed:
                kept = output / "derived-source"
                shutil.copytree(source, kept, symlinks=True)
                derived_tree = str(kept)
            else:
                derived_tree = None
            receipt = {
                "schema": "direct-owner-coverage-overlay-v1",
                "prepared": True,
                "helper": {
                    "path": str(Path(__file__).resolve()),
                    "bytes": Path(__file__).stat().st_size,
                    "sha256": sha256_file(Path(__file__)),
                },
                "archive_is_immutable_input": True,
                "archive": actual_archive,
                "manifest": {"path": str(manifest_path), "sha256": sha256_file(manifest_path)},
                "original_identity": original_identity,
                "patches": patches,
                "test_only_contract": contract,
                "derived_identity": derived_identity,
                "post_test_identity": post_test_identity,
                "locks": locks,
                "platform": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
                "steps": steps,
                "all_requested_passed": all_requested_passed,
                "derived_tree": derived_tree,
                "native_linux_qualified": False,
                "live_qualified": False,
                "production_source_modified": False,
                "limitations": [
                    "The derived source exists only for tests and must never enter a live or production build.",
                    "Failing invariant tests expose frozen production behavior; this overlay does not repair it.",
                    "A macOS result is local generated evidence, not native Linux qualification.",
                    "No frontend, account, vault, cache, server, network, or live test is accessed.",
                ],
            }
        write_json(output / "overlay-receipt.json", receipt)
        print(json.dumps(receipt, indent=2, sort_keys=True))
        return 0 if receipt["all_requested_passed"] else 1
    except (OSError, RuntimeError, KeyError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        failure = {
            "schema": "direct-owner-coverage-overlay-v1",
            "prepared": False,
            "error": str(error),
            "steps": steps,
            "native_linux_qualified": False,
            "live_qualified": False,
        }
        write_json(output / "overlay-receipt.json", failure)
        print(json.dumps(failure, indent=2, sort_keys=True), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
