import pathlib, sys, subprocess, shlex, json, hashlib, datetime
base = pathlib.Path(__file__).resolve().parents[2]
surface, revision, run_label, shot_label = sys.argv[1:]
assert surface in ('mac', 'concord') and revision in ('274', '289')
assert all(x and all(c.isalnum() or c == '-' for c in x) for x in [run_label, shot_label])
root = (base / '.superpowers/platform-preparation/platform-isolated-e72cfb39' if surface == 'mac' else
        pathlib.Path('/home/acfrazier/274bot-campaign/multirevision-20260910/platform-isolated-e72cfb39'))
source = root / 'live' / ('r' + revision + '-alcher-' + run_label) / 'terminal.log'
if surface == 'mac':
    raw = source.read_bytes()
else:
    code = 'import pathlib,sys;sys.stdout.buffer.write(pathlib.Path(' + repr(str(source)) + ').read_bytes())'
    raw = subprocess.check_output(['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', 'concord', 'python3 -c ' + shlex.quote(code)])
sys.path.insert(0, str(base / '.superpowers/platform-preparation/terminal-reader'))
import pyte
screen = pyte.Screen(140, 40)
pyte.Stream(screen).feed(raw.decode(errors='replace'))
text = '\n'.join(f'{i + 1:02d}: {line}' for i, line in enumerate(screen.display)) + '\n'
dest = base / 'docs/compat/evidence/platform-isolated' / (surface + '-tui-e72cfb39') / ('r' + revision + '-' + run_label) / 'controls'
dest.mkdir(parents=True, exist_ok=True)
p = dest / shot_label
assert not p.with_suffix('.json').exists()
p.with_suffix('.ansi').write_bytes(raw)
p.with_suffix('.txt').write_text(text)
r = dict(captured_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), source=str(source),
         terminal_size=[140, 40], decoder='pyte0.8.2', raw_sha256=hashlib.sha256(raw).hexdigest(),
         screen_sha256=hashlib.sha256(text.encode()).hexdigest(),
         screen_file=str(p.with_suffix('.txt').relative_to(base)), active_pass_text_present=b'PASS: live alcher ' in raw)
p.with_suffix('.json').write_text(json.dumps(r, indent=2) + '\n')
print(json.dumps(r))
print(text)
