//! Versioned loadout lists on the process writer's single SQLite connection.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};

use api::loadout::CarryEntry;
use api::local_state::{LoadoutEdit, LoadoutKey};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::sql::{self, Commit, Db, Evidence, Ledger, Loaded, SqlFailure, Staging, WaitCause};
use super::*;

const DOMAIN: LocalStateDomain = LocalStateDomain::LoadoutsReferences;
const SOURCE: &str = "loadouts.json";
const OPERATOR: &str = "operator-presets";
const MAX_ACCOUNT_LISTS: usize = 256;
const BODY_CHUNK_BYTES: usize = 32 * 1024;

pub(super) const SCHEMA: &str = "
CREATE TABLE loadout_head (
    store TEXT PRIMARY KEY CHECK (length(store) <= 256),
    version BLOB NOT NULL CHECK (length(version) = 8),
    digest BLOB CHECK (digest IS NULL OR length(digest) = 32),
    unreadable INTEGER NOT NULL CHECK (unreadable >= 0),
    lossy INTEGER NOT NULL CHECK (lossy >= 0)
) STRICT, WITHOUT ROWID;
CREATE TABLE loadout_row (
    store TEXT NOT NULL REFERENCES loadout_head(store),
    position INTEGER NOT NULL CHECK (position BETWEEN 0 AND 511),
    chunk INTEGER NOT NULL CHECK (chunk >= 0),
    name TEXT CHECK ((chunk = 0 AND name IS NOT NULL) OR (chunk > 0 AND name IS NULL)),
    body BLOB NOT NULL CHECK (length(body) BETWEEN 1 AND 32768),
    PRIMARY KEY (store, position, chunk)
) STRICT, WITHOUT ROWID;
";

/// A shared immutable list at one durable version, independent of a bot identity.
#[derive(Clone, Debug)]
pub struct LoadoutSnapshot {
    key: LoadoutKey,
    version: StoreVersion,
    snapshot: LeaseSnapshot,
    unreadable: usize,
    lossy: usize,
}

impl LoadoutSnapshot {
    /// The explicit owner of this list.
    pub fn key(&self) -> &LoadoutKey {
        &self.key
    }
    /// The durable version against which an edit is made.
    pub fn version(&self) -> StoreVersion {
        self.version
    }
    /// The ordered presets. Cloning this snapshot shares their allocation.
    pub fn rows(&self) -> &[Loadout] {
        self.snapshot.as_loadouts().expect("loadout snapshot")
    }
    /// Rows omitted because the old parser could not read them.
    pub fn unreadable_rows(&self) -> usize {
        self.unreadable
    }
    /// Usable rows with information outside the supported conversion mappings.
    pub fn lossy_rows(&self) -> usize {
        self.lossy
    }
    /// Whether the operator must accept this projection before saving it.
    pub fn read_only(&self) -> bool {
        self.unreadable != 0 || self.lossy != 0
    }

    fn of(key: &LoadoutKey, cached: &Cached) -> Self {
        let (unreadable, lossy) = counts(cached.rewrite);
        Self {
            key: key.clone(),
            version: cached.version,
            snapshot: cached.snapshot.clone(),
            unreadable,
            lossy,
        }
    }
}

impl StoreKey {
    pub(super) fn from_loadout_key(key: &LoadoutKey) -> Self {
        match key {
            LoadoutKey::OperatorPresets => Self::Loadouts,
            LoadoutKey::Account { account, scope } => Self::AccountLoadouts {
                profile: account.profile().into(),
                account: account.account().into(),
                revision: scope.revision().as_i32(),
                content: scope.content().into(),
            },
        }
    }

    fn loadout_name(&self) -> String {
        match self {
            Self::Loadouts => OPERATOR.into(),
            Self::AccountLoadouts {
                profile,
                account,
                revision,
                content,
            } => format!("account/{profile}/{account}/{revision}/{content}"),
            _ => unreachable!("loadout key"),
        }
    }
}

fn counts(rewrite: Rewrite) -> (usize, usize) {
    match rewrite {
        Rewrite::Lossless => (0, 0),
        Rewrite::Refused { unreadable, lossy } => (unreadable, lossy),
    }
}

fn snapshot_result(key: &LoadoutKey, file: &FileState) -> Result<LoadoutSnapshot, ErrorCode> {
    match file {
        FileState::Read(cached) => Ok(LoadoutSnapshot::of(key, cached)),
        FileState::Cold => Err(ErrorCode::NotReady { domain: DOMAIN }),
        FileState::Rejected(rejection) => Err(ErrorCode::Failed {
            domain: DOMAIN,
            reason: rejection.reason,
        }),
    }
}

impl LocalStateService {
    /// Queue a loadout read on the writer without waiting or constructing an account identity.
    pub fn start_loadout_readback(&self, key: &LoadoutKey) -> Result<ReadbackTicket, ErrorCode> {
        if !self.is_open(DOMAIN) {
            return Err(ErrorCode::NotReady { domain: DOMAIN });
        }
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return Err(ErrorCode::NotReady { domain: DOMAIN });
        }
        self.enqueue_readback(&mut state, &StoreKey::from_loadout_key(key))
    }

    /// Poll only the read identified by `ticket`. No disk access or wait occurs here.
    pub fn poll_loadout_readback(
        &self,
        key: &LoadoutKey,
        ticket: ReadbackTicket,
    ) -> Option<Result<LoadoutSnapshot, ErrorCode>> {
        self.finish_loadout_readback(key, ticket, None)
    }

    /// Wait on a startup/control thread only, never on a slot or frame thread.
    pub fn wait_loadout_readback(
        &self,
        key: &LoadoutKey,
        ticket: ReadbackTicket,
        timeout: Duration,
    ) -> Result<LoadoutSnapshot, ErrorCode> {
        self.finish_loadout_readback(key, ticket, Some(Instant::now() + timeout))
            .unwrap_or(Err(ErrorCode::NotReady { domain: DOMAIN }))
    }

    /// Read the immutable cached list. Use `start_loadout_readback` to hydrate or refresh it.
    pub fn loadout_snapshot(&self, key: &LoadoutKey) -> Result<LoadoutSnapshot, ErrorCode> {
        if !self.is_open(DOMAIN) {
            return Err(ErrorCode::NotReady { domain: DOMAIN });
        }
        let state = self.shared.state.lock();
        let store = state
            .stores
            .get(&StoreKey::from_loadout_key(key))
            .ok_or(ErrorCode::NotReady { domain: DOMAIN })?;
        snapshot_result(key, &store.file)
    }

    fn finish_loadout_readback(
        &self,
        key: &LoadoutKey,
        ticket: ReadbackTicket,
        deadline: Option<Instant>,
    ) -> Option<Result<LoadoutSnapshot, ErrorCode>> {
        let store_key = StoreKey::from_loadout_key(key);
        let mut state = self.shared.state.lock();
        loop {
            let Some(store) = state
                .stores
                .get(&store_key)
                .filter(|store| store.resident == ticket.resident)
            else {
                return Some(Err(ErrorCode::NotReady { domain: DOMAIN }));
            };
            if store.readbacks >= ticket.target {
                return Some(snapshot_result(key, &store.file));
            }
            if self
                .shared
                .settled
                .wait_until(&mut state, deadline?)
                .timed_out()
            {
                return None;
            }
        }
    }
}

fn db_error(error: rusqlite::Error, what: &str) -> SqlFailure {
    sql::wait_of(&error).map_or_else(|| sql::refused_by(error, what), SqlFailure::Wait)
}

fn damaged() -> SqlFailure {
    SqlFailure::refused(
        FailureReason::Checksum,
        "The stored loadout list is invalid. Restore the paired state backup.",
    )
}

#[derive(Serialize)]
struct GearRef<'a> {
    worn: &'a BTreeMap<String, String>,
    unassigned: &'a [String],
    #[serde(serialize_with = "serialize_carry")]
    carry: &'a [CarryEntry],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gear {
    worn: BTreeMap<String, String>,
    unassigned: Vec<String>,
    #[serde(deserialize_with = "deserialize_carry")]
    carry: Vec<CarryEntry>,
}

// Keep normalized unit-quantity carry entries as compact as their legacy source.
// The public canonical JSON remains unchanged; only the private SQL body uses this encoding.
fn serialize_carry<S: serde::Serializer>(
    rows: &[CarryEntry],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    struct Entry<'a>(&'a CarryEntry);
    impl Serialize for Entry<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            if self.0.qty == 1 {
                self.0.item.serialize(serializer)
            } else {
                (&self.0.item, self.0.qty).serialize(serializer)
            }
        }
    }
    let mut sequence = serializer.serialize_seq(Some(rows.len()))?;
    for row in rows {
        sequence.serialize_element(&Entry(row))?;
    }
    sequence.end()
}

fn deserialize_carry<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<CarryEntry>, D::Error> {
    struct Entry(CarryEntry);
    impl<'de> Deserialize<'de> for Entry {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Entry;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("an item string or an [item, quantity] pair")
                }
                fn visit_str<E: serde::de::Error>(self, item: &str) -> Result<Entry, E> {
                    Ok(Entry(CarryEntry {
                        item: item.to_owned(),
                        qty: 1,
                    }))
                }
                fn visit_string<E: serde::de::Error>(self, item: String) -> Result<Entry, E> {
                    Ok(Entry(CarryEntry { item, qty: 1 }))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    sequence: A,
                ) -> Result<Entry, A::Error> {
                    let (item, qty) = Deserialize::deserialize(
                        serde::de::value::SeqAccessDeserializer::new(sequence),
                    )?;
                    Ok(Entry(CarryEntry { item, qty }))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<CarryEntry>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a carry list")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut rows = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
            while let Some(Entry(row)) = sequence.next_element()? {
                rows.push(row);
            }
            Ok(rows)
        }
    }
    deserializer.deserialize_seq(Visitor)
}

fn validate_rows(rows: &[Loadout]) -> Result<(), ErrorCode> {
    let refuse = |reason| ErrorCode::Validation {
        domain: DOMAIN,
        reason,
    };
    if rows.len() > files::MAX_LOADOUT_ROWS {
        return Err(refuse(ValidationReason::TooManyRows));
    }
    let mut names = HashSet::with_capacity(rows.len());
    for row in rows {
        if row.name.trim().is_empty() {
            return Err(refuse(ValidationReason::Empty));
        }
        if !names.insert(row.name.as_str()) {
            return Err(refuse(ValidationReason::DuplicateId));
        }
        if row
            .worn
            .keys()
            .any(|slot| !api::loadout::is_worn_slot(slot))
            || row
                .worn
                .values()
                .chain(row.unassigned.iter())
                .any(|item| item.trim().is_empty())
            || row
                .carry
                .iter()
                .any(|entry| entry.item.trim().is_empty() || entry.qty == 0)
        {
            return Err(refuse(ValidationReason::Schema));
        }
    }
    Ok(())
}

pub(super) fn check_edit_size(
    edit: &LoadoutEdit,
    rows: &[Loadout],
    bytes: usize,
) -> Result<usize, ErrorCode> {
    let name = match edit.op() {
        LoadoutOp::Upsert(row) => row.name.as_str(),
        LoadoutOp::Rename { from, .. } => from.as_str(),
        LoadoutOp::Remove { .. } => return Ok(0),
    };
    let previous = rows
        .iter()
        .find(|row| row.name == name)
        .map(files::row_bytes)
        .transpose()
        .map_err(|_| ErrorCode::Validation {
            domain: DOMAIN,
            reason: ValidationReason::Schema,
        })?
        .unwrap_or(0);
    if bytes <= files::MAX_LOADOUT_ROW_BYTES || bytes <= previous {
        Ok(previous)
    } else {
        Err(ErrorCode::Validation {
            domain: DOMAIN,
            reason: ValidationReason::TooLong,
        })
    }
}

fn write_head(conn: &Connection, name: &str, cached: &Cached) -> rusqlite::Result<()> {
    let (unreadable, lossy) = counts(cached.rewrite);
    conn.execute("INSERT INTO loadout_head (store,version,digest,unreadable,lossy) VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(store) DO UPDATE SET version=excluded.version,digest=excluded.digest,unreadable=excluded.unreadable,lossy=excluded.lossy",
        params![name, sql::be(cached.version.get()), cached.digest.as_ref().map(|digest| digest.as_slice()), unreadable as i64, lossy as i64])?;
    Ok(())
}

// A supported source can contain a single 4 MiB row. Bind and read its body in
// bounded pieces so SQLite never needs a second whole-row allocation.
struct BodyWriter<'a, 'statement> {
    insert: &'a mut rusqlite::Statement<'statement>,
    store: &'a str,
    position: i64,
    name: &'a str,
    chunk: i64,
    error: Option<rusqlite::Error>,
}
impl Write for BodyWriter<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for body in bytes.chunks(BODY_CHUNK_BYTES) {
            let name = (self.chunk == 0).then_some(self.name);
            if let Err(error) =
                self.insert
                    .execute(params![self.store, self.position, self.chunk, name, body])
            {
                self.error = Some(error);
                return Err(io::Error::other("writing a loadout body chunk"));
            }
            self.chunk += 1;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BodyReader<'a, 'statement> {
    rows: rusqlite::Rows<'statement>,
    buffer: &'a mut Vec<u8>,
    offset: usize,
    chunk: i64,
    error: Option<rusqlite::Error>,
}
impl io::Read for BodyReader<'_, '_> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        if into.is_empty() {
            return Ok(0);
        }
        if self.offset == self.buffer.len() {
            let row = match self.rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => return Ok(0),
                Err(error) => {
                    self.error = Some(error);
                    return Err(io::Error::other("reading a loadout body chunk"));
                }
            };
            let chunk: i64 = row.get(0).map_err(io::Error::other)?;
            let body = row
                .get_ref(1)
                .map_err(io::Error::other)?
                .as_blob()
                .map_err(io::Error::other)?;
            if chunk != self.chunk || body.is_empty() || body.len() > BODY_CHUNK_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid loadout body chunks",
                ));
            }
            self.buffer.clear();
            self.buffer.extend_from_slice(body);
            self.offset = 0;
            self.chunk += 1;
        }
        let count = into.len().min(self.buffer.len() - self.offset);
        into[..count].copy_from_slice(&self.buffer[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

fn write_rows(
    conn: &Connection,
    name: &str,
    rows: &[Loadout],
    old: &[Loadout],
) -> rusqlite::Result<()> {
    let mut insert = conn.prepare(
        "INSERT INTO loadout_row (store,position,chunk,name,body) VALUES (?1,?2,?3,?4,?5)",
    )?;
    for (position, row) in rows.iter().enumerate() {
        if old.get(position) == Some(row) {
            continue;
        }
        conn.execute(
            "DELETE FROM loadout_row WHERE store=?1 AND position=?2",
            params![name, position as i64],
        )?;
        let writer = BodyWriter {
            insert: &mut insert,
            store: name,
            position: position as i64,
            name: &row.name,
            chunk: 0,
            error: None,
        };
        let mut writer = io::BufWriter::with_capacity(BODY_CHUNK_BYTES, writer);
        let result = serde_json::to_writer(
            &mut writer,
            &GearRef {
                worn: &row.worn,
                unassigned: &row.unassigned,
                carry: &row.carry,
            },
        )
        .map_err(io::Error::other)
        .and_then(|()| writer.flush());
        let (writer, _) = writer.into_parts();
        if let Some(error) = writer.error {
            return Err(error);
        }
        result.map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    conn.execute(
        "DELETE FROM loadout_row WHERE store=?1 AND position>=?2",
        params![name, rows.len() as i64],
    )?;
    Ok(())
}

pub(super) fn load(
    conn: &Connection,
    name: &str,
    known: Option<&Cached>,
) -> Result<Loaded<Cached>, SqlFailure> {
    let head = conn
        .query_row(
            "SELECT version,digest,unreadable,lossy FROM loadout_head WHERE store=?1",
            [name],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| db_error(error, "reading the loadout head"))?;
    let Some((version, digest, unreadable, lossy)) = head else {
        return Ok(Loaded {
            head: StoreVersion::INITIAL,
            observed: None,
        });
    };
    let version = StoreVersion::from_durable(sql::decode_u64(&version).ok_or_else(damaged)?);
    let digest: Option<ContentDigest> = digest
        .map(|bytes| bytes.try_into().map_err(|_| damaged()))
        .transpose()?;
    let (unreadable, lossy) = (
        usize::try_from(unreadable).map_err(|_| damaged())?,
        usize::try_from(lossy).map_err(|_| damaged())?,
    );
    let rewrite = if unreadable == 0 && lossy == 0 {
        Rewrite::Lossless
    } else {
        Rewrite::Refused { unreadable, lossy }
    };
    if known.is_some_and(|known| {
        known.version == version && known.digest == digest && known.rewrite == rewrite
    }) {
        return Ok(Loaded {
            head: version,
            observed: None,
        });
    }
    let mut query = conn
        .prepare(
            "SELECT position,name FROM loadout_row WHERE store=?1 AND chunk=0 ORDER BY position",
        )
        .map_err(|error| db_error(error, "reading loadout rows"))?;
    let mut stored = query
        .query([name])
        .map_err(|error| db_error(error, "reading loadout rows"))?;
    let mut rows = Vec::new();
    let mut chunks = conn
        .prepare("SELECT chunk,body FROM loadout_row WHERE store=?1 AND position=?2 ORDER BY chunk")
        .map_err(|error| db_error(error, "reading loadout body chunks"))?;
    let mut buffer = Vec::with_capacity(BODY_CHUNK_BYTES);
    while let Some(row) = stored
        .next()
        .map_err(|error| db_error(error, "reading loadout rows"))?
    {
        let position =
            usize::try_from(row.get::<_, i64>(0).map_err(|_| damaged())?).map_err(|_| damaged())?;
        if position != rows.len() || rows.len() == files::MAX_LOADOUT_ROWS {
            return Err(damaged());
        }
        let row_name = row.get(1).map_err(|_| damaged())?;
        buffer.clear();
        let mut reader = BodyReader {
            rows: chunks
                .query(params![name, position as i64])
                .map_err(|error| db_error(error, "reading loadout body chunks"))?,
            buffer: &mut buffer,
            offset: 0,
            chunk: 0,
            error: None,
        };
        let gear = serde_json::from_reader::<_, Gear>(&mut reader);
        if let Some(error) = reader.error {
            return Err(db_error(error, "reading loadout body chunks"));
        }
        let gear = gear.map_err(|_| damaged())?;
        rows.push(Loadout {
            name: row_name,
            worn: gear.worn,
            unassigned: gear.unassigned,
            carry: gear.carry,
        });
    }
    let (written_len, actual_digest) = files::loadouts_fingerprint(&rows).map_err(|_| damaged())?;
    validate_rows(&rows).map_err(|_| damaged())?;
    if digest.is_some_and(|digest| digest != actual_digest)
        || (digest.is_none() && !rows.is_empty())
    {
        return Err(damaged());
    }
    Ok(Loaded {
        head: version,
        observed: Some(Cached {
            version,
            at: DurableTimestamp::now(),
            snapshot: LeaseSnapshot::loadouts(rows),
            preferences: None,
            refreshed_at: Instant::now(),
            digest,
            durable: true,
            written_len,
            rewrite,
        }),
    })
}

impl Db {
    fn read_loadout_list(
        &mut self,
        key: &StoreKey,
        known: Option<&Cached>,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Option<Cached>, SqlFailure> {
        self.open_for_domain()?;
        let mut result = self.read_loadout_open(key, known, clock, notices);
        self.after_failure(&mut result);
        result
    }

    fn read_loadout_open(
        &mut self,
        key: &StoreKey,
        known: Option<&Cached>,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Option<Cached>, SqlFailure> {
        let name = key.loadout_name();
        let mut evidence = Evidence {
            source: self.root.loadouts_file(),
            digest: None,
            domain: DOMAIN.code(),
            ledger_source: SOURCE.into(),
            what: "loadouts",
        };
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|error| db_error(error, "starting a loadout read"))?;
        let loaded = load(&tx, &name, known)?;
        let ledger = if key == &StoreKey::Loadouts {
            sql::read_ledger_of(&tx, DOMAIN.code(), SOURCE)?
        } else {
            None
        };
        if loaded.head != StoreVersion::INITIAL {
            tx.commit()
                .map_err(|error| db_error(error, "ending a loadout read"))?;
            if let Some(ledger) = ledger {
                self.settle_plain_import(&evidence, ledger, notices);
            }
            return Ok(loaded.observed);
        }
        if ledger.is_some() {
            return Err(SqlFailure::refused(FailureReason::Schema, "The loadout import is closed but its head is missing. Restore the paired state backup."));
        }
        drop(tx);
        let guard = self.guard()?;
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = sql::begin_write_reserved(
            conn,
            &self.root,
            &guard,
            "importing loadouts",
            sql::DB_MAX_PAGES,
        )?;
        if load(&tx, &name, None)?.head != StoreVersion::INITIAL {
            return Err(SqlFailure::Wait(WaitCause::Busy));
        }
        if key == &StoreKey::Loadouts && sql::read_ledger_of(&tx, DOMAIN.code(), SOURCE)?.is_some()
        {
            return Err(SqlFailure::Wait(WaitCause::Busy));
        }
        let closed =
            key == &StoreKey::Loadouts && sql::read_ledger_of(&tx, DOMAIN.code(), "*")?.is_some();
        let document = if key == &StoreKey::Loadouts && !closed {
            match vault::read_private_file(&evidence.source, MAX_LOADOUTS_BYTES) {
                Ok(raw) => {
                    let document = files::decode_loadouts(&raw).map_err(|error| SqlFailure::refused(error.reason(), format!(
                        "Loadouts refused at {}: {error}; it is neither leased nor written until it reads cleanly. Correct the source and retry. No loadouts were imported.", evidence.source.display())))?;
                    evidence.digest = Some(files::digest(raw.as_bytes()));
                    Some(document)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(SqlFailure::refused(FailureReason::Io, format!("Loadouts refused at {}: {error}. Restore a readable private file and retry. Nothing was imported.", evidence.source.display()))),
            }
        } else {
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM loadout_head WHERE store<>?1",
                    [OPERATOR],
                    |row| row.get(0),
                )
                .map_err(|error| db_error(error, "counting account loadouts"))?;
            if key != &StoreKey::Loadouts && count >= MAX_ACCOUNT_LISTS as i64 {
                return Err(SqlFailure::refused(
                    FailureReason::Io,
                    "The account loadout store limit is full; no stored list was evicted.",
                ));
            }
            None
        };
        let (snapshot, rewrite) = document.map_or_else(
            || (LeaseSnapshot::loadouts(Vec::new()), Rewrite::Lossless),
            |(rows, rewrite)| (LeaseSnapshot::loadouts(rows), rewrite),
        );
        let rows = snapshot.as_loadouts().expect("loadout snapshot");
        let (written_len, canonical_digest) =
            files::loadouts_fingerprint(rows).map_err(|_| damaged())?;
        validate_rows(rows).map_err(|error| {
            SqlFailure::refused(
                FailureReason::Schema,
                format!(
                    "Loadouts refused at {}: {error}. Nothing was imported.",
                    evidence.source.display()
                ),
            )
        })?;
        let version = sql::next_version(&tx, clock)?;
        let cached = Cached {
            version,
            at: DurableTimestamp::now(),
            snapshot,
            preferences: None,
            refreshed_at: Instant::now(),
            digest: evidence.digest.map(|_| canonical_digest),
            durable: true,
            written_len,
            rewrite,
        };
        write_head(&tx, &name, &cached)
            .map_err(|error| db_error(error, "writing the loadout head"))?;
        write_rows(
            &tx,
            &name,
            cached.snapshot.as_loadouts().expect("loadout snapshot"),
            &[],
        )
        .map_err(|error| db_error(error, "importing loadout rows"))?;
        tx.execute(
            "UPDATE store_meta SET sequence=?1 WHERE id=1",
            [sql::be(version.get())],
        )
        .map_err(|error| db_error(error, "advancing the loadout version"))?;
        if key == &StoreKey::Loadouts && !closed {
            sql::insert_import(&tx, &evidence)?;
        }
        #[cfg(test)]
        if let Some(point) = &self.seams.point {
            point("before-loadouts-import-commit");
        }
        tx.commit()
            .map_err(|error| db_error(error, "committing the loadout import"))?;
        drop(guard);
        self.point("after-loadouts-import-commit");
        let (unreadable, lossy) = counts(rewrite);
        notices.push((Level::Info, format!("audit domain={} key={name} action=loadout-import origin=import old=0 new={version} rows={} unreadable={unreadable} lossy={lossy}", DOMAIN.code(), cached.snapshot.as_loadouts().expect("loadout snapshot").len())));
        if unreadable != 0 || lossy != 0 {
            notices.push((Level::Warn, format!("Loadouts {name} at {version} are a read-only projection: {unreadable} unreadable row(s), {lossy} lossy row(s). Original evidence is kept beside {}. Stop the root and run state loadouts accept-projection {name} to accept only the usable rows.", evidence.source.display())));
        }
        if key == &StoreKey::Loadouts && !closed {
            self.settle_plain_import(
                &evidence,
                Ledger {
                    digest: evidence.digest,
                    evidence: if evidence.digest.is_some() {
                        "pending"
                    } else {
                        "none"
                    }
                    .into(),
                },
                notices,
            );
        }
        Ok(Some(cached))
    }
}

struct LoadoutStaged<'a> {
    name: &'a str,
    staged: Staged,
    old: LeaseSnapshot,
}

impl Staging for LoadoutStaged<'_> {
    type Durable = Cached;
    const DOMAIN: LocalStateDomain = DOMAIN;
    // A complete 4 MiB list can use more pages than its canonical JSON.
    // Reserving the existing whole-database cap also covers overflow/index pages.
    const MAX_PAGES: u64 = sql::DB_MAX_PAGES;
    const HEAD: &'static str = "loadouts";
    const ACTION: &'static str = "loadout-write";
    fn unchanged(&self) -> bool {
        self.staged.unchanged
    }
    fn digest(&self) -> ContentDigest {
        self.staged.digest
    }
    fn set_version(&mut self, version: StoreVersion) {
        self.staged.version = version;
    }
    fn write(&self, tx: &Connection, version: StoreVersion) -> rusqlite::Result<()> {
        write_head(tx, self.name, &self.durable(version, self.digest()))?;
        write_rows(
            tx,
            self.name,
            self.staged
                .snapshot
                .as_loadouts()
                .expect("loadout snapshot"),
            self.old.as_loadouts().expect("loadout snapshot"),
        )
    }
    fn durable(&self, version: StoreVersion, digest: ContentDigest) -> Cached {
        Cached {
            version,
            at: self.staged.at,
            snapshot: self.staged.snapshot.clone(),
            preferences: None,
            refreshed_at: Instant::now(),
            digest: Some(digest),
            durable: true,
            written_len: self.staged.written_len,
            rewrite: Rewrite::Lossless,
        }
    }
}

impl Shared {
    pub(super) fn readback_loadouts(&self, db: &mut Db, key: &StoreKey) -> Option<WaitCause> {
        self.stall_read();
        let (known, clock) = {
            let state = self.state.lock();
            (
                state.stores.get(key).and_then(|store| match &store.file {
                    FileState::Read(cached) => Some(cached.clone()),
                    _ => None,
                }),
                state.clock,
            )
        };
        let mut notices = Vec::new();
        let result = db.read_loadout_list(key, known.as_ref(), clock, &mut notices);
        for (level, line) in notices {
            self.notice(level, format_args!("{line}"));
        }
        let mut state = self.state.lock();
        state.truncate.due |= db.wal_truncate_due();
        match result {
            Ok(observed) => state.adopt_loadouts(key, observed),
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { reason, detail }) => {
                let repeated = state.stores.get(key).is_some_and(|store| {
                    matches!(&store.file, FileState::Rejected(previous) if previous.reason == reason && previous.detail == detail)
                });
                if !repeated {
                    emit_local_notice(
                        Level::Warn,
                        format_args!("{detail}"),
                        #[cfg(test)]
                        state.hooks.read_refusal.as_ref(),
                    );
                }
                if let Some(store) = state.stores.get_mut(key) {
                    store.file = FileState::Rejected(Rejection { reason, detail });
                }
            }
        }
        state.readback_done(key);
        None
    }

    pub(super) fn write_loadouts(
        &self,
        db: &mut Db,
        key: &StoreKey,
        queued: &Queued,
    ) -> Option<WaitCause> {
        #[cfg(any(test, feature = "test-support"))]
        {
            let stall = self.state.lock().write_stall.clone();
            if let Some(stall) = stall {
                stall();
            }
            let fault = self.state.lock().write_fault.clone();
            if fault.is_some_and(|fault| fault()) {
                self.finish(key, queued, io_failure(DOMAIN), false);
                return None;
            }
            let unknown = self.state.lock().write_unknown.clone();
            if unknown.is_some_and(|unknown| unknown()) {
                self.finish_unresolved(
                    key,
                    queued,
                    io_failure(DOMAIN),
                    sql::unresolved_detail(
                        &"disk I/O error (test fault)",
                        Some("the test's fault"),
                    ),
                );
                return None;
            }
        }
        let (known, external, clock) = {
            let state = self.state.lock();
            let store = state.stores.get(key).expect("queued loadout store");
            match &store.file {
                FileState::Read(known) => (Some(known.clone()), store.external, state.clock),
                _ => (None, store.external, state.clock),
            }
        };
        let Some(known) = known else {
            self.finish(key, queued, dirty(DOMAIN), false);
            return None;
        };
        let name = key.loadout_name();
        let committed = db.commit(
            queued.request,
            clock,
            Some(&known),
            || self.begin_sql_write(key),
            |conn| load(conn, &name, Some(&known)),
            |current| {
                let file = CachedDecision::from_cached(current, DOMAIN);
                let external = external
                    + u64::from(current.version != known.version || current.digest != known.digest);
                let staged =
                    decide(key, &file, external, queued).map_err(
                        |settlement| match settlement {
                            Settlement::Failed(code) => code,
                            _ => ErrorCode::Failed {
                                domain: DOMAIN,
                                reason: FailureReason::Dirty,
                            },
                        },
                    )?;
                validate_rows(staged.snapshot.as_loadouts().expect("loadout snapshot"))?;
                Ok(LoadoutStaged {
                    name: &name,
                    staged,
                    old: current.snapshot.clone(),
                })
            },
        );
        let (settlement, changed) = match committed {
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { reason, detail }) => {
                self.notice(Level::Warn, format_args!("{detail}"));
                if let Some(store) = self.state.lock().stores.get_mut(key) {
                    store.file = FileState::Rejected(Rejection { reason, detail });
                }
                (
                    Settlement::Failed(ErrorCode::Failed {
                        domain: DOMAIN,
                        reason,
                    }),
                    false,
                )
            }
            Ok(Commit::Declined { code, observed, .. }) => {
                self.state.lock().adopt_loadouts(key, observed);
                (Settlement::Failed(code), false)
            }
            Ok(Commit::Unresolved { detail, observed }) => {
                self.state.lock().adopt_loadouts(key, observed);
                self.finish_unresolved(key, queued, io_failure(DOMAIN), detail);
                return None;
            }
            Ok(Commit::Abandoned) => (Settlement::Cancelled(Cancellation::Shutdown), false),
            Ok(Commit::Unchanged { staged, observed }) => {
                self.state.lock().adopt_loadouts(key, observed);
                (
                    Settlement::Completed(DurableReceipt::new(
                        queued.intent.identity().clone(),
                        staged.staged.previous,
                        DurableTimestamp::now(),
                        Some(queued.request),
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
                self.audit(
                    AuditCommit::after_commit(
                        queued.intent.identity().clone(),
                        CommitAction::LoadoutWrite,
                        staged.staged.previous,
                        version,
                        AuditOrigin::Submitted {
                            request: queued.request,
                        },
                        now,
                    ),
                    &AuditKind::Plain,
                );
                (
                    Settlement::Completed(DurableReceipt::new(
                        queued.intent.identity().clone(),
                        version,
                        now,
                        Some(queued.request),
                    )),
                    true,
                )
            }
        };
        self.finish(key, queued, settlement, changed);
        None
    }
}

impl State {
    fn adopt_loadouts(&mut self, key: &StoreKey, observed: Option<Cached>) {
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
}

pub(super) fn projection_evidence(
    root: &StateRoot,
    digest: Option<ContentDigest>,
) -> Option<PathBuf> {
    let digest = digest?;
    (0..100u32)
        .map(|index| {
            root.path().join(if index == 0 {
                "loadouts.json.migrated".into()
            } else {
                format!("loadouts.json.migrated-{index}")
            })
        })
        .find(|path| {
            vault::read_private_file(path, MAX_LOADOUTS_BYTES)
                .is_ok_and(|raw| files::digest(raw.as_bytes()) == digest)
        })
}

impl Db {
    pub(super) fn loadout_report(
        &self,
        name: &str,
    ) -> Result<Option<(Cached, Ledger)>, SqlFailure> {
        if name != OPERATOR {
            return Ok(None);
        }
        let conn = self.conn.as_ref().ok_or_else(sql::closed)?;
        let Some(cached) = load(conn, name, None)?.observed else {
            return Ok(None);
        };
        let Some(ledger) = sql::read_ledger_of(conn, DOMAIN.code(), SOURCE)? else {
            return Ok(None);
        };
        Ok(Some((cached, ledger)))
    }

    /// The maintenance caller holds the exclusive root lease and product guard.
    pub(super) fn accept_loadout_projection(
        &mut self,
        guard: &files::WriteLock,
        name: &str,
        expected: StoreVersion,
    ) -> Result<StoreVersion, SqlFailure> {
        if name != OPERATOR {
            return Err(SqlFailure::refused(
                FailureReason::Identity,
                "This key has no imported loadout projection. Nothing was changed.",
            ));
        }
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = sql::begin_write(conn, &self.root, guard, "accepting a loadout projection")?;
        let mut cached = load(&tx, name, None)?.observed.ok_or_else(damaged)?;
        if cached.version != expected || cached.rewrite == Rewrite::Lossless {
            return Err(SqlFailure::refused(
                FailureReason::Dirty,
                "The loadout head changed or has no projection to accept. Nothing was changed.",
            ));
        }
        let ledger = sql::read_ledger_of(&tx, DOMAIN.code(), SOURCE)?.ok_or_else(damaged)?;
        if projection_evidence(&self.root, ledger.digest).is_none() {
            return Err(SqlFailure::refused(FailureReason::Io, "The private loadout evidence is missing. Restore the .migrated evidence or recover from a repaired source. Nothing was changed."));
        }
        let version = sql::next_version(&tx, None)?;
        cached.version = version;
        cached.rewrite = Rewrite::Lossless;
        write_head(&tx, name, &cached)
            .map_err(|error| db_error(error, "clearing the loadout projection refusal"))?;
        tx.execute(
            "UPDATE store_meta SET sequence=?1 WHERE id=1",
            [sql::be(version.get())],
        )
        .map_err(|error| db_error(error, "advancing the loadout acceptance version"))?;
        tx.commit()
            .map_err(|error| db_error(error, "committing the loadout projection acceptance"))?;
        Ok(version)
    }
}
