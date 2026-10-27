//! Served cooking-catalog compatibility across API and data module aliases.

use std::sync::Arc;

use api::game_data::SelectedGameData;
use api::named_banks::{NamedBank, NamedBankFacts, BANK_CATALOG};
use {script_js::LoadIsolate, script_js::LoadIsolateError, script_js::LoadShape};

fn data() -> Arc<SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()
}

fn roster() -> NamedBankFacts {
    NamedBankFacts::from_banks(
        BANK_CATALOG
            .iter()
            .map(|definition| NamedBank {
                definition: Some(definition),
                ..NamedBank::new(definition.name, definition.tile)
            })
            .collect(),
    )
}

fn spawn_with(
    src: &str,
    game_data: Option<Arc<SelectedGameData>>,
    facts: NamedBankFacts,
) -> Result<LoadIsolate, LoadIsolateError> {
    LoadIsolate::spawn_with_content(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        game_data,
        Arc::new(facts),
    )
}

fn spawn(src: &str) -> LoadIsolate {
    spawn_with(src, Some(data()), roster()).unwrap()
}

fn probe(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr).unwrap()
}

const CATALOG_SOURCE: &str = r#"
import {
    COOK_LOCATIONS,
    COOK_LOCATION_OPTIONS,
    CUSTOM_LOCATION,
    cookLocation,
    resolveCookLocation,
} from '../../api/cooking/CookLocations.js';
import {
    CUSTOM_LOCATION as DATA_CUSTOM_LOCATION,
    MAX_SURFACE_CHEB,
    nearestCookSurface,
    buildCookLocations,
    findCookLocation,
    COOK_LOCATIONS as DATA_COOK_LOCATIONS,
} from '../../data/cookLocations.js';
import { BANK_LOCATIONS } from '../../api/bank/BankLocations.js';
import Tile from '../../geometry/Tile.js';

const catherby = cookLocation(' CATHERBY ');
const draynor = cookLocation('Draynor');
const alKharid = cookLocation('Al Kharid');
const tieOven = nearestCookSurface(new Tile(2725, 3491, 0));
const nearestRange = nearestCookSurface(new Tile(2817, 3450, 0));
const atZero = nearestCookSurface(new Tile(2817, 3444, 0), 0);
const belowZero = nearestCookSurface(new Tile(2817, 3444, 0), -1);
const atMaximumDistance = nearestCookSurface(new Tile(2838, 3455, 0), 20);
const beyondMaximumDistance = nearestCookSurface(new Tile(2838, 3455, 0), 19);
const wrongPlane = nearestCookSurface(new Tile(2817, 3444, 1));
const tooFar = nearestCookSurface(new Tile(3094, 3493, 0));

globalThis.__cooking_catalog = {
    apiDataArrayIdentity: COOK_LOCATIONS === DATA_COOK_LOCATIONS,
    apiDataCustomIdentity: CUSTOM_LOCATION === DATA_CUSTOM_LOCATION,
    names: COOK_LOCATIONS.map((location) => location.name),
    options: COOK_LOCATION_OPTIONS,
    everyBankUsesCatalogIdentity: COOK_LOCATIONS.every((location) => {
        const bank = BANK_LOCATIONS.find((candidate) => candidate.name === location.name);
        return Boolean(bank) && location.bank === bank && location.bank.tile === bank.tile;
    }),
    cookLocationIdentity: catherby === COOK_LOCATIONS.find((location) => location.name === 'Catherby'),
    findIdentity: findCookLocation(DATA_COOK_LOCATIONS, 'catherby') === catherby,
    buildIdentity: buildCookLocations() === COOK_LOCATIONS,
    buildIgnoresLegacyArgument: buildCookLocations([{ name: 'ignored' }]) === COOK_LOCATIONS,
    customLocation: CUSTOM_LOCATION,
    maxSurfaceCheb: MAX_SURFACE_CHEB,
    curatedCatherby: [
        catherby.bank.tile.x, catherby.bank.tile.z, catherby.bank.tile.level,
        catherby.surface.stand.x, catherby.surface.stand.z, catherby.surface.stand.level,
        catherby.surface.loc.x, catherby.surface.loc.z, catherby.surface.loc.level,
        catherby.surface.locName, catherby.surface.kind, catherby.surface.arriveRadius,
        catherby.verified,
    ],
    curatedDraynor: [
        draynor.surface.stand.x, draynor.surface.stand.z,
        draynor.surface.approach.x, draynor.surface.approach.z,
        draynor.surface.loc.x, draynor.surface.loc.z,
        draynor.surface.locName, draynor.surface.kind, draynor.verified,
    ],
    generatedAlKharid: [
        alKharid.surface.stand.x, alKharid.surface.stand.z,
        alKharid.surface.loc.x, alKharid.surface.loc.z,
        alKharid.surface.arriveRadius, alKharid.verified,
    ],
    allSurfacesStayOnBankPlaneAndWithinLimit: COOK_LOCATIONS.every((location) => {
        if (!location.surface) return true;
        const bank = location.bank.tile;
        const loc = location.surface.loc;
        return loc.level === bank.level
            && Math.max(Math.abs(loc.x - bank.x), Math.abs(loc.z - bank.z)) <= MAX_SURFACE_CHEB;
    }),
    catherbyTilesAreTileInstances: catherby.surface.stand instanceof Tile
        && catherby.surface.loc instanceof Tile,
    nearestOven: tieOven && [tieOven.x, tieOven.z, tieOven.kind],
    nearestSameKind: nearestRange && [nearestRange.x, nearestRange.z, nearestRange.kind],
    atMaximumDistance: atMaximumDistance && [
        atMaximumDistance.x, atMaximumDistance.z, atMaximumDistance.kind,
    ],
    beyondMaximumDistanceIsNull: beyondMaximumDistance === null,
    maxDistanceAtZero: atZero !== null,
    maxDistanceBelowZero: belowZero === null,
    wrongPlaneIsNull: wrongPlane === null,
    outsideSurfaceRangeIsNull: tooFar === null,
    noMatchIsNull: cookLocation('not a bank') === null,
};

export default class T extends LoopingBot {
    loop() {}
}
"#;

const FIND_SOURCE: &str = r#"
import { COOK_LOCATIONS, cookLocation } from '../../api/cooking/CookLocations.js';
import { findCookLocation } from '../../data/cookLocations.js';

const first = { name: 'FoO', marker: 1 };
const second = { name: 'foo', marker: 2 };
const falseyRows = findCookLocation([undefined, null, false, 0, '', first, second], ' FOO ');
const trace = [];
const target = {
    [Symbol.toPrimitive](hint) {
        trace.push(`target:${hint}`);
        return ' fOo ';
    },
};
const rowName = {
    [Symbol.toPrimitive](hint) {
        trace.push(`row:${hint}`);
        return 'foo';
    },
};
const rowWithGetter = {
    get name() {
        trace.push('name');
        return rowName;
    },
};
const coerced = findCookLocation([rowWithGetter], target);
const symbolRow = { name: Symbol('COOK') };
const symbolFound = findCookLocation([symbolRow], Symbol('cook'));
const whitespaceRow = findCookLocation([{ name: ' Foo ' }], 'foo');
let customFindReceiver = false;
const customRows = {
    find(predicate) {
        customFindReceiver = this === customRows;
        return predicate(first, 0, this) ? first : undefined;
    },
};
const customFound = findCookLocation(customRows, 'foo');
const sentinel = Object.freeze({ source: 'find-cook-location' });
const throwingRow = Object.defineProperty({}, 'name', {
    get() { throw sentinel; },
});
const throwingName = {
    [Symbol.toPrimitive]() { throw sentinel; },
};
const throwingFind = Object.defineProperty({}, 'find', {
    get() { throw sentinel; },
});
const throwingTarget = {
    [Symbol.toPrimitive]() { throw sentinel; },
};
const sameThrown = (call) => {
    try { call(); } catch (error) { return error === sentinel; }
    return false;
};
let invalidFindTypeError = false;
try {
    findCookLocation({}, 'foo');
} catch (error) {
    invalidFindTypeError = error instanceof TypeError;
}

globalThis.__cooking_find = {
    firstTie: findCookLocation([first, second], ' FOO ') === first,
    falseyRowsKeepFirstMatch: falseyRows === first,
    falseyCollectionsReturnNull: [null, undefined, false, 0, '']
        .every((locations) => findCookLocation(locations, 'foo') === null),
    absentReturnsNull: findCookLocation([first], 'missing') === null,
    rowNameIsNotTrimmed: whitespaceRow === null,
    symbolUsesStringCoercion: symbolFound === symbolRow,
    targetAndRowCoercionOrder: trace,
    getterErrorIdentity: sameThrown(() => findCookLocation([throwingRow], 'foo')),
    rowStringErrorIdentity: sameThrown(() => findCookLocation([{ name: throwingName }], 'foo')),
    findGetterErrorIdentity: sameThrown(() => findCookLocation(throwingFind, 'foo')),
    targetStringErrorIdentity: sameThrown(() => findCookLocation([first], throwingTarget)),
    customFindReceiverAndPredicate: customFindReceiver && customFound === first,
    invalidFindTypeError,
    apiCookLocationUsesCatalogIdentity: cookLocation(' catherby ') === COOK_LOCATIONS.find((location) => location.name === 'Catherby'),
};

export default class T extends LoopingBot {
    loop() {}
}
"#;

const RESOLVE_SOURCE: &str = r#"
import { COOK_LOCATIONS, resolveCookLocation } from '../../api/cooking/CookLocations.js';
import Tile from '../../geometry/Tile.js';

const names = COOK_LOCATIONS.map((location) => location.name);
const catherby = COOK_LOCATIONS.find((location) => location.name === 'Catherby');
const autoOrder = [];
const auto = resolveCookLocation(' Auto ', new Tile(3253, 3420, 0), (location) => {
    autoOrder.push(location.name);
    return location.name === 'Varrock East';
});
const namedArgs = [];
const named = resolveCookLocation(' CATHERBY ', null, (location) => {
    namedArgs.push(location);
    return true;
});
const conversionTrace = [];
const convertedSetting = resolveCookLocation({
    [Symbol.toPrimitive](hint) {
        conversionTrace.push(hint);
        return ' cAtHeRbY ';
    },
}, undefined, () => true);
const noPredicateCalls = { count: 0 };
const custom = resolveCookLocation('Custom', null, () => { noPredicateCalls.count++; return true; });
const blank = resolveCookLocation('   ', null, () => { noPredicateCalls.count++; return true; });
const unknown = resolveCookLocation('Nowhere', null, () => { noPredicateCalls.count++; return true; });
let unusedFromReads = 0;
const unusedFrom = new Proxy({}, {
    get() { unusedFromReads++; throw new Error('named resolution read from'); },
});
const namedWithUnusedFrom = resolveCookLocation('Catherby', unusedFrom, () => true);
const sentinel = Object.freeze({ source: 'resolve-cook-location' });
const errorOrder = [];
let predicateErrorIdentity = false;
try {
    resolveCookLocation('Auto', new Tile(3253, 3420, 0), (location) => {
        errorOrder.push(location.name);
        if (errorOrder.length === 3) throw sentinel;
        return false;
    });
} catch (error) {
    predicateErrorIdentity = error === sentinel;
}
let settingErrorIdentity = false;
try {
    resolveCookLocation({ [Symbol.toPrimitive]() { throw sentinel; } }, null);
} catch (error) {
    settingErrorIdentity = error === sentinel;
}
let nonFunctionErrorName = '';
try {
    resolveCookLocation('Catherby', undefined, null);
} catch (error) {
    nonFunctionErrorName = error.name;
}

globalThis.__cooking_resolve = {
    names,
    autoName: auto && auto.name,
    autoOrder,
    namedName: named && named.name,
    namedPredicateCalledOnceWithCatalogIdentity: namedArgs.length === 1 && namedArgs[0] === catherby,
    settingCoercion: conversionTrace,
    convertedSettingName: convertedSetting && convertedSetting.name,
    customIsNull: custom === null,
    blankIsNull: blank === null,
    unknownIsNull: unknown === null,
    ignoredPredicates: noPredicateCalls.count,
    namedResolutionDoesNotReadFrom: namedWithUnusedFrom === catherby && unusedFromReads === 0,
    defaultCatherby: resolveCookLocation('Catherby', null)?.name ?? null,
    defaultZanarisIsLocked: resolveCookLocation('Zanaris', null) === null,
    explicitNamedLockIsNull: resolveCookLocation('Catherby', null, () => false) === null,
    explicitAutoLockIsNull: resolveCookLocation('Auto', new Tile(3253, 3420, 0), () => false) === null,
    customPredicateCanUnlockZanaris: resolveCookLocation('Zanaris', null, () => true)?.name ?? null,
    predicateErrorIdentity,
    errorOrder,
    settingErrorIdentity,
    nonFunctionErrorName,
};

export default class T extends LoopingBot {
    loop() {}
}
"#;

const MISSING_DATA_SOURCE: &str = r#"
import {
    COOK_LOCATIONS,
    COOK_LOCATION_OPTIONS,
    cookLocation,
    resolveCookLocation,
} from '../../api/cooking/CookLocations.js';
import {
    buildCookLocations,
    findCookLocation,
    nearestCookSurface,
} from '../../data/cookLocations.js';

let resolutionError = '';
try {
    resolveCookLocation('Catherby');
} catch (error) {
    resolutionError = error.message;
}
let unknownResolutionError = '';
try {
    resolveCookLocation('Nowhere');
} catch (error) {
    unknownResolutionError = error.message;
}

globalThis.__cooking_offline = {
    locationCount: COOK_LOCATIONS.length,
    options: COOK_LOCATION_OPTIONS,
    cookLocationIsNull: cookLocation('Catherby') === null,
    findIsNull: findCookLocation(null, 'Catherby') === null,
    buildReturnsPrebuiltArray: buildCookLocations([{ name: 'ignored' }]) === COOK_LOCATIONS,
    customIsNull: resolveCookLocation('Custom') === null,
    blankIsNull: resolveCookLocation('  ') === null,
    unknownResolutionError,
    nearestIsNull: nearestCookSurface(undefined) === null,
    resolutionError,
};

export default class T extends LoopingBot {
    loop() {}
}
"#;

#[test]
fn runtime_catalog_exports_keep_aliases_options_surfaces_and_boundaries() {
    let iso = spawn(CATALOG_SOURCE);
    let catalog = probe(&iso, "__cooking_catalog");
    let expected_names: Vec<_> = roster()
        .banks()
        .iter()
        .map(|bank| bank.name.to_string())
        .collect();
    let mut expected_options = vec!["Auto".to_string()];
    expected_options.extend(expected_names.iter().cloned());
    expected_options.push("Custom".to_string());

    assert_eq!(catalog["names"], serde_json::json!(expected_names));
    assert_eq!(catalog["options"], serde_json::json!(expected_options));
    assert_eq!(catalog["apiDataArrayIdentity"], true);
    assert_eq!(catalog["apiDataCustomIdentity"], true);
    assert_eq!(catalog["everyBankUsesCatalogIdentity"], true);
    assert_eq!(catalog["cookLocationIdentity"], true);
    assert_eq!(catalog["findIdentity"], true);
    assert_eq!(catalog["buildIdentity"], true);
    assert_eq!(catalog["buildIgnoresLegacyArgument"], true);
    assert_eq!(catalog["customLocation"], "Custom");
    assert_eq!(catalog["maxSurfaceCheb"], 20);
    assert_eq!(
        catalog["curatedCatherby"],
        serde_json::json!([2809, 3441, 0, 2817, 3443, 0, 2817, 3444, 0, "Range", "oven", 0, true])
    );
    assert_eq!(
        catalog["curatedDraynor"],
        serde_json::json!([
            3100,
            3257,
            3102,
            3258,
            3100,
            3256,
            "Fireplace",
            "fire",
            true
        ])
    );
    assert_eq!(
        catalog["generatedAlKharid"],
        serde_json::json!([3271, 3179, 3271, 3180, 2, false])
    );
    assert_eq!(catalog["allSurfacesStayOnBankPlaneAndWithinLimit"], true);
    assert_eq!(catalog["catherbyTilesAreTileInstances"], true);
    assert_eq!(
        catalog["nearestOven"],
        serde_json::json!([2715, 3476, "oven"])
    );
    assert_eq!(
        catalog["nearestSameKind"],
        serde_json::json!([2818, 3455, "oven"])
    );
    assert_eq!(
        catalog["atMaximumDistance"],
        serde_json::json!([2818, 3455, "oven"])
    );
    assert_eq!(catalog["beyondMaximumDistanceIsNull"], true);
    assert_eq!(catalog["maxDistanceAtZero"], true);
    assert_eq!(catalog["maxDistanceBelowZero"], true);
    assert_eq!(catalog["wrongPlaneIsNull"], true);
    assert_eq!(catalog["outsideSurfaceRangeIsNull"], true);
    assert_eq!(catalog["noMatchIsNull"], true);
    iso.join();
}

#[test]
fn caller_row_lookup_preserves_coercion_order_identity_and_errors() {
    let iso = spawn(FIND_SOURCE);
    let found = probe(&iso, "__cooking_find");
    assert_eq!(found["firstTie"], true);
    assert_eq!(found["falseyRowsKeepFirstMatch"], true);
    assert_eq!(found["falseyCollectionsReturnNull"], true);
    assert_eq!(found["absentReturnsNull"], true);
    assert_eq!(found["rowNameIsNotTrimmed"], true);
    assert_eq!(found["symbolUsesStringCoercion"], true);
    assert_eq!(
        found["targetAndRowCoercionOrder"],
        serde_json::json!(["target:string", "name", "row:string"])
    );
    assert_eq!(found["getterErrorIdentity"], true);
    assert_eq!(found["rowStringErrorIdentity"], true);
    assert_eq!(found["findGetterErrorIdentity"], true);
    assert_eq!(found["targetStringErrorIdentity"], true);
    assert_eq!(found["customFindReceiverAndPredicate"], true);
    assert_eq!(found["invalidFindTypeError"], true);
    assert_eq!(found["apiCookLocationUsesCatalogIdentity"], true);
    iso.join();
}

#[test]
fn resolver_keeps_predicate_order_errors_and_default_unlocks() {
    let iso = spawn(RESOLVE_SOURCE);
    let resolved = probe(&iso, "__cooking_resolve");
    let expected_names: Vec<_> = roster()
        .banks()
        .iter()
        .map(|bank| bank.name.to_string())
        .collect();
    assert_eq!(resolved["names"], serde_json::json!(expected_names));
    assert_eq!(resolved["autoName"], "Varrock East");
    assert_eq!(resolved["autoOrder"], serde_json::json!(expected_names));
    assert_eq!(resolved["namedName"], "Catherby");
    assert_eq!(
        resolved["namedPredicateCalledOnceWithCatalogIdentity"],
        true
    );
    assert_eq!(resolved["settingCoercion"], serde_json::json!(["string"]));
    assert_eq!(resolved["convertedSettingName"], "Catherby");
    assert_eq!(resolved["customIsNull"], true);
    assert_eq!(resolved["blankIsNull"], true);
    assert_eq!(resolved["unknownIsNull"], true);
    assert_eq!(resolved["ignoredPredicates"], 0);
    assert_eq!(resolved["namedResolutionDoesNotReadFrom"], true);
    assert_eq!(resolved["defaultCatherby"], "Catherby");
    assert_eq!(resolved["defaultZanarisIsLocked"], true);
    assert_eq!(resolved["explicitNamedLockIsNull"], true);
    assert_eq!(resolved["explicitAutoLockIsNull"], true);
    assert_eq!(resolved["customPredicateCanUnlockZanaris"], "Zanaris");
    assert_eq!(resolved["predicateErrorIdentity"], true);
    assert_eq!(
        resolved["errorOrder"],
        serde_json::json!(expected_names.iter().take(3).collect::<Vec<_>>())
    );
    assert_eq!(resolved["settingErrorIdentity"], true);
    assert_eq!(resolved["nonFunctionErrorName"], "TypeError");
    iso.join();
}

#[test]
fn absent_selected_data_keeps_empty_catalog_and_explicit_resolution_error() {
    let iso = spawn_with(MISSING_DATA_SOURCE, None, roster()).unwrap();
    let offline = probe(&iso, "__cooking_offline");
    assert_eq!(offline["locationCount"], 0);
    assert_eq!(offline["options"], serde_json::json!(["Auto", "Custom"]));
    assert_eq!(offline["cookLocationIsNull"], true);
    assert_eq!(offline["findIsNull"], true);
    assert_eq!(offline["buildReturnsPrebuiltArray"], true);
    assert_eq!(offline["customIsNull"], true);
    assert_eq!(offline["blankIsNull"], true);
    assert_eq!(
        offline["unknownResolutionError"],
        offline["resolutionError"]
    );
    assert_eq!(offline["nearestIsNull"], true);
    assert_eq!(
        offline["resolutionError"],
        "game data unavailable: this server's content isn't verified (see profile/engine settings)"
    );
    iso.join();
}
