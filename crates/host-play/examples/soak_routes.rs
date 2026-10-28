//! Offline T1.4 fixture witnesses from production routing and hazard retreat.

use api::snapshot::WorldTile;
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: soak_routes PACK BATCHES_JSON OUTPUT_JSONL".into());
    }
    let world = Arc::new(NavWorld::load_pack(Path::new(&args[0]))?);
    let lease = nav_planner::producer::lease(&world)?;
    let batches: Vec<Batch> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut state = WorldState::empty();
    state.map_members = true;
    state.combat_level = Some(3);
    let mut out = BufWriter::new(std::fs::File::create(&args[2])?);
    for batch in batches {
        let targets: Vec<_> = batch
            .targets
            .into_iter()
            .map(tile)
            .filter(|target| world.collision.standable(*target))
            .collect();
        for source in batch.sources.into_iter().map(tile) {
            write_routes(&world, &state, &batch.name, source, &targets, "route", &mut out)?;
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
    }
    out.flush()?;
    Ok(())
}
