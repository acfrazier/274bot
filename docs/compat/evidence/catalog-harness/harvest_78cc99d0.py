"""Verify count-dialog progress and retain the four Tanner modal failures."""
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[4]
E = ROOT / 'docs/compat/evidence'
SHORT = '78cc99d0'


def read(p):
    return json.loads(p.read_text())


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def compact(o):
    if o is None:
        return None
    return {k: o[k] for k in ('tick', 'tile', 'item_ids', 'bank_ids',
                              'bank_generation', 'bank_open', 'main_modal', 'widget_ids')}


source = read(E / f'catalog-headed/source-{SHORT}.json')
build = read(E / f'catalog-headed/binary-headless-{SHORT}.json')
gate = read(E / f'catalog-headed/review-gates-{SHORT}.json')
checks = read(E / f'catalog-headed/checks-{SHORT}-inputs.json')
assert checks['all_passed'] and gate['all_required_reviews_approved'] and gate['root_fixture_checks_passed']
assert build['isolated_build'] and build['target_started_empty'] and build['exit_code'] == 0
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
for name, digest in source['files'].items():
    assert sha(pathlib.Path(source['source_root']) / name) == digest, name
for check in checks['checks'] + [checks['prior_count_dialog_pass'], checks['prior_count_dialog_clippy_pass']]:
    assert check['exit_code'] == 0 and sha(ROOT / check['log']) == check['log_sha256']

rows = []
for path in sorted((E / 'catalog-harness/live').glob(f'*-{SHORT}.json')):
    r = read(path)
    text = path.with_suffix('.log').read_text()
    assert sha(path.with_suffix('.log')) == r['log_sha256']
    assert r['exit_code'] == 1 and r['case'] in ('tanner_bot', 'tanner_bot_hard')
    assert all(r[k] == build[k] for k in ('host_commit', 'client_commit', 'binary_sha256'))
    identity = json.loads(next(l for l in text.splitlines() if l.startswith('{"phase":"identity"')))
    assert sha(pathlib.Path(identity['card']['source_path'])) == identity['card']['source_sha256']
    failures = [l for l in text.splitlines() if l.startswith('FAIL:')]
    assert len(failures) == 1 and 'not impl' not in text
    product = '1741' if r['case'] == 'tanner_bot' else '1743'
    assert f'has_item_id({product})>=1 not seen within 150 ticks' in failures[0]
    assert '[shim-count] 27 sent' in text and 'interact AnswerCount { value: 27 }' in text
    assert 'interact Npc { name: "Tanner", action: "Trade"' in text
    assert 'tanner interface did not open' in text and 'outcome=Some(Arrived' in text
    c = json.loads(failures[0].split('; core=', 1)[1])['witness']
    a, z, cy = c['baseline'], c['latest'], c['tanner_bot_cycle']
    assert all(o['ingame'] and o['scene_state'] == 2 for o in (a, z))
    assert a['tick'] < z['tick'] and not a['item_ids'] and c['post_start_observations'] > 0
    assert c['max_items'].get('Cow hide', c['max_items'].get('Cowhide')) == 27
    assert c['max_items']['Coins'] == 2000 and not cy['tanned'] and not cy['further']
    assert not cy['deposited'] and not cy['withdrawn']
    if cy['widget']:
        assert cy['widget']['main_modal'] == 679
        assert ('8686' if r['case'] == 'tanner_bot' else '8690') in map(str, cy['widget']['widget_ids'])
        assert cy['widget']['item_ids'] == {'995': 2000, '1739': 27}
    row = {k: r[k] for k in ('host_commit', 'client_commit', 'catalog_commit', 'case', 'exit_code', 'elapsed_seconds')}
    row.update(revision=int(r['revision']), outcome='FAIL', receipt=str(path.relative_to(ROOT)),
               log=str(path.with_suffix('.log').relative_to(ROOT)),
               build_provenance='3364 exact Git source blobs and isolated binary verified.',
               failure_class='tanner_trade_modal_timing_or_publication_pending_audit',
               failure_excerpt=failures[0].split('; evidence=', 1)[0],
               diagnosis='Actual hide withdrawal now works; selected Tanner Trade and nav arrival observed. Some cells also observe real interface679. Script times out before tanning; task t_59351ef0 distinguishes timing, dispatch and publication without changing timeouts or foreign source.',
               baseline=compact(a), latest=compact(z), actual_tanner_widget=compact(cy['widget']),
               scope='Count-dialog progress only; no tanning or full bank-cycle acceptance.')
    rows.append(row)
assert len(rows) == 4 and {(r['revision'], r['case']) for r in rows} == {
    (r, c) for r in (274, 289) for c in ('tanner_bot', 'tanner_bot_hard')}
for p in (E / 'catalog-harness/live').glob(f'*-{SHORT}.process.json'):
    assert p.with_name(p.name.replace('.process.json', '.json')).exists(), p

native = E / f'catalog-headed/r274-tanner-bot-8e7d965b-{SHORT}-gpu'
nr = read(native.with_suffix('.json'))
nb = read(E / f'catalog-headed/binary-{SHORT}.json')
assert nr['exit_code'] == nr['process_exit_code'] == 1
assert nr['binary_sha256'] == nb['binary_sha256'] == sha(pathlib.Path(nb['binary']))
assert sha(native.with_suffix('.log')) == nr['log_sha256']
assert sha(native.with_suffix('.timeline.jsonl')) == nr['timeline_sha256']
text = native.with_suffix('.log').read_text()
result = json.loads(next(l.removeprefix('FAIL: live script_tanner_bot ') for l in text.splitlines()
                         if l.startswith('FAIL: live script_tanner_bot ')))
assert result['outcome'] == 'FAIL' and result['predicate'] == 'has_item_id(1741)>=1'
assert sum(i['count'] for i in result['inv'] if i['id'] == 1739) == 27
assert sum(i['count'] for i in result['inv'] if i['id'] == 995) == 2000
snapshot = next(native.rglob('*.json'))
s = read(snapshot)
assert s['ingame'] and s['scene_state'] == 2
assert sum(i[1] for i in s['inv'] if i[0] == 1739) == 27
counts = [list(map(int, x)) for x in re.findall(r'present pixmap=(\d+) tex=(\d+) bind_noop=(\d+) bind_rereg=(\d+)', text)]
native_row = dict(receipt=str(native.with_suffix('.json').relative_to(ROOT)), outcome='FAIL',
                  process_elapsed_seconds=nr['elapsed_seconds'], scenario_elapsed_ms=result['elapsed_ms'],
                  snapshot=str(snapshot.relative_to(ROOT)), snapshot_sha256=sha(snapshot),
                  png=str(snapshot.with_suffix('.png').relative_to(ROOT)), png_sha256=sha(snapshot.with_suffix('.png')),
                  visual_read=True, cua_visual_read=True, last_present_counters=counts[-1] if counts else None,
                  observed='Native panel first showed bank, then terminal27hides/Coins2000/Trips0/Tanned0 on another Tanner walk. Real withdrawal; no tanning success. Profiles pane opened through actual CUA click; rendering is diagnostic only.')
summary = dict(source_files_verified=len(source['files']), passed=0, failed=4, rows=rows,
               native=native_row, performance_measurement=False, final_integrated_acceptance=False)
(E / f'catalog-harness/tanner-{SHORT}.json').write_text(json.dumps(summary, indent=2) + '\n')
ledger_path = E / 'catalog-harness/core-results.json'
ledger = read(ledger_path)
old = {r['receipt'] for r in ledger}
ledger.extend(r for r in rows if r['receipt'] not in old)
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
matrix_path = ROOT / 'docs/compat/support-matrix.json'
matrix = read(matrix_path)
for cell in matrix['rows']:
    if cell['display_name'] != 'TannerBot':
        continue
    selected = [r for r in rows if r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit']]
    refs = [r['receipt'] for r in selected]
    if cell['revision'] == 274 and cell['catalog_commit'].startswith('8e7d965b'):
        refs.append(native_row['receipt'])
    if refs:
        detail = 'Isolated78: actual27hide count withdrawal succeeds; Tanner Trade/modal wait fails before conversion. Some soft cells observe interface679; exact timing/publication audit pending. No tanning acceptance.'
        cell.update(status='PARTIAL', status_detail=detail, fixture_readiness=detail)
        cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) + refs + [f'docs/compat/evidence/catalog-harness/tanner-{SHORT}.json']))
matrix_path.write_text(json.dumps(matrix, indent=2) + '\n')
print(json.dumps(dict(completed=4, passed=0, failed=4, ledger_rows=len(ledger), native=native_row)))
