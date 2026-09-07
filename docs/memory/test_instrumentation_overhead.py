#!/usr/bin/env python3
"""Adversarial and protocol tests for instrumentation_overhead (stdlib only)."""
from __future__ import annotations

import copy
import hashlib
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

import instrumentation_overhead as ioh  # noqa: E402


def _sha(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


BINARY = _sha("frozen-candidate-binary")


def _role(name: str, pid: int, start: str, cpu: float, rss: float, cores: float) -> dict:
    return {
        "pid": pid,
        "start_identity": start,
        "cpu_s_enclosing_observation": cpu,
        "resident_median_bytes": rss,
        "sampled_resident_peak_bytes": rss * 1.01,
        "cpu_cores_interval": [cores * 0.999, cores],
    }


def _roles(
    *,
    controller_pid: int,
    launcher_pid: int,
    collector_pid: int,
    cpu_scale: float,
    rss_scale: float = 1.0,
) -> dict:
    return {
        "controller": _role(
            "controller",
            controller_pid,
            f"ctrl-{controller_pid}",
            1.0 * cpu_scale,
            27_000_000 * rss_scale,
            0.004 * cpu_scale,
        ),
        "game_server": _role(
            "game_server",
            4719,
            "server-start-fixed",
            3.0 * cpu_scale,
            1_100_000_000 * rss_scale,
            0.03 * cpu_scale,
        ),
        "launcher": _role(
            "launcher",
            launcher_pid,
            f"launch-{launcher_pid}",
            0.05 * cpu_scale,
            180_000_000 * rss_scale,
            0.0005 * cpu_scale,
        ),
        "collector": _role(
            "collector",
            collector_pid,
            f"coll-{collector_pid}",
            0.12 * cpu_scale,
            15_000_000 * rss_scale,
            0.001 * cpu_scale,
        ),
        "gateway": _role(
            "gateway",
            24860,
            "gateway-start-fixed",
            0.2 * cpu_scale,
            250_000_000 * rss_scale,
            0.002 * cpu_scale,
        ),
        "server_supervisor": _role(
            "server_supervisor",
            4718,
            "supervisor-start-fixed",
            0.001 * cpu_scale,
            50_000_000 * rss_scale,
            0.00001 * cpu_scale,
        ),
    }


def _match_keys(*, profiles_on: bool, n: int = 16, **overrides) -> dict:
    profiles = {
        "scheduling_profile": profiles_on,
        "render_profile": profiles_on,
        "responsiveness_profile": profiles_on,
        "responsiveness_fine": profiles_on,
        "gpu_completion_profile": False,
    }
    renderer = (
        {
            "by_ordinal": [
                {
                    "ordinal": 0,
                    "status": "enabled",
                    "backend": "absent",
                    "renderer_present": False,
                    "draw": False,
                    "full_rate": False,
                    "ended": False,
                }
            ]
        }
        if profiles_on
        else {"by_ordinal": [{"ordinal": 0, "status": "disabled_profile_off"}]}
    )
    keys = {
        "frontend": "tui",
        "n": n,
        "workload": "active",
        "render_policy": "none",
        "render_policy_requested": True,
        "single_renderer": False,
        "diagnostic_sidecar": False,
        "allocation_counting": False,
        **profiles,
        "stack_logging": False,
        "sustain": True,
        "terminal": True,
        "terminal_size": [120, 40],
        "nav_pack_sha256": _sha("nav-pack"),
        "nav_flags_sha256": _sha("nav-flags"),
        "renderer_settings": renderer,
        "cache_settings": {
            "cache_dir": "/tmp/cache",
            "unpack_root": "/tmp/unpack",
            "snapshot_version": "abcd",
            "content_identity_sha256": _sha("cache-content"),
        },
        "catalog_sha256": _sha("catalog"),
        "feature_flags": {
            "requested": "memory-profile-no-alloc",
            "locked": True,
            "allocation_counting": False,
        },
        "allocator_provenance": "std::alloc::System",
        "client_sources_sha256": _sha("client-src"),
        "failure_capture": False,
        "nav_captures": False,
        "tui_input_probes": False,
        "server_start_identity": {"pid": 4719, "start_identity": "server-start-fixed"},
        "server_port_listen": 43594,
        "server_configuration": {"target": "local"},
        "sampler_interval_s": 1.0,
        "sampler_duration_mode": "stop_controlled",
        "sampler_duration_s_requested": None,
        "host_conditions": {
            "os": "macOS",
            "architecture": "arm64",
            "logical_cpus": 16,
        },
        "sampler_backend": "libproc",
        "sampler_modules": {
            "process_accounting.py": _sha("pa"),
            "server_resources.py": _sha("sr"),
            "native_process_sample.py": _sha("nps"),
        },
    }
    keys.update(overrides)
    return keys


def _native_keys(*, profiles_on: bool, n: int = 16, **overrides) -> dict:
    keys = {
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
        "scheduling_profile_enabled": profiles_on,
        "render_profile_enabled": profiles_on,
        "gpu_completion_profile_enabled": False,
        "responsiveness_profile_enabled": profiles_on,
        "responsiveness_fine_enabled": profiles_on,
        "env_flags_requested": {
            "BOT_SCHEDULING_PROFILE": profiles_on,
            "BOT_RENDER_PROFILE": profiles_on,
            "BOT_GPU_COMPLETION_PROFILE": False,
            "BOT_RESPONSIVENESS_PROFILE": profiles_on,
            "BOT_RESPONSIVENESS_FINE": profiles_on,
        },
        "cache_dir_canonical_available": True,
        "cache_dir_canonical": "/private/tmp/cache",
        "renderer_config_by_ordinal": (
            [{"ordinal": 0, "status": "enabled", "backend": "absent"}]
            if profiles_on
            else [{"ordinal": 0, "status": "disabled_profile_off"}]
        ),
    }
    renderers = keys.pop('renderer_config_by_ordinal')
    renderers = [{**renderers[0], 'ordinal': i} for i in range(n)]
    keys.update(overrides)
    return {'qualification_settings': keys,
            'slot_runtime_settings_by_ordinal': [{'ordinal': i, 'draw': False, 'lowmem': True} for i in range(n)],
            'renderer_config_by_ordinal': renderers}


def _side(
    *,
    label: str,
    profiles_on: bool,
    t0: float,
    controller_pid: int,
    launcher_pid: int,
    collector_pid: int,
    cpu_scale: float = 1.0,
    n: int = 16,
    binary_sha: str = BINARY,
    identity: str | None = None,
    span: tuple[float, float] | None = None,
    binding_ok: bool = True,
    qualified: bool = True,
    exit_code: int = 0,
    status: str = "bound",
    reason: str | None = None,
    managed_status: str = "available",
    process_status: str = "available",
    native_status: str = "available",
    roles: dict | None = None,
    match_overrides: dict | None = None,
    native_overrides: dict | None = None,
    endpoint_scheduling: str | None = None,
    omit_helper: str | None = None,
) -> dict:
    duration = 120.0
    if span is None:
        span = (t0, t0 + duration)
    if identity is None:
        identity = _sha(f"identity-{label}-{t0}")
    role_map = roles or _roles(
        controller_pid=controller_pid,
        launcher_pid=launcher_pid,
        collector_pid=collector_pid,
        cpu_scale=cpu_scale,
    )
    if omit_helper:
        role_map = dict(role_map)
        role_map.pop(omit_helper, None)
    match_keys = _match_keys(profiles_on=profiles_on, n=n, **(match_overrides or {}))
    native_keys = _native_keys(profiles_on=profiles_on, n=n, **(native_overrides or {}))
    # OFF cells: no scheduling histograms. ON: available when requested.
    if endpoint_scheduling is None:
        endpoint_scheduling = "available" if profiles_on else "unavailable"
    return {
        "status": status,
        "reason": reason,
        "binding_ok": binding_ok,
        "qualified": qualified,
        "exit_code": exit_code,
        "receipt_id": f"candidate_n{n}_overhead_{label}",
        "run_dir": f"/tmp/runs/{label}",
        "n": n,
        "binary_sha256": binary_sha,
        "side_provenance": {"role": "candidate", "binary_sha256": binary_sha},
        "run_identity": {"identity_sha256": identity, "run_dir": f"/tmp/runs/{label}"},
        "observation_wall_span": list(span),
        "match_keys": match_keys,
        "native_qualification": {
            "status": native_status,
            "match_keys": native_keys,
            "slot_ordinals_present": True,
        },
        "managed_resources": {
            "status": managed_status,
            "instrumentation_overhead_measured": False,
            "process": {
                "status": process_status,
                "roles": role_map,
                "process_backend": "libproc",
                "continuous_coverage": True,
            },
        },
        "analysis": {
            "gates": {
                "resources": {
                    "cpu_cores": 0.03 * cpu_scale,
                    "cpu_seconds": 3.6 * cpu_scale,
                    "observation_s": 120.0,
                    "resident_median_bytes": 300_000_000 * cpu_scale,
                    "resident_max_bytes": 310_000_000 * cpu_scale,
                    "peak_resident_bytes": 320_000_000 * cpu_scale,
                    "contaminated": False,
                    "status": "unavailable", "reason": "missing_resource_provenance",
                },
                "scheduling": {"status": endpoint_scheduling},
            }
        },
        "endpoint_notes": {
            "scheduling": {"status": endpoint_scheduling},
            "decode": {"status": "unavailable"},
            "input": {"status": "unavailable"},
            "gpu": {"status": "unavailable"},
        },
        "shaped_for_compare": {},
    }


def _quartet(**kwargs) -> list[dict]:
    """Default healthy OFF/ON/ON/OFF quartet with small ON CPU uplift."""
    base_t = 1_000_000.0
    specs = [
        ("off_a", False, 1.00, 100, 200, 300),
        ("on_a", True, 1.02, 101, 201, 301),
        ("on_b", True, 1.03, 102, 202, 302),
        ("off_b", False, 1.01, 103, 203, 303),
    ]
    sides = []
    for i, (label, on, scale, cpid, lpid, colpid) in enumerate(specs):
        t0 = base_t + i * 200.0
        sides.append(
            _side(
                label=label,
                profiles_on=on,
                t0=t0,
                controller_pid=cpid,
                launcher_pid=lpid,
                collector_pid=colpid,
                cpu_scale=scale,
                **kwargs,
            )
        )
    return sides


class InstrumentationOverheadTests(unittest.TestCase):
    def test_controller_cpu_cannot_drive_host_screen(self):
        sides = _quartet()
        for i in (1, 2):
            sides[i]['managed_resources']['process']['roles']['controller']['cpu_cores_interval'] = [9.0, 10.0]
        out = self._analyze(sides)
        self.assertEqual(out['cpu_5pct_screen']['status'], 'within_5pct_empirical_screen')
        self.assertIn('controller', out['helpers'])
        self.assertEqual(out['host']['resident_median_bytes']['off_values'][0], 300_000_000)
        self.assertEqual(out['helpers']['controller']['resident_median_bytes']['off_values'][0], 27_000_000)

    def test_missing_rust_host_counters_does_not_fall_back_to_controller(self):
        sides = _quartet()
        sides[0]['analysis']['gates']['resources'].pop('cpu_cores')
        out = self._analyze(sides)
        self.assertEqual(out['reason'], 'process_role_metrics_unavailable')

    def test_nested_native_runtime_and_env_mismatches_rejected(self):
        for variant in ('port', 'extra_env', 'actual_flag', 'requested_flag', 'slot_runtime'):
            with self.subTest(variant=variant):
                sides = _quartet()
                keys = sides[1]['native_qualification']['match_keys']
                config = keys['qualification_settings']
                if variant == 'port': config['port'] = 43595
                if variant == 'extra_env': config['env_flags_requested']['OTHER'] = True
                if variant == 'actual_flag': config['scheduling_profile_enabled'] = False
                if variant == 'requested_flag': config['env_flags_requested']['BOT_SCHEDULING_PROFILE'] = False
                if variant == 'slot_runtime': keys['slot_runtime_settings_by_ordinal'][0]['lowmem'] = False
                out = self._analyze(sides)
                self.assertIn(out['reason'], ('disallowed_native_match_key_mismatch', 'native_profile_contract_invalid'))

    def test_cpu_range_crossing_margin_is_inconclusive(self):
        out = ioh._cpu_screen([1.0, 1.03], [1.05, 1.06])
        self.assertEqual(out['status'], 'inconclusive')
        self.assertEqual(out['reason'], 'observed_ratio_range_crosses_margin')

    def test_no_public_binding_injection(self):
        with self.assertRaises(TypeError):
            ioh.analyze_instrumentation_overhead(['a']*4, manifest_path='x', bind_side=lambda *a, **k: {})

    def _analyze(self, sides: list[dict], **kwargs):
        paths = [f"/fake/{i}.json" for i in range(4)]

        def fake_bind(path, **_kw):
            idx = paths.index(str(path))
            return sides[idx]

        with mock.patch.object(ioh.mea, 'bind_side', side_effect=fake_bind):
            return ioh.analyze_instrumentation_overhead(
                paths, manifest_path="/fake/manifest.json", **kwargs)

    def test_happy_path_resource_deltas_and_cpu_screen(self):
        result = self._analyze(_quartet())
        self.assertEqual(result["status"], "resource_deltas_available", result)
        self.assertFalse(result["instrumentation_overhead_measured"])
        self.assertTrue(result["resource_deltas_available"])
        self.assertFalse(result["final_acceptance_claim"])
        self.assertFalse(result["pair_eligible"])
        self.assertFalse(result["accepted_rss_saving"])
        self.assertEqual(result["protocol"]["actual_n"], 16)
        self.assertIn("controller", result["per_role"])
        self.assertEqual(result["host"]["category"], "host")
        self.assertEqual(result["server"]["category"], "server")
        self.assertIn("launcher", result["helpers"])
        self.assertIn("collector", result["helpers"])
        deltas = result["host"]["cpu_s"]["paired_deltas_on_minus_off"]
        self.assertIn("off_a_to_on_a", deltas)
        self.assertIn("on_b_to_off_b", deltas)
        self.assertGreater(deltas["off_a_to_on_a"]["on_minus_off"], 0)
        # Must not present paired deltas as statistical confidence intervals.
        self.assertIn("not confidence intervals", result["host"]["cpu_s"]["note"])
        self.assertIn("never a confidence interval", result["cpu_5pct_screen"]["rule"])
        screen = result["cpu_5pct_screen"]
        self.assertEqual(screen["status"], "within_5pct_empirical_screen", screen)
        # OFF histograms missing ⇒ latency unmeasured, not zero.
        self.assertEqual(result["latency_overhead"]["status"], "unmeasured")
        self.assertEqual(result["latency_overhead"]["reason"], "missing_off_histograms")

    def test_tui_input_probes_must_match_exactly(self):
        """Probe workload is not an instrumentation profile toggle."""
        sides = _quartet()
        sides[2]["match_keys"]["tui_input_probes"] = True
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "disallowed_match_key_mismatch", result)
        self.assertIn("tui_input_probes", result["differing_keys"])
        # Equal non-default probe flag across all four is allowed (still not latency proof).
        for s in sides:
            s["match_keys"]["tui_input_probes"] = True
        ok = self._analyze(sides)
        self.assertEqual(ok["status"], "resource_deltas_available", ok)
        self.assertEqual(ok["latency_overhead"]["status"], "unmeasured")

    def test_require_exactly_four_paths(self):
        out = ioh.analyze_instrumentation_overhead(
            ["a", "b", "c"],
            manifest_path="m",
        )
        self.assertEqual(out["reason"], "require_exactly_four_ordered_receipt_paths")

    def test_reorder_rejected(self):
        sides = _quartet()
        # Swap on_a and off_a settings by swapping sides 0 and 1 → order on/off/on/off
        sides[0], sides[1] = sides[1], sides[0]
        # Fix spans so chronological still holds but settings order wrong.
        base = 1_000_000.0
        for i, s in enumerate(sides):
            s["observation_wall_span"] = [base + i * 200, base + i * 200 + 120]
            s["run_identity"]["identity_sha256"] = _sha(f"reordered-{i}")
            s["run_dir"] = f"/tmp/runs/reordered-{i}"
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "profile_setting_order_mismatch", result)

    def test_overlapping_windows_rejected(self):
        sides = _quartet()
        sides[1]["observation_wall_span"] = [
            sides[0]["observation_wall_span"][0] + 10,
            sides[0]["observation_wall_span"][1] + 10,
        ]
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "overlapping_observation_windows", result)

    def test_duplicate_receipt_identity_rejected(self):
        sides = _quartet()
        sides[2]["run_identity"]["identity_sha256"] = sides[0]["run_identity"]["identity_sha256"]
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "duplicate_run_identity", result)

    def test_changed_binary_rejected(self):
        sides = _quartet()
        sides[2]["binary_sha256"] = _sha("other-binary")
        sides[2]["side_provenance"]["binary_sha256"] = sides[2]["binary_sha256"]
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "binary_sha256_mismatch_across_cells", result)

    def test_changed_cache_config_rejected(self):
        sides = _quartet()
        sides[1]["match_keys"]["cache_settings"] = dict(sides[1]["match_keys"]["cache_settings"])
        sides[1]["match_keys"]["cache_settings"]["content_identity_sha256"] = _sha("mutated-cache")
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "disallowed_match_key_mismatch", result)
        self.assertIn("cache_settings", result["differing_keys"])

    def test_changed_server_identity_rejected(self):
        sides = _quartet()
        # Ambient server PID change across cells.
        for name in ("game_server",):
            sides[2]["managed_resources"]["process"]["roles"][name] = dict(
                sides[2]["managed_resources"]["process"]["roles"][name]
            )
            sides[2]["managed_resources"]["process"]["roles"][name]["pid"] = 99999
            sides[2]["managed_resources"]["process"]["roles"][name]["start_identity"] = "mutated"
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "ambient_or_server_identity_mismatch:game_server", result)

    def test_changed_sampler_module_rejected(self):
        sides = _quartet()
        sides[3]["match_keys"]["sampler_modules"] = dict(sides[3]["match_keys"]["sampler_modules"])
        sides[3]["match_keys"]["sampler_modules"]["process_accounting.py"] = _sha("mutated-module")
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "disallowed_match_key_mismatch", result)
        self.assertIn("sampler_modules", result["differing_keys"])

    def test_missing_helper_evidence_rejected(self):
        sides = _quartet()
        sides[1] = _side(
            label="on_a",
            profiles_on=True,
            t0=1_000_200.0,
            controller_pid=101,
            launcher_pid=201,
            collector_pid=301,
            cpu_scale=1.02,
            omit_helper="collector",
        )
        # Keep identity unique / chronological with neighbors.
        sides[1]["run_identity"]["identity_sha256"] = _sha("on-a-no-collector")
        result = self._analyze(sides)
        self.assertIn(
            result["reason"],
            {"role_set_mismatch", "process_role_metrics_unavailable", "sides_incomplete"},
            result,
        )

    def test_fake_overhead_label_not_trusted(self):
        sides = _quartet()
        # Even if a side carries a forged measured flag, reader uses process evidence only.
        for s in sides:
            s["managed_resources"]["instrumentation_overhead_measured"] = True
            s["overhead"] = {"measured": True, "status": "measured"}
        result = self._analyze(sides)
        # Still computes from roles; does not auto-claim acceptance.
        self.assertFalse(result["final_acceptance_claim"])
        self.assertFalse(result["pair_eligible"])
        self.assertFalse(result["accepted_rss_saving"])

    def test_numeric_reset_and_unqualified_rejected(self):
        sides = _quartet()
        sides[2]["qualified"] = False
        sides[2]["status"] = "unavailable"
        sides[2]["reason"] = "workload_not_qualified"
        sides[2]["binding_ok"] = True
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "sides_incomplete", result)
        self.assertTrue(
            any("not_qualified" in c for c in result["remaining_conditions"]),
            result["remaining_conditions"],
        )

    def test_only_profile_toggle_changes_allowed(self):
        sides = _quartet()
        # Legal: profiles already differ OFF vs ON.
        ok = self._analyze(sides)
        self.assertEqual(ok["status"], "resource_deltas_available", ok)
        # Illegal: frontend change.
        sides[1]["match_keys"]["frontend"] = "panel"
        bad = self._analyze(sides)
        self.assertEqual(bad["reason"], "disallowed_match_key_mismatch", bad)
        self.assertIn("frontend", bad["differing_keys"])

    def test_mixed_profile_group_rejected(self):
        sides = _quartet()
        sides[1]["match_keys"]["responsiveness_fine"] = False  # not full ON group
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "invalid_or_mixed_profile_group", result)

    def test_gpu_enabled_rejected_in_tui_protocol(self):
        sides = _quartet()
        sides[1]["match_keys"]["gpu_completion_profile"] = True
        result = self._analyze(sides)
        self.assertEqual(result["reason"], "invalid_or_mixed_profile_group", result)

    def test_cpu_regression_screen(self):
        sides = _quartet()
        # Inflate Rust host CPU; controller must not drive the screen.
        for idx in (1, 2):
            sides[idx]['analysis']['gates']['resources']['cpu_cores'] = 0.05
        result = self._analyze(sides)
        self.assertEqual(result["cpu_5pct_screen"]["status"], "regression", result["cpu_5pct_screen"])

    def test_cpu_inconclusive_on_wide_spread(self):
        sides = _quartet()
        # OFF replicates differ by >5%.
        sides[0]["analysis"]["gates"]["resources"]["cpu_cores"] = 0.02
        sides[3]["analysis"]["gates"]["resources"]["cpu_cores"] = 0.03
        result = self._analyze(sides)
        self.assertEqual(result["cpu_5pct_screen"]["status"], "inconclusive", result["cpu_5pct_screen"])

    def test_owned_pids_may_differ_server_must_match(self):
        sides = _quartet()
        # Already different owned PIDs in fixture — should pass.
        result = self._analyze(sides)
        self.assertEqual(result["status"], "resource_deltas_available", result)
        pids = [
            s["managed_resources"]["process"]["roles"]["controller"]["pid"] for s in sides
        ]
        self.assertEqual(len(set(pids)), 4)

    def test_never_trusts_caller_bound_dict_without_bind_side(self):
        """Production path must call bind_side; pre-bound dicts are not an API."""
        called = []

        def tracking_bind(path, **kwargs):
            called.append((str(path), kwargs.get("role"), kwargs.get("manifest_path")))
            return _quartet()[len(called) - 1]

        paths = [f"/p/{i}" for i in range(4)]
        with mock.patch.object(ioh.mea, 'bind_side', side_effect=tracking_bind):
            ioh.analyze_instrumentation_overhead(paths, manifest_path="/manifest.json", role="candidate")
        self.assertEqual(len(called), 4)
        self.assertTrue(all(c[1] == "candidate" for c in called))
        self.assertTrue(all(c[2] == "/manifest.json" for c in called))

    def test_n_protocol_remaining_condition(self):
        sides = _quartet()
        for s in sides:
            s["n"] = 1
            s["match_keys"]["n"] = 1
            s["native_qualification"]["match_keys"]["qualification_settings"]["n"] = 1
            s["native_qualification"]["match_keys"]["renderer_config_by_ordinal"] = s["native_qualification"]["match_keys"]["renderer_config_by_ordinal"][:1]
        result = self._analyze(sides, expected_n=16)
        self.assertEqual(result["status"], "partial", result)
        self.assertFalse(result["instrumentation_overhead_measured"])
        self.assertTrue(
            any("protocol_expects_n=16" in c for c in result["remaining_conditions"]),
            result["remaining_conditions"],
        )

    def test_bind_side_invoked_for_real_module_default(self):
        """Default bind_side is matched_evidence_adapter.bind_side."""
        with mock.patch.object(ioh.mea, "bind_side", side_effect=RuntimeError("boom")) as m:
            with self.assertRaises(RuntimeError):
                ioh.analyze_instrumentation_overhead(
                    ["a", "b", "c", "d"],
                    manifest_path="m",
                )
            self.assertTrue(m.called)

    def test_real_single_artifact_cannot_claim_four_cell_overhead(self):
        """Latest functional cell is one ON-like run — not a four-cell proof."""
        receipt = (
            ROOT
            / "diagnostics"
            / "managed-pipeline-qualification-20260907T062612Z"
            / "cells"
            / "candidate_n1_pipeline_v2"
            / "receipt.json"
        )
        manifest = (
            ROOT
            / "diagnostics"
            / "matched-instrumented-build-20260907T054019Z"
            / "build-manifest.json"
        )
        server = (
            ROOT
            / "diagnostics"
            / "managed-pipeline-qualification-20260907T062612Z"
            / "server-identity.json"
        )
        host = (
            ROOT
            / "diagnostics"
            / "managed-pipeline-qualification-20260907T062612Z"
            / "host-conditions.json"
        )
        if not receipt.is_file() or not manifest.is_file():
            self.skipTest("real managed qualification artifacts absent")
        # Four copies of the same receipt must fail (duplicate identity), proving
        # the reader does not invent a four-cell overhead result from one file.
        paths = [receipt, receipt, receipt, receipt]
        result = ioh.analyze_instrumentation_overhead(
            paths,
            manifest_path=manifest,
            role="candidate",
            server_identity_path=server if server.is_file() else None,
            host_conditions_path=host if host.is_file() else None,
        )
        self.assertNotEqual(result.get("status"), "resource_deltas_available")
        self.assertFalse(result.get("instrumentation_overhead_measured"))
        self.assertFalse(result.get("final_acceptance_claim"))
        self.assertFalse(result.get("accepted_rss_saving"))
        # Either incomplete bind repeats or duplicate identity after binds.
        self.assertIn(
            result.get("reason"),
            {
                "duplicate_run_identity",
                "duplicate_run_dir",
                "sides_incomplete",
                "identity_or_span_missing",
                "profile_setting_order_mismatch",
                "invalid_or_mixed_profile_group",
            },
            result,
        )


class CpuScreenUnitTests(unittest.TestCase):
    def test_screen_helpers(self):
        within = ioh._cpu_screen([1.0, 1.01], [1.02, 1.03])
        self.assertEqual(within["status"], "within_5pct_empirical_screen")
        reg = ioh._cpu_screen([1.0, 1.0], [1.1, 1.1])
        self.assertEqual(reg["status"], "regression")
        wide = ioh._cpu_screen([1.0, 1.2], [1.0, 1.01])
        self.assertEqual(wide["status"], "inconclusive")


if __name__ == "__main__":
    unittest.main()
