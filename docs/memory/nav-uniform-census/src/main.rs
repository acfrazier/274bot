use nav::pack::decode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

const CAP: u64 = 128 * 1024 * 1024;
const LEVELS: usize = 4;
const TILE: usize = 32;
const HIST: usize = 512;

#[derive(Serialize)]
struct Level {
    level: usize,
    tiles: u64,
    uniform_tiles: u64,
    dense_tiles: u64,
}
#[derive(Serialize)]
struct Output {
    status: &'static str,
    input: Identity,
    source: Source,
    actual: Actual,
    levels: Vec<Level>,
    uniform_pair_histogram: Vec<u64>,
    accounting: Accounting,
    limits: Limits,
    interpretation: Interpretation,
}
#[derive(Serialize)]
struct Identity {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize)]
struct Source {
    host: &'static str,
    client: &'static str,
    frozen_host: &'static str,
    frozen_client: &'static str,
    source_manifest: &'static str,
    decoder: &'static str,
    tool: &'static str,
}
#[derive(Serialize)]
struct Actual {
    walk_len: u64,
    walk_capacity: u64,
    walk_element_bytes: usize,
    blocked_len: u64,
    blocked_capacity: u64,
    blocked_element_bytes: usize,
    width: usize,
    height: usize,
    origin: WorldTileJson,
    flags_present: bool,
    graph_edges: u64,
    graph_teleports: u64,
    bank_count: u64,
}
#[derive(Serialize)]
struct WorldTileJson {
    x: i32,
    z: i32,
    level: i32,
}
#[derive(Serialize)]
struct Accounting {
    resident_element_bytes: u64,
    tile_count: u64,
    nonuniform_tiles: u64,
    hypothetical_element_storage_estimate: u64,
    potential_element_reduction_estimate: i128,
    materiality_bytes: u64,
}
#[derive(Serialize)]
struct Limits {
    input_cap_bytes: u64,
    histogram_entries: usize,
    tile_width: usize,
    levels: usize,
}
#[derive(Serialize)]
struct Interpretation {
    hypothetical_element_storage_estimate: bool,
    not_actual_candidate_allocation: bool,
    not_rss: bool,
    no_acceptance: bool,
    omitted: [&'static str; 7],
}

fn checked(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b).ok_or("arithmetic overflow".into())
}
fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: nav-uniform-census INPUT")?;
    let p = Path::new(&path);
    let meta = std::fs::symlink_metadata(p).map_err(|_| "input stat failed")?;
    if !meta.file_type().is_file() || meta.len() > CAP {
        return Err("input must be regular, non-symlink, <=128MiB".into());
    }
    let mut f = File::open(p).map_err(|_| "input open failed")?;
    let mut bytes = Vec::with_capacity(meta.len().try_into().map_err(|_| "input too large")?);
    f.read_to_end(&mut bytes).map_err(|_| "input read failed")?;
    if bytes.len() as u64 != meta.len() || bytes.len() as u64 > CAP {
        return Err("input changed or cap breached".into());
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let (collision, graph, banks) =
        decode(&bytes).map_err(|e| format!("decoder failure: {e:?}"))?;
    let plane = collision
        .width
        .checked_mul(collision.height)
        .ok_or("arithmetic overflow")?;
    let mut histogram = vec![0u64; HIST];
    let mut levels = Vec::with_capacity(LEVELS);
    let mut dense: u64 = 0;
    let mut uniform: u64 = 0;
    for level in 0..LEVELS {
        let mut tiles: u64 = 0;
        let mut un: u64 = 0;
        let tz = collision.height.div_ceil(TILE);
        let tx = collision.width.div_ceil(TILE);
        for z0 in (0..collision.height).step_by(TILE) {
            for x0 in (0..collision.width).step_by(TILE) {
                tiles = checked(tiles, 1)?;
                let mut first = None;
                let mut same = true;
                for z in z0..(z0 + TILE).min(collision.height) {
                    for x in x0..(x0 + TILE).min(collision.width) {
                        let i = level
                            .checked_mul(plane)
                            .and_then(|v| v.checked_add(z * collision.width + x))
                            .ok_or("arithmetic overflow")?;
                        let pair = (collision.walk[i] as usize)
                            | (((collision.blocked[i >> 6] >> (i & 63)) & 1) as usize) << 8;
                        histogram[pair] = checked(histogram[pair], 1)?;
                        if let Some(v) = first {
                            same &= v == pair;
                        } else {
                            first = Some(pair);
                        }
                    }
                }
                if same {
                    un = checked(un, 1)?;
                    uniform = checked(uniform, 1)?;
                } else {
                    dense = checked(dense, 1)?;
                }
            }
        }
        let _ = (tx, tz);
        levels.push(Level {
            level,
            tiles,
            uniform_tiles: un,
            dense_tiles: tiles - un,
        });
    }
    let t = (collision.width.div_ceil(TILE) as u64)
        .checked_mul(collision.height.div_ceil(TILE) as u64)
        .and_then(|v| v.checked_mul(4))
        .ok_or("arithmetic overflow")?;
    let a = (collision.walk.capacity() as u64)
        .checked_add(
            (collision.blocked.capacity() as u64)
                .checked_mul(8)
                .ok_or("arithmetic overflow")?,
        )
        .ok_or("arithmetic overflow")?;
    let candidate = t
        .checked_mul(8)
        .and_then(|v| v.checked_add(dense.checked_mul(1152)?))
        .ok_or("arithmetic overflow")?;
    let signed = i128::from(a)
        .checked_sub(i128::from(candidate))
        .ok_or("signed arithmetic overflow")?;
    Ok(println!(
        "{}",
        serde_json::to_string_pretty(&Output {
            status: "ok",
            input: Identity {
                path,
                bytes: bytes.len() as u64,
                sha256: hash
            },
            source: Source {
                host: env!("NAV_CENSUS_HOST"),
                client: env!("NAV_CENSUS_CLIENT"),
                frozen_host: env!("NAV_CENSUS_FROZEN_HOST"),
                frozen_client: env!("NAV_CENSUS_FROZEN_CLIENT"),
                source_manifest: env!("NAV_CENSUS_SOURCE_MANIFEST"),
                decoder: "nav::pack::decode",
                tool: "nav-uniform-census"
            },
            actual: Actual {
                walk_len: collision.walk.len() as u64,
                walk_capacity: collision.walk.capacity() as u64,
                walk_element_bytes: std::mem::size_of::<u8>(),
                blocked_len: collision.blocked.len() as u64,
                blocked_capacity: collision.blocked.capacity() as u64,
                blocked_element_bytes: std::mem::size_of::<u64>(),
                width: collision.width,
                height: collision.height,
                origin: WorldTileJson {
                    x: collision.origin.x,
                    z: collision.origin.z,
                    level: collision.origin.level
                },
                flags_present: collision.flags.is_some(),
                graph_edges: graph.edges.len() as u64,
                graph_teleports: graph.teleports.len() as u64,
                bank_count: banks.len() as u64
            },
            levels,
            uniform_pair_histogram: histogram,
            accounting: Accounting {
                resident_element_bytes: a,
                tile_count: t,
                nonuniform_tiles: dense,
                hypothetical_element_storage_estimate: candidate,
                potential_element_reduction_estimate: signed,
                materiality_bytes: 16 * 1024 * 1024
            },
            limits: Limits {
                input_cap_bytes: CAP,
                histogram_entries: HIST,
                tile_width: TILE,
                levels: LEVELS
            },
            interpretation: Interpretation {
                hypothetical_element_storage_estimate: true,
                not_actual_candidate_allocation: true,
                not_rss: true,
                no_acceptance: true,
                omitted: [
                    "headers",
                    "allocator rounding",
                    "conversion peak",
                    "input buffer",
                    "graph/bank/page costs",
                    "public consumer audit",
                    "CPU/router costs"
                ]
            }
        })
        .map_err(|_| "json serialization failed")?
    ))
}
