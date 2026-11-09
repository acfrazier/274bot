//! Compat v1 walking table and map helper contracts.

use script_js::{LoadIsolate, LoadShape};
use serde_json::{json, Value};
use std::sync::Arc;

fn eval(source: &str, expression: &str) -> Value {
    let iso = LoadIsolate::spawn(source.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let result = iso.probe(expression).unwrap();
    iso.join();
    result
}

fn walking_class(body: &str) -> String {
    format!("export default class T extends LoopingBot {{ loop() {{ {body} }} }}")
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_walking__walk_destinations__contract() {
    let source = format!(
        "import {{ WALK_DESTINATIONS }} from '../../api/map/WalkDestinations.js';\n{}",
        walking_class(
            "globalThis.__rows = WALK_DESTINATIONS.map(({ name, tile }) => [name, tile.x, tile.z, tile.level]);"
        )
    );
    let rows = eval(&source, "globalThis.__rows");
    assert_eq!(
        rows,
        json!([
            ["Lumbridge", 3221, 3218, 0],
            ["Varrock", 3213, 3424, 0],
            ["Falador", 2965, 3378, 0],
            ["Ardougne", 2661, 3301, 0],
            ["Rellekka", 2668, 3660, 0],
            ["Taverley", 2895, 3435, 0],
            ["Draynor", 3093, 3243, 0],
            ["Al Kharid", 3269, 3167, 0],
            ["Edgeville", 3094, 3493, 0],
            ["Seers' Village", 2725, 3491, 0],
            ["Catherby", 2809, 3441, 0],
            ["Yanille", 2612, 3092, 0]
        ])
    );
}

#[test]
fn walk_via_declared_surface_uses_imported_shim_closure() {
    let source = format!(
        "import {{ WALK_DESTINATIONS, WALK_OPTIONS, resolveDestination }} from './declared_surface.js';\n{}",
        walking_class(
            "globalThis.__surface = [WALK_DESTINATIONS.length, WALK_OPTIONS[0], resolveDestination('fALaDoR').tile.x];"
        )
    );
    assert_eq!(
        eval(&source, "globalThis.__surface"),
        json!([12, "Lumbridge", 2965])
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_walking__walk_options__contract() {
    let source = format!(
        "import {{ WALK_OPTIONS }} from '../../api/map/WalkDestinations.js';\n{}",
        walking_class("globalThis.__names = WALK_OPTIONS;")
    );
    let names = eval(&source, "globalThis.__names");
    assert_eq!(
        names,
        json!([
            "Lumbridge",
            "Varrock",
            "Falador",
            "Ardougne",
            "Rellekka",
            "Taverley",
            "Draynor",
            "Al Kharid",
            "Edgeville",
            "Seers' Village",
            "Catherby",
            "Yanille"
        ])
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_walking__resolvedestination__contract() {
    let source = format!(
        "import {{ resolveDestination }} from '../../api/map/WalkDestinations.js';\n{}",
        walking_class(
            "const found = resolveDestination('  fALaDoR  '); globalThis.__result = [found && [found.name, found.tile.x, found.tile.z, found.tile.level], resolveDestination('not a destination')];"
        )
    );
    assert_eq!(
        eval(&source, "globalThis.__result"),
        json!([["Falador", 2965, 3378, 0], null])
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_walking__inessmine__contract() {
    let source = format!(
        "import {{ inEssMine }} from '../../api/map/regions.js';\n{}",
        walking_class(
            "globalThis.__values = [[inEssMine(45 * 64, 75 * 64), inEssMine(46 * 64 - 1, 76 * 64 - 1)], [inEssMine(46 * 64, 75 * 64), inEssMine(45 * 64, 76 * 64), inEssMine(-64, 75 * 64)]];"
        )
    );
    assert_eq!(
        eval(&source, "globalThis.__values"),
        json!([[true, true], [false, false, false]])
    );
}

/// The wilderness seam as the host would answer it for one zone (x 100..=110, z 200..=239, level
/// `(z - 200) / 8 + 1`, 0 outside); the real formula is `nav_planner::transport::WildernessRules`'s,
/// tested in the planner and through the host's adapter.
#[derive(Debug)]
struct OneZone;

impl script::surface::WildernessLevels for OneZone {
    fn level(&self, tile: api::snapshot::WorldTile) -> i32 {
        if (100..=110).contains(&tile.x)
            && (200..=239).contains(&tile.z)
            && (0..=3).contains(&tile.level)
        {
            (tile.z - 200) / 8 + 1
        } else {
            0
        }
    }
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_walking__wildernesslevelat__contract() {
    let source = format!(
        "import {{ wildernessLevelAt }} from '../../event/webwalk/wilderness.js';\n{}",
        walking_class(
            "globalThis.__levels = [wildernessLevelAt({ x: 100, z: 200, level: 0 }), wildernessLevelAt({ x: 110, z: 239, level: 3 }), wildernessLevelAt({ x: 100, z: 240, level: 0 }), wildernessLevelAt({ x: 111, z: 200, level: 0 }), wildernessLevelAt({ x: 100, z: 200, level: 4 })];"
        )
    );
    let rules: Arc<dyn script::surface::WildernessLevels> = Arc::new(OneZone);
    let iso =
        LoadIsolate::spawn_with_wilderness_rules(source, LoadShape::CompatClass, vec![], rules)
            .unwrap();
    iso.on_game_tick(1);
    let levels = iso.probe("globalThis.__levels").unwrap();
    iso.join();
    assert_eq!(levels, json!([1, 5, 0, 0, 0]));
}
