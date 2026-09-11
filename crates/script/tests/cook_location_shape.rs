//! CookBot reads bank.tile and surface.stand from posted Catherby facts.

use std::sync::Arc;

use api::named_banks::{resolve, CANDIDATES};
use api::snapshot::WorldTile;
use script::{LoadIsolate, LoadShape};

fn packed_booths() -> Vec<WorldTile> {
    CANDIDATES
        .iter()
        .flat_map(|c| c.booths.iter().copied())
        .collect()
}

fn spawn(src: &str) -> LoadIsolate {
    let facts = resolve(&packed_booths(), |_| true);
    LoadIsolate::spawn_with_content(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        None,
        Arc::new(facts),
    )
    .unwrap()
}

const SRC: &str = r#"
import {
    COOK_LOCATIONS,
    COOK_LOCATION_OPTIONS,
    CUSTOM_LOCATION,
    resolveCookLocation,
} from '../../api/cooking/CookLocations.js';
import { MAX_SURFACE_CHEB } from '../../data/cookLocations.js';
import { BANK_LOCATIONS } from '../../api/bank/BankLocations.js';

function xyz(t) {
    return t ? [t.x, t.z, t.level ?? 0] : null;
}

function consume(setting, mode, rangeName) {
    const custom = String(setting).trim().toLowerCase() === CUSTOM_LOCATION.toLowerCase();
    const where = custom ? null : resolveCookLocation(setting, { x: 2809, z: 3441, level: 0 });
    if (!custom && !where) {
        return { stopped: `no bank called '${setting}' that this account can open` };
    }
    if (mode === 'fire') {
        return {
            stopped: null,
            mode: 'fire',
            whereName: where?.name ?? CUSTOM_LOCATION,
            bank: xyz(where?.bank?.tile),
        };
    }
    const plan = where?.surface ?? null;
    if (where && !plan) {
        return {
            stopped: `nothing cookable within ${MAX_SURFACE_CHEB} tiles of the ${where.name} bank, set 'Cook on' to Fire`,
        };
    }
    const bank = where?.bank;
    const openPath = bank?.npcAccess ? 'npc' : bank?.access ? 'access' : 'booth';
    return {
        stopped: null,
        whereName: where?.name ?? CUSTOM_LOCATION,
        bank: xyz(where?.bank?.tile),
        bankIsTile: !!(where && where.bank && typeof where.bank.x === 'number'),
        stand: xyz(plan && plan.stand),
        approach: plan && plan.approach ? xyz(plan.approach) : null,
        loc: xyz(plan && plan.loc),
        locName: plan?.locName ?? rangeName,
        arriveRadius: plan?.arriveRadius ?? 0,
        verified: !!(where && where.verified),
        openPath,
        oldRange: where ? xyz(where.range) : null,
    };
}

globalThis.__options = COOK_LOCATION_OPTIONS;
globalThis.__names = COOK_LOCATIONS.map((l) => l.name);
globalThis.__bankNames = BANK_LOCATIONS.map((b) => b.name);
globalThis.__catherby = consume('Catherby', 'range', 'Range');
globalThis.__postedSurface = (() => {
    const where = resolveCookLocation('Catherby');
    const plan = where && where.surface;
    return {
        locName: plan ? plan.locName : 'missing-surface',
        loc: plan ? xyz(plan.loc) : null,
        verified: where ? where.verified : null,
        standIsLoc: !!(plan && plan.loc && plan.stand
            && plan.loc.x === plan.stand.x
            && plan.loc.z === plan.stand.z
            && (plan.loc.level ?? 0) === (plan.stand.level ?? 0)),
    };
})();
globalThis.__auto = consume('Auto', 'range', 'Range');
globalThis.__custom = consume('Custom', 'range', 'Range');
globalThis.__unknown = consume('Seers', 'range', 'Range');
globalThis.__falador = consume('Falador East', 'range', 'Range');
globalThis.__maxCheb = MAX_SURFACE_CHEB;

export default class T extends LoopingBot {
    loop() {}
}
"#;

fn probe_obj(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr).unwrap()
}

#[test]
fn catherby_consumer_reads_bank_tile_and_host_stand() {
    let iso = spawn(SRC);
    let got = probe_obj(&iso, "__catherby");
    assert_eq!(got.get("stopped"), Some(&serde_json::Value::Null));
    assert_eq!(got.get("whereName"), Some(&serde_json::json!("Catherby")));
    assert_eq!(got.get("bank"), Some(&serde_json::json!([2809, 3441, 0])));
    assert_eq!(got.get("stand"), Some(&serde_json::json!([2817, 3443, 0])));
    assert_eq!(got.get("bankIsTile"), Some(&serde_json::json!(false)));
    assert_eq!(got.get("oldRange"), Some(&serde_json::Value::Null));
    assert_eq!(got.get("approach"), Some(&serde_json::Value::Null));
    assert_eq!(got.get("loc"), Some(&serde_json::Value::Null));
    assert_eq!(got.get("locName"), Some(&serde_json::json!("Range")));
    assert_eq!(got.get("arriveRadius"), Some(&serde_json::json!(0)));
    assert_eq!(got.get("verified"), Some(&serde_json::json!(false)));
    assert_eq!(got.get("openPath"), Some(&serde_json::json!("booth")));
    let posted = probe_obj(&iso, "__postedSurface");
    assert_eq!(posted.get("locName"), Some(&serde_json::Value::Null));
    assert_eq!(posted.get("loc"), Some(&serde_json::Value::Null));
    assert_eq!(posted.get("verified"), Some(&serde_json::json!(false)));
    assert_eq!(posted.get("standIsLoc"), Some(&serde_json::json!(false)));
    iso.join();
}

#[test]
fn unknown_and_auto_do_not_invent_a_location() {
    let iso = spawn(SRC);
    assert_eq!(
        probe_obj(&iso, "__auto").get("stopped"),
        Some(&serde_json::json!(
            "no bank called 'Auto' that this account can open"
        ))
    );
    assert_eq!(
        probe_obj(&iso, "__unknown").get("stopped"),
        Some(&serde_json::json!(
            "no bank called 'Seers' that this account can open"
        ))
    );
    assert_eq!(
        probe_obj(&iso, "__falador").get("stopped"),
        Some(&serde_json::json!(
            "no bank called 'Falador East' that this account can open"
        ))
    );
    assert_eq!(probe_obj(&iso, "__maxCheb"), serde_json::json!(20));
    iso.join();
}

#[test]
fn custom_keeps_settings_fallback_and_options_stay_catherby() {
    let iso = spawn(SRC);
    let custom = probe_obj(&iso, "__custom");
    assert_eq!(custom.get("stopped"), Some(&serde_json::Value::Null));
    assert_eq!(custom.get("whereName"), Some(&serde_json::json!("Custom")));
    assert_eq!(custom.get("bank"), Some(&serde_json::Value::Null));
    assert_eq!(custom.get("stand"), Some(&serde_json::Value::Null));
    assert_eq!(custom.get("locName"), Some(&serde_json::json!("Range")));
    assert_eq!(
        probe_obj(&iso, "__options"),
        serde_json::json!(["Auto", "Catherby", "Custom"])
    );
    assert_eq!(probe_obj(&iso, "__names"), serde_json::json!(["Catherby"]));
    let banks = probe_obj(&iso, "__bankNames");
    let bank_names = banks.as_array().expect("bank names");
    assert!(
        bank_names.iter().any(|n| n == "Falador East"),
        "named-bank mapping must still exist beside cook stands, got {banks:?}"
    );
    assert!(
        !bank_names.iter().any(|n| n == "Catherby"),
        "Catherby must stay a cook stand, not a BANK_LOCATIONS alias"
    );
    iso.join();
}
