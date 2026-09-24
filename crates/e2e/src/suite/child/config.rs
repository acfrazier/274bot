use super::*;
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
    /// Operator-declared WORLD membership for the selected endpoint.
    pub world_members: Option<bool>,
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
    /// shape. Memory mode is a panel front-end flag and is appended by `command`; it is not
    /// included here because the profile resolver must see only profile options.
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
        push(
            "--world-members",
            self.world_members
                .map(|v| if v { "true".into() } else { "false".into() }),
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
        for flag in ["--live", "--external-ts", "--lowmem", "--highmem"] {
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
            if self.lowmem { "--lowmem" } else { "--highmem" }.into(),
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
pub(super) fn supported_arg(_arg: &str) -> bool {
    // No reference harness flag maps to a native executable flag yet: `--no-deploy`,
    // `--minutes`, `--base` and friends belong to the foreign runner. Declaring one here
    // without a native adapter would be inventing support.
    false
}
/// Reference `env` entries are applied only when they name a native host knob the suite
/// deliberately supports.
pub(super) fn typed_env_key(_key: &str) -> bool {
    false
}
