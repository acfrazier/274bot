use crate::core::*;
use crate::output::{CLIENT, HOST};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use std::fmt;
use std::path::Path;

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("unique JSON")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Unique, M::Error> {
                let mut out = serde_json::Map::new();
                while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                    if out.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                    out.insert(key, value);
                }
                Ok(Unique(Value::Object(out)))
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<Unique, S::Error> {
                let mut out = vec![];
                while let Some(Unique(v)) = seq.next_element()? {
                    out.push(v);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
pub fn unique(data: &[u8]) -> Result<Value> {
    serde_json::from_slice::<Unique>(data)
        .map(|x| x.0)
        .map_err(|e| {
            if e.to_string().contains("duplicate JSON key") {
                "duplicate JSON key"
            } else {
                "invalid JSON"
            }
        })
}
fn keys(v: &Value, expected: &[&str], reason: &'static str) -> Result<()> {
    let o = v.as_object().ok_or(reason)?;
    need(
        o.len() == expected.len() && expected.iter().all(|k| o.contains_key(*k)),
        reason,
    )
}
pub fn requested(v: &Value) -> Result<Option<u64>> {
    if v.is_null() {
        Ok(None)
    } else {
        Ok(Some(v.as_u64().ok_or("invalid requested time")?))
    }
}
pub fn manifest(path: &Path, expected: &str, portable: bool) -> Result<Value> {
    need(
        expected.len() == 64
            && expected
                .bytes()
                .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')),
        "manifest hash format",
    )?;
    let data = bounded(path, 65536)?;
    need(hash(&data) == expected, "manifest hash mismatch")?;
    let m = unique(&data)?;
    keys(
        &m,
        &[
            "schema",
            "inputs",
            "analysis_path",
            "provenance",
            "suppression_policy",
            "requested_time",
        ],
        "manifest schema fields",
    )?;
    need(m["schema"] == "heaptrack-owner-input/v1", "manifest schema")?;
    keys(
        &m["inputs"],
        &["raw", "interpreted", "peak", "receipt", "stderr"],
        "input roles",
    )?;
    let p = &m["provenance"];
    keys(
        p,
        &[
            "scope",
            "host",
            "client",
            "binary_sha256",
            "capture_tooling",
            "process_role",
            "collection_mode",
            "intentional_pause",
            "reinitialized",
            "capture_status",
            "campaign_capture_status",
        ],
        "provenance schema",
    )?;
    need(
        p["collection_mode"] == "direct-preload"
            && p["intentional_pause"] == false
            && p["reinitialized"] == false,
        "unsupported collection provenance",
    )?;
    need(
        p["campaign_capture_status"] == "failed_raw_size_guard",
        "failed capture boundary required",
    )?;
    need(
        p["capture_tooling"] == "1fe21610b6446779133b5ff0dc229ba93599ecc0",
        "capture tooling identity",
    )?;
    if portable {
        need(
            p["scope"] == "controlled_saved_fixture" && p["process_role"] == "owned_python_fixture",
            "portable fixtures only",
        )?;
    } else {
        need(
            cfg!(target_os = "linux"),
            "Linux required for released executor",
        )?;
        need(
            p["scope"] == "failed_saved_production_prefix" && p["process_role"] == "client_tui",
            "production scope/role",
        )?;
        need(
            p["host"] == HOST && p["client"] == CLIENT,
            "frozen source identity",
        )?;
        need(
            p["binary_sha256"]
                == "a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9",
            "frozen binary identity",
        )?;
        need(
            p["capture_status"] == "failed_raw_size_guard",
            "production capture failure reason",
        )?;
    }
    need(
        m["suppression_policy"] == "unsuppressed_replay_canonical_default_leaks_not_comparable",
        "suppression policy",
    )?;
    requested(&m["requested_time"])?;
    let selector = m["analysis_path"]
        .as_array()
        .ok_or("analysis receipt selector")?;
    need(
        selector.len() <= 8
            && selector
                .iter()
                .all(|v| v.as_str().is_some_and(|s| s.chars().count() <= 80)),
        "analysis receipt selector",
    )?;
    let l = Limits::new(&json!({}))?;
    for (k, e) in m["inputs"].as_object().unwrap() {
        let cap = if matches!(k.as_str(), "receipt" | "stderr") {
            MIB as u64
        } else {
            l.get(k)
        };
        let (path, size, _) = entry(e, cap)?;
        need(
            Path::new(path).is_absolute(),
            "absolute immutable input paths required",
        )?;
        need(!portable || size <= 2_000_000, "portable input cap")?;
    }
    Ok(m)
}
fn bound_entry(e: &Value) -> Result<Vec<u8>> {
    let (p, n, h) = entry(e, MIB as u64)?;
    let b = bounded(Path::new(p), MIB)?;
    need(
        b.len() as u64 == n && hash(&b) == h,
        "receipt/stderr hash mismatch",
    )?;
    Ok(b)
}
pub fn receipt(m: &Value) -> Result<[u64; 3]> {
    let mut r = unique(&bound_entry(&m["inputs"]["receipt"])?)?;
    for k in m["analysis_path"]
        .as_array()
        .ok_or("analysis receipt selector")?
    {
        r = r
            .get(k.as_str().ok_or("analysis receipt selector")?)
            .ok_or("analysis receipt selector")?
            .clone();
    }
    need(
        r["interpreter"]["exit_code"] == 0 && r["printer"]["exit_code"] == 0,
        "saved conversion/printer failed",
    )?;
    let argv = r["printer"]["argv"]
        .as_array()
        .ok_or("printer argv")?
        .iter()
        .map(|x| x.as_str().ok_or("printer argv"))
        .collect::<Result<Vec<_>>>()?;
    need(
        argv.contains(&"--merge-backtraces=0") && argv.contains(&"--flamegraph-cost-type=peak"),
        "canonical printer must be unmerged common-time peak",
    )?;
    need(
        !argv.iter().any(|a| {
            ["--filter", "--shorten-templates", "--flamegraph-cost-type="]
                .iter()
                .any(|p| a.starts_with(p))
                && *a != "--flamegraph-cost-type=peak"
        }),
        "unsupported canonical printer rendering/filter",
    )?;
    need(
        r["interpreter"]["output_size"] == m["inputs"]["interpreted"]["bytes"],
        "interpreter receipt output size",
    )?;
    let b = bound_entry(&m["inputs"]["stderr"])?;
    // The reference bytes regex allows ASCII whitespace around each decimal.
    let mut rest = b
        .strip_prefix(b"heaptrack stats:\n")
        .ok_or("unexpected interpreter stderr (conversion warning/error)")?;
    let mut out = [0; 3];
    for (i, label) in [
        b"allocations:".as_slice(),
        b"leaked allocations:",
        b"temporary allocations:",
    ]
    .iter()
    .enumerate()
    {
        rest = rest.trim_ascii_start();
        rest = rest
            .strip_prefix(*label)
            .ok_or("unexpected interpreter stderr (conversion warning/error)")?
            .trim_ascii_start();
        let n = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        need(
            n > 0,
            "unexpected interpreter stderr (conversion warning/error)",
        )?;
        out[i] = checked(
            text(&rest[..n])?
                .parse()
                .map_err(|_| "signed aggregate overflow/underflow")?,
        )?;
        rest = rest[n..]
            .strip_prefix(b"\n")
            .ok_or("unexpected interpreter stderr (conversion warning/error)")?;
    }
    need(
        rest.is_empty(),
        "unexpected interpreter stderr (conversion warning/error)",
    )?;
    Ok(out)
}
pub fn set_limits(l: &Limits) -> Result<()> {
    fn set(which: libc::c_int, n: u64) -> Result<()> {
        let r = libc::rlimit {
            rlim_cur: n as libc::rlim_t,
            rlim_max: n as libc::rlim_t,
        };
        need(
            unsafe { libc::setrlimit(which as _, &r) } == 0,
            "child rlimit setup",
        )
    }
    set(libc::RLIMIT_CPU as _, 3 * l.get("cpu") + 1)?;
    set(libc::RLIMIT_FSIZE as _, l.get("scratch"))?;
    if cfg!(target_os = "linux") {
        set(libc::RLIMIT_AS as _, l.get("address"))?;
    }
    CAP.store(
        l.get("table_bytes") as usize,
        std::sync::atomic::Ordering::SeqCst,
    );
    need(
        USED.load(std::sync::atomic::Ordering::SeqCst) <= l.get("table_bytes") as usize,
        "table byte cap",
    )
}
