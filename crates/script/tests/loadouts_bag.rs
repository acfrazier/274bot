// Task 6: settings.str('loadout') reads the posted operator bag.

use script::{LoadIsolate, LoadShape, SettingDef};

const LOADOUT_PROBE: &str = r#"
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = this.settings.str('loadout', '');
    }
}
"#;

#[test]
fn settings_str_returns_selected_loadout_name() {
    let iso = LoadIsolate::spawn(LOADOUT_PROBE.to_string(), LoadShape::CompatClass)
        .expect("spawn loadout probe");
    let mut bag = serde_json::Map::new();
    bag.insert("loadout".into(), serde_json::json!("melee"));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("loadout probe readable");
    assert_eq!(value, "melee", "str reads posted loadout name, not fallback");
    iso.join();
}

#[test]
fn loadout_combo_resolves_from_store_names_in_script_crate() {
    use script::{Loadout, LoadoutsStore, resolve_setting_options};

    let dir = std::env::temp_dir().join(format!(
        "274bot-loadouts-combo-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("loadouts.json");
    let mut store = LoadoutsStore::at(path);
    store.upsert(Loadout {
        name: "a".into(),
        worn: vec![],
        carry: vec![],
    });
    store.upsert(Loadout {
        name: "b".into(),
        worn: vec![],
        carry: vec![],
    });
    let def = SettingDef {
        id: "loadout".into(),
        ty: "string".into(),
        default: None,
        label: None,
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: Some("loadouts".into()),
        csv_toggle: None,
        help: None,
    };
    assert_eq!(
        resolve_setting_options(&def, &store),
        vec!["a".to_string(), "b".to_string()]
    );
}
