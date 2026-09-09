import importlib.util
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location("supervise", HERE / "supervise.py")
supervise = importlib.util.module_from_spec(spec); spec.loader.exec_module(supervise)

class ToolingTests(unittest.TestCase):
    def test_selector_presets_and_rejection(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "routes.tsv"
            p.write_text("100 -20 0 101 -20 0 0 4 0 0\n100 -20 0 101 -20 0 0 5 0 0\n100 -20 0 101 -20 0 0 6 0 0\n")
            self.assertEqual([r[7] for r in supervise.selectors(p)], [4, 5, 6])
            p.write_text("100 -20 0\n")
            with self.assertRaises(ValueError): supervise.selectors(p)
    def test_pair_noise_is_explicit(self):
        result = supervise.paired_delta([10, 11, 10], [11, 12, 11])
        self.assertEqual(result["n"], 3)
        self.assertEqual(result["median_delta"], 1)
    def test_eof_and_nonzero_receipts(self):
        with tempfile.TemporaryDirectory() as d:
            r = supervise.bounded(["/usr/bin/python3", "-c", "import os; os.close(1); os.close(2)"], Path(d), "eof")
            self.assertIsNone(r["failure"])
            r = supervise.bounded(["/usr/bin/python3", "-c", "raise SystemExit(23)"], Path(d), "exit")
            self.assertEqual(r["failure"], "exit")

if __name__ == "__main__": unittest.main()
