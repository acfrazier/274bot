"""Verify isolated named-bank vial cycles and retain Tanner count-dialog failures."""
import hashlib
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[4]
E = ROOT / 'docs/compat/evidence'
SHORT = '060e15a0'


def read(p):
    return json.loads(p.read_text())


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def compact(o):
    return {k: o[k] for k in ('tick', 'tile', 'item_ids', 'bank_ids',
                              'bank_generation', 'bank_open', 'bank_loaded')}


source = read(E / f'catalog-headed/source-{SHORT}.json')
build = read(E / f'catalog-headed/binary-headless-{SHORT}.json')
gate = read(E / f'catalog-headed/review-gates-{SHORT}.json')
assert build['isolated_build'] and build['target_started_empty'] and build['exit_code'] == 0
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
assert gate['all_required_reviews_approved'] and gate['root_fixture_checks_passed']
for name, digest in source['files'].items():
    assert sha(pathlib.Path(source['source_root']) / name) == digest, name

rows = []
for path in sorted((E / 'catalog-harness/live').glob(f'*-{SHORT}.json')):
    receipt = read(path)
    log = path.with_suffix('.log')
    text = log.read_text()
    assert sha(log) == receipt['log_sha256']
    assert all(receipt[k] == build[k] for k in ('host_commit', 'client_commit', 'binary_sha256'))
    identity = json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"identity"')))
    assert sha(pathlib.Path(identity['card']['source_path'])) == identity['card']['source_sha256']
    row = {k: receipt[k] for k in ('host_commit', 'client_commit', 'catalog_commit', 'case', 'exit_code', 'elapsed_seconds')}
    row.update(revision=int(receipt['revision']), receipt=str(path.relative_to(ROOT)),
               log=str(log.relative_to(ROOT)), build_provenance='3082 exact source blobs and isolated binary verified.',
               scope='Selected banking branches; full supported branches and final integrated frontend acceptance remain separate.')
    if receipt['exit_code']:
        failures = [l for l in text.splitlines() if l.startswith('FAIL:')]
        assert receipt['case'] in ('tanner_bot', 'tanner_bot_hard')
        assert any('not impl: reader.countDialogOpen' in l for l in failures)
        row.update(outcome='FAIL', failure_excerpt=failures,
                   failure_class='missing_count_dialog_adapter_mapping',
                   diagnosis='Input.invButton now dispatches. Tanner reaches missing reader.countDialogOpen; native count-dialog facts and answer-count already exist. Task t_ee0d3dbd maps both adapter members. No imported-script defect or dim.')
    else:
        assert receipt['case'] in ('vial_filler', 'vial_filler_east')
        result = json.loads(next(l.split('PASS: catalog_boundary_live: ', 1)[1]
                                 for l in text.splitlines() if l.startswith('PASS: catalog_boundary_live: ')))
        c = result['core']
        a, z = c['baseline'], c['latest']
        cy = c['vial_filler_cycle']
        f, d, w = [cy[k] for k in ('filled', 'deposited', 'withdrawn')]
        marks = [a, f, d, w, z]
        assert all(o['ingame'] and o['scene_state'] == 2 for o in marks)
        assert all(marks[i]['tick'] < marks[i + 1]['tick'] for i in range(len(marks) - 1))
        assert not a['item_ids'] and c['post_start_observations'] > 0
        assert f['item_ids']['227'] >= 1 and c['max_items']['Vial of water'] == 28
        assert d['bank_open'] and d['bank_loaded'] and not d['item_ids']
        assert d['bank_ids'] == {'227': 28, '229': 28}
        assert w['bank_open'] and w['bank_loaded']
        assert w['item_ids'] == {'229': 28} and w['bank_ids'] == {'227': 28}
        assert w['bank_generation'] == d['bank_generation'] > a['bank_generation']
        assert z['bank_generation'] > w['bank_generation'] and not z['bank_open']
        assert z['item_ids']['227'] >= 1 and sum(z['item_ids'].values()) == 28
        assert z['tile'][0] in (2949, 2950) and z['tile'][1:] == [3380, 0]
        assert cy['returned'] and cy['further']
        if receipt['case'] == 'vial_filler_east':
            assert 3010 <= d['tile'][0] <= 3016 and 3353 <= d['tile'][1] <= 3356
        else:
            assert 2944 <= d['tile'][0] <= 2950 and 3366 <= d['tile'][1] <= 3370
        row.update(outcome='PASS', observed={k: compact(v) for k, v in
                   [('first_fill', f), ('deposited', d), ('restocked', w), ('further_fill', z)]})
    rows.append(row)

for revision in (274, 289):
    for case in ('vial_filler', 'vial_filler_east', 'tanner_bot', 'tanner_bot_hard'):
        family = [r for r in rows if r['revision'] == revision and r['case'] == case]
        assert len(family) == 2 or (len(family) == 1 and family[0]['outcome'] == 'FAIL'), (revision, case)
for path in (E / 'catalog-harness/live').glob(f'*-{SHORT}.process.json'):
    assert path.with_name(path.name.replace('.process.json', '.json')).exists(), path
assert len(rows) == 12 and sum(r['outcome'] == 'PASS' for r in rows) == 8
summary = dict(source_files_verified=len(source['files']), passed=8, failed=4, rows=rows,
               performance_measurement=False, final_integrated_acceptance=False)
(E / f'catalog-harness/banking-{SHORT}.json').write_text(json.dumps(summary, indent=2) + '\n')
ledger_path = E / 'catalog-harness/core-results.json'
ledger = read(ledger_path)
old = {r['receipt'] for r in ledger}
ledger.extend(r for r in rows if r['receipt'] not in old)
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
matrix_path = ROOT / 'docs/compat/support-matrix.json'
matrix = read(matrix_path)
cases = {'VialFiller': ['vial_filler', 'vial_filler_east'], 'TannerBot': ['tanner_bot', 'tanner_bot_hard']}
for cell in matrix['rows']:
    selected = [r for r in rows if r['case'] in cases.get(cell['display_name'], [])
                and r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit']]
    if selected:
        detail = 'Isolated060: ' + ', '.join(r['case'] + '=' + r['outcome'] for r in selected)
        detail += '. Vial passes verify fill28/deposit28/restock28/return/further fill; Tanner missing count-dialog mapping remains explicit. Full branches/frontends separate.'
        cell.update(status='PARTIAL', status_detail=detail, fixture_readiness=detail)
        cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) + [r['receipt'] for r in selected]
                                              + [f'docs/compat/evidence/catalog-harness/banking-{SHORT}.json']))
matrix_path.write_text(json.dumps(matrix, indent=2) + '\n')
print(json.dumps(dict(completed=len(rows), passed=8, failed=4, ledger_rows=len(ledger))))
