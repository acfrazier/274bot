import ctypes,datetime,hashlib,json,os,pathlib,subprocess,threading,time
from ctypes import wintypes
base=pathlib.Path(__file__).resolve().parent
manifest=json.loads((base/'native-manifest.json').read_text())
out=pathlib.Path.home()/'274bot-runs'/'multirevision-20260910-native-b1cff8a7-r289-alcher'
assert not out.exists();out.mkdir(parents=True)
for name,h in manifest['files'].items():assert hashlib.sha256((base/name).read_bytes()).hexdigest()==h,name
login=json.loads((base/'login-public.json').read_text());catalog='100adccc037d9f6898080e1cad58fcfc43364775';binary=base/'catalog-watch.exe';nav=base/'nav/289/274bot.navpack'
env=dict(os.environ);env.pop('BOT_CPU',None);env.pop('BOT_LIVE',None)
env.update(BUDGET_S='180',BOT_DEBUG='1',RUST_BACKTRACE='1',LOGIN_RSAN=login['modulus'],LOGIN_RSAE=login['exponent']);env['274BOT_SMOKE_DIR']=str(out/'shots')
command=[str(binary),'--live','script_alcher','--profile','local-289','--revision','289','--host','127.0.0.1','--port','44594','--asset-host','127.0.0.1','--http-port','1080','--cache',str(base/'cache'),'--nav-pack',str(nav),'--nav-flags',str(nav.with_suffix('.navflags')),'--catalog',str(base/'inputs'/('rs2b0t-'+catalog)),'--unpack',str(out/'unpack'),'--vault',str(out/'vault')]
start=time.monotonic();receipt=dict(host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],binary_sha256=manifest['binary_sha256'],catalog_commit=catalog,command=command,started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),public_key_sha256=login['public_der_sha256'],elapsed_is_not_performance_measurement=True)
done=threading.Event()
with (out/'run.log').open('w') as log:
 p=subprocess.Popen(command,env=env,cwd=base,stdout=log,stderr=subprocess.STDOUT);receipt['pid']=p.pid;(out/'process.json').write_text(json.dumps(receipt,indent=2)+'\n')
 def responsiveness():
  user32=ctypes.WinDLL('user32',use_last_error=True)
  enum_cb=ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
  user32.EnumWindows.argtypes=[enum_cb,wintypes.LPARAM]
  user32.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
  user32.IsWindowVisible.argtypes=[wintypes.HWND]
  user32.SendMessageTimeoutW.argtypes=[wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM,wintypes.UINT,wintypes.UINT,ctypes.POINTER(ctypes.c_size_t)]
  user32.SendMessageTimeoutW.restype=ctypes.c_ssize_t
  with (out/'window-responsiveness.jsonl').open('w') as probe:
   while time.monotonic()-start<60 and not done.is_set():
    windows=[]
    @enum_cb
    def each(hwnd,param):
     pid=wintypes.DWORD();user32.GetWindowThreadProcessId(hwnd,ctypes.byref(pid))
     if pid.value==p.pid and user32.IsWindowVisible(hwnd):windows.append(hwnd)
     return True
    user32.EnumWindows(each,0)
    for hwnd in windows:
     result=ctypes.c_size_t();before=time.monotonic();ctypes.set_last_error(0)
     ok=user32.SendMessageTimeoutW(hwnd,0,0,0,0x0002,500,ctypes.byref(result))
     probe.write(json.dumps(dict(elapsed_seconds=round(before-start,3),pid=p.pid,hwnd=int(hwnd),responsive=bool(ok),probe_ms=round((time.monotonic()-before)*1000,3),error=ctypes.get_last_error() if not ok else 0))+'\n');probe.flush()
    done.wait(0.5)
 thread=threading.Thread(target=responsiveness,daemon=True);thread.start()
 try:code=p.wait(timeout=260)
 except subprocess.TimeoutExpired:
  p.terminate();p.wait(timeout=10);code=1;receipt['failure']='Native diagnostic exceeded outer 260-second bound'
 finally:done.set();thread.join(timeout=2)
lines=(out/'run.log').read_text(errors='replace').splitlines();errors=[line for line in lines if 'panicked at' in line or line.startswith('FAIL:') or (line.startswith('[script ') and ('not impl' in line or '] tick ' in line))]
files={str(q.relative_to(out)).replace('\\','/'):hashlib.sha256(q.read_bytes()).hexdigest() for q in out.rglob('*') if q.is_file() and (q.suffix in ('.png','.json','.jsonl','.log')) and 'vault' not in q.parts and 'unpack' not in q.parts}
receipt.update(finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),elapsed_seconds=round(time.monotonic()-start,3),process_exit_code=code,exit_code=code or (1 if errors else 0),runtime_errors=errors,log_sha256=hashlib.sha256((out/'run.log').read_bytes()).hexdigest(),evidence_files=files)
(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
raise SystemExit(receipt['exit_code'])
