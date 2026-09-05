#!/usr/bin/env python3
"""Run one diagnostic cell without overwriting the T4 baseline artifacts."""
import argparse, hashlib, json, os, pathlib, signal, subprocess, sys, time
import errno, fcntl, pty, struct, termios, threading
p = argparse.ArgumentParser()
p.add_argument('frontend', choices=['panel','tui'])
p.add_argument('n', type=int, choices=[1,32,128])
p.add_argument('workload', choices=['idle','active','lifecycle'])
p.add_argument('--nav-captures', action='store_true', help='Diagnostic only: capture navigation checkpoints and failure, temporarily focusing affected bot')
p.add_argument('--single-renderer', action='store_true', help='Panel: fixed slot zero draws; other slots simulate only')
p.add_argument('--headless', action='store_true', help='TUI diagnostic only: skip terminal drawing')
p.add_argument('--debug', action='store_true')
p.add_argument('--sustain', action='store_true')
p.add_argument('--stack-logging', action='store_true')
p.add_argument('--observe', type=int, default=600)
p.add_argument('--warmup', type=int, default=120)
a = p.parse_args()
if a.nav_captures and (a.frontend != "panel" or not a.single_renderer): p.error("--nav-captures requires panel --single-renderer")
if a.single_renderer and a.frontend != "panel": p.error("--single-renderer requires panel")
terminal = a.frontend == 'tui' and not a.headless
root = pathlib.Path(__file__).resolve().parents[2]
binary = root / 'target/release' / (a.frontend+'-play')
run = root / 'docs/memory/diagnostics' / (time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'_'+a.frontend+f'_n{a.n}_{a.workload}')
run.mkdir(parents=True, exist_ok=False)
env = os.environ.copy()
for k in ['BOT_CPU','BOT_LIVE','BOT_DEBUG','MallocStackLogging','MallocStackLoggingNoCompact','BOT_MEMORY_SUSTAIN','BOT_MEMORY_SINGLE_RENDERER','BOT_NAV_CAPTURES']:
    env.pop(k, None)
env.update(LIVE='1', BOT_TARGET='local', BOT_MEMORY_N=str(a.n), BOT_MEMORY_WORKLOAD=a.workload,
           BOT_MEMORY_OUTPUT=str(run/'samples.jsonl'), BOT_MEMORY_DIAGNOSTICS='1',
           BOT_MEMORY_WARMUP_S=str(a.warmup), BOT_MEMORY_OBSERVE_S=str(a.observe))
env.setdefault('RS2B0T','/Users/acfrazier/experiments/rs2b0t')
if a.debug: env['BOT_DEBUG'] = '1'
if a.nav_captures:
    env['BOT_NAV_CAPTURES'] = '1'
    env['274BOT_SMOKE_DIR'] = str(run/'captures')
if a.single_renderer: env['BOT_MEMORY_SINGLE_RENDERER'] = '1'
if a.sustain: env['BOT_MEMORY_SUSTAIN'] = '1'
if a.stack_logging: env['MallocStackLogging'] = '1'
def git(*args):
    return subprocess.check_output(['git',*args],cwd=root,text=True).strip()
def source_digest(directory):
    files = subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=directory).split(b'\0')
    digest = hashlib.sha256()
    for name in sorted(set(files)):
        if not name: continue
        relative = pathlib.Path(os.fsdecode(name))
        if not (relative.parts[0] == 'crates' or relative.name in ('Cargo.toml','Cargo.lock')): continue
        path = directory / relative
        if path.is_file(): digest.update(name+b'\0'+path.read_bytes()+b'\0')
    return digest.hexdigest()
meta = dict(host_sources_sha256=source_digest(root),client_sources_sha256=source_digest(root/'vendor/fr-client-rust'),frontend=a.frontend,n=a.n,workload=a.workload,warmup_s=a.warmup,observe_s=a.observe,
            diagnostic_only=True,nav_captures=a.nav_captures,single_renderer=a.single_renderer,render_policy=("fixed-one" if a.single_renderer else "rotating-all") if a.frontend == "panel" else "none",terminal=terminal,terminal_size=[120,40] if terminal else None,debug=a.debug,sustain=a.sustain,stack_logging=a.stack_logging,binary=str(binary),
            binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
            host_commit=git('rev-parse','HEAD'),client_commit=git('-C','vendor/fr-client-rust','rev-parse','HEAD'),
            host_diff_sha256=hashlib.sha256(git('diff','HEAD').encode()).hexdigest(),
            rs2b0t_commit=subprocess.check_output(['git','-C',env['RS2B0T'],'rev-parse','HEAD'],text=True).strip(),
            run_dir=str(run),started_unix=time.time())
with (run/'run.log').open('xb') as log:
    reader = None
    if terminal:
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',40,120,0,0))
        env['TERM']='xterm-256color'
        def terminal_session():
            os.setsid()
            fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        child=subprocess.Popen([str(binary)],cwd=root,env=env,stdin=slave,stdout=slave,stderr=slave,preexec_fn=terminal_session)
        os.close(slave)
        def drain_terminal():
            try:
                while True:
                    try: data=os.read(master,65536)
                    except OSError as error:
                        if error.errno==errno.EIO: break
                        raise
                    if not data: break
                    log.write(data)
            finally: os.close(master)
        reader=threading.Thread(target=drain_terminal)
        reader.start()
    else:
        child = subprocess.Popen([str(binary)],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
    meta['pid']=child.pid
    (run/'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
    print(json.dumps(meta),flush=True)
    def stop(sig,frame): child.terminate()
    signal.signal(signal.SIGTERM,stop)
    signal.signal(signal.SIGINT,stop)
    rc=child.wait()
    if reader: reader.join()
meta.update(exit_code=rc,ended_unix=time.time())
(run/'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
print(json.dumps({'run_dir':str(run),'exit_code':rc}),flush=True)
sys.exit(rc if rc >= 0 else 128-rc)
