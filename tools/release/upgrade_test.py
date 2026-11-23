"""Exercise released tui-play binaries against disposable state. See README.md."""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time


class Failure(Exception):
    """The release gate lacks an observation that it requires."""


class Refused(Failure):
    """An actual historical reader refused a supported saved format."""


def require(condition, message):
    if not condition:
        raise Failure(message)


def fresh_home(work):
    home = work / 'home'
    home.mkdir(mode=0o700)
    state = home / '.274bot'
    state.mkdir(mode=0o700)
    return state


def tui(binary, state, profile, cache):
    return [str(binary.resolve(strict=True)), '--profile', profile, '--revision', '289',
            '--cache', str(cache.resolve(strict=True)), '--unpack', str(state / 'unpack-289'),
            '--vault-pass-stdin']


def write_shape_vault(state, profiles, passphrase):
    # Only the historical fixture uses the 0.2.8 wire format. The live gate
    # never opens a vault except through the shipped or candidate executable.
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    except ImportError as error:
        raise Failure('control needs Python with the cryptography package') from error
    salt, nonce = os.urandom(16), os.urandom(12)
    rounds = 100_000
    key = hashlib.pbkdf2_hmac('sha256', passphrase, salt, rounds, 32)
    data = {name: dict(username=name, password=secrets.token_hex(12), uid=i,
                       settings=settings) for i, (name, settings) in enumerate(profiles.items(), 1)}
    cipher = AESGCM(key).encrypt(nonce, json.dumps(data).encode(), None)
    (state / 'vault-289').write_bytes(b'274VAULT\x01' + rounds.to_bytes(4, 'little') + salt + nonce + cipher)


def control(args, work):
    # These TUIs negotiate /crc before unlocking even a local vault. Replay
    # only the archived bytes; there is no game service or simulated outcome.
    with args.crc_file.open('rb') as source:
        crc = source.read(41)
    require(len(crc) == 40, 'expected a 40-byte archived /crc response')

    class Archive(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path != '/crc':
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', str(len(crc)))
            self.end_headers()
            self.wfile.write(crc)

        def log_message(self, *unused):
            pass

    with HTTPServer(('127.0.0.1', 0), Archive) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            historical_readers(args, work, server.server_port)
        finally:
            server.shutdown()
            thread.join()


def copy_stores(source, target):
    # Copy only saved user data, never caches, logs, locks or links.
    names = ('vault-289', 'panel-ui.json', 'loadouts.json', 'servers.json', 'script-settings.json',
             'js-scripts.json', 'rs2b0t-import', 'rs2b0t-path', 'bank-hints', 'script-runtime-settings')

    def copy(path, dest):
        require(not path.is_symlink(), f'{path.name}: snapshot contains a link')
        if path.is_dir():
            dest.mkdir()
            for child in path.iterdir():
                copy(child, dest / child.name)
        else:
            require(path.is_file(), f'{path.name}: not a regular file')
            shutil.copyfile(path, dest)

    for name in names:
        path = source / name
        if path.exists() or path.is_symlink():
            copy(path, target / name)


def startup_result(code, out, missing, lane):
    if code == 1 and 'a format from before 0.2.8' in out:
        files = [p for p in ('loadouts.json', 'bank-hints/local-289/alice.json', 'vault-289') if p in out]
        raise Refused(f'{lane}: refused {", ".join(files)}')
    require(code == 1 and f'no profile named {missing} in the vault:' in out,
            f'{lane}: unexpected startup outcome (exit {code}); no compatibility verdict')


def historical_readers(args, work, port):
    fixture = json.loads((Path(__file__).parent / 'fixtures/upgrade-028.json').read_text())
    profiles = fixture.pop('profile_settings')
    failures = []
    for lane in ('snapshot', 'older-shapes', 'vault-shapes'):
        directory = work / lane
        directory.mkdir()
        state = fresh_home(directory)
        if lane == 'vault-shapes':
            password = secrets.token_hex(24).encode()
            write_shape_vault(state, profiles, password)
            users = list(profiles)
        else:
            password = args.pass_file.read_bytes().rstrip(b'\r\n')
            users = [args.account]
            copy_stores(args.snapshot, state)
            if lane == 'older-shapes':
                for name, document in fixture.items():
                    target = state / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(json.dumps(document))
        # The historical TUI validates these names in order AFTER unlocking
        # the explicitly existing vault, before starting any slot. The final
        # (absent) name gives positive evidence without a timeout or login.
        missing = 'absent' + secrets.token_hex(3)
        cmd = tui(args.binary, state, 'local-289', args.cache)
        cmd.extend(['--host', '127.0.0.1', '--port', '1', '--asset-host', '127.0.0.1', '--http-port', str(port)])
        for user in [*users, missing]:
            cmd.extend(['--user', user])
        run = subprocess.run(cmd, env=dict(os.environ, HOME=str(state.parent)), input=password+b'\n',
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        out = run.stdout.decode(errors='replace')
        try:
            startup_result(run.returncode, out, missing, lane)
            print(f'{lane}: startup accepted data and validated existing profiles')
        except Refused as error:
            failures.append(str(error))
    # Probe the escaped Gatherer break separately from the startup refusal.
    # The game port is closed: this control checks admission, not gameplay.
    with tempfile.TemporaryDirectory(prefix='ug-', dir='/tmp') as socket_dir:
        socket = Path(socket_dir) / 's'
        try:
            pane = Pane(socket, work/'snapshot/home/.274bot', args.binary, 'local-289',
                        args.account, args.pass_file.read_bytes().rstrip(b'\r\n'), work,
                        'schema', args.cache, archive_port=port)
            pane.script_pane()
            pane.select('Gatherer')
            refused = 'compiled schema 4 unavailable for Gatherer'
            pane.key('t')
            screen = pane.until(lambda s: refused in s or
                                re.search(r'script: (running|waiting|working)\s+sel: Gatherer\b', s),
                                'no Gatherer admission decision')
            if refused in screen:
                failures.append('snapshot: Gatherer refused schema 4')
            else:
                pane.key('e')
                pane.until(lambda s: 'script: idle' in s, 'Gatherer did not stop')
                pane.parameter('Gatherer', 'skill: Mining')
                print('snapshot: Gatherer accepted schema 4 and restored Mining')
            pane.quit()
            if not failures:
                old = Pane(socket, work/'older-shapes/home/.274bot', args.binary, 'local-289',
                           args.account, args.pass_file.read_bytes().rstrip(b'\r\n'), work,
                           'legacy', args.cache, archive_port=port)
                old.saved_ui('Lobster:1', 'Bronze axe')
                old.quit()
                print('older-shapes: read legacy gear, carry quantity and manual movement preference')
        finally:
            close_server(socket)
    if failures:
        raise Refused(' | '.join(failures))


def close_server(socket):
    if socket.exists():
        subprocess.run(['tmux', '-S', str(socket), 'kill-server'], capture_output=True, timeout=5)


# Public compiled cards in the full 0.2.9 build. WalkTo and CombatSession
# are internal, not user-selectable. Each row differs from the shipped default.
CARDS = {'Gatherer': 'skill: Mining', 'Grinch': 'radius (1–16): 7',
         'Quester': 'Maximum deaths (0–255): 3', 'Sherlock': 'Allow teleports: On (script opt-in)'}


CATALOG = Path(__file__).resolve().parent / 'fixtures/upgrade-catalog'
PROBE = CATALOG / 'src/bot/scripts/UpgradeProbe.ts'
RAW = '  release:029  '
PROFILE = 'upgrade-289'


def rename_local_profile(state):
    roster = state / 'servers.json'
    doc = json.loads(roster.read_text())
    local, = (row for row in doc['servers'] if row['name'] == 'local-289')
    world = local['worlds'][0]
    require(local['revision'] == 289 and local['vault'] == 'vault-289' and local['transport'] == 'tcp'
            and world['host'] == world['asset_host'] == '127.0.0.1', 'expected shipped loopback server')
    local['name'] = PROFILE
    roster.write_text(json.dumps(doc))
    return f"{PROFILE} · {world['host']}:{world['port']} · revision 289"


def check_writer(state, profile, account, since):
    def load(name):
        return json.loads((state / name).read_text())
    require((state/'vault-289').read_bytes().startswith(b'274VAULT'), 'missing encrypted vault')
    servers = load('servers.json')
    require(servers['schema_version'] == 1 and any(s['name'] == profile for s in servers['servers']), 'missing server profile')
    require(any(row['name'] == 'upgrade' and {'item': 'Lobster', 'qty': 7} in row['carry']
                for row in load('loadouts.json')), 'missing upgrade loadout or carry quantity')
    prefs = load('panel-ui.json')
    require(prefs['nav']['pause_script_on_manual_walk_abort'] is False and prefs['session_log_file'] is True,
            'missing nondefault preference or session logging')
    require(load('script-settings.json').get('file:UpgradeProbe', {}).get('mode') == 'shared',
            'missing shared card override')
    require({'name': 'UpgradeProbe', 'path': str(PROBE)} in load('js-scripts.json'), 'missing file script')
    hint = load(f'bank-hints/{profile}/{account}.json')
    require(hint['profile'] == profile and hint['account'] == account and hint['revision'] == 289
            and hint['content'] and hint['schema_version'] == 1 and since <= hint['observed_at_unix'] <= time.time()
            and hint['rows'], 'no scoped bank observation from this run')
    runtime = load(f"script-runtime-settings/{profile}/{account}/289-{hint['content']}.json")
    require(all(runtime[k] == hint[k] for k in ('profile', 'account', 'revision', 'content', 'schema_version'))
            and {'namespace': 'UpgradeProbe', 'key': 'sentinel', 'raw': RAW} in runtime['rows'],
            'missing scoped runtime setting')
    # The shipped host reports the frame-open edge and the durable close
    # write separately. A hand-written equivalent JSON file proves neither.
    opened = closed = False
    for path in (state / 'logs').glob('session-*.log*'):
        with path.open(errors='replace') as log:
            for line in log:
                opened |= 'bank memory observed rows=' in line
                closed |= ('bank hints: saved rows=' in line and ' why=close ' in line
                           and f'bank-hints/{profile}/{account}.json' in line)
    require(opened and closed, 'missing live bank open or durable close observation')
    return f"bank hints: loaded rows={len(hint['rows'])} observed_at_unix={hint['observed_at_unix']}"


class Pane:
    def __init__(self, socket, state, binary, profile, account, password, work, phase, cache, archive_port=None):
        self.tmux = ['tmux', '-u', '-f', '/dev/null', '-S', str(socket)]
        self.phase = phase
        self.status = work / (phase + '.exit')
        self.logs = state / 'logs'
        self.previous_logs = set(self.logs.glob('session-*.log*'))
        cmd = tui(binary, state, profile, cache)
        if archive_port is not None:
            cmd.extend(['--host', '127.0.0.1', '--port', '1', '--asset-host', '127.0.0.1', '--http-port', str(archive_port)])
        if account:
            cmd.extend(['--user', account])
        # Paths are separate argv elements, never shell source. Only exit status
        # is retained. No pane contents, typed credentials, or environment dump.
        self.call('new-session', '-d', '-s', phase, '-x', '180', '-y', '60',
                  'env', f'HOME={state.parent}', 'sh', '-c',
                  'status=$1; shift; "$@"; echo $? >"$status"',
                  'upgrade', str(self.status), *cmd)
        self.until(lambda s: 'Vault passphrase:' in s, 'no unlock prompt', 90)
        subprocess.run([*self.tmux, 'load-buffer', '-b', 'unlock', '-'], input=password,
                       check=True, timeout=5)
        self.call('paste-buffer', '-d', '-b', 'unlock', '-t', phase)
        self.key('Enter')
        self.until(lambda s: 'headless' in s.lower(), 'did not reach main screen after unlock', 90)

    def call(self, *args):
        return subprocess.check_output([*self.tmux, *args], text=True, timeout=5, stderr=subprocess.DEVNULL)

    def screen(self):
        require(not self.status.exists(), f'{self.phase}: TUI exited before the required observation')
        screen = self.call('capture-pane', '-p', '-t', self.phase)
        require('Move these files aside' not in screen and 'a format from before' not in screen,
                f'{self.phase}: startup refused saved data')
        return screen

    def key(self, *keys):
        self.call('send-keys', '-t', self.phase, *keys)

    def until(self, predicate, why, seconds=15):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            screen = self.screen()
            if predicate(screen):
                return screen
            time.sleep(0.1)
        raise Failure(f'{self.phase}: {why}')

    def script_pane(self):
        self.key('F5')
        self.until(lambda s: 'script: idle' in s, 'script pane not idle')

    def select(self, card, badges='[Compiled] [Builtin]'):
        self.key('b')
        self.until(lambda s: '[Compiled] [Builtin]' in s, 'native picker not visible')
        selected = re.compile(r'> ' + re.escape(card) + r'\s+' + re.escape(badges))
        for _ in range(128):
            screen = self.screen()
            if selected.search(screen):
                self.key('Enter')
                self.until(lambda s: 'script — browse' not in s, 'picker did not close')
                return
            last = re.search(r'script: idle\s+sel: (\S+)', screen)
            self.key('Down')
            self.until(lambda s: not last or last.group() not in s, 'picker did not advance')
        raise Failure(f'{self.phase}: missing {badges} card {card}')

    def parameter(self, card, setting):
        self.key('v')
        row = re.compile(r'(?:>|│)\s*' + re.escape(setting) + r'\s*│')
        self.until(lambda s: '┌parameters' in s and row.search(s), f'{card}: missing {setting}')
        self.key('Escape')
        self.until(lambda s: '┌parameters' not in s, 'parameter editor did not close')

    def cards(self):
        self.script_pane()
        for card, setting in CARDS.items():
            self.select(card)
            self.parameter(card, setting)
            idle = re.compile(r'script: idle\s+sel: ' + card + r'\b')
            self.until(idle.search, 'card not idle')
            self.key('t')
            started = re.compile(r'script: (working|waiting)\s+sel: ' + card + r'\b')
            self.until(started.search, f'{card}: start did not reach working or waiting', 45)
            print(f'{self.phase}: {card}: {setting}; start observed', flush=True)
            self.key('e')
            self.until(idle.search, f'{card}: stop did not settle')

    def saved_ui(self, carry, unassigned=None):
        self.key('F3', 'l')
        self.until(lambda s: 'selected: upgrade' in s and f'carry item:qty: {carry}' in s
                   and (not unassigned or f'unassigned: {unassigned}' in s), 'loadout not restored')
        self.key('Escape')
        self.until(lambda s: 'carry item:qty:' not in s, 'loadout editor did not close')
        self.key('o')
        self.until(lambda s: 'random events:' in s, 'settings did not open')
        self.key(*(['Down'] * 11))
        self.until(lambda s: 'Pause script on manual movement: false' in s, 'preference not restored')
        self.key('Escape')

    def logged(self, message):
        def has_line(_screen):
            for path in set(self.logs.glob('session-*.log*')) - self.previous_logs:
                with path.open(errors='replace') as log:
                    if any(message in line for line in log):
                        return True
            return False
        self.until(has_line, f'missing new session-log receipt: {message}', 60)

    def probe(self, mode, raw, *, catalog=False):
        self.script_pane()
        if catalog:
            self.select('UpgradeCatalog', '[Compat] [Catalog]')
        else:
            self.select('UpgradeProbe', '[Compat] [File]')
        self.key('t')
        self.logged(f'UPGRADE_READ mode={mode} raw={raw}')
        self.key('e')
        self.until(lambda s: 'script: idle' in s, 'probe did not stop')

    def quit(self):
        self.key('C-q')
        self.until(lambda s: 'Quit tui-play' in s, 'missing quit confirmation')
        self.key('y')
        end = time.monotonic() + 30
        while time.monotonic() < end:
            if self.status.is_file() and self.status.read_text().strip() == '0':
                return
            time.sleep(.1)
        raise Failure(f'{self.phase}: TUI did not exit cleanly')


def live(args, work):
    # The checksum comes from the selected release package, not a version
    # printed by a build under test. Never run a candidate as the producer.
    def digest(path):
        h = hashlib.sha256()
        with path.open('rb') as stream:
            for chunk in iter(lambda: stream.read(1024*1024), b''):
                h.update(chunk)
        return h.hexdigest()
    shipped = digest(args.shipped)
    candidate = digest(args.candidate)
    require(shipped == args.shipped_sha256 and candidate != shipped, 'incorrect producer or identical candidate')
    require(digest(args.shipped_panel) == args.panel_sha256, 'incorrect producer panel')
    require(all(re.fullmatch(r'[a-z0-9]+', u) for u in (args.account, args.shared_account))
            and args.account != args.shared_account, 'supply two distinct disposable accounts')
    require(not os.environ.get('RS2B0T'), 'unset RS2B0T; only the persisted catalog root may supply the card')
    since = int(time.time())
    state = fresh_home(work)
    password = secrets.token_hex(32).encode()
    # Unix socket paths have a small platform limit. Keep this server private.
    with tempfile.TemporaryDirectory(prefix='ug-', dir='/tmp') as socket_dir:
        sock = Path(socket_dir)/'s'
        try:
            # The TUI has no editor for settings without a focused profile.
            # Only the shipped panel can produce this global override.
            def panel(profile):
                subprocess.run(tui(args.shipped_panel, state, profile, args.cache), input=password+b'\n',
                               env=dict(os.environ, HOME=str(state.parent)), stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, check=True, timeout=900)
            print(f'In Browse choose Not now. Load {PROBE}, set global mode=shared, enable logging and quit.', flush=True)
            panel('local-289')
            require((state/'rs2b0t-import').read_text().strip() == 'deferred', 'writer did not defer import')
            label = rename_local_profile(state)
            writer = Pane(sock, state, args.shipped, PROFILE, None, password, work, 'writer', args.cache)
            print('Configure the writer as described in README.md. Detach without quitting.', flush=True)
            subprocess.run([*writer.tmux, 'attach-session', '-t', 'writer'], check=True, timeout=1800)
            writer.probe('profile', json.dumps(RAW))
            writer.cards()
            writer.quit()
            hint = check_writer(state, PROFILE, args.account, since)
            # Import and defer are mutually exclusive. Exercise the real pre-import HOME on a copy.
            deferred_dir = work/'deferred'
            deferred_dir.mkdir()
            deferred = fresh_home(deferred_dir)
            copy_stores(state, deferred)
            before = Pane(sock, deferred, args.candidate, PROFILE, args.shared_account, password,
                          work, 'deferred', args.cache)
            before.script_pane()
            before.key('b')
            before.until(lambda s: '[Import catalog…]' in s, 'catalog defer was lost')
            before.key('Escape')
            before.until(lambda s: 'script — browse' not in s, 'deferred picker did not close')
            before.probe('shared', 'undefined')
            before.quit()
            require(not any('.pre-0.' in p.name for p in deferred.rglob('*')), 'candidate moved deferred data aside')
            print(f'In the shipped panel import catalog {CATALOG}, then quit.', flush=True)
            panel(PROFILE)
            require((state/'rs2b0t-path').read_text().strip() == str(CATALOG)
                    and not (state/'rs2b0t-import').exists(), 'writer did not import the fixture catalog')
            reader = Pane(sock, state, args.candidate, PROFILE, args.account, password, work, 'candidate', args.cache)
            reader.until(lambda s: label in s, 'custom server name and endpoint not restored')
            reader.logged(hint)
            reader.saved_ui('Lobster:7')
            reader.probe('profile', json.dumps(RAW))
            reader.probe('default', json.dumps(RAW), catalog=True)
            reader.cards()
            reader.quit()
            require(not any('.pre-0.' in p.name for p in state.rglob('*')), 'candidate moved data aside')
            print(f'writer_sha256={shipped}\ncandidate_sha256={candidate}')
        finally:
            close_server(sock)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest='mode', required=True)
    historical = modes.add_parser('control', help='offline historical startup and vault check')
    for arg in ('binary', 'snapshot', 'pass-file', 'crc-file'):
        historical.add_argument('--'+arg, type=Path, required=True)
    historical.add_argument('--account', required=True)
    gate = modes.add_parser('live', help='operator-assisted release upgrade')
    for arg in ('shipped', 'shipped-panel', 'candidate'):
        gate.add_argument('--'+arg, type=Path, required=True)
    gate.add_argument('--shipped-sha256', required=True)
    gate.add_argument('--panel-sha256', required=True)
    gate.add_argument('--shared-account', required=True)
    gate.add_argument('--account', required=True)
    for mode in (historical, gate):
        mode.add_argument('--cache', required=True, type=Path, help='local revision-289 client cache')
        mode.add_argument('--work', required=True, type=Path, help='new private directory')
    args = parser.parse_args()
    os.umask(0o077)
    try:
        work = args.work.resolve()
        work.mkdir(mode=0o700)
        (control if args.mode == 'control' else live)(args, work)
    except Refused as error:
        print(f'{args.mode}: FAIL: {error}', file=sys.stderr)
        return 1
    except (Failure, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'{args.mode}: FAIL: {error}', file=sys.stderr)
        return 2
    print(f'{args.mode}: PASS')
    return 0


if __name__ == '__main__':
    sys.exit(main())
