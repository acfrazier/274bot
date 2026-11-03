//! The host's dispatch lifecycle of one native Talk-to, through the real
//! `script_observe` pass: the request is settled in the pass that takes it,
//! waits while no pass can take it, and is reported unsent when a held pass
//! drops it. The Quester dialogue's open wait reads exactly this
//! (`InteractionDispatch`), so these are its host-boundary cases.
use super::*;
use script::native::{
    ActionContext, ActionError, ActionHandle, InteractionDispatch, NativeMachine, NativeTick,
    Script, ScriptFailure, ScriptFlow,
};
use std::task::Poll;

/// What the probe machine saw on each poll, by observed tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Queued,
    Unsent,
    Settled { accepted: bool, tick: u64 },
}

#[derive(Default)]
struct Probe {
    /// Talk-to request ids, in the order the machine queued them.
    sent: Vec<u64>,
    seen: Vec<(u64, Seen)>,
}

fn talk_to() -> script::interact::InteractReq {
    script::interact::InteractReq::Npc {
        name: "Goblin".into(),
        action: "Talk-to".into(),
        index: Some(0),
    }
}

/// Queues one Talk-to and follows it as the dialogue's open wait does: it
/// waits while the host holds the request, queues it again when the host
/// dropped it unsent, and ends when the host settles it.
struct TalkProbe {
    shared: Arc<parking_lot::Mutex<Probe>>,
    request: u64,
}

impl NativeMachine for TalkProbe {
    type Args = Arc<parking_lot::Mutex<Probe>>;
    type Output = ();

    fn begin(shared: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let request = cx.emit(talk_to())?;
        shared.lock().sent.push(request);
        Ok(Self { shared, request })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        let tick = cx.evidence().tick;
        let seen = match cx.interaction_dispatch(self.request) {
            InteractionDispatch::Queued => Seen::Queued,
            InteractionDispatch::Unsent => Seen::Unsent,
            InteractionDispatch::Settled(receipt) => Seen::Settled {
                accepted: receipt.accepted,
                tick: receipt.evidence.tick,
            },
        };
        self.shared.lock().seen.push((tick, seen));
        match seen {
            Seen::Queued => Poll::Pending,
            Seen::Unsent => {
                self.request = std::task::ready!(script::native::defer_budget(cx.emit(talk_to())))?;
                self.shared.lock().sent.push(self.request);
                Poll::Pending
            }
            Seen::Settled { .. } => Poll::Ready(Ok(())),
        }
    }
}

struct ProbeScript {
    shared: Arc<parking_lot::Mutex<Probe>>,
    handle: Option<ActionHandle<TalkProbe>>,
    begun: bool,
}

impl Script for ProbeScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if let Some(handle) = &self.handle {
            if tick.actions.poll(handle, &mut tick.cx).is_ready() {
                self.handle = None;
            }
        } else if !self.begun {
            self.begun = true;
            self.handle = Some(
                tick.actions
                    .begin::<TalkProbe>(Arc::clone(&self.shared), &mut tick.cx)
                    .expect("the probe queues its Talk-to"),
            );
        }
        Ok(ScriptFlow::Continue)
    }

    fn recovery_anchor(&self) -> Option<api::WorldTile> {
        Some(api::WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        })
    }
}

struct TalkRig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    world: Option<Arc<NavWorld>>,
    snapshot: GameSnapshot,
    driver: NavRec,
    shared: Arc<parking_lot::Mutex<Probe>>,
}

fn talk_rig() -> TalkRig {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let navs = Arc::new(Mutex::new(HashMap::new()));
    bind_test_route_scope(&navs, "alice");
    let shared = Arc::new(parking_lot::Mutex::new(Probe::default()));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(ProbeScript {
                shared: Arc::clone(&shared),
                handle: None,
                begun: false,
            }),
            None,
        )
        .unwrap();
    let mut client = nav_client();
    let npc_type = 500;
    {
        let cache = Arc::get_mut(&mut client.cache).expect("sole test cache owner");
        while cache.npcs.len() <= npc_type {
            cache.npcs.push(client::config::NpcType::default());
        }
        cache.npcs[npc_type] = client::config::NpcType {
            id: npc_type as i32,
            name: "Goblin".into(),
            op: vec![Some("Talk-to".into())],
            ..Default::default()
        };
    }
    let mut npc = client::dash3d::ClientNpc::at(1, 0);
    npc.r#type = Some(npc_type);
    client.npc[0] = Some(Box::new(npc));
    client.npc_ids[0] = 0;
    client.npc_count = 1;
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    TalkRig {
        scripts,
        cheats: Arc::new(Mutex::new(HashMap::new())),
        navs,
        world: None,
        snapshot,
        driver: NavRec::default(),
        shared,
    }
}

impl TalkRig {
    /// One observation pass. `up` is the connection; `hold` is the slot
    /// loop's whole-frame hold and `guardian` its random-event part.
    fn pass(&mut self, tick: u64, up: bool, hold: bool, guardian: bool) {
        super::super::script_observe_cached_with_channels(
            &mut self.driver,
            "alice",
            up,
            true,
            false,
            tick,
            Some((0, 0, 0)),
            None,
            None,
            Some(&self.snapshot),
            None,
            #[cfg(feature = "js")]
            None,
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &self.world,
            hold,
            guardian,
            false,
            None,
            None,
            None,
            None,
            #[cfg(feature = "js")]
            None,
            #[cfg(feature = "js")]
            super::super::script_channels::BrokerWorld::Unavailable,
            None,
            None,
            None,
            None,
        );
    }

    fn sent(&self) -> Vec<u64> {
        self.shared.lock().sent.clone()
    }

    fn seen(&self) -> Vec<(u64, Seen)> {
        self.shared.lock().seen.clone()
    }

    fn stop(&self) {
        script_slot(&self.scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .stop();
    }
}

/// A pass that may dispatch takes the queued Talk-to and records its receipt
/// in the same pass: the next poll finds it settled, stamped with that tick.
#[test]
fn a_talk_to_queued_before_a_pass_that_can_send_it_is_settled_in_that_pass() {
    let mut rig = talk_rig();
    rig.pass(1, true, false, false);
    assert_eq!(rig.driver.npc_ops, 1, "the emit pass sends the click");
    rig.pass(2, true, false, false);
    rig.stop();
    assert_eq!(rig.sent().len(), 1);
    assert_eq!(
        rig.seen(),
        vec![(
            2,
            Seen::Settled {
                accepted: true,
                tick: 1
            }
        )]
    );
}

impl TalkRig {
    /// A pass in which the script runs and queues its Talk-to but the host
    /// does not reach its dispatch (the test hook skips it once), so the
    /// click stays in the host's custody after the pass.
    fn emit_without_dispatch(&mut self, tick: u64) {
        crate::script_runtime::delay_ordered_drain_for_test();
        self.pass(tick, true, false, false);
        assert_eq!(self.driver.npc_ops, 0, "the delayed pass sends nothing");
        assert_eq!(self.sent().len(), 1, "the script queued its Talk-to");
    }

    fn seat(&self) -> Option<script::slot::SeatProbe> {
        script_slot(&self.scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .seat_probe()
    }
}

/// A disconnected pass does not dispatch, and nothing drops the queued click:
/// it waits in the host's custody however many ticks pass, and the first pass
/// that can send it does.
#[test]
fn a_talk_to_waits_queued_while_no_pass_can_send_it() {
    let mut rig = talk_rig();
    rig.emit_without_dispatch(1);
    for tick in 2..=20 {
        rig.pass(tick, false, false, false);
    }
    assert_eq!(rig.driver.npc_ops, 0, "a disconnected host sends nothing");
    rig.pass(21, true, false, false);
    assert_eq!(rig.driver.npc_ops, 1, "the first connected pass sends it");
    rig.pass(22, true, false, false);
    rig.stop();
    let seen = rig.seen();
    assert!(
        seen[..seen.len() - 1]
            .iter()
            .all(|(_, seen)| *seen == Seen::Queued),
        "{seen:?}"
    );
    assert_eq!(
        seen.last(),
        Some(&(
            22,
            Seen::Settled {
                accepted: true,
                tick: 21
            }
        )),
        "{seen:?}"
    );
    assert_eq!(rig.sent().len(), 1, "a queued click is not queued twice");
}

/// A frame the slot loop holds (scene load, welcome modal) is a held tick:
/// the script does not run and the seat drops the queued gameplay
/// (`SlotScript::seat_held_tick`, `GameplaySeat::held_tick_keeping_releases`).
/// The first poll after the hold finds the click unsent, not settled and not
/// still queued; the probe queues it again, and that pass sends it.
#[test]
fn a_held_frame_drops_the_queued_talk_to_unsent() {
    let mut rig = talk_rig();
    rig.emit_without_dispatch(1);
    rig.pass(2, true, true, false);
    rig.pass(3, true, true, false);
    let held_ops = rig.driver.npc_ops;
    let seat = rig.seat();
    rig.pass(4, true, false, false);
    let after_ops = rig.driver.npc_ops;
    rig.pass(5, true, false, false);
    rig.stop();
    let sent = rig.sent();
    let seen = rig.seen();
    let detail = format!("sent {sent:?} seen {seen:?} seat after hold {seat:?}");
    assert_eq!(held_ops, 0, "a held frame sends nothing; {detail}");
    assert_eq!(after_ops, 1, "the click queued again is sent; {detail}");
    assert_eq!(sent.len(), 2, "{detail}");
    assert_ne!(sent[0], sent[1], "{detail}");
    assert_eq!(
        seen,
        vec![
            (4, Seen::Unsent),
            (
                5,
                Seen::Settled {
                    accepted: true,
                    tick: 4
                }
            )
        ],
        "{detail}"
    );
}

/// The random-event guardian's hold freezes the seat, which drops queued
/// gameplay on its rising edge (`GameplaySeat::hold`). The click is unsent
/// after the hold, as with any other held frame.
#[test]
fn a_guardian_hold_drops_the_queued_talk_to_unsent() {
    let mut rig = talk_rig();
    rig.emit_without_dispatch(1);
    rig.pass(2, true, true, true);
    let held_ops = rig.driver.npc_ops;
    let seat = rig.seat();
    rig.pass(3, true, false, false);
    let after_ops = rig.driver.npc_ops;
    rig.pass(4, true, false, false);
    rig.stop();
    let sent = rig.sent();
    let seen = rig.seen();
    let detail = format!("sent {sent:?} seen {seen:?} seat during hold {seat:?}");
    assert_eq!(held_ops, 0, "a guardian hold sends nothing; {detail}");
    assert_eq!(after_ops, 1, "{detail}");
    assert_eq!(sent.len(), 2, "{detail}");
    assert_eq!(
        seen,
        vec![
            (3, Seen::Unsent),
            (
                4,
                Seen::Settled {
                    accepted: true,
                    tick: 3
                }
            )
        ],
        "{detail}"
    );
}
