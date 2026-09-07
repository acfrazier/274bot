#!/usr/bin/env python3
"""Focused tests for matched_evidence_adapter (stdlib only).

Exercises the production reader + real qualify_control / analyze_run paths.
Distinguishes binding success from final pair-gate eligibility.
"""
from __future__ import annotations

import hashlib
import json
import pathlib
import shutil
import sys
import tempfile
import unittest

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


HOST_CONDITIONS = {
    "hw.ncpu": 8,
    "hw.memsize": 16_000_000_000,
    "brand": "synthetic-test-host",
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
        n,
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

    if not skip_samples:
        rows, proof = _active_samples(n=n, observe_s=30, t0=5.0)
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

    receipt = {
        "id": f"{role}_n{n}",
        "index": 1 if role == "reference" else 2,
        "kind": "matched",
        "n": n,
        "binary": str(bin_path),
        "run_dir": receipt_run_dir if receipt_run_dir is not None else str(run_dir),
        "exit_code": exit_code,
        "effective_cli": _effective_cli(binary=str(bin_path), n=n),
        "started_utc": "2026-09-07T00:00:00Z",
        "ended_utc": "2026-09-07T00:01:00Z",
        "raw_hashes": raw_hashes,
        "sampler": {
            "pid_was": 100,
            "exit_code": 0,
            "interval_s": 1.0,
            "duration_s_requested": 60,
            "overhead": (
                "measured" if forge_overhead_measured
                else "unmeasured; no matched overhead proof for sampler"
            ),
            "output": str(cell / "server_resources.jsonl"),
        },
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
            "note": "explicit PID only; argv/env not recorded",
        },
    )
    return path


class MatchedEvidenceAdapterTests(unittest.TestCase):
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
        self.assertTrue(cand_b["binding_ok"], cand_b)
        self.assertEqual(mea.match_keys_complete(cand_b["match_keys"]), "nav_flags_sha256")
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "missing_match_key")

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
        self.assertTrue(cand_b["binding_ok"], cand_b)
        self.assertIsNone(cand_b["match_keys"].get("render_policy"))
        self.assertEqual(mea.match_keys_complete(cand_b["match_keys"]), "render_policy")
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "missing_match_key")
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
        self.assertEqual(cand_b["reason"], "binary_path_not_manifest_canonical_path")

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


if __name__ == "__main__":
    unittest.main()
