#!/usr/bin/env python3
"""Bounded real-probe smoke on NEW tiny generated input, plus frozen readback."""
import json
from pathlib import Path
import sys
import platform
import argparse
import sharded as sh


def configuration(run,name,source_binding_path=None,source_binding_sha256=None):
    run=sh.stage.owned(run)
    context=sh.stage.activate_run_binding(run,source_binding_path,source_binding_sha256)
    coordinate=context['binding'] is not None
    prefix='coordinate-ebf0f30-qualification-' if coordinate else 'singleton-qualification-'
    if not name.startswith(prefix) or not name.removeprefix(prefix).isdigit():
        raise ValueError('new numbered qualification destination required')
    return dict(coordinate=coordinate,
        schema=sh.COORDINATE_QUALIFICATION_SCHEMA if coordinate else sh.SCHEMA,
        phase='GQ' if coordinate else 'F1',
        candidate_id=context['binding']['candidate_id'] if coordinate else None,
        source_binding=context['reference'],
        reference_arm={'dense':'dense','refined':'tiled'} if coordinate else {'dense':'dense','tiled':'tiled'})


def main(run,name,source_binding_path=None,source_binding_sha256=None):
    run=sh.stage.owned(run)
    config=configuration(run,name,source_binding_path,source_binding_sha256)
    proof=run/name;proof.mkdir()
    guard=sh.stage.support.bounded([sys.executable,'-m','unittest','-v','test_sharded','test_sharded_guards'],
        proof,'scheduler-guards',cwd=sh.stage.HERE,**sh.QUAL_LIMITS)
    if guard['failure'] or b'skipped' in (proof/'scheduler-guards.err').read_bytes():
        raise ValueError('new scheduler guard qualification failed; evidence preserved')
    result=None
    if platform.system()=='Linux':
        pack=run/'fixtures'/f'{name}-tiny.bin'
        # The probe's original collision-map contract is 64x64. A 1x1 header is
        # decoder-valid but drives its fixed tile loops outside the decoded map.
        pack.write_bytes(sh.stage.wire.pack(64,64,origin=(100,200,0),pairs=[(0,False)]*(64*64*4)))
        routes=run/'fixtures'/f'{name}-59.tsv'
        routes.write_text('100 200 0 100 200 0 0 0 0 0\n'*59)
        auth=sh.generated_authorization(run,pack,routes,run/(name+'-smoke'),
            source_binding_path,source_binding_sha256)
        path=proof/(config['phase']+'-authorization.json');sh.write_json(path,auth)
        result=sh.run_phase(run,path,sh.stage.sha(path),standin=True,
            source_binding_path=source_binding_path,source_binding_sha256=source_binding_sha256)
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
                old_arm=config['reference_arm'][arm]
                old=sh.stage.HERE/f'run-05/qualification-{variant}/{old_arm}-{fixture}.out'
                n=sh.stage.output(new,variant=='counting',35)
                o=sh.stage.output(old,variant=='counting',35)
                sh.raw_samples(n[-1],35)
                for key in ('aggregate','logical_cells','lookup_checksum','narrow_checksum','layout','narrow_allocations','narrow_requested_bytes'):
                    assert n[-1][key]==o[-1][key],(arm,variant,fixture,key)
                comparisons.append(dict(arm=arm,variant=variant,fixture=fixture,
                    reference_arm=old_arm,
                    old_sha256=sh.stage.sha(old),new_sha256=sh.stage.sha(new),raw_calls=len(n[-1]['raw_elapsed_ns'])))
    identity={k:v for k,v in config.items() if k not in ('coordinate','reference_arm') and v is not None}
    sh.write_json(proof/'result.json',dict(identity,qualified=True,scope='generated/local tooling only',
        native_hard_as_qualified=platform.system()=='Linux',generated_smoke_executed=result is not None,
        smoke=sh.reference(result) if result is not None else None,tools=sh.tool_hashes(),
        guards={suffix:sh.reference(proof/('scheduler-guards'+suffix)) for suffix in ('.out','.err','.receipt.json')},
        platform=platform.platform(),hardware=sh.stage.hardware(),python_sha256=sh.stage.sha(sys.executable),
        helper_spans=spans,comparisons=comparisons,admissions={f'{a}-{v}':sh.reference(run/f'{a}-{v}-admission.json')
            for a in sh.stage.ARMS for v in ('clean','counting')}))
    print(json.dumps(dict(qualified=True,smoke_children=4 if result is not None else 0,
        native_hard_as_qualified=platform.system()=='Linux',comparisons=len(comparisons),proof=str(proof))))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('run',type=Path);parser.add_argument('name')
    parser.add_argument('--source-binding',type=Path);parser.add_argument('--source-binding-sha256')
    args=parser.parse_args()
    if bool(args.source_binding)!=bool(args.source_binding_sha256):
        parser.error('source binding path and external SHA256 are a pair')
    main(args.run,args.name,args.source_binding,args.source_binding_sha256)
