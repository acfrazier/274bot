//! The persisted bank hint: one account's last-seen bank rows on disk
//! (design-bank-snapshot §1.4), so planning works before any bank is
//! opened in a new process.
//!
//! `~/.274bot/bank-hints/<profile>/<account>.json`, schema 1, written
//! `0o600` by [`vault::write_private_file`] (atomic replace, parents
//! `0o700`). `<profile>` is the launch profile name (already a safe
//! component when profiles load); `<account>` is the slot username, which
//! nothing else validates, so [`account_component`] gates it. The path is
//! resolved **once, on the spawning thread** ([`HintFile::for_account`]):
//! the slot thread never calls [`crate::bot_file`], so a test's thread-local
//! [`crate::IsolatedEnv`] pin covers every save, and the operator's real
//! `~/.274bot` is never touched by a test.
//!
//! A load failure never blocks login: the memory stays `Unknown` and the
//! caller logs one line saying why.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use api::bank_memory::{BankMemory, HintRowsError, MAX_HINT_ROWS};
use serde::{Deserialize, Serialize};
use vault::{read_private_file, write_private_file};

/// The on-disk format this build writes and the only one it reads.
pub const SCHEMA_VERSION: u32 = 1;

/// The longest account name that may be a path component.
pub const MAX_ACCOUNT_BYTES: usize = 64;

/// The largest hint file read: the row cap at the widest JSON a row takes
/// plus the header, with room to spare.
const MAX_HINT_BYTES: u64 = 64 * 1024;

/// Why a hint was not loaded or saved. Row-level rejects wrap
/// [`HintRowsError`]; the rest are file-level.
#[derive(Debug)]
pub enum HintError {
    /// The account name cannot be one path component (§1.4).
    UnsafeAccount(String),
    /// The profile name cannot be one path component.
    UnsafeProfile(String),
    Io(io::Error),
    Json(serde_json::Error),
    /// The file was written by another schema.
    Schema {
        found: u32,
    },
    /// The file names another profile or account than the path it sits at.
    Identity {
        profile: String,
        account: String,
    },
    Rows(HintRowsError),
}

impl fmt::Display for HintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HintError::UnsafeAccount(name) => {
                write!(f, "account {name:?} is not a safe path component")
            }
            HintError::UnsafeProfile(name) => {
                write!(f, "profile {name:?} is not a safe path component")
            }
            HintError::Io(error) => write!(f, "io: {error}"),
            HintError::Json(error) => write!(f, "json: {error}"),
            HintError::Schema { found } => {
                write!(
                    f,
                    "schema_version {found} (this build reads {SCHEMA_VERSION})"
                )
            }
            HintError::Identity { profile, account } => {
                write!(f, "file names profile {profile:?} account {account:?}")
            }
            HintError::Rows(error) => write!(f, "rows: {error}"),
        }
    }
}

impl std::error::Error for HintError {}

impl From<io::Error> for HintError {
    fn from(error: io::Error) -> Self {
        HintError::Io(error)
    }
}

impl From<serde_json::Error> for HintError {
    fn from(error: serde_json::Error) -> Self {
        HintError::Json(error)
    }
}

impl From<HintRowsError> for HintError {
    fn from(error: HintRowsError) -> Self {
        HintError::Rows(error)
    }
}

/// The account name as one path component, or why it cannot be one:
/// non-empty, at most [`MAX_ACCOUNT_BYTES`], not `.` or `..`, and no byte
/// that is `/`, `\`, NUL or an ASCII control (`< 0x20`, `0x7f`). Everything
/// else — letters, digits, spaces (legal RS names), `_`, `-` — is used
/// verbatim. A rejected name means no load and no save for that slot.
pub fn account_component(account: &str) -> Result<&str, HintError> {
    if safe_component(account) {
        Ok(account)
    } else {
        Err(HintError::UnsafeAccount(account.to_owned()))
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ACCOUNT_BYTES
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| !matches!(byte, b'/' | b'\\' | 0..=0x1f | 0x7f))
}

/// The persisted document, schema 1:
///
/// ```json
/// {"schema_version":1,"profile":"local-289","account":"alice",
///  "observed_at_unix":1759700000,"rows":[[379,12],[995,3400]]}
/// ```
///
/// Rows are `(obj id, count)` sorted by id, unique, every count positive,
/// unnoted ids only (the bank stores unnoted stock).
#[derive(Debug, Serialize, Deserialize)]
struct HintDocument {
    schema_version: u32,
    profile: String,
    account: String,
    observed_at_unix: u64,
    rows: Vec<(i32, i32)>,
}

/// One account's hint file: the resolved path and the identity the file
/// must carry. Built on the spawning thread; the slot thread only loads
/// and saves through it.
#[derive(Debug, Clone)]
pub struct HintFile {
    path: PathBuf,
    profile: String,
    account: String,
}

impl HintFile {
    /// `~/.274bot/bank-hints/<profile>/<account>.json`, resolved through
    /// [`crate::bot_file`] on the calling thread. Both names must be safe
    /// path components.
    pub fn for_account(profile: &str, account: &str) -> Result<Self, HintError> {
        if !safe_component(profile) {
            return Err(HintError::UnsafeProfile(profile.to_owned()));
        }
        let account = account_component(account)?;
        let path = crate::bot_file("bank-hints")
            .join(profile)
            .join(format!("{account}.json"));
        Ok(Self::at(path, profile, account))
    }

    /// A hint file at an explicit path (the `LoadoutsStore::at` pattern for
    /// tests that write under their own scratch directory).
    pub fn at(path: PathBuf, profile: &str, account: &str) -> Self {
        Self {
            path,
            profile: profile.to_owned(),
            account: account.to_owned(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Fill `memory` from the file: `Ok(true)` when rows were loaded,
    /// `Ok(false)` when there is no file yet (a first run), `Err` when the
    /// file was refused — wrong schema, another identity, or rows that
    /// break the §1.4 rules. On `Ok(false)` and `Err` the memory is left as
    /// it was. The caller loads only into an `Unknown` memory (§1.4
    /// precedence).
    pub fn load_into(&self, memory: &mut BankMemory) -> Result<bool, HintError> {
        let raw = match read_private_file(&self.path, MAX_HINT_BYTES) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let document: HintDocument = serde_json::from_str(&raw)?;
        if document.schema_version != SCHEMA_VERSION {
            return Err(HintError::Schema {
                found: document.schema_version,
            });
        }
        if document.profile != self.profile || document.account != self.account {
            return Err(HintError::Identity {
                profile: document.profile,
                account: document.account,
            });
        }
        if document.rows.len() > MAX_HINT_ROWS {
            return Err(HintRowsError::TooManyRows.into());
        }
        memory.load_hint(document.rows, document.observed_at_unix)?;
        Ok(true)
    }

    /// Write the memory's rows if it holds unsaved observations: `Ok(true)`
    /// when a file was written, `Ok(false)` when nothing was pending. A
    /// write marks the memory saved; a failed write leaves it dirty for the
    /// next close or session end.
    pub fn save_if_dirty(&self, memory: &mut BankMemory) -> Result<bool, HintError> {
        if !memory.dirty() {
            return Ok(false);
        }
        let document = HintDocument {
            schema_version: SCHEMA_VERSION,
            profile: self.profile.clone(),
            account: self.account.clone(),
            observed_at_unix: memory.observed_at().map_or(0, |at| at.unix_secs),
            rows: memory.rows().to_vec(),
        };
        let raw = serde_json::to_vec(&document)?;
        write_private_file(&self.path, &raw)?;
        memory.mark_saved();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IsolatedEnv;
    use api::bank_memory::{ObservedAt, Origin};
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};
    use api::ItemDefView;

    const LOBSTER: i32 = 379;
    const COINS: i32 = 995;

    fn bank_row(id: i32, count: i32, slot: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some("fixture".into()),
                stackable: id == COINS,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Component,
            slot,
            count,
            actions: Vec::new(),
            component_id: 5292,
        }
    }

    fn observed(rows: &[(i32, i32)], unix_secs: u64) -> BankMemory {
        let views: Vec<ItemView> = rows
            .iter()
            .enumerate()
            .map(|(slot, &(id, count))| bank_row(id, count, slot as i32))
            .collect();
        let mut memory = BankMemory::default();
        memory.observe(
            &views,
            64,
            ObservedAt {
                unix_secs,
                tick: 3,
                bank_session: 1,
            },
        );
        memory
    }

    fn write_document(path: &Path, document: serde_json::Value) {
        write_private_file(path, document.to_string().as_bytes()).unwrap();
    }

    #[test]
    fn account_component_rejects_separators_dots_controls_and_empty() {
        for rejected in [
            "a/b",
            "a\\b",
            "..",
            ".",
            "a\0b",
            "a\x1bb",
            "tab\tname",
            "del\x7f",
            "",
        ] {
            assert!(
                matches!(
                    account_component(rejected),
                    Err(HintError::UnsafeAccount(name)) if name == rejected
                ),
                "{rejected:?} must be refused"
            );
        }
        let long = "a".repeat(MAX_ACCOUNT_BYTES + 1);
        assert!(matches!(
            account_component(&long),
            Err(HintError::UnsafeAccount(_))
        ));
        let widest = "b".repeat(MAX_ACCOUNT_BYTES);
        assert_eq!(account_component(&widest).unwrap(), widest);
        assert_eq!(account_component("alice smith").unwrap(), "alice smith");
        assert_eq!(account_component("Bob_2").unwrap(), "Bob_2");
        assert_eq!(account_component("x-y.z").unwrap(), "x-y.z");
        assert_eq!(account_component("...").unwrap(), "...");
    }

    #[test]
    fn for_account_resolves_under_the_thread_pinned_home() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice smith").unwrap();
        assert_eq!(
            file.path(),
            scratch
                .home
                .join(".274bot")
                .join("bank-hints")
                .join("local-289")
                .join("alice smith.json")
        );
        assert!(matches!(
            HintFile::for_account("local-289", "../alice"),
            Err(HintError::UnsafeAccount(_))
        ));
        assert!(matches!(
            HintFile::for_account("../local", "alice"),
            Err(HintError::UnsafeProfile(_))
        ));
        assert!(matches!(
            HintFile::for_account("", "alice"),
            Err(HintError::UnsafeProfile(_))
        ));
    }

    #[test]
    fn save_then_load_round_trips_rows_at_private_mode() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let mut memory = observed(&[(COINS, 3_400), (LOBSTER, 12)], 1_759_700_000);
        assert!(memory.dirty());
        assert!(
            file.save_if_dirty(&mut memory).unwrap(),
            "a dirty memory is written"
        );
        assert!(!memory.dirty());
        assert!(
            !file.save_if_dirty(&mut memory).unwrap(),
            "a saved memory is not rewritten"
        );
        assert!(file.path().starts_with(&scratch.home));

        let raw = std::fs::read_to_string(file.path()).unwrap();
        let document: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            document,
            serde_json::json!({
                "schema_version": 1,
                "profile": "local-289",
                "account": "alice",
                "observed_at_unix": 1_759_700_000u64,
                "rows": [[LOBSTER, 12], [COINS, 3_400]],
            })
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "written private: {mode:o}");
            let dir = std::fs::metadata(file.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir, 0o700, "parent created private: {dir:o}");
        }

        let mut loaded = BankMemory::default();
        assert!(file.load_into(&mut loaded).unwrap());
        assert_eq!(loaded.origin(), Origin::Hint);
        assert_eq!(loaded.rows(), &[(LOBSTER, 12), (COINS, 3_400)]);
        assert_eq!(loaded.count(LOBSTER), Some(12));
        assert_eq!(
            loaded.observed_at().map(|at| at.unix_secs),
            Some(1_759_700_000)
        );
        assert!(!loaded.dirty());
    }

    #[test]
    fn a_missing_file_is_a_first_run_not_an_error() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let mut memory = BankMemory::default();
        assert!(!file.load_into(&mut memory).unwrap());
        assert!(!memory.known());
    }

    #[test]
    fn load_rejects_schema_identity_cap_and_every_row_rule() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let good = |rows: serde_json::Value| {
            serde_json::json!({
                "schema_version": 1,
                "profile": "local-289",
                "account": "alice",
                "observed_at_unix": 5,
                "rows": rows,
            })
        };
        let mut schema = good(serde_json::json!([[LOBSTER, 1]]));
        schema["schema_version"] = serde_json::json!(2);
        let mut profile = good(serde_json::json!([[LOBSTER, 1]]));
        profile["profile"] = serde_json::json!("local-290");
        let mut account = good(serde_json::json!([[LOBSTER, 1]]));
        account["account"] = serde_json::json!("bob");
        let too_many: Vec<[i32; 2]> = (0..=MAX_HINT_ROWS as i32).map(|id| [id, 1]).collect();
        type Case = (&'static str, serde_json::Value, fn(&HintError) -> bool);
        let cases: Vec<Case> = vec![
            ("schema", schema, |e| {
                matches!(e, HintError::Schema { found: 2 })
            }),
            ("profile", profile, |e| {
                matches!(e, HintError::Identity { .. })
            }),
            ("account", account, |e| {
                matches!(e, HintError::Identity { .. })
            }),
            ("cap", good(serde_json::json!(too_many)), |e| {
                matches!(e, HintError::Rows(HintRowsError::TooManyRows))
            }),
            ("zero", good(serde_json::json!([[LOBSTER, 0]])), |e| {
                matches!(e, HintError::Rows(HintRowsError::NonPositiveCount))
            }),
            (
                "negative",
                good(serde_json::json!([[LOBSTER, 2], [COINS, -1]])),
                |e| matches!(e, HintError::Rows(HintRowsError::NonPositiveCount)),
            ),
            (
                "unsorted",
                good(serde_json::json!([[COINS, 1], [LOBSTER, 1]])),
                |e| matches!(e, HintError::Rows(HintRowsError::Unsorted)),
            ),
            (
                "duplicate",
                good(serde_json::json!([[LOBSTER, 1], [LOBSTER, 1]])),
                |e| matches!(e, HintError::Rows(HintRowsError::DuplicateId)),
            ),
            ("shape", serde_json::json!({"schema_version": 1}), |e| {
                matches!(e, HintError::Json(_))
            }),
        ];
        for (label, document, expected) in cases {
            write_document(file.path(), document);
            let mut memory = BankMemory::default();
            let error = file.load_into(&mut memory).unwrap_err();
            assert!(expected(&error), "{label}: {error}");
            assert!(
                !memory.known(),
                "{label}: a refused file leaves the memory Unknown"
            );
        }
        std::fs::write(file.path(), "not json").unwrap();
        let mut memory = BankMemory::default();
        assert!(matches!(
            file.load_into(&mut memory).unwrap_err(),
            HintError::Json(_)
        ));
        assert!(!memory.known());
    }

    #[test]
    fn save_tolerates_a_separate_scratch_path() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::at(scratch.dir.join("hint.json"), "p", "a");
        let mut memory = observed(&[(LOBSTER, 1)], 9);
        assert!(file.save_if_dirty(&mut memory).unwrap());
        let mut loaded = BankMemory::default();
        assert!(file.load_into(&mut loaded).unwrap());
        assert_eq!(loaded.rows(), &[(LOBSTER, 1)]);
        assert_eq!(loaded.observed_at().map(|at| at.unix_secs), Some(9));
    }

    #[test]
    fn an_empty_session_bank_saves_as_no_rows_and_loads_known_empty() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let mut memory = observed(&[], 4);
        assert!(file.save_if_dirty(&mut memory).unwrap());
        let mut loaded = BankMemory::default();
        assert!(file.load_into(&mut loaded).unwrap());
        assert!(loaded.known());
        assert_eq!(loaded.count(LOBSTER), Some(0));
    }

    #[test]
    fn relog_precedence_keeps_a_known_memory_and_loads_an_unknown_one() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        // A file on disk holding different rows than the session observed.
        let mut stale = observed(&[(LOBSTER, 1)], 1);
        file.save_if_dirty(&mut stale).unwrap();

        // Known in process: relog keeps the in-memory rows and never re-reads.
        let mut memory = observed(&[(COINS, 99)], 2);
        if memory.known() {
            memory.relogged();
        } else {
            file.load_into(&mut memory).unwrap();
        }
        assert_eq!(memory.origin(), Origin::Hint);
        assert_eq!(memory.rows(), &[(COINS, 99)]);
        assert!(memory.dirty(), "the unsaved session is still pending");

        // Unknown in process: the file is loaded.
        let mut fresh = BankMemory::default();
        if fresh.known() {
            fresh.relogged();
        } else {
            file.load_into(&mut fresh).unwrap();
        }
        assert_eq!(fresh.origin(), Origin::Hint);
        assert_eq!(fresh.rows(), &[(LOBSTER, 1)]);
    }

    #[test]
    fn a_tracked_open_and_close_saves_once_into_the_pinned_home() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let mut memory = BankMemory::default();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(5292, 1, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        assert_eq!(
            memory.track(&snapshot, 10),
            api::bank_memory::FrameEvent::Opened
        );
        snapshot.seed_bank_observation(5292, 2, Some(vec![bank_row(COINS, 43, 0)]), Vec::new());
        assert_eq!(
            memory.track(&snapshot, 11),
            api::bank_memory::FrameEvent::Observed
        );
        assert!(!file.path().exists(), "no save while the bank is open");
        snapshot.seed_bank_observation(-1, 2, None, Vec::new());
        assert_eq!(
            memory.track(&snapshot, 12),
            api::bank_memory::FrameEvent::Closed
        );
        assert!(file.save_if_dirty(&mut memory).unwrap());
        assert!(file.path().starts_with(&scratch.home));
        let raw = std::fs::read_to_string(file.path()).unwrap();
        assert!(raw.contains(&format!("[[{COINS},43]]")), "{raw}");
        assert!(raw.contains("\"observed_at_unix\":11"), "{raw}");
        assert!(
            !file.save_if_dirty(&mut memory).unwrap(),
            "exactly one save per close"
        );
    }
}
