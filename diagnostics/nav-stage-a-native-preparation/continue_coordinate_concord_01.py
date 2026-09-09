"""Continue generated qualification after installing its missing real toolchain."""
from pathlib import Path
import hashlib,json,subprocess,sys,time
root=Path('/home/acfrazier/274bot-campaign/nav-coordinate-a44a930-concord-01');tool=root/'host/docs/memory/nav-tiled-stage-a';run=tool/'coordinate-concord-run-01'
binding='7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb'
assert hashlib.sha256((tool/'coordinate-source-binding.json').read_bytes()).hexdigest()==binding
old=json.loads((root/'root-concord-qualification-progress.json').read_text());assert len(old)==1 and old[0]['name']=='guards' and old[0]['returncode']==1
sys.path.insert(0,str(tool));import stage_a as s
s.configure_source_binding(tool/'coordinate-source-binding.json',binding)
for arm in s.ARMS:
 for variant in ['clean','counting']:s.verify_arm(run,arm,variant)
args=['--source-binding',str(tool/'coordinate-source-binding.json'),'--source-binding-sha256',binding]
py=sys.executable
commands=[('guards-after-toolchain',[py,'stage_a.py','guards','--run',str(tool/'coordinate-concord-native-guards-02'),*args]),('generated-clean',[py,'stage_a.py','generated','--run',str(run),'--variant','clean',*args]),('generated-counting',[py,'stage_a.py','generated','--run',str(run),'--variant','counting',*args]),('integration',[py,'test_stage_a.py','--run',str(run),*args]),('scheduler',[py,'qualify_sharded.py',str(run),'coordinate-ebf0f30-qualification-02',*args])]
progress=[]
for name,cmd in commands:
 print('START',name,flush=True);start=time.monotonic()
 with (root/(name+'-continuation.log')).open('x') as f:r=subprocess.run(cmd,cwd=tool,stdout=f,stderr=subprocess.STDOUT)
 progress.append({'name':name,'command':cmd,'returncode':r.returncode,'wall_seconds':time.monotonic()-start})
 (root/'root-concord-continuation-qualification-progress.json').write_text(json.dumps(progress,indent=2)+'\n');print(json.dumps(progress[-1]),flush=True)
 if r.returncode:raise SystemExit(r.returncode)
print('Generated Concord qualification complete; no private/performance release',flush=True)
