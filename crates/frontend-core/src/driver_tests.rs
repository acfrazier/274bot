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
use crate::test_support::IsolatedStores;

/// A test's own scratch directory, holding an isolated bot home beside it
/// for as long as the test keeps it. Every path a driver resolves under
/// `~/.274bot` (session logs, the card library and its transpile cache, the
/// saved card settings, the catalogue pointer) lands under this directory,
/// never under the operator's home; the whole tree goes on drop.
struct Scratch {
    dir: PathBuf,
    _iso: IsolatedStores,
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.dir
    }
}

fn scratch(test: &str) -> Scratch {
    let iso = IsolatedStores::enter(&format!("driver-{test}"));
    let dir = iso.dir().join("scratch");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    Scratch { dir, _iso: iso }
}

/// A driver over the state root `dir/state`. Opening restores the Quest Paths
/// source into the process-wide registry, so it waits for a test that
/// owns that source. The guarded [`Scratch`] it takes keeps every
/// home-relative write inside the test: no caller can pass a bare path and
/// let the guard drop first.
fn open_driver(dir: &Scratch) -> HeadlessDriver {
    let _source = crate::quester_paths::registry_source_test_lock();
    let mut driver = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
    driver.session.preferences().unwrap().reload().unwrap();
    driver.poll();
    driver
}

#[test]
fn cold_preferences_adopt_saved_startup_models_without_writing_defaults() {
    let dir = scratch("cold-preferences-startup");
    let mut driver = open_driver(&dir);
    let _source = crate::quester_paths::registry_source_test_lock();
    let state = dir.join("saved");
    let path = state.join("panel-ui.json");
    let saved = br#"{"capture":false,"sidecar_50":true,"focused_50":false,"map_bake":"always","quester_paths":{"enabled":false,"folder":"saved-source"},"nav":{"allow_teleports":true,"pause_script_on_manual_walk_abort":false}}"#;
    vault::write_private_file(&path, saved).unwrap();
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(state),
        std::sync::Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    driver.session.bind_local_state(service);
    driver.models.walk_globals = driver.session.adopt_operator_preferences();
    let ready = |driver: &HeadlessDriver| driver.session.preferences().unwrap().ready();
    for _ in 0..4 {
        driver.poll();
        assert!(!ready(&driver));
        assert!(!driver.models.walk_globals.globals().allow_teleports());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(ready(&driver) && driver.models.walk_globals.globals().allow_teleports())
        && std::time::Instant::now() < deadline
    {
        driver.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(ready(&driver));
    assert!(driver.models.walk_globals.globals().allow_teleports());
    assert!(!driver.session.running_pause_script_on_manual_walk_abort());
    assert!(!driver.models.draw.capture());
    assert!(driver.models.draw.sidecar_50());
    assert!(!driver.models.draw.focused_50());
    assert_eq!(
        driver.models.draw_changes(),
        3,
        "the surface is told each saved switch that differs from the provisional default"
    );
    assert_eq!(
        driver.models.map.bake_gate().choice(),
        crate::MapBakeChoice::Always
    );
    assert_eq!(
        driver.models.quester.settings().folder(),
        Path::new("saved-source")
    );
    assert_eq!(std::fs::read(path).unwrap(), saved);
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
/// unlocked through `UnlockVault` over a vault holding `names`. The guarded
/// [`Scratch`] it takes keeps every home-relative write inside the test.
fn driver_in(dir: &Scratch, names: &[&str]) -> HeadlessDriver {
    let vault = vault_with(dir, names);
    let mut d = open_driver(dir);
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

/// A driver that owns its test's scratch and isolated home for the whole
/// test: the guard drops (and removes the home) when the test does. It
/// derefs to [`HeadlessDriver`], so a test drives it as before.
struct TestDriver {
    driver: HeadlessDriver,
    _scratch: Scratch,
}

impl std::ops::Deref for TestDriver {
    type Target = HeadlessDriver;

    fn deref(&self) -> &HeadlessDriver {
        &self.driver
    }
}

impl std::ops::DerefMut for TestDriver {
    fn deref_mut(&mut self) -> &mut HeadlessDriver {
        &mut self.driver
    }
}

fn driver(test: &str, names: &[&str]) -> TestDriver {
    let scratch = scratch(test);
    let driver = driver_in(&scratch, names);
    TestDriver {
        driver,
        _scratch: scratch,
    }
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
    wait_for_command(
        &mut d,
        remove.request(),
        "the removed profile's slot to retire",
        |d| {
            d.report(remove.request())
                .is_some_and(|report| report.is_settled())
        },
    );
}

/// A loaded host took more than five seconds to settle a queued preference
/// write. Use a 10-minute hang guard; it is not the success condition.
const SETTLE_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(600);

/// Poll the front-end driver until a product terminal signal appears.
fn wait_for_command(
    d: &mut HeadlessDriver,
    request: RequestId,
    waiting_for: &str,
    mut settled: impl FnMut(&mut HeadlessDriver) -> bool,
) {
    let deadline = std::time::Instant::now() + SETTLE_HANG_GUARD;
    loop {
        d.poll();
        if settled(d) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "command {request:?} did not settle while waiting for {waiting_for}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Poll the session until its report is terminal. Writes settle off the UI thread.
fn settle(d: &mut HeadlessDriver, request: RequestId) {
    wait_for_command(d, request, "its command report", |d| {
        d.report(request).is_some_and(|report| report.is_settled())
    });
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

/// Wait for the application's terminal product signal while polling the
/// session as a front end's frames do.
fn settled_app(d: &mut HeadlessDriver, request: RequestId) -> Option<Terminal> {
    d.report(request)?;
    wait_for_command(d, request, "the application member terminal", |d| {
        terminal_app(d, request).is_some()
    });
    terminal_app(d, request)
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
    let mut d = open_driver(&dir);
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
    let mut d = open_driver(&dir);
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

    let mut again = open_driver(&dir);
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
    let mut d = open_driver(&dir);
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
    let mut d = open_driver(&dir);
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
    let _sink = crate::log_file::sink_test_lock();
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
    let again = open_driver(&dir);
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

/// HEADED-RECORD: a watching front end sees each saved-setting
/// transition (pending while the disk holds the write, failed with the
/// writer's reason, saved once a retry lands), and nothing is recorded
/// while nobody watches.
#[test]
fn watched_setting_events_report_pending_failed_then_saved() {
    use crate::nav_prefs::NavPreference;
    use crate::{SettingEvent, SettingEventState};
    let dir = scratch("setting-events");
    let mut d = driver_in(&dir, &["alice"]);

    // Unwatched: a durable write records nothing.
    let submitted = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::AllowTeleports, true),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(settled_app(&mut d, submitted.request()), Some(done()));
    assert_eq!(
        d.session_mut().take_setting_events(),
        (Vec::new(), false),
        "no watcher, no recorded transitions"
    );

    d.session_mut().watch_setting_events(true);
    let release = hold_first_write(&d);
    let submitted = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::AllowTeleports, false),
            Targets::Application,
        )
        .expect("admitted");
    d.poll();
    let (events, incomplete) = d.session_mut().take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events,
        vec![SettingEvent::new(
            "nav.allow_teleports".into(),
            SettingEventState::Pending,
            None,
        )],
        "the held write is pending under its stable setting identity"
    );

    // Hold the preferences file's write lock so the held write fails when
    // released: the writer's bounded lock wait runs out (the same
    // contention the T5.3-S3 headed check uses).
    let path = d.session().preferences().expect("bound preferences").path();
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(std::path::PathBuf::from(lock_name))
        .expect("lock file");
    lock_file.try_lock().expect("hold the write lock");
    release.send(()).unwrap();
    let terminal = settled_app(&mut d, submitted.request());
    assert!(
        matches!(terminal, Some(Settlement::Failed(_))),
        "the blocked write settles failed: {terminal:?}"
    );
    let (events, incomplete) = d.session_mut().take_setting_events();
    assert!(!incomplete);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].setting(), "nav.allow_teleports");
    assert_eq!(events[0].state(), SettingEventState::Failed);
    assert!(
        events[0].error().is_some_and(|e| !e.is_empty()),
        "the failure carries the writer's reason: {events:?}"
    );

    // Saving another navigation setting must not share the failed setting's
    // history or clear it.
    lock_file.unlock().expect("release the write lock");
    drop(lock_file);
    let other = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::AllowWilderness, true),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(settled_app(&mut d, other.request()), Some(done()));
    let (events, incomplete) = d.session_mut().take_setting_events();
    assert!(!incomplete);
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0].setting(), "nav.allow_wilderness");
    assert_eq!(events[0].state(), SettingEventState::Pending);
    assert_eq!(events[1].setting(), "nav.allow_wilderness");
    assert_eq!(events[1].state(), SettingEventState::Saved);

    // A retry of the original setting reports its own durable result.
    let submitted = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::AllowTeleports, false),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(settled_app(&mut d, submitted.request()), Some(done()));
    let (events, incomplete) = d.session_mut().take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [
            SettingEventState::Cleared,
            SettingEventState::Pending,
            SettingEventState::Saved,
        ],
        "retry clears the retained failure, then reports its own write: {events:?}"
    );
    assert!(events
        .iter()
        .all(|event| { event.setting() == "nav.allow_teleports" && event.error().is_none() }));

    // Watching off drops anything undrained and records nothing more.
    d.session_mut().watch_setting_events(false);
    let submitted = d
        .submit(
            OperatorCommand::SetNavPreference(NavPreference::AllowTeleports, true),
            Targets::Application,
        )
        .expect("admitted");
    assert_eq!(settled_app(&mut d, submitted.request()), Some(done()));
    assert_eq!(d.session_mut().take_setting_events(), (Vec::new(), false));
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
    let context = crate::walk_map::WalkMapContext::browsing(
        nav_planner::map::identity::Digest([0; 32]),
        None,
        1,
    );
    let mut map = crate::walk_map::WalkMap::new();
    d.models_mut().map_mut().stage_walk_to(
        &mut map,
        context,
        None,
        nav_planner::router::FindOptions::default(),
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
    let mut again = open_driver(&dir);
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
    use crate::SettingEventState;
    d.session_mut().watch_setting_events(true);
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
    let (events, incomplete) = d.session_mut().take_setting_events();
    assert!(!incomplete);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].setting(), "profile order");
    assert_eq!(events[0].state(), SettingEventState::Failed);
    assert!(
        events[0].error().is_some_and(|error| !error.is_empty()),
        "the refusal carries its reason: {events:?}"
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
    assert_eq!(d.models().fleet().card(), &crate::FleetCard::OWN);
    assert_eq!(d.sync_card_label(), crate::OWN_CARD_LABEL);
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
    assert_eq!(d.sync_card_label(), "Gatherer");
    assert!(d.models().scripts().browse_selection().is_none());
    assert!(d.models().scripts().pending_browse("alice").is_none());
    assert_eq!(assignment(&d, "alice"), None);
    assert_eq!(assignment(&d, "bob"), None);
    set_card(&mut d, crate::FleetCard::FOCUSED);
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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
    let mut again = open_driver(&dir);
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
    let mut again = open_driver(&dir);
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    let _ = d.models_mut().scripts.take_notice();
    assert_eq!(plan(&mut d, crate::FleetAction::Start), Some(done()));
    // Without a game data set the Starts fail: the result still settles,
    // with the failures (a failed Start is a terminal, never Pending).
    let command = d.session().fleet_plan().expect("plan").command();
    let run = d.submit(command, Targets::Plan).expect("run");
    let mut shown = Vec::new();
    let take_notice = |d: &mut HeadlessDriver, shown: &mut Vec<String>| {
        if let Some(notice) = d.models_mut().scripts.take_notice() {
            shown.push(format!("{notice:?}"));
        }
    };
    wait_for_command(&mut d, run.request(), "the fleet result", |d| {
        take_notice(d, &mut shown);
        d.session()
            .fleet_result()
            .is_some_and(|result| result.request() == Some(run.request()) && result.settled())
    });
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{} | shown {shown:?}", result.text());
    assert!(
        result.text().starts_with("Start: failed 2"),
        "{}",
        result.text()
    );
    d.submit(OperatorCommand::SetMark(false), Targets::Profile("alice"))
        .expect("unmark");
    assert_eq!(plan(&mut d, crate::FleetAction::CopySettings), Some(done()));
    let command = d.session().fleet_plan().expect("plan").command();
    let copy = d.submit(command, Targets::Plan).expect("copy");
    wait_for_command(&mut d, copy.request(), "the fleet result", |d| {
        take_notice(d, &mut shown);
        d.session()
            .fleet_result()
            .is_some_and(|result| result.request() == Some(copy.request()) && result.settled())
    });
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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
        crate::FleetCard::pinned(crate::scripts::ScriptChoice::compiled(
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
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
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

/// Load `names`, give them Gatherer through a plan and run a planned Start
/// with no frame between: the click admits [`START_ADMIT_PER_FRAME`] and
/// every other Start waits in the paced queue. Returns the Start command
/// and the names that wait, each still Pending.
///
/// [`START_ADMIT_PER_FRAME`]: crate::scripts::START_ADMIT_PER_FRAME
fn queued_start(d: &mut HeadlessDriver, names: &[&str]) -> (RequestId, Vec<String>) {
    load(d, names);
    mark(d, names);
    set_card(d, crate::FleetCard::pinned(gatherer()));
    assert_eq!(plan(d, crate::FleetAction::Assign), Some(done()));
    run_plan(d);
    assert_eq!(plan(d, crate::FleetAction::Start), Some(done()));
    let command = d.session().fleet_plan().expect("plan").command();
    let start = d.submit(command, Targets::Plan).expect("start").request();
    let queued: Vec<String> = names
        .iter()
        .filter(|name| d.models().scripts().start_queue_place(name).is_some())
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(
        queued.len(),
        names.len() - crate::scripts::START_ADMIT_PER_FRAME,
        "the click admits one Start; the rest wait: {queued:?}"
    );
    for name in &queued {
        assert_eq!(terminal(d, start, name), None, "{name} is Pending");
    }
    let _ = d.models_mut().scripts.take_start_failures();
    (start, queued)
}

/// Frames pass and none of `names` is admitted: no place, no preparation,
/// and no Start attempt (without a game data set every attempt fails, so
/// one would show as a Start failure).
fn no_later_start(d: &mut HeadlessDriver, names: &[String]) {
    for _ in 0..20 {
        d.poll();
    }
    let failures = d.models_mut().scripts.take_start_failures();
    for name in names {
        let scripts = d.models().scripts();
        assert!(scripts.start_queue_place(name).is_none(), "{name} queued");
        #[cfg(feature = "js")]
        assert!(!scripts.has_start_preparation(name), "{name} preparing");
        assert!(
            !failures.iter().any(|(failed, _)| failed == name),
            "{name} was started later: {failures:?}"
        );
    }
}

fn operator_cancelled() -> Terminal {
    Settlement::Cancelled(api::identity::Cancellation::Operator)
}

const FOUR: [&str; 4] = ["alice", "bob", "carol", "dave"];

/// Review F3-2: a planned Stop cancels the frozen targets' queued Starts
/// through the coordinator. The Stop members are done, the original Start
/// members settle cancelled, and no queued Start runs later.
#[test]
fn a_planned_stop_cancels_queued_starts_and_both_commands_settle() {
    let mut d = driver("f3-stop-queued", &FOUR);
    let (start, queued) = queued_start(&mut d, &FOUR);
    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    let planned = plan_names(&d);
    for name in &queued {
        assert!(planned.contains(name), "a queued Start is a Stop target");
    }
    let stop = run_plan(&mut d);
    for name in &queued {
        assert_eq!(terminal(&d, stop, name), Some(done()), "Stop {name}");
        assert_eq!(
            terminal(&d, start, name),
            Some(operator_cancelled()),
            "Start {name}"
        );
    }
    assert!(
        d.report(start).is_some_and(|report| report.is_settled()),
        "the Start command is settled"
    );
    no_later_start(&mut d, &queued);
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
}

/// Review F3-3: Cancel of a waiting plan touches no submitted Start; Stop
/// Everyone then drains the queue and settles every original Start member.
#[test]
fn stop_everyone_settles_each_queued_start_command() {
    let mut d = driver("f3-stop-everyone", &FOUR);
    let (start, queued) = queued_start(&mut d, &FOUR);
    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    let cancel = d
        .submit(OperatorCommand::CancelFleetPlan, Targets::Application)
        .expect("cancel");
    assert_eq!(terminal_app(&d, cancel.request()), Some(done()));
    for name in &queued {
        assert!(
            d.models().scripts().start_queue_place(name).is_some(),
            "Cancel of the waiting plan leaves {name}'s submitted Start queued"
        );
        assert_eq!(terminal(&d, start, name), None, "{name}");
    }
    let stop = d
        .submit(OperatorCommand::StopScript, Targets::Everyone)
        .expect("stop everyone");
    settle(&mut d, stop.request());
    d.poll();
    for name in &queued {
        assert_eq!(
            terminal(&d, start, name),
            Some(operator_cancelled()),
            "Start {name}"
        );
    }
    assert!(
        d.report(start).is_some_and(|report| report.is_settled()),
        "the Start command is settled"
    );
    no_later_start(&mut d, &queued);
}

/// Review F3-3: an explicit queued Cancel, the cancel half of a confirmed
/// relog (`logged out`) and a member removed while it waits each settle
/// that member's original Start, and none of them starts later.
#[test]
fn explicit_cancel_relog_and_removal_settle_the_queued_start_command() {
    let mut d = driver("f3-cancel-paths", &FOUR);
    let (start, queued) = queued_start(&mut d, &FOUR);
    let [relogged, cancelled, removed] = [&queued[0], &queued[1], &queued[2]];
    d.models_mut().scripts.stage_cancel_queued("logged out");
    let relog = d
        .submit(
            OperatorCommand::CancelQueuedStart,
            Targets::Profile(relogged),
        )
        .expect("relog cancel");
    assert_eq!(terminal(&d, relog.request(), relogged), Some(done()));
    let cancel = d
        .submit(
            OperatorCommand::CancelQueuedStart,
            Targets::Profile(cancelled),
        )
        .expect("cancel");
    assert_eq!(terminal(&d, cancel.request(), cancelled), Some(done()));
    d.submit(OperatorCommand::Remove, Targets::Profile(removed))
        .expect("remove");
    settle(&mut d, start);
    assert_eq!(
        terminal(&d, start, relogged),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::NotLoggedIn
        )))
    );
    assert_eq!(terminal(&d, start, cancelled), Some(operator_cancelled()));
    assert_eq!(
        terminal(&d, start, removed),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::NotLoaded
        ))),
        "{removed} left the wall while queued"
    );
    no_later_start(&mut d, &queued);
}

#[cfg(feature = "js")]
const SETTINGS_CARD: &str = "export const SETTINGS = { target: { type: 'string', default: 'Guard' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

#[cfg(feature = "js")]
/// Load the real JS settings card used by the original copy-identity cases.
fn copy_plan(test: &str) -> TestDriver {
    let dir = scratch(test);
    let mut d = driver_in(&dir, &["alice", "bob", "carol"]);
    let path = dir.join("thiever.ts");
    std::fs::write(&path, SETTINGS_CARD).expect("card file");
    let card = d.models_mut().scripts.js_mut().load(&path).expect("card");
    let card = crate::scripts::ScriptChoice::from_script(script::ScriptSel::Loaded(
        card.source,
        card.identity_id(),
    ));
    load(&mut d, &["alice", "bob", "carol"]);
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    settle(&mut d, select.request());
    mark(&mut d, &["alice", "bob", "carol"]);
    set_card(&mut d, crate::FleetCard::pinned(card));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    assert!(assignment(&d, "bob").is_some());
    d.submit(OperatorCommand::SetMark(false), Targets::Profile("alice"))
        .expect("unmark");
    assert_eq!(plan(&mut d, crate::FleetAction::CopySettings), Some(done()));
    assert_eq!(plan_names(&d), ["bob", "carol"]);
    TestDriver {
        driver: d,
        _scratch: dir,
    }
}

#[cfg(not(feature = "js"))]
/// Load alice, bob and carol, give all a compiled Gatherer card, focus alice
/// and plan a copy of her settings to bob and carol (marked). Binding the
/// selected game data before loading lets native Gatherer settings persist.
fn native_copy_plan(test: &str) -> TestDriver {
    let dir = scratch(test);
    let mut d = driver_in(&dir, &["alice", "bob", "carol"]);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).expect("data");
    d.session_mut()
        .play_mut()
        .expect("play")
        .bind_script_test_data(data);
    load(&mut d, &["alice", "bob", "carol"]);
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    settle(&mut d, select.request());
    mark(&mut d, &["alice", "bob", "carol"]);
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    assert!(assignment(&d, "bob").is_some());
    d.submit(OperatorCommand::SetMark(false), Targets::Profile("alice"))
        .expect("unmark");
    assert_eq!(plan(&mut d, crate::FleetAction::CopySettings), Some(done()));
    assert_eq!(plan_names(&d), ["bob", "carol"]);
    TestDriver {
        driver: d,
        _scratch: dir,
    }
}

#[cfg(feature = "js")]
/// Review F3-1: a profile deleted and recreated under the same name
/// between Plan and Confirm is not written by the copy; the old one is a
/// named skip.
#[test]
fn a_copy_never_writes_a_recreated_profile_with_the_same_name() {
    let mut d = copy_plan("f3-copy-recreate");
    let vault = d.session_mut().vault_mut().expect("vault");
    assert!(vault.remove("bob").expect("remove"));
    vault
        .upsert(Profile {
            username: "bob".into(),
            password: "pw".into(),
            uid: 99,
            settings: ProfileSettings::default(),
        })
        .expect("new bob");
    d.poll();
    let before = d.session().vault().and_then(|v| v.get("bob")).cloned();
    let run = run_plan(&mut d);
    assert_eq!(terminal(&d, run, "carol"), Some(done()));
    assert_eq!(
        d.session().vault().and_then(|v| v.get("bob")).cloned(),
        before,
        "the new bob is untouched"
    );
    assert_eq!(assignment(&d, "bob"), None);
    let result = d.session().fleet_result().expect("result");
    assert!(
        result.text().contains("bob (profile no longer exists)"),
        "{}",
        result.text()
    );
}

#[cfg(feature = "js")]
/// Review F3-1: a target renamed between Plan and Confirm keeps its frozen
/// identity: the copy writes it under its new name and its member is done.
#[test]
fn a_copy_follows_a_renamed_target_by_identity() {
    let mut d = copy_plan("f3-copy-rename");
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
    let run = run_plan(&mut d);
    assert_eq!(terminal(&d, run, "bobby"), Some(done()), "same identity");
    assert_eq!(terminal(&d, run, "carol"), Some(done()));
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().failed, 0, "{}", result.text());
    assert_eq!(result.counts().done, 2, "{}", result.text());
}

#[cfg(feature = "js")]
/// Review F3-1: a save of a target that starts between Plan and Confirm
/// (here a rename the writer still holds) makes it a named skip at the
/// copy's write; it is never a competing write.
#[test]
fn a_copy_skips_a_target_whose_save_started_after_the_plan() {
    let mut d = copy_plan("f3-copy-saving");
    let gate = d.session().write_gate();
    let held = gate.lock().expect("gate");
    let mut bob = d
        .session()
        .vault()
        .and_then(|vault| vault.get("bob"))
        .cloned()
        .expect("bob");
    bob.username = "bobby".into();
    d.session_mut()
        .rename_profile("bob", bob, crate::ArmMirror::none(), "profile")
        .expect("rename staged");
    let command = d.session().fleet_plan().expect("plan").command();
    let run = d.submit(command, Targets::Plan).expect("copy").request();
    assert_eq!(
        terminal(&d, run, "bobby"),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::ProfileSaving
        )))
    );
    drop(held);
    settle(&mut d, run);
    d.session_mut().flush_writes();
    d.poll();
    assert_eq!(terminal(&d, run, "carol"), Some(done()));
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert!(
        result.text().contains("bobby (profile is still saving)"),
        "{}",
        result.text()
    );
}

#[cfg(not(feature = "js"))]
#[test]
fn native_copy_never_writes_a_recreated_profile_with_the_same_name() {
    let mut d = native_copy_plan("native-copy-recreate");
    let vault = d.session_mut().vault_mut().expect("vault");
    assert!(vault.remove("bob").expect("remove"));
    vault
        .upsert(Profile {
            username: "bob".into(),
            password: "pw".into(),
            uid: 99,
            settings: ProfileSettings::default(),
        })
        .expect("new bob");
    d.poll();
    let before = d.session().vault().and_then(|v| v.get("bob")).cloned();
    let run = run_plan(&mut d);
    assert_eq!(terminal(&d, run, "carol"), Some(done()));
    assert_eq!(
        d.session().vault().and_then(|v| v.get("bob")).cloned(),
        before,
        "the new bob is untouched"
    );
    assert_eq!(assignment(&d, "bob"), None);
    let result = d.session().fleet_result().expect("result");
    assert!(
        result.text().contains("bob (profile no longer exists)"),
        "{}",
        result.text()
    );
}

#[cfg(not(feature = "js"))]
#[test]
fn native_copy_skips_a_target_whose_save_started_after_the_plan() {
    let mut d = native_copy_plan("native-copy-saving");
    let gate = d.session().write_gate();
    let held = gate.lock().expect("gate");
    let mut bob = d
        .session()
        .vault()
        .and_then(|vault| vault.get("bob"))
        .cloned()
        .expect("bob");
    bob.username = "bobby".into();
    d.session_mut()
        .rename_profile("bob", bob, crate::ArmMirror::none(), "profile")
        .expect("rename staged");
    let command = d.session().fleet_plan().expect("plan").command();
    let run = d.submit(command, Targets::Plan).expect("copy").request();
    assert_eq!(
        terminal(&d, run, "bobby"),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::ProfileSaving
        )))
    );
    drop(held);
    settle(&mut d, run);
    d.session_mut().flush_writes();
    d.poll();
    assert_eq!(terminal(&d, run, "carol"), Some(done()));
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert!(
        result.text().contains("bobby (profile is still saving)"),
        "{}",
        result.text()
    );
}

/// Load alice, give her a loaded JS card through a plan, then submit her
/// own single Start with no frame after it: the card's preparation is
/// pending (an untallied preparation). Returns the Start command.
#[cfg(feature = "js")]
fn preparing_start(test: &str) -> (TestDriver, RequestId) {
    let dir = scratch(test);
    let mut d = driver_in(&dir, &["alice", "bob"]);
    let path = dir.join("looping.ts");
    std::fs::write(&path, SETTINGS_CARD).expect("card file");
    let card = d.models_mut().scripts.js_mut().load(&path).expect("card");
    let card = crate::scripts::ScriptChoice::from_script(script::ScriptSel::Loaded(
        card.source,
        card.identity_id(),
    ));
    load(&mut d, &["alice"]);
    mark(&mut d, &["alice"]);
    set_card(&mut d, crate::FleetCard::pinned(card));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    set_card(&mut d, crate::FleetCard::OWN);
    let start = d
        .submit(OperatorCommand::StartScript, Targets::Profile("alice"))
        .expect("start")
        .request();
    assert!(
        d.models().scripts().has_start_preparation("alice"),
        "the single Start waits on its card's preparation"
    );
    assert_eq!(terminal(&d, start, "alice"), None, "Pending");
    (
        TestDriver {
            driver: d,
            _scratch: dir,
        },
        start,
    )
}

/// `count` bots on Gatherer, bot000 focused (the copy's source), every
/// other bot marked and a planned native Copy settings waiting.
fn copy_to_many(test: &str, count: usize) -> TestDriver {
    let names: Vec<String> = (0..count).map(|index| format!("bot{index:03}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut d = driver(test, &refs);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).expect("data");
    d.session_mut()
        .play_mut()
        .expect("play")
        .bind_script_test_data(data);
    load(&mut d, &refs);
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("bot000"))
        .expect("select");
    settle(&mut d, select.request());
    mark(&mut d, &refs);
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    d.submit(OperatorCommand::SetMark(false), Targets::Profile("bot000"))
        .expect("unmark");
    assert_eq!(plan(&mut d, crate::FleetAction::CopySettings), Some(done()));
    assert_eq!(plan_names(&d).len(), count - 1);
    d
}

/// Poll as front-end frames do until this request and its fleet result settle.
fn settle_result(d: &mut HeadlessDriver, request: RequestId) {
    wait_for_command(d, request, "the fleet result", |d| {
        d.session()
            .fleet_result()
            .is_some_and(|result| result.request() == Some(request) && result.settled())
    });
}

fn starts_held(d: &HeadlessDriver) -> bool {
    d.session()
        .play()
        .expect("play")
        .script_start_wait_reason()
        .is_some()
}

/// Known issue `fleet-copy-settings-64-limit`, fixed in 0.2.6: a planned
/// native copy to more bots than the bound of pending native saves (64)
/// keeps a bounded number in flight, queues the rest and saves every
/// target; new Starts wait until the last save is accepted.
#[test]
fn a_native_copy_beyond_the_pending_save_bound_saves_every_bot() {
    for count in [67, 151] {
        let mut d = copy_to_many(&format!("f4-copy-{count}"), count);
        let command = d.session().fleet_plan().expect("plan").command();
        let run = d.submit(command, Targets::Plan).expect("run");
        assert!(starts_held(&d), "{count}: Starts wait for the copy");
        let queued = d
            .models()
            .scripts()
            .last_settings_sync()
            .expect("copy report")
            .pending();
        assert_eq!(queued, count - 1, "{count}: every target waits its turn");
        settle_result(&mut d, run.request());
        let result = d.session().fleet_result().expect("result");
        assert_eq!(result.counts().done, count - 1, "{}", result.text());
        assert_eq!(result.counts().failed, 0, "{}", result.text());
        assert_eq!(result.counts().skipped, 0, "{}", result.text());
        assert!(
            !starts_held(&d),
            "{count}: the hold ends with the last save"
        );
    }
}

/// Cancel while targets still wait for room: no waiting target is written
/// (each is reported skipped, "settings copy cancelled"), the saves
/// already started settle, and new Starts are released.
#[test]
fn cancelling_a_queued_copy_writes_no_waiting_bot_and_releases_starts() {
    let mut d = copy_to_many("f4-copy-cancel", 151);
    let command = d.session().fleet_plan().expect("plan").command();
    let run = d.submit(command, Targets::Plan).expect("run");
    d.submit(OperatorCommand::CancelSettingsSync, Targets::Application)
        .expect("cancel");
    settle_result(&mut d, run.request());
    let result = d.session().fleet_result().expect("result");
    let counts = result.counts();
    assert!(counts.skipped >= 150 - 48, "{}", result.text());
    assert!(counts.done + counts.failed <= 48, "{}", result.text());
    assert_eq!(counts.done + counts.failed + counts.skipped, 150);
    assert!(
        result.text().contains("(settings copy cancelled)"),
        "{}",
        result.text()
    );
    assert!(!starts_held(&d), "Cancel releases the start hold");
}

#[cfg(feature = "js")]
/// Review F3 r2-1: a fleet Start joins a single Start's untallied
/// preparation; the assignment then changes while it waits. The
/// preparation is withdrawn `reassigned`, and both Start commands settle
/// (neither stays Pending), and nothing starts.
#[test]
fn a_withdrawn_shared_preparation_settles_the_single_and_the_fleet_start() {
    let (mut d, single) = preparing_start("f3-shared-preparation");
    assert_eq!(plan(&mut d, crate::FleetAction::Start), Some(done()));
    let command = d.session().fleet_plan().expect("plan").command();
    let fleet = d.submit(command, Targets::Plan).expect("fleet").request();
    assert_eq!(terminal(&d, fleet, "alice"), None, "joined the preparation");
    let vault = d.session_mut().vault_mut().expect("vault");
    let mut alice = vault.get("alice").cloned().expect("alice");
    alice.settings.script_assignment = None;
    vault.upsert(alice).expect("reassign");
    settle(&mut d, single);
    settle(&mut d, fleet);
    let reassigned = Some(Settlement::Completed(Completion::Skipped(
        SkipReason::NotReady,
    )));
    assert_eq!(terminal(&d, single, "alice"), reassigned, "single Start");
    assert_eq!(terminal(&d, fleet, "alice"), reassigned, "fleet Start");
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    no_later_start(&mut d, &["alice".to_owned()]);
    assert_eq!(
        d.session().script_state("alice"),
        crate::scripts::ScriptRunState::Idle
    );
}

#[cfg(feature = "js")]
/// Review F3 r2-1: a planned Stop on a bot whose single Start still waits
/// on its loaded card's preparation cancels that preparation: the Stop
/// member is done, the Start member cancelled, and nothing starts later.
#[test]
fn a_planned_stop_cancels_a_held_loaded_card_preparation() {
    let (mut d, start) = preparing_start("f3-stop-preparation");
    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    assert_eq!(
        plan_names(&d),
        ["alice"],
        "a preparing Start is a Stop target"
    );
    let stop = run_plan(&mut d);
    assert_eq!(terminal(&d, stop, "alice"), Some(done()), "Stop");
    assert_eq!(
        terminal(&d, start, "alice"),
        Some(operator_cancelled()),
        "Start"
    );
    no_later_start(&mut d, &["alice".to_owned()]);
    assert_eq!(
        d.session().script_state("alice"),
        crate::scripts::ScriptRunState::Idle
    );
}

/// Review F3-4: a plan whose every target is gone at Confirm ends at once
/// as a terminal result naming each skip; nothing waits.
#[test]
fn a_plan_whose_last_target_is_gone_ends_with_named_skips() {
    let mut d = driver("f3-emptied", &["alice", "bob"]);
    d.poll();
    mark(&mut d, &["bob"]);
    set_card(&mut d, crate::FleetCard::pinned(gatherer()));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    delete_profile(&mut d, "bob");
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan)
        .expect("an emptied plan ends");
    assert!(d.session().fleet_plan().is_none(), "the plan ended");
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.request(), None, "no command record");
    assert_eq!(
        result.text(),
        "Assign: skipped 1: bob (profile no longer exists)"
    );
}

#[cfg(feature = "js")]
/// Review F3-4: an emptied copy plan gives back its prepared scope (and
/// with it the start hold) and writes nothing.
#[test]
fn an_emptied_copy_plan_releases_its_scope() {
    let mut d = copy_plan("f3-emptied-copy");
    let vault = d.session_mut().vault_mut().expect("vault");
    assert!(vault.remove("bob").expect("remove bob"));
    assert!(vault.remove("carol").expect("remove carol"));
    d.poll();
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan)
        .expect("an emptied plan ends");
    assert!(
        d.models().scripts().prepared_settings_sync().is_none(),
        "the prepared scope is released"
    );
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().skipped, 2, "{}", result.text());
}

#[cfg(not(feature = "js"))]
#[test]
fn an_emptied_native_copy_plan_releases_its_scope_and_start_hold() {
    let mut d = native_copy_plan("native-emptied-copy");
    assert!(d
        .session()
        .play()
        .expect("play")
        .script_start_wait_reason()
        .is_some());
    let vault = d.session_mut().vault_mut().expect("vault");
    assert!(vault.remove("bob").expect("remove bob"));
    assert!(vault.remove("carol").expect("remove carol"));
    d.poll();
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan)
        .expect("an emptied plan ends");
    assert!(
        d.models().scripts().prepared_settings_sync().is_none(),
        "the prepared scope is released"
    );
    assert!(d
        .session()
        .play()
        .expect("play")
        .script_start_wait_reason()
        .is_none());
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().skipped, 2, "{}", result.text());
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
    let mut again = open_driver(&dir);
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
    let mut again = open_driver(&dir);
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
    let mut again = open_driver(&dir);
    again.session_mut().set_spawn_workers(false);
    again
        .unlock(dir.join("vault"), PASSPHRASE, &play())
        .expect("unlock");
    again.poll();
    assert_eq!(order_names(&again), saved, "after a restart");
}

// --- Script-paint host controls (PAINT-CONTROLS-025) ---

use crate::scripts::{PaintControl, PaintFrame, ScriptRunState};
use crate::test_support::paint::TestPaint;

/// Hang guard for the script lifecycle waits below; the pass condition is
/// the slot's own state.
const PAINT_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(120);

/// The frame the standard paint-controls helper records (`gobank` is one
/// ordinary script button beside it), stamped with `generation`.
fn helper_paint(generation: u64) -> TestPaint {
    TestPaint::new(Some("Helper"), &["status"])
        .with_button("gobank", "Go bank")
        .with_host_controls()
        .with_generation(generation)
}

fn wait_state(d: &mut HeadlessDriver, name: &str, want: ScriptRunState) {
    let guard = std::time::Instant::now() + PAINT_HANG_GUARD;
    while d.session().script_state(name) != want {
        assert!(std::time::Instant::now() < guard, "{name}: never {want:?}");
        if let Some(play) = d.session().play() {
            play.pump_script_lifecycle(name);
        }
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Start the assigned native script on `name` and publish the helper's
/// frame on its status row, as the slot's observe does after a tick.
#[cfg(not(feature = "js"))]
fn run_compiled_helper(d: &mut HeadlessDriver, name: &str, generation: u64) {
    let start = d
        .submit(OperatorCommand::StartScript, Targets::Profile(name))
        .expect("start");
    settle(d, start.request());
    wait_state(d, name, ScriptRunState::Running);
    helper_paint(generation).publish(d.session().play().expect("play"), name);
    d.poll();
}

/// Start a real Load script on `name` and publish the helper's frame on its
/// status row, as the slot's observe does after a tick.
#[cfg(feature = "js")]
fn run_helper(d: &mut HeadlessDriver, name: &str, generation: u64) {
    d.session()
        .play()
        .expect("play")
        .script_start_load(
            name,
            "export function tick(api) { api._n = (api._n || 0) + 1 }".into(),
            script_js::LoadShape::NativeTick,
            None,
            vec![],
        )
        .expect("start");
    wait_state(d, name, ScriptRunState::Running);
    helper_paint(generation).publish(d.session().play().expect("play"), name);
    d.poll();
}

#[cfg(not(feature = "js"))]
fn run_helper(d: &mut HeadlessDriver, name: &str, generation: u64) {
    run_compiled_helper(d, name, generation);
}

#[cfg(not(feature = "js"))]
fn paint_driver_with_card(
    test: &str,
    names: &[&str],
    card: crate::scripts::ScriptChoice,
) -> TestDriver {
    let mut d = driver(test, names);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).expect("data");
    d.session_mut()
        .play_mut()
        .expect("play")
        .bind_script_test_data(data);
    d.submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load");
    mark(&mut d, names);
    set_card(&mut d, crate::FleetCard::pinned(card));
    assert_eq!(plan(&mut d, crate::FleetAction::Assign), Some(done()));
    run_plan(&mut d);
    for (i, name) in names.iter().enumerate() {
        run_compiled_helper(&mut d, name, 40 + i as u64);
    }
    d
}
fn paint_driver(test: &str, names: &[&str]) -> TestDriver {
    #[cfg(feature = "js")]
    {
        let mut d = driver(test, names);
        d.submit(OperatorCommand::Load, Targets::Everyone)
            .expect("load");
        for (i, name) in names.iter().enumerate() {
            run_helper(&mut d, name, 40 + i as u64);
        }
        d
    }
    #[cfg(not(feature = "js"))]
    {
        paint_driver_with_card(test, names, gatherer())
    }
}

/// `name`'s displayed frame, as a front end reads it each frame.
fn shown(d: &HeadlessDriver, name: &str) -> PaintFrame {
    d.session().script_paint(name).expect("displayed paint")
}

/// The labels the front ends draw for `frame`.
fn labels(frame: &PaintFrame) -> Vec<(String, bool)> {
    frame
        .view()
        .buttons()
        .map(|button| (button.label().to_owned(), button.enabled()))
        .collect()
}

/// Activate button `index` of `frame` and submit it for `target`, as the
/// panel's overlay and the TUI's chat pane do.
fn activate(
    d: &mut HeadlessDriver,
    frame: &PaintFrame,
    index: usize,
    target: &str,
) -> Option<Submitted> {
    let input = frame.view().activate(index)?;
    d.models_mut().scripts_mut().stage_paint_input(input);
    Some(
        d.submit(OperatorCommand::ScriptPaintInput, Targets::Profile(target))
            .expect("paint input admitted"),
    )
}

const PAUSE: usize = 1;
const STOP: usize = 2;

fn command_of(d: &HeadlessDriver, submitted: &Submitted) -> OperatorCommand {
    d.report(submitted.request()).expect("report").command()
}

fn refused(d: &HeadlessDriver, submitted: &Submitted, name: &str) -> bool {
    command_of(d, submitted) == OperatorCommand::ScriptPaintInput
        && matches!(
            terminal(d, submitted.request(), name),
            Some(Settlement::Failed(CommandFailure::Rejected(
                crate::command::Rejection::NotApplicable
            )))
        )
}

#[test]
fn paint_controls_pause_resume_and_stop_the_displayed_run_through_the_lifecycle_rows() {
    let mut d = paint_driver("paint-controls", &["alice"]);
    let running = shown(&d, "alice");
    assert_eq!(
        labels(&running),
        vec![
            ("Go bank".to_string(), true),
            ("Pause".to_string(), true),
            ("Stop".to_string(), true),
        ]
    );
    assert_eq!(
        running
            .view()
            .buttons()
            .nth(PAUSE)
            .and_then(|b| b.control()),
        Some(PaintControl::Pause)
    );

    let pause = activate(&mut d, &running, PAUSE, "alice").expect("pause applies");
    assert_eq!(command_of(&d, &pause), OperatorCommand::PauseScript);
    settle(&mut d, pause.request());
    assert_eq!(terminal(&d, pause.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Paused);

    // The retained frame is not republished while paused; it shows Resume.
    let paused = shown(&d, "alice");
    assert_eq!(paused.view().generation(), running.view().generation());
    assert_eq!(
        labels(&paused)[PAUSE..],
        [("Resume".to_string(), true), ("Stop".to_string(), true)]
    );
    let resume = activate(&mut d, &paused, PAUSE, "alice").expect("resume applies");
    assert_eq!(command_of(&d, &resume), OperatorCommand::ResumeScript);
    settle(&mut d, resume.request());
    assert_eq!(terminal(&d, resume.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);

    // Stop from Paused reaches the normal terminal state.
    let resumed = shown(&d, "alice");
    let again = activate(&mut d, &resumed, PAUSE, "alice").expect("pause");
    settle(&mut d, again.request());
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Paused);
    let stop_frame = shown(&d, "alice");
    let stop = activate(&mut d, &stop_frame, STOP, "alice").expect("stop applies");
    assert_eq!(command_of(&d, &stop), OperatorCommand::StopScript);
    settle(&mut d, stop.request());
    assert_eq!(terminal(&d, stop.request(), "alice"), Some(done()));
    wait_state(&mut d, "alice", ScriptRunState::Idle);
    assert!(
        d.session().script_paint("alice").is_none(),
        "Stop clears the published paint"
    );
    // A late control from the stopped run's frame controls nothing.
    let late = activate(&mut d, &stop_frame, STOP, "alice").expect("staged");
    assert!(refused(&d, &late, "alice"), "late Stop is refused");
}

#[test]
fn paint_stop_from_running_reaches_idle_with_one_stop_report() {
    let mut d = paint_driver("paint-stop-running", &["alice"]);
    let frame = shown(&d, "alice");
    let stop = activate(&mut d, &frame, STOP, "alice").expect("stop applies");
    let second = activate(&mut d, &frame, STOP, "alice").expect("staged");
    assert_eq!(command_of(&d, &stop), OperatorCommand::StopScript);
    assert!(
        refused(&d, &second, "alice"),
        "a double Stop opens no second Stop report"
    );
    settle(&mut d, stop.request());
    assert_eq!(terminal(&d, stop.request(), "alice"), Some(done()));
    wait_state(&mut d, "alice", ScriptRunState::Idle);
}

#[test]
fn a_double_pause_never_becomes_a_resume() {
    let mut d = paint_driver("paint-double-pause", &["alice"]);
    let frame = shown(&d, "alice");
    let first = activate(&mut d, &frame, PAUSE, "alice").expect("pause");
    let second = activate(&mut d, &frame, PAUSE, "alice").expect("staged");
    settle(&mut d, first.request());
    assert_eq!(command_of(&d, &first), OperatorCommand::PauseScript);
    assert!(refused(&d, &second, "alice"), "the stale Pause is refused");
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Paused);
}

#[test]
fn a_control_shown_for_one_profile_never_controls_the_newly_focused_one() {
    let mut d = paint_driver("paint-focus", &["alice", "bob"]);
    let alice = shown(&d, "alice");
    for index in [PAUSE, STOP] {
        let submitted = activate(&mut d, &alice, index, "bob").expect("staged");
        assert!(refused(&d, &submitted, "bob"), "control {index} refused");
    }
    assert_eq!(d.session().script_state("bob"), ScriptRunState::Running);
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);
}

#[test]
fn a_control_from_an_ended_run_never_controls_its_replacement() {
    let mut d = paint_driver("paint-replacement", &["alice"]);
    let old = shown(&d, "alice");
    let generation = old.view().generation();
    d.session().play().expect("play").script_stop("alice");
    wait_state(&mut d, "alice", ScriptRunState::Idle);
    // The replacement advertises the same ids under the same (recycled)
    // frame generation.
    run_helper(&mut d, "alice", generation);
    assert_eq!(shown(&d, "alice").view().generation(), generation);
    for index in [PAUSE, STOP] {
        let submitted = activate(&mut d, &old, index, "alice").expect("staged");
        assert!(refused(&d, &submitted, "alice"), "control {index} refused");
    }
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);
}

#[test]
fn a_stale_frame_generation_is_refused_before_dispatch() {
    let mut d = paint_driver("paint-stale-frame", &["alice"]);
    let old = shown(&d, "alice");
    helper_paint(old.view().generation() + 1).publish(d.session().play().expect("play"), "alice");
    d.poll();
    let submitted = activate(&mut d, &old, PAUSE, "alice").expect("staged");
    assert!(refused(&d, &submitted, "alice"));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);
}

#[test]
fn an_ordinary_paint_button_keeps_its_script_click_path() {
    let mut d = paint_driver("paint-ordinary", &["alice"]);
    let frame = shown(&d, "alice");
    let click = activate(&mut d, &frame, 0, "alice").expect("script click");
    assert_eq!(command_of(&d, &click), OperatorCommand::ScriptPaintInput);
    assert_eq!(terminal(&d, click.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);
    // A host id staged as a script click never reaches the mailbox.
    d.models_mut()
        .scripts_mut()
        .stage_paint_input(crate::scripts::PaintInput::click(
            script::shim::HOST_PAUSE_CONTROL_ID.into(),
            frame.view().generation(),
        ));
    let forged = d
        .submit(OperatorCommand::ScriptPaintInput, Targets::Profile("alice"))
        .expect("admitted");
    assert!(refused(&d, &forged, "alice"));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);
}

/// Rows 10 and 11 of the plan: the controls follow the run lifecycle, not a
/// native status phase. A running card whose status reads Blocked keeps
/// Pause and Stop; an idle slot that kept a terminal Blocked detail offers
/// neither and activates nothing.
#[test]
fn paint_controls_follow_the_lifecycle_state() {
    let paint =
        TestPaint::new(Some("Gatherer — Blocked"), &["blocked: no supply"]).with_host_controls();
    let expect = |state, pause: (&str, bool), stop: bool| {
        let frame = paint.frame_for_run("alice", state);
        assert_eq!(
            labels(&frame),
            vec![(pause.0.to_string(), pause.1), ("Stop".to_string(), stop)],
            "{state:?}"
        );
        assert_eq!(frame.view().activate(0).is_some(), pause.1, "{state:?}");
        assert_eq!(frame.view().activate(1).is_some(), stop, "{state:?}");
    };
    expect(ScriptRunState::Running, ("Pause", true), true);
    expect(ScriptRunState::Starting, ("Pause", true), true);
    expect(ScriptRunState::Paused, ("Resume", true), true);
    expect(ScriptRunState::Stopping, ("Pause", false), false);
    expect(ScriptRunState::Idle, ("Pause", false), false);
    expect(ScriptRunState::Error, ("Pause", false), false);
    // A frame read without a run (no host) offers no control.
    assert!(paint.frame().view().activate(0).is_none());
}

/// Publish a native paint frame on `name`'s status row, as the slot's
/// observe does after a native script publishes it.
fn publish_native_paint(
    d: &HeadlessDriver,
    name: &str,
    paint: std::sync::Arc<script::shim::ScriptPaint>,
) {
    let play = d.session().play().expect("play");
    let mut rows = play.statuses.lock().expect("status rows");
    let row = rows
        .iter_mut()
        .find(|row| row.username == name)
        .expect("status row");
    row.script_paint = Some(paint);
}

/// Press Pause, Resume and Stop on one native paint frame (its two
/// host controls are buttons 0 and 1) and check each press through
/// the lifecycle rows, as the front ends submit them.
#[cfg(feature = "js")]
fn native_paint_controls_case(test: &str, paint: std::sync::Arc<script::shim::ScriptPaint>) {
    let mut d = paint_driver(test, &["alice"]);
    publish_native_paint(&d, "alice", paint);
    d.poll();
    let running = shown(&d, "alice");
    assert_eq!(
        labels(&running),
        vec![("Pause".to_string(), true), ("Stop".to_string(), true)],
        "the native paint shows Pause and Stop"
    );
    let pause = activate(&mut d, &running, 0, "alice").expect("pause applies");
    assert_eq!(command_of(&d, &pause), OperatorCommand::PauseScript);
    settle(&mut d, pause.request());
    assert_eq!(terminal(&d, pause.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Paused);

    let paused = shown(&d, "alice");
    assert_eq!(
        labels(&paused),
        vec![("Resume".to_string(), true), ("Stop".to_string(), true)],
        "the retained native paint shows Resume while paused"
    );
    let resume = activate(&mut d, &paused, 0, "alice").expect("resume applies");
    assert_eq!(command_of(&d, &resume), OperatorCommand::ResumeScript);
    settle(&mut d, resume.request());
    assert_eq!(terminal(&d, resume.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);

    let again = shown(&d, "alice");
    let stop = activate(&mut d, &again, 1, "alice").expect("stop applies");
    assert_eq!(command_of(&d, &stop), OperatorCommand::StopScript);
    settle(&mut d, stop.request());
    assert_eq!(terminal(&d, stop.request(), "alice"), Some(done()));
    wait_state(&mut d, "alice", ScriptRunState::Idle);
    assert!(
        d.session().script_paint("alice").is_none(),
        "Stop clears the published native paint"
    );
}

#[cfg(not(feature = "js"))]
fn compiled_paint_controls_case(
    test: &str,
    card: crate::scripts::ScriptChoice,
    paint: std::sync::Arc<script::shim::ScriptPaint>,
) {
    let mut d = paint_driver_with_card(test, &["alice"], card);
    d.poll();
    publish_native_paint(&d, "alice", paint);
    d.poll();
    let running = shown(&d, "alice");
    assert_eq!(
        labels(&running),
        vec![("Pause".to_string(), true), ("Stop".to_string(), true)],
        "the native paint shows Pause and Stop"
    );
    let pause = activate(&mut d, &running, 0, "alice").expect("pause applies");
    assert_eq!(command_of(&d, &pause), OperatorCommand::PauseScript);
    settle(&mut d, pause.request());
    assert_eq!(terminal(&d, pause.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Paused);

    let paused = shown(&d, "alice");
    assert_eq!(
        labels(&paused),
        vec![("Resume".to_string(), true), ("Stop".to_string(), true)],
        "the retained native paint shows Resume while paused"
    );
    let resume = activate(&mut d, &paused, 0, "alice").expect("resume applies");
    assert_eq!(command_of(&d, &resume), OperatorCommand::ResumeScript);
    settle(&mut d, resume.request());
    assert_eq!(terminal(&d, resume.request(), "alice"), Some(done()));
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);

    let again = shown(&d, "alice");
    let stop = activate(&mut d, &again, 1, "alice").expect("stop applies");
    assert_eq!(command_of(&d, &stop), OperatorCommand::StopScript);
    settle(&mut d, stop.request());
    assert_eq!(terminal(&d, stop.request(), "alice"), Some(done()));
    wait_state(&mut d, "alice", ScriptRunState::Idle);
    assert!(
        d.session().script_paint("alice").is_none(),
        "Stop clears the published native paint"
    );
}

/// L5: the real native Gatherer paint frame, published on a running
/// slot, pauses, resumes and stops that run through the lifecycle
/// rows.
#[cfg(not(feature = "js"))]
#[test]
fn compiled_gatherer_paint_controls_pause_resume_and_stop() {
    use std::sync::Arc;
    let data = script::gatherer::status::StatusData {
        skill: "Woodcutting",
        method: Arc::from("normal"),
        phase: "gathering",
        area: Arc::from("Start"),
        target: Arc::from("Tree"),
        tool: Arc::from("Bronze axe"),
        bait: 0,
        food: 0,
        coins: 0,
        yielded: 0,
        dropped: 0,
        deposited: 0,
        trips: 0,
        xp: 0,
        xp_per_hour: None,
        bank: Arc::from(""),
        last_progress: 0,
        deaths: 0,
        recoveries: 0,
        recovery_step: 0,
        absent: 0,
        zone_gated: 0,
        excluded_targets: Arc::from(""),
        last_event: Arc::from(""),
    };
    compiled_paint_controls_case(
        "native-gatherer-paint",
        crate::scripts::ScriptChoice::compiled(
            crate::scripts::CompiledCardId::lookup("Gatherer").expect("Gatherer is compiled"),
        ),
        script::gatherer::status::paint(&data),
    );
}

/// The native Gatherer frame retains its original JS-backed lifecycle fixture.
#[cfg(feature = "js")]
#[test]
fn native_gatherer_paint_controls_pause_resume_and_stop() {
    use std::sync::Arc;
    let data = script::gatherer::status::StatusData {
        skill: "Woodcutting",
        method: Arc::from("normal"),
        phase: "gathering",
        area: Arc::from("Start"),
        target: Arc::from("Tree"),
        tool: Arc::from("Bronze axe"),
        bait: 0,
        food: 0,
        coins: 0,
        yielded: 0,
        dropped: 0,
        deposited: 0,
        trips: 0,
        xp: 0,
        xp_per_hour: None,
        bank: Arc::from(""),
        last_progress: 0,
        deaths: 0,
        recoveries: 0,
        recovery_step: 0,
        absent: 0,
        zone_gated: 0,
        excluded_targets: Arc::from(""),
        last_event: Arc::from(""),
    };
    native_paint_controls_case(
        "native-gatherer-paint",
        script::gatherer::status::paint(&data),
    );
}

/// L5: the frame the native Sherlock publishes (its card title, its
/// status lines, the host controls, and the run number it stamps as
/// the frame generation), published on a running slot, pauses,
/// resumes and stops that run the same way.
#[cfg(feature = "js")]
#[test]
fn native_sherlock_paint_controls_pause_resume_and_stop() {
    let paint = std::sync::Arc::new(script::shim::ScriptPaint {
        title: Some("Sherlock".into()),
        lines: vec!["Waiting for a clue".into(), "Clues solved: 0".into()],
        buttons: script::shim::host_control_buttons(),
        generation: 7,
        ..Default::default()
    });
    native_paint_controls_case("native-sherlock-paint", paint);
}

/// Every place a Start can wait before its slot runs.
#[derive(Clone, Copy, Debug)]
enum Waiting {
    /// A paced place behind another bot's Start (fleet Start).
    Paced,
    /// A fleet Assign & restart waiting for its compiled assignment write.
    #[cfg(not(feature = "js"))]
    CompiledRestartAfterWrite,
    /// A loaded card's preparation for a single Start (untallied).
    #[cfg(feature = "js")]
    Preparation,
    /// A loaded card's preparation for a fleet Start (tallied).
    #[cfg(feature = "js")]
    FleetPreparation,
    /// A fleet Assign & restart of a catalog card waiting for the catalog
    /// scan (nothing written yet).
    #[cfg(feature = "js")]
    CatalogRestart,
    /// A fleet Assign & restart waiting for its loaded assignment write.
    #[cfg(feature = "js")]
    RestartAfterWrite,
}

/// The three ways an operator withdraws a waiting Start.
#[derive(Clone, Copy, Debug)]
enum Withdraw {
    PlannedStop,
    StopEveryone,
    Cancel,
}

/// Planned Stop, Stop Everyone and explicit Cancel reach the native paced
/// Start and the native Assign & restart waiting behind its write.
#[cfg(not(feature = "js"))]
#[test]
fn every_native_waiting_start_is_withdrawn_by_stop_stop_everyone_and_cancel() {
    for waiting in [Waiting::Paced, Waiting::CompiledRestartAfterWrite] {
        for withdraw in [
            Withdraw::PlannedStop,
            Withdraw::StopEveryone,
            Withdraw::Cancel,
        ] {
            withdraw_case(waiting, withdraw);
        }
    }
}

/// Review F3 r3 (shadow High and sol61 1): planned Stop, Stop Everyone and
/// an explicit Cancel each reach every place a Start waits. The original
/// command's member settles cancelled, the Stop or Cancel member is done,
/// and nothing starts later. A catalog Assign & restart withdrawn before
/// the scan writes nothing; one withdrawn behind its write keeps the
/// assignment and drops only the Start.
#[cfg(feature = "js")]
#[test]
fn every_waiting_start_is_withdrawn_by_stop_stop_everyone_and_cancel() {
    for waiting in [
        Waiting::Paced,
        Waiting::Preparation,
        Waiting::FleetPreparation,
        Waiting::CatalogRestart,
        Waiting::RestartAfterWrite,
    ] {
        for withdraw in [
            Withdraw::PlannedStop,
            Withdraw::StopEveryone,
            Withdraw::Cancel,
        ] {
            withdraw_case(waiting, withdraw);
        }
    }
}
fn withdraw_case(waiting: Waiting, withdraw: Withdraw) {
    let case = format!("{waiting:?}/{withdraw:?}");
    let dir = scratch(&format!("f3-withdraw-{waiting:?}-{withdraw:?}").to_lowercase());
    let mut d = driver_in(&dir, &["alice", "bob"]);
    #[cfg(feature = "js")]
    let path = dir.join("looping.ts");
    #[cfg(feature = "js")]
    std::fs::write(&path, SETTINGS_CARD).expect("card file");
    #[cfg(feature = "js")]
    let js = d.models_mut().scripts.js_mut().load(&path).expect("card");
    #[cfg(feature = "js")]
    let js = crate::scripts::ScriptChoice::from_script(script::ScriptSel::Loaded(
        js.source,
        js.identity_id(),
    ));
    #[cfg(feature = "js")]
    let assign = |d: &mut HeadlessDriver, card: crate::scripts::ScriptChoice| {
        set_card(d, crate::FleetCard::pinned(card));
        assert_eq!(plan(d, crate::FleetAction::Assign), Some(done()));
        run_plan(d);
    };
    let gate = d.session().write_gate();
    let mut held = None;
    let (target, original) = match waiting {
        Waiting::Paced => {
            let (start, queued) = queued_start(&mut d, &["alice", "bob"]);
            (queued[0].clone(), start)
        }
        #[cfg(not(feature = "js"))]
        Waiting::CompiledRestartAfterWrite => {
            load(&mut d, &["alice"]);
            mark(&mut d, &["alice"]);
            set_card(&mut d, crate::FleetCard::pinned(gatherer()));
            held = Some(gate.lock().expect("gate"));
            assert_eq!(
                plan(&mut d, crate::FleetAction::AssignRestart),
                Some(done())
            );
            let command = d.session().fleet_plan().expect("plan").command();
            let restart = d.submit(command, Targets::Plan).expect("restart").request();
            ("alice".to_owned(), restart)
        }
        #[cfg(feature = "js")]
        Waiting::Preparation => {
            load(&mut d, &["alice"]);
            mark(&mut d, &["alice"]);
            assign(&mut d, js);
            let start = d
                .submit(OperatorCommand::StartScript, Targets::Profile("alice"))
                .expect("start")
                .request();
            ("alice".to_owned(), start)
        }
        #[cfg(feature = "js")]
        Waiting::FleetPreparation => {
            load(&mut d, &["alice"]);
            mark(&mut d, &["alice"]);
            assign(&mut d, js);
            set_card(&mut d, crate::FleetCard::OWN);
            assert_eq!(plan(&mut d, crate::FleetAction::Start), Some(done()));
            let command = d.session().fleet_plan().expect("plan").command();
            let start = d.submit(command, Targets::Plan).expect("start").request();
            ("alice".to_owned(), start)
        }
        #[cfg(feature = "js")]
        Waiting::CatalogRestart => {
            let root = dir.join("catalog-root");
            let scripts = root.join("src/bot/scripts");
            let card_dir = scripts.join("BoneBurier");
            std::fs::create_dir_all(&card_dir).expect("catalog");
            std::fs::write(
                scripts.join("index.ts"),
                "import BoneBurier from './BoneBurier/BoneBurier.js';\n\
                 ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });\n",
            )
            .expect("index");
            std::fs::write(card_dir.join("BoneBurier.ts"), SETTINGS_CARD).expect("card");
            load(&mut d, &["alice"]);
            mark(&mut d, &["alice"]);
            set_card(
                &mut d,
                crate::FleetCard::pinned(crate::scripts::ScriptChoice::from_script(
                    script::ScriptSel::Loaded(script::ScriptSource::Catalog, "BoneBurier".into()),
                )),
            );
            d.models_mut().scripts.stage_start_catalog_root(Some(root));
            assert_eq!(
                plan(&mut d, crate::FleetAction::AssignRestart),
                Some(done())
            );
            let command = d.session().fleet_plan().expect("plan").command();
            let restart = d.submit(command, Targets::Plan).expect("restart").request();
            ("alice".to_owned(), restart)
        }
        #[cfg(feature = "js")]
        Waiting::RestartAfterWrite => {
            load(&mut d, &["alice"]);
            mark(&mut d, &["alice"]);
            set_card(&mut d, crate::FleetCard::pinned(js));
            held = Some(gate.lock().expect("gate"));
            assert_eq!(
                plan(&mut d, crate::FleetAction::AssignRestart),
                Some(done())
            );
            let command = d.session().fleet_plan().expect("plan").command();
            let restart = d.submit(command, Targets::Plan).expect("restart").request();
            ("alice".to_owned(), restart)
        }
    };
    assert!(
        d.models().scripts().start_waiting(&target),
        "{case}: {target}'s Start waits"
    );
    assert_eq!(terminal(&d, original, &target), None, "{case}: Pending");
    let _ = d.models_mut().scripts.take_start_failures();
    match withdraw {
        Withdraw::PlannedStop => {
            d.submit(OperatorCommand::ClearMarks, Targets::Application)
                .expect("clear marks");
            mark(&mut d, &[target.as_str()]);
            assert_eq!(
                plan(&mut d, crate::FleetAction::Stop),
                Some(done()),
                "{case}: a waiting Start is a Stop target"
            );
            let command = d.session().fleet_plan().expect("plan").command();
            let stop = d.submit(command, Targets::Plan).expect("stop").request();
            assert_eq!(terminal(&d, stop, &target), Some(done()), "{case}: Stop");
        }
        Withdraw::StopEveryone => {
            d.submit(OperatorCommand::StopScript, Targets::Everyone)
                .expect("stop everyone");
        }
        Withdraw::Cancel => {
            let cancel = d
                .submit(
                    OperatorCommand::CancelQueuedStart,
                    Targets::Profile(&target),
                )
                .expect("cancel")
                .request();
            assert_eq!(
                terminal(&d, cancel, &target),
                Some(done()),
                "{case}: Cancel"
            );
        }
    }
    assert!(
        !d.models().scripts().start_waiting(&target),
        "{case}: nothing waits after the withdrawal"
    );
    drop(held);
    #[cfg(feature = "js")]
    for _ in 0..400 {
        d.poll();
        if terminal(&d, original, &target).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    #[cfg(not(feature = "js"))]
    wait_for_command(&mut d, original, "the cancelled member's terminal", |d| {
        terminal(d, original, &target).is_some()
    });
    assert_eq!(
        terminal(&d, original, &target),
        Some(operator_cancelled()),
        "{case}: the original command settles cancelled"
    );
    d.session_mut().flush_writes();
    #[cfg(feature = "js")]
    let deadline = std::time::Instant::now() + SETTLE_HANG_GUARD;
    #[cfg(feature = "js")]
    while d.models().scripts().catalog_work_pending() {
        assert!(
            std::time::Instant::now() < deadline,
            "{case}: catalog scan hung"
        );
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    no_later_start(&mut d, std::slice::from_ref(&target));
    assert_eq!(
        d.session().script_state(&target),
        crate::scripts::ScriptRunState::Idle,
        "{case}"
    );
    match waiting {
        #[cfg(feature = "js")]
        Waiting::CatalogRestart => {
            assert_eq!(assignment(&d, &target), None, "{case}: nothing written")
        }
        #[cfg(not(feature = "js"))]
        Waiting::CompiledRestartAfterWrite => {
            assert!(
                assignment(&d, &target).is_some(),
                "{case}: the write is kept"
            )
        }
        #[cfg(feature = "js")]
        Waiting::RestartAfterWrite => {
            assert!(
                assignment(&d, &target).is_some(),
                "{case}: the write is kept"
            )
        }
        #[cfg(feature = "js")]
        _ => {}
        #[cfg(not(feature = "js"))]
        Waiting::Paced => {}
    }
}

// ---- F4: Switch world through the fleet plan ----

fn arm_of(d: &HeadlessDriver, name: &str) -> std::sync::Arc<host_play::SlotArm> {
    d.session()
        .play()
        .and_then(|play| play.arm(name))
        .expect("live arm")
}

/// Publish `name`'s status row as its slot would, then two frames: the
/// fleet result follows a member settled by the first on the next.
fn publish(d: &mut HeadlessDriver, name: &str, edit: impl FnOnce(&mut host_play::SlotStatus)) {
    {
        let play = d.session().play().expect("play");
        let mut rows = play.statuses.lock().unwrap();
        if !rows.iter().any(|row| row.username == name) {
            rows.push(host_play::SlotStatus {
                username: name.into(),
                ..host_play::SlotStatus::default()
            });
        }
        edit(
            rows.iter_mut()
                .find(|row| row.username == name)
                .expect("row"),
        );
    }
    d.poll();
    d.poll();
}

/// A finished login: a new owner session for `name` on `world`, its scene
/// ready, and the status row the slot publishes for it.
fn log_in_on(d: &mut HeadlessDriver, name: &str, world: u16) {
    let arm = arm_of(d, name);
    if !arm.login_wanted() {
        arm.arm_explicit_login();
    }
    arm.configure_world_for_test(world);
    arm.complete_login_for_test();
    arm.scene_ready_for_test();
    publish(d, name, |row| {
        row.connected = true;
        row.ingame = true;
        row.scene_state = 2;
        row.world = Some(world);
        row.startup_phase = host_play::StartupPhase::Ready;
        row.error = None;
    });
}

/// A driver whose server lists public worlds 1-3, with `names` loaded and
/// in game on world 1, over a scratch the caller already holds.
fn world_driver_in(dir: &Scratch, names: &[&str]) -> HeadlessDriver {
    let mut d = driver_in(dir, names);
    d.session_mut().set_public_worlds_for_test(&[1, 2, 3]);
    load(&mut d, names);
    for name in names {
        log_in_on(&mut d, name, 1);
    }
    d
}

/// A driver whose server lists public worlds 1-3, with `names` loaded and
/// in game on world 1, owning its scratch home for the whole test.
fn world_driver(test: &str, names: &[&str]) -> TestDriver {
    let scratch = scratch(test);
    let driver = world_driver_in(&scratch, names);
    TestDriver {
        driver,
        _scratch: scratch,
    }
}

/// Plan Switch world (for `target` alone when given): the plan command's
/// terminal and its refusal text.
fn plan_switch(
    d: &mut HeadlessDriver,
    world: u16,
    target: Option<&str>,
) -> (Option<Terminal>, Option<String>) {
    if let Some(name) = target {
        d.session_mut().stage_plan_target(name);
    }
    let planned = d
        .submit(
            OperatorCommand::PlanFleetAction(crate::FleetAction::SwitchWorld(world)),
            Targets::Application,
        )
        .expect("plan admitted");
    (
        terminal_app(d, planned.request()),
        d.session()
            .command_failure(planned.request())
            .map(str::to_owned),
    )
}

fn saved_world(d: &HeadlessDriver, name: &str) -> Option<u16> {
    d.session()
        .vault()
        .and_then(|vault| vault.get(name))
        .and_then(|profile| profile.settings.world)
}

fn member_detail(d: &HeadlessDriver, request: RequestId, name: &str) -> String {
    let subject = Subject::Profile(d.session().profile_identity_or_synthetic(name));
    d.report(request)
        .and_then(|report| report.member(subject))
        .and_then(|member| member.detail())
        .unwrap_or_default()
        .to_owned()
}

/// Confirm the waiting Switch world plan; its writes stay where they are.
fn confirm_switch(d: &mut HeadlessDriver) -> RequestId {
    let command = d.session().fleet_plan().expect("a plan waits").command();
    d.submit(command, Targets::Plan).expect("run").request()
}

/// Let the world saves land and the relogs post.
fn land_saves(d: &mut HeadlessDriver) {
    d.session_mut().flush_writes();
    d.poll();
    d.poll();
}

/// The world bubble's switch: one bot, always asked; the world is saved
/// before anything logs out; the intent post settles nothing; the member
/// is done only when a new session is in game on the new world.
#[test]
fn a_world_switch_saves_first_then_relogs_and_settles_on_the_new_session() {
    let mut d = world_driver("f4-switch", &["alice", "bob"]);
    mark(&mut d, &["bob"]);
    assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
    let plan = d.session().fleet_plan().expect("plan");
    assert!(plan.needs_confirm(), "a world switch always asks");
    assert_eq!(plan_names(&d), ["alice"], "the bubble's bot, not the marks");
    assert_eq!(
        d.session().fleet_plan().expect("plan").prompt(),
        "Move 1 bot to world 2: alice. Each logs out and back in through the login \
         queue; a running script continues after it reconnects."
    );

    let gate = d.session().write_gate();
    let held = gate.lock().unwrap();
    let run = confirm_switch(&mut d);
    d.poll();
    let alice = arm_of(&d, "alice");
    assert_eq!(terminal(&d, run, "alice"), None, "pending on its save");
    assert!(
        !alice.wants_logout(),
        "nothing logs out while the world save is held"
    );
    drop(held);
    land_saves(&mut d);
    assert_eq!(saved_world(&d, "alice"), Some(2));
    assert!(alice.wants_logout(), "the durable save posts the turnover");
    assert!(alice.wants_login(), "the turnover logs back in");
    assert_eq!(alice.world_choice(), Some(2));
    assert_eq!(
        terminal(&d, run, "alice"),
        None,
        "posting the switch is not the relog"
    );
    assert!(!arm_of(&d, "bob").wants_logout(), "bob was not in the plan");
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: pending 1"
    );

    log_in_on(&mut d, "alice", 2);
    assert_eq!(terminal(&d, run, "alice"), Some(done()));
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: done 1: alice"
    );
}

/// A world switch keeps the bot a fleet member, both for a bot loaded with
/// Load and for one whose slot a single-bot focus opened (the unlock's
/// auto-login with MultiBox off, which now loads it): the logout and the
/// relog through the same slot never drop it from the fleet.
#[test]
fn a_world_switch_keeps_the_bot_a_fleet_member() {
    let mut d = driver("f4-al-member", &["alice", "bob"]);
    d.session_mut().set_public_worlds_for_test(&[1, 2, 3]);
    load(&mut d, &["bob"]);
    // alice's slot is opened by a focus, as the single-mode unlock does.
    d.session_mut().stage_select_opens_slot();
    d.submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    d.poll();
    for name in ["alice", "bob"] {
        log_in_on(&mut d, name, 1);
    }
    let members = |d: &HeadlessDriver| {
        let mut members = d.session().members().to_vec();
        members.sort();
        members
    };
    assert_eq!(members(&d), ["alice", "bob"], "both are members");
    for name in ["alice", "bob"] {
        assert_eq!(plan_switch(&mut d, 2, Some(name)).0, Some(done()));
        let run = confirm_switch(&mut d);
        land_saves(&mut d);
        assert!(
            arm_of(&d, name).wants_logout(),
            "{name}: the relog is posted"
        );
        assert_eq!(
            members(&d),
            ["alice", "bob"],
            "{name}: a member while it relogs"
        );
        log_in_on(&mut d, name, 2);
        assert_eq!(terminal(&d, run, name), Some(done()), "{name}");
        assert_eq!(members(&d), ["alice", "bob"], "{name}: a member on world 2");
        let row = d
            .session()
            .fleet_view()
            .rows()
            .iter()
            .find(|row| row.name() == name)
            .map(|row| row.loaded());
        assert_eq!(row, Some(true), "{name}: the Fleet row stays loaded");
    }
}

/// Review F4-SOL-1: the frontend's status rows can still describe the old
/// session, in game on world 1, when the slot's new session begins. The
/// switch settles only from the new session's own readiness and world:
/// a stale row never fails it, and a ready new session settles it even
/// before its row is published.
#[test]
fn a_stale_status_row_never_settles_the_new_session() {
    let mut d = world_driver("f4-stale-row", &["alice"]);
    assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    let alice = arm_of(&d, "alice");
    assert!(alice.wants_logout(), "the relog is posted");
    // The new session logs in on world 2; its scene is not ready yet and
    // the row still reads the old session, in game on world 1.
    alice.configure_world_for_test(2);
    alice.complete_login_for_test();
    for _ in 0..3 {
        d.poll();
    }
    assert_eq!(
        terminal(&d, run, "alice"),
        None,
        "a new key with the old row settles nothing"
    );
    alice.scene_ready_for_test();
    d.poll();
    d.poll();
    assert_eq!(terminal(&d, run, "alice"), Some(done()));
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: done 1: alice"
    );
}

/// Review F4 Grok 2: a slot restarted between admission and the post is
/// not the slot the command was for. The member is cancelled and the new
/// slot's mailbox never receives the switch.
#[test]
fn a_switch_never_posts_to_a_slot_it_was_not_admitted_on() {
    let mut d = world_driver("f4-restarted-slot", &["alice"]);
    assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
    let gate = d.session().write_gate();
    let held = gate.lock().unwrap();
    let run = confirm_switch(&mut d);
    d.poll();
    // The restarted slot logged in again on world 1 (a SwitchWorld posted
    // to it would log it out).
    let restarted = host_play::SlotArm::new(1, true);
    restarted.configure_world_for_test(1);
    restarted.complete_login_for_test();
    restarted.scene_ready_for_test();
    d.session_mut()
        .play_mut_for_test()
        .expect("play")
        .attach_arm("alice", std::sync::Arc::clone(&restarted));
    drop(held);
    land_saves(&mut d);
    assert_eq!(
        terminal(&d, run, "alice"),
        Some(Settlement::Cancelled(
            api::identity::Cancellation::ScopeInvalidated
        ))
    );
    assert!(
        !restarted.wants_logout(),
        "nothing reached the restarted slot"
    );
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: cancelled 1: alice (the bot's slot restarted)"
    );
}

/// Picking the current world, a world off the server's list and a server
/// with one world each write and relog nothing; the headless row is no
/// longer refused for want of an entry point.
#[test]
fn switch_world_refusals_write_and_relog_nothing() {
    let mut d = world_driver("f4-refusals", &["alice"]);
    let (terminal_same, why) = plan_switch(&mut d, 1, Some("alice"));
    assert!(matches!(terminal_same, Some(Settlement::Failed(_))));
    assert_eq!(
        why.as_deref(),
        Some("Switch world: no bot to act on (alice: already on this world)")
    );
    let (_, why) = plan_switch(&mut d, 9, None);
    assert_eq!(
        why.as_deref(),
        Some("Switch world: world 9 is not on this server's world list")
    );
    let direct = d
        .submit(OperatorCommand::SwitchWorld(9), Targets::Profile("alice"))
        .expect("the headless row is admitted")
        .request();
    assert_eq!(
        terminal(&d, direct, "alice"),
        Some(rejected(crate::command::Rejection::NotApplicable))
    );
    let same = d
        .submit(OperatorCommand::SwitchWorld(1), Targets::Profile("alice"))
        .expect("admitted")
        .request();
    assert_eq!(
        terminal(&d, same, "alice"),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::AlreadyOnWorld
        )))
    );
    land_saves(&mut d);
    assert_eq!(saved_world(&d, "alice"), None, "nothing was saved");
    assert!(!arm_of(&d, "alice").wants_logout(), "nothing relogged");

    let mut local = driver("f4-one-world", &["alice"]);
    load(&mut local, &["alice"]);
    log_in_on(&mut local, "alice", 1);
    let (_, why) = plan_switch(&mut local, 2, Some("alice"));
    assert_eq!(
        why.as_deref(),
        Some("Switch world: this server has one world")
    );
    assert!(local.session().switch_world_choices().is_none());
}

/// A marked batch admits the bots in game elsewhere and names each
/// refusal once: queued, logging in, already there, not logged in.
#[test]
fn a_mixed_switch_admits_ready_bots_and_names_each_refusal_once() {
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let mut d = world_driver("f4-mixed", &names);
    arm_of(&d, "bob").request_relog();
    publish(&mut d, "bob", |row| {
        row.connected = false;
        row.ingame = false;
        row.startup_phase = host_play::StartupPhase::Queueing;
    });
    publish(&mut d, "carol", |row| {
        row.connected = false;
        row.ingame = false;
        row.startup_phase = host_play::StartupPhase::Connecting;
    });
    log_in_on(&mut d, "dave", 2);
    let logout = d
        .submit(OperatorCommand::Logout, Targets::Profile("erin"))
        .expect("logout");
    settle(&mut d, logout.request());
    publish(&mut d, "erin", |row| {
        row.connected = false;
        row.ingame = false;
    });
    mark(&mut d, &names);
    assert_eq!(plan_switch(&mut d, 2, None).0, Some(done()));
    assert_eq!(plan_names(&d), ["alice"]);
    let prompt = d.session().fleet_plan().expect("plan").prompt().to_owned();
    assert!(
        prompt.starts_with("Move 1 marked bot to world 2: alice.")
            && prompt.ends_with(
                " Skipped 4: bob: queued for login, carol: logging in, dave: already on \
                 this world, erin: not logged in."
            ),
        "{prompt}"
    );
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    log_in_on(&mut d, "alice", 2);
    assert_eq!(terminal(&d, run, "alice"), Some(done()));
    let result = d.session().fleet_result().expect("result");
    assert_eq!(
        result.text(),
        "Switch world: skipped 4: bob (queued for login), carol (logging in), dave \
         (already on this world), erin (not logged in); done 1: alice"
    );
    for name in ["bob", "carol", "dave", "erin"] {
        assert_eq!(saved_world(&d, name), None, "{name} was not written");
    }
}

/// A failed world save fails the member with the save's reason and never
/// logs the bot out.
#[test]
fn a_failed_world_save_never_logs_the_bot_out() {
    let dir = scratch("f4-failed-save");
    let mut d = world_driver_in(&dir, &["alice"]);
    let session = arm_of(&d, "alice").session_key();
    assert_eq!(plan_switch(&mut d, 3, Some("alice")).0, Some(done()));
    // A directory where the vault file was: the next write cannot publish.
    let vault = dir.join("vault");
    std::fs::rename(&vault, dir.join("vault.aside")).expect("vault aside");
    std::fs::create_dir(&vault).expect("block");
    let run = confirm_switch(&mut d);
    wait_for_command(&mut d, run, "the failed world-save terminal", |d| {
        terminal(d, run, "alice").is_some()
    });
    assert!(
        matches!(terminal(&d, run, "alice"), Some(Settlement::Failed(_))),
        "{:?}",
        terminal(&d, run, "alice")
    );
    assert!(
        member_detail(&d, run, "alice").starts_with("world not saved"),
        "{}",
        member_detail(&d, run, "alice")
    );
    let alice = arm_of(&d, "alice");
    assert!(!alice.wants_logout(), "a failed save never logs out");
    assert_eq!(alice.session_key(), session, "the connection is intact");
}

/// Several bots relog one each: every member waits for its own new
/// session (the host's login queue paces the logins).
#[test]
fn each_switched_bot_settles_on_its_own_relog() {
    let names = ["alice", "bob", "carol"];
    let mut d = world_driver("f4-several", &names);
    mark(&mut d, &names);
    assert_eq!(plan_switch(&mut d, 3, None).0, Some(done()));
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    for name in names {
        assert!(arm_of(&d, name).wants_logout(), "{name} relogs");
    }
    log_in_on(&mut d, "bob", 3);
    assert_eq!(terminal(&d, run, "bob"), Some(done()));
    assert_eq!(terminal(&d, run, "alice"), None);
    assert_eq!(terminal(&d, run, "carol"), None);
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: done 1: bob; pending 2"
    );
}

/// Log out during the save or the relog cancels the switch: nothing
/// posts after it and the bot stays logged out.
#[test]
fn a_logout_during_the_switch_cancels_it_and_nothing_logs_back_in() {
    let mut d = world_driver("f4-logout", &["alice", "bob"]);
    mark(&mut d, &["alice", "bob"]);
    assert_eq!(plan_switch(&mut d, 2, None).0, Some(done()));
    let gate = d.session().write_gate();
    let held = gate.lock().unwrap();
    let run = confirm_switch(&mut d);
    // alice: Log out while her world save is held.
    d.submit(OperatorCommand::Logout, Targets::Profile("alice"))
        .expect("logout");
    drop(held);
    land_saves(&mut d);
    // bob: Log out after his relog was posted.
    assert!(arm_of(&d, "bob").wants_logout());
    d.submit(OperatorCommand::Logout, Targets::Profile("bob"))
        .expect("logout");
    d.poll();
    for name in ["alice", "bob"] {
        assert_eq!(
            terminal(&d, run, name),
            Some(Settlement::Cancelled(
                api::identity::Cancellation::Superseded
            )),
            "{name}"
        );
        let arm = arm_of(&d, name);
        assert!(arm.login_latched(), "{name} stays logged out");
        assert!(!arm.wants_login(), "{name}: nothing logs back in");
    }
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: cancelled 2: alice (a later Log out took over), \
         bob (a later Log out took over)"
    );
}

/// Review F4 Grok 1: a Log in after the relog was posted takes the slot
/// over; the result names the switch cancelled, in words.
#[test]
fn a_log_in_during_the_switch_cancels_it_in_words() {
    let mut d = world_driver("f4-login", &["alice"]);
    assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    assert!(arm_of(&d, "alice").wants_logout(), "the relog is posted");
    d.submit(OperatorCommand::Login, Targets::Profile("alice"))
        .expect("login");
    d.poll();
    assert_eq!(
        terminal(&d, run, "alice"),
        Some(Settlement::Cancelled(
            api::identity::Cancellation::Superseded
        ))
    );
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: cancelled 1: alice (a later Log in took over)"
    );
}

/// Review F4 r3 (Sol 2): the old session drops and the repeat guard
/// latches the switch's login off, with no second command: the result
/// names the guard, not a command.
#[test]
fn a_switch_the_repeat_guard_stops_names_the_guard_not_a_command() {
    let mut d = world_driver("f4-guard", &["alice"]);
    assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    let arm = arm_of(&d, "alice");
    assert!(arm.wants_logout(), "the relog is posted");
    arm.exit_and_trip_repeat_guard_for_test();
    // One poll settles the member from the slot; the next refreshes the
    // result, as the next frame does.
    d.poll();
    d.poll();
    assert_eq!(
        terminal(&d, run, "alice"),
        Some(Settlement::Cancelled(
            api::identity::Cancellation::Superseded
        ))
    );
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: cancelled 1: alice (the automatic login guard stopped repeated logins)"
    );
}

/// Review F4 r2 (Sol): every command that can take a posted switch over
/// is named for what it is, never as another command: Relog now and
/// another Switch world supersede it and are named; Quit ends the slot,
/// and the result says so.
#[test]
fn a_superseded_switch_names_the_command_that_took_it_over() {
    use api::identity::Cancellation::{ScopeInvalidated, Superseded};
    for (label, command, targets, cause, words) in [
        (
            "relog",
            OperatorCommand::RelogNow,
            Targets::Profile("alice"),
            Superseded,
            "a later Relog now took over",
        ),
        (
            "switch",
            OperatorCommand::SwitchWorld(3),
            Targets::Profile("alice"),
            Superseded,
            "a later Switch world took over",
        ),
        (
            "quit",
            OperatorCommand::Quit,
            Targets::Everyone,
            ScopeInvalidated,
            "the session or slot it was for ended",
        ),
    ] {
        let mut d = world_driver(&format!("f4-r2-{label}"), &["alice"]);
        assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
        let run = confirm_switch(&mut d);
        land_saves(&mut d);
        assert!(arm_of(&d, "alice").wants_logout(), "{label}: posted");
        d.submit(command, targets).expect(label);
        land_saves(&mut d);
        assert_eq!(
            terminal(&d, run, "alice"),
            Some(Settlement::Cancelled(cause)),
            "{label}"
        );
        assert_eq!(
            d.session().fleet_result().expect("result").text(),
            format!("Switch world: cancelled 1: alice ({words})"),
            "{label}"
        );
    }
}

/// Removing a bot whose relog is posted cancels its member.
#[test]
fn removing_a_switching_bot_cancels_it() {
    let mut d = world_driver("f4-remove", &["alice"]);
    assert_eq!(plan_switch(&mut d, 2, Some("alice")).0, Some(done()));
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    assert!(arm_of(&d, "alice").wants_logout(), "the relog is posted");
    d.submit(OperatorCommand::Remove, Targets::Profile("alice"))
        .expect("remove");
    d.poll();
    assert!(
        matches!(terminal(&d, run, "alice"), Some(Settlement::Cancelled(_))),
        "{:?}",
        terminal(&d, run, "alice")
    );
    // Review F4 r3: the reason says only what is known when it settles.
    // Remove logs the bot out first; that logout row, not a Log out
    // command, withdrew the switch's login.
    d.poll();
    assert_eq!(
        d.session().fleet_result().expect("result").text(),
        "Switch world: cancelled 1: alice (a logout was requested)"
    );
}

/// A relog that lands on another world, or a world the host refuses at
/// login, fails the member with what happened.
#[test]
fn a_relog_elsewhere_or_a_refused_world_fails_the_member() {
    let mut d = world_driver("f4-elsewhere", &["alice", "bob"]);
    mark(&mut d, &["alice", "bob"]);
    assert_eq!(plan_switch(&mut d, 2, None).0, Some(done()));
    let run = confirm_switch(&mut d);
    land_saves(&mut d);
    log_in_on(&mut d, "alice", 1);
    arm_of(&d, "bob").hold_login_on_error_for_test();
    publish(&mut d, "bob", |row| {
        row.connected = false;
        row.ingame = false;
        row.startup_phase = host_play::StartupPhase::Error;
        row.error = Some("world 2 refused the login".into());
    });
    let rejected = Settlement::Failed(CommandFailure::Failed(
        crate::command::FailureKind::WorldRejected,
    ));
    assert_eq!(terminal(&d, run, "alice"), Some(rejected.clone()));
    assert_eq!(
        member_detail(&d, run, "alice"),
        "logged in on world 1 instead"
    );
    assert_eq!(terminal(&d, run, "bob"), Some(rejected));
    assert_eq!(member_detail(&d, run, "bob"), "world 2 refused the login");
}

// ---- PANEL-STATE-GAP-1023: one projection of what each bot is doing ----

const NINE: [&str; 9] = [
    "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan",
];

/// Publish `names` as clients in game, the way their slot threads do.
fn publish_in_game(d: &HeadlessDriver, names: &[&str]) {
    let mut rows = d.session().play().expect("play").statuses.lock().unwrap();
    for name in names {
        rows.retain(|row| row.username != *name);
        rows.push(host_play::SlotStatus {
            username: (*name).into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            ..host_play::SlotStatus::default()
        });
    }
}

/// Set (or clear) the published script-walk destination of `names`.
fn publish_walks(d: &HeadlessDriver, names: &[&str], walk: Option<(i32, i32, i32)>) {
    let (x, z, level) = walk.unwrap_or((-1, -1, -1));
    let mut rows = d.session().play().expect("play").statuses.lock().unwrap();
    for row in rows
        .iter_mut()
        .filter(|row| names.contains(&row.username.as_str()))
    {
        (row.walk_x, row.walk_z, row.walk_level) = (x, z, level);
    }
}

#[cfg(feature = "js")]
fn start_loop_scripts(d: &mut HeadlessDriver, names: &[&str]) {
    for name in names {
        d.session()
            .play()
            .expect("play")
            .script_start_load(
                name,
                "export function tick(api) {}".into(),
                script_js::LoadShape::NativeTick,
                None,
                vec![],
            )
            .expect("start");
    }
    for name in names {
        wait_state(d, name, ScriptRunState::Running);
    }
}

#[cfg(not(feature = "js"))]
fn start_compiled_scripts(d: &mut HeadlessDriver, names: &[&str]) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).expect("data");
    d.session_mut()
        .play_mut()
        .expect("play")
        .bind_script_test_data(data);
    mark(d, names);
    set_card(
        d,
        crate::FleetCard::pinned(crate::scripts::ScriptChoice::compiled(
            crate::scripts::CompiledCardId::lookup("Gatherer").expect("Gatherer is compiled"),
        )),
    );
    assert_eq!(plan(d, crate::FleetAction::Assign), Some(done()));
    run_plan(d);
    for name in names {
        d.submit(OperatorCommand::SetMark(false), Targets::Profile(name))
            .expect("unmark");
    }
    for name in names {
        let start = d
            .submit(OperatorCommand::StartScript, Targets::Profile(name))
            .expect("start");
        settle(d, start.request());
        wait_state(d, name, ScriptRunState::Running);
    }
}

/// Every row's `(name, label)` as the fleet model hands them to a front end.
fn row_labels(d: &HeadlessDriver) -> Vec<(String, String)> {
    d.models()
        .fleet()
        .rows()
        .iter()
        .map(|row| (row.name().to_owned(), row.brief().to_owned()))
        .collect()
}

fn all_labels(label: &str) -> Vec<(String, String)> {
    NINE.iter()
        .map(|name| ((*name).to_owned(), label.to_owned()))
        .collect()
}

/// A driver with the nine bots loaded, in game, each running a script, with
/// script walks published on the first three and every bot marked.
fn nine_running_with_three_walks(test: &str) -> TestDriver {
    let mut d = driver(test, &NINE);
    load(&mut d, &NINE);
    publish_in_game(&d, &NINE);
    #[cfg(feature = "js")]
    start_loop_scripts(&mut d, &NINE);
    #[cfg(not(feature = "js"))]
    start_compiled_scripts(&mut d, &NINE);
    publish_walks(&d, &NINE[..3], Some((2550, 3348, 0)));
    d.poll();
    mark(&mut d, &NINE);
    d.poll();
    d
}

/// PANEL-STATE-GAP-1023, acceptance 1-3: Stop on nine marked bots in game,
/// three of them walking for their script, completes all nine, retires the
/// three script-walk observations in the runtime's status rows (not only in
/// the projection), keeps every client logged in, and leaves zero active
/// scripts and zero script walks.
#[test]
fn stopping_nine_marked_bots_retires_the_three_script_walks() {
    let mut d = nine_running_with_three_walks("psg-stop-nine");
    assert_eq!(row_labels(&d), all_labels("running"));
    let walking: Vec<_> = d
        .models()
        .fleet()
        .rows()
        .iter()
        .filter(|row| row.script_walk())
        .map(|row| row.name().to_owned())
        .collect();
    assert_eq!(walking, ["alice", "bob", "carol"]);
    assert_eq!(d.models().fleet().counts().marked(), 9);

    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    assert_eq!(plan_names(&d), NINE);
    let stop = run_plan(&mut d);
    for name in NINE {
        assert_eq!(terminal(&d, stop, name), Some(done()), "Stop {name}");
        wait_state(&mut d, name, ScriptRunState::Idle);
    }
    d.poll();
    let result = d.session().fleet_result().expect("result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().done, 9, "{}", result.text());

    // The runtime owner retired the destinations: the host rows say so.
    for row in d
        .session()
        .play()
        .expect("play")
        .statuses
        .lock()
        .unwrap()
        .iter()
    {
        assert_eq!(
            row.script_walk(),
            None,
            "{} kept a walk target",
            row.username
        );
    }
    // The projection: every client still in game, no script, no script walk.
    assert_eq!(row_labels(&d), all_labels("idle"));
    for row in d.models().fleet().rows() {
        assert_eq!(
            (
                row.script().is_live(),
                row.script_walk(),
                row.manual_walk(),
                row.connected(),
                row.phase()
            ),
            (false, false, false, true, crate::Phase::Ready),
            "{}",
            row.name()
        );
    }
}

/// PANEL-STATE-GAP-1023, acceptance 4: after that Stop, changing focus,
/// marks, the filter and the displayed script choice never brings a walk
/// label back: a label is a fact about the bot, not about a view of it.
#[test]
fn old_walk_labels_do_not_return_after_focus_mark_filter_or_choice_changes() {
    use crate::{MarkOp, MarkPredicate};
    let mut d = nine_running_with_three_walks("psg-views");
    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    run_plan(&mut d);
    for name in NINE {
        wait_state(&mut d, name, ScriptRunState::Idle);
    }
    d.poll();
    let idle = all_labels("idle");
    assert_eq!(row_labels(&d), idle);

    // Focus: every bot in turn, the three that walked first.
    for name in ["alice", "bob", "carol", "dave", "ivan"] {
        d.submit(OperatorCommand::Select, Targets::Profile(name))
            .expect("select");
        d.poll();
        assert_eq!(row_labels(&d), idle, "focus on {name}");
        assert_eq!(
            d.session()
                .fleet_view()
                .detail()
                .map(|detail| detail.row().brief()),
            Some("idle"),
            "the focused detail agrees"
        );
    }

    // Marks: clear, mark the three, mark by every status.
    d.submit(OperatorCommand::ClearMarks, Targets::Application)
        .expect("clear marks");
    mark(&mut d, &NINE[..3]);
    d.poll();
    assert_eq!(row_labels(&d), idle, "marks");
    for predicate in MarkPredicate::ALL {
        d.submit(
            OperatorCommand::MarkFleet(MarkOp::Where(predicate)),
            Targets::Application,
        )
        .expect("mark by status");
        d.poll();
        assert_eq!(row_labels(&d), idle, "mark by {}", predicate.label());
    }
    d.submit(OperatorCommand::ClearMarks, Targets::Application)
        .expect("clear marks");
    d.submit(
        OperatorCommand::MarkFleet(MarkOp::Where(MarkPredicate::Running)),
        Targets::Application,
    )
    .expect("mark running");
    assert_eq!(
        d.models().fleet().counts().marked(),
        0,
        "nothing is running"
    );

    // Filters: by label text, by name.
    for (text, shown) in [("running", 0), ("idle", 9), ("alice", 1), ("", 9)] {
        let filter = d.models_mut().fleet_mut().staged_filter_mut();
        filter.clear();
        filter.push_str(text);
        d.submit(OperatorCommand::SetFleetFilter, Targets::Application)
            .expect("filter");
        d.poll();
        assert_eq!(d.models().fleet().shown().len(), shown, "filter {text:?}");
        assert_eq!(row_labels(&d), idle, "filter {text:?}");
    }

    // The displayed script choice: a pending Browse pick on a bot that
    // walked changes its heading and nothing about what it is doing.
    d.models_mut()
        .scripts_mut()
        .set_pending_browse("alice", gatherer());
    d.poll();
    assert_eq!(row_labels(&d), idle, "script choice");
    let alice = d
        .session()
        .bot_projection(Some(d.models().scripts()), "alice")
        .expect("alice");
    assert_eq!(alice.displayed_script(), Some("Gatherer"));
    assert_eq!(alice.label(), "idle");
}

/// PANEL-STATE-GAP-1023, acceptance 6 and 7 (application half): a WalkTo
/// walk is a separate fact that a script Stop leaves visible; the label, the
/// Stop decision and the status filters all read the same row facts.
#[test]
fn a_manual_walk_stays_visible_after_stop_and_the_decisions_read_the_same_facts() {
    use crate::{MarkOp, MarkPredicate};
    let mut d = nine_running_with_three_walks("psg-manual");
    let arm = std::sync::Arc::new(std::sync::Mutex::new(host_play::WalkArm::default()));
    d.session()
        .walk_arms()
        .lock()
        .unwrap()
        .insert("ivan".to_owned(), std::sync::Arc::clone(&arm));
    arm.lock().unwrap().route = Some(std::sync::Arc::new(nav_planner::router::Route {
        legs: Vec::new(),
        dest: api::snapshot::WorldTile {
            x: 3220,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    }));
    d.poll();

    // Ivan's script goes first, so his walk is the only thing he is doing.
    d.session().play().expect("play").script_stop("ivan");
    wait_state(&mut d, "ivan", ScriptRunState::Idle);
    d.poll();
    {
        let ivan = d
            .session()
            .bot_projection(Some(d.models().scripts()), "ivan")
            .expect("ivan");
        assert_eq!(
            (
                ivan.script(),
                ivan.script_walk(),
                ivan.manual_walk(),
                ivan.label()
            ),
            (ScriptRunState::Idle, false, true, "running"),
            "a walk alone reads as activity"
        );
        assert_eq!(
            ivan.stop_eligibility(),
            crate::StopEligibility::NoScript,
            "a walk is not a script: Stop has nothing to stop"
        );
    }

    // Stop the other eight, including the three with script walks.
    d.submit(OperatorCommand::ClearMarks, Targets::Application)
        .expect("clear marks");
    mark(&mut d, &NINE[..8]);
    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    assert_eq!(plan_names(&d), &NINE[..8]);
    run_plan(&mut d);
    for name in &NINE[..8] {
        wait_state(&mut d, name, ScriptRunState::Idle);
    }
    d.poll();
    let mut want = all_labels("idle");
    want[8].1 = "running".into();
    assert_eq!(row_labels(&d), want, "only the manual walker is running");
    assert!(
        d.models().fleet().rows()[8].manual_walk(),
        "the manual walk survived the Stop"
    );
    assert!(d.session().walk_route_armed("ivan"));

    // The Mark-by-status predicates read the same facts as the label.
    for (predicate, marked) in [
        (MarkPredicate::Running, ["ivan"].as_slice()),
        (MarkPredicate::Idle, &NINE[..8]),
    ] {
        d.submit(OperatorCommand::ClearMarks, Targets::Application)
            .expect("clear marks");
        d.submit(
            OperatorCommand::MarkFleet(MarkOp::Where(predicate)),
            Targets::Application,
        )
        .expect("mark by status");
        let fleet = d.models().fleet();
        let got: Vec<_> = fleet
            .rows()
            .iter()
            .enumerate()
            .filter(|(index, _)| fleet.is_marked(*index))
            .map(|(_, row)| row.name())
            .collect();
        assert_eq!(got, marked, "Mark by status > {}", predicate.label());
    }

    // Stop of the walker alone is refused for want of a script, and the
    // refusal comes from the same fact the projection shows.
    d.submit(OperatorCommand::ClearMarks, Targets::Application)
        .expect("clear marks");
    mark(&mut d, &["ivan"]);
    let planned = d
        .submit(
            OperatorCommand::PlanFleetAction(crate::FleetAction::Stop),
            Targets::Application,
        )
        .expect("plan admitted");
    assert_eq!(
        d.session().command_failure(planned.request()),
        Some("Stop: no bot to act on (ivan: no script running)")
    );
    assert!(d.session().fleet_plan().is_none());
    assert!(arm.lock().unwrap().route.is_some());

    arm.lock().unwrap().route = None;
    d.poll();
    assert_eq!(row_labels(&d), all_labels("idle"));
}

/// PANEL-STATE-GAP-1023, acceptance 5: repeating Stop on bots that are
/// already stopped is refused at the command boundary, and that refusal logs
/// one `info` line naming the command, the bots and the reason. The first
/// Stop, which ran, logs none. The bots have names of their own because the
/// process log is shared by every test.
#[test]
fn a_repeated_stop_logs_one_refusal_line() {
    use crate::log::{global, LogScope, LogView};
    const BOTS: [&str; 3] = ["psg5-a", "psg5-b", "psg5-c"];
    let mut d = driver("psg-refusal", &BOTS);
    load(&mut d, &BOTS);
    publish_in_game(&d, &BOTS);
    #[cfg(feature = "js")]
    start_loop_scripts(&mut d, &BOTS);
    #[cfg(not(feature = "js"))]
    start_compiled_scripts(&mut d, &BOTS);
    mark(&mut d, &BOTS);
    d.poll();
    let refusal_lines = |d: &HeadlessDriver| -> Vec<String> {
        let _ = d;
        let mut view = LogView::new(LogScope::Process);
        view.edit_filter(|filter| filter.set_text("command refused"));
        global().refresh(&mut view);
        view.rows()
            .iter()
            .map(|row| row.message().to_owned())
            .filter(|line| line.contains("psg5-"))
            .collect()
    };

    assert_eq!(plan(&mut d, crate::FleetAction::Stop), Some(done()));
    run_plan(&mut d);
    for name in BOTS {
        wait_state(&mut d, name, ScriptRunState::Idle);
    }
    d.poll();
    assert_eq!(refusal_lines(&d), Vec::<String>::new(), "a Stop that ran");

    let before = d.session().refusals();
    let planned = d
        .submit(
            OperatorCommand::PlanFleetAction(crate::FleetAction::Stop),
            Targets::Application,
        )
        .expect("plan admitted");
    assert!(d.session().command_failure(planned.request()).is_some());
    assert_eq!(d.session().refusals(), before + 1);
    let lines = refusal_lines(&d);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    for part in [
        "PlanFleetAction(Stop)",
        "no bot to act on",
        "psg5-a: no script running",
        "psg5-b: no script running",
        "psg5-c: no script running",
    ] {
        assert!(line.contains(part), "{part:?} missing from {line:?}");
    }
}

/// PANEL-STATE-GAP-1023: every refusal line holds the command, the bots and
/// the reason. A member that carries no text still names its typed reason.
#[test]
fn a_refusal_line_names_the_command_the_bots_and_the_reason() {
    use crate::log::{global, LogScope, LogView};
    const BOT: &str = "psg5r-bot";
    let mut d = driver("psg-refusal-reason", &[BOT]);
    load(&mut d, &[BOT]);
    let lines_with = |needle: &str| -> Vec<String> {
        let mut view = LogView::new(LogScope::Process);
        view.edit_filter(|filter| filter.set_text("command refused: PickBrowseCard"));
        global().refresh(&mut view);
        view.rows()
            .iter()
            .map(|row| row.message().to_owned())
            .filter(|line| line.contains(needle))
            .collect()
    };

    // A command for the application, given a bot: refused before it opens.
    let before = d.session().refusals();
    assert!(d
        .submit(OperatorCommand::PickBrowseCard, Targets::Profile(BOT))
        .is_err());
    assert_eq!(d.session().refusals(), before + 1);
    let named = lines_with(BOT);
    assert_eq!(named.len(), 1, "{named:?}");
    for part in [
        "PickBrowseCard",
        "on psg5r-bot",
        "cannot take these targets",
    ] {
        assert!(
            named[0].contains(part),
            "{part:?} missing from {:?}",
            named[0]
        );
    }

    // The native command has no staged pick and no detailed error text.
    d.submit(OperatorCommand::PickBrowseCard, Targets::Application)
        .expect("opened");
    assert_eq!(d.session().refusals(), before + 2);
    let bare = lines_with("on application: NotStaged");
    assert!(!bare.is_empty(), "the typed reason is in the line");
    assert!(
        bare.iter()
            .all(|line| line.ends_with("on application: NotStaged")),
        "the bare typed reason survives without detailed text: {bare:?}"
    );
}

// ---- LOGOUT-RETRY-1025: Log out reaches a login that is still retrying ----

/// Publish `name` as a bot whose login was refused and whose slot still
/// wants the login, so the host retries it after its backoff.
fn publish_refused_retry(d: &mut HeadlessDriver, name: &str) {
    publish(d, name, |row| {
        row.connected = false;
        row.ingame = false;
        row.startup_phase = host_play::StartupPhase::Error;
        row.startup_phase_started = std::time::Instant::now();
        row.error = Some("Error connecting to server. (connect: Connection refused)".into());
    });
}

/// A bot whose login is refused and retried keeps retrying for ever unless
/// Log out reaches it. Fleet Log out (one shared plan for panel and TUI)
/// takes it, parks it so the slot makes no further login attempt, and
/// refuses a bot that never logged in with the one reason it always had.
#[test]
fn logout_retry_fleet_takes_a_bot_whose_login_is_retrying_and_stops_the_retries() {
    let mut d = driver("logout-retry-fleet", &["alice", "bob", "carol"]);
    load(&mut d, &["alice", "bob", "carol"]);
    d.submit(OperatorCommand::Login, Targets::Profile("bob"))
        .expect("login");
    publish_refused_retry(&mut d, "bob");
    let bob = d.session().fleet_view().row("bob").expect("bob").snapshot();
    assert_eq!(
        (bob.phase(), bob.retrying()),
        (crate::Phase::LoginError, true),
        "bob is between retries"
    );
    let arm = arm_of(&d, "bob");
    assert!(arm.wants_login(), "the slot would retry");

    mark(&mut d, &["alice", "bob", "carol"]);
    let planned = d
        .submit(
            OperatorCommand::PlanFleetAction(crate::FleetAction::Logout),
            Targets::Application,
        )
        .expect("plan admitted");
    assert_eq!(
        d.session().command_failure(planned.request()),
        None,
        "bob has a login to cancel"
    );
    assert_eq!(plan_names(&d), ["bob"]);
    let prompt = d.session().fleet_plan().expect("plan").prompt().to_owned();
    assert!(
        prompt.starts_with("Log out 3 marked bot")
            || prompt.starts_with("Log out 1 marked bot: bob."),
        "{prompt}"
    );
    assert!(
        prompt.contains("alice: not logged in") && prompt.contains("carol: not logged in"),
        "the bots that never logged in are skipped: {prompt}"
    );
    let run = run_plan(&mut d);
    assert_eq!(terminal(&d, run, "bob"), Some(done()));
    assert!(arm.login_latched(), "bob is parked logged out");
    assert!(!arm.wants_login(), "the retry loop has nothing to retry");
    for name in ["alice", "carol"] {
        assert!(
            !arm_of(&d, name).login_latched(),
            "{name} was never logged out: it never logged in"
        );
    }
    d.poll();
    let bob = d.session().fleet_view().row("bob").expect("bob").snapshot();
    assert_eq!(bob.phase(), crate::Phase::LoggedOut);
    assert!(
        !bob.logout_eligibility().eligible(),
        "a parked bot has nothing more to cancel"
    );
}

/// With only never-logged-in bots marked, Log out still has no bot to act
/// on, and the refusal names each one.
#[test]
fn logout_retry_fleet_still_refuses_bots_that_never_logged_in() {
    let mut d = driver("logout-retry-refusal", &["alice", "bob"]);
    load(&mut d, &["alice", "bob"]);
    mark(&mut d, &["alice", "bob"]);
    let planned = d
        .submit(
            OperatorCommand::PlanFleetAction(crate::FleetAction::Logout),
            Targets::Application,
        )
        .expect("plan admitted");
    assert_eq!(
        d.session().command_failure(planned.request()),
        Some("Log out: no bot to act on (alice: not logged in, bob: not logged in)")
    );
    assert!(d.session().fleet_plan().is_none());
}

// --- T5.3-S3: saved settings take effect from the latest durable snapshot ---

/// A disk for one driver's service: it holds the first `held` file writes
/// at their start until [`Disk::release`], and fails every write numbered
/// above the count set by [`Disk::fail_after`] (the counting starts when
/// the disk is installed).
struct Disk {
    release: std::sync::mpsc::Sender<()>,
    fail_after: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Disk {
    fn install(d: &HeadlessDriver, held: usize) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = std::sync::Mutex::new(gate);
        let writes = std::sync::Arc::new(AtomicUsize::new(0));
        let fail_after = std::sync::Arc::new(AtomicUsize::new(usize::MAX));
        let service = d
            .session()
            .local_state()
            .expect("the driver binds the process service");
        let stalled = std::sync::Arc::clone(&writes);
        service.set_write_stall_for_test(std::sync::Arc::new(move || {
            if stalled.fetch_add(1, SeqCst) < held {
                let _ = gate.lock().unwrap().recv();
            }
        }));
        let counted = std::sync::Arc::clone(&writes);
        let limit = std::sync::Arc::clone(&fail_after);
        service.set_write_fault_for_test(std::sync::Arc::new(move || {
            counted.load(SeqCst) > limit.load(SeqCst)
        }));
        Self {
            release,
            fail_after,
        }
    }

    /// Let the oldest held write run.
    fn release(&self) {
        self.release.send(()).expect("the writer waits at the gate");
    }

    /// Writes numbered above `n` fail (the first write is number 1).
    fn fail_after(&self, n: usize) {
        self.fail_after
            .store(n, std::sync::atomic::Ordering::SeqCst);
    }

    fn pass_all(&self) {
        self.fail_after(usize::MAX);
    }
}

/// One saved setting a command changes: how to stage and submit it, and
/// what the session shows before the write is durable and after.
struct Saved {
    name: &'static str,
    setting: crate::Setting,
    stage: fn(&mut HeadlessDriver),
    command: OperatorCommand,
    /// The default, nothing changed.
    unchanged: fn(&HeadlessDriver) -> bool,
    /// The value the command asks for, applied.
    changed: fn(&HeadlessDriver) -> bool,
}

fn nothing_staged(_: &mut HeadlessDriver) {}

const FIXTURE_SOURCE: &str = "s3-durable-source";

fn saved_settings() -> Vec<Saved> {
    use crate::nav_prefs::NavPreference as N;
    use crate::walk_permissions::DangerLevel;
    use crate::Setting as S;
    vec![
        Saved {
            name: "sidecar",
            setting: S::Draw(DrawSwitch::SidecarFullRate),
            stage: nothing_staged,
            command: OperatorCommand::SetDraw(DrawSwitch::SidecarFullRate, true),
            unchanged: |d| !d.models().draw().sidecar_50() && d.models().draw_changes() == 0,
            changed: |d| d.models().draw().sidecar_50() && d.models().draw_changes() == 1,
        },
        Saved {
            name: "capture",
            setting: S::Draw(DrawSwitch::CaptureInput),
            stage: nothing_staged,
            command: OperatorCommand::SetDraw(DrawSwitch::CaptureInput, false),
            unchanged: |d| d.models().draw().capture() && d.models().draw_changes() == 0,
            changed: |d| !d.models().draw().capture() && d.models().draw_changes() == 1,
        },
        Saved {
            name: "focused",
            setting: S::Draw(DrawSwitch::FocusedFullRate),
            stage: nothing_staged,
            command: OperatorCommand::SetDraw(DrawSwitch::FocusedFullRate, false),
            unchanged: |d| d.models().draw().focused_50() && d.models().draw_changes() == 0,
            changed: |d| !d.models().draw().focused_50() && d.models().draw_changes() == 1,
        },
        Saved {
            name: "teleports",
            setting: S::Nav(N::AllowTeleports),
            stage: nothing_staged,
            command: OperatorCommand::SetNavPreference(N::AllowTeleports, true),
            unchanged: |d| {
                !d.models().walk_globals().globals().allow_teleports()
                    && !d
                        .session()
                        .play()
                        .expect("play")
                        .walk_globals()
                        .allow_teleports
            },
            changed: |d| {
                d.models().walk_globals().globals().allow_teleports()
                    && d.session()
                        .play()
                        .expect("play")
                        .walk_globals()
                        .allow_teleports
            },
        },
        Saved {
            name: "danger",
            setting: S::DangerRouting,
            stage: nothing_staged,
            command: OperatorCommand::SetDangerLevel(DangerLevel::Always),
            unchanged: |d| d.models().walk_globals().danger_level() != DangerLevel::Always,
            changed: |d| d.models().walk_globals().danger_level() == DangerLevel::Always,
        },
        Saved {
            name: "pause",
            setting: S::Nav(N::PauseScriptOnManualWalkAbort),
            stage: nothing_staged,
            command: OperatorCommand::SetNavPreference(N::PauseScriptOnManualWalkAbort, false),
            unchanged: |d| d.session().running_pause_script_on_manual_walk_abort(),
            changed: |d| !d.session().running_pause_script_on_manual_walk_abort(),
        },
        Saved {
            name: "map-bake",
            setting: S::MapBake,
            stage: nothing_staged,
            command: OperatorCommand::SetMapBakeChoice(crate::MapBakeChoice::Always),
            unchanged: |d| d.models().map().bake_gate().choice() == crate::MapBakeChoice::Ask,
            changed: |d| d.models().map().bake_gate().choice() == crate::MapBakeChoice::Always,
        },
        Saved {
            name: "quester",
            setting: S::QuesterPaths,
            stage: |d| {
                d.session_mut().stage_quester_paths(
                    crate::QuesterPathsView::new(true, PathBuf::from(FIXTURE_SOURCE)),
                    true,
                    None,
                );
            },
            command: OperatorCommand::SetQuesterPaths,
            unchanged: |d| {
                d.models().quester().settings().folder() != Path::new(FIXTURE_SOURCE)
                    && script::quester::registry::source_for_preparation().folder
                        != Path::new(FIXTURE_SOURCE)
            },
            changed: |d| {
                d.models().quester().settings().folder() == Path::new(FIXTURE_SOURCE)
                    && script::quester::registry::source_for_preparation().folder
                        == Path::new(FIXTURE_SOURCE)
            },
        },
        Saved {
            name: "render-all-ack",
            setting: S::Notice(crate::OperatorNotice::RenderAllWarning),
            stage: |d| {
                d.session_mut()
                    .stage_notice(crate::OperatorNotice::RenderAllWarning)
            },
            command: OperatorCommand::AcknowledgeNotice,
            unchanged: |d| {
                d.session()
                    .preferences()
                    .expect("prefs")
                    .value("render_all_warning_ack")
                    .is_none()
            },
            changed: |d| {
                d.session()
                    .preferences()
                    .expect("prefs")
                    .value("render_all_warning_ack")
                    == Some(serde_json::Value::Bool(true))
            },
        },
        Saved {
            name: "background-ack",
            setting: S::Notice(crate::OperatorNotice::BackgroundBots),
            stage: |d| {
                d.session_mut()
                    .stage_notice(crate::OperatorNotice::BackgroundBots)
            },
            command: OperatorCommand::AcknowledgeNotice,
            unchanged: |d| {
                !crate::resources::background_bots_acked(&d.session().preferences().expect("prefs"))
            },
            changed: |d| {
                crate::resources::background_bots_acked(&d.session().preferences().expect("prefs"))
            },
        },
    ]
}

/// Every saved operational setting, held and then failed: nothing is
/// applied while its write is pending or after it failed, the callback
/// and the play never see the value, the command settles once as failed,
/// and the failure stays at the setting. A retry (the action that resolves
/// it) clears the failure at once and the value then takes effect from the
/// durable snapshot.
#[test]
fn a_held_then_failed_saved_setting_changes_nothing_and_fails_at_its_setting() {
    for case in saved_settings() {
        let mut d = driver(&format!("s3-held-failed-{}", case.name), &["alice"]);
        // The Quester source is process-wide: hold it across its own case,
        // after the driver (whose open takes the same lock) exists.
        let _source =
            (case.name == "quester").then(crate::quester_paths::registry_source_test_lock);
        d.poll();
        assert!((case.unchanged)(&d), "{}: starts at its default", case.name);
        let disk = Disk::install(&d, 1);
        disk.fail_after(0);
        (case.stage)(&mut d);
        let request = d
            .submit(case.command, Targets::Application)
            .expect("admitted")
            .request();
        for _ in 0..3 {
            d.poll();
            assert_eq!(terminal_app(&d, request), None, "{}: pending", case.name);
            assert!(
                matches!(
                    d.session().setting_status(case.setting),
                    crate::SettingStatus::Pending
                ),
                "{}: the setting says it is saving",
                case.name
            );
            assert!((case.unchanged)(&d), "{}: nothing applied yet", case.name);
        }
        disk.release();
        let terminal = settled_app(&mut d, request).expect("settled");
        assert!(
            matches!(
                terminal,
                Settlement::Failed(CommandFailure::Failed(
                    crate::command::FailureKind::WriteFailed
                ))
            ),
            "{}: one failed terminal, not applied: {terminal:?}",
            case.name
        );
        d.poll();
        assert!(
            (case.unchanged)(&d),
            "{}: a failed write changes nothing",
            case.name
        );
        let failure = d
            .session()
            .setting_failure(case.setting)
            .unwrap_or_else(|| panic!("{}: the failure stays at its setting", case.name))
            .to_owned();
        assert!(
            failure.starts_with("Not saved: "),
            "{}: {failure}",
            case.name
        );
        assert!(
            d.report(request).is_some_and(|report| report.is_settled()),
            "{}: the report settled",
            case.name
        );

        // The retry resolves the failure, then the durable value applies.
        disk.pass_all();
        (case.stage)(&mut d);
        let retry = d
            .submit(case.command, Targets::Application)
            .expect("admitted")
            .request();
        assert_eq!(
            d.session().setting_failure(case.setting),
            None,
            "{}: the retry clears the failure",
            case.name
        );
        assert_eq!(settled_app(&mut d, retry), Some(done()), "{}", case.name);
        d.poll();
        assert!((case.changed)(&d), "{}: durable, so applied", case.name);
        assert!(matches!(
            d.session().setting_status(case.setting),
            crate::SettingStatus::Saved
        ));
    }
}

/// The saved session file is a durable choice and a sink: nothing opens
/// while the write is pending or after it failed, and the file opens from
/// the durable value.
#[test]
fn a_held_then_failed_session_log_choice_opens_nothing_until_it_is_durable() {
    let _sink = crate::log_file::sink_test_lock();
    crate::log_file::apply_session_log_for_test(false);
    let mut d = driver("s3-session-log", &["alice"]);
    d.poll();
    let disk = Disk::install(&d, 1);
    disk.fail_after(0);
    let request = d
        .submit(OperatorCommand::SetSessionLog(true), Targets::Application)
        .expect("admitted")
        .request();
    d.poll();
    assert!(
        crate::log_file::session_log_path().is_none(),
        "pending: no file"
    );
    disk.release();
    assert!(matches!(
        settled_app(&mut d, request),
        Some(Settlement::Failed(_))
    ));
    d.poll();
    assert!(
        crate::log_file::session_log_path().is_none(),
        "failed: no file opened"
    );
    assert!(d
        .session()
        .setting_failure(crate::Setting::SessionLog)
        .is_some_and(|text| text.starts_with("Not saved: ")));

    disk.pass_all();
    let retry = d
        .submit(OperatorCommand::SetSessionLog(true), Targets::Application)
        .expect("admitted")
        .request();
    assert_eq!(settled_app(&mut d, retry), Some(done()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while crate::log_file::session_log_path().is_none() && std::time::Instant::now() < deadline {
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        crate::log_file::session_log_path().is_some(),
        "durable: file opens"
    );
    d.submit(OperatorCommand::SetSessionLog(false), Targets::Application)
        .expect("admitted");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while crate::log_file::session_log_path().is_some() && std::time::Instant::now() < deadline {
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(crate::log_file::session_log_path().is_none());
}

/// A retained failure is a UI message: it leaves by itself after the
/// error timeout when nothing resolved it first.
#[test]
fn a_retained_setting_failure_expires_by_itself() {
    let mut d = driver("s3-failure-expires", &["alice"]);
    d.poll();
    let disk = Disk::install(&d, 0);
    disk.fail_after(0);
    let setting = crate::Setting::Draw(DrawSwitch::SidecarFullRate);
    let request = d
        .submit(
            OperatorCommand::SetDraw(DrawSwitch::SidecarFullRate, true),
            Targets::Application,
        )
        .expect("admitted")
        .request();
    assert!(matches!(
        settled_app(&mut d, request),
        Some(Settlement::Failed(_))
    ));
    let now = std::time::Instant::now();
    d.poll_at(now);
    assert!(d.session().setting_failure(setting).is_some());
    d.poll_at(now + crate::ERROR_MESSAGE_TIMEOUT - std::time::Duration::from_secs(1));
    assert!(d.session().setting_failure(setting).is_some(), "not yet");
    d.poll_at(now + crate::ERROR_MESSAGE_TIMEOUT + std::time::Duration::from_secs(1));
    assert_eq!(d.session().setting_failure(setting), None, "expired");
    assert_eq!(
        d.session().next_setting_expiry(),
        None,
        "nothing left to wake for"
    );
}

/// A is held, B is accepted behind it, a second process changes another
/// member, A becomes durable and B fails: after both settled once the
/// active value is A's (not the value before A, and not failed B's), the
/// other process's change reached the models, and each command has one
/// terminal.
#[test]
fn a_durable_then_b_failed_leaves_a_active_with_a_second_process_between() {
    let mut d = driver("s3-a-b-second", &["alice"]);
    d.poll();
    let disk = Disk::install(&d, 1);
    disk.fail_after(1);
    let a = d
        .submit(
            OperatorCommand::SetDraw(DrawSwitch::SidecarFullRate, true),
            Targets::Application,
        )
        .expect("A admitted")
        .request();
    let b = d
        .submit(
            OperatorCommand::SetDraw(DrawSwitch::SidecarFullRate, false),
            Targets::Application,
        )
        .expect("B admitted")
        .request();
    d.poll();
    assert!(
        !d.models().draw().sidecar_50(),
        "A is held: nothing applied"
    );
    // A second process over the same root writes another member while A waits.
    let root = d
        .session()
        .preferences()
        .expect("prefs")
        .path()
        .parent()
        .expect("state root")
        .to_path_buf();
    {
        let other = crate::UiPreferences::open_for_test(root);
        other
            .set("focused_50", serde_json::Value::Bool(false))
            .expect("the second process writes");
    }
    disk.release();
    assert_eq!(settled_app(&mut d, a), Some(done()), "A is durable");
    assert!(
        matches!(settled_app(&mut d, b), Some(Settlement::Failed(_))),
        "B fails"
    );
    d.poll();
    assert!(
        d.models().draw().sidecar_50(),
        "active equals A, not the value before A and not failed B"
    );
    assert!(
        !d.models().draw().focused_50(),
        "the second process's version reached the model"
    );
    let file = preferences_file(&d);
    assert_eq!(file["sidecar_50"], serde_json::Value::Bool(true));
    assert_eq!(file["focused_50"], serde_json::Value::Bool(false));
    assert!(d
        .session()
        .setting_failure(crate::Setting::Draw(DrawSwitch::SidecarFullRate))
        .is_some());
    for request in [a, b] {
        assert!(d.report(request).is_some_and(|report| report.is_settled()));
    }
}

/// Another process's saved change (no request of this session behind it)
/// reaches every model once its durable version is read, and calls the
/// surface back.
#[test]
fn a_second_process_version_reaches_the_models_and_calls_the_surface_back() {
    let mut d = driver("s3-second-process", &["alice"]);
    d.poll();
    assert_eq!(d.models().draw_changes(), 0);
    let root = d
        .session()
        .preferences()
        .expect("prefs")
        .path()
        .parent()
        .expect("state root")
        .to_path_buf();
    {
        let other = crate::UiPreferences::open_for_test(root);
        other
            .set("sidecar_50", serde_json::Value::Bool(true))
            .expect("the second process writes");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !d.models().draw().sidecar_50() && std::time::Instant::now() < deadline {
        d.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(d.models().draw().sidecar_50());
    assert_eq!(d.models().draw_changes(), 1, "one callback for one change");
    for _ in 0..5 {
        d.poll();
    }
    assert_eq!(
        d.models().draw_changes(),
        1,
        "an unchanged version calls nothing back"
    );
}

/// A saved-setting command the preference queue refuses before accepting it
/// (the writer's queue is full) is a refused command, not a failed write:
/// one `info` line at the shared command boundary names it, the refusal is
/// counted, and the setting shows the reason. An accepted write that fails
/// later is a different result and logs no refusal.
#[test]
fn a_preference_refused_at_queue_admission_logs_one_refusal_at_the_command_boundary() {
    use crate::nav_prefs::NavPreference as N;
    let mut d = driver("s3-queue-refusal", &["alice"]);
    d.poll();
    let disk = Disk::install(&d, 1);
    let refusals = d.session().refusals();
    let setting = crate::Setting::Nav(N::AllowTeleports);
    let mut refused = Vec::new();
    let mut accepted = Vec::new();
    for index in 0..24 {
        let on = index % 2 == 0;
        let request = d
            .submit(
                OperatorCommand::SetNavPreference(N::AllowTeleports, on),
                Targets::Application,
            )
            .expect("the command is admitted")
            .request();
        match terminal_app(&d, request) {
            Some(Settlement::Failed(CommandFailure::Rejected(
                crate::command::Rejection::WriteRefused,
            ))) => refused.push(request),
            None => accepted.push(request),
            other => panic!("neither accepted nor refused by the queue: {other:?}"),
        }
    }
    assert!(
        !refused.is_empty() && !accepted.is_empty(),
        "the queue accepted some and refused the rest ({} / {})",
        accepted.len(),
        refused.len()
    );
    assert_eq!(
        d.session().refusals(),
        refusals + refused.len() as u64,
        "one refusal counted per refused command, none for the accepted ones"
    );
    let mut view = crate::log::LogView::new(crate::log::LogScope::Process);
    view.edit_filter(|filter| filter.set_text("command refused: SetNavPreference"));
    crate::log::global().refresh(&mut view);
    let lines = view
        .rows()
        .iter()
        .filter(|row| row.message().contains("WriteRefused"))
        .count();
    assert_eq!(lines, refused.len(), "one info line per refused command");
    assert!(
        d.session()
            .setting_failure(setting)
            .is_some_and(|text| text.starts_with("Not saved: ")),
        "the setting shows the reason"
    );
    // The accepted writes settle on their own: no further refusal line.
    drop(disk);
    for request in accepted {
        assert_eq!(settled_app(&mut d, request), Some(done()));
    }
    assert_eq!(d.session().refusals(), refusals + refused.len() as u64);
}
#[test]
fn skip_quest_acceptance_uses_the_shared_driver_command_and_reports_the_identity() {
    use script::native::{NativeTick, Script, ScriptFailure, ScriptFlow, SkippedQuest};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct SkipProbe(Arc<AtomicUsize>);

    impl Script for SkipProbe {
        fn tick(&mut self, _tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            Ok(ScriptFlow::Continue)
        }

        fn skip_quest(&mut self) -> Result<SkippedQuest, ScriptFailure> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(SkippedQuest {
                quest_id: Arc::from("driver-probe-quest"),
                step_id: Arc::from("driver-probe-step"),
            })
        }
    }

    let mut d = driver("skip-quest-accepted", &["alice"]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    settle(&mut d, load.request());
    let calls = Arc::new(AtomicUsize::new(0));
    d.session()
        .play_for_test()
        .expect("play")
        .script_start_handle()
        .start_test_script_with_card(
            "alice",
            script::CompiledId("Quester"),
            Box::new(SkipProbe(Arc::clone(&calls))),
            None,
        )
        .expect("start native probe");
    wait_state(&mut d, "alice", ScriptRunState::Running);

    let target = d
        .session()
        .play_for_test()
        .expect("play")
        .script_native_status("alice")
        .expect("running native status")
        .run;
    let refusals = d.session().refusals();
    let skip = d
        .submit(
            OperatorCommand::SkipQuest {
                target: Some(target),
            },
            Targets::Profile("alice"),
        )
        .expect("skip submission")
        .request();
    settle(&mut d, skip);
    assert_eq!(terminal(&d, skip, "alice"), Some(done()));
    let detail = d.report(skip).and_then(|report| {
        report
            .members()
            .first()
            .and_then(|member| member.detail())
            .map(str::to_owned)
    });
    assert_eq!(
        detail.as_deref(),
        Some("quest_id=driver-probe-quest step_id=driver-probe-step")
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(d.session().refusals(), refusals);
    assert_eq!(d.session().script_state("alice"), ScriptRunState::Running);

}

#[test]
fn skip_quest_without_a_running_queue_is_refused_with_one_shared_log_line() {
    let name = "skiprefusal";
    let mut d = driver("skip-quest-refused", &[name]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Profile(name))
        .expect("load");
    settle(&mut d, load.request());

    let refusals = d.session().refusals();
    let skip = d
        .submit(
            OperatorCommand::SkipQuest { target: None },
            Targets::Profile(name),
        )
        .expect("skip submission")
        .request();
    settle(&mut d, skip);
    assert!(matches!(
        terminal(&d, skip, name),
        Some(Settlement::Failed(CommandFailure::Rejected(
            crate::command::Rejection::NotApplicable
        )))
    ));
    assert_eq!(
        d.session().command_failure(skip),
        Some("bot is not running the Quester queue")
    );
    assert_eq!(d.session().refusals(), refusals + 1);
    let mut view = crate::log::LogView::new(crate::log::LogScope::Process);
    view.edit_filter(|filter| filter.set_text("command refused: SkipQuest on skiprefusal"));
    crate::log::global().refresh(&mut view);
    let refused = view
        .rows()
        .iter()
        .filter(|row| {
            row.message()
                .contains("bot is not running the Quester queue")
        })
        .count();
    assert_eq!(refused, 1, "one shared command refusal log line");
}
