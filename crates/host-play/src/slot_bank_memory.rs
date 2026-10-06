//! The slot thread's hand on its account's bank memory (design-bank-snapshot
//! §1.2–§1.4).
//!
//! `Play` owns one `Arc<RwLock<BankMemory>>` per account; the slot thread is
//! the only writer. The hint file is resolved once, on the spawning thread,
//! and handed in beside the `Arc`, so the slot thread never resolves `HOME`
//! and a test's thread-local `IsolatedEnv` pin covers every save.
//!
//! Lock discipline (§1.2): an idle frame takes one read guard to see nothing
//! is due; a write guard is taken only on the frames the open bank moved or
//! closed, for the relog decision at the session boundary, and for the two
//! short bookkeeping steps around hint I/O. **No guard is ever held across
//! file I/O, and no file I/O runs on the slot pump**: a load reads and
//! checks the file first and applies the rows under a short re-lock that
//! re-checks the §1.4 precedence; a save takes its document under a read
//! guard and submits it to the process's one hint writer ([`HintWriter`]),
//! which publishes it while the pump goes on with the next frame. The
//! writer hands the outcome back to the submitting slot and the slot
//! thread marks the published generation saved on its next frame.
//! `Play::bank_rows` therefore never waits on the disk, and neither does
//! the owning bot's frame. The compiled script tick borrows the memory
//! through a read guard the observer holds for the tick alone
//! (`script_observe.rs`).
//!
//! The writer is owned by the process, not by any slot (REVIEW-S2-R3
//! H1-R3): one lazily started `bank-hint-writer` thread serves every hint
//! path, keeps the newest published generation per path for the life of
//! the process, and never writes a document whose generation is not newer
//! than that. A slot that leaves past its exit bound leaves nothing
//! behind: its document stays queued under the same path, a successor's
//! newer one replaces it, and a stale one — whichever slot sent it — is
//! dropped, never written over a newer file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::{BankMemory, FrameEvent};
use api::host_log;
use api::hostlog::{Category, Level};
use api::snapshot::GameSnapshot;
use parking_lot::{Condvar, Mutex, RwLock};
use script::bank_hints::{HintError, HintFile, PendingSave};

/// One account's memory, shared by `Play` (UI reads) and its slot thread.
pub(crate) type SharedBankMemory = Arc<RwLock<BankMemory>>;

/// How long a leaving slot waits for the writer to settle what it queued
/// (§1.4: slot exit is a save point, and `Play` joins the slot thread
/// before the process exits). Past it the slot leaves with one hostlog
/// line; the document stays queued and the writer finishes on its own
/// unless the process exits first.
const EXIT_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A test's stand-in for `HintFile::write`: the publication step, run on
/// the writer thread with no guard held, which a test can stall or fail.
/// Carried by the slot's submissions, so two slots under test in one
/// process stall or fail their own publications only.
#[cfg(test)]
type Publish = Arc<dyn Fn(&HintFile, &PendingSave) -> Result<(), HintError> + Send + Sync>;

pub(crate) struct SlotBankMemory {
    memory: SharedBankMemory,
    /// `None` when the account name is not a login name or the connection
    /// has no launch profile: the memory then lives for the process only.
    hint: Option<SlotHint>,
}

/// The slot's side of hint publication: its file, and the channel the
/// writer answers its submissions on. The slot thread, the memory's only
/// writer, applies each answer ([`BankMemory::mark_saved`]) and logs it.
struct SlotHint {
    file: Arc<HintFile>,
    done: mpsc::Receiver<Published>,
    done_tx: mpsc::Sender<Published>,
    #[cfg(test)]
    publish: Option<Publish>,
    /// The generation this slot last submitted: what a test settles on.
    #[cfg(test)]
    submitted: std::cell::Cell<u64>,
}

/// The process's one hint writer: a `bank-hint-writer` thread started by
/// the first save point and kept for the life of the process, with one
/// entry per hint path it has ever been handed. A save point submits a
/// document under its path; a newer generation replaces a queued older
/// one, so at most one document per path waits behind the one in flight.
/// The thread takes the queued document of any path, writes it (atomic
/// replace, fsync) only if its generation is newer than the path's
/// published generation, records what it published, and answers the
/// submitting slot. The published generation outlives every slot, so a
/// document a departed slot left queued, or a stale one a later slot
/// submits, can never put older rows over newer ones.
static WRITER: LazyLock<HintWriter> = LazyLock::new(HintWriter::default);

#[derive(Default)]
struct HintWriter {
    state: Mutex<WriterState>,
    /// Signalled on every submission: the writer thread parks on it while
    /// no path has a document queued.
    wake: Condvar,
    /// Signalled whenever a path's `settled` generation moved: a leaving
    /// slot waits on it for its own generation.
    settled: Condvar,
}

#[derive(Default)]
struct WriterState {
    /// One entry per hint path submitted in this process, kept so the
    /// published generation survives the slot that wrote it. Bounded by
    /// the accounts the process has played.
    paths: HashMap<PathBuf, PathState>,
    /// The `bank-hint-writer` thread is running.
    started: bool,
}

struct PathState {
    /// The newest submitted document not yet taken by the writer thread.
    pending: Option<Job>,
    /// The generation of the newest document written to the path; `0`
    /// before any. Only the writer thread raises it, and only on a write
    /// that succeeded.
    published: u64,
    /// The newest generation the writer thread has finished with — written,
    /// failed or dropped as stale. A leaving slot's wait ends when this
    /// reaches its own generation: its document, or a newer one for the
    /// path, has been dealt with.
    settled: u64,
}

struct Job {
    file: Arc<HintFile>,
    document: PendingSave,
    why: &'static str,
    /// The submitting slot's channel; a slot that has left dropped its end.
    done: mpsc::Sender<Published>,
    #[cfg(test)]
    publish: Option<Publish>,
}

/// One submission's outcome, handed back to the slot that submitted it.
struct Published {
    generation: u64,
    rows: usize,
    why: &'static str,
    outcome: Outcome,
}

enum Outcome {
    /// The document was written.
    Saved,
    /// The document was not written: a generation at least as new, `by`,
    /// was already on disk when the writer took it.
    Superseded {
        by: u64,
    },
    Failed(HintError),
}

impl Job {
    #[cfg(not(test))]
    fn publish(&self) -> Result<(), HintError> {
        self.file.write(&self.document)
    }

    #[cfg(test)]
    fn publish(&self) -> Result<(), HintError> {
        match &self.publish {
            Some(publish) => publish(&self.file, &self.document),
            None => self.file.write(&self.document),
        }
    }
}

impl HintWriter {
    /// Queue `job` under its path, replacing a queued document of the same
    /// or an older generation, and start the writer thread if it is not
    /// running. A spawn failure drops the document with one hostlog line:
    /// the memory stays dirty for the next save point. `true` when the
    /// document was queued.
    fn submit(&self, job: Job) -> bool {
        let mut state = self.state.lock();
        if !state.started {
            let spawned = thread::Builder::new()
                .name("bank-hint-writer".into())
                .spawn(|| WRITER.run());
            match spawned {
                Ok(_) => {
                    state.started = true;
                    #[cfg(test)]
                    THREADS_SPAWNED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                Err(error) => {
                    drop(state);
                    host_log!(
                        Category::BankOp,
                        Level::Warn,
                        "bank hints: save failed why={} path={}: writer thread: {error}",
                        job.why,
                        job.file.path().display()
                    );
                    return false;
                }
            }
        }
        let generation = job.document.generation();
        match state.paths.get_mut(job.file.path()) {
            Some(path) => {
                // A queued newer document stays; this older one is never
                // needed (its slot's exit wait ends with the newer one).
                if path
                    .pending
                    .as_ref()
                    .is_none_or(|queued| queued.document.generation() <= generation)
                {
                    path.pending = Some(job);
                }
            }
            None => {
                state.paths.insert(
                    job.file.path().to_path_buf(),
                    PathState {
                        pending: Some(job),
                        published: 0,
                        settled: 0,
                    },
                );
            }
        }
        drop(state);
        self.wake.notify_one();
        true
    }

    /// Wait until the writer has finished with `generation` for `path` (or
    /// a newer one), or `timeout` passes. `None` on timeout; otherwise
    /// whether `generation` or a newer one is on disk.
    fn wait_settled(&self, path: &Path, generation: u64, timeout: Duration) -> Option<bool> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock();
        loop {
            // Every path a slot waits on was submitted to, so it is known.
            let known = state.paths.get(path)?;
            if known.settled >= generation {
                return Some(known.published >= generation);
            }
            if self.settled.wait_until(&mut state, deadline).timed_out() {
                return None;
            }
        }
    }

    /// The writer thread, for the life of the process: take the queued
    /// document of any path, publish it off every lock if it is newer than
    /// what the path holds, answer the slot, record the outcome, and park
    /// when nothing is queued.
    fn run(&self) {
        loop {
            let (job, published) = {
                let mut state = self.state.lock();
                let job = loop {
                    match state
                        .paths
                        .values_mut()
                        .find_map(|path| path.pending.take())
                    {
                        Some(job) => break job,
                        None => self.wake.wait(&mut state),
                    }
                };
                let published = state.paths[job.file.path()].published;
                (job, published)
            };
            let generation = job.document.generation();
            let outcome = if generation > published {
                match job.publish() {
                    Ok(()) => Outcome::Saved,
                    Err(error) => Outcome::Failed(error),
                }
            } else {
                Outcome::Superseded { by: published }
            };
            let saved = matches!(outcome, Outcome::Saved);
            // The answer is in the slot's channel before the path settles,
            // so a slot woken by `settled` finds it there. A slot that left
            // past its exit wait has dropped the receiver: the file is
            // written all the same, the log line is lost.
            let _ = job.done.send(Published {
                generation,
                rows: job.document.rows(),
                why: job.why,
                outcome,
            });
            {
                let mut state = self.state.lock();
                let path = state
                    .paths
                    .get_mut(job.file.path())
                    .expect("a taken document's path is known");
                if saved {
                    path.published = generation;
                }
                path.settled = path.settled.max(generation);
            }
            self.settled.notify_all();
        }
    }
}

/// How many times the writer thread was started in this process: at most
/// once, whatever slots came and went.
#[cfg(test)]
static THREADS_SPAWNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl SlotHint {
    fn new(file: HintFile) -> Self {
        let (done_tx, done) = mpsc::channel();
        Self {
            file: Arc::new(file),
            done,
            done_tx,
            #[cfg(test)]
            publish: None,
            #[cfg(test)]
            submitted: std::cell::Cell::new(0),
        }
    }

    fn file(&self) -> &HintFile {
        &self.file
    }

    /// Submit `document` to the process's writer under this slot's path,
    /// answered on this slot's channel.
    fn submit(&self, document: PendingSave, why: &'static str) -> bool {
        #[cfg(test)]
        self.submitted.set(document.generation());
        WRITER.submit(Job {
            file: Arc::clone(&self.file),
            document,
            why,
            done: self.done_tx.clone(),
            #[cfg(test)]
            publish: self.publish.clone(),
        })
    }
}

impl SlotBankMemory {
    pub(crate) fn new(memory: SharedBankMemory, hint: Option<HintFile>) -> Self {
        Self {
            memory,
            hint: hint.map(SlotHint::new),
        }
    }

    /// Publish this slot's documents through `publish` instead of
    /// `HintFile::write`.
    #[cfg(test)]
    fn with_publish(mut self, publish: Publish) -> Self {
        self.hint.as_mut().expect("a slot with a hint file").publish = Some(publish);
        self
    }

    /// The shared memory: the observer takes its read guard for the
    /// compiled tick alone; `Play::bank_rows` reads through its own `Arc`.
    pub(crate) fn memory(&self) -> &RwLock<BankMemory> {
        &self.memory
    }

    /// After `publish_snapshot`, every frame (§1.3): apply what the writer
    /// published since the last frame, mirror a loaded bank whose packets
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
        // The file is read and checked with no guard held: a slow disk
        // stalls this slot's boundary, never a UI read.
        let loaded = match hint.file().load() {
            Ok(Some(loaded)) => loaded,
            Ok(None) => return,
            Err(error) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: load rejected path={}: {error}",
                    hint.file().path().display()
                );
                return;
            }
        };
        let applied = {
            let mut memory = self.memory.write();
            // The §1.4 precedence, re-checked under the guard: a memory
            // this process filled meanwhile is kept, the file is dropped.
            if memory.known() {
                return;
            }
            loaded.apply(&mut memory).map(|()| {
                (
                    memory.rows().len(),
                    memory.observed_at().map_or(0, |at| at.unix_secs),
                )
            })
        };
        match applied {
            Ok((rows, observed_at_unix)) => host_log!(
                Category::BankOp,
                Level::Info,
                "bank hints: loaded rows={rows} observed_at_unix={observed_at_unix} path={}",
                hint.file().path().display()
            ),
            Err(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: load rejected path={}: {error}",
                hint.file().path().display()
            ),
        }
    }

    /// The session ended (a logout, a drop, or the slot thread leaving):
    /// queue what the session observed, if anything is pending. The slot
    /// thread's exit ([`ExitSave`]) waits for it to reach the disk.
    pub(crate) fn session_ended(&self) {
        self.save("session end");
    }

    /// Apply every answer the writer sent since the last frame. One
    /// `try_recv` per frame; nothing is allocated when nothing was
    /// answered.
    fn apply_published(&self) {
        let Some(hint) = &self.hint else {
            return;
        };
        while let Ok(published) = hint.done.try_recv() {
            self.apply(hint, published);
        }
    }

    /// Apply one answer: a document written, or superseded on disk by a
    /// newer generation, marks its generation saved under a short write
    /// guard (an observation newer than the document stays pending); a
    /// failure leaves the memory dirty for the next save point.
    fn apply(&self, hint: &SlotHint, published: Published) {
        match published.outcome {
            Outcome::Saved => {
                self.memory.write().mark_saved(published.generation);
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank hints: saved rows={} why={} path={}",
                    published.rows,
                    published.why,
                    hint.file().path().display()
                );
            }
            Outcome::Superseded { by } => {
                self.memory.write().mark_saved(published.generation);
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank hints: superseded generation={} by={by} why={} path={}",
                    published.generation,
                    published.why,
                    hint.file().path().display()
                );
            }
            Outcome::Failed(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: save failed why={} path={}: {error}",
                published.why,
                hint.file().path().display()
            ),
        }
    }

    /// A save point: submit the memory's unsaved rows, if any. The
    /// generation submitted, or `None` when nothing was (nothing pending,
    /// or a failure already logged).
    fn save(&self, why: &'static str) -> Option<u64> {
        let hint = self.hint.as_ref()?;
        // A publication that already finished is applied first, so a clean
        // memory is not queued again.
        self.apply_published();
        // The document is taken under a read guard (a UI read runs beside
        // it) that ends with this block; the publication holds no guard
        // and runs on the writer thread.
        let pending = {
            let memory = self.memory.read();
            hint.file().pending_save(&memory)
        };
        match pending {
            Ok(Some(document)) => {
                let generation = document.generation();
                hint.submit(document, why).then_some(generation)
            }
            Ok(None) => None,
            Err(error) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: save failed why={why} path={}: {error}",
                    hint.file().path().display()
                );
                None
            }
        }
    }

    /// The slot thread is leaving: submit what is pending and wait for the
    /// writer to finish with that generation — or a newer one for the
    /// path, whichever slot sent it — bounded by `timeout`. `true` when
    /// the rows the slot last observed, or newer ones, were on disk before
    /// it left.
    fn finish_within(&self, timeout: Duration) -> bool {
        let Some(generation) = self.save("slot exit") else {
            return true;
        };
        let hint = self.hint.as_ref().expect("a submission came from a hint");
        let settled = WRITER.wait_settled(hint.file().path(), generation, timeout);
        if settled.is_none() {
            host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: slot exit left a save in flight after {}s path={}",
                timeout.as_secs(),
                hint.file().path().display()
            );
        }
        self.apply_published();
        settled == Some(true)
    }
}

/// Saves the slot's pending observations when the slot thread leaves,
/// however it leaves (a Stop while parked at the title never reaches the
/// session boundary), and waits for the writer, bounded by
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
    use api::bank_memory::Origin;
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    use api::ItemDefView;
    use parking_lot::RwLockReadGuard;
    use script::IsolatedEnv;

    const COINS: i32 = 995;
    const BANK_COM: i32 = 5292;

    impl SlotBankMemory {
        /// The test's view of the memory.
        fn read(&self) -> RwLockReadGuard<'_, BankMemory> {
            self.memory.read()
        }

        fn hint_path(&self) -> std::path::PathBuf {
            self.hint.as_ref().unwrap().file().path().to_path_buf()
        }

        /// The generation this slot last submitted.
        fn submitted(&self) -> u64 {
            self.hint.as_ref().unwrap().submitted.get()
        }

        /// Let the writer finish with what this slot last submitted, then
        /// apply its answers: what the slot's next frames would do, without
        /// waiting for them.
        fn settle(&self) {
            assert!(
                WRITER
                    .wait_settled(&self.hint_path(), self.submitted(), Duration::from_secs(10))
                    .is_some(),
                "the writer finished with generation {}",
                self.submitted()
            );
            self.apply_published();
        }
    }

    /// A publication the test holds at the door: each call reports in on
    /// `entered` and waits for one `release` before writing for real.
    struct Gate {
        entered: mpsc::Receiver<u64>,
        release: mpsc::Sender<()>,
    }

    fn gated_publish() -> (Publish, Gate) {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        let publish: Publish = Arc::new(move |hint: &HintFile, document: &PendingSave| {
            entered_tx.send(document.generation()).unwrap();
            // A dropped gate (the test is over) lets the writer go.
            let _ = release_rx.lock().unwrap().recv();
            hint.write(document)
        });
        (publish, Gate { entered, release })
    }

    /// A gated slot over `memory` at the test's pinned `alice` hint path:
    /// what a restart of the slot constructs (`play_slots.rs` keeps the
    /// account's memory and resolves the same file again).
    fn gated_slot_over(memory: &SharedBankMemory) -> (SlotBankMemory, Gate) {
        let hint = HintFile::for_account("local-289", "alice").unwrap();
        let (publish, gate) = gated_publish();
        let slot = SlotBankMemory::new(Arc::clone(memory), Some(hint)).with_publish(publish);
        (slot, gate)
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

    fn slot_with_hint() -> (IsolatedEnv, SlotBankMemory) {
        let scratch = IsolatedEnv::enter("bank-hints");
        let hint = HintFile::for_account("local-289", "alice").unwrap();
        (
            scratch,
            SlotBankMemory::new(Arc::new(RwLock::new(BankMemory::default())), Some(hint)),
        )
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

    /// P2: the close-edge tick returns while the publication is still
    /// stalled on the writer thread, the pump's next frames return too,
    /// the UI read goes through, a newer observation taken meanwhile stays
    /// pending, and the next save point publishes it. The writer is held
    /// until the test releases it, so a pump that waited on the disk would
    /// hang here, not pass.
    #[test]
    fn the_close_edge_tick_returns_while_the_writer_is_stalled() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let memory = Arc::clone(&slot.memory);
        let (publish, gate) = gated_publish();
        let slot = slot.with_publish(publish);
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
        // The writer is inside the publication and stays there until
        // `release`; the close tick has already returned.
        assert_eq!(gate.entered.recv().unwrap(), 1);
        assert!(!path.exists(), "nothing is published yet");
        println!("close-edge tick with the writer stalled: {close_tick:?}");
        assert!(
            close_tick < Duration::from_secs(1),
            "the close tick waited on the writer: {close_tick:?}"
        );
        // The pump goes on: idle closed frames return at once.
        for _ in 0..3 {
            slot.observe_frame(&snapshot);
        }

        // The UI read is available right now: no guard spans the I/O.
        let rows = memory
            .try_read()
            .expect("a reader is never blocked by a save in flight");
        assert_eq!(rows.rows(), &[(COINS, 50)]);
        assert!(rows.dirty());
        drop(rows);
        let ui = play.bank_rows("alice");
        assert_eq!((ui.origin, ui.rows), (Origin::Session, vec![(COINS, 50)]));

        // A newer observation lands while the older document is in flight
        // (the slot thread is the only writer, so only a test can do this):
        // the write guard is free, and the publication must not mark it
        // saved.
        let mut newer = GameSnapshot::new();
        open_bank(&mut newer, 2, 43);
        memory
            .try_write()
            .expect("no writer is blocked by a save in flight")
            .track(&newer, 2);

        gate.release.send(()).unwrap();
        slot.settle();
        let raw = rows_on_disk(&path);
        assert!(
            raw.contains(&format!("\"rows\":[[{COINS},50]]")),
            "the document taken under the guard was published: {raw}"
        );
        assert!(
            slot.read().dirty(),
            "the newer observation is still pending"
        );
        assert_eq!(slot.read().rows(), &[(COINS, 43)]);

        // The next save point publishes the newer rows.
        slot.session_ended();
        assert_eq!(gate.entered.recv().unwrap(), 2);
        gate.release.send(()).unwrap();
        slot.settle();
        assert!(!slot.read().dirty());
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},43]]")), "{raw}");
    }

    /// P2: two quick closes publish in generation order, and a close that
    /// lands while an older document is still queued (not yet taken by the
    /// writer) supersedes it: the older rows are never written.
    #[test]
    fn quick_closes_publish_in_order_and_a_queued_document_is_superseded() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let (publish, gate) = gated_publish();
        let slot = slot.with_publish(publish);
        let mut snapshot = GameSnapshot::new();
        let mut open_and_close = |generation: u64, coins: i32| {
            open_bank(&mut snapshot, generation, coins);
            slot.observe_frame(&snapshot);
            close_bank(&mut snapshot, generation);
            slot.observe_frame(&snapshot);
        };

        // Generation 1 is taken by the writer and held at the door.
        open_and_close(1, 50);
        assert_eq!(gate.entered.recv().unwrap(), 1);
        // Generation 2 closes while 1 is in flight: queued behind it.
        open_and_close(2, 43);
        assert!(!path.exists(), "nothing is published yet");
        gate.release.send(()).unwrap();
        // The writer takes 2 next, in order, and is held again.
        assert_eq!(gate.entered.recv().unwrap(), 2);
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        // Generations 3 and 4 close while 2 is in flight: 4 replaces the
        // queued 3, which is never written.
        open_and_close(3, 7);
        open_and_close(4, 9);
        gate.release.send(()).unwrap();
        assert_eq!(gate.entered.recv().unwrap(), 4);
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},43]]")));
        gate.release.send(()).unwrap();
        slot.settle();
        assert!(
            gate.entered.try_recv().is_err(),
            "generation 3 was superseded, not published"
        );
        assert!(!slot.read().dirty(), "the newest generation was published");
        assert_eq!(slot.read().generation(), 4);
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},9]]")), "{raw}");
    }

    /// P2: the slot thread's exit waits for the writer, so the rows the
    /// session observed last are on disk, behind the close's document
    /// already in flight, when the thread is joined.
    #[test]
    fn the_exit_save_reaches_the_disk_before_the_slot_leaves() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let (entered_tx, entered) = mpsc::channel();
        let slot = slot.with_publish(Arc::new(move |hint: &HintFile, document: &PendingSave| {
            entered_tx.send(document.generation()).unwrap();
            // A slow disk.
            thread::sleep(Duration::from_millis(100));
            hint.write(document)
        }));
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        // The close's document is in flight (taken, not merely queued, so
        // the exit's document queues behind it rather than replacing it).
        assert_eq!(entered.recv().unwrap(), 1);
        // The bank is open again when the slot leaves: the exit save
        // carries these rows.
        open_bank(&mut snapshot, 2, 43);
        slot.observe_frame(&snapshot);
        assert!(slot.read().dirty());

        let left = Instant::now();
        drop(ExitSave(&slot));
        let waited = left.elapsed();
        assert_eq!(
            entered.recv().unwrap(),
            2,
            "the exit's document was published after the close's"
        );
        assert!(entered.try_recv().is_err(), "nothing else was published");
        assert!(
            waited >= Duration::from_millis(100),
            "the exit waited for its own publication: {waited:?}"
        );
        assert!(
            !slot.read().dirty(),
            "the exit save was published and applied before the slot left"
        );
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},43]]")), "{raw}");
    }

    /// P2: a writer stalled past the exit bound does not hold the slot
    /// thread; the slot leaves with the save still in flight (and one
    /// hostlog line), and the writer finishes on its own. The exit's
    /// document carries the generation already in flight, so once that one
    /// is on disk the writer drops the exit's as stale instead of writing
    /// it again.
    #[test]
    fn the_exit_wait_is_bounded_when_the_writer_stalls() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let (publish, gate) = gated_publish();
        let slot = slot.with_publish(publish);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv().unwrap(), 1);

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

        // The writer finishes once the disk answers: the stalled close
        // document is written, the exit's one (the same generation) is
        // superseded by it, and the answers clear the memory.
        gate.release.send(()).unwrap();
        slot.settle();
        assert!(!slot.read().dirty());
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        assert!(
            gate.entered.try_recv().is_err(),
            "the exit's stale document was never written"
        );
    }

    /// A failed publication leaves the rows pending and the memory readable.
    #[test]
    fn a_failed_save_keeps_the_rows_pending_for_the_next_save_point() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let slot = slot.with_publish(Arc::new(|_, _| {
            Err(HintError::Io(std::io::Error::other("disk full")))
        }));
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        slot.settle();
        assert!(
            slot.read().dirty(),
            "a failed write leaves the memory dirty"
        );
        assert!(!path.exists());
        // Nothing is left locked behind the failure.
        assert!(slot.memory().try_write().is_some());
        // The next save point tries again.
        slot.session_ended();
        slot.settle();
        assert!(slot.read().dirty());
        assert!(!path.exists());
    }

    #[test]
    fn fixture_open_withdraw_close_saves_once_into_the_pinned_home() {
        let (scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        assert!(path.starts_with(&scratch.home));
        let mut snapshot = GameSnapshot::new();
        slot.session_started();
        assert!(!slot.read().known(), "no file yet: Unknown");

        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().origin(), Origin::Session);
        assert_eq!(slot.read().rows(), &[(COINS, 50)]);
        assert!(!path.exists(), "no save while the bank is open");

        // The withdraw moves the live rows: the memory follows.
        open_bank(&mut snapshot, 2, 43);
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().rows(), &[(COINS, 43)]);

        close_bank(&mut snapshot, 2);
        slot.observe_frame(&snapshot);
        slot.settle();
        assert!(!slot.read().dirty(), "the close saved");
        let raw = rows_on_disk(&path);
        assert!(raw.contains(&format!("\"rows\":[[{COINS},43]]")), "{raw}");
        // Anything written after the close would replace this sentinel.
        std::fs::write(&path, "sentinel").unwrap();

        // Idle closed frames and the session end write nothing more.
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
        let (_scratch, slot) = slot_with_hint();
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
        // The close's publication is applied; the frames after it see an
        // empty hand-back channel, which is the steady state.
        slot.settle();
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
        let (_scratch, slot) = slot_with_hint();
        let hint = slot.hint.as_ref().unwrap().file().clone();
        // A file on disk the slot did not write this session.
        let mut on_disk = BankMemory::default();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 7, 0)]), Vec::new());
        on_disk.track(&snapshot, 1);
        hint.write(&hint.pending_save(&on_disk).unwrap().unwrap())
            .unwrap();

        // Unknown at the boundary: the file is loaded, origin Hint.
        slot.session_started();
        assert_eq!(slot.read().origin(), Origin::Hint);
        assert_eq!(slot.read().rows(), &[(COINS, 7)]);

        // This session sees different rows; a relog keeps them and never
        // re-reads the file.
        snapshot.seed_bank_observation(BANK_COM, 2, Some(vec![bank_row(COINS, 99, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().origin(), Origin::Session);
        let generation = slot.read().generation();
        slot.session_started();
        assert_eq!(slot.read().origin(), Origin::Hint);
        assert_eq!(slot.read().rows(), &[(COINS, 99)]);
        assert!(slot.read().generation() > generation);
        assert!(slot.read().dirty(), "the unsaved session is still pending");
        slot.session_ended();
        slot.settle();
        assert!(!slot.read().dirty());
        let raw = std::fs::read_to_string(hint.path()).unwrap();
        assert!(raw.contains(&format!("[[{COINS},99]]")), "{raw}");
    }

    #[test]
    fn a_rejected_file_leaves_the_memory_unknown_and_login_unaffected() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
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

    /// M1-R3: a name the server would refuse at login (raw length over
    /// twelve) gets no hint file at all; the slot's memory starts Unknown
    /// and stays so across the boundary, and no file appears under the
    /// pinned home for the session's observations.
    #[test]
    fn an_invalid_login_name_gets_no_hint_and_stays_unknown() {
        let scratch = IsolatedEnv::enter("bank-hints");
        let hint = HintFile::for_account("local-289", "ab3456789xyz1");
        assert!(matches!(&hint, Err(HintError::UnsafeAccount(name)) if name == "ab3456789xyz1"));
        // What `play_slots.rs` constructs after its one warning line.
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

    /// H1-R3: the reviewer's restart-after-timeout case. Three slots in a
    /// row leave past their exit bound with a save queued (the first one
    /// in flight, held at the door); a successor over the same account
    /// memory publishes generation N; the departed slots' documents never
    /// reach the file (the in-flight one lands first, in order; the queued
    /// ones are replaced), a late older document can never put the file
    /// below N, and no writer thread was started per slot.
    #[test]
    fn restarts_after_a_timed_out_exit_keep_order_and_one_writer_thread() {
        let _scratch = IsolatedEnv::enter("bank-hints");
        let memory: SharedBankMemory = Arc::default();
        let mut snapshot = GameSnapshot::new();

        // Slot 1 closes the bank (generation 1, taken and held) and leaves
        // past a 100 ms bound with that save in flight.
        let (slot1, gate1) = gated_slot_over(&memory);
        let path = slot1.hint_path();
        open_bank(&mut snapshot, 1, 50);
        slot1.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot1.observe_frame(&snapshot);
        assert_eq!(gate1.entered.recv().unwrap(), 1);
        assert!(!slot1.finish_within(Duration::from_millis(100)));
        assert_eq!(slot1.submitted(), 1);
        drop(slot1);

        // Slots 2 and 3 restart over the same memory (a relog bumps the
        // generation and keeps the unsaved rows), each leaving past its
        // bound with its exit document queued; 3's replaces 2's.
        let (slot2, gate2) = gated_slot_over(&memory);
        slot2.session_started();
        assert!(!slot2.finish_within(Duration::from_millis(100)));
        assert_eq!(slot2.submitted(), 2);
        drop(slot2);
        let (slot3, gate3) = gated_slot_over(&memory);
        slot3.session_started();
        assert!(!slot3.finish_within(Duration::from_millis(100)));
        assert_eq!(slot3.submitted(), 3);
        drop(slot3);
        assert!(!path.exists(), "the first publication is still held");

        // The successor observes new rows and closes: generation 5 (4 was
        // its relog) replaces the queued 3.
        let (successor, gate_n) = gated_slot_over(&memory);
        successor.session_started();
        open_bank(&mut snapshot, 2, 43);
        successor.observe_frame(&snapshot);
        close_bank(&mut snapshot, 2);
        successor.observe_frame(&snapshot);
        let n = successor.submitted();
        assert_eq!(n, 5);

        // The disk answers: slot 1's document lands first (it was in
        // flight), then the newest queued one, N; nothing in between.
        gate1.release.send(()).unwrap();
        assert_eq!(gate_n.entered.recv().unwrap(), n);
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        gate_n.release.send(()).unwrap();
        successor.settle();
        assert!(!successor.read().dirty(), "N was published and applied");
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},43]]")));
        assert!(
            gate2.entered.try_recv().is_err(),
            "slot 2's document was replaced"
        );
        assert!(
            gate3.entered.try_recv().is_err(),
            "slot 3's document was replaced"
        );
        assert!(gate1.entered.try_recv().is_err(), "slot 1 published once");

        // A late older document for the same path — whichever slot sends
        // it — is dropped as stale, never written over N, and its sender
        // is told so.
        let older: SharedBankMemory = Arc::default();
        let mut older_snapshot = GameSnapshot::new();
        open_bank(&mut older_snapshot, 1, 7);
        older.write().track(&older_snapshot, 1);
        open_bank(&mut older_snapshot, 2, 9);
        older.write().track(&older_snapshot, 1);
        assert_eq!(older.read().generation(), 2);
        let (late, late_gate) = gated_slot_over(&older);
        late.session_ended();
        assert_eq!(late.submitted(), 2);
        // An exit wait on that generation would end at once: N, newer, is
        // on disk for the path.
        assert_eq!(WRITER.wait_settled(&path, 2, Duration::ZERO), Some(true));
        // The writer takes the stale document, writes nothing, and answers
        // its sender (which may already have left; this one is still here).
        let hint = late.hint.as_ref().unwrap();
        let answer = hint.done.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(answer.generation, 2);
        assert!(matches!(answer.outcome, Outcome::Superseded { by } if by == n));
        late.apply(hint, answer);
        assert!(
            late_gate.entered.try_recv().is_err(),
            "the stale document was never written"
        );
        assert!(
            rows_on_disk(&path).contains(&format!("[[{COINS},43]]")),
            "the file stays at N"
        );
        assert!(!late.read().dirty(), "the sender was answered: superseded");

        assert_eq!(
            THREADS_SPAWNED.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one writer thread for the process, however many slots came and went"
        );
    }

    /// H1-R3: a burst of save points while the writer is held queues one
    /// document, the newest; releasing the writer publishes exactly that
    /// one after the held one, in order, and starts no further thread.
    #[test]
    fn a_burst_of_closes_publishes_the_newest_once_in_order() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let (publish, gate) = gated_publish();
        let slot = slot.with_publish(publish);
        let mut snapshot = GameSnapshot::new();
        open_bank(&mut snapshot, 1, 50);
        slot.observe_frame(&snapshot);
        close_bank(&mut snapshot, 1);
        slot.observe_frame(&snapshot);
        assert_eq!(gate.entered.recv().unwrap(), 1);

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
        let newest = slot.submitted();
        assert_eq!(newest, burst + 1);
        assert!(
            !path.exists(),
            "nothing is published while the writer is held"
        );

        gate.release.send(()).unwrap();
        assert_eq!(
            gate.entered.recv().unwrap(),
            newest,
            "the newest, right after the held one"
        );
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
        gate.release.send(()).unwrap();
        slot.settle();
        assert!(
            gate.entered.try_recv().is_err(),
            "the other {} were superseded",
            burst - 1
        );
        assert!(!slot.read().dirty());
        assert_eq!(slot.read().generation(), newest);
        let raw = rows_on_disk(&path);
        assert!(
            raw.contains(&format!("\"rows\":[[{COINS},{}]]", burst + 1)),
            "{raw}"
        );
        assert_eq!(THREADS_SPAWNED.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn per_bot_memory_is_a_header_plus_one_reserved_table() {
        use std::mem::size_of;
        let memory = size_of::<BankMemory>();
        let cell = size_of::<RwLock<BankMemory>>();
        let hint = size_of::<HintFile>();
        let slot_hint = size_of::<SlotHint>();
        let slot = size_of::<SlotBankMemory>();
        println!(
            "per-bot bank memory: BankMemory={memory}B RwLock<BankMemory>={cell}B (Arc inner +16B) \
             HintFile={hint}B (Arc'd) SlotHint={slot_hint}B SlotBankMemory={slot}B; rows heap = \
             8B x bank_size once loaded; one bank-hint-writer thread per process plus one \
             map entry per hint path"
        );
        assert!(memory <= 96, "BankMemory is {memory} bytes");
        assert!(slot <= 128, "SlotBankMemory is {slot} bytes");
    }
}
