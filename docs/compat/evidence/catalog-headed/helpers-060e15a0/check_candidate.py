import pathlib,hashlib,json,subprocess,os,datetime,sys
short=sys.argv[1];assert len(short)==8 and all(c in "0123456789abcdef" for c in short)
root=pathlib.Path(__file__).resolve().parents[2];e=root/'docs/compat/evidence/catalog-headed';m=json.loads((e/('source-'+short+'.json')).read_text());b=json.loads((e/('binary-headless-'+short+'.json')).read_text());s=pathlib.Path(m['source_root']);matrix=json.loads((s/'docs/compat/support-matrix.json').read_text());inputs={}
for cat in matrix['catalogs']:
 refs={cat['registry_path']:cat['registry_sha256']};refs.update({r['source_path']:r['source_sha256'] for r in matrix['rows'] if r['catalog_commit']==cat['commit']})
 for n,h in refs.items():
  rel=pathlib.Path(cat['read_only_path'])/n;data=(root/rel).read_bytes();assert hashlib.sha256(data).hexdigest()==h;dest=s/rel;dest.parent.mkdir(parents=True,exist_ok=True)
  with dest.open('xb') as f:f.write(data)
  inputs[str(rel)]=h
jobs=[('catalog',[b['binary']]),('scenario',['cargo','test','-p','scenario']),('clippy-catalog',['cargo','clippy','-p','host-play','--features','memory-profile','--test','catalog_boundary_live','--no-deps','--','-D','warnings'])];checks=[];env=dict(os.environ,CARGO_TARGET_DIR=b['cargo_target_dir'])
for name,cmd in jobs:
 p=e/(('checks-'+short+'-')+name+'.log');assert not p.exists()
 with p.open('w') as out:code=subprocess.call(cmd,cwd=s,env=env,stdout=out,stderr=subprocess.STDOUT)
 check=dict(name=name,command=cmd,exit_code=code,log=str(p.relative_to(root)),sha256=hashlib.sha256(p.read_bytes()).hexdigest());checks.append(check);print(json.dumps(check),flush=True);assert code==0
for n,h in {**m['files'],**inputs}.items():assert hashlib.sha256((s/n).read_bytes()).hexdigest()==h,n
(e/('checks-'+short+'.json')).write_text(json.dumps(dict(host_commit=m['host_commit'],client_commit=m['client_commit'],source_files_verified=len(m['files']),isolated_build=True,cargo_target_dir=b['cargo_target_dir'],supplementary_catalog_inputs=inputs,checks=checks,verified_at=datetime.datetime.now(datetime.timezone.utc).isoformat()),indent=2)+'\n')
