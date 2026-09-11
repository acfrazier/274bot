import pathlib,json,hashlib,subprocess,os,time,datetime
root=pathlib.Path(__file__).resolve().parents[2];ev=root/'docs/compat/evidence/catalog-headed';m=json.loads((ev/'source-78cc99d0.json').read_text());source=pathlib.Path(m['source_root']);target=root/'.superpowers/catalog-headed/target-isolated-78cc99d0'
assert target.is_dir();env=dict(os.environ);env.update(CARGO_TARGET_DIR=str(target),GIT_COMMIT=m['host_commit'],GIT_DIRTY='0');rows=[]
for label,args in [('catalog',['test','-p','host-play','--features','memory-profile','--test','catalog_boundary_live']),('scenario',['test','-p','scenario','--lib']),('clippy-catalog',['clippy','-p','host-play','--features','memory-profile','--test','catalog_boundary_live','--','-D','warnings'])]:
 p=ev/('checks-78cc99d0-inputs-'+label+'.log');assert not p.exists();before=time.monotonic()
 with p.open('w') as f:code=subprocess.call(['cargo']+args,cwd=source,env=env,stdout=f,stderr=subprocess.STDOUT)
 row=dict(label=label,command=['cargo']+args,exit_code=code,elapsed_seconds=round(time.monotonic()-before,3),log=str(p.relative_to(root)),log_sha256=hashlib.sha256(p.read_bytes()).hexdigest());rows.append(row);print(json.dumps(row),flush=True)
 if code:break
for name,h in m['files'].items():assert hashlib.sha256((source/name).read_bytes()).hexdigest()==h,name
ok=len(rows)==3 and all(r['exit_code']==0 for r in rows);out=dict(host_commit=m['host_commit'],client_commit=m['client_commit'],source_files_verified=len(m['files']),checks=rows,all_passed=ok,isolated_build=True)
out['prior_attempt']='docs/compat/evidence/catalog-headed/checks-78cc99d0.json';out['prior_count_dialog_pass']=json.loads((ev/'checks-78cc99d0.json').read_text())['checks'][0];assert out['prior_count_dialog_pass']['exit_code']==0
out['prior_count_dialog_clippy_pass']=json.loads((ev/'checks-78cc99d0-retry.json').read_text())['checks'][0];assert out['prior_count_dialog_clippy_pass']['exit_code']==0;out['frozen_inputs_link']=str(source/'.superpowers/inputs')
(ev/'checks-78cc99d0-inputs.json').write_text(json.dumps(out,indent=2)+'\n');gatep=ev/'review-gates-78cc99d0.json';gate=json.loads(gatep.read_text());gate['root_fixture_checks_passed']=ok;gatep.write_text(json.dumps(gate,indent=2)+'\n');raise SystemExit(not ok)
