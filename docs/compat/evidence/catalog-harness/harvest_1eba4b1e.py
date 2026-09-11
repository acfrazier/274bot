"""Verify reviewed cooking full cycles while retaining previous failures."""
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[4]
E = ROOT / 'docs/compat/evidence'
SHORT = '1eba4b1e'


def read(p):
    return json.loads(p.read_text())


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def compact(o):
    return {k: o[k] for k in ('tick', 'tile', 'item_ids', 'bank_ids',
                              'bank_generation', 'bank_open', 'bank_loaded', 'xp')}


source = read(E / f'catalog-headed/source-{SHORT}.json')
build = read(E / f'catalog-headed/binary-headless-{SHORT}.json')
assert build['isolated_build'] and not build['target_started_empty'] and build['exit_code'] == 0
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
for name, digest in source['files'].items():
    assert sha(pathlib.Path(source['source_root']) / name) == digest, name
gate = read(E / f'catalog-headed/review-gates-{SHORT}.json')
assert gate['all_required_reviews_approved'] and gate['root_fixture_checks_passed']

CASES = ('cook_bot','cook_bot_lobster')
rows=[]
for path in sorted((E/'catalog-harness/live').glob(f'*-{SHORT}.json')):
 r=read(path);log=path.with_suffix('.log');text=log.read_text()
 assert sha(log)==r['log_sha256'] and r['case'] in CASES
 assert all(r[k]==build[k] for k in ('host_commit','client_commit','binary_sha256'))
 identity=json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"identity"')))
 assert sha(pathlib.Path(identity['card']['source_path']))==identity['card']['source_sha256']
 row={k:r[k] for k in ('host_commit','client_commit','catalog_commit','case','exit_code','elapsed_seconds')}
 row.update(revision=int(r['revision']),receipt=str(path.relative_to(ROOT)),log=str(log.relative_to(ROOT)),build_provenance=f"{len(source['files'])} exact source blobs and isolated binary verified",scope='Selected core/options only; remaining supported options/frontends separate.')
 if r['exit_code']:
  failures=[l for l in text.splitlines() if l.startswith('FAIL:')]
  assert failures
  row.update(outcome='FAIL',failure=failures[-1].split('; evidence=')[0][:1200])
 else:
  c=json.loads(next(l.split('PASS: catalog_boundary_live: ',1)[1] for l in text.splitlines() if l.startswith('PASS: catalog_boundary_live: ')))['core']
  a,z=c['baseline'],c['latest'];case=r['case']
  assert a['ingame'] and z['ingame'] and a['scene_state']==z['scene_state']==2 and a['tick']<z['tick'] and c['post_start_observations']>0
  if case.startswith('ardy_'):
   cake=case=='ardy_cakes';cy=c['ardy_cakes_cycle' if cake else 'ardy_thiever_cycle'];first='stolen' if cake else 'pickpocketed';w,d=cy[first],cy['deposited'];item='1891' if cake else '995'
   assert a['item_ids'].get(item,0)==0 and a['tick']<w['tick']<d['tick']<z['tick']
   assert w['item_ids'].get(item,0)>0 and w['xp']['thieving']>a['xp']['thieving']
   assert d['bank_open'] and d['bank_loaded'] and d['bank_ids'].get(item,0)>=w['item_ids'][item] and d['item_ids'].get(item,0)==0
   assert not z['bank_open'] and z['bank_generation']>d['bank_generation']>a['bank_generation'] and z['item_ids'].get(item,0)>0 and z['xp']['thieving']>w['xp']['thieving']
   assert cy['returned'] and cy['further']
   if cake:assert not cy['noted'] and not cy['wrong_product']
   observed={first:compact(w),'deposited':compact(d),'further':compact(z)}
  else:
   cy=c['station_production_cycle'];w,p,d,rs=[cy[k] for k in ('withdrawn','produced','deposited','restocked')]
   marks=[a,w,p,d,rs,z];assert all(marks[i]['tick']<marks[i+1]['tick'] for i in range(len(marks)-1))
   recipe={'smelter_bot':('2349',{'436':1,'438':1},'smithing'),'smelter_bot_steel':('2353',{'440':1,'453':2},'smithing'),'cook_bot':('329',{'331':1},'cooking'),'cook_bot_lobster':('379',{'377':1},'cooking'),'flax_spinner':('1777',{'1779':1},'crafting')}
   product,inputs,skill=recipe[case]
   assert a['item_ids'].get(product,0)==0 and p['item_ids'].get(product,0)>0 and p['xp'][skill]>a['xp'][skill]
   for item,ratio in inputs.items():
    assert w['item_ids'].get(item,0)>p['item_ids'].get(item,0)
    assert rs['item_ids'].get(item,0)>z['item_ids'].get(item,0) and rs['bank_ids'].get(item,0)<d['bank_ids'].get(item,0)
   assert d['bank_open'] and d['bank_loaded'] and not d['item_ids'] and d['bank_ids'].get(product,0)>=p['item_ids'][product]
   assert rs['bank_open'] and rs['bank_loaded'] and rs['bank_generation']==d['bank_generation']>w['bank_generation']
   assert not z['bank_open'] and z['bank_generation']>rs['bank_generation'] and z['item_ids'].get(product,0)>0 and z['xp'][skill]>p['xp'][skill]
   assert cy['returned'] and cy['further'] and not cy['wrong_product'] and not cy['noted']
   observed={k:compact(cy[k]) for k in ('withdrawn','produced','deposited','restocked')};observed['further']=compact(z)
  row.update(outcome='PASS',observed=observed)
 rows.append(row)
for rev in (274,289):
 for case in CASES:
  family=[r for r in rows if r['revision']==rev and r['case']==case]
  assert len(family)==2 or (len(family)==1 and family[0]['outcome']=='FAIL'),(rev,case)
for p in (E/'catalog-harness/live').glob(f'*-{SHORT}.process.json'):
 assert p.with_name(p.name.replace('.process.json','.json')).exists(),p
summary=dict(source_files_verified=len(source['files']),passed=sum(r['outcome']=='PASS' for r in rows),failed=sum(r['outcome']=='FAIL' for r in rows),rows=rows)
(E/f'catalog-harness/cook-{SHORT}.json').write_text(json.dumps(summary,indent=2)+'\n')
ledger_path=E/'catalog-harness/core-results.json';ledger=read(ledger_path);old={r['receipt'] for r in ledger};ledger.extend(r for r in rows if r['receipt'] not in old);ledger_path.write_text(json.dumps(ledger,indent=2)+'\n')
matrix_path=ROOT/'docs/compat/support-matrix.json';matrix=read(matrix_path);cases={'CookBot':['cook_bot','cook_bot_lobster']}
for cell in matrix['rows']:
 selected=[r for r in rows if r['case'] in cases.get(cell['display_name'],[]) and r['revision']==cell['revision'] and r['catalog_commit']==cell['catalog_commit']]
 if selected:
  detail='Isolated1eba: '+', '.join(r['case']+'='+r['outcome'] for r in selected)+'. Passed cells include ordered work, bank deposit, closed return and further work. Failed cells retained; full options/frontends separate.'
  cell.update(status='PARTIAL',status_detail=detail,fixture_readiness=detail);cell['proof_refs']=list(dict.fromkeys(cell.get('proof_refs',[])+[r['receipt'] for r in selected]+[f'docs/compat/evidence/catalog-harness/cook-{SHORT}.json']))
matrix_path.write_text(json.dumps(matrix,indent=2)+'\n');print(json.dumps(dict(completed=len(rows),passed=summary['passed'],failed=summary['failed'],ledger_rows=len(ledger))))
