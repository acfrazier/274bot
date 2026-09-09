import hashlib,json,os,pathlib,platform,signal,subprocess,sys,tarfile,time
root=pathlib.Path(__file__).resolve().parent
m=json.loads((root/'source-manifest.json').read_text())
assert platform.system()=='Linux' and platform.machine()=='x86_64'
assert hashlib.sha256((root/'source.tar.gz').read_bytes()).hexdigest()==m['archive_sha256']
src=root/'source';src.mkdir(exist_ok=False)
with tarfile.open(root/'source.tar.gz') as t:
 members=t.getmembers();assert {x.name for x in members}=={x['path'] for x in m['files']}
 for x in members:
  assert x.isfile() and not pathlib.PurePosixPath(x.name).is_absolute() and '..' not in pathlib.PurePosixPath(x.name).parts
  f=src/x.name;f.parent.mkdir(parents=True,exist_ok=True);f.write_bytes(t.extractfile(x).read())
def verify():
 for r in m['files']:
  b=(src/r['path']).read_bytes();assert len(b)==r['bytes'] and hashlib.sha256(b).hexdigest()==r['sha256']
verify()
env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',PYTHONUNBUFFERED='1')
modules=['test_current_tui_calibration','test_run_managed_cell','test_run_diagnostic','test_build_provenance','test_managed_receipt']
commands=[([sys.executable,'-B','-m','unittest','-v',*['docs.memory.'+x for x in modules]],src,'full-focused'),([sys.executable,'-B','-m','unittest','-v','test_validate_direct_owner_capture'],src/'docs/memory','owner-validator')]
result={'scope':'native Linux generated functional qualification only; no live/private inputs','commit':m['commit'],'platform':platform.platform(),'machine':platform.machine(),'python':sys.version,'git':subprocess.check_output(['git','--version'],text=True).strip(),'source_verified_before':True,'steps':[]}
for argv,cwd,label in commands:
 start=time.monotonic();log=root/(label+'.log');timed=False
 with log.open('xb') as f:
  p=subprocess.Popen(argv,cwd=cwd,env=env,stdout=f,stderr=subprocess.STDOUT,start_new_session=True)
  try:rc=p.wait(timeout=180)
  except subprocess.TimeoutExpired:
   timed=True;os.killpg(p.pid,signal.SIGTERM)
   try:rc=p.wait(timeout=5)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);rc=p.wait(timeout=5)
 try:os.killpg(p.pid,0);group_absent=False
 except ProcessLookupError:group_absent=True
 b=log.read_bytes();result['steps'].append({'argv':argv,'cwd':str(cwd),'exit_code':rc,'timeout':timed,'elapsed_s':time.monotonic()-start,'owned_process_group_absent':group_absent,'log':log.name,'sha256':hashlib.sha256(b).hexdigest(),'bytes':len(b)})
 (root/'result.json').write_text(json.dumps(result,indent=2)+'\n')
verify();result['source_verified_after']=True
result['commands_passed']=all(x['exit_code']==0 and not x['timeout'] and x['owned_process_group_absent'] for x in result['steps'])
(root/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
