#!/usr/bin/env python3
"""Frozen, independent dense/tiled correctness probes. No default input path."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import selectors
import signal
import stat
import struct
import subprocess
import sys
import time
import shutil
import mmap

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BASE = '29b7aea779322c8611f83dc193e939ca7d756f75'
CANDIDATE = '8385babb23fd15b876506d4a3f6154984a6b2df1'
CLIENT = '3456edc8dabf7b25ada78110ffa56327af9f67a4'
GENERATED = dict(wall=240, cpu=180, rss=1024*1024*1024, address=64*1024**3, output=512*1024**2)
BUILD = dict(wall=600, cpu=480, rss=6*1024**3, address=64*1024**3, output=8*1024**2, file_size=1024**3)
REAL = dict(wall=900, cpu=800, rss=1024**3, address=4*1024**3, output=4*1024**3)

def sha(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for chunk in iter(lambda: f.read(1024*1024), b''): h.update(chunk)
    return h.hexdigest()

def save(p, value):
    p.write_text(json.dumps(value, indent=2, sort_keys=True)+'\n')

def admit(p, expected, cap):
    st = p.lstat()
    if not stat.S_ISREG(st.st_mode) or st.st_size > cap:
        raise ValueError('not a bounded regular non-symlink file: '+str(p))
    if len(expected) != 64 or sha(p) != expected:
        raise ValueError('sha256 mismatch: '+str(p))

def safe_header(b, max_cells):
    if len(b) < 25: return
    w, h = struct.unpack_from('<II', b, 17)
    # Invalid dimensions are rejected by BOTH original parsers before allocation.
    if 0 < w <= 16384 and 0 < h <= 16384 and w*h*4 > max_cells:
        raise ValueError('valid but unsafe decoder dimensions')

def safe_wire(b, max_cells):
    """Allocation admission only, NOT a second decode/error oracle.

    Stop at truncated/invalid syntax; let the original parser report errors.
    Reject valid grid allocation and length-prefixed string allocation bombs.
    Counts in original parsers cap preallocation to available bytes.
    """
    safe_header(b, max_cells)
    if len(b)<25 or b[:5]!=b'274V\x08': return
    w,h=struct.unpack_from('<II',b,17)
    if not (0<w<=16384 and 0<h<=16384): return
    pos=25+w*h*4+((w*h*4+63)//64)*8
    class End(Exception): pass
    def take(n):
        nonlocal pos
        if pos+n>len(b): raise End()
        old=pos;pos+=n;return old
    def uint(): return struct.unpack_from('<I',b,take(4))[0]
    def byte(): return b[take(1)]
    def string():
        n=uint()
        if n>1024**2: raise ValueError('unsafe declared string allocation')
        take(n)
    try:
        for _ in range(uint()):
            if byte()>8: return
            take(36)
            if byte()>4: return
            take(4)
            for _ in range(2): take(uint()*8)
            for _ in range(uint()): string()
            take(uint()*8);take(uint()*4)
        for _ in range(uint()):
            string();take(12);tag=byte()
            if tag==0: take(4)
            elif tag==1:
                string();take(4)
                if byte()!=0: string()
            else: return
    except End: pass

def bounded(cmd, outdir, name, wall=10, cpu=10, rss=1024**3,
            address=64*1024**3, output=1024**2, cwd=None, file_size=None):
    """New owned process group, rlimits plus sampled group RSS and pipe budget.

    Darwin RLIMIT_AS is only best-effort; real mode is refused there. RSS polling
    is not a hard instantaneous peak cap. CPU/AS are per process, RSS is group.
    """
    def limits():
        resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu))
        # Darwin rejects this RLIMIT_AS request and does not supply a reliable
        # hard address-space guard. Generated fixtures are dimension-bounded;
        # real mode fails closed on Darwin, never claims this guard active.
        if platform.system() == 'Linux':
            resource.setrlimit(resource.RLIMIT_AS, (address, address))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        resource.setrlimit(resource.RLIMIT_FSIZE, (file_size or output, file_size or output))
    start = time.monotonic(); failure = None; size = 0; peak = 0
    env = {k: v for k, v in os.environ.items() if not k.startswith(('CARGO_', 'RUST', 'NAV_', 'BOT_'))}
    env.update(CARGO_BUILD_JOBS='2', CARGO_INCREMENTAL='0', BOT_DEBUG='0')
    # Reserve evidence paths BEFORE starting any child. No orphan on EEXIST.
    for suffix in ('.out','.err','.receipt.json'):
        if (outdir/(name+suffix)).exists(): raise FileExistsError(name+suffix)
    files = [open(outdir/(name+'.out'), 'xb'), open(outdir/(name+'.err'), 'xb')]
    try:
        p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, start_new_session=True, preexec_fn=limits)
    except BaseException as e:
        for f in files: f.close()
        save(outdir/(name+'.receipt.json'),dict(command=[str(x) for x in cmd],failure='launch',error=repr(e)))
        raise
    assert p.stdout is not None and p.stderr is not None
    sel = selectors.DefaultSelector()
    for pipe, f in zip((p.stdout, p.stderr), files):
        os.set_blocking(pipe.fileno(), False); sel.register(pipe, selectors.EVENT_READ, f)
    try:
        cleaned = False
        while sel.get_map() or p.poll() is None:
            # EOF and leader exit are independent. Reap a genuinely completed
            # leader, then kill owned descendants even if they hold pipes open.
            if p.poll() is not None and not cleaned:
                cleanup_group(p.pid)
                cleaned = True
            if time.monotonic()-start > wall: failure = 'wall'; break
            ps = subprocess.run(['ps', '-axo', 'pgid=,rss='], capture_output=True, text=True, timeout=2, check=True)
            group_rss = sum(int(v[1])*1024 for line in ps.stdout.splitlines()
                            if len(v := line.split()) == 2 and int(v[0]) == p.pid)
            peak = max(peak, group_rss)
            if group_rss > rss: failure = 'rss'; break
            for key, _ in sel.select(.02):
                b = os.read(key.fileobj.fileno(), 65536)
                if not b: sel.unregister(key.fileobj); continue
                remaining = output-size
                key.data.write(b[:remaining]); size += min(len(b), remaining)
                if len(b) > remaining: failure = 'output'; break
            if failure: break
    except BaseException:
        failure = 'supervisor'; raise
    finally:
        p.poll()
        try:
            cleanup_group(p.pid)
        finally:
            rc = p.wait(timeout=5)
            for f in files: f.close()
            for pipe in (p.stdout, p.stderr): pipe.close()
            sel.close()
        if failure is None and rc != 0: failure = 'exit'
        receipt = dict(command=[str(x) for x in cmd], cwd=str(cwd or Path.cwd()),
                       wall_seconds=time.monotonic()-start, returncode=rc, failure=failure,
                       sampled_group_peak_bytes=peak, output_bytes=size,
                       address_guard_active=platform.system() == 'Linux',
                       limits=dict(wall=wall,cpu=cpu,rss=rss,address=address,output=output,file_size=file_size or output))
        save(outdir/(name+'.receipt.json'), receipt)
    return receipt

def cleanup_group(pgid):
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Darwin can return EPERM for a just-reaped empty group. Do not
        # suppress a real permission failure while owned processes remain.
        rows = subprocess.check_output(['ps','-axo','pgid=,stat='], text=True)
        if any(int(v[0]) == pgid and not v[1].startswith('Z')
               for row in rows.splitlines() if len(v := row.split()) == 2):
            raise

def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args])

def materialize(run):
    manifests = {}
    for arm, commit in [('dense', BASE), ('tiled', CANDIDATE)]:
        dest = run/arm; dest.mkdir()
        entries = []
        for repo, rev, prefixes, prefix in [
            (ROOT, commit, ('crates/nav/', 'crates/api/'), ''),
            (ROOT/'vendor/fr-client-rust', CLIENT, ('crates/client/',), 'vendor/fr-client-rust/')]:
            for line in git(repo, 'ls-tree', '-r', rev).decode().splitlines():
                meta, name = line.split('\t'); mode, kind, oid = meta.split()
                if not any(name.startswith(p) for p in prefixes): continue
                # Complete crate inputs (including include_bytes assets), no archives.
                if kind != 'blob' or mode != '100644': raise ValueError('unexpected source type '+name)
                b = git(repo, 'cat-file', 'blob', oid)
                if hashlib.sha1(b'blob '+str(len(b)).encode()+b'\0'+b).hexdigest() != oid:
                    raise ValueError('git object hash mismatch')
                p = dest/(prefix+name); p.parent.mkdir(parents=True, exist_ok=True); p.write_bytes(b)
                entries.append(dict(path=prefix+name, git_blob=oid, sha256=sha(p), size=len(b)))
        # Bind original manifests too; standalone workspace changes only package membership.
        for name in ('Cargo.toml', 'Cargo.lock'):
            (dest/('original-'+name)).write_bytes(git(ROOT,'show',commit+':'+name))
        (dest/'Cargo.toml').write_text('[workspace]\nresolver="2"\nmembers=["crates/nav","crates/api"]\n[workspace.dependencies]\nclient={path="vendor/fr-client-rust/crates/client"}\n')
        (dest/'Cargo.lock').write_bytes((dest/'original-Cargo.lock').read_bytes())
        nav = dest/'crates/nav'
        with (nav/'Cargo.toml').open('a') as f:
            f.write('\n[[bin]]\nname="differential-probe"\npath="src/differential.rs"\n')
        # Original lib module declarations as binary root: router/decoder untouched.
        source = (nav/'src/lib.rs').read_text()
        (nav/'src/differential.rs').write_text(source+'\nmod probe;\nfn main() { probe::run(); }\n')
        adapter = (("(c.origin, c.width, c.height, c.walk.len())", "Some((c.walk[i], c.blocked[i/64] & (1 << (i%64)) != 0))", "c.blocked.clone()") if arm=='dense' else
                   ("(c.origin(), c.width(), c.height(), c.logical_cell_count())", "c.packed_pair_at(i)", "c.packed_blocked_words().collect::<Vec<_>>()"))
        probe = (HERE/'probe.rs').read_text()
        for tag, text in zip(('GEOMETRY','PAIR','WORDS'), adapter): probe = probe.replace('unimplemented!("'+tag+'")', text)
        (nav/'src/probe.rs').write_text(probe)
        # Access bridge only: unchanged original router bytes remain a prefix.
        # Expose the existing bounded implementation, never reimplement it.
        with (nav/'src/router.rs').open('a') as f:
            f.write('\n#[path="router-access.rs"]\npub mod differential_access;\n')
        (nav/'src/router-access.rs').write_text('pub fn find(c: &super::WorldCollision, g: &super::TransportGraph, from: super::WorldTile, to: super::WorldTile, model: super::CostModel, budget: usize, opts: super::FindOptions, state: &super::WorldState) -> Result<super::Route, super::RouteError> { super::find_bounded_impl(c,g,from,to,model,budget,opts.allow_teleports,opts.allow_wilderness,state,opts.essence.as_ref(),false) }\n')
        # Bounded host routing helpers, exact source spans (not rewritten
        # approximations). No host crate, live client, accounts or assets.
        host=git(ROOT,'show',commit+':crates/host-play/src/lib.rs').decode()
        spans=[]; extracts=[]
        for marker in ('pub struct PendingBankFetch {','enum RouteOutcome {','fn route_or_bank_fetch(',
                       'fn approach_tiles(', 'struct ScriptRouteRequest {','impl ScriptRouteRequest {'):
            start=host.index(marker); end=host.index('\n}',start)+2
            text=host[start:end]; extracts.append(text)
            spans.append(dict(marker=marker,start_byte=len(host[:start].encode()),end_byte=len(host[:end].encode()),sha256=hashlib.sha256(text.encode()).hexdigest()))
        (nav/'src/host-probe.rs').write_text('\n\n'.join(extracts)+'\n')
        (dest/'original-host-lib.rs').write_text(host)
        save(dest/'host-spans.json',dict(commit=commit,source_sha256=hashlib.sha256(host.encode()).hexdigest(),spans=spans))
        save(dest/'original-source-manifest.json', dict(commit=commit,client=CLIENT,files=entries))
        manifests[arm] = dict(commit=commit, client=CLIENT, original_files=entries)
    save(run/'sources.json',manifests)

def fingerprint_tree(path):
    return {str(p.relative_to(path)): sha(p) for p in sorted(path.rglob('*'))
            if p.is_file() and 'target' not in p.relative_to(path).parts}

def verify_original_sources(dest):
    manifest=json.loads((dest/'original-source-manifest.json').read_text())
    suffixes={'crates/nav/Cargo.toml':b'\n[[bin]]\nname="differential-probe"\npath="src/differential.rs"\n',
              'crates/nav/src/router.rs':b'\n#[path="router-access.rs"]\npub mod differential_access;\n'}
    for entry in manifest['files']:
        b=(dest/entry['path']).read_bytes();suffix=suffixes.get(entry['path'],b'')
        if len(b)!=entry['size']+len(suffix) or (suffix and not b.endswith(suffix)):
            raise ValueError('unexpected original-source modification: '+entry['path'])
        original=b[:entry['size']]
        if hashlib.sha256(original).hexdigest()!=entry['sha256']: raise ValueError('original source changed: '+entry['path'])
        if hashlib.sha1(b'blob '+str(len(original)).encode()+b'\0'+original).hexdigest()!=entry['git_blob']: raise ValueError('original Git object mismatch')
    spans=json.loads((dest/'host-spans.json').read_text());host=(dest/'original-host-lib.rs').read_bytes()
    if hashlib.sha256(host).hexdigest()!=spans['source_sha256']: raise ValueError('host source changed')
    extracts=[]
    for s in spans['spans']:
        b=host[s['start_byte']:s['end_byte']]
        if hashlib.sha256(b).hexdigest()!=s['sha256']: raise ValueError('host span changed')
        extracts.append(b)
    if (dest/'crates/nav/src/host-probe.rs').read_bytes()!=b'\n\n'.join(extracts)+b'\n': raise ValueError('host helpers not exact frozen spans')

def toolchain():
    paths={name:subprocess.check_output(['rustup','which',name],text=True).strip() for name in ('rustc','cargo')}
    return dict(binaries={n:dict(path=p,sha256=sha(p)) for n,p in paths.items()},
                python_sha256=sha(sys.executable),rustc=subprocess.check_output(['rustc','-vV'],text=True),
                cargo=subprocess.check_output(['cargo','-V'],text=True),platform=platform.platform())

def build(run):
    for arm in ('dense','tiled'):
        dest=run/arm
        verify_original_sources(dest)
        compiler=toolchain()
        # Independent targets/locks; offline resolution may prune unrelated workspace packages.
        cmd=['cargo','build','--offline','--manifest-path',str(dest/'Cargo.toml'),'-p','nav','--bin','differential-probe']
        name=arm+'-build-'+str(len(list(run.glob(arm+'-build*.receipt.json'))))
        r=bounded(cmd,run,name,cwd=dest,**BUILD)
        if r['failure']: raise RuntimeError('build failed; retained '+arm+'-build receipt')
        binary=dest/'target/debug/differential-probe'
        save(run/(arm+'-admission.json'),dict(executable_sha256=sha(binary),source=fingerprint_tree(dest),toolchain=compiler))

def verify_arm(run, arm):
    a=json.loads((run/(arm+'-admission.json')).read_text()); d=run/arm
    verify_original_sources(d)
    if fingerprint_tree(d)!=a['source']: raise ValueError('frozen source moved: '+arm)
    admit(d/'target/debug/differential-probe',a['executable_sha256'],200*1024**2)

def compare_files(a,b):
    offset=0
    with a.open('rb') as x,b.open('rb') as y:
        while True:
            p=x.read(1024**2); q=y.read(1024**2)
            if p!=q:
                i=next((i for i,(v,w) in enumerate(zip(p,q)) if v!=w), min(len(p),len(q)))
                return dict(equal=False,first_difference=offset+i)
            if not p: return dict(equal=True,bytes=offset)
            offset+=len(p)

def output_inventory(path,expected):
    counts={};last=None
    with path.open('rb') as f:
        while True:
            line=f.readline(256)
            if not line: break
            tag,n=line.decode().rstrip('\n').split(' ');n=int(n)
            if not 0<=n<=REAL['output']: raise ValueError('invalid output frame size')
            counts[tag]=counts.get(tag,0)+1
            if tag=='complete-input-count': last=int(f.read(n))
            else: f.seek(n,1)
            if f.read(1)!=b'\n': raise ValueError('truncated output frame')
    if last!=expected or counts.get('complete-input-count')!=1: raise ValueError('incomplete result stream')
    return counts

def qualify(run, real=None):
    corpus=json.loads((run/'corpus.json').read_text())
    frozen=sha(run/'corpus.json')
    if len(corpus)>15000 or sum(e['bytes'] for e in corpus)>64*1024**2:
        raise ValueError('generated corpus cap')
    fixed_hash=sha(run/'fixed.tsv');fixed_selectors((run/'fixed.tsv').read_text())
    protocol=run/'protocol-fixture';protocol.mkdir(exist_ok=False)
    shutil.copyfile(run/'inputs/routes-open.bin',protocol/'input.bin')
    shutil.copyfile(run/'fixed.tsv',protocol/'routes.tsv')
    protocol_hash=sha(protocol/'input.bin')
    for e in corpus:
        p=run/e['path']; admit(p,e['sha256'],e['bytes'])
        if e['kind'] not in ('pack','corner','sidecar') or e['routes'] not in ('none','all'):
            raise ValueError('invalid selector schema')
        if p.resolve().parent != (run/'inputs').resolve(): raise ValueError('input escaped corpus')
        if e['bytes']>1024**2: raise ValueError('generated input size cap')
        safe_wire(p.read_bytes(),65536)
    for arm in ('dense','tiled'): verify_arm(run,arm)
    results=[]
    # Freeze the ordered selector list BEFORE either executable is started.
    selectors_file=run/'selectors.tsv'
    selectors_file.write_text(''.join(f"{e['path']}\t{e['kind']}\t{e['routes']}\n" for e in corpus))
    selector_hash=sha(selectors_file)
    save(run/'launch.json',dict(corpus_sha256=frozen,selectors_sha256=selector_hash,
         fixed_sha256=fixed_hash,
         protocol_fixture_sha256=protocol_hash,
         mode='real' if real else 'generated',limits=REAL if real else GENERATED,
         tools={n:sha(HERE/n) for n in ('harness.py','probe.rs','generate.py')},
         compiler=subprocess.check_output(['rustc','-vV'],text=True),
         cargo=subprocess.check_output(['cargo','-V'],text=True),python=sys.version,platform=platform.platform()))
    for arm in ('dense','tiled'):
        verify_arm(run,arm)
        if sha(selectors_file)!=selector_hash or sha(run/'corpus.json')!=frozen: raise ValueError('selector movement')
        if sha(run/'fixed.tsv')!=fixed_hash: raise ValueError('fixed selector movement')
        for e in corpus: admit(run/e['path'],e['sha256'],e['bytes'])
        r=bounded([str(run/arm/'target/debug/differential-probe'),str(selectors_file),str(run), 'real' if real else 'generated'],run,arm+'-probe',**(REAL if real else GENERATED))
        results.append(r)
        verify_arm(run,arm)
        if r['failure'] is None: output_inventory(run/(arm+'-probe.out'),len(corpus))
    proto=[]
    for arm in ('dense','tiled'):
        verify_arm(run,arm);admit(protocol/'input.bin',protocol_hash,1024**2);admit(protocol/'routes.tsv',fixed_hash,65536)
        r=bounded([str(run/arm/'target/debug/differential-probe'),str(protocol/'routes.tsv'),str(protocol),'real'],protocol,arm,**GENERATED)
        # This is the real protocol exercised with GENERATED BYTES ONLY. It
        # is not real-input authorization or native guard qualification.
        proto.append(r)
        verify_arm(run,arm)
        if r['failure'] is None: output_inventory(protocol/(arm+'.out'),1)
    comparison=compare_files(run/'dense-probe.out',run/'tiled-probe.out')
    pc=compare_files(protocol/'dense.out',protocol/'tiled.out')
    qualified=all(not r['failure'] for r in results+proto) and comparison['equal'] and pc['equal']
    save(run/'result.json',dict(arms=results,comparison=comparison,generated_protocol_arms=proto,generated_protocol_comparison=pc,qualified=qualified,input_count=len(corpus),
         output_inventory=output_inventory(run/'dense-probe.out',len(corpus)) if not results[0]['failure'] else None,
         output_hashes={str(p.relative_to(run)):sha(p) for p in [run/'dense-probe.out',run/'tiled-probe.out',protocol/'dense.out',protocol/'tiled.out']}))
    if not qualified: raise RuntimeError('qualification failed; full retained outputs/result.json')

def fixed_selectors(text):
    rows=text.splitlines()
    if not 1<=len(rows)<=256: raise ValueError('fixed route count must be 1..256')
    for row in rows:
        v=[int(x) for x in row.split()]
        if len(v)!=10: raise ValueError('expected 10 integers per fixed route')
        if any(not -16384<=v[i]<=32767 for i in (0,1,3,4)) or any(not 0<=v[i]<=3 for i in (2,5)):
            raise ValueError('unsafe route coordinates')
        if not (0<=v[6]<=15 and 0<=v[7]<=3 and 0<=v[8]<=4 and 0<=v[9]<=1):
            raise ValueError('unsafe route options/radius/model')
    return len(rows)

def qualify_guards(run):
    run.mkdir(parents=True,exist_ok=False)
    receipt=bounded([sys.executable,'-m','unittest','discover','-s',str(HERE),'-p','test_guard.py','-v'],run,'guard-tests',wall=60,cpu=30,file_size=8*1024**2)
    save(run/'guards.json',dict(platform=platform.platform(),system=platform.system(),
         harness_sha256=sha(HERE/'harness.py'),test_sha256=sha(HERE/'test_guard.py'),receipt_sha256=sha(run/'guard-tests.receipt.json'),
         stdout_sha256=sha(run/'guard-tests.out'),stderr_sha256=sha(run/'guard-tests.err'),
         generated_guards_passed=receipt['failure'] is None,
         native_real_qualified=platform.system()=='Linux' and receipt['failure'] is None and b'skipped' not in (run/'guard-tests.err').read_bytes()))
    if receipt['failure']: raise RuntimeError('guard qualification failed')

def check_real_release(auth,run):
    # Validate release and platform before even stat/open of any real input.
    if platform.system()!='Linux': raise ValueError('real mode requires Linux hard-AS qualification; unavailable on Darwin')
    if auth.get('mode')!='real' or auth.get('released') is not True: raise ValueError('root release absent')
    for name in ('launch.json','result.json','sources.json'):
        if auth.get(name+'_sha256')!=sha(run/name): raise ValueError('generated evidence not admitted: '+name)
    if not json.loads((run/'result.json').read_text())['qualified']: raise ValueError('generated comparison failed')
    guard=Path(auth['guard_report']);admit(guard,auth['guard_report_sha256'],1024**2)
    g=json.loads(guard.read_text())
    if not (g.get('native_real_qualified') is True and g.get('platform')==platform.platform()
            and g.get('harness_sha256')==sha(HERE/'harness.py') and g.get('test_sha256')==sha(HERE/'test_guard.py')):
        raise ValueError('native guards unqualified or moved')
    for name,key in [('guard-tests.receipt.json','receipt_sha256'),('guard-tests.out','stdout_sha256'),('guard-tests.err','stderr_sha256')]:
        admit(guard.parent/name,g[key],1024**2)
    for arm in ('dense','tiled'):
        verify_arm(run,arm)
        if auth.get(arm+'_admission_sha256')!=sha(run/(arm+'-admission.json')): raise ValueError('source/executable release mismatch')
    if auth.get('limits')!=REAL: raise ValueError('real limits must be explicitly admitted unchanged')

def real_run(run,authorization):
    admit(authorization,sha(authorization),1024**2)
    auth=json.loads(authorization.read_text());check_real_release(auth,run)
    src=Path(auth['input_path']);route_path=Path(auth['routes_path'])
    admit(route_path,auth['routes_sha256'],65536);fixed_selectors(route_path.read_text())
    admit(src,auth['input_sha256'],128*1024**2)
    if src.stat().st_size!=auth['input_bytes']: raise ValueError('input length not admitted')
    # ONE owned immutable snapshot; never run against a moving external pack.
    dest=run/'real-release';dest.mkdir(exist_ok=False)
    shutil.copyfile(src,dest/'input.bin');admit(dest/'input.bin',auth['input_sha256'],128*1024**2)
    shutil.copyfile(route_path,dest/'routes.tsv');save(dest/'authorization.json',auth)
    with (dest/'input.bin').open('rb') as f:
        with mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ) as b:
            if b[:5]!=b'274V\x08': raise ValueError('real release only v8 packs')
            safe_wire(b,70_000_000)
    results=[]
    for arm in ('dense','tiled'):
        check_real_release(auth,run)
        admit(dest/'input.bin',auth['input_sha256'],128*1024**2)
        admit(dest/'routes.tsv',auth['routes_sha256'],65536)
        r=bounded([str(run/arm/'target/debug/differential-probe'),str(dest/'routes.tsv'),str(dest),'real'],dest,arm,**REAL)
        results.append(r);verify_arm(run,arm)
        admit(dest/'input.bin',auth['input_sha256'],128*1024**2);admit(dest/'routes.tsv',auth['routes_sha256'],65536)
        if r['failure'] is None: output_inventory(dest/(arm+'.out'),1)
    comparison=compare_files(dest/'dense.out',dest/'tiled.out')
    save(dest/'result.json',dict(mode='real',comparison=comparison,arms=results,qualified=all(not r['failure'] for r in results) and comparison['equal']))
    if any(r['failure'] for r in results) or not comparison['equal']: raise RuntimeError('real comparison failed; retained evidence')

def main():
    p=argparse.ArgumentParser(); p.add_argument('action',choices=['prepare','build','generated','guards','real']); p.add_argument('--run',type=Path,required=True)
    p.add_argument('--authorization',type=Path)
    a=p.parse_args(); run=a.run.resolve()
    if HERE not in run.parents: p.error('run must be owned by this tool directory')
    if a.action=='prepare':
        run.mkdir(parents=True,exist_ok=False)
        tools=run/'tools';tools.mkdir()
        for name in ('harness.py','generate.py','probe.rs','test_guard.py'):
            (tools/name).write_bytes((HERE/name).read_bytes())
        materialize(run)
        import generate
        generate.generate(run)
    elif a.action=='build': build(run)
    elif a.action=='generated': qualify(run)
    elif a.action=='guards': qualify_guards(run)
    else:
        if not a.authorization: p.error('real mode requires root-issued authorization JSON')
        real_run(run,a.authorization)

if __name__=='__main__': main()
