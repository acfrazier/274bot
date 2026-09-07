#!/usr/bin/env python3
"""Run one diagnostic cell without overwriting the T4 baseline artifacts."""
import argparse, hashlib, json, os, pathlib, signal, subprocess, sys, time
import errno, struct, threading
from build_provenance import file_sha256, verify_build, recheck_files
from operator_home import bot_home_path

# Historical Mac checkout default for rs2b0t provenance. Never applied on win32.
_DEFAULT_RS2B0T_MAC = '/Users/acfrazier/experiments/rs2b0t'

_WINDOWS_TUI_TERMINAL_UNSUPPORTED = (
    'real TUI terminal diagnostic requires Windows ConPTY transport; '
    'API unavailable. Use panel or tui --headless for non-terminal launch. '
    'Do not treat a missing ConPTY as silent headless success.'
)

_UNIX_TTY_IMPORT_REQUIRED = (
    'real TUI terminal diagnostic requires Unix pty/fcntl/termios; '
    'modules unavailable. Use panel or tui --headless, or the Windows '
    'ConPTY path on win32.'
)

_TERMINAL_PROBE_ENDPOINT = (
    'terminal write only; use native input/draw counters for latency'
)


def build_parser():
    p = argparse.ArgumentParser()
    p.add_argument('frontend', choices=['panel','tui'])
    p.add_argument('n', type=int, choices=[1,16,32,128])
    p.add_argument('workload', choices=['idle','seeded-idle','active','lifecycle'])
    p.add_argument('--nav-captures', action='store_true', help='Diagnostic only: navigation checkpoints/failure. Panel: requires --single-renderer or --focused-one (GPU focus). TUI: data-only JSON evidence under captures/ (no renderer select / screenshots)')
    p.add_argument('--single-renderer', action='store_true', help='Panel legacy: fixed slot zero draws; other slots simulate only (BOT_MEMORY_SINGLE_RENDERER)')
    p.add_argument('--focused-one', action='store_true', help='Panel: fixed slot0 full-rate GPU; others simulation-only (deterministic prefs)')
    p.add_argument('--focused-background', action='store_true', help='Panel: fixed slot0 full-rate; other slots draw at 1 fps skip-paint')
    p.add_argument('--headless', action='store_true', help='TUI diagnostic only: skip terminal drawing')
    p.add_argument('--tui-input-probes', action='store_true', help='TUI PTY: toggle settings overlay once per second during observation; writes alone are not latency evidence')
    p.add_argument('--debug', action='store_true')
    p.add_argument('--no-diagnostics', action='store_true', help='Disable verbose diagnostics; retain boundary qualification')
    p.add_argument('--failure-capture', action='store_true', help='Set BOT_MEMORY_FAILURE_CAPTURE=1 (failure-boundary stop-reason). Independent of diagnostics; pair with --no-diagnostics for failure-only mode (no periodic sidecar)')
    p.add_argument('--binary', type=pathlib.Path, help='Use an immutable saved frontend build')
    p.add_argument('--build-manifest', type=pathlib.Path, help='Verify saved binary and runtime fixture hashes against this frozen-build manifest')
    p.add_argument('--build-role', choices=['reference', 'candidate'], help='Required with --build-manifest; selects the named frozen binary')
    p.add_argument('--sustain', action='store_true')
    stack_logging = p.add_mutually_exclusive_group()
    stack_logging.add_argument('--stack-logging', action='store_true')
    stack_logging.add_argument('--stack-logging-lite', action='store_true', help='Native diagnostic: retain only current allocation stacks')
    p.add_argument('--scheduling-profile', action='store_true', help='Collect batched active-loop work/sleep/interval diagnostics')
    p.add_argument('--render-profile', action='store_true', help='Collect per-slot renderer residency and host paint cadence')
    p.add_argument('--gpu-completion-profile', action='store_true', help='Panel+render-profile: bounded GPU queue.on_submitted_work_done completion samples')
    p.add_argument('--responsiveness-profile', action='store_true', help='Collect decode-to-script and input-to-UI endpoint latencies')
    p.add_argument('--responsiveness-fine', action='store_true', help='With --responsiveness-profile: also collect <=1ms fine latency histograms (recorded in metadata)')
    p.add_argument('--observe', type=int, default=600)
    p.add_argument('--warmup', type=int, default=120)
    return p

def validate_args(a, parser):
    """Reject invalid flag combinations before any process start or run dir."""
    if bool(a.build_manifest) != bool(a.build_role):
        parser.error('--build-manifest and --build-role must be supplied together')
    if a.build_manifest and not a.binary:
        parser.error('--build-manifest requires explicit --binary')
    if a.tui_input_probes and (a.frontend != 'tui' or a.headless):
        parser.error('--tui-input-probes requires the real TUI terminal')
    panel_modes = [a.single_renderer, a.focused_one, a.focused_background]
    if sum(bool(x) for x in panel_modes) > 1:
        parser.error('--single-renderer, --focused-one, and --focused-background are mutually exclusive')
    if a.frontend != 'panel' and any(panel_modes):
        parser.error('panel render flags require frontend panel')
    one_draw = a.single_renderer or a.focused_one
    if a.nav_captures and a.frontend == 'panel' and not one_draw:
        parser.error('--nav-captures on panel requires --single-renderer or --focused-one')
    # TUI --nav-captures is allowed as data-only diagnostic mode (no panel draw flags).
    if a.gpu_completion_profile:
        if a.frontend != 'panel':
            parser.error('--gpu-completion-profile requires frontend panel')
        if not a.render_profile:
            parser.error('--gpu-completion-profile requires --render-profile')
    if a.responsiveness_fine and not a.responsiveness_profile:
        parser.error('--responsiveness-fine requires --responsiveness-profile')

def requested_render_policy(a):
    if a.frontend != 'panel':
        return 'none'
    if a.single_renderer:
        return 'fixed-one'
    if a.focused_one:
        return 'focused-one'
    if a.focused_background:
        return 'focused-plus-background'
    return 'rotating-all'

# Keys always scrubbed from the child env so parent shell pollution cannot leak.
_SCRUB_CHILD_ENV = (
    'BOT_CPU', 'BOT_LIVE', 'BOT_DEBUG', 'MallocStackLogging', 'MallocStackLoggingNoCompact',
    'BOT_MEMORY_SUSTAIN', 'BOT_MEMORY_SINGLE_RENDERER', 'BOT_MEMORY_RENDER_POLICY',
    'BOT_MEMORY_FAILURE_CAPTURE', 'BOT_NAV_CAPTURES', 'BOT_SCHEDULING_PROFILE',
    'BOT_RENDER_PROFILE', 'BOT_GPU_COMPLETION_PROFILE', 'BOT_RESPONSIVENESS_PROFILE',
    'BOT_RESPONSIVENESS_FINE',
)

def apply_rs2b0t_default(env, *, platform=None):
    """Preserve historical Mac RS2B0T default off Windows only.

    On win32 never inject the Mac path; callers must set a real checkout path
    for rs2b0t provenance (no guessed SHA).
    """
    plat = sys.platform if platform is None else platform
    if plat != 'win32':
        env.setdefault('RS2B0T', _DEFAULT_RS2B0T_MAC)
    return env

def resolve_rs2b0t_commit(env, *, platform=None, git_check_output=None):
    """Return real git HEAD under env['RS2B0T'] or raise ValueError.

    Never invents a commit SHA. Windows requires an explicit non-empty RS2B0T.
    """
    plat = sys.platform if platform is None else platform
    run_git = git_check_output or (
        lambda args: subprocess.check_output(args, text=True).strip()
    )
    rs2 = env.get('RS2B0T')
    if not isinstance(rs2, str) or not rs2.strip():
        if plat == 'win32':
            raise ValueError(
                'RS2B0T must be set to an explicit checkout path on Windows '
                '(Mac default path is not applied; no fabricated commit SHA)'
            )
        raise ValueError('RS2B0T is unset or empty; cannot resolve rs2b0t_commit')
    path = pathlib.Path(rs2)
    if not path.is_dir():
        raise ValueError(f'RS2B0T path does not exist or is not a directory: {rs2}')
    try:
        commit = run_git(['git', '-C', str(path), 'rev-parse', 'HEAD'])
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError(f'RS2B0T git rev-parse failed under {rs2}: {error}') from error
    if not isinstance(commit, str) or not commit.strip():
        raise ValueError(f'RS2B0T git rev-parse returned empty under {rs2}')
    return commit.strip()

def build_child_env(a, run_dir, base_env=None, *, platform=None):
    """Construct the host-play child environment from CLI args.

    Shared by main and unit tests so scrub/set regressions are caught against
    the real launcher path (tests must not reimplement pop/set).
    `run_dir` may be str or pathlib.Path; samples.jsonl is placed under it.
    """
    run = pathlib.Path(run_dir)
    env = dict(base_env) if base_env is not None else os.environ.copy()
    for k in _SCRUB_CHILD_ENV:
        env.pop(k, None)
    env.update(
        LIVE='1',
        BOT_TARGET='local',
        BOT_MEMORY_N=str(a.n),
        BOT_MEMORY_WORKLOAD=a.workload,
        BOT_MEMORY_OUTPUT=str(run / 'samples.jsonl'),
        BOT_MEMORY_DIAGNOSTICS='0' if a.no_diagnostics else '1',
        BOT_MEMORY_WARMUP_S=str(a.warmup),
        BOT_MEMORY_OBSERVE_S=str(a.observe),
    )
    apply_rs2b0t_default(env, platform=platform)
    # Scrub inherited failure-capture above; set only when the CLI flag is on.
    if a.failure_capture:
        env['BOT_MEMORY_FAILURE_CAPTURE'] = '1'
    if a.debug:
        env['BOT_DEBUG'] = '1'
    if a.nav_captures:
        env['BOT_NAV_CAPTURES'] = '1'
        env['274BOT_SMOKE_DIR'] = str(run / 'captures')
    if a.single_renderer:
        env['BOT_MEMORY_SINGLE_RENDERER'] = '1'
    elif a.focused_one:
        env['BOT_MEMORY_RENDER_POLICY'] = 'focused-one'
    elif a.focused_background:
        env['BOT_MEMORY_RENDER_POLICY'] = 'focused-plus-background'
    if a.sustain:
        env['BOT_MEMORY_SUSTAIN'] = '1'
    if a.stack_logging:
        env['MallocStackLogging'] = '1'
    if a.stack_logging_lite:
        env['MallocStackLogging'] = 'lite'
    if a.scheduling_profile:
        env['BOT_SCHEDULING_PROFILE'] = '1'
    if a.render_profile:
        env['BOT_RENDER_PROFILE'] = '1'
    if a.gpu_completion_profile:
        env['BOT_GPU_COMPLETION_PROFILE'] = '1'
    if a.responsiveness_profile:
        env['BOT_RESPONSIVENESS_PROFILE'] = '1'
    if a.responsiveness_fine:
        env['BOT_RESPONSIVENESS_FINE'] = '1'
    return env

def catalog_path_for_env(env, *, windows=None):
    """js-scripts catalog under operator home (HOME/USERPROFILE parity)."""
    base = bot_home_path(environ=env, windows=windows)
    return (pathlib.Path(base) / '.274bot/js-scripts.json').resolve()

def require_terminal_transport(*, platform=None):
    """Lazy-load real terminal transport; fail closed (never silent headless).

    Returns a tag tuple:
      ``('unix', fcntl, pty, termios)`` on non-Windows
      ``('conpty', windows_conpty_module)`` on win32 when ConPTY helper loads

    Does not silently fall through to headless.
    """
    plat = sys.platform if platform is None else platform
    if plat == 'win32':
        try:
            import windows_conpty as wcp
            wcp.require_conpty_support(platform=plat)
        except Exception as error:
            raise RuntimeError(f'{_WINDOWS_TUI_TERMINAL_UNSUPPORTED} ({error})') from error
        return 'conpty', wcp
    try:
        import fcntl
        import pty
        import termios
    except ImportError as error:
        raise RuntimeError(f'{_UNIX_TTY_IMPORT_REQUIRED} ({error})') from error
    return 'unix', fcntl, pty, termios

def main(argv=None):
    p = build_parser()
    a = p.parse_args(argv)
    validate_args(a, p)
    terminal = a.frontend == 'tui' and not a.headless
    root = pathlib.Path(__file__).resolve().parents[2]
    binary = a.binary.resolve() if a.binary else root / 'target/release' / (a.frontend+'-play')
    run = root / 'docs/memory/diagnostics' / (time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'_'+a.frontend+f'_n{a.n}_{a.workload}')
    env = build_child_env(a, run)
    def git(*args):
        return subprocess.check_output(['git',*args],cwd=root,text=True).strip()
    def source_digest(directory):
        files = subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=directory).split(b'\0')
        digest = hashlib.sha256()
        for name in sorted(set(files)):
            if not name: continue
            relative = pathlib.Path(os.fsdecode(name))
            if not (relative.parts[0] == 'crates' or relative.name in ('Cargo.toml','Cargo.lock')): continue
            path = directory / relative
            if path.is_file(): digest.update(name+b'\0'+path.read_bytes()+b'\0')
        return digest.hexdigest()
    nav_pack = pathlib.Path(env.get('NAV_PACK', str(pathlib.Path.home()/'.274bot/274bot.navpack'))).resolve()
    nav_flags = pathlib.Path(env.get('NAV_FLAGS', str(nav_pack.with_suffix('.navflags')))).resolve()
    catalog_path = catalog_path_for_env(env)
    provenance = {'status': 'unavailable', 'reason': 'no_build_manifest', 'performance_acceptance': False}
    if a.build_manifest:
        try:
            provenance = verify_build(a.build_manifest, a.build_role, a.frontend, binary, nav_pack, nav_flags, catalog_path)
        except (ValueError, OSError, TypeError) as error:
            p.error(f'build provenance: {error}')
    try:
        rs2b0t_commit = resolve_rs2b0t_commit(env)
    except ValueError as error:
        p.error(f'rs2b0t provenance: {error}')
    if terminal:
        try:
            require_terminal_transport()
        except RuntimeError as error:
            p.error(str(error))
    run.mkdir(parents=True, exist_ok=False)
    render_policy = requested_render_policy(a)
    meta = dict(stack_logging_mode='lite' if a.stack_logging_lite else ('1' if a.stack_logging else None),host_sources_sha256=source_digest(root),client_sources_sha256=source_digest(root/'vendor/fr-client-rust'),frontend=a.frontend,n=a.n,workload=a.workload,warmup_s=a.warmup,observe_s=a.observe,
                nav_pack=str(nav_pack),nav_pack_sha256=hashlib.sha256(nav_pack.read_bytes()).hexdigest() if nav_pack.is_file() else None,nav_flags=str(nav_flags),
                diagnostic_only=True,scheduling_profile=a.scheduling_profile,render_profile=a.render_profile,gpu_completion_profile=a.gpu_completion_profile,responsiveness_profile=a.responsiveness_profile,responsiveness_fine=a.responsiveness_fine,diagnostic_sidecar=not a.no_diagnostics,failure_capture=a.failure_capture,nav_captures=a.nav_captures,single_renderer=a.single_renderer,render_policy=render_policy,render_policy_requested=True,terminal=terminal,terminal_size=[120,40] if terminal else None,debug=a.debug,sustain=a.sustain,stack_logging=a.stack_logging or a.stack_logging_lite,binary=str(binary),
                binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                host_commit=git('rev-parse','HEAD'),client_commit=git('-C','vendor/fr-client-rust','rev-parse','HEAD'),
                host_diff_sha256=hashlib.sha256(git('diff','HEAD').encode()).hexdigest(),
                rs2b0t_commit=rs2b0t_commit,
                run_dir=str(run),started_unix=time.time())
    # Legacy source/commit fields above describe this checkout, not the saved
    # binary. Build claims stay in an independently verified nested object.
    meta.update(build_provenance=provenance,
                tui_input_probes=a.tui_input_probes,
                checkout_source_labels_only=True,
                nav_flags_sha256=file_sha256(nav_flags) if nav_flags.is_file() else None,
                catalog_path=str(catalog_path),
                catalog_sha256=file_sha256(catalog_path) if catalog_path.is_file() else None,
                effective_cli=[sys.executable, str(pathlib.Path(__file__).resolve()), *(sys.argv[1:] if argv is None else [str(x) for x in argv])],
                feature_flags=provenance.get('feature_flags'),
                allocator_provenance=provenance.get('allocator_provenance'),
                allocation_counting=provenance.get('allocation_counting'))
    with (run/'run.log').open('xb') as log:
        reader = None
        probe = None
        conpty_session = None
        if terminal:
            transport = require_terminal_transport()
            env['TERM'] = 'xterm-256color'
            if transport[0] == 'conpty':
                wcp = transport[1]
                # Real 120x40 pseudoconsole — not headless. Child is a direct
                # CreateProcess child of this launcher (explicit-parent topology).
                conpty_session = wcp.spawn(
                    [str(binary)],
                    cwd=str(root),
                    env={str(k): str(v) for k, v in env.items()},
                    cols=120,
                    rows=40,
                )
                child = conpty_session
                meta['terminal_transport'] = 'conpty'
                if a.tui_input_probes:
                    from tui_input_probe import InputProbe
                    probe = InputProbe(conpty_session.input_writer(), run)
                def drain_terminal():
                    try:
                        while True:
                            data = conpty_session.read(65536)
                            if not data:
                                break
                            log.write(data)
                            log.flush()
                    finally:
                        pass
                reader = threading.Thread(target=drain_terminal, name='conpty-drain')
                reader.start()
            else:
                _tag, fcntl, pty, termios = transport
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
                def terminal_session():
                    os.setsid()
                    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
                child = subprocess.Popen(
                    [str(binary)],
                    cwd=root,
                    env=env,
                    stdin=slave,
                    stdout=slave,
                    stderr=slave,
                    preexec_fn=terminal_session,
                )
                os.close(slave)
                meta['terminal_transport'] = 'unix-pty'
                if a.tui_input_probes:
                    from tui_input_probe import InputProbe
                    probe = InputProbe(master, run)
                def drain_terminal():
                    try:
                        while True:
                            try:
                                data = os.read(master, 65536)
                            except OSError as error:
                                if error.errno == errno.EIO:
                                    break
                                raise
                            if not data:
                                break
                            log.write(data)
                    finally:
                        os.close(master)
                reader = threading.Thread(target=drain_terminal, name='unix-pty-drain')
                reader.start()
        else:
            child = subprocess.Popen([str(binary)], cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
        meta['pid'] = child.pid
        (run / 'metadata.json').write_text(json.dumps(meta, indent=2) + '\n')
        print(json.dumps(meta), flush=True)
        if probe:
            probe.start()
        def stop(sig, frame):
            child.terminate()
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        rc = child.wait()
        if probe:
            probe.close()
        # ClosePseudoConsole after wait so drain observes EOF (avoids deadlock).
        if conpty_session is not None:
            conpty_session.close_pseudoconsole()
        if reader:
            reader.join()
        if conpty_session is not None:
            conpty_session.close()
    meta.update(exit_code=rc, ended_unix=time.time())
    if probe:
        probe_path = run / 'input-probes.jsonl'
        meta['input_probe_result'] = dict(
            sent=probe.sent,
            error=probe.error,
            path=str(probe_path),
            sha256=file_sha256(probe_path) if probe_path.is_file() else None,
            endpoint=_TERMINAL_PROBE_ENDPOINT,
        )
    provenance_error = None
    if provenance['status'] == 'verified':
        try:
            recheck_files(provenance['files'])
            provenance['completion_status'] = 'unchanged'
        except (ValueError, OSError) as error:
            provenance_error = str(error)
            provenance.update(status='invalid', completion_status='changed_or_unreadable', error=provenance_error)
    (run/'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
    print(json.dumps({'run_dir':str(run),'exit_code':rc}),flush=True)
    if provenance_error:
        print(f'FAIL: build provenance: {provenance_error}', file=sys.stderr)
        sys.exit(1)
    if probe and probe.error:
        print(f'FAIL: input probe: {probe.error}', file=sys.stderr)
        sys.exit(1)
    sys.exit(rc if rc >= 0 else 128-rc)

if __name__ == '__main__':
    main()
