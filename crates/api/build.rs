use std::env;
use std::fs::{self, File};
use std::io;
use std::path::PathBuf;

#[path = "build_support/catalog_codegen.rs"]
mod catalog_codegen;
#[path = "src/test_support/local_asset_path.rs"]
mod local_asset_path;

use flate2::write::GzEncoder;
use flate2::Compression;

const GENERATE_GAME_DATA: &str = "python tools/local-data/build.py";
const BANK_CATALOG_JSON: &str = "data/game-data/bank-catalog.json";
const COOK_CATALOG_JSON: &str = "data/game-data/cook-catalog.json";
const LOCAL_ASSETS: &[&str] = &[
    "data/game-data/274.json",
    "data/game-data/289.json",
    "data/game-data/377.json",
    "data/game-data/377/debug.json",
    "data/game-data/377/quests.json",
    "data/game-data/377/gathering.json",
    "data/game-data/274/debug.json",
    "data/game-data/289/debug.json",
    "data/game-data/274/quests.json",
    "data/game-data/289/quests.json",
    "data/game-data/274/gathering.json",
    "data/game-data/289/gathering.json",
    "data/game-data/289/prereq_steps.json",
    BANK_CATALOG_JSON,
    COOK_CATALOG_JSON,
];

fn main() -> io::Result<()> {
    println!("cargo:rustc-check-cfg=cfg(no_local_game_data)");
    println!("cargo:rerun-if-env-changed=GITHUB_ACTIONS");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/test_support/local_asset_path.rs");
    println!("cargo:rerun-if-changed=build_support/catalog_codegen.rs");
    if env::var_os("CARGO_FEATURE_EMBEDDED_GAME_DATA").is_none() {
        return Ok(());
    }
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    let mut paths = Vec::with_capacity(LOCAL_ASSETS.len());
    let mut missing = false;
    for relative in LOCAL_ASSETS {
        let path = manifest_dir.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let path_string = path.to_string_lossy();
        let description =
            format!("API local asset `{relative}`; generate it with `{GENERATE_GAME_DATA}`");
        match local_asset_path::local_asset_path("", &path_string, &description) {
            Some(path) => paths.push(path),
            None => missing = true,
        }
    }
    if missing {
        println!("cargo:rustc-cfg=no_local_game_data");
        return Ok(());
    }

    let bank_catalog_json = fs::read_to_string(manifest_dir.join(BANK_CATALOG_JSON))
        .map_err(|error| io::Error::new(error.kind(), format!("{BANK_CATALOG_JSON}: {error}")))?;
    let bank_catalog_rust = catalog_codegen::bank_catalog_rust(&bank_catalog_json)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let cook_catalog_json = fs::read_to_string(manifest_dir.join(COOK_CATALOG_JSON))
        .map_err(|error| io::Error::new(error.kind(), format!("{COOK_CATALOG_JSON}: {error}")))?;
    let cook_catalog_rust = catalog_codegen::cook_catalog_rust(&cook_catalog_json)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    fs::write(out.join("bank_catalog.rs"), bank_catalog_rust)?;
    fs::write(out.join("cook_catalog.rs"), cook_catalog_rust)?;
    for (revision, source) in [(274, &paths[0]), (289, &paths[1]), (377, &paths[2])] {
        let mut input = File::open(source)?;
        let output = File::create(out.join(format!("game-data-{revision}.json.gz")))?;
        let mut encoder = GzEncoder::new(output, Compression::best());
        io::copy(&mut input, &mut encoder)?;
        encoder.finish()?;
    }
    Ok(())
}
