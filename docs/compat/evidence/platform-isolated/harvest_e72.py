"""Verify e72 TUI parameter controls and immutable build/raw provenance."""
import datetime
import hashlib
import json
import pathlib
import re

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
HOST = 'e72cfb39ef086a89313961be8ca54d1d555e01f4'
CLIENT = '9d090ed04957e4efc254f073cda97bc5510ca72b'


def read(p):
    return json.loads(p.read_text())


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def screen(folder, label):
    p = folder / 'controls' / label
    m = read(p.with_suffix('.json'))
    assert sha(p.with_suffix('.txt')) == m['screen_sha256']
    if 'prefix_bytes' in m:
        raw = (ROOT / m['source']).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == m['source_sha256']
        assert raw.rfind(b'\x1b[?1049l') == m['prefix_bytes']
        assert b'PASS: live alcher ' not in raw[:m['prefix_bytes']]
    else:
        assert sha(p.with_suffix('.ansi')) == m['raw_sha256']
    return p.with_suffix('.txt').read_text()


def inventory(text):
    m = re.search(r'inv: Rune chainbody x(\d+), Nature rune x(\d+), Coins x(\d+)', text)
    assert m
    return tuple(map(int, m.groups()))


archive = read(HERE / 'archive-e72cfb39.json')
assert sha(pathlib.Path(archive['archive'])) == archive['archive_sha256']
package = ROOT / '.superpowers/platform-preparation/platform-isolated-e72cfb39'
manifest = read(package / 'source-manifest.json')
assert len(manifest['files']) == 6779
for name, digest in manifest['files'].items():
    assert sha(package / name) == digest, name
helpers = read(HERE / 'helpers-e72cfb39/manifest.json')
for name, digest in helpers.items():
    assert sha(HERE / 'helpers-e72cfb39' / name) == digest

rows = []
for surface, revision in [('mac', 274), ('concord', 289)]:
    folder = HERE / f'{surface}-tui-e72cfb39/r{revision}-params'
    r = read(folder / 'receipt.json')
    raw = (folder / 'terminal.log').read_bytes()
    assert r['host_commit'] == HOST and r['client_commit'] == CLIENT
    assert r['exit_code'] == r['process_exit_code'] == 0
    assert not r['runtime_error'] and not r['wall_timeout']
    assert sha(folder / 'terminal.log') == r['terminal_sha256']
    assert sha(folder / 'stderr.log') == r['stderr_sha256']
    assert r['runner_sha256'] == helpers['run-tui-e72-controls.py']
    assert r['package_files_verified'] == 6779
    leave = raw.rfind(b'\x1b[?1049l')
    offset = raw.find(b'PASS: live alcher ')
    assert 0 <= leave < offset and raw.count(b'PASS: live alcher ') == 1
    assert leave == r['last_leave_alternate_screen_offset'] and offset == r['pass_offset']
    assert r['pass_after_terminal_restore']
    proof = json.loads(raw[offset:].decode().split('PASS: live alcher ', 1)[1].splitlines()[0])
    assert proof['outcome'] == 'PASS' and proof['scene'] == 2
    assert proof['predicate'] == 'stat_xp_gain(6)>=1'
    assert {x['id']: x['count'] for x in proof['inv']} == {1114: 26, 561: 26, 995: 30000}
    assert len((folder / 'input-forwarded.jsonl').read_text().splitlines()) == r['input_forwarded_count']
    row = dict(surface=surface, revision=revision, core_alchemy=True,
               proof_after_terminal_restore=True, binary_sha256=r['binary_sha256'],
               input_forwarded_count=r['input_forwarded_count'],
               receipt=str((folder / 'receipt.json').relative_to(ROOT)))
    for p in (folder / 'controls').glob('*.json'):
        screen(folder, p.stem)
    if surface == 'mac':
        assert 'Alchs per trip: 27.0_' in screen(folder, 'number-edit')
        saved = screen(folder, 'number-saved-before-exit')
        assert 'Alchs per trip: 3.0' in saved and 'script: idle' in saved
        row['controls'] = dict(numeric_edit_saved='27 to 3', applied_by_fresh_start=False)
    else:
        assert 'Items to alch: custom' in screen(folder, 'custom-saved')
        assert 'invalid number' in screen(folder, 'invalid-number')
        cancelled = screen(folder, 'cancelled')
        assert 'Alchs per trip: 27.0' in cancelled and 'invalid number' not in cancelled
        persisted = screen(folder, 'persisted')
        assert 'Items to alch: rune_chainbody' in persisted
        before = inventory(screen(folder, 'closed'))
        restarted = screen(folder, 'restarted-before-exit')
        after = inventory(restarted)
        assert 'script: running' in restarted and 'Runtime 2s | Alchs 1' in restarted
        assert after[0] < before[0] and after[1] < before[1] and after[2] > before[2]
        row['controls'] = dict(array_edit_saved=True, invalid_number_rejected=True,
                               numeric_cancel_restores_27=True, close_reopen_preserves_array=True,
                               before_start=before, after_start=after, fresh_cast_after_start=True)
    rows.append(row)

for surface in ('mac', 'linux'):
    b = read(HERE / f'{surface}-e72cfb39/platform-build.json')
    v = read(HERE / f'{surface}-e72cfb39/verified-source.json')
    assert b['host_commit'] == HOST and b['client_commit'] == CLIENT
    assert b['isolated_build'] and b['target_started_empty'] and b['source_files_verified'] == 6779
    assert v['archive_sha256'] == archive['archive_sha256'] and v['verified_files'] == 6779
    job, = b['jobs']
    assert job['exit_code'] == 0
    binary = pathlib.Path(job['binary']) if surface == 'mac' else ROOT / '.superpowers/platform-preparation/platform_proof-linux-e72cfb39'
    assert sha(binary) == job['binary_sha256']
    row = next(r for r in rows if (r['surface'] == 'mac') == (surface == 'mac'))
    assert row['binary_sha256'] == job['binary_sha256']
    for line in (HERE / f'{surface}-e72cfb39/build-platform_proof.jsonl').read_text().splitlines():
        value = json.loads(line)
        if value.get('executable') and value.get('target', {}).get('name') == 'platform_proof':
            assert pathlib.Path(value['executable']).is_relative_to(b['target_dir'])

ready = read(HERE / 'concord-tui-e72cfb39/concord-ready.json')
assert ready['host_commit'] == HOST and ready['source_files_verified'] == 6779
assert ready['engine_files_verified'] == 1213
stopped = read(HERE / 'concord-tui-e72cfb39/engine289-stopped.json')
# This receipt records socket.connect_ex errno values: 111 is ECONNREFUSED.
assert all(code == 111 for code in stopped['ports'].values())
assert stopped['owned_engine_stopped'] and 'ActiveState=inactive' in stopped['original_service']
summary = dict(host_commit=HOST, client_commit=CLIENT,
               verified_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), runs=rows,
               source_files_verified=6779, complete_supported_options=False,
               final_integrated_acceptance=False, performance_measurement=False,
               limits=['Existing production TUI plus four-line private IsolatedEnv entry.',
                       'Alchemy predicate is bounded smoke; not full catalog banking acceptance.',
                       'Numeric saved on Mac; no subsequent Start with value3 in that run.',
                       'Imported choices/defaults/showIf missing from shared metadata; task t_10ea7213 audits ownership.',
                       'Mac289 and Concord274 remain covered by prior CAF controls, not these e72 editing runs.',
                       'Post-exit captures are decoded before final terminal restore for active-screen claims.'])
(HERE / 'qualification-e72cfb39.json').write_text(json.dumps(summary, indent=2) + '\n')
print('Verified two e72 TUI runs, numeric/array editing and cancellation, persistence, and fresh Concord Start.')
