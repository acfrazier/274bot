from pathlib import Path
import sys,json,time,subprocess
r=Path(__file__).resolve().parents[2];sys.path.insert(0,str(r/'.superpowers/platform-preparation/terminal-reader'));import pyte
rev=sys.argv[1];label=sys.argv[2];p=r/f'docs/compat/evidence/paint-input-ui-025/tui{rev}-{label}';log=(r/'.superpowers/paint-ui'/f'tui-{rev}-{label}.log').open('w');proc=subprocess.Popen(['python3',str(r/'.superpowers/paint-ui/run_tui_025.py'),rev,label],cwd=r,stdout=log,stderr=subprocess.STDOUT);start=time.monotonic();events=[]
def frame():
 s=pyte.Screen(140,40)
 if (p/'terminal.log').exists():pyte.Stream(s).feed((p/'terminal.log').read_bytes().decode('utf-8','replace'))
 return s.display

def callbacks():return [l for l in (p/'stderr.log').read_text().splitlines() if l.startswith('[script ') and ('Go bank pressed' in l or 'Resume pressed' in l)] if (p/'stderr.log').exists() else []
def wait_for(name,predicate):
 while time.monotonic()-start<95:
  f=frame()
  if predicate(f):
   (p/(name+'.txt')).write_text('\n'.join(f)+'\n');events.append({'phase':name,'elapsed_seconds':round(time.monotonic()-start,3),'callbacks':callbacks()});print(name,flush=True);return f
  assert proc.poll() is None,'proof process exited before '+name
  time.sleep(.15)
 raise RuntimeError('UI predicate not reached before original watch: '+name)
def click(x,y):
 with (p/'control.jsonl').open('a') as out:out.write(json.dumps({'action':'click','x':x,'y':y})+'\n')
def label_click(f,needle):
 cells=[(line.index(needle)+3,i+1) for i,line in enumerate(f) if needle in line];assert len(cells)==1,cells;click(*cells[0])
try:
 f=wait_for('visible',lambda f:any('> [1] Go bank' in l for l in f) and any('script: running' in l for l in f));body=next(i for i,l in enumerate(f) if 'Pack ess:' in l);click(10,body+1);time.sleep(1.3);assert not callbacks();f=frame();label_click(f,'Go bank')
 f=wait_for('go-bank',lambda f:any('> [1] Resume' in l for l in f) and len(callbacks())==1);label_click(f,'> [1] Resume')
 f=wait_for('paint-resumed',lambda f:any('> [1] Go bank' in l for l in f) and len(callbacks())==2);label_click(f,'[Pause]')
 f=wait_for('host-paused',lambda f:any('script: paused' in l for l in f));label_click(f,'Go bank');time.sleep(1.3);assert len(callbacks())==2;f=frame();label_click(f,'[Resume]')
 f=wait_for('host-resumed',lambda f:any('script: running' in l for l in f));label_click(f,'[Stop]')
 f=wait_for('host-stopped',lambda f:any('script: idle' in l for l in f) and not any('> [1]' in l for l in f));assert len(callbacks())==2
 result={'ui_controls':'PASS','gameplay_acceptance':False,'above_button_ignored':True,'paused_button_ignored':True,'events':events,'callbacks':callbacks(),'note':'Real PTY input and actual rendered text coordinates; no clock/witness changes. Original Alcher watchdog remains active.'}
except Exception as e:result={'ui_controls':'FAIL','gameplay_acceptance':False,'error':str(e),'events':events,'callbacks':callbacks()}
(p/'controls-result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True);code=proc.wait();print('original diagnostic process exit',code,flush=True)
