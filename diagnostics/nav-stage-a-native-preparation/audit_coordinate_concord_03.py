"""Independent byte/readback audit; no imports from measured implementation."""
import hashlib,json,pathlib,subprocess,tarfile
base=pathlib.Path(__file__).resolve().parent/'scheduler-fixture-correction-native-01'
name='coordinate-concord-qualified-03'
r=json.loads((base/(name+'.receipt.json')).read_text());m=json.loads((base/(name+'.manifest.json')).read_text())
sha=lambda b:hashlib.sha256(b).hexdigest()
assert sha((base/(name+'.tar.gz')).read_bytes())==r['sha256']=='41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681'
entries={e['path']:e for e in m['files']};assert len(entries)==len(m['files'])==r['files']
data={}
with tarfile.open(base/(name+'.tar.gz')) as t:
 for x in t:
  assert x.isfile() and x.name in entries and x.name not in data
  b=t.extractfile(x).read();e=entries[x.name];assert len(b)==e['bytes'] and sha(b)==e['sha256'];data[x.name]=b
assert set(data)==set(entries)
root='/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01/'
tool='host/docs/memory/nav-tiled-stage-a/';run=tool+'coordinate-concord-run-01/'
proof=run+'coordinate-ebf0f30-qualification-03/'
def obj(p):return json.loads(data[p])
def ref(x):
 assert x['path'].startswith(root);p=x['path'][len(root):];assert sha(data[p])==x['sha256'];return p
q=obj(proof+'result.json');assert q==r['qualification'] and q['qualified'] and q['native_hard_as_qualified'] and q['generated_smoke_executed']
assert q['candidate_id']=='coordinate-ebf0f30' and q['phase']=='GQ'
assert q['source_binding']['sha256']==sha(data[tool+'coordinate-source-binding.json'])=='7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb'
for group in q['tools'].values():
 for n,h in group.items():
  b=data[tool+n];assert sha(b)==h
  original=subprocess.check_output(['git','show','113ce05:docs/memory/nav-tiled-stage-a/'+n]);assert b==original
for x in q['guards'].values():ref(x)
g=obj(proof+'scheduler-guards.receipt.json');assert g['returncode']==0 and g['failure'] is None and g['address_guard_active']
assert g['limits']==dict(address=4294967296,cpu=300,file_size=1048576,output=1048576,rss=536870912,wall=360)
assert b'Ran 29 tests' in data[proof+'scheduler-guards.err'] and b'\nOK\n' in data[proof+'scheduler-guards.err'] and b'skipped' not in data[proof+'scheduler-guards.err']
smoke_path=ref(q['smoke']);smoke=obj(smoke_path);assert smoke['completed']==smoke['budget']['children']==4 and smoke['status']=='complete'
assert [x['name'] for x in smoke['entries']]==['000-1-1-dense','001-1-1-refined','002-1-2-refined','003-1-2-dense']
for x in smoke['entries']:
 p=str(pathlib.PurePosixPath(smoke_path).parent/(x['name']+'.record.json'));assert sha(data[p])==x['record_sha256']
for x in q['admissions'].values():ref(x)
original=json.loads((base/'package-manifest.json').read_text());original_files={e['path']:e for e in original['files']}
for key,x in q['admissions'].items():
 p=ref(x);assert sha(data[p])==original_files[p]['sha256']
 a=obj(p);arm,variant=key.split('-');assert a['source_binding']==q['source_binding']
 # Native archive retains the full source tree and admitted executable.
 for rel,h in a['source'].items():assert sha(data[run+arm+'/'+rel])==h
 candidates=[p for p in data if p.startswith(run+arm+'/') and p.endswith('/stage-a-probe') and 'target-'+variant+'/' in p]
 assert len(candidates)==1 and sha(data[candidates[0]])==a['executable_sha256']
for c in q['comparisons']:
 new=run+'qualification-'+c['variant']+'/'+c['arm']+'-'+c['fixture']+'.out'
 old=tool+'run-05/qualification-'+c['variant']+'/'+c['reference_arm']+'-'+c['fixture']+'.out'
 assert sha(data[new])==c['new_sha256'] and sha(data[old])==c['old_sha256']
 n=json.loads(data[new].splitlines()[-1]);o=json.loads(data[old].splitlines()[-1])
 for key in ('aggregate','logical_cells','lookup_checksum','narrow_checksum','layout','narrow_allocations','narrow_requested_bytes'):assert n[key]==o[key]
 assert len(n['raw_elapsed_ns'])==840==c['raw_calls'] and n['narrow_allocations']==n['narrow_requested_bytes']==0
progress=obj('root-concord-qualification-progress.json');assert len(progress)==5 and all(x['returncode']==0 for x in progress)
out=dict(verified=True,scope='native generated functional qualification only; no real input or performance acceptance',archive_sha256=r['sha256'],members_verified=len(data),reviewed_tool_commit='113ce05525cd2bd27128bd3f420cafaad76c529c',source_binding_sha256=q['source_binding']['sha256'],four_admissions_unchanged=True,all_admitted_source_and_executable_hashes_verified=True,guards=g,comparisons=12,smoke_children=4,progress=progress)
(base/'root-coordinate-concord-qualified-03-audit.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps({k:v for k,v in out.items() if k not in ('guards','progress')},indent=2))
