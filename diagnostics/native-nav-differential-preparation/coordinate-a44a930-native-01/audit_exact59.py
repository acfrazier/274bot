"""Independent full-byte and framed exact59 readback after the owned run."""
from pathlib import Path
from collections import Counter
import hashlib,json,time
root=Path(__file__).resolve().parent;run=root/'host/docs/memory/nav-tiled-differential/coordinate-native-generated-01';real=run/'real-release'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  while b:=f.read(1024**2):h.update(b)
 return h.hexdigest()
auth=root/'exact59-authorization.json';assert sha(auth)=='f8ed2150e9d2c0eda91fdf525165a09a94011c52f1d3504c75821cd0256b038d'
a=json.loads(auth.read_text());r=json.loads((real/'result.json').read_text());assert r['qualified']
assert sha(real/'input.bin')==a['input_sha256'] and (real/'input.bin').stat().st_size==73438581
assert sha(real/'routes.tsv')==a['routes_sha256']=='49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125'
expected=[[int(x) for x in line.split()] for line in (real/'routes.tsv').read_text().splitlines()];assert len(expected)==59
hashes={};summaries={}
for arm in ('dense','refined'):
 p=real/(arm+'.out');hashes[arm]=sha(p);receipt=json.loads((real/(arm+'.receipt.json')).read_text());assert receipt['returncode']==0 and receipt['failure'] is None and receipt['address_guard_active']
 assert receipt['limits']==dict(address=4294967296,cpu=800,file_size=4294967296,output=4294967296,rss=1073741824,wall=900)
 counts=Counter();rows=[];geometry=[];current=None
 with p.open('rb') as f:
  while line:=f.readline(256):
   assert line.endswith(b'\n');tag,n=line.decode().rstrip('\n').split(' ');n=int(n);assert 0<=n<=4*1024**3;counts[tag]+=1
   if tag in ('cells','blocked-words','canonical','round-canonical'):
    if tag=='cells':assert n==65142784*15
    if tag=='blocked-words':assert n==8142848
    f.seek(n,1);payload=None
   else:
    assert n<=16*1024**2;payload=f.read(n);assert len(payload)==n
   assert f.read(1)==b'\n'
   if tag=='geometry':geometry.append(payload.decode())
   elif tag=='fixed-selector':current={'row':json.loads(payload),'payload_sha256':{}};rows.append(current)
   elif current is not None and tag in ('post-state','fixed-model','fixed-tele-model','host-outcome','host-route','host-bank','host-final'):
    assert tag not in current['payload_sha256'];current['payload_sha256'][tag]=hashlib.sha256(payload).hexdigest()
 assert [x['row'] for x in rows]==expected
 assert geometry==['(WorldTile { x: 1856, z: 1280, level: 0 }, 1792, 9088, 65142784)']*2
 assert counts['cells']==counts['blocked-words']==2 and counts['complete-input-count']==1
 for row in rows:
  keys=row['payload_sha256'];assert all(k in keys for k in ('post-state','fixed-model','fixed-tele-model'))
  assert ('host-route' in keys)!=('host-outcome' in keys)
  if 'host-bank' in keys:assert keys['host-final']==keys['host-route']
 summaries[arm]=dict(frames=dict(counts),rows=rows,geometry=geometry)
assert hashes['dense']==hashes['refined'] and summaries['dense']==summaries['refined']
# Hash equality is supplemented by actual streamed byte comparison.
compared=0
with (real/'dense.out').open('rb') as x,(real/'refined.out').open('rb') as y:
 while True:
  b=x.read(1024**2);c=y.read(1024**2);assert b==c
  if not b:break
  compared+=len(b)
assert r['comparison']==dict(bytes=compared,equal=True)
report=dict(verified=True,scope='fresh exact59 real revision274 full-byte correctness only; no performance or live-action acceptance',verified_unix_s=time.time(),authorization_sha256=sha(auth),input_sha256=a['input_sha256'],routes_sha256=a['routes_sha256'],source_binding=a['source_binding'],comparison=r['comparison'],output_sha256=hashes,arms=summaries,limitations=['No StageA CPU/p99/RSS or StageB acceptance','Bank/teleport plans are not live action execution'])
(root/'exact59-root-audit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='arms'},indent=2))
