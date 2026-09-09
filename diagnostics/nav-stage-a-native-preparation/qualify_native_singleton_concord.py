"""Relocate admitted binaries and run fresh generated-only Concord qualification."""
import argparse,hashlib,json,pathlib,subprocess,sys,tarfile,time
p=argparse.ArgumentParser();p.add_argument('--root',type=pathlib.Path,required=True);p.add_argument('--archive',required=True);p.add_argument('--sha256',required=True);p.add_argument('--qualification-name');p.add_argument('--source-binding-sha256');a=p.parse_args()
root=a.root.resolve();archive=root/a.archive
assert hashlib.sha256(archive.read_bytes()).hexdigest()==a.sha256
assert not (root/'host').exists()
with tarfile.open(archive) as t:
 members=t.getmembers();names=[m.name for m in members];assert len(names)==len(set(names))
 assert all(m.isfile() and not pathlib.PurePosixPath(m.name).is_absolute() and '..' not in pathlib.PurePosixPath(m.name).parts for m in members)
 t.extractall(root,filter='data')
manifest=json.loads((root/'concord-payload-manifest.json').read_text())
coordinate='candidate_id' in manifest
if coordinate:assert a.source_binding_sha256==manifest['source_binding_sha256']
else:assert a.source_binding_sha256 is None
qualification=a.qualification_name or ('coordinate-ebf0f30-qualification-04' if coordinate else 'singleton-qualification-04')
prefix='coordinate-ebf0f30-qualification-' if coordinate else 'singleton-qualification-'
assert qualification.startswith(prefix) and qualification.removeprefix(prefix).isdigit()
for e in manifest['files']:
 f=root/e['path'];assert f.stat().st_size==e['bytes'] and hashlib.sha256(f.read_bytes()).hexdigest()==e['sha256']
host=root/'host';client=host/'vendor/fr-client-rust';client.mkdir(parents=True)
host_packs=('host.pack','qualifier-ancestor.pack')+(('coordinate-overlay.pack',) if coordinate else ())
for repo,packs in ((host,host_packs),(client,('client.pack',))):
 subprocess.run(['git','init',str(repo)],check=True,capture_output=True)
 for name in packs:
  with (root/'git-objects'/name).open('rb') as data:subprocess.run(['git','-C',str(repo),'index-pack','--stdin'],stdin=data,check=True,capture_output=True)
tool=host/'docs/memory/nav-tiled-stage-a';run=root/manifest['destination_relative_run']
sys.path.insert(0,str(tool));import stage_a as stage
binding_args=(['--source-binding',str(tool/'coordinate-source-binding.json'),'--source-binding-sha256',a.source_binding_sha256] if coordinate else [])
if coordinate:stage.configure_source_binding(tool/'coordinate-source-binding.json',a.source_binding_sha256)
for arm in stage.ARMS:
 for variant in ('clean','counting'):stage.verify_arm(run,arm,variant)
(root/'root-concord-staging-verification.json').write_text(json.dumps(dict(verified=True,tool_commit=manifest['tool_commit'],files=len(manifest['files']),all_four_admissions_verified=True,scope='generated qualification only; no real release'),indent=2)+'\n')
# Preserve builder evidence without rewriting any of its qualification files.
# Fresh qualifier paths are selected explicitly for this relocated run.
release_name='coordinate-ebf0f30-standin-release' if coordinate else 'standin-release'
for name in ('qualification-clean','qualification-counting','integration-tests',release_name):
 old=run/name
 assert old.is_dir()
 old.rename(run/('builder-'+name))
py=sys.executable;guard_name='coordinate-concord-native-guards-01' if coordinate else 'concord-native-guards-01'
commands=[('guards',[py,'stage_a.py','guards','--run',str(tool/guard_name),*binding_args]),('generated-clean',[py,'stage_a.py','generated','--run',str(run),'--variant','clean',*binding_args]),('generated-counting',[py,'stage_a.py','generated','--run',str(run),'--variant','counting',*binding_args]),('integration',[py,'test_stage_a.py','--run',str(run),*binding_args]),('scheduler',[py,'qualify_sharded.py',str(run),qualification,*binding_args])]
progress=[]
for name,cmd in commands:
 start=time.monotonic();print('START',name,flush=True)
 with (root/(name+'.log')).open('x') as log:result=subprocess.run(cmd,cwd=tool,stdout=log,stderr=subprocess.STDOUT)
 progress.append(dict(name=name,command=cmd,returncode=result.returncode,wall_seconds=time.monotonic()-start))
 (root/'root-concord-qualification-progress.json').write_text(json.dumps(progress,indent=2)+'\n')
 print(json.dumps(progress[-1]),flush=True)
 if result.returncode:raise SystemExit(result.returncode)
print('Concord generated qualification finished; no real release.',flush=True)
