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
    names = {"sha", "publication_boundaries", "diagnostic_argv", "validate_build_receipt", "validate_stimulus_plan", "validate_stimulus_receipt", "validate_prepare_receipt"}
    nodes = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in names]
    namespace = {"time": __import__("time"), "hashlib": hashlib,"math": __import__("math"), "re": __import__("re"), "json": __import__("json"), "pathlib": pathlib}
    exec(compile(ast.Module(body=nodes, type_ignores=[]), "run-cohort-pair.py", "exec"), namespace)
    return namespace


class CohortPairControls(unittest.TestCase):
    def test_publication_uses_first_validated_managed_boundary(self):
        import tempfile
        factory = controller_functions()["publication_boundaries"]
        with tempfile.TemporaryDirectory() as temp:
            output = pathlib.Path(temp) / "publication.json"
            cls = factory(rmc.QualificationBoundaries, output, cell_id="baseline-focused-one-x", controller_identity={"pid": 8, "start_identity": "test"})
            tracker = cls()
            line = json.dumps({"phase": "observe-start", "elapsed_s": 120, "slots": [{"slot": 0}]})
            tracker.consume(line, 451.125)
            record = json.loads(output.read_text())
            self.assertEqual(record["monotonicSeconds"], 451.125)
            self.assertEqual(record["rawLineSha256"], hashlib.sha256(line.encode()).hexdigest())
            tracker.consume(line, 452)
            self.assertEqual(json.loads(output.read_text()), record)
            self.assertIn("duplicate or reversed start", tracker.errors)
            other = pathlib.Path(temp) / "invalid.json"
            bad = factory(rmc.QualificationBoundaries, other, cell_id="test", controller_identity={})()
            bad.consume("invalid JSON", 1)
            bad.consume(line, 2)
            self.assertFalse(other.exists())

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

    def test_stimulus_receipt_is_postrun_and_resource_bound(self):
        import tempfile
        fn = controller_functions()["validate_stimulus_receipt"]
        plan = {"helperSha256": "04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1"}
        identity = lambda n: "windows_creation_filetime:" + str(n)
        with tempfile.TemporaryDirectory() as temp:
            helper_path = pathlib.Path(temp) / "helper-receipt.json"
            helper = {"pid": 8, "startUtc": "2026-01-01T00:00:00Z", "outcome": "completed", "completedPulses": 120, "requestedPulses": 120,
                      "events": [{"index": i + 1, "direction": "Left" if i % 2 == 0 else "Right", "downSendInputResult": 1, "upSendInputResult": 1, "releaseSucceeded": True} for i in range(120)]}
            helper_path.write_text(json.dumps(helper))
            samples = [{"pid": pid, "process": role, "status": "available", "sample": {"start_identity": identity(pid), "user_s": 0.5, "system_s": 0.5, "resident_bytes": 1024}} for pid, role in ((7, "input-helper"), (9, "wrapper-sampler"))]
            summary = lambda n: {"pid": samples[n]["pid"], "process": samples[n]["process"], "status": "available", "start_identity": samples[n]["sample"]["start_identity"], "cpu_seconds": 1.0, "rss_bytes": 1024, "sampleIndexes": [n]}
            good = {"schema": "native-panel-input-stimulus-run-receipt-v1", "cellId": "baseline-focused-one-x", "outcome": "completed", "helperSha256": plan["helperSha256"], "cadenceMilliseconds": 1000, "pressMilliseconds": 80, "durationSeconds": 120, "startedUnix": 101, "captureEnabledVerified": True, "slotZeroFocusVerified": True, "helperPid": 7, "helperStartUtc": None, "helperStartIdentity": identity(7), "targetPid": 8, "targetStartIdentity": identity(8), "targetStartUtc": helper["startUtc"], "triggerDelaySeconds": 60, "resourceAccounting": {"status": "available", "helper": summary(0), "wrapper": summary(1), "targetIdentity": identity(8), "targetExcludedFromManagedTotals": True}, "sampler": {"label": "root-managed windows_process_sample", "backend": "windows_process_sample.sample_process"}, "samples": samples, "helperOutputFiles": {str(helper_path): hashlib.sha256(helper_path.read_bytes()).hexdigest()}, "performanceAcceptance": False, "inputCoveragePass": False}
            kwargs = {"plan": plan, "cell_id": good["cellId"], "run_started_unix": 100, "target_pid": 8, "target_start_identity": identity(8), "publication": {"schema": "cohort-observe-start-publication-v1", "cellId": good["cellId"]}}
            path = pathlib.Path(temp) / "envelope.json"
            path.write_text(json.dumps(good))
            self.assertEqual(fn(path, **kwargs), good)
            for delay in (60, 60.125, 89.999, 90):
                candidate = dict(good, triggerDelaySeconds=delay)
                path.write_text(json.dumps(candidate))
                self.assertEqual(fn(path, **kwargs)["triggerDelaySeconds"], delay)
            invalid = [{"triggerDelaySeconds": delay} for delay in (59.999, 90.001, True, False, None, "60", float("nan"), float("inf"), -float("inf"))]
            invalid += [{"cellId": "candidate-focused-one-x"}, {"startedUnix": 99}, {"startedUnix": float("nan")}, {"helperSha256": "pending"}, {"cadenceMilliseconds": 500}, {"resourceAccounting": {"status": "available"}}, {"targetPid": 99}, {"helperStartIdentity": "fake"}]
            for bad in invalid:
                candidate = dict(good); candidate.update(bad); path.write_text(json.dumps(candidate))
                with self.assertRaises(AssertionError):
                    fn(path, **kwargs)
            for field, value in (("user_s", None), ("system_s", float("nan")), ("resident_bytes", 0), ("start_identity", identity(99))):
                candidate = json.loads(json.dumps(good))
                candidate["samples"][0]["sample"][field] = value
                path.write_text(json.dumps(candidate))
                with self.assertRaises(AssertionError):
                    fn(path, **kwargs)
            path.write_text(json.dumps(good))
            helper_path.write_text("changed artifact")
            with self.assertRaisesRegex(AssertionError, "hash mismatch"):
                fn(path, **kwargs)

    def test_duration_and_tail_are_explicit_in_source(self):
        source = (HERE / "run-cohort-pair.py").read_text()
        self.assertIn('"warmup_s": 120', source)
        self.assertIn('"observe_s": 600', source)
        self.assertIn('"cohort_tail_s": 5', source)
        self.assertIn('"teardown_grace_s": 60', source)
        self.assertIn('"stimulus_receipt"', source)

    def test_no_launch_then_prepare_then_launch_order_is_explicit(self):
        validate_prepare = controller_functions()["validate_prepare_receipt"]
        cell_id = "baseline-focused-one-x"
        contract = {"kind": "no-launch contract test", "cell_id": cell_id, "performance_acceptance": False}
        self.assertFalse((HERE / "test-prepare-missing.json").exists())
        prepare = {"schema": "cohort-pair-prepare-no-launch-v1", "cell_id": cell_id, "client_started": False, "scheduled_task_created": False}
        self.assertIs(validate_prepare(prepare, cell_id=contract["cell_id"]), prepare)
        for bad in ({"cell_id": "candidate-focused-one-x"}, {"client_started": True}, {"scheduled_task_created": True}):
            candidate = dict(prepare); candidate.update(bad)
            with self.assertRaises(AssertionError):
                validate_prepare(candidate, cell_id=cell_id)
        launch_source = (HERE / "launch-cohort.ps1").read_text()
        self.assertIn("Exact no-launch prepare receipt is required before scheduling", launch_source)
        self.assertIn("COHORT_NO_LAUNCH_VALIDATION", (HERE / "run-contractcheck.ps1").read_text())

    def test_scheduled_launcher_serializes_frozen_child_bindings(self):
        source = (HERE / "launch-cohort.ps1").read_text()
        for binding in ("COHORT_HOST_ROOT", "COHORT_REFERENCE_STAGE", "COHORT_CANDIDATE_STAGE", "COHORT_PREFLIGHT_DIR", "COHORT_BUILD_MANIFEST", "COHORT_BUILD_RECEIPT", "COHORT_STIMULUS_PLAN", "COHORT_STIMULUS_RECEIPT", "COHORT_PREPARE_RECEIPT"):
            self.assertIn("`$env:" + binding, source)
        self.assertIn("cohort-reference-e25f328", source)
        self.assertIn("cohort-candidate-ca56e143", source)
        self.assertIn("Exact no-launch prepare receipt is required before scheduling", source)


if __name__ == "__main__":
    unittest.main()
