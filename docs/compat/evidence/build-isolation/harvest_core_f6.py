"""Recheck the isolated forty-cell core proof and append its immutable receipts."""
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(subprocess.check_output(
    ['git', 'rev-parse', '--show-toplevel'], cwd=pathlib.Path(__file__).parent,
    text=True).strip())
EVIDENCE = ROOT / 'docs/compat/evidence'
SOURCE = 'f6b9ec4b'
CASES = {
    'Alcher': ['alcher', 'alcher_custom', 'alcher_custom_alias',
               'alcher_custom_name', 'alcher_ordered', 'alcher_large_batch'],
    'BoneBurier': ['bone_burier'], 'ChickenKiller': ['chicken_killer'],
    'Thiever': ['thiever'], 'BankFletcher': ['bank_fletcher'],
}
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
build = json.loads((EVIDENCE / f'catalog-headed/binary-headless-{SOURCE}.json').read_text())
assert build['exit_code'] == 0 and build['isolated_build'] and build['target_started_empty']
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
manifest = json.loads((EVIDENCE / f'catalog-headed/source-{SOURCE}.json').read_text())
for name, expected in manifest['files'].items():
    assert sha(pathlib.Path(manifest['source_root']) / name) == expected, name
rows = []
for path in sorted((EVIDENCE / 'catalog-harness/live').glob(f'*-{SOURCE}.json')):
    r = json.loads(path.read_text())
    assert r['case'] in sum(CASES.values(), []), path
    assert r['exit_code'] == 0 and r['finished_at'], path
    assert r['binary_sha256'] == build['binary_sha256'], path
    assert r['host_commit'] == build['host_commit'] and r['client_commit'] == build['client_commit']
    log = path.with_suffix('.log')
    assert sha(log) == r['log_sha256'], path
    payloads = [json.loads(line.split('PASS: catalog_boundary_live: ', 1)[1])
                for line in log.read_text().splitlines()
                if line.startswith('PASS: catalog_boundary_live: ')]
    assert len(payloads) == 1
    core = payloads[0]['core']
    a, z = core['baseline'], core['latest']
    assert a['ingame'] and z['ingame'] and a['scene_state'] == z['scene_state'] == 2
    assert a['player'] == z['player'] and a['tick'] < z['tick']
    assert core['post_start_observations'] > 0
    xp = {k: z['xp'].get(k, 0) - v for k, v in a['xp'].items() if z['xp'].get(k, 0) > v}
    case = r['case']
    if case.startswith('alcher'):
        assert xp['magic'] >= 65 and z['item_ids'].get('995', 0) > a['item_ids'].get('995', 0)
        assert core['max_items']['Nature rune'] > z['item_ids'].get('561', 0)
        if case in ['alcher_custom_alias', 'alcher_custom_name']:
            cycle = core['alcher_generated_custom_cycle']; w = cycle['withdrawn']
            assert cycle['consumed'] and w['bank_loaded'] and w['bank_generation'] > a['bank_generation']
            assert w['item_ids']['1332'] == w['item_ids']['561'] == 1
            assert z['item_ids'].get('1332', 0) == z['item_ids'].get('561', 0) == 0
        else:
            assert core['max_items']['Rune chainbody'] > z['item_ids'].get('1114', 0)
            if case == 'alcher_ordered':
                assert core['ordered_first_exhausted'] and core['max_items']['Rune platebody'] >= 1 and xp['magic'] >= 130
            if case == 'alcher_large_batch':
                assert core['max_items']['Rune chainbody'] >= 1000 and core['max_items']['Nature rune'] >= 1000
    elif case == 'bone_burier':
        cycle = core['bone_bank_cycle']; o, w = cycle['opened'], cycle['withdrawn']
        assert cycle['first_batch_buried'] and cycle['buried_after_withdrawal'] and core['saw_bury_chat']
        assert o['bank_loaded'] and o['bank_generation'] > a['bank_generation']
        assert o['bank_ids']['526'] == w['item_ids']['526'] == 28
        assert a['tick'] < o['tick'] <= w['tick'] < z['tick']
        assert z['xp']['prayer'] > w['xp']['prayer'] > a['xp']['prayer']
        assert z['item_ids'].get('526', 0) < w['item_ids']['526']
    elif case == 'chicken_killer':
        assert xp['strength'] > 0 and xp['prayer'] > 0 and core['saw_bury_chat']
        assert core['max_items']['Bones'] > z['item_ids'].get('526', 0)
    elif case == 'thiever':
        assert xp['thieving'] > 0 and z['item_ids']['995'] > a['item_ids'].get('995', 0)
    elif case == 'bank_fletcher':
        cycle = core['bank_fletcher_cycle']; d, w = cycle['deposited'], cycle['withdrawn']
        assert cycle['first_pack_created'] and cycle['crafted_after_withdrawal']
        assert d['bank_loaded'] and d['bank_generation'] > a['bank_generation']
        assert d['bank_ids']['60'] >= 27 and w['item_ids']['1519'] == 27
        assert d['tick'] < w['tick'] < z['tick'] and z['xp']['fletching'] > w['xp']['fletching']
        assert z['item_ids'].get('60', 0) > 0 and z['item_ids'].get('1519', 0) < 27
    row = {k: r[k] for k in ['host_commit', 'client_commit', 'catalog_commit', 'case', 'exit_code', 'elapsed_seconds']}
    row.update(revision=int(r['revision']), outcome='PASS', receipt=str(path.relative_to(ROOT)),
               log=str(log.relative_to(ROOT)), xp_delta=xp, latest_items=z['items'],
               latest_item_ids=z['item_ids'], max_items=core['max_items'],
               post_start_observations=core['post_start_observations'],
               scope='Core or named option only; full supported options and frontend acceptance remain separate.',
               build_provenance='Isolated target started empty; immutable source files verified before and after build.')
    rows.append(row)
assert len(rows) == 40
assert len({(r['revision'], r['catalog_commit'], r['case']) for r in rows}) == 40
ledger_path = EVIDENCE / 'catalog-harness/core-results.json'
ledger = json.loads(ledger_path.read_text()); existing = {r['receipt'] for r in ledger}
ledger.extend(r for r in rows if r['receipt'] not in existing)
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
summary_ref = f'docs/compat/evidence/build-isolation/core-results-{SOURCE}.json'
(ROOT / summary_ref).write_text(json.dumps(rows, indent=2) + '\n')
matrix_path = ROOT / 'docs/compat/support-matrix.json'
matrix = json.loads(matrix_path.read_text())
for cell in matrix['rows']:
    if cell['display_name'] not in CASES:
        continue
    matched = [r for r in rows if r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit'] and r['case'] in CASES[cell['display_name']]]
    assert len(matched) == len(CASES[cell['display_name']])
    refs = [p for r in matched for p in [r['receipt'], r['log']]] + [summary_ref]
    cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) + refs))
    cell['status'] = 'PARTIAL'
    cell['fixture_readiness'] = 'Core and linked named options qualified with an isolated build and independently checked gameplay witnesses.'
    cell['status_detail'] = ('Isolated f6b9ec4b requalification passes this catalog/revision core and linked named options. '
                             'Earlier shared-target observations remain historical diagnostics. Full supported branches, '
                             'integrated source and frontend acceptance remain separate; existing option evidence is retained.')
matrix_path.write_text(json.dumps(matrix, indent=2) + '\n')
print(json.dumps({'verified_cells': len(rows), 'ledger_rows': len(ledger), 'source_files': len(manifest['files']), 'binary_sha256': build['binary_sha256']}))
