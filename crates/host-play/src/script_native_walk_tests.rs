//! Typed native walks through the real host pump: `script_observe`'s receipt
//! delivery and native drain, the off-pump route worker and `step_nav_bot`.
use super::*;
use client::dash3d::CollisionFlag;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkEnd, WalkEvent,
    WalkEventKind, WalkReceipt, WalkRequest,
};
use std::task::Poll;

#[derive(Default)]
struct Walker {
    /// Return `Blocked` from every tick while set.
    blocked: bool,
    /// On the next tick, cancel the walk and walk the same tile again.
    rewalk: bool,
    begun: usize,
    walks: usize,
    cross_first: Vec<Arc<str>>,
    protect: bool,
    disallow_prayer: bool,
    target: Option<WorldTile>,
    later_target: Option<WorldTile>,
    radius: u16,
    arrival: nav::arrival::ArrivalKind,
    result: Option<Result<WalkReceipt, ActionError>>,
    results: Vec<Result<WalkReceipt, ActionError>>,
    events: Vec<WalkEvent>,
}

/// One native `Walk` to `(4, 0, 0)`, begun on the first eligible tick and
/// polled on every later one; the terminal lands in the shared cell.
struct WalkerScript {
    shared: Arc<parking_lot::Mutex<Walker>>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for WalkerScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let mut shared = self.shared.lock();
        if std::mem::take(&mut shared.rewalk) {
            if let Some(handle) = self.handle.take() {
                tick.actions.cancel(handle);
            }
            shared.begun = 0;
        }
        if let Some(handle) = &self.handle {
            while let Some(event) = tick.actions.take_walk_event(handle, &mut tick.cx) {
                shared.events.push(event);
            }
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                shared.result = Some(result.clone());
                shared.results.push(result);
                self.handle = None;
            }
        } else if shared.begun < shared.walks {
            let first_walk = shared.begun == 0;
            let cross = if first_walk {
                shared.cross_first.clone()
            } else {
                Vec::new()
            };
            let target = if first_walk {
                shared.target.unwrap_or(WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                })
            } else {
                shared.later_target.unwrap_or(WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                })
            };
            shared.begun += 1;
            let request = WalkRequest {
                target,
                radius: shared.radius,
                arrival: shared.arrival,
                loc_id: None,
                options: script::FindOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: cross.into_boxed_slice(),
                protect: shared.protect,
                allow: script::native::WalkAllow {
                    prayer: !shared.disallow_prayer,
                },
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => {
                    let result = Err(error);
                    shared.result = Some(result.clone());
                    shared.results.push(result);
                }
            }
        }
        if shared.blocked {
            return Ok(ScriptFlow::Blocked(ScriptFailure {
                code: "test-blocked".into(),
                message: "blocked by the card".into(),
            }));
        }
        Ok(ScriptFlow::Continue)
    }
}

struct Rig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<Arc<NavWorld>>,
    shared: Arc<parking_lot::Mutex<Walker>>,
    driver: NavRec,
    client: client::client::Client,
    snapshot: GameSnapshot,
}

fn rig(world: Option<Arc<NavWorld>>, blocked: bool) -> Rig {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let shared = Arc::new(parking_lot::Mutex::new(Walker {
        blocked,
        walks: 1,
        ..Walker::default()
    }));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(WalkerScript {
                shared: Arc::clone(&shared),
                handle: None,
            }),
            None,
        )
        .unwrap();
    let mut client = nav_client();
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    Rig {
        scripts,
        cheats: Arc::new(Mutex::new(HashMap::new())),
        navs: Arc::new(Mutex::new(HashMap::new())),
        statuses: Arc::new(Mutex::new(vec![SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        }])),
        world,
        shared,
        driver: NavRec::default(),
        client,
        snapshot,
    }
}

fn open_rig(blocked: bool) -> Rig {
    rig(Some(Arc::new(open_world(40, 1))), blocked)
}
#[test]
fn native_follow_failure_preserves_hop_detail_in_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let failure = nav::traveller::TravelOutcome::Stalled {
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        aiming: WorldTile {
            x: 4,
            z: 0,
            level: 0,
        },
        why: nav::traveller::HopFailure::Dropped,
        tries: 3,
    };
    {
        let mut navs = rig.navs.lock().unwrap();
        super::apply_nav_follow_outcome(navs.get_mut("alice").unwrap(), Some(failure), false);
    }
    rig.observe(2);
    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::Failed);
    let detail = receipt
        .detail
        .as_deref()
        .expect("hop failure detail is owed to the caller");
    assert!(detail.contains("Dropped"), "{detail}");
    assert!(detail.contains("tries: 3"), "{detail}");
    assert!(detail.contains("aiming"), "{detail}");
}

#[test]
fn stale_and_legacy_follow_failures_do_not_acquire_native_detail() {
    let failure = || nav::traveller::TravelOutcome::Stalled {
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        aiming: WorldTile {
            x: 4,
            z: 0,
            level: 0,
        },
        why: nav::traveller::HopFailure::Dropped,
        tries: 3,
    };

    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.walk_outcome_detail = Some(Arc::from("newer correlated outcome"));
        bot.walk_request_id = bot.walk_request_id.wrapping_add(1);
        super::apply_nav_follow_outcome(bot, Some(failure()), false);
        assert_eq!(
            bot.walk_outcome_detail.as_deref(),
            Some("newer correlated outcome")
        );
        assert!(bot.native_walk_failure.is_none());
    }

    let mut legacy = NavBot {
        requested_route: Some((
            WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
            1,
            false,
            false,
            false,
            Default::default(),
        )),
        ..Default::default()
    };
    super::apply_nav_follow_outcome(&mut legacy, Some(failure()), false);
    assert!(legacy.native_walk_failure.is_none());
    assert!(
        legacy.walk_outcome_detail.is_none(),
        "a legacy route has no native receipt detail"
    );
}

fn zoned_open_world() -> NavWorld {
    let mut world = open_world(40, 1);
    let zones = vec![nav::zones::Zone::npc(
        WorldTile {
            x: 2,
            z: 0,
            level: 0,
        },
        0,
        nav::zones::ZoneClass::Always,
        u16::MAX,
        0,
    )];
    let kinds = vec![nav::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    let table = nav::zones::ZoneTable::from_parts(
        zones,
        kinds,
        vec![],
        vec![],
        vec![],
        world.collision.origin,
        world.collision.width as u32,
        world.collision.height as u32,
        &world.graph.wilderness,
    )
    .unwrap();
    world.graph.zones = Some(table);
    world
}
fn catalog_open_world() -> NavWorld {
    let mut world = open_world(40, 1);
    let mut zones = vec![
        nav::zones::Zone::npc(
            WorldTile {
                x: 2,
                z: 0,
                level: 0,
            },
            0,
            nav::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav::zones::Zone::npc(
            WorldTile {
                x: 3,
                z: 0,
                level: 0,
            },
            0,
            nav::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
    ];
    zones[0].group = 0;
    zones[1].group = 1;
    let kinds = vec![nav::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    let groups = vec![
        nav::zones::ZoneGroup::new(
            "white-wolf-mountain",
            "White Wolf Mountain",
            nav::router::AvoidRect {
                min_x: 2,
                max_x: 2,
                min_z: 0,
                max_z: 0,
                level: Some(0),
            },
            vec![0].into_boxed_slice(),
        ),
        nav::zones::ZoneGroup::new(
            "draynor-jail-guards",
            "Draynor jail guards",
            nav::router::AvoidRect {
                min_x: 3,
                max_x: 3,
                min_z: 0,
                max_z: 0,
                level: Some(0),
            },
            vec![1].into_boxed_slice(),
        ),
    ];
    let table = nav::zones::ZoneTable::from_parts(
        zones,
        kinds,
        groups,
        vec![],
        vec![],
        world.collision.origin,
        world.collision.width as u32,
        world.collision.height as u32,
        &world.graph.wilderness,
    )
    .unwrap();
    world.graph.zones = Some(table);
    world
}

impl Rig {
    fn observe(&mut self, tick: u64) {
        self.observe_with_here(
            tick,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
        );
    }

    fn rebuild_snapshot_at(&mut self, here: WorldTile) {
        let x = here.x - self.client.map_build_base_x;
        let z = here.z - self.client.map_build_base_z;
        let mut player = client::dash3d::ClientPlayer::at(x, z);
        player.x = x * 128 + player.size * 64;
        player.z = z * 128 + player.size * 64;
        self.client.local_player = Some(player);
        self.client.bump_gens(client::io::ServerProt::PLAYER_INFO);
        self.client
            .bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        self.snapshot.rebuild(&self.client);
    }

    fn observe_at(&mut self, tick: u64, here: WorldTile) {
        self.rebuild_snapshot_at(here);
        self.observe_with_here(tick, here);
    }

    fn observe_with_here(&mut self, tick: u64, here: WorldTile) {
        script_observe(
            &mut self.driver,
            "alice",
            true,
            true,
            tick,
            Some((here.x, here.z, here.level)),
            None,
            None,
            Some(&self.snapshot),
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &self.world,
            false,
            false,
        );
    }

    /// Steps from the snapshot's own (network route-head) tile, the same
    /// position authority the host passes in production.
    fn step(&mut self) {
        step_nav_bot(
            &mut self.driver,
            "alice",
            self.snapshot.tile(),
            &self.snapshot,
            &self.navs,
            &self.statuses,
            self.world.as_ref(),
            false,
            false,
            no_reach,
        );
    }

    fn step_at(&mut self, here: WorldTile) {
        self.rebuild_snapshot_at(here);
        self.step();
    }

    fn walk_armed(&self) -> bool {
        self.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(NavBot::script_walk_armed)
    }

    fn wait_routed(&self) {
        assert!(
            wait_until(5_000, || queued(&self.navs).is_some()),
            "the worker armed the native route"
        );
    }

    fn end(&self) -> Option<Result<WalkEnd, ActionError>> {
        self.shared
            .lock()
            .result
            .clone()
            .map(|result| result.map(|receipt| receipt.end))
    }

    fn slot(&self) -> Arc<Mutex<script::SlotScript>> {
        script_slot(&self.scripts, "alice").unwrap()
    }
}

#[test]
fn a_blocked_tick_dispatches_no_native_walk() {
    let mut rig = open_rig(true);
    rig.observe(1);
    assert_eq!(rig.shared.lock().begun, 1, "the tick began a walk");
    assert_eq!(
        rig.slot().lock().unwrap().state(),
        script::RunState::Idle,
        "terminal Blocked uses Stop, not a running dispatch hold"
    );
    assert_eq!(
        rig.slot().lock().unwrap().native_status().unwrap().phase,
        script::native::NativePhase::Blocked
    );
    assert!(
        !rig.walk_armed(),
        "Blocked closes dispatch: the tick's walk must not reach the navigator"
    );
    rig.step();
    assert_eq!(rig.driver.walked, None, "no hop is sent while Blocked");
}

#[test]
fn blocking_mid_walk_stops_the_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    rig.step();
    assert_eq!(rig.driver.walked, Some((4, 0)));
    rig.shared.lock().blocked = true;
    rig.observe(2);
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Idle);
    assert!(!rig.slot().lock().unwrap().want_run);
    assert!(rig.slot().lock().unwrap().native_run().is_none());
    assert_eq!(
        rig.slot()
            .lock()
            .unwrap()
            .native_status()
            .unwrap()
            .failure
            .as_ref()
            .unwrap()
            .message
            .as_ref(),
        "blocked by the card"
    );
    rig.driver.walked = None;
    rig.step();
    assert_eq!(queued(&rig.navs), None, "Blocked revokes the follow");
    assert_eq!(rig.driver.walked, None);
    assert!(rig
        .slot()
        .lock()
        .unwrap()
        .native_quiet_read(Instant::now())
        .is_none());
}
#[test]
fn terminal_blocked_resets_all_navigation_once_per_generation() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.shared.lock().blocked = true;
    rig.observe(4);
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Idle);
    let generation = rig
        .slot()
        .lock()
        .unwrap()
        .terminal_lifecycle_generation()
        .expect("Blocked records a Failed lifecycle");
    let (route_generation, walk_outcome_seq) = {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.inspect.generation = 9;
        bot.inspect.pending_id = 17;
        bot.bank_pick.posted = script::isolate_fb::BankSelectionInput {
            request_id: 23,
            generation: 4,
            bank_index: 2,
            kind: 1,
        };
        bot.duel_offer_partner = Some("Old partner".into());
        bot.walk_outcome_seq = 31;
        bot.walk_outcome_failed = true;
        bot.walk_outcome_blocked = true;
        (bot.route_generation, bot.walk_outcome_seq)
    };

    rig.observe(5);
    let (reset_route_generation, reset_inspect_generation, reset_walk_outcome_seq) = {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        assert_eq!(bot.route_generation, route_generation.wrapping_add(1));
        assert_eq!(bot.inspect.generation, 10);
        assert_eq!(bot.inspect.pending_id, 0);
        assert_eq!(
            bot.bank_pick.posted,
            script::isolate_fb::BankSelectionInput::default()
        );
        assert_eq!(bot.duel_offer_partner, None);
        assert!(!bot.walk_outcome_failed);
        assert!(!bot.walk_outcome_blocked);
        assert_eq!(bot.walk_outcome_seq, walk_outcome_seq.wrapping_add(1));
        assert!(bot.walk_guard.is_none());
        assert!(bot.walk_guard_off.is_some());
        assert_eq!(bot.terminal_nav_reset_generation, Some(generation));

        let sentinel = script::isolate_fb::BankSelectionInput {
            request_id: 29,
            generation: 5,
            bank_index: 3,
            kind: 2,
        };
        bot.inspect.pending_id = 99;
        bot.bank_pick.posted = sentinel;
        bot.duel_offer_partner = Some("Later partner".into());
        (
            bot.route_generation,
            bot.inspect.generation,
            bot.walk_outcome_seq,
        )
    };
    rig.observe(6);
    let navs = rig.navs.lock().unwrap();
    let bot = navs.get("alice").unwrap();
    assert_eq!(bot.route_generation, reset_route_generation);
    assert_eq!(bot.inspect.generation, reset_inspect_generation);
    assert_eq!(bot.inspect.pending_id, 99);
    assert_eq!(
        bot.bank_pick.posted,
        script::isolate_fb::BankSelectionInput {
            request_id: 29,
            generation: 5,
            bank_index: 3,
            kind: 2,
        }
    );
    assert_eq!(bot.duel_offer_partner.as_deref(), Some("Later partner"));
    assert_eq!(bot.walk_outcome_seq, reset_walk_outcome_seq);
}

#[test]
fn a_native_walk_refused_for_lack_of_a_world_gets_a_refused_receipt() {
    let mut rig = rig(None, false);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
}

fn seed_prayer(snapshot: &mut GameSnapshot, base: i32) {
    snapshot.seed_stats(vec![api::snapshot::StatView {
        index: 5,
        name: "prayer".into(),
        effective: base,
        base,
        xp: 0,
        used: true,
    }]);
}

#[test]
fn a_protected_walk_refuses_when_prayer_cannot_protect() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_prayer(&mut rig.snapshot, 36);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
    assert!(
        !rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.walk_guard.is_some()),
        "a refused protect walk must not arm a driver"
    );
}

#[test]
fn a_protected_walk_arms_the_hold_mode_driver() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_prayer(&mut rig.snapshot, 43);
    rig.observe(1);
    rig.wait_routed();
    assert!(
        rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.walk_guard.is_some()),
        "protect walk must arm WalkGuard before follow"
    );
}

#[test]
fn a_protected_walk_refuses_when_prayer_is_disallowed() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    rig.shared.lock().disallow_prayer = true;
    seed_prayer(&mut rig.snapshot, 43);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
    assert!(
        !rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.walk_guard.is_some()),
        "a prayer-disallowed protect walk must not arm a driver"
    );
}

fn seed_protect_frame(snapshot: &mut GameSnapshot, prayer_base: i32) {
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_stats(
        (0..25)
            .map(|index| api::snapshot::StatView {
                index,
                name: String::new(),
                effective: if index == 5 { prayer_base } else { 40 },
                base: if index == 5 { prayer_base } else { 40 },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect(),
    );
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    snapshot.seed_varps(
        data.prayers()
            .iter()
            .map(|row| api::snapshot::VarpView {
                index: row.varp,
                value: 0,
            })
            .collect(),
    );
    snapshot.seed_players(Vec::new());
    snapshot.seed_hitmarks(api::snapshot::HitmarksView {
        marks: [api::snapshot::HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        }; 4],
        loop_cycle: 0,
    });
}

fn seed_prayer_widgets(snapshot: &mut GameSnapshot) {
    use api::snapshot::{WidgetKind, WidgetRoot, WidgetView};
    snapshot.seed_main_modal(
        5608,
        [5621, 5622, 5623]
            .into_iter()
            .map(|component_id| WidgetView {
                kind: WidgetKind::Widget,
                component_id,
                layer_id: 5608,
                parent_id: 5608,
                root_component_id: 5608,
                root: WidgetRoot::Main,
                type_: 4,
                button_type: 1,
                client_code: 0,
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                scroll_height: 0,
                scroll_position: 0,
                hidden: false,
                text: None,
                alternate_text: None,
                button_text: None,
                target_verb: None,
                target_base: None,
                target_mask: 0,
                model_type: 0,
                model_id: 0,
                alternate_model_type: 0,
                alternate_model_id: 0,
                scripts: None,
                script_comparators: None,
                script_operands: None,
                varp_bindings: Vec::new(),
                colour: 0,
                actions: Vec::new(),
                items: Vec::new(),
            })
            .collect(),
    );
}

fn seed_missile_launch(snapshot: &mut GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let row = data.npc_by_config("ardougne_archer").unwrap();
    let here = snapshot
        .tile()
        .map(|(x, z, level)| api::snapshot::WorldTile { x, z, level })
        .unwrap_or(api::snapshot::WorldTile {
            x: 0,
            z: 0,
            level: 0,
        });
    let npc_tile = api::snapshot::WorldTile {
        x: here.x + 1,
        z: here.z,
        level: here.level,
    };
    let me = snapshot
        .local_player()
        .map(|player| player.player.index)
        .unwrap_or(1);
    snapshot.seed_npcs(vec![api::snapshot::NpcView {
        index: 7,
        r#type: Some(row.id as usize),
        name: row.display.clone(),
        actions: vec![Some("Attack".into())],
        tile: npc_tile,
        distance: 1,
        animation: -1,
        animation_frame: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 50,
        total_health: 50,
        face_entity: -1,
        target: Some(api::snapshot::ActorTargetView {
            kind: api::snapshot::ActorKind::Player,
            index: me,
        }),
        moving: false,
        running: false,
        in_combat: true,
        level: 37,
        size: 1,
        network: npc_tile,
        x: 0,
        z: 0,
        yaw: 0,
    }]);
    snapshot.seed_projectiles(vec![api::snapshot::ProjectileView {
        spotanim: 9,
        level: here.level,
        src: npc_tile,
        target: Some(api::snapshot::ActorTargetView {
            kind: api::snapshot::ActorKind::Player,
            index: me,
        }),
        t1: 0,
        t2: 30,
    }]);
}

fn seed_melee_attack(snapshot: &mut GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let melee_npc = data.npc_by_config("khazard_warlord").unwrap();
    let melee_sequence = data
        .style_seqs()
        .iter()
        .find(|row| row.style & 0x01 != 0)
        .unwrap()
        .seq_id;
    seed_missile_launch(snapshot);
    let mut npcs = snapshot.npcs().to_vec();
    let npc = &mut npcs[0];
    npc.r#type = Some(melee_npc.id as usize);
    npc.name = melee_npc.display.clone();
    npc.in_combat = false;
    npc.animation = melee_sequence;
    npc.animation_frame = 0;
    snapshot.seed_npcs(npcs);
    snapshot.seed_projectiles(Vec::new());
}

fn seed_protect_state(snapshot: &mut GameSnapshot, varp: Option<i32>) {
    let mut rows = snapshot.varps().to_vec();
    for row in &mut rows {
        row.value = i32::from(Some(row.index) == varp);
    }
    snapshot.seed_varps(rows);
}

fn protected_rig() -> Rig {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    rig.snapshot.seed_tick(1);
    rig.observe(1);
    rig.wait_routed();
    rig
}

fn raise_owned_missiles(rig: &mut Rig) {
    rig.snapshot.seed_tick(2);
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_projectiles(Vec::new());
    rig.snapshot.seed_npcs(Vec::new());
    rig.snapshot.seed_tick(3);
    rig.step();
}

#[derive(Clone, Copy, Debug)]
enum GuardEndSeam {
    Stop,
    Pause,
    Cancel,
    Manual,
    CarryHold,
    OwnerRevoked,
}

fn check_guard_end(seam: GuardEndSeam) {
    for was_on in [true, false] {
        let mut rig = protected_rig();
        raise_owned_missiles(&mut rig);
        match seam {
            GuardEndSeam::Stop => {
                rig.slot().lock().unwrap().stop();
                reset_script_nav(&rig.navs, "alice", None);
            }
            GuardEndSeam::Pause => {
                pause_script(&mut rig.slot().lock().unwrap(), &rig.navs, "alice");
            }
            GuardEndSeam::Cancel => abort_script_walk(&rig.navs, "alice"),
            GuardEndSeam::Manual => {
                rig.navs
                    .lock()
                    .unwrap()
                    .get_mut("alice")
                    .unwrap()
                    .cancel_for_manual_input();
            }
            GuardEndSeam::CarryHold => hold_script_nav(&rig.navs, "alice", None),
            GuardEndSeam::OwnerRevoked => {
                rig.slot().lock().unwrap().stop();
                rig.step();
            }
        }
        assert_eq!(
            rig.driver.if_button_components,
            vec![5622],
            "{seam:?} cannot click without the pump driver"
        );
        seed_protect_state(&mut rig.snapshot, was_on.then_some(96));
        rig.snapshot.seed_tick(4);
        rig.step();
        let expected = if was_on { vec![5622, 5622] } else { vec![5622] };
        assert_eq!(
            rig.driver.if_button_components, expected,
            "{seam:?} must discharge only an observed-on owned protect on the next pump"
        );
        rig.step();
        rig.snapshot.seed_tick(5);
        rig.step();
        assert_eq!(
            rig.driver.if_button_components, expected,
            "{seam:?} must not double-toggle a stale on observation"
        );
    }
}

#[test]
fn stop_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Stop);
}

#[test]
fn pause_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Pause);
}

#[test]
fn cancel_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Cancel);
}

#[test]
fn an_owed_off_is_suppressed_when_the_next_observation_is_already_off() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    seed_protect_state(&mut rig.snapshot, None);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "a conditional off must not turn an already-off protect back on"
    );
}

#[test]
fn manual_takeover_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Manual);
}

#[test]
fn carry_hold_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::CarryHold);
}

#[test]
fn owner_revocation_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::OwnerRevoked);
}

fn check_pending_arrival(previous: Option<i32>) {
    let mut rig = protected_rig();
    rig.snapshot.seed_tick(2);
    seed_protect_state(&mut rig.snapshot, previous);
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
    // T+1: the route ends before the enable/switch has been acknowledged.
    let mut client = nav_client();
    client.bump_gens(client::io::ServerProt::PLAYER_INFO);
    nav_snapshot_at(&mut client, &mut rig.snapshot, 4, 0);
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    seed_protect_state(&mut rig.snapshot, previous);
    rig.snapshot.seed_tick(3);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].route.is_none(),
        "T+1 really ends the route before the pending protect is observed"
    );
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "arrival cannot toggle an old style while the new protect is in flight"
    );
    // T+2: the server applies the in-flight protect after the walk ended.
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622],
        "the late acknowledged protect must receive its single off-click"
    );
    rig.step();
    rig.snapshot.seed_tick(5);
    seed_protect_state(&mut rig.snapshot, None);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
}

#[test]
fn arrival_before_protect_acknowledgement_owes_the_late_off() {
    check_pending_arrival(None);
}

#[test]
fn arrival_during_a_pending_protect_switch_does_not_double_toggle() {
    check_pending_arrival(Some(97));
}

#[test]
fn a_missing_prayer_widget_does_not_admit_or_log_a_guard_click() {
    let mut rig = protected_rig();
    seed_missile_launch(&mut rig.snapshot);
    rig.snapshot.seed_main_modal(-1, Vec::new());
    rig.snapshot.seed_tick(2);
    rig.step();
    assert!(rig.driver.if_button_components.is_empty());
    assert!(!rig.navs.lock().unwrap()["alice"]
        .walk_guard
        .as_ref()
        .unwrap()
        .blocks_follow(2));
    seed_prayer_widgets(&mut rig.snapshot);
    rig.snapshot.seed_tick(3);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "a refused proposal spends no pacing"
    );
}

#[test]
fn relog_discards_temporary_prayer_debt_without_a_click() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    crate::play_slots::reset_slot_session_work(
        "alice",
        &rig.scripts,
        &rig.cheats,
        &Arc::new(Mutex::new(HashMap::new())),
        &rig.navs,
        false,
    );
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
}

#[test]
fn death_discards_protect_without_a_toggle() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    let mut stats = rig.snapshot.stats().to_vec();
    stats
        .iter_mut()
        .find(|stat| stat.index == 3)
        .unwrap()
        .effective = 0;
    rig.snapshot.seed_stats(stats);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
}

#[test]
fn unprotectable_is_delivered_to_the_walk_owner() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_prayer(&mut rig.snapshot, 37);
    seed_protect_frame(&mut rig.snapshot, 37);
    rig.observe(1);
    rig.wait_routed();
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert!(
        rig.walk_armed(),
        "Unprotectable must leave the route following"
    );
    rig.observe(2);
    assert_eq!(rig.end(), None, "the warning must not complete its walk");
    {
        let shared = rig.shared.lock();
        assert_eq!(shared.events.len(), 1);
        assert_eq!(
            shared.events[0].kind,
            WalkEventKind::Unprotectable {
                protect: script::combat::GuardProtect::Missiles
            }
        );
        assert_eq!(
            shared.events[0].detail.as_ref(),
            "Prayer 40 needed for Protect from Missiles"
        );
    }
    for tick in 3..6 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        rig.observe(u64::from(tick));
    }
    assert!(
        rig.walk_armed(),
        "the owner still follows the unprotected crossing"
    );
    assert!(
        rig.driver.move_calls > 0,
        "the route keeps issuing follow work"
    );
    assert_eq!(
        rig.shared.lock().events.len(),
        1,
        "the owner sees the warning once"
    );
    assert!(rig.driver.if_button_components.is_empty());
}

#[test]
fn exhausted_guard_reports_once_and_keeps_the_walk_following() {
    let mut rig = protected_rig();
    let mut stats = rig.snapshot.stats().to_vec();
    stats
        .iter_mut()
        .find(|stat| stat.index == 5)
        .unwrap()
        .effective = 0;
    rig.snapshot.seed_stats(stats);
    seed_missile_launch(&mut rig.snapshot);
    for tick in 2..6 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        rig.observe(u64::from(tick));
    }
    let shared = rig.shared.lock();
    assert!(shared.result.is_none());
    assert_eq!(shared.events.len(), 1);
    assert_eq!(
        shared.events[0].kind,
        WalkEventKind::Unprotectable {
            protect: script::combat::GuardProtect::Missiles,
        }
    );
    assert_eq!(
        shared.events[0].detail.as_ref(),
        "No Prayer points or prayer potion available for Protect from Missiles"
    );
    assert!(rig.walk_armed());
    assert!(rig.driver.move_calls > 0);
    assert!(rig.driver.if_button_components.is_empty());
}

#[test]
fn unprotectable_leaves_a_preexisting_user_style_on_when_the_walk_ends() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_protect_frame(&mut rig.snapshot, 37);
    seed_prayer_widgets(&mut rig.snapshot);
    seed_protect_state(&mut rig.snapshot, Some(95));
    rig.snapshot.seed_tick(1);
    rig.observe(1);
    rig.wait_routed();
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    rig.observe(2);
    assert!(rig.walk_armed());
    assert_eq!(rig.shared.lock().events.len(), 1);
    assert!(rig.driver.if_button_components.is_empty());
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(3);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        Vec::<i32>::new(),
        "the guard never raised the user's Magic and must leave it on"
    );
}

#[test]
fn dropped_enable_then_end_expires_and_follows_the_next_walk_without_owning_user_prayer() {
    let mut rig = protected_rig();
    rig.shared.lock().walks = 2;
    rig.snapshot.seed_tick(2);
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_projectiles(Vec::new());
    rig.snapshot.seed_npcs(Vec::new());
    for tick in 3..=4 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        rig.observe(u64::from(tick));
    }
    assert_eq!(rig.shared.lock().begun, 2);
    rig.wait_routed();
    let moves = rig.driver.move_calls;
    rig.snapshot.seed_tick(5);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none(),
        "the dropped enable debt expires three ticks after its admission"
    );
    assert!(
        rig.driver.move_calls > moves,
        "the second walk follows on the deadline pump"
    );
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(8);
    rig.step();
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(9);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "neither stale debt nor the second guard may turn off a later user prayer"
    );
}

#[test]
fn a_dropped_switch_retires_the_guard_owned_old_protect() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5623],
        "the guard admits a switch from its owned Missiles style to Melee"
    );

    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    for tick in 5..=7 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5623, 5622],
        "when the server drops Melee, cleanup turns off the still-on owned Missiles"
    );
}

#[test]
fn a_dropped_switch_cleanup_does_not_restart_navigation_deferral() {
    let mut rig = protected_rig();
    rig.shared.lock().walks = 2;
    raise_owned_missiles(&mut rig);
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5623]);
    abort_script_walk(&rig.navs, "alice");
    rig.shared.lock().protect = false;
    rig.snapshot.seed_tick(5);
    rig.step();
    rig.observe(5);
    rig.snapshot.seed_tick(6);
    rig.step();
    rig.observe(6);
    assert_eq!(rig.shared.lock().begun, 2);
    rig.wait_routed();
    let moves = rig.driver.move_calls;
    rig.snapshot.seed_tick(7);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5623, 5622]);
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());
    assert!(
        rig.driver.move_calls > moves,
        "retiring the fallback must not start a second navigation deferral window"
    );
}

#[test]
fn a_dropped_switch_does_not_turn_off_a_user_owned_old_protect() {
    let mut rig = protected_rig();
    seed_protect_state(&mut rig.snapshot, Some(96));
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(2);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5623]);

    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    for tick in 3..=5 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![5623],
        "the old Missiles style was user-owned, so expiry cannot click it off"
    );
}

#[test]
fn a_real_switch_retires_the_new_protect_without_restoring_the_old_one() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5623]);

    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let melee_varp = data.prayer_by_name("Protect from Melee").unwrap().varp;
    seed_protect_state(&mut rig.snapshot, Some(melee_varp));
    rig.snapshot.seed_tick(5);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5623, 5623],
        "a real switch transfers cleanup to Melee without clicking old Missiles"
    );
}

#[test]
fn a_long_hold_gives_unavailable_owned_on_debt_a_fresh_cleanup_window() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);

    rig.snapshot.seed_ingame(1);
    rig.snapshot.seed_tick(40);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some(),
        "the first cleanup pump starts a new clock instead of expiring at stale guard time"
    );
    assert_eq!(rig.driver.if_button_components, vec![5622]);

    seed_protect_frame(&mut rig.snapshot, 43);
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(41);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622],
        "a newly readable on-varp is still retired within the fresh attempt window"
    );
}

#[test]
fn a_long_hold_gives_a_refused_cleanup_click_a_fresh_attempt_window() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);

    rig.snapshot.seed_main_modal(-1, Vec::new());
    rig.snapshot.seed_tick(40);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some(),
        "a refused first-pump dispatch keeps the newly started debt alive"
    );
    assert_eq!(rig.driver.if_button_components, vec![5622]);

    seed_prayer_widgets(&mut rig.snapshot);
    rig.snapshot.seed_tick(41);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622],
        "the cleanup click is admitted when its widget resolves on the next pump"
    );
}

#[test]
fn timed_guard_cleanup_drops_report_unavailable_varps_and_refused_clicks() {
    for (missing_varps, reason) in [
        (true, "owned protect varp unavailable within cleanup window"),
        (false, "off click refused within cleanup window"),
    ] {
        let mark = crate::walk_map::test_log::mark();
        let mut rig = protected_rig();
        raise_owned_missiles(&mut rig);
        rig.slot().lock().unwrap().stop();
        reset_script_nav(&rig.navs, "alice", None);
        if missing_varps {
            rig.snapshot.seed_ingame(1);
        } else {
            rig.snapshot.seed_main_modal(-1, Vec::new());
        }
        rig.snapshot.seed_tick(4);
        rig.step();
        assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());

        rig.snapshot.seed_tick(7);
        rig.step();
        assert!(
            rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none(),
            "unresolved cleanup remains bounded"
        );
        assert_eq!(rig.driver.if_button_components, vec![5622]);
        assert!(
            crate::walk_map::test_log::records_since(mark)
                .iter()
                .any(|(slot, message)| slot == "alice"
                    && message.starts_with("prayer cleanup drops protect debt component=5622 ")
                    && message.contains(reason)),
            "timed cleanup drop must explain its unresolved cause: {reason}"
        );
    }
}

#[test]
fn dropped_off_click_retains_observation_debt_and_gets_exactly_one_bounded_retry() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some(),
        "sending the off-click does not establish that the server applied it"
    );
    for tick in 5..=6 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
    rig.snapshot.seed_tick(7);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622, 5622]);
    for tick in 8..=10 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none());
    for tick in 11..=20 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622, 5622],
        "a dropped retry must not leave a permanent debt or admit a third off-click"
    );
}

#[test]
fn an_off_retry_does_not_block_a_later_walk_beyond_the_first_pacing_window() {
    let mut rig = protected_rig();
    rig.shared.lock().walks = 2;
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    rig.shared.lock().protect = false;
    rig.snapshot.seed_tick(4);
    rig.step();
    rig.observe(4);
    rig.snapshot.seed_tick(5);
    rig.step();
    rig.observe(5);
    assert_eq!(rig.shared.lock().begun, 2);
    rig.wait_routed();
    let moves = rig.driver.move_calls;
    rig.snapshot.seed_tick(7);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());
    assert!(
        rig.driver.move_calls > moves,
        "navigation resumes after three ticks even while the off receipt is outstanding"
    );
}

#[test]
fn another_owners_observed_protect_switch_settles_the_off_without_a_toggle() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    seed_protect_state(&mut rig.snapshot, Some(95));
    rig.snapshot.seed_tick(4);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none());
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "observing another style on means the owed style is already off"
    );
}

#[test]
fn observed_off_settles_cleanup_before_retry_and_leaves_a_later_user_enable_alone() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(4);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());
    seed_protect_state(&mut rig.snapshot, None);
    rig.snapshot.seed_tick(5);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none());
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(9);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
}

#[test]
fn an_unroutable_native_walk_delivers_one_failed_terminal() {
    let mut rig = rig(Some(Arc::new(open_world(3, 1))), false);
    rig.observe(1);
    assert!(
        wait_until(5_000, || rig.navs.lock().unwrap()["alice"]
            .route_worker
            .is_none()),
        "the planner completes the unreachable request"
    );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Failed)));
    for tick in 3..40 {
        rig.observe(tick);
        rig.step();
    }
    assert_eq!(rig.shared.lock().results.len(), 1);
    assert_eq!(rig.shared.lock().begun, 1);
    assert_eq!(rig.driver.walked, None);
}

#[test]
fn native_walk_failure_receipt_names_missing_route_supplies() {
    let world = toll_nav_world();
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    world.bind_named_bank_facts(&selected).unwrap();
    let mut rig = rig(Some(Arc::new(world)), false);
    rig.observe(1);
    assert!(
        wait_until(5_000, || rig.navs.lock().unwrap()["alice"]
            .route_worker
            .is_none()),
        "the planner completes the fare-gated request"
    );
    rig.observe(2);

    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::Failed);
    let detail = receipt
        .detail
        .as_deref()
        .expect("the failed walk receipt carries its navigation diagnosis");
    assert!(
        detail.contains("Insufficient route supplies: 10 more Coins (need 10, carrying 0)"),
        "{detail}"
    );
    assert!(detail.contains("native walk route failed"), "{detail}");
}

/// The host's own route arm, as the watchdog's recovery walk and legacy
/// script walks use it: no native authority.
fn host_walk(rig: &Rig, x: i32, retarget: bool) -> bool {
    crate::script_runtime::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    }
    .queue_route_in_snapshot(
        &rig.snapshot,
        x,
        0,
        0,
        nav::router::FindOptions::default(),
        0,
        retarget,
        0,
    )
}

#[test]
fn a_native_walk_refused_by_an_in_flight_route_gets_a_refused_receipt() {
    let mut rig = open_rig(false);
    // A host walk to the same destination is already in flight under the
    // legacy request id; the distinct native id is refused, not coalesced.
    assert!(host_walk(&rig, 4, false));
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
}

#[test]
fn pause_and_resume_end_a_native_walk_with_a_cancelled_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let slot = rig.slot();
        let mut slot = slot.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
        slot.resume();
    }
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
    rig.step();
    assert_eq!(rig.driver.walked, None, "no orphaned follow after Resume");
}

#[test]
fn a_watchdog_walk_displacing_a_native_walk_cancels_it() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    // The watchdog's recovery ArmWalk: the host arm without native authority.
    assert!(host_walk(&rig, 8, true));
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
}

#[test]
fn an_unproven_crossing_reaches_the_native_owner_typed() {
    use api::selected::{FactKey, QuestGate, Truth};
    let gate = QuestGate::Complete(FactKey(Arc::from("tbwt")));
    let undecided: Arc<[QuestGate]> = Arc::from([gate]);
    for (verdict, unresolved, expected) in [
        (
            Truth::Unknown,
            Arc::clone(&undecided),
            WalkEnd::NeedsEvidence(Arc::clone(&undecided)),
        ),
        (Truth::False, Arc::from([]), WalkEnd::Blocked),
    ] {
        let mut rig = open_rig(false);
        rig.observe(1);
        rig.wait_routed();
        // The follow's pre-send recheck refused the crossing.
        crate::script_runtime::apply_nav_follow_outcome(
            rig.navs.lock().unwrap().get_mut("alice").unwrap(),
            Some(nav::traveller::TravelOutcome::EvidenceUnproven {
                at: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                leg: 0,
                verdict,
                unresolved,
            }),
            false,
        );
        rig.observe(2);
        assert_eq!(rig.end(), Some(Ok(expected)));
    }
}

#[test]
fn cancelling_and_rewalking_the_same_destination_arms_the_new_walk() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    // The owner cancels its walk and walks the same destination again
    // before the pump has stepped the abandoned follow away.
    rig.shared.lock().rewalk = true;
    rig.observe(2);
    assert!(
        rig.navs.lock().unwrap()["alice"]
            .native_walk
            .as_ref()
            .is_some_and(|owner| owner.live()),
        "the new walk owns the route, not a refusal beside the abandoned one"
    );
    rig.observe(3);
    assert_eq!(rig.end(), None, "the re-walk is in flight, not refused");
    rig.wait_routed();
}

#[test]
fn native_cross_exemption_is_scoped_to_one_walk() {
    let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
    {
        let mut shared = rig.shared.lock();
        shared.walks = 2;
        shared.cross_first = vec![Arc::from("test-barrier@2,0,0")];
    }

    rig.observe(1);
    rig.wait_routed();
    crate::script_runtime::apply_nav_follow_outcome(
        rig.navs.lock().unwrap().get_mut("alice").unwrap(),
        Some(nav::traveller::TravelOutcome::Arrived {
            at: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
        }),
        false,
    );
    rig.observe(2);
    assert_eq!(rig.shared.lock().results.len(), 1);

    rig.observe(3);
    assert!(wait_until(5_000, || {
        rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.native_walk_failure.is_some())
    }));
    rig.observe(4);

    let shared = rig.shared.lock();
    assert_eq!(shared.results.len(), 2);
    assert!(shared.results[0].as_ref().unwrap().blocked.is_none());
    let second = shared.results[1].as_ref().unwrap();
    assert_eq!(second.end, WalkEnd::Refused);
    assert_eq!(
        second.blocked.as_deref(),
        Some(&[nav::zones::ZoneKey::Zone(0)][..])
    );
    assert!(second
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("test-barrier@2,0,0")));
}

#[test]
fn compat_catalog_exclusions_use_frozen_geometry_and_rules() {
    use script::shim::InspectAvoidWire;

    let world = catalog_open_world();
    let outside = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let inside_jail = WorldTile {
        x: 3_100,
        z: 3_230,
        level: 0,
    };
    let resolve_avoid = |from, state: &WorldState, id: &str| {
        let mut exclusions = crate::script_runtime::ScriptRouteExclusions::default();
        exclusions
            .avoid_wire
            .push(InspectAvoidWire::Catalog(id.to_string()));
        crate::script_runtime::resolve_route_exclusions(
            nav::router::FindOptions::default(),
            &world,
            from,
            outside,
            state,
            exclusions,
        )
    };

    let state = WorldState::empty();
    let (_, white_wolf) = resolve_avoid(outside, &state, "white-wolf-mountain").unwrap();
    assert_eq!(white_wolf.avoid.len(), 1);
    assert_eq!(
        (
            white_wolf.avoid[0].min_x,
            white_wolf.avoid[0].max_x,
            white_wolf.avoid[0].min_z,
            white_wolf.avoid[0].max_z,
            white_wolf.avoid[0].level,
        ),
        (2828, 2878, 3468, 3538, None)
    );

    let (_, jail) = resolve_avoid(outside, &state, "draynor-jail-guards").unwrap();
    assert_eq!(jail.avoid.len(), 4);
    let mut high_combat = WorldState::empty();
    high_combat.combat_level = Some(51);
    let (_, skipped_for_combat) =
        resolve_avoid(outside, &high_combat, "draynor-jail-guards").unwrap();
    assert!(skipped_for_combat.avoid.is_empty());
    let (_, skipped_for_endpoint) =
        resolve_avoid(inside_jail, &state, "draynor-jail-guards").unwrap();
    assert!(skipped_for_endpoint.avoid.is_empty());
}

#[test]
fn compat_named_exclusions_reject_unknown_and_over_limit_names() {
    use script::shim::InspectAvoidWire;

    let world = catalog_open_world();
    let from = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let state = WorldState::empty();
    let mut unknown = crate::script_runtime::ScriptRouteExclusions::default();
    unknown
        .avoid_wire
        .push(InspectAvoidWire::Catalog("no-such-zone".to_string()));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            unknown,
        )
        .unwrap_err(),
        "avoidZones: unknown zone \"no-such-zone\""
    );

    let mut unknown_cross = crate::script_runtime::ScriptRouteExclusions::default();
    unknown_cross.cross.push(Arc::from("no-such-zone"));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            unknown_cross,
        )
        .unwrap_err(),
        "crossZones: unknown zone \"no-such-zone\""
    );

    let too_many = crate::script_runtime::ScriptRouteExclusions {
        cross: (0..9).map(|_| Arc::from("test-barrier@2,0,0")).collect(),
        ..Default::default()
    };
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            too_many,
        )
        .unwrap_err(),
        "crossZones: more than 8 zone names"
    );
}

fn blocked_end_world() -> NavWorld {
    let mut world = open_world(40, 1);
    world.collision.blocked[0] |= 1 << 39;
    world
}

fn two_walk_rig() -> Rig {
    let mut rig = rig(Some(Arc::new(blocked_end_world())), false);
    {
        let mut shared = rig.shared.lock();
        shared.walks = 2;
        shared.later_target = Some(WorldTile {
            x: 39,
            z: 0,
            level: 0,
        });
    }
    rig.observe(1);
    rig.wait_routed();
    rig
}

fn drive_second_walk(rig: &mut Rig, first_tick: u64) -> Vec<Result<WalkEnd, ActionError>> {
    for tick in (first_tick..).take(40) {
        rig.observe(tick);
        if rig.shared.lock().results.len() >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    rig.shared
        .lock()
        .results
        .iter()
        .map(|result| result.clone().map(|receipt| receipt.end))
        .collect()
}

#[test]
fn review_control_generic_cancel_then_nopath_delivers_failed() {
    let mut rig = two_walk_rig();
    {
        let slot = rig.slot();
        let mut slot = slot.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
        slot.resume();
    }
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
    let ends = drive_second_walk(&mut rig, 3);
    eprintln!(
        "manual-click resident NavBot={}",
        std::mem::size_of::<NavBot>()
    );
    assert_eq!(ends.len(), 2, "control: the second walk gets its terminal");
    assert_eq!(ends[1], Ok(WalkEnd::Failed));
}

#[test]
fn review_manual_takeover_then_nopath_must_still_deliver_a_terminal() {
    let mut rig = two_walk_rig();
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::UserInput)));
    let first_request = rig.navs.lock().unwrap()["alice"].walk_outcome_request_id;
    let ends = drive_second_walk(&mut rig, 3);
    let bots = rig.navs.lock().unwrap();
    eprintln!(
        "manual-click native probe: ends={ends:?} NavBot={} live_refusal_id={} (cancelled request {first_request}) outcome_request_id={} reason={:?} native_walk_live={} requested_route={:?}",
        std::mem::size_of::<NavBot>(),
        bots["alice"].walk_live_refusal_id,
        bots["alice"].walk_outcome_request_id,
        bots["alice"].walk_outcome_cancel_reason,
        bots["alice"].native_walk.is_some(),
        bots["alice"].requested_route,
    );
    assert_eq!(
        ends.len(),
        2,
        "after a manual takeover the next native walk's NoPath must reach its owner"
    );
    assert_eq!(ends[1], Ok(WalkEnd::Failed));
}

#[test]
fn review_native_recovery_click_then_nopath_delivers_terminal() {
    let mut rig = two_walk_rig();
    let first_native = rig.navs.lock().unwrap()["alice"].walk_request_id;
    super::apply_watchdog_nav_action(
        script::WatchdogAction::ArmWalk {
            x: 8,
            z: 0,
            level: 0,
        },
        &mut rig.driver,
        Some(&rig.snapshot),
        Some((0, 0, 0)),
        &rig.navs,
        &rig.world,
        Some(WorldState::empty()),
        "alice",
    );
    {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        eprintln!(
            "manual-click recovery probe: native_request={first_native} \
             walk_request_id={} native_walk={} carried_walk={} live_refusal_id={}",
            bot.walk_request_id,
            bot.native_walk.is_some(),
            bot.carried_walk.is_some(),
            bot.walk_live_refusal_id,
        );
    }
    rig.observe(2);
    assert_eq!(
        rig.end(),
        Some(Ok(WalkEnd::Cancelled)),
        "replacing the native follow ends its first owner"
    );
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        2,
        false,
    ));
    {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        eprintln!(
            "manual-click after recovery click: outcome_request_id={} reason={:?} \
             live_refusal_id={} native_walk={} carried_walk={}",
            bot.walk_outcome_request_id,
            bot.walk_outcome_cancel_reason,
            bot.walk_live_refusal_id,
            bot.native_walk.is_some(),
            bot.carried_walk.is_some(),
        );
    }
    let ends = drive_second_walk(&mut rig, 3);
    eprintln!("manual-click native recovery treatment: ends={ends:?}");
    assert_eq!(
        ends.len(),
        2,
        "after native recovery identity and click, the next native NoPath reaches its owner"
    );
    assert_eq!(ends[1], Ok(WalkEnd::Failed));
}

#[test]
fn manual_cancellation_detail_clears_when_the_next_walk_arms() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        2,
        false,
    ));
    assert!(rig.navs.lock().unwrap()["alice"].manual_walk_cancelled_detail);

    let arm = super::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".into(),
        state: Some(WorldState::empty()),
        bank: Vec::new(),
    };
    let completed = arm
        .queue_route_in_snapshot_synced(
            &rig.snapshot,
            8,
            0,
            0,
            nav::router::FindOptions::default(),
            0,
            true,
            99,
            None,
        )
        .expect("the next walk route arms");
    assert!(
        !rig.navs.lock().unwrap()["alice"].manual_walk_cancelled_detail,
        "the detail clears on the next route arm"
    );
    assert_eq!(
        rig.navs.lock().unwrap()["alice"].walk_outcome_cancel_reason,
        script::isolate_fb::WalkCancelReason::UserInput,
        "the previously published receipt remains intact"
    );
    completed
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the new route worker completes");
}

#[test]
fn manual_takeover_of_carried_walk_keeps_its_receipt_identity() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let (request_id, generation, key) = {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.native_walk = None;
        (
            bot.walk_request_id,
            bot.route_generation,
            bot.requested_route
                .expect("the route keeps its request identity"),
        )
    };
    crate::script_runtime::hold_script_nav(&rig.navs, "alice", Some(7));
    let (nav_size, carried_size) = {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        let carried = bot.carried_walk.as_deref().expect("the route is carried");
        (std::mem::size_of_val(bot), std::mem::size_of_val(carried))
    };
    eprintln!("manual-click resident sizes: NavBot={nav_size} CarriedWalk={carried_size}");
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.cancel_for_manual_input();
        let (to, radius, teleports, _, _, _) = key;
        assert_eq!(bot.walk_outcome_request_id, request_id);
        assert_eq!(bot.walk_outcome_generation, generation);
        assert_eq!(
            (
                bot.walk_outcome_x,
                bot.walk_outcome_z,
                bot.walk_outcome_level
            ),
            (to.x, to.z, to.level)
        );
        assert_eq!(bot.walk_outcome_radius, radius);
        assert_eq!(bot.walk_outcome_allow_teleports, teleports);
        assert_eq!(
            bot.walk_outcome_cancel_reason,
            script::isolate_fb::WalkCancelReason::UserInput
        );
    }
}

#[test]
fn manual_click_terminal_resets_on_stop_before_the_next_load_snapshot() {
    let mut play = run_with_io(
        &PlayOptions {
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
    play.attach_arm("alice", SlotArm::new(7, false));
    let source = "export function tick(api) {}";
    play.script_start_load(
        "alice",
        source.into(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    play.navs
        .lock()
        .unwrap()
        .insert("alice".into(), super::following_script_walk());
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &play.scripts,
        &play.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    assert_eq!(
        play.navs.lock().unwrap()["alice"].walk_outcome_cancel_reason,
        script::isolate_fb::WalkCancelReason::UserInput
    );
    assert!(play.script_walk_cancelled_by_user("alice"));

    play.script_stop("alice");
    let navs = play.navs.lock().unwrap();
    assert_eq!(
        navs["alice"].walk_outcome_cancel_reason,
        script::isolate_fb::WalkCancelReason::None,
        "Stop clears the prior run's UserInput outcome"
    );
    assert_eq!(navs["alice"].walk_outcome_request_id, 0);
    assert_eq!(navs["alice"].walk_live_refusal_id, 0);
    drop(navs);
    assert!(!play.script_walk_cancelled_by_user("alice"));

    play.script_start_load(
        "alice",
        source.into(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let mut client = nav_client();
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    script_observe(
        &mut client,
        "alice",
        true,
        true,
        1,
        Some((0, 0, 0)),
        None,
        None,
        Some(&snapshot),
        None,
        &play.scripts,
        &play.cheats,
        &play.navs,
        &play.world,
        false,
        false,
    );
    let reason = script_slot(&play.scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__rs2b0t_host.snapshot.walk_outcome_cancel_reason")
        .unwrap();
    assert_eq!(
        reason.as_str(),
        Some("none"),
        "the next owner's first Load snapshot starts without the prior cancellation"
    );
    play.script_stop("alice");
}

fn manual_frame(hold: bool) -> crate::SlotFrameInput {
    crate::SlotFrameInput {
        hold,
        manual_move_intent: Some(crate::ManualMoveIntent::Minimap),
        manual_steps: 0,
    }
}

#[test]
fn manual_step_takeover_must_end_active_walk_before_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let request = rig.navs.lock().unwrap()["alice"].walk_request_id;
    let input = host::SlotInput::new();
    let queues = Mutex::new(HashMap::from([(
        "alice".into(),
        VecDeque::from([crate::WireCmd::Walk {
            x: 0,
            z: 1,
            level: 0,
        }]),
    )]));
    let (frame, human) = crate::play_slots::take_slot_frame_input(&input, "alice", &queues, false);
    assert_eq!(frame.manual_move_count(), 1);
    assert_eq!(human.len(), 1, "the human step is preserved");
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        frame,
        true,
        1,
        false,
    ));
    assert!(
        !rig.walk_armed(),
        "manual movement takes ownership before either follow pump"
    );
    let before = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    rig.step();
    rig.observe(2);
    assert_eq!(
        rig.driver.walked, None,
        "the old walk must not send a later hop"
    );
    assert_eq!(rig.end(), Some(Ok(WalkEnd::UserInput)));
    assert_eq!(rig.shared.lock().results.len(), 1, "exactly one terminal");
    assert_eq!(
        rig.slot().lock().unwrap().state(),
        script::RunState::Running,
        "OFF cancels without forcing the owning script to pause"
    );
    let mut bots = rig.navs.lock().unwrap();
    let bot = bots.get_mut("alice").unwrap();
    assert_eq!(bot.walk_outcome_request_id, request);
    assert_eq!(
        bot.walk_outcome_cancel_reason,
        script::isolate_fb::WalkCancelReason::UserInput
    );
    assert_eq!(bot.user_move_intent_seq, 1);
    assert_eq!(bot.manual_takeover_watermark, before);
    assert!(!bot.walking_decision_is_current(before - 1));
    assert!(
        bot.walking_decision_is_current(before),
        "fresh observed decisions are permitted"
    );
    bot.mark_walk_outcome_posted(before);
    bot.clear_walk_outcome();
    assert_eq!(
        bot.walk_outcome_seq, before,
        "old release cannot erase cancellation"
    );
    drop(bots);
    rig.observe(3);
    rig.step();
    assert_eq!(rig.shared.lock().results.len(), 1);
    assert_eq!(rig.driver.walked, None);
}

#[test]
fn manual_config_on_native_pause_revokes_action_resume_makes_fresh_decision() {
    let mut rig = two_walk_rig();
    let original = rig.navs.lock().unwrap()["alice"].walk_request_id;
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        true,
    ));
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Paused);
    assert!(!rig.slot().lock().unwrap().has_native_actions());
    assert!(rig.navs.lock().unwrap()["alice"].carried_walk.is_none());
    rig.observe(2);
    rig.step();
    assert_eq!(rig.driver.walked, None);
    assert_eq!(rig.shared.lock().begun, 1, "no decision while Paused");
    rig.slot().lock().unwrap().resume();
    let ends = drive_second_walk(&mut rig, 3);
    assert_eq!(ends.len(), 2);
    assert_eq!(
        ends[1],
        Ok(WalkEnd::Failed),
        "the explicit Resume permits a fresh decision"
    );
    let shared = rig.shared.lock();
    let next = shared.results[1].as_ref().unwrap();
    assert_ne!(
        next.request_id, original,
        "the next walk is not replay of the cancelled request"
    );
}

#[test]
fn held_manual_click_still_cancels_and_other_slot_survives() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let route = rig.navs.lock().unwrap()["alice"].route.clone();
    rig.navs.lock().unwrap().insert(
        "bob".into(),
        NavBot {
            route,
            walk_request_id: 99,
            ..NavBot::default()
        },
    );
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(true),
        true,
        1,
        false,
    ));
    assert!(!rig.walk_armed());
    assert!(rig.navs.lock().unwrap()["bob"].script_walk_armed());
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    rig.observe(2);
    assert!(
        !crate::script_runtime::take_manual_walk_ownership(
            &rig.scripts,
            &rig.navs,
            "alice",
            manual_frame(true),
            true,
            2,
            false,
        ),
        "an already ended operation cannot produce a second terminal"
    );
    let bots = rig.navs.lock().unwrap();
    assert_eq!(bots["alice"].walk_outcome_seq, seq);
    assert_eq!(bots["alice"].user_move_intent_seq, 2);
}

#[test]
fn manual_round3_click_after_arrival_keeps_the_completed_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    crate::script_runtime::apply_nav_follow_outcome(
        rig.navs.lock().unwrap().get_mut("alice").unwrap(),
        Some(nav::traveller::TravelOutcome::Arrived {
            at: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
        }),
        false,
    );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::RouteEnded)));
    let before = {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        assert!(bot.route.is_none());
        assert!(bot.requested_route.is_some(), "dedupe retains the identity");
        (
            bot.walk_outcome_seq,
            bot.walk_outcome_request_id,
            bot.route_generation,
        )
    };
    let taken = crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        2,
        false,
    );
    {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        eprintln!(
            "click-after-arrival taken={taken} outcome_seq={} request={} failed={} reason={:?} intent={}",
            bot.walk_outcome_seq,
            bot.walk_outcome_request_id,
            bot.walk_outcome_failed,
            bot.walk_outcome_cancel_reason,
            bot.user_move_intent_seq,
        );
        assert!(!taken, "a completed request is not active walking");
        assert_eq!(
            (
                bot.walk_outcome_seq,
                bot.walk_outcome_request_id,
                bot.route_generation
            ),
            before,
            "the click must not replace the completed receipt"
        );
        assert!(!bot.walk_outcome_failed);
        assert_eq!(
            bot.walk_outcome_cancel_reason,
            script::isolate_fb::WalkCancelReason::None
        );
        assert_eq!(bot.manual_takeover_watermark, 0);
        assert_eq!(bot.user_move_intent_seq, 1, "idle intent is still observed");
    }
    rig.observe(3);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::RouteEnded)));
    assert_eq!(rig.shared.lock().results.len(), 1);
}

#[test]
fn operator_paused_walk_is_not_cancelled_by_manual_intent() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let cell = rig.slot();
        let mut slot = cell.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
    }
    let old_seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    assert!(!crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let bots = rig.navs.lock().unwrap();
    assert_eq!(bots["alice"].walk_outcome_seq, old_seq);
    assert_eq!(bots["alice"].manual_takeover_watermark, 0);
    assert_eq!(bots["alice"].user_move_intent_seq, 1);
}

#[test]
fn reconnect_gated_owner_is_not_taken_over_before_its_ready_observation() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let cell = rig.slot();
        let mut slot = cell.lock().unwrap();
        slot.on_is_up(false);
        assert!(
            slot.want_run,
            "reconnect gating does not change operator intent"
        );
        assert_eq!(slot.state(), script::RunState::Paused);
    }
    let original = {
        let bots = rig.navs.lock().unwrap();
        (
            bots["alice"].walk_request_id,
            bots["alice"].walk_outcome_seq,
        )
    };
    assert!(!crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let bots = rig.navs.lock().unwrap();
    assert_eq!(
        (
            bots["alice"].walk_request_id,
            bots["alice"].walk_outcome_seq
        ),
        original
    );
    assert_eq!(bots["alice"].manual_takeover_watermark, 0);
    assert_eq!(bots["alice"].user_move_intent_seq, 1);
}

#[test]
fn takeover_during_pending_worker_cannot_reinstall_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    let (generation, request_id) = {
        let bots = rig.navs.lock().unwrap();
        (
            bots["alice"].route_generation,
            bots["alice"].walk_request_id,
        )
    };
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    rig.navs
        .lock()
        .unwrap()
        .get_mut("alice")
        .unwrap()
        .publish_route(
            generation,
            request_id,
            false,
            crate::walk_plan::RouteOutcome::Routed(Route {
                legs: vec![],
                dest: WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
                ticks: 4.0,
            }),
        );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::UserInput)));
    rig.step();
    assert!(!rig.walk_armed());
    assert_eq!(rig.driver.walked, None);
    assert_eq!(rig.navs.lock().unwrap()["alice"].walk_outcome_seq, seq);
}

#[test]
fn queued_walking_decisions_are_fenced_but_observed_fresh_walk_is_allowed() {
    use script::shim::InteractReq;
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let stale = [
        InteractReq::Walk {
            x: 11,
            z: 0,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 90,
            avoid: vec![],
            cross: vec![],
        },
        InteractReq::WalkNear {
            x: 12,
            z: 0,
            level: 0,
            radius: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 91,
            avoid: vec![],
            cross: vec![],
        },
        InteractReq::WalkNearestBank,
        InteractReq::WalkTo {
            x: 13,
            z: 0,
            level: 0,
        },
    ];
    rig.slot().lock().unwrap().restore_host_interacts(
        stale
            .into_iter()
            .map(|req| script::load::QueuedInteract {
                req,
                observed_walk_outcome_seq: 0,
            })
            .collect(),
    );
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    let original = rig.navs.lock().unwrap()["alice"].walk_outcome_request_id;
    rig.observe(2);
    assert!(!rig.walk_armed());
    assert_eq!(rig.driver.walked, None);
    {
        let bots = rig.navs.lock().unwrap();
        assert_eq!(bots["alice"].walk_outcome_seq, seq);
        assert_eq!(bots["alice"].walk_outcome_request_id, original);
        assert_eq!(
            bots["alice"].walk_outcome_cancel_reason,
            script::isolate_fb::WalkCancelReason::UserInput
        );
    }
    rig.slot()
        .lock()
        .unwrap()
        .restore_host_interacts(vec![script::load::QueuedInteract {
            req: InteractReq::WalkTo {
                x: 14,
                z: 0,
                level: 0,
            },
            observed_walk_outcome_seq: seq,
        }]);
    rig.observe(3);
    assert_eq!(
        rig.driver.walked,
        Some((14, 0)),
        "an explicit decision based on the takeover snapshot remains legal with pause OFF"
    );
}

#[test]
fn held_manual_input_cancels_only_its_operator_walk_once() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let route = rig.navs.lock().unwrap()["alice"].route.clone();
    let alice = Arc::new(Mutex::new(crate::WalkArm {
        route: route.clone(),
        route_generation: 17,
        ..crate::WalkArm::default()
    }));
    let bob = Arc::new(Mutex::new(crate::WalkArm {
        route,
        route_generation: 19,
        ..crate::WalkArm::default()
    }));
    let arms = Arc::new(Mutex::new(HashMap::from([
        ("alice".into(), Arc::clone(&alice)),
        ("bob".into(), Arc::clone(&bob)),
    ])));
    assert!(crate::cancel_walk_arm_on_manual_input(
        "alice",
        &arms,
        Some(WorldTile {
            x: 0,
            z: 0,
            level: 0
        }),
        manual_frame(true),
    ));
    assert!(!crate::cancel_walk_arm_on_manual_input(
        "alice",
        &arms,
        Some(WorldTile {
            x: 0,
            z: 0,
            level: 0
        }),
        manual_frame(true),
    ));
    assert!(alice.lock().unwrap().route.is_none());
    assert_eq!(alice.lock().unwrap().route_generation, 18);
    assert!(bob.lock().unwrap().route.is_some());
    assert_eq!(bob.lock().unwrap().route_generation, 19);
}
const WATER_CENTER: WorldTile = WorldTile {
    x: 60,
    z: 60,
    level: 0,
};
const WATER_SHORE: WorldTile = WorldTile {
    x: 20,
    z: 60,
    level: 0,
};
const ISOLATED_SHORE: WorldTile = WorldTile {
    x: 65,
    z: 65,
    level: 0,
};

fn water_area_world(separating_wall: bool) -> NavWorld {
    const SIZE: usize = 128;
    let mut flags = vec![0u32; SIZE * SIZE];
    for x in 20..=100 {
        for z in 20..=100 {
            if (x, z) != (WATER_SHORE.x as usize, WATER_SHORE.z as usize)
                && (x, z) != (ISOLATED_SHORE.x as usize, ISOLATED_SHORE.z as usize)
            {
                flags[z * SIZE + x] |= CollisionFlag::SQ_BLOCKED as u32;
            }
        }
    }
    if separating_wall {
        for z in 0..SIZE {
            flags[z * SIZE + 10] |= CollisionFlag::SQ_BLOCKED as u32;
        }
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    )
}

fn water_area_rig(separating_wall: bool, radius: u16) -> Rig {
    let mut rig = rig(Some(Arc::new(water_area_world(separating_wall))), false);
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(WATER_CENTER);
        shared.radius = radius;
        shared.arrival = nav::arrival::ArrivalKind::Area;
    }
    for tile in [
        WATER_CENTER,
        WorldTile {
            x: WATER_CENTER.x - 1,
            ..WATER_CENTER
        },
        WorldTile {
            x: WATER_CENTER.x + 1,
            ..WATER_CENTER
        },
        WorldTile {
            z: WATER_CENTER.z - 1,
            ..WATER_CENTER
        },
        WorldTile {
            z: WATER_CENTER.z + 1,
            ..WATER_CENTER
        },
    ] {
        let x = (tile.x - rig.client.map_build_base_x) as usize;
        let z = (tile.z - rig.client.map_build_base_z) as usize;
        rig.client.collision[0].flags[x][z] |= CollisionFlag::SQ_BLOCKED;
    }
    rig.rebuild_snapshot_at(WorldTile {
        x: 0,
        z: 0,
        level: 0,
    });
    rig
}

fn wait_for_native_route_outcome(rig: &Rig) {
    assert!(
        wait_until(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| bot.walk_outcome_seq != 0 && bot.route_worker.is_none())
        }),
        "the native route worker published a terminal outcome"
    );
}

#[test]
fn area_walk_routes_to_reachable_shore_and_native_receipt_settles_there() {
    let mut rig = water_area_rig(false, 40);
    rig.observe(1);
    rig.wait_routed();

    let route = rig.navs.lock().unwrap()["alice"]
        .route
        .clone()
        .expect("reachable shore route");
    let goal = route.dest;
    assert_eq!(
        goal, WATER_SHORE,
        "the connected shoreline is the reachable area goal"
    );
    assert!(
        goal.x
            .abs_diff(WATER_CENTER.x)
            .max(goal.z.abs_diff(WATER_CENTER.z))
            <= 40,
        "goal {goal:?} is within the area's Chebyshev radius"
    );
    assert!(rig.world.as_ref().unwrap().collision.standable(goal));
    assert_ne!(
        goal, WATER_CENTER,
        "area routing does not require the water center"
    );
    assert_ne!(
        goal, ISOLATED_SHORE,
        "a disconnected stand is not a route goal"
    );

    rig.step_at(goal);
    rig.observe_at(2, goal);
    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::Arrived);
}

#[test]
fn area_walk_does_not_arrive_on_a_disconnected_candidate() {
    let mut rig = water_area_rig(true, 40);
    rig.observe(1);
    wait_for_native_route_outcome(&rig);
    rig.observe(2);

    assert_eq!(rig.end(), Some(Ok(WalkEnd::Failed)));
    assert!(
        rig.navs.lock().unwrap()["alice"].route.is_none(),
        "a standable candidate across a full collision wall is not reachable"
    );
}

#[test]
fn zero_radius_area_refuses_a_nonstandable_target() {
    let mut rig = water_area_rig(false, 0);
    rig.observe(1);
    wait_for_native_route_outcome(&rig);
    rig.observe(2);

    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::Failed);
    let detail = receipt
        .detail
        .as_deref()
        .expect("NoPath diagnostic is owed");
    assert!(detail.contains("Area arrival"), "{detail}");
    assert!(detail.contains(&format!("to {WATER_CENTER:?}")), "{detail}");
    assert!(detail.contains("within radius 0"), "{detail}");
}

#[test]
fn reach_walk_keeps_adjacent_solid_target_arrival() {
    let target = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let mut world = open_world(40, 40);
    let index = target.z as usize * world.collision.width + target.x as usize;
    world.collision.blocked[index / 64] |= 1 << (index % 64);
    let mut rig = rig(Some(Arc::new(world)), false);
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(target);
        shared.radius = 1;
        shared.arrival = nav::arrival::ArrivalKind::Reach;
    }
    let x = (target.x - rig.client.map_build_base_x) as usize;
    let z = (target.z - rig.client.map_build_base_z) as usize;
    rig.client.collision[0].flags[x][z] |= CollisionFlag::SQ_BLOCKED;
    rig.rebuild_snapshot_at(WorldTile {
        x: 0,
        z: 0,
        level: 0,
    });
    rig.observe(1);
    rig.wait_routed();

    let goal = rig.navs.lock().unwrap()["alice"]
        .route
        .as_ref()
        .expect("reach route to the solid target's approach")
        .dest;
    assert_eq!(
        goal.x.abs_diff(target.x).max(goal.z.abs_diff(target.z)),
        1,
        "Reach retains the old adjacent-solid target behavior"
    );
    rig.step_at(goal);
    rig.observe_at(2, goal);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Arrived)));
}

#[test]
fn area_route_refresh_keeps_mode_through_native_receipt() {
    let mut rig = water_area_rig(false, 40);
    rig.observe(1);
    rig.wait_routed();
    assert!(
        wait_until(5_000, || {
            rig.navs.lock().unwrap().get("alice").is_some_and(|bot| {
                bot.route_worker.is_none() && bot.pending_route.is_none() && bot.route.is_some()
            })
        }),
        "the initial area route completed"
    );

    let (request_id, generation, authority) = {
        let navs = rig.navs.lock().unwrap();
        let bot = navs.get("alice").unwrap();
        (
            bot.walk_request_id,
            bot.route_generation,
            bot.native_walk.clone().expect("native walk owner"),
        )
    };
    let arm = crate::script_runtime::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".to_owned(),
        state: None,
        bank: Vec::new(),
    };
    assert!(arm.refresh_route_in_snapshot(
        &rig.snapshot,
        WATER_CENTER,
        40,
        nav::router::FindOptions {
            allow_teleports: true,
            ..nav::router::FindOptions::default()
        },
        request_id,
        crate::script_runtime::ScriptRouteExclusions::default(),
        Some(authority),
        None,
        nav::arrival::ArrivalKind::Area,
    ));
    assert!(
        wait_until(5_000, || {
            rig.navs.lock().unwrap().get("alice").is_some_and(|bot| {
                bot.route_generation != generation
                    && bot.route_worker.is_none()
                    && bot.pending_route.is_none()
                    && bot.route.is_some()
            })
        }),
        "the refreshed area route completed"
    );

    let (goal, arrival) = {
        let navs = rig.navs.lock().unwrap();
        let bot = navs.get("alice").unwrap();
        (
            bot.route.as_ref().expect("refreshed route").dest,
            bot.route_arrival,
        )
    };
    assert_eq!(arrival, nav::arrival::ArrivalKind::Area);
    assert_eq!(goal, WATER_SHORE);
    rig.step_at(goal);
    rig.observe_at(2, goal);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Arrived)));
}

#[test]
#[ignore = "requires WORLD_NAV_PACK pointing to the real 289 nav pack"]
fn real_catherby_water_centroid_accepts_area_shore_but_preserves_reach_refusal() {
    use crate::ScriptRouteRequest;
    use nav::arrival::ArrivalKind;

    let pack = std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK is required");
    let world = Arc::new(NavWorld::load_pack(std::path::Path::new(&pack)).unwrap());
    let centre = WorldTile {
        x: 2848,
        z: 3426,
        level: 0,
    };
    let shore = WorldTile {
        x: 2840,
        z: 3436,
        level: 0,
    };
    assert!(!world.collision.standable(centre));
    assert!(world.collision.standable(shore));
    let mut request = ScriptRouteRequest {
        generation: 1,
        request_id: 1,
        world,
        from: shore,
        to: centre,
        radius: 40,
        loc_id: None,
        arrival: ArrivalKind::Area,
        opts: FindOptions::default(),
        state: None,
        bank: Vec::new(),
        live_candidates: None,
        exclusions: None,
        completion: Default::default(),
    };
    let (area, _) = request.calculate();
    let RouteOutcome::Routed(route) = area else {
        panic!("Area arrival must accept the real standable Catherby shoreline");
    };
    assert_eq!(route.dest, shore);
    assert!(request.world.collision.standable(route.dest));
    request.arrival = ArrivalKind::Reach;
    let (reach, _) = request.calculate();
    assert!(matches!(reach, RouteOutcome::NoPath));
}

#[test]
fn lifecycle_followups_stop_clears_combat_raise_but_preserves_user_prayer() {
    let mut rig = open_rig(false);
    rig.slot().lock().unwrap().stop();
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests =
        Arc::new(api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap());
    let mut document: serde_json::Value = serde_json::from_str(include_str!(
        "../../script/paths/289/fixtures/combat_melee_upkeep.json"
    ))
    .unwrap();
    let args = &mut document["roles"][0]["sequences"][0]["steps"][0]["args"];
    args["target"]["npc"] = serde_json::json!("ardougne_archer");
    let path = script::quester::compile::compile_path(
        &serde_json::to_vec(&document).unwrap(),
        &selected,
        &quests,
    )
    .unwrap();
    let quester = script::quester::runner::Quester::new(
        api::selected::RunKey {
            slot: 0,
            run: 0,
            session: 0,
        },
        path,
        Arc::clone(&selected),
        quests,
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    );
    rig.slot()
        .lock()
        .unwrap()
        .start_test_script(Box::new(quester), Some(Arc::clone(&selected)))
        .unwrap();
    let here = WorldTile {
        x: 2457,
        z: 3302,
        level: 0,
    };
    rig.rebuild_snapshot_at(here);
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    seed_missile_launch(&mut rig.snapshot);
    rig.snapshot.seed_chat_lines(Vec::new());
    rig.snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Imp Catcher".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    let skin = selected.prayer_by_name("Thick Skin").unwrap();
    let protect = selected.prayer_by_name("Protect from Missiles").unwrap();
    let mut varps = rig.snapshot.varps().to_vec();
    varps
        .iter_mut()
        .find(|row| row.index == skin.varp)
        .unwrap()
        .value = 1;
    rig.snapshot.seed_varps(varps);
    let raised_tick = (1..=20)
        .find(|tick| {
            rig.snapshot.seed_tick(*tick);
            rig.observe_with_here(u64::from(*tick), here);
            rig.driver
                .if_button_components
                .contains(&protect.button_com)
        })
        .expect("the real Combat must admit a protect raise");
    // Stop before Combat polls that accepted receipt: ownership must include
    // the host-accepted raise, not only the machine's last polled mask.
    let before_stop = rig.driver.if_button_components.len();
    let mut varps = rig.snapshot.varps().to_vec();
    varps
        .iter_mut()
        .find(|row| row.index == protect.varp)
        .unwrap()
        .value = 1;
    rig.snapshot.seed_varps(varps);
    rig.slot().lock().unwrap().stop();
    rig.snapshot.seed_tick(raised_tick + 1);
    rig.observe_with_here(u64::from(raised_tick + 1), here);
    assert_eq!(
        &rig.driver.if_button_components[before_stop..],
        &[protect.button_com],
        "Stop owes only Combat's accepted protect, never the user's Thick Skin"
    );
    rig.observe_with_here(u64::from(raised_tick + 1), here);
    assert_eq!(rig.driver.if_button_components.len(), before_stop + 1);
    assert_eq!(
        rig.snapshot
            .varps()
            .iter()
            .find(|row| row.index == skin.varp)
            .unwrap()
            .value,
        1
    );
    let mut varps = rig.snapshot.varps().to_vec();
    varps
        .iter_mut()
        .find(|row| row.index == protect.varp)
        .unwrap()
        .value = 0;
    rig.snapshot.seed_varps(varps);
    rig.snapshot.seed_tick(raised_tick + 2);
    rig.observe_with_here(u64::from(raised_tick + 2), here);
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Idle);
    assert_eq!(rig.driver.if_button_components.len(), before_stop + 1);
}
