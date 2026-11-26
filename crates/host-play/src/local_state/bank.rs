//! The bank hints in the state database (T5.4, S5a): one `bank_hint` row for
//! each account and content scope, and one `import_ledger` row for each
//! account's legacy `bank-hints/<profile>/<account>.json`, the first-import
//! source.
//!
//! **One authority.** From the first read of an identity the database is the
//! only place its hint lives. The legacy file is read once, by the strict
//! reader ([`files::read_hint_source`]), and then only kept as `.migrated`
//! evidence: nothing writes it again and a later change to it is never read.
//!
//! **The import.** The first readback of an account's hint, under the product
//! guard and in one write transaction, checks the account's ledger row and
//! then reads the source:
//!
//! - no file: the ledger records `absent`, and the hint starts empty;
//! - a file that records the requesting scope, or none (a schema-1 file from
//!   before scopes: it takes the scope that asks, as 0.2.8 read it): its rows
//!   become the hint of the requesting scope and the ledger records
//!   `imported`, in the same transaction. The ledger is keyed by the source,
//!   so a second scope of the account never imports the file again;
//! - a file that records another scope: nothing is taken or recorded. It
//!   stays unclaimed for the scope it records, and the requesting scope
//!   starts empty;
//! - a file the strict reader refuses (a field named twice, another account,
//!   an old format, broken rows): the store is refused with the path, the
//!   cause and the remedy. No head, no default and no ledger row is
//!   committed, and the file is not changed.
//!
//! After an `imported` commit the source moves to `<file>.migrated` and the
//! ledger records `completed`. A crash in between leaves `pending`, which the
//! next readback finishes.
//!
//! **A read-only projection.** A file with fields this build does not know
//! imports its usable rows, stores them with the number of omitted fields
//! and refuses every later save of that hint (the same rule the file backend
//! had). `state bank-hints accept-projection` clears the refusal once the
//! operator accepts that saves hold the usable rows only
//! ([`Db::accept_bank_projection`]).

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::local_state::{
    AuditCommit, AuditOrigin, CommitAction, DurableReceipt, ErrorCode, FailureReason,
    LeaseSnapshot, LocalStateDomain, StoreVersion, ValidationReason,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::files::{self, ContentDigest, ReadError, Rewrite, StateRoot, StoreKey};
use super::sql::{
    be, begin_write, bounded_path, closed, decode_u64, hex, next_version, open_connection,
    read_ledger_of, refused_by, wait_of, Commit, Db, Evidence, Ledger, Loaded, SqlFailure, Staging,
    WaitCause, BANK_ROWS_MAX, MAX_BANK_HINTS,
};
use super::{
    decide, dirty, io_failure, Cached, CachedDecision, DecisionFile, FileState, Queued, Rejection,
    Shared, Staged, State,
};
#[cfg(test)]
use super::{DomainEdit, WriteIntent};

/// The import ledger's domain of the bank hints.
pub(super) const LEDGER_DOMAIN: &str = "bank-hints";

/// One bank hint's key: the account and the content scope it was observed
/// under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BankKey {
    pub(super) profile: String,
    pub(super) account: String,
    pub(super) revision: i32,
    pub(super) content: String,
}

impl BankKey {
    /// The bank key of a store, `None` for any other domain's store.
    pub(super) fn of(key: &StoreKey) -> Option<Self> {
        match key {
            StoreKey::BankHint {
                profile,
                account,
                revision,
                content,
            } => Some(Self {
                profile: profile.clone(),
                account: account.clone(),
                revision: *revision,
                content: content.clone(),
            }),
            _ => None,
        }
    }

    /// The legacy hint file of this account.
    fn source_path(&self, root: &StateRoot) -> PathBuf {
        root.bank_hint_file(&self.profile, &self.account)
    }

    /// The import ledger's source key: the account, never the scope.
    fn ledger_source(&self) -> String {
        format!("{}/{}", self.profile, self.account)
    }

    /// The ledger row that records that this scope read the account's legacy
    /// hint and found it recorded for another scope: it decides this scope's
    /// saves and never consumes the source.
    fn scope_marker_source(&self) -> String {
        format!(
            "{}#{}/{}",
            self.ledger_source(),
            self.revision,
            self.content
        )
    }

    /// What a log line calls the hint.
    fn label(&self) -> String {
        format!(
            "{}/{} revision {} content {}",
            self.profile, self.account, self.revision, self.content
        )
    }
}

/// One stored hint.
#[derive(Debug, Clone)]
pub(super) struct BankDurable {
    /// The version the hint was committed at. For an absent hint (`digest`
    /// is `None`) the version the service itself holds for it.
    pub(super) version: StoreVersion,
    /// When the rows were observed, in Unix seconds.
    pub(super) observed_at: u64,
    pub(super) snapshot: LeaseSnapshot,
    /// The digest of what is stored; `None` while no hint is stored.
    pub(super) digest: Option<ContentDigest>,
    /// How many fields of the imported file this build did not know: the
    /// hint is read-only while this is not zero.
    pub(super) omitted: u32,
}

/// What a bank readback found.
pub(super) enum BankRead {
    /// No hint is stored for this identity.
    Absent,
    Hint(BankDurable),
    /// The account's legacy source was refused: the store is refused with
    /// its path, cause and remedy, and nothing was committed.
    Refused {
        reason: FailureReason,
        detail: Arc<str>,
    },
}

/// A readback's result and the lines the operator and the audit need.
pub(super) struct BankHydrated {
    pub(super) read: BankRead,
    pub(super) notices: Vec<(Level, String)>,
}

/// The stored form of the rows: 4-byte big-endian id, then count, for each
/// row.
pub(super) fn encode_rows(rows: &[(i32, i32)]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rows.len() * 8);
    for (id, count) in rows {
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.extend_from_slice(&count.to_be_bytes());
    }
    bytes
}

fn decode_rows(bytes: &[u8]) -> Option<Vec<(i32, i32)>> {
    if !bytes.len().is_multiple_of(8) || bytes.len() > BANK_ROWS_MAX {
        return None;
    }
    Some(
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|row| {
                (
                    i32::from_be_bytes([row[0], row[1], row[2], row[3]]),
                    i32::from_be_bytes([row[4], row[5], row[6], row[7]]),
                )
            })
            .collect(),
    )
}

/// What a hint's digest is taken over: the observation time, then the
/// stored rows. It is the proof of an uncertain COMMIT and the unchanged
/// check of a save.
pub(super) fn stored_bytes(rows: &[(i32, i32)], observed_at: u64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + rows.len() * 8);
    bytes.extend_from_slice(&observed_at.to_be_bytes());
    bytes.extend_from_slice(&encode_rows(rows));
    bytes
}

fn durable_of(
    version: StoreVersion,
    observed_at: u64,
    stored: &[u8],
    omitted: u32,
) -> Result<BankDurable, SqlFailure> {
    let rows = decode_rows(stored).ok_or_else(|| {
        SqlFailure::refused(
            FailureReason::Checksum,
            "a stored bank hint does not hold whole rows",
        )
    })?;
    let mut hashed = Vec::with_capacity(8 + stored.len());
    hashed.extend_from_slice(&observed_at.to_be_bytes());
    hashed.extend_from_slice(stored);
    let snapshot = LeaseSnapshot::bank_hints(rows).map_err(|error| {
        SqlFailure::refused(
            FailureReason::Checksum,
            format!("a stored bank hint breaks the row rules ({error})"),
        )
    })?;
    Ok(BankDurable {
        version,
        observed_at,
        snapshot,
        digest: Some(files::digest(&hashed)),
        omitted,
    })
}

fn absent_durable(version: StoreVersion) -> BankDurable {
    BankDurable {
        version,
        observed_at: 0,
        snapshot: LeaseSnapshot::bank_hints(Vec::new()).expect("no rows meet the row rules"),
        digest: None,
        omitted: 0,
    }
}

/// The stored hint of `key`, if any.
fn read_bank_row(conn: &Connection, key: &BankKey) -> Result<Option<BankDurable>, SqlFailure> {
    let row = conn
        .query_row(
            "SELECT version, observed_at, rows, omitted FROM bank_hint
             WHERE profile = ?1 AND account = ?2 AND revision = ?3 AND content = ?4",
            params![key.profile, key.account, key.revision, key.content],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| refused_by(error, "reading a bank hint"))?;
    let Some((version, observed_at, stored, omitted)) = row else {
        return Ok(None);
    };
    let (Some(version), Ok(observed_at), Ok(omitted)) = (
        decode_u64(&version).map(StoreVersion::from_durable),
        u64::try_from(observed_at),
        u32::try_from(omitted),
    ) else {
        return Err(SqlFailure::refused(
            FailureReason::Checksum,
            "a stored bank hint carries a version or a time that is not a number",
        ));
    };
    durable_of(version, observed_at, &stored, omitted).map(Some)
}

fn bank_count(conn: &Connection) -> Result<u64, SqlFailure> {
    conn.query_row("SELECT count(*) FROM bank_hint", [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|count| u64::try_from(count).unwrap_or(0))
    .map_err(|error| refused_by(error, "counting the bank hints"))
}

/// One account's first-import row of the ledger.
fn read_bank_ledger(conn: &Connection, key: &BankKey) -> Result<Option<Ledger>, SqlFailure> {
    read_ledger_of(conn, LEDGER_DOMAIN, &key.ledger_source())
}

fn insert_bank_row(
    conn: &Connection,
    key: &BankKey,
    version: StoreVersion,
    observed_at: i64,
    rows: &[(i32, i32)],
    omitted: usize,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO bank_hint
             (profile, account, revision, content, version, observed_at, rows, omitted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (profile, account, revision, content) DO UPDATE SET
             version = excluded.version, observed_at = excluded.observed_at,
             rows = excluded.rows, omitted = excluded.omitted",
        params![
            key.profile,
            key.account,
            key.revision,
            key.content,
            be(version.get()),
            observed_at,
            encode_rows(rows),
            i64::try_from(omitted).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(())
}

/// The audit line of an import, a bank hint's counterpart of the
/// preferences' (`sql::audit_line`).
fn audit_line(key: &BankKey, action: &str, version: StoreVersion, detail: &str) -> String {
    format!(
        "audit domain=bank-hints profile={} account={} action={action} origin=import \
         {}→{version} at={}; bank-hints: {detail}",
        key.profile,
        key.account,
        StoreVersion::INITIAL,
        DurableTimestamp::now().unix_seconds()
    )
}

/// The refusal of a source the strict reader refused: the exact path, the
/// cause and the remedy.
fn source_refusal(display: &str, error: &ReadError) -> BankRead {
    let remedy = match error {
        ReadError::DuplicateField { .. } => {
            "Edit the file to remove one of them, or remove the file to start empty."
        }
        ReadError::Identity { .. } => {
            "Move the file to the folder and name of the account it names, or remove it to \
             start empty."
        }
        ReadError::UnsupportedSavedFormat => {
            "This saved data is from before 0.2.8, which this build cannot read. Start again \
             and choose to move it aside, or remove it to start empty."
        }
        ReadError::Schema { .. } => {
            "Use the 274bot version that wrote it, or remove the file to start empty."
        }
        ReadError::Scope { .. } => {
            "Edit the file to record both parts of its scope or neither, or remove it to start \
             empty."
        }
        ReadError::Rows(_) => {
            "Edit the rows to be sorted, unique and positive (at most 1024), or remove the file \
             to start empty."
        }
        ReadError::Io(_) => {
            "Restore a readable private hint file (a regular file, mode 600) at that path, or \
             remove it to start empty."
        }
        _ => "Fix the file so it is one JSON hint document, or remove it to start empty.",
    };
    BankRead::Refused {
        reason: error.reason(),
        detail: Arc::from(format!(
            "Bank hints refused at {display:?}: {error}. The file was not changed and nothing \
             was imported. {remedy}"
        )),
    }
}

impl Db {
    /// One bank readback: the stored hint of `key`, importing the account's
    /// legacy hint file first when its first import is still undecided (see
    /// the module notes).
    pub(super) fn bank_read(
        &mut self,
        key: &BankKey,
        clock: Option<StoreVersion>,
    ) -> Result<BankHydrated, SqlFailure> {
        self.open_for_domain()?;
        let mut result = self.bank_read_open(key, clock);
        self.after_failure(&mut result);
        result
    }

    fn bank_read_open(
        &mut self,
        key: &BankKey,
        clock: Option<StoreVersion>,
    ) -> Result<BankHydrated, SqlFailure> {
        let mut notices = Vec::new();
        let (row, ledger) = {
            let conn = self.conn.as_mut().ok_or_else(closed)?;
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(|error| match wait_of(&error) {
                    Some(cause) => SqlFailure::Wait(cause),
                    None => refused_by(error, "reading a bank hint"),
                })?;
            let row = read_bank_row(&tx, key)?;
            let ledger = read_bank_ledger(&tx, key)?;
            tx.commit()
                .map_err(|error| refused_by(error, "ending the bank hint read"))?;
            (row, ledger)
        };
        match (row, ledger) {
            (row, Some(ledger)) => {
                if ledger.evidence == "pending" {
                    let evidence = Evidence {
                        source: key.source_path(&self.root),
                        digest: ledger.digest,
                        domain: LEDGER_DOMAIN,
                        ledger_source: key.ledger_source(),
                        what: "bank hint",
                    };
                    self.finish_import_evidence(&evidence, &mut notices);
                }
                Ok(BankHydrated {
                    read: row.map_or(BankRead::Absent, BankRead::Hint),
                    notices,
                })
            }
            // A stored hint with no ledger row cannot come from this
            // module's writes; the stored hint stands and no import runs.
            (Some(row), None) => Ok(BankHydrated {
                read: BankRead::Hint(row),
                notices,
            }),
            (None, None) => self.bank_first_import(key, clock, notices),
        }
    }

    /// The first import of `key`'s account (module notes), in one write
    /// transaction under the guard.
    fn bank_first_import(
        &mut self,
        key: &BankKey,
        clock: Option<StoreVersion>,
        mut notices: Vec<(Level, String)>,
    ) -> Result<BankHydrated, SqlFailure> {
        let guard = self.guard()?;
        let source = key.source_path(&self.root);
        let display = source.display().to_string();
        #[cfg(test)]
        let seam = self.seams.point.clone();
        let point = move |name: &'static str| {
            #[cfg(test)]
            if let Some(seam) = &seam {
                seam(name);
            }
            #[cfg(not(test))]
            let _ = name;
        };
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(conn, &self.root, &guard, "starting the bank hint import")?;
        // Another process may have decided this source since the read.
        if let Some(row) = read_bank_row(&tx, key)? {
            return Ok(BankHydrated {
                read: BankRead::Hint(row),
                notices,
            });
        }
        if read_bank_ledger(&tx, key)?.is_some()
            || read_ledger_of(&tx, LEDGER_DOMAIN, UNKNOWN_SOURCES)?.is_some()
        {
            return Ok(BankHydrated {
                read: BankRead::Absent,
                notices,
            });
        }
        let commit_error = |error: rusqlite::Error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the bank hint import"),
        };
        let ledger_row =
            |tx: &Connection, digest: Option<&ContentDigest>, kind: &str, evidence: &str| {
                tx.execute(
                    "INSERT INTO import_ledger (domain, source, digest, kind, evidence, path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        LEDGER_DOMAIN,
                        key.ledger_source(),
                        digest.map(|digest| digest.as_slice()),
                        kind,
                        evidence,
                        bounded_path(&display)
                    ],
                )
                .map(|_| ())
                .map_err(|error| refused_by(error, "writing the bank hint import"))
            };
        let source_hint = match files::read_hint_source(&source, &key.profile, &key.account) {
            Ok(source_hint) => source_hint,
            Err(error) => {
                return Ok(BankHydrated {
                    read: source_refusal(&display, &error),
                    notices,
                })
            }
        };
        let Some(hint) = source_hint else {
            ledger_row(&tx, None, "absent", "none")?;
            tx.commit().map_err(commit_error)?;
            notices.push((
                Level::Info,
                audit_line(
                    key,
                    "bank-hint-start-empty",
                    StoreVersion::INITIAL,
                    &format!("no legacy file at {display}"),
                ),
            ));
            return Ok(BankHydrated {
                read: BankRead::Absent,
                notices,
            });
        };
        if let Some((revision, content)) = &hint.scope {
            if *revision != key.revision || *content != key.content {
                // Recorded for another scope: not this scope's to take, and
                // not consumed. A marker of this scope records that it looked,
                // so this scope's own saves are decided without the source.
                tx.execute(
                    "INSERT OR IGNORE INTO import_ledger
                         (domain, source, digest, kind, evidence, path)
                     VALUES (?1, ?2, NULL, 'other-scope', 'none', ?3)",
                    params![
                        LEDGER_DOMAIN,
                        key.scope_marker_source(),
                        bounded_path(&display)
                    ],
                )
                .map_err(|error| refused_by(error, "writing the bank hint scope marker"))?;
                tx.commit().map_err(commit_error)?;
                return Ok(BankHydrated {
                    read: BankRead::Absent,
                    notices,
                });
            }
        }
        let Ok(observed_at) = i64::try_from(hint.observed_at_unix) else {
            return Ok(BankHydrated {
                read: source_refusal(
                    &display,
                    &ReadError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "its observation time is not a time this build can store",
                    )),
                ),
                notices,
            });
        };
        if bank_count(&tx)? >= MAX_BANK_HINTS {
            return Ok(BankHydrated {
                read: BankRead::Refused {
                    reason: FailureReason::Io,
                    detail: Arc::from(format!(
                        "Bank hints refused at {display:?}: the database already holds the \
                         most bank hints it keeps ({MAX_BANK_HINTS}), so this account's file \
                         cannot be imported. The file was not changed. Remove hints of \
                         accounts you no longer use, then retry."
                    )),
                },
                notices,
            });
        }
        let rows = hint
            .snapshot
            .as_bank_hints()
            .expect("a hint source holds bank rows");
        point("before-bank-import-commit");
        let version = next_version(&tx, clock)?;
        insert_bank_row(&tx, key, version, observed_at, rows, hint.omitted_fields)
            .and_then(|()| {
                tx.execute(
                    "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                    params![be(version.get())],
                )
                .map(|_| ())
            })
            .map_err(|error| refused_by(error, "writing the bank hint import"))?;
        ledger_row(&tx, Some(&hint.digest), "imported", "pending")?;
        tx.commit().map_err(commit_error)?;
        point("after-bank-import-commit");
        let durable = read_bank_row_after(self, key)?;
        notices.push((
            Level::Info,
            audit_line(
                key,
                "bank-hint-import",
                version,
                &format!(
                    "imported {} row(s) from {display} sha256={}",
                    rows.len(),
                    hex(&hint.digest)
                ),
            ),
        ));
        if hint.omitted_fields > 0 {
            notices.push((
                Level::Warn,
                format!(
                    "local state: the bank hint file {display} has {} field(s) this build does \
                     not know; its rows are imported but the hint is read-only. Its original \
                     is kept as .migrated evidence. Run '{} state bank-hints accept-projection \
                     {}' to save hints that hold the usable rows only.",
                    hint.omitted_fields,
                    self.frontend.binary(),
                    key.store_name()
                ),
            ));
        }
        let evidence = Evidence {
            source,
            digest: Some(hint.digest),
            domain: LEDGER_DOMAIN,
            ledger_source: key.ledger_source(),
            what: "bank hint",
        };
        drop(guard);
        self.finish_import_evidence(&evidence, &mut notices);
        Ok(BankHydrated {
            read: BankRead::Hint(durable),
            notices,
        })
    }

    /// One bank save, through the one commit path the preferences take
    /// ([`Db::commit`]). `known` is what the service holds for the hint.
    pub(super) fn bank_commit<X, B>(
        &mut self,
        request: RequestId,
        clock: Option<StoreVersion>,
        key: &BankKey,
        known: &BankDurable,
        begun: B,
        decide: X,
    ) -> Result<Commit<BankStaged>, SqlFailure>
    where
        X: FnOnce(&BankDurable) -> Result<BankStaged, ErrorCode>,
        B: FnOnce() -> bool,
    {
        self.open_for_domain()?;
        self.commit(
            request,
            clock,
            Some(known),
            begun,
            |conn| load_bank(conn, key, known),
            decide,
        )
    }

    /// Clear the read-only refusal of an imported hint (the operator
    /// accepted that saves hold the usable rows only), under the maintenance
    /// lease and the guard `guard`. Refuses without a change when the hint's
    /// version is not `expected`, or the original is not kept as evidence.
    pub(super) fn accept_bank_projection(
        &mut self,
        guard: &files::WriteLock,
        key: &BankKey,
        expected: StoreVersion,
    ) -> Result<(), SqlFailure> {
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(
            conn,
            &self.root,
            guard,
            "starting the projection acceptance",
        )?;
        let Some(row) = read_bank_row(&tx, key)? else {
            return Err(SqlFailure::refused(
                FailureReason::Dirty,
                format!(
                    "there is no stored bank hint for {}; nothing was changed",
                    key.label()
                ),
            ));
        };
        if row.version != expected || row.omitted == 0 {
            return Err(SqlFailure::refused(
                FailureReason::Dirty,
                format!(
                    "the bank hint for {} changed or has no read-only projection to accept \
                     (version {}, {} omitted field(s)); nothing was changed",
                    key.label(),
                    row.version,
                    row.omitted
                ),
            ));
        }
        let version = next_version(&tx, None)?;
        tx.execute(
            "UPDATE bank_hint SET version = ?1, omitted = 0
             WHERE profile = ?2 AND account = ?3 AND revision = ?4 AND content = ?5",
            params![
                be(version.get()),
                key.profile,
                key.account,
                key.revision,
                key.content
            ],
        )
        .and_then(|_| {
            tx.execute(
                "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                params![be(version.get())],
            )
        })
        .map_err(|error| refused_by(error, "accepting the projection"))?;
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the projection acceptance"),
        })
    }

    /// The stored hint of `key` with the import ledger's evidence of it,
    /// for the maintenance command.
    pub(super) fn bank_report(
        &mut self,
        key: &BankKey,
    ) -> Result<Option<(BankDurable, Ledger)>, SqlFailure> {
        let conn = self.conn.as_ref().ok_or_else(closed)?;
        let Some(row) = read_bank_row(conn, key)? else {
            return Ok(None);
        };
        let Some(ledger) = read_bank_ledger(conn, key)? else {
            return Ok(None);
        };
        Ok(Some((row, ledger)))
    }
}

impl BankKey {
    /// The name the maintenance command takes: `profile/account/revision/content`.
    pub(super) fn store_name(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.profile, self.account, self.revision, self.content
        )
    }

    /// The kept original of this account's imported hint file: the
    /// `.migrated` sibling of the source that holds `digest`.
    pub(super) fn evidence_file(
        &self,
        root: &StateRoot,
        digest: Option<ContentDigest>,
    ) -> Option<PathBuf> {
        let digest = digest?;
        let source = self.source_path(root);
        let name = source.file_name()?.to_string_lossy().into_owned();
        (0..100u32)
            .map(|attempt| {
                source.with_file_name(match attempt {
                    0 => format!("{name}.migrated"),
                    n => format!("{name}.migrated-{n}"),
                })
            })
            .find(|candidate| {
                std::fs::read(candidate).is_ok_and(|bytes| files::digest(&bytes) == digest)
            })
    }

    /// The key a maintenance command names.
    pub(super) fn parse(name: &str) -> Option<Self> {
        let mut parts = name.split('/');
        let (profile, account, revision, content) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || profile.is_empty() || account.is_empty() || content.is_empty()
        {
            return None;
        }
        Some(Self {
            profile: profile.to_string(),
            account: account.to_string(),
            revision: revision.parse().ok()?,
            content: content.to_string(),
        })
    }
}

fn read_bank_row_after(db: &mut Db, key: &BankKey) -> Result<BankDurable, SqlFailure> {
    let conn = db.conn.as_ref().ok_or_else(closed)?;
    read_bank_row(conn, key)?.ok_or_else(closed)
}

/// What a bank save's transaction starts from: the head, and the stored
/// hint when it is not the version the caller holds.
fn load_bank(
    conn: &Connection,
    key: &BankKey,
    known: &BankDurable,
) -> Result<Loaded<BankDurable>, SqlFailure> {
    let row = read_bank_row(conn, key)?;
    let head = row
        .as_ref()
        .map_or(StoreVersion::INITIAL, |row| row.version);
    let observed = match row {
        Some(row) if known.digest.is_some() && known.version == row.version => None,
        Some(row) => Some(row),
        None if known.digest.is_none() => None,
        // A hint that was stored and is gone: the service's own version
        // stands, and the save re-creates the hint.
        None => Some(absent_durable(known.version)),
    };
    Ok(Loaded { head, observed })
}

/// One bank save, staged: what [`decide`] made of the edit, and the key it
/// is stored under.
pub(super) struct BankStaged {
    key: BankKey,
    staged: Staged,
}

impl Staging for BankStaged {
    type Durable = BankDurable;
    const DOMAIN: LocalStateDomain = LocalStateDomain::BankHints;
    const HEAD: &'static str = LEDGER_DOMAIN;
    const ACTION: &'static str = "bank-hint-write";

    fn unchanged(&self) -> bool {
        self.staged.unchanged
    }

    fn admit(&self, tx: &Connection) -> Result<(), (ErrorCode, &'static str)> {
        let domain = LocalStateDomain::BankHints;
        let io = (
            ErrorCode::Failed {
                domain,
                reason: FailureReason::Io,
            },
            "the bank hints could not be checked",
        );
        if i64::try_from(self.staged.at.unix_seconds()).is_err() {
            return Err((
                ErrorCode::Validation {
                    domain,
                    reason: ValidationReason::TooLong,
                },
                "the observation time cannot be stored",
            ));
        }
        // The import decided this account's source before any save: a save
        // against an undecided source would let a later import replace it.
        let decided = match read_bank_ledger(tx, &self.key) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => read_ledger_of(tx, LEDGER_DOMAIN, &self.key.scope_marker_source())
                .map(|marker| marker.is_some()),
            Err(error) => Err(error),
        };
        match decided {
            Ok(true) => {}
            Ok(false) => {
                return Err((
                    ErrorCode::Failed {
                        domain,
                        reason: FailureReason::Dirty,
                    },
                    "this account's legacy bank hint was not imported yet",
                ))
            }
            Err(_) => return Err(io),
        }
        let present = read_bank_row(tx, &self.key);
        match present {
            Ok(Some(_)) => Ok(()),
            Ok(None) => match bank_count(tx) {
                Ok(count) if count < MAX_BANK_HINTS => Ok(()),
                Ok(_) => Err((
                    ErrorCode::Busy { domain },
                    "the database holds the most bank hints it keeps",
                )),
                Err(_) => Err(io),
            },
            Err(_) => Err(io),
        }
    }

    fn digest(&self) -> ContentDigest {
        files::digest(&self.staged.bytes)
    }

    fn set_version(&mut self, version: StoreVersion) {
        self.staged.version = version;
    }

    fn write(&self, tx: &Connection, version: StoreVersion) -> rusqlite::Result<()> {
        let rows = self
            .staged
            .snapshot
            .as_bank_hints()
            .expect("a bank save stages bank rows");
        let observed_at = i64::try_from(self.staged.at.unix_seconds()).unwrap_or(i64::MAX);
        insert_bank_row(tx, &self.key, version, observed_at, rows, 0)
    }

    fn durable(&self, version: StoreVersion, digest: ContentDigest) -> BankDurable {
        BankDurable {
            version,
            observed_at: self.staged.at.unix_seconds(),
            snapshot: self.staged.snapshot.clone(),
            digest: Some(digest),
            omitted: 0,
        }
    }
}

impl State {
    /// Take what a bank readback found into the store `key`: a hint at the
    /// version the database holds, or none. A version the store did not hold
    /// is an outside change (another process's commit), counted once.
    pub(super) fn adopt_bank(&mut self, key: &StoreKey, read: BankRead) {
        let Some(store) = self.stores.get(key) else {
            return;
        };
        let known = !matches!(store.file, FileState::Cold);
        let label = BankKey::of(key)
            .map(|bank| bank.label())
            .unwrap_or_default();
        match read {
            BankRead::Refused { reason, detail } => self.reject_bank(key, reason, detail),
            BankRead::Hint(durable) => {
                if let FileState::Read(cached) = &store.file {
                    if cached.digest.is_some() && cached.version == durable.version {
                        if let Some(FileState::Read(cached)) =
                            self.stores.get_mut(key).map(|store| &mut store.file)
                        {
                            cached.refreshed_at = Instant::now();
                        }
                        return;
                    }
                }
                self.observe_version(durable.version);
                let store = self.stores.get_mut(key).expect("checked above");
                if known {
                    store.external += 1;
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        "local state: bank-hints changed outside this service, now {} for {label}",
                        durable.version
                    );
                }
                let rewrite = if durable.omitted > 0 {
                    host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "local state: bank-hints hint {label} has {} omitted field(s); its rows \
                         are read but the hint is never rewritten until the projection is \
                         accepted",
                        durable.omitted
                    );
                    Rewrite::Refused {
                        unreadable: 0,
                        lossy: durable.omitted as usize,
                    }
                } else {
                    Rewrite::Lossless
                };
                store.file = FileState::Read(Cached {
                    version: durable.version,
                    at: DurableTimestamp::from_unix_seconds(durable.observed_at),
                    snapshot: durable.snapshot,
                    preferences: None,
                    refreshed_at: Instant::now(),
                    digest: durable.digest,
                    durable: true,
                    written_len: 0,
                    rewrite,
                });
            }
            BankRead::Absent => {
                if let FileState::Read(cached) = &store.file {
                    if cached.digest.is_none() {
                        if let Some(FileState::Read(cached)) =
                            self.stores.get_mut(key).map(|store| &mut store.file)
                        {
                            cached.refreshed_at = Instant::now();
                        }
                        return;
                    }
                }
                let version = self.next_version();
                let store = self.stores.get_mut(key).expect("checked above");
                let Some(version) = version else {
                    store.file = FileState::Rejected(Rejection {
                        reason: FailureReason::Io,
                        detail: "the version space is spent".into(),
                    });
                    return;
                };
                if known {
                    store.external += 1;
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        "local state: bank-hints removed outside this service, now {version} \
                         for {label}"
                    );
                }
                let absent = absent_durable(version);
                store.file = FileState::Read(Cached {
                    version,
                    at: DurableTimestamp::now(),
                    snapshot: absent.snapshot,
                    preferences: None,
                    refreshed_at: Instant::now(),
                    digest: None,
                    durable: true,
                    written_len: 0,
                    rewrite: Rewrite::Lossless,
                });
            }
        }
    }

    /// The database refused the bank hint `key`: no lease and no save until
    /// a later read accepts it. Logged once per distinct cause.
    pub(super) fn reject_bank(&mut self, key: &StoreKey, reason: FailureReason, detail: Arc<str>) {
        let Some(store) = self.stores.get_mut(key) else {
            return;
        };
        if let FileState::Rejected(rejection) = &store.file {
            if rejection.detail == detail {
                return;
            }
        }
        super::emit_local_notice(
            Level::Warn,
            format_args!(
                "local state: bank-hints refused: {detail}; it is neither leased nor written \
                 until it reads cleanly"
            ),
            #[cfg(test)]
            self.hooks.read_refusal.as_ref(),
        );
        if matches!(store.file, FileState::Read(_)) {
            store.external += 1;
        }
        store.file = FileState::Rejected(Rejection { reason, detail });
    }
}

impl Shared {
    /// One bank hint's readback on the writer thread: open the database,
    /// import the account's legacy file when its first import is undecided,
    /// and take the stored hint into the store. `Some` when the database
    /// asked to wait: the job is requeued.
    pub(super) fn readback_bank(&self, db: &mut Db, key: &StoreKey) -> Option<WaitCause> {
        #[cfg(any(test, feature = "test-support"))]
        {
            let read_stall = self.state.lock().read_stall.clone();
            if let Some(stall) = read_stall {
                stall();
            }
        }
        let bank = BankKey::of(key).expect("a bank readback names a bank hint");
        let clock = self.state.lock().clock;
        let read = db.bank_read(&bank, clock);
        let mut state = self.state.lock();
        match read {
            Err(SqlFailure::Wait(cause)) => {
                drop(state);
                return Some(cause);
            }
            Ok(hydrated) => {
                // The notices are logged outside the service lock.
                drop(state);
                for (level, line) in hydrated.notices {
                    self.notice(level, format_args!("{line}"));
                }
                state = self.state.lock();
                state.adopt_bank(key, hydrated.read);
            }
            Err(SqlFailure::Refused { reason, detail }) => state.reject_bank(key, reason, detail),
        }
        state.bank_readback_retry = Default::default();
        state.readback_done(key);
        None
    }

    /// One bank hint save through the database: the worker decides the edit
    /// against the hint the write transaction reads, at a version allocated
    /// there. `Some` when the database asked to wait: the caller requeues the
    /// save.
    pub(super) fn write_bank(
        &self,
        db: &mut Db,
        key: &StoreKey,
        queued: &Queued,
    ) -> Option<WaitCause> {
        let domain = key.domain();
        let request = queued.request;
        #[cfg(any(test, feature = "test-support"))]
        {
            let stall = self.state.lock().write_stall.clone();
            if let Some(stall) = stall {
                stall();
            }
            let fault = self.state.lock().write_fault.clone();
            if fault.is_some_and(|fault| fault()) {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: {domain} write failed (test fault) path={}",
                    self.root.database_file().display()
                );
                self.finish(key, queued, io_failure(domain), false);
                return None;
            }
        }
        if self.state.lock().abandoned {
            self.finish(
                key,
                queued,
                Settlement::Cancelled(Cancellation::Shutdown),
                false,
            );
            return None;
        }
        let bank = BankKey::of(key).expect("a bank write names a bank hint");
        let (known, external, clock) = {
            let state = self.state.lock();
            let Some(store) = state.stores.get(key) else {
                drop(state);
                self.finish(key, queued, dirty(domain), false);
                return None;
            };
            let known = match &store.file {
                FileState::Read(cached) => BankDurable {
                    version: cached.version,
                    observed_at: cached.at.unix_seconds(),
                    snapshot: cached.snapshot.clone(),
                    digest: cached.digest,
                    omitted: match cached.rewrite {
                        Rewrite::Refused { lossy, .. } => lossy as u32,
                        Rewrite::Lossless => 0,
                    },
                },
                FileState::Rejected(rejection) => {
                    let settlement = Settlement::Failed(ErrorCode::Failed {
                        domain,
                        reason: rejection.reason,
                    });
                    let detail = Arc::clone(&rejection.detail);
                    drop(state);
                    self.finish_with_detail(key, queued, settlement, Some(detail));
                    return None;
                }
                FileState::Cold => {
                    drop(state);
                    self.finish(key, queued, dirty(domain), false);
                    return None;
                }
            };
            (known, store.external, state.clock)
        };
        let known_version = known.version;
        let committed = db.bank_commit(
            request,
            clock,
            &bank,
            &known,
            || self.begin_sql_write(key),
            |durable| {
                let external_now = if durable.version == known_version {
                    external
                } else {
                    external + 1
                };
                let file = DecisionFile::Read(CachedDecision {
                    version: durable.version,
                    snapshot: None,
                    preferences: None,
                    digest: durable.digest,
                    durable: true,
                    written_len: 0,
                    rewrite: if durable.omitted > 0 {
                        Rewrite::Refused {
                            unreadable: 0,
                            lossy: durable.omitted as usize,
                        }
                    } else {
                        Rewrite::Lossless
                    },
                });
                let staged =
                    decide(key, &file, external_now, queued).map_err(
                        |settlement| match settlement {
                            Settlement::Failed(code) => code,
                            Settlement::Completed(_) | Settlement::Cancelled(_) => {
                                ErrorCode::Failed {
                                    domain,
                                    reason: FailureReason::Dirty,
                                }
                            }
                        },
                    )?;
                // A test may hold a save that will commit, inside its
                // transaction; one that changes nothing is not held.
                #[cfg(test)]
                if !staged.unchanged {
                    let hook = self.state.lock().hooks.bank_write.clone();
                    if let (Some(hook), WriteIntent::Port(DomainEdit::Bank(hint))) =
                        (hook, &queued.intent)
                    {
                        hook(hint.rows());
                    }
                }
                Ok(BankStaged {
                    key: bank.clone(),
                    staged,
                })
            },
        );
        match committed {
            Err(SqlFailure::Wait(cause)) => Some(cause),
            Err(SqlFailure::Refused { reason, detail }) => {
                self.state
                    .lock()
                    .reject_bank(key, reason, Arc::clone(&detail));
                self.finish_with_detail(
                    key,
                    queued,
                    Settlement::Failed(ErrorCode::Failed { domain, reason }),
                    Some(detail),
                );
                None
            }
            Ok(Commit::Declined {
                code,
                detail,
                observed,
            }) => {
                self.adopt_observed(key, observed);
                self.finish_with_detail(key, queued, Settlement::Failed(code), detail);
                None
            }
            Ok(Commit::Unresolved { detail, observed }) => {
                self.adopt_observed(key, observed);
                self.finish_unresolved(key, queued, io_failure(domain), detail);
                None
            }
            Ok(Commit::Abandoned) => {
                self.finish(
                    key,
                    queued,
                    Settlement::Cancelled(Cancellation::Shutdown),
                    false,
                );
                None
            }
            Ok(Commit::Unchanged { staged, observed }) => {
                self.adopt_observed(key, observed);
                self.finish(
                    key,
                    queued,
                    Settlement::Completed(DurableReceipt::new(
                        queued.intent.identity().clone(),
                        staged.staged.previous,
                        DurableTimestamp::now(),
                        Some(request),
                    )),
                    false,
                );
                None
            }
            Ok(Commit::Committed {
                staged,
                durable,
                observed,
                truncate_due,
                recovered,
            }) => {
                let version = durable.version;
                {
                    let mut state = self.state.lock();
                    state.observe_version(version);
                    if truncate_due {
                        state.truncate.due = true;
                    }
                    if let Some(store) = state.stores.get_mut(key) {
                        if observed.is_some() {
                            store.external += 1;
                        }
                        store.file = FileState::Read(Cached {
                            version,
                            at: staged.staged.at,
                            snapshot: staged.staged.snapshot.clone(),
                            preferences: None,
                            refreshed_at: Instant::now(),
                            digest: durable.digest,
                            durable: true,
                            written_len: 0,
                            rewrite: Rewrite::Lossless,
                        });
                    }
                }
                let now = DurableTimestamp::now();
                let identity = queued.intent.identity().clone();
                if recovered {
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        "local state: recovered commit epoch={} request={} after an uncertain \
                         COMMIT; it is durable at {version}",
                        db.epoch.unwrap_or_default(),
                        request.get()
                    );
                }
                if let Some(audit) = &staged.staged.audit {
                    self.audit(
                        AuditCommit::after_commit(
                            identity.clone(),
                            CommitAction::BankHintWrite,
                            staged.staged.previous,
                            version,
                            AuditOrigin::Submitted { request },
                            now,
                        ),
                        audit,
                    );
                }
                self.finish(
                    key,
                    queued,
                    Settlement::Completed(DurableReceipt::new(
                        identity,
                        version,
                        now,
                        Some(request),
                    )),
                    true,
                );
                None
            }
        }
    }

    /// Take the hint another writer left, which a save found instead of the
    /// one the service held, into the store.
    fn adopt_observed(&self, key: &StoreKey, observed: Option<BankDurable>) {
        let Some(observed) = observed else {
            return;
        };
        let read = if observed.digest.is_some() {
            BankRead::Hint(observed)
        } else {
            BankRead::Absent
        };
        self.state.lock().adopt_bank(key, read);
    }
}

/// The ledger source key of every source the bank hints ever had: a
/// recovery that could not read the old database records it, so no legacy
/// hint file is offered for an automatic import again (a source whose
/// disposition is unknown is never imported).
const UNKNOWN_SOURCES: &str = "*";

/// Non-preference data from the preserved database, bounded by its page cap.
pub(super) struct StateCarry {
    ledger: Vec<CarriedLedger>,
    hints: Vec<CarriedHint>,
    script: Option<ScriptCarry>,
}

struct ScriptCarry {
    sequence: StoreVersion,
    operator_head: Option<(Vec<u8>, i64, i64)>,
    operator_cards: Vec<(String, Vec<u8>)>,
    runtime: Vec<CarriedRuntime>,
}

struct CarriedRuntime {
    key: BankKey,
    version: Vec<u8>,
    body: Vec<u8>,
    digest: Vec<u8>,
    omitted: i64,
}

impl ScriptCarry {
    fn read(conn: &Connection) -> Result<Self, SqlFailure> {
        let sequence = super::sql::read_sequence(conn)?;
        let operator_head = conn
            .query_row(
                "SELECT version, encoded_bytes, card_count FROM operator_script_head WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| refused_by(error, "reading the operator settings head"))?;
        let operator_cards = conn
            .prepare("SELECT card_key, bag FROM operator_script_card")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .and_then(Iterator::collect)
            })
            .map_err(|error| refused_by(error, "reading the operator settings cards"))?;
        let runtime = conn
            .prepare(
                "SELECT profile, account, revision, content, version, body, digest, omitted
                 FROM script_runtime_settings",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok(CarriedRuntime {
                            key: BankKey {
                                profile: row.get(0)?,
                                account: row.get(1)?,
                                revision: row.get(2)?,
                                content: row.get(3)?,
                            },
                            version: row.get(4)?,
                            body: row.get(5)?,
                            digest: row.get(6)?,
                            omitted: row.get(7)?,
                        })
                    })
                    .and_then(Iterator::collect)
            })
            .map_err(|error| refused_by(error, "reading the script runtime settings"))?;
        Ok(Self {
            sequence,
            operator_head,
            operator_cards,
            runtime,
        })
    }

    fn restore(
        &self,
        conn: &mut Connection,
        root: &StateRoot,
        guard: &files::WriteLock,
    ) -> Result<(), SqlFailure> {
        let what = "restoring the script settings";
        if let Some((version, bytes, count)) = &self.operator_head {
            let tx = begin_write(conn, root, guard, what)?;
            tx.execute(
                "INSERT INTO operator_script_head (id, version, encoded_bytes, card_count)
                 VALUES (1, ?1, ?2, ?3)",
                params![version, bytes, count],
            )
            .map_err(|error| refused_by(error, what))?;
            {
                let mut insert = tx
                    .prepare(
                        "INSERT INTO operator_script_card (card_key, bag)
                                             VALUES (?1, ?2)",
                    )
                    .map_err(|error| refused_by(error, what))?;
                for (key, bag) in &self.operator_cards {
                    insert
                        .execute(params![key, bag])
                        .map_err(|error| refused_by(error, what))?;
                }
            }
            tx.commit().map_err(|error| refused_by(error, what))?;
        }
        for row in &self.runtime {
            let tx = begin_write(conn, root, guard, what)?;
            tx.execute(
                "INSERT INTO script_runtime_settings
                 (profile, account, revision, content, version, body, digest, omitted)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    row.key.profile,
                    row.key.account,
                    row.key.revision,
                    row.key.content,
                    row.version,
                    row.body,
                    row.digest,
                    row.omitted
                ],
            )
            .map_err(|error| refused_by(error, what))?;
            tx.commit().map_err(|error| refused_by(error, what))?;
        }
        Ok(())
    }
}

struct CarriedLedger {
    domain: String,
    source: String,
    digest: Option<Vec<u8>>,
    kind: String,
    evidence: String,
    path: String,
}

struct CarriedHint {
    key: BankKey,
    observed_at: i64,
    rows: Vec<u8>,
    omitted: i64,
}

impl StateCarry {
    /// How many bank hints are carried.
    pub(super) fn hints(&self) -> usize {
        self.hints.len()
    }

    pub(super) fn runtime_stores(&self) -> usize {
        self.script
            .as_ref()
            .map_or(0, |script| script.runtime.len())
    }

    pub(super) fn operator_cards(&self) -> usize {
        self.script
            .as_ref()
            .map_or(0, |script| script.operator_cards.len())
    }
}

/// The hints one restoring transaction writes: at most 256 hints, so one
/// transaction stays within the pages the WAL admission reserves for it.
const CARRY_BATCH: usize = 256;

impl Db {
    /// Read other domains under exclusive recovery ownership, before
    /// preserving the old database. No live connection is open here.
    pub(super) fn read_state_carry(&mut self) -> Result<Option<StateCarry>, SqlFailure> {
        if std::fs::symlink_metadata(self.root.database_file()).is_err() {
            return Ok(None);
        }
        let conn = open_connection(&self.root)?;
        let schema: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| refused_by(error, "reading the schema version"))?;
        if schema < 2 {
            return Ok(None);
        }
        let ledger = conn
            .prepare(
                "SELECT domain, source, digest, kind, evidence, path FROM import_ledger
                 WHERE domain <> 'preferences'",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok(CarriedLedger {
                            domain: row.get(0)?,
                            source: row.get(1)?,
                            digest: row.get(2)?,
                            kind: row.get(3)?,
                            evidence: row.get(4)?,
                            path: row.get(5)?,
                        })
                    })
                    .and_then(Iterator::collect)
            })
            .map_err(|error| refused_by(error, "reading the import ledger"))?;
        let hints = conn
            .prepare(
                "SELECT profile, account, revision, content, observed_at, rows, omitted
                 FROM bank_hint ORDER BY id",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok(CarriedHint {
                            key: BankKey {
                                profile: row.get(0)?,
                                account: row.get(1)?,
                                revision: row.get(2)?,
                                content: row.get(3)?,
                            },
                            observed_at: row.get(4)?,
                            rows: row.get(5)?,
                            omitted: row.get(6)?,
                        })
                    })
                    .and_then(Iterator::collect)
            })
            .map_err(|error| refused_by(error, "reading the bank hints"))?;
        let script = (schema >= 3)
            .then(|| ScriptCarry::read(&conn))
            .transpose()?;
        Ok(Some(StateCarry {
            ledger,
            hints,
            script,
        }))
    }

    /// Carry the script settings, ledger and bank hints into the new history.
    /// Bank hints take new versions; script settings keep theirs, and the
    /// sequence stays at least as high as the preserved versions.
    pub(super) fn restore_state_carry(
        &mut self,
        guard: &files::WriteLock,
        carry: Option<&StateCarry>,
    ) -> Result<(), SqlFailure> {
        let Some(carry) = carry else {
            return Ok(());
        };
        if let Some(script) = &carry.script {
            let conn = self.conn.as_mut().ok_or_else(closed)?;
            let tx = begin_write(conn, &self.root, guard, "preserving the version sequence")?;
            let sequence = super::sql::read_sequence(&tx)?.max(script.sequence);
            tx.execute(
                "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                [be(sequence.get())],
            )
            .map_err(|error| refused_by(error, "preserving the version sequence"))?;
            tx.commit()
                .map_err(|error| refused_by(error, "preserving the version sequence"))?;
            script.restore(conn, &self.root, guard)?;
        }
        let mut ledger = carry.ledger.iter();
        let mut hints = carry.hints.chunks(CARRY_BATCH);
        loop {
            let conn = self.conn.as_mut().ok_or_else(closed)?;
            let tx = begin_write(conn, &self.root, guard, "restoring the bank hints")?;
            let batch = hints.next();
            let mut wrote = false;
            for row in ledger.by_ref() {
                tx.execute(
                    "INSERT OR IGNORE INTO import_ledger
                         (domain, source, digest, kind, evidence, path)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        row.domain,
                        row.source,
                        row.digest,
                        row.kind,
                        row.evidence,
                        row.path
                    ],
                )
                .map_err(|error| refused_by(error, "restoring the import ledger"))?;
                wrote = true;
            }
            for hint in batch.into_iter().flatten() {
                let version = next_version(&tx, None)?;
                tx.execute(
                    "INSERT INTO bank_hint
                         (profile, account, revision, content, version, observed_at, rows, omitted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        hint.key.profile,
                        hint.key.account,
                        hint.key.revision,
                        hint.key.content,
                        be(version.get()),
                        hint.observed_at,
                        hint.rows,
                        hint.omitted
                    ],
                )
                .and_then(|_| {
                    tx.execute(
                        "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                        params![be(version.get())],
                    )
                })
                .map_err(|error| refused_by(error, "restoring the bank hints"))?;
                wrote = true;
            }
            tx.commit().map_err(|error| match wait_of(&error) {
                Some(cause) => SqlFailure::Wait(cause),
                None => refused_by(error, "committing the restored bank hints"),
            })?;
            if !wrote {
                return Ok(());
            }
        }
    }

    /// An unreadable old database cannot grant its sources new authority.
    pub(super) fn mark_state_sources_unknown(
        &mut self,
        guard: &files::WriteLock,
    ) -> Result<(), SqlFailure> {
        let conn = self.conn.as_mut().ok_or_else(closed)?;
        let tx = begin_write(
            conn,
            &self.root,
            guard,
            "recording the unknown legacy sources",
        )?;
        for domain in [LEDGER_DOMAIN, "script-settings", "script-runtime-settings"] {
            tx.execute(
                "INSERT OR IGNORE INTO import_ledger (domain, source, digest, kind, evidence, path)
                 VALUES (?1, ?2, NULL, 'unknown-after-recovery', 'none', '')",
                params![domain, UNKNOWN_SOURCES],
            )
            .map_err(|error| refused_by(error, "recording the unknown legacy sources"))?;
        }
        tx.commit().map_err(|error| match wait_of(&error) {
            Some(cause) => SqlFailure::Wait(cause),
            None => refused_by(error, "committing the unknown legacy sources"),
        })
    }
}

/// One bank hint as the state database holds it, for live-cell evidence and
/// seeding (test builds only).
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBankHint {
    pub observed_at_unix: u64,
    pub rows: Vec<(i32, i32)>,
}

#[cfg(any(test, feature = "test-support"))]
impl StateRoot {
    /// The most recently saved hint of `account` under `profile`, whatever
    /// content it was observed under, read through a connection of its own.
    /// `Ok(None)` while the database or the hint does not exist.
    pub fn stored_bank_hint(
        &self,
        profile: &str,
        account: &str,
    ) -> Result<Option<StoredBankHint>, String> {
        let Some(conn) = self.test_connection(rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)? else {
            return Ok(None);
        };
        let row = conn
            .query_row(
                "SELECT observed_at, rows FROM bank_hint
                 WHERE profile = ?1 AND account = ?2 ORDER BY version DESC LIMIT 1",
                params![profile, account],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(|error| format!("reading the stored hint: {error}"))?;
        row.map(|(observed_at, raw)| {
            Ok(StoredBankHint {
                observed_at_unix: u64::try_from(observed_at)
                    .map_err(|_| "a negative observation time".to_string())?,
                rows: decode_rows(&raw).ok_or_else(|| "rows that are not whole".to_string())?,
            })
        })
        .transpose()
    }

    /// Replace the rows of the stored hints of `account` under `profile`
    /// while no service runs on this root, as a live cell seeds a stale
    /// hint between two processes. The version and the time stay.
    pub fn replace_stored_bank_hint_rows(
        &self,
        profile: &str,
        account: &str,
        rows: &[(i32, i32)],
    ) -> Result<(), String> {
        let Some(conn) = self.test_connection(rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return Err("there is no state database to change".to_string());
        };
        let changed = conn
            .execute(
                "UPDATE bank_hint SET rows = ?3 WHERE profile = ?1 AND account = ?2",
                params![profile, account, encode_rows(rows)],
            )
            .map_err(|error| format!("replacing the stored rows: {error}"))?;
        if changed == 0 {
            return Err(format!("no stored bank hint of {profile}/{account}"));
        }
        Ok(())
    }

    /// Delete the stored hints of `account` under `profile` while no service
    /// runs on this root, as the cold live cell starts from no hint.
    pub fn remove_stored_bank_hint(&self, profile: &str, account: &str) -> Result<(), String> {
        let Some(conn) = self.test_connection(rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return Ok(());
        };
        conn.execute(
            "DELETE FROM bank_hint WHERE profile = ?1 AND account = ?2",
            params![profile, account],
        )
        .map_err(|error| format!("removing the stored hint: {error}"))?;
        Ok(())
    }

    fn test_connection(&self, flags: rusqlite::OpenFlags) -> Result<Option<Connection>, String> {
        match std::fs::symlink_metadata(self.database_file()) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("the state database: {error}")),
        }
        let conn = Connection::open_with_flags(self.database_file(), flags)
            .map_err(|error| format!("opening the state database: {error}"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("the state database: {error}"))?;
        Ok(Some(conn))
    }
}
