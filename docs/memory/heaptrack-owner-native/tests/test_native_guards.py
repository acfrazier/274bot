"""Real native process failures; Linux AS remains explicitly gated."""
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile
import unittest
from test_differential import ROOT, EXE
sys.path.insert(0, str(ROOT))
import heaptrack_owner_replay as replay
import heaptrack_owner_runner as runner
from test_heaptrack_owner_replay import smoke_manifest
from test_native_parity import EXE as CHILD
import hashlib


class NativeGuards(unittest.TestCase):
    def test_native_protocol_failures(self):
        for case in ('abnormal', 'incomplete', 'malformed', 'duplicate'):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(Exception) as caught:
                    runner.supervise([str(EXE), '--guard', case], Path(directory), replay.LIMITS, sys.platform == 'linux')
                self.assertTrue(getattr(caught.exception, 'replay_resources')['owned_child_reaped'])
                self.assertFalse((Path(directory)/'.pending').exists())

    def test_all_external_samples_and_admission(self):
        from test_heaptrack_owner_replay import ReplayTests
        ReplayTests().test_resource_guards_and_parent_unchanged()

    def guarded(self, case, limits, reason):
        before = (resource.getrlimit(resource.RLIMIT_CPU), resource.getrlimit(resource.RLIMIT_FSIZE), dict(os.environ))
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(replay.Invalid) as caught:
                runner.supervise([str(EXE), '--guard', case], Path(directory), replay.Budget(limits).limits, sys.platform == 'linux')
            self.assertIn(reason, str(caught.exception))
            self.assertTrue(getattr(caught.exception, 'replay_resources')['owned_child_reaped'])
            self.assertFalse((Path(directory)/'.pending').exists())
        self.assertEqual(before, (resource.getrlimit(resource.RLIMIT_CPU), resource.getrlimit(resource.RLIMIT_FSIZE), dict(os.environ)))

    def test_native_cpu(self):
        self.guarded('cpu', {}, 'CPU guard')

    def test_native_wall_reap(self):
        self.guarded('wall', {'wall': 1}, 'wall')

    def test_native_phase_end(self):
        self.guarded('phase_end', {}, 'wall')

    def test_native_rss(self):
        self.guarded('rss', {'rss': 32*replay.MIB}, 'RSS guard')

    @unittest.skipUnless(sys.platform == 'linux', 'Linux native RLIMIT_AS requires root qualification')
    def test_native_address(self):
        self.guarded('address', {}, 'allocation failure')

    def test_explicit_hash_and_no_fallback(self):
        for options in (['--engine', 'native'], ['--engine', 'native', '--native-executable', str(CHILD), '--native-sha256', '0'*64]):
            with self.subTest(options=options), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                m, h = smoke_manifest(root)
                p = subprocess.run([sys.executable, '-B', str(ROOT/'heaptrack_owner_runner.py'), '--manifest', str(m), '--manifest-sha256', h, '--output', str(root/'out'), '--portable-fixture']+options, capture_output=True, timeout=30)
                self.assertEqual(p.returncode, 1)
                self.assertEqual([x.name for x in (root/'out').iterdir()], ['failure.json'])
                self.assertIn('native', json.loads((root/'out/failure.json').read_text())['reason'])

    def test_native_output_table_scratch_fail_closed(self):
        for limits, reason in [({'table_bytes': 1}, 'table byte cap'), ({'output': 100}, 'output cap'), ({'scratch': 100}, 'scratch')]:
            with self.subTest(limits=limits), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                m, h = smoke_manifest(root)
                p = subprocess.run([sys.executable, '-B', str(ROOT/'heaptrack_owner_runner.py'), '--manifest', str(m), '--manifest-sha256', h, '--output', str(root/'out'), '--portable-fixture', '--engine', 'native', '--native-executable', str(CHILD), '--native-sha256', hashlib.sha256(CHILD.read_bytes()).hexdigest(), '--limits', json.dumps(limits)], capture_output=True, timeout=30)
                self.assertEqual(p.returncode, 1)
                self.assertEqual([x.name for x in (root/'out').iterdir()], ['failure.json'])
                receipt = json.loads((root/'out/failure.json').read_text())
                self.assertIn(reason, receipt['reason'])
                self.assertTrue(receipt['resources']['owned_child_reaped'])


if __name__ == '__main__':
    unittest.main()
