#!/usr/bin/env python3
"""Tests for one-cell managed launcher + process accounting orchestration.

Uses real dummy processes and fixture scripts — no live bot/server/networking.
"""
from __future__ import annotations

import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import build_provenance as bp  # noqa: E402
import managed_receipt as mr  # noqa: E402
import run_managed_cell as rmc  # noqa: E402
import server_resources as sr  # noqa: E402

MEMORY = ROOT
ACCOUNTING = MEMORY / "process_accounting.py"


def _sha(path: pathlib.Path) -> str:
    return bp.file_sha256(path)


def _write(path: pathlib.Path, text: str = "", data: bytes | None = None) -> pathlib.Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    if data is not None:
        path.write_bytes(data)
    else:
        path.write_text(text)
    return path


class FixtureTree:
    """Minimal verified build + server/host sidecars + fixture launcher."""

    def __init__(self, root: pathlib.Path):
        self.root = root
        self.binary = _write(root / "binary", data=b"frozen-bin-v1")
        self.nav_pack = _write(root / "nav.pack", data=b"navpack")
        self.nav_flags = _write(root / "nav.flags", data=b"navflags")
        self.catalog = _write(root / "js-scripts.json", data=b'{"scripts":[]}')
        self.server_identity = _write(
            root / "server_identity.json", json.dumps({"fixture": "server_identity"})
        )
        self.host_conditions = _write(
            root / "host_conditions.json", json.dumps({"fixture": "host_conditions"})
        )
        dig = lambda p: _sha(p)
        self.manifest_obj = {
            "candidate": {
                "commit": "a" * 40,
                "branch": "test-branch",
                "sources_sha256_pre": "b" * 64,
                "sources_sha256_post": "b" * 64,
                "sources_stable_across_build": True,
                "build_exit": 0,
                "client": {"commit": "c" * 40, "sources_sha256": "d" * 64},
            },
            "features": {
                "requested": "memory-profile-no-alloc",
                "locked": True,
                "allocator": "std::alloc::System",
                "allocation_counting": False,
            },
            "binaries": {
                "candidate_tui_play": {
                    "path": str(self.binary),
                    "sha256": dig(self.binary),
                }
            },
            "nav": {
                "nav_pack": str(self.nav_pack),
                "nav_pack_sha256": dig(self.nav_pack),
                "nav_flags": str(self.nav_flags),
                "nav_flags_sha256": dig(self.nav_flags),
            },
            "catalog": {
                "js_scripts_json": str(self.catalog),
                "js_scripts_json_sha256": dig(self.catalog),
            },
        }
        self.manifest = _write(root / "manifest.json", json.dumps(self.manifest_obj))
        self.fixture_launcher = self._write_fixture_launcher(root / "fixture_launcher.py")
        self.cells = root / "cells"
        self.cells.mkdir()
        self.helper = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(600)"]
        )
        self.game_server = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(600)"]
        )
        self._owned = [self.helper, self.game_server]

    def _write_fixture_launcher(self, path: pathlib.Path) -> pathlib.Path:
        path.write_text(
            r'''#!/usr/bin/env python3
"""Dummy launcher: prints metadata JSON, owns a frontend child, writes run artifacts."""
import hashlib, json, os, pathlib, subprocess, sys, time

def main():
    # Args after script: observe_s teardown_s mode run_parent
    observe = float(sys.argv[1])
    teardown = float(sys.argv[2])
    mode = sys.argv[3]
    run_parent = pathlib.Path(sys.argv[4])
    binary = pathlib.Path(os.environ["FIXTURE_BINARY"]).resolve()
    effective_cli = json.loads(os.environ["FIXTURE_EFFECTIVE_CLI"])
    run = run_parent / ("run_%d" % os.getpid())
    run.mkdir(parents=True, exist_ok=False)
    if mode == "early_fail":
        child = subprocess.Popen([sys.executable, "-c", "import sys; sys.exit(42)"])
    elif mode == "missing_meta":
        # Never print metadata; hang then exit 1
        time.sleep(observe + teardown)
        sys.exit(1)
    else:
        child = subprocess.Popen(
            [sys.executable, "-c",
             "import time,sys; time.sleep(%s); sys.exit(0)" % (observe + teardown)]
        )
    started = time.time()
    meta = {
        "pid": child.pid,
        "run_dir": str(run),
        "binary": str(binary),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "effective_cli": effective_cli,
        "started_unix": started,
        "warmup_s": 0,
        "observe_s": observe,
        "frontend": "tui",
        "n": 1,
        "workload": "idle",
    }
    if mode != "missing_meta":
        print(json.dumps(meta), flush=True)
        (run / "metadata.json").write_text(json.dumps(dict(meta)) + "\n")
        (run / "samples.jsonl").write_text("{}\n")
        qual = run / "samples.qualification.jsonl"
        # Partial line then complete after short delay (tests retain partials).
        with qual.open("ab") as q:
            q.write(b'{"phase":"partial"')
            q.flush()
            time.sleep(min(0.15, observe * 0.25))
            q.write(b',"ok":true}\n')
            q.flush()
        time.sleep(max(0.0, observe - 0.15))
        with qual.open("ab") as q:
            q.write(b'{"phase":"observe_end"}\n')
        if mode == "early_fail":
            rc = child.wait(timeout=5)
            meta.update(exit_code=rc, ended_unix=time.time())
            (run / "metadata.json").write_text(json.dumps(meta) + "\n")
            print(json.dumps({"run_dir": str(run), "exit_code": rc}), flush=True)
            sys.exit(0)
        time.sleep(teardown)
        rc = child.wait(timeout=max(5, teardown + 2))
        meta.update(exit_code=rc, ended_unix=time.time())
        (run / "metadata.json").write_text(json.dumps(meta) + "\n")
        print(json.dumps({"run_dir": str(run), "exit_code": rc}), flush=True)
        sys.exit(0 if rc == 0 else 0)  # launcher stays 0 when frontend ok/fail distinct
    sys.exit(1)

if __name__ == "__main__":
    main()
'''
        )
        path.chmod(0o755)
        return path

    def close(self):
        for proc in self._owned:
            if proc.poll() is None:
                proc.send_signal(signal.SIGTERM)
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=3)

    def base_spec(self, *, observe=0.6, teardown=1.2, mode="ok", max_wall_s=20.0,
                  interval=0.2, cell_id="cell_ok", index=1) -> dict:
        # launcher_argv is the exact Popen list (no shell).
        run_parent = self.root / "frontend_runs"
        run_parent.mkdir(exist_ok=True)
        launcher_argv = [
            sys.executable,
            str(self.fixture_launcher),
            str(observe),
            str(teardown),
            mode,
            str(run_parent),
        ]
        return {
            "id": cell_id,
            "index": index,
            "kind": "diagnostic",
            "binary": str(self.binary),
            "build_manifest": str(self.manifest),
            "build_role": "candidate",
            "frontend": "tui",
            "launcher_argv": launcher_argv,
            # Diagnostic-parser mirror args (must match binary/manifest/role).
            "diagnostic_argv": [
                "tui",
                "1",
                "idle",
                "--binary",
                str(self.binary),
                "--build-manifest",
                str(self.manifest),
                "--build-role",
                "candidate",
                "--observe",
                str(int(max(1, round(observe)))),
                "--warmup",
                "0",
                "--headless",
            ],
            "server_identity_path": str(self.server_identity),
            "host_conditions_path": str(self.host_conditions),
            "nav_pack": str(self.nav_pack),
            "nav_flags": str(self.nav_flags),
            "catalog_path": str(self.catalog),
            "game_server_pid": self.game_server.pid,
            "ambient_helpers": {"ambient_helper": self.helper.pid},
            "sampler_interval_s": interval,
            "max_wall_s": max_wall_s,
            "observe_s": observe,
            "warmup_s": 0.0,
            "teardown_grace_s": teardown,
        }


class ManagedCellTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.fx = FixtureTree(pathlib.Path(self.temp.name))
        self.addCleanup(self.fx.close)

    def _run(self, spec: dict, **kwargs):
        spec_path = self.fx.root / "spec.json"
        _write(spec_path, json.dumps(spec))
        kwargs.setdefault("accounting_script", ACCOUNTING)
        return rmc.run_managed_cell(spec_path, self.fx.cells, **kwargs)

    def test_successful_observation_then_delayed_teardown(self):
        spec = self.fx.base_spec(observe=0.5, teardown=1.0, mode="ok", interval=0.15)
        report = self._run(spec)
        self.assertEqual(report["status"], "completed", report)
        self.assertEqual(report["sampler_result"]["exit_code"], 0, report)
        self.assertEqual(report["sampler_result"].get("completion"), "controlled_stop", report)
        cell_dir = pathlib.Path(report["cell_dir"])
        self.assertTrue((cell_dir / "launch.json").is_file())
        self.assertTrue((cell_dir / "receipt.json").is_file())
        self.assertTrue((cell_dir / "process_accounting.jsonl").is_file())
        receipt = json.loads((cell_dir / "receipt.json").read_text())
        self.assertEqual(receipt["exit_code"], 0)
        self.assertEqual(receipt["launcher_exit_code"], 0)
        # Frontend and launcher exits kept as distinct fields.
        self.assertIn("exit_code", receipt)
        self.assertIn("launcher_exit_code", receipt)
        self.assertIsInstance(receipt["exit_code"], int)
        self.assertIsInstance(receipt["launcher_exit_code"], int)
        launch = json.loads((cell_dir / "launch.json").read_text())
        self.assertEqual(launch["binary_sha256"], _sha(self.fx.binary))
        self.assertFalse(receipt["performance_acceptance"])
        # No zombie owned children: launcher/collector gone.
        if report.get("launcher_pid"):
            self.assertFalse(_alive(report["launcher_pid"]))
        if report.get("collector_pid"):
            self.assertFalse(_alive(report["collector_pid"]))

    def test_early_frontend_fail_stops_collector_no_retry(self):
        spec = self.fx.base_spec(observe=0.3, teardown=0.2, mode="early_fail", cell_id="early")
        report = self._run(spec)
        self.assertIn(report["status"], ("failed_or_unavailable", "failed", "incomplete"))
        self.assertEqual(report.get("attempts"), 1)
        cell_dir = pathlib.Path(report["cell_dir"])
        receipt = json.loads((cell_dir / "receipt.json").read_text())
        self.assertEqual(receipt["exit_code"], 42)
        self.assertEqual(receipt["launcher_exit_code"], 0)
        self.assertIn("frontend_failed_or_incomplete", receipt["binding_errors"])

    def test_sampler_fail_recorded_not_replaced(self):
        # Point accounting at a non-executable path so child fails immediately.
        bad = self.fx.root / "no_such_accounting.py"
        spec = self.fx.base_spec(cell_id="samp_fail", observe=0.4, teardown=0.4)
        report = self._run(spec, accounting_script=bad)
        self.assertNotEqual(report.get("sampler_result", {}).get("exit_code"), 0)
        receipt = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())
        self.assertIn("sampler_failed_or_incomplete", receipt["binding_errors"])
        # Still one attempt; no silent replacement sampler.
        self.assertEqual(report.get("attempts"), 1)

    def test_missing_metadata_no_retry_preserves_cell(self):
        spec = self.fx.base_spec(
            observe=0.4, teardown=0.2, mode="missing_meta", cell_id="nometa", max_wall_s=3.0
        )
        report = self._run(spec)
        self.assertEqual(report.get("attempts"), 1)
        self.assertTrue(pathlib.Path(report["cell_dir"]).is_dir())
        # Receipt still written when possible; run_dir may be missing.
        receipt_path = pathlib.Path(report["cell_dir"]) / "receipt.json"
        self.assertTrue(receipt_path.is_file())
        receipt = json.loads(receipt_path.read_text())
        self.assertEqual(receipt["status"], "failed_or_unavailable")

    def test_deadline_cleanup_owned_children_only(self):
        # Long observe so max_wall trips; ambient + game_server must survive.
        spec = self.fx.base_spec(
            observe=30, teardown=30, mode="ok", cell_id="deadline", max_wall_s=1.5, interval=0.2
        )
        report = self._run(spec)
        self.assertTrue(report.get("max_wall_exceeded"), report)
        self.assertTrue(_alive(self.fx.game_server.pid), "must not signal game_server")
        self.assertTrue(_alive(self.fx.helper.pid), "must not signal ambient helper")
        if report.get("launcher_pid"):
            self.assertFalse(_alive(report["launcher_pid"]))
        if report.get("collector_pid"):
            self.assertFalse(_alive(report["collector_pid"]))
        # Caller PID is this test process — still alive by definition.

    def test_exclusive_cell_create_no_overwrite(self):
        spec = self.fx.base_spec(cell_id="excl", observe=0.4, teardown=0.5)
        r1 = self._run(spec)
        self.assertIn(r1["status"], ("completed", "failed_or_unavailable", "failed", "incomplete"))
        with self.assertRaises(FileExistsError):
            self._run(spec)

    def test_preflight_rejects_changed_binary_without_launch(self):
        spec = self.fx.base_spec(cell_id="preflight_bin", observe=0.4, teardown=0.4)
        # Corrupt binary after manifest freeze.
        self.fx.binary.write_bytes(b"tampered")
        report = self._run(spec)
        self.assertEqual(report["status"], "preflight_failed")
        self.assertFalse(report.get("launched", True))
        cell_dir = pathlib.Path(report["cell_dir"])
        self.assertTrue((cell_dir / "cell_report.json").is_file())
        self.assertFalse((cell_dir / "launch.json").exists())
        self.assertFalse(list(cell_dir.glob("**/process_accounting.jsonl")))

    def test_preflight_rejects_dead_server_identity(self):
        dead = subprocess.Popen([sys.executable, "-c", "import sys; sys.exit(0)"])
        dead.wait(timeout=5)
        spec = self.fx.base_spec(cell_id="dead_srv", observe=0.3, teardown=0.3)
        spec["game_server_pid"] = dead.pid
        report = self._run(spec)
        self.assertEqual(report["status"], "preflight_failed")
        self.assertFalse(report.get("launched", True))

    def test_argv_inconsistent_with_spec_rejected(self):
        spec = self.fx.base_spec(cell_id="bad_argv")
        # diagnostic_argv claims different binary than spec.binary
        other = _write(self.fx.root / "other_bin", data=b"other")
        spec["diagnostic_argv"] = [
            "tui", "1", "idle",
            "--binary", str(other),
            "--build-manifest", str(self.fx.manifest),
            "--build-role", "candidate",
            "--observe", "1", "--warmup", "0", "--headless",
        ]
        report = self._run(spec)
        self.assertEqual(report["status"], "preflight_failed")

    def test_receipt_hashes_and_frontend_launcher_distinction(self):
        spec = self.fx.base_spec(cell_id="hash", observe=0.5, teardown=0.8)
        report = self._run(spec)
        cell_dir = pathlib.Path(report["cell_dir"])
        receipt = json.loads((cell_dir / "receipt.json").read_text())
        launch = json.loads((cell_dir / "launch.json").read_text())
        self.assertEqual(receipt["launch_sha256"], _sha(cell_dir / "launch.json"))
        self.assertEqual(receipt["binary_sha256"], launch["binary_sha256"])
        if receipt.get("run_dir"):
            for name, digest in receipt.get("raw_hashes", {}).items():
                self.assertEqual(digest, _sha(pathlib.Path(receipt["run_dir"]) / name))
        self.assertIsInstance(receipt["exit_code"], int)
        self.assertIsInstance(receipt["launcher_exit_code"], int)


def _alive(pid: int) -> bool:
    if not isinstance(pid, int) or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


if __name__ == "__main__":
    unittest.main()
