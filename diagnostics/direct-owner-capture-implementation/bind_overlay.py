"""Offline hash binding and frozen-index apply check; no checkout or native build."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
H = 'c0709aba2f8b45e42193225cf8f4e7325b5ca9bf'
C = '3456edc8dabf7b25ada78110ffa56327af9f67a4'
INITIAL = '70a09540bb554cf28d07ecc2c9afd0f7b0157542'

def git(root, *args, env=None):
    return subprocess.run(['git', *args], cwd=root, env=env, capture_output=True)

def digest(data):
    return hashlib.sha256(data).hexdigest()

def additions(root, names):
    output = b''
    for name in names:
        result = git(root, 'diff', '--no-index', '--', '/dev/null', name)
        assert result.returncode == 1, result.stderr
        output += result.stdout
    return output

host_new = ['crates/api/tests/direct_owner_capture.rs', 'crates/host-play/src/owner_capture_output.rs', 'crates/host-play/tests/direct_owner_capture.rs', 'crates/script/tests/direct_owner_capture.rs']
host_paths = git(ROOT, 'diff-tree', '--no-commit-id', '--name-only', '-r', INITIAL).stdout.decode().splitlines()
host_paths = [p for p in host_paths if p.startswith('crates/')]
client_root = ROOT/'vendor/fr-client-rust'
client_paths = ['crates/client/Cargo.toml','crates/client/src/core/world.rs','crates/client/src/io/packet.rs']
client_new = ['crates/client/src/core/world_owner_capture.rs','crates/client/tests/direct_owner_capture.rs']
initial = git(ROOT, 'diff', INITIAL+'^', INITIAL, '--', *host_paths).stdout
correction = git(ROOT, 'diff', 'HEAD', '--', *host_paths).stdout + additions(ROOT, host_new)
client_patch = git(client_root, 'diff', C, '--', *client_paths).stdout + additions(client_root, client_new)
patches = {'host-initial.patch': initial, 'host-correction.patch': correction, 'client-overlay.patch': client_patch}
for name, data in patches.items():
    (OUT/name).write_bytes(data)
checks = {}
for label, root, base, names in [('host', ROOT, H, ['host-initial.patch','host-correction.patch']), ('client', client_root, C, ['client-overlay.patch'])]:
    with tempfile.TemporaryDirectory(dir=OUT) as tmp:
        env = dict(os.environ, GIT_INDEX_FILE=str(Path(tmp).resolve()/'index'))
        result = git(root, 'read-tree', base, env=env)
        assert result.returncode == 0, result.stderr
        checks[label] = []
        for name in names:
            result = git(root, 'apply', '--cached', '--check', str(OUT/name), env=env)
            checks[label].append({'patch':name, 'exit':result.returncode, 'stderr':result.stderr.decode()})
            if result.returncode:
                break
            result = git(root, 'apply', '--cached', str(OUT/name), env=env)
            assert result.returncode == 0, result.stderr
manifest = {'original_host':H,'original_client':C,'initial_failed_host':INITIAL,
            'current_host':git(ROOT,'rev-parse','HEAD').stdout.decode().strip(),
            'client_head':git(client_root,'rev-parse','HEAD').stdout.decode().strip(),
            'patches':{name:{'bytes':len(data),'sha256':digest(data)} for name,data in patches.items()},
            'frozen_index_checks':checks,'members':{}}
for label,root,names in [('host',ROOT,host_paths+host_new),('client',client_root,client_paths+client_new)]:
    manifest['members'][label] = [{'path':p,'bytes':len((root/p).read_bytes()),'sha256':digest((root/p).read_bytes())} for p in names]
(OUT/'overlay-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps({'checks':checks,'patches':manifest['patches']},indent=2))
