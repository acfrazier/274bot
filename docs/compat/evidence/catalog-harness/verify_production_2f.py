"""Recheck saved 2f production witnesses and native evidence without a LIVE run."""
from pathlib import Path
import hashlib
import json
import re

root = Path(__file__).resolve().parents[4]
e = root / 'docs/compat/evidence'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
source = json.loads((e / 'catalog-headed/source-2f1bd9cf.json').read_text())
for name, digest in source['files'].items():
    assert sha(Path(source['source_root']) / name) == digest, name
build = json.loads((e / 'catalog-headed/binary-headless-2f1bd9cf.json').read_text())
assert build['isolated_build'] and sha(Path(build['binary'])) == build['binary_sha256']
rows = json.loads((e / 'catalog-harness/production-2f1bd9cf.json').read_text())['rows']
assert len(rows) == 8 and sum(r['outcome'] == 'PASS' for r in rows) == 4
for row in rows:
    p = root / row['receipt']
    receipt = json.loads(p.read_text())
    text = p.with_suffix('.log').read_text()
    assert sha(p.with_suffix('.log')) == receipt['log_sha256']
    assert all(receipt[k] == build[k] for k in ['host_commit', 'client_commit', 'binary_sha256'])
    if row['outcome'] == 'FAIL':
        assert receipt['exit_code'] == 1 and 'not impl: Input.invButton' in text
        assert receipt['case'] in ['tanner_bot', 'tanner_bot_hard']
        continue
    assert receipt['exit_code'] == 0 and receipt['case'] == 'vial_filler'
    c = json.loads(next(l.split('PASS: catalog_boundary_live: ', 1)[1]
                       for l in text.splitlines() if l.startswith('PASS: catalog_boundary_live: ')))['core']
    a, z = c['baseline'], c['latest']
    cy = c['vial_filler_cycle']
    f, d, w = [cy[k] for k in ['filled', 'deposited', 'withdrawn']]
    assert a['ingame'] and a['scene_state'] == 2 and not a['item_ids']
    assert a['tick'] < f['tick'] < d['tick'] < w['tick'] < z['tick']
    assert f['item_ids']['227'] > 0 and f['item_ids'].get('229', 0) < 28
    assert d['bank_open'] and d['bank_loaded'] and not d['item_ids'] and d['bank_ids']['227'] == 28
    assert w['bank_open'] and w['bank_loaded'] and w['bank_generation'] == d['bank_generation'] > a['bank_generation']
    assert w['item_ids'] == {'229': 28} and w['bank_ids']['227'] == 28
    assert not z['bank_open'] and z['bank_generation'] > w['bank_generation']
    assert z['item_ids']['227'] > 0 and z['item_ids'].get('229', 0) < 28
    assert cy['returned'] and cy['further']

b = json.loads((e / 'catalog-headed/binary-2f1bd9cf.json').read_text())
p = e / 'catalog-headed/r289-vial-filler-8e7d965b-2f1bd9cf-gpu'
r = json.loads(p.with_suffix('.json').read_text())
assert b['isolated_build'] and sha(Path(b['binary'])) == b['binary_sha256'] == r['binary_sha256']
assert r['exit_code'] == 0 and not r['runtime_errors']
assert sha(p.with_suffix('.log')) == r['log_sha256']
assert sha(p.with_suffix('.timeline.jsonl')) == r['timeline_sha256']
text = p.with_suffix('.log').read_text()
passed = json.loads(next(l.split('PASS: live script_vial_filler ', 1)[1]
                        for l in text.splitlines() if l.startswith('PASS: live script_vial_filler ')))
assert passed['outcome'] == 'PASS' and passed['scene'] == 2
snap_path = next(p.rglob('*.json'))
snap = json.loads(snap_path.read_text())
inv = {}
for item in snap['inventory']:
    inv[item['def']['id']] = inv.get(item['def']['id'], 0) + item['count']
assert snap['ingame'] and snap['scene_state'] == 2 and inv == {227: 1, 229: 27}
counters = [tuple(map(int, m)) for m in re.findall(r'present pixmap=(\d+) tex=(\d+) bind_noop=(\d+) bind_rereg=(\d+)', text)]
assert counters and counters[-1][0] == 0 and counters[-1][1] > 0
png = snap_path.with_suffix('.png')
out = dict(host_commit=b['host_commit'], client_commit=b['client_commit'],
           source_files_verified=len(source['files']), core_passed=4, core_failed=4,
           native=dict(receipt=str(p.with_suffix('.json').relative_to(root)), outcome='PASS',
                       inventory=inv, final_counters=counters[-1],
                       adapter=next(l for l in text.splitlines() if '[panel] adapter' in l),
                       snapshot=str(snap_path.relative_to(root)), snapshot_sha256=sha(snap_path),
                       png=str(png.relative_to(root)), png_sha256=sha(png), visual_read=True,
                       scope='Root read the internal PNG: scene2, Trips1, Filled28, first new water vial after bank return. GPU texture presentation; no complete rendering-equivalence or performance claim.'))
(e / 'catalog-harness/qualification-production-2f1bd9cf.json').write_text(json.dumps(out, indent=2) + '\n')
print('Verified four West vial passes, four retained Tanner mapping failures, and native289 Vial GPU proof.')
