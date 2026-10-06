//! The slot thread's hand on its account's bank memory (design-bank-snapshot
//! §1.2–§1.4).
//!
//! `Play` owns one `Arc<RwLock<BankMemory>>` per account; the slot thread is
//! the only writer. The hint file is resolved once, on the spawning thread,
//! and handed in beside the `Arc`, so the slot thread never resolves `HOME`
//! and a test's thread-local `IsolatedEnv` pin covers every save.
//!
//! Lock discipline: an idle frame takes one read guard to see nothing is due;
//! a write guard is taken only on the frames the open bank moved or closed,
//! at the session boundary and for a save. The compiled script tick borrows
//! the memory through a read guard the frame closure holds across the tick.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use api::bank_memory::{BankMemory, FrameEvent};
use api::host_log;
use api::hostlog::{Category, Level};
use api::snapshot::GameSnapshot;
use parking_lot::{RwLock, RwLockReadGuard};
use script::bank_hints::HintFile;

/// One account's memory, shared by `Play` (UI reads) and its slot thread.
pub(crate) type SharedBankMemory = Arc<RwLock<BankMemory>>;

pub(crate) struct SlotBankMemory {
    memory: SharedBankMemory,
    /// `None` when the account name is not a safe path component or the
    /// connection has no launch profile: the memory then lives for the
    /// process only.
    hint: Option<HintFile>,
}

impl SlotBankMemory {
    pub(crate) fn new(memory: SharedBankMemory, hint: Option<HintFile>) -> Self {
        Self { memory, hint }
    }

    /// The borrow a frame hands the compiled script.
    pub(crate) fn read(&self) -> RwLockReadGuard<'_, BankMemory> {
        self.memory.read()
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
                let memory = self.memory.read();
                host_log!(
                    Category::BankOp,
                    Level::Info,
                    "bank memory observed rows={} size={} generation={}",
                    memory.rows().len(),
                    snapshot.bank_size(),
                    memory.generation()
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
        let mut memory = self.memory.write();
        if memory.known() {
            memory.relogged();
            host_log!(
                Category::BankOp,
                Level::Info,
                "bank memory relog origin={:?} rows={}",
                memory.origin(),
                memory.rows().len()
            );
            return;
        }
        let Some(hint) = &self.hint else {
            return;
        };
        match hint.load_into(&mut memory) {
            Ok(true) => host_log!(
                Category::BankOp,
                Level::Info,
                "bank hints: loaded rows={} observed_at_unix={} path={}",
                memory.rows().len(),
                memory.observed_at().map_or(0, |at| at.unix_secs),
                hint.path().display()
            ),
            Ok(false) => {}
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
        let mut memory = self.memory.write();
        match hint.save_if_dirty(&mut memory) {
            Ok(true) => host_log!(
                Category::BankOp,
                Level::Info,
                "bank hints: saved rows={} why={why} path={}",
                memory.rows().len(),
                hint.path().display()
            ),
            Ok(false) => {}
            Err(error) => host_log!(
                Category::BankOp,
                Level::Warn,
                "bank hints: save failed why={why} path={}: {error}",
                hint.path().display()
            ),
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
    use script::IsolatedEnv;

    const COINS: i32 = 995;
    const BANK_COM: i32 = 5292;

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
        hint.save_if_dirty(&mut on_disk).unwrap();

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
        let mut play = crate::run_with_io(
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
        );
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
             HintFile={hint}B SlotBankMemory={slot}B; rows heap = 8B x bank_size once loaded"
        );
        assert!(memory <= 96, "BankMemory is {memory} bytes");
        assert!(slot <= 128, "SlotBankMemory is {slot} bytes");
    }
}
