"""One root-owned saved fixture pair, after the preserved pre-engine cwd failure."""
from pathlib import Path
import datetime
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time

stage = Path('/home/acfrazier/owner-native-a55972a')
out = stage/'saved-smoke-2'
assert not out.exists()
os.chdir(stage)
assert hashlib.sha256((stage/'native-stage-manifest.json').read_bytes()).hexdigest() == 'd00b2bb28af11ab9bbd4b21ea992d87d7f4d5d084b3e16febb563f04ca53adaf'
manifest = json.loads((stage/'native-stage-manifest.json').read_text())
for item in manifest['files']:
    p = stage/item['path']
    assert p.is_file() and not p.is_symlink() and p.stat().st_size == item['bytes']
    assert hashlib.sha256(p.read_bytes()).hexdigest() == item['sha256']
assert json.loads((stage/'qualification-1/qualification.json').read_text())['status'] == 'fixture_qualification_passed'
mem = {k: int(v.split()[0])*1024 for k, v in (x.split(':', 1) for x in Path('/proc/meminfo').read_text().splitlines())}
conflicts = []
for p in Path('/proc').iterdir():
    if not p.name.isdigit() or int(p.name) == os.getpid():
        continue
    try:
        name = (p/'comm').read_text().strip()
        if any(s in name for s in ('cargo', 'rustc', 'heaptrack', 'tui-play', 'panel', 'perf')):
            conflicts.append(dict(pid=int(p.name), name=name))
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        pass
disk = shutil.disk_usage(stage).free
assert mem['MemAvailable'] >= 768*1024**2 and disk >= 8*1024**3 and not conflicts
out.mkdir(mode=0o700)
receipt = dict(status='incomplete', qualification_repeated=False, prior_smoke_launch='saved-smoke-1: failed before engines', unix=time.time(), utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), wrapper_pid=os.getpid(), cwd=str(Path.cwd()), boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(), mem_available=mem['MemAvailable'], disk_free=disk, conflicts=conflicts, all_stage_files_hashed=len(manifest['files']), commands=[], production_inputs=False)
(out/'admission.json').write_text(json.dumps(receipt, indent=2)+'\n')
sys.path.insert(0, str(stage/'docs/memory'))
sys.path.insert(0, str(stage/'docs/memory/heaptrack-owner-native/tests'))
from test_heaptrack_owner_replay import smoke_manifest
from test_native_parity import SAVED_HASHES

try:
    fixture_manifest, digest = smoke_manifest(out)
    bound = json.loads(fixture_manifest.read_text())
    for kind, entry in bound['inputs'].items():
        p = Path(entry['path'])
        assert p.is_relative_to(stage) and p.is_file() and not p.is_symlink()
        assert entry['bytes'] <= 2_000_000 and p.stat().st_size == entry['bytes']
        assert hashlib.sha256(p.read_bytes()).hexdigest() == entry['sha256'] == SAVED_HASHES[kind]
    receipt['manifest_sha256'] = digest
    exe = stage/'docs/memory/heaptrack-owner-native/target/release/heaptrack-owner-child'
    native_hash = '98b5a2a20b0daf66057024f4618972fb34219b34568a09fa3be89a674d265b7f'
    assert hashlib.sha256(exe.read_bytes()).hexdigest() == native_hash
    for engine in ('python', 'native'):
        command = [sys.executable, '-B', str(stage/'docs/memory/heaptrack_owner_runner.py'), '--manifest', str(fixture_manifest), '--manifest-sha256', digest, '--output', str(out/engine), '--portable-fixture', '--engine', engine, '--limits', json.dumps(dict(cpu=30, wall=60))]
        if engine == 'native':
            command += ['--native-executable', str(exe), '--native-sha256', native_hash]
        started = time.time()
        record = dict(engine=engine, argv=command, started_unix=started)
        receipt['commands'].append(record)
        with (out/(engine+'.stdout')).open('wb') as stdout, (out/(engine+'.stderr')).open('wb') as stderr:
            child = subprocess.Popen(command, cwd=stage, stdout=stdout, stderr=stderr, start_new_session=True)
            record['runner_pid'] = child.pid
            (out/'progress.json').write_text(json.dumps(receipt, indent=2)+'\n')
            try:
                rc = child.wait(timeout=90)
            except BaseException:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
                raise
        record.update(returncode=rc, wall_s=time.time()-started, runner_reaped=True)
        assert rc == 0, engine+' runner failed'
    a, b = out/'python/result', out/'native/result'
    assert {p.name for p in a.iterdir()} == {p.name for p in b.iterdir()}
    comparisons = []
    for p in sorted(a.iterdir()):
        q = b/p.name
        if p.suffix == '.tsv':
            assert p.read_bytes() == q.read_bytes(), p.name
        else:
            x, y = json.loads(p.read_bytes()), json.loads(q.read_bytes())
            for v in (x, y):
                v.pop('resources', None)
                v.get('tables', {}).pop('high_charged_bytes', None)
            assert x == y, p.name
        comparisons.append(dict(name=p.name, python_sha256=hashlib.sha256(p.read_bytes()).hexdigest(), native_sha256=hashlib.sha256(q.read_bytes()).hexdigest(), equal_excluding_documented_resource_fields=True))
    receipt.update(status='saved_fixture_pair_passed', comparisons=comparisons, linux_supervision=True, production_capacity=False)
except BaseException as exc:
    receipt.update(status='failed', error=type(exc).__name__+': '+str(exc))
    raise
finally:
    receipt['ended_unix'] = time.time()
    (out/'root-receipt.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps({k: receipt.get(k) for k in ('status', 'error', 'wrapper_pid', 'ended_unix')}))
