//! `Tools.bestFromTiers` export and frozen RangingGuildLogic sibling load.

use script::load::{JsLibrary, LoadIsolate, LoadShape};
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

const TIER_SRC: &str = r#"
import { bestFromTiers } from '../../api/acquisition/Tools.js';

const BOWS = [
    { name: 'Magic shortbow', level: 50 },
    { name: 'Yew shortbow', level: 40 },
    { name: 'Shortbow', level: 1 },
];

const gateCalls = [];
globalThis.__gateHit = bestFromTiers(40, BOWS, (n) => {
    gateCalls.push(n);
    return n === 'Yew shortbow';
});
globalThis.__gateCalls = gateCalls;

const skipCalls = [];
globalThis.__skipHit = bestFromTiers(39, BOWS, (n) => {
    skipCalls.push(n);
    return n === 'Shortbow';
});
globalThis.__skipCalls = skipCalls;

const noneCalls = [];
globalThis.__noneHit = bestFromTiers(0, BOWS, (n) => {
    noneCalls.push(n);
    return true;
});
globalThis.__noneCalls = noneCalls;

const exhaustCalls = [];
globalThis.__exhaust = bestFromTiers(50, BOWS, (n) => {
    exhaustCalls.push(n);
    return false;
});
globalThis.__exhaustCalls = exhaustCalls;

globalThis.__truthy = bestFromTiers(50, BOWS, (n) => (n === 'Magic shortbow' ? 1 : false));

globalThis.__boundary = bestFromTiers(40, BOWS, () => true);

const nanCalls = [];
globalThis.__levelNan = bestFromTiers(NaN, BOWS, (n) => {
    nanCalls.push(n);
    return true;
});
globalThis.__levelNanCalls = nanCalls;

try {
    bestFromTiers(40, BOWS, null);
    globalThis.__nonFnReached = 'called';
} catch (e) {
    globalThis.__nonFnReached = String(e && e.name ? e.name : e);
}

globalThis.__nonFnIneligible = bestFromTiers(0, BOWS, null);

try {
    bestFromTiers(40, BOWS, (n) => {
        if (n === 'Yew shortbow') throw new Error('avail boom');
        return false;
    });
    globalThis.__availBoom = 'called';
} catch (e) {
    globalThis.__availBoom = String(e && e.message ? e.message : e);
}

// Frozen `for (const t of tiers)` reads `t.level` on every visited tier, so a
// null tier after an unaccepted one throws before `B` is reached.
const orderTiers = [{ name: 'A', level: 1 }, null, { name: 'B', level: 1 }];
const orderCalls = [];
try {
    globalThis.__orderHit = bestFromTiers(5, orderTiers, (n) => {
        orderCalls.push(n);
        return n === 'B';
    });
} catch (e) {
    globalThis.__orderHit = String(e && e.message ? e.message : e);
}
globalThis.__orderCalls = orderCalls;

export default class T extends LoopingBot {
    loop() {}
}
"#;

fn tier_spawn() -> LoadIsolate {
    LoadIsolate::spawn(TIER_SRC.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

#[test]
fn best_from_tiers_export_orders_level_before_available_and_short_circuits() {
    let iso = tier_spawn();
    assert_eq!(iso.probe("__gateHit").unwrap(), "Yew shortbow");
    assert_eq!(
        iso.probe("__gateCalls").unwrap(),
        serde_json::json!(["Yew shortbow"]),
        "level 40 skips magic (50) at the native gate without calling available"
    );
    assert_eq!(iso.probe("__skipHit").unwrap(), "Shortbow");
    assert_eq!(
        iso.probe("__skipCalls").unwrap(),
        serde_json::json!(["Shortbow"]),
        "level 39 skips higher tiers before probing availability"
    );
    assert_eq!(iso.probe("__noneHit").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__noneCalls").unwrap(),
        serde_json::json!([]),
        "level 0 never reaches availability"
    );
    assert_eq!(iso.probe("__exhaust").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__exhaustCalls").unwrap(),
        serde_json::json!(["Magic shortbow", "Yew shortbow", "Shortbow"])
    );
    assert_eq!(
        iso.probe("__truthy").unwrap(),
        "Magic shortbow",
        "truthy non-true callback results still select the tier"
    );
    assert_eq!(iso.probe("__boundary").unwrap(), "Yew shortbow");
    assert_eq!(iso.probe("__levelNan").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__levelNanCalls").unwrap(),
        serde_json::json!([]),
        "NaN player level never reaches availability"
    );
    assert_eq!(
        iso.probe("__nonFnReached").unwrap(),
        "TypeError",
        "null available throws once an eligible tier is reached"
    );
    assert_eq!(
        iso.probe("__nonFnIneligible").unwrap(),
        serde_json::Value::Null,
        "null available is not invoked when no tier is eligible"
    );
    assert_eq!(iso.probe("__availBoom").unwrap(), "avail boom");
    assert_eq!(
        iso.probe("__orderHit").unwrap(),
        "Cannot read properties of null (reading 'level')"
    );
    assert_eq!(
        iso.probe("__orderCalls").unwrap(),
        serde_json::json!(["A"]),
        "the frozen loop stops at the null tier it cannot read"
    );
    iso.join();
}

fn scratch(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("274bot-tool-tiers-{}-{}", std::process::id(), name))
}

#[test]
fn ranging_guild_logic_module_evaluates_with_best_from_tiers() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let card_path = root.join("src/bot/scripts/RangingGuild/RangingGuild.ts");
    if !card_path.is_file() {
        return;
    }
    let origin = std::fs::read_to_string(&card_path).expect("RangingGuild.ts");
    let dir = scratch("ranging-logic");
    std::fs::create_dir_all(&dir).unwrap();
    let cache = JsCache::new(dir.join("sib-cache"));
    let siblings = script::resolve_sibling_modules(
        &card_path,
        &origin,
        &cache,
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
            shape: Some("CompatClass".into()),
            api_family: None,
        },
    )
    .expect("RangingGuild siblings resolve");
    assert!(
        siblings
            .iter()
            .any(|(url, _)| url.contains("RangingGuildLogic")),
        "logic sibling must be in graph: {:?}",
        siblings.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>()
    );

    let harness = r#"
import { bestBow, BOWS } from './RangingGuildLogic.js';
globalThis.__bows = BOWS.length;
globalThis.__bow = bestBow(40, (n) => n === 'Yew shortbow');
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = LoadIsolate::spawn(harness.to_string(), LoadShape::CompatClass, siblings)
        .expect("RangingGuildLogic harness spawns");
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__bows").unwrap(), 12);
    assert_eq!(iso.probe("__bow").unwrap(), "Yew shortbow");
    for line in iso.drain_logs() {
        assert!(
            !line.contains("not impl"),
            "RangingGuildLogic eval must not throw notImpl: {line}"
        );
    }
    iso.join();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ranging_guild_catalog_card_spawns_without_best_from_tiers_not_impl() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let card_path = root.join("src/bot/scripts/RangingGuild/RangingGuild.ts");
    if !card_path.is_file() {
        eprintln!("skip: pinned catalog lacks RangingGuild");
        return;
    }
    let dir = scratch("ranging-card");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    lib.ensure_js(ScriptSource::Catalog, "RangingGuild")
        .expect("RangingGuild transpile");
    let card = lib
        .get(ScriptSource::Catalog, "RangingGuild")
        .cloned()
        .expect("RangingGuild catalog row");
    let cache = JsCache::new(dir.join("sib-cache"));
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        &cache,
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
            shape: Some(format!("{:?}", card.shape)),
            api_family: None,
        },
    )
    .expect("RangingGuild siblings");
    let iso = LoadIsolate::spawn_with_game_data(
        card.js.clone(),
        card.shape,
        siblings,
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .expect("RangingGuild card spawn");
    iso.on_game_tick(1);
    let mut blockers = Vec::new();
    for line in iso.drain_logs() {
        if line.contains("not impl") || line.contains("Error") {
            blockers.push(line);
        }
    }
    iso.join();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        blockers.is_empty(),
        "full RangingGuild load blockers (report next gap if any):\n{}",
        blockers.join("\n")
    );
}
