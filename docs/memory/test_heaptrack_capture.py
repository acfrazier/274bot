import os
import pathlib
import tempfile
import unittest
import sys
from unittest import mock

import heaptrack_capture as hc


class HeaptrackCaptureTests(unittest.TestCase):
    def test_plan_bounds_are_literal_bytes(self):
        self.assertEqual(hc.MAX_ANALYSIS_AS_BYTES, 805306368)
        self.assertEqual(hc.MAX_ANALYSIS_RSS_BYTES, 536870912)
        self.assertEqual(hc.MAX_RAW_BYTES, 3221225472)

    def test_bounded_child_timeout_is_reaped_and_stderr_is_separate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            stdout = root / "stdout"
            stderr = root / "stderr"
            with self.assertRaises(hc.CaptureError):
                hc.run_bounded(
                    [sys.executable, "-c", "import sys,time; sys.stderr.write('x'); sys.stderr.flush(); time.sleep(10)"],
                    stdin_path=None, stdout_path=stdout, stderr_path=stderr, timeout_s=0.1,
                    as_bytes=2 * 1024 ** 3, sample=lambda _pid: 0,
                )
            self.assertEqual(stderr.read_text(), "x")
            self.assertTrue(stdout.exists())

    def test_analyze_rejects_empty_interpreted_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            raw = root / hc.RAW_NAME
            raw.write_bytes(b"raw")
            os.chmod(raw, 0o600)
            def fake_bounded(argv, *, stdout_path, **kwargs):
                pathlib.Path(stdout_path).write_bytes(b"")
                return {"exit_code": 0}
            with mock.patch.object(hc, "require_free_disk"), \
                 mock.patch.object(hc, "require_mem_available"), \
                 mock.patch.object(hc, "run_bounded", side_effect=fake_bounded):
                with self.assertRaises(hc.CaptureError):
                    hc.analyze(root, sample=lambda _pid: 0)
    def test_prepare_output_is_unique_owned_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "capture"
            info = hc.prepare_output(path)
            self.assertEqual(path.stat().st_mode & 0o777, 0o700)
            self.assertEqual(info["raw"], str((path / "alloc.raw").resolve()))
            with self.assertRaises(hc.CaptureError):
                hc.prepare_output(path)

    def test_prepare_output_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            target = root / "target"
            target.mkdir()
            link = root / "capture"
            link.symlink_to(target, target_is_directory=True)
            with self.assertRaises(hc.CaptureError):
                hc.prepare_output(link)

    def test_child_env_does_not_mutate_parent(self):
        before = {"PATH": "/bin", "LD_PRELOAD": "old"}
        result = hc.child_env(before, {"raw": "/tmp/x/alloc.raw"}, {"path": "/lib/h.so"})
        self.assertEqual(before["LD_PRELOAD"], "old")
        self.assertEqual(result["LD_PRELOAD"], "/lib/h.so")
        self.assertEqual(result["DUMP_HEAPTRACK_OUTPUT"], "/tmp/x/alloc.raw")

    def test_exact_analysis_argv_has_no_wrapper_or_shell(self):
        argv = hc.analysis_argv(pathlib.Path("/tmp/capture"))
        self.assertEqual(argv["interpreter"], ["/usr/lib/heaptrack/libexec/heaptrack_interpret"])
        self.assertEqual(argv["printer"][:4], ["/usr/bin/heaptrack_print", "--merge-backtraces=0",
                                                 "--flamegraph-cost-type=peak", "--print-flamegraph"])
        self.assertNotIn("/bin/sh", argv["interpreter"] + argv["printer"])

    def test_verify_raw_requires_regular0600_owned_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "alloc.raw"
            path.write_bytes(b"raw")
            os.chmod(path, 0o600)
            self.assertEqual(hc.verify_raw(path)["size"], 3)
            os.chmod(path, 0o644)
            with self.assertRaises(hc.CaptureError):
                hc.verify_raw(path)

    def test_preload_hash_mismatch_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "preload.so"
            path.write_bytes(b"wrong")
            with mock.patch.object(hc, "sys", mock.Mock(platform="linux")):
                with self.assertRaises(hc.CaptureError):
                    hc.verify_preload(path)


if __name__ == "__main__":
    unittest.main()
