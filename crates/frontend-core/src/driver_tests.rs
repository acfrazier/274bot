//! Headless driver over real backends, composed as the front ends compose
//! it: the process local-state service over a scratch state root (the real
//! preferences and loadout files), a real vault file opened through
//! `UnlockVault`, a real [`host_play::Play`] started by that command
//! (spawns attach arms without worker threads, so no server is contacted)
//! and the real slot mailboxes. Every check reads what an operator would
//! see: the command report, the session's state, the file on disk, and a
//! new session over the same state root.

use api::identity::Settlement;
use std::path::{Path, PathBuf};

use vault::{Profile, ProfileSettings, Vault};

use super::*;
use crate::command::{CommandFailure, Completion, DrawSwitch, Pending, SkipReason, Subject};
use crate::launch::InstancePermit;

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-t33-driver-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A driver over the state root `state`. Opening restores the Quest Paths
/// source into the process-wide registry, so it waits for a test that
/// owns that source.
fn open_driver(state: &Path) -> HeadlessDriver {
    let _source = crate::quester_paths::registry_source_test_lock();
    HeadlessDriver::open(state, InstancePermit::skip())
}

/// A real vault file at `dir/vault` holding `names`.
fn vault_with(dir: &Path, names: &[&str]) -> PathBuf {
    let path = dir.join("vault");
    let mut vault = Vault::create(&path, PASSPHRASE).expect("vault");
    for (uid, name) in names.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*name).to_string(),
                password: "pw".into(),
                uid: uid as i32 + 1,
                settings: ProfileSettings {
                    auto_login: false,
                    ..ProfileSettings::default()
                },
            })
            .expect("profile");
    }
    path
}

const PASSPHRASE: &str = "test-passphrase-01";

/// The play every driver test starts: a direct session to a port nothing
/// listens on (no worker is spawned unless a test loads a member, and the
/// tests keep workers off, so no server is contacted).
fn play() -> PlayConfig {
    PlayConfig::direct("127.0.0.1", 43594, "/tmp", true)
}

/// A driver composed like a front end (local state under `dir/state`),
/// unlocked through `UnlockVault` over a vault holding `names`.
fn driver_in(dir: &Path, names: &[&str]) -> HeadlessDriver {
    let vault = vault_with(dir, names);
    let mut d = open_driver(&dir.join("state"));
    d.session_mut().set_spawn_workers(false);
    let unlock = d
        .unlock(vault, PASSPHRASE, &play())
        .expect("unlock admitted");
    assert_eq!(
        d.report(unlock.request())
            .and_then(|report| report.members().first())
            .and_then(|member| member.terminal())
            .cloned(),
        Some(done()),
        "UnlockVault opens the vault and starts the play"
    );
    d
}

fn driver(test: &str, names: &[&str]) -> HeadlessDriver {
    driver_in(&scratch(test), names)
}

type Terminal = Settlement<Completion, CommandFailure>;

fn terminal(driver: &HeadlessDriver, request: RequestId, name: &str) -> Option<Terminal> {
    let subject = Subject::Profile(driver.session().profile_identity_or_synthetic(name));
    let member = driver.report(request)?.member(subject)?;
    member.terminal().cloned()
}

fn done() -> Terminal {
    Settlement::Completed(Completion::Done)
}

#[test]
fn load_settles_from_its_operation_and_login_on_the_slot_acknowledgement() {
    let mut d = driver("load-login", &["alice"]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load admitted");
    assert_eq!(d.session().members(), &["alice".to_string()]);
    assert_eq!(terminal(&d, load.request(), "alice"), Some(done()));

    let login = d
        .submit(OperatorCommand::Login, Targets::Profile("alice"))
        .expect("login admitted");
    // The member was admitted on alice's slot, posted to its mailbox, and
    // the mailbox's IntentApplied settled it.
    let report = d.report(login.request()).expect("login report");
    let member = report.members().first().expect("one member");
    assert_eq!(
        member.scope(),
        api::identity::Scope::Slot(
            d.session()
                .play()
                .and_then(|play| play.arm("alice"))
                .expect("alice's arm")
                .slot()
        )
    );
    assert_eq!(terminal(&d, login.request(), "alice"), Some(done()));
    assert!(d
        .session()
        .play()
        .expect("play")
        .arm("alice")
        .expect("arm")
        .wants_login());
}

#[test]
fn logout_posts_to_the_member_slot_and_settles_on_its_acknowledgement() {
    let mut d = driver("logout", &["alice", "bob"]);
    d.submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    let names = ["alice".to_string(), "bob".to_string()];
    let logout = d
        .submit(OperatorCommand::Logout, Targets::Profiles(&names))
        .expect("logout admitted");
    for name in &names {
        assert_eq!(terminal(&d, logout.request(), name), Some(done()));
        let arm = d.session().play().and_then(|p| p.arm(name)).expect("arm");
        assert!(arm.wants_logout());
    }
}

#[test]
fn load_all_selects_the_focus_hint_when_nothing_is_selected() {
    let mut d = driver("load-all-hint", &["alice", "bob"]);
    d.session_mut().set_focus_hint(Some("bob"));
    let load = d
        .submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    assert_eq!(load.added(), 2);
    assert_eq!(d.session().selected(), Some("bob"));
}

#[test]
fn select_and_render_policy_settle_at_dispatch() {
    let mut d = driver("select", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    assert_eq!(terminal(&d, select.request(), "alice"), Some(done()));
    assert_eq!(d.session().selected(), Some("alice"));
    let render = d
        .submit(OperatorCommand::SetRenderAll(true), Targets::Application)
        .expect("render all");
    let report = d.report(render.request()).expect("report");
    assert!(report.is_settled());
}

#[test]
fn pause_without_a_running_script_is_skipped_with_its_reason() {
    let mut d = driver("pause", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let pause = d
        .submit(OperatorCommand::PauseScript, Targets::Profile("alice"))
        .expect("pause");
    // The run owner reports the skip on the next poll.
    d.poll();
    assert_eq!(
        terminal(&d, pause.request(), "alice"),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::NoRunningScript
        )))
    );
}

#[test]
fn targets_must_fit_the_row() {
    let mut d = driver("targets", &["alice"]);
    assert_eq!(
        d.submit(OperatorCommand::Select, Targets::Selected)
            .expect_err("nothing selected"),
        SubmitError::NoTarget(OperatorCommand::Select)
    );
    assert_eq!(
        d.submit(
            OperatorCommand::SetRenderAll(true),
            Targets::Profile("alice")
        )
        .expect_err("application row"),
        SubmitError::WrongTargets(OperatorCommand::SetRenderAll(true))
    );
    assert_eq!(
        d.submit(OperatorCommand::SwitchWorld(1), Targets::Profile("alice"))
            .expect_err("no entry point"),
        SubmitError::NoEntryPoint(OperatorCommand::SwitchWorld(1))
    );
}

#[test]
fn a_pending_owner_member_settles_on_a_later_poll() {
    let mut d = driver("owner-poll", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let remove = d
        .submit(OperatorCommand::Remove, Targets::Profile("alice"))
        .expect("remove");
    let subject = Subject::Profile(d.session().profile_identity_or_synthetic("alice"));
    let pending = d
        .report(remove.request())
        .and_then(|report| report.member(subject))
        .and_then(|member| member.pending());
    assert!(matches!(pending, None | Some(Pending::Accepted)));
    for _ in 0..200 {
        d.poll();
        if d.report(remove.request())
            .is_some_and(|report| report.is_settled())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        d.report(remove.request())
            .is_some_and(|report| report.is_settled()),
        "remove settles once the slot retires"
    );
}

/// Poll until `request` settles (writes are durable off the UI thread).
fn settle(d: &mut HeadlessDriver, request: RequestId) {
    for _ in 0..400 {
        d.poll();
        if d.report(request).is_some_and(|report| report.is_settled()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("command {request:?} did not settle");
}

#[test]
fn guardian_and_raster_rows_write_the_profile_and_settle_from_the_writer() {
    let mut d = driver("guardian-raster", &["alice"]);
    d.session_mut()
        .stage_guardian(crate::session::GuardianEdit::fields(false, "magic", false));
    let guardian = d
        .submit(OperatorCommand::SetGuardian, Targets::Profile("alice"))
        .expect("guardian admitted");
    settle(&mut d, guardian.request());
    assert_eq!(terminal(&d, guardian.request(), "alice"), Some(done()));
    d.session_mut()
        .stage_guardian(crate::session::GuardianEdit::policy(
            "River troll",
            crate::GuardianPolicy::Fight,
        ));
    let policy = d
        .submit(OperatorCommand::SetGuardian, Targets::Profile("alice"))
        .expect("policy admitted");
    settle(&mut d, policy.request());
    let raster = d
        .submit(
            OperatorCommand::SetRaster(crate::command::RasterMode::Cpu),
            Targets::Profile("alice"),
        )
        .expect("raster admitted");
    settle(&mut d, raster.request());
    let alice = d.session().profile("alice").expect("alice");
    assert!(!alice.random_events());
    assert_eq!(
        alice.random_event_policy("river troll"),
        crate::GuardianPolicy::Fight
    );
    assert_eq!(alice.raster(), crate::command::RasterMode::Cpu);
}

#[test]
fn an_unknown_profile_write_is_rejected_with_the_owner_text() {
    let mut d = driver("guardian-unknown", &["alice"]);
    d.session_mut()
        .stage_guardian(crate::session::GuardianEdit::fields(true, "strength", true));
    let guardian = d
        .submit(OperatorCommand::SetGuardian, Targets::Profile("ghost"))
        .expect("admitted");
    assert!(matches!(
        terminal(&d, guardian.request(), "ghost"),
        Some(Settlement::Failed(CommandFailure::Rejected(_)))
    ));
    assert_eq!(
        d.session().command_failure(guardian.request()),
        Some("random: no profile ghost")
    );
}

fn terminal_app(d: &HeadlessDriver, request: RequestId) -> Option<Terminal> {
    d.report(request)?
        .member(Subject::Application)?
        .terminal()
        .cloned()
}

/// The application member's terminal once it settles, polling the session
/// as a front end's frames do. A preference write the writer thread still
/// holds when its bounded wait ends keeps the member pending until the
/// write's own terminal (a busy disk delays it; nothing fails for that):
/// this waits for that product signal. [`SETTLE_HANG_GUARD`] only ends a
/// test whose writer hung; it is not the pass condition.
fn settled_app(d: &mut HeadlessDriver, request: RequestId) -> Option<Terminal> {
    let start = std::time::Instant::now();
    loop {
        if let Some(terminal) = terminal_app(d, request) {
            return Some(terminal);
        }
        d.report(request)?;
        assert!(
            start.elapsed() < SETTLE_HANG_GUARD,
            "the command never settled: its writer hung"
        );
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// See [`settled_app`]: far above any disk delay of a loaded host.
const SETTLE_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(600);

fn rejected(rejection: crate::command::Rejection) -> Terminal {
    Settlement::Failed(CommandFailure::Rejected(rejection))
}

/// The preferences document on disk, as `d`'s service wrote it.
fn preferences_file(d: &HeadlessDriver) -> serde_json::Map<String, serde_json::Value> {
    let path = d.session().preferences().expect("bound preferences").path();
    let bytes = std::fs::read(&path).expect("the preferences file exists");
    serde_json::from_slice(&bytes).expect("the preferences file is JSON")
}

/// [`preferences_file`] while a write may still be landing: `None` until
/// the file exists and parses.
fn preferences_file_now(d: &HeadlessDriver) -> Option<serde_json::Value> {
    let path = d.session().preferences().expect("bound preferences").path();
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[test]
fn unlock_with_a_wrong_passphrase_is_refused_and_keeps_the_vault() {
    let dir = scratch("unlock-wrong");
    let vault = vault_with(&dir, &["alice"]);
    let mut d = open_driver(&dir.join("state"));
    let wrong = d
        .unlock(vault.clone(), "not-the-passphrase", &play())
        .expect("admitted");
    assert_eq!(
        d.report(wrong.request())
            .and_then(|report| report.member(Subject::Application))
            .and_then(|member| member.terminal())
            .cloned(),
        Some(rejected(crate::command::Rejection::VaultRefused))
    );
    assert!(d.session().command_failure(wrong.request()).is_some());
    assert!(!d.session().vault_open(), "a refused unlock opens nothing");
    assert!(
        vault.is_file(),
        "a wrong passphrase never deletes the vault"
    );

    let right = d.unlock(vault, PASSPHRASE, &play()).expect("admitted");
    assert_eq!(terminal_app(&d, right.request()), Some(done()));
    assert!(d.session().vault_open());
    assert!(
        d.session().has_play(),
        "the unlock started the play session"
    );
    assert_eq!(d.session().profile_names(), ["alice".to_string()]);
}

#[test]
fn create_vault_makes_a_new_vault_file_that_a_new_session_unlocks() {
    let dir = scratch("create-vault");
    let vault = dir.join("new-vault");
    let mut d = open_driver(&dir.join("state"));
    let create = d
        .unlock(vault.clone(), PASSPHRASE, &play())
        .expect("admitted");
    assert_eq!(
        d.report(create.request()).map(CommandReport::command),
        Some(OperatorCommand::CreateVault)
    );
    assert_eq!(terminal_app(&d, create.request()), Some(done()));
    assert!(vault.is_file(), "CreateVault wrote the vault file");
    drop(d);

    let mut again = open_driver(&dir.join("state"));
    let unlock = again.unlock(vault, PASSPHRASE, &play()).expect("admitted");
    assert_eq!(
        again.report(unlock.request()).map(CommandReport::command),
        Some(OperatorCommand::UnlockVault)
    );
    assert_eq!(terminal_app(&again, unlock.request()), Some(done()));
}

#[test]
fn reset_vault_deletes_the_locked_file_and_refuses_while_unlocked() {
    let dir = scratch("reset-vault");
    let vault = vault_with(&dir, &["alice"]);
    let mut d = open_driver(&dir.join("state"));
    d.session_mut().stage_vault_reset(vault.clone());
    let reset = d
        .submit(OperatorCommand::ResetVault, Targets::Application)
        .expect("admitted");
    assert_eq!(terminal_app(&d, reset.request()), Some(done()));
    assert!(!vault.is_file(), "ResetVault deleted the locked vault file");

    let open_dir = scratch("reset-vault-open");
    let mut open = driver_in(&open_dir, &["bob"]);
    let path = open_dir.join("vault");
    assert!(path.is_file(), "the open vault's file exists");
    open.session_mut().stage_vault_reset(path.clone());
    let refused = open
        .submit(OperatorCommand::ResetVault, Targets::Application)
        .expect("admitted");
    assert_eq!(
        terminal_app(&open, refused.request()),
        Some(rejected(crate::command::Rejection::NotApplicable))
    );
    assert!(path.is_file(), "nothing is deleted while a vault is open");
}

#[test]
fn a_vault_row_with_nothing_staged_is_refused_not_done() {
    let dir = scratch("vault-unstaged");
    let mut d = open_driver(&dir.join("state"));
    let unlock = d
        .submit(OperatorCommand::UnlockVault, Targets::Application)
        .expect("admitted");
    assert_eq!(
        terminal_app(&d, unlock.request()),
        Some(rejected(crate::command::Rejection::NotStaged))
    );
    assert!(!d.session().vault_open());
}

#[test]
fn preference_rows_write_the_file_and_a_new_session_reads_them_back() {
    use crate::nav_prefs::NavPreference;
    let dir = scratch("prefs");
    let mut d = driver_in(&dir, &["alice"]);
    let rows = [
        OperatorCommand::SetNavPreference(NavPreference::AllowTeleports, true),
        OperatorCommand::SetDangerLevel(crate::walk_permissions::DangerLevel::Always),
        OperatorCommand::SetDraw(DrawSwitch::SidecarFullRate, true),
        OperatorCommand::SetDraw(DrawSwitch::FocusedFullRate, false),
        OperatorCommand::SetSessionLog(true),
        OperatorCommand::SetMapBakeChoice(crate::MapBakeChoice::Always),
    ];
    for row in rows {
        let submitted = d.submit(row, Targets::Application).expect("admitted");
        assert_eq!(
            settled_app(&mut d, submitted.request()),
            Some(done()),
            "{row:?}"
        );
    }
    // What the operator sees in this session.
    assert!(d.models().walk_globals().globals().allow_teleports());
    assert!(d.models().draw().sidecar_50());
    assert!(!d.models().draw().focused_50());
    assert!(
        crate::log_file::session_log_path().is_some(),
        "the session file is open"
    );
    // The file on disk.
    let file = preferences_file(&d);
    assert_eq!(
        file["nav"]["allow_teleports"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        file["nav"]["allow_danger_zones"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(file["sidecar_50"], serde_json::Value::Bool(true));
    assert_eq!(file["focused_50"], serde_json::Value::Bool(false));
    assert_eq!(file["session_log_file"], serde_json::Value::Bool(true));
    assert_eq!(file["map_bake"], serde_json::Value::String("always".into()));
    // A new session over the same state root.
    let again = open_driver(&dir.join("state"));
    assert_eq!(
        again
            .session()
            .nav_preference(NavPreference::AllowTeleports),
        Some(true)
    );
    assert!(again.models().walk_globals().globals().allow_teleports());
    assert_eq!(
        again.models().walk_globals().danger_level(),
        crate::walk_permissions::DangerLevel::Always
    );
    assert!(again.models().draw().sidecar_50());
    assert!(!again.models().draw().focused_50());
    assert_eq!(again.session().session_log_preference(), Some(true));
    assert_eq!(
        again.session().map_bake_preference(),
        Some(crate::MapBakeChoice::Always)
    );
    // Turning the file off again is the same command.
    let off = d
        .submit(OperatorCommand::SetSessionLog(false), Targets::Application)
        .expect("admitted");
    assert_eq!(settled_app(&mut d, off.request()), Some(done()));
    assert!(crate::log_file::session_log_path().is_none());
}

/// Hold the next file write of `d`'s local-state service (the busy disk)
/// until the returned sender sends; later writes run at once.
fn hold_first_write(d: &HeadlessDriver) -> std::sync::mpsc::Sender<()> {
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let first = std::sync::atomic::AtomicBool::new(true);
    d.session()
        .local_state()
        .expect("the driver binds the process service")
        .set_write_stall_for_test(std::sync::Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                // The busy disk: the first write waits for the test.
                let _ = gate.lock().unwrap().recv();
            }
        }));
    release
}

/// T3.3 r4 (review finding 1): a preference write the disk holds past the
/// bounded wait leaves its command pending, never `Failed`: the operator
/// is not told a save failed that later lands. It settles `Done` when the
/// write is durable, and the file and a fresh read then hold the value.
/// T3.3 r5 (review finding 6): the driver's lent walk view then holds the
/// value too, as after a write durable at submission.
#[test]
fn a_held_preference_write_stays_pending_and_settles_done_when_it_lands() {
    use crate::nav_prefs::NavPreference;
    let mut d = driver("held-pref", &["alice"]);
    let release = hold_first_write(&d);

    let submitted = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::AllowTeleports, true),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(
        terminal_app(&d, submitted.request()),
        None,
        "a write still held after the bounded wait is pending, not failed"
    );
    d.poll();
    assert_eq!(terminal_app(&d, submitted.request()), None);
    assert_eq!(
        d.session().nav_preference(NavPreference::AllowTeleports),
        Some(false),
        "nothing is durable yet"
    );
    assert!(!d.models().walk_globals().globals().allow_teleports());

    release.send(()).unwrap();
    assert_eq!(settled_app(&mut d, submitted.request()), Some(done()));
    assert!(
        d.models().walk_globals().globals().allow_teleports(),
        "the lent walk view adopts the settled write"
    );
    assert_eq!(
        preferences_file(&d)["nav"]["allow_teleports"],
        serde_json::Value::Bool(true)
    );
    d.session()
        .preferences()
        .expect("bound preferences")
        .reload()
        .unwrap();
    assert_eq!(
        d.session().nav_preference(NavPreference::AllowTeleports),
        Some(true)
    );
}

/// T3.3 r5 (review finding 5): the newest command for a setting wins. An
/// older write the disk held settles after a newer command for the same
/// setting was accepted and made durable; the running session keeps the
/// newer value, as the file does.
#[test]
fn an_older_held_write_settling_late_does_not_undo_a_newer_command() {
    use crate::nav_prefs::NavPreference;
    let mut d = driver("newest-pref", &["alice"]);
    assert!(
        d.session().running_pause_script_on_manual_walk_abort(),
        "the pause is on by default"
    );
    let release = hold_first_write(&d);

    let older = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::PauseScriptOnManualWalkAbort, false),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(terminal_app(&d, older.request()), None, "the write is held");

    // The disk lets the older write land; the session has not polled it.
    release.send(()).unwrap();
    let start = std::time::Instant::now();
    while preferences_file_now(&d).and_then(|file| {
        file.get("nav")?
            .get("pause_script_on_manual_walk_abort")?
            .as_bool()
    }) != Some(false)
    {
        assert!(
            start.elapsed() < SETTLE_HANG_GUARD,
            "the older write never landed: its writer hung"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let newer = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::PauseScriptOnManualWalkAbort, true),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(settled_app(&mut d, newer.request()), Some(done()));
    assert_eq!(settled_app(&mut d, older.request()), Some(done()));
    assert!(
        d.session().running_pause_script_on_manual_walk_abort(),
        "the running session matches the newest command"
    );
    assert_eq!(
        preferences_file(&d)["nav"]["pause_script_on_manual_walk_abort"],
        serde_json::Value::Bool(true)
    );
}

#[test]
fn marks_and_the_fleet_filter_change_only_through_their_rows() {
    let mut d = driver("marks", &["alice", "bob"]);
    let alice = d.session().profile_identity_or_synthetic("alice");
    let bob = d.session().profile_identity_or_synthetic("bob");
    let mark = d
        .submit(OperatorCommand::SetMark(true), Targets::Profile("alice"))
        .expect("mark");
    assert_eq!(terminal(&d, mark.request(), "alice"), Some(done()));
    assert!(d.models().fleet().marks().contains(alice));
    d.session_mut().stage_mark_all([alice, bob]);
    d.submit(OperatorCommand::MarkAll, Targets::Application)
        .expect("mark all");
    assert_eq!(d.models().fleet().marks().len(), 2);
    d.models_mut()
        .fleet_mut()
        .staged_filter_mut()
        .push_str("bob");
    assert_eq!(
        d.models().fleet().filter().text(),
        "",
        "a staged edit is not shown yet"
    );
    let filter = d
        .submit(OperatorCommand::SetFleetFilter, Targets::Application)
        .expect("filter");
    assert_eq!(terminal_app(&d, filter.request()), Some(done()));
    assert_eq!(d.models().fleet().filter().text(), "bob");
    d.submit(OperatorCommand::ClearMarks, Targets::Application)
        .expect("clear");
    assert!(d.models().fleet().marks().is_empty());
}

#[test]
fn a_notice_or_log_save_with_nothing_staged_is_refused_not_done() {
    let mut d = driver("unstaged", &["alice"]);
    for row in [OperatorCommand::AcknowledgeNotice, OperatorCommand::SaveLog] {
        let submitted = d.submit(row, Targets::Application).expect("admitted");
        assert_eq!(
            terminal_app(&d, submitted.request()),
            Some(rejected(crate::command::Rejection::NotStaged)),
            "{row:?}"
        );
    }
}

#[test]
fn save_log_writes_the_staged_text_and_settles_from_the_writer() {
    let dir = scratch("save-log");
    let mut d = driver_in(&dir, &["alice"]);
    let path = dir.join("saved.log");
    d.session_mut()
        .stage_log_save(path.clone(), "line one\nline two\n".into());
    let save = d
        .submit(OperatorCommand::SaveLog, Targets::Application)
        .expect("admitted");
    settle(&mut d, save.request());
    assert_eq!(terminal_app(&d, save.request()), Some(done()));
    assert_eq!(
        std::fs::read_to_string(&path).expect("saved log"),
        "line one\nline two\n"
    );
}

#[test]
fn quit_retires_every_slot_and_closes_the_play() {
    let mut d = driver("quit", &["alice", "bob"]);
    d.submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    let quit = d
        .submit(OperatorCommand::Quit, Targets::Everyone)
        .expect("quit");
    for name in ["alice", "bob"] {
        assert_eq!(terminal(&d, quit.request(), name), Some(done()), "{name}");
    }
    assert!(!d.session().has_play(), "Quit closed the play session");
}

#[test]
fn walk_to_without_a_confirmed_destination_is_refused_and_arms_nothing() {
    let mut d = driver("walk-to", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    // Nothing staged: the row is refused with its typed reason.
    let unstaged = d
        .submit(OperatorCommand::WalkTo, Targets::Profile("alice"))
        .expect("admitted");
    assert!(
        matches!(
            terminal(&d, unstaged.request(), "alice"),
            Some(Settlement::Failed(CommandFailure::Rejected(_)))
        ),
        "{:?}",
        terminal(&d, unstaged.request(), "alice")
    );
    // A staged walk over a map with no picked destination: the map
    // refuses it the same way, and no route is armed.
    let context =
        crate::walk_map::WalkMapContext::browsing(nav::map::identity::Digest([0; 32]), None, 1);
    let mut map = crate::walk_map::WalkMap::new();
    d.models_mut().map_mut().stage_walk_to(
        &mut map,
        context,
        None,
        nav::router::FindOptions::default(),
    );
    let walk = d
        .submit(OperatorCommand::WalkTo, Targets::Profile("alice"))
        .expect("admitted");
    assert!(
        matches!(
            terminal(&d, walk.request(), "alice"),
            Some(Settlement::Failed(CommandFailure::Rejected(_)))
        ),
        "{:?}",
        terminal(&d, walk.request(), "alice")
    );
    assert!(d.session().command_failure(walk.request()).is_some());
    assert!(!d.session().walk_route_armed("alice"));
}

#[test]
fn start_script_with_nothing_assigned_is_refused_with_its_reason() {
    let mut d = driver("start-script", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let start = d
        .submit(OperatorCommand::StartScript, Targets::Profile("alice"))
        .expect("admitted");
    settle(&mut d, start.request());
    let terminal = terminal(&d, start.request(), "alice");
    assert!(
        matches!(
            terminal,
            Some(Settlement::Failed(_) | Settlement::Completed(Completion::Skipped(_)))
        ),
        "a Start with no card never reports Done: {terminal:?}"
    );
    assert!(
        d.session().command_failure(start.request()).is_some()
            || matches!(
                terminal,
                Some(Settlement::Completed(Completion::Skipped(_)))
            ),
        "the operator sees why nothing started"
    );
    assert_eq!(
        d.session().script_state("alice"),
        crate::scripts::ScriptRunState::Idle,
        "no script runs"
    );
}

/// The names of the fleet model's shown rows, in their order.
fn shown_names(d: &HeadlessDriver) -> Vec<String> {
    d.models()
        .fleet()
        .shown_rows()
        .map(|(_, row)| row.name().to_string())
        .collect()
}

/// Every vault profile and every loaded slot is one row, in the saved Load
/// order (seeded in vault order), loaded or not. Loaded is its own flag,
/// not the phase: a loaded bot that is offline is still loaded.
#[test]
fn the_fleet_universe_is_every_profile_once_with_an_explicit_loaded_flag() {
    let mut d = driver("fleet-universe", &["alice", "bob", "carol"]);
    d.submit(OperatorCommand::Load, Targets::Profile("bob"))
        .expect("load bob");
    d.poll();
    let fleet = d.models().fleet();
    let rows: Vec<(&str, bool)> = fleet
        .rows()
        .iter()
        .map(|row| (row.name(), row.loaded()))
        .collect();
    assert_eq!(rows, [("alice", false), ("bob", true), ("carol", false)]);
    let bob = &fleet.rows()[1];
    assert_eq!(bob.phase(), crate::Phase::Offline, "loaded, not logged in");
    assert_eq!(
        fleet.identities(),
        [
            d.session().profile_identity_or_synthetic("alice"),
            d.session().profile_identity_or_synthetic("bob"),
            d.session().profile_identity_or_synthetic("carol"),
        ],
        "one row per identity: the loaded vault profile is not listed twice"
    );
    assert_eq!(fleet.counts().shown_loaded(), 1);
    assert_eq!(fleet.counts().shown_unloaded(), 2);
}

/// An empty vault has no rows; a loaded slot without a vault row (an
/// orphan) is one row; with the vault locked, only loaded slots remain.
#[test]
fn empty_locked_and_orphan_universes_have_one_row_per_identity() {
    let mut d = driver("fleet-empty", &[]);
    d.poll();
    assert!(d.models().fleet().rows().is_empty());

    let mut d = driver("fleet-orphan", &["alice", "bob"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load alice");
    d.poll();
    assert!(d
        .session_mut()
        .vault_mut()
        .expect("vault")
        .remove("alice")
        .expect("remove alice's vault row"));
    d.poll();
    let names: Vec<(&str, bool)> = d
        .models()
        .fleet()
        .rows()
        .iter()
        .map(|row| (row.name(), row.loaded()))
        .collect();
    assert_eq!(
        names,
        [("bob", false), ("alice", true)],
        "the orphan stays one row, after the ordered profiles"
    );
    assert_eq!(
        d.models().fleet().identity(1),
        Some(crate::ProfileIdentity::synthetic("alice"))
    );

    crate::test_support::session::clear_vault(d.session_mut());
    d.poll();
    let names: Vec<&str> = d
        .models()
        .fleet()
        .rows()
        .iter()
        .map(|row| row.name())
        .collect();
    assert_eq!(
        names,
        ["alice"],
        "a locked vault lists the loaded slots only"
    );
}

/// A filter hides rows; it never drops a mark, and the counts say how many
/// marks it hides. The order is shared by every surface that reads the
/// model.
#[test]
fn a_filter_hides_rows_but_keeps_their_marks_and_counts_them() {
    let mut d = driver("fleet-filter-marks", &["alice", "bob", "carol"]);
    d.poll();
    for name in ["alice", "carol"] {
        d.submit(OperatorCommand::SetMark(true), Targets::Profile(name))
            .expect("mark");
    }
    d.models_mut()
        .fleet_mut()
        .staged_filter_mut()
        .push_str("bob");
    d.submit(OperatorCommand::SetFleetFilter, Targets::Application)
        .expect("filter");
    assert_eq!(shown_names(&d), ["bob"]);
    let counts = d.models().fleet().counts();
    assert_eq!(counts.marked(), 2, "filtering drops no mark");
    assert_eq!(counts.shown_marked(), 0);
    assert_eq!(counts.hidden_marked(), 2);

    d.models_mut().fleet_mut().staged_filter_mut().clear();
    d.submit(OperatorCommand::SetFleetFilter, Targets::Application)
        .expect("clear filter");
    d.submit(
        OperatorCommand::SetFleetSort(crate::FleetOrder::Name),
        Targets::Application,
    )
    .expect("sort");
    assert_eq!(shown_names(&d), ["alice", "bob", "carol"]);
    assert_eq!(d.models().fleet().counts().shown_marked(), 2);
}

/// Unloading a marked profile keeps its mark (the row stays, not loaded);
/// deleting the profile drops the row and the mark.
#[test]
fn unloading_keeps_a_mark_and_deleting_the_profile_drops_it() {
    let mut d = driver("fleet-unload-delete", &["alice", "bob"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load alice");
    d.poll();
    let alice = d.session().profile_identity_or_synthetic("alice");
    d.submit(OperatorCommand::SetMark(true), Targets::Profile("alice"))
        .expect("mark");
    let remove = d
        .submit(OperatorCommand::Remove, Targets::Profile("alice"))
        .expect("remove");
    settle(&mut d, remove.request());
    assert!(d.session().members().is_empty(), "alice left the fleet");
    let fleet = d.models().fleet();
    let index = fleet.index_of(alice).expect("alice keeps her row");
    assert!(!fleet.rows()[index].loaded());
    assert!(fleet.marks().contains(alice), "unloading keeps the mark");

    let delete = d
        .submit(OperatorCommand::DeleteProfile, Targets::Profile("alice"))
        .expect("delete");
    settle(&mut d, delete.request());
    let fleet = d.models().fleet();
    assert_eq!(
        fleet.index_of(alice),
        None,
        "the deleted profile has no row"
    );
    assert!(!fleet.marks().contains(alice), "deleting drops the mark");
    assert_eq!(fleet.counts().marked(), 0);
}

/// A durable rename keeps the profile's identity, so its mark, and the
/// universe is rebuilt once for it without any front end asking.
#[test]
fn a_rename_keeps_the_mark_and_rebuilds_the_universe_once() {
    let mut d = driver("fleet-rename", &["alice", "bob"]);
    d.poll();
    let alice = d.session().profile_identity_or_synthetic("alice");
    d.submit(OperatorCommand::SetMark(true), Targets::Profile("alice"))
        .expect("mark");
    let before = d.models().fleet().rows_generation();
    let mut renamed = d
        .session()
        .vault()
        .and_then(|vault| vault.get("alice"))
        .cloned()
        .expect("alice");
    renamed.username = "alicia".into();
    d.session_mut()
        .rename_profile("alice", renamed, crate::ArmMirror::none(), "profile")
        .expect("rename");
    d.session_mut().flush_writes();
    d.poll();
    let fleet = d.models().fleet();
    assert_eq!(
        fleet.rows_generation(),
        before + 1,
        "one rebuild for one rename"
    );
    let index = fleet.index_of(alice).expect("same identity");
    assert_eq!(fleet.rows()[index].name(), "alicia");
    assert!(fleet.marks().contains(alice));
}

/// A steady frame recomputes nothing: refreshing with nothing changed
/// neither rebuilds the universe nor moves the generation.
#[test]
fn a_steady_refresh_rebuilds_and_recounts_nothing() {
    let mut d = driver("fleet-steady", &["alice", "bob"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    d.poll();
    let (rebuilds, generation) = (
        d.models().fleet().rows_generation(),
        d.models().fleet().generation(),
    );
    for _ in 0..10 {
        d.poll();
    }
    assert_eq!(d.models().fleet().rows_generation(), rebuilds);
    assert_eq!(d.models().fleet().generation(), generation);
}

/// A deleted profile's member settles from its vault write. Its name no
/// longer resolves to its identity once the row is staged away, so the
/// owner keeps the identity it had at dispatch.
#[test]
fn delete_profile_settles_done_when_its_write_lands() {
    let mut d = driver("delete-settles", &["alice", "bob"]);
    let alice = d.session().profile_identity_or_synthetic("alice");
    let delete = d
        .submit(OperatorCommand::DeleteProfile, Targets::Profile("alice"))
        .expect("delete");
    settle(&mut d, delete.request());
    let terminal = d
        .report(delete.request())
        .and_then(|report| report.member(Subject::Profile(alice)))
        .and_then(|member| member.terminal())
        .cloned();
    assert_eq!(terminal, Some(done()));
}

// ---- F2: the saved Load order -------------------------------------------

/// The stored Load order (`"<uid>": <key>` members, by key), named
/// through the vault: `None` until the file holds one.
fn stored_order_now(d: &HeadlessDriver) -> Option<Vec<String>> {
    let file = preferences_file_now(d)?;
    let members = file.get(crate::PROFILE_ORDER_KEY)?.as_object()?.clone();
    let vault = d.session().vault()?;
    let mut keyed: Vec<(i64, String)> = members
        .iter()
        .filter_map(|(uid, key)| {
            let uid: i32 = uid.parse().ok()?;
            let name = vault.profiles().find(|p| p.uid == uid)?.username.clone();
            Some((key.as_i64()?, name))
        })
        .collect();
    keyed.sort();
    Some(keyed.into_iter().map(|(_, name)| name).collect())
}

fn stored_order(d: &HeadlessDriver) -> Vec<String> {
    stored_order_now(d).expect("a stored Load order")
}

fn order_names(d: &HeadlessDriver) -> Vec<String> {
    d.session().profile_order().names().to_vec()
}

fn row_names(d: &HeadlessDriver) -> Vec<String> {
    d.models()
        .fleet()
        .rows()
        .iter()
        .map(|row| row.name().to_string())
        .collect()
}

fn id(d: &HeadlessDriver, name: &str) -> crate::ProfileIdentity {
    d.session().profile_identity_or_synthetic(name)
}

/// Stage and submit one reorder; its settled terminal.
fn reorder(d: &mut HeadlessDriver, op: crate::ReorderOp) -> Option<Terminal> {
    d.session_mut().stage_reorder(op);
    let submitted = d
        .submit(OperatorCommand::ReorderProfiles, Targets::Application)
        .expect("admitted");
    settled_app(d, submitted.request())
}

fn move_op(d: &HeadlessDriver, moving: &str, to: &str, after: bool) -> crate::ReorderOp {
    let target = id(d, to);
    crate::ReorderOp::Move {
        moving: id(d, moving),
        to: if after {
            crate::Placement::After(target)
        } else {
            crate::Placement::Before(target)
        },
    }
}

/// With no saved order the order is the vault's (alphabetical). A drag
/// saves the new order: the file, the fleet rows, the members and Load
/// all follow it.
#[test]
fn a_reorder_saves_the_load_order_and_every_view_follows_it() {
    let mut d = driver("order-move", &["alice", "bob", "carol"]);
    d.poll();
    assert_eq!(order_names(&d), ["alice", "bob", "carol"], "seeded");
    assert!(
        preferences_file_now(&d).is_none_or(|file| file.get(crate::PROFILE_ORDER_KEY).is_none()),
        "seeding writes nothing"
    );
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    assert_eq!(order_names(&d), ["carol", "alice", "bob"]);
    assert_eq!(stored_order(&d), ["carol", "alice", "bob"]);
    d.poll();
    assert_eq!(row_names(&d), ["carol", "alice", "bob"]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    settle(&mut d, load.request());
    assert_eq!(d.session().members(), ["carol", "alice", "bob"], "Load all");
    let op = move_op(&d, "bob", "carol", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    assert_eq!(
        d.session().members(),
        ["bob", "carol", "alice"],
        "members follow"
    );
}

/// Reorder only applies in Load order; Name and Status are view sorts and
/// leave the saved order byte for byte.
#[test]
fn view_sorts_never_write_the_order_and_refuse_a_reorder() {
    let mut d = driver("order-sorts", &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    let path = d.session().preferences().expect("prefs").path();
    let before = std::fs::read(&path).expect("file");
    for order in [crate::FleetOrder::Name, crate::FleetOrder::Status] {
        d.submit(OperatorCommand::SetFleetSort(order), Targets::Application)
            .expect("sort");
        let op = move_op(&d, "alice", "bob", true);
        assert_eq!(
            reorder(&mut d, op),
            Some(rejected(crate::command::Rejection::NotApplicable)),
            "{order:?}"
        );
    }
    assert_eq!(std::fs::read(&path).expect("file"), before, "byte for byte");
    assert_eq!(order_names(&d), ["carol", "alice", "bob"]);
}

/// A drag while a filter hides rows lands relative to its target in the
/// full order; hidden profiles keep their relative order. Move marked to
/// top keeps the marked profiles' order. Nothing moved: no write.
#[test]
fn filtered_moves_and_marked_to_top_keep_the_other_profiles_in_order() {
    let mut d = driver("order-filtered", &["alice", "bob", "carol", "dave"]);
    d.poll();
    d.models_mut().fleet_mut().staged_filter_mut().push('c');
    d.submit(OperatorCommand::SetFleetFilter, Targets::Application)
        .expect("filter");
    assert_eq!(shown_names(&d), ["alice", "carol"], "bob and dave hidden");
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    assert_eq!(order_names(&d), ["carol", "alice", "bob", "dave"]);
    for name in ["dave", "alice"] {
        d.submit(OperatorCommand::SetMark(true), Targets::Profile(name))
            .expect("mark");
    }
    assert_eq!(reorder(&mut d, crate::ReorderOp::MarkedToTop), Some(done()));
    assert_eq!(order_names(&d), ["alice", "dave", "carol", "bob"]);
    let path = d.session().preferences().expect("prefs").path();
    let before = std::fs::read(&path).expect("file");
    assert_eq!(reorder(&mut d, crate::ReorderOp::MarkedToTop), Some(done()));
    assert_eq!(
        std::fs::read(&path).expect("file"),
        before,
        "no move, no write"
    );
}

/// A new profile is appended, a rename keeps its place (and is saved), a
/// deleted profile leaves the order, and a new session restores it.
#[test]
fn the_order_follows_create_rename_delete_and_survives_a_restart() {
    let dir = scratch("order-lifecycle");
    let mut d = driver_in(&dir, &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    d.session_mut()
        .vault_mut()
        .expect("vault")
        .upsert(Profile {
            username: "dave".into(),
            password: "pw".into(),
            uid: 40,
            settings: ProfileSettings::default(),
        })
        .expect("dave");
    d.poll();
    assert_eq!(
        order_names(&d),
        ["carol", "alice", "bob", "dave"],
        "appended"
    );
    let mut renamed = d
        .session()
        .vault()
        .and_then(|vault| vault.get("alice"))
        .cloned()
        .expect("alice");
    renamed.username = "alicia".into();
    d.session_mut()
        .rename_profile("alice", renamed, crate::ArmMirror::none(), "profile")
        .expect("rename");
    d.session_mut().flush_writes();
    d.poll();
    assert_eq!(
        order_names(&d),
        ["carol", "alicia", "bob", "dave"],
        "in place"
    );
    let delete = d
        .submit(OperatorCommand::DeleteProfile, Targets::Profile("bob"))
        .expect("delete");
    settle(&mut d, delete.request());
    d.poll();
    assert_eq!(order_names(&d), ["carol", "alicia", "dave"]);
    assert_eq!(row_names(&d), ["carol", "alicia", "dave"]);
    // Let every quiet write land, then a new session over the same state
    // and vault restores the order.
    d.session_mut().flush_writes();
    let start = std::time::Instant::now();
    while stored_order_now(&d).as_deref()
        != Some(&["carol", "alicia", "dave"].map(String::from)[..])
    {
        assert!(start.elapsed() < SETTLE_HANG_GUARD, "the order write hung");
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    drop(d);
    let mut again = open_driver(&dir.join("state"));
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(order_names(&again), ["carol", "alicia", "dave"]);
    assert_eq!(row_names(&again), ["carol", "alicia", "dave"]);
}

/// A reorder the disk holds stays pending and settles Done when it lands.
#[test]
fn a_held_order_write_stays_pending_until_durable() {
    let mut d = driver("order-held", &["alice", "bob"]);
    d.poll();
    let release = hold_first_write(&d);
    let op = move_op(&d, "bob", "alice", false);
    d.session_mut().stage_reorder(op);
    let submitted = d
        .submit(OperatorCommand::ReorderProfiles, Targets::Application)
        .expect("admitted");
    assert_eq!(terminal_app(&d, submitted.request()), None, "pending");
    d.poll();
    assert_eq!(terminal_app(&d, submitted.request()), None);
    release.send(()).unwrap();
    assert_eq!(settled_app(&mut d, submitted.request()), Some(done()));
    assert_eq!(stored_order(&d), ["bob", "alice"]);
}

/// A refused write fails the command and puts back the last durable
/// order: a failure never shows as the saved order.
#[test]
fn a_failed_order_write_reports_failure_and_restores_the_durable_order() {
    let mut d = driver("order-failed", &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    let path = d.session().preferences().expect("prefs").path();
    std::fs::write(
        &path,
        vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1],
    )
    .expect("oversize the preferences file");
    let _ = d.session().preferences().expect("prefs").reload();
    let op = move_op(&d, "bob", "carol", false);
    let terminal = reorder(&mut d, op);
    assert!(
        matches!(terminal, Some(Settlement::Failed(_))),
        "the write is refused: {terminal:?}"
    );
    d.poll();
    assert_eq!(
        order_names(&d),
        ["carol", "alice", "bob"],
        "durable order back"
    );
    assert_eq!(row_names(&d), ["carol", "alice", "bob"]);
}

/// Removing a wall member keeps its place in the saved order; loading it
/// again puts it back in that place among the members.
#[test]
fn removing_a_member_keeps_its_saved_place() {
    let mut d = driver("order-remove", &["alice", "bob", "carol"]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    settle(&mut d, load.request());
    let remove = d
        .submit(OperatorCommand::Remove, Targets::Profile("bob"))
        .expect("remove");
    settle(&mut d, remove.request());
    assert_eq!(order_names(&d), ["alice", "bob", "carol"]);
    assert_eq!(row_names(&d), ["alice", "bob", "carol"]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Profile("bob"))
        .expect("load bob");
    settle(&mut d, load.request());
    assert_eq!(
        d.session().members(),
        ["alice", "bob", "carol"],
        "back in place"
    );
}

/// A second reorder while the first write is held supersedes it: both
/// settle Done in order, the file and the shown order end at the second,
/// and later polls write nothing more.
#[test]
fn a_superseded_order_write_settles_and_the_newest_order_wins() {
    let mut d = driver("order-superseded", &["alice", "bob", "carol"]);
    d.poll();
    let release = hold_first_write(&d);
    let op = move_op(&d, "carol", "alice", false);
    d.session_mut().stage_reorder(op);
    let first = d
        .submit(OperatorCommand::ReorderProfiles, Targets::Application)
        .expect("first");
    let op = move_op(&d, "alice", "bob", true);
    d.session_mut().stage_reorder(op);
    let second = d
        .submit(OperatorCommand::ReorderProfiles, Targets::Application)
        .expect("second");
    assert_eq!(order_names(&d), ["carol", "bob", "alice"], "shown at once");
    assert_eq!(terminal_app(&d, first.request()), None, "first pending");
    assert_eq!(terminal_app(&d, second.request()), None, "second pending");
    release.send(()).unwrap();
    assert_eq!(settled_app(&mut d, first.request()), Some(done()));
    assert_eq!(settled_app(&mut d, second.request()), Some(done()));
    assert_eq!(order_names(&d), ["carol", "bob", "alice"]);
    assert_eq!(stored_order(&d), ["carol", "bob", "alice"]);
    let path = d.session().preferences().expect("prefs").path();
    let before = std::fs::metadata(&path).expect("file").modified().ok();
    let bytes = std::fs::read(&path).expect("file");
    for _ in 0..3 {
        d.poll();
    }
    assert_eq!(
        std::fs::read(&path).expect("file"),
        bytes,
        "no further write"
    );
    assert_eq!(
        std::fs::metadata(&path).expect("file").modified().ok(),
        before
    );
}

// ---- F3: the fleet card, marks over the shown rows and frozen plans ----

fn gatherer() -> crate::scripts::ScriptChoice {
    crate::scripts::ScriptChoice::compiled(
        crate::scripts::CompiledCardId::lookup("Gatherer").expect("Gatherer is compiled"),
    )
}

fn set_card(d: &mut HeadlessDriver, card: crate::FleetCard) {
    d.models_mut().fleet_mut().stage_card(card);
    let set = d
        .submit(OperatorCommand::SetFleetCard, Targets::Application)
        .expect("card");
    assert_eq!(terminal_app(d, set.request()), Some(done()));
}

fn plan(d: &mut HeadlessDriver, action: crate::FleetAction) -> Option<Terminal> {
    let planned = d
        .submit(
            OperatorCommand::PlanFleetAction(action),
            Targets::Application,
        )
        .expect("plan admitted");
    terminal_app(d, planned.request())
}

fn mark(d: &mut HeadlessDriver, names: &[&str]) {
    for name in names {
        d.submit(OperatorCommand::SetMark(true), Targets::Profile(name))
            .expect("mark");
    }
}

fn assignment(d: &HeadlessDriver, name: &str) -> Option<String> {
    crate::scripts::Scripts::assignment_name(d.session(), name).map(str::to_owned)
}

fn plan_names(d: &HeadlessDriver) -> Vec<String> {
    d.session()
        .fleet_plan()
        .expect("a plan waits")
        .target_names()
        .map(str::to_owned)
        .collect()
}

/// Run the waiting plan (Confirm) and let its writes land.
fn run_plan(d: &mut HeadlessDriver) -> RequestId {
    let command = d.session().fleet_plan().expect("a plan waits").command();
    let run = d.submit(command, Targets::Plan).expect("run admitted");
    settle(d, run.request());
    d.session_mut().flush_writes();
    d.poll();
    run.request()
}

fn load(d: &mut HeadlessDriver, names: &[&str]) {
    for name in names {
        let load = d
            .submit(OperatorCommand::Load, Targets::Profile(name))
            .expect("load");
        settle(d, load.request());
    }
    d.poll();
}

/// Picking a fleet card changes which card the buttons use and nothing
/// else: no Browse pick, no pending heading, no assignment.
#[test]
fn picking_a_fleet_card_changes_no_pick_assignment_or_run() {
    let mut d = driver("f3-pick", &["alice", "bob"]);
    d.poll();
    assert_eq!(d.models().fleet().card(), &crate::FleetCard::Own);
    assert_eq!(d.sync_card_label(), crate::OWN_CARD_LABEL);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(d.sync_card_label(), "Gatherer");
    assert!(d.models().scripts().browse_selection().is_none());
    assert!(d.models().scripts().pending_browse("alice").is_none());
    assert_eq!(assignment(&d, "alice"), None);
    assert_eq!(assignment(&d, "bob"), None);
    set_card(&mut d, crate::FleetCard::Focused);
    assert_eq!(d.sync_card_label(), "Focused bot's card: none");
}

/// The writing actions need a card: each bot's own card refuses them, and
/// an own-card Start with no assignment has no bot to act on.
#[test]
fn own_card_refuses_assign_and_a_start_with_no_assignment() {
    let mut d = driver("f3-own", &["alice"]);
    d.poll();
    mark(&mut d, &["alice"]);
    for action in [
        crate::FleetAction::Assign,
        crate::FleetAction::AssignRestart,
        crate::FleetAction::CopySettings,
    ] {
        assert!(
            matches!(plan(&mut d, action), Some(Settlement::Failed(_))),
            "{action:?} needs a card"
        );
    }
    let start = plan(&mut d, crate::FleetAction::Start);
    assert!(matches!(start, Some(Settlement::Failed(_))), "{start:?}");
    assert!(d.session().fleet_plan().is_none());
}

/// A marked Assign runs at once over the frozen marks: a profile marked
/// after the plan does not join it, and the one result settles with the
/// targets done.
#[test]
fn a_marked_plan_freezes_its_targets_and_reports_once() {
    let mut d = driver("f3-frozen", &["alice", "bob", "carol"]);
    d.poll();
    mark(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    let waiting = d.session().fleet_plan().expect("plan");
    assert!(
        !waiting.needs_confirm(),
        "a safe marked Assign is one click"
    );
    assert!(
        waiting
            .prompt()
            .starts_with("Assign Gatherer to 2 marked bots: alice, bob."),
        "{}",
        waiting.prompt()
    );
    assert_eq!(plan_names(&d), ["alice", "bob"]);
    // Marked after the plan, before it runs: not in it.
    mark(&mut d, &["carol"]);
    run_plan(&mut d);
    assert_eq!(assignment(&d, "alice").as_deref(), Some("Gatherer"));
    assert_eq!(assignment(&d, "bob").as_deref(), Some("Gatherer"));
    assert_eq!(assignment(&d, "carol"), None, "a later mark never joins");
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().done, 2, "{}", result.text());
    assert!(
        result.text().starts_with("Assign: done 2: alice, bob"),
        "{}",
        result.text()
    );
}

/// With no marks the plan covers the loaded shown rows only and asks
/// first; Cancel drops it and writes nothing.
#[test]
fn a_no_marks_plan_covers_loaded_shown_rows_and_cancel_writes_nothing() {
    let mut d = driver("f3-all", &["alice", "bob", "carol"]);
    load(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    let waiting = d.session().fleet_plan().expect("plan");
    assert!(waiting.needs_confirm(), "all asks first");
    assert_eq!(waiting.scope(), crate::PlanScope::AllShown);
    assert_eq!(plan_names(&d), ["alice", "bob"], "carol is not loaded");
    let cancel = d
        .submit(OperatorCommand::CancelFleetPlan, Targets::Application)
        .expect("cancel");
    assert_eq!(terminal_app(&d, cancel.request()), Some(done()));
    assert!(d.session().fleet_plan().is_none());
    d.session_mut().flush_writes();
    d.poll();
    assert_eq!(assignment(&d, "alice"), None);
    assert!(
        d.submit(OperatorCommand::AssignScript, Targets::Plan)
            .is_err(),
        "no plan, nothing runs"
    );
}

/// A newer plan replaces the waiting one, and its prompt says so.
#[test]
fn a_new_plan_supersedes_the_waiting_one_with_a_notice() {
    let mut d = driver("f3-supersede", &["alice", "bob"]);
    load(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    assert_eq!(plan(&mut d, crate::FleetAction::Login), Some(done()));
    let waiting = d.session().fleet_plan().expect("plan");
    assert_eq!(waiting.action(), crate::FleetAction::Login);
    assert!(
        waiting
            .prompt()
            .starts_with("Replaces the waiting Assign plan. "),
        "{}",
        waiting.prompt()
    );
    assert!(
        d.submit(OperatorCommand::AssignScript, Targets::Plan)
            .is_err(),
        "the replaced plan cannot run"
    );
}

/// Delete and recreate a profile with the same name after the plan: the
/// plan does not act on the new identity and names the old one as gone.
#[test]
fn a_recreated_profile_with_the_same_name_is_not_in_the_old_plan() {
    let mut d = driver("f3-recreate", &["alice", "bob"]);
    d.poll();
    mark(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    let delete = d
        .submit(OperatorCommand::DeleteProfile, Targets::Profile("alice"))
        .expect("delete");
    settle(&mut d, delete.request());
    d.session_mut()
        .vault_mut()
        .expect("vault")
        .upsert(Profile {
            username: "alice".into(),
            password: "pw".into(),
            uid: 99,
            settings: ProfileSettings::default(),
        })
        .expect("new alice");
    d.poll();
    run_plan(&mut d);
    assert_eq!(assignment(&d, "alice"), None, "the new alice is untouched");
    assert_eq!(assignment(&d, "bob").as_deref(), Some("Gatherer"));
    let result = d.session().fleet_result().expect("result");
    assert!(
        result.text().contains("alice (profile no longer exists)"),
        "{}",
        result.text()
    );
}

/// Mark operations act on the shown rows; a hidden mark stays.
#[test]
fn mark_operations_cover_the_shown_rows_and_keep_hidden_marks() {
    use crate::{MarkOp, MarkPredicate, ShownMarks};
    let mut d = driver("f3-marks", &["alice", "bob", "carol", "dave"]);
    load(&mut d, &["alice", "bob"]);
    let mark_op = |d: &mut HeadlessDriver, op: MarkOp| {
        let s = d
            .submit(OperatorCommand::MarkFleet(op), Targets::Application)
            .expect("mark op");
        assert_eq!(terminal_app(d, s.request()), Some(done()));
    };
    let marked = |d: &HeadlessDriver| -> Vec<String> {
        let fleet = d.models().fleet();
        fleet
            .rows()
            .iter()
            .enumerate()
            .filter(|(index, _)| fleet.is_marked(*index))
            .map(|(_, row)| row.name().to_owned())
            .collect()
    };
    mark_op(&mut d, MarkOp::Where(MarkPredicate::NotLoaded));
    assert_eq!(marked(&d), ["carol", "dave"]);
    assert_eq!(d.models().fleet().shown_marks(), ShownMarks::Some);
    mark_op(&mut d, MarkOp::InvertShown);
    assert_eq!(marked(&d), ["alice", "bob"]);
    mark_op(&mut d, MarkOp::Shown);
    assert_eq!(d.models().fleet().shown_marks(), ShownMarks::All);
    d.models_mut().fleet_mut().staged_filter_mut().push('a');
    d.submit(OperatorCommand::SetFleetFilter, Targets::Application)
        .expect("filter");
    mark_op(&mut d, MarkOp::UnmarkShown);
    assert_eq!(marked(&d), ["bob"], "bob is hidden and keeps his mark");
    assert_eq!(d.models().fleet().counts().hidden_marked(), 1);
    assert_eq!(d.models().fleet().shown_marks(), ShownMarks::None);
}

/// Review R1: a 600-profile order fits the shared preferences service (a
/// value is at most 4,096 bytes): reorders become durable through the real
/// service, a restart restores them, and each profile costs one small
/// stored member.
#[test]
fn a_600_profile_order_is_durable_and_restores_after_a_restart() {
    let dir = scratch("order-600");
    let names: Vec<String> = (0..600).map(|i| format!("bot{i:03}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut d = driver_in(&dir, &refs);
    d.poll();
    // Move the last profile to the top, then three marked ones to the top.
    let op = move_op(&d, "bot599", "bot000", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    for name in ["bot300", "bot010", "bot500"] {
        d.submit(OperatorCommand::SetMark(true), Targets::Profile(name))
            .expect("mark");
    }
    assert_eq!(reorder(&mut d, crate::ReorderOp::MarkedToTop), Some(done()));
    let expected: Vec<String> = ["bot010", "bot300", "bot500", "bot599"]
        .into_iter()
        .map(String::from)
        .chain(
            names
                .iter()
                .filter(|n| !["bot010", "bot300", "bot500", "bot599"].contains(&n.as_str()))
                .cloned(),
        )
        .collect();
    assert_eq!(order_names(&d), expected);
    assert_eq!(stored_order(&d), expected, "durable");
    // The stored size per profile.
    let stored = preferences_file(&d)[crate::PROFILE_ORDER_KEY].clone();
    let members = stored.as_object().expect("an object");
    assert_eq!(members.len(), 600);
    let row_bytes: usize = members
        .iter()
        .map(|(uid, key)| crate::PROFILE_ORDER_KEY.len() + 1 + uid.len() + key.to_string().len())
        .sum();
    let per_profile = row_bytes as f64 / 600.0;
    eprintln!(
        "profile_order: 600 members, {row_bytes} key+value bytes, {per_profile:.1} per profile"
    );
    assert!(per_profile < 32.0, "{per_profile} bytes per profile");
    d.session_mut().flush_writes();
    drop(d);
    let mut again = open_driver(&dir.join("state"));
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(order_names(&again), expected, "restored after a restart");
}

/// Review R2: a rename keeps the profile's place whether it is held or
/// fails, because the order is keyed by identity: the live order, the
/// stored order and the order after a restart all keep it.
#[test]
fn a_held_or_failed_rename_keeps_the_saved_place() {
    let dir = scratch("order-rename-fail");
    let mut d = driver_in(&dir, &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    let op = move_op(&d, "alice", "bob", true);
    assert_eq!(reorder(&mut d, op), Some(done()));
    let saved = ["carol", "bob", "alice"].map(String::from);
    assert_eq!(order_names(&d), saved);
    assert_eq!(stored_order(&d), saved);
    let stored_before = preferences_file(&d)[crate::PROFILE_ORDER_KEY].clone();
    // Block the vault file so the rename's write fails.
    let vault_path = dir.join("vault");
    let aside = vault_path.with_extension("aside");
    std::fs::rename(&vault_path, &aside).expect("aside");
    std::fs::create_dir(&vault_path).expect("block");
    let mut renamed = d
        .session()
        .vault()
        .and_then(|vault| vault.get("alice"))
        .cloned()
        .expect("alice");
    renamed.username = "alicia".into();
    d.session_mut()
        .rename_profile("alice", renamed, crate::ArmMirror::none(), "profile")
        .expect("rename staged");
    d.poll();
    assert_eq!(
        order_names(&d),
        ["carol", "bob", "alicia"],
        "held: the staged name in the same place"
    );
    d.session_mut().flush_writes();
    std::fs::remove_dir(&vault_path).expect("unblock");
    std::fs::rename(&aside, &vault_path).expect("restore");
    d.poll();
    assert_eq!(order_names(&d), saved, "failed: the old name in its place");
    assert_eq!(
        preferences_file(&d)[crate::PROFILE_ORDER_KEY],
        stored_before,
        "the stored order never changed"
    );
    assert_eq!(stored_order(&d), saved);
    d.session_mut().flush_writes();
    drop(d);
    let mut again = open_driver(&dir.join("state"));
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(order_names(&again), saved, "after a restart");
}

/// A settings copy is planned from the focused bot with the fleet card:
/// it asks first, names its source, holds the prepared scope until
/// Confirm, and Cancel drops the scope and writes nothing.
#[test]
fn a_copy_settings_plan_asks_first_and_cancel_releases_its_scope() {
    let mut d = driver("f3-copy", &["alice", "bob"]);
    load(&mut d, &["alice", "bob"]);
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    settle(&mut d, select.request());
    mark(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    assert_eq!(assignment(&d, "bob").as_deref(), Some("Gatherer"));
    d.submit(OperatorCommand::SetMark(false), Targets::Profile("alice"))
        .expect("unmark");
    let copy = d
        .submit(
            OperatorCommand::PlanFleetAction(crate::FleetAction::CopySettings),
            Targets::Application,
        )
        .expect("plan");
    let member = d
        .report(copy.request())
        .and_then(|report| report.member(Subject::Application))
        .expect("member");
    assert_eq!(member.terminal(), Some(&done()), "{:?}", member.detail());
    let waiting = d.session().fleet_plan().expect("plan");
    assert!(waiting.needs_confirm(), "a copy always asks");
    assert!(
        waiting
            .prompt()
            .starts_with("Copy alice's Gatherer settings to 1 marked bot: bob."),
        "{}",
        waiting.prompt()
    );
    let scope = d
        .models()
        .scripts()
        .prepared_settings_sync()
        .expect("the frozen scope waits");
    assert_eq!(scope.source(), "alice");
    let cancel = d
        .submit(OperatorCommand::CancelFleetPlan, Targets::Application)
        .expect("cancel");
    assert_eq!(terminal_app(&d, cancel.request()), Some(done()));
    assert!(
        d.models().scripts().prepared_settings_sync().is_none(),
        "Cancel releases the prepared scope"
    );
    assert!(d.session().fleet_plan().is_none());
}

/// The banner never carries a second copy of a planned action's report: a
/// planned Start and a planned Copy settings report through the plan's one
/// result line only.
#[test]
fn a_planned_start_or_copy_reports_only_through_its_result() {
    let mut d = driver("f3-banner", &["alice", "bob"]);
    load(&mut d, &["alice", "bob"]);
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    settle(&mut d, select.request());
    mark(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    let _ = d.models_mut().scripts.take_notice();
    assert_eq!(plan(&mut d, crate::FleetAction::Start), Some(done()));
    // Without a game the Starts fail at once; the run may stay open for
    // its paced queue, so it is polled, not settled.
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan).expect("run");
    let mut shown = Vec::new();
    let drain = |d: &mut HeadlessDriver, shown: &mut Vec<String>| {
        if let Some(notice) = d.models_mut().scripts.take_notice() {
            shown.push(format!("{notice:?}"));
        }
        d.poll();
    };
    for _ in 0..5 {
        drain(&mut d, &mut shown);
    }
    let result = d.session().fleet_result().expect("result");
    assert!(result.text().starts_with("Start"), "{}", result.text());
    d.submit(OperatorCommand::SetMark(false), Targets::Profile("alice"))
        .expect("unmark");
    assert_eq!(plan(&mut d, crate::FleetAction::CopySettings), Some(done()));
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan).expect("copy");
    for _ in 0..5 {
        drain(&mut d, &mut shown);
        d.session_mut().flush_writes();
    }
    drain(&mut d, &mut shown);
    let result = d.session().fleet_result().expect("result");
    assert!(result.text().starts_with("Copy"), "{}", result.text());
    // An ordinary error (a Start that failed without a game) may show; a
    // bulk report never does.
    for text in &shown {
        assert!(
            !["Start selected", "started", "Apply", "saved"]
                .iter()
                .any(|bulk| text.contains(bulk)),
            "a planned report on the banner: {shown:?}"
        );
    }
}

/// A rename and a card change between the plan and its Confirm neither
/// retarget nor re-card it: the renamed bot keeps its frozen identity and
/// takes the frozen card.
#[test]
fn a_rename_or_card_change_before_confirm_never_retargets_the_plan() {
    let mut d = driver("f3-rename-card", &["alice", "bob", "carol"]);
    d.poll();
    mark(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    let mut renamed = d
        .session()
        .vault()
        .and_then(|vault| vault.get("bob"))
        .cloned()
        .expect("bob");
    renamed.username = "bobby".into();
    d.session_mut()
        .rename_profile("bob", renamed, crate::ArmMirror::none(), "profile")
        .expect("rename");
    d.session_mut().flush_writes();
    d.poll();
    set_card(
        &mut d,
        crate::FleetCard::Pinned(crate::scripts::ScriptChoice::compiled(
            crate::scripts::CompiledCardId::QUESTER,
        )),
    );
    mark(&mut d, &["carol"]);
    run_plan(&mut d);
    assert_eq!(assignment(&d, "alice").as_deref(), Some("Gatherer"));
    assert_eq!(
        assignment(&d, "bobby").as_deref(),
        Some("Gatherer"),
        "same identity"
    );
    assert_eq!(assignment(&d, "carol"), None);
}

/// A bot whose profile is being saved (here, a held rename) is a named skip
/// of the plan, never a competing write; a plan's queued writes keep its
/// result Pending until they land.
#[test]
fn a_profile_being_saved_is_a_named_skip_and_queued_writes_keep_the_result_pending() {
    let mut d = driver("f3-saving", &["alice", "bob"]);
    d.poll();
    mark(&mut d, &["alice", "bob"]);
    set_card(&mut d, crate::FleetCard::Pinned(gatherer()));
    let gate = d.session().write_gate();
    let held = gate.lock().expect("gate");
    let mut renamed = d
        .session()
        .vault()
        .and_then(|vault| vault.get("bob"))
        .cloned()
        .expect("bob");
    renamed.username = "bobby".into();
    d.session_mut()
        .rename_profile("bob", renamed, crate::ArmMirror::none(), "profile")
        .expect("rename staged");
    d.poll();
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    assert_eq!(plan_names(&d), ["alice"]);
    let prompt = d.session().fleet_plan().expect("plan").prompt().to_owned();
    assert!(prompt.contains("bobby: profile is being saved"), "{prompt}");
    let command = d.session().fleet_plan().expect("plan").command();
    let run = d.submit(command, Targets::Plan).expect("run");
    for _ in 0..5 {
        d.poll();
    }
    let result = d.session().fleet_result().expect("result");
    assert!(
        !result.settled(),
        "a queued write is Pending: {}",
        result.text()
    );
    drop(held);
    settle(&mut d, run.request());
    d.session_mut().flush_writes();
    d.poll();
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().done, 1, "{}", result.text());
    assert_eq!(assignment(&d, "alice").as_deref(), Some("Gatherer"));
    assert_eq!(assignment(&d, "bobby"), None, "never a competing write");
}

/// The stored Load order's members (`"<uid>"` keys, sorted), once every
/// quiet write has landed: waits until they are exactly the vault's uids.
fn stored_uids_settled(d: &mut HeadlessDriver) -> Vec<i32> {
    let start = std::time::Instant::now();
    loop {
        d.session_mut().flush_writes();
        d.poll();
        let mut vault: Vec<i32> = d
            .session()
            .vault()
            .expect("vault")
            .profiles()
            .map(|profile| profile.uid)
            .collect();
        vault.sort_unstable();
        let stored: Option<Vec<i32>> = preferences_file_now(d).and_then(|file| {
            let members = file.get(crate::PROFILE_ORDER_KEY)?.as_object()?.clone();
            let mut uids: Vec<i32> = members.keys().filter_map(|uid| uid.parse().ok()).collect();
            uids.sort_unstable();
            Some(uids)
        });
        if stored.as_ref() == Some(&vault) {
            return vault;
        }
        assert!(
            start.elapsed() < SETTLE_HANG_GUARD,
            "the stored order never matched the vault: {stored:?} vs {vault:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn uid_of(d: &HeadlessDriver, name: &str) -> i32 {
    d.session()
        .vault()
        .and_then(|vault| vault.get(name))
        .map(|profile| profile.uid)
        .expect("a vault row")
}

/// Create `name` the way the TUI does (a staged new profile, no form), so
/// its uid is the session's fresh uid.
fn create_profile(d: &mut HeadlessDriver, name: &str) {
    d.session_mut().stage_new_profile(name, "pw");
    let create = d
        .session_mut()
        .submit(
            OperatorCommand::CreateProfile,
            Targets::Application,
            &mut crate::HeadlessSurface::new(),
        )
        .expect("create");
    assert_eq!(settled_app(d, create.request()), Some(done()), "{name}");
    d.session_mut().flush_writes();
    d.poll();
}

fn delete_profile(d: &mut HeadlessDriver, name: &str) {
    let delete = d
        .submit(OperatorCommand::DeleteProfile, Targets::Profile(name))
        .expect("delete");
    settle(d, delete.request());
    d.session_mut().flush_writes();
    d.poll();
}

/// Review R3: a durable deletion drops the profile's stored member, and a
/// profile created next never reuses its uid (the highest one), so it is
/// appended instead of taking the deleted profile's place; a restart
/// keeps that.
#[test]
fn a_deleted_profiles_member_is_pruned_and_its_uid_never_reused() {
    let dir = scratch("order-uid-reuse");
    let mut d = driver_in(&dir, &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    assert_eq!(stored_order(&d), ["carol", "alice", "bob"]);
    let carol = uid_of(&d, "carol");
    delete_profile(&mut d, "carol");
    let stored = stored_uids_settled(&mut d);
    assert_eq!(stored.len(), 2);
    assert!(!stored.contains(&carol), "pruned: {stored:?}");
    create_profile(&mut d, "dave");
    assert!(
        uid_of(&d, "dave") > carol,
        "a fresh uid above every uid held"
    );
    assert_eq!(order_names(&d), ["alice", "bob", "dave"], "appended");
    assert_eq!(stored_uids_settled(&mut d).len(), 3);
    assert_eq!(stored_order(&d), ["alice", "bob", "dave"]);
    drop(d);
    let mut again = open_driver(&dir.join("state"));
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(
        order_names(&again),
        ["alice", "bob", "dave"],
        "after a restart"
    );
}

/// Review R3: repeated create and delete with a fixed number of profiles
/// keeps the stored order to the vault's profiles: no deleted history is
/// retained.
#[test]
fn repeated_create_and_delete_keeps_the_stored_order_to_the_vault() {
    let mut d = driver("order-churn", &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    let mut previous = "bob".to_string();
    for round in 0..8 {
        let name = format!("churn{round}");
        create_profile(&mut d, &name);
        delete_profile(&mut d, &previous);
        let stored = stored_uids_settled(&mut d);
        assert_eq!(stored.len(), 3, "round {round}: {stored:?}");
        assert_eq!(order_names(&d), ["carol", "alice", name.as_str()]);
        previous = name;
    }
}

/// Review R3: while a deletion is not durable its member stays stored, and
/// a failed deletion puts the profile back in its saved place; the stored
/// order never changes and a restart keeps it.
#[test]
fn a_pending_or_failed_deletion_keeps_the_saved_place() {
    let dir = scratch("order-delete-fail");
    let mut d = driver_in(&dir, &["alice", "bob", "carol"]);
    d.poll();
    let op = move_op(&d, "carol", "alice", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    let saved = ["carol", "alice", "bob"].map(String::from);
    let stored_before = preferences_file(&d)[crate::PROFILE_ORDER_KEY].clone();
    // Pending: the vault write is held.
    let gate = d.session().write_gate();
    let held = gate.lock().expect("gate");
    let delete = d
        .submit(OperatorCommand::DeleteProfile, Targets::Profile("alice"))
        .expect("delete");
    for _ in 0..3 {
        d.poll();
    }
    assert_eq!(order_names(&d), ["carol", "bob"], "staged away");
    assert_eq!(
        preferences_file(&d)[crate::PROFILE_ORDER_KEY],
        stored_before,
        "pending: the member stays stored"
    );
    // Failed: block the vault file, then let the write run.
    let vault_path = dir.join("vault");
    let aside = vault_path.with_extension("aside");
    std::fs::rename(&vault_path, &aside).expect("aside");
    std::fs::create_dir(&vault_path).expect("block");
    drop(held);
    settle(&mut d, delete.request());
    d.session_mut().flush_writes();
    std::fs::remove_dir(&vault_path).expect("unblock");
    std::fs::rename(&aside, &vault_path).expect("restore");
    d.poll();
    assert_eq!(order_names(&d), saved, "failed: back in place");
    assert_eq!(
        preferences_file(&d)[crate::PROFILE_ORDER_KEY],
        stored_before,
        "the stored order never changed"
    );
    drop(d);
    let mut again = open_driver(&dir.join("state"));
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(order_names(&again), saved, "after a restart");
}

/// Review R4: a 600-profile marked move whose order write fails inside the
/// file replacement (the state directory refuses the new file) fails the
/// command, puts the durable order back, and leaves the stored order byte
/// for byte as before: no member of the move lands, before or after a
/// restart. A reorder is one file write.
#[cfg(unix)]
#[test]
fn a_failed_large_marked_move_leaves_the_stored_order_exactly_as_before() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("order-600-fail");
    let names: Vec<String> = (0..600).map(|i| format!("bot{i:03}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut d = driver_in(&dir, &refs);
    d.poll();
    let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let refuse = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let path = d.session().preferences().expect("prefs").path();
    let state_dir = path.parent().expect("a state directory").to_path_buf();
    {
        let writes = std::sync::Arc::clone(&writes);
        let refuse = std::sync::Arc::clone(&refuse);
        let state_dir = state_dir.clone();
        d.session()
            .local_state()
            .expect("the driver binds the process service")
            .set_write_stall_for_test(std::sync::Arc::new(move || {
                writes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if refuse.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = std::fs::set_permissions(
                        &state_dir,
                        std::fs::Permissions::from_mode(0o500),
                    );
                }
            }));
    }
    let op = move_op(&d, "bot599", "bot000", false);
    assert_eq!(reorder(&mut d, op), Some(done()));
    assert_eq!(
        writes.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one reorder, one file write"
    );
    let saved = order_names(&d);
    let bytes_before = std::fs::read(&path).expect("file");
    for name in ["bot300", "bot010", "bot500"] {
        d.submit(OperatorCommand::SetMark(true), Targets::Profile(name))
            .expect("mark");
    }
    refuse.store(true, std::sync::atomic::Ordering::SeqCst);
    let terminal = reorder(&mut d, crate::ReorderOp::MarkedToTop);
    refuse.store(false, std::sync::atomic::Ordering::SeqCst);
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).expect("unblock");
    assert!(
        matches!(terminal, Some(Settlement::Failed(_))),
        "the write fails: {terminal:?}"
    );
    d.poll();
    assert_eq!(order_names(&d), saved, "the durable order back");
    assert_eq!(
        std::fs::read(&path).expect("file"),
        bytes_before,
        "byte for byte"
    );
    d.session_mut().flush_writes();
    drop(d);
    let mut again = open_driver(&dir.join("state"));
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(order_names(&again), saved, "after a restart");
}
