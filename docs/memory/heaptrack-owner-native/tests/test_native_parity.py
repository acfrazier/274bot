"""Native saved-smoke differential tests; no production input paths."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from test_heaptrack_owner_replay import smoke_manifest

EXE = ROOT / 'heaptrack-owner-native/target/release/heaptrack-owner-child'
SAVED_HASHES = {
    'raw': 'f0b8bbcff61b7fd0c0ff7257ef40fc3c615365630367849736ff0fa031698146',
    'interpreted': '96d8b258f3bbbdf97deeb4b56691d12a23fa43ac5023cdfc3b53c9f6aa435acb',
    'peak': 'bbfa1a2bdd75ae37fb9c3a58317a43d48ba741643b0e63c1f9b5a59cd364a943',
    'receipt': '1f7c8d28af396ea75368fb2c32a3fb17884e66b436a580808c25584388e05a78',
    'stderr': '6bd12888cdcc6f777f2d4ba9b8096876057c0aed2391e4b5ea1a7540d15e365b',
}


class NativeParity(unittest.TestCase):
    maxDiff = None
    def test_saved_smoke(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, digest = smoke_manifest(root)
            for role, entry in json.loads(manifest.read_bytes())['inputs'].items():
                self.assertLessEqual(entry['bytes'], 2_000_000)
                self.assertEqual(entry['sha256'], SAVED_HASHES[role])
            for engine in ('python', 'native'):
                command = [sys.executable, '-B', str(ROOT/'heaptrack_owner_runner.py'),
                           '--manifest', str(manifest), '--manifest-sha256', digest,
                           '--output', str(root/engine), '--portable-fixture', '--engine', engine]
                if engine == 'native':
                    command += ['--native-executable', str(EXE), '--native-sha256',
                                hashlib.sha256(EXE.read_bytes()).hexdigest()]
                result = subprocess.run(command, capture_output=True, timeout=90)
                failure = root/engine/'failure.json'
                self.assertEqual(result.returncode, 0, failure.read_text() if failure.exists() else result.stderr.decode())
            reference = root/'python/result'
            native = root/'native/result'
            self.assertEqual({p.name for p in reference.iterdir()}, {p.name for p in native.iterdir()})
            for path in reference.iterdir():
                with self.subTest(file=path.name):
                    if path.suffix == '.tsv':
                        self.assertEqual(path.read_bytes(), (native/path.name).read_bytes())
                    else:
                        a, b = json.loads(path.read_bytes()), json.loads((native/path.name).read_bytes())
                        a.pop('resources', None)
                        b.pop('resources', None)
                        # Engine-specific capacity accounting is a resource,
                        # not semantic replay state; retain all table counts.
                        a.get('tables', {}).pop('high_charged_bytes', None)
                        b.get('tables', {}).pop('high_charged_bytes', None)
                        self.assertEqual(a, b)


if __name__ == '__main__':
    unittest.main()
