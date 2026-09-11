"""Independently qualify both exact-source N2 runs from full raw boundaries."""
import datetime,hashlib,json,pathlib,subprocess
root=pathlib.Path(subprocess.check_output(['git','rev-parse','--show-toplevel'],cwd=pathlib.Path(__file__).parent,text=True).strip());e=root/'docs/compat/evidence/two-slot-isolation';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();m=json.loads((root/'docs/compat/evidence/catalog-headed/source-3383b099.json').read_text());b=json.loads((e/'binary-3383b099.json').read_text());assert b['isolated_build'] and b['target_started_empty'] and sha(pathlib.Path(b['binary']))==b['binary_sha256']
for name,h in m['files'].items():assert sha(pathlib.Path(m['source_root'])/name)==h,name
rows=[]
for revision in [274,289]:
 folder=e/f'r{revision}-3383b099';r=json.loads((folder/'receipt.json').read_text());log=folder/'run.log';assert r['exit_code']==0 and r['binary_sha256']==b['binary_sha256'] and sha(log)==r['log_sha256'];w=json.loads(next(line for line in log.read_text().splitlines() if line.startswith('{"phase":"witness"')))['witness'];a,c=w['a'],w['b'];assert a['account']!=c['account'] and a['settings']!=c['settings'];assert a['kind']=='chainbody' and c['kind']=='scimitar'
 for slot in [a,c]:
  assert slot['post_start']>0 and not slot['mixed_identity'] and not slot['foreign_seen'];start=slot['baseline'];first=slot['first'];assert start['magic_xp']==166636 and start['magic']==55 and start['coins']==start['own_noted']==start['own_unnoted']==start['natures']==0
  assert slot['peak_own']==slot['peak_natures']==27 and first['tick']>start['tick'] and first['magic_xp']>start['magic_xp'] and first['coins']>0 and first['own_noted']==first['natures']==26
  for key in ['baseline','first','drain','pause_end','stop_boundary','after_stop']:
   obs=slot[key];assert obs['ingame'] and obs['scene_state']==2 and obs['inventory_tab_available'] and obs['foreign_noted']==obs['foreign_unnoted']==0
 assert [w[k] for k in ['pause_a','pause_b','resume_a','stop_a','stop_b']]==['paused','running','running','idle','running']
 for key in ['magic_xp','coins','natures','own_noted','own_unnoted']:
  assert a['drain'][key]==a['pause_end'][key] and a['stop_boundary'][key]==a['after_stop'][key]
 assert a['pause_end']['tick']>a['drain']['tick'] and a['after_stop']['tick']>a['stop_boundary']['tick']
 for slot,before,after in [(c,'drain','pause_end'),(a,'pause_end','further'),(c,'stop_boundary','after_stop')]:
  x,y=slot[before],slot[after];assert y['tick']>x['tick'] and y['magic_xp']>x['magic_xp'] and y['coins']>x['coins'] and y['natures']<x['natures'] and y['own_noted']<x['own_noted']
 x,y=c['stop_boundary'],c['after_stop'];assert y['magic_xp']-x['magic_xp']==65 and y['coins']-x['coins']==1536 and x['natures']-y['natures']==x['own_noted']-y['own_noted']==1
 rows.append(dict(revision=revision,outcome='PASS',receipt=str((folder/'receipt.json').relative_to(root)),log_sha256=r['log_sha256'],elapsed_seconds=r['elapsed_seconds'],post_stop_peer_delta=dict(ticks=y['tick']-x['tick'],magic_xp=65,coins=1536,nature_runes_consumed=1,own_noted_items_consumed=1),witness=w))
result=dict(verified_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),host_commit=b['host_commit'],client_commit=b['client_commit'],source_files_verified=len(m['files']),rows=rows,scope='Controlled host-play N2 component isolation: distinct accounts/settings/items, actual work, Pause stability after drain with peer work, Resume progress, Stop stability and fresh peer work after actual Stop. Separate from native/TUI controls, recovery, N32 and final integrated-source acceptance.',performance_claim=False);(e/'qualification-3383b099.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps([dict(revision=r['revision'],outcome=r['outcome'],post_stop_peer_delta=r['post_stop_peer_delta']) for r in rows]))
