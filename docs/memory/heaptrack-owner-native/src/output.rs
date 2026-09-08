use crate::core::*;
use crate::replay::{Analysis, NEW, Tables};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::Ordering;

pub const HOST: &str = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf";
pub const CLIENT: &str = "3456edc8dabf7b25ada78110ffa56327af9f67a4";
const RULES: [(&str, &str, u64, u64, &str, &str, &str); 3] = [
    (
        "nav::world::NavWorld::load_pack",
        "crates/nav/src/world.rs",
        49,
        56,
        "nav_world_startup",
        "process_shared_or_transient",
        HOST,
    ),
    (
        "client::client::client::Client::from_shared",
        "crates/client/src/client/client.rs",
        300,
        948,
        "client_cache_interface_world",
        "mixed_shared_private",
        CLIENT,
    ),
    (
        "client::core::world::World::new",
        "crates/client/src/core/world.rs",
        30,
        130,
        "client_world",
        "source_per_client",
        CLIENT,
    ),
];
const DOMAINS: [(&str, &str, &str, &str, &str); 4] = [
    (
        "crates/api/src/snapshot.rs",
        "game_snapshot_families",
        "unknown",
        HOST,
        "532-687",
    ),
    (
        "crates/script/src/isolate_fb.rs",
        "script_fingerprint_encoded_buffers",
        "unknown",
        HOST,
        "1386-1448",
    ),
    (
        "crates/script/src/slot.rs",
        "script_slot",
        "unknown",
        HOST,
        "26-52",
    ),
    (
        "crates/host-play/src/memory.rs",
        "harness_infrastructure",
        "process_infrastructure",
        HOST,
        "1014-1078",
    ),
];
fn path_match(file: &str, path: &str) -> bool {
    file == path || file.strip_suffix(path).is_some_and(|s| s.ends_with('/'))
}
// Insert fields in Python's order: family IDs hash this exact compact JSON.
fn classification(
    symbol: &str,
    file: &str,
    line: u64,
    domain: &str,
    ownership: &str,
    anchor: Value,
    unknown: bool,
) -> Value {
    let mut v = serde_json::Map::new();
    for (k, x) in [
        ("symbol", json!(symbol)),
        ("file", json!(file)),
        ("line", json!(line)),
        ("domain", json!(domain)),
        ("ownership", json!(ownership)),
        ("anchor", anchor),
        ("unknown_symbol", json!(unknown)),
    ] {
        v.insert(k.into(), x);
    }
    Value::Object(v)
}
fn anchor(commit: &str, path: &str, lines: String, domain: bool) -> Value {
    let mut v = serde_json::Map::new();
    v.insert("commit".into(), json!(commit));
    v.insert("path".into(), json!(path));
    v.insert("lines".into(), json!(lines));
    if domain {
        v.insert(
            "mapping".into(),
            json!("domain_only_not_exact_allocation_owner"),
        );
    }
    Value::Object(v)
}
pub fn classify(t: &Tables, trace: u64, frozen: bool, g: &mut Guard) -> Result<Value> {
    let frames = t.stack(trace, g)?;
    let mut unknown = frames.is_empty();
    for id in &frames {
        let ip = t.ip(*id)?;
        unknown |= ip.len() < 3;
        for i in (2..ip.len()).step_by(3) {
            g.pulse(0)?;
            unknown |= t.string(ip[i])?.is_empty();
        }
    }
    let mut fallback = None;
    for id in frames {
        let ip = t.ip(id)?;
        for i in (2..ip.len()).step_by(3) {
            g.pulse(0)?;
            let fun = text(t.string(ip[i])?)?;
            let file = if i + 1 < ip.len() {
                text(t.string(ip[i + 1])?)?
            } else {
                ""
            };
            let line = if i + 2 < ip.len() { ip[i + 2] } else { 0 };
            if fun.is_empty() {
                continue;
            }
            if fallback.is_none() && !NEW.contains(&fun.as_bytes()) {
                fallback = Some((fun, file, line));
            }
            if ![
                "nav::",
                "client::",
                "api::",
                "host_play::",
                "script::",
                "tui_play::",
            ]
            .iter()
            .any(|p| fun.starts_with(p))
            {
                continue;
            }
            let (mut domain, mut ownership, mut a) = ("unresolved_source", "unknown", Value::Null);
            if frozen {
                for (symbol, path, start, end, owner, boundary, commit) in RULES {
                    if (fun == symbol
                        || fun
                            .strip_prefix(symbol)
                            .is_some_and(|s| s.starts_with("::h")))
                        && path_match(file, path)
                        && (start..=end).contains(&line)
                    {
                        domain = owner;
                        ownership = boundary;
                        a = anchor(commit, path, format!("{start}-{end}"), false);
                        break;
                    }
                }
                if a.is_null() {
                    for (path, owner, boundary, commit, lines) in DOMAINS {
                        if path_match(file, path) {
                            domain = owner;
                            ownership = boundary;
                            a = anchor(commit, path, lines.into(), true);
                            break;
                        }
                    }
                }
            }
            return Ok(classification(
                fun, file, line, domain, ownership, a, unknown,
            ));
        }
    }
    let (fun, file, line) = fallback.unwrap_or(("<unresolved>", "", 0));
    Ok(classification(
        fun,
        file,
        line,
        "allocator_or_native_boundary",
        "unknown",
        Value::Null,
        unknown,
    ))
}
pub fn cell(v: &Value) -> String {
    serde_json::to_string(v).unwrap()
}
struct Output<'a> {
    root: &'a Path,
    total: u64,
    files: serde_json::Map<String, Value>,
}
impl<'a> Output<'a> {
    fn write<I: IntoIterator<Item = String>>(
        &mut self,
        name: &str,
        rows: I,
        g: &mut Guard,
    ) -> Result<()> {
        g.check()?;
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.root.join(name))
            .map_err(|_| "output open")?;
        let mut h = Sha256::new();
        let mut size = 0;
        for row in rows {
            let b = row.as_bytes();
            self.total = add(self.total, b.len() as u64)?;
            size = add(size, b.len() as u64)?;
            need(self.total <= g.limits.get("output"), "output cap")?;
            need(self.total <= g.limits.get("scratch"), "scratch cap")?;
            for chunk in b.chunks(65536) {
                g.pulse(chunk.len())?;
                f.write_all(chunk).map_err(|_| "output write/scratch cap")?;
                h.update(chunk);
            }
        }
        g.check()?;
        self.files
            .insert(name.into(), json!({"bytes":size,"sha256":digest(h)}));
        Ok(())
    }
}
pub fn write_outputs(
    a: &Analysis,
    root: &Path,
    provenance: Value,
    resources: Value,
    g: &mut Guard,
) -> Result<()> {
    g.check()?;
    let t = &a.tables;
    let first = &a.first;
    let frozen = provenance["host"] == HOST && provenance["client"] == CLIENT;
    let (mut traces, mut ips, mut strings) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::from([0]));
    let mut families = BTreeMap::new();
    let mut membership = BTreeMap::new();
    for snap in a.snapshots.values() {
        for trace in snap.rows.keys() {
            g.pulse(0)?;
            if membership.contains_key(trace) {
                continue;
            }
            let mut value = classify(t, *trace, frozen, g)?;
            let unknown = value
                .as_object_mut()
                .unwrap()
                .shift_remove("unknown_symbol")
                .unwrap()
                .as_bool()
                .unwrap();
            let family = hash(cell(&value).as_bytes());
            families.insert(family.clone(), value);
            membership.insert(*trace, (family, unknown));
            let mut node = *trace;
            let mut depth = 0;
            while node != 0 {
                g.pulse(0)?;
                depth += 1;
                need(depth <= g.limits.get("depth"), "stack depth cap")?;
                traces.insert(node);
                let [ip, parent] = t.traces[node as usize];
                ips.insert(ip);
                node = parent;
            }
        }
    }
    for index in &ips {
        let ip = t.ip(*index)?;
        if !ip.is_empty() {
            strings.insert(ip[1]);
        }
        for i in (2..ip.len()).step_by(3) {
            g.pulse(0)?;
            strings.insert(ip[i]);
            if i + 1 < ip.len() {
                strings.insert(ip[i + 1]);
            }
        }
    }
    let mut out = Output {
        root,
        total: 0,
        files: serde_json::Map::new(),
    };
    let mut rows = vec!["string_id\tutf8_bytes\ttext_json\n".into()];
    for index in strings {
        g.pulse(0)?;
        let s = t.string(index)?;
        rows.push(format!(
            "{index}\t{}\t{}\n",
            s.len(),
            cell(&json!(text(s)?))
        ));
    }
    out.write("strings.tsv", rows, g)?;
    let mut rows = vec!["ip_id\tfields_hex_json\n".into()];
    for index in ips {
        g.pulse(0)?;
        rows.push(format!(
            "{index}\t{}\n",
            cell(&json!(
                t.ip(index)?
                    .iter()
                    .map(|n| format!("{n:x}"))
                    .collect::<Vec<_>>()
            ))
        ));
    }
    out.write("ips.tsv", rows, g)?;
    out.write(
        "traces.tsv",
        std::iter::once("trace_id\tip_id\tparent_trace_id\n".into()).chain(traces.iter().map(
            |n| {
                format!(
                    "{n}\t{}\t{}\n",
                    t.traces[*n as usize][0], t.traces[*n as usize][1]
                )
            },
        )),
        g,
    )?;
    out.write(
        "families.tsv",
        std::iter::once("family_id\tprovenance_json\n".into()).chain(
            families
                .iter()
                .map(|(id, v)| format!("{id}\t{}\n", cell(v))),
        ),
        g,
    )?;
    let mut cutoffs = serde_json::Map::new();
    for (name, snap) in &a.snapshots {
        let mut costs = BTreeMap::<String, [u64; 2]>::new();
        let (mut unknown_bytes, mut unknown_symbols) = (0, 0);
        for (trace, row) in &snap.rows {
            g.pulse(0)?;
            let (family, unknown) = &membership[trace];
            let f = costs.entry(family.clone()).or_default();
            f[0] = add(f[0], row[0])?;
            f[1] = add(f[1], row[1])?;
            if families[family]["ownership"] == "unknown" {
                unknown_bytes = add(unknown_bytes, row[0])?;
            }
            if *unknown {
                unknown_symbols = add(unknown_symbols, row[0])?;
            }
        }
        let mut ordered: Vec<_> = snap.rows.iter().collect();
        guarded_sort(
            &mut ordered,
            |a, b| b.1[0].cmp(&a.1[0]).then(a.0.cmp(b.0)),
            g,
        )?;
        g.check()?;
        out.write(
            &format!("{name}-stacks.tsv"),
            std::iter::once(
                "trace_id\tfamily_id\trequested_bytes\tlive_count\tallocation_calls_full_trace\n"
                    .into(),
            )
            .chain(ordered.iter().map(|(trace, row)| {
                format!(
                    "{trace}\t{}\t{}\t{}\t{}\n",
                    membership[*trace].0, row[0], row[1], row[2]
                )
            })),
            g,
        )?;
        let mut ordered: Vec<_> = costs.iter().collect();
        guarded_sort(
            &mut ordered,
            |a, b| b.1[0].cmp(&a.1[0]).then(a.0.cmp(b.0)),
            g,
        )?;
        g.check()?;
        out.write(
            &format!("{name}-families.tsv"),
            std::iter::once("family_id\trequested_bytes\tlive_count\n".into()).chain(
                ordered
                    .iter()
                    .map(|(id, row)| format!("{id}\t{}\t{}\n", row[0], row[1])),
            ),
            g,
        )?;
        let mut ordered: Vec<_> = snap.descriptors.iter().collect();
        guarded_sort(
            &mut ordered,
            |a, b| b[5].cmp(&a[5]).then(a[0].cmp(&b[0])),
            g,
        )?;
        g.check()?;
        out.write(
            &format!("{name}-descriptors.tsv"),
            std::iter::once(
                "descriptor_id\ttrace_id\tsize\tlive_count\tallocation_calls\trequested_bytes\n"
                    .into(),
            )
            .chain(ordered.iter().map(|r| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    r[0], r[1], r[2], r[3], r[4], r[5]
                )
            })),
            g,
        )?;
        let mut sum = 0;
        for row in costs.values() {
            g.pulse(0)?;
            sum = add(sum, row[0])?;
        }
        need(sum == snap.bytes, "family accounting")?;
        let mark = &first[name];
        let null = Value::Null;
        cutoffs.insert(name.clone(), json!({
            "kind":match name.as_str(){"peak"=>"first_common_time_global_peak_validation","last_mark"=>"last_actual_timestamp_prefix","eof"=>"captured_prefix_end",_=>"requested_mark_prefix"},
            "actual_mark":if name=="last_mark"||name=="requested"{mark}else{&null},
            "position":if name=="eof"{&first["eof"]}else{mark},
            "next_mark":if name=="requested"{&first["next_mark"]}else{&null},
            "last_observed_mark":if name=="eof"{&first["last_mark"]}else{&null},
            "event_tail_time_unbounded":name=="eof"&&!first["last_event"].is_null()&&first["last_event"]["line"].as_u64()>first["last_mark"]["line"].as_u64(),
            "requested_time":if name=="requested"{&provenance["requested_time"]}else{&null},
            "bytes":snap.bytes,"count":snap.count,"unknown_ownership_bytes":unknown_bytes,"unknown_symbol_bytes":unknown_symbols
        }));
    }
    let suppressions = t
        .suppressions
        .iter()
        .map(|s| text(s))
        .collect::<Result<Vec<_>>>()?;
    let receipt = json!({
        "schema":"heaptrack-owner-replay/v1","status":"validated_prefix_diagnostic","capture_complete":false,"acceptance":false,
        "population":"recorded_tracked_requested_bytes","provenance":provenance,"raw":a.raw,"interpreted":first,"cutoffs":cutoffs,
        "canonical_peak":a.comparison,"canonical_full_positive_multiset_equal":true,
        "leak_comparison":"unsuppressed_replay_only_no_exact_canonical_leak_oracle","suppressions":suppressions,"suppressions_applied":false,
        "tables":{"strings":t.strings.offsets.len()-1,"ips":t.ips.offsets.len()-1,"traces":t.traces.len()-1,"descriptors":t.desc.len(),"high_charged_bytes":HIGH.load(Ordering::SeqCst)},
        "resources":resources,"limits":g.limits.0,"files":out.files,
        "limitations":["Capture FAILED; missing observe-end/Stop/post-join is not repaired by replay.","Unknown frees and unrecorded/realloc-to-zero events remain missing coverage.","Hashes do not detect pre-manifest whole-record suffix loss; EOF is not shutdown.","Descriptor IDs are not pointers, instances, ages, snapshots or publication epochs.","Requested bytes are not RSS, allocator retained pages, V8/pool/mmap/GPU inventory.","Source domains are not per-instance ownership, private N1 attribution or N16 scaling."]
    });
    out.write(
        "receipt.json",
        [serde_json::to_string_pretty(&receipt).unwrap() + "\n"],
        g,
    )?;
    g.check()
}
