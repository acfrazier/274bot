"""Read-only pre-admission observations; creates no live authorization."""
import hashlib,json,os,pathlib,subprocess,sys,time
base=pathlib.Path('/home/acfrazier/274bot-campaign/direct-owner-runtime-392dbecc-20260909')
sys.path.insert(0,str(base/'admission-02/controller/docs/memory'))
import cache_provenance as cp,run_managed_cell as r,server_resources as sr
out=base/'pre-admission-observation-01';out.mkdir(exist_ok=False)
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return h.hexdigest()
def write(name,j):(out/name).write_text(json.dumps(j,indent=2)+'\n')
started=time.time();server=pathlib.Path('/home/acfrazier/274bot-campaign/server-4c95f87')
cache=cp.capture(server/'data/pack/client',pathlib.Path('/home/acfrazier/.274bot/unpack'));assert cache['snapshot_version']=='2faf336eeb0462ed';write('cache.json',cache)
unit=subprocess.check_output(['systemctl','show','274bot-concord-test-server.service','--property=MainPID','--value'],text=True).strip();pid=int(unit);sample=sr.sample_process(pid,timeout=2);assert sample['state']!='Z'
probe=r.probe_direct_server();write('server-sample.json',dict(pid=pid,sample=sample,probe=probe))
paths=[server/'data/config/world.json',server/'concord-fixture-manifest.json',server/'data/config/public.pem',server/'server-login-public.json',pathlib.Path('/home/acfrazier/.274bot/js-scripts.json')]
bindings=[dict(path=str(p),bytes=p.stat().st_size,sha256=sha(p)) for p in paths];write('file-bindings.json',bindings)
conditions=r.linux_preflight_snapshot();write('host-conditions.json',conditions)
commands=subprocess.check_output(['ps','-axo','pid=,comm='],text=True).splitlines();names={'stage-a-probe','cargo','rustc','heaptrack','tui-play','panel-play'}
conflicts=[line.strip() for line in commands if pathlib.Path(line.split(None,1)[1]).name in names];write('process-check.json',dict(matches=conflicts,scope='preliminary fixed process names only; not a typed conflict admission'))
result=dict(scope='pre-admission observations only; no account receipt, authorization, capture or performance claim',started_unix=started,ended_unix=time.time(),server_pid=pid,server_start_identity=sample['start_identity'],cache_snapshot_version=cache['snapshot_version'],cache_content_identity_sha256=cache['content_identity_sha256'],conflict_count=len(conflicts),files=[dict(path=p.name,sha256=sha(p)) for p in sorted(out.iterdir()) if p.is_file()],admitted=False)
write('result.json',result);print(json.dumps(result,indent=2))
