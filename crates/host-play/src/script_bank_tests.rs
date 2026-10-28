use super::harness::{native_pick_action, HeldDriver};
use super::*;
use api::named_banks::NamedBank;
use client::dash3d::CollisionFlag;
use nav_planner::collision::{pack_walk, WorldCollision};
use nav_planner::transport::TransportGraph;
use std::sync::atomic::{AtomicU64, Ordering};

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}
fn world(wall: bool) -> Arc<NavWorld> {
    let mut flags = vec![0; 4 * 16 * 16];
    if wall {
        for z in 0..16 {
            flags[z * 16 + 6] =
                CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;
        }
    }
    let (walk, blocked) = pack_walk(&flags);
    Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![],
    ))
}
fn navs(banks: Vec<NamedBank>) -> Arc<Mutex<HashMap<String, super::super::NavBot>>> {
    let mut bot = super::super::NavBot::default();
    bot.bank_pick.facts = Some(Arc::new(NamedBankFacts::from_banks(banks)));
    Arc::new(Mutex::new(HashMap::from([("test".to_string(), bot)])))
}
fn bank(name: &'static str, x: i32, z: i32) -> NamedBank {
    NamedBank::new(name, tile(x, z))
}

#[cfg(feature = "js")]
#[test]
fn bank_pick_v2_explicit_opt_ins_control_the_returned_bank_over_settings_defaults() {
    let catalog = api::named_banks::BANK_CATALOG;
    let mage = NamedBank {
        definition: catalog.iter().find(|b| b.name == "Mage Arena"),
        ..bank("Mage Arena", 3091, 3958)
    };
    let zanaris = NamedBank {
        definition: catalog.iter().find(|b| b.name == "Zanaris"),
        ..bank("Zanaris", 3092, 3958)
    };
    let canifis = NamedBank {
        definition: catalog.iter().find(|b| b.name == "Canifis"),
        routable: false,
        ..bank("Canifis", 3520, 3510)
    };
    let required_quests = ["Lost City", canifis.definition.unwrap().quest.unwrap()];
    let public_banks = [mage, zanaris, bank("Public", 3093, 3958)];
    for (banks, defaults, options, expected) in [
        (
            public_banks.to_vec(),
            false,
            "use_mage_bank: true, use_zanaris_bank: false",
            "Mage Arena",
        ),
        (
            public_banks.to_vec(),
            false,
            "use_mage_bank: false, use_zanaris_bank: true",
            "Zanaris",
        ),
        (
            public_banks.to_vec(),
            true,
            "use_mage_bank: false, use_zanaris_bank: false",
            "Public",
        ),
        (public_banks.to_vec(), true, "", "Mage Arena"),
        (vec![canifis], false, "", "Canifis"),
    ] {
        let source = format!(
            r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {{
  if (started) return;
  started = true;
  globalThis.result = await api.bankNearestReachable({{ from: {{x:3091,z:3958,level:0}}, {options} }});
}}
"#
        );
        let mut slot = script::SlotScript::new();
        slot.bind_incarnation(1);
        slot.bind_surface_factory(Arc::new(script_js::surface::JsSurfaceFactory::default()));
        let settings = serde_json::json!({"useMageBank": defaults, "useZanarisBank": defaults});
        let start = crate::script_runtime::js_surface_start(
            crate::script_runtime::js_start_card(source, script_js::LoadShape::NativeTick, vec![]),
            settings.as_object().cloned().map(Arc::new),
            Some(api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()),
            Arc::new(NamedBankFacts::from_banks(banks.clone())),
            Arc::from(Vec::<api::loadout::Loadout>::new()),
        );
        slot.start_surface_with_settings(start).unwrap();
        let ready_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match slot.poll_start() {
                script::slot::StartPoll::Settled(script::slot::StartOutcome::Ready) => break,
                script::slot::StartPoll::Pending => {
                    assert!(Instant::now() < ready_deadline, "Load start stalled");
                    std::thread::yield_now();
                }
                other => panic!("Load start failed: {other:?}"),
            }
        }
        slot.on_is_up(true);
        let navs = navs(banks);
        let world = Some(world(false));
        let mut state = WorldState::empty();
        state.quests.extend(required_quests.map(String::from));
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_tick(1);
        snapshot.seed_tile(tile(3091, 3958));
        snapshot.seed_quest_statuses(
            required_quests
                .iter()
                .map(|name| api::snapshot::QuestStatusView {
                    component_id: -1,
                    name: (*name).into(),
                    colour: 0x00F800,
                })
                .collect(),
            true,
        );
        let post = |slot: &mut script::SlotScript, game_tick: bool| {
            let bank_selection = slot.bank_selection_input();
            super::super::with_script_snapshot_input_shorts(
                1,
                Some((3091, 3958, 0)),
                true,
                None,
                Some(&snapshot),
                None,
                world.as_deref(),
                None,
                false,
                false,
                false,
                0,
                false,
                0,
                false,
                None,
                Default::default(),
                &[],
                Default::default(),
                bank_selection,
                None,
                None,
                0,
                |input, native| {
                    assert!(slot.post_snapshot_delta(input, native, false));
                },
            );
            let (_, legacy, _) = slot.drain_host_interacts();
            assert!(legacy.is_empty(), "select uses the typed native seat");
            let mut cx = script::ScriptCtx {
                driver: &mut HeldDriver,
                tick: 1,
                here: Some((3091, 3958, 0)),
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: Some(&snapshot),
                obj_names: None,
                compiled: script::CompiledTick::default(),
            };
            if game_tick {
                slot.on_game_tick(&mut cx);
            } else {
                slot.on_snapshot_change(&mut cx);
            }
            slot.probe("true").unwrap();
        };
        post(&mut slot, true);
        let action = (0..8)
            .find_map(|_| {
                post(&mut slot, false);
                slot.take_native_action()
            })
            .expect("v2 select starts the real native bank picker");
        let authority = action.authority();
        let script::native::HostEffect::BankPick(request) = action.effect else {
            panic!("v2 select must emit a BankPick action");
        };
        assert!(request.allow_air_fallback);
        let evidence = EvidenceStamp {
            run: authority.run(),
            tick: 1,
            sequence: 1,
        };
        queue_native_bank_pick_scoped(
            &navs,
            "test",
            api::identity::Scope::Slot(api::identity::SlotKey::from_incarnation(1).unwrap()),
            &world,
            Some(&state),
            NativeBankPickInput {
                request,
                authority,
                evidence,
            },
            None,
        );
        let deadline = Instant::now() + BANK_SELECTION_WINDOW;
        let (authority, receipt) = loop {
            poll_bank_pick(&navs, "test", Instant::now());
            if let Some(receipt) = navs
                .lock()
                .unwrap()
                .get_mut("test")
                .unwrap()
                .bank_pick
                .take_native_receipt()
            {
                break receipt;
            }
            assert!(Instant::now() < deadline, "native select service stalled");
            std::thread::yield_now();
        };
        if expected == "Canifis" {
            assert_eq!(receipt.selected.bank_index, 0);
            assert_eq!(receipt.selected.kind, NativePickKind::AirFallback);
            assert!(receipt.selected.access.is_none());
        }
        slot.complete_native_bank_pick(&authority, receipt);
        for _ in 0..8 {
            post(&mut slot, false);
        }
        assert_eq!(
            slot.probe("globalThis.result.ok && globalThis.result.value.name")
                .unwrap(),
            expected
        );
        let publication = slot
            .probe("globalThis.__rs2b0t_host.snapshot.bank_selection")
            .unwrap();
        assert_eq!(publication["bank"]["name"], expected);
        if expected == "Canifis" {
            assert_eq!(publication["kind"], "fallback");
        }
        assert!(
            navs.lock().unwrap()["test"].route.is_none(),
            "selection must not move"
        );
        slot.stop();
    }
}

/// Dispatch borrows each frame's gating facts. A queued bank selection keeps
/// frame A's facts when the next frame lends different facts and repeats the walk.
#[test]
fn queued_bank_pick_keeps_the_state_of_the_frame_that_queued_it() {
    let navs = navs(vec![bank("bank", 12, 12)]);
    let world = Some(world(false));
    let mut frame_a = WorldState::empty();
    frame_a.quests.insert("Frame A".into());
    let mut frame_b = WorldState::empty();
    frame_b.quests.insert("Frame B".into());
    let scope = api::identity::Scope::Slot(api::identity::SlotKey::from_incarnation(1).unwrap());
    let mut driver = HeldDriver;
    let snapshot = GameSnapshot::new();
    for state in [&frame_a, &frame_b] {
        super::super::dispatch_script_interact_cached_scoped(
            &mut driver,
            &super::super::InteractContext {
                snapshot: &snapshot,
                obj_names: None,
                here: Some((0, 0, 0)),
                navs: &navs,
                world: &world,
                state: Some(state),
                bank: None,
                name: "test",
                compatibility_scope: Some(scope),
                cache: &None,
                obj_names_arc: &None,
            },
            vec![script::shim::InteractReq::WalkNearestBank],
        );
    }
    let all = navs.lock().unwrap();
    let active = all["test"]
        .bank_pick
        .current
        .as_ref()
        .expect("a far routable bank queues selection work");
    assert_eq!(*active.job.request.state, frame_a);
}

#[test]
fn typed_select_air_fallback_is_opt_in_when_no_route_target_exists() {
    let canifis = NamedBank {
        routable: false,
        ..bank("Canifis", 8, 8)
    };
    static APPROACHED: api::named_banks::BankDefinition = api::named_banks::BankDefinition {
        name: "Approached",
        tile: WorldTile {
            x: 100,
            z: 100,
            level: 0,
        },
        approach: Some(WorldTile {
            x: 10,
            z: 8,
            level: 3,
        }),
        skill: None,
        quest: None,
        setting: None,
        object: None,
        open_first: None,
        npc: None,
        choose: None,
    };
    let approached = NamedBank {
        definition: Some(&APPROACHED),
        routable: false,
        ..bank("Approached", 100, 100)
    };
    let tied = NamedBank {
        routable: false,
        ..bank("Tied", 6, 8)
    };
    let world = Some(world(false));
    for (banks, expected_air) in [
        (vec![canifis], canifis.air_tile()),
        (vec![approached, tied], approached.air_tile()),
    ] {
        for allow_air_fallback in [false, true] {
            let (mut slot, action) =
                native_pick_action(banks.clone(), tile(8, 8), None, allow_air_fallback);
            let authority = action.authority();
            let script::native::HostEffect::BankPick(request) = action.effect else {
                panic!("Select must emit its typed bank request");
            };
            let candidates = bank_candidates(
                world.as_deref().unwrap(),
                &request.facts,
                request.from,
                &WorldState::empty(),
                request.preferences,
                None,
                Some(&request),
            );
            assert_eq!(candidates.target_count, 0);
            assert_eq!(
                candidates.order.len(),
                if allow_air_fallback { banks.len() } else { 0 }
            );
            assert!(
                candidates
                    .order
                    .iter()
                    .enumerate()
                    .all(|(index, candidate)| index == candidate.index),
                "air order uses approach, ignores plane and preserves equal-distance roster order"
            );
            let navs = navs(banks.clone());
            let evidence = EvidenceStamp {
                run: authority.run(),
                tick: 1,
                sequence: 1,
            };
            queue_native_bank_pick_scoped(
                &navs,
                "test",
                api::identity::Scope::Slot(api::identity::SlotKey::from_incarnation(1).unwrap()),
                &world,
                Some(&WorldState::empty()),
                NativeBankPickInput {
                    request,
                    authority,
                    evidence,
                },
                None,
            );
            let (_, receipt) = navs
                .lock()
                .unwrap()
                .get_mut("test")
                .unwrap()
                .bank_pick
                .take_native_receipt()
                .expect("zero-target selection settles inline");
            if allow_air_fallback {
                assert_eq!(receipt.selected.bank_index, 0);
                assert_eq!(receipt.selected.kind, NativePickKind::AirFallback);
                assert_eq!(receipt.selected.access_tile, expected_air);
            } else {
                assert_eq!(receipt.selected.bank_index, u16::MAX);
                assert_eq!(receipt.selected.kind, NativePickKind::NoCandidate);
            }
            assert!(receipt.selected.access.is_none());
            slot.stop();
        }
    }
}

#[test]
fn typed_select_routes_before_using_a_closer_air_fallback_with_compatibility_bounds() {
    let canifis = NamedBank {
        routable: false,
        ..bank("Canifis", 8, 8)
    };
    let public = bank("Public", 14, 8);
    let banks = vec![canifis, public];
    let (walk, blocked) = pack_walk(&vec![0; 4 * 16 * 16]);
    let world = Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![nav_planner::pack::BankStand {
            name: "Bank booth".into(),
            tile: public.tile,
            access: nav_planner::pack::BankAccess::Booth { op: 2 },
        }],
    ));
    let (mut slot, action) = native_pick_action(banks.clone(), tile(8, 8), None, true);
    let authority = action.authority();
    let script::native::HostEffect::BankPick(request) = action.effect else {
        panic!("Select must emit its typed bank request");
    };
    let evidence = EvidenceStamp {
        run: authority.run(),
        tick: 1,
        sequence: 1,
    };
    let policy = super::harness::request(
        &world,
        authority.request_id().get(),
        1,
        request.from,
        WorldState::empty(),
        false,
        Some(NativeBankPickInput {
            request: request.clone(),
            authority: authority.clone(),
            evidence,
        }),
    );
    assert_eq!(
        policy.first(),
        Some(0),
        "legacy air order keeps the closer bank"
    );
    assert!(policy.target_count > 0);
    assert_eq!(policy.target_budget(), 500_001);
    assert!(matches!(
        policy.search_shape(false),
        Some((
            BankPickSearchKind::First,
            SearchRequestShape::First {
                max_settles: 500_001,
                ..
            }
        ))
    ));
    assert_eq!(BANK_SELECTION_WINDOW, Duration::from_secs(5));
    assert!(near_bank(tile(8, 8), tile(12, 12)));
    assert!(!near_bank(tile(8, 8), tile(13, 8)));
    assert!(!near_bank(
        tile(8, 8),
        WorldTile {
            x: 8,
            z: 8,
            level: 1
        }
    ));
    let navs = navs(banks);
    queue_native_bank_pick_scoped(
        &navs,
        "test",
        api::identity::Scope::Slot(api::identity::SlotKey::from_incarnation(1).unwrap()),
        &Some(world),
        Some(&WorldState::empty()),
        NativeBankPickInput {
            request,
            authority,
            evidence,
        },
        None,
    );
    let deadline = Instant::now() + BANK_SELECTION_WINDOW;
    let (_, receipt) = loop {
        poll_bank_pick(&navs, "test", Instant::now());
        if let Some(receipt) = navs
            .lock()
            .unwrap()
            .get_mut("test")
            .unwrap()
            .bank_pick
            .take_native_receipt()
        {
            break receipt;
        }
        assert!(Instant::now() < deadline, "bounded route selection stalled");
        std::thread::yield_now();
    };
    assert_eq!(receipt.selected.bank_index, 1);
    assert_eq!(receipt.selected.kind, NativePickKind::Reachable);
    assert!(receipt.selected.access.is_some());
    assert!(
        navs.lock().unwrap()["test"].route.is_none(),
        "select does not walk"
    );
    slot.stop();
}

#[test]
fn native_bank_access_resolves_teller_and_object_metadata() {
    let flags = vec![0; 4 * 16 * 16];
    let (walk, blocked) = pack_walk(&flags);
    let teller = nav_planner::pack::BankStand {
        name: "Gundai".to_owned(),
        tile: tile(8, 8),
        access: nav_planner::pack::BankAccess::Npc {
            name: "Gundai".to_owned(),
            op: 4,
            choose: None,
        },
    };
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![
            nav_planner::pack::BankStand {
                name: "Bank booth".to_owned(),
                tile: tile(7, 8),
                access: nav_planner::pack::BankAccess::Booth { op: 2 },
            },
            teller,
        ],
    );
    let definition = api::named_banks::BANK_CATALOG
        .iter()
        .find(|definition| definition.name == "Mage Arena")
        .unwrap();
    let mage_bank = api::named_banks::NamedBank {
        name: definition.name,
        tile: tile(8, 8),
        definition: Some(definition),
        routable: true,
    };
    let (access_tiles, access) = native_bank_access(&world, mage_bank).unwrap();
    let access_tile = access_tiles[0];
    assert_ne!(access_tile, access.stand_tile);
    assert_eq!(access_tile.level, access.stand_tile.level);
    assert_eq!(access.kind, NativeAccessKind::Teller);
    assert_eq!(access.name.as_deref(), Some("Gundai"));
    assert_eq!(access.stand_op, 4);
    assert_eq!(
        access.choose.as_deref(),
        Some("I'd like to access my bank account")
    );

    let shantay = api::named_banks::BANK_CATALOG
        .iter()
        .find(|definition| definition.name == "Shantay Pass")
        .unwrap();
    let shantay_bank = api::named_banks::NamedBank {
        name: shantay.name,
        tile: shantay.tile,
        definition: Some(shantay),
        routable: true,
    };
    let (access_tiles, access) = native_bank_access(&world, shantay_bank).unwrap();
    let chest_tile = api::snapshot::WorldTile {
        x: shantay.tile.x + 1,
        z: shantay.tile.z,
        level: shantay.tile.level,
    };
    assert!(access_tiles.contains(&access.stand_tile));
    assert_ne!(access.stand_tile, chest_tile);
    assert_eq!(access.stand_tile.level, chest_tile.level);
    assert_eq!(
        access
            .stand_tile
            .x
            .abs_diff(chest_tile.x)
            .max(access.stand_tile.z.abs_diff(chest_tile.z)),
        1
    );
    assert_eq!(access.kind, NativeAccessKind::Booth);
    assert_eq!(access.name.as_deref(), Some("Shantay chest"));
    assert_eq!(access.stand_op, 0);
}

#[test]
fn native_bank_access_declared_teller_does_not_require_packed_geometry() {
    let definition = api::named_banks::BANK_CATALOG
        .iter()
        .find(|definition| definition.name == "Mage Arena")
        .unwrap();
    for stands in [
        Vec::new(),
        vec![
            nav_planner::pack::BankStand {
                name: "Bank booth".to_owned(),
                tile: definition.tile,
                access: nav_planner::pack::BankAccess::Booth { op: 2 },
            },
            nav_planner::pack::BankStand {
                name: "Other teller".to_owned(),
                tile: definition.tile,
                access: nav_planner::pack::BankAccess::Npc {
                    name: "Other teller".to_owned(),
                    op: 1,
                    choose: None,
                },
            },
        ],
    ] {
        let flags = vec![0; 4 * 16 * 16];
        let (walk, blocked) = pack_walk(&flags);
        let world = NavWorld::from_parts(
            WorldCollision {
                origin: tile(definition.tile.x - 8, definition.tile.z - 8),
                width: 16,
                height: 16,
                walk,
                blocked,
                flags: None,
            },
            TransportGraph::default(),
            stands,
        );
        let mage_bank = api::named_banks::NamedBank {
            name: definition.name,
            tile: definition.tile,
            definition: Some(definition),
            routable: true,
        };
        let (access_tiles, access) = native_bank_access(&world, mage_bank).unwrap();
        assert_eq!(access_tiles, vec![definition.tile]);
        assert_eq!(access.stand_tile, definition.tile);
        assert_eq!(access.kind, NativeAccessKind::Teller);
        assert_eq!(access.name.as_deref(), Some("Gundai"));
        assert_eq!(access.stand_op, 0);
        assert_eq!(
            access.choose.as_deref(),
            Some("I'd like to access my bank account")
        );
        assert!(native_bank_access(&world, bank("Unmapped", 8, 8)).is_none());
    }
}

#[test]
fn bank_label_slots_are_one_based_first_match_over_holes() {
    let actions: Vec<Option<String>> = vec![
        None,
        Some("Withdraw-5".into()),
        Some("withdraw 5".into()),
        Some("Deposit_All".into()),
    ];
    assert_eq!(ops::action_op_in(&actions, "Withdraw 5"), Some(2));
    assert_eq!(ops::action_op_in(&actions, "withdraw_5"), Some(2));
    assert_eq!(ops::action_op_in(&actions, "WITHDRAW-5"), Some(2));
    assert_eq!(ops::action_op_in(&actions, "Deposit All"), Some(4));
    assert_eq!(ops::action_op_in(&actions, "Withdraw-1"), None);
    assert_eq!(ops::action_op_in(&[], "Withdraw-5"), None);
}

/// Characterization (T2.2): the All slot is the first label whose
/// normalized form merely contains `all` — even inside another word —
/// with no last-label fallback.
#[test]
fn bank_all_slot_is_first_containing_all_without_fallback() {
    let actions: Vec<Option<String>> = vec![
        Some("Deposit-1".into()),
        None,
        Some("Deposit-All".into()),
        Some("Deposit-X".into()),
    ];
    assert_eq!(ops::all_op_in(&actions), Some(3));
    assert_eq!(ops::all_op_in(&[Some("Small".into())]), Some(1));
    assert_eq!(
        ops::all_op_in(&[Some("Deposit-1".into()), Some("Deposit-X".into())]),
        None
    );
    assert_eq!(ops::all_op_in(&[]), None);
}

/// Characterization (T2.2): the fill order is an exact `Withdraw {count}`
/// label first (even when the count is the whole stack), then All only
/// when the fill consumes the current stock, then the X continuation.
#[test]
fn bank_fill_is_exact_then_whole_stock_all_then_x() {
    let actions: Vec<Option<String>> = vec![
        Some("Withdraw-1".into()),
        Some("Withdraw-5".into()),
        Some("Withdraw-10".into()),
        Some("Withdraw-All".into()),
        Some("Withdraw-X".into()),
    ];
    assert_eq!(ops::fill_click_in(&actions, 1, 50), Some((1, false)));
    assert_eq!(ops::fill_click_in(&actions, 5, 5), Some((2, false)));
    assert_eq!(ops::fill_click_in(&actions, 10, 50), Some((3, false)));
    assert_eq!(ops::fill_click_in(&actions, 50, 50), Some((4, false)));
    assert_eq!(ops::fill_click_in(&actions, 7, 50), Some((5, true)));
    assert_eq!(ops::fill_click_in(&actions, 7, 7), Some((4, false)));
    assert_eq!(ops::fill_click_in(&actions, 0, 50), Some((5, true)));
    // No exact, not the whole stack, no X: no fill click at all.
    let fixed: Vec<Option<String>> = vec![Some("Withdraw-1".into()), Some("Withdraw-All".into())];
    assert_eq!(ops::fill_click_in(&fixed, 7, 50), None);
    // The whole stack without an All label still routes through X.
    let no_all: Vec<Option<String>> = vec![Some("Withdraw-1".into()), Some("Withdraw-X".into())];
    assert_eq!(ops::fill_click_in(&no_all, 7, 7), Some((2, true)));
}

/// Characterization (T2.2): only the resolved Withdraw-X label is the
/// open-only dialog op; every other slot settles on the observed delta.
#[test]
fn bank_open_only_withdraw_x_reads_the_resolved_label() {
    let actions: Vec<Option<String>> = vec![
        Some("Withdraw-1".into()),
        Some("Withdraw-X".into()),
        Some("withdraw x".into()),
    ];
    assert!(!open_only_withdraw_x(&actions, 1));
    assert!(open_only_withdraw_x(&actions, 2));
    assert!(open_only_withdraw_x(&actions, 3));
    assert!(!open_only_withdraw_x(&actions, 0));
    assert!(!open_only_withdraw_x(&actions, 4));
    assert!(!open_only_withdraw_x(&actions, -1));
}

fn teller(
    index: usize,
    name: &str,
    x: i32,
    z: i32,
    level: i32,
    distance: i32,
) -> api::snapshot::NpcView {
    let tile = WorldTile { x, z, level };
    api::snapshot::NpcView {
        index,
        r#type: Some(1),
        name: Some(name.into()),
        actions: vec![Some("Bank".into())],
        tile,
        distance,
        animation: 0,
        animation_frame: 0,
        pose_animation: 0,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 0,
        total_health: 0,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 1,
        network: tile,
        x,
        z,
        yaw: 0,
    }
}

/// Characterization (T2.2 r2): the host teller pick keys on
/// `(d_stand, d_here, distance, index)` in that order, after the
/// trimmed-name filter and the SAME_BANK-of-stand / 10-of-here gate
/// (Chebyshev, +10_000 per plane mismatch).
#[test]
fn teller_pick_keys_on_stand_then_here_then_distance_then_index() {
    let stand = WorldTile {
        x: 100,
        z: 100,
        level: 0,
    };
    let here = Some((90, 100, 0));
    let pick = |rows: &[api::snapshot::NpcView]| {
        teller_pick(rows, here, stand, "Banker").map(|npc| npc.index)
    };
    // d_stand dominates the posted distance.
    assert_eq!(
        pick(&[
            teller(11, "Banker", 103, 100, 0, 1),
            teller(10, "Banker", 102, 100, 0, 7),
        ]),
        Some(10)
    );
    // A d_stand tie is decided by d_here, not the posted distance.
    assert_eq!(
        pick(&[
            teller(10, "Banker", 102, 100, 0, 1),
            teller(11, "Banker", 98, 100, 0, 9),
        ]),
        Some(11)
    );
    // A (d_stand, d_here) tie is decided by distance, then index.
    // (102, 95) and (102, 105) are both Chebyshev 5 from the stand and
    // 12 from here.
    assert_eq!(
        pick(&[
            teller(10, "Banker", 102, 95, 0, 8),
            teller(11, "Banker", 102, 105, 0, 3),
        ]),
        Some(11)
    );
    assert_eq!(
        pick(&[
            teller(12, "Banker", 102, 95, 0, 3),
            teller(10, "Banker", 102, 105, 0, 3),
        ]),
        Some(10)
    );
    // Name filter: trimmed and case-folded; a wrong name at the stand
    // loses to the named teller farther away.
    assert_eq!(
        pick(&[
            teller(10, "Trader", 100, 100, 0, 0),
            teller(11, " banker ", 104, 100, 0, 5),
        ]),
        Some(11)
    );
    // Gate: beyond SAME_BANK of the stand and beyond 10 of here — and a
    // plane mismatch prices a teller out entirely. Within 10 of here
    // alone is enough.
    assert_eq!(pick(&[teller(10, "Banker", 120, 100, 0, 1)]), None);
    assert_eq!(pick(&[teller(10, "Banker", 100, 100, 1, 0)]), None);
    assert_eq!(pick(&[teller(10, "Banker", 85, 100, 0, 4)]), Some(10));
    // With no `here`, d_here is i32::MAX for every row and the stand
    // distance alone decides among the gated rows.
    assert_eq!(
        teller_pick(
            &[
                teller(10, "Banker", 104, 100, 0, 1),
                teller(11, "Banker", 101, 100, 0, 9),
            ],
            None,
            stand,
            "Banker",
        )
        .map(|npc| npc.index),
        Some(11)
    );
}

/// This module pairs fixed-base request construction with each tree's
/// production calculator for the real-content old/new result capture. It
/// does not add a private worker.
#[cfg(test)]
mod bank_pick_capture {
    use super::super::{
        BankPickJob, BankPickRequest, BankPickSpec, BankPickStepGate, BankPickWorkerJob,
        LegacyPickResult, PickKind, PickResult,
    };
    use super::*;
    use api::identity::{ProcessIncarnation, Settlement, SlotKey};
    use nav_planner::router::{SearchCursor, SearchPreflight, SearchRequest, SearchRequestShape};
    use std::fmt::Debug;
    use std::num::NonZeroU64;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::super::harness::request;
    fn request_for_banks(
        world: &Arc<NavWorld>,
        banks: Vec<NamedBank>,
        request_id: u64,
        generation: u64,
        walking: bool,
    ) -> Arc<BankPickRequest> {
        let facts = Arc::new(NamedBankFacts::from_banks(banks));
        let from = tile(0, 0);
        let state = WorldState::default();
        let BankCandidates {
            order,
            target_count,
        } = super::super::bank_candidates(
            world,
            &facts,
            from,
            &state,
            BankPreferences::default(),
            None,
            None,
        );
        super::super::make_bank_pick_request(
            request_id,
            nav_planner::producer::lease(world).unwrap(),
            facts,
            from,
            FindOptions::default(),
            state,
            order,
            target_count,
            (walking && target_count != 0).then_some(generation),
            None,
        )
    }

    fn legacy_result(
        request: &BankPickRequest,
        world: &NavWorld,
        walking: bool,
    ) -> LegacyPickResult {
        let first = request.first();
        let near = request.native.is_none()
            && first.is_some_and(|index| near_bank(request.from, request.route_target(index)));
        if first.is_none() || (near && !walking) {
            LegacyPickResult {
                index: first,
                access_tile: None,
                kind: if near {
                    PickKind::NearShortcut
                } else {
                    PickKind::NoCandidate
                },
                route: None,
            }
        } else {
            request.legacy_calculate(world, near)
        }
    }

    fn append_tile(out: &mut Vec<u8>, tile: WorldTile) {
        out.extend_from_slice(&tile.x.to_le_bytes());
        out.extend_from_slice(&tile.z.to_le_bytes());
        out.extend_from_slice(&tile.level.to_le_bytes());
    }

    fn append_blob(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value);
    }

    fn outcome_bytes(
        label: &str,
        request: &BankPickRequest,
        generation: u64,
        index: Option<usize>,
        access_tile: Option<WorldTile>,
        kind: PickKind,
        route: Option<&dyn Debug>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        append_blob(&mut out, label.as_bytes());
        out.extend_from_slice(&generation.to_le_bytes());
        out.extend_from_slice(&request.request_id.to_le_bytes());
        out.extend_from_slice(&index.map_or(-1, |index| index as i32).to_le_bytes());
        out.push(kind as u8);
        match index {
            Some(index) => {
                out.push(1);
                let bank = request.facts.banks()[index];
                append_blob(&mut out, bank.name.as_bytes());
                append_tile(&mut out, bank.tile);
                append_tile(&mut out, bank.air_tile());
                out.push(u8::from(bank.routable));
            }
            None => out.push(0),
        }
        match access_tile {
            Some(tile) => {
                out.push(1);
                append_tile(&mut out, tile);
            }
            None => out.push(0),
        }
        if let Some(route) = route {
            out.push(1);
            append_blob(&mut out, format!("{route:?}").as_bytes());
        } else {
            out.push(0);
        }
        if request.native.is_some() {
            let selected = request.selected(index, kind, access_tile);
            append_blob(&mut out, format!("{selected:?}").as_bytes());
        } else {
            append_blob(&mut out, &[]);
        }
        out
    }

    fn service() -> Arc<parking_lot::Mutex<WorkService>> {
        static NEXT_NONCE: AtomicU64 = AtomicU64::new(1);
        let nonce = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
        let incarnation = ProcessIncarnation::from_start_nonce(
            NonZeroU64::new(nonce).expect("test service nonce remains nonzero"),
        );
        Arc::new(parking_lot::Mutex::new(WorkService::with_clock_for_tests(
            incarnation,
            Arc::new(api::work::SystemClock),
        )))
    }

    fn wait_for_returned_capacity(
        service: &Arc<parking_lot::Mutex<WorkService>>,
        expected_jobs: usize,
    ) {
        // Terminal publication precedes the request thread's final charge return.
        loop {
            let observed = service.lock().publications().capacity().sequence();
            if service.lock().capacity().jobs() == expected_jobs {
                break;
            }
            WorkService::wait_for_capacity_change(service, observed);
        }
    }

    struct BankPickGateRelease(Option<mpsc::SyncSender<()>>);

    impl BankPickGateRelease {
        fn release(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    impl Drop for BankPickGateRelease {
        fn drop(&mut self) {
            self.release();
        }
    }

    fn scope() -> Scope {
        Scope::Slot(SlotKey::from_incarnation(1).unwrap())
    }

    #[test]
    fn stored_ticket_reconstructs_exact_revision_and_rejects_wrong_revision() {
        let service = service();
        let scope = scope();
        let world = world(false);
        let request = request_for_banks(&world, vec![bank("bank", 8, 8)], 1, 1, true);
        let (input, preflights) = request
            .work_input(1, false, "test")
            .expect("test request has a bounded route search");
        let job = BankPickJob {
            generation: 1,
            request,
            deadline: None,
        };
        let work = BankPickWorkerJob::for_job("test", job, input, preflights);
        let ticket = {
            let mut service = service.lock();
            let identity = service.mint_identity(scope, None).unwrap();
            let conditions = ProcessConditions::new(service.incarnation());
            service
                .try_submit::<BankPickSpec, _>(identity, &input, work, &conditions)
                .unwrap_or_else(|refused| {
                    let (reason, _) = refused.into_parts();
                    panic!("ticket test submission refused: {reason:?}")
                })
        };
        WorkService::wait_terminal(&service, &ticket).unwrap();

        let stored = super::super::BankPickTicketRef::from_ticket(ticket);
        let reconstructed = stored.ticket();
        assert_eq!(reconstructed.identity(), ticket.identity());
        assert_eq!(reconstructed.revision(), ticket.revision());
        assert!(service.lock().borrow_terminal(&reconstructed).is_ok());

        let wrong_revision = WorkTicket::<BankPickSpec>::new(
            stored.identity,
            stored.revision.next().expect("test revision can advance"),
        );
        assert!(matches!(
            service.lock().borrow_terminal(&wrong_revision),
            Err(api::work::LedgerError::StaleIdentity)
        ));
    }

    fn cooperative_bytes(
        service: &Arc<parking_lot::Mutex<WorkService>>,
        scope: Scope,
        label: &str,
        request: Arc<BankPickRequest>,
        generation: u64,
        walking: bool,
    ) -> Vec<u8> {
        let first = request.first();
        let near = request.native.is_none()
            && first.is_some_and(|index| near_bank(request.from, request.route_target(index)));
        if first.is_none() || (near && !walking) {
            let kind = if near {
                PickKind::NearShortcut
            } else {
                PickKind::NoCandidate
            };
            let result = PickResult::plain(first, None, kind);
            let bytes = outcome_bytes(
                label,
                &request,
                generation,
                result.index,
                result.access_tile,
                result.kind,
                None,
            );
            let retained = result.owner_result_bytes().unwrap();
            eprintln!("bank_pick_result_bytes label={label} bytes={retained}");
            let reservation = nav_planner::producer::OwnerResultLedger::shared()
                .try_reserve_bytes(retained)
                .unwrap();
            drop(reservation.hold(result));
            return bytes;
        }

        let route_only = near;
        let (input, preflights) = request
            .work_input(generation, route_only, "test")
            .expect("capture request has a bounded route search");
        let job = BankPickJob {
            generation,
            request: Arc::clone(&request),
            deadline: None,
        };
        let work = BankPickWorkerJob::for_job("test", job, input, preflights);
        let ticket = {
            let mut service = service.lock();
            let identity = service.mint_identity(scope, None).unwrap();
            let conditions = ProcessConditions::new(service.incarnation());
            service
                .try_submit::<BankPickSpec, _>(identity, &input, work, &conditions)
                .unwrap_or_else(|refused| {
                    let (reason, _) = refused.into_parts();
                    panic!("real-content bank-pick capture refused: {reason:?}")
                })
        };
        WorkService::wait_terminal(service, &ticket)
            .expect("cooperative bank-pick capture returns");
        let result_bytes = {
            let service = service.lock();
            match service.borrow_terminal(&ticket).unwrap() {
                Some(Settlement::Completed(result)) => result
                    .owner_result_bytes()
                    .expect("captured result has a bounded owner charge"),
                _ => panic!("cooperative capture did not complete"),
            }
        };
        eprintln!("bank_pick_result_bytes label={label} bytes={result_bytes}");
        let reservation = nav_planner::producer::OwnerResultLedger::shared()
            .try_reserve_bytes(result_bytes)
            .expect("captured result fits exact owner custody");
        let result = match service.lock().consume(&ticket).unwrap() {
            Settlement::Completed(result) => result,
            _ => panic!("captured result changed before consume"),
        };
        let route_debug = result.route.as_ref().map(|route| route as &dyn Debug);
        if result.route.is_some() {
            assert!(
                result.nav_lease.is_some(),
                "route result retains its nav lease"
            );
        }
        let bytes = outcome_bytes(
            label,
            &request,
            generation,
            result.index,
            result.access_tile,
            result.kind,
            route_debug,
        );
        let owner = result.into_owner_result(ticket.identity(), ticket.revision(), reservation);
        if let Some(plan) = owner.value().plan.as_ref() {
            assert_eq!(plan.lease.identity(), ticket.identity());
            assert_eq!(plan.lease.revision(), ticket.revision());
            assert!(plan.lease.view(&request.lease).is_ok());
        }
        drop(owner);
        bytes
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The capture compares the same complete real request against both implementations."
    )]
    fn append_case(
        output: &mut Vec<u8>,
        service: &Arc<parking_lot::Mutex<WorkService>>,
        scope: Scope,
        label: &str,
        world: &Arc<NavWorld>,
        request_id: u64,
        generation: u64,
        from: WorldTile,
        state: WorldState,
        walking: bool,
        native: Option<NativeBankPickInput>,
    ) {
        let request = request(world, request_id, generation, from, state, walking, native);
        let legacy = legacy_result(&request, world, walking);
        let legacy_route = legacy.route.as_ref().map(|route| route as &dyn Debug);
        let old = outcome_bytes(
            label,
            &request,
            generation,
            legacy.index,
            legacy.access_tile,
            legacy.kind,
            legacy_route,
        );
        let new = cooperative_bytes(
            service,
            scope,
            label,
            Arc::clone(&request),
            generation,
            walking,
        );
        assert_eq!(old, new, "old/new bank-pick bytes differ for {label}");
        append_blob(output, &old);
    }

    #[test]
    fn native_bank_selection_reaches_a_bank_from_falador_chickens() {
        let Some(pack) = script::ctx::test_support::local_asset_path(
            "NAV_PACK",
            "",
            "real 289 navpack for Falador chicken bank selection",
        ) else {
            return;
        };
        let loaded = NavWorld::load_pack(&pack).unwrap();
        loaded
            .bind_named_bank_facts(
                &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
            )
            .unwrap();
        let world = Arc::new(loaded);
        let banks = world.named_bank_facts().unwrap().banks().to_vec();
        let service = service();
        for from in (3013..=3045)
            .flat_map(|x| (3278..=3310).map(move |z| tile(x, z)))
            .filter(|from| world.collision.walkable(*from))
        {
            let (slot, action) = native_pick_action(banks.clone(), from, None, false);
            let authority = action.authority();
            let script::native::HostEffect::BankPick(native_request) = action.effect else {
                panic!("native Select must emit its real bank request");
            };
            let request = request(
                &world,
                authority.request_id().get(),
                1,
                from,
                WorldState::default().with_map_members(true),
                false,
                Some(NativeBankPickInput {
                    request: native_request,
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 1,
                        sequence: 1,
                    },
                    authority,
                }),
            );
            let (input, preflights) = request
                .work_input(1, false, "test")
                .expect("chicken bank selection must fit its bounded work charge");
            let work = BankPickWorkerJob::for_job(
                "test",
                BankPickJob {
                    generation: 1,
                    request: Arc::clone(&request),
                    deadline: None,
                },
                input,
                preflights,
            );
            let ticket = {
                let mut service = service.lock();
                let identity = service.mint_identity(scope(), None).unwrap();
                let conditions = ProcessConditions::new(service.incarnation());
                service
                    .try_submit::<BankPickSpec, _>(identity, &input, work, &conditions)
                    .unwrap_or_else(|refused| panic!("bank selection refused: {:?}", refused.into_parts().0))
            };
            WorkService::wait_terminal(&service, &ticket).unwrap();
            let Settlement::Completed(result) = service.lock().consume(&ticket).unwrap() else {
                panic!("bank selection did not complete");
            };
            eprintln!("chicken bank from={from:?} kind={:?} bank={:?} access={:?}",
                result.kind, result.index.map(|index| banks[index].name), result.access_tile);
            assert_eq!(result.kind, PickKind::Reachable, "from={from:?}");
            assert!(result.access_tile.is_some());
            assert!(result.index.is_some());
            drop(slot);
        }
    }

    #[test]
    #[ignore = "requires NAV_PACK for the immutable real 289 navpack and BOT_BANK_PICK_CAPTURE_OUT"]
    fn captures_real_content_bank_pick_outcomes() {
        let pack = std::env::var_os("NAV_PACK")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
            .expect("set NAV_PACK to the reviewed real 289 navpack");
        let loaded = NavWorld::load_pack(&pack).unwrap();
        loaded
            .bind_named_bank_facts(
                &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
            )
            .unwrap();
        let world = Arc::new(loaded);
        let state = WorldState::default().with_map_members(true);
        let service = service();
        let scope = scope();
        let mut output = Vec::new();

        append_case(
            &mut output,
            &service,
            scope,
            "compat-select-far",
            &world,
            1,
            1,
            tile(3080, 3472),
            state.clone(),
            false,
            None,
        );
        append_case(
            &mut output,
            &service,
            scope,
            "compat-select-near",
            &world,
            2,
            2,
            tile(3093, 3490),
            state.clone(),
            false,
            None,
        );
        append_case(
            &mut output,
            &service,
            scope,
            "compat-walk-near",
            &world,
            3,
            3,
            tile(3093, 3490),
            state.clone(),
            true,
            None,
        );
        append_case(
            &mut output,
            &service,
            scope,
            "compat-walk-lumbridge",
            &world,
            4,
            4,
            tile(3222, 3218),
            state.clone(),
            true,
            None,
        );

        let banks = world.named_bank_facts().unwrap().banks().to_vec();
        let (slot, action) =
            native_pick_action(banks.clone(), tile(3080, 3472), Some("Edgeville"), false);
        let authority = action.authority();
        let script::native::HostEffect::BankPick(native_request) = action.effect else {
            panic!("native capture must use the real BankPick action");
        };
        let evidence = EvidenceStamp {
            run: authority.run(),
            tick: 1,
            sequence: 1,
        };
        append_case(
            &mut output,
            &service,
            scope,
            "native-explicit-edgeville",
            &world,
            authority.request_id().get(),
            5,
            native_request.from,
            state.clone(),
            false,
            Some(NativeBankPickInput {
                request: native_request,
                authority,
                evidence,
            }),
        );
        drop(slot);

        let (slot, action) = native_pick_action(banks, tile(3080, 3472), None, false);
        let authority = action.authority();
        let script::native::HostEffect::BankPick(native_request) = action.effect else {
            panic!("native auto capture must use the real BankPick action");
        };
        let evidence = EvidenceStamp {
            run: authority.run(),
            tick: 2,
            sequence: 1,
        };
        append_case(
            &mut output,
            &service,
            scope,
            "native-auto",
            &world,
            authority.request_id().get(),
            6,
            native_request.from,
            state,
            false,
            Some(NativeBankPickInput {
                request: native_request,
                authority,
                evidence,
            }),
        );
        drop(slot);

        let path = std::env::var_os("BOT_BANK_PICK_CAPTURE_OUT")
            .expect("set BOT_BANK_PICK_CAPTURE_OUT to the capture destination");
        std::fs::write(path, output).unwrap();
    }

    #[test]
    #[ignore = "requires NAV_PACK for the immutable real 289 navpack"]
    fn qualifies_real_content_bank_pick_whole_job_memory() {
        use api::identity::Deadline;
        use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};
        use std::num::NonZeroU32;

        let pack = std::env::var_os("NAV_PACK")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
            .expect("set NAV_PACK to the reviewed real 289 navpack");
        let loaded = NavWorld::load_pack(&pack).unwrap();
        loaded
            .bind_named_bank_facts(
                &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
            )
            .unwrap();
        let world = Arc::new(loaded);
        let mut charge_bytes = 0;
        let mut result_bytes = 0;
        let mut slices = 0;
        let mut largest_slice = Duration::ZERO;
        let mut completed = false;
        let allocations = allocation_counter::measure(|| {
            let request = request(
                &world,
                1,
                1,
                tile(3080, 3472),
                WorldState::default().with_map_members(true),
                true,
                None,
            );
            let (input, preflights) = request.work_input(1, false, "test").unwrap();
            charge_bytes = input.charge.total_bytes().unwrap();
            let mut worker = BankPickWorkerJob::for_job(
                "test",
                BankPickJob {
                    generation: 1,
                    request,
                    deadline: None,
                },
                input,
                preflights,
            );
            let clock = SystemClock;
            let cancellation = SharedCancellation::new();
            for _ in 0..100_000 {
                let budget = SliceBudget::new(
                    NonZeroU32::new(64).unwrap(),
                    Deadline::after(clock.now(), Duration::from_millis(10)),
                );
                let mut context = StepContext::new(&clock, budget, &cancellation);
                let started = Instant::now();
                let outcome = worker.step(&mut context);
                largest_slice = largest_slice.max(started.elapsed());
                slices += 1;
                match outcome {
                    StepOutcome::Yielded => {}
                    StepOutcome::Completed(result) => {
                        assert!(result.route.is_some(), "real bank route must complete");
                        result_bytes = result.owner_result_bytes().unwrap();
                        completed = true;
                        break;
                    }
                    StepOutcome::Failed(never) => match never {},
                }
            }
            assert!(completed, "bank search exceeded the bounded slice count");
        });
        eprintln!(
            "bank_pick_whole_job peak_bytes={} charge_bytes={charge_bytes} \
             result_bytes={result_bytes} slices={slices} largest_slice_ns={}",
            allocations.bytes_max,
            largest_slice.as_nanos(),
        );
        assert!(charge_bytes <= BANK_PICK_JOB_BYTES);
        assert!(
            allocations.bytes_max <= charge_bytes as u64,
            "whole-job allocations exceed the admitted charge: {allocations:?}"
        );
        assert!(result_bytes <= 64 * 1024 * 1024);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a captured live Select input, its real navpack, and an output path"]
    fn measures_captured_native_bank_pick_cpu() {
        super::super::harness::measures_captured_native_bank_pick_cpu();
    }

    fn stage_pick(
        navs: &Arc<Mutex<HashMap<String, crate::NavBot>>>,
        world: Arc<NavWorld>,
        request_id: u64,
        generation: u64,
        scope: Scope,
        gate: Option<BankPickStepGate>,
    ) {
        let facts = navs
            .lock()
            .unwrap()
            .get("test")
            .and_then(|bot| bot.bank_pick.facts.clone())
            .unwrap();
        let state = WorldState::default();
        let from = tile(0, 0);
        let BankCandidates {
            order,
            target_count,
        } = super::super::bank_candidates(
            &world,
            &facts,
            from,
            &state,
            BankPreferences::default(),
            None,
            None,
        );
        let request = super::super::make_bank_pick_request(
            request_id,
            nav_planner::producer::lease(&world).unwrap(),
            facts,
            from,
            FindOptions::default(),
            state,
            order,
            target_count,
            None,
            None,
        );
        let deadline = Some(Instant::now() + BANK_SELECTION_WINDOW);
        let job = BankPickJob {
            generation,
            request: Arc::clone(&request),
            deadline,
        };
        let (input, preflights) = request.work_input(generation, false, "test").unwrap();
        let worker = match gate {
            Some(gate) => BankPickWorkerJob::gated(
                "test",
                super::super::CooperativeBankPickJob::new(job, input, preflights),
                gate,
            ),
            None => BankPickWorkerJob::for_job("test", job, input, preflights),
        };
        let mut all = navs.lock().unwrap();
        let pick = &mut all.get_mut("test").unwrap().bank_pick;
        pick.generation = generation;
        pick.last_id = request_id;
        pick.current = Some(super::super::ActivePick {
            job: BankPickJob {
                generation,
                request,
                deadline,
            },
            scope,
            deadline,
            ticket: None,
            cancellation: None,
            pending_job: Some(Box::new(worker)),
            submitting: false,
            capacity_sequence: None,
            owner_result_full: false,
            last_submission_refusal: None,
        });
    }

    #[test]
    fn bank_preflight_charges_shape_future_and_materializes_targets_in_worker() {
        let world = world(false);
        let request = request_for_banks(
            &world,
            vec![bank("first", 8, 8), bank("second", 12, 12)],
            1,
            1,
            false,
        );
        let expected = request
            .order
            .iter()
            .filter_map(|candidate| {
                let bank = &request.facts.banks()[candidate.index];
                bank.routable.then_some(bank.tile)
            })
            .collect::<Vec<_>>();
        let target_count = request.target_count;
        let mut preflight_result = None;
        let allocations = allocation_counter::measure(|| {
            preflight_result = request.work_input(1, false, "test");
        });
        assert_eq!(
            allocations.count_total, 0,
            "shape-only preflight allocates nothing"
        );
        let (input, preflights) =
            preflight_result.expect("bounded bank request preflights before admission");
        let primary = preflights
            .primary
            .as_ref()
            .expect("primary preflight exists");
        assert_eq!(primary.kind, super::super::BankPickSearchKind::Many);
        assert_eq!(
            primary.token.input_bytes(),
            target_count * std::mem::size_of::<WorldTile>()
        );
        let reservation = primary.token.reservation();
        assert_eq!(reservation.zone_filter_bytes, 0);
        assert_eq!(
            input.charge.input_bytes(),
            input.base_input_bytes + primary.token.input_bytes() + reservation.future_layout_bytes
        );
        assert!(reservation.peak_bytes >= reservation.result_bytes);
        assert_eq!(input.charge.scratch_bytes(), reservation.peak_bytes);
        assert_eq!(input.charge.result_bytes(), 0);
        assert_eq!(
            input.charge.total_bytes(),
            Some(
                input.base_input_bytes
                    + primary.token.input_bytes()
                    + reservation.future_layout_bytes
                    + reservation.peak_bytes
            )
        );
        assert!(input.charge.total_bytes().unwrap() <= BANK_PICK_JOB_BYTES);

        let job = BankPickJob {
            generation: 1,
            request: Arc::clone(&request),
            deadline: None,
        };
        let mut worker = super::super::CooperativeBankPickJob::new(job, input, preflights);
        assert!(worker.search_request.is_none());
        assert!(worker.target_buffer.is_none());

        worker
            .begin_search_materialization(super::super::BankPickSearchKind::Many)
            .unwrap();
        assert_eq!(
            worker.target_buffer.as_ref().unwrap().capacity(),
            target_count
        );
        assert!(worker.search_request.is_none());
        assert!(
            worker.retained().bytes() <= worker.input.charge.total_bytes().unwrap(),
            "the allocated target capacity remains inside its preflight charge"
        );
        while worker.build_order < worker.job.request.order.len() {
            worker
                .materialize_target_or_row(super::super::BankPickSearchKind::Many)
                .unwrap();
        }
        worker
            .finish_target_materialization(super::super::BankPickSearchKind::Many)
            .unwrap();
        match worker.search_request.take().unwrap() {
            SearchRequest::Many { targets, .. } => {
                assert_eq!(targets.as_ref(), expected.as_slice())
            }
            _ => panic!("bank candidate targets retain their ordered Many request"),
        }
    }

    #[test]
    fn bank_pick_result_limit_rejects_oversize_without_clipping() {
        let maximum = nav_planner::producer::OWNER_RESULT_MAX_BYTES;
        assert!(super::super::bank_pick_result_within_bound(maximum));
        assert!(!super::super::bank_pick_result_within_bound(maximum + 1));
    }

    #[test]
    fn oversized_owner_result_reservation_releases_completed_terminal_charge() {
        let service = service();
        let scope = scope();
        let navs = navs(vec![bank("bank", 8, 8)]);
        let world = world(false);
        stage_pick(&navs, Arc::clone(&world), 1, 1, scope, None);
        assert!(super::super::submit_pending_bank_pick_with_service(
            &navs, "test", scope, true, &service,
        ));
        let (ticket, job) = {
            let all = navs.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            (active.ticket.unwrap().ticket(), active.job.clone())
        };
        WorkService::wait_terminal(&service, &ticket)
            .expect("bank-pick terminal completes before owner reservation");
        let (terminal_before, noops_before) = {
            let service = service.lock();
            let result = match service.borrow_terminal(&ticket).unwrap() {
                Some(Settlement::Completed(result)) => result,
                _ => panic!("bank-pick terminal completed before owner reservation"),
            };
            (
                (result.index, result.access_tile, result.kind),
                service.cancellation_handle_noops(),
            )
        };

        assert!(super::super::reserve_bank_pick_result(
            &navs,
            "test",
            scope,
            &job,
            ticket,
            nav_planner::producer::OWNER_RESULT_MAX_BYTES + 1,
            nav_planner::producer::OwnerResultLedger::shared(),
        )
        .is_none());
        drop(job);
        assert!(navs.lock().unwrap()["test"].bank_pick.current.is_none());
        wait_for_returned_capacity(&service, 0);
        let service = service.lock();
        assert_eq!(service.capacity(), api::work::WorkCapacity::EMPTY);
        assert_eq!(service.cancellation_handle_noops(), noops_before + 1);
        match service
            .borrow_terminal(&ticket)
            .expect("abandoning an unconsumed terminal preserves its identity")
        {
            Some(Settlement::Completed(result)) => assert_eq!(
                (result.index, result.access_tile, result.kind),
                terminal_before,
                "owner reservation refusal does not rewrite the completed terminal"
            ),
            _ => panic!("owner reservation refusal must not rewrite the completed terminal"),
        }
    }

    #[test]
    fn full_owner_ledger_keeps_the_completed_terminal_charged_for_retry() {
        let service = service();
        let scope = scope();
        let navs = navs(vec![bank("bank", 8, 8)]);
        let world = world(false);
        stage_pick(&navs, Arc::clone(&world), 1, 1, scope, None);
        assert!(super::super::submit_pending_bank_pick_with_service(
            &navs, "test", scope, true, &service,
        ));
        let (ticket, job) = {
            let all = navs.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            (active.ticket.unwrap().ticket(), active.job.clone())
        };
        WorkService::wait_terminal(&service, &ticket)
            .expect("bank-pick terminal completes before owner reservation");
        let bytes = match service.lock().borrow_terminal(&ticket).unwrap() {
            Some(Settlement::Completed(result)) => result
                .owner_result_bytes()
                .expect("completed bank pick has a bounded owner charge"),
            _ => panic!("bank-pick terminal did not complete"),
        };
        assert!(bytes > 0);

        let full_ledger = nav_planner::producer::OwnerResultLedger::with_limits(bytes - 1, bytes);
        super::super::consume_completed_bank_pick(
            &navs,
            "test",
            scope,
            job.clone(),
            ticket,
            bytes,
            &service,
            &full_ledger,
        );
        let retained_request = {
            let all = navs.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            assert_eq!(active.ticket.unwrap().ticket(), ticket);
            assert!(active.owner_result_full);
            Arc::clone(&active.job.request)
        };
        assert!(Arc::ptr_eq(&retained_request, &job.request));
        assert_eq!(full_ledger.used_bytes(), 0);
        assert_eq!(service.lock().capacity().jobs(), 1);
        assert!(matches!(
            service.lock().borrow_terminal(&ticket),
            Ok(Some(Settlement::Completed(_)))
        ));

        let ledger = nav_planner::producer::OwnerResultLedger::with_limits(bytes, bytes);
        super::super::consume_completed_bank_pick(
            &navs, "test", scope, job, ticket, bytes, &service, &ledger,
        );
        let (posted, retained) = {
            let all = navs.lock().unwrap();
            let pick = &all["test"].bank_pick;
            assert!(pick.current.is_none());
            let retained = pick
                .owner_result
                .as_ref()
                .unwrap()
                .value()
                .owner_result_bytes()
                .unwrap();
            (pick.posted, retained)
        };
        assert_eq!(posted.request_id, 1);
        assert_eq!(posted.generation, 1);
        assert_eq!(posted.bank_index, 0);
        assert_eq!(posted.kind, PickKind::Reachable as u8);
        assert_eq!(retained, bytes);
        assert_eq!(ledger.used_bytes(), bytes);
        wait_for_returned_capacity(&service, 0);
        assert_eq!(service.lock().capacity(), api::work::WorkCapacity::EMPTY);

        drop(navs);
        assert_eq!(ledger.used_bytes(), 0);
    }

    #[test]
    fn walking_pick_publishes_a_borrowed_compact_route_and_rejects_a_stale_owner() {
        for stale in [false, true] {
            let service = service();
            let scope = scope();
            let banks = vec![bank("bank", 8, 8)];
            let navs = navs(banks.clone());
            let world = world(false);
            let request = request_for_banks(&world, banks, 7, 11, true);
            let (input, preflights) = request.work_input(11, false, "test").unwrap();
            let job = BankPickJob {
                generation: 11,
                request: Arc::clone(&request),
                deadline: None,
            };
            let worker = BankPickWorkerJob::for_job("test", job.clone(), input, preflights);
            {
                let mut all = navs.lock().unwrap();
                let bot = all.get_mut("test").unwrap();
                bot.route_generation = 11;
                bot.requested_route = request.first().map(|index| request.route_key(index));
                bot.bank_pick.generation = 11;
                bot.bank_pick.last_id = 7;
                bot.bank_pick.current = Some(super::super::ActivePick {
                    job: job.clone(),
                    scope,
                    deadline: None,
                    ticket: None,
                    cancellation: None,
                    pending_job: Some(Box::new(worker)),
                    submitting: false,
                    capacity_sequence: None,
                    owner_result_full: false,
                    last_submission_refusal: None,
                });
            }
            assert!(super::super::submit_pending_bank_pick_with_service(
                &navs, "test", scope, true, &service,
            ));
            let ticket = navs.lock().unwrap()["test"]
                .bank_pick
                .current
                .as_ref()
                .unwrap()
                .ticket
                .unwrap()
                .ticket();
            WorkService::wait_terminal(&service, &ticket).unwrap();
            let bytes = match service.lock().borrow_terminal(&ticket).unwrap() {
                Some(Settlement::Completed(result)) => result.owner_result_bytes().unwrap(),
                _ => panic!("walking selection did not complete"),
            };
            if stale {
                navs.lock()
                    .unwrap()
                    .get_mut("test")
                    .unwrap()
                    .route_generation += 1;
            }
            let ledger = nav_planner::producer::OwnerResultLedger::with_limits(bytes, bytes);
            super::super::consume_completed_bank_pick(
                &navs, "test", scope, job, ticket, bytes, &service, &ledger,
            );
            {
                let all = navs.lock().unwrap();
                let bot = &all["test"];
                assert!(
                    bot.route.is_none(),
                    "compact publication must not expand a legacy route"
                );
                assert_eq!(bot.has_route(), !stale);
                assert!(bot.bank_pick.current.is_none());
                if !stale {
                    let path = bot.route_path_for_follow().unwrap();
                    let plan = path.plan_for_follow().unwrap();
                    assert_eq!(plan.identity(), ticket.identity());
                    assert_eq!(plan.revision(), ticket.revision());
                    let Some(crate::walk_map::RouteProjectionPath::Compact(view)) =
                        bot.route_projection()
                    else {
                        panic!("the map must borrow the same compact route");
                    };
                    assert_eq!(view.destination(), plan.destination());
                    assert_eq!(bot.route_destination(), Some(plan.destination()));
                    assert!(view.legs().next().is_some());
                    assert_eq!(ledger.used_bytes(), bytes);
                } else {
                    assert_eq!(ledger.used_bytes(), 0);
                    assert!(bot.bank_pick.owner_result.is_none());
                }
            }
            wait_for_returned_capacity(&service, 0);
            assert_eq!(service.lock().capacity(), api::work::WorkCapacity::EMPTY);
            drop(navs);
            assert_eq!(ledger.used_bytes(), 0);
        }
    }

    #[test]
    fn many_cursor_and_results_are_dropped_before_single_fallback() {
        let world = world(false);
        let request = request_for_banks(&world, vec![bank("bank", 8, 8)], 1, 7, true);
        let (input, preflights) = request
            .work_input(7, false, "test")
            .expect("walking request has a bounded Many and Single reservation");
        let charge = input.charge;
        let revision = input.revision;
        let job = BankPickJob {
            generation: 7,
            request,
            deadline: None,
        };
        let mut worker = super::super::CooperativeBankPickJob::new(job, input, preflights);
        worker
            .begin_search_materialization(super::super::BankPickSearchKind::Many)
            .unwrap();
        while worker.build_order < worker.job.request.order.len() {
            worker
                .materialize_target_or_row(super::super::BankPickSearchKind::Many)
                .unwrap();
        }
        worker
            .finish_target_materialization(super::super::BankPickSearchKind::Many)
            .unwrap();
        let many_request = worker.search_request.take().unwrap();
        let many_preflight = worker.current_preflight().unwrap();
        let shared_state = Arc::clone(&worker.job.request.state);
        let state_refs = Arc::strong_count(&shared_state);
        worker.cursor = Some(
            SearchCursor::new_preflighted(
                many_request,
                worker.job.request.lease.clone(),
                Arc::clone(&shared_state),
                many_preflight,
            )
            .unwrap(),
        );
        worker.stage = super::super::BankPickStage::Search;
        worker.pending_many_results =
            Some(vec![Err(nav_planner::router::TargetError::NoPath)].into_boxed_slice());
        assert_eq!(Arc::strong_count(&shared_state), state_refs + 1);
        assert!(worker.cursor.is_some());
        assert!(worker.pending_many_results.is_some());

        worker.start_fallback_search();

        assert!(worker.cursor.is_none());
        assert!(worker.target_buffer.is_none());
        assert!(worker.pending_many_results.is_none());
        assert!(worker.pending_route.is_none());
        assert!(worker.search_preflights.primary.is_none());
        assert!(worker.search_preflights.fallback.is_some());
        assert_eq!(Arc::strong_count(&shared_state), state_refs);
        assert!(matches!(
            &worker.search_request,
            Some(SearchRequest::Single { .. })
        ));
        assert_eq!(worker.input.charge, charge);
        assert_eq!(worker.input.revision, revision);
        assert!(worker.retained().bytes() <= charge.total_bytes().unwrap());
    }

    #[test]
    fn preflight_token_mismatch_fails_closed() {
        let world = world(false);
        let lease = nav_planner::producer::lease(&world).unwrap();
        let state = Arc::new(WorldState::default());
        let preflight = SearchCursor::preflight(
            SearchRequestShape::Single {
                options: FindOptions::default(),
                avoid_capacity: 0,
                max_settles: 1,
            },
            &lease,
            &state,
        )
        .expect("small single request has a representable shape");
        let request = SearchRequest::Many {
            from: tile(0, 0),
            targets: vec![tile(8, 8)].into_boxed_slice(),
            options: FindOptions::default(),
            avoid: Box::default(),
            max_settles: 1,
            reconstruct_best: false,
        };
        assert!(matches!(
            SearchCursor::new_preflighted(request, lease, state, preflight),
            Err(nav_planner::router::SearchCursorError::PreflightMismatch)
        ));
    }

    #[test]
    fn cancellation_releases_preflight_tokens_and_partial_targets() {
        let world = world(false);
        let request = request_for_banks(&world, vec![bank("bank", 8, 8)], 1, 1, true);
        let (input, preflights) = request
            .work_input(1, false, "test")
            .expect("walking request reserves its single-search fallback");
        let job = BankPickJob {
            generation: 1,
            request,
            deadline: None,
        };
        let mut worker = super::super::CooperativeBankPickJob::new(job, input, preflights);
        worker
            .begin_search_materialization(super::super::BankPickSearchKind::Many)
            .unwrap();
        worker
            .materialize_target_or_row(super::super::BankPickSearchKind::Many)
            .unwrap();
        assert!(worker.target_buffer.is_some());
        assert!(worker.search_preflights.fallback.is_some());

        let outcome = worker.checkpoint(Checkpoint::Cancel(Cancellation::ScopeInvalidated));
        assert!(matches!(outcome, Some(StepOutcome::Yielded)));
        assert!(worker.target_buffer.is_none());
        assert!(worker.search_request.is_none());
        assert!(worker.search_preflights.primary.is_none());
        assert!(worker.search_preflights.fallback.is_none());
        assert!(worker.lease.is_none());
    }

    #[test]
    fn timeout_fallback_reuses_admission_charge_and_revision() {
        let world = world(false);
        let navs = navs(vec![bank("bank", 8, 8)]);
        let facts = navs
            .lock()
            .unwrap()
            .get("test")
            .and_then(|bot| bot.bank_pick.facts.clone())
            .unwrap();
        let state = WorldState::default();
        let from = tile(0, 0);
        let BankCandidates {
            order,
            target_count,
        } = super::super::bank_candidates(
            &world,
            &facts,
            from,
            &state,
            BankPreferences::default(),
            None,
            None,
        );
        let request = super::super::make_bank_pick_request(
            1,
            nav_planner::producer::lease(&world).unwrap(),
            facts,
            from,
            FindOptions::default(),
            state,
            order,
            target_count,
            Some(7),
            None,
        );
        let (input, preflights) = request
            .work_input(7, false, "test")
            .expect("bounded bank pick has an admission charge");
        let revision = input.revision;
        let charge = input.charge;
        let phase_total = |preflight: &SearchPreflight| {
            let reservation = preflight.reservation();
            input.base_input_bytes
                + preflight.input_bytes()
                + reservation.future_layout_bytes
                + reservation.peak_bytes
        };
        let primary_total = phase_total(&preflights.primary.as_ref().unwrap().token);
        let fallback_total = phase_total(preflights.fallback.as_ref().unwrap());
        assert_eq!(charge.result_bytes(), 0);
        assert_eq!(
            charge.total_bytes(),
            Some(primary_total.max(fallback_total))
        );
        assert!(charge.total_bytes().unwrap() <= BANK_PICK_JOB_BYTES);

        let job = BankPickJob {
            generation: 7,
            request,
            deadline: Some(Instant::now() - Duration::from_millis(1)),
        };
        let mut worker = super::super::CooperativeBankPickJob::new(job, input, preflights);
        worker
            .begin_search_materialization(super::super::BankPickSearchKind::Many)
            .unwrap();
        worker
            .materialize_target_or_row(super::super::BankPickSearchKind::Many)
            .unwrap();
        assert_eq!(worker.target_buffer.as_ref().unwrap().len(), 1);

        assert!(worker.timeout_fallback_due(Instant::now()));
        worker.start_timeout_fallback();

        assert!(worker.fallback_started);
        assert!(!worker.timeout_fallback_due(Instant::now()));
        assert_eq!(worker.input.revision, revision);
        assert_eq!(worker.input.charge, charge);
        assert!(matches!(
            &worker.search_request,
            Some(nav_planner::router::SearchRequest::Single { .. })
        ));
        assert!(worker.cursor.is_none());
        assert!(worker.target_buffer.is_none());
        assert!(worker.search_preflights.primary.is_none());
        assert!(worker.search_preflights.fallback.is_some());
        assert!(worker.pending_many_results.is_none());
        let fallback_preflight = worker.current_preflight().unwrap();
        assert!(worker.current_preflight().is_none());
        let fallback_request = worker.search_request.take().unwrap();
        worker.cursor = Some(
            SearchCursor::new_preflighted(
                fallback_request,
                worker.job.request.lease.clone(),
                Arc::clone(&worker.job.request.state),
                fallback_preflight,
            )
            .unwrap(),
        );
        let outcome = worker.checkpoint(Checkpoint::Cancel(Cancellation::ScopeInvalidated));
        assert!(matches!(outcome, Some(StepOutcome::Yielded)));
        assert!(worker.cursor.is_none());
        assert!(worker.search_preflights.fallback.is_none());
    }

    #[test]
    fn full_refused_nonwalking_pick_publishes_air_fallback_at_deadline() {
        let service = service();
        let world = world(false);
        let generation = 9;
        let mut admitted = Vec::new();
        let mut refused = None;
        for request_id in 1..=65 {
            let pending = navs(vec![bank("bank", 8, 8)]);
            stage_pick(
                &pending,
                Arc::clone(&world),
                request_id,
                generation,
                scope(),
                None,
            );
            assert!(super::super::submit_pending_bank_pick_with_service(
                &pending,
                "test",
                scope(),
                true,
                &service,
            ));
            if pending.lock().unwrap()["test"]
                .bank_pick
                .current
                .as_ref()
                .unwrap()
                .ticket
                .is_none()
            {
                refused = Some(pending);
                break;
            }
            admitted.push(pending);
        }
        let pending = refused.expect("the real WorkService job limit refuses demand with Full");
        let (request_id, deadline) = {
            let all = pending.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            assert_eq!(
                active.last_submission_refusal,
                Some(SubmissionRefusal::Full)
            );
            assert!(active.pending_job.is_some());
            (active.job.request.request_id, active.deadline.unwrap())
        };
        let capacity = service.lock().publications().capacity().sequence();
        let posted =
            super::super::poll_bank_pick_with_service(&pending, "test", deadline, &service);
        let retired = pending.lock().unwrap()["test"].bank_pick.current.is_none();
        assert_eq!(
            service.lock().publications().capacity().sequence(),
            capacity
        );
        drop(admitted);
        drop(pending);
        wait_for_returned_capacity(&service, 0);
        assert_eq!(WorkService::shutdown(&service), Ok(()));
        assert_eq!(posted.request_id, request_id);
        assert_eq!(posted.generation, generation);
        assert_eq!(posted.bank_index, 0);
        assert_eq!(posted.kind, PickKind::AirFallback as u8);
        assert!(retired, "deadline discards the unadmitted retained job");
    }

    #[test]
    fn stopping_refusal_posts_no_candidate_without_retry() {
        let service = service();
        assert_eq!(WorkService::shutdown(&service), Ok(()));
        let navs = navs(vec![bank("bank", 8, 8)]);
        stage_pick(&navs, world(false), 7, 9, scope(), None);
        assert!(!super::super::submit_pending_bank_pick_with_service(
            &navs,
            "test",
            scope(),
            true,
            &service,
        ));
        {
            let all = navs.lock().unwrap();
            let pick = &all["test"].bank_pick;
            assert!(pick.current.is_none(), "permanent refusal retires demand");
            assert!(matches!(
                pick.pending_inline
                    .as_ref()
                    .map(|pending| pending.result.kind),
                Some(PickKind::NoCandidate)
            ));
        }
        let capacity = service.lock().publications().capacity().sequence();
        let posted =
            super::super::poll_bank_pick_with_service(&navs, "test", Instant::now(), &service);
        assert_eq!(posted.request_id, 7);
        assert_eq!(posted.generation, 9);
        assert_eq!(posted.bank_index, -1);
        assert_eq!(posted.kind, PickKind::NoCandidate as u8);
        assert!(!super::super::submit_pending_bank_pick_with_service(
            &navs,
            "test",
            scope(),
            true,
            &service,
        ));
        assert_eq!(
            service.lock().publications().capacity().sequence(),
            capacity
        );
        assert_eq!(service.lock().capacity(), api::work::WorkCapacity::EMPTY);
    }

    #[test]
    fn overcharge_publishes_a_failure_for_selection_and_walking() {
        for walking in [false, true] {
            let navs = navs(vec![bank("bank", 8, 8)]);
            let mut state = WorldState::default();
            // Reserved captured storage counts even when its map is empty.
            state
                .varps
                .reserve(BANK_PICK_JOB_BYTES / std::mem::size_of::<(i32, i32)>());
            assert!(!super::super::queue_bank(
                &navs,
                "test",
                scope(),
                &Some(world(false)),
                Some(&state),
                tile(0, 0),
                FindOptions::default(),
                7,
                BankPreferences::default(),
                None,
                walking,
                None,
            ));
            let all = navs.lock().unwrap();
            let bot = &all["test"];
            assert!(bot.bank_pick.current.is_none());
            assert!(bot.bank_pick.pending_inline.is_none());
            assert_eq!(
                bot.bank_pick.owner_result.as_ref().unwrap().value().kind,
                PickKind::NoCandidate
            );
            if walking {
                assert!(bot.walk_outcome_failed);
                assert_eq!(bot.walk_outcome_generation, bot.route_generation);
                assert_eq!(bot.walk_outcome_request_id, 0);
                assert_eq!((bot.walk_outcome_x, bot.walk_outcome_z), (8, 8));
                assert!(bot.requested_route.is_none());
            } else {
                assert_eq!(bot.bank_pick.posted.request_id, 7);
                assert_eq!(bot.bank_pick.posted.kind, PickKind::NoCandidate as u8);
            }
        }
    }

    #[test]
    fn burst_input_equality_uses_captured_query_contents() {
        let world = world(false);
        let first = navs(vec![bank("bank", 8, 8)]);
        let same = navs(vec![bank("bank", 8, 8)]);
        let different = navs(vec![bank("other", 12, 12)]);
        stage_pick(&first, Arc::clone(&world), 1, 1, scope(), None);
        stage_pick(&same, Arc::clone(&world), 2, 2, scope(), None);
        stage_pick(&different, world, 3, 3, scope(), None);
        let first = first.lock().unwrap();
        assert!(first["test"]
            .bank_pick
            .query_inputs_match_for_test(&same.lock().unwrap()["test"].bank_pick,));
        assert!(!first["test"]
            .bank_pick
            .query_inputs_match_for_test(&different.lock().unwrap()["test"].bank_pick,));
    }

    #[test]
    fn full_demand_waits_for_capacity_change_then_retries_the_same_request() {
        let service = service();
        let scope = scope();
        let world = world(false);
        let first = navs(vec![bank("bank", 8, 8)]);
        let second = navs(vec![bank("bank", 8, 8)]);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let mut release_gate = BankPickGateRelease(Some(release_tx));
        stage_pick(
            &first,
            Arc::clone(&world),
            1,
            1,
            scope,
            Some(BankPickStepGate {
                entered: entered_tx,
                release: release_rx,
            }),
        );
        stage_pick(&second, Arc::clone(&world), 1, 1, scope, None);
        let submit = |navs: &_, force| {
            super::super::submit_pending_bank_pick_with_service(
                navs, "test", scope, force, &service,
            )
        };
        assert!(submit(&first, true));
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the real first bank-pick cursor reaches its bounded gate");
        assert!(submit(&second, true));
        assert_eq!(service.lock().capacity().jobs(), 2);

        let mut accepted = vec![Arc::clone(&second)];
        let mut full_demand = None;
        // The WorkService admits at most 64 retained jobs; these bounded
        // actual requests reach Full earlier if their preflight charges fill
        // the byte ledger first.
        for request_id in 3..=65 {
            let pending = navs(vec![bank("bank", 8, 8)]);
            stage_pick(&pending, Arc::clone(&world), request_id, 1, scope, None);
            let charge = {
                let all = pending.lock().unwrap();
                all["test"]
                    .bank_pick
                    .current
                    .as_ref()
                    .unwrap()
                    .pending_job
                    .as_ref()
                    .unwrap()
                    .inner
                    .input
                    .charge
                    .total_bytes()
                    .unwrap()
            };
            assert!(charge > 0, "every bank request has a real preflight charge");
            assert!(charge <= super::super::BANK_PICK_JOB_BYTES);
            let before = service.lock().capacity();
            let submission_ok = submit(&pending, true);
            let after = service.lock().capacity();
            let (ticket_present, pending_job_present, retained_request) = {
                let all = pending.lock().unwrap();
                let active = all["test"].bank_pick.current.as_ref().unwrap();
                (
                    active.ticket.is_some(),
                    active.pending_job.is_some(),
                    active
                        .ticket
                        .is_none()
                        .then(|| Arc::clone(&active.job.request)),
                )
            };

            if ticket_present {
                assert!(submission_ok, "a valid bank request was accepted");
                assert!(!pending_job_present);
                assert_eq!(after.jobs(), before.jobs() + 1);
                assert!(
                    after.bytes().saturating_sub(before.bytes()) >= charge,
                    "the accepted capacity increase includes the actual preflight charge"
                );
                accepted.push(pending);
            } else {
                assert!(
                    submission_ok,
                    "an unticketed retained request is specifically a Full refusal"
                );
                assert!(pending_job_present);
                assert_eq!(after, before, "a refused request does not change capacity");
                full_demand = Some((pending, retained_request.unwrap(), submission_ok));
                break;
            }
        }
        let (third, retained_request, full_retained) =
            full_demand.expect("bounded real bank requests must reach the WorkService Full limit");
        let observed = service.lock().publications().capacity().sequence();
        {
            let all = third.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            assert!(active.ticket.is_none());
            assert!(active.pending_job.is_some());
            assert_eq!(active.capacity_sequence, Some(observed));
            assert!(Arc::ptr_eq(&active.job.request, &retained_request));
            assert!(matches!(
                all["test"].bank_pick.work_observation_for_test(),
                Some(super::super::BankPickWorkObservation::Full {
                    capacity_sequence,
                    ..
                }) if capacity_sequence == observed
            ));
        }
        let retried_without_change = submit(&third, false);
        assert_eq!(
            service.lock().publications().capacity().sequence(),
            observed
        );
        release_gate.release();

        let first_ticket = first.lock().unwrap()["test"]
            .bank_pick
            .current
            .as_ref()
            .unwrap()
            .ticket
            .unwrap()
            .ticket();
        WorkService::wait_terminal(&service, &first_ticket).unwrap();
        let posted =
            super::super::poll_bank_pick_with_service(&first, "test", Instant::now(), &service);
        assert_eq!(posted.kind, PickKind::Reachable as u8);
        wait_for_returned_capacity(&service, accepted.len());
        assert_eq!(service.lock().capacity().jobs(), accepted.len());
        assert_ne!(
            service.lock().publications().capacity().sequence(),
            observed
        );

        let retried_after_change = submit(&third, false);
        let same_request = {
            let all = third.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            active.ticket.is_some() && Arc::ptr_eq(&active.job.request, &retained_request)
        };
        for pending in accepted.iter().chain(std::iter::once(&third)) {
            let ticket = pending.lock().unwrap()["test"]
                .bank_pick
                .current
                .as_ref()
                .unwrap()
                .ticket
                .unwrap()
                .ticket();
            WorkService::wait_terminal(&service, &ticket).unwrap();
            let posted = super::super::poll_bank_pick_with_service(
                pending,
                "test",
                Instant::now(),
                &service,
            );
            assert_eq!(posted.kind, PickKind::Reachable as u8);
        }
        wait_for_returned_capacity(&service, 0);
        assert_eq!(service.lock().capacity(), api::work::WorkCapacity::EMPTY);
        assert!(full_retained, "Full retains the actual third owner demand");
        assert!(!retried_without_change, "unchanged capacity must not retry");
        assert!(retried_after_change, "released capacity must retry");
        assert!(same_request, "a retry keeps the original captured request");
    }

    #[derive(Clone, Copy)]
    enum PickInvalidation {
        Reset,
        RouteReplacement,
        OwnerDrop,
    }

    fn invalidation_does_not_wait_for_work_service(invalidation: PickInvalidation) {
        let service = service();
        let scope = scope();
        let navs = navs(vec![bank("bank", 8, 8)]);
        let world = world(false);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let mut release_gate = BankPickGateRelease(Some(release_tx));
        stage_pick(
            &navs,
            Arc::clone(&world),
            1,
            1,
            scope,
            Some(BankPickStepGate {
                entered: entered_tx,
                release: release_rx,
            }),
        );
        assert!(super::super::submit_pending_bank_pick_with_service(
            &navs, "test", scope, true, &service,
        ));
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("real cooperative bank-pick job reaches the bounded gate");
        let one_job = {
            let all = navs.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            assert!(active.ticket.is_some());
            service.lock().capacity()
        };
        assert_eq!(one_job.jobs(), 1);

        match invalidation {
            PickInvalidation::Reset => {
                super::super::super::reset_script_nav(&navs, "test", None);
                super::super::drain_bank_pick_cancellation(&navs, "test");
                super::super::drain_bank_pick_retired_owner(&navs, "test");
            }
            PickInvalidation::RouteReplacement => {
                let (replaced_tx, replaced_rx) = mpsc::sync_channel(1);
                let navs_for_replace = Arc::clone(&navs);
                let service_guard = service.lock();
                let replacer = std::thread::spawn(move || {
                    {
                        let mut all = navs_for_replace.lock().unwrap();
                        let bot = all.get_mut("test").unwrap();
                        bot.bank_pick.reset_for_route_replacement();
                        bot.route_generation = bot.route_generation.checked_add(1).unwrap();
                    }
                    super::super::drain_bank_pick_cancellation(&navs_for_replace, "test");
                    super::super::drain_bank_pick_retired_owner(&navs_for_replace, "test");
                    replaced_tx.send(()).unwrap();
                });
                let replaced = replaced_rx.recv_timeout(Duration::from_secs(2));
                drop(service_guard);
                replacer.join().unwrap();
                replaced.expect("route replacement invalidates without WorkService");
            }
            PickInvalidation::OwnerDrop => {
                let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
                let navs_for_drop = Arc::clone(&navs);
                let service_guard = service.lock();
                let dropper = std::thread::spawn(move || {
                    let removed = navs_for_drop.lock().unwrap().remove("test");
                    drop(removed);
                    dropped_tx.send(()).unwrap();
                });
                dropped_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("dropping NavBot never waits on the WorkService mutex");
                drop(service_guard);
                dropper.join().unwrap();
                let fresh = super::navs(vec![bank("bank", 8, 8)]);
                let bot = fresh.lock().unwrap().remove("test").unwrap();
                navs.lock().unwrap().insert("test".to_owned(), bot);
            }
        }

        let still_charged = service.lock().capacity();
        assert_eq!(
            still_charged.jobs(),
            1,
            "old charge survives owner invalidation"
        );
        assert_eq!(still_charged.bytes(), one_job.bytes());

        let next_generation = match invalidation {
            PickInvalidation::OwnerDrop => 1,
            PickInvalidation::Reset | PickInvalidation::RouteReplacement => 3,
        };
        stage_pick(&navs, Arc::clone(&world), 2, next_generation, scope, None);
        assert!(super::super::submit_pending_bank_pick_with_service(
            &navs, "test", scope, true, &service,
        ));
        let (new_ticket, two_jobs) = {
            let all = navs.lock().unwrap();
            let active = all["test"].bank_pick.current.as_ref().unwrap();
            (active.ticket.unwrap().ticket(), service.lock().capacity())
        };
        assert_eq!(
            two_jobs.jobs(),
            2,
            "replacement fits beside the old live job"
        );
        assert_eq!(two_jobs.bytes(), one_job.bytes() * 2);
        let observed = service.lock().publications().capacity().sequence();
        release_gate.release();
        WorkService::wait_for_capacity_change(&service, observed);
        let after_old_return = service.lock().capacity();
        assert_eq!(after_old_return.jobs(), 1);
        assert_eq!(after_old_return.bytes(), one_job.bytes());

        WorkService::wait_terminal(&service, &new_ticket).expect("replacement bank-pick completes");
        let posted =
            super::super::poll_bank_pick_with_service(&navs, "test", Instant::now(), &service);
        assert_eq!(posted.request_id, 2);
        assert_eq!(posted.kind, PickKind::Reachable as u8);
        wait_for_returned_capacity(&service, 0);
        assert_eq!(service.lock().capacity().jobs(), 0);
        let current = navs.lock().unwrap();
        assert!(current["test"].bank_pick.current.is_none());
    }

    #[test]
    fn route_replacement_releases_completed_terminal_charge_before_generation_change() {
        let service = service();
        let scope = scope();
        let navs = navs(vec![bank("bank", 8, 8)]);
        let world = world(false);
        stage_pick(&navs, Arc::clone(&world), 1, 1, scope, None);
        assert!(super::super::submit_pending_bank_pick_with_service(
            &navs, "test", scope, true, &service,
        ));
        let old_ticket = navs.lock().unwrap()["test"]
            .bank_pick
            .current
            .as_ref()
            .unwrap()
            .ticket
            .unwrap()
            .ticket();
        WorkService::wait_terminal(&service, &old_ticket)
            .expect("old cooperative bank-pick job completes");
        assert_eq!(service.lock().capacity().jobs(), 1);
        let (old_result_before, noops_before) = {
            let service = service.lock();
            let result = match service.borrow_terminal(&old_ticket).unwrap() {
                Some(Settlement::Completed(result)) => result,
                _ => panic!("old cooperative bank-pick job completed"),
            };
            (
                (result.index, result.access_tile, result.kind),
                service.cancellation_handle_noops(),
            )
        };

        let (replaced_tx, replaced_rx) = mpsc::sync_channel(1);
        let navs_for_replace = Arc::clone(&navs);
        let service_guard = service.lock();
        let replacer = std::thread::spawn(move || {
            {
                let mut all = navs_for_replace.lock().unwrap();
                let bot = all.get_mut("test").unwrap();
                bot.bank_pick.reset_for_route_replacement();
                bot.route_generation = bot.route_generation.checked_add(1).unwrap();
            }
            super::super::drain_bank_pick_cancellation(&navs_for_replace, "test");
            super::super::drain_bank_pick_retired_owner(&navs_for_replace, "test");
            replaced_tx.send(()).unwrap();
        });
        let replaced = replaced_rx.recv_timeout(Duration::from_secs(2));
        drop(service_guard);
        replacer.join().unwrap();
        replaced.expect("route replacement abandons without WorkService");
        wait_for_returned_capacity(&service, 0);
        {
            let service = service.lock();
            assert_eq!(service.capacity(), api::work::WorkCapacity::EMPTY);
            assert_eq!(service.cancellation_handle_noops(), noops_before + 1);
            match service
                .borrow_terminal(&old_ticket)
                .expect("abandoning an unconsumed terminal preserves its identity")
            {
                Some(Settlement::Completed(result)) => assert_eq!(
                    (result.index, result.access_tile, result.kind),
                    old_result_before,
                    "route replacement does not rewrite the completed terminal"
                ),
                _ => panic!("route replacement must not rewrite the completed terminal"),
            }
        }

        stage_pick(&navs, Arc::clone(&world), 2, 3, scope, None);
        assert!(super::super::submit_pending_bank_pick_with_service(
            &navs, "test", scope, true, &service,
        ));
        let new_ticket = navs.lock().unwrap()["test"]
            .bank_pick
            .current
            .as_ref()
            .unwrap()
            .ticket
            .unwrap()
            .ticket();
        assert!(matches!(
            service.lock().borrow_terminal(&old_ticket),
            Err(api::work::LedgerError::StaleIdentity)
        ));
        WorkService::wait_terminal(&service, &new_ticket)
            .expect("replacement bank-pick job completes");
        let posted =
            super::super::poll_bank_pick_with_service(&navs, "test", Instant::now(), &service);
        assert_eq!(posted.request_id, 2);
        assert_eq!(posted.kind, PickKind::Reachable as u8);
        wait_for_returned_capacity(&service, 0);
        assert_eq!(service.lock().capacity().jobs(), 0);
    }

    #[test]
    fn reset_route_replacement_and_owner_drop_keep_old_charge_until_worker_return() {
        invalidation_does_not_wait_for_work_service(PickInvalidation::Reset);
        invalidation_does_not_wait_for_work_service(PickInvalidation::RouteReplacement);
        invalidation_does_not_wait_for_work_service(PickInvalidation::OwnerDrop);
    }
}

#[cfg(feature = "js")]
#[test]
#[ignore = "event-driven runtime endpoint (0.2.6): native answers must wake only their consumer and publish its next prompt without another game tick"]
fn bank_callback_answers_progress_on_unchanged_host_frames_without_game_ticks() {
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};

    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        globalThis.__matches = [];
        await Bank.depositAllMatching((name) => {
            globalThis.__matches.push(name);
            return false;
        });
        globalThis.__done = true;
    }
}
"#;
    let scripts: crate::ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let slot = crate::script_slot_or_insert(&scripts, "test");
    {
        let mut slot = slot.lock().unwrap();
        let start = crate::script_runtime::js_surface_start(
            crate::script_runtime::js_start_card(
                source.into(),
                script_js::LoadShape::CompatClass,
                vec![],
            ),
            None,
            Some(api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()),
            Arc::new(NamedBankFacts::from_banks(vec![])),
            Arc::from(Vec::<api::loadout::Loadout>::new()),
        );
        slot.start_surface_with_settings(start).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match slot.poll_start() {
                script::slot::StartPoll::Settled(script::slot::StartOutcome::Ready) => break,
                script::slot::StartPoll::Pending => {
                    assert!(Instant::now() < deadline, "Load start stalled");
                    std::thread::yield_now();
                }
                other => panic!("Load start failed: {other:?}"),
            }
        }
    }
    let side: Vec<_> = (0..5)
        .map(|index| ItemView {
            def: api::ItemDefView {
                id: 100 + index,
                name: Some(format!("Kept item {index}")),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::BankSide,
            action_family: ItemActionFamily::Component,
            slot: index,
            count: 1,
            actions: vec![Some("Deposit-1".into()), Some("Deposit-All".into())],
            component_id: 701,
        })
        .collect();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tick(1);
    snapshot.seed_tile(tile(3091, 3958));
    snapshot.seed_inventory(
        side.iter()
            .cloned()
            .map(|mut item| {
                item.container = ItemContainer::Inventory;
                item.action_family = ItemActionFamily::Held;
                item.actions.clear();
                item.component_id = -1;
                item
            })
            .collect(),
        28,
    );
    snapshot.seed_bank_observation(600, 1, Some(vec![]), side);
    snapshot.seed_side_modal(700);
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    let navs = navs(vec![]);
    let observe = |tick, tick_edge| {
        crate::script_runtime::script_observe::script_observe(
            &mut HeldDriver,
            "test",
            true,
            tick_edge,
            tick,
            Some((3091, 3958, 0)),
            None,
            None,
            Some(&snapshot),
            None,
            &scripts,
            &cheats,
            &navs,
            &None,
            false,
            false,
        )
    };
    let probe = |expression| slot.lock().unwrap().probe(expression).unwrap();
    for _ in 0..4 {
        assert!(
            !observe(0, false),
            "no callback answer can wake before a tick"
        );
    }
    let tick = (1..=4)
        .find(|tick| {
            observe(*tick, true);
            for _ in 0..8 {
                observe(*tick, false);
                if probe("globalThis.__matches?.length ?? 0") == serde_json::json!(1) {
                    return true;
                }
            }
            false
        })
        .expect("the real host starts the first bank predicate");

    for _ in 0..32 {
        observe(tick, false);
        if probe("globalThis.__done === true") == serde_json::json!(true) {
            break;
        }
    }
    assert_eq!(
        probe("globalThis.__matches"),
        serde_json::json!([
            "Kept item 0",
            "Kept item 1",
            "Kept item 2",
            "Kept item 3",
            "Kept item 4"
        ]),
        "ready bank answers must progress without a new snapshot or game tick"
    );
    assert_eq!(probe("globalThis.__done === true"), serde_json::json!(true));
    assert!(
        !observe(tick, false),
        "a completed bank has no callback wake"
    );
    slot.lock().unwrap().stop();
}
