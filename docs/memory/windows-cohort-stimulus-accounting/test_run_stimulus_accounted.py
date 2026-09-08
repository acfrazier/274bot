import json
import tempfile
import unittest
from pathlib import Path

import run_stimulus_accounted as controller


class Clock:
    def __init__(self, value=75.0):
        self.value = value

    def monotonic(self):
        return self.value

    def sleep(self, seconds):
        self.value += seconds


class Process:
    pid = 4242
    returncode = 0

    def __init__(self, polls=1):
        self.polls = polls
        self.args = None

    def poll(self):
        if self.polls:
            self.polls -= 1
            return None
        return self.returncode

    def communicate(self):
        return ("helper stdout", "")


def manifest(root, **changes):
    value = {
        "schema": "native-panel-input-stimulus-accounting-manifest-v1",
        "cellId": "reference-focused-one-test",
        "sourceSha256": "source-hash",
        "cadenceMilliseconds": 1000,
        "pressMilliseconds": 80,
        "durationSeconds": 120,
        "triggerWindowSeconds": [60, 90],
        "noRetry": True,
        "sampleIntervalSeconds": 1,
        "observeStartPublication": {"monotonicSeconds": 0, "receiptSha256": "trigger"},
        "target": {
            "pid": 3131, "startUtc": "2026-09-08T13:00:00Z",
            "startIdentity": "target-start", "sessionId": 2,
            "binary": str(root / "panel-play.exe"), "binarySha256": "target-hash",
            "observedRect": [228, 228, 2326, 1154], "gameImagePoint": [628, 528],
            "scene2": True, "captureEnabled": True, "slotZeroFocus": True,
        },
        "helper": {"path": str(root / "invoke.ps1"), "sha256": controller.HELPER_SHA256},
        "output": {
            "directory": str(root / "output"), "label": "test",
            "receiptPath": str(root / "helper-receipt.json"),
            "eventsPath": str(root / "helper-events.json"),
            "postRunEnvelope": str(root / "post-run.json"),
        },
    }
    value.update(changes)
    return value


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "panel-play.exe").write_bytes(b"binary")
        (self.root / "invoke.ps1").write_bytes(b"helper")
        self.original_sha = controller._sha
        controller._sha = lambda path: controller.HELPER_SHA256 if path.name == "invoke.ps1" else "target-hash"

    def tearDown(self):
        controller._sha = self.original_sha
        self.temp.cleanup()

    def write_manifest(self, value):
        path = self.root / "manifest.json"
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def sampler(self, pid):
        return {"start_identity": "target-start" if pid == 3131 else "helper-start",
                "resident_bytes": 100, "user_s": 1, "system_s": 1}

    def test_wrong_identity_does_not_spawn(self):
        value = manifest(self.root)
        value["target"]["startIdentity"] = "wrong"
        called = []
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: called.append(a))
        self.assertEqual(result["outcome"], "incomplete")
        self.assertEqual(called, [])

    def test_missing_or_wrong_session_binding_fails_closed(self):
        value = manifest(self.root)
        del value["target"]["sessionId"]
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: self.fail("must not spawn"))
        self.assertIn("missing target binding", result["incompleteReason"])

    def test_wrong_helper_hash_does_not_spawn(self):
        value = manifest(self.root)
        value["helper"]["sha256"] = "0" * 64
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: self.fail("must not spawn"))
        self.assertIn("helper hash", result["incompleteReason"])

    def test_prelaunch_without_completed_receipt_is_incomplete(self):
        value = manifest(self.root)
        process = Process(polls=0)
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: process)
        self.assertEqual(result["spawnCount"], 1)
        self.assertEqual(result["outcome"], "incomplete")

    def test_receipt_mismatch_is_incomplete(self):
        value = manifest(self.root)
        process = Process(polls=0)
        (self.root / "helper-receipt.json").write_text(json.dumps({
            "schema": "native-panel-input-stimulus-v1", "label": "wrong",
        }), encoding="utf-8")
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: process)
        self.assertEqual(result["outcome"], "incomplete")
        self.assertEqual(result["incompleteReason"], "helper receipt binding mismatch")

    def test_missed_trigger_does_not_spawn(self):
        value = manifest(self.root)
        value["observeStartPublication"]["monotonicSeconds"] = 0
        result = controller.run(self.write_manifest(value), clock=Clock(91), sampler=self.sampler,
                                popen=lambda *a, **k: self.fail("must not spawn"))
        self.assertIn("window missed", result["incompleteReason"])

    def test_exactly_one_spawn_and_safe_argument_array(self):
        value = manifest(self.root)
        process = Process(polls=1)
        captured = []
        def launch(args, **kwargs):
            captured.append((args, kwargs))
            return process
        (self.root / "helper-receipt.json").write_text("{}", encoding="utf-8")
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler, popen=launch)
        self.assertEqual(result["spawnCount"], 1)
        self.assertEqual(len(captured), 1)
        self.assertIsInstance(captured[0][0], list)
        self.assertEqual(captured[0][0].count("-File"), 1)

    def test_sampler_failure_is_retained_not_zero(self):
        value = manifest(self.root)
        process = Process(polls=0)
        def failing(pid):
            raise RuntimeError("lost sampler coverage")
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=failing,
                                popen=lambda *a, **k: process)
        self.assertTrue(all(row["status"] == "unavailable" for row in result["samples"]))
        self.assertEqual(result["managedProcesses"], [])

    def test_helper_failure_and_cleanup_receipt_mismatch_are_incomplete(self):
        value = manifest(self.root)
        process = Process(polls=0)
        process.returncode = 7
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: process)
        self.assertEqual(result["outcome"], "incomplete")
        self.assertIn("helper failure", result["incompleteReason"])

    def test_receipt_and_events_are_archived(self):
        value = manifest(self.root)
        process = Process(polls=0)
        for name in ("helper-receipt.json", "helper-events.json"):
            (self.root / name).write_text(name, encoding="utf-8")
        result = controller.run(self.write_manifest(value), clock=Clock(), sampler=self.sampler,
                                popen=lambda *a, **k: process)
        self.assertIn(str(self.root / "helper-receipt.json"), result["helperOutputFiles"])
        self.assertTrue((self.root / "output" / "helper-events.json").is_file())


if __name__ == "__main__":
    unittest.main()
