import datetime,hashlib,json,os,pathlib,shutil,subprocess,time
r=pathlib.Path(__file__).resolve().parents[2];e=r/'docs/compat/evidence/catalog-headed';short='7c2eef6e';src=r/'.superpowers/review-exports/catalog-root-7c2eef6e';m=json.loads(src.with_name(src.name+'-manifest.json').read_text());target=r/'.superpowers/catalog-headed/target-isolated-28d97f38'
assert target.is_dir();assert not (e/f'checks-paired-{short}.json').exists();assert m['host_commit'].startswith(short)
for name,h in m['files'].items():assert hashlib.sha256((src/name).read_bytes()).hexdigest()==h,name
(e/f'source-{short}.json').write_text(json.dumps(m,indent=2)+'\n')
for name in ('inputs','world-capabilities'):
 dest=src/'.superpowers'/name;dest.parent.mkdir(exist_ok=True);assert not dest.exists();dest.symlink_to(r/'.superpowers'/name,target_is_directory=True)
review=json.loads((r/'docs/compat/evidence/paired-catalog-fixtures/root-review-verification.json').read_text());assert review['actual_model']=='grok-4.5' and review['metadata']['review_outcome']=='approved'
env=os.environ.copy();env['CARGO_TARGET_DIR']=str(target);records=[]
for label,command in [('test',['cargo','test','-p','host-play','--test','paired_catalog_live','--','--nocapture']),('clippy',['cargo','clippy','-p','host-play','--test','paired_catalog_live','--no-deps','--','-D','warnings']),('binary',['cargo','test','-p','host-play','--test','paired_catalog_live','--no-run','--message-format=json'])]:
 log=e/f'checks-paired-{short}-{label}.log';before=time.monotonic()
 with log.open('x') as f:res=subprocess.run(command,cwd=src,env=env,stdout=f,stderr=subprocess.STDOUT)
 body=log.read_text();record=dict(label=label,command=command,exit_code=res.returncode,elapsed_seconds=round(time.monotonic()-before,3),log=str(log.relative_to(r)),log_sha256=hashlib.sha256(log.read_bytes()).hexdigest());records.append(record)
 result=dict(host_commit=m['host_commit'],client_commit=m['client_commit'],source_files_verified=len(m['files']),source_root=str(src),isolated_build=True,cargo_target_dir=str(target),target_started_empty=False,cache_reuse='Root28 completed source-verified functional compiler cache; new exact7c source directory. No independently empty-started claim for7c.',checks=records,all_passed=len(records)==3 and all(x['exit_code']==0 for x in records))
 (e/f'checks-paired-{short}.json').write_text(json.dumps(result,indent=2)+'\n')
 assert res.returncode==0,record
 assert not any('SKIP' in x or 'skipping' in x.lower() for x in body.splitlines()),body[-1000:]
 if label=='binary':
  arts=[]
  for line in body.splitlines():
   if not line.startswith('{'):continue
   row=json.loads(line)
   if row.get('reason')=='compiler-artifact' and row.get('target',{}).get('name')=='paired_catalog_live' and row.get('executable'):arts.append(row)
  assert len(arts)==1;binary=r/'.superpowers/catalog-headed'/f'paired-catalog-{short}';assert not binary.exists();shutil.copy2(arts[0]['executable'],binary);result.update(binary=str(binary),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),binary_bytes=binary.stat().st_size,review_receipt='docs/compat/evidence/paired-catalog-fixtures/root-review-verification.json',elapsed_is_not_performance_measurement=True);(e/f'binary-paired-{short}.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(record),flush=True)
for name,h in m['files'].items():assert hashlib.sha256((src/name).read_bytes()).hexdigest()==h,name
