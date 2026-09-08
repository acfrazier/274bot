#!/usr/bin/env python3
"""Audit retained comparison and mapping without running another timing cell."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
NATIVE = REPO/'docs/memory/heaptrack-owner-native'
sys.path.insert(0, str(NATIVE/'tests'))
sys.path.insert(0, str(NATIVE.parent))
import test_heaptrack_owner_replay as original
import test_differential as d

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def flatten(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from flatten(test)
        else:
            yield test

mapping = json.loads((NATIVE/'test-mapping.json').read_text())
assert set(mapping) == {n.removeprefix('test_') for n in unittest.defaultTestLoader.getTestCaseNames(original.ReplayTests)}
suite = unittest.defaultTestLoader.discover(str(NATIVE/'tests'))
ids = {'.'.join(t.id().split('.')[-2:]) for t in flatten(suite)}
core_log = (HERE/'core-tests.log').read_text()
for names in mapping.values():
    for name in names:
        assert name in ids or f'test {name} ... ok' in core_log, name
reports = [json.loads((HERE/role/'results.json').read_text()) for role in ('baseline','candidate')]
for role, report in zip(('baseline','candidate'), reports):
    assert report['status'] == 'comparison_passed'
    for name, digest in report['binaries'].items():
        assert sha(HERE/role/name) == digest
    for case in report['cases']:
        for repeat in case['repeats']:
            assert sha(HERE/role/repeat['file']) == repeat['sha256']
    for name, digest in report['source_hashes'].items():
        if role == 'baseline' and name.startswith('docs/memory/heaptrack-owner-native/'):
            frozen = subprocess.check_output(['git','show','8b4d6f5a3686e85e5fed55574b205936d4be45a1:'+name],cwd=REPO)
            assert hashlib.sha256(frozen).hexdigest() == digest
        else:
            assert sha(REPO/name) == digest
conflicts = {}
for role in ('baseline','candidate'):
    conflicts[role] = [line.strip() for line in (HERE/role/'processes.txt').read_text().splitlines() if re.search(r'/(cargo|rustc|heaptrack-owner-\w+|panel|tui-play)$',line)]
# Preserve the newly found baseline discrepancy instead of normalizing it away.
with d.fixture(b'c 0\nc a\n', b'c 0\nc a\n', b'f; 0\n') as entries:
    request = dict(op='analyze',requested_time=0,outputs=True)
    request.update({k:Path(v['path']).read_bytes().hex() for k,v in entries.items()})
    expected = d.reference.analyze(entries,requested_time=0)
    outputs = {}
    for role in ('baseline','candidate'):
        process = subprocess.run([str(HERE/role/'heaptrack-owner-fixture')],input=json.dumps(request).encode(),capture_output=True,timeout=60)
        assert process.returncode == 0
        result = json.loads(process.stdout)
        assert result['first']['last_event'] is None
        outputs[role] = result['first']['definitions']
    mismatch = dict(reference=expected['first']['definitions'], **outputs)
    assert mismatch['baseline'] == mismatch['candidate'] != mismatch['reference']
print(json.dumps(dict(mapping_keys=len(mapping), native_tests=len(ids), retained_repetitions=sum(len(c['repeats']) for r in reports for c in r['cases']), source_and_binary_and_artifact_hashes_match=True, known_zero_descriptor_mismatch=mismatch, process_snapshot_conflicts=conflicts,
                     sizes={str(p.relative_to(HERE)):p.stat().st_size for p in HERE.rglob('*') if p.is_file() and p.name != 'audit.json' and p.suffix in ('.json','.log','.py','.md')}
                    ),indent=2))
