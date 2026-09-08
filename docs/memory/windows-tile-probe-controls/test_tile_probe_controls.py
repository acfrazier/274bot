"""No-launch tests for the generated owner-census artifacts and readers."""
import ast
import copy
import hashlib
import json
import pathlib
import os
import sys
import tempfile
import types
import unittest

HOST_ROOT = pathlib.Path(os.environ.get("RENDER_OWNER_CENSUS_HOST_ROOT", pathlib.Path(__file__).resolve().parents[1]))
ROOT = HOST_ROOT / "docs/memory"
sys.path.insert(0, str(ROOT))
import build_provenance
import run_diagnostic as diagnostic
import run_managed_cell as managed
if not ROOT.is_dir():
    raise RuntimeError("BotTest host root with reviewed validators is required")
sys.path.insert(0, str(ROOT))
import matched_evidence_adapter as reader

CONTROLLER = pathlib.Path(__file__).with_name("run-panel-tile-probe-focused-one.py")
PREPARE = pathlib.Path(__file__).with_name("prepare-tile-probe.ps1")
CONTRACT_RUNNER = pathlib.Path(__file__).with_name("run-contractcheck-tile-probe.ps1")


def expression(name, namespace):
    tree = ast.parse(CONTROLLER.read_text())
    for node in tree.body:
        if name == "spec" and isinstance(node, ast.Assign) and getattr(node.targets[0], "id", None) == name:
            return eval(compile(ast.Expression(node.value), str(CONTROLLER), "eval"), namespace)
        if (isinstance(node, ast.Expr) and isinstance(node.value, ast.Call)
                and getattr(node.value.func, "id", None) == "dump"
                and isinstance(node.value.args[0], ast.Name)
                and node.value.args[0].id == name):
            return eval(compile(ast.Expression(node.value.args[1]), str(CONTROLLER), "eval"), namespace)
    raise AssertionError("missing generated expression: " + name)


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


class GeneratedContractTests(unittest.TestCase):
    def test_prepare_splits_privileged_preflight_from_bot_test_contract(self):
        prepare = PREPARE.read_text()
        runner = CONTRACT_RUNNER.read_text()
        self.assertIn("preflight-tile-probe.ps1') -CellId $contractId", prepare)
        self.assertIn("$contract.checks[0].client_started -ne $false", prepare)
        self.assertIn("$env:USERNAME -ne 'BotTest'", runner)
        self.assertIn("check-tile-probe-contract.py", runner)
        self.assertNotIn("check-tile-probe-contract.py", prepare)

    def test_frozen_identity_and_stage_constants_reject_old_census_values(self):
        source = CONTROLLER.read_text()
        self.assertIn('renderer-owner-census-9268890', source)
        self.assertIn('9268890217d968cfeb7c66ebb11dd5c3dd2c084f', source)
        self.assertIn('abb811bd0afa1acd99319ccd5bc36bfb241080f9', source)
        for stale in ('renderer-owner-census-66ba7c1', '66ba7c1abfa08bf04df44b82b087668aee1cf60a',
                      '9f72c2f93fa63af1b921248603eee133ff44fd54',
                      '2046ca2444a3389d9d0a993c156fde286dd9c9caac61c8766b65d9a21bf45d7f'):
            self.assertNotIn(stale, source)

    def test_single_cell_spec_is_complete_and_parser_accepts_argv(self):
        value = expression("spec", {
            "cell_id": "native-render-owner-census-focused-plus-background-tile-fixture",
            "binary": pathlib.Path("/fixture/panel-play.exe"),
            "manifest": pathlib.Path("/fixture/build-manifest.json"),
            "server_id": pathlib.Path("/fixture/server-identity.json"),
            "conditions": pathlib.Path("/fixture/host-conditions.json"),
            "catalog": pathlib.Path("/fixture/js-scripts.json"),
            "side": {"commit": "6" * 40},
            "mem": pathlib.Path("/fixture/docs/memory"),
            "diag": ["panel", "16", "active", "--binary", "/fixture/panel-play.exe", "--build-manifest", "/fixture/build-manifest.json", "--build-role", "reference", "--sustain", "--warmup", "30", "--observe", "120", "--focused-background", "--render-profile", "--gpu-completion-profile", "--scheduling-profile", "--responsiveness-profile", "--responsiveness-fine", "--failure-capture"],
            "pid": 6728, "server": pathlib.Path("/fixture/server"),
            "home": pathlib.Path("/fixture/home"), "sys": types.SimpleNamespace(executable="python"),
            "os": types.SimpleNamespace(environ={"NAV_PACK": "/fixture/navpack", "NAV_FLAGS": "/fixture/navflags"}, getppid=lambda: 8123),
            "CLIENT_SOURCES": "b" * 64,
        })
        normalized = managed.validate_spec(value)
        self.assertEqual(normalized["index"], 1)
        self.assertEqual(normalized["manifest_role"], "control")
        args = diagnostic.build_parser().parse_args(value["diagnostic_argv"])
        diagnostic.validate_args(args, diagnostic.build_parser())
        self.assertEqual((args.n, args.warmup, args.observe), (16, 30, 120))
        self.assertTrue(args.focused_background)
        self.assertFalse(args.nav_captures)
        broken = dict(value); broken.pop("index")
        with self.assertRaises(managed.CellError): managed.validate_spec(broken)

    def test_generated_manifest_round_trips_through_real_build_verifier(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            binary = root / "panel-play.exe"; binary.write_bytes(b"frozen fixture")
            nav_pack = root / "navpack"; nav_pack.write_bytes(b"nav")
            nav_flags = root / "navflags"; nav_flags.write_bytes(b"flags")
            catalog = root / "js-scripts.json"; catalog.write_text("[]")
            side = {"commit": "9" * 40, "branch": "codex/memory-diagnostics", "build_exit": 0, "sources_sha256_pre": "a" * 64, "sources_sha256_post": "a" * 64, "host_sources_file_count": 871, "host_sources_aggregate_sha256": "a" * 64, "sources_stable_across_build": True, "client": {"commit": "a" * 40, "sources_sha256": "b" * 64}}
            env = {"NAV_PACK": str(nav_pack), "NAV_FLAGS": str(nav_flags)}
            manifest = expression("manifest", {
                "side": side, "binary": binary, "BINARY_SHA": digest(binary),
                "nav_pack": nav_pack, "nav_flags": nav_flags, "catalog": catalog,
                "os": types.SimpleNamespace(environ=env), "sha": digest,
            })
            path = root / "build-manifest.json"; path.write_text(json.dumps(manifest))
            result = build_provenance.verify_build(path, "reference", "panel", binary, nav_pack, nav_flags, catalog)
            self.assertEqual(result["manifest_binary_key"], "control_panel_play")
            self.assertEqual(result["host_sources_sha256"], "a" * 64)
            self.assertEqual(result["client_sources_sha256"], "b" * 64)
            manifest["control"].pop("sources_sha256_pre")
            path.write_text(json.dumps(manifest))
            with self.assertRaises(ValueError): build_provenance.verify_build(path, "reference", "panel", binary, nav_pack, nav_flags, catalog)

    def test_conditions_and_server_outputs_are_strict(self):
        preflight = {"adapter": "Intel(R) Graphics", "utc": "2026-09-07T21:22:13Z", "consoleSessionId": 2,
                     "vm": {"Name": "274bot-builder", "State": "Off", "MemoryAssigned": 0},
                     "quietServices": [{"Name": "ClickToRunSvc", "Status": "Stopped"}],
                     "drivers": [{"Name": "Intel(R) Graphics", "DriverVersion": "1.0", "PNPDeviceID": "PCI\\VEN_8086"}],
                     "sessions": ["bottest console 2 Active"], "processes": [],
                     "processLasso": {"running": False, "processes": []},
                     "dxdiag": [{"cardName": "Intel(R) Graphics", "driverVersion": "1.0", "currentMode": None,
                                 "hybridGraphicsGPU": None, "monitorName": None}],
                     "server": {"ProcessId": 1, "Name": "node.exe", "CreationDate": "/Date(1)/"},
                     "performanceAcceptance": False}
        conditions = expression("conditions", {"platform": types.SimpleNamespace(platform=lambda: "Windows-11"),
            "os": types.SimpleNamespace(environ={"USERNAME": "BotTest"}), "cell_id": "native-render-owner-census-focused-plus-background-tile-fixture",
            "preflight_record": preflight})
        self.assertTrue(reader._native_windows_conditions_complete(conditions))
        for field in ("purpose", "terminal_transport_expected", "panel_render_attribution", "native_preflight"):
            broken = copy.deepcopy(conditions); broken.pop(field)
            self.assertFalse(reader._native_windows_conditions_complete(broken), field)
        broken = copy.deepcopy(conditions)
        broken["native_preflight"]["vm"].pop("MemoryAssigned")
        self.assertFalse(reader._native_windows_conditions_complete(broken))
        server = expression("server_id", {"pid": 1, "sample": {"start_identity": "filetime:123"}, "launch": {},
            "server": pathlib.Path("/fixture/server"), "sha": lambda path: "a" * 64})
        self.assertEqual(server["configuration"]["bind_host"], "127.0.0.1")
        self.assertEqual(server["port_listen"], 43594)
        self.assertEqual((server["ProcessId"], server["Name"], server["CreationDate"]), (1, "node.exe", "filetime:123"))
        for field in ("ProcessId", "Name", "CreationDate"):
            invalid = dict(server); invalid.pop(field)
            self.assertNotIn(field, invalid)


if __name__ == "__main__":
    unittest.main()
