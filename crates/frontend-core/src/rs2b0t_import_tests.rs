//! Parser and mapping proofs against a fixture written by rs2b0t's own
//! `serializeProfileFile` (rs2b0t 2de6bb23; generator and SHA-256 in the
//! RS2B0T-IMPORT evidence). Generic names and fake passwords only.

use std::path::PathBuf;

use host_play::{InstancePermit, Play, PlayOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings, RasterMode, ScriptAssignment, Vault};

use super::*;
use crate::operations::Outcome;

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
            .map(|(name, schema, dim)| {
                CatalogCard::new(
                    schema,
                    dim.as_deref(),
                    ScriptAssignment {
                        source_kind: "catalog".into(),
                        identity: name.clone(),
                        display_name: name.clone(),
                        unavailable: None,
                    },
                )
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
struct ImportSurface<'a> {
    scripts: &'a mut crate::scripts::Scripts,
    import: &'a mut ProfileImport,
}

impl crate::SlotSurface for ImportSurface<'_> {
    type Io = u32;

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut crate::SpawnProfile<'_>,
        retained: Option<u32>,
    ) -> crate::SlotAttach<u32> {
        profile.set_raster(crate::command::RasterMode::Off);
        crate::SlotAttach::new(retained.unwrap_or_default(), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> crate::Models<'_> {
        crate::Models::none()
            .with_scripts(self.scripts)
            .with_import(self.import)
    }

    fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {}
}

fn submit_import_row(
    core: &mut OperatorSession<u32>,
    scripts: &mut crate::scripts::Scripts,
    import: &mut ProfileImport,
    command: crate::OperatorCommand,
    targets: crate::Targets<'_>,
) {
    let mut surface = ImportSurface { scripts, import };
    core.submit(command, targets, &mut surface).unwrap();
}

fn submit_planned_import_rows(
    core: &mut OperatorSession<u32>,
    scripts: &mut crate::scripts::Scripts,
    import: &mut ProfileImport,
) -> Result<(), ImportError> {
    let planned = import
        .planned_writes()
        .map(|(index, name, command)| (index, name.to_owned(), command))
        .collect::<Vec<_>>();
    for (index, name, command) in planned {
        match command {
            crate::OperatorCommand::CreateProfile => {
                import.stage_create_account(index)?;
                submit_import_row(core, scripts, import, command, crate::Targets::Application);
            }
            crate::OperatorCommand::SaveProfile => {
                import.stage_save_account(index)?;
                submit_import_row(
                    core,
                    scripts,
                    import,
                    command,
                    crate::Targets::Profile(&name),
                );
            }
            _ => panic!("unexpected profile import writer: {command:?}"),
        }
        import.cancel_staged_account(index);
    }
    Ok(())
}

fn confirm_import(
    core: &mut OperatorSession<u32>,
    scripts: &mut crate::scripts::Scripts,
    import: &mut ProfileImport,
) {
    import.prepare_confirmation(core, scripts).unwrap();
    submit_planned_import_rows(core, scripts, import).unwrap();
}

fn apply_import(
    core: &mut OperatorSession<u32>,
    plan: ImportPlan,
) -> Result<ImportRun, ImportError> {
    static NEXT_SCRIPTS_DIR: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT_SCRIPTS_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = temp_dir(&format!("apply-import-{id}"));
    let mut scripts = crate::scripts::Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    let mut import = ProfileImport::new();
    import.start_run(PathBuf::new(), plan);
    submit_planned_import_rows(core, &mut scripts, &mut import)?;
    let Some((_, run)) = import.run.take() else {
        return Err(ImportFailure::NoPreview.into());
    };
    Ok(run)
}

fn account<'a>(plan: &'a ImportPlan, name: &str) -> &'a AccountPlan {
    plan.accounts()
        .iter()
        .find(|a| a.username() == name)
        .unwrap()
}

fn skipped_items(account: &AccountPlan) -> Vec<&str> {
    account.skipped().iter().map(Skipped::item).collect()
}

fn assert_no_password(text: &str) {
    for password in PASSWORDS {
        assert!(!text.contains(password), "password leaked in: {text}");
    }
}

// ==== review regressions (RS2B0T-IMPORT-REVIEW probes) ======================
// These use only the API the reviewed commit e06be28d2 already had, so the
// same tests show each failure there and the fix here.

const REVIEW_SECRET: &str = "audit-secret-19371";

/// The review's Thiever card: two number parameters and a tile.
struct ThieverCatalog(Vec<script::SettingDef>);

impl ThieverCatalog {
    fn new() -> Self {
        Self(vec![
            def("leashRadius", "number", &[], Some("3"), Some("40")),
            def("foodWithdraw", "number", &[], Some("1"), Some("27")),
            def("spot", "tile", &[], None, None),
        ])
    }
}

impl ImportCatalog for ThieverCatalog {
    fn loaded(&self) -> bool {
        true
    }

    fn card(&self, name: &str) -> Option<CatalogCard<'_>> {
        (name == "Thiever").then(|| {
            CatalogCard::new(
                &self.0,
                None,
                ScriptAssignment {
                    source_kind: "catalog".into(),
                    identity: name.into(),
                    display_name: name.into(),
                    unavailable: None,
                },
            )
        })
    }
}

/// One account `user` (a usable generic password, world 1) with `storage`.
fn probe_file_for(user: &str, storage: Value) -> ExportFile {
    parse_profile_file(
        &json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [{ "username": user, "password": "generic-pass", "world": 1 }],
            "tabs": [],
            "activeTab": "Main",
            "storage": { user: storage },
        })
        .to_string(),
    )
    .unwrap()
}

fn probe_file(storage: Value) -> ExportFile {
    probe_file_for("rvimprobe", storage)
}

/// A vault holding `rvimprobe` with the Thiever bag `bag`.
fn probe_vault(test: &str, bag: Value) -> Vault {
    let mut vault = vault_with(test, &[]);
    let mut settings = ProfileSettings {
        lowmem: false,
        ..ProfileSettings::default()
    };
    settings
        .script_settings
        .insert("catalog:Thiever".into(), bag.as_object().unwrap().clone());
    vault
        .upsert(Profile {
            username: "rvimprobe".into(),
            password: "old-pass".into(),
            uid: 274_000_012,
            settings,
        })
        .unwrap();
    vault
}

/// A session whose every commit fails: its vault path becomes a directory.
fn failing_session(test: &str) -> OperatorSession<u32> {
    let dir = temp_dir(test);
    let path = dir.join("vault");
    let core = session(Vault::create(&path, "test-passphrase-01").unwrap());
    std::fs::rename(&path, dir.join("vault.original")).unwrap();
    std::fs::create_dir(&path).unwrap();
    core
}

fn durable(result: &AccountResult) -> bool {
    matches!(result, AccountResult::Created | AccountResult::Overwritten)
}

/// Review H3: a malformed header's value (here holding a password) never
/// reaches an error's text or its `Debug`.
#[test]
fn review_h3_a_credential_in_a_bad_header_never_reaches_an_error() {
    let cases = [
        // The review's input.
        json!({
            "kind": { "password": REVIEW_SECRET },
            "v": 1,
            "profiles": [{ "username": "rvimprobe", "password": REVIEW_SECRET }],
            "tabs": [],
            "activeTab": "Main",
        }),
        json!({ "kind": REVIEW_SECRET, "v": 1, "profiles": [], "tabs": [], "activeTab": "Main" }),
        json!({
            "kind": PROFILE_FILE_KIND,
            "v": { "password": REVIEW_SECRET },
            "profiles": [],
            "tabs": [],
            "activeTab": "Main",
        }),
        json!({
            "kind": PROFILE_FILE_KIND,
            "v": [REVIEW_SECRET],
            "profiles": [],
            "tabs": [],
            "activeTab": "Main",
        }),
        json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [],
            "tabs": [],
            "activeTab": "Main",
            "storage": { REVIEW_SECRET: "x" },
        }),
        // Re-review R2-H3: the password repeated as another listed
        // account's username, keying a damaged storage box.
        json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [
                { "username": "rv2probe", "password": REVIEW_SECRET },
                { "username": REVIEW_SECRET, "password": "rv2-generic-pass" },
            ],
            "tabs": [],
            "activeTab": "Main",
            "storage": { REVIEW_SECRET: 5 },
        }),
        json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [
                { "username": "rv2probe", "password": REVIEW_SECRET },
                { "username": REVIEW_SECRET, "password": "rv2-generic-pass" },
            ],
            "tabs": [],
            "activeTab": "Main",
            "storage": { REVIEW_SECRET: { "k": 1 } },
        }),
    ];
    for case in cases {
        let error = parse_profile_file(&case.to_string()).unwrap_err();
        let shown = format!("{error} | {error:?}");
        assert!(!shown.contains(REVIEW_SECRET), "{shown}");
    }
    // A file that parses keeps file text out of its `Debug` too: the
    // secret here sits in a username, a tab and a stored value.
    let file = parse_profile_file(
        &json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [
                { "username": "rv2probe", "password": REVIEW_SECRET, "tab": REVIEW_SECRET },
                { "username": REVIEW_SECRET, "password": "rv2-generic-pass" },
            ],
            "tabs": [REVIEW_SECRET],
            "activeTab": REVIEW_SECRET,
            "storage": { "rv2probe": { "other": REVIEW_SECRET } },
        })
        .to_string(),
    )
    .unwrap();
    let shown = format!("{file:?}");
    assert!(!shown.contains(REVIEW_SECRET), "{shown}");
}

/// Review H2: Overwrite keeps a card's parameters the export does not
/// carry, and a refused value keeps the profile's own.
#[test]
fn review_h2_overwrite_keeps_what_the_export_does_not_carry() {
    let vault = probe_vault("review-h2", json!({ "leashRadius": 8, "foodWithdraw": 27 }));
    let catalog = ThieverCatalog::new();

    // The review's case: only leashRadius is in the export.
    let file = probe_file(json!({ "set:Thiever:leashRadius": "99" }));
    let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::Overwrite);
    let settings = plan.accounts()[0].settings().unwrap();
    let bag = &settings.script_settings["catalog:Thiever"];
    assert_eq!(bag["leashRadius"], json!(40));
    assert_eq!(bag.get("foodWithdraw"), Some(&json!(27)), "{bag:?}");
    assert!(!settings.lowmem);

    // A refused value: the current one stays.
    let file = probe_file(json!({
        "set:Thiever:leashRadius": "12",
        "set:Thiever:foodWithdraw": "lots",
    }));
    let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::Overwrite);
    let bag = &plan.accounts()[0].settings().unwrap().script_settings["catalog:Thiever"];
    assert_eq!(bag["leashRadius"], json!(12));
    assert_eq!(bag.get("foodWithdraw"), Some(&json!(27)), "{bag:?}");
}

/// Review M2: stored numbers and tile parts are read with JavaScript's
/// `Number()`, as rs2b0t reads them, then limited to the range.
#[test]
fn review_m2_stored_numbers_are_read_as_rs2b0t_reads_them() {
    let vault = vault_with("review-m2", &[]);
    let catalog = ThieverCatalog::new();
    let imported = |storage: Value, key: &str| {
        let file = probe_file(storage);
        let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::Skip);
        plan.accounts()[0]
            .settings()
            .unwrap()
            .script_settings
            .get("catalog:Thiever")
            .and_then(|bag| bag.get(key))
            .cloned()
    };
    for (raw, want) in [
        ("0x10", 16),
        ("0b1000", 8),
        ("0o17", 15),
        // Empty is 0, then limited to the minimum of 1.
        ("", 1),
        (" 1e1 ", 10),
        ("12", 12),
        ("99", 27),
    ] {
        assert_eq!(
            imported(json!({ "set:Thiever:foodWithdraw": raw }), "foodWithdraw"),
            Some(json!(want)),
            "{raw:?}"
        );
    }
    for raw in ["-0x10", "Infinity", "0x", "1e", "12px", "0x1g"] {
        assert_eq!(
            imported(json!({ "set:Thiever:foodWithdraw": raw }), "foodWithdraw"),
            None,
            "{raw:?}"
        );
    }
    for (raw, want) in [
        (" 3200 , 0x10 ", json!({ "x": 3200, "z": 16, "level": 0 })),
        ("3200,3201,", json!({ "x": 3200, "z": 3201, "level": 0 })),
        ("3200,3201,1", json!({ "x": 3200, "z": 3201, "level": 1 })),
    ] {
        assert_eq!(
            imported(json!({ "set:Thiever:spot": raw }), "spot"),
            Some(want),
            "{raw:?}"
        );
    }
}

/// Review H1, reproducer B: a failed import stays failed after its record
/// leaves the session's bounded history; a write nobody saw settle before
/// that is not reported saved either.
#[test]
fn review_h1_an_evicted_failure_is_never_reported_saved() {
    let mut core = failing_session("review-h1-evicted");
    let catalog = ThieverCatalog::new();
    let file = probe_file(json!({}));
    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    let run = apply_import(&mut core, plan).unwrap();
    let file = probe_file_for("rvimunseen", json!({}));
    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    let unseen = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    let AccountResult::Failed(first_error) = run.results(&core)[0].1 else {
        panic!("the write must fail");
    };
    for _ in 0..65 {
        core.stop_scripts(&["not-loaded".into()]);
    }
    let results = run.results(&core);
    assert!(
        matches!(results[0].1, AccountResult::Failed(_)),
        "{results:?}"
    );
    let AccountResult::Failed(cached_error) = results[0].1 else {
        unreachable!();
    };
    assert!(std::ptr::eq(first_error, cached_error));
    assert!(!run.succeeded(&core));
    let results = unseen.results(&core);
    assert!(!durable(&results[0].1), "{results:?}");
    assert!(!unseen.succeeded(&core));
    assert_eq!(core.vault().unwrap().profiles().count(), 0);
}

/// Review H1, reproducer A: an import superseded by a later save of the
/// same profile in a commit that failed is not reported saved.
#[test]
fn review_h1_a_superseded_failed_import_is_not_reported_saved() {
    let mut core = failing_session("review-h1-superseded");
    let gate = core.write_gate();
    let hold = gate.lock().unwrap();
    let file = probe_file(json!({}));
    let plan = plan_import(
        &file,
        core.vault().unwrap(),
        &ThieverCatalog::new(),
        ConflictPolicy::Skip,
    );
    let run = apply_import(&mut core, plan).unwrap();
    let row = core.vault().unwrap().get("rvimprobe").unwrap().clone();
    let later = core
        .save_profile(row, ArmMirror::none(), "review later write")
        .unwrap();
    drop(hold);
    core.flush_writes();
    assert!(matches!(
        core.operation(later).unwrap().outcome("rvimprobe"),
        Some(o) if o.is_failed()
    ));
    let results = run.results(&core);
    assert!(!durable(&results[0].1), "{results:?}");
    assert!(!run.succeeded(&core));
    assert_eq!(core.vault().unwrap().profiles().count(), 0);
}

/// Review M1: a FIFO is not read without bound (the review's stream was a
/// valid export followed by 17 MiB of spaces).
#[cfg(unix)]
#[test]
fn review_m1_a_fifo_is_not_read_without_bound() {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};

    let dir = temp_dir("review-m1");
    let fifo = dir.join("large.fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: `name` is a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let writer_path = fifo.clone();
    let writer = std::thread::spawn(move || {
        // Waits for a reader; stops at the first refused write.
        let Ok(mut out) = std::fs::OpenOptions::new().write(true).open(&writer_path) else {
            return;
        };
        let header = br#"{"kind":"rs2b0t-multibox-profiles","v":1,"profiles":[],"tabs":[],"activeTab":"Main"}"#;
        if out.write_all(header).is_err() {
            return;
        }
        let padding = [b' '; 65536];
        for _ in 0..272 {
            if out.write_all(&padding).is_err() {
                return;
            }
        }
    });
    let read = read_profile_file(&fifo);
    // Release a writer still waiting for a reader, so it ends.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !writer.is_finished() && Instant::now() < deadline {
        drop(
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo),
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(read.is_err(), "{read:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review M3: an account whose credentials the 289 server refuses (a
/// 26-character password in the review's live run) is not imported by
/// default, and nothing shows the credential.
#[test]
fn review_m3_credentials_the_289_server_refuses_are_not_imported_by_default() {
    let long = "abcdefghijklmnopqrstuvwxyz";
    let file = parse_profile_file(
        &json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [
                { "username": "rvimlong", "password": long },
                { "username": "rvimempty", "password": "" },
                { "username": "rvimthirteen1", "password": "generic-pass" },
                { "username": "rvimlatin", "password": "pass\u{1F600}word" },
                { "username": "rvimfine", "password": "twenty-chars-exactly" },
            ],
            "tabs": [],
            "activeTab": "Main",
        })
        .to_string(),
    )
    .unwrap();
    let vault = vault_with("review-m3", &[]);
    let plan = plan_import(&file, &vault, &FakeCatalog::new(), ConflictPolicy::Skip);
    for unusable in &plan.accounts()[..4] {
        assert!(unusable.settings().is_none(), "{}", unusable.username());
        assert_ne!(unusable.status(), AccountStatus::New);
    }
    assert_eq!(plan.accounts()[4].status(), AccountStatus::New);
    assert!(!plan.lines().join("\n").contains(long));
}

// ==== end of review regressions ==============================================

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
fn refuses_another_kind_or_version_by_the_type_found() {
    let error = parse_profile_file(&with(|o| {
        o.insert("kind".into(), json!("something-else"));
    }))
    .unwrap_err();
    assert!(
        matches!(
            error.0,
            ImportFailure::Kind {
                found: JsonType::String
            }
        ),
        "{error:?}"
    );
    assert!(!error.to_string().contains("something-else"), "{error}");

    let error = parse_profile_file(&with(|o| {
        o.remove("kind");
    }))
    .unwrap_err();
    assert!(
        matches!(
            error.0,
            ImportFailure::Kind {
                found: JsonType::Missing
            }
        ),
        "{error:?}"
    );

    let error = parse_profile_file(&with(|o| {
        o.insert("v".into(), json!(2));
    }))
    .unwrap_err();
    assert!(
        matches!(
            error.0,
            ImportFailure::Version {
                found: JsonType::Number
            }
        ),
        "{error:?}"
    );

    let error = parse_profile_file(&with(|o| {
        o.insert("v".into(), json!("1"));
    }))
    .unwrap_err();
    assert!(
        matches!(
            error.0,
            ImportFailure::Version {
                found: JsonType::String
            }
        ),
        "{error:?}"
    );
}

#[test]
fn refuses_malformed_shapes_without_quoting_a_password() {
    assert!(matches!(
        parse_profile_file("not json"),
        Err(ImportError(ImportFailure::NotJson))
    ));
    assert!(matches!(
        parse_profile_file("[1]"),
        Err(ImportError(ImportFailure::Shape))
    ));
    for missing in ["profiles", "tabs", "activeTab"] {
        let error = parse_profile_file(&with(|o| {
            o.remove(missing);
        }))
        .unwrap_err();
        assert!(
            matches!(error.0, ImportFailure::MissingLists),
            "{missing}: {error}"
        );
    }
    let error = parse_profile_file(&with(|o| {
        o.insert("tabs".into(), json!(["ok", 3]));
    }))
    .unwrap_err();
    assert!(matches!(error.0, ImportFailure::TabsNotStrings));

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
            matches!(error.0, ImportFailure::Entry { index: 1, .. }),
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
    assert!(
        matches!(error.0, ImportFailure::World { index: 1 }),
        "{error}"
    );
    assert_no_password(&error.to_string());
    let error = parse_profile_file(&with(|o| {
        o.insert(
            "profiles".into(),
            json!([{ "username": "alice", "password": "fake-pass-alice", "tab": 1 }]),
        );
    }))
    .unwrap_err();
    assert!(
        matches!(error.0, ImportFailure::Tab { index: 1 }),
        "{error}"
    );

    let alice = || StorageOwner::Listed(1);
    for (storage, want) in [
        (json!([]), ImportFailure::Storage),
        (json!({ "": {} }), ImportFailure::StorageUser),
        (
            json!({ "alice": "x" }),
            ImportFailure::StorageBox { owner: alice() },
        ),
        (
            json!({ "alice": { "k": 1 } }),
            ImportFailure::StorageValue { owner: alice() },
        ),
        (
            json!({ "fake-pass-alice": "x" }),
            ImportFailure::StorageBox {
                owner: StorageOwner::Unlisted,
            },
        ),
    ] {
        let error = parse_profile_file(&with(|o| {
            o.insert("storage".into(), storage);
        }))
        .unwrap_err();
        assert_eq!(format!("{error:?}"), format!("{want:?}"));
        assert_no_password(&error.to_string());
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
    assert!(matches!(missing.0, ImportFailure::Read { .. }));
    let cause = std::error::Error::source(&missing)
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(cause.kind(), std::io::ErrorKind::NotFound);
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(matches!(
        read_profile_file(&path),
        Err(ImportError(ImportFailure::NotJson))
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
        unknown.reason().contains("no script of this name"),
        "{unknown:?}"
    );
    let future = &bob.skipped()[1];
    assert!(
        future.reason().contains("not a Global setting"),
        "{future:?}"
    );

    let file_items: Vec<&str> = plan.file_skipped().iter().map(Skipped::item).collect();
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
        .find(|s| s.item() == "selected script 'CowKiller'")
        .unwrap();
    assert!(dimmed.reason().contains("dimmed"), "{dimmed:?}");
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
        let skip = alice.skipped().iter().find(|s| s.item() == item).unwrap();
        assert!(skip.reason().contains("catalog is not loaded"), "{skip:?}");
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
            ArmMirror::none(),
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
    let mut scripts = Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    let mut flow = ProfileImport::new();
    assert_eq!(flow.stage(), Stage::Path);
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::PreviewProfileImport,
        crate::Targets::Application,
    );
    assert!(flow.error().unwrap().contains(PROFILE_FILE_NAME));

    *flow.path_mut() = path.display().to_string();
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::EditProfileImportPath,
        crate::Targets::Application,
    );
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::PreviewProfileImport,
        crate::Targets::Application,
    );
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

    confirm_import(&mut core, &mut scripts, &mut flow);
    assert_eq!(flow.stage(), Stage::Report);
    core.flush_writes();
    assert!(core.vault().unwrap().get("alice").is_some());
    assert!(flow.can_delete(&core));
    assert!(path.exists(), "deleted without asking");
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::SetProfileImportDeletePrompt(true),
        crate::Targets::Application,
    );
    assert!(flow.delete_asked());
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::SetProfileImportDeletePrompt(false),
        crate::Targets::Application,
    );
    assert!(!flow.delete_asked());
    assert!(path.exists());
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::SetProfileImportDeletePrompt(true),
        crate::Targets::Application,
    );
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::DeleteProfileImportExport,
        crate::Targets::Application,
    );
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
    assert!(flow.preview(&core, &scripts).is_err());
    assert!(flow.error().unwrap().contains("vault is locked"));
    let _ = std::fs::remove_dir_all(dir);
}

// ---- round 2: durable results, merge report, reading, numbers ---------------

#[test]
fn js_number_reads_strings_as_ecmascript_string_to_number() {
    let read = [
        ("", 0.0),
        ("  \t", 0.0),
        ("\u{feff}\u{a0} 12 \n\u{2028}", 12.0),
        ("00012", 12.0),
        ("0x10", 16.0),
        ("0X1f", 31.0),
        ("0o17", 15.0),
        ("0b1000", 8.0),
        ("0x0000", 0.0),
        ("1e3", 1000.0),
        ("-1.5E-1", -0.15),
        (".5", 0.5),
        ("5.", 5.0),
        ("+7", 7.0),
        ("1e400", f64::INFINITY),
        ("Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
        // 2^53 - 1 is exact; 2^53 + 1 and 2^53 + 3 round half to even.
        ("0x1fffffffffffff", 9_007_199_254_740_991.0),
        ("0x20000000000001", 9_007_199_254_740_992.0),
        ("0x20000000000003", 9_007_199_254_740_996.0),
        // 80 bits of ones round up to 2^80.
        (
            "0xffffffffffffffffffff",
            1_208_925_819_614_629_174_706_176.0,
        ),
    ];
    for (text, want) in read {
        assert_eq!(js_string_to_number(text), want, "{text:?}");
    }
    assert!(js_string_to_number("-0").is_sign_negative());
    for text in [
        "-0x10", "+0x10", "0x", "0b2", "0o8", "0x 1", "1e", "1e+", "e5", ".", "+", "1.2.3", "12px",
        "1_000", "infinity", "inf", "NaN", "+-1", "\u{85}1",
    ] {
        assert!(js_string_to_number(text).is_nan(), "{text:?}");
    }
}

#[test]
fn tiles_must_be_whole() {
    let vault = vault_with("tiles", &[]);
    let catalog = ThieverCatalog::new();
    for raw in ["3200.5,3200", "3200", "a,b", "1,2,Infinity"] {
        let file = probe_file(json!({ "set:Thiever:spot": raw }));
        let plan = plan_import(&file, &vault, &catalog, ConflictPolicy::Skip);
        let account = &plan.accounts()[0];
        assert!(
            account.settings().unwrap().script_settings.is_empty(),
            "{raw:?}"
        );
        assert_eq!(skipped_items(account), ["Thiever.spot"], "{raw:?}");
    }
}

#[test]
fn the_file_is_read_only_up_to_the_limit() {
    use std::cell::Cell;
    use std::rc::Rc;

    struct Counting<R> {
        inner: R,
        read: Rc<Cell<u64>>,
    }
    impl<R: std::io::Read> std::io::Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read.set(self.read.get() + n as u64);
            Ok(n)
        }
    }
    // An endless stream: refused once one byte past the limit is read.
    let read = Rc::new(Cell::new(0));
    let endless = Counting {
        inner: std::io::repeat(b' '),
        read: Rc::clone(&read),
    };
    assert!(read_bounded(endless, 1024, 0).unwrap().is_none());
    assert!(read.get() <= 1025, "read {} bytes", read.get());
    let exact = std::io::Read::take(std::io::repeat(b' '), 1024);
    assert_eq!(
        read_bounded(exact, 1024, 1024).unwrap().unwrap().len(),
        1024
    );

    let dir = temp_dir("bounded");
    // A regular file past 16 MiB (sparse, so nothing is written).
    let big = dir.join("big.json");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_FILE_BYTES + 1)
        .unwrap();
    assert!(matches!(
        read_profile_file(&big),
        Err(ImportError(ImportFailure::TooLarge { .. }))
    ));
    // A directory is not a file.
    assert!(matches!(
        read_profile_file(&dir),
        Err(ImportError(ImportFailure::NotAFile { .. }))
    ));
    // A FIFO with no writer is refused at once, never waited on.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let fifo = dir.join("empty.fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `name` is a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(matches!(
            read_profile_file(&fifo),
            Err(ImportError(ImportFailure::NotAFile { .. }))
        ));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn overwrite_says_per_parameter_what_happens_and_what_stays() {
    let vault = probe_vault(
        "merge-report",
        json!({ "leashRadius": 8, "foodWithdraw": 27, "spot": { "x": 1, "z": 2, "level": 0 } }),
    );
    let file = probe_file(json!({
        "set:Thiever:leashRadius": "99",
        "set:Thiever:foodWithdraw": "lots",
    }));
    let plan = plan_import(
        &file,
        &vault,
        &ThieverCatalog::new(),
        ConflictPolicy::Overwrite,
    );
    let account = &plan.accounts()[0];
    let leash = account
        .imported()
        .iter()
        .find(|line| line.starts_with("Thiever.leashRadius"))
        .unwrap();
    assert!(leash.contains("was 8"), "{leash}");
    assert_eq!(account.kept().len(), 1, "{:?}", account.kept());
    assert!(account.kept()[0].starts_with("Thiever.spot"));
    let refused = account
        .skipped()
        .iter()
        .find(|s| s.item() == "Thiever.foodWithdraw")
        .unwrap();
    assert!(refused.reason().contains("27"), "{refused:?}");

    // The report carries the same per-parameter lines.
    let mut core = session(vault);
    let plan = plan_import(
        &file,
        core.vault().unwrap(),
        &ThieverCatalog::new(),
        ConflictPolicy::Overwrite,
    );
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert_eq!(run.results(&core)[0].1, AccountResult::Overwritten);
    let report = run.lines(&core);
    assert!(report.iter().any(|line| line.contains("Thiever.spot")));
    assert!(report.iter().any(|line| line.contains("was 8")));
    let bag = &core
        .vault()
        .unwrap()
        .get("rvimprobe")
        .unwrap()
        .settings
        .script_settings["catalog:Thiever"];
    assert_eq!(bag["foodWithdraw"], json!(27));
    assert_eq!(bag["spot"], json!({ "x": 1, "z": 2, "level": 0 }));
}

#[test]
fn an_overwrite_without_a_world_keeps_the_profiles_world() {
    let mut vault = probe_vault("keep-world", json!({}));
    let mut row = vault.get("rvimprobe").unwrap().clone();
    row.settings.world = Some(2);
    vault.upsert(row).unwrap();
    let file = parse_profile_file(&with(|o| {
        o.insert(
            "profiles".into(),
            json!([{ "username": "rvimprobe", "password": "generic-pass" }]),
        );
    }))
    .unwrap();
    let plan = plan_import(
        &file,
        &vault,
        &FakeCatalog::new(),
        ConflictPolicy::Overwrite,
    );
    assert_eq!(plan.accounts()[0].settings().unwrap().world, Some(2));
}

#[test]
fn unusable_accounts_are_imported_only_on_request_and_never_shortened() {
    let long = "abcdefghijklmnopqrstuvwxyz";
    let file = parse_profile_file(
        &json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [{ "username": "rvimlong", "password": long }],
            "tabs": [],
            "activeTab": "Main",
        })
        .to_string(),
    )
    .unwrap();
    let vault = vault_with("unusable", &[]);
    let plan = plan_import(&file, &vault, &FakeCatalog::new(), ConflictPolicy::Skip);
    let account = &plan.accounts()[0];
    assert_eq!(account.status(), AccountStatus::Unusable);
    assert_eq!(account.problem(), Some(CredentialProblem::PasswordTooLong));
    assert!(!plan.writes());

    let plan = plan_import(
        &file,
        &vault,
        &FakeCatalog::new(),
        ImportChoices {
            policy: ConflictPolicy::Skip,
            include_unusable: true,
        },
    );
    let account = &plan.accounts()[0];
    assert_eq!(account.status(), AccountStatus::New);
    assert_eq!(account.problem(), Some(CredentialProblem::PasswordTooLong));
    assert_eq!(account.row.as_ref().unwrap().password, long);
    assert!(!plan.lines().join("\n").contains(long));

    for (username, password, want) in [
        (
            "rvimthirteen1",
            "generic-pass",
            CredentialProblem::UsernameTooLong,
        ),
        (
            "rvim\nline",
            "generic-pass",
            CredentialProblem::UsernameCharacters,
        ),
        ("rvimempty", "", CredentialProblem::PasswordEmpty),
        (
            "rvimemoji",
            "pass\u{1F600}",
            CredentialProblem::PasswordCharacters,
        ),
    ] {
        assert_eq!(
            credential_problem(username, password),
            Some(want),
            "{username:?}"
        );
    }
    // Latin-1 and exactly the limits are fine.
    assert_eq!(
        credential_problem("twelve-chars", "caf\u{e9}-twenty-chars"),
        None
    );
}

#[test]
fn results_are_durable_only_from_the_imports_own_settled_write() {
    // A durable write nobody saw settle before its record left the
    // session's history: not confirmed.
    let mut core = session(vault_with("unknown", &[]));
    let catalog = ThieverCatalog::new();
    let file = probe_file(json!({}));
    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    for _ in 0..65 {
        core.stop_scripts(&["not-loaded".into()]);
    }
    assert_eq!(run.results(&core)[0].1, AccountResult::Unknown);
    assert!(!run.succeeded(&core));

    // Seen settled once, it stays durable after the record is gone.
    let file = probe_file_for("rvimseen", json!({}));
    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert_eq!(run.results(&core)[0].1, AccountResult::Created);
    for _ in 0..65 {
        core.stop_scripts(&["not-loaded".into()]);
    }
    assert_eq!(run.results(&core)[0].1, AccountResult::Created);
    assert!(run.succeeded(&core));

    // Superseded in a commit that succeeded: the import's own write is
    // not confirmed, though the later one is.
    let gate = core.write_gate();
    let hold = gate.lock().unwrap();
    let file = probe_file_for("rvimlater", json!({}));
    let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
    let run = apply_import(&mut core, plan).unwrap();
    let row = core.vault().unwrap().get("rvimlater").unwrap().clone();
    core.save_profile(row, ArmMirror::remember(None), "later edit")
        .unwrap();
    drop(hold);
    core.flush_writes();
    assert_eq!(run.results(&core)[0].1, AccountResult::Superseded);
    assert!(!run.succeeded(&core));
}

#[test]
fn the_shared_flow_offers_deletion_only_when_every_write_is_durable() {
    let dir = temp_dir("flow-failed");
    let path = dir.join(PROFILE_FILE_NAME);
    std::fs::write(&path, FIXTURE).unwrap();
    let mut core = failing_session("flow-failed-vault");
    let mut scripts = Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    let mut flow = ProfileImport::new();
    *flow.path_mut() = path.display().to_string();
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::EditProfileImportPath,
        crate::Targets::Application,
    );
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::PreviewProfileImport,
        crate::Targets::Application,
    );
    confirm_import(&mut core, &mut scripts, &mut flow);
    assert!(flow.writing(&core));
    core.flush_writes();
    assert!(flow.settled(&core));
    assert!(!flow.can_delete(&core));
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::SetProfileImportDeletePrompt(true),
        crate::Targets::Application,
    );
    assert!(!flow.delete_asked());
    assert!(path.exists(), "a failed import cannot offer deletion");
    assert_no_password(&flow.report_lines(&core).join("\n"));
    let _ = std::fs::remove_dir_all(dir);
}

// ---- round 4: a password repeated in another field (review R3-H3) ----------
// The redaction registry is process-wide, so each test (and each matrix
// case) uses its own password and checks only that it is absent.

/// A one-account export: `rvr4probe` with password `secret`, then `edit`.
fn export_with_password(secret: &str, edit: impl FnOnce(&mut Value, &str)) -> String {
    let mut raw = json!({
        "kind": PROFILE_FILE_KIND,
        "v": 1,
        "profiles": [{ "username": "rvr4probe", "password": secret }],
        "tabs": [],
        "activeTab": "Main",
        "storage": { "rvr4probe": {} },
    });
    edit(&mut raw, secret);
    raw.to_string()
}

/// Every place a test's password showed. All surfaces are checked before
/// the test fails, so a failure names each one.
#[derive(Default)]
struct Leaks(Vec<String>);

impl Leaks {
    fn check(&mut self, secret: &str, what: &str, text: &str) {
        if text.contains(secret) {
            self.0.push(format!("{what}: {text}"));
        }
    }

    fn assert_none(self) {
        assert!(
            self.0.is_empty(),
            "the password shows in:\n{}",
            self.0.join("\n")
        );
    }
}

/// Review input A: the account's password repeated as its tab. The tab is
/// reported as not imported, without the password, in the preview, the
/// `Skipped` values and the report after the real write.
#[test]
fn r3_h3_a_password_repeated_in_a_tab_reaches_no_preview_report_or_debug() {
    const SECRET: &str = "r4a-tab-pw-60231";
    let file = parse_profile_file(&export_with_password(SECRET, |raw, secret| {
        raw["profiles"][0]["tab"] = json!(secret);
    }))
    .unwrap();
    let mut core = session(vault_with("r4-input-a", &[]));
    let mut leaks = Leaks::default();
    let plan = plan_import(
        &file,
        core.vault().unwrap(),
        &FakeCatalog::new(),
        ConflictPolicy::Skip,
    );
    let preview = plan.lines().join("\n");
    assert!(preview.contains("not imported: tab"), "{preview}");
    leaks.check(SECRET, "the preview", &preview);
    let skipped = format!(
        "{:?} {:?}",
        plan.file_skipped(),
        account(&plan, "rvr4probe").skipped()
    );
    leaks.check(SECRET, "Skipped Debug", &skipped);
    let run = apply_import(&mut core, plan).unwrap();
    core.flush_writes();
    assert!(run.succeeded(&core));
    let report = run.lines(&core).join("\n");
    assert!(report.contains("rvr4probe: tab"), "{report}");
    leaks.check(SECRET, "the report", &report);
    // The vault still gets the real password.
    let row = core.vault().unwrap().get("rvr4probe").unwrap();
    assert_eq!(row.password.as_str(), SECRET);
    leaks.assert_none();
}

/// Review input B: a skipped account's password as another account's clue
/// duel partner, through the real shared flow. No line of the process log
/// shows it, and the vault holds the partner as the file gives it.
#[test]
fn r3_h3_b_a_skipped_accounts_password_reaches_no_log_line() {
    const SECRET: &str = "r4b-skip-pw-60232";
    let store = crate::log::global();
    let dir = temp_dir("r4-input-b");
    let path = dir.join(PROFILE_FILE_NAME);
    std::fs::write(
        &path,
        json!({
            "kind": PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [
                { "username": "r4bsource", "password": SECRET },
                { "username": "r4btarget", "password": "r4b-generic-pass" },
            ],
            "tabs": [],
            "activeTab": "Main",
            "storage": { "r4btarget": { "set:Global:clueDuelPartner": SECRET } },
        })
        .to_string(),
    )
    .unwrap();
    let mut core = session(vault_with(
        "r4-input-b-vault",
        &[("r4bsource", 274_000_041)],
    ));
    let mut scripts = Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    let mut flow = ProfileImport::new();
    *flow.path_mut() = path.display().to_string();
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::EditProfileImportPath,
        crate::Targets::Application,
    );
    submit_import_row(
        &mut core,
        &mut scripts,
        &mut flow,
        crate::OperatorCommand::PreviewProfileImport,
        crate::Targets::Application,
    );
    assert_eq!(flow.error(), None);
    let mut leaks = Leaks::default();
    let plan = flow.plan().unwrap();
    assert_eq!(account(plan, "r4bsource").status(), AccountStatus::Exists);
    assert_eq!(account(plan, "r4btarget").status(), AccountStatus::New);
    leaks.check(SECRET, "the preview", &flow.preview_lines().join("\n"));
    confirm_import(&mut core, &mut scripts, &mut flow);
    core.flush_writes();
    assert!(flow.settled(&core));
    leaks.check(SECRET, "the report", &flow.report_lines(&core).join("\n"));

    let mut view = crate::log::LogView::new(crate::log::LogScope::All);
    store.refresh(&mut view);
    let messages: Vec<&str> = view.rows().iter().map(|row| row.message()).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("create profile r4btarget")),
        "the import logged no line for its write"
    );
    for message in &messages {
        leaks.check(SECRET, "a log line", message);
    }
    let row = core.vault().unwrap().get("r4btarget").unwrap();
    assert_eq!(row.settings.clue_duel_partner, SECRET);
    let _ = std::fs::remove_dir_all(dir);
    leaks.assert_none();
}

/// The review's Thiever card for the residual matrix: one parameter of
/// each kind a stored value can be refused or imported as.
struct ResidualCatalog(Vec<script::SettingDef>);

impl ImportCatalog for ResidualCatalog {
    fn loaded(&self) -> bool {
        true
    }

    fn card(&self, name: &str) -> Option<CatalogCard<'_>> {
        (name == "Thiever").then(|| CatalogCard {
            schema: &self.0,
            unloadable: None,
            assignment: ScriptAssignment {
                source_kind: "catalog".into(),
                identity: name.into(),
                display_name: name.into(),
                unavailable: None,
            },
        })
    }
}

/// The rest of the review's matrix: the password in each other field the
/// import shows reaches no preview line, no `Skipped` or imported line, no
/// report line after the real write and no log line; an imported value
/// still reaches the vault as the file gives it.
#[test]
fn r3_h3_a_password_repeated_in_any_other_field_reaches_no_text() {
    type Edit = fn(&mut Value, &str);
    let cases: [(&str, Edit); 20] = [
        ("tab list", |raw, s| raw["tabs"] = json!([s])),
        ("active tab", |raw, s| raw["activeTab"] = json!(s)),
        ("unlisted storage owner", |raw, s| {
            raw["storage"][s] = json!({ "k": "x" });
        }),
        ("unknown storage key", |raw, s| {
            raw["storage"]["rvr4probe"][s] = json!("x");
        }),
        ("unknown Global setting", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:Global:{s}")] = json!("x");
        }),
        ("unknown script", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:{s}:k")] = json!("x");
        }),
        ("unknown parameter", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:Thiever:{s}")] = json!("x");
        }),
        ("malformed settings key", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:{s}")] = json!("x");
        }),
        ("MapPicker setting", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:MapPicker:{s}")] = json!("x");
        }),
        ("Loadouts setting", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:Loadouts:{s}")] = json!("x");
        }),
        ("PriceBooks setting", |raw, s| {
            raw["storage"]["rvr4probe"][format!("set:PriceBooks:{s}")] = json!("x");
        }),
        ("rejected renderer value", |raw, s| {
            raw["storage"]["rvr4probe"]["rendererEnabled"] = json!(s);
        }),
        ("unknown selected script", |raw, s| {
            raw["storage"]["rvr4probe"]["selectedScript"] = json!(s);
        }),
        ("rejected lamp skill", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Global:lampSkill"] = json!(s);
        }),
        ("rejected number", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Thiever:number"] = json!(s);
        }),
        ("rejected tile", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Thiever:tile"] = json!(s);
        }),
        ("rejected option", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Thiever:choice"] = json!(s);
        }),
        ("dropped list entry", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Thiever:list"] = json!(s);
        }),
        ("imported clue duel partner", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Global:clueDuelPartner"] = json!(s);
        }),
        ("imported string parameter", |raw, s| {
            raw["storage"]["rvr4probe"]["set:Thiever:text"] = json!(s);
        }),
    ];
    let catalog = ResidualCatalog(vec![
        def("number", "number", &[], None, None),
        def("tile", "tile", &[], None, None),
        def("text", "string", &[], None, None),
        def("choice", "string", &["yes"], None, None),
        def("list", "list", &["yes"], None, None),
    ]);
    let store = crate::log::global();
    let mut leaks = Leaks::default();
    let mut secrets = Vec::new();
    for (index, (label, edit)) in cases.into_iter().enumerate() {
        let secret = format!("r4m{index:02}-pw-60233");
        let file = parse_profile_file(&export_with_password(&secret, edit)).unwrap();
        let mut core = session(vault_with(&format!("r4-matrix-{index}"), &[]));
        let plan = plan_import(&file, core.vault().unwrap(), &catalog, ConflictPolicy::Skip);
        let what = |part: &str| format!("{label}: {part}");
        leaks.check(&secret, &what("preview"), &plan.lines().join("\n"));
        let probe = account(&plan, "rvr4probe");
        assert_eq!(probe.status(), AccountStatus::New, "{label}");
        let kept = format!(
            "{:?} {:?} {:?}",
            plan.file_skipped(),
            probe.skipped(),
            probe.imported()
        );
        leaks.check(&secret, &what("Skipped Debug"), &kept);
        let run = apply_import(&mut core, plan).unwrap();
        core.flush_writes();
        assert!(run.succeeded(&core), "{label}");
        leaks.check(&secret, &what("report"), &run.lines(&core).join("\n"));
        let row = core.vault().unwrap().get("rvr4probe").unwrap();
        match label {
            "imported clue duel partner" => {
                assert_eq!(row.settings.clue_duel_partner, secret);
            }
            "imported string parameter" => assert_eq!(
                row.settings
                    .script_settings
                    .get("catalog:Thiever")
                    .and_then(|bag| bag.get("text")),
                Some(&json!(secret))
            ),
            _ => {}
        }
        secrets.push((label, secret));
    }
    let mut view = crate::log::LogView::new(crate::log::LogScope::All);
    store.refresh(&mut view);
    for row in view.rows() {
        for (label, secret) in &secrets {
            leaks.check(secret, &format!("{label}: log line"), row.message());
        }
    }
    leaks.assert_none();
}
