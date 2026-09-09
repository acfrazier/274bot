"""Root native qualification; reuse reviewed matrix/runner with an owned Cargo cache."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import sys

p = argparse.ArgumentParser()
p.add_argument('--root', type=Path, required=True)
a = p.parse_args()
r = a.root.resolve()
out = r / 'root-coverage-native-02'
out.mkdir()
helper = r / 'coverage-overlay/stage_coverage_overlay.py'
expected = {
    helper: '6e1aab8eae3bd217f1f2b320f8b1b45e7a8fae924563dd7c74449ed4af5a3082',
    r / 'coverage-overlay/coverage-test-only.patch': 'dcb84047157f75c7ad21868b72eb7c3434cb818e8ba3e4b732f6b40d435c1b6e',
    r / 'tui-test-only.patch': '3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4',
    r / 'frozen-direct-owner-source.tar.gz': '2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d',
    r / 'frozen-source-manifest.json': 'ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e',
}
def write_atomic(path, value):
    pending = path.with_name(path.name + '.next')
    with pending.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(pending, path)

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
for path, digest in expected.items():
    assert sha(path) == digest, path
assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
spec = importlib.util.spec_from_file_location('coverage', helper)
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)
sys.argv = [str(helper), '--archive', str(r / 'frozen-direct-owner-source.tar.gz'),
            '--manifest', str(r / 'frozen-source-manifest.json'), '--output-dir', str(out / 'stage'),
            '--tui-patch', str(r / 'tui-test-only.patch'), '--keep-source']
with (out / 'stage.log').open('x') as log, contextlib.redirect_stdout(log):
    assert q.main() == 0
source = out / 'stage/derived-source'
manifest = json.loads((r / 'frozen-source-manifest.json').read_text())
target = r / 'linux-qualification-01/target'
assert target.is_dir() and not target.is_symlink()
env = q.safe_environment(target)
env['PATH'] = '/home/builder/.cargo/bin:' + env.get('PATH', '')
env['PYTHONDONTWRITEBYTECODE'] = '1'
env['CARGO_BUILD_TARGET'] = 'x86_64-unknown-linux-gnu'
for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'):
    env.pop(key, None)
steps = []
receipt = {'scope': 'Linux generated coverage only; no live or performance admission',
           'prior_failed_attempt': 'root-coverage-native-01', 'prior_failure_archive_sha256': '46d7076954362c3eed75fab9757e8c2c97251e068de5d082bccbed6226ecfcf9',
           'stage_receipt_is_preparation_only': True, 'live_qualified': False,
           'helper_sha256': sha(helper), 'runner_sha256': sha(Path(__file__)),
           'environment': {k: env.get(k) for k in ('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER')},
           'source_before': q.verify_tree(source, manifest, q.DERIVED_HASHES),
           'locks_before': q.verify_locks(source, manifest, 'before-native-tests'),
           'steps': steps, 'all_requested_passed': False}
write_atomic(out / 'launch.json', receipt)
try:
    matrix = q.gap_matrix(source) + q.feature_off_matrix(source) + q.feature_on_matrix(source)
    assert len(matrix) == 19
    receipt['required_steps'] = [name for name, _, _ in matrix]
    for name, command, cwd in matrix:
        step_env = dict(env)
        if name in {'feature-off-host', 'feature-on-host'}:
            step_env.pop('BOT_CPU', None)
            step_env['R274_TEST_FORCE_NO_GPU'] = '1'
        if shutil.disk_usage(r).free < 4 * 1024**3:
            receipt['resource_failure'] = 'less than 4GiB free before ' + name
            break
        print('START', name, flush=True)
        step = q.run_step(name, command, cwd, out, step_env, 900)
        steps.append(step)
        write_atomic(out / 'steps.json', steps)
        print('END', name, step['exit'], round(step['wall_s'], 3), flush=True)
        if step['exit'] != 0 or step['timed_out'] or not step['cleanup']['group_absent']:
            break
    receipt['all_requested_passed'] = len(steps) == 19 and all(s['exit'] == 0 and not s['timed_out'] and s['cleanup']['group_absent'] for s in steps)
finally:
    receipt['not_run'] = receipt.get('required_steps', [])[len(steps):]
    receipt['disk_after'] = shutil.disk_usage(r)._asdict()
    receipt['source_after'] = q.verify_tree(source, manifest, q.DERIVED_HASHES)
    receipt['locks_after'] = q.verify_locks(source, manifest, 'after-native-tests')
    for path, digest in expected.items():
        assert sha(path) == digest, path
    write_atomic(out / 'result.json', receipt)
print('COMPLETE', receipt['all_requested_passed'], len(steps), flush=True)
raise SystemExit(0 if receipt['all_requested_passed'] and len(steps) == 19 else 1)
