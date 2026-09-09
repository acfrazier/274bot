import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import prepare_invocation as prep


class PreparationTests(unittest.TestCase):
    def test_real_frozen_source_identity(self):
        root = Path(__file__).resolve().parents[1]
        archive = root / "direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz"
        manifest = root / "direct-owner-native-preparation/artifact/frozen/frozen-source-manifest.json"
        source = prep.verify_source(archive, manifest)
        self.assertEqual(source["host_original"], prep.HOST_COMMIT)
        self.assertEqual(source["host_reviewed"], prep.HOST_REVIEWED_COMMIT)

    def test_placeholder_is_rejected(self):
        with self.assertRaises(ValueError):
            prep.regular("/ROOT/not-real", "binary")

    def test_native_qualification_must_be_true(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "qualification.json"
            path.write_text(json.dumps({
                "schema": "direct-owner-native-generated-qualification-v1",
                "linux_generated_qualified": False,
                "live_qualified": False,
                "frontend_launched": False,
                "source_verified": True,
                "locks_unchanged": True,
            }))
            with self.assertRaisesRegex(ValueError, "missing or false"):
                prep.verify_qualification(path, {})

    def test_build_uses_schema_faithful_candidate_and_feature_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "tui-play"
            binary.write_bytes(b"offline fixture")
            manifest = root / "build-manifest.json"
            manifest.write_text(json.dumps({
                "candidate": {
                    "commit": prep.HOST_COMMIT,
                    "build_exit": 0,
                    "sources_sha256_pre": "a" * 64,
                    "sources_sha256_post": "a" * 64,
                    "sources_stable_across_build": True,
                    "client": {"commit": prep.CLIENT_COMMIT},
                },
                "features": {
                    "requested": ["memory-profile-no-alloc", "memory-owner-capture"],
                    "locked": True, "allocation_counting": False,
                    "snapshot_dedup": False,
                },
                "binaries": {"candidate_tui_play": {
                    "path": str(binary), "sha256": prep.digest(binary),
                }},
            }))
            manifest_data = json.loads(manifest.read_text())
            manifest_data["candidate"]["branch"] = "direct-owner-derivative"
            manifest_data["candidate"]["client"]["sources_sha256"] = "b" * 64
            manifest_data["features"]["requested"] = "memory-profile-no-alloc,memory-owner-capture"
            manifest_data["features"]["allocator"] = "std::alloc::System"
            manifest.write_text(json.dumps(manifest_data))
            derivative = {
                "checkout_host_commit": prep.HOST_COMMIT,
                "checkout_client_commit": prep.CLIENT_COMMIT,
            }
            result = prep.verify_build(manifest, binary, derivative)
            self.assertEqual(result["checkout_client_commit"], prep.CLIENT_COMMIT)

    def test_controller_requires_exact_reviewed_tool_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            controller = root / "run_current_tui_calibration.py"
            controller.write_bytes(b"not the reviewed controller")
            manifest = root / "install-manifest.json"
            manifest.write_text(json.dumps({
                "review_commit": prep.CONTROLLER_COMMIT,
                "only_untracked_controller_tools_changed": {
                    "run_current_tui_calibration.py": {"after": prep.CONTROLLER_SHA256}
                },
            }))
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                prep.verify_controller(controller, manifest)


if __name__ == "__main__":
    unittest.main()
