//! Native-ledger Sherlock checks. These call [`Script::tick`], not `tick_host`.
use super::*;
use api::game_data::SelectedGameData;
use api::obj_names::ItemDefView;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{
    ActorView, ChatLineView, GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily,
    ItemContainer, LocalPlayerView, NpcView, PlayerView, StatView, VarpView, WorldStateView,
    WorldTile,
};
use script::native::{ledger, HostEffect, InteractionReceipt, NativeTick};
use serde_json::Value;
use std::time::{Duration, Instant};

struct Output;
impl NativeOutput for Output {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<script::paint::ScriptPaint>) {}
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

fn chat_line(sequence: i32, text: &str) -> ChatLineView {
    ChatLineView {
        type_: 0,
        username: None,
        text: text.into(),
        sequence,
    }
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
        tile: WorldTile {
            x: at.x + 1,
            z: at.z,
            level: at.level,
        },
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
        network: WorldTile {
            x: at.x + 1,
            z: at.z,
            level: at.level,
        },
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
    held: bool,
    run: RunKey,
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
                index: script::combat::OPTION_NODEF,
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
                network: stand,
                actor: actor(stand),
                combat_level: 60,
                skill_level: 0,
                headicons: 0,
                weapon: None,
            },
            energy: 100,
            weight: 0,
            animation_update: None,
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
            held: false,
            run: run_key(),
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
    fn set_hitpoints(&mut self, hitpoints: i32) {
        self.stats
            .iter_mut()
            .find(|stat| stat.name == "hitpoints")
            .expect("hitpoints stat")
            .effective = hitpoints;
        self.refresh();
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

    fn set_prayer(&mut self, varp: i32, on: bool) {
        self.varps
            .iter_mut()
            .find(|row| row.index == varp)
            .expect("selected prayer varp")
            .value = i32::from(on);
        self.refresh();
    }

    fn prayer_is_on(&self, varp: i32) -> bool {
        self.varps
            .iter()
            .find(|row| row.index == varp)
            .is_some_and(|row| row.value != 0)
    }

    fn prepare_policy_s2_combat(&mut self) {
        let imp = self.data.npc_by_config("imp").expect("selected imp");
        self.npcs[0].r#type = Some(usize::try_from(imp.id).expect("positive imp id"));
        self.npcs[0].name = imp.display.clone();
        self.npcs[0].target = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: self.local.player.index,
        });
        self.npcs[0].in_combat = true;
        self.local.player.actor.target = Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: self.npcs[0].index,
        });
        self.local.player.actor.in_combat = true;
        self.refresh();
        self.snapshot
            .seed_projectiles(vec![api::snapshot::ProjectileView {
                spotanim: 9,
                level: 0,
                src: self.npcs[0].tile,
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: self.local.player.index,
                }),
                t1: 0,
                t2: 30,
            }]);
    }
}

fn script(world: &World, hygiene_pending: bool) -> Sherlock {
    let mut hygiene_owned = RaisedPrayers::empty();
    if hygiene_pending {
        if let Some(prayer) = world
            .data
            .prayers()
            .iter()
            .find(|prayer| world.prayer_is_on(prayer.varp))
        {
            hygiene_owned.accepted(prayer.varp, true, 0);
        }
    }
    Sherlock {
        tables: Some(CombatTables::build(Arc::clone(&world.data)).expect("tables")),
        hygiene_pending: !hygiene_owned.is_empty(),
        hygiene_owned,
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
    let revision = selected_revision(&world.data);
    let evidence = EvidenceStamp {
        run: world.run,
        tick,
        sequence: tick,
    };
    let mut output = Output;
    let frame = HostFrame::for_test(
        Some(world.here),
        Some(&world.snapshot),
        None,
        script::CompiledTick {
            condition: None,
            poison_unknown_pulse: None,
            cure_poison: script::survival::CurePoison::WhenNeeded,
            walk_danger: None,
            wilderness: None,
            pvp_globals: None,
            selected: Some(&world.data),
            reach: None,
            reach_flood: None,
            bank_memory: None,
            shop_keepers: None,
            collision: None,
            nav: None,
            hold: world.held,
            world_members: api::selected::Truth::Unknown,
            interacts: Some(Vec::new()),
            service_gate: None,
        },
    );
    script::native::with_test_tick(
        frame,
        evidence,
        revision,
        (Duration::from_millis(tick * 600), Instant::now()),
        ledger,
        &mut output,
        f,
    )
}

fn begin_policy_s2_combat(
    script: &mut Sherlock,
    world: &World,
    ledger: &mut Option<Box<ledger::Ledger>>,
) {
    let npc_type =
        i32::try_from(world.npcs[0].r#type.expect("combat npc type")).expect("selected npc type");
    let request = CombatRequest {
        target: script::combat::Target::Npc {
            types: Arc::from([npc_type]),
            pick: script::combat::Pick::Nearest,
            not_targeting_others: true,
        },
        ..CombatRequest::default()
    };
    let tables = Arc::clone(script.tables.as_ref().expect("combat tables"));
    let handle = with_tick(world, ledger, 0, |tick| {
        tick.actions
            .begin::<Combat>(
                (
                    Arc::new(request),
                    tables,
                    script::native::PvpGlobals::fail_closed(),
                ),
                &mut tick.cx,
            )
            .expect("begin policy Combat")
    });
    script.fight = Some(Fight::Combat(handle));
    script.combat_id = Some(700);
}

fn drive(
    script: &mut Sherlock,
    world: &World,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> (Result<ScriptFlow, ScriptFailure>, Vec<InteractReq>) {
    with_tick(world, ledger, tick, |native| {
        let flow = script.tick(native);
        let interacts = native
            .frame_mut()
            .compiled_mut()
            .interacts
            .take()
            .unwrap_or_default();
        (flow, interacts)
    })
}

fn live_owner(ledger: &Option<Box<ledger::Ledger>>) -> bool {
    ledger
        .as_ref()
        .is_some_and(|ledger| ledger.test_owner_live())
}

fn complete_outbox(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, accepted: bool) {
    let Some(ledger) = ledger.as_mut() else {
        return;
    };
    while !ledger.test_outbox().is_empty() {
        let action = ledger.test_outbox_mut().remove(0);
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
                    accepted,
                    chat_since: 0,
                },
            );
        }
    }
}

fn accept_outbox(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64) {
    complete_outbox(ledger, tick, true);
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
fn network_coords_native_clue_reads_snapshot_heads_without_a_v8_scene() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    crate::scene::observed::on_reset();
    let mut world = World::new(0);
    world.local.player.actor.tile.x += 40;
    world.local.player.actor.tile.z += 40;
    world.npcs[0].tile = WorldTile {
        x: world.stand.x + 60,
        z: world.stand.z + 60,
        level: world.stand.level,
    };
    world.npcs[0].network = WorldTile {
        x: world.stand.x + 1,
        z: world.stand.z,
        level: world.stand.level,
    };
    world.npcs[0].distance = 60;
    world.refresh();
    let mut script = script(&world, false);
    let mut ledger = None;
    until_combat(&mut script, &mut world, &mut ledger, 1);
}

#[test]
fn pause_delivers_the_cancelled_outcome_instead_of_parking() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    assert!(!script.hygiene_pending);

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
    if !api::test_support::game_data_available() {
        return;
    }
    let mut world = World::new(6);
    let mut script = script(&world, true);
    let mut ledger = None;

    let (flow, _) = drive(&mut script, &world, &mut ledger, 0);
    flow.expect("hygiene begin");
    assert!(
        matches!(script.fight, Some(Fight::ClearPrayers(_))),
        "scoped hygiene must begin ClearPrayers"
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
fn policy_s2_startup_preserves_user_prayers() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = World::new(1);
    let thick_skin = world.data.prayer_by_name("Thick Skin").unwrap();
    let mut script = script(&world, false);
    let mut ledger = None;
    let (flow, _) = drive(&mut script, &world, &mut ledger, 1);
    flow.expect("startup");
    assert!(!matches!(script.fight, Some(Fight::ClearPrayers(_))));
    assert!(!script.hygiene_pending);
    assert!(world.prayer_is_on(thick_skin.varp));
}

#[test]
fn native_fight_is_one_owner_with_no_compiled_interacts() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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
fn policy_s2_pause_clears_only_accepted_combat_raise_before_poll() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut world = World::new(0);
    let skin = world.data.prayer_by_name("Thick Skin").unwrap().clone();
    let displaced = world
        .data
        .prayer_by_name("Protect from Melee")
        .unwrap()
        .clone();
    let raised = world
        .data
        .prayer_by_name("Protect from Missiles")
        .unwrap()
        .clone();
    world.set_prayer(skin.varp, true);
    world.set_prayer(displaced.varp, true);
    world.stats[5].effective = 43;
    world.stats[5].base = 43;
    world.prepare_policy_s2_combat();
    let mut script = script(&world, false);
    let mut ledger = None;
    begin_policy_s2_combat(&mut script, &world, &mut ledger);

    let (flow, interacts) = drive(&mut script, &world, &mut ledger, 1);
    flow.expect("Combat prayer raise");
    assert!(interacts.is_empty());
    let prayer_buttons: Vec<i32> = ledger
        .as_ref()
        .unwrap()
        .test_outbox()
        .iter()
        .filter_map(|action| match &action.effect {
            HostEffect::Interaction(InteractReq::IfButton { component_id }) => Some(*component_id),
            _ => None,
        })
        .collect();
    assert!(
        prayer_buttons.contains(&raised.button_com),
        "Combat's accepted batch must contain Protect from Missiles: {prayer_buttons:?}"
    );
    accept_outbox(&mut ledger, 1);

    world.set_prayer(displaced.varp, false);
    world.set_prayer(raised.varp, true);
    script.interrupt(Interrupt::Pause);
    assert_eq!(script.outcome, Some(Outcome::cancelled(700)));

    let (flow, interacts) = drive(&mut script, &world, &mut ledger, 2);
    flow.expect("begin scoped prayer cleanup");
    assert!(interacts.is_empty());
    assert!(matches!(script.fight, Some(Fight::ClearPrayers(_))));
    let (flow, _) = drive(&mut script, &world, &mut ledger, 3);
    flow.expect("poll scoped prayer cleanup");
    let cleanup_buttons: Vec<i32> = ledger
        .as_ref()
        .unwrap()
        .test_outbox()
        .iter()
        .filter_map(|action| match &action.effect {
            HostEffect::Interaction(InteractReq::IfButton { component_id }) => Some(*component_id),
            _ => None,
        })
        .collect();
    assert_eq!(cleanup_buttons, vec![raised.button_com]);
    accept_outbox(&mut ledger, 3);

    world.set_prayer(raised.varp, false);
    drive(&mut script, &world, &mut ledger, 4)
        .0
        .expect("finish scoped prayer cleanup");
    assert!(world.prayer_is_on(skin.varp));
    assert!(!world.prayer_is_on(displaced.varp));
    assert!(!world.prayer_is_on(raised.varp));
    let cancelled = super::take_combat_page().expect("resume receives cancellation");
    assert_eq!(cancelled.get("id").and_then(Value::as_u64), Some(700));
    assert_eq!(
        cancelled.get("end").and_then(Value::as_str),
        Some("cancelled")
    );
}

#[test]
fn policy_s2_pending_or_refused_raise_never_becomes_cleanup_ownership() {
    if !api::test_support::game_data_available() {
        return;
    }
    for refused in [false, true] {
        let mut world = World::new(0);
        let skin = world.data.prayer_by_name("Thick Skin").unwrap().clone();
        let original = world
            .data
            .prayer_by_name("Protect from Melee")
            .unwrap()
            .clone();
        let replacement = world
            .data
            .prayer_by_name("Protect from Missiles")
            .unwrap()
            .clone();
        world.set_prayer(skin.varp, true);
        world.set_prayer(original.varp, true);
        world.stats[5].effective = 43;
        world.stats[5].base = 43;
        world.prepare_policy_s2_combat();
        let mut script = script(&world, false);
        let mut ledger = None;
        begin_policy_s2_combat(&mut script, &world, &mut ledger);

        let (flow, _) = drive(&mut script, &world, &mut ledger, 1);
        flow.expect("Combat prayer raise");
        let prayer_buttons: Vec<i32> = ledger
            .as_ref()
            .unwrap()
            .test_outbox()
            .iter()
            .filter_map(|action| match &action.effect {
                HostEffect::Interaction(InteractReq::IfButton { component_id }) => {
                    Some(*component_id)
                }
                _ => None,
            })
            .collect();
        assert!(prayer_buttons.contains(&replacement.button_com));
        if refused {
            complete_outbox(&mut ledger, 1, false);
        }

        script.interrupt(Interrupt::Pause);
        if !refused {
            ledger.as_mut().unwrap().test_outbox_mut().clear();
        }
        for tick in 2..=3 {
            drive(&mut script, &world, &mut ledger, tick)
                .0
                .expect("cancelled Combat resumes without prayer cleanup");
        }
        let prayer_buttons: Vec<i32> = ledger
            .as_ref()
            .unwrap()
            .test_outbox()
            .iter()
            .filter_map(|action| match &action.effect {
                HostEffect::Interaction(InteractReq::IfButton { component_id })
                    if [skin.button_com, original.button_com, replacement.button_com]
                        .contains(component_id) =>
                {
                    Some(*component_id)
                }
                _ => None,
            })
            .collect();
        assert!(
            prayer_buttons.is_empty(),
            "refused={refused}: {prayer_buttons:?}"
        );
        assert!(world.prayer_is_on(skin.varp));
        assert!(world.prayer_is_on(original.varp));
        assert!(!world.prayer_is_on(replacement.varp));
    }
}

#[test]
fn policy_s2_pause_does_not_retain_displaced_ownership() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut world = World::new(0);
    let ranged = world
        .data
        .prayer_by_name("Protect from Missiles")
        .unwrap()
        .clone();
    let magic = world
        .data
        .prayer_by_name("Protect from Magic")
        .unwrap()
        .clone();
    world.stats[5].effective = 43;
    world.stats[5].base = 43;
    world.prepare_policy_s2_combat();
    let mut script = script(&world, false);
    let mut ledger = None;
    begin_policy_s2_combat(&mut script, &world, &mut ledger);
    drive(&mut script, &world, &mut ledger, 1).0.unwrap();
    accept_outbox(&mut ledger, 1);
    world.set_prayer(ranged.varp, true);
    world.prepare_policy_s2_combat();
    drive(&mut script, &world, &mut ledger, 2).0.unwrap();
    accept_outbox(&mut ledger, 2);
    assert!(script.hygiene_owned.contains(ranged.varp));
    world
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim: 88,
            level: 0,
            src: world.npcs[0].tile,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: world.local.player.index,
            }),
            t1: 0,
            t2: 30,
        }]);
    drive(&mut script, &world, &mut ledger, 3).0.unwrap();
    assert!(ledger.as_ref().unwrap().test_outbox().iter().any(|action| {
        matches!(action.effect, HostEffect::Interaction(InteractReq::IfButton { component_id })
            if component_id == magic.button_com)
    }));
    accept_outbox(&mut ledger, 3);
    world.set_prayer(ranged.varp, false);
    world.set_prayer(magic.varp, true);
    script.interrupt(Interrupt::Pause);
    assert!(!script.hygiene_owned.contains(ranged.varp));
    assert!(script.hygiene_owned.contains(magic.varp));
    // The user switches back while paused. That activation is not Combat's.
    world.set_prayer(magic.varp, false);
    world.set_prayer(ranged.varp, true);
    for tick in 4..=5 {
        drive(&mut script, &world, &mut ledger, tick).0.unwrap();
    }
    assert!(ledger.as_ref().unwrap().test_outbox().is_empty());
    assert!(world.prayer_is_on(ranged.varp));
    assert!(!script.hygiene_pending);
}

#[test]
fn native_death_then_pause_preserves_a_user_prayer_raised_after_respawn() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut world = World::new(0);
    let raised = world
        .data
        .prayer_by_name("Protect from Missiles")
        .unwrap()
        .clone();
    world.stats[5].effective = 43;
    world.stats[5].base = 43;
    world.prepare_policy_s2_combat();
    let mut script = script(&world, false);
    let mut ledger = None;
    begin_policy_s2_combat(&mut script, &world, &mut ledger);
    drive(&mut script, &world, &mut ledger, 1)
        .0
        .expect("Combat prayer raise");
    assert!(ledger.as_ref().unwrap().test_outbox().iter().any(|action| {
        matches!(
            action.effect,
            HostEffect::Interaction(InteractReq::IfButton { component_id })
                if component_id == raised.button_com
        )
    }));
    accept_outbox(&mut ledger, 1);
    world.set_prayer(raised.varp, true);
    world.prepare_policy_s2_combat();
    drive(&mut script, &world, &mut ledger, 2)
        .0
        .expect("observe the accepted Combat raise");
    accept_outbox(&mut ledger, 2);
    assert!(
        script.hygiene_owned.contains(raised.varp),
        "the death must settle an actual cached Combat obligation"
    );

    // Real death turns prayers off. The chat latch cancels the live action
    // before any future cleanup can mistake a respawned user's raise for it.
    world.set_hitpoints(0);
    world.set_prayer(raised.varp, false);
    world
        .snapshot
        .seed_chat_lines(vec![chat_line(1, "Oh dear, you are dead!")]);
    world.held = true;
    let (flow, interactions) = drive(&mut script, &world, &mut ledger, 3);
    assert_eq!(flow.expect("death tick"), ScriptFlow::Continue);
    assert!(interactions.is_empty());
    assert!(!live_owner(&ledger), "death revokes the Combat owner");

    world.set_hitpoints(40);
    world.set_prayer(raised.varp, true);
    script.interrupt(Interrupt::Pause);
    world.held = false;
    for tick in 4..=5 {
        drive(&mut script, &world, &mut ledger, tick)
            .0
            .expect("post-respawn Pause tick");
        assert!(
            !ledger.as_ref().is_some_and(|ledger| {
                ledger.test_outbox().iter().any(|action| {
                    matches!(
                        action.effect,
                        HostEffect::Interaction(InteractReq::IfButton { component_id })
                            if component_id == raised.button_com
                    )
                })
            }),
            "Pause must not click off the prayer the user raised after respawn"
        );
    }
    assert!(world.prayer_is_on(raised.varp));
}

#[test]
fn allocation_counts_by_tick_class() {
    if !api::test_support::game_data_available() {
        return;
    }
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

#[test]
fn native_chat_death_aborts_a_guard_once_at_restored_hp_even_while_held() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = until_combat(&mut script, &mut world, &mut ledger, 1);
    let original_token = script.token.expect("guarded clue token");
    assert!(matches!(script.fight, Some(Fight::Combat(_))));

    // The first observed death signal arrives after effective HP is restored.
    world.set_hitpoints(40);
    world
        .snapshot
        .seed_chat_lines(vec![chat_line(1, "Oh dear, you are dead!")]);
    world.held = true;
    let (flow, interactions) = drive(&mut script, &world, &mut ledger, begun + 1);
    assert_eq!(flow.expect("death tick"), ScriptFlow::Continue);
    assert!(interactions.is_empty(), "death emits no clue work");
    assert!(
        script.fight.is_none(),
        "death cancels the active guard action"
    );
    assert!(script.token.is_none(), "death ends the current clue token");
    assert_eq!(script.solved, 0, "death is not a solved clue");
    assert!(
        !live_owner(&ledger),
        "death revokes the guard's action owner"
    );

    // The same ring entry is a duplicate, not a second death. The held clue
    // returns to normal processing on the next frame with a fresh token.
    world.held = false;
    let (flow, _) = drive(&mut script, &world, &mut ledger, begun + 2);
    flow.expect("post-death clue tick");
    let resumed_token = script.token.expect("the held clue starts again");
    assert_ne!(resumed_token, original_token, "the old token stays ended");
    assert_eq!(script.solved, 0);
}

#[test]
fn native_hp_zero_without_death_chat_freezes_and_resumes_the_live_guard() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = until_combat(&mut script, &mut world, &mut ledger, 1);
    let token = script.token.expect("guarded clue token");
    assert!(matches!(script.fight, Some(Fight::Combat(_))));

    world.set_hitpoints(0);
    let (flow, interactions) = drive(&mut script, &world, &mut ledger, begun + 1);
    assert_eq!(flow.expect("HP-zero tick"), ScriptFlow::Continue);
    assert!(interactions.is_empty(), "zero HP emits no work");
    assert_eq!(script.token, Some(token), "HP alone does not end the token");
    assert!(
        matches!(script.fight, Some(Fight::Combat(_))),
        "guard stays paused"
    );
    assert_eq!(script.solved, 0, "HP alone does not solve the clue");

    world.set_hitpoints(40);
    let (flow, _) = drive(&mut script, &world, &mut ledger, begun + 2);
    flow.expect("recovered tick");
    assert_eq!(script.token, Some(token), "recovery resumes the same clue");
    assert_eq!(script.solved, 0);
}

#[test]
fn native_hp_zero_with_an_incomplete_stat_page_keeps_the_live_guard() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    let mut world = World::new(0);
    let mut script = script(&world, false);
    let mut ledger = None;
    let begun = 1;
    let (flow, _) = drive(&mut script, &world, &mut ledger, begun);
    flow.expect("begin guarded clue tick");
    let token = script.token.expect("guarded clue token");
    world.set_hitpoints(0);
    let original_attack = {
        let attack = world
            .stats
            .iter_mut()
            .find(|stat| stat.name == "attack")
            .expect("attack stat");
        let original = (attack.base, attack.used);
        attack.base = 0;
        attack.used = true;
        original
    };
    world.refresh();
    let (flow, interactions) = drive(&mut script, &world, &mut ledger, begun + 1);
    assert_eq!(
        flow.expect("partial-stat HP-zero tick"),
        ScriptFlow::Continue
    );
    assert!(interactions.is_empty(), "zero HP emits no work");
    assert_eq!(
        script.token,
        Some(token),
        "unrelated stat gaps do not end the clue"
    );
    assert_eq!(script.solved, 0);

    let attack = world
        .stats
        .iter_mut()
        .find(|stat| stat.name == "attack")
        .expect("attack stat");
    attack.base = original_attack.0;
    attack.used = original_attack.1;
    world.set_hitpoints(40);
    let (flow, _) = drive(&mut script, &world, &mut ledger, begun + 2);
    flow.expect("complete-stat recovered tick");
    assert_eq!(script.token, Some(token), "recovery resumes the same clue");
    assert_eq!(script.solved, 0);
}

#[test]
fn native_death_latch_survives_inactive_reconnect_and_replaced_chat_ring() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    let mut world = World::new(0);
    world
        .snapshot
        .seed_chat_lines(vec![chat_line(10, "Welcome back")]);
    let mut script = script(&world, false);
    let mut ledger = None;
    let _ = drive(&mut script, &world, &mut ledger, 1);
    let token = script.token.expect("guarded clue token");

    world.run.session += 1;
    world.snapshot.seed_ingame(0);
    let (flow, interactions) = drive(&mut script, &world, &mut ledger, 2);
    assert_eq!(flow.expect("inactive reconnect tick"), ScriptFlow::Continue);
    assert!(interactions.is_empty(), "inactive frames emit no work");
    assert_eq!(
        script.token,
        Some(token),
        "inactive frame preserves the clue"
    );

    // A replacement chat ring has a lower sequence head. The shared latch
    // reconciles it and consumes the new death line once on the resumed run.
    world.refresh();
    world
        .snapshot
        .seed_chat_lines(vec![chat_line(1, "Oh dear, you are dead!")]);
    let (flow, interactions) = drive(&mut script, &world, &mut ledger, 3);
    assert_eq!(flow.expect("reconnected death tick"), ScriptFlow::Continue);
    assert!(interactions.is_empty(), "death emits no clue work");
    assert!(
        script.token.is_none(),
        "reconnected death ends the live token"
    );
    assert_eq!(script.solved, 0);
}

fn start_sherlock_slot(world: &World) -> script::slot::SlotScript {
    script::registry::install_cards(&crate::compat::sherlock::CARDS)
        .expect("install canonical Sherlock card for native slot tests");
    let mut slot = script::slot::SlotScript::new();
    slot.bind_incarnation(1);
    slot.start_compiled(
        "alice",
        CARD.id,
        Arc::new(SettingsBag::new()),
        Arc::clone(&world.data),
        Arc::new(api::named_banks::NamedBankFacts::empty()),
        None,
    )
    .expect("start Sherlock through its compiled-card slot path");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            script::slot::StartPoll::Settled(script::slot::StartOutcome::Ready) => break,
            script::slot::StartPoll::Pending => {
                assert!(Instant::now() < deadline, "Sherlock preparation stalled");
                std::thread::yield_now();
            }
            outcome => panic!("unexpected Sherlock Start outcome: {outcome:?}"),
        }
    }
    slot
}

fn tick_sherlock_slot(slot: &mut script::slot::SlotScript, world: &World, tick: u64) {
    let mut driver = script::ctx::test_support::NullDriver::default();
    slot.on_game_tick(&mut script::ctx::ScriptCtx {
        driver: &mut driver,
        tick,
        here: Some(world.here),
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(&world.snapshot),
        obj_names: None,
        compiled: script::CompiledTick {
            condition: None,
            poison_unknown_pulse: None,
            walk_danger: None,
            wilderness: None,
            pvp_globals: None,
            cure_poison: script::survival::CurePoison::WhenNeeded,
            selected: Some(&world.data),
            reach: None,
            reach_flood: None,
            bank_memory: None,
            shop_keepers: None,
            collision: None,
            nav: None,
            hold: world.held,
            world_members: api::selected::Truth::Unknown,
            interacts: None,
            service_gate: None,
        },
    });
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClueCombatCaller {
    Guardian,
    Keeper,
}

impl ClueCombatCaller {
    fn label(self) -> &'static str {
        match self {
            Self::Guardian => "guardian",
            Self::Keeper => "keeper",
        }
    }
}

fn keeper_retry_world() -> World {
    let data = data();
    let key = data
        .talk_key()
        .expect("selected talk-key data")
        .keys
        .iter()
        .find(|key| {
            key.spawn.is_some()
                && key.keeper.kind == "type"
                && key.keeper.id.is_some()
                && key
                    .keeper
                    .name
                    .as_deref()
                    .is_some_and(|name| !name.is_empty())
        })
        .expect("a spawned, named keeper type row");
    let clue_id = key.id;
    let keeper_id = key.keeper.id.expect("packed keeper type");
    let keeper_name = key
        .keeper
        .name
        .as_deref()
        .expect("selected keeper display name")
        .to_owned();
    let spawn = key.spawn.as_ref().expect("published keeper spawn");
    let stand = WorldTile {
        x: spawn.x,
        z: spawn.z,
        level: spawn.plane,
    };
    let far = WorldTile {
        x: stand.x + 10,
        z: stand.z,
        level: stand.level,
    };

    let mut world = World::new(0);
    world.stand = stand;
    world.here = (stand.x, stand.z, stand.level);
    world.inventory[0] = item(clue_id, "Clue scroll", 0, 1);
    world.local.player.network = far;
    world.local.player.actor.tile = far;
    world.npcs[0].r#type = Some(usize::try_from(keeper_id).expect("positive keeper type"));
    world.npcs[0].name = Some(keeper_name);
    world.npcs[0].actions = vec![Some("Attack".into())];
    world.npcs[0].tile = stand;
    world.npcs[0].network = stand;
    world.npcs[0].distance = 1;
    world.refresh();
    world
}

struct NativeStandWalk {
    authority: script::native::HostAuthority,
    request_id: u64,
    target: WorldTile,
    required_after_tick: u64,
}

#[derive(Default)]
struct SherlockNativePass {
    walks: Vec<NativeStandWalk>,
    attacks: usize,
    other_native_effects: usize,
}

fn is_attack(request: &InteractReq) -> bool {
    matches!(request, InteractReq::Npc { action, .. } if action == "Attack")
}

fn sherlock_native_pass(
    slot: &mut script::slot::SlotScript,
    world: &World,
    tick: u64,
) -> SherlockNativePass {
    tick_sherlock_slot(slot, world, tick);
    let mut pass = SherlockNativePass::default();
    while let Some(action) = slot.take_native_action() {
        let authority = action.authority();
        let request_id = action.request_id.get();
        let evidence = EvidenceStamp {
            run: action.run(),
            tick,
            sequence: tick,
        };
        match action.effect {
            HostEffect::Walk(request) => pass.walks.push(NativeStandWalk {
                authority,
                request_id,
                target: request.target,
                required_after_tick: request.required_after.tick,
            }),
            HostEffect::Interaction(request) => {
                if is_attack(&request) {
                    pass.attacks += 1;
                }
                slot.complete_native_interaction(
                    &authority,
                    InteractionReceipt {
                        request_id,
                        evidence,
                        accepted: true,
                        chat_since: 0,
                    },
                );
            }
            _ => pass.other_native_effects += 1,
        }
    }
    pass.attacks += slot
        .drain_interacts()
        .iter()
        .filter(|request| is_attack(request))
        .count();
    pass
}

fn post_stand_walk_failure(
    slot: &mut script::slot::SlotScript,
    walk: &NativeStandWalk,
    observed_tick: u64,
    end: script::native::WalkEnd,
    detail: Option<&str>,
    refusal: Option<script::native::WalkRefusal>,
) -> Option<String> {
    let receipt = script::native::WalkReceipt {
        request_id: walk.request_id,
        evidence: EvidenceStamp {
            run: walk.authority.run(),
            tick: observed_tick,
            sequence: observed_tick,
        },
        goal: None,
        end,
        blocked: None,
        detail: detail.map(Arc::from),
        refusal,
        assessment: None,
        escape: None,
        prereq_allow: None,
    };
    let failure_detail = receipt.failure_detail().map(|detail| detail.to_string());
    if let Some(detail) = detail {
        assert_eq!(failure_detail.as_deref(), Some(detail));
    }
    slot.complete_native_walk(&walk.authority, receipt);
    failure_detail
}

fn begin_clue_stand_walk(
    caller: ClueCombatCaller,
) -> (World, script::slot::SlotScript, NativeStandWalk) {
    crate::compat::clue::on_reset();
    let mut world = match caller {
        ClueCombatCaller::Guardian => World::new(0),
        ClueCombatCaller::Keeper => keeper_retry_world(),
    };
    let mut slot = start_sherlock_slot(&world);
    let far = WorldTile {
        x: world.stand.x + 10,
        z: world.stand.z,
        level: world.stand.level,
    };
    let mut first_walk = None;
    for tick in 1..=8 {
        if caller == ClueCombatCaller::Guardian && tick == 2 {
            // The clue's posted `here` remains its arrived stand; the local
            // network tile is a distinct native observation for Combat.
            world.local.player.network = far;
            world.local.player.actor.tile = far;
            world.refresh();
        }
        let pass = sherlock_native_pass(&mut slot, &world, tick);
        assert_eq!(
            pass.other_native_effects,
            0,
            "{} emitted an unexpected native effect before the stand walk",
            caller.label()
        );
        assert_eq!(
            pass.attacks,
            0,
            "{} attacked before reaching the native stand",
            caller.label()
        );
        if !pass.walks.is_empty() {
            assert_eq!(
                pass.walks.len(),
                1,
                "{} emitted more than one stand walk in one tick",
                caller.label()
            );
            first_walk = pass.walks.into_iter().next();
            break;
        }
    }
    let first_walk = first_walk.unwrap_or_else(|| {
        panic!(
            "{} Delegation::request did not reach the native stand planner",
            caller.label()
        )
    });
    assert_eq!(
        first_walk.target,
        world.stand,
        "{} walks to its Delegation::request stand",
        caller.label()
    );
    (world, slot, first_walk)
}

/// Sherlock has two production `begin::<Combat>` sites:
/// `compat/sherlock.rs:408` retries the request stored from `Delegation` by
/// `begin_combat` at `compat/sherlock.rs:860`, and
/// `compat/sherlock.rs:844` starts a fresh `d.request()`. Both are one clue
/// caller family; the pending branch is not a distinct kind. The policy helper
/// above uses a default request with no stand. These compiled-card checks cover
/// the only stand-bearing caller's actual guardian and keeper delegations.
#[test]
fn clue_guardian_and_keeper_stand_walks_retry_three_times_and_stop_on_refusal() {
    if !api::test_support::game_data_available() {
        return;
    }
    for caller in [ClueCombatCaller::Guardian, ClueCombatCaller::Keeper] {
        let (world, mut slot, mut walk) = begin_clue_stand_walk(caller);
        let details = [
            "first stand route failed",
            "second stand route failed",
            "last stand route failed",
        ];
        let mut request_ids = vec![walk.request_id];
        for (attempt, &detail) in details.iter().enumerate() {
            let observed_tick = walk.required_after_tick + 1;
            let failure_detail = post_stand_walk_failure(
                &mut slot,
                &walk,
                observed_tick,
                script::native::WalkEnd::NoPath,
                Some(detail),
                None,
            );
            if attempt == details.len() - 1 {
                assert_eq!(
                    failure_detail.as_deref(),
                    Some("last stand route failed"),
                    "{} final receipt retains its own failure detail",
                    caller.label()
                );
                let terminal = sherlock_native_pass(&mut slot, &world, observed_tick);
                assert!(
                    terminal.walks.is_empty(),
                    "{} re-walked after the third failure",
                    caller.label()
                );
                assert_eq!(
                    terminal.attacks,
                    0,
                    "{} attacked after the third failure",
                    caller.label()
                );
                assert!(
                    !slot.live_native_owner(),
                    "{} retained the Combat owner after exhausting stand walks",
                    caller.label()
                );
                assert_eq!(
                    slot.native_status().map(|status| status.phase),
                    Some(NativePhase::Blocked),
                    "{} exposes terminal failure after the third stand receipt",
                    caller.label()
                );
                break;
            }

            let failed_tick = sherlock_native_pass(&mut slot, &world, observed_tick);
            assert!(
                failed_tick.walks.is_empty(),
                "{} retried in the receipt tick instead of waiting for a later tick",
                caller.label()
            );
            assert_eq!(failed_tick.attacks, 0);
            let retry_tick = observed_tick + 1;
            let retry = sherlock_native_pass(&mut slot, &world, retry_tick);
            assert_eq!(
                retry.walks.len(),
                1,
                "{} retry walk {attempt}",
                caller.label()
            );
            assert_eq!(retry.other_native_effects, 0);
            assert_eq!(
                retry.attacks,
                0,
                "{} attacked while retrying",
                caller.label()
            );
            let next = retry.walks.into_iter().next().expect("one retry walk");
            assert_eq!(next.target, world.stand);
            assert_eq!(
                next.required_after_tick,
                retry_tick,
                "{} retry is scheduled from the later observed tick",
                caller.label()
            );
            assert!(
                !request_ids.contains(&next.request_id),
                "{} retry reused a prior host walk request id",
                caller.label()
            );
            request_ids.push(next.request_id);
            walk = next;
        }
        assert_eq!(
            request_ids.len(),
            3,
            "{} must make exactly three total stand walks",
            caller.label()
        );
        slot.stop();

        let (world, mut slot, walk) = begin_clue_stand_walk(caller);
        let observed_tick = walk.required_after_tick + 1;
        let detail = post_stand_walk_failure(
            &mut slot,
            &walk,
            observed_tick,
            script::native::WalkEnd::Failed,
            None,
            Some(script::native::WalkRefusal::Unsurvivable),
        );
        assert_eq!(
            detail.as_deref(),
            Some("route is unsurvivable"),
            "{} typed refusal detail",
            caller.label()
        );
        let terminal = sherlock_native_pass(&mut slot, &world, observed_tick);
        assert!(
            terminal.walks.is_empty(),
            "{} retried a typed hard refusal",
            caller.label()
        );
        assert_eq!(
            terminal.attacks,
            0,
            "{} attacked after a typed hard refusal",
            caller.label()
        );
        assert!(
            !slot.live_native_owner(),
            "{} retained the Combat owner after a typed hard refusal",
            caller.label()
        );
        assert_eq!(
            slot.native_status().map(|status| status.phase),
            Some(NativePhase::Blocked),
            "{} exposes terminal failure after a typed refusal",
            caller.label()
        );
        let after = sherlock_native_pass(&mut slot, &world, observed_tick + 1);
        assert!(
            after.walks.is_empty(),
            "{} emitted a later retry after refusal",
            caller.label()
        );
        assert_eq!(after.attacks, 0);
        slot.stop();
    }
    crate::compat::clue::on_reset();
}

fn settle_sherlock_restart(slot: &mut script::slot::SlotScript) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while slot.state() == script::slot::RunState::Starting {
        assert!(Instant::now() < deadline, "Sherlock recreation stalled");
        slot.observe_lifecycle();
        std::thread::yield_now();
    }
    assert_eq!(slot.state(), script::slot::RunState::Running);
}

#[test]
fn native_sherlock_watchdog_recreation_observes_death_from_gap_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    let mut world = World::new(0);
    world
        .snapshot
        .seed_chat_lines(vec![chat_line(10, "Welcome back")]);
    let mut slot = start_sherlock_slot(&world);
    tick_sherlock_slot(&mut slot, &world, 1);
    assert_eq!(
        slot.native_status().map(|status| status.phase),
        Some(NativePhase::Working),
        "the original clue session is live before recreation"
    );
    let _ = slot.drain_interacts();

    // The death arrives after the watchdog has dropped the old card but
    // before the recreated card sees its first frame.
    slot.restart_from_identity(Instant::now())
        .expect("watchdog recreation");
    world.snapshot.seed_chat_lines(vec![
        chat_line(11, "Oh dear, you are dead!"),
        chat_line(10, "Welcome back"),
    ]);
    settle_sherlock_restart(&mut slot);
    tick_sherlock_slot(&mut slot, &world, 2);
    assert!(
        slot.drain_interacts().is_empty(),
        "the gap death suppresses work from a restarted clue"
    );
    assert_eq!(
        slot.native_status().map(|status| status.phase),
        Some(NativePhase::Waiting),
        "the recreated card observes the death instead of baselining it"
    );

    tick_sherlock_slot(&mut slot, &world, 3);
    assert_eq!(
        slot.native_status().map(|status| status.phase),
        Some(NativePhase::Working),
        "the gap death is consumed once and does not suppress later work"
    );
    slot.stop();
}

#[test]
fn native_sherlock_fresh_start_baselines_stale_death_chat() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::compat::clue::on_reset();
    let mut world = World::new(0);
    world
        .snapshot
        .seed_chat_lines(vec![chat_line(10, "Oh dear, you are dead!")]);
    let mut slot = start_sherlock_slot(&world);
    tick_sherlock_slot(&mut slot, &world, 1);
    assert_eq!(
        slot.native_status().map(|status| status.phase),
        Some(NativePhase::Working),
        "a new Start baselines old chat rather than replaying its death"
    );
    slot.stop();
}
