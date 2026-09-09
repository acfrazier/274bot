#!/usr/bin/env python3
"""Versioned singleton-warmed Stage A protocol, separate from legacy real."""
import stage_a as stage
from contextlib import contextmanager
import resource
import signal
import time
import argparse
import hashlib
import json
import math
import mmap
import os
from pathlib import Path
import platform
import shutil
import sys

SCHEMA = 'stage-a-singleton-v1'
RAW_SCHEMA = 'stage-a-raw-v1'
RAW_ORDER = 'sweep-row-lane/8/3'
CEILINGS = {'F1': [480,360,4], 'F2': [1800,1500,118], 'acceptance': [7200,6000,708]}
STORAGE = dict(release_bytes=512*1024**2, free_reserve_bytes=1024**3,
               child_bytes=256*1024, record_bytes=16384, json_bytes=256*1024)
ORIGINAL_TSV = '49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125'
ORIGINAL_PACK = '2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30'
QUAL_LIMITS = dict(wall=360,cpu=300,rss=512*1024**2,address=4*1024**3,output=1024**2)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path,value,cap=256*1024):
    data=(json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n').encode()
    if len(data)>cap:
        raise ValueError('bounded JSON output exceeded')
    # Atomic constant-size checkpoints, not a duplicated growing history.
    tmp=path.with_name(path.name+'.pending')
    with tmp.open('xb') as f:
        f.write(data);f.flush();os.fsync(f.fileno())
    os.replace(tmp,path)


def reference(path):
    return dict(path=str(path.absolute()),sha256=stage.sha(path))


def read_ref(ref,cap=256*1024):
    p=Path(ref['path'])
    stage.admit(p,ref['sha256'],cap)
    return json.loads(p.read_text())


def tool_hashes():
    return dict(stage=stage.tools(),scheduler={n:stage.sha(stage.HERE/n)
        for n in ('sharded.py','test_sharded.py','test_sharded_guards.py','qualify_sharded.py')})


def normalized_rows(path):
    rows=stage.selectors(path)
    if len(rows)!=59:
        raise ValueError('exactly 59 ordinal rows required')
    return [(' '.join(map(str,row))+'\n').encode() for row in rows]


def contract(rows):
    return dict(schema=SCHEMA,raw_schema=RAW_SCHEMA,raw_order=RAW_ORDER,
        tools=tool_hashes(),arms=stage.ARMS,client=stage.support.CLIENT,
        shard_sha256=[digest(r) for r in rows],
        schedules={p:[list(x) for x in schedule(p)] for p in CEILINGS},
        ceilings=CEILINGS,total=[9000,7500],child_limits=stage.LIMITS,
        headroom=[80,60],storage=STORAGE)


def generated_authorization(run,pack,routes,destination):
    # This convenience constructor cannot issue a real authorization or touch
    # anything outside the run's generated fixture directory.
    run=stage.owned(run)
    if pack.resolve().parent!=run/'fixtures' or routes.resolve().parent!=run/'fixtures':
        raise ValueError('stand-in cannot access external input')
    prerequisites=dict(released=True,mode='standin',limits=stage.LIMITS,
        tools=stage.tools(),platform=platform.platform(),hardware=stage.hardware(),
        order=json.loads((stage.HERE/'proposed-manifest.json').read_text())['order'],
        manifest_sha256=stage.sha(stage.HERE/'proposed-manifest.json'),correctness_review_released=True,
        input_path=str(pack),input_sha256=stage.sha(pack),input_bytes=pack.stat().st_size,
        routes_path=str(routes),routes_sha256=stage.sha(routes))
    for variant in ('clean','counting'):
        p=run/f'qualification-{variant}/result.json'
        if p.exists(): prerequisites[variant+'_qualification_sha256']=stage.sha(p)
        for arm in stage.ARMS:
            p=run/f'{arm}-{variant}-admission.json'
            if p.exists(): prerequisites[f'{arm}-{variant}-admission_sha256']=stage.sha(p)
    return dict(schema=SCHEMA,phase='F1',released=True,mode='standin',run=str(run),
        destination=str(stage.owned(destination)),prerequisites=prerequisites,
        contract=contract(normalized_rows(routes)))


def storage_guard(dest,reserve=0):
    size=0
    for p in dest.rglob('*'):
        if p.is_symlink(): raise ValueError('symlink in release artifacts')
        if p.is_file(): size+=p.stat().st_size
    if size+reserve>STORAGE['release_bytes']:
        raise ValueError('whole-release storage bound')
    if shutil.disk_usage(dest).free<STORAGE['free_reserve_bytes']+STORAGE['release_bytes']-size:
        raise ValueError('insufficient available disk for bounded release')
    return size


def native_preflight(root,fresh=False):
    # Root binds fresh host context; repeat live boot, memory and competing-work
    # checks rather than treating an old successful qualification as admission.
    pre=read_ref(root['preflight'])
    if (pre.get('hardware')!=stage.hardware() or pre.get('no_competing_work') is not True
            or pre.get('boot_id')!=Path('/proc/sys/kernel/random/boot_id').read_text().strip()
            or not 0<=time.time()-pre.get('timestamp',0)<=(300 if fresh else 10800)
            or pre.get('idle_percent',0)<90 or pre.get('steal_percent')!=0):
        raise ValueError('root native preflight missing/stale')
    memory={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()}
    if memory['MemAvailable']<512*1024**2 or memory['SwapTotal']!=memory['SwapFree']:
        raise ValueError('native memory/swap admission')
    def counters():return list(map(int,Path('/proc/stat').read_text().splitlines()[0].split()[1:9]))
    before=counters();time.sleep(.05);after=counters()
    elapsed=sum(after)-sum(before);idle=after[3]+after[4]-before[3]-before[4]
    if elapsed<=0 or idle/elapsed<.90 or after[7]!=before[7]:
        raise ValueError('native idle/steal admission')
    ps=stage.subprocess.check_output(['ps','-axo','comm='],text=True)
    conflicts={'stage-a-probe','cargo','rustc','heaptrack','tui-play','panel-play'}
    if any(Path(line.strip()).name in conflicts for line in ps.splitlines()):
        raise ValueError('competing measurement/build process')


def check_scheduler_qualification(root):
    q=read_ref(root['scheduler_qualification'])
    if (q.get('qualified') is not True or q.get('native_hard_as_qualified') is not True
            or q.get('tools')!=tool_hashes() or q.get('platform')!=platform.platform()
            or q.get('hardware')!=stage.hardware() or q.get('python_sha256')!=stage.sha(sys.executable)):
        raise ValueError('native scheduler qualification missing/stale')
    guards=q['guards']
    for suffix,ref in guards.items():
        stage.admit(Path(ref['path']),ref['sha256'],1024**2)
    if set(guards)!={'.out','.err','.receipt.json'}:raise ValueError('incomplete scheduler guard evidence')
    receipt=read_ref(guards['.receipt.json'],1024**2)
    if (receipt.get('failure') is not None or receipt.get('returncode')!=0
            or receipt.get('limits')!=dict(QUAL_LIMITS,file_size=QUAL_LIMITS['output'])
            or receipt.get('command')!=[sys.executable,'-m','unittest','-v','test_sharded','test_sharded_guards']
            or receipt.get('cwd')!=str(stage.HERE)
            or receipt.get('address_guard_active') is not True
            or b'skipped' in Path(guards['.err']['path']).read_bytes()):
        raise ValueError('native scheduler guards failed/skipped')
    smoke=read_ref(q['smoke'])
    if smoke.get('status')!='complete' or smoke.get('phase')!='F1' or smoke.get('completed')!=4:
        raise ValueError('new probe smoke incomplete')
    bind_entries(Path(q['smoke']['path']).parent,smoke['entries'],'F1')
    collect(Path(q['smoke']['path']).parent,smoke['entries'],'F1')


def launch(run,arm,pack,selector,out,name,standin):
    # Reuse original source, decoder-allocation and owned-group guards, but do
    # not duplicate run_one's full data/result history for every child.
    binary=stage.verify_arm(run,arm,'clean')
    with pack.open('rb') as f:
        with mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ) as data:
            stage.support.safe_wire(data,65536 if standin else 70_000_000)
    before=resource.getrusage(resource.RUSAGE_CHILDREN)
    receipt=stage.support.bounded([str(binary),str(pack),str(selector),'generated' if standin else 'released'],out,name,**stage.LIMITS)
    after=resource.getrusage(resource.RUSAGE_CHILDREN)
    child_cpu=after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime
    if receipt['failure'] or receipt['returncode']!=0:
        raise ValueError('child failure; no retry')
    data=stage.output(out/(name+'.out'),False,1)
    raw_samples(data[-1],1)
    return data,receipt,child_cpu


def validate_receipt(receipt,child_cpu):
    if (receipt.get('failure') is not None or receipt.get('returncode')!=0
            or receipt.get('limits')!=dict(stage.LIMITS,file_size=stage.LIMITS['output'])
            or type(receipt.get('wall_seconds')) not in (int,float)
            or not 0<=receipt['wall_seconds']<=80
            or type(child_cpu) not in (int,float) or not 0<=child_cpu<=60):
        raise ValueError('unvalidated/over-headroom child receipt')


def bind_entries(dest,entries,phase):
    expected=schedule(phase)
    for i,entry in enumerate(entries):
        j,row,arm=expected[i]
        if entry['slot']!=[j,row,arm] or entry['name']!=f'{i:03d}-{j}-{row}-{arm}':
            raise ValueError('entry ordinal/path binding')
        if set(entry['files'])!={'.out','.err','.receipt.json'}:
            raise ValueError('missing output bindings')
        for suffix,h in entry['files'].items():
            stage.admit(dest/(entry['name']+suffix),h,STORAGE['child_bytes'])
        record=dest/(entry['name']+'.record.json')
        stage.admit(record,digest((json.dumps(entry,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n').encode()),16384)


def collect(dest,entries,phase):
    expected=schedule(phase)
    if len(entries)!=len(expected):
        raise ValueError('partial schedule cannot aggregate')
    results=[]
    for entry,slot in zip(entries,expected):
        if entry['slot']!=list(slot): raise ValueError('ordinal/pair/arm/order mismatch')
        for suffix,h in entry['files'].items():
            stage.admit(dest/(entry['name']+suffix),h,STORAGE['child_bytes'])
        data=stage.output(dest/(entry['name']+'.out'),False,1)
        validate_data(data)
        r=json.loads((dest/(entry['name']+'.receipt.json')).read_text())
        validate_receipt(r,entry.get('child_cpu'))
        results.append((slot,data))
    aggregates={}
    for (_,row,_),data in results:
        old=aggregates.setdefault(row,data[-1]['aggregate'])
        if old!=data[-1]['aggregate']: raise ValueError('row aggregate mismatch')
    return results


def aggregate(results):
    if [slot for slot,_ in results]!=schedule('acceptance'):
        raise ValueError('six complete replicates required')
    batches={}
    peaks={r:{a:[] for a in stage.ARMS} for r in range(1,60)}
    for (j,row,arm),data in results:
        validate_data(data)
        batch=batches.setdefault((j,arm),dict(raw=[],cpu=0,lane_cpu=[0,0,0]))
        summary=data[-1];phases={v['phase']:v for v in data[:-1]}
        batch['raw'].extend(raw_samples(summary,1));batch['cpu']+=phases['hot_routes']['cpu_ns']
        for lane in range(3):batch['lane_cpu'][lane]+=summary['lane_cpu_ns'][lane]
        peaks[row][arm].append(phases['decoded_converted_retained_input']['process_peak_rss_bytes'])
    for b in batches.values():
        if len(b['raw'])!=1416: raise ValueError('incomplete raw population')
        b['p99']=p99(b['raw']);b['lane_p99']=[p99(b['raw'][lane::3]) for lane in range(3)]
    metrics={name:stage.paired([batches[j,'dense'][key] for j in range(1,7)],
        [batches[j,'tiled'][key] for j in range(1,7)],threshold,relative)
        for name,key,threshold,relative in [('route_cpu','cpu',.05,True),('route_p99','p99',2_000_000,False)]}
    metrics['row_peak']={str(row):stage.paired(v['dense'],v['tiled'],8388608) for row,v in peaks.items()}
    return dict(metrics=metrics,batches=[dict(pair=j,arm=a,**{k:v for k,v in b.items() if k!='raw'}) for (j,a),b in batches.items()])


def row_peak(results,row):
    values={a:[] for a in stage.ARMS}
    for (j,r,a),data in results:
        if r==row:values[a].append((j,data[2]['process_peak_rss_bytes']))
    if any([j for j,_ in v]!=list(range(1,7)) for v in values.values()):
        raise ValueError('six row peak pairs required')
    return stage.paired([v for _,v in values['dense']],[v for _,v in values['tiled']],8388608)


def run_phase(run,authorization,authorization_sha256,standin=False):
    started=(time.monotonic(),total_cpu(),time.process_time())
    auth=read_ref(dict(path=str(authorization.absolute()),sha256=authorization_sha256))
    if auth.get('phase') not in CEILINGS:raise ValueError('unknown phase')
    with deadline(CEILINGS[auth['phase']][0]):
        return _run_phase(run,authorization,authorization_sha256,standin,started)


def _run_phase(run,authorization,authorization_sha256,standin,started):
    start,cpu_start,own_start=started
    run=stage.owned(run);auth_ref=dict(path=str(authorization.absolute()),sha256=authorization_sha256)
    auth=read_ref(auth_ref);phase=auth.get('phase')
    if phase not in CEILINGS or auth.get('schema')!=SCHEMA or auth.get('released') is not True or auth.get('mode')!=('standin' if standin else 'real'):
        raise ValueError('explicit versioned phase authorization required')
    root_ref=auth_ref if phase=='F1' else auth['root']
    root=read_ref(root_ref)
    if root.get('phase')!='F1' or root.get('run')!=str(run) or root.get('mode')!=auth['mode']:
        raise ValueError('original release identity mismatch')
    dest=stage.owned(Path(root['destination']))
    if run not in dest.parents:raise ValueError('release destination outside admitted run')
    if phase=='F1':dest.mkdir(parents=True,exist_ok=False)
    out=dest/phase;out.mkdir(exist_ok=False)
    # Claim BEFORE hashing/staging/source admission. If setup is interrupted its
    # cost is unknown and this phase stays non-restartable, even with zero probes.
    write_json(out/'claim.json',dict(schema=SCHEMA,phase=phase,root=root_ref,
        authorization=auth_ref,status='claimed',interruption_budget_unknown=True),16384)
    claim_ref=reference(out/'claim.json')
    # No external pack read before inherited original admission passes.
    prereq=root['prerequisites'];stage.check_release(run,prereq,standin)
    if not standin:
        check_scheduler_qualification(root);native_preflight(auth,fresh=True)
    pack=Path(prereq['input_path']);routes=Path(prereq['routes_path'])
    if standin and (pack.resolve().parent!=run/'fixtures' or routes.resolve().parent!=run/'fixtures'):
        raise ValueError('stand-in external input forbidden')
    if not standin and (prereq['input_sha256']!=ORIGINAL_PACK or prereq['input_bytes']!=73438581 or prereq['routes_sha256']!=ORIGINAL_TSV):
        raise ValueError('original real pack/full TSV required')
    stage.admit(pack,prereq['input_sha256'],1024**2 if standin else 128*1024**2)
    stage.admit(routes,prereq['routes_sha256'],65536)
    rows=normalized_rows(routes)
    if root.get('contract')!=contract(rows): raise ValueError('root schema/source/tool/schedule/shard contract changed')

    prior=dict(wall=0,cpu=0);ancestors=[]
    if phase!='F1':
        if auth.get('review_approved') is not True: raise ValueError('parent evidence review required')
        if phase=='acceptance' and auth.get('method_accepted') is not True: raise ValueError('root temporal method decision required')
        parent_ref=auth['parent'];parent=read_ref(parent_ref)
        wanted='F1' if phase=='F2' else 'F2'
        if (parent.get('phase')!=wanted or parent.get('status')!='complete'
                or parent.get('root')!=root_ref or parent.get('completed')!=len(schedule(wanted))):
            raise ValueError('incomplete or foreign continuation receipt')
        ancestors=[parent_ref,*parent.get('ancestors',[])]
        if len(ancestors)!=(1 if phase=='F2' else 2): raise ValueError('broken F1/F2 chain')
        prior=parent['budget']
        if (any(type(prior.get(k)) not in (float,int) or not math.isfinite(prior[k])
                    or not 0<=prior[k]<=CEILINGS[wanted][i] for i,k in enumerate(('wall','cpu')))
                or prior.get('stopped') or prior.get('reserved_cpu') or prior.get('reserved_wall')):
            raise ValueError('interrupted/failed budget is not continuable')
        if phase=='acceptance' and (7.5*prior['wall']>7200 or 7.5*prior['cpu']>6000):
            raise ValueError('full feasibility projection exceeds envelope')
    ceiling=CEILINGS[phase]
    # Acceptance has its own phase cap as well as the total campaign cap.
    wall_limit=min(9000,prior['wall']+7200) if phase=='acceptance' else ceiling[0]
    cpu_limit=min(7500,prior['cpu']+6000) if phase=='acceptance' else ceiling[1]
    budget=Budget(wall_limit,cpu_limit,prior)
    budget.start=start;budget.start_cpu=cpu_start;budget.own_start=own_start

    entries=[];chain=[];completed_data=[];checkpoint_ref=None

    def bind():
        read_ref(claim_ref,16384)
        if checkpoint_ref is not None:read_ref(checkpoint_ref,16384)
        if read_ref(auth_ref)!=auth or read_ref(root_ref)!=root:
            raise ValueError('loaded authorization mutated')
        stage.check_release(run,prereq,standin)
        if not standin:check_scheduler_qualification(root)
        if root['contract']!=contract(rows):raise ValueError('tool/config mutation')
        stage.admit(pack,prereq['input_sha256'],prereq['input_bytes'])
        if pack.stat().st_size!=prereq['input_bytes']:raise ValueError('original pack size changed')
        stage.admit(routes,prereq['routes_sha256'],65536)
        stage.admit(dest/'input.bin',prereq['input_sha256'],prereq['input_bytes'])
        if (dest/'input.bin').stat().st_size!=prereq['input_bytes']:raise ValueError('copy size changed')
        for i,h in enumerate(root['contract']['shard_sha256'],1):
            stage.admit(dest/'selectors'/f'{i}.tsv',h,65536)
        for ref in ancestors:
            receipt=read_ref(ref)
            if receipt.get('status')!='complete' or receipt.get('root')!=root_ref:
                raise ValueError('continuation mutation')
            read_ref(receipt['authorization'])
            checkpoint=read_ref(receipt['checkpoint'],16384)
            if (checkpoint.get('status')!='validated' or checkpoint.get('completed')!=len(schedule(receipt['phase']))
                    or any(checkpoint['budget'][k]>receipt['budget'][k] for k in ('wall','cpu'))):
                raise ValueError('continuation checkpoint/budget mismatch')
            bind_entries(Path(ref['path']).parent,receipt['entries'],receipt['phase'])
        bind_entries(out,entries,phase)
        if not standin:native_preflight(auth)
        storage_guard(dest)

    try:
        remaining_wall=wall_limit-budget.snapshot()['wall']
        if remaining_wall<=0:raise ValueError('setup exhausted global wall budget')
        signal.setitimer(signal.ITIMER_REAL,remaining_wall)
        with cpu_deadline(cpu_limit-budget.snapshot()['cpu']):
            if phase=='F1':
                storage_guard(dest)
                shutil.copyfile(pack,dest/'input.bin');(dest/'input.bin').chmod(0o444)
                selectors=dest/'selectors';selectors.mkdir()
                for i,row in enumerate(rows,1):(selectors/f'{i}.tsv').write_bytes(row)
            bind()
            for ref in reversed(ancestors):
                receipt=read_ref(ref)
                chain.extend(collect(Path(ref['path']).parent,receipt['entries'],receipt['phase']))
            # Complete F2 receipt must bind the actual complete original F1.
            if phase=='acceptance' and [slot for slot,_ in chain]!=schedule('F1')+schedule('F2'):
                raise ValueError('partial feasibility cannot release acceptance')
            aggregates={row:data[-1]['aggregate'] for (_,row,_),data in chain}
            for i,slot in enumerate(schedule(phase)):
                j,row,arm=slot;name=f'{i:03d}-{j}-{row}-{arm}'
                bind();storage_guard(dest,1024**2);budget.reserve()
                write_json(out/'checkpoint.json',dict(status='reserved',index=i,slot=slot,budget=budget.snapshot()),16384)
                checkpoint_ref=reference(out/'checkpoint.json')
                # Full child CPU is already reserved. Bound supervisor CPU
                # independently while waiting, leaving two seconds for ps/reap.
                remaining=cpu_limit-budget.snapshot()['cpu']-90-2
                if remaining<=0:raise ValueError('supervisor CPU headroom exhausted')
                signal.setitimer(signal.ITIMER_PROF,remaining)
                data,receipt,child_cpu=launch(run,arm,dest/'input.bin',dest/'selectors'/f'{row}.tsv',out,name,standin)
                remaining=cpu_limit-budget.snapshot()['cpu']
                if remaining<=0:raise ValueError('global CPU exhausted')
                signal.setitimer(signal.ITIMER_PROF,remaining)
                validate_data(data);bind()
                if data[-1].get('input_bytes')!=prereq['input_bytes']:
                    raise ValueError('probe input size mismatch')
                validate_receipt(receipt,child_cpu)
                old=aggregates.setdefault(row,data[-1]['aggregate'])
                if old!=data[-1]['aggregate']:raise ValueError('row aggregate mismatch')
                budget.finish(True)
                entry=dict(slot=list(slot),name=name,child_cpu=child_cpu,
                    files={suffix:stage.sha(out/(name+suffix)) for suffix in ('.out','.err','.receipt.json')})
                write_json(out/(name+'.record.json'),entry,16384)
                entries.append(entry)
                write_json(out/'checkpoint.json',dict(status='validated',completed=len(entries),budget=budget.snapshot()),16384)
                checkpoint_ref=reference(out/'checkpoint.json')
                completed_data.append((slot,data))
                if phase=='acceptance' and j==6 and arm=='dense':
                    metric=row_peak(completed_data,row)
                    if metric['classification']=='fail':
                        write_json(out/'early-gate.json',dict(row=row,metric=metric,scope='generated-only' if standin else 'park/review'))
                        raise ValueError('first evaluable row peak gate failure')
            bind();results=collect(out,entries,phase)
            metrics=aggregate(results) if phase=='acceptance' else {}
            bind()
            used=budget.snapshot()
            # Prepay a bounded final publication tail; no omitted supervisor CPU.
            used['wall']+=1;used['cpu']+=.1
            used['publication_reserved_wall']=used.get('publication_reserved_wall',0)+1
            used['publication_reserved_cpu']=used.get('publication_reserved_cpu',0)+.1
            if used['wall']>wall_limit or used['cpu']>cpu_limit:raise ValueError('finalization budget exhausted')
            result=dict(schema=SCHEMA,phase=phase,status='complete',
                scope='generated stand-in only' if standin else ('all-59 three-lane single-row-warmed Stage A routing comparison' if phase=='acceptance' else 'feasibility only'),
                root=root_ref,authorization=auth_ref,ancestors=ancestors,completed=len(entries),
                budget=used,claim=claim_ref,checkpoint=checkpoint_ref,entries=entries,**metrics)
            final=out/'result.json';write_json(final,result)
            if budget.snapshot()['wall']>used['wall'] or budget.snapshot()['cpu']>used['cpu']:
                raise ValueError('final publication exceeded prepaid tail')
            return final
    except BaseException as e:
        if budget.pending:
            try:budget.finish(False)
            except ValueError:pass
        write_json(out/'failure.json',dict(schema=SCHEMA,phase=phase,status='failed',error=repr(e),
            completed=len(entries),budget=budget.snapshot(),root=root_ref,authorization=auth_ref))
        # A final publication that overran its tail is not a continuation token.
        if (out/'result.json').exists():
            write_json(out/'result.json',dict(status='failed',failure=repr(e)))
        raise


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run',required=True,type=Path)
    parser.add_argument('--authorization',required=True,type=Path)
    parser.add_argument('--authorization-sha256',required=True)
    parser.add_argument('--standin',action='store_true')
    args=parser.parse_args()
    run_phase(args.run,args.authorization,args.authorization_sha256,args.standin)


def total_cpu():
    own=resource.getrusage(resource.RUSAGE_SELF)
    children=resource.getrusage(resource.RUSAGE_CHILDREN)
    return own.ru_utime+own.ru_stime+children.ru_utime+children.ru_stime


class Deadline(BaseException):
    pass


@contextmanager
def deadline(seconds):
    # Main-thread POSIX alarm interrupts hashing, ps and pipe waits too. The
    # immutable runner's BaseException/finally path kills and reaps its group.
    if seconds<=0 or signal.getitimer(signal.ITIMER_REAL)!=(0.0,0.0):
        raise ValueError('deadline unavailable or exhausted')
    def expire(*_):
        raise Deadline('global active wall deadline')
    previous=signal.signal(signal.SIGALRM,expire)
    interrupts={s:signal.signal(s,expire) for s in (signal.SIGTERM,signal.SIGHUP)}
    signal.setitimer(signal.ITIMER_REAL,seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL,0)
        signal.signal(signal.SIGALRM,previous)
        for s,handler in interrupts.items():signal.signal(s,handler)


@contextmanager
def cpu_deadline(seconds):
    if seconds<=0 or signal.getitimer(signal.ITIMER_PROF)!=(0.0,0.0):
        raise ValueError('CPU deadline unavailable/exhausted')
    def expire(*_):raise Deadline('global supervisor CPU deadline')
    previous=signal.signal(signal.SIGPROF,expire)
    signal.setitimer(signal.ITIMER_PROF,seconds)
    try:yield
    finally:
        signal.setitimer(signal.ITIMER_PROF,0)
        signal.signal(signal.SIGPROF,previous)


class Budget:
    def __init__(self, wall, cpu_limit, prior=None, clock=time.monotonic, cpu=total_cpu):
        self.clock=clock;self.cpu_clock=cpu
        self.start=clock();self.start_cpu=cpu()
        self.own_start=time.process_time()
        self.wall_limit=wall;self.cpu_limit=cpu_limit
        self.prior=prior or dict(wall=0,cpu=0)
        self.penalty=0;self.pending=False;self.stopped=False

    def snapshot(self):
        own=time.process_time()-self.own_start
        return dict(wall=self.prior['wall']+self.clock()-self.start,
                    cpu=self.prior['cpu']+self.cpu_clock()-self.start_cpu+self.penalty,
                    supervisor_cpu=self.prior.get('supervisor_cpu',0)+own,
                    waited_children_cpu=self.prior.get('waited_children_cpu',0)+max(0,self.cpu_clock()-self.start_cpu-own),
                    unknown_cpu_charge=self.prior.get('unknown_cpu_charge',0)+self.penalty,
                    publication_reserved_wall=self.prior.get('publication_reserved_wall',0),
                    publication_reserved_cpu=self.prior.get('publication_reserved_cpu',0),
                    reserved_wall=120 if self.pending else 0,
                    reserved_cpu=90 if self.pending else 0, stopped=self.stopped)

    def reserve(self):
        used=self.snapshot()
        if (self.pending or self.stopped or used['wall']+120>self.wall_limit
                or used['cpu']+90>self.cpu_limit):
            raise ValueError('global wall/CPU reservation refused')
        self.pending=True

    def finish(self, validated):
        if not self.pending:
            raise ValueError('no reservation')
        if not validated:
            # Unknown final child CPU never refunds; charge the full allowance
            # in addition to measured supervisor/waited CPU (conservative).
            self.penalty+=90;self.stopped=True
        self.pending=False
        used=self.snapshot()
        if used['wall']>self.wall_limit or used['cpu']>self.cpu_limit:
            self.stopped=True
            raise ValueError('global budget exhausted')


def schedule(phase):
    if phase not in ('F1','F2','acceptance'):
        raise ValueError('unknown phase')
    if phase == 'acceptance':
        return [(j,r,a) for j in range(1,7) for r in range(1,60)
                for a in (('dense','tiled') if j%2 else ('tiled','dense'))]
    return [(1,r,a) for r in (range(1,3) if phase=='F1' else range(3,60))
            for a in (('dense','tiled') if r%2 else ('tiled','dense'))]


def raw_samples(summary, rows):
    values=summary.get('raw_elapsed_ns',[])
    if (summary.get('raw_schema')!=RAW_SCHEMA or summary.get('raw_order')!=RAW_ORDER
            or len(values)!=rows*24 or summary.get('aggregate',{}).get('calls')!=rows*24
            or summary.get('route_p99_ns')!=p99(values)):
        raise ValueError('raw schema/order/cardinality/quantile mismatch')
    lane=summary.get('lane_cpu_ns',[])
    if len(lane)!=3 or any(type(x) is not int or x<0 for x in lane):
        raise ValueError('lane CPU mismatch')
    return values


def validate_data(data):
    if len(data)!=9 or [v.get('phase') for v in data[:-1]]!=stage.PHASES:
        raise ValueError('missing/unordered phases')
    for value in data[:-1]:
        for key in ('elapsed_ns','cpu_ns','since_start_ns','process_cpu_ns','current_rss_bytes','process_peak_rss_bytes'):
            if type(value.get(key)) is not int or value[key]<0:
                raise ValueError('invalid phase metric')
    if data[-1].get('summary') is not True or data[-1].get('diagnostic') is not False:
        raise ValueError('clean summary required')
    for value in data[-1].get('aggregate',{}).values():
        if type(value) not in (int,float) or not math.isfinite(value) or value<0:
            raise ValueError('invalid aggregate')
    raw_samples(data[-1],1)


def p99(values):
    if not values or any(type(x) is not int or x < 0 for x in values):
        raise ValueError('nonempty nonnegative integer durations required')
    return sorted(values)[(len(values)*99 + 99)//100 - 1]


if __name__=='__main__':main()
