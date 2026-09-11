"""Verify and retain the first reviewed Mule/resource adapter failures."""
import hashlib,json,pathlib
ROOT=pathlib.Path(__file__).resolve().parents[4]
E=ROOT/'docs/compat/evidence';SHORT='28d97f38'
def read(p):return json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
source=read(E/f'catalog-headed/source-{SHORT}.json');build=read(E/f'catalog-headed/binary-headless-{SHORT}.json');gate=read(E/f'catalog-headed/review-gates-{SHORT}.json')
assert build['isolated_build'] and build['target_started_empty'] and build['exit_code']==0
assert sha(pathlib.Path(build['binary']))==build['binary_sha256']
assert gate['all_required_reviews_approved'] and gate['root_fixture_checks_passed']
checks=read(E/f'catalog-headed/checks-{SHORT}.json');assert checks['all_passed']
for check in checks['checks']:assert sha(ROOT/check['log'])==check['log_sha256'] and check['exit_code']==0
for name,digest in source['files'].items():assert sha(pathlib.Path(source['source_root'])/name)==digest,name
missing={'mule_crafter':'reader.inventory','gnome_chop':'Traversal.preload','gnome_fletch_short':'Traversal.preload','gnome_fletch_long':'Traversal.preload','coal_trucks':'Tools.bestPickaxe'}
rows=[]
for path in sorted((E/'catalog-harness/live').glob(f'*-{SHORT}.json')):
 r=read(path);log=path.with_suffix('.log');text=log.read_text();assert sha(log)==r['log_sha256'];assert r['exit_code']==1
 assert all(r[k]==build[k] for k in ('host_commit','client_commit','binary_sha256'))
 identity=json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"identity"')))
 assert sha(pathlib.Path(identity['card']['source_path']))==identity['card']['source_sha256']
 baseline=json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"baseline-after-preparation"')))['observation']
 start=json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"start"')))
 assert baseline['ingame'] and baseline['scene_state']==2 and start['count']==1
 failures=[l for l in text.splitlines() if l.startswith('FAIL:')];assert len(failures)==1
 op=missing[r['case']];assert failures[0].endswith('not impl: '+op)
 assert not any(l.startswith('PASS:') for l in text.splitlines())
 row={k:r[k] for k in ('host_commit','client_commit','catalog_commit','case','exit_code','elapsed_seconds')}
 row.update(revision=int(r['revision']),receipt=str(path.relative_to(ROOT)),log=str(log.relative_to(ROOT)),outcome='FAIL',failure_excerpt=failures[0],failure_class='missing_adapter_'+op.replace('.','_'),missing_operation=op,baseline=baseline,build_provenance=f"{len(source['files'])} exact source blobs and isolated binary verified.",scope='No script core accepted. Newer catalog family gated after first old-catalog failure; full supported branches/frontend acceptance remains pending.')
 rows.append(row)
assert len(rows)==10 and {(r['revision'],r['case']) for r in rows}=={(rev,case) for rev in (274,289) for case in missing}
for path in (E/'catalog-harness/live').glob(f'*-{SHORT}.process.json'):assert path.with_name(path.name.replace('.process.json','.json')).exists()
# Root read both the running native panel and this exact saved terminal PNG.
np=E/f'catalog-headed/r289-mule-crafter-8e7d965b-{SHORT}-gpu.json';nr=read(np);nb=read(E/f'catalog-headed/binary-{SHORT}.json');assert nr['exit_code']==nr['process_exit_code']==1
assert sha(np.with_suffix('.log'))==nr['log_sha256'] and sha(np.with_suffix('.timeline.jsonl'))==nr['timeline_sha256']
assert nb['binary_sha256']==nr['binary_sha256']==sha(pathlib.Path(nb['binary']))
assert any('not impl: reader.inventory' in x for x in nr['runtime_errors'])
shots=list(np.with_suffix('').glob('shots/*/*.json'));assert len(shots)==1
sp=shots[0];png=sp.with_suffix('.png');s=read(sp);assert s['ingame'] and s['scene_state']==2
assert {x['def']['id']:x['count'] for x in s['inventory']}=={1438:1}
assert {x['def']['id']:x['count'] for x in s['bank']}=={1436:200}
assert s['modals']['main']==5292 and s['bank_loaded']
native=dict(outcome='FAIL',revision=289,catalog_commit=nr['catalog_commit'],case='mule_crafter',host_commit=nr['host_commit'],client_commit=nr['client_commit'],receipt=str(np.relative_to(ROOT)),snapshot=str(sp.relative_to(ROOT)),snapshot_sha256=sha(sp),png=str(png.relative_to(ROOT)),png_sha256=sha(png),visual_read=True,visual_observation='Actual Falador East bank open with 200 essence and held Air talisman; panel onPaint and log show not impl: reader.inventory. No essence withdrawn or craft accepted.',process_elapsed_seconds=nr['elapsed_seconds'],runtime_error_lines=len(nr['runtime_errors']),renderer_claim='GPU requested; no texture-presentation counters were emitted in this run, so no routing or GPU performance claim.')
(E/f'catalog-headed/summary-native-mule-{SHORT}.json').write_text(json.dumps(native,indent=2)+'\n')
summary=dict(source_files_verified=len(source['files']),passed=0,failed=len(rows),rows=rows,native=native)
(E/f'catalog-harness/resources-mule-{SHORT}.json').write_text(json.dumps(summary,indent=2)+'\n')
lp=E/'catalog-harness/core-results.json';ledger=read(lp);prior={r['receipt'] for r in ledger};ledger.extend(r for r in rows if r['receipt'] not in prior);lp.write_text(json.dumps(ledger,indent=2)+'\n')
mp=ROOT/'docs/compat/support-matrix.json';matrix=read(mp);cases={'MuleCrafter':['mule_crafter'],'GnomeMagicChopper':['gnome_chop','gnome_fletch_short','gnome_fletch_long'],'CoalTrucks':['coal_trucks']}
for cell in matrix['rows']:
 selected=[r for r in rows if r['revision']==cell['revision'] and r['catalog_commit']==cell['catalog_commit'] and r['case'] in cases.get(cell['display_name'],[])]
 refs=[r['receipt'] for r in selected]
 if selected:detail='Isolated28: '+', '.join(r['case']+' FAIL missing '+r['missing_operation'] for r in selected)+'. No core accepted; host mapping follow-ups101/102, no foreign-defect dim.'
 elif cell['display_name']=='MuleCrafter' and cell['revision']==289 and cell['catalog_commit']==native['catalog_commit']:detail='Native28: FAIL reader.inventory at actual Falador East bank. Air talisman held; essence not withdrawn. No core accepted.';refs=[native['receipt']]
 else:continue
 cell.update(status='PARTIAL',status_detail=detail,fixture_readiness=detail);cell['proof_refs']=list(dict.fromkeys(cell.get('proof_refs',[])+refs+[f'docs/compat/evidence/catalog-harness/resources-mule-{SHORT}.json']))
mp.write_text(json.dumps(matrix,indent=2)+'\n')
print(json.dumps(dict(completed=len(rows),passed=0,failed=len(rows),ledger_rows=len(ledger))))
