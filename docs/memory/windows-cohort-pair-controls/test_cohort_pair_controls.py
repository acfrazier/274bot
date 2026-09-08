"""No-launch contract tests for the native cohort pair controls."""
import ast
import hashlib
import json
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).parent
HOST_MEMORY = HERE.parent
sys.path.insert(0, str(HOST_MEMORY))
import run_managed_cell as rmc


def controller_functions():
    tree = ast.parse((HERE / "run-cohort-pair.py").read_text())
    names = {"diagnostic_argv", "validate_build_receipt", "validate_stimulus_plan"}
    nodes = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in names]
    namespace = {"re": __import__("re"), "json": __import__("json"), "pathlib": pathlib}
    exec(compile(ast.Module(body=nodes, type_ignores=[]), "run-cohort-pair.py", "exec"), namespace)
    return namespace


class CohortPairControls(unittest.TestCase):
    def test_actual_parser_emits_n16_pair_contract(self):
        fn = controller_functions()["diagnostic_argv"]
        argv = fn("panel-play.exe", "manifest.json", "reference", "focused-one")
        args = rmc.parse_diagnostic_argv(argv)
        self.assertEqual((args.n, args.warmup, args.observe), (16, 120, 600))
        self.assertTrue(args.focused_one)
        self.assertFalse(args.focused_background)
        self.assertTrue(args.no_diagnostics)
        self.assertTrue(args.render_profile)
        self.assertTrue(args.gpu_completion_profile)
        self.assertTrue(args.failure_capture)
        self.assertNotIn("--cpu-fallback", argv)
        self.assertNotIn("--owner-census", argv)

    def test_receipt_requires_exact_role_source_and_real_hash(self):
        fn = controller_functions()["validate_build_receipt"]
        info = {"manifest_role": "reference", "host_commit": "e25f32806957b1a44c75a598cbdb7ab53afc5383"}
        good = {"schema": "native-cohort-build-receipt-v1", "role": "baseline", "manifest_role": "reference", "host_commit": info["host_commit"], "client_commit": "abb811bd0afa1acd99319ccd5bc36bfb241080f9", "binary": "C:\\ProgramData\\x\\panel-play.exe", "binary_sha256": "a" * 64}
        self.assertIs(fn(good, role_name="baseline", role_info=info, binary_path=pathlib.Path(good["binary"]), expected_client=good["client_commit"]), good)
        for field, value in (("schema", ""), ("role", "candidate"), ("host_commit", "f" * 40), ("binary_sha256", "pending"), ("binary", "C:\\other\\panel-play.exe")):
            bad = dict(good)
            bad[field] = value
            with self.assertRaises(AssertionError, msg=field):
                fn(bad, role_name="baseline", role_info=info, binary_path=pathlib.Path(good["binary"]), expected_client=good["client_commit"])

    def test_stimulus_plan_is_prelaunch_only_and_fixed_protocol(self):
        fn = controller_functions()["validate_stimulus_plan"]
        good = {"schema": "native-panel-input-stimulus-plan-v1", "helperSha256": "04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1", "cadenceMilliseconds": 1000, "pressMilliseconds": 80, "durationSeconds": 120, "triggerWindowAfterObserveStartSeconds": [60, 90], "noRetry": True, "receiptPath": "future-receipt.json"}
        path = HERE / "test-stimulus-plan.json"
        path.write_text(json.dumps(good))
        try:
            self.assertEqual(fn(path), good)
            for key, value in (("durationSeconds", 600), ("helperSha256", "pending"), ("pid", 7)):
                bad = dict(good)
                bad[key] = value
                path.write_text(json.dumps(bad))
                with self.assertRaises(AssertionError, msg=key):
                    fn(path)
        finally:
            path.unlink(missing_ok=True)

    def test_helper_source_is_immutable_copy(self):
        original = HERE.parent / "windows-visual-proof-tools" / "invoke-panel-input-stimulus.ps1"
        helper = HERE / "invoke-panel-input-stimulus.ps1"
        self.assertTrue(original.is_file())
        self.assertEqual(helper.read_bytes(), original.read_bytes())
        self.assertEqual(hashlib.sha256(helper.read_bytes()).hexdigest(), "04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1")

    def test_duration_and_tail_are_explicit_in_source(self):
        source = (HERE / "run-cohort-pair.py").read_text()
        self.assertIn('"warmup_s": 120', source)
        self.assertIn('"observe_s": 600', source)
        self.assertIn('"cohort_tail_s": 5', source)
        self.assertIn('"teardown_grace_s": 60', source)
        self.assertIn('"stimulus_receipt"', source)


if __name__ == "__main__":
    unittest.main()
