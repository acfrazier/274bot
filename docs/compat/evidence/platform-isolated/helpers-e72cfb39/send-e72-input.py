import pathlib, sys, subprocess, shlex, json, datetime
base = pathlib.Path(__file__).resolve().parents[2]
surface, revision, label, payload = sys.argv[1:]
assert surface in ('mac', 'concord') and revision in ('274', '289')
assert label and all(c.isalnum() or c == '-' for c in label)
request = json.loads(payload)
assert request['action'] in ('click', 'key', 'text')
request['requested_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
root = (base / '.superpowers/platform-preparation/platform-isolated-e72cfb39' if surface == 'mac' else
        pathlib.Path('/home/acfrazier/274bot-campaign/multirevision-20260910/platform-isolated-e72cfb39'))
run = root / 'live' / ('r' + revision + '-alcher-' + label)
code = ('import pathlib,json,subprocess; p=pathlib.Path(' + repr(str(run)) + '); '
        'assert not (p/"receipt.json").exists(); r=json.loads((p/"process.json").read_text()); '
        'cmd=subprocess.check_output(["ps","-p",str(r["pid"]),"-o","command="],text=True); '
        'assert ' + repr(str(root / 'bin/platform_proof-e72cfb39')) + ' in cmd; '
        'f=(p/"control.jsonl").open("a"); f.write(' + repr(json.dumps(request) + '\n') + '); f.close()')
if surface == 'mac':
    subprocess.run([sys.executable, '-c', code], check=True)
else:
    subprocess.run(['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', 'concord', 'python3 -c ' + shlex.quote(code)], check=True)
print(json.dumps(request))
