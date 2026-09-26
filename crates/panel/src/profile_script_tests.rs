use super::*;
use crate::test_support::TestDir;
use frontend_core::scripts::ReloadOutcome;
use std::fs;
use std::time::{Duration, Instant};
use vault::{Profile, ProfileSettings, Vault};

/// Pump worker validation and per-frame Start settlement until all script
/// lifecycle work has settled.
fn settle(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while (s.reload_validation_pending() || s.scripts.starts_pending()) && Instant::now() < deadline
    {
        s.core.poll_host();
        s.poll_scripts();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !s.reload_validation_pending(),
        "script validation did not settle"
    );
    assert!(!s.scripts.starts_pending(), "script Start did not settle");
}

fn reload(s: &mut Session) -> ReloadOutcome {
    s.begin_script_reload_clicked();
    settle(s);
    s.take_reload_outcome()
        .expect("reload operation must publish an outcome")
}

fn refresh_catalog(s: &mut Session, root: &std::path::Path) -> ReloadOutcome {
    s.begin_refresh_catalog_at(root);
    settle(s);
    s.take_reload_outcome()
        .expect("catalog refresh must publish an outcome")
}

/// Pump `name`'s lifecycle until it reaches `want` (Stop returns
/// before the reap).
fn wait_state(s: &Session, name: &str, want: script::RunState) {
    let play = s.core.play().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while play.script_state(name) != want && Instant::now() < deadline {
        play.pump_script_lifecycle(name);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(play.script_state(name), want, "{name}");
}

/// Operator Start of `sel` on `profile` through the shared coordinator.
fn start_sel(s: &mut Session, profile: &str, sel: script::ScriptSel) -> Result<(), String> {
    s.scripts
        .start_selected(&mut s.core, profile, Some(&sel), None)
}

fn run_generations(s: &Session, names: &[&str]) -> Vec<(String, u64)> {
    let play = s.core.play().unwrap();
    names
        .iter()
        .filter_map(|n| {
            play.script_runtime_generation(n)
                .map(|g| (n.to_string(), g))
        })
        .collect()
}

fn tmp(name: &str) -> TestDir {
    TestDir::new(&format!("profile-script-{name}"))
}

fn session_with_profiles(names: &[&str]) -> (Session, TestDir) {
    script::IsolatedEnv::ensure_thread();
    let dir = tmp("vault");
    let path = dir.join("v.vault");
    let mut vault = Vault::create(&path, "bot").unwrap();
    for (i, name) in names.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*name).into(),
                password: "pw".into(),
                uid: 1 + i as i32,
                settings: ProfileSettings::default(),
            })
            .unwrap();
    }
    let mut s = Session::new();
    s.core.set_vault(Some(vault));
    s.persist_ui = false;
    (s, dir)
}

#[test]
fn focus_restores_assignment_not_other_profile_pending() {
    let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
    s.set_pending_browse(
        "alice",
        script::ScriptSel::Loaded(script::ScriptSource::Catalog, "ChickenKiller".into()),
    );
    s.persist_successful_assignment(
        "bob",
        ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "Alcher".into(),
            display_name: "Alcher".into(),
            unavailable: None,
        },
    );
    s.restore_script_heading("alice");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "ChickenKiller".into()
        ))
    );
    s.restore_script_heading("bob");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "Alcher".into()
        ))
    );
    s.set_focus_for_test("bob");
    assert!(!s.heading_is_pending());
    s.set_focus_for_test("alice");
    s.restore_script_heading("alice");
    assert!(s.heading_is_pending());
}

#[test]
fn two_profiles_keep_isolated_settings_after_legacy_claim() {
    let (mut s, dir) = session_with_profiles(&["alice", "bob"]);
    let store = dir.join("script-settings.json");
    fs::write(&store, r#"{"catalog:SmithingBot":{"bar":"Adamant"}}"#).unwrap();
    s.scripts.legacy = script::ScriptSettingsStore::at(store);
    let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
    assert_eq!(a.get("bar"), Some(&Value::String("Adamantite".into())));
    s.set_profile_setting(
        "alice",
        script::ScriptSource::Catalog,
        "SmithingBot",
        Path::new(""),
        "bar",
        Value::String("Iron".into()),
    );
    let b = s.profile_overrides("bob", "catalog:SmithingBot", "SmithingBot");
    assert_eq!(b.get("bar"), Some(&Value::String("Adamantite".into())));
    let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
    assert_eq!(a.get("bar"), Some(&Value::String("Iron".into())));
}

#[test]
fn missing_file_assignment_is_kept_not_substituted() {
    let (mut s, _dir) = session_with_profiles(&["alice"]);
    s.persist_successful_assignment(
        "alice",
        script::missing_file_assignment(
            Path::new("/tmp/missing-bot.ts"),
            "bot",
            "missing file: /tmp/missing-bot.ts",
        ),
    );
    let asg = s.profile_assignment("alice").unwrap();
    assert_eq!(asg.identity, "/tmp/missing-bot.ts");
    assert!(asg.unavailable.is_some());
    assert_ne!(asg.display_name, asg.identity);
    s.restore_script_heading("alice");
    match &s.script_sel {
        Some(script::ScriptSel::Loaded(script::ScriptSource::File, id)) => {
            assert_eq!(id, "/tmp/missing-bot.ts");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn start_all_without_play_reports_per_profile_and_is_idempotent() {
    let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
    s.core.fleet_mut().add("alice");
    s.core.fleet_mut().add("bob");
    s.script_start_all();
    let first = s.error.clone().unwrap();
    assert!(first.contains("Start all"));
    assert!(first.contains("failed"));
    s.script_start_all();
    assert_eq!(s.error.as_deref(), Some(first.as_str()));
}

#[test]
fn same_stem_files_are_distinct_and_hash_whitespace() {
    script::IsolatedEnv::ensure_thread();
    let dir = tmp("files");
    let a = dir.join("one");
    let b = dir.join("two");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let fa = a.join("bot.ts");
    let fb = b.join("bot.ts");
    let src = "export default class T extends LoopingBot { override loop() {} }\n";
    fs::write(&fa, src).unwrap();
    fs::write(&fb, src).unwrap();
    let mut lib = script::JsLibrary::with_cache(dir.join("store.json"), dir.join("cache"));
    let ca = lib.load(&fa).unwrap();
    let cb = lib.load(&fb).unwrap();
    assert_ne!(ca.identity_key(), cb.identity_key());
    assert_eq!(ca.name, cb.name);
    assert!(!lib
        .raw_source_changed(script::ScriptSource::File, &fa.to_string_lossy())
        .unwrap());
    fs::write(&fa, format!("{src} ")).unwrap();
    assert!(lib
        .raw_source_changed(script::ScriptSource::File, &fa.to_string_lossy())
        .unwrap());
    assert!(!lib
        .raw_source_changed(script::ScriptSource::File, &fb.to_string_lossy())
        .unwrap());
}

const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

fn empty_play() -> host_play::Play {
    host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
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

fn session_with_play(names: &[&str]) -> (Session, TestDir) {
    let (mut s, dir) = session_with_profiles(names);
    s.scripts.js = script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    let mut play = empty_play();
    for name in names {
        play.attach_arm(name, host_play::SlotArm::new(42, false));
        s.core.fleet_mut().add(name);
    }
    s.core.set_play(Some(play));
    if let Some(first) = names.first() {
        s.set_focus_for_test(first);
    }
    (s, dir)
}

fn write_bot(dir: &std::path::Path, file: &str, src: &str) -> std::path::PathBuf {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, src).unwrap();
    path
}

fn fake_catalog(root: &std::path::Path, bots: &[(&str, &str)]) {
    let scripts = root.join("src/bot/scripts");
    fs::create_dir_all(&scripts).unwrap();
    let mut index =
        String::from("import { ScriptRegistry } from '../runtime/ScriptRegistry.js';\n");
    for (name, src) in bots {
        let folder = name.replace(' ', "");
        index.push_str(&format!("import {folder} from './{folder}/{folder}.js';\n"));
        let d = scripts.join(&folder);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(format!("{folder}.ts")), src).unwrap();
    }
    for (name, _) in bots {
        let folder = name.replace(' ', "");
        index.push_str(&format!(
            "ScriptRegistry.register({{ name: '{name}', description: '{name}', category: 'Test', create: () => new {folder}() }});\n"
        ));
    }
    fs::write(scripts.join("index.ts"), index).unwrap();
}

#[test]
fn initial_runtime_load_failure_survives_another_card_start() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let bad_source = "export const apiVersion = 2;\nthrow new Error('initial-load-proof');\nexport function tick(api) {}\n";
    let bad_path = write_bot(&dir, "bad.ts", bad_source);
    let bad = s.scripts.js.load(&bad_path).unwrap();
    s.script_sel = Some(script::ScriptSel::Loaded(
        bad.source,
        bad_path.to_string_lossy().into_owned(),
    ));
    s.script_start_selected();
    settle(&mut s);
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("initial-load-proof"),
        "{:?}",
        s.error
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Idle
    );
    assert!(
        s.profile_assignment("alice").is_none(),
        "a broken Start is never the profile's assignment"
    );

    let good_path = write_bot(&dir, "good.ts", BOT_TS);
    s.scripts.js.load(&good_path).unwrap();
    start_file_on(&mut s, "bob", &good_path);
    let failure = s
        .scripts
        .js
        .load_failure(&bad.identity_key())
        .expect("initial runtime failure must remain inspectable after another card starts");
    assert_eq!(failure.path, bad_path);
    assert_eq!(failure.stage, script::load::LoadStage::RuntimeLoad);
    assert_eq!(
        failure.fingerprint,
        script::raw_content_fingerprint(&bad_path, bad_source)
    );
    assert_eq!(failure.api_family, Some(script::ApiFamily::V2));
    assert!(s
        .scripts
        .js
        .named_failure_output()
        .contains("initial-load-proof"));
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn initial_load_refusal_preserves_failure_and_retry_clears_only_its_identity() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let helper = write_bot(&dir, "gate.ts", "export const fail = true;");
    let source = "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('retry-load-proof');\nexport function tick(api) {}";
    let path = write_bot(&dir, "retry.ts", source);
    let card = s.scripts.js.load(&path).unwrap();
    let sel = script::ScriptSel::Loaded(card.source, path.to_string_lossy().into_owned());
    // Start returns before V8 setup: the runtime failure settles after
    // it and reaches the operator as the Start error it used to be.
    start_sel(&mut s, "alice", sel.clone()).expect("Start is accepted before setup");
    settle(&mut s);
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("retry-load-proof"),
        "{:?}",
        s.error
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Idle
    );
    let failure = s
        .scripts
        .js
        .load_failure(&card.identity_key())
        .unwrap()
        .clone();
    assert!(s.profile_assignment("alice").is_none());
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("alice"),
        Some(0)
    );
    assert_eq!(
        start_sel(&mut s, "missing", sel.clone()).unwrap_err(),
        "no slot: missing"
    );
    assert_eq!(
        s.scripts.js.load_failure(&card.identity_key()),
        Some(&failure)
    );

    let other_path = write_bot(
        &dir,
        "other.ts",
        &format!("throw new Error('other-load-proof');\n{BOT_TS}"),
    );
    let other = s.scripts.js.load(&other_path).unwrap();
    start_sel(
        &mut s,
        "bob",
        script::ScriptSel::Loaded(other.source, other_path.to_string_lossy().into_owned()),
    )
    .expect("Start is accepted before setup");
    settle(&mut s);
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("other-load-proof"),
        "{:?}",
        s.error
    );
    assert!(s.profile_assignment("bob").is_none());
    let other_failure = s
        .scripts
        .js
        .load_failure(&other.identity_key())
        .unwrap()
        .clone();
    assert_eq!(other_failure.api_family, Some(script::ApiFamily::V1));
    assert_eq!(other_failure.stage, script::LoadStage::RuntimeLoad);

    // Resolve the changed sibling on explicit Start, without replacing the card
    // or running a throwaway validation isolate that could clear the diagnostic.
    fs::write(&helper, "export const fail = false;").unwrap();
    start_sel(&mut s, "alice", sel.clone()).unwrap();
    settle(&mut s);
    assert!(s.scripts.js.load_failure(&card.identity_key()).is_none());
    assert_eq!(
        s.scripts.js.load_failure(&other.identity_key()),
        Some(&other_failure)
    );
    let play = s.core.play().unwrap();
    assert_eq!(play.script_runtime_generation("alice"), Some(1));
    assert_eq!(
        play.script_source_identity("alice"),
        Some(card.identity_key())
    );
    assert_eq!(
        start_sel(
            &mut s,
            "alice",
            script::ScriptSel::Loaded(other.source, other_path.to_string_lossy().into_owned())
        )
        .unwrap_err(),
        "script already active: stop it first"
    );
    assert_eq!(
        s.scripts.js.load_failure(&other.identity_key()),
        Some(&other_failure)
    );
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("alice"),
        Some(1)
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn claim_legacy_read_path_writes_vault_once() {
    let (mut s, dir) = session_with_profiles(&["alice"]);
    let vault_path = dir.join("v.vault");
    let before = fs::read(&vault_path).unwrap();
    let _ = s.profile_overrides("alice", "catalog:Alcher", "Alcher");
    s.core.flush_writes();
    let after_claim = fs::read(&vault_path).unwrap();
    assert_ne!(
        before, after_claim,
        "first claim may persist the migrated key"
    );
    let _ = s.merged_profile_bag(
        "alice",
        script::ScriptSource::Catalog,
        "Alcher",
        Path::new(""),
        &[],
    );
    let _ = s.profile_overrides("alice", "catalog:Alcher", "Alcher");
    s.core.flush_writes();
    let after_reads = fs::read(&vault_path).unwrap();
    assert_eq!(
        after_claim, after_reads,
        "already-migrated read path must not rewrite the vault"
    );
}

#[test]
fn external_ts_load_start_stop_persists_assignment_only_on_success() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "ext.ts", BOT_TS);
    assert!(s.profile_assignment("alice").is_none());
    s.load_js(&path);
    assert_eq!(s.error, None, "{:?}", s.error);
    assert!(s.heading_is_pending());
    assert!(s.profile_assignment("alice").is_none());
    s.enqueue_transpile(
        script::ScriptSource::File,
        path.to_string_lossy().into_owned(),
        true,
    );
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let play = s.core.play().unwrap();
    assert_eq!(play.script_state("alice"), script::RunState::Running);
    let asg = s
        .profile_assignment("alice")
        .expect("success-only assignment");
    assert!(asg.identity.contains("ext.ts"), "{}", asg.identity);
    assert!(asg.unavailable.is_none());
    play.script_stop("alice");
    wait_state(&s, "alice", script::RunState::Idle);
    assert_eq!(
        s.profile_assignment("alice").unwrap().identity,
        asg.identity,
        "stop keeps last successful assignment"
    );
}

#[test]
fn load_js_selects_without_auto_start_and_same_path_does_not_duplicate() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "once.ts", BOT_TS);
    s.load_js(&path);
    s.enqueue_transpile(
        script::ScriptSource::File,
        path.to_string_lossy().into_owned(),
        true,
    );
    assert_eq!(s.error, None, "{:?}", s.error);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Idle,
        "load must not Start"
    );
    let first = s
        .scripts
        .js
        .cards()
        .iter()
        .filter(|c| c.source == script::ScriptSource::File)
        .count();
    assert_eq!(first, 1);
    s.load_js(&path);
    let again = s
        .scripts
        .js
        .cards()
        .iter()
        .filter(|c| c.source == script::ScriptSource::File)
        .count();
    assert_eq!(again, 1, "same path must replace, not duplicate");
    s.script_start_selected();
    settle(&mut s);
    s.core.play().unwrap().script_stop("alice");
    assert_eq!(reload(&mut s), ReloadOutcome::NothingChanged);
}

#[test]
fn reload_unchanged_reports_exact_string() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "same.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let out = reload(&mut s);
    settle(&mut s);
    assert_eq!(out, ReloadOutcome::NothingChanged);
    assert_eq!(s.error.as_deref(), Some(script::NOTHING_CHANGED_RELOAD));
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn ui_reload_validation_keeps_the_control_thread_responsive() {
    let (mut session, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "runaway.ts", BOT_TS);
    session.load_js(&path);
    let old_js = session
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js
        .clone();
    fs::write(
        &path,
        "while (true) {}\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();

    let started = Instant::now();
    session.begin_script_reload_clicked();
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the UI action must not wait for hostile module evaluation"
    );
    assert!(
        session.reload_validation_pending(),
        "the throwaway V8 runtime belongs to the worker"
    );
    assert_eq!(
        session.focused_name().as_deref(),
        Some("alice"),
        "unrelated control state remains immediately readable"
    );

    settle(&mut session);
    assert!(
        session
            .error
            .as_deref()
            .is_some_and(|error| error.contains("prepare") || error.contains("timed out")),
        "{:?}",
        session.error
    );
    assert_eq!(
        session
            .scripts
            .js
            .get(script::ScriptSource::File, &path.to_string_lossy())
            .expect("old card remains installed")
            .js,
        old_js,
        "failed worker validation cannot replace the live library card"
    );
}

#[test]
fn catalog_validation_keeps_the_control_thread_responsive() {
    let (mut session, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-worker");
    fake_catalog(&root, &[("RunawayBot", BOT_TS)]);
    session
        .scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    session
        .scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "RunawayBot")
        .unwrap();
    let old_js = session
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "RunawayBot")
        .expect("loaded card")
        .js
        .clone();
    fs::write(
        root.join("src/bot/scripts/RunawayBot/RunawayBot.ts"),
        "while (true) {}\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();

    let started = Instant::now();
    session.begin_refresh_catalog_at(&root);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "catalog refresh must not wait for hostile module evaluation"
    );
    assert!(
        session.reload_validation_pending(),
        "catalog validation belongs to the worker"
    );
    assert_eq!(session.focused_name().as_deref(), Some("alice"));

    settle(&mut session);
    assert!(matches!(
        session.take_reload_outcome(),
        Some(ReloadOutcome::Applied { .. })
    ));
    assert_eq!(
        session
            .scripts
            .js
            .get(script::ScriptSource::Catalog, "RunawayBot")
            .expect("old card remains installed")
            .js,
        old_js,
        "failed worker validation cannot replace the live catalog card"
    );
    assert!(
        session
            .scripts
            .catalog_refresh_report()
            .is_some_and(|report| report.contains("failed")),
        "{:?}",
        session.scripts.catalog_refresh_report()
    );
}

#[test]
fn external_unchanged_reload_dispatches_validation_once() {
    let (mut session, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "ExampleBot.ts", BOT_TS);
    session.load_js(&path);
    let card = session
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .expect("loaded File card")
        .clone();

    let watch = host_play::external_loader::ExternalWatch::default();
    let now = Instant::now();
    watch.configure("alice", path.clone(), card.sha256.clone());
    watch.note_scene(true, 2);
    watch.note_inventory(now, "alice", host_play::external_loader::BONES_COUNT, 0);
    watch.note_prereq_passed();
    watch.note_load(
        1,
        host_play::external_loader::SCRIPT_NAME,
        &path,
        &card.identity_key(),
        &card.sha256,
        true,
        false,
    );
    watch.begin_start(now).unwrap();

    session.script_start_selected();
    settle(&mut session);
    assert_eq!(session.error, None, "{:?}", session.error);

    let burial_logs = (1..=host_play::external_loader::BONES_COUNT)
        .map(|n| format!("script: Buried bones #{n}"))
        .collect::<Vec<_>>();
    watch.note_logs(now + Duration::from_millis(1), "alice", &burial_logs);
    watch.note_inventory(now + Duration::from_millis(1), "alice", 12, 45);
    watch.request_stop(now + Duration::from_millis(2));
    session.script_stop();
    wait_state(&session, "alice", script::RunState::Idle);
    watch.note_logs(
        now + Duration::from_millis(2),
        "alice",
        &["script: BoneBurier stopped — 10 buried, +45 prayer xp".into()],
    );
    watch.note_stop(now + Duration::from_millis(2), true, false);
    assert_eq!(
        watch.requested_operation(),
        Some(host_play::external_loader::Operation::ReloadUnchanged)
    );

    fs::write(&path, format!("{BOT_TS}// unexpected external change\n")).unwrap();

    session.install_external_core_watch(Some(watch.clone()));
    session.pump_external_loader();
    assert!(
        session.reload_validation_pending(),
        "first dispatch must launch asynchronous validation"
    );
    settle(&mut session);
    session.pump_external_loader();

    assert_eq!(
        watch.status(),
        host_play::external_loader::ExternalWatchStatus::Failed
    );
    assert!(
        watch
            .failure()
            .as_deref()
            .is_some_and(|failure| failure.contains("unchanged reload")),
        "{:?}",
        watch.failure()
    );
}

#[test]
fn reload_clicked_warns_before_replacing_running() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "run.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let old_js = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .js
        .clone();
    fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
    let first = reload(&mut s);
    assert_eq!(first, ReloadOutcome::NeedsConfirm);
    assert!(
        s.error.as_deref().unwrap_or("").contains("running"),
        "{:?}",
        s.error
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    let now_js = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .js
        .clone();
    assert_eq!(old_js, now_js, "preview must not replace registration");
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_commit_gates_pause_during_prep() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "pause.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
    s.begin_script_reload_clicked();
    assert!(s.reload_validation_pending());
    s.core.play().unwrap().script_pause("alice");
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Paused
    );
    let old_js = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .js
        .clone();
    settle(&mut s);
    assert_eq!(s.take_reload_outcome(), Some(ReloadOutcome::NeedsConfirm));
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("paused during prepare"),
        "{:?}",
        s.error
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Paused
    );
    let now_js = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .js
        .clone();
    assert_eq!(old_js, now_js);
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn prepare_failure_preserves_old_instance() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "keep.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    let old = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .clone();
    fs::write(&path, "const x = 1;\n").unwrap();
    let out = reload(&mut s);
    settle(&mut s);
    match out {
        ReloadOutcome::Failed(e) => assert!(
            e.contains("shape") || e.contains("unloadable") || e.contains("prepare"),
            "{e}"
        ),
        other => panic!("expected Failed, got {other:?}"),
    }
    let now = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap();
    assert_eq!(now.origin, old.origin);
    assert_eq!(now.js, old.js);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn live_settings_reach_running_and_paused() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let src = "export default class T extends LoopingBot { override loop() {} }\n";
    let path = write_bot(&dir, "live.ts", src);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    assert!(s.set_profile_setting(
        "alice",
        script::ScriptSource::File,
        "live",
        &path,
        "n",
        Value::String("2".into()),
    ));
    // The run receives the bag once the write is durable.
    s.core.flush_writes();
    let play = s.core.play().unwrap();
    let identity = play.script_source_identity("alice").unwrap();
    let gen = play.script_runtime_generation("alice").unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("n".into(), Value::String("2".into()));
    assert!(
        !play.script_post_settings_fenced("alice", &bag, &identity, gen),
        "unchanged bag after live Running delivery is not reposted"
    );
    play.script_pause("alice");
    assert!(s.set_profile_setting(
        "alice",
        script::ScriptSource::File,
        "live",
        &path,
        "n",
        Value::String("3".into()),
    ));
    s.core.flush_writes();
    bag.insert("n".into(), Value::String("3".into()));
    let play = s.core.play().unwrap();
    assert!(
        !play.script_post_settings_fenced("alice", &bag, &identity, gen),
        "unchanged bag after live Paused delivery is not reposted"
    );
    play.script_stop("alice");
    assert!(!play.script_post_settings_fenced("alice", &bag, &identity, gen));
}

#[test]
fn catalog_prepare_failure_does_not_mutate_or_block_valid() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog");
    fake_catalog(&root, &[("GoodBot", BOT_TS), ("BadBot", BOT_TS)]);
    s.scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "GoodBot")
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "BadBot")
        .unwrap();
    let good_js = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "GoodBot")
        .unwrap()
        .js
        .clone();
    let bad = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "BadBot")
        .unwrap()
        .clone();
    fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        "export default class T extends LoopingBot { override loop() { return; } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/bot/scripts/BadBot/BadBot.ts"),
        "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    let good = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "GoodBot")
        .unwrap();
    assert!(!good.js.is_empty(), "successful prepare must keep js");
    assert_ne!(good.js, good_js, "changed good card commits prepared js");
    let bad_now = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "BadBot")
        .unwrap();
    assert_eq!(bad_now.origin, bad.origin);
    assert_eq!(bad_now.js, bad.js);
    assert!(
        s.error.as_deref().unwrap_or("").contains("BadBot")
            || s.scripts
                .catalog_refresh_report()
                .unwrap_or("")
                .contains("failed"),
        "independent failure is reported: {:?} {:?}",
        s.error,
        s.scripts.catalog_refresh_report()
    );
    assert!(
        s.scripts
            .js
            .load_failures()
            .iter()
            .any(|f| f.name == "BadBot"),
        "failed catalog card stays inspectable: {:?}",
        s.scripts.js.load_failures()
    );
}

#[test]
fn catalog_mixed_batch_keeps_two_failures_after_success() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-mixed");
    fake_catalog(
        &root,
        &[
            ("GoodBot", BOT_TS),
            ("BadParse", BOT_TS),
            ("BadImport", BOT_TS),
        ],
    );
    s.scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "GoodBot")
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "BadParse")
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "BadImport")
        .unwrap();
    fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        "export default class T extends LoopingBot { override loop() { return; } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/bot/scripts/BadParse/BadParse.ts"),
        "export default class T extends LoopingBot { override loop() { const x = \"unterminated } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/bot/scripts/BadImport/BadImport.ts"),
        "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    let names: Vec<_> = s
        .scripts
        .js
        .load_failures()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(
        names.contains(&"BadParse") && names.contains(&"BadImport"),
        "both failures stay inspectable: {names:?} err={:?} report={:?}",
        s.error,
        s.scripts.catalog_refresh_report()
    );
    assert!(!names.contains(&"GoodBot"));
    assert!(s.scripts.js.named_failure_output().contains("BadParse"));
    assert!(s
        .scripts
        .catalog_refresh_report()
        .unwrap_or("")
        .contains("BadParse"));
}

#[test]
fn catalog_refresh_warns_before_stopping_running() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-run");
    fake_catalog(&root, &[("RunBot", BOT_TS)]);
    s.scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "RunBot")
        .unwrap();
    s.persist_successful_assignment(
        "alice",
        ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "RunBot".into(),
            display_name: "RunBot".into(),
            unavailable: None,
        },
    );
    s.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "RunBot".into(),
    ));
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    fs::write(
        root.join("src/bot/scripts/RunBot/RunBot.ts"),
        format!("{BOT_TS}// changed\n"),
    )
    .unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running,
        "catalog refresh must warn before stopping"
    );
    assert!(
        s.scripts.reload_warning().is_some()
            || s.error.as_deref().unwrap_or("").contains("running"),
        "{:?} {:?}",
        s.error,
        s.scripts.reload_warning().map(|w| &w.running)
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn start_after_launch_runs_saved_catalog_assignment_before_any_browse() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-launch");
    fake_catalog(&root, &[("RunBot", BOT_TS)]);
    let iso = script::IsolatedEnv::enter("launch-start");
    iso.set_rs2b0t(&root);
    s.persist_successful_assignment(
        "alice",
        ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "RunBot".into(),
            display_name: "RunBot".into(),
            unavailable: None,
        },
    );
    focus_profile(&mut s, "alice");
    assert!(
        s.scripts
            .js
            .get(script::ScriptSource::Catalog, "RunBot")
            .is_none(),
        "fresh launch: catalog not filled yet"
    );
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

fn focus_profile(s: &mut Session, name: &str) {
    s.set_focus_for_test(name);
    s.restore_script_heading(name);
}

fn start_file_on(s: &mut Session, profile: &str, path: &std::path::Path) {
    focus_profile(s, profile);
    let sel = script::ScriptSel::Loaded(
        script::ScriptSource::File,
        path.to_string_lossy().into_owned(),
    );
    s.script_sel = Some(sel.clone());
    s.set_pending_browse(profile, sel);
    s.script_start_selected();
    settle(s);
    assert_eq!(s.error, None, "{profile} start: {:?}", s.error);
}

fn warn_shared_reload(s: &mut Session, path: &std::path::Path) {
    fs::write(path, format!("{BOT_TS}// changed\n")).unwrap();
    focus_profile(s, "alice");
    assert_eq!(reload(s), ReloadOutcome::NeedsConfirm);
}

fn assert_applied(out: ReloadOutcome, restarted: usize, failed: usize) {
    match out {
        ReloadOutcome::Applied {
            restarted: got_r,
            failed: got_f,
            ..
        } => {
            assert_eq!(got_r, restarted, "restarted");
            assert_eq!(got_f, failed, "failed");
        }
        other => {
            panic!("expected Applied {{ restarted: {restarted}, failed: {failed} }}, got {other:?}")
        }
    }
}

#[test]
fn cancel_reload_preserves_running_and_paused_executions() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    s.core.play().unwrap().script_pause("bob");
    let generations = run_generations(&s, &["alice", "bob"]);
    let old_js = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .js
        .clone();
    warn_shared_reload(&mut s, &path);
    assert!(s.script_reload_confirmation_pending());
    s.cancel_reload();
    assert!(!s.script_reload_confirmation_pending());
    assert_eq!(run_generations(&s, &["alice", "bob"]), generations);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    assert_eq!(
        s.core.play().unwrap().script_state("bob"),
        script::RunState::Paused
    );
    assert_eq!(
        s.scripts
            .js
            .get(script::ScriptSource::File, &path.to_string_lossy())
            .unwrap()
            .js,
        old_js
    );
    // A later click must prepare and warn again, never reuse cancelled consent.
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    s.core.play().unwrap().script_stop("alice");
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn reload_confirm_does_not_authorize_switched_selection() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path_a = write_bot(&dir, "a.ts", BOT_TS);
    let path_b = write_bot(&dir, "b.ts", BOT_TS);
    s.load_js(&path_a);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    focus_profile(&mut s, "bob");
    s.load_js(&path_b);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    s.core.play().unwrap().script_pause("bob");
    let old_b = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path_b.to_string_lossy())
        .unwrap()
        .js
        .clone();
    fs::write(&path_a, format!("{BOT_TS}// a changed\n")).unwrap();
    fs::write(&path_b, format!("{BOT_TS}// b changed\n")).unwrap();
    focus_profile(&mut s, "alice");
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    assert!(
        s.scripts
            .reload_warning()
            .is_some_and(|w| w.running.iter().any(|n| n == "alice")),
        "{:?}",
        s.scripts.reload_warning().map(|w| &w.running)
    );
    focus_profile(&mut s, "bob");
    let second = reload(&mut s);
    settle(&mut s);
    assert_eq!(
        second,
        ReloadOutcome::NeedsConfirm,
        "A's warning must not confirm B"
    );
    assert_eq!(
        s.core.play().unwrap().script_state("bob"),
        script::RunState::Paused,
        "B must not be replaced without its own warning"
    );
    let now_b = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path_b.to_string_lossy())
        .unwrap()
        .js
        .clone();
    assert_eq!(old_b, now_b, "B registration stays until B is confirmed");
    s.core.play().unwrap().script_stop("alice");
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn reload_warn_survives_focus_only() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path_a = write_bot(&dir, "keep-a.ts", BOT_TS);
    let path_b = write_bot(&dir, "keep-b.ts", BOT_TS);
    s.load_js(&path_a);
    s.script_start_selected();
    settle(&mut s);
    focus_profile(&mut s, "bob");
    s.load_js(&path_b);
    fs::write(&path_a, format!("{BOT_TS}// changed\n")).unwrap();
    focus_profile(&mut s, "alice");
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    let warned = s.scripts.reload_warning().unwrap().lookup.clone();
    focus_profile(&mut s, "bob");
    assert!(
        s.scripts.reload_warning().is_some(),
        "focus alone must not cancel a bound warning"
    );
    assert_eq!(s.scripts.reload_warning().unwrap().lookup, warned);
    focus_profile(&mut s, "alice");
    let out = reload(&mut s);
    settle(&mut s);
    match out {
        ReloadOutcome::Applied { restarted, .. } => assert_eq!(restarted, 1),
        other => panic!("focus-only must leave A's confirm valid, got {other:?}"),
    }
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn catalog_confirm_gates_newly_paused() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-pause");
    fake_catalog(&root, &[("PauseBot", BOT_TS)]);
    s.scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "PauseBot")
        .unwrap();
    s.persist_successful_assignment(
        "alice",
        ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "PauseBot".into(),
            display_name: "PauseBot".into(),
            unavailable: None,
        },
    );
    s.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "PauseBot".into(),
    ));
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let old_js = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "PauseBot")
        .unwrap()
        .js
        .clone();
    fs::write(
        root.join("src/bot/scripts/PauseBot/PauseBot.ts"),
        format!("{BOT_TS}// changed\n"),
    )
    .unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_pause("alice");
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Paused,
        "newly paused catalog bot needs its own warning"
    );
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("paused during prepare")
            || s.scripts
                .reload_warning()
                .is_some_and(|w| !w.paused_during_prep.is_empty()),
        "{:?} {:?}",
        s.error,
        s.scripts.reload_warning().map(|w| &w.paused_during_prep)
    );
    let now_js = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "PauseBot")
        .unwrap()
        .js
        .clone();
    assert_eq!(old_js, now_js);
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_toplevel_throw_fails_before_replacement() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "throw.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let old = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .clone();
    fs::write(
        &path,
        "throw new Error('prep boom');\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();
    let out = reload(&mut s);
    settle(&mut s);
    match out {
        ReloadOutcome::Failed(e) => assert!(
            e.contains("prep boom") || e.contains("load:") || e.contains("prepare"),
            "{e}"
        ),
        other => panic!("top-level throw must fail before replacement, got {other:?}"),
    }
    let now = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap();
    assert_eq!(now.origin, old.origin);
    assert_eq!(now.js, old.js);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_missing_named_export_fails_before_replacement() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let sib = write_bot(&dir, "helper.ts", "export function helper() {}\n");
    let src = "import { missingFn } from './helper.js';\nexport default class T extends LoopingBot { override loop() { missingFn(); } }\n";
    let good = src.replace("missingFn", "helper");
    let path = write_bot(&dir, "named.ts", &good);
    let _ = sib;
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let old = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap()
        .clone();
    fs::write(&path, src).unwrap();
    let out = reload(&mut s);
    settle(&mut s);
    match out {
        ReloadOutcome::Failed(e) => assert!(
            e.contains("missingFn") || e.contains("load:") || e.contains("prepare"),
            "{e}"
        ),
        other => panic!("missing named export must fail before replacement, got {other:?}"),
    }
    let now = s
        .scripts
        .js
        .get(script::ScriptSource::File, &path.to_string_lossy())
        .unwrap();
    assert_eq!(now.origin, old.origin);
    assert_eq!(now.js, old.js);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn catalog_disk_change_after_warn_does_not_start_stale_prepared() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-stale");
    fake_catalog(&root, &[("StaleBot", BOT_TS)]);
    s.scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "StaleBot")
        .unwrap();
    s.persist_successful_assignment(
        "alice",
        ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "StaleBot".into(),
            display_name: "StaleBot".into(),
            unavailable: None,
        },
    );
    s.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "StaleBot".into(),
    ));
    s.script_start_selected();
    settle(&mut s);
    let old_js = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "StaleBot")
        .unwrap()
        .js
        .clone();
    let bot = root.join("src/bot/scripts/StaleBot/StaleBot.ts");
    fs::write(&bot, format!("{BOT_TS}// first\n")).unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    fs::write(&bot, format!("{BOT_TS}// second\n")).unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running,
        "content change after warn must not authorize the previous prepared set"
    );
    let now = s
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "StaleBot")
        .unwrap();
    assert_eq!(now.js, old_js);
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_removal_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.core.play_mut().unwrap().stop_slot("bob");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert!(
        !s.error.as_deref().unwrap_or("").contains("bob"),
        "removal is cancellation, not a start failure: {:?}",
        s.error
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_reports_true_startup_failure_without_aborting_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.scripts.fail_reload_start_for = Some("bob".into());
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 1);
    assert!(
        s.error.as_deref().unwrap_or("").contains("bob"),
        "true post-stop start failure must stay visible: {:?}",
        s.error
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    // A failed replacement start leaves the stopped target stopped.
    wait_state(&s, "bob", script::RunState::Idle);
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_native_stop_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.core.play().unwrap().script_stop("bob");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    wait_state(&s, "bob", script::RunState::Idle);
    assert!(s
        .core
        .play()
        .unwrap()
        .script_source_identity("bob")
        .is_none());
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_session_stop_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    focus_profile(&mut s, "bob");
    s.script_stop();
    focus_profile(&mut s, "alice");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    wait_state(&s, "bob", script::RunState::Idle);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

#[test]
fn reload_stop_all_clears_pending_without_restart() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.script_stop_all();
    assert!(s.scripts.pending_reload().is_none());
    wait_state(&s, "alice", script::RunState::Idle);
    wait_state(&s, "bob", script::RunState::Idle);
}

#[test]
fn stop_all_during_a_reload_reap_drops_the_queued_replacement() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "reap.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    let card = s.scripts.js.load(&path).unwrap();
    // Reload shape: the old isolate is reaping and its replacement Start is
    // queued behind the reap (no observe has run in between).
    s.core.stop_script("alice");
    s.core
        .start_script(
            "alice",
            frontend_core::ScriptStart::Load {
                js: card.js.clone(),
                shape: card.shape,
                bag: None,
                siblings: Vec::new(),
            },
            Some(card.identity_key()),
        )
        .unwrap();
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Stopping
    );

    s.script_stop_all();

    assert_eq!(s.scripts.last_bulk_report(), Some("Stop all: stopped 1"));
    wait_state(&s, "alice", script::RunState::Idle);
    for _ in 0..20 {
        s.pump_status();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Idle,
        "the queued replacement must not start after Stop all"
    );
}

#[test]
fn reload_logout_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    let bob_gen = s.core.play().unwrap().script_runtime_generation("bob");
    s.logout("bob");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert_eq!(
        s.core.play().unwrap().script_state("bob"),
        script::RunState::Running,
        "logout skips replacement; it does not stop the isolate"
    );
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("bob"),
        bob_gen
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn reload_logout_all_skips_replacement() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    let alice_gen = s.core.play().unwrap().script_runtime_generation("alice");
    let bob_gen = s.core.play().unwrap().script_runtime_generation("bob");
    s.logout_all();
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 0, 0);
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("alice"),
        alice_gen
    );
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("bob"),
        bob_gen
    );
    s.core.play().unwrap().script_stop("alice");
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn reload_reassignment_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    let other = write_bot(&dir, "other.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    let bob_gen = s.core.play().unwrap().script_runtime_generation("bob");
    s.persist_successful_assignment(
        "bob",
        ScriptAssignment {
            source_kind: "file".into(),
            identity: other.to_string_lossy().into_owned(),
            display_name: "other".into(),
            unavailable: None,
        },
    );
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert_eq!(
        s.core.play().unwrap().script_state("bob"),
        script::RunState::Running,
        "reassignment wins; do not stop the old isolate"
    );
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("bob"),
        bob_gen
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn reload_new_start_after_stop_is_not_consumed() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.core.play().unwrap().script_stop("bob");
    // The panel's Start is disabled while the reap runs; Start again
    // once the slot is Idle, as the operator would.
    wait_state(&s, "bob", script::RunState::Idle);
    start_file_on(&mut s, "bob", &path);
    assert_eq!(
        s.core.play().unwrap().script_state("bob"),
        script::RunState::Running
    );
    let bob_gen = s.core.play().unwrap().script_runtime_generation("bob");
    focus_profile(&mut s, "alice");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert_eq!(
        s.core.play().unwrap().script_runtime_generation("bob"),
        bob_gen,
        "a new Start after cancellation is a new generation"
    );
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
    s.core.play().unwrap().script_stop("bob");
}

#[test]
fn reload_of_a_running_script_restarts_it() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "restart.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    let play = s.core.play().unwrap();
    let identity = play.script_source_identity("alice").unwrap();
    let before = play.script_runtime_generation("alice").unwrap();
    fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    assert_applied(reload(&mut s), 1, 0);
    // The old isolate is still reaping; the restart is queued behind it.
    settle(&mut s);
    let play = s.core.play().unwrap();
    assert_eq!(play.script_state("alice"), script::RunState::Running);
    assert!(play.script_runtime_generation("alice").unwrap() > before);
    assert_eq!(
        play.script_source_identity("alice").as_deref(),
        Some(identity.as_str())
    );
    assert_eq!(s.error, None, "{:?}", s.error);
    assert_eq!(s.profile_assignment("alice").unwrap().key(), identity);
    play.script_stop("alice");
}

#[test]
fn start_all_lists_a_member_whose_setup_fails_after_the_click() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let good_path = write_bot(&dir, "good.ts", BOT_TS);
    let bad_path = write_bot(
        &dir,
        "bad.ts",
        "throw new Error('bulk-load-proof');\nexport function tick(api) {}\n",
    );
    let good = s.scripts.js.load(&good_path).unwrap();
    let bad = s.scripts.js.load(&bad_path).unwrap();
    s.persist_successful_assignment("alice", good.assignment());
    s.persist_successful_assignment("bob", bad.assignment());
    s.script_start_all();
    assert_eq!(s.error.as_deref(), Some("Start all: started 2, skipped 0"));
    settle(&mut s);
    let report = s.error.clone().unwrap_or_default();
    assert!(
        report.starts_with("Start all: started 1, skipped 0, failed 1: bob: ")
            && report.contains("bulk-load-proof"),
        "{report}"
    );
    let play = s.core.play().unwrap();
    assert_eq!(play.script_state("alice"), script::RunState::Running);
    assert_eq!(play.script_state("bob"), script::RunState::Idle);
    let failure = s
        .scripts
        .js
        .load_failure(&bad.identity_key())
        .expect("recorded");
    assert_eq!(failure.stage, script::LoadStage::RuntimeLoad);
    play.script_stop("alice");
}

#[test]
fn catalog_native_stop_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let root = dir.join("catalog-stop");
    fake_catalog(&root, &[("StopBot", BOT_TS)]);
    s.scripts
        .js
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .js
        .ensure_js(script::ScriptSource::Catalog, "StopBot")
        .unwrap();
    let asg = ScriptAssignment {
        source_kind: "catalog".into(),
        identity: "StopBot".into(),
        display_name: "StopBot".into(),
        unavailable: None,
    };
    s.persist_successful_assignment("alice", asg.clone());
    s.persist_successful_assignment("bob", asg);
    s.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "StopBot".into(),
    ));
    focus_profile(&mut s, "alice");
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    focus_profile(&mut s, "bob");
    s.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "StopBot".into(),
    ));
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    fs::write(
        root.join("src/bot/scripts/StopBot/StopBot.ts"),
        format!("{BOT_TS}// changed\n"),
    )
    .unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("bob");
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    wait_state(&s, "bob", script::RunState::Idle);
    assert_eq!(
        s.core.play().unwrap().script_state("alice"),
        script::RunState::Running
    );
    s.core.play().unwrap().script_stop("alice");
}

const THIEVER_TS: &str = "export const SETTINGS = { target: { type: 'string', default: 'Man' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

/// Apply to all from the Script prefs editor: the focused profile's bag
/// for its card reaches the other same-card member (saved and pushed to
/// its run), a member on another card is skipped and untouched.
#[test]
fn apply_to_all_reaches_same_card_members_only() {
    let (mut s, dir) = session_with_play(&["alice", "bob", "carol"]);
    let thiever = write_bot(&dir, "thiever.ts", THIEVER_TS);
    let miner = write_bot(&dir, "miner.ts", BOT_TS);
    s.load_js(&thiever);
    s.load_js(&miner);
    start_file_on(&mut s, "bob", &thiever);
    start_file_on(&mut s, "carol", &miner);
    wait_state(&s, "bob", script::RunState::Running);
    let card = s
        .scripts
        .js
        .get(script::ScriptSource::File, &thiever.to_string_lossy())
        .unwrap()
        .clone();
    s.persist_successful_assignment("alice", card.assignment());
    // Loading the files left alice browsing the miner; she edits the thiever.
    s.set_pending_browse(
        "alice",
        script::ScriptSel::Loaded(card.source, card.identity_id()),
    );
    focus_profile(&mut s, "alice");
    assert!(s.set_profile_setting(
        "alice",
        card.source,
        &card.name,
        &card.path,
        "target",
        Value::String("Guard".into()),
    ));

    s.prepare_settings_sync();
    let scope = s.scripts.prepared_settings_sync().unwrap();
    assert_eq!(scope.targets, ["bob"]);
    assert_eq!(scope.skipped.len(), 1);
    s.apply_settings_sync();
    s.core.flush_writes();
    s.poll_scripts();

    let key = card.identity_key();
    let saved = |s: &Session, name: &str| {
        s.core
            .vault()
            .unwrap()
            .get(name)
            .unwrap()
            .settings
            .script_settings
            .get(&key)
            .and_then(|bag| bag.get("target").cloned())
    };
    assert_eq!(saved(&s, "bob"), Some(Value::String("Guard".into())));
    assert_eq!(saved(&s, "carol"), None);
    let report = s.scripts.last_settings_sync().unwrap();
    assert_eq!((report.saved, report.delivered), (1, 1));
    assert_eq!(s.error.as_deref(), Some(report.summary()));
    s.core.play().unwrap().script_stop("bob");
    s.core.play().unwrap().script_stop("carol");
}

/// A parameter edit whose save fails never reaches the running script:
/// the run is posted only once the write is durable.
#[test]
fn a_failed_parameter_save_is_not_pushed_to_the_run() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "live.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    wait_state(&s, "alice", script::RunState::Running);
    s.core.flush_writes();
    // The writer's temp file cannot be created where a directory sits.
    let blocker = dir.join("v.vault").with_extension("tmp");
    fs::create_dir_all(&blocker).unwrap();
    s.set_profile_setting(
        "alice",
        script::ScriptSource::File,
        "live",
        &path,
        "n",
        Value::String("2".into()),
    );
    s.core.flush_writes();
    fs::remove_dir_all(&blocker).unwrap();
    assert_eq!(s.core.take_write_failures().len(), 1, "the save failed");
    let play = s.core.play().unwrap();
    let identity = play.script_source_identity("alice").unwrap();
    let generation = play.script_runtime_generation("alice").unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("n".into(), Value::String("2".into()));
    assert!(
        play.script_post_settings_fenced("alice", &bag, &identity, generation),
        "the unsaved value was already posted to the run"
    );
    play.script_stop("alice");
}
