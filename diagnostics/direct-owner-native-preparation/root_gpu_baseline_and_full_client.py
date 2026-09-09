"""Record original GPU failure attribution and complete both client test inventories."""
import argparse, hashlib, importlib.util, json, os, shutil, tarfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--root',type=Path,required=True);a=p.parse_args();r=a.root.resolve();out=r/'root-gpu-baseline-and-client-01';out.mkdir()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):
 n=p.with_name(p.name+'.next')
 with n.open('x') as f:json.dump(d,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
 os.replace(n,p)
helper=r/'coverage-overlay/stage_coverage_overlay.py';assert sha(helper)=='6e1aab8eae3bd217f1f2b320f8b1b45e7a8fae924563dd7c74449ed4af5a3082'
spec=importlib.util.spec_from_file_location('q',helper);q=importlib.util.module_from_spec(spec);spec.loader.exec_module(q)
archive=r/'original-client-3456.tar';assert sha(archive)=='448b2ea7e4f76658bed71d5ec1016bc084230894530f9b35073cabd24c67e18b'
with tarfile.open(archive) as t:
 for m in t:
  rel=Path(m.name);assert not rel.is_absolute() and '..' not in rel.parts and rel.parts[0]=='original-client';dest=out/rel
  if m.isdir():dest.mkdir(parents=True,exist_ok=True)
  elif m.isfile():
   dest.parent.mkdir(parents=True,exist_ok=True)
   with t.extractfile(m) as src,dest.open('xb') as dst:shutil.copyfileobj(src,dst)
   dest.chmod(m.mode)
  elif m.issym():
   target=Path(m.linkname);assert not target.is_absolute() and '..' not in target.parts;dest.parent.mkdir(parents=True,exist_ok=True);dest.symlink_to(m.linkname)
  else:raise RuntimeError('Unsupported archive member '+m.name)
original=out/'original-client';candidate=r/'root-coverage-native-03-remainder/stage/derived-source';manifest=json.loads((r/'frozen-source-manifest.json').read_text())
def inventory(root):return {str(p.relative_to(root)):sha(p) for p in root.rglob('*') if p.is_file()}
original_before=inventory(original);candidate_before=q.verify_tree(candidate,manifest,q.DERIVED_HASHES)
env=q.safe_environment(r/'linux-qualification-01/target');env.update(PATH='/home/builder/.cargo/bin:'+env.get('PATH',''),PYTHONDONTWRITEBYTECODE='1',CARGO_BUILD_TARGET='x86_64-unknown-linux-gnu',SKIP_GPU='0')
for k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER'):env.pop(k,None)
base=['cargo','test','--offline','--locked','-p','client'];tail=['--','--test-threads=1']
matrix=[('original-gpu-texture-shade',base+['--test','gpu_texture','gpu_textured_shade_scales_texel_brightness','--','--exact','--nocapture','--test-threads=1'],original),('candidate-client-all-tests-feature-off',base+['--tests','--no-fail-fast']+tail,candidate/'vendor/fr-client-rust'),('candidate-client-all-tests-feature-on',base+['--features','memory-owner-capture','--tests','--no-fail-fast']+tail,candidate/'vendor/fr-client-rust')]
steps=[];record={'scope':'Original-client baseline attribution and complete client inventory; expected failures stay failures, no native/live acceptance','original_commit':'3456edc8dabf7b25ada78110ffa56327af9f67a4','archive_sha256':sha(archive),'helper_sha256':sha(helper),'runner_sha256':sha(Path(__file__)),'environment':{k:env.get(k) for k in ('CARGO_TARGET_DIR','CARGO_BUILD_TARGET','SKIP_GPU','BOT_CPU','BOT_MEMORY_OWNER_CAPTURE','R274_TEST_FORCE_NO_GPU')},'original_source_before':original_before,'candidate_source_before':candidate_before,'steps':steps,'live_qualified':False}
write(out/'launch.json',record)
try:
 for name,command,cwd in matrix:
  assert shutil.disk_usage(r).free>=4*1024**3
  print('START',name,flush=True);step=q.run_step(name,command,cwd,out,env,900);steps.append(step);write(out/'steps.json',steps);print('END',name,step['exit'],round(step['wall_s'],3),flush=True)
  if step['timed_out'] or not step['cleanup']['group_absent']:break
finally:
 record['original_source_after']=inventory(original);record['candidate_source_after']=q.verify_tree(candidate,manifest,q.DERIVED_HASHES);record['source_unchanged']=record['original_source_after']==original_before and record['candidate_source_after']==candidate_before;write(out/'result.json',record)
print('RECORDED',len(steps),'commands; failures are not passing tests',flush=True)
raise SystemExit(0 if len(steps)==3 and record['source_unchanged'] else 1)
