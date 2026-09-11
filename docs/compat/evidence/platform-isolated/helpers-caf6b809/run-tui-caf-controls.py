"""Run the existing TUI in a real PTY and forward bounded operator input."""
import datetime, fcntl, hashlib, json, os, pathlib, pty, select
import struct, subprocess, sys, termios, time

root = pathlib.Path(sys.argv[1]).resolve()
revision, label = sys.argv[2:]
assert revision in ('274', '289')
assert label and all(c.isalnum() or c == '-' for c in label)
mac = sys.platform == 'darwin'
expected = (pathlib.Path('/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/.superpowers/platform-preparation/platform-isolated-caf6b809') if mac else
            pathlib.Path('/home/acfrazier/274bot-campaign/multirevision-20260910/platform-isolated-caf6b809'))
assert root == expected
manifest = json.loads((root / 'source-manifest.json').read_text())
build = json.loads((root / 'platform-build.json').read_text())
assert build['host_commit'] == 'caf6b809f42294edf80b745208c206fdcccbaa0c'
assert build['client_commit'] == '9d090ed04957e4efc254f073cda97bc5510ca72b'
job = next(j for j in build['jobs'] if j['name'] == 'platform_proof')
binary = root / 'bin/platform_proof-caf6b809'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert build['isolated_build'] and build['target_started_empty'] and job['exit_code'] == 0
assert sha(binary) == job['binary_sha256']
for name, digest in manifest['files'].items():
    assert sha(root / name) == digest, name
out = root / 'live' / ('r' + revision + '-alcher-' + label)
assert not out.exists()
out.mkdir(parents=True)
catalog = root / 'inputs/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a'
nav = root / 'nav' / revision / '274bot.navpack'
if mac:
    engine = pathlib.Path('/Users/acfrazier/experiments') / ('Server/engine' if revision == '274' else 'lostcity-289/engine')
else:
    engine = root.parent / 'server-289' if revision == '289' else root.parent.parent / 'server-4c95f87'
http_port = ('80' if mac else '8080') if revision == '274' else '1080'
cmd = [str(binary), '--live', 'script_alcher', '--profile', 'local-' + revision,
       '--revision', revision, '--host', '127.0.0.1', '--port', '43594' if revision == '274' else '44594',
       '--asset-host', '127.0.0.1', '--http-port', http_port, '--engine', str(engine),
       '--nav-pack', str(nav), '--nav-flags', str(nav.with_suffix('.navflags')),
       '--catalog', str(catalog), '--vault', str(out / 'private-vault'), '--unpack', str(out / 'unpack')]
env = dict(os.environ)
chosen = dict(LIVE='1', BOT_CPU='1', BOT_DEBUG='1', BUDGET_S='180', RUST_BACKTRACE='1', TERM='xterm-256color')
env.update(chosen)
for name in ['BOT_LIVE', 'BOT_MEMORY_N', 'BOT_MEMORY_WORKLOAD', 'BOT_MEMORY_SUSTAIN']:
    env.pop(name, None)
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
started = datetime.datetime.now(datetime.timezone.utc).isoformat()
before = time.monotonic()
errors = (out / 'stderr.log').open('wb')
p = subprocess.Popen(cmd, cwd=root, env=env, stdin=slave, stdout=slave, stderr=errors)
os.close(slave)
r = dict(command=cmd, pid=p.pid, started_at=started, environment=chosen,
         host_commit=build['host_commit'], client_commit=build['client_commit'],
         binary_sha256=job['binary_sha256'], overlay=manifest['overlays'],
         catalog_commit=catalog.name[7:], revision=revision, case='alcher', platform=sys.platform,
         package_files_verified=len(manifest['files']), elapsed_is_not_performance_measurement=True)
(out / 'process.json').write_text(json.dumps(r, indent=2) + '\n')
print(json.dumps(r), flush=True)
control = out / 'control.jsonl'
control.touch()
seen = 0
timeout = False
keys = {'escape': b'\x1b', 'up': b'\x1b[A', 'down': b'\x1b[B',
        'left': b'\x1b[D', 'right': b'\x1b[C', 'enter': b'\r', 'tab': b'\t',
        'backtab': b'\x1b[Z', 'backspace': b'\x7f'}
with (out / 'terminal.log').open('wb') as log:
    while True:
        requests = control.read_text().splitlines()
        for line in requests[seen:]:
            request = json.loads(line)
            if request['action'] == 'click':
                x, y = request['x'], request['y']
                assert isinstance(x, int) and isinstance(y, int) and 1 <= x <= 140 and 1 <= y <= 40
                sequence = f'\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m'.encode()
            elif request['action'] == 'key':
                sequence = keys[request['key']]
            else:
                assert request['action'] == 'text'
                value = request['text']
                assert 1 <= len(value) <= 160 and all(32 <= ord(c) <= 126 for c in value)
                sequence = value.encode()
            os.write(master, sequence)
            with (out / 'input-forwarded.jsonl').open('a') as f:
                f.write(json.dumps(dict(elapsed_seconds=round(time.monotonic() - before, 3),
                                       request=request, terminal_bytes_hex=sequence.hex())) + '\n')
            seen += 1
        if time.monotonic() - before > 260:
            timeout = True
            p.terminate()
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
        ready, _, _ = select.select([master], [], [], 1)
        if ready:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            log.write(data)
            log.flush()
        elif p.poll() is not None:
            break
os.close(master)
code = p.wait()
errors.close()
raw = (out / 'terminal.log').read_bytes()
err = (out / 'stderr.log').read_bytes()
decoded = (raw + err).decode(errors='replace')
bad = any(s in decoded for s in ['panicked at', 'FAIL:', 'not impl:'])
passed = 'PASS: live alcher ' in decoded
leave = raw.rfind(b'\x1b[?1049l')
pass_offset = raw.find(b'PASS: live alcher ')
r.update(finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
         elapsed_seconds=round(time.monotonic() - before, 3), process_exit_code=code,
         wall_timeout=timeout, runtime_error=bad, pass_marker=passed,
         exit_code=code or int(timeout or bad or not passed), terminal_sha256=sha(out / 'terminal.log'),
         terminal_bytes=len(raw), stderr_sha256=sha(out / 'stderr.log'), stderr_bytes=len(err),
         input_forwarded_count=seen, runner_sha256=sha(pathlib.Path(__file__)),
         last_leave_alternate_screen_offset=leave, pass_offset=pass_offset,
         pass_after_terminal_restore=0 <= leave < pass_offset)
(out / 'receipt.json').write_text(json.dumps(r, indent=2) + '\n')
print(json.dumps(r), flush=True)
raise SystemExit(r['exit_code'])
