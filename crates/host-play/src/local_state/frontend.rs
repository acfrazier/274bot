//! What a frontend holds of the process local-state service (T5.2 r3): the
//! operator preferences (`panel-ui.json`, [`Preferences`]) and the operator
//! loadouts its form edits ([`ServiceLoadouts`]). Both are operator-wide
//! stores, one file each for every identity of their domain, so a frontend
//! reads and writes under one fixed operator identity.
//!
//! Every read and write goes through the service's writer thread: nothing
//! here opens a file. Reads re-read the file at most once per
//! [`PREFERENCES_FRESHNESS`] (a peer frontend's write is seen), and writes
//! wait, bounded, until they are durable, as the old synchronous file write
//! did on an operator action.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::identity::Settlement;
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, DurableReceipt, ErrorCode,
    LoadoutEdit, LoadoutOp, LocalStateDomain, LocalStatePort, PreferencesEdit, StoreVersion,
    SubmitReceipt, MAX_PREF_VALUE_BYTES,
};
use parking_lot::Mutex;
use script::local_state::StatePort;
use script::{LoadoutsBacking, LoadoutsBaseline, LoadoutsSaveError};
use serde_json::{Map, Value};

use super::{files, LocalStateService, RequestState};

/// How long a frontend's read or write waits for the writer thread.
pub const FRONTEND_TIMEOUT: Duration = Duration::from_secs(5);

/// How stale the preferences a frontend reads may be: once a document is
/// held, a read older than this asks the writer thread to read the file
/// again (without waiting), and a later read sees the result.
pub const PREFERENCES_FRESHNESS: Duration = Duration::from_secs(1);

/// How many times an update reads and merges again after another writer
/// changed the file under it.
const UPDATE_ATTEMPTS: usize = 4;

/// The identity a frontend reads and writes the operator stores under. The
/// operator stores ignore account and scope (every identity of their
/// domain is the one file), but a durable identity needs both.
fn operator_identity(domain: LocalStateDomain) -> DomainIdentity {
    DomainIdentity::new(
        domain,
        AccountKey::try_new("operator", "operator").expect("a fixed login identity"),
        ContentScope::try_new(client::io::ClientRevision::R289, "operator")
            .expect("a fixed content identity"),
    )
}

/// The operator preferences shared by the panel, the TUI and the
/// composition (walk permissions, the quester Path folder, the session log
/// switch, ...), through the process service. One top-level member of the
/// file is one preference row; an object too long for one row is written
/// member by member. A clone is one reference count: clones share the
/// service handle, the identity and what was last read.
#[derive(Clone)]
pub struct Preferences {
    shared: Arc<Shared>,
}

/// What every clone of one [`Preferences`] holds.
struct Shared {
    service: Arc<LocalStateService>,
    identity: DomainIdentity,
    held: Mutex<Held>,
}

/// The document last read, parsed once per store version.
#[derive(Default)]
struct Held {
    version: Option<StoreVersion>,
    document: Arc<Map<String, Value>>,
    read_at: Option<Instant>,
}

impl Preferences {
    pub(super) fn new(service: Arc<LocalStateService>) -> Self {
        Self {
            shared: Arc::new(Shared {
                service,
                identity: operator_identity(LocalStateDomain::Preferences),
                held: Mutex::default(),
            }),
        }
    }

    /// The service behind these preferences.
    pub(super) fn service(&self) -> &Arc<LocalStateService> {
        &self.shared.service
    }

    /// The operator identity these preferences read and write under.
    #[cfg(test)]
    pub(super) fn identity(&self) -> &DomainIdentity {
        &self.shared.identity
    }

    /// The file behind the store, for messages.
    pub fn path(&self) -> PathBuf {
        self.service().root().preferences_file()
    }

    /// The whole document. The first read (nothing held yet) reads the file
    /// on the writer thread and waits for it, bounded by
    /// [`FRONTEND_TIMEOUT`]. Later reads never wait: they answer with what
    /// the service holds, and when the last read of the file is older than
    /// [`PREFERENCES_FRESHNESS`] they queue a re-read, so a peer process's
    /// write is seen within about that long. Cheap per frame: the document
    /// is parsed once per store version. An absent file is an empty
    /// document; a refused file is an error naming it.
    pub fn document(&self) -> io::Result<Arc<Map<String, Value>>> {
        let shared = &*self.shared;
        let (stale, holding) = {
            let held = shared.held.lock();
            (
                held.read_at
                    .is_none_or(|at| at.elapsed() >= PREFERENCES_FRESHNESS),
                held.version.is_some(),
            )
        };
        if stale {
            if !holding {
                return self.reload();
            }
            shared.service.refresh(&shared.identity);
            shared.held.lock().read_at = Some(Instant::now());
        }
        let version = shared
            .service
            .lease_version(&shared.identity)
            .map_err(|code| self.error(code))?;
        {
            let held = shared.held.lock();
            if held.version == Some(version) {
                return Ok(Arc::clone(&held.document));
            }
        }
        let lease = shared
            .service
            .lease(&shared.identity)
            .map_err(|code| self.error(code))?;
        self.adopt(&lease)
    }

    /// One top-level preference; `None` when it is absent or the store
    /// cannot be read.
    pub fn value(&self, key: &str) -> Option<Value> {
        self.document().ok()?.get(key).cloned()
    }

    /// Read the file again now and wait for it (bounded by
    /// [`FRONTEND_TIMEOUT`]), whatever its freshness: what an explicit
    /// reload, or a test that just wrote the file, needs.
    pub fn reload(&self) -> io::Result<Arc<Map<String, Value>>> {
        let shared = &*self.shared;
        let read = shared.service.readback(&shared.identity, FRONTEND_TIMEOUT);
        // A failed read counts too: the next read retries after the
        // freshness interval, not on every frame.
        shared.held.lock().read_at = Some(Instant::now());
        let (lease, _) = read.map_err(|code| self.error(code))?;
        self.adopt(&lease)
    }

    /// Read the current document from the file, let `merge` change it, and
    /// write each changed top-level preference (an object too long for one
    /// row, each changed member), each against the version the one before
    /// left and each durable before the next. When another writer changed
    /// the file first, read and merge again (a few times at most). A
    /// preference `merge` removes cannot be written and is an error.
    pub fn update(
        &self,
        mut merge: impl FnMut(&mut Map<String, Value>) -> io::Result<()>,
    ) -> io::Result<()> {
        let shared = &*self.shared;
        let mut conflict = None;
        for _ in 0..UPDATE_ATTEMPTS {
            let (lease, _) = shared
                .service
                .readback(&shared.identity, FRONTEND_TIMEOUT)
                .map_err(|code| self.error(code))?;
            let before = self.adopt(&lease)?;
            let mut after = (*before).clone();
            merge(&mut after)?;
            let mut expected = lease.version();
            let mut conflicted = false;
            for (key, value) in preference_edits(&before, &after)? {
                let edit =
                    PreferencesEdit::try_new(shared.identity.clone(), Some(expected), &key, &value)
                        .map_err(|code| self.error(code))?;
                match settle(&shared.service, DomainEdit::Preferences(edit))
                    .map_err(|error| self.wrap(error))?
                {
                    Settled::Durable(receipt) => expected = receipt.version(),
                    Settled::Conflict(code) => {
                        conflict = Some(code);
                        conflicted = true;
                        break;
                    }
                }
            }
            if !conflicted {
                shared.held.lock().read_at = Some(Instant::now());
                return Ok(());
            }
        }
        Err(self.error(conflict.expect("only a conflict retries")))
    }

    /// Set one top-level preference.
    pub fn set(&self, key: &str, value: Value) -> io::Result<()> {
        self.update(|document| {
            document.insert(key.to_string(), value.clone());
            Ok(())
        })
    }

    fn adopt(&self, lease: &DomainLease) -> io::Result<Arc<Map<String, Value>>> {
        let rows = lease.snapshot().as_preferences().unwrap_or(&[]);
        let document = Arc::new(files::preference_document(rows).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("preferences: unreadable rows ({})", self.path().display()),
            )
        })?);
        let mut held = self.shared.held.lock();
        held.version = Some(lease.version());
        held.document = Arc::clone(&document);
        Ok(document)
    }

    fn error(&self, code: ErrorCode) -> io::Error {
        io::Error::new(
            error_kind(code),
            format!("preferences: {code} ({})", self.path().display()),
        )
    }

    fn wrap(&self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            format!("preferences: {error} ({})", self.path().display()),
        )
    }
}

/// The preference writes that turn `before` into `after`: one per changed
/// top-level member, or, for an object too long for one row, one per
/// changed member of it (`<member>.<nested>`). A removal cannot be written.
fn preference_edits(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> io::Result<Vec<(String, String)>> {
    let removal = |key: &str| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("preference {key:?} cannot be removed, only set"),
        )
    };
    if let Some(gone) = before.keys().find(|key| !after.contains_key(*key)) {
        return Err(removal(gone));
    }
    let mut edits = Vec::new();
    for (key, value) in after {
        let old = before.get(key);
        if old == Some(value) {
            continue;
        }
        let text = value.to_string();
        if text.len() <= MAX_PREF_VALUE_BYTES {
            edits.push((key.clone(), text));
            continue;
        }
        let too_long = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("preference {key:?} is too long for the preferences store"),
            )
        };
        let Value::Object(members) = value else {
            return Err(too_long());
        };
        let old = match old {
            None => None,
            Some(Value::Object(old)) => Some(old),
            Some(_) => return Err(too_long()),
        };
        if let Some(gone) = old.and_then(|old| old.keys().find(|key| !members.contains_key(*key))) {
            return Err(removal(&format!("{key}.{gone}")));
        }
        for (member, value) in members {
            if old.and_then(|old| old.get(member)) != Some(value) {
                edits.push((format!("{key}.{member}"), value.to_string()));
            }
        }
    }
    Ok(edits)
}

/// The operator loadouts a frontend's form edits ([`script::LoadoutsStore`]
/// over this backing): loads read the file again on the writer thread, and
/// saves are typed edits against the version the form loaded, each
/// durable before the next.
pub struct ServiceLoadouts {
    service: Arc<LocalStateService>,
    identity: DomainIdentity,
}

impl ServiceLoadouts {
    pub(super) fn new(service: Arc<LocalStateService>) -> Self {
        Self {
            service,
            identity: operator_identity(LocalStateDomain::LoadoutsReferences),
        }
    }

    fn describe(&self, what: impl std::fmt::Display) -> String {
        format!(
            "loadouts: {what} ({})",
            self.service.root().loadouts_file().display()
        )
    }
}

impl LoadoutsBacking for ServiceLoadouts {
    fn load(&self) -> Result<LoadoutsBaseline, String> {
        let (lease, _) = self
            .service
            .readback(&self.identity, FRONTEND_TIMEOUT)
            .map_err(|code| self.describe(code))?;
        Ok(baseline_of(&lease))
    }

    fn commit(
        &self,
        ops: Vec<LoadoutOp>,
        expected: StoreVersion,
    ) -> Result<LoadoutsBaseline, LoadoutsSaveError> {
        let mut expected = expected;
        for op in ops {
            let edit = match op {
                LoadoutOp::Upsert(loadout) => {
                    LoadoutEdit::upsert(self.identity.clone(), Some(expected), loadout)
                }
                LoadoutOp::Remove { name } => {
                    LoadoutEdit::remove(self.identity.clone(), Some(expected), &name)
                }
            }
            .map_err(|code| LoadoutsSaveError::Failed(self.describe(code)))?;
            match settle(&self.service, DomainEdit::Loadout(edit))
                .map_err(|error| LoadoutsSaveError::Failed(self.describe(error)))?
            {
                Settled::Durable(receipt) => expected = receipt.version(),
                Settled::Conflict(code) => {
                    let latest = self.load().map_err(LoadoutsSaveError::Failed)?;
                    return Err(LoadoutsSaveError::Conflict {
                        latest,
                        reason: self.describe(code),
                    });
                }
            }
        }
        let lease = self
            .service
            .lease(&self.identity)
            .map_err(|code| LoadoutsSaveError::Failed(self.describe(code)))?;
        Ok(baseline_of(&lease))
    }
}

fn baseline_of(lease: &DomainLease) -> LoadoutsBaseline {
    LoadoutsBaseline {
        rows: lease.snapshot().as_loadouts().unwrap_or(&[]).to_vec(),
        version: lease.version(),
    }
}

/// How a frontend's write ended, short of an error.
enum Settled {
    Durable(DurableReceipt),
    /// Another writer changed the store first: retry against what it holds.
    Conflict(ErrorCode),
}

/// Submit `edit` and wait, bounded, for its terminal.
fn settle(service: &LocalStateService, edit: DomainEdit) -> io::Result<Settled> {
    let request = match service.submit(edit) {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(code) => {
            return Err(io::Error::new(error_kind(code), code.to_string()));
        }
    };
    match service.wait_request(request, FRONTEND_TIMEOUT) {
        RequestState::Settled(Settlement::Completed(receipt)) => Ok(Settled::Durable(receipt)),
        RequestState::Settled(Settlement::Failed(code @ ErrorCode::Conflict { .. })) => {
            Ok(Settled::Conflict(code))
        }
        RequestState::Settled(Settlement::Failed(code)) => {
            Err(io::Error::new(error_kind(code), code.to_string()))
        }
        RequestState::Settled(Settlement::Cancelled(why)) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("the write was cancelled ({why:?})"),
        )),
        RequestState::Pending => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "the write was not durable within {FRONTEND_TIMEOUT:?}; it stays queued and \
                 settles later"
            ),
        )),
        RequestState::Forgotten => Err(io::Error::other(
            "the service no longer holds the write's settlement",
        )),
    }
}

fn error_kind(code: ErrorCode) -> io::ErrorKind {
    match code {
        ErrorCode::Validation { .. } => io::ErrorKind::InvalidInput,
        ErrorCode::Conflict { .. } => io::ErrorKind::AlreadyExists,
        ErrorCode::NotReady { .. } => io::ErrorKind::TimedOut,
        ErrorCode::Busy { .. } => io::ErrorKind::WouldBlock,
        ErrorCode::Failed { .. } => io::ErrorKind::InvalidData,
        ErrorCode::ProcessService(_) => io::ErrorKind::Other,
    }
}
