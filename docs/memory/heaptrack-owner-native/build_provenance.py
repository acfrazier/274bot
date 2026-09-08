#!/usr/bin/env python3
"""Build the isolated native unit-test artifact and record local build identities."""
import ast
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parent

def command(args):
    return subprocess.check_output(args, cwd=ROOT, timeout=180).decode()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    build = command(['cargo', 'test', '--release', '--locked', '--offline', '--bin', 'heaptrack-owner-child', '--no-run', '--message-format=json'])
    artifacts = [json.loads(line) for line in build.splitlines() if line.startswith('{')]
    tests = [a['executable'] for a in artifacts if a.get('reason')=='compiler-artifact' and a.get('profile', {}).get('test') and a.get('executable')]
    assert len(tests)==1
    staged = ROOT/'target/release/heaptrack-owner-core-tests'
    shutil.copy2(tests[0], staged)
    unit_names = {line.split(': test')[0] for line in command([str(staged), '--list']).splitlines() if ': test' in line}
    sys.path.insert(0, str(ROOT/'tests'))
    suite = unittest.defaultTestLoader.discover(str(ROOT/'tests'))
    def ids(suite):
        for item in suite:
            if isinstance(item, unittest.TestSuite): yield from ids(item)
            else: yield '.'.join(item.id().split('.')[-2:])
    native_names = set(ids(suite))
    source = ast.parse((ROOT.parent/'test_heaptrack_owner_replay.py').read_text())
    original = {n.name[5:] for n in ast.walk(source) if isinstance(n, ast.FunctionDef) and n.name.startswith('test_')}
    mapping = json.loads((ROOT/'test-mapping.json').read_text())
    assert len(original)==21 and set(mapping)==original
    assert all(name in native_names | unit_names for group in mapping.values() for name in group)
    files = sorted([*ROOT.glob('src/*.rs'), *ROOT.glob('tests/*.py'), *ROOT.glob('*.py'), ROOT/'Cargo.toml', ROOT/'Cargo.lock', ROOT/'test-mapping.json', ROOT.parent/'heaptrack_owner_replay.py', ROOT.parent/'heaptrack_owner_runner.py'])
    compiler = Path(command(['rustc', '--print', 'sysroot']).strip())/'bin/rustc'
    value = dict(schema='heaptrack-owner-native-build/v1', platform=sys.platform,
                 rustc=command(['rustc', '-Vv']), cargo=command(['cargo', '-V']), compiler_sha256=sha(compiler),
                 sources={str(p.relative_to(ROOT.parent)):sha(p) for p in files},
                 executables={name:sha(ROOT/'target/release'/name) for name in ('heaptrack-owner-child','heaptrack-owner-fixture','heaptrack-owner-core-tests')},
                 original_test_mapping_count=len(mapping), native_python_test_count=len(native_names), native_core_test_count=len(unit_names),
                 linux_qualification=False, production_retry_authorized=False)
    (ROOT/'build-provenance.json').write_text(json.dumps(value,indent=2)+'\n')
    print(json.dumps({k:value[k] for k in ('executables','original_test_mapping_count','native_python_test_count','native_core_test_count')},indent=2))


if __name__=='__main__':
    main()
