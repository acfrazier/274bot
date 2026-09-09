"""Generated-only integration assertions, using admitted original frozen probes.
Set NAV_EXTENSION_RUN to a NEW qualified run to audit both complete streams.
No golden routes or foreign policy: only explicit gates and required outcomes.
"""
import json
import os
from pathlib import Path
import unittest
import audit
import harness as h

# Independently explicit expectations, not imported from the fixture generator.
# Full state order: inventory, worn, stats, varps, quests.
STATES = {
    0: b'([], [], [], [], [])',
    1: '([], [], [(2, 3), (6, 25)], [(150, 160)], ["Rune Mysteries", "é"])'.encode(),
    2: b'([(995, 10)], [1712], [], [], [])',
    3: '([(995, 10)], [1712], [(2, 3), (6, 25)], [(150, 160)], ["Rune Mysteries", "é"])'.encode(),
    4: b'([(554, 50), (556, 150), (563, 50)], [], [(6, 25)], [], [])',
    5: b'([(1712, 1)], [], [], [], [])',
    6: b'([(995, 5000)], [], [], [], [])',
}
EXPECT = {
    (1,1,4): 'Teleport', (1,0,4): 'NoPath', (1,1,0): 'NoPath',
    (1,1,1): 'NoPath', (1,1,5): 'NoPath', (1,1,6): 'NoPath',
    (1,5,4): 'Teleport', (1,5,0): 'NoPath',
    (2,1,5): 'Teleport', (2,1,2): 'NoPath', (2,5,0): 'BankSession',
    (2,5,2): 'BankSession', (2,4,0): 'NoPath', (2,1,4): 'NoPath',
    (2,0,5): 'NoPath',
    (3,0,6): 'Boat', (3,0,0): 'NoPath', (3,0,2): 'NoPath',
    (3,4,2): 'NoPath', (3,1,6): 'Boat',
}

def assert_extension(path, protocol=False):
    rows=[]; current=None; active=protocol
    for tag,payload in audit.frames(path):
        if tag=='input':
            assert payload is not None
            active=json.loads(payload)=='inputs/routes-extension.bin'
        if not active: continue
        if tag=='fixed-selector':
            assert payload is not None
            current={'row':json.loads(payload),'frames':{}};rows.append(current)
        elif current is not None:
            current['frames'].setdefault(tag,[]).append(payload)
    seen=set();counts={'Teleport':0,'BankSession':0,'Boat':0,'NoPath':0}
    for index,case in enumerate(rows):
        v=case['row'];f=case['frames']
        # Exact independent state facts also protect unchanged families 0..3.
        assert f.get('post-state')==[STATES[v[7]]], (v,'state',f.get('post-state'))
        if index<90: continue # Original fixed rows remain an unchanged prefix.
        if tuple(v[:5])!=(100,200,0,102,202): continue
        key=(v[5],v[6],v[7])
        if key not in EXPECT: continue
        assert v[8] in (0,4) and v[9] in (0,1)
        selector=tuple(v);assert selector not in seen;seen.add(selector)
        expected=EXPECT[key];counts[expected]+=1
        for tag in ('fixed-model','fixed-tele-model'):
            assert len(f.get(tag,[]))==1,(v,tag)
        if expected=='NoPath':
            assert f.get('host-outcome')==[b'"NoPath"'] and 'host-bank' not in f and 'host-route' not in f,(v,f)
        else:
            route=f['host-route'];assert len(route)==1 and route[0].startswith(b'(Ok(Route {'),(v,f)
            kind='Teleport' if expected=='BankSession' else expected
            assert ('kind: '+kind).encode() in route[0],(v,route)
            if expected=='BankSession':
                bank=f['host-bank'];assert len(bank)==1
                for step in (b'Walk { x: 100, z: 200, level: 0 }',b'Open',b'DepositAll',b'Withdraw { id: 1712, count: 1 }',b'Close',b'allow_bank_fetch: false'):
                    assert step in bank[0],(v,step,bank)
                assert b'Wear {' not in bank[0]
                assert f['host-final']==route,(v,f)
            else: assert 'host-bank' not in f,(v,f)
    expected_rows={(100,200,0,102,202,level,bits,preset,radius,model)
                   for level,bits,preset in EXPECT for radius in (0,4) for model in (0,1)}
    assert len(rows)==170
    assert seen==expected_rows,('missing',expected_rows-seen,'extra',seen-expected_rows)
    return counts

class Extension(unittest.TestCase):
    def test_independent_full_stream_assertions(self):
        name=os.environ.get('NAV_EXTENSION_RUN')
        if not name:self.skipTest('set NAV_EXTENSION_RUN for frozen-arm generated integration')
        run=Path(name).resolve()
        binding=os.environ.get('NAV_COORDINATE_SOURCE_BINDING')
        binding_sha256=os.environ.get('NAV_COORDINATE_SOURCE_BINDING_SHA256')
        if bool(binding)!=bool(binding_sha256):raise ValueError('coordinate binding path/hash are a pair')
        context=h.source_context(run,Path(binding).resolve() if binding else None,binding_sha256)
        for arm in context['arms']:
            h.verify_arm(run,arm,context)
            for path,protocol in [(run/(arm+'-probe.out'),False),(run/'protocol-fixture'/(arm+'.out'),True)]:
                with self.subTest(arm=arm,path=str(path)):
                    counts=assert_extension(path,protocol)
                    self.assertTrue(all(counts.values()))

if __name__=='__main__':unittest.main()
