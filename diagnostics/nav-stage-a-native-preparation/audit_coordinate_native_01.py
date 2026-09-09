import hashlib,json,subprocess,tarfile
from pathlib import Path
base=Path(__file__).resolve().parent;archive=base/'native-coordinate-a44a930-concord-01.tar.gz'
def sha(b):return hashlib.sha256(b).hexdigest()
assert sha(archive.read_bytes())=='18e574254751f4307965bb78e91282ec545af701ae5d1d8cc60ecf07d17afe6c'
with tarfile.open(archive) as t:
 def data(n):return t.extractfile(n).read()
 def obj(n):return json.loads(data(n))
 m=obj('concord-payload-manifest.json');names=[e.name for e in t.getmembers()];assert len(names)==len(set(names))==771
 for e in m['files']:assert len(data(e['path']))==e['bytes'] and sha(data(e['path']))==e['sha256']
 tool='host/docs/memory/nav-tiled-stage-a/';run=m['destination_relative_run']+'/'
 q=obj(run+'coordinate-ebf0f30-qualification-01/result.json');progress=obj('builder-provenance/root-qualification-progress.json')
 assert len(progress)==8 and all(e['returncode']==0 for e in progress)
 assert q['qualified'] and q['native_hard_as_qualified'] and q['generated_smoke_executed'] and q['phase']=='GQ'
 for group in q['tools'].values():
  for n,h in group.items():assert sha(data(tool+n))==h and sha(subprocess.check_output(['git','show','a44a930:docs/memory/nav-tiled-stage-a/'+n]))==h
 admissions={};effective={}
 for arm in ['dense','refined']:
  eff=obj(run+arm+'/effective-source-manifest.json');overlays=[x for x in eff['files'] if x['provenance']!='base'];assert len(overlays)==(arm=='refined')
  if overlays:assert overlays[0]['path']=='crates/nav/src/collision.rs' and overlays[0]['git_blob']=='07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7'
  for e in eff['files']:
   b=subprocess.check_output(['git',*(['-C','vendor/fr-client-rust'] if e['path'].startswith('vendor/fr-client-rust/') else []),'cat-file','blob',e['git_blob']]);assert len(b)==e['size'] and sha(b)==e['sha256']
  effective[arm]={'base':eff['base_commit'],'files':len(eff['files']),'overlays':overlays}
  for v in ['clean','counting']:
   label=arm+'-'+v;a=obj(run+label+'-admission.json');assert sha(data(run+label+'-admission.json'))==q['admissions'][label]['sha256']
   assert sha(data(run+arm+'/effective-source-manifest.json'))==a['effective_source_manifest_sha256']
   assert sha(data(run+arm+'/target-'+v+'/release/stage-a-probe'))==a['executable_sha256']
   for n,h in a['source'].items():assert sha(data(run+arm+'/'+n))==h
   admissions[label]=a['executable_sha256']
 comparisons=[]
 for c in q['comparisons']:
  old=data(tool+'run-05/qualification-'+c['variant']+'/'+c['reference_arm']+'-'+c['fixture']+'.out');new=data(run+'qualification-'+c['variant']+'/'+c['arm']+'-'+c['fixture']+'.out')
  assert sha(old)==c['old_sha256'] and sha(new)==c['new_sha256']
  o=json.loads(old.decode().splitlines()[-1]);n=json.loads(new.decode().splitlines()[-1])
  for k in ['aggregate','logical_cells','layout','lookup_count','lookup_checksum','narrow_checksum']:assert o[k]==n[k],k
  assert len(n['raw_elapsed_ns'])==c['raw_calls']==840 and n['aggregate']['calls']==840
  if c['variant']=='counting':assert n['narrow_allocations']==n['narrow_requested_bytes']==0
  comparisons.append({k:c[k] for k in ['variant','arm','fixture','raw_calls']})
 smoke=obj(run+'coordinate-ebf0f30-qualification-01-smoke/GQ/result.json');assert smoke['status']=='complete' and smoke['completed']==smoke['budget']['children']==4 and smoke['phase']=='GQ'
 result={'scope':'root native generated evidence audit only; no real performance acceptance','archive_sha256':sha(archive.read_bytes()),'members_verified':771,'steps_verified':len(progress),'effective_sources':effective,'admitted_binaries':admissions,'raw_comparisons':comparisons,'native_hard_as_qualified':True,'generated_smoke_children':4,'real_children':0}
 (base/'root-coordinate-native-audit-01.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
