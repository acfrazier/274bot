#!/usr/bin/env python3
"""CLI + unit tests for qualify_control (stdlib only)."""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
QUALIFY = ROOT / "qualify_control.py"
REAL_PASS = ROOT / "diagnostics" / "20260906T181830Z_panel_n32_active"
REAL_FAIL = ROOT / "diagnostics" / "20260906T182432Z_panel_n32_active"


def _write_jsonl(path: pathlib.Path, rows):
    path.write_text("".join(json.dumps(r) + "\n" for r in rows))


def _active_fixture(run: pathlib.Path, *, n=1, observe_s=180, exit_code=0, workload="active"):
    (run / "metadata.json").write_text(
        json.dumps({"exit_code": exit_code, "n": n, "observe_s": observe_s, "workload": workload})
    )
    rows = [
        dict(
            phase="observe",
            elapsed_s=t,
            ready=n,
            active=n if workload != "seeded-idle" else 0,
            allocation_counting=False,
            diagnostic_sidecar=False,
            rust_allocations=None,
            rust_allocated_bytes=None,
            rust_live_bytes=None,
            process_cpu_user_s=t / 2,
            process_cpu_system_s=t / 4,
            client_tick_count=t * 50,
            resident_bytes=100,
            v8_live_isolates=0 if workload == "seeded-idle" else 1,
            snapshot_inflight_bytes=0,
            snapshot_inflight_capacity=0,
        )
        for t in [0, observe_s // 2, observe_s]
    ]
    if workload == "seeded-idle":
        proof = [
            dict(
                phase=phase,
                slots=[
                    dict(
                        name="bot",
                        state="Idle",
                        error=None,
                        runtime=None,
                        client=dict(ingame=True, scene_state=2, x=2661, z=3306, level=0),
                    )
                ],
            )
            for phase in ("observe-start", "observe-end")
        ]
    else:
        proof = [
            dict(
                phase=phase,
                slots=[
                    dict(
                        name="bot",
                        state="Running",
                        error=None,
                        runtime={"paint": {"lines": [f"Steals: {count}"]}},
                    )
                ],
            )
            for phase, count in (("observe-start", 2), ("observe-end", 7))
        ]
    _write_jsonl(run / "samples.jsonl", rows)
    _write_jsonl(run / "samples.qualification.jsonl", proof)
    return rows, proof


def _cli(run_dir: pathlib.Path, *extra, output: pathlib.Path | None = None):
    cmd = [sys.executable, str(QUALIFY), str(run_dir), *extra]
    if output is not None:
        cmd.extend(["--output", str(output)])
    else:
        cmd.append("--no-write")
    proc = subprocess.run(cmd, capture_output=True, text=True, cwd=str(ROOT.parent.parent))
    body = proc.stdout.strip()
    data = json.loads(body) if body else None
    return proc.returncode, data, proc.stderr


class QualifyCliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.run_dir = pathlib.Path(self.temp.name)

    def test_valid_active_exits_0(self):
        _active_fixture(self.run_dir, workload="active")
        code, data, err = _cli(self.run_dir)
        self.assertEqual(err, "")
        self.assertEqual(code, 0)
        self.assertTrue(data["qualified"])
        self.assertEqual(data["errors"], [])
        self.assertIn("qualification", data)

    def test_valid_seeded_idle_exits_0(self):
        _active_fixture(self.run_dir, workload="seeded-idle")
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 0)
        self.assertTrue(data["qualified"])
        self.assertEqual(data["workload"], "seeded-idle")

    def test_zero_progress_exits_1(self):
        _rows, proof = _active_fixture(self.run_dir)
        proof[1]["slots"][0]["runtime"]["paint"]["lines"] = ["Steals: 2"]
        _write_jsonl(self.run_dir / "samples.qualification.jsonl", proof)
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("progress" in e for e in data["errors"]))

    def test_below_scale_exits_1(self):
        rows, _ = _active_fixture(self.run_dir)
        rows[1]["ready"] = 0
        _write_jsonl(self.run_dir / "samples.jsonl", rows)
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("scale" in e for e in data["errors"]))

    def test_wrong_instrumentation_exits_1(self):
        rows, _ = _active_fixture(self.run_dir)
        rows[1]["allocation_counting"] = True
        _write_jsonl(self.run_dir / "samples.jsonl", rows)
        code, data, _ = _cli(self.run_dir)  # default counting=false
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("instrumentation" in e or "allocator" in e for e in data["errors"]))

    def test_missing_boundary_exits_1(self):
        _, proof = _active_fixture(self.run_dir)
        proof.pop()
        _write_jsonl(self.run_dir / "samples.qualification.jsonl", proof)
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertTrue(any("boundary" in e for e in data["errors"]))

    def test_incomplete_process_exits_1(self):
        _active_fixture(self.run_dir, exit_code=1)
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("process" in e or "incomplete" in e for e in data["errors"]))

    def test_missing_file_exits_1(self):
        _active_fixture(self.run_dir)
        (self.run_dir / "samples.jsonl").unlink()
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("missing required file" in e for e in data["errors"]))

    def test_malformed_json_exits_1(self):
        _active_fixture(self.run_dir)
        (self.run_dir / "metadata.json").write_text("{not-json")
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("malformed JSON" in e for e in data["errors"]))

    def test_malformed_samples_jsonl_exits_1(self):
        _active_fixture(self.run_dir)
        (self.run_dir / "samples.jsonl").write_text("{ok: false\n")
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertTrue(any("malformed JSON" in e and "samples.jsonl" in e for e in data["errors"]))

    def test_unsupported_workload_exits_1(self):
        _active_fixture(self.run_dir, workload="idle")
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertFalse(data["qualified"])
        self.assertTrue(any("unsupported workload" in e for e in data["errors"]))

    def test_lifecycle_unsupported(self):
        _active_fixture(self.run_dir, workload="lifecycle")
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 1)
        self.assertTrue(any("unsupported workload" in e for e in data["errors"]))

    def test_writes_output_path(self):
        _active_fixture(self.run_dir)
        out = pathlib.Path(self.temp.name) / "out" / "control-qualification.json"
        code, data, _ = _cli(self.run_dir, output=out)
        self.assertEqual(code, 0)
        self.assertTrue(out.is_file())
        on_disk = json.loads(out.read_text())
        self.assertEqual(on_disk["qualified"], data["qualified"])

    def test_default_modes_are_false(self):
        _active_fixture(self.run_dir)
        code, data, _ = _cli(self.run_dir)
        self.assertEqual(code, 0)
        self.assertFalse(data["counting"])
        self.assertFalse(data["diagnostics"])


@unittest.skipUnless(REAL_PASS.is_dir() and REAL_FAIL.is_dir(), "real diagnostic runs absent")
class RealDataCliTests(unittest.TestCase):
    def test_known_pass_run(self):
        out = pathlib.Path(tempfile.mkdtemp()) / "control-qualification.json"
        code, data, _ = _cli(REAL_PASS, output=out)
        self.assertEqual(code, 0, msg=data)
        self.assertTrue(data["qualified"])
        self.assertEqual(data["errors"], [])
        # Do not leave artifact in the real run dir
        self.assertFalse((REAL_PASS / "control-qualification.json").exists())
        self.assertTrue(out.is_file())

    def test_known_fail_run(self):
        out = pathlib.Path(tempfile.mkdtemp()) / "control-qualification.json"
        code, data, _ = _cli(REAL_FAIL, output=out)
        self.assertEqual(code, 1, msg=data)
        self.assertFalse(data["qualified"])
        errors = data["errors"]
        self.assertTrue(any("progress" in e and "live14f0f_11" in e for e in errors), errors)
        self.assertTrue(any("scale" in e for e in errors), errors)
        self.assertFalse((REAL_FAIL / "control-qualification.json").exists())


if __name__ == "__main__":
    unittest.main()
