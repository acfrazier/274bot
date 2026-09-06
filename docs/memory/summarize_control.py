import json,pathlib,sys,statistics,math
root=pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0,str(root/'docs/memory'))
from cpu_screen import summarize,steals
run=pathlib.Path(sys.argv[1]).resolve()
meta=json.loads((run/'metadata.json').read_text());assert 'exit_code' in meta,'run still active'
rows=[json.loads(x) for x in (run/'samples.jsonl').read_text().splitlines()]
obs=[r for r in rows if r['phase']=='observe'];end=[r for r in rows if r['phase']=='teardown']
result={'run':run.name,'qualification':summarize(run,False,False),'meta':meta,'groups':[],'memory':{}}
assert len(obs)>1 and end
start,last=obs[0],obs[-1]
for field in ['resident_bytes','peak_resident_bytes','v8_used_bytes','v8_total_bytes','snapshot_inflight_bytes','snapshot_inflight_capacity','gpu_tracked_bytes','gpu_buffer_bytes','gpu_texture_bytes']:
 values=[r[field] for r in obs if r.get(field) is not None]
 if not values:continue
 result['memory'][field]={'first':values[0],'last':values[-1],'min':min(values),'median':statistics.median(values),'p95':sorted(values)[math.ceil(.95*len(values))-1],'max':max(values)}
result['resident_first_minute_median']=statistics.median(r['resident_bytes'] for r in obs if r['elapsed_s']<=start['elapsed_s']+60)
result['resident_last_minute_median']=statistics.median(r['resident_bytes'] for r in obs if r['elapsed_s']>=last['elapsed_s']-60)
result['teardown']={k:end[-1].get(k) for k in ['resident_bytes','peak_resident_bytes','ready','active','v8_live_isolates','v8_used_bytes','snapshot_inflight_bytes','snapshot_inflight_capacity','gpu_tracked_bytes']}
result['resident_last_teardown_30s_median']=statistics.median(r['resident_bytes'] for r in end if r['elapsed_s']>=end[-1]['elapsed_s']-30)
result['peak_resident_whole_run']=max(r['peak_resident_bytes'] for r in rows)
result['v8_sampled_isolates_min']=min(r['v8_sampled_isolates'] for r in obs)
result['v8_sample_age_max_ms']=max((r['v8_max_sample_age_ms'] for r in obs if r['v8_max_sample_age_ms'] is not None), default=None)
for i in range(2):
 a,b=start['scheduling'][i],last['scheduling'][i]
 g={'drawing':i==1,**{k:b[k]-a[k] for k in ['cycles','work_ns','requested_sleep_ns','actual_sleep_ns','work_overruns','intervals','interval_ns']}}
 for k in ['work_ns','requested_sleep_ns','actual_sleep_ns']:g[k.replace('_ns','_mean_ms')]=g[k]/g['cycles']/1e6 if g['cycles'] else None
 g['interval_mean_ms']=g['interval_ns']/g['intervals']/1e6 if g['intervals'] else None
 result['groups'].append(g)
for label,count,total in [('client_tick','client_tick_count','client_tick_total_ns'),('script_tick','script_tick_count','script_tick_total_ns'),('ui_frame','ui_frame_count','ui_frame_total_ns')]:
 n=last[count]-start[count];result[label+'_mean_ms']=(last[total]-start[total])/n/1e6 if n else None
proof=[json.loads(x) for x in (run/'samples.qualification.jsonl').read_text().splitlines()]
final=next(r for r in proof if r['phase']=='observe-end')
result['final_statuses']={s['name']:(s.get('runtime') or {}).get('paint') for s in final['slots']}
(run/'memory-summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k not in ['meta','final_statuses']},indent=2))
