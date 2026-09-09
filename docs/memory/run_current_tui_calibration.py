#!/usr/bin/env python3
"""Prepare and run exactly one current-source native TUI N1 or N16 diagnostic.

This controller is intentionally a thin adapter around run_managed_cell.py.  It
never builds, starts, stops, or mutates the game server, and it never retries a
cell.  Preflight-only prints a no-launch contract without reserving output.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import signal
import subprocess
import sys
import threading
import time
from typing import Any, Callable, Dict, Mapping, Optional, Sequence

HERE = pathlib.Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import build_provenance as bp  # noqa: E402
import run_managed_cell as rmc  # noqa: E402
import server_resources as sr  # noqa: E402
import heaptrack_capture as hc  # noqa: E402
import validate_direct_owner_capture as vdoc  # noqa: E402

EXPECTED_HOST = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf"
EXPECTED_CLIENT = "3456edc8dabf7b25ada78110ffa56327af9f67a4"
SERVER_PORT = 43594
MEM_AVAILABLE_GUARD_BYTES = 128 * 1024 * 1024
MEM_GUARD_INTERVAL_S = 0.5
WARMUP_S = 120
OBSERVE_S = 600
TEARDOWN_GRACE_S = 60
SUPPORTED_N = (1, 16)
DIRECT_DIAGNOSTIC_HOST = bp.DIRECT_DIAGNOSTIC_HOST
DIRECT_DIAGNOSTIC_CLIENT = bp.DIRECT_DIAGNOSTIC_CLIENT
DIRECT_HOST_SOURCE_DIGEST = bp.DIRECT_HOST_SOURCE_DIGEST
DIRECT_CLIENT_SOURCE_DIGEST = bp.DIRECT_CLIENT_SOURCE_DIGEST
DIRECT_WARMUP_S = 30
DIRECT_OBSERVE_S = 120
DIRECT_MEM_AVAILABLE_BYTES = 256 * 1024 * 1024
DIRECT_FRONTEND_RSS_BYTES = 512 * 1024 * 1024
DIRECT_OUTPUT_BYTES = 64 * 1024 * 1024
DIRECT_FRONTEND_WALL_S = 360
DIRECT_OUTER_WALL_S = 365


class CalibrationError(RuntimeError):
    """A fail-closed configuration, provenance, or execution error."""


def requested_n(args: argparse.Namespace) -> int:
    value = getattr(args, "n", 16)
    if type(value) is not int or value not in SUPPORTED_N:
        raise CalibrationError(f"n must be an exact integer in {SUPPORTED_N}; got {value!r}")
    return value


def sha256(path: pathlib.Path) -> str:
    return bp.file_sha256(path)


def _git(path: pathlib.Path, *args: str) -> str:
    try:
        return subprocess.check_output(["git", "-C", str(path), *args], text=True).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise CalibrationError(f"git query failed for {path}: {exc}") from exc


def check_source(host_checkout: pathlib.Path, expected_host: str, expected_client: str,
                 *, include_untracked: bool = False) -> Dict[str, Any]:
    host_checkout = host_checkout.resolve(strict=True)
    client = host_checkout / "vendor" / "fr-client-rust"
    if not (client / ".git").exists() and not (host_checkout / ".gitmodules").is_file():
        raise CalibrationError("client submodule checkout is missing")
    host_commit = _git(host_checkout, "rev-parse", "HEAD")
    client_commit = _git(client, "rev-parse", "HEAD")
    if host_commit != expected_host:
        raise CalibrationError(f"host commit {host_commit} != expected {expected_host}")
    if client_commit != expected_client:
        raise CalibrationError(f"client commit {client_commit} != expected {expected_client}")
    for label, path in (("host", host_checkout), ("client", client)):
        status_args = ("status", "--porcelain") if include_untracked else (
            "status", "--porcelain", "--untracked-files=no"
        )
        status = _git(path, *status_args)
        if status:
            raise CalibrationError(f"{label} source checkout is dirty")
    return {"host_commit": host_commit, "client_commit": client_commit, "host_clean": True, "client_clean": True}


def _hex_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise CalibrationError(f"server identity has invalid {label}")
    return value


def validate_server_identity(path: pathlib.Path, expected_pid: int, expected_start: str) -> Dict[str, Any]:
    try:
        data = json.loads(path.resolve(strict=True).read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise CalibrationError(f"cannot read server identity: {exc}") from exc
    if not isinstance(data, dict) or data.get("pid") != expected_pid or data.get("start_identity") != expected_start:
        raise CalibrationError("server identity sidecar does not match declared PID/start identity")
    config = data.get("configuration")
    if not isinstance(config, dict) or data.get("port_listen") != SERVER_PORT:
        raise CalibrationError(f"server identity must declare loopback port_listen {SERVER_PORT}")
    if config.get("bind_host") not in ("127.0.0.1", "localhost"):
        raise CalibrationError("server identity must declare loopback bind_host")
    for key in ("config_sha256", "fixture_manifest_sha256", "public_key_sha256"):
        _hex_digest(data.get(key), key)
    for key in ("world_json_sha256", "fixture_manifest_sha256"):
        _hex_digest(config.get(key), f"configuration.{key}")
    return data


def _server_public_environment(server_root: pathlib.Path) -> Dict[str, str]:
    """Bind launch credentials to the declared public server artifact only."""
    root = server_root.resolve(strict=True)
    if not root.is_dir():
        raise CalibrationError("server root must be a directory")
    public_path = root / "server-login-public.json"
    try:
        public = json.loads(public_path.read_text())
        modulus = public["modulus_decimal"]
        exponent = public["exponent_decimal"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as exc:
        raise CalibrationError(f"server public login artifact is unreadable: {exc}") from exc
    if not all(isinstance(value, str) and value for value in (modulus, exponent)):
        raise CalibrationError("server public login artifact is incomplete")
    return {"ENGINE_DIR": str(root), "LOGIN_RSAN": modulus, "LOGIN_RSAE": exponent}


def launch_environment(args: argparse.Namespace, base: Optional[Mapping[str, str]] = None) -> Dict[str, str]:
    env = clean_environment(base, n=requested_n(args),
                            direct_owner_capture=getattr(args, 'direct_owner_capture', False) is True)
    env.update({"LIVE": "1", "BOT_TARGET": "local", "RS2B0T": str(args.rs2b0t.resolve()),
                "NAV_PACK": str(args.nav_pack.resolve()), "NAV_FLAGS": str(args.nav_flags.resolve())})
    env.update(_server_public_environment(args.server_root))
    return env


def validate_feature_contract(manifest_path: pathlib.Path, *, direct_owner_capture=False) -> None:
    try:
        features = json.loads(manifest_path.resolve(strict=True).read_text())["features"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as exc:
        raise CalibrationError(f"build feature contract is unreadable: {exc}") from exc
    expected = bp.DIRECT_FEATURES if direct_owner_capture else "memory-profile-no-alloc"
    if (not isinstance(features, dict) or features.get("requested") != expected
            or features.get("locked") is not True or features.get("allocation_counting") is not False
            or (direct_owner_capture and features.get('allocator') != 'std::alloc::System')
            or (direct_owner_capture and features.get('snapshot_dedup') is not False)
            or "snapshot-dedup" in str(features.get("enabled", ""))
            or "snapshot-dedup" in str(features.get("requested", ""))):
        raise CalibrationError("build manifest is not the locked memory-profile-no-alloc/System feature contract")


def validate_server_artifact_hashes(server_root: pathlib.Path, identity: Mapping[str, Any]) -> None:
    root = server_root.resolve(strict=True)
    config = identity["configuration"]
    bindings = (("config_sha256", root / "data/config/world.json"),
                ("fixture_manifest_sha256", root / "concord-fixture-manifest.json"),
                ("public_key_sha256", root / "data/config/public.pem"))
    for key, path in bindings:
        expected = identity[key]
        if not path.is_file() or sha256(path) != expected:
            raise CalibrationError(f"server {key} is not bound to the declared artifact")
    if config["world_json_sha256"] != identity["config_sha256"]:
        raise CalibrationError("server configuration world hash is inconsistent")


def validate_live_server(args: argparse.Namespace) -> Dict[str, Any]:
    """Re-sample the declared server before reserving any run output."""
    identity = validate_server_identity(args.server_identity, args.server_pid, args.server_start_identity)
    try:
        sample = sr.sample_process(args.server_pid, timeout=5.0)
    except (sr.SampleError, OSError, ValueError, TypeError) as exc:
        raise CalibrationError(f"live server identity sample failed: {exc}") from exc
    if sample.get("start_identity") != identity["start_identity"]:
        raise CalibrationError("live server identity does not match declared PID/start identity")
    return sample


def diagnostic_argv(binary: pathlib.Path, manifest: pathlib.Path, role: str, n: int = 16,
                    heaptrack_output: Optional[pathlib.Path] = None,
                    direct_owner_capture: bool = False,
                    run_dir: Optional[pathlib.Path] = None,
                    frontend_handoff: Optional[pathlib.Path] = None) -> list[str]:
    warmup = DIRECT_WARMUP_S if direct_owner_capture else WARMUP_S
    observe = DIRECT_OBSERVE_S if direct_owner_capture else OBSERVE_S
    argv = [
        "tui", str(n), "active",
        "--binary", str(binary),
        "--build-manifest", str(manifest),
        "--build-role", role,
        "--no-diagnostics", "--sustain",
        "--warmup", str(warmup), "--observe", str(observe),
    ]
    if heaptrack_output is not None:
        argv.extend(["--heaptrack-output", str(heaptrack_output.resolve())])
    if direct_owner_capture:
        if run_dir is None or frontend_handoff is None:
            raise CalibrationError('direct owner argv requires reserved run and handoff paths')
        argv.extend(['--direct-owner-capture', '--run-dir', str(run_dir.resolve()),
                     '--frontend-handoff', str(frontend_handoff.resolve())])
    return argv


def clean_environment(base: Optional[Mapping[str, str]] = None, n: int = 16,
                      direct_owner_capture: bool = False) -> Dict[str, str]:
    env = dict(base if base is not None else os.environ)
    forced_off = (
        "BOT_DEBUG", "BOT_CPU", "BOT_SCHEDULING_PROFILE", "BOT_RESPONSIVENESS_PROFILE",
        "BOT_RESPONSIVENESS_FINE", "BOT_RENDER_PROFILE", "BOT_GPU_COMPLETION_PROFILE",
        "MallocStackLogging", "MallocStackLoggingNoCompact", "BOT_MEMORY_FAILURE_CAPTURE",
        "BOT_MEMORY_OWNER_CAPTURE",
    )
    for key in forced_off:
        env.pop(key, None)
    warmup = DIRECT_WARMUP_S if direct_owner_capture else WARMUP_S
    observe = DIRECT_OBSERVE_S if direct_owner_capture else OBSERVE_S
    env.update(
        BOT_MEMORY_N=str(n), BOT_MEMORY_WORKLOAD="active", BOT_MEMORY_WARMUP_S=str(warmup),
        BOT_MEMORY_OBSERVE_S=str(observe), BOT_MEMORY_DIAGNOSTICS="0",
        BOT_MEMORY_SUSTAIN="1", BOT_SCHEDULING_PROFILE="0", BOT_RESPONSIVENESS_PROFILE="0",
        BOT_RESPONSIVENESS_FINE="0", BOT_RENDER_PROFILE="0", BOT_GPU_COMPLETION_PROFILE="0",
    )
    if direct_owner_capture:
        env['BOT_MEMORY_OWNER_CAPTURE'] = '1'
    return env


def build_spec(args: argparse.Namespace, source: Mapping[str, Any]) -> Dict[str, Any]:
    output = args.output.resolve()
    cell_id = output.stem
    if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", cell_id) is None:
        raise CalibrationError("output basename must produce a safe cell id")
    heaptrack_output = getattr(args, "heaptrack_output", None)
    direct = getattr(args, 'direct_owner_capture', False) is True
    if direct and (requested_n(args) != 1 or heaptrack_output is not None):
        raise CalibrationError('direct owner capture requires N1 and excludes Heaptrack')
    release_contract = getattr(args, 'release_contract', None)
    if direct and release_contract is None:
        raise CalibrationError('direct owner capture requires an explicit root release contract')
    release_contract_path = (
        pathlib.Path(release_contract).resolve() if release_contract is not None else None
    )
    if heaptrack_output is not None:
        if requested_n(args) != 1:
            raise CalibrationError("Heaptrack capture requires N1")
        hc.verify_preload()
        heaptrack_output = pathlib.Path(heaptrack_output).resolve()
        if heaptrack_output.exists() or heaptrack_output.is_symlink():
            raise CalibrationError("Heaptrack output must be a unique unused path")
    spec_path = output.with_suffix(output.suffix + '.spec.json')
    cells_root = output.with_suffix(output.suffix + '.cells')
    cell_dir = cells_root / cell_id
    run_dir = cell_dir / 'frontend-run'
    handoff = cell_dir / 'frontend-handoff.json'
    diag = diagnostic_argv(args.binary.resolve(), args.build_manifest.resolve(), args.build_role,
                          n=requested_n(args), heaptrack_output=heaptrack_output,
                          direct_owner_capture=direct, run_dir=run_dir,
                          frontend_handoff=handoff)
    launcher = [sys.executable, str(HERE / "run_diagnostic.py"), *diag]
    timing: Dict[str, Any] = {"max_wall_s": DIRECT_OUTER_WALL_S if direct else 960}
    if heaptrack_output is not None:
        # Preserve the 960-second frontend lifecycle; analysis is post-exit.
        timing.update(live_max_wall_s=960, analysis_windows_s=[180, 180], max_wall_s=1320)
    spec = {
        "id": cell_id, "index": 1, "kind": "diagnostic", "frontend": "tui",
        "build_role": args.build_role, "binary": str(args.binary.resolve()),
        "build_manifest": str(args.build_manifest.resolve()), "n": requested_n(args),
        "server_identity_path": str(args.server_identity.resolve()),
        "host_conditions_path": str(args.host_conditions.resolve()),
        "nav_pack": str(args.nav_pack.resolve()), "nav_flags": str(args.nav_flags.resolve()),
        "catalog_path": str(args.catalog.resolve()), "launcher_argv": launcher,
        "diagnostic_argv": diag, "game_server_pid": args.server_pid,
        "ambient_helpers": {"ssh_parent": args.ssh_parent_pid},
        "sampler_interval_s": 0.5,
        "warmup_s": DIRECT_WARMUP_S if direct else WARMUP_S,
        "observe_s": DIRECT_OBSERVE_S if direct else OBSERVE_S,
        "teardown_grace_s": TEARDOWN_GRACE_S,
        **timing,
        "process_backend": "system", "requested_backend": "none",
        "memory_guard": {"metric": "MemAvailable", "limit_bytes": DIRECT_MEM_AVAILABLE_BYTES if direct else MEM_AVAILABLE_GUARD_BYTES, "poll_interval_s": MEM_GUARD_INTERVAL_S},
        "source": dict(source), "server_root": str(args.server_root.resolve()),
 "rs2b0t": str(args.rs2b0t.resolve()), "performance_acceptance": False,
 "cache_dir": str(args.cache_dir.resolve()), "unpack_root": str(args.unpack_root.resolve()),
 "heaptrack": ({"output": str(heaptrack_output), "preload": hc.verify_preload()}
               if heaptrack_output is not None else None),
    }
    if direct:
        admissions = {
            'conflict': str(args.conflict_receipt.resolve()),
            'account': str(args.account_admission.resolve()),
            'population': str(args.population_admission.resolve()),
            'cache': str(args.cache_admission.resolve()),
            'server_health': str(args.server_health_receipt.resolve()),
        }
        spec['capture_contract'] = {
            'mode': 'direct-owner-v1', 'cell_dir': str(cell_dir.resolve()),
            'run_dir': str(run_dir.resolve()), 'frontend_handoff_path': str(handoff.resolve()),
            'owned_output_paths': [str(path.resolve()) for path in
                                   (output, spec_path, cell_dir, run_dir)],
            'warmup_s': DIRECT_WARMUP_S, 'observe_s': DIRECT_OBSERVE_S,
            'teardown_grace_s': TEARDOWN_GRACE_S, 'guard_interval_s': 0.5,
            'handoff_deadline_s': 5.0, 'mem_available_floor_bytes': DIRECT_MEM_AVAILABLE_BYTES,
            'frontend_rss_limit_bytes': DIRECT_FRONTEND_RSS_BYTES,
            'owned_output_limit_bytes': DIRECT_OUTPUT_BYTES,
            'frontend_wall_limit_s': DIRECT_FRONTEND_WALL_S,
            'outer_wall_limit_s': DIRECT_OUTER_WALL_S,
            'source_lineage': source.get('source_lineage'),
            'release_contract': str(release_contract_path),
            'admission_receipts': admissions,
        }
    return spec


def validate_inputs(args: argparse.Namespace) -> Dict[str, Any]:
    direct = getattr(args, 'direct_owner_capture', False) is True
    expected_host = DIRECT_DIAGNOSTIC_HOST if direct else args.expected_host_commit
    expected_client = DIRECT_DIAGNOSTIC_CLIENT if direct else args.expected_client_commit
    source = check_source(
        args.host_checkout, expected_host, expected_client, include_untracked=direct
    )
    identity = validate_live_server(args)
    for label, path in (("binary", args.binary), ("build manifest", args.build_manifest), ("host conditions", args.host_conditions), ("nav pack", args.nav_pack), ("nav flags", args.nav_flags), ("catalog", args.catalog), ("server root", args.server_root), ("rs2b0t", args.rs2b0t)):
        if not path.exists():
            raise CalibrationError(f"{label} does not exist: {path}")
    verifier = bp.verify_direct_owner_build if direct else bp.verify_build
    provenance = verifier(args.build_manifest, args.build_role, "tui", args.binary,
                          args.nav_pack, args.nav_flags, args.catalog)
    validate_feature_contract(args.build_manifest, direct_owner_capture=direct)
    if direct:
        source.update(
            host_sources_sha256=bp.source_digest(args.host_checkout),
            client_sources_sha256=bp.source_digest(args.host_checkout / 'vendor' / 'fr-client-rust'),
            source_lineage=json.loads(args.build_manifest.read_text())['candidate']['source_lineage'],
        )
        if (source['host_sources_sha256'] != DIRECT_HOST_SOURCE_DIGEST
                or source['client_sources_sha256'] != DIRECT_CLIENT_SOURCE_DIGEST):
            raise CalibrationError('direct owner source digest differs from fresh build')
        for label in ('release_contract', 'conflict_receipt', 'account_admission',
                      'population_admission', 'cache_admission', 'server_health_receipt'):
            path = getattr(args, label, None)
            if path is None or not pathlib.Path(path).resolve(strict=True).is_file():
                raise CalibrationError('direct owner requires root admission receipt: ' + label)
    if getattr(args, "heaptrack_output", None) is not None:
        hc.verify_preload()
    declared_identity = validate_server_identity(args.server_identity, args.server_pid, args.server_start_identity)
    validate_server_artifact_hashes(args.server_root, declared_identity)
    return source


def read_mem_available() -> Optional[int]:
    try:
        for line in pathlib.Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        return None
    return None


class MemoryGuard:
    def __init__(self, on_trigger: Callable[[str], None], reader: Callable[[], Optional[int]] = read_mem_available):
        self.on_trigger, self.reader = on_trigger, reader
        self.stop = threading.Event()
        self.triggered: Optional[str] = None
        self.thread = threading.Thread(target=self._run, name="memavailable-guard", daemon=True)

    def _run(self) -> None:
        while not self.stop.wait(MEM_GUARD_INTERVAL_S):
            available = self.reader()
            if available is None or available < MEM_AVAILABLE_GUARD_BYTES:
                self.triggered = ("MemAvailable unavailable" if available is None else
                                  f"MemAvailable {available} < {MEM_AVAILABLE_GUARD_BYTES}")
                self.on_trigger(self.triggered)
                return

    def start(self) -> None: self.thread.start()
    def close(self) -> None:
        self.stop.set()
        self.thread.join(timeout=2)


def _exclusive_json(path: pathlib.Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, sort_keys=True)
        handle.write("\n")


def run(args: argparse.Namespace, spec: Dict[str, Any]) -> int:
    output = args.output.resolve()
    spec_path = output.with_suffix(output.suffix + ".spec.json")
    cells_root = output.with_suffix(output.suffix + ".cells")
    n = requested_n(args)
    if output.exists() or spec_path.exists() or cells_root.exists():
        raise CalibrationError("refusing existing output/spec/cell path; choose a unique output")
    _exclusive_json(spec_path, spec)
    direct = getattr(args, 'direct_owner_capture', False) is True
    triggered: Dict[str, str] = {}
    def cleanup(reason: str) -> None:
        triggered["reason"] = reason
        if hasattr(signal, "SIGUSR1"):
            os.kill(os.getpid(), signal.SIGUSR1)
    guard = None if direct else MemoryGuard(cleanup)
    report: Dict[str, Any] = {"status": "failed_or_unavailable", "launched": "unknown", "n": n,
                              "attempts": 1, "execution_attempted": True}
    previous_handler = signal.getsignal(signal.SIGUSR1) if hasattr(signal, "SIGUSR1") else None
    def abort_from_guard(signum: int, frame: Any) -> None:
        raise RuntimeError("predeclared host memory guard: owned managed cell cancelled")
    if guard is not None and hasattr(signal, "SIGUSR1"):
        signal.signal(signal.SIGUSR1, abort_from_guard)
    launch_env = launch_environment(args, base=os.environ.copy())
    if guard is not None:
        guard.start()
    try:
        report = rmc.run_managed_cell(spec_path, cells_root, cwd=args.host_checkout,
                                      environment=launch_env)
    except Exception as exc:  # preserve a durable failed artifact and honest attempt state
        report["error"] = str(exc)
    finally:
        if guard is not None:
            guard.close()
        if guard is not None and hasattr(signal, "SIGUSR1"):
            signal.signal(signal.SIGUSR1, previous_handler)
    if triggered:
        report["status"] = "failed_or_unavailable"
        report["memory_guard"] = {"status": "triggered", "reason": triggered["reason"], "cleanup": "owned runner cleanup requested"}
    elif guard is not None:
        report["memory_guard"] = {"status": "not_triggered", "limit_bytes": MEM_AVAILABLE_GUARD_BYTES, "poll_interval_s": MEM_GUARD_INTERVAL_S}
    else:
        report['memory_guard'] = {'status': 'managed_direct_guard',
                                  'limit_bytes': DIRECT_MEM_AVAILABLE_BYTES,
                                  'poll_interval_s': MEM_GUARD_INTERVAL_S}
    if direct:
        try:
            source_post = check_source(
                args.host_checkout, DIRECT_DIAGNOSTIC_HOST, DIRECT_DIAGNOSTIC_CLIENT,
                include_untracked=True,
            )
            source_post.update(
                host_sources_sha256=bp.source_digest(args.host_checkout),
                client_sources_sha256=bp.source_digest(args.host_checkout / 'vendor' / 'fr-client-rust'),
            )
            if (source_post['host_sources_sha256'] != DIRECT_HOST_SOURCE_DIGEST
                    or source_post['client_sources_sha256'] != DIRECT_CLIENT_SOURCE_DIGEST):
                raise CalibrationError('direct owner source changed during managed cell')
            report['source_postrun'] = source_post
            cell_dir = pathlib.Path(report['cell_dir'])
            receipt = json.loads((cell_dir / 'receipt.json').read_text())
            owner_path = pathlib.Path(receipt['run_dir']) / 'samples.owners.jsonl'
            owner_sha = receipt['raw_hashes']['samples.owners.jsonl']
            report['owner_protocol'] = vdoc.validate_file(owner_path, owner_sha)
            if report['owner_protocol'].get('protocol_complete') is not True:
                raise CalibrationError('direct owner protocol incomplete')
        except (CalibrationError, ValueError, OSError, KeyError, TypeError,
                json.JSONDecodeError) as exc:
            report['status'] = 'failed_or_unavailable'
            report['direct_owner_error'] = str(exc)
    report["n"] = n
    report["performance_acceptance"] = False
    _exclusive_json(output, report)
    return 0 if report.get("status") == "completed" and not triggered else 1


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--n", type=int, choices=SUPPORTED_N, default=16)
    p.add_argument("--host-checkout", type=pathlib.Path, required=True)
    p.add_argument("--binary", type=pathlib.Path, required=True)
    p.add_argument("--build-manifest", type=pathlib.Path, required=True)
    p.add_argument("--build-role", choices=("reference", "candidate"), required=True)
    p.add_argument("--server-root", type=pathlib.Path, required=True)
    p.add_argument("--rs2b0t", type=pathlib.Path, required=True)
    p.add_argument("--cache-dir", type=pathlib.Path, required=True)
    p.add_argument("--unpack-root", type=pathlib.Path, required=True)
    p.add_argument("--nav-pack", type=pathlib.Path, required=True)
    p.add_argument("--nav-flags", type=pathlib.Path, required=True)
    p.add_argument("--catalog", type=pathlib.Path, required=True)
    p.add_argument("--host-conditions", type=pathlib.Path, required=True)
    p.add_argument("--server-identity", type=pathlib.Path, required=True)
    p.add_argument("--server-pid", type=int, required=True)
    p.add_argument("--server-start-identity", required=True)
    p.add_argument("--ssh-parent-pid", type=int, required=True)
    p.add_argument("--expected-host-commit", default=EXPECTED_HOST)
    p.add_argument("--expected-client-commit", default=EXPECTED_CLIENT)
    p.add_argument("--output", type=pathlib.Path, required=True)
    p.add_argument("--heaptrack-output", type=pathlib.Path,
                   help="unique direct Heaptrack output directory for N1")
    p.add_argument("--direct-owner-capture", action="store_true")
    p.add_argument("--conflict-receipt", type=pathlib.Path)
    p.add_argument("--account-admission", type=pathlib.Path)
    p.add_argument("--population-admission", type=pathlib.Path)
    p.add_argument("--cache-admission", type=pathlib.Path)
    p.add_argument("--server-health-receipt", type=pathlib.Path)
    p.add_argument("--release-contract", type=pathlib.Path)
    p.add_argument("--preflight-only", action="store_true")
    return p


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = parser().parse_args(argv)
    try:
        source = validate_inputs(args)
        spec = build_spec(args, source)
        if args.preflight_only:
            print(json.dumps({"status": "preflight_only", "launched": False, "attempts": 0, "spec": spec, "performance_acceptance": False}, sort_keys=True))
            return 0
        return run(args, spec)
    except (CalibrationError, OSError, ValueError, TypeError, json.JSONDecodeError) as exc:
        print(json.dumps({"status": "preflight_failed", "launched": False, "attempts": 0, "error": str(exc), "performance_acceptance": False}), file=sys.stderr)
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
