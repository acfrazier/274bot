use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[path = "build_support/catalog_codegen.rs"]
mod catalog_codegen;
#[path = "src/test_support/local_asset_path.rs"]
mod local_asset_path;
#[path = "src/test_support/local_asset_path_377.rs"]
mod local_asset_path_377;

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

const GENERATE_GAME_DATA: &str = "python tools/local-data/build.py";
const BANK_CATALOG_JSON: &str = "data/game-data/bank-catalog.json";
const COOK_CATALOG_JSON: &str = "data/game-data/cook-catalog.json";
const GATHER_CATALOG_JSON: &str = "data/game-data/gather-catalog.json";
const LOCAL_ASSETS: &[&str] = &[
    "data/game-data/274.json",
    "data/game-data/289.json",
    "data/game-data/274/debug.json",
    "data/game-data/289/debug.json",
    "data/game-data/274/quests.json",
    "data/game-data/289/quests.json",
    "data/game-data/274/gathering.json",
    "data/game-data/289/gathering.json",
    "data/game-data/289/prereq_steps.json",
    BANK_CATALOG_JSON,
    COOK_CATALOG_JSON,
    GATHER_CATALOG_JSON,
];
const LOCAL_377_ASSETS: &[&str] = &[
    "data/game-data/377.json",
    "data/game-data/377/debug.json",
    "data/game-data/377/quests.json",
    "data/game-data/377/gathering.json",
];

struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes: u64,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
}

fn sha256_hex(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut result = String::with_capacity(64);
    for byte in digest.as_ref() {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

fn write_game_data_pack_identities(
    out: &Path,
    packs: &[(i32, String, String, u64)],
) -> io::Result<()> {
    let mut generated = String::from("&[\n");
    for (revision, path, sha256, bytes) in packs {
        generated.push_str(&format!(
            "    GameDataPackIdentity {{ revision: {revision}, path: {path:?}, sha256: {sha256:?}, bytes: {bytes} }},\n"
        ));
    }
    generated.push_str("]\n");
    fs::write(out.join("game_data_pack_identities.rs"), generated)
}

fn main() -> io::Result<()> {
    println!("cargo:rustc-check-cfg=cfg(no_local_game_data)");
    println!("cargo:rustc-check-cfg=cfg(no_local_game_data_377)");
    println!("cargo:rerun-if-env-changed=GITHUB_ACTIONS");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/catalog_codegen.rs");
    println!("cargo:rerun-if-changed=src/test_support/local_asset_path.rs");
    println!("cargo:rerun-if-changed=src/test_support/local_asset_path_377.rs");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    fs::write(out.join("game_data_pack_identities.rs"), "&[]\n")?;
    if env::var_os("CARGO_FEATURE_EMBEDDED_GAME_DATA").is_none() {
        return Ok(());
    }

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
    let mut paths_377 = Vec::with_capacity(LOCAL_377_ASSETS.len());
    let mut missing_377 = false;
    for relative in LOCAL_377_ASSETS {
        let path = manifest_dir.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let path_string = path.to_string_lossy();
        let description = format!("API revision-377 local asset `{relative}`");
        match local_asset_path_377::local_asset_path_377("", &path_string, &description) {
            Some(path) => paths_377.push(path),
            None => missing_377 = true,
        }
    }
    if missing_377 {
        println!("cargo:rustc-cfg=no_local_game_data_377");
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

    let gather_catalog_json = fs::read_to_string(manifest_dir.join(GATHER_CATALOG_JSON))
        .map_err(|error| io::Error::new(error.kind(), format!("{GATHER_CATALOG_JSON}: {error}")))?;
    let gather_catalog_rust = catalog_codegen::gather_catalog_rust(&gather_catalog_json)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    fs::write(out.join("bank_catalog.rs"), bank_catalog_rust)?;
    fs::write(out.join("cook_catalog.rs"), cook_catalog_rust)?;
    fs::write(out.join("gather_camps.rs"), gather_catalog_rust.camps)?;
    fs::write(out.join("fishing_gear.rs"), gather_catalog_rust.fishing)?;
    fs::write(
        out.join("fish_camp_cook_plans.rs"),
        gather_catalog_rust.cook,
    )?;
    fs::write(out.join("quest_junk.rs"), gather_catalog_rust.quest_junk)?;

    let source_377 = (!missing_377).then(|| (377, &paths_377[0]));
    let mut pack_identities = Vec::with_capacity(3);
    for (revision, source) in [(274, &paths[0]), (289, &paths[1])]
        .into_iter()
        .chain(source_377)
    {
        let mut input = HashingReader {
            inner: File::open(source)?,
            hasher: Sha256::new(),
            bytes: 0,
        };
        let output = File::create(out.join(format!("game-data-{revision}.json.gz")))?;
        let mut encoder = GzEncoder::new(output, Compression::best());
        io::copy(&mut input, &mut encoder)?;
        encoder.finish()?;
        pack_identities.push((
            revision,
            format!("data/game-data/{revision}.json"),
            sha256_hex(input.hasher.finalize()),
            input.bytes,
        ));
    }
    write_game_data_pack_identities(&out, &pack_identities)?;
    Ok(())
}
