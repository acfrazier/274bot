//! Offline T1.4 fixture witnesses from production routing and hazard retreat.

use api::snapshot::WorldTile;
use client::config::{Cache, IfType, IfTypeMut};
use client::dash3d::{BuildArea, CollisionMap, LocAngle, LocShape};
use client::io::{ClientRevision, JagFile};
use client::{Client, ClientConfig};
use nav_planner::router::{find_many_with, FindOptions, Leg};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

#[derive(Deserialize)]
struct Batch {
    name: String,
    sources: Vec<[i32; 3]>,
    targets: Vec<[i32; 3]>,
    #[serde(default)]
    retreats: Vec<[[i32; 3]; 2]>,
    #[serde(default)]
    approaches: Vec<Approach>,
}

#[derive(Clone, Copy, Deserialize)]
struct Approach {
    source: [i32; 3],
    target: [i32; 3],
    id: i32,
    shape: i32,
    angle: i32,
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
                let mut transports = 0;
                for leg in &route.legs {
                    match leg {
                        Leg::Walk { tiles: walk } => {
                            tiles.extend(walk.iter().map(|step| [step.x, step.z, step.level]));
                        }
                        Leg::Transport { edge } => {
                            tiles.push([edge.at.x, edge.at.z, edge.at.level]);
                            tiles.push([edge.to.x, edge.to.z, edge.to.level]);
                            transports += 1;
                        }
                    }
                }
                row["tiles"] = json!(tiles);
                row["transports"] = json!(transports);
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

fn approach_row(batch: &str, approach: Approach) -> serde_json::Value {
    json!({
        "batch": batch,
        "kind": "client-approach",
        "source": approach.source,
        "target": approach.target,
        "id": approach.id,
        "shape": approach.shape,
        "angle": approach.angle
    })
}

fn write_approach_error(
    batch: &str,
    approach: Approach,
    scene_base: Option<[i32; 2]>,
    error: &str,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut row = approach_row(batch, approach);
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

fn write_client_approach(
    client: &mut Client,
    flags: &nav_planner::pack::FlagsSidecarLoad,
    batch: &str,
    approach: Approach,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let flags_origin = flags.origin;
    let flags_width = flags.width;
    let flags_height = flags.height;
    let flags = flags.flags.as_slice();
    let source = tile(approach.source);
    let target = tile(approach.target);
    if source.level != target.level {
        return write_approach_error(
            batch,
            approach,
            None,
            "source-target-level-mismatch",
            out,
        );
    }
    if !(0..BuildArea::LEVELS).contains(&source.level) {
        return write_approach_error(batch, approach, None, "invalid-level", out);
    }
    if !(0..=LocAngle::SOUTH).contains(&approach.angle) {
        return write_approach_error(batch, approach, None, "invalid-angle", out);
    }

    let Some(loc) = usize::try_from(approach.id)
        .ok()
        .and_then(|id| client.cache.locs.get(id))
    else {
        return write_approach_error(batch, approach, None, "loc-id-not-in-cache", out);
    };
    let centrepiece = approach.shape == LocShape::CENTREPIECE_STRAIGHT
        || approach.shape == LocShape::CENTREPIECE_DIAGONAL
        || approach.shape == LocShape::GROUND_DECOR;
    let (loc_width, loc_length, loc_angle, loc_shape, forceapproach) = if centrepiece {
        let (width, height) = if approach.angle == LocAngle::WEST || approach.angle == LocAngle::EAST {
            (loc.width, loc.length)
        } else {
            (loc.length, loc.width)
        };
        let mut forceapproach = loc.forceapproach;
        if approach.angle != 0 {
            forceapproach =
                ((forceapproach << approach.angle) & 0xf) + (forceapproach >> (4 - approach.angle));
        }
        (width, height, 0, 0, forceapproach)
    } else {
        let Some(loc_shape) = approach.shape.checked_add(1) else {
            return write_approach_error(batch, approach, None, "invalid-shape", out);
        };
        (0, 0, approach.angle, loc_shape, 0)
    };

    let Some((first_base_x, last_base_x)) = scene_base_range(source.x, target.x) else {
        return write_approach_error(batch, approach, None, "endpoints-do-not-fit-scene", out);
    };
    let Some((first_base_z, last_base_z)) = scene_base_range(source.z, target.z) else {
        return write_approach_error(batch, approach, None, "endpoints-do-not-fit-scene", out);
    };

    let mut base_x = first_base_x;
    while base_x <= last_base_x {
        let mut base_z = first_base_z;
        while base_z <= last_base_z {
            let scene_base = [base_x, base_z];
            if !scene_fits_flags(base_x, base_z, flags_origin, flags_width, flags_height) {
                write_approach_error(
                    batch,
                    approach,
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
                        let index =
                            plane_offset + (grid_z + z as usize) * flags_width + grid_x + x as usize;
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
                    2,
                );
                if !reached {
                    write_approach_error(
                        batch,
                        approach,
                        Some(scene_base),
                        "client-no-route",
                        out,
                    )?;
                } else if client.try_move_path.is_empty() {
                    write_approach_error(
                        batch,
                        approach,
                        Some(scene_base),
                        "client-route-path-empty",
                        out,
                    )?;
                } else {
                    let mut row = approach_row(batch, approach);
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
            members: true,
            lowmem: false,
        },
        cache,
        ifaces,
        ifaces_mut,
        ClientRevision::R289,
    );
    let batches: Vec<Batch> = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let mut state = WorldState::empty();
    state.map_members = true;
    state.combat_level = Some(3);
    let mut out = BufWriter::new(std::fs::File::create(&args[4])?);
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
        for approach in batch.approaches {
            write_client_approach(
                &mut client,
                &loaded_flags,
                &batch.name,
                approach,
                &mut out,
            )?;
        }
    }
    out.flush()?;
    Ok(())
}
