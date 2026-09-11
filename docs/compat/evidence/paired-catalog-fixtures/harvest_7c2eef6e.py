"""Retain actual paired preparation and missing tick-subscriber failures."""
import hashlib,json,pathlib
R=pathlib.Path(__file__).resolve().parents[4];E=R/'docs/compat/evidence';short='7c2eef6e'
def read(p):return json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
b=read(E/f'catalog-headed/binary-paired-{short}.json');s=read(E/f'catalog-headed/source-{short}.json');assert b['all_passed'] and b['isolated_build'];assert sha(pathlib.Path(b['binary']))==b['binary_sha256']
for name,h in s['files'].items():assert sha(pathlib.Path(s['source_root'])/name)==h
for check in b['checks']:assert check['exit_code']==0 and sha(R/check['log'])==check['log_sha256']
review=read(E/'paired-catalog-fixtures/root-review-verification.json');assert review['actual_model']=='grok-4.5' and review['metadata']['review_outcome']=='approved'
rows=[]
for p in sorted((E/'paired-catalog-fixtures/live').glob(f'*-{short}.json')):
 r=read(p);log=p.with_suffix('.log');assert r['exit_code']==1 and sha(log)==r['log_sha256'];assert all(r[k]==b[k] for k in ('host_commit','client_commit','binary_sha256'));lines=log.read_text().splitlines();events=[json.loads(x) for x in lines if x.startswith('{')];identity=next(x for x in events if x.get('phase')=='identity');baselines=[x for x in events if x.get('phase')=='baseline-after-preparation'];starts=[x for x in events if x.get('phase')=='start'];failure=[x for x in lines if x.startswith('FAIL:')];assert len(failure)==1;assert not any(x.startswith('PASS:') for x in lines)
 row={k:r[k] for k in ('host_commit','client_commit','catalog_commit','revision','exit_code','elapsed_seconds')};row.update(receipt=str(p.relative_to(R)),log=str(log.relative_to(R)),outcome='FAIL',case='nature_crafter_air_pair' if r['case']=='air' else 'duel_arena_pair',failure_excerpt=failure[0],starts=starts,baselines=baselines,build_provenance='Exact7c committed source; source-verified root28 functional compiler cache reused, no independent empty-start claim.',scope='Neither partial paired outcome nor full core accepted. Newer catalog gated after old-catalog failure.')
 if r['case']=='air':
  assert 'preparation timeout' in failure[0] and 'a_started=true b_started=false' in failure[0];assert len(starts)==len(baselines)==1
  row.update(failure_class='fixture_bank_ack_at_ruins_and_missing_shared_start_barrier',diagnosis='Runner teleported to Air ruins then attempted open_nearest_booth; Master started alone. Both-revision paired preparation failed; no two-script gameplay claim. Task103 owns correction.')
 else:
  assert 'not impl: BotHost.addTickListener' in failure[0];assert len(starts)==len(baselines)==2
  assert all(x['observation']['ingame'] and x['observation']['scene_state']==2 and x['observation']['weapon_equipped'] for x in baselines)
  row.update(failure_class='missing_adapter_BotHost_addTickListener',diagnosis='Two prepared actual Duel scripts reached Start; required native tick callback mapping absent. Task104 designs lifecycle-safe adapter; no duel/combat acceptance.')
 rows.append(row)
assert len(rows)==4 and {(x['revision'],x['case']) for x in rows}=={(r,c) for r in (274,289) for c in ('nature_crafter_air_pair','duel_arena_pair')}
summary=dict(source_files_verified=len(s['files']),passed=0,failed=4,rows=rows);(E/f'paired-catalog-fixtures/summary-{short}.json').write_text(json.dumps(summary,indent=2)+'\n')
p=E/'catalog-harness/core-results.json';ledger=read(p);old={r['receipt'] for r in ledger};ledger.extend(r for r in rows if r['receipt'] not in old);p.write_text(json.dumps(ledger,indent=2)+'\n')
p=R/'docs/compat/support-matrix.json';matrix=read(p);mapping={'NatureCrafter':'nature_crafter_air_pair','Duel Arena Combat Trainer':'duel_arena_pair'};matched=[]
for cell in matrix['rows']:
 found=[r for r in rows if r['case']==mapping.get(cell['display_name']) and r['revision']==cell['revision'] and r['catalog_commit']==cell['catalog_commit']]
 if not found:continue
 row=found[0];detail='Isolated7c paired FAIL: '+row['failure_excerpt']+'. No paired core accepted.';cell.update(status='PARTIAL',status_detail=detail,fixture_readiness=detail);cell['proof_refs']=list(dict.fromkeys(cell.get('proof_refs',[])+[row['receipt'],f'docs/compat/evidence/paired-catalog-fixtures/summary-{short}.json']));matched.append(cell['display_name'])
assert len(matched)==4,matched
p.write_text(json.dumps(matrix,indent=2)+'\n');print(json.dumps(dict(completed=4,passed=0,failed=4,ledger_rows=len(ledger),matrix_updates=len(matched))))
