//! The operator preferences database (T5.3): one SQLite database per state
//! root, `<root>/state/state.sqlite3`, with SQLite's `-wal` and `-shm`
//! beside it. Only the service's writer thread calls this module, so every
//! open, PRAGMA, read, merge, write, checkpoint and close runs there; a bot
//! never holds a connection, a statement, a read transaction or a copy of
//! the document.
//!
//! **Zero lock waits.** The busy timeout is zero and no busy handler is
//! installed. Every mutating transaction (startup schema and epoch work,
//! import, a save, a checkpoint) first takes the product guard
//! (`panel-ui.json.lock`, the lock every preferences writer of this product
//! has always held) without waiting. A held guard, `SQLITE_BUSY`, a full
//! WAL or a maintenance lease answers [`SqlFailure::Wait`]: the caller
//! requeues the job and retries at the existing delayed intervals until the
//! request's own deadline.
//!
//! **Bounded WAL.** Autocheckpoint and close-time checkpoints are off and
//! cache spill is off, so one transaction appends at most [`MAX_PAGES`]
//! frames. A transaction is admitted only while `F + P <= W` frames
//! ([`WAL_MAX_FRAMES`]), where `F` is the WAL's frame count since its last
//! reset. A commit that leaves `F >= W / 2` asks for one idle TRUNCATE
//! checkpoint, which resets the WAL to zero bytes. Every SQL write starts
//! at `begin_write`, which takes a borrowed guard as proof and admits the
//! reservation first: schema, epoch, import and export ledgers, evidence
//! and saves alike.
//!
//! **Shutdown.** A save's transaction has a begun boundary
//! ([`Db::commit`]): before it the service may still cancel the save, after
//! it the transaction keeps its own outcome (commit, rollback or uncertain
//! resolution), so a request is never cancelled and committed at once.
//!
//! **Versions and request keys.** Every durable version is allocated inside
//! the write transaction from the stored sequence, so two processes on one
//! root never commit the same version. Each opening process draws a
//! process epoch from the database once; a commit records the pair (epoch,
//! process-local request id) with the committed document's digest, which
//! is what resolves an uncertain COMMIT.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::hostlog::Level;
use api::identity::RequestId;
use api::local_state::{ErrorCode, FailureReason, LocalStateDomain, StoreVersion};
use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{Map, Value};

use super::files::{self, ContentDigest, StateRoot};
use super::root_use::RootUse;

/// The database's application ID (`PRAGMA application_id`): ASCII `274S`.
/// A database with another non-zero ID is foreign and refused.
pub(super) const APPLICATION_ID: i32 = 0x3237_3453;
/// The schema version this build writes and reads (`PRAGMA user_version`).
pub(super) const USER_VERSION: i32 = 1;
/// The database page size: the neutral bound of one preference row value.
pub(super) const PAGE_SIZE: u64 = 4096;
/// D: the largest preference document, the old file's bound.
pub(super) const DOCUMENT_MAX: u64 = files::MAX_PREFERENCES_BYTES;
/// P: the database page cap, `4D / page`: one live document, one
/// replacement, one for heads, ledgers and schema, one for free pages.
pub(super) const MAX_PAGES: u64 = 4 * DOCUMENT_MAX / PAGE_SIZE;
/// How many preference transactions the queue admits: eight queued and one
/// active.
const TRANSACTION_SLOTS: u64 = 9;
/// W: the hard WAL maximum in frames, root-wide.
pub(super) const WAL_MAX_FRAMES: u64 = TRANSACTION_SLOTS * MAX_PAGES;
/// The frame count after a commit that schedules the idle TRUNCATE.
pub(super) const WAL_TRUNCATE_AT: u64 = WAL_MAX_FRAMES / 2;
/// One WAL frame on disk: its 24-byte header and one page.
const WAL_FRAME_BYTES: u64 = 24 + PAGE_SIZE;
/// The hard WAL maximum in bytes: the 32-byte header and W frames.
pub(super) const WAL_MAX_BYTES: u64 = 32 + WAL_MAX_FRAMES * WAL_FRAME_BYTES;
/// The page cache target (`cache_size` in KiB, negative): 2D.
const CACHE_SIZE: i64 = -(2 * DOCUMENT_MAX as i64 / 1024);
/// The advisory SQLite heap limit: 8D.
pub(super) const SOFT_HEAP_LIMIT: i64 = 8 * DOCUMENT_MAX as i64;
/// The longest string, BLOB or row: one document plus a row's own bytes.
const LENGTH_LIMIT: i32 = DOCUMENT_MAX as i32 + 4096;
/// The longest SQL statement: every statement here is a fixed one.
const SQL_LENGTH_LIMIT: i32 = 4096;
/// The most bound parameters of one statement.
const VARIABLE_LIMIT: i32 = 16;
/// How many recent commits the database keeps for uncertain-COMMIT matching.
const RECENT_COMMITS: i64 = 8;
/// The preferences row of `domain_head`.
const PREFERENCES_HEAD: &str = "preferences";

const SCHEMA: &str = "
CREATE TABLE store_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    epoch BLOB NOT NULL CHECK (length(epoch) = 8),
    sequence BLOB NOT NULL CHECK (length(sequence) = 8)
) STRICT;
CREATE TABLE domain_head (
    domain TEXT PRIMARY KEY,
    version BLOB NOT NULL CHECK (length(version) = 8)
) STRICT;
CREATE TABLE preference_document (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    body BLOB NOT NULL CHECK (length(body) <= 1048576)
) STRICT;
CREATE TABLE recent_commit (
    seq INTEGER PRIMARY KEY,
    epoch BLOB NOT NULL CHECK (length(epoch) = 8),
    request BLOB NOT NULL CHECK (length(request) = 8),
    domain TEXT NOT NULL,
    old_version BLOB NOT NULL CHECK (length(old_version) = 8),
    new_version BLOB NOT NULL CHECK (length(new_version) = 8),
    action TEXT NOT NULL,
    digest BLOB NOT NULL CHECK (length(digest) = 32),
    UNIQUE (epoch, request)
) STRICT;
CREATE TABLE legacy_source (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    digest BLOB CHECK (digest IS NULL OR length(digest) = 32),
    kind TEXT NOT NULL,
    evidence TEXT NOT NULL,
    path TEXT NOT NULL CHECK (length(path) <= 4096)
) STRICT;
CREATE TABLE export_head (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    export_id BLOB NOT NULL CHECK (length(export_id) = 8),
    version BLOB NOT NULL CHECK (length(version) = 8),
    digest BLOB NOT NULL CHECK (length(digest) = 32),
    state TEXT NOT NULL
) STRICT;
INSERT INTO store_meta (id, epoch, sequence)
    VALUES (1, x'0000000000000000', x'0000000000000000');
";

/// Which shipped frontend runs the service: the refusal texts name its
/// maintenance commands, and a headless composition names both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Frontend {
    /// `panel-play`.
    Panel,
    /// `tui-play`.
    Tui,
    /// A headless or test composition: both binaries' forms.
    #[default]
    Headless,
}

impl Frontend {
    /// The binary a maintenance message names first.
    pub(super) fn binary(self) -> &'static str {
        self.binaries()[0]
    }

    fn binaries(self) -> &'static [&'static str] {
        match self {
            Self::Panel => &["panel-play"],
            Self::Tui => &["tui-play"],
            Self::Headless => &["panel-play", "tui-play"],
        }
    }
}

/// Why a database job did not run now. Each is retried at the delayed
/// intervals until the request's deadline, never waited for on the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WaitCause {
    /// Another process of this product holds the preferences guard.
    Guard,
    /// SQLite answered `SQLITE_BUSY`/`SQLITE_LOCKED`: another connection
    /// holds the write lock.
    Busy,
    /// The WAL cannot take another transaction until an idle reset: a
    /// reader keeps the TRUNCATE from completing.
    Wal,
    /// An explicit maintenance command holds the root exclusive.
    Maintenance,
}

impl WaitCause {
    /// What a pending save shows, and the cause a save that reaches its
    /// deadline fails with.
    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Guard => "Preferences not saved: another 274bot process is saving preferences.",
            Self::Busy => {
                "Preferences not saved: another writer holds the preferences database lock."
            }
            Self::Wal => "Preferences not saved: waiting for a SQLite reader to release the WAL.",
            Self::Maintenance => {
                "Preferences not saved: a maintenance command (export or recovery) holds this root."
            }
        }
    }

    /// The cause alone, for a sentence that is not about a save that was
    /// not saved ("another writer holds the preferences database lock").
    pub(super) fn reason(self) -> &'static str {
        let text = self.text();
        text.strip_prefix("Preferences not saved: ")
            .unwrap_or(text)
            .trim_end_matches('.')
    }
}

/// What a database job ended with, short of success.
#[derive(Debug, Clone)]
pub(super) enum SqlFailure {
    /// Requeue and retry later.
    Wait(WaitCause),
    /// The preferences store is refused until a later read accepts it (or
    /// an explicit recovery): leases and saves fail with `reason`.
    Refused {
        reason: FailureReason,
        detail: Arc<str>,
    },
}

impl SqlFailure {
    fn refused(reason: FailureReason, detail: impl Into<String>) -> Self {
        Self::Refused {
            reason,
            detail: Arc::from(detail.into()),
        }
    }
}

/// The durable preference document at one version, as the writer holds it.
#[derive(Debug, Clone)]
pub(super) struct Durable {
    pub(super) version: StoreVersion,
    pub(super) projection: files::PreferenceProjection,
    pub(super) digest: ContentDigest,
    pub(super) len: usize,
}

impl Durable {
    fn parse(version: StoreVersion, body: &[u8]) -> Result<Self, SqlFailure> {
        let Ok(Value::Object(document)) = serde_json::from_slice::<Value>(body) else {
            return Err(SqlFailure::refused(
                FailureReason::Checksum,
                "the stored preferences document is not a JSON object",
            ));
        };
        Ok(Self {
            version,
            projection: files::project_preferences(document),
            digest: files::digest(body),
            len: body.len(),
        })
    }
}

/// What a hydration found: the durable document and the lines the
/// operator and the audit need (an import, a quarantine, a reimport).
pub(super) struct Hydrated {
    pub(super) durable: Durable,
    pub(super) notices: Vec<(Level, String)>,
}

/// How a save's transaction ended.
pub(super) enum Commit<S> {
    /// Committed at `durable.version`.
    Committed {
        staged: S,
        durable: Durable,
        /// What the transaction found when another writer had changed it
        /// since the caller's cached version.
        observed: Option<Durable>,
        /// The commit left `F >= W / 2`: schedule one idle TRUNCATE.
        truncate_due: bool,
        /// `Some` when the COMMIT's own result was uncertain and the
        /// recent-commit row proved it.
        recovered: bool,
    },
    /// The edit changes nothing durable: rolled back, settles at the
    /// current version without a write.
    Unchanged {
        staged: S,
        observed: Option<Durable>,
    },
    /// The edit was refused (conflict, validation, a COMMIT that failed
    /// cleanly): rolled back.
    Declined {
        code: ErrorCode,
        detail: Option<Arc<str>>,
        observed: Option<Durable>,
    },
    /// The shutdown bound passed before this save's transaction began (the
    /// begun boundary of [`Db::commit`]): nothing was written, and the
    /// request is already settled cancelled.
    Abandoned,
}

/// What one TRUNCATE attempt saw: SQLite's busy flag, its log and
/// checkpointed frame counts, and the WAL file's length after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Truncated {
    pub(super) busy: bool,
    pub(super) log: i64,
    pub(super) checkpointed: i64,
    pub(super) wal_bytes: u64,
}

/// A test's injected COMMIT outcome, applied to a real open transaction.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitFault {
    /// Commit for real, then report `SQLITE_IOERR` (the outcome is known
    /// only by the recent-commit row).
    IoErrAfterCommit,
    /// Report `SQLITE_IOERR` without committing.
    IoErrWithoutCommit,
    /// Report `SQLITE_FULL` without committing.
    Full,
    /// Report `SQLITE_READONLY` without committing.
    ReadOnly,
    /// Report `SQLITE_BUSY` without committing.
    Busy,
}

/// What a test sees of the worker connection after a save: `None` when it
/// was closed, else (back in autocommit, every setting read back).
#[cfg(test)]
pub(crate) type ConnectionProbe = Arc<dyn Fn(Option<(bool, bool)>) + Send + Sync>;

/// The test seams of one database: a stall run inside a real SQL statement
/// on the writer (the first hydration, a refresh, a save), a COMMIT
/// outcome, and a failing reconnect after an uncertain COMMIT.
#[cfg(test)]
#[derive(Default, Clone)]
pub(crate) struct Seams {
    pub(crate) stall_in_sql: Option<Arc<dyn Fn() + Send + Sync>>,
    pub(crate) commit_fault: Option<Arc<dyn Fn() -> Option<CommitFault> + Send + Sync>>,
    /// Asked when the connection is reopened to settle an uncertain COMMIT
    /// ([`Db::resolve_uncertain`]): `Some` fails that reopening as given
    /// (a busy database, a refused one).
    pub(super) reconnect_fault: Option<Arc<dyn Fn() -> Option<SqlFailure> + Send + Sync>>,
    /// Called at named points of the import and a save (a child process
    /// aborts there, or a stall holds there), and as `"lease-rows"` inside
    /// every lease-row build on the writer thread (`files::preference_rows`,
    /// whoever calls it and whatever lock the caller holds).
    pub(crate) point: Option<Arc<dyn Fn(&'static str) + Send + Sync>>,
    /// Called at the end of every [`Db::commit`] with the worker
    /// connection's state: `None` when it was closed, else whether it is
    /// back in autocommit and whether every relied-on setting (page size
    /// included) reads back on it.
    pub(crate) after_commit: Option<ConnectionProbe>,
}

/// The writer thread's database: its connection (opened on first use), the
/// process epoch drawn once, and the root-use lease.
pub(super) struct Db {
    root: StateRoot,
    frontend: Frontend,
    root_use: Option<RootUse>,
    conn: Option<Connection>,
    epoch: Option<u64>,
    data_version: Option<i64>,
    #[cfg(test)]
    pub(crate) seams: Seams,
}

impl Db {
    pub(super) fn new(root: StateRoot, frontend: Frontend, root_use: Option<RootUse>) -> Self {
        Self {
            root,
            frontend,
            root_use,
            conn: None,
            epoch: None,
            data_version: None,
            #[cfg(test)]
            seams: Seams::default(),
        }
    }

    /// The process epoch, once drawn.
    pub(super) fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    /// Whether a connection is open.
    pub(super) fn is_open(&self) -> bool {
        self.conn.is_some()
    }

    /// Open (or reopen) the database, set it up, draw this process's epoch
    /// once, bring the legacy JSON source in, and read the durable
    /// document. `clock` is the service's last issued version. Every
    /// mutating transaction on the way (schema, epoch, import and export
    /// ledgers, evidence) runs under the product guard and is admitted
    /// against the WAL bound: a full WAL answers [`WaitCause::Wal`] and the
    /// open is retried after the idle reset.
    pub(super) fn hydrate(&mut self, clock: Option<StoreVersion>) -> Result<Hydrated, SqlFailure> {
        self.ensure_root_use()?;
        let guard = self.guard()?;
        let mut result = (|| {
            if self.conn.is_none() {
                self.conn = Some(open_connection(&self.root)?);
                self.data_version = None;
            }
            #[cfg(test)]
            self.stall_in_sql()?;
            self.init_schema(&guard)?;
            if self.epoch.is_none() {
                self.allocate_epoch(&guard)?;
            }
            let mut notices = Vec::new();
            let evidence = self.reconcile_legacy(&guard, clock, &mut notices)?;
            drop(guard);
            if let Some(evidence) = evidence {
                self.finish_import_evidence(&evidence, &mut notices);
            }
            let durable = self.read_durable()?;
            Ok(Hydrated { durable, notices })
        })();
        self.after_failure(&mut result);
        result
    }

    /// The durable document when another connection committed since this
    /// connection last read (`PRAGMA data_version`), else `None`.
    pub(super) fn refresh(&mut self) -> Result<Option<Durable>, SqlFailure> {
        #[cfg(test)]
        self.stall_in_sql()?;
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        let version: i64 = conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(|error| refused_by(error, "reading data_version"))?;
        if Some(version) == self.data_version {
            return Ok(None);
        }
        let mut result = self.read_durable().map(Some);
        self.after_failure(&mut result);
        result
    }

    /// One save: under the product guard, admit the transaction against the
    /// WAL bound, then in `BEGIN IMMEDIATE` read the head (and the document
    /// when it changed since `known`), let `decide` stage the edit, and
    /// commit it at a version allocated from the stored sequence.
    ///
    /// `begun` is the atomic abandonment boundary: it runs once the
    /// transaction has begun and before anything is written, and answers
    /// whether the service still wants the save. `false` rolls back with
    /// [`Commit::Abandoned`]; `true` commits to the transaction's own
    /// outcome (commit, rollback or uncertain resolution) whatever the
    /// service does next.
    pub(super) fn commit<S, D, B>(
        &mut self,
        request: RequestId,
        clock: Option<StoreVersion>,
        known: Option<&Durable>,
        begun: B,
        decide: D,
    ) -> Result<Commit<S>, SqlFailure>
    where
        S: Staging,
        D: FnOnce(&Durable) -> Result<S, ErrorCode>,
        B: FnOnce() -> bool,
    {
        let epoch = self.epoch.ok_or_else(closed)?;
        self.ensure_root_use()?;
        let guard = self.guard()?;
        let mut result = (|| {
            if self.conn.is_none() {
                // A reconnect within this service keeps its epoch.
                self.conn = Some(open_connection(&self.root)?);
                self.data_version = None;
            }
            let key = SaveKey {
                epoch,
                request,
                clock,
            };
            self.commit_locked(&guard, key, known, begun, decide)
        })();
        drop(guard);
        self.after_failure(&mut result);
        #[cfg(test)]
        if let Some(after) = &self.seams.after_commit {
            after(
                self.conn
                    .as_ref()
                    .map(|conn| (conn.is_autocommit(), verify_settings(conn).is_ok())),
            );
        }
        result
    }

    /// The idle TRUNCATE checkpoint, under the product guard. `Ok` with
    /// SQLite's counts either way; `busy` when a reader kept it from the
    /// reset.
    pub(super) fn truncate(&mut self) -> Result<Truncated, SqlFailure> {
        let guard = self.guard()?;
        self.truncate_locked(&guard)
    }

    /// [`Self::truncate`] for a caller that already holds the guard.
    fn truncate_locked(&mut self, _guard: &files::WriteLock) -> Result<Truncated, SqlFailure> {
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        let (busy, log, checkpointed): (i64, i64, i64) = conn
            .query_row("PRAGMA main.wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|error| match wait_of(&error) {
                Some(cause) => SqlFailure::Wait(cause),
                None => refused_by(error, "checkpointing the WAL"),
            })?;
        Ok(Truncated {
            busy: busy != 0,
            log,
            checkpointed,
            wal_bytes: wal_len(&self.root),
        })
    }

    /// For a maintenance command, which holds the guard and the exclusive
    /// root: make room for its ledger writes. A WAL past the admission bound
    /// gets one idle TRUNCATE; when a reader keeps that from completing, the
    /// command refuses with the cause and the full retry, never growing the
    /// log.
    fn make_wal_room(&mut self, guard: &files::WriteLock, retry: &str) -> Result<(), SqlFailure> {
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        match admit_wal(conn, &self.root) {
            Err(SqlFailure::Wait(WaitCause::Wal)) => {}
            other => return other,
        }
        let reset = self.truncate_locked(guard)?;
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        match admit_wal(conn, &self.root) {
            Err(SqlFailure::Wait(WaitCause::Wal | WaitCause::Busy)) => Err(SqlFailure::refused(
                FailureReason::Io,
                format!(
                    "Preferences refused at {:?}: the SQLite write-ahead log holds {} frames \
                         ({} bytes), too many for another transaction ({WAL_MAX_FRAMES} frames \
                         at most), and another program keeps reading the database, so the log \
                         cannot be reset. Close every program that has the database open, then \
                         {retry}",
                    self.root.database_file().display().to_string(),
                    reset.log,
                    reset.wal_bytes
                ),
            )),
            other => other,
        }
    }

    /// The retry sentence of a maintenance refusal: the same command again.
    fn retry_export(&self) -> String {
        let mut text = String::new();
        for (n, bin) in self.frontend.binaries().iter().enumerate() {
            if n > 0 {
                text.push_str(" or ");
            }
            text.push_str(&format!("'{bin} state preferences export --same-root'"));
        }
        format!("Run {text} again.")
    }

    fn commit_locked<S, D, B>(
        &mut self,
        guard: &files::WriteLock,
        key: SaveKey,
        known: Option<&Durable>,
        begun: B,
        decide: D,
    ) -> Result<Commit<S>, SqlFailure>
    where
        S: Staging,
        D: FnOnce(&Durable) -> Result<S, ErrorCode>,
        B: FnOnce() -> bool,
    {
        let SaveKey {
            epoch,
            request,
            clock,
        } = key;
        #[cfg(test)]
        let fault = self.seams.commit_fault.as_ref().and_then(|fault| fault());
        #[cfg(test)]
        let point = self.seams.point.clone();
        #[cfg(test)]
        self.stall_in_sql()?;
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "starting a save")?;
        // The begun boundary: past this the transaction keeps its own
        // outcome; short of it a service that is shutting down cancels.
        if !begun() {
            drop(tx);
            return Ok(Commit::Abandoned);
        }
        let head = read_head(&tx)?;
        let observed = match known {
            Some(known) if known.version == head => None,
            _ => Some(read_document(&tx, head)?),
        };
        let current = observed.as_ref().or(known).ok_or_else(closed)?;
        let mut staged = match decide(current) {
            Ok(staged) => staged,
            Err(code) => {
                drop(tx);
                return Ok(Commit::Declined {
                    code,
                    detail: None,
                    observed,
                });
            }
        };
        if staged.unchanged() {
            drop(tx);
            return Ok(Commit::Unchanged { staged, observed });
        }
        let sequence = read_sequence(&tx)?;
        let base = [Some(sequence), clock, Some(head)]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(StoreVersion::INITIAL);
        let Some(version) = base.checked_next() else {
            drop(tx);
            return Ok(Commit::Declined {
                code: ErrorCode::Busy {
                    domain: LocalStateDomain::Preferences,
                },
                detail: Some(Arc::from(
                    "the preferences version space is spent; nothing was saved",
                )),
                observed,
            });
        };
        staged.set_version(version);
        let body = staged.bytes();
        let digest = files::digest(body);
        let written = (|| -> rusqlite::Result<()> {
            tx.execute(
                "INSERT INTO preference_document (id, body) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET body = excluded.body",
                params![body],
            )?;
            write_head(&tx, version)?;
            tx.execute(
                "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                params![be(version.get())],
            )?;
            tx.execute(
                "INSERT INTO recent_commit
                     (epoch, request, domain, old_version, new_version, action, digest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    be(epoch),
                    be(request.get()),
                    PREFERENCES_HEAD,
                    be(head.get()),
                    be(version.get()),
                    "preferences-write",
                    digest.as_slice()
                ],
            )?;
            tx.execute(
                "DELETE FROM recent_commit WHERE seq NOT IN
                     (SELECT seq FROM recent_commit ORDER BY seq DESC LIMIT ?1)",
                params![RECENT_COMMITS],
            )?;
            Ok(())
        })();
        if let Err(error) = written {
            drop(tx);
            self.confirm_rolled_back()?;
            return match wait_of(&error) {
                Some(cause) => Err(SqlFailure::Wait(cause)),
                None => Ok(Commit::Declined {
                    code: statement_failure(&error),
                    detail: Some(Arc::from(format!("the save was not written: {error}"))),
                    observed,
                }),
            };
        }
        // A test's hand inside the open write transaction (it may hold it,
        // or die in it).
        #[cfg(test)]
        if let Some(point) = &point {
            point("before-commit");
        }
        #[cfg(test)]
        let committed = match fault {
            None => tx.commit(),
            Some(CommitFault::IoErrAfterCommit) => tx
                .commit()
                .and(Err(sqlite_error(rusqlite::ffi::SQLITE_IOERR_WRITE))),
            Some(other) => {
                // The failure arrives while the worker's transaction is
                // still active, as a COMMIT that errors without ending it
                // leaves it: the handling below has to roll back itself.
                let mut tx = tx;
                tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
                drop(tx);
                Err(sqlite_error(match other {
                    CommitFault::IoErrWithoutCommit => rusqlite::ffi::SQLITE_IOERR_FSYNC,
                    CommitFault::Full => rusqlite::ffi::SQLITE_FULL,
                    CommitFault::ReadOnly => rusqlite::ffi::SQLITE_READONLY,
                    CommitFault::Busy | CommitFault::IoErrAfterCommit => rusqlite::ffi::SQLITE_BUSY,
                }))
            }
        };
        #[cfg(not(test))]
        let committed = tx.commit();
        let durable = Durable {
            version,
            projection: staged.projection(),
            digest,
            len: body.len(),
        };
        // A test's hand right after the COMMIT returned: the save is durable
        // and nothing of it is settled yet (it may die here).
        #[cfg(test)]
        if committed.is_ok() {
            if let Some(point) = &point {
                point("after-commit");
            }
        }
        let mut recovered = false;
        if let Err(error) = committed {
            let code = error.sqlite_error_code();
            match code {
                Some(rusqlite::ErrorCode::SystemIoFailure) => {
                    // Uncertain: the COMMIT may or may not have reached the
                    // disk. Reconnect on this worker and look for this
                    // operation's row; never replay it as a new request.
                    match self.resolve_uncertain(epoch, request, version, &digest) {
                        Ok(true) => {
                            self.point("commit-uncertain-found");
                            recovered = true;
                        }
                        Ok(false) => {
                            self.point("commit-uncertain-lost");
                            return Ok(Commit::Declined {
                                code: ErrorCode::Failed {
                                    domain: LocalStateDomain::Preferences,
                                    reason: FailureReason::Io,
                                },
                                detail: Some(unknown_outcome(
                                    &error,
                                    "it was not retried, and the preferences show what the \
                                     database holds",
                                )),
                                observed,
                            });
                        }
                        Err(failure) => {
                            // The reconnect that would have settled it
                            // failed (plan §4.2): the save stays unresolved.
                            // It is neither retried (a retry decides against
                            // the head its own COMMIT may have made, and
                            // would settle "unchanged" or apply twice) nor
                            // reported as not saved, and a refused reopening
                            // keeps that state in front of its own cause.
                            self.point("commit-uncertain-unresolved");
                            let cause = match &failure {
                                SqlFailure::Wait(cause) => cause.reason(),
                                SqlFailure::Refused { detail, .. } => detail.trim_end_matches('.'),
                            };
                            return Ok(Commit::Declined {
                                code: ErrorCode::Failed {
                                    domain: LocalStateDomain::Preferences,
                                    reason: FailureReason::Io,
                                },
                                detail: Some(unknown_outcome(
                                    &error,
                                    &format!(
                                        "it was not retried, and the database could not be \
                                         reopened to check it ({cause}); the preferences show \
                                         what the database holds once it opens"
                                    ),
                                )),
                                observed,
                            });
                        }
                    }
                }
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                    self.confirm_rolled_back()?;
                    self.point("commit-busy-rolled-back");
                    return Err(SqlFailure::Wait(WaitCause::Busy));
                }
                _ => {
                    self.confirm_rolled_back()?;
                    self.point("commit-refused-rolled-back");
                    let corrupt = matches!(
                        code,
                        Some(
                            rusqlite::ErrorCode::DatabaseCorrupt
                                | rusqlite::ErrorCode::NotADatabase
                        )
                    );
                    if corrupt {
                        return Err(
                            self.recover_refusal(&format!("the database is damaged: {error}"))
                        );
                    }
                    return Ok(Commit::Declined {
                        code: ErrorCode::Failed {
                            domain: LocalStateDomain::Preferences,
                            reason: FailureReason::Io,
                        },
                        detail: Some(Arc::from(format!(
                            "preferences not saved: the database refused the commit ({error})"
                        ))),
                        observed,
                    });
                }
            }
        }
        let frames = self
            .conn
            .as_ref()
            .and_then(|conn| wal_frames(conn, &self.root).ok())
            .unwrap_or(0);
        Ok(Commit::Committed {
            staged,
            durable,
            observed,
            truncate_due: frames >= WAL_TRUNCATE_AT,
            recovered,
        })
    }

    /// After a failed COMMIT that should have rolled back: confirm the
    /// connection is back in autocommit, rolling back explicitly when not.
    /// A connection that cannot get there is closed and the store refused.
    fn confirm_rolled_back(&mut self) -> Result<(), SqlFailure> {
        let Some(conn) = self.conn.as_ref() else {
            return Ok(());
        };
        if !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        if conn.is_autocommit() {
            return Ok(());
        }
        self.conn = None;
        Err(SqlFailure::refused(
            FailureReason::Dirty,
            "a failed save left a transaction open; the connection was closed",
        ))
    }

    /// Resolve an uncertain COMMIT: close this connection, reconnect on the
    /// same worker (same epoch), and look for this operation's row with the
    /// committed digest. `true` when it committed.
    fn resolve_uncertain(
        &mut self,
        epoch: u64,
        request: RequestId,
        version: StoreVersion,
        digest: &ContentDigest,
    ) -> Result<bool, SqlFailure> {
        self.conn = None;
        self.data_version = None;
        #[cfg(test)]
        if let Some(fault) = self
            .seams
            .reconnect_fault
            .as_ref()
            .and_then(|fault| fault())
        {
            return Err(fault);
        }
        let conn = open_connection(&self.root)?;
        let found = conn
            .query_row(
                "SELECT new_version FROM recent_commit
                 WHERE epoch = ?1 AND request = ?2 AND digest = ?3",
                params![be(epoch), be(request.get()), digest.as_slice()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|error| refused_by(error, "resolving an uncertain save"))?;
        self.conn = Some(conn);
        Ok(found.and_then(|raw| decode_u64(&raw)) == Some(version.get()))
    }

    fn after_failure<T>(&mut self, result: &mut Result<T, SqlFailure>) {
        if let Err(SqlFailure::Refused { reason, detail }) = result {
            // A damaged database names the explicit recovery commands.
            if *reason == FailureReason::Checksum && !detail.starts_with("Preferences refused") {
                *detail = Arc::from(recover_text(
                    &self.root.database_file().display().to_string(),
                    detail,
                    self.frontend,
                ));
            }
            // A refused store reopens from scratch on its next read: no
            // descriptor stays open on a database this service refused.
            self.conn = None;
            self.data_version = None;
        }
    }

    fn ensure_root_use(&mut self) -> Result<(), SqlFailure> {
        if self.root_use.is_some() {
            return Ok(());
        }
        match RootUse::shared(&self.root) {
            Ok(Some(lease)) => {
                self.root_use = Some(lease);
                Ok(())
            }
            Ok(None) => Err(SqlFailure::Wait(WaitCause::Maintenance)),
            Err(error) => Err(SqlFailure::refused(
                FailureReason::Io,
                format!(
                    "Preferences refused at {:?}: {error}. The root-use lock could not be taken.",
                    self.root.instance_lock_file().display().to_string()
                ),
            )),
        }
    }

    fn guard(&self) -> Result<files::WriteLock, SqlFailure> {
        match files::try_lock_for_write(&self.root.preferences_file()) {
            Ok(Some(lock)) => Ok(lock),
            Ok(None) => Err(SqlFailure::Wait(WaitCause::Guard)),
            Err(error) => Err(SqlFailure::refused(
                FailureReason::Io,
                format!(
                    "Preferences refused at {:?}: the preferences guard could not be taken: \
                     {error}",
                    files::lock_path(&self.root.preferences_file())
                        .display()
                        .to_string()
                ),
            )),
        }
    }

    /// A test's named point (no effect in product builds).
    #[cfg(test)]
    pub(super) fn point(&self, name: &'static str) {
        if let Some(point) = &self.seams.point {
            point(name);
        }
    }

    #[cfg(not(test))]
    pub(super) fn point(&self, _name: &'static str) {}

    #[cfg(test)]
    fn stall_in_sql(&self) -> Result<(), SqlFailure> {
        let (Some(stall), Some(conn)) = (self.seams.stall_in_sql.clone(), self.conn.as_ref())
        else {
            return Ok(());
        };
        // The stall runs inside a real SQL statement on this connection.
        conn.create_scalar_function(
            "t53_hold",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                stall();
                Ok(1_i64)
            },
        )
        .map_err(|error| refused_by(error, "installing the test stall"))?;
        conn.query_row("SELECT t53_hold()", [], |row| row.get::<_, i64>(0))
            .map_err(|error| refused_by(error, "the test stall"))?;
        Ok(())
    }

    /// Create the schema on an uninitialized database, adopt the schema
    /// another process completed, or refuse a foreign or newer one; in one
    /// `BEGIN IMMEDIATE` under the guard.
    fn init_schema(&mut self, guard: &files::WriteLock) -> Result<(), SqlFailure> {
        let path = self.root.database_file();
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "opening the schema")?;
        let (application, user_version, tables) = (|| -> rusqlite::Result<(i32, i32, i64)> {
            Ok((
                tx.query_row("PRAGMA application_id", [], |row| row.get(0))?,
                tx.query_row("PRAGMA user_version", [], |row| row.get(0))?,
                tx.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })()
        .map_err(|error| refused_by(error, "reading the schema"))?;
        let display = path.display().to_string();
        if application == 0 && user_version == 0 && tables == 0 {
            tx.execute_batch(SCHEMA)
                .and_then(|()| {
                    tx.execute_batch(&format!(
                        "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = \
                         {USER_VERSION};"
                    ))
                })
                .map_err(|error| refused_by(error, "creating the schema"))?;
        } else if application == APPLICATION_ID && user_version > USER_VERSION {
            return Err(SqlFailure::refused(
                FailureReason::Schema,
                format!(
                    "Preferences refused at {display:?}: the database has schema version \
                     {user_version}, newer than this build reads ({USER_VERSION}). Use a \
                     compatible 274bot version or a separate correctly owned root."
                ),
            ));
        } else if application != APPLICATION_ID || user_version != USER_VERSION {
            return Err(SqlFailure::refused(
                FailureReason::Schema,
                format!(
                    "Preferences refused at {display:?}: the file is not a 274bot state \
                     database (application id {application:#x}, schema version \
                     {user_version}). Use a compatible 274bot version or a separate correctly \
                     owned root."
                ),
            ));
        }
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the schema"),
        })?;
        Ok(())
    }

    /// Draw this process's epoch: a checked increment of the stored
    /// counter, never wrapped.
    fn allocate_epoch(&mut self, guard: &files::WriteLock) -> Result<(), SqlFailure> {
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "drawing the process epoch")?;
        let stored: Vec<u8> = tx
            .query_row("SELECT epoch FROM store_meta WHERE id = 1", [], |row| {
                row.get(0)
            })
            .map_err(|error| refused_by(error, "reading the epoch counter"))?;
        let Some(next) = decode_u64(&stored).and_then(|epoch| epoch.checked_add(1)) else {
            return Err(SqlFailure::refused(
                FailureReason::Schema,
                "the process epoch space of this preferences database is spent; nothing is \
                 reused or wrapped",
            ));
        };
        tx.execute(
            "UPDATE store_meta SET epoch = ?1 WHERE id = 1",
            params![be(next)],
        )
        .map_err(|error| refused_by(error, "storing the epoch"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the epoch"),
        })?;
        self.epoch = Some(next);
        Ok(())
    }

    /// Bring the legacy `panel-ui.json` in, under the guard: the first
    /// import, a malformed source's defaults, or the once-only reimport of
    /// a published export that 0.2.5 changed (plan §5.1, §5.2). An export
    /// that was interrupted after its prepared row is resolved first, in
    /// every ledger state, so Ready never precedes its publication or its
    /// refusal. `Some` when a valid import committed and its source still
    /// has to become `.migrated` evidence.
    fn reconcile_legacy(
        &mut self,
        guard: &files::WriteLock,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Option<Evidence>, SqlFailure> {
        let source = self.root.preferences_file();
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        let ledger = read_ledger(conn)?;
        let export = match read_export(conn)? {
            Some(export) if export.state == "prepared" => {
                Some(self.resolve_prepared_export(guard, &source, &export, notices)?)
            }
            other => other,
        };
        match ledger {
            None => self.first_import(guard, &source, clock, notices),
            Some(ledger) => {
                if ledger.evidence == "pending" {
                    return Ok(Some(Evidence {
                        source,
                        digest: ledger.digest,
                    }));
                }
                self.reimport_if_changed(guard, &source, &ledger, export, clock, notices)?;
                Ok(None)
            }
        }
    }

    fn first_import(
        &mut self,
        guard: &files::WriteLock,
        source: &Path,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Option<Evidence>, SqlFailure> {
        let display = source.display().to_string();
        match read_legacy(source) {
            Legacy::Absent => {
                let version = self.commit_ledger(
                    guard,
                    clock,
                    &files::preferences_bytes(&Map::new()).map_err(|_| closed())?,
                    LedgerRow {
                        digest: None,
                        kind: "absent",
                        evidence: "none",
                        path: &display,
                    },
                )?;
                notices.push((
                    Level::Info,
                    audit_line(
                        "preferences-start-empty",
                        StoreVersion::INITIAL,
                        version,
                        &format!("no legacy file at {display}"),
                    ),
                ));
                Ok(None)
            }
            Legacy::Valid {
                bytes,
                canonical,
                members,
            } => {
                let digest = files::digest(&bytes);
                self.point("before-import-commit");
                let version = self.commit_ledger(
                    guard,
                    clock,
                    &canonical,
                    LedgerRow {
                        digest: Some(digest),
                        kind: "imported",
                        evidence: "pending",
                        path: &display,
                    },
                )?;
                self.point("after-import-commit");
                notices.push((
                    Level::Info,
                    audit_line(
                        "preferences-import",
                        StoreVersion::INITIAL,
                        version,
                        &format!(
                            "imported {members} member(s) from {display} sha256={}",
                            hex(&digest)
                        ),
                    ),
                ));
                Ok(Some(Evidence {
                    source: source.to_path_buf(),
                    digest: Some(digest),
                }))
            }
            Legacy::Malformed(bytes) => {
                let digest = files::digest(&bytes);
                let copy = self.quarantine_source(source, &bytes)?;
                let version = self.commit_ledger(
                    guard,
                    clock,
                    &files::preferences_bytes(&Map::new()).map_err(|_| closed())?,
                    LedgerRow {
                        digest: Some(digest),
                        kind: "quarantined",
                        evidence: "completed",
                        path: &copy.display().to_string(),
                    },
                )?;
                notices.push((
                    Level::Warn,
                    format!(
                        "local state: preferences file {display} was not valid preferences \
                         JSON; its bytes are kept in {} and default preferences were started",
                        copy.display()
                    ),
                ));
                notices.push((
                    Level::Info,
                    audit_line(
                        "preferences-defaults-after-quarantine",
                        StoreVersion::INITIAL,
                        version,
                        &format!("source {display} kept in {}", copy.display()),
                    ),
                ));
                Ok(None)
            }
            Legacy::Refused(failure) => {
                Err(self.source_failure(&display, failure, SourceUse::FirstImport))
            }
        }
    }

    /// The verified private copy of the bytes of a malformed source, with
    /// its directory synced (plan §5.1, Q3). A copy that cannot be made or
    /// reach the disk refuses: the evidence is never discarded.
    fn quarantine_source(&self, source: &Path, bytes: &[u8]) -> Result<PathBuf, SqlFailure> {
        let display = source.display().to_string();
        let copy = files::quarantine_copy(source, bytes, "corrupt").map_err(|error| {
            SqlFailure::refused(
                FailureReason::Io,
                format!(
                    "Preferences refused at {display:?}: the file is not valid preferences JSON \
                     and its copy could not be kept ({error}). No source ledger or defaults \
                     were saved. Free space or fix the directory's permissions, then retry."
                ),
            )
        })?;
        self.point("after-quarantine-copy");
        if let Some(dir) = source.parent() {
            files::sync_directory(dir).map_err(|error| {
                SqlFailure::refused(
                    FailureReason::Io,
                    format!(
                        "Preferences refused at {display:?}: the copy {} of the malformed file \
                         did not reach the disk ({error}). No defaults were saved. Retry.",
                        copy.display()
                    ),
                )
            })?;
        }
        Ok(copy)
    }

    /// The cause-specific refusal of a source that could not be read (plan
    /// §5.1): the exact path, the cause and the remedy, for the first
    /// import, an authorized reimport and an explicit recovery alike.
    pub(super) fn source_failure(
        &self,
        display: &str,
        failure: SourceFailure,
        usage: SourceUse,
    ) -> SqlFailure {
        let outcome = match usage {
            SourceUse::FirstImport => "No source ledger or defaults were saved.",
            SourceUse::Reimport => {
                "The same-root export stays authorized; nothing was imported or saved."
            }
            SourceUse::Recovery => "Nothing was changed.",
        };
        match failure {
            SourceFailure::Unrepresentable(cause) => match usage {
                SourceUse::FirstImport | SourceUse::Reimport => {
                    self.recover_refusal_at(display, &cause)
                }
                SourceUse::Recovery => {
                    let mut text = format!(
                        "Preferences refused at {display:?}: {cause}. {outcome} Name a smaller \
                         private JSON object with --source, or recover with --defaults."
                    );
                    for bin in self.frontend.binaries() {
                        text.push_str(&format!(
                            " Run '{bin} state preferences recover --source \
                             \"<private-valid-json>\"' or '{bin} state preferences recover \
                             --defaults'."
                        ));
                    }
                    SqlFailure::refused(FailureReason::Schema, text)
                }
            },
            SourceFailure::Ambiguous(cause) => {
                let remedy = match usage {
                    SourceUse::FirstImport => "Edit the file as said, or remove it to start empty.",
                    SourceUse::Reimport => {
                        "Edit the file as said, or remove it to keep the saved preferences."
                    }
                    SourceUse::Recovery => {
                        "Edit the file as said, or name another private JSON object with \
                         --source."
                    }
                };
                SqlFailure::refused(
                    FailureReason::Schema,
                    format!(
                        "Preferences refused at {display:?}: {cause}. The file was not changed. \
                         {outcome} {remedy} Retry after fixing this cause."
                    ),
                )
            }
            SourceFailure::Unsafe(cause) => SqlFailure::refused(
                FailureReason::Io,
                format!(
                    "Preferences refused at {display:?}: {cause}. {outcome} Retry after fixing \
                     this cause."
                ),
            ),
            SourceFailure::Unreadable(cause) => {
                let remedy = match usage {
                    SourceUse::FirstImport => {
                        "Restore readable private preferences JSON (a regular file, mode 600) \
                         at that path, or remove it to start empty."
                    }
                    SourceUse::Reimport => {
                        "Restore readable private preferences JSON (a regular file, mode 600) \
                         at that path, or remove it to keep the saved preferences."
                    }
                    SourceUse::Recovery => {
                        "Name a readable private JSON object (a regular file, mode 600) with \
                         --source."
                    }
                };
                SqlFailure::refused(
                    FailureReason::Io,
                    format!(
                        "Preferences refused at {display:?}: {cause}. {outcome} {remedy} Retry \
                         after fixing this cause."
                    ),
                )
            }
        }
    }

    /// The refusal of a damaged database or an over-limit source: stop
    /// every process, then the explicit recovery commands.
    fn recover_refusal(&self, cause: &str) -> SqlFailure {
        self.recover_refusal_at(&self.root.database_file().display().to_string(), cause)
    }

    fn recover_refusal_at(&self, display: &str, cause: &str) -> SqlFailure {
        SqlFailure::refused(
            FailureReason::Schema,
            recover_text(display, cause, self.frontend),
        )
    }

    /// A changed legacy file that no published export authorized (plan
    /// §5.2): another 274bot version or an outside edit wrote it. The saved
    /// preferences and the file stay as they are.
    fn external_edit_refusal(&self, display: &str) -> SqlFailure {
        let mut forms = String::new();
        for (n, bin) in self.frontend.binaries().iter().enumerate() {
            if n > 0 {
                forms.push_str(" or ");
            }
            forms.push_str(&format!(
                "'{bin} state preferences recover --source \"{display}\"'"
            ));
        }
        SqlFailure::refused(
            FailureReason::Dirty,
            format!(
                "Preferences refused at {display:?}: the file was changed after preferences \
                 moved to SQLite, and no same-root export authorized it (another 274bot version \
                 or an outside edit wrote it; mixed versions are not supported). The saved \
                 preferences and the file are untouched. Stop every other 274bot version. Then \
                 move that file away to keep the saved preferences, or take the file's content \
                 instead with {forms} (it must be a private, valid JSON object; recovery \
                 preserves the old files first). Retry after that."
            ),
        )
    }

    /// What an authorized (published) same-root export did while this
    /// version was stopped, and what an unauthorized file did (plan §5.1,
    /// §5.2). A changed valid export is imported once as the latest intent;
    /// unchanged export bytes are ignored; a malformed export follows the
    /// Q3 quarantine; an unreadable one refuses with its cause and leaves
    /// the authorization unconsumed. A changed file with no published export
    /// refuses as a mixed-version or outside edit. `export` is already
    /// resolved: the caller completed or refused an interrupted export.
    fn reimport_if_changed(
        &mut self,
        guard: &files::WriteLock,
        source: &Path,
        ledger: &Ledger,
        export: Option<ExportHead>,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<(), SqlFailure> {
        let display = source.display().to_string();
        let authorized = export.filter(|export| export.state == "published");
        match read_legacy(source) {
            Legacy::Absent => Ok(()),
            Legacy::Refused(failure) => match authorized {
                Some(_) => Err(self.source_failure(&display, failure, SourceUse::Reimport)),
                None => {
                    notices.push((
                        Level::Warn,
                        format!(
                            "local state: {display} cannot be read; the saved SQLite \
                             preferences stay in use and that file is not imported"
                        ),
                    ));
                    Ok(())
                }
            },
            Legacy::Valid {
                bytes,
                canonical,
                members,
            } => {
                let digest = files::digest(&bytes);
                match authorized {
                    Some(export) if digest == export.digest => Ok(()),
                    Some(export) => {
                        let previous = self.head()?;
                        let version = self.commit_reimport(
                            guard,
                            clock,
                            &canonical,
                            export.export_id,
                            LedgerRow {
                                digest: Some(digest),
                                kind: "reimported",
                                evidence: "completed",
                                path: &display,
                            },
                        )?;
                        notices.push((
                            Level::Info,
                            audit_line(
                                "preferences-reimport",
                                previous,
                                version,
                                &format!(
                                    "the exported file {display} was changed by another \
                                     version; imported {members} member(s) once as the latest \
                                     preferences sha256={}",
                                    hex(&digest)
                                ),
                            ),
                        ));
                        Ok(())
                    }
                    None if Some(digest) == ledger.digest => Ok(()),
                    None => Err(self.external_edit_refusal(&display)),
                }
            }
            Legacy::Malformed(bytes) => {
                let digest = files::digest(&bytes);
                match authorized {
                    Some(export) => {
                        let copy = self.quarantine_source(source, &bytes)?;
                        let previous = self.head()?;
                        let version = self.commit_reimport(
                            guard,
                            clock,
                            &files::preferences_bytes(&Map::new()).map_err(|_| closed())?,
                            export.export_id,
                            LedgerRow {
                                digest: Some(digest),
                                kind: "quarantined",
                                evidence: "completed",
                                path: &copy.display().to_string(),
                            },
                        )?;
                        notices.push((
                            Level::Warn,
                            format!(
                                "local state: the exported preferences file {display} was \
                                 changed into content that is not valid preferences JSON; its \
                                 bytes are kept in {} and default preferences were started",
                                copy.display()
                            ),
                        ));
                        notices.push((
                            Level::Info,
                            audit_line(
                                "preferences-defaults-after-quarantine",
                                previous,
                                version,
                                &format!("exported file {display} kept in {}", copy.display()),
                            ),
                        ));
                        Ok(())
                    }
                    None if Some(digest) == ledger.digest => Ok(()),
                    None => Err(self.external_edit_refusal(&display)),
                }
            }
        }
    }

    /// Complete an export that was interrupted after its prepared row (plan
    /// §5.2), by export ID and verified digest, under the product guard and
    /// the shared root-use lease (which no maintenance command can hold at
    /// once). The file at the name already holds the prepared bytes: sync
    /// the directory and record it published. Else the staged file, when
    /// its digest is the recorded one, replaces the name (a copy of an
    /// unrelated file is kept first), is synced and recorded published.
    /// Anything else refuses with the full retry command, and the old
    /// legacy file is never taken for a 0.2.5 edit.
    fn resolve_prepared_export(
        &mut self,
        guard: &files::WriteLock,
        target: &Path,
        export: &ExportHead,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<ExportHead, SqlFailure> {
        let display = target.display().to_string();
        let staged = staged_export_name(target, export.export_id);
        let id = export.export_id;
        let refuse = |cause: String| {
            SqlFailure::refused(
                FailureReason::Dirty,
                format!(
                    "Preferences refused at {display:?}: {cause}. Stop all 274bot processes for \
                     this root. {} Nothing was imported or replaced.",
                    self.retry_export()
                ),
            )
        };
        let existing = match fs::read(target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(refuse(format!(
                    "same-root export {id} was interrupted and the file at that name cannot be \
                     read ({error})"
                )))
            }
        };
        let published = existing
            .as_deref()
            .is_some_and(|bytes| files::digest(bytes) == export.digest);
        if !published {
            let staged_bytes = fs::read(&staged).map_err(|error| {
                refuse(format!(
                    "same-root export {id} was interrupted and its staged file {} cannot be \
                     read ({error})",
                    staged.display()
                ))
            })?;
            if files::digest(&staged_bytes) != export.digest {
                return Err(refuse(format!(
                    "same-root export {id} was interrupted and its staged file {} does not \
                     match the recorded sha256={}",
                    staged.display(),
                    hex(&export.digest)
                )));
            }
            if let Some(existing) = &existing {
                files::quarantine_copy(target, existing, "before-export").map_err(|error| {
                    refuse(format!(
                        "same-root export {id} was interrupted and a copy of the file at that \
                         name could not be kept ({error})"
                    ))
                })?;
            }
            fs::rename(&staged, target).map_err(|error| {
                refuse(format!(
                    "same-root export {id} was interrupted and could not be published ({error})"
                ))
            })?;
        }
        if let Some(dir) = target.parent() {
            files::sync_directory(dir).map_err(|error| {
                refuse(format!(
                    "same-root export {id} was published but its directory could not be synced \
                     ({error})"
                ))
            })?;
        }
        let completed = self.mark_export(guard, export, "published")?;
        let version = self.head()?;
        notices.push((
            Level::Info,
            audit_line(
                "preferences-export",
                version,
                version,
                &format!(
                    "completed the interrupted export {id} to {display} sha256={}",
                    hex(&export.digest)
                ),
            ),
        ));
        Ok(completed)
    }

    /// Record `state` for `export`, by its ID and digest only, in an
    /// admitted transaction.
    fn mark_export(
        &mut self,
        guard: &files::WriteLock,
        export: &ExportHead,
        state: &str,
    ) -> Result<ExportHead, SqlFailure> {
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "recording the export")?;
        let changed = tx
            .execute(
                "UPDATE export_head SET state = ?1
                 WHERE id = 1 AND export_id = ?2 AND digest = ?3",
                params![state, be(export.export_id), export.digest.as_slice()],
            )
            .map_err(|error| refused_by(error, "recording the export"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the export record"),
        })?;
        if changed != 1 {
            return Err(SqlFailure::refused(
                FailureReason::Dirty,
                format!(
                    "Preferences refused at {:?}: the export ledger no longer names export {}. \
                     Stop all 274bot processes for this root. {}",
                    self.root.preferences_file().display().to_string(),
                    export.export_id,
                    self.retry_export()
                ),
            ));
        }
        Ok(ExportHead {
            export_id: export.export_id,
            digest: export.digest,
            state: state.to_string(),
        })
    }

    fn head(&self) -> Result<StoreVersion, SqlFailure> {
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        read_head(conn)
    }

    /// One ledger transaction: the document, a new version, the head and
    /// the legacy-source row together.
    fn commit_ledger(
        &mut self,
        guard: &files::WriteLock,
        clock: Option<StoreVersion>,
        body: &[u8],
        row: LedgerRow<'_>,
    ) -> Result<StoreVersion, SqlFailure> {
        let LedgerRow {
            digest,
            kind,
            evidence,
            path,
        } = row;
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "starting the import")?;
        if read_ledger(&tx)?.is_some() {
            // Another process imported first: adopt its result.
            return read_head(&tx);
        }
        let version = next_version(&tx, clock)?;
        let path = bounded_path(path);
        (|| -> rusqlite::Result<()> {
            tx.execute(
                "INSERT INTO preference_document (id, body) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET body = excluded.body",
                params![body],
            )?;
            write_head(&tx, version)?;
            tx.execute(
                "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                params![be(version.get())],
            )?;
            tx.execute(
                "INSERT INTO legacy_source (id, digest, kind, evidence, path)
                 VALUES (1, ?1, ?2, ?3, ?4)",
                params![digest.as_ref().map(|d| d.as_slice()), kind, evidence, path],
            )?;
            Ok(())
        })()
        .map_err(|error| refused_by(error, "writing the import"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the import"),
        })?;
        Ok(version)
    }

    /// Consume the published export `export_id`: its changed file replaces
    /// the document at a new version (or the Q3 defaults replace it, for a
    /// malformed one) with the new source digest, kind and path, in the one
    /// transaction.
    fn commit_reimport(
        &mut self,
        guard: &files::WriteLock,
        clock: Option<StoreVersion>,
        body: &[u8],
        export_id: u64,
        row: LedgerRow<'_>,
    ) -> Result<StoreVersion, SqlFailure> {
        let LedgerRow {
            digest, kind, path, ..
        } = row;
        let digest = digest.ok_or_else(closed)?;
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "starting the reimport")?;
        let export = read_export(&tx)?;
        if !export
            .is_some_and(|export| export.state == "published" && export.export_id == export_id)
        {
            return read_head(&tx);
        }
        let version = next_version(&tx, clock)?;
        let path = bounded_path(path);
        (|| -> rusqlite::Result<()> {
            tx.execute(
                "UPDATE preference_document SET body = ?1 WHERE id = 1",
                params![body],
            )?;
            write_head(&tx, version)?;
            tx.execute(
                "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                params![be(version.get())],
            )?;
            tx.execute("UPDATE export_head SET state = 'consumed' WHERE id = 1", [])?;
            tx.execute(
                "UPDATE legacy_source SET digest = ?1, kind = ?2, evidence = 'completed', \
                 path = ?3 WHERE id = 1",
                params![digest.as_slice(), kind, path],
            )?;
            Ok(())
        })()
        .map_err(|error| refused_by(error, "writing the reimport"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the reimport"),
        })?;
        Ok(version)
    }

    /// After a valid import committed: keep the exact source as private
    /// `.migrated` evidence (only while it still holds the imported bytes)
    /// and sync its directory. A failure here is a separate warning, never
    /// a rollback of the import; the next open retries it. Recording the
    /// evidence is a mutating transaction: it takes the guard again and is
    /// admitted against the WAL bound, and when it cannot run now the
    /// ledger stays `pending` for the next start.
    fn finish_import_evidence(&mut self, evidence: &Evidence, notices: &mut Vec<(Level, String)>) {
        let display = evidence.source.display().to_string();
        self.point("before-evidence");
        let moved = (|| -> io::Result<Option<PathBuf>> {
            let bytes = match fs::read(&evidence.source) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            if Some(files::digest(&bytes)) != evidence.digest {
                return Ok(None);
            }
            let target = migrated_name(&evidence.source)?;
            fs::rename(&evidence.source, &target)?;
            if let Some(dir) = evidence.source.parent() {
                files::sync_directory(dir)?;
            }
            Ok(Some(target))
        })();
        match moved {
            Ok(target) => {
                if let Err(failure) = self.mark_evidence_complete() {
                    let why = match failure {
                        SqlFailure::Wait(cause) => cause.text().to_string(),
                        SqlFailure::Refused { detail, .. } => detail.to_string(),
                    };
                    notices.push((
                        Level::Warn,
                        format!(
                            "local state: the imported preferences evidence could not be \
                             recorded now ({why}); the next start records it"
                        ),
                    ));
                }
                if let Some(target) = target {
                    notices.push((
                        Level::Info,
                        format!(
                            "local state: imported preferences source {display} is kept as \
                             {}",
                            target.display()
                        ),
                    ));
                }
            }
            Err(error) => notices.push((
                Level::Warn,
                format!(
                    "local state: preferences were imported into SQLite, but {display} could \
                     not be kept as .migrated evidence ({error}); the import stands and the \
                     next start tries again"
                ),
            )),
        }
    }

    /// Mark the import evidence completed, under the guard and WAL
    /// admission.
    fn mark_evidence_complete(&mut self) -> Result<(), SqlFailure> {
        let guard = self.guard()?;
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, &guard, "recording the import evidence")?;
        tx.execute(
            "UPDATE legacy_source SET evidence = 'completed' WHERE id = 1",
            [],
        )
        .map_err(|error| refused_by(error, "recording the import evidence"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the import evidence"),
        })
    }

    fn read_durable(&mut self) -> Result<Durable, SqlFailure> {
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| match wait_of(&error) {
                Some(cause) => SqlFailure::Wait(cause),
                None => refused_by(error, "reading preferences"),
            })?;
        let head = read_head(&tx)?;
        let durable = read_document(&tx, head)?;
        let version: i64 = tx
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(|error| refused_by(error, "reading data_version"))?;
        tx.commit()
            .map_err(|error| refused_by(error, "ending the read"))?;
        self.data_version = Some(version);
        Ok(durable)
    }

    /// Open an existing database for the maintenance export (the caller
    /// holds the root exclusive and the guard): set it up, make room in the
    /// WAL for its ledger writes, and check its schema. `Ok(false)` when
    /// there is no database at the root.
    pub(super) fn open_for_maintenance(
        &mut self,
        guard: &files::WriteLock,
    ) -> Result<bool, SqlFailure> {
        if fs::symlink_metadata(self.root.database_file()).is_err() {
            return Ok(false);
        }
        self.conn = Some(open_connection(&self.root)?);
        let retry = self.retry_export();
        let mut result = self
            .make_wal_room(guard, &retry)
            .and_then(|()| self.init_schema(guard));
        self.after_failure(&mut result);
        result.map(|()| true)
    }

    /// Same-root export (plan §5.2): stage the latest durable document as
    /// private JSON beside `panel-ui.json`, record it prepared, keep a copy
    /// of an unrelated file at the name, publish, then record it published.
    /// An interruption anywhere after the prepared row is resolved by the
    /// next start (or this command again) from the export ID and digest.
    /// What was exported.
    pub(super) fn export_same_root(
        &mut self,
        guard: &files::WriteLock,
    ) -> Result<Exported, SqlFailure> {
        let durable = self.read_durable()?;
        let target = self.root.preferences_file();
        let display = target.display().to_string();
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        let previous = read_export(conn)?;
        let export_id = previous.as_ref().map_or(1, |export| export.export_id + 1);
        let body = files::preferences_bytes(&durable.projection.document)
            .map_err(|error| SqlFailure::refused(FailureReason::Schema, error.to_string()))?;
        let digest = files::digest(&body);
        let staged = staged_export_name(&target, export_id);
        let io_failed = |what: &str, error: io::Error| {
            SqlFailure::refused(
                FailureReason::Io,
                format!("Export to {display:?} failed: {what}: {error}. Nothing was replaced."),
            )
        };
        let _ = fs::remove_file(&staged);
        vault::create_private_file(&staged, &body)
            .map_err(|error| io_failed("staging the JSON", error))?;
        fs::File::open(&staged)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_failed("syncing the staged JSON", error))?;
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, guard, "recording the prepared export")?;
        tx.execute(
            "INSERT INTO export_head (id, export_id, version, digest, state)
             VALUES (1, ?1, ?2, ?3, 'prepared')
             ON CONFLICT (id) DO UPDATE SET export_id = excluded.export_id,
                 version = excluded.version, digest = excluded.digest, state = 'prepared'",
            params![be(export_id), be(durable.version.get()), digest.as_slice()],
        )
        .map_err(|error| refused_by(error, "recording the prepared export"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the prepared export"),
        })?;
        self.point("export-before-publish");
        let mut kept = None;
        match fs::read(&target) {
            Ok(existing) if files::digest(&existing) != digest => {
                kept = Some(
                    files::quarantine_copy(&target, &existing, "before-export")
                        .map_err(|error| io_failed("keeping a copy of the existing file", error))?,
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_failed("reading the existing file", error)),
        }
        fs::rename(&staged, &target).map_err(|error| io_failed("publishing the JSON", error))?;
        self.point("export-after-rename");
        if let Some(dir) = target.parent() {
            files::sync_directory(dir)
                .map_err(|error| io_failed("syncing the directory", error))?;
        }
        self.point("export-before-marker");
        self.mark_export(
            guard,
            &ExportHead {
                export_id,
                digest,
                state: "prepared".to_string(),
            },
            "published",
        )?;
        Ok(Exported {
            export_id,
            version: durable.version,
            digest: hex(&digest),
            path: target,
            kept,
        })
    }

    /// Start a new preference history (plan §5.3): the caller already
    /// preserved the old database set and removed it. Create a fresh
    /// database, and commit `body` as its first version (one, from a
    /// sequence and an epoch counter of zero) with the recovery ledger.
    pub(super) fn start_new_history(
        &mut self,
        guard: &files::WriteLock,
        body: &[u8],
        digest: Option<ContentDigest>,
        kind: &str,
        evidence_path: &str,
    ) -> Result<StoreVersion, SqlFailure> {
        self.conn = Some(open_connection(&self.root)?);
        let mut result = self.init_schema(guard).and_then(|()| {
            self.commit_ledger(
                guard,
                None,
                body,
                LedgerRow {
                    digest,
                    kind,
                    evidence: "completed",
                    path: evidence_path,
                },
            )
        });
        self.after_failure(&mut result);
        result
    }

    /// Close the connection (no close-time checkpoint).
    pub(super) fn close(&mut self) {
        self.conn = None;
        self.data_version = None;
    }
}

/// The evidence work left after an import commit.
struct Evidence {
    source: PathBuf,
    digest: Option<ContentDigest>,
}

/// What one same-root export wrote.
pub(super) struct Exported {
    pub(super) export_id: u64,
    pub(super) version: StoreVersion,
    pub(super) digest: String,
    pub(super) path: PathBuf,
    pub(super) kept: Option<PathBuf>,
}

/// One preference save's staged result, as [`Db::commit`] writes it.
pub(super) trait Staging {
    /// The edit changes nothing durable.
    fn unchanged(&self) -> bool;
    /// The canonical document bytes to store.
    fn bytes(&self) -> &[u8];
    /// The worker's projection of the new document.
    fn projection(&self) -> files::PreferenceProjection;
    /// Record the allocated version.
    fn set_version(&mut self, version: StoreVersion);
}

struct Ledger {
    digest: Option<ContentDigest>,
    evidence: String,
}

struct ExportHead {
    export_id: u64,
    digest: ContentDigest,
    state: String,
}

/// What the legacy JSON source holds.
pub(super) enum Legacy {
    Absent,
    /// A private JSON object every member of which is representable:
    /// `bytes` are the file's bytes, `canonical` the stored form (the
    /// pretty serialization this product always wrote), already checked
    /// against the document limit.
    Valid {
        bytes: Vec<u8>,
        canonical: Vec<u8>,
        members: usize,
    },
    Malformed(Vec<u8>),
    Refused(SourceFailure),
}

/// Why a legacy source cannot be read (never treated as absent).
pub(super) enum SourceFailure {
    /// Over the size bound, or a member that cannot be held as a row.
    Unrepresentable(String),
    /// Unsafe owner or mode, with its remedy.
    Unsafe(String),
    /// A field named twice in one object: which value was meant is
    /// unknown, so the file is refused whole, and its cause carries the
    /// field, the object and the edit that removes the duplicate.
    Ambiguous(String),
    /// Any other read or metadata failure (I/O, wrong type).
    Unreadable(String),
}

/// Read a legacy preferences JSON file privately (owner, mode, type and
/// size checked on the open file), bounded, and classify it (plan §5.1):
/// only a confirmed `NotFound` is absent.
pub(super) fn read_legacy(path: &Path) -> Legacy {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() > DOCUMENT_MAX => {
            return Legacy::Refused(SourceFailure::Unrepresentable(format!(
                "the file holds {} bytes, more than preferences may hold ({DOCUMENT_MAX})",
                meta.len()
            )));
        }
        Ok(meta) if !meta.is_file() => {
            return Legacy::Refused(SourceFailure::Unreadable("it is not a regular file".into()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Legacy::Absent,
        Err(error) => return Legacy::Refused(SourceFailure::Unreadable(error.to_string())),
    }
    let text = match vault::read_private_file(path, DOCUMENT_MAX) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Legacy::Absent,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return malformed(path);
        }
        Err(error) => return Legacy::Refused(classify_io(&error)),
    };
    let document = match files::object_without_duplicate_fields(&text) {
        Ok(document) => document,
        Err(error @ files::ReadError::DuplicateField { .. }) => {
            return Legacy::Refused(SourceFailure::Ambiguous(error.to_string()));
        }
        Err(_) => return malformed(path),
    };
    let (_, unrepresentable) = files::preference_rows(&document);
    if unrepresentable > 0 {
        return Legacy::Refused(SourceFailure::Unrepresentable(format!(
            "{unrepresentable} member(s) cannot be held as preference rows (a name outside the \
             row alphabet, or a nested value longer than one row)"
        )));
    }
    let canonical = match files::preferences_bytes(&document) {
        Ok(canonical) if canonical.len() as u64 <= DOCUMENT_MAX => canonical,
        Ok(canonical) => {
            return Legacy::Refused(SourceFailure::Unrepresentable(format!(
                "the file holds {} bytes, but this build stores its {} member(s) as {} bytes, \
                 more than preferences may hold ({DOCUMENT_MAX})",
                text.len(),
                document.len(),
                canonical.len()
            )));
        }
        Err(error) => {
            return Legacy::Refused(SourceFailure::Unrepresentable(format!(
                "the file cannot be stored as preferences ({error})"
            )));
        }
    };
    Legacy::Valid {
        members: document.len(),
        canonical,
        bytes: text.into_bytes(),
    }
}

/// The SHA-256 of the whole file at `path`, whatever its size or content,
/// when it is a readable regular file; else `None`. A recovery records it,
/// so the file it leaves in place is not taken for a later outside edit.
pub(super) fn legacy_file_digest(path: &Path) -> Option<ContentDigest> {
    use sha2::{Digest as _, Sha256};
    if !fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).ok()?;
    Some(hasher.finalize().into())
}

/// The exact bytes of a malformed source (not UTF-8, not JSON, or not an
/// object), read again within the same checks.
fn malformed(path: &Path) -> Legacy {
    match files::corrupt_preferences(path) {
        Ok(Some(bytes)) => Legacy::Malformed(bytes),
        Ok(None) => Legacy::Refused(SourceFailure::Unreadable(
            "the file changed while it was read".into(),
        )),
        Err(error) => Legacy::Refused(classify_io(&error)),
    }
}

fn classify_io(error: &io::Error) -> SourceFailure {
    let text = error.to_string();
    if error.kind() == io::ErrorKind::PermissionDenied {
        if text.contains("larger than") {
            return SourceFailure::Unrepresentable(text);
        }
        if text.contains("owned by") || text.contains("writable by") || text.contains("chmod") {
            return SourceFailure::Unsafe(text);
        }
    }
    SourceFailure::Unreadable(text)
}

/// `<file>.migrated`, or `<file>.migrated-<n>` while that name is taken.
fn migrated_name(source: &Path) -> io::Result<PathBuf> {
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?
        .to_string_lossy()
        .into_owned();
    for attempt in 0..100u32 {
        let candidate = source.with_file_name(match attempt {
            0 => format!("{name}.migrated"),
            n => format!("{name}.migrated-{n}"),
        });
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{name}.migrated: no free name"),
    ))
}

/// Open the database at `root` without creating it through SQLite: the
/// private directory and an empty private file are made first, then every
/// file of the set is checked by path metadata only.
pub(super) fn open_connection(root: &StateRoot) -> Result<Connection, SqlFailure> {
    let dir = root.state_dir();
    let path = root.database_file();
    let display = path.display().to_string();
    let io_refusal = |what: &Path, error: io::Error| {
        SqlFailure::refused(
            FailureReason::Io,
            format!(
                "Preferences refused at {:?}: {error}",
                what.display().to_string()
            ),
        )
    };
    vault::create_private_dir(&dir).map_err(|error| io_refusal(&dir, error))?;
    check_private(&dir, vault::PrivateKind::Dir)?;
    match vault::create_private_file(&path, b"") {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io_refusal(&path, error)),
    }
    for file in database_set(root) {
        check_private(&file, vault::PrivateKind::File)?;
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|error| {
        SqlFailure::refused(
            FailureReason::Io,
            format!("Preferences refused at {display:?}: the database did not open: {error}"),
        )
    })?;
    configure(&conn, &display)?;
    Ok(conn)
}

/// The database, `-wal` and `-shm` paths.
pub(super) fn database_set(root: &StateRoot) -> [PathBuf; 3] {
    let path = root.database_file();
    let mut wal = path.clone().into_os_string();
    wal.push("-wal");
    let mut shm = path.clone().into_os_string();
    shm.push("-shm");
    [path, PathBuf::from(wal), PathBuf::from(shm)]
}

fn wal_len(root: &StateRoot) -> u64 {
    let [_, wal, _] = database_set(root);
    fs::symlink_metadata(wal).map_or(0, |meta| meta.len())
}

/// The WAL's frame count since its last reset (`F`), from a NOOP checkpoint
/// outside any transaction, under the guard the caller holds. An absent or
/// zero-length WAL of a verified WAL-mode connection is zero; an unknown
/// count is never taken for an empty log.
fn wal_frames(conn: &Connection, root: &StateRoot) -> Result<u64, SqlFailure> {
    let (busy, log): (i64, i64) = conn
        .query_row("PRAGMA main.wal_checkpoint(NOOP)", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "counting WAL frames"),
        })?;
    if busy != 0 {
        return Err(SqlFailure::Wait(WaitCause::Busy));
    }
    match u64::try_from(log) {
        Ok(frames) => Ok(frames),
        Err(_) if wal_len(root) == 0 => Ok(0),
        Err(_) => Err(SqlFailure::refused(
            FailureReason::Io,
            format!("the WAL frame count is unknown ({log}); the database is not in WAL mode"),
        )),
    }
}

/// Refuse a transaction whose reservation would pass the WAL's frame or
/// byte maximum: `F + P <= W`, and the file (or its frames' own length,
/// whichever is larger) plus `P` frames within the byte maximum.
fn admit_wal(conn: &Connection, root: &StateRoot) -> Result<(), SqlFailure> {
    let frames = wal_frames(conn, root)?;
    let bytes = wal_len(root);
    if frames + MAX_PAGES > WAL_MAX_FRAMES
        || bytes.max(32 + frames * WAL_FRAME_BYTES) + MAX_PAGES * WAL_FRAME_BYTES > WAL_MAX_BYTES
    {
        return Err(SqlFailure::Wait(WaitCause::Wal));
    }
    Ok(())
}

/// Begin a mutating transaction (plan §4.4): the product guard is proven by
/// the borrowed lock, the WAL reservation is admitted, and only then does
/// `BEGIN IMMEDIATE` run. Every SQL write of this module starts here, so no
/// schema, epoch, ledger, evidence or export write bypasses the bound.
fn begin_write<'c>(
    conn: &'c mut Connection,
    root: &StateRoot,
    _guard: &files::WriteLock,
    what: &str,
) -> Result<rusqlite::Transaction<'c>, SqlFailure> {
    admit_wal(conn, root)?;
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, what),
        })
}

/// The staged file of export `export_id`, beside the legacy file.
fn staged_export_name(target: &Path, export_id: u64) -> PathBuf {
    target.with_file_name(format!("panel-ui.json.export-{export_id}"))
}

/// What one save carries into its transaction: the process epoch, the
/// request's process-local ID and the service's clock.
#[derive(Clone, Copy)]
struct SaveKey {
    epoch: u64,
    request: RequestId,
    clock: Option<StoreVersion>,
}

/// What the legacy-source ledger row records for one action.
struct LedgerRow<'a> {
    digest: Option<ContentDigest>,
    kind: &'a str,
    evidence: &'a str,
    path: &'a str,
}

/// Which read of a source a refusal speaks for: the first import, an
/// authorized downgrade reimport or an explicit recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceUse {
    FirstImport,
    Reimport,
    Recovery,
}

/// Path-metadata owner, mode and type checks of one file of the set (never
/// an open beside SQLite's own descriptors). A missing file passes.
pub(super) fn check_private(path: &Path, kind: vault::PrivateKind) -> Result<(), SqlFailure> {
    match vault::check_private_path(path, kind) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SqlFailure::refused(
            FailureReason::Io,
            format!(
                "Preferences refused at {:?}: {error}",
                path.display().to_string()
            ),
        )),
    }
}

/// Set every connection option §4.4–§4.5 relies on, and read each back: a
/// mismatch refuses preferences.
fn configure(conn: &Connection, display: &str) -> Result<(), SqlFailure> {
    let failed = |what: &str, error: rusqlite::Error| {
        SqlFailure::refused(
            FailureReason::Io,
            format!("Preferences refused at {display:?}: {what} failed: {error}"),
        )
    };
    conn.busy_timeout(Duration::ZERO)
        .map_err(|error| failed("busy_timeout", error))?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .map_err(|error| failed("NO_CKPT_ON_CLOSE", error))?;
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, LENGTH_LIMIT),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, SQL_LENGTH_LIMIT),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, VARIABLE_LIMIT),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
    ] {
        conn.set_limit(limit, value)
            .map_err(|error| failed("a length limit", error))?;
    }
    conn.execute_batch("PRAGMA page_size = 4096")
        .map_err(|error| failed("page_size", error))?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None if matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)
            ) =>
            {
                SqlFailure::refused(
                    FailureReason::Checksum,
                    format!("the database is damaged: {error}"),
                )
            }
            None => failed("journal_mode", error),
        })?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(SqlFailure::refused(
            FailureReason::Io,
            format!(
                "Preferences refused at {display:?}: SQLite WAL mode is unavailable here \
                 (journal mode {mode}); only local same-host filesystems are supported, and \
                 there is no weaker fallback. Use a state root on a local disk."
            ),
        ));
    }
    let mut settings = format!(
        "PRAGMA synchronous = FULL; PRAGMA wal_autocheckpoint = 0; PRAGMA cache_spill = OFF; \
         PRAGMA mmap_size = 0; PRAGMA max_page_count = {MAX_PAGES}; PRAGMA cache_size = \
         {CACHE_SIZE}; PRAGMA soft_heap_limit = {SOFT_HEAP_LIMIT};"
    );
    if cfg!(target_os = "macos") {
        settings.push_str(" PRAGMA fullfsync = ON; PRAGMA checkpoint_fullfsync = ON;");
    }
    for statement in settings.split_inclusive(';') {
        run_pragma(conn, statement.trim()).map_err(|error| failed(statement.trim(), error))?;
    }
    verify_settings(conn).map_err(|mismatch| {
        // A database created with another page size cannot hold this
        // build's WAL and page bounds: name the way out.
        let remedy = if mismatch.contains("page_size") {
            ". Use a compatible 274bot version or a separate correctly owned root"
        } else {
            ""
        };
        SqlFailure::refused(
            FailureReason::Io,
            format!("Preferences refused at {display:?}: {mismatch}{remedy}."),
        )
    })
}

/// Read back every setting the WAL and page limits hold under.
pub(super) fn verify_settings(conn: &Connection) -> Result<(), String> {
    let value = |sql: &str| -> Result<i64, String> {
        conn.query_row(sql, [], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("{sql} could not be read back: {error}"))
    };
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|error| format!("journal_mode could not be read back: {error}"))?;
    let mut checks = vec![
        ("PRAGMA synchronous", 2),
        ("PRAGMA busy_timeout", 0),
        ("PRAGMA wal_autocheckpoint", 0),
        ("PRAGMA cache_spill", 0),
        ("PRAGMA max_page_count", MAX_PAGES as i64),
        ("PRAGMA page_size", PAGE_SIZE as i64),
        ("PRAGMA cache_size", CACHE_SIZE),
        ("PRAGMA soft_heap_limit", SOFT_HEAP_LIMIT),
    ];
    if cfg!(target_os = "macos") {
        checks.push(("PRAGMA fullfsync", 1));
        checks.push(("PRAGMA checkpoint_fullfsync", 1));
    }
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(format!("journal_mode is {mode}, expected wal"));
    }
    for (sql, expected) in checks {
        let found = value(sql)?;
        if found != expected {
            return Err(format!("{sql} is {found}, expected {expected}"));
        }
    }
    // `PRAGMA mmap_size` answers no row when memory mapping is compiled
    // out; either way it must not map.
    let mmap: Option<i64> = conn
        .query_row("PRAGMA mmap_size", [], |row| row.get(0))
        .optional()
        .map_err(|error| format!("mmap_size could not be read back: {error}"))?;
    if mmap.unwrap_or(0) != 0 {
        return Err(format!("mmap_size is {mmap:?}, expected 0"));
    }
    match conn.db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE) {
        Ok(true) => {}
        Ok(false) => return Err("close-time checkpoints are enabled".into()),
        Err(error) => return Err(format!("NO_CKPT_ON_CLOSE could not be read back: {error}")),
    }
    for (limit, expected) in [
        (Limit::SQLITE_LIMIT_LENGTH, LENGTH_LIMIT),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, SQL_LENGTH_LIMIT),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, VARIABLE_LIMIT),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
    ] {
        let found = conn
            .limit(limit)
            .map_err(|error| format!("{limit:?} could not be read back: {error}"))?;
        if found != expected {
            return Err(format!("{limit:?} is {found}, expected {expected}"));
        }
    }
    Ok(())
}

fn run_pragma(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    let mut statement = conn.prepare(sql)?;
    let mut rows = statement.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

fn read_head(conn: &Connection) -> Result<StoreVersion, SqlFailure> {
    let raw: Option<Vec<u8>> = conn
        .query_row(
            "SELECT version FROM domain_head WHERE domain = ?1",
            params![PREFERENCES_HEAD],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| refused_by(error, "reading the preferences head"))?;
    match raw {
        None => Ok(StoreVersion::INITIAL),
        Some(raw) => decode_u64(&raw)
            .map(StoreVersion::from_durable)
            .ok_or_else(|| {
                SqlFailure::refused(
                    FailureReason::Checksum,
                    "the stored preferences version is not 8 bytes",
                )
            }),
    }
}

fn read_sequence(conn: &Connection) -> Result<StoreVersion, SqlFailure> {
    let raw: Vec<u8> = conn
        .query_row("SELECT sequence FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| refused_by(error, "reading the version sequence"))?;
    decode_u64(&raw)
        .map(StoreVersion::from_durable)
        .ok_or_else(|| {
            SqlFailure::refused(
                FailureReason::Checksum,
                "the stored version sequence is not 8 bytes",
            )
        })
}

/// The next durable version inside the caller's write transaction: a
/// checked increment of the largest of the stored sequence, the service
/// clock and the head. `Busy` (refuse, never wrap) when spent.
fn next_version(
    conn: &Connection,
    clock: Option<StoreVersion>,
) -> Result<StoreVersion, SqlFailure> {
    let base = [Some(read_sequence(conn)?), clock, Some(read_head(conn)?)]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(StoreVersion::INITIAL);
    base.checked_next().ok_or_else(|| {
        SqlFailure::refused(
            FailureReason::Schema,
            "the preferences version space is spent; nothing is reused or wrapped",
        )
    })
}

fn write_head(conn: &Connection, version: StoreVersion) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO domain_head (domain, version) VALUES (?1, ?2)
         ON CONFLICT (domain) DO UPDATE SET version = excluded.version",
        params![PREFERENCES_HEAD, be(version.get())],
    )?;
    Ok(())
}

fn read_document(conn: &Connection, version: StoreVersion) -> Result<Durable, SqlFailure> {
    let body: Option<Vec<u8>> = conn
        .query_row(
            "SELECT body FROM preference_document WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| refused_by(error, "reading the preferences document"))?;
    match body {
        Some(body) => Durable::parse(version, &body),
        None => {
            let empty = files::preferences_bytes(&Map::new()).map_err(|_| closed())?;
            Durable::parse(version, &empty)
        }
    }
}

fn read_ledger(conn: &Connection) -> Result<Option<Ledger>, SqlFailure> {
    conn.query_row(
        "SELECT digest, evidence FROM legacy_source WHERE id = 1",
        [],
        |row| {
            let digest: Option<Vec<u8>> = row.get(0)?;
            Ok(Ledger {
                digest: digest.and_then(|raw| raw.try_into().ok()),
                evidence: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|error| refused_by(error, "reading the import ledger"))
}

fn read_export(conn: &Connection) -> Result<Option<ExportHead>, SqlFailure> {
    conn.query_row(
        "SELECT export_id, digest, state FROM export_head WHERE id = 1",
        [],
        |row| {
            let id: Vec<u8> = row.get(0)?;
            let digest: Vec<u8> = row.get(1)?;
            Ok((id, digest, row.get::<_, String>(2)?))
        },
    )
    .optional()
    .map_err(|error| refused_by(error, "reading the export ledger"))?
    .map(|(id, digest, state)| {
        Ok(ExportHead {
            export_id: decode_u64(&id).ok_or_else(|| {
                SqlFailure::refused(FailureReason::Checksum, "the export id is not 8 bytes")
            })?,
            digest: digest.try_into().map_err(|_| {
                SqlFailure::refused(FailureReason::Checksum, "the export digest is not 32 bytes")
            })?,
            state,
        })
    })
    .transpose()
}

/// A full u64 as the fixed-width big-endian BLOB the schema stores.
pub(super) fn be(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

pub(super) fn decode_u64(raw: &[u8]) -> Option<u64> {
    <[u8; 8]>::try_from(raw).ok().map(u64::from_be_bytes)
}

fn hex(digest: &ContentDigest) -> String {
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn bounded_path(path: &str) -> &str {
    let mut end = path.len().min(4096);
    while !path.is_char_boundary(end) {
        end -= 1;
    }
    &path[..end]
}

/// One saved-state info line of a ledger action (import, defaults after a
/// quarantine, reimport, export, recovery).
pub(super) fn audit_line(
    action: &str,
    previous: StoreVersion,
    version: StoreVersion,
    detail: &str,
) -> String {
    format!(
        "audit domain=preferences profile=operator account=operator action={action} \
         origin=import {previous}→{version} at={}; preferences: {detail}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

/// The recovery refusal (plan §5.5): stop every process for the root, then
/// one of the explicit recovery commands of each named binary.
pub(super) fn recover_text(display: &str, cause: &str, frontend: Frontend) -> String {
    let mut text = format!(
        "Preferences refused at {display:?}: {cause}. Stop all 274bot processes for this root."
    );
    for bin in frontend.binaries() {
        text.push_str(&format!(
            " Run '{bin} state preferences recover --source \"<private-valid-json>\"' or '{bin} \
             state preferences recover --defaults'."
        ));
    }
    text.push_str(
        " Recovery preserves the old files first. Defaults recovery discards saved preferences.",
    );
    text
}

fn closed() -> SqlFailure {
    SqlFailure::refused(
        FailureReason::Dirty,
        "the preferences database is not open; it reopens on the next read",
    )
}

/// The wait an SQLite error asks for, when it is lock contention.
pub(super) fn wait_of(error: &rusqlite::Error) -> Option<WaitCause> {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            Some(WaitCause::Busy)
        }
        _ => None,
    }
}

/// A store refusal for an SQLite error during `what`: corruption names the
/// recovery path, anything else its cause.
fn refused_by(error: rusqlite::Error, what: &str) -> SqlFailure {
    let reason = match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            FailureReason::Checksum
        }
        _ => FailureReason::Io,
    };
    SqlFailure::refused(reason, format!("{what}: {error}"))
}

/// A statement failure inside a save, after rollback.
fn statement_failure(error: &rusqlite::Error) -> ErrorCode {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DiskFull | rusqlite::ErrorCode::TooBig) => {
            ErrorCode::Validation {
                domain: LocalStateDomain::Preferences,
                reason: api::local_state::ValidationReason::TooLong,
            }
        }
        _ => ErrorCode::Failed {
            domain: LocalStateDomain::Preferences,
            reason: FailureReason::Io,
        },
    }
}

/// What a save says when the disk error at its COMMIT left its outcome
/// unknown (plan §4.2): the COMMIT may have reached the disk, so the save is
/// never said not to be saved, and never retried. `then` says what follows.
fn unknown_outcome(error: &rusqlite::Error, then: &str) -> Arc<str> {
    Arc::from(format!(
        "the save's outcome is unknown after a disk error at COMMIT ({error}); {then}"
    ))
}

#[cfg(test)]
fn sqlite_error(code: std::os::raw::c_int) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
}
