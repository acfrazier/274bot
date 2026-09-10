import hashlib,json,pathlib,tarfile
root=pathlib.Path('/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01')
release=root/'host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-CF1-release-01'
files=sorted([p for base in (release,root/'root-coordinate-cf1-admission') for p in base.rglob('*') if p.is_file()])
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return h.hexdigest()
manifest={'files':[dict(path=str(p.relative_to(root)),bytes=p.stat().st_size,sha256=sha(p)) for p in files]}
assert files and not any(p.is_symlink() for p in files)
arc=root/'root-coordinate-cf1-evidence.tar.gz'
assert not arc.exists()
with tarfile.open(arc,'w:gz',compresslevel=1) as t:
 for p in files:t.add(p,arcname=str(p.relative_to(root)),recursive=False)
manifest.update(archive_sha256=sha(arc),archive_bytes=arc.stat().st_size)
(root/'root-coordinate-cf1-evidence.manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps({k:v for k,v in manifest.items() if k!='files'},indent=2))
for p in sorted(release.rglob('result.json')):
 j=json.loads(p.read_text());print(json.dumps(dict(path=str(p),status=j.get('status'),completed=j.get('completed'),budget=j.get('budget')),indent=2))
