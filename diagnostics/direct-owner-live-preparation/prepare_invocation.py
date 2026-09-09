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
import re
from pathlib import Path
from typing import Any, NoReturn

SCHEMA = "direct-owner-live-preparation-v1"
ARCHIVE_SHA256 = "2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d"
HOST_COMMIT = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf"
CLIENT_COMMIT = "3456edc8dabf7b25ada78110ffa56327af9f67a4"
HOST_REVIEWED_COMMIT = "3cdc3e4fbeb960fe4c270eb994ce22746787eb2d"
CLIENT_REVIEWED_COMMIT = "5c73a4a27f3d72834c2c2a071668eb197eb39fd9"
CONTROLLER_COMMIT = "e707e2d"
CONTROLLER_SHA256 = "8634665855d87aa93d27f10bc386f87d04be4c522b24e5379174f45cc2f37312"
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


def fail(message: str) -> NoReturn:
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
    if (not isinstance(provenance, dict)
            or provenance.get("host_original") != HOST_COMMIT
            or provenance.get("client_original") != CLIENT_COMMIT
            or provenance.get("host_reviewed") != HOST_REVIEWED_COMMIT
            or provenance.get("client_reviewed") != CLIENT_REVIEWED_COMMIT):
        fail("frozen source manifest H/C identity mismatch")
    return {"archive_sha256": ARCHIVE_SHA256, "host_original": HOST_COMMIT,
            "client_original": CLIENT_COMMIT, "host_reviewed": HOST_REVIEWED_COMMIT,
            "client_reviewed": CLIENT_REVIEWED_COMMIT}


def verify_derivative_admission(path: Path, source: dict[str, Any]) -> dict[str, Any]:
    admission = obj(path, "derivative source admission")
    if admission.get("schema") != "direct-owner-derivative-admission-v1":
        fail("derivative source admission schema mismatch")
    if admission.get("archive_sha256") != source["archive_sha256"]:
        fail("derivative admission archive identity mismatch")
    if admission.get("source_provenance") != source:
        fail("derivative admission provenance mismatch")
    for key in ("checkout_host_commit", "checkout_client_commit"):
        value = admission.get(key)
        if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{40}", value) is None:
            fail(f"derivative admission {key} must be a clean checkout identity")
    if admission.get("host_clean") is not True or admission.get("client_clean") is not True:
        fail("derivative admission requires clean host and client checkouts")
    if admission.get("materialized_from_archive") is not True:
        fail("derivative admission must bind materialization to the frozen archive")
    return {"schema": admission["schema"], "checkout_host_commit": admission["checkout_host_commit"],
            "checkout_client_commit": admission["checkout_client_commit"],
            "host_clean": True, "client_clean": True, "materialized_from_archive": True}


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


def verify_build(manifest: Path, binary: Path, derivative: dict[str, Any]) -> dict[str, Any]:
    build = obj(manifest, "direct-owner build manifest")
    candidate = build.get("candidate")
    if not isinstance(candidate, dict) or candidate.get("commit") != derivative["checkout_host_commit"]:
        fail("build manifest candidate checkout identity mismatch")
    client = candidate.get("client")
    if not isinstance(client, dict) or client.get("commit") != derivative["checkout_client_commit"]:
        fail("build manifest candidate client identity mismatch")
    if (candidate.get("build_exit") != 0 or candidate.get("sources_stable_across_build") is not True
            or not isinstance(candidate.get("branch"), str) or not candidate["branch"]):
        fail("build manifest candidate source/build evidence is not stable")
    source_pre = candidate.get("sources_sha256_pre")
    source_post = candidate.get("sources_sha256_post")
    if (not isinstance(source_pre, str) or re.fullmatch(r"[0-9a-f]{64}", source_pre) is None
            or source_post != source_pre):
        fail("build manifest candidate source digests differ")
    client_sources = client.get("sources_sha256")
    if not isinstance(client_sources, str) or re.fullmatch(r"[0-9a-f]{64}", client_sources) is None:
        fail("build manifest client source digest is invalid")
    features = build.get("features")
    if not isinstance(features, dict):
        fail("build manifest features object is required")
    requested = features.get("requested")
    if not isinstance(requested, str) or not requested:
        fail("build manifest requested features must be a non-empty string")
    requested_values = requested.replace(",", " ").split()
    if ("memory-profile-no-alloc" not in requested_values
            or "memory-owner-capture" not in requested_values
            or features.get("locked") is not True
            or features.get("allocation_counting") is not False
            or features.get("snapshot_dedup") is not False
            or "snapshot-dedup" in requested_values
            or not isinstance(features.get("allocator"), str) or not features["allocator"]):
        fail("build manifest must prove the locked owner-capture feature contract")
    binaries = build.get("binaries")
    binary_entry = binaries.get("candidate_tui_play") if isinstance(binaries, dict) else None
    expected = binary_entry.get("sha256") if isinstance(binary_entry, dict) else None
    if not isinstance(expected, str) or digest(binary) != expected:
        fail("direct-owner binary does not match build manifest")
    recorded_path = binary_entry.get("path") if isinstance(binary_entry, dict) else None
    if not isinstance(recorded_path, str) or Path(recorded_path).resolve() != binary.resolve():
        fail("direct-owner binary path differs from build manifest")
    return {"manifest_sha256": digest(manifest), "binary_sha256": expected,
            "checkout_host_commit": derivative["checkout_host_commit"],
            "checkout_client_commit": derivative["checkout_client_commit"]}


def verify_controller(path: Path, install_manifest: Path) -> dict[str, Any]:
    controller = obj(install_manifest, "N1 controller install manifest")
    if controller.get("review_commit") != CONTROLLER_COMMIT:
        fail("N1 controller review identity mismatch")
    tools = controller.get("only_untracked_controller_tools_changed")
    entry = tools.get("run_current_tui_calibration.py") if isinstance(tools, dict) else None
    if not isinstance(entry, dict) or entry.get("after") != CONTROLLER_SHA256:
        fail("N1 controller install manifest tool identity mismatch")
    if digest(path) != CONTROLLER_SHA256:
        fail("N1 controller digest mismatch")
    return {"controller": str(path), "controller_manifest_sha256": digest(install_manifest),
            "controller_sha256": CONTROLLER_SHA256, "review_commit": CONTROLLER_COMMIT}


def build_release_contract(args: argparse.Namespace) -> dict[str, Any]:
    archive = regular(args.source_archive, "source archive")
    source_manifest = regular(args.source_manifest, "source manifest")
    derivative_admission = regular(args.derivative_admission, "derivative admission")
    qualification = regular(args.qualification, "qualification")
    build_manifest = regular(args.build_manifest, "build manifest")
    binary = regular(args.binary, "binary")
    controller = regular(args.controller, "controller")
    controller_manifest = regular(args.controller_manifest, "controller manifest")
    nav_pack = regular(args.nav_pack, "nav pack")
    source = verify_source(archive, source_manifest)
    derivative = verify_derivative_admission(derivative_admission, source)
    qual = verify_qualification(qualification, source)
    build = verify_build(build_manifest, binary, derivative)
    ctl = verify_controller(controller, controller_manifest)
    if digest(nav_pack) != NAV_PACK_SHA256:
        fail("nav pack hash mismatch")
    return {
        "schema": SCHEMA,
        "releaseable": False,
        "execution_attempted": False,
        "contract": CONTRACT,
        "identities": {"source": source, "derivative": derivative, "build": build, "controller": ctl,
                       "nav_pack_sha256": NAV_PACK_SHA256},
        "inputs": {"source_archive": str(archive), "source_manifest": str(source_manifest),
                   "derivative_admission": str(derivative_admission),
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
    for name in ("source_archive", "source_manifest", "derivative_admission", "qualification", "build_manifest",
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
