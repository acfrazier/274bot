import ast
import json
import os
import pathlib
import re
import sys
import subprocess
import types
import unittest

HERE = pathlib.Path(__file__).parent
CONTROLLER = HERE / "run-tile-boxed-focused-one.py"
HOST_MEMORY = HERE.parent
sys.path.insert(0, str(HOST_MEMORY))
import run_managed_cell as rmc


def function(name):
    tree = ast.parse(CONTROLLER.read_text())
    node = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == name)
    namespace = {}
    exec(compile(ast.Module(body=[node], type_ignores=[]), str(CONTROLLER), "exec"), namespace)
    return namespace[name]


class LongBoxedTileControls(unittest.TestCase):
    def test_actual_argv_parser_and_callback_contract(self):
        argv = function("diagnostic_argv")("panel.exe", "manifest.json", "reference", "focused-one")
        args = rmc.parse_diagnostic_argv(argv)
        self.assertEqual(args.n, 16)
        self.assertEqual(args.warmup, 30)
        self.assertEqual(args.observe, 600)
        self.assertTrue(args.focused_one)
        self.assertFalse(args.focused_background)
        self.assertFalse(args.cpu_fallback)
        self.assertFalse(args.nav_captures)
        self.assertTrue(args.no_diagnostics)
        self.assertTrue(args.failure_capture)
        self.assertTrue(args.render_profile)
        self.assertTrue(args.gpu_completion_profile)

    def test_generated_spec_has_long_namespace_and_full_server_binding(self):
        source = CONTROLLER.read_text()
        tree = ast.parse(source)
        spec_node = next(n for n in tree.body if isinstance(n, ast.Assign) and getattr(n.targets[0], "id", None) == "spec")
        diag = function("diagnostic_argv")("panel.exe", "manifest.json", "reference", "focused-one")
        namespace = {
            "role_info": {"manifest_role": "reference"},
            "role": "baseline", "mode": "focused-one", "cell_id": "baseline-focused-one-long",
            "binary": pathlib.Path("C:/fixture/panel-play.exe"), "manifest": pathlib.Path("C:/fixture/build-manifest.json"),
            "server_id": pathlib.Path("C:/fixture/server-identity.json"), "conditions": pathlib.Path("C:/fixture/conditions.json"),
            "catalog": pathlib.Path("C:/fixture/catalog.json"), "diag": diag, "pid": 7,
            "server": pathlib.Path("C:/fixture/server"), "home": pathlib.Path("C:/fixture/home"),
            "sys": types.SimpleNamespace(executable="python"), "mem": pathlib.Path("C:/fixture/docs/memory"),
            "os": types.SimpleNamespace(environ={"NAV_PACK": "C:/fixture/nav", "NAV_FLAGS": "C:/fixture/flags"}, getppid=lambda: 1),
        }
        spec = eval(compile(ast.Expression(spec_node.value), str(CONTROLLER), "eval"), namespace)
        self.assertEqual(spec["id"], "native-panel-tile-boxed-long-baseline-focused-one-long")
        self.assertEqual((spec["count"], spec["mode"], spec["observe_s"], spec["warmup_s"], spec["teardown_grace_s"]), (16, "focused-one", 600, 30, 60))
        self.assertEqual(spec["requested_backend"], "gpu")
        self.assertIn("server_identity_path", spec)
        self.assertIn("--observe", spec["diagnostic_argv"])
        self.assertEqual(spec["diagnostic_argv"][spec["diagnostic_argv"].index("--observe") + 1], "600")

    def test_negative_duration_mode_backend_and_missing_server_configuration_fail(self):
        good = function("diagnostic_argv")("panel.exe", "manifest.json", "reference", "focused-one")
        parsed = rmc.parse_diagnostic_argv(good)
        self.assertEqual(parsed.observe, 600)
        for bad in (good + ["--cpu-fallback"],
                    good + ["--owner-census"],):
            with self.assertRaises(Exception):
                rmc.parse_diagnostic_argv(bad)
        contract = ast.parse((HERE / "check-tile-probe-contract.py").read_text())
        node = next(n for n in contract.body if isinstance(n, ast.FunctionDef) and n.name == "server_configuration_complete")
        ns = {"re": re}
        exec(compile(ast.Module(body=[node], type_ignores=[]), "check-tile-probe-contract.py", "exec"), ns)
        valid = {key: "a" * 64 for key in ("world_json_sha256", "maps_addition_sha256", "wordenc_addition_sha256")}
        valid.update(bind_host="127.0.0.1", node_version="24.19.0")
        self.assertTrue(ns["server_configuration_complete"]({"configuration": valid}))
        for key in ("world_json_sha256", "maps_addition_sha256", "wordenc_addition_sha256"):
            malformed = dict(valid)
            malformed[key] = "x"
            self.assertFalse(ns["server_configuration_complete"]({"configuration": malformed}))
        argv_contract = next(n for n in contract.body if isinstance(n, ast.FunctionDef) and n.name == "long_argv_contract_complete")
        ns = {"rmc": rmc, "re": __import__("re")}
        exec(compile(ast.Module(body=[argv_contract], type_ignores=[]), "check-tile-boxed-long-controls.py", "exec"), ns)
        self.assertTrue(ns["long_argv_contract_complete"](parsed, good))
        for field, value in (("observe", 120), ("warmup", 120), ("focused_one", False), ("cpu_fallback", True), ("nav_captures", True), ("render_profile", False), ("gpu_completion_profile", False)):
            altered = types.SimpleNamespace(**vars(parsed))
            setattr(altered, field, value)
            self.assertFalse(ns["long_argv_contract_complete"](altered), field)
        legacy = types.SimpleNamespace(**{key: value for key, value in vars(parsed).items() if key != "cpu_fallback"})
        self.assertTrue(ns["long_argv_contract_complete"](legacy, good))
        self.assertFalse(ns["long_argv_contract_complete"](legacy, good + ["--cpu-fallback"]))

    def test_no_launch_callback_reads_generated_spec_and_does_not_launch(self):
        contract = ast.parse((HERE / "check-tile-probe-contract.py").read_text())
        node = next(n for n in contract.body if isinstance(n, ast.FunctionDef) and n.name == "no_launch")
        with __import__("tempfile").TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            server_path = root / "server.json"
            conditions_path = root / "conditions.json"
            spec_path = root / "spec.json"
            digests = {key: "b" * 64 for key in ("world_json_sha256", "maps_addition_sha256", "wordenc_addition_sha256")}
            digests.update(bind_host="127.0.0.1", node_version="24.19.0")
            server_path.write_text(json.dumps({"configuration": digests}))
            conditions_path.write_text(json.dumps({"native": True}))
            argv = function("diagnostic_argv")("panel.exe", "manifest.json", "reference", "focused-one")
            spec = {"id": "native-panel-tile-boxed-long-" + "baseline-focused-one-long-contractcheck",
                    "count": 16, "mode": "focused-one", "warmup_s": 30, "observe_s": 600, "teardown_grace_s": 60,
                    "requested_backend": "gpu", "requested_adapter": "Intel(R) Graphics",
                    "diagnostic_argv": argv, "server_identity_path": str(server_path), "host_conditions_path": str(conditions_path),
                    "cell_id": "baseline-focused-one-long-contractcheck"}
            spec_path.write_text(json.dumps(spec))
            class FakeRmc:
                @staticmethod
                def load_spec(path): return json.loads(path.read_text())
                @staticmethod
                def validate_spec(value): return value
                @staticmethod
                def parse_diagnostic_argv(value): return rmc.parse_diagnostic_argv(value)
                @staticmethod
                def require_argv_consistent_with_spec(*args): return None
                @staticmethod
                def preflight(*args): return {"server_pid": 123}
            fake_mea = types.SimpleNamespace(_native_windows_conditions_complete=lambda value: value == {"native": True})
            (root / "274bot-runs").mkdir()
            old_home = os.environ.get("HOME")
            os.environ["HOME"] = str(root)
            ns = {"json": json, "re": re, "os": types.SimpleNamespace(environ={}), "pathlib": pathlib,
                  "rmc": FakeRmc, "mea": fake_mea, "contract_id": spec["cell_id"], "target_cell_id": "baseline-focused-one-long",
                  "cell_id": spec["cell_id"], "checks": [], "long_argv_contract_complete": None}
            functions = [n for n in contract.body if isinstance(n, ast.FunctionDef) and n.name in {"server_configuration_complete", "long_argv_contract_complete", "no_launch"}]
            exec(compile(ast.Module(body=functions, type_ignores=[]), str(HERE / "check-tile-boxed-long-controls.py"), "exec"), ns)
            try:
                self.assertEqual(ns["no_launch"]([str(spec_path)]), 0)
                self.assertEqual(ns["checks"][0]["launched"], False)
            finally:
                if old_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = old_home

    def test_exact_original_native_parser_accepts_long_argv(self):
        original_source = subprocess.run(
            ["git", "show", "35eab6c^:docs/memory/run_diagnostic.py"],
            cwd=HERE.parents[3], check=True, capture_output=True, text=True,
        ).stdout
        original = ast.parse(original_source)
        nodes = [next(n for n in original.body if isinstance(n, ast.FunctionDef) and n.name == name)
                 for name in ("build_parser", "validate_args")]
        ns = {"argparse": __import__("argparse"), "pathlib": pathlib}
        exec(compile(ast.Module(body=nodes, type_ignores=[]), "run_diagnostic.py@35eab6c^", "exec"), ns)
        argv = function("diagnostic_argv")("panel.exe", "manifest.json", "reference", "focused-one")
        parser = ns["build_parser"]()
        args = parser.parse_args(argv)
        ns["validate_args"](args, parser)
        self.assertEqual((args.n, args.warmup, args.observe, args.focused_one), (16, 30, 600, True))
        self.assertFalse(hasattr(args, "cpu_fallback"))
        contract = ast.parse((HERE / "check-tile-probe-contract.py").read_text())
        node = next(n for n in contract.body if isinstance(n, ast.FunctionDef) and n.name == "long_argv_contract_complete")
        contract_ns = {"re": re}
        exec(compile(ast.Module(body=[node], type_ignores=[]), "check-tile-boxed-long-controls.py", "exec"), contract_ns)
        self.assertTrue(contract_ns["long_argv_contract_complete"](args, argv))

    def test_power_shell_helpers_are_parse_targets_and_namespace_is_distinct(self):
        expected = {"prepare-tile-probe.ps1", "stage-contract-tile-probe.ps1", "run-contractcheck-tile-probe.ps1", "preflight-tile-boxed.ps1", "launch-tile-boxed.ps1", "poll-tile-boxed.ps1", "archive-tile-boxed.ps1", "validate-tile-boxed-ast.ps1"}
        self.assertTrue(expected.issubset({p.name for p in HERE.iterdir()}))
        for path in HERE.glob("*.ps1"):
            text = path.read_text()
            self.assertNotIn("TILE_BOXED_BUILD_ROLE", text)
            self.assertNotIn("managed-tile-boxed-'", text)
            self.assertIn("long", text.lower())

    def test_role_hashes_and_no_cpu_cleaning_remain_frozen(self):
        source = CONTROLLER.read_text()
        self.assertIn("9268890217d968cfeb7c66ebb11dd5c3dd2c084f", source)
        self.assertIn("fb3589ac28583242b999ac864ea69c4ef8fa5923", source)
        self.assertIn("e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5", source)
        self.assertIn("a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f", source)
        clean = function("clean_environment")
        env = {"BOT_CPU": "1", "BOT_RENDER_OWNER_CENSUS": "1", "BOT_DEBUG": "1"}
        clean.__globals__["os"] = types.SimpleNamespace(environ=env)
        clean()
        self.assertEqual(env, {})


if __name__ == "__main__":
    unittest.main()
