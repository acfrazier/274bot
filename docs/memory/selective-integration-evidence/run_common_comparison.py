#!/usr/bin/env python3
"""Thin launcher for the shared existing-API test; reuses campaign libproc sampler."""
import hashlib, json, os, pathlib, subprocess, sys, threading, time
ROOT=pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'docs/memory'))
from native_process_sample import sample_process
label,n=sys.argv[1],int(sys.argv[2])
assert label in ('baseline','selected') and n in (1,32)
evidence=pathlib.Path(__file__).resolve().parent
provenance=json.loads((evidence/f'compare-{label}-provenance.json').read_text())
binary=pathlib.Path(provenance['executable'])
assert hashlib.sha256(binary.read_bytes()).hexdigest()==provenance['executable_sha256']
run=evidence/f'compare-{label}-n{n}'
run.mkdir()
env=dict(os.environ)
for key in list(env):
    if key.startswith(('BOT_MEMORY_','BOT_SCHEDULING_','BOT_RENDER_','BOT_RESPONSIVENESS_')) or key in ('BUDGET_S','BOT_CPU','BOT_DEBUG','BOT_LIVE'):
        env.pop(key)
env.update(LIVE='1',BOT_TARGET='local',COMPARE_N=str(n),COMPARE_WORKLOAD='thiever',COMPARE_RUN_DIR=str(run),HOME=str(run/'home'),COMPARE_CACHE='/Users/acfrazier/experiments/Server/engine/data/pack/client',ENGINE_DIR='/Users/acfrazier/experiments/Server/engine',RS2B0T='/Users/acfrazier/experiments/rs2b0t',NAV_PACK=str(evidence/'274bot.navpack'),NAV_FLAGS=str(evidence/'274bot.navflags'))
argv=provenance['run_argv']
meta={'build':provenance,'argv':argv,'env':{k:env[k] for k in ('LIVE','BOT_TARGET','COMPARE_N','COMPARE_WORKLOAD','COMPARE_RUN_DIR','HOME','COMPARE_CACHE','ENGINE_DIR','RS2B0T','NAV_PACK','NAV_FLAGS')},'started_unix':time.time(),'sampler':'existing native_process_sample.py::sample_process','test_only_driver':True,'frontend':'headless common Play test; no UI','duration_s':60}
for name in ('274bot.navpack','274bot.navflags'):
    meta[name+'_sha256']=hashlib.sha256((evidence/name).read_bytes()).hexdigest()
for name,path in (('server','/Users/acfrazier/experiments/Server'),('catalog',env['RS2B0T'])):
    meta[name+'_commit']=subprocess.check_output(['git','-C',path,'rev-parse','HEAD'],text=True).strip()
    meta[name+'_diff_sha256']=hashlib.sha256(subprocess.check_output(['git','-C',path,'diff','HEAD'])).hexdigest()
log=(run/'run.log').open('w')
samples=(run/'process.jsonl').open('w')
markers=(run/'markers.jsonl').open('w')
lock=threading.Lock(); identity=None
p=subprocess.Popen(argv,cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,bufsize=1)
meta['pid']=p.pid
(run/'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
def take(tag):
    global identity
    with lock:
        before=time.monotonic()
        row={'tag':tag,'pid':p.pid,'monotonic_s':before,'unix_s':time.time()}
        try:
            row['sample']=sample_process(p.pid)
            ident=row['sample']['start_identity']
            if identity is None: identity=ident
            if ident!=identity: raise RuntimeError('process identity changed')
        except Exception as error: row['error']=str(error)
        row['finished_monotonic_s']=time.monotonic()
        samples.write(json.dumps(row)+'\n');samples.flush()
def drain():
    for line in p.stdout:
        log.write(line);log.flush()
        at=line.find('COMPARE {')
        if at>=0:
            try:
                marker=json.loads(line[at+8:]);marker['received_monotonic_s']=time.monotonic()
                markers.write(json.dumps(marker)+'\n');markers.flush()
                take(marker['phase'])
            except ValueError: pass
reader=threading.Thread(target=drain);reader.start()
start=time.monotonic()
while p.poll() is None and time.monotonic()-start<600:
    take('periodic');time.sleep(1)
if p.poll() is None:
    meta['outer_timeout']=True;p.terminate()
    try:p.wait(timeout=5)
    except subprocess.TimeoutExpired:p.kill();p.wait()
reader.join(timeout=5)
meta.update(exit_code=p.wait(),finished_unix=time.time(),process_identity=identity)
(run/'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
for f in (log,samples,markers):f.close()
print(json.dumps({'run_dir':str(run),'exit_code':meta['exit_code']}),flush=True)
sys.exit(meta['exit_code'])
