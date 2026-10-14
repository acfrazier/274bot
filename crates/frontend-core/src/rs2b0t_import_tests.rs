//! Parser and mapping proofs against a fixture written by rs2b0t's own
//! `serializeProfileFile` (rs2b0t 2de6bb23; generator and SHA-256 in the
//! RS2B0T-IMPORT evidence). Generic names and fake passwords only.

use std::path::PathBuf;

use host_play::{InstancePermit, Play, PlayOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings, RasterMode, ScriptAssignment, Vault};

use super::*;

const FIXTURE: &str = include_str!("../tests/fixtures/rs2b0t-profiles.json");
const PASSWORDS: [&str; 2] = ["fake-pass-alice", "fake-pass-bob"];

fn def(
    id: &str,
    ty: &str,
    options: &[&str],
    min: Option<&str>,
    max: Option<&str>,
) -> script::SettingDef {
    script::SettingDef {
        id: id.into(),
        ty: ty.into(),
        default: None,
        label: None,
        min: min.map(Into::into),
        max: max.map(Into::into),
        step: None,
        options: options.iter().map(|o| o.to_string()).collect(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }
}

/// A catalog with rs2b0t's CowKiller and Woodcutter parameter shapes (a
/// subset), optionally dimming one card.
struct FakeCatalog {
    loaded: bool,
    cards: Vec<(String, Vec<script::SettingDef>, Option<String>)>,
}

impl FakeCatalog {
    fn new() -> Self {
        Self {
            loaded: true,
            cards: vec![
                (
                    "CowKiller".into(),
                    vec![
                        def(
                            "location",
                            "string",
                            &["Auto", "Lumbridge cow field"],
                            None,
                            None,
                        ),
                        def("leashRadius", "number", &[], Some("3"), Some("30")),
                        def(
                            "combatStyle",
                            "string",
                            &["melee", "mage", "range"],
                            None,
                            None,
                        ),
                        def("buryBones", "boolean", &[], None, None),
                    ],
                    None,
                ),
                (
                    "Woodcutter".into(),
                    vec![
                        def("treeName", "string", &["Tree", "Oak", "Willow"], None, None),
                        def("leashRadius", "number", &[], Some("1"), Some("30")),
                    ],
                    None,
                ),
            ],
        }
    }

    fn dim(mut self, name: &str) -> Self {
        for card in &mut self.cards {
            if card.0 == name {
                card.2 = Some("needs an unsupported import".into());
            }
        }
        self
    }
}

impl ImportCatalog for FakeCatalog {
    fn loaded(&self) -> bool {
        self.loaded
    }

    fn card(&self, name: &str) -> Option<CatalogCard<'_>> {
        self.cards
            .iter()
            .find(|card| card.0 == name)
            .map(|(name, schema, dim)| CatalogCard {
                schema,
                unloadable: dim.as_deref(),
                assignment: ScriptAssignment {
                    source_kind: "catalog".into(),
                    identity: name.clone(),
                    display_name: name.clone(),
                    unavailable: None,
                },
            })
    }
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-rs2b0t-import-{}-{test}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn vault_with(test: &str, profiles: &[(&str, i32)]) -> Vault {
    let path = temp_dir(test).join("vault");
    let mut vault = Vault::create(&path, "test-passphrase-01").unwrap();
    for (name, uid) in profiles {
        vault
            .upsert(Profile {
                username: name.to_string(),
                password: "old-pass".into(),
                uid: *uid,
                settings: ProfileSettings {
                    lowmem: false,
                    ..ProfileSettings::default()
                },
            })
            .unwrap();
    }
    vault
}

fn empty_play() -> Play {
    host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
}

fn session(vault: Vault) -> OperatorSession<u32> {
    let mut session = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.start(vault, empty_play());
    session
}

fn account<'a>(plan: &'a ImportPlan, name: &str) -> &'a AccountPlan {
    plan.accounts()
        .iter()
        .find(|a| a.username() == name)
        .unwrap()
}

fn skipped_items(account: &AccountPlan) -> Vec<&str> {
    account.skipped().iter().map(|s| s.item.as_str()).collect()
}

fn assert_no_password(text: &str) {
    for password in PASSWORDS {
        assert!(!text.contains(password), "password leaked in: {text}");
    }
}

// ---- parser -----------------------------------------------------------------

#[test]
fn parses_the_fixture_rs2b0t_wrote() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let names: Vec<&str> = file
        .accounts()
        .iter()
        .map(ExportAccount::username)
        .collect();
    assert_eq!(names, ["alice", "bob"]);
    assert_eq!(file.accounts()[0].tab(), Some("Combat"));
    assert_eq!(file.accounts()[0].world(), Some(2));
    assert_eq!(file.accounts()[1].tab(), None);
    assert_eq!(file.accounts()[1].world(), None);
    assert_eq!(file.tabs(), ["Combat"]);
    assert_eq!(file.active_tab(), "Main");
    assert_eq!(
        file.storage().keys().collect::<Vec<_>>(),
        ["alice", "bob", "carol"]
    );
    // The per-tab login copy is never kept.
    assert_eq!(file.storage()["alice"]["creds"], "");
    assert_no_password(&format!("{file:?}"));
}

fn with(edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> String {
    let mut value: Value = serde_json::from_str(FIXTURE).unwrap();
    edit(value.as_object_mut().unwrap());
    value.to_string()
}

#[test]
fn refuses_another_kind_or_version_with_a_clear_message() {
    let error = parse_profile_file(&with(|o| {
        o.insert("kind".into(), json!("something-else"));
    }))
    .unwrap_err();
    assert!(matches!(error, ImportError::Kind { .. }));
    let text = error.to_string();
    assert!(text.contains("\"something-else\""), "{text}");
    assert!(text.contains(PROFILE_FILE_KIND), "{text}");
    assert!(text.contains("not an rs2b0t profile export"), "{text}");

    let error = parse_profile_file(&with(|o| {
        o.remove("kind");
    }))
    .unwrap_err();
    assert!(error.to_string().contains("kind is missing"), "{error}");

    let error = parse_profile_file(&with(|o| {
        o.insert("v".into(), json!(2));
    }))
    .unwrap_err();
    assert!(matches!(error, ImportError::Version { .. }));
    assert!(
        error.to_string().contains("version is 2, expected 1"),
        "{error}"
    );

    let error = parse_profile_file(&with(|o| {
        o.insert("v".into(), json!("1"));
    }))
    .unwrap_err();
    assert!(matches!(error, ImportError::Version { .. }), "{error}");
}

#[test]
fn refuses_malformed_shapes_without_quoting_a_password() {
    assert!(matches!(
        parse_profile_file("not json"),
        Err(ImportError::NotJson)
    ));
    assert!(matches!(parse_profile_file("[1]"), Err(ImportError::Shape)));
    for missing in ["profiles", "tabs", "activeTab"] {
        let error = parse_profile_file(&with(|o| {
            o.remove(missing);
        }))
        .unwrap_err();
        assert!(
            matches!(error, ImportError::MissingLists),
            "{missing}: {error}"
        );
    }
    let error = parse_profile_file(&with(|o| {
        o.insert("tabs".into(), json!(["ok", 3]));
    }))
    .unwrap_err();
    assert!(matches!(error, ImportError::TabsNotStrings));

    let entry_cases = [
        json!("alice"),
        json!({ "username": "", "password": "fake-pass-alice" }),
        json!({ "username": "alice" }),
        json!({ "username": "alice", "password": 7 }),
    ];
    for entry in entry_cases {
        let error = parse_profile_file(&with(|o| {
            o.insert("profiles".into(), json!([entry]));
        }))
        .unwrap_err();
        assert!(
            matches!(error, ImportError::Entry { index: 1, .. }),
            "{error}"
        );
        assert_no_password(&error.to_string());
    }
    let error = parse_profile_file(&with(|o| {
        o.insert(
            "profiles".into(),
            json!([{ "username": "alice", "password": "fake-pass-alice", "world": 4 }]),
        );
    }))
    .unwrap_err();
    assert!(matches!(error, ImportError::World { .. }), "{error}");
    assert_no_password(&error.to_string());
    let error = parse_profile_file(&with(|o| {
        o.insert(
            "profiles".into(),
            json!([{ "username": "alice", "password": "fake-pass-alice", "tab": 1 }]),
        );
    }))
    .unwrap_err();
    assert!(matches!(error, ImportError::Tab { .. }), "{error}");

    for (storage, want) in [
        (json!([]), "storage must be an object"),
        (json!({ "": {} }), "empty username"),
        (
            json!({ "alice": "x" }),
            "storage for 'alice' must be an object",
        ),
        (json!({ "alice": { "k": 1 } }), "'alice.k' must be a string"),
    ] {
        let error = parse_profile_file(&with(|o| {
            o.insert("storage".into(), storage);
        }))
        .unwrap_err();
        assert!(error.to_string().contains(want), "{error}");
    }
}

#[test]
fn storage_may_be_absent_and_main_tab_is_no_tab() {
    let file = parse_profile_file(&with(|o| {
        o.remove("storage");
        o.insert(
            "profiles".into(),
            json!([{ "username": "alice", "password": "fake-pass-alice", "tab": "Main" }]),
        );
    }))
    .unwrap();
    assert!(file.storage().is_empty());
    assert_eq!(file.accounts()[0].tab(), None);
}

#[test]
fn reads_the_file_from_disk() {
    let dir = temp_dir("read");
    let path = dir.join(PROFILE_FILE_NAME);
    std::fs::write(&path, FIXTURE).unwrap();
    assert_eq!(read_profile_file(&path).unwrap().accounts().len(), 2);
    let missing = read_profile_file(&dir.join("missing.json")).unwrap_err();
    assert!(matches!(missing, ImportError::Read { .. }));
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(matches!(
        read_profile_file(&path),
        Err(ImportError::NotJson)
    ));
    let _ = std::fs::remove_dir_all(dir);
}

// ---- mapping ----------------------------------------------------------------

#[test]
fn maps_every_item_or_reports_why_not() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let vault = vault_with("map", &[]);
    let plan = plan_import(&file, &vault, &FakeCatalog::new(), ConflictPolicy::Skip);
    assert_eq!(plan.count(AccountStatus::New), 2);

    let alice = account(&plan, "alice");
    assert_eq!(alice.status(), AccountStatus::New);
    let settings = alice.settings().unwrap();
    assert_eq!(settings.world, Some(2));
    assert_eq!(settings.lamp_skill, "attack");
    assert_eq!(settings.clue_duel_partner, "bob");
    assert_eq!(settings.raster, RasterMode::Off);
    assert_eq!(
        settings
            .script_assignment
            .as_ref()
            .map(ScriptAssignment::key),
        Some("catalog:CowKiller".to_string())
    );
    let bag = &settings.script_settings["catalog:CowKiller"];
    assert_eq!(bag["leashRadius"], json!(20));
    assert_eq!(bag["combatStyle"], json!("melee"));
    assert_eq!(bag["buryBones"], json!(true));
    assert!(!bag.contains_key("location"));
    assert!(!bag.contains_key("notASetting"));
    assert_eq!(
        skipped_items(alice),
        [
            "tab 'Combat'",
            "creds",
            "CowKiller.location",
            "CowKiller.notASetting",
            "Global.navTeleports",
            "Global.runAuto",
            "Loadouts.sets",
            "MapPicker.showBasemap",
        ]
    );

    let bob = account(&plan, "bob");
    let settings = bob.settings().unwrap();
    assert_eq!(settings.world, None);
    assert_eq!(settings.script_assignment, None);
    assert_eq!(settings.lamp_skill, "strength");
    let wood = &settings.script_settings["catalog:Woodcutter"];
    assert_eq!(wood["treeName"], json!("Oak"));
    assert_eq!(wood["leashRadius"], json!(30));
    assert!(bob
        .imported()
        .iter()
        .any(|line| line.contains("Woodcutter.leashRadius = 30 (400 limited")));
    assert_eq!(
        skipped_items(bob),
        [
            "selected script 'MyPrivateBot'",
            "Global.someFutureSetting",
            "Global.useMageBank",
            "MyPrivateBot.speed",
        ]
    );
    let unknown = &bob.skipped()[0];
    assert!(
        unknown.reason.contains("no script of this name"),
        "{unknown:?}"
    );
    let future = &bob.skipped()[1];
    assert!(future.reason.contains("not a Global setting"), "{future:?}");

    let file_items: Vec<&str> = plan
        .file_skipped()
        .iter()
        .map(|s| s.item.as_str())
        .collect();
    assert_eq!(
        file_items,
        ["tabs (Combat)", "stored settings for 'carol' (1 items)"]
    );

    // Nothing in the preview shows a password.
    let preview = plan.lines().join("\n");
    assert_no_password(&preview);
    assert!(preview.contains(PLAINTEXT_WARNING));
    assert!(preview.contains("not imported: Global.runAuto"));
}

#[test]
fn every_rs2b0t_global_setting_has_an_explicit_answer() {
    // rs2b0t `GLOBAL_SETTINGS_CORE` + `NAV_SETTINGS` keys (Settings.ts).
    let keys = [
        "lampSkill",
        "bankCommonJunk",
        "clueDuelPartner",
        "useZanarisBank",
        "useMageBank",
        "runAuto",
        "runEnergyMin",
        "navTeleports",
        "navPathStallTicks",
        "navPathDeviation",
        "navCameraFollow",
        "showNavPath",
        "navPathShowText",
        "navPathTextSize",
        "navPathColorPath",
        "navPathColorTransport",
        "navPathColorClick",
        "navPathColorText",
        "navPathSceneExpand",
        "navPathClientSegment",
        "navPathColorClient",
        "navPathColorClientRunAlt",
    ];
    for key in keys {
        let mut mapped = Mapped::default();
        map_global(&mut mapped, key, "attack");
        let answered = !mapped.imported.is_empty()
            || mapped
                .skipped
                .iter()
                .all(|s| !s.reason.contains("this importer knows"));
        assert!(answered, "{key} has no explicit mapping");
    }
}

#[test]
fn dimmed_scripts_keep_parameters_but_are_not_assigned() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let vault = vault_with("dim", &[]);
    let plan = plan_import(
        &file,
        &vault,
        &FakeCatalog::new().dim("CowKiller"),
        ConflictPolicy::Skip,
    );
    let alice = account(&plan, "alice");
    let settings = alice.settings().unwrap();
    assert_eq!(settings.script_assignment, None);
    assert_eq!(
        settings.script_settings["catalog:CowKiller"]["leashRadius"],
        json!(20)
    );
    let dimmed = alice
        .skipped()
        .iter()
        .find(|s| s.item == "selected script 'CowKiller'")
        .unwrap();
    assert!(dimmed.reason.contains("dimmed"), "{dimmed:?}");
}

#[test]
fn without_the_catalog_scripts_are_reported_not_dropped() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let vault = vault_with("no-catalog", &[]);
    let mut catalog = FakeCatalog::new();
    catalog.loaded = false;
    let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::Skip);
    let alice = account(&plan, "alice");
    assert!(alice.settings().unwrap().script_settings.is_empty());
    for item in [
        "selected script 'CowKiller'",
        "CowKiller.leashRadius",
        "CowKiller.buryBones",
    ] {
        let skip = alice.skipped().iter().find(|s| s.item == item).unwrap();
        assert!(skip.reason.contains("catalog is not loaded"), "{skip:?}");
    }
}

#[test]
fn existing_profiles_are_skipped_by_default_or_overwritten_on_request() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let vault = vault_with("conflict", &[("alice", 274_000_005)]);
    let catalog = FakeCatalog::new();

    let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::default());
    let alice = account(&plan, "alice");
    assert_eq!(alice.status(), AccountStatus::Exists);
    assert!(alice.settings().is_none());
    // A new profile's uid follows the vault's highest.
    assert_eq!(account(&plan, "bob").row.as_ref().unwrap().uid, 274_000_006);

    let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::Overwrite);
    let alice = account(&plan, "alice");
    assert_eq!(alice.status(), AccountStatus::Overwrite);
    let row = alice.row.as_ref().unwrap();
    assert_eq!(row.uid, 274_000_005);
    assert_eq!(row.password, "fake-pass-alice");
    // Settings the export does not carry stay as they were.
    assert!(!row.settings.lowmem);
    assert_eq!(row.settings.world, Some(2));
}

#[test]
fn duplicate_and_blank_usernames_are_reported() {
    let file = parse_profile_file(&with(|o| {
        o.insert(
            "profiles".into(),
            json!([
                { "username": "alice", "password": "fake-pass-alice" },
                { "username": "alice", "password": "fake-pass-alice" },
                { "username": "  ", "password": "fake-pass-bob" },
            ]),
        );
    }))
    .unwrap();
    let vault = vault_with("dupes", &[]);
    let plan = plan_import(&file, &vault, &FakeCatalog::new(), ConflictPolicy::Skip);
    let statuses: Vec<AccountStatus> = plan.accounts().iter().map(|a| a.status()).collect();
    assert_eq!(
        statuses,
        [
            AccountStatus::New,
            AccountStatus::Duplicate,
            AccountStatus::BlankName
        ]
    );
}

// ---- apply ------------------------------------------------------------------

#[test]
fn import_writes_through_the_session_and_reruns_are_idempotent() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let mut core = session(vault_with("apply", &[]));
    let catalog = FakeCatalog::new();

    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert!(run.settled(&core));
    assert!(run.succeeded(&core));
    let results = run.results(&core);
    assert_eq!(
        results,
        [
            ("alice", AccountResult::Created),
            ("bob", AccountResult::Created)
        ]
    );
    let alice = core.vault().unwrap().get("alice").unwrap().clone();
    assert_eq!(alice.password, "fake-pass-alice");
    assert_eq!(alice.settings.world, Some(2));
    assert_eq!(
        alice.settings.script_settings["catalog:CowKiller"]["combatStyle"],
        json!("melee")
    );
    let report = run.lines(&core).join("\n");
    assert_no_password(&report);
    assert!(report.contains("2 created"), "{report}");
    assert!(report.contains("alice: Global.runAuto"), "{report}");
    assert!(report.contains("file: tabs (Combat)"), "{report}");

    // Again with Skip: nothing to write.
    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    assert_eq!(plan.count(AccountStatus::Exists), 2);
    assert!(!plan.writes());
    // Again with Overwrite: the vault already matches the export.
    let plan = plan_import(
        &file,
        core.vault().unwrap(),
        &catalog,
        ConflictPolicy::Overwrite,
    );
    assert_eq!(plan.count(AccountStatus::Unchanged), 2);
    assert!(!plan.writes());
    let before: Vec<Profile> = core.vault().unwrap().profiles().cloned().collect();
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert_eq!(
        run.results(&core),
        [
            ("alice", AccountResult::NotWritten(AccountStatus::Unchanged)),
            ("bob", AccountResult::NotWritten(AccountStatus::Unchanged)),
        ]
    );
    let after: Vec<Profile> = core.vault().unwrap().profiles().cloned().collect();
    assert_eq!(before, after);
}

#[test]
fn overwrite_replaces_the_password_and_keeps_the_uid() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let mut core = session(vault_with("overwrite", &[("alice", 274_000_009)]));
    let plan = plan_import(
        &file,
        core.vault().unwrap(),
        &FakeCatalog::new(),
        ConflictPolicy::Overwrite,
    );
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert_eq!(run.results(&core)[0], ("alice", AccountResult::Overwritten));
    let alice = core.vault().unwrap().get("alice").unwrap();
    assert_eq!(alice.uid, 274_000_009);
    assert_eq!(alice.password, "fake-pass-alice");
    assert!(!alice.settings.lowmem);
}

#[test]
fn a_profile_created_after_the_preview_is_never_replaced() {
    let file = parse_profile_file(FIXTURE).unwrap();
    let mut core = session(vault_with("race", &[]));
    let plan = plan_import(
        &file,
        core.vault().unwrap(),
        &FakeCatalog::new(),
        ConflictPolicy::Skip,
    );
    let id = core
        .create_profile(
            Profile {
                username: "alice".into(),
                password: "typed-by-hand".into(),
                uid: 1,
                settings: ProfileSettings::default(),
            },
            ArmMirror::None,
            "credentials",
        )
        .unwrap();
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert_eq!(
        core.operation(id).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert!(matches!(
        run.results(&core)[0],
        ("alice", AccountResult::Failed(_))
    ));
    assert!(!run.succeeded(&core));
    assert_eq!(
        core.vault().unwrap().get("alice").unwrap().password,
        "typed-by-hand"
    );
}

#[test]
fn the_shared_flow_previews_confirms_and_deletes_only_on_request() {
    let dir = temp_dir("flow");
    let path = dir.join(PROFILE_FILE_NAME);
    std::fs::write(&path, FIXTURE).unwrap();
    let mut core = session(vault_with("flow-vault", &[]));
    let scripts = Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    let mut flow = ProfileImport::new();
    assert_eq!(flow.stage(), Stage::Path);
    flow.preview(&core, &scripts);
    assert!(flow.error().unwrap().contains(PROFILE_FILE_NAME));

    *flow.path_mut() = path.display().to_string();
    flow.preview(&core, &scripts);
    assert_eq!(flow.error(), None);
    assert_eq!(flow.stage(), Stage::Preview);
    assert!(
        core.vault().unwrap().get("alice").is_none(),
        "preview wrote"
    );
    // No catalog in this session: scripts are reported, not dropped.
    assert!(flow
        .preview_lines()
        .iter()
        .any(|line| line.contains("selected script 'CowKiller'") && line.contains("catalog")));

    flow.confirm(&mut core, &scripts);
    assert_eq!(flow.stage(), Stage::Report);
    core.flush_writes();
    assert!(core.vault().unwrap().get("alice").is_some());
    assert!(flow.can_delete(&core));
    assert!(path.exists(), "deleted without asking");
    flow.ask_delete();
    assert!(flow.delete_asked());
    flow.cancel_delete();
    assert!(path.exists());
    flow.ask_delete();
    flow.confirm_delete(&core);
    assert!(!path.exists());
    assert!(!flow.can_delete(&core));
    let report = flow.report_lines(&core).join("\n");
    assert!(report.contains("Deleted"), "{report}");
    assert_no_password(&report);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_locked_vault_refuses_the_preview() {
    let core = OperatorSession::<u32>::new(InstancePermit::SkipLock);
    let dir = temp_dir("locked");
    let scripts = Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    let mut flow = ProfileImport::new();
    *flow.path_mut() = "rs2b0t-profiles.json".into();
    flow.preview(&core, &scripts);
    assert!(flow.error().unwrap().contains("vault is locked"));
    let _ = std::fs::remove_dir_all(dir);
}
