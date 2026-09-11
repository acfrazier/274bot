"""Qualify observed bank/restock/return and sustained work, not process exit alone."""
import hashlib,json,pathlib,statistics
root=pathlib.Path(__file__).resolve().parents[4];folder=pathlib.Path(__file__).resolve().parent
manifest=json.loads((folder/'source-6d750e65.json').read_text());source=pathlib.Path(manifest['source_root'])
for p,h in manifest['files'].items():assert hashlib.sha256((source/p).read_bytes()).hexdigest()==h,p
identity=json.loads((folder/'binary-6d750e65.json').read_text());assert identity['isolated_build'];assert hashlib.sha256(pathlib.Path(identity['binary']).read_bytes()).hexdigest()==identity['binary_sha256']
def count(row,where,name):return sum(v['count'] for v in row['slot']['runtime'].get(where,[]) if v['name']==name)
def client(row):return row['slot']['diagnostics']['client']
def near(row,x,z,r):p=client(row)['position'];return p[2]==0 and max(abs(p[0]-x),abs(p[1]-z))<=r
def compact(row):
 s=row['slot'];return dict(elapsed_s=row['elapsed_s'],actor=s['name'],state=s['state'],client=client(row),food=count(row,'inventory','Lobster'),coins=count(row,'inventory','Coins'),bank_lobster=count(row,'bank','Lobster'),bank_open=s['runtime'].get('bank_open'),bank_loaded=s['runtime'].get('bank_loaded'),paint=s['runtime'].get('paint'),dispatched=s['runtime']['dispatched'],last_completed_tick=s['runtime']['last_completed_tick'])
results=[]
for rev in ['274','289']:
 d=folder/f'n1-r{rev}-6d750e65';receipt=json.loads((d/'receipt.json').read_text());assert receipt['exit_code']==0 and not receipt['wall_timeout'];assert receipt['binary_sha256']==identity['binary_sha256']
 for p,h in receipt['files'].items():assert hashlib.sha256((d/p).read_bytes()).hexdigest()==h,p
 raw=[json.loads(v) for v in (d/'samples.diagnostics.jsonl').read_text().splitlines()];q=[json.loads(v) for v in (d/'samples.qualification.jsonl').read_text().splitlines()];assert [v['phase'] for v in q]==['observe-start','observe-end'];start,end=[v['elapsed_s'] for v in q];assert end-start>=600
 names={s['name'] for v in raw for s in v['slots']};assert len(names)==1
 for v in q:
  assert len(v['slots'])==1;s=v['slots'][0];assert s['state']=='Running' and s['error'] is None and s['client']['ingame'] and s['client']['scene_state']==2
 rows=[dict(elapsed_s=v['elapsed_s'],slot=v['slots'][0]) for v in raw if v['slots'][0]['state']=='Running' and v['slots'][0]['seed']['status']=='Passed' and 'inventory' in v['slots'][0]['runtime']]
 assert all(v['failure'] is None for v in raw);assert all(r['slot']['error'] is None for r in rows)
 baseline=next(r for r in rows if count(r,'inventory','Lobster')==4)
 bank=next(r for r in rows if r['elapsed_s']>baseline['elapsed_s'] and count(r,'inventory','Lobster')==3 and count(r,'bank','Lobster')==2000 and r['slot']['runtime'].get('bank_open') and r['slot']['runtime'].get('bank_loaded') and near(r,2656,3286,2))
 restock=next(r for r in rows if r['elapsed_s']>bank['elapsed_s'] and count(r,'inventory','Lobster')==22 and near(r,2656,3286,3))
 returned=next(r for r in rows if r['elapsed_s']>restock['elapsed_s'] and near(r,2661,3306,3) and client(r)['xp']>client(restock)['xp'] and count(r,'inventory','Coins')>count(restock,'inventory','Coins'))
 assert client(bank)['xp']>client(baseline)['xp'];assert any('Held { name: "Lobster", action: "Eat" }' in r['slot']['diagnostics']['recent_requests'] for r in rows if r['elapsed_s']<=bank['elapsed_s'])
 assert all(client(r)['ingame'] and client(r)['scene_state']==2 for r in [baseline,bank,restock,returned])
 windows=[]
 for i in range(4):
  part=[r for r in rows if start+i*150<=r['elapsed_s']<min(end,start+(i+1)*150)];assert part;delta=client(part[-1])['xp']-client(part[0])['xp'];assert delta>0,(rev,i,delta);windows.append(dict(start_s=part[0]['elapsed_s'],end_s=part[-1]['elapsed_s'],xp_delta=delta))
 observed=[r for r in rows if start<=r['elapsed_s']<=end];last=observed[-1];assert client(last)['xp']>client(returned)['xp'];assert last['slot']['runtime']['last_completed_tick']>baseline['slot']['runtime']['last_completed_tick']
 bank_after=[r for r in rows if r['elapsed_s']>=bank['elapsed_s'] and count(r,'bank','Lobster')==1981];metrics=[json.loads(v) for v in (d/'samples.jsonl').read_text().splitlines()];obs=[v for v in metrics if v['phase']=='observe'];assert obs
 cpu=(obs[-1]['process_cpu_user_s']+obs[-1]['process_cpu_system_s']-obs[0]['process_cpu_user_s']-obs[0]['process_cpu_system_s'])/(obs[-1]['elapsed_s']-obs[0]['elapsed_s'])*100
 results.append(dict(revision=rev,outcome='PASS',scope='N=1 sustained functional prerequisite',receipt=str(d.relative_to(root)/'receipt.json'),actor=next(iter(names)),observed_seconds=end-start,baseline=compact(baseline),bank_before=compact(bank),restock=compact(restock),returned_with_further_steals=compact(returned),last_observed=compact(last),bank_decrement_witness=compact(bank_after[0]) if bank_after else None,positive_xp_windows=windows,observation_xp_delta=client(last)['xp']-client(observed[0])['xp'],diagnostic_rss_median_bytes=statistics.median(v['resident_bytes'] for v in obs),diagnostic_cpu_percent=cpu,limits=['Overlapping Grok/build/live work; no matched performance or savings claim','One bank/restock/return trip; four positive 150-second XP windows','1 Hz runtime sampler missed 289 bank-after count; actual 3-to-22 food increase at bank and subsequent return/XP are observed','289 bank trip occurs during warmup; sustained work is observed afterward through 600-second window']))
summary=dict(host_commit=identity['host_commit'],client_commit=identity['client_commit'],binary_sha256=identity['binary_sha256'],source_files_verified=len(manifest['files']),catalog_commit='8e7d965be2071d6ec65c3265e12af797082d720a',script_sha256='5cff25a75db3166a3bf3ab6ca1a8ba37ed77e0e9d48292178f79767316b66ad9',results=results,next_gate='N32 must independently prove every actor; N1 is not a fleet pass')
(folder/'n1-results-6d750e65.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps([dict(revision=r['revision'],outcome=r['outcome'],xp_delta=r['observation_xp_delta'],positive_xp_windows=r['positive_xp_windows']) for r in results],indent=2))
