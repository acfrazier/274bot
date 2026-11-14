use std::env;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use client::io::ClientRevision;
use host_play::walk_map::{Catalogue, Search};
use nav_planner::map::formats::{ServicePois, ShippedCatalogue};
use nav_planner::world::NavWorld;

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))
}

fn check(base: &Path) -> Result<serde_json::Value, String> {
    let world = Arc::new(
        NavWorld::load_pack(&base.join("nav/289/274bot.navpack"))
            .map_err(|error| format!("load navigation pack: {error}"))?,
    );
    let services = Arc::new(
        ServicePois::decode_navpois(&read_file(&base.join("nav/289/274bot.navpois"))?)
            .map_err(|error| format!("decode service POIs: {error}"))?,
    );
    let map_root = base.join("map/289");
    let description_path = map_root.join(host_play::map_cache::SHIPPED_CATALOGUE_NAME);
    let description: ShippedCatalogue = serde_json::from_slice(&read_file(&description_path)?)
        .map_err(|error| format!("decode shipped catalogue description: {error}"))?;
    if description.identity.revision != 289 {
        return Err(format!(
            "expected revision 289, found {}",
            description.identity.revision
        ));
    }
    description
        .validate(description.identity)
        .map_err(|error| format!("validate shipped catalogue description: {error}"))?;
    let catalogue_dir = map_root
        .join("catalogues")
        .join(description.key.0.to_string());
    let manifest = description
        .read_manifest(&read_file(&catalogue_dir.join("manifest.json"))?)
        .map_err(|error| format!("read shipped catalogue manifest: {error}"))?;
    let client = Arc::new(
        manifest
            .read_payload(&read_file(&catalogue_dir.join("client-pois.json"))?)
            .map_err(|error| format!("read client POIs: {error}"))?,
    );
    let game_data = api::game_data::for_revision(ClientRevision::R289)
        .map_err(|error| format!("load revision 289 game data: {error}"))?;
    let catalogue = Catalogue::new(world, description.identity, Some(client), Some(services))
        .map_err(|error| format!("build map catalogue: {error}"))?
        .with_game_data(game_data)
        .map_err(|error| format!("name map catalogue: {error}"))?;
    let entries = catalogue.entries().len();
    if entries == 0 {
        return Err("map catalogue has no entries".into());
    }

    let mut searches = serde_json::Map::new();
    let mut search = Search::default();
    for query in ["bank", "lum", "Nulo", "Cook"] {
        search
            .update(&catalogue, query)
            .map_err(|error| format!("search {query:?}: {error}"))?;
        let hits = search.results().len();
        if hits == 0 {
            return Err(format!("search {query:?} has no results"));
        }
        searches.insert(query.into(), serde_json::json!(hits));
    }
    Ok(serde_json::json!({
        "revision": 289,
        "entries": entries,
        "searches": searches,
    }))
}

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(base) = args.next() else {
        eprintln!("FAIL: expected one staged package directory");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("FAIL: expected one staged package directory");
        return ExitCode::FAILURE;
    }
    match check(Path::new(&base)) {
        Ok(receipt) => {
            println!("{receipt}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("FAIL: {error}");
            ExitCode::FAILURE
        }
    }
}
