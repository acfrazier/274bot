//! The shipped preferences maintenance commands (T5.3 §5.2, §5.3, §5.5):
//!
//! ```text
//! panel-play state preferences export --same-root
//! panel-play state preferences recover --source "/absolute/path/private-valid.json"
//! panel-play state preferences recover --defaults
//! ```
//!
//! (and the same three for `tui-play`). Each takes the root-use lease
//! exclusive without waiting, so it never runs beside a user of the root,
//! and the preferences guard; it does its work and exits. It opens no
//! window, terminal session or game slot, and never starts a normal
//! preferences session.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use api::host_log;
use api::hostlog::{Category, Level};
use api::local_state::StoreVersion;
use serde_json::Map;

use super::bank;
use super::files::{self, StateRoot};
use super::root_use::RootUse;
use super::sql::{self, Frontend, Legacy, SqlFailure};

/// One maintenance command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceAction {
    /// `state preferences export --same-root`: write the latest durable
    /// preferences as `panel-ui.json` in the same root, for 0.2.9.
    ExportSameRoot,
    /// `state preferences recover --source <path>`: preserve the database
    /// set, then start a new preference history from a private, valid JSON
    /// object at an absolute path.
    RecoverSource(PathBuf),
    /// `state preferences recover --defaults`: preserve the database set,
    /// then start a new preference history with default preferences.
    RecoverDefaults,
    /// `state bank-hints accept-projection <profile>/<account>/<revision>/<content>`:
    /// after the operator confirms, clear the read-only refusal of an
    /// imported bank hint whose file held fields this build does not know,
    /// so later saves hold the usable rows only.
    AcceptBankProjection(String),
}

/// Why a maintenance command did nothing (or stopped), in the operator's
/// words, with the remedy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MaintenanceError(String);

impl MaintenanceError {
    /// The full message.
    pub fn message(&self) -> &str {
        &self.0
    }
}

/// The maintenance help, for `{bin} state preferences --help`.
pub fn maintenance_help(frontend: Frontend) -> String {
    let bin = frontend.binary();
    format!(
        "usage:\n  {bin} state preferences export --same-root\n  {bin} state preferences recover \
         --source \"/absolute/path/private-valid.json\"\n  {bin} state preferences recover \
         --defaults\n  {bin} state bank-hints accept-projection \
         <profile>/<account>/<revision>/<content>\n\nStop every 274bot process for this root \
         first; the command refuses while \
         another new-version process uses it. The root is the one normal use resolves \
         (<home>/.274bot).\n\nexport --same-root writes the latest saved preferences as \
         panel-ui.json in the same root, for 0.2.9. A changed file there is imported once at \
         the next start of this version; an unchanged one is ignored.\n\nrecover first \
         preserves the whole database set (state.sqlite3, -wal and -shm) in a private \
         state/preserved-<time> directory, then starts a new preference history: its version \
         numbers restart and are not comparable with the preserved ones. --source imports a \
         private, valid JSON object; --defaults discards the saved preferences (they stay in \
         the preserved set).\n\naccept-projection: a bank hint file with fields this build does \
         not know is imported read-only, and its original is kept as .migrated evidence. The \
         command shows the hint, its version, the evidence file and how many fields were \
         omitted, and asks for confirmation (a terminal is required) before later saves hold \
         the usable rows only.\n"
    )
}

/// Parse a maintenance command line (the arguments after the binary
/// name). `None` when the arguments are not a maintenance command (they do
/// not start with `state`); `Some(Err)` with the usage error for a bad or
/// combined command, before any root is opened.
pub fn parse_maintenance(
    args: &[String],
    frontend: Frontend,
) -> Option<Result<MaintenanceAction, String>> {
    let first = args.first()?;
    if first != "state" {
        return None;
    }
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    Some(match words.as_slice() {
        ["state", "preferences", "export", "--same-root"] => Ok(MaintenanceAction::ExportSameRoot),
        ["state", "preferences", "recover", "--source", source] => {
            Ok(MaintenanceAction::RecoverSource(PathBuf::from(source)))
        }
        ["state", "preferences", "recover", "--defaults"] => Ok(MaintenanceAction::RecoverDefaults),
        ["state", "bank-hints", "accept-projection", key] => {
            Ok(MaintenanceAction::AcceptBankProjection((*key).to_string()))
        }
        _ => Err(format!(
            "not one maintenance command (one action per run; export and recovery cannot be \
             combined)\n{}",
            maintenance_help(frontend)
        )),
    })
}

/// The shipped entry point: run a maintenance command line at the root
/// normal use resolves, print its report or refusal, and give the exit
/// code. `None` when `args` is not a maintenance command line. Called
/// before any frontend, vault or Play startup.
pub fn maintenance_command(frontend: Frontend, args: &[String]) -> Option<i32> {
    use std::io::Write as _;
    if args.len() >= 3
        && args[0] == "state"
        && matches!(args[1].as_str(), "preferences" | "bank-hints")
        && matches!(args[2].as_str(), "--help" | "-h" | "help")
    {
        let _ = write!(std::io::stdout(), "{}", maintenance_help(frontend));
        return Some(0);
    }
    let action = match parse_maintenance(args, frontend)? {
        Ok(action) => action,
        Err(usage) => {
            let _ = writeln!(std::io::stderr(), "{}: {usage}", frontend.binary());
            return Some(2);
        }
    };
    let root = StateRoot::new(script::bot_home().join(".274bot"));
    Some(
        match run_maintenance(&root, frontend, &action, &confirm_on_terminal) {
            Ok(report) => {
                let _ = writeln!(std::io::stdout(), "{report}");
                0
            }
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "{error}");
                1
            }
        },
    )
}

/// Run `action` at `root` for the `frontend` binary. `Ok` is the report
/// the command prints. An action that needs the operator's confirmation
/// (accepting a bank hint's projection) is refused: use
/// [`run_maintenance`] with a way to ask.
pub fn run_preferences_maintenance(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
) -> Result<String, MaintenanceError> {
    run_maintenance(root, frontend, action, &|_| false)
}

/// [`run_preferences_maintenance`] with `confirm`, the operator's answer to
/// the question an action asks before it changes anything.
pub fn run_maintenance(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    run(
        root,
        frontend,
        action,
        sql::Db::new(root.clone(), frontend, None),
        confirm,
    )
}

/// Ask on the terminal: `true` only for the answer `yes` typed on a
/// terminal. A command run without a terminal never confirms.
fn confirm_on_terminal(question: &str) -> bool {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return false;
    }
    let _ = write!(std::io::stderr(), "{question}\nType yes to continue: ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    stdin.lock().read_line(&mut answer).is_ok() && answer.trim() == "yes"
}

/// [`run_preferences_maintenance`] with a test's seams in the database
/// (a child process aborts at a named point of the export).
#[cfg(test)]
pub(super) fn run_preferences_maintenance_with_seams(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
    seams: sql::Seams,
) -> Result<String, MaintenanceError> {
    let mut db = sql::Db::new(root.clone(), frontend, None);
    db.seams = seams;
    run(root, frontend, action, db, &|_| false)
}

fn run(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
    mut db: sql::Db,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    let display = root.path().display().to_string();
    let Some(lease) = RootUse::exclusive(root).map_err(|error| {
        MaintenanceError(format!(
            "Cannot export or recover preferences at {display:?}: the root-use lock could not \
             be taken ({error})."
        ))
    })?
    else {
        return Err(MaintenanceError(lease_conflict(&display, frontend, action)));
    };
    let guard = files::try_lock_for_write(&root.preferences_file()).map_err(|error| {
        MaintenanceError(format!(
            "Cannot export or recover preferences at {display:?}: the preferences guard could \
             not be taken ({error})."
        ))
    })?;
    let Some(guard) = guard else {
        return Err(MaintenanceError(lease_conflict(&display, frontend, action)));
    };
    let result = match action {
        MaintenanceAction::ExportSameRoot => export(&mut db, &display, &guard),
        MaintenanceAction::RecoverSource(source) => recover(&mut db, root, Some(source), &guard),
        MaintenanceAction::RecoverDefaults => recover(&mut db, root, None, &guard),
        MaintenanceAction::AcceptBankProjection(name) => {
            accept_bank_projection(&mut db, root, name, &guard, confirm)
        }
    };
    db.close();
    drop(guard);
    drop(lease);
    result
}

/// Accept the read-only projection of one imported bank hint (see
/// [`MaintenanceAction::AcceptBankProjection`]): show what is read, ask, and
/// only then clear the refusal. Nothing changes without the evidence kept
/// beside the source and the operator's yes.
fn accept_bank_projection(
    db: &mut sql::Db,
    root: &StateRoot,
    name: &str,
    guard: &files::WriteLock,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    let Some(key) = bank::BankKey::parse(name) else {
        return Err(MaintenanceError(format!(
            "{name:?} is not a bank hint key. Name it as <profile>/<account>/<revision>/<content>, \
             for example local-289/alice/289/lostcity-289. Nothing was changed."
        )));
    };
    let opened = db.open_for_maintenance(guard).map_err(refusal)?;
    if !opened {
        return Err(MaintenanceError(
            "There is no state database in this root; nothing was changed.".to_string(),
        ));
    }
    let Some((row, ledger)) = db.bank_report(&key).map_err(refusal)? else {
        return Err(MaintenanceError(format!(
            "There is no stored bank hint {name}; nothing was changed. Names are listed in the \
             log line 'bank-hints hint ... has N omitted field(s)'."
        )));
    };
    if row.omitted == 0 {
        return Err(MaintenanceError(format!(
            "The bank hint {name} has no read-only projection to accept; nothing was changed."
        )));
    }
    let evidence = key.evidence_file(root, ledger.digest).ok_or_else(|| {
        MaintenanceError(format!(
            "The original of the bank hint {name} is not kept as a .migrated file beside \
             {}, so its omitted fields cannot be accepted away. Restore the original there, \
             or leave the hint read-only. Nothing was changed.",
            root.bank_hint_file(&key.profile, &key.account).display()
        ))
    })?;
    let question = format!(
        "Bank hint {name} is read-only at {}: {} usable row(s), {} omitted field(s) kept in \
         the original {}. Accepting means every later save of this hint holds the usable \
         rows only, and the omitted fields stay only in that file.",
        row.version,
        row.snapshot.as_bank_hints().map_or(0, <[_]>::len),
        row.omitted,
        evidence.display()
    );
    if !confirm(&question) {
        return Err(MaintenanceError(format!(
            "{question} Not confirmed (a terminal and the answer yes are required); nothing was \
             changed."
        )));
    }
    db.accept_bank_projection(guard, &key, row.version)
        .map_err(refusal)?;
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "audit domain=bank-hints profile={} account={} action=bank-hint-accept-projection \
         origin=maintenance from={} omitted={} evidence={}",
        key.profile,
        key.account,
        row.version,
        row.omitted,
        evidence.display()
    );
    Ok(format!(
        "Accepted the projection of bank hint {name}: later saves hold its {} usable row(s) only. \
         The original stays in {}.",
        row.snapshot.as_bank_hints().map_or(0, <[_]>::len),
        evidence.display()
    ))
}

fn export(
    db: &mut sql::Db,
    root: &str,
    guard: &files::WriteLock,
) -> Result<String, MaintenanceError> {
    let opened = db.open_for_maintenance(guard).map_err(refusal)?;
    if !opened {
        return Err(MaintenanceError(format!(
            "There are no SQLite preferences at {root:?} to export; nothing was written."
        )));
    }
    let exported = db.export_same_root(guard).map_err(refusal)?;
    let kept = exported
        .kept
        .as_ref()
        .map(|kept| format!("; the file that was there is kept as {}", kept.display()))
        .unwrap_or_default();
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "{}",
        sql::audit_line(
            "preferences-export",
            exported.version,
            exported.version,
            &format!(
                "export {} of {} to {} sha256={}{kept}",
                exported.export_id,
                exported.version,
                exported.path.display(),
                exported.digest
            ),
        )
    );
    Ok(format!(
        "Exported preferences {} to {} (export {}, sha256 {}){kept}. Stop this version before \
         starting 0.2.9 in this home. The next start of this version imports the file once if \
         0.2.9 changed it.",
        exported.version,
        exported.path.display(),
        exported.export_id,
        exported.digest
    ))
}

fn recover(
    db: &mut sql::Db,
    root: &StateRoot,
    source: Option<&PathBuf>,
    guard: &files::WriteLock,
) -> Result<String, MaintenanceError> {
    let (body, digest, kind, from) = match source {
        Some(source) => {
            let display = source.display().to_string();
            if !source.is_absolute() {
                return Err(MaintenanceError(format!(
                    "Preferences refused at {display:?}: the source must be an absolute path. \
                     Nothing was changed."
                )));
            }
            match sql::read_legacy(source) {
                Legacy::Valid {
                    bytes, canonical, ..
                } => (
                    canonical,
                    Some(files::digest(&bytes)),
                    "recovered-source",
                    display,
                ),
                Legacy::Absent => {
                    return Err(MaintenanceError(format!(
                        "Preferences refused at {display:?}: there is no file there. Nothing \
                         was changed."
                    )))
                }
                Legacy::Malformed(_) => {
                    return Err(MaintenanceError(format!(
                        "Preferences refused at {display:?}: the file is not a JSON object of \
                         preferences. Nothing was changed. Name a private, valid JSON object."
                    )))
                }
                Legacy::Refused(failure) => {
                    return Err(refusal(db.source_failure(
                        &display,
                        failure,
                        sql::SourceUse::Recovery,
                    )));
                }
            }
        }
        None => (
            files::preferences_bytes(&Map::new())
                .map_err(|error| MaintenanceError(error.to_string()))?,
            None,
            "recovered-defaults",
            "defaults".to_string(),
        ),
    };
    // The bank hints and the other domains' import ledger belong to the
    // database too, and a preferences recovery must not reset them: read
    // them from the old database before it is preserved.
    let carry = db.read_bank_carry();
    let preserved = preserve(root).map_err(|error| {
        MaintenanceError(format!(
            "Recovery stopped at {:?}: the old database files could not be preserved ({error}). \
             Nothing was removed or replaced.",
            root.state_dir().display().to_string()
        ))
    })?;
    // The file at `panel-ui.json`, whatever it holds, is what this recovery
    // accounts for: the next start must not take it for a later outside
    // edit (it may be the very over-limit file that caused the refusal).
    let accounted = sql::legacy_file_digest(&root.preferences_file()).or(digest);
    let version = db
        .start_new_history(guard, &body, accounted, kind, &from)
        .map_err(refusal)?;
    let bank_note = match carry {
        Ok(carry) => {
            db.restore_bank_carry(guard, carry.as_ref())
                .map_err(refusal)?;
            match carry {
                Some(carry) => {
                    format!("{} bank hint(s) were carried over unchanged", carry.hints())
                }
                None => "there were no bank hints to carry over".to_string(),
            }
        }
        Err(failure) => {
            db.mark_bank_sources_unknown(guard).map_err(refusal)?;
            let why = match failure {
                SqlFailure::Wait(cause) => cause.reason().to_string(),
                SqlFailure::Refused { detail, .. } => detail.to_string(),
            };
            format!(
                "the bank hints could not be read from the old database ({why}); they stay in \
                 the preserved set, and no legacy bank hint file is imported again"
            )
        }
    };
    let kept = preserved
        .as_ref()
        .map(|dir| format!("the old database set is preserved in {}", dir.display()))
        .unwrap_or_else(|| "there was no old database set to preserve".into());
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "{}",
        sql::audit_line(
            &format!("preferences-{kind}"),
            StoreVersion::INITIAL,
            version,
            &format!("preferences history restarted from {from}; {kept}"),
        )
    );
    Ok(format!(
        "Preferences recovered from {from} at {version}: preferences history restarted; \
         {kept}; {bank_note}."
    ))
}

/// Create `dir` new (an existing one is `AlreadyExists`) owner-only from the
/// start where the platform has modes, whatever the process umask (a umask
/// only removes bits). The preserved copies of the database never sit in a
/// directory others can enter, not even before the first one is written.
fn create_new_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(dir)
    }
}

/// Keep a private, verified copy of every existing file of the database
/// set in a new `preserved-<time>` directory, sync it, then remove the
/// originals. No connection is open in this process, and the exclusive
/// lease keeps every other user out. `None` when there was no set.
fn preserve(root: &StateRoot) -> io::Result<Option<PathBuf>> {
    let set: Vec<PathBuf> = sql::database_set(root)
        .into_iter()
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .collect();
    if set.is_empty() {
        return Ok(None);
    }
    let state = root.state_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut dir = None;
    for attempt in 0..100u32 {
        let candidate = state.join(match attempt {
            0 => format!("preserved-{stamp}"),
            n => format!("preserved-{stamp}-{n}"),
        });
        match create_new_private_dir(&candidate) {
            Ok(()) => {
                dir = Some(candidate);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let dir = dir.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free preserved-<time> name",
        )
    })?;
    for path in &set {
        let bytes = fs::read(path)?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?;
        let copy = dir.join(name);
        vault::create_private_file(&copy, &bytes)?;
        let held = fs::read(&copy)?;
        if files::digest(&held) != files::digest(&bytes) {
            return Err(io::Error::other(format!(
                "{}: the copy does not hold the file's bytes",
                copy.display()
            )));
        }
        fs::File::open(&copy)?.sync_all()?;
    }
    files::sync_directory(&dir)?;
    files::sync_directory(&state)?;
    for path in &set {
        fs::remove_file(path)?;
    }
    files::sync_directory(&state)?;
    Ok(Some(dir))
}

fn refusal(failure: SqlFailure) -> MaintenanceError {
    match failure {
        SqlFailure::Refused { detail, .. } => MaintenanceError(detail.to_string()),
        SqlFailure::Wait(cause) => MaintenanceError(format!(
            "{} Stop every 274bot process for this root, then retry the same maintenance \
             command.",
            cause.text()
        )),
    }
}

/// The maintenance lease conflict (plan §5.5).
fn lease_conflict(root: &str, frontend: Frontend, action: &MaintenanceAction) -> String {
    let mut text = format!(
        "Cannot export or recover preferences at {root:?}: another new-version process uses \
         this root. Stop that process. Retry the same maintenance command."
    );
    let bin = frontend.binary();
    match action {
        MaintenanceAction::ExportSameRoot => {
            text.push_str(&format!(
                " Run '{bin} state preferences export --same-root' again."
            ));
        }
        MaintenanceAction::RecoverSource(_) | MaintenanceAction::RecoverDefaults => {
            text.push_str(&format!(
                " Run '{bin} state preferences recover --source \"<private-valid-json>\"' or \
                 '{bin} state preferences recover --defaults'."
            ));
        }
        MaintenanceAction::AcceptBankProjection(name) => {
            text.push_str(&format!(
                " Run '{bin} state bank-hints accept-projection {name}' again."
            ));
        }
    }
    text
}
