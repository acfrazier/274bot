import datetime,hashlib,json,os,pathlib,shutil,subprocess,sys,time
root=pathlib.Path(__file__).resolve().parents[2];short=sys.argv[1];folder=root/'docs/compat/evidence/catalog-headed';manifest=json.loads((folder/('source-'+short+'.json')).read_text());source=pathlib.Path(manifest['source_root'])
for name,h in manifest['files'].items():assert hashlib.sha256((source/name).read_bytes()).hexdigest()==h,name
target=root/'.superpowers/review-exports/t_a74e9684-combat-target'
assert target.is_dir()
env=dict(os.environ);env.update(CARGO_TARGET_DIR=str(target),GIT_COMMIT=manifest['host_commit'],GIT_DIRTY='0')
jobs=[('headless','catalog_boundary_live',['test','-p','host-play','--test','catalog_boundary_live','--features','memory-profile','--no-run','--message-format=json'],'catalog-boundary-'),('native','catalog_watch',['build','-p','panel','--example','catalog_watch','--message-format=json'],'catalog-watch-'),('paired','paired_catalog_live',['test','-p','host-play','--test','paired_catalog_live','--no-run','--message-format=json'],'paired-catalog-')]
for mode,name,args,prefix in [j for j in jobs if j[0] != 'paired']:
 log=folder/('build-'+mode+'-'+short);assert not log.with_suffix('.log').exists();command=['cargo']+args;before=time.monotonic();start=datetime.datetime.now(datetime.timezone.utc).isoformat()
 with log.with_suffix('.jsonl').open('w') as out,log.with_suffix('.log').open('w') as err:
  p=subprocess.Popen(command,cwd=source,env=env,stdout=out,stderr=err);print(json.dumps(dict(mode=mode,pid=p.pid,command=command)),flush=True);code=p.wait()
 receipt=dict(mode=mode,command=command,started_at=start,elapsed_seconds=round(time.monotonic()-before,3),exit_code=code,isolated_build=True,cargo_target_dir=str(target),target_started_empty=False,cache_reuse="Exclusive root compiler cache reused after completed Wildy and loader checks; source hashes and binary identity reverified; not empty-started")
 if code:print(json.dumps(receipt),flush=True);sys.exit(code)
 rows=[json.loads(s) for s in log.with_suffix('.jsonl').read_text().splitlines()];bins=[r['executable'] for r in rows if r.get('reason')=='compiler-artifact' and r.get('target',{}).get('name')==name and r.get('executable')];assert len(bins)==1,bins
 binary=root/'.superpowers/catalog-headed'/(prefix+short);assert not binary.exists();shutil.copy2(bins[0],binary)
 for path,h in manifest['files'].items():assert hashlib.sha256((source/path).read_bytes()).hexdigest()==h,path
 receipt.update(host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],source_root=str(source),binary=str(binary),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),binary_bytes=binary.stat().st_size,source_files_verified=len(manifest['files']),elapsed_is_not_performance_measurement=True)
 (folder/(('binary-headless-' if mode=='headless' else 'binary-tui-' if mode=='tui' else 'binary-paired-' if mode=='paired' else 'binary-')+short+'.json')).write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt),flush=True)
