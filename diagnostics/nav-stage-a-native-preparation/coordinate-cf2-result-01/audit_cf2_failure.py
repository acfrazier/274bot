"""Root raw evidence audit; never imports or reruns the measured scheduler."""
import argparse,hashlib,json,math,pathlib,tarfile
p=argparse.ArgumentParser();p.add_argument('--archive-sha256',required=True);args=p.parse_args()
root=pathlib.Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()
manifest=json.loads((root/'root-coordinate-cf2-evidence.manifest.json').read_text())
arc=root/'root-coordinate-cf2-evidence.tar.gz';assert sha(arc.read_bytes())==manifest['archive_sha256']==args.archive_sha256
expected={x['path']:x for x in manifest['files']};assert len(expected)==len(manifest['files']);data={};seen={}
with tarfile.open(arc,'r|gz') as t:
 for m in t:
  assert m.isfile() and m.name in expected and m.name not in seen
  h=hashlib.sha256();parts=[];count=0
  with t.extractfile(m) as f:
   while b:=f.read(1024*1024):
    h.update(b);count+=len(b)
    if not m.name.endswith('/input.bin'):parts.append(b)
  assert count==expected[m.name]['bytes'] and h.hexdigest()==expected[m.name]['sha256'];seen[m.name]=dict(bytes=count,sha256=h.hexdigest())
  if not m.name.endswith('/input.bin'):data[m.name]=b''.join(parts)
assert set(seen)==set(expected)
remote='/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01/'
base='host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-CF1-release-01/'
obj=lambda p:json.loads(data[p])
def ref(r):
 assert r['path'].startswith(remote)
 p=r['path'][len(remote):];assert sha(data[p])==r['sha256'];return obj(p)
auth=obj('root-coordinate-cf2-admission/CF2-authorization.json');assert sha(data['root-coordinate-cf2-admission/CF2-authorization.json'])=='1df23854e84d11decbdda2332b91623f4ed427b201dfdcd11b1d7513bf3c0a4a'
r=obj(base+'CF2/failure.json');prior=ref(auth['parent']);assert base+'CF2/result.json' not in data
assert r['status']=='failed' and r['phase']=='CF2' and r['completed']==57
assert r['error']=="ValueError('native idle/steal admission')"
checkpoint=obj(base+'CF2/checkpoint.json');claim=obj(base+'CF2/claim.json')
assert checkpoint['status']=='validated' and checkpoint['completed']==57
assert ref(claim['authorization'])==auth and claim['status']=='claimed'
r['entries']=[dict(name=name[:-12],record_sha256=sha(b)) for path,b in sorted(data.items()) if path.startswith(base+'CF2/') and (name:=path.split('/')[-1]).endswith('.record.json')]
assert len(r['entries'])==57
assert ref(r['authorization'])==auth and ref(r['root'])==ref(auth['root'])
rootauth=ref(r['root']);assert r['root']['sha256']=='62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec'
assert seen[base+'input.bin']==dict(bytes=73438581,sha256='2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30')
routes=(root.parent.parent/'nav-stage-a-real-proposal-01/routes.tsv').read_bytes();assert sha(routes)=='49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125'
rows=[b' '.join(str(int(x)).encode() for x in line.split())+b'\n' for line in routes.splitlines() if line.strip() and not line.lstrip().startswith(b'#')];assert len(rows)==59
for i,b in enumerate(rows,1):assert data[base+f'selectors/{i}.tsv']==b and sha(b)==rootauth['contract']['shard_sha256'][i-1]
phases=['startup','retained_input','decoded_converted_retained_input','dropped_input_world_live','hot_lookup','hot_routes','world_drop','post_drop']
limits=dict(address=4294967296,cpu=90,file_size=262144,output=262144,rss=1073741824,wall=120)
allchildren=[];aggregates={}
for phase,report,ordinals in [('CF1',prior,range(1,3)),('CF2',r,range(3,60))]:
 slots=[[1,row,arm] for row in ordinals for arm in (['dense','refined'] if row%2 else ['refined','dense'])]
 if phase=='CF2':slots=slots[:57]
 assert len(report['entries'])==len(slots)==report['completed'];out=base+phase+'/'
 for i,(entry,slot) in enumerate(zip(report['entries'],slots)):
  name=f'{i:03d}-1-{slot[1]}-{slot[2]}';assert entry['name']==name and sha(data[out+name+'.record.json'])==entry['record_sha256']
  record=obj(out+name+'.record.json');assert record['slot']==slot and record['phase']==phase and record['source_binding']==r['source_binding']
  assert set(record['files'])=={'.out','.err','.receipt.json'}
  for suffix,h in record['files'].items():assert sha(data[out+name+suffix])==h
  receipt=obj(out+name+'.receipt.json');assert receipt['returncode']==0 and receipt['failure'] is None and receipt['address_guard_active'] is True and receipt['limits']==limits
  assert 0<=receipt['wall_seconds']<=80 and 0<=record['child_cpu']<=60
  srows=[json.loads(line) for line in data[out+name+'.out'].splitlines()];assert len(srows)==9 and [s.get('phase') for s in srows[:-1]]==phases
  for s in srows[:-1]:
   for key in ('elapsed_ns','cpu_ns','since_start_ns','process_cpu_ns','current_rss_bytes','process_peak_rss_bytes'):assert type(s[key])is int and s[key]>=0
  s=srows[-1];assert s['summary'] is True and s['diagnostic'] is False and s['raw_order']=='sweep-row-lane/8/3' and s['raw_schema']=='stage-a-raw-v1'
  assert len(s['raw_elapsed_ns'])==24 and all(type(x)is int and x>=0 for x in s['raw_elapsed_ns'])
  assert s['route_p99_ns']==max(s['raw_elapsed_ns']) and len(s['lane_cpu_ns'])==3 and all(type(x)is int and x>=0 for x in s['lane_cpu_ns'])
  assert s['input_bytes']==73438581 and s['logical_cells']==65142784 and s['narrow_allocations']==s['narrow_requested_bytes']==0
  assert s['aggregate']['calls']==24 and s['aggregate']==aggregates.setdefault(slot[1],s['aggregate'])
  allchildren.append(dict(phase=phase,name=name,row=slot[1],arm=slot[2],child_cpu=record['child_cpu'],receipt_wall=receipt['wall_seconds'],peak_rss=srows[2]['process_peak_rss_bytes'],route_cpu=srows[5]['cpu_ns'],route_p99=s['route_p99_ns'],aggregate=s['aggregate']))
assert len(allchildren)==61 and len(aggregates)==31
budget=r['budget'];assert budget['children']==65 and budget['reserved_children']==budget['reserved_cpu']==budget['reserved_wall']==budget['unknown_cpu_charge']==0 and budget['stopped'] is False
assert budget['wall']<1800 and budget['cpu']<1500
assert checkpoint['budget']['children']==65 and checkpoint['budget']['wall']<=budget['wall'] and checkpoint['budget']['cpu']<=budget['cpu']
component_delta=budget['cpu']-sum(budget[k] for k in ('supervisor_cpu','waited_children_cpu','unknown_cpu_charge','publication_reserved_cpu'))
# Budget.snapshot samples its clocks separately; retain the observed difference.
assert abs(component_delta)<.001
priorbudget=prior['budget'];original=prior['prior_campaign_ledger']
fullfresh={k:budget[k]-original[k] for k in ('wall','cpu')};projection={k:7.5*v for k,v in fullfresh.items()}
result=dict(qualified=False,archive_and_partial_evidence_verified=True,failure=r['error'],scope='failed partial CF2 evidence only; no continuation or acceptance token',archive_sha256=args.archive_sha256,members=len(seen),completed=57,total_fresh_children=61,cumulative_budget=budget,original_budget=original,cf2_cost={k:budget[k]-priorbudget[k] for k in ('wall','cpu')},partial_fresh_feasibility_cost=fullfresh,acceptance_projection=None,cpu_component_sampling_delta_s=component_delta,waited_minus_cf2_probe_cpu_s=budget['waited_children_cpu']-priorbudget['waited_children_cpu']-sum(c['child_cpu'] for c in allchildren if c['phase']=='CF2'),children=allchildren)
(root/'root-failure-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k!='children'},indent=2))
