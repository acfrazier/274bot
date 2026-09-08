#!/usr/bin/env python3
"""Prepare and run exactly one current-source native TUI N16 diagnostic.

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

EXPECTED_HOST = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf"
EXPECTED_CLIENT = "3456edc8dabf7b25ada78110ffa56327af9f67a4"
SERVER_PORT = 43594
MEM_AVAILABLE_GUARD_BYTES = 128 * 1024 * 1024
MEM_GUARD_INTERVAL_S = 0.5
WARMUP_S = 120
OBSERVE_S = 600
TEARDOWN_GRACE_S = 60


class CalibrationError(RuntimeError):
    """A fail-closed configuration, provenance, or execution error."""


def sha256(path: pathlib.Path) -> str:
    return bp.file_sha256(path)


def _git(path: pathlib.Path, *args: str) -> str:
    try:
        return subprocess.check_output(["git", "-C", str(path), *args], text=True).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise CalibrationError(f"git query failed for {path}: {exc}") from exc


def check_source(host_checkout: pathlib.Path, expected_host: str, expected_client: str) -> Dict[str, Any]:
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
        status = _git(path, "status", "--porcelain", "--untracked-files=no")
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
    env = clean_environment(base)
    env.update({"LIVE": "1", "BOT_TARGET": "local", "RS2B0T": str(args.rs2b0t.resolve()),
                "NAV_PACK": str(args.nav_pack.resolve()), "NAV_FLAGS": str(args.nav_flags.resolve())})
    env.update(_server_public_environment(args.server_root))
    return env


def validate_feature_contract(manifest_path: pathlib.Path) -> None:
    try:
        features = json.loads(manifest_path.resolve(strict=True).read_text())["features"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as exc:
        raise CalibrationError(f"build feature contract is unreadable: {exc}") from exc
    if (not isinstance(features, dict) or features.get("requested") != "memory-profile-no-alloc"
            or features.get("locked") is not True or features.get("allocation_counting") is not False
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


def diagnostic_argv(binary: pathlib.Path, manifest: pathlib.Path, role: str) -> list[str]:
    return [
        "tui", "16", "active",
        "--binary", str(binary),
        "--build-manifest", str(manifest),
        "--build-role", role,
        "--no-diagnostics", "--sustain",
        "--warmup", str(WARMUP_S), "--observe", str(OBSERVE_S),
    ]


def clean_environment(base: Optional[Mapping[str, str]] = None) -> Dict[str, str]:
    env = dict(base if base is not None else os.environ)
    forced_off = (
        "BOT_DEBUG", "BOT_CPU", "BOT_SCHEDULING_PROFILE", "BOT_RESPONSIVENESS_PROFILE",
        "BOT_RESPONSIVENESS_FINE", "BOT_RENDER_PROFILE", "BOT_GPU_COMPLETION_PROFILE",
        "MallocStackLogging", "MallocStackLoggingNoCompact", "BOT_MEMORY_FAILURE_CAPTURE",
    )
    for key in forced_off:
        env.pop(key, None)
    env.update(
        BOT_MEMORY_N="16", BOT_MEMORY_WORKLOAD="active", BOT_MEMORY_WARMUP_S=str(WARMUP_S),
        BOT_MEMORY_OBSERVE_S=str(OBSERVE_S), BOT_MEMORY_DIAGNOSTICS="0",
        BOT_MEMORY_SUSTAIN="1", BOT_SCHEDULING_PROFILE="0", BOT_RESPONSIVENESS_PROFILE="0",
        BOT_RESPONSIVENESS_FINE="0", BOT_RENDER_PROFILE="0", BOT_GPU_COMPLETION_PROFILE="0",
    )
    return env


def build_spec(args: argparse.Namespace, source: Mapping[str, Any]) -> Dict[str, Any]:
    output = args.output.resolve()
    cell_id = output.stem
    if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", cell_id) is None:
        raise CalibrationError("output basename must produce a safe cell id")
    diag = diagnostic_argv(args.binary.resolve(), args.build_manifest.resolve(), args.build_role)
    launcher = [sys.executable, str(HERE / "run_diagnostic.py"), *diag]
    return {
        "id": cell_id, "index": 1, "kind": "diagnostic", "frontend": "tui",
        "build_role": args.build_role, "binary": str(args.binary.resolve()),
        "build_manifest": str(args.build_manifest.resolve()),
        "server_identity_path": str(args.server_identity.resolve()),
        "host_conditions_path": str(args.host_conditions.resolve()),
        "nav_pack": str(args.nav_pack.resolve()), "nav_flags": str(args.nav_flags.resolve()),
        "catalog_path": str(args.catalog.resolve()), "launcher_argv": launcher,
        "diagnostic_argv": diag, "game_server_pid": args.server_pid,
        "ambient_helpers": {"ssh_parent": args.ssh_parent_pid},
        "sampler_interval_s": 0.5, "warmup_s": WARMUP_S, "observe_s": OBSERVE_S,
        "teardown_grace_s": TEARDOWN_GRACE_S, "max_wall_s": WARMUP_S + OBSERVE_S + TEARDOWN_GRACE_S + 180,
        "process_backend": "system", "requested_backend": "none",
        "memory_guard": {"metric": "MemAvailable", "limit_bytes": MEM_AVAILABLE_GUARD_BYTES, "poll_interval_s": MEM_GUARD_INTERVAL_S},
        "source": dict(source), "server_root": str(args.server_root.resolve()),
 "rs2b0t": str(args.rs2b0t.resolve()), "performance_acceptance": False,
 "cache_dir": str(args.cache_dir.resolve()), "unpack_root": str(args.unpack_root.resolve()),
    }


def validate_inputs(args: argparse.Namespace) -> Dict[str, Any]:
    source = check_source(args.host_checkout, args.expected_host_commit, args.expected_client_commit)
    identity = validate_live_server(args)
    for label, path in (("binary", args.binary), ("build manifest", args.build_manifest), ("host conditions", args.host_conditions), ("nav pack", args.nav_pack), ("nav flags", args.nav_flags), ("catalog", args.catalog), ("server root", args.server_root), ("rs2b0t", args.rs2b0t)):
        if not path.exists():
            raise CalibrationError(f"{label} does not exist: {path}")
    # This is the same reviewed manifest verifier used by run_managed_cell.
    bp.verify_build(args.build_manifest, args.build_role, "tui", args.binary, args.nav_pack, args.nav_flags, args.catalog)
    validate_feature_contract(args.build_manifest)
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
    if output.exists() or spec_path.exists() or cells_root.exists():
        raise CalibrationError("refusing existing output/spec/cell path; choose a unique output")
    _exclusive_json(spec_path, spec)
    triggered: Dict[str, str] = {}
    def cleanup(reason: str) -> None:
        triggered["reason"] = reason
        if hasattr(signal, "SIGUSR1"):
            os.kill(os.getpid(), signal.SIGUSR1)
    guard = MemoryGuard(cleanup)
    report: Dict[str, Any] = {"status": "failed_or_unavailable", "launched": "unknown",
                              "attempts": 1, "execution_attempted": True}
    previous_handler = signal.getsignal(signal.SIGUSR1) if hasattr(signal, "SIGUSR1") else None
    def abort_from_guard(signum: int, frame: Any) -> None:
        raise RuntimeError("predeclared host memory guard: owned managed cell cancelled")
    if hasattr(signal, "SIGUSR1"):
        signal.signal(signal.SIGUSR1, abort_from_guard)
    previous_env = os.environ.copy()
    launch_env = launch_environment(args, base=previous_env)
    os.environ.clear()
    os.environ.update(launch_env)
    guard.start()
    try:
        report = rmc.run_managed_cell(spec_path, cells_root, cwd=args.host_checkout)
    except Exception as exc:  # preserve a durable failed artifact and honest attempt state
        report["error"] = str(exc)
    finally:
        guard.close()
        os.environ.clear()
        os.environ.update(previous_env)
        if hasattr(signal, "SIGUSR1"):
            signal.signal(signal.SIGUSR1, previous_handler)
    if triggered:
        report["status"] = "failed_or_unavailable"
        report["memory_guard"] = {"status": "triggered", "reason": triggered["reason"], "cleanup": "owned runner cleanup requested"}
    else:
        report["memory_guard"] = {"status": "not_triggered", "limit_bytes": MEM_AVAILABLE_GUARD_BYTES, "poll_interval_s": MEM_GUARD_INTERVAL_S}
    report["performance_acceptance"] = False
    _exclusive_json(output, report)
    return 0 if report.get("status") == "completed" and not triggered else 1


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
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
