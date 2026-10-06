//! The persisted bank hint: one account's last-seen bank rows on disk
//! (design-bank-snapshot §1.4), so planning works before any bank is
//! opened in a new process.
//!
//! `~/.274bot/bank-hints/<profile>/<account>.json`, schema 1, written
//! `0o600` by [`vault::write_private_file`] (atomic replace, parents
//! `0o700`). `<profile>` is the launch profile name and must meet the rule
//! profiles are loaded under, [`vault::valid_component`]; `<account>` is the
//! slot username as the client's login identity spells it
//! ([`account_component`]), so one account is one file however the
//! operator typed it. The path is resolved **once, on the spawning thread**
//! ([`HintFile::for_account`]): the slot thread never calls
//! [`crate::bot_file`], so a test's thread-local [`crate::IsolatedEnv`] pin
//! covers every save, and the operator's real `~/.274bot` is never touched
//! by a test.
//!
//! No file I/O runs under the owner's bank-memory guard (§1.2): a load
//! reads and checks the file first ([`HintFile::load`]) and the owner
//! applies the result under a short guard; a save serializes under the
//! owner's guard ([`HintFile::pending_save`]) and publishes after it is
//! released ([`HintFile::write`]).
//!
//! A load failure never blocks login: the memory stays `Unknown` and the
//! caller logs one line saying why.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use api::bank_memory::{BankMemory, HintRowsError, MAX_HINT_ROWS};
use client::util::JString;
use serde::{Deserialize, Serialize};
use vault::{read_private_file, valid_component, write_private_file};

/// The on-disk format this build writes and the only one it reads.
pub const SCHEMA_VERSION: u32 = 1;

/// The most characters of a name the client's login hash reads
/// (`JString.toUserhash`): the rest of a longer name is not part of the
/// account.
pub const MAX_ACCOUNT_CHARS: usize = 12;

/// The largest hint file read: the row cap at the widest JSON a row takes
/// plus the header, with room to spare.
const MAX_HINT_BYTES: u64 = 64 * 1024;

/// Why a hint was not loaded or saved. Row-level rejects wrap
/// [`HintRowsError`]; the rest are file-level.
#[derive(Debug)]
pub enum HintError {
    /// The account name is not a game login name ([`account_component`]).
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
                write!(
                    f,
                    "account {name:?} is not a login name (letters, digits, spaces and \
                     underscores only, with a letter or digit in the first \
                     {MAX_ACCOUNT_CHARS})"
                )
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

/// The account name as the one path component the client's login identity
/// gives it, or why it cannot be one. The identity is what the client logs
/// the name in as, computed by the client's own decoder:
/// `JString.toRawUsername(JString.toUserhash(name))` — a whitespace trim,
/// the first [`MAX_ACCOUNT_CHARS`] characters hashed base-37 (case folded,
/// a space and an underscore both the zero digit, so leading zeros vanish
/// and the name ends at its last letter or digit), decoded back. So
/// `Alice Smith`, `alice_smith` and ` alice smith` are one account and get
/// one file, `alice_smith.json`; `_ab3456789xyz` logs in as
/// `ab3456789xy` (the underscore spends one of the twelve) and gets that
/// file; `ab3456789xyz1` logs in as `ab3456789xyz`. A name with any other
/// character is refused before it is hashed (the hash would fold it to a
/// separator), and so is a name the hash spells as no account at all (no
/// letter or digit within the twelve, e.g. `____________a`), which means no
/// load and no save for that slot.
pub fn account_component(account: &str) -> Result<String, HintError> {
    let unsafe_account = || HintError::UnsafeAccount(account.to_owned());
    if !account
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        return Err(unsafe_account());
    }
    let hash = JString::to_userhash(account);
    // The decoder answers `invalid_name` for a hash that spells no account;
    // of what `to_userhash` can return, that is exactly zero.
    if hash == 0 {
        return Err(unsafe_account());
    }
    let identity = JString::to_raw_username(hash as i64);
    debug_assert!(
        valid_component(&identity),
        "the decoder spells {account:?} as {identity:?}"
    );
    Ok(identity)
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

/// A hint file's rows, read and checked with no guard held
/// ([`HintFile::load`]); the owner applies them under a short write guard.
#[derive(Debug)]
pub struct LoadedHint {
    rows: Vec<(i32, i32)>,
    observed_at_unix: u64,
}

impl LoadedHint {
    /// Fill `memory` from the file's rows (`Hint` origin, nothing to save),
    /// or refuse them whole when they break the §1.4 row rules and leave
    /// the memory as it was. The owner applies only into an `Unknown`
    /// memory (§1.4 precedence), re-checked under its guard.
    pub fn apply(self, memory: &mut BankMemory) -> Result<(), HintError> {
        memory.load_hint(self.rows, self.observed_at_unix)?;
        Ok(())
    }
}

/// What a save point took from the memory under its guard
/// ([`HintFile::pending_save`]): the serialized document and the generation
/// it describes. Published after the guard is released ([`HintFile::write`]);
/// on success the owner marks that generation saved.
#[derive(Debug)]
pub struct PendingSave {
    raw: Vec<u8>,
    generation: u64,
    rows: usize,
}

impl PendingSave {
    /// The memory generation the document describes: the argument to
    /// `BankMemory::mark_saved` once the write succeeded.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// How many rows the document carries.
    pub fn rows(&self) -> usize {
        self.rows
    }
}

impl HintFile {
    /// `~/.274bot/bank-hints/<profile>/<account>.json`, resolved through
    /// [`crate::bot_file`] on the calling thread. The profile must be a
    /// [`vault::valid_component`] (the rule it was loaded under); the
    /// account is spelled by [`account_component`].
    pub fn for_account(profile: &str, account: &str) -> Result<Self, HintError> {
        if !valid_component(profile) {
            return Err(HintError::UnsafeProfile(profile.to_owned()));
        }
        let account = account_component(account)?;
        let path = crate::bot_file("bank-hints")
            .join(profile)
            .join(format!("{account}.json"));
        Ok(Self::at(path, profile, &account))
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

    /// The account identity the file carries ([`account_component`]).
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Read and check the file, holding no guard: `Ok(Some)` carries the
    /// rows to apply, `Ok(None)` is no file yet (a first run), `Err` is a
    /// refused file — unreadable, wrong schema, another identity, or more
    /// rows than [`MAX_HINT_ROWS`].
    pub fn load(&self) -> Result<Option<LoadedHint>, HintError> {
        let raw = match read_private_file(&self.path, MAX_HINT_BYTES) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
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
        Ok(Some(LoadedHint {
            rows: document.rows,
            observed_at_unix: document.observed_at_unix,
        }))
    }

    /// Serialize the memory's rows if it holds unsaved observations; no
    /// I/O, so the caller may hold a read guard. `None` when nothing is
    /// pending.
    pub fn pending_save(&self, memory: &BankMemory) -> Result<Option<PendingSave>, HintError> {
        if !memory.dirty() {
            return Ok(None);
        }
        let document = HintDocument {
            schema_version: SCHEMA_VERSION,
            profile: self.profile.clone(),
            account: self.account.clone(),
            observed_at_unix: memory.observed_at().map_or(0, |at| at.unix_secs),
            rows: memory.rows().to_vec(),
        };
        Ok(Some(PendingSave {
            raw: serde_json::to_vec(&document)?,
            generation: memory.generation(),
            rows: document.rows.len(),
        }))
    }

    /// Publish a pending save, holding no guard. On `Ok` the owner marks
    /// `pending.generation()` saved; on `Err` the memory stays dirty for
    /// the next close or session end.
    pub fn write(&self, pending: &PendingSave) -> Result<(), HintError> {
        write_private_file(&self.path, &pending.raw)?;
        Ok(())
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

    /// The owner's load as one call: read and check, then apply.
    fn load_into(file: &HintFile, memory: &mut BankMemory) -> Result<bool, HintError> {
        match file.load()? {
            Some(loaded) => loaded.apply(memory).map(|()| true),
            None => Ok(false),
        }
    }

    /// The owner's save as one call: serialize, publish, mark saved.
    fn save_if_dirty(file: &HintFile, memory: &mut BankMemory) -> Result<bool, HintError> {
        let Some(pending) = file.pending_save(memory)? else {
            return Ok(false);
        };
        file.write(&pending)?;
        memory.mark_saved(pending.generation());
        Ok(true)
    }

    /// What the client logs the name in as: `toRawUsername(toUserhash(name))`.
    fn client_login_name(name: &str) -> String {
        let hash = client::util::JString::to_userhash(name);
        client::util::JString::to_raw_username(i64::try_from(hash).unwrap())
    }

    #[test]
    fn account_component_spells_the_name_as_the_client_login_identity() {
        // Every accepted spelling is the client's own login identity: case
        // folds, a space is an underscore, surrounding separators go,
        // leading separators spend the twelve-character budget, and a
        // longer name logs in as its first twelve characters.
        let twelve = "ab3456789xyZ";
        assert_eq!(twelve.len(), MAX_ACCOUNT_CHARS);
        let thirteen = "ab3456789xyz1";
        assert_eq!(thirteen.len(), MAX_ACCOUNT_CHARS + 1);
        for (typed, file) in [
            ("alice", "alice"),
            ("Alice Smith", "alice_smith"),
            ("alice_smith", "alice_smith"),
            ("ALICE_SMITH", "alice_smith"),
            (" alice smith", "alice_smith"),
            ("alice smith_", "alice_smith"),
            ("_alice_smith_", "alice_smith"),
            ("bm4h30nbov_0", "bm4h30nbov_0"),
            ("Bob_2", "bob_2"),
            ("a  b", "a__b"),
            (twelve, "ab3456789xyz"),
            // Surrounding whitespace is trimmed before the twelve.
            (" ab3456789xyZ ", "ab3456789xyz"),
            // REVIEW-BANK-SNAPSHOT-S2-R2 M1-R2: a leading underscore is one
            // of the twelve, so the last letter falls off, as it does at
            // the client's login.
            ("_ab3456789xyz", "ab3456789xy"),
            ("__ab3456789xyz", "ab3456789x"),
            (thirteen, "ab3456789xyz"),
        ] {
            assert_eq!(account_component(typed).unwrap(), file, "{typed:?}");
            assert_eq!(
                account_component(typed).unwrap(),
                client_login_name(typed),
                "{typed:?}: the file name is the client's login spelling"
            );
        }
    }

    #[test]
    fn account_component_rejects_hostile_and_no_account_names() {
        // Refused before the hash sees them: the client would fold each of
        // these characters to a separator and log into some other account.
        for hostile in [
            "a/b",
            "a\\b",
            "..",
            ".",
            "../alice",
            "x-y.z",
            "a\0b",
            "a\x1bb",
            "tab\tname",
            "del\x7f",
            "élan",
        ] {
            assert!(
                matches!(
                    account_component(hostile),
                    Err(HintError::UnsafeAccount(name)) if name == hostile
                ),
                "{hostile:?} must be refused"
            );
        }
        // Refused after the hash: the client's decoder spells these as no
        // account (`invalid_name`), including a letter past twelve
        // separators (M1-R2).
        for empty in ["", " ", "___", "_ _", "____________a"] {
            assert_eq!(client_login_name(empty), "invalid_name", "{empty:?}");
            assert!(
                matches!(
                    account_component(empty),
                    Err(HintError::UnsafeAccount(name)) if name == empty
                ),
                "{empty:?} must be refused"
            );
        }
    }

    #[test]
    fn for_account_resolves_under_the_thread_pinned_home() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "Alice Smith").unwrap();
        assert_eq!(
            file.path(),
            scratch
                .home
                .join(".274bot")
                .join("bank-hints")
                .join("local-289")
                .join("alice_smith.json")
        );
        assert_eq!(
            HintFile::for_account("local-289", "alice_smith")
                .unwrap()
                .path(),
            file.path(),
            "space and underscore spell the same account: one file"
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
    fn the_profile_keeps_its_own_component_rule() {
        let scratch = IsolatedEnv::enter("bank-hints");
        // Any profile name the server list accepts keeps its persistence:
        // there is no account-sized limit on it, and `.` is legal in it.
        let long = "a".repeat(65);
        assert!(valid_component(&long));
        let file = HintFile::for_account(&long, "alice").unwrap();
        assert_eq!(
            file.path(),
            scratch
                .home
                .join(".274bot")
                .join("bank-hints")
                .join(&long)
                .join("alice.json")
        );
        let mut memory = observed(&[(LOBSTER, 1)], 9);
        assert!(save_if_dirty(&file, &mut memory).unwrap());
        let mut loaded = BankMemory::default();
        assert!(load_into(&file, &mut loaded).unwrap());
        assert_eq!(loaded.rows(), &[(LOBSTER, 1)]);
        assert!(HintFile::for_account("rel.2", "alice").is_ok());
        // The two rules differ where the server list does: a space is an
        // account separator but never a profile character.
        assert!(matches!(
            HintFile::for_account("local 289", "alice"),
            Err(HintError::UnsafeProfile(_))
        ));
        assert!(matches!(
            HintFile::for_account("..", "alice"),
            Err(HintError::UnsafeProfile(_))
        ));
    }

    #[test]
    fn one_account_spelled_two_ways_shares_one_file() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let typed = HintFile::for_account("local-289", "Alice Smith").unwrap();
        let mut memory = observed(&[(COINS, 7)], 3);
        assert!(save_if_dirty(&typed, &mut memory).unwrap());
        let relogged = HintFile::for_account("local-289", "alice_smith").unwrap();
        let mut loaded = BankMemory::default();
        assert!(load_into(&relogged, &mut loaded).unwrap());
        assert_eq!(loaded.rows(), &[(COINS, 7)]);
        let raw = std::fs::read_to_string(relogged.path()).unwrap();
        assert!(
            raw.contains("\"account\":\"alice_smith\""),
            "the file carries the login spelling: {raw}"
        );
    }

    #[test]
    fn save_then_load_round_trips_rows_at_private_mode() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let mut memory = observed(&[(COINS, 3_400), (LOBSTER, 12)], 1_759_700_000);
        assert!(memory.dirty());
        assert!(
            save_if_dirty(&file, &mut memory).unwrap(),
            "a dirty memory is written"
        );
        assert!(!memory.dirty());
        assert!(
            !save_if_dirty(&file, &mut memory).unwrap(),
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
        assert!(load_into(&file, &mut loaded).unwrap());
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
        assert!(!load_into(&file, &mut memory).unwrap());
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
            let error = load_into(&file, &mut memory).unwrap_err();
            assert!(expected(&error), "{label}: {error}");
            assert!(
                !memory.known(),
                "{label}: a refused file leaves the memory Unknown"
            );
        }
        std::fs::write(file.path(), "not json").unwrap();
        let mut memory = BankMemory::default();
        assert!(matches!(
            load_into(&file, &mut memory).unwrap_err(),
            HintError::Json(_)
        ));
        assert!(!memory.known());
    }

    #[test]
    fn save_tolerates_a_separate_scratch_path() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::at(scratch.dir.join("hint.json"), "p", "a");
        let mut memory = observed(&[(LOBSTER, 1)], 9);
        assert!(save_if_dirty(&file, &mut memory).unwrap());
        let mut loaded = BankMemory::default();
        assert!(load_into(&file, &mut loaded).unwrap());
        assert_eq!(loaded.rows(), &[(LOBSTER, 1)]);
        assert_eq!(loaded.observed_at().map(|at| at.unix_secs), Some(9));
    }

    #[test]
    fn an_empty_session_bank_saves_as_no_rows_and_loads_known_empty() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        let mut memory = observed(&[], 4);
        assert!(save_if_dirty(&file, &mut memory).unwrap());
        let mut loaded = BankMemory::default();
        assert!(load_into(&file, &mut loaded).unwrap());
        assert!(loaded.known());
        assert_eq!(loaded.count(LOBSTER), Some(0));
    }

    #[test]
    fn relog_precedence_keeps_a_known_memory_and_loads_an_unknown_one() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let file = HintFile::for_account("local-289", "alice").unwrap();
        // A file on disk holding different rows than the session observed.
        let mut stale = observed(&[(LOBSTER, 1)], 1);
        save_if_dirty(&file, &mut stale).unwrap();

        // Known in process: relog keeps the in-memory rows and never re-reads.
        let mut memory = observed(&[(COINS, 99)], 2);
        if memory.known() {
            memory.relogged();
        } else {
            load_into(&file, &mut memory).unwrap();
        }
        assert_eq!(memory.origin(), Origin::Hint);
        assert_eq!(memory.rows(), &[(COINS, 99)]);
        assert!(memory.dirty(), "the unsaved session is still pending");

        // Unknown in process: the file is loaded.
        let mut fresh = BankMemory::default();
        if fresh.known() {
            fresh.relogged();
        } else {
            load_into(&file, &mut fresh).unwrap();
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
        assert!(save_if_dirty(&file, &mut memory).unwrap());
        assert!(file.path().starts_with(&scratch.home));
        let raw = std::fs::read_to_string(file.path()).unwrap();
        assert!(raw.contains(&format!("[[{COINS},43]]")), "{raw}");
        assert!(raw.contains("\"observed_at_unix\":11"), "{raw}");
        assert!(
            !save_if_dirty(&file, &mut memory).unwrap(),
            "exactly one save per close"
        );
    }
}
