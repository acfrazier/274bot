use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use nav_planner::bank_fetch::BankRows;
use nav_planner::collision::{pack_walk, WorldCollision};
use nav_planner::contract::PackId;
use nav_planner::map::formats::{
    ClientPois, Coverage, CoverageLevel, ServicePois, NAVPOIS_VERSION,
};
use nav_planner::map::identity::{CatalogueIdentity, Digest};
use nav_planner::map::poi::*;
use nav_planner::map::spatial::GameTile;
use nav_planner::map::{Rows, Text};
use nav_planner::router::FindOptions;
use nav_planner::tile::Tile;
use nav_planner::transport::{TransportEdge, TransportGraph, TransportKind};
use nav_planner::world::NavWorld;
use nav_planner::WorldState;

use super::*;

use crate as map_host;
#[path = "../../tests/support/map_fixture.rs"]
mod map_fixture;
use map_fixture::MapFixture;

fn bound_context(play: &crate::Play) -> MapContext {
    MapContext {
        focus: play.map_focus("alice"),
        nav: play.server_profile().unwrap().nav_pack_id().unwrap(),
        overlay: None,
        generation: 1,
    }
}

fn routing_test_admission() -> crate::admission::Admission {
    crate::admission::Admission::manual(
        FindOptions::default(),
        Default::default(),
        0,
        crate::WalkGlobals::default(),
    )
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
fn install_zone_wall(world: &mut NavWorld) {
    let zones = (0..8)
        .map(|z| {
            nav_planner::zones::Zone::npc(
                wt(3203, 3200 + z, 0),
                0,
                nav_planner::zones::ZoneClass::Always,
                u16::MAX,
                0,
            )
        })
        .collect();
    let kinds = vec![nav_planner::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    world.graph.zones = Some(
        nav_planner::zones::ZoneTable::from_parts(
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
        .unwrap(),
    );
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
fn services(mut records: Vec<PoiRecord>) -> Arc<ServicePois> {
    records.sort_by_key(|r| r.key);
    let doc = ServicePois {
        schema: NAVPOIS_VERSION,
        coverage: coverage(),
        records: Rows::new(records).unwrap(),
    };
    let bytes = doc.encode_navpois().unwrap();
    Arc::new(ServicePois::decode_navpois(&bytes).unwrap())
}
fn pack_id(world: &NavWorld) -> PackId {
    nav_planner::producer::identity(world)
        .expect("fixture world has an ownership token")
        .pack()
}
fn context(world: &NavWorld) -> MapContext {
    MapContext {
        focus: Some(FocusToken::capture(&crate::SlotArm::new(1, false))),
        nav: pack_id(world),
        overlay: None,
        generation: 1,
    }
}
fn edge(kind: TransportKind, at: WorldTile, to: WorldTile) -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind,
        player_delta: None,
        at,
        to,
        loc_id: 100,
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
    }
}

fn offline_play(world: NavWorld) -> crate::Play {
    let mut play = crate::Play::new(&crate::PlayOptions {
        host: "example.invalid".into(),
        transport: client::Transport::Tcp,
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

fn push_slot(
    play: &mut crate::Play,
    name: &str,
    origin: Tile,
    world: Option<u16>,
    connected: bool,
    ingame: bool,
) {
    play.attach_arm(name, crate::SlotArm::new(1, false));
    play.statuses.lock().unwrap().push(crate::SlotStatus {
        username: name.into(),
        world,
        connected,
        ingame,
        tile_x: origin.x,
        tile_z: origin.z,
        tile_level: origin.level,
        ..Default::default()
    });
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
        Some(client),
        Some(services(vec![supplement, npc, label])),
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
    assert_eq!(result.name(), "Canifis/Bank");
    assert_eq!(result.display_name().to_string(), "Canifis Bank");
    let mut model = MapModel::default();
    let mut ctx = context(catalogue.world());
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
fn slash_place_labels_display_as_spaces_and_remain_searchable() {
    let world = Arc::new(world(t(0, 0, 0), 8, &[]));
    let label = record(
        EntityKind::Label,
        4,
        4,
        SourceSpace::ServerGame { plane: 0 },
        PoiKind::Label { priority: 2 },
        "Port/Sarim",
    );
    let catalogue = Catalogue::new(world, identity(), None, Some(services(vec![label]))).unwrap();
    let entry = catalogue
        .entries()
        .find(|e| e.name() == "Port/Sarim")
        .unwrap();
    assert_eq!(DisplayName::new("Port/Sarim").to_string(), "Port Sarim");
    assert_eq!(DisplayName::new("Lumbridge").to_string(), "Lumbridge");
    assert_eq!(entry.display_name().to_string(), "Port Sarim");
    let mut buf = String::with_capacity(32);
    let cap = buf.capacity();
    display_name_into("Port/Sarim", &mut buf);
    assert_eq!(buf, "Port Sarim");
    assert_eq!(
        buf.capacity(),
        cap,
        "display_name_into must write into a reserved buffer without growing it"
    );
    buf.clear();
    display_name_into("Port/Sarim", &mut buf);
    assert_eq!(buf.capacity(), cap);
    let mut search = Search::default();
    search.update(&catalogue, "port sarim").unwrap();
    assert_eq!(
        catalogue.entry(search.results()[0]).unwrap().name(),
        "Port/Sarim"
    );
    search.update(&catalogue, "Port/Sarim").unwrap();
    assert_eq!(
        catalogue.entry(search.results()[0]).unwrap().name(),
        "Port/Sarim"
    );
}

#[test]
fn missing_sources_do_not_invent_banks() {
    let world = Arc::new(world(t(0, 0, 0), 5, &[]));
    let with_services =
        Catalogue::new(Arc::clone(&world), identity(), None, Some(services(vec![]))).unwrap();
    assert_eq!(with_services.service_status(), SourceStatus::Available);
    assert_eq!(with_services.client_status(), SourceStatus::Unavailable);
    assert!(with_services
        .entries()
        .all(|e| !matches!(e.kind(), PoiKind::Bank)));

    let catalogue = Catalogue::new(world, identity(), None, None).unwrap();
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
    let catalogue = Catalogue::new(Arc::new(world), identity(), None, None).unwrap();
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
    let ctx = context(&world);
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
fn confirmations_capture_once_and_expire_on_destination_binding_or_origin_loss() {
    let world = world(t(0, 0, 0), 5, &[]);
    let ctx = context(&world);
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
    let other_focus = MapContext {
        focus: context(&world).focus,
        ..ctx
    };
    let switched = model
        .confirm(
            ActionKind::Walk,
            &other_focus,
            Some(t(0, 0, 0)),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(switched.destination(), t(2, 2, 0));
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
        .walk_on(
            &world,
            &ctx,
            "alice",
            &WorldState::empty(),
            &BankRows::default(),
            routing_test_admission(),
            &arms,
        )
        .unwrap();
    assert_eq!(route.route.dest, wt(2, 2, 0));
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
            .walk_on(
                &world,
                &ctx,
                "alice",
                &WorldState::empty(),
                &BankRows::default(),
                routing_test_admission(),
                &arms
            )
            .unwrap_err(),
        ActionError::NoPath
    );
}

#[test]
fn present_blocked_origin_is_not_a_missing_player_and_does_not_arm_walk() {
    let world = world(t(3218, 3218, 0), 5, &[(2, 2, 0)]);
    let ctx = context(&world);
    let mut model = MapModel::default();
    model.bind(ctx);
    let dest = t(3222, 3222, 0);
    model.select_tile(&world, dest);
    let command = model
        .confirm(
            ActionKind::Walk,
            &ctx,
            Some(t(3220, 3220, 0)),
            FindOptions::default(),
        )
        .unwrap();
    let arms = crate::WalkArms::default();
    let error = command
        .walk_on(
            &world,
            &ctx,
            "alice",
            &WorldState::empty(),
            &BankRows::default(),
            routing_test_admission(),
            &arms,
        )
        .unwrap_err();
    assert_eq!(error, ActionError::OriginNotStandable);
    assert!(
        arms.lock().expect("walk arms").is_empty(),
        "refusal must not queue a route"
    );
    model.select_tile(&world, dest);
    assert_eq!(
        model
            .confirm(ActionKind::Walk, &ctx, None, FindOptions::default())
            .unwrap_err(),
        ActionError::NoOrigin,
        "a genuinely missing player keeps the missing-origin reason"
    );
    model.select_tile(&world, dest);
    assert_eq!(
        model
            .confirm(
                ActionKind::Walk,
                &ctx,
                Some(t(3220, 3220, 4)),
                FindOptions::default()
            )
            .unwrap_err(),
        ActionError::InvalidCoordinates,
        "a present invalid plane is not a missing player"
    );
}

#[test]
fn walk_failure_names_membership_when_a_members_only_route_exists() {
    let base = world(t(0, 0, 0), 2, &[]);
    let from = wt(0, 0, 0);
    let to = wt(0, 0, 1);
    let mut graph = TransportGraph::default();
    graph.edges.push(TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Boat,
        player_delta: None,
        at: from,
        to,
        loc_id: 657,
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
        members_req: true,
        wildy_cap: None,
        quest_gates: None,
    });
    graph.at.insert(from, vec![0]);
    let nav = NavWorld::from_parts(base.collision, graph, vec![]);
    let context = context(&nav);
    let mut model = MapModel::default();
    model.bind(context);
    assert_eq!(model.select_tile(&nav, t(0, 0, 1)), Some(t(0, 0, 1)));
    let command = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(t(0, 0, 0)),
            FindOptions::default(),
        )
        .expect("selected members-only destination");
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let error = command
        .walk_on(
            &nav,
            &context,
            "alice",
            &WorldState::empty(),
            &BankRows::default(),
            routing_test_admission(),
            &arms,
        )
        .expect_err("F2P must refuse the members-only route");
    assert_eq!(error, ActionError::MembersOnly);
    assert_eq!(error.to_string(), "This route requires a members' world");
}

#[test]
fn debug_authority_requires_local_profile_and_loopback_host() {
    use crate::ProfileClass;
    assert!(!super::debug_teleport_authorized(
        ProfileClass::Remote,
        "localhost"
    ));
    assert!(!super::debug_teleport_authorized(
        ProfileClass::Local,
        "192.168.1.2"
    ));
    assert!(super::debug_teleport_authorized(
        ProfileClass::Local,
        "127.0.0.1"
    ));
    assert_eq!(
        super::actions::debug_authorized(ProfileClass::Remote, "localhost"),
        Err(ActionError::Unauthorized)
    );
    assert_eq!(
        super::actions::debug_authorized(ProfileClass::Local, "192.168.1.2"),
        Err(ActionError::Unauthorized)
    );
    assert_eq!(
        super::actions::debug_authorized(ProfileClass::Local, "127.0.0.1"),
        Ok(())
    );
}

#[test]
fn play_map_teleport_authorized_follows_bound_target_and_host() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = world(t(3200, 3200, 0), 8, &[]);
    let local = MapFixture::new(&world, "local-289");
    assert!(local.play(t(3200, 3200, 0)).map_teleport_authorized());
    let prod = MapFixture::new(&world, "public-289");
    assert!(!prod.play(t(3200, 3200, 0)).map_teleport_authorized());
    assert!(!offline_play(world).map_teleport_authorized());
}

#[test]
fn host_rejects_remote_teleport_before_queueing() {
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
        ..context(play.world.as_deref().unwrap())
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
    let ctx = context(&world);
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
fn bound_host_walk_arms_and_rejects_a_foreign_world_pack_without_replacing_the_route() {
    let fixture = MapFixture::new(&world(t(3200, 3200, 0), 8, &[]), "local-289");
    let origin = t(3201, 3201, 0);
    let destination = t(3205, 3206, 0);
    let play = fixture.play(origin);
    let ctx = bound_context(&play);
    let loaded_world = play.world().unwrap();
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(&loaded_world, destination);
    let command = model
        .confirm(ActionKind::Walk, &ctx, Some(origin), FindOptions::default())
        .unwrap();
    let snapshot = api::snapshot::GameSnapshot::new();
    let WalkActionOutcome::Walking(route) = play
        .map_walk(
            command,
            &ctx,
            &snapshot,
            &WorldState::empty(),
            &BankRows::default(),
            Default::default(),
            &arms,
        )
        .unwrap()
    else {
        panic!("manual route should be armed directly");
    };
    assert_eq!(route.route.dest, wt(3205, 3206, 0));
    let generation = {
        let arms = arms.lock().unwrap();
        let arm = arms["alice"].lock().unwrap();
        assert_eq!(arm.queued_tile(), Some(destination));
        arm.route_generation
    };

    // The context and selected tile both come from a distinct world with
    // identical geometry. The host must reject this genuine foreign-pack
    // command without replacing the existing route.
    let foreign_world = world(t(3200, 3200, 0), 8, &[]);
    let foreign = MapContext {
        nav: pack_id(&foreign_world),
        ..ctx
    };
    assert_ne!(foreign.nav, ctx.nav);
    model.bind(foreign);
    model.select_tile(&foreign_world, t(3202, 3202, 0));
    let command = model
        .confirm(
            ActionKind::Walk,
            &foreign,
            Some(origin),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(
        play.map_walk(
            command,
            &foreign,
            &snapshot,
            &WorldState::empty(),
            &BankRows::default(),
            Default::default(),
            &arms
        )
        .unwrap_err(),
        ActionError::Stale
    );
    let arms = arms.lock().unwrap();
    let arm = arms["alice"].lock().unwrap();
    assert_eq!(arm.queued_tile(), Some(destination));
    assert_eq!(arm.route_generation, generation);
}

#[test]
fn ready_catalogue_retains_searchable_normalized_access() {
    use crate::map_cache::{
        fixture::CountingMapProducer, MapCacheRoot, MapDemand, MapDemandManager, MapJobStatus,
        MapProfileDescriptor,
    };
    use nav_planner::map::formats::{CatalogueManifest, PayloadReceipt};

    let world = Arc::new(world(t(0, 0, 0), 8, &[(3, 3, 0)]));
    let fixture = MapFixture::new(&world, "local-289");
    let identity = identity();
    let key = identity.key().unwrap();
    let cache_root = MapCacheRoot::from_root(fixture.root.join("map-cache"));
    let directory = cache_root.catalogue_dir(identity.revision);
    std::fs::create_dir_all(&directory).unwrap();
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
    let descriptor = MapProfileDescriptor::new(
        identity.revision,
        identity.content,
        digest(4),
        identity.policy,
    )
    .unwrap();
    let manager = MapDemandManager::new(cache_root, Arc::new(CountingMapProducer::default()));
    let handle = manager
        .request(descriptor, MapDemand::CatalogueOnly)
        .unwrap();
    let hang_guard = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match handle.status() {
            MapJobStatus::Ready => break,
            MapJobStatus::Queued | MapJobStatus::Running(_) => {
                let remaining = hang_guard.saturating_duration_since(std::time::Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "catalogue WorkAdmission hang guard expired"
                );
                let outcome = handle.wait_for_change(remaining);
                assert_ne!(
                    outcome,
                    script::work_admission::PublicationWaitOutcome::Unavailable,
                    "WorkAdmission became unavailable before the catalogue hang guard"
                );
            }
            other => panic!("catalogue ended {other:?}"),
        }
    }
    let ready_map = handle.ready().unwrap();
    let ready = Arc::clone(&ready_map.catalogue);
    let catalogue = Catalogue::from_ready(
        Arc::clone(&world),
        identity,
        Some(Arc::clone(&ready_map.catalogue)),
        None,
    )
    .unwrap();
    drop(ready_map);
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
    let route = nav_planner::router::find_with(
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
            route: Some(Arc::new(route.clone())),
            map_route_generation: 7,
            ..Default::default()
        },
    );
    let manual = Mutex::new(crate::WalkArm::default());
    let read = |projection: Option<RouteProjection<'_>>| {
        projection.map(|p| (p.stamp.source, p.stamp.generation, p.route.destination()))
    };
    assert_eq!(
        play.with_map_route("alice", Some(&manual), None, read),
        Some((RouteSource::Script, 7, wt(3, 3, 0)))
    );
    assert_eq!(play.with_map_route("bob", Some(&manual), None, read), None);
    {
        let mut manual = manual.lock().unwrap();
        manual.route = Some(Arc::new(nav_planner::router::Route {
            dest: wt(2, 3, 0),
            ..route.clone()
        }));
        manual.route_generation = 8;
    }
    assert_eq!(
        play.with_map_route("alice", Some(&manual), None, read),
        Some((RouteSource::Script, 7, wt(3, 3, 0)))
    );
    let live = nav_planner::router::Route {
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
    manual.lock().unwrap().route = None;
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
    });
    let world = Arc::new(world(t(0, 0, 0), 8, &[(3, 3, 0)]));
    let catalogue = Catalogue::new(world, identity(), None, Some(services(vec![r]))).unwrap();
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
    client.gens.player_info = 1;
    client.gens.scene = 1;
    client.gens.session = 1;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    let world = world(t(3200, 3200, 0), 8, &[]);
    let ctx = context(&world);
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
    let replaced = context(&world);
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
    let data = || -> Arc<api::game_data::SelectedGameData> {
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "schema_version": 5,
                "revision": 289,
                "items": [],
                "consumption": [],
                "pickpocket": [],
                "teleports": [{
                    "name": "Harbour teleport", "source_row": "harbour", "spell": "harbour",
                    "component_id": 42, "members": false, "level": 1, "runes": [],
                    "experience": 1, "tele_coord": "0_0_0_8_9", "x": 8, "z": 9, "plane": 0
                }, {
                    "name": "Unrelated spell", "source_row": "elsewhere", "spell": "elsewhere",
                    "component_id": 43, "members": false, "level": 1, "runes": [],
                    "experience": 1, "tele_coord": "0_0_0_9_9", "x": 9, "z": 9, "plane": 0
                }]
            }))
            .unwrap(),
        )
    };
    let world = Arc::new(world);
    let catalogue = Catalogue::new(world, identity(), None, None)
        .unwrap()
        .with_game_data(data())
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
            &BankRows::default(),
            routing_test_admission(),
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

#[test]
fn single_bot_walk_and_teleport_follow_a_focus_switch() {
    let nav = world(t(3200, 3200, 0), 8, &[]);
    let fixture = MapFixture::new(&nav, "local-289");
    let mut play = fixture.play(t(3201, 3201, 0));
    push_slot(&mut play, "bob", t(3202, 3202, 0), None, true, true);
    play.cheats
        .lock()
        .unwrap()
        .insert("alice".into(), Default::default());
    play.cheats
        .lock()
        .unwrap()
        .insert("bob".into(), Default::default());
    let dest_ctx = bound_context(&play);
    let baked = play.world().unwrap();
    let dest = t(3205, 3206, 0);
    let mut model = MapModel::default();
    model.bind(dest_ctx);
    assert_eq!(model.select_tile(&baked, dest), Some(dest));
    play.focus("bob");
    let bob_ctx = MapContext {
        focus: play.map_focus("bob"),
        ..dest_ctx
    };
    let command = model
        .confirm(
            ActionKind::Walk,
            &bob_ctx,
            Some(t(3202, 3202, 0)),
            FindOptions::default(),
        )
        .unwrap();
    let snapshot = api::snapshot::GameSnapshot::new();
    let arms = Arc::new(Mutex::new(HashMap::new()));
    play.map_walk(
        command,
        &bob_ctx,
        &snapshot,
        &WorldState::empty(),
        &BankRows::default(),
        Default::default(),
        &arms,
    )
    .unwrap();
    assert_eq!(
        arms.lock().unwrap()["bob"].lock().unwrap().queued_tile(),
        Some(dest)
    );
    assert!(!arms.lock().unwrap().contains_key("alice"));

    model.bind(bob_ctx);
    assert_eq!(model.select_tile(&baked, dest), Some(dest));
    let command = model
        .confirm(
            ActionKind::Teleport,
            &bob_ctx,
            Some(t(3202, 3202, 0)),
            FindOptions::default(),
        )
        .unwrap();
    assert_eq!(play.map_teleport(command, &bob_ctx), Ok(()));
    let queues = play.cheats.lock().unwrap();
    assert_eq!(queues["alice"].len(), 0);
    assert_eq!(queues["bob"].len(), 1);
}

#[test]
fn group_walk_mixed_eligibility_own_origins_and_consumes_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    let log_mark = crate::walk_map::test_log::mark();
    let nav = world(t(3200, 3200, 0), 8, &[]);
    let fixture = MapFixture::new(&nav, "local-289");
    let mut play = fixture.play(t(3201, 3201, 0));
    push_slot(&mut play, "bob", t(3202, 3202, 0), None, true, true);
    push_slot(
        &mut play,
        "logged-out",
        t(3201, 3201, 0),
        None,
        false,
        false,
    );
    push_slot(&mut play, "nopos", t(0, 0, 0), None, true, true);
    push_slot(&mut play, "scripter", t(3204, 3204, 0), None, true, true);
    play.mark_script_running_for_walk("scripter");
    push_slot(&mut play, "bot3", t(3201, 3201, 1), None, true, true);

    let dest_ctx = bound_context(&play);
    let baked = play.world().unwrap();
    let mut model = MapModel::default();
    model.bind(dest_ctx);
    let dest = t(3205, 3206, 0);
    assert_eq!(model.select_tile(&baked, dest), Some(dest));

    let empty = WorldState::empty();
    let snapshot = api::snapshot::GameSnapshot::new();
    let bank = BankRows::default();
    let names = ["alice", "bob", "logged-out", "nopos", "scripter", "bot3"];
    let reqs: Vec<WalkSlotRequest<'_>> = names
        .iter()
        .map(|name| WalkSlotRequest {
            name,
            state: &empty,
            snapshot: Some(&snapshot),
            bank: &bank,
            risk_input: Default::default(),
        })
        .collect();

    assert_eq!(
        match play.walk_eligibility("alice") {
            WalkSlotStatus::Eligible(ready) => ready.origin,
            other => panic!("{other:?}"),
        },
        t(3201, 3201, 0)
    );
    assert_eq!(
        match play.walk_eligibility("bob") {
            WalkSlotStatus::Eligible(ready) => ready.origin,
            other => panic!("{other:?}"),
        },
        t(3202, 3202, 0)
    );
    assert_eq!(
        play.walk_eligibility("logged-out"),
        WalkSlotStatus::Excluded(WalkExclude::NotLoggedIn)
    );
    assert_eq!(
        play.walk_eligibility("nopos"),
        WalkSlotStatus::Excluded(WalkExclude::NoPosition)
    );
    assert_eq!(
        play.walk_eligibility("scripter"),
        WalkSlotStatus::Excluded(WalkExclude::RunningScript)
    );

    let plan = model
        .confirm_walk_plan(&dest_ctx, FindOptions::default())
        .unwrap();
    assert!(model.pending().is_none());
    assert_eq!(
        model
            .confirm_walk_plan(&dest_ctx, FindOptions::default())
            .unwrap_err(),
        ActionError::NoSelection
    );

    let alice_cmd = plan.command("alice", t(3201, 3201, 0));
    let bob_cmd = plan.command("bob", t(3202, 3202, 0));
    assert_eq!(alice_cmd.origin(), t(3201, 3201, 0));
    assert_eq!(bob_cmd.origin(), t(3202, 3202, 0));
    assert_eq!(alice_cmd.destination(), dest);
    assert_eq!(bob_cmd.destination(), dest);

    let arms = Arc::new(Mutex::new(HashMap::new()));
    let report = play.map_walk_group(plan, &dest_ctx, &reqs, &arms);
    let kinds: Vec<(&str, WalkSlotOutcomeKind)> = report
        .outcomes
        .iter()
        .map(|o| (o.name.as_str(), o.kind.clone()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("alice", WalkSlotOutcomeKind::Walking),
            ("bob", WalkSlotOutcomeKind::Walking),
            (
                "logged-out",
                WalkSlotOutcomeKind::Excluded(WalkExclude::NotLoggedIn)
            ),
            (
                "nopos",
                WalkSlotOutcomeKind::Excluded(WalkExclude::NoPosition)
            ),
            (
                "scripter",
                WalkSlotOutcomeKind::Excluded(WalkExclude::RunningScript)
            ),
            ("bot3", WalkSlotOutcomeKind::Failed(ActionError::NoPath)),
        ]
    );
    let reasons: Vec<Option<&str>> = report.outcomes.iter().map(|o| o.kind.reason()).collect();
    assert_eq!(
        reasons,
        vec![
            None,
            None,
            Some("not logged in"),
            Some("no position yet"),
            Some("running a script"),
            Some("no path"),
        ]
    );
    let records = crate::walk_map::test_log::records_since(log_mark);
    assert_eq!(records.len(), names.len() + 4, "{records:?}");
    for (name, request_outcome, terminal_reason) in [
        ("alice", "success ", None),
        ("bob", "success ", None),
        ("logged-out", "refused=NoFocus", Some("reason=NoFocus")),
        ("nopos", "refused=NoOrigin", Some("reason=NoOrigin")),
        (
            "scripter",
            "refused=RunningScript",
            Some("reason=RunningScript"),
        ),
        ("bot3", "refused=NoPath", Some("reason=NoPath")),
    ] {
        let lines: Vec<_> = records.iter().filter(|(slot, _)| slot == name).collect();
        let request_lines: Vec<_> = lines
            .iter()
            .filter(|(_, message)| !message.starts_with("WalkTo outcome="))
            .collect();
        assert_eq!(request_lines.len(), 1, "{name}: {lines:?}");
        let message = &request_lines[0].1;
        assert!(message.contains(request_outcome), "{message}");
        assert!(message.contains("members_source=unknown "), "{message}");
        let terminal_lines: Vec<_> = lines
            .iter()
            .filter(|(_, message)| message.starts_with("WalkTo outcome="))
            .collect();
        assert_eq!(
            terminal_lines.len(),
            usize::from(terminal_reason.is_some()),
            "{name}: {lines:?}"
        );
        if let Some(reason) = terminal_reason {
            assert!(terminal_lines[0].1.contains(reason), "{terminal_lines:?}");
        }
    }
    let arms = arms.lock().unwrap();
    assert_eq!(arms["alice"].lock().unwrap().queued_tile(), Some(dest));
    assert_eq!(arms["bob"].lock().unwrap().queued_tile(), Some(dest));
    let bot3 = arms
        .get("bot3")
        .expect("refused slot retains its arm shell");
    assert!(
        bot3.lock().unwrap().route.is_none(),
        "failed bot3 must not have an armed route or movement"
    );
}

#[test]
fn grouped_zone_refusal_keeps_its_named_detail() {
    let mut nav = world(t(3200, 3200, 0), 8, &[]);
    install_zone_wall(&mut nav);
    let fixture = MapFixture::new(&nav, "local-289");
    let play = fixture.play(t(3201, 3201, 0));
    let dest_ctx = bound_context(&play);
    let baked = play.world().unwrap();
    let mut model = MapModel::default();
    model.bind(dest_ctx);
    let destination = t(3205, 3206, 0);
    assert_eq!(model.select_tile(&baked, destination), Some(destination));
    let plan = model
        .confirm_walk_plan(&dest_ctx, FindOptions::default())
        .unwrap();

    let state = WorldState::empty();
    let bank = BankRows::default();
    let snapshot = api::snapshot::GameSnapshot::new();
    let request = [WalkSlotRequest {
        name: "alice",
        state: &state,
        snapshot: Some(&snapshot),
        bank: &bank,
        risk_input: Default::default(),
    }];
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let report = play.map_walk_group(plan, &dest_ctx, &request, &arms);
    assert!(!report.legacy_zones_unavailable);
    let WalkSlotOutcomeKind::Failed(error) = &report.outcomes[0].kind else {
        unreachable!("zone refusal must carry its error");
    };
    let ActionError::RiskRefused { refusal, detail } = error else {
        unreachable!("zone refusal must retain its typed risk diagnosis");
    };
    assert!(
        matches!(
            refusal,
            script::native::WalkRefusal::NoRouteWithinBounds { .. }
        ),
        "{refusal:?}"
    );
    assert!(detail.contains("blocked by danger zones:"), "{detail}");
    assert!(detail.contains("test-barrier@3203,"), "{detail}");
    assert_eq!(error.to_string(), detail.as_str());
}
/// Each failure keeps its own group-walk reason: a world-pack mismatch reads
/// "stale" and a membership refusal "members-only path", never a generic label.
#[test]
fn group_walk_reasons_label_stale_and_members_only_separately() {
    let reason = |error| WalkSlotOutcomeKind::Failed(error).reason();
    assert_eq!(
        reason(ActionError::BlockedByZones { detail: None }),
        Some("blocked by danger zones")
    );
    assert_eq!(
        ActionError::BlockedByZones { detail: None }.to_string(),
        "No path without crossing a danger zone; tick \"Route through danger zones\" to walk anyway"
    );
    assert_eq!(reason(ActionError::Blocked), Some("destination blocked"));
    assert_eq!(reason(ActionError::Stale), Some("stale"));
    assert_eq!(reason(ActionError::MembersOnly), Some("members-only path"));
}

#[test]
fn incomplete_search_group_reason_is_not_a_risk_refusal() {
    let detail = "search_result=BudgetExhausted nodes_settled=unavailable (incomplete search)";
    let error = ActionError::SearchIncomplete {
        detail: detail.to_owned(),
    };
    assert_eq!(error.short(), "route search incomplete");
    assert_eq!(
        WalkSlotOutcomeKind::Failed(error.clone()).reason(),
        Some("route search incomplete"),
    );
    assert_eq!(
        error.to_string(),
        format!("Route search incomplete: {detail}"),
    );
}

#[test]
fn incomplete_search_does_not_clear_existing_walk_arm() {
    const WIDTH: usize = 4003;
    const HEIGHT: usize = 2001;
    const SPLIT_X: usize = 2001;
    let cells = WIDTH * HEIGHT * 4;
    let mut blocked = vec![0u64; cells.div_ceil(64)];
    for level in 0..4 {
        for z in 0..HEIGHT {
            let index = level * WIDTH * HEIGHT + z * WIDTH + SPLIT_X;
            blocked[index >> 6] |= 1 << (index & 63);
        }
    }
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: wt(0, 0, 0),
            width: WIDTH,
            height: HEIGHT,
            walk: vec![0; cells],
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    );
    let from = wt(0, 1000, 0);
    let nearby = wt(1, 1000, 0);
    let destination = wt((WIDTH - 1) as i32, 1000, 0);
    let state = WorldState::empty();
    let route = Arc::new(
        nav_planner::router::find_with(
            &world.collision,
            &world.graph,
            from,
            nearby,
            FindOptions::default(),
            &state,
        )
        .unwrap(),
    );
    let arm = crate::walk_arm::WalkArm {
        route: Some(Arc::clone(&route)),
        route_generation: 17,
        refusal: Some(script::native::WalkRefusal::EscapeInProgress),
        refusal_detail: Some(Arc::from("previous risk refusal")),
        admission_pending: true,
        ..Default::default()
    };
    let arms: crate::walk_arm::WalkArms = Arc::new(Mutex::new(HashMap::from([(
        "alice".to_owned(),
        Arc::new(Mutex::new(arm)),
    )])));
    let options = FindOptions {
        zones: nav_planner::zones::ZoneExempt::all(),
        ..FindOptions::default()
    };
    let admission = crate::admission::Admission::manual(
        options,
        Default::default(),
        23,
        crate::WalkGlobals::default(),
    );
    let failure = crate::arm_walk_on(
        &world,
        t(from.x, from.z, from.level),
        t(destination.x, destination.z, destination.level),
        options,
        &state,
        &BankRows::default(),
        admission,
        &arms,
        Some("alice"),
    )
    .unwrap_err();
    assert!(failure.incomplete_search);
    assert!(matches!(
        failure.refusal,
        Some(script::native::WalkRefusal::SearchIncomplete { .. })
    ));

    let arms_guard = arms.lock().unwrap();
    let arm = arms_guard["alice"].lock().unwrap();
    assert!(arm
        .route
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, &route)));
    assert_eq!(arm.route_generation, 17);
    assert_eq!(
        arm.refusal,
        Some(script::native::WalkRefusal::EscapeInProgress)
    );
    assert_eq!(arm.refusal_detail.as_deref(), Some("previous risk refusal"));
    assert!(arm.admission_pending);
}

#[test]
fn manual_walk_refuses_before_boarding_and_names_the_total_coin_shortfall() {
    if api::test_support::local_asset_path(
        "",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/data/game-data/289.json"
        ),
        "revision 289 game data for manual walk shortfall test",
    )
    .is_none()
    {
        return;
    }
    let mut nav = world(t(0, 0, 0), 1, &[]);
    for level in 0..2 {
        let mut boat = edge(TransportKind::Boat, wt(0, 0, level), wt(0, 0, level + 1));
        boat.consumed_req = vec![(995, 30)];
        nav.graph.edges.push(boat);
    }
    // A free detour exists only when danger zones are exempted. Its zone
    // witness must not replace the shortfall on the legal paid route.
    nav.graph.edges.extend([
        edge(TransportKind::Ladder, wt(0, 0, 0), wt(0, 0, 3)),
        edge(TransportKind::Ladder, wt(0, 0, 3), wt(0, 0, 2)),
    ]);
    nav.graph.zones = Some(
        nav_planner::zones::ZoneTable::from_parts(
            vec![nav_planner::zones::Zone::npc(
                wt(0, 0, 3),
                0,
                nav_planner::zones::ZoneClass::Always,
                u16::MAX,
                0,
            )],
            vec![nav_planner::zones::ZoneKind::new(
                "free-unsafe-detour",
                "Unsafe detour",
                123,
                0,
                false,
                false,
            )],
            vec![],
            vec![],
            vec![],
            nav.collision.origin,
            1,
            1,
            &nav.graph.wilderness,
        )
        .unwrap(),
    );
    nav.graph.rebuild_index(&nav.collision);
    nav.bind_named_bank_facts(
        &api::game_data::for_revision(client::io::ClientRevision::R289).unwrap(),
    )
    .unwrap();
    let ctx = context(&nav);
    let arms = crate::WalkArms::default();
    let state = WorldState {
        inv: HashMap::from([(995, 30)]),
        ..WorldState::empty()
    };
    assert!(nav_planner::router::find_with(
        &nav.collision,
        &nav.graph,
        wt(0, 0, 0),
        wt(0, 0, 2),
        FindOptions {
            zones: nav_planner::zones::ZoneExempt::all(),
            ..FindOptions::default()
        },
        &state
    )
    .is_ok());
    let mut model = MapModel::default();
    model.bind(ctx);
    model.select_tile(&nav, t(0, 0, 2));
    let command = model
        .confirm(
            ActionKind::Walk,
            &ctx,
            Some(t(0, 0, 0)),
            FindOptions::default(),
        )
        .unwrap();
    let error = command
        .walk_on(
            &nav,
            &ctx,
            "alice",
            &state,
            &BankRows::default(),
            routing_test_admission(),
            &arms,
        )
        .unwrap_err();
    let message = error.to_string();
    assert!(
        matches!(error, ActionError::InsufficientItems { .. }),
        "{message}"
    );
    assert!(
        message.to_lowercase().contains("30 more coins"),
        "{message}"
    );
    assert!(message.contains("need 60, carrying 30"), "{message}");
    // A held default refusal is the pre-S2b bare NoPath: no arm is created.
    assert!(arms.lock().unwrap().get("alice").is_none());
}

#[test]
fn eligible_ordinary_group_walk_without_snapshot_uses_validated_status_origin() {
    let origin = t(3201, 3201, 0);
    let destination = t(3205, 3206, 0);
    let fixture = MapFixture::new(&world(t(3200, 3200, 0), 8, &[]), "local-289");
    let play = fixture.play(origin);
    let dest_ctx = bound_context(&play);
    let baked = play.world().unwrap();
    let mut model = MapModel::default();
    model.bind(dest_ctx);
    assert_eq!(model.select_tile(&baked, destination), Some(destination));
    let plan = model
        .confirm_walk_plan(&dest_ctx, FindOptions::default())
        .unwrap();
    let state = WorldState::empty();
    let bank = BankRows::default();
    let request = [WalkSlotRequest {
        name: "alice",
        state: &state,
        snapshot: None,
        bank: &bank,
        risk_input: Default::default(),
    }];
    let arms = Arc::new(Mutex::new(HashMap::new()));

    let report = play.map_walk_group(plan, &dest_ctx, &request, &arms);
    assert_eq!(report.outcomes[0].kind, WalkSlotOutcomeKind::Walking);
    assert_eq!(
        arms.lock().unwrap()["alice"].lock().unwrap().queued_tile(),
        Some(destination)
    );
}

#[test]
fn catalog_gated_group_walk_without_snapshot_cannot_start_a_prerequisite() {
    use nav_planner::quest_gates::{QuestEvidence, QuestGates};
    if api::test_support::local_asset_path(
        "",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/data/game-data/289.json"
        ),
        "revision 289 game data for catalog-gated group walk test",
    )
    .is_none()
    {
        return;
    }

    struct ClosedGate;
    impl api::quest_progress::EvidenceProvider for ClosedGate {
        fn observe_gate(
            &self,
            _: &api::selected::QuestGate,
            required_after: api::quest_progress::EvidenceStamp,
        ) -> Option<api::prereq_knowledge::Observation> {
            Some(api::prereq_knowledge::Observation {
                truth: api::selected::Truth::False,
                observed: Some(required_after),
            })
        }
    }

    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let catalog = api::selected::FamilyPreparation::run(move |worker| {
        selected.prepare_quests(worker).unwrap();
        selected.prepare_prereq_steps(worker).unwrap()
    })
    .unwrap()
    .join()
    .unwrap();
    let (row, entry) = catalog
        .iter()
        .flat_map(|(_, row)| row.crossings.iter().map(move |entry| (row, entry)))
        .find(|(_, entry)| {
            let crossing = &entry.crossing;
            crossing.takeoff != crossing.to
                && crossing.takeoff.level == crossing.to.level
                && crossing.takeoff.x.abs_diff(crossing.to.x) <= 4
                && crossing.takeoff.z.abs_diff(crossing.to.z) <= 4
        })
        .expect("selected catalog has a local gated crossing");
    let crossing = &entry.crossing;
    let origin = t(
        crossing.takeoff.x,
        crossing.takeoff.z,
        crossing.takeoff.level,
    );
    let destination = t(
        crossing.to.x + (crossing.to.x - crossing.takeoff.x).signum(),
        crossing.to.z + (crossing.to.z - crossing.takeoff.z).signum(),
        crossing.to.level,
    );
    let size = crossing
        .takeoff
        .x
        .abs_diff(crossing.to.x)
        .max(crossing.takeoff.z.abs_diff(crossing.to.z)) as usize
        + 3;
    let grid_origin = wt(
        crossing.takeoff.x.min(crossing.at.x).min(crossing.to.x) - 1,
        crossing.takeoff.z.min(crossing.at.z).min(crossing.to.z) - 1,
        0,
    );
    // Both endpoints are valid stands; wall bits forbid walking between them.
    let mut flags = vec![client::dash3d::CollisionFlag::SQ_BLOCKED as u32; 4 * size * size];
    for stand in [crossing.takeoff, crossing.to] {
        let x = (stand.x - grid_origin.x) as usize;
        let z = (stand.z - grid_origin.z) as usize;
        flags[stand.level as usize * size * size + z * size + x] =
            (client::dash3d::CollisionFlag::WALK_BLOCK_FLAGS
                & !client::dash3d::CollisionFlag::SQ_BLOCKED) as u32;
    }
    // Map clicks require a face-free destination, beyond the transport landing.
    let x = (destination.x - grid_origin.x) as usize;
    let z = (destination.z - grid_origin.z) as usize;
    flags[destination.level as usize * size * size + z * size + x] = 0;
    let (walk, blocked) = pack_walk(&flags);
    let mut gate_edge = edge(TransportKind::Door, crossing.at, crossing.to);
    gate_edge.loc_id = crossing.loc_id;
    gate_edge.option = i32::from(crossing.option);
    gate_edge.takeoff = Some(crossing.takeoff);
    gate_edge.quest_gates =
        Some(QuestGates::new([api::selected::QuestGate::Window(row.establishes.clone())]).unwrap());
    let mut graph = TransportGraph::default();
    graph.at.insert(crossing.takeoff, vec![0]);
    graph.edges.push(gate_edge);
    let nav = NavWorld::from_parts(
        WorldCollision {
            origin: grid_origin,
            width: size,
            height: size,
            walk,
            blocked,
            flags: None,
        },
        graph,
        vec![],
    );
    let mut state = WorldState::empty();
    state.establishable = Some(catalog.establishable_windows());
    state.quest_evidence = Some(QuestEvidence::new(
        Arc::new(ClosedGate),
        api::quest_progress::EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 0,
        },
    ));
    assert!(
        nav_planner::router::find_actionable_quest_gates(
            &nav.collision,
            &nav.graph,
            crossing.takeoff,
            &[wt(destination.x, destination.z, destination.level)],
            FindOptions::default(),
            &state,
            &[],
        )
        .is_some(),
        "the route must need a real catalog-backed prerequisite"
    );

    let fixture = MapFixture::new(&nav, "local-289");
    let play = fixture.play(origin);
    let context = bound_context(&play);
    let mut model = MapModel::default();
    model.bind(context);
    model.select_tile(&play.world().unwrap(), destination);
    let plan = model
        .confirm_walk_plan(&context, FindOptions::default())
        .unwrap();
    let bank = BankRows::default();
    let request = [WalkSlotRequest {
        name: "alice",
        state: &state,
        snapshot: None,
        bank: &bank,
        risk_input: Default::default(),
    }];
    let arms = Arc::new(Mutex::new(HashMap::new()));
    let report = play.map_walk_group(plan, &context, &request, &arms);
    assert_eq!(
        report.outcomes[0].kind,
        WalkSlotOutcomeKind::Failed(ActionError::NoPath)
    );
    assert!(arms.lock().unwrap()["alice"]
        .lock()
        .unwrap()
        .route
        .is_none());
    assert_eq!(play.script_state("alice"), script::RunState::Idle);
}
