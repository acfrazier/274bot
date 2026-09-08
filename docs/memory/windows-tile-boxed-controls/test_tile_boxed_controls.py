import ast
import pathlib
import types
import unittest

HERE = pathlib.Path(__file__).parent
CONTROLLER = HERE / "run-tile-boxed-focused-one.py"


def expression(name, namespace):
    tree = ast.parse(CONTROLLER.read_text())
    for node in tree.body:
        if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name) and node.targets[0].id == name:
            return eval(compile(ast.Expression(node.value), str(CONTROLLER), "eval"), namespace)
    raise AssertionError(name)


class BoxedTileControls(unittest.TestCase):
    def test_controller_has_actual_pair_roles_and_frozen_identities(self):
        source = CONTROLLER.read_text()
        tree = ast.parse(source)
        roles = next(node.value for node in tree.body if isinstance(node, ast.Assign) and getattr(node.targets[0], "id", None) == "ROLES")
        value = eval(compile(ast.Expression(roles), str(CONTROLLER), "eval"), {"pathlib": pathlib})
        self.assertEqual(value["baseline"]["host_commit"], "9268890217d968cfeb7c66ebb11dd5c3dd2c084f")
        self.assertEqual(value["candidate"]["host_commit"], "fb3589ac28583242b999ac864ea69c4ef8fa5923")
        self.assertEqual(value["baseline"]["binary_sha256"], "e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5")
        self.assertEqual(value["candidate"]["binary_sha256"], "a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f")
        self.assertEqual(value["baseline"]["host_sources_file_count"], 871)
        self.assertEqual(value["candidate"]["host_sources_file_count"], 876)
        self.assertNotIn("5ee9b6efb2342452ceeb7958f864fd68d0daadd1", source)

    def test_generated_spec_uses_clean_argv_for_both_roles(self):
        common = {
            "role_info": {"manifest_role": "reference"}, "role": "baseline", "mode": "focused-one",
            "cell_id": "baseline-focused-one-fixture", "binary": pathlib.Path("/fixture/panel-play.exe"),
            "manifest": pathlib.Path("/fixture/build-manifest.json"), "server_id": pathlib.Path("/fixture/server.json"),
            "conditions": pathlib.Path("/fixture/conditions.json"), "catalog": pathlib.Path("/fixture/catalog.json"),
            "mem": pathlib.Path("/fixture/docs/memory"), "diag": ["panel", "16", "active", "--warmup", "30", "--observe", "120", "--focused-one", "--no-diagnostics", "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"],
            "pid": 7, "server": pathlib.Path("/fixture/server"), "home": pathlib.Path("/fixture/home"),
            "sys": types.SimpleNamespace(executable="python"), "os": types.SimpleNamespace(environ={"NAV_PACK": "/fixture/nav", "NAV_FLAGS": "/fixture/flags"}, getppid=lambda: 1),
        }
        spec = expression("spec", common)
        self.assertEqual(spec["cell_id"], "baseline-focused-one-fixture")
        self.assertEqual(spec["build_role"], "reference")
        self.assertEqual(spec["observe_s"], 120)
        self.assertEqual(spec["warmup_s"], 30)
        self.assertNotIn("--nav-captures", spec["diagnostic_argv"])
        self.assertIn("--failure-capture", spec["diagnostic_argv"])

    def test_actual_controller_argv_and_environment_seams(self):
        tree = ast.parse(CONTROLLER.read_text())
        functions = {n.name: n for n in tree.body if isinstance(n, ast.FunctionDef)}
        namespace = {"os": types.SimpleNamespace(environ={"BOT_RENDER_OWNER_CENSUS": "1", "BOT_DEBUG": "1"})}
        for name in ("clean_environment", "diagnostic_argv"):
            exec(compile(ast.Module(body=[functions[name]], type_ignores=[]), str(CONTROLLER), "exec"), namespace)
        namespace["clean_environment"]()
        self.assertNotEqual(namespace["os"].environ.get("BOT_RENDER_OWNER_CENSUS"), "1")
        for role in ("reference", "candidate"):
            for mode in ("focused-one", "focused-plus-background"):
                argv = namespace["diagnostic_argv"]("panel.exe", "manifest.json", role, mode)
                self.assertIn("--no-diagnostics", argv)
                self.assertIn("--failure-capture", argv)
                self.assertNotIn("--nav-captures", argv)

    def test_control_contract_and_archive_require_raw_runs(self):
        prepare = (HERE / "prepare-tile-probe.ps1").read_text()
        contract = (HERE / "run-contractcheck-tile-probe.ps1").read_text()
        archive = (HERE / "archive-tile-boxed.ps1").read_text()
        self.assertIn("client_started -ne $false", prepare)
        self.assertIn("check-tile-probe-contract.py", contract)
        self.assertIn("run-tile-boxed-focused-one.py", contract)
        self.assertIn("preflight-tile-boxed-", contract)
        self.assertNotIn("RENDER_OWNER_CENSUS_", contract)
        self.assertNotIn("preflight-tile-probe-", contract)
        self.assertNotIn("run-panel-tile-probe", contract)
        self.assertIn("rawSources", archive)
        self.assertIn("raw-run-{0:d2}", archive)
        self.assertIn("performanceAcceptance=$false", archive)
        check = (HERE / "check-tile-probe-contract.py").read_text()
        self.assertIn('os.environ.get("BOT_RENDER_OWNER_CENSUS") != "1"', check)
        self.assertIn('contract_id == cell_id + "-contractcheck"', check)

    def test_each_control_is_present_and_python_parses(self):
        for path in HERE.iterdir():
            if path.suffix == ".py" and path.name != pathlib.Path(__file__).name:
                ast.parse(path.read_text())
        expected = {"prepare-tile-probe.ps1", "run-contractcheck-tile-probe.ps1", "preflight-tile-boxed.ps1", "launch-tile-boxed.ps1", "poll-tile-boxed.ps1", "archive-tile-boxed.ps1"}
        self.assertTrue(expected.issubset({p.name for p in HERE.iterdir()}))


if __name__ == "__main__":
    unittest.main()
