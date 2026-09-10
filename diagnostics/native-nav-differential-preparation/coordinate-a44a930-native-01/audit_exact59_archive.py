from pathlib import Path
import hashlib,json,tarfile
root=Path(__file__).resolve().parent
r=json.loads((root/'exact59-evidence.receipt.json').read_text());m=json.loads((root/'exact59-evidence.manifest.json').read_text());archive=root/r['archive']
def digest(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  while b:=f.read(1024**2):h.update(b)
 return h.hexdigest()
assert digest(archive)==r['sha256']=='27e5f0a8d305a2dcbf4f654e7e42a1630b9af1f3fd5ffb3c141b42bdba59ccce'
entries={e['path']:e for e in m['files']};seen=set();small={};total=0
with tarfile.open(archive,mode='r|gz') as t:
 for x in t:
  assert x.isfile() and x.name in entries and x.name not in seen;seen.add(x.name)
  h=hashlib.sha256();n=0;parts=[]
  with t.extractfile(x) as f:
   while b:=f.read(1024**2):
    h.update(b);n+=len(b)
    if x.size<1024**2:parts.append(b)
  assert n==entries[x.name]['bytes'] and h.hexdigest()==entries[x.name]['sha256'];total+=n
  if parts:small[x.name]=b''.join(parts)
assert seen==set(entries) and total==r['uncompressed_bytes']
audit=json.loads(small['exact59-root-audit.json']);assert audit['verified'] and audit['comparison']==dict(bytes=2119776634,equal=True)
prefix='host/docs/memory/nav-tiled-differential/coordinate-native-generated-01/real-release/'
for arm in ('dense','refined'):
 assert entries[prefix+arm+'.out']['sha256']==audit['output_sha256'][arm]=='bbb179b7bedf32e8811db730b6fe69e069c67d9f80aca62bf14a0c577e7dd022'
assert audit['arms']['dense']==audit['arms']['refined'] and len(audit['arms']['dense']['rows'])==59
assert hashlib.sha256(small['exact59-authorization.json']).hexdigest()==audit['authorization_sha256']=='f8ed2150e9d2c0eda91fdf525165a09a94011c52f1d3504c75821cd0256b038d'
assert entries[prefix+'input.bin']['sha256']==audit['input_sha256'] and entries[prefix+'routes.tsv']['sha256']==audit['routes_sha256']
out=dict(verified=True,archive_sha256=r['sha256'],files=len(seen),uncompressed_bytes=total,full_output_bytes_per_arm=2119776634,full_output_sha256=audit['output_sha256']['dense'],rows=59,scope='independent complete archived bytes readback; correctness only')
(root/'exact59-root-archive-audit.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out,indent=2))
