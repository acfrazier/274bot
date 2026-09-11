import datetime,fcntl,hashlib,json,os,pathlib,pty,select,struct,subprocess,sys,termios,time
base=pathlib.Path('/home/acfrazier/274bot-campaign/multirevision-20260910');root=base/'platform-isolated-b1cff8a7';revision=sys.argv[1];case=sys.argv[2];assert revision=='289' and case in ('alcher','thiever','chicken_killer')
m=json.loads((root/'source-manifest.json').read_text());build=json.loads((root/'platform-build.json').read_text());job=next(j for j in build['jobs'] if j['name']=='platform_proof');assert build['isolated_build'] and build['target_started_empty'];assert hashlib.sha256((root/'bin/platform_proof-b1cff8a7').read_bytes()).hexdigest()==job['binary_sha256']
for n,h in m['files'].items():assert hashlib.sha256((root/n).read_bytes()).hexdigest()==h,n
out=root/'live'/('r'+revision+'-'+case+'-controls');assert not out.exists();out.mkdir(parents=True);catalog=root/'inputs/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a';nav=root/'nav'/revision/'274bot.navpack';engine=base/'server-289';cmd=[str(root/'bin/platform_proof-b1cff8a7'),'--live','script_'+case,'--profile','local-'+revision,'--revision',revision,'--host','127.0.0.1','--port','44594','--asset-host','127.0.0.1','--http-port','1080','--engine',str(engine),'--nav-pack',str(nav),'--nav-flags',str(nav.with_suffix('.navflags')),'--catalog',str(catalog),'--vault',str(out/'private-vault'),'--unpack',str(out/'unpack')]
env=dict(os.environ);chosen=dict(LIVE='1',BOT_CPU='1',BOT_DEBUG='1',BUDGET_S='180',RUST_BACKTRACE='1',TERM='xterm-256color');env.update(chosen)
for name in ['BOT_LIVE','BOT_MEMORY_N','BOT_MEMORY_WORKLOAD','BOT_MEMORY_SUSTAIN']:env.pop(name,None)
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,140,0,0));started=datetime.datetime.now(datetime.timezone.utc).isoformat();before=time.monotonic();error_log=(out/'stderr.log').open('wb');p=subprocess.Popen(cmd,cwd=root,env=env,stdin=slave,stdout=slave,stderr=error_log);os.close(slave);r=dict(command=cmd,pid=p.pid,started_at=started,environment=chosen,host_commit=build['host_commit'],client_commit=build['client_commit'],binary_sha256=job['binary_sha256'],overlay=m['overlays'],catalog_commit=catalog.name[7:],revision=revision,case=case,source_files_verified=build['source_files_verified'],package_files_verified=len(m['files']),elapsed_is_not_performance_measurement=True);(out/'process.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r),flush=True);timeout=False;control_seen=0;control=out/"control.jsonl";control.touch();input_log=out/"input-forwarded.jsonl"
with (out/'terminal.log').open('wb') as log:
 while True:
  requests=control.read_text().splitlines()
  for request_line in requests[control_seen:]:
   request=json.loads(request_line);assert request['action'] in ('click','escape')
   if request['action']=='click':
    x,y=request['x'],request['y'];assert isinstance(x,int) and isinstance(y,int) and 1<=x<=140 and 1<=y<=40
    sequence=f'\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m'.encode()
   else:sequence=b'\x1b'
   os.write(master,sequence)
   with input_log.open('a') as f:f.write(json.dumps(dict(elapsed_seconds=round(time.monotonic()-before,3),request=request,terminal_bytes_hex=sequence.hex()))+'\n')
   control_seen+=1
  if time.monotonic()-before>260:
   timeout=True;p.terminate()
   try:p.wait(timeout=10)
   except subprocess.TimeoutExpired:p.kill()
  ready,_,_=select.select([master],[],[],1)
  if ready:
   try:data=os.read(master,65536)
   except OSError:break
   if not data:break
   log.write(data);log.flush()
  elif p.poll() is not None:break
os.close(master);code=p.wait();error_log.close();raw=(out/'terminal.log').read_bytes();errors=(out/'stderr.log').read_bytes();decoded=(raw+errors).decode(errors='replace');bad=('panicked at' in decoded or 'FAIL:' in decoded or 'not impl:' in decoded);passed='PASS:' in decoded;r.update(finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),elapsed_seconds=round(time.monotonic()-before,3),process_exit_code=code,wall_timeout=timeout,runtime_error=bad,pass_marker=passed,exit_code=code or int(timeout or bad or not passed),terminal_sha256=hashlib.sha256(raw).hexdigest(),terminal_bytes=len(raw),stderr_sha256=hashlib.sha256(errors).hexdigest(),stderr_bytes=len(errors),input_forwarded_count=control_seen,runner_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest());(out/'receipt.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r),flush=True);sys.exit(r['exit_code'])
