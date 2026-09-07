#!/usr/bin/env python3
"""Fail-closed recovery of a checked Windows server observation.

This creates a new host-conditions artifact. It never edits or re-hashes an
original receipt, launch record, or sidecar.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
from pathlib import Path
from typing import Any, Mapping

FILETIME_EPOCH_OFFSET_S = 11_644_473_600
FILETIME_TICKS_PER_SECOND = 10_000_000
_CREATION_RE = re.compile(r"^/Date\((?P<ms>-?\d+)\)/$")


class RecoveryError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_object(path: Path) -> Mapping[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RecoveryError(f"cannot read JSON object {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise RecoveryError(f"JSON value is not an object: {path}")
    return value


def creation_date_to_filetime(value: Any) -> int:
    """Convert WMI /Date(milliseconds since Unix epoch)/ exactly to 100ns ticks."""
    if not isinstance(value, str):
        raise RecoveryError("CreationDate must be a string")
    match = _CREATION_RE.fullmatch(value)
    if not match:
        raise RecoveryError(f"invalid WMI CreationDate: {value!r}")
    milliseconds = int(match.group("ms"))
    return (FILETIME_EPOCH_OFFSET_S * FILETIME_TICKS_PER_SECOND) + milliseconds * 10_000


def identity_filetime(identity: Mapping[str, Any]) -> int:
    value = identity.get("start_identity")
    prefix = "windows_creation_filetime:"
    if not isinstance(value, str) or not value.startswith(prefix):
        raise RecoveryError("server identity lacks Windows creation FILETIME")
    ticks = value[len(prefix):]
    if not ticks.isdigit():
        raise RecoveryError("server identity FILETIME is not an unsigned integer")
    return int(ticks)


def recover_conditions(
    preflight: Mapping[str, Any],
    server_identity: Mapping[str, Any],
    original_conditions: Mapping[str, Any],
    *,
    preflight_path: Path,
    server_identity_path: Path,
    original_conditions_path: Path,
    expected_name: str = "node.exe",
    expected_session: int = 2,
) -> dict[str, Any]:
    native = preflight.get("native_preflight")
    if not isinstance(native, dict):
        raise RecoveryError("preflight.native_preflight is missing")
    processes = native.get("processes")
    if not isinstance(processes, list):
        raise RecoveryError("preflight.native_preflight.processes is missing")
    pid = server_identity.get("pid")
    if type(pid) is not int or pid <= 0:
        raise RecoveryError("server identity pid is invalid")
    expected_ticks = identity_filetime(server_identity)
    matches = []
    for process in processes:
        if not isinstance(process, dict):
            continue
        if process.get("ProcessId") != pid:
            continue
        if process.get("Name") != expected_name or process.get("SessionId") != expected_session:
            continue
        try:
            ticks = creation_date_to_filetime(process.get("CreationDate"))
        except RecoveryError:
            continue
        # WMI's /Date(...) value is millisecond precision; Win32 identity is
        # 100-ns precision. Require equality at the precision of the row, not
        # a fabricated exact remainder.
        if ticks // 10_000 == expected_ticks // 10_000:
            matches.append(process)
    if len(matches) != 1:
        raise RecoveryError(f"server process match is not unique: {len(matches)} matches")

    output = copy.deepcopy(dict(original_conditions))
    output_native = output.get("native_preflight")
    if not isinstance(output_native, dict):
        raise RecoveryError("original conditions.native_preflight is missing")
    if isinstance(output_native.get("server"), dict):
        raise RecoveryError("refusing to replace an existing server observation")
    output_native["server"] = copy.deepcopy(matches[0])
    source_paths = [preflight_path, server_identity_path, original_conditions_path]
    output["provenance_recovery"] = {
        "status": "reconstructed",
        "not_original_artifact": True,
        "method": "unique_process_row_pid_name_session_exact_creation_filetime",
        "source_artifacts": [
            {"path": str(path.resolve()), "sha256": sha256_file(path)} for path in source_paths
        ],
        "matched_server": {
            "pid": pid,
            "name": expected_name,
            "session_id": expected_session,
            "creation_filetime": expected_ticks,
            "creation_match_precision": "milliseconds (WMI row); identity retained at 100ns",
        },
        "performance_acceptance": False,
    }
    return output


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", type=Path, required=True)
    parser.add_argument("--server-identity", type=Path, required=True)
    parser.add_argument("--original-conditions", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--name", default="node.exe")
    parser.add_argument("--session", type=int, default=2)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        result = recover_conditions(
            load_object(args.preflight), load_object(args.server_identity), load_object(args.original_conditions),
            preflight_path=args.preflight.resolve(strict=True),
            server_identity_path=args.server_identity.resolve(strict=True),
            original_conditions_path=args.original_conditions.resolve(strict=True),
            expected_name=args.name, expected_session=args.session,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as stream:
            json.dump(result, stream, indent=2, allow_nan=False)
            stream.write("\n")
    except (OSError, RecoveryError, TypeError, ValueError) as exc:
        print(f"recovery unavailable: {exc}")
        return 2
    print(f"reconstructed host conditions: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
