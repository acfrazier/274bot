#!/usr/bin/env python3
"""Frozen Stage A preparation/admission/qualification. Real runs need root release."""
import argparse
import hashlib
import json
import math
import mmap
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys

import frozen_support as support
import frozen_generate as wire

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
ARMS = {'dense': support.BASE, 'tiled': support.CANDIDATE}
FROZEN = {
    'frozen_support.py': '099c4bf8e25bf09b6181e5d51575872340f3b27fc286341315fc6fa799969ba6',
    'frozen_probe.rs': 'ee91e2370d04de8a1c9b9d55eaf9cbb3c8bbd9d0153b4748208fc50656e40cd8',
    'frozen_generate.py': 'f51c88b73186b9e7a50ea45494cc160523339d2be9c6e10f8134d7de05bdd784',
}
TOOLS = (*FROZEN, 'stage_a.py', 'stage_probe.rs', 'allocator.rs', 'test_stage_a.py', 'proposed-manifest.json')
LIMITS = dict(wall=120, cpu=90, rss=1024**3, address=4*1024**3, output=256*1024)
BUILD = dict(support.BUILD)
PHASES = ['startup', 'retained_input', 'decoded_converted_retained_input',
          'dropped_input_world_live', 'hot_lookup', 'hot_routes', 'world_drop', 'post_drop']
NAV_SUFFIX = ('\n[[bin]]\nname="differential-probe"\npath="src/differential.rs"\n'
              '\n[[bin]]\nname="stage-a-probe"\npath="src/stage.rs"\n'
              '\n[dependencies.libc]\nversion="0.2"\n\n[features]\nstage-counting=[]\n')
ROUTER_SUFFIX = '\n#[path="router-access.rs"]\npub mod differential_access;\n'
COLLISION_SUFFIX = '\n#[path="stage-layout.rs"]\npub mod stage_layout;\n'
sha = support.sha
save = support.save
admit = support.admit

def owned(path):
    path = Path(path).absolute()
    if HERE not in path.parents or path == HERE:
        raise ValueError('output must be below owned Stage A directory')
    if any(p.is_symlink() for p in [path, *path.parents]):
        raise ValueError('symlink output path')
    return path

def tools():
    for name, digest in FROZEN.items(): admit(HERE/name, digest, 128*1024)
    return {n: sha(HERE/n) for n in TOOLS}

def hardware():
    u=platform.uname()
    return dict(system=u.system,node=u.node,release=u.release,machine=u.machine,cpus=os.cpu_count())

def no_ambient_config(dest):
    # Cargo ancestor/home configuration can override declared release settings.
    # Do not read credentials; reject config presence rather than silently inherit.
    for base in [dest,*dest.parents,Path.home()]:
        for name in ('config','config.toml'):
            if (base/'.cargo'/name).exists(): raise ValueError('ambient Cargo config must be explicitly removed from the measurement environment: '+str(base/'.cargo'/name))

def tree(path):
    out = {}
    for p in sorted(path.rglob('*')):
        rel = p.relative_to(path)
        if rel.parts[0].startswith('target-'): continue
        if p.is_symlink(): raise ValueError('symlink source')
        if p.is_file(): out[str(rel)] = sha(p)
    return out

def span(text, marker):
    start = text.index(marker); end = text.index('\n}', start)+2
    return text[start:end]

def layout_source(arm):
    common = '''use super::*;
fn usable(p: *const u8, bytes: usize) -> usize {
    if bytes == 0 { return 0; }
    #[cfg(target_os="linux")] unsafe { libc::malloc_usable_size(p as *mut libc::c_void) }
    #[cfg(target_os="macos")] unsafe { libc::malloc_size(p as *const libc::c_void) }
}
'''
    if arm == 'dense':
        return common+'''pub fn geometry(c: &WorldCollision)->(WorldTile,usize,usize,usize) {(c.origin,c.width,c.height,c.walk.len())}
pub fn layout(c: &WorldCollision)->[usize;10] {
    let walk=c.walk.capacity(); let blocked=c.blocked.capacity()*8;
    [0,0,0,0,walk,blocked,usable(c.walk.as_ptr(),walk),usable(c.blocked.as_ptr() as *const u8,blocked),std::mem::size_of::<WorldCollision>(),0]
}
'''
    return common+'''pub fn geometry(c: &WorldCollision)->(WorldTile,usize,usize,usize) {(c.origin(),c.width(),c.height(),c.logical_cell_count())}
pub fn layout(c: &WorldCollision)->[usize;10] {
    assert_eq!(std::mem::size_of::<DenseTile>(),1152); assert_eq!(std::mem::align_of::<DenseTile>(),8);
    let dir=c.directory.len()*std::mem::size_of::<u64>(); let pool=c.dense.len()*std::mem::size_of::<DenseTile>();
    [c.directory.len(),dir,c.dense.len(),pool,0,0,usable(c.directory.as_ptr() as *const u8,dir),usable(c.dense.as_ptr() as *const u8,pool),std::mem::size_of::<WorldCollision>(),std::mem::align_of::<DenseTile>()]
}
'''

def prepare(run):
    run=owned(run); run.mkdir(parents=True,exist_ok=False)
    tool_hashes=tools()
    # Copied helper executes materialization only; its correctness probe is never run.
    support.HERE=run/'immutable-helper'; support.HERE.mkdir()
    shutil.copyfile(HERE/'frozen_probe.rs',support.HERE/'probe.rs')
    support.ROOT=ROOT
    support.materialize(run)
    facts=span((HERE/'frozen_probe.rs').read_text(),'fn facts(')
    for arm in ARMS:
        dest=run/arm; nav=dest/'crates/nav'; src=nav/'src'
        original=support.git(ROOT,'show',ARMS[arm]+':crates/nav/Cargo.toml')
        (nav/'Cargo.toml').write_bytes(original+NAV_SUFFIX.encode())
        (src/'collision.rs').write_bytes((src/'collision.rs').read_bytes()+COLLISION_SUFFIX.encode())
        (src/'stage-layout.rs').write_text(layout_source(arm))
        (src/'stage-facts.rs').write_text(facts+'\n')
        (src/'stage.rs').write_text((src/'lib.rs').read_text()+'\nmod stage_probe;\nfn main(){stage_probe::run();}\n')
        shutil.copyfile(HERE/'stage_probe.rs',src/'stage_probe.rs')
        shutil.copyfile(HERE/'allocator.rs',src/'stage-allocator.rs')
        verify_original(dest, arm)
    save(run/'prepared.json',dict(tools=tool_hashes,arms=ARMS,client=support.CLIENT,
         trees={a:tree(run/a) for a in ARMS}, compiler=support.toolchain()))
    fixtures(run)

def verify_original(dest, arm):
    manifest=json.loads((dest/'original-source-manifest.json').read_text())
    if manifest['commit'] != ARMS[arm] or manifest['client'] != support.CLIENT:
        raise ValueError('wrong arm source')
    suffixes={'crates/nav/Cargo.toml':NAV_SUFFIX,'crates/nav/src/router.rs':ROUTER_SUFFIX,
              'crates/nav/src/collision.rs':COLLISION_SUFFIX}
    for e in manifest['files']:
        data=(dest/e['path']).read_bytes(); suffix=suffixes.get(e['path'],'').encode()
        if len(data)!=e['size']+len(suffix) or (suffix and not data.endswith(suffix)):
            raise ValueError('original source suffix changed: '+e['path'])
        original=data[:e['size']]
        if hashlib.sha256(original).hexdigest()!=e['sha256'] or hashlib.sha1(b'blob '+str(len(original)).encode()+b'\0'+original).hexdigest()!=e['git_blob']:
            raise ValueError('original source hash changed: '+e['path'])
    host=(dest/'original-host-lib.rs').read_bytes(); spans=json.loads((dest/'host-spans.json').read_text())
    if spans['commit']!=ARMS[arm] or hashlib.sha256(host).hexdigest()!=spans['source_sha256']: raise ValueError('host source mismatch')
    extracts=[]
    for s in spans['spans']:
        b=host[s['start_byte']:s['end_byte']]
        if hashlib.sha256(b).hexdigest()!=s['sha256']: raise ValueError('host span mismatch')
        extracts.append(b)
    if (dest/'crates/nav/src/host-probe.rs').read_bytes()!=b'\n\n'.join(extracts)+b'\n': raise ValueError('host helper mismatch')

def build(run, variant):
    run=owned(run); prep=json.loads((run/'prepared.json').read_text())
    if prep['tools']!=tools(): raise ValueError('prepared tool mutation')
    for arm in ARMS:
        receipt_path=run/f'{arm}-{variant}-admission.json'
        if receipt_path.exists(): raise ValueError('admitted binary cannot be rebuilt; prepare a new run')
        dest=run/arm; verify_original(dest,arm)
        no_ambient_config(dest)
        # Cargo can prune the lock on the FIRST build only. Freeze all other files.
        actual=tree(dest); expected=prep['trees'][arm]
        if {k:v for k,v in actual.items() if k!='Cargo.lock'}!={k:v for k,v in expected.items() if k!='Cargo.lock'}: raise ValueError('prebuild source moved')
        prior=list(run.glob(arm+'-*-admission.json'))
        if prior:
            for p in prior:
                a=json.loads(p.read_text())
                if a['source']!=actual: raise ValueError('previously admitted lock/source moved')
        elif actual!=expected: raise ValueError('initial lock moved')
        compiler=support.toolchain()
        if compiler!=prep['compiler']: raise ValueError('compiler moved after prepare')
        target=dest/('target-'+variant)
        cmd=['cargo','build','--offline','--release','--manifest-path',str(dest/'Cargo.toml'),
             '--target-dir',str(target),'-p','nav','--bin','stage-a-probe']
        if prior: cmd.append('--locked')
        if variant=='counting': cmd+=['--features','stage-counting']
        receipt=support.bounded(cmd,run,arm+'-'+variant+'-build',cwd=dest,**BUILD)
        if receipt['failure']: raise RuntimeError('build failed; receipt preserved')
        binary=target/'release/stage-a-probe'
        save(receipt_path,dict(arm=arm,commit=ARMS[arm],variant=variant,client=support.CLIENT,
             executable_sha256=sha(binary),source=tree(dest),tools=tools(),compiler=compiler,
             lock_sha256=sha(dest/'Cargo.lock'),command=cmd))

def verify_arm(run, arm, variant):
    if arm not in ARMS or variant not in ('clean','counting'): raise ValueError('unsupported arm/variant')
    run=owned(run); a=json.loads((run/f'{arm}-{variant}-admission.json').read_text()); dest=run/arm
    if (a['arm'],a['commit'],a['variant'],a['client'])!=(arm,ARMS[arm],variant,support.CLIENT): raise ValueError('admission identity')
    verify_original(dest,arm)
    if a['tools']!=tools() or a['source']!=tree(dest): raise ValueError('admitted source/tool moved')
    if sha(dest/'Cargo.lock')!=a['lock_sha256']: raise ValueError('admitted lock moved')
    binary=dest/f'target-{variant}/release/stage-a-probe'
    admit(binary,a['executable_sha256'],200*1024**2)
    return binary

def selectors(path):
    path=Path(path); admit(path,sha(path),65536)
    if path.suffix=='.json':
        data=json.loads(path.read_text())
        if not isinstance(data,list) or any(not isinstance(r,list) or any(type(v)!=int for v in r) for r in data): raise ValueError('JSON must be arrays of ten integers')
        text=''.join(' '.join(map(str,r))+'\n' for r in data)
    elif path.suffix=='.tsv': text=path.read_text()
    else: raise ValueError('only JSON arrays or TSV supported')
    support.fixed_selectors(text)
    return [[int(v) for v in row.split()] for row in text.splitlines()]

def fixtures(run):
    dest=run/'fixtures'; dest.mkdir()
    corpus=[]
    rows=[[100,200,0,100,200,0,0,0,0,0], [100,200,0,110,210,0,0,0,0,1],
          [100,200,0,102,202,1,0,0,0,0]]
    rows += [[100,200,0,102,202,level,bits,state,radius,model]
             for level,bits,state in [(1,1,4),(1,1,0),(2,1,5),(2,5,0),(3,0,6),(3,0,0),(3,8,1),(1,15,3)]
             for radius in (0,4) for model in (0,1)]
    (dest/'routes.tsv').write_text(''.join(' '.join(map(str,r))+'\n' for r in rows))
    save(dest/'routes.json',rows)
    for mode in ('all-uniform','all-dense','gated'):
        if mode=='gated':
            # Exact independent wire minima from immutable extension generator.
            pair_wire=lambda v:wire.U(len(v))+b''.join(wire.I(a)+wire.I(b) for a,b in v)
            def gated(kind,at,to,ident,option,ticks,skills,items):
                return bytes([kind])+b''.join(wire.I(v) for v in (*at,*to,ident,option,ticks))+bytes([0])+wire.I(-1)+pair_wire(skills)+pair_wire(items)+wire.U(0)*3
            b=wire.pack(edges=[gated(4,(0,0,0),(102,202,1),0,0,3,[(6,25)],[(554,1),(556,3),(563,1)]),
                gated(4,(0,0,0),(102,202,2),1712,4,2,[],[(1712,1)]),
                gated(3,(100,200,0),(102,202,3),378,1,7,[],[(995,30)])],banks=[wire.bank()])
        else:
            pairs=[(0,False)]*(64*64*4)
            if mode=='all-dense':
                # Each tile differs at its last valid cell; open long-route coverage remains.
                for p in range(4):
                    for z in (31,63):
                        for x in (31,63): pairs[p*4096+z*64+x]=(255,True)
            b=wire.pack(64,64,(100,200,0),pairs)
        path=dest/(mode+'.bin');path.write_bytes(b)
        corpus.append(dict(name=mode,path=str(path.relative_to(run)),sha256=sha(path),bytes=len(b)))
    save(run/'fixtures.json',dict(corpus=corpus,routes_sha256=sha(dest/'routes.tsv'),routes_json_sha256=sha(dest/'routes.json')))

def output(path, diagnostic, rows):
    data=[json.loads(line) for line in path.read_text().splitlines()]
    if len(data)!=len(PHASES)+1 or [r.get('phase') for r in data[:-1]]!=PHASES: raise ValueError('incomplete or unordered phases')
    last=data[-1]
    if last.get('summary') is not True or last.get('diagnostic')!=diagnostic or last['aggregate']['calls']!=rows*8*3: raise ValueError('wrong workload summary')
    for key in ('since_start_ns','process_cpu_ns','process_peak_rss_bytes'):
        values=[r[key] for r in data[:-1]]
        if values!=sorted(values): raise ValueError('nonmonotonic '+key)
    if any(r['current_rss_bytes']<=0 for r in data[:-1]): raise ValueError('missing current RSS')
    if last['startup_peak_rss_bytes']>data[0]['process_peak_rss_bytes']: raise ValueError('startup peak ordering')
    if diagnostic and (last['narrow_allocations'] or last['narrow_requested_bytes']): raise ValueError('warmed collision read allocated')
    return data

def run_one(run, arm, variant, pack, routes, outdir, name, released=False):
    binary=verify_arm(run,arm,variant); outdir=owned(outdir)
    digest=sha(pack); route_hash=sha(routes); rows=selectors(routes)
    admit(pack,digest,128*1024**2 if released else 1024**2)
    with pack.open('rb') as f:
        with mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ) as b: support.safe_wire(b,70_000_000 if released else 65536)
    r=support.bounded([str(binary),str(pack),str(routes),'released' if released else 'generated'],outdir,name,**LIMITS)
    verify_arm(run,arm,variant);admit(pack,digest,128*1024**2);admit(routes,route_hash,65536)
    if r['failure']: raise RuntimeError('probe failed; bounded receipt retained: '+name)
    data=output(outdir/(name+'.out'),variant=='counting',len(rows))
    save(outdir/(name+'.result.json'),dict(admission_sha256=sha(run/f'{arm}-{variant}-admission.json'),input_sha256=digest,routes_sha256=route_hash,
         diagnostic=variant=='counting',receipt=r,data=data))
    return data

def generated(run, variant):
    run=owned(run); corpus=json.loads((run/'fixtures.json').read_text()); results=[]
    dest=run/('qualification-'+variant);dest.mkdir()
    routes=run/'fixtures/routes.tsv';admit(routes,corpus['routes_sha256'],65536)
    for e in corpus['corpus']:
        pack=run/e['path'];admit(pack,e['sha256'],e['bytes'])
        pair=[]
        for arm in ARMS:
            data=run_one(run,arm,variant,pack,routes,dest,arm+'-'+e['name']); pair.append(data[-1])
        for key in ('aggregate','lookup_checksum','narrow_checksum','logical_cells'):
            if pair[0][key]!=pair[1][key]: raise ValueError('generated aggregate mismatch: '+key)
        d=pair[1]['layout']; expected=(16,0) if e['name']=='all-uniform' else (16,16) if e['name']=='all-dense' else (4,0)
        if (d[0],d[2])!=expected or d[1]!=d[0]*8 or d[3]!=d[2]*1152 or d[4:6]!=[0,0] or d[9]!=8: raise ValueError('candidate layout mismatch')
        results.append(dict(fixture=e['name'],summary=pair))
    if not all(results[0]['summary'][0]['aggregate'][k]>0 for k in ('routes','no_path','tiles')): raise ValueError('route coverage absent')
    if not all(results[2]['summary'][0]['aggregate'][k]>0 for k in ('transports','banks')): raise ValueError('gated coverage absent')
    save(dest/'result.json',dict(qualified=True,scope='synthetic tooling qualification only',variant=variant,results=results,
         tools=tools(),platform=platform.platform(),native_hard_as_qualified=False))

def guards(run):
    run=owned(run);run.mkdir(parents=True,exist_ok=False)
    r=support.bounded([sys.executable,str(HERE/'test_stage_a.py'),'-v'],run,'guards',wall=60,cpu=30,rss=512*1024**2,address=4*1024**3,output=1024**2)
    passed=r['failure'] is None and b'skipped' not in (run/'guards.err').read_bytes()
    save(run/'result.json',dict(qualified=passed,native_hard_as_qualified=passed and platform.system()=='Linux',
        tools=tools(),platform=platform.platform(),evidence={n:sha(run/n) for n in ('guards.out','guards.err','guards.receipt.json')}))
    if not passed: raise RuntimeError('guard tests failed; retained evidence')

def paired(baseline, candidate, threshold, relative=False):
    if len(baseline)!=len(candidate) or len(baseline)<3 or any(not math.isfinite(x) or x<0 for x in baseline+candidate): raise ValueError('need >=3 finite nonnegative pairs')
    center=statistics.median(baseline)
    if relative and center<=0: raise ValueError('zero relative baseline')
    scale=center if relative else 1
    deltas=[(c-b)/scale for b,c in zip(baseline,candidate)]
    delta=(statistics.median(candidate)-center)/scale
    noise=(max(baseline)-min(baseline))/scale
    classification='inconclusive' if noise>=threshold or delta-noise<=threshold<delta+noise else 'fail' if delta>threshold else 'pass'
    return dict(n=len(baseline),baseline_median=center,candidate_median=statistics.median(candidate),paired_deltas=deltas,
                median_delta=delta,baseline_repeat_range=noise,threshold=threshold,classification=classification)

def check_release(run, auth, standin=False):
    # Never stat/read an external pack until ALL release prerequisites pass.
    if auth.get('released') is not True or auth.get('mode')!=('standin' if standin else 'real'): raise ValueError('explicit root release required')
    if not standin and platform.system()!='Linux': raise ValueError('native Linux hard-AS qualification required')
    if auth.get('limits')!=LIMITS: raise ValueError('bounds may not be relaxed')
    if auth.get('tools')!=tools(): raise ValueError('root tool binding missing')
    if auth.get('platform')!=platform.platform(): raise ValueError('hardware/platform binding missing')
    if auth.get('hardware')!=hardware(): raise ValueError('hardware identity mismatch')
    if auth.get('order')!=json.loads((HERE/'proposed-manifest.json').read_text())['order']: raise ValueError('explicit root order approval missing')
    if auth.get('manifest_sha256')!=sha(HERE/'proposed-manifest.json'): raise ValueError('manifest binding missing')
    for variant in ('clean','counting'):
        p=run/f'qualification-{variant}/result.json'
        q=json.loads(p.read_text())
        if auth.get(variant+'_qualification_sha256')!=sha(p) or q.get('qualified') is not True or q.get('tools')!=tools() or q.get('variant')!=variant or q.get('platform')!=platform.platform(): raise ValueError('generated qualification missing or stale')
        for arm in ARMS:
            verify_arm(run,arm,variant)
            if auth.get(f'{arm}-{variant}-admission_sha256')!=sha(run/f'{arm}-{variant}-admission.json'): raise ValueError('root arm binding missing')
    if not standin:
        guard=Path(auth['guard_path']);admit(guard,auth['guard_sha256'],1024**2);g=json.loads(guard.read_text())
        if g.get('native_hard_as_qualified') is not True or g.get('tools')!=tools() or g.get('platform')!=platform.platform(): raise ValueError('native guard report unqualified')
        for name in ('guards.out','guards.err','guards.receipt.json'): admit(guard.parent/name,g['evidence'][name],1024**2)
        if json.loads((guard.parent/'guards.receipt.json').read_text()).get('failure') is not None or b'skipped' in (guard.parent/'guards.err').read_bytes(): raise ValueError('failed or skipped native guards')
    if not auth.get('hardware') or auth.get('correctness_review_released') is not True: raise ValueError('root hardware/correctness release missing')

def released_run(run, auth, standin=False):
    run=owned(run);check_release(run,auth,standin)
    pack=Path(auth['input_path']);route=Path(auth['routes_path'])
    if standin and (pack.resolve().parent!=run/'fixtures' or route.resolve().parent!=run/'fixtures'): raise ValueError('stand-in cannot read external input')
    admit(pack,auth['input_sha256'],1024**2 if standin else 128*1024**2)
    if pack.stat().st_size!=auth['input_bytes']: raise ValueError('root input length mismatch')
    admit(route,auth['routes_sha256'],65536);rows=selectors(route)
    dest=run/('standin-release' if standin else 'real-release');dest.mkdir()
    shutil.copyfile(pack,dest/'input.bin');save(dest/'authorization.json',auth)
    (dest/'routes.tsv').write_text(''.join(' '.join(map(str,r))+'\n' for r in rows))
    admit(dest/'input.bin',auth['input_sha256'],128*1024**2)
    schedule=json.loads((HERE/'proposed-manifest.json').read_text())['order']
    results=[]
    save(dest/'launch.json',dict(order=schedule,authorization=auth,normalized_routes_sha256=sha(dest/'routes.tsv'),hardware=hardware()))
    try:
        for i,arm in enumerate(schedule):
            check_release(run,auth,standin)
            results.append(dict(arm=arm,data=run_one(run,arm,'clean',dest/'input.bin',dest/'routes.tsv',dest,f'{i:02d}-{arm}',released=not standin)))
            if results[-1]['data'][-1]['aggregate']!=results[0]['data'][-1]['aggregate']: raise ValueError('paired workload aggregates differ')
            save(dest/'progress.json',dict(completed=len(results),results=results))
    except BaseException as e:
        save(dest/'result.json',dict(qualified=False,failure=repr(e),results=results));raise
    metrics={}
    for name,threshold,relative in [('peak',8*1024**2,False),('cold',.10,True),('route_cpu',.05,True),('route_p99',2_000_000,False)]:
        samples={a:[] for a in ARMS}
        for r in results:
            phases={p['phase']:p for p in r['data'][:-1]};summary=r['data'][-1]
            value={'peak':phases['decoded_converted_retained_input']['process_peak_rss_bytes'],
                   'cold':phases['retained_input']['elapsed_ns']+phases['decoded_converted_retained_input']['elapsed_ns'],
                   'route_cpu':phases['hot_routes']['cpu_ns'],'route_p99':summary['route_p99_ns']}[name]
            samples[r['arm']].append(value)
        metrics[name]=paired(samples['dense'],samples['tiled'],threshold,relative)
    save(dest/'result.json',dict(scope='stand-in qualification only' if standin else 'Stage A only; no resident acceptance',metrics=metrics,results=results))

def main():
    p=argparse.ArgumentParser();p.add_argument('action',choices=['prepare','build','generated','guards','real']);p.add_argument('--run',type=Path,required=True)
    p.add_argument('--variant',choices=['clean','counting'],default='clean');p.add_argument('--authorization',type=Path);p.add_argument('--authorization-sha256')
    a=p.parse_args();run=owned(a.run)
    if a.action=='prepare':prepare(run)
    elif a.action=='build':build(run,a.variant)
    elif a.action=='generated':generated(run,a.variant)
    elif a.action=='guards':guards(run)
    else:
        if not a.authorization or not a.authorization_sha256: p.error('root authorization path AND external hash required')
        admit(a.authorization,a.authorization_sha256,1024**2);released_run(run,json.loads(a.authorization.read_text()))

if __name__=='__main__':main()
