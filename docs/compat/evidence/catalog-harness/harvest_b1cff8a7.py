"""Recompute the bounded B1 catalog cells and retain every failed run."""
import hashlib
import json
import pathlib
import subprocess

root = pathlib.Path(subprocess.check_output(
    ['git', 'rev-parse', '--show-toplevel'], cwd=pathlib.Path(__file__).parent,
    text=True).strip())
e = root / 'docs/compat/evidence'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
build = json.loads((e/'catalog-headed/binary-headless-b1cff8a7.json').read_text())
source = json.loads((e/'catalog-headed/source-b1cff8a7.json').read_text())
assert build['isolated_build'] and build['exit_code'] == 0
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
for name, digest in source['files'].items():
    assert sha(pathlib.Path(source['source_root'])/name) == digest, name

def brief(observation):
    return {k: observation[k] for k in ['tick', 'tile', 'item_ids', 'bank_ids',
            'bank_generation', 'bank_open', 'bank_loaded', 'xp']}

rows = []
for path in sorted((e/'catalog-harness/live').glob('*-b1cff8a7.json')):
    r = json.loads(path.read_text()); log = path.with_suffix('.log')
    text = log.read_text(); lines = text.splitlines()
    assert r['finished_at'] and sha(log) == r['log_sha256']
    assert all(r[k] == build[k] for k in ['host_commit', 'client_commit', 'binary_sha256'])
    identity = json.loads(next(l for l in lines if l.startswith('{"phase":"identity"')))
    assert sha(pathlib.Path(identity['card']['source_path'])) == identity['card']['source_sha256']
    row = {k: r[k] for k in ['host_commit', 'client_commit', 'catalog_commit',
                           'case', 'exit_code', 'elapsed_seconds']}
    row.update(revision=int(r['revision']), receipt=str(path.relative_to(root)),
               log=str(log.relative_to(root)),
               build_provenance='Isolated B1 binary and all2548 original source files reverified.',
               scope='Selected catalog core/options; full frontend and remaining branches separate.')
    case = r['case']
    if r['exit_code']:
        failures = [l for l in lines if l.startswith('FAIL:')]
        assert failures
        row.update(outcome='FAIL', failure_excerpt=failures)
        if case == 'flax_picker':
            assert int(r['revision']) == 274 and 'has_item_id(1779)>=28' in failures[-1]
            assert failures[-1].count('InvRow { id: 1779,') == 6
            diagnosis = 'Native capture and dispatch source confirm coordinate-only loc selection reaches co-located Wall980 instead of selected Flax2646. Identity repair t_c35281b3; no foreign-card defect verdict.'
            failure_class = 'host_selected_loc_identity_loss'
        elif case == 'gnome_course_radius':
            assert 'course re-sync: step 0 (log balance) -> 6 (obstacle pipe)' in text
            assert "You can't enter the pipe from this side." in text
            diagnosis = 'Radius8 resyncs to the exit pipe after one lap despite refreshed native distances; independent ownership audit t_26092c52 precedes any dimming decision.'
            failure_class = 'radius_option_resynchronization_investigation'
        elif case == 'vial_filler':
            assert 'open and acknowledge the exact empty-vial seed bank' in failures[-1]
            diagnosis = 'Before Start, one-shot opener accepts Sent from approach walk at distance2 and never issues bank operation. Root2f1bd9cf7 places seed actor adjacent; new LIVE pending.'
            failure_class = 'fixture_one_shot_approach_without_bank_open'
        elif case == 'vial_filler_east':
            assert 'fresh_bank_item_id(227)>=1' in failures[-1]
            assert failures[-1].count('InvRow { id: 227,') == 28
            assert 'could not open the bank' in text
            diagnosis = 'Script withdrew and filled28 water vials, then repeatedly requests booth3011,3354 from outside the bank at3010,3352. Approach/selection ownership remains unresolved.'
            failure_class = 'bank_return_approach_investigation'
        else:
            raise AssertionError(case)
        row.update(failure_class=failure_class, diagnosis=diagnosis)
    else:
        c = json.loads(next(l.split('PASS: catalog_boundary_live: ', 1)[1]
                            for l in lines if l.startswith('PASS: catalog_boundary_live: ')))['core']
        a, z = c['baseline'], c['latest']
        assert a['ingame'] and z['ingame'] and a['scene_state'] == z['scene_state'] == 2
        assert a['tick'] < z['tick'] and c['post_start_observations'] > 0
        if case in ['chicken_killer_bank', 'flax_picker']:
            cycle = c[case+'_cycle']; d = cycle['deposited']
            item = '314' if case == 'chicken_killer_bank' else '1779'
            assert cycle['returned'] and cycle['further'] and not a['item_ids']
            assert d['bank_open'] and d['bank_loaded'] and not d['item_ids']
            assert d['bank_ids'][item] >= 1 and d['tick'] < z['tick']
            assert z['item_ids'][item] >= 1 and not z['bank_open']
            assert z['bank_generation'] > d['bank_generation'] > a['bank_generation']
            if case == 'chicken_killer_bank':
                assert cycle['combat_loot'] and a['levels']['attack'] == a['levels']['strength'] == 30
                assert z['xp']['strength'] > d['xp']['strength'] > a['xp']['strength']
            else:
                assert cycle['first_pack'] and d['bank_ids'][item] == 28
            observed = dict(deposit=brief(d), further=brief(z))
        elif case == 'gnome_course':
            cycle = c['gnome_course_cycle']; marks = [a, cycle['log'], cycle['ground_return'], cycle['pipe'], z]
            assert all(marks[i]['tick'] < marks[i+1]['tick'] for i in range(4))
            assert [m['xp']['agility'] for m in marks] == [0, 7, 27, 32, 94]
            assert z['tile'][2] == 0 and max(abs(z['tile'][0]-2474), abs(z['tile'][1]-3429)) <= 3
            assert cycle['second_lap']
            observed = dict(landmarks=[brief(m) for m in marks], qualification='Full lap and completed next log at94XP')
        elif case in ['potion_maker', 'potion_maker_named']:
            cycle = c['potion_maker_cycle']; u, f, d, w = [cycle[k] for k in ['unfinished', 'finished', 'deposited', 'withdrawn']]
            herb, unf, final = ('249', '91', '121') if case == 'potion_maker' else ('257', '99', '139')
            assert not a['item_ids'] and cycle['further'] and not cycle['wrong_product'] and not cycle['filter_violated']
            assert a['tick'] < u['tick'] < f['tick'] < d['tick'] < w['tick'] < z['tick']
            assert u['item_ids'][unf] > 0 and f['item_ids'][final] > 0
            assert d['bank_open'] and d['bank_loaded'] and not d['item_ids'] and d['bank_ids'][final] == 14
            assert w['bank_open'] and w['bank_loaded'] and w['bank_generation'] == d['bank_generation']
            assert w['item_ids'][herb] == w['item_ids']['227'] == 14
            assert z['item_ids'][final] > 0 and z['xp']['herblore'] > w['xp']['herblore'] > a['xp']['herblore']
            if case == 'potion_maker_named':
                assert d['bank_ids']['249'] == w['bank_ids']['249'] == 14 and '249' not in z['item_ids']
            observed = {k: brief(cycle[k]) for k in ['unfinished', 'finished', 'deposited', 'withdrawn']}
            observed['further'] = brief(z)
        else:
            raise AssertionError(case)
        row.update(outcome='PASS', observed=observed)
    rows.append(row)

assert len(rows) == 25 and sum(r['outcome'] == 'PASS' for r in rows) == 18
p = e/'catalog-harness/core-results.json'; ledger = json.loads(p.read_text())
old = {r['receipt'] for r in ledger}; ledger.extend(r for r in rows if r['receipt'] not in old)
p.write_text(json.dumps(ledger, indent=2)+'\n')
result = dict(source_files_verified=len(source['files']), passed=18, failed=7, rows=rows)
(e/'catalog-harness/combined-b1cff8a7.json').write_text(json.dumps(result, indent=2)+'\n')
p = root/'docs/compat/support-matrix.json'; matrix = json.loads(p.read_text())
cases = {'ChickenKiller': ['chicken_killer_bank'], 'FlaxPicker': ['flax_picker'],
         'GnomeCourse': ['gnome_course', 'gnome_course_radius'],
         'PotionMaker': ['potion_maker', 'potion_maker_named'],
         'VialFiller': ['vial_filler', 'vial_filler_east']}
details = {
    'ChickenKiller': 'B1 all four Loot-count bank cycles prove deposited feathers, closed-bank return and fresh feathers with further StrengthXP. Core melee and these cycles pass; other settings/frontends separate.',
    'FlaxPicker': 'B1 both289 catalog cycles pass full pack, deposit28 and fresh picking after return. 274 still fails on co-located loc identity loss; t_c35281b3 gates requalification.',
    'GnomeCourse': 'B1 four default cells prove94XP and next-log completion. Radius8/default-course is an independently diagnosed imported resync defect (05k); the card remains enabled, with this option limitation recorded in branch_limits.',
    'PotionMaker': 'B1 all eight default/named recipe cells prove exact unfinished and finished potions, HerbloreXP, deposit14, input restock and further finished output. Named Ranarr preserves14 unselected Guam in bank. Other branches/frontends separate.',
    'VialFiller': 'B1 West fails pre-Start on one-shot approach fixture; corrected adjacent seed is pending LIVE. East fills28 water vials but fails bank return/open, with ownership unresolved. Shop branch remains separately pending.'}
for cell in matrix['rows']:
    name = cell['display_name']
    selected = [r for r in rows if name in cases and r['case'] in cases[name]
                and r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit']]
    if not selected:
        continue
    cell['status'] = 'PARTIAL'; cell['fixture_readiness'] = details[name]
    cell['status_detail'] = details[name]
    cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) +
        [v for r in selected for v in [r['receipt'], r['log']]] +
        ['docs/compat/evidence/catalog-harness/combined-b1cff8a7.json']))
p.write_text(json.dumps(matrix, indent=2)+'\n')
print(json.dumps(dict(completed=len(rows), passed=18, failed=7, ledger_rows=len(ledger))))
