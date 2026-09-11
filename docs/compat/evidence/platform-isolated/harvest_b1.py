"""Recheck the retained B1 platform observations; preserve qualification limits."""
from pathlib import Path
import datetime
import hashlib
import json
import re

root = Path(__file__).resolve().parents[4]
e = root / 'docs/compat/evidence'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
result = {'verified_at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'mac_native': []}
b = json.loads((e / 'catalog-headed/binary-b1cff8a7.json').read_text())
assert b['isolated_build'] and sha(Path(b['binary'])) == b['binary_sha256']
for p in sorted((e / 'catalog-headed').glob('r*-b1cff8a7*.json')):
    if not re.match(r'^r(274|289)-', p.name) or p.name.endswith('.process.json'):
        continue
    r = json.loads(p.read_text())
    assert r['binary_sha256'] == b['binary_sha256']
    assert sha(p.with_suffix('.log')) == r['log_sha256']
    assert sha(p.with_suffix('.timeline.jsonl')) == r['timeline_sha256']
    body = p.with_suffix('.log').read_text()
    snap_path = next(p.with_suffix('').rglob('*.json'))
    snap = json.loads(snap_path.read_text())
    png = snap_path.with_suffix('.png')
    assert png.exists() and snap['ingame'] and snap['scene_state'] == 2
    counts = {}
    for x in snap['inventory']:
        counts[x['def']['id']] = counts.get(x['def']['id'], 0) + x['count']
    counters = [tuple(map(int, m)) for m in re.findall(
        r'present pixmap=(\d+) tex=(\d+) bind_noop=(\d+) bind_rereg=(\d+)', body)]
    route = 'not sampled'
    if counters:
        route = ('CPU pixmap' if counters[-1][0] > 0 and counters[-1][1] == 0 else
                 'GPU texture' if counters[-1][1] > 0 and counters[-1][0] == 0 else 'mixed')
    outcome = 'FAIL' if r['exit_code'] else 'PASS'
    if outcome == 'PASS':
        assert 'PASS: live script_alcher ' in body
        assert counts == {1114: 26, 561: 26, 995: 30000} and not r['runtime_errors']
    else:
        assert 'flax-picker' in p.name and counts == {1779: 6}
    result['mac_native'].append(dict(
        receipt=str(p.relative_to(root)), outcome=outcome, scene_state=2, inventory=counts,
        route=route, final_counters=counters[-1] if counters else None,
        adapter=next(l for l in body.splitlines() if '[panel] adapter' in l),
        snapshot=str(snap_path.relative_to(root)), snapshot_sha256=sha(snap_path),
        png=str(png.relative_to(root)), png_sha256=sha(png), visual_read=True,
        scope='Core/native rendering observation only; every PNG read by root. Terminal paint can lag paired inventory snapshot. Flax failure remains unqualified gameplay. No performance or complete rendering-equivalence claim.'))
assert len(result['mac_native']) == 6

p = e / 'platform-isolated/windows-native-b1cff8a7'
r = json.loads((p / 'receipt.json').read_text())
assert r['exit_code'] == 0 and not r['runtime_errors']
for name, h in r['evidence_files'].items():
    assert sha(p / name) == h, name
wbuild = json.loads((e / 'platform-isolated/windows-b1cff8a7/platform-build.json').read_text())
assert wbuild['isolated_build'] and wbuild['target_started_empty']
assert next(j for j in wbuild['jobs'] if j['name'] == 'catalog_watch')['binary_sha256'] == r['binary_sha256']
s = json.loads(next((p / 'shots').rglob('*.json')).read_text())
assert s['ingame'] and s['scene_state'] == 2
inv = {x['def']['id']: x['count'] for x in s['inventory']}
assert inv == {1114: 26, 561: 26, 995: 30000}
assert next(x['xp'] for x in s['stats'] if x['name'] == 'magic') == 166701
probes = [json.loads(l) for l in (p / 'window-responsiveness.jsonl').read_text().splitlines()]
bad = [x for x in probes if not x['responsive']]
assert len(probes) == 225 and len(bad) == 2 and all(x['elapsed_seconds'] < 4 for x in bad)
assert json.loads((p / 'task-cleanup.json').read_text())['verified_absent']
result['windows_native'] = dict(
    outcome='PASS', receipt=str((p / 'receipt.json').relative_to(root)), inventory=inv,
    magic_xp=166701, adapter=next(l for l in (p / 'run.log').read_text().splitlines() if '[panel] adapter' in l),
    visual_read=True, probe_count=len(probes), startup_timeouts=bad,
    scope='289 old-catalog Alcher core and read internal rendered PNG. No frame counters sampled, so actual client GPU routing and complete frontend controls remain unqualified. Owned one-off task removed.')

p = e / 'platform-isolated/linux-b1cff8a7/live/r289-alcher-8e7d965b.json'
r = json.loads(p.read_text())
assert r['exit_code'] == 0 and sha(p.with_suffix('.log')) == r['log_sha256']
c = json.loads(next(l.split('PASS: catalog_boundary_live: ', 1)[1]
                   for l in p.with_suffix('.log').read_text().splitlines()
                   if l.startswith('PASS: catalog_boundary_live: ')))['core']
a, z = c['baseline'], c['latest']
assert z['xp']['magic'] > a['xp']['magic'] and z['item_ids']['995'] > a['item_ids'].get('995', 0)
result['linux_headless'] = dict(
    outcome='PASS', receipt=str(p.relative_to(root)), magic_xp_delta=z['xp']['magic'] - a['xp']['magic'],
    item_ids=z['item_ids'], scope='289 new-catalog Alcher core in isolated Linux build, not TUI/frontend/performance proof.')

p = e / 'platform-isolated/concord-tui-b1cff8a7'
result['concord_tui'] = []
for variant in ['r289-alcher', 'r289-alcher-controls']:
    d = p / variant
    r = json.loads((d / 'receipt.json').read_text())
    assert r['exit_code'] == 0 and r['pass_marker'] and not r['runtime_error']
    assert sha(d / 'terminal.log') == r['terminal_sha256']
    raw = (d / 'terminal.log').read_text(errors='replace')
    passed = json.JSONDecoder().raw_decode(raw.split('PASS: live alcher ', 1)[1])[0]
    assert passed['outcome'] == 'PASS' and passed['scene'] == 2
    assert {x['id']: x['count'] for x in passed['inv']} == {1114: 26, 561: 26, 995: 30000}
    if variant.endswith('controls'):
        assert sha(d / 'stderr.log') == r['stderr_sha256'] and r['input_forwarded_count'] == 3
    result['concord_tui'].append(dict(
        outcome='PASS', receipt=str((d / 'receipt.json').relative_to(root)), pass_evidence=passed,
        scope='Actual B1 289 new-catalog PTY TUI core. First run stderr interleaves display; separate-controls run still exposes PASS stdout corruption. No full clean-screen acceptance.'))
controls = p / 'controls'
screens = {n: (controls / (n + '.txt')).read_text() for n in ['paused1', 'paused2', 'resumed', 'stopped']}
for n in ['paused1', 'paused2']:
    assert 'script: paused' in screens[n] and '[Resume]' in screens[n]
    assert 'Rune chainbody x7, Nature rune x7, Coins x600000' in screens[n]
assert 'script: running' in screens['resumed'] and 'Rune chainbody, Nature rune, Coins x780000' in screens['resumed']
assert 'script: idle' in screens['stopped'] and 'Coins x810000' in screens['stopped'] and '17: ┌chat' in screens['stopped']
for path in controls.glob('*.json'):
    r = json.loads(path.read_text())
    assert sha(path.with_suffix('.ansi')) == r['raw_sha256'] and sha(path.with_suffix('.txt')) == r['screen_sha256']
result['tui_controls'] = dict(
    pause_inventory_stable=True, post_resume_coins=780000, paused_coins=600000,
    stopped_coins_after_inflight=810000, stop_idle=True, forwarded_inputs=3,
    scope='Actual terminal mouse Pause/Resume/Stop and observed UI/inventory states. Start/Browse/settings/reconnect remain separate. PASS output display defect assigned t_35ee61ac.')
assert json.loads((p / 'engine-stop.json').read_text())['engine_absent']
result['concord_engine_stopped'] = True
result['native_controls'] = dict(
    observations='docs/compat/evidence/catalog-headed/native-controls-b1-actions.jsonl',
    sha256=sha(e / 'catalog-headed/native-controls-b1-actions.jsonl'),
    qualified=['Pause after drain', 'Resume with new work', 'Stop idle/paint clear', 'Start with new work'],
    limits='CUA native production controls observed by root; durable terminal internal captures identify the same binary/session but are not screenshots of each action. First run ended before Resume/Stop.')
(e / 'platform-isolated/qualification-b1cff8a7.json').write_text(json.dumps(result, indent=2) + '\n')
print('Verified B1 native6, Windows, Linux, Concord2 and controls; limitations retained')
