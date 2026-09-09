#!/usr/bin/env python3
"""Audit full framed outputs and exact frozen-source provenance, without running probes."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile
import harness as h

def frames(path):
    with path.open('rb') as f:
        while True:
            line=f.readline(256)
            if not line: return
            tag,n=line.decode().rstrip('\n').split(' ');n=int(n)
            if not 0<=n<=4*1024**3: raise ValueError('unbounded frame')
            if tag in ('cells','blocked-words','canonical','round-canonical'):
                f.seek(n,1);payload=None
            else:
                if n>16*1024**2: raise ValueError('audit text frame cap')
                payload=f.read(n)
                assert len(payload)==n
            assert f.read(1)==b'\n'
            yield tag,payload

def audit(run):
    result=json.loads((run/'result.json').read_text());corpus=json.loads((run/'corpus.json').read_text())
    assert result['qualified'] is True
    assert len(corpus)==len({e['path'] for e in corpus})==result['input_count']
    for e in corpus:
        h.admit(run/e['path'],e['sha256'],e['bytes']);h.safe_wire((run/e['path']).read_bytes(),65536)
    launch=json.loads((run/'launch.json').read_text())
    for name,key in [('corpus.json','corpus_sha256'),('selectors.tsv','selectors_sha256'),('fixed.tsv','fixed_sha256')]:
        assert h.sha(run/name)==launch[key]
    for name,digest in launch['tools'].items():
        assert h.sha(run/'tools'/name)==digest
    sources={}
    for arm,rev in [('dense',h.BASE),('tiled',h.CANDIDATE)]:
        h.verify_arm(run,arm)
        d=run/arm;manifest=json.loads((d/'original-source-manifest.json').read_text())
        assert manifest['commit']==rev and manifest['client']==h.CLIENT
        # Compare every source manifest Git oid against the named commit tree,
        # not merely against another self-authored manifest or worktree HEAD.
        git_files={}
        for repo,commit,prefix in [(h.ROOT,rev,''),(h.ROOT/'vendor/fr-client-rust',h.CLIENT,'vendor/fr-client-rust/')]:
            for line in h.git(repo,'ls-tree','-r',commit).decode().splitlines():
                meta,path=line.split('\t');git_files[prefix+path]=meta.split()[2]
        for e in manifest['files']: assert git_files[e['path']]==e['git_blob']
        assert h.git(h.ROOT,'show',rev+':crates/host-play/src/lib.rs')==(d/'original-host-lib.rs').read_bytes()
        for name in ('Cargo.toml','Cargo.lock'):
            assert h.git(h.ROOT,'show',rev+':'+name)==(d/('original-'+name)).read_bytes()
        sources[arm]=dict(commit=rev,files=len(manifest['files']),original_bytes=sum(e['size'] for e in manifest['files']),
            executable_sha256=h.sha(d/'target/debug/differential-probe'),admission_sha256=h.sha(run/(arm+'-admission.json')),
            source_manifest_sha256=h.sha(d/'original-source-manifest.json'),lock_sha256=h.sha(d/'Cargo.lock'))
    for path,digest in result['output_hashes'].items(): assert h.sha(run/path)==digest
    assert h.compare_files(run/'dense-probe.out',run/'tiled-probe.out')==result['comparison']
    assert h.compare_files(run/'protocol-fixture/dense.out',run/'protocol-fixture/tiled.out')==result['generated_protocol_comparison']
    records=Counter();routes=Counter();errors=Counter();inputs=[];noncanonical=[];last_input=None
    # Framing lets the audit skip raw bytes safely; no newline heuristics for
    # binary collision/wire payloads. Full arm byte equality was checked above.
    for tag,payload in frames(run/'dense-probe.out'):
        records[tag]+=1
        if payload is None: continue
        if tag=='input':
            last_input=json.loads(payload);inputs.append(last_input)
        if tag in ('decode-error','side-error','load-error'): errors[payload.decode()]+=1
        if tag in ('model','tele-model','options','walking-options','after-bank','host-route','host-final','fixed-model','fixed-tele-model','budget'):
            if b'Err(NoPath)' in payload: routes['NoPath']+=1
            elif b'Err(BudgetExhausted)' in payload: routes['BudgetExhausted']+=1
            else:
                assert payload.startswith(b'(Ok(Route {')
                routes['Ok']+=1
                for kind in re.findall(rb'kind: (\w+)',payload): routes['leg-'+kind.decode()]+=1
    assert inputs==[e['path'] for e in corpus]
    assert dict(records)==result['output_inventory']
    pairs=5*(3*3*4)**2
    assert records['pair']==pairs
    assert records['options']==records['walking-options']==pairs*4*16
    assert records['budget']==pairs*3
    assert records['step']==(9*512+512)*2*4*2*8
    assert records['radius-selector']==pairs*2
    assert records['fixed-selector']==5*90
    for kind in ('Door','Stairs','Ladder','Teleport','EssenceExit'):
        assert routes['leg-'+kind]>0,kind
    assert records['host-bank']>0 and records['after-bank']>0
    assert routes['NoPath']>0 and routes['BudgetExhausted']>0
    # Verify baseline canonicalization actually changes the noncanonical
    # ordered/trailing streams, rather than claiming input==canonical.
    with (run/'dense-probe.out').open('rb') as f:
        while line:=f.readline():
            tag,n=line.decode().strip().split(' ');n=int(n)
            if tag=='input': last_input=json.loads(f.read(n))
            elif tag=='canonical' and last_input in ('inputs/ordered.bin','inputs/trailing.bin'):
                canonical=f.read(n);assert canonical!=(run/last_input).read_bytes();noncanonical.append(last_input)
            else:f.seek(n,1)
            assert f.read(1)==b'\n'
    assert len(noncanonical)==2
    report=dict(verified=True,input_count=len(corpus),input_bytes=sum(e['bytes'] for e in corpus),max_input_bytes=max(e['bytes'] for e in corpus),
        sources=sources,frames=dict(records),route_results=dict(routes),error_payload_counts=dict(errors),noncanonical_inputs=noncanonical,
        corpus_sha256=h.sha(run/'corpus.json'),result_sha256=h.sha(run/'result.json'))
    h.save(run/'audit.json',report)
    print(json.dumps({k:v for k,v in report.items() if k not in ('error_payload_counts','frames')},indent=2))

def archive(run):
    # Complete retained evidence, including full outputs and frozen input/source
    # bytes. Build target caches/binaries stay local; their hashes are admitted.
    dest=h.HERE/'evidence';dest.mkdir(exist_ok=True)
    path=dest/(run.name+'.tar.gz')
    if path.exists():raise FileExistsError(path)
    entries=[]
    with tarfile.open(path,'w:gz',compresslevel=6) as tar:
        for p in sorted(run.rglob('*')):
            rel=p.relative_to(run)
            if not p.is_file() or 'target' in rel.parts or '__pycache__' in rel.parts:continue
            assert not p.is_symlink()
            tar.add(p,arcname=str(Path(run.name)/rel),recursive=False)
            entries.append(dict(path=str(rel),bytes=p.stat().st_size,sha256=h.sha(p)))
    h.save(dest/(run.name+'-archive.json'),dict(archive_sha256=h.sha(path),bytes=path.stat().st_size,files=entries))
    # Read every archived regular file back and verify its exact bytes.
    with tarfile.open(path,'r:gz') as tar:
        for member,e in zip(tar.getmembers(),entries):
            assert member.name==str(Path(run.name)/e['path'])
            stream=tar.extractfile(member);assert stream is not None
            digest=hashlib.sha256()
            with stream:
                while chunk:=stream.read(1024**2):digest.update(chunk)
            assert digest.hexdigest()==e['sha256']
    print(json.dumps(dict(archive=str(path),bytes=path.stat().st_size,sha256=h.sha(path),files=len(entries))))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('run',type=Path);p.add_argument('--archive-only',action='store_true');a=p.parse_args()
    run=a.run.resolve();assert h.HERE in run.parents
    if a.archive_only:archive(run)
    else:audit(run)
