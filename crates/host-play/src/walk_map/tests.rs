use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use nav::collision::{pack_walk, WorldCollision};
use nav::map::formats::{ClientPois, Coverage, CoverageLevel, ServiceIdentity, ServicePois};
use nav::map::identity::{CatalogueIdentity, Digest};
use nav::map::poi::*;
use nav::map::spatial::GameTile;
use nav::map::{Rows, Text};
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::transport::{TransportEdge, TransportGraph, TransportKind};
use nav::world::NavWorld;
use nav::WorldState;

use super::*;

fn t(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}
fn wt(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}
fn digest(n: u8) -> Digest {
    Digest([n; 32])
}
fn identity() -> CatalogueIdentity {
    CatalogueIdentity {
        revision: 289,
        content: digest(1),
        policy: digest(2),
    }
}
fn coverage() -> Coverage {
    Coverage {
        npc_placements: CoverageLevel::Unavailable,
        bank_services: CoverageLevel::Limited,
        place_labels: CoverageLevel::Unavailable,
        unresolved: Rows::new(vec![]).unwrap(),
    }
}
fn world(origin: Tile, size: usize, blocks: &[(usize, usize, usize)]) -> NavWorld {
    let mut flags = vec![0u32; 4 * size * size];
    for &(x, z, plane) in blocks {
        flags[plane * size * size + z * size + x] =
            client::dash3d::CollisionFlag::SQ_BLOCKED as u32;
    }
    let (walk, blocked) = pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: wt(origin.x, origin.z, origin.level),
            width: size,
            height: size,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![],
    )
}
fn record(
    entity: EntityKind,
    x: i32,
    z: i32,
    source: SourceSpace,
    kind: PoiKind,
    name: &str,
) -> PoiRecord {
    let plane = source.game_plane().unwrap().unwrap();
    PoiRecord {
        key: PoiKey {
            entity,
            id: 2213,
            x,
            z,
            source,
            shape: if entity == EntityKind::Loc { 10 } else { 0 },
            rotation: 0,
        },
        name: Text::new(name).unwrap(),
        kind,
        effective_plane: plane,
        footprint: Footprint {
            width: 1,
            length: 1,
        },
        display: DisplayAnchor {
            x: f64::from(x) + 0.5,
            z: f64::from(z) + 0.5,
            plane,
        },
        evidence: Rows::new(vec![CapabilityEvidence::ActiveQuickBooth]).unwrap(),
        walk_target: None,
    }
}
fn services(mut records: Vec<PoiRecord>, nav: Digest) -> AuthenticatedServices {
    records.sort_by_key(|r| r.key);
    let doc = ServicePois {
        schema: 1,
        identity: ServiceIdentity {
            revision: 289,
            content: digest(1),
            nav_sha256: nav,
            source_sha256: digest(3),
            generator_sha256: digest(4),
            policy: digest(5),
        },
        coverage: coverage(),
        records: Rows::new(records).unwrap(),
    };
    let bytes = doc.encode_navpois().unwrap();
    AuthenticatedServices::decode(&bytes, doc.identity, Digest::of(&bytes)).unwrap()
}
fn context() -> MapContext {
    MapContext {
        focus: Some(FocusToken::capture(&crate::SlotArm::new(1, false))),
        nav: digest(8),
        overlay: None,
        generation: 1,
    }
}
fn edge(kind: TransportKind, at: WorldTile, to: WorldTile) -> TransportEdge {
    TransportEdge {
        kind,
        at,
        to,
        loc_id: 100,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    }
}

#[test]
fn merged_bridge_access_annotation_and_label_remain_distinct() {
    let world = Arc::new(world(
        t(3508, 3476, 0),
        12,
        &[(5, 3, 0), (3, 3, 0), (2, 2, 0)],
    ));
    let booth = record(
        EntityKind::Loc,
        3513,
        3479,
        SourceSpace::ClientVisual {
            plane: 1,
            link_below: true,
        },
        PoiKind::Bank,
        "Bank booth",
    );
    let marker = record(
        EntityKind::MapFunction,
        3511,
        3479,
        SourceSpace::ClientVisual {
            plane: 1,
            link_below: true,
        },
        PoiKind::Bank,
        "Bank symbol",
    );
    let mut supplement = booth.clone();
    supplement.evidence = Rows::new(vec![CapabilityEvidence::SourceService {
        capability: Capability::Bank,
        trigger: ServiceTrigger::Operation {
            slot: OperationSlot::new(2).unwrap(),
        },
        eligibility: Eligibility::Conditional,
        reference: Text::new("scripts/bank:oploc2").unwrap(),
        source_sha256: digest(3),
    }])
    .unwrap();
    let mut npc = record(
        EntityKind::Npc,
        3514,
        3479,
        SourceSpace::ServerGame { plane: 0 },
        PoiKind::Bank,
        "Banker",
    );
    npc.key.id = 1036;
    let label = record(
        EntityKind::Label,
        3510,
        3478,
        SourceSpace::ServerGame { plane: 0 },
        PoiKind::Label { priority: 1 },
        "Canifis/Bank",
    );
    let client = Arc::new(ClientPois {
        schema: 1,
        identity: identity(),
        coverage: coverage(),
        records: Rows::new(vec![booth, marker]).unwrap(),
    });
    let catalogue = Catalogue::new(
        world,
        identity(),
        digest(8),
        Some(client),
        Some(services(vec![supplement, npc, label], digest(8))),
    )
    .unwrap();
    let access = catalogue
        .entries()
        .find(|e| e.name() == "Bank booth")
        .unwrap();
    assert_eq!(access.records().count(), 2);
    assert_eq!(access.anchor(), t(3513, 3479, 0));
    assert_eq!(access.walk_target(), Some(t(3512, 3479, 0)));
    assert_eq!(access.eligibility(), Eligibility::Conditional);
    assert!(!access.approach_known());
    assert_eq!(
        catalogue
            .entries()
            .find(|e| e.name() == "Banker")
            .unwrap()
            .anchor()
            .level,
        0
    );
    let annotation = catalogue
        .entries()
        .find(|e| e.meaning() == Meaning::Annotation)
        .unwrap();
    assert_eq!(annotation.walk_target(), None);
    let mut search = Search::default();
    search.update(&catalogue, "CANIFIS bank").unwrap();
    let result = catalogue.entry(search.results()[0]).unwrap();
    assert_eq!(result.meaning(), Meaning::PlaceLabel);
    let mut model = MapModel::default();
    let mut ctx = context();
    ctx.overlay = Some(catalogue.key());
    model.bind(ctx);
    model.select_poi(&catalogue, result.index()).unwrap();
    assert_eq!(
        model.pending().unwrap().target,
        None,
        "blocked label cannot snap to a nearby stand"
    );
}

#[test]
fn wrong_service_binding_rejects_and_missing_sources_do_not_invent_banks() {
    let world = Arc::new(world(t(0, 0, 0), 5, &[]));
    assert!(Catalogue::new(
        Arc::clone(&world),
        identity(),
        digest(8),
        None,
        Some(services(vec![], digest(9)))
    )
    .is_err());
    let catalogue = Catalogue::new(world, identity(), digest(8), None, None).unwrap();
    assert_eq!(catalogue.service_status(), SourceStatus::Unavailable);
    assert_eq!(catalogue.client_status(), SourceStatus::Unavailable);
    assert!(catalogue
        .entries()
        .all(|e| !matches!(e.kind(), PoiKind::Bank)));
}

#[test]
fn teleport_pins_use_landings_and_transport_directions_share_access_pin() {
    let mut world = world(t(0, 0, 0), 12, &[]);
    world.graph.edges = vec![
        edge(TransportKind::Ladder, wt(3, 4, 0), wt(3, 4, 1)),
        edge(TransportKind::Ladder, wt(3, 4, 0), wt(4, 4, 1)),
    ];
    world.graph.teleports = vec![
        edge(TransportKind::Teleport, wt(0, 0, 0), wt(8, 9, 0)),
        edge(TransportKind::Teleport, wt(0, 0, 0), wt(8, 9, 0)),
    ];
    let catalogue = Catalogue::new(Arc::new(world), identity(), digest(8), None, None).unwrap();
    let transport = catalogue
        .entries()
        .find(|e| e.meaning() == Meaning::Transport)
        .unwrap();
    assert_eq!(transport.anchor(), t(3, 4, 0));
    assert_eq!(transport.transports().count(), 2);
    let landing = catalogue
        .entries()
        .find(|e| e.meaning() == Meaning::TeleportLanding)
        .unwrap();
    assert_eq!(landing.anchor(), t(8, 9, 0));
    assert_eq!(landing.transports().count(), 2);
    assert!(catalogue.entries().all(|e| e.anchor() != t(0, 0, 0)));
}

#[test]
fn radius_snap_miss_extremes_and_plane_changes_cannot_arm() {
    let world = world(t(0, 0, 0), 4, &[(1, 1, 0)]);
    let mut model = MapModel::default();
    let ctx = context();
    model.bind(ctx);
    assert_eq!(model.select_tile(&world, t(1, 1, 0)), Some(t(0, 1, 0)));
    assert_eq!(model.select_tile(&world, t(20, 0, 0)), None);
    assert_eq!(
        model
            .confirm(
                ActionKind::Walk,
                &ctx,
                Some(t(0, 0, 0)),
                FindOptions::default()
            )
            .unwrap_err(),
        ActionError::Blocked
    );
    assert!(model.pending().is_none());
    assert_eq!(model.select_tile(&world, t(i32::MIN, i32::MAX, 0)), None);
    model.select_tile(&world, t(1, 2, 0));
    model.set_plane(1);
    assert!(model.pending().is_none());
}

#[test]
fn confirmations_capture_once_and_expire_on_focus_identity_or_origin_loss() {
    let world = world(t(0, 0, 0), 5, &[]);
    let ctx = context();
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(&world, t(2, 2, 0));
    assert_eq!(
        model
            .confirm(ActionKind::Walk, &ctx, None, FindOptions::default())
            .unwrap_err(),
        ActionError::NoOrigin
    );
    assert!(model.pending().is_none());
    for changed in [
        MapContext {
            focus: context().focus,
            ..ctx
        },
        MapContext {
            overlay: Some(digest(4)),
            ..ctx
        },
        MapContext {
            generation: 2,
            ..ctx
        },
    ] {
        model.select_tile(&world, t(2, 2, 0));
        assert_eq!(
            model
                .confirm(
                    ActionKind::Walk,
                    &changed,
                    Some(t(0, 0, 0)),
                    FindOptions::default()
                )
                .unwrap_err(),
            ActionError::Stale
        );
    }
    model.select_tile(&world, t(2, 2, 0));
    let opts = FindOptions {
        allow_teleports: true,
        allow_bank_fetch: true,
        ..Default::default()
    };
    let command = model
        .confirm(ActionKind::Walk, &ctx, Some(t(0, 0, 0)), opts)
        .unwrap();
    assert_eq!(command.options(), opts);
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let route = command
        .walk_on(&world, &ctx, "alice", &WorldState::empty(), &[], &arms)
        .unwrap();
    assert_eq!(route.dest, wt(2, 2, 0));
    assert_eq!(
        arms.lock().unwrap()["alice"].lock().unwrap().queued_tile(),
        Some(t(2, 2, 0))
    );
    assert_eq!(
        model
            .confirm(ActionKind::Walk, &ctx, Some(t(0, 0, 0)), opts)
            .unwrap_err(),
        ActionError::NoSelection
    );
    model.select_tile(&world, t(2, 2, 1));
    let command = model
        .confirm(
            ActionKind::Walk,
            &ctx,
            Some(t(0, 0, 0)),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(
        command
            .walk_on(&world, &ctx, "alice", &WorldState::empty(), &[], &arms)
            .unwrap_err(),
        ActionError::NoPath
    );
}

#[test]
fn debug_authority_checks_target_and_actual_host_and_encodes_plane() {
    use client::BotTarget;
    assert_eq!(
        super::actions::debug_authorized(BotTarget::Prod, "localhost"),
        Err(ActionError::Unauthorized)
    );
    assert_eq!(
        super::actions::debug_authorized(BotTarget::Local, "192.168.1.2"),
        Err(ActionError::Unauthorized)
    );
    assert_eq!(
        super::actions::debug_authorized(BotTarget::Local, "127.0.0.1"),
        Ok(())
    );
    assert_eq!(
        super::actions::teleport_command(t(3253, 3266, 2)).unwrap(),
        "tele 2,50,51,53,2"
    );
}

#[test]
fn borrowed_script_route_is_visible_when_manual_idle_and_live_has_precedence() {
    let world = world(t(0, 0, 0), 5, &[]);
    let route = nav::router::find_with(
        &world.collision,
        &world.graph,
        wt(0, 0, 0),
        wt(3, 3, 0),
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap();
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "alice".into(),
        crate::script_runtime::NavBot {
            route: Some(route.clone()),
            map_route_generation: 7,
            ..Default::default()
        },
    )])));
    let paint = crate::ScriptNavPaint { navs };
    paint.with_map_route("alice", |p| {
        let p = p.unwrap();
        assert_eq!(p.stamp.source, RouteSource::Script);
        assert_eq!(p.stamp.generation, 7);
        assert_eq!(p.route.dest, wt(3, 3, 0));
    });
    assert_eq!(
        RouteProjection::live(&route, 9, None).stamp.source,
        RouteSource::Live
    );
}

#[test]
fn validated_stand_is_never_shifted_twice() {
    let mut r = record(
        EntityKind::Loc,
        3,
        3,
        SourceSpace::ClientVisual {
            plane: 1,
            link_below: true,
        },
        PoiKind::Bank,
        "Booth",
    );
    r.walk_target = Some(WalkTarget {
        tile: GameTile {
            x: 2,
            z: 3,
            plane: 0,
        },
        nav_sha256: digest(8),
    });
    let catalogue = Catalogue::new(
        Arc::new(world(t(0, 0, 0), 8, &[(3, 3, 0)])),
        identity(),
        digest(8),
        None,
        Some(services(vec![r], digest(8))),
    )
    .unwrap();
    let e = catalogue.entries().next().unwrap();
    assert_eq!(e.walk_target(), Some(t(2, 3, 0)));
    assert!(e.approach_known());
}
