import datetime,hashlib,json,os,pathlib,shutil,subprocess,time
root=pathlib.Path(__file__).resolve().parents[2];out=root/'docs/compat/evidence/navigation-load';proof=root/'.superpowers/navigation-proof'
for variant in ['external','bundled']:
 m=json.loads((out/f'private-source-{variant}.json').read_text());source=pathlib.Path(m['source_root']);base=json.loads((root/m['unchanged_source_manifest']).read_text());expected=base['files']|m['private_file_overrides']
 for name,h in expected.items():assert hashlib.sha256((source/name).read_bytes()).hexdigest()==h,name
 for package,name in [('host-play','nav_origin_probe'),('panel','startup_watch')]:
  stem=out/f'build-isolated-{variant}-{name}';assert not stem.with_suffix('.jsonl').exists();env=dict(os.environ,CARGO_TARGET_DIR=str(proof/('target-isolated-'+variant)),GIT_COMMIT=m['host_commit'],GIT_DIRTY='1');command=['cargo','build','-p',package,'--example',name,'--message-format=json'];start=datetime.datetime.now(datetime.timezone.utc).isoformat();before=time.monotonic()
  with stem.with_suffix('.jsonl').open('w') as log,stem.with_suffix('.log').open('w') as err:
   p=subprocess.Popen(command,cwd=source,env=env,stdout=log,stderr=err);print(json.dumps({'variant':variant,'example':name,'pid':p.pid}),flush=True);code=p.wait()
  record={'variant':variant,'example':name,'command':command,'started_at':start,'elapsed_seconds':round(time.monotonic()-before,3),'exit_code':code,'source':str(source),'source_identity':str(out/f'private-source-{variant}.json'),'elapsed_is_not_performance_measurement':True,'isolated_build':True,'cargo_target_dir':env['CARGO_TARGET_DIR']}
  if code:
   stem.with_suffix('.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record),flush=True);raise SystemExit(code)
  lines=[json.loads(s) for s in stem.with_suffix('.jsonl').read_text().splitlines()];bins=[v['executable'] for v in lines if v.get('reason')=='compiler-artifact' and v.get('target',{}).get('name')==name and v.get('executable')];assert len(bins)==1
  dest=proof/f'{name}-{variant}-d2372fe0';assert not dest.exists();shutil.copy2(bins[0],dest)
  for path,h in expected.items():assert hashlib.sha256((source/path).read_bytes()).hexdigest()==h,path
  record.update(binary=str(dest),binary_sha256=hashlib.sha256(dest.read_bytes()).hexdigest(),binary_bytes=dest.stat().st_size,source_files_verified=len(expected));stem.with_suffix('.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record),flush=True)
