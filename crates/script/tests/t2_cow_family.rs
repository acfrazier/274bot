#![cfg(feature = "load")]

//! Direct Load-isolate characterization of the public Cow catalog module.
use std::sync::Arc;

use api::game_data::SelectedGameData;
use api::named_banks::{NamedBankFacts, BANK_CATALOG};
use api::snapshot::WorldTile;
use script::{LoadIsolate, LoadShape};

const COW_MODULE: &str = r#"
import Tile from '../../geometry/Tile.js';
import {
    AL_KHARID_BANK,
    ARDOUGNE_WEST_BANK,
    COW_LOCATIONS,
    COW_LOCATION_OPTIONS,
    DRAYNOR_BANK,
    FALADOR_EAST_BANK,
    TOLL_COIN_TARGET,
    cowBankDestination,
    isCowFieldLootTile,
    nearestCowLocation,
    needsTollCoins,
    resolveCowLocation,
    shouldBootstrapTollCoins,
} from '../../data/cowKillerLocations.js';

globalThis.__cow = {
    locations: COW_LOCATIONS,
    options: COW_LOCATION_OPTIONS,
    bankConstants: [AL_KHARID_BANK, DRAYNOR_BANK, FALADOR_EAST_BANK, ARDOUGNE_WEST_BANK],
    alKharid: AL_KHARID_BANK,
    draynor: DRAYNOR_BANK,
    faladorEast: FALADOR_EAST_BANK,
    ardougneWest: ARDOUGNE_WEST_BANK,
    coinTarget: TOLL_COIN_TARGET,
    xyz(tile) {
        return tile == null ? null : [tile.x, tile.z, tile.level];
    },
    isTile(value) {
        return value instanceof Tile;
    },
    resolve: resolveCowLocation,
    nearest: nearestCowLocation,
    loot: isCowFieldLootTile,
    needsToll: needsTollCoins,
    bankDestination: cowBankDestination,
    bootstrap: shouldBootstrapTollCoins,
};

export default class CowProbe extends LoopingBot {
    loop() {}
}
"#;

fn selected_data() -> Arc<SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289)
        .expect("R289 selected game data")
}

fn selected_banks(data: &SelectedGameData) -> NamedBankFacts {
    let placements = data
        .bank_placements()
        .expect("R289 selected bank placements");
    nav::named_banks::resolve(BANK_CATALOG, &placements.rows, |_| true)
}

fn bank_tile(facts: &NamedBankFacts, name: &str) -> WorldTile {
    facts
        .banks()
        .iter()
        .find(|bank| bank.name == name)
        .unwrap_or_else(|| panic!("selected named-bank facts omit {name}"))
        .tile
}

fn xyz(tile: WorldTile) -> [i32; 3] {
    [tile.x, tile.z, tile.level]
}

fn load(data: Option<Arc<SelectedGameData>>, banks: NamedBankFacts) -> LoadIsolate {
    LoadIsolate::spawn_with_content(
        COW_MODULE.to_owned(),
        LoadShape::CompatClass,
        Vec::new(),
        data,
        Arc::new(banks),
    )
    .expect("start public Cow module in Load")
}

fn probe(isolate: &LoadIsolate, expression: &str) -> serde_json::Value {
    isolate
        .probe(expression)
        .unwrap_or_else(|error| panic!("Cow probe `{expression}` failed: {error}"))
}

#[test]
fn public_catalog_exports_every_location_option_and_fallback_bank_identity() {
    let isolate = load(None, NamedBankFacts::empty());

    assert_eq!(
        probe(
            &isolate,
            "__cow.locations.map(location => [location.name, __cow.xyz(location.anchor), location.usesAlKharidToll, location.bankDestination ? [location.bankDestination.name, __cow.xyz(location.bankDestination.tile)] : null])"
        ),
        serde_json::json!([
            ["Lumbridge cow field", [3255, 3288, 0], true, null],
            ["North-west of Lumbridge", [3168, 3329, 0], false, ["Draynor", [3093, 3243, 0]]],
            ["South of Falador", [3033, 3306, 0], false, ["Falador East", [3013, 3355, 0]]],
            ["East Ardougne cow field", [2664, 3347, 0], false, ["Ardougne West", [2616, 3332, 0]]]
        ])
    );
    assert_eq!(
        probe(&isolate, "__cow.options"),
        serde_json::json!([
            "Auto",
            "Lumbridge cow field",
            "North-west of Lumbridge",
            "South of Falador",
            "East Ardougne cow field",
            "Start tile"
        ])
    );
    assert_eq!(
        probe(&isolate, "__cow.bankConstants.map(__cow.xyz)"),
        serde_json::json!([
            [3269, 3167, 0],
            [3093, 3243, 0],
            [3013, 3355, 0],
            [2616, 3332, 0]
        ])
    );
    assert_eq!(probe(&isolate, "__cow.coinTarget"), serde_json::json!(20));
    assert_eq!(
        probe(
            &isolate,
            "__cow.bankConstants.every(__cow.isTile) && __cow.locations.every(location => __cow.isTile(location.anchor) && (!location.bankDestination || __cow.isTile(location.bankDestination.tile)))"
        ),
        serde_json::json!(true),
        "the public values must remain Tile instances"
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.locations[1].bankDestination.tile === __cow.draynor, __cow.locations[2].bankDestination.tile === __cow.faladorEast, __cow.locations[3].bankDestination.tile === __cow.ardougneWest, __cow.bankDestination(__cow.locations[1], false) === __cow.locations[1].bankDestination, __cow.bankDestination(__cow.locations[0], false) === null, __cow.bankDestination(__cow.locations[0], true).tile === __cow.alKharid]"
        ),
        serde_json::json!([true, true, true, true, true, true]),
        "bank constants, nested destinations, and returned destinations must keep their object identity"
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.bankDestination(__cow.locations[0], true).name, __cow.xyz(__cow.bankDestination(__cow.locations[0], true).tile)]"
        ),
        serde_json::json!(["Al Kharid", [3269, 3167, 0]])
    );

    isolate.join();
}

#[test]
fn public_resolution_keeps_named_auto_and_stable_nearest_identity() {
    let isolate = load(None, NamedBankFacts::empty());

    assert_eq!(
        probe(
            &isolate,
            "__cow.locations.map(location => __cow.resolve(location.name, null) === location && __cow.resolve(` ${location.name.toUpperCase()} `, null) === location && __cow.resolve('Auto', location.anchor) === location && __cow.nearest(location.anchor) === location)"
        ),
        serde_json::json!([true, true, true, true]),
        "named resolution and Auto/nearest must return the original catalog rows"
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.resolve('auto', {x: 3269, z: 3167, level: 0}).name, __cow.resolve(' sOuTh Of FaLaDoR ', {x: 0, z: 0, level: 0}) === __cow.locations[2], __cow.resolve('Start tile', __cow.locations[0].anchor) === null, __cow.resolve('unknown', __cow.locations[0].anchor) === null]"
        ),
        serde_json::json!(["Lumbridge cow field", true, true, true])
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.nearest({x: 2686, z: 3000, level: 0}).name, __cow.nearest({x: 2686, z: 3000, level: 0}).name]"
        ),
        serde_json::json!(["South of Falador", "South of Falador"]),
        "equal nearest distances must retain the first catalog row"
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.resolve('Auto') === null, __cow.resolve('Auto', null) === null, __cow.resolve('Auto', {x: 3255.5, z: 3288, level: 0}) === null, __cow.nearest() === null, __cow.nearest({x: 2147483648, z: 3288, level: 0}) === null, __cow.nearest({x: 3255, z: 3288}) === __cow.locations[0]]"
        ),
        serde_json::json!([true, true, true, true, true, true]),
        "invalid x/z values fail closed, while an omitted level keeps the existing zero-level default"
    );

    isolate.join();
}

#[test]
fn loot_leash_has_inclusive_chebyshev_bounds_and_rejects_other_planes() {
    let isolate = load(None, NamedBankFacts::empty());

    assert_eq!(
        probe(
            &isolate,
            "[__cow.loot(__cow.locations[0].anchor, 18, {x: 3273, z: 3306, level: 0}), __cow.loot(__cow.locations[0].anchor, 18, {x: 3274, z: 3306, level: 0}), __cow.loot(__cow.locations[0].anchor, 18, {x: 3273, z: 3306, level: 1}), __cow.loot(__cow.locations[0].anchor, 0, __cow.locations[0].anchor), __cow.loot(__cow.locations[0].anchor, 0, {x: 3256, z: 3288, level: 0}), __cow.loot(__cow.locations[0].anchor, -1, __cow.locations[0].anchor), __cow.loot(__cow.locations[0].anchor, NaN, __cow.locations[0].anchor)]"
        ),
        serde_json::json!([true, false, false, true, false, false, false])
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.loot(null, 18, __cow.locations[0].anchor), __cow.loot(__cow.locations[0].anchor, 18, null), __cow.loot({x: 3255.5, z: 3288, level: 0}, 18, __cow.locations[0].anchor), __cow.loot(__cow.locations[0].anchor, 18, {x: 3255, z: 3288})]"
        ),
        serde_json::json!([false, false, false, true]),
        "invalid anchor and tile coordinates fail closed; an omitted level defaults to zero"
    );

    isolate.join();
}

#[test]
fn toll_helpers_honor_enablement_coin_and_distance_boundaries() {
    let isolate = load(None, NamedBankFacts::empty());

    assert_eq!(
        probe(
            &isolate,
            "[__cow.needsToll(__cow.locations[0], true), __cow.needsToll(__cow.locations[0], false), __cow.needsToll(__cow.locations[1], true), __cow.needsToll(null, true)]"
        ),
        serde_json::json!([true, false, false, false])
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.bootstrap(__cow.locations[0], {x: 3269, z: 3167, level: 0}, 0, true), __cow.bootstrap(__cow.locations[0], {x: 3269, z: 3167, level: 0}, 19.999, true), __cow.bootstrap(__cow.locations[0], {x: 3269, z: 3167, level: 0}, 20, true), __cow.bootstrap(__cow.locations[0], {x: 3269, z: 3167, level: 0}, 20.001, true), __cow.bootstrap(__cow.locations[0], {x: 3349, z: 3167, level: 0}, 0, true), __cow.bootstrap(__cow.locations[0], {x: 3350, z: 3167, level: 0}, 0, true), __cow.bootstrap(__cow.locations[0], {x: 3269, z: 3167, level: 1}, 0, true), __cow.bootstrap(__cow.locations[0], {x: 3269, z: 3167, level: 0}, 0, false), __cow.bootstrap(__cow.locations[2], {x: 3269, z: 3167, level: 0}, 0, true), __cow.bootstrap(null, {x: 3269, z: 3167, level: 0}, 0, true), __cow.bootstrap(__cow.locations[0], {x: 3269.5, z: 3167, level: 0}, 0, true)]"
        ),
        serde_json::json!([true, true, false, false, true, false, false, false, false, false, false]),
        "bootstrap uses a strict coin threshold, an inclusive 80-tile radius, and plane-aware distance"
    );

    isolate.join();
}

#[test]
fn selected_named_bank_content_feeds_exports_and_nested_destinations() {
    let data = selected_data();
    let banks = selected_banks(&data);
    let expected = [
        xyz(bank_tile(&banks, "Al Kharid")),
        xyz(bank_tile(&banks, "Draynor")),
        xyz(bank_tile(&banks, "Falador East")),
        xyz(bank_tile(&banks, "Ardougne West")),
    ];
    let isolate = load(Some(data), banks.clone());

    assert_eq!(
        probe(&isolate, "__cow.bankConstants.map(__cow.xyz)"),
        serde_json::json!(expected),
        "public bank constants must use the selected named-bank tiles"
    );
    assert_eq!(
        probe(
            &isolate,
            "__cow.locations.slice(1).map(location => [location.bankDestination.name, __cow.xyz(location.bankDestination.tile)])"
        ),
        serde_json::json!([
            ["Draynor", expected[1]],
            ["Falador East", expected[2]],
            ["Ardougne West", expected[3]]
        ])
    );
    assert_eq!(
        probe(
            &isolate,
            "[__cow.locations[1].bankDestination.tile === __cow.draynor, __cow.locations[2].bankDestination.tile === __cow.faladorEast, __cow.locations[3].bankDestination.tile === __cow.ardougneWest, __cow.bankDestination(__cow.locations[0], true).tile === __cow.alKharid]"
        ),
        serde_json::json!([true, true, true, true]),
        "selected bank tiles must retain one shared object identity in the public module"
    );

    isolate.join();
}
