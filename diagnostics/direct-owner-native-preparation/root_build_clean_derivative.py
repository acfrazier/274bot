"""Root-owned native build from exact archived bytes with real Git identities."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--root', type=Path, required=True)
a = p.parse_args()
root = a.root.resolve()
out = root / 'root-clean-build-01'
out.mkdir()
source = out / 'host'
manifest = json.loads((root / 'frozen-source-manifest.json').read_text())
assert hashlib.sha256((root / 'frozen-source-manifest.json').read_bytes()).hexdigest() == 'ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e'
expected = {row['path']: row for row in manifest['source_members']}

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def git(tree, *args):
    return subprocess.check_output(['git', '-C', str(tree), *args], stderr=subprocess.PIPE).decode().strip()

for row in json.loads((root / 'bundles.json').read_text()):
    bundle = root / row['path']
    assert bundle.stat().st_size == row['bytes'] and digest(bundle) == row['sha256']
subprocess.run(['git', 'clone', str(root / 'host.bundle'), str(source)], check=True)
git(source, 'switch', '-c', 'codex/direct-owner-host-archive')
client = source / 'vendor/fr-client-rust'
subprocess.run(['git', 'clone', str(root / 'client.bundle'), str(client)], check=True)
git(client, 'switch', '-c', 'codex/direct-owner-client-archive')

def verify():
    actual = {str(q.relative_to(source)) for q in source.rglob('*')
              if (q.is_file() or q.is_symlink()) and '.git' not in q.relative_to(source).parts}
    assert actual == set(expected)
    for name, row in expected.items():
        q = source / name
        data = os.readlink(q).encode() if q.is_symlink() else q.read_bytes()
        mode = '120000' if q.is_symlink() else ('100755' if q.stat().st_mode & 0o111 else '100644')
        assert (len(data), hashlib.sha256(data).hexdigest(), mode) == (row['bytes'], row['sha256'], row['mode']), name
    assert git(source, 'rev-parse', 'HEAD') == 'dcdbeebf36665a1d07156402a5f64c823769e00d'
    assert git(client, 'rev-parse', 'HEAD') == 'b74dfb3c998b10371b263189774b055ecf880380'
    assert not git(source, 'status', '--porcelain', '--untracked-files=all')
    assert not git(client, 'status', '--porcelain', '--untracked-files=all')
    hashes = {}
    for label, tree in [('host', source), ('client', client)]:
        h = hashlib.sha256()
        names = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=tree).split(b'\0')
        for name in sorted(set(names)):
            if not name:
                continue
            rel = Path(os.fsdecode(name))
            if (rel.parts[0] == 'crates' or rel.name in ('Cargo.toml', 'Cargo.lock')) and (tree / rel).is_file():
                h.update(name + b'\0' + (tree / rel).read_bytes() + b'\0')
        hashes[label] = h.hexdigest()
    return {'source_member_count': len(expected), 'archive': manifest['archive'],
            'host_commit': git(source, 'rev-parse', 'HEAD'), 'client_commit': git(client, 'rev-parse', 'HEAD'),
            'host_branch': git(source, 'branch', '--show-current'), 'source_digests': hashes,
            'all_source_bytes_modes_verified': True, 'host_clean': True, 'client_clean': True,
            'locks': manifest['locks']}

before = verify()
tool = root / 'qualify_linux.py'
assert digest(tool) == '3a9857bf0a52a32d1b24bcdaf3509fc22f2e034a688ed5c967f4a5207c46d221'
spec = importlib.util.spec_from_file_location('reviewed_qualifier', tool)
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)
env = q.safe_environment()
env['PATH'] = '/home/builder/.cargo/bin:' + env.get('PATH', '')
env['PYTHONDONTWRITEBYTECODE'] = '1'
env['CARGO_TARGET_DIR'] = str(root / 'linux-qualification-01/target')
command = ['cargo', 'build', '--offline', '--locked', '--release', '--target', 'x86_64-unknown-linux-gnu',
           '-p', 'tui', '--bin', 'tui-play', '--features', 'memory-profile-no-alloc,memory-owner-capture']
print('START clean Git derivative release build', flush=True)
step = q.run_step('cargo-build-clean-derivative', command, source, out, env, 900)
result = {'built': False, 'live_qualified': False, 'source_before': before, 'step': step,
          'source_provenance': manifest['provenance'], 'compiler': q.tool_output([shutil.which('rustc', path=env['PATH']), '--version'], source),
          'build_environment': {k: env.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER', 'CARGO_TARGET_DIR', 'BOT_CPU', 'BOT_MEMORY_OWNER_CAPTURE')}}
try:
    assert step['exit'] == 0 and not step['timed_out']
    result['source_after'] = verify()
    assert result['source_after'] == before
    binary = Path(env['CARGO_TARGET_DIR']) / 'x86_64-unknown-linux-gnu/release/tui-play'
    dest = out / 'tui-play'
    shutil.copy2(binary, dest)
    assert digest(binary) == digest(dest)
    result['binary'] = {'path': str(dest), 'bytes': dest.stat().st_size, 'sha256': digest(dest),
                        'file': q.tool_output(['file', str(dest)], source), 'readelf': q.tool_output(['readelf', '-h', str(dest)], source),
                        'ldd': q.tool_output(['ldd', str(dest)], source)}
    assert all(result['binary'][k]['exit'] == 0 for k in ('file', 'readelf', 'ldd'))
    result['built'] = True
finally:
    q.write_json(out / 'result.json', result)
print(json.dumps({'built': result['built'], 'binary': result.get('binary', {}).get('sha256'), 'live_qualified': False}), flush=True)
