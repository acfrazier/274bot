"""Preserve completed native generated qualification; never run private inputs."""
from pathlib import Path
import hashlib,json,tarfile
root=Path('/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01')
tool=root/'host/docs/memory/nav-tiled-stage-a'
proof=tool/'coordinate-concord-run-01/coordinate-ebf0f30-qualification-03'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  while b:=f.read(1024**2):h.update(b)
 return h.hexdigest()
progress=json.loads((root/'root-concord-qualification-progress.json').read_text())
assert len(progress)==5 and all(r['returncode']==0 for r in progress)
result=json.loads((proof/'result.json').read_text())
assert result['qualified'] and result['native_hard_as_qualified'] and result['generated_smoke_executed']
assert len(result['comparisons'])==12
paths=[root/'root-concord-qualification-progress.json',root/'root-concord-staging-verification.json',root/'qualify_native_singleton_concord.py',root/'concord-payload-manifest.json']
paths += [root/(r['name']+'.log') for r in progress]
paths += [p for p in tool.rglob('*') if p.is_file()]
assert len(paths)==len(set(paths)) and all(not p.is_symlink() for p in paths)
name='coordinate-concord-qualified-03'
entries=[dict(path=str(p.relative_to(root)),bytes=p.stat().st_size,sha256=sha(p)) for p in sorted(paths)]
manifest=dict(scope='native generated qualification only; includes immutable relocated builder/reference lineage; no private input or performance acceptance',files=entries)
(root/(name+'.manifest.json')).write_text(json.dumps(manifest,indent=2)+'\n')
with tarfile.open(root/(name+'.tar.gz'),'x:gz') as t:
 for e in entries:t.add(root/e['path'],arcname=e['path'],recursive=False)
archive=root/(name+'.tar.gz')
receipt=dict(archive=archive.name,sha256=sha(archive),bytes=archive.stat().st_size,files=len(entries),uncompressed_bytes=sum(e['bytes'] for e in entries),qualification=result)
(root/(name+'.receipt.json')).write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({k:v for k,v in receipt.items() if k!='qualification'},indent=2))
