//! Native-ledger Sherlock checks. These call [`Script::tick`], not `tick_host`.
use super::*;
use crate::native::{
    ledger, HostEffect, InteractionReceipt, NativeActions, NativeTick, RetainedMemory,
};
use api::game_data::SelectedGameData;
use api::obj_names::ItemDefView;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{
    ActorView, GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily, ItemContainer,
    LocalPlayerView, NpcView, PlayerView, SnapshotView, StatView, VarpView, WorldStateView,
    WorldTile,
};
use serde_json::Value;
use std::time::{Duration, Instant};

struct Output;
impl NativeOutput for Output {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, _: &str) {}
    fn settings_applied(&mut self, _: u64) {}
}

fn data() -> Arc<SelectedGameData> {
    api::game_data::for_revision(ClientRevision::R289).expect("selected data")
}

fn run_key() -> RunKey {
    RunKey {
        slot: 1,
        run: 1,
        session: 1,
    }
}

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn actor(at: WorldTile) -> ActorView {
    ActorView {
        name: None,
        actions: Vec::new(),
        tile: at,
        distance: 0,
        animation: -1,
        animation_frame: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 40,
        total_health: 40,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
    }
}

fn item(id: i32, name: &str, slot: i32, count: i32) -> api::snapshot::ItemView {
    api::snapshot::ItemView {
        def: ItemDefView {
            id,
            name: Some(name.into()),
            stackable: false,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count,
        actions: Vec::new(),
        component_id: 3214,
    }
}

fn wizard(at: WorldTile) -> NpcView {
    NpcView {
        index: 7,
        r#type: Some(1007),
        name: Some("Zamorak Wizard".into()),
        actions: vec![Some("Attack".into())],
        tile: tile(at.x + 1, at.z),
        distance: 1,
        animation: -1,
        animation_frame: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 80,
        total_health: 80,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 65,
        size: 1,
        network: tile(at.x + 1, at.z),
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn guarded_stand(data: &SelectedGameData) -> (i32, WorldTile) {
    let row = data
        .trails()
        .expect("trails")
        .rows
        .iter()
        .find(|row| {
            row.role == "clue"
                && row.params.iter().any(|param| param.key == "trail_guardian")
                && row
                    .params
                    .iter()
                    .any(|param| param.key == "trail_sextant" && param.value == "yes")
        })
        .expect("a guarded sextant row");
    let token = row
        .params
        .iter()
        .find(|param| param.key == "trail_coord")
        .expect("trail_coord")
        .value
        .clone();
    let parts: Vec<i32> = token
        .split('_')
        .filter_map(|part| part.parse().ok())
        .collect();
    let [level, map_x, map_z, local_x, local_z] = parts[..] else {
        panic!("decodable trail_coord");
    };
    (
        row.id,
        WorldTile {
            x: map_x * 64 + local_x,
            z: map_z * 64 + local_z,
            level,
        },
    )
}

struct World {
    data: Arc<SelectedGameData>,
    snapshot: GameSnapshot,
    stand: WorldTile,
    here: (i32, i32, i32),
    inventory: Vec<api::snapshot::ItemView>,
    npcs: Vec<NpcView>,
    varps: Vec<VarpView>,
    stats: Vec<StatView>,
    local: LocalPlayerView,
}

impl World {
    fn new(prayers_on: usize) -> Self {
        let data = data();
        let (clue_id, stand) = guarded_stand(&data);
        let spade = data.item_by_alias("spade").expect("spade");
        let sextant = data.item_by_alias("trail_sextant").expect("sextant");
        let watch = data.item_by_alias("trail_watch").expect("watch");
        let chart = data.item_by_alias("trail_chart").expect("chart");
        let inventory = vec![
            item(clue_id, "Clue scroll", 0, 1),
            item(spade.id, spade.name.as_deref().unwrap_or("Spade"), 1, 1),
            item(
                sextant.id,
                sextant.name.as_deref().unwrap_or("Sextant"),
                2,
                1,
            ),
            item(watch.id, watch.name.as_deref().unwrap_or("Watch"), 3, 1),
            item(chart.id, chart.name.as_deref().unwrap_or("Chart"), 4, 1),
        ];
        let varps = data
            .prayers()
            .iter()
            .enumerate()
            .map(|(index, row)| VarpView {
                index: row.varp,
                value: i32::from(index < prayers_on),
            })
            .chain([VarpView {
                index: crate::combat::OPTION_NODEF,
                value: 0,
            }])
            .collect();
        let stats = (0..25)
            .map(|index| StatView {
                index,
                name: api::snapshot::stat_name(index as usize).to_string(),
                effective: 40,
                base: 40,
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect();
        let local = LocalPlayerView {
            player: PlayerView {
                index: 1,
                actor: actor(stand),
                combat_level: 60,
                skill_level: 0,
                weapon: None,
            },
            energy: 100,
            weight: 0,
        };
        let mut world = Self {
            data,
            snapshot: GameSnapshot::new(),
            stand,
            here: (stand.x, stand.z, stand.level),
            inventory,
            npcs: vec![wizard(stand)],
            varps,
            stats,
            local,
        };
        world.refresh();
        world
    }

    fn refresh(&mut self) {
        self.snapshot.seed_ingame(2);
        self.snapshot.seed_world(WorldStateView {
            map_base_x: self.stand.x - 40,
            map_base_z: self.stand.z - 40,
            members: true,
            ..WorldStateView::default()
        });
        self.snapshot.seed_stats(self.stats.clone());
        self.snapshot.seed_varps(self.varps.clone());
        self.snapshot.seed_inventory(self.inventory.clone(), 28);
        self.snapshot.seed_equipment(Vec::new());
        self.snapshot.seed_npcs(self.npcs.clone());
        self.snapshot.seed_players(Vec::new());
        self.snapshot.seed_projectiles(Vec::new());
        self.snapshot.seed_side_tabs(Vec::new(), 0);
        self.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        self.snapshot.seed_chat_lines(Vec::new());
        self.snapshot.seed_local_player(self.local.clone());
    }

    fn set_prayers(&mut self, on: usize) {
        for (index, row) in self.varps.iter_mut().enumerate() {
            if self
                .data
                .prayers()
                .iter()
                .any(|prayer| prayer.varp == row.index)
            {
                row.value = i32::from(index < on);
            }
        }
        self.refresh();
    }
}

fn script(world: &World, hygiene_pending: bool) -> Sherlock {
    Sherlock {
        tables: Some(CombatTables::build(Arc::clone(&world.data)).expect("tables")),
        hygiene_pending,
        dirty: true,
        ..Default::default()
    }
}

fn with_tick<R>(
    world: &World,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    let pin = world.data.selected_pin().unwrap();
    let evidence = EvidenceStamp {
        run: run_key(),
        tick,
        sequence: tick,
    };
    let mut retained = RetainedMemory::default();
    let mut budget = ledger::TickBudget::default();
    budget.observe(tick);
    let mut actions = NativeActions { _private: () };
    let mut output = Output;
    let mut native = NativeTick {
        actions: &mut actions,
        cx: crate::native::ActionContext {
            evidence,
            observed_walk_outcome_seq: 0,
            pin: &pin,
            snapshot: SnapshotView::new(Some(&world.snapshot), evidence),
            retained: &mut retained,
            action_id: 0,
            active_now: Duration::from_millis(tick * 600),
            wall_now: Instant::now(),
            ledger,
            budget: &mut budget,
            eligible: true,
        },
        output: &mut output,
        pairs: None,
        frame: HostFrame {
            here: Some(world.here),
            snapshot: Some(&world.snapshot),
            obj_names: None,
            compiled: crate::CompiledTick {
                selected: Some(&world.data),
                reach: None,
                hold: false,
                interacts: Some(Vec::new()),
            },
        },
    };
    f(&mut native)
}

fn drive(
    script: &mut Sherlock,
    world: &World,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> (Result<ScriptFlow, ScriptFailure>, Vec<InteractReq>) {
    with_tick(world, ledger, tick, |native| {
        let flow = script.tick(native);
        let interacts = native.frame.compiled.interacts.take().unwrap_or_default();
        (flow, interacts)
    })
}

fn live_owner(ledger: &Option<Box<ledger::Ledger>>) -> bool {
    ledger
        .as_ref()
        .and_then(|ledger| ledger.owner.as_ref())
        .is_some_and(|owner| owner.live())
}

fn accept_outbox(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64) {
    let Some(ledger) = ledger.as_mut() else {
        return;
    };
    while !ledger.outbox.is_empty() {
        let action = ledger.outbox.remove(0);
        if matches!(action.effect, HostEffect::Interaction(_)) {
            ledger.complete_interaction(
                &action.authority(),
                InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: action.run(),
                        tick,
                        sequence: tick,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
        }
    }
}

fn until_combat(
    script: &mut Sherlock,
    world: &mut World,
    ledger: &mut Option<Box<ledger::Ledger>>,
    start: u64,
) -> u64 {
    for tick in start..start + 12 {
        let (flow, interacts) = drive(script, world, ledger, tick);
        flow.expect("tick");
        if matches!(script.fight, Some(Fight::Combat(_))) {
            assert!(
                interacts.is_empty(),
                "compiled interacts while Combat is live: {interacts:?}"
            );
            assert!(live_owner(ledger), "exactly one native owner");
            return tick;
        }
        accept_outbox(ledger, tick);
    }
    panic!(
        "Combat did not begin; fight={} pending={} outcome={:?} blocked={:?}",
        script.fight.is_some(),
        script.pending.is_some(),
        script.outcome,
        script.blocked
    );
}

#[test]
fn pause_delivers_the_cancelled_outcome_instead_of_parking() {
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = until_combat(&mut script, &mut world, &mut ledger, 1);
    let old_id = script.combat_id.expect("delegated id");
    assert!(matches!(script.fight, Some(Fight::Combat(_))));

    script.interrupt(Interrupt::Pause);
    assert!(
        script
            .outcome
            .is_some_and(|outcome| { outcome == Outcome::cancelled(old_id) }),
        "Pause must keep Outcome::cancelled({old_id}), got {:?}",
        script.outcome
    );
    assert!(script.fight.is_none());
    assert!(script.hygiene_pending);

    let mut resumed_combat = false;
    for tick in begun + 1..begun + 8 {
        let (flow, interacts) = drive(&mut script, &world, &mut ledger, tick);
        let _ = flow.expect("resume tick");
        if matches!(script.fight, Some(Fight::Combat(_))) {
            assert_ne!(
                script.combat_id,
                Some(old_id),
                "fresh delegation after cancelled"
            );
            assert!(interacts.is_empty());
            resumed_combat = true;
            break;
        }
        accept_outbox(&mut ledger, tick);
    }
    assert!(
        resumed_combat,
        "Resume must deliver cancelled and re-delegate, not wait forever; outcome={:?} fight={} pending={}",
        script.outcome,
        script.fight.is_some(),
        script.pending.is_some()
    );
}

#[test]
fn clear_prayers_completion_error_does_not_clear_hygiene_pending() {
    let mut world = World::new(6);
    let mut script = script(&world, true);
    let mut ledger = None;

    let (flow, _) = drive(&mut script, &world, &mut ledger, 0);
    flow.expect("hygiene begin");
    assert!(
        matches!(script.fight, Some(Fight::ClearPrayers(_))),
        "startup hygiene must begin ClearPrayers"
    );
    assert!(script.hygiene_pending);

    let (flow, interacts) = drive(&mut script, &world, &mut ledger, 1);
    flow.expect("hygiene poll");
    assert!(interacts.is_empty(), "{interacts:?}");
    accept_outbox(&mut ledger, 1);

    let (flow, _) = drive(&mut script, &world, &mut ledger, 2);
    flow.expect("hygiene retry batch");
    accept_outbox(&mut ledger, 2);

    let _ = drive(&mut script, &world, &mut ledger, 3);
    let _ = drive(&mut script, &world, &mut ledger, 4);
    let _ = drive(&mut script, &world, &mut ledger, 5);

    world.set_prayers(0);
    let (flow, _) = drive(&mut script, &world, &mut ledger, 6);
    match flow {
        Ok(ScriptFlow::Blocked(failure)) => {
            assert!(
                script.hygiene_pending,
                "Poll::Ready(Err(_)) must not mark hygiene complete"
            );
            assert_eq!(failure.code.as_ref(), "combat-failed");
            assert!(
                failure.message.contains("prayer cleanup") || failure.message.contains("settle"),
                "{}",
                failure.message
            );
        }
        other => panic!(
            "expected Blocked hygiene failure, got {other:?}; pending={} fight={}",
            script.hygiene_pending,
            script.fight.is_some()
        ),
    }
    assert!(
        script.hygiene_pending,
        "a ClearPrayers completion error must keep hygiene pending"
    );
}

#[test]
fn startup_hygiene_clears_prayers_before_the_first_pump() {
    let world = World::new(1);
    let mut script = script(&world, true);
    let mut ledger = None;
    let (flow, interacts) = drive(&mut script, &world, &mut ledger, 1);
    flow.expect("startup");
    assert!(
        matches!(script.fight, Some(Fight::ClearPrayers(_))),
        "prayer bit on at card start is cleared before the first pump"
    );
    assert!(
        interacts.is_empty(),
        "no Dig/walk while startup hygiene is live: {interacts:?}"
    );
}

#[test]
fn native_fight_is_one_owner_with_no_compiled_interacts() {
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = until_combat(&mut script, &mut world, &mut ledger, 1);
    assert!(live_owner(&ledger));
    let (flow, interacts) = drive(&mut script, &world, &mut ledger, begun + 1);
    flow.expect("steady");
    assert!(
        interacts.is_empty(),
        "steady delegated tick must not queue compiled interacts: {interacts:?}"
    );
    assert!(
        matches!(script.fight, Some(Fight::Combat(_))),
        "steady tick keeps the live Combat owner"
    );
    assert!(live_owner(&ledger));
}

#[test]
fn ready_ok_lands_as_the_next_combat_page() {
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = until_combat(&mut script, &mut world, &mut ledger, 1);
    accept_outbox(&mut ledger, begun);
    world.local.player.actor.target = Some(api::snapshot::ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    });
    world.local.player.actor.in_combat = true;
    world.npcs[0].target = Some(api::snapshot::ActorTargetView {
        kind: ActorKind::Player,
        index: 1,
    });
    world.npcs[0].in_combat = true;
    world.refresh();
    let _ = drive(&mut script, &world, &mut ledger, begun + 1);
    accept_outbox(&mut ledger, begun + 1);

    world.npcs[0].health = 0;
    world.refresh();
    let mut delivered = false;
    for tick in begun + 2..begun + 10 {
        let combat_id = script.combat_id;
        let (flow, interacts) = drive(&mut script, &world, &mut ledger, tick);
        flow.expect("kill tick");
        accept_outbox(&mut ledger, tick);
        if script.fight.is_none() {
            let Some(combat_id) = combat_id else {
                continue;
            };
            let report = super::take_combat_page().expect(
                "Ready(Ok) must land as the next page's combat report; a parked token is not delivery",
            );
            assert_eq!(
                report.get("id").and_then(Value::as_u64),
                Some(u64::from(combat_id)),
                "next page combat id must match the outstanding delegation: {report}"
            );
            assert_eq!(
                report.get("end").and_then(Value::as_str),
                Some("killed"),
                "next page must carry the Killed report: {report}"
            );
            assert!(
                interacts.iter().any(|req| matches!(
                    req,
                    InteractReq::Held { action, .. } if action == "Dig"
                )),
                "post-kill Spade Dig required; a parked token is not delivery (token={:?}, interacts={:?})",
                script.token,
                interacts
            );
            assert!(
                script.token.is_some(),
                "the clue token remains after the report; it is not itself delivery"
            );
            delivered = true;
            break;
        }
    }
    assert!(
        delivered,
        "Ready(Ok) must land as the next page's combat report"
    );
}

#[test]
fn pause_with_a_prayer_bit_runs_hygiene_before_the_cancelled_report() {
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = until_combat(&mut script, &mut world, &mut ledger, 1);
    world.set_prayers(1);
    script.interrupt(Interrupt::Pause);
    let (flow, interacts) = drive(&mut script, &world, &mut ledger, begun + 1);
    flow.expect("hygiene after pause");
    assert!(
        matches!(script.fight, Some(Fight::ClearPrayers(_))),
        "Ready(Err(Cancelled)) runs the hygiene step when a prayer bit is on"
    );
    assert!(interacts.is_empty());
}

#[test]
fn allocation_counts_by_tick_class() {
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let mut entry = None;
    let mut entry_tick = 0;
    for tick in 1..12 {
        let measured = allocation_counter::measure(|| {
            let _ = drive(&mut script, &world, &mut ledger, tick);
        });
        accept_outbox(&mut ledger, tick);
        if matches!(script.fight, Some(Fight::Combat(_))) {
            entry = Some(measured);
            entry_tick = tick;
            break;
        }
    }
    let entry = entry.expect("entry tick began Combat");
    let steady = allocation_counter::measure(|| {
        let _ = drive(&mut script, &world, &mut ledger, entry_tick + 1);
    });
    world.npcs[0].health = 0;
    world.refresh();
    let mut report = None;
    for tick in entry_tick + 2..entry_tick + 10 {
        let measured = allocation_counter::measure(|| {
            let _ = drive(&mut script, &world, &mut ledger, tick);
        });
        accept_outbox(&mut ledger, tick);
        if script.fight.is_none() {
            report = Some(measured);
            break;
        }
    }
    let report = report.expect("report tick");
    eprintln!(
        "CLUE-COMBAT-1 allocations: entry count={} bytes={}; steady count={} bytes={}; report count={} bytes={}",
        entry.count_total,
        entry.bytes_current,
        steady.count_total,
        steady.bytes_current,
        report.count_total,
        report.bytes_current
    );
}
