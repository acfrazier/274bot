//! Root-wide launch roster, hydrated by the process writer before selection.

use std::io;

use rusqlite::{params, Connection, OptionalExtension};

use super::sql::{self, Db, Evidence, Ledger, SqlFailure, WaitCause};
use super::*;
use crate::servers::{LaunchProfile, LaunchTransport, LaunchWorld, LoginKey, Servers};

const DOMAIN: &str = "servers";
const SOURCE: &str = "servers.json";
/// The existing database capacity also bounds legacy input. This is not a new
/// smaller file-format limit, and a refusal never closes the first import.
pub(super) const MAX_SOURCE_BYTES: u64 = sql::DB_MAX_PAGES * sql::PAGE_SIZE;

pub(super) const SCHEMA: &str = "
CREATE TABLE server_profile (
    name_key TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    revision INTEGER NOT NULL CHECK (revision IN (274, 289, 377)),
    transport TEXT NOT NULL CHECK (transport IN ('tcp', 'wss')),
    login_key BLOB NOT NULL,
    vault TEXT NOT NULL COLLATE NOCASE UNIQUE,
    members INTEGER CHECK (members IN (0, 1)),
    plaintext INTEGER NOT NULL CHECK (plaintext IN (0, 1))
) STRICT;
CREATE TABLE server_world (
    profile TEXT NOT NULL REFERENCES server_profile(name_key),
    position INTEGER NOT NULL CHECK (position >= 0),
    number INTEGER NOT NULL CHECK (number BETWEEN 1 AND 65535),
    host TEXT NOT NULL,
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    node_id INTEGER NOT NULL CHECK (node_id > 0),
    asset_host TEXT NOT NULL,
    asset_port INTEGER NOT NULL CHECK (asset_port BETWEEN 1 AND 65535),
    PRIMARY KEY (profile, position),
    UNIQUE (profile, number)
) STRICT, WITHOUT ROWID;
";

/// An immutable, versioned launch roster shared by selection and bound Play.
#[derive(Clone, Debug)]
pub struct ServerRoster {
    version: StoreVersion,
    servers: Arc<Servers>,
}

impl ServerRoster {
    /// The validated saved roster, without additions from the preset catalog.
    pub fn servers(&self) -> &Servers {
        &self.servers
    }

    /// The durable version future roster edits must check before replacement.
    pub fn version(&self) -> StoreVersion {
        self.version
    }
}

/// A pending or refused launch roster. No refusal selects default profiles.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ServerRosterError {
    /// The writer has not yet published its first durable snapshot.
    #[error("Server profiles are still loading")]
    NotReady,
    /// A standalone roster from before the supported saved format.
    #[error("{path}: saved data is in a format from before 0.2.8", path = .path.display())]
    UnsupportedSavedFormat { path: PathBuf },
    /// The legacy source cannot supply a complete admitted roster.
    #[error("Server profiles refused at {path}: {cause}. Correct the source and restart. No roster was imported.", path = .path.display())]
    Content { path: PathBuf, cause: String },
    /// The source or database could not supply a complete validated roster.
    #[error("{0}")]
    Refused(String),
}

#[derive(Default)]
pub(super) struct ServerState {
    snapshot: Option<ServerRoster>,
    refusal: Option<ServerRosterError>,
    pub(super) queued: bool,
    pub(super) retry: Retry,
    notices: Vec<String>,
}

impl ServerState {
    pub(super) fn reject(&mut self, detail: impl Into<String>) {
        self.snapshot = None;
        self.queued = false;
        self.refusal = Some(ServerRosterError::Refused(detail.into()));
    }
}

impl LocalStateService {
    /// Poll startup readiness without filesystem access or waiting.
    pub fn server_roster(&self) -> Result<ServerRoster, ServerRosterError> {
        let mut state = self.shared.state.lock();
        if let Some(snapshot) = &state.servers.snapshot {
            return Ok(snapshot.clone());
        }
        if let Some(error) = &state.servers.refusal {
            return Err(error.clone());
        }
        if state.shutdown || !self.shared.start_writer(&mut state) {
            return Err(ServerRosterError::Refused(
                "The state service has stopped".into(),
            ));
        }
        if !state.servers.queued {
            state.servers.queued = true;
            state.servers.retry.first = Some(Instant::now());
            self.shared.wake.notify_one();
        }
        Err(ServerRosterError::NotReady)
    }

    /// Retry after the operator moves an unsupported legacy roster aside.
    /// Other refusals and completed imports retain their disposition.
    pub fn retry_server_roster(&self) -> Result<ServerRoster, ServerRosterError> {
        {
            let mut state = self.shared.state.lock();
            if matches!(
                state.servers.refusal,
                Some(ServerRosterError::UnsupportedSavedFormat { .. })
            ) {
                state.servers.refusal = None;
                state.servers.retry = Retry::default();
            }
        }
        self.server_roster()
    }

    /// Wait only on a startup/control thread, never on a frame or slot thread.
    pub fn wait_server_roster(&self, timeout: Duration) -> Result<ServerRoster, ServerRosterError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.server_roster() {
                Err(ServerRosterError::NotReady) => {}
                result => return result,
            }
            let mut state = self.shared.state.lock();
            if state.servers.snapshot.is_some() || state.servers.refusal.is_some() {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(ServerRosterError::NotReady);
            }
            self.shared.settled.wait_until(&mut state, deadline);
        }
    }

    /// Drain display notices which the writer already logged and recorded.
    pub fn take_server_notices(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.state.lock().servers.notices)
    }
}

impl Shared {
    pub(super) fn hydrate_servers(&self, db: &mut Db) -> Option<WaitCause> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(stall) = self.state.lock().read_stall.clone() {
            stall();
        }
        let clock = self.state.lock().clock;
        let mut notices = Vec::new();
        let result = match db.read_servers(clock, &mut notices) {
            Ok(result) => result,
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { detail, .. }) => Err(ServerRosterError::Refused(format!(
                "Server profiles refused at {}: {detail}",
                self.root.database_file().display()
            ))),
        };
        let truncate_due = db.wal_truncate_due();
        let mut display = Vec::new();
        for (level, notice) in notices {
            self.notice(level, format_args!("{notice}"));
            if level == Level::Warn {
                display.push(notice);
            }
        }
        if let Err(error) = &result {
            self.notice(Level::Warn, format_args!("{error}"));
        }
        let mut state = self.state.lock();
        state.truncate.due |= truncate_due;
        state.servers.queued = false;
        state.servers.retry = Retry::default();
        state.servers.notices.extend(display);
        match result {
            Ok(snapshot) => {
                state.clock = state.clock.max(Some(snapshot.version));
                state.servers.snapshot = Some(snapshot);
            }
            Err(error) => state.servers.refusal = Some(error),
        }
        None
    }
}

fn source_error(path: &Path, error: impl std::fmt::Display) -> ServerRosterError {
    ServerRosterError::Content {
        path: path.to_path_buf(),
        cause: error.to_string(),
    }
}

fn db_error(error: rusqlite::Error, what: &str) -> SqlFailure {
    sql::wait_of(&error).map_or_else(|| sql::refused_by(error, what), SqlFailure::Wait)
}

impl Db {
    fn read_servers(
        &mut self,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Result<ServerRoster, ServerRosterError>, SqlFailure> {
        self.open_for_domain()?;
        let mut result = self.read_servers_open(clock, notices);
        self.after_failure(&mut result);
        result
    }

    fn read_servers_open(
        &mut self,
        clock: Option<StoreVersion>,
        notices: &mut Vec<(Level, String)>,
    ) -> Result<Result<ServerRoster, ServerRosterError>, SqlFailure> {
        self.ensure_root_use()?;
        let mut evidence = Evidence {
            source: self.root.path().join(SOURCE),
            digest: None,
            domain: DOMAIN,
            ledger_source: SOURCE.into(),
            what: "server profiles",
        };
        #[cfg(test)]
        let point = self.seams.point.clone();
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|error| db_error(error, "reading the server roster"))?;
        let version = read_version(&tx)?;
        let ledger = sql::read_ledger_of(&tx, DOMAIN, SOURCE)?;
        if let Some(version) = version {
            let snapshot = read_roster(&tx, version)?;
            tx.commit()
                .map_err(|error| db_error(error, "ending the server roster read"))?;
            if let Some(ledger) = ledger {
                self.settle_plain_import(&evidence, ledger, notices);
            }
            return Ok(Ok(snapshot));
        }
        drop(tx);
        let guard = self.guard()?;
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = sql::begin_write(conn, &self.root, &guard, "importing the server roster")?;
        if read_version(&tx)?.is_some() {
            return Err(SqlFailure::Wait(WaitCause::Busy));
        }
        let ledger = sql::read_ledger_of(&tx, DOMAIN, SOURCE)?;
        if ledger.is_some() || sql::read_ledger_of(&tx, DOMAIN, "*")?.is_some() {
            return Err(SqlFailure::refused(FailureReason::Schema,
                "The server roster has a closed import but no head. Restore the paired state backup."));
        }
        let servers = match vault::read_private_file(&evidence.source, MAX_SOURCE_BYTES) {
            Ok(raw) => {
                evidence.digest = Some(files::digest(raw.as_bytes()));
                match Servers::decode_legacy(&evidence.source, raw.as_bytes()) {
                    Ok(servers) => servers,
                    Err(error) => return Ok(Err(source_error(&evidence.source, error))),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let worlds = self.root.path().join("worlds.json");
                match worlds.metadata() {
                    Ok(metadata) if metadata.is_file() => {
                        return Ok(Err(ServerRosterError::UnsupportedSavedFormat {
                            path: worlds,
                        }))
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Ok(Err(source_error(&worlds, error))),
                }
                let defaults = Servers::builtins();
                if let Err(error) = defaults.validate() {
                    return Ok(Err(source_error(&evidence.source, error)));
                }
                defaults
            }
            Err(error) => return Ok(Err(source_error(&evidence.source, error))),
        };
        let version = sql::next_version(&tx, clock)?;
        let Some(mut budget) = self::write_roster(&tx, &servers)? else {
            return Ok(Err(source_error(
                &evidence.source,
                "The complete roster exceeds the existing SQLite row-length limit or 4 MiB transaction page reservation",
            )));
        };
        tx.execute(
            "INSERT INTO domain_head (domain, version) VALUES (?1, ?2)",
            params![DOMAIN, sql::be(version.get())],
        )
        .map_err(|error| db_error(error, "writing the server roster head"))?;
        tx.execute(
            "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
            [sql::be(version.get())],
        )
        .map_err(|error| db_error(error, "advancing the server roster version"))?;
        sql::insert_import(&tx, &evidence)?;
        if !budget.admits()? {
            return Ok(Err(source_error(&evidence.source,
                "The complete roster and its import receipt exceed the existing transaction page reservation")));
        }
        drop(budget);
        #[cfg(test)]
        if let Some(point) = point {
            point("before-servers-import-commit");
        }
        tx.commit()
            .map_err(|error| db_error(error, "committing the server roster import"))?;
        drop(guard);
        self.point("after-servers-import-commit");
        notices.push((
            Level::Info,
            format!(
                "local state: servers import action={} profiles={} old=0 new={} source={}",
                if evidence.digest.is_some() {
                    "imported"
                } else {
                    "defaults"
                },
                servers.servers.len(),
                version,
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
        Ok(Ok(ServerRoster {
            version,
            servers: Arc::new(servers),
        }))
    }
}

fn read_version(conn: &Connection) -> Result<Option<StoreVersion>, SqlFailure> {
    let raw: Option<Vec<u8>> = conn
        .query_row(
            "SELECT version FROM domain_head WHERE domain = ?1",
            [DOMAIN],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| db_error(error, "reading the server roster version"))?;
    raw.map(|raw| {
        sql::decode_u64(&raw)
            .map(StoreVersion::from_durable)
            .ok_or_else(|| {
                SqlFailure::refused(
                    FailureReason::Schema,
                    "The server roster version is invalid",
                )
            })
    })
    .transpose()
}

fn write_roster<'conn>(
    conn: &'conn Connection,
    servers: &Servers,
) -> Result<Option<ImportBudget<'conn>>, SqlFailure> {
    let mut budget = ImportBudget::new(conn)?;
    let mut profile = conn.prepare("INSERT INTO server_profile
        (name_key, display_name, position, revision, transport, login_key, vault, members, plaintext)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")
        .map_err(|error| db_error(error, "preparing the server profile import"))?;
    let mut world = conn
        .prepare(
            "INSERT INTO server_world
        (profile, position, number, host, port, node_id, asset_host, asset_port)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(|error| db_error(error, "preparing the server world import"))?;
    for (position, item) in servers.servers.iter().enumerate() {
        let key = item.name.to_ascii_lowercase();
        let login = serde_json::to_vec(&item.login_key)
            .map_err(|error| SqlFailure::refused(FailureReason::Schema, error.to_string()))?;
        // SQLITE_LIMIT_LENGTH already reserves 4096 bytes above DOCUMENT_MAX for
        // record headers and numeric fields. Refuse oversized content before binding.
        if key
            .len()
            .saturating_add(item.name.len())
            .saturating_add(login.len())
            .saturating_add(item.vault.len())
            > sql::DOCUMENT_MAX as usize
        {
            return Ok(None);
        }
        profile
            .execute(params![
                key,
                item.name,
                position as i64,
                item.revision,
                match item.transport {
                    LaunchTransport::Tcp => "tcp",
                    LaunchTransport::Wss => "wss",
                },
                login,
                item.vault,
                item.members,
                item.allow_plaintext_offhost
            ])
            .map_err(|error| db_error(error, "importing a server profile"))?;
        if !budget.admits()? {
            return Ok(None);
        }
        for (position, item) in item.worlds.iter().enumerate() {
            if key
                .len()
                .saturating_add(item.host.len())
                .saturating_add(item.asset_host.len())
                > sql::DOCUMENT_MAX as usize
            {
                return Ok(None);
            }
            world
                .execute(params![
                    key,
                    position as i64,
                    item.number,
                    item.host,
                    item.port,
                    item.node_id,
                    item.asset_host,
                    item.asset_port
                ])
                .map_err(|error| db_error(error, "importing a server world"))?;
            if !budget.admits()? {
                return Ok(None);
            }
        }
    }
    Ok(Some(budget))
}

struct ImportBudget<'conn> {
    occupied_before: u64,
    fixed: u64,
    pages: rusqlite::Statement<'conn>,
    free: Option<rusqlite::Statement<'conn>>,
}

impl<'conn> ImportBudget<'conn> {
    fn new(conn: &'conn Connection) -> Result<Self, SqlFailure> {
        // Existing roots and shared metadata can be dirtied without allocating pages.
        let fixed: u64 = conn
            .query_row(
                "SELECT count(*) FROM dbstat WHERE name IN (
                SELECT name FROM sqlite_schema WHERE tbl_name IN
                ('server_profile','server_world','domain_head','import_ledger','store_meta'))",
                [],
                |row| row.get::<_, u32>(0).map(u64::from),
            )
            .map_err(|error| db_error(error, "reserving the server import pages"))?;
        let (occupied_before, free_before) = Self::occupied(conn)?;
        // A freelist trunk contains page_size/4 - 2 entries. Page 1 also changes.
        let freelist_trunks = sql::DB_MAX_PAGES.div_ceil(sql::PAGE_SIZE / 4 - 2);
        Ok(Self {
            occupied_before,
            fixed: fixed + freelist_trunks + 1,
            pages: conn
                .prepare("PRAGMA page_count")
                .map_err(|error| db_error(error, "preparing the server page counter"))?,
            free: if free_before == 0 {
                None
            } else {
                Some(
                    conn.prepare("PRAGMA freelist_count")
                        .map_err(|error| db_error(error, "preparing the server free-page counter"))?,
                )
            },
        })
    }

    fn occupied(conn: &Connection) -> Result<(u64, u64), SqlFailure> {
        let pages: u64 = conn
            .query_row("PRAGMA page_count", [], |row| {
                row.get::<_, u32>(0).map(u64::from)
            })
            .map_err(|error| db_error(error, "counting server import pages"))?;
        let free: u64 = conn
            .query_row("PRAGMA freelist_count", [], |row| {
                row.get::<_, u32>(0).map(u64::from)
            })
            .map_err(|error| db_error(error, "counting free server import pages"))?;
        Ok((pages.saturating_sub(free), free))
    }

    fn admits(&mut self) -> Result<bool, SqlFailure> {
        let pages: u64 = self
            .pages
            .query_row([], |row| row.get::<_, u32>(0).map(u64::from))
            .map_err(|error| db_error(error, "counting server import pages"))?;
        let free: u64 = if let Some(statement) = self.free.as_mut() {
            statement
                .query_row([], |row| row.get::<_, u32>(0).map(u64::from))
                .map_err(|error| db_error(error, "counting free server import pages"))?
        } else {
            0
        };
        // Inserts need no further freelist queries once it is empty. If SQLite
        // releases a page later, page_count alone is a conservative upper bound.
        if free == 0 {
            self.free = None;
        }
        Ok(pages
            .saturating_sub(free)
            .saturating_sub(self.occupied_before)
            + self.fixed
            <= sql::MAX_PAGES)
    }
}

fn read_roster(conn: &Connection, version: StoreVersion) -> Result<ServerRoster, SqlFailure> {
    let mut servers = Servers {
        schema_version: 1,
        servers: Vec::new(),
    };
    let mut statement = conn
        .prepare(
            "SELECT display_name, revision, transport, login_key,
        vault, members, plaintext FROM server_profile ORDER BY position",
        )
        .map_err(|error| db_error(error, "preparing the server roster read"))?;
    let rows = statement
        .query_map([], |row| {
            let transport: String = row.get(2)?;
            let login: Vec<u8> = row.get(3)?;
            let login_key: LoginKey = serde_json::from_slice(&login).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Blob,
                    Box::new(error),
                )
            })?;
            Ok(LaunchProfile {
                name: row.get(0)?,
                revision: row.get(1)?,
                transport: if transport == "tcp" {
                    LaunchTransport::Tcp
                } else {
                    LaunchTransport::Wss
                },
                worlds: Vec::new(),
                login_key,
                vault: row.get(4)?,
                members: row.get(5)?,
                allow_plaintext_offhost: row.get(6)?,
            })
        })
        .map_err(|error| db_error(error, "reading the server roster"))?;
    for row in rows {
        servers
            .servers
            .push(row.map_err(|error| db_error(error, "decoding a server profile"))?);
    }
    let mut statement = conn
        .prepare(
            "SELECT p.position, w.number, w.host, w.port, w.node_id,
        w.asset_host, w.asset_port FROM server_world w JOIN server_profile p ON p.name_key=w.profile
        ORDER BY p.position, w.position",
        )
        .map_err(|error| db_error(error, "preparing the server worlds read"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)? as usize,
                LaunchWorld {
                    number: row.get(1)?,
                    host: row.get(2)?,
                    port: row.get(3)?,
                    node_id: row.get(4)?,
                    asset_host: row.get(5)?,
                    asset_port: row.get(6)?,
                },
            ))
        })
        .map_err(|error| db_error(error, "reading the server worlds"))?;
    for row in rows {
        let (position, world) = row.map_err(|error| db_error(error, "decoding a server world"))?;
        let profile = servers.servers.get_mut(position).ok_or_else(|| {
            SqlFailure::refused(FailureReason::Schema, "The server roster order is invalid")
        })?;
        profile.worlds.push(world);
    }
    servers
        .validate()
        .map_err(|error| SqlFailure::refused(FailureReason::Schema, error))?;
    Ok(ServerRoster {
        version,
        servers: Arc::new(servers),
    })
}
