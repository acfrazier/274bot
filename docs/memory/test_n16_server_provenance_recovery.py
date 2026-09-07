from __future__ import annotations

import copy
import hashlib
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import n16_server_provenance_recovery as recovery  # noqa: E402


PID = 6728
FILETIME = 134332846721025908
CREATION_MS = 1788811072102


def process(pid=PID, name="node.exe", session=2, creation=CREATION_MS):
    return {
        "ProcessId": pid,
        "ParentProcessId": 10008,
        "Name": name,
        "SessionId": session,
        "CreationDate": f"/Date({creation})/",
        "CommandLine": "node.exe --import tsx src/app.ts",
    }


def identity(pid=PID, ticks=FILETIME):
    return {"pid": pid, "start_identity": f"windows_creation_filetime:{ticks}"}


class RecoveryTests(unittest.TestCase):
    def test_filetime_conversion_preserves_100ns_precision(self):
        self.assertEqual(recovery.creation_date_to_filetime(f"/Date({CREATION_MS})/"), FILETIME - 5_908)
        self.assertEqual(recovery.creation_date_to_filetime(f"/Date({CREATION_MS + 1})/"), FILETIME + 4_092)

    def test_unique_pid_name_session_and_creation_match(self):
        preflight = {"native_preflight": {"processes": [process(), process(pid=7)]}}
        conditions = {"native_preflight": {"adapter": "Intel"}}
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = [root / name for name in ("preflight.json", "server.json", "conditions.json")]
            for path in paths:
                path.write_text("{}")
            result = recovery.recover_conditions(
                preflight, identity(), conditions,
                preflight_path=paths[0], server_identity_path=paths[1], original_conditions_path=paths[2],
            )
        self.assertEqual(result["native_preflight"]["server"], process())
        self.assertTrue(result["provenance_recovery"]["not_original_artifact"])

    def test_absent_duplicate_wrong_processes_fail_closed(self):
        base = {"native_preflight": {"processes": [process(pid=7)]}}
        with self.assertRaisesRegex(recovery.RecoveryError, "0 matches"):
            recovery.recover_conditions(base, identity(), {"native_preflight": {}},
                                         preflight_path=pathlib.Path("p"),
                                         server_identity_path=pathlib.Path("s"),
                                         original_conditions_path=pathlib.Path("c"))
        duplicate = {"native_preflight": {"processes": [process(), process()]}}
        with self.assertRaisesRegex(recovery.RecoveryError, "2 matches"):
            recovery.recover_conditions(duplicate, identity(), {"native_preflight": {}},
                                         preflight_path=pathlib.Path("p"),
                                         server_identity_path=pathlib.Path("s"),
                                         original_conditions_path=pathlib.Path("c"))
        wrong_name = {"native_preflight": {"processes": [process(name="powershell.exe")]}}
        with self.assertRaisesRegex(recovery.RecoveryError, "0 matches"):
            recovery.recover_conditions(wrong_name, identity(), {"native_preflight": {}},
                                         preflight_path=pathlib.Path("p"),
                                         server_identity_path=pathlib.Path("s"),
                                         original_conditions_path=pathlib.Path("c"))

    def test_source_hashes_are_stable_and_existing_server_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = {name: root / f"{name}.json" for name in ("preflight", "server", "conditions")}
            paths["preflight"].write_text(json.dumps({"native_preflight": {"processes": [process()]}}))
            paths["server"].write_text(json.dumps(identity()))
            paths["conditions"].write_text(json.dumps({"native_preflight": {"adapter": "Intel"}}))
            before = {key: hashlib.sha256(path.read_bytes()).hexdigest() for key, path in paths.items()}
            result = recovery.recover_conditions(
                recovery.load_object(paths["preflight"]), recovery.load_object(paths["server"]),
                recovery.load_object(paths["conditions"]),
                preflight_path=paths["preflight"], server_identity_path=paths["server"],
                original_conditions_path=paths["conditions"],
            )
            after = {key: hashlib.sha256(path.read_bytes()).hexdigest() for key, path in paths.items()}
            self.assertEqual(before, after)
            self.assertEqual([x["sha256"] for x in result["provenance_recovery"]["source_artifacts"]], list(before.values()))
            existing = copy.deepcopy(result)
            with self.assertRaisesRegex(recovery.RecoveryError, "existing server"):
                recovery.recover_conditions(
                    recovery.load_object(paths["preflight"]), recovery.load_object(paths["server"]), existing,
                    preflight_path=paths["preflight"], server_identity_path=paths["server"],
                    original_conditions_path=paths["conditions"],
                )


if __name__ == "__main__":
    unittest.main()
