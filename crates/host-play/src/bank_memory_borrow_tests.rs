//! The compiled tick's bank-memory borrow ends with the tick
//! (design-bank-snapshot §1.2): the observer takes the read guard for the
//! native frame evaluation alone and releases it before effect dispatch,
//! logging and the queued cheats that close the same observe call.
use super::*;
use api::bank_memory::{BankMemory, Origin};
use parking_lot::RwLock;
use script::native::{NativeTick, Script, ScriptFailure, ScriptFlow};

/// A native script that records what the frame borrow shows it.
struct BorrowProbe {
    seen: Arc<Mutex<Vec<Option<Origin>>>>,
}

impl Script for BorrowProbe {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        self.seen
            .lock()
            .unwrap()
            .push(tick.cx.snapshot().bank_memory().map(BankMemory::origin));
        Ok(ScriptFlow::Continue)
    }
}

/// A driver whose cheat send, the last step of the observe call, probes
/// whether the bank memory can still be written: it can only once the
/// tick's read guard is gone.
struct LockProbe {
    memory: Arc<RwLock<BankMemory>>,
    writable_at_cheat: Arc<Mutex<Vec<bool>>>,
    sink: Sink,
}

impl Driver for LockProbe {
    fn cheat_admission(&self) -> client::CheatAdmission {
        client::CheatAdmission::Granted
    }
    fn send_cheat(&mut self, _cmd: &str) -> client::CheatSend {
        self.writable_at_cheat
            .lock()
            .unwrap()
            .push(self.memory.try_write().is_some());
        client::CheatSend::Sent
    }
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _ty: i32,
    ) -> bool {
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        Some((0, 0))
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.sink
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

#[test]
fn the_compiled_tick_borrow_is_released_before_queued_cheats_dispatch() {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(BorrowProbe {
                seen: Arc::clone(&seen),
            }),
            Some(selected),
        )
        .unwrap();

    // A known memory: the tick must see its origin through the borrow.
    let mut bank = GameSnapshot::new();
    bank.seed_bank_observation(
        5292,
        1,
        Some(vec![api::snapshot::ItemView {
            def: api::ItemDefView {
                id: 995,
                name: Some("Coins".into()),
                stackable: true,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: api::snapshot::ItemContainer::Bank,
            action_family: api::snapshot::ItemActionFamily::Component,
            slot: 0,
            count: 50,
            actions: Vec::new(),
            component_id: 5292,
        }]),
        Vec::new(),
    );
    let mut memory = BankMemory::default();
    memory.track(&bank, 1);
    memory.relogged();
    let memory = Arc::new(RwLock::new(memory));

    let cheats = Arc::new(Mutex::new(HashMap::from([(
        "alice".to_string(),
        VecDeque::from(["getvar tutorial".to_string()]),
    )])));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let writable_at_cheat = Arc::new(Mutex::new(Vec::new()));
    let mut driver = LockProbe {
        memory: Arc::clone(&memory),
        writable_at_cheat: Arc::clone(&writable_at_cheat),
        sink: Sink,
    };

    let observation = script_observe_cached_with_channels(
        &mut driver,
        "alice",
        true,
        true,
        false,
        1,
        None,
        None,
        None,
        Some(&snapshot),
        Some(&memory),
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &None,
        false,
        false,
        None,
        None,
        None,
        None,
        None,
        crate::script_channels::BrokerWorld::Unavailable,
        None,
        None,
    );

    assert!(observation.wrote, "the tick ran and the cheat was sent");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some(Origin::Hint)],
        "the compiled tick saw the memory through the frame borrow"
    );
    assert_eq!(
        *writable_at_cheat.lock().unwrap(),
        vec![true],
        "the queued cheat dispatched with the tick's read guard released"
    );
    assert!(
        memory.try_write().is_some(),
        "nothing outlives the observe call"
    );
    assert!(cheats.lock().unwrap()["alice"].is_empty());
}

#[test]
fn without_a_memory_the_tick_sees_none_and_cheats_still_dispatch() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(BorrowProbe {
                seen: Arc::clone(&seen),
            }),
            None,
        )
        .unwrap();
    let cheats = Arc::new(Mutex::new(HashMap::from([(
        "alice".to_string(),
        VecDeque::from(["getvar tutorial".to_string()]),
    )])));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let writable_at_cheat = Arc::new(Mutex::new(Vec::new()));
    let unused = Arc::new(RwLock::new(BankMemory::default()));
    let mut driver = LockProbe {
        memory: Arc::clone(&unused),
        writable_at_cheat: Arc::clone(&writable_at_cheat),
        sink: Sink,
    };

    script_observe_cached_with_channels(
        &mut driver,
        "alice",
        true,
        true,
        false,
        1,
        None,
        None,
        None,
        Some(&snapshot),
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &None,
        false,
        false,
        None,
        None,
        None,
        None,
        None,
        crate::script_channels::BrokerWorld::Unavailable,
        None,
        None,
    );

    assert_eq!(*seen.lock().unwrap(), vec![None]);
    assert_eq!(*writable_at_cheat.lock().unwrap(), vec![true]);
}
