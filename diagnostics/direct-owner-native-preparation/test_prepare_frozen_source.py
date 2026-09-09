#!/usr/bin/env python3
"""Regression tests for frozen direct-owner source preparation."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


TOOL_DIR = Path(__file__).resolve().parent
PREPARE = TOOL_DIR / "prepare_frozen_source.py"
EXPECTED_DERIVED = {
    "crates/host-play/src/lib.rs": "f997f22f00bb205cdad74eec70d9dfc5750c3f8f80d9ba52c024f5ea4ea9120f",
    "crates/host-play/src/memory.rs": "76e633acf9d4dcd1fd03f1a57ea722f5111025790071679119e7e5932ece2991",
}


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class FrozenPreparationTests(unittest.TestCase):
    def prepare(self, output: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(PREPARE), "prepare", "--output-dir", str(output)],
            cwd=TOOL_DIR.parents[1],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )

    def test_original_objects_plus_bound_patches_are_admitted_deterministically(self):
        with tempfile.TemporaryDirectory(prefix="direct-owner-prepare-test-") as temp_name:
            temp = Path(temp_name)
            first = temp / "first"
            second = temp / "second"
            for output in (first, second):
                result = self.prepare(output)
                self.assertEqual(result.returncode, 0, result.stdout)

                preparation = json.loads((output / "preparation-result.json").read_text())
                self.assertTrue(preparation["prepared"])
                self.assertTrue(preparation["archive_verified"])
                self.assertTrue(preparation["production_tree_unchanged"])

                audit = json.loads((output / "provenance-audit.json").read_text())
                self.assertTrue(audit["verified"])
                host = {row["path"]: row for row in audit["host"]}
                derived = {
                    path: row["materialized"]["sha256"]
                    for path, row in host.items()
                    if row["classification"] == "derived_test_only"
                }
                self.assertEqual(derived, EXPECTED_DERIVED)
                self.assertTrue(
                    all(row["production_prefix_equal"] for row in host.values() if row["classification"] == "derived_test_only")
                )
                self.assertTrue(
                    all(row["materialized_matches_reviewed"] for row in host.values() if row["classification"] == "exact_reviewed")
                )
                self.assertTrue(all(row["materialized_matches_reviewed"] for row in audit["client"]))

            first_archive = first / "frozen-direct-owner-source.tar.gz"
            second_archive = second / "frozen-direct-owner-source.tar.gz"
            self.assertEqual(first_archive.stat().st_size, second_archive.stat().st_size)
            self.assertEqual(file_sha256(first_archive), file_sha256(second_archive))


if __name__ == "__main__":
    unittest.main()
