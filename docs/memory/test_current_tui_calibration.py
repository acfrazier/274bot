#!/usr/bin/env python3
import argparse
import json
import os
import pathlib
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import run_current_tui_calibration as runner


class CurrentTuiCalibrationControls(unittest.TestCase):
    def _direct_args(self, root):
        return argparse.Namespace(
            direct_owner_capture=True, n=1, output=root / "owner.json",
            binary=root / "binary", build_manifest=root / "manifest.json",
            build_role="candidate", server_identity=root / "server.json",
            host_conditions=root / "conditions.json", nav_pack=root / "nav",
            nav_flags=root / "flags", catalog=root / "catalog.json", server_pid=42,
            ssh_parent_pid=7, server_root=root, rs2b0t=root / "rs2b0t",
            cache_dir=root / "cache", unpack_root=root / "unpack",
            conflict_receipt=root / "conflicts.json",
            account_admission=root / "account.json",
            population_admission=root / "population.json",
            cache_admission=root / "cache-admission.json",
            server_health_receipt=root / "server-health.json",
        )

    def test_direct_owner_spec_is_exact_opt_in_and_default_is_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = self._direct_args(root)
            spec = runner.build_spec(args, {
                "host_commit": runner.DIRECT_DIAGNOSTIC_HOST,
                "client_commit": runner.DIRECT_DIAGNOSTIC_CLIENT,
                "host_sources_sha256": runner.DIRECT_HOST_SOURCE_DIGEST,
                "client_sources_sha256": runner.DIRECT_CLIENT_SOURCE_DIGEST,
            })
            contract = spec["capture_contract"]
            self.assertEqual(contract["mode"], "direct-owner-v1")
            self.assertEqual((spec["warmup_s"], spec["observe_s"], spec["teardown_grace_s"]), (30, 120, 60))
            self.assertEqual((spec["sampler_interval_s"], spec["max_wall_s"]), (0.5, 365))
            self.assertEqual(spec["memory_guard"]["limit_bytes"], 268435456)
            self.assertEqual(contract["frontend_rss_limit_bytes"], 536870912)
            self.assertEqual(contract["owned_output_limit_bytes"], 67108864)
            self.assertEqual(contract["frontend_wall_limit_s"], 360)
            self.assertEqual(pathlib.Path(contract["frontend_handoff_path"]), pathlib.Path(contract["cell_dir"]) / "frontend-handoff.json")
            self.assertEqual(pathlib.Path(contract["run_dir"]), pathlib.Path(contract["cell_dir"]) / "frontend-run")
            argv = spec["diagnostic_argv"]
            for flag in ("--direct-owner-capture", "--frontend-handoff", "--run-dir", "--no-diagnostics", "--sustain"):
                self.assertIn(flag, argv)
            self.assertEqual(argv[argv.index("--warmup") + 1], "30")
            self.assertEqual(argv[argv.index("--observe") + 1], "120")

            args.direct_owner_capture = False
            ordinary = runner.build_spec(args, {})
            self.assertNotIn("capture_contract", ordinary)
            self.assertEqual((ordinary["warmup_s"], ordinary["observe_s"], ordinary["teardown_grace_s"]), (120, 600, 60))
            self.assertEqual(ordinary["max_wall_s"], 960)
            self.assertEqual(ordinary["memory_guard"]["limit_bytes"], 128 * 1024 * 1024)

    def test_owner_environment_is_scrubbed_and_only_direct_mode_reemits(self):
        polluted = {"BOT_MEMORY_OWNER_CAPTURE": "1", "PATH": "/usr/bin"}
        ordinary = runner.clean_environment(polluted, n=1)
        self.assertNotIn("BOT_MEMORY_OWNER_CAPTURE", ordinary)
        direct = runner.clean_environment(polluted, n=1, direct_owner_capture=True)
        self.assertEqual(direct["BOT_MEMORY_OWNER_CAPTURE"], "1")
        self.assertEqual(direct["BOT_MEMORY_WARMUP_S"], "30")
        self.assertEqual(direct["BOT_MEMORY_OBSERVE_S"], "120")

    def test_direct_owner_rejects_heaptrack_and_non_n1(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = self._direct_args(root)
            args.heaptrack_output = root / "heaptrack"
            with self.assertRaises(runner.CalibrationError):
                runner.build_spec(args, {})
            args.heaptrack_output = None
            args.n = 16
            with self.assertRaises(runner.CalibrationError):
                runner.build_spec(args, {})

    def test_diagnostic_argv_is_exact_resource_only_contract(self):
        argv = runner.diagnostic_argv(pathlib.Path("/bin/tui-play"), pathlib.Path("/tmp/build.json"), "candidate")
        self.assertEqual(argv[:3], ["tui", "16", "active"])
        self.assertIn("--no-diagnostics", argv)
        self.assertIn("--sustain", argv)
        self.assertEqual(argv[argv.index("--warmup") + 1], "120")
        self.assertEqual(argv[argv.index("--observe") + 1], "600")
        for forbidden in ("--debug", "--tui-input-probes", "--stack-logging", "--scheduling-profile", "--render-profile", "--responsiveness-profile"):
            self.assertNotIn(forbidden, argv)

    def test_n1_cli_and_environment_agree_in_generated_contract(self):
        argv = runner.diagnostic_argv(pathlib.Path("/bin/tui-play"), pathlib.Path("/tmp/build.json"), "candidate", n=1)
        self.assertEqual(argv[:3], ["tui", "1", "active"])
        clean = runner.clean_environment({}, n=1)
        self.assertEqual(clean["BOT_MEMORY_N"], "1")

    def test_n1_spec_receipt_matches_cli_and_environment(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = argparse.Namespace(
                n=1, output=root / "n1.json", binary=root / "binary", build_manifest=root / "manifest.json",
                build_role="candidate", server_identity=root / "server.json", host_conditions=root / "conditions.json",
                nav_pack=root / "nav", nav_flags=root / "flags", catalog=root / "catalog.json", server_pid=42,
                ssh_parent_pid=7, server_root=root, rs2b0t=root / "rs2b0t", cache_dir=root / "cache",
                unpack_root=root / "unpack",
            )
            spec = runner.build_spec(args, {"host_commit": runner.EXPECTED_HOST})
            self.assertEqual(spec["n"], 1)
            self.assertEqual(spec["diagnostic_argv"][1], "1")
            self.assertEqual(runner.clean_environment({}, n=spec["n"])["BOT_MEMORY_N"], spec["diagnostic_argv"][1])

    def test_default_n16_remains_the_cli_and_environment_contract(self):
        self.assertEqual(runner.parser()._option_string_actions["--n"].default, 16)
        self.assertEqual(runner.diagnostic_argv(pathlib.Path("/bin/tui-play"), pathlib.Path("/tmp/build.json"), "candidate")[1], "16")
        self.assertEqual(runner.clean_environment({})["BOT_MEMORY_N"], "16")

    def test_wall_budget_is_unchanged_without_capture_and_split_with_capture(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            common = dict(
                output=root / "n.json", n=1, binary=root / "binary", build_manifest=root / "manifest.json",
                build_role="candidate", server_identity=root / "server.json", host_conditions=root / "conditions.json",
                nav_pack=root / "nav", nav_flags=root / "flags", catalog=root / "catalog.json", server_pid=42,
                ssh_parent_pid=7, server_root=root, rs2b0t=root / "rs2b0t", cache_dir=root / "cache",
                unpack_root=root / "unpack",
            )
            ordinary = runner.build_spec(argparse.Namespace(**common), {})
            self.assertEqual(ordinary["max_wall_s"], 960)
            self.assertNotIn("live_max_wall_s", ordinary)
            with mock.patch.object(runner.hc, "verify_preload", return_value={"path": "/lib/libheaptrack_preload.so"}):
                capture = runner.build_spec(argparse.Namespace(**common, heaptrack_output=root / "raw"), {})
            self.assertEqual(capture["live_max_wall_s"], 960)
            self.assertEqual(capture["analysis_windows_s"], [180, 180])
            self.assertEqual(capture["max_wall_s"], 1320)

    def test_invalid_n_fails_before_launch(self):
        with self.assertRaises(SystemExit):
            runner.parser().parse_args(["--n", "2"])

    def test_invalid_programmatic_n_fails_closed_before_launch(self):
        for value in (2, True, False, "1", 1.0, None):
            with self.subTest(value=value), self.assertRaises(runner.CalibrationError):
                runner.requested_n(argparse.Namespace(n=value))

    def test_missing_programmatic_n_keeps_default(self):
        self.assertEqual(runner.requested_n(argparse.Namespace()), 16)

    def test_clean_environment_forces_all_hot_flags_off(self):
        env = {key: "1" for key in ("BOT_DEBUG", "BOT_CPU", "BOT_SCHEDULING_PROFILE", "BOT_RENDER_PROFILE", "BOT_RESPONSIVENESS_PROFILE", "MallocStackLogging")}
        clean = runner.clean_environment(env)
        self.assertEqual(clean["BOT_MEMORY_N"], "16")
        self.assertEqual(clean["BOT_MEMORY_WORKLOAD"], "active")
        self.assertEqual(clean["BOT_MEMORY_DIAGNOSTICS"], "0")
        for key in ("BOT_DEBUG", "BOT_CPU", "MallocStackLogging"):
            self.assertNotIn(key, clean)
        for key in ("BOT_SCHEDULING_PROFILE", "BOT_RENDER_PROFILE", "BOT_RESPONSIVENESS_PROFILE", "BOT_RESPONSIVENESS_FINE", "BOT_GPU_COMPLETION_PROFILE"):
            self.assertEqual(clean[key], "0")

    def test_server_identity_requires_fresh_declared_server_and_real_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "server.json"
            valid = {"pid": 42, "start_identity": "start:42", "port_listen": 43594,
                     "config_sha256": "a" * 64, "fixture_manifest_sha256": "b" * 64,
                     "public_key_sha256": "c" * 64, "configuration": {
                         "bind_host": "127.0.0.1", "world_json_sha256": "a" * 64,
                         "fixture_manifest_sha256": "b" * 64,
                     }}
            path.write_text(json.dumps(valid))
            self.assertEqual(runner.validate_server_identity(path, 42, "start:42")["pid"], 42)
            valid["configuration"]["fixture_manifest_sha256"] = "bad"
            path.write_text(json.dumps(valid))
            with self.assertRaises(runner.CalibrationError):
                runner.validate_server_identity(path, 42, "start:42")

    def test_memory_guard_triggers_cleanup_callback(self):
        seen = []
        values = iter([None, runner.MEM_AVAILABLE_GUARD_BYTES - 1])
        guard = runner.MemoryGuard(seen.append, reader=lambda: next(values))
        guard.start()
        guard.thread.join(timeout=2)
        guard.close()
        self.assertEqual(len(seen), 1)
        self.assertIn("MemAvailable", seen[0])

    def test_memory_guard_fails_closed_when_meminfo_unavailable(self):
        seen = []
        guard = runner.MemoryGuard(seen.append, reader=lambda: None)
        guard.start()
        guard.thread.join(timeout=2)
        guard.close()
        self.assertEqual(len(seen), 1)
        self.assertIn("unavailable", seen[0])

    def test_launch_environment_binds_explicit_operator_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            server = root / "server"
            (server / "data/config").mkdir(parents=True)
            (server / "server-login-public.json").write_text(json.dumps({
                "modulus_decimal": "123", "exponent_decimal": "65537"}))
            args = argparse.Namespace(n=1, server_root=server, rs2b0t=root / "rs2b0t",
                                     nav_pack=root / "nav", nav_flags=root / "flags")
            env = runner.launch_environment(args)
            self.assertEqual(env["ENGINE_DIR"], str(server.resolve()))
            self.assertEqual(env["RS2B0T"], str((root / "rs2b0t").resolve()))
            self.assertEqual(env["NAV_PACK"], str((root / "nav").resolve()))
            self.assertEqual(env["LOGIN_RSAN"], "123")
            self.assertEqual(env["BOT_MEMORY_N"], "1")

    def test_launch_environment_preserves_operator_context_from_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            server = root / "server"
            (server / "data/config").mkdir(parents=True)
            (server / "server-login-public.json").write_text(json.dumps({
                "modulus_decimal": "123", "exponent_decimal": "65537"}))
            args = argparse.Namespace(server_root=server, rs2b0t=root / "rs2b0t",
                                     nav_pack=root / "nav", nav_flags=root / "flags")
            base = {"PATH": "/usr/bin", "HOME": "/operator", "TERM": "xterm",
                    "BOT_DEBUG": "1", "BOT_CPU": "1"}
            env = runner.launch_environment(args, base=base)
            self.assertEqual(env["PATH"], "/usr/bin")
            self.assertEqual(env["HOME"], "/operator")
            self.assertEqual(env["TERM"], "xterm")
            self.assertNotIn("BOT_DEBUG", env)
            self.assertNotIn("BOT_CPU", env)

    def test_feature_contract_rejects_counting_or_snapshot_dedup(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "manifest.json"
            path.write_text(json.dumps({"features": {
                "requested": "memory-profile-no-alloc", "locked": True,
                "allocation_counting": True, "enabled": "snapshot-dedup"}}))
            with self.assertRaises(runner.CalibrationError):
                runner.validate_feature_contract(path)

    def test_preflight_only_has_no_launch_or_output_reservation(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = mock.Mock(output=pathlib.Path(tmp) / "future.json", n=16)
            fake_spec = {"id": "future", "launcher_argv": ["never"]}
            with mock.patch.object(runner, "validate_inputs", return_value={"host_commit": runner.EXPECTED_HOST}), \
                 mock.patch.object(runner, "build_spec", return_value=fake_spec), \
                 mock.patch.object(runner.rmc, "run_managed_cell") as managed:
                args.preflight_only = True
                with mock.patch.object(runner, "parser", return_value=mock.Mock(parse_args=lambda _: args)):
                    self.assertEqual(runner.main([]), 0)
                managed.assert_not_called()
            self.assertFalse(args.output.exists())
            self.assertFalse(args.output.with_suffix(args.output.suffix + ".spec.json").exists())

    def test_incomplete_managed_state_is_failed_and_never_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / "result.json"
            args = mock.Mock(output=output, n=16)
            spec = {"id": "result"}
            with mock.patch.object(runner, "launch_environment", return_value={}), \
                 mock.patch.object(runner.rmc, "run_managed_cell", return_value={"status": "failed_or_unavailable", "launched": True, "attempts": 1}):
                self.assertEqual(runner.run(args, spec), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "failed_or_unavailable")
            self.assertFalse(report["performance_acceptance"])
            self.assertEqual(report["memory_guard"]["status"], "not_triggered")

    def test_run_receipt_retains_selected_n_after_managed_runner(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            output = root / "result.json"
            args = argparse.Namespace(output=output, host_checkout=root, n=1)
            managed_report = {"status": "failed_or_unavailable", "launched": False, "attempts": 1}
            with mock.patch.object(runner, "launch_environment", return_value={}), \
                 mock.patch.object(runner.rmc, "run_managed_cell", return_value=managed_report):
                self.assertEqual(runner.run(args, {"id": "result"}), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["n"], 1)

    def test_existing_output_is_refused_before_managed_launch(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / "result.json"
            output.write_text("existing")
            args = mock.Mock(output=output, n=16)
            with self.assertRaisesRegex(runner.CalibrationError, "existing output"):
                runner.run(args, {"id": "result"})

    def test_run_passes_preserved_context_to_managed_runner(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / "result.json"
            args = mock.Mock(output=output, host_checkout=pathlib.Path(tmp), n=16)
            captured = {}
            def managed(*_args, **_kwargs):
                captured.update({key: os.environ.get(key) for key in ("PATH", "HOME", "TERM", "BOT_DEBUG")})
                return {"status": "failed_or_unavailable", "launched": False, "attempts": 1}
            with mock.patch.dict(os.environ, {"PATH": "/operator/bin", "HOME": "/operator", "TERM": "xterm", "BOT_DEBUG": "1"}, clear=True), \
                 mock.patch.object(runner, "launch_environment", side_effect=lambda _args, base=None: runner.clean_environment(base)), \
                 mock.patch.object(runner.rmc, "run_managed_cell", side_effect=managed):
                self.assertEqual(runner.run(args, {"id": "result"}), 1)
            self.assertEqual(captured["PATH"], "/operator/bin")
            self.assertEqual(captured["HOME"], "/operator")
            self.assertEqual(captured["TERM"], "xterm")
            # The controller passes a child-only environment; it must not
            # mutate the controller's inherited environment.
            self.assertEqual(captured["BOT_DEBUG"], "1")

    def test_managed_exception_does_not_claim_no_launch(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / "result.json"
            args = mock.Mock(output=output, host_checkout=pathlib.Path(tmp), n=16)
            with mock.patch.object(runner, "launch_environment", return_value={}), \
                 mock.patch.object(runner.rmc, "run_managed_cell", side_effect=RuntimeError("cancelled after launch")):
                self.assertEqual(runner.run(args, {"id": "result"}), 1)
            report = json.loads(output.read_text())
            self.assertNotEqual(report["launched"], False)
            self.assertTrue(report["execution_attempted"])

    def test_preflight_samples_declared_server_identity(self):
        args = mock.Mock(server_identity=pathlib.Path("server.json"), server_pid=42,
                         server_start_identity="start:42")
        identity = {"pid": 42, "start_identity": "start:42"}
        with mock.patch.object(runner, "validate_server_identity", return_value=identity), \
             mock.patch.object(runner.sr, "sample_process", return_value=identity) as sample:
            runner.validate_live_server(args)
        sample.assert_called_once_with(42, timeout=5.0)

    def test_preflight_rejects_server_pid_start_mismatch(self):
        args = mock.Mock(server_identity=pathlib.Path("server.json"), server_pid=42,
                         server_start_identity="start:42")
        identity = {"pid": 42, "start_identity": "start:42"}
        with mock.patch.object(runner, "validate_server_identity", return_value=identity), \
             mock.patch.object(runner.sr, "sample_process", return_value={"pid": 42, "start_identity": "reused"}):
            with self.assertRaisesRegex(runner.CalibrationError, "live server identity"):
                runner.validate_live_server(args)

    def test_memory_guard_interrupts_managed_call_and_owned_child_is_cleaned(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / "result.json"
            child = []
            managed_finished = threading.Event()
            args = mock.Mock(output=output, host_checkout=pathlib.Path(tmp), n=16,
                             rs2b0t=pathlib.Path(tmp), nav_pack=pathlib.Path(tmp),
                             nav_flags=pathlib.Path(tmp), server_root=pathlib.Path(tmp))
            def managed(*_args, **_kwargs):
                import subprocess
                proc = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(0.1)"])
                child.append(proc)
                try:
                    time.sleep(2)
                    managed_finished.set()
                finally:
                    if proc.poll() is None:
                        proc.terminate()
                        proc.wait(timeout=2)
                return {"status": "completed", "attempts": 1}
            args.host_checkout = pathlib.Path(tmp)
            actual_guard = runner.MemoryGuard
            reader = mock.Mock(return_value=None)
            def injected_guard(on_trigger):
                return actual_guard(on_trigger, reader=reader)
            with mock.patch.object(runner, "launch_environment", return_value={}), \
                 mock.patch.object(runner.rmc, "run_managed_cell", side_effect=managed), \
                 mock.patch.object(runner, "MEM_GUARD_INTERVAL_S", 0.01), \
                 mock.patch.object(runner, "MemoryGuard", side_effect=injected_guard):
                self.assertEqual(runner.run(args, {"id": "result"}), 1)
            self.assertTrue(child)
            self.assertIsNotNone(child[0].poll())
            self.assertFalse(managed_finished.is_set())
            reader.assert_called()
            self.assertEqual(json.loads(output.read_text())["memory_guard"]["status"], "triggered")

    def test_source_mismatch_fails_before_build_or_launch(self):
        with tempfile.TemporaryDirectory() as tmp:
            host = pathlib.Path(tmp) / "host"
            client = host / "vendor" / "fr-client-rust"
            client.mkdir(parents=True)
            (host / ".gitmodules").write_text("submodule")
            with mock.patch.object(runner, "_git", side_effect=["wrong-host", runner.EXPECTED_CLIENT]):
                with self.assertRaisesRegex(runner.CalibrationError, "host commit"):
                    runner.check_source(host, runner.EXPECTED_HOST, runner.EXPECTED_CLIENT)

    def test_direct_source_cleanliness_includes_untracked_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            host = pathlib.Path(tmp) / 'host'
            client = host / 'vendor' / 'fr-client-rust'
            client.mkdir(parents=True)
            (host / '.gitmodules').write_text('submodule')
            replies = [
                runner.DIRECT_DIAGNOSTIC_HOST, runner.DIRECT_DIAGNOSTIC_CLIENT,
                '?? untracked-generated-file',
            ]
            with mock.patch.object(runner, '_git', side_effect=replies) as git:
                with self.assertRaisesRegex(runner.CalibrationError, 'dirty'):
                    runner.check_source(
                        host, runner.DIRECT_DIAGNOSTIC_HOST,
                        runner.DIRECT_DIAGNOSTIC_CLIENT, include_untracked=True,
                    )
            self.assertEqual(git.call_args_list[-1].args[1:], ('status', '--porcelain'))


if __name__ == "__main__":
    unittest.main()
