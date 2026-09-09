"""Generated protocol fixtures, not measured owner evidence or live admission."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import validate_direct_owner_capture as v


def fixture():
    def record(kind, **kw):
        return dict(schema=v.SCHEMA, kind=kind, **kw)
    def field(owner, name):
        if (owner, name) in v.OPAQUE:
            return dict(owner=owner, field=name, complete=False, reason='opaque_unknown',
                        elapsed_observer_ns=0, **dict.fromkeys(v.NUMBERS))
        return dict(owner=owner, field=name, complete=True, reason='ok',
                    elapsed_observer_ns=0, **dict.fromkeys(v.NUMBERS, 0))
    def encoded(kind, request, when):
        return record('encoded_buf_meta', slot_token=1, buf_kind=kind,
                      request_id=request, mono_ns=when, frame_serial=1, len=1, capacity=8)
    rows = [encoded(0, 0, 1), encoded(1, 0, 2)]
    for p in range(3):
        issued = (p + 1) * 100_000_000_000
        if p == 2:
            rows.append(record('script_stop', begin_ns=250_000_000_000, end_ns=250_000_000_001))
        rows.append(record('owner_request', phase=p, request_id=p+1, slot_token=1,
                           issued_ns=issued, deadline_ns=issued+1_000_000_000))
        fields = [field(o, n) for o in ('host_snapshot', 'nav_snapshot') for n in sorted(v.SNAPSHOT)]
        for owner, names in {
            'world': 'struct_header groundh squares sprites dynamic_sprites free_dynamic_sprites occluders occlusion_cycle',
            'client': ' '.join(sorted(v.CLIENT)),
            'ifaces_mut': 'outer template_entries private_entries shared_entries',
        }.items():
            fields += [field(owner, n) for n in names.split()]
        epochs = [dict(source=o, ingame=True, scene_state=2, draw=False, gens=[0]*11,
                       base=[0,0], tile=[0,0,0], family_gates=[True]*25)
                  for o in ('client', 'host_snapshot', 'nav_snapshot')]
        for source in ('nav_snapshot', 'slotscript'):
            if source == 'slotscript':
                fields = [field('fingerprint', n) for n in sorted(v.FINGERPRINT if p != 2 else {'absent','builder_capacity'})]
                fields += [field('slotscript', n) for n in ('pending_logs','last_error','ipc_builder_capacity','compiled_or_isolate_native')]
            rows.append(record('owner_fragment', phase=p, request_id=p+1, slot_token=1,
                               frame_serial=p+1, source=source, begin_ns=issued+1+(source=='slotscript'),
                               end_ns=issued+1+(source=='slotscript'), visits=1, complete=True,
                               reason='ok', rows=fields, epochs=epochs, script_state='Idle' if p==2 else 'Running',
                               fingerprint_present=p!=2))
        if p < 2:
            rows.append(encoded(2,p+1,issued+3))
    rows.append(record('owner_terminal', complete=True, reason='ok'))
    return rows


class ValidationTests(unittest.TestCase):
    def test_complete_generated_protocol(self):
        self.assertFalse(v.validate_rows(fixture())['native_qualified'])

    def test_negative_mutations(self):
        mutations = [
            lambda r: r.pop(),
            lambda r: r.insert(3, copy.deepcopy(r[2])),
            lambda r: r[3].update(slot_token=2),
            lambda r: r[3].pop('slot_token'),
            lambda r: r[3].update(request_id=2),
            lambda r: r[3].update(begin_ns=0),
            lambda r: r[3].update(visits=262145),
            lambda r: r[3].update(complete=False),
            lambda r: r[3]['rows'].pop(),
            lambda r: r[3]['epochs'][0].update(scene_state=1),
            lambda r: r[4].update(frame_serial=99),
            lambda r: r[0].update(capacity=0),
            lambda r: r[5].update(mono_ns=999999999999),
            lambda r: r[3]['rows'][0].update(capacity_bytes=-1),
            lambda r: r[3]['rows'].append(copy.deepcopy(r[3]['rows'][0])),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                rows = fixture()
                mutate(rows)
                with self.assertRaises(ValueError):
                    v.validate_rows(rows)

    def test_file_identity_truncation_and_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'owner.jsonl'
            data = b''.join(json.dumps(r).encode()+b'\n' for r in fixture())
            path.write_bytes(data)
            v.validate_file(path, hashlib.sha256(data).hexdigest())
            with self.assertRaises(ValueError):
                v.validate_file(path, '0'*64)
            for malformed in (data[:-1], b'{"kind":1,"kind":2}\n', b'x'*262145):
                path.write_bytes(malformed)
                with self.assertRaises(ValueError):
                    v.validate_file(path, hashlib.sha256(malformed).hexdigest())


if __name__ == '__main__':
    unittest.main()
