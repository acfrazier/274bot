#!/usr/bin/python3
"""Run the real TUI in a local PTY and retain scene-readiness output."""
import fcntl, json, os, pty, re, select, signal, struct, sys, termios, time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
proof=root/'.superpowers/profile-native'
out=root/'docs/compat/evidence/session-profile/native-macos'
out.mkdir(parents=True,exist_ok=True)
fixture=json.loads((proof/'fixture.json').read_text())
binary='/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/.superpowers/profile-native/bin/tui-play'
args=[binary,'--revision','274','--vault',fixture['vault'],'--vault-pass','profile-proof','--user',fixture['username']]
env=os.environ.copy()
for name in list(env):
    if name.startswith('BOT_') or name in ('CLIENT_REVISION','ENGINE_DIR','CLIENT_UNPACK_DIR','LOGIN_RSAN','LOGIN_RSAE','NAV_PACK','NAV_FLAGS','RS2B0T'):
        env.pop(name,None)
env['TERM']='xterm-256color'
started=time.time()
pid,master=pty.fork()
if pid==0:
    os.chdir(root)
    os.execve(binary,args,env)
def size(cols):
    fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',44,cols,0,0))
size(155)
data=bytearray(); ready=False; sent_quit=False; status=None; last_resize=started; quit_at=None
log=out/'tui-local-274.ansi'
with log.open('wb') as stream:
    while time.time()-started<100:
        now=time.time()
        if select.select([master],[],[],0.2)[0]:
            try: chunk=os.read(master,65536)
            except OSError: chunk=b''
            if chunk:
                data.extend(chunk); stream.write(chunk); stream.flush()
                if (b'ingame scene 2' in data or re.search(rb'\x1b\[(\d+);9Hingame\x1b\[\1;16Hscene\x1b\[\1;22H2', data)) and not ready:
                    ready=True
                    print('Observed TUI state: ingame scene 2',flush=True)
        done,wait_status=os.waitpid(pid,os.WNOHANG)
        if done:
            status=wait_status; break
        if ready and not sent_quit:
            os.write(master,b'q'); sent_quit=True; quit_at=now
        if not ready and now-started>=85 and not sent_quit:
            print('FAIL: TUI did not display ingame scene 2 within 85 seconds',flush=True)
            os.write(master,b'q'); sent_quit=True; quit_at=now
        if quit_at is not None and now-quit_at>10:
            os.kill(pid,signal.SIGTERM)
            _,status=os.waitpid(pid,0); break
        if not sent_quit and now-last_resize>3:
            size(155+(int(now-started)//3)%2); last_resize=now
if status is None:
    os.kill(pid,signal.SIGTERM); _,status=os.waitpid(pid,0)
os.close(master)
code=os.waitstatus_to_exitcode(status)
result={'binary':binary,'platform':sys.platform,'source_head':'a02c9dd531124a5bb8dfb936bed20bd92e52730d','revision':274,'username':fixture['username'],'started_at':started,'elapsed_seconds':time.time()-started,'pty_rows':44,'pty_columns':[155,156],'native_window_visual_inspection':False,'terminal_access_note':'CUA refused iTerm2 app access; actual TUI verified through local macOS PTY','readiness_marker':'ingame scene 2' if ready else None,'quit_key_sent':sent_quit,'process_exit_code':code,'passed':ready and code==0,'transcript':str(log.relative_to(root))}
(out/'tui-local-274.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2),flush=True)
sys.exit(0 if result['passed'] else 1)
