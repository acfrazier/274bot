import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import prepare_invocation as prep


class PreparationTests(unittest.TestCase):
    DERIVATIVE_HOST = "1111111111111111111111111111111111111111"
    DERIVATIVE_CLIENT = "2222222222222222222222222222222222222222"

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
                    "commit": self.DERIVATIVE_HOST,
                    "build_exit": 0,
                    "sources_sha256_pre": "a" * 64,
                    "sources_sha256_post": "a" * 64,
                    "sources_stable_across_build": True,
                    "branch": "direct-owner-derivative",
                    "client": {
                        "commit": self.DERIVATIVE_CLIENT,
                        "sources_sha256": "b" * 64,
                    },
                },
                "features": {
                    "requested": "memory-profile-no-alloc,memory-owner-capture",
                    "locked": True, "allocation_counting": False,
                    "snapshot_dedup": False,
                    "allocator": "std::alloc::System",
                },
                "binaries": {"candidate_tui_play": {
                    "path": str(binary), "sha256": prep.digest(binary),
                }},
            }))
            derivative = {
                "checkout_host_commit": self.DERIVATIVE_HOST,
                "checkout_client_commit": self.DERIVATIVE_CLIENT,
            }
            result = prep.verify_build(manifest, binary, derivative)
            self.assertEqual(result["checkout_client_commit"], self.DERIVATIVE_CLIENT)

    def test_derivative_rejects_provenance_relabel(self):
        source = {
            "archive_sha256": prep.ARCHIVE_SHA256,
            "host_original": prep.HOST_COMMIT,
            "client_original": prep.CLIENT_COMMIT,
            "host_reviewed": prep.HOST_REVIEWED_COMMIT,
            "client_reviewed": prep.CLIENT_REVIEWED_COMMIT,
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "derivative-admission.json"
            for identity in (
                prep.HOST_COMMIT,
                prep.CLIENT_COMMIT,
                prep.HOST_REVIEWED_COMMIT,
                prep.CLIENT_REVIEWED_COMMIT,
            ):
                path.write_text(json.dumps({
                    "schema": "direct-owner-derivative-admission-v1",
                    "archive_sha256": prep.ARCHIVE_SHA256,
                    "source_provenance": source,
                    "checkout_host_commit": identity,
                    "checkout_client_commit": self.DERIVATIVE_CLIENT,
                    "host_clean": True,
                    "client_clean": True,
                    "materialized_from_archive": True,
                }))
                with self.assertRaisesRegex(ValueError, "must not relabel provenance"):
                    prep.verify_derivative_admission(path, source)

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
