//! Profile rows (S8, OA:10): each account's `uid` and settings in the state
//! database, keyed by (canonical server profile, canonical account), plus
//! the vault binding and the pending two-write records that join the rows
//! to the credential-only vault (`crate::profile_vault`).
//!
//! **Visibility.** A row is live only while its server has a binding and
//! the row carries the binding's generation in state `active`. The rows a
//! first conversion stages before its binding exists are invisible to every
//! join; the next unlock deletes them before it stages again. Activation is
//! one FULL transaction that inserts the binding (the generation the staged
//! rows carry) and the pending seal record, so the complete profile set
//! becomes live atomically while each staging transaction stays within the
//! SQLite heap and WAL bounds. Forgotten-passphrase reset moves the binding
//! to a new generation, which retires every earlier row at once; a later
//! bounded collection deletes retired rows and their settings.
//!
//! **Settings.** One row's settings are stored as parts: the base part
//! (every field except the script settings map) and one part per script
//! settings bag, each split into pieces of at most [`PIECE_BYTES`], so an
//! edit rewrites only the parts it changed and no row value exceeds the
//! SQLite length limit.
//!
//! Every function here runs on the writer thread's connection through
//! [`LocalStateService::profile_sql`]; callers block on their own reply and
//! never on a frame or slot thread.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{Map, Value};
use vault::{FileDigest, ProfileSettings};

use super::sql::{self, Db, SqlFailure};
use super::*;

/// The largest stored piece of one settings part.
pub const PIECE_BYTES: usize = 32 * 1024;
/// The settings bytes one staging transaction writes at most (Main ruling
/// 2026-11-27: bounded chunks keep the 8 MiB SQLite heap gate).
pub const STAGE_BATCH_BYTES: usize = 1024 * 1024;
/// How many rows one collection or cleanup transaction deletes at most.
const COLLECT_ROWS: i64 = 64;
/// How long a caller waits for its profile job before reporting it stuck.
const PROFILE_SQL_TIMEOUT: Duration = Duration::from_secs(60);
/// The base settings part (every field except the script settings map).
const BASE_PART: &str = "";
/// The prefix of one script settings bag's part.
const BAG_PART: &str = "bag:";

pub(super) const SCHEMA: &str = "
CREATE TABLE vault_binding (
    server TEXT PRIMARY KEY CHECK (length(server) BETWEEN 1 AND 64),
    component TEXT NOT NULL UNIQUE CHECK (length(component) BETWEEN 1 AND 255),
    origin TEXT NOT NULL CHECK (origin IN ('import', 'create', 'reset')),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    absent_ok INTEGER NOT NULL CHECK (absent_ok IN (0, 1))
) STRICT, WITHOUT ROWID;
CREATE TABLE vault_pending (
    server TEXT NOT NULL CHECK (length(server) BETWEEN 1 AND 64),
    kind TEXT NOT NULL CHECK (kind IN ('seal', 'rename', 'reset')),
    old_digest BLOB CHECK (old_digest IS NULL OR length(old_digest) = 32),
    new_digest BLOB CHECK (new_digest IS NULL OR length(new_digest) = 32),
    old_row INTEGER,
    new_row INTEGER,
    PRIMARY KEY (server, kind)
) STRICT, WITHOUT ROWID;
CREATE TABLE profile_row (
    id INTEGER PRIMARY KEY,
    server TEXT NOT NULL CHECK (length(server) BETWEEN 1 AND 64),
    account TEXT NOT NULL CHECK (length(account) BETWEEN 1 AND 12),
    username TEXT NOT NULL CHECK (length(username) BETWEEN 1 AND 12),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    state TEXT NOT NULL CHECK (state IN ('active', 'pending', 'retired')),
    uid INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX profile_row_live ON profile_row (server, generation, account)
    WHERE state = 'active';
CREATE INDEX profile_row_server ON profile_row (server, generation, state);
CREATE TABLE profile_setting (
    row INTEGER NOT NULL REFERENCES profile_row (id),
    part TEXT NOT NULL,
    chunk INTEGER NOT NULL CHECK (chunk >= 0),
    body BLOB NOT NULL CHECK (length(body) BETWEEN 1 AND 32768),
    PRIMARY KEY (row, part, chunk)
) STRICT, WITHOUT ROWID;
";

/// Why a profile database operation did not complete.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileSqlError {
    /// The database refused the operation; the detail names the remedy.
    #[error("{0}")]
    Refused(String),
    /// The operation could not run (a busy lock past its deadline, or the
    /// service stopped). Nothing was committed by it.
    #[error("{0}")]
    Unavailable(String),
}

/// How a server's binding was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingOrigin {
    /// The first conversion of a legacy vault.
    Import,
    /// An explicit vault create.
    Create,
    /// Forgotten-passphrase reset of a server that had no binding.
    Reset,
}

impl BindingOrigin {
    fn code(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Create => "create",
            Self::Reset => "reset",
        }
    }

    fn parse(code: &str) -> Option<Self> {
        match code {
            "import" => Some(Self::Import),
            "create" => Some(Self::Create),
            "reset" => Some(Self::Reset),
            _ => None,
        }
    }
}

/// One server's vault binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    /// The bound vault component (lowercase).
    pub component: String,
    pub origin: BindingOrigin,
    /// Rows of this generation are live.
    pub generation: i64,
    /// The vault file may be absent (after reset, or before a new vault is
    /// written): an explicit create may write it.
    pub absent_ok: bool,
}

/// What a pending two-write record guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    Seal,
    Rename,
    Reset,
}

impl PendingKind {
    fn code(self) -> &'static str {
        match self {
            Self::Seal => "seal",
            Self::Rename => "rename",
            Self::Reset => "reset",
        }
    }

    fn parse(code: &str) -> Option<Self> {
        match code {
            "seal" => Some(Self::Seal),
            "rename" => Some(Self::Rename),
            "reset" => Some(Self::Reset),
            _ => None,
        }
    }
}

/// A pending two-write record: the vault image the write expects to replace
/// and the image it publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRecord {
    pub kind: PendingKind,
    /// The image before the write (`None`: a reset of an absent file).
    pub old_digest: Option<FileDigest>,
    /// The image after the write (`None` for a reset).
    pub new_digest: Option<FileDigest>,
    /// A rename's live row.
    pub old_row: Option<i64>,
    /// A rename's prepared row.
    pub new_row: Option<i64>,
}

/// One stored profile row with its decoded settings.
#[derive(Clone, PartialEq)]
pub struct StoredProfile {
    pub id: i64,
    /// The canonical account (`AccountKey::account`).
    pub account: String,
    /// The exact username the credential is stored under.
    pub username: String,
    pub uid: i32,
    pub settings: ProfileSettings,
}

impl std::fmt::Debug for StoredProfile {
    // Settings stay out of debug output; only identities are shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredProfile")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("username", &self.username)
            .field("uid", &self.uid)
            .finish_non_exhaustive()
    }
}

/// One server's joinable state.
#[derive(Debug, Clone, Default)]
pub struct ProfileState {
    pub binding: Option<BindingRow>,
    /// The server the configured component is bound to, if any.
    pub component_server: Option<String>,
    pub pending: Vec<PendingRecord>,
    /// Live rows of the bound generation.
    pub rows: Vec<StoredProfile>,
    /// A pending rename's prepared rows.
    pub prepared: Vec<StoredProfile>,
    /// Rows exist for an unbound server: an interrupted staging.
    pub unbound_rows: bool,
}

impl ProfileState {
    pub fn pending(&self, kind: PendingKind) -> Option<&PendingRecord> {
        self.pending.iter().find(|record| record.kind == kind)
    }
}

/// A new row: its key, exact username, uid and settings.
#[derive(Clone)]
pub struct NewProfileRow {
    pub account: String,
    pub username: String,
    pub uid: i32,
    pub settings: ProfileSettings,
}

/// One item of a staged first conversion: a row (by the caller's key) or
/// one piece of one of its settings parts.
pub enum StageItem {
    Row {
        key: usize,
        account: String,
        username: String,
        uid: i32,
    },
    Piece {
        key: usize,
        part: String,
        chunk: u32,
        body: Vec<u8>,
    },
}

impl StageItem {
    /// The bytes this item writes, for the staging batch bound.
    pub fn bytes(&self) -> usize {
        match self {
            Self::Row { username, .. } => 64 + username.len(),
            Self::Piece { part, body, .. } => 32 + part.len() + body.len(),
        }
    }
}

/// One change to one settings part of a stored row.
pub enum PartChange {
    Put { part: String, body: Vec<u8> },
    Delete { part: String },
}

/// The parts of `settings`: the base part, then one part per bag in key
/// order. Takes the settings so the bags move rather than copy.
pub fn encode_parts(mut settings: ProfileSettings) -> Result<Vec<(String, Vec<u8>)>, String> {
    let bags = std::mem::take(&mut settings.script_settings);
    let mut parts = Vec::with_capacity(1 + bags.len());
    parts.push((
        BASE_PART.to_string(),
        serde_json::to_vec(&settings).map_err(|error| format!("profile settings: {error}"))?,
    ));
    for (key, bag) in bags {
        let body = serde_json::to_vec(&bag).map_err(|error| format!("settings bag: {error}"))?;
        parts.push((format!("{BAG_PART}{key}"), body));
    }
    Ok(parts)
}

/// The part changes that turn `old`'s stored parts into `new`'s.
pub fn diff_parts(old: &ProfileSettings, new: &ProfileSettings) -> Result<Vec<PartChange>, String> {
    let mut changes = Vec::new();
    // Every field except the bags, without listing the fields here: a field
    // added to `ProfileSettings` later is part of the base automatically.
    let base = |settings: &ProfileSettings| -> Result<Vec<u8>, String> {
        let mut value =
            serde_json::to_value(settings).map_err(|error| format!("profile settings: {error}"))?;
        if let Some(object) = value.as_object_mut() {
            object.insert("script_settings".into(), Value::Object(Map::new()));
        }
        serde_json::to_vec(&value).map_err(|error| format!("profile settings: {error}"))
    };
    let new_base = base(new)?;
    if base(old)? != new_base {
        changes.push(PartChange::Put {
            part: BASE_PART.into(),
            body: new_base,
        });
    }
    for (key, bag) in &new.script_settings {
        if old.script_settings.get(key) != Some(bag) {
            changes.push(PartChange::Put {
                part: format!("{BAG_PART}{key}"),
                body: serde_json::to_vec(bag).map_err(|error| format!("settings bag: {error}"))?,
            });
        }
    }
    for key in old.script_settings.keys() {
        if !new.script_settings.contains_key(key) {
            changes.push(PartChange::Delete {
                part: format!("{BAG_PART}{key}"),
            });
        }
    }
    Ok(changes)
}

/// Decode a row's parts back into its settings.
fn decode_parts(parts: BTreeMap<String, Vec<u8>>) -> Result<ProfileSettings, SqlFailure> {
    let mut settings: Option<ProfileSettings> = None;
    let mut bags = BTreeMap::new();
    for (part, body) in parts {
        if part == BASE_PART {
            settings = Some(serde_json::from_slice(&body).map_err(|_| damaged())?);
        } else if let Some(key) = part.strip_prefix(BAG_PART) {
            let bag: Map<String, Value> = serde_json::from_slice(&body).map_err(|_| damaged())?;
            bags.insert(key.to_string(), bag);
        } else {
            return Err(damaged());
        }
    }
    let mut settings = settings.ok_or_else(damaged)?;
    settings.script_settings = bags;
    Ok(settings)
}

fn damaged() -> SqlFailure {
    SqlFailure::refused(
        FailureReason::Checksum,
        "The stored profile rows are damaged. Restore the paired state backup.",
    )
}

fn db_error(error: rusqlite::Error, what: &str) -> SqlFailure {
    sql::wait_of(&error).map_or_else(|| sql::refused_by(error, what), SqlFailure::Wait)
}

/// The WAL pages one transaction that writes `bytes` of settings reserves:
/// every piece page twice (table and index interior) plus fixed overhead.
fn pages_for(bytes: usize) -> u64 {
    (bytes as u64 / sql::PAGE_SIZE + 1) * 2 + 64
}

/// One profile database operation on the writer's connection.
type ProfileOp = Box<dyn FnMut(&mut Db) -> Result<(), SqlFailure> + Send>;

/// One queued profile database operation and its caller's reply.
pub(super) struct ProfileJob {
    run: Mutex<ProfileOp>,
    fail: Box<dyn Fn(String) + Send + Sync>,
    pub(super) retry: Retry,
    pub(super) what: &'static str,
}

impl ProfileJob {
    /// Run the job once on the writer: `Some` asks to retry later.
    pub(super) fn run(&self, db: &mut Db) -> Option<sql::WaitCause> {
        let mut run = self.run.lock();
        match run(db) {
            Ok(()) => None,
            Err(SqlFailure::Wait(cause)) => Some(cause),
            Err(SqlFailure::Refused { detail, .. }) => {
                (self.fail)(detail.to_string());
                None
            }
        }
    }

    /// Settle the caller with `text`: the job will not run.
    pub(super) fn fail(&self, text: String) {
        (self.fail)(text);
    }
}

impl LocalStateService {
    /// Run `op` on the writer's connection and wait for its result. The
    /// caller's thread blocks (an unlock, the profile writer, a maintenance
    /// command); a frame or slot thread never calls this. A busy database is
    /// retried at the delayed intervals until the preference deadline.
    fn profile_sql<T: Send + 'static>(
        &self,
        what: &'static str,
        mut op: impl FnMut(&mut Db) -> Result<T, SqlFailure> + Send + 'static,
    ) -> Result<T, ProfileSqlError> {
        let (reply, answer) = mpsc::sync_channel::<Result<T, ProfileSqlError>>(1);
        let done = reply.clone();
        let job = ProfileJob {
            run: Mutex::new(Box::new(move |db: &mut Db| {
                let mut result = db.open_for_domain().and_then(|()| op(db));
                db.after_failure(&mut result);
                match result {
                    Ok(value) => {
                        let _ = done.try_send(Ok(value));
                        Ok(())
                    }
                    Err(SqlFailure::Wait(cause)) => Err(SqlFailure::Wait(cause)),
                    Err(SqlFailure::Refused { detail, .. }) => {
                        let _ = done.try_send(Err(ProfileSqlError::Refused(detail.to_string())));
                        Ok(())
                    }
                }
            })),
            fail: Box::new(move |text| {
                let _ = reply.try_send(Err(ProfileSqlError::Unavailable(text)));
            }),
            retry: Retry::default(),
            what,
        };
        {
            let mut state = self.shared.state.lock();
            if state.shutdown || !self.shared.start_writer(&mut state) {
                return Err(ProfileSqlError::Unavailable(format!(
                    "{what}: the state service has stopped"
                )));
            }
            state.profile_jobs.push_back(job);
            self.shared.wake.notify_one();
        }
        answer
            .recv_timeout(PROFILE_SQL_TIMEOUT)
            .unwrap_or_else(|_| {
                Err(ProfileSqlError::Unavailable(format!(
                    "{what}: the state service did not answer"
                )))
            })
    }

    /// Read `server`'s binding, pending records and live rows, and which
    /// server binds `component`.
    pub fn profile_state(
        &self,
        server: &str,
        component: &str,
    ) -> Result<ProfileState, ProfileSqlError> {
        let (server, component) = (server.to_string(), component.to_string());
        self.profile_sql("reading profile rows", move |db| {
            read_state(db, &server, &component)
        })
    }

    /// Delete one bounded batch of an unbound server's interrupted staging.
    /// `true` while more remains. Refused once the server is bound.
    pub fn clear_unbound_profiles(&self, server: &str) -> Result<bool, ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("clearing an interrupted profile staging", move |db| {
            write(db, "clearing staged profile rows", 256, |tx| {
                if binding_of(tx, &server)?.is_some() {
                    return Err(SqlFailure::refused(
                        FailureReason::Identity,
                        "This server is already bound; its rows are not staging.",
                    ));
                }
                delete_rows_batch(
                    tx,
                    "SELECT id FROM profile_row WHERE server = ?1 LIMIT ?2",
                    params![server, COLLECT_ROWS],
                )
            })
        })
    }

    /// Forget every vault binding, pending step and profile row: the state
    /// a fresh root has. For in-process tests that run several live
    /// preparations, each with its own throwaway vault, over one root (a
    /// live run otherwise gets a fresh isolated root of its own).
    #[cfg(any(test, feature = "test-support"))]
    pub fn forget_profiles_for_test(&self) -> Result<(), ProfileSqlError> {
        self.profile_sql("forgetting the profile rows", |db| {
            write(db, "forgetting the profile rows", sql::MAX_PAGES, |tx| {
                tx.execute_batch(
                    "DELETE FROM profile_setting; DELETE FROM profile_row;
                     DELETE FROM vault_pending; DELETE FROM vault_binding;",
                )
                .map_err(|error| db_error(error, "forgetting the profile rows"))
            })
        })
    }

    /// Admit a first conversion that writes `bytes` of settings: the
    /// database's occupied pages plus the import's pages stay within the
    /// whole cap (`P + B + R`). Reserved before any row is staged.
    pub fn admit_profile_import(
        &self,
        bytes: usize,
    ) -> Result<Result<(), String>, ProfileSqlError> {
        self.profile_sql("admitting a profile import", move |db| {
            let conn = db.conn.as_ref().ok_or_else(sql::closed)?;
            let pages: u64 = conn
                .query_row("PRAGMA page_count", [], |row| {
                    row.get::<_, u32>(0).map(u64::from)
                })
                .map_err(|error| db_error(error, "counting database pages"))?;
            let free: u64 = conn
                .query_row("PRAGMA freelist_count", [], |row| {
                    row.get::<_, u32>(0).map(u64::from)
                })
                .map_err(|error| db_error(error, "counting free database pages"))?;
            let need = pages_for(bytes) / 2;
            let occupied = pages.saturating_sub(free);
            Ok(if occupied + need > sql::DB_TOTAL_PAGES {
                Err(format!(
                    "The legacy vault's settings need about {need} database pages, and the state \
                     database has {} of {} free. Nothing was imported; the vault stays as it is.",
                    sql::DB_TOTAL_PAGES.saturating_sub(occupied),
                    sql::DB_TOTAL_PAGES
                ))
            } else {
                Ok(())
            })
        })
    }

    /// The generation a new staging or binding of `server` uses: above
    /// every row the server has.
    pub fn next_profile_generation(&self, server: &str) -> Result<i64, ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("reading the profile generation", move |db| {
            let conn = db.conn.as_ref().ok_or_else(sql::closed)?;
            next_generation(conn, &server)
        })
    }

    /// Stage one bounded batch of a first conversion's rows under the unbound
    /// `server` at `generation`. `ids` maps the caller's row keys to stored
    /// ids across batches and is returned updated.
    pub fn stage_profiles(
        &self,
        server: &str,
        generation: i64,
        items: Vec<StageItem>,
        ids: HashMap<usize, i64>,
    ) -> Result<HashMap<usize, i64>, ProfileSqlError> {
        let server = server.to_string();
        let bytes: usize = items.iter().map(StageItem::bytes).sum();
        let mut job = Some((items, ids));
        self.profile_sql("staging profile rows", move |db| {
            let Some((items, ids)) = job.as_ref() else {
                return Err(SqlFailure::refused(FailureReason::Io, "staging job reused"));
            };
            let ids = write(db, "staging profile rows", pages_for(bytes), |tx| {
                if binding_of(tx, &server)?.is_some() {
                    return Err(SqlFailure::refused(
                        FailureReason::Identity,
                        "This server was bound while its rows were staged. Nothing was imported.",
                    ));
                }
                stage(tx, &server, generation, items, ids.clone())
            })?;
            job = None;
            crate::profile_vault::crash_point("stage-batch-committed");
            Ok(ids)
        })
    }

    /// Activate a staged first conversion in one FULL transaction: insert the
    /// binding at the staged generation and the pending seal record with the
    /// source and replacement digests. Refused when the server or component
    /// is bound or the staged set is not `expected_rows` rows.
    pub fn activate_seal(
        &self,
        server: &str,
        component: &str,
        generation: i64,
        expected_rows: usize,
        old_digest: FileDigest,
        new_digest: FileDigest,
    ) -> Result<(), ProfileSqlError> {
        let (server, component) = (server.to_string(), component.to_string());
        self.profile_sql("activating the profile conversion", move |db| {
            write(db, "activating the profile conversion", 64, |tx| {
                if binding_of(tx, &server)?.is_some() || component_owner(tx, &component)?.is_some()
                {
                    return Err(SqlFailure::refused(
                        FailureReason::Identity,
                        "The server or its vault component is already bound. Nothing was imported.",
                    ));
                }
                let staged: i64 = tx
                    .query_row(
                        "SELECT count(*) FROM profile_row WHERE server = ?1 AND generation = ?2 \
                         AND state = 'active'",
                        params![server, generation],
                        |row| row.get(0),
                    )
                    .map_err(|error| db_error(error, "counting staged profile rows"))?;
                if staged != expected_rows as i64 {
                    return Err(SqlFailure::refused(
                        FailureReason::Dirty,
                        "The staged profile rows are incomplete. Nothing was imported; unlock again.",
                    ));
                }
                insert_binding(tx, &server, &component, BindingOrigin::Import, generation, false)?;
                insert_pending(
                    tx,
                    &server,
                    PendingKind::Seal,
                    Some(&old_digest),
                    Some(&new_digest),
                    None,
                    None,
                )?;
                crate::profile_vault::crash_point("before-activation-commit");
                Ok(())
            })
        })
    }

    /// Replace a pending seal's expected replacement digest (a recovered seal
    /// prepares a new image from the unchanged source).
    pub fn reprepare_seal(
        &self,
        server: &str,
        new_digest: FileDigest,
    ) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("preparing the pending seal again", move |db| {
            write(db, "preparing the pending seal again", 16, |tx| {
                let changed = tx
                    .execute(
                        "UPDATE vault_pending SET new_digest = ?2 WHERE server = ?1 AND kind = 'seal'",
                        params![server, new_digest.as_slice()],
                    )
                    .map_err(|error| db_error(error, "updating the pending seal"))?;
                if changed != 1 {
                    return Err(SqlFailure::refused(
                        FailureReason::Dirty,
                        "No pending seal remains for this server.",
                    ));
                }
                Ok(())
            })
        })
    }

    /// Settle a published seal: delete its pending record. The binding stays
    /// and closes the first import.
    pub fn settle_seal(&self, server: &str) -> Result<(), ProfileSqlError> {
        self.delete_pending(server, PendingKind::Seal, "settling the vault seal")
    }

    /// Delete `server`'s pending `kind` record.
    fn delete_pending(
        &self,
        server: &str,
        kind: PendingKind,
        what: &'static str,
    ) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql(what, move |db| {
            write(db, what, 16, |tx| {
                tx.execute(
                    "DELETE FROM vault_pending WHERE server = ?1 AND kind = ?2",
                    params![server, kind.code()],
                )
                .map_err(|error| db_error(error, what))?;
                Ok(())
            })
        })
    }

    /// Retire `ids` (rows whose credential is gone: an interrupted create or
    /// delete, or a confirmed delete after its credential was hidden).
    pub fn retire_profiles(&self, server: &str, ids: Vec<i64>) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("retiring profile rows", move |db| {
            write(db, "retiring profile rows", 64 + ids.len() as u64, |tx| {
                let mut retire = tx
                    .prepare(
                        "UPDATE profile_row SET state = 'retired' WHERE id = ?1 AND server = ?2",
                    )
                    .map_err(|error| db_error(error, "retiring profile rows"))?;
                for id in &ids {
                    retire
                        .execute(params![id, server])
                        .map_err(|error| db_error(error, "retiring profile rows"))?;
                }
                Ok(())
            })
        })
    }

    /// Insert `rows` live under the bound `server` in one transaction (a
    /// create prepares its row before its credential). A live row of the
    /// same canonical account refuses the whole set; retired rows never do.
    /// Returns the stored ids in order.
    pub fn create_profiles(
        &self,
        server: &str,
        rows: Vec<NewProfileRow>,
    ) -> Result<Result<Vec<i64>, String>, ProfileSqlError> {
        let server = server.to_string();
        let mut encoded = Vec::with_capacity(rows.len());
        let mut bytes = 0;
        for row in rows {
            let parts = encode_parts(row.settings).map_err(ProfileSqlError::Refused)?;
            bytes += parts.iter().map(|(_, body)| body.len()).sum::<usize>();
            encoded.push((row.account, row.username, row.uid, parts));
        }
        self.profile_sql("creating profile rows", move |db| {
            write(db, "creating profile rows", pages_for(bytes), |tx| {
                let Some(binding) = binding_of(tx, &server)? else {
                    return Ok(Err("This server has no vault binding.".to_string()));
                };
                let mut ids = Vec::with_capacity(encoded.len());
                for (account, username, uid, parts) in &encoded {
                    if live_row(tx, &server, binding.generation, account)?.is_some() {
                        return Ok(Err(format!(
                            "A profile for account {account} already exists on this server."
                        )));
                    }
                    let id = insert_row(
                        tx,
                        &server,
                        account,
                        username,
                        binding.generation,
                        "active",
                        *uid,
                    )?;
                    insert_parts(tx, id, parts)?;
                    ids.push(id);
                }
                Ok(Ok(ids))
            })
        })
    }

    /// Apply settings part changes to live rows in one transaction.
    pub fn update_profiles(
        &self,
        server: &str,
        changes: Vec<(i64, Vec<PartChange>)>,
    ) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        let bytes: usize = changes
            .iter()
            .flat_map(|(_, parts)| parts.iter())
            .map(|change| match change {
                PartChange::Put { body, .. } => body.len(),
                PartChange::Delete { .. } => 0,
            })
            .sum();
        self.profile_sql("saving profile settings", move |db| {
            write(db, "saving profile settings", pages_for(bytes), |tx| {
                for (id, parts) in &changes {
                    let live: Option<String> = tx
                        .query_row(
                            "SELECT state FROM profile_row WHERE id = ?1 AND server = ?2",
                            params![id, server],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|error| db_error(error, "reading a profile row"))?;
                    if live.as_deref() != Some("active") {
                        return Err(SqlFailure::refused(
                            FailureReason::Dirty,
                            "A profile row changed under this save. Nothing was saved; unlock again.",
                        ));
                    }
                    for change in parts {
                        match change {
                            PartChange::Put { part, body } => {
                                delete_part(tx, *id, part)?;
                                insert_part(tx, *id, part, body)?;
                            }
                            PartChange::Delete { part } => delete_part(tx, *id, part)?,
                        }
                    }
                }
                Ok(())
            })
        })
    }

    /// Prepare a rename: insert the renamed row as `pending` (uid kept,
    /// settings from the combined edit) and the pending rename record with
    /// both vault digests. The old row stays live. Refused when another live
    /// row owns the target account.
    pub fn prepare_rename(
        &self,
        server: &str,
        old_row: i64,
        row: NewProfileRow,
        old_digest: FileDigest,
        new_digest: FileDigest,
    ) -> Result<Result<i64, String>, ProfileSqlError> {
        let server = server.to_string();
        let parts = encode_parts(row.settings).map_err(ProfileSqlError::Refused)?;
        let bytes = parts.iter().map(|(_, body)| body.len()).sum();
        let (account, username, uid) = (row.account, row.username, row.uid);
        self.profile_sql("preparing a profile rename", move |db| {
            write(db, "preparing a profile rename", pages_for(bytes), |tx| {
                let Some(binding) = binding_of(tx, &server)? else {
                    return Ok(Err("This server has no vault binding.".to_string()));
                };
                if !read_pending(tx, &server)?.is_empty() {
                    return Ok(Err(
                        "Another vault write of this server is still pending.".to_string()
                    ));
                }
                match live_row(tx, &server, binding.generation, &account)? {
                    Some(owner) if owner != old_row => {
                        return Ok(Err(format!(
                            "Account {account} already belongs to another profile."
                        )));
                    }
                    _ => {}
                }
                let id = insert_row(
                    tx,
                    &server,
                    &account,
                    &username,
                    binding.generation,
                    "pending",
                    uid,
                )?;
                insert_parts(tx, id, &parts)?;
                insert_pending(
                    tx,
                    &server,
                    PendingKind::Rename,
                    Some(&old_digest),
                    Some(&new_digest),
                    Some(old_row),
                    Some(id),
                )?;
                Ok(Ok(id))
            })
        })
    }

    /// Finalize a published rename in one transaction: the prepared row
    /// becomes live and the old row is retired (a spelling-only rename of
    /// the same account replaces the row in place), and the record goes.
    pub fn finalize_rename(&self, server: &str) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("finalizing a profile rename", move |db| {
            write(db, "finalizing a profile rename", 64, |tx| {
                let record = read_pending(tx, &server)?
                    .into_iter()
                    .find(|record| record.kind == PendingKind::Rename)
                    .ok_or_else(|| {
                        SqlFailure::refused(FailureReason::Dirty, "No pending rename remains.")
                    })?;
                let (Some(old_row), Some(new_row)) = (record.old_row, record.new_row) else {
                    return Err(damaged());
                };
                tx.execute(
                    "UPDATE profile_row SET state = 'retired' WHERE id = ?1 AND server = ?2",
                    params![old_row, server],
                )
                .map_err(|error| db_error(error, "retiring the renamed row"))?;
                tx.execute(
                    "UPDATE profile_row SET state = 'active' WHERE id = ?1 AND server = ?2",
                    params![new_row, server],
                )
                .map_err(|error| db_error(error, "activating the renamed row"))?;
                tx.execute(
                    "DELETE FROM vault_pending WHERE server = ?1 AND kind = 'rename'",
                    params![server],
                )
                .map_err(|error| db_error(error, "settling the rename"))?;
                crate::profile_vault::crash_point("before-rename-finalize-commit");
                Ok(())
            })
        })
    }

    /// Discard a rename whose vault replacement never happened: its
    /// prepared row is retired and the record goes; the old row stays live.
    pub fn discard_rename(&self, server: &str) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("discarding a profile rename", move |db| {
            write(db, "discarding a profile rename", 64, |tx| {
                if let Some(record) = read_pending(tx, &server)?
                    .into_iter()
                    .find(|record| record.kind == PendingKind::Rename)
                {
                    tx.execute(
                        "UPDATE profile_row SET state = 'retired' WHERE id = ?1 AND server = ?2",
                        params![record.new_row, server],
                    )
                    .map_err(|error| db_error(error, "discarding the prepared row"))?;
                }
                tx.execute(
                    "DELETE FROM vault_pending WHERE server = ?1 AND kind = 'rename'",
                    params![server],
                )
                .map_err(|error| db_error(error, "discarding the rename"))?;
                Ok(())
            })
        })
    }

    /// Record a forgotten-passphrase reset of `server`'s vault at `component`
    /// before its file is deleted: joins are blocked while it is pending.
    /// `digest` is the confirmed file's image (`None`: no file is there).
    pub fn prepare_reset(
        &self,
        server: &str,
        component: &str,
        digest: Option<FileDigest>,
    ) -> Result<Result<(), String>, ProfileSqlError> {
        let (server, component) = (server.to_string(), component.to_string());
        self.profile_sql("preparing a vault reset", move |db| {
            write(db, "preparing a vault reset", 16, |tx| {
                if let Err(refusal) = check_binding(tx, &server, &component)? {
                    return Ok(Err(refusal));
                }
                tx.execute(
                    "DELETE FROM vault_pending WHERE server = ?1 AND kind = 'reset'",
                    params![server],
                )
                .map_err(|error| db_error(error, "preparing a vault reset"))?;
                insert_pending(
                    tx,
                    &server,
                    PendingKind::Reset,
                    digest.as_ref(),
                    None,
                    None,
                    None,
                )?;
                Ok(Ok(()))
            })
        })
    }

    /// Settle a reset whose file is absent, in one transaction: the binding
    /// moves to a new generation (every earlier row is retired), any pending
    /// seal or rename is cancelled, and the file may now be created. A
    /// server with no binding gets one, so the first import stays closed.
    pub fn finish_reset(&self, server: &str, component: &str) -> Result<(), ProfileSqlError> {
        let (server, component) = (server.to_string(), component.to_string());
        self.profile_sql("settling a vault reset", move |db| {
            write(db, "settling a vault reset", 64, |tx| {
                let generation = next_generation(tx, &server)?;
                match binding_of(tx, &server)? {
                    Some(_) => {
                        tx.execute(
                            "UPDATE vault_binding SET generation = ?2, absent_ok = 1 WHERE server = ?1",
                            params![server, generation],
                        )
                        .map_err(|error| db_error(error, "moving the binding generation"))?;
                    }
                    None => insert_binding(
                        tx,
                        &server,
                        &component,
                        BindingOrigin::Reset,
                        generation,
                        true,
                    )?,
                }
                tx.execute(
                    "UPDATE profile_row SET state = 'retired' WHERE server = ?1 AND state = 'pending'",
                    params![server],
                )
                .map_err(|error| db_error(error, "cancelling a prepared rename"))?;
                tx.execute("DELETE FROM vault_pending WHERE server = ?1", params![server])
                    .map_err(|error| db_error(error, "cancelling pending vault writes"))?;
                Ok(())
            })
        })
    }

    /// Cancel a pending reset whose file is still the confirmed one.
    pub fn cancel_reset(&self, server: &str) -> Result<(), ProfileSqlError> {
        self.delete_pending(server, PendingKind::Reset, "cancelling a vault reset")
    }

    /// Bind `server` to `component` for an explicit vault create, before the
    /// file is written: a new binding (`absent_ok`), or the existing one
    /// when it allows an absent file. Refused for any other binding.
    pub fn bind_created(
        &self,
        server: &str,
        component: &str,
    ) -> Result<Result<(), String>, ProfileSqlError> {
        let (server, component) = (server.to_string(), component.to_string());
        self.profile_sql("binding a new vault", move |db| {
            write(db, "binding a new vault", 16, |tx| {
                if let Err(refusal) = check_binding(tx, &server, &component)? {
                    return Ok(Err(refusal));
                }
                match binding_of(tx, &server)? {
                    Some(binding) if binding.absent_ok => Ok(Ok(())),
                    Some(_) => Ok(Err(
                        "This server's vault exists in the state database but its file is \
                         missing. Restore the vault file, or reset it first."
                            .to_string(),
                    )),
                    None => {
                        if !read_pending(tx, &server)?.is_empty() {
                            return Ok(Err("A vault write of this server is pending.".into()));
                        }
                        let generation = next_generation(tx, &server)?;
                        insert_binding(
                            tx,
                            &server,
                            &component,
                            BindingOrigin::Create,
                            generation,
                            true,
                        )?;
                        Ok(Ok(()))
                    }
                }
            })
        })
    }

    /// Record that `server`'s vault file exists again.
    pub fn mark_vault_present(&self, server: &str) -> Result<(), ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("recording the new vault", move |db| {
            write(db, "recording the new vault", 16, |tx| {
                tx.execute(
                    "UPDATE vault_binding SET absent_ok = 0 WHERE server = ?1",
                    params![server],
                )
                .map_err(|error| db_error(error, "recording the new vault"))?;
                Ok(())
            })
        })
    }

    /// Delete one bounded batch of `server`'s retired and earlier-generation
    /// rows and their settings. `true` while more remains.
    pub fn collect_retired_profiles(&self, server: &str) -> Result<bool, ProfileSqlError> {
        let server = server.to_string();
        self.profile_sql("collecting retired profile rows", move |db| {
            write(db, "collecting retired profile rows", 256, |tx| {
                let Some(binding) = binding_of(tx, &server)? else {
                    return Ok(false);
                };
                delete_rows_batch(
                    tx,
                    "SELECT id FROM profile_row WHERE server = ?1 \
                     AND (state = 'retired' OR generation <> ?3) \
                     AND id NOT IN (SELECT new_row FROM vault_pending \
                                    WHERE server = ?1 AND new_row IS NOT NULL) \
                     LIMIT ?2",
                    params![server, COLLECT_ROWS, binding.generation],
                )
            })
        })
    }

    /// Bind an unbound `server` for missing-row recovery and insert `rows`
    /// with fresh uids (maintenance under the exclusive lease).
    pub fn recover_profiles(
        &self,
        server: &str,
        component: &str,
        rows: Vec<NewProfileRow>,
    ) -> Result<Result<Vec<i64>, String>, ProfileSqlError> {
        let (server_owned, component) = (server.to_string(), component.to_string());
        let bound = self.profile_sql("binding a recovered vault", move |db| {
            write(db, "binding a recovered vault", 16, |tx| {
                if let Err(refusal) = check_binding(tx, &server_owned, &component)? {
                    return Ok(Err(refusal));
                }
                if binding_of(tx, &server_owned)?.is_none() {
                    let generation = next_generation(tx, &server_owned)?;
                    insert_binding(
                        tx,
                        &server_owned,
                        &component,
                        BindingOrigin::Create,
                        generation,
                        false,
                    )?;
                }
                Ok(Ok(()))
            })
        })?;
        if let Err(refusal) = bound {
            return Ok(Err(refusal));
        }
        self.create_profiles(server, rows)
    }
}

/// Begin a write under the product guard with `pages` reserved, run `f`,
/// and commit.
fn write<T>(
    db: &mut Db,
    what: &str,
    pages: u64,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, SqlFailure>,
) -> Result<T, SqlFailure> {
    let guard = db.guard()?;
    let conn = db.conn.as_mut().ok_or_else(sql::closed)?;
    let tx =
        sql::begin_write_reserved(conn, &db.root, &guard, what, pages.min(sql::WAL_MAX_FRAMES))?;
    let value = f(&tx)?;
    tx.commit().map_err(|error| db_error(error, what))?;
    drop(guard);
    Ok(value)
}

fn read_state(db: &mut Db, server: &str, component: &str) -> Result<ProfileState, SqlFailure> {
    let conn = db.conn.as_mut().ok_or_else(sql::closed)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|error| db_error(error, "starting a profile read"))?;
    let binding = binding_of(&tx, server)?;
    let component_server = component_owner(&tx, component)?;
    let pending = read_pending(&tx, server)?;
    let mut state = ProfileState {
        binding: binding.clone(),
        component_server,
        pending,
        ..ProfileState::default()
    };
    match binding {
        None => {
            state.unbound_rows = tx
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM profile_row WHERE server = ?1)",
                    params![server],
                    |row| row.get(0),
                )
                .map_err(|error| db_error(error, "reading staged profile rows"))?;
        }
        Some(binding) => {
            let mut rows = read_rows(&tx, server, binding.generation)?;
            for row in rows.drain(..) {
                if row.1 == "pending" {
                    state.prepared.push(row.0);
                } else {
                    state.rows.push(row.0);
                }
            }
        }
    }
    tx.commit()
        .map_err(|error| db_error(error, "ending a profile read"))?;
    Ok(state)
}

fn read_rows(
    tx: &Connection,
    server: &str,
    generation: i64,
) -> Result<Vec<(StoredProfile, String)>, SqlFailure> {
    let mut statement = tx
        .prepare(
            "SELECT id, account, username, uid, state FROM profile_row \
             WHERE server = ?1 AND generation = ?2 AND state IN ('active', 'pending') ORDER BY id",
        )
        .map_err(|error| db_error(error, "reading profile rows"))?;
    let heads: Vec<(i64, String, String, i32, String)> = statement
        .query_map(params![server, generation], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|error| db_error(error, "reading profile rows"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|error| db_error(error, "reading profile rows"))?;
    let mut parts: HashMap<i64, BTreeMap<String, Vec<u8>>> = HashMap::with_capacity(heads.len());
    let mut pieces = tx
        .prepare(
            "SELECT s.row, s.part, s.chunk, s.body FROM profile_setting s \
             JOIN profile_row r ON r.id = s.row \
             WHERE r.server = ?1 AND r.generation = ?2 AND r.state IN ('active', 'pending') \
             ORDER BY s.row, s.part, s.chunk",
        )
        .map_err(|error| db_error(error, "reading profile settings"))?;
    let mut rows = pieces
        .query(params![server, generation])
        .map_err(|error| db_error(error, "reading profile settings"))?;
    let mut expected: Option<(i64, String, i64)> = None;
    while let Some(row) = rows
        .next()
        .map_err(|error| db_error(error, "reading profile settings"))?
    {
        let id: i64 = row.get(0).map_err(|_| damaged())?;
        let part: String = row.get(1).map_err(|_| damaged())?;
        let chunk: i64 = row.get(2).map_err(|_| damaged())?;
        let body = row
            .get_ref(3)
            .map_err(|_| damaged())?
            .as_blob()
            .map_err(|_| damaged())?;
        let next = match &expected {
            Some((last_id, last_part, last_chunk)) if *last_id == id && *last_part == part => {
                last_chunk + 1
            }
            _ => 0,
        };
        if chunk != next {
            return Err(damaged());
        }
        parts
            .entry(id)
            .or_default()
            .entry(part.clone())
            .or_default()
            .extend_from_slice(body);
        expected = Some((id, part, chunk));
    }
    heads
        .into_iter()
        .map(|(id, account, username, uid, state)| {
            let settings = decode_parts(parts.remove(&id).unwrap_or_default())?;
            Ok((
                StoredProfile {
                    id,
                    account,
                    username,
                    uid,
                    settings,
                },
                state,
            ))
        })
        .collect()
}

fn binding_of(tx: &Connection, server: &str) -> Result<Option<BindingRow>, SqlFailure> {
    let row: Option<(String, String, i64, i64)> = tx
        .query_row(
            "SELECT component, origin, generation, absent_ok FROM vault_binding WHERE server = ?1",
            params![server],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| db_error(error, "reading the vault binding"))?;
    row.map(|(component, origin, generation, absent_ok)| {
        Ok(BindingRow {
            component,
            origin: BindingOrigin::parse(&origin).ok_or_else(damaged)?,
            generation,
            absent_ok: absent_ok != 0,
        })
    })
    .transpose()
}

fn component_owner(tx: &Connection, component: &str) -> Result<Option<String>, SqlFailure> {
    tx.query_row(
        "SELECT server FROM vault_binding WHERE component = ?1",
        params![component],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| db_error(error, "reading the vault binding"))
}

/// `Err` with the operator's remedy when `server` is bound to another
/// component or `component` is bound to another server.
fn check_binding(
    tx: &Connection,
    server: &str,
    component: &str,
) -> Result<Result<(), String>, SqlFailure> {
    if let Some(binding) = binding_of(tx, server)? {
        if binding.component != component {
            return Ok(Err(binding_mismatch(server, component, &binding.component)));
        }
    }
    if let Some(owner) = component_owner(tx, component)? {
        if owner != server {
            return Ok(Err(component_taken(server, component, &owner)));
        }
    }
    Ok(Ok(()))
}

/// The refusal for a server whose configured component is not its binding.
pub fn binding_mismatch(server: &str, configured: &str, bound: &str) -> String {
    format!(
        "Server profile {server} is bound to vault component {bound:?}, but its configuration \
         names {configured:?}. Restore the configured binding (vault {bound:?}) in the server \
         roster. Nothing was opened or changed."
    )
}

/// The refusal for a component another server is bound to.
pub fn component_taken(server: &str, component: &str, owner: &str) -> String {
    format!(
        "Vault component {component:?} is bound to server profile {owner}, not {server}. \
         Restore the configured binding in the server roster. Nothing was opened or changed."
    )
}

fn read_pending(tx: &Connection, server: &str) -> Result<Vec<PendingRecord>, SqlFailure> {
    let mut statement = tx
        .prepare(
            "SELECT kind, old_digest, new_digest, old_row, new_row FROM vault_pending \
             WHERE server = ?1 ORDER BY kind",
        )
        .map_err(|error| db_error(error, "reading pending vault writes"))?;
    type PendingColumns = (
        String,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<i64>,
        Option<i64>,
    );
    let rows: Vec<PendingColumns> = statement
        .query_map(params![server], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|error| db_error(error, "reading pending vault writes"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| db_error(error, "reading pending vault writes"))?;
    let digest = |raw: Option<Vec<u8>>| -> Result<Option<FileDigest>, SqlFailure> {
        raw.map(|raw| FileDigest::try_from(raw.as_slice()).map_err(|_| damaged()))
            .transpose()
    };
    rows.into_iter()
        .map(|(kind, old, new, old_row, new_row)| {
            Ok(PendingRecord {
                kind: PendingKind::parse(&kind).ok_or_else(damaged)?,
                old_digest: digest(old)?,
                new_digest: digest(new)?,
                old_row,
                new_row,
            })
        })
        .collect()
}

fn next_generation(tx: &Connection, server: &str) -> Result<i64, SqlFailure> {
    let rows: i64 = tx
        .query_row(
            "SELECT coalesce(max(generation), 0) FROM profile_row WHERE server = ?1",
            params![server],
            |row| row.get(0),
        )
        .map_err(|error| db_error(error, "reading the profile generation"))?;
    let bound: i64 = tx
        .query_row(
            "SELECT coalesce(max(generation), 0) FROM vault_binding WHERE server = ?1",
            params![server],
            |row| row.get(0),
        )
        .map_err(|error| db_error(error, "reading the profile generation"))?;
    rows.max(bound).checked_add(1).ok_or_else(damaged)
}

fn insert_binding(
    tx: &Connection,
    server: &str,
    component: &str,
    origin: BindingOrigin,
    generation: i64,
    absent_ok: bool,
) -> Result<(), SqlFailure> {
    tx.execute(
        "INSERT INTO vault_binding (server, component, origin, generation, absent_ok) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            server,
            component,
            origin.code(),
            generation,
            i64::from(absent_ok)
        ],
    )
    .map_err(|error| db_error(error, "binding the vault"))?;
    Ok(())
}

fn insert_pending(
    tx: &Connection,
    server: &str,
    kind: PendingKind,
    old: Option<&FileDigest>,
    new: Option<&FileDigest>,
    old_row: Option<i64>,
    new_row: Option<i64>,
) -> Result<(), SqlFailure> {
    tx.execute(
        "INSERT INTO vault_pending (server, kind, old_digest, new_digest, old_row, new_row) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            server,
            kind.code(),
            old.map(|digest| digest.as_slice()),
            new.map(|digest| digest.as_slice()),
            old_row,
            new_row
        ],
    )
    .map_err(|error| db_error(error, "recording a pending vault write"))?;
    Ok(())
}

fn live_row(
    tx: &Connection,
    server: &str,
    generation: i64,
    account: &str,
) -> Result<Option<i64>, SqlFailure> {
    tx.query_row(
        "SELECT id FROM profile_row WHERE server = ?1 AND generation = ?2 AND account = ?3 \
         AND state = 'active'",
        params![server, generation, account],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| db_error(error, "reading a live profile row"))
}

fn insert_row(
    tx: &Connection,
    server: &str,
    account: &str,
    username: &str,
    generation: i64,
    state: &str,
    uid: i32,
) -> Result<i64, SqlFailure> {
    tx.prepare(
        "INSERT INTO profile_row (server, account, username, generation, state, uid) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .and_then(|mut insert| {
        insert.execute(params![server, account, username, generation, state, uid])
    })
    .map_err(|error| db_error(error, "inserting a profile row"))?;
    Ok(tx.last_insert_rowid())
}

fn insert_parts(tx: &Connection, row: i64, parts: &[(String, Vec<u8>)]) -> Result<(), SqlFailure> {
    for (part, body) in parts {
        insert_part(tx, row, part, body)?;
    }
    Ok(())
}

fn insert_part(tx: &Connection, row: i64, part: &str, body: &[u8]) -> Result<(), SqlFailure> {
    let mut insert = tx
        .prepare("INSERT INTO profile_setting (row, part, chunk, body) VALUES (?1, ?2, ?3, ?4)")
        .map_err(|error| db_error(error, "writing profile settings"))?;
    for (chunk, piece) in body.chunks(PIECE_BYTES).enumerate() {
        insert
            .execute(params![row, part, chunk as i64, piece])
            .map_err(|error| db_error(error, "writing profile settings"))?;
    }
    Ok(())
}

fn delete_part(tx: &Connection, row: i64, part: &str) -> Result<(), SqlFailure> {
    tx.prepare("DELETE FROM profile_setting WHERE row = ?1 AND part = ?2")
        .and_then(|mut delete| delete.execute(params![row, part]))
        .map_err(|error| db_error(error, "deleting profile settings"))?;
    Ok(())
}

/// Delete the rows `select` names (one bounded batch) with their settings.
/// `true` when the batch was full, so more may remain.
fn delete_rows_batch(
    tx: &Connection,
    select: &str,
    args: impl rusqlite::Params,
) -> Result<bool, SqlFailure> {
    let ids: Vec<i64> = tx
        .prepare(select)
        .and_then(|mut statement| {
            statement
                .query_map(args, |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()
        })
        .map_err(|error| db_error(error, "selecting profile rows to delete"))?;
    for id in &ids {
        tx.execute("DELETE FROM profile_setting WHERE row = ?1", params![id])
            .map_err(|error| db_error(error, "deleting profile settings"))?;
        tx.execute("DELETE FROM profile_row WHERE id = ?1", params![id])
            .map_err(|error| db_error(error, "deleting a profile row"))?;
    }
    Ok(ids.len() as i64 == COLLECT_ROWS)
}

fn stage(
    tx: &Connection,
    server: &str,
    generation: i64,
    items: &[StageItem],
    mut ids: HashMap<usize, i64>,
) -> Result<HashMap<usize, i64>, SqlFailure> {
    let mut piece = tx
        .prepare("INSERT INTO profile_setting (row, part, chunk, body) VALUES (?1, ?2, ?3, ?4)")
        .map_err(|error| db_error(error, "staging profile settings"))?;
    let mut row_insert = tx
        .prepare(
            "INSERT INTO profile_row (server, account, username, generation, state, uid) \
             VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
        )
        .map_err(|error| db_error(error, "staging profile rows"))?;
    for item in items {
        match item {
            StageItem::Row {
                key,
                account,
                username,
                uid,
            } => {
                row_insert
                    .execute(params![server, account, username, generation, uid])
                    .map_err(|error| db_error(error, "staging profile rows"))?;
                let id = tx.last_insert_rowid();
                ids.insert(*key, id);
            }
            StageItem::Piece {
                key,
                part,
                chunk,
                body,
            } => {
                let row = *ids.get(key).ok_or_else(|| {
                    SqlFailure::refused(FailureReason::Io, "a staged piece has no row")
                })?;
                piece
                    .execute(params![row, part, i64::from(*chunk), body])
                    .map_err(|error| db_error(error, "staging profile settings"))?;
            }
        }
    }
    Ok(ids)
}

/// The settings parts of one row as staging items, split into pieces.
pub fn stage_items(key: usize, row: NewProfileRow, out: &mut Vec<StageItem>) -> Result<(), String> {
    let parts = encode_parts(row.settings)?;
    out.push(StageItem::Row {
        key,
        account: row.account,
        username: row.username,
        uid: row.uid,
    });
    for (part, body) in parts {
        for (chunk, piece) in body.chunks(PIECE_BYTES).enumerate() {
            out.push(StageItem::Piece {
                key,
                part: part.clone(),
                chunk: chunk as u32,
                body: piece.to_vec(),
            });
        }
    }
    Ok(())
}
