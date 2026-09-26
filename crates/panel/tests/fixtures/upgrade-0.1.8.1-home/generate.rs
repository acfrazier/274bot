//! Reproduction source for the committed 0.1.8.1 home fixture.
//! Copy this file to `crates/panel/examples/generate_upgrade_fixture.rs` in
//! the exact 0.1.8.1 source tree and run it with the fixture home path.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use panel::nav_settings::NavSettings;
use panel::theme::ChromeColors;
use panel::ui_state::{save_at, PanelUiState};
use script::{Loadout, LoadoutsStore, ScriptSettingsStore, ScriptSource};
use serde_json::{json, Map};
use vault::{Profile, ProfileSettings, RasterMode, ScriptAssignment, Vault};

fn main() {
    let home = PathBuf::from(std::env::args_os().nth(1).expect("fixture HOME argument"));
    let bot = home.join(".274bot");
    std::fs::create_dir_all(&bot).unwrap();

    let vault_path = bot.join("vault-289");
    let _ = std::fs::remove_file(&vault_path);
    let mut vault = Vault::create(&vault_path, "bot").unwrap();

    let mut chicken = Map::new();
    chicken.insert("buryBones".into(), json!(false));
    chicken.insert("eatAtPercent".into(), json!(47));
    chicken.insert("food".into(), json!("Lobster"));
    chicken.insert("loadout".into(), json!("melee-main"));
    chicken.insert("patrol".into(), json!(["Lumbridge", "Falador"]));
    chicken.insert("homeTile".into(), json!({"x": 3232, "z": 3298, "level": 0}));
    let mut chicken_bags = BTreeMap::new();
    chicken_bags.insert("catalog:ChickenKiller".into(), chicken);

    vault
        .upsert(Profile {
            username: "fixture-auto".into(),
            password: "bot".into(),
            uid: 289_018_101,
            settings: ProfileSettings {
                lowmem: false,
                auto_login: true,
                world: Some(2),
                tutorial_skipped: Some(true),
                raster: RasterMode::Cpu,
                random_events: false,
                lamp_skill: "magic".into(),
                lamp_auto: false,
                script_assignment: Some(ScriptAssignment {
                    source_kind: "catalog".into(),
                    identity: "ChickenKiller".into(),
                    display_name: "Chicken Killer".into(),
                    unavailable: None,
                }),
                script_settings: chicken_bags,
            },
        })
        .unwrap();

    let mut thiever = Map::new();
    thiever.insert("food".into(), json!("Cake"));
    thiever.insert("bankAt".into(), json!(9));
    let mut thiever_bags = BTreeMap::new();
    thiever_bags.insert("catalog:Thiever".into(), thiever);
    vault
        .upsert(Profile {
            username: "fixture-manual".into(),
            password: "bot".into(),
            uid: 289_018_102,
            settings: ProfileSettings {
                lowmem: true,
                auto_login: false,
                world: Some(1),
                tutorial_skipped: Some(false),
                raster: RasterMode::Off,
                random_events: true,
                lamp_skill: "prayer".into(),
                lamp_auto: true,
                script_assignment: Some(ScriptAssignment {
                    source_kind: "catalog".into(),
                    identity: "Thiever".into(),
                    display_name: "Thiever".into(),
                    unavailable: Some("fixture catalog intentionally not bundled".into()),
                }),
                script_settings: thiever_bags,
            },
        })
        .unwrap();

    vault
        .upsert(Profile {
            username: "fixture-defaults".into(),
            password: "bot".into(),
            uid: 289_018_103,
            settings: ProfileSettings::default(),
        })
        .unwrap();
    drop(vault);

    let mut loadouts = LoadoutsStore::at(bot.join("loadouts.json"));
    loadouts.upsert(
        Loadout::new("melee-main")
            .with_slot("hat", "Rune full helm")
            .with_slot("righthand", "Rune scimitar")
            .with_slot("body", "Rune platebody")
            .with_carry("Lobster", 12)
            .with_carry("Strength potion(4)", 2),
    );
    loadouts.upsert(
        Loadout::new("utility")
            .with_slot("back", "Cape")
            .with_slot("legacy-mystery-slot", "Ghostspeak amulet")
            .with_carry("Coins", 5000),
    );
    loadouts.save().unwrap();

    let mut legacy_settings = ScriptSettingsStore::at(bot.join("script-settings.json"));
    legacy_settings.set_bool(ScriptSource::Catalog, "ChickenKiller", "buryBones", false);
    legacy_settings.set_num(ScriptSource::Catalog, "ChickenKiller", "eatAtPercent", 47.0);
    legacy_settings.set_str(
        ScriptSource::Catalog,
        "ChickenKiller",
        "loadout",
        "melee-main",
    );
    legacy_settings.save().unwrap();

    let mut chrome = ChromeColors::default();
    chrome.accent = "#22CC88".into();
    chrome.bg = "#101820".into();
    chrome.text = "#F0F4F8".into();
    let state = PanelUiState {
        last_focus: Some("fixture-manual".into()),
        collapsed: HashMap::from([
            (
                "fixture-auto".into(),
                HashMap::from([("script".into(), false), ("debug".into(), true)]),
            ),
            (
                "fixture-manual".into(),
                HashMap::from([("profile".into(), true)]),
            ),
        ]),
        nav: NavSettings {
            allow_teleports: true,
            allow_wilderness: true,
            allow_bank_fetch: true,
            show_nav_path: true,
            hop_labels: false,
            hop_label_px: 19,
            color_path: "#123456".into(),
            color_transport: "#654321".into(),
            color_click: "#ABCDEF".into(),
            color_text: "#FEDCBA".into(),
            collision_fill: true,
            nsew_labels: true,
            client_trail: true,
            color_collision: "#112233".into(),
            color_client: "#445566".into(),
            color_client_run_alt: "#778899".into(),
            component_flood: true,
            camera_follow: true,
        },
        rail_preview: HashMap::from([
            ("fixture-auto".into(), true),
            ("fixture-manual".into(), false),
        ]),
        raster: RasterMode::Cpu,
        lowmem: false,
        server_revision: 289,
        section_order: vec![
            "profile".into(),
            "status".into(),
            "script".into(),
            "debug".into(),
            "log".into(),
            "parameters".into(),
        ],
        script_category_order: vec!["Combat".into(), "Skilling".into(), "Utility".into()],
        script_load_last_dir: Some(PathBuf::from("fixture-scripts")),
        script_catalog_last_dir: Some(PathBuf::from("fixture-catalog")),
        show_parameters_rail: true,
        capture: false,
        panel_sections: HashMap::from([("debug".into(), false), ("log".into(), true)]),
        config_collapsed: HashMap::from([("rendering".into(), true), ("navigation".into(), false)]),
        chrome,
    };
    save_at(&bot.join("panel-ui.json"), &state);
}
