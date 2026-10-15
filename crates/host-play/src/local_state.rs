//! The process local-state service (T5.2, `I-state-port`): the functional
//! [`LocalStatePort`] over the current files (`21-census-ownership-dag.md`
//! §4.2–§4.3, D10).
//!
//! One [`LocalStateService`] per composition, created by the composition
//! root with an explicit [`StateRoot`] and the set of domains it opens. It
//! owns one `local-state-writer` thread, which does every file read and
//! write: cold hydration, readbacks, and durable publication. Nothing here
//! reads `HOME`, and no caller thread ever touches a file.
//!
//! - **Leases** ([`LocalStatePort::lease`]) are the immutable owned
//!   snapshot the service last read or wrote, at the store's version. A
//!   cold store answers [`ErrorCode::NotReady`] and queues its hydration;
//!   no disk read happens on the caller's thread.
//! - **Submissions** ([`LocalStatePort::submit`]) are nonblocking: a bank
//!   hint replaces a queued hint of the same account (the replaced request
//!   settles `Cancelled(Superseded)`); loadout edits queue in order behind
//!   a small cap and refuse `Busy` beyond it. Acceptance is not durability.
//! - **Settlements** ([`LocalStatePort::observe`]) are kept in a bounded
//!   ledger: `Completed` with the [`DurableReceipt`] once the bytes are on
//!   disk, `Failed` with a typed [`ErrorCode`] (a lost expected-version
//!   race, a store I/O failure), or `Cancelled`. A write whose bytes equal
//!   the file's current bytes settles `Completed` at the current version
//!   without touching the disk: the old hint writer's "never write a
//!   document that is not newer" rule, kept.
//! - **Audit:** one `host_log!` info line ([`AuditCommit`]) per durable
//!   write, after the receipt exists.
//!
//! Versions are per store and per process incarnation: the current files
//! carry no version, so [`StoreVersion::INITIAL`] is whatever the file held
//! when first read, and each durable commit or external change observed by
//! a readback advances it. Expected-version writes therefore guard against
//! this process's own races, which is the only kind the current files have
//! (the vault-style atomic replace is the cross-process guard).
//!
//! The current files are still edited directly by the frontends' loadout
//! forms until `T5.5` moves those forms to this port. Host-only entry
//! points ([`LocalStateService::readback_lease`]) re-read a store on the
//! writer thread, bounded, at the two points the old code read the file
//! synchronously: a script Start and a session boundary.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Cancellation, DurableTimestamp, RequestId, RequestIdMinter, Settlement};
use api::loadout::Loadout;
use api::local_state::{
    AccountKey, AuditCommit, AuditOrigin, CommitAction, ContentScope, DomainEdit, DomainIdentity,
    DomainLease, DurableReceipt, ErrorCode, FailureReason, IdentityError, LeaseSnapshot,
    LocalStateDomain, LocalStatePort, StoreVersion, SubmitReceipt, WriteSettlement,
};
use parking_lot::{Condvar, Mutex};
use script::local_state::SlotState;

mod files;

use files::{ContentDigest, StoreKey};
pub use files::{StateRoot, HINT_SCHEMA_VERSION};

/// How many settlements the ledger keeps: a slot observes its own few
/// requests within frames; a reader that comes back much later reads
/// `None`, never a wrong settlement.
const LEDGER_RETENTION: usize = 256;

/// How many loadout edits may wait behind the one in flight.
const LOADOUT_QUEUE_CAP: usize = 8;

/// How long a script Start waits for the loadouts readback: the same
/// synchronous file read the Start did before, now on the writer thread.
pub const START_LEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// A test's stand-in for the file publication step: run on the writer
/// thread with no lock held, so a test can stall or fail it.
#[cfg(test)]
pub(crate) type WriteHook =
    Arc<dyn Fn(&std::path::Path, &[u8]) -> std::io::Result<()> + Send + Sync>;

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
                Arc::clone(&self.service) as Arc<dyn LocalStatePort + Send + Sync>,
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
    /// seen), bounded by [`START_LEASE_TIMEOUT`]. A name that is not a
    /// login identity, or a domain that did not answer in time, refuses
    /// the Start with its typed reason.
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
            Err(code) => Err(format!("operator loadouts: {code}")),
        }
    }
}

/// The process service: the port plus the host-only bounded waits. Cheap
/// to share as `Arc`; the writer thread stops when the last handle drops.
pub struct LocalStateService {
    shared: Arc<Shared>,
}

struct Shared {
    root: StateRoot,
    open: [bool; 4],
    minter: Arc<RequestIdMinter>,
    state: Mutex<State>,
    /// Signalled on every submission and readback request: the writer
    /// thread parks on it while nothing is queued.
    wake: Condvar,
    /// Signalled whenever a settlement lands or a readback completes: the
    /// host-only waits park on it.
    settled: Condvar,
}

#[derive(Default)]
struct State {
    stores: HashMap<StoreKey, Store>,
    ledger: HashMap<RequestId, WriteSettlement>,
    ledger_order: VecDeque<RequestId>,
    started: bool,
    shutdown: bool,
    #[cfg(test)]
    write_hook: Option<WriteHook>,
}

#[derive(Default)]
struct Store {
    /// What the service last read or wrote; `None` while cold.
    cache: Option<Cached>,
    /// A hydration or readback is queued or in flight.
    readback: Readback,
    /// How many readbacks have completed: a bounded wait follows it.
    readbacks: u64,
    /// Accepted edits not yet taken by the writer thread.
    queue: VecDeque<Queued>,
    /// The writer thread holds an edit of this store.
    in_flight: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Readback {
    #[default]
    Idle,
    Queued,
    InFlight,
}

struct Cached {
    version: StoreVersion,
    at: DurableTimestamp,
    snapshot: LeaseSnapshot,
    /// The digest of the file's bytes, `None` when there is no file.
    digest: Option<ContentDigest>,
}

struct Queued {
    request: RequestId,
    edit: DomainEdit,
}

/// One unit of writer-thread work, taken one at a time: never stored, so
/// its size is the edit's.
#[allow(clippy::large_enum_variant)]
enum Job {
    Readback(StoreKey),
    Write(StoreKey, Queued),
}

impl LocalStateService {
    /// Open the service at `root` for `domains`, minting request ids from
    /// the composition's `minter`. Domains not named stay closed: their
    /// leases and submissions answer [`ErrorCode::NotReady`], and no file
    /// is ever read or written for them. The loadouts store, when open,
    /// starts hydrating at once.
    pub fn open(
        root: StateRoot,
        domains: &[LocalStateDomain],
        minter: Arc<RequestIdMinter>,
    ) -> Arc<Self> {
        let mut open = [false; 4];
        for domain in domains {
            open[domain_index(*domain)] = true;
        }
        let service = Arc::new(Self {
            shared: Arc::new(Shared {
                root,
                open,
                minter,
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
                settled: Condvar::new(),
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
        if service.is_open(LocalStateDomain::LoadoutsReferences) {
            let mut state = service.shared.state.lock();
            service
                .shared
                .request_readback(&mut state, StoreKey::Loadouts);
        }
        service
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
    /// for the identity yet (never written); `Ok(Some)` otherwise, with
    /// the rows the file holds now, so an edit made by another writer
    /// (a frontend loadouts form, an older process) is seen. `NotReady`
    /// when the readback did not complete within `timeout`.
    ///
    /// This is the host's replacement for the two synchronous file reads
    /// the old code did: a script Start reading the loadouts file, and a
    /// session boundary reading the account's hint file. Never called on a
    /// tick.
    pub fn readback_lease(
        &self,
        identity: &DomainIdentity,
        timeout: Duration,
    ) -> Result<Option<DomainLease>, ErrorCode> {
        let key = self.store_key(identity)?;
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        let epoch = state.stores.get(&key).map_or(0, |store| store.readbacks);
        self.shared.request_readback(&mut state, key.clone());
        loop {
            let store = &state.stores[&key];
            if store.readbacks > epoch {
                let Some(cached) = &store.cache else {
                    return Err(ErrorCode::NotReady {
                        domain: identity.domain(),
                    });
                };
                if cached.digest.is_none() {
                    return Ok(None);
                }
                return lease_of(identity, cached).map(Some);
            }
            if self
                .shared
                .settled
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return Err(ErrorCode::NotReady {
                    domain: identity.domain(),
                });
            }
        }
    }

    /// Wait at most `timeout` for `request`'s settlement. `None` on
    /// timeout or for a request the ledger no longer holds.
    pub fn wait_settled(&self, request: RequestId, timeout: Duration) -> Option<WriteSettlement> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            if let Some(settlement) = state.ledger.get(&request) {
                return Some(settlement.clone());
            }
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

    /// Wait at most `timeout` until `identity`'s store has nothing queued
    /// and nothing in flight: a leaving slot's wait for the newest hint,
    /// whichever slot submitted it. `true` when quiescent in time.
    pub fn wait_quiescent(&self, identity: &DomainIdentity, timeout: Duration) -> bool {
        let Ok(key) = self.store_key(identity) else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            let busy = state
                .stores
                .get(&key)
                .is_some_and(|store| store.in_flight || !store.queue.is_empty());
            if !busy {
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
        self.shared.state.lock().write_hook = Some(hook);
    }

    fn store_key(&self, identity: &DomainIdentity) -> Result<StoreKey, ErrorCode> {
        let domain = identity.domain();
        if !self.is_open(domain) {
            return Err(ErrorCode::NotReady { domain });
        }
        StoreKey::of(identity).ok_or(ErrorCode::NotReady { domain })
    }
}

impl LocalStatePort for LocalStateService {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        let key = self.store_key(identity)?;
        let mut state = self.shared.state.lock();
        match state
            .stores
            .get(&key)
            .and_then(|store| store.cache.as_ref())
        {
            Some(cached) => lease_of(identity, cached),
            None => {
                // Cold: queue the hydration so a later lease succeeds; never
                // read the disk here.
                self.shared.request_readback(&mut state, key);
                Err(ErrorCode::NotReady {
                    domain: identity.domain(),
                })
            }
        }
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        let domain = edit.identity().domain();
        let key = match self.store_key(edit.identity()) {
            Ok(key) => key,
            Err(code) => return SubmitReceipt::Refused(code),
        };
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return SubmitReceipt::Refused(ErrorCode::Busy { domain });
        }
        let store = state.stores.entry(key.clone()).or_default();
        if store.cache.is_none() {
            // A write against a cold store would overwrite what nobody has
            // read: hydrate first.
            self.shared.request_readback(&mut state, key);
            return SubmitReceipt::Refused(ErrorCode::NotReady { domain });
        }
        let request = match self.shared.minter.mint() {
            Ok(request) => request,
            Err(_) => return SubmitReceipt::Refused(ErrorCode::Busy { domain }),
        };
        let superseded = match &key {
            // A newer hint of the same account makes a queued one moot: at
            // most one waits behind the one in flight.
            StoreKey::BankHint { .. } => store.queue.pop_front().map(|queued| queued.request),
            StoreKey::Loadouts => {
                if store.queue.len() >= LOADOUT_QUEUE_CAP {
                    return SubmitReceipt::Refused(ErrorCode::Busy { domain });
                }
                None
            }
        };
        store.queue.push_back(Queued { request, edit });
        if let Some(superseded) = superseded {
            record(
                &mut state,
                superseded,
                Settlement::Cancelled(Cancellation::Superseded),
            );
        }
        self.shared.start_writer(&mut state);
        drop(state);
        self.shared.wake.notify_one();
        if superseded.is_some() {
            self.shared.settled.notify_all();
        }
        SubmitReceipt::Accepted { request }
    }

    fn observe(&self, request: RequestId) -> Option<WriteSettlement> {
        self.shared.state.lock().ledger.get(&request).cloned()
    }
}

impl Drop for LocalStateService {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.shutdown = true;
        drop(state);
        self.shared.wake.notify_all();
    }
}

impl Shared {
    /// Queue a readback of `key` unless one is already queued or running,
    /// and make sure the writer thread is up.
    fn request_readback(self: &Arc<Self>, state: &mut State, key: StoreKey) {
        let store = state.stores.entry(key).or_default();
        if store.readback == Readback::Idle {
            store.readback = Readback::Queued;
        }
        self.start_writer(state);
        self.wake.notify_one();
    }

    fn start_writer(self: &Arc<Self>, state: &mut State) {
        if state.started || state.shutdown {
            return;
        }
        let shared = Arc::clone(self);
        match thread::Builder::new()
            .name("local-state-writer".into())
            .spawn(move || shared.run())
        {
            Ok(_) => state.started = true,
            Err(error) => host_log!(
                Category::Lifecycle,
                Level::Warn,
                "local state: writer thread: {error}"
            ),
        }
    }

    /// The writer thread: take the next readback or write of any store,
    /// do its I/O with no lock held, record the outcome, and park when
    /// nothing is queued. Exits when the service is dropped, settling
    /// whatever was still queued as cancelled.
    fn run(&self) {
        loop {
            let job = {
                let mut state = self.state.lock();
                loop {
                    if state.shutdown {
                        self.settle_all_on_shutdown(&mut state);
                        return;
                    }
                    if let Some(job) = take_job(&mut state) {
                        break job;
                    }
                    self.wake.wait(&mut state);
                }
            };
            match job {
                Job::Readback(key) => self.readback(key),
                Job::Write(key, queued) => self.write(key, queued),
            }
            self.settled.notify_all();
        }
    }

    fn settle_all_on_shutdown(&self, state: &mut State) {
        let requests: Vec<RequestId> = state
            .stores
            .values_mut()
            .flat_map(|store| store.queue.drain(..).map(|queued| queued.request))
            .collect();
        for request in requests {
            record(
                state,
                request,
                Settlement::Cancelled(Cancellation::Shutdown),
            );
        }
        self.settled.notify_all();
    }

    fn readback(&self, key: StoreKey) {
        let read_at = DurableTimestamp::now();
        let path = key.path(&self.root);
        let read = files::read(&key, &self.root, read_at);
        let mut state = self.state.lock();
        let store = state.stores.entry(key.clone()).or_default();
        match read {
            Ok(Some(document)) => {
                let changed = store
                    .cache
                    .as_ref()
                    .is_none_or(|cached| cached.digest != Some(document.digest));
                if changed {
                    let version = match &store.cache {
                        // The first read is the initial version: the file
                        // carries none of its own.
                        None => StoreVersion::INITIAL,
                        Some(cached) => match cached.version.checked_next() {
                            Some(next) => next,
                            None => cached.version,
                        },
                    };
                    if store.cache.is_some() {
                        host_log!(
                            Category::Lifecycle,
                            Level::Info,
                            "local state: {} changed outside this process, now {version} path={}",
                            key.domain(),
                            path.display()
                        );
                    }
                    store.cache = Some(Cached {
                        version,
                        at: document.at,
                        snapshot: document.snapshot,
                        digest: Some(document.digest),
                    });
                }
            }
            Ok(None) => {
                let removed = store
                    .cache
                    .as_ref()
                    .is_some_and(|cached| cached.digest.is_some());
                let version = match &store.cache {
                    Some(cached) if removed => {
                        cached.version.checked_next().unwrap_or(cached.version)
                    }
                    Some(cached) => cached.version,
                    None => StoreVersion::INITIAL,
                };
                if removed {
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        "local state: {} removed outside this process, now {version} path={}",
                        key.domain(),
                        path.display()
                    );
                }
                if store.cache.is_none() || removed {
                    store.cache = Some(Cached {
                        version,
                        at: read_at,
                        snapshot: empty_snapshot(&key),
                        digest: None,
                    });
                }
            }
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: {} file rejected path={}: {error}",
                    key.domain(),
                    path.display()
                );
                // The old readers' behaviour: a refused file is an empty
                // store (and the next durable write replaces it); a store
                // already read keeps what it holds.
                if store.cache.is_none() {
                    store.cache = Some(Cached {
                        version: StoreVersion::INITIAL,
                        at: read_at,
                        snapshot: empty_snapshot(&key),
                        digest: None,
                    });
                }
            }
        }
        store.readback = Readback::Idle;
        store.readbacks += 1;
    }

    fn write(&self, key: StoreKey, queued: Queued) {
        let Queued { request, edit } = queued;
        let domain = key.domain();
        let path = key.path(&self.root);
        // The bytes and the next snapshot are computed from the cache under
        // the lock; the publication runs with no lock held.
        let staged = {
            let state = self.state.lock();
            let cached = state.stores[&key]
                .cache
                .as_ref()
                .expect("a write was accepted only against a hydrated store");
            if !edit.applies_to(cached.version) {
                let found = cached.version;
                let expected = edit
                    .expected()
                    .expect("a blind write applies to any version");
                drop(state);
                self.finish_write(
                    &key,
                    request,
                    Settlement::Failed(ErrorCode::Conflict { expected, found }),
                );
                return;
            }
            stage(&key, cached, &edit)
        };
        let (bytes, snapshot, at, action) = match staged {
            Ok(staged) => staged,
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: {domain} encode failed path={}: {error}",
                    path.display()
                );
                self.finish_write(
                    &key,
                    request,
                    Settlement::Failed(ErrorCode::Failed {
                        domain,
                        reason: FailureReason::Schema,
                    }),
                );
                return;
            }
        };
        let digest = files::digest(&bytes);
        let (unchanged, current_version) = {
            let state = self.state.lock();
            let cached = state.stores[&key].cache.as_ref().expect("hydrated");
            (cached.digest == Some(digest), cached.version)
        };
        if unchanged {
            // The same bytes are already durable: nothing to write, and no
            // commit to audit.
            self.finish_write(
                &key,
                request,
                Settlement::Completed(DurableReceipt::new(
                    edit.identity().clone(),
                    current_version,
                    DurableTimestamp::now(),
                    Some(request),
                )),
            );
            return;
        }
        let Some(version) = current_version.checked_next() else {
            // The version space is exhausted: refuse, never wrap.
            self.finish_write(
                &key,
                request,
                Settlement::Failed(ErrorCode::Busy { domain }),
            );
            return;
        };
        let published = self.publish(&path, &bytes);
        match published {
            Ok(()) => {
                let now = DurableTimestamp::now();
                let identity = edit.identity().clone();
                {
                    let mut state = self.state.lock();
                    let store = state.stores.get_mut(&key).expect("hydrated");
                    store.cache = Some(Cached {
                        version,
                        at,
                        snapshot,
                        digest: Some(digest),
                    });
                }
                let audit = AuditCommit::after_commit(
                    identity.clone(),
                    action,
                    current_version,
                    version,
                    AuditOrigin::Submitted { request },
                    now,
                );
                host_log!(Category::Lifecycle, Level::Info, "{audit}");
                self.finish_write(
                    &key,
                    request,
                    Settlement::Completed(DurableReceipt::new(
                        identity,
                        version,
                        now,
                        Some(request),
                    )),
                );
            }
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "local state: {domain} write failed path={}: {error}",
                    path.display()
                );
                self.finish_write(
                    &key,
                    request,
                    Settlement::Failed(ErrorCode::Failed {
                        domain,
                        reason: FailureReason::Io,
                    }),
                );
            }
        }
    }

    fn finish_write(&self, key: &StoreKey, request: RequestId, settlement: WriteSettlement) {
        let mut state = self.state.lock();
        if let Some(store) = state.stores.get_mut(key) {
            store.in_flight = false;
        }
        record(&mut state, request, settlement);
    }

    #[cfg(not(test))]
    fn publish(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        files::publish(path, bytes)
    }

    #[cfg(test)]
    fn publish(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        let hook = self.state.lock().write_hook.clone();
        match hook {
            Some(hook) => hook(path, bytes),
            None => files::publish(path, bytes),
        }
    }
}

/// The next job, readbacks first (a lease is waiting on one), else the
/// oldest queued write of any store that has nothing in flight.
fn take_job(state: &mut State) -> Option<Job> {
    for (key, store) in state.stores.iter_mut() {
        if store.readback == Readback::Queued {
            store.readback = Readback::InFlight;
            return Some(Job::Readback(key.clone()));
        }
    }
    for (key, store) in state.stores.iter_mut() {
        if store.in_flight || store.cache.is_none() {
            continue;
        }
        if let Some(queued) = store.queue.pop_front() {
            store.in_flight = true;
            return Some(Job::Write(key.clone(), queued));
        }
    }
    None
}

/// Record `request`'s settlement in the bounded ledger.
fn record(state: &mut State, request: RequestId, settlement: WriteSettlement) {
    if state.ledger.insert(request, settlement).is_none() {
        state.ledger_order.push_back(request);
        while state.ledger_order.len() > LEDGER_RETENTION {
            if let Some(evicted) = state.ledger_order.pop_front() {
                state.ledger.remove(&evicted);
            }
        }
    }
}

type Staged = (Vec<u8>, LeaseSnapshot, DurableTimestamp, CommitAction);

/// The bytes, next snapshot, observation stamp and audited action of
/// applying `edit` to `cached`.
fn stage(key: &StoreKey, cached: &Cached, edit: &DomainEdit) -> Result<Staged, serde_json::Error> {
    match edit {
        DomainEdit::Bank(hint) => {
            let bytes = files::hint_bytes(key, hint.rows(), hint.observed_at())?;
            let snapshot = LeaseSnapshot::bank_hints(hint.rows().to_vec())
                .expect("an admitted hint already met the row rules");
            Ok((
                bytes,
                snapshot,
                hint.observed_at(),
                CommitAction::BankHintWrite,
            ))
        }
        DomainEdit::Loadout(edit) => {
            let rows = cached.snapshot.as_loadouts().unwrap_or(&[]);
            let next = files::apply_loadout_edit(rows, edit);
            let bytes = files::loadouts_bytes(&next)?;
            Ok((
                bytes,
                LeaseSnapshot::loadouts(next),
                DurableTimestamp::now(),
                CommitAction::LoadoutWrite,
            ))
        }
        DomainEdit::Preferences(_) | DomainEdit::Knowledge(_) => {
            unreachable!("only loadouts and bank hints have a current file")
        }
    }
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
        StoreKey::BankHint { .. } => {
            LeaseSnapshot::bank_hints(Vec::new()).expect("no rows meet the row rules")
        }
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
