//! CookBot reads bank.tile and surface.stand from posted Catherby facts, and
//! frozen `resolveCookLocation` (`api/cooking/CookLocations.ts:25-49`) picks
//! the location: named only when unlocked, `Auto` the nearest unlocked one.

use std::sync::Arc;

use api::named_banks::{NamedBankFacts, BANK_CATALOG};
use nav::named_banks::resolve;
use script::{LoadIsolate, LoadShape};

fn spawn_with(src: &str, facts: NamedBankFacts) -> LoadIsolate {
    LoadIsolate::spawn_with_content(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        None,
        Arc::new(facts),
        Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
    )
    .unwrap()
}

/// The full bank catalog, every bank reachable.
fn spawn(src: &str) -> LoadIsolate {
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let facts = resolve(BANK_CATALOG, &data.bank_placements().unwrap().rows, |_| {
        true
    });
    spawn_with(src, facts)
}

const SRC: &str = r#"
import {
    COOK_LOCATIONS,
    COOK_LOCATION_OPTIONS,
    CUSTOM_LOCATION,
    resolveCookLocation,
} from '../../api/cooking/CookLocations.js';
import { MAX_SURFACE_CHEB } from '../../data/cookLocations.js';

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
const far = { x: 3200, z: 3200, level: 0 };
globalThis.__lockedNamed = resolveCookLocation('Catherby', far, () => false);
globalThis.__lockedAuto = resolveCookLocation('Auto', far, () => false);
globalThis.__asked = [];
globalThis.__askedAuto = resolveCookLocation('Auto', far, (loc) => {
    globalThis.__asked.push([loc.name, xyz(loc.bank.tile)]);
    return true;
})?.name ?? null;

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
fn auto_takes_the_nearest_unlocked_location_and_unknown_stays_null() {
    let iso = spawn(SRC);
    let auto = probe_obj(&iso, "__auto");
    assert_eq!(auto.get("stopped"), Some(&serde_json::Value::Null));
    assert_eq!(auto.get("whereName"), Some(&serde_json::json!("Catherby")));
    assert_eq!(auto.get("bank"), Some(&serde_json::json!([2809, 3441, 0])));
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
fn a_locked_location_is_null_by_name_and_for_auto() {
    let iso = spawn(SRC);
    assert_eq!(probe_obj(&iso, "__lockedNamed"), serde_json::Value::Null);
    assert_eq!(probe_obj(&iso, "__lockedAuto"), serde_json::Value::Null);
    assert_eq!(
        probe_obj(&iso, "__asked"),
        serde_json::json!([["Catherby", [2809, 3441, 0]]]),
        "the caller's predicate sees each CookLocation"
    );
    assert_eq!(
        probe_obj(&iso, "__askedAuto"),
        serde_json::json!("Catherby")
    );
    iso.join();

    // The default gate is the bank's own: a catalog without the Catherby
    // bank cannot open it.
    let iso = spawn_with(SRC, NamedBankFacts::empty());
    assert_eq!(
        probe_obj(&iso, "__catherby").get("stopped"),
        Some(&serde_json::json!(
            "no bank called 'Catherby' that this account can open"
        ))
    );
    assert_eq!(
        probe_obj(&iso, "__auto").get("stopped"),
        Some(&serde_json::json!(
            "no bank called 'Auto' that this account can open"
        ))
    );
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
    iso.join();
}
