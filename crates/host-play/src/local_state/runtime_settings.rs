//! Script runtime settings live in the shared state database. The schema-1 JSON
//! envelope is the bounded storage encoding; each `raw` inside it remains text.

use super::{
    files::{
        self, ContentDigest, ReadDocument, Rewrite, StoreKey, MAX_SCRIPT_RUNTIME_SETTINGS_BYTES,
    },
    sql::{self, Commit, Db, Evidence, Ledger, Loaded, SqlFailure, Staging, WaitCause},
    AuditKind, Cached, FileState, Queued, Rejection, Shared, State, WriteIntent,
};
use api::hostlog::Level;
use api::identity::{Cancellation, DurableTimestamp, Settlement};
use api::local_state::{
    AuditCommit, AuditOrigin, CommitAction, DomainEdit, DurableReceipt, ErrorCode, FailureReason,
    LeaseSnapshot, LocalStateDomain, RuntimeSettingsEdit, StoreVersion, ValidationReason,
};
use rusqlite::{params, Connection, TransactionBehavior};
use std::io;
use std::time::Instant;

const DOMAIN: LocalStateDomain = LocalStateDomain::ScriptRuntimeSettings;

pub(super) const SCHEMA: &str = "
    CREATE TABLE script_runtime_settings (
        profile TEXT NOT NULL,
        account TEXT NOT NULL,
        revision INTEGER NOT NULL,
        content TEXT NOT NULL,
        version BLOB NOT NULL CHECK (length(version) = 8),
        body BLOB NOT NULL CHECK (length(body) <= 1048576),
        digest BLOB NOT NULL CHECK (length(digest) = 32),
        omitted INTEGER NOT NULL CHECK (omitted >= 0),
        PRIMARY KEY (profile, account, revision, content)
    ) STRICT, WITHOUT ROWID;";

#[derive(Clone, Copy)]
struct Scope<'a> {
    profile: &'a str,
    account: &'a str,
    revision: i32,
    content: &'a str,
}

impl<'a> Scope<'a> {
    fn of(key: &'a StoreKey) -> Self {
        let StoreKey::ScriptRuntimeSettings {
            profile,
            account,
            revision,
            content,
        } = key
        else {
            unreachable!("runtime settings scope");
        };
        Self {
            profile,
            account,
            revision: *revision,
            content,
        }
    }

    fn source(self) -> String {
        format!(
            "{}/{}/{}-{}",
            self.profile, self.account, self.revision, self.content
        )
    }
}

fn empty() -> Cached {
    Cached {
        version: StoreVersion::INITIAL,
        snapshot: LeaseSnapshot::script_runtime_settings(Vec::new()),
        at: DurableTimestamp::now(),
        preferences: None,
        refreshed_at: Instant::now(),
        digest: None,
        durable: false,
        written_len: 0,
        rewrite: Rewrite::Lossless,
    }
}

fn omitted(rewrite: Rewrite) -> usize {
    match rewrite {
        Rewrite::Lossless => 0,
        Rewrite::Refused { unreadable, lossy } => unreadable + lossy,
    }
}

fn cached(document: ReadDocument, version: StoreVersion) -> Cached {
    Cached {
        version,
        at: document.at,
        snapshot: document.snapshot,
        preferences: None,
        refreshed_at: Instant::now(),
        digest: Some(document.digest),
        durable: true,
        written_len: document.written_len,
        rewrite: document.rewrite,
    }
}

struct Body<'a> {
    version: StoreVersion,
    bytes: &'a [u8],
    digest: ContentDigest,
    omitted: usize,
}

fn write_row(conn: &Connection, scope: Scope<'_>, body: Body<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO script_runtime_settings
             (profile, account, revision, content, version, body, digest, omitted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (profile, account, revision, content) DO UPDATE SET
             version = excluded.version, body = excluded.body,
             digest = excluded.digest, omitted = excluded.omitted",
        params![
            scope.profile,
            scope.account,
            scope.revision,
            scope.content,
            sql::be(body.version.get()),
            body.bytes,
            body.digest.as_slice(),
            body.omitted as i64
        ],
    )?;
    Ok(())
}

fn db_error(error: rusqlite::Error, what: &str) -> SqlFailure {
    sql::wait_of(&error).map_or_else(|| sql::refused_by(error, what), SqlFailure::Wait)
}

fn load(
    conn: &Connection,
    key: &StoreKey,
    known: Option<&Cached>,
) -> Result<Loaded<Cached>, SqlFailure> {
    let scope = Scope::of(key);
    let map_err = |error| db_error(error, "reading a script runtime settings scope");
    let mut query = conn
        .prepare(
            "SELECT version, digest, omitted,
                CASE WHEN version = ?5 AND digest = ?6 THEN NULL ELSE body END
         FROM script_runtime_settings
         WHERE profile = ?1 AND account = ?2 AND revision = ?3 AND content = ?4",
        )
        .map_err(map_err)?;
    let mut rows = query
        .query(params![
            scope.profile,
            scope.account,
            scope.revision,
            scope.content,
            known.map(|cached| sql::be(cached.version.get())),
            known.and_then(|cached| cached.digest.as_ref().map(|digest| digest.as_slice())),
        ])
        .map_err(map_err)?;
    let Some(row) = rows.next().map_err(map_err)? else {
        return Ok(Loaded {
            head: StoreVersion::INITIAL,
            observed: known
                .is_none_or(|cached| cached.digest.is_some())
                .then(empty),
        });
    };
    let damaged = || {
        SqlFailure::refused(
            FailureReason::Checksum,
            "damaged script runtime settings row",
        )
    };
    let version = row
        .get_ref(0)
        .map_err(map_err)?
        .as_blob()
        .map_err(|_| damaged())?;
    let head = StoreVersion::from_durable(sql::decode_u64(version).ok_or_else(damaged)?);
    let digest = row
        .get_ref(1)
        .map_err(map_err)?
        .as_blob()
        .map_err(|_| damaged())?;
    let stored_omitted: i64 = row.get(2).map_err(map_err)?;
    let observed = match row.get_ref(3).map_err(map_err)? {
        rusqlite::types::ValueRef::Null => None,
        body => {
            let bytes = body.as_blob().map_err(|_| damaged())?;
            if bytes.len() as u64 > MAX_SCRIPT_RUNTIME_SETTINGS_BYTES {
                return Err(damaged());
            }
            let raw = std::str::from_utf8(bytes).map_err(|_| damaged())?;
            let document = files::decode_runtime_settings(key, raw, DurableTimestamp::now())
                .map_err(|_| damaged())?;
            if document.digest.as_slice() != digest
                || omitted(document.rewrite) as i64 != stored_omitted
            {
                return Err(damaged());
            }
            Some(cached(document, head))
        }
    };
    Ok(Loaded { head, observed })
}

impl Db {
    fn read_runtime_settings(
        &mut self,
        key: &StoreKey,
        known: Option<&Cached>,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Option<Cached>, SqlFailure> {
        self.open_for_domain()?;
        let mut result = self.read_runtime_open(key, known, clock, notices);
        self.after_failure(&mut result);
        result
    }

    fn read_runtime_open(
        &mut self,
        key: &StoreKey,
        known: Option<&Cached>,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Option<Cached>, SqlFailure> {
        let scope = Scope::of(key);
        let mut evidence = Evidence {
            source: self.root.script_runtime_settings_file(
                scope.profile,
                scope.account,
                scope.revision,
                scope.content,
            ),
            digest: None,
            domain: DOMAIN.code(),
            ledger_source: scope.source(),
            what: "script runtime settings",
        };
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| db_error(error, "starting a script runtime settings read"))?;
        let loaded = load(&tx, key, known)?;
        let ledger = sql::read_ledger_of(&tx, evidence.domain, &evidence.ledger_source)?;
        if ledger.is_some()
            || loaded.head != StoreVersion::INITIAL
            || sql::read_ledger_of(&tx, evidence.domain, "*")?.is_some()
        {
            tx.commit()
                .map_err(|error| db_error(error, "ending a script runtime settings read"))?;
            if let Some(ledger) = ledger {
                self.settle_plain_import(&evidence, ledger, notices);
            }
            return Ok(loaded.observed);
        }
        drop(tx);
        let guard = self.guard()?;
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = sql::begin_write(
            conn,
            &self.root,
            &guard,
            "starting a script runtime settings import",
        )?;
        if sql::read_ledger_of(&tx, evidence.domain, &evidence.ledger_source)?.is_some()
            || sql::read_ledger_of(&tx, evidence.domain, "*")?.is_some()
        {
            // A competing importer committed before this guard was taken.
            return Err(SqlFailure::Wait(WaitCause::Busy));
        }
        let imported =
            match vault::read_private_file(&evidence.source, MAX_SCRIPT_RUNTIME_SETTINGS_BYTES) {
                Ok(raw) => {
                    let document =
                        files::decode_runtime_settings(key, &raw, DurableTimestamp::now())
                            .map_err(|error| {
                                SqlFailure::refused(
                                    error.reason(),
                                    format!(
                                        "Script runtime settings refused at {}: {error}",
                                        evidence.source.display()
                                    ),
                                )
                            })?;
                    let version = sql::next_version(&tx, clock)?;
                    evidence.digest = Some(document.digest);
                    write_row(
                        &tx,
                        scope,
                        Body {
                            version,
                            bytes: raw.as_bytes(),
                            digest: document.digest,
                            omitted: omitted(document.rewrite),
                        },
                    )
                    .map_err(|error| db_error(error, "importing script runtime settings"))?;
                    tx.execute(
                        "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
                        [sql::be(version.get())],
                    )
                    .map_err(|error| {
                        db_error(error, "advancing the script runtime settings version")
                    })?;
                    cached(document, version)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => empty(),
                Err(error) => {
                    return Err(SqlFailure::refused(
                        FailureReason::Io,
                        format!(
                            "Script runtime settings refused at {}: {error}",
                            evidence.source.display()
                        ),
                    ))
                }
            };
        sql::insert_import(&tx, &evidence)?;
        tx.commit()
            .map_err(|error| db_error(error, "committing the script runtime settings import"))?;
        drop(guard);
        if evidence.digest.is_some() {
            notices.push((
                Level::Info,
                format!(
                    "local state: script-runtime-settings imported version={} source={}",
                    imported.version,
                    evidence.source.display()
                ),
            ));
            self.settle_plain_import(
                &evidence,
                Ledger {
                    digest: evidence.digest,
                    evidence: "pending".into(),
                },
                notices,
            );
        }
        Ok(Some(imported))
    }
}

struct Change {
    bytes: Vec<u8>,
    snapshot: LeaseSnapshot,
    digest: ContentDigest,
}

struct Staged<'a> {
    scope: Scope<'a>,
    previous: StoreVersion,
    change: Option<Change>,
}

impl Staging for Staged<'_> {
    type Durable = Cached;
    const DOMAIN: LocalStateDomain = DOMAIN;
    const HEAD: &'static str = "script-runtime-settings";
    const ACTION: &'static str = "script-runtime-settings-write";

    fn unchanged(&self) -> bool {
        self.change.is_none()
    }
    fn digest(&self) -> ContentDigest {
        self.change.as_ref().expect("changed save").digest
    }
    fn set_version(&mut self, _: StoreVersion) {}

    fn write(&self, tx: &Connection, version: StoreVersion) -> rusqlite::Result<()> {
        let change = self.change.as_ref().expect("changed save");
        write_row(
            tx,
            self.scope,
            Body {
                version,
                bytes: &change.bytes,
                digest: change.digest,
                omitted: 0,
            },
        )
    }

    fn durable(&self, version: StoreVersion, digest: ContentDigest) -> Cached {
        let change = self.change.as_ref().expect("changed save");
        Cached {
            version,
            at: DurableTimestamp::now(),
            snapshot: change.snapshot.clone(),
            preferences: None,
            refreshed_at: Instant::now(),
            digest: Some(digest),
            durable: true,
            written_len: change.bytes.len(),
            rewrite: Rewrite::Lossless,
        }
    }
}

fn stage<'a>(
    key: &'a StoreKey,
    current: &Cached,
    edit: &RuntimeSettingsEdit,
    external: bool,
    accepted_at: StoreVersion,
) -> Result<Staged<'a>, ErrorCode> {
    if external || !edit.applies_to(current.version) {
        return Err(ErrorCode::Conflict {
            expected: edit.expected().unwrap_or(accepted_at),
            found: current.version,
        });
    }
    if matches!(current.rewrite, Rewrite::Refused { .. }) {
        return Err(ErrorCode::Failed {
            domain: DOMAIN,
            reason: FailureReason::Schema,
        });
    }
    let rows = current
        .snapshot
        .as_script_runtime_settings()
        .expect("runtime settings lease");
    let change = if files::runtime_setting_index(rows, edit.namespace(), edit.key())
        .is_ok_and(|index| rows[index].raw() == edit.raw())
    {
        None
    } else {
        let rows = files::apply_script_runtime_setting(rows, edit)?;
        let bytes =
            files::script_runtime_settings_bytes(key, &rows).map_err(|_| ErrorCode::Failed {
                domain: DOMAIN,
                reason: FailureReason::Schema,
            })?;
        if bytes.len() as u64 > MAX_SCRIPT_RUNTIME_SETTINGS_BYTES {
            return Err(ErrorCode::Validation {
                domain: DOMAIN,
                reason: ValidationReason::TooLong,
            });
        }
        Some(Change {
            digest: files::digest(&bytes),
            bytes,
            snapshot: LeaseSnapshot::script_runtime_settings(rows),
        })
    };
    Ok(Staged {
        scope: Scope::of(key),
        previous: current.version,
        change,
    })
}

impl State {
    fn adopt_runtime(&mut self, key: &StoreKey, observed: Option<Cached>) {
        if let Some(cached) = observed {
            self.observe_version(cached.version);
            if let Some(store) = self.stores.get_mut(key) {
                if matches!(&store.file, FileState::Read(known) if known.version != cached.version || known.digest != cached.digest)
                {
                    store.external += 1;
                }
                store.file = FileState::Read(cached);
            }
        } else if let Some(FileState::Read(cached)) =
            self.stores.get_mut(key).map(|store| &mut store.file)
        {
            cached.refreshed_at = Instant::now();
        }
    }

    fn reject_runtime(&mut self, key: &StoreKey, rejection: Rejection) {
        if let Some(store) = self.stores.get_mut(key) {
            if matches!(store.file, FileState::Read(_)) {
                store.external += 1;
            }
            store.file = FileState::Rejected(rejection);
        }
    }
}

impl Shared {
    pub(super) fn readback_runtime(&self, db: &mut Db, key: &StoreKey) -> Option<WaitCause> {
        self.stall_read();
        let (known, clock) = {
            let state = self.state.lock();
            let known = state.stores.get(key).and_then(|store| match &store.file {
                FileState::Read(cached) => Some(cached.clone()),
                _ => None,
            });
            (known, state.clock)
        };
        let mut notices = Vec::new();
        let result = db.read_runtime_settings(key, known.as_ref(), clock, &mut notices);
        let truncate_due = matches!(&result, Ok(Some(_))) && db.wal_truncate_due();
        for (level, line) in notices {
            self.notice(level, format_args!("{line}"));
        }
        let mut state = self.state.lock();
        state.truncate.due |= truncate_due;
        match result {
            Ok(observed) => state.adopt_runtime(key, observed),
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { reason, detail }) => {
                state.reject_runtime(key, Rejection { reason, detail });
            }
        }
        state.readback_done(key);
        None
    }

    pub(super) fn write_runtime(
        &self,
        db: &mut Db,
        key: &StoreKey,
        queued: &Queued,
    ) -> Option<WaitCause> {
        let WriteIntent::Port(DomainEdit::ScriptRuntimeSettings(edit)) = &queued.intent else {
            unreachable!("runtime settings write");
        };
        let (known, external, clock) = {
            let state = self.state.lock();
            let store = state
                .stores
                .get(key)
                .expect("queued runtime settings scope");
            match &store.file {
                FileState::Read(known) => (Ok(known.clone()), store.external, state.clock),
                FileState::Rejected(rejection) => {
                    (Err(rejection.reason), store.external, state.clock)
                }
                FileState::Cold => unreachable!("the writer hydrates before taking a write"),
            }
        };
        let known = match known {
            Ok(known) => known,
            Err(reason) => {
                self.finish(
                    key,
                    queued,
                    Settlement::Failed(ErrorCode::Failed {
                        domain: DOMAIN,
                        reason,
                    }),
                    false,
                );
                return None;
            }
        };
        let request = queued.request;
        let committed = db.open_for_domain().and_then(|()| {
            db.commit(
                request,
                clock,
                Some(&known),
                || self.begin_sql_write(key),
                |conn| load(conn, key, Some(&known)),
                |current| {
                    stage(
                        key,
                        current,
                        edit,
                        external != queued.external
                            || current.version != known.version
                            || current.digest != known.digest,
                        queued.accepted_at,
                    )
                },
            )
        });
        let (settlement, changed) = match committed {
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { reason, detail }) => {
                self.state
                    .lock()
                    .reject_runtime(key, Rejection { reason, detail });
                (
                    Settlement::Failed(ErrorCode::Failed {
                        domain: DOMAIN,
                        reason,
                    }),
                    false,
                )
            }
            Ok(Commit::Declined { code, observed, .. }) => {
                self.state.lock().adopt_runtime(key, observed);
                (Settlement::Failed(code), false)
            }
            Ok(Commit::Unresolved { detail, observed }) => {
                self.state.lock().adopt_runtime(key, observed);
                self.finish_unresolved(key, queued, super::io_failure(DOMAIN), detail);
                return None;
            }
            Ok(Commit::Abandoned) => (Settlement::Cancelled(Cancellation::Shutdown), false),
            Ok(Commit::Unchanged { staged, observed }) => {
                self.state.lock().adopt_runtime(key, observed);
                (
                    Settlement::Completed(DurableReceipt::new(
                        edit.identity().clone(),
                        staged.previous,
                        DurableTimestamp::now(),
                        Some(request),
                    )),
                    false,
                )
            }
            Ok(Commit::Committed {
                staged,
                durable,
                truncate_due,
                ..
            }) => {
                let version = durable.version;
                let now = durable.at;
                {
                    let mut state = self.state.lock();
                    state.observe_version(version);
                    state.truncate.due |= truncate_due;
                    if let Some(store) = state.stores.get_mut(key) {
                        store.file = FileState::Read(durable);
                    }
                }
                let identity = edit.identity().clone();
                self.audit(
                    AuditCommit::after_commit(
                        identity.clone(),
                        CommitAction::ScriptRuntimeSettingsWrite,
                        staged.previous,
                        version,
                        AuditOrigin::Submitted { request },
                        now,
                    ),
                    &AuditKind::Plain,
                );
                (
                    Settlement::Completed(DurableReceipt::new(
                        identity,
                        version,
                        now,
                        Some(request),
                    )),
                    true,
                )
            }
        };
        self.finish(key, queued, settlement, changed);
        None
    }
}
