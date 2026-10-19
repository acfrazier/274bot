#![cfg(feature = "load")]

use script::content::RUNE_ROUTES;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

const SRC: &str = r#"
import Tile from '../../geometry/Tile.js';
import { RUNES, RUNE_OPTIONS, DEFAULT_RUNE } from '../../data/runeCraftLocations.js';

const keys = Object.keys(RUNES);
globalThis.__rune_catalog_probe = {
    selectedFacts: globalThis.__rs2b0t_host.content.selected_facts,
    keys,
    options: RUNE_OPTIONS,
    defaultRune: DEFAULT_RUNE,
    rows: keys.map((name) => {
        const route = RUNES[name];
        return {
            name,
            routeKeys: Object.keys(route),
            rune: route.rune,
            talisman: route.talisman,
            level: route.level,
            bank: route.bank,
            ruins: {
                x: route.ruins.x,
                z: route.ruins.z,
                level: route.ruins.level,
            },
            ruinsKeys: Object.keys(route.ruins),
            isTile: route.ruins instanceof Tile,
            hasTilePrototype: Object.getPrototypeOf(route.ruins) === Tile.prototype,
        };
    }),
};

const runesBeforeContentChange = RUNES;
const optionsBeforeContentChange = RUNE_OPTIONS;
const airBeforeContentChange = RUNES['Air rune'];
globalThis.__rs2b0t_host.content = {};
globalThis.__rune_retention_probe = {
    sameRunes: RUNES === runesBeforeContentChange,
    sameOptions: RUNE_OPTIONS === optionsBeforeContentChange,
    sameAir: RUNES['Air rune'] === airBeforeContentChange,
    keys: Object.keys(RUNES),
    options: RUNE_OPTIONS,
    defaultRune: DEFAULT_RUNE,
    air: {
        bank: RUNES['Air rune'].bank,
        x: RUNES['Air rune'].ruins.x,
        z: RUNES['Air rune'].ruins.z,
        level: RUNES['Air rune'].ruins.level,
        isTile: RUNES['Air rune'].ruins instanceof Tile,
    },
};

const rejectsWithTypeError = (invoke) => {
    try {
        invoke();
        return false;
    } catch (error) {
        return error instanceof TypeError;
    }
};
globalThis.__rune_invalid_input_probe = {
    missingTileConstructor: rejectsWithTypeError(() =>
        __rs2b0t_rune_craft_locations()
    ),
    nonConstructorTile: rejectsWithTypeError(() =>
        __rs2b0t_rune_craft_locations({})
    ),
};

export default class RuneCatalogProbe extends LoopingBot {
    loop() {}
}
"#;

fn spawn_without_selected_game_data() -> LoadIsolate {
    LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![])
        .expect("runecrafting catalog loads without selected game data")
}

fn expected_route(route: &script::content::RuneRoute) -> Value {
    json!({
        "rune": route.rune,
        "talisman": route.talisman,
        "level": route.level,
        "bank": route.bank,
        "ruins": {
            "x": route.ruins.x,
            "z": route.ruins.z,
            "level": route.ruins.level,
        },
    })
}

#[test]
fn runecrafting_catalog_preserves_routes_shape_order_and_retention() {
    let iso = spawn_without_selected_game_data();
    let catalog = iso
        .probe("__rune_catalog_probe")
        .expect("catalog probe is available");
    let retention = iso
        .probe("__rune_retention_probe")
        .expect("retention probe is available");
    let invalid = iso
        .probe("__rune_invalid_input_probe")
        .expect("invalid-input probe is available");
    iso.join();

    let expected_names = RUNE_ROUTES
        .iter()
        .map(|route| route.rune)
        .collect::<Vec<_>>();
    assert_eq!(catalog["selectedFacts"], json!(false));
    assert_eq!(catalog["keys"], json!(expected_names));
    assert_eq!(catalog["options"], json!(expected_names));
    assert_eq!(catalog["defaultRune"], json!("Air rune"));

    let rows = catalog["rows"]
        .as_array()
        .expect("catalog rows are an array");
    assert_eq!(rows.len(), RUNE_ROUTES.len());
    for (observed, route) in rows.iter().zip(RUNE_ROUTES) {
        assert_eq!(observed["name"], json!(route.rune));
        assert_eq!(observed["routeKeys"], json!(["rune", "talisman", "level", "bank", "ruins"]));
        assert_eq!(
            json!({
                "rune": observed["rune"],
                "talisman": observed["talisman"],
                "level": observed["level"],
                "bank": observed["bank"],
                "ruins": observed["ruins"],
            }),
            expected_route(route),
            "route {} must match the Rust route roster",
            route.rune
        );
        assert_eq!(observed["ruinsKeys"], json!(["x", "z", "level"]));
        assert_eq!(observed["isTile"], json!(true));
        assert_eq!(observed["hasTilePrototype"], json!(true));
    }

    assert_eq!(retention["sameRunes"], json!(true));
    assert_eq!(retention["sameOptions"], json!(true));
    assert_eq!(retention["sameAir"], json!(true));
    assert_eq!(retention["keys"], json!(expected_names));
    assert_eq!(retention["options"], json!(expected_names));
    assert_eq!(retention["defaultRune"], json!("Air rune"));
    assert_eq!(
        retention["air"],
        expected_route(RUNE_ROUTES.first().expect("default rune route exists"))
            .as_object()
            .map(|route| {
                json!({
                    "bank": route["bank"],
                    "x": route["ruins"]["x"],
                    "z": route["ruins"]["z"],
                    "level": route["ruins"]["level"],
                    "isTile": true,
                })
            })
            .expect("route record is an object")
    );
    assert_eq!(invalid["missingTileConstructor"], json!(true));
    assert_eq!(invalid["nonConstructorTile"], json!(true));
}
