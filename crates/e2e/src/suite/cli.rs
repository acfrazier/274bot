//! `e2e-suite` command line: `list`, `dry-run` and `run`.
//!
//! `list` and `dry-run` never start a service, build, log in, play, or mutate anything:
//! they resolve the manifest, apply the selection, and report. `run` requires an explicit
//! native profile/catalog configuration, creates a fresh durable run directory (or
//! resumes one whose identity and selection are unchanged), launches the selected cases
//! in order, and writes the ledger.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::child::{self, ChildSpec, NativeConfig};
use super::identity::{self, ProfileIdentity, SettingsIdentity};
use super::ledger::{AttemptEnd, AttemptStatus, CleanupSummary, Ledger, ReceiptSummary, Summary};
use super::manifest::{CaseEntry, RunnerKind, SuiteManifest};
use super::receipt::{self, Verdict};
use super::select::{self, Level, SelectRequest, Selection};
use super::{SuiteResult, EMBEDDED_MANIFEST};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_INTERRUPTED: i32 = 130;

const USAGE: &str = "\
usage: e2e-suite <list|dry-run|run> [selection] [config]

selection:
  --level quick|full|smart      level selection (default quick)
  --only SUB[,SUB...]           substring selection (replaces the level; repeatable)
  --changed-path PATH           changed path for smart selection (repeatable)
  --manifest PATH               manifest override (default: the tracked fixture)
  --json                        machine-readable output for list/dry-run

config (dry-run and run):
  --profile NAME                native server profile (required by run)
  --revision REV --host HOST --port PORT
  --engine DIR --cache DIR --catalog DIR   (catalog is required by run)
  --vault PATH --lowmem|--highmem --mainland
  --nav-paints on|off            headed diagnostic paints (default on)
  --exec-core PATH --exec-pair PATH        direct native executables
  --exec-external PATH          direct external_watch executable (loader smoke)
  --external-ts ABS             raw TypeScript input for the loader smoke
  --cwd DIR                     working directory for the children
  --child-arg ARG               extra argument appended to every child (repeatable)

`--lowmem` is the default; the selected mode is passed as a typed panel flag. `--mainland` is passed to a child as
`BOT_MAINLAND=1`, never as a flag. `--external-ts` must be absolute (the panel refuses a
relative raw source) and is bound by path and content when the loader smoke is selected;
a core/pair-only selection never resolves it.

run:
  --run-dir DIR                 durable run directory (required; must be fresh)
  --resume                      continue an existing run directory
  --verbose                     stream child output";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    List,
    DryRun,
    Run,
    Help,
}

#[derive(Debug)]
struct Args {
    command: Command,
    level: Level,
    only: Vec<String>,
    changed: Option<Vec<String>>,
    manifest: Option<PathBuf>,
    json: bool,
    verbose: bool,
    run_dir: Option<PathBuf>,
    resume: bool,
    config: NativeConfig,
}

fn parse_args(argv: &[String]) -> SuiteResult<Args> {
    let mut command = None;
    let mut level = Level::Quick;
    let mut only = Vec::new();
    let mut changed: Option<Vec<String>> = None;
    let mut manifest = None;
    let mut json = false;
    let mut verbose = false;
    let mut run_dir = None;
    let mut resume = false;
    let mut memory_choice = None;
    let mut config = NativeConfig {
        profile: String::new(),
        revision: None,
        host: None,
        port: None,
        engine: None,
        cache: None,
        catalog: PathBuf::new(),
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
    };
    let mut catalog_given = false;

    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| -> SuiteResult<String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "list" | "dry-run" | "run" => {
                if command.is_some() {
                    return Err("exactly one command (list, dry-run, run) is allowed".into());
                }
                command = Some(match arg.as_str() {
                    "list" => Command::List,
                    "dry-run" => Command::DryRun,
                    _ => Command::Run,
                });
            }
            "--help" | "-h" => command = Some(Command::Help),
            "--level" => level = Level::parse(&value("--level")?)?,
            "--only" => only.extend(
                value("--only")?
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .filter(|part| !part.is_empty()),
            ),
            "--changed-path" => changed
                .get_or_insert_with(Vec::new)
                .push(value("--changed-path")?),
            "--changed-paths-file" => {
                let path = value("--changed-paths-file")?;
                let text = std::fs::read_to_string(&path)
                    .map_err(|error| format!("--changed-paths-file {path}: {error}"))?;
                let list = changed.get_or_insert_with(Vec::new);
                list.extend(
                    text.lines()
                        .map(|line| line.trim().to_string())
                        .filter(|line| !line.is_empty()),
                );
            }
            "--manifest" => manifest = Some(PathBuf::from(value("--manifest")?)),
            "--json" => json = true,
            "--verbose" | "-v" => verbose = true,
            "--run-dir" => run_dir = Some(PathBuf::from(value("--run-dir")?)),
            "--resume" => resume = true,
            "--nav-paints" => {
                config.nav_paints = match value("--nav-paints")?.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("--nav-paints expects on or off".into()),
                };
            }
            "--profile" => config.profile = value("--profile")?,
            "--revision" => config.revision = Some(value("--revision")?),
            "--host" => config.host = Some(value("--host")?),
            "--port" => {
                let port = value("--port")?;
                config.port = Some(
                    port.parse::<u16>()
                        .ok()
                        .filter(|port| *port != 0)
                        .ok_or_else(|| "--port needs a port from 1 to 65535".to_string())?,
                );
            }
            "--engine" => config.engine = Some(PathBuf::from(value("--engine")?)),
            "--cache" => config.cache = Some(PathBuf::from(value("--cache")?)),
            "--catalog" => {
                config.catalog = PathBuf::from(value("--catalog")?);
                catalog_given = true;
            }
            "--vault" => config.vault = Some(PathBuf::from(value("--vault")?)),
            "--lowmem" | "--highmem" => {
                let requested = arg == "--lowmem";
                if memory_choice.is_some_and(|current| current != requested) {
                    return Err("--lowmem and --highmem conflict".into());
                }
                memory_choice = Some(requested);
                config.lowmem = requested;
            }
            "--mainland" => config.mainland = true,
            "--exec-core" => config.exec_core = Some(PathBuf::from(value("--exec-core")?)),
            "--exec-pair" => config.exec_pair = Some(PathBuf::from(value("--exec-pair")?)),
            "--exec-external" => {
                config.exec_external = Some(PathBuf::from(value("--exec-external")?))
            }
            "--external-ts" => {
                let raw = value("--external-ts")?;
                let path = PathBuf::from(&raw);
                // The same rule the panel applies to its own flag: a relative raw source would be
                // resolved against the child's working directory, which the suite cannot bind.
                if !path.is_absolute() {
                    return Err(format!(
                        "--external-ts {raw} is relative; the panel refuses a relative raw source, \
                         so pass an absolute path"
                    ));
                }
                config.external_ts = Some(path);
            }
            "--cwd" => config.cwd = Some(PathBuf::from(value("--cwd")?)),
            "--child-arg" => config.extra_args.push(value("--child-arg")?),
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    let command = command.ok_or_else(|| format!("no command given\n{USAGE}"))?;
    if command == Command::DryRun || command == Command::Run {
        if let Some(paths) = &changed {
            if paths.is_empty() {
                return Err("--changed-path needs at least one path".into());
            }
        }
    }
    if command == Command::Run && !catalog_given {
        return Err("run requires --catalog DIR (the native catalog clone root)".into());
    }
    Ok(Args {
        command,
        level,
        only,
        changed,
        manifest,
        json,
        verbose,
        run_dir,
        resume,
        config,
    })
}

/// Entry point used by the binary. Returns the process exit code.
pub fn main<I: IntoIterator<Item = String>>(argv: I) -> i32 {
    let argv: Vec<String> = argv.into_iter().collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("e2e-suite: {error}");
            return EXIT_USAGE;
        }
    };
    match args.command {
        Command::Help => {
            println!("{USAGE}");
            EXIT_OK
        }
        Command::List => match list(&args) {
            Ok(()) => EXIT_OK,
            Err(error) => {
                eprintln!("e2e-suite: {error}");
                EXIT_USAGE
            }
        },
        Command::DryRun => match dry_run(&args) {
            Ok(()) => EXIT_OK,
            Err(error) => {
                eprintln!("e2e-suite: {error}");
                EXIT_USAGE
            }
        },
        Command::Run => match run(&args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("e2e-suite: {error}");
                EXIT_USAGE
            }
        },
    }
}

fn load_manifest(args: &Args) -> SuiteResult<SuiteManifest> {
    match &args.manifest {
        Some(path) => SuiteManifest::load(path),
        None => SuiteManifest::parse(EMBEDDED_MANIFEST.as_bytes(), "embedded fixture"),
    }
}

fn select_cases(args: &Args, manifest: &SuiteManifest) -> SuiteResult<Selection> {
    select::select(
        manifest,
        &SelectRequest {
            level: args.level,
            only: &args.only,
            changed_paths: args.changed.as_deref(),
            repo_root: identity::repo_root(),
        },
    )
}

/// Inherited env names that change what `ProfileOptions::resolve_with_env` selects.
/// Values are not serialized; the resolved paths and their content digests are.
const BOUND_PROFILE_ENV: &[&str] = &[
    "BOT_SERVER_PROFILE",
    "BOT_REVISION",
    "BOT_TARGET",
    "ENGINE_DIR",
    "CLIENT_UNPACK_DIR",
    "NAV_PACK",
    "NAV_FLAGS",
    "BOT_CACHE_MANIFEST",
    "RS2B0T",
];

fn launch_profile_env(cwd: &Path) -> SuiteResult<host_play::profile::ProfileEnvironment> {
    if std::env::var_os("LOGIN_RSAN")
        .filter(|value| !value.is_empty())
        .is_some()
        || std::env::var_os("LOGIN_RSAE")
            .filter(|value| !value.is_empty())
            .is_some()
    {
        return Err(
            "LOGIN_RSAN/LOGIN_RSAE are set; the suite does not record credentials and will not \
             bind an env-supplied RSA key. Unset them so the engine pem is the identity"
                .into(),
        );
    }
    let mut env = host_play::profile::ProfileEnvironment::capture();
    env.working_dir = Some(cwd.to_path_buf());
    Ok(env)
}

/// Parse the argv the child will actually receive: profile flags then extra_args.
fn effective_profile_options(
    config: &NativeConfig,
) -> SuiteResult<(host_play::ProfileOptions, Vec<String>)> {
    let mut argv = config.profile_args();
    argv.extend(config.extra_args.iter().cloned());
    host_play::parse_profile_args(argv).map_err(|error| format!("child argv: {error}"))
}

fn settings_env_keys(config: &NativeConfig) -> Vec<String> {
    let mut keys = config.env_keys();
    for name in BOUND_PROFILE_ENV {
        if std::env::var_os(name).is_some_and(|value| !value.is_empty()) {
            keys.push((*name).to_string());
        }
    }
    keys
}

/// The profile inputs the *child* itself resolves, from the effective argv, cwd and env.
fn bind_profile(config: &NativeConfig, cwd: &Path) -> SuiteResult<ProfileIdentity> {
    let env = launch_profile_env(cwd)?;
    let (options, _rest) = effective_profile_options(config)?;
    let selection = options.resolve_with_env(None, &env).map_err(|error| {
        format!(
            "the native profile resolver rejects this configuration ({error}); the suite binds the inputs \
             the child resolves, so it cannot substitute a different selection"
        )
    })?;
    let revision = selection.revision().as_i32() as u16;
    let unpack_overridden = options.unpack_dir.is_some() || env.unpack_dir.is_some();
    let engine_dir = selection.engine_dir();
    let catalog_path = selection
        .catalog_root()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| config.catalog.clone());
    let resolved = identity::ResolvedInputs {
        selection: selection.selection().name().to_string(),
        cache: selection.cache_dir().display().to_string(),
        vault: selection.vault_path().display().to_string(),
        nav_pack: selection.nav_pack().display().to_string(),
        nav_flags: selection.nav_flags().display().to_string(),
        content: selection.content_dir().display().to_string(),
        unpack: selection.unpack_dir().display().to_string(),
    };
    Ok(ProfileIdentity {
        profile: options
            .profile
            .clone()
            .unwrap_or_else(|| config.profile.clone()),
        revision: options.revision.clone().or_else(|| config.revision.clone()),
        host: options.host.clone().or_else(|| config.host.clone()),
        port: options.port.or(config.port),
        selection: resolved.selection.clone(),
        resolved: resolved.clone(),
        engine: Some(identity::bind_engine_pem(engine_dir)),
        cache: identity::bind_cache(Path::new(&resolved.cache), revision),
        catalog: identity::InputDigest::catalog(&catalog_path),
        vault: identity::bind_input(Path::new(&resolved.vault)),
        nav_pack: identity::bind_input(Path::new(&resolved.nav_pack)),
        nav_flags: identity::bind_input(Path::new(&resolved.nav_flags)),
        content: identity::bind_content(Path::new(&resolved.content)),
        unpack: identity::bind_unpack(Path::new(&resolved.unpack), revision, unpack_overridden),
        lowmem: config.lowmem,
        mainland: config.mainland,
        jobs: 1,
    })
}

/// Bind the external loader source when — and only when — the selection launches the loader smoke.
///
/// An ordinary core/pair selection never resolves the external fixture: a run with no external
/// case must not fail because an external resource is missing, and its identity must not carry an
/// input it never read. When the loader smoke *is* selected, the source is bound by path and by
/// content, so a same-path byte change refuses a resume before any spawn.
fn bind_external(
    manifest: &SuiteManifest,
    selection: &Selection,
    config: &NativeConfig,
) -> SuiteResult<Option<identity::ExternalSource>> {
    let launches_external = selection
        .runnable(manifest)
        .iter()
        .any(|case| case.runner() == RunnerKind::External);
    if !launches_external {
        return Ok(None);
    }
    identity::ExternalSource::resolve(config.external_ts.as_deref()).map(Some)
}

fn list(args: &Args) -> SuiteResult<()> {
    let manifest = load_manifest(args)?;
    let selection = select_cases(args, &manifest)?;
    if args.json {
        let payload = serde_json::json!({
            "suite_id": manifest.suite_id,
            "reference_commit": manifest.provenance.reference_commit,
            "level": selection.level.map(|level| level.as_str()),
            "only": selection.only,
            "why": selection.why,
            "changed_paths": selection.changed,
            "cases": selection.cases,
            "unavailable": selection
                .unavailable
                .iter()
                .map(|(id, reason)| serde_json::json!({"id": id, "reason": reason}))
                .collect::<Vec<_>>(),
            "counts": manifest.status_counts(),
            "intended_scripts": manifest.intended_scripts,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return Ok(());
    }
    let counts = manifest.status_counts();
    let summary = counts
        .iter()
        .map(|(status, count)| format!("{count} {status}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "{} case(s) in {}: {summary}",
        manifest.cases.len(),
        manifest.suite_id
    );
    println!("{}", selection.why);
    for id in &selection.cases {
        let Some(case) = manifest.case(id) else {
            continue;
        };
        let covers = match case.kind {
            super::manifest::CaseKind::Native => match case.runner() {
                super::manifest::RunnerKind::Core => format!(
                    "core {} ({})",
                    case.scenario.as_deref().unwrap_or("-"),
                    case.core_case.as_deref().unwrap_or("-")
                ),
                super::manifest::RunnerKind::Pair => format!(
                    "pair {} ({})",
                    case.scenario.as_deref().unwrap_or("-"),
                    case.pair_case.as_deref().unwrap_or("-")
                ),
                super::manifest::RunnerKind::External => format!(
                    "external {} ({})",
                    case.live.as_deref().unwrap_or("-"),
                    host_play::external_loader::TERMINAL_SHOT
                ),
            },
            super::manifest::CaseKind::Reference => {
                format!("reference {}", case.harness.as_deref().unwrap_or("-"))
            }
        };
        let status = format!("{:?}", case.selection_status()).to_lowercase();
        let marker = if case.is_runnable() { " " } else { "!" };
        println!(
            "  {marker}{status:<10} {:<28} {covers}  budget={}m",
            case.id,
            case.budget_min()
        );
    }
    if !selection.unavailable.is_empty() {
        println!(
            "{} selected case(s) cannot execute:",
            selection.unavailable.len()
        );
        for (id, reason) in &selection.unavailable {
            println!("  unavailable {id}: {reason}");
        }
    }
    Ok(())
}

fn dry_run(args: &Args) -> SuiteResult<()> {
    // A configuration the run refuses is not planned as a runnable one: the suite does not
    // hand a child an inner deadline it did not record (see `child::validate_deadline_env`).
    child::validate_deadline_env()?;
    let manifest = load_manifest(args)?;
    let selection = select_cases(args, &manifest)?;
    let print_commands = !args.config.catalog.as_os_str().is_empty();
    // The printed line is the line a child would receive: the executable resolved and
    // hashed into the run identity, never the manifest's cargo template. A plan whose
    // executable cannot be resolved refuses instead of printing a line the run would
    // then reject.
    let mut unresolved: Option<String> = None;
    let binaries = if print_commands {
        match args.config.binaries(
            selection
                .runnable(&manifest)
                .into_iter()
                .map(|case| case.runner()),
            &manifest,
            identity::repo_root().as_deref(),
        ) {
            Ok(binaries) => binaries,
            Err(error) => {
                unresolved = Some(error);
                BTreeMap::new()
            }
        }
    } else {
        BTreeMap::new()
    };
    // A plan whose external source cannot be bound refuses as well: the printed line names a path
    // the run would reject, so it must not be presented as a plan.
    if print_commands && unresolved.is_none() {
        if let Err(error) = bind_external(&manifest, &selection, &args.config) {
            unresolved = Some(error);
        }
    }
    let planned: Vec<(&CaseEntry, Result<Vec<String>, String>)> = selection
        .cases
        .iter()
        .filter_map(|id| manifest.case(id))
        .map(|case| {
            let outcome = if !case.is_runnable() {
                Err(case
                    .unavailable
                    .as_ref()
                    .map(|unavailable| format!("{}: {}", unavailable.code, unavailable.reason))
                    .unwrap_or_else(|| "no native adapter".into()))
            } else if !print_commands {
                Err("pass --catalog with --profile to print the exact child command lines".into())
            } else {
                args.config.command(case, &binaries)
            };
            (case, outcome)
        })
        .collect();
    if args.json {
        let payload = serde_json::json!({
            "suite_id": manifest.suite_id,
            "why": selection.why,
            "planned": planned
                .iter()
                .map(|(case, outcome)| serde_json::json!({
                    "id": case.id,
                    "runnable": case.is_runnable(),
                    "budget_min": case.budget_min(),
                    "command": outcome.as_ref().ok(),
                    "unavailable": outcome.as_ref().err(),
                }))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return refuse_unresolved_plan(unresolved);
    }
    println!("{}", selection.why);
    for (index, (case, outcome)) in planned.iter().enumerate() {
        match outcome {
            Ok(command) => println!(
                "[{}/{}] would run {} ({})",
                index + 1,
                planned.len(),
                case.id,
                command.join(" ")
            ),
            Err(reason) => println!(
                "[{}/{}] would not run {} ({})",
                index + 1,
                planned.len(),
                case.id,
                reason
            ),
        }
    }
    if !print_commands {
        println!("note: pass --catalog with --profile to print the exact child command lines");
    }
    refuse_unresolved_plan(unresolved)
}

/// A plan that could not resolve the executable it would launch refuses: the suite binds
/// the bytes of every child it starts, so an unresolvable plan is a configuration error,
/// not a preview of a run that would start something else.
fn refuse_unresolved_plan(unresolved: Option<String>) -> SuiteResult<()> {
    match unresolved {
        Some(error) => Err(format!("refusing to plan a run: {error}")),
        None => Ok(()),
    }
}

fn run(args: &Args) -> SuiteResult<i32> {
    let manifest = load_manifest(args)?;
    let selection = select_cases(args, &manifest)?;
    if selection.cases.is_empty() {
        return Err(format!("nothing selected: {}", selection.why));
    }
    let run_dir = args
        .run_dir
        .clone()
        .ok_or_else(|| "run requires --run-dir DIR".to_string())?;
    args.config.validate()?;
    // Before binary resolution, the identity, the ledger and any resume check: an inherited
    // native deadline control would move the child's inner deadline behind the recorded
    // budget, so the run (and a resume) refuses it with no child launched.
    child::validate_deadline_env()?;

    let repo = identity::repo_root();
    let client = repo.as_deref().and_then(identity::client_root);

    // Binary identity for every runner kind this selection will launch, resolved and
    // hashed *before* the ledger exists: an unresolved executable is a configuration
    // error the run refuses, not an identity the suite records and later compares. The
    // same map is what every child is launched from, so the bytes in the ledger and the
    // bytes in the child are one executable — `cargo run` is never re-invoked after the
    // identity was captured.
    let binaries = args.config.binaries(
        selection
            .runnable(&manifest)
            .into_iter()
            .map(|case| case.runner()),
        &manifest,
        repo.as_deref(),
    )?;

    let cwd = args.config.launch_cwd(repo.as_deref())?;
    // Bound only for a selection that launches the loader smoke: an ordinary core/pair run must
    // not resolve (or require) an external resource it never reads.
    let external = bind_external(&manifest, &selection, &args.config)?;
    let settings = SettingsIdentity {
        level: selection.level.map(|level| level.as_str().to_string()),
        only: selection.only.clone(),
        changed_paths: selection.changed.clone(),
        changed_source: format!("{:?}", selection.changed_source).to_lowercase(),
        child_args: args.config.extra_args.clone(),
        nav_paints: args.config.nav_paints,
        child_env_keys: settings_env_keys(&args.config),
        cwd: cwd.display().to_string(),
    };
    let profile = bind_profile(&args.config, &cwd)?;
    let run_identity = identity::capture(&identity::IdentityInputs {
        manifest_bytes: EMBEDDED_MANIFEST.as_bytes(),
        manifest: &manifest,
        repo_root: repo.clone(),
        client_root: client.clone(),
        binaries: binaries.clone(),
        profile,
        external: external.clone(),
        settings,
        selection: &selection.cases,
    });
    let manifest_identity = if let Some(path) = &args.manifest {
        // A --manifest override must bind its own bytes, not the embedded fixture.
        let bytes =
            std::fs::read(path).map_err(|error| format!("manifest {}: {error}", path.display()))?;
        let mut identity = run_identity.clone();
        identity.manifest_sha256 = identity::sha256(&bytes);
        identity
    } else {
        run_identity
    };

    // Fail closed before the ledger exists: a run that could not bind an input could not
    // detect that the input changed, and a resume of such a run proves nothing.
    let unresolved = manifest_identity.unresolved();
    if !unresolved.is_empty() {
        return Err(format!(
            "refusing to run: the suite cannot bind {}; resolve the input (a built executable, a \
             catalog with src/bot/scripts, a readable repository) or pass it explicitly",
            unresolved.join(", ")
        ));
    }

    let mut ledger = if args.resume {
        let ledger = Ledger::resume(&run_dir)?;
        ledger.check_resume(&manifest_identity, &selection.cases)?;
        ledger
    } else {
        Ledger::create(
            &run_dir,
            &manifest.suite_id,
            &manifest_identity,
            &selection.cases,
        )?
    };

    println!(
        "e2e-suite run: {} case(s) selected — {}",
        selection.cases.len(),
        selection.why
    );
    println!("identity: {}", identity::summary_value(&manifest_identity));
    if let Some(source) = &external {
        println!("external loader source: {}", source.summary());
    }
    if !selection.unavailable.is_empty() {
        println!(
            "{} selected case(s) have no native adapter and will be recorded unavailable",
            selection.unavailable.len()
        );
    }

    let shots_root = ledger.shots_root();
    let mut stop_reason: Option<String> = None;
    let mut interrupted = false;

    for (index, id) in selection.cases.iter().enumerate() {
        let Some(case) = manifest.case(id) else {
            continue;
        };
        let position = index + 1;
        if let Some(attempt) = ledger.attempt(id) {
            println!(
                "[{position}/{}] {id} -> skipped ({}) — retained from an earlier run, never retried",
                selection.cases.len(),
                attempt.status.as_str()
            );
            continue;
        }
        if !case.is_runnable() {
            let reason = case
                .unavailable
                .as_ref()
                .map(|unavailable| format!("{}: {}", unavailable.code, unavailable.reason))
                .unwrap_or_else(|| "no native adapter".into());
            println!(
                "[{position}/{}] {id} -> UNAVAILABLE: {reason}",
                selection.cases.len()
            );
            ledger.record_unavailable(id, &reason)?;
            continue;
        }

        let command = args.config.command(case, &binaries)?;
        let spec = ChildSpec {
            label: case.id.clone(),
            command: command.clone(),
            env: args.config.child_env(case, &shots_root, &BTreeMap::new()),
            cwd: Some(cwd.clone()),
        };
        let budget = Duration::from_secs(case.budget_min() as u64 * 60);
        let log_path = ledger.log_path(id);
        let requested = format!(
            "{}:{}:budget={}s",
            case.runner().as_str(),
            case.live.as_deref().unwrap_or("-"),
            budget.as_secs()
        );
        let case_identity = serde_json::json!({
            "id": case.id,
            "script": case.script,
            "script_key": case.script_key,
            "scenario": case.scenario,
            "runner": case.runner().as_str(),
            "core_case": case.core_case,
            "pair_case": case.pair_case,
            // The loader smoke's bound input, recorded on the case row as well as in the run
            // identity: what this case read, and whether it is the producer's frozen fixture.
            "external": match (case.runner(), &external) {
                (RunnerKind::External, Some(source)) => serde_json::json!({
                    "path": source.path,
                    "sha256": source.sha256,
                    "bytes": source.bytes,
                    "default_fixture": source.default_fixture,
                    "frozen_match": source.frozen_match,
                    "harmless_whitespace_sha256": source.harmless_whitespace_sha256,
                }),
                _ => serde_json::Value::Null,
            },
            "reference_cases": case.reference_cases.iter().map(|reference| serde_json::json!({
                "id": reference.id,
                "status": format!("{:?}", reference.status).to_lowercase(),
                "budget_min": reference.budget_min,
            })).collect::<Vec<_>>(),
        });
        ledger.start_attempt(
            id,
            requested,
            &command,
            &spec.env.keys().cloned().collect::<Vec<_>>(),
            case_identity,
            Some(format!(
                "{}/{}",
                super::ledger::LOGS_DIR,
                super::ledger::safe_file_name(id) + ".log"
            )),
        )?;
        println!(
            "[{position}/{}] {id} START ({}m budget)",
            selection.cases.len(),
            case.budget_min()
        );

        let before = receipt::scan_captures(&shots_root);
        let run = child::run(&spec, budget, &log_path, args.verbose)?;
        // Attribution is by *new* files only: a same-named capture written by an earlier
        // case or an earlier run is not this case's evidence.
        let after = receipt::scan_captures(&shots_root);
        let captures = receipt::new_captures(&before, &after);
        let receipts = receipt::parse(&run.output_tail);
        let receipt_summary = Some(ReceiptSummary {
            scenario: receipts
                .scenario_pass
                .as_ref()
                .or(receipts.scenario_fail.as_ref())
                .map(|receipt| receipt.line.clone()),
            catalog_core: receipts.core.as_ref().map(|receipt| receipt.line.clone()),
            paired_core: receipts.pair.as_ref().map(|receipt| receipt.line.clone()),
            external: receipts
                .external
                .as_ref()
                .map(|receipt| receipt.line.clone()),
            shot_lines: receipts.shot_lines.len(),
        });

        let cleanup_note = run.cleanup.note.clone();
        let cleanup_summary = Some(CleanupSummary {
            killed_signal: run.cleanup.killed_signal,
            escalated_to_sigkill: run.cleanup.escalated_to_sigkill,
            // Faithful on every path: `reaped` means the direct child was waited *and* no
            // process of its group survives. A normal exit that left a descendant running
            // is not a reaped tree, so it is not rounded up to `true` here.
            reaped: run.cleanup.reaped,
            note: if run.cleanup.note.is_empty() {
                "child exited on its own".to_string()
            } else {
                run.cleanup.note.clone()
            },
        });

        let (status, reason, shared) = if run.interrupted {
            (
                AttemptStatus::Interrupted,
                "interrupted by the operator; the owned process tree was terminated".to_string(),
                true,
            )
        } else if run.timed_out {
            let reaped = run.cleanup.reaped;
            (
                if reaped {
                    AttemptStatus::Timeout
                } else {
                    AttemptStatus::CleanupFailed
                },
                format!(
                    "budget of {}m expired; {}",
                    case.budget_min(),
                    if cleanup_note.is_empty() {
                        "process tree reaped".to_string()
                    } else {
                        cleanup_note.clone()
                    }
                ),
                !reaped,
            )
        } else if let Some(reason) = cleanup_failure(&run) {
            // A normal exit is not a reaped tree: a descendant that outlived the direct
            // child, and could not be terminated inside the suite's bound, is a harness
            // failure that stops the run instead of launching the next case.
            (AttemptStatus::CleanupFailed, reason, true)
        } else {
            match receipt::validate(case, &receipts, run.exit_code, &captures, external.as_ref()) {
                Verdict::Passed => (AttemptStatus::Passed, String::new(), false),
                Verdict::PendingVisualReview { captures: recorded } => (
                    AttemptStatus::PendingVisualReview,
                    if recorded.is_empty() {
                        "functional receipts verified; the result stays pending visual review \
                         (no capture is contracted for this case)"
                            .to_string()
                    } else {
                        format!(
                            "{} capture(s) recorded; a human readback is still required",
                            recorded.len()
                        )
                    },
                    false,
                ),
                Verdict::CaseFailure { reason } => (AttemptStatus::Failed, reason, false),
                Verdict::SharedFailure { kind, reason } => (
                    AttemptStatus::SharedFailure,
                    format!("{kind}: {reason}"),
                    true,
                ),
            }
        };
        let completed = format!(
            "{}:{}:exit={}:receipts={}{}",
            case.runner().as_str(),
            case.live.as_deref().unwrap_or("-"),
            run.exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| format!("signal:{}", run.signal.unwrap_or(-1))),
            receipts
                .scenario_pass
                .as_ref()
                .map(|_| "scenario")
                .unwrap_or("-"),
            if receipts.core.is_some() {
                "+catalog_core"
            } else if receipts.pair.is_some() {
                "+paired_core"
            } else if receipts.external.is_some() {
                "+external_loader"
            } else {
                ""
            }
        );
        ledger.finish_attempt(
            id,
            AttemptEnd {
                status,
                reason: reason.clone(),
                completed_operation: Some(completed),
                exit_code: run.exit_code,
                signal: run.signal,
                receipts: receipt_summary,
                captures,
                elapsed_ms: run.elapsed_ms,
                cleanup: cleanup_summary,
                log_truncated: run.log_truncated,
            },
        )?;
        println!(
            "[{position}/{}] {id} -> {} ({:.1}s){}",
            selection.cases.len(),
            status.as_str(),
            run.elapsed_ms as f64 / 1000.0,
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        );
        if shared {
            stop_reason = Some(reason);
            interrupted = run.interrupted;
            break;
        }
    }

    if stop_reason.is_some() {
        ledger.set_stop_reason(stop_reason.clone())?;
    }
    let summary = ledger.summarize()?;
    report(&run_dir, &summary, &selection);

    if interrupted {
        return Ok(EXIT_INTERRUPTED);
    }
    let only_requested_unavailable = !selection.only.is_empty()
        && (!selection.unavailable.is_empty() || selection.cases.is_empty());
    if !summary.failures.is_empty()
        || summary.carried_unsuccessful > 0
        || summary.not_reached > 0
        || stop_reason.is_some()
        || only_requested_unavailable
    {
        return Ok(EXIT_FAILURE);
    }
    Ok(EXIT_OK)
}

/// A run whose owned process tree could not be reaped is a shared harness failure: the
/// suite cannot claim the tree is gone, so it stops instead of launching the next case.
/// `reaped` is only true once the direct child has been waited *and* the owned tree holds
/// no process *and* the pipes reached EOF, on every exit path — not only on a timeout.
fn cleanup_failure(run: &child::ChildRun) -> Option<String> {
    if run.cleanup.reaped {
        return None;
    }
    Some(format!(
        "the child's owned process tree could not be reaped within the suite's bound: {}",
        if run.cleanup.note.is_empty() {
            "no cleanup note"
        } else {
            run.cleanup.note.as_str()
        }
    ))
}

fn report(run_dir: &Path, summary: &Summary, selection: &Selection) {
    println!(
        "attempted {} passed {} pending_visual_review {} failed {} timeout {} shared_failure {} \
         unavailable {} skipped_existing {} carried_success {} carried_unsuccessful {} not_reached {}",
        summary.attempted,
        summary.passed,
        summary.pending_visual_review,
        summary.failed,
        summary.timeout,
        summary.shared_failure,
        summary.unavailable,
        summary.skipped_existing,
        summary.carried_success,
        summary.carried_unsuccessful,
        summary.not_reached,
    );
    if let Some(reason) = &summary.stop_reason {
        println!("STOP: {reason}");
    }
    if !summary.failures.is_empty() {
        println!("{} unsuccessful case(s):", summary.failures.len());
        for failure in &summary.failures {
            println!(
                "  {} {}: {}",
                failure.status.as_str(),
                failure.case_id,
                failure.reason
            );
        }
    }
    if summary.unavailable > 0 && !selection.unavailable.is_empty() {
        println!("unavailable (explicit reasons, nothing substituted):");
        for (id, reason) in &selection.unavailable {
            println!("  {id}: {reason}");
        }
    }
    println!(
        "ledger: {}",
        run_dir.join(super::ledger::STATE_FILE).display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_parsing_rejects_unknown_flags_and_missing_values() {
        let error = parse_args(&["run".to_string(), "--nope".to_string()]).unwrap_err();
        assert!(error.contains("unknown flag"), "{error}");
        let error = parse_args(&["run".to_string(), "--profile".to_string()]).unwrap_err();
        assert!(error.contains("--profile needs a value"), "{error}");
        let error = parse_args(&["run".to_string()]).unwrap_err();
        assert!(error.contains("--catalog"), "{error}");
        let error = parse_args(&[]).unwrap_err();
        assert!(error.contains("no command"), "{error}");
        let error = parse_args(&[String::new()]).unwrap_err();
        assert!(error.contains("unknown argument"), "{error}");
        let args = parse_args(&[
            "run".to_string(),
            "--only".to_string(),
            "thiever,ardy".to_string(),
            "--catalog".to_string(),
            "/tmp".to_string(),
            "--profile".to_string(),
            "local-274".to_string(),
        ])
        .unwrap();
        assert_eq!(args.only, vec!["thiever", "ardy"]);
        assert_eq!(args.config.port, None);
        let error = parse_args(&[
            "run".to_string(),
            "--lowmem".to_string(),
            "--highmem".to_string(),
        ])
        .unwrap_err();
        assert!(error.contains("conflict"), "{error}");
    }

    /// A cleanup that could not reap the process tree is a shared failure on *every* exit
    /// path, not only on a timeout: the suite cannot claim the child is gone.
    #[test]
    fn a_cleanup_that_could_not_reap_the_tree_stops_the_run() {
        let run = |reaped: bool, note: &str| child::ChildRun {
            pid: 1,
            exit_code: Some(0),
            signal: None,
            timed_out: false,
            interrupted: false,
            output_tail: String::new(),
            log_truncated: false,
            elapsed_ms: 0,
            cleanup: child::CleanupSummary {
                killed_signal: None,
                escalated_to_sigkill: false,
                reaped,
                note: note.into(),
            },
        };
        assert!(cleanup_failure(&run(true, "child exited on its own")).is_none());
        let reason = cleanup_failure(&run(false, "a descendant outlived the direct child"))
            .expect("a surviving tree is a harness failure");
        assert!(reason.contains("could not be reaped"), "{reason}");
        assert!(reason.contains("descendant"), "{reason}");
    }

    #[test]
    fn list_and_dry_run_do_not_touch_a_run_directory() {
        let dir = std::env::temp_dir().join(format!("274bot-suite-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for command in ["list", "dry-run"] {
            let code = main([
                command.to_string(),
                "--level".to_string(),
                "quick".to_string(),
                "--only".to_string(),
                "thiever".to_string(),
                "--run-dir".to_string(),
                dir.display().to_string(),
            ]);
            assert_eq!(code, EXIT_OK, "{command}");
            assert!(!dir.exists(), "{command} must not create the run directory");
        }
    }

    #[test]
    fn run_without_a_run_dir_or_with_an_empty_selection_fails_before_launching() {
        let code = main([
            "run".to_string(),
            "--only".to_string(),
            "thiever".to_string(),
            "--catalog".to_string(),
            std::env::temp_dir().display().to_string(),
            "--profile".to_string(),
            "local-274".to_string(),
        ]);
        assert_eq!(code, EXIT_USAGE, "a run without --run-dir is a usage error");

        let dir = std::env::temp_dir().join(format!("274bot-suite-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let code = main([
            "run".to_string(),
            "--only".to_string(),
            "nothing-matches-this".to_string(),
            "--catalog".to_string(),
            std::env::temp_dir().display().to_string(),
            "--profile".to_string(),
            "local-274".to_string(),
            "--run-dir".to_string(),
            dir.display().to_string(),
        ]);
        assert_eq!(code, EXIT_USAGE, "an empty selection is a request error");
    }
}
