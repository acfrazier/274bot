"""Final two-suite retry after capture-only serde feature wiring correction."""
import json
import os
from pathlib import Path
import subprocess
OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[1]
results=[]
for package in ('host-play','tui'):
    command=['cargo','test','--offline','-p',package,'--features','memory-profile-no-alloc,memory-owner-capture','--','--test-threads=1']
    env=dict(os.environ, BOT_MEMORY_OWNER_CAPTURE='0')
    env.pop('LIVE', None)
    with (OUT/(package+'-on-final.log')).open('w') as stream:
        result=subprocess.run(command,cwd=ROOT,env=env,stdout=stream,stderr=subprocess.STDOUT,timeout=240)
    results.append({'name':package+'-on','command':command,'exit':result.returncode,'log':package+'-on-final.log'})
    (OUT/'final-retries.json').write_text(json.dumps(results,indent=2)+'\n')
    print(package,result.returncode,flush=True)
assert all(r['exit']==0 for r in results)
