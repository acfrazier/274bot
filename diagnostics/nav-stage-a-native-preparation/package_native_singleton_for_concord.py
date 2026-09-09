"""Package qualified Linux output; this grants no native measurement release."""
import argparse, hashlib, json, tarfile
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument('--root', type=Path, required=True)
p.add_argument('--run-name', required=True)
p.add_argument('--qualification-name', required=True)
p.add_argument('--destination-run-name', required=True)
p.add_argument('--archive-name', required=True)
p.add_argument('--source-binding-sha256')
a=p.parse_args()
for name in (a.run_name,a.qualification_name,a.destination_run_name,a.archive_name):
    if Path(name).name != name or name in ('.','..'):
        raise ValueError('single path component required')
root=a.root.resolve(); tool=root/'host/docs/memory/nav-tiled-stage-a'; run=tool/a.run_name

def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as stream:
        for data in iter(lambda:stream.read(1024*1024),b''):h.update(data)
    return h.hexdigest()

def read(path): return json.loads(path.read_text())

payload=read(root/'payload-manifest.json')
coordinate='candidate_id' in payload
if coordinate:
    assert a.source_binding_sha256==payload['source_binding_sha256']
    assert a.run_name.startswith('coordinate-native-build-')
    assert a.qualification_name.startswith('coordinate-ebf0f30-qualification-')
    assert a.destination_run_name.startswith('coordinate-concord-run-')
    assert a.archive_name.startswith('native-coordinate-')
else:assert a.source_binding_sha256 is None
for entry in payload['files']:
    rel=Path(entry['path'])
    if rel.is_absolute() or '..' in rel.parts:raise ValueError('unsafe source manifest path')
    f=root/rel
    assert f.is_file() and not f.is_symlink()
    assert f.stat().st_size==entry['bytes'] and sha(f)==entry['sha256']
for variant in ('clean','counting'):
    q=read(run/f'qualification-{variant}/result.json')
    assert q['qualified'] is True and q['platform'].startswith('Linux')
    if coordinate:assert (q['schema'],q['candidate_id'],q['source_binding']['sha256'],q['phase'])==(
        'stage-a-coordinate-generated-result-v1',payload['candidate_id'],a.source_binding_sha256,'tooling-qualification')
q=read(run/a.qualification_name/'result.json')
assert q['qualified'] is True and q['native_hard_as_qualified'] is True
assert q['platform'].startswith('Linux')
if coordinate:assert (q['schema'],q['candidate_id'],q['source_binding']['sha256'],q['phase'])==(
    'stage-a-coordinate-generated-v1',payload['candidate_id'],a.source_binding_sha256,'GQ')
for family in ('stage','scheduler'):
    for name,digest in q['tools'][family].items():assert sha(tool/name)==digest
progress=read(root/'root-qualification-progress.json')
assert len(progress)>=8 and all(x['returncode']==0 for x in progress)
files={}
def add(path,name):
    assert path.is_file() and not path.is_symlink()
    assert name not in files
    files[name]=path
# Original source payload includes frozen Git packs and historical readbacks.
for entry in payload['files']:add(root/entry['path'],entry['path'])
prefix='host/docs/memory/nav-tiled-stage-a/'
for f in sorted(run.rglob('*')):
    if f.is_symlink():raise ValueError('symlink in qualified run')
    if not f.is_file():continue
    rel=f.relative_to(run); parts=rel.parts
    if len(parts)>1 and parts[0] in ('dense','tiled','refined') and parts[1].startswith('target-'):
        if Path(*parts[2:])!=Path('release/stage-a-probe'):continue
    add(f,prefix+a.destination_run_name+'/'+str(rel))
for name in ('payload-manifest.json','root-staging-verification.json','root-qualification-progress.json'):
    add(root/name,'builder-provenance/'+name)
for name in ('root-reuse-verification.json','reused-source-progress.json'):
    if (root/name).exists():add(root/name,'builder-provenance/'+name)
for entry in progress:
    log=root/(entry['name']+'.log')
    add(log,'builder-provenance/'+log.name)
guards=tool/('coordinate-native-guards-01' if coordinate else 'native-guards-01')
g=read(guards/'result.json')
assert g['qualified'] is True and g['native_hard_as_qualified'] is True
if coordinate:assert (g['schema'],g['candidate_id'],g['source_binding']['sha256'],g['phase'])==(
    'stage-a-coordinate-guard-result-v1',payload['candidate_id'],a.source_binding_sha256,'tooling-guards')
for f in sorted(guards.iterdir()):
    if f.is_file():add(f,'builder-provenance/'+guards.name+'/'+f.name)
manifest=dict(scope='same admitted Linux binaries relocated without rebuilding; fresh Concord generated qualification and root release remain required',tool_commit=payload['tool_commit'],source_run=str(run),destination_relative_run=prefix+a.destination_run_name,files=[dict(path=n,bytes=f.stat().st_size,sha256=sha(f)) for n,f in files.items()])
if coordinate:manifest.update(candidate_id=payload['candidate_id'],source_binding_sha256=a.source_binding_sha256)
mp=root/(a.archive_name+'.manifest.json')
with mp.open('x') as out:json.dump(manifest,out,indent=2);out.write('\n')
add(mp,'concord-payload-manifest.json')
archive=root/a.archive_name
with tarfile.open(archive,'x:gz') as tar:
    for name,f in files.items():tar.add(f,arcname=name,recursive=False)
receipt=dict(archive=archive.name,files=len(files),bytes=archive.stat().st_size,sha256=sha(archive),uncompressed_bytes=sum(f.stat().st_size for f in files.values()))
with (root/(a.archive_name+'.receipt.json')).open('x') as out:json.dump(receipt,out,indent=2);out.write('\n')
print(json.dumps(receipt,indent=2))
