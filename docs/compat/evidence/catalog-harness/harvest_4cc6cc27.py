"""Verify RuneCrafter cycles and preserve the first Ardougne failures."""
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[4]
E = ROOT / 'docs/compat/evidence'
SHORT = '4cc6cc27'


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
               log=str(log.relative_to(ROOT)), build_provenance='3190 exact source blobs and isolated binary verified.',
               scope='Selected core/options only; remaining branches and final integrated frontend acceptance separate.')
    if r['exit_code']:
        failures = [l for l in text.splitlines() if l.startswith('FAIL:')]
        assert len(failures) == 1 and r['case'] in ('ardy_cakes', 'ardy_thiever', 'ardy_thiever_knight')
        failure = failures[0]
        assert 'stat_xp_gain(17)>=1 not seen within 150 ticks' in failure
        c = json.loads(failure.split('; core=', 1)[1])['witness']
        a, z = c['baseline'], c['latest']
        assert all(o['ingame'] and o['scene_state'] == 2 for o in (a, z))
        assert a['tick'] < z['tick'] and c['post_start_observations'] > 0
        assert a['tile'] == z['tile'] and not a['item_ids'] and not z['item_ids']
        assert a['xp']['thieving'] == z['xp']['thieving'] == c['max_xp']['thieving']
        row.update(outcome='FAIL', failure_excerpt=failure.split('; evidence=', 1)[0],
                   failure_class='ardy_no_first_action_pending_native_cake_audit',
                   diagnosis='No movement, item or XP change. Root found an incomplete nearest-object cake shim; t_fa68337f audits exact ownership for each case before implementation. No foreign-script verdict or dim.',
                   baseline=compact(a), latest=compact(z), post_start_observations=c['post_start_observations'])
    else:
        result = json.loads(next(l.split('PASS: catalog_boundary_live: ', 1)[1]
                                 for l in text.splitlines() if l.startswith('PASS: catalog_boundary_live: ')))
        c = result['core']
        a, z = c['baseline'], c['latest']
        assert a['ingame'] and z['ingame'] and a['scene_state'] == z['scene_state'] == 2
        assert not a['item_ids'] and a['tick'] < z['tick'] and c['post_start_observations'] > 0
        assert r['case'] in ('rune_crafter', 'rune_crafter_earth')
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
    for case in ('rune_crafter', 'rune_crafter_earth', 'ardy_cakes', 'ardy_thiever', 'ardy_thiever_knight'):
        family = [r for r in rows if r['revision'] == rev and r['case'] == case]
        assert len(family) == 2 or (len(family) == 1 and family[0]['outcome'] == 'FAIL'), (rev, case)
for p in (E / 'catalog-harness/live').glob(f'*-{SHORT}.process.json'):
    assert p.with_name(p.name.replace('.process.json', '.json')).exists(), p

# The native Earth image and CUA presentation were read during this root turn.
native_row = read(E / f'catalog-headed/summary-native-earth-{SHORT}.json')
assert native_row['outcome'] == 'PASS' and native_row['visual_read']
for kind in ('snapshot', 'png'):
    assert sha(ROOT / native_row[kind]) == native_row[kind + '_sha256']
native_receipt = ROOT / native_row['receipt']
native_raw = read(native_receipt)
assert native_raw['exit_code'] == 0 and not native_raw['runtime_errors']
assert sha(native_receipt.with_suffix('.log')) == native_raw['log_sha256']
assert sha(native_receipt.with_suffix('.timeline.jsonl')) == native_raw['timeline_sha256']
nb = read(E / f'catalog-headed/binary-{SHORT}.json')
assert nb['binary_sha256'] == native_raw['binary_sha256'] == sha(pathlib.Path(nb['binary']))
assert len(rows) == 14 and sum(r['outcome'] == 'PASS' for r in rows) == 8
summary = dict(source_files_verified=len(source['files']), passed=sum(r['outcome'] == 'PASS' for r in rows),
               failed=sum(r['outcome'] == 'FAIL' for r in rows), rows=rows, native=native_row)
(E / f'catalog-harness/runes-ardy-{SHORT}.json').write_text(json.dumps(summary, indent=2) + '\n')
ledger_path = E / 'catalog-harness/core-results.json'
ledger = read(ledger_path)
old = {r['receipt'] for r in ledger}
ledger.extend(r for r in rows if r['receipt'] not in old)
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
matrix_path = ROOT / 'docs/compat/support-matrix.json'
matrix = read(matrix_path)
cases = {'RuneCrafter': ['rune_crafter', 'rune_crafter_earth'], 'ArdyCakes': ['ardy_cakes'], 'ArdyThiever': ['ardy_thiever', 'ardy_thiever_knight']}
for cell in matrix['rows']:
    selected = [r for r in rows if r['case'] in cases.get(cell['display_name'], [])
                and r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit']]
    if selected:
        detail = 'Isolated4cc: ' + ', '.join(r['case'] + '=' + r['outcome'] for r in selected)
        detail += '. Exact item/XP/bank-return/further-work witnesses verified for passes. Failures retained; full supported branches/frontends remain separate.'
        cell.update(status='PARTIAL', status_detail=detail, fixture_readiness=detail)
        cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) + [r['receipt'] for r in selected]
                                              + [f'docs/compat/evidence/catalog-harness/runes-ardy-{SHORT}.json']))
matrix_path.write_text(json.dumps(matrix, indent=2) + '\n')
print(json.dumps(dict(completed=len(rows), passed=summary['passed'], failed=summary['failed'], ledger_rows=len(ledger))))
