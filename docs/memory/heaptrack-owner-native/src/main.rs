mod contract;
mod core;
mod output;
mod replay;

use core::*;
use serde_json::{Value, json};
use std::path::Path;

#[global_allocator]
static ALLOCATOR: core::Charged = core::Charged;

fn main() {
    if let Err(e) = run() {
        let _ = send(&json!({"error":e}));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut values = std::collections::BTreeMap::new();
    let mut portable = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--portable-fixture" {
            need(!portable, "duplicate argument")?;
            portable = true;
            i += 1;
        } else {
            need(
                matches!(
                    args[i].as_str(),
                    "--manifest" | "--manifest-sha256" | "--output" | "--limits"
                ),
                "unknown argument",
            )?;
            need(i + 1 < args.len(), "missing argument")?;
            need(
                values
                    .insert(args[i].as_str(), args[i + 1].as_str())
                    .is_none(),
                "duplicate argument",
            )?;
            i += 2;
        }
    }
    let manifest = values.get("--manifest").ok_or("missing manifest")?;
    let expected = values
        .get("--manifest-sha256")
        .ok_or("missing manifest hash")?;
    let root = Path::new(values.get("--output").ok_or("missing output")?).join(".pending");
    let limits = Limits::new(&contract::unique(
        values.get("--limits").unwrap_or(&"{}").as_bytes(),
    )?)?;
    contract::set_limits(&limits)?;
    let mut g = Guard::new(limits, true);
    let result = (|| {
        g.check()?;
        let m = contract::manifest(Path::new(manifest), expected, portable)?;
        let stats = contract::receipt(&m)?;
        let a = replay::analyze(
            &m["inputs"],
            contract::requested(&m["requested_time"])?,
            &mut g,
        )?;
        need(
            [
                a.raw["allocations"].as_u64().unwrap(),
                a.raw["count"].as_u64().unwrap(),
                a.raw["temporary"].as_u64().unwrap(),
            ] == stats,
            "interpreter stats mismatch",
        )?;
        let mut provenance = m["provenance"].clone();
        provenance["manifest_sha256"] = json!(expected);
        let mut hashes = serde_json::Map::new();
        for (k, e) in m["inputs"].as_object().unwrap() {
            hashes.insert(k.clone(), json!({"bytes":e["bytes"],"sha256":e["sha256"]}));
        }
        provenance["input_hashes"] = Value::Object(hashes);
        provenance["requested_time"] = m["requested_time"].clone();
        provenance["suppression_policy"] = m["suppression_policy"].clone();
        let resources = json!({"linux_release":!portable,"qualification":if portable{"portable_fixture_only"}else{"requires_root_native_guard_smoke"},"completed_passes":g.measurements,"child_pid":std::process::id()});
        output::write_outputs(&a, &root, provenance, resources, &mut g)?;
        g.next(4)?;
        g.check()?;
        send(&json!({"done":true,"measurements":g.measurements}))
    })();
    if let Err(e) = result {
        send(
            &json!({"error":e,"progress":{"line":g.line,"offset":g.offset,"records":g.records,"cpu_s":cpu()-g.start_cpu,"wall_s":mono()-g.start}}),
        )?;
        std::process::exit(1);
    }
    Ok(())
}
