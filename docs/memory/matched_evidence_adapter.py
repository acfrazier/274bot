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
    "server_start_identity",
    "server_port_listen",
    "server_configuration",
    "sampler_interval_s",
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


def _typed_equal(left, right):
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(_typed_equal(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(_typed_equal(a, b) for a, b in zip(left, right))
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
    for key in MATCH_KEY_FIELDS:
        if key in (
            "server_start_identity",
            "server_port_listen",
            "server_configuration",
            "sampler_interval_s",
            "sampler_duration_s_requested",
            "host_conditions",
        ):
            continue
        out[key] = meta.get(key)
    if extras:
        for key in (
            "server_start_identity",
            "server_port_listen",
            "server_configuration",
            "sampler_interval_s",
            "sampler_duration_s_requested",
            "host_conditions",
        ):
            if key in extras:
                out[key] = extras[key]
    return out


def match_keys_complete(keys: dict) -> Optional[str]:
    """Return missing field name if any match key is null/empty/nested-null."""
    for key in MATCH_KEY_FIELDS:
        if key not in keys:
            return key
        if _deep_missing(keys.get(key)):
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

    # Reuse the reviewed launcher verifier instead of duplicating a weaker
    # manifest/fixture implementation. It verifies real canonical file paths.
    verified_build = bp.verify_build(manifest_path, receipt['_binding_role'], meta['frontend'],
                                     meta['binary'], meta.get('nav_pack'), meta.get('nav_flags'), meta.get('catalog_path'))
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

    sampler = receipt.get("sampler") if isinstance(receipt.get("sampler"), dict) else {}
    if _is_missing(sampler.get("interval_s")):
        return _unavailable("receipt_sampler_interval_missing")
    if _is_missing(sampler.get("duration_s_requested")):
        return _unavailable("receipt_sampler_duration_missing")
    for field in ('interval_s', 'duration_s_requested'):
        if type(sampler[field]) not in (int, float) or not math.isfinite(sampler[field]) or sampler[field] <= 0:
            return _unavailable('receipt_sampler_configuration_invalid', field=field)
    server_extras["sampler_interval_s"] = sampler.get("interval_s")
    server_extras["sampler_duration_s_requested"] = sampler.get("duration_s_requested")

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
        if not isinstance(hc, dict) or _deep_missing(hc):
            return _unavailable("host_conditions_invalid", path=str(hcp))
        server_extras["host_conditions"] = hc
    elif isinstance(meta.get("host_conditions"), dict) and not _deep_missing(meta.get("host_conditions")):
        server_extras["host_conditions"] = meta.get("host_conditions")
    elif isinstance(receipt.get("host_conditions"), dict) and not _deep_missing(receipt.get("host_conditions")):
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
    recheck = {
        "metadata.json": _file_hash_if_present(meta_path),
        "samples.jsonl": _file_hash_if_present(run_dir / "samples.jsonl"),
        "samples.qualification.jsonl": _file_hash_if_present(
            run_dir / "samples.qualification.jsonl"
        ),
    }
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

    wall_span = _observation_wall_span_from_analysis(analysis, meta)
    if wall_span is None:
        return _unavailable(
            "observation_window_invalid",
            observation_window=analysis.get("observation_window") if isinstance(analysis, dict) else None,
        )

    overhead_status = _evaluate_helper_overhead(
        receipt=receipt,
        cell_dir=canonical_path(cell_dir) if cell_dir is not None else receipt_path.parent,
    )

    try:
        samples = rm.load_run_samples(run_dir)
    except (OSError, ValueError, FileNotFoundError) as exc:
        return _unavailable("samples_unreadable", error=str(exc), path=str(run_dir))
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
        "overhead": overhead_status,
        "shaped_for_compare": shaped,
        "endpoint_notes": {
            "scheduling": (analysis.get("gates") or {}).get("scheduling"),
            "decode": (analysis.get("gates") or {}).get("decode"),
            "input": (analysis.get("gates") or {}).get("input"),
            "gpu": (analysis.get("gates") or {}).get("gpu"),
        },
        "n": meta.get("n"),
        "slot_ordinals_present": _slot_ordinals_present(samples, meta),
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


def _slot_ordinals_present(samples: list[dict], meta: dict) -> bool:
    """Stable fixture/ordinal mapping across runs is not yet emitted by launcher."""
    del samples, meta
    return False


def _evaluate_helper_overhead(*, receipt: dict, cell_dir: pathlib.Path) -> dict:
    """Overhead stays unavailable without continuous helper accounting.

    Snapshot before/after files and receipt.sampler.overhead labels are not
    sufficient. Do not invent a measured status.
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
            "helper/process/server accounting; snapshots and labels are not enough"
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
    if isinstance(n_ref, int) and n_ref > 1:
        if not (
            reference.get("slot_ordinals_present") and candidate.get("slot_ordinals_present")
        ):
            out["reason"] = "missing_stable_slot_ordinals"
            return out

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
    "match_keys_complete",
    "preserve_failed_cell",
    "read_matched_pair",
    "sha256_file",
]
