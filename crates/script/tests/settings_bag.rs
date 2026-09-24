// Task 5: operator settings bag — prelude reads host-posted values; store
// persists overrides keyed by (source, name) at 0o600.

use script::{LoadIsolate, LoadShape, ScriptSource, SettingDef};

const SETTINGS_PROBE: &str = r#"
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            leash: this.settings.num('leashRadius', 0),
            bury: this.settings.bool('buryBones', false),
            style: this.settings.str('combatStyle', ''),
        };
    }
}
"#;

#[test]
fn prelude_reads_posted_settings_bag_not_only_fallback() {
    let iso = LoadIsolate::spawn(SETTINGS_PROBE.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn settings probe");
    let mut bag = serde_json::Map::new();
    bag.insert("leashRadius".into(), serde_json::json!(25));
    bag.insert("buryBones".into(), serde_json::json!(false));
    bag.insert("combatStyle".into(), serde_json::json!("mage"));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("settings probe readable");
    assert_eq!(value["leash"], 25, "num reads posted bag, not fallback 0");
    assert_eq!(value["bury"], false, "bool reads posted bag");
    assert_eq!(value["style"], "mage", "str reads posted bag");
    iso.join();
}

// The posted bag is the value `JSON.parse` would give: ordinary objects at
// every level, JS Numbers for every integer, and object values a shim can
// stringify.
#[test]
fn posted_bag_values_are_plain_json_values() {
    let src = r#"
import { SettingsStore } from '../../runtime/Settings.js';
export default class T extends LoopingBot {
    loop() {
        const bag = globalThis.__rs2b0t_host.settingsBag;
        globalThis.__probe = {
            bagProto: Object.getPrototypeOf(bag) === Object.prototype,
            nestedProto: Object.getPrototypeOf(bag.spots.north) === Object.prototype,
            rowProto: Object.getPrototypeOf(bag.rows[0]) === Object.prototype,
            nestedOwn: bag.spots.hasOwnProperty('north'),
            bigType: typeof bag.big,
            bigValue: bag.big === 9007199254740992,
            negType: typeof bag.neg,
            protoKeyOwn: Object.prototype.hasOwnProperty.call(bag.odd, '__proto__'),
            protoKeyKept: Object.getPrototypeOf(bag.odd) === Object.prototype,
            display: SettingsStore.displayString('T', 'camp', { default: '' }),
            saved: SettingsStore.saved('T', 'rows'),
            text: `${bag.camp}`,
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn settings shape probe");
    let bag = serde_json::json!({
        "spots": { "north": { "x": 1, "z": 2, "level": 0 } },
        "rows": [{ "item": "Lobster" }],
        "big": 9007199254740993u64,
        "neg": -9007199254740993i64,
        "odd": { "__proto__": { "polluted": true } },
        "camp": { "name": "north camp" },
    });
    iso.post_settings_bag(bag.as_object().unwrap());
    iso.on_game_tick(1);
    let logs = iso.drain_logs();
    assert_eq!(
        iso.probe("__probe").expect("probe readable"),
        serde_json::json!({
            "bagProto": true,
            "nestedProto": true,
            "rowProto": true,
            "nestedOwn": true,
            "bigType": "number",
            "bigValue": true,
            "negType": "number",
            "protoKeyOwn": true,
            "protoKeyKept": true,
            "display": "[object Object]",
            "saved": "[object Object]",
            "text": "[object Object]",
        }),
        "logs: {logs:?}"
    );
    iso.join();
}

#[test]
fn settings_store_round_trips_overrides_at_private_mode() {
    let dir = std::env::temp_dir().join(format!(
        "274bot-script-settings-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("script-settings.json");

    let schema = vec![
        SettingDef {
            id: "buryBones".into(),
            ty: "boolean".into(),
            default: Some("true".into()),
            label: Some("Bury bones".into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        },
        SettingDef {
            id: "leashRadius".into(),
            ty: "number".into(),
            default: Some("12".into()),
            label: Some("Leash".into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        },
    ];

    {
        let mut store = script::ScriptSettingsStore::at(path.clone());
        store.set_bool(ScriptSource::Catalog, "ChickenKiller", "buryBones", false);
        store.set_num(ScriptSource::Catalog, "ChickenKiller", "leashRadius", 20.0);
        store.save().expect("save settings");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "script-settings.json must be 0o600");
    }

    let store = script::ScriptSettingsStore::at(path);
    let bag = store.merged_bag(ScriptSource::Catalog, "ChickenKiller", &schema, None);
    assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(false)));
    assert_eq!(bag.get("leashRadius").and_then(|v| v.as_f64()), Some(20.0));
    assert_eq!(
        store.card_key(ScriptSource::File, "BoneBurier"),
        "file:BoneBurier"
    );
}

const TILE_LIST_PROBE: &str = r#"
import Tile from '../../geometry/Tile.js';
import { Game } from '../../api/game/Game.js';
import { SettingsBag } from '../../runtime/Settings.js';

const FALLBACK = new Tile(3000, 3001, 3);

export default class T extends LoopingBot {
    loop() {
        globalThis.__rs2b0t_host.snapshot = {
            here: { x: 3209, z: 3214, level: 2 },
        };
        const tile = this.settings.tile('startTile', FALLBACK);
        const translated = tile.translate(2, -1);
        const explicit = new SettingsBag({
            startTile: { x: 3210, z: 3210, level: 1 },
        }).tile('startTile', FALLBACK);
        globalThis.__probe = {
            tile,
            tileIdentity: tile instanceof Tile,
            distanceToGameTile: tile.distanceTo(Game.tile()),
            crossPlaneDistance: tile.distanceTo(new Tile(3211, 3215, 1)),
            directNativeDistance: globalThis.__rs2b0t_distance(
                { x: 3208, z: 3212, level: 2 },
                { x: 3209, z: 3214, level: 2 },
            ),
            invalidDistanceRejected: (() => {
                try {
                    tile.distanceTo({ x: 'nope', z: 3212, level: 2 });
                    return false;
                } catch (error) {
                    return String(error).includes('invalid tile distance');
                }
            })(),
            hugeSafeIntegerDistance: tile.distanceTo({
                x: Number.MAX_SAFE_INTEGER,
                z: 3212,
                level: 2,
            }),
            fractionalDistance: tile.distanceTo({ x: 3208.5, z: 3212, level: 2 }),

            translated,
            translatedIdentity: translated instanceof Tile,
            translatedEquals: translated.equals(new Tile(3210, 3211, 2)),
            missingUsesFallback: this.settings.tile('missingTile', FALLBACK) === FALLBACK,
            invalidUsesFallback: this.settings.tile('invalidTile', FALLBACK) === FALLBACK,
            outOfWorldUsesFallback:
                this.settings.tile('outOfWorldTile', FALLBACK) === FALLBACK,
            fractionalUsesFallback:
                this.settings.tile('fractionalTile', FALLBACK) === FALLBACK,
            explicitIdentity: explicit instanceof Tile,
            explicitDistance: explicit.distanceTo(new Tile(3212, 3211, 1)),
            list: this.settings.list('targets'),
            leash: this.settings.num('leashRadius', 0),
            bury: this.settings.bool('buryBones', false),
            style: this.settings.str('combatStyle', ''),
        };
    }
}
"#;

#[test]
fn tile_and_list_schema_defaults_round_trip_through_prelude() {
    let schema = vec![
        SettingDef {
            id: "startTile".into(),
            ty: "tile".into(),
            default: Some("3200,3200,0".into()),
            label: None,
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        },
        SettingDef {
            id: "targets".into(),
            ty: "list".into(),
            default: Some("bones,shells".into()),
            label: None,
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        },
    ];
    let mut bag = script::merge_bag(&schema, &serde_json::Map::new(), None);
    bag.insert(
        "startTile".into(),
        serde_json::json!({ "x": 3208, "z": 3212, "level": 2 }),
    );
    bag.insert(
        "invalidTile".into(),
        serde_json::json!({ "x": 3208, "z": "bad", "level": 2 }),
    );
    bag.insert(
        "outOfWorldTile".into(),
        serde_json::json!({ "x": 16_384, "z": 3212, "level": 2 }),
    );
    bag.insert(
        "fractionalTile".into(),
        serde_json::json!({ "x": 3208.5, "z": 3212, "level": 2 }),
    );
    bag.insert("leashRadius".into(), serde_json::json!(8));
    bag.insert("buryBones".into(), serde_json::json!(true));
    bag.insert("combatStyle".into(), serde_json::json!("mage"));
    let iso = LoadIsolate::spawn(TILE_LIST_PROBE.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn tile/list probe");
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("tile/list probe readable");
    assert_eq!(value["tile"]["x"], 3208, "settings.tile must not fall back");
    assert_eq!(value["tile"]["z"], 3212);
    assert_eq!(value["tile"]["level"], 2);
    assert_eq!(
        value["tileIdentity"], true,
        "posted tile uses imported Tile"
    );
    assert_eq!(value["distanceToGameTile"], 2);
    assert_eq!(value["crossPlaneDistance"], 1_000_003);
    assert_eq!(
        value["directNativeDistance"], 2,
        "the synchronous Rust geometry callback must be registered"
    );
    assert_eq!(
        value["invalidDistanceRejected"], true,
        "non-numeric coordinates must fail"
    );
    assert_eq!(
        value["hugeSafeIntegerDistance"].as_f64(),
        Some(9_007_199_254_737_783.0),
        "MAX_SAFE_INTEGER is a frozen-computable number"
    );

    assert_eq!(value["fractionalDistance"], 0.5);

    assert_eq!(
        value["translated"],
        serde_json::json!({ "x": 3210, "z": 3211, "level": 2 })
    );
    assert_eq!(value["translatedIdentity"], true);
    assert_eq!(value["translatedEquals"], true);
    assert_eq!(value["missingUsesFallback"], true);
    assert_eq!(value["invalidUsesFallback"], true);
    assert_eq!(value["outOfWorldUsesFallback"], true);
    assert_eq!(value["fractionalUsesFallback"], true);
    assert_eq!(value["explicitIdentity"], true);
    assert_eq!(value["explicitDistance"], 2);
    assert_eq!(
        value["list"],
        serde_json::json!(["bones", "shells"]),
        "settings.list must not fall back"
    );
    assert_eq!(value["leash"], 8);
    assert_eq!(value["bury"], true);
    assert_eq!(value["style"], "mage");
    iso.join();
}

#[test]
fn merge_bag_coerces_string_array_default_to_json_array() {
    let schema = vec![SettingDef {
        id: "items".into(),
        ty: "string[]".into(),
        default: Some("Maple longbow,Yew longbow".into()),
        label: Some("Items to alch".into()),
        min: None,
        max: None,
        step: None,
        options: vec![
            "Maple longbow".into(),
            "Yew longbow".into(),
            "Custom".into(),
        ],
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }];
    let bag = script::merge_bag(&schema, &serde_json::Map::new(), None);
    assert_eq!(
        bag.get("items"),
        Some(&serde_json::json!(["Maple longbow", "Yew longbow"])),
        "string[] defaults must post as arrays, not a free-text string"
    );
}

#[test]
fn merged_bag_from_spread_schema_posts_inject_over_defaults() {
    let src = r#"
export const SETTINGS = {
    target: { type: 'string', default: 'Man', label: 'Target' },
    ...PERIODIC_BANK_SETTINGS,
};
"#;
    let schema = script::settings_schema_from_source(src);
    assert!(
        schema.iter().any(|s| s.id == "bankStrategy"),
        "schema must include spread banking ids for Script prefs"
    );
    let mut inject = serde_json::Map::new();
    inject.insert("target".into(), serde_json::json!("Guard"));
    inject.insert("banking".into(), serde_json::json!("None"));
    let bag = script::merge_bag(&schema, &serde_json::Map::new(), Some(&inject));
    assert_eq!(bag.get("target"), Some(&serde_json::json!("Guard")));
    assert_eq!(
        bag.get("bankStrategy"),
        Some(&serde_json::json!("Off")),
        "spread defaults land even when inject omits them"
    );

    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = this.settings.str('target', 'Man');
    }
}
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
    )
    .expect("spawn");
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__probe").unwrap(),
        "Guard",
        "Start bag must beat SETTINGS default Man"
    );
    iso.join();
}
