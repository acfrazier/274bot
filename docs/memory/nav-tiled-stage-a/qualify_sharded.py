#!/usr/bin/env python3
"""Bounded real-probe smoke on NEW tiny generated input, plus frozen readback."""
import json
from pathlib import Path
import sys
import platform
import sharded as sh


def main(run,name):
    run=sh.stage.owned(run)
    if not name.startswith('singleton-qualification-') or not name.removeprefix('singleton-qualification-').isdigit():
        raise ValueError('new numbered qualification destination required')
    proof=run/name;proof.mkdir()
    guard=sh.stage.support.bounded([sys.executable,'-m','unittest','-v','test_sharded','test_sharded_guards'],
        proof,'scheduler-guards',cwd=sh.stage.HERE,**sh.QUAL_LIMITS)
    if guard['failure'] or b'skipped' in (proof/'scheduler-guards.err').read_bytes():
        raise ValueError('new scheduler guard qualification failed; evidence preserved')
    pack=run/'fixtures'/f'{name}-tiny.bin'
    pack.write_bytes(sh.stage.wire.pack(1,1,origin=(100,200,0)))
    routes=run/'fixtures'/f'{name}-59.tsv'
    routes.write_text('100 200 0 100 200 0 0 0 0 0\n'*59)
    auth=sh.generated_authorization(run,pack,routes,run/(name+'-smoke'))
    path=proof/'F1-authorization.json';sh.write_json(path,auth)
    result=sh.run_phase(run,path,sh.stage.sha(path),standin=True)
    receipt=json.loads(result.read_text())
    assert receipt['completed']==4
    assert len(list((run/(name+'-smoke')).rglob('input.bin')))==1
    original=sh.stage.support.git(sh.stage.ROOT,'show','f24de7c:docs/memory/nav-tiled-stage-a/stage_probe.rs').decode()
    current=(sh.stage.HERE/'stage_probe.rs').read_text()
    spans={}
    for label,start,end in [('route_calls','fn call_route(','pub fn run()'),
            ('warm_timed','    let requests: Vec<_>','    drop(requests);'),
            ('owners','    assert_eq!(\n        Arc::strong_count','    let diagnostic =')]:
        a=original[original.index(start):original.index(end)]
        b=current[current.index(start):current.index(end)]
        assert a==b,label
        spans[label]=sh.digest(a.encode())
    comparisons=[]
    for variant in ('clean','counting'):
        for arm in sh.stage.ARMS:
            sh.stage.verify_arm(run,arm,variant)
            for fixture in ('all-uniform','all-dense','gated'):
                new=run/f'qualification-{variant}/{arm}-{fixture}.out'
                old=sh.stage.HERE/f'run-05/qualification-{variant}/{arm}-{fixture}.out'
                n=sh.stage.output(new,variant=='counting',35)
                o=sh.stage.output(old,variant=='counting',35)
                sh.raw_samples(n[-1],35)
                for key in ('aggregate','logical_cells','lookup_checksum','narrow_checksum','layout','narrow_allocations','narrow_requested_bytes'):
                    assert n[-1][key]==o[-1][key],(arm,variant,fixture,key)
                comparisons.append(dict(arm=arm,variant=variant,fixture=fixture,
                    old_sha256=sh.stage.sha(old),new_sha256=sh.stage.sha(new),raw_calls=len(n[-1]['raw_elapsed_ns'])))
    sh.write_json(proof/'result.json',dict(qualified=True,scope='generated/local tooling only',
        native_hard_as_qualified=platform.system()=='Linux',smoke=sh.reference(result),tools=sh.tool_hashes(),
        guards={suffix:sh.reference(proof/('scheduler-guards'+suffix)) for suffix in ('.out','.err','.receipt.json')},
        platform=platform.platform(),hardware=sh.stage.hardware(),python_sha256=sh.stage.sha(sys.executable),
        helper_spans=spans,comparisons=comparisons,admissions={f'{a}-{v}':sh.reference(run/f'{a}-{v}-admission.json')
            for a in sh.stage.ARMS for v in ('clean','counting')}))
    print(json.dumps(dict(qualified=True,smoke_children=4,comparisons=len(comparisons),proof=str(proof))))


if __name__=='__main__':main(Path(sys.argv[1]),sys.argv[2])
