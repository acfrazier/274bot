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
        self.assertEqual(prep.verify_source(archive, manifest)["host_commit"], prep.HOST_COMMIT)

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


if __name__ == "__main__":
    unittest.main()
