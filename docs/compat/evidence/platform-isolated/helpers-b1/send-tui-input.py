import subprocess,sys,json,shlex,datetime,pathlib
x,y=int(sys.argv[1]),int(sys.argv[2]);label=sys.argv[3];assert 1<=x<=140 and 1<=y<=40 and label.isalnum()
request=dict(action='click',x=x,y=y,label=label,requested_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
remote='/home/acfrazier/274bot-campaign/multirevision-20260910/platform-isolated-b1cff8a7/live/r289-alcher-controls/control.jsonl'
code='import pathlib; p=pathlib.Path('+repr(remote)+'); assert p.exists(); f=p.open("a"); f.write('+repr(json.dumps(request)+'\n')+'); f.close()'
subprocess.run(['ssh','-o','BatchMode=yes','-o','ConnectTimeout=10','concord','python3 -c '+shlex.quote(code)],check=True);print(json.dumps(request))
