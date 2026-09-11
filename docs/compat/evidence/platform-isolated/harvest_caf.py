"""Verify CAF TUI raw receipts, terminal ordering and observed controls."""
import datetime
import hashlib
import json
import pathlib
import re

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
HOST = 'caf6b809f42294edf80b745208c206fdcccbaa0c'
CLIENT = '9d090ed04957e4efc254f073cda97bc5510ca72b'


def read(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def screen(folder, label):
    p = folder / 'controls' / label
    meta = read(p.with_suffix('.json'))
    assert sha(p.with_suffix('.ansi')) == meta['raw_sha256']
    assert sha(p.with_suffix('.txt')) == meta['screen_sha256']
    return p.with_suffix('.txt').read_text()


def inventory(text):
    match = re.search(r'inv: Rune chainbody x(\d+), Nature rune x(\d+), Coins x(\d+)', text)
    assert match, text
    return tuple(map(int, match.groups()))


rows = []
for surface, revision, label in [
    ('mac', 274, 'controls'), ('mac', 274, 'controls-r2'),
    ('mac', 289, 'controls'), ('concord', 274, 'controls'),
    ('concord', 289, 'controls'),
]:
    folder = HERE / f'{surface}-tui-caf6b809' / f'r{revision}-{label}'
    receipt = read(folder / 'receipt.json')
    raw = (folder / 'terminal.log').read_bytes()
    errors = (folder / 'stderr.log').read_bytes()
    assert receipt['host_commit'] == HOST and receipt['client_commit'] == CLIENT
    assert receipt['exit_code'] == receipt['process_exit_code'] == 0
    assert not receipt['runtime_error'] and not receipt['wall_timeout']
    assert sha(folder / 'terminal.log') == receipt['terminal_sha256']
    assert sha(folder / 'stderr.log') == receipt['stderr_sha256']
    assert len(raw) == receipt['terminal_bytes'] and len(errors) == receipt['stderr_bytes']
    restore = raw.rfind(b'\x1b[?1049l')
    result = raw.find(b'PASS: live alcher ')
    assert 0 <= restore < result and raw.count(b'PASS: live alcher ') == 1
    assert restore == receipt['last_leave_alternate_screen_offset']
    assert result == receipt['pass_offset'] and receipt['pass_after_terminal_restore']
    proof = json.loads(raw[result:].decode().split('PASS: live alcher ', 1)[1].splitlines()[0])
    assert proof['outcome'] == 'PASS' and proof['scene'] == 2
    assert proof['predicate'] == 'stat_xp_gain(6)>=1'
    items = {x['id']: x['count'] for x in proof['inv']}
    assert items[1114] == items[561] == 26 and items[995] == 30000
    forwarded = [json.loads(x) for x in (folder / 'input-forwarded.jsonl').read_text().splitlines()]
    assert len(forwarded) == receipt['input_forwarded_count']
    assert sha(HERE / 'helpers-caf6b809/run-tui-caf-controls.py') == receipt['runner_sha256']
    row = dict(surface=surface, revision=revision, run=label, core_alchemy=True,
               proof_after_terminal_restore=True, binary_sha256=receipt['binary_sha256'],
               receipt=str((folder / 'receipt.json').relative_to(ROOT)))
    if not (surface == 'mac' and revision == 274 and label == 'controls'):
        first, second = screen(folder, 'paused1'), screen(folder, 'paused2')
        assert 'script: paused' in first and 'script: paused' in second
        assert inventory(first) == inventory(second)
        resumed = screen(folder, 'resumed')
        assert 'script: running' in resumed
        before, after = inventory(second), inventory(resumed)
        assert after[0] < before[0] and after[1] < before[1] and after[2] > before[2]
        stop_label = 'stopped-before-restore' if surface == 'concord' and revision == 274 else 'stopped'
        stopped = screen(folder, stop_label)
        assert 'script: idle' in stopped and 'script paint' not in stopped
        assert inventory(stopped)[2] >= after[2]
        row['controls'] = dict(paused_inventory=before, resumed_inventory=after,
                               stopped_inventory=inventory(stopped), pause_resume_stop=True)
    if surface == 'mac' and revision == 274 and label == 'controls':
        assert 'Alcher  [Compat] [Catalog]' in screen(folder, 'browse')
        assert 'sel: Alcher' in screen(folder, 'selected')
        stopped, started = screen(folder, 'stopped-initial'), screen(folder, 'start-selected')
        assert 'script: idle' in stopped and 'script: running' in started
        assert inventory(started)[2] > inventory(stopped)[2]
        row['browse_select_start'] = True
    if surface == 'concord' and revision == 289:
        assert 'Alcher  [Compat] [Catalog]' in screen(folder, 'browsing')
        assert 'sel: Alcher' in screen(folder, 'selected')
        assert 'parameters' in screen(folder, 'params-open')
        restarted = screen(folder, 'restarted')
        assert 'script: running' in restarted and inventory(restarted)[2] > inventory(stopped)[2]
        assert 'script: idle' in screen(folder, 'final')
        row['browse_select_start'] = True
        row['parameter_editing'] = 'BLOCKED: numeric and string-array editors missing; background glyph bleed'
    if surface == 'concord' and revision == 274:
        assert b'login code 5' in errors
        assert b'handshake ok' in errors
        row['transient_login_5_recovered'] = True
    rows.append(row)

for surface in ('mac', 'linux'):
    build = read(HERE / f'{surface}-caf6b809/platform-build.json')
    assert build['host_commit'] == HOST and build['client_commit'] == CLIENT
    assert build['isolated_build'] and build['target_started_empty']
    assert build['source_files_verified'] == 6292
    job, = build['jobs']
    assert job['exit_code'] == 0
    for row in rows:
        if (row['surface'] == 'mac') == (surface == 'mac'):
            assert row['binary_sha256'] == job['binary_sha256']
    for line in (HERE / f'{surface}-caf6b809/build-platform_proof.jsonl').read_text().splitlines():
        value = json.loads(line)
        if value.get('executable') and value.get('target', {}).get('name') == 'platform_proof':
            target = (read(HERE / 'mac-caf6b809/actual-target.json')['actual_target']
                      if surface == 'mac' else build['target_dir'])
            assert pathlib.Path(value['executable']).is_relative_to(target)

for revision in (274, 289):
    stopped = read(HERE / f'concord-tui-caf6b809/engine{revision}-stopped.json')
    assert not any(stopped['ports'].values())
    assert 'ActiveState=inactive' in stopped['original_service']

summary = dict(host_commit=HOST, client_commit=CLIENT, verified_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
               runs=rows, complete_supported_options=False, final_integrated_acceptance=False,
               performance_measurement=False,
               limits=['Real PTY control smoke with existing production TUI and private IsolatedEnv entry.',
                       'Core alchemy predicate is bounded smoke, not full catalog banking acceptance.',
                       'Post-restore pyte snapshots mix normal-screen output; they are not active TUI corruption.',
                       'Parameter editing gap remains pending task t_32400aa2.'])
(HERE / 'qualification-caf6b809.json').write_text(json.dumps(summary, indent=2) + '\n')
print('Verified five CAF TUI runs, deferred proof output, four Pause/Resume/Stop paths and two Browse/Start paths.')
