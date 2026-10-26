//! The process local-state service (T5.2, `I-state-port`): the functional
//! [`LocalStatePort`] over the current files (`21-census-ownership-dag.md`
//! §4.2–§4.3, D10).
//!
//! One [`LocalStateService`] per process, created by the composition root
//! (a frontend's session, or the `Play` of a headless host) with an
//! explicit [`StateRoot`] and the set of domains it opens. It owns one
//! `local-state-writer` thread, which does every file read and write: cold
//! hydration, readbacks, and durable publication. Nothing here reads
//! `HOME`, and no caller thread ever touches a file.
//!
//! - **Leases** ([`LocalStatePort::lease`]) are the immutable owned
//!   snapshot the service last read or wrote, at the store's version. A
//!   cold store answers [`ErrorCode::NotReady`] and queues its hydration;
//!   no disk read happens on the caller's thread. A file the service
//!   refused to read (unparseable, another schema, another identity or
//!   content scope, unreadable, larger than its bound) is never an empty
//!   store: its leases and submissions answer [`ErrorCode::Failed`] with the
//!   bounded reason, and the file is never written, until a later read
//!   accepts it. A file whose content the service cannot hold whole is read
//!   but never rewritten, in every domain. A loadouts file whose rows share
//!   a name (an older editor could save one) is renamed apart by the read
//!   that finds it, before any lease or write, under the file's lock and
//!   losing nothing ([`Shared::dedupe_loadouts`]); while that cannot be
//!   done the file is refused.
//! - **Submissions** ([`LocalStatePort::submit`]) are nonblocking: a bank
//!   hint replaces a queued hint of the same account (the replaced request
//!   settles `Cancelled(Superseded)`, and the service remembers which
//!   request replaced it); loadout and preference edits queue in order
//!   behind a small cap and refuse `Busy` beyond it, and a loadout edit that
//!   would take the file past its row or byte budget is refused as a
//!   validation error. Acceptance is not durability.
//! - **Writes** are one transaction under the file's cross-process
//!   [`WriteLock`](files::WriteLock): every writer this product runs (this
//!   service, another composition's, another process's on the same `HOME`)
//!   holds it from the precondition read to the final check, so none
//!   interleaves with another. The write re-reads the file first; if it
//!   changed since the service last read or wrote it (a hand edit, an older
//!   build), or changed after the edit was accepted, the edit settles as a
//!   [`ErrorCode::Conflict`] at the version the service now holds for what
//!   it found, and the file is left as it is. The bytes are staged and
//!   flushed, the file is checked once more immediately before the rename,
//!   the directory entries are flushed (the whole chain up to the root's
//!   parent unless this service already made the file's entry durable),
//!   and the file is read back: only when it still holds the bytes written
//!   is the write acknowledged; otherwise it settles `Conflict`, never
//!   `Completed`. A write whose bytes equal what this service already made
//!   durable settles `Completed` at the current version without touching
//!   the disk.
//! - **Settlements** ([`LocalStatePort::observe`]) are kept per store
//!   identity, a few each: `Completed` with the [`DurableReceipt`] once
//!   durable, `Failed` with a typed [`ErrorCode`], or `Cancelled`. A
//!   request whose settlement the service no longer holds is
//!   [`RequestState::Forgotten`], never pending forever. Every accepted
//!   request settles exactly once.
//! - **Audit:** one `host_log!` info line ([`AuditCommit`]) per durable
//!   write, after the bytes and their directory entries are durable and
//!   read back; one `terminal` line per accepted write that ends any other
//!   way (failed, conflicted, cancelled). A repair the service makes on its
//!   own (a malformed preferences file moved aside, loadouts renamed apart)
//!   has one info audit line of its own and a warning for the operator.
//! - **Budget:** at most [`MAX_BANK_STORES`] bank-hint identities are held;
//!   a new one evicts the least recently used idle one, or is refused
//!   `Busy` when none is idle. A lease is one bounded snapshot.
//! - **Shutdown:** dropping the last handle stops new work and lets the
//!   writer finish every accepted write within [`SHUTDOWN_TIMEOUT`]. Past
//!   that bound every accepted write not yet being published settles
//!   `Cancelled(Shutdown)` with its own terminal line, and a write already
//!   being renamed into place settles when it finishes.
//!
//! Versions come from one clock per service: every store change takes the
//! next value, so a version is never reused, even by a store that was
//! evicted and read again. The current files carry no version; expected-
//! version writes guard this process's own races, and the write
//! precondition (the file's content digest, checked under the lock and
//! again before the rename) guards against other writers.
//!
//! **Residual window.** A writer that does not take the lock (a text
//! editor, a build older than T5.2 r3) is caught by the checks, except in
//! two instants: between the last pre-rename check and the `rename(2)`
//! itself (a few system calls), where its write is replaced by this one;
//! and an in-place save that opened the file before the rename and writes
//! after it, which lands in the replaced (unlinked) file. A replacement
//! after the final read-back is a later change, not a false receipt: the
//! acknowledged bytes were durable, and the next write sees the change.
//!
//! Host-only entry points that need a fresh file read submit bounded
//! writer-thread readbacks where the old code read synchronously: script
//! Start ([`LocalStateService::readback_lease`]), a session boundary and a
//! loadout form. Preferences read their cached document; a stale cache queues
//! a refresh without waiting, and accepted patches settle separately.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Cancellation, DurableTimestamp, RequestId, RequestIdMinter, Settlement};
use api::loadout::Loadout;
use api::local_state::{
    AccountKey, AuditCommit, AuditOrigin, CommitAction, ContentScope, DomainEdit, DomainIdentity,
    DomainLease, DurableReceipt, ErrorCode, FailureReason, IdentityError, LeaseSnapshot, LoadoutOp,
    LocalStateDomain, LocalStatePort, StoreVersion, SubmitReceipt, ValidationReason,
    WriteSettlement,
};
use parking_lot::{Condvar, Mutex};
use script::local_state::{SlotState, StatePort};

mod files;
mod frontend;

use files::{ContentDigest, LockError, ReadDocument, ReadError, RenameRefusal, Rewrite, StoreKey};
pub use files::{StateRoot, HINT_SCHEMA_VERSION, MAX_PREFERENCES_BYTES};
pub use frontend::{
    PreferenceError, PreferencePatch, PreferenceSnapshot, Preferences, QueuedUpdate,
    ServiceLoadouts, Update,
};

/// How many edits of one operator store (loadouts, preferences) may wait
/// behind the one in flight.
const OPERATOR_QUEUE_CAP: usize = 8;
/// Live preference requests and unprojected results, separate from neutral history.
const PREFERENCE_WATCH_CAP: usize = OPERATOR_QUEUE_CAP + 1;
/// How many bank-hint identities the service holds at once. A new one
/// evicts the least recently used idle store (nothing queued, in flight
/// or being read); with none idle it is refused `Busy`.
pub const MAX_BANK_STORES: usize = 256;

/// How many settlements each store keeps for its observers: a slot reads
/// its own within frames, and one account has at most one hint in flight
/// and one queued.
const SETTLED_PER_STORE: usize = 8;

/// How long a script Start waits for the loadouts readback: the same
/// synchronous file read the Start did before, now on the writer thread.
pub const START_LEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long dropping the service waits for the writer to finish accepted writes
/// and any readback already in progress. Exit waits while the writer runs.
/// Losing a save is worse than a slower exit.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

/// A front-end test's stall at the start of each file write
/// ([`LocalStateService::set_write_stall_for_test`]).
#[cfg(any(test, feature = "test-support"))]
pub type WriteStall = Arc<dyn Fn() + Send + Sync>;
/// A test's decision, at the start of each file write, to fail it
/// ([`LocalStateService::set_write_fault_for_test`]).
#[cfg(any(test, feature = "test-support"))]
pub type WriteFault = Arc<dyn Fn() -> bool + Send + Sync>;
/// A test's stall before the writer starts a store hydration or readback.
#[cfg(any(test, feature = "test-support"))]
pub type ReadStall = Arc<dyn Fn() + Send + Sync>;

/// A test's stand-in for the file publication step: run on the writer
/// thread with no state lock held (the file's write lock is held), so a
/// test can stall or fail it. It replaces the real publication, including
/// its last check.
#[cfg(test)]
pub(crate) type WriteHook = Arc<dyn Fn(&Path, &[u8]) -> std::io::Result<()> + Send + Sync>;

/// A test's hand inside the real publication: run once the bytes are
/// staged and flushed, immediately before the last check and the rename,
/// with the file's write lock held and no state lock.
#[cfg(test)]
pub(crate) type PublishHook = Arc<dyn Fn(&Path) + Send + Sync>;

/// A test's stand-in for one directory flush.
#[cfg(test)]
pub(crate) type SyncHook = Arc<dyn Fn(&Path) -> std::io::Result<()> + Send + Sync>;

/// A test's witness of each audit line, called where the line is logged.
#[cfg(any(test, feature = "test-support"))]
pub type AuditHook = Arc<dyn Fn(&AuditCommit) + Send + Sync>;

/// A test's witness of each terminal line, called where the line is logged.
#[cfg(test)]
pub(crate) type TerminalHook = Arc<dyn Fn(&str) + Send + Sync>;

/// A test's witness of each line of a repair (a preferences quarantine,
/// loadouts renamed apart): the operator's notice, the audit line, or why
/// the repair could not be made. Called where the line is logged.
#[cfg(test)]
pub(crate) type NoticeHook = Arc<dyn Fn(Level, &str) + Send + Sync>;

/// What one `Play` composes for its slots (§21 §4.2): the process service,
/// the launch profile's namespace every account keys under, and the
/// content scope observations are made under. Absent for the legacy
/// direct connection, which has no launch profile and so opens no state
/// domains: its slots plan with explicitly absent operator rows and keep
/// their bank memory for the process only, as before.
pub struct StateComposition {
    service: Arc<LocalStateService>,
    namespace: String,
    scope: ContentScope,
}

impl StateComposition {
    /// Compose `service` for accounts under `namespace` (the launch profile
    /// name) observing under `scope`.
    pub fn new(service: Arc<LocalStateService>, namespace: &str, scope: ContentScope) -> Self {
        Self {
            service,
            namespace: namespace.to_string(),
            scope,
        }
    }

    pub fn service(&self) -> &Arc<LocalStateService> {
        &self.service
    }

    /// The same namespace and scope over another `service`: a frontend's
    /// process service replacing the one a `Play` opened for itself.
    pub fn with_service(&self, service: Arc<LocalStateService>) -> Self {
        Self {
            service,
            namespace: self.namespace.clone(),
            scope: self.scope.clone(),
        }
    }

    pub fn scope(&self) -> &ContentScope {
        &self.scope
    }

    /// The durable account key of the slot named `username`, or why the
    /// name is not one (a name the server would refuse at login).
    pub fn account(&self, username: &str) -> Result<AccountKey, IdentityError> {
        AccountKey::try_new(&self.namespace, username)
    }

    /// The durable identity of `domain` for the slot named `username`.
    pub fn identity(
        &self,
        domain: LocalStateDomain,
        username: &str,
    ) -> Result<DomainIdentity, IdentityError> {
        self.account(username)
            .map(|account| DomainIdentity::new(domain, account, self.scope.clone()))
    }

    /// The capability the slot named `username` is bound with, or `None`
    /// (with one warning line) when the name is not a login identity: such
    /// a slot never logs in, so nothing is observed under it.
    pub fn slot_state(&self, username: &str) -> Option<Arc<SlotState>> {
        match self.account(username) {
            Ok(account) => Some(Arc::new(SlotState::new(
                Arc::clone(&self.service) as Arc<dyn StatePort>,
                account,
                self.scope.clone(),
            ))),
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    slot = username,
                    "local state: {error}; the slot has no state capability"
                );
                None
            }
        }
    }

    /// The operator's current loadout rows for a Start on `username`'s
    /// slot: a readback of the file (an edit by the frontend's form is
    /// seen, and the service's cache is refreshed for the run's later
    /// leases), bounded by [`START_LEASE_TIMEOUT`]. No file is no rows. A
    /// name that is not a login identity, a refused file, or a domain that
    /// did not answer in time refuses with its typed reason.
    pub fn operator_loadouts(&self, username: &str) -> Result<Vec<Loadout>, String> {
        let identity = self
            .identity(LocalStateDomain::LoadoutsReferences, username)
            .map_err(|error| format!("operator loadouts: {error}"))?;
        match self.service.readback_lease(&identity, START_LEASE_TIMEOUT) {
            Ok(Some(lease)) => Ok(lease
                .snapshot()
                .as_loadouts()
                .map(<[Loadout]>::to_vec)
                .unwrap_or_default()),
            Ok(None) => Ok(Vec::new()),
            Err(code) => Err(format!(
                "operator loadouts: {code} ({})",
                self.service.root().loadouts_file().display()
            )),
        }
    }
}

/// Where an accepted request stands, for host-side observers that must
/// tell "still pending" from "no longer held".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    /// Queued or in flight.
    Pending,
    /// Its terminal.
    Settled(WriteSettlement),
    /// The service no longer holds its settlement (its store was evicted,
    /// or newer settlements of the store displaced it), or never accepted
    /// it. The outcome is unknown: treat the work as not saved.
    Forgotten,
}

/// A queued re-read of one store's file
/// ([`LocalStateService::start_readback`]): the completed-readback count
/// that answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadbackTicket {
    target: u64,
}

/// Whether an accepted request's rows reached the disk, following the
/// requests that superseded it: a replaced hint is durable only when the
/// request that replaced it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Durability {
    /// The request, or the newest request that replaced it, is durable.
    Durable(DurableReceipt),
    /// It, or its replacement, settled without becoming durable.
    NotDurable(WriteSettlement),
    /// Still queued or in flight when the wait ended.
    Pending,
    /// The service no longer holds the settlement: unknown.
    Forgotten,
}

/// The process service: the port plus the host-only bounded waits. Cheap
/// to share as `Arc`; dropping the last handle drains and joins the writer.
pub struct LocalStateService {
    shared: Arc<Shared>,
}

struct Shared {
    root: StateRoot,
    open: [bool; 4],
    minter: Arc<RequestIdMinter>,
    preference_path: Arc<str>,
    preference_not_ready: PreferenceError,
    state: Mutex<State>,
    /// Signalled on every submission and readback request: the writer
    /// thread parks on it while nothing is queued.
    wake: Condvar,
    /// Signalled whenever a settlement lands, a readback completes, or the
    /// writer stops: the host-only waits park on it.
    settled: Condvar,
    /// The writer thread, joined when the service is dropped.
    writer: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PreferenceWatchPoll {
    Pending,
    Settled(WriteSettlement),
    Missing,
}

struct PreferenceWatch {
    settlement: Option<WriteSettlement>,
    queued_bytes: usize,
}

#[derive(Default)]
struct State {
    stores: HashMap<StoreKey, Store>,
    /// Every accepted request whose settlement is not yet forgotten, and
    /// the store it belongs to.
    requests: HashMap<RequestId, StoreKey>,
    /// At most nine live requests or terminals, retained until frontend projection.
    preference_watches: HashMap<RequestId, PreferenceWatch>,
    /// Accepted preference intent bytes until terminal settlement.
    preference_queue_bytes: usize,
    /// The default document/view is prepared before the writer starts.
    preference_default: files::PreferenceProjection,
    /// The last version issued, by any store.
    clock: Option<StoreVersion>,
    /// Orders readbacks and writes (FIFO across stores) and stamps store
    /// use (least recently used eviction).
    seq: u64,
    writer: WriterPhase,
    shutdown: bool,
    /// The shutdown bound passed: every accepted write not yet being
    /// published was cancelled, and the writer publishes nothing more.
    abandoned: bool,
    /// How long dropping the service waits ([`SHUTDOWN_TIMEOUT`] unless a
    /// test shortens it).
    #[cfg(test)]
    shutdown_timeout: Option<Duration>,
    #[cfg(any(test, feature = "test-support"))]
    hooks: Hooks,
    /// A front end's test stall at the start of each file write (it holds
    /// a write to watch a command stay pending), run on the writer thread
    /// with no lock held.
    #[cfg(any(test, feature = "test-support"))]
    write_stall: Option<WriteStall>,
    /// A test's refusing disk: asked after the stall of each file write.
    #[cfg(any(test, feature = "test-support"))]
    write_fault: Option<WriteFault>,
    /// A test stall before a worker read, installed before open queues hydration.
    #[cfg(any(test, feature = "test-support"))]
    read_stall: Option<ReadStall>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default, Clone)]
struct Hooks {
    #[cfg(test)]
    write: Option<WriteHook>,
    #[cfg(test)]
    publish: Option<PublishHook>,
    #[cfg(test)]
    sync: Option<SyncHook>,
    audit: Option<AuditHook>,
    #[cfg(test)]
    terminal: Option<TerminalHook>,
    #[cfg(test)]
    notice: Option<NoticeHook>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum WriterPhase {
    #[default]
    NotStarted,
    Running,
    Exited,
}

#[derive(Default)]
struct Store {
    file: FileState,
    /// A readback is queued or in flight.
    readback: Readback,
    /// When the queued readback was asked for (FIFO order).
    readback_seq: u64,
    /// How many readbacks have completed: a bounded wait follows it.
    readbacks: u64,
    /// A save asked for the next readback: a preferences file whose
    /// content is malformed is quarantined by it ([`Shared::read_repairing`]).
    recover: bool,
    /// Accepted edits not yet taken by the writer thread.
    queue: VecDeque<Queued>,
    /// The writer thread holds this store's edit.
    in_flight: Option<InFlight>,
    /// The last few settlements, for observers.
    settled: VecDeque<Settled>,
    /// How many changes made outside this service have been observed: an
    /// edit accepted before one of them conflicts.
    external: u64,
    /// When the store was last used (least recently used eviction).
    used: u64,
}

impl Store {
    fn idle(&self) -> bool {
        self.queue.is_empty() && self.in_flight.is_none() && self.readback == Readback::Idle
    }
}

#[derive(Default)]
enum FileState {
    /// Never read.
    #[default]
    Cold,
    /// What the service last read or wrote.
    Read(Cached),
    /// The file was refused: no lease, no write, until it reads cleanly.
    Rejected(Rejection),
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Readback {
    #[default]
    Idle,
    Queued,
    InFlight,
    /// In flight, and asked for again after it started: another follows,
    /// so a caller that asked after the read began sees a later read.
    InFlightAgain,
}

#[derive(Clone)]
struct Cached {
    version: StoreVersion,
    at: DurableTimestamp,
    snapshot: LeaseSnapshot,
    /// The worker-parsed preference document and typed walk view.
    preferences: Option<files::PreferenceProjection>,
    /// When this snapshot was last checked on the writer.
    refreshed_at: Instant,
    /// The digest of the file's bytes, `None` when there is no file.
    digest: Option<ContentDigest>,
    /// Whether this service made these bytes durable itself: flushed, and
    /// every directory entry from the file up to the root's parent flushed.
    /// Bytes another writer left, or bytes a write of this service renamed
    /// into place without finishing that chain, are not known durable: an
    /// identical write still publishes them, and flushes the whole chain.
    durable: bool,
    /// How many bytes the rows take as the service writes them.
    written_len: usize,
    /// Whether a write may replace the file.
    rewrite: Rewrite,
}

/// The edit the writer thread holds for one store.
#[derive(Clone)]
struct InFlight {
    terminal: Terminal,
    /// The writer passed its last check and is renaming the bytes into
    /// place: past the shutdown bound it is left to finish.
    publishing: bool,
}

/// What a `terminal` line names for one accepted write: the write's
/// identity, operation and request, never its payload.
#[derive(Clone)]
struct Terminal {
    identity: DomainIdentity,
    action: CommitAction,
    request: RequestId,
}

impl Terminal {
    /// The write's terminal line, when it is not a durable commit (that one
    /// is the [`AuditCommit`] line): the audit line's fields and the
    /// settlement, at its level. A completed write that changed nothing is
    /// a debug line.
    fn line(&self, settlement: &WriteSettlement, successor: Option<RequestId>) -> (Level, String) {
        let (level, outcome) = match settlement {
            Settlement::Completed(receipt) => (
                Level::Debug,
                format!("completed-unchanged {}", receipt.version()),
            ),
            Settlement::Failed(ErrorCode::Conflict { expected, found }) => (
                Level::Warn,
                format!("conflict expected={expected} found={found}"),
            ),
            Settlement::Failed(code) => (Level::Warn, format!("failed ({code})")),
            Settlement::Cancelled(Cancellation::Superseded) => (
                Level::Info,
                match successor {
                    Some(next) => format!("cancelled-superseded by={}", next.get()),
                    None => "cancelled-superseded".into(),
                },
            ),
            Settlement::Cancelled(Cancellation::Shutdown) => {
                (Level::Warn, "cancelled-shutdown".into())
            }
            Settlement::Cancelled(other) => (Level::Info, format!("cancelled ({other:?})")),
        };
        (
            level,
            format!(
                "terminal domain={} profile={} account={} action={} request={} settlement={outcome}",
                self.identity.domain().code(),
                self.identity.account().profile(),
                self.identity.account().account(),
                self.action,
                self.request.get()
            ),
        )
    }
}

struct Rejection {
    reason: FailureReason,
    detail: Arc<str>,
}

enum WriteIntent {
    Port(DomainEdit),
    Preference {
        identity: DomainIdentity,
        patch: PreferencePatch,
    },
}

impl WriteIntent {
    fn identity(&self) -> &DomainIdentity {
        match self {
            Self::Port(edit) => edit.identity(),
            Self::Preference { identity, .. } => identity,
        }
    }

    fn action(&self) -> CommitAction {
        match self {
            Self::Port(DomainEdit::Preferences(_)) | Self::Preference { .. } => {
                CommitAction::PreferencesWrite
            }
            Self::Port(DomainEdit::Loadout(_)) => CommitAction::LoadoutWrite,
            Self::Port(DomainEdit::Bank(_)) => CommitAction::BankHintWrite,
            Self::Port(DomainEdit::Knowledge(_)) => CommitAction::KnowledgeWrite,
        }
    }

    fn expected(&self) -> Option<StoreVersion> {
        match self {
            Self::Port(edit) => edit.expected(),
            Self::Preference { .. } => None,
        }
    }
}

struct Queued {
    request: RequestId,
    intent: WriteIntent,
    /// Acceptance order, for FIFO across stores.
    seq: u64,
    /// The store's version when the edit was accepted.
    accepted_at: StoreVersion,
    /// The store's external-change count when the edit was accepted.
    external: u64,
    /// What the edit may add to the file, for the budget.
    growth: Growth,
}

impl Queued {
    fn terminal(&self) -> Terminal {
        Terminal {
            identity: self.intent.identity().clone(),
            action: self.intent.action(),
            request: self.request,
        }
    }
}

/// An upper bound of what one queued edit adds to the loadouts file.
#[derive(Default, Clone, Copy)]
struct Growth {
    rows: usize,
    bytes: usize,
}

struct Settled {
    request: RequestId,
    settlement: WriteSettlement,
    /// The request that superseded this one, when it was replaced.
    successor: Option<RequestId>,
}

/// One unit of writer-thread work, taken one at a time.
#[allow(clippy::large_enum_variant)]
enum Job {
    Readback(StoreKey),
    Write(StoreKey, Queued),
}

impl LocalStateService {
    /// Open the service at `root` for `domains`, minting request ids from
    /// the composition's `minter`. Domains not named stay closed: their
    /// leases and submissions answer [`ErrorCode::NotReady`], and no file
    /// is ever read or written for them. The operator stores (loadouts,
    /// preferences), when open, start hydrating at once.
    pub fn open(
        root: StateRoot,
        domains: &[LocalStateDomain],
        minter: Arc<RequestIdMinter>,
    ) -> Arc<Self> {
        Self::open_with_state(root, domains, minter, State::default())
    }

    fn open_with_state(
        root: StateRoot,
        domains: &[LocalStateDomain],
        minter: Arc<RequestIdMinter>,
        state: State,
    ) -> Arc<Self> {
        let mut open = [false; 4];
        for domain in domains {
            open[domain_index(*domain)] = true;
        }
        let preference_path: Arc<str> = Arc::from(root.preferences_file().display().to_string());
        let preference_not_ready = PreferenceError::from_cached(
            Arc::clone(&preference_path),
            Arc::from("preference document is not ready yet"),
            Some(ErrorCode::NotReady {
                domain: LocalStateDomain::Preferences,
            }),
        );
        let service = Arc::new(Self {
            shared: Arc::new(Shared {
                root,
                open,
                minter,
                preference_path,
                preference_not_ready,
                state: Mutex::new(state),
                wake: Condvar::new(),
                settled: Condvar::new(),
                writer: Mutex::new(None),
            }),
        });
        host_log!(
            Category::Lifecycle,
            Level::Info,
            "local state: root={} open={}",
            service.shared.root.path().display(),
            domains
                .iter()
                .map(|domain| domain.code())
                .collect::<Vec<_>>()
                .join(",")
        );
        for (domain, key) in [
            (LocalStateDomain::LoadoutsReferences, StoreKey::Loadouts),
            (LocalStateDomain::Preferences, StoreKey::Preferences),
        ] {
            if service.is_open(domain) {
                let mut state = service.shared.state.lock();
                if state.admit(&key).is_ok() {
                    service.shared.request_readback(&mut state, &key);
                }
            }
        }
        service
    }

    /// Open the process stores with a read stall installed before any worker
    /// hydration can begin. Used to exercise cold cache-only readers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_process_with_read_stall_for_test(
        root: StateRoot,
        read_stall: ReadStall,
    ) -> Arc<Self> {
        let state = State {
            read_stall: Some(read_stall),
            ..State::default()
        };
        Self::open_with_state(
            root,
            &[
                LocalStateDomain::Preferences,
                LocalStateDomain::LoadoutsReferences,
                LocalStateDomain::BankHints,
            ],
            Arc::new(RequestIdMinter::new()),
            state,
        )
    }

    /// The process service a frontend's composition root opens once at
    /// `root`: every domain with a current file (preferences, loadouts,
    /// bank hints), with its own request ids. The same service is handed
    /// to the frontend's `Play` ([`crate::Play::adopt_local_state`]), so one
    /// process has one writer for these files.
    pub fn open_process(root: StateRoot) -> Arc<Self> {
        Self::open(
            root,
            &[
                LocalStateDomain::Preferences,
                LocalStateDomain::LoadoutsReferences,
                LocalStateDomain::BankHints,
            ],
            Arc::new(RequestIdMinter::new()),
        )
    }

    /// Return the cached immutable preference document without file I/O.
    pub(super) fn preferences_document(
        self: &Arc<Self>,
        identity: &DomainIdentity,
        freshness: Duration,
    ) -> Result<Arc<serde_json::Map<String, serde_json::Value>>, PreferenceError> {
        self.preferences_snapshot(identity, freshness)
            .map(|(_, document)| document)
    }

    /// Return the cached immutable preference document with the durable
    /// version it was published under, without file I/O. The version moves
    /// only when the document's bytes changed (this service's write, or
    /// another writer's), so it is the applied-version key of a reader.
    pub(super) fn preferences_snapshot(
        self: &Arc<Self>,
        identity: &DomainIdentity,
        freshness: Duration,
    ) -> Result<
        (
            StoreVersion,
            Arc<serde_json::Map<String, serde_json::Value>>,
        ),
        PreferenceError,
    > {
        if !self.is_open(identity.domain()) {
            return Err(self.shared.preference_not_ready.clone());
        }
        let key = StoreKey::Preferences;
        let mut state = self.shared.state.lock();
        let Some(store) = state.stores.get(&key) else {
            return Err(self.shared.preference_not_ready.clone());
        };
        let (version, document, stale) = match &store.file {
            FileState::Cold => return Err(self.shared.preference_not_ready.clone()),
            FileState::Rejected(rejection) => {
                return Err(PreferenceError::from_cached(
                    Arc::clone(&self.shared.preference_path),
                    Arc::clone(&rejection.detail),
                    Some(ErrorCode::Failed {
                        domain: LocalStateDomain::Preferences,
                        reason: rejection.reason,
                    }),
                ));
            }
            FileState::Read(cached) => {
                let document = cached
                    .preferences
                    .as_ref()
                    .map(|projection| Arc::clone(&projection.document))
                    .unwrap_or_else(|| Arc::clone(&state.preference_default.document));
                (
                    cached.version,
                    document,
                    cached.refreshed_at.elapsed() >= freshness,
                )
            }
        };
        if stale {
            self.shared.request_readback(&mut state, &key);
        }
        Ok((version, document))
    }

    pub(super) fn preferences_walk_globals(
        self: &Arc<Self>,
        identity: &DomainIdentity,
        freshness: Duration,
    ) -> Result<crate::walk_permissions::WalkGlobals, PreferenceError> {
        if !self.is_open(identity.domain()) {
            return Err(self.shared.preference_not_ready.clone());
        }
        let key = StoreKey::Preferences;
        let mut state = self.shared.state.lock();
        let Some(store) = state.stores.get(&key) else {
            return Err(self.shared.preference_not_ready.clone());
        };
        let (walk_globals, stale) = match &store.file {
            FileState::Cold => return Err(self.shared.preference_not_ready.clone()),
            FileState::Rejected(rejection) => {
                return Err(PreferenceError::from_cached(
                    Arc::clone(&self.shared.preference_path),
                    Arc::clone(&rejection.detail),
                    Some(ErrorCode::Failed {
                        domain: LocalStateDomain::Preferences,
                        reason: rejection.reason,
                    }),
                ));
            }
            FileState::Read(cached) => {
                let projection = cached
                    .preferences
                    .as_ref()
                    .unwrap_or(&state.preference_default);
                (
                    projection.walk_globals.clone(),
                    cached.refreshed_at.elapsed() >= freshness,
                )
            }
        };
        if stale {
            self.shared.request_readback(&mut state, &key);
        }
        walk_globals.map_err(|cause| {
            PreferenceError::from_cached(Arc::clone(&self.shared.preference_path), cause, None)
        })
    }

    pub(super) fn has_preference_watch(&self, request: RequestId) -> bool {
        self.shared
            .state
            .lock()
            .preference_watches
            .contains_key(&request)
    }

    pub(super) fn unprojected_preference_requests(&self) -> Vec<RequestId> {
        self.shared
            .state
            .lock()
            .preference_watches
            .iter()
            .filter_map(|(request, watch)| watch.settlement.is_some().then_some(*request))
            .collect()
    }

    pub(super) fn poll_preference_watch(&self, request: RequestId) -> PreferenceWatchPoll {
        let mut state = self.shared.state.lock();
        match state
            .preference_watches
            .get(&request)
            .and_then(|watch| watch.settlement.clone())
        {
            Some(settlement) => {
                state.preference_watches.remove(&request);
                PreferenceWatchPoll::Settled(settlement)
            }
            None if state.preference_watches.contains_key(&request) => PreferenceWatchPoll::Pending,
            None => PreferenceWatchPoll::Missing,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn wait_preference_watch(
        &self,
        request: RequestId,
        wait: Duration,
    ) -> PreferenceWatchPoll {
        let deadline = Instant::now() + wait;
        let mut state = self.shared.state.lock();
        loop {
            match state
                .preference_watches
                .get(&request)
                .and_then(|watch| watch.settlement.clone())
            {
                Some(settlement) => {
                    state.preference_watches.remove(&request);
                    return PreferenceWatchPoll::Settled(settlement);
                }
                None if !state.preference_watches.contains_key(&request) => {
                    return PreferenceWatchPoll::Missing;
                }
                None => {}
            }
            if wait.is_zero()
                || self
                    .shared
                    .settled
                    .wait_until(&mut state, deadline)
                    .timed_out()
            {
                return PreferenceWatchPoll::Pending;
            }
        }
    }

    pub(super) fn enqueue_preference_patch(
        &self,
        identity: &DomainIdentity,
        patch: PreferencePatch,
    ) -> Result<RequestId, ErrorCode> {
        let domain = LocalStateDomain::Preferences;
        let key = StoreKey::Preferences;
        if !self.is_open(domain) {
            return Err(ErrorCode::NotReady { domain });
        }
        let mut state = self.shared.state.lock();
        if state.shutdown || state.preference_watches.len() >= PREFERENCE_WATCH_CAP {
            return Err(ErrorCode::Busy { domain });
        }
        if state
            .preference_queue_bytes
            .saturating_add(patch.encoded_bytes)
            > files::MAX_PREFERENCES_BYTES as usize
        {
            return Err(ErrorCode::Validation {
                domain,
                reason: ValidationReason::TooLong,
            });
        }
        let store = state
            .stores
            .get(&key)
            .ok_or(ErrorCode::NotReady { domain })?;
        if store.queue.len() >= OPERATOR_QUEUE_CAP {
            return Err(ErrorCode::Busy { domain });
        }
        if let FileState::Rejected(rejection) = &store.file {
            return Err(ErrorCode::Failed {
                domain,
                reason: rejection.reason,
            });
        }
        if !self.shared.start_writer(&mut state) {
            return Err(ErrorCode::Busy { domain });
        }
        let request = self
            .shared
            .minter
            .mint()
            .map_err(|_| ErrorCode::Busy { domain })?;
        let seq = state.next_seq();
        let accepted_at = state.clock.unwrap_or(StoreVersion::INITIAL);
        let queued_bytes = patch.encoded_bytes;
        let store = state
            .stores
            .get_mut(&key)
            .expect("the preference store was opened before admission");
        let external = store.external;
        store.queue.push_back(Queued {
            request,
            intent: WriteIntent::Preference {
                identity: identity.clone(),
                patch,
            },
            seq,
            accepted_at,
            external,
            growth: Growth::default(),
        });
        state.preference_queue_bytes += queued_bytes;
        state.preference_watches.insert(
            request,
            PreferenceWatch {
                settlement: None,
                queued_bytes,
            },
        );
        state.requests.insert(request, key);
        drop(state);
        self.shared.wake.notify_one();
        Ok(request)
    }
    /// The operator preferences (`panel-ui.json`) through this service.
    pub fn preferences(self: &Arc<Self>) -> Preferences {
        Preferences::new(Arc::clone(self))
    }

    /// The operator loadouts a frontend's form edits, through this
    /// service.
    pub fn operator_loadouts(self: &Arc<Self>) -> ServiceLoadouts {
        ServiceLoadouts::new(Arc::clone(self))
    }

    /// The explicit root every path derives from.
    pub fn root(&self) -> &StateRoot {
        &self.shared.root
    }

    /// Whether `domain` was opened.
    pub fn is_open(&self, domain: LocalStateDomain) -> bool {
        self.shared.open[domain_index(domain)]
    }

    /// Re-read `identity`'s current file on the writer thread and lease the
    /// result, waiting at most `timeout`. `Ok(None)` when there is no file
    /// for the identity yet (never written); `Ok(Some)` otherwise, with the
    /// rows the file holds now, so an edit made by another writer (an older
    /// process, a hand edit) is seen. A read that was already running when
    /// this was called does not count: the answer comes from a read that
    /// began after the call. A refused file is [`ErrorCode::Failed`] with
    /// its reason, never `Ok(None)`; `NotReady` when the readback did not
    /// complete within `timeout`.
    ///
    /// This is the host's replacement for the synchronous file reads the
    /// old code did: a script Start reading the loadouts file, and a
    /// session boundary reading the account's hint file. Never called on a
    /// tick.
    pub fn readback_lease(
        &self,
        identity: &DomainIdentity,
        timeout: Duration,
    ) -> Result<Option<DomainLease>, ErrorCode> {
        let (lease, exists) = self.readback(identity, timeout)?;
        Ok(exists.then_some(lease))
    }

    /// [`Self::readback_lease`] that leases an absent file too: its empty
    /// rows at the store's real version, and whether a file exists.
    pub(crate) fn readback(
        &self,
        identity: &DomainIdentity,
        timeout: Duration,
    ) -> Result<(DomainLease, bool), ErrorCode> {
        let ticket = self.start_readback(identity, false)?;
        self.finish_readback(identity, ticket, Some(Instant::now() + timeout))
            .unwrap_or(Err(ErrorCode::NotReady {
                domain: identity.domain(),
            }))
    }

    /// Queue a re-read of `identity`'s file and return what
    /// [`Self::finish_readback`] waits for: a read that began after this
    /// call. With `recover`, the read is a save's (a preferences update
    /// about to write): the one read that quarantines a preferences file
    /// whose content is malformed and starts fresh preferences (the
    /// recovery the old shared writer made on a save). A read for anything
    /// else refuses such a file, so its readers fail closed until a save
    /// recovers it.
    pub(crate) fn start_readback(
        &self,
        identity: &DomainIdentity,
        recover: bool,
    ) -> Result<ReadbackTicket, ErrorCode> {
        let key = self.store_key(identity)?;
        let domain = identity.domain();
        let mut state = self.shared.state.lock();
        state.admit(&key)?;
        if recover {
            if let Some(store) = state.stores.get_mut(&key) {
                store.recover = true;
            }
        }
        let Some(target) = self.shared.request_readback(&mut state, &key) else {
            return Err(ErrorCode::NotReady { domain });
        };
        Ok(ReadbackTicket { target })
    }

    /// The result of the read `ticket` waits for: the rows at the store's
    /// real version and whether a file exists, or the typed refusal.
    /// `None` while that read has not completed by `deadline`; without a
    /// deadline, `None` at once (a frame's check, never a wait).
    pub(crate) fn finish_readback(
        &self,
        identity: &DomainIdentity,
        ticket: ReadbackTicket,
        deadline: Option<Instant>,
    ) -> Option<Result<(DomainLease, bool), ErrorCode>> {
        let domain = identity.domain();
        let key = match self.store_key(identity) {
            Ok(key) => key,
            Err(code) => return Some(Err(code)),
        };
        let mut state = self.shared.state.lock();
        loop {
            let Some(store) = state.stores.get(&key) else {
                return Some(Err(ErrorCode::NotReady { domain }));
            };
            if store.readbacks >= ticket.target {
                return Some(match &store.file {
                    FileState::Cold => Err(ErrorCode::NotReady { domain }),
                    FileState::Rejected(rejection) => Err(ErrorCode::Failed {
                        domain,
                        reason: rejection.reason,
                    }),
                    FileState::Read(cached) => {
                        lease_of(identity, cached).map(|lease| (lease, cached.digest.is_some()))
                    }
                });
            }
            let deadline = deadline?;
            if self
                .shared
                .settled
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return None;
            }
        }
    }

    /// Queue a re-read of `identity`'s file without waiting for it: a later
    /// lease sees what the file holds then. Nothing happens for a closed
    /// domain or a service that is stopping.
    pub fn refresh(&self, identity: &DomainIdentity) {
        let Ok(key) = self.store_key(identity) else {
            return;
        };
        let mut state = self.shared.state.lock();
        if state.admit(&key).is_ok() {
            self.shared.request_readback(&mut state, &key);
        }
    }

    /// Where `request` stands now: what a slot reads on its frames.
    pub fn request_state(&self, request: RequestId) -> RequestState {
        self.shared.state.lock().request_state(request)
    }

    /// Wait at most `timeout` for `request` to leave
    /// [`RequestState::Pending`].
    pub fn wait_request(&self, request: RequestId, timeout: Duration) -> RequestState {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            let current = state.request_state(request);
            if current != RequestState::Pending {
                return current;
            }
            if self
                .shared
                .settled
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return RequestState::Pending;
            }
        }
    }

    /// Wait at most `timeout` until `request`'s rows are durable or known
    /// not to be, following each request that superseded it: a leaving
    /// slot's wait for the hint it handed over, whichever slot's newer hint
    /// replaced it.
    pub fn wait_durable(&self, request: RequestId, timeout: Duration) -> Durability {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        let mut current = request;
        loop {
            match state.settled_of(current) {
                None => return Durability::Forgotten,
                Some(None) => {
                    if self
                        .shared
                        .settled
                        .wait_until(&mut state, deadline)
                        .timed_out()
                    {
                        return Durability::Pending;
                    }
                }
                Some(Some(settled)) => match (&settled.settlement, settled.successor) {
                    (Settlement::Completed(receipt), _) => {
                        return Durability::Durable(receipt.clone())
                    }
                    (Settlement::Cancelled(Cancellation::Superseded), Some(next)) => {
                        current = next;
                    }
                    (settlement, _) => return Durability::NotDurable(settlement.clone()),
                },
            }
        }
    }

    /// Test-support seam for front-end tests: run `stall` on the writer
    /// thread at the start of every file write (a busy disk on demand).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_write_stall_for_test(&self, stall: WriteStall) {
        self.shared.state.lock().write_stall = Some(stall);
    }

    /// Test-support seam for front-end tests: `fault` runs on the writer
    /// thread at the start of each file write, after the stall; when it
    /// returns true that write settles `Failed(Io)`, as a disk that refuses
    /// the replacement does, on any platform.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_write_fault_for_test(&self, fault: WriteFault) {
        self.shared.state.lock().write_fault = Some(fault);
    }

    /// Make a hydrated preference lease stale without waiting for a clock tick.
    #[cfg(test)]
    pub(crate) fn expire_preference_cache_for_test(&self) {
        let mut state = self.shared.state.lock();
        let store = state
            .stores
            .get_mut(&StoreKey::Preferences)
            .expect("preferences are open");
        let FileState::Read(cached) = &mut store.file else {
            panic!("the preference cache must be hydrated before expiry");
        };
        cached.refreshed_at = Instant::now()
            .checked_sub(frontend::PREFERENCES_FRESHNESS)
            .expect("the test clock can represent the preference freshness interval");
    }

    /// Whether a preference readback is demanded, and how many finished.
    #[cfg(test)]
    pub(crate) fn preference_readback_state_for_test(&self) -> (bool, u64) {
        let state = self.shared.state.lock();
        let store = state
            .stores
            .get(&StoreKey::Preferences)
            .expect("preferences are open");
        (store.readback != Readback::Idle, store.readbacks)
    }

    /// Wait for existing preference read demand without queuing another read.
    #[cfg(test)]
    pub(crate) fn wait_preference_readbacks_for_test(
        &self,
        target: u64,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            let store = state
                .stores
                .get(&StoreKey::Preferences)
                .expect("preferences are open");
            if store.readbacks >= target {
                return true;
            }
            if self
                .shared
                .settled
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return false;
            }
        }
    }

    /// Publish every file write through `hook` instead of the real file.
    #[cfg(test)]
    pub(crate) fn set_write_hook(&self, hook: WriteHook) {
        self.shared.state.lock().hooks.write = Some(hook);
    }

    /// Flush every directory through `hook` instead of the real flush.
    #[cfg(test)]
    pub(crate) fn set_sync_hook(&self, hook: SyncHook) {
        self.shared.state.lock().hooks.sync = Some(hook);
    }

    /// Call `hook` with every audit record where its line is logged.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_audit_hook(&self, hook: AuditHook) {
        self.shared.state.lock().hooks.audit = Some(hook);
    }

    /// Run `hook` inside every real publication, after staging and before
    /// the last check.
    #[cfg(test)]
    pub(crate) fn set_publish_hook(&self, hook: PublishHook) {
        self.shared.state.lock().hooks.publish = Some(hook);
    }

    /// Call `hook` with every terminal line where it is logged.
    #[cfg(test)]
    pub(crate) fn set_terminal_hook(&self, hook: TerminalHook) {
        self.shared.state.lock().hooks.terminal = Some(hook);
    }

    /// Call `hook` with each line of a repair where it is logged.
    #[cfg(test)]
    pub(crate) fn set_notice_hook(&self, hook: NoticeHook) {
        self.shared.state.lock().hooks.notice = Some(hook);
    }

    /// Wait `timeout` instead of [`SHUTDOWN_TIMEOUT`] when dropped.
    #[cfg(test)]
    pub(crate) fn set_shutdown_timeout(&self, timeout: Duration) {
        self.shared.state.lock().shutdown_timeout = Some(timeout);
    }

    /// The settlement ledger, readable after the last handle is dropped.
    #[cfg(test)]
    pub(crate) fn ledger(&self) -> Ledger {
        Ledger(Arc::clone(&self.shared))
    }

    fn store_key(&self, identity: &DomainIdentity) -> Result<StoreKey, ErrorCode> {
        let domain = identity.domain();
        if !self.is_open(domain) {
            return Err(ErrorCode::NotReady { domain });
        }
        StoreKey::of(identity).ok_or(ErrorCode::NotReady { domain })
    }
}

/// A test's view of the settlements after the service's last handle went.
#[cfg(test)]
pub(crate) struct Ledger(Arc<Shared>);

#[cfg(test)]
impl Ledger {
    pub(crate) fn request_state(&self, request: RequestId) -> RequestState {
        self.0.state.lock().request_state(request)
    }

    /// Wait at most `timeout` for the writer thread to stop.
    pub(crate) fn wait_writer_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.0.state.lock();
        while state.writer == WriterPhase::Running {
            if self.0.settled.wait_until(&mut state, deadline).timed_out() {
                break;
            }
        }
        state.writer != WriterPhase::Running
    }
}

impl StatePort for LocalStateService {
    fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
        let key = self.store_key(identity)?;
        let domain = identity.domain();
        let mut state = self.shared.state.lock();
        let answer = match &state.admit(&key)?.file {
            FileState::Read(cached) => Some(Ok(cached.version)),
            FileState::Rejected(rejection) => Some(Err(ErrorCode::Failed {
                domain,
                reason: rejection.reason,
            })),
            FileState::Cold => None,
        };
        if let Some(answer) = answer {
            return answer;
        }
        // Cold, as a lease would be: queue the hydration, read nothing.
        self.shared.request_readback(&mut state, &key);
        Err(ErrorCode::NotReady { domain })
    }
}

impl LocalStatePort for LocalStateService {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        let key = self.store_key(identity)?;
        let domain = identity.domain();
        let mut state = self.shared.state.lock();
        let answer = match &state.admit(&key)?.file {
            FileState::Read(cached) => Some(lease_of(identity, cached)),
            FileState::Rejected(rejection) => Some(Err(ErrorCode::Failed {
                domain,
                reason: rejection.reason,
            })),
            FileState::Cold => None,
        };
        if let Some(answer) = answer {
            return answer;
        }
        // Cold: queue the hydration so a later lease succeeds; never read
        // the disk here.
        self.shared.request_readback(&mut state, &key);
        Err(ErrorCode::NotReady { domain })
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        let domain = edit.identity().domain();
        let key = match self.store_key(edit.identity()) {
            Ok(key) => key,
            Err(code) => return SubmitReceipt::Refused(code),
        };
        // A loadout row is measured before the lock is taken.
        let growth = match growth_of(&edit) {
            Ok(growth) => growth,
            Err(code) => return refused_visibly(&key, code),
        };
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return SubmitReceipt::Refused(ErrorCode::Busy { domain });
        }
        let store = match state.admit(&key) {
            Ok(store) => store,
            Err(code) => return SubmitReceipt::Refused(code),
        };
        let (accepted_at, growth) = match gate(&key, store, &edit, growth) {
            Gate::Accept(version, growth) => (version, growth),
            Gate::Hydrate(code) => {
                // Cold, or a refused file: (re)read it, so a later
                // submission finds it ready or repaired. Never written
                // meanwhile.
                self.shared.request_readback(&mut state, &key);
                return SubmitReceipt::Refused(code);
            }
            Gate::Refuse(code) => {
                drop(state);
                return refused_visibly(&key, code);
            }
        };
        if !self.shared.start_writer(&mut state) {
            return SubmitReceipt::Refused(ErrorCode::Busy { domain });
        }
        let request = match self.shared.minter.mint() {
            Ok(request) => request,
            Err(_) => return SubmitReceipt::Refused(ErrorCode::Busy { domain }),
        };
        let seq = state.next_seq();
        let store = state
            .stores
            .get_mut(&key)
            .expect("the store was admitted under this lock");
        let superseded = match &key {
            // A newer hint of the same account makes a queued one moot: at
            // most one waits behind the one in flight.
            StoreKey::BankHint { .. } => store.queue.pop_front().map(|queued| queued.terminal()),
            StoreKey::Loadouts | StoreKey::Preferences => None,
        };
        let external = store.external;
        store.queue.push_back(Queued {
            request,
            intent: WriteIntent::Port(edit),
            seq,
            accepted_at,
            external,
            growth,
        });
        state.requests.insert(request, key.clone());
        let cancelled = Settlement::Cancelled(Cancellation::Superseded);
        let superseded = superseded.filter(|terminal| {
            state.settle(&key, terminal.request, cancelled.clone(), Some(request))
        });
        drop(state);
        self.shared.wake.notify_one();
        if let Some(terminal) = superseded {
            self.shared.settled.notify_all();
            self.shared.terminal(&terminal, &cancelled, Some(request));
        }
        SubmitReceipt::Accepted { request }
    }

    fn observe(&self, request: RequestId) -> Option<WriteSettlement> {
        match self.request_state(request) {
            RequestState::Settled(settlement) => Some(settlement),
            RequestState::Pending | RequestState::Forgotten => None,
        }
    }
}

impl Drop for LocalStateService {
    fn drop(&mut self) {
        let (outstanding, timeout) = {
            let mut state = self.shared.state.lock();
            state.shutdown = true;
            #[cfg(test)]
            let timeout = state.shutdown_timeout.unwrap_or(SHUTDOWN_TIMEOUT);
            #[cfg(not(test))]
            let timeout = SHUTDOWN_TIMEOUT;
            (state.outstanding_writes(), timeout)
        };
        self.shared.wake.notify_all();
        if outstanding > 0 {
            host_log!(
                Category::Lifecycle,
                Level::Info,
                "local state: stopping with {outstanding} write(s) accepted; finishing them"
            );
        }
        let deadline = Instant::now() + timeout;
        let stopped = {
            let mut state = self.shared.state.lock();
            while state.writer == WriterPhase::Running {
                if self
                    .shared
                    .settled
                    .wait_until(&mut state, deadline)
                    .timed_out()
                {
                    break;
                }
            }
            state.writer != WriterPhase::Running
        };
        let handle = self.shared.writer.lock().take();
        if stopped {
            if let Some(handle) = handle {
                if handle.join().is_err() {
                    host_log!(
                        Category::Lifecycle,
                        Level::Error,
                        "local state: the writer thread ended in a panic"
                    );
                }
            }
            return;
        }
        // Past the bound: every accepted write the writer has not begun to
        // rename into place gets its terminal now, each with its own line;
        // the writer publishes nothing more. One being renamed is left to
        // settle when it finishes (or is lost unacknowledged with the
        // process).
        let (cancelled, publishing) = self.shared.abandon();
        self.shared.settled.notify_all();
        host_log!(
            Category::Lifecycle,
            Level::Warn,
            "local state: shutdown bound of {timeout:?} passed; {cancelled} accepted write(s) \
             settled cancelled, {publishing} still being renamed into place settle when they \
             finish"
        );
    }
}

/// What admission decided for a submission against a store.
enum Gate {
    /// Accept against the store at this version, charging this growth.
    Accept(StoreVersion, Growth),
    /// Refuse, and read the file (cold, or refused and maybe repaired).
    Hydrate(ErrorCode),
    /// Refuse as it is, visibly.
    Refuse(ErrorCode),
}

/// Admit `edit` against `store`, or refuse it: a cold or refused store is
/// read (again) first, a store whose rows a rewrite would lose is never
/// written, an operator store's queue is capped, a preference value must
/// be JSON, and a loadouts edit is charged against the file's row and byte
/// budget together with the edits queued before it.
fn gate(key: &StoreKey, store: &Store, edit: &DomainEdit, mut growth: Growth) -> Gate {
    let domain = key.domain();
    let cached = match &store.file {
        FileState::Cold => return Gate::Hydrate(ErrorCode::NotReady { domain }),
        FileState::Rejected(rejection) => {
            return Gate::Hydrate(ErrorCode::Failed {
                domain,
                reason: rejection.reason,
            })
        }
        FileState::Read(cached) => cached,
    };
    if let Rewrite::Refused { .. } = cached.rewrite {
        // The rows read are not everything the file holds: a rewrite would
        // lose the rest. Re-read: a repaired file is accepted again.
        return Gate::Hydrate(ErrorCode::Failed {
            domain,
            reason: FailureReason::Schema,
        });
    }
    if key.operator_wide() && store.queue.len() >= OPERATOR_QUEUE_CAP {
        return Gate::Refuse(ErrorCode::Busy { domain });
    }
    if let DomainEdit::Preferences(edit) = edit {
        let malformed = edit.changes().iter().any(|change| {
            change
                .value()
                .is_some_and(|value| serde_json::from_str::<serde_json::Value>(value).is_err())
        });
        if malformed {
            return Gate::Refuse(ErrorCode::Validation {
                domain,
                reason: ValidationReason::Schema,
            });
        }
    }
    if let StoreKey::Loadouts = key {
        let rows = cached.snapshot.as_loadouts().unwrap_or(&[]);
        if let DomainEdit::Loadout(edit) = edit {
            if let LoadoutOp::Upsert(loadout) = edit.op() {
                // Replacing a row adds no row (its bytes are still charged
                // in full: an upper bound).
                if rows.iter().any(|row| row.name == loadout.name) {
                    growth.rows = 0;
                }
            }
        }
        let queued = store
            .queue
            .iter()
            .fold(Growth::default(), |sum, queued| Growth {
                rows: sum.rows + queued.growth.rows,
                bytes: sum.bytes + queued.growth.bytes,
            });
        if growth.rows > 0 && rows.len() + queued.rows + growth.rows > files::MAX_LOADOUT_ROWS {
            return Gate::Refuse(ErrorCode::Validation {
                domain,
                reason: ValidationReason::TooManyRows,
            });
        }
        if growth.bytes > 0
            && (cached.written_len + queued.bytes + growth.bytes) as u64 > files::MAX_LOADOUTS_BYTES
        {
            return Gate::Refuse(ErrorCode::Validation {
                domain,
                reason: ValidationReason::TooLong,
            });
        }
    }
    Gate::Accept(cached.version, growth)
}

/// An upper bound of what `edit` adds to its file: a loadout upsert may
/// add its row (one row, its printed bytes), a rename replaces one (no
/// row, its new printed bytes); nothing else grows a file past its bound
/// (a hint's rows are capped by the hint itself).
fn growth_of(edit: &DomainEdit) -> Result<Growth, ErrorCode> {
    let DomainEdit::Loadout(edit) = edit else {
        return Ok(Growth::default());
    };
    let (loadout, rows) = match edit.op() {
        LoadoutOp::Upsert(loadout) => (loadout, 1),
        LoadoutOp::Rename { loadout, .. } => (loadout, 0),
        LoadoutOp::Remove { .. } => return Ok(Growth::default()),
    };
    let domain = LocalStateDomain::LoadoutsReferences;
    let bytes = files::row_bytes(loadout).map_err(|_| ErrorCode::Validation {
        domain,
        reason: ValidationReason::Schema,
    })?;
    if bytes > files::MAX_LOADOUT_ROW_BYTES {
        return Err(ErrorCode::Validation {
            domain,
            reason: ValidationReason::TooLong,
        });
    }
    Ok(Growth { rows, bytes })
}

/// Refuse a submission past a budget or into a refused store, with one
/// line saying so.
fn refused_visibly(key: &StoreKey, code: ErrorCode) -> SubmitReceipt {
    host_log!(
        Category::Lifecycle,
        Level::Warn,
        "local state: {} submission refused: {code}",
        key.domain()
    );
    SubmitReceipt::Refused(code)
}

impl State {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// The next version of any store, or `None` once the space is spent:
    /// versions refuse instead of wrapping or reusing a value.
    fn next_version(&mut self) -> Option<StoreVersion> {
        let next = match self.clock {
            None => StoreVersion::INITIAL,
            Some(last) => last.checked_next()?,
        };
        self.clock = Some(next);
        Some(next)
    }

    /// The store of `key`, admitted (and stamped used) if it is new. A new
    /// bank-hint identity past [`MAX_BANK_STORES`] evicts the least
    /// recently used idle bank store, or is refused `Busy`.
    fn admit(&mut self, key: &StoreKey) -> Result<&mut Store, ErrorCode> {
        let used = self.next_seq();
        if !self.stores.contains_key(key) {
            if let StoreKey::BankHint { .. } = key {
                let held = self.stores.len()
                    - usize::from(self.stores.contains_key(&StoreKey::Loadouts))
                    - usize::from(self.stores.contains_key(&StoreKey::Preferences));
                if held >= MAX_BANK_STORES {
                    self.evict_idle_bank_store(key)?;
                }
            }
            self.stores.insert(key.clone(), Store::default());
        }
        let store = self.stores.get_mut(key).expect("admitted above");
        store.used = used;
        Ok(store)
    }

    fn evict_idle_bank_store(&mut self, wanted: &StoreKey) -> Result<(), ErrorCode> {
        let victim = self
            .stores
            .iter()
            .filter(|(key, store)| matches!(key, StoreKey::BankHint { .. }) && store.idle())
            .min_by_key(|(_, store)| store.used)
            .map(|(key, _)| key.clone());
        let Some(victim) = victim else {
            host_log!(
                Category::Lifecycle,
                Level::Warn,
                "local state: {} refused: {MAX_BANK_STORES} identities held and none idle",
                wanted.domain()
            );
            return Err(ErrorCode::Busy {
                domain: wanted.domain(),
            });
        };
        if let Some(store) = self.stores.remove(&victim) {
            for settled in store.settled {
                self.requests.remove(&settled.request);
            }
        }
        if let StoreKey::BankHint {
            profile, account, ..
        } = &victim
        {
            host_log!(
                Category::Lifecycle,
                Level::Info,
                "local state: bank-hints holds {MAX_BANK_STORES} identities; forgot idle {profile}/{account}"
            );
        }
        Ok(())
    }

    /// Record `request`'s settlement in its store's bounded list, once: a
    /// request already settled (cancelled at the shutdown bound while its
    /// writer was still working) keeps its first terminal. The oldest
    /// settlement past the bound is forgotten.
    fn settle(
        &mut self,
        key: &StoreKey,
        request: RequestId,
        settlement: WriteSettlement,
        successor: Option<RequestId>,
    ) -> bool {
        let State {
            stores,
            requests,
            preference_watches,
            preference_queue_bytes,
            ..
        } = self;
        let Some(store) = stores.get(key) else {
            requests.remove(&request);
            if let Some(watch) = preference_watches.remove(&request) {
                *preference_queue_bytes = preference_queue_bytes.saturating_sub(watch.queued_bytes);
            }
            return false;
        };
        if store
            .settled
            .iter()
            .any(|settled| settled.request == request)
            || preference_watches
                .get(&request)
                .is_some_and(|watch| watch.settlement.is_some())
        {
            return false;
        }
        if let Some(watch) = preference_watches.get_mut(&request) {
            if watch.settlement.is_none() {
                *preference_queue_bytes = preference_queue_bytes.saturating_sub(watch.queued_bytes);
                watch.queued_bytes = 0;
                watch.settlement = Some(settlement.clone());
            }
        }
        let store = stores.get_mut(key).expect("the store was checked above");
        store.settled.push_back(Settled {
            request,
            settlement,
            successor,
        });
        while store.settled.len() > SETTLED_PER_STORE {
            if let Some(forgotten) = store.settled.pop_front() {
                requests.remove(&forgotten.request);
            }
        }
        true
    }

    /// The shutdown bound passed: settle every accepted write the writer
    /// has not begun to rename into place `Cancelled(Shutdown)` (the queued
    /// ones, and an in-flight one short of its rename), and stop the writer
    /// from publishing more. The terminals to log, and how many in-flight
    /// writes are being renamed and settle when they finish.
    fn abandon(&mut self) -> (Vec<Terminal>, usize) {
        self.abandoned = true;
        let mut terminals = Vec::new();
        let mut publishing = 0;
        let mut cancelled = Vec::new();
        for (key, store) in &mut self.stores {
            for queued in store.queue.drain(..) {
                terminals.push(queued.terminal());
                cancelled.push((key.clone(), queued.request));
            }
            match &store.in_flight {
                Some(in_flight) if in_flight.publishing => publishing += 1,
                Some(in_flight) => {
                    terminals.push(in_flight.terminal.clone());
                    cancelled.push((key.clone(), in_flight.terminal.request));
                }
                None => {}
            }
        }
        for (key, request) in cancelled {
            self.settle(
                &key,
                request,
                Settlement::Cancelled(Cancellation::Shutdown),
                None,
            );
        }
        (terminals, publishing)
    }

    /// `Some(Some)` settled, `Some(None)` pending, `None` forgotten.
    fn settled_of(&self, request: RequestId) -> Option<Option<&Settled>> {
        let key = self.requests.get(&request)?;
        let store = self.stores.get(key)?;
        Some(
            store
                .settled
                .iter()
                .find(|settled| settled.request == request),
        )
    }

    fn request_state(&self, request: RequestId) -> RequestState {
        match self.settled_of(request) {
            None => RequestState::Forgotten,
            Some(None) => RequestState::Pending,
            Some(Some(settled)) => RequestState::Settled(settled.settlement.clone()),
        }
    }

    fn outstanding_writes(&self) -> usize {
        self.stores
            .values()
            .map(|store| store.queue.len() + usize::from(store.in_flight.is_some()))
            .sum()
    }

    /// Take what a read of `key`'s file found into the store: a change from
    /// what the service held takes the next version (and is counted as an
    /// outside change when the store had been read before), a refused file
    /// becomes [`FileState::Rejected`]. Logs each change once.
    fn adopt(
        &mut self,
        key: &StoreKey,
        read: Result<Option<ReadDocument>, ReadError>,
        read_at: DurableTimestamp,
        path: &Path,
    ) {
        let domain = key.domain();
        let Some(store) = self.stores.get(key) else {
            return;
        };
        let known = !matches!(store.file, FileState::Cold);
        let rejected_detail = match &read {
            Err(error) => Some(Arc::<str>::from(error.to_string())),
            _ => None,
        };
        let unchanged = match (&read, &store.file) {
            (Ok(Some(document)), FileState::Read(cached)) => cached.digest == Some(document.digest),
            (Ok(None), FileState::Read(cached)) => cached.digest.is_none(),
            (Err(_), FileState::Rejected(rejection)) => {
                Some(&rejection.detail) == rejected_detail.as_ref()
            }
            _ => false,
        };
        if unchanged {
            if let Some(FileState::Read(cached)) =
                self.stores.get_mut(key).map(|store| &mut store.file)
            {
                cached.refreshed_at = Instant::now();
            }
            return;
        }
        let next = read.is_ok();
        let version = if next { self.next_version() } else { None };
        let store = self.stores.get_mut(key).expect("checked above");
        let file = match read {
            Ok(document) => {
                let Some(version) = version else {
                    store.file = FileState::Rejected(Rejection {
                        reason: FailureReason::Io,
                        detail: "the version space is spent".into(),
                    });
                    return;
                };
                match document {
                    Some(document) => {
                        if known {
                            host_log!(
                                Category::Lifecycle,
                                Level::Info,
                                "local state: {domain} changed outside this service, now {version} path={}",
                                path.display()
                            );
                        }
                        if let Rewrite::Refused { unreadable, lossy } = document.rewrite {
                            host_log!(
                                Category::Lifecycle,
                                Level::Warn,
                                "local state: {domain} file has {unreadable} unreadable row(s) and \
                                 {lossy} row(s) that would not write back as read; its rows are \
                                 read but the service never rewrites it path={}",
                                path.display()
                            );
                        }
                        FileState::Read(Cached {
                            version,
                            at: document.at,
                            snapshot: document.snapshot,
                            preferences: document.preferences,
                            refreshed_at: Instant::now(),
                            digest: Some(document.digest),
                            durable: false,
                            written_len: document.written_len,
                            rewrite: document.rewrite,
                        })
                    }
                    None => {
                        if known {
                            host_log!(
                                Category::Lifecycle,
                                Level::Info,
                                "local state: {domain} removed outside this service, now {version} path={}",
                                path.display()
                            );
                        }
                        FileState::Read(Cached {
                            version,
                            at: read_at,
                            snapshot: empty_snapshot(key),
                            preferences: matches!(key, StoreKey::Preferences)
                                .then(|| self.preference_default.clone()),
                            refreshed_at: Instant::now(),
                            digest: None,
                            durable: false,
                            written_len: empty_len(key),
                            rewrite: Rewrite::Lossless,
                        })
                    }
                }
            }
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: {domain} file refused path={}: {error}; it is neither leased \
                     nor written until it reads cleanly",
                    path.display()
                );
                FileState::Rejected(Rejection {
                    reason: error.reason(),
                    detail: rejected_detail.expect("a refused read has its detail"),
                })
            }
        };
        if known {
            store.external += 1;
        }
        store.file = file;
    }
}

/// What the writer staged for one edit, decided outside the service lock.
struct Staged {
    bytes: Vec<u8>,
    snapshot: LeaseSnapshot,
    at: DurableTimestamp,
    action: CommitAction,
    /// A worker-parsed preference projection, when this write changed that file.
    preferences: Option<files::PreferenceProjection>,
    /// The version the edit applies to, and the one it commits.
    previous: StoreVersion,
    version: StoreVersion,
    /// Whether these bytes are already durable as the file's content.
    unchanged: bool,
    /// Whether this request changed the saved state and needs a new version.
    advance_version: bool,
    /// The one saved-write audit line to emit after durability, if any.
    audit: Option<AuditKind>,
    /// The digest of what the file held when the write was decided (`None`
    /// for no file): the last check before the rename compares against it.
    precondition: Option<ContentDigest>,
    /// Whether this service had made the file's current entry durable: if
    /// not (no file yet, another writer's file, or an earlier write whose
    /// directory chain did not finish), the whole chain is flushed.
    durable_before: bool,
}

enum AuditKind {
    Plain,
    Preferences(PreferenceAuditSummary),
}

struct PreferenceAuditSummary {
    text: String,
}

impl std::fmt::Display for PreferenceAuditSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

const PREFERENCE_AUDIT_MAX_BYTES: usize = 4_096;
const PREFERENCE_AUDIT_MAX_CHANGES: usize = 32;

#[derive(Clone, Copy)]
enum PreferenceAuditScalarKind {
    Boolean,
    MapBake,
    ServerRevision,
}

enum PreferenceAuditScalar<'a> {
    Absent,
    Boolean(bool),
    MapBake(&'a str),
    ServerRevision(u64),
}

struct PreferenceAuditChange<'a> {
    path: Vec<&'a str>,
    old: Option<&'a serde_json::Value>,
    new: Option<&'a serde_json::Value>,
    scalar: Option<PreferenceAuditScalarKind>,
    count: usize,
}

fn preference_audit_summary(
    before: &serde_json::Map<String, serde_json::Value>,
    after: &serde_json::Map<String, serde_json::Value>,
) -> Option<PreferenceAuditSummary> {
    let mut changes = Vec::new();
    let mut total = 0;
    let mut path = Vec::new();
    collect_preference_audit_map(before, after, &mut path, &mut changes, &mut total);
    if total == 0 {
        return None;
    }
    changes.sort_unstable_by(|left, right| compare_audit_paths(&left.path, &right.path));

    let mut text = String::new();
    let mut entry = String::new();
    let mut emitted = 0;
    for change in &changes {
        entry.clear();
        write_preference_audit_change(change, &mut entry);
        let remaining_after = total.saturating_sub(emitted + 1);
        let suffix_bytes = if remaining_after == 0 {
            0
        } else {
            omitted_audit_suffix_bytes(remaining_after, true)
        };
        let separator_bytes = usize::from(emitted > 0) * 2;
        let next_len = text
            .len()
            .saturating_add(separator_bytes)
            .saturating_add(entry.len())
            .saturating_add(suffix_bytes);
        if next_len > PREFERENCE_AUDIT_MAX_BYTES {
            break;
        }
        if emitted > 0 {
            text.push_str("; ");
        }
        text.push_str(&entry);
        emitted += 1;
        if emitted == PREFERENCE_AUDIT_MAX_CHANGES {
            break;
        }
    }

    let omitted = total.saturating_sub(emitted);
    if omitted > 0 {
        if !text.is_empty() {
            text.push_str("; ");
        }
        write!(text, "+{omitted} omitted").expect("writing to a String cannot fail");
    }
    Some(PreferenceAuditSummary { text })
}

fn omitted_audit_suffix_bytes(count: usize, has_entry: bool) -> usize {
    let separator = if has_entry { 2 } else { 0 };
    separator + 1 + count.ilog10() as usize + " omitted".len()
}

fn collect_preference_audit_map<'a>(
    before: &'a serde_json::Map<String, serde_json::Value>,
    after: &'a serde_json::Map<String, serde_json::Value>,
    path: &mut Vec<&'a str>,
    changes: &mut Vec<PreferenceAuditChange<'a>>,
    total: &mut usize,
) {
    for (key, old) in before {
        path.push(key);
        collect_preference_audit_change(Some(old), after.get(key), path, changes, total);
        path.pop();
    }
    for (key, new) in after {
        if before.contains_key(key) {
            continue;
        }
        path.push(key);
        collect_preference_audit_change(None, Some(new), path, changes, total);
        path.pop();
    }
}

fn collect_preference_audit_change<'a>(
    old: Option<&'a serde_json::Value>,
    new: Option<&'a serde_json::Value>,
    path: &mut Vec<&'a str>,
    changes: &mut Vec<PreferenceAuditChange<'a>>,
    total: &mut usize,
) {
    if old == new {
        return;
    }
    if !(path.len() == 1
        && matches!(
            path[0],
            "collapsed" | "debug_panel" | "profile_order" | "rail_preview"
        ))
    {
        if let (Some(before), Some(after)) = (
            old.and_then(serde_json::Value::as_object),
            new.and_then(serde_json::Value::as_object),
        ) {
            collect_preference_audit_map(before, after, path, changes, total);
            return;
        }
    }
    *total = (*total).saturating_add(1);
    if changes.len() < PREFERENCE_AUDIT_MAX_CHANGES {
        changes.push(PreferenceAuditChange {
            path: path.clone(),
            old,
            new,
            scalar: preference_audit_scalar_kind(path),
            count: count_preference_audit_values(old, new),
        });
        return;
    }
    let Some((largest, _)) = changes
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| compare_audit_paths(&left.path, &right.path))
    else {
        return;
    };
    if compare_audit_paths(path, &changes[largest].path).is_lt() {
        changes[largest] = PreferenceAuditChange {
            path: path.clone(),
            old,
            new,
            scalar: preference_audit_scalar_kind(path),
            count: count_preference_audit_values(old, new),
        };
    }
}

fn count_preference_audit_values(
    old: Option<&serde_json::Value>,
    new: Option<&serde_json::Value>,
) -> usize {
    if old == new {
        return 0;
    }
    match (old, new) {
        (Some(serde_json::Value::Object(before)), Some(serde_json::Value::Object(after))) => {
            count_preference_audit_map(before, after)
        }
        (Some(serde_json::Value::Array(before)), Some(serde_json::Value::Array(after))) => {
            let paired = before.iter().zip(after).fold(0usize, |count, (old, new)| {
                count.saturating_add(count_preference_audit_values(Some(old), Some(new)))
            });
            let removed = before[after.len().min(before.len())..]
                .iter()
                .fold(0usize, |count, value| {
                    count.saturating_add(count_preference_audit_payload(value))
                });
            let added = after[before.len().min(after.len())..]
                .iter()
                .fold(0usize, |count, value| {
                    count.saturating_add(count_preference_audit_payload(value))
                });
            paired.saturating_add(removed).saturating_add(added)
        }
        _ => old
            .map_or(0, count_preference_audit_payload)
            .max(new.map_or(0, count_preference_audit_payload)),
    }
    .max(1)
}

fn count_preference_audit_map(
    before: &serde_json::Map<String, serde_json::Value>,
    after: &serde_json::Map<String, serde_json::Value>,
) -> usize {
    let changed_before = before.iter().fold(0usize, |count, (key, old)| {
        count.saturating_add(count_preference_audit_values(Some(old), after.get(key)))
    });
    after
        .iter()
        .filter(|(key, _)| !before.contains_key(*key))
        .fold(changed_before, |count, (_, value)| {
            count.saturating_add(count_preference_audit_payload(value))
        })
}

fn count_preference_audit_payload(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(members) if !members.is_empty() => {
            members.values().fold(0usize, |count, value| {
                count.saturating_add(count_preference_audit_payload(value))
            })
        }
        serde_json::Value::Array(members) if !members.is_empty() => {
            members.iter().fold(0usize, |count, value| {
                count.saturating_add(count_preference_audit_payload(value))
            })
        }
        _ => 1,
    }
}

fn compare_audit_paths(left: &[&str], right: &[&str]) -> std::cmp::Ordering {
    left.iter().cmp(right.iter())
}

fn preference_audit_scalar_kind(path: &[&str]) -> Option<PreferenceAuditScalarKind> {
    if path.iter().any(|member| sensitive_audit_key(member)) {
        return None;
    }
    match path {
        [key]
            if matches!(
                *key,
                "background_bots_ack"
                    | "capture"
                    | "focused_50"
                    | "lowmem"
                    | "raster"
                    | "render_all_warning_ack"
                    | "session_log_file"
                    | "sidecar_50"
            ) =>
        {
            Some(PreferenceAuditScalarKind::Boolean)
        }
        ["nav", key]
            if matches!(
                *key,
                "allow_bank_fetch"
                    | "allow_danger_zones"
                    | "allow_teleports"
                    | "allow_wilderness"
                    | "pause_script_on_manual_walk_abort"
                    | "script_scope_notice_ack"
                    | "show_special_areas"
                    | "survivable_routing"
                    | "survivable_routing_notice_ack"
            ) =>
        {
            Some(PreferenceAuditScalarKind::Boolean)
        }
        ["quester_paths", "enabled"] => Some(PreferenceAuditScalarKind::Boolean),
        ["map_bake"] => Some(PreferenceAuditScalarKind::MapBake),
        ["server_revision"] => Some(PreferenceAuditScalarKind::ServerRevision),
        _ => None,
    }
}

fn sensitive_audit_key(key: &str) -> bool {
    [
        "secret",
        "password",
        "passwd",
        "token",
        "credential",
        "auth",
        "cookie",
        "bearer",
    ]
    .iter()
    .any(|needle| {
        key.as_bytes().windows(needle.len()).any(|part| {
            part.iter()
                .zip(needle.as_bytes())
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
        })
    })
}

fn write_preference_audit_change(change: &PreferenceAuditChange<'_>, output: &mut String) {
    let redacted = change.path.iter().any(|member| sensitive_audit_key(member));
    let key_bytes = change
        .path
        .iter()
        .map(|member| member.len())
        .sum::<usize>()
        .saturating_add(change.path.len().saturating_sub(1));
    if redacted {
        output.push_str("<redacted-key>");
    } else if key_bytes > api::local_state::MAX_PREF_KEY_CHARS {
        output.push_str("<long-key>");
    } else {
        frontend::write_preference_path(&change.path, output);
    }
    let old = change
        .scalar
        .and_then(|kind| preference_audit_scalar(kind, change.old));
    let new = change
        .scalar
        .and_then(|kind| preference_audit_scalar(kind, change.new));
    if let (Some(old), Some(new)) = (old, new) {
        output.push_str(": ");
        write_preference_audit_scalar(old, output);
        output.push_str(" -> ");
        write_preference_audit_scalar(new, output);
    } else {
        let action = match (change.old, change.new) {
            (None, Some(_)) => "set",
            (Some(_), None) => "removed",
            _ => "changed",
        };
        write!(output, ": {action} (count={})", change.count)
            .expect("writing to a String cannot fail");
    }
}

fn preference_audit_scalar<'a>(
    kind: PreferenceAuditScalarKind,
    value: Option<&'a serde_json::Value>,
) -> Option<PreferenceAuditScalar<'a>> {
    match (kind, value) {
        (_, None) => Some(PreferenceAuditScalar::Absent),
        (PreferenceAuditScalarKind::Boolean, Some(serde_json::Value::Bool(value))) => {
            Some(PreferenceAuditScalar::Boolean(*value))
        }
        (PreferenceAuditScalarKind::MapBake, Some(serde_json::Value::String(value)))
            if matches!(value.as_str(), "ask" | "always") =>
        {
            Some(PreferenceAuditScalar::MapBake(value))
        }
        (PreferenceAuditScalarKind::ServerRevision, Some(serde_json::Value::Number(value))) => {
            value
                .as_u64()
                .filter(|revision| *revision <= u16::MAX.into())
                .map(PreferenceAuditScalar::ServerRevision)
        }
        _ => None,
    }
}

fn write_preference_audit_scalar(value: PreferenceAuditScalar<'_>, output: &mut String) {
    match value {
        PreferenceAuditScalar::Absent => output.push_str("absent"),
        PreferenceAuditScalar::Boolean(value) => {
            write!(output, "{value}").expect("writing to a String cannot fail");
        }
        PreferenceAuditScalar::MapBake(value) => output.push_str(value),
        PreferenceAuditScalar::ServerRevision(value) => {
            write!(output, "{value}").expect("writing to a String cannot fail");
        }
    }
}

#[cfg(test)]
mod preference_audit_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_shows_approved_scalars_and_hides_payloads() {
        let before = json!({
            "capture": false,
            "legacy.key": false,
            "map_bake": "ask",
            "quester_paths": { "enabled": false, "folder": "/private/source" },
            "debug_panel": { "pins": ["old pin"], "recent": [] },
            "secret_token": "old-sensitive-value",
            "unknown": "old-unknown-value",
            "weird": { "bad\nkey;": "old-payload" },
        });
        let after = json!({
            "capture": true,
            "legacy.key": true,
            "map_bake": "always",
            "quester_paths": { "enabled": true, "folder": "/private/changed" },
            "debug_panel": { "pins": ["new pin"], "recent": ["new recent"] },
            "secret_token": "new-sensitive-value",
            "unknown": "new-unknown-value",
            "weird": { "bad\nkey;": "new-payload" },
        });

        let summary = preference_audit_summary(
            before.as_object().expect("an object"),
            after.as_object().expect("an object"),
        )
        .expect("a changed preference document");
        assert!(summary.text.contains("capture: false -> true"));
        assert!(summary.text.contains("legacy\\.key: changed (count=1)"));
        assert!(summary
            .text
            .contains("weird.bad\\nkey\\;: changed (count=1)"));
        assert!(!summary.text.contains('\n'));
        assert!(summary.text.contains("map_bake: ask -> always"));
        assert!(summary
            .text
            .contains("quester_paths.enabled: false -> true"));
        assert!(summary.text.contains("quester_paths.folder: changed"));
        assert!(summary.text.contains("<redacted-key>: changed (count=1)"));
        assert!(summary.text.contains("debug_panel: changed (count=2)"));
        assert!(!summary.text.contains("old pin"));
        assert!(!summary.text.contains("new recent"));
        assert!(!summary.text.contains("/private/"));
        assert!(!summary.text.contains("sensitive-value"));
        assert!(!summary.text.contains("unknown-value"));
        assert!(preference_audit_summary(
            before.as_object().expect("an object"),
            before.as_object().expect("an object"),
        )
        .is_none());
    }

    #[test]
    fn summary_stays_within_byte_and_change_limits() {
        let before: serde_json::Map<String, serde_json::Value> = (0..40)
            .map(|index| (format!("key{index:02}"), json!(0)))
            .collect();
        let after: serde_json::Map<String, serde_json::Value> = (0..40)
            .map(|index| (format!("key{index:02}"), json!(1)))
            .collect();

        let summary = preference_audit_summary(&before, &after).expect("changed keys");
        assert!(summary.text.len() <= PREFERENCE_AUDIT_MAX_BYTES);
        assert!(summary.text.ends_with("+8 omitted"));
        let long_before: serde_json::Map<String, serde_json::Value> = (0..40)
            .map(|index| (format!("{}-{index:02}", "k".repeat(120)), json!(0)))
            .collect();
        let long_after: serde_json::Map<String, serde_json::Value> = (0..40)
            .map(|index| (format!("{}-{index:02}", "k".repeat(120)), json!(1)))
            .collect();
        let long = preference_audit_summary(&long_before, &long_after).expect("long keys");
        assert!(long.text.len() <= PREFERENCE_AUDIT_MAX_BYTES);
        assert!(long.text.ends_with(" omitted"));
    }
}

impl Shared {
    /// Queue a readback of the admitted store `key` (unless one is queued
    /// already) and make sure the writer is up. What the store's completed
    /// readback count must reach for a read that began after this call, or
    /// `None` when the writer cannot run.
    fn request_readback(self: &Arc<Self>, state: &mut State, key: &StoreKey) -> Option<u64> {
        if !self.start_writer(state) {
            return None;
        }
        let seq = state.next_seq();
        let store = state.stores.get_mut(key)?;
        let target = match store.readback {
            Readback::Idle => {
                store.readback = Readback::Queued;
                store.readback_seq = seq;
                store.readbacks + 1
            }
            Readback::Queued => store.readbacks + 1,
            Readback::InFlight | Readback::InFlightAgain => {
                store.readback = Readback::InFlightAgain;
                store.readbacks + 2
            }
        };
        self.wake.notify_one();
        Some(target)
    }

    /// Start the writer thread unless it runs. `false` when it cannot run
    /// (spawn refused, or the service is stopping): nothing is queued then.
    fn start_writer(self: &Arc<Self>, state: &mut State) -> bool {
        match state.writer {
            WriterPhase::Running => return true,
            WriterPhase::Exited => return false,
            WriterPhase::NotStarted if state.shutdown => return false,
            WriterPhase::NotStarted => {}
        }
        let shared = Arc::clone(self);
        match thread::Builder::new()
            .name("local-state-writer".into())
            .spawn(move || shared.run())
        {
            Ok(handle) => {
                state.writer = WriterPhase::Running;
                *self.writer.lock() = Some(handle);
                true
            }
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: writer thread did not start: {error}"
                );
                false
            }
        }
    }

    /// The writer thread: take the next readback or write of any store,
    /// do its I/O with no lock held, record the outcome, and park when
    /// nothing is queued. A job that panics is contained: its request
    /// fails and the store reads its file again. Once the service is
    /// dropped, only hydration needed by accepted writes continues;
    /// the thread exits when every accepted write settles.
    fn run(self: Arc<Self>) {
        let mut after_shutdown = 0usize;
        loop {
            let (job, stopping) = {
                let mut state = self.state.lock();
                loop {
                    if let Some(job) = take_job(&mut state) {
                        break (job, state.shutdown);
                    }
                    if state.shutdown {
                        state.writer = WriterPhase::Exited;
                        drop(state);
                        self.settled.notify_all();
                        if after_shutdown > 0 {
                            host_log!(
                                Category::Lifecycle,
                                Level::Info,
                                "local state: writer stopped after finishing {after_shutdown} write(s) at shutdown"
                            );
                        }
                        return;
                    }
                    self.wake.wait(&mut state);
                }
            };
            if stopping && matches!(job, Job::Write(..)) {
                after_shutdown += 1;
            }
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| match &job {
                Job::Readback(key) => self.readback(key),
                Job::Write(key, queued) => self.write(key, queued),
            }));
            if let Err(payload) = outcome {
                self.contain(&job, payload.as_ref());
            }
            self.settled.notify_all();
        }
    }

    /// A job panicked: settle what it held, so no accepted request is left
    /// without a terminal and no flag is left set.
    fn contain(self: &Arc<Self>, job: &Job, payload: &(dyn Any + Send)) {
        let message = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a non-text panic".into());
        let mut state = self.state.lock();
        match job {
            Job::Readback(key) => {
                if let Some(store) = state.stores.get_mut(key) {
                    store.readback = Readback::Idle;
                    store.readbacks += 1;
                }
                host_log!(
                    Category::Lifecycle,
                    Level::Error,
                    "local state: reading {} panicked: {message}",
                    key.domain()
                );
            }
            Job::Write(key, queued) => {
                // What the store holds is no longer known: fail the write
                // and those queued behind it (accepted against that state),
                // and read the file again.
                let domain = key.domain();
                let mut failed = vec![queued.terminal()];
                if let Some(store) = state.stores.get_mut(key) {
                    store.in_flight = None;
                    store.file = FileState::Cold;
                    failed.extend(store.queue.drain(..).map(|queued| queued.terminal()));
                }
                let settlement = Settlement::Failed(ErrorCode::Failed {
                    domain,
                    reason: FailureReason::Dirty,
                });
                let mut terminals = Vec::new();
                for terminal in failed {
                    if state.settle(key, terminal.request, settlement.clone(), None) {
                        terminals.push(terminal);
                    }
                }
                if !state.shutdown {
                    self.request_readback(&mut state, key);
                }
                drop(state);
                host_log!(
                    Category::Lifecycle,
                    Level::Error,
                    "local state: writing {domain} panicked: {message}; the request failed and \
                     the store reads its file again"
                );
                for terminal in &terminals {
                    self.terminal(terminal, &settlement, None);
                }
            }
        }
    }

    fn readback(&self, key: &StoreKey) {
        let recover = self
            .state
            .lock()
            .stores
            .get_mut(key)
            .is_some_and(|store| std::mem::take(&mut store.recover));
        let read_at = DurableTimestamp::now();
        let read = self.read_repairing(key, read_at, recover, None);
        let path = key.path(&self.root);
        let mut state = self.state.lock();
        state.adopt(key, read, read_at, &path);
        let seq = state.next_seq();
        if let Some(store) = state.stores.get_mut(key) {
            store.readbacks += 1;
            store.readback = match store.readback {
                Readback::InFlightAgain => {
                    store.readback_seq = seq;
                    Readback::Queued
                }
                _ => Readback::Idle,
            };
        }
    }

    /// Read `key`'s file, first repairing, under the file's
    /// [`files::WriteLock`], what the service repairs rather than refuses:
    /// a loadouts file whose rows share a name is renamed apart, on every
    /// read ([`Self::dedupe_loadouts`]); a preferences file whose content is
    /// malformed (not JSON, or JSON that is not an object) is quarantined
    /// and started fresh, only on the read a save makes (`recover`,
    /// [`LocalStateService::start_readback`];
    /// [`Self::quarantine_preferences`]). The file is then read as the
    /// repair left it. `held` is the lock when the caller (a write) holds it
    /// already; otherwise the repair takes it. Every other refusal (owner,
    /// mode, size, an I/O error, content the service cannot hold whole)
    /// stands, and a repair that cannot be made leaves the file as it is,
    /// refused, and says why.
    fn read_repairing(
        &self,
        key: &StoreKey,
        read_at: DurableTimestamp,
        recover: bool,
        held: Option<&files::WriteLock>,
    ) -> Result<Option<ReadDocument>, ReadError> {
        #[cfg(any(test, feature = "test-support"))]
        {
            let read_stall = self.state.lock().read_stall.clone();
            if let Some(stall) = read_stall {
                stall();
            }
        }
        let read = files::read(key, &self.root, read_at);
        let Some(repair) = Repair::of(key, &read, recover) else {
            return read;
        };
        let path = key.path(&self.root);
        let _lock = if held.is_some() {
            None
        } else {
            let taken = files::lock_for_write(&path, Instant::now() + files::LOCK_TIMEOUT, &|| {
                self.state.lock().abandoned
            });
            match taken {
                Ok(lock) => Some(lock),
                Err(error) => {
                    self.notice(
                        Level::Warn,
                        format!(
                            "local state: {}: its lock was not taken ({error:?}); it is left \
                             as it is path={}",
                            repair.unmade(),
                            path.display()
                        ),
                    );
                    return read;
                }
            }
        };
        let repaired = match repair {
            Repair::Dedupe => self.dedupe_loadouts(key, &path),
            Repair::Quarantine => self.quarantine_preferences(&path),
        };
        match repaired {
            Ok(()) => files::read(key, &self.root, read_at),
            Err(error) => {
                self.notice(
                    Level::Warn,
                    format!(
                        "local state: {}: {error}; it is left as it is path={}",
                        repair.unmade(),
                        path.display()
                    ),
                );
                read
            }
        }
    }

    /// Under the loadouts file's lock: when `path` still holds rows that
    /// share a name, rename them apart in one publication that loses
    /// nothing (the one-time repair of a file an older editor saved with
    /// two loadouts of one name; names key the rows, so until then no edit
    /// of it names one row). The file's exact bytes are kept first in a new
    /// `loadouts.json.duplicates-<time>` beside it, read back, flushed, and
    /// its directory entry flushed. Then the same rows, each with its
    /// content and in its place, are published with every later row of a
    /// name renamed `<name> (2)`, `(3)`, … ([`files::rename_apart`]), only
    /// while the file still holds the bytes kept, and the directory entry
    /// is flushed. The operator is told (a warning naming the copy and each
    /// rename) and the repair is audited (one info line). A failure before
    /// the publication leaves the file as it is (a copy already made stays,
    /// holding the file's exact bytes), and the next read tries again.
    fn dedupe_loadouts(&self, key: &StoreKey, path: &Path) -> std::io::Result<()> {
        let Some((text, mut rows)) = files::loadouts_sharing_names(path)? else {
            // Repaired or removed meanwhile: read it as it is.
            return Ok(());
        };
        let renamed = files::rename_apart(&mut rows);
        let bytes = files::loadouts_bytes(&rows).map_err(std::io::Error::other)?;
        if bytes.len() as u64 > files::MAX_LOADOUTS_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "renamed apart, its rows would take {} bytes, more than a loadouts file may \
                     hold ({})",
                    bytes.len(),
                    files::MAX_LOADOUTS_BYTES
                ),
            ));
        }
        let quarantined = files::quarantine_copy(path, text.as_bytes(), "duplicates")?;
        let kept_in = |error: std::io::Error, what: &str| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "a copy is kept in {}, but {what}: {error}",
                    quarantined.display()
                ),
            )
        };
        let dirs: Vec<PathBuf> = path.parent().map(Path::to_path_buf).into_iter().collect();
        self.sync_dirs(&dirs).map_err(|error| {
            kept_in(
                error,
                "its directory entry did not reach the disk, so the file was not replaced",
            )
        })?;
        let kept = files::digest(text.as_bytes());
        self.publish(path, &bytes, || {
            if self.state.lock().abandoned {
                return Err(std::io::Error::other(
                    "the service passed its shutdown bound",
                ));
            }
            if files::current_digest(key, &self.root)? == Some(kept) {
                Ok(())
            } else {
                Err(std::io::Error::other(
                    "the file changed while its loadouts were being renamed apart",
                ))
            }
        })
        .map_err(|error| kept_in(error, "the file was not replaced"))?;
        let renames = renamed
            .iter()
            .map(|rename| format!("row {} {:?} → {:?}", rename.row + 1, rename.from, rename.to))
            .collect::<Vec<_>>()
            .join(", ");
        let told = format!(
            "local state: loadouts file {} had loadouts that share a name; its bytes are kept in \
             {}, and each later one was renamed, keeping its gear and its place: {renames}",
            path.display(),
            quarantined.display()
        );
        match self.sync_dirs(&dirs) {
            Ok(()) => {
                self.notice(Level::Warn, told);
                self.notice(
                    Level::Info,
                    format!(
                        "audit domain={} action=loadouts-dedupe path={} quarantined={} \
                         renamed=[{renames}] at={}",
                        LocalStateDomain::LoadoutsReferences,
                        path.display(),
                        quarantined.display(),
                        DurableTimestamp::now().unix_seconds()
                    ),
                );
            }
            // The renamed rows are in place, and the copy is durable: a
            // crash that loses this entry leaves the old file, and the next
            // read renames it apart again.
            Err(error) => self.notice(
                Level::Warn,
                format!(
                    "{told}; but its directory entry did not reach the disk ({error}), so the \
                     renames are not durable yet"
                ),
            ),
        }
        Ok(())
    }

    /// Under the preferences file's lock: if `path` still holds malformed
    /// content, keep its exact bytes in a new `<file>.corrupt-<time>`
    /// beside it (flushed, and its directory entry flushed), then replace
    /// the file with fresh, empty preferences, but only while it still
    /// holds those bytes. Tells the operator (a warning) and audits the
    /// quarantine (one info line naming both paths). A failure before the
    /// copy is on disk leaves the file as it is.
    fn quarantine_preferences(&self, path: &Path) -> std::io::Result<()> {
        let Some(bytes) = files::corrupt_preferences(path)? else {
            // Repaired or removed meanwhile: read it as it is.
            return Ok(());
        };
        let quarantined = files::quarantine_copy(path, &bytes, "corrupt")?;
        if let Some(dir) = path.parent() {
            self.sync_dirs(&[dir.to_path_buf()])?;
        }
        let corrupt = files::digest(&bytes);
        let fresh =
            files::preferences_bytes(&serde_json::Map::new()).map_err(std::io::Error::other)?;
        self.publish(path, &fresh, || {
            let now = files::corrupt_preferences(path)?;
            if now.as_deref().map(files::digest) == Some(corrupt) {
                Ok(())
            } else {
                Err(std::io::Error::other(
                    "the file changed while it was being moved aside",
                ))
            }
        })
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "a copy is kept in {}, but the file was not replaced: {error}",
                    quarantined.display()
                ),
            )
        })?;
        self.notice(
            Level::Warn,
            format!(
                "local state: preferences file {} was not valid preferences JSON; its bytes are \
                 kept in {} and fresh preferences were started",
                path.display(),
                quarantined.display()
            ),
        );
        self.notice(
            Level::Info,
            format!(
                "audit domain={} action=preferences-quarantine path={} quarantined={} at={}",
                LocalStateDomain::Preferences,
                path.display(),
                quarantined.display(),
                DurableTimestamp::now().unix_seconds()
            ),
        );
        Ok(())
    }

    /// Log one line of a repair (the operator's notice, the audit line, or
    /// why the repair could not be made), and hand it to a test's hook.
    fn notice(&self, level: Level, line: String) {
        host_log!(Category::Lifecycle, level, "{line}");
        #[cfg(test)]
        {
            let hook = self.state.lock().hooks.notice.clone();
            if let Some(hook) = hook {
                hook(level, &line);
            }
        }
    }

    /// One write, as one transaction under the file's [`files::WriteLock`]:
    /// read the file, decide, stage and flush the bytes, check the file
    /// once more, rename, flush the directory entries, read the file back.
    /// Only a write whose bytes the file still holds at the end is
    /// acknowledged `Completed`; a change by a writer that did not take the
    /// lock settles it `Conflict`.
    fn write(&self, key: &StoreKey, queued: &Queued) {
        let domain = key.domain();
        let path = key.path(&self.root);
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
                    path.display()
                );
                self.finish(key, queued, io_failure(domain), false);
                return;
            }
        }
        let guard =
            match files::lock_for_write(&path, Instant::now() + files::LOCK_TIMEOUT, &|| {
                self.state.lock().abandoned
            }) {
                Ok(lock) => lock,
                Err(LockError::Abandoned) => {
                    // The shutdown bound passed while it waited: it was settled
                    // cancelled then.
                    self.finish(
                        key,
                        queued,
                        Settlement::Cancelled(Cancellation::Shutdown),
                        false,
                    );
                    return;
                }
                Err(LockError::TimedOut) => {
                    host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "local state: {domain} write waited {:?} for another writer's lock \
                     path={}; not written",
                        files::LOCK_TIMEOUT,
                        files::lock_path(&path).display()
                    );
                    self.finish(
                        key,
                        queued,
                        Settlement::Failed(ErrorCode::Busy { domain }),
                        false,
                    );
                    return;
                }
                Err(LockError::Io(error)) => {
                    host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "local state: {domain} write could not lock path={}: {error}",
                        files::lock_path(&path).display()
                    );
                    self.finish(key, queued, io_failure(domain), false);
                    return;
                }
            };
        // The precondition: what the file holds now, under the lock (rows
        // another writer left sharing a name are renamed apart first, so
        // this edit is never decided against them).
        let read_at = DurableTimestamp::now();
        let read = self.read_repairing(key, read_at, false, Some(&guard));
        let decision = {
            let mut state = self.state.lock();
            state.adopt(key, read, read_at, &path);
            state.stores.get(key).map(|store| {
                let file = match &store.file {
                    FileState::Read(cached) => {
                        DecisionFile::Read(CachedDecision::from_cached(cached, key.domain()))
                    }
                    FileState::Rejected(rejection) => DecisionFile::Rejected(rejection.reason),
                    FileState::Cold => DecisionFile::Cold,
                };
                (file, store.external)
            })
        };
        let Some((file, external)) = decision else {
            self.finish(
                key,
                queued,
                Settlement::Failed(ErrorCode::Failed {
                    domain,
                    reason: FailureReason::Dirty,
                }),
                false,
            );
            return;
        };
        let mut staged = match decide(key, &file, external, queued) {
            Ok(staged) => staged,
            Err(settlement) => {
                self.finish(key, queued, settlement, false);
                return;
            }
        };
        if staged.advance_version {
            let version = self.state.lock().next_version();
            match version {
                Some(version) => staged.version = version,
                None => {
                    // The version space is spent: refuse, never wrap.
                    self.finish(
                        key,
                        queued,
                        Settlement::Failed(ErrorCode::Busy { domain }),
                        false,
                    );
                    return;
                }
            }
        }
        if staged.unchanged {
            // These bytes are already durable, written by this service: no
            // write, and no commit to audit.
            self.finish(
                key,
                queued,
                Settlement::Completed(DurableReceipt::new(
                    queued.intent.identity().clone(),
                    staged.previous,
                    DurableTimestamp::now(),
                    Some(request),
                )),
                false,
            );
            return;
        }
        let mut refusal = None;
        let published = self.publish(&path, &staged.bytes, || {
            self.last_check(key, &path, staged.precondition)
                .map_err(|check| {
                    let error = std::io::Error::other(check.to_string());
                    refusal = Some(check);
                    error
                })
        });
        if let Err(error) = published {
            match refusal {
                Some(LastCheck::Abandoned) => self.finish(
                    key,
                    queued,
                    Settlement::Cancelled(Cancellation::Shutdown),
                    false,
                ),
                Some(LastCheck::Changed) => {
                    self.conflict_on_change(
                        key,
                        queued,
                        &path,
                        &guard,
                        "while this write was staged",
                    );
                }
                Some(LastCheck::Io(_)) | None => {
                    host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "local state: {domain} write failed path={}: {error}",
                        path.display()
                    );
                    self.finish(key, queued, io_failure(domain), false);
                }
            }
            return;
        }
        // Renamed. Now the directory entries: the whole chain up to the
        // root's parent unless this service had already made the file's
        // entry durable (a write that created the file, or one after an
        // earlier write left the chain unfinished, flushes every link).
        let dirs = if staged.durable_before {
            path.parent().map(Path::to_path_buf).into_iter().collect()
        } else {
            files::entry_chain(&path, &self.root)
        };
        let synced = self.sync_dirs(&dirs);
        // Read it back: bytes the file no longer holds are not acknowledged.
        let written = files::digest(&staged.bytes);
        if files::current_digest(key, &self.root).ok() != Some(Some(written)) {
            self.conflict_on_change(
                key,
                queued,
                &path,
                &guard,
                "while its directory entry was flushed",
            );
            return;
        }
        {
            let mut state = self.state.lock();
            if let Some(store) = state.stores.get_mut(key) {
                // The file holds these bytes now, durable or not.
                store.file = FileState::Read(Cached {
                    version: staged.version,
                    at: staged.at,
                    snapshot: staged.snapshot,
                    preferences: staged.preferences,
                    refreshed_at: Instant::now(),
                    digest: Some(written),
                    durable: synced.is_ok(),
                    written_len: staged.bytes.len(),
                    rewrite: Rewrite::Lossless,
                });
            }
        }
        if let Err(error) = synced {
            host_log!(
                Category::Lifecycle,
                Level::Warn,
                "local state: {domain} written but its directory entry did not reach the disk; \
                 the write is not durable path={}: {error}",
                path.display()
            );
            self.finish(key, queued, io_failure(domain), false);
            return;
        }
        let now = DurableTimestamp::now();
        let identity = queued.intent.identity().clone();
        if let Some(audit) = &staged.audit {
            self.audit(
                AuditCommit::after_commit(
                    identity.clone(),
                    staged.action,
                    staged.previous,
                    staged.version,
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
                staged.version,
                now,
                Some(request),
            )),
            true,
        );
    }

    /// The check immediately before the rename, inside the publication:
    /// the file still holds what the write was decided against, and the
    /// shutdown bound has not passed. On success the write is marked as
    /// being renamed into place.
    fn last_check(
        &self,
        key: &StoreKey,
        path: &Path,
        precondition: Option<ContentDigest>,
    ) -> Result<(), LastCheck> {
        #[cfg(test)]
        {
            let hook = self.state.lock().hooks.publish.clone();
            if let Some(hook) = hook {
                hook(path);
            }
        }
        #[cfg(not(test))]
        let _ = path;
        if files::current_digest(key, &self.root).map_err(LastCheck::Io)? != precondition {
            return Err(LastCheck::Changed);
        }
        let mut state = self.state.lock();
        if state.abandoned {
            return Err(LastCheck::Abandoned);
        }
        if let Some(in_flight) = state
            .stores
            .get_mut(key)
            .and_then(|store| store.in_flight.as_mut())
        {
            in_flight.publishing = true;
        }
        Ok(())
    }

    /// A writer that does not take the lock changed the file during this
    /// write (which holds `lock`): take what it left into the store (an
    /// outside change at a new version) and settle the write as the
    /// conflict it is.
    fn conflict_on_change(
        &self,
        key: &StoreKey,
        queued: &Queued,
        path: &Path,
        lock: &files::WriteLock,
        when: &str,
    ) {
        let domain = key.domain();
        let read_at = DurableTimestamp::now();
        let read = self.read_repairing(key, read_at, false, Some(lock));
        let settlement = {
            let mut state = self.state.lock();
            state.adopt(key, read, read_at, path);
            match state.stores.get(key).map(|store| &store.file) {
                Some(FileState::Read(cached)) => Settlement::Failed(ErrorCode::Conflict {
                    expected: queued.intent.expected().unwrap_or(queued.accepted_at),
                    found: cached.version,
                }),
                Some(FileState::Rejected(rejection)) => Settlement::Failed(ErrorCode::Failed {
                    domain,
                    reason: rejection.reason,
                }),
                Some(FileState::Cold) | None => Settlement::Failed(ErrorCode::Failed {
                    domain,
                    reason: FailureReason::Dirty,
                }),
            }
        };
        host_log!(
            Category::Lifecycle,
            Level::Warn,
            "local state: {domain} changed outside this service {when}; the write is not \
             acknowledged and the file keeps the other writer's bytes path={}",
            path.display()
        );
        self.finish(key, queued, settlement, false);
    }

    /// Settle the writer's edit, once, and log its terminal: an edit that
    /// committed has its audit line already; one cancelled at the shutdown
    /// bound has its terminal already.
    fn finish(
        &self,
        key: &StoreKey,
        queued: &Queued,
        settlement: WriteSettlement,
        committed: bool,
    ) {
        let mut state = self.state.lock();
        let abandoned = state.abandoned;
        let cancelled_at_bound = state
            .stores
            .get_mut(key)
            .and_then(|store| store.in_flight.take())
            .is_some_and(|in_flight| abandoned && !in_flight.publishing);
        if cancelled_at_bound {
            return;
        }
        if state.settle(key, queued.request, settlement.clone(), None) && !committed {
            drop(state);
            self.terminal(&queued.terminal(), &settlement, None);
        }
    }

    /// Log one write's terminal line (and hand it to a test's hook).
    fn terminal(
        &self,
        terminal: &Terminal,
        settlement: &WriteSettlement,
        successor: Option<RequestId>,
    ) {
        let (level, line) = terminal.line(settlement, successor);
        host_log!(Category::Lifecycle, level, "{line}");
        #[cfg(test)]
        {
            let hook = self.state.lock().hooks.terminal.clone();
            if let Some(hook) = hook {
                hook(&line);
            }
        }
    }

    fn sync_dirs(&self, dirs: &[PathBuf]) -> std::io::Result<()> {
        #[cfg(test)]
        let hook = self.state.lock().hooks.sync.clone();
        for dir in dirs {
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook(dir)?;
                continue;
            }
            files::sync_directory(dir)?;
        }
        Ok(())
    }

    fn audit(&self, audit: AuditCommit, kind: &AuditKind) {
        match kind {
            AuditKind::Plain => host_log!(Category::Lifecycle, Level::Info, "{audit}"),
            AuditKind::Preferences(summary) => host_log!(
                Category::Lifecycle,
                Level::Info,
                "{audit}; preferences: {summary}"
            ),
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            let hook = self.state.lock().hooks.audit.clone();
            if let Some(hook) = hook {
                hook(&audit);
            }
        }
    }

    /// The shutdown bound passed: settle what the writer has not begun to
    /// rename into place, each with its terminal line. How many were
    /// cancelled, and how many are being renamed.
    fn abandon(&self) -> (usize, usize) {
        let (terminals, publishing) = self.state.lock().abandon();
        let cancelled = Settlement::Cancelled(Cancellation::Shutdown);
        for terminal in &terminals {
            self.terminal(terminal, &cancelled, None);
        }
        (terminals.len(), publishing)
    }

    #[cfg(not(test))]
    fn publish(
        &self,
        path: &Path,
        bytes: &[u8],
        check: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        vault::write_private_file_checked(path, bytes, check)
    }

    /// The real publication, unless a test replaced its rename: the hook
    /// then runs after the last check, in place of the rename.
    #[cfg(test)]
    fn publish(
        &self,
        path: &Path,
        bytes: &[u8],
        check: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let hook = self.state.lock().hooks.write.clone();
        match hook {
            Some(hook) => {
                check()?;
                hook(path, bytes)
            }
            None => vault::write_private_file_checked(path, bytes, check),
        }
    }
}

/// Why the check before the rename stopped a write.
#[derive(Debug, thiserror::Error)]
enum LastCheck {
    #[error("the file changed while the write was staged")]
    Changed,
    #[error("the service passed its shutdown bound")]
    Abandoned,
    #[error("the file could not be read before the rename: {0}")]
    Io(std::io::Error),
}

/// What a read repairs, under the file's lock, rather than refuses
/// ([`Shared::read_repairing`]).
#[derive(Debug, Clone, Copy)]
enum Repair {
    /// Loadout rows share a name: they are renamed apart
    /// ([`Shared::dedupe_loadouts`]), on every read.
    Dedupe,
    /// A malformed preferences file, read for a save: it is moved aside
    /// and fresh preferences are started ([`Shared::quarantine_preferences`]).
    Quarantine,
}

impl Repair {
    /// The repair a read of `key` that found `read` calls for, if any.
    fn of(
        key: &StoreKey,
        read: &Result<Option<ReadDocument>, ReadError>,
        recover: bool,
    ) -> Option<Self> {
        match (key, read) {
            (StoreKey::Loadouts, Err(ReadError::SharedNames { .. })) => Some(Repair::Dedupe),
            (StoreKey::Preferences, Err(error)) if recover && error.is_malformed() => {
                Some(Repair::Quarantine)
            }
            _ => None,
        }
    }

    /// What the file holds and that the repair was not made, for the line
    /// saying why.
    fn unmade(self) -> &'static str {
        match self {
            Repair::Dedupe => {
                "loadouts file has loadouts that share a name, but they could not be renamed apart"
            }
            Repair::Quarantine => "preferences file is malformed but could not be moved aside",
        }
    }
}

fn io_failure(domain: LocalStateDomain) -> WriteSettlement {
    Settlement::Failed(ErrorCode::Failed {
        domain,
        reason: FailureReason::Io,
    })
}

/// Decide `queued` against a worker-owned copy of the latest file state.
/// Preference cloning, patching, serialization and typed projection run
/// after the service lock is released.
struct CachedDecision {
    version: StoreVersion,
    snapshot: Option<LeaseSnapshot>,
    preferences: Option<files::PreferenceProjection>,
    digest: Option<ContentDigest>,
    durable: bool,
    written_len: usize,
    rewrite: Rewrite,
}

impl CachedDecision {
    fn from_cached(cached: &Cached, domain: LocalStateDomain) -> Self {
        let needs_snapshot = domain == LocalStateDomain::LoadoutsReferences
            || (domain == LocalStateDomain::Preferences && cached.preferences.is_none());
        Self {
            version: cached.version,
            snapshot: needs_snapshot.then(|| cached.snapshot.clone()),
            preferences: cached.preferences.clone(),
            digest: cached.digest,
            durable: cached.durable,
            written_len: cached.written_len,
            rewrite: cached.rewrite,
        }
    }
}

enum DecisionFile {
    Read(CachedDecision),
    Rejected(FailureReason),
    Cold,
}

fn decide(
    key: &StoreKey,
    file: &DecisionFile,
    external: u64,
    queued: &Queued,
) -> Result<Staged, WriteSettlement> {
    let domain = key.domain();
    let cached = match file {
        DecisionFile::Read(cached) => cached,
        DecisionFile::Rejected(reason) => {
            return Err(Settlement::Failed(ErrorCode::Failed {
                domain,
                reason: *reason,
            }))
        }
        DecisionFile::Cold => {
            return Err(Settlement::Failed(ErrorCode::Failed {
                domain,
                reason: FailureReason::Dirty,
            }))
        }
    };
    if matches!(&queued.intent, WriteIntent::Port(_)) && external != queued.external {
        // Port edits are versioned against the observed file; preference
        // patches are member intents and rebase on the worker's latest read.
        return Err(Settlement::Failed(ErrorCode::Conflict {
            expected: queued.intent.expected().unwrap_or(queued.accepted_at),
            found: cached.version,
        }));
    }
    if let WriteIntent::Port(edit) = &queued.intent {
        if !edit.applies_to(cached.version) {
            return Err(Settlement::Failed(ErrorCode::Conflict {
                expected: edit
                    .expected()
                    .expect("a blind write applies to any version"),
                found: cached.version,
            }));
        }
    }
    if matches!(&cached.rewrite, Rewrite::Refused { .. }) {
        return Err(Settlement::Failed(ErrorCode::Failed {
            domain,
            reason: FailureReason::Schema,
        }));
    }
    let (bytes, snapshot, at, action, preferences, audit) = match &queued.intent {
        WriteIntent::Port(DomainEdit::Bank(hint)) => {
            let bytes = files::hint_bytes(key, hint.rows(), hint.observed_at())
                .map_err(|_| schema_failure(domain))?;
            let snapshot = LeaseSnapshot::bank_hints(hint.rows().to_vec())
                .expect("an admitted hint already met the row rules");
            (
                bytes,
                snapshot,
                hint.observed_at(),
                CommitAction::BankHintWrite,
                None,
                Some(AuditKind::Plain),
            )
        }
        WriteIntent::Port(DomainEdit::Loadout(edit)) => {
            let rows = cached
                .snapshot
                .as_ref()
                .and_then(LeaseSnapshot::as_loadouts)
                .unwrap_or(&[]);
            let next = files::apply_loadout_edit(rows, edit).map_err(|refusal| {
                Settlement::Failed(match refusal {
                    RenameRefusal::MissingSource => ErrorCode::Conflict {
                        expected: edit.expected().unwrap_or(cached.version),
                        found: cached.version,
                    },
                    RenameRefusal::NameTaken => ErrorCode::Validation {
                        domain,
                        reason: ValidationReason::DuplicateId,
                    },
                })
            })?;
            let bytes = files::loadouts_bytes(&next).map_err(|_| schema_failure(domain))?;
            if next.len() > files::MAX_LOADOUT_ROWS && next.len() > rows.len() {
                return Err(Settlement::Failed(ErrorCode::Validation {
                    domain,
                    reason: ValidationReason::TooManyRows,
                }));
            }
            if bytes.len() as u64 > files::MAX_LOADOUTS_BYTES && bytes.len() > cached.written_len {
                return Err(Settlement::Failed(ErrorCode::Validation {
                    domain,
                    reason: ValidationReason::TooLong,
                }));
            }
            (
                bytes,
                LeaseSnapshot::loadouts(next),
                DurableTimestamp::now(),
                CommitAction::LoadoutWrite,
                None,
                Some(AuditKind::Plain),
            )
        }
        WriteIntent::Port(DomainEdit::Preferences(edit)) => {
            let before =
                current_preference_document(cached).ok_or_else(|| schema_failure(domain))?;
            let mut document = before.as_ref().clone();
            if files::apply_preferences_edit(&mut document, edit.changes()).is_none() {
                return Err(Settlement::Failed(ErrorCode::Validation {
                    domain,
                    reason: ValidationReason::Schema,
                }));
            }
            let (bytes, snapshot, projection, summary) =
                stage_preference_document(document, &before, cached, domain)?;
            (
                bytes,
                snapshot,
                DurableTimestamp::now(),
                CommitAction::PreferencesWrite,
                Some(projection),
                summary.map(AuditKind::Preferences),
            )
        }
        WriteIntent::Preference { patch, .. } => {
            let before =
                current_preference_document(cached).ok_or_else(|| schema_failure(domain))?;
            let mut document = before.as_ref().clone();
            if apply_preference_patch(&mut document, patch).is_err() {
                return Err(Settlement::Failed(ErrorCode::Validation {
                    domain,
                    reason: ValidationReason::Schema,
                }));
            }
            let (bytes, snapshot, projection, summary) =
                stage_preference_document(document, &before, cached, domain)?;
            (
                bytes,
                snapshot,
                DurableTimestamp::now(),
                CommitAction::PreferencesWrite,
                Some(projection),
                summary.map(AuditKind::Preferences),
            )
        }
        WriteIntent::Port(DomainEdit::Knowledge(_)) => {
            unreachable!("knowledge has no current file, so no store admits it")
        }
    };
    let unchanged = cached.durable && cached.digest == Some(files::digest(&bytes));
    // A semantic no-op can still publish canonical bytes or flush an outside
    // writer's file. Every such publication needs its saved-state info line.
    let audit = audit.or_else(|| (!unchanged).then_some(AuditKind::Plain));
    let advance_version = !unchanged && audit.is_some();
    Ok(Staged {
        bytes,
        snapshot,
        at,
        action,
        preferences,
        previous: cached.version,
        version: cached.version,
        unchanged,
        advance_version,
        audit,
        precondition: cached.digest,
        durable_before: cached.durable,
    })
}

fn current_preference_document(
    cached: &CachedDecision,
) -> Option<Arc<serde_json::Map<String, serde_json::Value>>> {
    cached
        .preferences
        .as_ref()
        .map(|projection| Arc::clone(&projection.document))
        .or_else(|| {
            files::preference_document(
                cached
                    .snapshot
                    .as_ref()
                    .and_then(LeaseSnapshot::as_preferences)
                    .unwrap_or(&[]),
            )
            .map(Arc::new)
        })
}

fn stage_preference_document(
    document: serde_json::Map<String, serde_json::Value>,
    before: &Arc<serde_json::Map<String, serde_json::Value>>,
    cached: &CachedDecision,
    domain: LocalStateDomain,
) -> Result<
    (
        Vec<u8>,
        LeaseSnapshot,
        files::PreferenceProjection,
        Option<PreferenceAuditSummary>,
    ),
    WriteSettlement,
> {
    let projection = files::project_preferences(document);
    let summary = preference_audit_summary(before, &projection.document);
    let (rows, unrepresentable) = files::preference_rows(&projection.document);
    if unrepresentable > 0 {
        return Err(Settlement::Failed(ErrorCode::Validation {
            domain,
            reason: ValidationReason::TooLong,
        }));
    }
    let bytes =
        files::preferences_bytes(&projection.document).map_err(|_| schema_failure(domain))?;
    if bytes.len() as u64 > files::MAX_PREFERENCES_BYTES && bytes.len() > cached.written_len {
        return Err(Settlement::Failed(ErrorCode::Validation {
            domain,
            reason: ValidationReason::TooLong,
        }));
    }
    Ok((bytes, LeaseSnapshot::preferences(rows), projection, summary))
}

fn apply_preference_patch(
    document: &mut serde_json::Map<String, serde_json::Value>,
    patch: &PreferencePatch,
) -> Result<(), ()> {
    for member in &patch.members {
        let Some((leaf, parents)) = member.path.split_last() else {
            return Err(());
        };
        let Some(object) = preference_parent_mut(document, parents, member.value.is_some()) else {
            continue;
        };
        match &member.value {
            Some(value) => {
                object.insert(leaf.clone(), value.clone());
            }
            None => {
                object.remove(leaf);
            }
        }
    }
    Ok(())
}

fn preference_parent_mut<'a>(
    mut object: &'a mut serde_json::Map<String, serde_json::Value>,
    parents: &[String],
    create: bool,
) -> Option<&'a mut serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value;

    for segment in parents {
        if create && !object.contains_key(segment) {
            object.insert(segment.clone(), Value::Object(serde_json::Map::new()));
        }
        let value = object.get_mut(segment)?;
        if !value.is_object() {
            if !create {
                return None;
            }
            *value = Value::Object(serde_json::Map::new());
        }
        object = value.as_object_mut()?;
    }
    Some(object)
}

fn schema_failure(domain: LocalStateDomain) -> WriteSettlement {
    Settlement::Failed(ErrorCode::Failed {
        domain,
        reason: FailureReason::Schema,
    })
}

/// The next job: the oldest queued readback, else the oldest queued write
/// of any store with nothing in flight. During shutdown only a cold store
/// with accepted writes retains its hydration demand.
fn take_job(state: &mut State) -> Option<Job> {
    if state.shutdown {
        for store in state.stores.values_mut() {
            let needed_by_write = matches!(store.file, FileState::Cold) && !store.queue.is_empty();
            if store.readback != Readback::Idle && !needed_by_write {
                store.readback = Readback::Idle;
                store.readbacks += 1;
            }
        }
    }
    if let Some(key) = state
        .stores
        .iter()
        .filter(|(_, store)| store.readback == Readback::Queued)
        .min_by_key(|(_, store)| store.readback_seq)
        .map(|(key, _)| key.clone())
    {
        let store = state.stores.get_mut(&key).expect("found above");
        store.readback = Readback::InFlight;
        return Some(Job::Readback(key));
    }
    let key = state
        .stores
        .iter()
        .filter(|(_, store)| store.in_flight.is_none() && !matches!(store.file, FileState::Cold))
        .filter_map(|(key, store)| store.queue.front().map(|queued| (key, queued.seq)))
        .min_by_key(|(_, seq)| *seq)
        .map(|(key, _)| key.clone())?;
    let store = state.stores.get_mut(&key).expect("found above");
    let queued = store.queue.pop_front().expect("found above");
    store.in_flight = Some(InFlight {
        terminal: queued.terminal(),
        publishing: false,
    });
    Some(Job::Write(key, queued))
}

fn lease_of(identity: &DomainIdentity, cached: &Cached) -> Result<DomainLease, ErrorCode> {
    DomainLease::try_new(
        identity.clone(),
        cached.version,
        cached.at,
        cached.snapshot.clone(),
    )
}

fn empty_snapshot(key: &StoreKey) -> LeaseSnapshot {
    match key {
        StoreKey::Loadouts => LeaseSnapshot::loadouts(Vec::new()),
        StoreKey::Preferences => LeaseSnapshot::preferences(Vec::new()),
        StoreKey::BankHint { .. } => {
            LeaseSnapshot::bank_hints(Vec::new()).expect("no rows meet the row rules")
        }
    }
}

/// What an empty store's rows take as the service writes them: `[]` for
/// loadouts, `{}` for preferences.
fn empty_len(key: &StoreKey) -> usize {
    match key {
        StoreKey::Loadouts | StoreKey::Preferences => 2,
        StoreKey::BankHint { .. } => 0,
    }
}

const fn domain_index(domain: LocalStateDomain) -> usize {
    match domain {
        LocalStateDomain::Preferences => 0,
        LocalStateDomain::BankHints => 1,
        LocalStateDomain::LoadoutsReferences => 2,
        LocalStateDomain::Knowledge => 3,
    }
}

#[cfg(test)]
#[path = "local_state_tests.rs"]
mod tests;
