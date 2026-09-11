import pathlib, json, hashlib, shutil, datetime, subprocess, socket
base = pathlib.Path('/home/acfrazier/274bot-campaign/multirevision-20260910')
root = base / 'platform-isolated-e72cfb39'
engine = base / 'server-289'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
m = json.loads((root / 'source-manifest.json').read_text())
b = json.loads((base / 'platform-build-e72cfb39.json').read_text())
j = next(j for j in b['jobs'] if j['name'] == 'platform_proof')
assert b['host_commit'] == m['host_commit'] and b['client_commit'] == m['client_commit']
assert b['isolated_build'] and b['target_started_empty'] and j['exit_code'] == 0
for n, h in m['files'].items():
    assert sha(root / n) == h, n
binary = base / 'platform_proof-linux-e72cfb39'
assert sha(binary) == j['binary_sha256']
(root / 'bin').mkdir()
shutil.copy2(binary, root / 'bin/platform_proof-e72cfb39')
shutil.copy2(base / 'platform-build-e72cfb39.json', root / 'platform-build.json')
assert sha(root / 'bin/platform_proof-e72cfb39') == j['binary_sha256']
em = json.loads((engine / 'fixture-manifest.json').read_text())
for entry in em['files']:
    assert sha(engine / entry['path']) == entry['sha256'], entry['path']
service = subprocess.check_output(['systemctl', 'show', '274bot-concord-test-server.service',
                                  '--property=ActiveState,SubState,User,WorkingDirectory'], text=True)
assert 'ActiveState=inactive' in service
for port in [44594, 1080]:
    s = socket.socket()
    assert s.connect_ex(('127.0.0.1', port)) != 0, port
    s.close()
r = dict(verified_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
         host_commit=b['host_commit'], client_commit=b['client_commit'],
         source_files_verified=len(m['files']), binary_sha256=j['binary_sha256'],
         engine_manifest_sha256=sha(engine / 'fixture-manifest.json'),
         engine_files_verified=len(em['files']), original_service=service, overlays=m.get('overlays', {}))
(root / 'concord-ready.json').write_text(json.dumps(r, indent=2) + '\n')
print(json.dumps(r), flush=True)
