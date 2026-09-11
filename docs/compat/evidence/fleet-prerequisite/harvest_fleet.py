"""Stream per-actor witnesses; --progress reports incomplete runs without accepting them."""
import hashlib,json,pathlib,statistics,sys
root=pathlib.Path(__file__).resolve().parents[4];folder=pathlib.Path(__file__).resolve().parent;d=folder/sys.argv[1];progress='--progress' in sys.argv;p=json.loads((d/'process.json').read_text());n=p['n'];q=[json.loads(l) for l in (d/'samples.qualification.jsonl').read_text().splitlines()];start=next((r['elapsed_s'] for r in q if r['phase']=='observe-start'),None);end=next((r['elapsed_s'] for r in q if r['phase']=='observe-end'),None);actors={};failures=[]
def near(r,x,z,rad):p=r['position'];return p[2]==0 and max(abs(p[0]-x),abs(p[1]-z))<=rad
def count(rt,field,name):return sum(x['count'] for x in rt.get(field,[]) if x['name']==name)
with (d/'samples.diagnostics.jsonl').open() as f:
 for line in f:
  try:v=json.loads(line)
  except json.JSONDecodeError:
   if progress:break
   raise
  if v['failure']:failures.append(v['failure'])
  for s in v['slots']:
   a=actors.setdefault(s['name'],dict(name=s['name'],quarters=[{} for _ in range(4)]));rt=s['runtime'];c=s.get('diagnostics',{}).get('client')
   if s['error']:failures.append(s['name']+': '+s['error'])
   if s['state']!='Running' or not c or 'inventory' not in rt:continue
   if s['seed']['status']=='Passed':a['seed_qualified']=True
   r=dict(elapsed_s=v['elapsed_s'],state=s['state'],tick=c['client_tick'],xp=c['xp'],hp=c['hp'],position=c['position'],ingame=c['ingame'],scene_state=c['scene_state'],food=count(rt,'inventory','Lobster'),coins=count(rt,'inventory','Coins'),bank_lobster=count(rt,'bank','Lobster'),bank_open=rt['bank_open'],bank_loaded=rt['bank_loaded'],last_completed_tick=rt['last_completed_tick'],dispatched=rt['dispatched'])
   a['last_running']=r
   if 'baseline' not in a and r['food']==4:a['baseline']=r
   if 'baseline' in a and 'bank_before' not in a and 0<r['food']<=3 and r['bank_lobster']==2000 and r['bank_open'] and r['bank_loaded'] and near(r,2656,3286,2):a['bank_before']=r
   if 'bank_before' in a and 'bank_after' not in a and r['bank_loaded'] and r['bank_lobster']==a['bank_before']['bank_lobster']-(22-a['bank_before']['food']):a['bank_after']=r
   if 'bank_before' in a and 'restock' not in a and r['food']==22 and near(r,2656,3286,3):a['restock']=r
   if 'restock' in a and 'return_arrival' not in a and r['elapsed_s']>a['restock']['elapsed_s'] and near(r,2661,3306,3):a['return_arrival']=r
   if 'return_arrival' in a and 'further_steals' not in a and r['elapsed_s']>=a['return_arrival']['elapsed_s'] and r['xp']>a['restock']['xp'] and r['coins']>a['restock']['coins'] and near(r,2661,3306,19):a['further_steals']=r
   if any('Held { name: "Lobster", action: "Eat" }'==x for x in s['diagnostics']['recent_requests']):a['observed_eat_request']=True
   if start is not None and r['elapsed_s']>=start and (end is None or r['elapsed_s']<=end):
    a.setdefault('observe_first',r);a['observe_last']=r;i=min(3,int((r['elapsed_s']-start)/150));a['quarters'][i].setdefault('first',r);a['quarters'][i]['last']=r
required=['baseline','bank_before','restock','return_arrival','further_steals','observe_first','observe_last'];missing={name:[k for k in required if k not in a] for name,a in actors.items()};missing={name:k for name,k in missing.items() if k};brief=dict(n=n,actors=len(actors),banked=sum('restock' in a for a in actors.values()),returned=sum('return_arrival' in a for a in actors.values()),further_steals=sum('further_steals' in a for a in actors.values()),bank_after_counts=sum('bank_after' in a for a in actors.values()),observation_complete=end is not None,failures=list(set(failures)),missing=missing)
if progress:print(json.dumps(brief,indent=2));sys.exit(0)
assert len(actors)==n and not failures and not missing,brief;assert start is not None and end is not None and end-start>=600
for a in actors.values():
 assert a.get('seed_qualified') and a.get('observed_eat_request');assert a['bank_before']['xp']>a['baseline']['xp'];assert a['observe_last']['xp']>a['observe_first']['xp'];assert a['observe_last']['last_completed_tick']>a['observe_first']['last_completed_tick'];assert a['observe_last']['xp']>=a['further_steals']['xp'];a['observation_xp_delta']=a['observe_last']['xp']-a['observe_first']['xp'];a['quarter_xp_deltas']=[v['last']['xp']-v['first']['xp'] if v else None for v in a.pop('quarters')]
for phase in q:
 assert len(phase['slots'])==n
 for s in phase['slots']:assert s['state']=='Running' and s['error'] is None and s['client']['ingame'] and s['client']['scene_state']==2
receipt=json.loads((d/'receipt.json').read_text());assert receipt['exit_code']==0 and not receipt['wall_timeout'];assert receipt['n']==n
for name,h in receipt['files'].items():assert hashlib.sha256((d/name).read_bytes()).hexdigest()==h,name
identity=json.loads((folder/'binary-6d750e65.json').read_text());assert receipt['binary_sha256']==identity['binary_sha256'];assert hashlib.sha256(pathlib.Path(identity['binary']).read_bytes()).hexdigest()==identity['binary_sha256'];m=json.loads((folder/'source-6d750e65.json').read_text());source=pathlib.Path(m['source_root'])
for name,h in m['files'].items():assert hashlib.sha256((source/name).read_bytes()).hexdigest()==h,name
metrics=[json.loads(l) for l in (d/'samples.jsonl').read_text().splitlines()];obs=[r for r in metrics if r['phase']=='observe'];assert obs and all(r['n']==n for r in obs);cpu=(obs[-1]['process_cpu_user_s']+obs[-1]['process_cpu_system_s']-obs[0]['process_cpu_user_s']-obs[0]['process_cpu_system_s'])/(obs[-1]['elapsed_s']-obs[0]['elapsed_s'])*100
summary=dict(outcome='PASS',n=n,revision=p['revision'],host_commit=p['host_commit'],client_commit=p['client_commit'],binary_sha256=p['binary_sha256'],scope='Sustained functional fleet qualification on this candidate; final integrated-source regression remains separate',observed_seconds=end-start,receipt=str(d.relative_to(root)/'receipt.json'),source_files_verified=len(m['files']),actors=actors,diagnostics=dict(rss_median_bytes=statistics.median(v['resident_bytes'] for v in obs),process_cpu_percent=cpu),limits=['1 Hz sampler can miss bank-after counts; actual food replenishment at loaded bank, observed return and further client XP/coins qualify each actor','Bank trips can occur in warmup; every actor must also progress throughout the observation window','Overlapping agent/build activity: diagnostic samples are not a matched performance comparison or savings claim'])
(d/'qualification.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(dict(brief,minimum_observation_xp_delta=min(a['observation_xp_delta'] for a in actors.values()),outcome='PASS'),indent=2))
