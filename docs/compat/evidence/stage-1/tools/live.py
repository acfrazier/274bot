from pathlib import Path
import os,subprocess,json,hashlib,time,datetime
root=Path.cwd();campaign=root.parent/'rs2b0t-multirevision';ev=root/'docs/compat/evidence/stage-1';target=campaign/'.superpowers/review-exports/t_a74e9684-combat-target'
host=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();client=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root/'vendor/fr-client-rust',text=True).strip()
env={**os.environ,'CARGO_TARGET_DIR':str(target),'GIT_COMMIT':host,'GIT_DIRTY':'0'}
cmd=['cargo','test','-p','host-play','--features','memory-profile','--test','revision_boundary_live','--test','world_boundary_live','--no-run','--message-format=json']
with (ev/'live-build.jsonl').open('x') as f, (ev/'live-build.log').open('x') as e: code=subprocess.call(cmd,env=env,stdout=f,stderr=e)
assert code==0,code
bins={}
for line in (ev/'live-build.jsonl').read_text().splitlines():
 try:r=json.loads(line)
 except ValueError:continue
 if r.get('reason')=='compiler-artifact' and r.get('executable'):bins[r['target']['name']]=Path(r['executable'])
assert {'revision_boundary_live','world_boundary_live'}<=bins.keys()
rows=[]
for revision in ['274','289']:
 engine=Path('/Users/acfrazier/experiments/Server/engine' if revision=='274' else '/Users/acfrazier/experiments/lostcity-289/engine')
 for case in ['boundary','nav_full','nav_door','guardian_lamp']:
  label=f'r{revision}-{case}';binary=bins['revision_boundary_live' if case=='boundary' else 'world_boundary_live'];runenv={'LIVE':'1'}
  if case=='boundary':runenv.update(BOUNDARY_REVISION=revision,BOUNDARY_ENGINE_DIR=str(engine))
  else:
   pack=campaign/'.superpowers/world-capabilities'/revision
   runenv.update(WORLD_REVISION=revision,WORLD_ENGINE_DIR=str(engine),WORLD_CASE=case,WORLD_NAV_PACK=str(pack/'274bot.navpack'),WORLD_NAV_FLAGS=str(pack/'274bot.navflags'))
  cmd=[str(binary),'--ignored','--nocapture'];start=time.monotonic();started=datetime.datetime.now(datetime.timezone.utc).isoformat();log=ev/(label+'.log')
  with log.open('x') as f:code=subprocess.call(cmd,env={**env,**runenv},stdout=f,stderr=subprocess.STDOUT)
  row=dict(host_commit=host,client_commit=client,command=cmd,environment=runenv,cwd=str(root),started_at=started,seconds=round(time.monotonic()-start,3),exit_code=code,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),log_sha256=hashlib.sha256(log.read_bytes()).hexdigest(),fixture_preparation_is_not_action_proof=True,elapsed_is_not_performance_measurement=True)
  (ev/(label+'.json')).write_text(json.dumps(row,indent=2)+'\n');rows.append(row);print(json.dumps(row),flush=True)
  if code:raise SystemExit(code)
