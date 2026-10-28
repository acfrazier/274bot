//! Offline T1.4 fixture witnesses from production routing and hazard retreat.

use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, ClientConfig, ClientPlayer};
use client::config::{Cache, IfType, IfTypeMut};
use client::dash3d::{BuildArea, ClientNpc, CollisionMap, LocAngle, LocShape};
use client::io::{ClientRevision, JagFile};
use host::{Guardian, GuardianLiveContext, RandomKind};
use nav_planner::router::{find_many_with, FindOptions, Leg};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use vault::ProfileSettings;

#[derive(Deserialize)]
struct Batch {
    name: String,
    sources: Vec<[i32; 3]>,
    targets: Vec<[i32; 3]>,
    #[serde(default)]
    retreats: Vec<[[i32; 3]; 2]>,
    #[serde(default)]
    approaches: Vec<Approach>,
    #[serde(default)]
    walks: Vec<Walk>,
    #[serde(default)]
    guardians: Vec<GuardianRequest>,
    #[serde(default)]
    hazards: Vec<Walk>,
}

#[derive(Clone, Copy, Deserialize)]
struct Approach {
    source: [i32; 3],
    target: [i32; 3],
    id: i32,
    shape: i32,
    angle: i32,
}

#[derive(Clone, Copy, Deserialize)]
struct Walk {
    source: [i32; 3],
    target: [i32; 3],
}

#[derive(Clone, Copy, Deserialize)]
struct GuardianRequest {
    source: [i32; 3],
    npc_id: usize,
    offset: [i32; 2],
}

struct GuardianAudit {
    data: Arc<api::game_data::SelectedGameData>,
    settings: ProfileSettings,
    snapshot: GameSnapshot,
}

#[derive(Clone, Copy)]
enum ClientRoute {
    Approach(Approach),
    Walk(Walk),
    Hazard(Walk),
    Guardian(GuardianRequest),
}

impl ClientRoute {
    fn endpoints(self) -> ([i32; 3], [i32; 3]) {
        match self {
            Self::Approach(approach) => (approach.source, approach.target),
            Self::Walk(walk) | Self::Hazard(walk) => (walk.source, walk.target),
            Self::Guardian(request) => (request.source, request.source),
        }
    }
}

fn tile(value: [i32; 3]) -> WorldTile {
    WorldTile {
        x: value[0],
        z: value[1],
        level: value[2],
    }
}

fn write_routes(
    world: &NavWorld,
    state: &WorldState,
    batch: &str,
    source: WorldTile,
    targets: &[WorldTile],
    kind: &str,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    if !world.collision.standable(source) {
        serde_json::to_writer(
            &mut *out,
            &json!({"batch":batch,"source":[source.x,source.z,source.level],
                "error":"source-not-standable"}),
        )?;
        writeln!(out)?;
    }
    let search = find_many_with(
        &world.collision,
        &world.graph,
        source,
        targets,
        FindOptions::default(),
        state,
    );
    for (index, target) in targets.iter().enumerate() {
        let mut row = json!({"batch":batch,"kind":kind,
            "source":[source.x,source.z,source.level],
            "target":[target.x,target.z,target.level]});
        match search.route(index) {
            Ok(route) => {
                let mut tiles = vec![[source.x, source.z, source.level]];
                let mut transport_edges = Vec::new();
                for leg in &route.legs {
                    match leg {
                        Leg::Walk { tiles: walk } => {
                            tiles.extend(walk.iter().map(|step| [step.x, step.z, step.level]));
                        }
                        Leg::Transport { edge } => {
                            tiles.push([edge.at.x, edge.at.z, edge.at.level]);
                            tiles.push([edge.to.x, edge.to.z, edge.to.level]);
                            let has_requirements = edge.takeoff.is_some()
                                || edge.player_delta.is_some()
                                || !edge.skill_req.is_empty()
                                || !edge.item_req.is_empty()
                                || !edge.consumed_req.is_empty()
                                || !edge.item_returns.is_empty()
                                || !edge.quest_req.is_empty()
                                || !edge.varp_req.is_empty()
                                || !edge.worn_req.is_empty()
                                || !edge.worn_all_req.is_empty()
                                || !edge.forbidden_held.is_empty()
                                || !edge.forbidden_worn.is_empty()
                                || edge.members_req
                                || edge.wildy_cap.is_some()
                                || edge.quest_gates.is_some();
                            transport_edges.push(json!({
                                "kind": if matches!(
                                    edge.kind,
                                    nav_planner::transport::TransportKind::Door
                                ) { "Door" } else { "Other" },
                                "at": [edge.at.x, edge.at.z, edge.at.level],
                                "to": [edge.to.x, edge.to.z, edge.to.level],
                                "loc_id": edge.loc_id,
                                "open_loc_id": edge.open_loc_id,
                                "has_requirements": has_requirements,
                            }));
                        }
                    }
                }
                row["tiles"] = json!(tiles);
                row["transports"] = json!(transport_edges.len());
                row["transport_edges"] = json!(transport_edges);
                row["ticks"] = json!(route.ticks);
            }
            Err(error) => row["error"] = json!(format!("{error:?}")),
        }
        serde_json::to_writer(&mut *out, &row)?;
        writeln!(out)?;
    }
    Ok(())
}
fn write_json_line(
    out: &mut impl Write,
    row: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(&mut *out, row)?;
    writeln!(out)?;
    Ok(())
}

fn client_row(batch: &str, route: ClientRoute) -> serde_json::Value {
    let (source, target) = route.endpoints();
    let mut row = json!({"batch":batch,"source":source,"target":target});
    match route {
        ClientRoute::Approach(approach) => {
            row["kind"] = json!("client-approach");
            row["id"] = json!(approach.id);
            row["shape"] = json!(approach.shape);
            row["angle"] = json!(approach.angle);
        }
        ClientRoute::Walk(_) => row["kind"] = json!("client-walk"),
        ClientRoute::Hazard(_) => row["kind"] = json!("guardian-hazard"),
        ClientRoute::Guardian(request) => {
            row["kind"] = json!("guardian-flee");
            row["npc_id"] = json!(request.npc_id);
            row["npc_start"] = json!([
                request.source[0] + request.offset[0],
                request.source[1] + request.offset[1],
                request.source[2]
            ]);
        }
    }
    row
}

fn write_client_error(
    batch: &str,
    route: ClientRoute,
    scene_base: Option<[i32; 2]>,
    error: &str,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut row = client_row(batch, route);
    if let Some(scene_base) = scene_base {
        row["scene_base"] = json!(scene_base);
    }
    row["error"] = json!(error);
    write_json_line(out, &row)
}

/// Every 8-aligned base that puts both tile coordinates at indices 1..=102.
fn scene_base_range(first_tile: i32, second_tile: i32) -> Option<(i32, i32)> {
    let lower = i64::from(first_tile.max(second_tile)) - i64::from(BuildArea::SIZE - 2);
    let upper = i64::from(first_tile.min(second_tile)) - 1;
    let lower_zone = lower.div_euclid(8) + if lower.rem_euclid(8) != 0 { 1 } else { 0 };
    let first = lower_zone * 8;
    let last = upper.div_euclid(8) * 8;
    if first > last {
        return None;
    }
    Some((i32::try_from(first).ok()?, i32::try_from(last).ok()?))
}

fn scene_fits_flags(
    base_x: i32,
    base_z: i32,
    origin: WorldTile,
    width: usize,
    height: usize,
) -> bool {
    let local_x = i64::from(base_x) - i64::from(origin.x);
    let local_z = i64::from(base_z) - i64::from(origin.z);
    local_x >= 0
        && local_z >= 0
        && local_x + i64::from(BuildArea::SIZE) <= width as i64
        && local_z + i64::from(BuildArea::SIZE) <= height as i64
}

fn write_client_route(
    client: &mut Client,
    flags: &nav_planner::pack::FlagsSidecarLoad,
    batch: &str,
    route: ClientRoute,
    guardian_audit: &mut GuardianAudit,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let flags_origin = flags.origin;
    let flags_width = flags.width;
    let flags_height = flags.height;
    let flags = flags.flags.as_slice();
    let (source, target) = route.endpoints();
    let source = tile(source);
    let target = tile(target);
    if source.level != target.level {
        return write_client_error(batch, route, None, "source-target-level-mismatch", out);
    }
    if !(0..BuildArea::LEVELS).contains(&source.level) {
        return write_client_error(batch, route, None, "invalid-level", out);
    }
    let (loc_width, loc_length, loc_angle, loc_shape, forceapproach) =
        if let ClientRoute::Approach(approach) = route {
            if !(0..=LocAngle::SOUTH).contains(&approach.angle) {
                return write_client_error(batch, route, None, "invalid-angle", out);
            }
            let Some(loc) = usize::try_from(approach.id)
                .ok()
                .and_then(|id| client.cache.locs.get(id))
            else {
                return write_client_error(batch, route, None, "loc-id-not-in-cache", out);
            };
            let centrepiece = approach.shape == LocShape::CENTREPIECE_STRAIGHT
                || approach.shape == LocShape::CENTREPIECE_DIAGONAL
                || approach.shape == LocShape::GROUND_DECOR;
            if centrepiece {
                let (width, height) =
                    if approach.angle == LocAngle::WEST || approach.angle == LocAngle::EAST {
                        (loc.width, loc.length)
                    } else {
                        (loc.length, loc.width)
                    };
                let mut forceapproach = loc.forceapproach;
                if approach.angle != 0 {
                    forceapproach = ((forceapproach << approach.angle) & 0xf)
                        + (forceapproach >> (4 - approach.angle));
                }
                (width, height, 0, 0, forceapproach)
            } else {
                let Some(loc_shape) = approach.shape.checked_add(1) else {
                    return write_client_error(batch, route, None, "invalid-shape", out);
                };
                (0, 0, approach.angle, loc_shape, 0)
            }
        } else {
            (0, 0, 0, 0, 0)
        };
    let move_type = if matches!(route, ClientRoute::Approach(_)) {
        2
    } else {
        0
    };

    let Some((first_base_x, last_base_x)) = scene_base_range(source.x, target.x) else {
        return write_client_error(batch, route, None, "endpoints-do-not-fit-scene", out);
    };
    let Some((first_base_z, last_base_z)) = scene_base_range(source.z, target.z) else {
        return write_client_error(batch, route, None, "endpoints-do-not-fit-scene", out);
    };

    let mut base_x = first_base_x;
    while base_x <= last_base_x {
        let mut base_z = first_base_z;
        while base_z <= last_base_z {
            let scene_base = [base_x, base_z];
            if !scene_fits_flags(base_x, base_z, flags_origin, flags_width, flags_height) {
                write_client_error(
                    batch,
                    route,
                    Some(scene_base),
                    "scene-outside-flags-grid",
                    out,
                )?;
            } else {
                let level = source.level as usize;
                let grid_x = (i64::from(base_x) - i64::from(flags_origin.x)) as usize;
                let grid_z = (i64::from(base_z) - i64::from(flags_origin.z)) as usize;
                let plane_cells = flags_width * flags_height;
                let plane_offset = level * plane_cells;
                let collision: &mut CollisionMap = &mut client.collision[level];
                collision.start_x = 0;
                collision.start_z = 0;
                collision.size_x = BuildArea::SIZE;
                collision.size_z = BuildArea::SIZE;
                // Match the client's scene reset on the edges; the full interior
                // comes only from the actual raw flags sidecar.
                collision.reset();
                for x in 1..BuildArea::SIZE - 1 {
                    for z in 1..BuildArea::SIZE - 1 {
                        let index = plane_offset
                            + (grid_z + z as usize) * flags_width
                            + grid_x
                            + x as usize;
                        collision.flags[x as usize][z as usize] = flags[index] as i32;
                    }
                }

                let source_x = source.x - base_x;
                let source_z = source.z - base_z;
                let target_x = target.x - base_x;
                let target_z = target.z - base_z;
                client.map_build_base_x = base_x;
                client.map_build_base_z = base_z;
                client.minusedlevel = source.level;
                client.out.pos = 0;
                client.try_move_path.clear();
                if let ClientRoute::Guardian(request) = route {
                    write_guardian_route(client, guardian_audit, batch, request, scene_base, out)?;
                    if base_z == last_base_z {
                        break;
                    }
                    base_z += 8;
                    continue;
                }
                let reached = client.tryMove(
                    source_x,
                    source_z,
                    target_x,
                    target_z,
                    false,
                    loc_width,
                    loc_length,
                    loc_angle,
                    loc_shape,
                    forceapproach,
                    move_type,
                );
                if !reached {
                    write_client_error(batch, route, Some(scene_base), "client-no-route", out)?;
                } else if client.try_move_path.is_empty() {
                    write_client_error(
                        batch,
                        route,
                        Some(scene_base),
                        "client-route-path-empty",
                        out,
                    )?;
                } else {
                    let mut row = client_row(batch, route);
                    row["scene_base"] = json!(scene_base);
                    row["tiles"] = json!(client
                        .try_move_path
                        .iter()
                        .map(|&(x, z)| [base_x + x, base_z + z, source.level])
                        .collect::<Vec<_>>());
                    write_json_line(out, &row)?;
                }
            }
            if base_z == last_base_z {
                break;
            }
            base_z += 8;
        }
        if base_x == last_base_x {
            break;
        }
        base_x += 8;
    }
    Ok(())
}

fn write_guardian_route(
    client: &mut Client,
    audit: &mut GuardianAudit,
    batch: &str,
    request: GuardianRequest,
    scene_base: [i32; 2],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = &audit.data;
    let npc_data = data
        .npc_name(i32::try_from(request.npc_id)?)
        .ok_or("Guardian request has no selected-content NPC")?;
    let [base_x, base_z] = scene_base;
    let [world_x, world_z, level] = request.source;
    let (x, z) = (world_x - base_x, world_z - base_z);
    let player = client.local_player.as_mut().ok_or("audit player missing")?;
    player.entity.x = x * 128 + 64;
    player.entity.z = z * 128 + 64;
    player.route_x[0] = x;
    player.route_z[0] = z;
    player.entity.route_length = 0;
    let npc = client.npc[0].as_mut().ok_or("audit NPC missing")?;
    npc.r#type = Some(request.npc_id);
    npc.entity.face_entity = 32768;
    npc.entity.x = (x + request.offset[0]) * 128 + 64;
    npc.entity.z = (z + request.offset[1]) * 128 + 64;
    let settings = &audit.settings;
    let context = GuardianLiveContext {
        event_policies: &settings.random_event_policies,
        game_data: Some(data),
    };
    let mut guardian = Guardian::new();
    let snapshot = &mut audit.snapshot;
    // This bound includes every nearer-ring retry. The real Guardian chooses
    // exact goals, nearest fallback, and ring order through the real Client.
    let ticks = npc_data.maxrange.max(npc_data.wanderrange).max(0) + 5;
    for _ in 0..ticks {
        client.gens.npc = client.gens.npc.wrapping_add(1);
        client.gens.player = client.gens.player.wrapping_add(1);
        client.gens.player_info = client.gens.player_info.wrapping_add(1);
        client.gens.scene = client.gens.scene.wrapping_add(1);
        snapshot.rebuild(client);
        let status =
            guardian.tick_with_live_policies(client, snapshot, settings, &context, 0, None);
        if status.kind != Some(RandomKind::Evade) || !status.hold {
            return write_client_error(
                batch,
                ClientRoute::Guardian(request),
                Some(scene_base),
                "guardian-did-not-own-evade",
                out,
            );
        }
        if !client.try_move_path.is_empty() {
            let flee_path = client.try_move_path.clone();
            let mut row = client_row(batch, ClientRoute::Guardian(request));
            row["scene_base"] = json!(scene_base);
            row["target"] = json!(flee_path
                .last()
                .map(|&(x, z)| [base_x + x, base_z + z, level]));
            row["tiles"] = json!(flee_path
                .iter()
                .map(|&(x, z)| [base_x + x, base_z + z, level])
                .collect::<Vec<_>>());
            write_json_line(out, &row)?;
            // resolve() returns to flee_from with an exact ground walk.
            // Cover resolution from every intermediate flee tile, not only
            // the final tile. These are actual client BFS routes too.
            for (return_x, return_z) in flee_path {
                client.out.pos = 0;
                client.try_move_path.clear();
                let reached = client.tryMove(return_x, return_z, x, z, false, 0, 0, 0, 0, 0, 0);
                let mut row = json!({
                    "batch":batch, "kind":"guardian-return", "npc_id":request.npc_id,
                    "source":[base_x + return_x,base_z + return_z,level],
                    "target":request.source, "scene_base":scene_base,
                });
                if !reached {
                    row["error"] = json!("client-no-route");
                } else if client.try_move_path.is_empty() {
                    row["error"] = json!("client-route-path-empty");
                } else {
                    row["tiles"] = json!(client
                        .try_move_path
                        .iter()
                        .map(|&(x, z)| [base_x + x, base_z + z, level])
                        .collect::<Vec<_>>());
                }
                write_json_line(out, &row)?;
            }
            return Ok(());
        }
    }
    write_client_error(
        batch,
        ClientRoute::Guardian(request),
        Some(scene_base),
        "client-no-route",
        out,
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: soak_routes PACK FLAGS CACHE BATCHES_JSON OUTPUT_JSONL".into());
    }
    let world = Arc::new(NavWorld::load_pack(Path::new(&args[0]))?);
    let loaded_flags = nav_planner::pack::read_flags_sidecar(Path::new(&args[1]), true)?;
    if loaded_flags.origin != world.collision.origin
        || loaded_flags.width != world.collision.width
        || loaded_flags.height != world.collision.height
    {
        return Err("flags sidecar header does not match nav world".into());
    }
    let expected_flag_words = loaded_flags
        .width
        .checked_mul(loaded_flags.height)
        .and_then(|cells| cells.checked_mul(BuildArea::LEVELS as usize))
        .ok_or("flags sidecar dimensions overflow")?;
    if loaded_flags.flags.len() != expected_flag_words {
        return Err("flags sidecar word count does not match nav world".into());
    }
    let lease = nav_planner::producer::lease(&world)?;
    let cache_dir = args[2].clone();
    let cache = Arc::new(Cache::unpack(&JagFile::new(std::fs::read(
        Path::new(&cache_dir).join("config"),
    )?)));
    let ifaces: Arc<Vec<Option<Box<IfType>>>> = Arc::new(Vec::new());
    let ifaces_mut: Arc<Vec<Option<Arc<IfTypeMut>>>> = Arc::new(Vec::new());
    let mut client = Client::from_shared_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir,
            members: false,
            lowmem: false,
        },
        cache,
        ifaces,
        ifaces_mut,
        ClientRevision::R289,
    );
    client.self_slot = 0;
    client.ingame = true;
    client.scene_state = 2;
    let mut player = ClientPlayer::at(0, 0);
    player.name = Some("T14RouteAudit".to_string());
    client.local_player = Some(player);
    client.npc[0] = Some(Box::new(ClientNpc::at(0, 0)));
    client.npc_ids[0] = 0;
    client.npc_count = 1;
    let batches: Vec<Batch> = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let mut state = WorldState::empty();
    // The custom WSS roster has no membership declaration. Its Remote
    // profile resolves Unknown as false, independently of local world.json.
    state.map_members = false;
    state.combat_level = Some(3);
    let mut out = BufWriter::new(std::fs::File::create(&args[4])?);
    let mut guardian_audit = GuardianAudit {
        data: api::game_data::for_revision(ClientRevision::R289)?,
        settings: ProfileSettings::default(),
        snapshot: GameSnapshot::new(),
    };
    for batch in batches {
        let targets: Vec<_> = batch
            .targets
            .into_iter()
            .map(tile)
            .filter(|target| world.collision.standable(*target))
            .collect();
        for source in batch.sources.into_iter().map(tile) {
            write_routes(
                &world,
                &state,
                &batch.name,
                source,
                &targets,
                "route",
                &mut out,
            )?;
        }
        for [here, hazard] in batch.retreats {
            if let Some(target) = script::gatherer::test_hazard_retreat_stand(
                tile(here),
                tile(hazard),
                lease.collision(),
            ) {
                write_routes(
                    &world,
                    &state,
                    &batch.name,
                    tile(here),
                    &[target],
                    "hazard-retreat",
                    &mut out,
                )?;
            }
        }
        for route in batch
            .approaches
            .into_iter()
            .map(ClientRoute::Approach)
            .chain(batch.walks.into_iter().map(ClientRoute::Walk))
            .chain(batch.guardians.into_iter().map(ClientRoute::Guardian))
            .chain(batch.hazards.into_iter().map(ClientRoute::Hazard))
        {
            write_client_route(
                &mut client,
                &loaded_flags,
                &batch.name,
                route,
                &mut guardian_audit,
                &mut out,
            )?;
        }
    }
    out.flush()?;
    Ok(())
}
