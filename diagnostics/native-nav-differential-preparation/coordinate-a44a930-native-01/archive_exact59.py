"""Archive the completed exact59 proof without changing its source files."""
from pathlib import Path
import gzip,hashlib,json,tarfile
root=Path(__file__).resolve().parent;real=root/'host/docs/memory/nav-tiled-differential/coordinate-native-generated-01/real-release'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  while b:=f.read(1024**2):h.update(b)
 return h.hexdigest()
assert json.loads((root/'exact59-root-audit.json').read_text())['verified']
paths=sorted(p for p in real.iterdir() if p.is_file())
paths += [root/n for n in ('exact59-root-audit.json','exact59-authorization.json','exact59-preflight.json','prepare_exact59_release.py','audit_exact59.py')]
assert len(paths)==len(set(paths)) and all(not p.is_symlink() for p in paths)
entries=[dict(path=str(p.relative_to(root)),bytes=p.stat().st_size,sha256=sha(p)) for p in paths]
manifest=dict(scope='exact59 correctness raw evidence only; no performance acceptance',files=entries)
(root/'exact59-evidence.manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
archive=root/'exact59-evidence.tar.gz'
with archive.open('xb') as raw,gzip.GzipFile(fileobj=raw,mode='wb',compresslevel=1) as gz,tarfile.open(fileobj=gz,mode='w') as t:
 for e in entries:t.add(root/e['path'],arcname=e['path'],recursive=False)
receipt=dict(archive=archive.name,sha256=sha(archive),bytes=archive.stat().st_size,files=len(entries),uncompressed_bytes=sum(e['bytes'] for e in entries))
(root/'exact59-evidence.receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
