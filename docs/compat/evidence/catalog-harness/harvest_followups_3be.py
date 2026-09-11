"""Recompute completed isolated 3be68eaf follow-up cells, preserving failures."""
import hashlib
import json
import pathlib
import subprocess

root = pathlib.Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], cwd=pathlib.Path(__file__).parent, text=True).strip())
e = root / 'docs/compat/evidence'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
build = json.loads((e/'catalog-headed/binary-headless-3be68eaf.json').read_text())
source = json.loads((e/'catalog-headed/source-3be68eaf.json').read_text())
assert build['exit_code'] == 0 and build['isolated_build']
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
for name, digest in source['files'].items():
    assert sha(pathlib.Path(source['source_root']) / name) == digest, name
rows = []
for path in sorted((e/'catalog-harness/live').glob('*-3be68eaf.json')):
    r = json.loads(path.read_text())
    log = path.with_suffix('.log'); text = log.read_text()
    assert r['finished_at'] and sha(log) == r['log_sha256']
    assert r['host_commit'] == build['host_commit'] and r['binary_sha256'] == build['binary_sha256']
    identity = json.loads(next(line for line in text.splitlines() if line.startswith('{"phase":"identity"')))
    assert sha(pathlib.Path(identity['card']['source_path'])) == identity['card']['source_sha256']
    row = {k: r[k] for k in ['host_commit', 'client_commit', 'catalog_commit', 'case', 'exit_code', 'elapsed_seconds']}
    row.update(revision=int(r['revision']), receipt=str(path.relative_to(root)), log=str(log.relative_to(root)),
               build_provenance='Isolated target; binary and all 2339 frozen source files reverified.',
               scope='Selected corrected follow-ups; full supported settings and frontend acceptance remain separate.')
    if r['exit_code']:
        failures = [line for line in text.splitlines() if line.startswith('FAIL:')]
        assert failures
        row.update(outcome='FAIL', failure_excerpt=failures)
        if r['case'] == 'flax_picker':
            if int(r['revision']) == 289:
                assert 'not impl: reader.toLocal' in text
                reason = 'Missing reader.toLocal and next actions.walkTo adapter mapping; corrective card t_15c05b42.'
            else:
                assert 'has_item_id(1779)>=28 not seen within 150 ticks' in text and failures[-1].count('InvRow { id: 1779,') == 7
                reason = 'Seven flax then full-pack deadline. Source audit suspects deferred static-loc invalidation; card t_192feefa must reproduce it before repair. No foreign-script defect established.'
            row.update(failure_class='host_mapping_or_observation_investigation', diagnosis=reason)
        elif r['case'] == 'chicken_killer_bank':
            assert 'periodic bank: completed' in text and 'deadline 180s exceeded' in text
            row.update(failure_class='fixture_deadline_after_bank_return', diagnosis='Actual bank trip completes and combat is requested again; no new Feather314 before unchanged 180-second deadline. Root is auditing preparation and travel/combat time. No product/card defect established.')
        elif r['case'] == 'gnome_course_radius':
            assert "course re-sync: step 0 (log balance) -> 6 (obstacle pipe)" in text
            assert "You can't enter the pipe from this side." in text
            row.update(failure_class='radius_option_resynchronization_investigation', diagnosis='Radius8 completes first lap then frozen AgilityBot resyncs from next log to nearby exit pipe and retries from the wrong side. Exact source and native distance freshness are under ownership audit; no dimming decision yet.')
        else:
            raise AssertionError(r['case'])
    else:
        c = json.loads(next(line.split('PASS: catalog_boundary_live: ', 1)[1] for line in text.splitlines() if line.startswith('PASS: catalog_boundary_live: ')))['core']
        a,z = c['baseline'],c['latest']
        assert a['ingame'] and z['ingame'] and a['scene_state']==z['scene_state']==2 and a['tick']<z['tick']
        assert c['post_start_observations'] > 0
        if r['case']=='superheater_fire_battlestaff':
            cycle=c['superheater_cycle'];d,w=cycle['deposited'],cycle['withdrawn']
            assert a['levels']['attack']>=30 and not a['item_ids'] and not a['equipment_ids']
            assert cycle['first_bars'] and cycle['produced_after_withdrawal'] and not cycle['wrong_product'] and not cycle['wrong_staff']
            assert d['bank_open'] and d['bank_loaded'] and d['bank_generation']>a['bank_generation'] and w['bank_generation']==d['bank_generation']
            assert d['bank_ids']['2349']==13 and w['item_ids']['436']==w['item_ids']['438']==13
            assert d['tick']<w['tick']<z['tick'] and z['item_ids']['2349']>=1
            assert all(z['item_ids'][item]<w['item_ids'][item] for item in ['436','438','561'])
            assert z['equipment_ids'].get('1393')==1 and not z['bank_open']
            assert all(z['xp'][skill]>w['xp'][skill]>a['xp'][skill] for skill in ['magic','smithing'])
            row.update(outcome='PASS', cycle=cycle, xp_delta={k:z['xp'][k]-a['xp'][k] for k in ['magic','smithing']}, latest_item_ids=z['item_ids'])
        elif r['case']=='gnome_course':
            cycle=c['gnome_course_cycle'];landmarks=[a,cycle['log'],cycle['ground_return'],cycle['pipe'],z]
            assert all(landmarks[i]['tick']<landmarks[i+1]['tick'] for i in range(4))
            assert [x['xp']['agility'] for x in landmarks]==[0,7,27,32,94]
            assert z['tile'][2]==0 and max(abs(z['tile'][0]-2474),abs(z['tile'][1]-3429))<=3
            assert cycle['second_lap']
            row.update(outcome='PASS', cycle=cycle, latest_tile=z['tile'], agility_xp=94,
                       qualification='One complete lap and observed next-log completion; stronger than earlier first-lap-only flag.')
        else:
            raise AssertionError(r['case'])
    rows.append(row)
assert len(rows)>=12 and sum(r['outcome']=='PASS' for r in rows)==6
p=e/'catalog-harness/core-results.json';ledger=json.loads(p.read_text());old={r['receipt'] for r in ledger};ledger.extend(r for r in rows if r['receipt'] not in old);p.write_text(json.dumps(ledger,indent=2)+'\n')
(e/'catalog-harness/followups-3be68eaf.json').write_text(json.dumps(dict(source_files_verified=len(source['files']),rows=rows),indent=2)+'\n')
p=root/'docs/compat/support-matrix.json';matrix=json.loads(p.read_text())
for cell in matrix['rows']:
    chosen=[r for r in rows if r['revision']==cell['revision'] and r['catalog_commit']==cell['catalog_commit']]
    cases={'Superheater':'superheater_fire_battlestaff','GnomeCourse':'gnome_course','FlaxPicker':'flax_picker','ChickenKiller':'chicken_killer_bank'}
    if cell['display_name'] not in cases:continue
    selected=[r for r in chosen if r['case']==cases[cell['display_name']]]
    if not selected:continue
    cell['status']='PARTIAL'
    if cell['display_name']=='Superheater':
        cell['fixture_readiness']='Isolated Bronze/Steel and corrected Attack30 alternative Fire battlestaff cycles pass.'
        cell['status_detail']='Exact Fire battlestaff1393 equipped; 13 Bronze bars deposited, 13+13 ores restocked, further bar and rune/ore consumption with Magic/Smithing XP. Previous Attack1 fixture failures retained; other settings/frontends separate.'
    elif cell['display_name']=='GnomeCourse':
        cell['fixture_readiness']='Default course now proves one lap plus actual next log at XP94.'
        cell['status_detail']='Both catalogs/revisions pass stronger isolated default proof. Radius8 resyncs to exit pipe and fails; source/distance ownership audit pending before any dimming decision.'
    elif cell['display_name']=='FlaxPicker':
        cell['fixture_readiness']='Actual picking observed; adapter mapping and native static-loc freshness investigation pending.'
        cell['status_detail']='289 fails on reader.toLocal after picking. 274 has seven flax at full-pack deadline; source audit suspects native cache invalidation. Neither is accepted or classified as a foreign-script defect.'
    else:
        cell['fixture_readiness']='Core melee passes; Loot-count banking returns but further-feather deadline fails.'
        cell['status_detail']='Both old-catalog revision cells complete the periodic bank trip but lack required fresh Feather314 before unchanged 180 seconds. Fixture/travel/combat diagnosis pending; no card defect established.'
    cell['proof_refs']=list(dict.fromkeys(cell.get('proof_refs',[])+[v for r in selected for v in [r['receipt'],r['log']]]+['docs/compat/evidence/catalog-harness/followups-3be68eaf.json']))
p.write_text(json.dumps(matrix,indent=2)+'\n')
print(json.dumps(dict(completed=len(rows),passed=6,failed=len(rows)-6,ledger_rows=len(ledger))))
