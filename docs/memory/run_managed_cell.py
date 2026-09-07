#!/usr/bin/env python3
"""One-cell managed orchestration: reviewed launcher + schema-2 process accounting.

Explicit predeclared JSON cell spec only — no name/env discovery, no retries.
Does not start, stop, or signal the game server or ambient helper PIDs.
Does not claim qualification, overhead, or performance acceptance.
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import signal
import subprocess
import sys
import time
from typing import Any, Dict, List, Mapping, Optional, Sequence

_ROOT = pathlib.Path(__file__).resolve().parent
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

import build_provenance as bp  # noqa: E402
import managed_receipt as mr  # noqa: E402
import run_diagnostic as rd  # noqa: E402
import server_resources as sr  # noqa: E402

CLEANUP_WAIT_S = 15.0
SAMPLE_TIMEOUT_S = 2.0
DEFAULT_ACCOUNTING = _ROOT / "process_accounting.py"


class CellError(RuntimeError):
    """Managed-cell configuration or preflight failure."""


def _utc() -> str:
    return mr.utc_now()


def _is_pos_num(value: Any) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and value > 0
        and value == value
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
    if not isinstance(cell_id, str) or not cell_id.strip():
        raise CellError("spec.id must be a non-empty string")
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
        if not isinstance(name, str) or not name.strip():
            raise CellError("ambient helper role names must be non-empty strings")
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
                and val == val
            ):
                raise CellError(f"spec.{key} must be a finite nonnegative number")
    return out


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


def require_argv_consistent_with_spec(spec: Mapping[str, Any], args: argparse.Namespace) -> None:
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


def exclusive_cell_dir(cells_root: pathlib.Path, cell_id: str) -> pathlib.Path:
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
    for key in ("server_identity_path", "host_conditions_path"):
        p = pathlib.Path(spec[key]).resolve(strict=True)
        json.loads(p.read_text())
    server_sample = sr.sample_process(int(spec["game_server_pid"]), timeout=sample_timeout)
    ambient_identities: Dict[str, Any] = {}
    for name, pid in sorted(spec["ambient_helpers"].items()):
        ambient_identities[name] = {
            "pid": pid,
            "sample": sr.sample_process(int(pid), timeout=sample_timeout),
        }
    return {
        "provenance": provenance,
        "server_sample": server_sample,
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
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def _start_identity(pid: int, timeout: float = SAMPLE_TIMEOUT_S) -> Optional[str]:
    try:
        return sr.sample_process(pid, timeout=timeout).get("start_identity")
    except (sr.SampleError, OSError):
        return None


def _terminate_owned(
    proc: Optional[subprocess.Popen],
    *,
    label: str,
    wait_s: float = CLEANUP_WAIT_S,
    frontend_pid: Optional[int] = None,
    frontend_identity: Optional[str] = None,
    launcher_pid: Optional[int] = None,
) -> Dict[str, Any]:
    """Terminate an owned child only. Optional frontend escalation is identity-checked."""
    info: Dict[str, Any] = {"label": label, "signaled": False, "exit_code": None}
    if proc is None:
        return info
    info["pid"] = proc.pid
    if proc.poll() is not None:
        info["exit_code"] = proc.returncode
        return info
    info["signaled"] = True
    try:
        proc.send_signal(signal.SIGTERM)
    except OSError as exc:
        info["error"] = str(exc)
    try:
        proc.wait(timeout=wait_s)
        info["exit_code"] = proc.returncode
        return info
    except subprocess.TimeoutExpired:
        pass
    if (
        frontend_pid
        and launcher_pid
        and frontend_identity
        and label == "launcher"
        and _pid_alive(frontend_pid)
    ):
        current = _start_identity(frontend_pid)
        if current == frontend_identity:
            try:
                os.kill(frontend_pid, signal.SIGTERM)
                info["frontend_sigterm"] = True
            except OSError as exc:
                info["frontend_sigterm_error"] = str(exc)
            deadline = time.monotonic() + wait_s
            while time.monotonic() < deadline and _pid_alive(frontend_pid):
                time.sleep(0.05)
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


def _build_sampler_config(
    spec: Mapping[str, Any], roles: Mapping[str, int], argv: Sequence[str]
) -> Dict[str, Any]:
    return {
        "interval_s": float(spec["sampler_interval_s"]),
        "duration_s_requested": None,
        "duration_mode": "stop_controlled",
        "schema": 2,
        "roles": {k: int(v) for k, v in roles.items()},
        "argv": list(argv),
        "module": str(DEFAULT_ACCOUNTING.resolve()),
        "module_sha256": bp.file_sha256(DEFAULT_ACCOUNTING)
        if DEFAULT_ACCOUNTING.is_file()
        else None,
    }


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
) -> Dict[str, Any]:
    """Run exactly one managed cell. Never retries. Never signals server/ambient."""
    accounting_script = pathlib.Path(accounting_script or DEFAULT_ACCOUNTING)
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
    observe_end_mono: Optional[float] = None

    # --- validate + exclusive cell dir + preflight (no children) ---
    try:
        raw_spec = load_spec(spec_path)
        spec = validate_spec(raw_spec)
        args = parse_diagnostic_argv(spec["diagnostic_argv"])
        require_argv_consistent_with_spec(spec, args)
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
        if isinstance(cid, str) and cid.strip():
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

        launch_path = cell_dir / "launch.json"
        sampler_argv_preview = [sys.executable, str(accounting_script)]
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
            sampler_config=_build_sampler_config(
                spec, sampler_roles_placeholder, sampler_argv_preview
            ),
        )
        report["launch_started_utc"] = launch_rec["started_utc"]

        launcher_log = cell_dir / "logs" / "launcher.log"
        env = os.environ.copy()
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
        role_args = [f"{name}={pid}" for name, pid in sorted(roles.items())]
        collector_argv = [
            sys.executable,
            str(accounting_script),
            *role_args,
            str(sampler_out),
            "--interval",
            str(float(spec["sampler_interval_s"])),
        ]
        report["sampler_argv"] = collector_argv
        report["sampler_roles"] = roles
        t1 = time.monotonic()
        collector_out_fh = (cell_dir / "logs" / "collector.stdout").open("xb")
        try:
            collector = subprocess.Popen(
                collector_argv,
                cwd=str(_ROOT),
                stdout=collector_out_fh,
                stderr=subprocess.STDOUT,
            )
            report["collector_pid"] = collector.pid
            startup["collector_start_gap_s"] = time.monotonic() - t1
            startup["roles"] = roles
        except OSError as exc:
            # Sampler failed to start — record, do not replace, continue to wait launcher.
            collector = None
            collector_exit = 127
            report["sampler_premature_exit"] = 127
            report["sampler_start_error"] = str(exc)
            startup["collector_start_error"] = str(exc)

        log_tail = IncrementalLines(launcher_log)
        qual_tail: Optional[IncrementalLines] = None
        interval = float(spec["sampler_interval_s"])
        observe_s = float(pf["observe_s"])
        warmup_s = float(pf["warmup_s"])
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
                        frontend_pid = int(meta["pid"])
                        frontend_identity = _start_identity(frontend_pid)
                    report["frontend_pid"] = frontend_pid
                    report["frontend_start_identity"] = frontend_identity
                    report["run_dir"] = str(run_dir) if run_dir else None
                    if run_dir is not None:
                        qual_tail = IncrementalLines(run_dir / "samples.qualification.jsonl")
                    started_unix = meta.get("started_unix")
                    if isinstance(started_unix, (int, float)):
                        elapsed_wall = time.time() - float(started_unix)
                        remain = max(0.0, warmup_s + observe_s - elapsed_wall)
                        observe_end_mono = now + remain
                    else:
                        observe_end_mono = now + warmup_s + observe_s

            if qual_tail is not None:
                for qline in qual_tail.poll():
                    qualification_lines.append(qline)

            if (
                not collector_stop_requested
                and observe_end_mono is not None
                and now >= observe_end_mono + 2.0 * interval
                and collector is not None
                and collector.poll() is None
            ):
                collector.send_signal(signal.SIGTERM)
                collector_stop_requested = True
                collector_stop_mono = now
                report["collector_stop_requested_utc"] = _utc()

            if launcher.poll() is not None:
                launcher_exit = launcher.returncode
                if collector is not None and collector.poll() is None:
                    collector.send_signal(signal.SIGTERM)
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
                frontend_pid = int(meta["pid"])
                frontend_identity = _start_identity(frontend_pid)

        if max_wall_exceeded:
            _terminate_owned(collector, label="collector", wait_s=CLEANUP_WAIT_S)
            _terminate_owned(
                launcher,
                label="launcher",
                wait_s=CLEANUP_WAIT_S,
                frontend_pid=frontend_pid,
                frontend_identity=frontend_identity,
                launcher_pid=launcher.pid if launcher else None,
            )
        else:
            if launcher is not None and launcher.poll() is None:
                remaining = max(0.1, wall_deadline - time.monotonic())
                try:
                    launcher.wait(timeout=remaining)
                except subprocess.TimeoutExpired:
                    max_wall_exceeded = True
                    report["max_wall_exceeded"] = True
                    _terminate_owned(collector, label="collector", wait_s=CLEANUP_WAIT_S)
                    _terminate_owned(
                        launcher,
                        label="launcher",
                        wait_s=CLEANUP_WAIT_S,
                        frontend_pid=frontend_pid,
                        frontend_identity=frontend_identity,
                        launcher_pid=launcher.pid if launcher else None,
                    )
            if launcher is not None and launcher.poll() is not None:
                launcher_exit = launcher.returncode
            if collector is not None and collector.poll() is None:
                collector.send_signal(signal.SIGTERM)
            if collector is not None:
                try:
                    collector.wait(timeout=CLEANUP_WAIT_S)
                except subprocess.TimeoutExpired:
                    _terminate_owned(collector, label="collector", wait_s=CLEANUP_WAIT_S)

        if launcher is not None and launcher.poll() is not None:
            launcher_exit = launcher.returncode
        if collector is not None and collector.poll() is not None:
            collector_exit = collector.returncode

        cleanup = {
            "launcher": _terminate_owned(
                launcher if launcher and launcher.poll() is None else None,
                label="launcher",
                frontend_pid=frontend_pid,
                frontend_identity=frontend_identity,
                launcher_pid=launcher.pid if launcher else None,
            ),
            "collector": _terminate_owned(
                collector if collector and collector.poll() is None else None,
                label="collector",
            ),
        }
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
        sampler_result = {
            "exit_code": int(collector_exit) if isinstance(collector_exit, int) else 1,
            "output": str(sampler_out) if sampler_out and sampler_out.is_file() else None,
            "completion": summary.get("completion"),
            "status": summary.get("status"),
            "duration_mode": "stop_controlled",
            "duration_s_requested": None,
        }
        report["sampler_result"] = sampler_result

        receipt_path = cell_dir / "receipt.json"
        if launch_path is not None and launch_path.is_file():
            receipt = mr.complete(
                receipt_path,
                launch_path=launch_path,
                run_dir=str(run_dir) if run_dir is not None else None,
                launcher_exit_code=int(launcher_exit) if isinstance(launcher_exit, int) else 1,
                sampler_result=sampler_result,
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

    except FileExistsError:
        raise
    except Exception as exc:
        report["status"] = "failed_or_unavailable"
        report["error"] = str(exc)
        report["ended_utc"] = _utc()
        _terminate_owned(collector, label="collector")
        _terminate_owned(
            launcher,
            label="launcher",
            frontend_pid=frontend_pid,
            frontend_identity=frontend_identity,
            launcher_pid=launcher.pid if launcher else None,
        )
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
    p.add_argument(
        "--accounting-script",
        type=pathlib.Path,
        default=DEFAULT_ACCOUNTING,
        help="process_accounting.py path (tests may inject a fixture)",
    )
    return p


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = build_cli()
    a = p.parse_args(list(argv) if argv is not None else None)
    try:
        report = run_managed_cell(a.spec, a.cells_root, accounting_script=a.accounting_script)
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
