#!/usr/bin/env python3
"""Prepare, but never execute, the direct-owner live release contract.

This tool reads only public/provenance artifacts and root-supplied admission
receipts. It never reads credentials, account names, vaults, servers, caches,
or starts a process. Missing or mismatched identity/qualification is fatal.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

SCHEMA = "direct-owner-live-preparation-v1"
ARCHIVE_SHA256 = "2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d"
HOST_COMMIT = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf"
CLIENT_COMMIT = "3456edc8dabf7b25ada78110ffa56327af9f67a4"
CONTROLLER_COMMIT = "e707e2d"
NAV_PACK_SHA256 = "2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30"

# These are release-contract values, not measured results.
CONTRACT = {
    "n": 1,
    "pty": [120, 40],
    "warmup_s": 30,
    "observe_s": 120,
    "after_stop_s": 60,
    "owner_requests": {"A": 30, "B": 90, "C_teardown": 30},
    "poll_interval_s": 0.5,
    "runtime_limits": {
        "mem_available_floor_bytes": 256 * 1024**2,
        "frontend_rss_ceiling_bytes": 512 * 1024**2,
        "output_ceiling_bytes": 64 * 1024**2,
        "wall_ceiling_s": 360,
    },
    "preflight": {
        "mem_available_min_bytes": 768 * 1024**2,
        "free_output_bytes": 256 * 1024**2,
        "swap_required": "disabled",
        "conflicts": "none",
        "server": "root-admitted-healthy",
    },
    "attempts": 1,
}


def fail(message: str) -> None:
    raise ValueError(message)


def regular(path: str, label: str) -> Path:
    if not isinstance(path, str) or not path or path.startswith("<") or "TODO" in path:
        fail(f"{label}: explicit non-placeholder path required")
    p = Path(path)
    if not p.is_absolute() or not p.is_file() or p.is_symlink():
        fail(f"{label}: absolute regular file required")
    return p


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def obj(path: Path, label: str) -> dict[str, Any]:
    value: Any
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"{label}: unreadable JSON: {exc}")
    if not isinstance(value, dict):
        fail(f"{label}: JSON object required")
    return value


def verify_source(archive: Path, manifest: Path) -> dict[str, Any]:
    if digest(archive) != ARCHIVE_SHA256:
        fail("frozen source archive hash mismatch")
    data = obj(manifest, "frozen source manifest")
    archive_info = data.get("archive")
    provenance = data.get("provenance")
    if not isinstance(archive_info, dict) or archive_info.get("sha256") != ARCHIVE_SHA256:
        fail("frozen source manifest archive identity mismatch")
    if not isinstance(provenance, dict) or provenance.get("host_original") != HOST_COMMIT or provenance.get("client_original") != CLIENT_COMMIT:
        fail("frozen source manifest H/C identity mismatch")
    return {"archive_sha256": ARCHIVE_SHA256, "host_commit": HOST_COMMIT,
            "client_commit": CLIENT_COMMIT}


def verify_qualification(path: Path, source: dict[str, Any]) -> dict[str, Any]:
    q = obj(path, "native qualification")
    if q.get("schema") != "direct-owner-native-generated-qualification-v1":
        fail("native qualification schema mismatch")
    if q.get("linux_generated_qualified") is not True:
        fail("native Linux generated qualification is missing or false")
    if q.get("live_qualified") is not False or q.get("frontend_launched") is not False:
        fail("qualification receipt has an invalid live boundary")
    if q.get("source_verified") is not True or q.get("locks_unchanged") is not True:
        fail("qualification receipt lacks source/lock proof")
    return {"schema": q["schema"], "linux_generated_qualified": True,
            "live_qualified": False, "source": source}


def verify_build(manifest: Path, binary: Path) -> dict[str, Any]:
    build = obj(manifest, "direct-owner build manifest")
    if build.get("hostCommit") != HOST_COMMIT or build.get("clientCommit") != CLIENT_COMMIT:
        fail("build manifest H/C identity mismatch")
    features = build.get("features")
    if not isinstance(features, dict):
        fail("build manifest features object is required")
    text = json.dumps(features, sort_keys=True)
    if "memory-owner-capture" not in text or "snapshot-dedup" in text:
        fail("build manifest must prove memory-owner-capture and reject snapshot-dedup")
    binaries = build.get("binaries")
    candidate = binaries.get("candidate_tui_play") if isinstance(binaries, dict) else None
    expected = candidate.get("sha256") if isinstance(candidate, dict) else None
    if not isinstance(expected, str) or digest(binary) != expected:
        fail("direct-owner binary does not match build manifest")
    return {"manifest_sha256": digest(manifest), "binary_sha256": expected,
            "host_commit": HOST_COMMIT, "client_commit": CLIENT_COMMIT}


def verify_controller(path: Path, install_manifest: Path) -> dict[str, Any]:
    controller = obj(install_manifest, "N1 controller install manifest")
    if controller.get("review_commit") != CONTROLLER_COMMIT:
        fail("N1 controller review identity mismatch")
    return {"controller": str(path), "controller_manifest_sha256": digest(install_manifest),
            "review_commit": CONTROLLER_COMMIT}


def build_release_contract(args: argparse.Namespace) -> dict[str, Any]:
    archive = regular(args.source_archive, "source archive")
    source_manifest = regular(args.source_manifest, "source manifest")
    qualification = regular(args.qualification, "qualification")
    build_manifest = regular(args.build_manifest, "build manifest")
    binary = regular(args.binary, "binary")
    controller = regular(args.controller, "controller")
    controller_manifest = regular(args.controller_manifest, "controller manifest")
    nav_pack = regular(args.nav_pack, "nav pack")
    source = verify_source(archive, source_manifest)
    qual = verify_qualification(qualification, source)
    build = verify_build(build_manifest, binary)
    ctl = verify_controller(controller, controller_manifest)
    if digest(nav_pack) != NAV_PACK_SHA256:
        fail("nav pack hash mismatch")
    return {
        "schema": SCHEMA,
        "releaseable": False,
        "execution_attempted": False,
        "contract": CONTRACT,
        "identities": {"source": source, "build": build, "controller": ctl,
                       "nav_pack_sha256": NAV_PACK_SHA256},
        "inputs": {"source_archive": str(archive), "source_manifest": str(source_manifest),
                   "qualification": str(qualification), "build_manifest": str(build_manifest),
                   "binary": str(binary), "controller": str(controller),
                   "controller_manifest": str(controller_manifest), "nav_pack": str(nav_pack)},
        "root_admissions_required": [
            "exact disposable account/owner fixture and private login binding",
            "cache snapshot/version/file hashes and canonical unpack root",
            "server PID/start identity/health and public server artifact hashes",
            "host MemAvailable/free-output/no-swap/conflict receipts",
            "one real 120x40 PTY and existing e707e2d account-selection/seed contract",
        ],
        "validator_boundary": {
            "protocol_validator": "docs/memory/validate_direct_owner_capture.py",
            "native_qualified": False,
            "rss_reconciliation": False,
            "account_admission": "separate root receipt; not inferred from JSONL",
        },
    }


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("source_archive", "source_manifest", "qualification", "build_manifest",
                 "binary", "controller", "controller_manifest", "nav_pack"):
        p.add_argument("--" + name.replace("_", "-"), required=True)
    p.add_argument("--output", required=True)
    args = p.parse_args(argv)
    try:
        result = build_release_contract(args)
        output = Path(args.output)
        if not output.is_absolute() or output.exists() or output.is_symlink():
            fail("output must be a new absolute regular-file path")
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("x") as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write("\n")
        print(json.dumps({"status": "prepared_offline", "releaseable": False,
                          "execution_attempted": False, "output": str(output)}, sort_keys=True))
        return 0
    except (OSError, ValueError, TypeError, KeyError) as exc:
        print(json.dumps({"status": "refused", "releaseable": False, "error": str(exc)}, sort_keys=True))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
