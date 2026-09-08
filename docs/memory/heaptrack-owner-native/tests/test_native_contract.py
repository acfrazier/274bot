"""Direct native contract rejection: parent validation cannot mask child errors."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from test_native_parity import EXE
from test_differential import ROOT
from test_heaptrack_owner_replay import smoke_manifest
import heaptrack_owner_runner as runner
import heaptrack_owner_replay as replay


class NativeContract(unittest.TestCase):
    def reject(self, change=None, duplicate=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, digest = smoke_manifest(root)
            value = json.loads(path.read_bytes())
            if change:
                change(value)
            data = json.dumps(value).encode()
            if duplicate:
                data = data[:-1]+b',"schema":"heaptrack-owner-input/v1"}'
            path.write_bytes(data)
            digest = hashlib.sha256(data).hexdigest()
            try:
                m = runner.load_manifest(path, digest, True)
                runner.verify_receipt(m)
            except (replay.Invalid, KeyError, TypeError):
                pass
            else:
                self.fail('reference unexpectedly accepted adversary')
            out = root/'out'; out.mkdir(); (out/'.pending').mkdir()
            child = subprocess.run([str(EXE), '--manifest', str(path), '--manifest-sha256', digest, '--output', str(out), '--portable-fixture'], capture_output=True, timeout=30)
            self.assertNotEqual(child.returncode, 0)
            self.assertIn('error', json.loads(child.stdout.splitlines()[-1]))
            self.assertEqual(list((out/'.pending').iterdir()), [])

    def test_duplicate_keys_and_provenance(self):
        self.reject(duplicate=True)
        for key, value in [('intentional_pause', True), ('reinitialized', True), ('collection_mode', 'attach'), ('capture_tooling', '0'*40), ('campaign_capture_status', 'passed'), ('scope', 'production'), ('process_role', 'client_tui')]:
            with self.subTest(key=key):
                self.reject(lambda m: m['provenance'].__setitem__(key, value))
        for key, value in [('requested_time', True), ('requested_time', -1), ('requested_time', 1<<64), ('analysis_path', ['x']*9), ('suppression_policy', 'suppressed'), ('schema', 'wrong')]:
            with self.subTest(key=key, value=value):
                self.reject(lambda m: m.__setitem__(key, value))
        self.reject(lambda m: m['inputs']['stderr'].__setitem__('sha256', '0'*64))

    def test_receipt_warning_options_and_counters(self):
        # Generated metadata copies only; never modify the saved fixture.
        for case in ('warning', 'exit', 'size', 'filter', 'merged', 'nonpeak'):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                p = Path(directory)/'metadata'
                def change(m):
                    role = 'stderr' if case == 'warning' else 'receipt'
                    source = Path(m['inputs'][role]['path']).read_bytes()
                    if case == 'warning':
                        data = source+b'warning\n'
                    else:
                        receipt = json.loads(source)
                        analysis = receipt['analysis']
                        if case == 'exit': analysis['interpreter']['exit_code'] = 1
                        if case == 'size': analysis['interpreter']['output_size'] += 1
                        if case == 'filter': analysis['printer']['argv'].append('--filter=foo')
                        if case == 'merged': analysis['printer']['argv'].remove('--merge-backtraces=0')
                        if case == 'nonpeak': analysis['printer']['argv'].remove('--flamegraph-cost-type=peak')
                        data = json.dumps(receipt).encode()
                    p.write_bytes(data)
                    m['inputs'][role] = dict(path=str(p), bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
                self.reject(change)


if __name__ == '__main__':
    unittest.main()
