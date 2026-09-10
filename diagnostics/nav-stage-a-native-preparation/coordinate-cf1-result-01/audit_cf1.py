import hashlib,json,pathlib,tarfile,math
root=pathlib.Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()
m=json.loads((root/'root-coordinate-cf1-evidence.manifest.json').read_text())
arc=root/'root-coordinate-cf1-evidence.tar.gz';assert sha(arc.read_bytes())==m['archive_sha256']=='6b6314fe9e60235ab92feeb65f972921104c66bccbc5fabcf385e4e28f576cdf'
expected={r['path']:r for r in m['files']};data={}
with tarfile.open(arc) as t:
 for p in t:
  assert p.isfile() and p.name in expected and p.name not in data
  b=t.extractfile(p).read();e=expected[p.name]
  assert len(b)==e['bytes'] and sha(b)==e['sha256'];data[p.name]=b
assert set(data)==set(expected)
remote='/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01/'
base='host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-CF1-release-01/'
out=base+'CF1/'
obj=lambda p:json.loads(data[p])
def ref(r):
 assert r['path'].startswith(remote)
 p=r['path'][len(remote):];assert sha(data[p])==r['sha256'];return obj(p)
r=obj(out+'result.json');auth=ref(r['authorization']);assert ref(r['root'])==auth
assert sha(data['root-coordinate-cf1-admission/CF1-authorization.json'])=='62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec'
assert r['status']=='complete' and r['completed']==4 and r['phase']=='CF1'
assert sha(data[base+'input.bin'])==auth['prerequisites']['input_sha256']
assert len(data[base+'input.bin'])==73438581
rows=(root.parent/'coordinate-cf1-preparation-01'/'CF1-authorization.json').read_bytes();assert sha(rows)==r['authorization']['sha256']
routefile=root.parent.parent/'nav-stage-a-real-proposal-01'/'routes.tsv'
routebytes=routefile.read_bytes();assert sha(routebytes)==auth['prerequisites']['routes_sha256']
route_rows=[b' '.join(str(int(x)).encode() for x in line.split())+b'\n' for line in routebytes.splitlines() if line.strip() and not line.lstrip().startswith(b'#')]
assert len(route_rows)==59
for i,b in enumerate(route_rows,1):assert data[base+f'selectors/{i}.tsv']==b
slots=[[1,1,'dense'],[1,1,'refined'],[1,2,'refined'],[1,2,'dense']];children=[];aggregates={}
for i,(entry,slot) in enumerate(zip(r['entries'],slots)):
 name=f'{i:03d}-{slot[0]}-{slot[1]}-{slot[2]}';assert entry['name']==name
 assert sha(data[out+name+'.record.json'])==entry['record_sha256']
 record=obj(out+name+'.record.json');assert record['slot']==slot and record['source_binding']==r['source_binding']
 for suffix,h in record['files'].items():assert sha(data[out+name+suffix])==h
 receipt=obj(out+name+'.receipt.json');assert receipt['returncode']==0 and receipt['failure'] is None and receipt['address_guard_active'] is True
 assert receipt['limits']==dict(address=4294967296,cpu=90,file_size=262144,output=262144,rss=1073741824,wall=120)
 samples=[json.loads(l) for l in data[out+name+'.out'].splitlines()];s=samples[-1]
 assert len(samples)==9 and s['summary'] is True and s['raw_order']=='sweep-row-lane/8/3'
 assert len(s['raw_elapsed_ns'])==24 and all(type(x)is int and x>=0 for x in s['raw_elapsed_ns'])
 assert s['narrow_allocations']==s['narrow_requested_bytes']==0
 assert s['input_bytes']==73438581 and s['logical_cells']==65142784
 assert s['aggregate']==aggregates.setdefault(slot[1],s['aggregate'])
 children.append(dict(name=name,child_cpu=record['child_cpu'],receipt_wall=receipt['wall_seconds'],peak_rss=s['process_peak_rss_bytes'],raw_calls=len(s['raw_elapsed_ns']),aggregate=s['aggregate']))
b=r['budget'];prior=r['prior_campaign_ledger'];checkpoint=ref(r['checkpoint']);ref(r['claim'])
assert b['children']==8 and prior['children']==4 and not b['stopped']
assert b['reserved_children']==b['reserved_cpu']==b['reserved_wall']==b['unknown_cpu_charge']==0
cpu_component_delta=b['cpu']-b['supervisor_cpu']-b['waited_children_cpu']-b['publication_reserved_cpu']
assert abs(cpu_component_delta)<0.001  # snapshot samples CPU clocks separately
child_cpu_delta=b['waited_children_cpu']-prior['waited_children_cpu']-sum(x['child_cpu'] for x in children)
assert b['wall']<1800 and b['cpu']<1500 and checkpoint['completed']==4
result=dict(qualified=True,scope='CF1 raw artifact and ledger verification only; no CF2 or performance acceptance',archive_sha256=m['archive_sha256'],members=len(data),cpu_component_sampling_delta_s=cpu_component_delta,waited_minus_recorded_child_cpu_s=child_cpu_delta,children=children,cumulative_budget=b,prior_budget=prior,fresh_wall=b['wall']-prior['wall'],fresh_cpu=b['cpu']-prior['cpu'])
(root/'root-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
