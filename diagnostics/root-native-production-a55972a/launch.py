"""One root-released native production-prefix replay; no retry or capture."""
from pathlib import Path
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time

stage = Path('/home/acfrazier/owner-native-a55972a')
output = stage/'production-replay-1'
control = stage/'production-replay-1-control'
assert not output.exists() and not control.exists()
control.mkdir(mode=0o700)
receipt = dict(status='preflight', wrapper_pid=os.getpid(), started_unix=time.time(), attempt_limit=1, production_capture=False, limits_changed=False)
try:
    mpath = stage/'production-input.json'
    expected = '7fcd08aed20e44c81dd495f83ae663b167f4392b84c39a79ce927b45165e1595'
    assert hashlib.sha256(mpath.read_bytes()).hexdigest() == expected
    manifest = json.loads(mpath.read_text())
    stage_manifest = stage/'native-stage-manifest.json'
    assert hashlib.sha256(stage_manifest.read_bytes()).hexdigest() == 'd00b2bb28af11ab9bbd4b21ea992d87d7f4d5d084b3e16febb563f04ca53adaf'
    sources = json.loads(stage_manifest.read_text())
    for item in sources['files']:
        p = stage/item['path']
        assert p.is_file() and not p.is_symlink() and p.stat().st_size == item['bytes']
        assert hashlib.sha256(p.read_bytes()).hexdigest() == item['sha256']
    # Do not add a fourth full raw/interpreted read: the reviewed streaming
    # reader checks their bound bytes/hash and stat identity in its passes.
    inputs = {}
    for kind, entry in manifest['inputs'].items():
        p = Path(entry['path'])
        assert p.is_file() and not p.is_symlink() and p.stat().st_size == entry['bytes']
        stat = p.stat()
        inputs[kind] = dict(path=str(p), bytes=stat.st_size, inode=stat.st_ino, device=stat.st_dev, mtime_ns=stat.st_mtime_ns, ctime_ns=stat.st_ctime_ns)
        if kind in ('receipt', 'stderr'):
            assert hashlib.sha256(p.read_bytes()).hexdigest() == entry['sha256']
    mem = {k: int(v.split()[0])*1024 for k, v in (x.split(':', 1) for x in Path('/proc/meminfo').read_text().splitlines())}
    disk = shutil.disk_usage(stage).free
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
    assert mem['MemAvailable'] >= 768*1024**2 and disk >= 8*1024**3 and not conflicts
    exe = stage/'docs/memory/heaptrack-owner-native/target/release/heaptrack-owner-child'
    native_hash = '98b5a2a20b0daf66057024f4618972fb34219b34568a09fa3be89a674d265b7f'
    assert hashlib.sha256(exe.read_bytes()).hexdigest() == native_hash
    command = [sys.executable, '-B', str(stage/'docs/memory/heaptrack_owner_runner.py'), '--manifest', str(mpath), '--manifest-sha256', expected, '--output', str(output), '--engine', 'native', '--native-executable', str(exe), '--native-sha256', native_hash]
    receipt.update(status='admitted', admission_unix=time.time(), boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(), mem_available=mem['MemAvailable'], disk_free=disk, conflicts=conflicts, stage_files_hashed=len(sources['files']), input_stat=inputs, argv=command, cwd=str(stage), manifest_sha256=expected, native_sha256=native_hash)
    (control/'admission.json').write_text(json.dumps(receipt, indent=2)+'\n')
    with (control/'runner.stdout').open('wb') as stdout, (control/'runner.stderr').open('wb') as stderr:
        child = subprocess.Popen(command, cwd=stage, stdout=stdout, stderr=stderr, start_new_session=True)
        receipt.update(status='running', runner_pid=child.pid, launched_unix=time.time())
        (control/'launch.json').write_text(json.dumps(receipt, indent=2)+'\n')
        print(json.dumps({k: receipt[k] for k in ('status', 'runner_pid', 'launched_unix')}), flush=True)
        try:
            code = child.wait(timeout=930)
        except BaseException:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            raise
    receipt.update(status='runner_finished', returncode=code, runner_reaped=True)
except BaseException as exc:
    receipt.update(status='wrapper_failed', error=type(exc).__name__+': '+str(exc))
    raise
finally:
    receipt['ended_unix'] = time.time()
    (control/'completion.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps({k: receipt.get(k) for k in ('status', 'returncode', 'error', 'runner_pid', 'ended_unix')}), flush=True)
sys.exit(code)
