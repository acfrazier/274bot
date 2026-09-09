"""Bounded offline suites. No live flags, cache fixtures or frontend launch."""
import json
import os
from pathlib import Path
import subprocess
import signal
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
cases = []
for capture in (False, True):
    suffix = 'on' if capture else 'off'
    for package in ('api', 'host', 'script', 'host-play', 'tui'):
        features = []
        if package == 'script':
            features.append('load')
        if package in ('host-play', 'tui'):
            features.append('memory-profile-no-alloc')
        if capture:
            features.append('memory-owner-capture')
        command = ['cargo', 'test', '--offline', '-p', package]
        if features:
            command += ['--features', ','.join(features)]
        command += ['--', '--test-threads=1']
        cases.append((package + '-' + suffix, ROOT, command))
    for kind, flag in (('unit', '--lib'), ('integration', '--tests')):
        command = ['cargo', 'test', '--offline', '-p', 'client', flag]
        if capture:
            command += ['--features', 'memory-owner-capture']
        command += ['--', '--test-threads=1']
        cases.append(('client-' + kind + '-' + suffix, ROOT / 'vendor/fr-client-rust', command))
results = []
env = dict(os.environ)
env.pop('LIVE', None)
env['BOT_MEMORY_OWNER_CAPTURE'] = '0'
for name, cwd, command in cases:
    start = time.monotonic()
    with (OUT / (name + '.log')).open('w') as log:
        log.write(json.dumps({'cwd': str(cwd), 'command': command}) + '\n')
        log.flush()
        result = subprocess.Popen(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = result.wait(timeout=600)
        except subprocess.TimeoutExpired:
            code = 'timeout'
        finally:
            try:
                os.killpg(result.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            result.wait()
    results.append({'name': name, 'command': command, 'exit': code, 'wall_s': time.monotonic() - start})
    (OUT / 'regressions.json').write_text(json.dumps(results, indent=2) + '\n')
    print(name, code, flush=True)
