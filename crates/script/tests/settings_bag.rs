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
    let iso = LoadIsolate::spawn(SETTINGS_PROBE.to_string(), LoadShape::CompatClass)
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
    let bag = store.merged_bag(
        ScriptSource::Catalog,
        "ChickenKiller",
        &schema,
        None,
    );
    assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(false)));
    assert_eq!(
        bag.get("leashRadius").and_then(|v| v.as_f64()),
        Some(20.0)
    );
    assert_eq!(
        store.card_key(ScriptSource::File, "BoneBurier"),
        "file:BoneBurier"
    );
}
