//! Command construction and owned child execution.
//!
//! The suite launches the existing native executables (`panel` `catalog_watch` for core
//! witnesses, `panel` `pair_watch` for pair witnesses) with an explicit, validated
//! native profile/input configuration. It never builds its own scenario engine and never
//! runs a foreign runtime.
//!
//! Ownership: every child is started in its own process group, its whole tree is
//! terminated on budget expiry or interrupt, and cleanup is bounded (`SIGTERM`, a short
//! grace, then `SIGKILL`). A process tree that cannot be reaped inside that bound is
//! reported as a harness failure instead of being ignored.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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

#[cfg(not(unix))]
pub fn install_signal_handler() {
    // No handler: on Windows the console control handler would need an extra crate, and
    // a Ctrl-C there terminates the suite without a ledger update. The child still runs
    // in its own process group (`CREATE_NEW_PROCESS_GROUP`).
}

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
    /// Direct native executables. When absent the manifest's cargo template is used.
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

    /// The executable identity recorded for a runner kind. A direct executable is hashed;
    /// the cargo template records the command instead of inventing a file hash.
    pub fn binary(
        &self,
        runner: RunnerKind,
        manifest: &SuiteManifest,
    ) -> SuiteResult<BinaryIdentity> {
        let direct = match runner {
            RunnerKind::Core => self.exec_core.as_ref(),
            RunnerKind::Pair => self.exec_pair.as_ref(),
        };
        match direct {
            Some(path) => BinaryIdentity::direct(path),
            None => {
                let template = match runner {
                    RunnerKind::Core => &manifest.defaults.exec.core,
                    RunnerKind::Pair => &manifest.defaults.exec.pair,
                };
                Ok(BinaryIdentity::cargo(&template.program, &template.args))
            }
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
pub fn run(
    spec: &ChildSpec,
    budget: Duration,
    log_path: &Path,
    verbose: bool,
) -> SuiteResult<ChildRun> {
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
    let mut child = command
        .spawn()
        .map_err(|error| format!("{}: cannot launch {}: {error}", spec.label, spec.command[0]))?;
    let pid = child.id();

    let log = Arc::new(Mutex::new(LogWriter::open(log_path)?));
    let tail = Arc::new(Mutex::new(TailBuffer::default()));
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_reader(
            stdout,
            Arc::clone(&log),
            Arc::clone(&tail),
            verbose,
            spec.label.clone(),
        ));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_reader(
            stderr,
            Arc::clone(&log),
            Arc::clone(&tail),
            verbose,
            spec.label.clone(),
        ));
    }

    let deadline = started + budget;
    let mut timed_out = false;
    let mut interrupted = false;
    let mut cleanup = CleanupSummary::default();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => {
                return Err(format!("{}: wait failed: {error}", spec.label));
            }
        }
        if interrupt_requested() {
            interrupted = true;
            cleanup = terminate_tree(pid, spec);
            break child.wait().ok();
        }
        if Instant::now() >= deadline {
            timed_out = true;
            cleanup = terminate_tree(pid, spec);
            break child.wait().ok();
        }
        std::thread::sleep(POLL);
    };

    for reader in readers {
        let _ = reader.join();
    }
    log.lock().unwrap().flush().ok();
    let log_truncated = log.lock().unwrap().truncated;
    let output_tail = tail.lock().unwrap().text();

    let (exit_code, signal) = match status {
        Some(status) => classify_exit(status),
        None => (None, None),
    };
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

/// Terminate the child's whole process group with a bounded escalation.
fn terminate_tree(pid: u32, spec: &ChildSpec) -> CleanupSummary {
    let mut summary = CleanupSummary {
        killed_signal: Some(termination_signal()),
        escalated_to_sigkill: false,
        reaped: false,
        note: String::new(),
    };
    if !signal_tree(pid, termination_signal()) {
        summary.note = "process group was already gone when the suite terminated it".into();
        summary.reaped = true;
        return summary;
    }
    if !wait_for_exit(pid, CLEANUP_GRACE) {
        summary.escalated_to_sigkill = true;
        if signal_tree(pid, force_signal()) {
            summary.note = format!(
                "{} did not exit within {:?}; forced kill",
                spec.label, CLEANUP_GRACE
            );
        } else {
            summary.note = format!("{}: forced kill found no process group", spec.label);
        }
        summary.reaped = wait_for_exit(pid, CLEANUP_KILL_WAIT);
    } else {
        summary.reaped = true;
    }
    if !summary.reaped && summary.note.is_empty() {
        summary.note = format!(
            "{} could not be reaped within {:?}",
            spec.label,
            CLEANUP_GRACE + CLEANUP_KILL_WAIT
        );
    }
    summary
}

/// Poll the process group until it is gone (or the bound lapses).
fn wait_for_exit(pid: u32, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        if !process_group_alive(pid) {
            return true;
        }
        std::thread::sleep(POLL);
    }
    !process_group_alive(pid)
}

#[cfg(unix)]
fn termination_signal() -> i32 {
    libc::SIGTERM
}

#[cfg(not(unix))]
fn termination_signal() -> i32 {
    15
}

#[cfg(unix)]
fn force_signal() -> i32 {
    libc::SIGKILL
}

#[cfg(not(unix))]
fn force_signal() -> i32 {
    9
}

#[cfg(unix)]
fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn own_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn signal_tree(pid: u32, signal: i32) -> bool {
    unsafe { libc::killpg(pid as libc::pid_t, signal) == 0 }
}

#[cfg(windows)]
fn signal_tree(pid: u32, _signal: i32) -> bool {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn process_group_alive(pid: u32) -> bool {
    unsafe { libc::killpg(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn process_group_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(unix)]
fn classify_exit(status: std::process::ExitStatus) -> (Option<i32>, Option<i32>) {
    use std::os::unix::process::ExitStatusExt;
    (status.code(), status.signal())
}

#[cfg(not(unix))]
fn classify_exit(status: std::process::ExitStatus) -> (Option<i32>, Option<i32>) {
    (status.code(), None)
}

fn spawn_reader<R: std::io::Read + Send + 'static>(
    stream: R,
    log: Arc<Mutex<LogWriter>>,
    tail: Arc<Mutex<TailBuffer>>,
    verbose: bool,
    label: String,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.split(b'\n') {
            let Ok(line) = line else { break };
            if verbose {
                eprintln!("[{}] {}", label, String::from_utf8_lossy(&line));
            }
            log.lock().unwrap().write_line(&line);
            tail.lock().unwrap().push(&line);
        }
    })
}

/// Bounded log file writer: past the cap it records that it stopped.
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
