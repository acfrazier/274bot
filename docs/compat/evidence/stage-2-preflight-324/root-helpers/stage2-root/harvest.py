from pathlib import Path
import json,hashlib,sys
root=Path(__file__).resolve().parents[2];ev=root/'docs/compat/evidence';short=sys.argv[1]
def read(p):return json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
build=read(ev/f'catalog-headed/binary-headless-{short}.json');source=read(ev/f'catalog-headed/source-{short}.json');assert build['isolated_build'] and build['exit_code']==0 and sha(Path(build['binary']))==build['binary_sha256']
for name,h in source['files'].items():assert sha(Path(source['source_root'])/name)==h,name
rows=[]
for p in sorted((ev/'catalog-harness/live').glob('*-'+short+'.json')):
 r=read(p);assert r['exit_code'] in [0,1];log=p.with_suffix('.log');assert sha(log)==r['log_sha256'];assert all(r[k]==build[k] for k in ['host_commit','client_commit','binary_sha256']);lines=log.read_text().splitlines();ident=json.loads(next(l for l in lines if l.startswith('{"phase":"identity"')));assert sha(Path(ident['card']['source_path']))==ident['card']['source_sha256'];baselines=[json.loads(l)['observation'] for l in lines if l.startswith('{"phase":"baseline-after-preparation"')];assert all(b['ingame'] and b['scene_state']==2 for b in baselines)
 row={k:r[k] for k in ['host_commit','client_commit','catalog_commit','case','exit_code','elapsed_seconds']};row.update(revision=int(r['revision']),receipt=str(p.relative_to(root)),log=str(log.relative_to(root)),outcome='PASS' if r['exit_code']==0 else 'FAIL',build_provenance=f'{len(source["files"])} exact source files and isolated binary reverified',scope='Controlled core gameplay qualification; elapsed time is not a performance measurement.')
 if r['exit_code']==0:
  value=json.loads(next(l for l in lines if l.startswith('PASS: catalog_boundary_live: ')).split('PASS: catalog_boundary_live: ',1)[1]);c=value['core'];assert c['baseline']['ingame'] and c['baseline']['scene_state']==2 and c['post_start_observations']>0
  row['core']=c
 else:row['failure']=next(l for l in lines if l.startswith('FAIL:')).split('; evidence=')[0]
 row['start_baseline_present']=bool(baselines)
 rows.append(row)
cases=['auto_fighter_mage', 'flax_aio_pick']
index={(r['revision'],r['case'],r['catalog_commit']):r for r in rows};old='100adccc037d9f6898080e1cad58fcfc43364775';new='8e7d965be2071d6ec65c3265e12af797082d720a'
complete=all((rev,case,old) in index and (index[(rev,case,old)]['exit_code']!=0 or (rev,case,new) in index) for rev in [274,289] for case in cases)
skipped=[{'revision':rev,'case':case,'catalog_commit':new,'reason':'Older-catalog cell failed; diagnose before the second catalog'} for rev in [274,289] for case in cases if (rev,case,old) in index and index[(rev,case,old)]['exit_code']!=0 and (rev,case,new) not in index]
summary={'host_commit':build['host_commit'],'client_commit':build['client_commit'],'source_files_verified':len(source['files']),'passed':sum(r['exit_code']==0 for r in rows),'failed':sum(r['exit_code']==1 for r in rows),'batch_complete':complete,'skipped_cells':skipped,'rows':rows}
(ev/f'catalog-harness/qualification-{short}.json').write_text(json.dumps(summary,indent=2)+'\n')
p=ev/'catalog-harness/core-results.json';ledger=read(p);known={r['receipt'] for r in ledger};ledger.extend(r for r in rows if r['receipt'] not in known);p.write_text(json.dumps(ledger,indent=2)+'\n');print(json.dumps({k:v for k,v in summary.items() if k!='rows'}));print('ledger rows',len(ledger))
