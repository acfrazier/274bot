#!/usr/bin/env python3
"""Focused tests for matched_evidence_adapter (stdlib only).

Exercises the production reader + real qualify_control / analyze_run paths.
Distinguishes binding success from final pair-gate eligibility.
"""
from __future__ import annotations

import hashlib
import datetime
import json
import copy
import pathlib
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import matched_evidence_adapter as mea  # noqa: E402
import reference_metrics as rm  # noqa: E402

MANIFEST = ROOT / "shared-nav-build-manifest.json"
N1_BATCH = ROOT / "diagnostics" / "shared-nav-clean-screen-20260907T004029Z"
N1_CONTROL_RECEIPT = N1_BATCH / "cell_01_control_n1_profiles_on" / "receipt.json"
N1_CANDIDATE_RECEIPT = N1_BATCH / "cell_02_candidate_n1_profiles_on" / "receipt.json"
N1_N16_RECEIPT = N1_BATCH / "cell_03_candidate_n16_profiles_on" / "receipt.json"
N1_SERVER = N1_BATCH / "server_identity.json"


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _tag(label: str) -> str:
    """Stable 64-hex digest used as realistic hash/commit stand-in."""
    return _sha(label.encode("utf-8"))


def _write_json(path: pathlib.Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2) + "\n")


def _write_jsonl(path: pathlib.Path, rows) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows))


def _active_samples(n: int = 1, observe_s: int = 30, t0: float = 5.0):
    """Observe samples with elapsed_s offset so wall span sits inside process envelope."""
    rows = [
        dict(
            phase="observe",
            elapsed_s=t0 + t,
            ready=n,
            active=n,
            allocation_counting=False,
            diagnostic_sidecar=False,
            rust_allocations=None,
            rust_allocated_bytes=None,
            rust_live_bytes=None,
            process_cpu_user_s=t / 2,
            process_cpu_system_s=t / 4,
            client_tick_count=int(t * 50),
            resident_bytes=100 + int(t),
            peak_resident_bytes=200,
            v8_live_isolates=1,
            snapshot_inflight_bytes=0,
            snapshot_inflight_capacity=0,
        )
        for t in (0.0, observe_s / 2.0, float(observe_s))
    ]
    proof = []
    for phase, count in (("observe-start", 2), ("observe-end", 7)):
        slots = [
            dict(
                name=f"bot{i}" if n > 1 else "bot",
                state="Running",
                error=None,
                runtime={"paint": {"lines": [f"Steals: {count}"]}},
            )
            for i in range(n)
        ]
        proof.append(dict(phase=phase, slots=slots))
    return rows, proof


def _runtime_settings(*, lowmem=False, midi_active=False, midi_volume=0,
                      wave_enabled=False, wave_volume=0, draw=True, loop_cycle=1):
    return {
        "lowmem": lowmem,
        "midi_active": midi_active,
        "midi_volume": midi_volume,
        "wave_enabled": wave_enabled,
        "wave_volume": wave_volume,
        "draw": draw,
        "loop_cycle": loop_cycle,
        "loop_cycle_meaning": "client_mainloop_counter",
    }


def _qual_settings(*, n=1, render_profile_enabled=True, **overrides):
    base = {
        "cache_dir": "/tmp/qual-cache",
        "cache_dir_canonical": "/private/tmp/qual-cache",
        "cache_dir_canonical_available": True,
        "cache_content_hash": None,
        "cache_content_hash_reason": "not_hashed_at_boundary; launcher/preflight may hash path independently",
        "host": "127.0.0.1",
        "port": 43594,
        "lowmem_requested": False,
        "mainland": True,
        "frontend": "tui",
        "n": n,
        "workload": "active",
        "render_policy_requested": "none",
        "single_renderer": False,
        "diagnostics": False,
        "failure_capture": False,
        "env_flags_requested": {
            "BOT_SCHEDULING_PROFILE": True,
            "BOT_RENDER_PROFILE": True,
            "BOT_GPU_COMPLETION_PROFILE": False,
            "BOT_RESPONSIVENESS_PROFILE": True,
            "BOT_RESPONSIVENESS_FINE": False,
        },
        "scheduling_profile_enabled": True,
        "render_profile_enabled": render_profile_enabled,
        "gpu_completion_profile_enabled": False,
        "responsiveness_profile_enabled": True,
        "responsiveness_fine_enabled": False,
        "client_lowmem_actual": {
            "available": True,
            "source": "per_slot",
            "path": "slots[].runtime_settings.lowmem",
            "note": "null until first observed frame",
        },
        "client_audio_actual": {
            "settings_available": True,
            "settings_source": "per_slot",
            "settings_path": "slots[].runtime_settings.{midi_active,midi_volume,wave_enabled,wave_volume}",
            "settings_note": "MIDI/wave enable+volume scalars",
            "physical_output_available": False,
            "physical_output_reason": "host speaker/device/sink ownership is not observed",
        },
        "client_renderer_actual": (
            {
                "available": True,
                "source": "per_slot",
                "path": "slots[].renderer",
                "note": "scoped render_profile",
            }
            if render_profile_enabled
            else {"available": False, "reason": "render_profile disabled"}
        ),
    }
    base.update(overrides)
    return base


def _native_qual_proof(
    *,
    n: int,
    name_prefix: str,
    started_unix: float,
    observe_s: float = 30.0,
    t0: float = 5.0,
    render_profile_enabled: bool = True,
    runtime_settings=None,
    settings_override=None,
    mutate=None,
):
    """Build native-shaped observe-start/end qualification boundaries."""
    settings = _qual_settings(n=n, render_profile_enabled=render_profile_enabled)
    if settings_override:
        settings = dict(settings)
        settings.update(settings_override)
    rs_base = runtime_settings if runtime_settings is not None else _runtime_settings()

    def slots_for(phase_tag: str, loop_base: int):
        out = []
        for i in range(n):
            name = f"{name_prefix}{i}" if n > 1 else name_prefix
            rs = dict(rs_base)
            rs["loop_cycle"] = loop_base + i  # freshness differs; not a match key
            slot = {
                "ordinal": i,
                "name": name,
                "responsiveness_slot_id": 1000 + i + hash(name_prefix) % 100,
                "cadence_slot_id": 2000 + i + hash(name_prefix) % 100,
                "state": "Running",
                "error": None,
                "runtime": {"paint": {"lines": [f"Steals: {2 if phase_tag == 'start' else 7}"]}},
                "client": {"ingame": True, "scene_state": 2, "x": 1, "z": 2, "level": 0},
                "runtime_settings": rs,
            }
            if render_profile_enabled:
                slot["renderer"] = {
                    "available": True,
                    "source": "host::render_profile::read",
                    "slot_id": slot['cadence_slot_id'],
                    "generation": 1 if phase_tag == "start" else 2,
                    "renderer_present": True,
                    "backend": "cpu",
                    "draw": True,
                    "full_rate": False,
                    "updated_ms": 100 if phase_tag == "start" else 200,
                    "ended": False,
                }
            out.append(slot)
        return out

    start_elapsed = t0
    end_elapsed = t0 + observe_s
    start_before = started_unix + start_elapsed
    start_after = start_before + 0.001
    end_before = started_unix + end_elapsed
    end_after = end_before + 0.001
    proof = [
        {
            "phase": "observe-start",
            "elapsed_s": start_elapsed,
            "slots": slots_for("start", 10),
            "settings": dict(settings),
            "elapsed_wall_bracket": {
                "before_unix_s": start_before,
                "after_unix_s": start_after,
                "meaning": "SystemTime reads bracketing this row's harness elapsed Instant read",
            },
        },
        {
            "phase": "observe-end",
            "elapsed_s": end_elapsed,
            "slots": slots_for("end", 1000),
            "settings": dict(settings),
            "elapsed_wall_bracket": {
                "before_unix_s": end_before,
                "after_unix_s": end_after,
                "meaning": "SystemTime reads bracketing this row's harness elapsed Instant read",
            },
        },
    ]
    if mutate:
        mutate(proof)
    return proof


HOST_CONDITIONS = {
    "hw.ncpu": 8,
    "hw.memsize": 16_000_000_000,
    "brand": "synthetic-test-host",
}

NATIVE_WINDOWS_CONDITIONS = {
    "platform": "Windows-11-10.0.26100-SP0",
    "user": "BotTest",
    "purpose": "frozen native host observation",
    "builds_stopped_before_run": True,
    "terminal_transport_expected": True,
    "panel_render_attribution": False,
    "performance_acceptance": False,
    "native_preflight": {
        "adapter": "nvidia",
        "utc": "2026-09-07T21:22:13.9682564Z",
        "consoleSessionId": 2,
        "vm": {"Name": "274bot-builder", "State": "Off", "MemoryAssigned": 0},
        "quietServices": [{"Name": "ClickToRunSvc", "Status": "Stopped"}],
        "drivers": [{"Name": "NVIDIA GPU", "DriverVersion": "1.0", "PNPDeviceID": "PCI\\\\VEN_10DE"}],
        "sessions": [" console 2 Active "],
        "processes": [],
        "processLasso": {"running": False, "processes": []},
        "dxdiag": [{"cardName": "NVIDIA GPU", "driverVersion": "1.0", "currentMode": None,
                     "hybridGraphicsGPU": None, "monitorName": None}],
        "server": {"ProcessId": 6728, "Name": "node.exe", "CreationDate": "/Date(1)/"},
        "performanceAcceptance": False,
    },
    "process_lasso": {"running": False, "processes": []},
    "dxdiag": [{"cardName": "NVIDIA GPU", "driverVersion": "1.0", "monitorName": None}],
}

CLIENT_COMMIT = _tag("client-commit-shared")
CLIENT_SOURCES = _tag("client-sources-shared")
NAV_PACK = _tag("nav-pack-shared")
NAV_FLAGS = _tag("nav-flags-shared")
CATALOG = _tag("catalog-shared")
ALLOCATOR = "std::alloc::System"


def _complete_meta(
    *,
    run_dir: str,
    binary: str,
    binary_sha256: str,
    started_unix: float,
    ended_unix: float,
    n: int = 1,
    host_sources: str,
    client_sources: str = CLIENT_SOURCES,
    client_commit: str = CLIENT_COMMIT,
    extra: dict | None = None,
) -> dict:
    meta = {
        "exit_code": 0,
        "n": n,
        "observe_s": 30,
        "warmup_s": 5,
        "workload": "active",
        "frontend": "tui",
        "started_unix": started_unix,
        "ended_unix": ended_unix,
        "run_dir": run_dir,
        "binary": binary,
        "binary_sha256": binary_sha256,
        "host_commit": _tag("checkout-not-build"),
        "client_commit": client_commit,
        "host_sources_sha256": host_sources,
        "client_sources_sha256": client_sources,
        "nav_pack_sha256": NAV_PACK,
        "nav_flags_sha256": NAV_FLAGS,
        "render_policy": "none",
        "render_policy_requested": True,
        "scheduling_profile": True,
        "responsiveness_profile": True,
        "responsiveness_fine": False,
        "render_profile": True,
        "gpu_completion_profile": False,
        "diagnostic_sidecar": False,
        "stack_logging": False,
        "stack_logging_mode": None,
        "debug": False,
        "single_renderer": False,
        "sustain": True,
        "terminal": True,
        "terminal_size": [120, 40],
        "allocation_counting": False,
        "renderer_settings": {"quality": "default"},
        "cache_settings": {"cache": "default"},
        "catalog_sha256": CATALOG,
        "feature_flags": {"requested": "memory-profile-no-alloc", "locked": True, "allocation_counting": False},
        "allocator_provenance": ALLOCATOR,
        "failure_capture": False,
        "nav_captures": False,
        "pid": 1,
        "host_conditions": dict(HOST_CONDITIONS),
    }
    if extra:
        meta.update(extra)
    return meta


def _effective_cli(*, binary: str, n: int, frontend: str = "tui") -> list:
    return [
        "python3",
        "run_diagnostic.py",
        frontend,
        str(n),
        "active",
        "--sustain",
        "--no-diagnostics",
        "--scheduling-profile",
        "--render-profile",
        "--responsiveness-profile",
        "--binary",
        binary,
        "--warmup",
        "5",
        "--observe",
        "30",
    ]


def _build_side(
    root: pathlib.Path,
    *,
    role: str,
    cell_name: str,
    run_name: str,
    binary_bytes: bytes,
    started_unix: float,
    ended_unix: float,
    build_commit: str,
    sources_sha: str,
    n: int = 1,
    exit_code: int = 0,
    corrupt_meta_sha: bool = False,
    skip_samples: bool = False,
    qualify_fail: bool = False,
    receipt_run_dir: str | None = None,
    include_server: bool = True,
    forge_overhead_measured: bool = False,
    meta_extra: dict | None = None,
    receipt_extra: dict | None = None,
    omit_receipt_fields: tuple[str, ...] = (),
    binary_path_override: pathlib.Path | None = None,
    native_qual: bool = False,
    name_prefix: str | None = None,
    runtime_settings=None,
    settings_override=None,
    qual_mutate=None,
    duration_mode: str | None = None,
    duration_s_requested=60,
) -> dict:
    """Create one synthetic side: binary, run dir, receipt, mini-manifest fragment."""
    del include_server  # reserved
    cell = root / cell_name
    cell.mkdir(parents=True, exist_ok=True)
    run_dir = root / "runs" / run_name
    run_dir.mkdir(parents=True, exist_ok=True)
    bin_dir = root / "bins"
    bin_dir.mkdir(parents=True, exist_ok=True)

    bin_name = f"{role}-tui.bin"
    bin_path = binary_path_override if binary_path_override is not None else (bin_dir / bin_name)
    if binary_path_override is None or not bin_path.exists():
        bin_path.parent.mkdir(parents=True, exist_ok=True)
        bin_path.write_bytes(binary_bytes)
    bin_sha = _sha(bin_path.read_bytes())

    meta_sha = "deadbeef" * 8 if corrupt_meta_sha else bin_sha
    meta = _complete_meta(
        run_dir=str(run_dir),
        binary=str(bin_path),
        binary_sha256=meta_sha,
        started_unix=started_unix,
        ended_unix=ended_unix,
        n=n,
        host_sources=sources_sha,
        extra=meta_extra,
    )
    meta["exit_code"] = exit_code
    _write_json(run_dir / "metadata.json", meta)
    (cell / 'fixture-meta-path.txt').write_text(str(run_dir / 'metadata.json'))

    if not skip_samples:
        rows, proof = _active_samples(n=n, observe_s=30, t0=5.0)
        if native_qual:
            prefix = name_prefix if name_prefix is not None else ("ref" if role == "reference" else "cand")
            proof = _native_qual_proof(
                n=n,
                name_prefix=prefix,
                started_unix=started_unix,
                observe_s=30.0,
                t0=5.0,
                runtime_settings=runtime_settings,
                settings_override=settings_override,
                mutate=qual_mutate,
            )
        if qualify_fail:
            for slot in proof[1]["slots"]:
                slot["runtime"]["paint"]["lines"] = ["Steals: 2"]
        _write_jsonl(run_dir / "samples.jsonl", rows)
        _write_jsonl(run_dir / "samples.qualification.jsonl", proof)

    raw_hashes = {
        "metadata.json": mea.sha256_file(run_dir / "metadata.json"),
        "samples.jsonl": mea.sha256_file(run_dir / "samples.jsonl") if (run_dir / "samples.jsonl").is_file() else None,
        "samples.qualification.jsonl": (
            mea.sha256_file(run_dir / "samples.qualification.jsonl")
            if (run_dir / "samples.qualification.jsonl").is_file()
            else None
        ),
    }

    sampler = {
        "pid_was": 100,
        "exit_code": 0,
        "interval_s": 1.0,
        "overhead": (
            "measured" if forge_overhead_measured
            else "unmeasured; no matched overhead proof for sampler"
        ),
        "output": str(cell / "server_resources.jsonl"),
    }
    if duration_mode is None:
        sampler["duration_s_requested"] = duration_s_requested
    elif duration_mode == "fixed":
        sampler["duration_mode"] = "fixed"
        sampler["duration_s_requested"] = duration_s_requested
    elif duration_mode == "stop_controlled":
        sampler["duration_mode"] = "stop_controlled"
        sampler["duration_s_requested"] = None
    else:
        sampler["duration_mode"] = duration_mode
        sampler["duration_s_requested"] = duration_s_requested

    receipt = {
        "id": f"{role}_n{n}",
        "index": 1 if role == "reference" else 2,
        "kind": "matched",
        "n": n,
        "binary": str(bin_path),
        "run_dir": receipt_run_dir if receipt_run_dir is not None else str(run_dir),
        "exit_code": exit_code,
        "effective_cli": _effective_cli(binary=str(bin_path), n=n),
        "started_utc": datetime.datetime.fromtimestamp(started_unix - 1, datetime.timezone.utc).isoformat().replace('+00:00', 'Z'),
        "ended_utc": datetime.datetime.fromtimestamp(ended_unix + 1, datetime.timezone.utc).isoformat().replace('+00:00', 'Z'),
        "raw_hashes": raw_hashes,
        "sampler": sampler,
        "host_conditions": dict(HOST_CONDITIONS),
        "qualification": {"qualified": True, "errors": []},  # label must be ignored
        "status": "completed",
    }
    if receipt_extra:
        receipt.update(receipt_extra)
    for field in omit_receipt_fields:
        receipt.pop(field, None)

    receipt_path = cell / "receipt.json"
    _write_json(receipt_path, receipt)
    # Snapshot helpers only — not continuous accounting.
    _write_json(
        cell / "helper_resources_before.json",
        {"sampler": {"pid": 100, "user_s": 0.0, "system_s": 0.0}, "utc": "t0"},
    )
    _write_json(
        cell / "helper_resources_after.json",
        {"sampler": {"pid": 100, "user_s": 0.1, "system_s": 0.0}, "utc": "t1"},
    )
    (cell / "server_resources.jsonl").write_text(
        json.dumps({"elapsed_s": 0.0, "resident_bytes": 1}) + "\n"
    )

    return {
        "role": role,
        "cell": cell,
        "run_dir": run_dir,
        "receipt_path": receipt_path,
        "binary_path": bin_path,
        "binary_sha256": bin_sha,
        "build_commit": build_commit,
        "sources_sha": sources_sha,
        "bin_name": bin_name,
        "raw_hashes": raw_hashes,
    }


def _write_manifest(root: pathlib.Path, ref: dict, cand: dict) -> pathlib.Path:
    path = root / "manifest.json"
    manifest = {
        "binaries": {
            "control_tui_play": {
                "path": str(ref["binary_path"]),
                "filename": ref["bin_name"],
                "sha256": ref["binary_sha256"],
                "size_bytes": ref["binary_path"].stat().st_size,
            },
            "candidate_tui_play": {
                "path": str(cand["binary_path"]),
                "filename": cand["bin_name"],
                "sha256": cand["binary_sha256"],
                "size_bytes": cand["binary_path"].stat().st_size,
            },
        },
        "control": {
            "commit": ref["build_commit"],
            "branch": "control-branch",
            "sources_sha256_pre": ref["sources_sha"],
            "sources_sha256_post": ref["sources_sha"],
            "sources_stable_across_build": True,
            "build_exit": 0,
            "client": {
                "commit": CLIENT_COMMIT,
                "sources_sha256": CLIENT_SOURCES,
            },
        },
        "candidate": {
            "commit_at_freeze_snap": cand["build_commit"],
            "branch": "candidate-branch",
            "sources_sha256_pre": cand["sources_sha"],
            "sources_sha256_post": cand["sources_sha"],
            "sources_stable_across_build": True,
            "build_exit": 0,
            "client": {
                "commit": CLIENT_COMMIT,
                "sources_sha256": CLIENT_SOURCES,
            },
        },
        "features": {
            "requested": "memory-profile-no-alloc",
            "locked": True,
            "allocator": ALLOCATOR,
            "allocation_counting": False,
        },
        "catalog": {"js_scripts_json_sha256": CATALOG},
        "nav": {
            "nav_pack_sha256": NAV_PACK,
            "nav_flags_sha256": NAV_FLAGS,
        },
        "server": {"target": "local"},
    }
    for section, key, label in (('nav', 'nav_pack', 'nav-pack-shared'),
                                ('nav', 'nav_flags', 'nav-flags-shared'),
                                ('catalog', 'js_scripts_json', 'catalog-shared')):
        asset = root / key
        asset.write_bytes(label.encode())
        manifest[section][key] = str(asset)
    _write_json(path, manifest)
    return path


def _server_identity(root: pathlib.Path) -> pathlib.Path:
    path = root / "server_identity.json"
    _write_json(
        path,
        {
            "pid": 9,
            "start_identity": "macos_lstart:test",
            "port_listen": 43594,
            "configuration": {"target": "local", "version": "synthetic-fixture-server"},
            "note": "explicit PID only; argv/env not recorded",
        },
    )
    _bind_fixture_artifacts(root, root / 'manifest.json', path)
    return path


def _bind_fixture_artifacts(root, manifest_path, server_path):
    """Create a real persisted fixture chain without repairing test omissions."""
    manifest = json.loads(manifest_path.read_text())
    for receipt_path in root.glob('*/receipt.json'):
        receipt = json.loads(receipt_path.read_text())
        # Fixture path itself (not deliberately corrupted receipt run_dir).
        meta_path = pathlib.Path((receipt_path.parent / 'fixture-meta-path.txt').read_text())
        if not meta_path.is_file():
            continue
        meta = json.loads(meta_path.read_text())
        role = 'reference' if meta['binary'] == manifest['binaries']['control_tui_play']['path'] else 'candidate'
        meta.update(nav_pack=manifest['nav']['nav_pack'], nav_flags=manifest['nav']['nav_flags'],
                    catalog_path=manifest['catalog']['js_scripts_json'])
        build = mea.bp.verify_build(manifest_path, role, meta['frontend'], meta['binary'],
                                    meta['nav_pack'], meta['nav_flags'], meta['catalog_path'])
        build['completion_status'] = 'unchanged'
        meta['build_provenance'] = build
        _write_json(meta_path, meta)
        if isinstance(receipt.get('raw_hashes'), dict):
            receipt['raw_hashes']['metadata.json'] = mea.sha256_file(meta_path)
        receipt.update(manifest_path=str(manifest_path), manifest_sha256=mea.sha256_file(manifest_path),
                       server_identity_path=str(server_path), server_identity_sha256=mea.sha256_file(server_path))
        _write_json(receipt_path, receipt)


class MatchedEvidenceAdapterTests(unittest.TestCase):
    def test_probe_flag_and_artifact_binding(self):
        ref, _, manifest, server = self._positive_pair()
        receipt = json.loads(ref['receipt_path'].read_text())
        meta_path = ref['run_dir']/'metadata.json'
        meta = json.loads(meta_path.read_text())
        receipt['effective_cli'].append('--tui-input-probes')
        meta['tui_input_probes'] = True
        artifact = ref['run_dir']/'input-probes.jsonl'
        artifact.write_text('{"kind":"complete","sent":1}\n')
        digest = mea.sha256_file(artifact)
        meta['input_probe_result'] = dict(sent=1, error=None, path=str(artifact), sha256=digest)
        _write_json(meta_path, meta)
        receipt['raw_hashes']['metadata.json'] = mea.sha256_file(meta_path)
        receipt['raw_hashes']['input-probes.jsonl'] = digest
        _write_json(ref['receipt_path'], receipt)
        def bind():
            return mea.bind_side(ref['receipt_path'], role='reference', manifest_path=manifest,
                                 server_identity_path=server)
        good = bind()
        self.assertTrue(good['binding_ok'], good)
        self.assertTrue(good['match_keys']['tui_input_probes'])
        self.assertEqual(good['raw_hashes']['input-probes.jsonl'], digest)
        artifact.write_text('changed bytes')
        self.assertEqual(bind()['reason'], 'input_probe_artifact_invalid')
        artifact.write_text('{"kind":"complete","sent":1}\n')
        receipt['raw_hashes'].pop('input-probes.jsonl')
        _write_json(ref['receipt_path'], receipt)
        self.assertEqual(bind()['reason'], 'receipt_raw_hash_field_missing')
        receipt['effective_cli'].remove('--tui-input-probes')
        _write_json(ref['receipt_path'], receipt)
        self.assertEqual(bind()['reason'], 'receipt_cli_metadata_mismatch:tui_input_probes')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)

    def _positive_pair(self, **cand_kw):
        ref = _build_side(
            self.root,
            role="reference",
            cell_name="cell_ref",
            run_name="run_ref",
            binary_bytes=b"CONTROL-BINARY-v1",
            started_unix=1_000_000.0,
            ended_unix=1_000_100.0,
            build_commit=_tag("commit-control-aaaa"),
            sources_sha=_tag("sources-control-aaaa"),
        )
        cand = _build_side(
            self.root,
            role="candidate",
            cell_name="cell_cand",
            run_name="run_cand",
            binary_bytes=b"CANDIDATE-BINARY-v2",
            started_unix=1_000_200.0,
            ended_unix=1_000_300.0,
            build_commit=_tag("commit-candidate-bbbb"),
            sources_sha=_tag("sources-candidate-bbbb"),
            **cand_kw,
        )
        manifest = _write_manifest(self.root, ref, cand)
        server = _server_identity(self.root)
        return ref, cand, manifest, server

    def test_root_contract_counterexamples_rejected(self):
        probes = ('invalid_utc', 'wrong_utc_envelope', 'missing_enabled_flag', 'extra_enabled_flag',
                  'duplicate_observe', 'missing_manifest_fixtures', 'bad_source_digest',
                  'wrong_manifest_hash', 'asset_corruption', 'server_sidecar_swap',
                  'nonfinite_metadata', 'bool_pid', 'missing_build_proof', 'changed_build_proof',
                  'launcher_failed', 'sampler_failed', 'explicit_binding_error', 'bool_feature',
                  'digest_only_runtime_fixtures')
        for probe in probes:
            with self.subTest(probe=probe):
                ref, _, manifest_path, server = self._positive_pair()
                receipt = json.loads(ref['receipt_path'].read_text())
                meta_path = ref['run_dir'] / 'metadata.json'
                meta = json.loads(meta_path.read_text())
                manifest = json.loads(manifest_path.read_text())
                if probe == 'invalid_utc': receipt['started_utc'] = 'nonsense'
                if probe == 'wrong_utc_envelope': receipt['started_utc'] = '2026-09-07T00:00:00Z'
                if probe == 'missing_enabled_flag': receipt['effective_cli'].remove('--scheduling-profile')
                if probe == 'extra_enabled_flag': receipt['effective_cli'].append('--failure-capture')
                if probe == 'duplicate_observe': receipt['effective_cli'] += ['--observe', '999']
                if probe == 'missing_manifest_fixtures': manifest.pop('nav')
                if probe == 'bad_source_digest':
                    manifest['control']['sources_sha256_pre'] = 'junk'
                    manifest['control']['sources_sha256_post'] = 'junk'
                if probe == 'nonfinite_metadata': meta['observe_s'] = float('nan')
                if probe == 'bool_pid': meta['pid'] = True
                if probe == 'missing_build_proof': meta.pop('build_provenance')
                if probe == 'changed_build_proof': meta['build_provenance']['completion_status'] = 'changed'
                if probe == 'launcher_failed': receipt['launcher_exit_code'] = 1
                if probe == 'sampler_failed': receipt['sampler_result'] = {'exit_code': 1}
                if probe == 'explicit_binding_error': receipt['binding_errors'] = ['failed']
                if probe == 'bool_feature': meta['feature_flags']['locked'] = 1
                # Hashes remain; paths omitted — must not fall back to manifest paths.
                if probe == 'digest_only_runtime_fixtures':
                    for field in ('nav_pack', 'nav_flags', 'catalog_path'):
                        meta.pop(field, None)
                _write_json(manifest_path, manifest)
                receipt['manifest_sha256'] = mea.sha256_file(manifest_path)
                if probe == 'wrong_manifest_hash': receipt['manifest_sha256'] = '0' * 64
                _write_json(meta_path, meta)
                receipt['raw_hashes']['metadata.json'] = mea.sha256_file(meta_path)
                _write_json(ref['receipt_path'], receipt)
                if probe == 'asset_corruption': pathlib.Path(meta['nav_flags']).write_bytes(b'corrupt')
                if probe == 'server_sidecar_swap': _write_json(server, {'pid': 10, 'start_identity': 'different', 'port_listen': 43594})
                bound = mea.bind_side(ref['receipt_path'], role='reference', manifest_path=manifest_path, server_identity_path=server)
                self.assertFalse(bound['binding_ok'], (probe, bound))
                self.assertEqual(bound['status'], 'unavailable')
                if probe == 'digest_only_runtime_fixtures':
                    self.assertEqual(bound['reason'], 'metadata_runtime_fixture_path_missing')
                    self.assertIn(bound.get('field'), ('nav_pack', 'nav_flags', 'catalog_path'))

    def test_checkout_labels_can_differ_from_saved_binary_sources(self):
        ref, _, manifest, server = self._positive_pair()
        meta_path = ref['run_dir'] / 'metadata.json'
        meta = json.loads(meta_path.read_text())
        for field in ('host_commit', 'host_sources_sha256', 'client_commit', 'client_sources_sha256'):
            meta[field] = _tag('different-checkout-' + field)
        meta['checkout_source_labels_only'] = True
        _write_json(meta_path, meta)
        receipt = json.loads(ref['receipt_path'].read_text())
        receipt['raw_hashes']['metadata.json'] = mea.sha256_file(meta_path)
        _write_json(ref['receipt_path'], receipt)
        bound = mea.bind_side(ref['receipt_path'], role='reference', manifest_path=manifest, server_identity_path=server)
        self.assertTrue(bound['binding_ok'], bound)
        self.assertEqual(bound['match_keys']['client_sources_sha256'], CLIENT_SOURCES)

    def test_fixture_change_during_independent_analysis_is_rejected(self):
        ref, _, manifest, server = self._positive_pair()
        actual = mea.qc.qualify
        def mutate_then_qualify(*args, **kwargs):
            (self.root / 'nav_flags').write_bytes(b'changed during analysis')
            return actual(*args, **kwargs)
        with mock.patch.object(mea.qc, 'qualify', side_effect=mutate_then_qualify):
            bound = mea.bind_side(ref['receipt_path'], role='reference', manifest_path=manifest, server_identity_path=server)
        self.assertFalse(bound['binding_ok'], bound)
        self.assertIn('changed during evidence', bound.get('detail', ''))

    def test_host_conditions_file_must_match_persisted_hash(self):
        ref, _, manifest, server = self._positive_pair()
        host = self.root / 'host.json'
        _write_json(host, HOST_CONDITIONS)
        receipt = json.loads(ref['receipt_path'].read_text())
        receipt.update(host_conditions_path=str(host), host_conditions_sha256=mea.sha256_file(host))
        _write_json(ref['receipt_path'], receipt)
        bound = mea.bind_side(ref['receipt_path'], role='reference', manifest_path=manifest,
                              server_identity_path=server, host_conditions_path=host)
        self.assertTrue(bound['binding_ok'], bound)
        _write_json(host, dict(HOST_CONDITIONS, brand='different-host'))
        bound = mea.bind_side(ref['receipt_path'], role='reference', manifest_path=manifest,
                              server_identity_path=server, host_conditions_path=host)
        self.assertFalse(bound['binding_ok'])
        self.assertEqual(bound['reason'], 'receipt_host_conditions_hash_mismatch')

    def test_native_windows_conditions_allow_observed_optional_absence(self):
        self.assertTrue(mea._native_windows_conditions_complete(NATIVE_WINDOWS_CONDITIONS))

    def test_native_windows_conditions_require_identity_and_controls(self):
        for field in (("platform", None), ("user", "")):
            with self.subTest(field=field[0]):
                value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
                value[field[0]] = field[1]
                self.assertFalse(mea._native_windows_conditions_complete(value))
        for field in ("vm", "drivers", "sessions", "processLasso", "dxdiag", "server"):
            with self.subTest(field=field):
                value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
                value["native_preflight"][field] = None
                self.assertFalse(mea._native_windows_conditions_complete(value))
        value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
        value["native_preflight"]["performanceAcceptance"] = None
        self.assertFalse(mea._native_windows_conditions_complete(value))

        for field in ("purpose", "builds_stopped_before_run", "terminal_transport_expected", "panel_render_attribution"):
            with self.subTest(field=field):
                value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
                value[field] = None
                self.assertFalse(mea._native_windows_conditions_complete(value))

    def test_native_windows_conditions_file_binds_without_rewriting_provenance(self):
        ref, _, manifest, server = self._positive_pair()
        host = self.root / "host.json"
        _write_json(host, NATIVE_WINDOWS_CONDITIONS)
        receipt = json.loads(ref["receipt_path"].read_text())
        digest = mea.sha256_file(host)
        receipt.update(host_conditions_path=str(host), host_conditions_sha256=digest)
        _write_json(ref["receipt_path"], receipt)
        bound = mea.bind_side(ref["receipt_path"], role="reference", manifest_path=manifest,
                              server_identity_path=server, host_conditions_path=host)
        self.assertTrue(bound["binding_ok"], bound)
        self.assertEqual(bound["match_keys"]["host_conditions"], NATIVE_WINDOWS_CONDITIONS)
        self.assertEqual(json.loads(host.read_text()), NATIVE_WINDOWS_CONDITIONS)
        self.assertEqual(receipt["host_conditions_sha256"], digest)

    def test_native_windows_conditions_fail_closed_for_malformed_and_inconsistent_observations(self):
        value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
        value["native_preflight"]["consoleSessionId"] = "two"
        value["native_preflight"]["dxdiag"][0].update(
            currentMode="unknown", hybridGraphicsGPU="unknown", monitorName="unknown",
        )
        value["dxdiag"][0]["monitorName"] = "unknown"
        process = {
            "ProcessId": 6728, "ParentProcessId": 1, "SessionId": 2,
            "Name": "node.exe", "CreationDate": "/Date(1)/", "CommandLine": "unknown",
        }
        value["native_preflight"]["processes"] = [process]
        value["native_preflight"]["processLasso"]["processes"] = [process]
        value["process_lasso"]["processes"] = [process]
        self.assertFalse(mea._native_windows_conditions_complete(value))
        # A dense recognized native shape must not fall back to legacy
        # _deep_missing validation merely because it has no null leaves.
        keys: dict[str, object] = {key: True for key in mea.MATCH_KEY_FIELDS}
        keys["frontend"] = "panel"
        keys["terminal"] = False
        keys["terminal_size"] = None
        keys["sampler_duration_mode"] = "fixed"
        keys["sampler_duration_s_requested"] = 1
        keys["host_conditions"] = value
        self.assertFalse(mea._deep_missing(value))
        self.assertEqual(mea.match_keys_complete(keys), "host_conditions")
        value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
        value["process_lasso"]["running"] = True
        self.assertFalse(mea._native_windows_conditions_complete(value))
        value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
        value["native_preflight"]["processLasso"]["running"] = True
        self.assertFalse(mea._native_windows_conditions_complete(value))
        value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
        value["native_preflight"]["processLasso"]["files"] = [{"path": "", "sha256": "bad"}]
        self.assertFalse(mea._native_windows_conditions_complete(value))
        value = json.loads(json.dumps(NATIVE_WINDOWS_CONDITIONS))
        value["dxdiag"][0]["cardName"] = "different"
        self.assertFalse(mea._native_windows_conditions_complete(value))

    def test_legacy_host_conditions_keep_recursive_strict_validation(self):
        self.assertTrue(mea._host_conditions_complete(HOST_CONDITIONS))
        invalid = json.loads(json.dumps(HOST_CONDITIONS))
        invalid["nested"] = {"required": None}
        self.assertFalse(mea._host_conditions_complete(invalid))

    def test_positive_binding_and_qualification_overhead_unavailable(self):
        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(
            ref["receipt_path"],
            role="reference",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        self.assertTrue(ref_b["binding_ok"], ref_b)
        self.assertTrue(cand_b["binding_ok"], cand_b)
        self.assertTrue(ref_b["qualified"], ref_b.get("qualification"))
        self.assertTrue(cand_b["qualified"], cand_b.get("qualification"))
        self.assertEqual(ref_b["status"], "bound")
        self.assertEqual(cand_b["status"], "bound")
        self.assertEqual(ref_b["raw_hash_status"], "receipt_recorded_and_verified")
        # Different side provenance allowed.
        self.assertNotEqual(
            ref_b["side_provenance"]["binary_sha256"],
            cand_b["side_provenance"]["binary_sha256"],
        )
        self.assertNotEqual(
            ref_b["side_provenance"]["manifest_build_commit"],
            cand_b["side_provenance"]["manifest_build_commit"],
        )
        self.assertNotEqual(
            ref_b["side_provenance"]["manifest_sources_sha256"],
            cand_b["side_provenance"]["manifest_sources_sha256"],
        )
        self.assertFalse(ref_b["side_provenance"]["host_commit_is_build_authority"])
        self.assertIsNone(mea.match_keys_complete(ref_b["match_keys"]))
        self.assertEqual(ref_b["match_keys"], cand_b["match_keys"])
        self.assertFalse(ref_b["overhead"]["measured"])
        self.assertEqual(ref_b["overhead"]["status"], "unavailable")

        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertTrue(pair["binding_ok"])
        self.assertFalse(pair["pair_eligible"])
        self.assertEqual(pair["status"], "unavailable")
        self.assertEqual(pair["reason"], "overhead_unavailable")
        self.assertFalse(pair["final_acceptance_claim"])
        # Diagnostic compare stays closed (resources still unavailable and/or overhead).
        cmp_ = pair.get("compare_matched_runs") or {}
        self.assertIn(
            cmp_.get("reason"),
            {"overhead_unknown", "unavailable_matched_run", "missing_match_provenance"},
        )
        self.assertNotEqual(cmp_.get("status"), "available")
        self.assertFalse(cmp_.get("accepted_saving", False))

    def test_missing_null_match_key_unavailable(self):
        ref, cand, manifest, server = self._positive_pair(
            meta_extra={"nav_flags_sha256": None},
        )
        # Manifest still has nav flags — must NOT enrich missing metadata.
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        ref_b = mea.bind_side(
            ref["receipt_path"],
            role="reference",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=ref["cell"],
        )
        self.assertTrue(ref_b["binding_ok"], ref_b)
        self.assertFalse(cand_b["binding_ok"], cand_b)
        self.assertEqual(cand_b['reason'], 'metadata_runtime_fixture_mismatch')
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "side_binding_failed")

    def test_null_render_policy_stays_unavailable(self):
        ref, cand, manifest, server = self._positive_pair(
            meta_extra={"render_policy": None},
        )
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        ref_b = mea.bind_side(
            ref["receipt_path"],
            role="reference",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=ref["cell"],
        )
        self.assertFalse(cand_b["binding_ok"], cand_b)
        self.assertEqual(cand_b['reason'], 'receipt_cli_metadata_mismatch:render_policy')
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "side_binding_failed")
        # resource_match_keys must not default TUI null → "none"
        keys = rm.resource_match_keys_from_meta({"frontend": "tui", "render_policy": None})
        self.assertIsNone(keys.get("render_policy"))

    def test_nested_null_renderer_settings_unavailable(self):
        ref, cand, manifest, server = self._positive_pair(
            meta_extra={"renderer_settings": {"quality": None}},
        )
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        ref_b = mea.bind_side(
            ref["receipt_path"],
            role="reference",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=ref["cell"],
        )
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "missing_match_key")
        self.assertEqual(pair.get("candidate_missing_match_key"), "renderer_settings")

    def test_swapped_binary_hash_rejected(self):
        ref, cand, manifest, server = self._positive_pair(corrupt_meta_sha=True)
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "binary_hash_mismatch_metadata")

    def test_receipt_binary_mismatch_rejected(self):
        other = self.root / "bins" / "other.bin"
        other.parent.mkdir(parents=True, exist_ok=True)
        other.write_bytes(b"OTHER-BINARY")
        ref, cand, manifest, server = self._positive_pair(
            receipt_extra={"binary": str(other)},
        )
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "receipt_metadata_binary_mismatch")

    def test_copied_same_hash_non_canonical_path_rejected(self):
        ref, cand, manifest, server = self._positive_pair()
        # Copy candidate binary to a different path with identical bytes.
        copy_path = self.root / "bins-copy" / "candidate-copy.bin"
        copy_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(cand["binary_path"], copy_path)
        # Point metadata + receipt + CLI at the copy; keep manifest on original path.
        meta = json.loads((cand["run_dir"] / "metadata.json").read_text())
        meta["binary"] = str(copy_path)
        _write_json(cand["run_dir"] / "metadata.json", meta)
        receipt = json.loads(cand["receipt_path"].read_text())
        receipt["binary"] = str(copy_path)
        receipt["effective_cli"] = _effective_cli(binary=str(copy_path), n=1)
        receipt["raw_hashes"] = {
            "metadata.json": mea.sha256_file(cand["run_dir"] / "metadata.json"),
            "samples.jsonl": mea.sha256_file(cand["run_dir"] / "samples.jsonl"),
            "samples.qualification.jsonl": mea.sha256_file(
                cand["run_dir"] / "samples.qualification.jsonl"
            ),
        }
        _write_json(cand["receipt_path"], receipt)
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"], cand_b)
        self.assertEqual(cand_b["reason"], "artifact_validation_failed")
        self.assertIn('binary path differs', cand_b['detail'])

    def test_missing_receipt_fields_rejected(self):
        ref, cand, manifest, server = self._positive_pair(
            omit_receipt_fields=("id", "index", "kind", "effective_cli", "started_utc", "ended_utc"),
        )
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "receipt_missing_field")
        self.assertIn(cand_b.get("field"), {"id", "index", "kind", "effective_cli", "started_utc", "ended_utc"})

    def test_missing_receipt_raw_hashes_not_binding_ok(self):
        ref, cand, manifest, server = self._positive_pair(omit_receipt_fields=("raw_hashes",))
        cand_b = mea.bind_side(
            cand["receipt_path"],
            role="candidate",
            manifest_path=manifest,
            server_identity_path=server,
            cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "receipt_raw_hashes_missing")
        self.assertEqual(cand_b.get("raw_hash_status"), "live_snapshot_only")

    def test_role_specific_differing_source_allowed_when_bound(self):
        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertTrue(ref_b["binding_ok"] and cand_b["binding_ok"], (ref_b, cand_b))
        self.assertNotEqual(
            ref_b["side_provenance"]["manifest_sources_sha256"],
            cand_b["side_provenance"]["manifest_sources_sha256"],
        )
        # Wrong role binding (candidate receipt as reference) must fail path/hash/role.
        wrong = mea.bind_side(
            cand["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertFalse(wrong["binding_ok"])
        self.assertIn(
            wrong["reason"],
            {
                "binary_hash_mismatch_manifest",
                "binary_path_not_manifest_canonical_path",
                "binary_not_bound_to_manifest_entry",
                "artifact_validation_failed",
            },
        )

    def test_duplicate_run_rejected(self):
        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        # Reuse reference binding as candidate.
        pair = mea.bind_pair(reference=ref_b, candidate=dict(ref_b, role="candidate"))
        self.assertEqual(pair["reason"], "duplicate_run_identity")

    def test_overlapping_windows_rejected(self):
        ref = _build_side(
            self.root, role="reference", cell_name="c1", run_name="r1",
            binary_bytes=b"A", started_unix=1000.0, ended_unix=1100.0,
            build_commit=_tag("c1"), sources_sha=_tag("s1"),
        )
        # Observation wall = started+5 .. started+35 → overlap with ref.
        cand = _build_side(
            self.root, role="candidate", cell_name="c2", run_name="r2",
            binary_bytes=b"B", started_unix=1020.0, ended_unix=1120.0,
            build_commit=_tag("c2"), sources_sha=_tag("s2"),
        )
        manifest = _write_manifest(self.root, ref, cand)
        server = _server_identity(self.root)
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertTrue(ref_b["binding_ok"] and cand_b["binding_ok"], (ref_b, cand_b))
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "overlapping_observation_windows")

    def test_receipt_run_dir_mismatch(self):
        ref, cand, manifest, server = self._positive_pair(
            receipt_run_dir="/tmp/does-not-match-meta-run",
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "run_dir_missing")

    def test_metadata_run_dir_missing_field_rejected(self):
        ref, cand, manifest, server = self._positive_pair()
        meta = json.loads((cand["run_dir"] / "metadata.json").read_text())
        del meta["run_dir"]
        _write_json(cand["run_dir"] / "metadata.json", meta)
        receipt = json.loads(cand["receipt_path"].read_text())
        receipt["raw_hashes"] = {
            "metadata.json": mea.sha256_file(cand["run_dir"] / "metadata.json"),
            "samples.jsonl": mea.sha256_file(cand["run_dir"] / "samples.jsonl"),
            "samples.qualification.jsonl": mea.sha256_file(
                cand["run_dir"] / "samples.qualification.jsonl"
            ),
        }
        _write_json(cand["receipt_path"], receipt)
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "metadata_missing_field")
        self.assertEqual(cand_b.get("field"), "run_dir")

    def test_failed_qualification(self):
        ref, cand, manifest, server = self._positive_pair(qualify_fail=True)
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertTrue(cand_b["binding_ok"], cand_b)
        self.assertFalse(cand_b["qualified"])
        self.assertEqual(cand_b["status"], "unavailable")
        self.assertEqual(cand_b["reason"], "workload_not_qualified")
        # Receipt label said qualified True — must not matter.
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "side_not_qualified")

    def test_endpoint_mismatch(self):
        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        # Force endpoint asymmetry after binding.
        ref_b = dict(ref_b)
        cand_b = dict(cand_b)
        ref_b["endpoint_notes"] = {
            "decode": {"status": "available", "target_verdict": "meet"},
            "input": {"status": "unavailable"},
            "gpu": {"status": "unavailable"},
            "scheduling": {"status": "unavailable"},
        }
        cand_b["endpoint_notes"] = {
            "decode": {"status": "unavailable", "reason": "no_slot"},
            "input": {"status": "unavailable"},
            "gpu": {"status": "unavailable"},
            "scheduling": {"status": "unavailable"},
        }
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "endpoint_mismatch")

    def test_n_gt1_without_ordinals_unavailable(self):
        ref = _build_side(
            self.root, role="reference", cell_name="c1", run_name="r1",
            binary_bytes=b"A16", started_unix=1000.0, ended_unix=1100.0,
            build_commit=_tag("c1"), sources_sha=_tag("s1"), n=16,
        )
        cand = _build_side(
            self.root, role="candidate", cell_name="c2", run_name="r2",
            binary_bytes=b"B16", started_unix=1200.0, ended_unix=1300.0,
            build_commit=_tag("c2"), sources_sha=_tag("s2"), n=16,
        )
        manifest = _write_manifest(self.root, ref, cand)
        server = _server_identity(self.root)
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertTrue(ref_b["binding_ok"] and cand_b["binding_ok"], (ref_b, cand_b))
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "missing_stable_slot_ordinals")

    def test_forged_overhead_without_helper_accounting(self):
        ref, cand, manifest, server = self._positive_pair(forge_overhead_measured=True)
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertFalse(ref_b["overhead"]["measured"])
        self.assertEqual(ref_b["overhead"]["status"], "unavailable")
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "overhead_unavailable")

    def test_match_key_mismatch(self):
        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand2 = _build_side(
            self.root, role="candidate", cell_name="cell_cand2", run_name="run_cand2",
            binary_bytes=b"CANDIDATE-BINARY-v3", started_unix=1_000_200.0,
            ended_unix=1_000_300.0, build_commit=_tag("commit-candidate-bbbb"),
            sources_sha=_tag("sources-candidate-bbbb"),
        )
        man = json.loads(manifest.read_text())
        man["binaries"]["candidate_tui_play"] = {
            "path": str(cand2["binary_path"]),
            "filename": cand2["bin_name"],
            "sha256": cand2["binary_sha256"],
            "size_bytes": cand2["binary_path"].stat().st_size,
        }
        _write_json(manifest, man)
        _bind_fixture_artifacts(self.root, manifest, server)
        cand_b = mea.bind_side(
            cand2["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand2["cell"],
        )
        cand_b = dict(cand_b)
        cand_b["match_keys"] = dict(cand_b["match_keys"])
        cand_b["match_keys"]["sustain"] = False
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "match_key_mismatch")
        self.assertIn("sustain", pair.get("differing_keys") or [])

    def test_cpu_fallback_requested_backend_match_keys(self):
        """CPU proof cannot silently match a GPU control via missing backend keys."""
        # Legacy TUI meta defaults to none/false.
        keys = mea.construct_match_keys({"frontend": "tui", "n": 1, "workload": "active"})
        self.assertFalse(keys["cpu_fallback"])
        self.assertEqual(keys["requested_backend"], "none")
        # Panel legacy defaults to gpu.
        panel_keys = mea.construct_match_keys({"frontend": "panel", "n": 1, "workload": "idle"})
        self.assertFalse(panel_keys["cpu_fallback"])
        self.assertEqual(panel_keys["requested_backend"], "gpu")
        # Explicit CPU is recorded and differs from GPU.
        cpu_keys = mea.construct_match_keys({
            "frontend": "panel", "n": 1, "workload": "idle",
            "cpu_fallback": True, "requested_backend": "cpu_fallback",
        })
        self.assertTrue(cpu_keys["cpu_fallback"])
        self.assertEqual(cpu_keys["requested_backend"], "cpu_fallback")
        self.assertNotEqual(panel_keys["requested_backend"], cpu_keys["requested_backend"])

        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertTrue(ref_b.get("binding_ok"), ref_b)
        self.assertTrue(cand_b.get("binding_ok"), cand_b)
        # Mutate candidate match keys to CPU — pair must refuse GPU/CPU mix.
        cand_b = dict(cand_b)
        cand_b["match_keys"] = dict(cand_b["match_keys"])
        cand_b["match_keys"]["cpu_fallback"] = True
        cand_b["match_keys"]["requested_backend"] = "cpu_fallback"
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "match_key_mismatch")
        diff = pair.get("differing_keys") or []
        self.assertTrue(
            "requested_backend" in diff or "cpu_fallback" in diff,
            diff,
        )

    def test_failure_capture_mismatch_not_dropped_by_whitelist(self):
        """compare_matched_runs must not drop instrumentation flags from equality."""
        base = {
            "status": "available",
            "overhead": "measured",
            "cpu_cores": 0.1,
            "resident_median_bytes": 100,
            "contaminated": False,
            "match_metadata": {
                "frontend": "tui",
                "n": 1,
                "workload": "active",
                "nav_pack_sha256": NAV_PACK,
                "nav_flags_sha256": NAV_FLAGS,
                "renderer_settings": {"quality": "default"},
                "cache_settings": {"cache": "default"},
                "catalog_sha256": CATALOG,
                "feature_flags": {"f": True},
                "allocator_provenance": ALLOCATOR,
                "client_sources_sha256": CLIENT_SOURCES,
                "failure_capture": False,
                "scheduling_profile": True,
            },
            "side_provenance": {
                "binary_sha256": _tag("bin-a"),
                "host_sources_sha256": _tag("host-a"),
            },
        }
        cand = dict(base)
        cand["match_metadata"] = dict(base["match_metadata"], failure_capture=True)
        cand["side_provenance"] = dict(base["side_provenance"], binary_sha256=_tag("bin-b"))
        out = rm.compare_matched_runs(cand, base)
        self.assertEqual(out["reason"], "mismatched_provenance_or_settings")
        self.assertNotEqual(out.get("cpu_non_regression"), True)

    def test_preserve_failed_cell(self):
        cell = mea.preserve_failed_cell(
            cell_id="candidate_n16_profiles_on",
            receipt_path=N1_N16_RECEIPT if N1_N16_RECEIPT.is_file() else None,
            reason="functional_failure",
            details={"qualified": False},
        )
        self.assertEqual(cell["status"], "preserved_failure")
        self.assertFalse(cell["final_acceptance_claim"])

    def test_resource_match_keys_exclude_side_digests(self):
        meta = {
            "frontend": "tui",
            "n": 1,
            "workload": "active",
            "binary_sha256": "bin-a",
            "host_sources_sha256": "host-a",
            "client_sources_sha256": "client",
            "nav_pack_sha256": "np",
            "nav_flags_sha256": "nf",
            "renderer_settings": {"a": 1},
            "cache_settings": {"b": 2},
            "catalog_sha256": "cat",
            "feature_flags": {"f": True},
            "allocator_provenance": "system",
            "failure_capture": False,
            "scheduling_profile": True,
        }
        keys = rm.resource_match_keys_from_meta(meta)
        self.assertNotIn("binary_sha256", keys)
        self.assertNotIn("host_sources_sha256", keys)
        self.assertIn("failure_capture", keys)
        self.assertIn("scheduling_profile", keys)
        side = rm.resource_side_provenance_from_meta(meta)
        self.assertEqual(side["binary_sha256"], "bin-a")
        other = dict(meta, binary_sha256="bin-b", host_sources_sha256="host-b")
        self.assertEqual(
            rm.resource_match_keys_from_meta(meta),
            rm.resource_match_keys_from_meta(other),
        )

    @unittest.skipUnless(
        N1_CONTROL_RECEIPT.is_file() and N1_CANDIDATE_RECEIPT.is_file() and MANIFEST.is_file(),
        "N1 shared-nav artifacts not present",
    )
    def test_real_n1_pair_unavailable_missing_provenance_or_overhead(self):
        pair = mea.read_matched_pair(
            reference_receipt=N1_CONTROL_RECEIPT,
            candidate_receipt=N1_CANDIDATE_RECEIPT,
            manifest_path=MANIFEST,
            server_identity_path=N1_SERVER if N1_SERVER.is_file() else None,
            preserve_cells=[
                mea.preserve_failed_cell(
                    cell_id="candidate_n16_profiles_on",
                    receipt_path=N1_N16_RECEIPT if N1_N16_RECEIPT.is_file() else None,
                    reason="functional_failure",
                )
            ],
        )
        self.assertFalse(pair.get("pair_eligible"))
        self.assertEqual(pair.get("status"), "unavailable")
        self.assertFalse(pair.get("final_acceptance_claim"))
        # Legacy N1 lacks receipt raw hashes / host conditions / some match keys.
        self.assertIn(
            pair.get("reason"),
            {
                "missing_match_key",
                "overhead_unavailable",
                "side_binding_failed",
                "side_not_qualified",
                "match_key_mismatch",
            },
        )
        self.assertTrue(pair.get("preserved_cells"))
        self.assertNotEqual(pair.get("reason"), "paired_within_margin")
        self.assertNotIn(pair.get("status"), ("available", "accepted", "pass"))
        # Must not claim binding_ok on incomplete legacy receipt chain.
        ref = pair.get("reference") or {}
        cand = pair.get("candidate") or {}
        if ref.get("binding_ok") or cand.get("binding_ok"):
            # If a side somehow binds, pair still unavailable and not accepted.
            self.assertFalse(pair.get("pair_eligible"))
        else:
            self.assertEqual(pair.get("reason"), "side_binding_failed")

    def _bind_native_pair(self, **kwargs):
        ref = _build_side(
            self.root, role="reference", cell_name="nref", run_name="nrun_ref",
            binary_bytes=b"NREF", started_unix=2_000_000.0, ended_unix=2_000_100.0,
            build_commit=_tag("n-ref-c"), sources_sha=_tag("n-ref-s"),
            native_qual=True, name_prefix="alpha", **kwargs,
        )
        cand = _build_side(
            self.root, role="candidate", cell_name="ncand", run_name="nrun_cand",
            binary_bytes=b"NCAND", started_unix=2_000_200.0, ended_unix=2_000_300.0,
            build_commit=_tag("n-cand-c"), sources_sha=_tag("n-cand-s"),
            native_qual=True, name_prefix="beta", **kwargs,
        )
        manifest = _write_manifest(self.root, ref, cand)
        server = _server_identity(self.root)
        ref_b = mea.bind_side(ref["receipt_path"], role="reference", manifest_path=manifest,
                              server_identity_path=server, cell_dir=ref["cell"])
        cand_b = mea.bind_side(cand["receipt_path"], role="candidate", manifest_path=manifest,
                               server_identity_path=server, cell_dir=cand["cell"])
        return ref, cand, manifest, server, ref_b, cand_b

    def test_n16_match_by_ordinal_despite_different_names(self):
        """Cross-run identity is ordinal; generated names may differ per side."""
        _, _, _, _, ref_b, cand_b = self._bind_native_pair(n=16)
        self.assertTrue(ref_b["binding_ok"], ref_b)
        self.assertTrue(cand_b["binding_ok"], cand_b)
        self.assertTrue(ref_b["slot_ordinals_present"])
        self.assertTrue(cand_b["slot_ordinals_present"])
        self.assertEqual(ref_b["observation_wall_span_source"], "native_elapsed_wall_bracket")
        # Names differ across sides.
        ref_names = [m["name"] for m in ref_b["ordinal_mapping"]]
        cand_names = [m["name"] for m in cand_b["ordinal_mapping"]]
        self.assertNotEqual(ref_names, cand_names)
        self.assertTrue(all(n.startswith("alpha") for n in ref_names))
        self.assertTrue(all(n.startswith("beta") for n in cand_names))
        # Native match keys equal (by ordinal, not name).
        self.assertEqual(
            ref_b["native_qualification"]["match_keys"],
            cand_b["native_qualification"]["match_keys"],
        )
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertTrue(pair["binding_ok"])
        self.assertEqual(pair["reason"], "overhead_unavailable")
        self.assertEqual(pair["native_ordinal_mapping"]["cross_run_identity"], "ordinal")
        # loop_cycle not in match keys
        rs0 = ref_b["native_qualification"]["match_keys"]["slot_runtime_settings_by_ordinal"][0]
        self.assertNotIn("loop_cycle", rs0)

    def test_native_duplicate_ordinal_rejected(self):
        def mutate(proof):
            proof[0]["slots"][1]["ordinal"] = 0
        # CLI allows only n in {1,16,32,128}; use 16 for multi-slot native cases.
        ref = _build_side(
            self.root, role="reference", cell_name="c1", run_name="r1",
            binary_bytes=b"A", started_unix=1000.0, ended_unix=1100.0,
            build_commit=_tag("c"), sources_sha=_tag("s"), n=16,
            native_qual=True, qual_mutate=mutate,
        )
        cand = _build_side(
            self.root, role="candidate", cell_name="c2", run_name="r2",
            binary_bytes=b"B", started_unix=1200.0, ended_unix=1300.0,
            build_commit=_tag("c2"), sources_sha=_tag("s2"), n=16, native_qual=True,
        )
        manifest = _write_manifest(self.root, ref, cand)
        server = _server_identity(self.root)
        ref_b = mea.bind_side(ref["receipt_path"], role="reference", manifest_path=manifest,
                              server_identity_path=server, cell_dir=ref["cell"])
        self.assertTrue(ref_b["binding_ok"], ref_b)
        self.assertFalse(ref_b["slot_ordinals_present"])
        self.assertIn("ordinal_duplicate", ref_b["native_qualification"]["reason"])

    def test_native_missing_reordered_mismatched_ids(self):
        def missing_slot(proof):
            proof[0]["slots"] = proof[0]["slots"][:-1]

        def reordered(proof):
            proof[0]["slots"] = list(reversed(proof[0]["slots"]))
            # keep ordinal values so order check fails
            for i, s in enumerate(proof[0]["slots"]):
                s["ordinal"] = len(proof[0]["slots"]) - 1 - i

        def mismatched_ids(proof):
            proof[1]["slots"][0]["responsiveness_slot_id"] = 999999

        for name, mut in (
            ("missing", missing_slot),
            ("reordered", reordered),
            ("mismatched_ids", mismatched_ids),
        ):
            with self.subTest(case=name):
                root = pathlib.Path(tempfile.mkdtemp(dir=self.root))
                ref = _build_side(
                    root, role="reference", cell_name="c1", run_name="r1",
                    binary_bytes=b"A" + name.encode(), started_unix=1000.0, ended_unix=1100.0,
                    build_commit=_tag(name + "c"), sources_sha=_tag(name + "s"), n=16,
                    native_qual=True, qual_mutate=mut,
                )
                cand = _build_side(
                    root, role="candidate", cell_name="c2", run_name="r2",
                    binary_bytes=b"B" + name.encode(), started_unix=1200.0, ended_unix=1300.0,
                    build_commit=_tag(name + "c2"), sources_sha=_tag(name + "s2"), n=16,
                    native_qual=True,
                )
                manifest = _write_manifest(root, ref, cand)
                server = _server_identity(root)
                ref_b = mea.bind_side(ref["receipt_path"], role="reference", manifest_path=manifest,
                                      server_identity_path=server, cell_dir=ref["cell"])
                self.assertTrue(ref_b["binding_ok"], ref_b)
                self.assertFalse(ref_b["slot_ordinals_present"], ref_b["native_qualification"])
                cand_b = mea.bind_side(cand["receipt_path"], role="candidate", manifest_path=manifest,
                                       server_identity_path=server, cell_dir=cand["cell"])
                pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
                self.assertFalse(pair.get("pair_eligible"))
                self.assertIn(
                    pair["reason"],
                    {
                        "missing_stable_slot_ordinals",
                        "native_qualification_asymmetric",
                        "native_qualification_unavailable",
                        # Missing slots can also fail independent workload qualification first.
                        "side_not_qualified",
                    },
                )

    def test_none_vs_false_runtime_settings(self):
        def null_rs(proof):
            for boundary in proof:
                for slot in boundary["slots"]:
                    slot["runtime_settings"] = None

        # Observed false is valid.
        _, _, _, _, ref_b, cand_b = self._bind_native_pair(
            n=16, runtime_settings=_runtime_settings(lowmem=False, draw=False),
        )
        self.assertTrue(ref_b.get("binding_ok"), ref_b)
        self.assertTrue(ref_b["slot_ordinals_present"])
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "overhead_unavailable")

        # Null cannot fill defaults → native unavailable (direct consume; n need not be CLI-legal).
        ref = _build_side(
            self.root, role="reference", cell_name="nullref", run_name="nullrun",
            binary_bytes=b"NULLR", started_unix=3_000_000.0, ended_unix=3_000_100.0,
            build_commit=_tag("nullc"), sources_sha=_tag("nulls"), n=16,
            native_qual=True, qual_mutate=null_rs,
        )
        native = mea.consume_native_qualification(
            ref["run_dir"], n=16, meta=json.loads((ref["run_dir"] / "metadata.json").read_text())
        )
        self.assertEqual(native["status"], "unavailable")
        self.assertIn("runtime_settings_null", native["reason"])

    def test_differing_runtime_scalars_backend_cadence(self):
        # Different lowmem across sides → native match key mismatch.
        ref = _build_side(
            self.root, role="reference", cell_name="d1", run_name="dr1",
            binary_bytes=b"D1", started_unix=4_000_000.0, ended_unix=4_000_100.0,
            build_commit=_tag("d1c"), sources_sha=_tag("d1s"), n=16, native_qual=True,
            runtime_settings=_runtime_settings(lowmem=True),
        )
        cand = _build_side(
            self.root, role="candidate", cell_name="d2", run_name="dr2",
            binary_bytes=b"D2", started_unix=4_000_200.0, ended_unix=4_000_300.0,
            build_commit=_tag("d2c"), sources_sha=_tag("d2s"), n=16, native_qual=True,
            runtime_settings=_runtime_settings(lowmem=False),
        )
        manifest = _write_manifest(self.root, ref, cand)
        server = _server_identity(self.root)
        ref_b = mea.bind_side(ref["receipt_path"], role="reference", manifest_path=manifest,
                              server_identity_path=server, cell_dir=ref["cell"])
        cand_b = mea.bind_side(cand["receipt_path"], role="candidate", manifest_path=manifest,
                               server_identity_path=server, cell_dir=cand["cell"])
        self.assertTrue(ref_b.get("binding_ok") and cand_b.get("binding_ok"), (ref_b, cand_b))
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "native_match_key_mismatch")

        # Backend difference — isolate in nested root so fixture rebinding stays local.
        def backend_gpu(proof):
            for b in proof:
                for s in b["slots"]:
                    if "renderer" in s:
                        s["renderer"]["backend"] = "gpu"

        root_b = pathlib.Path(tempfile.mkdtemp(dir=self.root))
        ref2 = _build_side(
            root_b, role="reference", cell_name="b1", run_name="br1",
            binary_bytes=b"B1", started_unix=5_000_000.0, ended_unix=5_000_100.0,
            build_commit=_tag("b1c"), sources_sha=_tag("b1s"), n=16, native_qual=True,
        )
        cand2 = _build_side(
            root_b, role="candidate", cell_name="b2", run_name="br2",
            binary_bytes=b"B2", started_unix=5_000_200.0, ended_unix=5_000_300.0,
            build_commit=_tag("b2c"), sources_sha=_tag("b2s"), n=16, native_qual=True,
            qual_mutate=backend_gpu,
        )
        man2 = _write_manifest(root_b, ref2, cand2)
        server2 = _server_identity(root_b)
        ref_b2 = mea.bind_side(ref2["receipt_path"], role="reference", manifest_path=man2,
                               server_identity_path=server2, cell_dir=ref2["cell"])
        cand_b2 = mea.bind_side(cand2["receipt_path"], role="candidate", manifest_path=man2,
                                server_identity_path=server2, cell_dir=cand2["cell"])
        self.assertTrue(ref_b2.get("binding_ok") and cand_b2.get("binding_ok"), (ref_b2, cand_b2))
        pair2 = mea.bind_pair(reference=ref_b2, candidate=cand_b2)
        self.assertEqual(pair2["reason"], "native_match_key_mismatch")

        # Cadence/profile enabled difference.
        root_p = pathlib.Path(tempfile.mkdtemp(dir=self.root))
        ref3 = _build_side(
            root_p, role="reference", cell_name="p1", run_name="pr1",
            binary_bytes=b"P1", started_unix=6_000_000.0, ended_unix=6_000_100.0,
            build_commit=_tag("p1c"), sources_sha=_tag("p1s"), n=16, native_qual=True,
            settings_override={"scheduling_profile_enabled": True},
        )
        cand3 = _build_side(
            root_p, role="candidate", cell_name="p2", run_name="pr2",
            binary_bytes=b"P2", started_unix=6_000_200.0, ended_unix=6_000_300.0,
            build_commit=_tag("p2c"), sources_sha=_tag("p2s"), n=16, native_qual=True,
            settings_override={"scheduling_profile_enabled": False},
        )
        man3 = _write_manifest(root_p, ref3, cand3)
        server3 = _server_identity(root_p)
        ref_b3 = mea.bind_side(ref3["receipt_path"], role="reference", manifest_path=man3,
                               server_identity_path=server3, cell_dir=ref3["cell"])
        cand_b3 = mea.bind_side(cand3["receipt_path"], role="candidate", manifest_path=man3,
                                server_identity_path=server3, cell_dir=cand3["cell"])
        self.assertTrue(ref_b3.get("binding_ok") and cand_b3.get("binding_ok"), (ref_b3, cand_b3))
        pair3 = mea.bind_pair(reference=ref_b3, candidate=cand_b3)
        self.assertEqual(pair3["reason"], "native_match_key_mismatch")

    def test_stop_controlled_duration_mode(self):
        ref, cand, manifest, server, ref_b, cand_b = self._bind_native_pair(
            n=1, duration_mode="stop_controlled", duration_s_requested=None,
        )
        self.assertTrue(ref_b["binding_ok"], ref_b)
        self.assertEqual(ref_b["match_keys"]["sampler_duration_mode"], "stop_controlled")
        self.assertIsNone(ref_b["match_keys"]["sampler_duration_s_requested"])
        self.assertIsNone(mea.match_keys_complete(ref_b["match_keys"]))
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "overhead_unavailable")

        # Fixed mode still rejects null duration.
        bad = _build_side(
            self.root, role="reference", cell_name="fix1", run_name="fx1",
            binary_bytes=b"FX", started_unix=7_000_000.0, ended_unix=7_000_100.0,
            build_commit=_tag("fxc"), sources_sha=_tag("fxs"),
            duration_mode="fixed", duration_s_requested=None,
        )
        cand_ok = _build_side(
            self.root, role="candidate", cell_name="fx2", run_name="fx2",
            binary_bytes=b"FY", started_unix=7_000_200.0, ended_unix=7_000_300.0,
            build_commit=_tag("fyc"), sources_sha=_tag("fys"),
        )
        man = _write_manifest(self.root, bad, cand_ok)
        server = _server_identity(self.root)
        bound = mea.bind_side(bad["receipt_path"], role="reference", manifest_path=man,
                              server_identity_path=server)
        self.assertFalse(bound["binding_ok"])
        self.assertEqual(bound["reason"], "receipt_sampler_configuration_invalid")

        # stop_controlled with non-null duration rejected.
        bad2 = _build_side(
            self.root, role="reference", cell_name="sc1", run_name="sc1",
            binary_bytes=b"SC", started_unix=8_000_000.0, ended_unix=8_000_100.0,
            build_commit=_tag("scc"), sources_sha=_tag("scs"),
            duration_mode="stop_controlled", duration_s_requested=30,
        )
        # force non-null after build
        receipt = json.loads(bad2["receipt_path"].read_text())
        receipt["sampler"]["duration_s_requested"] = 30
        _write_json(bad2["receipt_path"], receipt)
        cand2 = _build_side(
            self.root, role="candidate", cell_name="sc2", run_name="sc2",
            binary_bytes=b"SD", started_unix=8_000_200.0, ended_unix=8_000_300.0,
            build_commit=_tag("sdc"), sources_sha=_tag("sds"),
        )
        man2 = _write_manifest(self.root, bad2, cand2)
        server = _server_identity(self.root)
        bound2 = mea.bind_side(bad2["receipt_path"], role="reference", manifest_path=man2,
                               server_identity_path=server)
        self.assertFalse(bound2["binding_ok"])
        self.assertEqual(bound2["reason"], "receipt_sampler_stop_controlled_duration_must_be_null")

    def test_runtime_config_transition_start_end_unavailable(self):
        def change_end_settings(proof):
            proof[1]["settings"] = dict(proof[1]["settings"])
            proof[1]["settings"]["lowmem_requested"] = True

        ref = _build_side(
            self.root, role="reference", cell_name="tr1", run_name="tr1",
            binary_bytes=b"TR", started_unix=9_000_000.0, ended_unix=9_000_100.0,
            build_commit=_tag("trc"), sources_sha=_tag("trs"), n=16,
            native_qual=True, qual_mutate=change_end_settings,
        )
        meta = json.loads((ref["run_dir"] / "metadata.json").read_text())
        native = mea.consume_native_qualification(ref["run_dir"], n=16, meta=meta)
        self.assertEqual(native["status"], "unavailable")
        self.assertEqual(native["reason"], "runtime_config_changed_between_observe_start_and_end")

    def test_positive_still_reaches_overhead_unavailable_without_native(self):
        """Legacy synthetic N=1 without native rows still binds; ordinals absent OK for n=1."""
        ref, cand, manifest, server = self._positive_pair()
        ref_b = mea.bind_side(ref["receipt_path"], role="reference", manifest_path=manifest,
                              server_identity_path=server, cell_dir=ref["cell"])
        self.assertTrue(ref_b["binding_ok"])
        self.assertFalse(ref_b["slot_ordinals_present"])
        self.assertEqual(ref_b["match_keys"]["sampler_duration_mode"], "fixed")


class NativeRuntimeContractTests(unittest.TestCase):
    def test_nonterminal_panel_null_size_is_explicitly_inapplicable(self):
        base: dict = {key: True for key in mea.MATCH_KEY_FIELDS}
        base.update(
            frontend='panel', n=1, terminal=False, terminal_size=None,
            sampler_duration_mode='fixed', sampler_duration_s_requested=1,
        )
        base['host_conditions'] = HOST_CONDITIONS
        self.assertIsNone(
            mea.match_keys_complete(base)
        )
        missing_post_terminal_size = dict(base, host_conditions=None)
        self.assertEqual(
            mea.match_keys_complete(missing_post_terminal_size),
            'host_conditions',
        )
        omitted = dict(base)
        omitted.pop('terminal_size')
        self.assertEqual(
            mea.match_keys_complete(omitted),
            'terminal_size',
        )
        tui = dict(base, frontend='tui')
        self.assertEqual(
            mea.match_keys_complete(tui),
            'terminal_size',
        )
        terminal = dict(base, terminal=True)
        self.assertEqual(
            mea.match_keys_complete(terminal),
            'terminal_size',
        )
        sized = dict(base, terminal_size=[120, 40])
        self.assertEqual(
            mea.match_keys_complete(sized),
            'terminal_size',
        )

    def test_one_renderer_mode_accepts_explicit_absence(self):
        proof = _native_qual_proof(n=16, name_prefix='panel', started_unix=1000,
                                  settings_override={'frontend':'panel', 'render_policy_requested':'focused-one'})
        for boundary in proof:
            for slot in boundary['slots'][1:]:
                slot['renderer'].update(renderer_present=False, backend=None)
        with tempfile.TemporaryDirectory() as tmp:
            path=pathlib.Path(tmp)
            (path/'samples.qualification.jsonl').write_text('\n'.join(json.dumps(r) for r in proof)+'\n')
            result=mea.consume_native_qualification(path,n=16,meta={'frontend':'panel','workload':'active',
                                                                  'started_unix':1000,'ended_unix':1100})
        self.assertEqual(result['status'],'available',result)
        self.assertEqual(sum(r['backend']=='absent' for r in result['match_keys']['renderer_config_by_ordinal']),15)

    def test_typed_native_fields_and_renderer_identity(self):
        proof = _native_qual_proof(n=1,name_prefix='slot',started_unix=1000)
        changes = [
            lambda b: b['settings'].update(scheduling_profile_enabled=1),
            lambda b: b['settings'].update(n='1'),
            lambda b: b['slots'][0].update(responsiveness_slot_id=1.5),
            lambda b: b['slots'][0]['runtime_settings'].update(loop_cycle=1.5),
            lambda b: b['slots'][0]['renderer'].update(generation=float('nan')),
            lambda b: b['slots'][0]['renderer'].update(slot_id=123),
            lambda b: b['slots'][0]['renderer'].update(backend='unknown'),
            lambda b: b['slots'][0]['renderer'].update(ended=True),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            path=pathlib.Path(tmp)
            for change in changes:
                rows=copy.deepcopy(proof)
                for b in rows:
                    change(b)
                (path/'samples.qualification.jsonl').write_text('\n'.join(json.dumps(r) for r in rows)+'\n')
                result=mea.consume_native_qualification(path,n=1,meta={'started_unix':1000,'ended_unix':1100})
                self.assertEqual(result['status'],'unavailable',result)


if __name__ == "__main__":
    unittest.main()
