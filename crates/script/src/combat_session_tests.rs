//! Native-ledger checks of the `api.combat` card. These call [`Script::tick`]
//! with the shared Combat machine over a seeded snapshot.
use super::*;
use crate::native::{
    ledger, HostEffect, InteractionReceipt, NativeActions, NativeOutput, RetainedMemory,
};
use crate::shim::InteractReq;
use api::game_data::SelectedGameData;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{
    ActorTargetView, ActorView, ChatLineView, GameSnapshot, HitmarkView, HitmarksView,
    LocalPlayerView, NpcView, PlayerView, SnapshotView, StatView, VarpView, WorldStateView,
    WorldTile,
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Statuses(Vec<ScriptStatus>);
impl NativeOutput for Statuses {
    fn status(&mut self, status: ScriptStatus) {
        self.0.push(status);
    }
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, _: &str) {}
    fn settings_applied(&mut self, _: u64) {}
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

const STAND: WorldTile = WorldTile {
    x: 3222,
    z: 3218,
    level: 0,
};

/// One bot beside one npc of a selected config, with every prayer row posted.
pub(crate) struct World {
    pub(crate) data: Arc<SelectedGameData>,
    pub(crate) snapshot: GameSnapshot,
    npcs: Vec<NpcView>,
    players: Vec<PlayerView>,
    varps: Vec<VarpView>,
    stats: Vec<StatView>,
    local: LocalPlayerView,
    attacking: bool,
    pub(crate) held: bool,
}

impl World {
    pub(crate) fn new(npc_config: &str) -> Self {
        let data = api::game_data::for_revision(ClientRevision::R289).expect("selected data");
        let row = data.npc_by_config(npc_config).expect("selected npc");
        let at = WorldTile {
            x: STAND.x + 1,
            ..STAND
        };
        let npc = NpcView {
            index: 7,
            r#type: Some(usize::try_from(row.id).expect("positive npc id")),
            name: row.display.clone(),
            actions: vec![Some("Attack".into())],
            tile: at,
            distance: 1,
            animation: -1,
            animation_frame: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 8,
            total_health: 8,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 2,
            size: 1,
            network: at,
            x: 0,
            z: 0,
            yaw: 0,
        };
        let varps = data
            .prayers()
            .iter()
            .map(|row| VarpView {
                index: row.varp,
                value: 0,
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
                effective: if index == 5 { 43 } else { 40 },
                base: if index == 5 { 43 } else { 40 },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect();
        let local = LocalPlayerView {
            player: PlayerView {
                index: 1,
                network: STAND,
                actor: actor(STAND),
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
            npcs: vec![npc],
            players: Vec::new(),
            varps,
            stats,
            local,
            attacking: false,
            held: false,
        };
        world.refresh();
        world
    }

    pub(crate) fn npc_type(&self) -> i32 {
        i32::try_from(self.npcs[0].r#type.unwrap()).unwrap()
    }

    pub(crate) fn refresh(&mut self) {
        self.snapshot.seed_ingame(2);
        self.snapshot.seed_world(WorldStateView {
            map_base_x: STAND.x - 40,
            map_base_z: STAND.z - 40,
            members: true,
            ..WorldStateView::default()
        });
        self.snapshot.seed_stats(self.stats.clone());
        self.snapshot.seed_varps(self.varps.clone());
        self.snapshot.seed_inventory(Vec::new(), 28);
        self.snapshot.seed_equipment(Vec::new());
        self.snapshot.seed_npcs(self.npcs.clone());
        self.snapshot.seed_players(self.players.clone());
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
        let projectiles = if self.attacking {
            vec![api::snapshot::ProjectileView {
                spotanim: 9,
                level: 0,
                src: self.npcs[0].tile,
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: self.local.player.index,
                }),
                t1: 0,
                t2: 30,
            }]
        } else {
            Vec::new()
        };
        self.snapshot.seed_projectiles(projectiles);
    }

    /// The npc is fighting the bot and a classified missile is incoming, so
    /// Combat wants Protect from Missiles.
    pub(crate) fn attack(&mut self) {
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
        self.attacking = true;
        self.refresh();
    }

    pub(crate) fn prayer(&self, name: &str) -> api::game_data::PrayerFact {
        self.data
            .prayer_by_name(name)
            .expect("selected prayer")
            .clone()
    }

    pub(crate) fn set_prayer(&mut self, varp: i32, on: bool) {
        self.varps
            .iter_mut()
            .find(|row| row.index == varp)
            .expect("selected prayer varp")
            .value = i32::from(on);
        self.refresh();
    }

    pub(crate) fn prayer_is_on(&self, varp: i32) -> bool {
        self.varps
            .iter()
            .any(|row| row.index == varp && row.value != 0)
    }

    pub(crate) fn die(&mut self) {
        self.stats
            .iter_mut()
            .find(|stat| stat.index == 3)
            .expect("hitpoints")
            .effective = 0;
        for row in &mut self.varps {
            row.value = 0;
        }
        self.refresh();
        self.snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "Oh dear, you are dead!".into(),
            sequence: 1,
        }]);
    }

    /// Another player stands next to the bot and is attacking nobody.
    fn bystander(&mut self) {
        self.npcs.clear();
        let at = WorldTile {
            x: STAND.x - 1,
            ..STAND
        };
        self.players = vec![PlayerView {
            index: 2,
            network: at,
            actor: ActorView {
                name: Some("bob".into()),
                ..actor(at)
            },
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        }];
        self.refresh();
    }
}

fn with_tick<R>(
    world: &World,
    ledger: &mut Option<Box<ledger::Ledger>>,
    output: &mut Statuses,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    let pin = world.data.selected_pin().unwrap();
    let evidence = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: tick,
    };
    let mut retained = RetainedMemory::default();
    let mut budget = ledger::TickBudget::default();
    budget.observe(tick);
    let mut actions = NativeActions { _private: () };
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
            eligible: !world.held,
        },
        output,
        pairs: None,
        frame: crate::native::HostFrame {
            here: Some((STAND.x, STAND.z, STAND.level)),
            snapshot: Some(&world.snapshot),
            obj_names: None,
            compiled: crate::CompiledTick {
                selected: Some(&world.data),
                reach: None,
                reach_flood: None,
                bank_memory: None,
                collision: None,
                hold: world.held,
                world_members: api::selected::Truth::Unknown,
                interacts: Some(Vec::new()),
            },
        },
    };
    f(&mut native)
}

struct Harness {
    world: World,
    card: CombatSessionCard,
    cell: SettleCell,
    ledger: Option<Box<ledger::Ledger>>,
    output: Statuses,
}

impl Harness {
    fn new(world: World, target: crate::combat::Target) -> Self {
        let tables = CombatTables::build(Arc::clone(&world.data)).expect("tables");
        let request = CombatRequest {
            target,
            ..CombatRequest::default()
        };
        let cell = SettleCell::default();
        let card = CombatSessionCard::new(Arc::new(request), tables, Arc::clone(&cell));
        Self {
            world,
            card,
            cell,
            ledger: None,
            output: Statuses::default(),
        }
    }

    fn npc(world: World) -> Self {
        let target = crate::combat::Target::Npc {
            types: Arc::from([world.npc_type()]),
            pick: crate::combat::Pick::Nearest,
            not_targeting_others: true,
        };
        Self::new(world, target)
    }

    fn tick(&mut self, tick: u64) -> ScriptFlow {
        let card = &mut self.card;
        with_tick(
            &self.world,
            &mut self.ledger,
            &mut self.output,
            tick,
            |native| card.tick(native),
        )
        .expect("card tick")
    }

    fn buttons(&self) -> Vec<i32> {
        self.ledger
            .as_ref()
            .map(|ledger| {
                ledger
                    .outbox
                    .iter()
                    .filter_map(|action| match &action.effect {
                        HostEffect::Interaction(InteractReq::IfButton { component_id }) => {
                            Some(*component_id)
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn outbox(&self) -> Vec<InteractReq> {
        self.ledger
            .as_ref()
            .map(|ledger| {
                ledger
                    .outbox
                    .iter()
                    .filter_map(|action| match &action.effect {
                        HostEffect::Interaction(request) => Some(request.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn accept(&mut self, tick: u64) {
        let Some(ledger) = self.ledger.as_mut() else {
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

    fn settled(&self) -> Option<Settle> {
        self.cell.lock().unwrap().clone()
    }

    /// Drive until Combat's raise of `raised` is in the outbox, accept it and
    /// observe it on. Returns the next tick.
    fn until_raised(&mut self, raised: &api::game_data::PrayerFact) -> u64 {
        for tick in 1..20 {
            assert_eq!(self.tick(tick), ScriptFlow::Continue);
            let raise = self.buttons().contains(&raised.button_com);
            self.accept(tick);
            if raise {
                self.world.set_prayer(raised.varp, true);
                self.world.attack();
                assert_eq!(self.tick(tick + 1), ScriptFlow::Continue);
                self.accept(tick + 1);
                return tick + 2;
            }
        }
        panic!("Combat never raised {}", raised.name);
    }
}

#[test]
fn pause_clears_only_the_bot_raised_prayer_and_settles_interrupted() {
    let mut world = World::new("imp");
    let skin = world.prayer("Thick Skin");
    let raised = world.prayer("Protect from Missiles");
    world.set_prayer(skin.varp, true);
    world.attack();
    let mut harness = Harness::npc(world);
    let mut tick = harness.until_raised(&raised);
    assert!(harness.card.prayer_cleanup().contains(raised.varp));
    assert!(!harness.card.prayer_cleanup().contains(skin.varp));

    harness.card.interrupt(Interrupt::Pause);
    let mut cleared = Vec::new();
    while harness.settled().is_none() {
        assert!(tick < 40, "pause never settled");
        harness.tick(tick);
        cleared.extend(harness.buttons());
        if harness.buttons().contains(&raised.button_com) {
            harness.accept(tick);
            harness.world.set_prayer(raised.varp, false);
        }
        tick += 1;
    }
    assert_eq!(cleared, vec![raised.button_com], "only the bot's raise");
    assert_eq!(
        harness.settled(),
        Some(Settle::Interrupted(InterruptCause::Pause))
    );
    assert!(harness.world.prayer_is_on(skin.varp), "user prayer kept");
    assert!(!harness.world.prayer_is_on(raised.varp));
    assert!(harness.card.prayer_cleanup().is_empty());
    assert_eq!(harness.tick(tick), ScriptFlow::Complete);
}

#[test]
fn death_retires_the_obligation_without_clicking_prayers() {
    let mut world = World::new("imp");
    let raised = world.prayer("Protect from Missiles");
    world.attack();
    let mut harness = Harness::npc(world);
    let tick = harness.until_raised(&raised);
    assert!(harness.card.prayer_cleanup().contains(raised.varp));
    harness.world.die();
    assert_eq!(harness.tick(tick), ScriptFlow::Complete);
    assert_eq!(
        harness.settled(),
        Some(Settle::Interrupted(InterruptCause::Died))
    );
    assert!(harness.buttons().is_empty(), "death clicks nothing");
    assert!(
        harness.card.prayer_cleanup().is_empty(),
        "nothing is handed to the host after death"
    );
}

#[test]
fn an_unsettled_stop_reports_the_accepted_raise_for_the_host() {
    let mut world = World::new("imp");
    let skin = world.prayer("Thick Skin");
    let raised = world.prayer("Protect from Missiles");
    world.set_prayer(skin.varp, true);
    world.attack();
    let mut harness = Harness::npc(world);
    harness.until_raised(&raised);
    let owed = harness.card.prayer_cleanup();
    assert!(owed.contains(raised.varp));
    assert!(!owed.contains(skin.varp), "user prayers are never owed");
}

#[test]
fn attackers_players_never_initiates_against_a_player_who_is_not_attacking() {
    let mut world = World::new("imp");
    world.bystander();
    let mut harness = Harness::new(
        world,
        crate::combat::Target::Attacker {
            npcs: false,
            players: true,
        },
    );
    let mut tick = 1;
    while harness.settled().is_none() {
        assert!(tick < 60, "no-target never settled");
        harness.tick(tick);
        assert!(
            !harness
                .outbox()
                .iter()
                .any(|request| matches!(request, InteractReq::Player { .. })),
            "Combat initiated against a non-attacking player"
        );
        harness.accept(tick);
        tick += 1;
    }
    let Some(Settle::Fought(report)) = harness.settled() else {
        panic!("expected a fought report: {:?}", harness.settled());
    };
    assert_eq!(report.end, crate::api_combat::ReportEnd::NoTarget);
    assert_eq!(report.swings, 0);
}

#[test]
fn a_target_that_never_appears_settles_the_machine_report() {
    let mut world = World::new("imp");
    world.npcs.clear();
    world.refresh();
    let npc = world.data.npc_by_config("imp").unwrap().id;
    let mut harness = Harness::new(
        world,
        crate::combat::Target::Npc {
            types: Arc::from([npc]),
            pick: crate::combat::Pick::Nearest,
            not_targeting_others: true,
        },
    );
    let mut tick = 1;
    while harness.settled().is_none() {
        assert!(tick < 60, "never settled");
        harness.tick(tick);
        harness.accept(tick);
        tick += 1;
    }
    assert!(matches!(
        harness.settled(),
        Some(Settle::Fought(CombatSummary {
            end: crate::api_combat::ReportEnd::NoTarget,
            ..
        }))
    ));
    let stages: Vec<_> = harness
        .output
        .0
        .iter()
        .map(|status| match &status.fields[0].value {
            StatusValue::Text(text) => text.to_string(),
            other => panic!("stage is text: {other:?}"),
        })
        .collect();
    assert!(stages.contains(&"fighting".to_string()), "{stages:?}");
    assert!(
        stages.windows(2).all(|pair| pair[0] != pair[1]),
        "status is published only on change: {stages:?}"
    );
}

#[test]
fn a_held_frame_neither_begins_nor_settles() {
    let mut world = World::new("imp");
    world.held = true;
    let mut harness = Harness::npc(world);
    for tick in 1..5 {
        assert_eq!(harness.tick(tick), ScriptFlow::Continue);
    }
    assert!(harness
        .ledger
        .as_ref()
        .is_none_or(|ledger| ledger.owner.is_none()));
    assert!(harness.settled().is_none());
    // A Pause before Combat began still settles, with nothing to clear.
    harness.card.interrupt(Interrupt::Pause);
    harness.world.held = false;
    assert_eq!(harness.tick(5), ScriptFlow::Complete);
    assert_eq!(
        harness.settled(),
        Some(Settle::Interrupted(InterruptCause::Pause))
    );
}

/// The card lives only while a session is live; the Combat machine is inline
/// (as in Sherlock), so one live session costs one heap block of this size.
#[test]
fn card_stays_compact() {
    let size = std::mem::size_of::<CombatSessionCard>();
    println!("api.combat live card={size}B");
    assert!(size <= 1024);
}
