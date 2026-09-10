#!/usr/bin/env python3
"""Tests for one-cell managed launcher + process accounting orchestration.

Uses real dummy processes and fixture scripts — no live bot/server/networking.
"""
from __future__ import annotations

import copy
import json
import os
import pathlib
import signal
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from typing import Any
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import build_provenance as bp  # noqa: E402
import cache_provenance as cp  # noqa: E402
import managed_receipt as mr  # noqa: E402
import run_managed_cell as rmc  # noqa: E402
import run_diagnostic as rd  # noqa: E402
import server_resources as sr  # noqa: E402

MEMORY = ROOT
ACCOUNTING = MEMORY / "process_accounting.py"
SERVER_RESOURCES = MEMORY / "server_resources.py"
NATIVE_PROCESS_SAMPLE = MEMORY / "native_process_sample.py"
WINDOWS_PROCESS_SAMPLE = MEMORY / "windows_process_sample.py"


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
        # Unix: ignore SIGTERM so cleanup must escalate to SIGKILL.
        # Windows: SIGTERM maps to TerminateProcess and cannot be ignored; soft stage is already fatal.
        child = subprocess.Popen([sys.executable, '-c',
            "import sys,time; "
            "(sys.platform == 'win32') or "
            "__import__('signal').signal(__import__('signal').SIGTERM, __import__('signal').SIG_IGN); "
            "time.sleep(60)"])
    else:
        child = subprocess.Popen(
            [sys.executable, "-c",
             "import time,sys; time.sleep(%s); sys.exit(0)" % (observe + teardown + (1 if mode == 'delayed' else 0))]
        )
    helper = None
    if mode == "conpty_fake":
        # Exercise the Windows delayed collector handoff on non-Windows CI.
        helper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(600)"])
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
    if helper is not None:
        meta.update(
            terminal_transport="conpty",
            conpty_helpers=[{
                "pid": helper.pid,
                "parent_pid": os.getpid(),
                "image_name": "conhost.exe",
                "start_identity": "fixture-helper",
            }],
        )
    if mode != "missing_meta":
        print(json.dumps(meta), flush=True)
        (run / "metadata.json").write_text(json.dumps(dict(meta)) + "\n")
        (run / "samples.jsonl").write_text("{}\n")
        lifecycle = run / "samples.lifecycle.jsonl"
        def mark(event):
            with lifecycle.open("a") as out:
                out.write(json.dumps({"event": event, "monotonic_s": time.monotonic()}) + "\n")
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
        if mode == 'lifecycle':
            mark('observe-end')
        if mode == "early_fail":
            rc = child.wait(timeout=5)
            meta.update(exit_code=rc, ended_unix=time.time())
            (run / "metadata.json").write_text(json.dumps(meta) + "\n")
            print(json.dumps({"run_dir": str(run), "exit_code": rc}), flush=True)
            sys.exit(0)
        if mode == 'lifecycle':
            time.sleep(0.4)
            mark('Stop')
            time.sleep(0.3)
            mark('C')
            time.sleep(max(0.0, teardown - 0.7))
        else:
            time.sleep(teardown)
        rc = child.wait(timeout=max(5, teardown + 2))
        if helper is not None:
            helper.terminate()
            helper.wait(timeout=5)
        meta.update(exit_code=rc, ended_unix=time.time())
        (run / "metadata.json").write_text(json.dumps(meta) + "\n")
        print(json.dumps({"run_dir": str(run), "exit_code": rc}), flush=True)
        if mode == 'lifecycle':
            mark('launcher-exit')
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

    def _direct_spec(self):
        spec = self.fx.base_spec(cell_id='direct_contract')
        cache, unpack = _make_cache_tree(self.fx.root / 'direct-cache')
        result = (self.fx.root / 'direct_contract.json').resolve()
        spec_path = result.with_suffix(result.suffix + '.spec.json')
        cell_dir = result.with_suffix(result.suffix + '.cells') / 'direct_contract'
        run_dir = cell_dir / 'frontend-run'
        handoff = cell_dir / 'frontend-handoff.json'
        diag = [
            'tui', '1', 'active', '--binary', str(self.fx.binary),
            '--build-manifest', str(self.fx.manifest), '--build-role', 'candidate',
            '--no-diagnostics', '--sustain', '--warmup', '30', '--observe', '120',
            '--direct-owner-capture', '--run-dir', str(run_dir),
            '--frontend-handoff', str(handoff),
        ]
        spec.update(
            diagnostic_argv=diag, n=1, warmup_s=30, observe_s=120,
            teardown_grace_s=60, sampler_interval_s=.5, max_wall_s=365,
            process_backend='system', requested_backend='none', heaptrack=None,
            cache_dir=str(cache), unpack_root=str(unpack),
            capture_contract={
                'mode': 'direct-owner-v1', 'cell_dir': str(cell_dir),
                'run_dir': str(run_dir), 'frontend_handoff_path': str(handoff),
                'owned_output_paths': [str(result), str(spec_path),
                                       str(cell_dir), str(run_dir)],
                'warmup_s': 30, 'observe_s': 120, 'teardown_grace_s': 60,
                'guard_interval_s': .5, 'handoff_deadline_s': 5.0,
                'mem_available_floor_bytes': 268435456,
                'frontend_rss_limit_bytes': 536870912,
                'owned_output_limit_bytes': 67108864,
                'frontend_wall_limit_s': 360, 'outer_wall_limit_s': 365,
                'source_lineage': {'bound': True},
                'release_contract': str(self.fx.root / 'root-release.json'),
                'admission_receipts': {name: str(self.fx.root / (name + '.json'))
                                       for name in ('conflict', 'account', 'population', 'cache', 'server_health')},
            },
        )
        return spec

    def _direct_preflight_fixtures(self, spec, *, now=200.0):
        not_before = now - 100.0
        not_after = now - 90.0
        issued = now - 80.0
        expires = now + 100.0
        manifest = json.loads(self.fx.manifest.read_text())
        provenance = {
            'status': 'verified',
            'manifest_sha256': _sha(self.fx.manifest),
            'build_commit': manifest['candidate']['commit'],
            'client_commit': manifest['candidate']['client']['commit'],
            'host_sources_sha256': manifest['candidate']['sources_sha256_pre'],
            'client_sources_sha256': manifest['candidate']['client']['sources_sha256'],
            'files': {
                'manifest': {'path': str(self.fx.manifest.resolve()), 'sha256': _sha(self.fx.manifest)},
                'binary': {'path': str(self.fx.binary.resolve()), 'sha256': _sha(self.fx.binary)},
                'nav_pack': {'path': str(self.fx.nav_pack.resolve()), 'sha256': _sha(self.fx.nav_pack)},
                'nav_flags': {'path': str(self.fx.nav_flags.resolve()), 'sha256': _sha(self.fx.nav_flags)},
                'catalog': {'path': str(self.fx.catalog.resolve()), 'sha256': _sha(self.fx.catalog)},
            },
        }
        cache = cp.capture(spec['cache_dir'], spec['unpack_root'])
        server_start = json.loads(self.fx.server_identity.read_text())['start_identity']
        context = {
            'fixture_binding_sha256': 'f' * 64,
            'host_commit': provenance['build_commit'],
            'client_commit': provenance['client_commit'],
            'host_sources_sha256': provenance['host_sources_sha256'],
            'client_sources_sha256': provenance['client_sources_sha256'],
            'build_manifest_sha256': provenance['manifest_sha256'],
            'binary_sha256': _sha(self.fx.binary),
            'nav_pack_sha256': _sha(self.fx.nav_pack),
            'nav_flags_sha256': _sha(self.fx.nav_flags),
            'catalog_sha256': _sha(self.fx.catalog),
            'cache_content_identity_sha256': cache['content_identity_sha256'],
            'cache_snapshot_version': cache['snapshot_version'],
            'server_pid': self.fx.game_server.pid,
            'server_start_identity': server_start,
            'server_executable_basename': 'python3',
        }
        common = {
            'mode': 'direct-owner-v1', 'n': 1, 'workload': 'active',
            'frontend': 'tui', 'admitted': True,
            'observed_start_unix_s': not_before + 1.0,
            'observed_end_unix_s': not_before + 2.0,
            'boot_id': 'boot', 'context': context,
        }
        evidence = {
            'conflict': {
                'checked_classes': ['build', 'test', 'profiler', 'tui-panel-frontend'],
                'matches': [],
            },
            'account': {
                'fixture_binding_sha256': 'f' * 64,
                'slot_count': 1, 'active_slot_count': 1,
            },
            'population': {
                'fixture_binding_sha256': 'f' * 64,
                'requested_n': 1, 'admitted_n': 1, 'workload': 'active',
            },
            'cache': {
                'fixture_binding_sha256': 'f' * 64,
                'content_identity_sha256': cache['content_identity_sha256'],
                'snapshot_version': cache['snapshot_version'],
            },
            'server_health': {
                'pid': self.fx.game_server.pid,
                'start_identity': server_start,
                'executable_basename': 'python3',
                'probe': {'host': '127.0.0.1', 'port': 43594, 'succeeded': True},
            },
        }
        receipt_bindings = {}
        for kind, path_value in spec['capture_contract']['admission_receipts'].items():
            path = pathlib.Path(path_value)
            receipt = {
                'schema': f'direct-owner-{kind.replace("_", "-")}-admission-v1',
                'kind': kind,
                **common,
                'evidence': evidence[kind],
            }
            path.write_text(json.dumps(receipt, sort_keys=True))
            receipt_bindings[kind] = {'path': str(path.resolve()), 'sha256': _sha(path)}
        contract = spec['capture_contract']
        release = {
            'schema': 'direct-owner-root-release-v1',
            'mode': 'direct-owner-v1', 'n': 1, 'workload': 'active', 'frontend': 'tui',
            'issued_unix_s': issued,
            'observation_window': {
                'not_before_unix_s': not_before,
                'not_after_unix_s': not_after,
            },
            'expires_unix_s': expires,
            'boot_id': 'boot', 'context': context,
            'spec_binding': {
                'result_path': contract['owned_output_paths'][0],
                'cell_dir': contract['cell_dir'], 'run_dir': contract['run_dir'],
                'frontend_handoff_path': contract['frontend_handoff_path'],
                'build_manifest_path': str(self.fx.manifest.resolve()),
                'binary_path': str(self.fx.binary.resolve()),
                'nav_pack_path': str(self.fx.nav_pack.resolve()),
                'nav_flags_path': str(self.fx.nav_flags.resolve()),
                'catalog_path': str(self.fx.catalog.resolve()),
                'cache_dir': str(pathlib.Path(spec['cache_dir']).resolve()),
                'unpack_root': str(pathlib.Path(spec['unpack_root']).resolve()),
            },
            'receipt_bindings': receipt_bindings,
        }
        release_path = pathlib.Path(contract['release_contract'])
        release_path.write_text(json.dumps(release, sort_keys=True))

        def sample(pid, *, timeout):
            identity = server_start if pid == self.fx.game_server.pid else 'ambient-start'
            return {'pid': pid, 'start_identity': identity, 'resident_bytes': 1, 'state': 'S'}

        def snapshot(**changes):
            value = {
                'mem_available_bytes': 805306368,
                'swap_total_bytes': 0,
                'swap_free_bytes': 0,
                'vmstat': {'pswpin': 0, 'pswpout': 0, 'oom_kill': 0},
                'cgroup': {'identity': 'cg', 'boot_id': 'boot',
                           'memory_events': {'oom': 0, 'oom_kill': 0}},
            }
            value.update(changes)
            return value

        return {
            'provenance': provenance, 'cache': cache, 'context': context,
            'release': release, 'release_path': release_path,
            'sample': sample, 'snapshot': snapshot, 'now': now,
        }

    def _direct_lifecycle_fixture(self, launcher, *, now):
        result_path = self.fx.root / 'direct.json'
        spec_path = result_path.with_suffix(result_path.suffix + '.spec.json')
        cell_id = 'direct'
        cell_dir = result_path.with_suffix(result_path.suffix + '.cells') / cell_id
        run_dir = cell_dir / 'frontend-run'
        handoff = cell_dir / 'frontend-handoff.json'
        cache, unpack = _make_cache_tree(self.fx.root / 'private-lifecycle-cache')
        spec = self.fx.base_spec(cell_id=cell_id)
        spec.update(
            n=1, warmup_s=30, observe_s=120, teardown_grace_s=60,
            sampler_interval_s=.5, max_wall_s=365, process_backend='system',
            requested_backend='none', heaptrack=None,
            cache_dir=str(cache), unpack_root=str(unpack),
        )
        spec['diagnostic_argv'] = [
            '--binary', str(self.fx.binary), '--build-manifest', str(self.fx.manifest),
            '--build-role', 'candidate', '--no-diagnostics', '--sustain', '--warmup', '30',
            '--observe', '120', '--direct-owner-capture', '--run-dir', str(run_dir),
            '--frontend-handoff', str(handoff), 'tui', '1', 'active',
        ]
        spec['launcher_argv'] = [str(launcher)] + spec['diagnostic_argv']
        spec['capture_contract'] = {
            'mode': 'direct-owner-v1', 'cell_dir': str(cell_dir.resolve()),
            'run_dir': str(run_dir.resolve()),
            'frontend_handoff_path': str(handoff.resolve()),
            'owned_output_paths': [
                str(result_path.resolve()), str(spec_path.resolve()),
                str(cell_dir.resolve()), str(run_dir.resolve()),
            ],
            'warmup_s': 30, 'observe_s': 120, 'teardown_grace_s': 60,
            'guard_interval_s': 0.5, 'handoff_deadline_s': 5,
            'mem_available_floor_bytes': 268435456,
            'frontend_rss_limit_bytes': 536870912,
            'owned_output_limit_bytes': 67108864,
            'frontend_wall_limit_s': 360, 'outer_wall_limit_s': 365,
            'source_lineage': {'fixture': True},
            'release_contract': str(self.fx.root / 'private-root-release.json'),
            'admission_receipts': {
                name: str(self.fx.root / ('private-' + name + '.json'))
                for name in ('conflict', 'account', 'population', 'cache', 'server_health')
            },
        }
        spec_path.write_text(json.dumps(spec))
        fixtures = self._direct_preflight_fixtures(spec, now=now)
        admission_bindings = {
            'release_contract': {
                'path': str(fixtures['release_path'].resolve()),
                'sha256': _sha(fixtures['release_path']),
            },
            **{
                'receipt_' + kind: {
                    'path': str(pathlib.Path(path).resolve()),
                    'sha256': _sha(pathlib.Path(path)),
                }
                for kind, path in spec['capture_contract']['admission_receipts'].items()
            },
        }
        fake_pf = {
            'binary': str(self.fx.binary.resolve()),
            'manifest': str(self.fx.manifest.resolve()),
            'provenance': fixtures['provenance'],
            'server_pid': self.fx.game_server.pid,
            'server_sample': sr.sample_process(self.fx.game_server.pid, timeout=2),
            'ambient_identities': {'ambient_helper': {
                'pid': self.fx.helper.pid,
                'sample': sr.sample_process(self.fx.helper.pid, timeout=2),
            }},
            'direct_cache_snapshot': fixtures['cache'],
            'direct_preflight': {
                'admission_bindings': admission_bindings,
                'release_expires_unix_s': fixtures['release']['expires_unix_s'],
            },
        }
        return {
            'spec': spec, 'spec_path': spec_path, 'result_path': result_path,
            'cell_dir': cell_dir, 'run_dir': run_dir, 'handoff': handoff,
            'fixtures': fixtures, 'fake_pf': fake_pf,
        }

    def test_direct_contract_and_argv_paths_are_exact_and_mode_only(self):
        spec = self._direct_spec()
        normalized = rmc.validate_spec(spec)
        args = rmc.parse_diagnostic_argv(spec['diagnostic_argv'])
        rmc.require_argv_consistent_with_spec(normalized, args, _test_launcher=True)
        for key, value in (
            ('n', 16), ('warmup_s', 31), ('sampler_interval_s', .6),
            ('max_wall_s', 366), ('process_backend', 'libproc'),
        ):
            with self.subTest(key=key), self.assertRaises(rmc.CellError):
                rmc.validate_spec(spec | {key: value})
        unknown = json.loads(json.dumps(spec))
        unknown['capture_contract']['unreviewed'] = True
        with self.assertRaisesRegex(rmc.CellError, 'unknown'):
            rmc.validate_spec(unknown)
        mismatch = json.loads(json.dumps(spec))
        mismatch['capture_contract']['run_dir'] = str(self.fx.root / 'elsewhere')
        with self.assertRaises(rmc.CellError):
            rmc.require_argv_consistent_with_spec(
                rmc.validate_spec(mismatch), rmc.parse_diagnostic_argv(mismatch['diagnostic_argv']),
                _test_launcher=True)
        foreign_output = json.loads(json.dumps(spec))
        foreign_output['capture_contract']['owned_output_paths'][0] = str(
            self.fx.root / 'unrelated-result.json'
        )
        with self.assertRaisesRegex(rmc.CellError, 'owned paths'):
            rmc.validate_spec(foreign_output)
        ordinary = self.fx.base_spec()
        ordinary['capture_contract'] = {'mode': 'direct-owner-v1'}
        with self.assertRaises(rmc.CellError):
            rmc.validate_spec(ordinary)

        argv_mismatch = json.loads(json.dumps(spec))
        index = argv_mismatch['diagnostic_argv'].index('--run-dir') + 1
        argv_mismatch['diagnostic_argv'][index] = str(self.fx.root / 'redirected-run')
        with self.assertRaisesRegex(rmc.CellError, 'run-dir'):
            rmc.require_argv_consistent_with_spec(
                rmc.validate_spec(argv_mismatch),
                rmc.parse_diagnostic_argv(argv_mismatch['diagnostic_argv']),
                _test_launcher=True,
            )

    def test_owned_output_scanner_deduplicates_hardlinks_and_rejects_symlinks(self):
        root = self.fx.root / 'owned'
        nested = root / 'nested'
        nested.mkdir(parents=True)
        first = root / 'bytes'
        first.write_bytes(b'12345')
        os.link(first, nested / 'same')
        scanner = rmc.OwnedOutputScanner([root, nested])
        sample = scanner.scan()
        self.assertEqual(sample['total_bytes'], 5)
        external = self.fx.root / 'external'
        external.write_bytes(b'secret')
        (root / 'escape').symlink_to(external)
        with self.assertRaisesRegex(rmc.CellError, 'symlink'):
            scanner.scan()
        self.assertEqual(external.read_bytes(), b'secret')

    def test_owned_output_scanner_rejects_disappearance_but_allows_handoff_replacement(self):
        root = self.fx.root / 'owned-replacement'
        root.mkdir()
        stable = root / 'stable'
        stable.write_text('stable')
        handoff = root / 'frontend-handoff.json'
        handoff.write_text('spawned')
        scanner = rmc.OwnedOutputScanner([root], replaceable_paths=[handoff])
        scanner.scan()
        stable.unlink()
        with self.assertRaisesRegex(rmc.CellError, 'disappeared'):
            scanner.scan()

        stable.write_text('stable')
        scanner = rmc.OwnedOutputScanner([root], replaceable_paths=[handoff])
        scanner.scan()
        replacement = root / 'frontend-handoff.next.json'
        replacement.write_text('exited')
        os.replace(replacement, handoff)
        sample = scanner.scan()
        self.assertGreater(sample['total_bytes'], 0)

        transient = root / 'frontend-handoff.next.json'
        scanner = rmc.OwnedOutputScanner(
            [root], replaceable_paths=[handoff, transient]
        )
        transient.write_text('next')
        scanner.scan()
        transient.unlink()
        scanner.scan()

    def test_owned_output_scanner_fails_in_scan_disappearance_and_replayed_replacement(self):
        root = self.fx.root / 'owned-races'
        root.mkdir()
        victim = root / 'victim'
        victim.write_text('evidence')
        real_scandir = os.scandir

        def remove_after_listing(path):
            entries = list(real_scandir(path))
            if pathlib.Path(path) == root:
                victim.unlink()
            return entries

        scanner = rmc.OwnedOutputScanner([root])
        with mock.patch.object(rmc.os, 'scandir', side_effect=remove_after_listing):
            with self.assertRaisesRegex(rmc.CellError, 'disappeared'):
                scanner.scan()

        handoff = root / 'frontend-handoff.json'
        handoff.write_text('spawned')
        scanner = rmc.OwnedOutputScanner([root], replaceable_paths=[handoff])
        scanner.scan()
        replacement = root / 'frontend-handoff.next.json'
        replacement.write_text('exited')
        os.replace(replacement, handoff)
        scanner.scan()
        replacement.write_text('replayed')
        os.replace(replacement, handoff)
        with self.assertRaisesRegex(rmc.CellError, 'replacement'):
            scanner.scan()

    def test_handoff_identity_deadline_and_exit_proof_are_fail_closed(self):
        spawn = {
            'nonce': 'abc', 'frontend_pid': 101, 'frontend_start_identity': 'start:101',
            'launcher_pid': 77, 'run_dir': '/tmp/run', 'spawn_before_monotonic_s': 10.0,
            'spawn_after_monotonic_s': 10.1, 'frontend': 'tui', 'terminal': True,
            'terminal_size': [120, 40], 'n': 1, 'workload': 'active',
            'warmup_s': 30, 'observe_s': 120, 'teardown_grace_s': 60,
        }
        state = {'schema': 1, 'mode': 'direct-owner-v1', 'state': 'spawned', 'spawn': spawn}
        accepted = rmc.validate_frontend_handoff(
            state, launcher_pid=77, run_dir=pathlib.Path('/tmp/run'),
            receive_monotonic_s=10.49, launcher_start_monotonic_s=9.9)
        self.assertEqual(accepted['nonce'], 'abc')
        with self.assertRaisesRegex(rmc.CellError, 'first RSS'):
            rmc.require_first_sample_deadline(accepted, sample_end_monotonic_s=10.500001)
        rmc.require_first_sample_deadline(accepted, sample_end_monotonic_s=10.5)
        exited = {'schema': 1, 'mode': 'direct-owner-v1', 'state': 'exited',
                  'spawn': spawn, 'frontend_start_identity': 'start:101',
                  'exit_code': 0, 'wait_return_monotonic_s': 11.0}
        rmc.validate_exit_handoff(exited, accepted, receipt_monotonic_s=11.1)
        exited['spawn'] = dict(spawn, nonce='replay')
        with self.assertRaises(rmc.CellError):
            rmc.validate_exit_handoff(exited, accepted, receipt_monotonic_s=11.1)

        exited['spawn'] = spawn
        metadata = {
            'run_dir': '/tmp/run', 'pid': 101, 'exit_code': 0,
            'direct_owner_spawn': spawn,
            'direct_owner_exit': {
                'frontend_start_identity': 'start:101', 'exit_code': 0,
                'wait_return_monotonic_s': 11.0,
            },
        }
        rmc.validate_direct_final_metadata(metadata, accepted, exited, pathlib.Path('/tmp/run'))
        metadata['direct_owner_exit']['exit_code'] = 9
        with self.assertRaisesRegex(rmc.CellError, 'metadata'):
            rmc.validate_direct_final_metadata(metadata, accepted, exited, pathlib.Path('/tmp/run'))

    def test_guard_thresholds_accept_equal_and_fail_over_without_imputation(self):
        scanner = mock.Mock(scan=mock.Mock(return_value={'total_bytes': 67108864,
                                                          'breakdown': [], 'digest': 'd'}))
        sample = {'pid': 101, 'start_identity': 'start:101', 'parent_pid': 77,
                  'resident_bytes': 536870912}
        row = rmc.direct_guard_sample(
            scheduled=10.5, started=10.5, ended=10.6, spawn_before=10.0,
            frontend_pid=101, frontend_identity='start:101', launcher_pid=77,
            process_sample=sample, mem_available=268435456, output_scanner=scanner)
        self.assertIsNone(row['failure_reason'])
        for change, expected in (
            ({'mem_available': 268435455}, 'MemAvailable'),
            ({'resident_bytes': 536870913}, 'frontend_rss'),
            ({'output_bytes': 67108865}, 'owned_output'),
            ({'scheduled': 370.0, 'started': 370.0, 'ended': 370.000001,
              'spawn_before': 10.0}, 'frontend_wall'),
        ):
            with self.subTest(change=change):
                values = dict(scheduled=10.5, started=10.5, ended=10.6, spawn_before=10.0,
                              frontend_pid=101, frontend_identity='start:101', launcher_pid=77,
                              process_sample=dict(sample), mem_available=268435456,
                              output_scanner=scanner)
                if 'resident_bytes' in change:
                    values['process_sample']['resident_bytes'] = change.pop('resident_bytes')
                if 'output_bytes' in change:
                    scanner.scan.return_value = {'total_bytes': change.pop('output_bytes'), 'breakdown': [], 'digest': 'd'}
                values.update(change)
                self.assertIn(expected, rmc.direct_guard_sample(**values)['failure_reason'])
                scanner.scan.return_value = {'total_bytes': 67108864, 'breakdown': [], 'digest': 'd'}

    def test_guard_rejects_large_overshoot_skipped_grid_and_identity_failures(self):
        scanner = mock.Mock(scan=mock.Mock(return_value={
            'total_bytes': 67108864, 'breakdown': [], 'digest': 'd'}))
        base: dict[str, Any] = dict(
            scheduled=10.5, started=10.5, ended=10.6, spawn_before=10.0,
            frontend_pid=101, frontend_identity='start:101', launcher_pid=77,
            process_sample={'pid': 101, 'start_identity': 'start:101',
                            'parent_pid': 77, 'resident_bytes': 536870912},
            mem_available=268435456, output_scanner=scanner,
        )
        cases = (
            ({'ended': 11.000001}, 'guard_schedule_overrun'),
            ({'process_sample': None}, 'frontend_sample_missing'),
            ({'process_sample': dict(base['process_sample'], pid=102)}, 'frontend_pid_mismatch'),
            ({'process_sample': dict(base['process_sample'], start_identity='reused')},
             'frontend_identity_changed'),
            ({'process_sample': dict(base['process_sample'], parent_pid=88)},
             'frontend_parent_mismatch'),
            ({'process_sample': dict(base['process_sample'], resident_bytes=2 ** 40)},
             'frontend_rss_above_ceiling'),
            ({'mem_available': 0}, 'MemAvailable_below_floor'),
        )
        for changes, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(
                    rmc.direct_guard_sample(**(base | changes))['failure_reason'], expected)
        scanner.scan.return_value = {'total_bytes': 2 ** 40, 'breakdown': [], 'digest': 'jump'}
        row = rmc.direct_guard_sample(**base)
        self.assertEqual(row['failure_reason'], 'owned_output_above_ceiling')
        self.assertEqual(row['owned_output_bytes'], 2 ** 40)

    def test_direct_preflight_rejects_missing_raw_swap_counters(self):
        spec = self._direct_spec()
        self.fx.manifest_obj['candidate']['source_lineage'] = spec['capture_contract']['source_lineage']
        self.fx.manifest.write_text(json.dumps(self.fx.manifest_obj))
        server_identity = json.loads(self.fx.server_identity.read_text())['start_identity']

        def sample(pid, *, timeout):
            identity = server_identity if pid == self.fx.game_server.pid else 'ambient-start'
            return {'pid': pid, 'start_identity': identity, 'resident_bytes': 1,
                    'state': 'S'}

        incomplete = {
            'mem_available_bytes': 805306368,
            'vmstat': {'pswpin': 0, 'pswpout': 0, 'oom_kill': 0},
            'cgroup': {'identity': 'cg', 'boot_id': 'boot',
                       'memory_events': {'oom': 0}},
        }
        with mock.patch.object(rmc.bp, 'verify_direct_owner_build', return_value={
                'status': 'verified', 'files': {}}), \
                mock.patch.object(rmc.pa, 'process_sampler', return_value=sample), \
                self.assertRaisesRegex(rmc.CellError, 'memory counters'):
            rmc.preflight(
                spec, rmc.parse_diagnostic_argv(spec['diagnostic_argv']),
                direct_snapshot=lambda: incomplete,
                disk_usage=lambda _path: mock.Mock(free=268435456),
                sleep=lambda _seconds: None,
                platform_name='linux', machine='x86_64',
            )

    def test_direct_preflight_rejects_the_root_reproduced_semantic_gap(self):
        spec = self._direct_spec()
        self.fx.manifest_obj['candidate']['source_lineage'] = spec['capture_contract']['source_lineage']
        self.fx.manifest.write_text(json.dumps(self.fx.manifest_obj))
        server_identity = json.loads(self.fx.server_identity.read_text())['start_identity']

        def sample(pid, *, timeout):
            identity = server_identity if pid == self.fx.game_server.pid else 'ambient-start'
            return {'pid': pid, 'start_identity': identity, 'resident_bytes': 1, 'state': 'S'}

        snapshot = {
            'mem_available_bytes': 805306368,
            'swap_total_bytes': 0,
            'swap_free_bytes': 0,
            'vmstat': {'pswpin': 0, 'pswpout': 0, 'oom_kill': 0},
            'cgroup': {'identity': 'cg', 'boot_id': 'boot',
                       'memory_events': {'oom': 0, 'oom_kill': 0}},
        }
        cases = {
            'untyped': {'anything': True},
            'expired': {'expires_unix': 0, 'observed_unix': 0},
            'rejected': {'admitted': False, 'status': 'rejected'},
            'wrong_identity_and_conflict': {
                'server_pid': -1, 'server_start_identity': 'wrong',
                'matches': [{'pid': 99, 'start_identity': 'wrong', 'class': 'frontend'}],
            },
        }
        for name, receipt in cases.items():
            with self.subTest(name=name):
                for path in spec['capture_contract']['admission_receipts'].values():
                    pathlib.Path(path).write_text(json.dumps(receipt))
                pathlib.Path(spec['capture_contract']['release_contract']).write_text(
                    json.dumps(receipt)
                )
                with mock.patch.object(rmc.bp, 'verify_direct_owner_build', return_value={
                        'status': 'verified', 'files': {}}), \
                        mock.patch.object(rmc.pa, 'process_sampler', return_value=sample), \
                        mock.patch.object(rmc.subprocess, 'Popen') as popen:
                    with self.assertRaises(rmc.CellError):
                        rmc.preflight(
                            spec, rmc.parse_diagnostic_argv(spec['diagnostic_argv']),
                            direct_snapshot=mock.Mock(side_effect=[copy.deepcopy(snapshot),
                                                                   copy.deepcopy(snapshot)]),
                            disk_usage=lambda _path: mock.Mock(free=268435456),
                            sleep=lambda _seconds: None,
                            platform_name='linux', machine='x86_64',
                        )
                    popen.assert_not_called()

    def test_direct_receipt_and_release_schema_mutations_fail_closed(self):
        spec = self._direct_spec()
        self.fx.manifest_obj['candidate']['source_lineage'] = spec['capture_contract']['source_lineage']
        self.fx.manifest.write_text(json.dumps(self.fx.manifest_obj))

        def run(fixtures, probe=None):
            with mock.patch.object(rmc.bp, 'verify_direct_owner_build',
                                   return_value=fixtures['provenance']), \
                    mock.patch.object(rmc.pa, 'process_sampler',
                                      return_value=fixtures['sample']):
                return rmc.preflight(
                    spec, rmc.parse_diagnostic_argv(spec['diagnostic_argv']),
                    direct_snapshot=mock.Mock(side_effect=[fixtures['snapshot'](),
                                                           fixtures['snapshot']()]),
                    disk_usage=lambda _path: mock.Mock(free=268435456),
                    sleep=lambda _seconds: None,
                    platform_name='linux', machine='x86_64',
                    wall_time=lambda: fixtures['now'],
                    server_probe=lambda: probe or {
                        'host': '127.0.0.1', 'port': 43594, 'succeeded': True,
                        'observed_start_unix_s': 199.0, 'observed_end_unix_s': 199.1,
                    },
                    executable_basename=lambda _pid: 'python3',
                )

        def mutate_receipt(kind, change):
            fixtures = self._direct_preflight_fixtures(spec)
            path = pathlib.Path(spec['capture_contract']['admission_receipts'][kind])
            value = json.loads(path.read_text())
            change(value)
            path.write_text(json.dumps(value, sort_keys=True))
            release = fixtures['release']
            release['receipt_bindings'][kind]['sha256'] = _sha(path)
            fixtures['release_path'].write_text(json.dumps(release, sort_keys=True))
            with self.assertRaises(rmc.CellError):
                run(fixtures)

        # Every receipt kind rejects omitted/unknown fields, rejection, stale
        # observations, and an identity mismatch even when root re-hashes it.
        for kind in ('conflict', 'account', 'population', 'cache', 'server_health'):
            for label, change in (
                ('missing', lambda value: value.pop('evidence')),
                ('unknown', lambda value: value.__setitem__('unknown', True)),
                ('rejected', lambda value: value.__setitem__('admitted', False)),
                ('stale', lambda value: value.__setitem__('observed_end_unix_s', 99.0)),
                ('identity', lambda value: value['context'].__setitem__(
                    'binary_sha256', '0' * 64)),
            ):
                with self.subTest(kind=kind, mutation=label):
                    mutate_receipt(kind, change)

        # Common schema fields are exact in both name and JSON type.
        for field, bad in (
            ('schema', 1), ('kind', None), ('mode', ['direct-owner-v1']),
            ('n', True), ('workload', 1), ('frontend', {}), ('admitted', 1),
            ('observed_start_unix_s', '101'), ('observed_end_unix_s', float('inf')),
            ('boot_id', ''), ('context', []),
        ):
            with self.subTest(field=field):
                mutate_receipt('account', lambda value, field=field, bad=bad:
                               value.__setitem__(field, bad))

        specialized = (
            ('conflict', lambda value: value['evidence'].__setitem__(
                'checked_classes', 'build,test,profiler,tui-panel-frontend')),
            ('conflict', lambda value: value['evidence'].__setitem__(
                'checked_classes', ['build', 'test', 'profiler'])),
            ('conflict', lambda value: value['evidence'].__setitem__(
                'checked_classes', ['build', 'test', 'profiler', 'unknown'])),
            ('conflict', lambda value: value['evidence'].__setitem__('matches', {})),
            ('conflict', lambda value: value['evidence'].__setitem__('matches', [{
                'class': 'test', 'pid': 99, 'start_identity': 'start:99',
                'executable_basename': 'python3',
            }])),
            ('conflict', lambda value: value['evidence'].__setitem__('matches', [{
                'class': 'test', 'pid': True, 'start_identity': 'start:99',
                'executable_basename': 'python3',
            }])),
            ('conflict', lambda value: value['evidence'].__setitem__('matches', [{
                'class': 'test', 'pid': 99, 'start_identity': '',
                'executable_basename': 'python3',
            }])),
            ('conflict', lambda value: value['evidence'].__setitem__('matches', [{
                'class': 'test', 'pid': 99, 'start_identity': 'start:99',
                'executable_basename': '/usr/bin/python3',
            }])),
            ('account', lambda value: value['evidence'].__setitem__(
                'fixture_binding_sha256', '0' * 64)),
            ('account', lambda value: value['evidence'].__setitem__('slot_count', 2)),
            ('account', lambda value: value['evidence'].__setitem__('active_slot_count', True)),
            ('population', lambda value: value['evidence'].__setitem__(
                'fixture_binding_sha256', '0' * 64)),
            ('population', lambda value: value['evidence'].__setitem__('requested_n', True)),
            ('population', lambda value: value['evidence'].__setitem__('admitted_n', 2)),
            ('population', lambda value: value['evidence'].__setitem__('workload', 'idle')),
            ('cache', lambda value: value['evidence'].__setitem__(
                'fixture_binding_sha256', '0' * 64)),
            ('cache', lambda value: value['evidence'].__setitem__(
                'content_identity_sha256', '0' * 64)),
            ('cache', lambda value: value['evidence'].__setitem__('snapshot_version', 'bad')),
            ('server_health', lambda value: value['evidence'].__setitem__('pid', -1)),
            ('server_health', lambda value: value['evidence'].__setitem__(
                'start_identity', 'wrong')),
            ('server_health', lambda value: value['evidence'].__setitem__(
                'executable_basename', '/usr/bin/python3')),
            ('server_health', lambda value: value['evidence']['probe'].__setitem__(
                'succeeded', False)),
            ('server_health', lambda value: value['evidence']['probe'].__setitem__(
                'host', '0.0.0.0')),
            ('server_health', lambda value: value['evidence']['probe'].__setitem__(
                'port', 43595)),
        )
        for index, (kind, change) in enumerate(specialized):
            with self.subTest(kind=kind, specialized=index):
                mutate_receipt(kind, change)

        # Each shared identity is compared with independently measured current
        # state rather than accepted merely because all receipts repeat it.
        fixtures = self._direct_preflight_fixtures(spec)
        for context_field in fixtures['context']:
            with self.subTest(context_field=context_field):
                mutate_receipt(
                    'account',
                    lambda value, context_field=context_field:
                    value['context'].__setitem__(context_field, None),
                )

        fixtures = self._direct_preflight_fixtures(spec)
        left = pathlib.Path(spec['capture_contract']['admission_receipts']['account'])
        right = pathlib.Path(spec['capture_contract']['admission_receipts']['population'])
        left_bytes, right_bytes = left.read_bytes(), right.read_bytes()
        left.write_bytes(right_bytes)
        right.write_bytes(left_bytes)
        fixtures['release']['receipt_bindings']['account']['sha256'] = _sha(left)
        fixtures['release']['receipt_bindings']['population']['sha256'] = _sha(right)
        fixtures['release_path'].write_text(json.dumps(fixtures['release'], sort_keys=True))
        with self.assertRaises(rmc.CellError):
            run(fixtures)

        release_mutations = (
            lambda value: value.__setitem__('unknown', True),
            lambda value: value.pop('expires_unix_s'),
            lambda value: value.__setitem__('schema', 'unknown'),
            lambda value: value.__setitem__('expires_unix_s', 199.0),
            lambda value: value['observation_window'].__setitem__('not_after_unix_s', 301.0),
            lambda value: value['context'].__setitem__('server_start_identity', 'wrong'),
            lambda value: value['spec_binding'].__setitem__('binary_path', '/wrong'),
            lambda value: value['receipt_bindings']['cache'].__setitem__('sha256', '0' * 64),
        )
        for index, change in enumerate(release_mutations):
            fixtures = self._direct_preflight_fixtures(spec)
            release = fixtures['release']
            change(release)
            fixtures['release_path'].write_text(json.dumps(release, sort_keys=True))
            with self.subTest(release_mutation=index), self.assertRaises(rmc.CellError):
                run(fixtures)

        fixtures = self._direct_preflight_fixtures(spec)
        stale_current_probe = {
            'host': '127.0.0.1', 'port': 43594, 'succeeded': True,
            'observed_start_unix_s': 119.0, 'observed_end_unix_s': 119.1,
        }
        with self.assertRaisesRegex(rmc.CellError, 'current loopback server probe'):
            run(fixtures, stale_current_probe)

    def test_direct_preflight_boundaries_counter_drift_and_receipt_mutation(self):
        spec = self._direct_spec()
        self.fx.manifest_obj['candidate']['source_lineage'] = spec['capture_contract']['source_lineage']
        self.fx.manifest.write_text(json.dumps(self.fx.manifest_obj))
        fixtures = self._direct_preflight_fixtures(spec)
        sample = fixtures['sample']
        snapshot = fixtures['snapshot']
        injected = {
            'wall_time': lambda: fixtures['now'],
            'server_probe': lambda: {
                'host': '127.0.0.1', 'port': 43594, 'succeeded': True,
                'observed_start_unix_s': 199.0, 'observed_end_unix_s': 199.1,
            },
            'executable_basename': lambda _pid: 'python3',
        }

        args = rmc.parse_diagnostic_argv(spec['diagnostic_argv'])
        patches = (
            mock.patch.object(rmc.bp, 'verify_direct_owner_build',
                              return_value=fixtures['provenance']),
            mock.patch.object(rmc.pa, 'process_sampler', return_value=sample),
        )
        with patches[0], patches[1]:
            accepted = rmc.preflight(
                spec, args, direct_snapshot=mock.Mock(side_effect=[snapshot(), snapshot()]),
                disk_usage=lambda _path: mock.Mock(free=268435456),
                sleep=lambda _seconds: None, platform_name='linux', machine='x86_64',
                **injected)
            self.assertEqual(accepted['direct_preflight']['output_free_bytes'], 268435456)
            self.assertEqual(accepted['direct_preflight']['server_probe']['port'], 43594)
            self.assertIn('release_contract', accepted['direct_preflight']['admission_bindings'])

        for label, before, after, free in (
            ('mem', snapshot(mem_available_bytes=805306367), snapshot(), 268435456),
            ('disk', snapshot(), snapshot(), 268435455),
            ('swap', snapshot(swap_total_bytes=2, swap_free_bytes=1), snapshot(), 268435456),
            ('vmstat', snapshot(), snapshot(vmstat={'pswpin': 0, 'pswpout': 0,
                                                    'oom_kill': 1}), 268435456),
            ('cgroup', snapshot(), snapshot(cgroup={
                'identity': 'cg', 'boot_id': 'other-boot',
                'memory_events': {'oom': 0, 'oom_kill': 0}}), 268435456),
        ):
            with self.subTest(label=label), \
                    mock.patch.object(rmc.bp, 'verify_direct_owner_build',
                                      return_value=fixtures['provenance']), \
                    mock.patch.object(rmc.pa, 'process_sampler', return_value=sample), \
                    self.assertRaises(rmc.CellError):
                rmc.preflight(
                    spec, args, direct_snapshot=mock.Mock(side_effect=[before, after]),
                    disk_usage=lambda _path, free=free: mock.Mock(free=free),
                    sleep=lambda _seconds: None, platform_name='linux', machine='x86_64',
                    **injected)

        receipt = pathlib.Path(spec['capture_contract']['admission_receipts']['conflict'])
        def mutate_receipt(_seconds):
            receipt.write_text('{"changed": true}\n')
        with mock.patch.object(rmc.bp, 'verify_direct_owner_build',
                               return_value=fixtures['provenance']), \
                mock.patch.object(rmc.pa, 'process_sampler', return_value=sample), \
                self.assertRaisesRegex(rmc.CellError, 'admission receipt changed'):
            rmc.preflight(
                spec, args, direct_snapshot=mock.Mock(side_effect=[snapshot(), snapshot()]),
                disk_usage=lambda _path: mock.Mock(free=268435456),
                sleep=mutate_receipt, platform_name='linux', machine='x86_64',
                **injected)

        self._direct_preflight_fixtures(spec)

        changing_executable = mock.Mock(side_effect=['python3', 'other-server'])
        with mock.patch.object(rmc.bp, 'verify_direct_owner_build',
                               return_value=fixtures['provenance']), \
                mock.patch.object(rmc.pa, 'process_sampler', return_value=sample), \
                self.assertRaisesRegex(rmc.CellError, 'executable identity changed'):
            rmc.preflight(
                spec, args, direct_snapshot=mock.Mock(side_effect=[snapshot(), snapshot()]),
                disk_usage=lambda _path: mock.Mock(free=268435456),
                sleep=lambda _seconds: None, platform_name='linux', machine='x86_64',
                executable_basename=changing_executable,
                wall_time=injected['wall_time'], server_probe=injected['server_probe'],
            )

        def zombie_sample(pid, *, timeout):
            value = sample(pid, timeout=timeout)
            if pid == self.fx.game_server.pid:
                value['state'] = 'Z'
            return value
        with mock.patch.object(rmc.bp, 'verify_direct_owner_build',
                               return_value=fixtures['provenance']), \
                mock.patch.object(rmc.pa, 'process_sampler', return_value=zombie_sample), \
                self.assertRaisesRegex(rmc.CellError, 'zombie'):
            rmc.preflight(
                spec, args, direct_snapshot=mock.Mock(side_effect=[snapshot(), snapshot()]),
                disk_usage=lambda _path: mock.Mock(free=268435456),
                sleep=lambda _seconds: None, platform_name='linux', machine='x86_64',
                **injected)

    def test_direct_binding_toctou_after_preflight_stops_before_popen(self):
        spec = self._direct_spec()
        self.fx.manifest_obj['candidate']['source_lineage'] = spec['capture_contract']['source_lineage']
        self.fx.manifest.write_text(json.dumps(self.fx.manifest_obj))
        fixtures = self._direct_preflight_fixtures(spec)
        server_sample = fixtures['sample'](self.fx.game_server.pid, timeout=2)
        ambient_sample = fixtures['sample'](self.fx.helper.pid, timeout=2)
        admission_bindings = {
            'release_contract': {
                'path': str(fixtures['release_path'].resolve()),
                'sha256': _sha(fixtures['release_path']),
            },
            **{
                'receipt_' + kind: {
                    'path': str(pathlib.Path(path).resolve()),
                    'sha256': _sha(pathlib.Path(path)),
                }
                for kind, path in spec['capture_contract']['admission_receipts'].items()
            },
        }
        pf = {
            'provenance': fixtures['provenance'],
            'server_sample': server_sample,
            'server_pid': self.fx.game_server.pid,
            'ambient_identities': {
                'ambient_helper': {'pid': self.fx.helper.pid, 'sample': ambient_sample},
            },
            'binary': str(self.fx.binary.resolve()),
            'observe_s': 120.0, 'warmup_s': 30.0,
            'direct_preflight': {
                'admission_bindings': admission_bindings,
                'release_expires_unix_s': time.time() + 60.0,
            },
            'direct_cache_snapshot': fixtures['cache'],
        }
        receipt = pathlib.Path(spec['capture_contract']['admission_receipts']['account'])
        release = fixtures['release_path']
        original_receipt = receipt.read_bytes()
        original_release = release.read_bytes()
        original_nav = self.fx.nav_pack.read_bytes()
        real_wall_time = time.time
        real_create = mr.create_launch
        spec_path = pathlib.Path(spec['capture_contract']['owned_output_paths'][1])
        spec_path.parent.mkdir(parents=True, exist_ok=True)
        spec_path.write_text(json.dumps(spec))
        cell_dir = pathlib.Path(spec['capture_contract']['cell_dir'])
        for label, mutate, current_time in (
            ('admission_receipt', lambda: receipt.write_text('{"changed":true}\n'), None),
            ('release_contract', lambda: release.write_text('{"changed":true}\n'), None),
            ('provenance_file', lambda: self.fx.nav_pack.write_text('changed\n'), None),
            ('expired_release', lambda: None,
             pf['direct_preflight']['release_expires_unix_s'] + 1.0),
        ):
            with self.subTest(binding=label):
                receipt.write_bytes(original_receipt)
                release.write_bytes(original_release)
                self.fx.nav_pack.write_bytes(original_nav)
                if cell_dir.exists():
                    shutil.rmtree(cell_dir)

                def mutate_after_launch_receipt(*args, **kwargs):
                    result = real_create(*args, **kwargs)
                    mutate()
                    return result

                with mock.patch.object(rmc, 'preflight', return_value=pf), \
                        mock.patch.object(rmc, '_role_identity',
                                          side_effect=lambda pid, **_kwargs: {
                                              'pid': pid, 'start_identity': f'start:{pid}'}), \
                        mock.patch.object(
                            rmc.mr, 'create_launch', side_effect=mutate_after_launch_receipt), \
                        mock.patch.object(
                            rmc.time, 'time', side_effect=lambda current_time=current_time:
                            current_time if current_time is not None else real_wall_time()), \
                        mock.patch.object(rmc.subprocess, 'Popen') as popen:
                    report = rmc.run_managed_cell(
                        spec_path, cell_dir.parent,
                        accounting_script=ACCOUNTING, _test_launcher=True,
                    )
                popen.assert_not_called()
                self.assertFalse(report['launched'])
                self.assertEqual(report['status'], 'failed_or_unavailable')

    def test_conpty_helper_handoff_missing_fails_closed(self):
        with mock.patch.object(rmc.sys, 'platform', 'win32'):
            with self.assertRaises(rmc.CellError):
                rmc._capture_conpty_helpers({}, 100, set(), 'system')

    def test_windows_non_conpty_handoff_does_not_require_helpers(self):
        with mock.patch.object(rmc.sys, 'platform', 'win32'):
            self.assertEqual(
                rmc._capture_conpty_helpers(
                    {'terminal_transport': 'panel'}, 100, set(), 'system', required=False
                ),
                {},
            )

    def test_conpty_discovery_empty_fails_closed(self):
        fake_parent = mock.Mock()
        fake_parent.child_processes.return_value = []
        fake_parent.select_conpty_helpers.return_value = []
        with mock.patch.dict(sys.modules, {"windows_process_parent": fake_parent}):
            with self.assertRaises(RuntimeError):
                rd.conpty_helper_handoff(launcher_pid=100, frontend_pid=200, platform="win32")
        fake_parent.child_processes.assert_called_once_with(100)
        fake_parent.select_conpty_helpers.assert_called_once_with([], frontend_pid=200)

    def test_unix_collector_starts_once_before_metadata_processing(self):
        calls = []
        real_popen = rmc.subprocess.Popen

        def recording_popen(argv, *args, **kwargs):
            calls.append(tuple(str(value) for value in argv))
            return real_popen(argv, *args, **kwargs)

        with mock.patch.object(rmc.subprocess, "Popen", side_effect=recording_popen):
            report = self._run(self.fx.base_spec(observe=.4, teardown=.5, interval=.15))
        self.assertEqual(report["status"], "completed", report)
        accounting_calls = [call for call in calls if str(ACCOUNTING) in call]
        launcher_calls = [call for call in calls if str(self.fx.fixture_launcher) in call]
        self.assertEqual(len(accounting_calls), 1, calls)
        self.assertEqual(len(launcher_calls), 1, calls)
        self.assertLess(calls.index(launcher_calls[0]), calls.index(accounting_calls[0]), calls)
        self.assertNotIn("conpty_helper_0=", " ".join(accounting_calls[0]))

    def test_windows_conpty_starts_once_after_helpers_are_bound(self):
        """The delayed Windows start includes the handed-off helper role."""
        calls = []
        real_popen = rmc.subprocess.Popen
        native_platform = sys.platform
        if native_platform == "win32":
            import windows_process_parent as native_parent
            read_native_parent = native_parent.parent_pid
        else:
            read_native_parent = rmc.parent_pid

        def recording_popen(argv, *args, **kwargs):
            calls.append(tuple(str(value) for value in argv))
            return real_popen(argv, *args, **kwargs)

        fake_parent = mock.Mock()
        def portable_parent(pid):
            with mock.patch.object(sys, "platform", native_platform):
                return read_native_parent(pid)

        fake_parent.parent_pid.side_effect = portable_parent

        def capture_helpers(meta, launcher_pid, forbidden, backend, *, required=True):
            self.assertTrue(required)
            helper = meta["conpty_helpers"][0]
            return {
                "conpty_helper_0": {
                    "pid": helper["pid"],
                    "start_identity": rmc._start_identity(helper["pid"], backend=backend),
                }
            }

        real_sampler = rmc.pa.process_sampler("system")

        def portable_sampler(pid, *, timeout):
            # The production Windows branch is selected by the runner patch;
            # keep fixture identity sampling on this host's native backend.
            old_platform = sys.platform
            sys.platform = native_platform
            try:
                return real_sampler(pid, timeout=timeout)
            finally:
                sys.platform = old_platform

        with mock.patch.object(rmc.sys, "platform", "win32"), \
                mock.patch.dict(sys.modules, {"windows_process_parent": fake_parent}), \
                mock.patch.object(rmc.pa, "process_sampler", return_value=portable_sampler), \
                mock.patch.object(rmc, "_capture_conpty_helpers", side_effect=capture_helpers), \
                mock.patch.object(rmc.subprocess, "Popen", side_effect=recording_popen):
            report = self._run(
                self.fx.base_spec(
                    observe=.4, teardown=.6, interval=.15, mode="conpty_fake", cell_id="conpty_once"
                )
            )
        # The simulated win32 cleanup cannot query a Unix frontend after it
        # exits; the durable assertions below target delayed collector wiring.
        self.assertIn(report["status"], ("completed", "failed_or_unavailable"), report)
        self.assertEqual(report["sampler_result"]["exit_code"], 0, report)
        accounting_calls = [call for call in calls if str(ACCOUNTING) in call]
        launcher_calls = [call for call in calls if str(self.fx.fixture_launcher) in call]
        self.assertEqual(len(accounting_calls), 1, calls)
        self.assertEqual(len(launcher_calls), 1, calls)
        self.assertLess(calls.index(launcher_calls[0]), calls.index(accounting_calls[0]), calls)
        self.assertIn("conpty_helper_0=", " ".join(accounting_calls[0]))
        self.assertIn("conpty_helper_0", report["sampler_result"]["role_identities"])
        self.assertEqual(
            report["sampler_result"]["conpty_helpers"],
            {"conpty_helper_0": report["sampler_result"]["role_identities"]["conpty_helper_0"]},
        )

    def test_conpty_helper_handoff_identity_and_parent_are_bound(self):
        fake_parent = mock.Mock()
        fake_parent.parent_pid.return_value = 100
        with mock.patch.dict(sys.modules, {'windows_process_parent': fake_parent}):
            with mock.patch.object(rmc.sys, 'platform', 'win32'), \
                    mock.patch.object(rmc, '_start_identity', return_value='creation:7'):
                result = rmc._capture_conpty_helpers(
                    {'conpty_helpers': [{'pid': 200, 'parent_pid': 100,
                                         'image_name': 'conhost.exe',
                                         'start_identity': 'creation:7'}]},
                    100, {300}, 'system')
        self.assertEqual(result, {'conpty_helper_0': {'pid': 200, 'start_identity': 'creation:7'}})
        fake_parent.parent_pid.assert_called_once_with(200)

    def test_conpty_helper_handoff_rejects_pid_reuse(self):
        fake_parent = mock.Mock()
        fake_parent.parent_pid.return_value = 100
        with mock.patch.dict(sys.modules, {'windows_process_parent': fake_parent}):
            with mock.patch.object(rmc.sys, 'platform', 'win32'), \
                    mock.patch.object(rmc, '_start_identity', return_value='creation:new'):
                with self.assertRaises(rmc.CellError):
                    rmc._capture_conpty_helpers(
                        {'conpty_helpers': [{'pid': 200, 'parent_pid': 100,
                                             'image_name': 'conhost.exe',
                                             'start_identity': 'creation:old'}]},
                        100, set(), 'system')

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

    def test_default_collector_stops_after_pad_before_generated_stop_and_c(self):
        stop_requests = []
        real_request = rmc._request_collector_stop

        def record_request(collector, stop_path, **kwargs):
            stop_requests.append({
                'monotonic_s': time.monotonic(),
                'collector_alive': collector is not None and collector.poll() is None,
            })
            return real_request(collector, stop_path, **kwargs)

        spec = self.fx.base_spec(
            observe=.3, teardown=1.2, mode='lifecycle', interval=.1,
            cell_id='default_lifecycle',
        )
        with mock.patch.object(
            rmc, '_request_collector_stop', side_effect=record_request
        ):
            report = self._run(spec)
        self.assertEqual(report['status'], 'completed', report)
        self.assertEqual(len(stop_requests), 1, stop_requests)
        self.assertTrue(stop_requests[0]['collector_alive'], stop_requests)
        events = {
            row['event']: row['monotonic_s']
            for row in (
                json.loads(line)
                for line in (pathlib.Path(report['run_dir']) / 'samples.lifecycle.jsonl')
                .read_text().splitlines()
            )
        }
        self.assertEqual(set(events), {'observe-end', 'Stop', 'C', 'launcher-exit'})
        self.assertGreaterEqual(
            stop_requests[0]['monotonic_s'], events['observe-end'] + .2
        )
        self.assertLess(stop_requests[0]['monotonic_s'], events['Stop'])
        self.assertLess(stop_requests[0]['monotonic_s'], events['C'])
        self.assertLess(stop_requests[0]['monotonic_s'], events['launcher-exit'])

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
        signals = report['cleanup']['frontend']['signals']
        # Unix: soft TERM ignored → hard SIGKILL. Windows: soft is already
        # TerminateProcess (fatal); hard label is TERMINATE if escalation runs.
        hard = rmc._hard_signal_label()
        self.assertTrue(signals, report)
        if hard == 'SIGKILL':
            self.assertIn('SIGKILL', signals, report)
        else:
            self.assertTrue(
                any(s in ('SIGTERM', hard) for s in signals),
                report,
            )
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

    def test_requested_backend_spec_matches_diagnostic_argv(self):
        """spec.requested_backend must agree with diagnostic --cpu-fallback."""
        # Default tui: requested_backend none (implicit) is fine.
        ok = rmc.validate_spec(self.fx.base_spec() | {"requested_backend": "none"})
        self.assertEqual(ok["requested_backend"], "none")
        with self.assertRaises(rmc.CellError):
            rmc.validate_spec(self.fx.base_spec() | {"requested_backend": "vulkan"})

        # Panel CPU argv vs GPU label is rejected at argv consistency.
        panel_cpu = self.fx.base_spec(cell_id="cpu_mismatch")
        panel_cpu["frontend"] = "panel"
        panel_cpu["diagnostic_argv"] = [
            "panel", "1", "idle",
            "--binary", str(self.fx.binary),
            "--build-manifest", str(self.fx.manifest),
            "--build-role", "candidate",
            "--observe", "1", "--warmup", "0",
            "--cpu-fallback",
        ]
        panel_cpu["requested_backend"] = "gpu"
        report = self._run(panel_cpu)
        self.assertEqual(report["status"], "preflight_failed", report)
        self.assertTrue(
            "requested_backend" in str(report.get("error", "")).lower()
            or "cpu" in str(report.get("error", "")).lower()
            or "preflight" in report["status"],
            report,
        )

        # Matching cpu_fallback label is accepted by validate + argv check (preflight
        # may still fail later on fixtures; we only assert consistency path).
        panel_ok = dict(panel_cpu)
        panel_ok["id"] = "cpu_ok_argv"
        panel_ok["requested_backend"] = "cpu_fallback"
        args = rmc.parse_diagnostic_argv(panel_ok["diagnostic_argv"])
        rmc.require_argv_consistent_with_spec(panel_ok, args, _test_launcher=True)
        self.assertEqual(rd.requested_backend(args), "cpu_fallback")

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
        expected = {"process_accounting.py", "server_resources.py"}
        if sys.platform == "win32":
            # Producer-host system counters live in windows_process_sample.py.
            expected.add("windows_process_sample.py")
        self.assertEqual(set(modules), expected)
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
        if sys.platform == "win32":
            self.assertEqual(
                pathlib.Path(modules["windows_process_sample.py"]["path"]).resolve(),
                WINDOWS_PROCESS_SAMPLE.resolve(),
            )
            self.assertEqual(
                modules["windows_process_sample.py"]["sha256"], _sha(WINDOWS_PROCESS_SAMPLE)
            )
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

    def test_windows_sampler_module_bound_when_producer_is_win32(self):
        """Launch pin includes windows_process_sample.py only for win32 system producer."""
        roles = {"game_server": 1, "controller": 2, "ambient": 3}
        argv = [sys.executable, str(ACCOUNTING), "out.jsonl"]
        with mock.patch.object(rmc.sys, "platform", "win32"):
            cfg = rmc._build_sampler_config(
                {"sampler_interval_s": 0.25, "process_backend": "system"},
                roles,
                argv,
                accounting_script=ACCOUNTING,
            )
        self.assertEqual(
            set(cfg["modules"]),
            {"process_accounting.py", "server_resources.py", "windows_process_sample.py"},
        )
        self.assertEqual(
            pathlib.Path(cfg["modules"]["windows_process_sample.py"]["path"]).resolve(),
            WINDOWS_PROCESS_SAMPLE.resolve(),
        )
        self.assertEqual(
            cfg["modules"]["windows_process_sample.py"]["sha256"], _sha(WINDOWS_PROCESS_SAMPLE)
        )
        # Non-Windows producer must not pin the unused Windows module.
        with mock.patch.object(rmc.sys, "platform", "darwin"):
            cfg_mac = rmc._build_sampler_config(
                {"sampler_interval_s": 0.25, "process_backend": "system"},
                roles,
                argv,
                accounting_script=ACCOUNTING,
            )
        self.assertEqual(
            set(cfg_mac["modules"]),
            {"process_accounting.py", "server_resources.py"},
        )
        self.assertNotIn("windows_process_sample.py", cfg_mac["modules"])

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

    def test_successful_cell_uses_stop_file_not_only_sigterm(self):
        spec = self.fx.base_spec(observe=0.5, teardown=0.5, mode="ok", interval=0.15)
        report = self._run(spec)
        self.assertEqual(report["status"], "completed", report)
        stop_path = report.get("collector_stop_path")
        self.assertIsInstance(stop_path, str, report)
        self.assertTrue(pathlib.Path(stop_path).is_file(), report)
        self.assertTrue(stop_path.endswith("collector.stop"), report)
        cell_dir = pathlib.Path(report["cell_dir"]).resolve()
        self.assertEqual(pathlib.Path(stop_path).resolve().parent, cell_dir)
        stop_req = report.get("collector_stop_request") or {}
        self.assertTrue((stop_req.get("stop_file") or {}).get("ok"), report)
        # Sampler completed with controlled_stop via portable channel.
        self.assertEqual(report["sampler_result"].get("completion"), "controlled_stop", report)
        self.assertEqual(report["sampler_result"]["exit_code"], 0, report)
        argv = report.get("sampler_argv") or []
        self.assertIn("--stop-file", argv)
        self.assertIn(stop_path, argv)

    def test_portable_cleanup_signal_labels_no_undefined_sigkill(self):
        # Unit-level: win32 branch must not evaluate signal.SIGKILL.
        self.assertEqual(rmc._soft_signal_label(), "SIGTERM")
        with mock.patch.object(rmc.sys, "platform", "win32"):
            self.assertEqual(rmc._hard_signal_label(), "TERMINATE")
            # Constructing the cleanup stage list must not touch missing SIGKILL.
            stages = []
            for stage in ("soft", "hard"):
                if stage == "soft":
                    stages.append(rmc._soft_signal_label())
                else:
                    stages.append(rmc._hard_signal_label())
            self.assertEqual(stages, ["SIGTERM", "TERMINATE"])
        if hasattr(signal, "SIGKILL"):
            with mock.patch.object(rmc.sys, "platform", "linux"):
                self.assertEqual(rmc._hard_signal_label(), "SIGKILL")

    def test_pid_alive_rejects_invalid_and_detects_live_child(self):
        self.assertFalse(rmc._pid_alive(0))
        self.assertFalse(rmc._pid_alive(-1))
        self.assertFalse(rmc._pid_alive(True))  # type: ignore[arg-type]
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        self.addCleanup(lambda: child.kill() if child.poll() is None else None)
        self.addCleanup(lambda: child.wait(timeout=5) if child.poll() is None else None)
        time.sleep(0.05)
        self.assertTrue(rmc._pid_alive(child.pid))
        child.kill()
        child.wait(timeout=5)
        # Brief settle for Windows process-object teardown.
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline and rmc._pid_alive(child.pid):
            time.sleep(0.05)
        self.assertFalse(rmc._pid_alive(child.pid))

    def test_parent_pid_rejects_foreign_and_accepts_owned_child(self):
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"],
        )

        def _reap():
            if child.poll() is None:
                try:
                    child.kill()
                except OSError:
                    pass
                try:
                    child.wait(timeout=5)
                except Exception:
                    pass

        self.addCleanup(_reap)
        time.sleep(0.05)
        parent = rmc.parent_pid(child.pid)
        self.assertEqual(parent, os.getpid())
        # Owned child of this process is not a child of a foreign launcher PID.
        with self.assertRaises(rmc.CellError):
            rmc._capture_owned_frontend(
                child.pid, launcher_pid=1, forbidden=set(), backend="system"
            )
        # Missing PID fails closed.
        with self.assertRaises(rmc.CellError):
            rmc.parent_pid(2_000_000_001)

    def test_cleanup_rechecks_parent_while_launcher_alive(self):
        # Foreign PID with wrong parent must not be signaled even with matching fake identity.
        helper_pid = self.fx.helper.pid
        identity = rmc._start_identity(helper_pid, backend="system")
        self.assertIsNotNone(identity)
        # Launcher PID is this test process (alive); helper's parent is not us typically
        # if helper was started by FixtureTree — parent is test runner. Use a fake launcher.
        fake_launcher = helper_pid  # distinct process; helper is not child of itself
        # helper's parent is not helper_pid, so parent check fails when launcher "alive".
        result = rmc._cleanup_frontend(
            helper_pid,
            identity,
            backend="system",
            launcher_pid=fake_launcher,
            wait_s=0.01,
        )
        self.assertEqual(result["signals"], [])
        self.assertTrue(result.get("orphan_risk"))
        self.assertTrue(_alive(helper_pid))

    def test_direct_lifecycle_fixture_binds_live_identities_and_release(self):
        now = 1_000.0
        lifecycle = self._direct_lifecycle_fixture(self.fx.fixture_launcher, now=now)
        spec = rmc.validate_spec(json.loads(lifecycle['spec_path'].read_text()))
        fake_pf = lifecycle['fake_pf']
        bindings = fake_pf['direct_preflight']['admission_bindings']

        self.assertEqual(spec, lifecycle['spec'])
        self.assertEqual(
            fake_pf['server_sample']['start_identity'],
            sr.sample_process(self.fx.game_server.pid, timeout=2)['start_identity'],
        )
        self.assertEqual(
            fake_pf['ambient_identities']['ambient_helper']['sample']['start_identity'],
            sr.sample_process(self.fx.helper.pid, timeout=2)['start_identity'],
        )
        self.assertEqual(
            fake_pf['direct_cache_snapshot'],
            cp.capture(spec['cache_dir'], spec['unpack_root']),
        )
        self.assertEqual(
            set(bindings),
            {'release_contract', 'receipt_conflict', 'receipt_account',
             'receipt_population', 'receipt_cache', 'receipt_server_health'},
        )
        release = json.loads(pathlib.Path(spec['capture_contract']['release_contract']).read_text())
        expiry = fake_pf['direct_preflight']['release_expires_unix_s']
        self.assertEqual(release['expires_unix_s'], expiry)
        self.assertGreater(expiry, now)
        bp.recheck_files(bindings)
        validated = rmc.validate_direct_admissions(
            spec,
            fake_pf['provenance'],
            {'cgroup': {'boot_id': 'boot'}},
            fake_pf['server_sample'],
            fake_pf['direct_cache_snapshot'],
            now=now,
            server_probe={
                'host': '127.0.0.1', 'port': 43594, 'succeeded': True,
                'observed_start_unix_s': release['issued_unix_s'],
                'observed_end_unix_s': now,
            },
            server_executable='python3',
        )
        self.assertEqual(validated['bindings'], bindings)
        self.assertEqual(validated['release_expires_unix_s'], expiry)

    @unittest.skipUnless(sys.platform.startswith('linux'), 'direct lifecycle qualification is Linux-only')
    def test_linux_direct_lifecycle_keeps_collector_through_stop_c_and_exit(self):
        launcher = self.fx.root / 'direct_launcher.py'
        launcher.write_text(textwrap.dedent('''\
            #!/usr/bin/env python3
            import argparse, json, os, pathlib, subprocess, sys, time
            p = argparse.ArgumentParser()
            p.add_argument('--direct-owner-capture', action='store_true')
            p.add_argument('--run-dir', type=pathlib.Path, required=True)
            p.add_argument('--frontend-handoff', type=pathlib.Path, required=True)
            p.add_argument('--binary'); p.add_argument('--build-manifest'); p.add_argument('--build-role')
            p.add_argument('--no-diagnostics', action='store_true'); p.add_argument('--sustain', action='store_true')
            p.add_argument('--warmup', type=int); p.add_argument('--observe', type=int)
            p.add_argument('frontend'); p.add_argument('n'); p.add_argument('workload')
            a = p.parse_args()
            before = time.monotonic()
            child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(1.4)'])
            after = time.monotonic()
            stat = (pathlib.Path('/proc') / str(child.pid) / 'stat').read_text()
            start = stat[stat.rfind(')') + 2:].split()[19]
            spawn = {'nonce': 'fixture-nonce', 'launcher_pid': os.getpid(),
                     'frontend_pid': child.pid,
                     'frontend_start_identity': 'linux_proc_start_ticks:' + start,
                     'run_dir': str(a.run_dir.resolve()),
                     'spawn_before_monotonic_s': before, 'spawn_after_monotonic_s': after,
                     'frontend': 'tui', 'terminal': True, 'terminal_size': [120, 40],
                     'n': 1, 'workload': 'active', 'warmup_s': 30, 'observe_s': 120,
                     'teardown_grace_s': 60}
            handoff = {'schema': 1, 'mode': 'direct-owner-v1', 'state': 'spawned', 'spawn': spawn}
            a.frontend_handoff.write_text(json.dumps(handoff, sort_keys=True) + '\\n')
            meta = {'schema': 1, 'run_dir': str(a.run_dir.resolve()), 'pid': child.pid,
                    'capture_mode': 'direct-owner-v1', 'direct_owner_spawn': spawn}
            (a.run_dir / 'metadata.json').write_text(json.dumps(meta, sort_keys=True) + '\\n')
            print('BOT_DIAGNOSTIC_METADATA=' + json.dumps(meta, sort_keys=True), flush=True)
            qualification = a.run_dir / 'samples.qualification.jsonl'
            qualification.write_text(json.dumps(
                {'phase': 'observe-start', 'elapsed_s': 0, 'slots': [{}]}) + '\\n')
            lifecycle = a.run_dir / 'samples.lifecycle.jsonl'
            def mark(event):
                with lifecycle.open('a') as out:
                    out.write(json.dumps(
                        {'event': event, 'monotonic_s': time.monotonic()}) + '\\n')
            time.sleep(.1)
            with qualification.open('a') as out:
                out.write(json.dumps(
                    {'phase': 'observe-end', 'elapsed_s': 1, 'slots': [{}]}) + '\\n')
            mark('observe-end')
            time.sleep(.3)
            mark('Stop')
            time.sleep(.7)
            mark('C')
            rc = child.wait(); wait_return = time.monotonic()
            exited = dict(handoff, state='exited', frontend_start_identity=spawn['frontend_start_identity'],
                          exit_code=rc, wait_return_monotonic_s=wait_return)
            next_path = a.frontend_handoff.with_name('frontend-handoff.next.json')
            next_path.write_text(json.dumps(exited, sort_keys=True) + '\\n')
            os.replace(next_path, a.frontend_handoff)
            meta.update(exit_code=rc, direct_owner_exit={
                'frontend_start_identity': spawn['frontend_start_identity'], 'exit_code': rc,
                'wait_return_monotonic_s': wait_return})
            (a.run_dir / 'metadata.json').write_text(json.dumps(meta, sort_keys=True) + '\\n')
            print('BOT_DIAGNOSTIC_METADATA=' + json.dumps(meta, sort_keys=True), flush=True)
            mark('launcher-exit')
        '''))
        launcher.chmod(0o755)
        lifecycle = self._direct_lifecycle_fixture(launcher, now=time.time())
        result_path = lifecycle['result_path']
        spec_path = lifecycle['spec_path']
        cell_dir = lifecycle['cell_dir']
        run_dir = lifecycle['run_dir']
        handoff = lifecycle['handoff']
        fake_pf = lifecycle['fake_pf']
        stop_requests = []
        real_request = rmc._request_collector_stop

        def record_request(collector, stop_path, **kwargs):
            stop_requests.append({
                'monotonic_s': time.monotonic(),
                'collector_alive': collector is not None and collector.poll() is None,
            })
            return real_request(collector, stop_path, **kwargs)

        with mock.patch.object(rmc, 'preflight', return_value=fake_pf), \
             mock.patch.object(rmc.mr, 'complete', return_value={'status': 'ok', 'binding_errors': []}), \
             mock.patch.object(rmc, '_request_collector_stop', side_effect=record_request):
            report = rmc.run_managed_cell(
                spec_path, result_path.with_suffix(result_path.suffix + '.cells'),
                _test_launcher=True)
        self.assertEqual(report['status'], 'ok', report)
        self.assertEqual(report['direct_owner']['spawn']['launcher_pid'], report['launcher_pid'])
        self.assertIsNotNone(report['direct_owner']['exit'])
        self.assertEqual(report['direct_owner']['exit']['exit_code'], 0)
        self.assertTrue(report['collector_stop_requested'])
        self.assertEqual(report['collector_exit_code'], 0)
        self.assertGreaterEqual(len(stop_requests), 1, stop_requests)
        self.assertTrue(stop_requests[0]['collector_alive'], stop_requests)
        events = {
            row['event']: row['monotonic_s']
            for row in (
                json.loads(line)
                for line in (run_dir / 'samples.lifecycle.jsonl').read_text().splitlines()
            )
        }
        self.assertEqual(set(events), {'observe-end', 'Stop', 'C', 'launcher-exit'})
        self.assertTrue(all(
            request['monotonic_s'] >= events['launcher-exit']
            and request['monotonic_s'] >= events['C']
            for request in stop_requests
        ), stop_requests)
        self.assertGreater(events['launcher-exit'] - events['observe-end'], 1.0)
        self.assertTrue((cell_dir / 'direct-owner-guard.jsonl').is_file())
        self.assertTrue((cell_dir / 'direct-owner-guard-summary.json').is_file())
        self.assertEqual(json.loads(handoff.read_text())['state'], 'exited')


def _alive(pid: int) -> bool:
    """Test helper: portable process liveness (same semantics as run_managed_cell)."""
    return rmc._pid_alive(pid) if isinstance(pid, int) else False


if __name__ == "__main__":
    unittest.main()
