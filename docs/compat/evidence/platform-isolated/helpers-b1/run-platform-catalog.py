import datetime,hashlib,json,os,pathlib,subprocess,sys,time
root=pathlib.Path(sys.argv[1]).resolve();engine=pathlib.Path(sys.argv[2]).resolve();revision,case,catalog=sys.argv[3:]
assert revision in ('274','289') and case in ('alcher','chicken_killer','thiever')
assert catalog in ('100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a')
identity=json.loads((root/'platform-build.json').read_text());job=next(x for x in identity['jobs'] if x['name']=='catalog_boundary_live');assert job['exit_code']==0
assert identity.get('isolated_build') and identity.get('target_started_empty'), 'BLOCKED: platform LIVE requires verified isolated build'
manifest=json.loads((root/'source-manifest.json').read_text())
assert manifest['host_commit']==identity['host_commit'] and manifest['client_commit']==identity['client_commit']
for name,h in manifest['files'].items():assert hashlib.sha256((root/name).read_bytes()).hexdigest()==h,name
binary=pathlib.Path(job['binary']);assert hashlib.sha256(binary.read_bytes()).hexdigest()==job['binary_sha256']
nav=root/'nav'/revision/'274bot.navpack';out=root/'live'/f'r{revision}-{case}-{catalog[:8]}'
out.parent.mkdir(exist_ok=True);assert not out.with_suffix('.log').exists()
env=dict(os.environ);env.update(LIVE='1',BOT_CPU='1',BOT_DEBUG='1',RUST_BACKTRACE='1',CATALOG_REVISION=revision,CATALOG_SCENARIO=case,CATALOG_COMMIT=catalog,CATALOG_ROOT=str(root/'inputs'/('rs2b0t-'+catalog)),CATALOG_NAV_PACK=str(nav),CATALOG_NAV_FLAGS=str(nav.with_suffix('.navflags')),CATALOG_ENGINE_DIR=str(engine))
command=[str(binary),'catalog_boundary_live','--ignored','--exact','--nocapture'];receipt=dict(host_commit=identity['host_commit'],client_commit=identity['client_commit'],platform=sys.platform,revision=revision,case=case,catalog_commit=catalog,binary_sha256=job['binary_sha256'],command=command,environment={k:v for k,v in env.items() if k.startswith('CATALOG_') or k in ('LIVE','BOT_CPU','BOT_DEBUG','RUST_BACKTRACE')},started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),elapsed_is_not_performance_measurement=True)
before=time.monotonic()
with out.with_suffix('.log').open('w') as log:
 p=subprocess.Popen(command,cwd=root/'source',env=env,stdout=log,stderr=subprocess.STDOUT);receipt['pid']=p.pid;out.with_suffix('.process.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt),flush=True);code=p.wait()
receipt.update(exit_code=code,elapsed_seconds=round(time.monotonic()-before,3),finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),log_sha256=hashlib.sha256(out.with_suffix('.log').read_bytes()).hexdigest(),nav_sha256=hashlib.sha256(nav.read_bytes()).hexdigest())
receipt.update(isolated_build=True,source_files_verified=len(manifest['files']),cargo_target_dir=identity['target_dir'])
out.with_suffix('.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt),flush=True);sys.exit(code)
