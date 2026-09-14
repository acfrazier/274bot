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
use super::identity::{self, BinaryIdentity, ProfileIdentity, SettingsIdentity};
use super::ledger::{AttemptEnd, AttemptStatus, CleanupSummary, Ledger, ReceiptSummary, Summary};
use super::manifest::{CaseEntry, SuiteManifest};
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
  --vault PATH --lowmem --mainland
  --exec-core PATH --exec-pair PATH        direct native executables
  --cwd DIR                     working directory for the children
  --child-arg ARG               extra argument appended to every child (repeatable)

`--lowmem` is the default and matches the profile's own setting; `--highmem` is refused
because the panel executables expose no memory flag. `--mainland` is passed to a child as
`BOT_MAINLAND=1`, never as a flag.

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
        exec_core: None,
        exec_pair: None,
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
            "--lowmem" => config.lowmem = true,
            "--highmem" => config.lowmem = false,
            "--mainland" => config.mainland = true,
            "--exec-core" => config.exec_core = Some(PathBuf::from(value("--exec-core")?)),
            "--exec-pair" => config.exec_pair = Some(PathBuf::from(value("--exec-pair")?)),
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

/// Content identity for an engine/cache path: a file is hashed, a directory is digested as
/// a bounded tree (unresolved when it is past the bound).
fn input_digest(path: &Path) -> identity::InputDigest {
    if path.is_file() {
        identity::InputDigest::file(path)
    } else {
        identity::InputDigest::tree(path)
    }
}

/// The vault the panel itself would resolve: `--vault` when given (validated to exist),
/// else the shared default path. An absence is a *defined* identity — the panel creates
/// the file on first use — never an invented one.
fn vault_digest(given: Option<&Path>) -> identity::InputDigest {
    let path = match given {
        Some(path) => path.to_path_buf(),
        None => host_play::default_vault_path(),
    };
    if path.exists() {
        identity::InputDigest::file(&path)
    } else {
        identity::InputDigest::absent(
            &path,
            "no vault file at the resolved path; the panel would create it",
        )
    }
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
    let manifest = load_manifest(args)?;
    let selection = select_cases(args, &manifest)?;
    let planned: Vec<(&CaseEntry, Option<Vec<String>>)> = selection
        .cases
        .iter()
        .filter_map(|id| manifest.case(id))
        .map(|case| {
            let command = if case.is_runnable() && !args.config.catalog.as_os_str().is_empty() {
                args.config.command(case, &manifest).ok()
            } else {
                None
            };
            (case, command)
        })
        .collect();
    if args.json {
        let payload = serde_json::json!({
            "suite_id": manifest.suite_id,
            "why": selection.why,
            "planned": planned
                .iter()
                .map(|(case, command)| serde_json::json!({
                    "id": case.id,
                    "runnable": case.is_runnable(),
                    "budget_min": case.budget_min(),
                    "command": command,
                    "unavailable": case.unavailable.as_ref().map(|u| u.code.clone()),
                }))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return Ok(());
    }
    println!("{}", selection.why);
    for (index, (case, command)) in planned.iter().enumerate() {
        match command {
            Some(command) => println!(
                "[{}/{}] would run {} ({})",
                index + 1,
                planned.len(),
                case.id,
                command.join(" ")
            ),
            None => println!(
                "[{}/{}] would not run {} ({})",
                index + 1,
                planned.len(),
                case.id,
                case.unavailable
                    .as_ref()
                    .map(|unavailable| unavailable.code.as_str())
                    .unwrap_or("no native adapter")
            ),
        }
    }
    if args.config.catalog.as_os_str().is_empty() {
        println!("note: pass --catalog with --profile to print the exact child command lines");
    }
    Ok(())
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

    let repo = identity::repo_root();
    let client = repo.as_deref().and_then(identity::client_root);

    // Binary identity for every runner kind this selection will launch, resolved and
    // hashed *before* the ledger exists: an unresolved executable is a configuration
    // error the run refuses, not an identity the suite records and later compares.
    let mut binaries: BTreeMap<String, BinaryIdentity> = BTreeMap::new();
    for case in selection.runnable(&manifest) {
        let kind = case.runner();
        if !binaries.contains_key(kind.as_str()) {
            binaries.insert(
                kind.as_str().to_string(),
                args.config.binary(kind, &manifest, repo.as_deref())?,
            );
        }
    }

    let settings = SettingsIdentity {
        level: selection.level.map(|level| level.as_str().to_string()),
        only: selection.only.clone(),
        changed_paths: selection.changed.clone(),
        changed_source: format!("{:?}", selection.changed_source).to_lowercase(),
        child_args: args.config.extra_args.clone(),
        child_env_keys: args.config.env_keys(),
    };
    let profile = ProfileIdentity {
        profile: args.config.profile.clone(),
        revision: args.config.revision.clone(),
        host: args.config.host.clone(),
        port: args.config.port,
        engine: args.config.engine.as_ref().map(|path| input_digest(path)),
        cache: args.config.cache.as_ref().map(|path| input_digest(path)),
        catalog: identity::InputDigest::catalog(&args.config.catalog),
        vault: vault_digest(args.config.vault.as_deref()),
        lowmem: args.config.lowmem,
        mainland: args.config.mainland,
        jobs: 1,
    };
    let run_identity = identity::capture(&identity::IdentityInputs {
        manifest_bytes: EMBEDDED_MANIFEST.as_bytes(),
        manifest: &manifest,
        repo_root: repo.clone(),
        client_root: client.clone(),
        binaries: binaries.clone(),
        profile,
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

        let command = args.config.command(case, &manifest)?;
        let spec = ChildSpec {
            label: case.id.clone(),
            command: command.clone(),
            env: args.config.child_env(case, &shots_root, &BTreeMap::new()),
            cwd: args
                .config
                .cwd
                .clone()
                .or_else(|| repo.clone())
                .or_else(|| std::env::current_dir().ok()),
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
            shot_lines: receipts.shot_lines.len(),
        });

        let cleanup_note = run.cleanup.note.clone();
        let cleanup_summary = Some(CleanupSummary {
            killed_signal: run.cleanup.killed_signal,
            escalated_to_sigkill: run.cleanup.escalated_to_sigkill,
            reaped: if run.timed_out || run.interrupted {
                run.cleanup.reaped
            } else {
                true
            },
            note: if run.cleanup.note.is_empty() && !(run.timed_out || run.interrupted) {
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
        } else {
            match receipt::validate(case, &receipts, run.exit_code, &captures) {
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
