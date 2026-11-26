//! Root-wide operator defaults. Profile claims remain in the vault until S8.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use script::ScriptSource;
use serde_json::{Map, Value};

use super::sql::{self, Commit, Db, Evidence, Ledger, Loaded, SqlFailure, Staging, WaitCause};
use super::*;

const DOMAIN: LocalStateDomain = LocalStateDomain::OperatorScriptSettings;
const SOURCE: &str = "script-settings.json";
pub(super) const MAX_BYTES: usize = 1024 * 1024;
pub(super) const MAX_CARDS: usize = 8192;

pub(super) const SCHEMA: &str = "
CREATE TABLE operator_script_head (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version BLOB NOT NULL CHECK (length(version) = 8),
    encoded_bytes INTEGER NOT NULL CHECK (encoded_bytes BETWEEN 2 AND 1048576),
    card_count INTEGER NOT NULL CHECK (card_count BETWEEN 0 AND 8192)
) STRICT;
CREATE TABLE operator_script_card (
    card_key TEXT PRIMARY KEY,
    bag BLOB NOT NULL CHECK (length(bag) BETWEEN 2 AND 1048576)
) STRICT, WITHOUT ROWID;
";

type Bag = Map<String, Value>;
type Cards = BTreeMap<String, Arc<Bag>>;

/// A typed failure while loading or writing root-wide operator defaults.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ScriptSettingsError {
    /// The legacy source exists but could not be read.
    #[error("script settings at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
    /// The legacy source is not a strict map of bags.
    #[error("script settings at {path}: invalid document: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: Arc<serde_json::Error>,
    },
    /// The state service cannot load or save these defaults.
    #[error("script settings at {path}: {detail}")]
    State { path: PathBuf, detail: Arc<str> },
}

impl ScriptSettingsError {
    /// The legacy source path associated with these settings.
    pub fn path(&self) -> &Path {
        match self {
            Self::Read { path, .. } | Self::Decode { path, .. } | Self::State { path, .. } => path,
        }
    }
}

#[derive(Default)]
pub(super) struct OperatorState {
    snapshot: Option<Snapshot>,
    refusal: Option<ScriptSettingsError>,
    read_queued: bool,
    read_retry: Retry,
    queue: VecDeque<Edit>,
    queued_bytes: usize,
    active: bool,
    notice: Option<String>,
}

struct Snapshot {
    version: StoreVersion,
    cards: Cards,
}

/// A cache-only view of the operator's per-card defaults and a write capability.
#[derive(Clone)]
pub struct OperatorScriptSettings {
    service: Arc<LocalStateService>,
}

impl LocalStateService {
    /// Access operator defaults on this service's existing SQLite connection.
    pub fn operator_script_settings(self: &Arc<Self>) -> OperatorScriptSettings {
        OperatorScriptSettings {
            service: Arc::clone(self),
        }
    }
}

impl OperatorScriptSettings {
    /// Request first hydration and report whether its durable snapshot is ready.
    pub fn readiness(&self) -> Result<bool, ScriptSettingsError> {
        let shared = &self.service.shared;
        let mut state = shared.state.lock();
        self.request(&mut state)
    }

    fn request(&self, state: &mut State) -> Result<bool, ScriptSettingsError> {
        let shared = &self.service.shared;
        if !self.service.is_open(DOMAIN) {
            return Err(state_error(
                &shared.root,
                "operator script settings are closed",
            ));
        }
        if let Some(error) = &state.operator_settings.refusal {
            return Err(error.clone());
        }
        if state.operator_settings.snapshot.is_some() {
            return Ok(true);
        }
        if !state.operator_settings.read_queued {
            if state.shutdown || !shared.start_writer(state) {
                return Err(state_error(&shared.root, "the state service has stopped"));
            }
            state.operator_settings.read_queued = true;
            state.operator_settings.read_retry.first = Some(Instant::now());
            shared.wake.notify_one();
        }
        Ok(false)
    }

    /// Wait for first hydration on a startup or control thread, not a frame or slot.
    pub fn wait_ready(&self, timeout: Duration) -> Result<(), ScriptSettingsError> {
        let shared = &self.service.shared;
        let deadline = Instant::now() + timeout;
        let mut state = shared.state.lock();
        while !self.request(&mut state)? {
            if shared.settled.wait_until(&mut state, deadline).timed_out() {
                return if self.request(&mut state)? {
                    Ok(())
                } else {
                    Err(state_error(
                        &shared.root,
                        "operator defaults are still loading",
                    ))
                };
            }
        }
        Ok(())
    }

    /// Get a card's saved bag. An empty bag is present, not absent.
    pub fn card(&self, source: ScriptSource, name: &str) -> Option<Arc<Bag>> {
        let key = script::card_key(source, name);
        self.service
            .shared
            .state
            .lock()
            .operator_settings
            .snapshot
            .as_ref()?
            .cards
            .get(&key)
            .cloned()
    }

    /// Copy one card's overrides, or return an empty bag when it has none.
    pub fn overrides(&self, source: ScriptSource, name: &str) -> Bag {
        self.card(source, name)
            .map_or_else(Map::new, |bag| (*bag).clone())
    }

    /// Take the notice about a retired file, if the first read found one.
    pub fn take_notice(&self) -> Option<String> {
        self.service
            .shared
            .state
            .lock()
            .operator_settings
            .notice
            .take()
    }

    /// Queue one field edit. The returned handle settles after the commit.
    pub fn set_value(
        &self,
        source: ScriptSource,
        name: &str,
        id: &str,
        value: Value,
    ) -> Result<OperatorScriptSettingsEdit, ScriptSettingsError> {
        let shared = &self.service.shared;
        let path = source_path(&shared.root);
        let card = script::card_key(source, name);
        let card_width = encoded_string_len(&card);
        let bytes = card_width
            .saturating_add(encoded_string_len(id))
            .saturating_add(encoded_size(&value));
        if bytes > MAX_BYTES {
            return Err(limit_error(&shared.root));
        }
        let mut state = shared.state.lock();
        if !self.request(&mut state)? {
            return Err(state_error(
                &shared.root,
                "operator defaults are still loading",
            ));
        }
        if state.shutdown
            || state.operator_settings.queue.len() >= OPERATOR_QUEUE_CAP
            || state.operator_settings.queued_bytes.saturating_add(bytes) > MAX_BYTES
            || state.preference_watches.len() + state.operator_settings.inflight_and_queued()
                > OPERATOR_QUEUE_CAP
        {
            return Err(state_error(&shared.root, "the settings queue is full"));
        }
        let request = shared
            .minter
            .mint()
            .map_err(|_| state_error(&shared.root, "the settings request ID space is spent"))?;
        let done = Arc::new(EditResult::default());
        state.operator_settings.queued_bytes += bytes;
        state.operator_settings.queue.push_back(Edit {
            card,
            card_width,
            key: id.to_owned(),
            value,
            request,
            bytes,
            done: Arc::clone(&done),
            retry: Retry::default(),
        });
        shared.wake.notify_one();
        Ok(OperatorScriptSettingsEdit { done, path })
    }
}

#[derive(Default)]
struct EditResult {
    result: Mutex<Option<Result<(), ScriptSettingsError>>>,
    ready: Condvar,
}

/// A receipt for one operator defaults edit.
pub struct OperatorScriptSettingsEdit {
    done: Arc<EditResult>,
    path: PathBuf,
}

impl OperatorScriptSettingsEdit {
    /// Read the terminal result, if the writer settled this edit.
    pub fn poll(&self) -> Option<Result<(), ScriptSettingsError>> {
        self.done.result.lock().clone()
    }

    /// Wait for settlement on a control thread, not a frame or slot.
    pub fn wait(&self, timeout: Duration) -> Result<(), ScriptSettingsError> {
        let deadline = Instant::now() + timeout;
        let mut result = self.done.result.lock();
        loop {
            if let Some(result) = &*result {
                return result.clone();
            }
            if self
                .done
                .ready
                .wait_until(&mut result, deadline)
                .timed_out()
                && result.is_none()
            {
                return Err(ScriptSettingsError::State {
                    path: self.path.clone(),
                    detail: Arc::from("the edit is still pending"),
                });
            }
        }
    }
}

pub(super) struct Edit {
    card: String,
    card_width: usize,
    key: String,
    value: Value,
    request: RequestId,
    bytes: usize,
    done: Arc<EditResult>,
    retry: Retry,
}

impl Edit {
    fn finish(&self, result: Result<(), ScriptSettingsError>) {
        *self.done.result.lock() = Some(result);
        self.done.ready.notify_all();
    }
}

pub(super) enum OperatorJob {
    Read,
    Edit(Edit),
}

impl OperatorState {
    pub(super) fn inflight_and_queued(&self) -> usize {
        self.queue.len() + usize::from(self.active)
    }

    pub(super) fn take_job(
        &mut self,
        now: Instant,
        stopping: bool,
    ) -> Result<OperatorJob, Option<Instant>> {
        if stopping && self.queue.is_empty() {
            return Err(None);
        }
        if self.read_queued {
            return if self.read_retry.due(now) {
                Ok(OperatorJob::Read)
            } else {
                Err(self.read_retry.at)
            };
        }
        let Some(edit) = self.queue.front_mut() else {
            return Err(None);
        };
        if !edit.retry.due(now) {
            return Err(edit.retry.at);
        }
        edit.retry.first.get_or_insert(now);
        self.active = true;
        Ok(OperatorJob::Edit(
            self.queue.pop_front().expect("the oldest edit is due"),
        ))
    }

    pub(super) fn requeue(
        &mut self,
        job: OperatorJob,
        root: &StateRoot,
        cause: WaitCause,
        now: Instant,
        abandoned: bool,
    ) {
        match job {
            OperatorJob::Read if !abandoned && !self.read_retry.expired(now) => {
                self.read_retry.delay(now)
            }
            OperatorJob::Read => self.reject(root, cause.text()),
            OperatorJob::Edit(mut edit) => {
                self.active = false;
                if abandoned || edit.retry.expired(now) {
                    self.queued_bytes -= edit.bytes;
                    edit.finish(Err(state_error(root, cause.text())));
                } else {
                    edit.retry.delay(now);
                    self.queue.push_front(edit);
                }
            }
        }
    }

    pub(super) fn reject(&mut self, root: &StateRoot, why: impl Into<Arc<str>>) {
        self.fail(state_error(root, why));
    }

    fn fail(&mut self, error: ScriptSettingsError) {
        self.snapshot = None;
        self.read_queued = false;
        while let Some(edit) = self.queue.pop_front() {
            self.queued_bytes -= edit.bytes;
            edit.finish(Err(error.clone()));
        }
        self.refusal = Some(error);
    }

    pub(super) fn contain(&mut self, job: &OperatorJob, root: &StateRoot, reason: &str) {
        if let OperatorJob::Edit(edit) = job {
            self.active = false;
            self.queued_bytes -= edit.bytes;
            edit.finish(Err(state_error(root, "the edit's result is unknown")));
        }
        self.reject(root, format!("the settings job stopped: {reason}"));
    }
}

impl Shared {
    pub(super) fn run_operator_job(&self, db: &mut Db, job: &OperatorJob) -> Option<WaitCause> {
        match job {
            OperatorJob::Read => self.hydrate_operator_settings(db),
            OperatorJob::Edit(edit) => self.write_operator_settings(db, edit),
        }
    }

    fn hydrate_operator_settings(&self, db: &mut Db) -> Option<WaitCause> {
        let clock = self.state.lock().clock;
        let mut notices = Vec::new();
        let result = match db.read_operator_settings(clock, &mut notices) {
            Ok(result) => result,
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { detail, .. }) => Err(state_error(&self.root, detail)),
        };
        let truncate_due = result.is_ok() && db.wal_truncate_due();
        let mut retired = None;
        for (level, notice) in notices {
            self.notice(level, format_args!("{notice}"));
            if level == Level::Warn && notice.starts_with("local state: retired ") {
                retired = Some(notice);
            }
        }
        if let Err(error) = &result {
            self.notice(
                Level::Warn,
                format_args!("{error}. Correct the source and restart."),
            );
        }
        let mut state = self.state.lock();
        state.truncate.due |= truncate_due;
        state.operator_settings.read_queued = false;
        state.operator_settings.read_retry = Retry::default();
        state.operator_settings.notice = retired;
        match result {
            Ok(snapshot) => {
                state.clock = state.clock.max(Some(snapshot.version));
                state.operator_settings.snapshot = Some(snapshot);
            }
            Err(error) => state.operator_settings.fail(error),
        }
        None
    }

    fn write_operator_settings(&self, db: &mut Db, edit: &Edit) -> Option<WaitCause> {
        let clock = self.state.lock().clock;
        let committed = db.commit(
            edit.request,
            clock,
            None,
            || !self.state.lock().abandoned,
            |conn| read_for_edit(conn, &edit.card),
            |current| stage(current, edit),
        );
        let (result, adopted, truncate_due) = match committed {
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { detail, .. }) => {
                (Err(state_error(&self.root, detail)), None, false)
            }
            Ok(Commit::Declined { code, detail, .. }) => {
                let error = detail.map_or_else(
                    || match code {
                        ErrorCode::Validation { .. } => limit_error(&self.root),
                        code => state_error(&self.root, code.to_string()),
                    },
                    |detail| state_error(&self.root, detail),
                );
                (Err(error), None, false)
            }
            Ok(Commit::Unresolved { detail, .. }) => {
                let mut state = self.state.lock();
                state.operator_settings.snapshot = None;
                state.operator_settings.read_queued = true;
                state.operator_settings.read_retry.first = Some(Instant::now());
                (Err(state_error(&self.root, detail)), None, false)
            }
            Ok(Commit::Abandoned) => (
                Err(state_error(
                    &self.root,
                    "the edit was cancelled at shutdown",
                )),
                None,
                false,
            ),
            Ok(Commit::Unchanged { staged, .. }) => {
                let previous = staged.previous;
                (Ok(()), Some((staged, previous)), false)
            }
            Ok(Commit::Committed {
                staged,
                durable,
                truncate_due,
                ..
            }) => {
                self.notice(Level::Info, format_args!(
                    "local state: script-settings parameter edit request={} card={} key={} {} -> {}",
                    edit.request.get(), edit.card, edit.key, staged.previous, durable.head.version,
                ));
                (Ok(()), Some((staged, durable.head.version)), truncate_due)
            }
        };
        let mut state = self.state.lock();
        if let Some((staged, version)) = adopted {
            let cache = &mut state.operator_settings;
            if let Some(snapshot) = cache
                .snapshot
                .as_mut()
                .filter(|snapshot| snapshot.version == staged.previous)
            {
                if !staged.unchanged() {
                    snapshot.cards.insert(edit.card.clone(), staged.bag);
                    snapshot.version = version;
                }
            } else {
                // Another holder committed between our cached read and this edit.
                cache.snapshot = None;
                cache.read_queued = true;
                cache.read_retry.first = Some(Instant::now());
            }
            state.clock = state.clock.max(Some(version));
        }
        state.truncate.due |= truncate_due;
        state.operator_settings.active = false;
        state.operator_settings.queued_bytes -= edit.bytes;
        drop(state);
        if let Err(error) = &result {
            self.notice(
                Level::Warn,
                format_args!(
                    "script-settings edit request={} refused: {error}",
                    edit.request.get()
                ),
            );
        }
        edit.finish(result);
        None
    }
}

fn source_path(root: &StateRoot) -> PathBuf {
    root.path().join(SOURCE)
}

fn state_error(root: &StateRoot, detail: impl Into<Arc<str>>) -> ScriptSettingsError {
    ScriptSettingsError::State {
        path: source_path(root),
        detail: detail.into(),
    }
}

fn limit_error(root: &StateRoot) -> ScriptSettingsError {
    state_error(
        root,
        "the settings exceed 1 MiB or 8,192 cards; reduce their size and retry",
    )
}

fn decode_error(path: &Path, source: serde_json::Error) -> ScriptSettingsError {
    ScriptSettingsError::Decode {
        path: path.to_owned(),
        source: Arc::new(source),
    }
}

fn json_error(message: impl std::fmt::Display) -> serde_json::Error {
    serde::de::Error::custom(message)
}

fn parse_source(path: &Path, raw: &str) -> Result<BTreeMap<String, Bag>, ScriptSettingsError> {
    let (value, duplicate) =
        api::json::value_and_duplicate_fields(raw).map_err(|error| decode_error(path, error))?;
    if let Some(duplicate) = duplicate {
        return Err(decode_error(path, json_error(duplicate)));
    }
    serde_json::from_value(value).map_err(|error| decode_error(path, error))
}

#[derive(Clone, Copy)]
struct Head {
    version: StoreVersion,
    used: usize,
    count: usize,
}

impl Default for Head {
    fn default() -> Self {
        Self {
            version: StoreVersion::INITIAL,
            used: 2,
            count: 0,
        }
    }
}

impl Head {
    fn inserted(&mut self, key_width: usize, value_width: usize) {
        self.used += key_width + 1 + value_width + usize::from(self.count != 0);
        self.count += 1;
    }

    fn within_limits(self) -> bool {
        self.used <= MAX_BYTES && self.count <= MAX_CARDS
    }
}

fn read_head(conn: &Connection) -> Result<Option<Head>, SqlFailure> {
    conn.query_row(
        "SELECT version, encoded_bytes, card_count FROM operator_script_head WHERE id = 1",
        [],
        |row| {
            let raw = row
                .get_ref(0)?
                .as_blob()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            let version = sql::decode_u64(raw).ok_or(rusqlite::Error::InvalidQuery)?;
            let bounded = |index| {
                usize::try_from(row.get::<_, i64>(index)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)
            };
            Ok(Head {
                version: StoreVersion::from_durable(version),
                used: bounded(1)?,
                count: bounded(2)?,
            })
        },
    )
    .optional()
    .map_err(|error| sql::refused_by(error, "reading the operator script settings head"))
}

fn write_head(conn: &Connection, head: Head) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO operator_script_head (id, version, encoded_bytes, card_count) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE SET version = excluded.version, encoded_bytes = excluded.encoded_bytes,
           card_count = excluded.card_count", params![sql::be(head.version.get()), head.used as i64, head.count as i64],
    )?;
    Ok(())
}

fn read_cards(conn: &Connection, head: Head) -> Result<Snapshot, SqlFailure> {
    let read = (|| {
        let mut stmt =
            conn.prepare("SELECT card_key, bag FROM operator_script_card ORDER BY card_key")?;
        let mut rows = stmt.query([])?;
        let mut cards = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let bytes = row
                .get_ref(1)?
                .as_blob()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            let bag = decode_bag(bytes)?;
            cards.insert(row.get(0)?, Arc::new(bag));
        }
        if cards.len() != head.count {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Snapshot {
            version: head.version,
            cards,
        })
    })();
    read.map_err(|error| sql::refused_by(error, "reading operator script settings"))
}

fn decode_bag(bytes: &[u8]) -> rusqlite::Result<Bag> {
    serde_json::from_slice(bytes).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(error))
    })
}

impl Db {
    fn read_operator_settings(
        &mut self,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Result<Snapshot, ScriptSettingsError>, SqlFailure> {
        self.open_for_domain()?;
        let mut result = self.read_operator_open(clock, notices);
        self.after_failure(&mut result);
        result
    }

    fn read_operator_open(
        &mut self,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Result<Snapshot, ScriptSettingsError>, SqlFailure> {
        let mut evidence = Evidence {
            source: source_path(&self.root),
            digest: None,
            domain: DOMAIN.code(),
            ledger_source: SOURCE.to_owned(),
            what: "operator script settings",
        };
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| db_error(error, "reading operator script settings"))?;
        let observed = read_head(&tx)?;
        if let Some(head) = observed {
            let snapshot = read_cards(&tx, head)?;
            let ledger = sql::read_ledger_of(&tx, DOMAIN.code(), SOURCE)?;
            tx.commit()
                .map_err(|error| db_error(error, "ending the operator settings read"))?;
            if let Some(ledger) = ledger {
                self.settle_plain_import(&evidence, ledger, notices);
            }
            return Ok(Ok(snapshot));
        }
        drop(tx);
        let guard = self.guard()?;
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = sql::begin_write(
            conn,
            &self.root,
            &guard,
            "starting the operator settings import",
        )?;
        if read_head(&tx)?.is_some() {
            // The winning holder published the whole import under the same guard.
            return Err(SqlFailure::Wait(WaitCause::Busy));
        }
        if sql::read_ledger_of(&tx, DOMAIN.code(), SOURCE)?.is_some()
            || sql::read_ledger_of(&tx, DOMAIN.code(), "*")?.is_some()
        {
            return Err(SqlFailure::refused(FailureReason::Schema,
                "operator defaults have an import receipt but no head; restore the paired state backup"));
        }
        let cards = match vault::read_private_file(&evidence.source, MAX_BYTES as u64) {
            Ok(raw) => {
                evidence.digest = Some(files::digest(raw.as_bytes()));
                match parse_source(&evidence.source, &raw) {
                    Ok(cards) => cards,
                    Err(error) => return Ok(Err(error)),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                return Ok(Err(ScriptSettingsError::Read {
                    path: evidence.source,
                    source: Arc::new(error),
                }))
            }
        };
        if cards.len() > MAX_CARDS {
            return Ok(Err(limit_error(&self.root)));
        }
        let mut head = Head {
            version: sql::next_version(&tx, clock)?,
            ..Head::default()
        };
        let snapshot = {
            let mut stmt = tx
                .prepare("INSERT INTO operator_script_card (card_key, bag) VALUES (?1, ?2)")
                .map_err(|error| db_error(error, "preparing the operator settings import"))?;
            let mut durable = BTreeMap::new();
            for (card, bag) in cards {
                let bytes = serde_json::to_vec(&bag).map_err(|error| {
                    SqlFailure::refused(FailureReason::Schema, error.to_string())
                })?;
                head.inserted(encoded_string_len(&card), bytes.len());
                if !head.within_limits() {
                    return Ok(Err(limit_error(&self.root)));
                }
                stmt.execute(params![card, bytes])
                    .map_err(|error| db_error(error, "importing an operator settings bag"))?;
                durable.insert(card, Arc::new(bag));
            }
            Snapshot {
                version: head.version,
                cards: durable,
            }
        };
        write_head(&tx, head)
            .map_err(|error| db_error(error, "writing the operator settings head"))?;
        tx.execute(
            "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
            [sql::be(head.version.get())],
        )
        .map_err(|error| db_error(error, "advancing the operator settings version"))?;
        sql::insert_import(&tx, &evidence)?;
        tx.commit()
            .map_err(|error| db_error(error, "committing the operator settings import"))?;
        drop(guard);
        notices.push((
            Level::Info,
            format!(
                "local state: script-settings import {} cards={} version={} source={}",
                if evidence.digest.is_some() {
                    "imported"
                } else {
                    "absent"
                },
                head.count,
                head.version,
                evidence.source.display()
            ),
        ));
        if evidence.digest.is_some() {
            self.settle_plain_import(
                &evidence,
                Ledger {
                    digest: evidence.digest,
                    evidence: "pending".into(),
                },
                notices,
            );
        }
        Ok(Ok(snapshot))
    }
}

fn db_error(error: rusqlite::Error, what: &str) -> SqlFailure {
    sql::wait_of(&error).map_or_else(|| sql::refused_by(error, what), SqlFailure::Wait)
}

struct Card {
    head: Head,
    bag: Option<Arc<Bag>>,
    width: usize,
}

fn read_for_edit(conn: &Connection, card: &str) -> Result<Loaded<Card>, SqlFailure> {
    let head = read_head(conn)?.ok_or_else(|| {
        SqlFailure::refused(FailureReason::Schema, "operator settings head is missing")
    })?;
    let current = conn
        .query_row(
            "SELECT bag FROM operator_script_card WHERE card_key = ?1",
            [card],
            |row| {
                let bytes = row
                    .get_ref(0)?
                    .as_blob()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok((bytes.len(), Arc::new(decode_bag(bytes)?)))
            },
        )
        .optional()
        .map_err(|error| db_error(error, "reading an operator settings bag"))?;
    let (width, bag) = current.map_or((0, None), |(width, bag)| (width, Some(bag)));
    Ok(Loaded {
        head: head.version,
        observed: Some(Card { head, bag, width }),
    })
}

struct Staged<'a> {
    card: &'a str,
    bag: Arc<Bag>,
    bytes: Option<Vec<u8>>,
    head: Head,
    previous: StoreVersion,
}

fn stage<'a>(current: &Card, edit: &'a Edit) -> Result<Staged<'a>, ErrorCode> {
    let mut head = current.head;
    let validation = |reason| ErrorCode::Validation {
        domain: DOMAIN,
        reason,
    };
    let exceeds = validation(ValidationReason::TooLong);
    let (bag, bytes) = if let Some(bag) = current
        .bag
        .as_ref()
        .filter(|bag| bag.get(&edit.key) == Some(&edit.value))
    {
        (Arc::clone(bag), None)
    } else {
        let mut bag = current.bag.as_deref().cloned().unwrap_or_default();
        bag.insert(edit.key.clone(), edit.value.clone());
        let bytes = serde_json::to_vec(&bag).map_err(|_| validation(ValidationReason::Schema))?;
        if current.bag.is_some() {
            head.used = head
                .used
                .checked_sub(current.width)
                .ok_or(ErrorCode::Failed {
                    domain: DOMAIN,
                    reason: FailureReason::Checksum,
                })?
                + bytes.len();
        } else {
            head.inserted(edit.card_width, bytes.len());
        }
        if !head.within_limits() {
            return Err(exceeds);
        }
        (Arc::new(bag), Some(bytes))
    };
    Ok(Staged {
        card: &edit.card,
        bag,
        bytes,
        previous: head.version,
        head,
    })
}

impl Staging for Staged<'_> {
    type Durable = Card;
    const DOMAIN: LocalStateDomain = DOMAIN;
    const HEAD: &'static str = DOMAIN.code();
    const ACTION: &'static str = "parameter-edit";

    fn unchanged(&self) -> bool {
        self.bytes.is_none()
    }
    fn digest(&self) -> files::ContentDigest {
        files::digest(self.bytes.as_deref().expect("a changed bag is encoded"))
    }
    fn set_version(&mut self, version: StoreVersion) {
        self.head.version = version;
    }
    fn write(&self, tx: &Connection, _version: StoreVersion) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO operator_script_card (card_key, bag) VALUES (?1, ?2)
             ON CONFLICT (card_key) DO UPDATE SET bag = excluded.bag",
            params![self.card, self.bytes],
        )?;
        write_head(tx, self.head)
    }
    fn durable(&self, _version: StoreVersion, _digest: files::ContentDigest) -> Card {
        Card {
            head: self.head,
            bag: Some(Arc::clone(&self.bag)),
            width: self.bytes.as_ref().map_or(0, Vec::len),
        }
    }
}

fn encoded_size(value: &Value) -> usize {
    #[derive(Default)]
    struct Counter(usize);
    impl io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter::default();
    serde_json::to_writer(&mut counter, value).map_or(usize::MAX, |()| counter.0)
}

fn encoded_string_len(value: &str) -> usize {
    2 + value
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>()
}
