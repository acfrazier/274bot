from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

import reference_metrics as rm

ROOT = pathlib.Path(__file__).resolve().parent


def envelope(path: pathlib.Path):
    b = {"start_mono_ns": 100, "end_mono_ns": 200, "tail_ns": 50}
    return [
        {"record": "cohort-header", "schema_version": 1, "tail_name": "DEFAULT_TAIL_NS", "tail_ns": 50,
         "observe_ns": 100, "capacity": 512, "frontend": "panel",
         "input_population": {"kind": "focused-one", "slots": [0]},
         "decode_population": {"kind": "all-run-slots", "n": 1}, "boundaries": b,
         "clock_domain": "responsiveness_process_mono", "sidecar_path": str(path)},
        {"record": "cohort-batch", "schema_version": 1, "boundaries": b,
         "records": [
             {"id": {"slot_id": 1, "generation": 7, "sequence": 99, "start_mono_ns": 150, "surface": "Decode"},
              "complete_mono_ns": 250, "outcome": "Completed"},
             {"id": {"slot_id": 0, "generation": 3, "sequence": 1, "start_mono_ns": 199, "surface": "Panel"},
              "complete_mono_ns": 200, "outcome": "Completed"}],
         "losses": [], "next_cursor": 8, "complete": False, "loss_count": 0, "journal_overflow_n": 0},
        {"record": "cohort-terminal", "schema_version": 1, "boundaries": b, "terminal": True,
         "available": True, "records_n": 2, "losses_n": 0, "pending_n": 0, "frontend": "panel"},
    ]


class CohortReaderTests(unittest.TestCase):
    def test_full_archived_run_analyze_and_require(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td)
            meta = {"frontend": "panel", "n": 1, "workload": "active", "responsiveness_profile": True,
                    "scheduling_profile": False, "render_profile": False, "gpu_completion_profile": False,
                    "started_unix": 1.0}
            (run / "metadata.json").write_text(json.dumps(meta))
            (run / "samples.jsonl").write_text(json.dumps({"phase": "observe", "elapsed_s": 1}) + "\n")
            sidecar = run / "samples.cohort.jsonl"
            sidecar.write_text("\n".join(json.dumps(x) for x in envelope(sidecar)) + "\n")
            result = rm.analyze_run(run)
            self.assertEqual(result["gates"]["decode_cohort"]["target_verdict"], "meet")
            self.assertEqual(result["gates"]["input_cohort"]["target_verdict"], "meet")
            proc = subprocess.run([sys.executable, str(ROOT / "reference_metrics.py"), str(run),
                                   "--require", "decode_cohort,input_cohort"], text=True,
                                  capture_output=True, check=False)
            self.assertEqual(proc.returncode, 0, proc.stderr)

    def test_old_archive_is_legacy_visible_and_cohort_unavailable(self):
        with tempfile.TemporaryDirectory() as td:
            run = pathlib.Path(td)
            (run / "metadata.json").write_text(json.dumps({"frontend": "tui", "n": 1}))
            (run / "samples.jsonl").write_text(json.dumps({"phase": "observe", "elapsed_s": 1}) + "\n")
            result = rm.analyze_run(run)
            self.assertEqual(result["gates"]["decode_cohort"]["status"], "unavailable")
            self.assertEqual(result["gates"]["decode_cohort"]["reason"], "cohort_schema_missing")
            self.assertIn("decode", result["gates"])

    def test_ns_boundary_and_tail_rules(self):
        self.assertEqual(rm.cohort_reader.p99_ns([1_100_000], rm.cohort_reader.FINE_BOUNDS_MS)["upper_ms"], 2)
        self.assertEqual(rm.cohort_reader.p99_ns([100_000_000])["upper_ms"], 100)
        self.assertEqual(rm.cohort_reader.p99_ns([100_000_001], rm.cohort_reader.FINE_BOUNDS_MS)["reason"], "overflow_bucket")


if __name__ == "__main__":
    unittest.main()
