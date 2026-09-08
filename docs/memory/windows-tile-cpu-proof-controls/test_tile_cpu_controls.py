import ast
import pathlib
import types
import unittest
import tempfile
import json
import os
import runpy
HERE = pathlib.Path(__file__).parent
CONTROLLER = HERE / "run-tile-cpu-focused-one.py"
class CpuControls(unittest.TestCase):
    def test_parser_namespace_and_explicit_cpu_backend(self):
        tree = ast.parse(CONTROLLER.read_text())
        funcs = {node.name: node for node in tree.body if isinstance(node, ast.FunctionDef)}
        namespace = {"os": types.SimpleNamespace(environ={"BOT_CPU": "inherited", "BOT_DEBUG": "1", "BOT_RENDER_OWNER_CENSUS": "1"})}
        exec(compile(ast.Module(body=[funcs["clean_environment"], funcs["diagnostic_argv"]], type_ignores=[]), str(CONTROLLER), "exec"), namespace)
        namespace["clean_environment"]()
        self.assertNotIn("BOT_CPU", namespace["os"].environ)
        import sys
        sys.path.insert(0, str(CONTROLLER.parents[1]))
        import run_diagnostic as rd
        argv = namespace["diagnostic_argv"]("panel.exe", "manifest.json", "reference", "focused-one")
        args = rd.build_parser().parse_args(argv)
        rd.validate_args(args, rd.build_parser())
        self.assertIsInstance(args, __import__("argparse").Namespace)
        self.assertTrue(args.cpu_fallback)
        self.assertEqual(rd.requested_backend(args), "cpu_fallback")

    def test_real_parser_and_backend_seams(self):
        source = CONTROLLER.read_text()
        tree = ast.parse(source)
        self.assertIn('"--cpu-fallback"', source)
        self.assertIn('"--nav-captures"', source)
        self.assertNotIn('"--gpu-completion-profile"', source)
        self.assertIn('isinstance(args, argparse.Namespace)', source)
        self.assertIn('requested_backend(args) == "cpu_fallback"', source)
        self.assertIn('clean_environment()', source)
        self.assertTrue(any(isinstance(n, ast.FunctionDef) and n.name == "diagnostic_argv" for n in tree.body))
    def test_identity_and_n1_timing_contract(self):
        source = CONTROLLER.read_text()
        for value in ("9268890217d968cfeb7c66ebb11dd5c3dd2c084f", "fb3589ac28583242b999ac864ea69c4ef8fa5923", "e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5", "a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f"):
            self.assertIn(value, source)
        self.assertIn('"count": 1', source)
        self.assertIn('"warmup_s": 30', source)
        self.assertIn('"observe_s": 120', source)
        self.assertIn('"teardown_grace_s": 60', source)
    def test_contract_is_actual_controller_assembly(self):
        source = (HERE / "check-tile-cpu-contract.py").read_text()
        self.assertIn("rmc.validate_spec", source)
        self.assertIn("rmc.parse_diagnostic_argv", source)
        self.assertIn("rmc.preflight", source)
        self.assertIn("isinstance(args, __import__(\"argparse\").Namespace)", source)
        self.assertIn("rmc.require_argv_consistent_with_spec", source)
        self.assertIn("launched\": False", source)
        self.assertIn("except SystemExit as exc", source)
        self.assertIn("_native_windows_conditions_complete", source)
        self.assertIn("TILE_CPU_PREFLIGHT_CELL_ID", source)

    def test_contract_execution_path_writes_distinct_receipt(self):
        source = (HERE / "check-tile-cpu-contract.py").read_text()
        stage = (HERE / "stage-contract-tile-cpu.ps1").read_text()
        runner = (HERE / "run-contractcheck-tile-cpu.ps1").read_text()
        self.assertIn("runpy.run_path", source)
        self.assertIn("out.write_text", source)
        self.assertIn('"launched": False', source)
        self.assertIn('"functional_only":True', source)
        self.assertIn("run-tile-cpu-focused-one.py", stage)
        self.assertIn("('run-'+$contractId+'.py')", stage)
        self.assertIn("Copy-Item $source $destination", stage)
        self.assertIn("Get-FileHash $runner", runner)
        self.assertIn("run-tile-cpu-focused-one.py", runner)
        self.assertNotIn("Staged contract checker missing", runner)
        self.assertIn("$env:TILE_CPU_PREFLIGHT_CELL_ID=$CellId", runner)

    def test_contract_stage_chain_uses_target_and_contract_runner_names(self):
        stage = (HERE / "stage-tile-cpu.ps1").read_text()
        contract_stage = (HERE / "stage-contract-tile-cpu.ps1").read_text()
        contract_run = (HERE / "run-contractcheck-tile-cpu.ps1").read_text()
        target = "baseline-focused-one"
        contract = target + "-contractcheck"
        self.assertIn("('run-'+$CellId+'.py')", stage)
        self.assertIn("('preflight-tile-cpu-'+$CellId+'.json')", stage)
        self.assertIn("('run-'+$contractId+'.py')", contract_stage)
        self.assertIn("$source=Join-Path $PSScriptRoot 'run-tile-cpu-focused-one.py'", contract_stage)
        self.assertIn("$runner=Join-Path $stage ('run-'+$contract+'.py')", contract_run)
        self.assertIn("(Join-Path $PSScriptRoot 'check-tile-cpu-contract.py')", contract_run)
        self.assertNotIn("check-tile-cpu-contract-$contract", contract_stage)
        self.assertEqual(contract, "baseline-focused-one-contractcheck")

    def test_contract_executes_no_launch_and_writes_receipt(self):
        import sys
        sys.path.insert(0, str(HERE.parent))
        import run_managed_cell as rmc
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            stage = root / "stage"
            stage.mkdir()
            runs = root / "runs"
            spec_path = root / "spec.json"
            conditions = {
                "purpose": "CPU contract test", "platform": "Windows test", "user": "BotTest",
                "builds_stopped_before_run": True, "terminal_transport_expected": False,
                "panel_render_attribution": True, "backend": "cpu_fallback", "performance_acceptance": False,
                "native_preflight": {"adapter": "test", "utc": "now", "consoleSessionId": 1,
                    "vm": {"Name": "test", "State": "Off", "MemoryAssigned": 0},
                    "quietServices": [{"Name": "test", "Status": "Stopped"}],
                    "drivers": [{"Name": "test", "DriverVersion": "1", "PNPDeviceID": "test"}],
                    "sessions": ["session"], "processes": [{"ProcessId": 1, "ParentProcessId": 2,
                        "Name": "test", "SessionId": 1, "CreationDate": "now", "CommandLine": "test"}],
                    "processLasso": {"running": False, "processes": []},
                    "dxdiag": [{"cardName": "test", "driverVersion": "1"}],
                    "server": {"ProcessId": 101, "Name": "node.exe", "CreationDate": "now"},
                    "performanceAcceptance": False},
            }
            for name in ("binary", "manifest", "server", "nav", "flags", "catalog"):
                (root / name).write_text("{}\n")
            diag = ["panel", "1", "active", "--binary", str(root / "binary"), "--build-manifest", str(root / "manifest"),
                    "--build-role", "reference", "--sustain", "--warmup", "30", "--observe", "120", "--focused-one",
                    "--cpu-fallback", "--nav-captures", "--failure-capture", "--no-diagnostics", "--render-profile",
                    "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine"]
            spec = {"id": "baseline-focused-one-contractcheck", "index": 1, "kind": "diagnostic", "frontend": "panel",
                    "binary": str(root / "binary"), "build_manifest": str(root / "manifest"),
                    "server_identity_path": str(root / "server"), "host_conditions_path": str(root / "conditions"),
                    "nav_pack": str(root / "nav"), "nav_flags": str(root / "flags"), "catalog_path": str(root / "catalog"),
                    "build_role": "reference", "launcher_argv": [sys.executable, str(HERE.parent / "run_diagnostic.py"), *diag],
                    "diagnostic_argv": diag, "game_server_pid": 101, "ambient_helpers": {"bootstrap": 202},
                    "sampler_interval_s": 0.5, "max_wall_s": 900, "observe_s": 120, "warmup_s": 30,
                    "teardown_grace_s": 60, "requested_backend": "cpu_fallback", "cpu_fallback": True,
                    "cache_dir": str(root / "cache"), "unpack_root": str(root / "unpack")}
            (root / "conditions").write_text(json.dumps(conditions))
            (root / "cache").mkdir(); (root / "unpack").mkdir(); spec_path.write_text(json.dumps(spec))
            (stage / "run-baseline-focused-one-contractcheck.py").write_text(
                "import run_managed_cell as rmc, sys\nsys.exit(rmc.main([r'" + str(spec_path) + "']))\n")
            env = {"USERNAME": "BotTest", "TILE_CPU_CONTRACT_ID": "baseline-focused-one-contractcheck",
                   "TILE_CPU_TARGET_CELL_ID": "baseline-focused-one", "TILE_CPU_PREFLIGHT_CELL_ID": "baseline-focused-one",
                   "TILE_CPU_BUILD_ROLE": "baseline", "TILE_CPU_MODE": "focused-one",
                   "TILE_CPU_CELL_ID": "baseline-focused-one-contractcheck", "TILE_CPU_HOST_ROOT": str(HERE.parents[2]),
                   "TILE_CPU_STAGE_ROOT": str(stage), "TILE_CPU_RUNS_ROOT": str(runs)}
            old_env = os.environ.copy(); old_preflight = rmc.preflight
            os.environ.update(env); rmc.preflight = lambda actual_spec, actual_args: {"server_pid": 101}
            try:
                runpy.run_path(str(HERE / "check-tile-cpu-contract.py"), run_name="__main__")
            finally:
                rmc.preflight = old_preflight; os.environ.clear(); os.environ.update(old_env)
            receipt = json.loads((runs / "tile-cpu-contract-baseline-focused-one-contractcheck.json").read_text())
            self.assertFalse(receipt["checks"][0]["launched"])
            self.assertTrue(receipt["functional_only"])

    def test_native_shape_is_fail_closed_and_cpu_attributed(self):
        source = (HERE / "run-tile-cpu-focused-one.py").read_text()
        preflight = (HERE / "preflight-tile-cpu.ps1").read_text()
        contract = (HERE / "check-tile-cpu-contract.py").read_text()
        for field in ("panel_render_attribution", "native_preflight", "processLasso", "dxdiag", "consoleSessionId", "quietServices", "drivers", "processes"):
            self.assertIn(field, source + preflight)
        self.assertIn("mea._native_windows_conditions_complete(conditions)", contract)
        self.assertIn('conditions["backend"] == "cpu_fallback"', contract)

    def test_real_validate_spec_and_preflight_seam(self):
        import sys
        sys.path.insert(0, str(HERE.parents[1]))
        import run_managed_cell as rmc
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = {name: root / name for name in ("binary", "manifest", "server", "conditions", "nav", "flags", "catalog")}
            for path in paths.values():
                path.write_text("{}\n")
            diag = ["panel", "1", "active", "--binary", str(paths["binary"]), "--build-manifest", str(paths["manifest"]), "--build-role", "reference", "--sustain", "--warmup", "30", "--observe", "120", "--cpu-fallback", "--focused-one", "--nav-captures", "--failure-capture", "--no-diagnostics", "--render-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine"]
            args = rmc.parse_diagnostic_argv(diag)
            spec = {"id": "cpu-contract-test", "index": 1, "kind": "diagnostic", "frontend": "panel", "binary": str(paths["binary"]), "build_manifest": str(paths["manifest"]), "server_identity_path": str(paths["server"]), "host_conditions_path": str(paths["conditions"]), "nav_pack": str(paths["nav"]), "nav_flags": str(paths["flags"]), "catalog_path": str(paths["catalog"]), "build_role": "reference", "launcher_argv": [sys.executable, str(HERE.parents[1] / "run_diagnostic.py"), *diag], "diagnostic_argv": diag, "game_server_pid": 101, "ambient_helpers": {"bootstrap": 202}, "sampler_interval_s": 0.5, "max_wall_s": 900, "observe_s": 120, "warmup_s": 30, "teardown_grace_s": 60, "requested_backend": "cpu_fallback", "cache_dir": str(root / "cache"), "unpack_root": str(root / "unpack")}
            (root / "cache").mkdir()
            (root / "unpack").mkdir()
            normalized = rmc.validate_spec(spec)
            rmc.require_argv_consistent_with_spec(normalized, args, _test_launcher=True)
            seen = []
            original = rmc.preflight
            rmc.preflight = lambda actual_spec, actual_args: seen.append((actual_spec, actual_args)) or {"server_pid": actual_spec["game_server_pid"]}
            try:
                result = rmc.preflight(normalized, args)
            finally:
                rmc.preflight = original
            self.assertEqual(result["server_pid"], 101)
            self.assertEqual(len(seen), 1)
            self.assertIsInstance(seen[0][1], __import__("argparse").Namespace)
    def test_all_python_controls_parse(self):
        for path in HERE.glob("*.py"):
            ast.parse(path.read_text())
if __name__ == "__main__":
    unittest.main()
