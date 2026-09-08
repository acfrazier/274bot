#!/usr/bin/env python3
"""Artifact-backed matched evidence reader.

Binds explicit receipt paths → canonical run metadata/raw hashes → named
manifest binary (recomputed SHA-256) → independently recomputed
``qualify_control.qualify`` and ``reference_metrics.analyze_run(qualify=True)``.

Missing, null, or unverifiable values are unavailable — never equality.
Does not claim paired performance acceptance, invent overhead proofs, or
relabel caller ``qualified`` / ``overhead`` labels as evidence.
"""
from __future__ import annotations

import hashlib
import contextlib
import datetime
import io
import json
import math
import pathlib
import sys
from typing import Any, Optional

ROOT = pathlib.Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import qualify_control as qc  # noqa: E402
import reference_metrics as rm  # noqa: E402
import run_diagnostic as rd  # noqa: E402
import build_provenance as bp  # noqa: E402
import managed_resource_binding as mrb  # noqa: E402

PASS_MEANS = (
    "artifact-bound matched evidence reader only; "
    "not final performance or lifecycle acceptance"
)

# Strict equality-checked configuration (both sides must present + equal).
# Instrumentation flags are match keys: absent must not default to False.
MATCH_KEY_FIELDS = (
    "frontend",
    "n",
    "workload",
    "render_policy",
    "render_policy_requested",
    "single_renderer",
    "diagnostic_sidecar",
    "allocation_counting",
    "scheduling_profile",
    "responsiveness_profile",
    "responsiveness_fine",
    "render_profile",
    "gpu_completion_profile",
    "stack_logging",
    "sustain",
    "terminal",
    "terminal_size",
    "nav_pack_sha256",
    "nav_flags_sha256",
    "renderer_settings",
    "cache_settings",
    "catalog_sha256",
    "feature_flags",
    "allocator_provenance",
    "client_sources_sha256",
    "failure_capture",
    "nav_captures",
    "cpu_fallback",
    "requested_backend",
    "server_start_identity",
    "server_port_listen",
    "server_configuration",
    "sampler_interval_s",
    "sampler_duration_mode",
    "sampler_duration_s_requested",
    "host_conditions",
)

# Present and role-correct; may differ across reference vs candidate.
SIDE_PROVENANCE_FIELDS = (
    "role",
    "binary_path",
    "binary_sha256",
    "manifest_binary_key",
    "manifest_build_commit",
    "manifest_sources_sha256_pre",
    "manifest_sources_sha256_post",
    "manifest_sources_sha256",
    "manifest_sources_stable",
    "manifest_branch",
    "client_commit",
    "client_sources_sha256",
    "host_commit_checkout",  # current checkout label only; not build authority
)

REQUIRED_RUN_FILES = (
    "metadata.json",
    "samples.jsonl",
    "samples.qualification.jsonl",
)

REQUIRED_RECEIPT_FIELDS = (
    "id",
    "index",
    "kind",
    "run_dir",
    "exit_code",
    "binary",
    "effective_cli",
    "started_utc",
    "ended_utc",
)

REQUIRED_METADATA_FIELDS = (
    "run_dir",
    "exit_code",
    "binary",
    "binary_sha256",
    "frontend",
    "n",
    "workload",
    "warmup_s",
    "observe_s",
    "started_unix",
    "ended_unix",
    "pid",
)

_MANIFEST_BINARY_KEYS = {
    ("reference", "tui"): "control_tui_play",
    ("control", "tui"): "control_tui_play",
    ("candidate", "tui"): "candidate_tui_play",
    ("reference", "panel"): "control_panel_play",
    ("control", "panel"): "control_panel_play",
    ("candidate", "panel"): "candidate_panel_play",
}

_ROLE_MANIFEST_SIDE = {
    "reference": "control",
    "control": "control",
    "candidate": "candidate",
}

_FLAG_TO_META = {
    "--sustain": ("sustain", True),
    "--no-sustain": ("sustain", False),
    "--scheduling-profile": ("scheduling_profile", True),
    "--render-profile": ("render_profile", True),
    "--responsiveness-profile": ("responsiveness_profile", True),
    "--responsiveness-fine": ("responsiveness_fine", True),
    "--gpu-completion-profile": ("gpu_completion_profile", True),
    "--diagnostics": ("diagnostic_sidecar", True),
    "--no-diagnostics": ("diagnostic_sidecar", False),
    "--single-renderer": ("single_renderer", True),
    "--stack-logging": ("stack_logging", True),
    "--allocation-counting": ("allocation_counting", True),
    "--failure-capture": ("failure_capture", True),
    "--nav-captures": ("nav_captures", True),
}


def _unavailable(reason: str, **extra: Any) -> dict:
    out = {
        "status": "unavailable",
        "reason": reason,
        "pass_means": PASS_MEANS,
        "final_acceptance_claim": False,
        "binding_ok": False,
        "pair_eligible": False,
    }
    out.update(extra)
    return out


def _is_missing(value: Any) -> bool:
    if value is None:
        return True
    if value == "":
        return True
    if value == {} or value == []:
        return True
    return False


def _deep_missing(value: Any) -> bool:
    """True if value is missing or any nested dict/list leaf is null/empty."""
    if _is_missing(value):
        return True
    if isinstance(value, dict):
        if not value:
            return True
        return any(_deep_missing(v) for v in value.values())
    if isinstance(value, list):
        if not value:
            return True
        return any(_deep_missing(v) for v in value)
    return False


def _native_windows_conditions_recognized(value: Any) -> bool:
    """Treat the native marker as a distinct, fail-closed schema."""
    return isinstance(value, dict) and "native_preflight" in value


def _native_windows_conditions_complete(value: Any) -> bool:
    """Accept only the bounded native Windows observation shape."""
    if not isinstance(value, dict):
        return False
    if type(value.get("platform")) is not str or not value["platform"]:
        return False
    if type(value.get("user")) is not str or not value["user"]:
        return False
    if type(value.get("purpose")) is not str or not value["purpose"]:
        return False
    for field in ("builds_stopped_before_run", "terminal_transport_expected", "panel_render_attribution"):
        if type(value.get(field)) is not bool:
            return False
    pre = value.get("native_preflight")
    if not isinstance(pre, dict):
        return False
    if type(pre.get("adapter")) is not str or not pre["adapter"]:
        return False
    if type(pre.get("utc")) is not str or not pre["utc"]:
        return False
    if type(pre.get("consoleSessionId")) is not int:
        return False
    vm = pre.get("vm")
    if not isinstance(vm, dict):
        return False
    if type(vm.get("Name")) is not str or not vm["Name"]:
        return False
    if type(vm.get("State")) is not str or not vm["State"]:
        return False
    if type(vm.get("MemoryAssigned")) is not int or vm["MemoryAssigned"] < 0:
        return False
    services = pre.get("quietServices")
    if not isinstance(services, list) or not services:
        return False
    for service in services:
        if not isinstance(service, dict):
            return False
        for field in ("Name", "Status"):
            if type(service.get(field)) is not str or not service[field]:
                return False
    drivers = pre.get("drivers")
    if not isinstance(drivers, list) or not drivers:
        return False
    for driver in drivers:
        if not isinstance(driver, dict):
            return False
        for field in ("Name", "DriverVersion", "PNPDeviceID"):
            if type(driver.get(field)) is not str or not driver[field]:
                return False
    sessions = pre.get("sessions")
    if not isinstance(sessions, list) or not sessions or any(
        type(session) is not str or not session for session in sessions
    ):
        return False
    processes = pre.get("processes")
    if not isinstance(processes, list):
        return False
    for process in processes:
        if not isinstance(process, dict):
            return False
        for field in ("ProcessId", "ParentProcessId", "SessionId"):
            if type(process.get(field)) is not int:
                return False
        for field in ("Name", "CreationDate"):
            if type(process.get(field)) is not str or not process[field]:
                return False
        command_line = process.get("CommandLine")
        if command_line is not None and (type(command_line) is not str or not command_line):
            return False
    lasso = pre.get("processLasso")
    if not isinstance(lasso, dict) or type(lasso.get("running")) is not bool:
        return False
    if not isinstance(lasso.get("processes"), list):
        return False
    if lasso["running"] and not lasso["processes"]:
        return False
    if "files" in lasso and not isinstance(lasso["files"], list):
        return False
    for file_entry in lasso.get("files", []):
        if not isinstance(file_entry, dict):
            return False
        path = file_entry.get("path")
        digest = file_entry.get("sha256")
        if type(path) is not str or not path:
            return False
        if type(digest) is not str or len(digest) != 64 or any(c not in "0123456789abcdefABCDEF" for c in digest):
            return False
    dxdiag = pre.get("dxdiag")
    if not isinstance(dxdiag, list) or not dxdiag:
        return False
    for display in dxdiag:
        if not isinstance(display, dict):
            return False
        for field in ("cardName", "driverVersion"):
            if type(display.get(field)) is not str or not display[field]:
                return False
        for field in ("currentMode", "hybridGraphicsGPU", "monitorName"):
            if field in display and display[field] is not None and (
                type(display[field]) is not str or not display[field]
            ):
                return False
    server = pre.get("server")
    if not isinstance(server, dict) or type(server.get("ProcessId")) is not int:
        return False
    for field in ("Name", "CreationDate"):
        if type(server.get(field)) is not str or not server[field]:
            return False
    if type(pre.get("performanceAcceptance")) is not bool:
        return False
    if type(value.get("performance_acceptance")) is not bool:
        return False
    if "process_lasso" in value:
        duplicate = value["process_lasso"]
        if not isinstance(duplicate, dict) or type(duplicate.get("running")) is not bool:
            return False
        if not isinstance(duplicate.get("processes"), list):
            return False
        if not _typed_equal(duplicate, lasso):
            return False
    if "dxdiag" in value:
        if not isinstance(value["dxdiag"], list) or not _typed_subset_equal(value["dxdiag"], dxdiag):
            return False
    return True


def _host_conditions_complete(value: Any) -> bool:
    """Validate native observations by schema; retain strict legacy behavior."""
    if _native_windows_conditions_recognized(value):
        return _native_windows_conditions_complete(value)
    return isinstance(value, dict) and not _deep_missing(value)


def _typed_equal(left, right):
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(_typed_equal(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(_typed_equal(a, b) for a, b in zip(left, right))
    return left == right


def _typed_subset_equal(left, right):
    """Compare a duplicate observation while allowing omitted optional fields."""
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return all(key in right and _typed_subset_equal(value, right[key]) for key, value in left.items())
    if isinstance(left, list):
        return len(left) == len(right) and all(_typed_subset_equal(a, b) for a, b in zip(left, right))
    return left == right


def sha256_file(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_path(path: pathlib.Path | str) -> pathlib.Path:
    """Resolve to a canonical file path. No artifact-root fence."""
    p = pathlib.Path(path).expanduser()
    try:
        return p.resolve(strict=False)
    except OSError:
        return p.absolute()


def _load_json(path: pathlib.Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _file_hash_if_present(path: pathlib.Path) -> Optional[str]:
    if not path.is_file():
        return None
    return sha256_file(path)


def _observation_wall_span_from_analysis(analysis: dict, meta: dict) -> Optional[tuple[float, float]]:
    """Reuse analyzed observation_window bounds; never whole-process span alone.

    Requires finite increasing obs_start/obs_end contained in process times when
    process times exist. Process-local mono origins are never compared across runs.
    """
    window = analysis.get("observation_window") if isinstance(analysis, dict) else None
    if not isinstance(window, dict):
        return None
    if window.get("status") == "unavailable":
        return None
    start = window.get("obs_start_unix")
    end = window.get("obs_end_unix")
    if not isinstance(start, (int, float)) or not isinstance(end, (int, float)):
        return None
    start_f, end_f = float(start), float(end)
    if not (end_f > start_f):
        return None
    started = meta.get("started_unix")
    ended = meta.get("ended_unix")
    if isinstance(started, (int, float)) and isinstance(ended, (int, float)):
        if start_f < float(started) - 1e-6 or end_f > float(ended) + 1e-6:
            return None
        if not (float(ended) > float(started)):
            return None
    return (start_f, end_f)


def _windows_overlap(a: tuple[float, float], b: tuple[float, float]) -> bool:
    return not (a[1] <= b[0] or b[1] <= a[0])


def _manifest_binary_entry(manifest: dict, key: str) -> Optional[dict]:
    binaries = manifest.get("binaries")
    if not isinstance(binaries, dict):
        return None
    entry = binaries.get(key)
    return entry if isinstance(entry, dict) else None


def _manifest_side_block(manifest: dict, role: str) -> Optional[dict]:
    side_name = _ROLE_MANIFEST_SIDE.get(role)
    if side_name is None:
        return None
    block = manifest.get(side_name)
    return block if isinstance(block, dict) else None


def _build_commit_from_side(side: dict) -> Optional[str]:
    for key in ("commit", "commit_at_freeze_snap"):
        val = side.get(key)
        if isinstance(val, str) and val:
            return val
    return None


def _normalize_role(role: str) -> str:
    r = (role or "").strip().lower()
    if r in ("reference", "control"):
        return "reference" if r == "reference" else "control"
    if r == "candidate":
        return "candidate"
    return r


def construct_match_keys(meta: dict, *, extras: Optional[dict] = None) -> dict:
    """Build strict match-key dict. Missing/null members stay present as None.

    Never default TUI render_policy null → \"none\". Absent stays unavailable.
    """
    out: dict[str, Any] = {}
    # Legacy launcher lacked this opt-in flag; its validated CLI proves off.
    out['tui_input_probes'] = meta.get('tui_input_probes', False)
    # Legacy launcher lacked explicit backend request; default from frontend.
    cpu_fb = meta.get('cpu_fallback', False)
    if type(cpu_fb) is not bool:
        cpu_fb = False
    out['cpu_fallback'] = cpu_fb
    rb = meta.get('requested_backend')
    if rb is None:
        if cpu_fb:
            rb = 'cpu_fallback'
        elif meta.get('frontend') == 'panel':
            rb = 'gpu'
        else:
            rb = 'none'
    out['requested_backend'] = rb
    for key in MATCH_KEY_FIELDS:
        if key in (
            "server_start_identity",
            "server_port_listen",
            "server_configuration",
            "sampler_interval_s",
            "sampler_duration_mode",
            "sampler_duration_s_requested",
            "host_conditions",
            "cpu_fallback",
            "requested_backend",
        ):
            continue
        out[key] = meta.get(key)
    if extras:
        for key in (
            "server_start_identity",
            "server_port_listen",
            "server_configuration",
            "sampler_interval_s",
            "sampler_duration_mode",
            "sampler_duration_s_requested",
            "host_conditions",
        ):
            if key in extras:
                out[key] = extras[key]
    return out


def match_keys_complete(keys: dict) -> Optional[str]:
    """Return missing field name if any match key is null/empty/nested-null.

    ``sampler_duration_s_requested`` may be explicit null only when
    ``sampler_duration_mode == \"stop_controlled\"`` (schema-2 continuous).
    Fixed mode still requires a finite positive requested duration.
    """
    mode = keys.get("sampler_duration_mode")
    for key in MATCH_KEY_FIELDS:
        if key not in keys:
            return key
        val = keys.get(key)
        if key == "sampler_duration_s_requested":
            if mode == "stop_controlled":
                if val is not None:
                    return key
                continue
            if type(val) not in (int, float):
                return key
            fval = float(val)
            if not math.isfinite(fval) or fval <= 0:
                return key
            continue
        if key == "sampler_duration_mode":
            if val not in ("fixed", "stop_controlled"):
                return key
            continue
        if key == "terminal_size":
            # Panel launches are deliberately non-terminal. Explicit null is
            # an inapplicable geometry field, not missing evidence.
            if keys.get("frontend") == "panel" and keys.get("terminal") is False:
                if val is not None:
                    return key
                continue
            if (
                type(val) is not list
                or len(val) != 2
                or any(type(size) is not int or size <= 0 for size in val)
            ):
                return key
            continue
        if key == "host_conditions":
            if not _host_conditions_complete(val):
                return key
            continue
        if _deep_missing(val):
            return key
    return None


def construct_side_provenance(
    *,
    role: str,
    meta: dict,
    binary_path: pathlib.Path,
    binary_sha256: str,
    manifest_key: str,
    manifest_side: dict,
) -> dict:
    client = manifest_side.get("client") if isinstance(manifest_side.get("client"), dict) else {}
    pre = manifest_side.get("sources_sha256_pre")
    post = manifest_side.get("sources_sha256_post")
    stable = (
        isinstance(pre, str)
        and isinstance(post, str)
        and pre != ""
        and post != ""
        and pre == post
    )
    return {
        "role": role,
        "binary_path": str(binary_path),
        "binary_sha256": binary_sha256,
        "manifest_binary_key": manifest_key,
        "manifest_build_commit": _build_commit_from_side(manifest_side),
        "manifest_sources_sha256_pre": pre,
        "manifest_sources_sha256_post": post,
        "manifest_sources_sha256": pre if stable else None,
        "manifest_sources_stable": stable,
        "manifest_branch": manifest_side.get("branch"),
        "client_commit": client.get("commit"),
        "client_sources_sha256": client.get("sources_sha256"),
        "host_commit_checkout": meta.get("host_commit"),
        # Explicit: checkout host_commit is not build authority.
        "host_commit_is_build_authority": False,
    }


def _cli_flag_value(cli: list, flag: str) -> Optional[str]:
    for i, tok in enumerate(cli):
        if tok == flag and i + 1 < len(cli):
            return str(cli[i + 1])
    return None


def _validate_effective_cli(receipt: dict, meta: dict) -> Optional[str]:
    """Use the launcher's actual parser, including last-wins option semantics."""
    cli = receipt.get("effective_cli")
    if not isinstance(cli, list) or len(cli) < 5 or not all(isinstance(x, str) for x in cli):
        return "receipt_effective_cli_invalid"
    try:
        parser = rd.build_parser()
        with contextlib.redirect_stderr(io.StringIO()):
            args = parser.parse_args(cli[2:])
            rd.validate_args(args, parser)
    except (SystemExit, TypeError, ValueError):
        return "receipt_effective_cli_parse_failed"
    if args.binary is None:
        return "receipt_cli_missing_binary"
    if canonical_path(args.binary) != canonical_path(meta.get("binary")):
        return "receipt_cli_binary_mismatch"
    if "--warmup" not in cli or "--observe" not in cli:
        return "receipt_cli_missing_timing"
    values = {
        "frontend": args.frontend, "n": args.n, "workload": args.workload,
        "warmup_s": args.warmup, "observe_s": args.observe,
        "sustain": args.sustain, "diagnostic_sidecar": not args.no_diagnostics,
        "failure_capture": args.failure_capture, "nav_captures": args.nav_captures,
        "single_renderer": args.single_renderer,
        "terminal": args.frontend == 'tui' and not args.headless,
        "render_policy": rd.requested_render_policy(args), "render_policy_requested": True,
        "stack_logging": args.stack_logging or args.stack_logging_lite,
        "stack_logging_mode": 'lite' if args.stack_logging_lite else ('1' if args.stack_logging else None),
        "debug": args.debug,
    }
    for field in ('scheduling_profile', 'render_profile', 'gpu_completion_profile',
                  'responsiveness_profile', 'responsiveness_fine'):
        values[field] = getattr(args, field)
    for field, expected in values.items():
        if field not in meta or meta[field] != expected or (type(expected) is bool and type(meta[field]) is not bool):
            return f"receipt_cli_metadata_mismatch:{field}"
    probe_flag = meta.get('tui_input_probes', False)
    if type(probe_flag) is not bool or probe_flag != args.tui_input_probes:
        return 'receipt_cli_metadata_mismatch:tui_input_probes'
    # Legacy launcher omitted cpu_fallback/requested_backend; CLI default proves GPU/none.
    cpu_flag = bool(getattr(args, 'cpu_fallback', False))
    meta_cpu = meta.get('cpu_fallback', False)
    if type(meta_cpu) is not bool or meta_cpu != cpu_flag:
        return 'receipt_cli_metadata_mismatch:cpu_fallback'
    expected_backend = rd.requested_backend(args)
    meta_backend = meta.get('requested_backend')
    if meta_backend is None:
        if cpu_flag:
            return 'receipt_cli_metadata_mismatch:requested_backend'
        meta_backend = expected_backend
    if meta_backend != expected_backend:
        return 'receipt_cli_metadata_mismatch:requested_backend'
    if args.build_manifest is not None:
        if canonical_path(args.build_manifest) != canonical_path(receipt.get('manifest_path')):
            return 'receipt_cli_manifest_mismatch'
        if args.build_role != receipt.get('_binding_role'):
            return 'receipt_cli_build_role_mismatch'
    return None


def _validate_meta_against_manifest_fixtures(meta: dict, manifest: dict, manifest_side: dict) -> Optional[str]:
    """When both metadata and manifest carry a fixture field, they must agree.

    Never fill missing metadata from the manifest.
    """
    nav = manifest.get("nav") if isinstance(manifest.get("nav"), dict) else {}
    catalog = manifest.get("catalog") if isinstance(manifest.get("catalog"), dict) else {}
    features = manifest.get("features") if isinstance(manifest.get("features"), dict) else {}
    client = manifest_side.get("client") if isinstance(manifest_side.get("client"), dict) else {}

    pairs = [
        ("nav_pack_sha256", nav.get("nav_pack_sha256")),
        ("nav_flags_sha256", nav.get("nav_flags_sha256")),
        ("catalog_sha256", catalog.get("js_scripts_json_sha256")),
        ("client_sources_sha256", client.get("sources_sha256")),
        ("client_commit", client.get("commit")),
    ]
    for meta_key, man_val in pairs:
        meta_val = meta.get('build_provenance', {}).get(meta_key) if meta_key.startswith('client_') else meta.get(meta_key)
        if _is_missing(meta_val) or _is_missing(man_val):
            continue
        if meta_val != man_val:
            return f"metadata_manifest_mismatch:{meta_key}"

    if not _is_missing(meta.get("allocator_provenance")) and not _is_missing(features.get("allocator")):
        if meta.get("allocator_provenance") != features.get("allocator"):
            return "metadata_manifest_mismatch:allocator_provenance"

    meta_ff = meta.get("feature_flags")
    if isinstance(meta_ff, dict) and features:
        # Compare overlapping keys only when both present.
        for k in ("requested", "locked", "allocation_counting"):
            if k in meta_ff and k in features and meta_ff.get(k) != features.get(k):
                return f"metadata_manifest_mismatch:feature_flags.{k}"
    return None


def _bind_side(
    receipt_path: pathlib.Path | str,
    *,
    role: str,
    manifest_path: pathlib.Path | str,
    counting: bool = False,
    diagnostics: bool = False,
    server_identity_path: Optional[pathlib.Path | str] = None,
    host_conditions_path: Optional[pathlib.Path | str] = None,
    cell_dir: Optional[pathlib.Path | str] = None,
) -> dict:
    """Bind one cell receipt through the authoritative artifact chain.

    ``binding_ok`` requires the full receipt→metadata→recorded raw hashes→
    exact named manifest binary path→independent qualify/analyze chain.
    ``pair_eligible`` is never set True here.
    """
    role_n = _normalize_role(role)
    if role_n not in ("reference", "control", "candidate"):
        return _unavailable("invalid_role", field="role", role=role)

    receipt_path = canonical_path(receipt_path)
    manifest_path = canonical_path(manifest_path)
    if not receipt_path.is_file():
        return _unavailable("missing_receipt", path=str(receipt_path))
    if not manifest_path.is_file():
        return _unavailable("missing_manifest", path=str(manifest_path))

    try:
        receipt_bytes = receipt_path.read_bytes()
        receipt = json.loads(receipt_bytes)
        receipt_file_sha256 = sha256_bytes(receipt_bytes)
    except (OSError, json.JSONDecodeError) as exc:
        return _unavailable("receipt_unreadable", error=str(exc), path=str(receipt_path))
    if not isinstance(receipt, dict):
        return _unavailable("receipt_not_object", path=str(receipt_path))

    for field in REQUIRED_RECEIPT_FIELDS:
        if field not in receipt or _is_missing(receipt.get(field)):
            return _unavailable("receipt_missing_field", field=field, path=str(receipt_path))
    if not isinstance(receipt['id'], str) or type(receipt['index']) is not int or receipt['index'] < 1:
        return _unavailable('receipt_identity_invalid')
    if receipt['kind'] not in ('matched', 'overhead', 'diagnostic') or type(receipt['exit_code']) is not int:
        return _unavailable('receipt_kind_or_exit_invalid')
    if receipt.get('binding_errors') or receipt.get('status') == 'failed_or_unavailable':
        return _unavailable('receipt_reports_binding_errors')
    if 'launcher_exit_code' in receipt and (type(receipt['launcher_exit_code']) is not int or receipt['launcher_exit_code'] != 0):
        return _unavailable('launcher_failed_or_invalid')
    sampler_result = receipt.get('sampler_result')
    if sampler_result is not None and (not isinstance(sampler_result, dict) or type(sampler_result.get('exit_code')) is not int or sampler_result['exit_code'] != 0):
        return _unavailable('sampler_failed_or_invalid')
    receipt['_binding_role'] = 'reference' if role_n in ('control', 'reference') else 'candidate'

    def utc_seconds(value):
        if not isinstance(value, str) or not value.endswith('Z'):
            raise ValueError('receipt time requires explicit UTC')
        return datetime.datetime.fromisoformat(value[:-1] + '+00:00').timestamp()

    receipt_start, receipt_end = utc_seconds(receipt['started_utc']), utc_seconds(receipt['ended_utc'])
    if not receipt_end > receipt_start:
        return _unavailable('receipt_time_envelope_invalid')
    if not receipt.get('manifest_path') or canonical_path(receipt['manifest_path']) != manifest_path:
        return _unavailable('receipt_manifest_path_mismatch_or_missing')
    bp._digest(receipt.get('manifest_sha256'), 'receipt manifest hash')
    if sha256_file(manifest_path) != receipt['manifest_sha256']:
        return _unavailable('receipt_manifest_hash_mismatch')

    run_dir = canonical_path(receipt["run_dir"])
    if not run_dir.is_dir():
        return _unavailable("run_dir_missing", path=str(run_dir))

    missing_files = [name for name in REQUIRED_RUN_FILES if not (run_dir / name).is_file()]
    if missing_files:
        return _unavailable(
            "missing_run_files",
            path=str(run_dir),
            missing=missing_files,
        )

    meta_path = run_dir / "metadata.json"
    try:
        meta = _load_json(meta_path)
    except (OSError, json.JSONDecodeError) as exc:
        return _unavailable("metadata_unreadable", error=str(exc), path=str(meta_path))
    if not isinstance(meta, dict):
        return _unavailable("metadata_not_object", path=str(meta_path))

    for field in REQUIRED_METADATA_FIELDS:
        if field not in meta or _is_missing(meta.get(field)):
            return _unavailable("metadata_missing_field", field=field, run_dir=str(run_dir))
    for field in ('n', 'pid', 'exit_code'):
        if type(meta[field]) is not int or (field != 'exit_code' and meta[field] < 1):
            return _unavailable('metadata_integer_invalid', field=field)
    for field in ('warmup_s', 'observe_s', 'started_unix', 'ended_unix'):
        if type(meta[field]) not in (int, float) or not math.isfinite(meta[field]) or meta[field] < 0:
            return _unavailable('metadata_number_invalid', field=field)
    if not receipt_start <= meta['started_unix'] < meta['ended_unix'] <= receipt_end:
        return _unavailable('receipt_metadata_time_envelope_mismatch')
    if type(meta.get('allocation_counting')) is not bool or meta['allocation_counting'] != counting:
        return _unavailable('allocation_counting_configuration_mismatch')
    if type(meta.get('diagnostic_sidecar')) is not bool or meta['diagnostic_sidecar'] != diagnostics:
        return _unavailable('diagnostic_configuration_mismatch')

    if canonical_path(meta["run_dir"]) != run_dir:
        return _unavailable(
            "receipt_metadata_run_dir_mismatch",
            receipt_run_dir=str(run_dir),
            metadata_run_dir=str(canonical_path(meta["run_dir"])),
        )

    receipt_exit = receipt.get("exit_code")
    meta_exit = meta.get("exit_code")
    if receipt_exit != meta_exit:
        return _unavailable(
            "receipt_metadata_exit_mismatch",
            receipt_exit=receipt_exit,
            metadata_exit=meta_exit,
        )

    # Receipt binary must agree with metadata binary (canonical path).
    if canonical_path(receipt["binary"]) != canonical_path(meta["binary"]):
        return _unavailable(
            "receipt_metadata_binary_mismatch",
            receipt_binary=str(canonical_path(receipt["binary"])),
            metadata_binary=str(canonical_path(meta["binary"])),
        )

    cli_err = _validate_effective_cli(receipt, meta)
    if cli_err is not None:
        return _unavailable(cli_err, path=str(receipt_path))

    # Runtime fixtures need actual metadata paths (launcher keys nav_pack,
    # nav_flags, catalog_path). Digest-only claims are not binding: verify_build
    # would otherwise fall back to manifest-recorded paths when actual is None.
    runtime_fixture_paths = {}
    for field in ('nav_pack', 'nav_flags', 'catalog_path'):
        value = meta.get(field)
        if not isinstance(value, str) or not value:
            return _unavailable('metadata_runtime_fixture_path_missing', field=field)
        runtime_fixture_paths[field] = value

    # Reuse the reviewed launcher verifier instead of duplicating a weaker
    # manifest/fixture implementation. It verifies real canonical file paths.
    verified_build = bp.verify_build(
        manifest_path,
        receipt['_binding_role'],
        meta['frontend'],
        meta['binary'],
        runtime_fixture_paths['nav_pack'],
        runtime_fixture_paths['nav_flags'],
        runtime_fixture_paths['catalog_path'],
    )
    recorded_build = meta.get('build_provenance')
    if not isinstance(recorded_build, dict) or recorded_build.get('status') != 'verified' or recorded_build.get('completion_status') != 'unchanged':
        return _unavailable('completed_build_provenance_missing_or_invalid')
    for field, expected in verified_build.items():
        if not _typed_equal(recorded_build.get(field), expected):
            return _unavailable('metadata_build_provenance_mismatch', field=field)
    # All runtime fields remain explicit. Build client identities come from the
    # recorded and verified nested build object, never checkout source labels.
    for field in ('feature_flags', 'allocator_provenance', 'allocation_counting'):
        if not _typed_equal(meta.get(field), verified_build[field]):
            return _unavailable('metadata_build_configuration_mismatch', field=field)
    for field, file_key in (('nav_pack_sha256', 'nav_pack'), ('nav_flags_sha256', 'nav_flags'), ('catalog_sha256', 'catalog')):
        if meta.get(field) != verified_build['files'][file_key]['sha256']:
            return _unavailable('metadata_runtime_fixture_mismatch', field=field)

    started = meta.get("started_unix")
    ended = meta.get("ended_unix")
    if not (isinstance(started, (int, float)) and isinstance(ended, (int, float)) and float(ended) > float(started)):
        return _unavailable("metadata_time_envelope_invalid", run_dir=str(run_dir))

    # Live snapshot hashes of raw artifacts.
    snapshot_hashes = {
        "metadata.json": _file_hash_if_present(meta_path),
        "samples.jsonl": _file_hash_if_present(run_dir / "samples.jsonl"),
        "samples.qualification.jsonl": _file_hash_if_present(
            run_dir / "samples.qualification.jsonl"
        ),
    }
    if meta.get('tui_input_probes') is True:
        result = meta.get('input_probe_result')
        probe_path = run_dir / 'input-probes.jsonl'
        digest = _file_hash_if_present(probe_path)
        if (not isinstance(result, dict) or result.get('error') is not None
                or type(result.get('sent')) is not int or result['sent'] <= 0
                or not isinstance(result.get('path'), str)
                or canonical_path(result['path']) != probe_path
                or digest is None or result.get('sha256') != digest):
            return _unavailable('input_probe_artifact_invalid')
        snapshot_hashes['input-probes.jsonl'] = digest
    if any(v is None for v in snapshot_hashes.values()):
        return _unavailable("raw_hash_failed", path=str(run_dir), raw_hashes=snapshot_hashes)

    # Binding requires receipt-recorded raw hashes (not merely a live snapshot).
    recorded = receipt.get("raw_hashes") or receipt.get("artifact_hashes")
    if not isinstance(recorded, dict) or not recorded:
        return _unavailable(
            "receipt_raw_hashes_missing",
            path=str(receipt_path),
            snapshot_hashes=snapshot_hashes,
            raw_hash_status="live_snapshot_only",
        )
    for name, snap in snapshot_hashes.items():
        if name not in recorded or _is_missing(recorded.get(name)):
            return _unavailable(
                "receipt_raw_hash_field_missing",
                field=name,
                path=str(receipt_path),
                raw_hash_status="incomplete_recorded",
            )
        if recorded.get(name) != snap:
            return _unavailable(
                "receipt_raw_hash_mismatch",
                field=name,
                recorded=recorded.get(name),
                actual=snap,
                raw_hash_status="recorded_mismatch",
            )
    raw_hashes = dict(snapshot_hashes)
    raw_hash_status = "receipt_recorded_and_verified"

    run_identity = {
        "run_dir": str(run_dir),
        "raw_hashes": raw_hashes,
        "raw_hash_status": raw_hash_status,
        "identity_sha256": sha256_bytes(
            json.dumps(
                {"run_dir": str(run_dir), "raw_hashes": raw_hashes},
                sort_keys=True,
            ).encode()
        ),
    }

    binary_path = canonical_path(meta["binary"])
    if not binary_path.is_file():
        return _unavailable("binary_missing", path=str(binary_path))

    try:
        actual_sha = sha256_file(binary_path)
    except OSError as exc:
        return _unavailable("binary_unreadable", path=str(binary_path), error=str(exc))

    meta_sha = meta.get("binary_sha256")
    if meta_sha != actual_sha:
        return _unavailable(
            "binary_hash_mismatch_metadata",
            path=str(binary_path),
            metadata_sha256=meta_sha,
            actual_sha256=actual_sha,
        )

    try:
        manifest = _load_json(manifest_path)
    except (OSError, json.JSONDecodeError) as exc:
        return _unavailable("manifest_unreadable", error=str(exc), path=str(manifest_path))
    if not isinstance(manifest, dict):
        return _unavailable("manifest_not_object", path=str(manifest_path))

    frontend = meta.get("frontend")
    role_for_key = "control" if role_n in ("reference", "control") else "candidate"
    manifest_key = _MANIFEST_BINARY_KEYS.get((role_for_key, str(frontend)))
    if manifest_key is None:
        manifest_key = _MANIFEST_BINARY_KEYS.get((role_n, str(frontend)))
    if manifest_key is None:
        return _unavailable(
            "no_manifest_binary_key",
            role=role_n,
            frontend=frontend,
        )

    entry = _manifest_binary_entry(manifest, manifest_key)
    if entry is None:
        return _unavailable(
            "manifest_binary_entry_missing",
            key=manifest_key,
            path=str(manifest_path),
        )

    entry_sha = entry.get("sha256")
    entry_path_raw = entry.get("path")
    if _is_missing(entry_sha) or _is_missing(entry_path_raw):
        return _unavailable(
            "manifest_binary_entry_incomplete",
            key=manifest_key,
        )

    entry_path = canonical_path(entry_path_raw)
    # Exact canonical named manifest binary path required (symlink→same path OK).
    # A same-hash copy at a different path is not the declared path.
    if entry_path != binary_path:
        return _unavailable(
            "binary_path_not_manifest_canonical_path",
            key=manifest_key,
            binary_path=str(binary_path),
            manifest_path=str(entry_path),
            binary_sha256=actual_sha,
            manifest_sha256=entry_sha,
        )
    if entry_sha != actual_sha:
        return _unavailable(
            "binary_hash_mismatch_manifest",
            key=manifest_key,
            actual_sha256=actual_sha,
            manifest_sha256=entry_sha,
        )

    manifest_side = _manifest_side_block(manifest, role_for_key)
    if manifest_side is None:
        return _unavailable(
            "manifest_side_block_missing",
            role=role_for_key,
            path=str(manifest_path),
        )

    side_prov = construct_side_provenance(
        role=role_n if role_n != "control" else "reference",
        meta=meta,
        binary_path=binary_path,
        binary_sha256=actual_sha,
        manifest_key=manifest_key,
        manifest_side=manifest_side,
    )
    if _is_missing(side_prov.get("manifest_build_commit")):
        return _unavailable(
            "manifest_build_commit_missing",
            role=role_for_key,
            side_provenance=side_prov,
        )
    if not side_prov.get("manifest_sources_stable"):
        return _unavailable(
            "manifest_sources_sha256_unstable_or_missing",
            role=role_for_key,
            side_provenance=side_prov,
        )
    if _is_missing(side_prov.get("client_commit")) or _is_missing(side_prov.get("client_sources_sha256")):
        return _unavailable(
            "manifest_client_provenance_missing",
            role=role_for_key,
            side_provenance=side_prov,
        )

    # Legacy top-level source/client labels describe the checkout. The actual
    # binary source/client evidence was independently verified above.

    fix_err = _validate_meta_against_manifest_fixtures(meta, manifest, manifest_side)
    if fix_err is not None:
        return _unavailable(fix_err, run_dir=str(run_dir))

    # Features/allocator fixture bindings required on manifest for side completeness.
    features = manifest.get("features") if isinstance(manifest.get("features"), dict) else {}
    if _is_missing(features.get("allocator")):
        return _unavailable("manifest_allocator_missing", path=str(manifest_path))
    if "allocation_counting" not in features:
        return _unavailable("manifest_allocation_counting_missing", path=str(manifest_path))

    # Server / sampler / host identity (non-sensitive only). Never invent.
    server_extras: dict[str, Any] = {}
    if server_identity_path is None:
        return _unavailable("server_identity_path_required")
    sip = canonical_path(server_identity_path)
    if not sip.is_file():
        return _unavailable("server_identity_missing", path=str(sip))
    if not receipt.get('server_identity_path') or canonical_path(receipt['server_identity_path']) != sip:
        return _unavailable('receipt_server_identity_path_mismatch_or_missing')
    if receipt.get('server_identity_sha256') != sha256_file(sip):
        return _unavailable('receipt_server_identity_hash_mismatch')
    try:
        sid = _load_json(sip)
    except (OSError, json.JSONDecodeError) as exc:
        return _unavailable(
            "server_identity_unreadable",
            path=str(sip),
            error=str(exc),
        )
    if not isinstance(sid, dict):
        return _unavailable("server_identity_not_object", path=str(sip))
    if not isinstance(sid.get('start_identity'), str) or not sid['start_identity']:
        return _unavailable("server_start_identity_missing", path=str(sip))
    if type(sid.get('port_listen')) is not int or not 0 < sid['port_listen'] < 65536:
        return _unavailable("server_port_listen_missing", path=str(sip))
    if type(sid.get('pid')) is not int or sid['pid'] <= 0:
        return _unavailable('server_pid_missing_or_invalid')
    server_extras["server_start_identity"] = {'pid': sid['pid'], 'start_identity': sid['start_identity']}
    server_extras["server_port_listen"] = sid.get("port_listen")
    server_extras['server_configuration'] = sid.get('configuration')

    sampler_raw = receipt.get("sampler")
    sampler: dict[str, Any] = sampler_raw if isinstance(sampler_raw, dict) else {}
    interval = sampler.get("interval_s")
    if type(interval) not in (int, float):
        return _unavailable("receipt_sampler_interval_missing")
    interval_f = float(interval)
    if not math.isfinite(interval_f) or interval_f <= 0:
        return _unavailable("receipt_sampler_configuration_invalid", field="interval_s")
    duration_mode = sampler.get("duration_mode")
    duration_req = sampler.get("duration_s_requested")
    # Legacy receipts omit duration_mode; treat present positive duration as fixed.
    if duration_mode is None:
        if "duration_s_requested" not in sampler:
            return _unavailable("receipt_sampler_duration_missing")
        duration_mode = "fixed"
    if duration_mode not in ("fixed", "stop_controlled"):
        return _unavailable("receipt_sampler_duration_mode_invalid", field="duration_mode")
    if duration_mode == "fixed":
        if type(duration_req) not in (int, float):
            return _unavailable("receipt_sampler_configuration_invalid", field="duration_s_requested")
        duration_f = float(duration_req)
        if not math.isfinite(duration_f) or duration_f <= 0:
            return _unavailable("receipt_sampler_configuration_invalid", field="duration_s_requested")
    else:
        # stop_controlled: duration_s_requested must be explicit JSON null.
        if "duration_s_requested" not in sampler:
            return _unavailable("receipt_sampler_duration_missing")
        if duration_req is not None:
            return _unavailable(
                "receipt_sampler_stop_controlled_duration_must_be_null",
                field="duration_s_requested",
            )
    server_extras["sampler_interval_s"] = interval_f
    server_extras["sampler_duration_mode"] = duration_mode
    server_extras["sampler_duration_s_requested"] = (
        float(duration_req) if duration_mode == "fixed" else None
    )

    # Host conditions: required non-sensitive machine identity blob.
    if host_conditions_path is not None:
        hcp = canonical_path(host_conditions_path)
        if not hcp.is_file():
            return _unavailable("host_conditions_missing", path=str(hcp))
        if not receipt.get('host_conditions_path') or canonical_path(receipt['host_conditions_path']) != hcp:
            return _unavailable('receipt_host_conditions_path_mismatch_or_missing')
        if receipt.get('host_conditions_sha256') != sha256_file(hcp):
            return _unavailable('receipt_host_conditions_hash_mismatch')
        try:
            hc = _load_json(hcp)
        except (OSError, json.JSONDecodeError) as exc:
            return _unavailable("host_conditions_unreadable", path=str(hcp), error=str(exc))
        if not _host_conditions_complete(hc):
            return _unavailable("host_conditions_invalid", path=str(hcp))
        server_extras["host_conditions"] = hc
    elif _host_conditions_complete(meta.get("host_conditions")):
        server_extras["host_conditions"] = meta.get("host_conditions")
    elif _host_conditions_complete(receipt.get("host_conditions")):
        server_extras["host_conditions"] = receipt.get("host_conditions")
    else:
        return _unavailable("host_conditions_missing")

    # Metadata is authoritative — no manifest enrichment of missing match keys.
    match_keys = construct_match_keys(meta, extras=server_extras)
    match_keys['client_sources_sha256'] = recorded_build['client_sources_sha256']

    # Independent qualification (never trust receipt.qualification labels).
    qualification = qc.qualify(run_dir, counting=counting, diagnostics=diagnostics)
    analysis = rm.analyze_run(
        run_dir,
        qualify=True,
        counting=counting,
        diagnostics=diagnostics,
    )

    # Post-read content recheck: raw files must still match recorded hashes.
    recheck = {name: _file_hash_if_present(run_dir / name) for name in raw_hashes}
    if recheck != raw_hashes:
        return _unavailable(
            "raw_hash_changed_after_read",
            before=raw_hashes,
            after=recheck,
            raw_hash_status="mutated_after_bind_read",
        )
    bp.recheck_files(verified_build['files'])
    if sha256_file(sip) != receipt['server_identity_sha256']:
        return _unavailable('server_identity_changed_after_read')
    if host_conditions_path is not None and sha256_file(hcp) != receipt['host_conditions_sha256']:
        return _unavailable('host_conditions_changed_after_read')
    if sha256_file(receipt_path) != receipt_file_sha256:
        return _unavailable('receipt_changed_after_read')

    native_qualification = consume_native_qualification(run_dir, n=meta.get("n"), meta=meta)
    wall_span_source = None
    if native_qualification.get("status") == "available" and native_qualification.get("observation_wall_span"):
        wall_span = tuple(native_qualification["observation_wall_span"])
        wall_span_source = "native_elapsed_wall_bracket"
    else:
        wall_span = _observation_wall_span_from_analysis(analysis, meta)
        wall_span_source = "analysis_observation_window" if wall_span is not None else None
        # Malformed native brackets (rows present with ordinals but invalid wall) fail closed
        # when the file already carries native-shaped boundaries.
        if native_qualification.get("reason") == "elapsed_wall_bracket_missing":
            # Legacy qualification rows without brackets keep analysis fallback.
            pass
        elif native_qualification.get("slot_ordinals_present") is False and native_qualification.get("reason") not in (
            None,
            "qualification_empty",
            "qualification_file_missing",
            "settings_missing_or_not_object",
            "observe_start_settings:settings_missing_or_not_object",
        ):
            # If qualification looks native-shaped (ordinals attempted) but failed validation
            # including wall brackets, do not silently fall back to launcher+elapsed.
            reason = native_qualification.get("reason") or ""
            if isinstance(reason, str) and (
                reason.startswith("elapsed_wall")
                or reason.startswith("wall_")
                or reason.startswith("boundary_elapsed")
                or "wall_bracket" in reason
            ):
                return _unavailable(
                    "native_observation_wall_bracket_invalid",
                    native_reason=reason,
                )
    if wall_span is None:
        return _unavailable(
            "observation_window_invalid",
            observation_window=analysis.get("observation_window") if isinstance(analysis, dict) else None,
            native_qualification_reason=native_qualification.get("reason"),
        )

    managed_resources = mrb.bind(receipt, native_qualification, sid)
    if managed_resources.get('status') == 'available':
        # These keys come from the native boundary and pre/post-bound cache and
        # collector artifacts, never caller-supplied renderer/cache labels.
        match_keys.update(managed_resources['match_keys'])

    overhead_status = _evaluate_helper_overhead(
        receipt=receipt,
        cell_dir=canonical_path(cell_dir) if cell_dir is not None else receipt_path.parent,
    )

    try:
        samples = rm.load_run_samples(run_dir)
    except (OSError, ValueError, FileNotFoundError) as exc:
        return _unavailable("samples_unreadable", error=str(exc), path=str(run_dir))
    del samples  # loaded to ensure readability; metrics come from analysis
    if any(_file_hash_if_present(run_dir / name) != digest for name, digest in raw_hashes.items()):
        return _unavailable('raw_hash_changed_after_sample_read')

    qualified = qualification.get("qualified") is True and not qualification.get("errors")
    exit_ok = meta_exit == 0
    binding_ok = True

    resources_gate = (analysis.get("gates") or {}).get("resources") or {}
    shaped = {
        "status": resources_gate.get("status", "unavailable"),
        "reason": resources_gate.get("reason"),
        "match_metadata": rm.resource_match_keys_from_meta(meta),
        "side_provenance": {
            "binary_sha256": actual_sha,
            "host_sources_sha256": side_prov.get("manifest_sources_sha256"),
            "manifest_build_commit": side_prov.get("manifest_build_commit"),
            "role": side_prov.get("role"),
        },
        "overhead": "unknown",
        "cpu_cores": resources_gate.get("cpu_cores"),
        "resident_median_bytes": resources_gate.get("resident_median_bytes"),
        "contaminated": bool((analysis.get("observation_window") or {}).get("contaminated")),
        "observation_window": analysis.get("observation_window"),
        "workload_qualification": qualification,
    }

    missing_key = match_keys_complete(match_keys)
    return {
        "status": "bound" if binding_ok and qualified and exit_ok else "unavailable",
        "reason": None
        if binding_ok and qualified and exit_ok
        else (
            "workload_not_qualified"
            if binding_ok and not qualified
            else ("nonzero_exit" if binding_ok and not exit_ok else "bind_incomplete")
        ),
        "pass_means": PASS_MEANS,
        "final_acceptance_claim": False,
        "binding_ok": binding_ok,
        "pair_eligible": False,
        "role": side_prov.get("role"),
        "receipt_path": str(receipt_path),
        "receipt_id": receipt.get("id"),
        "run_dir": str(run_dir),
        "run_identity": run_identity,
        "raw_hashes": raw_hashes,
        "raw_hash_status": raw_hash_status,
        "binary_path": str(binary_path),
        "binary_sha256": actual_sha,
        "manifest_path": str(manifest_path),
        "manifest_binary_key": manifest_key,
        "side_provenance": side_prov,
        "match_keys": match_keys,
        "match_keys_missing": missing_key,
        "qualification": qualification,
        "qualified": qualified,
        "exit_code": meta_exit,
        "analysis": analysis,
        "observation_wall_span": wall_span,
        "observation_wall_span_source": wall_span_source,
        "overhead": overhead_status,
        "managed_resources": managed_resources,
        "shaped_for_compare": shaped,
        "endpoint_notes": {
            "scheduling": (analysis.get("gates") or {}).get("scheduling"),
            "decode": (analysis.get("gates") or {}).get("decode"),
            "input": (analysis.get("gates") or {}).get("input"),
            "gpu": (analysis.get("gates") or {}).get("gpu"),
        },
        "n": meta.get("n"),
        "native_qualification": native_qualification,
        "slot_ordinals_present": _slot_ordinals_present_from_native(native_qualification),
        "ordinal_mapping": native_qualification.get("ordinal_mapping"),
    }


def bind_side(receipt_path, *, role, manifest_path, counting=False, diagnostics=False,
              server_identity_path=None, host_conditions_path=None, cell_dir=None):
    """Malformed or unreadable artifacts are unavailable, never a traceback/pass."""
    try:
        return _bind_side(receipt_path, role=role, manifest_path=manifest_path,
                          counting=counting, diagnostics=diagnostics,
                          server_identity_path=server_identity_path,
                          host_conditions_path=host_conditions_path, cell_dir=cell_dir)
    except (ValueError, TypeError, KeyError, OSError, OverflowError) as error:
        return _unavailable('artifact_validation_failed', detail=str(error))


def _finite_number(value: Any) -> bool:
    return type(value) in (int, float) and math.isfinite(float(value))


def _load_qualification_boundaries(run_dir: pathlib.Path) -> tuple[Optional[list[dict]], Optional[str]]:
    path = run_dir / "samples.qualification.jsonl"
    if not path.is_file():
        return None, "qualification_file_missing"
    try:
        rows: list[dict] = []
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip():
                continue
            obj = json.loads(line)
            if not isinstance(obj, dict):
                return None, f"qualification_row_not_object:{line_no}"
            rows.append(obj)
        return rows, None
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as exc:
        return None, f"qualification_unreadable:{exc}"


# Stable per-slot runtime settings compared across runs (loop_cycle is freshness only).
_RUNTIME_SETTINGS_MATCH_FIELDS = (
    "lowmem",
    "midi_active",
    "midi_volume",
    "wave_enabled",
    "wave_volume",
    "draw",
)

# Global qualification settings that are equality match keys when native rows exist.
_QUAL_SETTINGS_MATCH_FIELDS = (
    "host",
    "port",
    "lowmem_requested",
    "mainland",
    "frontend",
    "n",
    "workload",
    "render_policy_requested",
    "single_renderer",
    "diagnostics",
    "failure_capture",
    "scheduling_profile_enabled",
    "render_profile_enabled",
    "gpu_completion_profile_enabled",
    "responsiveness_profile_enabled",
    "responsiveness_fine_enabled",
)

_WALL_BRACKET_TOL_S = 1.0
_WALL_BRACKET_MAX_WIDTH_S = 5.0


def _extract_runtime_settings_match(rs: Any) -> tuple[Optional[dict], Optional[str]]:
    """Null/missing settings cannot fill defaults; observed false is distinct from null."""
    if rs is None:
        return None, "runtime_settings_null"
    if not isinstance(rs, dict):
        return None, "runtime_settings_not_object"
    out: dict[str, Any] = {}
    for field in _RUNTIME_SETTINGS_MATCH_FIELDS:
        if field not in rs:
            return None, f"runtime_settings_missing:{field}"
        val = rs[field]
        if val is None:
            return None, f"runtime_settings_null_field:{field}"
        if field in ("lowmem", "midi_active", "wave_enabled", "draw"):
            if type(val) is not bool:
                return None, f"runtime_settings_type:{field}"
        elif field in ("midi_volume", "wave_volume"):
            if type(val) is not int:
                return None, f"runtime_settings_type:{field}"
        out[field] = val
    # Freshness only — require typed presence, never equality-match across runs.
    if type(rs.get("loop_cycle")) is not int or not -(1 << 31) <= rs['loop_cycle'] < (1 << 31):
        return None, "runtime_settings_loop_cycle_invalid"
    if rs.get("loop_cycle_meaning") != "client_mainloop_counter":
        return None, "runtime_settings_loop_cycle_meaning_invalid"
    return out, None


def _extract_renderer_match(slot: dict, *, profile_enabled: bool) -> tuple[Optional[dict], Optional[str]]:
    """Renderer optional when profile off (explicit disabled); required when on."""
    if not profile_enabled:
        if "renderer" in slot and slot.get("renderer") is not None:
            # Profile off should not publish live renderer rows as match evidence.
            return {"status": "disabled_profile_off"}, None
        return {"status": "disabled_profile_off"}, None
    renderer = slot.get("renderer")
    if not isinstance(renderer, dict):
        return None, "renderer_required_when_profile_enabled"
    if renderer.get("available") is not True:
        return None, "renderer_available_not_true"
    backend = renderer.get("backend")
    for field, expected_type in (
        ("renderer_present", bool),
        ("draw", bool),
        ("full_rate", bool),
        ("ended", bool),
    ):
        if type(renderer.get(field)) is not expected_type:
            return None, f"renderer_field_invalid:{field}"
    if type(renderer.get("generation")) is not int or not 0 <= renderer['generation'] < (1 << 64):
        return None, "renderer_generation_invalid"
    if (type(renderer.get('slot_id')) is not int
            or renderer['slot_id'] != slot.get('cadence_slot_id')):
        return None, 'renderer_slot_identity_mismatch'
    if renderer['ended']:
        return None, 'renderer_generation_ended'
    if renderer['renderer_present']:
        if backend not in ('cpu', 'cpu_fallback', 'gpu'):
            return None, 'renderer_backend_invalid'
    elif backend is not None:
        return None, 'absent_renderer_has_backend'
    else:
        # Explicitly observed absence is valid in the one-renderer panel mode.
        backend = 'absent'
    # Stable config only — omit timestamps, generation, run-local slot_id.
    return {
        "status": "enabled",
        "backend": backend,
        "renderer_present": renderer["renderer_present"],
        "draw": renderer["draw"],
        "full_rate": renderer["full_rate"],
        "ended": renderer["ended"],
    }, None


def _validate_slot_rows(slots: Any, *, n: int, phase: str) -> tuple[Optional[list[dict]], Optional[str]]:
    if not isinstance(slots, list):
        return None, f"slots_not_array:{phase}"
    if len(slots) != n:
        return None, f"slots_count_mismatch:{phase}"
    rows: list[dict] = []
    ordinals: set[int] = set()
    names: set[str] = set()
    resp_ids: set[Any] = set()
    cadence_ids: set[Any] = set()
    for idx, slot in enumerate(slots):
        if not isinstance(slot, dict):
            return None, f"slot_not_object:{phase}:{idx}"
        ordinal = slot.get("ordinal")
        if type(ordinal) is not int or isinstance(ordinal, bool):
            return None, f"ordinal_invalid:{phase}:{idx}"
        if ordinal in ordinals:
            return None, f"ordinal_duplicate:{phase}:{ordinal}"
        ordinals.add(ordinal)
        name = slot.get("name")
        if not isinstance(name, str) or not name:
            return None, f"slot_name_invalid:{phase}:{idx}"
        if name in names:
            return None, f"slot_name_duplicate:{phase}:{name}"
        names.add(name)
        resp_id = slot.get("responsiveness_slot_id")
        cadence_id = slot.get("cadence_slot_id")
        if any(type(v) is not int or not 0 <= v < (1 << 64) for v in (resp_id, cadence_id)):
            return None, f"slot_ids_invalid:{phase}:{idx}"
        if resp_id in resp_ids:
            return None, f"responsiveness_slot_id_duplicate:{phase}"
        if cadence_id in cadence_ids:
            return None, f"cadence_slot_id_duplicate:{phase}"
        resp_ids.add(resp_id)
        cadence_ids.add(cadence_id)
        rows.append(slot)
    expected = set(range(n))
    if ordinals != expected:
        return None, f"ordinals_not_complete_0_to_n_minus_1:{phase}"
    # Order by ordinal for stable mapping (array order must also match ordinals).
    ordered = sorted(rows, key=lambda s: s["ordinal"])
    for i, slot in enumerate(ordered):
        if slot["ordinal"] != i:
            return None, f"ordinal_sort_gap:{phase}"
    # Prefer native emission order already matching ordinals.
    for i, slot in enumerate(rows):
        if slot.get("ordinal") != i:
            return None, f"slots_not_ordinal_ordered:{phase}"
    return rows, None


def _extract_settings_match(settings: Any) -> tuple[Optional[dict], Optional[str]]:
    if not isinstance(settings, dict):
        return None, "settings_missing_or_not_object"
    out: dict[str, Any] = {}
    for field in _QUAL_SETTINGS_MATCH_FIELDS:
        if field not in settings:
            return None, f"settings_missing:{field}"
        val = settings[field]
        if val is None:
            return None, f"settings_null:{field}"
        if field in ('host', 'frontend', 'workload', 'render_policy_requested'):
            if not isinstance(val, str) or not val:
                return None, f'settings_type:{field}'
        elif field in ('port', 'n'):
            if type(val) is not int or val < 1 or (field == 'port' and val > 65535):
                return None, f'settings_type:{field}'
        elif type(val) is not bool:
            return None, f'settings_type:{field}'
        out[field] = val
    env = settings.get("env_flags_requested")
    if not isinstance(env, dict) or not env:
        return None, "env_flags_requested_missing"
    for flag in (
        "BOT_SCHEDULING_PROFILE",
        "BOT_RENDER_PROFILE",
        "BOT_GPU_COMPLETION_PROFILE",
        "BOT_RESPONSIVENESS_PROFILE",
        "BOT_RESPONSIVENESS_FINE",
    ):
        if flag not in env or type(env[flag]) is not bool:
            return None, f"env_flags_requested_invalid:{flag}"
    out["env_flags_requested"] = {k: env[k] for k in sorted(env) if type(env[k]) is bool}
    # Cache canonical path is a match key when available; unavailable is explicit gap.
    canon_ok = settings.get("cache_dir_canonical_available")
    if type(canon_ok) is not bool:
        return None, "cache_dir_canonical_available_invalid"
    if canon_ok:
        canon = settings.get("cache_dir_canonical")
        if not isinstance(canon, str) or not canon:
            return None, "cache_dir_canonical_missing"
        out["cache_dir_canonical"] = canon
        out["cache_dir_canonical_available"] = True
    else:
        # Do not invent a path; record unavailable marker as match key value.
        out["cache_dir_canonical"] = None
        out["cache_dir_canonical_available"] = False
        reason = settings.get("cache_dir_canonical_reason")
        if not isinstance(reason, str) or not reason:
            return None, "cache_dir_canonical_reason_missing"
        out["cache_dir_canonical_reason"] = reason
    # cache_content_hash is always null at boundary — never a trustable match key.
    if settings.get("cache_content_hash") is not None:
        return None, "cache_content_hash_unexpected_non_null"
    # Physical audio output stays unobserved; settings markers must not invent it.
    audio = settings.get("client_audio_actual")
    if isinstance(audio, dict) and audio.get("physical_output_available") is True:
        return None, "physical_audio_output_must_remain_unobserved"
    return out, None


def _wall_span_from_native_boundaries(
    start: dict, end: dict, meta: dict
) -> tuple[Optional[tuple[float, float]], Optional[str]]:
    """Prefer native elapsed_wall_bracket envelope (start.before → end.after)."""
    sb = start.get("elapsed_wall_bracket")
    eb = end.get("elapsed_wall_bracket")
    if not isinstance(sb, dict) or not isinstance(eb, dict):
        return None, "elapsed_wall_bracket_missing"
    s_before, s_after = sb.get("before_unix_s"), sb.get("after_unix_s")
    e_before, e_after = eb.get("before_unix_s"), eb.get("after_unix_s")
    vals = (s_before, s_after, e_before, e_after)
    if not all(_finite_number(v) for v in vals):
        return None, "elapsed_wall_bracket_nonfinite"
    s_before_f, s_after_f = float(s_before), float(s_after)
    e_before_f, e_after_f = float(e_before), float(e_after)
    if not (s_before_f <= s_after_f and e_before_f <= e_after_f):
        return None, "elapsed_wall_bracket_unordered"
    if (s_after_f - s_before_f) > _WALL_BRACKET_MAX_WIDTH_S or (e_after_f - e_before_f) > _WALL_BRACKET_MAX_WIDTH_S:
        return None, "elapsed_wall_bracket_too_wide"
    obs_start, obs_end = s_before_f, e_after_f
    if not (obs_end > obs_start):
        return None, "wall_span_non_positive"
    started = meta.get("started_unix")
    ended = meta.get("ended_unix")
    if _finite_number(started) and _finite_number(ended):
        if obs_start < float(started) - 1e-3 or obs_end > float(ended) + 1e-3:
            return None, "wall_span_outside_launcher_envelope"
        if not (float(ended) > float(started)):
            return None, "launcher_time_envelope_invalid"
    se, ee = start.get("elapsed_s"), end.get("elapsed_s")
    if not (_finite_number(se) and _finite_number(ee)):
        return None, "boundary_elapsed_s_invalid"
    se_f, ee_f = float(se), float(ee)
    if ee_f < se_f:
        return None, "boundary_elapsed_s_decreasing"
    elapsed_d = ee_f - se_f
    wall_lo = e_before_f - s_after_f
    wall_hi = e_after_f - s_before_f
    if elapsed_d - wall_hi > _WALL_BRACKET_TOL_S:
        return None, "elapsed_exceeds_wall_bracket_envelope"
    if wall_lo - elapsed_d > _WALL_BRACKET_TOL_S:
        return None, "wall_bracket_envelope_inconsistent_with_elapsed"
    return (obs_start, obs_end), None


def consume_native_qualification(
    run_dir: pathlib.Path | str,
    *,
    n: Any,
    meta: Optional[dict] = None,
) -> dict:
    """Validate hash-bound samples.qualification.jsonl native ordinals/settings.

    Cross-run identity is ordinal only. Run-local names/slot ids must be unique
    and stable start→end within a run, but are never equality-matched across runs.
    """
    out: dict[str, Any] = {
        "status": "unavailable",
        "reason": None,
        "slot_ordinals_present": False,
        "ordinal_mapping": None,
        "match_keys": None,
        "observation_wall_span": None,
        "observation_wall_span_source": None,
        "physical_audio_output": "unobserved",
        "cache_content_hash": None,
        "cache_content_hash_note": "null_at_boundary; fingerprint integration pending",
        "overhead": "unavailable_until_process_evidence_integrated",
    }
    if type(n) is not int or isinstance(n, bool) or n < 1:
        out["reason"] = "n_invalid"
        return out
    meta = meta if isinstance(meta, dict) else {}
    rows, err = _load_qualification_boundaries(canonical_path(run_dir))
    if err is not None or rows is None:
        out["reason"] = err or "qualification_unavailable"
        return out
    if not rows:
        out["reason"] = "qualification_empty"
        return out

    # Exactly one ordered observe-start then observe-end (no extras).
    phases = [r.get("phase") for r in rows]
    if phases != ["observe-start", "observe-end"]:
        # Allow only those two phases present once each in order.
        if phases.count("observe-start") != 1 or phases.count("observe-end") != 1:
            out["reason"] = "qualification_phase_count_invalid"
            return out
        start_i = phases.index("observe-start")
        end_i = phases.index("observe-end")
        if start_i >= end_i or len(rows) != 2:
            out["reason"] = "qualification_phase_order_or_extra_rows"
            return out
    start, end = rows[0], rows[1]
    if start.get("phase") != "observe-start" or end.get("phase") != "observe-end":
        out["reason"] = "qualification_phase_order_or_extra_rows"
        return out

    start_slots, serr = _validate_slot_rows(start.get("slots"), n=n, phase="observe-start")
    if serr or start_slots is None:
        out["reason"] = serr
        return out
    end_slots, eerr = _validate_slot_rows(end.get("slots"), n=n, phase="observe-end")
    if eerr or end_slots is None:
        out["reason"] = eerr
        return out

    # Within-run stable mapping: name + instrumentation ids equal at each ordinal.
    ordinal_mapping: list[dict[str, Any]] = []
    for i in range(n):
        s_slot, e_slot = start_slots[i], end_slots[i]
        if s_slot.get("name") != e_slot.get("name"):
            out["reason"] = f"slot_name_changed_across_boundaries:{i}"
            return out
        if s_slot.get("responsiveness_slot_id") != e_slot.get("responsiveness_slot_id"):
            out["reason"] = f"responsiveness_slot_id_changed:{i}"
            return out
        if s_slot.get("cadence_slot_id") != e_slot.get("cadence_slot_id"):
            out["reason"] = f"cadence_slot_id_changed:{i}"
            return out
        ordinal_mapping.append(
            {
                "ordinal": i,
                # Run-local identity only — exposed for within-run mapping, not cross-run keys.
                "name": s_slot["name"],
                "responsiveness_slot_id": s_slot["responsiveness_slot_id"],
                "cadence_slot_id": s_slot["cadence_slot_id"],
            }
        )

    start_settings, sserr = _extract_settings_match(start.get("settings"))
    if sserr or start_settings is None:
        out["reason"] = f"observe_start_settings:{sserr}"
        return out
    end_settings, eserr = _extract_settings_match(end.get("settings"))
    if eserr or end_settings is None:
        out["reason"] = f"observe_end_settings:{eserr}"
        return out
    if start_settings != end_settings:
        out["reason"] = "runtime_config_changed_between_observe_start_and_end"
        out["settings_start"] = start_settings
        out["settings_end"] = end_settings
        return out
    if start_settings['n'] != n:
        out['reason'] = 'native_settings_n_mismatch'
        return out
    for field in ('frontend', 'workload'):
        if field in meta and start_settings[field] != meta[field]:
            out['reason'] = 'native_settings_metadata_mismatch:' + field
            return out

    render_enabled = start_settings.get("render_profile_enabled") is True
    if type(start_settings.get("render_profile_enabled")) is not bool:
        out["reason"] = "render_profile_enabled_not_bool"
        return out

    slot_runtime_keys: list[dict[str, Any]] = []
    slot_renderer_keys: list[dict[str, Any]] = []
    for i in range(n):
        s_slot, e_slot = start_slots[i], end_slots[i]
        s_rs, rserr = _extract_runtime_settings_match(s_slot.get("runtime_settings"))
        if rserr or s_rs is None:
            out["reason"] = f"observe_start_runtime_settings:{i}:{rserr}"
            return out
        e_rs, rerr = _extract_runtime_settings_match(e_slot.get("runtime_settings"))
        if rerr or e_rs is None:
            out["reason"] = f"observe_end_runtime_settings:{i}:{rerr}"
            return out
        if s_rs != e_rs:
            out["reason"] = f"runtime_settings_changed_across_boundaries:{i}"
            return out
        slot_runtime_keys.append({"ordinal": i, **s_rs})

        s_ren, renerr = _extract_renderer_match(s_slot, profile_enabled=render_enabled)
        if renerr or s_ren is None:
            out["reason"] = f"observe_start_renderer:{i}:{renerr}"
            return out
        e_ren, reerr = _extract_renderer_match(e_slot, profile_enabled=render_enabled)
        if reerr or e_ren is None:
            out["reason"] = f"observe_end_renderer:{i}:{reerr}"
            return out
        if s_ren != e_ren:
            out["reason"] = f"renderer_config_changed_across_boundaries:{i}"
            return out
        slot_renderer_keys.append({"ordinal": i, **s_ren})

    wall_span, wall_err = _wall_span_from_native_boundaries(start, end, meta)
    if wall_err or wall_span is None:
        out["reason"] = wall_err
        return out

    match_keys = {
        "qualification_settings": start_settings,
        # Ordered by ordinal — cross-run identity is ordinal, not name/id.
        "slot_runtime_settings_by_ordinal": slot_runtime_keys,
        "renderer_config_by_ordinal": slot_renderer_keys,
    }
    out.update(
        {
            "status": "available",
            "reason": None,
            "slot_ordinals_present": True,
            "ordinal_mapping": ordinal_mapping,
            "match_keys": match_keys,
            "observation_wall_span": list(wall_span),
            "observation_wall_span_source": "native_elapsed_wall_bracket",
        }
    )
    return out


def _slot_ordinals_present_from_native(native: dict) -> bool:
    return native.get("status") == "available" and native.get("slot_ordinals_present") is True


def _evaluate_helper_overhead(*, receipt: dict, cell_dir: pathlib.Path) -> dict:
    """Overhead stays unavailable without continuous helper accounting.

    Snapshot before/after files and receipt.sampler.overhead labels are not
    sufficient. Do not invent a measured status. Root process_evidence module
    is separate subsequent work.
    """
    sampler = receipt.get("sampler") if isinstance(receipt.get("sampler"), dict) else {}
    label = sampler.get("overhead")
    before = cell_dir / "helper_resources_before.json"
    after = cell_dir / "helper_resources_after.json"
    has_snapshots = before.is_file() and after.is_file()
    continuous = False
    return {
        "status": "unavailable",
        "reason": "helper_overhead_accounting_missing",
        "sampler_label": label,
        "has_helper_snapshots": has_snapshots,
        "continuous_helper_series": continuous,
        "measured": False,
        "note": (
            "Real overhead requires declared OFF/ON/ON/OFF cells plus continuous "
            "helper/process/server accounting; snapshots and labels are not enough. "
            "process_evidence integration is separate and pending."
        ),
    }


def bind_pair(
    *,
    reference: dict,
    candidate: dict,
) -> dict:
    """Combine two bind_side results into a pair assessment.

    Distinguishes binding success from final pair-gate eligibility.
    No caller bypasses for incomplete match keys or N>1 without ordinals.
    Cross-run slot identity is ordinal only when native qualification is present.
    """
    out: dict[str, Any] = {
        "status": "unavailable",
        "reason": None,
        "pass_means": PASS_MEANS,
        "final_acceptance_claim": False,
        "binding_ok": False,
        "pair_eligible": False,
        "reference": reference,
        "candidate": candidate,
        "preserved_cells": [],
        "compare_matched_runs": None,
        "paired_fine_p99_margin": None,
    }

    if not isinstance(reference, dict) or not isinstance(candidate, dict):
        out["reason"] = "missing_side_binding"
        return out

    if not reference.get("binding_ok") or not candidate.get("binding_ok"):
        out["reason"] = "side_binding_failed"
        out["reference_reason"] = reference.get("reason")
        out["candidate_reason"] = candidate.get("reason")
        return out

    # Recorded raw-hash binding required on both sides.
    if reference.get("raw_hash_status") != "receipt_recorded_and_verified":
        out["reason"] = "reference_raw_hash_not_receipt_bound"
        out["reference_raw_hash_status"] = reference.get("raw_hash_status")
        return out
    if candidate.get("raw_hash_status") != "receipt_recorded_and_verified":
        out["reason"] = "candidate_raw_hash_not_receipt_bound"
        out["candidate_raw_hash_status"] = candidate.get("raw_hash_status")
        return out

    out["binding_ok"] = True

    if not reference.get("qualified") or not candidate.get("qualified"):
        out["reason"] = "side_not_qualified"
        return out

    ref_id = (reference.get("run_identity") or {}).get("identity_sha256")
    cand_id = (candidate.get("run_identity") or {}).get("identity_sha256")
    if not ref_id or not cand_id:
        out["reason"] = "missing_run_identity"
        return out
    if ref_id == cand_id:
        out["reason"] = "duplicate_run_identity"
        return out
    if reference.get("run_dir") == candidate.get("run_dir"):
        out["reason"] = "duplicate_run_dir"
        return out

    ref_span = reference.get("observation_wall_span")
    cand_span = candidate.get("observation_wall_span")
    if not (isinstance(ref_span, (list, tuple)) and isinstance(cand_span, (list, tuple))):
        out["reason"] = "observation_window_unavailable"
        return out
    if len(ref_span) != 2 or len(cand_span) != 2:
        out["reason"] = "observation_window_unavailable"
        return out
    if _windows_overlap(tuple(ref_span), tuple(cand_span)):  # type: ignore[arg-type]
        out["reason"] = "overlapping_observation_windows"
        out["reference_span"] = ref_span
        out["candidate_span"] = cand_span
        return out

    ref_keys = reference.get("match_keys") or {}
    cand_keys = candidate.get("match_keys") or {}
    miss_r = match_keys_complete(ref_keys)
    miss_c = match_keys_complete(cand_keys)
    if miss_r or miss_c:
        out["reason"] = "missing_match_key"
        out["reference_missing_match_key"] = miss_r
        out["candidate_missing_match_key"] = miss_c
        return out

    if ref_keys != cand_keys:
        diff = sorted(
            k
            for k in set(ref_keys) | set(cand_keys)
            if ref_keys.get(k) != cand_keys.get(k)
        )
        out["reason"] = "match_key_mismatch"
        out["differing_keys"] = diff
        return out

    n_ref = reference.get("n")
    n_cand = candidate.get("n")
    if n_ref != n_cand:
        out["reason"] = "n_mismatch"
        return out

    ref_native = reference.get("native_qualification") or {}
    cand_native = candidate.get("native_qualification") or {}
    ref_nat_ok = ref_native.get("status") == "available"
    cand_nat_ok = cand_native.get("status") == "available"

    if isinstance(n_ref, int) and n_ref > 1:
        if not (
            reference.get("slot_ordinals_present") and candidate.get("slot_ordinals_present")
        ):
            out["reason"] = "missing_stable_slot_ordinals"
            out["reference_native_reason"] = ref_native.get("reason")
            out["candidate_native_reason"] = cand_native.get("reason")
            return out
        if not (ref_nat_ok and cand_nat_ok):
            out["reason"] = "native_qualification_unavailable"
            out["reference_native_reason"] = ref_native.get("reason")
            out["candidate_native_reason"] = cand_native.get("reason")
            return out

    # When either side has validated native qualification, both must and match keys equal.
    if ref_nat_ok or cand_nat_ok:
        if not (ref_nat_ok and cand_nat_ok):
            out["reason"] = "native_qualification_asymmetric"
            out["reference_native_reason"] = ref_native.get("reason")
            out["candidate_native_reason"] = cand_native.get("reason")
            return out
        ref_nm = ref_native.get("match_keys")
        cand_nm = cand_native.get("match_keys")
        if not isinstance(ref_nm, dict) or not isinstance(cand_nm, dict):
            out["reason"] = "native_match_keys_missing"
            return out
        if ref_nm != cand_nm:
            diff_n = sorted(
                k for k in set(ref_nm) | set(cand_nm) if ref_nm.get(k) != cand_nm.get(k)
            )
            out["reason"] = "native_match_key_mismatch"
            out["differing_native_keys"] = diff_n
            return out
        out["native_ordinal_mapping"] = {
            "reference": ref_native.get("ordinal_mapping"),
            "candidate": cand_native.get("ordinal_mapping"),
            "cross_run_identity": "ordinal",
            "note": "names and run-local slot ids are not cross-run match keys",
        }

    # Endpoint family notes: decode ≠ input ≠ GPU ≠ TUI flush.
    ref_ep = reference.get("endpoint_notes") or {}
    cand_ep = candidate.get("endpoint_notes") or {}
    for name in ("decode", "input", "gpu", "scheduling"):
        r = ref_ep.get(name) if isinstance(ref_ep.get(name), dict) else None
        c = cand_ep.get(name) if isinstance(cand_ep.get(name), dict) else None
        if r is None or c is None:
            # Missing endpoint semantics on either side → unavailable, not equal.
            out["reason"] = "endpoint_semantics_unavailable"
            out["endpoint"] = name
            return out
        if r.get("status") == "available" and c.get("status") != "available":
            out["reason"] = "endpoint_mismatch"
            out["endpoint"] = name
            return out
        if c.get("status") == "available" and r.get("status") != "available":
            out["reason"] = "endpoint_mismatch"
            out["endpoint"] = name
            return out

    ref_oh = reference.get("overhead") or {}
    cand_oh = candidate.get("overhead") or {}
    if ref_oh.get("measured") is not True or cand_oh.get("measured") is not True:
        out["reason"] = "overhead_unavailable"
        out["reference_overhead"] = ref_oh
        out["candidate_overhead"] = cand_oh
        shaped_r = reference.get("shaped_for_compare") or {}
        shaped_c = candidate.get("shaped_for_compare") or {}
        shaped_r = dict(shaped_r)
        shaped_c = dict(shaped_c)
        shaped_r["overhead"] = "unknown"
        shaped_c["overhead"] = "unknown"
        out["compare_matched_runs"] = rm.compare_matched_runs(shaped_c, shaped_r)
        out["status"] = "unavailable"
        out["pair_eligible"] = False
        out["note"] = (
            "Binding and match keys may succeed while pair eligibility stays "
            "unavailable without helper-overhead proof"
        )
        return out

    shaped_r = reference.get("shaped_for_compare") or {}
    shaped_c = candidate.get("shaped_for_compare") or {}
    out["compare_matched_runs"] = rm.compare_matched_runs(shaped_c, shaped_r)
    out["status"] = "unavailable"
    out["reason"] = "paired_acceptance_not_authorized"
    out["pair_eligible"] = False
    return out


def read_matched_pair(
    *,
    reference_receipt: pathlib.Path | str,
    candidate_receipt: pathlib.Path | str,
    manifest_path: pathlib.Path | str,
    server_identity_path: Optional[pathlib.Path | str] = None,
    host_conditions_path: Optional[pathlib.Path | str] = None,
    counting: bool = False,
    diagnostics: bool = False,
    preserve_cells: Optional[list[dict]] = None,
) -> dict:
    """High-level pair reader from explicit receipt paths + shared manifest."""
    ref = bind_side(
        reference_receipt,
        role="reference",
        manifest_path=manifest_path,
        counting=counting,
        diagnostics=diagnostics,
        server_identity_path=server_identity_path,
        host_conditions_path=host_conditions_path,
    )
    cand = bind_side(
        candidate_receipt,
        role="candidate",
        manifest_path=manifest_path,
        counting=counting,
        diagnostics=diagnostics,
        server_identity_path=server_identity_path,
        host_conditions_path=host_conditions_path,
    )
    pair = bind_pair(reference=ref, candidate=cand)
    if preserve_cells:
        pair["preserved_cells"] = list(preserve_cells)
    return pair


def preserve_failed_cell(
    *,
    cell_id: str,
    receipt_path: Optional[pathlib.Path | str] = None,
    reason: str,
    details: Optional[dict] = None,
) -> dict:
    """Record a failed/unrun cell so it is not silently dropped from a matrix."""
    out = {
        "cell_id": cell_id,
        "status": "preserved_failure",
        "reason": reason,
        "receipt_path": str(receipt_path) if receipt_path is not None else None,
        "final_acceptance_claim": False,
    }
    if details:
        out["details"] = details
    return out


__all__ = [
    "MATCH_KEY_FIELDS",
    "SIDE_PROVENANCE_FIELDS",
    "PASS_MEANS",
    "bind_pair",
    "bind_side",
    "canonical_path",
    "construct_match_keys",
    "construct_side_provenance",
    "consume_native_qualification",
    "match_keys_complete",
    "preserve_failed_cell",
    "read_matched_pair",
    "sha256_file",
]
