"""Final readback of frozen artifacts and explicit no-rebuild rejection."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import stage_a as s

run=s.HERE/'run-04'
verified=[]
for arm in s.ARMS:
    for variant in ('clean','counting'):
        b=s.verify_arm(run,arm,variant);verified.append({'arm':arm,'variant':variant,'sha256':s.sha(b)})
# Build action must stop before Cargo; existing admission/source/executable stay exact.
before={str(p):s.sha(p) for p in run.glob('*-admission.json')}
r=subprocess.run([sys.executable,str(s.HERE/'stage_a.py'),'build','--run',str(run),'--variant','clean'],capture_output=True,text=True,timeout=15)
assert r.returncode!=0 and 'admitted binary cannot be rebuilt' in r.stderr
assert before=={str(p):s.sha(p) for p in run.glob('*-admission.json')}
for arm in s.ARMS:
    for variant in ('clean','counting'):s.verify_arm(run,arm,variant)
inventory=json.loads((s.HERE/'evidence/run-04-archive.json').read_text())
archive=s.HERE/'evidence'/inventory['archive'];assert s.sha(archive)==inventory['sha256']
with tarfile.open(archive) as tar:
    assert len(tar.getmembers())==len(inventory['files'])
    for e in inventory['files']:
        f=tar.extractfile(e['path']);assert f is not None
        assert hashlib.sha256(f.read()).hexdigest()==e['sha256']
result=dict(qualified=True,verified_binaries=verified,archive_members=len(inventory['files']),archive_sha256=inventory['sha256'],rebuild_rejected_returncode=r.returncode,rebuild_stderr=r.stderr,
            scope='final generated-only readback; no real input executed')
s.save(s.HERE/'evidence/final-readback.json',result)
print(json.dumps(result,indent=2))
