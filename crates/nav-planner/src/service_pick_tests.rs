use api::service_pick::{ServiceCandidate, ServiceKind};
use api::snapshot::WorldTile;

use crate::collision::{pack_walk, WorldCollision, WALK_SCENERY, WALL_WEST};
use crate::named_banks::{AccessReach, Footprint};
use crate::router::{FindOptions, SearchRequest};
use crate::service_pick::{
    nearest_service_air, nearest_service_routed, routed_request, service_access_tiles, Sides,
};
use crate::transport::{TransportEdge, TransportGraph, TransportKind};
use crate::world_state::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn collision(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
    let mut plane = vec![0u32; width * height];
    for &(x, z, flags) in extras {
        plane[z as usize * width + x as usize] |= flags;
    }
    let mut flags = vec![0u32; plane.len() * 4];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

#[test]
fn packed_forceapproach_rotates_open_edge_for_a_one_by_two_footprint() {
    let wc = collision(8, 8, &[]);
    let footprint = Footprint {
        x: 2,
        z: 2,
        level: 0,
        width: 1,
        length: 2,
    };
    let east: Vec<_> = service_access_tiles(
        &wc,
        footprint,
        AccessReach::InLine,
        Sides::from_approach(0b1101, 0),
    )
    .collect();
    assert_eq!(east, vec![tile(3, 2), tile(3, 3)]);

    let south: Vec<_> = service_access_tiles(
        &wc,
        footprint,
        AccessReach::InLine,
        Sides::from_approach(0b1101, 1),
    )
    .collect();
    assert_eq!(south, vec![tile(2, 1)]);

    let west: Vec<_> = service_access_tiles(
        &wc,
        footprint,
        AccessReach::InLine,
        Sides::from_approach(0b1101, 2),
    )
    .collect();
    assert_eq!(west, vec![tile(1, 2), tile(1, 3)]);

    let north: Vec<_> = service_access_tiles(
        &wc,
        footprint,
        AccessReach::InLine,
        Sides::from_approach(0b1101, 3),
    )
    .collect();
    assert_eq!(north, vec![tile(2, 4)]);
}

#[test]
fn approach_zero_allows_each_side_and_fire_like_footprint_excludes_its_tile() {
    let wc = collision(8, 8, &[]);
    let footprint = Footprint {
        x: 2,
        z: 2,
        level: 0,
        width: 1,
        length: 2,
    };
    let all: Vec<_> = service_access_tiles(
        &wc,
        footprint,
        AccessReach::Adjacent,
        Sides::from_approach(0, 0),
    )
    .collect();
    assert!(all.iter().any(|t| t.x < 2));
    assert!(all.iter().any(|t| t.x > 2));
    assert!(all.iter().any(|t| t.z < 2));
    assert!(all.iter().any(|t| t.z > 3));

    let one_tile = Footprint::tile(tile(4, 4));
    assert!(
        !service_access_tiles(&wc, one_tile, AccessReach::Adjacent, Sides::ALL)
            .any(|access| access == tile(4, 4))
    );
}

#[test]
fn service_access_excludes_a_walled_shared_edge() {
    let wc = collision(8, 8, &[(3, 2, WALL_WEST)]);
    let footprint = Footprint {
        x: 2,
        z: 2,
        level: 0,
        width: 1,
        length: 2,
    };
    let access: Vec<_> =
        service_access_tiles(&wc, footprint, AccessReach::InLine, Sides::ALL).collect();
    assert!(!access.contains(&tile(3, 2)));
    assert!(access.contains(&tile(3, 3)));
}

fn candidate(index: u16, at: WorldTile, access: &[WorldTile]) -> ServiceCandidate<'_> {
    ServiceCandidate {
        kind: ServiceKind::Bank,
        index,
        tile: at,
        width: 1,
        length: 1,
        access,
    }
}

#[test]
fn air_pick_ranks_borrowed_candidates_without_allocating() {
    let near_access = [tile(2, 2)];
    let far_access = [tile(4, 3)];
    let candidates = [
        candidate(1, tile(3, 3), &far_access),
        candidate(2, tile(3, 2), &near_access),
    ];
    let mut result = None;
    let measured = allocation_counter::measure(|| {
        result = nearest_service_air(candidates.iter().copied(), tile(0, 0));
    });
    assert_eq!(measured.count_total, 0);
    let pick = result.expect("air candidate");
    assert_eq!(pick.index, 2);
    assert_eq!(pick.access_tile, tile(2, 2));
}

fn transport_to(from: WorldTile, to: WorldTile) -> TransportGraph {
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: from,
        to,
        loc_id: 1,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(from).or_default().push(0);
    graph.edges.push(edge);
    graph
}

#[test]
fn routed_pick_chooses_cheaper_route_than_air_nearest_candidate() {
    let wc = collision(40, 40, &[]);
    let from = tile(1, 1);
    let near = tile(10, 2);
    let cheap = tile(30, 30);
    let graph = transport_to(from, cheap);
    let targets = [near, cheap];
    let result = nearest_service_routed(
        &wc,
        &graph,
        from,
        &targets,
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        (100, 100),
    );
    assert_eq!(result.route().expect("reachable candidate").dest, cheap);
}

#[test]
fn routed_pick_skips_isolated_keeper_access_in_the_complete_target_set() {
    let blocked = WALK_SCENERY;
    let wc = collision(
        12,
        12,
        &[
            (2, 2, blocked),
            (3, 2, blocked),
            (4, 2, blocked),
            (2, 3, blocked),
            (4, 3, blocked),
            (2, 4, blocked),
            (3, 4, blocked),
            (4, 4, blocked),
        ],
    );
    let isolated = tile(3, 3);
    let reachable = tile(8, 1);
    let result = nearest_service_routed(
        &wc,
        &TransportGraph::default(),
        tile(1, 1),
        &[isolated, reachable],
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        (1000, 1000),
    );
    assert_eq!(
        result
            .route()
            .expect("the other keeper remains reachable")
            .dest,
        reachable,
    );
}

#[test]
fn routed_pick_searches_all_72_targets_and_air_pick_remains_air_nearest() {
    let wc = collision(48, 48, &[]);
    let from = tile(1, 1);
    let mut targets = Vec::with_capacity(72);
    for n in 0..71 {
        targets.push(tile(10 + n % 12, 2 + n / 12));
    }
    let last_by_air = tile(35, 35);
    targets.push(last_by_air);
    let graph = transport_to(from, last_by_air);
    let search = nearest_service_routed(
        &wc,
        &graph,
        from,
        &targets,
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        (1000, 1000),
    );
    assert_eq!(
        search
            .route()
            .expect("the final target is directly routed")
            .dest,
        last_by_air
    );

    let access_near = [targets[0]];
    let access_last = [last_by_air];
    let candidates = [
        candidate(0, targets[0], &access_near),
        candidate(71, last_by_air, &access_last),
    ];
    assert_eq!(
        nearest_service_air(candidates.into_iter(), from)
            .expect("air pick")
            .index,
        0
    );
}

#[test]
fn routed_search_does_not_choose_a_quest_gated_last_target() {
    let wc = collision(48, 48, &[]);
    let from = tile(1, 1);
    let near = tile(10, 2);
    let gated_last = tile(35, 35);
    let mut graph = transport_to(from, gated_last);
    graph.edges[0]
        .quest_req
        .push(String::from("Unfinished quest"));
    let targets = [near, gated_last];
    let search = nearest_service_routed(
        &wc,
        &graph,
        from,
        &targets,
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        (1000, 1000),
    );
    assert_eq!(
        search
            .route()
            .expect("ungated candidate remains reachable")
            .dest,
        near
    );
}

#[test]
fn cooperative_routed_request_owns_one_complete_target_set_with_teleports_off() {
    let targets: Box<[WorldTile]> = (0..72).map(|n| tile(n, 0)).collect();
    let request = routed_request(
        tile(1, 1),
        targets,
        FindOptions {
            allow_teleports: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        },
        Box::default(),
        123,
    );
    match request {
        SearchRequest::First {
            targets,
            fallback,
            options,
            max_settles,
            ..
        } => {
            assert_eq!(targets.len(), 72);
            assert!(fallback.is_empty());
            assert!(!options.allow_teleports);
            assert!(!options.allow_bank_fetch);
            assert_eq!(max_settles, 123);
        }
        _ => panic!("routed request must be a first-goal cooperative search"),
    }
}

#[test]
fn bank_only_binding_does_not_initialize_shop_keeper_catalog() {
    if !api::test_support::game_data_available() {
        return;
    }
    for revision in [
        api::selected::ClientRevision::R274,
        api::selected::ClientRevision::R289,
    ] {
        let data = api::game_data::for_revision(revision).expect("selected content");
        let world = crate::world::NavWorld::from_parts(
            collision(8, 8, &[]),
            TransportGraph::default(),
            Vec::new(),
        );
        world
            .bind_named_bank_facts(&data)
            .expect("bank-only binding");
        assert!(world.named_bank_facts().is_some());
        assert!(world.shop_keeper_facts().is_none());
    }
}

#[test]
fn keeper_placement_cache_is_bound_once_per_world_and_shared_without_per_bot_rows() {
    if !api::test_support::game_data_available() {
        return;
    }
    use std::sync::Arc;

    for revision in [
        api::selected::ClientRevision::R274,
        api::selected::ClientRevision::R289,
    ] {
        let data = api::game_data::for_revision(revision).expect("selected content");
        let world = crate::world::NavWorld::from_parts(
            collision(8, 8, &[]),
            TransportGraph::default(),
            Vec::new(),
        );
        assert!(world.shop_keeper_facts().is_none());
        let mut binding = Ok(());
        let bound = allocation_counter::measure(|| {
            binding = world.bind_shop_keeper_facts(&data);
        });
        binding.expect("bind selected shop facts");
        assert!(
            bound.bytes_max <= 64 * 1024,
            "shared shop catalog and keeper binding exceeded 64 KiB: {bound:?}"
        );
        let first = world.shop_keeper_facts().expect("bound shop keepers");
        let expected: usize = api::shop_facts::keepers(&data)
            .expect("generated shop facts")
            .map(|keeper| {
                data.npc_placements()
                    .expect("selected NPC placements")
                    .rows
                    .iter()
                    .filter(|placement| placement.npc_id == keeper.keeper_id)
                    .count()
            })
            .sum();
        assert!(expected > 0);
        assert_eq!(first.rows().len(), expected);
        // One shared process/profile Arc owns 24 placements, measured 2,720 B (api/src/shop_facts.rs:1065-1082), not per bot.
        assert!(
            std::mem::size_of_val(first.rows())
                + std::mem::size_of::<api::service_pick::ShopKeeperFacts>()
                + 2 * std::mem::size_of::<usize>()
                <= 3 * 1024,
            "shared process/profile keeper rows exceeded 3 KiB"
        );
        // Shared process/profile rows measured 112 B each for 24 placements (api/src/shop_facts.rs:1065-1082), not per bot.
        assert!(
            std::mem::size_of::<api::service_pick::ResolvedShopKeeper>() <= 112,
            "a shared keeper placement row exceeded 112 B"
        );
        let rows = first.rows().as_ptr();
        let measured = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                let shared = Arc::clone(world.shop_keeper_facts().expect("cached"));
                assert!(Arc::ptr_eq(first, &shared));
                assert_eq!(shared.rows().as_ptr(), rows);
            }
        });
        assert_eq!(measured.count_total, 0);
        assert!(
            std::mem::size_of::<std::sync::OnceLock<Arc<api::service_pick::ShopKeeperFacts>>>()
                <= 16,
            "the shared world keeper cache slot grew"
        );
        assert_eq!(
            world.bind_shop_keeper_facts(&data),
            Err(crate::world::ShopKeeperBindError::AlreadyBound)
        );
        println!(
            "A3 keeper cache {revision:?}: rows={} row_bytes={} retained_bytes={} world_slot_bytes={}",
            first.rows().len(),
            std::mem::size_of::<api::service_pick::ResolvedShopKeeper>(),
            std::mem::size_of_val(first.rows())
                + std::mem::size_of::<api::service_pick::ShopKeeperFacts>()
                + 2 * std::mem::size_of::<usize>(),
            std::mem::size_of::<std::sync::OnceLock<Arc<api::service_pick::ShopKeeperFacts>>>(),
        );
    }
}

#[test]
#[ignore = "requires the reviewed 289 reference pack in all three nav pack variables"]
fn reference_pack_routes_harpoon_to_harry_from_catherby_and_gerrant_from_port_sarim() {
    let pack = std::env::var_os("NAV_PACK").expect("set NAV_PACK to the reviewed reference pack");
    assert_eq!(std::env::var_os("WORLD_NAV_PACK").as_ref(), Some(&pack));
    assert_eq!(std::env::var_os("NAV_ARRIVAL_PACK").as_ref(), Some(&pack));
    let bytes = std::fs::read(&pack).expect("read reviewed reference pack");
    let world = crate::world::NavWorld::from_bytes(&bytes).expect("decode reviewed reference pack");
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected 289 content");
    world
        .bind_shop_keeper_facts(&data)
        .expect("bind selected shop placements");
    let facts = world
        .shop_keeper_facts()
        .expect("shared resolved keeper rows");
    let item = data.item_by_alias("harpoon").expect("harpoon").id;
    let candidates: Vec<_> = facts.candidates_for(&data, item, true).collect();
    assert_eq!(candidates.len(), 2, "the pinned harpoon keeper domain");
    let targets: Vec<_> = candidates
        .iter()
        .flat_map(|candidate| candidate.access.iter().copied())
        .collect();
    // Use the account seed from host-play's real Catherby recovery cells,
    // not the first collision-clear square of a region that includes sea.
    let catherby = tile(2840, 3436);
    assert!(world.collision.standable(catherby), "Catherby live seed");
    let port_sarim = tile(3020, 3230);
    assert!(world.collision.standable(port_sarim), "Port Sarim origin");
    let state = WorldState::empty().with_map_members(true);
    for (label, from, keeper_config) in [
        ("Catherby beach", catherby, "harry"),
        ("Port Sarim", port_sarim, "gerrant"),
    ] {
        let started = std::time::Instant::now();
        let result = nearest_service_routed(
            &world.collision,
            &world.graph,
            from,
            &targets,
            FindOptions::default(),
            &state,
            &[],
            (
                crate::router::BANK_TARGET_BUDGET,
                crate::router::NODE_BUDGET,
            ),
        );
        let route = result.route().unwrap_or_else(|error| {
            panic!("no routed vendor from {label} {from:?} to {targets:?}: {error:?}")
        });
        assert_eq!(
            (route.ticks * 2.0).fract(),
            0.0,
            "{label}: a real route is a whole count of half ticks, got {} ticks",
            route.ticks
        );
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.access.contains(&route.dest))
            .expect("chosen access belongs to the complete eligible domain");
        let keeper = &facts.rows()[usize::from(candidate.index)].keeper;
        assert_eq!(keeper.keeper_config, keeper_config, "{label}");
        println!(
            "A3 reference pack: origin={label} from={from:?} keeper={} npc={} access={:?} ticks={} elapsed_us={} complete_targets={}",
            keeper.keeper_config,
            keeper.keeper_id,
            route.dest,
            route.ticks,
            started.elapsed().as_micros(),
            targets.len(),
        );
    }
}
