//! Root-wide launch roster, hydrated by the process writer before selection.

use std::io;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use super::profiles::{binding_mismatch, component_taken};
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

/// One requested mutation to the root-wide saved server roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerRosterEdit {
    /// Add a new custom server profile to the end of the roster.
    Add(LaunchProfile),
    /// Add a named preset from the shipped catalog through the same checks.
    AddPreset(String),
    /// Replace the existing profile matching `target` (case-insensitive),
    /// preserving its roster position.
    Edit {
        target: String,
        profile: LaunchProfile,
    },
    /// Remove the existing profile matching `target` (case-insensitive).
    /// Any S8 vault binding and profile rows for that server are preserved.
    Remove { target: String },
}

impl ServerRosterEdit {
    pub fn action_label(&self) -> &'static str {
        match self {
            Self::Add(_) => "add",
            Self::AddPreset(_) => "add-preset",
            Self::Edit { .. } => "edit",
            Self::Remove { .. } => "remove",
        }
    }

    pub fn target_label(&self) -> &str {
        match self {
            Self::Add(profile) => &profile.name,
            Self::AddPreset(name) => name,
            Self::Edit { target, .. } | Self::Remove { target } => target,
        }
    }
}

/// Why a server roster edit was refused before commit.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServerRosterMutationError {
    /// Another write changed the roster since the version this edit expected.
    #[error(
        "Server roster changed before this edit ran (expected version {}, found version {}). \
         Reload the roster and retry.",
        expected.get(),
        found.get()
    )]
    Conflict {
        expected: StoreVersion,
        found: StoreVersion,
    },
    /// The named server profile or shipped preset does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The resulting roster failed `Servers::validate`.
    #[error("Invalid server roster: {0}")]
    Validation(String),
    /// S8 server-to-vault binding enforcement refused the edit.
    #[error("{0}")]
    Binding(String),
    /// The resulting roster exceeded the SQLite row or transaction page bound.
    #[error("{0}")]
    Capacity(String),
    /// The local-state service could not complete the edit.
    #[error("{0}")]
    Unavailable(String),
}

type ServerMutationOutcome =
    Result<(StoreVersion, ServerRoster), (ServerRosterMutationError, Option<ServerRoster>)>;

/// Non-blocking handle for one submitted server roster mutation.
/// Frame and slot threads poll [`Self::try_take`]; startup/CLI/test callers
/// may wait with [`Self::wait`].
pub struct ServerRosterMutation {
    request: RequestId,
    rx: mpsc::Receiver<Result<ServerRoster, ServerRosterMutationError>>,
}

impl ServerRosterMutation {
    /// The host request identity minted for this mutation.
    pub fn request(&self) -> RequestId {
        self.request
    }

    /// Poll for completion without blocking the caller thread.
    pub fn try_take(&self) -> Option<Result<ServerRoster, ServerRosterMutationError>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                ServerRosterMutationError::Unavailable("The state service has stopped".into()),
            )),
        }
    }

    /// Wait on a startup/control/CLI thread, never on a frame or slot thread.
    pub fn wait(&self, timeout: Duration) -> Result<ServerRoster, ServerRosterMutationError> {
        match self.rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ServerRosterMutationError::Unavailable(
                "Timed out waiting for the server roster edit".into(),
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(
                ServerRosterMutationError::Unavailable("The state service has stopped".into()),
            ),
        }
    }
}

pub(super) struct ServerEditJob {
    pub(super) request: RequestId,
    pub(super) expected: StoreVersion,
    pub(super) edit: ServerRosterEdit,
    pub(super) retry: Retry,
    reply: mpsc::SyncSender<Result<ServerRoster, ServerRosterMutationError>>,
}

impl ServerEditJob {
    pub(super) fn fail(&self, detail: impl Into<String>) {
        let _ = self
            .reply
            .try_send(Err(ServerRosterMutationError::Unavailable(detail.into())));
    }
}

#[derive(Default)]
pub(super) struct ServerState {
    snapshot: Option<ServerRoster>,
    refusal: Option<ServerRosterError>,
    pub(super) queued: bool,
    pub(super) retry: Retry,
    pub(super) edits: VecDeque<ServerEditJob>,
    notices: Vec<String>,
}

impl ServerState {
    pub(super) fn reject(&mut self, detail: impl Into<String>) {
        self.snapshot = None;
        self.queued = false;
        self.refusal = Some(ServerRosterError::Refused(detail.into()));
    }

    pub(super) fn abandon(&mut self) {
        for job in self.edits.drain(..) {
            job.fail("The state service has stopped");
        }
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

    /// Queue one atomic CAS roster edit on the single writer thread without
    /// blocking the caller. Frame and slot threads poll the returned handle.
    pub fn submit_server_roster_edit(
        &self,
        expected: StoreVersion,
        edit: ServerRosterEdit,
    ) -> Result<ServerRosterMutation, ServerRosterMutationError> {
        let request = self.shared.minter.mint().map_err(|_| {
            ServerRosterMutationError::Unavailable("Host request identities are exhausted".into())
        })?;
        let (reply, rx) = mpsc::sync_channel(1);
        let mut state = self.shared.state.lock();
        if state.servers.edits.len() >= OPERATOR_QUEUE_CAP {
            return Err(ServerRosterMutationError::Unavailable(
                "The server roster edit queue is full; retry after pending edits finish".into(),
            ));
        }
        if state.shutdown || !self.shared.start_writer(&mut state) {
            return Err(ServerRosterMutationError::Unavailable(
                "The state service has stopped".into(),
            ));
        }
        state.servers.edits.push_back(ServerEditJob {
            request,
            expected,
            edit,
            retry: Retry::default(),
            reply,
        });
        self.shared.wake.notify_one();
        Ok(ServerRosterMutation { request, rx })
    }

    /// Non-blocking add of a custom server profile.
    pub fn submit_add_server(
        &self,
        expected: StoreVersion,
        profile: LaunchProfile,
    ) -> Result<ServerRosterMutation, ServerRosterMutationError> {
        self.submit_server_roster_edit(expected, ServerRosterEdit::Add(profile))
    }

    /// Non-blocking add of a shipped preset by name.
    pub fn submit_add_server_preset(
        &self,
        expected: StoreVersion,
        preset: impl Into<String>,
    ) -> Result<ServerRosterMutation, ServerRosterMutationError> {
        self.submit_server_roster_edit(expected, ServerRosterEdit::AddPreset(preset.into()))
    }

    /// Non-blocking edit of an existing server profile.
    pub fn submit_update_server(
        &self,
        expected: StoreVersion,
        target: impl Into<String>,
        profile: LaunchProfile,
    ) -> Result<ServerRosterMutation, ServerRosterMutationError> {
        self.submit_server_roster_edit(
            expected,
            ServerRosterEdit::Edit {
                target: target.into(),
                profile,
            },
        )
    }

    /// Non-blocking removal of an existing server profile.
    pub fn submit_remove_server(
        &self,
        expected: StoreVersion,
        target: impl Into<String>,
    ) -> Result<ServerRosterMutation, ServerRosterMutationError> {
        self.submit_server_roster_edit(
            expected,
            ServerRosterEdit::Remove {
                target: target.into(),
            },
        )
    }

    /// Run one atomic CAS roster edit and wait on a startup/CLI/control thread.
    /// Never call from a frame or slot thread.
    pub fn edit_server_roster(
        &self,
        expected: StoreVersion,
        edit: ServerRosterEdit,
        timeout: Duration,
    ) -> Result<ServerRoster, ServerRosterMutationError> {
        self.submit_server_roster_edit(expected, edit)?
            .wait(timeout)
    }

    /// Blocking custom-server add (startup/CLI/control threads only).
    pub fn add_server(
        &self,
        expected: StoreVersion,
        profile: LaunchProfile,
        timeout: Duration,
    ) -> Result<ServerRoster, ServerRosterMutationError> {
        self.edit_server_roster(expected, ServerRosterEdit::Add(profile), timeout)
    }

    /// Blocking shipped-preset add (startup/CLI/control threads only).
    pub fn add_server_preset(
        &self,
        expected: StoreVersion,
        preset: impl Into<String>,
        timeout: Duration,
    ) -> Result<ServerRoster, ServerRosterMutationError> {
        self.edit_server_roster(
            expected,
            ServerRosterEdit::AddPreset(preset.into()),
            timeout,
        )
    }

    /// Blocking server edit (startup/CLI/control threads only).
    pub fn update_server(
        &self,
        expected: StoreVersion,
        target: impl Into<String>,
        profile: LaunchProfile,
        timeout: Duration,
    ) -> Result<ServerRoster, ServerRosterMutationError> {
        self.edit_server_roster(
            expected,
            ServerRosterEdit::Edit {
                target: target.into(),
                profile,
            },
            timeout,
        )
    }

    /// Blocking server removal (startup/CLI/control threads only).
    pub fn remove_server(
        &self,
        expected: StoreVersion,
        target: impl Into<String>,
        timeout: Duration,
    ) -> Result<ServerRoster, ServerRosterMutationError> {
        self.edit_server_roster(
            expected,
            ServerRosterEdit::Remove {
                target: target.into(),
            },
            timeout,
        )
    }
}

impl Shared {
    pub(super) fn hydrate_servers(&self, db: &mut Db) -> Option<WaitCause> {
        #[cfg(any(test, feature = "test-support"))]
        {
            let stall = self.state.lock().read_stall.clone();
            if let Some(stall) = stall {
                stall();
            }
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

    pub(super) fn mutate_servers(&self, db: &mut Db, job: &ServerEditJob) -> Option<WaitCause> {
        #[cfg(any(test, feature = "test-support"))]
        {
            let stall = self.state.lock().write_stall.clone();
            if let Some(stall) = stall {
                stall();
            }
        }
        let clock = self.state.lock().clock;
        let outcome = match db.mutate_servers(job.expected, &job.edit, clock) {
            Ok(outcome) => outcome,
            Err(SqlFailure::Wait(cause)) => return Some(cause),
            Err(SqlFailure::Refused { detail, .. }) => {
                let _ = job
                    .reply
                    .try_send(Err(ServerRosterMutationError::Unavailable(
                        detail.to_string(),
                    )));
                return None;
            }
        };
        let truncate_due = db.wal_truncate_due();
        match outcome {
            Ok((previous, roster)) => {
                let version = roster.version();
                {
                    let mut state = self.state.lock();
                    state.truncate.due |= truncate_due;
                    state.clock = state.clock.max(Some(version));
                    state.servers.refusal = None;
                    state.servers.snapshot = Some(roster.clone());
                }
                let now_unix = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let audit = AuditCommit::after_commit(
                    frontend::operator_identity(LocalStateDomain::Servers),
                    CommitAction::ServersWrite,
                    previous,
                    version,
                    AuditOrigin::Submitted {
                        request: job.request,
                    },
                    DurableTimestamp::from_unix_seconds(now_unix),
                );
                self.audit(audit, &AuditKind::Plain);
                self.notice(
                    Level::Info,
                    format_args!(
                        "local state: servers edit action={} target={} profiles={} old={} new={} request={}",
                        job.edit.action_label(),
                        job.edit.target_label(),
                        roster.servers().servers.len(),
                        previous.get(),
                        version.get(),
                        job.request.get()
                    ),
                );
                let _ = job.reply.try_send(Ok(roster));
            }
            Err((error, latest)) => {
                let mut state = self.state.lock();
                state.truncate.due |= truncate_due;
                if let Some(latest) = latest {
                    state.clock = state.clock.max(Some(latest.version()));
                    state.servers.snapshot = Some(latest);
                }
                drop(state);
                let _ = job.reply.try_send(Err(error));
            }
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

    fn mutate_servers(
        &mut self,
        expected: StoreVersion,
        edit: &ServerRosterEdit,
        clock: Option<StoreVersion>,
    ) -> Result<ServerMutationOutcome, SqlFailure> {
        self.open_for_domain()?;
        let mut result = self.mutate_servers_open(expected, edit, clock);
        self.after_failure(&mut result);
        result
    }

    fn mutate_servers_open(
        &mut self,
        expected: StoreVersion,
        edit: &ServerRosterEdit,
        clock: Option<StoreVersion>,
    ) -> Result<ServerMutationOutcome, SqlFailure> {
        self.ensure_root_use()?;
        let guard = self.guard()?;
        let conn = self.conn.as_mut().ok_or_else(sql::closed)?;
        let tx = sql::begin_write(conn, &self.root, &guard, "editing the server roster")?;
        let Some(current_version) = read_version(&tx)? else {
            return Ok(Err((
                ServerRosterMutationError::Unavailable(
                    "Server profiles have not been initialized yet".into(),
                ),
                None,
            )));
        };
        let current_roster = read_roster(&tx, current_version)?;
        if current_version != expected {
            return Ok(Err((
                ServerRosterMutationError::Conflict {
                    expected,
                    found: current_version,
                },
                Some(current_roster),
            )));
        }
        let mut candidate = current_roster.servers().clone();
        let mut edited_old: Option<LaunchProfile> = None;
        let mut added_profile: Option<LaunchProfile> = None;
        match edit {
            ServerRosterEdit::Add(profile) => {
                candidate.servers.push(profile.clone());
                added_profile = Some(profile.clone());
            }
            ServerRosterEdit::AddPreset(name) => {
                let Some(preset) = Servers::shipped_preset(name) else {
                    return Ok(Err((
                        ServerRosterMutationError::NotFound(format!(
                            "Unknown shipped server preset {name:?}"
                        )),
                        None,
                    )));
                };
                candidate.servers.push(preset.clone());
                added_profile = Some(preset);
            }
            ServerRosterEdit::Edit { target, profile } => {
                let Some(index) = candidate
                    .servers
                    .iter()
                    .position(|existing| existing.name.eq_ignore_ascii_case(target))
                else {
                    return Ok(Err((
                        ServerRosterMutationError::NotFound(format!(
                            "Unknown server profile {target:?} in the durable server roster"
                        )),
                        None,
                    )));
                };
                edited_old = Some(candidate.servers[index].clone());
                candidate.servers[index] = profile.clone();
            }
            ServerRosterEdit::Remove { target } => {
                let Some(index) = candidate
                    .servers
                    .iter()
                    .position(|existing| existing.name.eq_ignore_ascii_case(target))
                else {
                    return Ok(Err((
                        ServerRosterMutationError::NotFound(format!(
                            "Unknown server profile {target:?} in the durable server roster"
                        )),
                        None,
                    )));
                };
                candidate.servers.remove(index);
            }
        }
        if let Err(error) = candidate.validate() {
            return Ok(Err((ServerRosterMutationError::Validation(error), None)));
        }
        if let Err(error) =
            enforce_s8_bindings(&tx, edit, edited_old.as_ref(), added_profile.as_ref())?
        {
            return Ok(Err((ServerRosterMutationError::Binding(error), None)));
        }
        let next_version = sql::next_version(&tx, clock)?;
        let mut budget = ImportBudget::new(&tx)?;
        tx.execute("DELETE FROM server_world", [])
            .map_err(|error| db_error(error, "clearing server worlds for roster edit"))?;
        tx.execute("DELETE FROM server_profile", [])
            .map_err(|error| db_error(error, "clearing server profiles for roster edit"))?;
        if !write_roster_rows(&tx, &candidate, &mut budget)? {
            return Ok(Err((
                ServerRosterMutationError::Capacity(
                    "The complete roster exceeds the existing SQLite row-length limit or 4 MiB \
                     transaction page reservation"
                        .into(),
                ),
                None,
            )));
        }
        tx.execute(
            "UPDATE domain_head SET version = ?2 WHERE domain = ?1",
            params![DOMAIN, sql::be(next_version.get())],
        )
        .map_err(|error| db_error(error, "updating the server roster head"))?;
        tx.execute(
            "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
            [sql::be(next_version.get())],
        )
        .map_err(|error| db_error(error, "advancing the server roster version"))?;
        if !budget.admits()? {
            return Ok(Err((
                ServerRosterMutationError::Capacity(
                    "The server roster edit exceeds the existing transaction page reservation"
                        .into(),
                ),
                None,
            )));
        }
        drop(budget);
        tx.commit()
            .map_err(|error| db_error(error, "committing the server roster edit"))?;
        drop(guard);
        Ok(Ok((
            current_version,
            ServerRoster {
                version: next_version,
                servers: Arc::new(candidate),
            },
        )))
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

fn binding_component_of(conn: &Connection, server_key: &str) -> Result<Option<String>, SqlFailure> {
    conn.query_row(
        "SELECT component FROM vault_binding WHERE server = ?1",
        params![server_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| db_error(error, "reading the server vault binding"))
}

fn binding_owner_of(conn: &Connection, component_key: &str) -> Result<Option<String>, SqlFailure> {
    conn.query_row(
        "SELECT server FROM vault_binding WHERE component = ?1",
        params![component_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| db_error(error, "reading the vault component binding"))
}

fn has_pending_vault_work(conn: &Connection, server_key: &str) -> Result<bool, SqlFailure> {
    conn.query_row(
        "SELECT 1 FROM vault_pending WHERE server = ?1 LIMIT 1",
        params![server_key],
        |_| Ok(true),
    )
    .optional()
    .map(|found| found.unwrap_or(false))
    .map_err(|error| db_error(error, "reading pending server vault writes"))
}

/// Enforce S8 permanent server-to-vault bindings inside the roster write transaction.
fn enforce_s8_bindings(
    conn: &Connection,
    edit: &ServerRosterEdit,
    edited_old: Option<&LaunchProfile>,
    added_profile: Option<&LaunchProfile>,
) -> Result<Result<(), String>, SqlFailure> {
    const PERMANENT_REMEDY: &str =
        "Vault bindings are permanent; add a new server name and a new vault component instead.";
    match edit {
        ServerRosterEdit::Remove { .. } => Ok(Ok(())),
        ServerRosterEdit::Add(_) | ServerRosterEdit::AddPreset(_) => {
            let profile = added_profile.expect("added profile resolved");
            let key = profile.name.to_ascii_lowercase();
            let component = profile.vault.to_ascii_lowercase();
            if let Some(bound) = binding_component_of(conn, &key)? {
                if bound != component {
                    return Ok(Err(format!(
                        "{} Re-adding {} requires vault component {bound:?}. {PERMANENT_REMEDY}",
                        binding_mismatch(&profile.name, &component, &bound),
                        profile.name
                    )));
                }
            }
            if let Some(owner) = binding_owner_of(conn, &component)? {
                if owner != key {
                    return Ok(Err(format!(
                        "{} {PERMANENT_REMEDY}",
                        component_taken(&profile.name, &component, &owner)
                    )));
                }
            }
            Ok(Ok(()))
        }
        ServerRosterEdit::Edit { profile, .. } => {
            let old = edited_old.expect("edited profile resolved");
            let old_key = old.name.to_ascii_lowercase();
            let old_component = old.vault.to_ascii_lowercase();
            let new_key = profile.name.to_ascii_lowercase();
            let new_component = profile.vault.to_ascii_lowercase();
            let old_bound = binding_component_of(conn, &old_key)?;
            let old_pending = has_pending_vault_work(conn, &old_key)?;
            if old_bound.is_some() || old_pending {
                if new_key != old_key {
                    return Ok(Err(format!(
                        "Server profile {:?} is bound to a vault (or has a pending vault write) \
                         and cannot be renamed to {:?}. {PERMANENT_REMEDY}",
                        old.name, profile.name
                    )));
                }
                if let Some(bound) = &old_bound {
                    if *bound != new_component {
                        return Ok(Err(format!(
                            "{} {PERMANENT_REMEDY}",
                            binding_mismatch(&profile.name, &new_component, bound)
                        )));
                    }
                }
                if old_pending && new_component != old_component {
                    return Ok(Err(format!(
                        "Server profile {:?} has a pending vault write and cannot change its vault \
                         component from {old_component:?} to {new_component:?}. {PERMANENT_REMEDY}",
                        old.name
                    )));
                }
            }
            if new_key != old_key {
                if let Some(bound) = binding_component_of(conn, &new_key)? {
                    if bound != new_component {
                        return Ok(Err(format!(
                            "{} Re-using server name {:?} requires vault component {bound:?}. \
                             {PERMANENT_REMEDY}",
                            binding_mismatch(&profile.name, &new_component, &bound),
                            profile.name
                        )));
                    }
                } else if has_pending_vault_work(conn, &new_key)? {
                    return Ok(Err(format!(
                        "Server profile {:?} has a pending vault write and cannot be claimed by \
                         renaming {:?}. {PERMANENT_REMEDY}",
                        profile.name, old.name
                    )));
                }
            }
            if let Some(owner) = binding_owner_of(conn, &new_component)? {
                if owner != new_key {
                    return Ok(Err(format!(
                        "{} {PERMANENT_REMEDY}",
                        component_taken(&profile.name, &new_component, &owner)
                    )));
                }
            }
            Ok(Ok(()))
        }
    }
}

fn write_roster<'conn>(
    conn: &'conn Connection,
    servers: &Servers,
) -> Result<Option<ImportBudget<'conn>>, SqlFailure> {
    let mut budget = ImportBudget::new(conn)?;
    if write_roster_rows(conn, servers, &mut budget)? {
        Ok(Some(budget))
    } else {
        Ok(None)
    }
}

fn write_roster_rows(
    conn: &Connection,
    servers: &Servers,
    budget: &mut ImportBudget<'_>,
) -> Result<bool, SqlFailure> {
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
            return Ok(false);
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
            return Ok(false);
        }
        for (position, item) in item.worlds.iter().enumerate() {
            if key
                .len()
                .saturating_add(item.host.len())
                .saturating_add(item.asset_host.len())
                > sql::DOCUMENT_MAX as usize
            {
                return Ok(false);
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
                return Ok(false);
            }
        }
    }
    Ok(true)
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
                    conn.prepare("PRAGMA freelist_count").map_err(|error| {
                        db_error(error, "preparing the server free-page counter")
                    })?,
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
