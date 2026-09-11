import datetime,hashlib,json,os,pathlib,shutil,subprocess,sys,time
root=pathlib.Path(sys.argv[1]).resolve();target=pathlib.Path(sys.argv[2]);win=os.name=='nt'
assert not target.exists(), 'Build target must start empty'
if win:
 nasm=root.parent/'tools/nasm-3.02';assert (nasm/'nasm.exe').exists();os.environ['PATH']=str(nasm)+os.pathsep+os.environ.get('PATH','')
manifest=json.loads((root/'source-manifest.json').read_text())
assert json.loads((root/'verified-source.json').read_text())['host_commit']==manifest['host_commit']
for name,h in manifest['files'].items():assert hashlib.sha256((root/name).read_bytes()).hexdigest()==h,name
cargo=pathlib.Path.home()/'.cargo/bin'/('cargo.exe' if win else 'cargo')
env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(target),CARGO_BUILD_JOBS='4' if win else '2',GIT_COMMIT=manifest['host_commit'],GIT_DIRTY='0');env['PATH']=str(cargo.parent)+os.pathsep+env.get('PATH','')
jobs=[('catalog_boundary_live',['test','-p','host-play','--test','catalog_boundary_live','--features','memory-profile','--no-run','--message-format=json'])]
jobs += [('catalog_watch',['build','-p','panel','--example','catalog_watch','--message-format=json'])] if win else [('platform_proof',['build','-p','tui','--example','platform_proof','--features','memory-profile','--message-format=json'])]
(root/'bin').mkdir(exist_ok=True);results=[]
for name,args in jobs:
 prefix=root/('build-'+name);cmd=[str(cargo)]+args;start=datetime.datetime.now(datetime.timezone.utc).isoformat();before=time.monotonic()
 with prefix.with_suffix('.jsonl').open('w') as out,prefix.with_suffix('.log').open('w') as err:
  p=subprocess.Popen(cmd,cwd=root/'source',env=env,stdout=out,stderr=err);print(json.dumps(dict(phase='build-start',name=name,pid=p.pid,command=cmd)),flush=True);code=p.wait()
 row=dict(name=name,command=cmd,started_at=start,elapsed_seconds=round(time.monotonic()-before,3),exit_code=code)
 if code==0:
  messages=[json.loads(x) for x in prefix.with_suffix('.jsonl').read_text().splitlines()]
  candidates=[x['executable'] for x in messages if x.get('reason')=='compiler-artifact' and x.get('target',{}).get('name')==name and x.get('executable')]
  assert len(candidates)==1,(name,candidates)
  artifact=root/'bin'/(name+'-'+manifest['host_commit'][:8]+('.exe' if win else ''));shutil.copy2(candidates[0],artifact)
  row.update(binary=str(artifact),binary_sha256=hashlib.sha256(artifact.read_bytes()).hexdigest(),binary_bytes=artifact.stat().st_size)
 results.append(row)
 for source,h in manifest['files'].items():assert hashlib.sha256((root/source).read_bytes()).hexdigest()==h,source
 receipt=dict(overlays=manifest.get('overlays',{}),host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],platform=sys.platform,source_files_verified=len(manifest['files']),target_dir=str(target),isolated_build=True,target_started_empty=True,jobs=results,elapsed_is_not_performance_measurement=True)
 (root/'platform-build.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(row),flush=True)
 if code:sys.exit(code)
