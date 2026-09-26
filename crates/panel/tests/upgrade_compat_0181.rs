use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use panel::ui_state::{load_at, save_at};
use script::{LoadoutsStore, ScriptSettingsStore, ScriptSource};
use serde_json::json;
use vault::{RasterMode, Vault};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-upgrade-0181-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_bot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upgrade-0.1.8.1-home/home/.274bot")
}

fn copy_fixture_file(bot: &Path, scratch: &Scratch, name: &str) -> PathBuf {
    let destination = scratch.0.join(name);
    std::fs::copy(bot.join(name), &destination).unwrap();
    destination
}

#[test]
fn current_reader_opens_real_0181_home_and_preserves_every_meaning_on_save() {
    let fixture = fixture_bot_dir();
    let scratch = Scratch::new();

    let original_vault = std::fs::read(fixture.join("vault-289")).unwrap();
    assert_eq!(&original_vault[..8], b"274VAULT");
    assert_eq!(original_vault[8], 1, "0.1.8.1 vault format version");
    assert_eq!(
        u32::from_le_bytes(original_vault[9..13].try_into().unwrap()),
        100_000,
        "0.1.8.1 PBKDF2-HMAC-SHA256 round count"
    );
    assert!(
        !original_vault
            .windows("fixture-auto".len())
            .any(|window| window == b"fixture-auto"),
        "profile identity must remain encrypted"
    );
    assert!(
        !original_vault.windows(3).any(|window| window == b"bot"),
        "throwaway password must not appear in plaintext"
    );

    let vault_path = copy_fixture_file(&fixture, &scratch, "vault-289");
    let mut opened = Vault::unlock(&vault_path, "bot").unwrap();
    let auto = opened.get("fixture-auto").unwrap().clone();
    assert_eq!(auto.password, "bot");
    assert_eq!(auto.uid, 289_018_101);
    assert!(!auto.settings.lowmem);
    assert!(auto.settings.auto_login);
    assert_eq!(auto.settings.world, Some(2));
    assert_eq!(auto.settings.tutorial_skipped, Some(true));
    assert_eq!(auto.settings.raster, RasterMode::Cpu);
    assert!(!auto.settings.random_events);
    assert_eq!(auto.settings.lamp_skill, "magic");
    assert!(!auto.settings.lamp_auto);
    let assignment = auto.settings.script_assignment.as_ref().unwrap();
    assert_eq!(assignment.source_kind, "catalog");
    assert_eq!(assignment.identity, "ChickenKiller");
    assert_eq!(assignment.display_name, "Chicken Killer");
    assert_eq!(assignment.unavailable, None);
    let bag = &auto.settings.script_settings["catalog:ChickenKiller"];
    assert_eq!(bag["buryBones"], json!(false));
    assert_eq!(bag["eatAtPercent"], json!(47));
    assert_eq!(bag["food"], json!("Lobster"));
    assert_eq!(bag["loadout"], json!("melee-main"));
    assert_eq!(bag["patrol"], json!(["Lumbridge", "Falador"]));
    assert_eq!(bag["homeTile"], json!({"x": 3232, "z": 3298, "level": 0}));

    let manual = opened.get("fixture-manual").unwrap().clone();
    assert_eq!(manual.password, "bot");
    assert_eq!(manual.uid, 289_018_102);
    assert!(manual.settings.lowmem);
    assert!(!manual.settings.auto_login);
    assert_eq!(manual.settings.world, Some(1));
    assert_eq!(manual.settings.tutorial_skipped, Some(false));
    assert_eq!(manual.settings.raster, RasterMode::Off);
    assert!(manual.settings.random_events);
    assert_eq!(manual.settings.lamp_skill, "prayer");
    assert!(manual.settings.lamp_auto);
    let assignment = manual.settings.script_assignment.as_ref().unwrap();
    assert_eq!(assignment.source_kind, "catalog");
    assert_eq!(assignment.identity, "Thiever");
    assert_eq!(assignment.display_name, "Thiever");
    assert_eq!(
        assignment.unavailable.as_deref(),
        Some("fixture catalog intentionally not bundled")
    );
    assert_eq!(
        manual.settings.script_settings["catalog:Thiever"]["food"],
        json!("Cake")
    );
    assert_eq!(
        manual.settings.script_settings["catalog:Thiever"]["bankAt"],
        json!(9)
    );

    let defaults = opened.get("fixture-defaults").unwrap().clone();
    assert_eq!(defaults.password, "bot");
    assert_eq!(defaults.uid, 289_018_103);
    assert!(defaults.settings.lowmem);
    assert!(!defaults.settings.auto_login);
    assert_eq!(defaults.settings.world, None);
    assert_eq!(defaults.settings.tutorial_skipped, None);
    assert_eq!(defaults.settings.raster, RasterMode::Gpu);
    assert!(defaults.settings.random_events);
    assert_eq!(defaults.settings.lamp_skill, "strength");
    assert!(defaults.settings.lamp_auto);
    assert!(defaults.settings.script_assignment.is_none());
    assert!(defaults.settings.script_settings.is_empty());

    let expected_profiles: Vec<_> = opened.profiles().cloned().collect();
    opened.upsert(auto).unwrap();
    drop(opened);
    let reopened = Vault::unlock(&vault_path, "bot").unwrap();
    assert_eq!(
        reopened.profiles().cloned().collect::<Vec<_>>(),
        expected_profiles,
        "a current rewrite must preserve every 0.1.8.1 profile field"
    );

    let prefs_path = copy_fixture_file(&fixture, &scratch, "panel-ui.json");
    let prefs = load_at(&prefs_path);
    assert_eq!(prefs.last_focus.as_deref(), Some("fixture-manual"));
    assert!(!prefs.collapsed["fixture-auto"]["script"]);
    assert!(prefs.collapsed["fixture-auto"]["debug"]);
    assert!(prefs.collapsed["fixture-manual"]["profile"]);
    assert!(prefs.nav.allow_teleports);
    assert!(prefs.nav.allow_wilderness);
    assert!(prefs.nav.allow_bank_fetch);
    assert!(prefs.nav.show_nav_path);
    assert!(!prefs.nav.hop_labels);
    assert_eq!(prefs.nav.hop_label_px, 19);
    assert_eq!(prefs.nav.color_path, "#123456");
    assert_eq!(prefs.nav.color_transport, "#654321");
    assert_eq!(prefs.nav.color_click, "#ABCDEF");
    assert_eq!(prefs.nav.color_text, "#FEDCBA");
    assert!(prefs.nav.collision_fill);
    assert!(prefs.nav.nsew_labels);
    assert!(prefs.nav.client_trail);
    assert_eq!(prefs.nav.color_collision, "#112233");
    assert_eq!(prefs.nav.color_client, "#445566");
    assert_eq!(prefs.nav.color_client_run_alt, "#778899");
    assert!(prefs.nav.component_flood);
    assert!(prefs.nav.camera_follow);
    assert!(prefs.rail_preview["fixture-auto"]);
    assert!(!prefs.rail_preview["fixture-manual"]);
    assert_eq!(prefs.raster, RasterMode::Cpu);
    assert!(!prefs.lowmem);
    assert_eq!(prefs.server_revision, 289);
    assert_eq!(
        prefs.section_order,
        ["profile", "status", "script", "debug", "log", "parameters"]
    );
    assert_eq!(
        prefs.script_category_order,
        ["Combat", "Skilling", "Utility"]
    );
    assert_eq!(
        prefs.script_load_last_dir.as_deref(),
        Some(Path::new("fixture-scripts"))
    );
    assert_eq!(
        prefs.script_catalog_last_dir.as_deref(),
        Some(Path::new("fixture-catalog"))
    );
    assert!(prefs.show_parameters_rail);
    assert!(!prefs.capture);
    assert!(!prefs.panel_sections["debug"]);
    assert!(prefs.panel_sections["log"]);
    assert!(prefs.config_collapsed["rendering"]);
    assert!(!prefs.config_collapsed["navigation"]);
    assert!(
        !prefs.background_bots_ack,
        "new preference defaults without resetting old prefs"
    );
    assert_eq!(prefs.chrome.accent, "#22CC88");
    assert_eq!(prefs.chrome.bg, "#101820");
    assert_eq!(prefs.chrome.text, "#F0F4F8");
    let expected_prefs = prefs.clone();
    save_at(&prefs_path, &prefs);
    assert_eq!(load_at(&prefs_path), expected_prefs);

    let loadouts_path = copy_fixture_file(&fixture, &scratch, "loadouts.json");
    let mut loadouts = LoadoutsStore::at(loadouts_path.clone());
    let expected_loadouts = loadouts.snapshot();
    assert_eq!(loadouts.names(), ["melee-main", "utility"]);
    let melee = loadouts.get("melee-main").unwrap();
    assert_eq!(melee.worn["hat"], "Rune full helm");
    assert_eq!(melee.worn["righthand"], "Rune scimitar");
    assert_eq!(melee.unassigned, ["Rune platebody"]);
    assert_eq!(melee.carry[0].item, "Lobster");
    assert_eq!(melee.carry[0].qty, 12);
    assert_eq!(melee.carry[1].item, "Strength potion(4)");
    assert_eq!(melee.carry[1].qty, 2);
    let utility = loadouts.get("utility").unwrap();
    assert_eq!(utility.worn["back"], "Cape");
    assert_eq!(utility.unassigned, ["Ghostspeak amulet"]);
    assert_eq!(utility.carry[0].item, "Coins");
    assert_eq!(utility.carry[0].qty, 5000);
    loadouts.upsert(expected_loadouts[0].clone());
    loadouts.save().unwrap();
    assert_eq!(
        LoadoutsStore::at(loadouts_path).snapshot(),
        expected_loadouts
    );

    let settings_path = copy_fixture_file(&fixture, &scratch, "script-settings.json");
    let mut settings = ScriptSettingsStore::at(settings_path.clone());
    let overrides = settings.overrides(ScriptSource::Catalog, "ChickenKiller");
    assert_eq!(overrides["buryBones"], json!(false));
    assert_eq!(overrides["eatAtPercent"], json!(47.0));
    assert_eq!(overrides["loadout"], json!("melee-main"));
    settings.set_value(
        ScriptSource::Catalog,
        "ChickenKiller",
        "buryBones",
        overrides["buryBones"].clone(),
    );
    settings.save().unwrap();
    assert_eq!(
        ScriptSettingsStore::at(settings_path).overrides(ScriptSource::Catalog, "ChickenKiller"),
        overrides
    );
}
