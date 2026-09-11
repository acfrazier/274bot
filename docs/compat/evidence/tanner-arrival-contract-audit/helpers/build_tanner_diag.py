import hashlib,json,os,pathlib,shutil,subprocess,time
r=pathlib.Path(__file__).resolve().parents[2];e=r/'docs/compat/evidence/tanner-arrival-contract-audit';m=json.loads((e/'source-diagnostic-78.json').read_text());src=pathlib.Path(m['source_root']);target=r/'.superpowers/catalog-headed/target-isolated-28d97f38';assert target.is_dir();env=os.environ.copy();env['CARGO_TARGET_DIR']=str(target)
for name,h in m['files'].items():assert hashlib.sha256((src/name).read_bytes()).hexdigest()==h,name
checks=[]
for label,command in [('build',['cargo','test','-p','host-play','--features','memory-profile','--test','catalog_boundary_live','--no-run','--message-format=json']),('clippy',['cargo','clippy','-p','host-play','--features','memory-profile','--lib','--no-deps','--','-D','warnings'])]:
 p=e/f'diagnostic-{label}.log';before=time.monotonic()
 with p.open('x') as log:res=subprocess.run(command,cwd=src,env=env,stdout=log,stderr=subprocess.STDOUT)
 checks.append(dict(command=command,exit_code=res.returncode,elapsed_seconds=round(time.monotonic()-before,3),log=str(p.relative_to(r)),log_sha256=hashlib.sha256(p.read_bytes()).hexdigest()));(e/'diagnostic-checks.json').write_text(json.dumps(checks,indent=2)+'\n');print(json.dumps(checks[-1]),flush=True);assert res.returncode==0
 if label=='build':
  artifacts=[json.loads(l) for l in p.read_text().splitlines() if l.startswith('{')];artifacts=[a for a in artifacts if a.get('reason')=='compiler-artifact' and a.get('target',{}).get('name')=='catalog_boundary_live' and a.get('executable')];assert len(artifacts)==1;binary=r/'.superpowers/catalog-headed/tanner-arrival-78diag';assert not binary.exists();shutil.copy2(artifacts[0]['executable'],binary)
identity=dict(host_commit=m['host_commit'],client_commit=m['client_commit'],source_root=str(src),source_manifest='docs/compat/evidence/tanner-arrival-contract-audit/source-diagnostic-78.json',binary=str(binary),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),binary_bytes=binary.stat().st_size,isolated_build=True,target_started_empty=False,cargo_target_dir=str(target),cache_reuse='Completed root28/7c functional compiler cache; new exact78 source with one documented diagnostic-only overlay. No independent empty-start claim.',diagnostic_only=True,acceptance_eligible=False,checks=checks)
(e/'diagnostic-binary.json').write_text(json.dumps(identity,indent=2)+'\n')
for name,h in m['files'].items():assert hashlib.sha256((src/name).read_bytes()).hexdigest()==h,name
