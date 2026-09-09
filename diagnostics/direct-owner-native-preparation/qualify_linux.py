#!/usr/bin/env python3
"""Build and qualify the frozen direct-owner diagnostic on native Linux.

This script never launches a frontend, account, cache, server, or live test. Its
Linux verdict covers only frozen-source admission, compilation, generated owner
seams, observer allocation/thread CPU, existing managed guard/cleanup fixtures,
and generated protocol validation.
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
import stat
import subprocess
import sys
import tarfile
import time
from typing import Any

ARCHIVE_ROOT = "direct-owner-source"
OBSERVER_THREAD_CPU_LIMIT_NS = 5_000_000
STEP_TIMEOUT_SECONDS = 900


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def mode_string(path: Path) -> str:
    status = path.lstat()
    if stat.S_ISLNK(status.st_mode):
        return "120000"
    if stat.S_ISREG(status.st_mode):
        return "100755" if status.st_mode & 0o111 else "100644"
    raise RuntimeError(f"unsupported extracted member type: {path}")


def safe_member_path(name: str) -> Path:
    pure = PurePosixPath(name)
    if pure.is_absolute() or ".." in pure.parts or not pure.parts or pure.parts[0] != ARCHIVE_ROOT:
        raise RuntimeError(f"unsafe or unexpected archive path: {name}")
    return Path(*pure.parts)


def extract_archive(archive_path: Path, output: Path) -> Path:
    extraction = output / "extracted"
    extraction.mkdir()
    with tarfile.open(archive_path, mode="r:gz") as archive:
        for member in archive:
            relative = safe_member_path(member.name)
            destination = extraction / relative
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
                destination.chmod(member.mode)
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            if member.issym():
                target = PurePosixPath(member.linkname)
                if target.is_absolute() or ".." in target.parts:
                    raise RuntimeError(f"unsafe symlink target: {member.name} -> {member.linkname}")
                destination.symlink_to(member.linkname)
            elif member.isfile():
                source = archive.extractfile(member)
                if source is None:
                    raise RuntimeError(f"cannot read archive member: {member.name}")
                with destination.open("xb") as stream:
                    shutil.copyfileobj(source, stream, length=1024 * 1024)
                destination.chmod(member.mode)
            else:
                raise RuntimeError(f"unsupported archive member type: {member.name}")
    return extraction / ARCHIVE_ROOT


def verify_source(source: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    expected = {row["path"]: row for row in manifest["source_members"]}
    actual_paths = []
    for path in source.rglob("*"):
        if path.is_file() or path.is_symlink():
            actual_paths.append(path.relative_to(source).as_posix())
    if set(actual_paths) != set(expected):
        missing = sorted(set(expected) - set(actual_paths))
        extra = sorted(set(actual_paths) - set(expected))
        raise RuntimeError(f"source member set mismatch: missing={missing[:3]} extra={extra[:3]}")
    for relative in sorted(expected):
        path = source / relative
        row = expected[relative]
        data = os.readlink(path).encode() if path.is_symlink() else path.read_bytes()
        if len(data) != row["bytes"] or hashlib.sha256(data).hexdigest() != row["sha256"]:
            raise RuntimeError(f"source member identity mismatch: {relative}")
        if mode_string(path) != row["mode"]:
            raise RuntimeError(f"source member mode mismatch: {relative}")
    locks = {}
    for relative, expected_lock in manifest["locks"].items():
        actual_lock = {
            "bytes": (source / relative).stat().st_size,
            "sha256": file_sha256(source / relative),
        }
        if actual_lock != expected_lock:
            raise RuntimeError(f"lockfile mismatch: {relative}")
        locks[relative] = actual_lock
    return {"verified": True, "source_member_count": len(expected), "locks": locks}


def tool_output(command: list[str], cwd: Path) -> dict[str, Any]:
    result = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return {"command": command, "exit": result.returncode, "output": result.stdout}


def executable_identity(command: str) -> dict[str, Any]:
    found = shutil.which(command)
    if found is None:
        raise RuntimeError(f"required tool is not executable: {command}")
    invoked = Path(found).absolute()
    resolved = invoked.resolve(strict=True)
    return {
        "command": command,
        "invoked_path": str(invoked),
        "symlink_target": os.readlink(invoked) if invoked.is_symlink() else None,
        "resolved_path": str(resolved),
        "bytes": resolved.stat().st_size,
        "sha256": file_sha256(resolved),
    }


def safe_environment() -> dict[str, str]:
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
    ):
        env.pop(name, None)
    env["BOT_MEMORY_OWNER_CAPTURE"] = "0"
    env["BOT_CPU"] = "1"
    return env


def run_step(
    name: str,
    command: list[str],
    cwd: Path,
    output: Path,
    env: dict[str, str],
    timeout: int = STEP_TIMEOUT_SECONDS,
) -> dict[str, Any]:
    log_path = output / "logs" / f"{name}.log"
    log_path.parent.mkdir(exist_ok=True)
    started = time.monotonic()
    exit_code: int | str
    timed_out = False
    with log_path.open("x") as log:
        log.write(json.dumps({"cwd": str(cwd), "command": command, "timeout_s": timeout}) + "\n")
        log.flush()
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            exit_code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            exit_code = "timeout"
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
    return {
        "name": name,
        "cwd": str(cwd),
        "command": command,
        "exit": exit_code,
        "timed_out": timed_out,
        "wall_s": time.monotonic() - started,
        "log": str(log_path),
        "log_bytes": log_path.stat().st_size,
        "log_sha256": file_sha256(log_path),
    }


def host_target(rustc_vv: str) -> str:
    for line in rustc_vv.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    raise RuntimeError("rustc -vV did not report a host target")


def generated_observer(log_path: Path) -> dict[str, Any]:
    matches = re.findall(r"GENERATED_OBSERVER (\{[^\n]+\})", log_path.read_text(errors="replace"))
    if len(matches) != 1:
        raise RuntimeError(f"expected one generated observer record, found {len(matches)}")
    record = json.loads(matches[0])
    criteria = {
        "allocations_zero": record.get("allocations") == 0,
        "allocated_bytes_zero": record.get("allocated_bytes") == 0,
        "thread_cpu_within_fragment_budget": type(record.get("thread_cpu_ns")) is int
        and 0 <= record["thread_cpu_ns"] <= OBSERVER_THREAD_CPU_LIMIT_NS,
        "scratch_within_512kib": type(record.get("scratch_reserved_bytes")) is int
        and 0 <= record["scratch_reserved_bytes"] <= 524288,
        "fixture_self_declares_no_native_verdict": record.get("native_qualified") is False,
    }
    return {
        "record": record,
        "criteria": criteria,
        "qualified": all(criteria.values()),
        "thread_cpu_limit_ns": OBSERVER_THREAD_CPU_LIMIT_NS,
        "scope": "generated empty-owner pre-observe + nav/COW seam in an isolated test process",
    }


def command_matrix(source: Path, target: str, mode: str, support: Path) -> list[tuple[str, list[str], Path]]:
    common = ["cargo"]
    flags = ["--offline", "--locked", "--target", target]
    feature_pair = "memory-profile-no-alloc,memory-owner-capture"
    matrix = [
        (
            "cargo-check-tui",
            common + ["check", *flags, "-p", "tui", "--features", feature_pair],
            source,
        ),
        (
            "host-play-generated-observer",
            common
            + [
                "test",
                *flags,
                "-p",
                "host-play",
                "--features",
                feature_pair,
                "--test",
                "direct_owner_capture",
                "--",
                "--test-threads=1",
                "--nocapture",
            ],
            source,
        ),
        (
            "protocol-validator-generated",
            [sys.executable, "-m", "unittest", "-v", "test_validate_direct_owner_capture.py"],
            support,
        ),
    ]
    if mode == "source-check":
        return matrix
    return [
        (
            "cargo-feature-tree",
            common
            + [
                "tree",
                "--offline",
                "--locked",
                "--target",
                target,
                "-e",
                "features",
                "-p",
                "tui",
                "--features",
                feature_pair,
            ],
            source,
        ),
        (
            "cargo-build-tui",
            common
            + [
                "build",
                *flags,
                "-p",
                "tui",
                "--bin",
                "tui-play",
                "--features",
                feature_pair,
            ],
            source,
        ),
        (
            "api-owner-lib",
            common
            + ["test", *flags, "-p", "api", "--features", "memory-owner-capture", "--lib", "owner_capture", "--", "--test-threads=1"],
            source,
        ),
        (
            "api-owner-integration",
            common
            + ["test", *flags, "-p", "api", "--features", "memory-owner-capture", "--test", "direct_owner_capture", "--", "--test-threads=1"],
            source,
        ),
        (
            "host-owner-lib",
            common
            + ["test", *flags, "-p", "host", "--features", "memory-owner-capture", "--lib", "owner_capture", "--", "--test-threads=1"],
            source,
        ),
        (
            "script-fingerprint-lib",
            common
            + ["test", *flags, "-p", "script", "--features", "load,memory-owner-capture", "--lib", "fingerprint_owner_capture", "--", "--test-threads=1"],
            source,
        ),
        (
            "script-stop-integration",
            common
            + ["test", *flags, "-p", "script", "--features", "load,memory-owner-capture", "--test", "direct_owner_capture", "--", "--test-threads=1"],
            source,
        ),
        (
            "client-world-owner-lib",
            common
            + ["test", *flags, "-p", "client", "--features", "memory-owner-capture", "--lib", "world_owner_capture", "--", "--test-threads=1"],
            source / "vendor/fr-client-rust",
        ),
        (
            "client-packet-integration",
            common
            + ["test", *flags, "-p", "client", "--features", "memory-owner-capture", "--test", "direct_owner_capture", "--", "--test-threads=1"],
            source / "vendor/fr-client-rust",
        ),
        (
            "host-play-owner-lib",
            common
            + ["test", *flags, "-p", "host-play", "--features", feature_pair, "--lib", "owner_capture", "--", "--test-threads=1"],
            source,
        ),
        *matrix,
        (
            "tui-feature-tests",
            common
            + ["test", *flags, "-p", "tui", "--features", feature_pair, "--", "--test-threads=1"],
            source,
        ),
        (
            "managed-guard-cleanup-generated",
            [
                sys.executable,
                "-m",
                "unittest",
                "-v",
                "test_run_managed_cell.ManagedCellTests.test_invalid_specs_and_actual_launcher_mismatch_precede_launch",
                "test_run_managed_cell.ManagedCellTests.test_early_frontend_fail_stops_collector_no_retry",
                "test_run_managed_cell.ManagedCellTests.test_deadline_cleanup_owned_children_only",
                "test_run_managed_cell.ManagedCellTests.test_identity_mismatch_cleanup_never_signals_foreign_process",
            ],
            source / "docs/memory",
        ),
    ]


def coverage_contract(mode: str) -> dict[str, dict[str, Any]]:
    if mode == "source-check":
        return {
            "frozen_compile": {
                "steps": ["cargo-check-tui"],
                "claim": "offline locked TUI check with memory-profile-no-alloc,memory-owner-capture on the local host target",
            },
            "generated_observer": {
                "steps": ["host-play-generated-observer"],
                "claim": "generated empty-owner pre-observe and nav/COW seam; allocation count, requested bytes, scratch reservation, and thread CPU bound",
            },
            "protocol_validator": {
                "steps": ["protocol-validator-generated"],
                "claim": "reviewed generated positive fixture plus negative protocol/order/cap/file-identity mutations",
            },
        }
    return {
        "feature_graph_and_binary": {
            "steps": ["cargo-feature-tree", "cargo-build-tui", "cargo-check-tui"],
            "claim": "offline locked native tui-play build/check with the exact feature pair; feature graph and binary identities recorded separately",
        },
        "generated_observer": {
            "steps": ["host-play-generated-observer"],
            "claim": "generated empty-owner pre-observe and nav/COW seam; zero allocation/requested bytes, fixed scratch bound, and <=5 ms thread CPU",
        },
        "owner_budget_mailbox_output_guards": {
            "steps": ["api-owner-lib", "host-owner-lib", "host-play-owner-lib"],
            "claim": "focused owner tests cover checked capacity formulas and visit/deadline limits; three-request/full/stale-slot mailbox rejection; fixed COW scratch/phase timing; cumulative 256 KiB owner-output failure receipt",
        },
        "stop_and_cleanup": {
            "steps": ["script-stop-integration", "managed-guard-cleanup-generated"],
            "claim": "generated script Stop clears fingerprint while retaining unknown builder capacity; managed invalid-spec/launcher mismatch pre-launch rejection, early failure without retry, deadline cleanup, and foreign-process identity refusal",
        },
        "protocol_validator": {
            "steps": ["protocol-validator-generated"],
            "claim": "reviewed generated positive fixture plus negative protocol/order/cap/file-identity mutations",
        },
    }


def binary_identity_commands_passed(binary: dict[str, Any] | None) -> bool:
    return binary is not None and all(
        binary.get(name, {}).get("exit") == 0 for name in ("file", "readelf", "ldd")
    )


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--mode", choices=("linux", "source-check"), default="linux")
    args = parser.parse_args()
    archive = args.archive.resolve()
    manifest_path = args.manifest.resolve()
    output = args.output_dir.resolve()
    tool_root = Path(__file__).resolve().parent
    support = tool_root / "support"
    if not output.is_absolute() or output.exists():
        print(json.dumps({"qualified": False, "error": "output-dir must be a fresh absolute path"}), file=sys.stderr)
        return 2
    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir()
    steps: list[dict[str, Any]] = []
    try:
        manifest = json.loads(manifest_path.read_text())
        expected_archive = manifest["archive"]
        actual_archive = {"bytes": archive.stat().st_size, "sha256": file_sha256(archive)}
        if actual_archive != {"bytes": expected_archive["bytes"], "sha256": expected_archive["sha256"]}:
            raise RuntimeError("archive identity differs from manifest")
        if args.mode == "linux" and (sys.platform != "linux" or platform.machine().lower() not in ("x86_64", "amd64")):
            raise RuntimeError("native qualification requires Linux x86_64; no cross-platform substitution")
        source = extract_archive(archive, output)
        source_verification = verify_source(source, manifest)
        for name, expected in manifest["qualification_support"]["members"].items():
            path = support / name
            actual = {"bytes": path.stat().st_size, "sha256": file_sha256(path)}
            if actual != expected:
                raise RuntimeError(f"reviewed qualification support mismatch: {name}")
        source_verification["qualification_support_verified"] = True
        write_json(output / "source-verification.json", source_verification)

        rustc = tool_output(["rustc", "-vV"], source)
        cargo = tool_output(["cargo", "-Vv"], source)
        python = tool_output([sys.executable, "--version"], source)
        if rustc["exit"] or cargo["exit"] or python["exit"]:
            raise RuntimeError("required compiler/tool identity command failed")
        target = host_target(rustc["output"])
        tool_identities = {
            "rustc": executable_identity("rustc"),
            "cargo": executable_identity("cargo"),
            "python": executable_identity(sys.executable),
        }
        if args.mode == "linux":
            tool_identities.update(
                {
                    "file": executable_identity("file"),
                    "readelf": executable_identity("readelf"),
                    "ldd": executable_identity("ldd"),
                }
            )
        environment = {
            "mode": args.mode,
            "platform": platform.platform(),
            "machine": platform.machine(),
            "python": python,
            "rustc": rustc,
            "cargo": cargo,
            "tool_binaries": tool_identities,
            "qualification_script": {
                "path": str(Path(__file__).resolve()),
                "bytes": Path(__file__).stat().st_size,
                "sha256": file_sha256(Path(__file__)),
            },
            "target": target,
            "capture_environment": "BOT_MEMORY_OWNER_CAPTURE=0; generated fixture explicitly enables only its isolated seam",
            "live_environment_removed": True,
        }
        write_json(output / "environment.json", environment)
        env = safe_environment()
        env["CARGO_TARGET_DIR"] = str(output / "target")
        for name, command, cwd in command_matrix(source, target, args.mode, support):
            step = run_step(name, command, cwd, output, env)
            steps.append(step)
            write_json(output / "steps.json", steps)

        observer_step = next(row for row in steps if row["name"] == "host-play-generated-observer")
        observer = generated_observer(Path(observer_step["log"]))
        observer["command_passed"] = observer_step["exit"] == 0
        observer["qualified"] = observer["qualified"] and observer["command_passed"]
        write_json(output / "generated-observer.json", observer)

        locks_after = {
            relative: {"bytes": (source / relative).stat().st_size, "sha256": file_sha256(source / relative)}
            for relative in manifest["locks"]
        }
        lock_unchanged = locks_after == manifest["locks"]
        binary = None
        if args.mode == "linux":
            binary_path = output / "target" / target / "debug" / "tui-play"
            if binary_path.is_file():
                artifact_dir = output / "artifacts"
                artifact_dir.mkdir()
                artifact = artifact_dir / "tui-play"
                shutil.copy2(binary_path, artifact)
                binary = {
                    "path": str(artifact),
                    "bytes": artifact.stat().st_size,
                    "sha256": file_sha256(artifact),
                    "file": tool_output(["file", str(artifact)], output),
                    "readelf": tool_output(["readelf", "-h", str(artifact)], output),
                    "ldd": tool_output(["ldd", str(artifact)], output),
                }
                write_json(output / "binary.json", binary)
        all_steps_passed = all(row["exit"] == 0 for row in steps)
        executed_steps = [row["name"] for row in steps]
        coverage = coverage_contract(args.mode)
        covered_steps = {step for item in coverage.values() for step in item["steps"]}
        coverage_steps_executed = covered_steps.issubset(executed_steps)
        binary_identity_passed = (
            binary_identity_commands_passed(binary) if args.mode == "linux" else None
        )
        linux_generated_qualified = (
            args.mode == "linux"
            and sys.platform == "linux"
            and platform.machine().lower() in ("x86_64", "amd64")
            and all_steps_passed
            and observer["qualified"]
            and lock_unchanged
            and binary_identity_passed is True
            and coverage_steps_executed
        )
        qualification = {
            "schema": "direct-owner-native-generated-qualification-v1",
            "mode": args.mode,
            "all_steps_passed": all_steps_passed,
            "source_verified": True,
            "locks_unchanged": lock_unchanged,
            "generated_observer_qualified": observer["qualified"],
            "executed_steps": executed_steps,
            "coverage_steps_executed": coverage_steps_executed,
            "binary_identity_commands_passed": binary_identity_passed,
            "linux_generated_qualified": linux_generated_qualified,
            "live_qualified": False,
            "frontend_launched": False,
            "accounts_accessed": False,
            "cache_accessed": False,
            "server_accessed": False,
            "runtime_capture_default_off": True,
            "coverage": coverage,
            "limitations": [
                "Generated empty owners do not represent populated native scene cost.",
                "No frontend/live workload, account, cache, or server admission is performed.",
                "The protocol fixture remains synthetic parser evidence and reports native_qualified=false itself.",
                "Root must separately admit live host resources, identities, guards, workload, and one-attempt procedure.",
            ],
        }
        write_json(output / "qualification.json", qualification)
        print(json.dumps(qualification, indent=2, sort_keys=True))
        return 0 if (
            linux_generated_qualified
            if args.mode == "linux"
            else all_steps_passed
            and observer["qualified"]
            and lock_unchanged
            and coverage_steps_executed
        ) else 1
    except (OSError, RuntimeError, KeyError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        write_json(
            output / "qualification.json",
            {
                "schema": "direct-owner-native-generated-qualification-v1",
                "mode": args.mode,
                "linux_generated_qualified": False,
                "live_qualified": False,
                "error": str(error),
                "steps": steps,
            },
        )
        print(json.dumps({"qualified": False, "error": str(error)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
