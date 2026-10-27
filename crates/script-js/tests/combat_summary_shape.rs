//! The `api.combat.fight` summary the script receives (PVP-DESIGN-1026 §9.6).
//!
//! The `fought` terminal is the host's `CombatSummary` turned into one JSON
//! value and handed to V8 by the shared `Outcome::Done` path (`serde_v8`), the
//! same path every machine family uses. V1 added `pvpSource` to it, so this
//! pins two properties the lab memory A/B relies on:
//!
//! 1. every summary has the same own keys in the same order, whether
//!    `pvpSource` is `null` or a source (the field is never added
//!    conditionally), and the envelope the script sees has fast properties;
//! 2. (The V8 used-heap bound for 1 000 retained summaries is the unit test
//!    `materialize::machine_v8::tests`, which measures one isolate directly.)
//!
//! Design/code difference: §9.6 expects the summary to be "built from a cached
//! shape" with fast properties. The shared path builds `value` and `report` as
//! null-prototype dictionary-mode objects (printed below as `dictionary`), for
//! every family. This test therefore does not require fast properties on
//! them; see the V1 report.
//!
//! This is a `harness = false` executable so `--allow-natives-syntax` can be set before any V8 platform exists.

use script::api_combat::{CombatEnd, CombatPage, CombatSummary, ReportEnd};
use script::api_gather::GatherPhase;
use script::combat::PvpSource;
use script::shim::InteractReq;
use script_js::wire::{encode_snapshot_delta_with_native, NativeFactsInput, SnapshotFingerprint};
use script_js::{LoadIsolate, LoadShape};

mod common;

const BASE_SCRIPT: &str = r#"
export const apiVersion = 2;
export function tick(api) { globalThis.__api = api; }
"#;

/// The keys of one `report`, in order: the nine counters of the pre-V1 summary
/// and `pvpSource`.
const REPORT_KEYS: [&str; 13] = [
    "end",
    "reason",
    "npcType",
    "ticks",
    "swings",
    "casts",
    "damageTaken",
    "food",
    "prayerDoses",
    "boostDoses",
    "antifireDoses",
    "protectSwitches",
    "pvpSource",
];

fn probe(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr)
        .unwrap_or_else(|error| panic!("probe failed: {error}"))
}

fn post(
    iso: &LoadIsolate,
    tick: u64,
    page: Option<&CombatPage>,
    end: Option<&CombatEnd>,
    last: Option<&SnapshotFingerprint>,
) -> SnapshotFingerprint {
    let mut input = common::ingame_snapshot();
    input.tick = tick;
    let native = NativeFactsInput {
        api_combat: page,
        api_combat_outcome: end,
        ..NativeFactsInput::default()
    };
    let (bytes, fingerprint) = encode_snapshot_delta_with_native(last, &input, native, false);
    iso.post_snapshot(bytes);
    iso.on_game_tick(tick);
    let _ = probe(iso, "true");
    fingerprint
}

fn summary(pvp_source: Option<PvpSource>) -> CombatSummary {
    CombatSummary {
        end: ReportEnd::Killed,
        reason: None,
        npc_type: 81,
        ticks: 40,
        swings: 9,
        casts: 0,
        damage_taken: 3,
        food: 1,
        prayer_doses: 0,
        boost_doses: 0,
        antifire_doses: 0,
        protect_switches: 0,
        pvp_source,
    }
}

/// One whole fight: `api.combat.fight` admitted, the host's `fought` terminal
/// delivered, the result appended to `globalThis.__kept`.
fn one_fight(iso: &LoadIsolate, tick: &mut u64, pvp_source: Option<PvpSource>) {
    probe(
        iso,
        r#"(() => {
            globalThis.__done = false;
            __api.combat.fight({ target: { npc: ['cow'] }, budgetTicks: 5 })
                .then((result) => { globalThis.__done = true; (globalThis.__kept ??= []).push(result); });
            return true;
        })()"#,
    );
    let current = probe(iso, "__api.tick").as_u64().expect("numeric tick");
    iso.on_game_tick(current);
    let _ = probe(iso, "true");
    let token = iso
        .drain_interacts()
        .into_iter()
        .find_map(|request| match request {
            InteractReq::CombatFight { request_id, .. } => Some(request_id),
            _ => None,
        })
        .expect("the fight request reaches the host");
    let end = CombatEnd::Fought {
        token,
        report: summary(pvp_source),
    };
    let page = CombatPage {
        token: 0,
        phase: GatherPhase::Running,
        status: None,
    };
    *tick += 1;
    post(iso, *tick, Some(&page), Some(&end), None);
    assert_eq!(probe(iso, "globalThis.__done"), true, "the fight settled");
}

const SHAPE_FACTS: &str = r#"(() => {
    const fast = new Function('o', 'return %HasFastProperties(o)');
    const facts = (o) => ({
        fast: fast(o),
        prototype: Object.getPrototypeOf(o) === Object.prototype,
        keys: Object.keys(o),
    });
    return globalThis.__kept.map((result) => ({
        envelope: facts(result),
        value: facts(result.value),
        report: facts(result.value.report),
        pvpSource: result.value.report.pvpSource,
    }));
})()"#;

fn main() {
    // Must run before V8 exists in this process.
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    script_js::bootstrap_v8_platform().expect("process-main V8 bootstrap");
    let iso = LoadIsolate::spawn(BASE_SCRIPT.into(), LoadShape::NativeTick, vec![])
        .expect("spawn combat-summary isolate");
    let mut tick = 1;
    post(&iso, tick, None, None, None);

    // 1. Shape: a summary without a source and one of every source.
    for source in [
        None,
        Some(PvpSource::Request),
        Some(PvpSource::Global),
        Some(PvpSource::Both),
    ] {
        one_fight(&iso, &mut tick, source);
    }
    let facts = probe(&iso, SHAPE_FACTS);
    let facts = facts.as_array().expect("one fact row per fight");
    assert_eq!(facts.len(), 4);
    let expected: Vec<serde_json::Value> = REPORT_KEYS
        .iter()
        .map(|key| serde_json::Value::from(*key))
        .collect();
    for (index, row) in facts.iter().enumerate() {
        assert!(
            row["envelope"]["fast"].as_bool().unwrap_or(false),
            "summary {index} envelope must have fast properties: {row}"
        );
        assert!(
            row["envelope"]["prototype"].as_bool().unwrap_or(false),
            "summary {index} envelope must keep Object.prototype: {row}"
        );
        // Every summary is built the same way, so every one has the same mode.
        assert_eq!(
            (&row["report"]["fast"], &row["report"]["prototype"]),
            (
                &facts[0]["report"]["fast"],
                &facts[0]["report"]["prototype"]
            ),
            "summary {index} report is built unlike the first"
        );
        assert_eq!(
            row["report"]["keys"].as_array().expect("keys"),
            &expected,
            "summary {index}: every report carries the same keys in the same order"
        );
        assert_eq!(
            row["envelope"]["keys"],
            serde_json::json!(["kind", "value"])
        );
        assert_eq!(
            row["value"]["keys"],
            serde_json::json!(["end", "token", "report"])
        );
    }
    println!(
        "combat summary objects: report fast={} object-prototype={} ({})",
        facts[0]["report"]["fast"],
        facts[0]["report"]["prototype"],
        if facts[0]["report"]["fast"] == true {
            "fast"
        } else {
            "dictionary"
        }
    );
    assert!(facts[0]["pvpSource"].is_null());
    assert_eq!(facts[1]["pvpSource"], "request");
    assert_eq!(facts[2]["pvpSource"], "global");
    assert_eq!(facts[3]["pvpSource"], "both");

    iso.join();
    println!("combat_summary_shape ... ok");
}
