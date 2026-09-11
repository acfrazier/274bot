import pathlib,subprocess,json,datetime,hashlib,sys
root=pathlib.Path(__file__).resolve().parents[2];label=sys.argv[1];assert label.isalnum()
out=root/'docs/compat/evidence/platform-isolated/concord-tui-b1cff8a7/controls';out.mkdir(parents=True,exist_ok=True);raw=out/(label+'.ansi');assert not raw.exists()
remote='concord:/home/acfrazier/274bot-campaign/multirevision-20260910/platform-isolated-b1cff8a7/live/r289-alcher-controls/terminal.log'
subprocess.run(['scp','-q','-o','BatchMode=yes','-o','ConnectTimeout=10',remote,str(raw)],check=True)
r=subprocess.run(['python3',str(root/'.superpowers/platform-preparation/read-terminal.py'),str(raw)],capture_output=True,text=True,check=True)
(raw.with_suffix('.txt')).write_text(r.stdout);(raw.with_suffix('.json')).write_text(json.dumps(dict(observed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),raw_sha256=hashlib.sha256(raw.read_bytes()).hexdigest(),raw_bytes=raw.stat().st_size,screen_sha256=hashlib.sha256(r.stdout.encode()).hexdigest(),method='Interpret actual captured PTY stdout using pyte0.8.2; stderr held separately by private runner'),indent=2)+'\n');print(r.stdout)
