import hashlib,json,pathlib,sqlite3,subprocess
root=pathlib.Path(subprocess.check_output(["git","rev-parse","--show-toplevel"],cwd=pathlib.Path(__file__).resolve().parent,text=True).strip())
ledger_path=root/'docs/compat/evidence/catalog-harness/core-results.json'
ledger=json.loads(ledger_path.read_text());existing={r['receipt'] for r in ledger};added=[]
for receipt in sorted((root/'docs/compat/evidence/catalog-harness/live').glob('*.json')):
 if receipt.name.endswith('.process.json') or not receipt.stem.endswith(('-a4157243','-50f2be8a')): continue
 rel=str(receipt.relative_to(root));r=json.loads(receipt.read_text());log=receipt.with_suffix('.log')
 assert r.get('finished_at') and hashlib.sha256(log.read_bytes()).hexdigest()==r['log_sha256'],receipt
 if rel in existing: continue
 row={k:r[k] for k in ['host_commit','client_commit','catalog_commit','case','exit_code','elapsed_seconds']};row.update(receipt=rel,log=str(log.relative_to(root)),revision=int(r['revision']),scope='Core or named option only; full supported options and frontend acceptance remain separate.',build_provenance='Isolated target started empty; immutable source files verified before and after build.')
 if r['exit_code']:
  row['outcome']='FAIL';row['failure_class']='missing_host_compatibility_mapping';row['failure_excerpt']=[l for l in log.read_text().splitlines() if l.startswith('FAIL:') or ('[script ' in l and ('not impl:' in l or 'TypeError' in l))]
  assert row['failure_excerpt'],receipt
 else:
  payload=json.loads(next(l.split('PASS: catalog_boundary_live: ',1)[1] for l in log.read_text().splitlines() if l.startswith('PASS: catalog_boundary_live: ')));core=payload['core'];before,after=core['baseline'],core['latest'];assert before['ingame'] and before['scene_state']==2 and after['ingame'] and after['scene_state']==2 and core['post_start_observations']>0
  case=r['case'];base=next(k for k in ['dart_fletcher','herb_cleaner','gem_cutter','gnome_course'] if case.startswith(k));cycle=core[base+'_cycle']
  if base=='dart_fletcher': assert cycle['first'] and cycle['further'] and not cycle['wrong_tier']
  elif base in ['herb_cleaner','gem_cutter']:
   assert cycle['first_pack'] and cycle['deposited'] and cycle['withdrawn'] and not cycle['filter_violated']
   assert cycle['cleaned_after_withdrawal' if base=='herb_cleaner' else 'cut_after_withdrawal']
   assert cycle['deposited']['bank_loaded'] and cycle['deposited']['bank_generation']>before['bank_generation'] and cycle['withdrawn']['tick']>=cycle['deposited']['tick']
  else:
   assert cycle['second_lap'];seq=[before,cycle['log'],cycle['ground_return'],cycle['pipe'],after];assert all(a['tick']<b['tick'] and a['xp']['agility']<b['xp']['agility'] for a,b in zip(seq,seq[1:]))
  row.update(outcome='PASS',xp_delta={k:after['xp'].get(k,0)-v for k,v in before['xp'].items() if after['xp'].get(k,0)>v},latest_items=after['items'],latest_item_ids=after['item_ids'],post_start_observations=core['post_start_observations'],cycle=cycle)
 ledger.append(row);added.append(rel)
ledger_path.write_text(json.dumps(ledger,indent=2)+'\n')
s_path=root/'docs/compat/support-matrix.json';s=json.loads(s_path.read_text())
variants={'DartFletcher':['dart_fletcher','dart_fletcher_iron'],'HerbCleaner':['herb_cleaner','herb_cleaner_named'],'GemCutter':['gem_cutter','gem_cutter_named'],'GnomeCourse':['gnome_course']}
for row in s['rows']:
 name=row['display_name']
 if name not in variants: continue
 ref='50f2be8a' if name=='GnomeCourse' else 'a4157243';refs=[]
 for case in variants[name]:
  stem=f'r{row["revision"]}-{case.replace("_","-")}-{row["catalog_commit"][:8]}-{ref}'
  rel=f'docs/compat/evidence/catalog-harness/live/{stem}.json';assert any(r['receipt']==rel and r['outcome']=='PASS' for r in ledger)
  refs += [rel,rel[:-5]+'.log']
 row['proof_refs']=list(dict.fromkeys(row.get('proof_refs',[])+refs+['docs/compat/evidence/inventory-production/isolated-results-a4157243.json' if name!='GnomeCourse' else 'docs/compat/evidence/location-world/isolated-results-50f2be8a.json']))
 row['status']='PARTIAL';row['fixture_readiness']='Controlled local fixture qualified for the linked core/named option on an isolated build; full branch and frontend acceptance remains separate.'
 detail={'DartFletcher':'Bronze and Iron branches consume exact inputs and produce the selected dart tier with further Fletching XP.','HerbCleaner':'Default and named-filter branches clean a full pack, deposit, restock and clean further exact guam leaves with Herblore XP.','GemCutter':'Default and named-filter branches cut a full pack, deposit, restock and cut further exact sapphires with Crafting XP.','GnomeCourse':'Default radius completes ordered log, ground-return and pipe milestones, then further Agility XP on the next lap. Radius-8 old-catalog cells on both revisions fail at our missing Reachability.walkable mapping; no foreign-script defect is established.'}[name]
 row['status_detail']=f'Reviewed isolated {ref} passes this catalog/revision core proof. {detail} Remaining supported settings and integrated/frontend qualification remain outstanding.'
s_path.write_text(json.dumps(s,indent=2)+'\n')
world=[{k:v for k,v in r.items() if k!='cycle'} for r in ledger if r['host_commit'].startswith('50f2be8a')];assert len(world)==7
(root/'docs/compat/evidence/location-world/isolated-results-50f2be8a.json').write_text(json.dumps(world,indent=2)+'\n')
db=sqlite3.connect('file:/Users/acfrazier/.hermes/kanban/boards/274bot/kanban.db?mode=ro',uri=True);db.row_factory=sqlite3.Row
r=dict(db.execute('select * from task_runs where id=1196').fetchone());r['metadata']=json.loads(r['metadata']) if r['metadata'] else None
(root/'docs/compat/evidence/spell-facts/review.json').parent.mkdir(parents=True,exist_ok=True)
(root/'docs/compat/evidence/spell-facts/review.json').write_text(json.dumps(r,indent=2)+'\n')
path=root/'docs/compat/evidence/build-isolation/actual-review-sessions.json';sessions=json.loads(path.read_text());print('sessions keys',list(sessions))
print(json.dumps({'ledger_rows':len(ledger),'appended':added,'world_cells':len(world)},indent=2))
