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

use crate as map_host;
#[path = "../../tests/support/map_fixture.rs"]
mod map_fixture;
use map_fixture::MapFixture;

fn bound_context(play: &crate::Play) -> MapContext {
    MapContext {
        focus: play.map_focus("alice"),
        nav: Digest::from_hex(
            &play
                .server_profile()
                .unwrap()
                .nav_identity()
                .unwrap()
                .nav_sha256,
        )
        .unwrap(),
        overlay: None,
        generation: 1,
    }
}

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

fn offline_play(world: NavWorld) -> crate::Play {
    let mut play = crate::Play::new(&crate::PlayOptions {
        host: "example.invalid".into(),
        port: 43594,
        cache_dir: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/walk-map-empty-cache"
        )
        .into(),
        lowmem: true,
        mainland: false,
    });
    play.world = Some(Arc::new(world));
    play
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
fn debug_authority_requires_local_target_and_loopback_host() {
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
}

#[test]
fn host_rejects_remote_teleport_and_expired_slot_before_queueing() {
    let mut play = offline_play(world(t(3200, 3200, 0), 8, &[]));
    play.focused = Some("alice".into());
    play.arms
        .insert("alice".into(), crate::SlotArm::new(1, false));
    play.cheats
        .lock()
        .unwrap()
        .insert("alice".into(), Default::default());
    play.statuses.lock().unwrap().push(crate::SlotStatus {
        username: "alice".into(),
        ingame: true,
        tile_x: 3201,
        tile_z: 3201,
        tile_level: 0,
        ..Default::default()
    });
    let ctx = MapContext {
        focus: play.map_focus("alice"),
        ..context()
    };
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(play.world.as_deref().unwrap(), t(3203, 3204, 2));
    let command = model
        .confirm(
            ActionKind::Teleport,
            &ctx,
            Some(t(3201, 3201, 0)),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(
        play.map_teleport(command, &ctx),
        Err(ActionError::Unauthorized)
    );
    assert!(play.cheats.lock().unwrap()["alice"].is_empty());
    model.select_tile(play.world.as_deref().unwrap(), t(3203, 3204, 2));
    let command = model
        .confirm(
            ActionKind::Teleport,
            &ctx,
            Some(t(3201, 3201, 0)),
            FindOptions::default(),
        )
        .unwrap();
    // Same username and uid do not preserve the old slot's authority.
    play.arms
        .insert("alice".into(), crate::SlotArm::new(1, false));
    assert_eq!(play.map_teleport(command, &ctx), Err(ActionError::Stale));
    assert!(play.cheats.lock().unwrap()["alice"].is_empty());
}

#[test]
fn local_loopback_teleport_consumes_selection_and_queues_exact_coordinates_once() {
    let fixture = MapFixture::new(&world(t(3200, 3200, 0), 8, &[]), "local-289");
    let origin = t(3201, 3201, 0);
    let play = fixture.play(origin);
    play.cheats
        .lock()
        .unwrap()
        .insert("alice".into(), Default::default());
    let ctx = bound_context(&play);
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(&play.world().unwrap(), t(3203, 3204, 2));
    let command = model
        .confirm(
            ActionKind::Teleport,
            &ctx,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(play.map_teleport(command, &ctx), Ok(()));
    assert_eq!(
        model
            .confirm(
                ActionKind::Teleport,
                &ctx,
                Some(origin),
                FindOptions::default()
            )
            .unwrap_err(),
        ActionError::NoSelection
    );
    assert_eq!(
        play.cheats.lock().unwrap()["alice"]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["tele 2,50,50,3,4"]
    );
}

#[test]
fn local_loopback_teleport_without_spawned_queue_is_no_focus() {
    let fixture = MapFixture::new(&world(t(3200, 3200, 0), 8, &[]), "local-289");
    let origin = t(3201, 3201, 0);
    let play = fixture.play(origin);
    let ctx = bound_context(&play);
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(&play.world().unwrap(), t(3203, 3204, 2));
    let command = model
        .confirm(
            ActionKind::Teleport,
            &ctx,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(play.map_teleport(command, &ctx), Err(ActionError::NoFocus));
    assert!(
        !play.cheats.lock().unwrap().contains_key("alice"),
        "a missing spawn_slot queue must stay missing"
    );
}

#[test]
fn blocked_tile_teleports_to_requested_and_refuses_walk_on_the_same_selection() {
    let fixture = MapFixture::new(&world(t(3200, 3200, 0), 8, &[]), "local-289");
    let origin = t(3201, 3201, 0);
    let requested = t(4000, 4001, 2);
    let play = fixture.play(origin);
    play.cheats
        .lock()
        .unwrap()
        .insert("alice".into(), Default::default());
    let ctx = bound_context(&play);
    let world = play.world().unwrap();
    let mut model = MapModel::default();
    model.bind(ctx);
    assert_eq!(model.select_tile(&world, requested), None);
    let pending = model
        .pending()
        .expect("a miss still selects the requested tile");
    assert_eq!(pending.requested, requested);
    assert_eq!(pending.target, None);
    assert_eq!(
        model.availability(ActionKind::Walk, &ctx, Some(origin)),
        Err(ActionError::Blocked)
    );
    assert!(
        model.pending().is_some(),
        "Walk availability must not consume"
    );
    let command = model
        .confirm(
            ActionKind::Teleport,
            &ctx,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(command.destination(), requested);
    assert_eq!(play.map_teleport(command, &ctx), Ok(()));
    assert_eq!(
        play.cheats.lock().unwrap()["alice"]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [api::interact::tele_args(requested.level, requested.x, requested.z).as_str()]
    );
}

#[test]
fn teleport_uses_requested_tile_when_walk_snaps() {
    let world = world(t(0, 0, 0), 4, &[(1, 1, 0)]);
    let ctx = context();
    let origin = t(0, 0, 0);
    let requested = t(1, 1, 0);
    let mut model = MapModel::default();
    model.bind(ctx);
    let snapped = model
        .select_tile(&world, requested)
        .expect("radius-16 snap");
    assert_ne!(snapped, requested);
    assert_eq!(model.pending().unwrap().requested, requested);
    assert_eq!(model.pending().unwrap().target, Some(snapped));
    assert_eq!(
        model.availability(ActionKind::Walk, &ctx, Some(origin)),
        Ok(snapped)
    );
    assert_eq!(
        model.availability(ActionKind::Teleport, &ctx, Some(origin)),
        Ok(requested)
    );
    let walk = model
        .confirm(ActionKind::Walk, &ctx, Some(origin), FindOptions::default())
        .unwrap();
    assert_eq!(walk.destination(), snapped);
    model.select_tile(&world, requested);
    let teleport = model
        .confirm(
            ActionKind::Teleport,
            &ctx,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(teleport.destination(), requested);
}

#[test]
fn bound_host_walk_arms_and_rejects_a_foreign_nav_without_replacing_the_route() {
    let fixture = MapFixture::new(&world(t(3200, 3200, 0), 8, &[]), "local-289");
    let origin = t(3201, 3201, 0);
    let destination = t(3205, 3206, 0);
    let play = fixture.play(origin);
    let ctx = bound_context(&play);
    let world = play.world().unwrap();
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(&world, destination);
    let command = model
        .confirm(ActionKind::Walk, &ctx, Some(origin), FindOptions::default())
        .unwrap();
    let route = play
        .map_walk(command, &ctx, &WorldState::empty(), &[], &arms)
        .unwrap();
    assert_eq!(route.dest, wt(3205, 3206, 0));
    let generation = {
        let arms = arms.lock().unwrap();
        let arm = arms["alice"].lock().unwrap();
        assert_eq!(arm.queued_tile(), Some(destination));
        arm.route_generation
    };

    // Both caller-supplied contexts agree, so only the host's actual bound
    // identity can reject this command.
    let foreign = MapContext {
        nav: digest(99),
        ..ctx
    };
    model.bind(foreign);
    model.select_tile(&world, t(3202, 3202, 0));
    let command = model
        .confirm(
            ActionKind::Walk,
            &foreign,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(
        play.map_walk(command, &foreign, &WorldState::empty(), &[], &arms)
            .unwrap_err(),
        ActionError::Stale
    );
    let arms = arms.lock().unwrap();
    let arm = arms["alice"].lock().unwrap();
    assert_eq!(arm.queued_tile(), Some(destination));
    assert_eq!(arm.route_generation, generation);
}

#[test]
fn ready_catalogue_retains_searchable_normalized_access_and_rejects_foreign_identity() {
    use nav::map::cache::ReadyCatalogue;
    use nav::map::formats::{CatalogueManifest, PayloadReceipt};

    let world = Arc::new(world(t(0, 0, 0), 8, &[(3, 3, 0)]));
    let fixture = MapFixture::new(&world, "local-289");
    let identity = identity();
    let key = identity.key().unwrap();
    let directory = fixture.root.join(key.0.to_string());
    std::fs::create_dir(&directory).unwrap();
    let document = ClientPois {
        schema: 1,
        identity,
        coverage: coverage(),
        records: Rows::new(vec![record(
            EntityKind::Loc,
            3,
            3,
            SourceSpace::ClientVisual {
                plane: 1,
                link_below: true,
            },
            PoiKind::Bank,
            "Bridge bank",
        )])
        .unwrap(),
    };
    let bytes = document.encode().unwrap();
    let receipt = CatalogueManifest {
        schema: 1,
        identity,
        key,
        record_count: 1,
        payload: PayloadReceipt {
            bytes: bytes.len() as u32,
            sha256: Digest::of(&bytes),
        },
    };
    std::fs::write(directory.join("client-pois.json"), bytes).unwrap();
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let ready = Arc::new(ReadyCatalogue::open(&directory, identity).unwrap());
    assert!(matches!(
        Catalogue::from_ready(
            Arc::clone(&world),
            CatalogueIdentity {
                content: digest(99),
                ..identity
            },
            digest(8),
            Some(Arc::clone(&ready)),
            None,
        ),
        Err(nav::map::MapError::Identity)
    ));
    let catalogue = Catalogue::from_ready(
        Arc::clone(&world),
        identity,
        digest(8),
        Some(Arc::clone(&ready)),
        None,
    )
    .unwrap();
    drop(ready);
    let mut search = Search::default();
    search.update(&catalogue, "BRIDGE BANK").unwrap();
    let entries: Vec<_> = search
        .results()
        .iter()
        .map(|&i| {
            let entry = catalogue.entry(i).unwrap();
            (
                entry.name(),
                entry.meaning(),
                entry.anchor(),
                entry.walk_target(),
            )
        })
        .collect();
    assert_eq!(
        entries,
        [("Bridge bank", Meaning::Access, t(3, 3, 0), Some(t(2, 3, 0)))]
    );
}

#[test]
fn actual_route_projection_follows_live_script_manual_precedence_and_focus() {
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
    let play = offline_play(world);
    play.navs.lock().unwrap().insert(
        "alice".into(),
        crate::script_runtime::NavBot {
            route: Some(route.clone()),
            map_route_generation: 7,
            ..Default::default()
        },
    );
    let mut manual = crate::WalkArm::default();
    let read = |projection: Option<RouteProjection<'_>>| {
        projection.map(|p| (p.stamp.source, p.stamp.generation, p.route.dest))
    };
    assert_eq!(
        play.with_map_route("alice", Some(&manual), None, read),
        Some((RouteSource::Script, 7, wt(3, 3, 0)))
    );
    assert_eq!(play.with_map_route("bob", Some(&manual), None, read), None);
    manual.route = Some(nav::router::Route {
        dest: wt(2, 3, 0),
        ..route.clone()
    });
    manual.route_generation = 8;
    assert_eq!(
        play.with_map_route("alice", Some(&manual), None, read),
        Some((RouteSource::Script, 7, wt(3, 3, 0)))
    );
    let live = nav::router::Route {
        dest: wt(4, 3, 0),
        ..route
    };
    assert_eq!(
        play.with_map_route(
            "alice",
            Some(&manual),
            Some(RouteProjection::live(&live, 9, None)),
            read
        ),
        Some((RouteSource::Live, 9, wt(4, 3, 0)))
    );
    play.navs.lock().unwrap().remove("alice");
    assert_eq!(
        play.with_map_route("alice", Some(&manual), None, read),
        Some((RouteSource::Manual, 8, wt(2, 3, 0)))
    );
    manual.route = None;
    assert_eq!(
        play.with_map_route("alice", Some(&manual), None, read),
        None
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

#[test]
fn observed_services_expire_with_snapshot_or_focus_and_keep_real_plane_and_slot() {
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig, ClientNpc};
    use client::config::{Cache, NpcType};
    use client::dash3d::ClientPlayer;
    use client::io::ClientRevision;

    let cache = Cache {
        npcs: vec![NpcType {
            name: "Banker".into(),
            op: vec![Some("Talk-to".into()), None, Some("Bank".into())],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut client = Client::from_shared_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/walk-map-empty-cache"
            )
            .into(),
            members: true,
            lowmem: true,
        },
        Arc::new(cache),
        Arc::new(vec![]),
        Arc::new(vec![]),
        ClientRevision::R289,
    );
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.minusedlevel = 1;
    client.local_player = Some(ClientPlayer::at(20, 12));
    for i in 0..=MAX_OBSERVED_SERVICES {
        let mut npc = ClientNpc {
            r#type: Some(0),
            ..Default::default()
        };
        npc.entity.x = 20 * 128 + 64;
        npc.entity.z = 13 * 128 + 64;
        client.npc[i + 7] = Some(Box::new(npc));
        client.npc_ids[i] = (i + 7) as i32;
    }
    client.npc_count = (MAX_OBSERVED_SERVICES + 1) as i32;
    client.gens.npc = 1;
    client.gens.player = 1;
    client.gens.scene = 1;
    client.gens.session = 1;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    let ctx = context();
    let observed = observed_services(&snapshot, ctx, ctx).unwrap();
    assert_eq!(observed.len(), MAX_OBSERVED_SERVICES);
    assert_eq!(observed.first().unwrap().npc_index, 7);
    assert_eq!(
        observed.last().unwrap().npc_index,
        MAX_OBSERVED_SERVICES + 6
    );
    for p in observed {
        assert_eq!(
            (p.kind, p.tile, p.evidence, p.context),
            (
                PoiKind::Bank,
                t(3220, 3213, 1),
                CapabilityEvidence::ClientOperation {
                    capability: Capability::Bank,
                    slot: OperationSlot::new(3).unwrap(),
                },
                ctx
            )
        );
    }
    let replaced = context();
    assert!(matches!(
        observed_services(&snapshot, ctx, replaced),
        Err(ActionError::Stale)
    ));
    snapshot.reset_session(client.gens);
    assert!(matches!(
        observed_services(&snapshot, ctx, ctx),
        Err(ActionError::NoOrigin)
    ));
}

#[test]
fn selected_spell_names_match_only_bound_spell_landings() {
    let mut world = world(t(0, 0, 0), 12, &[]);
    let mut spell = edge(TransportKind::Teleport, wt(0, 0, 0), wt(8, 9, 0));
    spell.loc_id = 0;
    world.graph.teleports = vec![
        spell,
        edge(TransportKind::Teleport, wt(0, 0, 0), wt(9, 9, 0)),
    ];
    let data =
        |content: Digest| -> Arc<api::game_data::SelectedGameData> {
            Arc::new(serde_json::from_value(serde_json::json!({
            "schema_version": 4, "revision": 289,
            "provenance": {
                "cache_identity": { "cache_id": "fixture", "content_id": content.to_string() },
                "inputs": [], "content_inputs": [], "decoder_sources": []
            },
            "items": [], "consumption": [], "pickpocket": [],
            "teleports": [{
                "name": "Harbour teleport", "source_row": "harbour", "spell": "harbour",
                "component_id": 42, "members": false, "level": 1, "runes": [],
                "experience": 1, "tele_coord": "0_0_0_8_9", "x": 8, "z": 9, "plane": 0
            }, {
                "name": "Unrelated spell", "source_row": "elsewhere", "spell": "elsewhere",
                "component_id": 43, "members": false, "level": 1, "runes": [],
                "experience": 1, "tele_coord": "0_0_0_9_9", "x": 9, "z": 9, "plane": 0
            }]
        })).unwrap())
        };
    let world = Arc::new(world);
    assert!(matches!(
        Catalogue::new(Arc::clone(&world), identity(), digest(8), None, None)
            .unwrap()
            .with_game_data(data(digest(9))),
        Err(nav::map::MapError::Identity)
    ));
    let catalogue = Catalogue::new(world, identity(), digest(8), None, None)
        .unwrap()
        .with_game_data(data(identity().content))
        .unwrap();
    let mut search = Search::default();
    search.update(&catalogue, "HARBOUR").unwrap();
    let names: Vec<_> = search
        .results()
        .iter()
        .map(|&i| {
            let e = catalogue.entry(i).unwrap();
            (e.name(), e.anchor(), e.meaning())
        })
        .collect();
    assert_eq!(
        names,
        vec![("Harbour teleport", t(8, 9, 0), Meaning::TeleportLanding)]
    );
    search.update(&catalogue, "Unrelated spell").unwrap();
    assert!(
        search.results().is_empty(),
        "an item/NPC transport is not a spell with the same landing"
    );
}

#[test]
fn replacing_a_manual_arm_cannot_reuse_a_cached_route_stamp() {
    let world = world(t(0, 0, 0), 5, &[]);
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let arm_route = |from| {
        crate::arm_walk_on(
            &world,
            from,
            t(3, 3, 0),
            FindOptions::default(),
            &WorldState::empty(),
            &[],
            &arms,
            Some("alice"),
        )
        .unwrap();
        let all = arms.lock().unwrap();
        let arm = all["alice"].lock().unwrap();
        RouteProjection::manual(&arm).unwrap().stamp
    };
    let first = arm_route(t(0, 0, 0));
    arms.lock().unwrap().clear();
    let replacement = arm_route(t(4, 0, 0));
    assert_ne!(
        first, replacement,
        "same destination but a different route after arm replacement"
    );
}
