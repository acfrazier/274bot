//! Frozen `COOK_LOCATIONS` (`data/cookLocations.ts:118-132`) pairs every bank
//! with a curated or nearest cook surface, and `resolveCookLocation`
//! (`api/cooking/CookLocations.ts:25-49`) picks a named location only when
//! unlocked, `Auto` the nearest unlocked one. CookBot consumes the result.

use std::sync::Arc;

use api::game_data::SelectedGameData;
use api::named_banks::{NamedBankFacts, BANK_CATALOG};
use nav::named_banks::resolve;
use script::{LoadIsolate, LoadShape};

fn data() -> Arc<SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()
}

/// The full bank roster, every bank reachable.
fn roster() -> NamedBankFacts {
    resolve(
        BANK_CATALOG,
        &data().bank_placements().unwrap().rows,
        |_| true,
    )
}

fn spawn_with(
    src: &str,
    game_data: Option<Arc<SelectedGameData>>,
    facts: NamedBankFacts,
) -> Result<LoadIsolate, String> {
    LoadIsolate::spawn_with_content(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        game_data,
        Arc::new(facts),
        Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
    )
}

fn spawn(src: &str) -> LoadIsolate {
    spawn_with(src, Some(data()), roster()).unwrap()
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

function consume(setting, from) {
    const custom = String(setting).trim().toLowerCase() === CUSTOM_LOCATION.toLowerCase();
    const where = custom ? null : resolveCookLocation(setting, from ?? { x: 2809, z: 3441, level: 0 });
    if (!custom && !where) {
        return { stopped: `no bank called '${setting}' that this account can open` };
    }
    const plan = where?.surface ?? null;
    if (where && !plan) {
        return {
            stopped: `nothing cookable within ${MAX_SURFACE_CHEB} tiles of the ${where.name} bank, set 'Cook on' to Fire`,
        };
    }
    const bank = where?.bank;
    return {
        stopped: null,
        whereName: where?.name ?? CUSTOM_LOCATION,
        bank: xyz(bank?.tile),
        stand: xyz(plan && plan.stand),
        approach: plan && plan.approach ? xyz(plan.approach) : null,
        loc: xyz(plan && plan.loc),
        locName: plan?.locName ?? 'Range',
        kind: plan?.kind ?? null,
        arriveRadius: plan?.arriveRadius ?? 0,
        verified: !!(where && where.verified),
        obstacles: where?.obstacles ?? null,
        openPath: bank?.npcAccess ? 'npc' : bank?.access ? 'access' : 'booth',
    };
}

globalThis.__options = COOK_LOCATION_OPTIONS;
globalThis.__names = COOK_LOCATIONS.map((l) => l.name);
globalThis.__catherby = consume('Catherby');
globalThis.__seers = consume(' seers ');
globalThis.__autoCatherby = consume('Auto');
globalThis.__autoVarrock = consume('Auto', { x: 3253, z: 3420, level: 0 });
globalThis.__varrockWest = consume('Varrock West');
globalThis.__custom = consume('Custom');
globalThis.__unknown = consume('Nowhere');
globalThis.__maxCheb = MAX_SURFACE_CHEB;
const far = { x: 3200, z: 3200, level: 0 };
globalThis.__lockedNamed = resolveCookLocation('Catherby', far, () => false);
globalThis.__lockedAuto = resolveCookLocation('Auto', far, () => false);
globalThis.__asked = [];
globalThis.__onlyCatherby = resolveCookLocation('Auto', far, (loc) => {
    globalThis.__asked.push(loc.name);
    return loc.name === 'Catherby' && xyz(loc.bank.tile).join() === '2809,3441,0';
})?.name ?? null;

export default class T extends LoopingBot {
    loop() {}
}
"#;

fn probe(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr).unwrap()
}

#[test]
fn every_bank_is_a_location_between_auto_and_custom() {
    let names: Vec<_> = roster().banks().iter().map(|bank| bank.name).collect();
    let iso = spawn(SRC);
    assert_eq!(probe(&iso, "__names"), serde_json::json!(names));
    let mut options = vec!["Auto"];
    options.extend(names);
    options.push("Custom");
    assert_eq!(probe(&iso, "__options"), serde_json::json!(options));
    iso.join();
}

#[test]
fn a_curated_bank_keeps_its_hand_walked_surface() {
    let iso = spawn(SRC);
    let catherby = probe(&iso, "__catherby");
    assert_eq!(catherby["whereName"], "Catherby");
    assert_eq!(catherby["bank"], serde_json::json!([2809, 3441, 0]));
    assert_eq!(catherby["stand"], serde_json::json!([2817, 3443, 0]));
    assert_eq!(catherby["loc"], serde_json::json!([2817, 3444, 0]));
    assert_eq!(catherby["approach"], serde_json::Value::Null);
    assert_eq!(catherby["locName"], "Range");
    assert_eq!(catherby["kind"], "oven");
    assert_eq!(catherby["arriveRadius"], 0);
    assert_eq!(catherby["verified"], true);
    assert_eq!(catherby["obstacles"], serde_json::json!(["door", "gate"]));
    assert_eq!(catherby["openPath"], "booth");
    let seers = probe(&iso, "__seers");
    assert_eq!(seers["whereName"], "Seers");
    assert_eq!(seers["approach"], serde_json::json!([2713, 3484, 0]));
    assert_eq!(seers["loc"], serde_json::json!([2715, 3476, 0]));
    assert_eq!(seers["verified"], true);
    iso.join();
}

#[test]
fn auto_takes_the_nearest_unlocked_bank_and_its_nearest_oven() {
    let iso = spawn(SRC);
    assert_eq!(probe(&iso, "__autoCatherby")["whereName"], "Catherby");
    let varrock = probe(&iso, "__autoVarrock");
    assert_eq!(varrock["whereName"], "Varrock East");
    assert_eq!(
        varrock["loc"],
        serde_json::json!([3237, 3409, 0]),
        "the nearer of two ovens"
    );
    assert_eq!(varrock["stand"], serde_json::json!([3237, 3408, 0]));
    assert_eq!(varrock["kind"], "oven");
    assert_eq!(varrock["locName"], "Range");
    assert_eq!(varrock["arriveRadius"], 2);
    assert_eq!(varrock["verified"], false);
    assert_eq!(
        probe(&iso, "__varrockWest")["stopped"],
        "nothing cookable within 20 tiles of the Varrock West bank, set 'Cook on' to Fire"
    );
    assert_eq!(
        probe(&iso, "__unknown")["stopped"],
        "no bank called 'Nowhere' that this account can open"
    );
    assert_eq!(probe(&iso, "__custom")["whereName"], "Custom");
    assert_eq!(probe(&iso, "__maxCheb"), serde_json::json!(20));
    iso.join();
}

#[test]
fn a_locked_location_is_null_by_name_and_for_auto() {
    let iso = spawn(SRC);
    assert_eq!(probe(&iso, "__lockedNamed"), serde_json::Value::Null);
    assert_eq!(probe(&iso, "__lockedAuto"), serde_json::Value::Null);
    assert_eq!(
        probe(&iso, "__onlyCatherby"),
        serde_json::json!("Catherby"),
        "Auto honours the caller's predicate over each CookLocation"
    );
    let names: Vec<_> = roster().banks().iter().map(|bank| bank.name).collect();
    assert_eq!(probe(&iso, "__asked"), serde_json::json!(names));
    iso.join();

    // The default gate is the bank's own: without the Catherby bank in the
    // roster there is no Catherby location to open.
    let iso = spawn_with(SRC, Some(data()), NamedBankFacts::empty()).unwrap();
    assert_eq!(
        probe(&iso, "__catherby")["stopped"],
        "no bank called 'Catherby' that this account can open"
    );
    assert_eq!(
        probe(&iso, "__autoCatherby")["stopped"],
        "no bank called 'Auto' that this account can open"
    );
    iso.join();
}

/// Without selected game data there are no locations, and resolving one
/// reports the missing data instead of claiming the bank is locked.
#[test]
fn missing_game_data_is_the_explicit_error() {
    let src = r#"
import { COOK_LOCATIONS, resolveCookLocation } from '../../api/cooking/CookLocations.js';
let error = null;
try { resolveCookLocation('Catherby', { x: 2809, z: 3441, level: 0 }); } catch (e) { error = String(e.message); }
globalThis.__offline = { count: COOK_LOCATIONS.length, custom: resolveCookLocation('Custom'), error };
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = spawn_with(src, None, roster()).unwrap();
    let offline = probe(&iso, "__offline");
    assert_eq!(offline["count"], 0);
    assert_eq!(offline["custom"], serde_json::Value::Null);
    assert!(
        offline["error"]
            .as_str()
            .unwrap_or_default()
            .contains("game data unavailable"),
        "{offline:?}"
    );
    iso.join();
}
