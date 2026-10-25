use super::*;
use crate::test_support::TestDir;
use frontend_core::scripts::{CardSource, CompiledCardId, ScriptChoice, ScriptRunState};
#[cfg(feature = "js")]
use frontend_core::scripts::{LoadStage, ReloadOutcome};
use frontend_core::test_support::scripts::{Assignment, TestSchema};
#[cfg(feature = "js")]
use frontend_core::test_support::scripts::{
    compiled_identity_key, nothing_changed_reload_message, raw_content_fingerprint,
    session_script_bag, TestLibrary,
};
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use frontend_core::FleetAction;
use std::fs;
use std::time::{Duration, Instant};

/// The core's Everyone script rows, which no panel button submits since the
/// fleet toolbar replaced the rail's Start all / Stop all: these tests drive
/// the rows' own semantics (pacing, queued replacements, reports).
trait EveryoneScripts {
    fn script_start_all(&mut self);
    #[cfg(feature = "js")]
    fn script_stop_all(&mut self);
}

impl EveryoneScripts for Session {
    fn script_start_all(&mut self) {
        #[cfg(feature = "js")]
        {
            let root = self.start_catalog_root();
            self.scripts.stage_start_catalog_root(root);
        }
        self.submit_script_command(OperatorCommand::StartScript, Targets::Everyone);
        self.apply_script_notice();
    }

    #[cfg(feature = "js")]
    fn script_stop_all(&mut self) {
        self.submit_script_command(OperatorCommand::StopScript, Targets::Everyone);
        self.apply_script_notice();
    }
}

/// Pin `choice` as the fleet card, as the Fleet window's drop-down does:
/// stage it and submit `SetFleetCard`, nothing else.
fn pin_fleet_card(s: &mut Session, choice: ScriptChoice) {
    s.fleet_model
        .stage_card(frontend_core::FleetCard::pinned(choice));
    s.submit(OperatorCommand::SetFleetCard, Targets::Application);
}

/// Plan `action` as a frame does: the session sync refreshes the fleet
/// model first, then the button submits.
fn plan(s: &mut Session, action: FleetAction) {
    s.pump_status();
    s.plan_fleet_action(action);
}

/// Pump the session until the latest fleet result has settled.
fn settle_fleet_result(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !s.core.fleet_result().is_some_and(|result| result.settled()) {
        assert!(Instant::now() < deadline, "the fleet result did not settle");
        s.pump_status();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pump catalogue, worker validation and Start receipts until all script
/// lifecycle work has settled.
#[cfg(feature = "js")]
fn settle(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while (s.reload_validation_pending()
        || s.scripts.catalog_work_pending()
        || s.scripts.starts_pending())
        && Instant::now() < deadline
    {
        s.core.poll_host();
        s.poll_scripts();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !s.reload_validation_pending(),
        "script validation did not settle"
    );
    assert!(
        !s.scripts.catalog_work_pending(),
        "catalogue did not settle"
    );
    assert!(!s.scripts.starts_pending(), "script Start did not settle");
}

#[cfg(feature = "js")]
fn reload(s: &mut Session) -> ReloadOutcome {
    s.begin_script_reload_clicked();
    settle(s);
    s.take_reload_outcome()
        .expect("reload operation must publish an outcome")
}

#[cfg(feature = "js")]
fn refresh_catalog(s: &mut Session, root: &std::path::Path) -> ReloadOutcome {
    s.begin_refresh_catalog_at(root);
    settle(s);
    s.take_reload_outcome()
        .expect("catalog refresh must publish an outcome")
}

/// Pump `name`'s lifecycle until it reaches `want` (Stop returns
/// before the reap).
fn wait_state(s: &Session, name: &str, want: ScriptRunState) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while s.core.script_state(name) != want && Instant::now() < deadline {
        s.core.play_for_test().unwrap().pump_script_lifecycle(name);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(s.core.script_state(name), want, "{name}");
}

/// Select a script for one profile through the shared Browse and selection rows.
fn select_for_profile(
    s: &mut Session,
    profile: &str,
    selection: ScriptChoice,
) -> Result<(), String> {
    s.scripts.stage_browse_pick(selection);
    let picked = s
        .submit_script_command(OperatorCommand::PickBrowseCard, Targets::Application)
        .ok_or_else(|| {
            s.error
                .clone()
                .unwrap_or_else(|| "Browse selection was not submitted".into())
        })?;
    if let Some(error) = s.core.command_failure(picked.request()) {
        return Err(error.to_owned());
    }
    let selected = s
        .submit_script_command(OperatorCommand::SelectScriptCard, Targets::Profile(profile))
        .ok_or_else(|| {
            s.error
                .clone()
                .unwrap_or_else(|| "profile selection was not submitted".into())
        })?;
    if let Some(error) = s.core.command_failure(selected.request()) {
        return Err(error.to_owned());
    }
    Ok(())
}

/// Operator Start of `sel` on `profile` through the shared command rows.
#[cfg(feature = "js")]
fn start_sel(s: &mut Session, profile: &str, selection: ScriptChoice) -> Result<(), String> {
    select_for_profile(s, profile, selection)?;
    s.script_start_profile(profile)
}

#[cfg(feature = "js")]
fn prepare_transpile(s: &mut Session, source: CardSource, lookup: &str) {
    s.scripts.stage_transpile_target(source, lookup);
    let submitted = s
        .submit_script_command(OperatorCommand::PrepareScriptCard, Targets::Application)
        .expect("script preparation command is submitted");
    assert!(
        s.core.command_failure(submitted.request()).is_none(),
        "script preparation was refused: {:?}",
        s.error
    );
}

#[cfg(feature = "js")]
fn run_generations(s: &Session, names: &[&str]) -> Vec<(String, u64)> {
    let play = s.core.play_for_test().unwrap();
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
    IsolatedStores::ensure_thread();
    let dir = tmp("vault");
    let path = dir.join("v.vault");
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    for (i, name) in names.iter().enumerate() {
        vault.put(FixtureProfile::new(name, "pw").uid(1 + i as i32));
    }
    let mut s = Session::new();
    install_vault(&mut s.core, vault);
    s.persist_ui = false;
    (s, dir)
}

#[test]
fn focus_restores_assignment_not_other_profile_pending() {
    let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
    s.persist_successful_assignment("bob", Assignment::catalog("Alcher"));
    s.core.flush_writes();
    s.pump_status();
    select_for_profile(
        &mut s,
        "alice",
        ScriptChoice::loaded(CardSource::Catalog, "ChickenKiller"),
    )
    .unwrap();
    s.restore_script_heading("alice");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "ChickenKiller"))
    );
    s.restore_script_heading("bob");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "Alcher"))
    );
    s.set_focus_for_test("bob");
    assert!(!s.heading_is_pending());
    s.set_focus_for_test("alice");
    s.restore_script_heading("alice");
    assert!(s.heading_is_pending());
}

#[test]
#[cfg(feature = "js")]
fn two_profiles_keep_isolated_settings_after_legacy_claim() {
    let (mut s, dir) = session_with_profiles(&["alice", "bob"]);
    let store = dir.join("script-settings.json");
    fs::write(&store, r#"{"catalog:SmithingBot":{"bar":"Adamant"}}"#).unwrap();
    s.scripts.set_legacy_store(store);
    let root = dir.join("catalog");
    fake_catalog(&root, &[("SmithingBot", BOT_TS)]);
    s.import_rs2b0t_catalog(&root).unwrap();
    settle(&mut s);
    let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
    assert_eq!(a.get("bar"), Some(&Value::String("Adamantite".into())));
    s.set_profile_setting(
        "alice",
        CardSource::Catalog,
        "SmithingBot",
        Path::new(""),
        "bar",
        Value::String("Iron".into()),
    );
    s.core.flush_writes();
    s.pump_status();
    let b = s.profile_overrides("bob", "catalog:SmithingBot", "SmithingBot");
    assert_eq!(b.get("bar"), Some(&Value::String("Adamantite".into())));
    s.core.flush_writes();
    s.pump_status();
    let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
    assert_eq!(a.get("bar"), Some(&Value::String("Iron".into())));
}

#[test]
fn missing_file_assignment_is_kept_not_substituted() {
    let (mut s, _dir) = session_with_profiles(&["alice"]);
    s.persist_successful_assignment(
        "alice",
        Assignment::missing_file(
            Path::new("/tmp/missing-bot.ts"),
            "bot",
            "missing file: /tmp/missing-bot.ts",
        ),
    );
    let asg = s.profile_assignment("alice").unwrap();
    assert_eq!(asg.identity(), "/tmp/missing-bot.ts");
    assert!(asg.unavailable().is_some());
    assert_ne!(asg.display_name(), asg.identity());
    s.restore_script_heading("alice");
    match &s.script_sel {
        Some(sel) => {
            assert_eq!(
                sel.loaded_card(),
                Some((CardSource::File, "/tmp/missing-bot.ts"))
            );
        }
        None => panic!("no heading restored: {:?}", s.script_sel),
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
#[cfg(feature = "js")]
fn same_stem_files_are_distinct_and_hash_whitespace() {
    IsolatedStores::ensure_thread();
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
    let mut lib = TestLibrary::with_cache(dir.join("store.json"), dir.join("cache"));
    let ca = lib.load(&fa).unwrap();
    let cb = lib.load(&fb).unwrap();
    assert_ne!(ca.identity_key(), cb.identity_key());
    assert_eq!(ca.name(), cb.name());
    assert!(!lib
        .raw_source_changed(CardSource::File, &fa.to_string_lossy())
        .unwrap());
    fs::write(&fa, format!("{src} ")).unwrap();
    assert!(lib
        .raw_source_changed(CardSource::File, &fa.to_string_lossy())
        .unwrap());
    assert!(!lib
        .raw_source_changed(CardSource::File, &fb.to_string_lossy())
        .unwrap());
}

#[cfg(feature = "js")]
const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

fn empty_play() -> host_play::Play {
    frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    })
}

fn session_with_play(names: &[&str]) -> (Session, TestDir) {
    let (mut s, dir) = session_with_profiles(names);
    #[cfg(feature = "js")]
    s.scripts
        .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
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

#[cfg(feature = "js")]
fn write_bot(dir: &std::path::Path, file: &str, src: &str) -> std::path::PathBuf {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, src).unwrap();
    path
}

#[cfg(feature = "js")]
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

#[cfg(feature = "js")]
#[test]
fn initial_runtime_load_failure_survives_another_card_start() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let bad_source = "export const apiVersion = 2;\nthrow new Error('initial-load-proof');\nexport function tick(api) {}\n";
    let bad_path = write_bot(&dir, "bad.ts", bad_source);
    s.load_js(&bad_path);
    let bad_identity = s
        .scripts
        .card(CardSource::File, &bad_path.to_string_lossy())
        .unwrap()
        .identity_key()
        .to_owned();
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
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Idle);
    assert!(
        s.profile_assignment("alice").is_none(),
        "a broken Start is never the profile's assignment"
    );

    let good_path = write_bot(&dir, "good.ts", BOT_TS);
    s.scripts.load_card(&good_path).unwrap();
    start_file_on(&mut s, "bob", &good_path);
    let failure = s
        .scripts
        .load_failure(&bad_identity)
        .expect("initial runtime failure must remain inspectable after another card starts");
    assert_eq!(failure.path(), bad_path.as_path());
    assert_eq!(failure.stage(), LoadStage::RuntimeLoad);
    assert_eq!(
        s.scripts.load_failure_fingerprint(&bad_identity),
        Some(raw_content_fingerprint(&bad_path, bad_source))
    );
    assert_eq!(
        s.scripts.load_failure_api_family(&bad_identity).as_deref(),
        Some("v2")
    );
    assert!(s
        .scripts
        .named_failure_output()
        .contains("initial-load-proof"));
    s.core.play_for_test().unwrap().script_stop("bob");
}

/// The unlock's auto-login of the last focused profile with MultiBox off:
/// the bot runs and is a fleet member, so a loaded script starts on it.
/// (Before, the unlock opened its slot without loading it, and Start
/// refused with `no slot: <name>`; found in the F4 rs2b2t live session.)
#[cfg(feature = "js")]
#[test]
fn the_unlock_focus_is_a_fleet_member_and_starts_a_loaded_script() {
    IsolatedStores::ensure_thread();
    let dir = tmp("unlock-focus");
    let vault_path = dir.join("v.vault");
    let mut vault = VaultFixture::create(&vault_path, "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(1).auto_login(true));
    vault.put(FixtureProfile::new("bob", "pw").uid(2));
    drop(vault);
    let mut s = Session::new();
    s.preferences.reload().unwrap();
    s.poll_panel_preferences();
    assert!(s.panel_preferences_ready());
    s.persist_ui = false;
    s.core.set_spawn_workers(false);
    assert!(!s.multibox);
    assert!(s.unlock_at(&vault_path, "test-passphrase-01"));
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    assert!(s
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .wants_login());
    s.scripts
        .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
    let path = write_bot(&dir, "unlock-focus.ts", BOT_TS);
    let card = s.scripts.load_card(&path).unwrap();
    let sel = ScriptChoice::loaded(card.source(), &path.to_string_lossy());
    start_sel(&mut s, "alice", sel).expect("a loaded script starts on the unlock's bot");
    assert_eq!(s.core.members(), ["alice".to_string()]);
}

#[cfg(feature = "js")]
#[test]
fn initial_load_refusal_preserves_failure_and_retry_clears_only_its_identity() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let helper = write_bot(&dir, "gate.ts", "export const fail = true;");
    let source = "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('retry-load-proof');\nexport function tick(api) {}";
    let path = write_bot(&dir, "retry.ts", source);
    let card = s.scripts.load_card(&path).unwrap();
    let sel = ScriptChoice::loaded(card.source(), &path.to_string_lossy());
    // Start returns before V8 setup: the runtime failure settles after
    // it and reaches the operator as the Start error it used to be.
    start_sel(&mut s, "alice", sel.duplicate()).expect("Start is accepted before setup");
    settle(&mut s);
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("retry-load-proof"),
        "{:?}",
        s.error
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Idle);
    let failure = s
        .scripts
        .load_failure(card.identity_key())
        .expect("recorded")
        .copyable_text();
    assert!(s.profile_assignment("alice").is_none());
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("alice"),
        Some(0)
    );
    // A non-member is a profile that is not loaded; the words say so.
    assert_eq!(
        start_sel(&mut s, "missing", sel.duplicate()).unwrap_err(),
        "missing is not loaded"
    );
    assert_eq!(
        s.scripts
            .load_failure(card.identity_key())
            .map(|failure| failure.copyable_text()),
        Some(failure.clone())
    );

    let other_path = write_bot(
        &dir,
        "other.ts",
        &format!("throw new Error('other-load-proof');\n{BOT_TS}"),
    );
    let other = s.scripts.load_card(&other_path).unwrap();
    start_sel(
        &mut s,
        "bob",
        ScriptChoice::loaded(other.source(), &other_path.to_string_lossy()),
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
        .load_failure(other.identity_key())
        .expect("recorded")
        .copyable_text();
    assert_eq!(
        s.scripts
            .load_failure_api_family(other.identity_key())
            .as_deref(),
        Some("v1")
    );
    assert_eq!(
        s.scripts
            .load_failure(other.identity_key())
            .map(|failure| failure.stage()),
        Some(LoadStage::RuntimeLoad)
    );

    // Resolve the changed sibling on explicit Start, without replacing the card
    // or running a throwaway validation isolate that could clear the diagnostic.
    fs::write(&helper, "export const fail = false;").unwrap();
    start_sel(&mut s, "alice", sel.duplicate()).unwrap();
    settle(&mut s);
    assert!(s.scripts.load_failure(card.identity_key()).is_none());
    assert_eq!(
        s.scripts
            .load_failure(other.identity_key())
            .map(|failure| failure.copyable_text()),
        Some(other_failure.clone())
    );
    let play = s.core.play_for_test().unwrap();
    assert_eq!(play.script_runtime_generation("alice"), Some(1));
    assert_eq!(
        play.script_source_identity("alice"),
        Some(card.identity_key().to_owned())
    );
    assert_eq!(
        start_sel(
            &mut s,
            "alice",
            ScriptChoice::loaded(other.source(), &other_path.to_string_lossy())
        )
        .unwrap_err(),
        "script already active: stop it first"
    );
    assert_eq!(
        s.scripts
            .load_failure(other.identity_key())
            .map(|failure| failure.copyable_text()),
        Some(other_failure.clone())
    );
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("alice"),
        Some(1)
    );
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
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
    let schema = TestSchema::empty();
    let _ = s.merged_profile_bag(
        "alice",
        CardSource::Catalog,
        "Alcher",
        Path::new(""),
        schema.schema_ref(),
    );
    let _ = s.profile_overrides("alice", "catalog:Alcher", "Alcher");
    s.core.flush_writes();
    let after_reads = fs::read(&vault_path).unwrap();
    assert_eq!(
        after_claim, after_reads,
        "already-migrated read path must not rewrite the vault"
    );
}

#[cfg(feature = "js")]
#[test]
fn external_ts_load_start_stop_persists_assignment_only_on_success() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "ext.ts", BOT_TS);
    assert!(s.profile_assignment("alice").is_none());
    s.load_js(&path);
    assert_eq!(s.error, None, "{:?}", s.error);
    assert!(s.heading_is_pending());
    assert!(s.profile_assignment("alice").is_none());
    prepare_transpile(&mut s, CardSource::File, &path.to_string_lossy());
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let asg = s
        .profile_assignment("alice")
        .expect("success-only assignment");
    assert!(asg.identity().contains("ext.ts"), "{}", asg.identity());
    assert!(asg.unavailable().is_none());
    s.script_stop();
    wait_state(&s, "alice", ScriptRunState::Idle);
    assert_eq!(
        s.profile_assignment("alice").unwrap().identity(),
        asg.identity(),
        "stop keeps last successful assignment"
    );
}

#[cfg(feature = "js")]
#[test]
fn load_js_selects_without_auto_start_and_same_path_does_not_duplicate() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "once.ts", BOT_TS);
    s.load_js(&path);
    prepare_transpile(&mut s, CardSource::File, &path.to_string_lossy());
    assert_eq!(s.error, None, "{:?}", s.error);
    assert_eq!(
        s.core.script_state("alice"),
        ScriptRunState::Idle,
        "load must not Start"
    );
    let first = s
        .scripts
        .cards()
        .filter(|c| c.source() == CardSource::File)
        .count();
    assert_eq!(first, 1);
    s.load_js(&path);
    let again = s
        .scripts
        .cards()
        .filter(|c| c.source() == CardSource::File)
        .count();
    assert_eq!(again, 1, "same path must replace, not duplicate");
    s.script_start_selected();
    settle(&mut s);
    s.core.play_for_test().unwrap().script_stop("alice");
    assert_eq!(reload(&mut s), ReloadOutcome::NothingChanged);
}

#[test]
#[cfg(feature = "js")]
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
    assert_eq!(s.error.as_deref(), Some(nothing_changed_reload_message()));
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[test]
#[cfg(feature = "js")]
fn ui_reload_validation_keeps_the_control_thread_responsive() {
    let (mut session, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "runaway.ts", BOT_TS);
    session.load_js(&path);
    let old_js = session
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    fs::write(
        &path,
        "while (true) {}\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();

    // Same non-blocking contract as the catalog refresh below: the pending
    // flag is the real transition, and the deadline only fails a true hang.
    let started = Instant::now();
    session.begin_script_reload_clicked();
    assert!(
        started.elapsed() < Duration::from_secs(10),
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
            .card_snapshot(CardSource::File, &path.to_string_lossy())
            .expect("old card remains installed")
            .js(),
        old_js.as_str(),
        "failed worker validation cannot replace the live library card"
    );
}

#[cfg(feature = "js")]
#[test]
fn catalog_validation_keeps_the_control_thread_responsive() {
    let (mut session, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-worker");
    fake_catalog(&root, &[("RunawayBot", BOT_TS)]);
    session
        .scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    session
        .scripts
        .ensure_card(CardSource::Catalog, "RunawayBot")
        .unwrap();
    let old_js = session
        .scripts
        .card_snapshot(CardSource::Catalog, "RunawayBot")
        .expect("loaded card")
        .js()
        .to_owned();
    fs::write(
        root.join("src/bot/scripts/RunawayBot/RunawayBot.ts"),
        "while (true) {}\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();

    // The control thread must not wait for hostile module evaluation: begin
    // returns while validation is still owned by the worker. The pending flag
    // below is the real transition (a blocking begin would return settled),
    // not a wall-clock bound, which flakes under load. The generous deadline
    // only fails a begin that truly hangs on evaluation.
    let started = Instant::now();
    session.begin_refresh_catalog_at(&root);
    assert!(
        started.elapsed() < Duration::from_secs(10),
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
        Some(outcome) if outcome.applied_counts().is_some()
    ));
    assert_eq!(
        session
            .scripts
            .card_snapshot(CardSource::Catalog, "RunawayBot")
            .expect("old card remains installed")
            .js(),
        old_js.as_str(),
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
#[cfg(feature = "js")]
fn external_unchanged_reload_dispatches_validation_once() {
    let (mut session, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "ExampleBot.ts", BOT_TS);
    session.load_js(&path);
    let card = session
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded File card");

    let watch = host_play::external_loader::ExternalWatch::default();
    let now = Instant::now();
    watch.configure("alice", path.clone(), card.sha256().to_owned());
    watch.note_scene(true, 2);
    watch.note_inventory(now, "alice", host_play::external_loader::BONES_COUNT, 0);
    watch.note_prereq_passed();
    watch.note_load(
        1,
        host_play::external_loader::SCRIPT_NAME,
        &path,
        card.identity_key(),
        card.sha256(),
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
    wait_state(&session, "alice", ScriptRunState::Idle);
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

#[cfg(feature = "js")]
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
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
    let first = reload(&mut s);
    assert_eq!(first, ReloadOutcome::NeedsConfirm);
    assert!(
        s.error.as_deref().unwrap_or("").contains("running"),
        "{:?}",
        s.error
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    let now_js = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    assert_eq!(old_js, now_js, "preview must not replace registration");
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[test]
#[cfg(feature = "js")]
fn reload_commit_gates_pause_during_prep() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "pause.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
    s.begin_script_reload_clicked();
    assert!(s.reload_validation_pending());
    s.core.play_for_test().unwrap().script_pause("alice");
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Paused);
    let old_js = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
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
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Paused);
    let now_js = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    assert_eq!(old_js, now_js);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[test]
#[cfg(feature = "js")]
fn prepare_failure_preserves_old_instance() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "keep.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    let old = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card");
    fs::write(&path, "const x = 1;\n").unwrap();
    let out = reload(&mut s);
    settle(&mut s);
    match out {
        out if out.failure().is_some_and(|e| {
            e.contains("shape") || e.contains("unloadable") || e.contains("prepare")
        }) =>
        {
            let e = out.failure().unwrap();
            assert!(
                e.contains("shape") || e.contains("unloadable") || e.contains("prepare"),
                "{e}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let now = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card");
    assert_eq!(now.origin(), old.origin());
    assert_eq!(now.js(), old.js());
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[test]
#[cfg(feature = "js")]
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
        CardSource::File,
        "live",
        &path,
        "n",
        Value::String("2".into()),
    ));
    // The run receives the bag once the write is durable.
    s.core.flush_writes();
    let play = s.core.play_for_test().unwrap();
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
        CardSource::File,
        "live",
        &path,
        "n",
        Value::String("3".into()),
    ));
    s.core.flush_writes();
    bag.insert("n".into(), Value::String("3".into()));
    let play = s.core.play_for_test().unwrap();
    assert!(
        !play.script_post_settings_fenced("alice", &bag, &identity, gen),
        "unchanged bag after live Paused delivery is not reposted"
    );
    play.script_stop("alice");
    assert!(!play.script_post_settings_fenced("alice", &bag, &identity, gen));
}

#[test]
#[cfg(feature = "js")]
fn catalog_prepare_failure_does_not_mutate_or_block_valid() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog");
    fake_catalog(&root, &[("GoodBot", BOT_TS), ("BadBot", BOT_TS)]);
    s.scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "GoodBot")
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "BadBot")
        .unwrap();
    let good_js = s
        .scripts
        .card_snapshot(CardSource::Catalog, "GoodBot")
        .expect("loaded card")
        .js()
        .to_owned();
    let bad = s
        .scripts
        .card_snapshot(CardSource::Catalog, "BadBot")
        .expect("loaded card");
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
        .card_snapshot(CardSource::Catalog, "GoodBot")
        .expect("loaded card");
    assert!(!good.js().is_empty(), "successful prepare must keep js");
    assert_ne!(
        good.js(),
        good_js.as_str(),
        "changed good card commits prepared js"
    );
    let bad_now = s
        .scripts
        .card_snapshot(CardSource::Catalog, "BadBot")
        .expect("loaded card");
    assert_eq!(bad_now.origin(), bad.origin());
    assert_eq!(bad_now.js(), bad.js());
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
        s.scripts.load_failures().any(|f| f.name() == "BadBot"),
        "failed catalog card stays inspectable: {:?}",
        s.scripts.load_failure_count()
    );
}

#[cfg(feature = "js")]
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
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "GoodBot")
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "BadParse")
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "BadImport")
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
    let names: Vec<String> = s
        .scripts
        .load_failures()
        .map(|f| f.name().to_owned())
        .collect();
    assert!(
        names.iter().any(|name| name == "BadParse") && names.iter().any(|name| name == "BadImport"),
        "both failures stay inspectable: {names:?} err={:?} report={:?}",
        s.error,
        s.scripts.catalog_refresh_report()
    );
    assert!(!names.iter().any(|name| name == "GoodBot"));
    assert!(s.scripts.named_failure_output().contains("BadParse"));
    assert!(s
        .scripts
        .catalog_refresh_report()
        .unwrap_or("")
        .contains("BadParse"));
}

#[cfg(feature = "js")]
#[test]
fn marked_gatherer_start_and_focus_edits_keep_running_thievers_assignment() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    s.core
        .play_mut_for_test()
        .unwrap()
        .bind_script_test_data(data);
    let root = dir.join("catalog-assignment");
    fake_catalog(&root, &[("Thiever", THIEVER_TS)]);
    s.scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.persist_successful_assignment("alice", Assignment::catalog("Thiever"));
    s.core.flush_writes();
    focus_profile(&mut s, "alice");
    s.script_start_selected();
    settle(&mut s);
    s.core.flush_writes();
    wait_state(&s, "alice", ScriptRunState::Running);
    let generation = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("alice");

    // Marks persist when focus moves to Bob, but plain Start must skip
    // Alice before it can queue a card or save an assignment there.
    s.fleet_model.mark_all_for_test([
        frontend_core::ProfileIdentity::uid(1),
        frontend_core::ProfileIdentity::uid(2),
    ]);
    focus_profile(&mut s, "bob");
    let card_id = CompiledCardId::lookup("Gatherer").unwrap();
    pin_fleet_card(&mut s, ScriptChoice::compiled(card_id));
    plan(&mut s, FleetAction::Start);
    settle(&mut s);
    s.core.flush_writes();
    wait_state(&s, "bob", ScriptRunState::Running);
    let result = s.core.fleet_result().expect("the Start result");
    assert!(
        result.text().contains("alice (already active)"),
        "{}",
        result.text()
    );
    assert_eq!(
        s.profile_assignment("alice"),
        Some(Assignment::catalog("Thiever"))
    );
    assert_eq!(
        s.profile_assignment("bob"),
        Some(Assignment::compiled(card_id))
    );
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("alice"),
        generation
    );

    assert!(s.set_profile_setting(
        "bob",
        CardSource::Catalog,
        "Gatherer",
        std::path::Path::new(""),
        "radius",
        serde_json::json!(24),
    ));
    focus_profile(&mut s, "alice");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "Thiever"))
    );
    assert!(s.set_profile_setting(
        "alice",
        CardSource::Catalog,
        "Thiever",
        &root.join("src/bot/scripts/Thiever/Thiever.ts"),
        "target",
        serde_json::json!("Guard"),
    ));
    s.core.flush_writes();
    let disk = VaultFixture::unlock(&dir.join("v.vault"), "test-passphrase-01");
    assert_eq!(
        disk.assignment_of("alice"),
        Some(Assignment::catalog("Thiever"))
    );
    assert!(disk
        .profile_bag("alice", &compiled_identity_key(card_id))
        .is_none());
    assert_eq!(
        disk.profile_bag("alice", "catalog:Thiever")
            .and_then(|bag| bag.get("target").cloned()),
        Some(Value::String("Guard".into()))
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.script_stop_all();
}

#[cfg(feature = "js")]
#[test]
fn catalog_refresh_warns_before_stopping_running() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-run");
    fake_catalog(&root, &[("RunBot", BOT_TS)]);
    s.scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "RunBot")
        .unwrap();
    s.persist_successful_assignment("alice", Assignment::catalog("RunBot"));
    s.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "RunBot"));
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
        s.core.script_state("alice"),
        ScriptRunState::Running,
        "catalog refresh must warn before stopping"
    );
    assert!(
        s.scripts.reload_warning().is_some()
            || s.error.as_deref().unwrap_or("").contains("running"),
        "{:?} {:?}",
        s.error,
        s.scripts.reload_warning().map(|w| w.running())
    );
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn start_after_launch_runs_saved_catalog_assignment_before_any_browse() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-launch");
    fake_catalog(&root, &[("RunBot", BOT_TS)]);
    let iso = IsolatedStores::enter("launch-start");
    iso.set_catalog_root(&root);
    s.persist_successful_assignment("alice", Assignment::catalog("RunBot"));
    focus_profile(&mut s, "alice");
    settle(&mut s);
    assert!(
        s.scripts
            .card_snapshot(CardSource::Catalog, "RunBot")
            .is_some(),
        "restoring a saved catalog assignment fills the catalog once"
    );
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

/// The bot's Start runs the card its Script section shows. An earlier Browse
/// pick stays the shared Browse card after the fleet assigns another card;
/// it must not replace the card shown (operator report, 2026-10-23).
#[cfg(feature = "js")]
#[test]
fn start_runs_the_shown_card_after_a_fleet_assign_over_an_older_browse_pick() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let old = s
        .scripts
        .load_card(&write_bot(&dir, "old.ts", BOT_TS))
        .unwrap();
    let new = s
        .scripts
        .load_card(&write_bot(&dir, "new.ts", BOT_TS))
        .unwrap();
    focus_profile(&mut s, "alice");
    select_for_profile(
        &mut s,
        "alice",
        ScriptChoice::loaded(old.source(), old.identity_id()),
    )
    .unwrap();

    let identity = s.core.profile_identity("alice").unwrap();
    s.fleet_model.set_mark_for_test(identity, true);
    pin_fleet_card(
        &mut s,
        ScriptChoice::loaded(new.source(), new.identity_id()),
    );
    plan(&mut s, FleetAction::Assign);
    if s.core.fleet_plan().is_some() {
        s.confirm_fleet_plan();
    }
    s.core.flush_writes();
    settle_fleet_result(&mut s);
    assert_eq!(
        s.profile_assignment("alice").unwrap().key(),
        new.identity_key().to_owned()
    );
    assert_eq!(
        s.scripts
            .heading(&s.core, "alice")
            .map(|card| card.label().to_owned()),
        Some(
            ScriptChoice::loaded(new.source(), new.identity_id())
                .label()
                .to_owned()
        ),
        "the Script section shows the assigned card"
    );

    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    wait_state(&s, "alice", ScriptRunState::Running);
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_source_identity("alice")
            .as_deref(),
        Some(new.identity_key()),
        "Start ran {:?}, not the shown card",
        s.core
            .play_for_test()
            .unwrap()
            .script_source_identity("alice")
    );
    s.core.play_for_test().unwrap().script_stop("alice");
}

fn focus_profile(s: &mut Session, name: &str) {
    s.set_focus_for_test(name);
    s.restore_script_heading(name);
}

#[cfg(feature = "js")]
fn start_file_on(s: &mut Session, profile: &str, path: &std::path::Path) {
    focus_profile(s, profile);
    let sel = ScriptChoice::loaded(CardSource::File, &path.to_string_lossy());
    select_for_profile(s, profile, sel).unwrap();
    s.script_start_selected();
    settle(s);
    assert_eq!(s.error, None, "{profile} start: {:?}", s.error);
}

#[cfg(feature = "js")]
fn warn_shared_reload(s: &mut Session, path: &std::path::Path) {
    fs::write(path, format!("{BOT_TS}// changed\n")).unwrap();
    focus_profile(s, "alice");
    assert_eq!(reload(s), ReloadOutcome::NeedsConfirm);
}

#[cfg(feature = "js")]
fn assert_applied(out: ReloadOutcome, restarted: usize, failed: usize) {
    match out.applied_counts() {
        Some((got_r, _, got_f)) => {
            assert_eq!(got_r, restarted, "restarted");
            assert_eq!(got_f, failed, "failed");
        }
        None => {
            panic!("expected Applied {{ restarted: {restarted}, failed: {failed} }}, got {out:?}")
        }
    }
}

#[cfg(feature = "js")]
#[test]
fn cancel_reload_preserves_running_and_paused_executions() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    s.core.play_for_test().unwrap().script_pause("bob");
    let generations = run_generations(&s, &["alice", "bob"]);
    let old_js = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    warn_shared_reload(&mut s, &path);
    assert!(s.script_reload_confirmation_pending());
    s.cancel_reload();
    assert!(!s.script_reload_confirmation_pending());
    assert_eq!(run_generations(&s, &["alice", "bob"]), generations);
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    assert_eq!(s.core.script_state("bob"), ScriptRunState::Paused);
    assert_eq!(
        s.scripts
            .card_snapshot(CardSource::File, &path.to_string_lossy())
            .expect("loaded card")
            .js(),
        old_js.as_str()
    );
    // A later click must prepare and warn again, never reuse cancelled consent.
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    s.core.play_for_test().unwrap().script_stop("alice");
    s.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
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
    s.core.play_for_test().unwrap().script_pause("bob");
    let old_b = s
        .scripts
        .card_snapshot(CardSource::File, &path_b.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    fs::write(&path_a, format!("{BOT_TS}// a changed\n")).unwrap();
    fs::write(&path_b, format!("{BOT_TS}// b changed\n")).unwrap();
    focus_profile(&mut s, "alice");
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    assert!(
        s.scripts
            .reload_warning()
            .is_some_and(|w| w.running().iter().any(|n| n == "alice")),
        "{:?}",
        s.scripts.reload_warning().map(|w| w.running())
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
        s.core.script_state("bob"),
        ScriptRunState::Paused,
        "B must not be replaced without its own warning"
    );
    let now_b = s
        .scripts
        .card_snapshot(CardSource::File, &path_b.to_string_lossy())
        .expect("loaded card")
        .js()
        .to_owned();
    assert_eq!(old_b, now_b, "B registration stays until B is confirmed");
    s.core.play_for_test().unwrap().script_stop("alice");
    s.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
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
    let warned = s.scripts.reload_warning().unwrap().lookup().to_string();
    focus_profile(&mut s, "bob");
    assert!(
        s.scripts.reload_warning().is_some(),
        "focus alone must not cancel a bound warning"
    );
    assert_eq!(s.scripts.reload_warning().unwrap().lookup(), warned);
    focus_profile(&mut s, "alice");
    let out = reload(&mut s);
    settle(&mut s);
    match out.applied_counts() {
        Some((restarted, _, _)) => assert_eq!(restarted, 1),
        None => panic!("focus-only must leave A's confirm valid, got {out:?}"),
    }
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn catalog_confirm_gates_newly_paused() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-pause");
    fake_catalog(&root, &[("PauseBot", BOT_TS)]);
    s.scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "PauseBot")
        .unwrap();
    s.persist_successful_assignment("alice", Assignment::catalog("PauseBot"));
    s.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "PauseBot"));
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    let old_js = s
        .scripts
        .card_snapshot(CardSource::Catalog, "PauseBot")
        .expect("loaded card")
        .js()
        .to_owned();
    fs::write(
        root.join("src/bot/scripts/PauseBot/PauseBot.ts"),
        format!("{BOT_TS}// changed\n"),
    )
    .unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_pause("alice");
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.script_state("alice"),
        ScriptRunState::Paused,
        "newly paused catalog bot needs its own warning"
    );
    assert!(
        s.error
            .as_deref()
            .unwrap_or("")
            .contains("paused during prepare")
            || s.scripts
                .reload_warning()
                .is_some_and(|w| !w.paused_during_prep().is_empty()),
        "{:?} {:?}",
        s.error,
        s.scripts.reload_warning().map(|w| w.paused_during_prep())
    );
    let now_js = s
        .scripts
        .card_snapshot(CardSource::Catalog, "PauseBot")
        .expect("loaded card")
        .js()
        .to_owned();
    assert_eq!(old_js, now_js);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
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
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card");
    fs::write(
        &path,
        "throw new Error('prep boom');\nexport default class T extends LoopingBot { override loop() {} }\n",
    )
    .unwrap();
    let out = reload(&mut s);
    settle(&mut s);
    match out.failure() {
        Some(e) => assert!(
            e.contains("prep boom") || e.contains("load:") || e.contains("prepare"),
            "{e}"
        ),
        None => panic!("top-level throw must fail before replacement, got {out:?}"),
    }
    let now = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card");
    assert_eq!(now.origin(), old.origin());
    assert_eq!(now.js(), old.js());
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
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
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card");
    fs::write(&path, src).unwrap();
    let out = reload(&mut s);
    settle(&mut s);
    match out.failure() {
        Some(e) => assert!(
            e.contains("missingFn") || e.contains("load:") || e.contains("prepare"),
            "{e}"
        ),
        None => panic!("missing named export must fail before replacement, got {out:?}"),
    }
    let now = s
        .scripts
        .card_snapshot(CardSource::File, &path.to_string_lossy())
        .expect("loaded card");
    assert_eq!(now.origin(), old.origin());
    assert_eq!(now.js(), old.js());
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn catalog_disk_change_after_warn_does_not_start_stale_prepared() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let root = dir.join("catalog-stale");
    fake_catalog(&root, &[("StaleBot", BOT_TS)]);
    s.scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "StaleBot")
        .unwrap();
    s.persist_successful_assignment("alice", Assignment::catalog("StaleBot"));
    s.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "StaleBot"));
    s.script_start_selected();
    settle(&mut s);
    let old_js = s
        .scripts
        .card_snapshot(CardSource::Catalog, "StaleBot")
        .expect("loaded card")
        .js()
        .to_owned();
    let bot = root.join("src/bot/scripts/StaleBot/StaleBot.ts");
    fs::write(&bot, format!("{BOT_TS}// first\n")).unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    fs::write(&bot, format!("{BOT_TS}// second\n")).unwrap();
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    assert_eq!(
        s.core.script_state("alice"),
        ScriptRunState::Running,
        "content change after warn must not authorize the previous prepared set"
    );
    let now = s
        .scripts
        .card_snapshot(CardSource::Catalog, "StaleBot")
        .expect("loaded card");
    assert_eq!(now.js(), old_js.as_str());
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn reload_removal_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.core.play_mut_for_test().unwrap().stop_slot("bob");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert!(
        !s.error.as_deref().unwrap_or("").contains("bob"),
        "removal is cancellation, not a start failure: {:?}",
        s.error
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn reload_reports_true_startup_failure_without_aborting_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.scripts.set_fail_reload_start_for(Some("bob".into()));
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 1);
    assert!(
        s.error.as_deref().unwrap_or("").contains("bob"),
        "true post-stop start failure must stay visible: {:?}",
        s.error
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    // A failed replacement start leaves the stopped target stopped.
    wait_state(&s, "bob", ScriptRunState::Idle);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn reload_native_stop_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.core.play_for_test().unwrap().script_stop("bob");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    wait_state(&s, "bob", ScriptRunState::Idle);
    assert!(s
        .core
        .play_for_test()
        .unwrap()
        .script_source_identity("bob")
        .is_none());
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
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
    wait_state(&s, "bob", ScriptRunState::Idle);
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
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
    wait_state(&s, "alice", ScriptRunState::Idle);
    wait_state(&s, "bob", ScriptRunState::Idle);
}

#[cfg(feature = "js")]
#[test]
fn stop_all_during_a_reload_reap_drops_the_queued_replacement() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "reap.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    // Assign & restart holds the replacement behind the old isolate's reap.
    pin_fleet_card(
        &mut s,
        ScriptChoice::loaded(CardSource::File, &path.to_string_lossy()),
    );
    let identity = s.core.profile_identity("alice").unwrap();
    s.fleet_model.set_mark_for_test(identity, true);
    plan(&mut s, FleetAction::AssignRestart);
    assert!(
        s.core.fleet_plan().is_some_and(|plan| plan.needs_confirm()),
        "a running script's restart asks first"
    );
    s.confirm_fleet_plan();
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Stopping);

    s.script_stop_all();

    assert_eq!(s.scripts.last_bulk_report(), Some("Stop all: stopped 1"));
    wait_state(&s, "alice", ScriptRunState::Idle);
    for _ in 0..20 {
        s.pump_status();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        s.core.script_state("alice"),
        ScriptRunState::Idle,
        "the queued replacement must not start after Stop all"
    );
}

#[cfg(feature = "js")]
#[test]
fn reload_logout_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    let bob_gen = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("bob");
    s.logout("bob");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert_eq!(
        s.core.script_state("bob"),
        ScriptRunState::Running,
        "logout skips replacement; it does not stop the isolate"
    );
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("bob"),
        bob_gen
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
    s.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
#[test]
fn reload_logout_all_skips_replacement() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    let alice_gen = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("alice");
    let bob_gen = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("bob");
    s.logout_all();
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 0, 0);
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("alice"),
        alice_gen
    );
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("bob"),
        bob_gen
    );
    s.core.play_for_test().unwrap().script_stop("alice");
    s.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
#[test]
fn reload_reassignment_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    let other = write_bot(&dir, "other.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    let bob_gen = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("bob");
    s.persist_successful_assignment("bob", Assignment::file(&other.to_string_lossy(), "other"));
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert_eq!(
        s.core.script_state("bob"),
        ScriptRunState::Running,
        "reassignment wins; do not stop the old isolate"
    );
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("bob"),
        bob_gen
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
    s.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
#[test]
fn reload_new_start_after_stop_is_not_consumed() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let path = write_bot(&dir, "shared.ts", BOT_TS);
    s.load_js(&path);
    start_file_on(&mut s, "alice", &path);
    start_file_on(&mut s, "bob", &path);
    warn_shared_reload(&mut s, &path);
    s.core.play_for_test().unwrap().script_stop("bob");
    // The panel's Start is disabled while the reap runs; Start again
    // once the slot is Idle, as the operator would.
    wait_state(&s, "bob", ScriptRunState::Idle);
    start_file_on(&mut s, "bob", &path);
    assert_eq!(s.core.script_state("bob"), ScriptRunState::Running);
    let bob_gen = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("bob");
    focus_profile(&mut s, "alice");
    let out = reload(&mut s);
    settle(&mut s);
    assert_applied(out, 1, 0);
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("bob"),
        bob_gen,
        "a new Start after cancellation is a new generation"
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
    s.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
#[test]
fn reload_of_a_running_script_restarts_it() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "restart.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    let play = s.core.play_for_test().unwrap();
    let identity = play.script_source_identity("alice").unwrap();
    let before = play.script_runtime_generation("alice").unwrap();
    fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
    assert_eq!(reload(&mut s), ReloadOutcome::NeedsConfirm);
    assert_applied(reload(&mut s), 1, 0);
    // The old isolate is still reaping; the restart is queued behind it.
    settle(&mut s);
    let play = s.core.play_for_test().unwrap();
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    assert!(play.script_runtime_generation("alice").unwrap() > before);
    assert_eq!(
        play.script_source_identity("alice").as_deref(),
        Some(identity.as_str())
    );
    assert_eq!(s.error, None, "{:?}", s.error);
    assert_eq!(s.profile_assignment("alice").unwrap().key(), identity);
    play.script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn start_all_lists_a_member_whose_setup_fails_after_the_click() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let good_path = write_bot(&dir, "good.ts", BOT_TS);
    let bad_path = write_bot(
        &dir,
        "bad.ts",
        "throw new Error('bulk-load-proof');\nexport function tick(api) {}\n",
    );
    let good = s.scripts.load_card(&good_path).unwrap();
    let bad = s.scripts.load_card(&bad_path).unwrap();
    s.persist_successful_assignment("alice", good.assignment());
    s.persist_successful_assignment("bob", bad.assignment());
    s.script_start_all();
    let click = s.error.clone().unwrap_or_default();
    assert_eq!(click, "Start all: started 1, queued 1, skipped 0");
    settle(&mut s);
    let report = s.error.clone().unwrap_or_default();
    assert!(
        report.starts_with("Start all: started 1, skipped 0, failed 1: bob: ")
            && report.contains("bulk-load-proof"),
        "{report}"
    );
    let play = s.core.play_for_test().unwrap();
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    assert_eq!(s.core.script_state("bob"), ScriptRunState::Idle);
    let failure = s
        .scripts
        .load_failure(bad.identity_key())
        .expect("recorded");
    assert_eq!(failure.stage(), LoadStage::RuntimeLoad);
    play.script_stop("alice");
}

/// A marked Start on each bot's own card runs at once and its one result
/// line follows the paced Starts: the marked row whose setup fails after
/// the click is listed there when it lands, and the banner never carries a
/// second copy.
#[cfg(feature = "js")]
#[test]
fn fleet_result_follows_the_paced_starts_of_a_marked_start() {
    let (mut s, dir) = session_with_play(&["alice", "bob", "carol", "dave"]);
    let good_path = write_bot(&dir, "good.ts", BOT_TS);
    let bad_path = write_bot(
        &dir,
        "bad.ts",
        "throw new Error('bulk-load-proof');\nexport function tick(api) {}\n",
    );
    let good = s.scripts.load_card(&good_path).unwrap();
    let bad = s.scripts.load_card(&bad_path).unwrap();
    for name in ["alice", "bob", "dave"] {
        s.persist_successful_assignment(name, good.assignment());
    }
    s.persist_successful_assignment("carol", bad.assignment());
    for name in ["alice", "bob", "carol"] {
        let identity = s.core.profile_identity(name).unwrap();
        s.fleet_model.set_mark_for_test(identity, true);
    }

    plan(&mut s, FleetAction::Start);
    assert!(s.core.fleet_plan().is_none(), "a marked Start runs at once");
    let result = s.core.fleet_result().expect("the Start result");
    assert!(
        !result.settled(),
        "the queued Starts are still pending: {}",
        result.text()
    );
    settle(&mut s);
    s.pump_status();
    let result = s.core.fleet_result().expect("the Start result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(
        (result.counts().done, result.counts().failed),
        (2, 1),
        "{}",
        result.text()
    );
    assert!(
        result.text().contains("carol") && result.text().contains("bulk-load-proof"),
        "{}",
        result.text()
    );
    let play = s.core.play_for_test().unwrap();
    for name in ["alice", "bob"] {
        assert_eq!(s.core.script_state(name), ScriptRunState::Running, "{name}");
        play.script_stop(name);
    }
    assert_eq!(s.core.script_state("carol"), ScriptRunState::Idle);
    assert_eq!(
        s.core.script_state("dave"),
        ScriptRunState::Idle,
        "unmarked"
    );
}

/// Assign & restart from the Fleet window swaps the card on a running marked
/// bot and starts it on an idle one, after a confirmation that names the
/// interrupted bot; the one result follows the paced Starts to their end.
/// Assign on each bot's own card is refused with its reason and changes
/// nothing.
#[cfg(feature = "js")]
#[test]
fn fleet_assign_and_restart_swaps_the_card_and_follows_the_paced_starts() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let old = s
        .scripts
        .load_card(&write_bot(&dir, "old.ts", BOT_TS))
        .unwrap();
    let new = s
        .scripts
        .load_card(&write_bot(&dir, "new.ts", BOT_TS))
        .unwrap();
    s.persist_successful_assignment("alice", old.assignment());
    s.script_start_profile("alice").unwrap();
    settle(&mut s);
    let before = s
        .core
        .play_for_test()
        .unwrap()
        .script_runtime_generation("alice")
        .unwrap();
    for name in ["alice", "bob"] {
        let identity = s.core.profile_identity(name).unwrap();
        s.fleet_model.set_mark_for_test(identity, true);
    }

    plan(&mut s, FleetAction::Assign);
    assert!(
        s.error
            .as_deref()
            .is_some_and(|error| error.contains("pick a card")),
        "{:?}",
        s.error
    );
    assert!(s.core.fleet_plan().is_none());
    assert_eq!(s.profile_assignment("bob"), None);
    s.error = None;

    pin_fleet_card(
        &mut s,
        ScriptChoice::loaded(new.source(), new.identity_id()),
    );
    plan(&mut s, FleetAction::AssignRestart);
    let plan = s.core.fleet_plan().expect("the restart waits for Confirm");
    assert!(plan.needs_confirm());
    assert_eq!(plan.interrupted(), ["alice"]);
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["alice", "bob"]);
    s.confirm_fleet_plan();
    s.core.flush_writes();
    s.pump_status();
    settle(&mut s);
    settle_fleet_result(&mut s);
    let play = s.core.play_for_test().unwrap();
    for name in ["alice", "bob"] {
        assert_eq!(
            s.core.script_state(name),
            ScriptRunState::Running,
            "{name}; result={:?}; error={:?}; assignment={:?}; starts_pending={}; queue={:?}",
            s.core.fleet_result().map(|result| result.text()),
            s.error,
            s.profile_assignment(name),
            s.scripts.starts_pending(),
            s.scripts.start_queue_place(name),
        );
        assert_eq!(
            s.profile_assignment(name).unwrap().key(),
            new.identity_key().to_owned(),
            "{name}"
        );
    }
    assert!(play.script_runtime_generation("alice").unwrap() > before);
    let result = s.core.fleet_result().expect("the restart result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().done, 2, "{}", result.text());
    for name in ["alice", "bob"] {
        play.script_stop(name);
    }
}

#[cfg(feature = "js")]
#[test]
fn catalog_native_stop_skips_target_and_reloads_peer() {
    let (mut s, dir) = session_with_play(&["alice", "bob"]);
    let root = dir.join("catalog-stop");
    fake_catalog(&root, &[("StopBot", BOT_TS)]);
    s.scripts
        .fill_catalog_blocking(&root, &dir.join("rs2b0t-path"))
        .unwrap();
    s.scripts
        .ensure_card(CardSource::Catalog, "StopBot")
        .unwrap();
    s.persist_successful_assignment("alice", Assignment::catalog("StopBot"));
    s.persist_successful_assignment("bob", Assignment::catalog("StopBot"));
    s.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "StopBot"));
    focus_profile(&mut s, "alice");
    s.script_start_selected();
    settle(&mut s);
    assert_eq!(s.error, None, "{:?}", s.error);
    focus_profile(&mut s, "bob");
    s.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "StopBot"));
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
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("bob");
    refresh_catalog(&mut s, &root);
    settle(&mut s);
    wait_state(&s, "bob", ScriptRunState::Idle);
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
const THIEVER_TS: &str = "export const SETTINGS = { target: { type: 'string', default: 'Man' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

/// The Scripts window's Apply to same-card members: the focused profile's
/// bag for its card reaches the other same-card member (saved and pushed to
/// its run) after the plan row's confirmation; a member on another card is
/// skipped and untouched. The prompt and the result are the fleet plan's.
#[cfg(feature = "js")]
#[test]
fn apply_to_all_reaches_same_card_members_only() {
    let (mut s, dir) = session_with_play(&["alice", "bob", "carol"]);
    let thiever = write_bot(&dir, "thiever.ts", THIEVER_TS);
    let miner = write_bot(&dir, "miner.ts", BOT_TS);
    s.load_js(&thiever);
    s.load_js(&miner);
    start_file_on(&mut s, "bob", &thiever);
    start_file_on(&mut s, "carol", &miner);
    wait_state(&s, "bob", ScriptRunState::Running);
    let card = s
        .scripts
        .card_snapshot(CardSource::File, &thiever.to_string_lossy())
        .expect("loaded card");
    s.persist_successful_assignment("alice", card.assignment());
    // Loading the files left alice browsing the miner; she edits the thiever.
    select_for_profile(
        &mut s,
        "alice",
        ScriptChoice::loaded(card.source(), card.identity_id()),
    )
    .unwrap();
    focus_profile(&mut s, "alice");
    assert!(s.set_profile_setting(
        "alice",
        card.source(),
        card.name(),
        card.path(),
        "target",
        Value::String("Guard".into()),
    ));

    s.pump_status();
    s.copy_focused_settings_to_same_card();
    let plan = s.core.fleet_plan().expect("the copy waits for Confirm");
    assert!(plan.needs_confirm());
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["bob"]);
    let scope = s.scripts.prepared_settings_sync().unwrap();
    assert_eq!(scope.targets(), ["bob"]);
    assert_eq!(scope.skipped().len(), 1);
    s.confirm_fleet_plan();
    s.core.flush_writes();
    settle_fleet_result(&mut s);

    let key = card.identity_key().to_owned();
    let saved = |s: &Session, name: &str| {
        session_script_bag(&s.core, name, &key).and_then(|bag| bag.get("target").cloned())
    };
    assert_eq!(saved(&s, "bob"), Some(Value::String("Guard".into())));
    assert_eq!(saved(&s, "carol"), None);
    let report = s.scripts.last_settings_sync().unwrap();
    assert_eq!((report.saved(), report.delivered()), (1, 1));
    let result = s.core.fleet_result().expect("the copy result");
    assert_eq!(result.counts().done, 1, "{}", result.text());
    s.core.play_for_test().unwrap().script_stop("bob");
    s.core.play_for_test().unwrap().script_stop("carol");
}

/// A parameter edit whose save fails never reaches the running script:
/// the run is posted only once the write is durable.
#[cfg(feature = "js")]
#[test]
fn a_failed_parameter_save_is_not_pushed_to_the_run() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let path = write_bot(&dir, "live.ts", BOT_TS);
    s.load_js(&path);
    s.script_start_selected();
    settle(&mut s);
    wait_state(&s, "alice", ScriptRunState::Running);
    s.core.flush_writes();
    // The publish cannot replace a directory: keep the vault aside and put one
    // at its name.
    let vault_file = dir.join("v.vault");
    let aside = vault_file.with_extension("aside");
    fs::rename(&vault_file, &aside).unwrap();
    fs::create_dir(&vault_file).unwrap();
    s.set_profile_setting(
        "alice",
        CardSource::File,
        "live",
        &path,
        "n",
        Value::String("2".into()),
    );
    s.core.flush_writes();
    fs::remove_dir(&vault_file).unwrap();
    fs::rename(&aside, &vault_file).unwrap();
    assert_eq!(s.core.take_write_failures().len(), 1, "the save failed");
    let play = s.core.play_for_test().unwrap();
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

/// A banner set after a Start (here a profile-write failure) survives that
/// Start settling Ready, even when the Start cleared a load failure.
#[cfg(feature = "js")]
#[test]
fn panel_ready_start_keeps_a_newer_banner() {
    let (mut s, dir) = session_with_play(&["alice"]);
    let helper = write_bot(&dir, "gate.ts", "export const fail = true;");
    let path = write_bot(
        &dir,
        "retry.ts",
        "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('panel-first-load');\nexport function tick(api) {}\n",
    );
    let card = s.scripts.load_card(&path).unwrap();
    let sel = ScriptChoice::loaded(card.source(), &path.to_string_lossy());
    start_sel(&mut s, "alice", sel.duplicate()).unwrap();
    settle(&mut s);
    fs::write(&helper, "export const fail = false;").unwrap();

    start_sel(&mut s, "alice", sel).unwrap();
    s.error = Some("script: vault write failed".into());
    settle(&mut s);
    assert!(s.scripts.load_failure(card.identity_key()).is_none());
    assert_eq!(s.error.as_deref(), Some("script: vault write failed"));
    s.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn start_all_paces_admission_through_the_shared_coordinator() {
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut s, dir) = session_with_play(&names);
    let path = write_bot(&dir, "loop.ts", BOT_TS);
    let card = s.scripts.load_card(&path).unwrap();
    for name in names {
        s.persist_successful_assignment(name, card.assignment());
    }
    s.script_start_all();
    let starting = names
        .iter()
        .filter(|name| s.core.script_state(name) == ScriptRunState::Starting)
        .count();
    assert!(
        starting <= frontend_core::scripts::START_ADMIT_PER_FRAME,
        "panel Start all dispatched {starting} in the click frame"
    );
    assert!(starting > 0 || s.scripts.starts_pending());
    settle(&mut s);
    for name in names {
        wait_state(&s, name, ScriptRunState::Running);
        s.core.play_for_test().unwrap().script_stop(name);
    }
}

/// The Fleet window's Copy settings plans the focused bot's parameters for
/// the marked same-card bots only: an unmarked bot on the same card is
/// left out and named as such, and nothing is written before Confirm.
#[cfg(feature = "js")]
#[test]
fn fleet_apply_settings_is_narrowed_to_the_marked_bots() {
    let (mut s, dir) = session_with_play(&["alice", "bob", "dave"]);
    let thiever = write_bot(&dir, "thiever.ts", THIEVER_TS);
    s.load_js(&thiever);
    let card = s
        .scripts
        .card_snapshot(CardSource::File, &thiever.to_string_lossy())
        .expect("loaded card");
    for name in ["alice", "bob", "dave"] {
        s.persist_successful_assignment(name, card.assignment());
    }
    focus_profile(&mut s, "alice");
    s.script_sel = Some(ScriptChoice::loaded(card.source(), card.identity_id()));
    let bob = s.core.profile_identity("bob").unwrap();
    s.fleet_model.set_mark_for_test(bob, true);

    // Newly assigned: alice has no per-card key, only a legacy override.
    s.scripts
        .set_legacy_str(card.source(), card.name(), "target", "Guard");
    s.core.flush_writes();
    let vault_file = dir.join("v.vault");
    let before = fs::read(&vault_file).unwrap();

    pin_fleet_card(
        &mut s,
        ScriptChoice::loaded(card.source(), card.identity_id()),
    );
    plan(&mut s, FleetAction::CopySettings);
    assert!(
        s.core.fleet_plan().is_some_and(|plan| plan.needs_confirm()),
        "a settings copy always asks first"
    );
    s.core.flush_writes();

    let scope = s.scripts.prepared_settings_sync().expect("prepared");
    assert_eq!(scope.targets(), ["bob"]);
    assert!(scope.skipped().is_empty(), "an unmarked bot is not named");
    assert_eq!(scope.unmarked(), Some(1));
    assert_eq!(
        scope.overrides().get("target"),
        Some(&serde_json::json!("Guard"))
    );
    assert!(scope.prompt().contains("1 marked same-card bot(s)"));
    assert!(scope.prompt().contains("1 unmarked bot(s) left unchanged"));
    assert_eq!(
        fs::read(&vault_file).unwrap(),
        before,
        "preparing wrote the vault before Apply"
    );
}

#[test]
fn marked_compiled_start_keeps_assignments_across_focus_changes() {
    let (mut s, _dir) = session_with_play(&["alice", "bob"]);
    let card = CompiledCardId::lookup("Gatherer").expect("compiled Gatherer card");
    for name in ["alice", "bob"] {
        s.persist_successful_assignment(name, Assignment::compiled(card));
    }
    s.core.flush_writes();
    s.script_start_profile("alice").expect("compiled Start");
    wait_state(&s, "alice", ScriptRunState::Running);

    let alice = s.core.profile_identity("alice").unwrap();
    let bob = s.core.profile_identity("bob").unwrap();
    s.fleet_model.set_mark_for_test(alice, true);
    s.fleet_model.set_mark_for_test(bob, true);
    focus_profile(&mut s, "bob");
    pin_fleet_card(&mut s, ScriptChoice::compiled(card));
    plan(&mut s, FleetAction::Start);
    wait_state(&s, "bob", ScriptRunState::Running);
    settle_fleet_result(&mut s);

    let result = s.core.fleet_result().expect("the Start result");
    assert!(result.text().contains("alice (already active)"), "{}", result.text());
    assert_eq!(
        s.profile_assignment("alice"),
        Some(Assignment::compiled(card))
    );
    assert_eq!(
        s.profile_assignment("bob"),
        Some(Assignment::compiled(card))
    );
    assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
    assert_eq!(s.core.script_state("bob"), ScriptRunState::Running);
    for name in ["alice", "bob"] {
        s.core.play_for_test().unwrap().script_stop(name);
    }
}
#[cfg(feature = "js")]
#[test]
fn memory_relog_warns_before_cancelling_a_queued_start() {
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut s, dir) = session_with_play(&names);
    let path = write_bot(&dir, "queued-relog.ts", BOT_TS);
    let card = s.scripts.load_card(&path).unwrap();
    for name in names {
        s.persist_successful_assignment(name, card.assignment());
    }
    s.script_start_all();
    let queued = names
        .iter()
        .find(|name| s.scripts.start_queue_place(name).is_some())
        .expect("Start all leaves work queued")
        .to_string();
    s.set_focus_for_test(&queued);

    assert!(s.focused_memory_relog_warning());
    assert!(
        !s.request_focused_memory_relog(),
        "the first click only arms the warning"
    );
    assert!(s.mem_relog_armed());
    assert!(
        s.scripts.start_queue_place(&queued).is_some(),
        "arming the warning keeps the queued Start"
    );
    assert!(
        !s.core
            .play_for_test()
            .unwrap()
            .arm(&queued)
            .unwrap()
            .wants_logout(),
        "arming the warning does not start the relog"
    );

    assert!(
        s.request_focused_memory_relog(),
        "the second click starts the relog"
    );
    assert!(s.scripts.start_queue_place(&queued).is_none());
    let arm = s.core.play_for_test().unwrap().arm(&queued).unwrap();
    assert!(
        !arm.wants_logout(),
        "an offline slot has no session to log out"
    );
    assert!(arm.wants_login(), "RelogNow arms the next login");
    assert!(!arm.login_latched(), "RelogNow must not latch Logout");
}
