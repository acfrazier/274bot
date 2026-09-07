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


def _write_json(path: pathlib.Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2) + "\n")


def _write_jsonl(path: pathlib.Path, rows) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows))


def _active_samples(n: int = 1, observe_s: int = 30, t0: float = 0.0):
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


def _complete_meta(
    *,
    run_dir: str,
    binary: str,
    binary_sha256: str,
    started_unix: float,
    ended_unix: float,
    n: int = 1,
    host_sources: str = "host-src-ref",
    client_sources: str = "client-src-shared",
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
        "host_commit": "checkout-not-build",
        "client_commit": "client-aaa",
        "host_sources_sha256": host_sources,
        "client_sources_sha256": client_sources,
        "nav_pack_sha256": "nav-pack-aaa",
        "nav_flags_sha256": "nav-flags-aaa",
        "render_policy": "none",
        "render_policy_requested": "none",
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
        "catalog_sha256": "catalog-aaa",
        "feature_flags": {"feature": True},
        "allocator_provenance": "system",
        "failure_capture": False,
        "nav_captures": False,
        "pid": 1,
    }
    if extra:
        meta.update(extra)
    return meta


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
) -> dict:
    """Create one synthetic side: binary, run dir, receipt, mini-manifest fragment."""
    cell = root / cell_name
    cell.mkdir(parents=True, exist_ok=True)
    run_dir = root / "runs" / run_name
    run_dir.mkdir(parents=True, exist_ok=True)
    bin_dir = root / "bins"
    bin_dir.mkdir(parents=True, exist_ok=True)

    bin_name = f"{role}-tui.bin"
    bin_path = bin_dir / bin_name
    bin_path.write_bytes(binary_bytes)
    bin_sha = _sha(binary_bytes)

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
        rows, proof = _active_samples(n=n, observe_s=30)
        if qualify_fail:
            for slot in proof[1]["slots"]:
                slot["runtime"]["paint"]["lines"] = ["Steals: 2"]
        _write_jsonl(run_dir / "samples.jsonl", rows)
        _write_jsonl(run_dir / "samples.qualification.jsonl", proof)

    receipt = {
        "id": f"{role}_n{n}",
        "index": 1 if role == "reference" else 2,
        "kind": "matched",
        "n": n,
        "binary": str(bin_path),
        "run_dir": receipt_run_dir if receipt_run_dir is not None else str(run_dir),
        "exit_code": exit_code,
        "started_utc": "2026-09-07T00:00:00Z",
        "ended_utc": "2026-09-07T00:01:00Z",
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
        "qualification": {"qualified": True, "errors": []},  # label must be ignored
        "status": "completed",
    }
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
                "commit": "client-aaa",
                "sources_sha256": "client-src-shared",
            },
        },
        "candidate": {
            "commit_at_freeze_snap": cand["build_commit"],
            "branch": "candidate-branch",
            "sources_sha256_pre": cand["sources_sha"],
            "sources_sha256_post": cand["sources_sha"],
            "client": {
                "commit": "client-aaa",
                "sources_sha256": "client-src-shared",
            },
        },
        "features": {
            "requested": "memory-profile-no-alloc",
            "locked": True,
            "allocator": "std::alloc::System",
            "allocation_counting": False,
        },
        "catalog": {"js_scripts_json_sha256": "catalog-aaa"},
        "nav": {
            "nav_pack_sha256": "nav-pack-aaa",
            "nav_flags_sha256": "nav-flags-aaa",
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
            build_commit="commit-control-aaaa",
            sources_sha="sources-control-aaaa",
        )
        cand = _build_side(
            self.root,
            role="candidate",
            cell_name="cell_cand",
            run_name="run_cand",
            binary_bytes=b"CANDIDATE-BINARY-v2",
            started_unix=1_000_200.0,
            ended_unix=1_000_300.0,
            build_commit="commit-candidate-bbbb",
            sources_sha="sources-candidate-bbbb",
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
        # Clear manifest nav so enrichment cannot fill the hole.
        man = json.loads(manifest.read_text())
        man["nav"]["nav_flags_sha256"] = None
        _write_json(manifest, man)
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
        self.assertTrue(ref_b["binding_ok"] and cand_b["binding_ok"])
        self.assertNotEqual(
            ref_b["side_provenance"]["manifest_sources_sha256"],
            cand_b["side_provenance"]["manifest_sources_sha256"],
        )
        # Wrong role binding (candidate receipt as reference) must fail hash/role.
        wrong = mea.bind_side(
            cand["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertFalse(wrong["binding_ok"])
        self.assertIn(
            wrong["reason"],
            {
                "binary_hash_mismatch_manifest",
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
            build_commit="c1", sources_sha="s1",
        )
        cand = _build_side(
            self.root, role="candidate", cell_name="c2", run_name="r2",
            binary_bytes=b"B", started_unix=1050.0, ended_unix=1150.0,
            build_commit="c2", sources_sha="s2",
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
        pair = mea.bind_pair(reference=ref_b, candidate=cand_b)
        self.assertEqual(pair["reason"], "overlapping_observation_windows")

    def test_receipt_run_dir_mismatch(self):
        ref, cand, manifest, server = self._positive_pair(
            receipt_run_dir="/tmp/does-not-match-meta-run",
        )
        # receipt points at missing dir
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertFalse(cand_b["binding_ok"])
        self.assertEqual(cand_b["reason"], "run_dir_missing")

    def test_failed_qualification(self):
        ref, cand, manifest, server = self._positive_pair(qualify_fail=True)
        cand_b = mea.bind_side(
            cand["receipt_path"], role="candidate", manifest_path=manifest,
            server_identity_path=server, cell_dir=cand["cell"],
        )
        self.assertTrue(cand_b["binding_ok"])
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
            build_commit="c1", sources_sha="s1", n=16,
        )
        cand = _build_side(
            self.root, role="candidate", cell_name="c2", run_name="r2",
            binary_bytes=b"B16", started_unix=1200.0, ended_unix=1300.0,
            build_commit="c2", sources_sha="s2", n=16,
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
        ref, cand, manifest, server = self._positive_pair(
            meta_extra={"frontend": "panel"},
        )
        # Candidate panel won't find panel binary key in our tui-only mini manifest
        # unless we only mutate after bind. Bind both as tui then tweak keys.
        ref_b = mea.bind_side(
            ref["receipt_path"], role="reference", manifest_path=manifest,
            server_identity_path=server, cell_dir=ref["cell"],
        )
        # Rebuild candidate as tui then change match key.
        cand2 = _build_side(
            self.root, role="candidate", cell_name="cell_cand2", run_name="run_cand2",
            binary_bytes=b"CANDIDATE-BINARY-v3", started_unix=1_000_200.0,
            ended_unix=1_000_300.0, build_commit="commit-candidate-bbbb",
            sources_sha="sources-candidate-bbbb",
        )
        # Point manifest candidate binary at cand2
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
        }
        keys = rm.resource_match_keys_from_meta(meta)
        self.assertNotIn("binary_sha256", keys)
        self.assertNotIn("host_sources_sha256", keys)
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
        # May bind sides or fail match keys depending on enrichment; never a pass.
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
        # Must not claim acceptance from old report labels.
        self.assertNotEqual(pair.get("reason"), "paired_within_margin")
        self.assertNotIn(pair.get("status"), ("available", "accepted", "pass"))


if __name__ == "__main__":
    unittest.main()
