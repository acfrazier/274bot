//! Test-only bounded stdin adapter. No paths or manifests accepted.
mod contract;
mod core;
mod negative;
mod output;
mod replay;
use core::*;
use serde_json::{Value, json};
use std::io::Read;
#[global_allocator]
static ALLOCATOR: Charged = Charged;
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    if let Some(s) = v.as_str() {
        need(
            s.len() % 2 == 0 && s.len() <= 4_000_000,
            "fixture hex byte cap",
        )?;
        return s
            .as_bytes()
            .chunks_exact(2)
            .map(|b| hex(b).map(|n| n as u8))
            .collect();
    }
    v.as_array()
        .ok_or("fixture byte array")?
        .iter()
        .map(|x| {
            x.as_u64()
                .filter(|n| *n < 256)
                .map(|n| n as u8)
                .ok_or("fixture byte")
        })
        .collect()
}
fn run() -> Result<Value> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(12_000_001)
        .read_to_end(&mut input)
        .map_err(|_| "fixture stdin")?;
    need(input.len() <= 12_000_000, "fixture request cap")?;
    let v = contract::unique(&input)?;
    let limits = Limits::new(v.get("limits").unwrap_or(&json!({})))?;
    contract::set_limits(&limits)?;
    let mut g = Guard::new(limits, false);
    g.check()?;
    match v["op"].as_str().ok_or("fixture op")? {
        "table" => {
            let mut tables = replay::Tables::new();
            for item in v["definitions"].as_array().ok_or("fixture definitions")? {
                let line = bytes(item)?;
                let record = parse(&line, false, g.limits.get("line") as usize)?;
                tables.define(&record, &mut g)?;
            }
            let trace = v["trace"].as_u64().ok_or("fixture trace")?;
            let stack = tables.stack(trace, &mut g)?;
            let canonical = replay::canonical(&tables, trace, &mut g)?;
            let class = output::classify(&tables, trace, v["frozen"] == true, &mut g)?;
            g.check()?;
            Ok(json!({"stack":stack,"canonical":canonical,"classification":class}))
        }
        "pretty" => {
            let data = bytes(&v["text"])?;
            Ok(json!(replay::pretty(&data, &mut g)?))
        }
        "parse" => {
            let b = bytes(&v["line"])?;
            let r = parse(
                &b,
                v["raw"].as_bool().unwrap_or(false),
                g.limits.get("line") as usize,
            )?;
            Ok(json!({"op":(r.op as char).to_string(),"nums":r.nums,"bytes":r.bytes}))
        }
        "analyze" => {
            let path = std::env::temp_dir()
                .join(format!("heaptrack-native-fixture-{}", std::process::id()));
            std::fs::create_dir(&path).map_err(|_| "fixture fresh directory")?;
            let scratch = Scratch(path);
            let mut entries = json!({});
            let mut total = 0usize;
            for kind in ["raw", "interpreted", "peak"] {
                let b = bytes(&v[kind])?;
                total = total.checked_add(b.len()).ok_or("fixture cap")?;
                need(b.len() <= 2_000_000 && total <= 6_000_000, "fixture cap")?;
                let p = scratch.0.join(kind);
                std::fs::write(&p, &b).map_err(|_| "fixture write")?;
                entries[kind] = json!({"path":p,"bytes":b.len(),"sha256":hash(&b)});
            }
            let a = replay::analyze(&entries, contract::requested(&v["requested_time"])?, &mut g)?;
            let snapshots: serde_json::Map<String, Value> = a
                .snapshots
                .iter()
                .map(|(k, s)| (k.clone(), s.value()))
                .collect();
            let mut files = json!({});
            if v["outputs"] == true {
                let path = scratch.0.join("output");
                std::fs::create_dir(&path).map_err(|_| "fixture output directory")?;
                output::write_outputs(
                    &a,
                    &path,
                    json!({"host":output::HOST,"client":output::CLIENT}),
                    json!({}),
                    &mut g,
                )?;
                for item in std::fs::read_dir(&path).map_err(|_| "fixture output list")? {
                    let item = item.map_err(|_| "fixture output entry")?;
                    let name = item
                        .file_name()
                        .into_string()
                        .map_err(|_| "fixture output name")?;
                    files[&name] = json!(
                        String::from_utf8(bounded(&item.path(), 32 * MIB)?)
                            .map_err(|_| "fixture output UTF8")?
                    );
                }
            }
            g.next(4)?;
            Ok(
                json!({"first":a.first,"raw":a.raw,"snapshots":snapshots,"canonical":a.comparison,"canonical_equal":true,"measurements":g.measurements,"files":files}),
            )
        }
        _ => Err("fixture op"),
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.is_empty() {
        run()
    } else if args.len() == 2 && args[0] == "--guard" {
        negative::run(&args[1])
    } else {
        Err("fixture arguments")
    };
    match result {
        Ok(v) => {
            let _ = send(&v);
        }
        Err(e) => {
            let _ = send(&json!({"error":e}));
            std::process::exit(1);
        }
    }
}
