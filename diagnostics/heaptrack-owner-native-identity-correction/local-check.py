#!/usr/bin/env python3
"""Bounded local identity/mapping checks, not the native qualification driver."""
import ast
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / 'docs/memory/heaptrack-owner-native'
sys.path.insert(0, str(PACKAGE.parent))
import heaptrack_owner_replay as reference


def reference_checks():
    original = b'# padding\n' * 7000
    results = {}
    with tempfile.TemporaryDirectory(prefix='owner-identity-') as directory:
        path = Path(directory) / 'raw'
        entry = dict(path=str(path), bytes=len(original),
                     sha256=hashlib.sha256(original).hexdigest())
        # Both cases deliberately simulate equal fstat results. The real file
        # write remains real; this is not a native filesystem-resolution probe.
        for name, index, expected in [('buffered', 1, None),
                                      ('unread', 69991, 'input hash mismatch')]:
            path.write_bytes(original)
            before = path.stat()
            stream = reference.Input(entry, 2_000_000, 1 << 20)
            changed = bytearray(original)
            changed[index] = ord('!')
            with patch.object(reference.os, 'fstat', return_value=before):
                iterator = iter(stream)
                first = next(iterator)[2]
                path.write_bytes(changed)
                try:
                    consumed = first + b''.join(row[2] for row in iterator)
                except reference.Invalid as error:
                    assert str(error) == expected, str(error)
                else:
                    assert expected is None
                    assert consumed == original
                    assert hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']
            results[name + '_simulated_equal_metadata'] = 'pass'
        # Real metadata change with a deliberately distinct representable mtime.
        path.write_bytes(original)
        iterator = iter(reference.Input(entry, 2_000_000, 1 << 20))
        next(iterator)
        reference.os.utime(path, ns=(1_000_000_000, 1_000_000_000))
        try:
            list(iterator)
        except reference.Invalid as error:
            assert str(error) == 'input changed during pass', str(error)
        else:
            raise AssertionError('metadata mutation accepted')
        results['real_metadata_change'] = 'pass'
    return results


def main():
    build = subprocess.check_output([
        'cargo', 'test', '--release', '--offline', '--locked', '--manifest-path',
        str(PACKAGE / 'Cargo.toml'), '--bin', 'heaptrack-owner-child',
        '--no-run', '--message-format=json'], text=True, timeout=180)
    rows = [json.loads(line) for line in build.splitlines() if line.startswith('{')]
    artifacts = [row['executable'] for row in rows
                 if row.get('reason') == 'compiler-artifact'
                 and row.get('profile', {}).get('test') and row.get('executable')]
    assert len(artifacts) == 1
    names = {line.split(': test')[0] for line in subprocess.check_output(
        [artifacts[0], '--list'], text=True, timeout=10).splitlines() if ': test' in line}
    suite = unittest.defaultTestLoader.discover(str(PACKAGE / 'tests'))

    def ids(group):
        for item in group:
            if isinstance(item, unittest.TestSuite):
                yield from ids(item)
            else:
                yield '.'.join(item.id().split('.')[-2:])

    native = set(ids(suite))
    source = ast.parse((PACKAGE.parent / 'test_heaptrack_owner_replay.py').read_text())
    original = {node.name[5:] for node in ast.walk(source)
                if isinstance(node, ast.FunctionDef) and node.name.startswith('test_')}
    mapping = json.loads((PACKAGE / 'test-mapping.json').read_text())
    assert len(original) == 21 and set(mapping) == original
    assert all(name in names | native for group in mapping.values() for name in group)
    repetitions = []
    for _ in range(20):
        result = subprocess.run([artifacts[0], 'core::migration_tests::', '--test-threads=8'],
                                capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, result.stdout + result.stderr
        assert '9 passed; 0 failed' in result.stdout, result.stdout
        repetitions.append(dict(exit_code=result.returncode, passed=9, failed=0))
    print(json.dumps(dict(mapping_count=len(mapping), core_count=len(names),
                         native_python_count=len(native), all_mapping_targets_exist=True,
                         migration_repetitions=repetitions, reference=reference_checks(),
                         linux_qualification=False, production_retry_authorized=False), indent=2))


if __name__ == '__main__':
    main()
