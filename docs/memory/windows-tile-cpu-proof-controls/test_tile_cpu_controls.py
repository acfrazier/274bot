import ast
import pathlib
import types
import unittest
import tempfile
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
