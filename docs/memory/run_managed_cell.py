#!/usr/bin/env python3
"""One-cell managed orchestration: reviewed launcher + schema-2 process accounting.

Explicit predeclared JSON cell spec only — no name/env discovery, no retries.
Does not start, stop, or signal the game server or ambient helper PIDs.
Does not claim qualification, overhead, or performance acceptance.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import re
import signal
import subprocess
import sys
import time
from typing import Any, Dict, List, Mapping, Optional, Sequence

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
    has_cache = 'cache_dir' in out and out['cache_dir'] is not None
    has_unpack = 'unpack_root' in out and out['unpack_root'] is not None
    if has_cache ^ has_unpack:
        raise CellError('spec.cache_dir and spec.unpack_root must both be set or both omitted')
    if has_cache:
        if not isinstance(out.get('cache_dir'), str) or not out['cache_dir']:
            raise CellError('spec.cache_dir must be a non-empty path string')
        if not isinstance(out.get('unpack_root'), str) or not out['unpack_root']:
            raise CellError('spec.unpack_root must be a non-empty path string')
    pids = [gs, os.getpid(), *cleaned_ambient.values()]
    if len(pids) != len(set(pids)):
        raise CellError('duplicate process role PID')
    return out


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


def preflight(
    spec: Mapping[str, Any],
    args: argparse.Namespace,
    *,
    sample_timeout: float = SAMPLE_TIMEOUT_S,
) -> Dict[str, Any]:
    """Verify build fixtures and live server identity. Never signals the server."""
    binary = pathlib.Path(spec["binary"]).resolve(strict=True)
    provenance = bp.verify_build(
        pathlib.Path(spec["build_manifest"]),
        spec["build_role"],
        spec["frontend"],
        binary,
        pathlib.Path(spec["nav_pack"]),
        pathlib.Path(spec["nav_flags"]),
        pathlib.Path(spec["catalog_path"]),
    )
    sidecars = {}
    for key in ("server_identity_path", "host_conditions_path"):
        p = pathlib.Path(spec[key]).resolve(strict=True)
        sidecars[key] = json.loads(p.read_text())
    sample = pa.process_sampler(spec['process_backend'])
    server_sample = sample(int(spec["game_server_pid"]), timeout=sample_timeout)
    identity = json.loads(pathlib.Path(spec['server_identity_path']).read_text())
    if (not isinstance(identity, dict) or type(identity.get('pid')) is not int or identity['pid'] != spec['game_server_pid']
            or identity.get('start_identity') != server_sample.get('start_identity')):
        raise CellError('server sidecar differs from sampled PID/start identity')
    host = sidecars['host_conditions_path']
    native = host.get('native_preflight') if isinstance(host, dict) else None
    if isinstance(native, dict):
        checked_server = native.get('server')
        if not isinstance(checked_server, dict):
            raise CellError('native host conditions must serialize checked server')
        if (checked_server.get('ProcessId') != spec['game_server_pid']
                or checked_server.get('Name') != 'node.exe'
                or checked_server.get('SessionId') != 2
                or not isinstance(checked_server.get('CreationDate'), str)
                or not checked_server['CreationDate']):
            raise CellError('native host conditions server does not match preflight server')
    ambient_identities: Dict[str, Any] = {}
    for name, pid in sorted(spec["ambient_helpers"].items()):
        ambient_identities[name] = {
            "pid": pid,
            "sample": sample(int(pid), timeout=sample_timeout),
        }
    return {
        "provenance": provenance,
        "server_sample": server_sample,
        "checked_server": native.get('server') if isinstance(native, dict) else None,
        "server_pid": int(spec["game_server_pid"]),
        "ambient_identities": ambient_identities,
        "binary": str(binary),
        "observe_s": float(spec["observe_s"]) if "observe_s" in spec else float(args.observe),
        "warmup_s": float(spec["warmup_s"]) if "warmup_s" in spec else float(args.warmup),
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

    # --- validate + exclusive cell dir + preflight (no children) ---
    try:
        raw_spec = load_spec(spec_path)
        spec = validate_spec(raw_spec)
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

    try:
        pf = preflight(spec, args)
    except (CellError, ValueError, OSError, sr.SampleError, TypeError, json.JSONDecodeError) as exc:
        return _preflight_fail_report(report, cell_dir, str(exc))

    write_json(
        cell_dir / "logs" / "preflight.json",
        {
            "server_pid": pf["server_pid"],
            "server_start_identity": pf["server_sample"].get("start_identity"),
            "checked_server": pf.get("checked_server"),
            "ambient": {
                k: v["sample"].get("start_identity") for k, v in pf["ambient_identities"].items()
            },
            "provenance_status": pf["provenance"].get("status"),
            "binary": pf["binary"],
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
            snapshot = cp.capture(spec["cache_dir"], spec["unpack_root"])
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
        )
        report["launch_started_utc"] = launch_rec["started_utc"]

        launcher_log = cell_dir / "logs" / "launcher.log"
        env = os.environ.copy()
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
        startup["launcher_start_gap_s"] = time.monotonic() - t0
        role_identities["launcher"] = _role_identity(
            launcher.pid, backend=backend, role="launcher"
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

            for line in log_tail.poll():
                meta = _parse_metadata_line(line)
                if meta and metadata is None:
                    metadata = meta
                    run_dir = pathlib.Path(str(meta["run_dir"])) if meta.get("run_dir") else None
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

            if metadata is None and launcher.poll() is not None and not collector_started:
                runner_errors.append('managed launcher produced no identity-bound metadata')
                break

            # After a validated observe-end, keep the collector alive for at
            # least two sampler intervals even if the launcher already exited.
            pad_elapsed = (
                observe_end_mono is not None
                and now >= observe_end_mono + 2.0 * interval
            )
            if (
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
            if meta and frontend_pid is None and meta.get("pid") is not None:
                frontend_pid, frontend_identity = _capture_owned_frontend(
                    meta['pid'], launcher.pid,
                    {*roles.values(), collector.pid if collector else -1}, backend)

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
            collector_stop_mono is None
            and not max_wall_exceeded
            and observe_end_mono is not None
        ):
            runner_errors.append('collector did not reach post-observation stop boundary')
        if launcher_exited_before_pad:
            runner_errors.append(
                'launcher exited before post-observation collector coverage'
            )
            report['launcher_exited_before_pad'] = True
        if max_wall_exceeded:
            runner_errors.append('max_wall_exceeded')
        if any(info.get('orphan_risk') for info in cleanup.values()):
            runner_errors.append('owned child cleanup unresolved')
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

        report["ended_utc"] = _utc()
        write_json(cell_dir / "cell_report.json", report)
        return report

    except Exception as exc:
        report["status"] = "failed_or_unavailable"
        report["error"] = str(exc)
        report["ended_utc"] = _utc()
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
