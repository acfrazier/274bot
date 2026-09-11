from pathlib import Path
import json,hashlib
root=Path(__file__).resolve().parents[4];ev=root/'docs/compat/evidence';rows=[]
for p in sorted((ev/'paired-catalog-fixtures/live').glob('*-3ba51755.json')):
 r=json.loads(p.read_text());log=p.with_suffix('.log');text=log.read_text();assert hashlib.sha256(log.read_bytes()).hexdigest()==r['log_sha256'];assert r['exit_code']==1
 failure=next(x for x in text.splitlines() if x.startswith('FAIL:'))
 row={k:r[k] for k in ['host_commit','client_commit','catalog_commit','revision','exit_code','elapsed_seconds']};row.update(case={'air':'nature_crafter_air_pair','mule':'mule_crafter_pair'}[r['case']],outcome='FAIL',failure_excerpt=failure[:1500],receipt=str(p.relative_to(root)),log=str(log.relative_to(root)),scope='Old catalog only; newer revision/catalog counterparts not run after family failure. Paired exchange not accepted.')
 rows.append(row)
assert len(rows)==4
p=ev/'catalog-harness/core-results.json';ledger=json.loads(p.read_text());seen={r['receipt'] for r in ledger};ledger.extend(r for r in rows if r['receipt'] not in seen);p.write_text(json.dumps(ledger,indent=2)+'\n')
p=root/'docs/compat/support-matrix.json';matrix=json.loads(p.read_text());names={'NatureCrafter':'nature_crafter_air_pair','MuleCrafter':'mule_crafter_pair'}
for cell in matrix['rows']:
 selected=[r for r in rows if r['case']==names.get(cell['display_name']) and r['revision']==cell['revision'] and r['catalog_commit']==cell['catalog_commit']]
 if selected:
  cell['status']='PARTIAL';cell['status_detail']='Paired 3ba51755 FAIL: no completed exchange. Paint-button dispatch fix is included; native trade mapping and honest bootstrap fixture need diagnosis. Previous failures retained; later catalog pair not retried.';cell['proof_refs']=list(dict.fromkeys(cell.get('proof_refs',[])+[r['receipt'] for r in selected]))
p.write_text(json.dumps(matrix,indent=2)+'\n');(ev/'paired-catalog-fixtures/summary-3ba51755.json').write_text(json.dumps({'passed':0,'failed':4,'rows':rows},indent=2)+'\n');print({'ledger_rows':len(ledger),'new_failures':len(rows)})
