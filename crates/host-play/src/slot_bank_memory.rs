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
//! file I/O**: a load reads and checks the file first and applies the rows
//! under a short re-lock that re-checks the §1.4 precedence; a save takes
//! its document under a read guard, publishes with no guard held, then marks
//! the published generation saved. `Play::bank_rows` therefore never waits
//! on the disk. The compiled script tick borrows the memory through a read
//! guard the observer holds for the tick alone (`script_observe.rs`).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use api::bank_memory::{BankMemory, FrameEvent};
use api::host_log;
use api::hostlog::{Category, Level};
use api::snapshot::GameSnapshot;
use parking_lot::RwLock;
use script::bank_hints::{HintError, HintFile, PendingSave};

/// One account's memory, shared by `Play` (UI reads) and its slot thread.
pub(crate) type SharedBankMemory = Arc<RwLock<BankMemory>>;

/// A test's stand-in for `HintFile::write`: the publication step, run with
/// no guard held, which a test can stall to probe the lock meanwhile.
#[cfg(test)]
type Writer = Box<dyn Fn(&HintFile, &PendingSave) -> Result<(), HintError> + Send + Sync>;

pub(crate) struct SlotBankMemory {
    memory: SharedBankMemory,
    /// `None` when the account name is not a safe path component or the
    /// connection has no launch profile: the memory then lives for the
    /// process only.
    hint: Option<HintFile>,
    #[cfg(test)]
    writer: Option<Writer>,
}

impl SlotBankMemory {
    pub(crate) fn new(memory: SharedBankMemory, hint: Option<HintFile>) -> Self {
        Self {
            memory,
            hint,
            #[cfg(test)]
            writer: None,
        }
    }

    /// Publish through `writer` instead of `HintFile::write`.
    #[cfg(test)]
    fn with_writer(mut self, writer: Writer) -> Self {
        self.writer = Some(writer);
        self
    }

    /// The shared memory: the observer takes its read guard for the
    /// compiled tick alone; `Play::bank_rows` reads through its own `Arc`.
    pub(crate) fn memory(&self) -> &RwLock<BankMemory> {
        &self.memory
    }

    /// After `publish_snapshot`, every frame (§1.3): mirror a loaded bank
    /// whose packets moved, and save the hint on the close edge.
    pub(crate) fn observe_frame(&self, snapshot: &GameSnapshot) {
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
        let loaded = match hint.load() {
            Ok(Some(loaded)) => loaded,
            Ok(None) => return,
            Err(error) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: load rejected path={}: {error}",
                    hint.path().display()
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
                hint.path().display()
            ),
            Err(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: load rejected path={}: {error}",
                hint.path().display()
            ),
        }
    }

    /// The session ended (a logout, a drop, or the slot thread leaving):
    /// save what the session observed, if anything is pending.
    pub(crate) fn session_ended(&self) {
        self.save("session end");
    }

    fn save(&self, why: &str) {
        let Some(hint) = &self.hint else {
            return;
        };
        // The document is taken under a read guard (a UI read runs beside
        // it) that ends with this block; the publication below holds no
        // guard at all.
        let pending = {
            let memory = self.memory.read();
            hint.pending_save(&memory)
        };
        let pending = match pending {
            Ok(Some(pending)) => pending,
            Ok(None) => return,
            Err(error) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    "bank hints: save failed why={why} path={}: {error}",
                    hint.path().display()
                );
                return;
            }
        };
        match self.write(hint, &pending) {
            Ok(()) => {
                // A short write marks the published generation saved; an
                // observation newer than the document stays pending.
                self.memory.write().mark_saved(pending.generation());
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank hints: saved rows={} why={why} path={}",
                    pending.rows(),
                    hint.path().display()
                );
            }
            Err(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: save failed why={why} path={}: {error}",
                hint.path().display()
            ),
        }
    }

    #[cfg(not(test))]
    fn write(&self, hint: &HintFile, pending: &PendingSave) -> Result<(), HintError> {
        hint.write(pending)
    }

    #[cfg(test)]
    fn write(&self, hint: &HintFile, pending: &PendingSave) -> Result<(), HintError> {
        match &self.writer {
            Some(writer) => writer(hint, pending),
            None => hint.write(pending),
        }
    }
}

/// Saves the slot's pending observations when the slot thread leaves,
/// however it leaves (a Stop while parked at the title never reaches the
/// session boundary).
pub(crate) struct ExitSave<'a>(pub(crate) &'a SlotBankMemory);

impl Drop for ExitSave<'_> {
    fn drop(&mut self) {
        self.0.save("slot exit");
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
    use std::sync::mpsc;
    use std::thread;

    const COINS: i32 = 995;
    const BANK_COM: i32 = 5292;

    impl SlotBankMemory {
        /// The test's view of the memory.
        fn read(&self) -> RwLockReadGuard<'_, BankMemory> {
            self.memory.read()
        }
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

    /// H1: the close save's publication is stalled on another thread; the
    /// UI read goes through at once, a newer observation taken meanwhile
    /// stays pending, and the next save point publishes it.
    #[test]
    fn a_stalled_save_blocks_no_reader_and_keeps_a_newer_observation_pending() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint.as_ref().unwrap().path().to_path_buf();
        let memory = Arc::clone(&slot.memory);
        let (entered_tx, entered) = mpsc::channel();
        let (release_tx, release) = mpsc::channel::<()>();
        let release = std::sync::Mutex::new(release);
        let slot = slot.with_writer(Box::new(move |hint: &HintFile, pending: &PendingSave| {
            // A slow disk: the publication waits until the test lets it go.
            entered_tx.send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
            hint.write(pending)
        }));
        let mut play = offline_play();
        play.bank_memories
            .insert("alice".into(), Arc::clone(&memory));

        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        // The close edge saves: the slot thread is inside the publication
        // once `entered` fires, and stays there until `release`.
        let saver = thread::spawn(move || {
            slot.observe_frame(&snapshot);
            slot
        });
        entered.recv().unwrap();
        assert!(!path.exists(), "nothing is published yet");

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
        // the write guard is free, and the save must not mark it saved.
        let mut newer = GameSnapshot::new();
        newer.seed_bank_observation(BANK_COM, 2, Some(vec![bank_row(COINS, 43, 0)]), Vec::new());
        memory
            .try_write()
            .expect("no writer is blocked by a save in flight")
            .track(&newer, 2);

        release_tx.send(()).unwrap();
        let slot = saver.join().unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
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
        release_tx.send(()).unwrap();
        slot.session_ended();
        entered.recv().unwrap();
        assert!(!slot.read().dirty());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(&format!("\"rows\":[[{COINS},43]]")), "{raw}");
    }

    /// A failed publication leaves the rows pending and the memory readable.
    #[test]
    fn a_failed_save_keeps_the_rows_pending_for_the_next_save_point() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint.as_ref().unwrap().path().to_path_buf();
        let slot = slot.with_writer(Box::new(|_, _| {
            Err(HintError::Io(std::io::Error::other("disk full")))
        }));
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        slot.observe_frame(&snapshot);
        assert!(
            slot.read().dirty(),
            "a failed write leaves the memory dirty"
        );
        assert!(!path.exists());
        // Nothing is left locked behind the failure.
        assert!(slot.memory().try_write().is_some());
    }

    #[test]
    fn fixture_open_withdraw_close_saves_once_into_the_pinned_home() {
        let (scratch, slot) = slot_with_hint();
        let path = slot.hint.as_ref().unwrap().path().to_path_buf();
        assert!(path.starts_with(&scratch.home));
        let mut snapshot = GameSnapshot::new();
        slot.session_started();
        assert!(!slot.read().known(), "no file yet: Unknown");

        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().origin(), Origin::Session);
        assert_eq!(slot.read().rows(), &[(COINS, 50)]);
        assert!(!path.exists(), "no save while the bank is open");

        // The withdraw moves the live rows: the memory follows.
        snapshot.seed_bank_observation(BANK_COM, 2, Some(vec![bank_row(COINS, 43, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        assert_eq!(slot.read().rows(), &[(COINS, 43)]);

        snapshot.seed_bank_observation(-1, 2, None, Vec::new());
        slot.observe_frame(&snapshot);
        assert!(!slot.read().dirty(), "the close saved");
        let raw = std::fs::read_to_string(&path).unwrap();
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
            std::fs::read_to_string(&path).unwrap(),
            "sentinel",
            "exactly one save per close"
        );
    }

    #[test]
    fn idle_frames_take_no_write_and_allocate_nothing() {
        let (_scratch, slot) = slot_with_hint();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_bank_observation(BANK_COM, 1, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        slot.observe_frame(&snapshot);
        let open = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                slot.observe_frame(&snapshot);
            }
        });
        assert_eq!(open.bytes_total, 0, "an open idle frame allocates nothing");
        snapshot.seed_bank_observation(-1, 1, None, Vec::new());
        slot.observe_frame(&snapshot);
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
        let hint = slot.hint.clone().unwrap();
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
        assert!(!slot.read().dirty());
        let raw = std::fs::read_to_string(hint.path()).unwrap();
        assert!(raw.contains(&format!("[[{COINS},99]]")), "{raw}");
    }

    #[test]
    fn a_rejected_file_leaves_the_memory_unknown_and_login_unaffected() {
        let (_scratch, slot) = slot_with_hint();
        let path = slot.hint.as_ref().unwrap().path().to_path_buf();
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
        let slot = size_of::<SlotBankMemory>();
        println!(
            "per-bot bank memory: BankMemory={memory}B RwLock<BankMemory>={cell}B (Arc inner +16B) \
             HintFile={hint}B SlotBankMemory={slot}B (test builds add the 16B writer seam); \
             rows heap = 8B x bank_size once loaded"
        );
        assert!(memory <= 96, "BankMemory is {memory} bytes");
        assert!(slot <= 128, "SlotBankMemory is {slot} bytes");
    }
}
