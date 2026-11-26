//! The slot thread's hand on its account's bank memory (design-bank-snapshot
//! §1.2–§1.4).
//!
//! Play shares one memory per login identity in its bound content scope.
//! The slot resolves the persistent identity before spawning. The state
//! service owns all hint I/O. Observe only queues work and polls results.
//! A hydration applies to an unknown memory only if the session and bank
//! observation generations still match.
//!
//! The slot stamps rows when it observes them. Saves and retries carry that
//! order to the service, which retains at most one queued save per key.
//! Only a durable receipt for the current attempt and memory generation
//! marks the rows saved. Exit can wait for its own request or its successor.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::{BankMemory, FrameEvent};
use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::local_state::{BankHint, DomainIdentity, ErrorCode, SubmitReceipt, WriteSettlement};
use api::snapshot::GameSnapshot;
use parking_lot::{Mutex, RwLock};
use script::bank_hints::account_component;

use super::local_state::{
    BankHintBinding, BankHydration, Durability, LocalStateService, RequestState,
};

/// Shared by the slots and UI of a bound account. The bank lock also
/// guards changes to the session generation and observation order.
#[derive(Default)]
pub(crate) struct BankMemoryCell {
    pub(crate) bank: RwLock<BankMemory>,
    session: AtomicU64,
    observation: AtomicU64,
}

pub(crate) type SharedBankMemory = Arc<BankMemoryCell>;

/// The key `Play.bank_memories` holds an account's memory under: the
/// login identity the client decodes the typed name to
/// ([`account_component`]), the identity the hint identity carries, so
/// `Alice Smith` and `alice_smith` — one account to the server and one
/// file to the service — are one memory with one generation counter, and
/// the service's per-account chronology is the memory's own. A name that is
/// not a login name (refused by the decoder: no hint, no file) is keyed
/// by its typed spelling, which no identity can equal — an identity is at
/// most twelve lowercase letters, digits and underscores starting and
/// ending with a letter or digit, and a refused name is longer, carries
/// another byte, or has no letter or digit — so that memory is the slot's
/// alone and lives for the process.
pub(crate) fn memory_key(username: &str) -> Cow<'_, str> {
    match account_component(username) {
        Ok(identity) => Cow::Owned(identity),
        Err(_) => Cow::Borrowed(username),
    }
}

/// One copy of an account's memory for a walk arm's BankBudget planner
/// (design-bank-snapshot §4 F6): its rows with their origin, taken under a
/// read guard that ends here. `None` (no memory attached) is the
/// never-observed `Unknown` bank. Called once per arm, never per tick.
pub(crate) fn planner_rows(
    memory: Option<&RwLock<BankMemory>>,
) -> nav_planner::bank_fetch::BankRows {
    memory.map_or_else(nav_planner::bank_fetch::BankRows::default, |memory| {
        nav_planner::bank_fetch::BankRows::of(&memory.read())
    })
}

/// How long a leaving slot waits for the service to settle what it queued
/// (§1.4: slot exit is a save point, and `Play` joins the slot thread
/// before the process exits). Past it the slot leaves with one hostlog
/// line; the document stays queued and the service finishes on its own
/// unless the process exits first.
const EXIT_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// The account's hint identity on the process state service: what the
/// spawning thread resolves and the slot thread loads and saves through.
pub(crate) struct SlotHintPort {
    service: Arc<LocalStateService>,
    binding: Arc<BankHintBinding>,
}

impl SlotHintPort {
    pub(crate) fn new(service: Arc<LocalStateService>, identity: DomainIdentity) -> Self {
        Self {
            service,
            binding: Arc::new(BankHintBinding::new(identity)),
        }
    }

    /// The database file the hint lives in, for log lines.
    fn path(&self) -> PathBuf {
        self.service.root().database_file()
    }
}

pub(crate) struct SlotBankMemory {
    memory: SharedBankMemory,
    /// `None` when the account name is not a login name or the connection
    /// has no launch profile: the memory then lives for the process only.
    hint: Option<SlotHint>,
}

/// The slot's side of hint publication: its port, and the requests it
/// submitted and has not yet applied. The slot thread, the memory's only
/// writer, applies each settlement ([`BankMemory::mark_saved`]) and logs it.
struct SlotHint {
    port: SlotHintPort,
    path: PathBuf,
    hydration: Mutex<Option<BankHydration>>,
    /// Submitted, unsettled requests: at most one in flight and one queued
    /// per account at the service, so this stays tiny.
    pending: Mutex<Vec<Pending>>,
}

/// What a save point handed the service: the request the slot's exit wait
/// follows, and the generation its settlement is read against.
#[derive(Clone, Copy)]
struct Pending {
    request: RequestId,
    generation: u64,
    rows: usize,
    why: &'static str,
}

/// What a save point did.
enum Saved {
    /// Nothing unsaved (or no hint port to save through).
    Clean,
    /// Submitted; the settlement is applied on a later frame.
    Submitted(Pending),
    /// Refused by the service (one line already logged); the rows stay
    /// pending.
    Refused(ErrorCode),
}

impl SlotHint {
    fn new(port: SlotHintPort) -> Self {
        let path = port.path();
        Self {
            port,
            path,
            hydration: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
        }
    }

    /// Submit the memory's rows to the service under the caller's read
    /// guard (so submissions follow generation order), answered through
    /// the service's per-account settlements. The pending list is held
    /// across the submission, so an accepted request is listed before
    /// anyone can look. A refusal is logged here.
    fn submit(&self, memory: &BankMemory, observation: u64, why: &'static str) -> Saved {
        let observed_at = memory.observed_at().map_or(0, |at| at.unix_secs);
        let hint = match BankHint::try_new(
            self.port.binding.identity().clone(),
            None,
            memory.rows().to_vec(),
            DurableTimestamp::from_unix_seconds(observed_at),
        ) {
            Ok(hint) => hint,
            Err(code) => {
                self.refused(why, code);
                return Saved::Refused(code);
            }
        };
        let mut pending = self.pending.lock();
        match self
            .port
            .service
            .submit_bank(&self.port.binding, hint, observation)
        {
            SubmitReceipt::Accepted { request } => {
                let submitted = Pending {
                    request,
                    generation: memory.generation(),
                    rows: memory.rows().len(),
                    why,
                };
                pending.push(submitted);
                Saved::Submitted(submitted)
            }
            SubmitReceipt::Refused(code) => {
                drop(pending);
                self.refused(why, code);
                Saved::Refused(code)
            }
        }
    }

    fn refused(&self, why: &'static str, code: ErrorCode) {
        host_log!(
            Category::BankOp,
            Level::Warn,
            "bank hints: save refused why={why} path={}: {code}",
            self.path.display()
        );
    }

    fn load_refused(&self, code: ErrorCode) {
        host_log!(
            Category::BankOp,
            Level::Warn,
            "bank hints: load rejected path={}: {code}",
            self.path.display()
        );
    }
}

impl SlotBankMemory {
    pub(crate) fn new(memory: SharedBankMemory, hint: Option<SlotHintPort>) -> Self {
        Self {
            memory,
            hint: hint.map(SlotHint::new),
        }
    }

    /// The shared memory: the observer takes its read guard for the
    /// compiled tick alone; `Play::bank_rows` reads through its own `Arc`.
    pub(crate) fn memory(&self) -> &RwLock<BankMemory> {
        &self.memory.bank
    }

    /// After `publish_snapshot`, every frame (§1.3): apply what the service
    /// settled since the last frame, mirror a loaded bank whose packets
    /// moved, and queue the hint on the close edge.
    pub(crate) fn observe_frame(&self, snapshot: &GameSnapshot) {
        self.apply_published();
        if !self.memory().read().frame_due(snapshot) {
            self.apply_hydrated();
            return;
        }
        let event = {
            let mut memory = self.memory().write();
            let event = memory.track(snapshot, unix_now());
            if matches!(event, FrameEvent::Opened | FrameEvent::Observed) {
                if let Some(hint) = &self.hint {
                    let order = hint.port.service.stamp_bank_observation(&hint.port.binding);
                    self.memory.observation.store(order, Relaxed);
                }
            }
            event
        };
        match event {
            FrameEvent::Opened => {
                let (rows, generation) = {
                    let memory = self.memory().read();
                    (memory.rows().len(), memory.generation())
                };
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank memory observed rows={rows} size={} generation={generation}",
                    snapshot.bank_size()
                );
            }
            FrameEvent::Observed | FrameEvent::Unchanged => {}
            FrameEvent::Closed => {
                self.save("close");
            }
        }
        self.apply_hydrated();
    }

    /// Start a new session without waiting for the hint. Keep known rows
    /// in memory, with `Hint` origin, until this session observes the bank.
    pub(crate) fn session_started(&self) {
        let (session, observation, known) = {
            let mut memory = self.memory().write();
            let session = self.memory.session.fetch_add(1, Relaxed) + 1;
            memory.relogged();
            (
                session,
                memory.generation(),
                memory
                    .known()
                    .then(|| (memory.origin(), memory.rows().len())),
            )
        };
        if let Some((origin, rows)) = known {
            if let Some(hint) = &self.hint {
                *hint.hydration.lock() = None;
            }
            host_log!(
                Category::BankOp,
                Level::Info,
                "bank memory relog origin={origin:?} rows={rows}"
            );
            return;
        }
        let Some(hint) = &self.hint else {
            return;
        };
        let result =
            hint.port
                .service
                .start_bank_hydration(&hint.port.binding, session, observation);
        *hint.hydration.lock() = match result {
            Ok(request) => Some(request),
            Err(code) => {
                hint.load_refused(code);
                None
            }
        };
    }

    fn apply_hydrated(&self) {
        let Some(hint) = &self.hint else {
            return;
        };
        let (request, result) = {
            let mut pending = hint.hydration.lock();
            let Some(result) = pending
                .as_ref()
                .and_then(|request| hint.port.service.poll_bank_hydration(request))
            else {
                return;
            };
            (pending.take().expect("polled a pending bank read"), result)
        };
        let mut memory = self.memory().write();
        if memory.known()
            || !request.matches(
                &hint.port.binding,
                self.memory.session.load(Relaxed),
                memory.generation(),
            )
        {
            return;
        }
        let lease = match result {
            Ok(Some(lease)) => lease,
            Ok(None) => return,
            Err(code) => {
                drop(memory);
                hint.load_refused(code);
                return;
            }
        };
        let rows = lease
            .snapshot()
            .as_bank_hints()
            .expect("a bank read delivers bank rows")
            .to_vec();
        let observed_at_unix = lease.at().unix_seconds();
        let applied = memory
            .load_hint(rows, observed_at_unix)
            .map(|()| memory.rows().len());
        drop(memory);
        match applied {
            Ok(rows) => host_log!(
                Category::BankOp,
                Level::Info,
                "bank hints: loaded rows={rows} observed_at_unix={observed_at_unix} path={}",
                hint.path.display()
            ),
            Err(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: load rejected path={}: rows: {error}",
                hint.path.display()
            ),
        }
    }

    /// The session ended (a logout, a drop, or the slot thread leaving):
    /// queue what the session observed, if anything is pending. The slot
    /// thread's exit ([`ExitSave`]) waits for it to reach the disk.
    pub(crate) fn session_ended(&self) {
        if let Some(hint) = &self.hint {
            *hint.hydration.lock() = None;
        }
        self.save("session end");
    }

    /// Apply every settlement the service reached since the last frame.
    /// One lookup per unsettled request; nothing is allocated when nothing
    /// was submitted.
    fn apply_published(&self) {
        let Some(hint) = &self.hint else {
            return;
        };
        // The settled requests are taken out under the pending lock and
        // applied after it is released: the memory's write guard is never
        // taken under it.
        let settled = {
            let mut pending = hint.pending.lock();
            if pending.is_empty() {
                return;
            }
            let mut settled = Vec::new();
            let mut index = 0;
            while index < pending.len() {
                match hint.port.service.request_state(pending[index].request) {
                    RequestState::Pending => index += 1,
                    RequestState::Settled(settlement) => {
                        settled.push((pending.swap_remove(index), Some(settlement)));
                    }
                    RequestState::Forgotten => settled.push((pending.swap_remove(index), None)),
                }
            }
            settled
        };
        for (done, settlement) in settled {
            match settlement {
                Some(settlement) => self.apply(hint, done, settlement),
                None => host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: the service no longer holds the outcome of the save why={} \
                     generation={}; the rows stay pending for the next save point path={}",
                    done.why,
                    done.generation,
                    hint.path.display()
                ),
            }
        }
    }

    /// Apply one settlement: only a durable write marks its generation
    /// saved, under a short write guard (an observation newer than the
    /// document stays pending). A superseded request marks nothing: the
    /// request that replaced it settles the generation, whichever slot sent
    /// it, and a failure leaves the memory dirty for the next save point.
    fn apply(&self, hint: &SlotHint, pending: Pending, settlement: WriteSettlement) {
        match settlement {
            Settlement::Completed(receipt) => {
                let mut memory = self.memory().write();
                if memory.generation() == pending.generation
                    && hint
                        .port
                        .service
                        .is_bank_attempt(&hint.port.binding, pending.request)
                {
                    memory.mark_saved(pending.generation);
                }
                drop(memory);
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank hints: saved rows={} why={} version={} path={}",
                    pending.rows,
                    pending.why,
                    receipt.version(),
                    hint.path.display()
                );
            }
            Settlement::Cancelled(Cancellation::Superseded) => host_log!(
                Category::BankOp,
                Level::Info,
                "bank hints: superseded generation={} why={}; the newer request settles it path={}",
                pending.generation,
                pending.why,
                hint.path.display()
            ),
            Settlement::Cancelled(reason) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: save cancelled ({reason:?}) why={} path={}",
                pending.why,
                hint.path.display()
            ),
            Settlement::Failed(code) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: save failed why={} path={}: {code}",
                pending.why,
                hint.path.display()
            ),
        }
    }

    /// A save point: submit the memory's unsaved rows, if any.
    fn save(&self, why: &'static str) -> Saved {
        let Some(hint) = &self.hint else {
            return Saved::Clean;
        };
        // A settlement that already landed is applied first, so a clean
        // memory is not queued again.
        self.apply_published();
        // The rows are taken and submitted under one read guard (a UI read
        // runs beside it): the submission is nonblocking, and holding the
        // guard across it keeps submissions in generation order when two
        // slots share the account.
        let memory = self.memory().read();
        if !memory.dirty() {
            return Saved::Clean;
        }
        hint.submit(&memory, self.memory.observation.load(Relaxed), why)
    }

    /// The slot thread is leaving: submit what is pending and wait until
    /// that request — or, when a newer document of the account replaced
    /// it, the request that did, whichever slot sent it — is durable,
    /// bounded by `timeout`. A store the service had to read again (cold)
    /// is read within the same bound and the rows submitted once more.
    /// `true` when the rows the slot last observed, or newer ones, were
    /// durable before it left.
    fn finish_within(&self, timeout: Duration) -> bool {
        let Some(hint) = &self.hint else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        let mut hydrated = false;
        let pending = loop {
            match self.save("slot exit") {
                Saved::Clean => return true,
                Saved::Submitted(pending) => break pending,
                Saved::Refused(ErrorCode::NotReady { .. }) if !hydrated => {
                    hydrated = true;
                    let left = deadline.saturating_duration_since(Instant::now());
                    if hint
                        .port
                        .service
                        .readback_lease(hint.port.binding.identity(), left)
                        .is_err()
                    {
                        return false;
                    }
                }
                Saved::Refused(_) => return false,
            }
        };
        let left = deadline.saturating_duration_since(Instant::now());
        let durable = hint.port.service.wait_durable(pending.request, left);
        self.apply_published();
        match durable {
            Durability::Durable(_) => true,
            Durability::Pending => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: slot exit left a save in flight after {}s path={}",
                    timeout.as_secs(),
                    hint.path.display()
                );
                false
            }
            Durability::NotDurable(settlement) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: slot exit save did not reach the disk ({settlement:?}) path={}",
                    hint.path.display()
                );
                false
            }
            Durability::Forgotten => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: slot exit save outcome is no longer held path={}",
                    hint.path.display()
                );
                false
            }
        }
    }
}

/// Saves the slot's pending observations when the slot thread leaves,
/// however it leaves (a Stop while parked at the title never reaches the
/// session boundary), and waits for the service, bounded by
/// [`EXIT_SAVE_TIMEOUT`].
pub(crate) struct ExitSave<'a>(pub(crate) &'a SlotBankMemory);

impl Drop for ExitSave<'_> {
    fn drop(&mut self) {
        self.0.finish_within(EXIT_SAVE_TIMEOUT);
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use api::bank_memory::Origin;
    use api::identity::{Cancellation, Settlement};
    use api::local_state::{
        AccountKey, ContentScope, DomainEdit, DomainIdentity, LocalStateDomain, LocalStatePort,
    };
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    use api::ItemDefView;
    use parking_lot::RwLockReadGuard;
    use script::IsolatedEnv;

    const COINS: i32 = 995;
    const BANK_COM: i32 = 5292;
    const PROFILE: &str = "local-289";
    const CONTENT: &str = "test-content";
    const WAIT: Duration = Duration::from_secs(60);

    /// One account's stored hint as the state database holds it, read
    /// through a connection of the test's own.
    struct HintView {
        root: super::super::local_state::StateRoot,
        account: String,
    }

    impl HintView {
        fn of(service: &LocalStateService, account: &str) -> Self {
            Self {
                root: service.root().clone(),
                account: account.to_string(),
            }
        }

        /// Whether the hint is stored.
        fn exists(&self) -> bool {
            self.json().is_some()
        }

        fn starts_with(&self, base: &std::path::Path) -> bool {
            self.root.path().starts_with(base)
        }

        /// `{"observed_at_unix":N,"rows":[[id,count],...]}`, or `None` while
        /// no save has reached the database.
        fn json(&self) -> Option<String> {
            let conn = rusqlite::Connection::open_with_flags(
                self.root.database_file(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .ok()?;
            let (observed_at, raw): (i64, Vec<u8>) = conn
                .query_row(
                    "SELECT observed_at, rows FROM bank_hint
                     WHERE profile = ?1 AND account = ?2 AND revision = 289 AND content = ?3",
                    rusqlite::params![PROFILE, self.account, CONTENT],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok()?;
            let rows: Vec<(i32, i32)> = raw
                .as_chunks::<8>()
                .0
                .iter()
                .map(|row| {
                    (
                        i32::from_be_bytes([row[0], row[1], row[2], row[3]]),
                        i32::from_be_bytes([row[4], row[5], row[6], row[7]]),
                    )
                })
                .collect();
            Some(serde_json::json!({ "observed_at_unix": observed_at, "rows": rows }).to_string())
        }
    }

    impl SlotBankMemory {
        fn read(&self) -> RwLockReadGuard<'_, BankMemory> {
            self.memory().read()
        }
    }

    struct TestState {
        scratch: IsolatedEnv,
        service: Arc<LocalStateService>,
        identity: DomainIdentity,
    }

    impl TestState {
        fn new(label: &str) -> Self {
            Self::open(label, None)
        }

        /// A state whose account has a legacy hint file `legacy` before its
        /// first read: the read is the import, and is not asserted here.
        fn with_legacy(label: &str, legacy: &[u8]) -> Self {
            Self::open(label, Some(legacy))
        }

        fn open(label: &str, legacy: Option<&[u8]>) -> Self {
            let scratch = IsolatedEnv::enter(label);
            let root = super::super::local_state::StateRoot::new(scratch.home.join(".274bot"));
            if let Some(legacy) = legacy {
                let path = root.bank_hint_file(PROFILE, "alice");
                vault::create_private_dir(path.parent().unwrap()).unwrap();
                vault::write_private_file(&path, legacy).unwrap();
            }
            let service = LocalStateService::open(
                root,
                &[
                    LocalStateDomain::BankHints,
                    LocalStateDomain::LoadoutsReferences,
                ],
                Arc::new(api::identity::RequestIdMinter::new()),
            );
            // Letting the writer drain past load stalls: dropping the last
            // handle abandons queued writes past this bound, and the drop
            // test below queues behind a held write on purpose.
            service.set_shutdown_timeout(Duration::from_secs(60));
            let identity = identity("alice");
            if legacy.is_none() {
                assert_eq!(service.readback_lease(&identity, WAIT).unwrap(), None);
            }
            Self {
                scratch,
                service,
                identity,
            }
        }

        fn slot(&self, memory: SharedBankMemory) -> SlotBankMemory {
            SlotBankMemory::new(
                memory,
                Some(SlotHintPort::new(
                    Arc::clone(&self.service),
                    self.identity.clone(),
                )),
            )
        }

        fn hint_path(&self) -> HintView {
            HintView::of(&self.service, "alice")
        }
    }

    fn identity(username: &str) -> DomainIdentity {
        DomainIdentity::new(
            LocalStateDomain::BankHints,
            AccountKey::try_new(PROFILE, username).unwrap(),
            ContentScope::try_new(client::io::ClientRevision::R289, CONTENT).unwrap(),
        )
    }

    struct Gate {
        entered: mpsc::Receiver<Vec<(i32, i32)>>,
        release: mpsc::Sender<()>,
    }

    fn gated_writer(service: &LocalStateService) -> Gate {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        service.set_bank_write_hook_for_test(Arc::new(move |rows| {
            let _ = entered_tx.send(rows.to_vec());
            let _ = release_rx.lock().unwrap().recv();
        }));
        Gate { entered, release }
    }

    fn gated_reader(service: &LocalStateService) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        service.set_read_stall_for_test(Arc::new(move || {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }));
        (entered, release)
    }

    fn wait_for_hint(slot: &SlotBankMemory) {
        let hint = slot.hint.as_ref().unwrap();
        let hydration = hint.hydration.lock().clone().expect("bank read queued");
        assert!(hint
            .port
            .service
            .wait_bank_hydration_for_test(&hydration, WAIT));
        slot.observe_frame(&GameSnapshot::new());
    }

    fn pending(slot: &SlotBankMemory) -> Vec<Pending> {
        slot.hint
            .as_ref()
            .expect("test slot has a hint port")
            .pending
            .lock()
            .clone()
    }

    fn last_request(slot: &SlotBankMemory) -> Pending {
        *pending(slot).last().expect("slot has submitted a request")
    }

    /// `request`'s settlement, waiting for it.
    fn settled(service: &LocalStateService, request: RequestId) -> WriteSettlement {
        match service.wait_request(request, WAIT) {
            RequestState::Settled(settlement) => settlement,
            other => panic!(
                "service did not settle request {}: {other:?}",
                request.get()
            ),
        }
    }

    fn settle(slot: &SlotBankMemory, service: &LocalStateService) -> WriteSettlement {
        let settled = settled(service, last_request(slot).request);
        slot.apply_published();
        settled
    }

    fn open_bank(snapshot: &mut GameSnapshot, generation: u64, coins: i32) {
        snapshot.seed_bank_observation(
            BANK_COM,
            generation,
            Some(vec![bank_row(COINS, coins, 0)]),
            Vec::new(),
        );
    }

    fn close_bank(snapshot: &mut GameSnapshot, generation: u64) {
        snapshot.seed_bank_observation(-1, generation, None, Vec::new());
    }

    fn rows_on_disk(view: &HintView) -> String {
        view.json().expect("a hint is stored")
    }

    fn bank_row(id: i32, count: i32, slot: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some("fixture".into()),
                stackable: id == COINS,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Component,
            slot,
            count,
            actions: Vec::new(),
            component_id: BANK_COM,
        }
    }

    /// Store `rows` observed at `at` for `account` through the service.
    fn write_hint(state: &TestState, account: &str, rows: &[(i32, i32)], at: u64) {
        let identity = identity(account);
        assert!(state.service.readback_lease(&identity, WAIT).is_ok());
        let hint = BankHint::try_new(
            identity,
            None,
            rows.to_vec(),
            DurableTimestamp::from_unix_seconds(at),
        )
        .unwrap();
        let request = state
            .service
            .submit(DomainEdit::Bank(hint))
            .request()
            .expect("accepted");
        assert!(matches!(
            settled(&state.service, request),
            Settlement::Completed(_)
        ));
    }

    fn offline_play() -> crate::Play {
        crate::run_with_io(
            &crate::PlayOptions {
                host: "127.0.0.1".into(),
                transport: client::Transport::Tcp,
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _, _| {},
        )
    }

    /// A close never waits for the publication: memory and UI reads proceed
    /// while the process local-state writer is held.
    #[test]
    fn the_close_edge_tick_returns_while_the_writer_is_stalled() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-close-stalled");
        let memory: SharedBankMemory = Arc::default();
        let slot = state.slot(Arc::clone(&memory));
        let path = state.hint_path();
        let gate = gated_writer(&state.service);
        let mut play = offline_play();
        play.bank_memories
            .insert("alice".into(), Arc::clone(&memory));

        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        let close_tick = Instant::now();
        slot.observe_frame(&snapshot);
        let close_tick = close_tick.elapsed();
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);
        assert!(!path.exists(), "nothing is published yet");
        assert!(
            close_tick < Duration::from_secs(1),
            "the close tick waited on the writer: {close_tick:?}"
        );
        for _ in 0..3 {
            slot.observe_frame(&snapshot);
        }

        let rows = memory
            .bank
            .try_read()
            .expect("a reader is never blocked by a save in flight");
        assert_eq!(rows.rows(), &[(COINS, 50)]);
        assert!(rows.dirty());
        drop(rows);
        let ui = play.bank_rows("alice");
        assert_eq!((ui.origin, ui.rows), (Origin::Session, vec![(COINS, 50)]));

        let mut newer = GameSnapshot::new();
        open_bank(&mut newer, 2, 43);
        assert!(slot.memory().try_write().is_some());
        slot.observe_frame(&newer);

        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},50]]")), "{raw}");
        assert!(
            slot.read().dirty(),
            "the newer observation is still pending"
        );
        assert_eq!(slot.read().rows(), &[(COINS, 43)]);

        slot.session_ended();
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 43)]);
        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!slot.read().dirty());
        assert!(rows_on_disk(&path).contains(&format!("\"rows\":[[{COINS},43]]")));
    }

    /// The service holds at most one queued hint per account, replacing
    /// generations 3 with 4 while generation 2 is in flight.
    #[test]
    fn quick_closes_publish_in_order_and_a_queued_document_is_superseded() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-quick-closes");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let gate = gated_writer(&state.service);
        let mut snapshot = GameSnapshot::new();
        let mut open_and_close = |generation: u64, coins: i32| {
            open_bank(&mut snapshot, generation, coins);
            slot.observe_frame(&snapshot);
            close_bank(&mut snapshot, generation);
            slot.observe_frame(&snapshot);
        };

        open_and_close(1, 50);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);
        open_and_close(2, 43);
        assert!(!path.exists(), "nothing is published yet");
        gate.release.send(()).unwrap();
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 43)]);
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        open_and_close(3, 7);
        open_and_close(4, 9);
        gate.release.send(()).unwrap();
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 9)]);
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},43]]")));
        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(
            gate.entered.try_recv().is_err(),
            "generation 3 was superseded, not published"
        );
        assert!(!slot.read().dirty(), "the newest generation was published");
        assert_eq!(slot.read().generation(), 4);
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},9]]")), "{raw}");
    }

    #[test]
    fn the_exit_save_reaches_the_disk_before_the_slot_leaves() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-exit-save");
        let slot = Arc::new(state.slot(Arc::default()));
        let path = state.hint_path();
        let gate = gated_writer(&state.service);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);
        open_bank(&mut snapshot, 2, 43);
        slot.observe_frame(&snapshot);
        assert!(slot.read().dirty());

        let exiting = Arc::clone(&slot);
        let exit = thread::spawn(move || drop(ExitSave(&exiting)));
        gate.release.send(()).unwrap();
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 43)]);
        assert!(!exit.is_finished(), "exit waits for its own queued request");
        gate.release.send(()).unwrap();
        exit.join().unwrap();
        assert!(
            !slot.read().dirty(),
            "the exit save was applied before leaving"
        );
        assert!(rows_on_disk(&path).contains(&format!("\"rows\":[[{COINS},43]]")));
    }

    #[test]
    fn the_exit_wait_is_bounded_when_the_writer_stalls() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-exit-bounded");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let gate = gated_writer(&state.service);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);

        let left = Instant::now();
        assert!(
            !slot.finish_within(Duration::from_millis(100)),
            "the stalled writer is reported, not waited out"
        );
        let waited = left.elapsed();
        assert!(
            (Duration::from_millis(100)..Duration::from_secs(5)).contains(&waited),
            "the exit wait is the bound: {waited:?}"
        );
        assert!(slot.read().dirty(), "nothing was published yet");
        assert!(!path.exists());

        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!slot.read().dirty());
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        assert!(
            gate.entered.try_recv().is_err(),
            "an identical exit document was settled without another write"
        );
    }

    #[test]
    fn a_failed_save_keeps_the_rows_pending_for_the_next_save_point() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-failed-save");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let failed = Arc::new(AtomicBool::new(false));
        let failed_once = Arc::clone(&failed);
        state
            .service
            .set_write_fault_for_test(Arc::new(move || !failed_once.swap(true, Ordering::SeqCst)));
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        let first = last_request(&slot).request;
        assert!(matches!(
            settled(&state.service, first),
            Settlement::Failed(ErrorCode::Failed {
                reason: api::local_state::FailureReason::Io,
                ..
            })
        ));
        slot.apply_published();
        assert!(
            slot.read().dirty(),
            "a failed write leaves the memory dirty"
        );
        assert!(!path.exists());
        assert!(
            slot.memory().try_write().is_some(),
            "failure releases the lock"
        );

        slot.session_ended();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!slot.read().dirty(), "the next save point reaches disk");
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
    }

    #[test]
    fn an_exit_retry_at_the_same_generation_waits_for_its_own_attempt() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-exit-retry");
        let slot = Arc::new(state.slot(Arc::default()));
        let path = state.hint_path();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        let failed = Arc::new(AtomicBool::new(false));
        let failed_once = Arc::clone(&failed);
        state
            .service
            .set_write_fault_for_test(Arc::new(move || !failed_once.swap(true, Ordering::SeqCst)));
        state
            .service
            .set_bank_write_hook_for_test(Arc::new(move |rows| {
                let _ = entered_tx.send(rows.to_vec());
                let _ = release_rx.lock().unwrap().recv();
            }));
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 17);
        slot.observe_frame(&snapshot);

        let left = Instant::now();
        drop(ExitSave(&slot));
        let waited = left.elapsed();
        assert!(
            waited < EXIT_SAVE_TIMEOUT / 2,
            "the failed publication settles the exit before its bound: {waited:?}"
        );
        assert!(slot.read().dirty(), "the failed rows stay pending");
        assert!(!path.exists());

        let retrying = Arc::clone(&slot);
        let retry = thread::spawn(move || retrying.finish_within(WAIT));
        assert_eq!(entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 17)]);
        let retry_request = last_request(&slot).request;
        assert_eq!(
            state
                .service
                .wait_request(retry_request, Duration::from_millis(100)),
            RequestState::Pending,
            "the earlier failure cannot settle the held retry"
        );
        assert!(
            !retry.is_finished(),
            "the retry is still held at publication"
        );
        assert!(slot.read().dirty(), "the held retry remains pending");
        assert!(!path.exists());

        release.send(()).unwrap();
        assert!(retry.join().unwrap());
        assert!(!slot.read().dirty(), "the retry was published and applied");
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},17]]")), "{raw}");
        assert!(
            entered.try_recv().is_err(),
            "the successful retry wrote once"
        );
    }

    #[test]
    fn fixture_open_withdraw_close_saves_once_into_the_pinned_home() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-fixture");
        let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&commits);
        state.service.set_audit_hook(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        assert!(path.starts_with(&state.scratch.home));
        let mut snapshot = GameSnapshot::new();
        slot.session_started();
        assert!(!slot.read().known(), "no file yet: Unknown");

        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().origin(), Origin::Session);
        assert_eq!(slot.read().rows(), &[(COINS, 50)]);
        assert!(!path.exists(), "no save while the bank is open");

        open_bank(&mut snapshot, 2, 43);
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().rows(), &[(COINS, 43)]);

        close_bank(&mut snapshot, 2);
        slot.observe_frame(&snapshot);
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!slot.read().dirty(), "the close saved");
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},43]]")), "{raw}");
        assert_eq!(commits.load(Ordering::SeqCst), 1, "the close saved once");

        for _ in 0..3 {
            slot.observe_frame(&snapshot);
        }
        slot.session_ended();
        drop(ExitSave(&slot));
        assert_eq!(
            commits.load(Ordering::SeqCst),
            1,
            "exactly one save per close"
        );
        assert_eq!(rows_on_disk(&path), raw);
    }

    #[test]
    fn idle_frames_take_no_write_and_allocate_nothing() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-idle");
        let slot = state.slot(Arc::default());
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        let open = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                slot.observe_frame(&snapshot);
            }
        });
        assert_eq!(open.bytes_total, 0, "an open idle frame allocates nothing");
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        let closed = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                slot.observe_frame(&snapshot);
                let _ = slot.read().count(COINS);
            }
        });
        assert_eq!(
            closed.bytes_total, 0,
            "a closed idle frame allocates nothing"
        );
    }

    #[test]
    fn boundary_precedence_keeps_a_known_memory_and_loads_an_unknown_one() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-boundary");
        let slot = state.slot(Arc::default());
        write_hint(&state, "alice", &[(COINS, 7)], 100);
        let mut snapshot = GameSnapshot::new();
        slot.session_started();
        wait_for_hint(&slot);
        assert_eq!(slot.read().origin(), Origin::Hint);
        assert_eq!(slot.read().rows(), &[(COINS, 7)]);

        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 99, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().origin(), Origin::Session);
        let generation = slot.read().generation();
        slot.session_started();
        assert_eq!(slot.read().origin(), Origin::Hint);
        assert_eq!(slot.read().rows(), &[(COINS, 99)]);
        assert!(slot.read().generation() > generation);
        assert!(slot.read().dirty(), "the unsaved session is still pending");
        slot.session_ended();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!slot.read().dirty());
        let raw = rows_on_disk(&state.hint_path());
        assert!(raw.contains(&format!("[[{COINS},99]]")), "{raw}");
    }

    #[test]
    fn a_held_bank_read_leaves_observe_running_and_unknown_until_delivery() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-held-read");
        write_hint(&state, "alice", &[(COINS, 7)], 100);
        let slot = Arc::new(state.slot(Arc::default()));
        let (entered, release) = gated_reader(&state.service);
        let (progress, returned) = mpsc::channel();
        let observing = Arc::clone(&slot);
        let worker = thread::spawn(move || {
            observing.session_started();
            observing.observe_frame(&GameSnapshot::new());
            progress.send(observing.read().origin()).unwrap();
        });
        entered.recv_timeout(WAIT).unwrap();
        let while_held = returned.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(while_held, Ok(Origin::Unknown));
        wait_for_hint(&slot);
        assert_eq!(slot.read().origin(), Origin::Hint);
        assert_eq!(slot.read().rows(), &[(COINS, 7)]);
        assert_eq!(slot.read().observed_at().unwrap().unix_secs, 100);
    }

    #[test]
    fn the_previous_sessions_read_cannot_fill_the_new_session() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-held-relog");
        write_hint(&state, "alice", &[(COINS, 7)], 100);
        let memory = SharedBankMemory::default();
        let first = state.slot(Arc::clone(&memory));
        let next = state.slot(memory);
        let (entered, release) = gated_reader(&state.service);
        first.session_started();
        entered.recv_timeout(WAIT).unwrap();
        next.session_started();
        release.send(()).unwrap();
        entered.recv_timeout(WAIT).unwrap();
        let frame = GameSnapshot::new();
        first.observe_frame(&frame);
        next.observe_frame(&frame);
        assert_eq!(next.read().origin(), Origin::Unknown);
        release.send(()).unwrap();
        wait_for_hint(&next);
        assert_eq!(next.read().origin(), Origin::Hint);
        assert_eq!(next.read().rows(), &[(COINS, 7)]);
    }

    #[test]
    fn a_live_bank_close_during_cold_hydration_keeps_and_saves_the_live_rows() {
        let _state_lease = crate::test_process::state();
        let state = TestState::with_legacy(
            "bank-hints-held-live",
            br#"{"schema_version":1,"profile":"local-289","account":"alice","revision":289,"content":"test-content","observed_at_unix":100,"rows":[[995,7]]}"#,
        );
        let slot = state.slot(Arc::default());
        let (entered, release) = gated_reader(&state.service);
        slot.session_started();
        entered.recv_timeout(WAIT).unwrap();
        let mut frame = GameSnapshot::new();
        open_bank(&mut frame, 1, 99);
        slot.observe_frame(&frame);
        close_bank(&mut frame, 1);
        slot.observe_frame(&frame);
        let before_release = (slot.read().origin(), slot.read().rows().to_vec());
        // Release this read and any retry from the first import.
        drop(release);
        assert_eq!(before_release, (Origin::Session, vec![(COINS, 99)]));
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        slot.observe_frame(&frame);
        assert_eq!(slot.read().origin(), Origin::Session);
        assert_eq!(slot.read().rows(), &[(COINS, 99)]);
        assert!(!slot.read().dirty());
        assert!(rows_on_disk(&state.hint_path()).contains("\"rows\":[[995,99]]"));
    }

    #[test]
    fn a_rejected_file_leaves_the_memory_unknown_and_login_unaffected() {
        let _state_lease = crate::test_process::state();
        let state = TestState::with_legacy("bank-hints-rejected", b"{\"schema_version\":7}");
        let slot = state.slot(Arc::default());
        slot.session_started();
        wait_for_hint(&slot);
        assert!(!slot.read().known());
    }

    #[test]
    fn without_a_hint_file_the_memory_lives_for_the_process() {
        let _state_lease = crate::test_process::state();
        let slot = SlotBankMemory::new(Arc::default(), None);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 5, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        slot.observe_frame(&snapshot);
        assert!(slot.read().dirty(), "nothing to save into");
        slot.session_started();
        assert_eq!(slot.read().origin(), Origin::Hint);
        assert_eq!(slot.read().rows(), &[(COINS, 5)]);
    }

    #[test]
    fn play_bank_rows_copies_the_memory_with_its_origin() {
        let _state_lease = crate::test_process::state();
        let mut play = offline_play();
        assert_eq!(
            play.bank_rows("ghost"),
            nav_planner::bank_fetch::BankRows::default(),
            "an unknown account is Unknown with no rows"
        );
        let memory: SharedBankMemory = Arc::default();
        play.bank_memories
            .insert("alice".into(), Arc::clone(&memory));
        assert_eq!(play.bank_rows("alice").origin, Origin::Unknown);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(
            BANK_COM,
            1,
            Some(vec![bank_row(COINS, 50, 0), bank_row(526, 2, 1)]),
            Vec::new(),
        );
        memory.bank.write().track(&snapshot, 1);
        let rows = play.bank_rows("alice");
        assert_eq!(rows.origin, Origin::Session);
        assert_eq!(rows.rows, vec![(526, 2), (COINS, 50)]);
        memory.bank.write().relogged();
        assert_eq!(play.bank_rows("alice").origin, Origin::Hint);
    }

    #[test]
    fn an_invalid_login_name_gets_no_hint_and_stays_unknown() {
        let _state_lease = crate::test_process::state();
        let scratch = IsolatedEnv::enter("bank-hints-invalid-name");
        assert!(AccountKey::try_new(PROFILE, "ab3456789xyz1").is_err());
        let slot = SlotBankMemory::new(Arc::default(), None);
        slot.session_started();
        assert!(!slot.read().known(), "no file to load: Unknown");
        assert_eq!(slot.read().origin(), Origin::Unknown);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        slot.session_ended();
        drop(ExitSave(&slot));
        assert!(slot.read().dirty(), "nothing to save into");
        assert!(
            !scratch.home.join(".274bot").join("bank-hints").exists(),
            "no hint directory was created for a refused name"
        );
    }

    #[test]
    fn alias_spellings_share_one_memory_and_one_generation_counter() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-alias");
        let mut play = offline_play();
        let (first, second) = {
            let mut slot_for = |typed: &str| {
                let memory = Arc::clone(
                    play.bank_memories
                        .entry(memory_key(typed).into_owned())
                        .or_default(),
                );
                let port = SlotHintPort::new(Arc::clone(&state.service), identity(typed));
                SlotBankMemory::new(memory, Some(port))
            };
            (slot_for("Alice Smith"), slot_for("alice_smith"))
        };
        assert_eq!(play.bank_memories.len(), 1, "one memory for both spellings");
        assert!(Arc::ptr_eq(&first.memory, &second.memory));
        let first_identity = first.hint.as_ref().unwrap().port.binding.identity();
        let second_identity = second.hint.as_ref().unwrap().port.binding.identity();
        assert_eq!(
            first_identity, second_identity,
            "both spellings resolve to one stored hint"
        );
        let path = HintView::of(&state.service, first_identity.account().account());

        let mut snapshot = GameSnapshot::new();
        first.session_started();
        for (generation, coins) in [(1, 10), (2, 20), (3, 30), (4, 40), (5, 50)] {
            open_bank(&mut snapshot, generation, coins);
            first.observe_frame(&snapshot);
        }
        close_bank(&mut snapshot, 5);
        first.observe_frame(&snapshot);
        assert_eq!(last_request(&first).generation, 5);
        assert!(matches!(
            settle(&first, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!first.read().dirty());
        assert!(rows_on_disk(&path).contains(&format!("\"rows\":[[{COINS},50]]")));
        drop(ExitSave(&first));

        second.session_started();
        assert_eq!(second.read().origin(), Origin::Hint);
        assert_eq!(second.read().rows(), &[(COINS, 50)]);
        assert_eq!(second.read().generation(), 6);
        open_bank(&mut snapshot, 6, 7);
        second.observe_frame(&snapshot);
        close_bank(&mut snapshot, 6);
        second.observe_frame(&snapshot);
        assert_eq!(last_request(&second).generation, 7);
        assert!(matches!(
            settle(&second, &state.service),
            Settlement::Completed(_)
        ));
        assert!(
            !second.read().dirty(),
            "the newest observation was published"
        );
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},7]]")), "{raw}");
        for typed in ["Alice Smith", "alice_smith", " alice smith"] {
            let rows = play.bank_rows(typed);
            assert_eq!(
                (rows.origin, rows.rows),
                (Origin::Session, vec![(COINS, 7)]),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn the_memory_key_is_the_login_identity_or_the_refused_spelling() {
        let _state_lease = crate::test_process::state();
        assert_eq!(memory_key("Alice Smith"), "alice_smith");
        assert_eq!(memory_key(" alice smith"), "alice_smith");
        assert_eq!(memory_key("_ab3456789xy"), "ab3456789xy");
        for refused in [
            "ab3456789xyz1",
            "_alice_smith_",
            " alice smith ",
            "____________",
            "ali.ce",
        ] {
            assert!(
                matches!(memory_key(refused), Cow::Borrowed(key) if key == refused),
                "{refused:?} is its own key"
            );
        }
        let mut play = offline_play();
        let memory: SharedBankMemory = Arc::default();
        play.bank_memories.insert(
            memory_key("_alice_smith_").into_owned(),
            Arc::clone(&memory),
        );
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        memory.bank.write().track(&snapshot, 1);
        assert_eq!(play.bank_rows("_alice_smith_").rows, vec![(COINS, 50)]);
        assert_eq!(
            play.bank_rows("alice_smith"),
            nav_planner::bank_fetch::BankRows::default(),
            "the login name shares nothing with the refused spelling"
        );
    }

    #[test]
    fn restarts_after_a_timed_out_exit_keep_order_and_one_writer_thread() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-restart-order");
        let memory: SharedBankMemory = Arc::default();
        let mut snapshot = GameSnapshot::new();
        let gate = gated_writer(&state.service);

        let slot1 = Arc::new(state.slot(Arc::clone(&memory)));
        open_bank(&mut snapshot, 1, 50);
        slot1.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot1.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);
        let first_request = pending(&slot1)[0].request;
        assert!(!slot1.finish_within(Duration::from_millis(100)));
        let slot1_duplicate = pending(&slot1).last().unwrap().request;

        let slot2 = Arc::new(state.slot(Arc::clone(&memory)));
        slot2.session_started();
        assert!(!slot2.finish_within(Duration::from_millis(100)));
        let slot2_request = last_request(&slot2).request;
        let slot3 = Arc::new(state.slot(Arc::clone(&memory)));
        slot3.session_started();
        assert!(!slot3.finish_within(Duration::from_millis(100)));
        let slot3_request = last_request(&slot3).request;
        assert!(!state.hint_path().exists(), "the first publication is held");

        let successor = Arc::new(state.slot(Arc::clone(&memory)));
        successor.session_started();
        open_bank(&mut snapshot, 2, 43);
        successor.observe_frame(&snapshot);
        close_bank(&mut snapshot, 2);
        successor.observe_frame(&snapshot);
        let newest = last_request(&successor);
        assert_eq!(newest.generation, 5);
        // The writer-thread counter was process-global; the service now owns it.
        // Recreated slots share this same service instead of starting writers.
        for other in [&slot1, &slot2, &slot3, &successor] {
            assert!(Arc::ptr_eq(
                &state.service,
                &other.hint.as_ref().unwrap().port.service
            ));
        }

        gate.release.send(()).unwrap();
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 43)]);
        successor.apply_published();
        assert!(
            successor.read().dirty(),
            "the superseded saves did not settle the held generation"
        );
        assert!(matches!(
            settled(&state.service, first_request),
            Settlement::Completed(_)
        ));
        assert_eq!(
            state.service.observe(slot1_duplicate),
            Some(Settlement::Cancelled(Cancellation::Superseded))
        );
        assert_eq!(
            state.service.observe(slot2_request),
            Some(Settlement::Cancelled(Cancellation::Superseded))
        );
        assert_eq!(
            state.service.observe(slot3_request),
            Some(Settlement::Cancelled(Cancellation::Superseded))
        );
        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&successor, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!successor.read().dirty(), "the newest document was applied");
        assert!(rows_on_disk(&state.hint_path()).contains(&format!("[[{COINS},43]]")));
        assert!(
            gate.entered.try_recv().is_err(),
            "only held and newest were written"
        );
        assert!(matches!(
            state.service.wait_durable(slot1_duplicate, WAIT),
            Durability::Durable(receipt) if receipt.request() == Some(newest.request)
        ));
    }

    #[test]
    fn a_burst_of_closes_publishes_the_newest_once_in_order() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-burst");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let gate = gated_writer(&state.service);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);

        let burst = 1_000u64;
        for generation in 2..=burst + 1 {
            open_bank(
                &mut snapshot,
                generation,
                i32::try_from(generation).unwrap(),
            );
            slot.observe_frame(&snapshot);
            close_bank(&mut snapshot, generation);
            slot.observe_frame(&snapshot);
        }
        let newest = last_request(&slot);
        assert_eq!(newest.generation, burst + 1);
        assert!(
            !path.exists(),
            "nothing is published while the writer is held"
        );

        gate.release.send(()).unwrap();
        assert_eq!(
            gate.entered.recv_timeout(WAIT).unwrap(),
            vec![(COINS, i32::try_from(burst + 1).unwrap())],
            "the newest document is queued directly after the held one"
        );
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(
            gate.entered.try_recv().is_err(),
            "the other {} requests were superseded",
            burst - 1
        );
        assert!(!slot.read().dirty());
        assert_eq!(slot.read().generation(), newest.generation);
        let raw = rows_on_disk(&path);
        assert!(
            raw.contains(&format!("\"rows\":[[{COINS},{}]]", burst + 1)),
            "{raw}"
        );
    }

    #[test]
    fn per_bot_memory_is_a_header_plus_one_reserved_table() {
        let _state_lease = crate::test_process::state();
        use std::mem::size_of;
        let memory = size_of::<BankMemory>();
        let cell = size_of::<RwLock<BankMemory>>();
        let port = size_of::<SlotHintPort>();
        let slot_hint = size_of::<SlotHint>();
        let slot = size_of::<SlotBankMemory>();
        let path = size_of::<PathBuf>();
        let pending = size_of::<Mutex<Vec<Pending>>>();
        println!(
            "per-bot bank memory: BankMemory={memory}B RwLock<BankMemory>={cell}B \
             SlotHintPort={port}B SlotHint={slot_hint}B SlotBankMemory={slot}B \
             (PathBuf={path}B pending={pending}B); \
             rows heap = 8B x bank_size once loaded; local-state writer is shared by service"
        );
        assert!(memory <= 96, "BankMemory is {memory} bytes");
        // SlotBankMemory is one shared header (the Arc) plus the slot's
        // single reserved table (its hint). A Windows PathBuf is a
        // `Wtf8Buf`: its byte `Vec` plus an `is_known_utf8` flag, which
        // padding makes 8 bytes wider than the Unix `Vec<u8>`. So the cap
        // prices that platform word in: 152 + 8 header on Unix, 160 + 8
        // header on Windows. Growth beyond either fails here.
        #[cfg(windows)]
        const SLOT_CAP: usize = 168;
        #[cfg(not(windows))]
        const SLOT_CAP: usize = 160;
        assert!(
            slot <= SLOT_CAP,
            "SlotBankMemory is {slot} bytes with its service identity"
        );
    }

    /// A writer whose first publication is held until released and whose
    /// second publication fails; later ones go through.
    fn held_then_failing_writer(service: &LocalStateService) -> Gate {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        let held = AtomicBool::new(false);
        service.set_bank_write_hook_for_test(Arc::new(move |rows| {
            if !held.swap(true, Ordering::SeqCst) {
                let _ = entered_tx.send(rows.to_vec());
                let _ = release_rx.lock().unwrap().recv();
            }
        }));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        service
            .set_write_fault_for_test(Arc::new(move || calls.fetch_add(1, Ordering::SeqCst) == 1));
        Gate { entered, release }
    }

    /// r2 item 3 (sol P1, grok High): close queues generation G, session
    /// end queues G again and supersedes it; the supersession marks
    /// nothing, the replacement fails, and G stays dirty until a durable
    /// commit: the exit save writes it.
    #[test]
    fn a_superseded_save_marks_nothing_and_a_failed_replacement_stays_dirty() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-superseded-then-failed");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let gate = held_then_failing_writer(&state.service);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);

        open_bank(&mut snapshot, 2, 64);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 2);
        slot.observe_frame(&snapshot);
        let close = last_request(&slot);
        slot.session_ended();
        let session_end = last_request(&slot);
        assert_eq!(close.generation, session_end.generation, "one generation G");
        assert_ne!(close.request, session_end.request);
        slot.observe_frame(&snapshot);
        assert!(
            slot.read().dirty(),
            "the supersession of the close is not a durable commit of G"
        );

        gate.release.send(()).unwrap();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Failed(ErrorCode::Failed { .. })
        ));
        assert!(
            slot.read().dirty(),
            "the failed replacement leaves G dirty for the next save point"
        );
        assert!(
            slot.finish_within(WAIT),
            "the exit save writes G and reports it on disk"
        );
        assert!(!slot.read().dirty());
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},64]]")), "{raw}");
    }

    /// r2 item 3: a slot whose exit save another slot's newer hint replaced
    /// waits for that replacement, and a replacement that fails is reported
    /// as not on disk (it used to read as saved once the store was quiet).
    #[test]
    fn an_exit_superseded_by_a_failing_replacement_is_not_reported_saved() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-exit-replacement-fails");
        let memory: SharedBankMemory = Arc::default();
        let leaving = Arc::new(state.slot(Arc::clone(&memory)));
        let staying = state.slot(Arc::clone(&memory));
        let gate = held_then_failing_writer(&state.service);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        leaving.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        leaving.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv_timeout(WAIT).unwrap(), vec![(COINS, 50)]);
        open_bank(&mut snapshot, 2, 43);
        leaving.observe_frame(&snapshot);

        let exiting = Arc::clone(&leaving);
        let exit = thread::spawn(move || exiting.finish_within(WAIT));
        let deadline = Instant::now() + WAIT;
        while pending(&leaving).len() < 2 {
            assert!(Instant::now() < deadline, "the exit save was not submitted");
            thread::yield_now();
        }
        open_bank(&mut snapshot, 3, 41);
        staying.observe_frame(&snapshot);
        close_bank(&mut snapshot, 3);
        staying.observe_frame(&snapshot);
        gate.release.send(()).unwrap();
        assert!(
            !exit.join().unwrap(),
            "the replacement failed: the leaving slot's rows are not on disk"
        );
        assert!(memory.bank.read().dirty());
    }

    /// r2 item 2 (grok probe): a save queued when the slot leaves and the
    /// last service handle is dropped is finished by the service, not
    /// cancelled.
    #[test]
    fn a_save_still_queued_when_the_slot_and_service_go_is_finished() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-exit-then-drop");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let bob = identity("bob");
        assert_eq!(state.service.readback_lease(&bob, WAIT).unwrap(), None);
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        // Bob's save is held inside its transaction.
        state
            .service
            .set_bank_write_hook_for_test(Arc::new(move |rows| {
                if rows == [(COINS, 1)] {
                    let _ = entered_tx.send(());
                    let _ = release_rx.lock().unwrap().recv();
                }
            }));
        let hint = BankHint::try_new(
            bob,
            None,
            vec![(COINS, 1)],
            DurableTimestamp::from_unix_seconds(1),
        )
        .unwrap();
        assert!(state
            .service
            .submit(DomainEdit::Bank(hint))
            .request()
            .is_some());
        entered.recv_timeout(WAIT).unwrap();

        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 64);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        slot.session_ended();
        assert!(
            !slot.finish_within(Duration::from_millis(200)),
            "the held writer is reported, not waited out"
        );
        assert!(slot.read().dirty());

        let TestState {
            scratch: _scratch,
            service,
            ..
        } = state;
        drop(slot);
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            let _ = release.send(());
        });
        drop(service);
        releaser.join().unwrap();
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},64]]")), "{raw}");
    }

    /// r2 item 8 (sol P1): a settlement the service no longer holds is
    /// dropped from the slot's pending list (never pending forever) and
    /// the rows stay dirty for the next save point.
    #[test]
    fn a_settlement_the_service_forgot_is_dropped_and_the_rows_stay_pending() {
        let _state_lease = crate::test_process::state();
        let state = TestState::new("bank-hints-forgotten");
        let slot = state.slot(Arc::default());
        // Hundreds of writes follow; what this checks is not durability.
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        let forgotten = last_request(&slot).request;
        assert!(matches!(
            settled(&state.service, forgotten),
            Settlement::Completed(_)
        ));
        // Before the slot reads it, newer settlements of the account push it
        // out of what the service keeps.
        let observed_at = slot.read().observed_at().unwrap().unix_secs;
        for _ in 0..300 {
            let hint = BankHint::try_new(
                state.identity.clone(),
                None,
                vec![(COINS, 50)],
                DurableTimestamp::from_unix_seconds(observed_at),
            )
            .unwrap();
            let request = state
                .service
                .submit(DomainEdit::Bank(hint))
                .request()
                .expect("accepted");
            assert!(matches!(
                settled(&state.service, request),
                Settlement::Completed(_)
            ));
        }
        slot.observe_frame(&snapshot);
        assert!(
            pending(&slot).is_empty(),
            "a forgotten settlement is not pending forever"
        );
        assert!(slot.read().dirty(), "its outcome is unknown: still dirty");
        slot.session_ended();
        assert!(matches!(
            settle(&slot, &state.service),
            Settlement::Completed(_)
        ));
        assert!(!slot.read().dirty());
        assert!(rows_on_disk(&state.hint_path()).contains(&format!("\"rows\":[[{COINS},50]]")));
    }
}
