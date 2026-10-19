use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use frontend_core::scripts::{CardSource, Loadouts};
use frontend_core::test_support::session::{
    vault_assignment, vault_debug_rows, vault_script_bag, vault_touch, SettingsFixture,
};
use frontend_core::test_support::VaultFixture;
use frontend_core::{MapBakeChoice, MapBakeGate, MapBakePrompt};
use host_play::local_state::{LocalStateService, StateRoot};
use host_play::map_cache::fixture::{
    fixture_descriptor, ship_fixture_terrain, CountingMapProducer,
};
use host_play::map_cache::{MapCacheRoot, MapDemand, MapDemandManager, MapJobStatus};
use panel::ui_state::{load_from, save_to};
use serde_json::json;

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
    let mut opened = VaultFixture::unlock(&vault_path, "bot");
    let auto = opened
        .profile("fixture-auto")
        .expect("fixture-auto is stored");
    assert_eq!(auto.draft().password_for_test(), "bot");
    assert_eq!(auto.uid(), 289_018_101);
    assert!(!auto.lowmem());
    assert!(auto.auto_login());
    assert_eq!(auto.draft().settings().world(), Some(2));
    assert_eq!(auto.tutorial_skipped(), Some(true));
    assert_eq!(auto.raster(), frontend_core::command::RasterMode::Cpu);
    assert!(!auto.random_events());
    assert_eq!(auto.draft().settings().lamp_skill(), "magic");
    assert!(!auto.draft().settings().lamp_auto());
    let assignment = vault_assignment(&opened, "fixture-auto").expect("auto assignment");
    assert_eq!(assignment.source_kind(), "catalog");
    assert_eq!(assignment.identity(), "ChickenKiller");
    assert_eq!(assignment.display_name(), "Chicken Killer");
    assert_eq!(assignment.unavailable(), None);
    let bag = vault_script_bag(&opened, "fixture-auto", "catalog:ChickenKiller")
        .expect("auto ChickenKiller bag");
    assert_eq!(bag["buryBones"], json!(false));
    assert_eq!(bag["eatAtPercent"], json!(47));
    assert_eq!(bag["food"], json!("Lobster"));
    assert_eq!(bag["loadout"], json!("melee-main"));
    assert_eq!(bag["patrol"], json!(["Lumbridge", "Falador"]));
    assert_eq!(bag["homeTile"], json!({"x": 3232, "z": 3298, "level": 0}));

    let manual = opened
        .profile("fixture-manual")
        .expect("fixture-manual is stored");
    assert_eq!(manual.draft().password_for_test(), "bot");
    assert_eq!(manual.uid(), 289_018_102);
    assert!(manual.lowmem());
    assert!(!manual.auto_login());
    assert_eq!(manual.draft().settings().world(), Some(1));
    assert_eq!(manual.tutorial_skipped(), Some(false));
    assert_eq!(manual.raster(), frontend_core::command::RasterMode::Off);
    assert!(manual.random_events());
    assert_eq!(manual.draft().settings().lamp_skill(), "prayer");
    assert!(manual.draft().settings().lamp_auto());
    let assignment = vault_assignment(&opened, "fixture-manual").expect("manual assignment");
    assert_eq!(assignment.source_kind(), "catalog");
    assert_eq!(assignment.identity(), "Thiever");
    assert_eq!(assignment.display_name(), "Thiever");
    assert_eq!(
        assignment.unavailable(),
        Some("fixture catalog intentionally not bundled")
    );
    let thiever_bag =
        vault_script_bag(&opened, "fixture-manual", "catalog:Thiever").expect("manual Thiever bag");
    assert_eq!(thiever_bag["food"], json!("Cake"));
    assert_eq!(thiever_bag["bankAt"], json!(9));

    let defaults = opened
        .profile("fixture-defaults")
        .expect("fixture-defaults is stored");
    assert_eq!(defaults.draft().password_for_test(), "bot");
    assert_eq!(defaults.uid(), 289_018_103);
    assert!(defaults.lowmem());
    assert!(!defaults.auto_login());
    assert_eq!(defaults.draft().settings().world(), None);
    assert_eq!(defaults.tutorial_skipped(), None);
    assert_eq!(defaults.raster(), frontend_core::command::RasterMode::Gpu);
    assert!(defaults.random_events());
    assert_eq!(defaults.draft().settings().lamp_skill(), "strength");
    assert!(defaults.draft().settings().lamp_auto());
    assert!(vault_assignment(&opened, "fixture-defaults").is_none());
    assert!(vault_script_bag(&opened, "fixture-defaults", "catalog:Thiever").is_none());

    let expected_profiles = vault_debug_rows(&opened);
    vault_touch(&mut opened, "fixture-auto");
    drop(opened);
    let reopened = VaultFixture::unlock(&vault_path, "bot");
    assert_eq!(
        vault_debug_rows(&reopened),
        expected_profiles,
        "a current rewrite must preserve every 0.1.8.1 profile field"
    );

    copy_fixture_file(&fixture, &scratch, "panel-ui.json");
    copy_fixture_file(&fixture, &scratch, "loadouts.json");
    // The scratch directory is the state root the current readers open: the
    // preferences and the loadouts form go through one process service.
    let service = LocalStateService::open_process(StateRoot::new(scratch.0.clone()));
    let preferences = frontend_core::UiPreferences::of_service_for_test(&service);
    let prefs = load_from(&preferences);
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
    assert_eq!(prefs.raster, frontend_core::command::RasterMode::Cpu);
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
    assert!(
        !prefs.session_log_file,
        "session log file defaults off for a 0.1.8.1 home"
    );
    assert_eq!(prefs.chrome.accent, "#22CC88");
    assert_eq!(prefs.chrome.bg, "#101820");
    assert_eq!(prefs.chrome.text, "#F0F4F8");
    let expected_prefs = prefs.clone();
    save_to(&preferences, &prefs);
    let reopened = LocalStateService::open_process(StateRoot::new(scratch.0.clone()));
    assert_eq!(
        load_from(&frontend_core::UiPreferences::of_service_for_test(
            &reopened
        )),
        expected_prefs
    );

    let mut loadouts = Loadouts::operator(&service);
    assert_eq!(loadouts.load_error(), None);
    let expected_loadouts = loadouts.loadouts().to_vec();
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
    // Save with the current writer: change one supply, then change it back,
    // so the file is rewritten from the typed rows twice.
    let mut edited = expected_loadouts[0].clone();
    edited.carry[0].qty += 1;
    loadouts.upsert(edited);
    loadouts.save().unwrap();
    loadouts.upsert(expected_loadouts[0].clone());
    loadouts.save().unwrap();
    let reopened = LocalStateService::open_process(StateRoot::new(scratch.0.clone()));
    assert_eq!(
        Loadouts::operator(&reopened).loadouts(),
        expected_loadouts.as_slice()
    );

    let settings_path = copy_fixture_file(&fixture, &scratch, "script-settings.json");
    let mut settings = SettingsFixture::at(settings_path.clone());
    let overrides = settings.overrides(CardSource::Catalog, "ChickenKiller");
    assert_eq!(overrides["buryBones"], json!(false));
    assert_eq!(overrides["eatAtPercent"], json!(47.0));
    assert_eq!(overrides["loadout"], json!("melee-main"));
    settings.set_value(
        CardSource::Catalog,
        "ChickenKiller",
        "buryBones",
        overrides["buryBones"].clone(),
    );
    settings.save().unwrap();
    assert_eq!(
        SettingsFixture::at(settings_path).overrides(CardSource::Catalog, "ChickenKiller"),
        overrides
    );
}

/// A 0.1.8.1 home has no map cache and no `map_bake` preference. With the
/// release's shipped terrain for the bound client cache, the first WalkTo
/// open installs it into a new `map-cache` without a bake question or a bake,
/// and every 0.1.8.1 file keeps its bytes.
#[test]
fn a_0181_home_opens_walkto_on_shipped_terrain_without_asking_and_keeps_its_files() {
    let fixture = fixture_bot_dir();
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let bot = home.join(".274bot");
    std::fs::create_dir_all(&bot).unwrap();
    let names = [
        "vault-289",
        "panel-ui.json",
        "loadouts.json",
        "script-settings.json",
    ];
    for name in names {
        std::fs::copy(fixture.join(name), bot.join(name)).unwrap();
    }
    let before: Vec<Vec<u8>> = names
        .iter()
        .map(|name| std::fs::read(bot.join(name)).unwrap())
        .collect();

    // The panel's gate starts from the 0.1.8.1 prefs, which never answered.
    let service = LocalStateService::open_process(StateRoot::new(bot.clone()));
    let prefs = load_from(&frontend_core::UiPreferences::of_service_for_test(&service));
    assert_eq!(prefs.map_bake, MapBakeChoice::Ask);
    let root = MapCacheRoot::from_home(&home);
    assert!(!root.path().exists(), "0.1.8.1 wrote no map cache");
    let shipped = ship_fixture_terrain(&scratch.0.join("package"));
    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::with_shipped_map(root.clone(), producer.clone(), shipped);
    let mut gate = MapBakeGate::new(prefs.map_bake);
    let handle = gate
        .open_for_test(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    assert_eq!(gate.prompt(), MapBakePrompt::None, "no bake question");
    let started = Instant::now();
    while handle.status() != MapJobStatus::Ready {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "WalkTo never settled: {:?}",
            handle.status()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(handle.ready().unwrap().images.is_some(), "terrain is shown");
    assert_eq!(producer.image_runs(), 0, "installed, not baked");
    assert!(root
        .image_dir(fixture_descriptor().image_identity())
        .unwrap()
        .is_dir());

    for (name, bytes) in names.iter().zip(&before) {
        assert_eq!(
            &std::fs::read(bot.join(name)).unwrap(),
            bytes,
            "{name} keeps its 0.1.8.1 bytes"
        );
    }
    let entries: BTreeSet<String> = std::fs::read_dir(&bot)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let mut expected: BTreeSet<String> = names.iter().map(|name| name.to_string()).collect();
    expected.insert("map-cache".into());
    assert_eq!(entries, expected, "only the map cache is new");
}
