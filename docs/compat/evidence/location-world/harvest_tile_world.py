"""Preserve the isolated Tile/world batch and its native Gnome observation."""
import hashlib
import json
import pathlib
import subprocess

root = pathlib.Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'],
                    cwd=pathlib.Path(__file__).parent, text=True).strip())
evidence = root / 'docs/compat/evidence'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
build = json.loads((evidence / 'catalog-headed/binary-headless-6401d3a4.json').read_text())
assert build['isolated_build'] and build['exit_code'] == 0
assert sha(pathlib.Path(build['binary'])) == build['binary_sha256']
rows = []
for path in sorted((evidence / 'catalog-harness/live').glob('*-6401d3a4.json')):
    r = json.loads(path.read_text()); log = path.with_suffix('.log')
    assert sha(log) == r['log_sha256'] and r['finished_at']
    assert r['host_commit'] == build['host_commit'] and r['binary_sha256'] == build['binary_sha256']
    row = {k: r[k] for k in ['host_commit', 'client_commit', 'catalog_commit', 'case', 'exit_code', 'elapsed_seconds']}
    row.update(revision=int(r['revision']), receipt=str(path.relative_to(root)), log=str(log.relative_to(root)),
               build_provenance='Isolated target started empty; immutable source verified before and after build.',
               scope='Door/gate core option or failed FlaxPicker default; frontend and other branches remain separate.')
    if r['exit_code']:
        assert r['case'] == 'flax_picker'
        row.update(outcome='FAIL', failure_class='missing_host_compatibility_mapping',
                   failure_excerpt=[line for line in log.read_text().splitlines() if line.startswith('FAIL:')],
                   diagnosis='No flax or interactions after Start. Frozen canReach maps only NPC/ground rows; FlaxPicker asks for loc coordinates. Coordinate mapping t_246e320b and subsequent canStep t_1a213450 remain required. No foreign-script defect established.')
        assert row['failure_excerpt']
    else:
        x = json.loads(next(line.split('PASS: catalog_boundary_live: ', 1)[1]
                            for line in log.read_text().splitlines() if line.startswith('PASS: catalog_boundary_live: ')))['core']
        a, z, cycle = x['baseline'], x['latest'], x['door_opener_cycle']
        assert a['ingame'] and z['ingame'] and a['scene_state'] == z['scene_state'] == 2
        assert a['tick'] < z['tick'] and x['post_start_observations'] > 0 and cycle['opened']
        selected = cycle['selected']; closed_id = 1551 if r['case'] == 'door_opener_gate' else 1530
        assert selected['id'] == closed_id and selected in a['loc_facts']
        assert not any(f['id'] == closed_id and f['x'] == selected['x'] and f['z'] == selected['z'] for f in z['loc_facts'])
        assert any(f['id'] == closed_id + 1 and f['level'] == selected['level'] and max(abs(f['x']-selected['x']), abs(f['z']-selected['z'])) <= 1 for f in z['loc_facts'])
        row.update(outcome='PASS', cycle=cycle, post_start_observations=x['post_start_observations'])
    rows.append(row)
assert len(rows) == 10 and sum(r['outcome'] == 'PASS' for r in rows) == 8
ledger_path = evidence / 'catalog-harness/core-results.json'; ledger = json.loads(ledger_path.read_text())
existing = {r['receipt'] for r in ledger}; ledger.extend(r for r in rows if r['receipt'] not in existing)
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
(evidence / 'location-world/isolated-results-6401d3a4.json').write_text(json.dumps(rows, indent=2) + '\n')
matrix_path = root / 'docs/compat/support-matrix.json'; matrix = json.loads(matrix_path.read_text())
for cell in matrix['rows']:
    if cell['display_name'] != 'DoorOpener': continue
    proofs = [r for r in rows if r['revision'] == cell['revision'] and r['catalog_commit'] == cell['catalog_commit'] and r['outcome'] == 'PASS']
    assert len(proofs) == 2
    cell['status'] = 'PARTIAL'
    cell['fixture_readiness'] = 'Default door and named gate branches pass isolated 6401d3a4 after native Tile distance mapping review.'
    cell['status_detail'] = 'Both branches show the selected closed loc replaced by its open loc after Start. Other supported settings and frontend acceptance remain separate.'
    refs = [p for r in proofs for p in [r['receipt'], r['log']]] + ['docs/compat/evidence/location-world/isolated-results-6401d3a4.json']
    cell['proof_refs'] = list(dict.fromkeys(cell.get('proof_refs', []) + refs))
matrix_path.write_text(json.dumps(matrix, indent=2) + '\n')
native = evidence / 'catalog-headed/r289-gnome-course-8e7d965b-6401d3a4'
n = json.loads(native.with_suffix('.json').read_text())
assert n['exit_code'] == 0 and n['runtime_errors'] == []
assert sha(native.with_suffix('.log')) == n['log_sha256'] and sha(native.with_suffix('.timeline.jsonl')) == n['timeline_sha256']
text = native.with_suffix('.log').read_text()
assert 'PASS: live script_gnome_course' in text and 'lap 1 complete' in text and 'lap 2 complete' in text
for level in [1, 2]: assert f'level: {level}, action:' in text
shots = {str(p.relative_to(root)): sha(p) for p in native.rglob('*') if p.is_file()}
assert any(p.endswith('.png') for p in shots)
proof = dict(receipt=str(native.with_suffix('.json').relative_to(root)), shots=shots,
             observed='Root read native panel during elevated rope/tree sections and later laps, plus the stored terminal PNG showing lap 1, seven obstacles and ground scene. Log records later lap completion and actual levels 1 and 2.',
             limits='One representative newer-catalog 289 native GPU run. F12 is disabled during live scenarios, so intermediate CUA screenshots are conversation observations; the terminal PNG is the persisted capture. No all-settings, lifecycle, 274 frontend or performance claim.')
(evidence / 'location-world/native-gnome-289-6401d3a4.json').write_text(json.dumps(proof, indent=2) + '\n')
print(json.dumps({'cells':len(rows),'pass':8,'fail':2,'ledger_rows':len(ledger),'native_exit':n['exit_code']}))
