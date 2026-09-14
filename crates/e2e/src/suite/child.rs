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
//! Platform: the wait/terminate/drain body below is shared and the ownership facility is
//! the only platform-specific part. Unix owns the child's process group with signals;
//! Windows owns it with a job object (`KILL_ON_JOB_CLOSE`, assigned before the suspended
//! child can run, so no descendant can exist outside the job). An unknown platform still
//! fails closed in [`run`] instead of launching a child it cannot own, so the shared body
//! is allowed to be dead code there rather than a second, weaker ownership path.
//!
//! Deadline environment: the suite refuses a nonempty inherited native deadline control
//! (see [`DEADLINE_ENV`]) before any launch. It hands a child the case budget and the
//! scenario's own inner deadline, and an inherited override would move the child's inner
//! deadline and its post-PASS window away from what the run recorded.

#![cfg_attr(
    not(any(unix, windows)),
    allow(dead_code, unused_imports, unused_variables)
)]

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
/// Additional drain bound granted after the leftover owned tree is force-stopped.
pub const PIPE_DRAIN_AFTER_KILL: Duration = Duration::from_secs(2);
/// Poll interval for the wait loop.
const POLL: Duration = Duration::from_millis(50);

/// The panel's mainland knob: `BOT_MAINLAND=1`, the same variable the `host-play` binary's
/// `--mainland` sets. `catalog_watch`/`pair_watch` expose no such flag, so the suite asks
/// for it in the child's environment.
pub const MAINLAND_ENV: &str = "BOT_MAINLAND";

/// Native deadline controls the suite refuses to inherit.
///
/// The bounded list is the *actually consumed* controls of the executables the suite
/// launches: `BUDGET_S` (`scenario::budget_s_from_env`) replaces the `ScenarioRunner`
/// deadline and keeps the panel window open after a proof PASS, so an inherited value
/// would silently move the child's inner deadline and its post-PASS window away from the
/// case budget the run recorded. Nothing speculative is listed: `BOT_MAINLAND`, `BOT_DEBUG`
/// and `BOT_CPU` stay deliberate operator knobs, and the profile env (`NAV_*`,
/// `ENGINE_DIR`, …) is bound through the identity instead.
pub const DEADLINE_ENV: &[&str] = &["BUDGET_S"];

/// The inherited native deadline controls that are actually set, in a stable order.
pub fn inherited_deadline_env() -> Vec<String> {
    DEADLINE_ENV
        .iter()
        .filter(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        .map(|name| (*name).to_string())
        .collect()
}

/// Refuse to run while an inherited native deadline control would change the child's inner
/// deadline behind the recorded budget.
///
/// Called before binary resolution, the identity, the ledger and any resume check, so a run
/// and a resume refuse alike with no child launched. The suite refuses instead of clearing
/// the variable, so the enforced policy is always the recorded budget and no identity field
/// has to record a policy an operator could still change.
pub fn validate_deadline_env() -> SuiteResult<()> {
    let inherited = inherited_deadline_env();
    if inherited.is_empty() {
        return Ok(());
    }
    let names = inherited.join(", ");
    Err(format!(
        "${names} is set in the suite's environment: the suite enforces the case budget and \
         the scenario's own inner deadline, and an inherited native deadline override would \
         silently change the child's inner deadline and its window after a proof PASS. Unset \
         {names} for this run (the panel reads it from its own environment, never from the \
         run's recorded budget)"
    ))
}

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

/// Install the console-control flag on Windows. `CTRL_C`/`CTRL_BREAK` store the interrupt
/// flag and are reported handled, so the wait loop reaps the owned tree and the run exits
/// 130 instead of the suite dying mid-case with the child still running. The other events
/// keep the system's own handling: the suite does not claim it can reap a tree inside the
/// few seconds the system allows there.
#[cfg(windows)]
pub fn install_signal_handler() {
    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT,
    };

    unsafe extern "system" fn handle(ctrltype: u32) -> i32 {
        match ctrltype {
            CTRL_C_EVENT | CTRL_BREAK_EVENT => {
                INTERRUPT.store(true, Ordering::SeqCst);
                TRUE
            }
            _ => FALSE,
        }
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handle), TRUE);
    }
}

/// An unknown platform installs no handler; [`run`] refuses to launch there, so there is no
/// child an interrupt could leave behind.
#[cfg(not(any(unix, windows)))]
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
    /// Headed diagnostic layers; the suite defaults on without persisting panel settings.
    pub nav_paints: bool,
    /// Direct native executables. When absent the manifest's cargo template is resolved to
    /// a built artifact and hashed (see [`NativeConfig::binary`]).
    pub exec_core: Option<PathBuf>,
    pub exec_pair: Option<PathBuf>,
    /// The dedicated external loader smoke's executable (`external_watch`).
    pub exec_external: Option<PathBuf>,
    /// The raw TypeScript input for the loader smoke. Absolute; absent means the child's own
    /// tracked default fixture, which the run identity still binds (see
    /// [`super::identity::ExternalSource`]).
    pub external_ts: Option<PathBuf>,
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
        if self.extra_args.iter().any(|arg| arg == "--nav-paints") {
            return Err(
                "use the suite --nav-paints option instead of --child-arg --nav-paints".into(),
            );
        }
        // A typed binding the run resolved and recorded must not be re-bound by a raw extra
        // argument appended after it: the child takes the last `--live`/`--external-ts`, so an
        // extra one would run something other than the selection the identity was captured for.
        // Both are refused unconditionally — an `--external-ts` reached only through `--child-arg`
        // is a source the run never bound.
        for flag in ["--live", "--external-ts"] {
            if self.extra_args.iter().any(|arg| arg == flag) {
                return Err(format!(
                    "use the suite's typed {flag} selection instead of --child-arg {flag}: a raw \
                     argument would override the binding the run identity recorded"
                ));
            }
        }
        if self.profile.trim().is_empty() {
            return Err("--profile must name the native server profile".into());
        }
        // The native resolver expands relative paths against the working directory. The
        // suite resolves them in its own process while a child resolves them in its own
        // cwd, so the two would not necessarily agree: refuse a relative input instead of
        // binding a path the child may not read.
        for (flag, path) in [
            ("--catalog", Some(&self.catalog)),
            ("--engine", self.engine.as_ref()),
            ("--cache", self.cache.as_ref()),
            ("--vault", self.vault.as_ref()),
            ("--exec-core", self.exec_core.as_ref()),
            ("--exec-pair", self.exec_pair.as_ref()),
            ("--exec-external", self.exec_external.as_ref()),
            // Only the *shape* of the raw source is checked here: it is resolved (and hashed)
            // when the selection actually launches the loader smoke, so a stale `--external-ts`
            // cannot force an ordinary core/pair run to resolve an external resource.
            ("--external-ts", self.external_ts.as_ref()),
            ("--cwd", self.cwd.as_ref()),
        ] {
            if let Some(path) = path {
                if path.is_relative() && !path.as_os_str().is_empty() {
                    return Err(format!(
                        "{flag} {} is relative; the child resolves it against its own working directory, so \
                         the suite cannot bind the path it reads. Pass an absolute path",
                        path.display()
                    ));
                }
            }
        }
        for name in [
            "ENGINE_DIR",
            "CLIENT_UNPACK_DIR",
            "NAV_PACK",
            "NAV_FLAGS",
            "BOT_CACHE_MANIFEST",
            "CARGO_TARGET_DIR",
        ] {
            if let Some(value) = std::env::var_os(name) {
                let value = PathBuf::from(value);
                if !value.as_os_str().is_empty() && value.is_relative() {
                    return Err(format!(
                        "${name}={} is relative; the profile resolver expands it against the suite's \
                         working directory while the child expands it against its own. Use an absolute path",
                        value.display()
                    ));
                }
            }
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
            ("--exec-external", self.exec_external.as_ref()),
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
            RunnerKind::External => (
                self.exec_external.as_ref(),
                &manifest.defaults.exec.external,
                "--exec-external",
            ),
        };
        match direct {
            Some(path) => BinaryIdentity::direct(path),
            None => BinaryIdentity::resolve_template(template, repo_root, flag),
        }
    }

    /// Resolve and hash the executable for every runner kind these cases will launch.
    ///
    /// The map is the run's executable identity *and* its launch program: the same
    /// [`BinaryIdentity`] that goes into the ledger is what [`NativeConfig::command`]
    /// launches. Resolving happens before the ledger exists, so an executable the suite
    /// cannot bind to bytes is a configuration error the run refuses rather than an
    /// identity it records and compares later.
    pub fn binaries(
        &self,
        runners: impl IntoIterator<Item = RunnerKind>,
        manifest: &SuiteManifest,
        repo_root: Option<&Path>,
    ) -> SuiteResult<BTreeMap<String, BinaryIdentity>> {
        let mut binaries: BTreeMap<String, BinaryIdentity> = BTreeMap::new();
        for runner in runners {
            if !binaries.contains_key(runner.as_str()) {
                binaries.insert(
                    runner.as_str().to_string(),
                    self.binary(runner, manifest, repo_root)?,
                );
            }
        }
        Ok(binaries)
    }

    /// The full command line for one case: the executable the run identity resolved and
    /// hashed, the shared profile flags, and the case's live name.
    ///
    /// The manifest's cargo template never appears here. It only names the artifact
    /// [`NativeConfig::binaries`] resolved; re-invoking `cargo run` after the identity was
    /// captured could rebuild different bytes under the same command, so a runner kind
    /// with no resolved executable refuses instead of falling back to the template.
    pub fn command(
        &self,
        case: &CaseEntry,
        binaries: &BTreeMap<String, BinaryIdentity>,
    ) -> SuiteResult<Vec<String>> {
        let live = case
            .live
            .as_deref()
            .ok_or_else(|| format!("{}: no native live name", case.id))?;
        let runner = case.runner();
        let flag = match runner {
            RunnerKind::Core => "--exec-core",
            RunnerKind::Pair => "--exec-pair",
            RunnerKind::External => "--exec-external",
        };
        let binary = binaries.get(runner.as_str()).ok_or_else(|| {
            format!(
                "{}: no {} executable was resolved into the run identity; pass {flag} PATH or \
                 build the manifest's cargo template before the run",
                case.id,
                runner.as_str()
            )
        })?;
        if !binary.resolved() {
            return Err(format!(
                "{}: the {} executable {} has no content identity; pass {flag} PATH",
                case.id,
                runner.as_str(),
                binary.program
            ));
        }
        // `binary.args` stays identity metadata (the cargo template it came from); the
        // resolved artifact takes only the profile flags.
        let mut command = vec![binary.program.clone()];
        command.extend(self.profile_args());
        command.extend([
            "--nav-paints".into(),
            if self.nav_paints { "on" } else { "off" }.into(),
        ]);
        command.push("--live".into());
        command.push(live.to_string());
        if runner == RunnerKind::External {
            // The typed source override, bound before launch. Absent, the child reads its own
            // compile-time fixture, which the run identity bound as well.
            if let Some(source) = &self.external_ts {
                command.push("--external-ts".into());
                command.push(source.display().to_string());
            }
        }
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
    /// identity so a changed environment request refuses resume. Inherited profile env
    /// (NAV_*, ENGINE_DIR, …) is bound through the resolver, not by copying values here.
    pub fn env_keys(&self) -> Vec<String> {
        let mut keys = vec![scenario::shot::SHOT_ROOT_ENV.to_string()];
        if self.mainland {
            keys.push(MAINLAND_ENV.to_string());
        }
        keys
    }

    /// Canonical working directory every child is launched in. Relative `--cwd` is
    /// already refused by [`NativeConfig::validate`].
    pub fn launch_cwd(&self, repo_root: Option<&Path>) -> SuiteResult<PathBuf> {
        let (raw, flag) = match &self.cwd {
            Some(cwd) => (cwd.clone(), "--cwd"),
            None => (
                repo_root
                    .map(Path::to_path_buf)
                    .or_else(|| std::env::current_dir().ok())
                    .ok_or_else(|| "cannot determine the child working directory".to_string())?,
                "working directory",
            ),
        };
        super::identity::canonicalize_launch_path(&raw, flag)
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
    /// The direct child's pid, which is also its process-group id (the group the suite
    /// owns and reaps).
    pub pid: u32,
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
#[cfg(any(unix, windows))]
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
    // Mainland is always overridden: an inherited BOT_MAINLAND cannot evade the
    // recorded profile flag. Removal happens after the copy so it actually takes effect.
    if !spec.env.contains_key(MAINLAND_ENV) {
        command.env_remove(MAINLAND_ENV);
    }
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    // The ownership facility exists *before* the spawn: on Windows the child is created
    // suspended and assigned to the job it will live in, so no descendant can be created
    // outside the tree the suite owns.
    let ownership = Ownership::new()?;
    ownership.prepare(&mut command);

    let started = Instant::now();
    let child = command
        .spawn()
        .map_err(|error| format!("{}: cannot launch {}: {error}", spec.label, spec.command[0]))?;
    // The guard owns the tree from here: an adopt failure below returns through its Drop,
    // which terminates and reaps whatever was started.
    let mut owned = OwnedChild::new(child, spec.label.clone(), ownership)?;

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

    // Bounded drain of the output pipes. A descendant that inherited stdout/stderr keeps
    // the pipe open after the direct child exits, so this waits a bound, then the owned tree
    // is terminated below to close the write ends. An unbounded join is never taken.
    let drained = drain_readers(&shared, readers.len(), PIPE_DRAIN);

    // Ownership is the *tree*, not the direct child. A direct child that exited is not a
    // tree that exited: a descendant can hold the pipes, or be detached with its own stdio
    // so that nothing looks wrong from here. Either way it is still ours to reap, and the
    // cleanup stays bounded (graceful stop, a grace, forced stop, a bounded wait).
    let case_reaped = status.is_some() && !owned.ownership.alive(pid);
    if !drained || !case_reaped {
        let mut parts: Vec<String> = Vec::new();
        if !drained {
            parts.push(format!(
                "output pipes were still open after {:?} (a descendant held them)",
                PIPE_DRAIN
            ));
        } else {
            parts.push("the direct child exited but a descendant was still running".to_string());
        }
        let leftover = owned.terminate();
        if leftover.reaped {
            if leftover.escalated_to_sigkill {
                parts.push("the leftover tree needed the forced stop".to_string());
            } else {
                parts.push("the leftover tree was terminated".to_string());
            }
        } else {
            parts.push(format!(
                "the leftover tree could not be reaped within the bound ({})",
                leftover.note
            ));
        }
        // A pipe that was still open gets a further bounded drain now that the write ends
        // are (or should be) closed.
        let fully_drained = drained || drain_readers(&shared, readers.len(), PIPE_DRAIN_AFTER_KILL);
        if !fully_drained {
            parts.push("output pipes did not close within the cleanup bound".to_string());
        }
        let note = format!("{}: {}", spec.label, parts.join("; "));
        cleanup.note = if cleanup.note.is_empty() {
            note
        } else {
            format!("{}; {note}", cleanup.note)
        };
        cleanup.killed_signal = cleanup
            .killed_signal
            .or_else(|| owned.ownership.graceful_signal());
        cleanup.escalated_to_sigkill |= leftover.escalated_to_sigkill;
        cleanup.reaped = leftover.reaped && status.is_some() && fully_drained;
    } else {
        // The direct child was waited and the owned tree holds no process.
        cleanup.reaped = true;
    }
    drop(owned);

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
        pid,
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

/// A platform with neither process groups nor job objects cannot be given bounded tree
/// ownership, so the suite refuses to launch a child there rather than claim ownership it
/// cannot enforce.
#[cfg(not(any(unix, windows)))]
pub fn run(
    spec: &ChildSpec,
    _budget: Duration,
    _log_path: &Path,
    _verbose: bool,
) -> SuiteResult<ChildRun> {
    Err(format!(
        "{}: owned child execution needs unix process groups or Windows job objects; this \
         platform is fail-closed (no bounded process-tree ownership), so the suite refuses to \
         launch",
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
    /// The OS facility that owns the whole tree (see [`Ownership`]).
    ownership: Ownership,
}

impl OwnedChild {
    /// Attach the launched child to the ownership facility.
    ///
    /// On Windows the child is still suspended here, so the job is assigned before a single
    /// instruction of it runs. A failure returns through `Drop`, which terminates and reaps
    /// what was started: no error path leaves a child behind.
    fn new(child: Child, label: String, ownership: Ownership) -> SuiteResult<Self> {
        let pid = child.id();
        let mut owned = OwnedChild {
            child,
            pid,
            label,
            reaped: false,
            status: None,
            ownership,
        };
        owned.ownership.adopt(&mut owned.child).map_err(|error| {
            format!(
                "{}: cannot own the launched child tree: {error}",
                owned.label
            )
        })?;
        Ok(owned)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn mark_reaped(&mut self) {
        self.reaped = true;
    }

    /// Terminate and reap the whole tree, bounded: the graceful stop, a grace, the forced
    /// stop, then a bounded wait for the tree to disappear. `reaped` is true only when the
    /// direct child has been waited *and* no process of the owned tree survives: a direct
    /// child that exited while a descendant lived on is not a reaped tree.
    fn terminate(&mut self) -> CleanupSummary {
        let mut summary = CleanupSummary {
            killed_signal: self.ownership.graceful_signal(),
            escalated_to_sigkill: false,
            reaped: false,
            note: String::new(),
        };
        if !self.ownership.request_graceful(self.pid) {
            summary.note = self.ownership.graceful_missing_note();
        } else {
            #[cfg(windows)]
            {
                summary.note = "sent CTRL_BREAK to the owned Windows console group".into();
            }
        }
        if !self.wait_tree(CLEANUP_GRACE) {
            summary.escalated_to_sigkill = true;
            if self.ownership.request_forced(self.pid) {
                summary.note = format!(
                    "{} did not exit within {:?}; forced stop",
                    self.label, CLEANUP_GRACE
                );
            } else if summary.note.is_empty() {
                summary.note = format!("{}: forced stop found no process tree", self.label);
            }
            summary.reaped = self.wait_tree(CLEANUP_KILL_WAIT);
        } else {
            summary.reaped = true;
        }
        if !summary.reaped {
            summary.note = if summary.note.is_empty() {
                format!(
                    "{} could not be reaped within {:?}",
                    self.label,
                    CLEANUP_GRACE + CLEANUP_KILL_WAIT
                )
            } else {
                format!("{}; not reaped within the bound", summary.note)
            };
        }
        summary
    }

    /// Poll, bounded, until the direct child has been waited *and* the owned tree holds no
    /// process. Returns whether the tree is fully reaped. The direct child is reaped first:
    /// an unreaped leader keeps the tree observable, so an unreaped exit would otherwise
    /// look like a live tree.
    fn wait_tree(&mut self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        loop {
            if !self.reaped {
                if let Ok(Some(status)) = self.child.try_wait() {
                    self.reaped = true;
                    self.status = Some(status);
                }
            }
            if self.reaped && !self.tree_alive() {
                return true;
            }
            if Instant::now() >= deadline {
                return self.reaped && !self.tree_alive();
            }
            std::thread::sleep(POLL);
        }
    }

    fn tree_alive(&self) -> bool {
        self.ownership.alive(self.pid)
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.reaped && !self.tree_alive() {
            return;
        }
        // Any other exit path — an early `?`, a panic, a cleanup that ran out of bound —
        // still owns the tree: force-stop the whole tree and wait, bounded, for the direct
        // child *and* every other member of it to be gone. The direct child is killed as
        // well, which covers the window where the ownership facility was never attached.
        let _ = self.ownership.request_forced(self.pid);
        let _ = self.child.kill();
        let deadline = Instant::now() + CLEANUP_KILL_WAIT;
        loop {
            if !self.reaped {
                if let Ok(Some(status)) = self.child.try_wait() {
                    self.reaped = true;
                    self.status = Some(status);
                }
            }
            if self.reaped && !self.tree_alive() {
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(POLL);
        }
    }
}

/// The OS facility that owns a child's process tree.
///
/// Unix owns a process group with signals; Windows owns a job object. Everything else in
/// this module — the deadline-aware wait loop, the forced escalation, the bounded pipe
/// drain, the receipt of "direct child reaped, tree gone, pipes closed" as three separate
/// facts — is shared, so there is exactly one runner body and one set of bounds.
#[cfg(unix)]
struct Ownership;

#[cfg(unix)]
impl Ownership {
    /// Unix needs no state: the group id *is* the child's pid.
    fn new() -> SuiteResult<Self> {
        Ok(Ownership)
    }

    /// Put the child in its own process group before it runs, so the suite can signal the
    /// whole tree and never itself.
    fn prepare(&self, command: &mut Command) {
        own_process_group(command);
    }

    /// Nothing to attach after the spawn: the group exists because of the spawn itself.
    fn adopt(&self, _child: &mut Child) -> std::io::Result<()> {
        Ok(())
    }

    fn request_graceful(&self, pid: u32) -> bool {
        signal_tree(pid, termination_signal())
    }

    fn request_forced(&self, pid: u32) -> bool {
        signal_tree(pid, force_signal())
    }

    /// Whether any process of the owned group is alive.
    fn alive(&self, pid: u32) -> bool {
        process_group_alive(pid)
    }

    fn graceful_signal(&self) -> Option<i32> {
        Some(termination_signal())
    }

    fn graceful_missing_note(&self) -> String {
        "the process group was already gone when the suite terminated it".into()
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

/// A Windows job object handle. Closing it is also the last-resort kill: the job is created
/// with `KILL_ON_JOB_CLOSE`, so the handle going away — including through a panic — ends
/// every process still in it.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl std::fmt::Debug for Job {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Job")
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct Ownership {
    job: Job,
}

#[cfg(windows)]
impl Ownership {
    fn new() -> SuiteResult<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(format!(
                "cannot create the job object that owns the child tree: {}",
                std::io::Error::last_os_error()
            ));
        }
        let ownership = Ownership { job: Job(job) };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                ownership.job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(format!(
                "cannot set kill-on-close on the job object that owns the child tree: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(ownership)
    }

    /// Create the child suspended and in its own console process group: it can then be
    /// assigned to the job before it runs a single instruction. `CREATE_NEW_PROCESS_GROUP`
    /// also keeps the operator's Ctrl-C off the child (as a unix child in its own group),
    /// so an interrupt reaches the suite, which reaps the tree.
    fn prepare(&self, command: &mut Command) {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED};
        command.creation_flags(CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP);
    }

    /// Assign the suspended child to the job and resume it: from here on no descendant can
    /// be created outside the tree the suite owns.
    fn adopt(&self, child: &mut Child) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let process = child.as_raw_handle() as HANDLE;
        if unsafe { AssignProcessToJobObject(self.job.0, process) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        resume_main_thread(child.id())
    }

    /// Ctrl-Break to the child's own console process group (`CREATE_NEW_PROCESS_GROUP` made
    /// its pid the group id). This fails when the suite has no console; the escalation then
    /// ends the tree inside the same bound.
    fn request_graceful(&self, pid: u32) -> bool {
        use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
        if pid == 0 {
            return false;
        }
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) != 0 }
    }

    /// The forced stop is the job, not a pid: every process still in it is terminated,
    /// including one that ignored the console event and one the suite never saw.
    fn request_forced(&self, _pid: u32) -> bool {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        unsafe { TerminateJobObject(self.job.0, 1) != 0 }
    }

    /// Whether the job holds any process. The job is the owner, so this is the tree, not the
    /// direct child. A query that fails cannot prove the tree is gone and counts as alive.
    fn alive(&self, _pid: u32) -> bool {
        use windows_sys::Win32::System::JobObjects::{
            JobObjectBasicAccountingInformation, QueryInformationJobObject,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        };
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let mut returned = 0u32;
        let ok = unsafe {
            QueryInformationJobObject(
                self.job.0,
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
                    as *mut core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                &mut returned,
            )
        };
        ok == 0 || accounting.ActiveProcesses > 0
    }

    /// Windows has no termination signal to report; the cleanup note names the mechanism.
    fn graceful_signal(&self) -> Option<i32> {
        None
    }

    fn graceful_missing_note(&self) -> String {
        "no console was available to deliver a graceful stop; the suite terminated the job".into()
    }
}

/// Resume the main thread of a `CREATE_SUSPENDED` child with documented APIs.
///
/// A suspended process cannot create a thread, so the thread owned by `pid` is the one the
/// spawn suspended; there is no window in which a second thread could be missed.
#[cfg(windows)]
fn resume_main_thread(pid: u32) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0usize;
    let mut ok = unsafe { Thread32First(snapshot, &mut entry) };
    while ok != 0 {
        if entry.th32OwnerProcessID == pid {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                let previous = unsafe { ResumeThread(thread) };
                unsafe { CloseHandle(thread) };
                if previous != u32::MAX {
                    resumed += 1;
                }
            }
        }
        ok = unsafe { Thread32Next(snapshot, &mut entry) };
    }
    unsafe { CloseHandle(snapshot) };
    if resumed == 0 {
        return Err(std::io::Error::other(
            "the suspended child had no thread to resume",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn classify_exit(status: ExitStatus) -> (Option<i32>, Option<i32>) {
    use std::os::unix::process::ExitStatusExt;
    (status.code(), status.signal())
}

/// Windows has no termination signals: the code the child or the job termination produced
/// is all there is, and `signal` stays empty.
#[cfg(windows)]
fn classify_exit(status: ExitStatus) -> (Option<i32>, Option<i32>) {
    (status.code(), None)
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

    const CATALOG_PATH: &str = if cfg!(windows) {
        r"C:\catalog"
    } else {
        "/catalog"
    };

    fn config() -> NativeConfig {
        NativeConfig {
            profile: "local-274".into(),
            revision: Some("274".into()),
            host: Some("127.0.0.1".into()),
            port: Some(43594),
            engine: None,
            cache: None,
            catalog: PathBuf::from(CATALOG_PATH),
            vault: None,
            lowmem: true,
            mainland: false,
            nav_paints: true,
            exec_core: None,
            exec_pair: None,
            exec_external: None,
            external_ts: None,
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
                CATALOG_PATH,
            ]
        );
        let (options, rest) =
            host_play::parse_profile_args(args.iter().map(String::as_str)).unwrap();
        assert!(
            rest.is_empty(),
            "the panel's live parser would reject these: {rest:?}"
        );
        assert_eq!(options.profile.as_deref(), Some("local-274"));
        assert_eq!(options.catalog_root, Some(PathBuf::from(CATALOG_PATH)));
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
    fn relative_input_paths_are_refused() {
        let mut vaulted = config();
        vaulted.vault = Some(PathBuf::from("relative-vault"));
        let error = vaulted.validate().unwrap_err();
        assert!(
            error.contains("--vault relative-vault is relative"),
            "{error}"
        );
        assert!(error.contains("absolute"), "{error}");

        let mut catalog_config = config();
        catalog_config.catalog = PathBuf::from("relative-catalog");
        let error = catalog_config.validate().unwrap_err();
        assert!(
            error.contains("--catalog relative-catalog is relative"),
            "{error}"
        );

        let mut exec = config();
        exec.exec_core = Some(PathBuf::from("relative-bin"));
        let error = exec.validate().unwrap_err();
        assert!(
            error.contains("--exec-core relative-bin is relative"),
            "{error}"
        );
    }

    /// A real file standing in for a built native executable.
    fn fake_executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n# {name} fixture\n")).unwrap();
        path
    }

    fn test_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-suite-child-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_case_command_is_the_resolved_executable_profile_flags_and_its_live_name() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let dir = test_dir("command");
        let core_bin = fake_executable(&dir, "fixture-core");
        let pair_bin = fake_executable(&dir, "fixture-pair");
        let mut config = config();
        config.catalog = PathBuf::from(manifest.defaults.exec.core.program.clone());
        config.exec_core = Some(core_bin.clone());
        config.exec_pair = Some(pair_bin.clone());
        config.mainland = true;

        let binaries = config
            .binaries(
                [RunnerKind::Core, RunnerKind::Pair],
                &manifest,
                Some(Path::new("/repo")),
            )
            .unwrap();
        let core_canonical = std::fs::canonicalize(&core_bin).unwrap();
        assert_eq!(
            binaries["core"].program,
            core_canonical.display().to_string(),
            "a direct executable is the canonical identity"
        );
        assert!(
            binaries["core"].sha256.is_some(),
            "the launched executable is content-bound"
        );

        let core = config.command(&case("thiever"), &binaries).unwrap();
        assert_eq!(core[0], core_canonical.display().to_string());
        assert_eq!(core[core.len() - 2..], ["--live", "script_thiever"]);
        assert!(!core.iter().any(|arg| arg == "--lowmem"));

        let pair = config
            .command(&case("nature_crafter_air"), &binaries)
            .unwrap();
        assert_eq!(
            pair[0],
            std::fs::canonicalize(&pair_bin)
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(
            pair[pair.len() - 2..],
            ["--live", "script_nature_crafter_air"]
        );
    }

    /// Without a direct executable the manifest's cargo template is only a *reference* to
    /// a built artifact: `command` launches the resolved, hashed path, never `cargo run`
    /// (which could rebuild different bytes under the same command after the identity was
    /// captured), and refuses when no executable was resolved at all.
    #[test]
    fn a_command_launches_the_resolved_artifact_never_the_cargo_template() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let template = &manifest.defaults.exec.core;
        let resolved = BinaryIdentity {
            kind: "resolved-cargo".into(),
            program: "/repo/target/debug/catalog_watch".into(),
            args: template.args.clone(),
            sha256: Some("0".repeat(64)),
            size: Some(1024),
            note: None,
        };
        assert!(!template.args.is_empty(), "{:?}", template.args);
        let binaries: BTreeMap<String, BinaryIdentity> =
            [("core".to_string(), resolved)].into_iter().collect();
        let config = config();

        let command = config.command(&case("thiever"), &binaries).unwrap();
        assert_eq!(
            command[0], "/repo/target/debug/catalog_watch",
            "the launched program is the resolved artifact"
        );
        assert!(
            !command
                .iter()
                .any(|arg| arg == "cargo" || arg == "run" || arg == &template.program),
            "the cargo template must never reach argv: {command:?}"
        );
        assert!(
            !command.windows(2).any(|pair| pair == ["run", "-p"]),
            "cargo's own arguments are identity metadata, not child arguments: {command:?}"
        );
        assert_eq!(command[command.len() - 2..], ["--live", "script_thiever"]);

        // An unresolved identity (a command string, no bytes) is refused.
        let unresolved: BTreeMap<String, BinaryIdentity> = [(
            "core".to_string(),
            BinaryIdentity {
                kind: "unresolved-cargo".into(),
                program: template.program.clone(),
                args: template.args.clone(),
                sha256: None,
                size: None,
                note: Some("cargo run is not an identity".into()),
            },
        )]
        .into_iter()
        .collect();
        let error = config.command(&case("thiever"), &unresolved).unwrap_err();
        assert!(error.contains("no content identity"), "{error}");
        assert!(error.contains("--exec-core"), "{error}");

        // No identity at all for the runner kind is refused too, with the flag that
        // resolves it.
        let error = config
            .command(&case("thiever"), &BTreeMap::new())
            .unwrap_err();
        assert!(error.contains("--exec-core"), "{error}");
        let error = config
            .command(&case("nature_crafter_air"), &BTreeMap::new())
            .unwrap_err();
        assert!(error.contains("--exec-pair"), "{error}");
    }

    /// `binaries` keeps one identity per runner kind and resolves nothing it was not
    /// asked for.
    #[test]
    fn binaries_binds_one_executable_per_runner_kind() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let mut config = config();
        config.exec_core = Some(fake_executable(&test_dir("binaries"), "fixture-core"));

        let binaries = config
            .binaries(
                [RunnerKind::Core, RunnerKind::Core],
                &manifest,
                Some(Path::new("/repo")),
            )
            .unwrap();
        assert_eq!(binaries.len(), 1);
        assert_eq!(
            binaries["core"].program,
            std::fs::canonicalize(config.exec_core.as_ref().unwrap())
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(binaries["core"].kind, "direct");
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

    /// The refused inherited deadline controls are the *actually consumed* ones: the list is
    /// pinned so a later edit cannot grow it into a speculative blanket environment
    /// sanitization (or quietly drop the control that review found). The refusal itself,
    /// including its message, is exercised end to end in `tests/suite_offline.rs`, where the
    /// child's environment is controlled exactly.
    #[test]
    fn the_refused_deadline_environment_is_the_bounded_native_list() {
        assert_eq!(super::DEADLINE_ENV, ["BUDGET_S"]);
        assert!(super::inherited_deadline_env()
            .iter()
            .all(|name| super::DEADLINE_ENV.contains(&name.as_str())));
    }

    /// The loader smoke's argv is the resolved external executable, the typed source override and
    /// the case's own live name — and a raw `--child-arg` cannot re-bind either typed selection.
    #[test]
    fn the_external_argv_carries_the_typed_source_override_and_only_that() {
        let source = if cfg!(windows) {
            PathBuf::from(r"C:\ext\ExampleBot.ts")
        } else {
            PathBuf::from("/ext/ExampleBot.ts")
        };
        let mut config = config();
        config.external_ts = Some(source.clone());
        let mut binaries = BTreeMap::new();
        binaries.insert(
            "external".to_string(),
            BinaryIdentity {
                kind: "direct".into(),
                program: "/bin/external_watch".into(),
                args: Vec::new(),
                sha256: Some("digest".into()),
                size: Some(1),
                note: None,
            },
        );
        let command = config.command(&case("external_loader"), &binaries).unwrap();
        assert_eq!(command[0], "/bin/external_watch");
        assert_eq!(
            command[command.len() - 4..],
            [
                "--live",
                "script_external_loader",
                "--external-ts",
                source.to_str().unwrap()
            ],
            "{command:?}"
        );

        // Without the typed override the child reads its own tracked fixture, which the run
        // identity bound: no flag is invented for it.
        let mut default_config = config.clone();
        default_config.external_ts = None;
        let command = default_config
            .command(&case("external_loader"), &binaries)
            .unwrap();
        assert_eq!(
            command[command.len() - 2..],
            ["--live", "script_external_loader"],
            "{command:?}"
        );

        // A raw extra argument must not re-bind the typed source or the typed live name.
        let mut evasive = config.clone();
        evasive.extra_args = vec!["--external-ts".into(), "/other.ts".into()];
        let error = evasive.validate().unwrap_err();
        assert!(error.contains("--external-ts"), "{error}");
        let mut relive = config.clone();
        relive.extra_args = vec!["--live".into(), "script_thiever".into()];
        let error = relive.validate().unwrap_err();
        assert!(error.contains("--live"), "{error}");
        assert!(
            config.validate().is_err(),
            "the fixture catalog path still fails closed for its own reason"
        );
    }

    /// The external runner resolves its own executable identity: `--exec-external` when given,
    /// otherwise the manifest's own `external_watch` template — never the core one.
    #[test]
    fn the_external_runner_resolves_its_own_executable() {
        let dir = test_dir("external-binary");
        let mut config = config();
        config.exec_external = Some(fake_executable(&dir, "external-watch"));
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let binaries = config
            .binaries([RunnerKind::External], &manifest, Some(Path::new("/repo")))
            .unwrap();
        assert_eq!(binaries.len(), 1);
        assert_eq!(
            binaries["external"].program,
            std::fs::canonicalize(config.exec_external.as_ref().unwrap())
                .unwrap()
                .display()
                .to_string()
        );
        assert!(binaries["external"].resolved());
        assert!(
            !binaries.contains_key("core"),
            "a loader-only selection resolves no catalog executable"
        );

        let error = config
            .binaries([RunnerKind::External], &manifest, None)
            .err();
        assert!(
            error.is_none(),
            "the direct executable resolves without a workspace root: {error:?}"
        );
    }
}
