"""Stage exact reviewed source and run generated Linux qualification only."""
import argparse,hashlib,json,os,pathlib,shutil,subprocess,sys,tarfile,time
p=argparse.ArgumentParser();p.add_argument('--root',type=pathlib.Path,required=True);p.add_argument('--archive',required=True);p.add_argument('--sha256',required=True)
p.add_argument('--reuse-root',type=pathlib.Path)
p.add_argument('--qualification-name');p.add_argument('--source-binding-sha256');a=p.parse_args()
root=a.root.resolve();archive=root/a.archive
assert hashlib.sha256(archive.read_bytes()).hexdigest()==a.sha256
assert not (root/'host').exists()
with tarfile.open(archive) as t:
 ms=t.getmembers();names=[m.name for m in ms];assert len(names)==len(set(names))
 assert all(m.isfile() and not pathlib.PurePosixPath(m.name).is_absolute() and '..' not in pathlib.PurePosixPath(m.name).parts for m in ms)
 t.extractall(root,filter='data')
m=json.loads((root/'payload-manifest.json').read_text())
coordinate='candidate_id' in m
if coordinate:
 assert a.source_binding_sha256==m['source_binding_sha256']
 assert a.reuse_root is None
else:assert a.source_binding_sha256 is None
for e in m['files']:
 f=root/e['path'];assert f.stat().st_size==e['bytes'] and hashlib.sha256(f.read_bytes()).hexdigest()==e['sha256']
host=root/'host';client=host/'vendor/fr-client-rust';client.mkdir(parents=True)
host_packs=('host.pack','qualifier-ancestor.pack')+(('coordinate-overlay.pack',) if coordinate else ())
for repo,packnames in ((host,host_packs),(client,('client.pack',))):
 subprocess.run(['git','init',str(repo)],check=True,capture_output=True)
 for pn in packnames:
  with (root/'git-objects'/pn).open('rb') as data:
   subprocess.run(['git','-C',str(repo),'index-pack','--stdin'],stdin=data,check=True,capture_output=True)
(root/'root-staging-verification.json').write_text(json.dumps(dict(verified=True,tool_commit=m['tool_commit'],files=len(m['files']),archive_sha256=a.sha256,scope='generated Linux qualification only'),indent=2)+'\n')
tool=host/'docs/memory/nav-tiled-stage-a';run=tool/('coordinate-native-build-01' if coordinate else 'native-build-01');py=sys.executable
binding_args=(['--source-binding',str(tool/'coordinate-source-binding.json'),'--source-binding-sha256',a.source_binding_sha256] if coordinate else [])
guard_name='coordinate-native-guards-01' if coordinate else 'native-guards-01'
qualification=a.qualification_name or ('coordinate-ebf0f30-qualification-01' if coordinate else 'singleton-qualification-01')
commands=[('guards',[py,'stage_a.py','guards','--run',str(tool/guard_name),*binding_args]),('prepare',[py,'stage_a.py','prepare','--run',str(run),*binding_args]),('build-clean',[py,'stage_a.py','build','--run',str(run),'--variant','clean',*binding_args]),('build-counting',[py,'stage_a.py','build','--run',str(run),'--variant','counting',*binding_args]),('generated-clean',[py,'stage_a.py','generated','--run',str(run),'--variant','clean',*binding_args]),('generated-counting',[py,'stage_a.py','generated','--run',str(run),'--variant','counting',*binding_args]),('integration',[py,'test_stage_a.py','--run',str(run),*binding_args]),('scheduler',[py,'qualify_sharded.py',str(run),qualification,*binding_args])]
env=dict(os.environ);env['PATH']='/home/builder/.cargo/bin:'+env.get('PATH','');progress=[]
if a.reuse_root:
 old=a.reuse_root.resolve();oldtool=old/'host/docs/memory/nav-tiled-stage-a';oldrun=oldtool/('coordinate-native-build-01' if coordinate else 'native-build-01')
 original_progress=json.loads((old/'root-qualification-progress.json').read_text())
 assert [e['name'] for e in original_progress[:7]]==[n for n,_ in commands[:7]]
 assert all(e['returncode']==0 for e in original_progress[:7])
 sys.path.insert(0,str(tool));import stage_a as stage
 if coordinate:stage.configure_source_binding(tool/'coordinate-source-binding.json',a.source_binding_sha256)
 for name,digest in stage.tools().items():assert stage.sha(oldtool/name)==digest
 copied=[]
 for source in sorted(oldrun.rglob('*')):
  rel=source.relative_to(oldrun);parts=rel.parts
  if len(parts)>1 and parts[0] in ('dense','tiled') and parts[1].startswith('target-'):
   if pathlib.Path(*parts[2:])!=pathlib.Path('release/stage-a-probe'):continue
  if source.is_symlink():raise ValueError('symlink in reused run')
  if not source.is_file():continue
  dest=run/rel;dest.parent.mkdir(parents=True,exist_ok=True)
  digest=stage.sha(source);shutil.copy2(source,dest);assert stage.sha(dest)==digest and stage.sha(source)==digest
  copied.append(dict(path=str(rel),sha256=digest,bytes=dest.stat().st_size))
 for arm in stage.ARMS:
  for variant in ('clean','counting'):stage.verify_arm(run,arm,variant)
 shutil.copytree(oldtool/guard_name,tool/guard_name)
 for entry in original_progress[:7]:
  name=entry['name'];shutil.copy2(old/(name+'.log'),root/(name+'.log'))
  progress.append(dict(entry,reused_from=str(old),reuse_note='same compiled inputs and admitted binaries; original completed result, not a fresh execution'))
 (root/'reused-source-progress.json').write_text(json.dumps(original_progress,indent=2)+'\n')
 (root/'root-reuse-verification.json').write_text(json.dumps(dict(verified=True,source_root=str(old),stage_tools=stage.tools(),files=copied,all_four_admissions_verified=True),indent=2)+'\n')
 (root/'root-qualification-progress.json').write_text(json.dumps(progress,indent=2)+'\n')
 commands=[('scheduler',[py,'qualify_sharded.py',str(run),a.qualification_name or ('coordinate-ebf0f30-qualification-02' if coordinate else 'singleton-qualification-02'),*binding_args])]
for name,cmd in commands:
 start=time.monotonic();print('START',name,flush=True)
 with (root/(name+'.log')).open('x') as log:
  result=subprocess.run(cmd,cwd=tool,env=env,stdout=log,stderr=subprocess.STDOUT)
 progress.append(dict(name=name,command=cmd,returncode=result.returncode,wall_seconds=time.monotonic()-start))
 (root/'root-qualification-progress.json').write_text(json.dumps(progress,indent=2)+'\n')
 print(json.dumps(progress[-1]),flush=True)
 if result.returncode:raise SystemExit(result.returncode)
print('Generated qualification finished; no real release.',flush=True)
