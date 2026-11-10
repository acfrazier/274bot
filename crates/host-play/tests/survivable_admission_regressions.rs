//! Regressions from the S2b review: held defaults preserve router admission and fleeing is always possible.
use api::condition::PoisonState;
use api::selected::ClientRevision;
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use host_play::admission::{self, Admission, RiskInput};
use host_play::{DangerDecision, DangerLevel, WalkGlobals, NET_AVAILABLE};
use nav_planner::collision::{pack_walk, WorldCollision};
use nav_planner::router::{find_with_avoid, FindOptions, Route};
use nav_planner::transport::TransportGraph;
use nav_planner::world::NavWorld;
use nav_planner::zones::{Zone, ZoneClass, ZoneExempt, ZoneKind, ZoneTable, NO_GROUP};
use nav_planner::WorldState;
use script::combat::risk::{RiskTables, UnknownWhy, Verdict};
use script::combat::CombatTables;
use script::native::{RiskPolicy, WalkBit, WalkOptions, WalkPermissions, WalkRefusal};

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn corridor(width: usize, spawns: &[i32]) -> NavWorld {
    let height = 3;
    let mut flags = vec![CollisionFlag::WALK_SCENERY as u32; width * height * 4];
    for level in 0..4 {
        for x in 0..width {
            flags[level * width * height + width + x] = 0;
        }
    }
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: tile(0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph::default();
    let combat =
        CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap();
    let npc = combat.selected().npc_by_config("icewarrior").unwrap();
    let kind = ZoneKind::new("ice-warrior", "Ice warrior", npc.id, 57, false, false);
    let zones: Vec<_> = spawns
        .iter()
        .map(|&x| {
            let mut zone = Zone::npc(tile(x, 1), 0, ZoneClass::Always, u16::MAX, 0);
            zone.group = NO_GROUP;
            zone
        })
        .collect();
    let kinds = if zones.is_empty() {
        Vec::new()
    } else {
        vec![kind]
    };
    graph.zones = Some(
        ZoneTable::from_parts(
            zones,
            kinds,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            collision.origin,
            width as u32,
            height as u32,
            &graph.wilderness,
        )
        .unwrap(),
    );
    NavWorld::from_parts(collision, graph, Vec::new())
}

fn input(hp: u8, poison: PoisonState) -> RiskInput {
    let mut input = RiskInput::default();
    input.pos = tile(0, 1);
    input.poison = poison;
    input.tick = 10;
    input.hp = hp;
    input.hp_max = 99;
    input.prayer = 10;
    input.prayer_base = 10;
    input.combat = Some(50);
    input.set_missing_facts(false);
    input.set_unattributed(false);
    input.set_input_locked(false);
    input.set_overflow(false);
    input
}

fn find(world: &NavWorld, from: WorldTile, to: WorldTile) -> Route {
    find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::default(),
        &[],
    )
    .expect("router route")
}

fn default_admission(input: RiskInput) -> Admission {
    Admission::manual(WalkGlobals::default().manual_decision(false), input, 1)
}

#[test]
fn probe_held_middle_matches_never_for_every_preference_source() {
    const { assert!(!NET_AVAILABLE) };
    let dir = std::env::temp_dir().join(format!("review-s2b-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Each source is one state root's shared preferences, read through the
    // process service as every admission reads them.
    let read = |name: &str, body: Option<&str>| {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        if let Some(body) = body {
            vault::write_private_file(&root.join("panel-ui.json"), body.as_bytes()).unwrap();
        }
        let service = host_play::local_state::LocalStateService::open_process(
            host_play::local_state::StateRoot::new(root),
        );
        let preferences = service.preferences();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match WalkGlobals::read(&preferences) {
                Err(error) if error.is_not_ready() => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{name}: preference hydration did not finish"
                    );
                    std::thread::yield_now();
                }
                result => break result,
            }
        }
    };
    let sources = [
        ("default", WalkGlobals::default()),
        ("absent", read("absent", None).unwrap()),
        (
            "old",
            read("old", Some(r#"{"nav":{"allow_danger_zones":false}}"#)).unwrap(),
        ),
        (
            "empty-nav",
            read("empty-nav", Some(r#"{"nav":{}}"#)).unwrap(),
        ),
        (
            "no-nav",
            read("no-nav", Some(r#"{"unrelated":1}"#)).unwrap(),
        ),
        (
            "explicit-middle",
            read(
                "explicit-middle",
                Some(r#"{"nav":{"allow_danger_zones":false,"survivable_routing":true}}"#),
            )
            .unwrap(),
        ),
        (
            "malformed",
            read("malformed", Some("{nav:")).unwrap_or_else(|_| WalkGlobals::fail_closed()),
        ),
    ];
    let never = WalkGlobals::default();
    let world = api::test_support::game_data_available().then(|| corridor(40, &[20]));
    for (name, globals) in sources {
        let expected_level = if name == "explicit-middle" {
            DangerLevel::WhenSurvivable
        } else {
            DangerLevel::Never
        };
        assert_eq!(globals.danger_level(), expected_level, "{name}");
        for script_grant in [false, true] {
            let script = WalkPermissions {
                allow_danger_zones: script_grant,
                ..WalkPermissions::default()
            };
            for walk_bit in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
                let walk = WalkOptions {
                    allow_danger_zones: walk_bit,
                    ..WalkOptions::default()
                };
                for crossing in [false, true] {
                    assert_eq!(
                        globals.danger_decision(script, walk, crossing),
                        never.danger_decision(script, walk, crossing),
                        "{name}: script={script_grant}, walk={walk_bit:?}, crossing={crossing}"
                    );
                }
            }
        }
        let decision =
            globals.danger_decision(WalkPermissions::default(), WalkOptions::default(), false);
        assert_eq!(
            decision,
            never.danger_decision(WalkPermissions::default(), WalkOptions::default(), false),
            "{name}: native inherit"
        );
        assert_eq!(decision.policy, RiskPolicy::Avoid, "{name}: native inherit");
        let manual = Admission::manual(globals.manual_decision(false), RiskInput::default(), 0);
        assert_eq!(
            manual.decision,
            Admission::manual(never.manual_decision(false), RiskInput::default(), 0).decision,
            "{name}: manual decision"
        );
        assert_eq!(manual.grants, ZoneExempt::NONE, "{name}: manual grants");
        assert!(
            !manual.decision.enforce,
            "{name}: held inherited verdict is advisory"
        );
        if let Some(world) = world.as_ref() {
            for (from, to) in [(tile(0, 1), tile(20, 1)), (tile(20, 1), tile(0, 1))] {
                let route = find(world, from, to);
                let mut risk_input = input(99, PoisonState::default());
                risk_input.pos = from;
                let request = Admission::manual(globals.manual_decision(false), risk_input, 1);
                let assessment = admission::assess(&route, world, &request);
                assert!(
                    admission::permits(&route, world, &request, &assessment),
                    "{name}: {from:?}->{to:?}"
                );
            }
        }
        eprintln!(
            "probe source={name} stored={:?} native=Avoid manual=Avoid",
            globals.danger_level(),
        );
    }
    // Requests permit danger routing, but keep the model's verdict enforced.
    let mut middle = WalkGlobals::default();
    middle.set_danger_level(DangerLevel::WhenSurvivable);
    let script = WalkPermissions {
        allow_danger_zones: true,
        ..WalkPermissions::default()
    };
    let allow = WalkOptions {
        allow_danger_zones: WalkBit::Allow,
        ..WalkOptions::default()
    };
    let forbid = WalkOptions {
        allow_danger_zones: WalkBit::Forbid,
        ..WalkOptions::default()
    };
    for (permissions, options) in [
        (script, WalkOptions::default()),
        (WalkPermissions::default(), allow),
        (script, forbid),
    ] {
        let decision = middle.danger_decision(permissions, options, false);
        assert_eq!(decision, never.danger_decision(permissions, options, false));
        assert!(decision.permitted);
        assert_eq!(decision.policy, RiskPolicy::Avoid);
        assert!(decision.enforce);
    }
    let unrequested = middle.danger_decision(WalkPermissions::default(), forbid, false);
    assert_eq!(
        unrequested,
        never.danger_decision(WalkPermissions::default(), forbid, false)
    );
    assert!(!unrequested.permitted);
    assert_eq!(unrequested.policy, RiskPolicy::Avoid);
    assert!(!unrequested.enforce);

    // Only the global override bypasses enforcement, with no Forbid exception.
    // Compat v1 reaches this decision through the same global override.
    for overridden in [
        WalkGlobals {
            allow_danger_zones: true,
            ..never
        },
        middle.for_script_family(true),
    ] {
        assert_eq!(overridden.danger_level(), DangerLevel::Always);
        for (permissions, options) in [
            (script, WalkOptions::default()),
            (WalkPermissions::default(), allow),
            (script, forbid),
            (WalkPermissions::default(), forbid),
        ] {
            let decision = overridden.danger_decision(permissions, options, false);
            assert!(decision.permitted);
            assert_eq!(decision.policy, RiskPolicy::Proceed);
            assert!(!decision.enforce);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn held_default_admits_router_endpoint_and_opt_in_proceeds() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = corridor(40, &[20]);
    let route = find(&world, tile(0, 1), tile(20, 1));
    let request = default_admission(input(99, PoisonState::default()));
    assert_eq!(request.decision.policy, RiskPolicy::Avoid);
    assert_eq!(request.grants, ZoneExempt::NONE);
    assert!(
        !request.decision.enforce,
        "the held default keeps its existing routing mode"
    );
    let assessment = admission::assess(&route, &world, &request);
    assert!(assessment.plan.complete);
    assert_eq!(assessment.plan.intervals.len(), 1);
    assert_eq!(assessment.plan.crossings.len(), 1);
    assert!(matches!(
        assessment.input.poison,
        PoisonState::Unknown { .. }
    ));
    assert_eq!(assessment.verdict, Verdict::Survivable);
    assert!(assessment.bound <= script::combat::risk::consts::DEATH_BOUND);
    assert!(matches!(
        assessment.poison_test,
        script::combat::risk::PoisonTest::PassesCharged { charged }
            if charged <= script::combat::risk::consts::DEATH_BOUND
    ));
    assert!(admission::permits(&route, &world, &request, &assessment));
    let opt_in = Admission {
        decision: DangerDecision {
            policy: RiskPolicy::Proceed,
            ..request.decision
        },
        ..request
    };
    assert_eq!(opt_in.decision.policy, RiskPolicy::Proceed);
    assert_eq!(opt_in.grants, ZoneExempt::NONE);
    assert!(admission::permits(&route, &world, &opt_in, &assessment));
}

#[test]
fn walking_out_from_inside_a_zone_is_not_an_entering_crossing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = corridor(80, &[2]);
    let route = find(&world, tile(2, 1), tile(78, 1));
    let mut start = input(99, PoisonState::default());
    start.pos = tile(2, 1);
    let request = default_admission(start);
    let assessment = admission::assess(&route, &world, &request);
    eprintln!(
        "probe origin verdict={:?} intervals={} crossings={} permitted={} reason={}",
        assessment.verdict,
        assessment.plan.intervals.len(),
        assessment.plan.crossings.len(),
        admission::permits(&route, &world, &request, &assessment),
        assessment.reason
    );
    assert!(
        assessment.plan.crossings.is_empty(),
        "leaving is not an entering crossing"
    );
    assert!(admission::permits(&route, &world, &request, &assessment));
    assert!(assessment.plan.complete);
    assert!(assessment.plan.intervals.iter().any(|row| row.escaping()));
    for policy in [RiskPolicy::Avoid, RiskPolicy::Proceed] {
        let enforcing = Admission {
            decision: DangerDecision {
                policy,
                enforce: true,
                ..request.decision
            },
            ..request
        };
        assert!(
            admission::permits(&route, &world, &enforcing, &assessment),
            "{policy:?}"
        );
    }
}

#[test]
fn no_crossing_walk_is_admitted_under_unknown_attacker_and_missing_facts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = corridor(16, &[]);
    let route = find(&world, tile(0, 1), tile(10, 1));
    let mut engaged = input(99, PoisonState::default());
    engaged.set_unattributed(true);
    let request = default_admission(engaged);
    let assessment = admission::assess(&route, &world, &request);
    eprintln!(
        "probe no-crossing engaged verdict={:?} crossings={} permitted={}",
        assessment.verdict,
        assessment.plan.crossings.len(),
        admission::permits(&route, &world, &request, &assessment)
    );
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(
        assessment.verdict,
        Verdict::Unknown(UnknownWhy::AlreadyEngaged)
    );
    assert!(admission::permits(&route, &world, &request, &assessment));

    let mut missing = input(99, PoisonState::default());
    missing.set_missing_facts(true);
    let request = default_admission(missing);
    let assessment = admission::assess(&route, &world, &request);
    eprintln!(
        "probe no-crossing missing-facts verdict={:?} permitted={}",
        assessment.verdict,
        admission::permits(&route, &world, &request, &assessment)
    );
    assert!(admission::permits(&route, &world, &request, &assessment));

    let clean = default_admission(input(99, PoisonState::default()));
    let assessment = admission::assess(&route, &world, &clean);
    assert_eq!(assessment.verdict, Verdict::Survivable);
    assert!(admission::permits(&route, &world, &clean, &assessment));
}

#[test]
fn input_problems_refuse_only_entering_crossings_in_every_enforcing_mode() {
    if !api::test_support::game_data_available() {
        return;
    }
    use script::combat::risk::LiveRow;
    use script::combat::{ActorKind, ActorRef};

    let world = corridor(40, &[20]);
    let safe_route = find(&world, tile(0, 1), tile(5, 1));
    let crossing_route = find(&world, tile(0, 1), tile(20, 1));
    let clean = input(99, PoisonState::Clear);
    let mut unattributed = clean;
    unattributed.set_unattributed(true);
    let mut missing = clean;
    missing.set_missing_facts(true);
    let mut overflow = clean;
    overflow.set_overflow(true);
    let mut too_many = clean;
    too_many.live_len = 5;
    let mut locked = clean;
    locked.set_input_locked(true);
    let mut unknown_npc = clean;
    unknown_npc.live_len = 1;
    unknown_npc.live[0] = LiveRow {
        actor: ActorRef {
            kind: ActorKind::Npc,
            index: 1,
        },
        ident: i32::MAX,
        max_hit: 255,
        rate: 1,
        due_tick: u16::MAX,
    };
    let mut player = unknown_npc;
    player.live[0].actor.kind = ActorKind::Player;
    for problem in [
        unattributed,
        missing,
        overflow,
        too_many,
        locked,
        unknown_npc,
        player,
    ] {
        for policy in [RiskPolicy::Avoid, RiskPolicy::Proceed] {
            let request = Admission {
                decision: DangerDecision {
                    policy,
                    enforce: true,
                    ..default_admission(problem).decision
                },
                ..default_admission(problem)
            };
            let assessment = admission::assess(&safe_route, &world, &request);
            assert!(assessment.plan.complete);
            assert!(assessment.plan.crossings.is_empty());
            assert!(matches!(assessment.verdict, Verdict::Unknown(_)));
            assert!(
                admission::permits(&safe_route, &world, &request, &assessment),
                "{problem:?}, {policy:?}"
            );
            let assessment = admission::assess(&crossing_route, &world, &request);
            assert!(!assessment.plan.crossings.is_empty());
            assert_eq!(
                admission::permits(&crossing_route, &world, &request, &assessment),
                policy == RiskPolicy::Proceed,
                "{problem:?}, {policy:?}",
            );
        }
    }
}

#[test]
fn reentering_after_an_origin_escape_is_a_crossing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = corridor(40, &[2]);
    let mut tiles: Vec<_> = (2..=20).map(|x| tile(x, 1)).collect();
    tiles.extend((2..20).rev().map(|x| tile(x, 1)));
    let route = Route {
        legs: vec![nav_planner::router::Leg::Walk { tiles }],
        dest: tile(2, 1),
        ticks: 20.0,
    };
    let mut risk_input = input(99, PoisonState::default());
    risk_input.pos = tile(2, 1);
    let request = Admission {
        decision: DangerDecision {
            enforce: true,
            ..default_admission(risk_input).decision
        },
        ..default_admission(risk_input)
    };
    let assessment = admission::assess(&route, &world, &request);
    assert_eq!(request.decision.policy, RiskPolicy::Avoid);
    assert_eq!(request.grants, ZoneExempt::NONE);
    assert!(request.decision.enforce);
    assert!(assessment.plan.complete);
    assert_eq!(assessment.plan.intervals.len(), 2);
    assert!(assessment.plan.intervals[0].escaping());
    assert!(!assessment.plan.intervals[1].escaping());
    assert_eq!(assessment.plan.crossings.len(), 1);
    assert!(matches!(
        assessment.input.poison,
        PoisonState::Unknown { .. }
    ));
    assert_eq!(assessment.verdict, Verdict::Survivable);
    assert!(assessment.bound <= script::combat::risk::consts::DEATH_BOUND);
    assert!(matches!(
        assessment.poison_test,
        script::combat::risk::PoisonTest::PassesCharged { charged }
            if charged <= script::combat::risk::consts::DEATH_BOUND
    ));
    assert!(admission::permits(&route, &world, &request, &assessment));
}

fn find_through(world: &NavWorld, from: WorldTile, to: WorldTile) -> Route {
    find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions {
            zones: ZoneExempt::all(),
            ..FindOptions::default()
        },
        &WorldState::default(),
        &[],
    )
    .expect("router route")
}

fn ice_r(world: &NavWorld) -> i32 {
    let combat =
        CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap();
    let ice = world.graph.zones.as_ref().unwrap().kinds()[0].npc_id;
    i32::from(RiskTables::build(&combat).npc(ice).unwrap().r)
}

/// R3 M-A (reviewer probe `fringe_origin_walk_into_acquisition_set_is_unjudged`):
/// an origin inside the pursuit envelope E_z but outside the acquisition set
/// A_z is not engaged. Walking into A_z is an entering crossing that enforcing
/// admission judges; only an engaged origin's walk out is an escape.
#[test]
fn fringe_origin_walk_into_the_acquisition_set_is_a_judged_crossing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let spawn = 30;
    let world = corridor(70, &[spawn]);
    let r = ice_r(&world);
    let dest = tile(69, 1);
    let enforcing = |from: WorldTile, hp: u8| {
        let mut risk_input = input(hp, PoisonState::Clear);
        risk_input.pos = from;
        Admission {
            decision: DangerDecision {
                enforce: true,
                ..default_admission(risk_input).decision
            },
            grants: ZoneExempt::NONE,
            ..default_admission(risk_input)
        }
    };
    for from_x in [spawn - r, spawn - r - 1] {
        let from = tile(from_x, 1);
        let route = find_through(&world, from, dest);
        let low = enforcing(from, 12);
        let assessment = admission::assess(&route, &world, &low);
        eprintln!(
            "probe fringe from x={from_x} (E_z from x={}) verdict={:?} crossings={} permitted={} reason={}",
            spawn - r,
            assessment.verdict,
            assessment.plan.crossings.len(),
            admission::permits(&route, &world, &low, &assessment),
            assessment.reason
        );
        assert!(assessment.plan.complete);
        assert!(assessment.plan.intervals.iter().all(|row| !row.escaping()));
        assert_eq!(assessment.plan.crossings.len(), 1, "from x={from_x}");
        assert_eq!(assessment.verdict, Verdict::Unsurvivable, "from x={from_x}");
        assert!(
            !admission::permits(&route, &world, &low, &assessment),
            "from x={from_x}"
        );
        let high = enforcing(from, 99);
        let assessment = admission::assess(&route, &world, &high);
        assert_eq!(assessment.verdict, Verdict::Survivable);
        assert!(admission::permits(&route, &world, &high, &assessment));
    }
    // Engaged control: standing on the spawn, the same walk out is escape.
    let from = tile(spawn, 1);
    let route = find_through(&world, from, dest);
    let request = enforcing(from, 12);
    let assessment = admission::assess(&route, &world, &request);
    assert!(assessment.plan.crossings.is_empty());
    assert!(assessment.plan.intervals[0].escaping());
    assert!(admission::permits(&route, &world, &request, &assessment));
}

/// Old: display overflow made this Unknown with an incomplete plan.
/// New: the timeline has a numeric death bound; display totals saturate.
#[test]
fn saturated_display_is_refused_numerically_by_every_enforcing_request() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Forty-one overlapping ice warrior envelopes exceed the display floor.
    let spawns: Vec<i32> = (20..=60).collect();
    let world = corridor(100, &spawns);
    let route = find_through(&world, tile(0, 1), tile(99, 1));
    let held = default_admission(input(99, PoisonState::Clear));
    let assessment = admission::assess(&route, &world, &held);
    eprintln!(
        "probe failed-assessment verdict={:?} complete={} reason={}",
        assessment.verdict, assessment.plan.complete, assessment.reason
    );
    assert_eq!(assessment.verdict, Verdict::Unsurvivable);
    assert!(assessment.bound > script::combat::risk::consts::DEATH_BOUND);
    assert!(assessment.plan.complete);
    assert!(!held.decision.enforce);
    assert!(admission::permits(&route, &world, &held, &assessment));
    for policy in [RiskPolicy::Avoid, RiskPolicy::Proceed] {
        for grants in [ZoneExempt::NONE, ZoneExempt::all()] {
            let enforcing = Admission {
                decision: DangerDecision {
                    policy,
                    enforce: true,
                    ..held.decision
                },
                grants,
                ..held
            };
            assert_eq!(
                admission::permits(&route, &world, &enforcing, &assessment),
                policy == RiskPolicy::Proceed,
                "{policy:?}"
            );
            let marker_only = Admission {
                compat_v1: true,
                ..enforcing
            };
            assert_eq!(
                admission::permits(&route, &world, &marker_only, &assessment),
                policy == RiskPolicy::Proceed,
                "the compatibility marker does not bypass the decision"
            );
            let decision = WalkGlobals::default()
                .for_script_family(true)
                .danger_decision(WalkPermissions::default(), WalkOptions::default(), false);
            assert_eq!(decision.policy, RiskPolicy::Proceed);
            assert!(!decision.enforce);
            let compat = Admission {
                decision,
                ..marker_only
            };
            assert!(admission::permits(&route, &world, &compat, &assessment));
        }
    }
    assert_eq!(
        admission::verdict_refusal(assessment.verdict),
        WalkRefusal::Unsurvivable
    );
}

/// A real long crossing formerly failed while narrowing the display floor.
/// The bound now decides admission, independently of saturated display totals.
#[test]
#[ignore = "needs WORLD_NAV_PACK (a real 289 pack)"]
fn real_pack_display_overflow_is_judged_numerically_by_enforcing_avoid() {
    if !api::test_support::game_data_available() {
        return;
    }
    let Some(path) = api::test_support::local_asset_path(
        "WORLD_NAV_PACK",
        "",
        "real 289 navigation pack for survivability regression",
    ) else {
        return;
    };
    let world = NavWorld::load_pack(&path).unwrap();
    let state = WorldState::default().with_map_members(true);
    let from = tile(3222, 3218);
    for to in [tile(2662, 3305), tile(2809, 3436)] {
        let route = find_with_avoid(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::all(),
                ..FindOptions::default()
            },
            &state,
            &[],
        )
        .expect("real router route");
        let mut risk_input = input(60, PoisonState::Clear);
        risk_input.pos = from;
        let request = Admission {
            decision: DangerDecision {
                enforce: true,
                ..default_admission(risk_input).decision
            },
            ..default_admission(risk_input)
        };
        let assessment = admission::assess(&route, &world, &request);
        let permitted = admission::permits(&route, &world, &request, &assessment);
        eprintln!(
            "probe real {:?}->{:?}: verdict={:?} complete={} permitted(enforcing Avoid)={permitted}",
            (from.x, from.z),
            (to.x, to.z),
            assessment.verdict,
            assessment.plan.complete
        );
        assert_eq!(assessment.verdict, Verdict::Unsurvivable);
        assert!(assessment.plan.complete);
        assert!(assessment.bound > script::combat::risk::consts::DEATH_BOUND);
        assert!(!permitted);
    }
}

/// The two walks of the 289 Quester campaign (QUEST-CAMPAIGN-1109) that the
/// survival check refused although a combat-40 account can make them: the
/// Black Knights' Fortress grill-door walk (seven knights; 61.3% before, with
/// every attacker summed) and a walk through Mort'ton (every Loar Shadow was
/// `MissingFacts`). Each walk declares its crossing, as the guide does, and the
/// account has no armour bonus and no food credit.
#[test]
#[ignore = "needs WORLD_NAV_PACK (a real 289 pack)"]
fn real_pack_fortress_and_mortton_walks_of_a_base_40_account_are_admitted() {
    if !api::test_support::game_data_available() {
        return;
    }
    let Some(path) = api::test_support::local_asset_path(
        "WORLD_NAV_PACK",
        "",
        "real 289 navigation pack for survivability regression",
    ) else {
        return;
    };
    let world = NavWorld::load_pack(&path).unwrap();
    assert!(
        !world.graph.multiway.is_empty(),
        "the pack carries the multiway table"
    );
    let state = WorldState::default().with_map_members(true);
    let level = |x, z, level| WorldTile { x, z, level };
    for (name, from, to) in [
        ("fortress", level(3016, 3517, 0), level(3025, 3511, 1)),
        ("mortton", level(3483, 3295, 0), level(3481, 3279, 0)),
    ] {
        let route = find_with_avoid(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::all(),
                ..FindOptions::default()
            },
            &state,
            &[],
        )
        .expect("real router route");
        let mut risk_input = input(40, PoisonState::Clear);
        risk_input.pos = from;
        risk_input.hp_max = 40;
        risk_input.defence = 40;
        risk_input.combat = Some(40);
        let request = Admission {
            decision: DangerDecision {
                policy: RiskPolicy::Avoid,
                enforce: true,
                ..default_admission(risk_input).decision
            },
            grants: ZoneExempt::all(),
            ..default_admission(risk_input)
        };
        let assessment = admission::assess(&route, &world, &request);
        assert!(assessment.plan.complete, "{name}: {}", assessment.reason);
        assert_eq!(
            assessment.verdict,
            Verdict::Survivable,
            "{name}: {}",
            assessment.reason
        );
        assert!(
            assessment.bound <= script::combat::risk::consts::DEATH_BOUND,
            "{name}: bound {}",
            assessment.bound
        );
        assert!(
            admission::permits(&route, &world, &request, &assessment),
            "{name}"
        );
    }
}
