from pathlib import Path
import hashlib,json,os,subprocess,sys,tarfile,time
root=Path(__file__).resolve().parent;m=json.loads((root/'manifest.json').read_text());assert hashlib.sha256((root/'source.tar.gz').read_bytes()).hexdigest()==m['archive_sha256'];assert not(root/'host').exists()
with tarfile.open(root/'source.tar.gz') as t:
 ms=t.getmembers();assert len(ms)==len(m['files']) and {x.name for x in ms}=={x['path'] for x in m['files']}
 for x in ms:assert x.isfile() and not Path(x.name).is_absolute() and '..' not in Path(x.name).parts
 t.extractall(root,filter='data')
for e in m['files']:
 b=(root/e['path']).read_bytes();assert len(b)==e['bytes'] and hashlib.sha256(b).hexdigest()==e['sha256']
host=root/'host';client=host/'vendor/fr-client-rust';client.mkdir(parents=True)
for repo,packs in [(host,['host.pack','coordinate-overlay.pack']),(client,['client.pack'])]:
 subprocess.run(['git','init',str(repo)],check=True,capture_output=True)
 for p in packs:
  with (root/'git-objects'/p).open('rb') as f:subprocess.run(['git','-C',str(repo),'index-pack','--stdin'],stdin=f,check=True,capture_output=True)
tool=host/'docs/memory/nav-tiled-differential';run=tool/'coordinate-native-generated-01';binding=host/'docs/memory/nav-tiled-stage-a/coordinate-source-binding.json';args=['--source-binding',str(binding),'--source-binding-sha256',m['binding_sha256']];py=sys.executable
commands=[('guards',[py,'harness.py','guards','--run',str(tool/'coordinate-native-guards-01'),*args]),('prepare',[py,'harness.py','prepare','--run',str(run),*args]),('build',[py,'harness.py','build','--run',str(run),*args]),('generated',[py,'harness.py','generated','--run',str(run),*args]),('audit',[py,'audit.py',str(run),*args]),('admission-extension-tests',[py,'-m','unittest','-v','test_admission','test_extension'])]
env=dict(os.environ,PATH='/home/builder/.cargo/bin:'+os.environ['PATH'],PYTHONDONTWRITEBYTECODE='1',NAV_EXTENSION_RUN=str(run),NAV_COORDINATE_SOURCE_BINDING=str(binding),NAV_COORDINATE_SOURCE_BINDING_SHA256=m['binding_sha256']);progress=[]
for name,cmd in commands:
 print('START',name,flush=True);start=time.monotonic()
 with (root/(name+'.log')).open('x') as f:r=subprocess.run(cmd,cwd=tool,env=env,stdout=f,stderr=subprocess.STDOUT)
 progress.append({'name':name,'command':cmd,'returncode':r.returncode,'wall_seconds':time.monotonic()-start});(root/'progress.json').write_text(json.dumps(progress,indent=2)+'\n');print(json.dumps(progress[-1]),flush=True)
 if r.returncode:raise SystemExit(r.returncode)
print('Generated full-byte differential qualification complete; no private input',flush=True)
