"""Recompute isolated Flax and solo runecrafting observations from raw logs."""
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[4]
E = ROOT / 'docs/compat/evidence'
SHORT = '6750713b'


def read(p):
    return json.loads(p.read_text())


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def compact(o):
    return {k: o[k] for k in ('tick', 'tile', 'item_ids', 'bank_ids',
                              'bank_generation', 'bank_open', 'bank_loaded', 'xp')}


source = read(E / f'catalog-headed/source-{SHORT}.json')
build = read(E / f'catalog-headed/binary-headless-{SHORT}.json')
assert build['isolated_build'] and build['target_started_empty'] and build['exit_code'] == 0
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
for name, digest in source['files'].items():
    assert sha(pathlib.Path(source['source_root']) / name) == digest, name
gate = read(E / f'catalog-headed/review-gates-{SHORT}.json')
assert gate['all_required_reviews_approved'] and gate['root_fixture_checks_passed']

rows = []
for path in sorted((E / 'catalog-harness/live').glob(f'*-{SHORT}.json')):
    r = read(path)
    log = path.with_suffix('.log')
    text = log.read_text()
    assert sha(log) == r['log_sha256']
    assert all(r[k] == build[k] for k in ('host_commit', 'client_commit', 'binary_sha256'))
    identity = json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"identity"')))
    assert sha(pathlib.Path(identity['card']['source_path'])) == identity['card']['source_sha256']
    row = {k: r[k] for k in ('host_commit', 'client_commit', 'catalog_commit', 'case', 'exit_code', 'elapsed_seconds')}
    row.update(revision=int(r['revision']), receipt=str(path.relative_to(ROOT)),
               log=str(log.relative_to(ROOT)), build_provenance='2822 exact source blobs and isolated binary verified.',
               scope='Selected core/options only; remaining branches and final integrated frontend acceptance separate.')
    if r['exit_code']:
        failures = [l for l in text.splitlines() if l.startswith('FAIL:')]
        assert failures
        row.update(outcome='FAIL', failure_excerpt=failures,
                   failure_class='unresolved_pending_root_analysis')
        if r['case'] == 'mule_crafter':
            assert any("bankTile: unknown bank 'Falador East'" in line for line in failures)
            row.update(failure_class='missing_named_bank_locations_mapping',
                       diagnosis='Imported bankTile reads BANK_LOCATIONS; shim exports an empty list. Named bank metadata is a missing host mapping, not a foreign-script defect. No dim or fabricated location.')
        if r['case'] == 'rune_crafter' and int(r['revision']) == 274:
            assert 'deadline 180s exceeded' in failures[-1] and 'has_item_id(556)<=0' in failures[-1]
            row.update(failure_class='air_bank_return_or_serial_observation_audit',
                       diagnosis='Crafting occurred; failed bank pack-empty watch at180s. Three Air controls pass. Audit t_8a8e6ef3 distinguishes bank/route behavior from fixture observation timing; no dim or deadline change.')
    else:
        result = json.loads(next(l.split('PASS: catalog_boundary_live: ', 1)[1]
                                 for l in text.splitlines() if l.startswith('PASS: catalog_boundary_live: ')))
        c = result['core']
        a, z = c['baseline'], c['latest']
        assert a['ingame'] and z['ingame'] and a['scene_state'] == z['scene_state'] == 2
        assert not a['item_ids'] and a['tick'] < z['tick'] and c['post_start_observations'] > 0
        if r['case'] == 'flax_picker':
            cy = c['flax_picker_cycle']
            d = cy['deposited']
            assert cy['first_pack'] and cy['returned'] and cy['further']
            assert d['bank_open'] and d['bank_loaded'] and not d['item_ids']
            assert d['bank_ids']['1779'] == 28 and z['item_ids']['1779'] >= 1
            assert z['bank_generation'] > d['bank_generation'] > a['bank_generation']
            assert not z['bank_open'] and d['tick'] < z['tick']
            observed = dict(deposited=compact(d), further=compact(z))
        else:
            assert r['case'] in ('rune_crafter', 'rune_crafter_earth', 'mule_crafter')
            rune = '557' if r['case'] == 'rune_crafter_earth' else '556'
            cy = c['rune_crafter_cycle']
            w, en, cr, ex, d, rs = [cy[k] for k in ('withdrawn', 'entered', 'crafted', 'exited', 'deposited', 'restocked')]
            marks = [a, w, en, cr, ex, d, rs, z]
            assert all(marks[i]['tick'] < marks[i + 1]['tick'] for i in range(len(marks) - 1))
            assert w['item_ids']['1436'] > 0 and en['item_ids']['1436'] > 0 and en['tile'][1] > 4000
            assert cr['tile'][1] > 4000 and cr['item_ids'][rune] > 0 and cr['item_ids'].get('1436', 0) == 0
            assert cr['xp']['runecraft'] > a['xp']['runecraft'] and ex['tile'][1] <= 4000
            assert ex['item_ids'][rune] > 0 and d['item_ids'].get(rune, 0) == 0
            assert d['bank_open'] and d['bank_loaded'] and d['bank_ids'][rune] >= cr['item_ids'][rune]
            assert rs['bank_open'] and rs['bank_loaded'] and rs['item_ids']['1436'] > 0
            assert rs['bank_ids']['1436'] < d['bank_ids']['1436']
            assert rs['bank_generation'] == d['bank_generation'] > a['bank_generation']
            assert not z['bank_open'] and z['bank_generation'] > rs['bank_generation']
            assert z['item_ids'][rune] > 0 and z['item_ids'].get('1436', 0) == 0
            assert z['xp']['runecraft'] > cr['xp']['runecraft']
            assert cy['returned'] and cy['further'] and not cy['wrong_product'] and not cy['noted']
            observed = {k: compact(cy[k]) for k in ('withdrawn', 'entered', 'crafted', 'exited', 'deposited', 'restocked')}
            observed['further'] = compact(z)
        row.update(outcome='PASS', observed=observed)
    rows.append(row)

# Every planned family must finish, or stop after its first retained failure.
for rev in (274, 289):
    for case in ('flax_picker', 'rune_crafter', 'rune_crafter_earth', 'mule_crafter'):
        family = [r for r in rows if r['revision'] == rev and r['case'] == case]
        assert len(family) == 2 or (len(family) == 1 and family[0]['outcome'] == 'FAIL'), (rev, case)
for p in (E / 'catalog-harness/live').glob(f'*-{SHORT}.process.json'):
    assert p.with_name(p.name.replace('.process.json', '.json')).exists(), p

# Root read the internal PNG and actual CUA rendering for the formerly failing Flax274 path.
native = E / f'catalog-headed/r274-flax-picker-100adccc-{SHORT}-gpu'
nb = read(E / f'catalog-headed/binary-{SHORT}.json')
nr = read(native.with_suffix('.json'))
assert nb['isolated_build'] and sha(pathlib.Path(nb['binary'])) == nb['binary_sha256'] == nr['binary_sha256']
assert nr['exit_code'] == 0 and not nr['runtime_errors']
assert sha(native.with_suffix('.log')) == nr['log_sha256']
assert sha(native.with_suffix('.timeline.jsonl')) == nr['timeline_sha256']
text = native.with_suffix('.log').read_text()
assert 'PASS: live script_flax_picker ' in text
counts = [tuple(map(int, x)) for x in re.findall(r'present pixmap=(\d+) tex=(\d+) bind_noop=(\d+) bind_rereg=(\d+)', text)]
assert counts and counts[-1][0] == 0 and counts[-1][1] > 0
snapshot = next(native.rglob('*.json'))
s = read(snapshot)
assert s['ingame'] and s['scene_state'] == 2 and s['inv'] == [[1779, 1]]
native_row = dict(receipt=str(native.with_suffix('.json').relative_to(ROOT)), outcome='PASS',
                  snapshot=str(snapshot.relative_to(ROOT)), snapshot_sha256=sha(snapshot),
                  png=str(snapshot.with_suffix('.png').relative_to(ROOT)), png_sha256=sha(snapshot.with_suffix('.png')),
                  visual_read=True, final_frame_counters=counts[-1],
                  observed='Scene2, Trips1, return to the flax field and one fresh Flax1779. Actual Metal texture presentation; no performance/equivalence claim.')
summary = dict(source_files_verified=len(source['files']), passed=sum(r['outcome'] == 'PASS' for r in rows),
               failed=sum(r['outcome'] == 'FAIL' for r in rows), rows=rows, native=native_row)
(E / f'catalog-harness/crafting-flax-{SHORT}.json').write_text(json.dumps(summary, indent=2) + '\n')
ledger_path = E / 'catalog-harness/core-results.json'
ledger = read(ledger_path)
old = {r['receipt'] for r in ledger}
ledger.extend(r for r in rows if r['receipt'] not in old)
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
matrix_path = ROOT / 'docs/compat/support-matrix.json'
matrix = read(matrix_path)
cases = {'FlaxPicker': ['flax_picker'], 'RuneCrafter': ['rune_crafter', 'rune_crafter_earth'], 'MuleCrafter': ['mule_crafter']}
for cell in matrix['rows']:
    selected = [r for r in rows if r['case'] in cases.get(cell['display_name'], [])
                and r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit']]
    if selected:
        detail = 'Isolated675: ' + ', '.join(r['case'] + '=' + r['outcome'] for r in selected)
        detail += '. Exact item/XP/bank-return/further-work witnesses verified for passes. Failures retained; full supported branches/frontends remain separate.'
        cell.update(status='PARTIAL', status_detail=detail, fixture_readiness=detail)
        cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) + [r['receipt'] for r in selected]
                                              + [f'docs/compat/evidence/catalog-harness/crafting-flax-{SHORT}.json']))
matrix_path.write_text(json.dumps(matrix, indent=2) + '\n')
print(json.dumps(dict(completed=len(rows), passed=summary['passed'], failed=summary['failed'], ledger_rows=len(ledger))))
