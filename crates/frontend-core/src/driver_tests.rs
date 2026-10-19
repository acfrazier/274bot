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
    let mut d = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
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

fn rejected(rejection: crate::command::Rejection) -> Terminal {
    Settlement::Failed(CommandFailure::Rejected(rejection))
}

/// The preferences document on disk, as `d`'s service wrote it.
fn preferences_file(d: &HeadlessDriver) -> serde_json::Map<String, serde_json::Value> {
    let path = d.session().preferences().expect("bound preferences").path();
    let bytes = std::fs::read(&path).expect("the preferences file exists");
    serde_json::from_slice(&bytes).expect("the preferences file is JSON")
}

#[test]
fn unlock_with_a_wrong_passphrase_is_refused_and_keeps_the_vault() {
    let dir = scratch("unlock-wrong");
    let vault = vault_with(&dir, &["alice"]);
    let mut d = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
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
    let mut d = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
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

    let mut again = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
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
    let mut d = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
    d.session_mut().stage_vault_reset(vault.clone());
    let reset = d
        .submit(OperatorCommand::ResetVault, Targets::Application)
        .expect("admitted");
    assert_eq!(terminal_app(&d, reset.request()), Some(done()));
    assert!(!vault.is_file(), "ResetVault deleted the locked vault file");

    let mut open = driver_in(&scratch("reset-vault-open"), &["bob"]);
    let path = open.session().preferences().expect("prefs").path();
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
    let mut d = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
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
            terminal_app(&d, submitted.request()),
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
    let again = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
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
    assert_eq!(terminal_app(&d, off.request()), Some(done()));
    assert!(crate::log_file::session_log_path().is_none());
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
    assert!(d.models().marks().contains(alice));
    d.session_mut().stage_mark_all([alice, bob]);
    d.submit(OperatorCommand::MarkAll, Targets::Application)
        .expect("mark all");
    assert_eq!(d.models().marks().len(), 2);
    d.models_mut()
        .fleet_filter_mut()
        .staged_mut()
        .push_str("bob");
    assert_eq!(
        d.models().fleet_filter().text(),
        "",
        "a staged edit is not shown yet"
    );
    let filter = d
        .submit(OperatorCommand::SetFleetFilter, Targets::Application)
        .expect("filter");
    assert_eq!(terminal_app(&d, filter.request()), Some(done()));
    assert_eq!(d.models().fleet_filter().text(), "bob");
    d.submit(OperatorCommand::ClearMarks, Targets::Application)
        .expect("clear");
    assert!(d.models().marks().is_empty());
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
