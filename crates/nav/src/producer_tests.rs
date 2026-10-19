//! The owner-boundary producers are checked against the storage they wrap
//! and against the F0 facade semantics they must not change: one identity
//! per loaded world, `NavLease::wrap` still fresh per call, bound
//! exemptions equal to the router's `ZoneExempt::contains_zone`.

use std::sync::{Arc, Barrier, Mutex};

use api::snapshot::WorldTile;

use crate::collision::{pack_walk, WorldCollision};
use crate::contract::{ContractError, NavLease, PackDigest, ZoneKey};
use crate::router::{AvoidRect, Leg, Route};
use crate::transport::{TransportEdge, TransportGraph, TransportKind, WildernessRules};
use crate::world::NavWorld;
use crate::zones::{Zone, ZoneClass, ZoneExempt, ZoneGroup, ZoneKind, ZoneTable};

use super::*;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn open_grid(width: usize, height: usize) -> WorldCollision {
    let plane = width * height;
    let flags = vec![0u32; 4 * plane];
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0, 0),
        width,
        height,
        walk,
        blocked,
        flags: Some(flags),
    }
}

/// Three zones: 0 and 1 are the two members of group 0, 2 is ungrouped.
fn zone_table() -> ZoneTable {
    let mut first = Zone::npc(tile(2, 2, 0), 1, ZoneClass::Always, u16::MAX, 0);
    first.group = 0;
    let mut second = Zone::npc(tile(8, 2, 0), 1, ZoneClass::Always, u16::MAX, 0);
    second.group = 0;
    let third = Zone::npc(tile(8, 8, 0), 1, ZoneClass::Always, u16::MAX, 0);
    ZoneTable::from_parts(
        vec![first, second, third],
        vec![ZoneKind::new("wolf", "Wolf", 96, 1, false, false)],
        vec![ZoneGroup::new(
            "white-wolf-mountain",
            "White Wolf Mountain",
            AvoidRect {
                min_x: 0,
                max_x: 10,
                min_z: 0,
                max_z: 4,
                level: None,
            },
            vec![0u16, 1u16],
        )],
        Vec::new(),
        Vec::new(),
        tile(0, 0, 0),
        12,
        12,
        &WildernessRules::default(),
    )
    .unwrap()
}

fn zone_world() -> Arc<NavWorld> {
    let mut world = NavWorld::from_parts(open_grid(12, 12), TransportGraph::default(), Vec::new());
    world.graph.zones = Some(zone_table());
    Arc::new(world)
}

#[test]
fn one_identity_per_world_and_wrap_stays_fresh() {
    let world = Arc::new(NavWorld::from_parts(
        open_grid(1, 1),
        TransportGraph::default(),
        Vec::new(),
    ));
    let first = identity(&world).unwrap();
    assert_eq!(identity(&world).unwrap(), first, "minted once");
    assert_eq!(lease(&world).unwrap().identity(), first);
    assert_eq!(collision_view(&world).unwrap().pack(), first.pack());
    assert_eq!(
        first.content(),
        PackDigest::of(&[]),
        "a synthetic world carries the empty-content digest"
    );
    // F0 semantics: every `wrap` is a distinct lease even of this world.
    let wrapped = NavLease::wrap(Arc::clone(&world), PackDigest::of(b"x")).unwrap();
    assert_ne!(wrapped.identity().pack(), first.pack());
    assert_eq!(identity(&world).unwrap(), first, "wrap never rebinds");
}

#[test]
fn bind_records_the_pack_digest_once() {
    let world = Arc::new(NavWorld::from_parts(
        open_grid(1, 1),
        TransportGraph::default(),
        Vec::new(),
    ));
    let content = PackDigest::of(b"pack bytes");
    let bound = bind(&world, content).unwrap();
    assert_eq!(bound.content(), content);
    assert_eq!(bind(&world, content).unwrap(), bound);
    assert_eq!(
        bind(&world, PackDigest::of(b"other bytes")).unwrap_err(),
        ContractError::PackMismatch
    );
    assert_eq!(identity(&world).unwrap(), bound);
    assert_eq!(lease(&world).unwrap().identity(), bound);
}

#[test]
fn from_bytes_records_the_bytes_digest() {
    let bytes = crate::pack::encode_grid(&crate::grid::StepGrid::fixture_open_3x3());
    let world = Arc::new(NavWorld::from_bytes(&bytes).unwrap());
    assert_eq!(identity(&world).unwrap().content(), PackDigest::of(&bytes));
    assert_eq!(
        bind(&world, PackDigest::of(b"not the pack")).unwrap_err(),
        ContractError::PackMismatch
    );
}

#[test]
fn route_view_borrows_the_owner_route_under_the_world_identity() {
    let world = zone_world();
    let other = zone_world();
    let route = Route {
        legs: vec![
            Leg::Walk {
                tiles: vec![tile(1, 1, 0), tile(2, 2, 0)],
            },
            Leg::Transport {
                edge: Box::new(TransportEdge {
                    kind: TransportKind::Door,
                    at: tile(2, 2, 0),
                    to: tile(3, 3, 0),
                    takeoff: None,
                    player_delta: None,
                    loc_id: 7,
                    option: 1,
                    ticks: 2,
                    dir: None,
                    open_loc_id: None,
                    skill_req: vec![],
                    item_req: vec![],
                    consumed_req: vec![],
                    item_returns: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    worn_all_req: vec![],
                    forbidden_held: vec![],
                    forbidden_worn: vec![],
                    members_req: false,
                    wildy_cap: None,
                    quest_gates: None,
                }),
            },
        ],
        dest: tile(3, 3, 0),
        ticks: 2.5,
    };
    let view = route_view(&*world, &route).unwrap();
    assert_eq!(view.pack(), identity(&world).unwrap().pack());
    assert_eq!(view.pack(), zones_view(&world).unwrap().unwrap().pack());
    let world_lease = lease(&world).unwrap();
    assert_eq!(view.pack(), world_lease.zones().unwrap().pack());
    assert_eq!(
        route_view(&world_lease, &route).unwrap().pack(),
        view.pack(),
        "the lease serves the same identity as the world"
    );
    assert_ne!(view.pack(), identity(&other).unwrap().pack());
    assert_ne!(
        view.pack(),
        NavLease::wrap(Arc::clone(&world), PackDigest::of(b"x"))
            .unwrap()
            .identity()
            .pack(),
        "a wrapped lease is its own identity, not the world's"
    );
    assert_eq!(view.destination(), route.dest);
    assert_eq!(view.ticks(), 2.5);
    assert_eq!(view.leg_count(), 2);
    assert_eq!(
        view.leg(0).unwrap().walk_tiles().unwrap(),
        &[tile(1, 1, 0), tile(2, 2, 0)]
    );
    let hop = view.leg(1).unwrap().transport().unwrap();
    assert_eq!(hop.kind(), TransportKind::Door);
    assert_eq!(hop.ticks(), 2);
    assert_eq!(hop.effective_takeoff(), tile(2, 2, 0));
    assert_eq!(view.leg(2).unwrap_err(), ContractError::NoSuchLeg);
    // Route storage lives with the owner; the view is a borrow.
    assert_eq!(route.legs.len(), 2);
}

#[test]
fn bound_exemption_matches_the_raw_contains_zone() {
    let world = zone_world();
    let zones = zones_view(&world).unwrap().unwrap();
    let table = world.graph.zones.as_ref().unwrap();
    let cases: [(&str, ZoneExempt); 6] = [
        ("none", ZoneExempt::NONE),
        ("zone0", ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap()),
        ("zone2", ZoneExempt::named(&[ZoneKey::Zone(2)]).unwrap()),
        ("group0", ZoneExempt::named(&[ZoneKey::Group(0)]).unwrap()),
        (
            "zone2+group0",
            ZoneExempt::named(&[ZoneKey::Zone(2), ZoneKey::Group(0)]).unwrap(),
        ),
        ("all", ZoneExempt::all()),
    ];
    for (name, raw) in cases {
        let bound = bind_exempt(zones, raw).unwrap();
        assert_eq!(bound.pack(), zones.pack(), "{name}");
        assert_eq!(bound.is_all(), raw.is_all(), "{name}");
        for index in 0..3u16 {
            let key = zones.bind(ZoneKey::Zone(index)).unwrap();
            assert_eq!(
                zones.exemption_contains(bound, key).unwrap(),
                raw.contains_zone(index, table),
                "{name}: zone {index}"
            );
        }
    }
    assert_eq!(
        bind_exempt(zones, ZoneExempt::named(&[ZoneKey::Zone(9)]).unwrap()).unwrap_err(),
        ContractError::UnknownKey
    );
    assert_eq!(
        bind_exempt(zones, ZoneExempt::named(&[ZoneKey::Group(1)]).unwrap()).unwrap_err(),
        ContractError::UnknownKey
    );
    // Another world's view refuses this world's binding.
    let other = zone_world();
    let foreign = zones_view(&other).unwrap().unwrap();
    let bound = bind_exempt(zones, ZoneExempt::named(&[ZoneKey::Zone(0)]).unwrap()).unwrap();
    assert_eq!(
        foreign.exemption_contains(bound, foreign.bind(ZoneKey::Zone(0)).unwrap()),
        Err(ContractError::PackMismatch)
    );
}

#[test]
fn bound_keys_and_group_members_name_the_raw_rows() {
    let world = zone_world();
    let zones = zones_view(&world).unwrap().unwrap();
    let table = world.graph.zones.as_ref().unwrap();
    for index in 0..3u16 {
        let bound = bind_key(zones, ZoneKey::Zone(index)).unwrap();
        assert_eq!(bound.key(), ZoneKey::Zone(index));
        assert_eq!(bound.pack(), zones.pack());
        assert_eq!(zones.zone(bound).unwrap().key(), bound);
    }
    let group = bind_key(zones, ZoneKey::Group(0)).unwrap();
    let members: Vec<_> = group_members(zones, group).unwrap().collect();
    let raw: Vec<_> = table.groups()[0]
        .members
        .iter()
        .map(|&member| bind_key(zones, ZoneKey::Zone(member)).unwrap())
        .collect();
    assert_eq!(members, raw);
    for member in members {
        assert_eq!(zones.zone(member).unwrap().group(), Some(group));
    }
    // The world's lease serves the same identity: its view accepts keys the
    // borrowed producer bound.
    let leased = lease(&world).unwrap();
    let leased = leased.zones().unwrap();
    assert_eq!(
        leased
            .zone(group_members(zones, group).unwrap().next().unwrap())
            .unwrap()
            .key(),
        raw[0]
    );

    assert_eq!(
        bind_key(zones, ZoneKey::Zone(3)),
        Err(ContractError::UnknownKey)
    );
    assert_eq!(
        bind_key(zones, ZoneKey::Group(1)),
        Err(ContractError::UnknownKey)
    );
    let zone = bind_key(zones, ZoneKey::Zone(0)).unwrap();
    assert!(matches!(
        group_members(zones, zone),
        Err(ContractError::UnknownKey)
    ));
    let other = zone_world();
    let foreign = zones_view(&other).unwrap().unwrap();
    assert!(matches!(
        group_members(foreign, group),
        Err(ContractError::PackMismatch)
    ));
}

#[test]
fn views_of_a_legacy_world_have_no_zone_table() {
    let world = Arc::new(NavWorld::from_grid(
        &crate::grid::StepGrid::fixture_open_3x3(),
    ));
    let legacy = lease(&world).unwrap();
    assert!(legacy.zones().is_none());
    assert!(zones_view(&world).unwrap().is_none());
    let wild = wilderness_view(&world).unwrap();
    assert!(!wild.contains(tile(1, 1, 0)));
    assert_eq!(wild.level(tile(1, 1, 0)), 0);
    assert!(collision_view(&world).unwrap().standable(tile(1, 1, 0)));
    assert_eq!(
        collision_view(&world).unwrap().pack(),
        legacy.identity().pack(),
        "the per-tick view and the lease share the world's identity"
    );
}

#[test]
fn owner_result_ledger_holds_charge_until_the_last_alias_drops() {
    let ledger = OwnerResultLedger::with_limits(100, 90);
    let reservation = ledger.try_reserve_bytes(80).unwrap();
    let result = reservation.clone().hold(String::from("owned"));
    let alias = Arc::clone(&result);
    drop(reservation);
    drop(result);
    assert_eq!(ledger.used_bytes(), 80);
    assert_eq!(alias.value(), "owned");
    drop(alias);
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn owner_result_ledger_refusals_leave_usage_unchanged() {
    let ledger = OwnerResultLedger::with_limits(100, 60);
    let oversized = match ledger.try_reserve_bytes(61) {
        Err(refusal) => refusal,
        Ok(_) => panic!("oversized result was reserved"),
    };
    assert_eq!(
        oversized,
        OwnerResultRefusal::TooLarge {
            requested: 61,
            maximum: 60,
        }
    );
    let held = ledger.try_reserve_bytes(60).unwrap();
    let full = match ledger.try_reserve_bytes(41) {
        Err(refusal) => refusal,
        Ok(_) => panic!("full ledger admitted another result"),
    };
    assert_eq!(
        full,
        OwnerResultRefusal::Full {
            requested: 41,
            used: 60,
            capacity: 100,
        }
    );
    assert_eq!(ledger.used_bytes(), 60);
    drop(held);
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn owner_result_bytes_includes_over_aligned_arc_padding() {
    #[repr(align(64))]
    struct Aligned;

    let ledger = OwnerResultLedger::with_limits(1024, 1024);
    let declared = OwnerResultLedger::owner_result_bytes::<Aligned>(0).unwrap();
    let allocations = allocation_counter::measure(|| {
        let reservation = ledger.try_reserve_bytes(declared).unwrap();
        let result = reservation.hold(Aligned);
        assert_eq!(result.charged_bytes(), declared);
        drop(result);
    });
    assert_eq!(usize::try_from(allocations.bytes_max).unwrap(), declared);
    assert_eq!(allocations.bytes_current, 0);
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn final_owner_arc_drops_under_nav_lock_during_service_reservation() {
    let ledger = Arc::new(OwnerResultLedger::with_limits(100, 100));
    let nav = Arc::new(Mutex::new(Some(
        ledger
            .try_reserve_bytes(40)
            .unwrap()
            .hold(String::from("bank route")),
    )));
    let service_operation = Arc::new(Mutex::new(()));
    let rendezvous = Arc::new(Barrier::new(2));

    let worker_ledger = Arc::clone(&ledger);
    let worker_nav = Arc::clone(&nav);
    let worker_service = Arc::clone(&service_operation);
    let worker_rendezvous = Arc::clone(&rendezvous);
    let worker = std::thread::spawn(move || {
        // Model a service operation reserving new capacity while the nav
        // owner retires its final result Arc under the slot mutex.
        let _service = worker_service.lock().unwrap();
        let reservation = worker_ledger.try_reserve_bytes(60).unwrap();
        worker_rendezvous.wait();
        let _nav = worker_nav.lock().unwrap();
        assert_eq!(worker_ledger.used_bytes(), 60);
        drop(reservation);
    });

    let mut nav = nav.lock().unwrap();
    rendezvous.wait();
    drop(nav.take());
    assert_eq!(ledger.used_bytes(), 60);
    drop(nav);

    worker.join().unwrap();
    assert_eq!(ledger.used_bytes(), 0);
}
