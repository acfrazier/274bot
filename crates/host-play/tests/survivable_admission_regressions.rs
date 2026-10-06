//! Regressions from the S2b review: held defaults preserve router admission and fleeing is always possible.
use api::selected::ClientRevision;
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use host_play::admission::{self, Admission, PoisonState, RiskInput};
use host_play::{DangerLevel, WalkGlobals, NET_AVAILABLE};
use nav::collision::{pack_walk, WorldCollision};
use nav::router::{find_with_avoid, FindOptions, Route};
use nav::transport::TransportGraph;
use nav::world::NavWorld;
use nav::zones::{Zone, ZoneClass, ZoneExempt, ZoneKind, ZoneTable, NO_GROUP};
use nav::WorldState;
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
    RiskInput {
        pos: tile(0, 1),
        poison,
        tick: 10,
        hp,
        hp_max: 99,
        prayer: 10,
        prayer_base: 10,
        combat: Some(50),
        missing_facts: false,
        unattributed: false,
        input_locked: false,
        overflow: false,
        ..RiskInput::default()
    }
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
    Admission::manual(FindOptions::default(), input, 1, WalkGlobals::default())
}

#[test]
fn probe_gated_default_is_never_for_every_preference_source() {
    const { assert!(!NET_AVAILABLE) };
    let dir = std::env::temp_dir().join(format!("review-s2b-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: &str| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    };
    let absent = dir.join("absent.json");
    let old = write("old.json", r#"{"nav":{"allow_danger_zones":false}}"#);
    let empty_nav = write("empty-nav.json", r#"{"nav":{}}"#);
    let no_nav = write("no-nav.json", r#"{"unrelated":1}"#);
    let explicit = write(
        "explicit.json",
        r#"{"nav":{"allow_danger_zones":false,"survivable_routing":true}}"#,
    );
    let malformed = write("malformed.json", "{nav:");
    let sources = [
        ("default", WalkGlobals::default()),
        ("absent", WalkGlobals::read_at(&absent).unwrap()),
        ("old", WalkGlobals::read_at(&old).unwrap()),
        ("empty-nav", WalkGlobals::read_at(&empty_nav).unwrap()),
        ("no-nav", WalkGlobals::read_at(&no_nav).unwrap()),
        ("explicit", WalkGlobals::read_at(&explicit).unwrap()),
        (
            "malformed",
            WalkGlobals::read_at(&malformed).unwrap_or_else(|_| WalkGlobals::fail_closed()),
        ),
    ];
    for (name, globals) in sources {
        assert_eq!(
            globals.effective_danger_level(),
            DangerLevel::Never,
            "{name}"
        );
        assert_eq!(
            globals.risk_policy(WalkPermissions::default(), WalkOptions::default()),
            RiskPolicy::Avoid,
            "{name}: native inherit"
        );
        let manual = Admission::manual(
            globals.manual_options(false),
            RiskInput::default(),
            0,
            globals,
        );
        assert_eq!(manual.policy, RiskPolicy::Avoid, "{name}: manual");
        assert_eq!(manual.grants, ZoneExempt::NONE, "{name}: manual grants");
        assert!(
            !manual.enforce,
            "{name}: held inherited verdict is advisory"
        );
        let world = corridor(40, &[20]);
        for (from, to) in [(tile(0, 1), tile(20, 1)), (tile(20, 1), tile(0, 1))] {
            let route = find(&world, from, to);
            let request = Admission::manual(
                globals.manual_options(false),
                RiskInput {
                    pos: from,
                    ..input(99, PoisonState::default())
                },
                1,
                globals,
            );
            let assessment = admission::assess(&route, &world, &request);
            assert!(
                admission::permits(&route, &world, &request, &assessment),
                "{name}: {from:?}->{to:?}"
            );
        }
        eprintln!(
            "probe source={name} stored={:?} effective={:?} native=Avoid manual=Avoid",
            globals.danger_level(),
            globals.effective_danger_level()
        );
    }
    // Explicit overrides still proceed; Forbid beats a script grant.
    let globals = WalkGlobals::default();
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
    assert_eq!(
        globals.risk_policy(script, WalkOptions::default()),
        RiskPolicy::Proceed
    );
    assert_eq!(
        globals.risk_policy(WalkPermissions::default(), allow),
        RiskPolicy::Proceed
    );
    assert_eq!(globals.risk_policy(script, forbid), RiskPolicy::Avoid);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn held_default_admits_router_endpoint_and_opt_in_proceeds() {
    let world = corridor(40, &[20]);
    let route = find(&world, tile(0, 1), tile(20, 1));
    let request = default_admission(input(99, PoisonState::default()));
    assert_eq!(request.policy, RiskPolicy::Avoid);
    let assessment = admission::assess(&route, &world, &request);
    eprintln!(
        "probe endpoint verdict={:?} intervals={} crossings={} reason={}",
        assessment.verdict,
        assessment.plan.intervals.len(),
        assessment.plan.crossings.len(),
        assessment.reason
    );
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
    assert!(admission::permits(&route, &world, &request, &assessment));
    assert_eq!(
        admission::verdict_refusal(assessment.verdict),
        WalkRefusal::Unknown(UnknownWhy::Poison)
    );
    let opt_in = Admission {
        policy: RiskPolicy::Proceed,
        ..request
    };
    assert!(admission::permits(&route, &world, &opt_in, &assessment));
}

#[test]
fn walking_out_from_inside_a_zone_is_not_an_entering_crossing() {
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
    for policy in [RiskPolicy::Avoid, RiskPolicy::Inherit, RiskPolicy::Proceed] {
        let enforcing = Admission {
            policy,
            enforce: true,
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
    let world = corridor(16, &[]);
    let route = find(&world, tile(0, 1), tile(10, 1));
    let mut engaged = input(99, PoisonState::default());
    engaged.unattributed = true;
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
    missing.missing_facts = true;
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
    use script::combat::risk::LiveRow;
    use script::combat::{ActorKind, ActorRef};

    let world = corridor(40, &[20]);
    let safe_route = find(&world, tile(0, 1), tile(5, 1));
    let crossing_route = find(&world, tile(0, 1), tile(20, 1));
    let clean = input(99, PoisonState::Clear);
    let mut unattributed = clean;
    unattributed.unattributed = true;
    let mut missing = clean;
    missing.missing_facts = true;
    let mut overflow = clean;
    overflow.overflow = true;
    let mut too_many = clean;
    too_many.live_len = 5;
    let mut locked = clean;
    locked.input_locked = true;
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
        for policy in [RiskPolicy::Avoid, RiskPolicy::Inherit, RiskPolicy::Proceed] {
            let request = Admission {
                policy,
                enforce: true,
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
    let world = corridor(40, &[2]);
    let mut tiles: Vec<_> = (2..=20).map(|x| tile(x, 1)).collect();
    tiles.extend((2..20).rev().map(|x| tile(x, 1)));
    let route = Route {
        legs: vec![nav::router::Leg::Walk { tiles }],
        dest: tile(2, 1),
        ticks: 20.0,
    };
    let request = Admission {
        enforce: true,
        ..default_admission(RiskInput {
            pos: tile(2, 1),
            ..input(99, PoisonState::default())
        })
    };
    let assessment = admission::assess(&route, &world, &request);
    assert!(assessment.plan.complete);
    assert_eq!(assessment.plan.intervals.len(), 2);
    assert!(assessment.plan.intervals[0].escaping());
    assert!(!assessment.plan.intervals[1].escaping());
    assert_eq!(assessment.plan.crossings.len(), 1);
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
    assert!(!admission::permits(&route, &world, &request, &assessment));
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
    let risks = RiskTables::build(world.graph.zones.as_ref().unwrap(), &combat);
    i32::from(risks.kind(0).unwrap().r)
}

/// R3 M-A (reviewer probe `fringe_origin_walk_into_acquisition_set_is_unjudged`):
/// an origin inside the pursuit envelope E_z but outside the acquisition set
/// A_z is not engaged. Walking into A_z is an entering crossing that enforcing
/// admission judges; only an engaged origin's walk out is an escape.
#[test]
fn fringe_origin_walk_into_the_acquisition_set_is_a_judged_crossing() {
    let spawn = 30;
    let world = corridor(70, &[spawn]);
    let r = ice_r(&world);
    let dest = tile(69, 1);
    let enforcing = |from: WorldTile, hp: u8| Admission {
        policy: RiskPolicy::Inherit,
        enforce: true,
        grants: ZoneExempt::NONE,
        ..default_admission(RiskInput {
            pos: from,
            ..input(hp, PoisonState::Clear)
        })
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

/// R3 M-B: a failed assessment (`Unknown(Overflow)` from a crossing whose
/// summed volley/floor exceeds every HP) carries an incomplete plan. Every
/// enforcing non-Proceed request refuses it; held, Proceed and compat v1
/// keep admitting it.
#[test]
fn failed_assessment_is_refused_by_every_enforcing_request() {
    // Forty-one overlapping ice warrior envelopes: the summed way-out floor
    // exceeds the u8 report bound, so the estimate itself fails.
    let spawns: Vec<i32> = (20..=60).collect();
    let world = corridor(100, &spawns);
    let route = find_through(&world, tile(0, 1), tile(99, 1));
    let held = default_admission(input(99, PoisonState::Clear));
    let assessment = admission::assess(&route, &world, &held);
    eprintln!(
        "probe failed-assessment verdict={:?} complete={} reason={}",
        assessment.verdict, assessment.plan.complete, assessment.reason
    );
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Overflow));
    assert!(!assessment.plan.complete);
    assert!(!held.enforce);
    assert!(admission::permits(&route, &world, &held, &assessment));
    for policy in [RiskPolicy::Avoid, RiskPolicy::Inherit, RiskPolicy::Proceed] {
        for grants in [ZoneExempt::NONE, ZoneExempt::all()] {
            let enforcing = Admission {
                policy,
                enforce: true,
                grants,
                ..held
            };
            assert_eq!(
                admission::permits(&route, &world, &enforcing, &assessment),
                policy == RiskPolicy::Proceed,
                "{policy:?}"
            );
            let compat = Admission {
                compat_v1: true,
                ..enforcing
            };
            assert!(admission::permits(&route, &world, &compat, &assessment));
        }
    }
    assert_eq!(
        admission::verdict_refusal(assessment.verdict),
        WalkRefusal::Unknown(UnknownWhy::Overflow)
    );
}

/// R3 M-B on the real pack (reviewer probe `timing_long_assessments`):
/// Lumbridge to Ardougne/Catherby through White Wolf Mountain at combat 50
/// has a crossing whose way-out floor (547 HP) exceeds every HP. The
/// estimate reports `Unknown(Overflow)`; enforcing Avoid must refuse it.
#[test]
#[ignore = "needs WORLD_NAV_PACK (a real 289 pack)"]
fn real_pack_overflowed_assessment_is_refused_by_enforcing_avoid() {
    let path = std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK");
    let world = NavWorld::load_pack(std::path::Path::new(&path)).unwrap();
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
        let request = Admission {
            policy: RiskPolicy::Avoid,
            enforce: true,
            ..default_admission(RiskInput {
                pos: from,
                ..input(60, PoisonState::Clear)
            })
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
        assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Overflow));
        assert!(!permitted);
    }
}
