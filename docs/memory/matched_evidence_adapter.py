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
import json
import pathlib
import sys
from typing import Any, Optional

ROOT = pathlib.Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import qualify_control as qc  # noqa: E402
import reference_metrics as rm  # noqa: E402

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
    "sampler_interval_s",
)

# Present and role-correct; may differ across reference vs candidate.
SIDE_PROVENANCE_FIELDS = (
    "role",
    "binary_path",
    "binary_sha256",
    "manifest_binary_key",
    "manifest_build_commit",
    "manifest_sources_sha256",
    "manifest_branch",
    "client_commit",
    "host_commit_checkout",  # current checkout label only; not build authority
)

REQUIRED_RUN_FILES = (
    "metadata.json",
    "samples.jsonl",
    "samples.qualification.jsonl",
)

# Manifest binary key by (role, frontend)
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


def _observation_wall_span(meta: dict, samples: list[dict]) -> Optional[tuple[float, float]]:
    """Return (start_unix, end_unix) for the observation window when possible.

    Prefer host wall timestamps from metadata; fall back to started_unix +
    sample elapsed_s offsets. Process-local mono origins are never compared
    across runs.
    """
    started = meta.get("started_unix")
    ended = meta.get("ended_unix")
    if isinstance(started, (int, float)) and isinstance(ended, (int, float)):
        if ended > started:
            return (float(started), float(ended))
    observe = [
        row
        for row in samples
        if isinstance(row, dict) and row.get("phase") == "observe"
    ]
    if (
        isinstance(started, (int, float))
        and len(observe) >= 2
        and all(isinstance(r.get("elapsed_s"), (int, float)) for r in observe)
    ):
        t0 = float(observe[0]["elapsed_s"])
        t1 = float(observe[-1]["elapsed_s"])
        if t1 > t0:
            return (float(started) + t0, float(started) + t1)
    return None


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
    """Build strict match-key dict. Missing/null members stay present as None."""
    out: dict[str, Any] = {}
    for key in MATCH_KEY_FIELDS:
        if key in (
            "server_start_identity",
            "server_port_listen",
            "sampler_interval_s",
        ):
            continue
        val = meta.get(key)
        if key == "render_policy" and val is None and meta.get("frontend") == "tui":
            val = "none"
        out[key] = val
    if extras:
        for key in (
            "server_start_identity",
            "server_port_listen",
            "sampler_interval_s",
        ):
            if key in extras:
                out[key] = extras[key]
    return out


def match_keys_complete(keys: dict) -> Optional[str]:
    """Return missing field name if any match key is null/empty; else None."""
    for key in MATCH_KEY_FIELDS:
        if key not in keys:
            return key
        if _is_missing(keys.get(key)):
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
    return {
        "role": role,
        "binary_path": str(binary_path),
        "binary_sha256": binary_sha256,
        "manifest_binary_key": manifest_key,
        "manifest_build_commit": _build_commit_from_side(manifest_side),
        "manifest_sources_sha256": manifest_side.get("sources_sha256_pre")
        or manifest_side.get("sources_sha256_post"),
        "manifest_branch": manifest_side.get("branch"),
        "client_commit": client.get("commit") or meta.get("client_commit"),
        "host_commit_checkout": meta.get("host_commit"),
        # Explicit: checkout host_commit is not build authority.
        "host_commit_is_build_authority": False,
    }


def bind_side(
    receipt_path: pathlib.Path | str,
    *,
    role: str,
    manifest_path: pathlib.Path | str,
    counting: bool = False,
    diagnostics: bool = False,
    server_identity_path: Optional[pathlib.Path | str] = None,
    cell_dir: Optional[pathlib.Path | str] = None,
) -> dict:
    """Bind one cell receipt through the authoritative artifact chain.

    Returns a structured dict. ``binding_ok`` means receipt→metadata→binary→
    manifest→raw files chained successfully and qualification was recomputed.
    ``pair_eligible`` is never set True here (pair checks are separate);
    overhead remains unavailable without continuous helper accounting.
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
        receipt = _load_json(receipt_path)
    except (OSError, json.JSONDecodeError) as exc:
        return _unavailable("receipt_unreadable", error=str(exc), path=str(receipt_path))
    if not isinstance(receipt, dict):
        return _unavailable("receipt_not_object", path=str(receipt_path))

    run_dir_raw = receipt.get("run_dir")
    if _is_missing(run_dir_raw):
        return _unavailable("receipt_missing_run_dir", path=str(receipt_path))
    run_dir = canonical_path(run_dir_raw)
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

    meta_run = meta.get("run_dir")
    if not _is_missing(meta_run):
        if canonical_path(meta_run) != run_dir:
            return _unavailable(
                "receipt_metadata_run_dir_mismatch",
                receipt_run_dir=str(run_dir),
                metadata_run_dir=str(canonical_path(meta_run)),
            )

    receipt_exit = receipt.get("exit_code")
    meta_exit = meta.get("exit_code")
    if receipt_exit is not None and meta_exit is not None and receipt_exit != meta_exit:
        return _unavailable(
            "receipt_metadata_exit_mismatch",
            receipt_exit=receipt_exit,
            metadata_exit=meta_exit,
        )

    raw_hashes = {
        "metadata.json": _file_hash_if_present(meta_path),
        "samples.jsonl": _file_hash_if_present(run_dir / "samples.jsonl"),
        "samples.qualification.jsonl": _file_hash_if_present(
            run_dir / "samples.qualification.jsonl"
        ),
    }
    if any(v is None for v in raw_hashes.values()):
        return _unavailable("raw_hash_failed", path=str(run_dir), raw_hashes=raw_hashes)

    # Completed run identity = canonical run dir + completed artifact hashes.
    run_identity = {
        "run_dir": str(run_dir),
        "raw_hashes": raw_hashes,
        "identity_sha256": sha256_bytes(
            json.dumps(
                {"run_dir": str(run_dir), "raw_hashes": raw_hashes},
                sort_keys=True,
            ).encode()
        ),
    }

    binary_raw = meta.get("binary") or receipt.get("binary")
    if _is_missing(binary_raw):
        return _unavailable("missing_binary_path", run_dir=str(run_dir))
    binary_path = canonical_path(binary_raw)
    if not binary_path.is_file():
        return _unavailable("binary_missing", path=str(binary_path))

    try:
        actual_sha = sha256_file(binary_path)
    except OSError as exc:
        return _unavailable("binary_unreadable", path=str(binary_path), error=str(exc))

    meta_sha = meta.get("binary_sha256")
    if _is_missing(meta_sha):
        return _unavailable("metadata_missing_binary_sha256", run_dir=str(run_dir))
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
    if _is_missing(frontend):
        return _unavailable("metadata_missing_frontend", run_dir=str(run_dir))

    # Role key: treat reference and control as the control manifest side.
    role_for_key = "control" if role_n in ("reference", "control") else "candidate"
    manifest_key = _MANIFEST_BINARY_KEYS.get((role_for_key, str(frontend)))
    if manifest_key is None:
        # Also try the literal role name for reference→control_tui mapping
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
    if entry_path != binary_path and entry_sha != actual_sha:
        # Paths may differ only if content hash still matches the named entry.
        return _unavailable(
            "binary_not_bound_to_manifest_entry",
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
    if entry_path != binary_path:
        # Same hash, different path — require the named manifest path to exist
        # and hash-match (duplicate reference ok when content matches).
        if not entry_path.is_file():
            return _unavailable(
                "manifest_binary_path_missing",
                path=str(entry_path),
                key=manifest_key,
            )
        try:
            entry_actual = sha256_file(entry_path)
        except OSError as exc:
            return _unavailable(
                "manifest_binary_unreadable",
                path=str(entry_path),
                error=str(exc),
            )
        if entry_actual != entry_sha:
            return _unavailable(
                "manifest_binary_file_hash_mismatch",
                path=str(entry_path),
                expected=entry_sha,
                actual=entry_actual,
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
    if _is_missing(side_prov.get("manifest_sources_sha256")):
        return _unavailable(
            "manifest_sources_sha256_missing",
            role=role_for_key,
            side_provenance=side_prov,
        )

    # Server / sampler identity (non-sensitive only).
    server_extras: dict[str, Any] = {}
    if server_identity_path is not None:
        sip = canonical_path(server_identity_path)
        if sip.is_file():
            try:
                sid = _load_json(sip)
            except (OSError, json.JSONDecodeError) as exc:
                return _unavailable(
                    "server_identity_unreadable",
                    path=str(sip),
                    error=str(exc),
                )
            if isinstance(sid, dict):
                server_extras["server_start_identity"] = sid.get("start_identity")
                server_extras["server_port_listen"] = sid.get("port_listen")
        else:
            return _unavailable("server_identity_missing", path=str(sip))

    sampler = receipt.get("sampler") if isinstance(receipt.get("sampler"), dict) else {}
    if "interval_s" in sampler:
        server_extras["sampler_interval_s"] = sampler.get("interval_s")

    # Catalog / nav fixture hashes may live only on the manifest today.
    features = manifest.get("features") if isinstance(manifest.get("features"), dict) else {}
    catalog = manifest.get("catalog") if isinstance(manifest.get("catalog"), dict) else {}
    nav = manifest.get("nav") if isinstance(manifest.get("nav"), dict) else {}

    # Enrich meta copy for match-key construction without mutating on-disk meta.
    meta_view = dict(meta)
    if _is_missing(meta_view.get("nav_flags_sha256")) and not _is_missing(nav.get("nav_flags_sha256")):
        meta_view["nav_flags_sha256"] = nav.get("nav_flags_sha256")
    if _is_missing(meta_view.get("catalog_sha256")) and not _is_missing(
        catalog.get("js_scripts_json_sha256")
    ):
        meta_view["catalog_sha256"] = catalog.get("js_scripts_json_sha256")
    if _is_missing(meta_view.get("feature_flags")) and features:
        meta_view["feature_flags"] = {
            "requested": features.get("requested"),
            "locked": features.get("locked"),
            "allocation_counting": features.get("allocation_counting"),
        }
    if _is_missing(meta_view.get("allocator_provenance")) and features.get("allocator"):
        meta_view["allocator_provenance"] = features.get("allocator")
    if _is_missing(meta_view.get("client_sources_sha256")):
        client = manifest_side.get("client") if isinstance(manifest_side.get("client"), dict) else {}
        if client.get("sources_sha256"):
            meta_view["client_sources_sha256"] = client.get("sources_sha256")
    # renderer_settings / cache_settings: still required when pairing; do not invent.

    match_keys = construct_match_keys(meta_view, extras=server_extras)

    # Independent qualification (never trust receipt.qualification labels).
    qualification = qc.qualify(run_dir, counting=counting, diagnostics=diagnostics)
    analysis = rm.analyze_run(
        run_dir,
        qualify=True,
        counting=counting,
        diagnostics=diagnostics,
    )

    # Observation window (wall clock) for pair overlap checks.
    try:
        samples = rm.load_run_samples(run_dir)
    except (OSError, ValueError, FileNotFoundError) as exc:
        return _unavailable("samples_unreadable", error=str(exc), path=str(run_dir))

    wall_span = _observation_wall_span(meta, samples)

    # Helper overhead: snapshots alone never prove continuous accounting.
    overhead_status = _evaluate_helper_overhead(
        receipt=receipt,
        cell_dir=canonical_path(cell_dir) if cell_dir is not None else receipt_path.parent,
    )

    qualified = qualification.get("qualified") is True and not qualification.get("errors")
    exit_ok = meta_exit == 0
    binding_ok = True  # chain above succeeded

    # Resource-shaped diagnostic payload for existing helpers (not pair pass).
    resources_gate = (analysis.get("gates") or {}).get("resources") or {}
    shaped = {
        "status": resources_gate.get("status", "unavailable"),
        "reason": resources_gate.get("reason"),
        "match_metadata": rm.resource_match_keys_from_meta(meta_view),
        "side_provenance": {
            "binary_sha256": actual_sha,
            "host_sources_sha256": side_prov.get("manifest_sources_sha256"),
            "manifest_build_commit": side_prov.get("manifest_build_commit"),
            "role": side_prov.get("role"),
        },
        "overhead": "unknown",  # never promote from receipt labels
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
    continuous = False  # no continuous helper CPU/RSS series in current artifacts
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
    require_match_keys_complete: bool = True,
    allow_n_gt1_without_ordinals: bool = False,
) -> dict:
    """Combine two bind_side results into a pair assessment.

    Distinguishes binding success from final pair-gate eligibility.
    Diagnostic compare_matched_runs / paired_fine_p99 may still run under
    status=unavailable; they never unlock acceptance.
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
    if _windows_overlap(tuple(ref_span), tuple(cand_span)):  # type: ignore[arg-type]
        out["reason"] = "overlapping_observation_windows"
        out["reference_span"] = ref_span
        out["candidate_span"] = cand_span
        return out

    ref_keys = reference.get("match_keys") or {}
    cand_keys = candidate.get("match_keys") or {}
    if require_match_keys_complete:
        miss_r = match_keys_complete(ref_keys)
        miss_c = match_keys_complete(cand_keys)
        if miss_r or miss_c:
            out["reason"] = "missing_match_key"
            out["reference_missing_match_key"] = miss_r
            out["candidate_missing_match_key"] = miss_c
            return out

    if ref_keys != cand_keys:
        # Identify first differing key for diagnostics.
        diff = sorted(
            k
            for k in set(ref_keys) | set(cand_keys)
            if ref_keys.get(k) != cand_keys.get(k)
        )
        out["reason"] = "match_key_mismatch"
        out["differing_keys"] = diff
        return out

    # N>1 requires ordinals or predeclared fleet-worst-case (not implemented).
    n_ref = reference.get("n")
    n_cand = candidate.get("n")
    if n_ref != n_cand:
        out["reason"] = "n_mismatch"
        return out
    if isinstance(n_ref, int) and n_ref > 1:
        if not allow_n_gt1_without_ordinals and not (
            reference.get("slot_ordinals_present") and candidate.get("slot_ordinals_present")
        ):
            out["reason"] = "missing_stable_slot_ordinals"
            return out

    # Endpoint family notes: decode ≠ input ≠ GPU ≠ TUI flush. Require same
    # availability class when both sides report endpoint gate dicts.
    ref_ep = reference.get("endpoint_notes") or {}
    cand_ep = candidate.get("endpoint_notes") or {}
    for name in ("decode", "input", "gpu", "scheduling"):
        r = ref_ep.get(name) if isinstance(ref_ep.get(name), dict) else None
        c = cand_ep.get(name) if isinstance(cand_ep.get(name), dict) else None
        if r is None or c is None:
            continue
        # Mismatch only when one side has available and the other does not with
        # a different reason class that implies different endpoint semantics.
        if r.get("status") == "available" and c.get("status") != "available":
            out["reason"] = "endpoint_mismatch"
            out["endpoint"] = name
            return out
        if c.get("status") == "available" and r.get("status") != "available":
            out["reason"] = "endpoint_mismatch"
            out["endpoint"] = name
            return out

    # Overhead: both sides must have measured continuous helper accounting.
    ref_oh = reference.get("overhead") or {}
    cand_oh = candidate.get("overhead") or {}
    if ref_oh.get("measured") is not True or cand_oh.get("measured") is not True:
        out["reason"] = "overhead_unavailable"
        out["reference_overhead"] = ref_oh
        out["candidate_overhead"] = cand_oh
        # Still emit diagnostic compare under unavailable eligibility.
        shaped_r = reference.get("shaped_for_compare") or {}
        shaped_c = candidate.get("shaped_for_compare") or {}
        # Force overhead unknown so compare_matched_runs stays closed.
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

    # Even with measured overhead, this reader still does not claim acceptance.
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
    )
    cand = bind_side(
        candidate_receipt,
        role="candidate",
        manifest_path=manifest_path,
        counting=counting,
        diagnostics=diagnostics,
        server_identity_path=server_identity_path,
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
