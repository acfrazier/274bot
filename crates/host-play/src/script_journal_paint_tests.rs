//! The host paint decision follows the journal owner, even without eligible ticks.
use super::*;
use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};
use script::quest_journal::{JournalMachine, JournalRequest};
use std::task::Poll;

struct Reader {
    handle: Option<ActionHandle<JournalMachine>>,
    facts: Arc<api::quest_facts::QuestCatalog>,
    fail: Arc<AtomicBool>,
}

impl Script for Reader {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.fail.load(Ordering::Acquire) {
            return Err(ScriptFailure {
                code: "forced".into(),
                message: "forced read failure".into(),
                retryable: false,
            });
        }
        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                self.handle = None;
                result.map_err(|error| ScriptFailure {
                    code: "read".into(),
                    message: format!("{error:?}").into(),
                    retryable: false,
                })?;
                return Ok(ScriptFlow::Complete);
            }
        } else {
            self.handle = Some(
                tick.actions
                    .begin(
                        JournalRequest {
                            quest: api::selected::FactKey::new("cook"),
                            facts: Arc::clone(&self.facts),
                        },
                        &mut tick.cx,
                    )
                    .map_err(|error| ScriptFailure {
                        code: "begin".into(),
                        message: format!("{error:?}").into(),
                        retryable: false,
                    })?,
            );
        }
        Ok(ScriptFlow::Continue)
    }
}

struct PaintRig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    driver: NavRec,
    snapshot: GameSnapshot,
    fail: Arc<AtomicBool>,
}

impl PaintRig {
    fn new() -> Self {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let facts = Arc::new(
            api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap(),
        );
        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let fail = Arc::new(AtomicBool::new(false));
        script_slot_or_insert(&scripts, "alice")
            .lock()
            .unwrap()
            .start_test_script(
                Box::new(Reader {
                    handle: None,
                    facts,
                    fail: Arc::clone(&fail),
                }),
                Some(selected),
            )
            .unwrap();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf8f800,
            }],
            true,
        );
        Self {
            scripts,
            cheats: Arc::new(Mutex::new(HashMap::new())),
            navs: Arc::new(Mutex::new(HashMap::new())),
            driver: NavRec::default(),
            snapshot,
            fail,
        }
    }

    fn observe(&mut self, tick: u64, up: bool, tick_edge: bool, hold: bool) -> bool {
        script_observe_cached_with_channels(
            &mut self.driver,
            "alice",
            up,
            tick_edge,
            false,
            tick,
            None,
            None,
            None,
            Some(&self.snapshot),
            None,
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &None,
            hold,
            false,
            None,
            None,
            None,
            None,
            None,
            super::super::script_channels::BrokerWorld::Unavailable,
            None,
            None,
        )
        .journal_paint_hidden
    }

    fn slot(&self) -> ScriptSlot {
        script_slot(&self.scripts, "alice").unwrap()
    }
}

#[test]
fn journal_paint_lease_survives_hold_and_clears_on_stop_without_tick() {
    let mut rig = PaintRig::new();
    assert!(
        rig.observe(1, true, true, false),
        "paint must be hidden before any journal click can be painted"
    );
    assert!(
        rig.observe(1, true, false, true),
        "hold freezes the read, not its paint ownership"
    );
    rig.slot().lock().unwrap().stop();
    assert!(
        !rig.observe(1, true, false, true),
        "Stop must restore paint without another script tick"
    );
}

#[test]
fn journal_paint_restores_after_forced_error_and_disconnect() {
    let mut rig = PaintRig::new();
    assert!(rig.observe(1, true, true, false));
    rig.fail.store(true, Ordering::Release);
    assert!(
        !rig.observe(2, true, true, false),
        "failed machine must release painting in its terminal observation"
    );
    let mut rig = PaintRig::new();
    assert!(rig.observe(1, true, true, false));
    rig.slot().lock().unwrap().reset_session_work();
    assert!(
        !rig.observe(1, false, false, false),
        "disconnect must restore paint even without an eligible tick"
    );
}
