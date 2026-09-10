#!/usr/bin/env python3
"""Portable setup probes of frozen bb267d1 lifecycle fixture. No bot/server/native."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
import traceback

REPO = pathlib.Path(__file__).resolve().parents[3]
COMMIT = "bb267d1c29a6219c9722fbc941fba143dc815c63"
NEEDED = (
    "docs/memory/test_run_managed_cell.py",
    "docs/memory/run_managed_cell.py",
    "docs/memory/managed_receipt.py",
    "docs/memory/build_provenance.py",
    "docs/memory/cache_provenance.py",
    "docs/memory/server_resources.py",
    "docs/memory/process_accounting.py",
    "docs/memory/native_process_sample.py",
    "docs/memory/windows_process_sample.py",
    "docs/memory/windows_process_parent.py",
    "docs/memory/run_diagnostic.py",
    "docs/memory/operator_home.py",
    "docs/memory/heaptrack_capture.py",
)
DIRECT_ADMISSION_BINDINGS = {
    "release_contract",
    "receipt_conflict",
    "receipt_account",
    "receipt_population",
    "receipt_cache",
    "receipt_server_health",
}


def git_show(path: str) -> bytes:
    return subprocess.check_output(["git", "show", f"{COMMIT}:{path}"], cwd=REPO)


def load_module(name: str, path: pathlib.Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def sha256_file(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def record(exc: BaseException) -> dict:
    return {"type": type(exc).__name__, "message": str(exc), "traceback": traceback.format_exc()}


def main() -> int:
    out = {
        "commit": COMMIT,
        "platform": sys.platform,
        "python": sys.version,
        "repo": str(REPO),
        "branch": subprocess.check_output(
            ["git", "branch", "--show-current"], cwd=REPO, text=True
        ).strip(),
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        "findings": [],
        "probes": {},
        "coverage_limits": [
            "macOS cannot execute the Linux-only dummy /proc launcher, Linux process identity format, or collector lifecycle.",
            "preflight remains mocked in this test; production admission temporal/lineage checks are not executed by it.",
            "complete() remains mocked; receipt hashing is not lifecycle evidence.",
        ],
    }
    stage = tempfile.mkdtemp(prefix="lifecycle-escalation-bb267d1-")
    out["stage"] = stage
    try:
        memory = pathlib.Path(stage) / "docs" / "memory"
        memory.mkdir(parents=True)
        missing = []
        for rel in NEEDED:
            dest = pathlib.Path(stage) / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            try:
                dest.write_bytes(git_show(rel))
            except subprocess.CalledProcessError:
                missing.append(rel)
        out["probes"]["extracted_missing"] = missing
        sys.path.insert(0, str(memory))
        rmc = load_module("run_managed_cell", memory / "run_managed_cell.py")
        mr = load_module("managed_receipt", memory / "managed_receipt.py")
        sr = load_module("server_resources", memory / "server_resources.py")
        testmod = load_module("test_run_managed_cell", memory / "test_run_managed_cell.py")

        case = testmod.ManagedCellTests("test_linux_direct_lifecycle_keeps_collector_through_stop_c_and_exit")
        case.setUp()
        try:
            skip = getattr(
                case.test_linux_direct_lifecycle_keeps_collector_through_stop_c_and_exit,
                "__unittest_skip_why__",
                None,
            )
            out["probes"]["linux_skip_decorator"] = {
                "skipUnless_linux": sys.platform.startswith("linux") is False,
                "reason_on_this_platform": "direct lifecycle qualification is Linux-only"
                if not sys.platform.startswith("linux")
                else None,
            }

            result_path = case.fx.root / "direct.json"
            spec_path = result_path.with_suffix(result_path.suffix + ".spec.json")
            cell_id = "direct"
            cell_dir = result_path.with_suffix(result_path.suffix + ".cells") / cell_id
            run_dir = cell_dir / "frontend-run"
            handoff = cell_dir / "frontend-handoff.json"
            spec = case.fx.base_spec(cell_id=cell_id)
            spec.update(
                n=1,
                warmup_s=30,
                observe_s=120,
                teardown_grace_s=60,
                sampler_interval_s=0.5,
                max_wall_s=365,
                process_backend="system",
                requested_backend="none",
                heaptrack=None,
            )
            spec["diagnostic_argv"] = [
                "--binary",
                str(case.fx.binary),
                "--build-manifest",
                str(case.fx.manifest),
                "--build-role",
                "candidate",
                "--no-diagnostics",
                "--sustain",
                "--warmup",
                "30",
                "--observe",
                "120",
                "--direct-owner-capture",
                "--run-dir",
                str(run_dir),
                "--frontend-handoff",
                str(handoff),
                "tui",
                "1",
                "active",
            ]
            spec["launcher_argv"] = [str(case.fx.root / "direct_launcher.py")] + spec["diagnostic_argv"]
            spec["capture_contract"] = {
                "mode": "direct-owner-v1",
                "cell_dir": str(cell_dir.resolve()),
                "run_dir": str(run_dir.resolve()),
                "frontend_handoff_path": str(handoff.resolve()),
                "owned_output_paths": [
                    str(result_path.resolve()),
                    str(spec_path.resolve()),
                    str(cell_dir.resolve()),
                    str(run_dir.resolve()),
                ],
                "warmup_s": 30,
                "observe_s": 120,
                "teardown_grace_s": 60,
                "guard_interval_s": 0.5,
                "handoff_deadline_s": 5,
                "mem_available_floor_bytes": 268435456,
                "frontend_rss_limit_bytes": 536870912,
                "owned_output_limit_bytes": 67108864,
                "frontend_wall_limit_s": 360,
                "outer_wall_limit_s": 365,
                "source_lineage": {"fixture": True},
                "release_contract": str(case.fx.root / "private-root-release.json"),
                "admission_receipts": {
                    name: str(case.fx.root / ("private-" + name + ".json"))
                    for name in ("conflict", "account", "population", "cache", "server_health")
                },
            }

            # Probe 1: frozen spec as written (no cache_dir/unpack_root).
            try:
                rmc.validate_spec(spec)
                out["probes"]["validate_spec_frozen"] = {"ok": True}
            except Exception as exc:
                out["probes"]["validate_spec_frozen"] = {"ok": False, "error": record(exc)}
                out["findings"].append(
                    {
                        "id": "missing_cache_dir_unpack_root",
                        "confirmed": True,
                        "when": "validate_spec of frozen lifecycle spec",
                        "error": type(exc).__name__ + ": " + str(exc),
                    }
                )

            try:
                case._direct_preflight_fixtures(spec)
                out["probes"]["fixtures_frozen"] = {"ok": True}
            except Exception as exc:
                out["probes"]["fixtures_frozen"] = {"ok": False, "error": record(exc)}
                out["findings"].append(
                    {
                        "id": "fixtures_keyerror_without_cache_tree",
                        "confirmed": True,
                        "when": "_direct_preflight_fixtures(spec) before cache roots exist",
                        "error": type(exc).__name__ + ": " + str(exc),
                    }
                )

            live_server = sr.sample_process(case.fx.game_server.pid, timeout=2)
            live_helper = sr.sample_process(case.fx.helper.pid, timeout=2)
            out["probes"]["live_identities"] = {
                "server": live_server.get("start_identity"),
                "helper": live_helper.get("start_identity"),
                "sidecar": json.loads(case.fx.server_identity.read_text()).get("start_identity"),
            }

            # Probe 2: add cache tree + roots, then fixtures (minimal correction path).
            cache, unpack = testmod._make_cache_tree(case.fx.root / "direct-cache")
            corrected = dict(spec)
            corrected["cache_dir"] = str(cache)
            corrected["unpack_root"] = str(unpack)
            try:
                rmc.validate_spec(corrected)
                out["probes"]["validate_spec_with_cache_roots"] = {"ok": True}
            except Exception as exc:
                out["probes"]["validate_spec_with_cache_roots"] = {"ok": False, "error": record(exc)}
                out["findings"].append(
                    {
                        "id": "validate_spec_still_fails_after_cache_roots",
                        "confirmed": True,
                        "error": type(exc).__name__ + ": " + str(exc),
                    }
                )

            try:
                fixtures = case._direct_preflight_fixtures(corrected)
                out["probes"]["fixtures_with_cache_roots"] = {"ok": True}
            except Exception as exc:
                fixtures = None
                out["probes"]["fixtures_with_cache_roots"] = {"ok": False, "error": record(exc)}
                out["findings"].append(
                    {
                        "id": "fixtures_fail_after_cache_roots",
                        "confirmed": True,
                        "error": type(exc).__name__ + ": " + str(exc),
                    }
                )

            if fixtures is not None:
                fake_helper = fixtures["sample"](case.fx.helper.pid, timeout=2)
                fake_server = fixtures["sample"](case.fx.game_server.pid, timeout=2)
                out["probes"]["generated_samples"] = {
                    "server": fake_server,
                    "helper": fake_helper,
                    "server_matches_live": fake_server.get("start_identity")
                    == live_server.get("start_identity"),
                    "helper_matches_live": fake_helper.get("start_identity")
                    == live_helper.get("start_identity"),
                    "server_matches_sidecar": fake_server.get("start_identity")
                    == out["probes"]["live_identities"]["sidecar"],
                }
                if fake_helper.get("start_identity") != live_helper.get("start_identity"):
                    out["findings"].append(
                        {
                            "id": "helper_start_identity_synthetic",
                            "confirmed": True,
                            "when": "fixtures['sample'] for ambient helper",
                            "generated": fake_helper.get("start_identity"),
                            "live": live_helper.get("start_identity"),
                            "note": "9344d75 used sr.sample_process for both server and helper.",
                        }
                    )

                release = json.loads(fixtures["release_path"].read_text())
                now = time.time()
                out["probes"]["expiry"] = {
                    "file_expires_unix_s": release.get("expires_unix_s"),
                    "file_issued_unix_s": release.get("issued_unix_s"),
                    "observation_window": release.get("observation_window"),
                    "now_unix_s": now,
                    "fake_pf_expiry_would_be": now + 60.0,
                    "file_expiry_in_past": float(release.get("expires_unix_s", 0)) <= now,
                }
                if float(release.get("expires_unix_s", 0)) <= now:
                    out["findings"].append(
                        {
                            "id": "release_json_expiry_not_current",
                            "confirmed": True,
                            "when": "generated private-root-release.json contents",
                            "file_expires_unix_s": release.get("expires_unix_s"),
                            "now_unix_s": now,
                            "note": "pre-Popen uses mocked direct_preflight.release_expires_unix_s, not this field.",
                        }
                    )

                bindings = {
                    "release_contract": {
                        "path": str(fixtures["release_path"].resolve()),
                        "sha256": sha256_file(fixtures["release_path"]),
                    },
                    **{
                        "receipt_" + kind: {
                            "path": str(pathlib.Path(path).resolve()),
                            "sha256": sha256_file(pathlib.Path(path)),
                        }
                        for kind, path in corrected["capture_contract"]["admission_receipts"].items()
                    },
                }
                present = []
                for label, entry in bindings.items():
                    path = pathlib.Path(entry["path"])
                    present.append(
                        {
                            "label": label,
                            "exists": path.is_file(),
                            "size": path.stat().st_size if path.is_file() else 0,
                            "sha256": entry["sha256"],
                            "regular_file": path.is_file() and not path.is_symlink(),
                        }
                    )
                out["probes"]["six_files"] = {
                    "keys": sorted(bindings),
                    "exact_set": set(bindings) == DIRECT_ADMISSION_BINDINGS,
                    "files": present,
                }
                try:
                    testmod.bp.recheck_files(bindings)
                    testmod.bp.recheck_files(fixtures["provenance"]["files"])
                    out["probes"]["pre_popen_hash_recheck"] = {"ok": True}
                except Exception as exc:
                    out["probes"]["pre_popen_hash_recheck"] = {"ok": False, "error": record(exc)}
                    out["findings"].append(
                        {
                            "id": "pre_popen_hash_recheck_failed",
                            "confirmed": True,
                            "error": type(exc).__name__ + ": " + str(exc),
                        }
                    )
                try:
                    normalized = mr._validated_bindings(bindings)
                    out["probes"]["create_launch_bindings"] = {
                        "ok": True,
                        "keys": sorted(normalized),
                    }
                except Exception as exc:
                    out["probes"]["create_launch_bindings"] = {"ok": False, "error": record(exc)}
                    out["findings"].append(
                        {
                            "id": "admission_bindings_rejected",
                            "confirmed": True,
                            "error": type(exc).__name__ + ": " + str(exc),
                        }
                    )

                context = fixtures["context"]
                out["probes"]["manifest_cache_context"] = {
                    "cache_content_identity_sha256": context.get("cache_content_identity_sha256"),
                    "cache_snapshot_version": context.get("cache_snapshot_version"),
                    "server_start_identity": context.get("server_start_identity"),
                    "server_executable_basename": context.get("server_executable_basename"),
                    "source_lineage_in_spec": corrected["capture_contract"]["source_lineage"],
                    "manifest_has_source_lineage": "source_lineage"
                    in json.loads(case.fx.manifest.read_text())["candidate"],
                    "cache_snapshot_schema": fixtures["cache"].get("schema"),
                    "cache_file_count": len(fixtures["cache"].get("files") or []),
                }
                live_basename = pathlib.Path(sys.executable).name
                out["probes"]["server_executable_basename"] = {
                    "generated": context.get("server_executable_basename"),
                    "sys_executable_name": live_basename,
                }

            try:
                args = rmc.parse_diagnostic_argv(spec["diagnostic_argv"])
                rmc.require_argv_consistent_with_spec(corrected if fixtures is not None else spec, args, _test_launcher=True)
                out["probes"]["diagnostic_argv"] = {
                    "ok": True,
                    "frontend": args.frontend,
                    "n": args.n,
                    "workload": args.workload,
                    "direct_owner_capture": args.direct_owner_capture,
                }
            except Exception as exc:
                out["probes"]["diagnostic_argv"] = {"ok": False, "error": record(exc)}
                out["findings"].append(
                    {
                        "id": "diagnostic_argv_incompatible",
                        "confirmed": True,
                        "error": type(exc).__name__ + ": " + str(exc),
                    }
                )
        finally:
            case.doCleanups()
    finally:
        shutil.rmtree(stage, ignore_errors=True)
        out["stage_removed"] = True

    result_path = pathlib.Path(__file__).with_name("probe-bb267d1-result.json")
    result_path.write_text(json.dumps(out, indent=2, default=str) + "\n")
    print(json.dumps({"result": str(result_path), "finding_ids": [f["id"] for f in out["findings"]]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
