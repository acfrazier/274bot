#!/usr/bin/env python3
import hashlib
import json
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
FIXTURE = ROOT / "target/debug/nav-fixture"
CENSUS = ROOT / "target/debug/nav-uniform-census"
SUPERVISOR = ROOT / "supervisor.py"


class CensusFixtures(unittest.TestCase):
    def run_fixture(self, mode, directory):
        path = directory / f"{mode}.navpack"
        subprocess.run([str(FIXTURE), mode, str(path)], check=True)
        result = subprocess.run([str(CENSUS), str(path)], text=True,
                                capture_output=True)
        return path, result

    def test_uniform_pairs_and_boundary(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            path, result = self.run_fixture("all-pairs", directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(result.stdout)
            self.assertEqual(len(report["uniform_pair_histogram"]), 512)
            self.assertEqual(sum(report["uniform_pair_histogram"]), 2048)
            self.assertEqual(report["actual"]["flags_present"], False)
            boundary, result = self.run_fixture("boundary", directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["actual"]["width"], 32)
            self.assertNotEqual(path.read_bytes(), boundary.read_bytes())

    def test_nonuniform_and_signed_accounting(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            path, result = self.run_fixture("nonuniform", directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(result.stdout)
            accounting = report["accounting"]
            self.assertGreater(accounting["nonuniform_tiles"], 0)
            self.assertEqual(
                accounting["hypothetical_element_storage_estimate"],
                8 * accounting["tile_count"] + 1152 * accounting["nonuniform_tiles"],
            )
            self.assertLess(accounting["potential_element_reduction_estimate"], 0)

    def test_decoder_rejections_and_hash_binding(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            for mode in ("invalid-dimensions", "overflow"):
                _, result = self.run_fixture(mode, directory)
                self.assertNotEqual(result.returncode, 0)
            path, _ = self.run_fixture("uniform", directory)
            rejected = subprocess.run(
                ["python3", str(SUPERVISOR), "--fixture", "--executable", str(CENSUS),
                 "--input", str(path), "--input-sha256", "0" * 64,
                 "--output-root", str(directory / "rejected")], capture_output=True)
            self.assertNotEqual(rejected.returncode, 0)

    def test_owned_fixture_receipt_and_portable_guard_label(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            path, result = self.run_fixture("planes", directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            output_root = directory / "receipts"
            run = subprocess.run(["python3", str(SUPERVISOR), "--fixture",
                                  "--executable", str(CENSUS), "--input", str(path),
                                  "--source", str(ROOT / "build.rs"),
                                  "--output-root", str(output_root)],
                                 text=True, capture_output=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            receipts = list(output_root.glob("nav-census-*/receipt.json"))
            self.assertEqual(len(receipts), 1)
            receipt = json.loads(receipts[0].read_text())
            self.assertFalse(receipt["linux_proc_guard"])
            self.assertTrue(receipt["sampled_rss_is_not_cumulative_peak"])
            self.assertIsNone(receipt["sampled_peak_rss_bytes"])
            self.assertEqual(receipt["sampled_memory_status"], "unavailable")

    def test_symlink_and_identity_mismatches_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            path, _ = self.run_fixture("uniform", directory)
            link = directory / "link.navpack"
            link.symlink_to(path)
            direct = subprocess.run([str(CENSUS), str(link)], capture_output=True)
            self.assertNotEqual(direct.returncode, 0)
            exe_hash = hashlib.sha256(CENSUS.read_bytes()).hexdigest()
            source_hash = hashlib.sha256((ROOT / "build.rs").read_bytes()).hexdigest()
            rejected = subprocess.run(
                ["python3", str(SUPERVISOR), "--executable", str(CENSUS),
                 "--input", str(path), "--source", str(ROOT / "build.rs"),
                 "--input-sha256", hashlib.sha256(path.read_bytes()).hexdigest(),
                 "--executable-sha256", "0" * 64, "--source-sha256", source_hash,
                 "--output-root", str(directory / "bad-exe")], capture_output=True)
            self.assertNotEqual(rejected.returncode, 0)
            rejected = subprocess.run(
                ["python3", str(SUPERVISOR), "--executable", str(CENSUS),
                 "--input", str(path), "--source", str(ROOT / "build.rs"),
                 "--input-sha256", hashlib.sha256(path.read_bytes()).hexdigest(),
                 "--executable-sha256", exe_hash, "--source-sha256", "0" * 64,
                 "--output-root", str(directory / "bad-source")], capture_output=True)
            self.assertNotEqual(rejected.returncode, 0)

    def test_owned_failures_keep_receipts(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            input_path, _ = self.run_fixture("uniform", directory)
            cases = {
                "nonzero": "#!/bin/sh\nexit 7\n",
                "output": "#!/usr/bin/env python3\nimport sys\nsys.stdout.write('x' * 2000000)\n",
                "split-output": "#!/usr/bin/env python3\nimport sys\nsys.stdout.write('x' * 600000)\nsys.stderr.write('y' * 600000)\n",
                "timeout": "#!/usr/bin/env python3\nimport time\ntime.sleep(61)\n",
            }
            for label, source in cases.items():
                program = directory / label
                program.write_text(source)
                program.chmod(0o700)
                output = directory / f"{label}-receipts"
                result = subprocess.run(
                    ["python3", str(SUPERVISOR), "--fixture", "--executable", str(program),
                     "--input", str(input_path), "--output-root", str(output)],
                    capture_output=True, text=True, timeout=70)
                self.assertNotEqual(result.returncode, 0, label)
                self.assertTrue(list(output.glob("nav-census-*/receipt.json")) or
                                list(output.glob("nav-census-failed-*/receipt.json")), label)


if __name__ == "__main__":
    unittest.main()
