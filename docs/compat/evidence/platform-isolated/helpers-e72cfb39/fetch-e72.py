import pathlib,subprocess,shutil,json,hashlib,datetime
root=pathlib.Path(__file__).resolve().parents[2];base=root/'.superpowers/platform-preparation';ev=root/'docs/compat/evidence/platform-isolated'
for surface,host,remote in [('mac',None,str(base/'platform-isolated-e72cfb39')),('linux','274bot-builder','/home/builder/274bot-campaign/multirevision-20260910/platform-isolated-e72cfb39')]:
 dest=ev/(surface+'-e72cfb39');dest.mkdir(exist_ok=True)
 for name in ['platform-build.json','verified-source.json','build-platform_proof.log','build-platform_proof.jsonl']:
  target=dest/name;assert not target.exists()
  if host:subprocess.run(['scp','-o','BatchMode=yes','-o','ConnectTimeout=10',host+':'+remote+'/'+name,str(target)],check=True)
  else:shutil.copy2(pathlib.Path(remote)/name,target)
for surface,revision,host,remote in [('mac',274,None,str(base/'platform-isolated-e72cfb39')),('concord',289,'concord','/home/acfrazier/274bot-campaign/multirevision-20260910/platform-isolated-e72cfb39')]:
 dest=ev/(surface+'-tui-e72cfb39')/('r'+str(revision)+'-params');dest.mkdir(exist_ok=True,parents=True)
 for name in ['process.json','receipt.json','terminal.log','stderr.log','control.jsonl','input-forwarded.jsonl']:
  target=dest/name;assert not target.exists();source=remote+'/live/r'+str(revision)+'-alcher-params/'+name
  if host:subprocess.run(['scp','-o','BatchMode=yes','-o','ConnectTimeout=10',host+':'+source,str(target)],check=True)
  else:shutil.copy2(pathlib.Path(source),target)
 r=json.loads((dest/'receipt.json').read_text());assert r['exit_code']==0 and r['pass_after_terminal_restore'];assert hashlib.sha256((dest/'terminal.log').read_bytes()).hexdigest()==r['terminal_sha256'];assert hashlib.sha256((dest/'stderr.log').read_bytes()).hexdigest()==r['stderr_sha256'];print(surface,revision,r['elapsed_seconds'],r['input_forwarded_count'],flush=True)
