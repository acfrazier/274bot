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
//!   but never rewritten, in every domain. Loadouts with duplicate names
//!   are refused whole because names cannot identify an individual row.
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
//!   way (failed, conflicted, cancelled). Import archives and a malformed
//!   legacy preferences file kept aside at import have their own notices.
//! - **Budget:** at most [`MAX_BANK_STORES`] bank-hint identities and the
//!   bounded script-settings identities are held; a new identity evicts the
//!   least recently used idle store of its domain, or is refused `Busy`.
//!   A lease is one bounded snapshot.
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
//! **Preferences (T5.3)** live in the state database
//! `<root>/state/state.sqlite3` (the private `sql` module), opened, read, written and
//! checkpointed only on the writer thread. Their versions are allocated
//! inside each write transaction from the database's own sequence (the
//! service clock follows them), so two processes never commit one version.
//! No SQL job waits for a lock: a held guard, `SQLITE_BUSY` or a full WAL
//! puts the job back with a delayed retry (1 ms doubling to 50 ms), and a
//! save that has not finished 5 s after its first attempt settles `Busy`
//! with its cause. Bank, loadout and Start jobs always run before the next
//! preference retry or checkpoint. The writer cannot preempt a job in
//! flight, so a Start's read never queues behind a first import of the
//! preferences that is running: it waits for that import to end first
//! ([`LocalStateService::readback_lease`]).
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
//! Script Start and loadout forms request bounded writer-thread readbacks.
//! Bank sessions queue a stamped readback and poll it on observe.
//! Preferences read their cached document. A stale cache queues a refresh
//! without waiting, and accepted patches settle separately.

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

mod bank;
mod files;
mod frontend;
mod maintenance;
mod operator_settings;
mod root_use;
mod runtime_settings;
mod sql;

use bank::BankWatermark;
#[cfg(any(test, feature = "test-support"))]
pub use bank::StoredBankHint;
pub(crate) use bank::{BankHintBinding, BankHydration};
pub(crate) use files::hint_has_pre_0_2_8_shape;
use files::{ContentDigest, LockError, ReadDocument, ReadError, RenameRefusal, Rewrite, StoreKey};
pub use files::{
    StateRoot, HINT_SCHEMA_VERSION, MAX_HINT_BYTES, MAX_LOADOUTS_BYTES, MAX_PREFERENCES_BYTES,
};
#[cfg(any(test, feature = "test-support"))]
pub use frontend::{transient_wait, wait_outcome, WaitOutcome};
pub use frontend::{
    PreferenceError, PreferencePatch, PreferenceSnapshot, Preferences, QueuedUpdate,
    ServiceLoadouts, Update,
};
pub use maintenance::{
    maintenance_command, maintenance_help, parse_maintenance, run_maintenance,
    run_preferences_maintenance, MaintenanceAction, MaintenanceError,
};
use operator_settings::{OperatorJob, OperatorState};
pub use operator_settings::{
    OperatorScriptSettings, OperatorScriptSettingsEdit, ScriptSettingsError,
};
pub use root_use::{RootUse, StateRootLockError};
pub use sql::Frontend;

/// How long one preference save may keep being retried (a contended
/// guard, `SQLITE_BUSY`, a full WAL) from its first attempt before it
/// settles `Busy` with its cause. It replaces the old file lock's wait.
const PREFERENCE_DEADLINE: Duration = Duration::from_secs(5);
/// The first and the largest delay between two attempts of a requeued
/// preference job: the old file lock's retry intervals, without sleeping.
const RETRY_FIRST: Duration = Duration::from_millis(1);
const RETRY_MAX: Duration = Duration::from_millis(50);
/// The most file-store and bank jobs the writer runs in a row while a
/// preference job is due. Without it a preference save waits behind every
/// queued file job: the 256 first imports of the resident cap take 6.98 s in
/// one process on the reference machine (`first_import_of_all_accounts_ms` of
/// `bank_budget_report`), over [`PREFERENCE_DEADLINE`]. The bound is the depth
/// of a preference queue ([`OPERATOR_QUEUE_CAP`], the design load of the
/// preference queue), so a save waits behind at most 8 file jobs: 8 x the
/// largest bank job measured there (107.8 ms, `bank-import-residence`), 0.9 s.
const FILE_JOB_BURST: u32 = OPERATOR_QUEUE_CAP as u32;

/// How many edits of one operator store (loadouts, preferences) may wait
/// behind the one in flight.
const OPERATOR_QUEUE_CAP: usize = 8;
/// Live preference requests and unprojected results, separate from neutral history.
const PREFERENCE_WATCH_CAP: usize = OPERATOR_QUEUE_CAP + 1;
/// How many bank-hint identities the service holds at once. A new one
/// evicts the least recently used idle store (nothing queued, in flight
/// or being read); with none idle it is refused `Busy`.
pub const MAX_BANK_STORES: usize = 256;
/// How many script runtime settings identities the service holds at once.
pub const MAX_SCRIPT_RUNTIME_SETTINGS_STORES: usize = 256;
/// A test's witness of how many provisional services
/// [`StateComposition::for_play`] opened: an injected process service opens
/// none.
#[cfg(test)]
pub(crate) static PROVISIONAL_OPENED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

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
/// A test's decision, at the start of each preference save, to settle it as
/// a save whose outcome is unknown
/// ([`LocalStateService::set_write_unknown_for_test`]).
#[cfg(any(test, feature = "test-support"))]
pub type WriteUnknown = Arc<dyn Fn() -> bool + Send + Sync>;
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

/// A test's witness of each line the service logs for file recovery or
/// refusal: the operator's notice, audit line, or why recovery could not be made.
#[cfg(test)]
pub(crate) type NoticeHook = Arc<dyn Fn(Level, &str) + Send + Sync>;

fn emit_local_notice(
    level: Level,
    line: std::fmt::Arguments<'_>,
    #[cfg(test)] hook: Option<&NoticeHook>,
) {
    host_log!(Category::Lifecycle, level, "{line}");
    #[cfg(test)]
    if let Some(hook) = hook {
        hook(level, &line.to_string());
    }
}
/// A test's witness of each complete writer job: its class and the whole
/// time the writer spent on it, from selection to the settled outcome
/// (cache, typed view, audit and terminal included).
#[cfg(test)]
pub(crate) type JobHook = Arc<dyn Fn(&'static str, Duration) + Send + Sync>;

/// A test's witness of each bank save, called inside the save's open
/// transaction with the rows it commits.
#[cfg(test)]
pub(crate) type BankWriteHook = Arc<dyn Fn(&[(i32, i32)]) + Send + Sync>;

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

    /// The composition one `Play` builds for the profile `namespace` under
    /// `scope`, at `state_root`. With a process service `injected` (a
    /// frontend's or a headless launch's one writer and one connection) the
    /// composition runs over it and nothing else is opened. Only a caller
    /// with none (a test fixture, a direct run of the library) gets a
    /// provisional service of its own, which opens its own connection when
    /// its first bank hint job runs.
    pub(crate) fn for_play(
        state_root: &Path,
        namespace: &str,
        scope: ContentScope,
        injected: Option<Arc<LocalStateService>>,
    ) -> Arc<Self> {
        let service = injected.unwrap_or_else(|| {
            #[cfg(test)]
            PROVISIONAL_OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            LocalStateService::open(
                StateRoot::new(state_root.to_path_buf()),
                &[
                    LocalStateDomain::LoadoutsReferences,
                    LocalStateDomain::BankHints,
                    LocalStateDomain::ScriptRuntimeSettings,
                ],
                Arc::new(RequestIdMinter::new()),
            )
        });
        Arc::new(Self::new(service, namespace, scope))
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

    /// Ready the account- and content-scoped store before a synchronous JS
    /// Start. An absent store is ready; refusals and timeouts stop the Start.
    #[cfg(feature = "js")]
    pub(crate) fn prepare_script_runtime_settings(&self, username: &str) -> Result<(), String> {
        let identity = self
            .identity(LocalStateDomain::ScriptRuntimeSettings, username)
            .map_err(|error| format!("script runtime settings: {error}"))?;
        let path = self.service.root().script_runtime_settings_file(
            identity.account().profile(),
            identity.account().account(),
            identity.scope().revision().as_i32(),
            identity.scope().content(),
        );
        self.service
            .readback_lease(&identity, START_LEASE_TIMEOUT)
            .map(|_| ())
            .map_err(|code| format!("script runtime settings: {code} ({})", path.display()))
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
/// ([`LocalStateService::queue_readback`]): the completed-readback count
/// that answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadbackTicket {
    resident: u64,
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
    open: [bool; LocalStateDomain::ALL.len()],
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
    Settled {
        settlement: WriteSettlement,
        detail: Option<Arc<str>>,
        /// The save's COMMIT may have reached the disk and the database
        /// could not settle it (plan §4.2).
        outcome_unknown: bool,
    },
    Missing,
}

struct PreferenceWatch {
    settlement: Option<WriteSettlement>,
    queued_bytes: usize,
    /// The bounded cause a failed request carries beyond its neutral code
    /// (a deadline reached while waiting for the WAL, a refused database).
    detail: Option<Arc<str>>,
    /// The failed save's COMMIT ended in a disk error that could not be
    /// settled: whether it reached the disk is unknown (plan §4.2).
    outcome_unknown: bool,
}

impl PreferenceWatch {
    /// What this watch settled as, once it has.
    fn settled(&self) -> Option<PreferenceWatchPoll> {
        self.settlement
            .clone()
            .map(|settlement| PreferenceWatchPoll::Settled {
                settlement,
                detail: self.detail.clone(),
                outcome_unknown: self.outcome_unknown,
            })
    }
}

/// The idle TRUNCATE checkpoint the writer owes the preferences WAL.
#[derive(Default)]
struct Truncate {
    /// A commit left `F >= W / 2`, or a save waits for room in the WAL.
    due: bool,
    /// When a busy attempt may run again.
    retry_at: Option<Instant>,
    backoff: Duration,
    /// Completed resets and busy attempts, for tests and measurements.
    resets: u64,
    busy: u64,
}

/// What the writer thread's database is opened with.
#[derive(Default)]
struct DbSeed {
    frontend: Frontend,
    root_use: Option<RootUse>,
}

#[derive(Default)]
struct State {
    stores: HashMap<StoreKey, Store>,
    /// Keep live bank chronology even when the cached store is evicted.
    bank_watermarks: HashMap<StoreKey, BankWatermark>,
    /// Every accepted request whose settlement is not yet forgotten, and
    /// the store it belongs to.
    requests: HashMap<RequestId, StoreKey>,
    /// At most nine live requests or terminals, retained until frontend projection.
    preference_watches: HashMap<RequestId, PreferenceWatch>,
    /// Accepted preference intent bytes until terminal settlement.
    preference_queue_bytes: usize,
    operator_settings: OperatorState,
    /// The default document/view is prepared before the writer starts.
    preference_default: files::PreferenceProjection,
    /// Why the preference job at the front is waiting, while it is.
    preference_wait: Option<sql::WaitCause>,
    /// Only the single preferences readback can ask for a SQL retry.
    /// Account hint stores never carry this process-owned state.
    preference_readback_retry: Retry,
    /// The one retry state of the bank hints' readbacks: every hint shares
    /// the process's one connection, so one delay serves them all. A hint
    /// store carries no retry state of its own.
    bank_readback_retry: Retry,
    /// Only scopes waiting on SQL need an independent hydration retry.
    runtime_readback_retries: HashMap<StoreKey, Retry>,
    /// How many file-store and bank jobs ran in a row while a preference job
    /// was due: at [`FILE_JOB_BURST`] the next job is the preference one.
    file_job_streak: u32,
    /// The idle WAL reset the writer owes.
    truncate: Truncate,
    /// Taken by the writer thread when it starts.
    db_seed: Option<DbSeed>,
    /// A test's seams inside the database (a stall in a real SQL call, a
    /// COMMIT outcome), copied to the writer's database before each job.
    #[cfg(test)]
    sql_seams: sql::Seams,
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
    /// A test's disk whose COMMIT ends in a disk error that cannot be
    /// settled: asked after the fault of each preference save.
    #[cfg(any(test, feature = "test-support"))]
    write_unknown: Option<WriteUnknown>,
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
    #[cfg(test)]
    read_refusal: Option<NoticeHook>,
    #[cfg(test)]
    job: Option<JobHook>,
    #[cfg(test)]
    bank_write: Option<BankWriteHook>,
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
    /// Identity of this residence in the cache, not a readback counter.
    resident: u64,
    file: FileState,
    /// A readback is queued or in flight.
    readback: Readback,
    /// When the queued readback was asked for (FIFO order).
    readback_seq: u64,
    /// How many readbacks have completed: a bounded wait follows it.
    readbacks: u64,
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
    /// A bank read needs a SQL retry and another read was requested during it.
    QueuedAgain,
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

#[derive(Clone)]
struct PendingRuntimeSetting {
    namespace: String,
    key: String,
    raw: Arc<str>,
}
/// The edit the writer thread holds for one store.
#[derive(Clone)]
struct InFlight {
    terminal: Terminal,
    setting: Option<PendingRuntimeSetting>,
    /// The writer passed its last check and is renaming the bytes into
    /// place, or its SQL transaction has begun: past the shutdown bound it
    /// is left to its own outcome (commit, rollback or uncertain
    /// resolution) and settles when that is known.
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
            Self::Port(DomainEdit::ScriptRuntimeSettings(_)) => {
                CommitAction::ScriptRuntimeSettingsWrite
            }
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
    /// The observation order for a slot bank save, zero for other writes.
    bank_observation: u64,
    /// What the edit may add to the file, for the budget.
    growth: Growth,
    /// The retry state of a preference save the database asked to wait:
    /// when its first attempt ran (its deadline counts from there), when it
    /// may run again, and the next delay.
    retry: Retry,
}

#[derive(Default, Clone, Copy)]
struct Retry {
    first: Option<Instant>,
    at: Option<Instant>,
    backoff: Duration,
}

impl Retry {
    /// Schedule the next attempt after `now`: 1 ms, doubling, at most 50 ms.
    fn delay(&mut self, now: Instant) {
        self.backoff = if self.backoff.is_zero() {
            RETRY_FIRST
        } else {
            (self.backoff * 2).min(RETRY_MAX)
        };
        self.at = Some(now + self.backoff);
    }

    fn due(&self, now: Instant) -> bool {
        self.at.is_none_or(|at| at <= now)
    }

    fn expired(&self, now: Instant) -> bool {
        self.first
            .is_some_and(|first| now.saturating_duration_since(first) >= PREFERENCE_DEADLINE)
    }
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
    OperatorSettings(OperatorJob),
    /// The idle TRUNCATE of the preferences WAL.
    Checkpoint,
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
        mut state: State,
    ) -> Arc<Self> {
        let mut open = [false; LocalStateDomain::ALL.len()];
        for domain in domains {
            open[domain_index(*domain)] = true;
        }
        if state.db_seed.is_none() {
            state.db_seed = Some(DbSeed::default());
        }
        let preference_path: Arc<str> = Arc::from(root.database_file().display().to_string());
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
                LocalStateDomain::ScriptRuntimeSettings,
                LocalStateDomain::OperatorScriptSettings,
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
        Self::open_process_with(root, Frontend::Headless, None)
    }

    /// [`Self::open_process`] with `frontend`'s maintenance commands.
    /// Product composition roots must pass their startup permit's
    /// `root_use` so the service shares the locked descriptor. `None` is
    /// for unbound library compositions, which take a shared lease and
    /// cannot coexist with an exclusive startup lock on another descriptor.
    pub fn open_process_with(
        root: StateRoot,
        frontend: Frontend,
        root_use: Option<RootUse>,
    ) -> Arc<Self> {
        let state = State {
            db_seed: Some(DbSeed { frontend, root_use }),
            ..State::default()
        };
        Self::open_with_state(
            root,
            &[
                LocalStateDomain::Preferences,
                LocalStateDomain::LoadoutsReferences,
                LocalStateDomain::BankHints,
                LocalStateDomain::ScriptRuntimeSettings,
                LocalStateDomain::OperatorScriptSettings,
            ],
            Arc::new(RequestIdMinter::new()),
            state,
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
        self.preferences_projection(identity, freshness, |projection| {
            projection.walk_globals.clone()
        })
    }

    pub(super) fn preferences_pvp_globals(
        self: &Arc<Self>,
        identity: &DomainIdentity,
        freshness: Duration,
    ) -> Result<script::native::PvpGlobals, PreferenceError> {
        self.preferences_projection(identity, freshness, |projection| {
            projection.pvp_globals.clone()
        })
    }

    /// One worker-built projection of the cached preferences, read without
    /// waiting for storage: a cold or refused store, or a projection the
    /// worker could not build, is a typed error.
    fn preferences_projection<T>(
        self: &Arc<Self>,
        identity: &DomainIdentity,
        freshness: Duration,
        pick: impl FnOnce(&files::PreferenceProjection) -> Result<T, Arc<str>>,
    ) -> Result<T, PreferenceError> {
        if !self.is_open(identity.domain()) {
            return Err(self.shared.preference_not_ready.clone());
        }
        let key = StoreKey::Preferences;
        let mut state = self.shared.state.lock();
        let Some(store) = state.stores.get(&key) else {
            return Err(self.shared.preference_not_ready.clone());
        };
        let (projected, stale) = match &store.file {
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
                (pick(projection), cached.refreshed_at.elapsed() >= freshness)
            }
        };
        if stale {
            self.shared.request_readback(&mut state, &key);
        }
        projected.map_err(|cause| {
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
            .and_then(PreferenceWatch::settled)
        {
            Some(settled) => {
                state.preference_watches.remove(&request);
                settled
            }
            None if state.preference_watches.contains_key(&request) => PreferenceWatchPoll::Pending,
            None => PreferenceWatchPoll::Missing,
        }
    }

    /// Why the preference save at the front of the queue is waiting, while
    /// it is: what a pending setting shows (plan §4.4).
    pub(super) fn preference_wait(&self) -> Option<&'static str> {
        self.shared
            .state
            .lock()
            .preference_wait
            .map(sql::WaitCause::text)
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
                .and_then(PreferenceWatch::settled)
            {
                Some(settled) => {
                    state.preference_watches.remove(&request);
                    return settled;
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

    /// Accept one preference patch for the writer. A refusal names its
    /// code, and its cause when the store is refused (the path, the cause
    /// and the remedy the refusal recorded), so the setting that tried to
    /// save shows why, not only the code.
    pub(super) fn enqueue_preference_patch(
        &self,
        identity: &DomainIdentity,
        patch: PreferencePatch,
    ) -> Result<RequestId, (ErrorCode, Option<Arc<str>>)> {
        let domain = LocalStateDomain::Preferences;
        let key = StoreKey::Preferences;
        if !self.is_open(domain) {
            return Err((ErrorCode::NotReady { domain }, None));
        }
        let mut state = self.shared.state.lock();
        let operator_writes = state.operator_settings.inflight_and_queued();
        if state.shutdown
            || state.preference_watches.len() >= PREFERENCE_WATCH_CAP
            || (operator_writes != 0
                && state.preference_watches.len() + operator_writes > OPERATOR_QUEUE_CAP)
        {
            return Err((ErrorCode::Busy { domain }, None));
        }
        if state
            .preference_queue_bytes
            .saturating_add(patch.encoded_bytes)
            > files::MAX_PREFERENCES_BYTES as usize
        {
            return Err((
                ErrorCode::Validation {
                    domain,
                    reason: ValidationReason::TooLong,
                },
                None,
            ));
        }
        let store = state
            .stores
            .get(&key)
            .ok_or((ErrorCode::NotReady { domain }, None))?;
        if store.queue.len() >= OPERATOR_QUEUE_CAP {
            return Err((ErrorCode::Busy { domain }, None));
        }
        if let FileState::Rejected(rejection) = &store.file {
            return Err((
                ErrorCode::Failed {
                    domain,
                    reason: rejection.reason,
                },
                Some(Arc::clone(&rejection.detail)),
            ));
        }
        if !self.shared.start_writer(&mut state) {
            return Err((ErrorCode::Busy { domain }, None));
        }
        let request = self
            .shared
            .minter
            .mint()
            .map_err(|_| (ErrorCode::Busy { domain }, None))?;
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
            bank_observation: 0,
            growth: Growth::default(),
            retry: Retry::default(),
        });
        state.preference_queue_bytes += queued_bytes;
        state.preference_watches.insert(
            request,
            PreferenceWatch {
                settlement: None,
                queued_bytes,
                detail: None,
                outcome_unknown: false,
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
    /// This is for synchronous callers such as script Start and slot exit.
    /// It never runs on a frame or session observe.
    ///
    /// A Start's read never queues behind the first import of the
    /// preferences: the writer cannot preempt a job in flight, so a read
    /// queued behind the running import would wait for all of it. While
    /// that import runs on the writer the call waits outside the writer's
    /// queue, and `timeout` counts that wait too (operator decision on the
    /// S1 occupancy gate, 2026-11-15). An import that is only queued, or
    /// that waits to retry because another connection holds the SQL write
    /// lock, holds nothing back: file reads are taken before preference
    /// jobs, so a Start keeps running (plan §4.4, §5.4). Automatic deferred
    /// startup waits for the preferences to be adopted
    /// (`OperatorSession::prepare_startup`). The preferences' own readbacks
    /// are the import and never wait for it.
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
        let deadline = Instant::now() + timeout;
        let ticket = self.queue_readback(identity, Some(deadline))?;
        self.finish_readback(identity, ticket, Some(deadline))
            .unwrap_or(Err(ErrorCode::NotReady {
                domain: identity.domain(),
            }))
    }

    /// Queue a re-read of `identity`'s store and return what
    /// [`Self::finish_readback`] waits for: a read that began after this
    /// call. `NotReady`, with nothing queued, while the preferences' first
    /// import is running on the writer: the read would queue behind a job
    /// the writer cannot preempt ([`Self::readback_lease`]). Only the
    /// fixture-only synchronous preference writer queues a re-read without
    /// waiting.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn start_readback(
        &self,
        identity: &DomainIdentity,
    ) -> Result<ReadbackTicket, ErrorCode> {
        self.queue_readback(identity, None)
    }

    /// Queue a re-read of `identity`'s store. Without `wait_until` it
    /// returns `NotReady` at once while a first import is in flight; with
    /// it, it waits outside the writer's queue for that import to end
    /// (`NotReady` at the deadline, or when the service stops first). The
    /// wait and the queueing are one critical section: the import cannot
    /// begin between them.
    fn queue_readback(
        &self,
        identity: &DomainIdentity,
        wait_until: Option<Instant>,
    ) -> Result<ReadbackTicket, ErrorCode> {
        let key = self.store_key(identity)?;
        let domain = identity.domain();
        let mut state = self.shared.state.lock();
        while self.shared.behind_first_import(&key, &state) {
            let Some(deadline) = wait_until else {
                return Err(ErrorCode::NotReady { domain });
            };
            if state.shutdown
                || self
                    .shared
                    .settled
                    .wait_until(&mut state, deadline)
                    .timed_out()
            {
                return Err(ErrorCode::NotReady { domain });
            }
        }
        self.enqueue_readback(&mut state, &key)
    }

    fn enqueue_readback(
        &self,
        state: &mut State,
        key: &StoreKey,
    ) -> Result<ReadbackTicket, ErrorCode> {
        let resident = state.admit(key)?.resident;
        let target = self
            .shared
            .request_readback(state, key)
            .ok_or(ErrorCode::NotReady {
                domain: key.domain(),
            })?;
        Ok(ReadbackTicket { resident, target })
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
        let key = match self.store_key(identity) {
            Ok(key) => key,
            Err(code) => return Some(Err(code)),
        };
        self.finish_readback_key(identity, &key, ticket, deadline)
    }

    fn finish_readback_key(
        &self,
        identity: &DomainIdentity,
        key: &StoreKey,
        ticket: ReadbackTicket,
        deadline: Option<Instant>,
    ) -> Option<Result<(DomainLease, bool), ErrorCode>> {
        let domain = key.domain();
        let mut state = self.shared.state.lock();
        loop {
            let Some(store) = state
                .stores
                .get(key)
                .filter(|store| store.resident == ticket.resident)
            else {
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

    /// Test-support seam for front-end tests: `unknown` runs on the writer
    /// thread at the start of each preference save, after the fault; when it
    /// returns true that save settles as one whose COMMIT ended in a disk
    /// error that the database could not settle (`Failed(Io)`, its outcome
    /// unknown, plan §4.2), exactly as the writer settles it, and nothing is
    /// written.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_write_unknown_for_test(&self, unknown: WriteUnknown) {
        self.shared.state.lock().write_unknown = Some(unknown);
    }

    /// Install the database seams (a stall inside a real SQL statement, a
    /// COMMIT outcome, named import points), from the writer's next job.
    #[cfg(test)]
    pub(crate) fn set_sql_seams_for_test(&self, seams: sql::Seams) {
        self.shared.state.lock().sql_seams = seams;
    }

    /// Completed and busy idle WAL resets, and whether one is owed.
    #[cfg(test)]
    pub(crate) fn truncate_counts_for_test(&self) -> (u64, u64, bool) {
        let state = self.shared.state.lock();
        (
            state.truncate.resets,
            state.truncate.busy,
            state.truncate.due,
        )
    }

    /// Open the process stores with database seams installed before the
    /// first hydration (and an optional read stall before it).
    #[cfg(test)]
    pub(crate) fn open_process_with_seams_for_test(
        root: StateRoot,
        seams: sql::Seams,
        read_stall: Option<ReadStall>,
    ) -> Arc<Self> {
        let state = State {
            sql_seams: seams,
            read_stall,
            ..State::default()
        };
        Self::open_with_state(
            root,
            &[
                LocalStateDomain::Preferences,
                LocalStateDomain::LoadoutsReferences,
                LocalStateDomain::BankHints,
                LocalStateDomain::ScriptRuntimeSettings,
                LocalStateDomain::OperatorScriptSettings,
            ],
            Arc::new(RequestIdMinter::new()),
            state,
        )
    }

    /// Open the process stores with `hook` already watching the writer, so
    /// the first hydration (open, schema, epoch, import) is a measured job.
    #[cfg(test)]
    pub(crate) fn open_process_timed_for_test(root: StateRoot, hook: JobHook) -> Arc<Self> {
        let state = State {
            hooks: Hooks {
                job: Some(hook),
                ..Hooks::default()
            },
            ..State::default()
        };
        Self::open_with_state(
            root,
            &[
                LocalStateDomain::Preferences,
                LocalStateDomain::LoadoutsReferences,
                LocalStateDomain::BankHints,
                LocalStateDomain::ScriptRuntimeSettings,
                LocalStateDomain::OperatorScriptSettings,
            ],
            Arc::new(RequestIdMinter::new()),
            state,
        )
    }

    /// Open the process stores with `hook` already receiving every notice,
    /// the first hydration's included.
    #[cfg(test)]
    pub(crate) fn open_process_noticed_for_test(root: StateRoot, hook: NoticeHook) -> Arc<Self> {
        let state = State {
            hooks: Hooks {
                notice: Some(hook),
                ..Hooks::default()
            },
            ..State::default()
        };
        Self::open_with_state(
            root,
            &[
                LocalStateDomain::Preferences,
                LocalStateDomain::LoadoutsReferences,
                LocalStateDomain::BankHints,
                LocalStateDomain::ScriptRuntimeSettings,
                LocalStateDomain::OperatorScriptSettings,
            ],
            Arc::new(RequestIdMinter::new()),
            state,
        )
    }

    /// Open the process stores with `stall` run inside a real SQL statement
    /// on the writer (the first hydration holds there; T5.3 §9.5).
    #[cfg(test)]
    pub(crate) fn open_process_with_sql_stall_for_test(
        root: StateRoot,
        stall: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<Self> {
        Self::open_process_with_seams_for_test(
            root,
            sql::Seams {
                stall_in_sql: Some(stall),
                ..sql::Seams::default()
            },
            None,
        )
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

    /// Witness every complete writer job with its class and total time.
    #[cfg(test)]
    pub(crate) fn set_job_hook_for_test(&self, hook: JobHook) {
        self.shared.state.lock().hooks.job = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_read_stall_for_test(&self, stall: ReadStall) {
        self.shared.state.lock().read_stall = Some(stall);
    }

    /// Witness each bank save inside its transaction (a test may hold it
    /// there), with the rows it commits.
    #[cfg(test)]
    pub(crate) fn set_bank_write_hook_for_test(&self, hook: BankWriteHook) {
        self.shared.state.lock().hooks.bank_write = Some(hook);
    }

    /// How many preference saves are queued and whether one is in flight.
    #[cfg(test)]
    pub(crate) fn preference_occupancy_for_test(&self) -> (usize, bool) {
        let state = self.shared.state.lock();
        state
            .stores
            .get(&StoreKey::Preferences)
            .map_or((0, false), |store| {
                (store.queue.len(), store.in_flight.is_some())
            })
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
    fn runtime_setting_raw(
        &self,
        identity: &DomainIdentity,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>, ErrorCode> {
        let store_key = self.store_key(identity)?;
        let domain = identity.domain();
        let mut state = self.shared.state.lock();
        let store = state.admit(&store_key)?;
        let queued = store
            .queue
            .iter()
            .rev()
            .find_map(|queued| match &queued.intent {
                WriteIntent::Port(DomainEdit::ScriptRuntimeSettings(edit))
                    if edit.namespace() == namespace && edit.key() == key =>
                {
                    Some(edit.raw())
                }
                _ => None,
            });
        let pending = queued.or_else(|| {
            let setting = store.in_flight.as_ref()?.setting.as_ref()?;
            (setting.namespace == namespace && setting.key == key).then_some(setting.raw.as_ref())
        });
        if let Some(raw) = pending {
            return Ok(Some(raw.to_owned()));
        }
        match &store.file {
            FileState::Read(cached) => {
                let rows =
                    cached
                        .snapshot
                        .as_script_runtime_settings()
                        .ok_or(ErrorCode::Validation {
                            domain,
                            reason: ValidationReason::Identity,
                        })?;
                return Ok(files::runtime_setting_index(rows, namespace, key)
                    .ok()
                    .map(|index| rows[index].raw().to_owned()));
            }
            FileState::Rejected(rejection) => {
                return Err(ErrorCode::Failed {
                    domain,
                    reason: rejection.reason,
                })
            }
            FileState::Cold => {}
        }
        // Cold: queue hydration rather than reading a file in the caller.
        self.shared.request_readback(&mut state, &store_key);
        Err(ErrorCode::NotReady { domain })
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        let key = match self.store_key(edit.identity()) {
            Ok(key) => key,
            Err(code) => return SubmitReceipt::Refused(code),
        };
        self.submit_edit(edit, &key, 0)
    }

    fn observe(&self, request: RequestId) -> Option<WriteSettlement> {
        match self.request_state(request) {
            RequestState::Settled(settlement) => Some(settlement),
            RequestState::Pending | RequestState::Forgotten => None,
        }
    }
}

impl LocalStateService {
    fn submit_edit(
        &self,
        edit: DomainEdit,
        key: &StoreKey,
        bank_observation: u64,
    ) -> SubmitReceipt {
        let domain = key.domain();
        // A loadout row is measured before the lock is taken.
        let growth = match growth_of(&edit) {
            Ok(growth) => growth,
            Err(code) => return refused_visibly(key, code),
        };
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return SubmitReceipt::Refused(ErrorCode::Busy { domain });
        }
        if bank_observation != 0 {
            if let Err(code) = state.check_bank_observation(key, bank_observation) {
                return SubmitReceipt::Refused(code);
            }
        }
        let store = match state.admit(key) {
            Ok(store) => store,
            Err(code) => return SubmitReceipt::Refused(code),
        };
        let cold_runtime_settings = matches!(key, StoreKey::ScriptRuntimeSettings { .. })
            && edit.expected().is_none()
            && matches!(&store.file, FileState::Cold);
        let cold_bank = bank_observation != 0 && matches!(&store.file, FileState::Cold);
        let admission = if cold_bank {
            Gate::Accept(StoreVersion::INITIAL, growth)
        } else {
            gate(key, store, &edit, growth)
        };
        let (accepted_at, growth) = match admission {
            Gate::Accept(version, growth) => (version, growth),
            Gate::Hydrate(code) => {
                // Cold, or a refused file: (re)read it, so a later
                // submission finds it ready or repaired. Never written
                // meanwhile.
                self.shared.request_readback(&mut state, key);
                return SubmitReceipt::Refused(code);
            }
            Gate::Refuse(code) => {
                drop(state);
                return refused_visibly(key, code);
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
            .get_mut(key)
            .expect("the store was admitted under this lock");
        let superseded = match (key, &edit) {
            (StoreKey::BankHint { .. }, _) => {
                store.queue.pop_front().map(|queued| queued.terminal())
            }
            (StoreKey::ScriptRuntimeSettings { .. }, DomainEdit::ScriptRuntimeSettings(edit)) => {
                let replaced = store.queue.iter().position(|queued| {
                    matches!(
                        &queued.intent,
                        WriteIntent::Port(DomainEdit::ScriptRuntimeSettings(pending))
                            if pending.namespace() == edit.namespace()
                                && pending.key() == edit.key()
                    )
                });
                replaced
                    .and_then(|index| store.queue.remove(index))
                    .map(|queued| queued.terminal())
            }
            _ => None,
        };
        let external = store.external;
        store.queue.push_back(Queued {
            request,
            intent: WriteIntent::Port(edit),
            seq,
            accepted_at,
            external,
            bank_observation,
            growth,
            retry: Retry {
                first: cold_bank.then(Instant::now),
                ..Retry::default()
            },
        });
        state.requests.insert(request, key.clone());
        if bank_observation != 0 {
            state.record_bank_attempt(key, request);
        }
        if cold_runtime_settings
            || (cold_bank
                && state
                    .stores
                    .get(key)
                    .is_some_and(|store| store.readback == Readback::Idle))
        {
            // The writer cannot take a cold store's queue until a readback settles.
            let _ = self.shared.request_readback(&mut state, key);
        }
        let cancelled = Settlement::Cancelled(Cancellation::Superseded);
        let superseded = superseded.filter(|terminal| {
            state.settle(key, terminal.request, cancelled.clone(), Some(request))
        });
        drop(state);
        self.shared.wake.notify_one();
        if let Some(terminal) = superseded {
            self.shared.settled.notify_all();
            self.shared.terminal(&terminal, &cancelled, Some(request));
        }
        SubmitReceipt::Accepted { request }
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
        // Past the bound: every accepted write the writer has not begun
        // (to rename into place, or to run its SQL transaction) gets its
        // terminal now, each with its own line; the writer begins nothing
        // more. One already begun is left to its own outcome and settles
        // when that is known (or is lost unacknowledged with the process).
        let (cancelled, publishing) = self.shared.abandon();
        self.shared.settled.notify_all();
        host_log!(
            Category::Lifecycle,
            Level::Warn,
            "local state: shutdown bound of {timeout:?} passed; {cancelled} accepted write(s) \
             settled cancelled, {publishing} already begun (a rename or an SQL transaction) \
             settle when they finish"
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
    let replaces_pending = match edit {
        DomainEdit::ScriptRuntimeSettings(edit) => store.queue.iter().any(|queued| {
            matches!(
                &queued.intent,
                WriteIntent::Port(DomainEdit::ScriptRuntimeSettings(pending))
                    if pending.namespace() == edit.namespace() && pending.key() == edit.key()
            )
        }),
        _ => false,
    };
    if key.operator_wide() && store.queue.len() >= OPERATOR_QUEUE_CAP {
        return Gate::Refuse(ErrorCode::Busy { domain });
    }
    if matches!(key, StoreKey::ScriptRuntimeSettings { .. })
        && store.queue.len() >= OPERATOR_QUEUE_CAP
        && !replaces_pending
    {
        return Gate::Refuse(ErrorCode::Busy { domain });
    }
    let cached = match &store.file {
        FileState::Cold
            if matches!(key, StoreKey::ScriptRuntimeSettings { .. })
                && edit.expected().is_none() =>
        {
            return Gate::Accept(StoreVersion::INITIAL, growth);
        }
        FileState::Cold => return Gate::Hydrate(ErrorCode::NotReady { domain }),
        FileState::Rejected(rejection) => {
            return Gate::Hydrate(ErrorCode::Failed {
                domain,
                reason: rejection.reason,
            });
        }
        FileState::Read(cached) => cached,
    };
    if let Rewrite::Refused { .. } = cached.rewrite {
        return Gate::Hydrate(ErrorCode::Failed {
            domain,
            reason: FailureReason::Schema,
        });
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

fn bounded_store_limit(key: &StoreKey) -> Option<usize> {
    match key {
        StoreKey::BankHint { .. } => Some(MAX_BANK_STORES),
        StoreKey::ScriptRuntimeSettings { .. } => Some(MAX_SCRIPT_RUNTIME_SETTINGS_STORES),
        StoreKey::Loadouts | StoreKey::Preferences => None,
    }
}

fn same_bounded_store_kind(left: &StoreKey, right: &StoreKey) -> bool {
    matches!(
        (left, right),
        (StoreKey::BankHint { .. }, StoreKey::BankHint { .. })
            | (
                StoreKey::ScriptRuntimeSettings { .. },
                StoreKey::ScriptRuntimeSettings { .. }
            )
    )
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
    /// bounded identity evicts the least recently used idle store of its
    /// domain, or is refused `Busy`.
    fn admit(&mut self, key: &StoreKey) -> Result<&mut Store, ErrorCode> {
        let used = self.next_seq();
        if !self.stores.contains_key(key) {
            if let Some(limit) = bounded_store_limit(key) {
                let held = self
                    .stores
                    .keys()
                    .filter(|candidate| same_bounded_store_kind(candidate, key))
                    .count();
                if held >= limit {
                    self.evict_idle_bounded_store(key, limit)?;
                }
            }
            self.stores.insert(
                key.clone(),
                Store {
                    resident: used,
                    ..Store::default()
                },
            );
        }
        let store = self.stores.get_mut(key).expect("admitted above");
        store.used = used;
        Ok(store)
    }

    fn evict_idle_bounded_store(
        &mut self,
        wanted: &StoreKey,
        limit: usize,
    ) -> Result<(), ErrorCode> {
        let victim = self
            .stores
            .iter()
            .filter(|(key, store)| same_bounded_store_kind(key, wanted) && store.idle())
            .min_by_key(|(_, store)| store.used)
            .map(|(key, _)| key.clone());
        let Some(victim) = victim else {
            host_log!(
                Category::Lifecycle,
                Level::Warn,
                "local state: {} refused: {limit} identities held and none idle",
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
        let (profile, account) = match &victim {
            StoreKey::BankHint {
                profile, account, ..
            }
            | StoreKey::ScriptRuntimeSettings {
                profile, account, ..
            } => (profile, account),
            StoreKey::Loadouts | StoreKey::Preferences => {
                unreachable!("only bounded account stores can be evicted")
            }
        };
        host_log!(
            Category::Lifecycle,
            Level::Info,
            "local state: {} holds {limit} identities; forgot idle {profile}/{account}",
            wanted.domain()
        );
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
            .sum::<usize>()
            + self.operator_settings.inflight_and_queued()
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
                            preferences: None,
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
                            snapshot: LeaseSnapshot::loadouts(Vec::new()),
                            preferences: None,
                            refreshed_at: Instant::now(),
                            digest: None,
                            durable: false,
                            written_len: 2,
                            rewrite: Rewrite::Lossless,
                        })
                    }
                }
            }
            Err(error) => {
                emit_local_notice(
                    Level::Warn,
                    format_args!(
                        "local state: {domain} file refused path={}: {error}; it is neither leased \
                         nor written until it reads cleanly",
                        path.display()
                    ),
                    #[cfg(test)]
                    self.hooks.read_refusal.as_ref(),
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

    /// A readback of `key` completed: count it, and run the next one when
    /// it was asked for again while this one ran.
    fn readback_done(&mut self, key: &StoreKey) {
        if matches!(key, StoreKey::ScriptRuntimeSettings { .. }) {
            self.runtime_readback_retries.remove(key);
        }
        let seq = self.next_seq();
        if let Some(store) = self.stores.get_mut(key) {
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

    /// The clock follows every durable version the database allocated, so
    /// a later version of any store is never one already used.
    fn observe_version(&mut self, version: StoreVersion) {
        if self.clock.is_none_or(|clock| clock < version) {
            self.clock = Some(version);
        }
    }

    /// Take what the preferences database holds into the store at its own
    /// durable version. A version this service did not hold is an outside
    /// change (another process's commit, an import), counted once. The
    /// writer prepared `adoption` before it took the lock
    /// ([`Shared::adoption`]): this is the short in-memory swap of plan §3.2.
    fn adopt_durable(&mut self, adoption: Adoption) {
        let (durable, snapshot) = match adoption {
            Adoption::Same(version) => {
                self.observe_version(version);
                if let Some(store) = self.stores.get_mut(&StoreKey::Preferences) {
                    if let FileState::Read(cached) = &mut store.file {
                        if cached.version == version {
                            cached.refreshed_at = Instant::now();
                        }
                    }
                }
                return;
            }
            Adoption::New { durable, snapshot } => (durable, snapshot),
        };
        self.observe_version(durable.version);
        let Some(store) = self.stores.get_mut(&StoreKey::Preferences) else {
            return;
        };
        if let FileState::Read(cached) = &mut store.file {
            if cached.version == durable.version {
                cached.refreshed_at = Instant::now();
                return;
            }
        }
        let known = matches!(store.file, FileState::Read(_));
        if known {
            store.external += 1;
            host_log!(
                Category::Lifecycle,
                Level::Info,
                "local state: preferences changed outside this service, now {}",
                durable.version
            );
        }
        store.file = FileState::Read(Cached {
            version: durable.version,
            at: DurableTimestamp::now(),
            snapshot,
            preferences: Some(durable.projection),
            refreshed_at: Instant::now(),
            digest: Some(durable.digest),
            durable: true,
            written_len: durable.len,
            rewrite: Rewrite::Lossless,
        });
    }

    /// The database refused the preferences: no lease and no save until a
    /// later open accepts it. Logged once per distinct cause.
    fn reject_preferences(&mut self, reason: FailureReason, detail: Arc<str>) {
        let Some(store) = self.stores.get_mut(&StoreKey::Preferences) else {
            return;
        };
        if let FileState::Rejected(rejection) = &store.file {
            if rejection.detail == detail {
                return;
            }
        }
        host_log!(
            Category::Lifecycle,
            Level::Warn,
            "local state: preferences refused: {detail}; they are neither leased nor saved until \
             the database opens cleanly"
        );
        if matches!(store.file, FileState::Read(_)) {
            store.external += 1;
        }
        store.file = FileState::Rejected(Rejection { reason, detail });
    }
}

/// A durable preference version the writer read, prepared to be taken into
/// the store (plan §3.2: parsed and projected outside every service mutex,
/// published under only a short in-memory lock). The one costly step is the
/// lease rows: every member of the document serialized, up to D bytes. It is
/// done here, before the lock, so the swap under the lock only moves values.
enum Adoption {
    /// The store already holds this version: only its freshness moves.
    Same(StoreVersion),
    /// A version the store does not hold, with its lease rows built.
    New {
        durable: sql::Durable,
        snapshot: LeaseSnapshot,
    },
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
    // The first write of the PvP setting adds the whole `pvp` object (and
    // clearing it removes the object): show the one member's old and new
    // value, not "set" for the object.
    if path.as_slice() == ["pvp"] && old.is_none() != new.is_none() {
        let present = old.or(new).and_then(serde_json::Value::as_object);
        if present.is_some_and(|object| object.len() == 1 && object.contains_key("start_fights")) {
            path.push("start_fights");
            collect_preference_audit_change(
                old.and_then(|value| value.get("start_fights")),
                new.and_then(|value| value.get("start_fights")),
                path,
                changes,
                total,
            );
            path.pop();
            return;
        }
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
            ) =>
        {
            Some(PreferenceAuditScalarKind::Boolean)
        }
        ["pvp", "start_fights"] => Some(PreferenceAuditScalarKind::Boolean),
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
    fn summary_shows_the_pvp_setting_old_and_new() {
        let before = json!({ "nav": { "allow_teleports": true } });
        let after = json!({ "nav": { "allow_teleports": true }, "pvp": { "start_fights": true } });
        let summary = preference_audit_summary(
            before.as_object().expect("an object"),
            after.as_object().expect("an object"),
        )
        .expect("a changed preference document");
        assert!(
            summary.text.contains("pvp.start_fights"),
            "{}",
            summary.text
        );
        assert!(summary.text.contains("true"), "{}", summary.text);
        let off = json!({ "pvp": { "start_fights": false } });
        let summary = preference_audit_summary(
            after.as_object().expect("an object"),
            off.as_object().expect("an object"),
        )
        .expect("a changed preference document");
        assert!(summary.text.contains("pvp.start_fights: true -> false"));
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
    /// Whether a read of `key` would queue behind the preferences' first
    /// import while it runs: `key` is another store, and the preferences are
    /// open, still cold, and their read is in flight on the writer (selected
    /// and not yet ended). A read that is only queued, or put back to retry
    /// because another connection holds the SQL write lock, is no reason to
    /// wait: the writer takes file reads before preference jobs
    /// ([`take_job`]), so a Start keeps running (plan §4.4, §5.4). A read or
    /// refused store has settled and holds nothing back.
    fn behind_first_import(&self, key: &StoreKey, state: &State) -> bool {
        !matches!(key, StoreKey::Preferences)
            && self.open[domain_index(LocalStateDomain::Preferences)]
            && state
                .stores
                .get(&StoreKey::Preferences)
                .is_some_and(|store| {
                    matches!(store.file, FileState::Cold)
                        && matches!(store.readback, Readback::InFlight | Readback::InFlightAgain)
                })
    }

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
            Readback::QueuedAgain => store.readbacks + 2,
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

    /// The writer thread: take the next readback, write or checkpoint of
    /// any store, do its I/O with no lock held, record the outcome, and
    /// park when nothing is due (until the earliest delayed retry, if one
    /// is waiting). The preferences database lives here, on this thread
    /// only. A job that panics is contained: its request fails and the
    /// store reads again. Once the service is dropped, only hydration
    /// needed by accepted writes continues; the thread exits when every
    /// accepted write settles. The database closes with the thread, with
    /// close-time checkpoints off.
    fn run(self: Arc<Self>) {
        let seed = self.state.lock().db_seed.take().unwrap_or_default();
        let mut db = sql::Db::new(self.root.clone(), seed.frontend, seed.root_use);
        let mut after_shutdown = 0usize;
        loop {
            let (job, stopping) = {
                let mut state = self.state.lock();
                loop {
                    let next = match take_job(&mut state, Instant::now()) {
                        Ok(job) => break (job, state.shutdown),
                        Err(next) => next,
                    };
                    if state.shutdown && next.is_none() {
                        state.writer = WriterPhase::Exited;
                        drop(state);
                        drop(db);
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
                    match next {
                        Some(at) => {
                            self.wake.wait_until(&mut state, at);
                        }
                        None => self.wake.wait(&mut state),
                    }
                }
            };
            if stopping
                && matches!(
                    job,
                    Job::Write(..) | Job::OperatorSettings(OperatorJob::Edit(_))
                )
            {
                after_shutdown += 1;
            }
            if self.skip_after_abandon(&job) {
                self.settled.notify_all();
                continue;
            }
            #[cfg(test)]
            {
                db.seams = self.state.lock().sql_seams.clone();
                files::set_rows_point(db.seams.point.clone());
            }
            #[cfg(test)]
            let (job_started, class) = (Instant::now(), job_class(&job));
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| match &job {
                Job::Readback(StoreKey::Preferences) => self.readback_preferences(&mut db),
                Job::Readback(key @ StoreKey::BankHint { .. }) => self.readback_bank(&mut db, key),
                Job::Readback(key @ StoreKey::ScriptRuntimeSettings { .. }) => {
                    self.readback_runtime(&mut db, key)
                }
                Job::Readback(key) => {
                    self.readback(key);
                    None
                }
                Job::Write(StoreKey::Preferences, queued) => {
                    self.write_preferences(&mut db, queued)
                }
                Job::Write(key @ StoreKey::BankHint { .. }, queued) => {
                    self.write_bank(&mut db, key, queued)
                }
                Job::Write(key @ StoreKey::ScriptRuntimeSettings { .. }, queued) => {
                    self.write_runtime(&mut db, key, queued)
                }
                Job::Write(key, queued) => {
                    self.write(key, queued);
                    None
                }
                Job::OperatorSettings(job) => self.run_operator_job(&mut db, job),
                Job::Checkpoint => self.checkpoint(&mut db),
            }));
            match outcome {
                Ok(None) => {
                    if matches!(job, Job::Write(StoreKey::Preferences, _)) {
                        self.state.lock().preference_wait = None;
                    }
                }
                Ok(Some(cause)) => self.requeue(job, cause),
                Err(payload) => self.contain(&job, payload.as_ref()),
            }
            #[cfg(test)]
            {
                let hook = self.state.lock().hooks.job.clone();
                if let Some(hook) = hook {
                    hook(class, job_started.elapsed());
                }
            }
            self.settled.notify_all();
        }
    }

    /// A preferences job the shutdown bound overtook before it began:
    /// hydration and the idle checkpoint start no SQL once the service is
    /// abandoned (a save is cancelled at its own begun boundary).
    fn skip_after_abandon(&self, job: &Job) -> bool {
        let mut state = self.state.lock();
        if !state.abandoned {
            return false;
        }
        match job {
            Job::Readback(
                key @ (StoreKey::Preferences
                | StoreKey::BankHint { .. }
                | StoreKey::ScriptRuntimeSettings { .. }),
            ) => {
                state.readback_done(key);
            }
            Job::OperatorSettings(OperatorJob::Read) => {
                state
                    .operator_settings
                    .reject(&self.root, "the state service has stopped");
            }
            Job::Checkpoint => state.truncate.due = false,
            Job::Readback(_) | Job::Write(..) | Job::OperatorSettings(OperatorJob::Edit(_)) => {
                return false
            }
        }
        true
    }

    /// A database job asked to wait: put it back with its next delayed
    /// retry, or settle a save whose deadline passed with the wait's cause.
    fn requeue(&self, job: Job, cause: sql::WaitCause) {
        let now = Instant::now();
        let mut state = self.state.lock();
        if cause == sql::WaitCause::Wal {
            state.truncate.due = true;
        }
        match job {
            Job::Readback(key) if matches!(key, StoreKey::BankHint { .. }) => {
                let expired = state.stores.get_mut(&key).and_then(|store| {
                    store.readback = match store.readback {
                        Readback::InFlightAgain => Readback::QueuedAgain,
                        _ => Readback::Queued,
                    };
                    if matches!(store.file, FileState::Cold)
                        && store
                            .queue
                            .front()
                            .is_some_and(|queued| queued.retry.expired(now))
                    {
                        store.queue.pop_front().map(|queued| queued.terminal())
                    } else {
                        None
                    }
                });
                state.bank_readback_retry.delay(now);
                if let Some(expired) = expired {
                    self.settle_waited(&mut state, &key, vec![expired], cause);
                }
            }
            Job::Readback(key @ StoreKey::ScriptRuntimeSettings { .. }) => {
                let State {
                    stores,
                    runtime_readback_retries,
                    ..
                } = &mut *state;
                let Some(store) = stores.get_mut(&key) else {
                    return;
                };
                store.readback = Readback::Queued;
                let retry = runtime_readback_retries.entry(key.clone()).or_default();
                retry.first.get_or_insert(now);
                retry.delay(now);
                if matches!(store.file, FileState::Cold)
                    && retry.expired(now)
                    && !store.queue.is_empty()
                {
                    let expired = store
                        .queue
                        .drain(..)
                        .map(|queued| queued.terminal())
                        .collect();
                    self.settle_waited(&mut state, &key, expired, cause);
                }
            }
            Job::Readback(key) => {
                let expired_writes = {
                    let State {
                        stores,
                        preference_readback_retry,
                        ..
                    } = &mut *state;
                    let Some(store) = stores.get_mut(&key) else {
                        return;
                    };
                    store.readback = Readback::Queued;
                    let first = *preference_readback_retry.first.get_or_insert(now);
                    preference_readback_retry.delay(now);
                    // Saves accepted over a store that cannot open within
                    // their deadline settle `Busy` with the cause; the open
                    // keeps being retried.
                    if matches!(store.file, FileState::Cold)
                        && now.saturating_duration_since(first) >= PREFERENCE_DEADLINE
                    {
                        store
                            .queue
                            .drain(..)
                            .map(|queued| queued.terminal())
                            .collect()
                    } else {
                        Vec::new()
                    }
                };
                state.preference_wait = Some(cause);
                self.settle_waited(&mut state, &key, expired_writes, cause);
            }
            Job::Write(key, mut queued) => {
                let terminal = queued.terminal();
                if let Some(store) = state.stores.get_mut(&key) {
                    store.in_flight = None;
                }
                if state.abandoned {
                    drop(state);
                    self.finish_terminal(
                        &key,
                        terminal,
                        Settlement::Cancelled(Cancellation::Shutdown),
                    );
                    return;
                }
                // A newer queued bank save replaces this retry.
                if let StoreKey::BankHint { .. } = key {
                    if let Some(next) = state
                        .stores
                        .get(&key)
                        .and_then(|store| store.queue.front())
                        .map(|next| next.request)
                    {
                        let cancelled = Settlement::Cancelled(Cancellation::Superseded);
                        let settled =
                            state.settle(&key, terminal.request, cancelled.clone(), Some(next));
                        drop(state);
                        if settled {
                            self.terminal(&terminal, &cancelled, Some(next));
                        }
                        return;
                    }
                }
                if queued.retry.expired(now) {
                    if matches!(key, StoreKey::Preferences) {
                        state.preference_wait = None;
                    }
                    self.settle_waited(&mut state, &key, vec![terminal], cause);
                    return;
                }
                queued.retry.delay(now);
                if matches!(key, StoreKey::Preferences) {
                    state.preference_wait = Some(cause);
                }
                if let Some(store) = state.stores.get_mut(&key) {
                    store.queue.push_front(queued);
                }
            }
            Job::OperatorSettings(job) => {
                let abandoned = state.abandoned;
                state
                    .operator_settings
                    .requeue(job, &self.root, cause, now, abandoned);
            }
            Job::Checkpoint => {
                state.truncate.busy += 1;
                state.truncate.backoff = if state.truncate.backoff.is_zero() {
                    RETRY_FIRST
                } else {
                    (state.truncate.backoff * 2).min(RETRY_MAX)
                };
                state.truncate.retry_at = Some(now + state.truncate.backoff);
            }
        }
    }

    /// Settle `terminals` `Busy`, each carrying the wait's cause for its
    /// setting, with their terminal lines.
    fn settle_waited(
        &self,
        state: &mut parking_lot::MutexGuard<'_, State>,
        key: &StoreKey,
        terminals: Vec<Terminal>,
        cause: sql::WaitCause,
    ) {
        let settlement = Settlement::Failed(ErrorCode::Busy {
            domain: key.domain(),
        });
        let detail: Arc<str> = if matches!(key, StoreKey::Preferences) {
            Arc::from(cause.text())
        } else {
            Arc::from(format!("{} not saved: {}.", key.domain(), cause.reason()))
        };
        let mut settled = Vec::new();
        for terminal in terminals {
            if let Some(watch) = state.preference_watches.get_mut(&terminal.request) {
                watch.detail = Some(Arc::clone(&detail));
            }
            if state.settle(key, terminal.request, settlement.clone(), None) {
                settled.push(terminal);
            }
        }
        parking_lot::MutexGuard::unlocked(state, || {
            host_log!(
                Category::Lifecycle,
                Level::Warn,
                "local state: {} save reached its {PREFERENCE_DEADLINE:?} deadline: {detail}",
                key.domain()
            );
            for terminal in &settled {
                self.terminal(terminal, &settlement, None);
            }
        });
    }

    /// Settle one request with its terminal line, outside the lock.
    fn finish_terminal(&self, key: &StoreKey, terminal: Terminal, settlement: WriteSettlement) {
        if self
            .state
            .lock()
            .settle(key, terminal.request, settlement.clone(), None)
        {
            self.terminal(&terminal, &settlement, None);
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
            Job::Readback(key @ StoreKey::BankHint { .. }) => {
                state.reject_bank(key, FailureReason::Io, message.into());
                state.bank_readback_retry = Retry::default();
                state.readback_done(key);
            }
            Job::Readback(key) => {
                state.runtime_readback_retries.remove(key);
                if let Some(store) = state.stores.get_mut(key) {
                    store.readback = Readback::Idle;
                    store.readbacks += 1;
                    if matches!(key, StoreKey::ScriptRuntimeSettings { .. }) {
                        store.file = FileState::Rejected(Rejection {
                            reason: FailureReason::Io,
                            detail: Arc::from(message.as_str()),
                        });
                    }
                }
                host_log!(
                    Category::Lifecycle,
                    Level::Error,
                    "local state: reading {} panicked: {message}",
                    key.domain()
                );
                // The first read of the preferences panicked: the store would
                // stay cold for the whole run, no save would ever settle and
                // nothing asks for another read. It is refused for this run
                // instead, with its cause and remedy, as any other refusal
                // is: Starts and bank loads go on, saves settle with the
                // refusal, and a later open reads the database again.
                if matches!(key, StoreKey::Preferences)
                    && state
                        .stores
                        .get(key)
                        .is_some_and(|store| matches!(store.file, FileState::Cold))
                {
                    let detail = sql::first_read_panicked(&self.root, &message);
                    state.reject_preferences(FailureReason::Io, detail);
                }
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
            Job::OperatorSettings(job) => {
                state.operator_settings.contain(job, &self.root, &message)
            }
            Job::Checkpoint => {
                state.truncate.due = false;
                drop(state);
                host_log!(
                    Category::Lifecycle,
                    Level::Error,
                    "local state: the preferences WAL checkpoint panicked: {message}"
                );
            }
        }
    }

    fn readback(&self, key: &StoreKey) {
        let read_at = DurableTimestamp::now();
        let read = self.read_loadouts(read_at);
        let path = key.path(&self.root);
        let mut state = self.state.lock();
        state.adopt(key, read, read_at, &path);
        state.readback_done(key);
    }

    /// Hydrate the preferences from the database (open, schema, epoch,
    /// legacy import) or, once open, follow another connection's commit
    /// (`PRAGMA data_version`). `Some` when the database asked to wait.
    fn readback_preferences(&self, db: &mut sql::Db) -> Option<sql::WaitCause> {
        let key = StoreKey::Preferences;
        #[cfg(any(test, feature = "test-support"))]
        {
            let read_stall = self.state.lock().read_stall.clone();
            if let Some(stall) = read_stall {
                stall();
            }
        }
        let (clock, settled) = {
            let state = self.state.lock();
            let settled = state
                .stores
                .get(&key)
                .is_some_and(|store| matches!(store.file, FileState::Read(_)));
            (state.clock, settled)
        };
        let read = if settled && db.is_open() {
            db.refresh()
        } else {
            db.hydrate(clock).map(|hydrated| {
                for (level, line) in hydrated.notices {
                    self.notice(level, format_args!("{line}"));
                }
                Some(hydrated.durable)
            })
        };
        // The lease rows of a version the store does not hold are built here,
        // with no service lock held; the swap below only moves them.
        let read = read.map(|durable| durable.map(|durable| self.adoption(durable)));
        let mut state = self.state.lock();
        match read {
            Err(sql::SqlFailure::Wait(cause)) => {
                drop(state);
                return Some(cause);
            }
            Ok(None) => {
                if let Some(FileState::Read(cached)) =
                    state.stores.get_mut(&key).map(|store| &mut store.file)
                {
                    cached.refreshed_at = Instant::now();
                }
            }
            Ok(Some(adoption)) => state.adopt_durable(adoption),
            Err(sql::SqlFailure::Refused { reason, detail }) => {
                state.reject_preferences(reason, detail);
            }
        }
        state.preference_wait = None;
        state.preference_readback_retry = Retry::default();
        state.readback_done(&key);
        None
    }

    /// Prepare `durable` to be taken into the preferences store: a version
    /// the store already holds needs nothing, and a new one gets its lease
    /// rows built. No service lock is held while they are built: the lock
    /// is taken only for a short look at the cached version here, and by
    /// the caller for the swap (plan §3.2). A test watches the build itself,
    /// inside `files::preference_rows`, never a marker beside it.
    fn adoption(&self, durable: sql::Durable) -> Adoption {
        let held = self
            .state
            .lock()
            .stores
            .get(&StoreKey::Preferences)
            .is_some_and(|store| {
                matches!(&store.file, FileState::Read(cached) if cached.version == durable.version)
            });
        if held {
            return Adoption::Same(durable.version);
        }
        let (rows, _) = files::preference_rows(&durable.projection.document);
        Adoption::New {
            snapshot: LeaseSnapshot::preferences(rows),
            durable,
        }
    }

    /// One preference save through the database (plan §4.2–§4.5): the
    /// worker decides the edit against the durable document read inside
    /// the write transaction, at a version allocated there. `Some` when the
    /// database asked to wait: the caller requeues the save.
    fn write_preferences(&self, db: &mut sql::Db, queued: &Queued) -> Option<sql::WaitCause> {
        let key = StoreKey::Preferences;
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
                self.finish(&key, queued, io_failure(domain), false);
                return None;
            }
            let unknown = self.state.lock().write_unknown.clone();
            if unknown.is_some_and(|unknown| unknown()) {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: {domain} write outcome unknown (test fault) path={}",
                    self.root.database_file().display()
                );
                self.finish_unresolved(
                    &key,
                    queued,
                    io_failure(domain),
                    sql::unresolved_detail(
                        &"disk I/O error (test fault)",
                        Some("the test's fault"),
                    ),
                );
                return None;
            }
        }
        // Work selected before the shutdown bound that has not begun its
        // transaction when the bound passes is cancelled; its terminal is
        // already out.
        if self.state.lock().abandoned {
            self.finish(
                &key,
                queued,
                Settlement::Cancelled(Cancellation::Shutdown),
                false,
            );
            return None;
        }
        let (known, external, clock) = {
            let state = self.state.lock();
            let Some(store) = state.stores.get(&key) else {
                drop(state);
                self.finish(&key, queued, dirty(domain), false);
                return None;
            };
            let known = match &store.file {
                FileState::Read(cached) => sql::Durable {
                    version: cached.version,
                    projection: cached
                        .preferences
                        .clone()
                        .unwrap_or_else(|| state.preference_default.clone()),
                    digest: cached.digest.unwrap_or_default(),
                    len: cached.written_len,
                },
                FileState::Rejected(rejection) => {
                    let settlement = Settlement::Failed(ErrorCode::Failed {
                        domain,
                        reason: rejection.reason,
                    });
                    let detail = Arc::clone(&rejection.detail);
                    drop(state);
                    self.finish_with_detail(&key, queued, settlement, Some(detail));
                    return None;
                }
                FileState::Cold => {
                    drop(state);
                    self.finish(&key, queued, dirty(domain), false);
                    return None;
                }
            };
            (known, store.external, state.clock)
        };
        let known_version = known.version;
        let committed = db.commit(
            request,
            clock,
            Some(&known),
            || self.begin_sql_write(&key),
            |conn| sql::load_preferences(conn, Some(&known)),
            |durable: &sql::Durable| {
                let external_now = if durable.version == known_version {
                    external
                } else {
                    external + 1
                };
                let file = DecisionFile::Read(CachedDecision {
                    version: durable.version,
                    snapshot: None,
                    preferences: Some(durable.projection.clone()),
                    digest: Some(durable.digest),
                    durable: true,
                    written_len: durable.len,
                    rewrite: Rewrite::Lossless,
                });
                decide(&key, &file, external_now, queued).map_err(|settlement| match settlement {
                    Settlement::Failed(code) => code,
                    Settlement::Completed(_) | Settlement::Cancelled(_) => ErrorCode::Failed {
                        domain,
                        reason: FailureReason::Dirty,
                    },
                })
            },
        );
        match committed {
            Err(sql::SqlFailure::Wait(cause)) => Some(cause),
            Err(sql::SqlFailure::Refused { reason, detail }) => {
                self.state
                    .lock()
                    .reject_preferences(reason, Arc::clone(&detail));
                self.finish_with_detail(
                    &key,
                    queued,
                    Settlement::Failed(ErrorCode::Failed { domain, reason }),
                    Some(detail),
                );
                None
            }
            Ok(sql::Commit::Declined {
                code,
                detail,
                observed,
            }) => {
                if let Some(observed) = observed {
                    let adoption = self.adoption(observed);
                    self.state.lock().adopt_durable(adoption);
                }
                self.finish_with_detail(&key, queued, Settlement::Failed(code), detail);
                None
            }
            Ok(sql::Commit::Unresolved { detail, observed }) => {
                if let Some(observed) = observed {
                    let adoption = self.adoption(observed);
                    self.state.lock().adopt_durable(adoption);
                }
                self.finish_unresolved(&key, queued, io_failure(domain), detail);
                None
            }
            Ok(sql::Commit::Abandoned) => {
                self.finish(
                    &key,
                    queued,
                    Settlement::Cancelled(Cancellation::Shutdown),
                    false,
                );
                None
            }
            Ok(sql::Commit::Unchanged { staged, observed }) => {
                if let Some(observed) = observed {
                    let adoption = self.adoption(observed);
                    self.state.lock().adopt_durable(adoption);
                }
                self.finish(
                    &key,
                    queued,
                    Settlement::Completed(DurableReceipt::new(
                        queued.intent.identity().clone(),
                        staged.previous,
                        DurableTimestamp::now(),
                        Some(request),
                    )),
                    false,
                );
                None
            }
            Ok(sql::Commit::Committed {
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
                    if let Some(store) = state.stores.get_mut(&key) {
                        if observed.is_some() {
                            store.external += 1;
                        }
                        store.file = FileState::Read(Cached {
                            version,
                            at: staged.at,
                            snapshot: staged.snapshot,
                            preferences: staged.preferences,
                            refreshed_at: Instant::now(),
                            digest: Some(durable.digest),
                            durable: true,
                            written_len: durable.len,
                            rewrite: Rewrite::Lossless,
                        });
                    }
                }
                let now = DurableTimestamp::now();
                let identity = queued.intent.identity().clone();
                if let Some(audit) = &staged.audit {
                    let audit_record = AuditCommit::after_commit(
                        identity.clone(),
                        staged.action,
                        staged.previous,
                        version,
                        AuditOrigin::Submitted { request },
                        now,
                    );
                    if recovered {
                        host_log!(
                            Category::Lifecycle,
                            Level::Info,
                            "local state: recovered commit epoch={} request={} after an uncertain \
                             COMMIT; it is durable at {version}",
                            db.epoch().unwrap_or_default(),
                            request.get()
                        );
                    }
                    self.audit(audit_record, audit);
                }
                self.finish(
                    &key,
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

    /// The idle TRUNCATE checkpoint: `Some` when it must be tried again.
    fn checkpoint(&self, db: &mut sql::Db) -> Option<sql::WaitCause> {
        if !db.is_open() {
            self.state.lock().truncate.due = false;
            return None;
        }
        match db.truncate() {
            Ok(truncated) if !truncated.busy => {
                let mut state = self.state.lock();
                state.truncate = Truncate {
                    resets: state.truncate.resets + 1,
                    busy: state.truncate.busy,
                    ..Truncate::default()
                };
                drop(state);
                host_log!(
                    Category::Lifecycle,
                    Level::Debug,
                    "local state: preferences WAL reset (log={} checkpointed={} bytes={})",
                    truncated.log,
                    truncated.checkpointed,
                    truncated.wal_bytes
                );
                None
            }
            Ok(_) => Some(sql::WaitCause::Busy),
            Err(sql::SqlFailure::Wait(cause)) => Some(cause),
            Err(sql::SqlFailure::Refused { detail, .. }) => {
                self.state.lock().truncate.due = false;
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: the preferences WAL checkpoint failed: {detail}"
                );
                None
            }
        }
    }

    /// [`Self::finish`] for a failed request whose cause goes beyond its
    /// neutral code.
    fn finish_with_detail(
        &self,
        key: &StoreKey,
        queued: &Queued,
        settlement: WriteSettlement,
        detail: Option<Arc<str>>,
    ) {
        self.finish_detailed(key, queued, settlement, detail, false);
    }

    /// [`Self::finish_with_detail`] for a save whose COMMIT ended in a disk
    /// error that the database could not settle: its watch keeps that, so a
    /// front end says its outcome is unknown and never that it was not saved
    /// (plan §4.2).
    fn finish_unresolved(
        &self,
        key: &StoreKey,
        queued: &Queued,
        settlement: WriteSettlement,
        detail: Arc<str>,
    ) {
        self.finish_detailed(key, queued, settlement, Some(detail), true);
    }

    fn finish_detailed(
        &self,
        key: &StoreKey,
        queued: &Queued,
        settlement: WriteSettlement,
        detail: Option<Arc<str>>,
        outcome_unknown: bool,
    ) {
        if let Some(detail) = detail {
            if let Some(watch) = self
                .state
                .lock()
                .preference_watches
                .get_mut(&queued.request)
            {
                watch.detail = Some(detail);
                watch.outcome_unknown = outcome_unknown;
            }
        }
        self.finish(key, queued, settlement, false);
    }

    fn read_loadouts(&self, read_at: DurableTimestamp) -> Result<Option<ReadDocument>, ReadError> {
        self.stall_read();
        files::read_loadouts(&self.root, read_at)
    }

    fn stall_read(&self) {
        #[cfg(any(test, feature = "test-support"))]
        {
            let read_stall = self.state.lock().read_stall.clone();
            if let Some(stall) = read_stall {
                stall();
            }
        }
    }

    /// Log one line of a repair (the operator's notice, the audit line, or
    /// why the repair could not be made), and hand it to a test's hook.
    fn notice(&self, level: Level, line: std::fmt::Arguments<'_>) {
        #[cfg(test)]
        let hook = self.state.lock().hooks.notice.clone();
        emit_local_notice(
            level,
            line,
            #[cfg(test)]
            hook.as_ref(),
        );
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
        let _guard =
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
        let read = self.read_loadouts(read_at);
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
                    self.conflict_on_change(key, queued, &path, "while this write was staged");
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
        if files::loadouts_digest(&self.root).ok() != Some(Some(written)) {
            self.conflict_on_change(key, queued, &path, "while its directory entry was flushed");
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
        if files::loadouts_digest(&self.root).map_err(LastCheck::Io)? != precondition {
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

    /// The atomic abandonment/begun boundary of a preference save's SQL
    /// transaction, called once the transaction has begun and before
    /// anything is written. Under the state lock it either sees the
    /// service abandoned (`false`: the save is rolled back and stays
    /// cancelled) or marks the in-flight request begun (`true`): from then
    /// on `abandon` leaves it to its own outcome, so a cancelled request
    /// never commits and a committed one never reports cancellation.
    fn begin_sql_write(&self, key: &StoreKey) -> bool {
        let mut state = self.state.lock();
        if state.abandoned {
            return false;
        }
        if let Some(in_flight) = state
            .stores
            .get_mut(key)
            .and_then(|store| store.in_flight.as_mut())
        {
            in_flight.publishing = true;
        }
        true
    }

    /// A writer that does not take the lock changed the file during this
    /// write: take what it left into the store (an outside change at a new
    /// version) and settle the write as a conflict.
    fn conflict_on_change(&self, key: &StoreKey, queued: &Queued, path: &Path, when: &str) {
        let domain = key.domain();
        let read_at = DurableTimestamp::now();
        let read = self.read_loadouts(read_at);
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
        let (terminals, publishing) = {
            let mut state = self.state.lock();
            state
                .operator_settings
                .reject(&self.root, "the state service has stopped");
            state.abandon()
        };
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

fn io_failure(domain: LocalStateDomain) -> WriteSettlement {
    Settlement::Failed(ErrorCode::Failed {
        domain,
        reason: FailureReason::Io,
    })
}

fn dirty(domain: LocalStateDomain) -> WriteSettlement {
    Settlement::Failed(ErrorCode::Failed {
        domain,
        reason: FailureReason::Dirty,
    })
}

impl sql::Staging for Staged {
    type Durable = sql::Durable;
    const DOMAIN: LocalStateDomain = LocalStateDomain::Preferences;
    const HEAD: &'static str = sql::PREFERENCES_HEAD;
    const ACTION: &'static str = "preferences-write";

    fn unchanged(&self) -> bool {
        self.unchanged
    }

    fn digest(&self) -> ContentDigest {
        files::digest(&self.bytes)
    }

    fn set_version(&mut self, version: StoreVersion) {
        self.version = version;
    }

    fn write(&self, tx: &rusqlite::Connection, version: StoreVersion) -> rusqlite::Result<()> {
        sql::write_preference_rows(tx, &self.bytes, version)
    }

    fn durable(&self, version: StoreVersion, digest: ContentDigest) -> sql::Durable {
        sql::Durable {
            version,
            projection: self.preferences.clone().unwrap_or_default(),
            digest,
            len: self.bytes.len(),
        }
    }
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
            let bytes = bank::stored_bytes(hint.rows(), hint.observed_at().unix_seconds());
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
        WriteIntent::Port(DomainEdit::ScriptRuntimeSettings(_) | DomainEdit::Knowledge(_)) => {
            unreachable!("this domain does not use the file writer")
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

/// Whether a preference job (its readback, or the save at the front of its
/// queue) could be taken now.
fn preference_job_due(state: &State, now: Instant) -> bool {
    state
        .stores
        .get(&StoreKey::Preferences)
        .is_some_and(|store| {
            (store.readback == Readback::Queued && state.preference_readback_retry.due(now))
                || (store.in_flight.is_none()
                    && !matches!(store.file, FileState::Cold)
                    && store
                        .queue
                        .front()
                        .is_some_and(|queued| queued.retry.due(now)))
        })
}

/// The next job, or (as `Err`) when the earliest delayed retry falls due.
///
/// File stores first: the oldest queued readback, else the oldest queued
/// write of any file store with nothing in flight. Bank, loadout/Start and
/// exit-save jobs are never behind a preference retry or a checkpoint
/// (plan §4.4), and keep their FIFO order; at most [`FILE_JOB_BURST`] of
/// them run in a row while a preference job is due, so a stream of them
/// cannot starve the preferences. Then the preferences: a due
/// readback, else a due save, else the idle TRUNCATE checkpoint, which
/// runs only while no file-store job is queued. During shutdown only a
/// cold store with accepted writes retains its hydration demand.
fn take_job(state: &mut State, now: Instant) -> Result<Job, Option<Instant>> {
    if state.shutdown {
        let State {
            stores,
            preference_readback_retry,
            bank_readback_retry,
            runtime_readback_retries,
            ..
        } = &mut *state;
        for (key, store) in stores {
            let needed_by_write = matches!(store.file, FileState::Cold) && !store.queue.is_empty();
            if store.readback != Readback::Idle && !needed_by_write {
                store.readback = Readback::Idle;
                store.readbacks += 1;
                match key {
                    StoreKey::Preferences => preference_readback_retry.at = None,
                    StoreKey::BankHint { .. } => bank_readback_retry.at = None,
                    StoreKey::ScriptRuntimeSettings { .. } => {
                        runtime_readback_retries.remove(key);
                    }
                    StoreKey::Loadouts => {}
                }
            }
        }
    }
    fn earlier(next: &mut Option<Instant>, at: Option<Instant>) {
        if let Some(at) = at {
            *next = Some(next.map_or(at, |next| next.min(at)));
        }
    }
    let mut next: Option<Instant> = None;
    let files_only = |key: &StoreKey| !matches!(key, StoreKey::Preferences);
    let is_bank = |key: &StoreKey| matches!(key, StoreKey::BankHint { .. });
    let bank_due = state.bank_readback_retry.due(now);
    // The file stores' and bank jobs run ahead of the preference jobs, but
    // only [`FILE_JOB_BURST`] in a row while a preference job is due.
    let pref_due = preference_job_due(state, now);
    let file_turn = !(pref_due && state.file_job_streak >= FILE_JOB_BURST);
    if let Some(key) = state
        .stores
        .iter()
        .filter(|(key, store)| {
            file_turn
                && files_only(key)
                && matches!(store.readback, Readback::Queued | Readback::QueuedAgain)
                && (bank_due || !is_bank(key))
                && (!matches!(key, StoreKey::ScriptRuntimeSettings { .. })
                    || state
                        .runtime_readback_retries
                        .get(key)
                        .is_none_or(|retry| retry.due(now)))
        })
        .min_by_key(|(_, store)| store.readback_seq)
        .map(|(key, _)| key.clone())
    {
        let store = state.stores.get_mut(&key).expect("found above");
        store.readback = match store.readback {
            Readback::QueuedAgain => Readback::InFlightAgain,
            _ => Readback::InFlight,
        };
        if pref_due {
            state.file_job_streak += 1;
        }
        return Ok(Job::Readback(key));
    }
    if !bank_due
        && state.stores.iter().any(|(key, store)| {
            is_bank(key) && matches!(store.readback, Readback::Queued | Readback::QueuedAgain)
        })
    {
        earlier(&mut next, state.bank_readback_retry.at);
    }
    if let Some(key) = state
        .stores
        .iter()
        .filter(|(key, store)| {
            file_turn
                && files_only(key)
                && store.in_flight.is_none()
                && !matches!(store.file, FileState::Cold)
        })
        .filter_map(|(key, store)| {
            store
                .queue
                .front()
                .filter(|queued| queued.retry.due(now))
                .map(|queued| (key, queued.seq))
        })
        .min_by_key(|(_, seq)| *seq)
        .map(|(key, _)| key.clone())
    {
        if pref_due {
            state.file_job_streak += 1;
        }
        return Ok(take_write(state, key, now));
    }
    for (key, store) in &state.stores {
        if !files_only(key) {
            continue;
        }
        if store.readback == Readback::Queued
            && matches!(key, StoreKey::ScriptRuntimeSettings { .. })
        {
            earlier(
                &mut next,
                state
                    .runtime_readback_retries
                    .get(key)
                    .and_then(|retry| retry.at),
            );
        }
        if store.in_flight.is_none() && !matches!(store.file, FileState::Cold) {
            if let Some(front) = store.queue.front().filter(|queued| !queued.retry.due(now)) {
                earlier(&mut next, front.retry.at);
            }
        }
    }
    if file_turn {
        match state.operator_settings.take_job(now, state.shutdown) {
            Ok(job) => {
                if pref_due {
                    state.file_job_streak += 1;
                }
                return Ok(Job::OperatorSettings(job));
            }
            Err(at) => earlier(&mut next, at),
        }
    }
    if let Some(store) = state.stores.get_mut(&StoreKey::Preferences) {
        if store.readback == Readback::Queued {
            if state.preference_readback_retry.due(now) {
                store.readback = Readback::InFlight;
                state.file_job_streak = 0;
                return Ok(Job::Readback(StoreKey::Preferences));
            }
            earlier(&mut next, state.preference_readback_retry.at);
        }
        if store.in_flight.is_none() && !matches!(store.file, FileState::Cold) {
            if let Some(front) = store.queue.front() {
                if front.retry.due(now) {
                    state.file_job_streak = 0;
                    return Ok(take_write(state, StoreKey::Preferences, now));
                }
                earlier(&mut next, front.retry.at);
            }
        }
    }
    if state.truncate.due && !state.shutdown {
        if state.truncate.retry_at.is_none_or(|at| at <= now) {
            return Ok(Job::Checkpoint);
        }
        earlier(&mut next, state.truncate.retry_at);
    }
    Err(next)
}

/// The class a complete writer job is measured under (the occupancy gate,
/// plan §4.4): preference open or refresh, preference save, the idle
/// TRUNCATE, and the file stores' readbacks and writes (bank hints, exit
/// saves, loadouts and the Start readback).
#[cfg(test)]
fn job_class(job: &Job) -> &'static str {
    match job {
        Job::Readback(StoreKey::Preferences) => "pref-readback",
        Job::Write(StoreKey::Preferences, _) => "pref-save",
        Job::Checkpoint => "wal-truncate",
        Job::Readback(StoreKey::BankHint { .. }) => "bank-readback",
        Job::Write(StoreKey::BankHint { .. }, _) => "bank-write",
        Job::Readback(StoreKey::ScriptRuntimeSettings { .. }) => "runtime-settings-readback",
        Job::Write(StoreKey::ScriptRuntimeSettings { .. }, _) => "runtime-settings-save",
        Job::OperatorSettings(OperatorJob::Read) => "script-settings-readback",
        Job::OperatorSettings(OperatorJob::Edit(_)) => "script-settings-save",
        Job::Readback(_) => "file-readback",
        Job::Write(..) => "file-write",
    }
}

/// Take the front write of `key` into flight; a preference save's deadline
/// starts at its first attempt.
fn take_write(state: &mut State, key: StoreKey, now: Instant) -> Job {
    let store = state
        .stores
        .get_mut(&key)
        .expect("the caller found the store");
    let mut queued = store.queue.pop_front().expect("the caller found a write");
    queued.retry.first.get_or_insert(now);
    let setting = match &queued.intent {
        WriteIntent::Port(DomainEdit::ScriptRuntimeSettings(edit)) => Some(PendingRuntimeSetting {
            namespace: edit.namespace().to_string(),
            key: edit.key().to_string(),
            raw: Arc::from(edit.raw()),
        }),
        _ => None,
    };
    store.in_flight = Some(InFlight {
        terminal: queued.terminal(),
        setting,
        publishing: false,
    });
    Job::Write(key, queued)
}

fn lease_of(identity: &DomainIdentity, cached: &Cached) -> Result<DomainLease, ErrorCode> {
    DomainLease::try_new(
        identity.clone(),
        cached.version,
        cached.at,
        cached.snapshot.clone(),
    )
}

const fn domain_index(domain: LocalStateDomain) -> usize {
    match domain {
        LocalStateDomain::Preferences => 0,
        LocalStateDomain::BankHints => 1,
        LocalStateDomain::LoadoutsReferences => 2,
        LocalStateDomain::Knowledge => 3,
        LocalStateDomain::ScriptRuntimeSettings => 4,
        LocalStateDomain::OperatorScriptSettings => 5,
    }
}

#[cfg(test)]
#[path = "local_state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "local_state/sql_tests.rs"]
mod sql_tests;

#[cfg(test)]
#[path = "local_state/occupancy_tests.rs"]
mod occupancy_tests;

#[cfg(all(test, target_pointer_width = "64"))]
mod preference_memory_tests {
    use super::*;

    #[test]
    fn preference_retry_does_not_grow_account_store_buckets() {
        // A bank-hint account repeats this bucket. The readback ticket's
        // residence stamp adds one word to distinguish evicted stores.
        // SQL readback retry still belongs to the process, not each bucket.
        assert_eq!(std::mem::size_of::<(StoreKey, Store)>(), 544);
    }
}
