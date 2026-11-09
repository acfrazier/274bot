//! Compat v1 contract tests for the mining, woodcutting, gathering, cooking and fishing rows of the rs2b0t script
//! API (COMPAT-FINISH-029, `impl/rows/{mining,woodcutting,gathering,cooking,fishing}.json`).
//!
//! The expected value of every case is the result of running the same source against the real upstream modules
//! (`tools/game-data/gather-oracle.ts`, fixture `fixtures/compat_v1_gather_oracle.json`): table contents, own-key
//! order, `undefined`-valued keys, `Tile` instances, `Set` versus array, argument coercion, error classes and the
//! order in which the caller's own count functions are called. Each case runs through the package-root barrel and
//! through every module route the case names, on the revision 289 selected content, and must encode identically.
//! Before the rows were implemented the modules were missing or their members threw `not impl`, so every test below
//! failed.

use client::io::ClientRevision;
use script_js::{LoadIsolate, LoadShape};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

const ORACLE: &str = include_str!("fixtures/compat_v1_gather_oracle.json");

type Data = Arc<api::game_data::SelectedGameData>;

fn oracle() -> Value {
    serde_json::from_str(ORACLE).expect("oracle fixture parses")
}

fn data(revision: ClientRevision) -> Data {
    api::game_data::for_revision(revision).expect("selected game data")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    /// `import { ... } from '@rs2b0t/api'`.
    Barrel,
    /// Each name from the module the oracle names for it (`../../data/miningLocations.js`).
    Modules,
}

impl Route {
    fn label(self) -> &'static str {
        match self {
            Self::Barrel => "barrel",
            Self::Modules => "modules",
        }
    }
}

/// One isolate: the case bodies are embedded as functions, so nothing needs `eval`.
fn run(route: Route, ids: &[&str], oracle: &Value, game: &Data) -> Value {
    let cases = &oracle["cases"];
    let mut names = BTreeSet::new();
    for id in ids {
        for name in cases[*id]["uses"].as_array().expect("uses") {
            names.insert(name.as_str().expect("name").to_string());
        }
    }
    let imports = match route {
        Route::Barrel => format!(
            "import {{ {} }} from '@rs2b0t/api';",
            names.into_iter().collect::<Vec<_>>().join(", ")
        ),
        Route::Modules => {
            let mut by_module: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
            for name in &names {
                let module = oracle["moduleOf"][name].as_str().expect("module route");
                by_module.entry(module).or_default().push(name.clone());
            }
            by_module
                .into_iter()
                .map(|(module, names)| {
                    format!("import {{ {} }} from '../../{module}';", names.join(", "))
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let functions = ids
        .iter()
        .map(|id| {
            let body = cases[*id]["body"].as_str().expect("body");
            format!("{:?}: () => {{ {body} }}", id)
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let encode = oracle["encode"].as_str().expect("encode");
    let source = format!(
        r#"
import Tile from '../../geometry/Tile.js';
{imports}
export default class T extends LoopingBot {{
    loop() {{
        {encode}
        const cases = {{
{functions}
        }};
        const out = {{}};
        for (const [id, fn] of Object.entries(cases)) {{
            try {{ out[id] = fn(); }} catch (e) {{ out[id] = 'THROW ' + e.constructor.name + ': ' + e.message; }}
        }}
        globalThis.__probe = JSON.stringify(out);
    }}
}}
"#
    );
    let iso =
        LoadIsolate::spawn_with_game_data(source, LoadShape::CompatClass, vec![], game.clone())
            .unwrap_or_else(|error| {
                panic!(
                    "the script must link on the {} route: {error:?}",
                    route.label()
                )
            });
    iso.on_game_tick(1);
    let raw = match iso.probe("globalThis.__probe") {
        Ok(raw) => raw,
        Err(error) => {
            let logs = iso.join();
            panic!("the loop must publish its results on the {} route: {error}\nisolate log: {logs:#?}", route.label());
        }
    };
    iso.join();
    serde_json::from_str(raw.as_str().expect("probe text")).expect("probe json")
}

/// The routes a case can take: the barrel when every name it reads is declared, the modules when each name has one.
fn routes_of(case: &Value, oracle: &Value, declared: &BTreeSet<String>) -> Vec<Route> {
    let uses: Vec<&str> = case["uses"]
        .as_array()
        .expect("uses")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut routes = Vec::new();
    if uses.iter().all(|name| declared.contains(*name)) {
        routes.push(Route::Barrel);
    }
    if uses.iter().all(|name| oracle["moduleOf"][name].is_string()) {
        routes.push(Route::Modules);
    }
    routes
}

/// Every case of `selector` on each route it can take; an encoded result differing from upstream fails.
fn contract(selector: &str) {
    let oracle = oracle();
    let declared: BTreeSet<String> = script_js::tooling::declared_abi::load_fixture()
        .expect("declared ABI fixture")
        .into_iter()
        .map(|export| export.name)
        .collect();
    let ids: Vec<&str> = oracle["cases"]
        .as_object()
        .expect("cases")
        .iter()
        .filter(|(_, case)| case["selector"] == selector)
        .map(|(id, _)| id.as_str())
        .collect();
    assert!(!ids.is_empty(), "the oracle has no case for {selector}");
    let game = data(ClientRevision::R289);
    let mut failures = Vec::new();
    let mut ran = BTreeSet::new();
    for route in [Route::Barrel, Route::Modules] {
        let serving: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| routes_of(&oracle["cases"][*id], &oracle, &declared).contains(&route))
            .collect();
        if serving.is_empty() {
            continue;
        }
        let got = run(route, &serving, &oracle, &game);
        for id in &serving {
            ran.insert(*id);
            let expected = oracle["cases"][*id]["expected"].as_str().expect("expected");
            let actual = got[*id].as_str().unwrap_or("<missing>");
            if actual != expected {
                failures.push(format!(
                    "{selector} / {id} via {}\n  expected {expected}\n  actual   {actual}",
                    route.label()
                ));
            }
        }
    }
    let unrouted: Vec<&&str> = ids.iter().filter(|id| !ran.contains(**id)).collect();
    assert!(
        unrouted.is_empty(),
        "no supported route serves {unrouted:?}"
    );
    assert!(
        failures.is_empty(),
        "{} case(s) differ from upstream:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

macro_rules! contract_tests {
    ($($name:ident => $selector:literal,)*) => {
        $(
            #[test]
            #[expect(non_snake_case, reason = "the test names are the census row names")]
            fn $name() {
                if !api::test_support::game_data_available() {
                    return;
                }
                contract($selector);
            }
        )*
    };
}

contract_tests! {
    compat_v1_mining__mining_locations__contract => "MINING_LOCATIONS",
    compat_v1_mining__mining_location_options__contract => "MINING_LOCATION_OPTIONS",
    compat_v1_mining__mining_location_option_labels__contract => "MINING_LOCATION_OPTION_LABELS",
    compat_v1_mining__mininglocationlabel__contract => "miningLocationLabel",
    compat_v1_mining__resolvemininglocation__contract => "resolveMiningLocation",
    compat_v1_mining__gas_rock_ids__contract => "GAS_ROCK_IDS",
    compat_v1_woodcutting__resolvewoodcuttinglocation__contract => "resolveWoodcuttingLocation",
    compat_v1_woodcutting__woodcutting_locations__contract => "WOODCUTTING_LOCATIONS",
    compat_v1_woodcutting__woodcutting_location_options__contract => "WOODCUTTING_LOCATION_OPTIONS",
    compat_v1_gathering__use_custom_position__contract => "USE_CUSTOM_POSITION",
    compat_v1_gathering__default_chase_radius__contract => "DEFAULT_CHASE_RADIUS",
    compat_v1_gathering__none_legacy__contract => "NONE_LEGACY",
    compat_v1_gathering__resolvecampradius__contract => "resolveCampRadius",
    compat_v1_gathering__resolvechaseradius__contract => "resolveChaseRadius",
    compat_v1_gathering__auto_legacy__contract => "AUTO_LEGACY",
    compat_v1_gathering__default_camp_radius__contract => "DEFAULT_CAMP_RADIUS",
    compat_v1_gathering__use_closest__contract => "USE_CLOSEST",
    compat_v1_gathering__use_start_position__contract => "USE_START_POSITION",
    compat_v1_cooking__cooksurfaceforfishcamp__contract => "cookSurfaceForFishCamp",
    compat_v1_cooking__resolvefishcampcooksurface__contract => "resolveFishCampCookSurface",
    compat_v1_fishing__all_fishing_gear_names__contract => "ALL_FISHING_GEAR_NAMES",
    compat_v1_fishing__fishing_locations__contract => "FISHING_LOCATIONS",
    compat_v1_fishing__fishing_location_options__contract => "FISHING_LOCATION_OPTIONS",
    compat_v1_fishing__fishing_methods__contract => "FISHING_METHODS",
    compat_v1_fishing__fishing_method_options__contract => "FISHING_METHOD_OPTIONS",
    compat_v1_fishing__fishing_shop_costs__contract => "FISHING_SHOP_COSTS",
    compat_v1_fishing__gerrant_only_fishing__contract => "GERRANT_ONLY_FISHING",
    compat_v1_fishing__buyplanscost__contract => "buyPlansCost",
    compat_v1_fishing__fishinggearshopcart__contract => "fishingGearShopCart",
    compat_v1_fishing__fishingrestockplan__contract => "fishingRestockPlan",
    compat_v1_fishing__fishingshopcost__contract => "fishingShopCost",
    compat_v1_fishing__fishingvendorfor__contract => "fishingVendorFor",
    compat_v1_fishing__gearkeepnames__contract => "gearKeepNames",
    compat_v1_fishing__gearlabel__contract => "gearLabel",
    compat_v1_fishing__hasfishinggear__contract => "hasFishingGear",
    compat_v1_fishing__isfishingbaitpiece__contract => "isFishingBaitPiece",
    compat_v1_fishing__missingfishinggear__contract => "missingFishingGear",
    compat_v1_fishing__planfishinggearacquire__contract => "planFishingGearAcquire",
    compat_v1_fishing__planfishinggearbuys__contract => "planFishingGearBuys",
    compat_v1_fishing__resolvefishmethod__contract => "resolveFishMethod",
    compat_v1_fishing__resolvefishinglocation__contract => "resolveFishingLocation",
    compat_v1_fishing__shopablemissingfishinggear__contract => "shopableMissingFishingGear",
    compat_v1_fishing__spotmatchesmethod__contract => "spotMatchesMethod",
    compat_v1_fishing__withbaittarget__contract => "withBaitTarget",
    compat_v1_fishing__whirlpool_ids__contract => "WHIRLPOOL_IDS",
    compat_v1_fishing__shilo_water_vendor__contract => "SHILO_WATER_VENDOR",
}

/// Every selector the five census row files name has a contract test above.
#[test]
fn every_census_selector_has_an_oracle_case() {
    let oracle = oracle();
    let selectors: BTreeSet<&str> = oracle["cases"]
        .as_object()
        .expect("cases")
        .values()
        .map(|case| case["selector"].as_str().expect("selector"))
        .collect();
    let tested = include_str!("compat_v1_gather.rs");
    for selector in &selectors {
        assert!(
            tested.contains(&format!("=> {selector:?},")),
            "{selector} has cases but no contract test"
        );
    }
    assert_eq!(
        selectors.len(),
        46,
        "the five families publish 46 distinct selectors"
    );
}

/// Whirlpool ids come from the selected content's NPC records on both revisions, and without selected content the
/// export refuses by name instead of publishing an empty set.
#[test]
fn whirlpool_ids_follow_the_selected_content_and_refuse_without_it() {
    let oracle = oracle();
    let ids = ["WHIRLPOOL_IDS.table"];
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let got = run(Route::Modules, &ids, &oracle, &data(revision));
        assert_eq!(
            got[ids[0]], oracle["cases"][ids[0]]["expected"],
            "{revision:?}"
        );
    }
    let source = r#"
import { WHIRLPOOL_IDS } from '../../data/fishingMethods.js';
export default class T extends LoopingBot {
    loop() {
        try { globalThis.__probe = String(WHIRLPOOL_IDS.size); } catch (e) { globalThis.__probe = String(e.message); }
    }
}
"#;
    let iso = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![])
        .expect("the module loads without game data");
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").expect("probe");
    iso.join();
    assert!(
        probe
            .as_str()
            .is_some_and(|text| text.contains("not impl") && text.contains("WHIRLPOOL_IDS")),
        "{probe:?}"
    );
}

/// `GAS_ROCK_IDS` is the one `data/miningRocks.js` set on every route and keeps its data-unavailable refusal.
#[test]
fn gas_rock_ids_are_one_set_on_every_route_and_refuse_without_content() {
    if !api::test_support::game_data_available() {
        return;
    }
    let source = r#"
import { GAS_ROCK_IDS as moduleSet } from '../../data/miningRocks.js';
import { GAS_ROCK_IDS as barrelSet } from '@rs2b0t/api';
export default class T extends LoopingBot {
    loop() {
        try { globalThis.__probe = String(moduleSet === barrelSet) + ':' + moduleSet.size; } catch (e) { globalThis.__probe = String(e.message); }
    }
}
"#;
    let with = LoadIsolate::spawn_with_game_data(
        source.into(),
        LoadShape::CompatClass,
        vec![],
        data(ClientRevision::R289),
    )
    .expect("loads");
    with.on_game_tick(1);
    assert_eq!(with.probe("globalThis.__probe").expect("probe"), "true:21");
    with.join();
    let without = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![]).expect("loads");
    without.on_game_tick(1);
    let probe = without.probe("globalThis.__probe").expect("probe");
    without.join();
    assert!(
        probe
            .as_str()
            .is_some_and(|text| text.contains("not impl") && text.contains("GAS_ROCK_IDS")),
        "{probe:?}"
    );
}

/// The vendor objects `fishingVendorFor` and the buy plans return are the very `GERRANT_VENDOR` and `HARRY_VENDOR`
/// objects the fishing-owned module exports (the acquisition family re-exports them from there).
#[test]
fn vendor_results_are_the_exported_vendor_objects() {
    if !api::test_support::game_data_available() {
        return;
    }
    let source = r#"
import { GERRANT_VENDOR, HARRY_VENDOR, fishingVendorFor, planFishingGearBuys, fishingGearShopCart } from '../../shim/fishing_gear.js';
import { FISHING_METHODS } from '../../data/fishingMethods.js';
export default class T extends LoopingBot {
    loop() {
        const w = { heldCount: () => 0, invCount: () => 0, bankCount: () => 0 };
        const rich = { heldCount: () => 0, invCount: (n) => (n === 'Coins' ? 1000 : 0), bankCount: () => 0 };
        const cart = fishingGearShopCart(FISHING_METHODS[2], rich);
        globalThis.__probe = JSON.stringify([
            fishingVendorFor('Feather') === GERRANT_VENDOR, fishingVendorFor('Fishing bait') === HARRY_VENDOR,
            planFishingGearBuys(FISHING_METHODS[5], rich, { near: { x: 2833, z: 3443 } })[0].vendor === HARRY_VENDOR,
            cart.length === 2 && cart.every((plan) => plan.vendor === GERRANT_VENDOR),
            GERRANT_VENDOR.stand !== GERRANT_VENDOR.bankStand, GERRANT_VENDOR.keeper, HARRY_VENDOR.keeper,
            planFishingGearBuys(FISHING_METHODS[2], w).length,
        ]);
    }
}
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        source.into(),
        LoadShape::CompatClass,
        vec![],
        data(ClientRevision::R289),
    )
    .expect("the fishing module loads");
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").expect("probe");
    iso.join();
    assert_eq!(probe, "[true,true,true,true,true,\"Gerrant\",\"Harry\",0]");
}
