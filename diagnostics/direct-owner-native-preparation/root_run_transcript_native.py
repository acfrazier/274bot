"""Root-native affected regression proof using the unchanged reviewed helpers."""
import argparse, contextlib, hashlib, importlib.util, json, os, shutil, sys
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--root',type=Path,required=True);a=p.parse_args();r=a.root.resolve();out=r/'root-transcript-native-01';out.mkdir()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):
 n=p.with_name(p.name+'.next')
 with n.open('x') as f:json.dump(d,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
 os.replace(n,p)
helper=r/'transcript-test-overlay/stage_transcript_overlay.py';assert sha(helper)=='c14b1b5648b993d10b0979de5fc14f49b97d147b502edcee1f91169bd51b9b54'
patch=helper.with_name('transcript-test-only.patch');assert sha(patch)=='1e5ac15b25d113a180f8212fdd282bd63d76e79c14aa200560c2184106deb995'
spec=importlib.util.spec_from_file_location('transcript',helper);t=importlib.util.module_from_spec(spec);spec.loader.exec_module(t)
q=t.load_helper(r/'coverage-overlay/stage_coverage_overlay.py');assert sha(r/'coverage-overlay/stage_coverage_overlay.py')==t.EXPECTED['coverage_helper']
target=r/'linux-qualification-01/target';assert shutil.disk_usage(r).free>=20*1024**3
argv=[str(helper),'--archive',str(r/'frozen-direct-owner-source.tar.gz'),'--manifest',str(r/'frozen-source-manifest.json'),'--coverage-helper',str(r/'coverage-overlay/stage_coverage_overlay.py'),'--coverage-patch',str(r/'coverage-overlay/coverage-test-only.patch'),'--tui-patch',str(r/'tui-test-only.patch'),'--transcript-patch',str(patch),'--output-dir',str(out/'focused'),'--target-dir',str(target),'--build-target','x86_64-unknown-linux-gnu','--run-focused','--timeout','900']
record={'scope':'Native original cache-dependence controls, corrected focused tests, and three complete affected host-play suites; prior GPU failures remain failed, no full GPU or live qualification','reviewed_commit':'a3abbddf8b8f90e795eb3ff691399644e2bbc346','helper_sha256':sha(helper),'patch_sha256':sha(patch),'runner_sha256':sha(Path(__file__)),'focused_argv':argv,'steps':[],'live_qualified':False,'native_gpu_qualified':False}
write(out/'launch.json',record)
print('START reviewed native transcript controls and focused tests',flush=True)
prior=sys.argv
try:
 sys.argv=argv
 with (out/'focused-helper.log').open('x') as log,contextlib.redirect_stdout(log),contextlib.redirect_stderr(log):code=t.main()
finally:sys.argv=prior
record['focused_helper_exit']=code
receipt=json.loads((out/'focused/overlay-receipt.json').read_text());record['focused_receipt_sha256']=sha(out/'focused/overlay-receipt.json')
write(out/'result.json',record)
assert code==0 and receipt['all_requested_passed'] and receipt['original_ambient_dependence_exposed'];assert len(receipt['original_probe_steps'])==2 and len(receipt['steps'])==4
print('PASS two original controls and four corrected focused commands',flush=True)
source=Path(receipt['source']);manifest=json.loads((r/'frozen-source-manifest.json').read_text());before=t.verify_transcript_contract(source,manifest,q);locks=q.verify_locks(source,manifest,'before-full-affected-tests')
env=q.safe_environment(target);env.update(PATH='/home/builder/.cargo/bin:'+env.get('PATH',''),PYTHONDONTWRITEBYTECODE='1',CARGO_BUILD_TARGET='x86_64-unknown-linux-gnu',SKIP_GPU='0',SNAPSHOT_FRAME_FIXTURE_CACHE_DIR=receipt['cache_fixtures']['empty']['path'])
for k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER'):env.pop(k,None)
matrix=[row for row in q.feature_off_matrix(source)+q.feature_on_matrix(source) if row[0] in ('feature-off-host-play-profile','feature-on-host-play-profile')]
assert len(matrix)==2
matrix.append(('snapshot-dedup-full-integration',['cargo','test','--offline','--locked','-p','host-play','--features','memory-profile-no-alloc,snapshot-dedup','--test','snapshot_frame_equivalence','--','--test-threads=1'],source))
record['source_before']=before;record['locks_before']=locks;record['environment']={k:env.get(k) for k in ('CARGO_TARGET_DIR','CARGO_BUILD_TARGET','BOT_CPU','BOT_MEMORY_OWNER_CAPTURE','R274_TEST_FORCE_NO_GPU','SKIP_GPU','SNAPSHOT_FRAME_FIXTURE_CACHE_DIR')}
try:
 for name,command,cwd in matrix:
  assert shutil.disk_usage(r).free>=4*1024**3
  print('START',name,flush=True);step=q.run_step(name,command,cwd,out,env,900);record['steps'].append(step);write(out/'steps.json',record['steps']);print('END',name,step['exit'],round(step['wall_s'],3),flush=True)
  if step['exit']!=0 or step['timed_out'] or not step['cleanup']['group_absent']:break
finally:
 record['source_after']=t.verify_transcript_contract(source,manifest,q);record['locks_after']=q.verify_locks(source,manifest,'after-full-affected-tests');record['source_unchanged']=record['source_after']==before;record['affected_suites_passed']=len(record['steps'])==3 and all(s['exit']==0 and not s['timed_out'] and s['cleanup']['group_absent'] for s in record['steps']);write(out/'result.json',record)
raise SystemExit(0 if record['affected_suites_passed'] and record['source_unchanged'] else 1)
