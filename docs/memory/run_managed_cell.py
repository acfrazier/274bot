#!/usr/bin/env python3
"""One-cell managed orchestration: reviewed launcher + schema-2 process accounting.

Explicit predeclared JSON cell spec only — no name/env discovery, no retries.
Does not start, stop, or signal the game server or ambient helper PIDs.
Does not claim qualification, overhead, or performance acceptance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import pathlib
import platform
import re
import selectors
import signal
import shutil
import socket
import stat
import subprocess
import sys
import time
from typing import Any, Dict, List, Mapping, NoReturn, Optional, Sequence

_ROOT = pathlib.Path(__file__).resolve().parent
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

import build_provenance as bp  # noqa: E402
import cache_provenance as cp  # noqa: E402
import managed_receipt as mr  # noqa: E402
import run_diagnostic as rd  # noqa: E402
import server_resources as sr  # noqa: E402
import process_accounting as pa  # noqa: E402
from operator_home import unpack_root_path  # noqa: E402

CLEANUP_WAIT_S = 15.0
SAMPLE_TIMEOUT_S = 2.0
DEFAULT_ACCOUNTING = _ROOT / "process_accounting.py"
DEFAULT_SERVER_RESOURCES = _ROOT / "server_resources.py"
DEFAULT_NATIVE_PROCESS_SAMPLE = _ROOT / "native_process_sample.py"
DEFAULT_WINDOWS_PROCESS_SAMPLE = _ROOT / "windows_process_sample.py"
DEFAULT_WINDOWS_PROCESS_PARENT = _ROOT / "windows_process_parent.py"
COLLECTOR_STOP_BASENAME = "collector.stop"
DIRECT_MODE = 'direct-owner-v1'
DIRECT_INTERVAL_S = 0.5
DIRECT_MEM_AVAILABLE_BYTES = 268435456
DIRECT_FRONTEND_RSS_BYTES = 536870912
DIRECT_OUTPUT_BYTES = 67108864
DIRECT_FRONTEND_WALL_S = 360.0
DIRECT_OUTER_WALL_S = 365.0
DIRECT_PREFLIGHT_MEM_AVAILABLE_BYTES = 805306368
DIRECT_PREFLIGHT_DISK_FREE_BYTES = 268435456
DIRECT_SERVER_PORT = 43594
DIRECT_ADMISSION_KINDS = ('conflict', 'account', 'population', 'cache', 'server_health')
DIRECT_CONFLICT_CLASSES = ('build', 'test', 'profiler', 'tui-panel-frontend')
DIRECT_SERVER_EXECUTABLE_IDENTITIES = ('proc-exe', 'sudo-readlink-v1')
DIRECT_IDENTITY_HELPER_TIMEOUT_S = 2.0
DIRECT_IDENTITY_STDOUT_LIMIT = 4096
DIRECT_IDENTITY_STDERR_LIMIT = 4096


class CellError(RuntimeError):
    """Managed-cell configuration or preflight failure."""


def _utc() -> str:
    return mr.utc_now()


def _is_pos_num(value: Any) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and value > 0
        and math.isfinite(value)
    )


def load_spec(path: pathlib.Path) -> Dict[str, Any]:
    data = json.loads(pathlib.Path(path).read_text())
    if not isinstance(data, dict):
        raise CellError("cell spec must be a JSON object")
    return data


def validate_spec(spec: Mapping[str, Any]) -> Dict[str, Any]:
    """Fail closed on missing/invalid fields. Returns a normalized copy."""
    out = dict(spec)
    cell_id = out.get("id")
    if not isinstance(cell_id, str) or re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', cell_id) is None:
        raise CellError("spec.id must be a safe single path component")
    index = out.get("index")
    if type(index) is not int or index < 1:
        raise CellError("spec.index must be an integer >= 1")
    kind = out.get("kind")
    if kind not in ("matched", "overhead", "diagnostic"):
        raise CellError("spec.kind must be matched|overhead|diagnostic")
    for key in (
        "binary",
        "build_manifest",
        "server_identity_path",
        "host_conditions_path",
        "nav_pack",
        "nav_flags",
        "catalog_path",
    ):
        if not isinstance(out.get(key), str) or not out[key]:
            raise CellError(f"spec.{key} must be a non-empty path string")
    if out.get("build_role") not in ("reference", "candidate"):
        raise CellError("spec.build_role must be reference|candidate")
    if out.get("frontend") not in ("tui", "panel"):
        raise CellError("spec.frontend must be tui|panel")
    argv = out.get("launcher_argv")
    if not isinstance(argv, list) or not argv or not all(isinstance(x, str) for x in argv):
        raise CellError("spec.launcher_argv must be a non-empty list of strings")
    diag = out.get("diagnostic_argv")
    if not isinstance(diag, list) or not diag or not all(isinstance(x, str) for x in diag):
        raise CellError("spec.diagnostic_argv must be a non-empty list of strings")
    gs = out.get("game_server_pid")
    if type(gs) is not int or isinstance(gs, bool) or gs <= 0:
        raise CellError("spec.game_server_pid must be a positive int")
    ambient = out.get("ambient_helpers") or {}
    if not isinstance(ambient, dict):
        raise CellError("spec.ambient_helpers must be an object of role→pid")
    cleaned_ambient: Dict[str, int] = {}
    for name, pid in ambient.items():
        if not isinstance(name, str) or not name.strip() or name != name.strip():
            raise CellError("ambient helper role names must be non-empty strings")
        if name in ('game_server', 'controller', 'launcher', 'collector', 'sampler'):
            raise CellError('reserved ambient role name')
        if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
            raise CellError(f"ambient helper {name!r} pid must be a positive int")
        cleaned_ambient[name] = pid
    out["ambient_helpers"] = cleaned_ambient
    if not _is_pos_num(out.get("sampler_interval_s")):
        raise CellError("spec.sampler_interval_s must be finite positive")
    if not _is_pos_num(out.get("max_wall_s")):
        raise CellError("spec.max_wall_s must be finite positive")
    if "live_max_wall_s" in out:
        if not _is_pos_num(out["live_max_wall_s"]):
            raise CellError("spec.live_max_wall_s must be finite positive")
        windows = out.get("analysis_windows_s")
        if (not isinstance(windows, list) or len(windows) != 2 or
                any(not _is_pos_num(value) for value in windows) or
                out["max_wall_s"] < out["live_max_wall_s"] + sum(windows)):
            raise CellError("spec.analysis_windows_s must declare two bounded post-exit windows")
    for key in ("observe_s", "warmup_s", "teardown_grace_s"):
        if key in out and out[key] is not None:
            val = out[key]
            if not (
                isinstance(val, (int, float))
                and not isinstance(val, bool)
                and val >= 0
                and math.isfinite(val)
            ):
                raise CellError(f"spec.{key} must be a finite nonnegative number")
    out.setdefault('process_backend', 'system')
    if out['process_backend'] not in ('system', 'libproc'):
        raise CellError('invalid process_backend')
    if 'requested_backend' in out and out['requested_backend'] is not None:
        if out['requested_backend'] not in ('gpu', 'cpu_fallback', 'none'):
            raise CellError('spec.requested_backend must be gpu|cpu_fallback|none')
    has_cache = 'cache_dir' in out and out['cache_dir'] is not None
    has_unpack = 'unpack_root' in out and out['unpack_root'] is not None
    if has_cache ^ has_unpack:
        raise CellError('spec.cache_dir and spec.unpack_root must both be set or both omitted')
    if has_cache:
        if not isinstance(out.get('cache_dir'), str) or not out['cache_dir']:
            raise CellError('spec.cache_dir must be a non-empty path string')
        if not isinstance(out.get('unpack_root'), str) or not out['unpack_root']:
            raise CellError('spec.unpack_root must be a non-empty path string')
    heaptrack = out.get('heaptrack')
    if heaptrack is not None:
        if not isinstance(heaptrack, dict) or not isinstance(heaptrack.get('output'), str) or not heaptrack['output']:
            raise CellError('spec.heaptrack must declare an output path')
        if not isinstance(heaptrack.get('preload'), dict) or not heaptrack['preload'].get('path'):
            raise CellError('spec.heaptrack must declare preload metadata')
    pids = [gs, os.getpid(), *cleaned_ambient.values()]
    if len(pids) != len(set(pids)):
        raise CellError('duplicate process role PID')
    contract = out.get('capture_contract')
    if contract is not None:
        _validate_direct_contract(out, contract)
    return out


def _canonical(path: Any) -> pathlib.Path:
    if not isinstance(path, str) or not path:
        raise CellError('direct owner path must be a non-empty string')
    return pathlib.Path(path).resolve()


def _validate_direct_contract(spec: Mapping[str, Any], contract: Any) -> None:
    if not isinstance(contract, dict) or contract.get('mode') != DIRECT_MODE:
        raise CellError('capture_contract must be the complete direct-owner-v1 object')
    expected_keys = {
        'mode', 'cell_dir', 'run_dir', 'frontend_handoff_path', 'owned_output_paths',
        'warmup_s', 'observe_s', 'teardown_grace_s', 'guard_interval_s',
        'handoff_deadline_s', 'mem_available_floor_bytes',
        'frontend_rss_limit_bytes', 'owned_output_limit_bytes',
        'frontend_wall_limit_s', 'outer_wall_limit_s', 'source_lineage',
        'release_contract', 'admission_receipts',
    }
    selector = contract.get('server_executable_identity')
    if selector is not None:
        expected_keys.add('server_executable_identity')
        if selector not in DIRECT_SERVER_EXECUTABLE_IDENTITIES:
            raise CellError('direct owner server executable identity selector is invalid')
    if set(contract) != expected_keys:
        unknown = sorted(set(contract) - expected_keys)
        missing = sorted(expected_keys - set(contract))
        raise CellError(
            f'direct owner capture contract unknown={unknown} missing={missing}'
        )
    exact_top = {
        'n': 1, 'frontend': 'tui', 'kind': 'diagnostic', 'warmup_s': 30,
        'observe_s': 120, 'teardown_grace_s': 60, 'sampler_interval_s': DIRECT_INTERVAL_S,
        'max_wall_s': DIRECT_OUTER_WALL_S, 'process_backend': 'system',
        'requested_backend': 'none', 'heaptrack': None,
    }
    for key, expected in exact_top.items():
        if spec.get(key) != expected:
            raise CellError('direct owner top-level contract mismatch: ' + key)
    exact_contract = {
        'warmup_s': 30, 'observe_s': 120, 'teardown_grace_s': 60,
        'guard_interval_s': DIRECT_INTERVAL_S, 'handoff_deadline_s': 5.0,
        'mem_available_floor_bytes': DIRECT_MEM_AVAILABLE_BYTES,
        'frontend_rss_limit_bytes': DIRECT_FRONTEND_RSS_BYTES,
        'owned_output_limit_bytes': DIRECT_OUTPUT_BYTES,
        'frontend_wall_limit_s': DIRECT_FRONTEND_WALL_S,
        'outer_wall_limit_s': DIRECT_OUTER_WALL_S,
    }
    for key, expected in exact_contract.items():
        if contract.get(key) != expected:
            raise CellError('direct owner capture contract mismatch: ' + key)
    cell_dir = _canonical(contract.get('cell_dir'))
    run_dir = _canonical(contract.get('run_dir'))
    handoff = _canonical(contract.get('frontend_handoff_path'))
    if run_dir != cell_dir / 'frontend-run' or handoff != cell_dir / 'frontend-handoff.json':
        raise CellError('direct owner reserved run/handoff paths mismatch')
    roots = contract.get('owned_output_paths')
    if not isinstance(roots, list) or len(roots) != 4 or any(not isinstance(p, str) for p in roots):
        raise CellError('direct owner requires four owned output paths')
    canonical_roots = [_canonical(path) for path in roots]
    result_path, spec_path, owned_cell, owned_run = canonical_roots
    expected_spec = result_path.with_suffix(result_path.suffix + '.spec.json')
    expected_cell = result_path.with_suffix(result_path.suffix + '.cells') / result_path.stem
    if (len(set(canonical_roots)) != 4 or spec_path != expected_spec
            or owned_cell != expected_cell or owned_cell != cell_dir
            or owned_run != run_dir):
        raise CellError('direct owner owned paths do not match result/spec/cell/run reservation')
    if not isinstance(contract.get('source_lineage'), dict) or not contract['source_lineage']:
        raise CellError('direct owner source lineage missing')
    if (not isinstance(contract.get('release_contract'), str)
            or not contract['release_contract']):
        raise CellError('direct owner root release contract missing')
    if not isinstance(spec.get('cache_dir'), str) or not isinstance(spec.get('unpack_root'), str):
        raise CellError('direct owner cache and unpack roots are required')
    admissions = contract.get('admission_receipts')
    required = set(DIRECT_ADMISSION_KINDS)
    if (not isinstance(admissions, dict) or set(admissions) != required
            or any(not isinstance(path, str) or not path for path in admissions.values())):
        raise CellError('direct owner root admission receipts missing')


class OwnedOutputScanner:
    """lstat-only, inode-deduplicating scanner for explicitly owned roots."""
    def __init__(self, roots, *, replaceable_paths=()):
        self.roots = [pathlib.Path(path) for path in roots]
        self.replaceable = {pathlib.Path(path) for path in replaceable_paths}
        self.identities = {}
        self.known_identities = {}
        self.replacements = set()

    def scan(self):
        inode_sizes = {}
        breakdown = []
        current = {}

        def visit(path, *, required=False):
            try:
                info = path.lstat()
            except FileNotFoundError:
                if required:
                    raise CellError('owned output disappeared during scan: ' + str(path))
                return
            mode = info.st_mode
            if stat.S_ISLNK(mode):
                raise CellError('owned output symlink rejected: ' + str(path))
            identity = (info.st_dev, info.st_ino, stat.S_IFMT(mode))
            previous = self.known_identities.get(path)
            if previous is not None and previous != identity:
                if path not in self.replaceable or path in self.replacements:
                    raise CellError('owned output inode/type replacement: ' + str(path))
                self.replacements.add(path)
            current[path] = identity
            if stat.S_ISDIR(mode):
                try:
                    children = list(os.scandir(path))
                except OSError as exc:
                    raise CellError('owned output directory unreadable: ' + str(path)) from exc
                for child in sorted(children, key=lambda entry: entry.name):
                    visit(path / child.name, required=True)
                return
            if not stat.S_ISREG(mode):
                raise CellError('owned output special file rejected: ' + str(path))
            try:
                fd = os.open(str(path), os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
                try:
                    opened = os.fstat(fd)
                finally:
                    os.close(fd)
            except OSError as exc:
                raise CellError('owned output file unreadable: ' + str(path)) from exc
            opened_identity = (opened.st_dev, opened.st_ino, stat.S_IFMT(opened.st_mode))
            if opened_identity != identity or not stat.S_ISREG(opened.st_mode):
                raise CellError('owned output mutated during scan: ' + str(path))
            inode = (opened.st_dev, opened.st_ino)
            inode_sizes.setdefault(inode, opened.st_size)
            breakdown.append({'path': str(path), 'bytes': opened.st_size,
                              'device': opened.st_dev, 'inode': opened.st_ino})

        for root in self.roots:
            visit(root)
        disappeared = (set(self.identities) - set(current)) - self.replaceable
        if disappeared:
            raise CellError('owned output disappeared during scan: ' + str(sorted(disappeared)[0]))
        self.identities = current
        self.known_identities.update(current)
        total = sum(inode_sizes.values())
        digest = hashlib.sha256(json.dumps(breakdown, sort_keys=True).encode()).hexdigest()
        return {'total_bytes': total, 'breakdown': breakdown, 'digest': digest}


def validate_frontend_handoff(value, *, launcher_pid, run_dir, receive_monotonic_s,
                              launcher_start_monotonic_s):
    if not isinstance(value, dict) or value.get('schema') != 1 or value.get('mode') != DIRECT_MODE:
        raise CellError('invalid direct owner handoff schema/mode')
    if value.get('state') not in ('spawned', 'exited') or not isinstance(value.get('spawn'), dict):
        raise CellError('invalid direct owner handoff state')
    spawn = dict(value['spawn'])
    required = {
        'frontend': 'tui', 'terminal': True, 'terminal_size': [120, 40], 'n': 1,
        'workload': 'active', 'warmup_s': 30, 'observe_s': 120, 'teardown_grace_s': 60,
    }
    for key, expected in required.items():
        if spawn.get(key) != expected:
            raise CellError('direct owner handoff mismatch: ' + key)
    if type(spawn.get('frontend_pid')) is not int or spawn['frontend_pid'] <= 0:
        raise CellError('invalid direct owner frontend PID')
    if spawn.get('launcher_pid') != launcher_pid:
        raise CellError('direct owner handoff launcher mismatch')
    if _canonical(spawn.get('run_dir')) != pathlib.Path(run_dir).resolve():
        raise CellError('direct owner handoff run directory mismatch')
    nonce = spawn.get('nonce')
    identity = spawn.get('frontend_start_identity')
    before = spawn.get('spawn_before_monotonic_s')
    after = spawn.get('spawn_after_monotonic_s')
    if (not isinstance(nonce, str) or not nonce or not isinstance(identity, str) or not identity
            or not _is_pos_num(before) or not _is_pos_num(after)
            or not launcher_start_monotonic_s <= before <= after <= receive_monotonic_s):
        raise CellError('invalid direct owner spawn identity/bracket')
    if receive_monotonic_s > before + DIRECT_INTERVAL_S:
        raise CellError('direct owner handoff missed first RSS deadline')
    return spawn


def require_first_sample_deadline(spawn, *, sample_end_monotonic_s):
    if sample_end_monotonic_s > spawn['spawn_before_monotonic_s'] + DIRECT_INTERVAL_S:
        raise CellError('direct owner first RSS sample missed deadline')


def validate_exit_handoff(value, spawn, *, receipt_monotonic_s):
    if (not isinstance(value, dict) or value.get('schema') != 1
            or value.get('mode') != DIRECT_MODE or value.get('state') != 'exited'
            or value.get('spawn') != spawn
            or value.get('frontend_start_identity') != spawn.get('frontend_start_identity')
            or type(value.get('exit_code')) is not int
            or not _is_pos_num(value.get('wait_return_monotonic_s'))
            or value['wait_return_monotonic_s'] > receipt_monotonic_s):
        raise CellError('invalid or mismatched direct owner exit handoff')
    return value


def validate_direct_final_metadata(metadata, spawn, exited, run_dir):
    expected_exit = {
        'frontend_start_identity': spawn['frontend_start_identity'],
        'exit_code': exited['exit_code'],
        'wait_return_monotonic_s': exited['wait_return_monotonic_s'],
    }
    if (not isinstance(metadata, dict)
            or pathlib.Path(str(metadata.get('run_dir'))).resolve() != pathlib.Path(run_dir).resolve()
            or metadata.get('pid') != spawn['frontend_pid']
            or metadata.get('exit_code') != exited['exit_code']
            or metadata.get('direct_owner_spawn') != spawn
            or metadata.get('direct_owner_exit') != expected_exit):
        raise CellError('direct owner final metadata disagrees with lifecycle handoff')


def load_direct_handoff(path):
    path = pathlib.Path(path)
    try:
        info = path.lstat()
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
        raise CellError('direct owner handoff is not a regular owned file')
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise CellError('direct owner handoff is malformed') from exc
    if not isinstance(value, dict):
        raise CellError('direct owner handoff is not an object')
    return value


def read_linux_mem_available(path='/proc/meminfo'):
    try:
        for line in pathlib.Path(path).read_text().splitlines():
            if line.startswith('MemAvailable:'):
                parts = line.split()
                if len(parts) != 3 or parts[2] != 'kB':
                    break
                value = int(parts[1]) * 1024
                return value if value >= 0 else None
    except (OSError, ValueError):
        pass
    return None


def linux_preflight_snapshot():
    mem = {}
    for line in pathlib.Path('/proc/meminfo').read_text().splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == 'kB':
            mem[parts[0].rstrip(':')] = int(parts[1]) * 1024
    vmstat = {}
    for line in pathlib.Path('/proc/vmstat').read_text().splitlines():
        key, value = line.split()
        if key in ('pswpin', 'pswpout', 'oom_kill'):
            vmstat[key] = int(value)
    if any(key not in mem for key in ('MemAvailable', 'SwapTotal', 'SwapFree')):
        raise CellError('direct owner Linux memory counters unavailable')
    if any(key not in vmstat for key in ('pswpin', 'pswpout', 'oom_kill')):
        raise CellError('direct owner Linux swap/OOM counters unavailable')
    cgroup_path = None
    for line in pathlib.Path('/proc/self/cgroup').read_text().splitlines():
        fields = line.split(':', 2)
        if len(fields) == 3 and fields[0] == '0':
            cgroup_path = fields[2].lstrip('/')
            break
    if cgroup_path is None:
        raise CellError('direct owner cgroup v2 identity unavailable')
    try:
        boot_id = pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    except OSError as exc:
        raise CellError('direct owner Linux boot identity unavailable') from exc
    if not boot_id:
        raise CellError('direct owner Linux boot identity unavailable')
    events = {}
    events_path = pathlib.Path('/sys/fs/cgroup') / cgroup_path / 'memory.events'
    for line in events_path.read_text().splitlines():
        key, value = line.split()
        if key in ('oom', 'oom_kill'):
            events[key] = int(value)
    if set(events) != {'oom', 'oom_kill'}:
        raise CellError('direct owner cgroup OOM counters unavailable')
    return {
        'mem_available_bytes': mem['MemAvailable'],
        'swap_total_bytes': mem['SwapTotal'], 'swap_free_bytes': mem['SwapFree'],
        'vmstat': vmstat,
        'cgroup': {
            'identity': cgroup_path, 'boot_id': boot_id,
            'memory_events': events,
        },
    }


def append_json_line(path, value):
    with pathlib.Path(path).open('a', encoding='utf-8') as output:
        json.dump(value, output, sort_keys=True, allow_nan=False)
        output.write('\n')
        output.flush()


def direct_guard_sample(*, scheduled, started, ended, spawn_before, frontend_pid,
                        frontend_identity, launcher_pid, process_sample,
                        mem_available, output_scanner):
    output = output_scanner.scan()
    failure = None
    if ended > scheduled + DIRECT_INTERVAL_S:
        failure = 'guard_schedule_overrun'
    elif not isinstance(process_sample, dict):
        failure = 'frontend_sample_missing'
    elif process_sample.get('pid') != frontend_pid:
        failure = 'frontend_pid_mismatch'
    elif process_sample.get('start_identity') != frontend_identity:
        failure = 'frontend_identity_changed'
    elif process_sample.get('parent_pid') != launcher_pid:
        failure = 'frontend_parent_mismatch'
    elif type(process_sample.get('resident_bytes')) is not int or process_sample['resident_bytes'] < 0:
        failure = 'frontend_rss_invalid'
    elif type(mem_available) is not int:
        failure = 'MemAvailable_unavailable'
    elif mem_available < DIRECT_MEM_AVAILABLE_BYTES:
        failure = 'MemAvailable_below_floor'
    elif process_sample['resident_bytes'] > DIRECT_FRONTEND_RSS_BYTES:
        failure = 'frontend_rss_above_ceiling'
    elif output['total_bytes'] > DIRECT_OUTPUT_BYTES:
        failure = 'owned_output_above_ceiling'
    elif ended - spawn_before > DIRECT_FRONTEND_WALL_S:
        failure = 'frontend_wall_above_ceiling'
    return {
        'scheduled_monotonic_s': scheduled, 'start_monotonic_s': started,
        'end_monotonic_s': ended, 'lateness_s': started - scheduled,
        'acquisition_s': ended - started, 'frontend_pid': frontend_pid,
        'frontend_start_identity': frontend_identity,
        'frontend_rss_bytes': process_sample.get('resident_bytes') if isinstance(process_sample, dict) else None,
        'mem_available_bytes': mem_available, 'owned_output_bytes': output['total_bytes'],
        'owned_output_digest': output['digest'], 'elapsed_frontend_wall_s': ended - spawn_before,
        'failure_reason': failure,
    }


def expected_unpack_root(*, cwd: Optional[pathlib.Path] = None) -> pathlib.Path:
    """Client unpack directory with the same HOME/USERPROFILE selection as launch."""
    return unpack_root_path(cwd=cwd)


def _role_identity(pid: int, *, backend: str, role: str, timeout: float = SAMPLE_TIMEOUT_S) -> Dict[str, Any]:
    """Sample an exact PID identity; never invent or substitute another process."""
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        raise CellError(f'role {role!r} pid must be a positive int')
    try:
        sample = pa.process_sampler(backend)(pid, timeout=timeout)
    except (sr.SampleError, OSError, ValueError, TypeError) as exc:
        raise CellError(f'role {role!r} identity unavailable: {exc}') from exc
    identity = sample.get('start_identity') if isinstance(sample, dict) else None
    if not isinstance(identity, str) or not identity:
        raise CellError(f'role {role!r} identity unavailable')
    return {'pid': int(pid), 'start_identity': identity}


def parse_diagnostic_argv(diag_argv: Sequence[str]) -> argparse.Namespace:
    parser = rd.build_parser()
    try:
        args = parser.parse_args(list(diag_argv))
    except SystemExit as exc:
        raise CellError(f"diagnostic_argv failed to parse: {diag_argv}") from exc
    try:
        rd.validate_args(args, parser)
    except SystemExit as exc:
        raise CellError("diagnostic_argv failed validate_args") from exc
    return args


def require_argv_consistent_with_spec(spec: Mapping[str, Any], args: argparse.Namespace,
                                    *, _test_launcher=False) -> None:
    if not args.binary or not args.build_manifest or not args.build_role:
        raise CellError("diagnostic_argv must include --binary --build-manifest --build-role")
    bin_s = str(pathlib.Path(args.binary).resolve())
    man_s = str(pathlib.Path(args.build_manifest).resolve())
    if bin_s != str(pathlib.Path(spec["binary"]).resolve()):
        raise CellError("diagnostic --binary does not match spec.binary")
    if man_s != str(pathlib.Path(spec["build_manifest"]).resolve()):
        raise CellError("diagnostic --build-manifest does not match spec.build_manifest")
    if args.build_role != spec["build_role"]:
        raise CellError("diagnostic --build-role does not match spec.build_role")
    if args.frontend != spec["frontend"]:
        raise CellError("diagnostic frontend does not match spec.frontend")
    expected_backend = rd.requested_backend(args)
    if 'requested_backend' in spec and spec['requested_backend'] is not None:
        if spec['requested_backend'] != expected_backend:
            raise CellError('spec.requested_backend does not match diagnostic argv')
    # CPU fallback cells cannot be declared as GPU targets via a mismatched field.
    if expected_backend == 'cpu_fallback' and spec.get('requested_backend') == 'gpu':
        raise CellError('cpu_fallback diagnostic cannot pair as gpu requested_backend')
    contract = spec.get('capture_contract')
    direct = isinstance(contract, dict) and contract.get('mode') == DIRECT_MODE
    if bool(getattr(args, 'direct_owner_capture', False)) != direct:
        raise CellError('diagnostic direct-owner mode does not match spec')
    if direct:
        if args.n != 1 or args.workload != 'active' or args.headless:
            raise CellError('direct-owner diagnostic shape does not match spec')
        if not args.no_diagnostics or not args.sustain or args.warmup != 30 or args.observe != 120:
            raise CellError('direct-owner timing/lifecycle argv mismatch')
        if pathlib.Path(args.run_dir).resolve() != _canonical(contract['run_dir']):
            raise CellError('direct-owner --run-dir does not match reserved run directory')
        if pathlib.Path(args.frontend_handoff).resolve() != _canonical(contract['frontend_handoff_path']):
            raise CellError('direct-owner --frontend-handoff does not match reserved handoff')
    if not _test_launcher:
        argv = spec['launcher_argv']
        if (len(argv) < 3 or pathlib.Path(argv[0]).resolve() != pathlib.Path(sys.executable).resolve()
                or pathlib.Path(argv[1]).resolve() != pathlib.Path(rd.__file__).resolve()
                or argv[2:] != spec['diagnostic_argv']):
            raise CellError('launcher argv must invoke this run_diagnostic with the declared arguments')
        for key, actual in (('observe_s', args.observe), ('warmup_s', args.warmup)):
            if key in spec and spec[key] != actual:
                raise CellError('timing differs from diagnostic argv: ' + key)


def exclusive_cell_dir(cells_root: pathlib.Path, cell_id: str) -> pathlib.Path:
    if not isinstance(cell_id, str) or re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', cell_id) is None:
        raise CellError('unsafe cell id')
    path = pathlib.Path(cells_root) / cell_id
    path.mkdir(parents=True, exist_ok=False)
    (path / "logs").mkdir(exist_ok=False)
    return path


def write_json(path: pathlib.Path, value: Any) -> None:
    with path.open("x", encoding="utf-8") as out:
        json.dump(value, out, indent=2, allow_nan=False)
        out.write("\n")


def _exact_object(value: Any, keys: set[str], label: str) -> Dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        raise CellError(f'direct owner {label} schema fields mismatch')
    return value


def _finite_time(value: Any, label: str) -> float:
    if (not isinstance(value, (int, float)) or isinstance(value, bool)
            or not math.isfinite(value) or value < 0):
        raise CellError(f'direct owner {label} must be a finite nonnegative number')
    return float(value)


def _hex(value: Any, label: str, lengths=(64,)) -> str:
    if (not isinstance(value, str) or len(value) not in lengths
            or re.fullmatch(r'[0-9a-f]+', value) is None):
        raise CellError(f'direct owner {label} identity is malformed')
    return value


def _basename(value: Any, label: str) -> str:
    if (not isinstance(value, str) or not value or value in ('.', '..')
            or pathlib.PurePath(value).name != value or '/' in value or '\\' in value):
        raise CellError(f'direct owner {label} executable basename is malformed')
    return value


def _read_regular_json_binding(path_value: Any, label: str):
    if not isinstance(path_value, str) or not path_value:
        raise CellError(f'direct owner {label} path is missing')
    path = pathlib.Path(path_value)
    try:
        before = path.lstat()
        if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
            raise CellError(f'direct owner {label} is not a regular file')
        if before.st_size <= 0 or before.st_size > 1024 * 1024:
            raise CellError(f'direct owner {label} size is invalid')
        raw = path.read_bytes()
        after = path.lstat()
    except OSError as exc:
        raise CellError(f'direct owner {label} is unreadable') from exc
    identity_before = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    identity_after = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
    if identity_before != identity_after or stat.S_ISLNK(after.st_mode):
        raise CellError(f'direct owner {label} changed while reading')
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise CellError(f'direct owner {label} is malformed JSON') from exc
    if not isinstance(value, dict):
        raise CellError(f'direct owner {label} must be an object')
    return value, {
        'path': str(path.resolve(strict=True)),
        'sha256': hashlib.sha256(raw).hexdigest(),
    }


def probe_direct_server():
    started = time.time()
    try:
        with socket.create_connection(('127.0.0.1', DIRECT_SERVER_PORT), timeout=2.0):
            pass
    except OSError as exc:
        raise CellError('direct owner loopback server probe failed') from exc
    ended = time.time()
    return {
        'host': '127.0.0.1', 'port': DIRECT_SERVER_PORT, 'succeeded': True,
        'observed_start_unix_s': started, 'observed_end_unix_s': ended,
    }


def linux_executable_basename(pid: int) -> str:
    try:
        value = pathlib.Path(os.readlink(f'/proc/{pid}/exe')).name
    except OSError as exc:
        raise CellError('direct owner server executable identity unavailable') from exc
    return _basename(value, 'server')


def _identity_unavailable(cause: Optional[BaseException] = None) -> NoReturn:
    error = CellError('direct owner server executable identity unavailable')
    if cause is None:
        raise error
    raise error from cause


def parse_sudo_readlink_output(raw: bytes) -> str:
    if not isinstance(raw, bytes) or not raw or len(raw) > DIRECT_IDENTITY_STDOUT_LIMIT:
        _identity_unavailable()
    try:
        value = raw.decode('utf-8', errors='strict')
    except UnicodeDecodeError as exc:
        _identity_unavailable(exc)
    if (not value.startswith('/') or value == '/' or value.endswith(' (deleted)')
            or '\x00' in value or '\\' in value or '//' in value
            or any(char.isspace() for char in value)):
        _identity_unavailable()
    components = value.split('/')[1:]
    if not components or any(component in ('', '.', '..') for component in components):
        _identity_unavailable()
    return _basename(components[-1], 'server')


def _signal_identity_helper_group(proc: subprocess.Popen, sig: int) -> None:
    if os.name == 'posix':
        try:
            os.killpg(proc.pid, sig)
            return
        except ProcessLookupError:
            return
        except OSError:
            pass
    try:
        if sig == signal.SIGTERM:
            proc.terminate()
        else:
            proc.kill()
    except (OSError, ProcessLookupError):
        pass


def _reap_identity_helper(proc: subprocess.Popen, *, terminate_group: bool) -> None:
    if terminate_group:
        _signal_identity_helper_group(proc, signal.SIGTERM)
    if proc.poll() is None:
        try:
            proc.wait(timeout=0.2)
        except subprocess.TimeoutExpired:
            pass
    if terminate_group:
        _signal_identity_helper_group(proc, signal.SIGKILL)
    if proc.poll() is None:
        try:
            proc.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            pass
    else:
        try:
            proc.wait(timeout=0)
        except subprocess.TimeoutExpired:
            pass


def sudo_readlink_executable_basename(
    pid: int, *, popen_factory=None, monotonic=time.monotonic,
) -> tuple[str, Dict[str, Any]]:
    """Run only the fixed sudo/readlink adapter with streaming-bounded pipes."""
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        _identity_unavailable()
    argv = [
        '/usr/bin/sudo', '-n', '--', '/usr/bin/readlink', '-n', f'/proc/{pid}/exe',
    ]
    factory = subprocess.Popen if popen_factory is None else popen_factory
    started = monotonic()
    proc = None
    stdout = bytearray()
    stderr = bytearray()
    completed = False
    try:
        proc = factory(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            shell=False,
            close_fds=True,
            start_new_session=True,
        )
        if proc.stdout is None or proc.stderr is None:
            _identity_unavailable()
        deadline = started + DIRECT_IDENTITY_HELPER_TIMEOUT_S
        with selectors.DefaultSelector() as selector:
            streams = ((proc.stdout, stdout, DIRECT_IDENTITY_STDOUT_LIMIT),
                       (proc.stderr, stderr, DIRECT_IDENTITY_STDERR_LIMIT))
            for pipe, target, limit in streams:
                os.set_blocking(pipe.fileno(), False)
                selector.register(pipe, selectors.EVENT_READ, (target, limit))
            while selector.get_map() or proc.poll() is None:
                remaining = deadline - monotonic()
                if remaining <= 0:
                    _identity_unavailable()
                if not selector.get_map():
                    time.sleep(min(0.01, remaining))
                    continue
                for key, _events in selector.select(timeout=remaining):
                    target, limit = key.data
                    try:
                        chunk = os.read(key.fd, min(4096, limit - len(target) + 1))
                    except BlockingIOError:
                        continue
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    available = limit - len(target)
                    target.extend(chunk[:available])
                    if len(chunk) > available:
                        _identity_unavailable()
        returncode = proc.wait(timeout=max(0.0, deadline - monotonic()))
        if returncode != 0:
            _identity_unavailable()
        basename = parse_sudo_readlink_output(bytes(stdout))
        result = basename, {
            'selector': 'sudo-readlink-v1',
            'wall_s': monotonic() - started,
            'exit_class': 'zero',
            'stdout_bytes': len(stdout),
            'stderr_bytes': len(stderr),
        }
        completed = True
        return result
    except CellError:
        raise
    except (OSError, ValueError, TypeError, subprocess.SubprocessError) as exc:
        _identity_unavailable(exc)
    finally:
        if proc is not None:
            _reap_identity_helper(proc, terminate_group=not completed)
            for pipe in (proc.stdout, proc.stderr):
                if pipe is not None:
                    try:
                        pipe.close()
                    except OSError:
                        pass


def linux_boot_identity() -> str:
    value = ''
    try:
        value = pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    except OSError as exc:
        _identity_unavailable(exc)
    if not value or any(char.isspace() for char in value):
        _identity_unavailable()
    return value


def bracketed_sudo_readlink_executable_basename(
    pid: int, *, expected_start_identity: str, expected_boot_id: str,
    sample, boot_identity=linux_boot_identity, reader=sudo_readlink_executable_basename,
    sample_timeout: float = SAMPLE_TIMEOUT_S,
) -> tuple[str, Dict[str, Any]]:
    """Discard the helper result unless PID/start/boot match on both sides."""
    if (type(pid) is not int or isinstance(pid, bool) or pid <= 0
            or not isinstance(expected_start_identity, str) or not expected_start_identity
            or not isinstance(expected_boot_id, str) or not expected_boot_id):
        _identity_unavailable()

    def identity_sample() -> Dict[str, str | int]:
        value = None
        boot = ''
        try:
            value = sample(pid, timeout=sample_timeout)
            boot = boot_identity()
        except (sr.SampleError, OSError, ValueError, TypeError, KeyError) as exc:
            _identity_unavailable(exc)
        reported_pid = value.get('pid', pid) if isinstance(value, dict) else None
        if (not isinstance(value, dict) or type(reported_pid) is not int
                or reported_pid != pid
                or value.get('start_identity') != expected_start_identity
                or boot != expected_boot_id):
            _identity_unavailable()
        return {
            'pid': pid,
            'start_identity': expected_start_identity,
            'boot_id': expected_boot_id,
        }

    before = identity_sample()
    basename, helper = reader(pid)
    after = identity_sample()
    return _basename(basename, 'server'), {
        'identity_before': before,
        'identity_after': after,
        'helper': helper,
    }


def _direct_spec_binding(spec: Mapping[str, Any]) -> Dict[str, str]:
    contract = spec['capture_contract']
    binding = {
        'result_path': str(_canonical(contract['owned_output_paths'][0])),
        'cell_dir': str(_canonical(contract['cell_dir'])),
        'run_dir': str(_canonical(contract['run_dir'])),
        'frontend_handoff_path': str(_canonical(contract['frontend_handoff_path'])),
        'build_manifest_path': str(pathlib.Path(spec['build_manifest']).resolve(strict=True)),
        'binary_path': str(pathlib.Path(spec['binary']).resolve(strict=True)),
        'nav_pack_path': str(pathlib.Path(spec['nav_pack']).resolve(strict=True)),
        'nav_flags_path': str(pathlib.Path(spec['nav_flags']).resolve(strict=True)),
        'catalog_path': str(pathlib.Path(spec['catalog_path']).resolve(strict=True)),
        'cache_dir': str(pathlib.Path(spec['cache_dir']).resolve(strict=True)),
        'unpack_root': str(pathlib.Path(spec['unpack_root']).resolve(strict=True)),
    }
    if 'server_executable_identity' in contract:
        binding['server_executable_identity'] = contract['server_executable_identity']
    return binding


def _direct_context(spec: Mapping[str, Any], provenance: Mapping[str, Any],
                    cache_snapshot: Mapping[str, Any], server_sample: Mapping[str, Any],
                    server_executable: str, fixture_binding: Any) -> Dict[str, Any]:
    context = {
        'fixture_binding_sha256': _hex(fixture_binding, 'fixture binding'),
        'host_commit': provenance.get('build_commit'),
        'client_commit': provenance.get('client_commit'),
        'host_sources_sha256': provenance.get('host_sources_sha256'),
        'client_sources_sha256': provenance.get('client_sources_sha256'),
        'build_manifest_sha256': bp.file_sha256(spec['build_manifest']),
        'binary_sha256': bp.file_sha256(spec['binary']),
        'nav_pack_sha256': bp.file_sha256(spec['nav_pack']),
        'nav_flags_sha256': bp.file_sha256(spec['nav_flags']),
        'catalog_sha256': bp.file_sha256(spec['catalog_path']),
        'cache_content_identity_sha256': cache_snapshot.get('content_identity_sha256'),
        'cache_snapshot_version': cache_snapshot.get('snapshot_version'),
        'server_pid': spec['game_server_pid'],
        'server_start_identity': server_sample.get('start_identity'),
        'server_executable_basename': _basename(server_executable, 'server'),
    }
    selector = spec['capture_contract'].get('server_executable_identity')
    if selector is not None:
        context['server_executable_identity'] = selector
    _hex(context['host_commit'], 'host commit', (40, 64))
    _hex(context['client_commit'], 'client commit', (40, 64))
    for key in (
        'host_sources_sha256', 'client_sources_sha256', 'build_manifest_sha256',
        'binary_sha256', 'nav_pack_sha256', 'nav_flags_sha256', 'catalog_sha256',
        'cache_content_identity_sha256',
    ):
        _hex(context[key], key)
    _hex(context['cache_snapshot_version'], 'cache snapshot version', (16,))
    if (type(context['server_pid']) is not int or context['server_pid'] <= 0
            or not isinstance(context['server_start_identity'], str)
            or not context['server_start_identity']):
        raise CellError('direct owner server identity is malformed')
    return context


def validate_direct_admissions(
    spec: Mapping[str, Any], provenance: Mapping[str, Any],
    before: Mapping[str, Any], server_sample: Mapping[str, Any],
    cache_snapshot: Mapping[str, Any], *, now: float,
    server_probe: Mapping[str, Any], server_executable: str,
) -> Dict[str, Any]:
    """Validate exact root release/receipt schemas without exposing payloads."""
    contract = spec['capture_contract']
    release, release_binding = _read_regular_json_binding(
        contract['release_contract'], 'root release contract'
    )
    release_keys = {
        'schema', 'mode', 'n', 'workload', 'frontend', 'issued_unix_s',
        'observation_window', 'expires_unix_s', 'boot_id', 'context',
        'spec_binding', 'receipt_bindings',
    }
    _exact_object(release, release_keys, 'root release contract')
    if (release['schema'] != 'direct-owner-root-release-v1'
            or release['mode'] != DIRECT_MODE or type(release['n']) is not int
            or release['n'] != 1 or release['workload'] != 'active'
            or release['frontend'] != 'tui'):
        raise CellError('direct owner root release contract identity mismatch')
    window = _exact_object(
        release['observation_window'],
        {'not_before_unix_s', 'not_after_unix_s'}, 'release observation window',
    )
    not_before = _finite_time(window['not_before_unix_s'], 'observation not-before')
    not_after = _finite_time(window['not_after_unix_s'], 'observation not-after')
    issued = _finite_time(release['issued_unix_s'], 'release issue time')
    expires = _finite_time(release['expires_unix_s'], 'release expiry')
    current = _finite_time(now, 'current time')
    if not not_before < not_after <= issued <= current < expires:
        raise CellError('direct owner root release contract is stale or temporally invalid')
    boot_id = before.get('cgroup', {}).get('boot_id')
    if not isinstance(boot_id, str) or not boot_id or release['boot_id'] != boot_id:
        raise CellError('direct owner root release boot identity mismatch')
    release_context_keys = {
            'fixture_binding_sha256', 'host_commit', 'client_commit',
            'host_sources_sha256', 'client_sources_sha256',
            'build_manifest_sha256', 'binary_sha256', 'nav_pack_sha256',
            'nav_flags_sha256', 'catalog_sha256', 'cache_content_identity_sha256',
            'cache_snapshot_version', 'server_pid', 'server_start_identity',
            'server_executable_basename',
    }
    if 'server_executable_identity' in contract:
        release_context_keys.add('server_executable_identity')
    release_context = _exact_object(
        release['context'], release_context_keys, 'release context',
    )
    expected_context = _direct_context(
        spec, provenance, cache_snapshot, server_sample, server_executable,
        release_context.get('fixture_binding_sha256'),
    )
    if release_context != expected_context:
        raise CellError('direct owner root release context identity mismatch')
    spec_binding_keys = {
            'result_path', 'cell_dir', 'run_dir', 'frontend_handoff_path',
            'build_manifest_path', 'binary_path', 'nav_pack_path', 'nav_flags_path',
            'catalog_path', 'cache_dir', 'unpack_root',
    }
    if 'server_executable_identity' in contract:
        spec_binding_keys.add('server_executable_identity')
    spec_binding = _exact_object(
        release['spec_binding'], spec_binding_keys, 'release spec binding',
    )
    if spec_binding != _direct_spec_binding(spec):
        raise CellError('direct owner root release spec/file binding mismatch')

    actual_probe = _exact_object(
        server_probe, {
            'host', 'port', 'succeeded', 'observed_start_unix_s',
            'observed_end_unix_s',
        }, 'current server probe',
    )
    probe_start = _finite_time(actual_probe['observed_start_unix_s'], 'probe start')
    probe_end = _finite_time(actual_probe['observed_end_unix_s'], 'probe end')
    if (actual_probe['host'] != '127.0.0.1' or type(actual_probe['port']) is not int
            or actual_probe['port'] != DIRECT_SERVER_PORT
            or actual_probe['succeeded'] is not True
            or not issued <= probe_start <= probe_end <= current):
        raise CellError('direct owner current loopback server probe failed')

    release_receipts = _exact_object(
        release['receipt_bindings'], set(DIRECT_ADMISSION_KINDS),
        'release receipt bindings',
    )
    bindings = {'release_contract': release_binding}
    summaries = {}
    for kind in DIRECT_ADMISSION_KINDS:
        declared = _exact_object(
            release_receipts[kind], {'path', 'sha256'}, f'{kind} receipt binding'
        )
        expected_path = pathlib.Path(contract['admission_receipts'][kind]).resolve(strict=True)
        if (not isinstance(declared['path'], str)
                or pathlib.Path(declared['path']).resolve(strict=True) != expected_path):
            raise CellError(f'direct owner {kind} receipt path binding mismatch')
        expected_digest = _hex(declared['sha256'], f'{kind} receipt digest')
        receipt, binding = _read_regular_json_binding(str(expected_path), f'{kind} receipt')
        if binding['sha256'] != expected_digest:
            raise CellError(f'direct owner {kind} receipt hash binding mismatch')
        common_keys = {
            'schema', 'kind', 'mode', 'n', 'workload', 'frontend', 'admitted',
            'observed_start_unix_s', 'observed_end_unix_s', 'boot_id',
            'context', 'evidence',
        }
        _exact_object(receipt, common_keys, f'{kind} receipt')
        expected_schema = f'direct-owner-{kind.replace("_", "-")}-admission-v1'
        if (receipt['schema'] != expected_schema or receipt['kind'] != kind
                or receipt['mode'] != DIRECT_MODE or type(receipt['n']) is not int
                or receipt['n'] != 1 or receipt['workload'] != 'active'
                or receipt['frontend'] != 'tui' or receipt['admitted'] is not True
                or receipt['boot_id'] != boot_id or receipt['context'] != expected_context):
            raise CellError(f'direct owner {kind} receipt admission/context mismatch')
        observed_start = _finite_time(
            receipt['observed_start_unix_s'], f'{kind} observation start'
        )
        observed_end = _finite_time(
            receipt['observed_end_unix_s'], f'{kind} observation end'
        )
        if not not_before <= observed_start < observed_end <= not_after:
            raise CellError(f'direct owner {kind} receipt observation is stale')
        evidence = receipt['evidence']
        if kind == 'conflict':
            evidence = _exact_object(
                evidence, {'checked_classes', 'matches'}, 'conflict evidence'
            )
            classes = evidence['checked_classes']
            if (not isinstance(classes, list) or len(classes) != len(DIRECT_CONFLICT_CLASSES)
                    or set(classes) != set(DIRECT_CONFLICT_CLASSES)
                    or any(not isinstance(value, str) for value in classes)):
                raise CellError('direct owner conflict classes omitted or unknown')
            matches = evidence['matches']
            if not isinstance(matches, list):
                raise CellError('direct owner conflict matches must be a list')
            for match in matches:
                match = _exact_object(
                    match, {'class', 'pid', 'start_identity', 'executable_basename'},
                    'conflict match',
                )
                if (match['class'] not in DIRECT_CONFLICT_CLASSES
                        or type(match['pid']) is not int or match['pid'] <= 0
                        or not isinstance(match['start_identity'], str)
                        or not match['start_identity']):
                    raise CellError('direct owner conflict match identity is malformed')
                _basename(match['executable_basename'], 'conflict match')
            if matches:
                raise CellError('direct owner conflicting process match admitted')
            summary = {'checked_classes': list(classes), 'match_count': 0}
        elif kind == 'account':
            evidence = _exact_object(
                evidence, {'fixture_binding_sha256', 'slot_count', 'active_slot_count'},
                'account evidence',
            )
            if (evidence['fixture_binding_sha256'] != expected_context['fixture_binding_sha256']
                    or type(evidence['slot_count']) is not int or evidence['slot_count'] != 1
                    or type(evidence['active_slot_count']) is not int
                    or evidence['active_slot_count'] != 1):
                raise CellError('direct owner account receipt does not admit one active fixture')
            summary = {'slot_count': 1, 'active_slot_count': 1}
        elif kind == 'population':
            evidence = _exact_object(
                evidence, {'fixture_binding_sha256', 'requested_n', 'admitted_n', 'workload'},
                'population evidence',
            )
            if (evidence['fixture_binding_sha256'] != expected_context['fixture_binding_sha256']
                    or type(evidence['requested_n']) is not int or evidence['requested_n'] != 1
                    or type(evidence['admitted_n']) is not int or evidence['admitted_n'] != 1
                    or evidence['workload'] != 'active'):
                raise CellError('direct owner population receipt mismatch')
            summary = {'requested_n': 1, 'admitted_n': 1, 'workload': 'active'}
        elif kind == 'cache':
            evidence = _exact_object(
                evidence, {'fixture_binding_sha256', 'content_identity_sha256', 'snapshot_version'},
                'cache evidence',
            )
            if (evidence['fixture_binding_sha256'] != expected_context['fixture_binding_sha256']
                    or evidence['content_identity_sha256'] != expected_context['cache_content_identity_sha256']
                    or evidence['snapshot_version'] != expected_context['cache_snapshot_version']):
                raise CellError('direct owner cache receipt identity mismatch')
            summary = {
                'content_identity_sha256': evidence['content_identity_sha256'],
                'snapshot_version': evidence['snapshot_version'],
            }
        else:
            evidence = _exact_object(
                evidence, {'pid', 'start_identity', 'executable_basename', 'probe'},
                'server health evidence',
            )
            declared_probe = _exact_object(
                evidence['probe'], {'host', 'port', 'succeeded'},
                'server health probe evidence',
            )
            if (type(evidence['pid']) is not int
                    or evidence['pid'] != expected_context['server_pid']
                    or evidence['start_identity'] != expected_context['server_start_identity']
                    or _basename(evidence['executable_basename'], 'server health')
                    != expected_context['server_executable_basename']
                    or declared_probe != {
                        'host': '127.0.0.1', 'port': DIRECT_SERVER_PORT,
                        'succeeded': True,
                    }):
                raise CellError('direct owner server health receipt mismatch')
            summary = {
                'pid': evidence['pid'], 'start_identity': evidence['start_identity'],
                'executable_basename': evidence['executable_basename'],
                'probe': dict(declared_probe),
            }
        bindings['receipt_' + kind] = binding
        summaries[kind] = {
            'schema': receipt['schema'],
            'observed_start_unix_s': observed_start,
            'observed_end_unix_s': observed_end,
            'evidence': summary,
        }
    return {
        'bindings': bindings, 'receipt_summaries': summaries,
        'release_expires_unix_s': expires, 'release_issued_unix_s': issued,
        'release_observation_window': dict(window),
        'context': expected_context,
    }


def preflight(
    spec: Mapping[str, Any],
    args: argparse.Namespace,
    *,
    sample_timeout: float = SAMPLE_TIMEOUT_S,
    direct_snapshot=linux_preflight_snapshot,
    disk_usage=shutil.disk_usage,
    sleep=time.sleep,
    platform_name=None,
    machine=None,
    wall_time=time.time,
    server_probe=probe_direct_server,
    executable_basename=linux_executable_basename,
    boot_identity=linux_boot_identity,
    sudo_executable_reader=sudo_readlink_executable_basename,
) -> Dict[str, Any]:
    """Verify build fixtures and live server identity. Never signals the server."""
    def validate_direct_snapshot(value, label):
        if not isinstance(value, dict):
            raise CellError(f'direct owner {label} preflight snapshot is not an object')
        for key in ('mem_available_bytes', 'swap_total_bytes', 'swap_free_bytes'):
            if type(value.get(key)) is not int or value[key] < 0:
                raise CellError(f'direct owner {label} memory counters unavailable')
        if value['swap_free_bytes'] > value['swap_total_bytes']:
            raise CellError(f'direct owner {label} swap counters invalid')
        vmstat = value.get('vmstat')
        if (not isinstance(vmstat, dict)
                or any(type(vmstat.get(key)) is not int or vmstat[key] < 0
                       for key in ('pswpin', 'pswpout', 'oom_kill'))):
            raise CellError(f'direct owner {label} swap/OOM counters unavailable')
        cgroup = value.get('cgroup')
        events = cgroup.get('memory_events') if isinstance(cgroup, dict) else None
        if (not isinstance(cgroup, dict) or not isinstance(cgroup.get('identity'), str)
                or not cgroup['identity'] or not isinstance(cgroup.get('boot_id'), str)
                or not cgroup['boot_id'] or not isinstance(events, dict)
                or set(events) != {'oom', 'oom_kill'}
                or any(type(counter) is not int or counter < 0 for counter in events.values())):
            raise CellError(f'direct owner {label} cgroup OOM counters unavailable')

    binary = pathlib.Path(spec["binary"]).resolve(strict=True)
    direct = isinstance(spec.get('capture_contract'), dict)
    verifier = bp.verify_direct_owner_build if direct else bp.verify_build
    provenance = verifier(
        pathlib.Path(spec["build_manifest"]),
        spec["build_role"],
        spec["frontend"],
        binary,
        pathlib.Path(spec["nav_pack"]),
        pathlib.Path(spec["nav_flags"]),
        pathlib.Path(spec["catalog_path"]),
    )
    if direct:
        manifest_value = json.loads(pathlib.Path(spec['build_manifest']).read_text())
        if manifest_value.get('candidate', {}).get('source_lineage') != spec['capture_contract'].get('source_lineage'):
            raise CellError('direct owner spec source lineage differs from build manifest')
    for key in ("server_identity_path", "host_conditions_path"):
        p = pathlib.Path(spec[key]).resolve(strict=True)
        json.loads(p.read_text())
    sample = pa.process_sampler(spec['process_backend'])
    server_sample = sample(int(spec["game_server_pid"]), timeout=sample_timeout)
    identity = json.loads(pathlib.Path(spec['server_identity_path']).read_text())
    if (not isinstance(identity, dict) or type(identity.get('pid')) is not int
            or identity['pid'] != spec['game_server_pid']
            or identity.get('start_identity') != server_sample.get('start_identity')):
        raise CellError('server sidecar differs from sampled PID/start identity')
    ambient_identities: Dict[str, Any] = {}
    for name, pid in sorted(spec["ambient_helpers"].items()):
        ambient_identities[name] = {
            "pid": pid,
            "sample": sample(int(pid), timeout=sample_timeout),
        }
    direct_preflight = None
    direct_cache_snapshot = None
    if direct:
        if server_sample.get('state') == 'Z' or not isinstance(server_sample.get('state'), str):
            raise CellError('direct owner server is zombie or process state unavailable')
        current_platform = platform_name or sys.platform
        current_machine = machine or platform.machine()
        if not current_platform.startswith('linux') or current_machine != 'x86_64':
            raise CellError('direct owner requires Linux x86_64')
        before = direct_snapshot()
        validate_direct_snapshot(before, 'before')
        if before['mem_available_bytes'] < DIRECT_PREFLIGHT_MEM_AVAILABLE_BYTES:
            raise CellError('direct owner preflight MemAvailable below 768 MiB')
        if before['swap_total_bytes'] - before['swap_free_bytes'] != 0:
            raise CellError('direct owner preflight has active swap use')
        contract = spec['capture_contract']
        executable_selector = contract.get('server_executable_identity', 'proc-exe')
        executable_ancillary = []

        def read_server_executable():
            if executable_selector != 'sudo-readlink-v1':
                return executable_basename(int(spec['game_server_pid']))
            basename, evidence = bracketed_sudo_readlink_executable_basename(
                spec['game_server_pid'],
                expected_start_identity=server_sample['start_identity'],
                expected_boot_id=before['cgroup']['boot_id'],
                sample=sample,
                boot_identity=boot_identity,
                reader=sudo_executable_reader,
                sample_timeout=sample_timeout,
            )
            executable_ancillary.append(evidence)
            return basename

        free = disk_usage(_canonical(contract['cell_dir'])).free
        if free < DIRECT_PREFLIGHT_DISK_FREE_BYTES:
            raise CellError('direct owner preflight output filesystem below 256 MiB free')
        try:
            direct_cache_snapshot = cp.capture(spec['cache_dir'], spec['unpack_root'])
        except (OSError, ValueError, TypeError, KeyError) as exc:
            raise CellError('direct owner current cache identity is unavailable') from exc
        current_probe = server_probe()
        server_executable = read_server_executable()
        admission = validate_direct_admissions(
            spec, provenance, before, server_sample, direct_cache_snapshot,
            now=wall_time(), server_probe=current_probe,
            server_executable=server_executable,
        )
        sleep(0.05)
        after = direct_snapshot()
        validate_direct_snapshot(after, 'after')
        if after['mem_available_bytes'] < DIRECT_PREFLIGHT_MEM_AVAILABLE_BYTES:
            raise CellError('direct owner preflight MemAvailable below 768 MiB')
        if after['swap_total_bytes'] - after['swap_free_bytes'] != 0:
            raise CellError('direct owner preflight has active swap use')
        server_after = sample(int(spec['game_server_pid']), timeout=sample_timeout)
        if server_after.get('start_identity') != server_sample.get('start_identity'):
            raise CellError('server identity changed during direct owner preflight')
        if server_after.get('state') == 'Z' or not isinstance(server_after.get('state'), str):
            raise CellError('direct owner server is zombie or process state unavailable')
        if (_basename(read_server_executable(), 'server')
                != server_executable):
            raise CellError('server executable identity changed during direct owner preflight')
        try:
            bp.recheck_files(admission['bindings'])
        except (OSError, ValueError, TypeError, KeyError) as exc:
            raise CellError('direct owner admission receipt changed during preflight') from exc
        if (after.get('vmstat') != before.get('vmstat')
                or after.get('cgroup') != before.get('cgroup')):
            raise CellError('swap or OOM counters changed during direct owner preflight')
        direct_preflight = {
            'before': before, 'after': after, 'output_free_bytes': free,
            'admission_bindings': admission['bindings'],
            'receipt_summaries': admission['receipt_summaries'],
            'release_issued_unix_s': admission['release_issued_unix_s'],
            'release_expires_unix_s': admission['release_expires_unix_s'],
            'release_observation_window': admission['release_observation_window'],
            'server_probe': current_probe, 'server_after': server_after,
        }
        if executable_selector == 'sudo-readlink-v1':
            direct_preflight['server_executable_identity'] = {
                'selector': executable_selector,
                'reads': executable_ancillary,
            }
    return {
        "provenance": provenance,
        "server_sample": server_sample,
        "server_pid": int(spec["game_server_pid"]),
        "ambient_identities": ambient_identities,
        "binary": str(binary),
        "observe_s": float(spec["observe_s"]) if "observe_s" in spec else float(args.observe),
        "warmup_s": float(spec["warmup_s"]) if "warmup_s" in spec else float(args.warmup),
        "direct_preflight": direct_preflight,
        "direct_cache_snapshot": direct_cache_snapshot,
    }


class IncrementalLines:
    """Byte-offset file reader; retains a partial trailing line until newline."""

    def __init__(self, path: pathlib.Path):
        self.path = pathlib.Path(path)
        self.pos = 0
        self.partial = b""

    def poll(self) -> List[str]:
        if not self.path.is_file():
            return []
        with self.path.open("rb") as fh:
            fh.seek(self.pos)
            chunk = fh.read()
            self.pos = fh.tell()
        if not chunk:
            return []
        self.partial += chunk
        lines: List[str] = []
        while b"\n" in self.partial:
            raw, self.partial = self.partial.split(b"\n", 1)
            lines.append(raw.decode("utf-8", errors="replace"))
        return lines


def _pid_alive(pid: int) -> bool:
    """True if the explicit local PID still refers to a running process.

    POSIX: classic ``os.kill(pid, 0)`` existence probe (no signal delivered).
    Windows: never ``os.kill(pid, 0)`` — that value is CTRL_C_EVENT and can hit a
    whole console group; use Win32 OpenProcess + WaitForSingleObject instead
    (same exit detection as windows_process_sample).
    """
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        return False
    if sys.platform == 'win32':
        try:
            import windows_process_sample as wps  # type: ignore
            return bool(wps.process_is_alive(int(pid)))
        except Exception:
            # Fail closed toward "alive" so cleanup does not skip owned children
            # when the helper cannot answer.
            return True
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def parent_pid(pid: int, *, timeout: float = SAMPLE_TIMEOUT_S) -> int:
    """Return parent PID for an explicit local PID. Fail closed; no name scan."""
    if type(pid) is not int or isinstance(pid, bool) or pid <= 0:
        raise CellError('invalid PID for parent lookup')
    if sys.platform == 'win32':
        try:
            import windows_process_parent as wpp  # type: ignore
        except ImportError as exc:
            raise CellError(f'windows parent helper unavailable: {exc}') from exc
        try:
            return int(wpp.parent_pid(pid))
        except Exception as exc:
            raise CellError(f'cannot establish frontend parent ownership: {exc}') from exc
    if sys.platform.startswith('linux'):
        try:
            text = (pathlib.Path('/proc') / str(pid) / 'stat').read_text()
            close = text.rfind(')')
            if close < 0:
                raise ValueError('missing comm terminator')
            return int(text[close + 2:].split()[1])
        except (OSError, ValueError, IndexError) as exc:
            raise CellError(f'cannot establish frontend parent ownership: {exc}') from exc
    try:
        out = subprocess.check_output(
            ['ps', '-o', 'ppid=', '-p', str(pid)],
            text=True,
            timeout=timeout,
        ).strip()
        return int(out)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        raise CellError(f'cannot establish frontend parent ownership: {exc}') from exc


def _soft_signal_label() -> str:
    return 'SIGTERM'


def _hard_signal_label() -> str:
    # signal.SIGKILL is undefined on win32; never evaluate it there.
    if sys.platform == 'win32' or not hasattr(signal, 'SIGKILL'):
        return 'TERMINATE'
    return 'SIGKILL'


def _signal_owned_pid(pid: int, *, stage: str) -> str:
    """Signal an owned non-child PID. Soft = TERM; hard = KILL/TerminateProcess.

    On Windows both stages use os.kill(SIGTERM) → TerminateProcess; labels differ
    so cleanup reports stay honest about escalation without referencing SIGKILL.
    """
    if stage == 'soft':
        os.kill(pid, signal.SIGTERM)
        return _soft_signal_label()
    if sys.platform == 'win32' or not hasattr(signal, 'SIGKILL'):
        # Escalate with TerminateProcess-class kill (same primitive as SIGTERM map).
        os.kill(pid, signal.SIGTERM)
        return _hard_signal_label()
    os.kill(pid, signal.SIGKILL)  # type: ignore[attr-defined]
    return _hard_signal_label()


def request_collector_stop_file(stop_path: pathlib.Path, *, reason: str = 'orchestrator_stop') -> Dict[str, Any]:
    """Exclusive-create the cell-local collector stop file (portable graceful IPC)."""
    info: Dict[str, Any] = {
        'path': str(stop_path),
        'created': False,
        'ok': False,
        'reason': reason,
    }
    try:
        path = pathlib.Path(stop_path)
        if not path.is_absolute():
            info['error'] = 'stop path must be absolute and cell-contained'
            return info
        parent = path.parent
        if not parent.is_dir():
            info['error'] = f'stop path parent missing: {parent}'
            return info
        payload = (json.dumps({'reason': reason, 'utc': _utc()}, sort_keys=True) + '\n').encode('utf-8')
        flags = os.O_CREAT | os.O_EXCL | os.O_WRONLY
        if hasattr(os, 'O_BINARY'):
            flags |= os.O_BINARY  # type: ignore[attr-defined]
        try:
            fd = os.open(str(path), flags, 0o644)
        except FileExistsError:
            info['ok'] = True
            info['already_present'] = True
            return info
        try:
            os.write(fd, payload)
        finally:
            os.close(fd)
        info['created'] = True
        info['ok'] = True
        return info
    except OSError as exc:
        info['error'] = str(exc)
        return info


def _request_collector_stop(
    collector: Optional[subprocess.Popen],
    stop_path: Optional[pathlib.Path],
    *,
    also_unix_sigterm: bool = True,
) -> Dict[str, Any]:
    """Graceful collector stop: stop-file first; Unix may also SIGTERM (handlers).

    Never uses send_signal(SIGTERM) as the graceful path on Windows — that maps
    to TerminateProcess and skips controlled_stop summary/exit 0.
    """
    result: Dict[str, Any] = {'stop_file': None, 'sigterm': False}
    if stop_path is not None:
        result['stop_file'] = request_collector_stop_file(stop_path)
    if (
        also_unix_sigterm
        and sys.platform != 'win32'
        and collector is not None
        and collector.poll() is None
    ):
        try:
            collector.send_signal(signal.SIGTERM)
            result['sigterm'] = True
        except OSError as exc:
            result['sigterm_error'] = str(exc)
    return result


def _start_identity(pid: int, timeout: float = SAMPLE_TIMEOUT_S, backend='system') -> Optional[str]:
    try:
        return pa.process_sampler(backend)(pid, timeout=timeout).get("start_identity")
    except (sr.SampleError, OSError, ValueError):
        return None


def _terminate_owned(
    proc: Optional[subprocess.Popen],
    *,
    label: str,
    wait_s: float = CLEANUP_WAIT_S,
    stop_path: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Terminate/reap an owned direct child. Frontend cleanup is independent.

    Collector path: optional stop-file graceful request first, then wait, then
    Popen.kill() escalation (TerminateProcess on Windows). Never signals server.
    """
    info: Dict[str, Any] = {"label": label, "signaled": False, "exit_code": None}
    if proc is None:
        return info
    info["pid"] = proc.pid
    if proc.poll() is not None:
        info["exit_code"] = proc.returncode
        return info
    if stop_path is not None and label == 'collector':
        info['graceful_stop'] = _request_collector_stop(proc, stop_path, also_unix_sigterm=True)
        try:
            proc.wait(timeout=wait_s)
            info["exit_code"] = proc.returncode
            info["signaled"] = bool((info.get('graceful_stop') or {}).get('sigterm'))
            return info
        except subprocess.TimeoutExpired:
            pass
    else:
        info["signaled"] = True
        try:
            # Direct children: soft TERM then kill. On Windows send_signal(SIGTERM)
            # is TerminateProcess — still valid for owned launcher/collector escalate.
            if sys.platform != 'win32':
                proc.send_signal(signal.SIGTERM)
            else:
                # Prefer kill for owned Windows children after any graceful channel.
                proc.kill()
                info['killed'] = True
                try:
                    proc.wait(timeout=wait_s)
                    info['exit_code'] = proc.returncode
                except subprocess.TimeoutExpired:
                    info['exit_code'] = proc.poll()
                    info['orphan_risk'] = True
                return info
        except OSError as exc:
            info["error"] = str(exc)
        try:
            proc.wait(timeout=wait_s)
            info["exit_code"] = proc.returncode
            return info
        except subprocess.TimeoutExpired:
            pass
    try:
        proc.kill()
    except OSError:
        pass
    try:
        proc.wait(timeout=wait_s)
        info["exit_code"] = proc.returncode
        info["killed"] = True
    except subprocess.TimeoutExpired:
        info["exit_code"] = proc.poll()
        info["orphan_risk"] = True
    return info


def _cleanup_frontend(
    pid,
    identity,
    *,
    backend,
    launcher_pid: Optional[int] = None,
    wait_s=CLEANUP_WAIT_S,
):
    """Independently clean a captured owned frontend, even after launcher exit.

    Before each escalation stage: require stable start_identity. While the
    launcher is still alive, also require parent_pid == launcher_pid. After
    launcher death, identity alone gates signals (parent may reparent to init).
    Never references signal.SIGKILL on win32.
    """
    result = {'pid': pid, 'signals': [], 'orphan_risk': False}
    if pid is None or not _pid_alive(pid):
        return result
    for stage in ('soft', 'hard'):
        if not _pid_alive(pid):
            return result
        # Recheck ownership immediately before EACH signal, including escalation.
        if launcher_pid is not None and _pid_alive(launcher_pid):
            try:
                parent = parent_pid(pid)
            except CellError:
                if not _pid_alive(pid):
                    return result
                result.update(
                    orphan_risk=True,
                    reason='frontend parent unavailable; no signal',
                )
                return result
            if parent != launcher_pid:
                result.update(
                    orphan_risk=True,
                    reason='frontend parent is not owned launcher; no signal',
                )
                return result
        if not identity or _start_identity(pid, backend=backend) != identity:
            result.update(orphan_risk=True, reason='frontend identity unavailable or changed; no signal')
            return result
        try:
            label = _signal_owned_pid(pid, stage=stage)
            result['signals'].append(label)
        except ProcessLookupError:
            return result
        except OSError as exc:
            result['error'] = str(exc)
            result['orphan_risk'] = _pid_alive(pid)
            return result
        deadline = time.monotonic() + wait_s
        while time.monotonic() < deadline:
            if not _pid_alive(pid):
                return result
            time.sleep(.05)
    result['orphan_risk'] = _pid_alive(pid)
    return result


def _capture_owned_frontend(pid, launcher_pid, forbidden, backend):
    if type(pid) is not int or pid <= 0 or pid in forbidden:
        raise CellError('invalid or foreign frontend PID')
    if not _pid_alive(pid):
        return pid, None
    try:
        parent = parent_pid(pid)
    except CellError:
        if not _pid_alive(pid):
            return pid, None
        raise
    if parent != launcher_pid:
        raise CellError('frontend is not a child of the owned launcher')
    return pid, _start_identity(pid, backend=backend)


def _capture_conpty_helpers(meta, launcher_pid, forbidden, backend, *, required=True):
    """Validate the identity-bound ConPTY handoff before collector start."""
    if sys.platform != 'win32':
        return {}
    helpers = meta.get('conpty_helpers') if isinstance(meta, dict) else None
    if not isinstance(helpers, list) or not helpers:
        if not required:
            return {}
        raise CellError('required ConPTY helper handoff missing')
    try:
        import windows_process_parent as wpp  # type: ignore
    except ImportError as exc:
        raise CellError(f'Windows ConPTY helper ownership unavailable: {exc}') from exc
    out = {}
    for index, helper in enumerate(helpers):
        if not isinstance(helper, dict) or type(helper.get('pid')) is not int:
            raise CellError('invalid ConPTY helper handoff')
        pid = int(helper['pid'])
        if pid in forbidden or pid <= 0 or helper.get('parent_pid') != launcher_pid:
            raise CellError('ConPTY helper ownership mismatch')
        if str(helper.get('image_name', '')).casefold() != 'conhost.exe':
            raise CellError('ConPTY helper image mismatch')
        if wpp.parent_pid(pid) != launcher_pid:
            raise CellError('ConPTY helper is not a child of the owned launcher')
        identity = helper.get('start_identity')
        actual = _start_identity(pid, backend=backend)
        if not isinstance(identity, str) or not identity or actual != identity:
            raise CellError('ConPTY helper identity unavailable or changed')
        out[f'conpty_helper_{index}'] = {'pid': pid, 'start_identity': identity}
    return out


class QualificationBoundaries:
    """Exact ordered native boundary pair; malformed evidence never completes."""
    def __init__(self):
        self.start = None
        self.end = None
        self.end_received_mono = None
        self.errors = []

    def consume(self, line, now):
        try:
            row = json.loads(line)
            if not isinstance(row, dict):
                raise ValueError('qualification row is not an object')
            phase = row.get('phase')
            if phase not in ('observe-start', 'observe-end'):
                raise ValueError('unexpected qualification phase')
            elapsed = row.get('elapsed_s')
            if type(elapsed) not in (int,float) or not math.isfinite(elapsed) or elapsed < 0:
                raise ValueError('invalid qualification elapsed')
            if not isinstance(row.get('slots'), list) or not row['slots']:
                raise ValueError('missing qualification slots')
            if phase == 'observe-start':
                if self.start is not None or self.end is not None:
                    raise ValueError('duplicate or reversed start')
                self.start = row
            else:
                if self.start is None or self.end is not None or elapsed <= self.start['elapsed_s']:
                    raise ValueError('missing start, duplicate end or reset elapsed')
                self.end = row
                self.end_received_mono = now
        except (ValueError, TypeError) as error:
            self.errors.append(str(error))

    def failures(self):
        return self.errors + ([] if self.start is not None and self.end is not None else ['missing observation boundary'])


def _build_sampler_config(
    spec: Mapping[str, Any], roles: Mapping[str, int], argv: Sequence[str], accounting_script=DEFAULT_ACCOUNTING
) -> Dict[str, Any]:
    accounting = pathlib.Path(accounting_script).resolve()
    modules: Dict[str, Any] = {
        'process_accounting.py': {
            'path': str(accounting),
            'sha256': bp.file_sha256(accounting) if accounting.is_file() else None,
        },
        'server_resources.py': {
            'path': str(pathlib.Path(sr.__file__).resolve()),
            'sha256': bp.file_sha256(pathlib.Path(sr.__file__).resolve()),
        },
    }
    if spec.get('process_backend') == 'libproc':
        native = pathlib.Path(DEFAULT_NATIVE_PROCESS_SAMPLE).resolve()
        try:
            import native_process_sample as nps  # type: ignore
            native = pathlib.Path(nps.__file__).resolve()
        except Exception:
            pass
        modules['native_process_sample.py'] = {
            'path': str(native),
            'sha256': bp.file_sha256(native) if native.is_file() else None,
        }
    # Win32 system counters live in windows_process_sample.py (imported by
    # server_resources). Pin those bytes whenever this host is the producer.
    # Readers must not infer this from their own sys.platform — binding validation
    # uses producer identity/provenance evidence instead.
    if sys.platform == 'win32' and spec.get('process_backend') == 'system':
        windows = pathlib.Path(DEFAULT_WINDOWS_PROCESS_SAMPLE).resolve()
        try:
            import windows_process_sample as wps  # type: ignore
            windows = pathlib.Path(wps.__file__).resolve()
        except Exception:
            pass
        modules['windows_process_sample.py'] = {
            'path': str(windows),
            'sha256': bp.file_sha256(windows) if windows.is_file() else None,
        }
    return {
        "interval_s": float(spec["sampler_interval_s"]),
        "duration_s_requested": None,
        "duration_mode": "stop_controlled",
        "schema": 2,
        "roles": {k: int(v) for k, v in roles.items()},
        "argv": list(argv),
        "process_backend": spec['process_backend'],
        "module": str(accounting),
        "module_sha256": modules['process_accounting.py']['sha256'],
        "modules": modules,
    }


def _recheck_sampler_modules(modules: Optional[Mapping[str, Any]]) -> List[str]:
    """Fail closed when any bound sampler module bytes change before completion."""
    errors: List[str] = []
    if not isinstance(modules, Mapping) or not modules:
        return ['sampler modules missing at completion']
    for name, binding in sorted(modules.items()):
        if not isinstance(binding, Mapping):
            errors.append(f'sampler module binding invalid: {name}')
            continue
        path_value = binding.get('path')
        expected = binding.get('sha256')
        if not isinstance(path_value, str) or not path_value:
            errors.append(f'sampler module path missing: {name}')
            continue
        path = pathlib.Path(path_value)
        try:
            actual = bp.file_sha256(path)
        except OSError as exc:
            errors.append(f'sampler module unreadable at completion: {name}: {exc}')
            continue
        if not isinstance(expected, str) or actual != expected:
            errors.append(f'sampler module changed since launch: {name}')
    return errors


def _parse_metadata_line(line: str) -> Optional[Dict[str, Any]]:
    line = line.strip()
    if not line.startswith("{"):
        return None
    try:
        obj = json.loads(line)
    except json.JSONDecodeError:
        return None
    if not isinstance(obj, dict):
        return None
    if "pid" in obj and "run_dir" in obj:
        return obj
    return None


def _accounting_summary(path: pathlib.Path) -> Dict[str, Any]:
    summary: Dict[str, Any] = {}
    if not path.is_file():
        return summary
    for line in path.read_text(errors="replace").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(row, dict) and row.get("type") == "summary":
            summary = row
    return summary


def _preflight_fail_report(
    report: Dict[str, Any],
    cell_dir: Optional[pathlib.Path],
    error: str,
) -> Dict[str, Any]:
    report.update(
        status="preflight_failed",
        launched=False,
        error=error,
        ended_utc=_utc(),
    )
    if cell_dir is not None:
        try:
            write_json(cell_dir / "cell_report.json", report)
        except OSError:
            pass
    return report


def run_managed_cell(
    spec_path: pathlib.Path,
    cells_root: pathlib.Path,
    *,
    accounting_script: Optional[pathlib.Path] = None,
    cwd: Optional[pathlib.Path] = None,
    _test_launcher: bool = False,
    environment: Optional[Mapping[str, str]] = None,
) -> Dict[str, Any]:
    """Run exactly one managed cell. Never retries. Never signals server/ambient."""
    # Children use different working directories. Resolve caller paths once so
    # the collector writes into the same exclusive cell directory as the runner.
    spec_path = pathlib.Path(spec_path).resolve()
    cells_root = pathlib.Path(cells_root).resolve()
    accounting_script = pathlib.Path(accounting_script or DEFAULT_ACCOUNTING).resolve()
    cwd = pathlib.Path(cwd).resolve() if cwd is not None else None
    report: Dict[str, Any] = {
        "schema": 1,
        "attempts": 1,
        "launched": False,
        "performance_acceptance": False,
        "started_utc": _utc(),
    }
    cell_dir: Optional[pathlib.Path] = None
    launcher: Optional[subprocess.Popen] = None
    collector: Optional[subprocess.Popen] = None
    collector_out_fh = None
    frontend_pid: Optional[int] = None
    frontend_identity: Optional[str] = None
    run_dir: Optional[pathlib.Path] = None
    launch_path: Optional[pathlib.Path] = None
    launcher_exit: Optional[int] = None
    collector_exit: Optional[int] = None
    max_wall_exceeded = False
    qualification_lines: List[str] = []
    startup: Dict[str, Any] = {}
    sampler_out: Optional[pathlib.Path] = None
    collector_stop_requested = False
    collector_stop_mono: Optional[float] = None
    collector_stop_path: Optional[pathlib.Path] = None
    observe_end_mono: Optional[float] = None
    launcher_exited_before_pad = False
    backend = 'system'
    boundaries = QualificationBoundaries()
    runner_errors = []
    direct = False
    direct_contract: Optional[Dict[str, Any]] = None
    direct_scanner: Optional[OwnedOutputScanner] = None
    direct_handoff_path: Optional[pathlib.Path] = None
    direct_guard_path: Optional[pathlib.Path] = None
    direct_guard_summary_path: Optional[pathlib.Path] = None
    direct_spawn: Optional[Dict[str, Any]] = None
    direct_exit: Optional[Dict[str, Any]] = None
    direct_next_grid: Optional[float] = None
    direct_last_good: Optional[Dict[str, Any]] = None
    direct_breach: Optional[Dict[str, Any]] = None
    direct_exit_pending: Optional[float] = None
    launcher_popen_return_mono: Optional[float] = None

    # --- validate + exclusive cell dir + preflight (no children) ---
    try:
        raw_spec = load_spec(spec_path)
        spec = validate_spec(raw_spec)
        direct_contract = spec.get('capture_contract')
        direct = isinstance(direct_contract, dict)
        backend = spec['process_backend']
        args = parse_diagnostic_argv(spec["diagnostic_argv"])
        require_argv_consistent_with_spec(spec, args, _test_launcher=_test_launcher)
    except (CellError, ValueError, OSError, TypeError, json.JSONDecodeError) as exc:
        # Reserve a report dir when id is known so failures are durable.
        raw: Any = None
        cid = None
        try:
            raw = json.loads(pathlib.Path(spec_path).read_text())
            cid = raw.get("id") if isinstance(raw, dict) else None
        except Exception:
            raw = None
            cid = None
        if isinstance(cid, str) and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', cid):
            try:
                cell_dir = exclusive_cell_dir(pathlib.Path(cells_root), cid)
                report["cell_dir"] = str(cell_dir)
                try:
                    write_json(cell_dir / "cell_spec.json", raw if isinstance(raw, dict) else {})
                except OSError:
                    pass
            except FileExistsError:
                report["cell_dir"] = str(pathlib.Path(cells_root) / cid)
                raise
            except OSError:
                pass
        return _preflight_fail_report(report, cell_dir, str(exc))

    try:
        cell_dir = exclusive_cell_dir(pathlib.Path(cells_root), spec["id"])
    except FileExistsError:
        raise
    report["cell_dir"] = str(cell_dir)
    write_json(cell_dir / "cell_spec.json", raw_spec)
    if direct:
        assert direct_contract is not None
        expected_cell = _canonical(direct_contract['cell_dir'])
        if cell_dir.resolve() != expected_cell:
            return _preflight_fail_report(report, cell_dir, 'direct owner cell directory mismatch')
        direct_handoff_path = _canonical(direct_contract['frontend_handoff_path'])
        next_handoff = direct_handoff_path.with_name('frontend-handoff.next.json')
        direct_run = _canonical(direct_contract['run_dir'])
        if any(path.exists() or path.is_symlink()
               for path in (direct_handoff_path, next_handoff, direct_run)):
            return _preflight_fail_report(report, cell_dir, 'direct owner reserved path already exists')
        try:
            direct_run.mkdir(exist_ok=False)
        except OSError as exc:
            return _preflight_fail_report(
                report, cell_dir, f'direct owner run reservation failed: {exc}'
            )
        run_dir = direct_run
        direct_guard_path = cell_dir / 'direct-owner-guard.jsonl'
        direct_guard_summary_path = cell_dir / 'direct-owner-guard-summary.json'
        direct_guard_path.open('x', encoding='utf-8').close()
        direct_scanner = OwnedOutputScanner(
            direct_contract['owned_output_paths'],
            replaceable_paths=(direct_handoff_path, next_handoff),
        )

    try:
        pf = preflight(spec, args)
    except (CellError, ValueError, OSError, sr.SampleError, TypeError, json.JSONDecodeError) as exc:
        return _preflight_fail_report(report, cell_dir, str(exc))

    write_json(
        cell_dir / "logs" / "preflight.json",
        {
            "server_pid": pf["server_pid"],
            "server_start_identity": pf["server_sample"].get("start_identity"),
            "ambient": {
                k: v["sample"].get("start_identity") for k, v in pf["ambient_identities"].items()
            },
            "provenance_status": pf["provenance"].get("status"),
            "binary": pf["binary"],
            "direct_preflight": pf.get("direct_preflight"),
        },
    )

    # --- launch + collect ---
    try:
        effective_cli = [str(x) for x in spec["launcher_argv"]]
        # Prelaunch role map: server/controller/ambient only (owned children added at runtime).
        sampler_roles_placeholder = {
            "game_server": int(spec["game_server_pid"]),
            "controller": os.getpid(),
            **{k: int(v) for k, v in spec["ambient_helpers"].items()},
        }
        seen: Dict[int, str] = {}
        for name, pid in sampler_roles_placeholder.items():
            if pid in seen:
                raise CellError(f"duplicate PID {pid} for roles {seen[pid]!r} and {name!r}")
            seen[pid] = name

        # Exact role identities for receipt binding: sample controller now; server/ambient
        # reuse preflight samples; launcher/collector immediately after each Popen.
        role_identities: Dict[str, Dict[str, Any]] = {
            "game_server": {
                "pid": int(pf["server_pid"]),
                "start_identity": pf["server_sample"].get("start_identity"),
            },
            "controller": _role_identity(os.getpid(), backend=backend, role="controller"),
        }
        if not isinstance(role_identities["game_server"]["start_identity"], str) or not role_identities["game_server"]["start_identity"]:
            raise CellError("role 'game_server' identity unavailable")
        for name, info in sorted(pf["ambient_identities"].items()):
            sample = info.get("sample") if isinstance(info, dict) else None
            identity = sample.get("start_identity") if isinstance(sample, dict) else None
            if not isinstance(identity, str) or not identity:
                raise CellError(f"role {name!r} identity unavailable")
            role_identities[name] = {"pid": int(info["pid"]) if "pid" in info else int(spec["ambient_helpers"][name]), "start_identity": identity}

        cache_provenance_path: Optional[pathlib.Path] = None
        if spec.get("cache_dir") is not None:
            if not _test_launcher:
                expected = expected_unpack_root(cwd=cwd).resolve()
                actual_unpack = pathlib.Path(spec["unpack_root"]).resolve()
                if actual_unpack != expected:
                    raise CellError(
                        f"unpack_root canonical {actual_unpack} != inherited {expected}"
                    )
            snapshot = (
                pf.get('direct_cache_snapshot') if direct
                else cp.capture(spec["cache_dir"], spec["unpack_root"])
            )
            if not isinstance(snapshot, dict):
                raise CellError('direct owner cache snapshot unavailable after preflight')
            cache_provenance_path = cell_dir / "cache-provenance.json"
            write_json(cache_provenance_path, snapshot)

        launch_path = cell_dir / "launch.json"
        sampler_argv_preview = [sys.executable, str(accounting_script)]
        sampler_config = _build_sampler_config(
            spec, sampler_roles_placeholder, sampler_argv_preview, accounting_script
        )
        launch_rec = mr.create_launch(
            launch_path,
            cell_id=spec["id"],
            index=int(spec["index"]),
            kind=spec["kind"],
            effective_cli=effective_cli,
            binary=pf["binary"],
            manifest_path=spec["build_manifest"],
            server_identity_path=spec["server_identity_path"],
            host_conditions_path=spec["host_conditions_path"],
            sampler_config=sampler_config,
            cache_provenance_path=str(cache_provenance_path) if cache_provenance_path is not None else None,
            capture_mode=DIRECT_MODE if direct else None,
            direct_admission_bindings=(
                (pf.get('direct_preflight') or {}).get('admission_bindings')
                if direct else None
            ),
        )
        report["launch_started_utc"] = launch_rec["started_utc"]

        if direct:
            direct_preflight = pf.get('direct_preflight') or {}
            bindings = direct_preflight.get('admission_bindings')
            expiry = direct_preflight.get('release_expires_unix_s')
            if not isinstance(bindings, dict) or not bindings:
                raise CellError('direct owner admission bindings unavailable before launch')
            bp.recheck_files(pf['provenance']['files'])
            bp.recheck_files(bindings)
            if (not isinstance(expiry, (int, float)) or isinstance(expiry, bool)
                    or not math.isfinite(expiry) or time.time() >= expiry):
                raise CellError('direct owner root release expired before launch')

        launcher_log = cell_dir / "logs" / "launcher.log"
        env = dict(environment) if environment is not None else os.environ.copy()
        if _test_launcher:
            env["FIXTURE_BINARY"] = pf["binary"]
            env["FIXTURE_EFFECTIVE_CLI"] = json.dumps(effective_cli)
        wall_deadline = time.monotonic() + float(spec["max_wall_s"])
        t0 = time.monotonic()
        log_fh = launcher_log.open("xb")
        try:
            launcher = subprocess.Popen(
                effective_cli,
                cwd=str(cwd) if cwd else None,
                stdout=log_fh,
                stderr=subprocess.STDOUT,
                env=env,
            )
        finally:
            log_fh.close()
        report["launched"] = True
        report["launcher_pid"] = launcher.pid
        launcher_popen_return_mono = time.monotonic()
        startup["launcher_start_gap_s"] = launcher_popen_return_mono - t0
        role_identities["launcher"] = _role_identity(
            launcher.pid, backend=backend, role="launcher"
        )
        if direct_scanner is not None:
            initial_output = direct_scanner.scan()
            report['direct_owner_initial_output'] = initial_output
            if initial_output['total_bytes'] > DIRECT_OUTPUT_BYTES:
                raise CellError(
                    'direct owner output exceeds limit immediately after launcher creation'
                )

        roles = dict(sampler_roles_placeholder)
        if launcher.pid in seen and seen[launcher.pid] != "launcher":
            raise CellError(
                f"launcher PID {launcher.pid} already mapped as {seen[launcher.pid]!r}"
            )
        roles["launcher"] = launcher.pid
        seen2: Dict[int, str] = {}
        for name, pid in roles.items():
            if pid in seen2:
                raise CellError(f"duplicate PID {pid} for roles {seen2[pid]!r} and {name!r}")
            seen2[pid] = name

        sampler_out = cell_dir / "process_accounting.jsonl"
        collector_stop_path = (cell_dir / COLLECTOR_STOP_BASENAME).resolve()
        if collector_stop_path.exists():
            raise CellError(f'stale collector stop path: {collector_stop_path}')
        try:
            collector_stop_path.relative_to(cell_dir.resolve())
        except ValueError as exc:
            raise CellError(f'collector stop path escapes cell dir: {collector_stop_path}') from exc
        collector_started = False

        def start_collector():
            nonlocal collector, collector_out_fh, collector_exit, collector_started
            role_args = [f"{name}={pid}" for name, pid in sorted(roles.items())]
            collector_argv = [sys.executable, str(accounting_script), *role_args,
                              str(sampler_out), "--interval", str(float(spec["sampler_interval_s"])),
                              '--process-backend', backend, '--stop-file', str(collector_stop_path)]
            report["sampler_argv"] = collector_argv
            report["sampler_roles"] = dict(roles)
            report["collector_stop_path"] = str(collector_stop_path)
            t1 = time.monotonic()
            collector_out_fh = (cell_dir / "logs" / "collector.stdout").open("xb")
            try:
                collector = subprocess.Popen(collector_argv, cwd=str(_ROOT),
                                              stdout=collector_out_fh, stderr=subprocess.STDOUT)
                report["collector_pid"] = collector.pid
                startup["collector_start_gap_s"] = time.monotonic() - t1
                startup["roles"] = dict(roles)
                role_identities["collector"] = _role_identity(collector.pid, backend=backend, role="collector")
                collector_started = True
            except OSError as exc:
                collector = None
                collector_exit = 127
                report["sampler_premature_exit"] = 127
                report["sampler_start_error"] = str(exc)
                startup["collector_start_error"] = str(exc)
                runner_errors.append(f"collector identity unavailable: {exc}")
            except CellError as exc:
                runner_errors.append(str(exc))

        # Unix collection ordering is unchanged: begin sampling immediately
        # after the launcher is owned. Windows waits for metadata so it can
        # distinguish ConPTY terminal transport from panel/headless paths.
        if sys.platform != 'win32':
            start_collector()

        log_tail = IncrementalLines(launcher_log)
        qual_tail: Optional[IncrementalLines] = None
        interval = float(spec["sampler_interval_s"])
        metadata: Optional[Dict[str, Any]] = None

        while True:
            now = time.monotonic()
            if now >= wall_deadline:
                max_wall_exceeded = True
                report["max_wall_exceeded"] = True
                break

            if direct:
                assert direct_handoff_path is not None
                assert direct_scanner is not None
                assert direct_guard_path is not None
                assert launcher_popen_return_mono is not None
                handoff = load_direct_handoff(direct_handoff_path)
                if direct_spawn is None:
                    if handoff is None:
                        pre_output = direct_scanner.scan()
                        if pre_output['total_bytes'] > DIRECT_OUTPUT_BYTES:
                            runner_errors.append('direct owner output limit before frontend handoff')
                            break
                        if now > launcher_popen_return_mono + 5.0:
                            runner_errors.append('direct owner frontend handoff deadline exceeded')
                            break
                    else:
                        candidate = validate_frontend_handoff(
                            handoff, launcher_pid=launcher.pid, run_dir=run_dir,
                            receive_monotonic_s=now,
                            launcher_start_monotonic_s=t0,
                        )
                        if handoff.get('state') != 'spawned':
                            runner_errors.append(
                                'direct owner exited handoff arrived before a valid RSS sample'
                            )
                            break
                        forbidden = {*roles.values(), collector.pid if collector else -1}
                        if candidate['frontend_pid'] in forbidden:
                            runner_errors.append('direct owner frontend PID is a reserved process')
                            break
                        started = time.monotonic()
                        process_sample = pa.process_sampler('system')(
                            candidate['frontend_pid'], timeout=min(SAMPLE_TIMEOUT_S, .2)
                        )
                        process_sample = dict(process_sample)
                        process_sample['pid'] = candidate['frontend_pid']
                        process_sample['parent_pid'] = parent_pid(
                            candidate['frontend_pid'], timeout=min(SAMPLE_TIMEOUT_S, .2)
                        )
                        ended = time.monotonic()
                        row = direct_guard_sample(
                            scheduled=candidate['spawn_before_monotonic_s'],
                            started=started, ended=ended,
                            spawn_before=candidate['spawn_before_monotonic_s'],
                            frontend_pid=candidate['frontend_pid'],
                            frontend_identity=candidate['frontend_start_identity'],
                            launcher_pid=launcher.pid, process_sample=process_sample,
                            mem_available=read_linux_mem_available(),
                            output_scanner=direct_scanner,
                        )
                        append_json_line(direct_guard_path, row)
                        require_first_sample_deadline(candidate, sample_end_monotonic_s=ended)
                        if row['failure_reason']:
                            direct_breach = row
                            runner_errors.append('direct owner guard: ' + row['failure_reason'])
                            break
                        direct_spawn = candidate
                        frontend_pid = candidate['frontend_pid']
                        frontend_identity = candidate['frontend_start_identity']
                        direct_last_good = row
                        direct_next_grid = candidate['spawn_before_monotonic_s'] + DIRECT_INTERVAL_S
                        report['frontend_pid'] = frontend_pid
                        report['frontend_start_identity'] = frontend_identity
                        report['run_dir'] = str(run_dir)
                else:
                    if handoff is not None:
                        if handoff.get('spawn') != direct_spawn:
                            runner_errors.append('direct owner handoff immutable spawn changed')
                            break
                        if handoff.get('state') == 'exited' and direct_exit is None:
                            direct_exit = validate_exit_handoff(
                                handoff, direct_spawn, receipt_monotonic_s=now
                            )
                            elapsed = (direct_exit['wait_return_monotonic_s']
                                       - direct_spawn['spawn_before_monotonic_s'])
                            if elapsed > DIRECT_FRONTEND_WALL_S:
                                runner_errors.append('direct owner frontend wall above ceiling')
                                break
                    if direct_exit is None and direct_next_grid is not None and now >= direct_next_grid:
                        started = time.monotonic()
                        try:
                            process_sample = pa.process_sampler('system')(
                                direct_spawn['frontend_pid'], timeout=min(SAMPLE_TIMEOUT_S, .2)
                            )
                            process_sample = dict(process_sample)
                            process_sample['pid'] = direct_spawn['frontend_pid']
                            process_sample['parent_pid'] = parent_pid(
                                direct_spawn['frontend_pid'], timeout=min(SAMPLE_TIMEOUT_S, .2)
                            )
                        except (CellError, sr.SampleError, OSError, ValueError, TypeError):
                            process_sample = None
                        ended = time.monotonic()
                        if process_sample is None:
                            if direct_exit_pending is None:
                                direct_exit_pending = started
                                append_json_line(direct_guard_path, {
                                    'scheduled_monotonic_s': direct_next_grid,
                                    'start_monotonic_s': started,
                                    'end_monotonic_s': ended,
                                    'frontend_pid': direct_spawn['frontend_pid'],
                                    'frontend_start_identity': direct_spawn['frontend_start_identity'],
                                    'frontend_rss_bytes': None,
                                    'state': 'exit_pending',
                                })
                            elif now > direct_exit_pending + DIRECT_INTERVAL_S:
                                runner_errors.append(
                                    'direct owner frontend disappeared without bounded exit proof'
                                )
                                break
                        else:
                            direct_exit_pending = None
                            row = direct_guard_sample(
                                scheduled=direct_next_grid, started=started, ended=ended,
                                spawn_before=direct_spawn['spawn_before_monotonic_s'],
                                frontend_pid=direct_spawn['frontend_pid'],
                                frontend_identity=direct_spawn['frontend_start_identity'],
                                launcher_pid=launcher.pid, process_sample=process_sample,
                                mem_available=read_linux_mem_available(),
                                output_scanner=direct_scanner,
                            )
                            append_json_line(direct_guard_path, row)
                            if row['failure_reason']:
                                direct_breach = row
                                runner_errors.append('direct owner guard: ' + row['failure_reason'])
                                break
                            direct_last_good = row
                            direct_next_grid += DIRECT_INTERVAL_S
                    elif direct_exit is not None and direct_next_grid is not None and now >= direct_next_grid:
                        started = time.monotonic()
                        output_sample = direct_scanner.scan()
                        mem_available = read_linux_mem_available()
                        ended = time.monotonic()
                        failure = None
                        if ended > direct_next_grid + DIRECT_INTERVAL_S:
                            failure = 'guard_schedule_overrun'
                        elif type(mem_available) is not int:
                            failure = 'MemAvailable_unavailable'
                        elif mem_available < DIRECT_MEM_AVAILABLE_BYTES:
                            failure = 'MemAvailable_below_floor'
                        elif output_sample['total_bytes'] > DIRECT_OUTPUT_BYTES:
                            failure = 'owned_output_above_ceiling'
                        row = {
                            'scheduled_monotonic_s': direct_next_grid,
                            'start_monotonic_s': started, 'end_monotonic_s': ended,
                            'lateness_s': started - direct_next_grid,
                            'acquisition_s': ended - started,
                            'frontend_pid': direct_spawn['frontend_pid'],
                            'frontend_start_identity': direct_spawn['frontend_start_identity'],
                            'frontend_rss_bytes': None, 'state': 'frontend_exited',
                            'mem_available_bytes': mem_available,
                            'owned_output_bytes': output_sample['total_bytes'],
                            'owned_output_digest': output_sample['digest'],
                            'elapsed_frontend_wall_s': (
                                direct_exit['wait_return_monotonic_s']
                                - direct_spawn['spawn_before_monotonic_s']
                            ),
                            'failure_reason': failure,
                        }
                        append_json_line(direct_guard_path, row)
                        if failure:
                            direct_breach = row
                            runner_errors.append('direct owner guard: ' + failure)
                            break
                        direct_last_good = row
                        direct_next_grid += DIRECT_INTERVAL_S

            for line in log_tail.poll():
                meta = _parse_metadata_line(line)
                if meta and metadata is None:
                    metadata = meta
                    reported_run = pathlib.Path(str(meta["run_dir"])) if meta.get("run_dir") else None
                    if direct:
                        if reported_run is None or reported_run.resolve() != pathlib.Path(run_dir).resolve():
                            raise CellError('direct owner metadata run directory mismatch')
                        if direct_spawn is None or meta.get('pid') != direct_spawn['frontend_pid']:
                            raise CellError('direct owner metadata arrived before matching handoff')
                    else:
                        run_dir = reported_run
                        if meta.get("pid") is not None:
                            frontend_pid, frontend_identity = _capture_owned_frontend(
                                meta['pid'], launcher.pid,
                                {*roles.values(), collector.pid if collector else -1}, backend)
                    needs_conpty_helpers = (
                        sys.platform == 'win32'
                        and meta.get('terminal_transport') == 'conpty'
                    )
                    helpers = _capture_conpty_helpers(
                        meta, launcher.pid, { *roles.values(), frontend_pid or -1 }, backend,
                        required=needs_conpty_helpers,
                    )
                    role_identities.update(helpers)
                    roles.update({name: info['pid'] for name, info in helpers.items()})
                    if not collector_started:
                        start_collector()
                    report["frontend_pid"] = frontend_pid
                    report["frontend_start_identity"] = frontend_identity
                    report["run_dir"] = str(run_dir) if run_dir else None
                    if run_dir is not None:
                        qual_tail = IncrementalLines(run_dir / "samples.qualification.jsonl")

            if qual_tail is not None:
                for qline in qual_tail.poll():
                    qualification_lines.append(qline)
                    boundaries.consume(qline, time.monotonic())
                observe_end_mono = boundaries.end_received_mono

            if metadata is None and launcher.poll() is not None and (direct or not collector_started):
                runner_errors.append('managed launcher produced no identity-bound metadata')
                break

            # After a validated observe-end, keep the collector alive for at
            # least two sampler intervals even if the launcher already exited.
            pad_elapsed = (
                observe_end_mono is not None
                and now >= observe_end_mono + 2.0 * interval
            )
            if (
                not direct
                and
                not collector_stop_requested
                and pad_elapsed
            ):
                if collector is not None and collector.poll() is None:
                    stop_info = _request_collector_stop(collector, collector_stop_path)
                    report["collector_stop_request"] = stop_info
                    if not (stop_info.get("stop_file") or {}).get("ok"):
                        runner_errors.append(
                            f"collector stop file request failed: "
                            f"{(stop_info.get('stop_file') or {}).get('error')}"
                        )
                collector_stop_requested = True
                collector_stop_mono = now
                report["collector_stop_requested_utc"] = _utc()

            launcher_done = launcher.poll() is not None
            if launcher_done:
                if launcher_exit is None:
                    launcher_exit = launcher.returncode
                if direct:
                    if direct_exit is None:
                        runner_errors.append('direct owner launcher exited without exit handoff')
                    if collector is not None and collector.poll() is None and not collector_stop_requested:
                        stop_info = _request_collector_stop(collector, collector_stop_path)
                        report["collector_stop_request"] = stop_info
                        if not (stop_info.get("stop_file") or {}).get("ok"):
                            runner_errors.append(
                                'collector stop file request failed: '
                                + str((stop_info.get('stop_file') or {}).get('error'))
                            )
                        collector_stop_requested = True
                        collector_stop_mono = now
                    break
                # Hold the loop only while a valid end still needs its pad.
                still_need_pad = (
                    observe_end_mono is not None
                    and collector_stop_mono is None
                    and now < observe_end_mono + 2.0 * interval
                )
                if still_need_pad:
                    # Launcher left before the required post-end collector hold.
                    # Keep waiting the pad wall-clock; do not fake role lifetime.
                    # Continuous required-role sampling will honestly fail once the
                    # launcher PID is gone; receipt stays failed (no retry).
                    launcher_exited_before_pad = True
                else:
                    if (
                        collector is not None
                        and collector.poll() is None
                        and not collector_stop_requested
                    ):
                        stop_info = _request_collector_stop(collector, collector_stop_path)
                        report["collector_stop_request"] = stop_info
                        if not (stop_info.get("stop_file") or {}).get("ok"):
                            runner_errors.append(
                                f"collector stop file request failed: "
                                f"{(stop_info.get('stop_file') or {}).get('error')}"
                            )
                        collector_stop_requested = True
                    break

            if (
                collector is not None
                and collector.poll() is not None
                and not collector_stop_requested
            ):
                collector_exit = collector.returncode
                report["sampler_premature_exit"] = collector_exit

            time.sleep(min(0.05, interval / 2.0))

        for line in log_tail.poll():
            meta = _parse_metadata_line(line)
            if meta and run_dir is None and meta.get("run_dir"):
                run_dir = pathlib.Path(str(meta["run_dir"]))
                report["run_dir"] = str(run_dir)
            if not direct and meta and frontend_pid is None and meta.get("pid") is not None:
                frontend_pid, frontend_identity = _capture_owned_frontend(
                    meta['pid'], launcher.pid,
                    {*roles.values(), collector.pid if collector else -1}, backend)

        if direct:
            if direct_spawn is None:
                runner_errors.append('direct owner spawn handoff was never accepted')
            if direct_exit is None:
                runner_errors.append('direct owner exit handoff was never accepted')
            if direct_spawn is not None and direct_exit is not None and run_dir is not None:
                try:
                    final_metadata = json.loads((pathlib.Path(run_dir) / 'metadata.json').read_text())
                    validate_direct_final_metadata(
                        final_metadata, direct_spawn, direct_exit, pathlib.Path(run_dir)
                    )
                    metadata = final_metadata
                except (CellError, OSError, ValueError, TypeError, json.JSONDecodeError) as exc:
                    runner_errors.append(str(exc))

        # The loop exits only on launcher completion or the external deadline.
        # Reap direct children and independently inspect the captured frontend.
        # Always attempt graceful collector stop before escalation cleanup.
        if (
            collector is not None
            and collector.poll() is None
            and not collector_stop_requested
            and collector_stop_path is not None
        ):
            stop_info = _request_collector_stop(collector, collector_stop_path)
            report["collector_stop_request"] = stop_info
            collector_stop_requested = True
        cleanup = {
            "collector": _terminate_owned(
                collector, label="collector", stop_path=collector_stop_path
            ),
            "launcher": _terminate_owned(launcher, label="launcher"),
            "frontend": _cleanup_frontend(
                frontend_pid,
                frontend_identity,
                backend=backend,
                launcher_pid=launcher.pid if launcher is not None else report.get("launcher_pid"),
            ),
        }
        launcher_exit = launcher.poll() if launcher is not None else launcher_exit
        collector_exit = collector.poll() if collector is not None else collector_exit
        if qual_tail is not None:
            for qline in qual_tail.poll():
                qualification_lines.append(qline)
                boundaries.consume(qline, time.monotonic())
            if qual_tail.partial:
                runner_errors.append('partial qualification row at completion')
        runner_errors.extend(boundaries.failures())
        # Pad failure only when observe-end arrived but the 2-interval stop did not.
        # Missing end is already reported via boundaries.failures().
        if (
            not direct
            and
            collector_stop_mono is None
            and not max_wall_exceeded
            and observe_end_mono is not None
        ):
            runner_errors.append('collector did not reach post-observation stop boundary')
        if not direct and launcher_exited_before_pad:
            runner_errors.append(
                'launcher exited before post-observation collector coverage'
            )
            report['launcher_exited_before_pad'] = True
        if max_wall_exceeded:
            runner_errors.append('max_wall_exceeded')
        if any(info.get('orphan_risk') for info in cleanup.values()):
            runner_errors.append('owned child cleanup unresolved')
        if direct and direct_scanner is not None and direct_guard_summary_path is not None:
            try:
                final_output = direct_scanner.scan()
                guard_summary = {
                    'schema': 1, 'mode': DIRECT_MODE,
                    'thresholds': {
                        'mem_available_floor_bytes': DIRECT_MEM_AVAILABLE_BYTES,
                        'frontend_rss_limit_bytes': DIRECT_FRONTEND_RSS_BYTES,
                        'owned_output_limit_bytes': DIRECT_OUTPUT_BYTES,
                        'frontend_wall_limit_s': DIRECT_FRONTEND_WALL_S,
                        'interval_s': DIRECT_INTERVAL_S,
                    },
                    'last_good_row': direct_last_good,
                    'breach_row': direct_breach,
                    'exit_pending_started_monotonic_s': direct_exit_pending,
                    'terminal_unsampled_interval': (
                        {
                            'last_sample_end_monotonic_s': direct_last_good.get('end_monotonic_s'),
                            'wait_return_monotonic_s': direct_exit.get('wait_return_monotonic_s'),
                            'continuous_rss_claim': False,
                        }
                        if direct_last_good is not None and direct_exit is not None else None
                    ),
                    'cleanup_request_monotonic_s': time.monotonic(),
                    'final_post_cleanup_output_scan': final_output,
                    'sampled_ceiling_limitation': (
                        'RSS/output are sampled ceilings; magnitude overshoot between samples is unbounded.'
                    ),
                }
                write_json(direct_guard_summary_path, guard_summary)
                final_with_summary = direct_scanner.scan()
                report['direct_owner_final_output'] = final_with_summary
                if final_with_summary['total_bytes'] > DIRECT_OUTPUT_BYTES:
                    runner_errors.append('direct owner final output above ceiling')
            except (CellError, OSError, ValueError, TypeError) as exc:
                runner_errors.append('direct owner final output scan failed: ' + str(exc))
        report["cleanup"] = cleanup
        report["startup"] = startup
        report["qualification_line_count"] = len(qualification_lines)
        report["launcher_exit_code"] = launcher_exit
        report["collector_exit_code"] = collector_exit
        report["collector_stop_requested"] = collector_stop_requested
        if collector_stop_mono is not None:
            report["collector_stop_after_observe_s"] = collector_stop_mono - (
                observe_end_mono or collector_stop_mono
            )

        summary = _accounting_summary(sampler_out) if sampler_out else {}
        runner_errors.extend(_recheck_sampler_modules(sampler_config.get("modules") if isinstance(sampler_config, dict) else None))
        if direct:
            try:
                bp.recheck_files(pf['provenance']['files'])
                admissions = (pf.get('direct_preflight') or {}).get('admission_bindings')
                if not isinstance(admissions, dict) or not admissions:
                    raise ValueError('direct owner admission bindings unavailable')
                bp.recheck_files(admissions)
            except (ValueError, OSError, TypeError, KeyError) as exc:
                runner_errors.append('direct owner build/source binding changed: ' + str(exc))
        # Require full identity set before claiming complete; never invent missing roles.
        required_roles = set(sampler_roles_placeholder) | {"launcher", "collector"}
        if set(role_identities) != required_roles:
            missing = sorted(required_roles - set(role_identities))
            if missing:
                runner_errors.append(
                    "role identities unavailable: " + ", ".join(missing)
                )
        for name, identity in sorted(role_identities.items()):
            if (
                not isinstance(identity, dict)
                or type(identity.get("pid")) is not int
                or not isinstance(identity.get("start_identity"), str)
                or not identity["start_identity"]
            ):
                runner_errors.append(f"role {name!r} identity incomplete")
        sampler_result = {
            "exit_code": int(collector_exit) if isinstance(collector_exit, int) else 1,
            "output": str(sampler_out) if sampler_out and sampler_out.is_file() else None,
            "completion": summary.get("completion"),
            "status": summary.get("status"),
            "duration_mode": "stop_controlled",
            "duration_s_requested": None,
            "role_identities": {
                name: {"pid": int(info["pid"]), "start_identity": info["start_identity"]}
                for name, info in role_identities.items()
                if isinstance(info, dict)
                and type(info.get("pid")) is int
                and isinstance(info.get("start_identity"), str)
                and info["start_identity"]
            },
            "conpty_helpers": {
                name: {"pid": int(info["pid"]), "start_identity": info["start_identity"]}
                for name, info in role_identities.items()
                if name.startswith("conpty_helper_")
            },
            "launcher_pid": int(launcher.pid) if launcher is not None else report.get("launcher_pid"),
            "collector_pid": int(collector.pid)
            if collector is not None
            else report.get("collector_pid"),
        }
        report["sampler_result"] = sampler_result
        report["role_identities"] = sampler_result["role_identities"]

        receipt_path = cell_dir / "receipt.json"
        if launch_path is not None and launch_path.is_file():
            receipt = mr.complete(
                receipt_path,
                launch_path=launch_path,
                run_dir=str(run_dir) if run_dir is not None else None,
                launcher_exit_code=int(launcher_exit) if isinstance(launcher_exit, int) else 1,
                sampler_result=sampler_result,
                runner_errors=runner_errors,
            )
            report["receipt_status"] = receipt.get("status")
            report["binding_errors"] = receipt.get("binding_errors")
            report["status"] = receipt.get("status") or "failed_or_unavailable"
        else:
            report["status"] = "failed_or_unavailable"

        if max_wall_exceeded and report.get("status") == "completed":
            report["status"] = "failed_or_unavailable"
            errs = list(report.get("binding_errors") or [])
            errs.append("max_wall_exceeded")
            report["binding_errors"] = errs

        if direct:
            report['direct_owner'] = {'spawn': direct_spawn, 'exit': direct_exit}
        report["ended_utc"] = _utc()
        write_json(cell_dir / "cell_report.json", report)
        return report

    except Exception as exc:
        report["status"] = "failed_or_unavailable"
        report["error"] = str(exc)
        report["ended_utc"] = _utc()
        if direct:
            report['direct_owner'] = {'spawn': direct_spawn, 'exit': direct_exit}
        if (
            collector is not None
            and collector.poll() is None
            and not collector_stop_requested
            and collector_stop_path is not None
        ):
            try:
                report["collector_stop_request"] = _request_collector_stop(
                    collector, collector_stop_path
                )
                collector_stop_requested = True
            except Exception:
                pass
        cleanup = {
            "collector": _terminate_owned(
                collector, label="collector", stop_path=collector_stop_path
            ),
            "launcher": _terminate_owned(launcher, label="launcher"),
            "frontend": _cleanup_frontend(
                frontend_pid,
                frontend_identity,
                backend=backend,
                launcher_pid=launcher.pid if launcher is not None else report.get("launcher_pid"),
            ),
        }
        report["cleanup"] = cleanup
        if (direct and direct_guard_summary_path is not None
                and not direct_guard_summary_path.exists()):
            try:
                final_output = direct_scanner.scan() if direct_scanner is not None else None
                write_json(direct_guard_summary_path, {
                    'schema': 1, 'mode': DIRECT_MODE, 'status': 'failed',
                    'error': str(exc), 'last_good_row': direct_last_good,
                    'breach_row': direct_breach,
                    'exit_pending_started_monotonic_s': direct_exit_pending,
                    'cleanup': cleanup,
                    'final_post_cleanup_output_scan': final_output,
                    'performance_acceptance': False,
                })
            except Exception as summary_error:
                report['direct_guard_summary_error'] = str(summary_error)
        if cell_dir is not None:
            try:
                write_json(cell_dir / "cell_report.json", report)
            except OSError:
                pass
            try:
                if (
                    launch_path
                    and launch_path.is_file()
                    and not (cell_dir / "receipt.json").exists()
                ):
                    out_path = cell_dir / "process_accounting.jsonl"
                    mr.complete(
                        cell_dir / "receipt.json",
                        launch_path=launch_path,
                        run_dir=str(run_dir) if run_dir else None,
                        launcher_exit_code=int(launcher_exit)
                        if isinstance(launcher_exit, int)
                        else 1,
                        sampler_result={
                            "exit_code": int(collector_exit)
                            if isinstance(collector_exit, int)
                            else 1,
                            "output": str(out_path) if out_path.is_file() else None,
                        },
                        runner_errors=[str(exc)],
                    )
            except Exception:
                pass
        return report
    finally:
        if collector_out_fh is not None:
            try:
                collector_out_fh.close()
            except OSError:
                pass


def build_cli() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("spec", type=pathlib.Path, help="explicit cell JSON spec path")
    p.add_argument(
        "cells_root", type=pathlib.Path, help="parent directory for exclusive cell dirs"
    )
    return p


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = build_cli()
    a = p.parse_args(list(argv) if argv is not None else None)
    try:
        report = run_managed_cell(a.spec, a.cells_root)
    except FileExistsError as exc:
        print(
            json.dumps({"status": "error", "error": f"cell dir exists: {exc}"}),
            flush=True,
        )
        return 2
    print(
        json.dumps(
            {
                "status": report.get("status"),
                "cell_dir": report.get("cell_dir"),
                "attempts": report.get("attempts"),
                "launched": report.get("launched"),
                "performance_acceptance": False,
            }
        ),
        flush=True,
    )
    if report.get("status") == "completed":
        return 0
    if report.get("status") == "preflight_failed":
        return 3
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
