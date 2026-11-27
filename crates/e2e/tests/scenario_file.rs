#![cfg(feature = "scenario-luau")]

//! Generic headless runner for Luau File scenarios.
//!
//! Run with `LIVE=1` and either `SCENARIO_FILE=<path>` or `SCENARIO_ID=<id>`
//! (a registry id implemented as a File scenario), plus the selected
//! profile's normal environment. Optional inputs are `SCENARIO_CASE`,
//! `SCENARIO_EXPECT_ENGINE`, `SCENARIO_EXPECT_CONTENT`, `SCENARIO_EXPECT_CRC`,
//! `SCENARIO_SEED_PACK`, `SCENARIO_SEED_PACK_GRANT`, `SCENARIO_EVIDENCE`, and
//! `SCENARIO_EXPECT_OUTCOME` (`PASS` by default; `FAIL` for a negative
//! lifecycle trace whose roles must all resolve).

mod common;

use std::path::PathBuf;

use common::file_scenario::{evidence_path, run as run_file_scenario};
use host_play::scenario_luau::{FileOptions, FileSource};

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn file_source() -> Result<FileSource, String> {
    match (env_path("SCENARIO_FILE"), env_text("SCENARIO_ID")) {
        (Some(path), None) => Ok(FileSource::Path(
            std::path::absolute(path).map_err(|err| err.to_string())?,
        )),
        (None, Some(id)) => FileSource::registered(&id)
            .ok_or_else(|| format!("SCENARIO_ID {id:?} is not a registered File scenario")),
        _ => Err("set exactly one of SCENARIO_FILE and SCENARIO_ID".into()),
    }
}

fn file_options() -> Result<FileOptions, String> {
    Ok(FileOptions {
        source: file_source()?,
        case: env_text("SCENARIO_CASE"),
        evidence: Some(evidence_path()),
        expect_engine: env_text("SCENARIO_EXPECT_ENGINE"),
        expect_content: env_text("SCENARIO_EXPECT_CONTENT"),
        expect_crc: env_text("SCENARIO_EXPECT_CRC"),
        seed_pack: env_path("SCENARIO_SEED_PACK"),
        seed_pack_grant: env_text("SCENARIO_SEED_PACK_GRANT"),
    })
}

#[test]
#[ignore = "requires a selected Local profile and a live engine"]
fn scenario_file() -> Result<(), String> {
    let options = file_options()?;
    let expect_fail = match env_text("SCENARIO_EXPECT_OUTCOME").as_deref() {
        None | Some("PASS") => false,
        Some("FAIL") => true,
        Some(other) => return Err(format!("SCENARIO_EXPECT_OUTCOME: unknown {other:?}")),
    };
    run_file_scenario(options, expect_fail)
}
