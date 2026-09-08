#!/usr/bin/env python3
"""Finite predeclared Mac-only comparison; no arbitrary input or qualification."""
import hashlib
import json
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
NATIVE = REPO/'docs/memory/heaptrack-owner-native'
sys.path.insert(0, str(NATIVE))
import qualify as q

CASES = ((1850048, False), (1850048, True), (1000, 'diversity'))
def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def normalized(result):
    result = json.loads(json.dumps(result))
    result.pop('measurements', None)
    for name, text in result['files'].items():
        if name.endswith('.json'):
            value = json.loads(text)
            value.pop('resources', None)
            value['tables'].pop('high_charged_bytes', None)
            result['files'][name] = value
    return result

def main():
    role = sys.argv[1]
    assert role in ('baseline', 'candidate') and sys.platform == 'darwin'
    root = HERE/role
    root.mkdir()
    sources = list((NATIVE/'src').glob('*.rs')) + [NATIVE/'Cargo.toml', NATIVE/'Cargo.lock', NATIVE/'qualify.py', Path(__file__)]
    source_hashes = {str(p.relative_to(REPO)): sha(p) for p in sources}
    binaries = {}
    for name in ('heaptrack-owner-child', 'heaptrack-owner-fixture'):
        shutil.copy2(NATIVE/'target/release'/name, root/name)
        binaries[name] = sha(root/name)
    report = dict(role=role, platform=platform.platform(), source_hashes=source_hashes, binaries=binaries,
                  compiler=subprocess.check_output(['rustc', '-Vv'], text=True), cases=[], status='incomplete',
                  production_read=False, qualification=False)
    (root/'freeze.json').write_text(json.dumps(report, indent=2)+'\n')
    (root/'processes.txt').write_text(subprocess.check_output(['ps', '-axo', 'pid,comm'], text=True))
    start = time.monotonic()
    try:
        for index, (size, wide) in enumerate(CASES):
            raw, interpreted, peak = q.generated(size, wide)
            assert max(map(len, (raw, interpreted, peak))) <= 2_000_000
            with q.fixture(raw, interpreted, peak, heads=False) as entries:
                expected = q.reference.analyze(entries, limits={'cpu':30, 'wall':60})
                with tempfile.TemporaryDirectory() as directory:
                    out = Path(directory)
                    q.reference.write_outputs(expected, out, dict(host=q.reference.HOST, client=q.reference.CLIENT))
                    expected = {k: expected[k] for k in ('raw','first','snapshots','canonical','canonical_equal')}
                    expected['files'] = {p.name:p.read_text() for p in out.iterdir()}
                    expected = normalized(expected)
            request = dict(op='analyze', raw=raw.hex(), interpreted=interpreted.hex(), peak=peak.hex(), requested_time=None, outputs=True, limits={'cpu':30,'wall':60})
            cell = dict(size=size, wide=wide, bytes=[len(raw),len(interpreted),len(interpreted)],
                        hashes=[hashlib.sha256(b).hexdigest() for b in (raw,interpreted,peak)],
                        records=[raw.count(b'\n'),interpreted.count(b'\n')], repeats=[])
            report['cases'].append(cell)
            for repeat in range(3):
                assert time.monotonic()-start < 720
                result, resources = q.owned(request, root/'heaptrack-owner-fixture', 900-(time.monotonic()-start))
                filename = f'case-{index}-repeat-{repeat}.json'
                (root/filename).write_text(json.dumps(dict(result=result, resources=resources), indent=2)+'\n')
                assert normalized(result) == expected, 'reference full semantic/output mismatch'
                if role == 'candidate':
                    base = json.loads((HERE/'baseline'/filename).read_text())['result']
                    assert normalized(result) == normalized(base), 'baseline full semantic/output mismatch'
                cell['repeats'].append(dict(file=filename, sha256=sha(root/filename), phases=result['measurements'], resources=resources))
            cell['median_cpu_s'] = [statistics.median(r['phases'][i]['cpu_s'] for r in cell['repeats']) for i in range(3)]
        assert source_hashes == {str(p.relative_to(REPO)):sha(p) for p in sources}
        assert binaries == {name:sha(root/name) for name in binaries}
        report['status'] = 'comparison_passed'
    except Exception as exc:
        report['failure'] = repr(exc)
        raise
    finally:
        report['elapsed_s'] = time.monotonic()-start
        (root/'results.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({"role":role, "status":report['status'], "medians":[c['median_cpu_s'] for c in report['cases']]}))

if __name__ == '__main__':
    main()
