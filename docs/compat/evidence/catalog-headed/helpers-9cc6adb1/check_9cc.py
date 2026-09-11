import pathlib,json,hashlib,subprocess,os,time,datetime
root=pathlib.Path(__file__).resolve().parents[2];ev=root/'docs/compat/evidence/catalog-headed';m=json.loads((ev/'source-9cc6adb1.json').read_text());source=pathlib.Path(m['source_root']);target=root/'.superpowers/catalog-headed/target-isolated-28d97f38'
assert target.is_dir();env=dict(os.environ);env.update(CARGO_TARGET_DIR=str(target),GIT_COMMIT=m['host_commit'],GIT_DIRTY='0');rows=[]
for label,args in [('inventory', ['test', '-p', 'script', '--test', 'client_adapter_inventory']), ('registry', ['test', '-p', 'script', '--test', 'rs2b0t_registry']), ('resolver', ['test', '-p', 'script', '--lib', 'resolve_item_options']), ('tui-params', ['test', '-p', 'tui', '--lib', 'script_params']), ('catalog', ['test', '-p', 'host-play', '--features', 'memory-profile', '--test', 'catalog_boundary_live']), ('clippy-inventory', ['clippy', '-p', 'script', '--test', 'client_adapter_inventory', '--', '-D', 'warnings']), ('clippy-script', ['clippy', '-p', 'script', '--lib', '--', '-D', 'warnings'])]:
 p=ev/('checks-9cc6adb1-'+label+'.log');assert not p.exists();before=time.monotonic()
 with p.open('w') as f:code=subprocess.call(['cargo']+args,cwd=source,env=env,stdout=f,stderr=subprocess.STDOUT)
 row=dict(label=label,command=['cargo']+args,exit_code=code,elapsed_seconds=round(time.monotonic()-before,3),log=str(p.relative_to(root)),log_sha256=hashlib.sha256(p.read_bytes()).hexdigest());rows.append(row);print(json.dumps(row),flush=True)
 if label=='nav-banks':assert 'SKIP:' not in p.read_text(), 'Pinned nav inputs must actually run'
 if code:break
for name,h in m['files'].items():assert hashlib.sha256((source/name).read_bytes()).hexdigest()==h,name
ok=len(rows)==7 and all(r['exit_code']==0 for r in rows);out=dict(host_commit=m['host_commit'],client_commit=m['client_commit'],source_files_verified=len(m['files']),checks=rows,all_passed=ok,isolated_build=True,target_started_empty=False,cache_reuse="Completed exclusive root28 target; exact9cc source")
(ev/'checks-9cc6adb1.json').write_text(json.dumps(out,indent=2)+'\n');gatep=ev/'review-gates-9cc6adb1.json';gate=json.loads(gatep.read_text());gate['root_fixture_checks_passed']=ok;gatep.write_text(json.dumps(gate,indent=2)+'\n');raise SystemExit(not ok)
