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
//! guard and hands it to the account's writer thread ([`HintWriter`]),
//! which publishes it while the pump goes on with the next frame. The
//! writer hands the result back and the slot thread marks the published
//! generation saved on its next frame. `Play::bank_rows` therefore never
//! waits on the disk, and neither does the owning bot's frame. The compiled
//! script tick borrows the memory through a read guard the observer holds
//! for the tick alone (`script_observe.rs`).

use std::sync::mpsc;
use std::sync::Arc;
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

/// How long a leaving slot waits for its writer to publish what is queued
/// (§1.4: slot exit is a save point, and `Play` joins the slot thread
/// before the process exits). Past it the slot leaves with one hostlog
/// line; the writer finishes on its own unless the process exits first.
const EXIT_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A test's stand-in for `HintFile::write`: the publication step, run on
/// the writer thread with no guard held, which a test can stall or fail.
#[cfg(test)]
type Publish = Box<dyn Fn(&HintFile, &PendingSave) -> Result<(), HintError> + Send + Sync>;

pub(crate) struct SlotBankMemory {
    memory: SharedBankMemory,
    /// `None` when the account name is not a login name or the connection
    /// has no launch profile: the memory then lives for the process only.
    hint: Option<HintWriter>,
}

/// One account's off-pump hint publication. A save point queues the
/// document and a `bank-hint-<account>` thread — spawned on demand and
/// gone once the queue is drained, the `nav-find`/`bank-pick` worker
/// shape — writes it (atomic replace, fsync). A newer document replaces a
/// queued older one, so at most one write ever waits behind the one in
/// flight and the file only ever receives rows in generation order. Each
/// publication is handed back through `done`; the slot thread, the
/// memory's only writer, applies [`BankMemory::mark_saved`] and logs it.
struct HintWriter {
    shared: Arc<WriterShared>,
    done: mpsc::Receiver<Published>,
    done_tx: mpsc::Sender<Published>,
}

/// What the slot thread and its writer thread share.
struct WriterShared {
    file: HintFile,
    queue: Mutex<Queue>,
    /// Signalled when the writer thread leaves: nothing is queued or in
    /// flight.
    idle: Condvar,
    #[cfg(test)]
    publish: Option<Publish>,
}

#[derive(Default)]
struct Queue {
    /// The newest document not yet taken by the writer thread.
    pending: Option<Job>,
    /// A writer thread is draining `pending`. Set when one is spawned and
    /// cleared by that thread, under the lock, only once `pending` is
    /// empty, so `pending.is_some()` implies `busy`.
    busy: bool,
}

struct Job {
    document: PendingSave,
    why: &'static str,
}

/// One publication's outcome, handed back to the slot thread.
struct Published {
    generation: u64,
    rows: usize,
    why: &'static str,
    result: Result<(), HintError>,
}

impl HintWriter {
    fn new(file: HintFile) -> Self {
        let (done_tx, done) = mpsc::channel();
        Self {
            shared: Arc::new(WriterShared {
                file,
                queue: Mutex::default(),
                idle: Condvar::new(),
                #[cfg(test)]
                publish: None,
            }),
            done,
            done_tx,
        }
    }

    fn file(&self) -> &HintFile {
        &self.shared.file
    }

    /// Queue `document` for publication, replacing a queued older one, and
    /// start the writer thread if none is draining the queue. A spawn
    /// failure drops the document: the memory stays dirty for the next
    /// save point.
    fn queue(&self, document: PendingSave, why: &'static str) {
        let mut queue = self.shared.queue.lock();
        queue.pending = Some(Job { document, why });
        if queue.busy {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let done = self.done_tx.clone();
        let spawned = thread::Builder::new()
            .name(format!("bank-hint-{}", self.shared.file.account()))
            .spawn(move || shared.drain(&done));
        match spawned {
            Ok(_) => queue.busy = true,
            Err(error) => {
                queue.pending = None;
                drop(queue);
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: save failed why={why} path={}: writer thread: {error}",
                    self.shared.file.path().display()
                );
            }
        }
    }

    /// Wait until nothing is queued or in flight, or `timeout` passes;
    /// `true` when the writer is idle.
    fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut queue = self.shared.queue.lock();
        while queue.busy {
            if self
                .shared
                .idle
                .wait_until(&mut queue, deadline)
                .timed_out()
            {
                break;
            }
        }
        !queue.busy
    }
}

impl WriterShared {
    /// The writer thread: publish the queued document until none is left,
    /// then leave. The outcome goes back before the next document is
    /// taken, so once `busy` clears every outcome has been sent.
    fn drain(&self, done: &mpsc::Sender<Published>) {
        loop {
            let job = {
                let mut queue = self.queue.lock();
                match queue.pending.take() {
                    Some(job) => job,
                    None => {
                        queue.busy = false;
                        self.idle.notify_all();
                        return;
                    }
                }
            };
            let result = self.publish(&job.document);
            // A slot that left past its exit wait has dropped the receiver:
            // the file is written all the same, the log line is lost.
            let _ = done.send(Published {
                generation: job.document.generation(),
                rows: job.document.rows(),
                why: job.why,
                result,
            });
        }
    }

    #[cfg(not(test))]
    fn publish(&self, document: &PendingSave) -> Result<(), HintError> {
        self.file.write(document)
    }

    #[cfg(test)]
    fn publish(&self, document: &PendingSave) -> Result<(), HintError> {
        match &self.publish {
            Some(publish) => publish(&self.file, document),
            None => self.file.write(document),
        }
    }
}

impl SlotBankMemory {
    pub(crate) fn new(memory: SharedBankMemory, hint: Option<HintFile>) -> Self {
        Self {
            memory,
            hint: hint.map(HintWriter::new),
        }
    }

    /// Publish through `publish` instead of `HintFile::write`.
    #[cfg(test)]
    fn with_publish(mut self, publish: Publish) -> Self {
        let hint = self.hint.as_mut().expect("a slot with a hint file");
        Arc::get_mut(&mut hint.shared)
            .expect("no writer thread has run yet")
            .publish = Some(publish);
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
            FrameEvent::Closed => self.save("close"),
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

    /// Apply every publication the writer finished: a short write marks the
    /// published generation saved (an observation newer than the document
    /// stays pending), a failure leaves the memory dirty for the next save
    /// point. One `try_recv` per frame; nothing is allocated when nothing
    /// was published.
    fn apply_published(&self) {
        let Some(hint) = &self.hint else {
            return;
        };
        while let Ok(published) = hint.done.try_recv() {
            match published.result {
                Ok(()) => {
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
                Err(error) => host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: save failed why={} path={}: {error}",
                    published.why,
                    hint.file().path().display()
                ),
            }
        }
    }

    fn save(&self, why: &'static str) {
        let Some(hint) = &self.hint else {
            return;
        };
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
            Ok(Some(document)) => hint.queue(document, why),
            Ok(None) => {}
            Err(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: save failed why={why} path={}: {error}",
                hint.file().path().display()
            ),
        }
    }

    /// The slot thread is leaving: queue what is pending and wait for the
    /// writer to publish everything queued, bounded by `timeout`. `true`
    /// when every save reached the disk before the slot left.
    fn finish_within(&self, timeout: Duration) -> bool {
        self.save("slot exit");
        let Some(hint) = &self.hint else {
            return true;
        };
        let idle = hint.wait_idle(timeout);
        if !idle {
            host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: slot exit left a save in flight after {}s path={}",
                timeout.as_secs(),
                hint.file().path().display()
            );
        }
        self.apply_published();
        idle
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

        /// Let the writer drain, then apply what it published: what the
        /// slot's next frames would do, without waiting for them.
        fn settle(&self) {
            assert!(
                self.hint
                    .as_ref()
                    .unwrap()
                    .wait_idle(Duration::from_secs(10)),
                "the writer drained"
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
        let publish: Publish = Box::new(move |hint: &HintFile, document: &PendingSave| {
            entered_tx.send(document.generation()).unwrap();
            // A dropped gate (the test is over) lets the writer go.
            let _ = release_rx.lock().unwrap().recv();
            hint.write(document)
        });
        (publish, Gate { entered, release })
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
        let slot = slot.with_publish(Box::new(move |hint: &HintFile, document: &PendingSave| {
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
    /// hostlog line), and the writer finishes on its own.
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

        // The detached writer finishes once the disk answers: the stalled
        // close document, then the one the exit queued behind it (the same
        // generation, still unacknowledged when the exit took it).
        gate.release.send(()).unwrap();
        assert_eq!(gate.entered.recv().unwrap(), 1);
        gate.release.send(()).unwrap();
        slot.settle();
        assert!(!slot.read().dirty());
        assert!(rows_on_disk(&path).contains(&format!("[[{COINS},50]]")));
    }

    /// A failed publication leaves the rows pending and the memory readable.
    #[test]
    fn a_failed_save_keeps_the_rows_pending_for_the_next_save_point() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint_path();
        let slot = slot.with_publish(Box::new(|_, _| {
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

    #[test]
    fn per_bot_memory_is_a_header_plus_one_reserved_table() {
        use std::mem::size_of;
        let memory = size_of::<BankMemory>();
        let cell = size_of::<RwLock<BankMemory>>();
        let hint = size_of::<HintFile>();
        let writer = size_of::<HintWriter>();
        let slot = size_of::<SlotBankMemory>();
        println!(
            "per-bot bank memory: BankMemory={memory}B RwLock<BankMemory>={cell}B (Arc inner +16B) \
             HintFile={hint}B HintWriter={writer}B (+ its Arc'd shared part holding the \
             HintFile, queue and condvar) SlotBankMemory={slot}B; rows heap = 8B x bank_size \
             once loaded; no writer thread while nothing is queued"
        );
        assert!(memory <= 96, "BankMemory is {memory} bytes");
        assert!(slot <= 128, "SlotBankMemory is {slot} bytes");
    }
}
