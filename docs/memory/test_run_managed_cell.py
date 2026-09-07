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
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import build_provenance as bp  # noqa: E402
import cache_provenance as cp  # noqa: E402
import managed_receipt as mr  # noqa: E402
import run_managed_cell as rmc  # noqa: E402
import server_resources as sr  # noqa: E402

MEMORY = ROOT
ACCOUNTING = MEMORY / "process_accounting.py"
SERVER_RESOURCES = MEMORY / "server_resources.py"
NATIVE_PROCESS_SAMPLE = MEMORY / "native_process_sample.py"


def _make_cache_tree(root: pathlib.Path) -> tuple[pathlib.Path, pathlib.Path]:
    cache = root / "pack" / "client"
    unpack = root / "unpack"
    cache.mkdir(parents=True, exist_ok=True)
    unpack.mkdir(parents=True, exist_ok=True)
    for name in cp.JAGS:
        (cache / name).write_bytes(name.encode())
    version = cp.file_sha256(cache / "versionlist")[:16]
    snapshot = unpack / version
    snapshot.mkdir(exist_ok=True)
    for name in cp.SNAPSHOTS:
        (snapshot / name).write_bytes(name.encode())
    for name in cp.STORE_FILES:
        (cache.parent / name).write_bytes(name.encode())
    return cache, unpack


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
        self.server_identity.write_text(json.dumps({
            'pid': self.game_server.pid,
            'start_identity': sr.sample_process(self.game_server.pid, timeout=2)['start_identity']}))

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
    elif mode == 'orphan_after_start':
        child = subprocess.Popen([sys.executable, '-c',
            'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)'])
    else:
        child = subprocess.Popen(
            [sys.executable, "-c",
             "import time,sys; time.sleep(%s); sys.exit(0)" % (observe + teardown + (1 if mode == 'delayed' else 0))]
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
            q.write(b'{"phase":"observe-start"')
            q.flush()
            time.sleep(min(0.15, observe * 0.25))
            q.write(b',"elapsed_s":0,"slots":[{}]}\n')
            q.flush()
        if mode == 'orphan_after_start':
            meta.update(exit_code=0, ended_unix=time.time())
            (run/'metadata.json').write_text(json.dumps(meta)+'\n')
            return
        if mode == 'delayed':
            time.sleep(1)
        time.sleep(max(0.0, observe - 0.15))
        with qual.open("ab") as q:
            if mode != 'missing_end':
                q.write(b'{"phase":"observe-end","elapsed_s":1,"slots":[{}]}\n')
            if mode == 'duplicate_end':
                q.write(b'{"phase":"observe-end","elapsed_s":1,"slots":[{}]}\n')
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
        kwargs.setdefault('_test_launcher', True)
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

    def test_relative_cell_root_and_accounting_path_reach_same_output(self):
        spec=self.fx.base_spec(observe=.4,teardown=1.0,interval=.15)
        path=self.fx.root/'relative-spec.json'
        path.write_text(json.dumps(spec))
        result=rmc.run_managed_cell(path,os.path.relpath(self.fx.cells),
            accounting_script=os.path.relpath(ACCOUNTING),_test_launcher=True)
        self.assertEqual(result['status'],'completed',result)
        cell=pathlib.Path(result['cell_dir'])
        self.assertTrue(cell.is_absolute())
        self.assertTrue((cell/'process_accounting.jsonl').is_file())
        receipt=json.loads((cell/'receipt.json').read_text())
        self.assertEqual(pathlib.Path(receipt['sampler_output_path']),cell/'process_accounting.jsonl')

    def test_real_delayed_qualification_controls_stop(self):
        spec = self.fx.base_spec(observe=.5, teardown=1, mode='delayed', interval=.15)
        report = self._run(spec)
        self.assertEqual(report['status'], 'completed', report)
        self.assertGreaterEqual(report['collector_stop_after_observe_s'], .3)
        rows = [json.loads(line) for line in
                (pathlib.Path(report['cell_dir'])/'process_accounting.jsonl').read_text().splitlines()]
        self.assertGreater(rows[-1]['elapsed_s'], 1.5)

    def test_short_teardown_before_pad_fails_honestly(self):
        # Teardown shorter than 2*interval: launcher dies while the required
        # post-observe collector hold is still running. Launcher remains a
        # required sampled role, so continuous collection fails honestly —
        # never completed, never retried, never by dropping/faking the role.
        interval = 0.25
        spec = self.fx.base_spec(
            observe=0.5, teardown=0.05, mode="ok", interval=interval, cell_id="short_td"
        )
        report = self._run(spec)
        self.assertEqual(report["status"], "failed_or_unavailable", report)
        self.assertEqual(report.get("launcher_exit_code"), 0, report)
        self.assertTrue(report.get("launcher_exited_before_pad"), report)
        # Pad wall-clock still held after observe-end even though measurement fails.
        self.assertIsNotNone(report.get("collector_stop_after_observe_s"), report)
        self.assertGreaterEqual(
            report["collector_stop_after_observe_s"], 2.0 * interval, report
        )
        receipt = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())
        self.assertEqual(receipt["status"], "failed_or_unavailable", receipt)
        runner_errors = receipt.get("runner_errors") or []
        self.assertTrue(
            any("launcher exited before post-observation" in e for e in runner_errors),
            receipt,
        )
        # Collector exit stays the real accounting outcome (typically nonzero
        # once the required launcher PID is gone); never rewritten to success.
        self.assertIsInstance(report.get("collector_exit_code"), int, report)

    def test_missing_or_duplicate_boundary_fails_durable_receipt(self):
        for mode in ('missing_end', 'duplicate_end'):
            with self.subTest(mode=mode):
                report = self._run(self.fx.base_spec(mode=mode, cell_id=mode, interval=.15))
                receipt = json.loads((pathlib.Path(report['cell_dir'])/'receipt.json').read_text())
                self.assertEqual(report['status'], 'failed_or_unavailable')
                self.assertEqual(receipt['status'], 'failed_or_unavailable')
                self.assertTrue(receipt['runner_errors'])

    def test_launcher_exit_does_not_leave_term_ignoring_frontend(self):
        # Short test cleanup bounds; production retains its declared 15 seconds.
        actual_cleanup = rmc._cleanup_frontend
        with mock.patch.object(rmc, '_cleanup_frontend',
                               side_effect=lambda *a, **kw: actual_cleanup(*a, **kw, wait_s=.2)):
            report = self._run(self.fx.base_spec(mode='orphan_after_start', interval=.15))
        self.assertFalse(_alive(report['frontend_pid']), report)
        self.assertIn('SIGKILL', report['cleanup']['frontend']['signals'])
        self.assertTrue(_alive(self.fx.helper.pid))
        self.assertTrue(_alive(self.fx.game_server.pid))

    def test_identity_mismatch_cleanup_never_signals_foreign_process(self):
        result = rmc._cleanup_frontend(self.fx.helper.pid, 'wrong identity', backend='system', wait_s=.01)
        self.assertTrue(result['orphan_risk'])
        self.assertEqual(result['signals'], [])
        self.assertTrue(_alive(self.fx.helper.pid))

    def test_invalid_specs_and_actual_launcher_mismatch_precede_launch(self):
        cases = [{'sampler_interval_s': float('inf')}, {'max_wall_s': float('nan')},
                 {'ambient_helpers': {'collector': self.fx.helper.pid}},
                 {'ambient_helpers': {'helper': self.fx.game_server.pid}},
                 {'id': '../outside'}, {'process_backend':'unknown'}]
        for change in cases:
            with self.subTest(change=change), self.assertRaises(rmc.CellError):
                rmc.validate_spec(self.fx.base_spec() | change)
        result = self._run(self.fx.base_spec(), _test_launcher=False)
        self.assertEqual(result['status'], 'preflight_failed')
        self.assertFalse(result['launched'])

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS backend')
    def test_native_backend_preflight_matches_native_collector_identity(self):
        spec = self.fx.base_spec(interval=.15)
        spec['process_backend']='libproc'
        native = rmc.pa.process_sampler('libproc')(self.fx.game_server.pid, timeout=2)
        self.fx.server_identity.write_text(json.dumps({'pid':self.fx.game_server.pid,
                                                     'start_identity':native['start_identity']}))
        report = self._run(spec)
        self.assertEqual(report['status'], 'completed', report)
        launch = json.loads((pathlib.Path(report['cell_dir'])/'launch.json').read_text())
        self.assertEqual(launch['sampler']['process_backend'], 'libproc')

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

    def test_nonobject_server_identity_is_durable_preflight_failure(self):
        self.fx.server_identity.write_text('[]')
        result = self._run(self.fx.base_spec())
        self.assertEqual(result['status'], 'preflight_failed')
        self.assertFalse(result['launched'])

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

    def test_default_no_cache_stays_compatible(self):
        report = self._run(self.fx.base_spec(cell_id="nocache", observe=0.5, teardown=0.8))
        self.assertEqual(report["status"], "completed", report)
        launch = json.loads((pathlib.Path(report["cell_dir"]) / "launch.json").read_text())
        self.assertNotIn("cache_provenance_path", launch)
        receipt = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())
        self.assertNotIn("cache_provenance_path", receipt)
        self.assertNotIn("cache_verified_after_completion_utc", receipt)

    def test_paired_cache_fields_required_together(self):
        cache, unpack = _make_cache_tree(self.fx.root / "cache_pair")
        with self.assertRaises(rmc.CellError):
            rmc.validate_spec(self.fx.base_spec() | {"cache_dir": str(cache)})
        with self.assertRaises(rmc.CellError):
            rmc.validate_spec(self.fx.base_spec() | {"unpack_root": str(unpack)})
        ok = rmc.validate_spec(
            self.fx.base_spec() | {"cache_dir": str(cache), "unpack_root": str(unpack)}
        )
        self.assertEqual(ok["cache_dir"], str(cache))
        self.assertEqual(ok["unpack_root"], str(unpack))

    def test_complete_cache_proof_pre_and_post(self):
        cache, unpack = _make_cache_tree(self.fx.root / "cache_ok")
        spec = self.fx.base_spec(cell_id="cache_ok", observe=0.5, teardown=0.8)
        spec["cache_dir"] = str(cache)
        spec["unpack_root"] = str(unpack)
        report = self._run(spec)
        self.assertEqual(report["status"], "completed", report)
        cell_dir = pathlib.Path(report["cell_dir"])
        prov_path = cell_dir / "cache-provenance.json"
        self.assertTrue(prov_path.is_file(), report)
        launch = json.loads((cell_dir / "launch.json").read_text())
        receipt = json.loads((cell_dir / "receipt.json").read_text())
        self.assertEqual(launch["cache_provenance_path"], str(prov_path.resolve()))
        self.assertEqual(launch["cache_provenance_sha256"], _sha(prov_path))
        self.assertEqual(
            launch["cache_content_identity_sha256"],
            json.loads(prov_path.read_text())["content_identity_sha256"],
        )
        self.assertIn("cache_verified_before_launch_utc", launch)
        self.assertIn("cache_verified_after_completion_utc", receipt)
        self.assertEqual(receipt["cache_provenance_path"], launch["cache_provenance_path"])
        # Full scope still verifies after observation.
        cp.verify_snapshot(json.loads(prov_path.read_text()))

    def test_cache_changed_during_observation_fails_receipt(self):
        cache, unpack = _make_cache_tree(self.fx.root / "cache_mut")
        spec = self.fx.base_spec(cell_id="cache_mut", observe=0.8, teardown=0.6, interval=0.15)
        spec["cache_dir"] = str(cache)
        spec["unpack_root"] = str(unpack)

        real_create = mr.create_launch

        def create_and_mutate(*args, **kwargs):
            value = real_create(*args, **kwargs)
            (cache / "config").write_bytes(b"mutated-during-observation")
            return value

        with mock.patch.object(rmc.mr, "create_launch", side_effect=create_and_mutate):
            report = self._run(spec)
        self.assertEqual(report["status"], "failed_or_unavailable", report)
        receipt = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())
        self.assertEqual(receipt["status"], "failed_or_unavailable")
        self.assertIn("cache_provenance_changed_or_invalid", receipt["binding_errors"])

    def test_role_identities_agree_with_collector_and_server_sidecar(self):
        report = self._run(self.fx.base_spec(cell_id="roles", observe=0.5, teardown=0.8, interval=0.15))
        self.assertEqual(report["status"], "completed", report)
        receipt = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())
        launch = json.loads((pathlib.Path(report["cell_dir"]) / "launch.json").read_text())
        result = receipt["sampler_result"]
        roles = result["role_identities"]
        expected = set(launch["sampler"]["roles"]) | {"launcher", "collector"}
        self.assertEqual(set(roles), expected)
        self.assertEqual(result["launcher_pid"], report["launcher_pid"])
        self.assertEqual(result["collector_pid"], report["collector_pid"])
        self.assertEqual(roles["launcher"]["pid"], result["launcher_pid"])
        self.assertEqual(roles["collector"]["pid"], result["collector_pid"])
        server = json.loads(self.fx.server_identity.read_text())
        self.assertEqual(roles["game_server"]["pid"], server["pid"])
        self.assertEqual(roles["game_server"]["start_identity"], server["start_identity"])
        # Prelaunch map excludes owned children.
        self.assertNotIn("launcher", launch["sampler"]["roles"])
        self.assertNotIn("collector", launch["sampler"]["roles"])
        self.assertEqual(launch["sampler"]["roles"]["game_server"], self.fx.game_server.pid)
        self.assertEqual(launch["sampler"]["roles"]["controller"], os.getpid())
        # First collector sample rows match recorded identities.
        rows = [
            json.loads(line)
            for line in (pathlib.Path(report["cell_dir"]) / "process_accounting.jsonl")
            .read_text()
            .splitlines()
        ]
        samples = [r for r in rows if r.get("type") == "sample"]
        self.assertTrue(samples, rows[:3])
        first = samples[0]["roles"]
        for name, identity in roles.items():
            if name not in first:
                continue
            row = first[name]
            if row.get("status") != "ok":
                continue
            self.assertEqual(row["pid"], identity["pid"], name)
            self.assertEqual(row["start_identity"], identity["start_identity"], name)

    def test_sampler_modules_bound_and_backend_identity_consistent(self):
        report = self._run(self.fx.base_spec(cell_id="mods", observe=0.5, teardown=0.8))
        self.assertEqual(report["status"], "completed", report)
        launch = json.loads((pathlib.Path(report["cell_dir"]) / "launch.json").read_text())
        modules = launch["sampler"]["modules"]
        self.assertEqual(set(modules), {"process_accounting.py", "server_resources.py"})
        self.assertEqual(
            pathlib.Path(modules["process_accounting.py"]["path"]).resolve(),
            ACCOUNTING.resolve(),
        )
        self.assertEqual(modules["process_accounting.py"]["sha256"], _sha(ACCOUNTING))
        self.assertEqual(
            pathlib.Path(modules["server_resources.py"]["path"]).resolve(),
            SERVER_RESOURCES.resolve(),
        )
        self.assertEqual(modules["server_resources.py"]["sha256"], _sha(SERVER_RESOURCES))
        self.assertEqual(
            pathlib.Path(launch["sampler"]["module"]).resolve(),
            pathlib.Path(modules["process_accounting.py"]["path"]).resolve(),
        )
        self.assertEqual(
            launch["sampler"]["module_sha256"], modules["process_accounting.py"]["sha256"]
        )
        # Injected private accounting script is recorded under the same key.
        injected = self.fx.root / "injected_accounting.py"
        injected.write_text(
            "import runpy, sys, pathlib\n"
            f"sys.path.insert(0, {str(MEMORY)!r})\n"
            f"sys.argv[0] = {str(ACCOUNTING)!r}\n"
            f"runpy.run_path({str(ACCOUNTING)!r}, run_name='__main__')\n"
        )
        report2 = self._run(
            self.fx.base_spec(cell_id="mods_inj", observe=0.5, teardown=0.8),
            accounting_script=injected,
        )
        self.assertEqual(report2["status"], "completed", report2)
        launch2 = json.loads((pathlib.Path(report2["cell_dir"]) / "launch.json").read_text())
        self.assertEqual(
            pathlib.Path(launch2["sampler"]["modules"]["process_accounting.py"]["path"]).resolve(),
            injected.resolve(),
        )
        self.assertEqual(
            launch2["sampler"]["module_sha256"],
            launch2["sampler"]["modules"]["process_accounting.py"]["sha256"],
        )
        self.assertEqual(
            launch2["sampler"]["modules"]["process_accounting.py"]["sha256"], _sha(injected)
        )

    @unittest.skipUnless(sys.platform == "darwin", "native macOS backend")
    def test_libproc_modules_include_native_and_identity_backend(self):
        spec = self.fx.base_spec(cell_id="libproc_mods", interval=0.15)
        spec["process_backend"] = "libproc"
        native = rmc.pa.process_sampler("libproc")(self.fx.game_server.pid, timeout=2)
        self.fx.server_identity.write_text(
            json.dumps(
                {"pid": self.fx.game_server.pid, "start_identity": native["start_identity"]}
            )
        )
        report = self._run(spec)
        self.assertEqual(report["status"], "completed", report)
        launch = json.loads((pathlib.Path(report["cell_dir"]) / "launch.json").read_text())
        modules = launch["sampler"]["modules"]
        self.assertEqual(
            set(modules),
            {"process_accounting.py", "server_resources.py", "native_process_sample.py"},
        )
        self.assertEqual(
            pathlib.Path(modules["native_process_sample.py"]["path"]).resolve(),
            NATIVE_PROCESS_SAMPLE.resolve(),
        )
        self.assertEqual(
            modules["native_process_sample.py"]["sha256"], _sha(NATIVE_PROCESS_SAMPLE)
        )
        result = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())[
            "sampler_result"
        ]
        self.assertEqual(
            result["role_identities"]["game_server"]["start_identity"], native["start_identity"]
        )

    def test_module_mutation_before_completion_fails_runner(self):
        injected = self.fx.root / "mut_accounting.py"
        injected.write_text(
            "import runpy, sys\n"
            f"sys.path.insert(0, {str(MEMORY)!r})\n"
            f"sys.argv[0] = {str(ACCOUNTING)!r}\n"
            f"runpy.run_path({str(ACCOUNTING)!r}, run_name='__main__')\n"
        )
        spec = self.fx.base_spec(cell_id="mod_mut", observe=0.6, teardown=0.6, interval=0.15)

        real_create = mr.create_launch

        def create_and_mutate_module(*args, **kwargs):
            value = real_create(*args, **kwargs)
            injected.write_text(injected.read_text() + "\n# mutated\n")
            return value

        with mock.patch.object(rmc.mr, "create_launch", side_effect=create_and_mutate_module):
            report = self._run(spec, accounting_script=injected)
        self.assertEqual(report["status"], "failed_or_unavailable", report)
        receipt = json.loads((pathlib.Path(report["cell_dir"]) / "receipt.json").read_text())
        self.assertTrue(
            any("module" in e.lower() or "changed" in e.lower() for e in receipt.get("runner_errors") or []),
            receipt,
        )


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
