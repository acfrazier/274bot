//! The slot thread's hand on its account's bank memory (design-bank-snapshot
//! §1.2–§1.4).
//!
//! `Play` owns one `Arc<RwLock<BankMemory>>` per account; the slot thread is
//! the only writer. The account's hint identity on the process local-state
//! service ([`SlotHintPort`], T5.2) is resolved once, on the spawning
//! thread, and handed in beside the `Arc`, so the slot thread never
//! resolves a root.
//!
//! Lock discipline (§1.2): an idle frame takes one read guard to see nothing
//! is due; a write guard is taken only on the frames the open bank moved or
//! closed, for the relog decision at the session boundary, and for the two
//! short bookkeeping steps around hint I/O. **No guard is ever held across
//! file I/O, and no file I/O runs on the slot pump**: a load asks the state
//! service for a readback of the account's current file (bounded, on the
//! service's writer thread) and applies the rows under a short re-lock that
//! re-checks the §1.4 precedence; a save submits the rows to the service
//! under a read guard — nonblocking, so the pump goes on with the next
//! frame — and the slot thread observes the settlement on a later frame and
//! marks the published generation saved. `Play::bank_rows` therefore never
//! waits on the disk, and neither does the owning bot's frame. The compiled
//! script tick borrows the memory through a read guard the observer holds
//! for the tick alone (`script_observe.rs`).
//!
//! Chronology (REVIEW-S2-R3 H1-R3, REVIEW-S2-R4 H2-R4), kept over the
//! service: the service keeps at most one hint queued per account behind
//! the one in flight, a newer submission replacing a queued older one
//! (which settles `Cancelled(Superseded)`); the submission is made under
//! the memory's read guard, so two slots sharing one account submit in
//! generation order and the service's per-account FIFO is the memory's own
//! order; the same bytes this service already made durable settle as
//! durable without a write. Only a durable commit marks a generation saved
//! (§21 §4.3): a superseded request marks nothing (the request that
//! replaced it settles the generation, and if it fails the memory is still
//! dirty for the next save point), and a settlement the service no longer
//! holds is dropped with the memory left dirty. A leaving slot waits for
//! its own request, not for the generation it carries: an earlier failure
//! at the same generation never stands in for a retry still in flight, and
//! a superseded exit save waits for the request that replaced it to become
//! durable.
//!
//! `Play` keys the memories by the account's login identity
//! ([`memory_key`]), the identity the hint identity carries, so every
//! spelling the client logs in as one account shares one memory and one
//! generation counter, as it shares one file (REVIEW-S2-R4 H1-R4).

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::{BankMemory, FrameEvent};
use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::local_state::{
    BankHint, DomainEdit, DomainIdentity, ErrorCode, LocalStatePort, SubmitReceipt, WriteSettlement,
};
use api::snapshot::GameSnapshot;
use parking_lot::{Mutex, RwLock};
use script::bank_hints::account_component;

use super::local_state::{Durability, LocalStateService, RequestState};

/// One account's memory, shared by `Play` (UI reads) and its slot thread.
pub(crate) type SharedBankMemory = Arc<RwLock<BankMemory>>;

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
pub(crate) fn planner_rows(memory: Option<&RwLock<BankMemory>>) -> nav::bank_fetch::BankRows {
    memory.map_or_else(nav::bank_fetch::BankRows::default, |memory| {
        nav::bank_fetch::BankRows::of(&memory.read())
    })
}

/// How long a leaving slot waits for the service to settle what it queued
/// (§1.4: slot exit is a save point, and `Play` joins the slot thread
/// before the process exits). Past it the slot leaves with one hostlog
/// line; the document stays queued and the service finishes on its own
/// unless the process exits first.
const EXIT_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the session boundary waits for the service to read the
/// account's current hint file: the disk read the boundary did itself
/// before, now bounded. Past it the memory stays `Unknown` with one line,
/// and login is unaffected.
const LOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// The account's hint identity on the process state service: what the
/// spawning thread resolves and the slot thread loads and saves through.
pub(crate) struct SlotHintPort {
    service: Arc<LocalStateService>,
    identity: DomainIdentity,
}

impl SlotHintPort {
    pub(crate) fn new(service: Arc<LocalStateService>, identity: DomainIdentity) -> Self {
        Self { service, identity }
    }

    /// The file the identity spells, for log lines.
    fn path(&self) -> PathBuf {
        self.service.root().bank_hint_file(
            self.identity.account().profile(),
            self.identity.account().account(),
        )
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
            pending: Mutex::new(Vec::new()),
        }
    }

    /// Submit the memory's rows to the service under the caller's read
    /// guard (so submissions follow generation order), answered through
    /// the service's per-account settlements. The pending list is held
    /// across the submission, so an accepted request is listed before
    /// anyone can look. A refusal is logged here.
    fn submit(&self, memory: &BankMemory, why: &'static str) -> Saved {
        let observed_at = memory.observed_at().map_or(0, |at| at.unix_secs);
        let hint = match BankHint::try_new(
            self.port.identity.clone(),
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
        match self.port.service.submit(DomainEdit::Bank(hint)) {
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
        &self.memory
    }

    /// After `publish_snapshot`, every frame (§1.3): apply what the service
    /// settled since the last frame, mirror a loaded bank whose packets
    /// moved, and queue the hint on the close edge.
    pub(crate) fn observe_frame(&self, snapshot: &GameSnapshot) {
        self.apply_published();
        if !self.memory.read().frame_due(snapshot) {
            return;
        }
        let event = self.memory.write().track(snapshot, unix_now());
        match event {
            FrameEvent::Opened => {
                let (rows, generation) = {
                    let memory = self.memory.read();
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
    }

    /// The ingame session boundary, before the first script tick (§1.4
    /// precedence): a memory this process already knows is kept and becomes
    /// `Hint`; only an `Unknown` one is filled from the file. A refused file
    /// never blocks login.
    pub(crate) fn session_started(&self) {
        {
            let mut memory = self.memory.write();
            if memory.known() {
                memory.relogged();
                let (origin, rows) = (memory.origin(), memory.rows().len());
                drop(memory);
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank memory relog origin={origin:?} rows={rows}"
                );
                return;
            }
        }
        let Some(hint) = &self.hint else {
            return;
        };
        // The file is read and checked on the service's writer thread with
        // no guard held: a slow disk stalls this slot's boundary, bounded,
        // never a UI read.
        let lease = match hint
            .port
            .service
            .readback_lease(&hint.port.identity, LOAD_TIMEOUT)
        {
            Ok(Some(lease)) => lease,
            Ok(None) => return,
            Err(code) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: load rejected path={}: {code}",
                    hint.path.display()
                );
                return;
            }
        };
        let rows = lease
            .snapshot()
            .as_bank_hints()
            .map(<[(i32, i32)]>::to_vec)
            .unwrap_or_default();
        let observed_at_unix = lease.at().unix_seconds();
        let applied = {
            let mut memory = self.memory.write();
            // The §1.4 precedence, re-checked under the guard: a memory
            // this process filled meanwhile is kept, the file is dropped.
            if memory.known() {
                return;
            }
            memory
                .load_hint(rows, observed_at_unix)
                .map(|()| memory.rows().len())
        };
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
                self.memory.write().mark_saved(pending.generation);
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
        let memory = self.memory.read();
        if !memory.dirty() {
            return Saved::Clean;
        }
        hint.submit(&memory, why)
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
                        .readback_lease(&hint.port.identity, left)
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
    use api::local_state::{AccountKey, ContentScope, DomainIdentity, LocalStateDomain};
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    use api::ItemDefView;
    use parking_lot::RwLockReadGuard;
    use script::IsolatedEnv;

    const COINS: i32 = 995;
    const BANK_COM: i32 = 5292;
    const PROFILE: &str = "local-289";
    const CONTENT: &str = "test-content";
    const WAIT: Duration = Duration::from_secs(60);

    impl SlotBankMemory {
        fn read(&self) -> RwLockReadGuard<'_, BankMemory> {
            self.memory.read()
        }
    }

    struct TestState {
        scratch: IsolatedEnv,
        service: Arc<LocalStateService>,
        identity: DomainIdentity,
    }

    impl TestState {
        fn new(label: &str) -> Self {
            let scratch = IsolatedEnv::enter(label);
            let service = LocalStateService::open(
                super::super::local_state::StateRoot::new(scratch.home.join(".274bot")),
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
            assert_eq!(service.readback_lease(&identity, WAIT).unwrap(), None);
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

        fn hint_path(&self) -> std::path::PathBuf {
            self.service.root().bank_hint_file(PROFILE, "alice")
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
        service.set_write_hook(Arc::new(move |path, bytes| {
            let document: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let rows: Vec<(i32, i32)> = serde_json::from_value(document["rows"].clone())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            entered_tx
                .send(rows)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let _ = release_rx.lock().unwrap().recv();
            vault::write_private_file(path, bytes)
        }));
        Gate { entered, release }
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

    fn rows_on_disk(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap()
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

    fn write_hint(state: &TestState, account: &str, rows: &[(i32, i32)], at: u64) {
        let path = state.service.root().bank_hint_file(PROFILE, account);
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": super::super::local_state::HINT_SCHEMA_VERSION,
            "profile": PROFILE,
            "account": account,
            "observed_at_unix": at,
            "rows": rows,
        }))
        .unwrap();
        vault::write_private_file(&path, &bytes).unwrap();
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
            .try_read()
            .expect("a reader is never blocked by a save in flight");
        assert_eq!(rows.rows(), &[(COINS, 50)]);
        assert!(rows.dirty());
        drop(rows);
        let ui = play.bank_rows("alice");
        assert_eq!((ui.origin, ui.rows), (Origin::Session, vec![(COINS, 50)]));

        let mut newer = GameSnapshot::new();
        open_bank(&mut newer, 2, 43);
        memory
            .try_write()
            .expect("no writer is blocked by a save in flight")
            .track(&newer, 2);

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
        let state = TestState::new("bank-hints-failed-save");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let failed = Arc::new(AtomicBool::new(false));
        let failed_once = Arc::clone(&failed);
        state.service.set_write_hook(Arc::new(move |path, bytes| {
            if !failed_once.swap(true, Ordering::SeqCst) {
                return Err(std::io::Error::other("disk full"));
            }
            vault::write_private_file(path, bytes)
        }));
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
        let state = TestState::new("bank-hints-exit-retry");
        let slot = Arc::new(state.slot(Arc::default()));
        let path = state.hint_path();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        let failed = Arc::new(AtomicBool::new(false));
        let failed_once = Arc::clone(&failed);
        state.service.set_write_hook(Arc::new(move |path, bytes| {
            if !failed_once.swap(true, Ordering::SeqCst) {
                return Err(std::io::Error::other("disk full"));
            }
            let document: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let rows: Vec<(i32, i32)> = serde_json::from_value(document["rows"].clone())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            entered_tx
                .send(rows)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let _ = release_rx.lock().unwrap().recv();
            vault::write_private_file(path, bytes)
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
        let state = TestState::new("bank-hints-fixture");
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
        std::fs::write(&path, "sentinel").unwrap();

        for _ in 0..3 {
            slot.observe_frame(&snapshot);
        }
        slot.session_ended();
        drop(ExitSave(&slot));
        assert_eq!(
            rows_on_disk(&path),
            "sentinel",
            "exactly one save per close"
        );
    }

    #[test]
    fn idle_frames_take_no_write_and_allocate_nothing() {
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
        let state = TestState::new("bank-hints-boundary");
        let slot = state.slot(Arc::default());
        write_hint(&state, "alice", &[(COINS, 7)], 100);
        let mut snapshot = GameSnapshot::new();
        slot.session_started();
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
    fn a_rejected_file_leaves_the_memory_unknown_and_login_unaffected() {
        let state = TestState::new("bank-hints-rejected");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"schema_version\":7}").unwrap();
        slot.session_started();
        assert!(!slot.read().known());
    }

    #[test]
    fn without_a_hint_file_the_memory_lives_for_the_process() {
        let slot = SlotBankMemory::new(Arc::new(RwLock::new(BankMemory::default())), None);
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
        let mut play = offline_play();
        assert_eq!(
            play.bank_rows("ghost"),
            nav::bank_fetch::BankRows::default(),
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
        memory.write().track(&snapshot, 1);
        let rows = play.bank_rows("alice");
        assert_eq!(rows.origin, Origin::Session);
        assert_eq!(rows.rows, vec![(526, 2), (COINS, 50)]);
        memory.write().relogged();
        assert_eq!(play.bank_rows("alice").origin, Origin::Hint);
    }

    #[test]
    fn an_invalid_login_name_gets_no_hint_and_stays_unknown() {
        let scratch = IsolatedEnv::enter("bank-hints-invalid-name");
        assert!(AccountKey::try_new(PROFILE, "ab3456789xyz1").is_err());
        let slot = SlotBankMemory::new(Arc::new(RwLock::new(BankMemory::default())), None);
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
        let first_identity = &first.hint.as_ref().unwrap().port.identity;
        let second_identity = &second.hint.as_ref().unwrap().port.identity;
        let path = state.service.root().bank_hint_file(
            first_identity.account().profile(),
            first_identity.account().account(),
        );
        assert_eq!(
            state.service.root().bank_hint_file(
                second_identity.account().profile(),
                second_identity.account().account(),
            ),
            path,
            "both spellings resolve to one service file"
        );

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
        memory.write().track(&snapshot, 1);
        assert_eq!(play.bank_rows("_alice_smith_").rows, vec![(COINS, 50)]);
        assert_eq!(
            play.bank_rows("alice_smith"),
            nav::bank_fetch::BankRows::default(),
            "the login name shares nothing with the refused spelling"
        );
    }

    #[test]
    fn restarts_after_a_timed_out_exit_keep_order_and_one_writer_thread() {
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
        let calls = std::sync::atomic::AtomicUsize::new(0);
        service.set_write_hook(Arc::new(move |path, bytes| {
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    let document: serde_json::Value = serde_json::from_slice(bytes)
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    let rows: Vec<(i32, i32)> = serde_json::from_value(document["rows"].clone())
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    let _ = entered_tx.send(rows);
                    let _ = release_rx.lock().unwrap().recv();
                    vault::write_private_file(path, bytes)
                }
                1 => Err(std::io::Error::other("disk full")),
                _ => vault::write_private_file(path, bytes),
            }
        }));
        Gate { entered, release }
    }

    /// r2 item 3 (sol P1, grok High): close queues generation G, session
    /// end queues G again and supersedes it; the supersession marks
    /// nothing, the replacement fails, and G stays dirty until a durable
    /// commit: the exit save writes it.
    #[test]
    fn a_superseded_save_marks_nothing_and_a_failed_replacement_stays_dirty() {
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
        assert!(memory.read().dirty());
    }

    /// r2 item 2 (grok probe): a save queued when the slot leaves and the
    /// last service handle is dropped is finished by the service, not
    /// cancelled.
    #[test]
    fn a_save_still_queued_when_the_slot_and_service_go_is_finished() {
        let state = TestState::new("bank-hints-exit-then-drop");
        let slot = state.slot(Arc::default());
        let path = state.hint_path();
        let bob = identity("bob");
        assert_eq!(state.service.readback_lease(&bob, WAIT).unwrap(), None);
        let bob_path = state.service.root().bank_hint_file(PROFILE, "bob");
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        state.service.set_write_hook(Arc::new(move |path, bytes| {
            if path == bob_path {
                let _ = entered_tx.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
            vault::write_private_file(path, bytes)
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
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("the queued save was dropped: {error}"));
        assert!(raw.contains(&format!("\"rows\":[[{COINS},64]]")), "{raw}");
    }

    /// r2 item 8 (sol P1): a settlement the service no longer holds is
    /// dropped from the slot's pending list (never pending forever) and
    /// the rows stay dirty for the next save point.
    #[test]
    fn a_settlement_the_service_forgot_is_dropped_and_the_rows_stay_pending() {
        let state = TestState::new("bank-hints-forgotten");
        let slot = state.slot(Arc::default());
        // Hundreds of writes follow; what this checks is not durability, so
        // they are plain unsynced writes with no directory flushes.
        state.service.set_write_hook(Arc::new(|path, bytes| {
            std::fs::create_dir_all(path.parent().expect("a hint has a directory"))?;
            std::fs::write(path, bytes)
        }));
        state.service.set_sync_hook(Arc::new(|_| Ok(())));
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
        for count in 0..300 {
            let hint = BankHint::try_new(
                state.identity.clone(),
                None,
                vec![(COINS, 1000 + count)],
                DurableTimestamp::from_unix_seconds(2),
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
