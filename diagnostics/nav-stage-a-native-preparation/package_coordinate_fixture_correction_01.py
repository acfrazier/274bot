"""New immutable relocation package: only reviewed scheduler fixture bytes change."""
from pathlib import Path
import hashlib,io,json,subprocess,tarfile
repo=Path(__file__).resolve().parents[2];root=Path(__file__).resolve().parent;out=root/'scheduler-fixture-correction-native-01'
base=root/'native-coordinate-a44a930-concord-01.tar.gz';expected='18e574254751f4307965bb78e91282ec545af701ae5d1d8cc60ecf07d17afe6c'
def sha(b):return hashlib.sha256(b).hexdigest()
def blob(ref,path):return subprocess.check_output(['git','-C',str(repo),'show',ref+':'+path])
assert sha(base.read_bytes())==expected
with tarfile.open(base) as t:
 ms=t.getmembers();assert all(m.isfile() for m in ms) and len(ms)==len({m.name for m in ms})
 data={m.name:t.extractfile(m).read() for m in ms};info={m.name:m for m in ms}
manifest=json.loads(data['concord-payload-manifest.json'])
for e in manifest['files']:assert len(data[e['path']])==e['bytes'] and sha(data[e['path']])==e['sha256']
old_commit=manifest['tool_commit'];new_commit=subprocess.check_output(['git','-C',str(repo),'rev-parse','113ce05']).decode().strip()
prefix='host/docs/memory/nav-tiled-stage-a/';path=prefix+'test_sharded.py';old=data[path];new=blob(new_commit,path.removeprefix('host/'));assert old!=new and old==blob(old_commit,path.removeprefix('host/'))
changed=[];checked=[]
for n,b in data.items():
 if n.startswith(prefix) and '/' not in n.removeprefix(prefix):
  current=blob(new_commit,n.removeprefix('host/'));checked.append(n)
  if current!=b:changed.append(n)
assert changed==[path],changed
review=(out/'review.json').read_bytes();rv=json.loads(review);assert rv['completion']['metadata']['review_outcome']=='approved' and rv['completion']['metadata']['reviewed_commit']=='113ce05'
record=dict(schema='root-scheduler-fixture-relocation-v1',base_archive_sha256=expected,base_tool_commit=old_commit,tool_commit=new_commit,review_sha256=sha(review),changed_path=path,old_sha256=sha(old),new_sha256=sha(new),unchanged_top_level_tools=sorted(set(checked)-{path}),scope='same four immutable stage admissions and binaries; only generated scheduler fixture changes; fresh native qualification required; no private release')
data['fixture-correction/original-concord-payload-manifest.json']=data['concord-payload-manifest.json'];data['fixture-correction/original-test_sharded.py']=old;data['fixture-correction/review.json']=review;data['fixture-correction/record.json']=(json.dumps(record,indent=2)+'\n').encode();data[path]=new
manifest.update(tool_commit=new_commit,scheduler_fixture_correction=dict(record_path='fixture-correction/record.json',record_sha256=sha(data['fixture-correction/record.json'])),scope=record['scope'])
manifest['files']=[dict(path=n,bytes=len(b),sha256=sha(b)) for n,b in sorted(data.items()) if n!='concord-payload-manifest.json']
data['concord-payload-manifest.json']=(json.dumps(manifest,indent=2)+'\n').encode()
archive=out/'native-coordinate-113ce05-concord-01.tar.gz'
with tarfile.open(archive,'x:gz') as t:
 for n,b in sorted(data.items()):
  m=info.get(n) or tarfile.TarInfo(n);m.size=len(b)
  if n not in info:m.mode=0o644
  t.addfile(m,io.BytesIO(b))
with tarfile.open(archive) as t:
 ms=t.getmembers();assert len(ms)==len(data) and {m.name for m in ms}==set(data)
 for m in ms:assert t.extractfile(m).read()==data[m.name]
receipt=dict(verified=True,archive=archive.name,sha256=sha(archive.read_bytes()),bytes=archive.stat().st_size,files=len(data),uncompressed_bytes=sum(map(len,data.values())),fixture_correction=record)
(out/'package-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');(out/'package-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');print(json.dumps(receipt,indent=2))
