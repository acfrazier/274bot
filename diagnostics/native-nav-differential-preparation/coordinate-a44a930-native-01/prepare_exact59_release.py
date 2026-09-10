"""Root admission for one exact59 comparison; never launches a probe."""
from pathlib import Path
import hashlib,json,os,platform,shutil,sys,time
root=Path(__file__).resolve().parent;tool=root/'host/docs/memory/nav-tiled-differential'
sys.path.insert(0,str(tool));import harness as h
run=tool/'coordinate-native-generated-01';binding=root/'host/docs/memory/nav-tiled-stage-a/coordinate-source-binding.json'
digest='7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb'
context=h.source_context(run,binding,digest)
assert not (run/'real-release').exists()
# Validate all source, native guards and generated evidence before private stat/open.
a=dict(schema='nav-coordinate-differential-release-v1',candidate_id='coordinate-ebf0f30',source_binding=context['reference'],phase='native-release',mode='real',released=True,scope='One fresh exact ordered59 full-byte correctness comparison; no StageA performance acceptance',limits=h.REAL)
for n in ('launch.json','result.json','sources.json'):a[n+'_sha256']=h.sha(run/n)
for arm in context['arms']:a[arm+'_admission_sha256']=h.sha(run/(arm+'-admission.json'))
guard=tool/'coordinate-native-guards-01/guards.json'
a.update(guard_report=str(guard),guard_report_sha256=h.sha(guard),input_path='/home/builder/274bot-campaign/nav-census-3d32602-0218/real-input.navpack',input_sha256='2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30',input_bytes=73438581,routes_path=str(root/'exact59-routes.tsv'),routes_sha256='49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125')
h.check_real_release(a,run,context)
started=time.time();boot=Path('/proc/sys/kernel/random/boot_id').read_text().strip()
def cpu():return [int(x) for x in Path('/proc/stat').read_text().splitlines()[0].split()[1:9]]
def vm():return {k:int(v) for k,v in (l.split() for l in Path('/proc/vmstat').read_text().splitlines()) if k in ('pswpin','pswpout','oom_kill')}
mem={}
for line in Path('/proc/meminfo').read_text().splitlines():
 k,v=line.split(':',1);mem[k]=int(v.split()[0])*1024
v0=vm();c0=cpu();time.sleep(1);c1=cpu();v1=vm();delta=[b-a for a,b in zip(c0,c1)];total=sum(delta)
idle=100*(delta[3]+delta[4])/total
matches=[]
for p in Path('/proc').iterdir():
 if not p.name.isdigit() or int(p.name)==os.getpid():continue
 try:
  comm=(p/'comm').read_text().strip();argv=(p/'cmdline').read_bytes().split(b'\0')
  classification=None
  if comm in ('cargo','rustc','cc','cc1','ld','lld'):classification='build'
  elif comm in ('differential-pr','stage-a-probe','tui-play','panel','heaptrack','perf','valgrind'):classification='probe-frontend-profiler'
  elif comm.startswith('python') and any(b'unittest' in v or b'pytest' in v or b'run_native.py' in v for v in argv):classification='test'
  if classification:
   stat=(p/'stat').read_text();start=stat[stat.rfind(')')+2:].split()[19]
   matches.append(dict(kind=classification,pid=int(p.name),start_identity=start,executable_basename=comm))
 except (FileNotFoundError,ProcessLookupError):pass
free=shutil.disk_usage(root).free
pre=dict(scope='fresh root exact59 correctness preflight',started_unix_s=started,ended_unix_s=time.time(),hardware=dict(system=platform.system(),machine=platform.machine(),node=platform.node(),cpus=os.cpu_count()),boot_id=boot,mem_total_bytes=mem['MemTotal'],mem_available_bytes=mem['MemAvailable'],swap_total_bytes=mem['SwapTotal'],swap_free_bytes=mem['SwapFree'],vmstat_before=v0,vmstat_after=v1,idle_interval_s=1,cpu_delta=delta,idle_percent=idle,steal_ticks=delta[7],disk_free_bytes=free,conflicting_matches=matches)
pre['qualified']=platform.system()=='Linux' and platform.machine()=='x86_64' and bool(boot) and mem['MemAvailable']>=2*1024**3 and mem['SwapTotal']==mem['SwapFree'] and v0==v1 and idle>=90 and delta[7]==0 and free>=10*1024**3 and not matches
p=root/'exact59-preflight.json';assert not p.exists();h.save(p,pre)
assert pre['qualified'],pre
# Root release checks precede this first private stat/open.
h.admit(Path(a['input_path']),a['input_sha256'],128*1024**2);assert Path(a['input_path']).stat().st_size==a['input_bytes']
h.admit(Path(a['routes_path']),a['routes_sha256'],65536);assert h.fixed_selectors(Path(a['routes_path']).read_text())==59
out=root/'exact59-authorization.json';assert not out.exists();a['root_preflight_sha256']=h.sha(p);h.save(out,a)
print(json.dumps(dict(preflight=pre,authorization_path=str(out),authorization_sha256=h.sha(out),source_binding_sha256=digest,private_input_hash_verified=True,route_count=59,probe_launched=False),indent=2))
