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
//! order; the same bytes already on disk settle as durable without a
//! write, so a stale document is never written over a newer file. A
//! leaving slot waits for its own request, not for the generation it
//! carries: an earlier failure at the same generation never stands in for a
//! retry still in flight, and a superseded exit save waits for the newer
//! document that replaced it to settle.
//!
//! `Play` keys the memories by the account's login identity
//! ([`memory_key`]), the identity the hint identity carries, so every
//! spelling the client logs in as one account shares one memory and one
//! generation counter, as it shares one file (REVIEW-S2-R4 H1-R4).

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::bank_memory::{BankMemory, FrameEvent};
use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::local_state::{
    BankHint, DomainEdit, DomainIdentity, ErrorCode, LocalStatePort, SubmitReceipt,
};
use api::snapshot::GameSnapshot;
use parking_lot::{Mutex, RwLock};
use script::bank_hints::account_component;

use super::local_state::LocalStateService;

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
    /// the service's ledger. What was submitted, or `None` when the service
    /// refused it (a refusal already logged).
    fn submit(&self, memory: &BankMemory, why: &'static str) -> Option<Pending> {
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
                return None;
            }
        };
        match self.port.service.submit(DomainEdit::Bank(hint)) {
            SubmitReceipt::Accepted { request } => {
                let pending = Pending {
                    request,
                    generation: memory.generation(),
                    rows: memory.rows().len(),
                    why,
                };
                self.pending.lock().push(pending);
                Some(pending)
            }
            SubmitReceipt::Refused(code) => {
                self.refused(why, code);
                None
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
    /// One ledger read per unsettled request; nothing is allocated when
    /// nothing was submitted.
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
                match hint.port.service.observe(pending[index].request) {
                    Some(settlement) => settled.push((pending.swap_remove(index), settlement)),
                    None => index += 1,
                }
            }
            settled
        };
        for (done, settlement) in settled {
            self.apply(hint, done, settlement);
        }
    }

    /// Apply one settlement: a document written, or replaced in the queue
    /// by a newer generation, marks its generation saved under a short
    /// write guard (an observation newer than the document stays pending);
    /// a failure leaves the memory dirty for the next save point.
    fn apply(
        &self,
        hint: &SlotHint,
        pending: Pending,
        settlement: Settlement<api::local_state::DurableReceipt, ErrorCode>,
    ) {
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
            Settlement::Cancelled(Cancellation::Superseded) => {
                self.memory.write().mark_saved(pending.generation);
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank hints: superseded generation={} why={} path={}",
                    pending.generation,
                    pending.why,
                    hint.path.display()
                );
            }
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

    /// A save point: submit the memory's unsaved rows, if any. What was
    /// submitted, or `None` when nothing was (nothing pending, or a
    /// refusal already logged).
    fn save(&self, why: &'static str) -> Option<Pending> {
        let hint = self.hint.as_ref()?;
        // A settlement that already landed is applied first, so a clean
        // memory is not queued again.
        self.apply_published();
        // The rows are taken and submitted under one read guard (a UI read
        // runs beside it): the submission is nonblocking, and holding the
        // guard across it keeps submissions in generation order when two
        // slots share the account.
        let memory = self.memory.read();
        if !memory.dirty() {
            return None;
        }
        hint.submit(&memory, why)
    }

    /// The slot thread is leaving: submit what is pending and wait for the
    /// service to finish with that request — or, when a newer document of
    /// the account replaced it, with that one, whichever slot sent it —
    /// bounded by `timeout`. `true` when the rows the slot last observed,
    /// or newer ones, were on disk before it left.
    fn finish_within(&self, timeout: Duration) -> bool {
        let Some(pending) = self.save("slot exit") else {
            return true;
        };
        let hint = self.hint.as_ref().expect("a submission came from a hint");
        let deadline = std::time::Instant::now() + timeout;
        let settled = hint.port.service.wait_settled(pending.request, timeout);
        let on_disk = match &settled {
            Some(Settlement::Completed(_)) => true,
            Some(Settlement::Cancelled(Cancellation::Superseded)) => {
                hint.port.service.wait_quiescent(
                    &hint.port.identity,
                    deadline.saturating_duration_since(std::time::Instant::now()),
                )
            }
            Some(_) => false,
            None => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: slot exit left a save in flight after {}s path={}",
                    timeout.as_secs(),
                    hint.path.display()
                );
                false
            }
        };
        self.apply_published();
        on_disk
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
