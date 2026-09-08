#!/usr/bin/env python3
import hashlib, json, statistics
from pathlib import Path

BASE = Path(__file__).resolve().parents[2]
ROOT = BASE / 'diagnostics/windows-lazy-upload-20260908'
CELLS = {
    'baseline-focused-one-20260908-0102': ('baseline', 'focused-one'),
    'baseline-focused-plus-background-20260908-0111': ('baseline', 'focused-plus-background'),
    'candidate-focused-one-20260908-0118': ('candidate', 'focused-one'),
    'candidate-focused-plus-background-20260908-0126': ('candidate', 'focused-plus-background'),
}
EXPECTED_TAR = {
 'baseline-focused-one-20260908-0102':'637b594951873aa01f05977bd4c654282cd0630313db868e573347f6688a5694',
 'baseline-focused-plus-background-20260908-0111':'306d937d513eaad0136ad59d737672115202609c414c3092c94c4058d8e26e46',
 'candidate-focused-one-20260908-0118':'6ac66be419307c9cba1f02d43a07734a6dd682ae1dd0d16b36b5ec9dcaf9ca8a',
 'candidate-focused-plus-background-20260908-0126':'e10791a380f8404030c194662bd0f50270dda4210152fe7dc83b1181e696bd79',
}

def lines(p): return [json.loads(x) for x in p.read_text(encoding='utf-8-sig').splitlines() if x.strip()]
def mib(x): return x / 1048576 if x is not None else None
def vals(rows, k): return [r[k] for r in rows if isinstance(r.get(k), (int, float))]
def all_keys(obj, out):
 if isinstance(obj, dict):
  out.update(obj)
  for v in obj.values(): all_keys(v, out)
 elif isinstance(obj, list):
  for v in obj: all_keys(v, out)
def nested(obj, key):
 if isinstance(obj, dict):
  if key in obj: yield obj[key]
  for v in obj.values(): yield from nested(v, key)
 elif isinstance(obj, list):
  for v in obj: yield from nested(v, key)
def first(obj, key): return next(nested(obj, key), None)
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024), b''): h.update(b)
 return h.hexdigest()

def main():
 table={'schema':'windows-lazy-upload-comparison-v1', 'protocol_sha256':sha(BASE/'docs/memory/windows-lazy-upload-comparison-protocol.md'), 'cells':{}, 'pairs':{}}
 for name,(role,mode) in CELLS.items():
  d=ROOT/name; raw=lines(d/'raw-run-01/samples.jsonl'); qlines=lines(d/'raw-run-01/samples.qualification.jsonl')
  bind=json.loads((ROOT/(name+'-binding.json')).read_text(encoding='utf-8-sig'))
  qstart=next((r.get('elapsed_s') for r in qlines if r.get('phase')=='observe-start'), None)
  qend=next((r.get('elapsed_s') for r in qlines if r.get('phase')=='observe-end'), None)
  obs=[r for r in raw if qstart is not None and qend is not None and qstart <= r.get('elapsed_s',-1) <= qend]
  if not obs: obs=raw
  rss=vals(obs,'resident_bytes'); peaks=vals(raw,'peak_resident_bytes'); elapsed=vals(obs,'elapsed_s')
  renders=[x for r in obs for x in (r.get('renderer_profile') or [])]
  render_keys=set(); all_keys(renders,render_keys)
  statuses={}
  for x in renders:
   for k in ('renderer_present','backend','draw','full_rate','completed','gpu_completed','paint_count','callback_count','render_completed'):
    if k in x: statuses.setdefault(k,{}).setdefault(str(x[k]),0); statuses[k][str(x[k])]+=1
  counter_max={}
  for x in renders:
   for k,v in x.items():
    if isinstance(v,(int,float)) and any(s in k for s in ('completed','paint','callback','completion')):
     counter_max[k]=max(counter_max.get(k,0),v)
  phase_counts={}
  for r in raw: phase_counts[r.get('phase','<missing>')]=phase_counts.get(r.get('phase','<missing>'),0)+1
  ready=[r.get('ready') for r in obs if isinstance(r.get('ready'),int)]; active=[r.get('active') for r in obs if isinstance(r.get('active'),int)]
  gpu=vals(obs,'gpu_tracked_bytes'); user=vals(obs,'process_cpu_user_s'); system=vals(obs,'process_cpu_system_s')
  # manifest's listed hashes are independently checked against the archived files.
  am=json.loads((d/'archive-manifest.json').read_text(encoding='utf-8-sig'))
  checks=[]
  for item in am.get('files',[]):
   rel=item['path'].replace('\\','/'); p=d/rel
   checks.append({'path':rel,'listed_sha256':item['sha256'],'actual_sha256':sha(p) if p.exists() else None,'ok':p.exists() and sha(p)==item['sha256'],'listed_length':item.get('length'),'actual_length':p.stat().st_size if p.exists() else None})
  tar=d.with_suffix('.tar.gz')
  table['cells'][name]={
   'role':role,'mode':mode,'archive_tar_sha256':sha(tar) if tar.exists() else None,'archive_tar_expected':EXPECTED_TAR[name],
   'archive_tar_ok':tar.exists() and sha(tar)==EXPECTED_TAR[name],'archive_manifest_file_count':len(checks),'archive_manifest_files_ok':all(x['ok'] for x in checks),
   'qualification_binding':{k:bind.get(k) for k in ('status','binding_ok','pair_eligible','final_acceptance_claim','performance_acceptance') if k in bind},
   'raw_rows':len(raw),'qualification_rows':len(qlines),'observation_rows':len(obs),'observe_bounds_s':[qstart,qend],
   'elapsed_observation_s':(elapsed[-1]-elapsed[0]) if len(elapsed)>1 else None,
   'rss_median_mib':mib(statistics.median(rss)) if rss else None,'rss_first_mib':mib(rss[0]) if rss else None,'rss_last_mib':mib(rss[-1]) if rss else None,'rss_min_mib':mib(min(rss)) if rss else None,'rss_max_mib':mib(max(rss)) if rss else None,
   'peak_max_mib':mib(max(peaks)) if peaks else None,
   'cpu_user_delta_s':user[-1]-user[0] if len(user)>1 else None,'cpu_system_delta_s':system[-1]-system[0] if len(system)>1 else None,
   'cpu_core_estimate':((user[-1]-user[0])+(system[-1]-system[0]))/(elapsed[-1]-elapsed[0]) if len(user)>1 and len(system)>1 and len(elapsed)>1 else None,
   'ready_min_max':[min(ready),max(ready)] if ready else None,'active_min_max':[min(active),max(active)] if active else None,
   'client_tick_delta':vals(obs,'client_tick_count')[-1]-vals(obs,'client_tick_count')[0] if len(vals(obs,'client_tick_count'))>1 else None,
   'ui_draw_delta':vals(obs,'ui_draw_count')[-1]-vals(obs,'ui_draw_count')[0] if len(vals(obs,'ui_draw_count'))>1 else None,'ui_frame_delta':vals(obs,'ui_frame_count')[-1]-vals(obs,'ui_frame_count')[0] if len(vals(obs,'ui_frame_count'))>1 else None,
   'gpu_tracked_mib_median':mib(statistics.median(gpu)) if gpu else None,'gpu_tracked_mib_max':mib(max(gpu)) if gpu else None,
   'renderer_observation_rows':len(renders),'renderer_status_counts':statuses,'renderer_key_inventory':sorted(render_keys),
   'renderer_counter_max':counter_max,'raw_phase_counts':phase_counts,
   'qualification_selected':{k:first(bind,k) for k in ('client_ticks_per_slot_s','steal_gains','observe_sample_n','simulation_intervals','simulation_ticks','render_completed_gpu','background_paint_cadence_hz','focused_paint_cadence_hz') if first(bind,k) is not None},
   'file_checks':checks,
  }
 # paired descriptive deltas, not acceptance verdicts
 for mode in ('focused-one','focused-plus-background'):
  b=table['cells'][next(n for n,(r,m) in CELLS.items() if r=='baseline' and m==mode)]; c=table['cells'][next(n for n,(r,m) in CELLS.items() if r=='candidate' and m==mode)]
  def delta(k): return c[k]-b[k] if isinstance(c.get(k),(int,float)) and isinstance(b.get(k),(int,float)) else None
  table['pairs'][mode]={k:delta(k) for k in ('rss_median_mib','rss_first_mib','rss_last_mib','rss_min_mib','rss_max_mib','peak_max_mib','cpu_core_estimate','gpu_tracked_mib_max','client_tick_delta','ui_draw_delta','ui_frame_delta')}
 Path(__file__).with_name('table.json').write_text(json.dumps(table,indent=2,sort_keys=True)+'\n',encoding='utf-8')
 print(json.dumps(table,indent=2,sort_keys=True))
if __name__=='__main__': main()
