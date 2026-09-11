from pathlib import Path
import json,hashlib,subprocess,re
root=Path.cwd();ev=root/'docs/compat/evidence/stage-1';client=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root/'vendor/fr-client-rust',text=True).strip();head=subprocess.check_output(['git','rev-parse','940531c3f'],text=True).strip();rows=[]
for rev in ['274','289']:
 for case in ['boundary','nav_full','nav_door','guardian_lamp']:
  folder=ev if rev=='274' and case in ['boundary','nav_full'] else ev/'fixture-corrections';p=folder/f'r{rev}-{case}.json';r=json.loads(p.read_text());log=p.with_suffix('.log');text=log.read_text();assert r['exit_code']==0 and r['client_commit']==client and hashlib.sha256(log.read_bytes()).hexdigest()==r['log_sha256']
  if case=='boundary':
   events=[json.loads(l) for l in text.splitlines() if l.startswith('{"phase"')];phases={e['phase']:e for e in events}
   for name in ['baseline-after-preparation','walk-applied','npc-dialogue-applied','loc-change-applied']:assert phases[name]['state']['ingame'] and phases[name]['state']['scene_state']==2
   end=phases['logout-reset-applied'];assert end['detail']['outcome']=='PASS' and not end['state']['ingame'] and end['state']['tile'] is None and end['state']['npc_count']==end['state']['loc_count']==0
   proof={'observed_phases':['walk-applied','npc-dialogue-applied','loc-change-applied','logout-reset-applied'],'logout_cleared':True}
  else:
   line=next(l for l in text.splitlines() if l.startswith('PASS: '+case+': '));proof=json.loads(line.split(': ',2)[2])
   if case=='guardian_lamp':assert all(proof[k] for k in ['interface','hold','consumed','xp_gain']) and proof['resumed_from']!=proof['walk_to']
   elif case=='nav_door':assert proof['before_door'][1]==1530 and proof['after_door'][1]==1531 and proof['traveller_runner']['outcome']=='PASS' and proof['traveller_runner']['tile']==[2817,3443,0]
   else:assert proof['engine_speed_ms'] is None and proof['evidence']['outcome']=='PASS'
  rows.append({'revision':int(rev),'case':case,'receipt':str(p.relative_to(root)),'log_sha256':r['log_sha256'],'host_commit':r['host_commit'],'observed':proof})
assert len(rows)==8
checks=json.loads((ev/'checks-rest.json').read_text());assert checks['all_passed'];initial=json.loads((ev/'checks.json').read_text());assert all(r['exit_code']==0 for r in initial['checks'][:2]);focused=json.loads((ev/'fixture-corrections/focused-checks.json').read_text());assert all(r['exit_code']==0 for r in focused['checks'])
source_delta=subprocess.check_output(['git','diff','--name-only','65938065','940531c3f'],text=True).splitlines();assert source_delta==['crates/host-play/tests/world_boundary_live.rs']
d=dict(ready_for_final_review=True,approved_for_publication=False,host_code_candidate=head,client_commit=client,checks={'workspace_passed':2603,'workspace_explicit_baseline_exclusions':['bright_catalog_cards_start_without_not_impl','declared_abi_fixture_matches_local_dts'],'client_tests_passed':1007,'format_and_strict_clippy':True,'fixture_followup_format_clippy':True},checks_from_candidate='659380651869bd12fc9d281a30037b3b0368b380',later_changes='Only three historical fixture corrections in world_boundary_live.rs; no product/client changes. Corrected door/lamp both revisions requalified; 289 boundary/nav also ran afterward. Prior 274 boundary/nav retained because unaffected.',source_delta=source_delta,live_passed=8,live=rows,retained_failures=['docs/compat/evidence/stage-1/baseline-catalog-start.log','docs/compat/evidence/stage-1/host-tests.log','docs/compat/evidence/stage-1/baseline-declared-abi.log','docs/compat/evidence/stage-1/host-tests-remaining.log','docs/compat/evidence/stage-1/r274-nav_door.log'],remaining_gate='Independent branchreviewer verdict, then root publication and fresh recursive checkout verification. No release/performance/full-catalog acceptance.')
(ev/'validation-ready.json').write_text(json.dumps(d,indent=2)+'\n');print({'live_passed':len(rows),'host':head,'client':client})
