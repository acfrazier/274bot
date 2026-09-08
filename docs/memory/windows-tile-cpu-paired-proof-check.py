#!/usr/bin/env python3
"""Recompute the local checks for the native CPU functional pair."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / "diagnostics" / "windows-tile-cpu-proof-20260908"
CELLS = {
    "baseline": "baseline-focused-one-cpu-native-20260908-0215",
    "candidate": "candidate-focused-one-cpu-native-20260908-0215",
}
EXPECTED_ARCHIVES = {
    "baseline": "a39b8a50ff4a83e3b641a780f4ed58a3352a265dc86897e3202ddcb00d4d798a",
    "candidate": "c267376258bef262ad130f5efb8818fddf3c31c1b07d70a86b68762f970294d4",
}


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    result = {"schema": "windows-tile-cpu-paired-proof-check-v1", "cells": {}}
    for role, name in CELLS.items():
        cell = BASE / name
        manifest = load(cell / "archive-manifest.json")
        files = []
        failures = []
        for entry in manifest["files"]:
            relative = Path(*entry["path"].replace("\\", "/").split("/"))
            path = cell / relative
            actual = {"path": entry["path"], "present": path.is_file()}
            if path.is_file():
                actual["length"] = path.stat().st_size
                actual["sha256"] = sha256(path)
                actual["hash_ok"] = actual["sha256"] == entry["sha256"]
                actual["length_ok"] = actual["length"] == entry["length"]
            else:
                actual["hash_ok"] = False
                actual["length_ok"] = False
            if not (actual["present"] and actual["hash_ok"] and actual["length_ok"]):
                failures.append(actual)
            files.append(actual)

        managed = cell / "managed-run"
        bound = load(managed / "native-bound.json")
        host = load(managed / "host-conditions.json")
        completion = load(managed / "completion.json")
        receipt = load(next((managed / "cells").glob("*/receipt.json")))
        match = bound["match_keys"]
        qualification = cell / "raw-run-01" / "samples.qualification.jsonl"
        boundaries = [json.loads(line) for line in qualification.read_text(encoding="utf-8-sig").splitlines() if line]
        result["cells"][role] = {
            "cell_id": manifest["cell_id"],
            "archive_sha256_from_receipt": EXPECTED_ARCHIVES[role],
            "archive_manifest_file_count": len(manifest["files"]),
            "archive_manifest_files_verified": not failures,
            "archive_manifest_failures": failures,
            "completion": {k: completion.get(k) for k in ("exit_code", "status", "completed")},
            "native_bound": {
                "status": bound.get("status"),
                "binding_ok": bound.get("binding_ok"),
                "pair_eligible": bound.get("pair_eligible"),
                "qualified": bound.get("qualification", {}).get("qualified"),
                "missing_match_keys": bound.get("match_keys_missing"),
                "reason": bound.get("reason"),
            },
            "metadata": {
                "requested_backend": host.get("requested_backend"),
                "actual_backend": host.get("backend"),
                "role": host.get("role"),
                "mode": host.get("mode"),
                "active_slots": match.get("n"),
                "renderer_settings": match.get("renderer_settings"),
                "server_configuration": match.get("server_configuration"),
            },
            "qualification_boundaries": [
                {"phase": item.get("phase"), "elapsed_s": item.get("elapsed_s"), "slots": item.get("slots")}
                for item in boundaries
            ],
            "receipt_identity": {
                "source_sha256": bound.get("side_provenance", {}).get("manifest_sources_sha256"),
                "client_sources_sha256": bound.get("side_provenance", {}).get("client_sources_sha256"),
                "binary_sha256": bound.get("binary_sha256"),
                "manifest_build_commit": bound.get("side_provenance", {}).get("manifest_build_commit"),
                "client_commit": bound.get("side_provenance", {}).get("client_commit"),
                "host_commit_checkout": bound.get("side_provenance", {}).get("host_commit_checkout"),
            },
            "file_results": files,
        }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
