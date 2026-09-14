//! Command construction and owned child execution.
//!
//! The suite launches the existing native executables (`panel` `catalog_watch` for core
//! witnesses, `panel` `pair_watch` for pair witnesses) with an explicit, validated
//! native profile/input configuration. It never builds its own scenario engine and never
//! runs a foreign runtime.
//!
//! Ownership: the child is started in its own process group and wrapped in an RAII guard,
//! so every exit path — success, `?` error, panic — terminates and reaps the whole tree.
//! Cleanup is bounded at each step (`SIGTERM`, a grace, `SIGKILL`), the wait is
//! deadline-aware, and the output pipes are drained with a deadline rather than an
//! unbounded `join`: a descendant that inherits the pipes after the direct child exits
//! cannot hang the suite. A tree that cannot be reaped inside the bound is reported as a
//! harness failure instead of being ignored.
//!
//! Platform: bounded process-group ownership uses unix process groups and signals. On
//! other platforms [`run`] fails closed instead of launching a child it cannot own.

use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::identity::BinaryIdentity;
use super::manifest::{CaseEntry, RunnerKind, SuiteManifest};
use super::SuiteResult;

/// Grace between a graceful termination and a forced kill (matches the native Stop
/// contract's 10s bound).
pub const CLEANUP_GRACE: Duration = Duration::from_secs(10);
/// How long a forced kill is given to be reaped before cleanup is reported failed.
pub const CLEANUP_KILL_WAIT: Duration = Duration::from_secs(5);
/// Bound on the per-case log file kept in the run directory.
pub const LOG_CAP_BYTES: u64 = 64 * 1024 * 1024;
/// Bound on the in-memory tail used for receipt parsing.
pub const TAIL_CAP_BYTES: usize = 1024 * 1024;
/// Bound on one output line held while looking for its newline. A longer line is
/// emitted wrapped (with a marker) instead of being buffered without limit.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Deadline for draining the child's pipes after the direct child has exited. A
/// descendant can inherit the pipe write ends, so this is a bound, not a join.
pub const PIPE_DRAIN: Duration = Duration::from_secs(5);
/// Additional drain bound granted after the leftover process group is force-killed.
pub const PIPE_DRAIN_AFTER_KILL: Duration = Duration::from_secs(2);
/// Poll interval for the wait loop.
const POLL: Duration = Duration::from_millis(50);

/// The panel's mainland knob: `BOT_MAINLAND=1`, the same variable the `host-play` binary's
/// `--mainland` sets. `catalog_watch`/`pair_watch` expose no such flag, so the suite asks
/// for it in the child's environment.
pub const MAINLAND_ENV: &str = "BOT_MAINLAND";

/// Set by the signal handler; checked by the wait loop so an interactive Ctrl-C still
/// reaps the child tree before the suite exits.
static INTERRUPT: AtomicBool = AtomicBool::new(false);

pub fn interrupt_flag() -> &'static AtomicBool {
    &INTERRUPT
}

pub fn interrupt_requested() -> bool {
    INTERRUPT.load(Ordering::SeqCst)
}

/// Install the SIGINT/SIGTERM flag on unix. The handler only stores a flag.
#[cfg(unix)]
pub fn install_signal_handler() {
    extern "C" fn handle(_signal: libc::c_int) {
        INTERRUPT.store(true, Ordering::SeqCst);
    }
    unsafe {
        libc::signal(libc::SIGINT, handle as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle as *const () as libc::sighandler_t);
    }
}

/// On non-unix platforms the suite installs no signal handler and [`run`] fails closed:
/// without a console-control handler an interrupt would leave the child running.
#[cfg(not(unix))]
pub fn install_signal_handler() {}

/// The explicit native profile/input configuration for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeConfig {
    pub profile: String,
    pub revision: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub engine: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub catalog: PathBuf,
    pub vault: Option<PathBuf>,
    pub lowmem: bool,
    pub mainland: bool,
    /// Direct native executables. When absent the manifest's cargo template is resolved to
    /// a built artifact and hashed (see [`NativeConfig::binary`]).
    pub exec_core: Option<PathBuf>,
    pub exec_pair: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    /// Extra arguments appended after the case's own arguments (operator overrides such
    /// as a debug flag). Preserved verbatim in the ledger's requested operation.
    pub extra_args: Vec<String>,
}

impl NativeConfig {
    /// Flags handed to the native executable, in the shared `host_play::parse_profile_args`
    /// shape. Only flags that parser consumes are emitted: `catalog_watch`/`pair_watch`
    /// reject an unknown flag with exit 2, so a front-end flag such as `--lowmem` would
    /// break every launch. Working directory, memory mode and mainland are not flags of
    /// those executables (see [`NativeConfig::child_env`] and [`NativeConfig::validate`]).
    pub fn profile_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        args.push("--profile".into());
        args.push(self.profile.clone());
        let mut push = |flag: &str, value: Option<String>| {
            if let Some(value) = value {
                args.push(flag.to_string());
                args.push(value);
            }
        };
        push("--revision", self.revision.clone());
        push("--host", self.host.clone());
        push("--port", self.port.map(|port| port.to_string()));
        push(
            "--engine",
            self.engine.as_ref().map(|p| p.display().to_string()),
        );
        push(
            "--cache",
            self.cache.as_ref().map(|p| p.display().to_string()),
        );
        push("--catalog", Some(self.catalog.display().to_string()));
        push(
            "--vault",
            self.vault.as_ref().map(|p| p.display().to_string()),
        );
        args
    }

    /// Validate the configuration before any launch. Explicit paths must exist.
    pub fn validate(&self) -> SuiteResult<()> {
        if self.profile.trim().is_empty() {
            return Err("--profile must name the native server profile".into());
        }
        if !self.lowmem {
            // `panel-play`/`catalog_watch`/`pair_watch` take no memory flag: the panel
            // applies the memory mode from the selected vault profile's settings. Asking
            // for highmem would silently run the profile's own mode, so refuse instead of
            // recording a request the adapters cannot execute.
            return Err("--highmem is not selectable by the current native adapters: the panel reads the \
                        memory mode from the vault profile and exposes no flag (pending adapter work). \
                        Leave `--lowmem` (the profile default) or select a highmem profile in the vault"
                .into());
        }
        if !self.catalog.is_dir() {
            return Err(format!(
                "--catalog {} is not a directory (the native catalog clone root)",
                self.catalog.display()
            ));
        }
        for (flag, path) in [
            ("--exec-core", self.exec_core.as_ref()),
            ("--exec-pair", self.exec_pair.as_ref()),
        ] {
            if let Some(path) = path {
                if !path.is_file() {
                    return Err(format!("{flag} {} is not a file", path.display()));
                }
            }
        }
        for (flag, path) in [
            ("--engine", self.engine.as_ref()),
            ("--cache", self.cache.as_ref()),
            ("--vault", self.vault.as_ref()),
        ] {
            if let Some(path) = path {
                if !path.exists() {
                    return Err(format!("{flag} {} does not exist", path.display()));
                }
            }
        }
        if let Some(cwd) = &self.cwd {
            if !cwd.is_dir() {
                return Err(format!("--cwd {} is not a directory", cwd.display()));
            }
        }
        Ok(())
    }

    /// The executable identity recorded for a runner kind.
    ///
    /// A direct executable is hashed. Without one the manifest's cargo template is
    /// *resolved* to the artifact a build produced (`target/{debug,release}[/examples]/
    /// <bin>`), which is hashed too: `cargo run` at evaluation time could rebuild different
    /// bytes under the same command, so an unresolved command string is never accepted as
    /// a runnable identity. The executor is built once before the ledger is written.
    pub fn binary(
        &self,
        runner: RunnerKind,
        manifest: &SuiteManifest,
        repo_root: Option<&Path>,
    ) -> SuiteResult<BinaryIdentity> {
        let (direct, template, flag) = match runner {
            RunnerKind::Core => (
                self.exec_core.as_ref(),
                &manifest.defaults.exec.core,
                "--exec-core",
            ),
            RunnerKind::Pair => (
                self.exec_pair.as_ref(),
                &manifest.defaults.exec.pair,
                "--exec-pair",
            ),
        };
        match direct {
            Some(path) => BinaryIdentity::direct(path),
            None => BinaryIdentity::resolve_template(template, repo_root, flag),
        }
    }

    /// The full command line for one case.
    pub fn command(&self, case: &CaseEntry, manifest: &SuiteManifest) -> SuiteResult<Vec<String>> {
        let live = case
            .live
            .as_deref()
            .ok_or_else(|| format!("{}: no native live name", case.id))?;
        let runner = case.runner();
        let direct = match runner {
            RunnerKind::Core => self.exec_core.as_ref(),
            RunnerKind::Pair => self.exec_pair.as_ref(),
        };
        let mut command = Vec::new();
        match direct {
            Some(path) => command.push(path.display().to_string()),
            None => {
                let template = match runner {
                    RunnerKind::Core => &manifest.defaults.exec.core,
                    RunnerKind::Pair => &manifest.defaults.exec.pair,
                };
                command.push(template.program.clone());
                command.extend(template.args.iter().cloned());
            }
        }
        command.extend(self.profile_args());
        command.push("--live".into());
        command.push(live.to_string());
        command.extend(
            case.reference_args
                .iter()
                .filter(|arg| supported_arg(arg))
                .cloned(),
        );
        command.extend(self.extra_args.iter().cloned());
        Ok(command)
    }

    /// Environment for one case: the suite's shot root so captures land inside the run
    /// directory, plus the typed case environment. Raw reference `env` is metadata only
    /// and is never applied wholesale.
    ///
    /// Mainland is not a flag of the panel executables; the panel reads it from
    /// `BOT_MAINLAND` (the same knob the `host-play` binary's `--mainland` sets), so the
    /// suite asks for it in the child's environment instead.
    pub fn child_env(
        &self,
        case: &CaseEntry,
        shots_root: &Path,
        base: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut env: BTreeMap<String, String> = base.clone();
        env.insert(
            scenario::shot::SHOT_ROOT_ENV.to_string(),
            shots_root.display().to_string(),
        );
        if self.mainland {
            env.insert(MAINLAND_ENV.to_string(), "1".to_string());
        }
        for (key, value) in &case.reference_env {
            if typed_env_key(key) {
                env.insert(key.clone(), value.clone());
            }
        }
        env
    }

    /// The environment *names* this configuration hands to a child. Recorded in the run
    /// identity so a changed environment request refuses resume.
    pub fn env_keys(&self) -> Vec<String> {
        let mut keys = vec![scenario::shot::SHOT_ROOT_ENV.to_string()];
        if self.mainland {
            keys.push(MAINLAND_ENV.to_string());
        }
        keys
    }
}

/// Typed per-case arguments the native adapter supports. The reference `args` are
/// preserved as metadata; only explicitly supported flags are forwarded.
fn supported_arg(_arg: &str) -> bool {
    // No reference harness flag maps to a native executable flag yet: `--no-deploy`,
    // `--minutes`, `--base` and friends belong to the foreign runner. Declaring one here
    // without a native adapter would be inventing support.
    false
}

/// Reference `env` entries are applied only when they name a native host knob the suite
/// deliberately supports.
fn typed_env_key(_key: &str) -> bool {
    false
}

/// A prepared child invocation.
#[derive(Debug, Clone)]
pub struct ChildSpec {
    pub label: String,
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanupSummary {
    pub killed_signal: Option<i32>,
    pub escalated_to_sigkill: bool,
    pub reaped: bool,
    pub note: String,
}

/// The bounded result of one child run.
#[derive(Debug, Clone)]
pub struct ChildRun {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub interrupted: bool,
    /// Bounded tail of the child's combined output, used for receipt parsing.
    pub output_tail: String,
    pub log_truncated: bool,
    pub elapsed_ms: u64,
    pub cleanup: CleanupSummary,
}

/// Launch one child, own its process tree, and return its bounded result.
#[cfg(unix)]
pub fn run(
    spec: &ChildSpec,
    budget: Duration,
    log_path: &Path,
    verbose: bool,
) -> SuiteResult<ChildRun> {
    if spec.command.is_empty() {
        return Err(format!("{}: empty command", spec.label));
    }
    // Fallible setup happens *before* the spawn: a log that cannot be opened must not
    // leave a launched child behind.
    let log = Arc::new(Mutex::new(LogWriter::open(log_path)?));
    let shared = Arc::new(SharedOutput::default());

    let mut command = Command::new(&spec.command[0]);
    command.args(&spec.command[1..]);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env_clear();
    // `env_clear` would hide PATH; keep the operator's environment and add the suite's.
    for (key, value) in std::env::vars() {
        command.env(key, value);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    own_process_group(&mut command);

    let started = Instant::now();
    let child = command
        .spawn()
        .map_err(|error| format!("{}: cannot launch {}: {error}", spec.label, spec.command[0]))?;
    let mut owned = OwnedChild::new(child, spec.label.clone());

    let mut readers = Vec::new();
    if let Some(stdout) = owned.child.stdout.take() {
        readers.push(spawn_reader(
            stdout,
            Arc::clone(&log),
            Arc::clone(&shared),
            verbose,
            spec.label.clone(),
        ));
    }
    if let Some(stderr) = owned.child.stderr.take() {
        readers.push(spawn_reader(
            stderr,
            Arc::clone(&log),
            Arc::clone(&shared),
            verbose,
            spec.label.clone(),
        ));
    }

    let deadline = started + budget;
    let mut timed_out = false;
    let mut interrupted = false;
    let mut cleanup = CleanupSummary::default();
    let mut status: Option<ExitStatus> = None;
    loop {
        match owned.try_wait() {
            Ok(Some(exit)) => {
                owned.mark_reaped();
                status = Some(exit);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                // The RAII guard reaps the tree on this return path.
                return Err(format!("{}: wait failed: {error}", spec.label));
            }
        }
        if interrupt_requested() {
            interrupted = true;
            cleanup = owned.terminate();
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            cleanup = owned.terminate();
            break;
        }
        std::thread::sleep(POLL);
    }
    let pid = owned.pid;
    let status = owned.status.or(status);
    drop(owned);

    // Deadline-aware drain. A descendant that inherited stdout/stderr keeps the pipe open
    // after the direct child exits, so this waits a bound, then force-kills whatever is
    // left in the group to close the write ends, then gives up and detaches: an unbounded
    // join is never taken.
    let drained = drain_readers(&shared, readers.len(), PIPE_DRAIN);
    if !drained {
        let leftover = signal_tree(pid, force_signal());
        let note = if leftover {
            format!(
                "{}: output pipes were still open after {:?} (a descendant held them); \
                 the leftover process group was force-killed",
                spec.label, PIPE_DRAIN
            )
        } else {
            format!(
                "{}: output pipes were still open after {:?}",
                spec.label, PIPE_DRAIN
            )
        };
        let _ = drain_readers(&shared, readers.len(), PIPE_DRAIN_AFTER_KILL);
        if cleanup.note.is_empty() {
            cleanup.note = note;
        } else {
            cleanup.note = format!("{}; {note}", cleanup.note);
        }
    }
    let output_tail = shared.tail.lock().unwrap().text();
    let log_truncated = log.lock().map(|l| l.truncated()).unwrap_or(false)
        || shared.wrapped_lines.load(Ordering::SeqCst) > 0;
    if let Ok(mut writer) = log.try_lock() {
        let _ = writer.flush();
    }

    let (exit_code, signal) = match status {
        Some(status) => classify_exit(status),
        None => (None, None),
    };
    if cleanup.note.is_empty() && !timed_out && !interrupted {
        cleanup.note = "child exited on its own".into();
    }
    Ok(ChildRun {
        exit_code,
        signal,
        timed_out,
        interrupted,
        output_tail,
        log_truncated,
        elapsed_ms: started.elapsed().as_millis() as u64,
        cleanup,
    })
}

/// Non-unix platforms have no process-group signals here, so the suite refuses to launch
/// a child rather than claim ownership it cannot enforce.
#[cfg(not(unix))]
pub fn run(
    spec: &ChildSpec,
    _budget: Duration,
    _log_path: &Path,
    _verbose: bool,
) -> SuiteResult<ChildRun> {
    Err(format!(
        "{}: owned child execution needs unix process groups and signals; this platform is \
         fail-closed (no bounded process-tree ownership), so the suite refuses to launch",
        spec.label
    ))
}

/// Wait for the reader threads to reach EOF, bounded. Returns whether they all finished.
fn drain_readers(shared: &SharedOutput, count: usize, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        if shared.readers_done.load(Ordering::SeqCst) >= count {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    shared.readers_done.load(Ordering::SeqCst) >= count
}

/// Output shared by the two reader threads and the parent.
#[derive(Default)]
struct SharedOutput {
    tail: Mutex<TailBuffer>,
    readers_done: AtomicUsize,
    wrapped_lines: AtomicUsize,
}

/// The owned direct child. Dropping the guard terminates and reaps the tree, so an early
/// `?` return or a panic cannot leak a process.
struct OwnedChild {
    child: Child,
    pid: u32,
    label: String,
    reaped: bool,
    /// The direct child's exit status once it has been reaped (the bounded cleanup keeps
    /// it, so a forced kill still reports its signal).
    status: Option<ExitStatus>,
}

impl OwnedChild {
    fn new(child: Child, label: String) -> Self {
        let pid = child.id();
        OwnedChild {
            child,
            pid,
            label,
            reaped: false,
            status: None,
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn mark_reaped(&mut self) {
        self.reaped = true;
    }

    /// Terminate the whole group with a bounded escalation, reaping the direct child when
    /// it exits inside the bound.
    fn terminate(&mut self) -> CleanupSummary {
        let mut summary = CleanupSummary {
            killed_signal: Some(termination_signal()),
            escalated_to_sigkill: false,
            reaped: false,
            note: String::new(),
        };
        if !signal_tree(self.pid, termination_signal()) {
            summary.note = "process group was already gone when the suite terminated it".into();
            summary.reaped = self.reap_bounded(CLEANUP_KILL_WAIT);
            return summary;
        }
        if !self.reap_bounded(CLEANUP_GRACE) {
            summary.escalated_to_sigkill = true;
            if signal_tree(self.pid, force_signal()) {
                summary.note = format!(
                    "{} did not exit within {:?}; forced kill",
                    self.label, CLEANUP_GRACE
                );
            } else {
                summary.note = format!("{}: forced kill found no process group", self.label);
            }
            summary.reaped = self.reap_bounded(CLEANUP_KILL_WAIT);
        } else {
            summary.reaped = true;
        }
        if !summary.reaped && summary.note.is_empty() {
            summary.note = format!(
                "{} could not be reaped within {:?}",
                self.label,
                CLEANUP_GRACE + CLEANUP_KILL_WAIT
            );
        }
        summary
    }

    /// Poll the direct child (bounded), so cleanup never blocks on an unbounded `wait`.
    fn reap_bounded(&mut self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.reaped = true;
                    self.status = Some(status);
                    return true;
                }
                Ok(None) => {}
                Err(_) => return false,
            }
            if Instant::now() >= deadline {
                return !self.group_alive();
            }
            std::thread::sleep(POLL);
        }
    }

    fn group_alive(&self) -> bool {
        process_group_alive(self.pid)
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        // Any exit path that is not the normal reaped one still reaps the tree.
        let _ = signal_tree(self.pid, force_signal());
        let deadline = Instant::now() + CLEANUP_KILL_WAIT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(POLL),
                Err(_) => return,
            }
        }
    }
}

#[cfg(unix)]
fn termination_signal() -> i32 {
    libc::SIGTERM
}

#[cfg(unix)]
fn force_signal() -> i32 {
    libc::SIGKILL
}

#[cfg(unix)]
fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(unix)]
fn signal_tree(pid: u32, signal: i32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::killpg(pid as libc::pid_t, signal) == 0 }
}

#[cfg(unix)]
fn process_group_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::killpg(pid as libc::pid_t, 0) == 0 }
}

#[cfg(unix)]
fn classify_exit(status: ExitStatus) -> (Option<i32>, Option<i32>) {
    use std::os::unix::process::ExitStatusExt;
    (status.code(), status.signal())
}

fn spawn_reader<R: Read + Send + 'static>(
    stream: R,
    log: Arc<Mutex<LogWriter>>,
    shared: Arc<SharedOutput>,
    verbose: bool,
    label: String,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut stream = stream;
        let mut buf = [0u8; 8 * 1024];
        let mut line: Vec<u8> = Vec::with_capacity(256);
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => {
                    let mut rest = &buf[..read];
                    while let Some(index) = rest.iter().position(|byte| *byte == b'\n') {
                        line.extend_from_slice(&rest[..index]);
                        emit(&log, &shared, verbose, &label, &line, false);
                        line.clear();
                        rest = &rest[index + 1..];
                    }
                    if !rest.is_empty() {
                        line.extend_from_slice(rest);
                        if line.len() >= MAX_LINE_BYTES {
                            // Bounded: an unterminated over-long line is emitted as a
                            // marked wrap instead of being buffered without limit. The
                            // marker keeps a torn receipt from being parsed as one.
                            emit(&log, &shared, verbose, &label, &line, true);
                            line.clear();
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        if !line.is_empty() {
            emit(&log, &shared, verbose, &label, &line, false);
        }
        shared.readers_done.fetch_add(1, Ordering::SeqCst);
        // `log` drops here (the last clone for this reader), flushing through Drop.
    })
}

fn emit(
    log: &Arc<Mutex<LogWriter>>,
    shared: &Arc<SharedOutput>,
    verbose: bool,
    label: &str,
    line: &[u8],
    wrapped: bool,
) {
    if wrapped {
        shared.wrapped_lines.fetch_add(1, Ordering::SeqCst);
        eprintln!(
            "[{label}] output line exceeded {MAX_LINE_BYTES} bytes without a newline; emitted wrapped"
        );
    }
    if verbose {
        let text = String::from_utf8_lossy(line);
        if wrapped {
            eprintln!("[{label}] (wrapped) {text}");
        } else {
            eprintln!("[{label}] {text}");
        }
    }
    let mut writer = log.lock().unwrap();
    if wrapped {
        writer.write_line(b"[suite] wrapped line follows");
    }
    writer.write_line(line);
    shared.tail.lock().unwrap().push(line);
}

/// Bounded log file writer: past the cap it records that it stopped. Flushes on drop, so
/// a detached reader cannot leave the log unflushed.
struct LogWriter {
    file: std::fs::File,
    written: u64,
    truncated: bool,
}

impl LogWriter {
    fn open(path: &Path) -> SuiteResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("log dir {}: {error}", parent.display()))?;
        }
        let file = std::fs::File::create(path)
            .map_err(|error| format!("log {}: {error}", path.display()))?;
        Ok(LogWriter {
            file,
            written: 0,
            truncated: false,
        })
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn write_line(&mut self, line: &[u8]) {
        if self.truncated {
            return;
        }
        if self.written + line.len() as u64 + 1 > LOG_CAP_BYTES {
            self.truncated = true;
            let _ = self.file.write_all(b"[suite] log truncated: cap reached\n");
            return;
        }
        let _ = self.file.write_all(line);
        let _ = self.file.write_all(b"\n");
        self.written += line.len() as u64 + 1;
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        let _ = self.file.flush();
    }
}

/// Bounded tail kept for receipt parsing. When the cap forces a drain mid-line the
/// partial first line is dropped, so a torn receipt is never parsed.
#[derive(Default)]
struct TailBuffer {
    bytes: Vec<u8>,
    head_partial: bool,
}

impl TailBuffer {
    fn push(&mut self, line: &[u8]) {
        self.bytes.extend_from_slice(line);
        self.bytes.push(b'\n');
        if self.bytes.len() > TAIL_CAP_BYTES {
            let excess = self.bytes.len() - TAIL_CAP_BYTES;
            self.bytes.drain(..excess);
            self.head_partial = true;
        }
    }

    fn text(&self) -> String {
        let bytes = if self.head_partial {
            let start = self
                .bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|index| index + 1)
                .unwrap_or(0);
            &self.bytes[start..]
        } else {
            &self.bytes[..]
        };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> NativeConfig {
        NativeConfig {
            profile: "local-274".into(),
            revision: Some("274".into()),
            host: Some("127.0.0.1".into()),
            port: Some(43594),
            engine: None,
            cache: None,
            catalog: PathBuf::from("/catalog"),
            vault: None,
            lowmem: true,
            mainland: false,
            exec_core: None,
            exec_pair: None,
            cwd: None,
            extra_args: Vec::new(),
        }
    }

    fn case(id: &str) -> CaseEntry {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        manifest.case(id).expect("case").clone()
    }

    /// The flags must be exactly the ones the native executables accept: the panel family
    /// parses them with `host_play::parse_profile_args` and hands the remainder to its live
    /// parser, which rejects anything but `--live`/`--smoke`/`--prod`. Consuming every flag
    /// here proves the child sees exactly `--live <name>`.
    #[test]
    fn profile_args_are_exactly_what_the_native_executables_accept() {
        let args = config().profile_args();
        assert_eq!(
            args,
            vec![
                "--profile",
                "local-274",
                "--revision",
                "274",
                "--host",
                "127.0.0.1",
                "--port",
                "43594",
                "--catalog",
                "/catalog",
            ]
        );
        let (options, rest) =
            host_play::parse_profile_args(args.iter().map(String::as_str)).unwrap();
        assert!(
            rest.is_empty(),
            "the panel's live parser would reject these: {rest:?}"
        );
        assert_eq!(options.profile.as_deref(), Some("local-274"));
        assert_eq!(options.catalog_root, Some(PathBuf::from("/catalog")));
        assert_eq!(options.port, Some(43594));
        assert!(
            config().validate().is_err(),
            "a missing catalog fails closed"
        );
    }

    /// Mainland is not a flag of these executables; it travels as `BOT_MAINLAND=1`. The
    /// memory mode is the profile's own setting, so `--highmem` is refused rather than
    /// recorded as a request the adapters cannot execute.
    #[test]
    fn mainland_travels_as_environment_and_highmem_fails_closed() {
        let mut config = config();
        config.mainland = true;
        let env = config.child_env(&case("thiever"), Path::new("/run/shots"), &BTreeMap::new());
        assert_eq!(env.get(super::MAINLAND_ENV).map(String::as_str), Some("1"));
        assert_eq!(
            env.get(scenario::shot::SHOT_ROOT_ENV).map(String::as_str),
            Some("/run/shots")
        );
        assert_eq!(
            config.env_keys(),
            vec![
                scenario::shot::SHOT_ROOT_ENV.to_string(),
                super::MAINLAND_ENV.to_string()
            ]
        );
        assert!(
            !config
                .profile_args()
                .iter()
                .any(|arg| arg == "--mainland" || arg == "--lowmem" || arg == "--highmem"),
            "front-end-only flags must never reach the child"
        );

        let mut high = config.clone();
        high.lowmem = false;
        let error = high.validate().unwrap_err();
        assert!(error.contains("--highmem"), "{error}");
    }

    #[test]
    fn a_case_command_is_the_executable_profile_flags_and_its_live_name() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let mut config = config();
        config.catalog = PathBuf::from(manifest.defaults.exec.core.program.clone());
        config.exec_core = Some(PathBuf::from("/bin/fixture-core"));
        config.exec_pair = Some(PathBuf::from("/bin/fixture-pair"));
        config.mainland = true;

        let core = config.command(&case("thiever"), &manifest).unwrap();
        assert_eq!(core[0], "/bin/fixture-core");
        assert_eq!(core[core.len() - 2..], ["--live", "script_thiever"]);
        assert!(!core.iter().any(|arg| arg == "--lowmem"));

        let pair = config
            .command(&case("nature_crafter_air"), &manifest)
            .unwrap();
        assert_eq!(pair[0], "/bin/fixture-pair");
        assert_eq!(
            pair[pair.len() - 2..],
            ["--live", "script_nature_crafter_air"]
        );

        // Without a direct executable the manifest template is used verbatim.
        config.exec_core = None;
        let templated = config.command(&case("thiever"), &manifest).unwrap();
        let mut expected = vec![manifest.defaults.exec.core.program.clone()];
        expected.extend(manifest.defaults.exec.core.args.iter().cloned());
        assert_eq!(templated[..expected.len()], expected[..]);
    }

    #[test]
    fn tail_buffer_is_bounded_and_drops_partial_head() {
        let mut tail = TailBuffer::default();
        for i in 0..4 {
            tail.push(format!("line {i}").as_bytes());
        }
        assert_eq!(tail.text(), "line 0\nline 1\nline 2\nline 3\n");
        for i in 0..(TAIL_CAP_BYTES / 8 + 16) {
            tail.push(format!("{i:06} filler").as_bytes());
        }
        assert!(tail.bytes.len() <= TAIL_CAP_BYTES, "tail stays bounded");
        assert!(tail.text().len() <= TAIL_CAP_BYTES);
        assert!(tail.text().ends_with('\n'));
    }

    #[test]
    fn unsupported_reference_args_and_env_are_not_forwarded() {
        assert!(!supported_arg("--no-deploy"));
        assert!(!supported_arg("--minutes"));
        assert!(!typed_env_key("HEADED"));
        assert!(!typed_env_key("E2E_MINUTES"));
    }
}
