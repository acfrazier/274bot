"""Archive observed failed qualification and its retained generated fixtures."""
from pathlib import Path
import hashlib,json,tarfile
root=Path('/home/acfrazier/274bot-campaign/nav-coordinate-a44a930-concord-01');tool=root/'host/docs/memory/nav-tiled-stage-a';run=tool/'coordinate-concord-run-01';proof=run/'coordinate-ebf0f30-qualification-02'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  while b:=f.read(1024**2):h.update(b)
 return h.hexdigest()
progress=json.loads((root/'root-concord-continuation-qualification-progress.json').read_text());assert len(progress)==5 and [r['returncode'] for r in progress]==[0,0,0,0,1]
receipt=json.loads((proof/'scheduler-guards.receipt.json').read_text());assert receipt['returncode']==-9 and receipt['limits']['cpu']==300
paths=[root/'root-concord-continuation-qualification-progress.json',root/'continue_coordinate_concord_01.py']
paths += [root/(r['name']+'-continuation.log') for r in progress]
for d in [proof,run/'qualification-clean',run/'qualification-counting',run/'integration-tests',tool/'coordinate-concord-native-guards-02']:
 assert d.is_dir(),d
 paths.extend(p for p in d.rglob('*') if p.is_file())
for line in (proof/'scheduler-guards.out').read_text().splitlines():
 assert line.startswith('retained fixture: ');d=Path(line.removeprefix('retained fixture: '));assert d.parent==tool and d.name.startswith('sharded-test-') and d.is_dir()
 paths.extend(p for p in d.rglob('*') if p.is_file())
assert len(paths)==len(set(paths)) and all(not p.is_symlink() for p in paths)
name='coordinate-concord-scheduler-failure-01'
entries=[dict(path=str(p.relative_to(root)),bytes=p.stat().st_size,sha256=sha(p)) for p in sorted(paths)]
manifest=dict(scope='failed generated scheduler qualification; previous generated clean/counting/integration retained; no private input',files=entries)
(root/(name+'.manifest.json')).write_text(json.dumps(manifest,indent=2)+'\n')
with tarfile.open(root/(name+'.tar.gz'),'x:gz') as t:
 for e in entries:t.add(root/e['path'],arcname=e['path'],recursive=False)
archive=root/(name+'.tar.gz');result=dict(archive=archive.name,sha256=sha(archive),bytes=archive.stat().st_size,files=len(entries),uncompressed_bytes=sum(e['bytes'] for e in entries),scheduler_receipt=receipt)
(root/(name+'.receipt.json')).write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
