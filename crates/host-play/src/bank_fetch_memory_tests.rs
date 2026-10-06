//! design-bank-snapshot §7 S5: the host BankBudget planner reads the
//! account's bank memory ([`nav::bank_fetch::BankRows`]), not the open
//! bank. A closed bank with a `Session` memory still plans; a `Session` or
//! `Unknown` shortage is `NoPath` in place; a `Hint` shortage costs exactly
//! one verifying trip that the open bank settles.
use super::*;
use api::bank_memory::{BankMemory, Origin};
use nav::bank_fetch::{nearest_bank_access, BankStep};
use parking_lot::RwLock;

const COINS: i32 = 995;
const KNIFE: i32 = 2;

fn origin_tile() -> Tile {
    Tile {
        x: 0,
        z: 0,
        level: 0,
    }
}

fn far_side() -> Tile {
    Tile {
        x: 4,
        z: 4,
        level: 0,
    }
}

fn fetch_on() -> FindOptions {
    FindOptions {
        allow_bank_fetch: true,
        ..FindOptions::default()
    }
}

/// The knife world's door gated on a `consumed_req` of `count` × `id`
/// instead of the worn knife (the Al Kharid toll's shape).
fn toll_world(id: i32, count: i32) -> NavWorld {
    let mut world = knife_nav_world(KNIFE);
    world.graph.edges[0].worn_req.clear();
    world.graph.edges[0].consumed_req = vec![(id, count)];
    world
}

/// The bank fixture placed in the 5×5 test world at scene `(x, z)`, with
/// its bank (the knife × 20) open and loaded, or closed.
fn fixture(x: i32, z: i32, open: bool) -> (Client, GameSnapshot) {
    let mut client = bank_fetch_client();
    client.map_build_base_x = 0;
    client.map_build_base_z = 0;
    client.local_player = Some(client::dash3d::ClientPlayer::at(x, z));
    if !open {
        client.main_modal_id = -1;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    assert_eq!(snapshot.bank_loaded(), open, "fixture bank open={open}");
    if !open {
        assert!(snapshot.bank().is_empty(), "a closed bank posts no rows");
    }
    (client, snapshot)
}

/// What the slot's observer leaves after the fixture bank was open once:
/// a `Session` memory holding the knife × 20.
fn session_memory() -> BankMemory {
    let (_, open) = fixture(0, 0, true);
    let mut memory = BankMemory::default();
    memory.track(&open, 1);
    assert_eq!(memory.origin(), Origin::Session);
    assert_eq!(memory.rows(), &[(KNIFE, 20)]);
    memory
}

/// The same memory after a relog: `Hint`, rows kept (§1.4).
fn hint_memory() -> BankMemory {
    let mut memory = session_memory();
    memory.relogged();
    assert_eq!(memory.origin(), Origin::Hint);
    memory
}

fn session_steps(arms: &WalkArms, slot: &str) -> Option<Vec<BankStep>> {
    arms.lock().unwrap().get(slot).and_then(|arm| {
        arm.lock()
            .unwrap()
            .bank_fetch
            .as_ref()
            .map(|pending| pending.steps.iter().cloned().collect())
    })
}

fn access_walk(world: &NavWorld) -> BankStep {
    let access = nearest_bank_access(
        &world.collision,
        world.banks(),
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    )
    .expect("the knife world has an access tile");
    BankStep::Walk {
        x: access.x,
        z: access.z,
        level: access.level,
    }
}

/// The `bank_session_and_direct_route_both_emit_replaced_once` shape with
/// the rows from a seeded `Session` memory and the fixture bank **closed**:
/// the manual WalkTo arms the knife trip without the bank being open.
#[test]
fn a_closed_bank_with_a_session_memory_plans_the_walk_to_session() {
    let world = knife_nav_world(KNIFE);
    let (_, closed) = fixture(0, 0, false);
    let state = WorldState::from_snapshot(&closed);
    let bank = BankRows::of(&session_memory());
    let arms: WalkArms = Arc::new(Mutex::new(HashMap::new()));
    let route = arm_walk_on(
        &world,
        origin_tile(),
        far_side(),
        fetch_on(),
        &state,
        &bank,
        &arms,
        Some("alice"),
    )
    .expect("a Session memory holding the knife plans the trip with the bank shut");
    assert_eq!(
        route.dest,
        WorldTile {
            x: 4,
            z: 4,
            level: 0
        }
    );
    assert_eq!(
        session_steps(&arms, "alice"),
        Some(vec![
            access_walk(&world),
            BankStep::Open,
            BankStep::Withdraw {
                id: KNIFE,
                count: 1
            },
            BankStep::Close,
            BankStep::Wear { id: KNIFE },
        ])
    );

    // The same closed bank with no memory is today's verdict.
    let arms: WalkArms = Arc::new(Mutex::new(HashMap::new()));
    assert!(arm_walk_on(
        &world,
        origin_tile(),
        far_side(),
        fetch_on(),
        &state,
        &BankRows::default(),
        &arms,
        Some("alice"),
    )
    .is_err());
    assert_eq!(session_steps(&arms, "alice"), None);
}

/// Manual WalkTo with fetch on over a 10-coin toll the memory lacks:
/// `Unknown` and `Session` are `NoPath` in place with no session armed; a
/// `Hint` arms Walk/Open/Withdraw/Close, the open bank (which holds no
/// coins) refuses the Withdraw and the session aborts, and the next arm
/// on the now-`Session` rows is `NoPath` — one trip (H1).
#[test]
fn a_toll_the_memory_lacks_is_no_path_unless_the_memory_is_a_hint() {
    let world = toll_world(COINS, 10);
    let (_, closed) = fixture(0, 0, false);
    let state = WorldState::from_snapshot(&closed);
    let arm = |bank: &BankRows, arms: &WalkArms| {
        arm_walk_on(
            &world,
            origin_tile(),
            far_side(),
            fetch_on(),
            &state,
            bank,
            arms,
            Some("alice"),
        )
    };

    for bank in [BankRows::default(), BankRows::of(&session_memory())] {
        let arms: WalkArms = Arc::new(Mutex::new(HashMap::new()));
        assert!(arm(&bank, &arms).is_err(), "{:?} is NoPath", bank.origin);
        assert_eq!(session_steps(&arms, "alice"), None, "no session armed");
    }

    let mut memory = hint_memory();
    let arms: WalkArms = Arc::new(Mutex::new(HashMap::new()));
    arm(&BankRows::of(&memory), &arms).expect("a Hint shortage plans the verifying trip");
    let walk = access_walk(&world);
    assert_eq!(
        session_steps(&arms, "alice"),
        Some(vec![
            walk.clone(),
            BankStep::Open,
            BankStep::Withdraw {
                id: COINS,
                count: 10
            },
            BankStep::Close,
        ])
    );

    // The trip: the player reaches the access tile and the fixture bank
    // opens without coins.
    let BankStep::Walk { x, z, level } = walk else {
        unreachable!()
    };
    let (mut client, open) = fixture(x, z, true);
    let entry = arms.lock().unwrap().get("alice").cloned().unwrap();
    let mut walk_arm = entry.lock().unwrap();
    for front in [
        Some(BankStep::Open),
        Some(BankStep::Withdraw {
            id: COINS,
            count: 10,
        }),
        None,
    ] {
        step_walk_arm_bank_fetch(
            &mut client,
            &open,
            &mut walk_arm,
            Some(&world),
            Some((x, z, level)),
            false,
        );
        assert_eq!(
            walk_arm
                .bank_fetch
                .as_ref()
                .and_then(|pending| pending.steps.front().cloned()),
            front
        );
    }
    assert!(
        walk_arm.route.is_none(),
        "the refused Withdraw ends the session and its route"
    );
    drop(walk_arm);

    // That open bank is the observation: the memory is Session now.
    memory.track(&open, 2);
    assert_eq!(memory.origin(), Origin::Session);
    let arms: WalkArms = Arc::new(Mutex::new(HashMap::new()));
    assert!(
        arm(&BankRows::of(&memory), &arms).is_err(),
        "the next arm is NoPath"
    );
    assert_eq!(session_steps(&arms, "alice"), None);
}

/// A `Hint` overlay plans a whole-stack `Withdraw` (the overlaid row is
/// exactly the shortage). The open bank holds more, so the step settles
/// through Withdraw-X with exactly the planned amount (D3).
#[test]
fn a_hint_whole_stack_withdraw_settles_through_withdraw_x() {
    let world = toll_world(KNIFE, 7);
    let (_, closed) = fixture(0, 0, false);
    let state = WorldState::from_snapshot(&closed);
    let mut hint = BankRows::of(&hint_memory());
    hint.rows.clear();
    let arms: WalkArms = Arc::new(Mutex::new(HashMap::new()));
    arm_walk_on(
        &world,
        origin_tile(),
        far_side(),
        fetch_on(),
        &state,
        &hint,
        &arms,
        Some("alice"),
    )
    .expect("the Hint overlay plans the trip");
    let walk = access_walk(&world);
    assert_eq!(
        session_steps(&arms, "alice"),
        Some(vec![
            walk.clone(),
            BankStep::Open,
            BankStep::Withdraw {
                id: KNIFE,
                count: 7
            },
            BankStep::Close,
        ])
    );
    let BankStep::Walk { x, z, level } = walk else {
        unreachable!()
    };
    let (mut client, open) = fixture(x, z, true);
    let entry = arms.lock().unwrap().get("alice").cloned().unwrap();
    let mut walk_arm = entry.lock().unwrap();
    let here = Some((x, z, level));
    // Walk lands on the access tile, Open on the loaded bank.
    for _ in 0..2 {
        step_walk_arm_bank_fetch(&mut client, &open, &mut walk_arm, Some(&world), here, false);
    }
    let out_before = client.out.pos;
    step_walk_arm_bank_fetch(&mut client, &open, &mut walk_arm, Some(&world), here, false);
    assert_eq!(client.out.pos, out_before, "the rewrite sends nothing");
    let steps: Vec<BankStep> = walk_arm
        .bank_fetch
        .as_ref()
        .map(|pending| pending.steps.iter().cloned().collect())
        .unwrap_or_default();
    assert_eq!(
        steps,
        vec![
            BankStep::WithdrawX { id: KNIFE },
            BankStep::WithdrawXAmount {
                id: KNIFE,
                count: 7
            },
            BankStep::Close,
        ],
        "20 banked knives give exactly 7 only through the amount dialog"
    );
    assert!(
        step_walk_arm_bank_fetch(&mut client, &open, &mut walk_arm, Some(&world), here, false),
        "Withdraw-X is sent on the next pump"
    );
}

/// A script walk (`Walk` interact) with fetch on reads the slot's memory:
/// a `Session` memory plans through a closed bank.
#[test]
fn a_script_walk_plans_from_the_slot_memory_with_the_bank_closed() {
    let world = Some(Arc::new(knife_nav_world(KNIFE)));
    let (mut client, closed) = fixture(0, 0, false);
    let state = WorldState::from_snapshot(&closed);
    let memory = RwLock::new(session_memory());
    let (navs, _) = empty_nav();
    let walk: script::shim::InteractReq = serde_json::from_value(serde_json::json!({
        "op": "walk",
        "x": 4,
        "z": 4,
        "level": 0,
        "allow_bank_fetch": true,
        "request_id": 41,
    }))
    .expect("walk request decodes");
    assert!(dispatch_script_interact_cached(
        &mut client,
        &closed,
        None,
        Some((0, 0, 0)),
        &navs,
        &world,
        Some(state),
        Some(&memory),
        "alice",
        [walk],
        None,
        None,
    ));
    assert!(
        wait_until(2_000, || navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.bank_fetch.is_some())),
        "the script walk latched a BankBudget session from the memory"
    );
    let steps: Vec<BankStep> = navs.lock().unwrap()["alice"]
        .bank_fetch
        .as_ref()
        .map(|pending| pending.steps.iter().cloned().collect())
        .unwrap();
    assert!(steps.contains(&BankStep::Withdraw {
        id: KNIFE,
        count: 1
    }));
}

/// `route_inspect_bank_planned` (§5): a compat inspect with the bank closed
/// and a known memory plans the bank session.
#[test]
fn a_compat_inspect_with_a_closed_bank_and_a_known_memory_is_bank_planned() {
    let world = Some(Arc::new(knife_nav_world(KNIFE)));
    let (mut client, closed) = fixture(0, 0, false);
    let state = WorldState::from_snapshot(&closed);
    let (navs, _) = empty_nav();
    let inspect = |client: &mut Client, memory: &RwLock<BankMemory>, request_id: u64| {
        let request: script::shim::InteractReq = serde_json::from_value(serde_json::json!({
            "op": "inspect-route",
            "x": 4,
            "z": 4,
            "level": 0,
            "from_x": 0,
            "from_z": 0,
            "from_level": 0,
            "allow_bank_fetch": true,
            "request_id": request_id,
        }))
        .expect("inspect request decodes");
        let reqs = script::isolate_fb::decode_interact_batch(
            &script::isolate_fb::encode_interact_batch(&[request]),
        )
        .expect("inspect wire decodes");
        dispatch_script_interact_cached(
            client,
            &closed,
            None,
            Some((0, 0, 0)),
            &navs,
            &world,
            Some(state.clone()),
            Some(memory),
            "alice",
            reqs,
            None,
            None,
        );
        let start = Instant::now();
        loop {
            let term = navs.lock().unwrap().get("alice").and_then(|bot| {
                [bot.inspect.latest.as_ref(), bot.inspect.prev.as_ref()]
                    .into_iter()
                    .flatten()
                    .find(|term| term.request_id == request_id)
                    .cloned()
            });
            if let Some(term) = term {
                return term;
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "inspect {request_id} did not publish"
            );
            std::thread::yield_now();
        }
    };
    let unknown = inspect(&mut client, &RwLock::new(BankMemory::default()), 501);
    assert!(
        !unknown.ok && !unknown.bank_planned,
        "an Unknown memory has nothing to plan from: {}",
        unknown.reason
    );
    let known = inspect(&mut client, &RwLock::new(session_memory()), 502);
    assert!(known.ok, "{}", known.reason);
    assert!(
        known.bank_planned,
        "the closed bank's memory plans the session"
    );
}

/// D6 pin: compat `Bank.*` and `Inventory.count` read the posted open bank
/// and pack only. With the fixture bank open they see the knives; with it
/// closed they answer empty/0 although the slot's memory (passed to the
/// observer) is `Session` and holds them.
#[test]
fn compat_bank_reads_stay_open_bank_only_with_a_known_memory() {
    const SRC: &str = r#"
import { Bank } from '../../api/bank/Bank.js';
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            items: Bank.items().length,
            count: Bank.count('Knife'),
            by_id: Bank.countById(2),
            held: Inventory.count('Knife'),
        };
    }
}
"#;
    let memory = RwLock::new(session_memory());
    let observe = |open: bool| {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        script_slot_or_insert(&scripts, "alice")
            .lock()
            .unwrap()
            .start_load_with_loadouts_settled(
                SRC.into(),
                script::LoadShape::CompatClass,
                vec![],
                &[],
            )
            .expect("the compat isolate starts");
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        let (navs, _) = empty_nav();
        let (mut client, snapshot) = fixture(0, 0, open);
        for tick in 1..=20 {
            script_observe_cached_with_channels(
                &mut client,
                "alice",
                true,
                true,
                false,
                tick,
                Some((0, 0, 0)),
                None,
                Some(WorldState::from_snapshot(&snapshot)),
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
            let probe = script_slot(&scripts, "alice")
                .unwrap()
                .lock()
                .unwrap()
                .probe("globalThis.__probe ?? null")
                .expect("probe");
            if !probe.is_null() {
                return probe;
            }
        }
        panic!("the compat loop never ran (open={open})");
    };
    let open = observe(true);
    assert_eq!(
        open,
        serde_json::json!({ "items": 1, "count": 20, "by_id": 20, "held": 0 }),
        "control: the open bank is read"
    );
    let closed = observe(false);
    assert_eq!(
        closed,
        serde_json::json!({ "items": 0, "count": 0, "by_id": 0, "held": 0 }),
        "a closed bank reads empty whatever the memory holds"
    );
}

/// A trip planned from the memory can leave the player at a bank away from
/// the arm-time origin: when the session clears there, the post-session
/// route is found again from where the player stands, under the live state;
/// with no route from there, the walk ends instead of aiming off-scene.
#[test]
fn the_post_session_route_resumes_from_the_bank() {
    let walk = access_walk(&knife_nav_world(KNIFE));
    let BankStep::Walk { x, z, level } = walk else {
        unreachable!()
    };
    let at_bank = WorldTile { x, z, level };
    // The fixture pack carries 3 bones (obj 1); the door takes `need`.
    for (need, routable) in [(3, true), (5, false)] {
        let world = toll_world(1, need);
        let (mut client, closed) = fixture(x, z, false);
        let origin = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let dest = WorldTile {
            x: 4,
            z: 4,
            level: 0,
        };
        let planned_state = WorldState {
            inv: HashMap::from([(1, need)]),
            ..WorldState::default()
        };
        let final_route = find_with(
            &world.collision,
            &world.graph,
            origin,
            dest,
            FindOptions::default(),
            &planned_state,
        )
        .expect("the planned post state crosses");
        let mut bot = NavBot {
            route: Some(final_route.clone()),
            bank_fetch: Some(PendingBankFetch {
                steps: std::collections::VecDeque::from([BankStep::Close]),
                dest,
                opts: FindOptions::default(),
                final_route,
                avoid: Vec::new(),
                progress: Default::default(),
            }),
            ..Default::default()
        };
        step_bank_fetch_on_bot(
            &mut client,
            &closed,
            &mut bot,
            Some(&world),
            Some((x, z, level)),
            false,
        );
        assert!(
            bot.bank_fetch.is_none(),
            "Close landed: the session cleared"
        );
        if routable {
            let route = bot.route.expect("the route resumes from the bank");
            assert_eq!(route.dest, dest);
            match route.legs.first() {
                Some(nav::router::Leg::Walk { tiles }) => {
                    assert_eq!(tiles.first(), Some(&at_bank))
                }
                other => panic!("the resumed route starts with a walk: {other:?}"),
            }
        } else {
            assert!(
                bot.route.is_none(),
                "no route from the bank under the live state: the walk ends"
            );
        }
    }
}

/// The toll world with its far corner `(4, 4)` solid: a radius walk there
/// takes the unmodeled-solid stand search (`calculate_solid`), and an Area
/// walk searches every standable tile around it at once.
fn solid_toll_world() -> NavWorld {
    let mut world = knife_nav_world_with_target(KNIFE, true);
    world.graph.edges[0].worn_req.clear();
    world.graph.edges[0].consumed_req = vec![(COINS, 10)];
    world
}

/// The closed fixture bank at the origin, with the client's own `(4, 4)`
/// solid as the world's is.
fn solid_fixture() -> (Client, GameSnapshot) {
    let (mut client, _) = fixture(0, 0, false);
    client.collision[0].flags[4][4] |= client::dash3d::CollisionFlag::SQ_BLOCKED;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    (client, snapshot)
}

/// The one verifying trip for the 10-coin toll.
fn toll_trip(world: &NavWorld) -> Vec<BankStep> {
    vec![
        access_walk(world),
        BankStep::Open,
        BankStep::Withdraw {
            id: COINS,
            count: 10,
        },
        BankStep::Close,
    ]
}

/// H1 (REVIEW-BANK-SNAPSHOT-S5): the solid-radius and Area goal searches
/// read a `Hint` as advisory, as the exact walk does. With the bank closed
/// and a hint that holds no coins or too few, every goal around the solid
/// `(4, 4)` is behind the 10-coin toll; the goal-set diagnosis is overlaid
/// on the hint, so the search reaches a goal and plans the one verifying
/// trip, and the post-session route goes to that goal. The same rows as a
/// `Session` or `Unknown` memory stay `NoPath` in place.
fn assert_one_trip_only_for_a_hint(arrival: nav::arrival::ArrivalKind) {
    let world = Arc::new(solid_toll_world());
    let (_, closed) = solid_fixture();
    let state = WorldState::from_snapshot(&closed);
    let request = |bank_origin, rows: &[(i32, i32)]| ScriptRouteRequest {
        generation: 1,
        request_id: 1,
        world: Arc::clone(&world),
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 4,
            z: 4,
            level: 0,
        },
        radius: 1,
        loc_id: None,
        arrival,
        opts: fetch_on(),
        state: Some(state.clone()),
        bank_origin,
        bank: rows.to_vec(),
        live_candidates: None,
        exclusions: None,
        completion: Default::default(),
    };
    for rows in [&[][..], &[(KNIFE, 20)][..], &[(COINS, 3)][..]] {
        let (outcome, targets) = request(Origin::Hint, rows).calculate();
        let RouteOutcome::BankSession { pending, route } = outcome else {
            panic!("{arrival:?} with Hint rows {rows:?}: no verifying trip");
        };
        assert_eq!(
            pending.steps.iter().cloned().collect::<Vec<_>>(),
            toll_trip(&world),
            "{arrival:?} with Hint rows {rows:?}"
        );
        assert!(targets.contains(&route.dest), "{:?}", route.dest);
        assert_eq!(pending.dest, route.dest);
        assert_eq!(pending.final_route.dest, route.dest);
        for bank_origin in [Origin::Session, Origin::Unknown] {
            assert!(
                matches!(
                    request(bank_origin, rows).calculate().0,
                    RouteOutcome::NoPath
                ),
                "{arrival:?} with {bank_origin:?} rows {rows:?} is NoPath in place"
            );
        }
    }
}

#[test]
fn a_solid_radius_walk_plans_the_trip_a_hint_lacks_the_toll_for() {
    assert_one_trip_only_for_a_hint(nav::arrival::ArrivalKind::Reach);
}

#[test]
fn an_area_walk_plans_the_trip_a_hint_lacks_the_toll_for() {
    assert_one_trip_only_for_a_hint(nav::arrival::ArrivalKind::Area);
}

/// The enabled compat dispatch (`walk-near`, radius 1, fetch on) to a solid
/// tile with the bank closed: a `Hint` memory without coins latches the one
/// verifying trip; a `Session` memory without coins and no memory at all
/// end the walk failed, with no session.
#[test]
fn a_compat_walk_near_a_solid_tile_plans_the_hint_trip_with_the_bank_closed() {
    let world = Some(Arc::new(solid_toll_world()));
    let (mut client, closed) = solid_fixture();
    let state = WorldState::from_snapshot(&closed);
    let mut walk = |memory: Option<&RwLock<BankMemory>>| {
        let (navs, _) = empty_nav();
        assert!(dispatch_script_interact_cached(
            &mut client,
            &closed,
            None,
            Some((0, 0, 0)),
            &navs,
            &world,
            Some(state.clone()),
            memory,
            "alice",
            [script::shim::InteractReq::WalkNear {
                x: 4,
                z: 4,
                level: 0,
                radius: 1,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: true,
                request_id: 43,
                avoid: Vec::new(),
                cross: Vec::new(),
            }],
            None,
            None,
        ));
        assert!(
            wait_until(2_000, || navs.lock().unwrap().get("alice").is_some_and(
                |bot| bot.bank_fetch.is_some() || bot.walk_outcome_failed
            )),
            "the route worker settled"
        );
        let navs = navs.lock().unwrap();
        let bot = &navs["alice"];
        (
            bot.bank_fetch
                .as_ref()
                .map(|pending| pending.steps.iter().cloned().collect::<Vec<_>>()),
            bot.walk_outcome_failed,
        )
    };
    let hint = RwLock::new(hint_memory());
    assert_eq!(
        walk(Some(&hint)),
        (Some(toll_trip(world.as_ref().unwrap())), false),
        "a Hint without coins plans the one trip"
    );
    let session = RwLock::new(session_memory());
    assert_eq!(
        walk(Some(&session)),
        (None, true),
        "Session is NoPath in place"
    );
    assert_eq!(walk(None), (None, true), "Unknown is NoPath in place");
}
