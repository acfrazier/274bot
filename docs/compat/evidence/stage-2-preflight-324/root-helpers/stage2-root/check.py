from pathlib import Path
import os,json,hashlib,shutil,subprocess,time,datetime
root=Path(__file__).resolve().parents[2];src=root/'.superpowers/review-exports/catalog-root-stage2-324';ev=root/'docs/compat/evidence/stage-2-preflight-324';ev.mkdir(exist_ok=True);m=json.loads(src.with_name(src.name+'-manifest.json').read_text());shutil.copy2(src.with_name(src.name+'-manifest.json'),ev/'source.json')
for name,h in m['files'].items():assert hashlib.sha256((src/name).read_bytes()).hexdigest()==h,name
inputs={}
for catalog in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
 source=root/f'.superpowers/inputs/rs2b0t-{catalog}';dest=src/f'.superpowers/inputs/rs2b0t-{catalog}';assert not dest.exists();shutil.copytree(source,dest)
 files={str(p.relative_to(source)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source.rglob('*') if p.is_file()}
 for p,h in files.items():assert hashlib.sha256((dest/p).read_bytes()).hexdigest()==h,p
 inputs[catalog]=files
(ev/'frozen-inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'.superpowers/review-exports/t_a74e9684-combat-target'),RS2B0T=str(src/'.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775'),GIT_COMMIT=m['host_commit'],GIT_DIRTY='0');env.pop('LIVE',None)
jobs=[('fmt',['cargo','fmt','--all','--','--check']),('clippy',['cargo','clippy','--workspace','--all-targets','--features','memory-profile','--locked','--','-D','warnings']),('script-tests',['cargo','test','-p','script','--features','memory-profile','--locked','--no-fail-fast','--','--test-threads=1']),('catalog-tests',['cargo','test','-p','host-play','--test','catalog_boundary_live','--features','memory-profile','--locked','--','--test-threads=1'])]
rows=[]
for label,command in jobs:
 start=time.monotonic()
 with (ev/(label+'.log')).open('x') as out:
  proc=subprocess.Popen(command,cwd=src,env=env,stdout=out,stderr=subprocess.STDOUT);print(label,proc.pid,flush=True);code=proc.wait()
 rows.append(dict(label=label,command=command,exit_code=code,elapsed_seconds=round(time.monotonic()-start,3)))
 (ev/'checks.json').write_text(json.dumps(dict(host_commit=m['host_commit'],client_commit=m['client_commit'],source_root=str(src),cargo_target_dir=env['CARGO_TARGET_DIR'],cache_reused=True,isolated_source=True,RS2B0T=env['RS2B0T'],elapsed_is_not_performance_measurement=True,checks=rows),indent=2)+'\n');print(rows[-1],flush=True)
for name,h in m['files'].items():assert hashlib.sha256((src/name).read_bytes()).hexdigest()==h,name
print('SOURCE VERIFIED',len(m['files']),flush=True)
