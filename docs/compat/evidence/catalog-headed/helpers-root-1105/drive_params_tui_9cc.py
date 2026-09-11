import pathlib,json,sys,time,re,hashlib,datetime
r=pathlib.Path(__file__).resolve().parents[2];sys.path.insert(0,str(r/'.superpowers/platform-preparation/terminal-reader'));import pyte
out=r/'docs/compat/evidence/catalog-params-9cc6adb1/tui274-choices2';cap=out/'controls';cap.mkdir(exist_ok=True);stage=0;start=time.monotonic();changed=0;state={}
def send(reqs):
 with (out/'control.jsonl').open('a') as f:
  for req in reqs:f.write(json.dumps(req)+'\n')
def key(k):return dict(action='key',key=k)
def click(x,y):return dict(action='click',x=x,y=y)
def capture(label,raw,text):
 p=cap/(label+'.ansi');p.write_bytes(raw);p.with_suffix('.txt').write_text(text);p.with_suffix('.json').write_text(json.dumps(dict(at=datetime.datetime.now(datetime.timezone.utc).isoformat(),raw_sha256=hashlib.sha256(raw).hexdigest(),raw_bytes=len(raw)),indent=2)+'\n');print(label,flush=True)
while time.monotonic()-start<190:
 raw=(out/'terminal.log').read_bytes();screen=pyte.Screen(140,40);pyte.Stream(screen).feed(raw.decode(errors='replace'));lines=screen.display;text='\n'.join(f'{i+1:02d}: {s}' for i,s in enumerate(lines));rows=lambda term:[i+1 for i,s in enumerate(lines) if term in s]
 if b'\x1b[?1049l' in raw:raise SystemExit('Process exited before UI sequence finished')
 if time.monotonic()-changed<0.3:time.sleep(0.1);continue
 next_stage=False
 if stage==0 and re.search(r'Alchs [1-9]',text) and rows('[Browse]'):
  capture('initial-alchemy',raw,text);y=rows('[Browse]')[0];send([click(28,y),click(6,y)]);next_stage=True
 elif stage==1 and rows('Alcher  [Compat]'):
  send([click(10,rows('Alcher  [Compat]')[0])]);next_stage=True
 elif stage==2 and 'sel: Alcher' in text and 'script — browse' in text:
  send([click(6,rows('[Browse]')[0])]);next_stage=True
 elif stage==3 and rows('[Params]'):
  send([click(6,rows('[Params]')[0])]);next_stage=True
 elif stage==4 and '> Items to alch:' in text:
  send([key('enter')]);next_stage=True
 elif stage==5 and '> [ ] custom' in text and '[ ] Rune chainbody' in text:
  capture('choices-before',raw,text);send([key('down')]*5+[dict(action='text',text=' ')]);next_stage=True
 elif stage==6 and '> [x] Rune chainbody' in text:
  capture('chain-selected',raw,text);send([key('enter')]);next_stage=True
 elif stage==7 and '> Items to alch:' in text:
  capture('saved-form',raw,text);send([key('escape')]);next_stage=True
 elif stage==8 and rows('[Params]') and 'parameters' not in text:
  send([click(6,rows('[Params]')[0])]);next_stage=True
 elif stage==9 and '> Items to alch:' in text:
  send([key('enter')]);next_stage=True
 elif stage==10 and 'Items to alch choices' in text and '[x] Rune chainbody' in text:
  capture('reopened-persisted',raw,text);send([key('escape'),key('escape')]);next_stage=True
 elif stage==11 and rows('[Params]') and 'parameters' not in text:
  capture('before-fresh-start',raw,text);m=re.search(r'Rune chainbody x(\d+), Nature rune x(\d+), Coins x(\d+)',text);assert m;state['before']=list(map(int,m.groups()));send([click(13,rows('[Browse]')[0])]);next_stage=True
 elif stage==12 and 'Alcher — Rune chainbody' in text:
  m=re.search(r'Rune chainbody x(\d+), Nature rune x(\d+), Coins x(\d+)',text)
  if m:
   now=list(map(int,m.groups()));old=state['before']
   if now[0]<old[0] and now[1]<old[1] and now[2]>old[2]:
    capture('fresh-start-alchemy',raw,text);state['after']=now;state['ui_sequence_completed']=True;(out/'ui-sequence.json').write_text(json.dumps(state,indent=2)+'\n');print(json.dumps(state),flush=True);break
 if next_stage:stage+=1;changed=time.monotonic();print('stage',stage,flush=True)
 time.sleep(0.2)
else:raise SystemExit('UI sequence deadline at stage '+str(stage))
