"""Root readback: all archive bytes plus independent Git/source/output checks."""
from pathlib import Path
import hashlib,json,subprocess,tarfile
root=Path(__file__).resolve().parent
repo=root.parents[2]
def digest(b):return hashlib.sha256(b).hexdigest()
def git(where,*args):return subprocess.check_output(['git','-C',str(where),*args])
manifest=json.loads((root/'coordinate-native-generated-01-archive.json').read_text())
archive=root/'coordinate-native-generated-01.tar.gz'
assert digest(archive.read_bytes())==manifest['archive_sha256']=='baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524'
expected={e['path']:e for e in manifest['files']};assert len(expected)==len(manifest['files'])
payload={};seen={}
with tarfile.open(archive,'r|gz') as t:
 for m in t:
  p=Path(m.name);assert m.isfile() and not p.is_absolute() and '..' not in p.parts and p.parts[0]=='coordinate-native-generated-01'
  rel=str(Path(*p.parts[1:]));assert rel in expected and rel not in seen
  e=expected[rel];assert m.size==e['bytes'];s=t.extractfile(m);h=hashlib.sha256();chunks=[]
  while b:=s.read(1024**2):
   h.update(b)
   if m.size<16*1024**2:chunks.append(b)
  assert h.hexdigest()==e['sha256'];seen[rel]=h.hexdigest()
  if chunks:payload[rel]=b''.join(chunks)
assert set(seen)==set(expected)
def read(name):return json.loads(payload[name])
r=read('result.json');assert r['qualified'] is True and r['input_count']==12954
assert r['comparison']=={'equal':True,'bytes':328120731}
assert r['generated_protocol_comparison']=={'equal':True,'bytes':80987}
for p,h in r['output_hashes'].items():assert seen[p]==h
assert seen['dense-probe.out']==seen['refined-probe.out']
assert seen['protocol-fixture/dense.out']==seen['protocol-fixture/refined.out']
assert r['source_binding']['sha256']==digest(payload['tools/coordinate-source-binding.json'])=='7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb'
corpus=read('corpus.json');assert len(corpus)==12954
for e in corpus:assert seen[e['path']]==e['sha256'] and expected[e['path']]['bytes']==e['bytes']
for name,h in read('launch.json')['tools'].items():
 assert seen['tools/'+name]==h
 assert payload['tools/'+name]==git(repo,'show','a44a930:docs/memory/nav-tiled-differential/'+name)
suffixes={'crates/nav/Cargo.toml':b'\n[[bin]]\nname="differential-probe"\npath="src/differential.rs"\n','crates/nav/src/router.rs':b'\n#[path="router-access.rs"]\npub mod differential_access;\n'}
source_counts={}
for arm,rev in [('dense','29b7aea779322c8611f83dc193e939ca7d756f75'),('refined','8385babb23fd15b876506d4a3f6154984a6b2df1')]:
 original=read(arm+'/original-source-manifest.json');effective=read(arm+'/effective-source-manifest.json');admission=read(arm+'-admission.json')
 assert original['commit']==effective['base_commit']==admission['base_commit']==rev
 assert admission['effective_source_manifest_sha256']==seen[arm+'/effective-source-manifest.json']
 for p,h in admission['source'].items():assert seen[arm+'/'+p]==h
 orig={e['path']:e for e in original['files']};eff={e['path']:e for e in effective['files']};assert set(orig)==set(eff)
 overlays=[]
 for p,e in orig.items():
  client=p.startswith('vendor/fr-client-rust/');where=repo/'vendor/fr-client-rust' if client else repo
  commit=original['client'] if client else rev;gp=p.removeprefix('vendor/fr-client-rust/') if client else p
  oid=git(where,'rev-parse',commit+':'+gp).decode().strip();assert oid==e['git_blob']
  base=git(where,'cat-file','blob',oid);assert digest(base)==e['sha256'] and len(base)==e['size']
  effective_bytes=base
  if arm=='refined' and p=='crates/nav/src/collision.rs':
   overlays.append(p);effective_bytes=git(repo,'cat-file','blob','07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7')
  ee=eff[p];assert digest(effective_bytes)==ee['sha256'] and len(effective_bytes)==ee['size']
  assert payload[arm+'/'+p]==effective_bytes+suffixes.get(p,b'')
 assert len(overlays)==(1 if arm=='refined' else 0)
 host=payload[arm+'/original-host-lib.rs'];assert host==git(repo,'show',rev+':crates/host-play/src/lib.rs')
 spans=read(arm+'/host-spans.json');assert digest(host)==spans['source_sha256'];pieces=[]
 for s in spans['spans']:
  b=host[s['start_byte']:s['end_byte']];assert digest(b)==s['sha256'];pieces.append(b)
 assert payload[arm+'/crates/nav/src/host-probe.rs']==b'\n\n'.join(pieces)+b'\n'
 source_counts[arm]=len(orig)
progress=json.loads((root/'progress.json').read_text());assert len(progress)==6 and all(p['returncode']==0 for p in progress)
out=dict(verified=True,archive_sha256=manifest['archive_sha256'],archive_files=len(seen),archive_uncompressed_bytes=sum(e['bytes'] for e in expected.values()),input_count=len(corpus),comparison=r['comparison'],protocol_comparison=r['generated_protocol_comparison'],source_counts=source_counts,source_binding=r['source_binding'],scope='native generated correctness only; no private input, CF1 or performance acceptance')
(root/'root-archive-audit.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out,indent=2))
