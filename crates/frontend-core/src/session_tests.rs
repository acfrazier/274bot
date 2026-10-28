//! Lifecycle proofs through the real host seam: a real [`host_play::Play`]
//! (no server contact), real [`SlotArm`] control flags, real script slots.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use host_play::{InstancePermit, Play, PlayOptions, SlotArm, SlotStatus, StartupPhase};
use vault::{Profile, ProfileSettings, Vault};

use super::*;
use crate::command::OperatorCommand;
#[cfg(feature = "js")]
use crate::scripts::LoadShape;
use crate::surface::{SlotAttach, SlotSurface};
use crate::ArmMirror;
use crate::WalkGlobals;
use api::lifecycle::MemoryMode;

/// Records lifecycle callbacks; IO values count attaches so reuse is visible.
#[derive(Default)]
struct Recorder {
    attached: u32,
    resets: Vec<String>,
    released: Vec<String>,
    rasters: Vec<crate::command::RasterMode>,
}

impl SlotSurface for Recorder {
    type Io = u32;

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut crate::SpawnProfile<'_>,
        retained: Option<u32>,
    ) -> SlotAttach<u32> {
        self.rasters.push(profile.raster());
        let io = retained.unwrap_or_else(|| {
            self.attached += 1;
            self.attached
        });
        SlotAttach::new(io, None, None)
    }

    fn lifetime_reset(&mut self, name: &str, _walk_was_armed: bool) {
        self.resets.push(name.to_string());
    }

    fn released(&mut self, name: &str) {
        self.released.push(name.to_string());
    }

    fn models(&mut self) -> crate::Models<'_> {
        crate::Models::none()
    }

    fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {}
}

/// The auto-login row for `name` through the command seam; its operation.
fn submit_auto_login<Io>(s: &mut OperatorSession<Io>, name: &str, on: bool) -> OperationId {
    s.submit_detached(OperatorCommand::SetAutoLogin(on), Targets::Profile(name))
        .unwrap()
        .operation()
        .expect("the write opened an operation")
}

/// The memory-mode row for `name` through the command seam; its operation.
fn submit_memory_mode<Io>(s: &mut OperatorSession<Io>, name: &str, lowmem: bool) -> OperationId {
    let mode = if lowmem {
        MemoryMode::Low
    } else {
        MemoryMode::High
    };
    s.submit_detached(OperatorCommand::SetMemoryMode(mode), Targets::Profile(name))
        .unwrap()
        .operation()
        .expect("the write opened an operation")
}

/// The profile-delete row for `name` through the command seam; its operation.
fn submit_vault_remove<Io>(s: &mut OperatorSession<Io>, name: &str) -> OperationId {
    s.submit_detached(OperatorCommand::DeleteProfile, Targets::Profile(name))
        .unwrap()
        .operation()
        .expect("the delete opened an operation")
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

#[test]
fn walk_globals_publish_before_start_and_to_the_current_play() {
    let mut session = OperatorSession::<()>::new(InstancePermit::SkipLock);
    let granted = WalkGlobals::new(true, true, true, true, false);
    session.set_walk_globals(granted);
    session.start(vault_with("walk-globals", &[]), empty_play());
    assert_eq!(session.play().unwrap().walk_globals(), granted.to_host());
    session.set_walk_globals(WalkGlobals::default());
    assert_eq!(
        session.play().unwrap().walk_globals(),
        WalkGlobals::default().to_host()
    );
}

fn vault_path(test: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!(
            "274bot-frontend-core-{}-{test}",
            std::process::id()
        ))
        .join("vault")
}

/// A vault file kept aside while a directory sits at its name: the next write
/// cannot publish over a directory (on any platform), and the durable copy is
/// intact for [`unblock_writes`].
struct BlockedWrites {
    path: std::path::PathBuf,
    aside: std::path::PathBuf,
}

fn block_writes(path: &std::path::Path) -> BlockedWrites {
    let aside = path.with_extension("aside");
    std::fs::rename(path, &aside).unwrap();
    std::fs::create_dir(path).unwrap();
    BlockedWrites {
        path: path.to_path_buf(),
        aside,
    }
}

fn unblock_writes(blocked: BlockedWrites) {
    std::fs::remove_dir(&blocked.path).unwrap();
    std::fs::rename(&blocked.aside, &blocked.path).unwrap();
}

fn vault_with(test: &str, profiles: &[(&str, i32, bool)]) -> Vault {
    let path = vault_path(test);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "test-passphrase-01").unwrap();
    for (name, uid, auto_login) in profiles {
        vault
            .upsert(Profile {
                username: name.to_string(),
                password: "pw".into(),
                uid: *uid,
                settings: ProfileSettings {
                    auto_login: *auto_login,
                    ..ProfileSettings::default()
                },
            })
            .unwrap();
    }
    vault
}

/// Session over a real empty Play; spawns attach arms without worker threads.
fn session(test: &str, profiles: &[(&str, i32, bool)]) -> OperatorSession<u32> {
    let mut session = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.start(vault_with(test, profiles), empty_play());
    session
}

#[cfg(feature = "js")]
fn test_scripts(dir: &std::path::Path) -> crate::scripts::Scripts {
    crate::scripts::Scripts::new(
        script_js::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    )
}

#[cfg(not(feature = "js"))]
fn test_scripts(_dir: &std::path::Path) -> crate::scripts::Scripts {
    crate::scripts::Scripts::new()
}

#[cfg(not(feature = "js"))]
fn wait_compiled_script_state(
    session: &mut OperatorSession<u32>,
    scripts: &mut crate::scripts::Scripts,
    profile: &str,
    expected: script::RunState,
) {
    let guard = Instant::now() + HANG_GUARD;
    loop {
        session.poll();
        scripts.poll(session);
        let state = session.play().unwrap().script_state(profile);
        if state == expected {
            return;
        }
        assert_ne!(
            state,
            script::RunState::Error,
            "{profile} ended in {state:?}"
        );
        assert!(
            Instant::now() < guard,
            "{profile} never reached {expected:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(not(feature = "js"))]
fn start_compiled_script(
    session: &mut OperatorSession<u32>,
    scripts: &mut crate::scripts::Scripts,
    profile: &str,
) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("the selected script revision has game data");
    session.play_mut().unwrap().bind_script_test_data(data);
    assert!(scripts.persist_assignment(
        session,
        profile,
        script::compiled_assignment(script::CompiledId("Gatherer")),
    ));
    session.flush_writes();
    scripts.start_profile(session, profile, None).unwrap();
    wait_compiled_script_state(session, scripts, profile, script::RunState::Running);
}

#[test]
fn new_profile_form_refuses_an_empty_password() {
    let iso = crate::test_support::IsolatedStores::enter("profile-editor-empty-password");
    let mut core = session("profile-editor-empty-password", &[]);
    let scripts = test_scripts(iso.dir());
    let mut editor = crate::ProfileEditor::new();
    editor.open(&core, "");
    editor.username_mut().push_str("alice");

    assert_eq!(
        editor.save(&mut core, &scripts, OperatorCommand::CreateProfile),
        Err(crate::SaveRefused::Refused)
    );
    assert_eq!(
        editor
            .save_state()
            .notice()
            .and_then(|notice| notice.error()),
        Some("credentials: password required")
    );
    assert!(core.profile("alice").is_none(), "nothing was saved");
    core.flush_writes();
    assert!(core.profile("alice").is_none(), "nothing was queued");
}

fn arm(session: &OperatorSession<u32>, name: &str) -> Arc<SlotArm> {
    session.play().unwrap().arm(name).expect("live arm")
}

fn push_status(session: &OperatorSession<u32>, row: SlotStatus) {
    session.play().unwrap().statuses.lock().unwrap().push(row);
}

fn set_status(session: &OperatorSession<u32>, row: SlotStatus) {
    let mut rows = session.play().unwrap().statuses.lock().unwrap();
    rows.retain(|existing| existing.username != row.username);
    rows.push(row);
}

fn row(name: &str) -> SlotStatus {
    SlotStatus {
        username: name.into(),
        ..SlotStatus::default()
    }
}

fn publish_login(session: &mut OperatorSession<u32>, name: &str, scene_state: i32) {
    set_status(
        session,
        SlotStatus {
            connected: true,
            ingame: true,
            scene_state,
            ..row(name)
        },
    );
    session.poll();
}

#[test]
fn load_all_does_not_move_selection_when_others_log_in() {
    let mut s = session(
        "load-all-keeps-sel",
        &[
            ("alice", 1, false),
            ("bob", 2, true),
            ("carol", 3, true),
            ("dave", 4, true),
        ],
    );
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    s.select("alice");
    assert_eq!(s.selected(), Some("alice"));

    let (op, added) = s.load_all(&mut surface, None);
    assert_eq!(added, 3);
    assert_eq!(
        s.operation(op).unwrap().outcome("bob"),
        Some(&Outcome::Completed)
    );
    assert_eq!(s.selected(), Some("alice"), "Load all is not selection");
    s.login_all(&mut surface);

    for name in ["bob", "carol", "dave"] {
        publish_login(&mut s, name, 1);
        assert_eq!(
            s.selected(),
            Some("alice"),
            "{name} at scene_state==1 must not take focus"
        );
        publish_login(&mut s, name, 2);
        assert_eq!(
            s.selected(),
            Some("alice"),
            "{name} logging in must not take focus"
        );
    }
    assert_eq!(s.selected(), Some("alice"));
    assert_eq!(
        s.members(),
        [
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "dave".to_string()
        ]
    );
}

#[test]
fn pick_focus_prefers_last_when_present() {
    let names = vec!["a".into(), "b".into()];
    assert_eq!(pick_focus(&names, Some("b")).as_deref(), Some("b"));
    assert_eq!(pick_focus(&names, Some("z")).as_deref(), Some("a"));
    assert_eq!(pick_focus(&names, None).as_deref(), Some("a"));
    assert_eq!(pick_focus(&[], Some("a")), None);
}

#[test]
fn load_all_picks_focus_when_none_was_selected() {
    let mut s = session(
        "load-all-no-sel",
        &[("alice", 1, true), ("bob", 2, true), ("carol", 3, true)],
    );
    let mut surface = Recorder::default();
    s.load_all(&mut surface, None);
    let picked = pick_focus(s.members(), None);
    assert_eq!(s.selected(), picked.as_deref());
    assert_eq!(picked.as_deref(), Some("alice"));
    s.login_all(&mut surface);
    publish_login(&mut s, "alice", 2);
    publish_login(&mut s, "bob", 2);
    assert_eq!(
        s.selected(),
        Some("alice"),
        "later logins must not steal the Load all pick"
    );
}

#[test]
fn load_all_picks_last_focus_when_nothing_was_selected() {
    let mut s = session(
        "load-all-last-focus",
        &[("alice", 1, true), ("bob", 2, true), ("carol", 3, true)],
    );
    let mut surface = Recorder::default();
    s.load_all(&mut surface, Some("carol"));
    assert_eq!(s.selected(), Some("carol"));
    s.login_all(&mut surface);
    publish_login(&mut s, "alice", 1);
    publish_login(&mut s, "alice", 2);
    assert_eq!(
        s.selected(),
        Some("carol"),
        "later logins must not steal the last-focus pick"
    );
}

#[test]
fn load_follows_saved_auto_login_while_login_arms_an_explicit_handshake() {
    let mut s = session("load-vs-login", &[("auto", 1, true), ("manual", 2, false)]);
    let mut surface = Recorder::default();

    let (op, added) = s.load("manual", &mut surface);
    assert!(added);
    assert_eq!(
        s.operation(op).unwrap().outcome("manual"),
        Some(&Outcome::Completed)
    );
    assert!(
        !arm(&s, "manual").wants_login(),
        "Load of a non-auto profile must not log it in"
    );
    s.load("auto", &mut surface);
    assert!(
        arm(&s, "auto").wants_login(),
        "Load follows saved auto-login"
    );
    assert_eq!(s.members(), ["auto".to_string(), "manual".to_string()]);
    assert_eq!(s.selected(), None, "Load is not selection");

    let login = s.login("manual", &mut surface);
    assert!(arm(&s, "manual").wants_login());
    assert!(!arm(&s, "manual").auto_login());
    assert_eq!(
        s.operation(login).unwrap().outcome("manual"),
        Some(&Outcome::Pending),
        "an accepted Login is not success before the slot is in game"
    );
    push_status(
        &s,
        SlotStatus {
            connected: true,
            ingame: true,
            scene_state: 2,
            ..row("manual")
        },
    );
    s.poll();
    assert_eq!(
        s.operation(login).unwrap().outcome("manual"),
        Some(&Outcome::Completed)
    );
}

#[test]
fn logout_cancels_a_queued_login_and_latches_until_the_next_login() {
    let mut s = session("queued-cancel", &[("alice", 1, true)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let login = s.login("alice", &mut surface);
    push_status(
        &s,
        SlotStatus {
            startup_phase: StartupPhase::Queueing,
            queue_position: 2,
            queue_total: 3,
            ..row("alice")
        },
    );
    s.poll();
    assert_eq!(
        s.operation(login).unwrap().outcome("alice"),
        Some(&Outcome::Pending)
    );

    let logout = s.logout("alice");
    let alice = arm(&s, "alice");
    assert!(!alice.wants_login(), "a queued handshake is withdrawn");
    assert!(alice.login_latched());
    assert_eq!(
        s.operation(login).unwrap().outcome("alice"),
        Some(&Outcome::Cancelled)
    );
    s.poll();
    assert_eq!(
        s.operation(logout).unwrap().outcome("alice"),
        Some(&Outcome::Completed),
        "a never-connected slot is logged out at once"
    );

    // A re-Load keeps the operator latch; only Log in lifts it.
    s.load("alice", &mut surface);
    assert!(!arm(&s, "alice").wants_login());
    s.login_all(&mut surface);
    assert!(arm(&s, "alice").wants_login());
    assert!(!s.fleet().latched("alice"));
}

#[test]
fn login_recreates_a_terminal_worker_with_its_retained_io_but_select_does_not() {
    let mut s = session("terminal-recreate", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let io = *s.slot_io("alice").unwrap();
    // The worker ended by itself: its thread finished and its row is terminal.
    let dead = arm(&s, "alice");
    let play = s.play_mut().unwrap();
    play.attach_finished_worker_for_test("alice", Arc::clone(&dead));
    play.statuses.lock().unwrap().push(SlotStatus {
        startup_phase: StartupPhase::Error,
        worker_terminal: Some(host_play::WorkerTerminal::Panicked),
        error: Some("slot worker panicked: synthetic".into()),
        ..row("alice")
    });
    s.poll();
    assert!(s.play().unwrap().arm("alice").is_none(), "poll reaps it");

    s.open_slot("alice", &mut surface).unwrap();
    s.select("alice");
    assert!(
        s.play().unwrap().arm("alice").is_none(),
        "selection alone must not replace a terminal worker"
    );
    assert_eq!(
        s.status("alice").and_then(|r| r.worker_terminal),
        Some(host_play::WorkerTerminal::Panicked)
    );

    let login = s.login("alice", &mut surface);
    let replacement = arm(&s, "alice");
    assert!(!Arc::ptr_eq(&replacement, &dead));
    assert!(replacement.wants_login());
    assert_eq!(s.slot_io("alice"), Some(&io), "the retained IO is reused");
    assert_eq!(
        s.operation(login).unwrap().outcome("alice"),
        Some(&Outcome::Pending)
    );
}

/// Member `b`, just dropped from the wall, whose worker has not exited
/// yet: it keeps running until the returned sender is dropped. The play
/// really spawns workers, at a port nothing listens on, so a login fails
/// and retries without contacting a server. Also returns the drop's Remove.
fn member_still_stopping(
    test: &str,
) -> (
    OperatorSession<u32>,
    Recorder,
    std::sync::mpsc::Sender<()>,
    OperationId,
) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let play = host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
            port,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let mut s = OperatorSession::new(InstancePermit::SkipLock);
    s.set_bypass_asset_startup(true);
    s.start(vault_with(test, &[("b", 2, false)]), play);
    let mut surface = Recorder::default();
    s.fleet_mut().add("b");
    let release = s
        .play_mut()
        .unwrap()
        .attach_blocked_worker_for_test("b", SlotArm::new(2, false));
    let remove = s.remove("b", Instant::now(), &mut surface).op;
    assert!(s.play().unwrap().slot_stopping("b"));
    (s, surface, release, remove)
}

/// Hang guard for every wait in this file, never a pass condition and never a
/// latency target. Each wait ends on the product's own settlement signal (a
/// settled Start, a settled operation, a worker that has exited, a published
/// line), and every accepted job and every stopping worker reaches one. Start
/// preparation shares one process-wide background worker with every other test
/// in this binary, so a wait on a loaded host queues behind all of them. The
/// guard only stops a deadlocked test and is minutes, not the expected duration.
const HANG_GUARD: Duration = Duration::from_secs(600);

/// Poll the session, as a front end does once per frame, until `done` holds.
/// `done` runs before the first poll and after every poll.
fn pump_until<Io>(
    s: &mut OperatorSession<Io>,
    what: &str,
    mut done: impl FnMut(&mut OperatorSession<Io>) -> bool,
) {
    let guard = Instant::now() + HANG_GUARD;
    while !done(s) {
        assert!(Instant::now() < guard, "{what}: hang guard expired");
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Poll until `op`'s outcome for alice is no longer Pending; the caller
/// asserts which terminal it reached.
fn wait_operation_settled<Io>(s: &mut OperatorSession<Io>, op: OperationId) {
    pump_until(s, "operation did not settle", |s| {
        s.operation(op).unwrap().outcome("alice") != Some(&Outcome::Pending)
    });
}

/// Let `b`'s old worker exit, then poll once.
fn finish_stopping(s: &mut OperatorSession<u32>, release: std::sync::mpsc::Sender<()>) {
    drop(release);
    let guard = Instant::now() + HANG_GUARD;
    while s.play().unwrap().slot_stopping("b") {
        assert!(
            Instant::now() < guard,
            "b's worker did not exit: hang guard expired"
        );
        std::thread::yield_now();
    }
    s.poll();
}

/// Operator report (Windows, 0.1.9): drop a wall member, then Load all and
/// Login all at once. The dropped member's worker had not exited yet, so
/// both spawns were refused and nothing brought it back: it came up later
/// parked "logged out". Its spawn now waits for the old worker, and the
/// Login all issued meanwhile reaches the new one.
#[test]
fn login_all_right_after_load_all_reaches_a_member_whose_old_worker_is_still_stopping() {
    let (mut s, mut surface, release, remove) = member_still_stopping("load-login-stopping");

    let (load, _) = s.load_all(&mut surface, None);
    let login = s.login_all(&mut surface);
    assert_eq!(s.members(), ["b".to_string()]);
    assert_eq!(
        s.operation(load).unwrap().outcome("b"),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        s.operation(login).unwrap().outcome("b"),
        Some(&Outcome::Pending),
        "the Login waits for the new worker, it does not fail"
    );
    assert_eq!(
        s.operation(remove).unwrap().outcome("b"),
        Some(&Outcome::Pending),
        "the old worker has not exited yet"
    );

    finish_stopping(&mut s, release);
    let b = s
        .play()
        .unwrap()
        .arm("b")
        .expect("b's worker spawns once its predecessor exited");
    assert!(b.wants_login(), "the Login all reached the new worker");
    assert!(!b.login_latched());
    assert_eq!(
        s.operation(remove).unwrap().outcome("b"),
        Some(&Outcome::Completed),
        "the removal is complete once its worker exited"
    );
    s.play_mut().unwrap().stop_slot("b");
}

#[test]
fn a_logout_issued_while_the_spawn_waits_holds_the_new_worker_logged_out() {
    let (mut s, mut surface, release, _) = member_still_stopping("load-logout-stopping");
    s.load_all(&mut surface, None);
    s.login_all(&mut surface);
    s.logout("b");

    finish_stopping(&mut s, release);
    let b = s.play().unwrap().arm("b").expect("b's worker spawns");
    assert!(b.login_latched(), "the later Logout wins");
    assert!(!b.wants_login());
    s.play_mut().unwrap().stop_slot("b");
}

#[test]
fn memory_toggle_while_spawn_is_deferred_updates_its_profile_and_arm() {
    let (mut s, mut surface, release, _) = member_still_stopping("deferred-memory-toggle");
    s.load_all(&mut surface, None);

    submit_memory_mode(&mut s, "b", false);
    let deferred = s.deferred.get("b").expect("replacement spawn is deferred");
    assert!(
        !deferred.profile.settings.lowmem,
        "the disposable spawn profile follows the toggle"
    );
    assert_eq!(
        deferred.arm.as_ref().and_then(|arm| arm.lowmem_handshake()),
        Some(false),
        "the deferred arm agrees with the panel gate before spawn"
    );

    finish_stopping(&mut s, release);
    assert_eq!(
        s.play()
            .unwrap()
            .arm("b")
            .expect("b's replacement worker spawns")
            .lowmem_handshake(),
        Some(false),
        "the spawned profile and arm retain the toggled mode"
    );
    s.play_mut().unwrap().stop_slot("b");
}

#[test]
fn failed_memory_toggle_resets_a_deferred_profile_and_arm() {
    let (mut s, mut surface, release, _) = member_still_stopping("deferred-memory-toggle-fail");
    s.load_all(&mut surface, None);
    let blocked = block_writes(&vault_path("deferred-memory-toggle-fail"));

    submit_memory_mode(&mut s, "b", false);
    s.flush_writes();
    unblock_writes(blocked);

    let deferred = s.deferred.get("b").expect("replacement spawn is deferred");
    assert!(
        deferred.profile.settings.lowmem,
        "a refused toggle restores the disposable spawn profile"
    );
    assert_eq!(
        deferred.arm.as_ref().and_then(|arm| arm.lowmem_handshake()),
        Some(true),
        "a refused toggle restores the deferred arm"
    );

    finish_stopping(&mut s, release);
    assert_eq!(
        s.play()
            .unwrap()
            .arm("b")
            .expect("b's replacement worker spawns")
            .lowmem_handshake(),
        Some(true),
        "the replacement worker starts on the restored durable mode"
    );
    s.play_mut().unwrap().stop_slot("b");
}

/// A member removed and loaded again before a poll saw its removal settle
/// (its worker already gone): the Remove completes when the new lifetime
/// starts. Pending reports are never evicted, so a Remove left pending by
/// each cycle would grow the operation history without bound.
#[test]
fn repeated_remove_and_reload_keeps_the_operation_history_bounded() {
    let mut s = session("remove-reload-cycle", &[("a", 1, false)]);
    let mut surface = Recorder::default();
    s.load("a", &mut surface);
    let mut removes = Vec::new();
    for _ in 0..80 {
        removes.push(s.remove("a", Instant::now(), &mut surface).op);
        s.load("a", &mut surface);
        s.poll();
    }
    assert!(
        s.operation(removes[0]).is_none(),
        "settled history is evicted"
    );
    let kept = removes
        .iter()
        .filter(|op| s.operation(**op).is_some())
        .count();
    assert!(kept < 64, "{kept} Remove reports retained");
    let last = *removes.last().unwrap();
    assert_eq!(
        s.operation(last).unwrap().outcome("a"),
        Some(&Outcome::Completed)
    );
}

#[test]
fn removing_the_selected_member_selects_its_neighbour_and_settles_after_disconnect() {
    let mut s = session(
        "remove-neighbour",
        &[("a", 1, false), ("b", 2, false), ("c", 3, false)],
    );
    let mut surface = Recorder::default();
    for name in ["a", "b", "c"] {
        s.load(name, &mut surface);
    }
    s.select("b");
    push_status(
        &s,
        SlotStatus {
            connected: true,
            ..row("b")
        },
    );
    let b = arm(&s, "b");
    let started = Instant::now();

    let removal = s.remove("b", started, &mut surface);
    assert_eq!(removal.reselected.as_deref(), Some("a"));
    assert_eq!(s.selected(), Some("a"));
    assert_eq!(s.play().unwrap().focused().as_deref(), Some("a"));
    assert_eq!(s.members(), ["a".to_string(), "c".to_string()]);
    assert!(b.wants_logout(), "a connected member gets a clean logout");
    assert!(
        !b.stop.load(Ordering::Relaxed),
        "removal never stops inline"
    );
    assert_eq!(surface.released, ["b".to_string()]);
    s.poll_at(started);
    assert_eq!(
        s.operation(removal.op).unwrap().outcome("b"),
        Some(&Outcome::Pending)
    );

    s.play().unwrap().statuses.lock().unwrap()[0].connected = false;
    s.poll_at(started + Duration::from_millis(1));
    assert!(b.stop.load(Ordering::Relaxed));
    assert!(!s.removal_pending("b"));
    assert_eq!(
        s.operation(removal.op).unwrap().outcome("b"),
        Some(&Outcome::Completed)
    );

    s.select("c");
    let removal = s.remove("c", started, &mut surface);
    assert_eq!(removal.reselected.as_deref(), Some("a"));
    let last = s.remove("a", started, &mut surface);
    assert!(last.selection_cleared, "no member remains to select");
    assert_eq!(s.selected(), None);
}

#[test]
fn removal_times_out_a_member_that_never_disconnects() {
    let mut s = session("remove-timeout", &[("a", 1, false)]);
    let mut surface = Recorder::default();
    s.load("a", &mut surface);
    push_status(
        &s,
        SlotStatus {
            connected: true,
            ..row("a")
        },
    );
    let a = arm(&s, "a");
    let started = Instant::now();
    s.remove("a", started, &mut surface);
    s.poll_at(started + SLOT_REMOVE_TIMEOUT - Duration::from_nanos(1));
    assert!(!a.stop.load(Ordering::Relaxed));
    s.poll_at(started + SLOT_REMOVE_TIMEOUT);
    assert!(a.stop.load(Ordering::Relaxed));
}

#[test]
fn reloading_during_removal_cancels_it_and_keeps_the_same_lifetime() {
    let mut s = session("remove-reload", &[("a", 1, true)]);
    let mut surface = Recorder::default();
    s.load("a", &mut surface);
    push_status(
        &s,
        SlotStatus {
            connected: true,
            ..row("a")
        },
    );
    let a = arm(&s, "a");
    let io = *s.slot_io("a").unwrap();
    let started = Instant::now();
    let removal = s.remove("a", started, &mut surface);
    assert!(s.slot_io("a").is_none());

    s.load("a", &mut surface);
    assert_eq!(s.slot_io("a"), Some(&io));
    assert!(
        a.wants_login(),
        "re-adding undoes the cancelled removal's logout for an auto member"
    );
    assert_eq!(
        s.operation(removal.op).unwrap().outcome("a"),
        Some(&Outcome::Cancelled)
    );
    s.play().unwrap().statuses.lock().unwrap()[0].connected = false;
    s.poll_at(started + SLOT_REMOVE_TIMEOUT);
    assert!(Arc::ptr_eq(&arm(&s, "a"), &a));
    assert!(!a.stop.load(Ordering::Relaxed));
}

#[cfg(feature = "js")]
fn looping_bot(dir: &std::path::Path) -> (String, LoadShape) {
    let path = dir.join("looping.ts");
    std::fs::write(
        &path,
        "export default class T extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    let mut library = script_js::JsLibrary::new(dir.join("js-scripts.json"));
    let card = library.load(&path).unwrap();
    (card.js, LoadShape::from_script(card.shape))
}

#[cfg(feature = "js")]
fn settle_start(s: &mut OperatorSession<u32>) -> Vec<StartSettled> {
    let mut settled = Vec::new();
    pump_until(s, "script Start did not settle", |s| {
        settled = s.take_settled_starts();
        !settled.is_empty()
    });
    settled
}

#[cfg(feature = "js")]
#[test]
fn offline_start_and_stop_settle_through_poll() {
    let iso = script::IsolatedEnv::enter("frontend-core-offline-start");
    let mut s = session("offline-start", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    assert!(!arm(&s, "alice").wants_login(), "the slot stays offline");
    let (js, shape) = looping_bot(&iso.dir);

    let op = s
        .start_script(
            "alice",
            ScriptStart::load(js, shape, None, Vec::new()),
            Some("file:looping".into()),
        )
        .unwrap();
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Pending)
    );
    let settled = settle_start(&mut s);
    assert_eq!(
        settled,
        vec![StartSettled {
            slot: "alice".into(),
            op,
            outcome: Some(script::StartOutcome::Ready),
        }]
    );
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        s.play().unwrap().script_source_identity("alice").as_deref(),
        Some("file:looping")
    );

    let pause = s.toggle_pause("alice");
    s.poll();
    assert_eq!(
        s.operation(pause).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    let resume = s.toggle_pause("alice");
    assert_eq!(
        s.operation(resume).unwrap().action(),
        ActionKind::ScriptResume
    );

    let stop = s.stop_script("alice");
    wait_operation_settled(&mut s, stop);
    assert_eq!(
        s.operation(stop).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        s.play().unwrap().script_state("alice"),
        script::RunState::Idle
    );
}

#[cfg(feature = "js")]
#[test]
fn a_start_refused_by_the_host_is_an_error_not_an_operation() {
    let mut s = session("start-refused", &[]);
    let refused = s.start_script(
        "ghost",
        ScriptStart::load(String::new(), LoadShape::Reject, None, Vec::new()),
        None,
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "no slot: ghost",
        "no slot, no pending operation"
    );
    assert!(s.last_operation().is_none());
}

#[test]
fn transitions_report_each_change_once() {
    let mut s = session("transitions", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    push_status(&s, row("alice"));
    s.poll();
    assert_eq!(
        s.transitions(),
        [SlotTransition {
            slot: "alice".into(),
            transition: Transition::SlotUp,
        }]
    );
    s.poll();
    assert!(s.transitions().is_empty());
    {
        let play = s.play().unwrap();
        let mut rows = play.statuses.lock().unwrap();
        rows[0].ingame = true;
        rows[0].scene_state = 2;
    }
    s.poll();
    assert_eq!(
        s.transitions()
            .iter()
            .map(|t| t.transition())
            .collect::<Vec<_>>(),
        [&Transition::Ingame, &Transition::scene(2)]
    );
}

/// Rows of `slot`'s ring in the shared store (tests use unique slot names:
/// the store is process-wide).
fn slot_log(slot: &str) -> Vec<(Source, Level, String)> {
    let mut view = crate::log::LogView::new(crate::log::LogScope::slot(slot));
    view.edit_filter(|f| f.set_min_level(Level::Debug));
    crate::log::global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|e| (e.source(), e.level(), e.message().to_string()))
        .collect()
}

fn assignment(identity: &str) -> vault::ScriptAssignment {
    vault::ScriptAssignment {
        source_kind: "compiled".into(),
        identity: identity.into(),
        display_name: identity.into(),
        unavailable: None,
    }
}

#[test]
fn durable_assignment_logs_identity_availability_and_skips_parameter_saves() {
    let name = "assignment-log-success";
    let mut s = session("assignment-log-success", &[(name, 1, false)]);
    let mut profile = s.vault().unwrap().get(name).unwrap().clone();
    profile.settings.script_assignment = Some(assignment("one"));
    let first = s
        .save_profile_with_log_action(profile, ArmMirror::none(), "script", "assignment")
        .unwrap();
    s.flush_writes();
    assert_eq!(
        slot_log(name),
        [(
            Source::Host,
            Level::Info,
            format!(
                "script assignment None -> compiled:one; action assignment; op#{}",
                first.0
            ),
        )]
    );

    let mut parameters = s.vault().unwrap().get(name).unwrap().clone();
    parameters
        .settings
        .script_settings
        .insert("compiled:one".into(), serde_json::Map::new());
    s.save_profile_with_log_action(parameters, ArmMirror::none(), "script", "parameter edit")
        .unwrap();
    s.flush_writes();

    let unchanged = s.vault().unwrap().get(name).unwrap().clone();
    s.save_profile_with_log_action(unchanged, ArmMirror::none(), "script", "assignment")
        .unwrap();
    s.flush_writes();
    assert_eq!(slot_log(name).len(), 1);

    let mut unavailable = s.vault().unwrap().get(name).unwrap().clone();
    let mut saved_assignment = unavailable.settings.script_assignment.clone().unwrap();
    saved_assignment.unavailable = Some("missing".into());
    unavailable.settings.script_assignment = Some(saved_assignment);
    let second = s
        .save_profile_with_log_action(unavailable, ArmMirror::none(), "script", "unavailable")
        .unwrap();
    s.flush_writes();
    assert_eq!(
        slot_log(name).last().unwrap(),
        &(
            Source::Host,
            Level::Info,
            format!(
                "script assignment compiled:one -> compiled:one; availability available -> unavailable: missing; action unavailable; op#{}",
                second.0
            ),
        )
    );
}

#[test]
fn assignment_logs_only_the_final_value_and_owner_of_a_coalesced_commit() {
    let final_profile = "assignment-log-final";
    let cancelled_profile = "assignment-log-cancelled";
    let mut s = session(
        "assignment-log-coalesced",
        &[(final_profile, 1, false), (cancelled_profile, 2, false)],
    );
    let gate = s.write_gate();
    let held = gate.lock().unwrap();

    let mut transient = s.vault().unwrap().get(cancelled_profile).unwrap().clone();
    transient.settings.script_assignment = Some(assignment("never-durable"));
    s.save_profile_with_log_action(transient, ArmMirror::none(), "script", "assignment")
        .unwrap();
    let mut restored = s.vault().unwrap().get(cancelled_profile).unwrap().clone();
    restored.settings.script_assignment = None;
    s.save_profile_with_log_action(restored, ArmMirror::none(), "script", "parameter edit")
        .unwrap();

    let mut intermediate = s.vault().unwrap().get(final_profile).unwrap().clone();
    intermediate.settings.script_assignment = Some(assignment("intermediate"));
    s.save_profile_with_log_action(intermediate, ArmMirror::none(), "script", "assignment")
        .unwrap();
    let mut final_value = s.vault().unwrap().get(final_profile).unwrap().clone();
    final_value.settings.script_assignment = Some(assignment("final"));
    let final_op = s
        .save_profile_with_log_action(final_value, ArmMirror::none(), "script", "Start")
        .unwrap();

    drop(held);
    s.flush_writes();
    assert!(slot_log(cancelled_profile).is_empty());
    assert_eq!(
        slot_log(final_profile),
        [(
            Source::Host,
            Level::Info,
            format!(
                "script assignment None -> compiled:final; action Start; op#{}",
                final_op.0
            ),
        )]
    );
}

#[test]
fn assignment_logs_ignore_rename_without_assignment_change() {
    let old_name = "assignment-log-rename-old";
    let new_name = "assignment-log-rename-new";
    let mut vault = vault_with("assignment-log-rename", &[(old_name, 17, false)]);
    let mut original = vault.get(old_name).unwrap().clone();
    original.settings.script_assignment = Some(assignment("stable"));
    vault.upsert(original).unwrap();

    let mut s = OperatorSession::<u32>::new(InstancePermit::SkipLock);
    s.set_spawn_workers(false);
    s.start(vault, empty_play());
    let mut renamed = s.vault().unwrap().get(old_name).unwrap().clone();
    renamed.username = new_name.into();
    s.rename_profile(old_name, renamed, ArmMirror::none(), "profile")
        .unwrap();
    s.flush_writes();

    let disk = Vault::unlock(&vault_path("assignment-log-rename"), "test-passphrase-01").unwrap();
    assert!(disk.get(old_name).is_none());
    let saved = disk.get(new_name).unwrap();
    assert_eq!(saved.uid, 17);
    assert_eq!(saved.settings.script_assignment, Some(assignment("stable")));
    assert!(slot_log(old_name).is_empty());
    assert!(slot_log(new_name).is_empty());
}

#[test]
fn coalesced_assignment_log_uses_last_assignment_change_not_parameter_edit() {
    let name = "assignment-log-owner";
    let mut s = session("assignment-log-owner", &[(name, 1, false)]);
    let gate = s.write_gate();
    let held = gate.lock().unwrap();

    let mut profile = s.vault().unwrap().get(name).unwrap().clone();
    profile.settings.script_assignment = Some(assignment("assigned"));
    let assign_op = s
        .save_profile_with_log_action(profile, ArmMirror::none(), "script", "Assign")
        .unwrap();
    let mut parameters = s.vault().unwrap().get(name).unwrap().clone();
    parameters
        .settings
        .script_settings
        .insert("compiled:assigned".into(), serde_json::Map::new());
    s.save_profile_with_log_action(parameters, ArmMirror::none(), "script", "parameter edit")
        .unwrap();

    drop(held);
    s.flush_writes();
    assert_eq!(
        slot_log(name),
        [(
            Source::Host,
            Level::Info,
            format!(
                "script assignment None -> compiled:assigned; action Assign; op#{}",
                assign_op.0
            ),
        )]
    );
    let disk = Vault::unlock(&vault_path("assignment-log-owner"), "test-passphrase-01").unwrap();
    assert!(disk
        .get(name)
        .unwrap()
        .settings
        .script_settings
        .contains_key("compiled:assigned"));
}

#[test]
fn dropping_session_logs_a_durable_assignment_without_polling() {
    let name = "assignment-log-drop";
    let op = {
        let mut s = session("assignment-log-drop", &[(name, 1, false)]);
        let mut profile = s.vault().unwrap().get(name).unwrap().clone();
        profile.settings.script_assignment = Some(assignment("final"));
        s.save_profile_with_log_action(profile, ArmMirror::none(), "script", "Assign")
            .unwrap()
    };
    assert_eq!(
        slot_log(name),
        [(
            Source::Host,
            Level::Info,
            format!(
                "script assignment None -> compiled:final; action Assign; op#{}",
                op.0
            ),
        )]
    );
}

#[test]
fn failed_assignment_save_does_not_log_a_mutation() {
    let name = "assignment-log-failed";
    let mut s = session("assignment-log-failed", &[(name, 1, false)]);
    let blocked = block_writes(&vault_path("assignment-log-failed"));
    let mut profile = s.vault().unwrap().get(name).unwrap().clone();
    profile.settings.script_assignment = Some(assignment("failed"));
    s.save_profile_with_log_action(profile, ArmMirror::none(), "script", "assignment")
        .unwrap();
    s.flush_writes();
    unblock_writes(blocked);
    assert!(slot_log(name).is_empty());
}

#[test]
fn lines_recorded_by_the_poll_carry_the_slots_current_tick() {
    let mut s = session("log-tick", &[("logtick-gus", 5, false)]);
    let mut surface = Recorder::default();
    s.load("logtick-gus", &mut surface);
    // The slot worker binds its thread and publishes the tick it observed.
    std::thread::spawn(|| {
        api::hostlog::bind_slot("logtick-gus");
        api::hostlog::set_tick(4242);
    })
    .join()
    .unwrap();
    push_status(&s, row("logtick-gus"));
    s.poll();
    let mut view = crate::log::LogView::new(crate::log::LogScope::slot("logtick-gus"));
    crate::log::global().refresh(&mut view);
    let up = view
        .rows()
        .iter()
        .find(|e| e.message() == "slot up")
        .unwrap();
    assert_eq!(up.tick(), Some(4242));
}

#[test]
fn a_poll_moves_transitions_onto_each_slots_log() {
    let mut s = session(
        "log-transitions",
        &[("logpoll-alice", 1, false), ("logpoll-bob", 2, false)],
    );
    let mut surface = Recorder::default();
    s.load("logpoll-alice", &mut surface);
    s.load("logpoll-bob", &mut surface);
    push_status(&s, row("logpoll-alice"));
    push_status(&s, row("logpoll-bob"));
    s.poll();
    {
        let play = s.play().unwrap();
        let mut rows = play.statuses.lock().unwrap();
        rows[0].ingame = true;
        rows[0].scene_state = 2;
        rows[1].error = Some("code 3: invalid username or password".into());
    }
    s.poll();
    // The Load each member got is logged (with its operation id) before
    // the status changes it caused.
    assert_eq!(
        slot_log("logpoll-alice"),
        [
            (Source::Host, Level::Info, "op#1 Load completed".to_string()),
            (Source::Host, Level::Info, "slot up".to_string()),
            (Source::Login, Level::Info, "ingame".to_string()),
            (Source::Host, Level::Info, "scene 2".to_string()),
        ]
    );
    assert_eq!(
        slot_log("logpoll-bob"),
        [
            (Source::Host, Level::Info, "op#2 Load completed".to_string()),
            (Source::Host, Level::Info, "slot up".to_string()),
            (
                Source::Login,
                Level::Error,
                "login code 3: invalid username or password".to_string()
            ),
        ]
    );
}

#[cfg(feature = "js")]
#[test]
fn script_lines_reach_the_slot_log_through_the_poll() {
    let iso = script::IsolatedEnv::enter("frontend-core-log-script");
    let mut s = session("log-script", &[("logpoll-carol", 3, false)]);
    let mut surface = Recorder::default();
    s.load("logpoll-carol", &mut surface);
    push_status(&s, row("logpoll-carol"));
    let path = iso.dir.join("broken.ts");
    std::fs::write(
        &path,
        "export default class T extends LoopingBot { \
         constructor() { super(); throw new Error('boom in setup'); } \
         override loop() {} }",
    )
    .unwrap();
    let mut library = script_js::JsLibrary::new(iso.dir.join("js-scripts.json"));
    let card = library.load(&path).unwrap();
    s.start_script(
        "logpoll-carol",
        ScriptStart::load(
            card.js,
            LoadShape::from_script(card.shape),
            None,
            Vec::new(),
        ),
        Some("file:broken".into()),
    )
    .unwrap();
    let mut taken = Vec::new();
    pump_until(&mut s, "the failing script's log lines", |s| {
        taken.extend(s.script_lines().iter().cloned());
        !taken.is_empty()
    });
    s.poll();
    assert!(s.script_lines().is_empty(), "lines are handed out once");
    assert!(
        taken
            .iter()
            .any(|(slot, line)| slot == "logpoll-carol" && line.contains("boom in setup")),
        "{taken:?}"
    );
    assert!(
        slot_log("logpoll-carol")
            .iter()
            .any(|(source, level, line)| *source == Source::Script
                && *level == Level::Error
                && line.contains("boom in setup")),
        "{:?}",
        slot_log("logpoll-carol")
    );
}

#[test]
fn a_row_published_before_log_in_does_not_cancel_it() {
    let mut s = session("stale-latch-row", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    s.logout("alice");
    // The worker last published while latched; it has not observed Log in yet.
    push_status(
        &s,
        SlotStatus {
            login_latched: true,
            ..row("alice")
        },
    );
    let login = s.login("alice", &mut surface);
    s.poll();
    assert_eq!(
        s.operation(login).unwrap().outcome("alice"),
        Some(&Outcome::Pending)
    );
    s.logout("alice");
    s.poll();
    assert_eq!(
        s.operation(login).unwrap().outcome("alice"),
        Some(&Outcome::Cancelled),
        "a later Log out does cancel it"
    );
}

#[test]
fn a_profile_write_is_queued_and_the_arm_follows_only_once_it_is_durable() {
    let mut s = session("write-queued", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let alice = arm(&s, "alice");

    let op = submit_auto_login(&mut s, "alice", true);

    assert!(
        s.vault().unwrap().get("alice").unwrap().settings.auto_login,
        "the edit is staged in memory at once"
    );
    assert!(
        !alice.auto_login(),
        "the running slot changes only after the write is durable"
    );
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Pending)
    );
    s.flush_writes();
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert!(alice.auto_login());
    let disk = Vault::unlock(&vault_path("write-queued"), "test-passphrase-01").unwrap();
    assert!(disk.get("alice").unwrap().settings.auto_login);
}

#[test]
fn consecutive_writes_land_in_order_with_the_last_one_on_disk() {
    let mut s = session("write-order", &[("alice", 1, false)]);
    s.set_random_settings("alice", false, "Magic", false)
        .unwrap();
    submit_auto_login(&mut s, "alice", true);
    s.set_random_settings("alice", true, "Prayer", true)
        .unwrap();
    s.flush_writes();
    let disk = Vault::unlock(&vault_path("write-order"), "test-passphrase-01").unwrap();
    let saved = &disk.get("alice").unwrap().settings;
    assert!(saved.auto_login, "the earlier field edit is not lost");
    assert!(saved.random_events);
    assert_eq!(saved.lamp_skill, "Prayer");
}

#[test]
fn random_event_policy_persists_and_mirrors_after_durable_write() {
    let mut s = session("random-event-policy", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let alice = arm(&s, "alice");
    let op = s
        .set_random_event_policy("alice", "River Troll", crate::GuardianPolicy::Fight)
        .unwrap();
    assert!(
        alice.random_event_policies.lock().unwrap().is_empty(),
        "a running slot changes only after the profile write is durable"
    );

    s.flush_writes();
    let saved = Vault::unlock(&vault_path("random-event-policy"), "test-passphrase-01").unwrap();
    assert_eq!(
        saved
            .get("alice")
            .unwrap()
            .settings
            .random_event_policy("river troll"),
        vault::RandomEventPolicy::Fight
    );
    assert_eq!(
        alice
            .random_event_policies
            .lock()
            .unwrap()
            .get("river troll"),
        Some(&vault::RandomEventPolicy::Fight)
    );
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
}

#[test]
fn survival_mirror_reaches_running_slot_within_one_frame() {
    let name = "survival-mirror-alice";
    let mut s = session("survival-mirror", &[(name, 1, false)]);
    let mut surface = Recorder::default();
    s.load(name, &mut surface);
    let alice = arm(&s, name);
    assert_eq!(
        alice.cure_poison(),
        script::survival::CurePoison::WhenNeeded,
        "a fresh arm mirrors the default"
    );
    let op = s
        .set_survival_settings(name, crate::CurePoison::Never)
        .unwrap();
    assert_eq!(
        alice.cure_poison(),
        script::survival::CurePoison::WhenNeeded,
        "a running slot changes only after the profile write is durable"
    );

    s.flush_writes();
    assert_eq!(
        s.operation(op).unwrap().outcome(name),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        alice.cure_poison(),
        script::survival::CurePoison::Never,
        "the durable write is on the arm for the next frame the slot observes"
    );
    let saved = Vault::unlock(&vault_path("survival-mirror"), "test-passphrase-01").unwrap();
    assert_eq!(
        saved.get(name).unwrap().settings.survival.cure_poison,
        vault::CurePoison::Never
    );
}

#[test]
fn set_survival_logs_one_audit_line() {
    let name = "survival-audit-alice";
    let mut s = session("survival-audit", &[(name, 1, false)]);
    let first = s
        .set_survival_settings(name, crate::CurePoison::Never)
        .unwrap();
    s.flush_writes();
    assert_eq!(
        slot_log(name),
        [(
            Source::Host,
            Level::Info,
            format!(
                "survival cure_poison when_needed -> never; profile {name}; action survival; op#{}",
                first.0
            ),
        )],
        "one durable survival write logs one audit line"
    );

    let second = s
        .set_survival_settings(name, crate::CurePoison::Always)
        .unwrap();
    s.flush_writes();
    assert_eq!(
        slot_log(name).len(),
        2,
        "each write logs its own line, no more"
    );
    assert_eq!(
        slot_log(name).last().unwrap(),
        &(
            Source::Host,
            Level::Info,
            format!(
                "survival cure_poison never -> always; profile {name}; action survival; op#{}",
                second.0
            ),
        ),
        "the old value is the row the write replaced"
    );
}

#[test]
fn combined_survival_writes_log_the_durable_transition_once() {
    let name = "survival-coalesced-alice";
    let mut s = session("survival-coalesced", &[(name, 1, false)]);
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let superseded = s
        .set_survival_settings(name, crate::CurePoison::Never)
        .unwrap();
    let final_write = s
        .set_survival_settings(name, crate::CurePoison::Always)
        .unwrap();
    drop(held);
    s.flush_writes();

    assert_eq!(
        s.operation(superseded).unwrap().outcome(name),
        Some(&Outcome::Cancelled)
    );
    assert_eq!(
        s.operation(final_write).unwrap().outcome(name),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        slot_log(name),
        [(
            Source::Host,
            Level::Info,
            format!(
                "survival cure_poison when_needed -> always; profile {name}; action survival; op#{}",
                final_write.0
            ),
        )],
        "one committed change logs its durable transition"
    );
}

#[test]
fn failed_pending_survival_write_then_success_logs_only_the_durable_transition() {
    let name = "survival-failed-then-success-alice";
    let test = "survival-failed-then-success";
    let mut s = session(test, &[(name, 1, false)]);
    let blocked = block_writes(&vault_path(test));

    let failed = s
        .set_survival_settings(name, crate::CurePoison::Never)
        .unwrap();
    let failed_write = s
        .writer
        .as_mut()
        .and_then(ProfileWriter::wait_take)
        .expect("the failed write reaches the writer");
    assert!(failed_write.result.is_err());

    unblock_writes(blocked);
    let successful = s
        .set_survival_settings(name, crate::CurePoison::Always)
        .unwrap();
    s.settle_write(failed_write);
    let successful_write = s
        .writer
        .as_mut()
        .and_then(ProfileWriter::wait_take)
        .expect("the following write reaches the writer");
    s.settle_write(successful_write);

    assert!(s
        .operation(failed)
        .unwrap()
        .outcome(name)
        .unwrap()
        .is_failed());
    assert_eq!(
        s.operation(successful).unwrap().outcome(name),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        slot_log(name),
        [(
            Source::Host,
            Level::Info,
            format!(
                "survival cure_poison when_needed -> always; profile {name}; action survival; op#{}",
                successful.0
            ),
        )],
        "the failed setting never appears in the audit history"
    );
}

#[test]
fn random_settings_write_path_unchanged() {
    // Regression pin: the guardian write path still persists through
    // save_profile and mirrors the running arm. This test names no
    // survival type, so it also passes on the base commit.
    let name = "survival-pin-alice";
    let mut s = session("survival-pin", &[(name, 1, false)]);
    let mut surface = Recorder::default();
    s.load(name, &mut surface);
    let alice = arm(&s, name);
    s.set_random_settings(name, false, "Magic", false).unwrap();
    s.flush_writes();
    assert!(!alice.random_events.load(Ordering::Relaxed));
    assert_eq!(alice.lamp_skill.lock().unwrap().as_str(), "Magic");
    let saved = Vault::unlock(&vault_path("survival-pin"), "test-passphrase-01").unwrap();
    let settings = &saved.get(name).unwrap().settings;
    assert!(!settings.random_events);
    assert_eq!(settings.lamp_skill, "Magic");
}

#[test]
fn a_failed_write_is_reported_restores_the_durable_value_and_leaves_the_arm() {
    let mut s = session("write-fail", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let alice = arm(&s, "alice");
    let before = alice.random_events.load(Ordering::Relaxed);
    let blocked = block_writes(&vault_path("write-fail"));

    let op = s
        .set_random_settings("alice", !before, "Magic", false)
        .unwrap();
    s.flush_writes();
    unblock_writes(blocked);

    assert!(matches!(
        s.operation(op).unwrap().outcome("alice"),
        Some(o) if o.is_failed()
    ));
    let failures = s.take_write_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        (failures[0].op(), failures[0].target(), failures[0].label()),
        (op, "alice", "random"),
        "{failures:?}"
    );
    assert!(
        failures[0].to_string().starts_with("random: "),
        "the banner text is `label: error`: {failures:?}"
    );
    assert_eq!(
        s.vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .random_events,
        before,
        "the staged edit is rolled back to the durable value"
    );
    assert_eq!(alice.random_events.load(Ordering::Relaxed), before);
}

#[cfg(feature = "js")]
#[test]
fn stop_scripts_drops_a_start_queued_behind_a_reap() {
    let iso = script::IsolatedEnv::enter("frontend-core-stop-queued");
    let mut s = session("stop-queued", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let (js, shape) = looping_bot(&iso.dir);
    let load = |js: &str| ScriptStart::load(js.to_string(), shape, None, Vec::new());
    s.start_script("alice", load(&js), None).unwrap();
    settle_start(&mut s);
    // Reload shape: Stop, then a replacement Start queued behind the reap.
    s.stop_script("alice");
    let replacement = s.start_script("alice", load(&js), None).unwrap();
    assert_eq!(
        s.play().unwrap().script_state("alice"),
        script::RunState::Stopping
    );

    let (op, stopped) = s.stop_scripts(&["alice".to_string()]);

    assert_eq!(stopped, 1, "a Stopping slot with a queued Start is stopped");
    wait_operation_settled(&mut s, op);
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    for _ in 0..20 {
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        s.play().unwrap().script_state("alice"),
        script::RunState::Idle,
        "the queued replacement never runs"
    );
    assert_eq!(
        s.operation(replacement).unwrap().outcome("alice"),
        Some(&Outcome::Cancelled)
    );
}

#[test]
fn a_write_replaced_in_the_same_commit_is_cancelled_and_its_mirror_never_runs() {
    let mut s = session("write-coalesce", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let alice = arm(&s, "alice");
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let on = submit_auto_login(&mut s, "alice", true);
    let off = submit_auto_login(&mut s, "alice", false);
    drop(held);
    s.flush_writes();

    assert_eq!(
        s.operation(on).unwrap().outcome("alice"),
        Some(&Outcome::Cancelled)
    );
    assert_eq!(
        s.operation(off).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert!(
        !alice.auto_login(),
        "the slot never observes the superseded value"
    );
    let disk = Vault::unlock(&vault_path("write-coalesce"), "test-passphrase-01").unwrap();
    assert!(!disk.get("alice").unwrap().settings.auto_login);
}

#[test]
fn coalesced_edits_of_different_settings_all_reach_the_running_slot() {
    let mut s = session("write-coalesce-fields", &[("alice", 1, true)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let alice = arm(&s, "alice");
    assert!(alice.auto_login());
    assert!(alice.random_events.load(Ordering::Relaxed));
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let auto = submit_auto_login(&mut s, "alice", false);
    let magic = s
        .set_random_settings("alice", false, "Magic", false)
        .unwrap();
    let prayer = s
        .set_random_settings("alice", false, "Prayer", false)
        .unwrap();
    drop(held);
    s.flush_writes();

    let disk = Vault::unlock(&vault_path("write-coalesce-fields"), "test-passphrase-01").unwrap();
    let saved = &disk.get("alice").unwrap().settings;
    assert!(!saved.auto_login);
    assert!(!saved.random_events && !saved.lamp_auto);
    assert_eq!(saved.lamp_skill, "Prayer");
    assert!(
        !alice.auto_login(),
        "the auto-login edit saved in the same commit reaches the slot"
    );
    assert!(!alice.random_events.load(Ordering::Relaxed));
    assert!(!alice.lamp_auto.load(Ordering::Relaxed));
    assert_eq!(*alice.lamp_skill.lock().unwrap(), "Prayer");
    let outcome = |op| {
        s.operation(op)
            .unwrap()
            .outcome("alice")
            .map(|o| o.duplicate())
    };
    assert_eq!(outcome(auto), Some(Outcome::Completed));
    assert_eq!(
        outcome(magic),
        Some(Outcome::Cancelled),
        "the same setting's newer edit in the commit replaced it"
    );
    assert_eq!(outcome(prayer), Some(Outcome::Completed));
}

#[test]
fn a_rename_is_one_transaction_and_a_failed_one_restores_both_names() {
    let mut s = session("rename-fail", &[("alice", 1, false)]);
    let mut renamed = s.vault().unwrap().get("alice").unwrap().clone();
    renamed.username = "alicia".into();
    let blocked = block_writes(&vault_path("rename-fail"));

    let op = s
        .rename_profile("alice", renamed.clone(), ArmMirror::none(), "credentials")
        .unwrap();
    assert!(s.vault().unwrap().get("alice").is_none(), "staged at once");
    s.flush_writes();
    unblock_writes(blocked);

    assert!(matches!(
        s.operation(op).unwrap().outcome("alicia"),
        Some(o) if o.is_failed()
    ));
    assert!(
        s.vault().unwrap().get("alice").is_some(),
        "old name restored"
    );
    assert!(
        s.vault().unwrap().get("alicia").is_none(),
        "new name rolled back"
    );
    let disk = Vault::unlock(&vault_path("rename-fail"), "test-passphrase-01").unwrap();
    assert!(disk.get("alice").is_some() && disk.get("alicia").is_none());

    let op = s
        .rename_profile("alice", renamed, ArmMirror::none(), "credentials")
        .unwrap();
    s.flush_writes();
    assert_eq!(
        s.operation(op).unwrap().outcome("alicia"),
        Some(&Outcome::Completed)
    );
    let disk = Vault::unlock(&vault_path("rename-fail"), "test-passphrase-01").unwrap();
    assert!(disk.get("alice").is_none() && disk.get("alicia").is_some());
}

/// Operator report: an edit form whose username field held another
/// profile's name saved as a rename onto it, overwriting that profile and
/// deleting the edited one.
#[test]
fn a_rename_onto_an_existing_username_is_refused_and_writes_nothing() {
    let mut s = session(
        "rename-collide",
        &[("aindniK", 1, true), ("Hans", 2, false)],
    );
    let mut onto = s.vault().unwrap().get("aindniK").unwrap().clone();
    onto.username = "Hans".into();
    onto.password = "typed".into();

    let error = s
        .rename_profile("aindniK", onto, ArmMirror::none(), "credentials")
        .unwrap_err();
    assert_eq!(error, "credentials: a profile named Hans already exists");
    s.flush_writes();

    let disk = Vault::unlock(&vault_path("rename-collide"), "test-passphrase-01").unwrap();
    for vault in [s.vault().unwrap(), &disk] {
        let hans = vault.get("Hans").unwrap();
        assert_eq!(
            (hans.uid, hans.password.as_str()),
            (2, "pw"),
            "Hans untouched"
        );
        let kept = vault
            .get("aindniK")
            .expect("the edited profile is not deleted");
        assert_eq!((kept.uid, kept.password.as_str()), (1, "pw"));
    }
}

#[test]
fn a_new_profile_with_an_existing_username_is_refused_and_writes_nothing() {
    let mut s = session("create-collide", &[("Hans", 2, false)]);
    let fresh = Profile {
        username: "Hans".into(),
        password: "typed".into(),
        uid: 9,
        settings: ProfileSettings::default(),
    };

    let error = s
        .create_profile(fresh.clone(), ArmMirror::none(), "credentials")
        .unwrap_err();
    assert_eq!(error, "credentials: a profile named Hans already exists");
    s.flush_writes();
    let disk = Vault::unlock(&vault_path("create-collide"), "test-passphrase-01").unwrap();
    for vault in [s.vault().unwrap(), &disk] {
        let hans = vault.get("Hans").unwrap();
        assert_eq!((hans.uid, hans.password.as_str()), (2, "pw"));
    }

    // A new username is created.
    let bob = Profile {
        username: "bob".into(),
        ..fresh
    };
    s.create_profile(bob, ArmMirror::none(), "credentials")
        .unwrap();
    s.flush_writes();
    let disk = Vault::unlock(&vault_path("create-collide"), "test-passphrase-01").unwrap();
    assert_eq!(disk.get("bob").unwrap().password, "typed");
}

/// The shared unsaved-changes predicate behind both profile editors: an
/// untouched form is clean, any field its Save would write makes it dirty,
/// and a target whose row is gone counts as dirty (leaving must ask, never
/// silently drop the draft).
#[test]
fn profile_form_dirty_tracks_edits_against_the_vault_row() {
    let s = session("form-dirty", &[("alice", 1, false)]);
    let row = s.vault().unwrap().get("alice").unwrap();
    let (user, pass, settings) = (
        row.username.clone(),
        row.password.clone(),
        row.settings.clone(),
    );

    assert!(
        !dirty(&s, Some("alice"), &user, &pass, &settings),
        "an editor opened on the row is clean"
    );
    assert!(
        dirty(&s, Some("alice"), &user, "typed-but-unsaved", &settings),
        "an edited password is unsaved"
    );
    assert!(
        dirty(&s, Some("alice"), "bob", &pass, &settings),
        "a renamed username is unsaved"
    );
    let mut tweaked = settings.clone();
    tweaked.world = Some(2);
    assert!(
        dirty(&s, Some("alice"), &user, &pass, &tweaked),
        "an edited setting is unsaved"
    );
    assert!(
        dirty(&s, Some("ghost"), &user, &pass, &settings),
        "a target with no row must ask before it is left"
    );
}

#[test]
fn profile_form_dirty_treats_a_blank_new_profile_as_clean() {
    let s = session("form-dirty-new", &[("alice", 1, false)]);
    let clean = ProfileSettings::default();

    assert!(
        !dirty(&s, None, "", "", &clean),
        "a blank new-profile form is clean"
    );
    assert!(
        !dirty(&s, Some(""), "  ", "", &clean),
        "whitespace alone is not a name"
    );
    assert!(
        dirty(&s, None, "bob", "", &clean),
        "a typed name is unsaved"
    );
    assert!(
        dirty(&s, None, "", "pw", &clean),
        "a typed password is unsaved"
    );
    let mut tweaked = clean.clone();
    tweaked.lamp_auto = !clean.lamp_auto;
    assert!(
        dirty(&s, None, "", "", &tweaked),
        "a non-default setting is unsaved"
    );
}

/// The Save projection for an existing target: only the trimmed username,
/// the password as typed, `world` and the trimmed `clue_duel_partner` count.
/// Whitespace-equivalent drafts are clean, and a concurrent change to a
/// field Save preserves (script assignment, raster, tutorial, …) never
/// makes an untouched editor look dirty.
#[test]
fn profile_form_dirty_projects_only_what_save_writes() {
    let mut s = session("form-dirty-projection", &[("alice", 1, false)]);
    let row = s.vault().unwrap().get("alice").unwrap().clone();

    // Whitespace around the username and the clue partner canonicalizes to
    // the stored row: Save would write the same values.
    assert!(
        !dirty(
            &s,
            Some("alice"),
            "  alice  ",
            &row.password,
            &ProfileSettings {
                clue_duel_partner: format!("  {}  ", row.settings.clue_duel_partner),
                ..row.settings.clone()
            },
        ),
        "whitespace-equivalent input is not dirty"
    );

    // A concurrent change to fields Save preserves leaves the open draft
    // clean: the editor still matches what its Save would write.
    let mut concurrent = row.settings.clone();
    concurrent.script_assignment = Some(vault::ScriptAssignment {
        source_kind: "catalog".into(),
        identity: "other".into(),
        display_name: String::new(),
        unavailable: None,
    });
    concurrent.raster = match row.settings.raster {
        vault::RasterMode::Gpu => vault::RasterMode::Cpu,
        _ => vault::RasterMode::Gpu,
    };
    concurrent.tutorial_skipped = Some(!row.settings.tutorial_skipped.unwrap_or(false));
    concurrent.lowmem = !row.settings.lowmem;
    concurrent.auto_login = !row.settings.auto_login;
    concurrent.random_events = !row.settings.random_events;
    {
        let mut updated = row.clone();
        updated.settings = concurrent;
        s.vault_mut().unwrap().upsert(updated).unwrap();
    }
    assert!(
        !dirty(
            &s,
            Some("alice"),
            &row.username,
            &row.password,
            &row.settings
        ),
        "a concurrent change to a non-owned setting is not dirty"
    );

    // Real edits still count.
    assert!(
        dirty(&s, Some("alice"), &row.username, "changed", &row.settings),
        "a real password edit is dirty"
    );
    let mut world = row.settings.clone();
    world.world = Some(row.settings.world.unwrap_or(1).wrapping_add(1));
    assert!(
        dirty(&s, Some("alice"), &row.username, &row.password, &world),
        "a real world edit is dirty"
    );
    let mut clue = row.settings.clone();
    clue.clue_duel_partner = format!("{}!", row.settings.clue_duel_partner);
    assert!(
        dirty(&s, Some("alice"), &row.username, &row.password, &clue),
        "a real clue-partner edit is dirty"
    );
}
#[test]
fn on_a_failed_commit_a_superseded_write_is_cancelled_and_only_the_final_one_fails() {
    let mut s = session("write-coalesce-fail", &[("alice", 1, false)]);
    let blocked = block_writes(&vault_path("write-coalesce-fail"));
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let on = submit_auto_login(&mut s, "alice", true);
    let off = s
        .set_random_settings("alice", false, "Magic", false)
        .unwrap();
    drop(held);
    s.flush_writes();
    unblock_writes(blocked);

    assert_eq!(
        s.operation(on).unwrap().outcome("alice"),
        Some(&Outcome::Cancelled)
    );
    assert!(matches!(
        s.operation(off).unwrap().outcome("alice"),
        Some(o) if o.is_failed()
    ));
    assert_eq!(s.take_write_failures().len(), 1, "one failure reported");
    let saved = s.vault().unwrap().get("alice").unwrap();
    assert!(!saved.settings.auto_login, "both staged edits roll back");
    assert!(saved.settings.random_events);
}

#[test]
fn a_parked_profile_spawns_from_its_durable_row_while_an_edit_is_saving() {
    let mut s = session("durable-spawn", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    let old_uid = 1;
    let mut edited = s.vault().unwrap().get("alice").unwrap().clone();
    edited.password = "new-pass".into();
    edited.uid = 99;
    edited.settings.random_events = !edited.settings.random_events;
    let gate = s.write_gate();

    // Slow write: Log in before it settles uses the durable row.
    let held = gate.lock().unwrap();
    s.save_profile(edited.clone(), ArmMirror::remember(None), "credentials")
        .unwrap();
    assert!(
        !s.profile_saving("alice"),
        "an existing profile is not gated"
    );
    s.login("alice", &mut surface);
    let alice = arm(&s, "alice");
    let durable_random = !edited_random(&s);
    assert_eq!(
        alice.random_events.load(Ordering::Relaxed),
        durable_random,
        "the arm carries the durable guardian setting"
    );
    assert_eq!(
        alice.uid.load(Ordering::Relaxed),
        old_uid,
        "the worker spawned from the durable row"
    );
    assert_eq!(s.durable_profile("alice").unwrap().password, "pw");
    assert_eq!(
        s.vault().unwrap().get("alice").unwrap().password,
        "new-pass"
    );

    // The write fails: nothing live changes and the staged edit rolls back.
    let blocked = block_writes(&vault_path("durable-spawn"));
    drop(held);
    s.flush_writes();
    unblock_writes(blocked);
    assert_eq!(s.vault().unwrap().get("alice").unwrap().password, "pw");
    assert_eq!(alice.uid.load(Ordering::Relaxed), old_uid);
    assert!(Arc::ptr_eq(&arm(&s, "alice"), &alice));

    // The write succeeds: the post-write mirror hands Play the new row.
    s.save_profile(edited, ArmMirror::remember(None), "credentials")
        .unwrap();
    s.flush_writes();
    assert_eq!(s.durable_profile("alice").unwrap().password, "new-pass");
    assert_eq!(
        alice.uid.load(Ordering::Relaxed),
        99,
        "remember_profile synced the running arm"
    );
}

/// The staged (unsaved) random-events value of `alice`.
fn edited_random(s: &OperatorSession<u32>) -> bool {
    s.vault()
        .unwrap()
        .get("alice")
        .unwrap()
        .settings
        .random_events
}

/// A vault written by the 0.1.8.1 release code (`a88d07764` vault crate,
/// passphrase `bot`): alice with every profile field set, bob at defaults.
const VAULT_0_1_8_1: &[u8] = include_bytes!("../tests/fixtures/vault-0.1.8.1.bin");

#[test]
fn a_0_1_8_1_vault_loads_spawns_and_saves_through_the_core_unchanged() {
    let path = vault_path("vault-0181");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, VAULT_0_1_8_1).unwrap();
    let mut s = OperatorSession::new(InstancePermit::SkipLock);
    s.set_spawn_workers(false);
    s.start(Vault::unlock(&path, "bot").unwrap(), empty_play());

    let alice = s.vault().unwrap().get("alice").unwrap().clone();
    assert_eq!(alice.password, "old-pass");
    assert_eq!(alice.uid, 274_000_501);
    assert!(alice.settings.auto_login && !alice.settings.lowmem);
    assert_eq!(alice.settings.world, Some(2));
    assert_eq!(alice.settings.tutorial_skipped, Some(true));
    assert_eq!(alice.settings.raster, vault::RasterMode::Cpu);
    assert!(!alice.settings.random_events && !alice.settings.lamp_auto);
    assert_eq!(alice.settings.lamp_skill, "prayer");
    assert_eq!(
        alice.settings.script_assignment.as_ref().unwrap().identity,
        "catalog:Thiever"
    );
    assert_eq!(
        alice.settings.script_settings["catalog:Thiever"]["food"],
        serde_json::json!("Lobster")
    );

    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    let arm = arm(&s, "alice");
    assert_eq!(arm.uid.load(Ordering::Relaxed), 274_000_501);
    assert!(arm.wants_login(), "saved auto-login is honoured");

    s.set_random_settings("alice", true, "attack", true)
        .unwrap();
    s.flush_writes();
    let reread = Vault::unlock(&path, "bot").unwrap();
    let saved = reread.get("alice").unwrap();
    assert!(saved.settings.random_events);
    assert_eq!(saved.settings.lamp_skill, "attack");
    let mut expected = alice.settings.clone();
    expected.random_events = true;
    expected.lamp_skill = "attack".into();
    expected.lamp_auto = true;
    assert_eq!(
        saved.settings, expected,
        "every other field survives the rewrite"
    );
    assert_eq!(saved.password, "old-pass");
    assert_eq!(reread.get("bob"), s.vault().unwrap().get("bob"));
}

// A profile form's saves settle by their write's operation id: a delayed
// writer lets the form move on (or the write fail) before the result lands.

use crate::{FormNotice, FormSettled, ProfileFormSave};

/// What a form saw settle: `(saved | failed, destination, same form)`.
fn outcomes(settled: &[FormSettled]) -> Vec<(&'static str, String, bool)> {
    settled
        .iter()
        .map(|s| {
            if let Some(saved) = s.as_saved() {
                (
                    "saved",
                    saved.record().destination().to_string(),
                    saved.same_form(),
                )
            } else if let Some(failed) = s.as_failed() {
                (
                    "failed",
                    failed.record().destination().to_string(),
                    failed.same_form(),
                )
            } else {
                panic!("unexpected settled save: {s:?}")
            }
        })
        .collect()
}

/// `alice` with `password`, as a form would submit it.
fn alice_with(s: &OperatorSession<u32>, password: &str) -> Profile {
    let mut profile = s.vault().unwrap().get("alice").unwrap().clone();
    profile.password = password.into();
    profile
}

/// Queue a credentials save of alice from `form`.
fn form_saves_alice(
    s: &mut OperatorSession<u32>,
    form: &mut ProfileFormSave,
    password: &str,
) -> OperationId {
    let draft = alice_with(s, password);
    let op = s
        .save_profile(draft, ArmMirror::none(), "credentials")
        .unwrap();
    form.submitted(s, op, Some("alice"), "alice");
    op
}

#[test]
fn a_write_failing_after_it_was_queued_shows_in_the_form_it_came_from() {
    let mut s = session("form-fail", &[("alice", 1, false)]);
    let mut form = ProfileFormSave::default();
    form.form_changed();
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let op = form_saves_alice(&mut s, &mut form, "typed");
    assert!(form.saving(&s), "the form's save has not settled");
    assert!(form.settle(&mut s).is_empty(), "still queued");
    assert!(form.notice().is_none());

    let blocked = block_writes(&vault_path("form-fail"));
    drop(held);
    s.flush_writes();
    unblock_writes(blocked);

    let failures = s.take_write_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        (failures[0].op(), failures[0].target()),
        (op, "alice"),
        "the failure names its operation and profile"
    );
    assert_eq!(
        outcomes(&form.settle(&mut s)),
        [("failed", "alice".into(), true)]
    );
    assert_eq!(
        form.notice(),
        Some(&FormNotice::failed(failures[0].to_string())),
        "the form shows the banner's text"
    );
    assert!(!form.saving(&s), "a failed save can be retried");
    let staged = s.vault().unwrap().get("alice").unwrap();
    assert_eq!(staged.password, "pw", "the durable value is back");

    form.edited();
    assert!(form.notice().is_none(), "the next edit clears the failure");
}

#[test]
fn a_write_failing_after_the_form_moved_on_never_shows_in_another_form() {
    let mut s = session("form-moved", &[("alice", 1, false)]);
    let mut form = ProfileFormSave::default();
    form.form_changed();
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    form_saves_alice(&mut s, &mut form, "typed");
    // Switched to another target, then closed and reopened.
    form.form_changed();
    form.form_changed();
    assert!(!form.saving(&s), "the new form has no save of its own");

    let blocked = block_writes(&vault_path("form-moved"));
    drop(held);
    s.flush_writes();
    unblock_writes(blocked);

    assert_eq!(
        outcomes(&form.settle(&mut s)),
        [("failed", "alice".into(), false)]
    );
    assert!(form.notice().is_none(), "nothing in the form showing now");
    assert_eq!(
        s.take_write_failures().len(),
        1,
        "the banner still reports it"
    );
}

#[test]
fn a_failed_save_hands_back_the_draft_that_did_not_land() {
    let mut s = session("form-record", &[("alice", 1, false)]);
    let mut form = ProfileFormSave::default();
    form.form_changed();
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let op = form_saves_alice(&mut s, &mut form, "typed");
    // An unrelated edit staged after the save changes the vault row, not
    // what the save carried.
    submit_auto_login(&mut s, "alice", true);
    assert_eq!(
        s.saves_in_flight()
            .iter()
            .map(|r| r.op())
            .collect::<Vec<_>>(),
        [op],
        "the session's record of the save is all that says it is in flight"
    );
    let blocked = block_writes(&vault_path("form-record"));
    drop(held);
    s.flush_writes();
    unblock_writes(blocked);

    assert!(s.saves_in_flight().is_empty());
    let settled = form.settle(&mut s);
    let [failed] = settled.as_slice() else {
        panic!("one failed save, got {settled:?}");
    };
    let Some(failed) = failed.as_failed() else {
        panic!("one failed save, got {settled:?}");
    };
    assert_eq!(failed.record().op(), op);
    assert_eq!(failed.record().source(), Some("alice"));
    assert_eq!(failed.record().destination(), "alice");
    let mut expected = alice_with(&s, "typed");
    expected.settings.auto_login = false;
    assert_eq!(
        failed.record().draft(),
        &expected,
        "the typed draft as submitted, not the restored row or a later edit"
    );
    assert_eq!(s.vault().unwrap().get("alice").unwrap().password, "pw");
    assert!(
        form.settle(&mut s).is_empty(),
        "a settled save is seen once"
    );
}

fn publish_alice_login(s: &mut OperatorSession<u32>, lowmem: bool) {
    {
        let mut rows = s.play().unwrap().statuses.lock().unwrap();
        if let Some(row) = rows.iter_mut().find(|row| row.username == "alice") {
            row.connected = true;
            row.ingame = true;
            row.scene_state = 2;
            row.login_latched = false;
            row.login_lowmem = Some(lowmem);
        } else {
            rows.push(SlotStatus {
                username: "alice".into(),
                connected: true,
                ingame: true,
                scene_state: 2,
                login_lowmem: Some(lowmem),
                ..SlotStatus::default()
            });
        }
    }
    s.poll();
}

#[test]
fn memory_toggle_stages_arms_and_settles() {
    let mut s = session("mem-toggle", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    assert_eq!(
        s.memory_status("alice"),
        None,
        "a spawn is not a successful login handshake"
    );
    assert_eq!(
        arm(&s, "alice").lowmem_handshake(),
        Some(true),
        "the effective spawn mode seeds the arm"
    );
    publish_alice_login(&mut s, true);
    assert!(!s.memory_status("alice").unwrap().differs());

    let op = submit_memory_mode(&mut s, "alice", false);
    assert_eq!(
        arm(&s, "alice").lowmem_handshake(),
        Some(false),
        "the toggle queues the next handshake without changing this login"
    );
    let notice = s.memory_status("alice").expect("recorded login");
    assert!(
        notice.differs(),
        "the entire client still follows the login mode"
    );
    assert_eq!((notice.login_lowmem, notice.desired_lowmem), (true, false));

    s.flush_writes();
    assert_eq!(
        s.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert!(
        !s.vault().unwrap().get("alice").unwrap().settings.lowmem,
        "the setting is durable"
    );
    assert_eq!(
        arm(&s, "alice").lowmem_handshake(),
        Some(false),
        "settle re-affirms the armed handshake"
    );
}

#[test]
fn older_memory_save_cannot_rearm_a_cancelled_queue_across_commits() {
    let mut s = session("mem-toggle-back", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    publish_alice_login(&mut s, true);

    let first = submit_memory_mode(&mut s, "alice", false);
    // Seal and finish the first commit, but delay delivering its completion
    // until after the operator has cancelled that queued mode.
    let written = s.writer.as_mut().unwrap().wait_take().unwrap();
    assert_eq!(written.op, first);
    assert!(!written.superseded);
    assert!(written.result.is_ok());
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    let second = submit_memory_mode(&mut s, "alice", true);
    assert_eq!(arm(&s, "alice").lowmem_handshake(), Some(true));
    assert!(!s.memory_status("alice").unwrap().differs());

    s.settle_write(written);
    let armed = arm(&s, "alice").lowmem_handshake();
    let differs = s.memory_status("alice").unwrap().differs();
    drop(held);
    s.flush_writes();

    assert_eq!(
        armed,
        Some(true),
        "an older successful memory save must not undo the toggle-back"
    );
    assert!(!differs, "a cancelled queue must stay cleared");
    assert_eq!(arm(&s, "alice").lowmem_handshake(), Some(true));
    assert!(!s.memory_status("alice").unwrap().differs());
    assert_eq!(
        s.operation(second).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
}

#[test]
fn a_superseded_save_settles_with_the_commit_it_was_written_in() {
    for fails in [false, true] {
        let test = format!("form-superseded-{fails}");
        let mut s = session(&test, &[("alice", 1, false)]);
        let mut form = ProfileFormSave::default();
        form.form_changed();
        let gate = s.write_gate();
        let held = gate.lock().unwrap();
        form_saves_alice(&mut s, &mut form, "first");
        // The operator discards that form; the next one saves alice again
        // before the writer got to the first.
        form.form_changed();
        let second = form_saves_alice(&mut s, &mut form, "second");
        let blocked = fails.then(|| block_writes(&vault_path(&test)));
        drop(held);
        s.flush_writes();
        if let Some(blocked) = blocked {
            unblock_writes(blocked);
        }

        let settled = outcomes(&form.settle(&mut s));
        if fails {
            assert_eq!(
                settled,
                [
                    ("failed", "alice".into(), false),
                    ("failed", "alice".into(), true),
                ]
            );
            let failures = s.take_write_failures();
            assert_eq!(
                failures.iter().map(|f| f.op()).collect::<Vec<_>>(),
                [second],
                "one report, by the write that owns the row"
            );
            assert!(form.notice().is_some_and(FormNotice::is_failed));
        } else {
            assert_eq!(
                settled,
                [
                    ("saved", "alice".into(), false),
                    ("saved", "alice".into(), true),
                ],
                "the superseded save is durable inside the later one's row"
            );
            assert_eq!(form.notice(), Some(&FormNotice::saved("Saved alice.")));
            let disk = Vault::unlock(&vault_path(&test), "test-passphrase-01").unwrap();
            assert_eq!(disk.get("alice").unwrap().password, "second");
        }
    }
}

#[test]
fn a_save_of_a_profile_deleted_before_it_was_written_leaves_nothing_to_show() {
    let mut s = session("form-deleted", &[("alice", 1, false)]);
    let mut form = ProfileFormSave::default();
    form.form_changed();
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    form_saves_alice(&mut s, &mut form, "typed");
    submit_vault_remove(&mut s, "alice");
    drop(held);
    s.flush_writes();

    assert!(
        form.settle(&mut s).is_empty(),
        "the profile is gone: no Saved, and no profile to select"
    );
    assert!(form.notice().is_none());
    assert!(!form.saving(&s));
    let disk = Vault::unlock(&vault_path("form-deleted"), "test-passphrase-01").unwrap();
    assert!(disk.get("alice").is_none());
}

#[test]
fn a_delete_that_fails_after_a_queued_save_reports_once_and_restores_the_profile() {
    let mut s = session("form-delete-fails", &[("alice", 1, false)]);
    let mut form = ProfileFormSave::default();
    form.form_changed();
    let gate = s.write_gate();
    let held = gate.lock().unwrap();
    form_saves_alice(&mut s, &mut form, "typed");
    let delete = submit_vault_remove(&mut s, "alice");
    let blocked = block_writes(&vault_path("form-delete-fails"));
    drop(held);
    s.flush_writes();
    unblock_writes(blocked);

    let failures = s.take_write_failures();
    assert_eq!(
        failures
            .iter()
            .map(|f| (f.op(), f.target()))
            .collect::<Vec<_>>(),
        [(delete, "alice")]
    );
    assert_eq!(
        outcomes(&form.settle(&mut s)),
        [("failed", "alice".into(), true)]
    );
    let restored = s.vault().unwrap().get("alice").expect("alice is back");
    assert_eq!(restored.password, "pw", "the durable row, not the draft");
}

#[test]
fn memory_toggle_before_load_applies_at_spawn_without_notice() {
    let mut s = session("mem-early", &[("alice", 1, true)]);
    let mut surface = Recorder::default();
    submit_memory_mode(&mut s, "alice", false);
    s.flush_writes();
    s.load("alice", &mut surface);
    assert_eq!(arm(&s, "alice").lowmem_handshake(), Some(false));
    assert_eq!(
        s.memory_status("alice"),
        None,
        "intent alone is not a successful handshake"
    );
    publish_alice_login(&mut s, false);
    assert!(!s.memory_status("alice").unwrap().differs());
}

#[test]
fn memory_status_is_none_without_a_recorded_login() {
    let s = session("mem-none", &[("alice", 1, false)]);
    assert_eq!(s.memory_status("alice"), None);
    assert_eq!(s.memory_status("ghost"), None);
    assert!(!s.memory_relog_warning("alice"));
}

#[test]
fn memory_relog_posts_one_atomic_intent_and_waits_for_owner_session() {
    let mut s = session("mem-relog", &[("alice", 1, true)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    s.login("alice", &mut surface);
    push_status(
        &s,
        SlotStatus {
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(true),
            ..row("alice")
        },
    );
    s.poll();

    submit_memory_mode(&mut s, "alice", false);
    s.flush_writes();
    let relog = s.request_memory_relog("alice", &mut surface);
    assert_eq!(
        s.operation(relog).unwrap().outcome("alice"),
        Some(&Outcome::Pending)
    );
    assert!(
        !arm(&s, "alice").login_latched(),
        "RelogNow must not latch Logout"
    );
    assert!(
        !arm(&s, "alice").wants_logout(),
        "no owner session to log out"
    );
    assert!(
        arm(&s, "alice").wants_login(),
        "RelogNow arms the login half"
    );
    assert!(
        s.memory_status("alice").unwrap().relog_pending,
        "the poll must see the armed relog"
    );

    // An offline status alone does not mint an owner session.
    {
        let mut rows = s.play().unwrap().statuses.lock().unwrap();
        rows[0].connected = false;
        rows[0].ingame = false;
        rows[0].login_latched = false;
    }
    s.poll();

    let alice = arm(&s, "alice");
    assert!(alice.wants_login(), "the owner retained login through exit");
    assert!(!alice.login_latched());
    assert!(
        s.memory_status("alice").unwrap().relog_pending,
        "the queued state remains visible through the login half"
    );
    assert!(
        s.memory_status("alice").unwrap().differs(),
        "an offline status cannot claim the login succeeded"
    );
    publish_alice_login(&mut s, false);
    assert!(
        s.memory_status("alice").unwrap().relog_pending,
        "a status row without a new owner SessionKey cannot settle the relog"
    );
    assert!(
        !s.memory_status("alice").unwrap().differs(),
        "the reported handshake memory mode is still reflected"
    );
}

#[test]
fn completed_relog_clears_the_pending_notice_after_new_owner_session() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-relog-complete");
    let alice = arm(&s, "alice");
    alice.complete_login_for_test();
    let previous = alice.session_key().unwrap();
    s.request_memory_relog("alice", &mut surface);
    assert!(s.memory_status("alice").unwrap().relog_pending);
    alice.complete_login_for_test();
    assert!(alice.session_key().unwrap().epoch() > previous.epoch());
    publish_alice_login(&mut s, false);
    assert!(!s.memory_status("alice").unwrap().relog_pending);
}

#[test]
fn failed_memory_relog_login_clears_the_queued_state() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-relog-login-failed");
    s.request_memory_relog("alice", &mut surface);
    park_alice(&s);
    s.poll();
    assert!(s.memory_status("alice").unwrap().relog_pending);

    let alice = arm(&s, "alice");
    alice.hold_login_on_error_for_test();
    s.poll();

    assert!(!alice.wants_login(), "the failed login remains held");
    let notice = s.memory_status("alice").unwrap();
    assert!(
        !notice.relog_pending,
        "a failed login half is no longer queued"
    );
    assert!(
        notice.differs(),
        "the failed handshake keeps the memory-mode notice"
    );
}

#[test]
fn manual_login_supersedes_a_pending_relog() {
    let mut s = session("mem-supersede", &[("alice", 1, true)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    s.login("alice", &mut surface);
    publish_alice_login(&mut s, true);
    s.request_memory_relog("alice", &mut surface);
    assert!(s.memory_status("alice").unwrap().relog_pending);
    s.login("alice", &mut surface);
    assert!(
        !s.memory_status("alice").unwrap().relog_pending,
        "an explicit Log in owns the slot again"
    );
}

#[test]
fn a_failed_memory_write_resets_the_armed_handshake() {
    let mut s = session("mem-write-fail", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    publish_alice_login(&mut s, true);
    let blocked = block_writes(&vault_path("mem-write-fail"));

    let op = submit_memory_mode(&mut s, "alice", false);
    assert_eq!(arm(&s, "alice").lowmem_handshake(), Some(false));
    s.flush_writes();
    unblock_writes(blocked);

    assert!(matches!(
        s.operation(op).unwrap().outcome("alice"),
        Some(o) if o.is_failed()
    ));
    assert!(s
        .take_write_failures()
        .iter()
        .any(|failure| failure.label() == "memory"));
    assert!(
        s.vault().unwrap().get("alice").unwrap().settings.lowmem,
        "the staged edit is rolled back to the durable value"
    );
    assert_eq!(
        arm(&s, "alice").lowmem_handshake(),
        Some(true),
        "the next handshake sends the restored value, never the refused one"
    );
    assert!(
        !s.memory_status("alice").unwrap().differs(),
        "no phantom notice survives the rollback"
    );
}

#[cfg(feature = "js")]
#[test]
fn memory_relog_warns_while_a_script_runs() {
    let iso = script::IsolatedEnv::enter("frontend-core-mem-warn");
    let mut s = session("mem-warn", &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    assert!(
        !s.memory_relog_warning("alice"),
        "no script: Relog-now needs no warning"
    );
    let (js, shape) = looping_bot(&iso.dir);
    s.start_script(
        "alice",
        ScriptStart::load(js, shape, None, Vec::new()),
        Some("file:looping".into()),
    )
    .unwrap();
    let settled = settle_start(&mut s);
    assert!(
        settled
            .iter()
            .any(|st| st.outcome == Some(script::StartOutcome::Ready)),
        "the looping script runs: {settled:?}"
    );
    assert!(
        s.memory_relog_warning("alice"),
        "a running script is interrupted by a relog"
    );

    let stop = s.stop_script("alice");
    wait_operation_settled(&mut s, stop);
    assert!(
        !s.memory_relog_warning("alice"),
        "a stopped script needs no warning"
    );
}

#[cfg(not(feature = "js"))]
#[test]
fn memory_relog_warns_while_a_compiled_script_runs() {
    let iso = script::IsolatedEnv::enter("frontend-core-mem-warn");
    let mut s = session("mem-warn", &[("alice", 1, false)]);
    let mut scripts = test_scripts(&iso.dir);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    assert!(
        !s.memory_relog_warning("alice"),
        "no script: Relog-now needs no warning"
    );
    start_compiled_script(&mut s, &mut scripts, "alice");
    assert!(
        s.memory_relog_warning("alice"),
        "a running script is interrupted by a relog"
    );

    let stop = s.stop_script("alice");
    wait_compiled_script_state(&mut s, &mut scripts, "alice", script::RunState::Idle);
    wait_operation_settled(&mut s, stop);
    assert!(
        !s.memory_relog_warning("alice"),
        "a stopped script needs no warning"
    );
}

fn memory_ingame_toggled(test: &str) -> (OperatorSession<u32>, Recorder) {
    let mut s = session(test, &[("alice", 1, false)]);
    let mut surface = Recorder::default();
    s.load("alice", &mut surface);
    s.login("alice", &mut surface);
    push_status(
        &s,
        SlotStatus {
            username: "alice".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(true),
            ..SlotStatus::default()
        },
    );
    s.poll();
    submit_memory_mode(&mut s, "alice", false);
    s.flush_writes();
    assert!(s.memory_status("alice").unwrap().differs());
    (s, surface)
}

fn park_alice(s: &OperatorSession<u32>) {
    let mut rows = s.play().unwrap().statuses.lock().unwrap();
    let row = rows
        .iter_mut()
        .find(|row| row.username == "alice")
        .expect("alice status");
    row.connected = false;
    row.ingame = false;
    row.scene_state = 0;
    row.login_latched = true;
}

#[test]
fn logged_out_memory_change_waits_for_login_without_relog_offer() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-offline-offer");
    s.logout("alice");
    park_alice(&s);
    s.poll();

    let notice = s.memory_status("alice").unwrap();
    assert!(notice.differs(), "logout must not discard the queued mode");
    assert!(
        !notice.can_relog(),
        "an offline slot has no session to relog"
    );
    assert!(!notice.notice_text().contains("Relog now"));
    s.login("alice", &mut surface);
    publish_alice_login(&mut s, false);
    assert!(!s.memory_status("alice").unwrap().differs());
}

#[test]
fn user_logout_cancels_a_pending_memory_relog() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-user-logout");
    s.request_memory_relog("alice", &mut surface);

    s.logout("alice");
    park_alice(&s);
    s.poll();

    let alice = arm(&s, "alice");
    assert!(
        !alice.wants_login(),
        "Logout must cancel the relog's login half"
    );
    assert!(alice.login_latched(), "the user's logout remains latched");
    assert!(!s.memory_status("alice").unwrap().relog_pending);
}

#[test]
fn user_logout_all_cancels_a_pending_memory_relog() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-user-logout-all");
    s.request_memory_relog("alice", &mut surface);

    s.logout_all();
    park_alice(&s);
    s.poll();

    let alice = arm(&s, "alice");
    assert!(
        !alice.wants_login(),
        "Logout all cancels the relog's login half"
    );
    assert!(alice.login_latched(), "the fleet logout remains latched");
}

#[test]
fn completed_memory_relog_clears_the_fleet_logout_latch() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-fleet-latch");
    s.request_memory_relog("alice", &mut surface);
    park_alice(&s);

    s.poll();

    assert!(!s.fleet().latched("alice"));
    assert!(s.fleet().should_auto_login("alice", true));
    assert!(!s.arm_for_profile("alice").unwrap().login_latched());
}

#[test]
fn login_all_without_a_handshake_keeps_the_memory_notice() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-login-all-ingame");

    s.login_all(&mut surface);
    s.poll();

    assert!(
        s.memory_status("alice").unwrap().differs(),
        "an in-game Login all cannot claim a handshake changed server mode"
    );
}

#[test]
fn login_that_cancels_a_pending_relog_keeps_the_memory_notice() {
    let (mut s, mut surface) = memory_ingame_toggled("mem-login-cancels-relog");
    s.request_memory_relog("alice", &mut surface);

    s.login("alice", &mut surface);
    s.poll();

    assert!(!arm(&s, "alice").wants_logout());
    assert!(
        s.memory_status("alice").unwrap().differs(),
        "no completed handshake means the server still uses the old mode"
    );
}

struct MemoryOverrideSurface;

impl SlotSurface for MemoryOverrideSurface {
    type Io = u32;

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut crate::SpawnProfile<'_>,
        retained: Option<u32>,
    ) -> SlotAttach<u32> {
        profile.set_lowmem(false);
        SlotAttach::new(retained.unwrap_or_default(), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> crate::Models<'_> {
        crate::Models::none()
    }

    fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {}
}

#[test]
fn session_memory_override_is_the_effective_mode_not_a_permanent_notice() {
    let mut s = session("mem-session-override", &[("alice", 1, false)]);
    let mut surface = MemoryOverrideSurface;
    s.load("alice", &mut surface);
    s.login("alice", &mut surface);
    publish_alice_login(&mut s, false);

    let notice = s.memory_status("alice").expect("spawn memory mode");
    assert!(
        !notice.differs(),
        "the session override is both the handshake and desired mode"
    );
}

fn capacity_log() -> Vec<(Level, String)> {
    let mut view = crate::log::LogView::new(crate::log::LogScope::Process);
    view.edit_filter(|f| f.set_min_level(Level::Debug));
    crate::log::global().refresh(&mut view);
    view.rows()
        .iter()
        .filter(|e| e.message().starts_with("Host "))
        .map(|e| (e.level(), e.message().to_string()))
        .collect()
}

#[test]
fn host_capacity_transitions_log_once_and_rebuild_the_line_only_on_change() {
    use crate::capacity::RenderPolicyLabel;
    use host::{CapacityEval, CapacityWarning};
    crate::log::global();
    let before = capacity_log().len();
    let mut s = OperatorSession::<u32>::new(InstancePermit::SkipLock);
    let warn = |late| {
        CapacityEval::Warn(CapacityWarning::new(
            late,
            71,
            8,
            Some(Duration::from_millis(1730)),
            Some(Duration::from_millis(970)),
            RenderPolicyLabel::Headless.to_host(),
        ))
    };
    let line = |late| {
        format!(
            "Host is falling behind: {late} of 71 bots late (p95 delay 1.73s, 0.97 s/tick, \
             render headless). Run fewer bots."
        )
    };

    let generation = s.capacity_generation();
    s.apply_capacity(warn(17));
    assert_eq!(s.capacity_notice().unwrap().notice_text(), line(17));
    let shown = s.capacity_generation();
    assert_ne!(shown, generation);
    let text = s.capacity_notice().unwrap().notice_text().as_ptr();
    s.apply_capacity(warn(17));
    assert_eq!(s.capacity_generation(), shown, "same figures: no rebuild");
    assert_eq!(s.capacity_notice().unwrap().notice_text().as_ptr(), text);
    s.apply_capacity(warn(18));
    assert_eq!(s.capacity_notice().unwrap().notice_text(), line(18));
    assert_ne!(s.capacity_generation(), shown);

    // Warn -> Silence -> Clear: hidden once, no second clear line.
    s.apply_capacity(CapacityEval::Silence);
    assert!(s.capacity_notice().is_none());
    s.apply_capacity(CapacityEval::Clear);
    // Clear -> Warn -> Clear.
    s.apply_capacity(warn(17));
    s.apply_capacity(CapacityEval::Clear);
    assert!(s.capacity_notice().is_none());
    // Warn, then the play goes away: the next evaluate clears and logs.
    s.apply_capacity(warn(17));
    let now = Instant::now();
    s.refresh_capacity(now);
    assert!(s.capacity_notice().is_none());
    assert_eq!(s.capacity_due, Some(now + CAPACITY_EVAL_EVERY));

    let hidden = "Host warning hidden: no game packets for 10 s, so the game server is quiet, \
                  not this machine";
    let back = "Host is keeping up again";
    assert_eq!(
        capacity_log()[before..],
        [
            (Level::Warn, line(17)),
            (Level::Info, hidden.to_string()),
            (Level::Warn, line(17)),
            (Level::Info, back.to_string()),
            (Level::Warn, line(17)),
            (Level::Info, back.to_string()),
        ]
    );
}

fn dirty(
    s: &OperatorSession<u32>,
    target: Option<&str>,
    username: &str,
    password: &str,
    settings: &ProfileSettings,
) -> bool {
    s.profile_form_dirty(
        target,
        &crate::ProfileDraft::from_parts(username, password, settings.clone()),
    )
}

/// Reopen both durable stores, adopt ranks while locked, then insert clients
/// without a poll between unlock and Load. The vault UID survives a rename.
#[test]
fn o1_restart_applies_saved_order_before_individual_loads() {
    let path = vault_path("o1-restart-order");
    let vault = vault_with(
        "o1-restart-order",
        &[("A", 1, false), ("B", 2, false), ("C", 3, false)],
    );
    drop(vault);
    let root = path.parent().unwrap().join("state");
    {
        let preferences = crate::UiPreferences::open_for_test(root.clone());
        preferences
            .set(
                crate::PROFILE_ORDER_KEY,
                serde_json::json!({"3": 10, "1": 20, "2": 30}),
            )
            .unwrap();
    }
    let service = host_play::local_state::LocalStateService::open_process(
        host_play::local_state::StateRoot::new(root),
    );
    let mut session = OperatorSession::<u32>::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.bind_local_state(service);
    session.preferences().unwrap().reload().unwrap();
    session.adopt_operator_preferences();
    assert!(!session.vault_open());
    let mut vault = Vault::unlock(&path, "test-passphrase-01").unwrap();
    let mut renamed = vault.get("A").unwrap().clone();
    renamed.username = "A-renamed".into();
    vault.remove("A").unwrap();
    vault.upsert(renamed).unwrap();
    session.start(vault, empty_play());
    assert_eq!(session.profile_names(), ["C", "A-renamed", "B"]);
    let mut surface = Recorder::default();
    for (name, expected) in [
        ("B", vec!["B"]),
        ("C", vec!["C", "B"]),
        ("A-renamed", vec!["C", "A-renamed", "B"]),
    ] {
        session.load(name, &mut surface);
        assert_eq!(session.members(), expected);
    }
    assert!(
        session.order_writes.is_empty(),
        "projection alone must not save"
    );
}

#[test]
fn o1_same_length_uid_name_projection_invalidates_without_a_save() {
    let mut session = session(
        "o1-sequence-generation",
        &[("A", 1, false), ("B", 2, false)],
    );
    session.reconcile_profile_order();
    session.order_saved = true;
    let mut fleet = crate::FleetModel::new();
    assert!(fleet.refresh(&session));
    assert_eq!(
        fleet
            .rows()
            .iter()
            .map(|row| row.name())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    // The authoritative vault now maps the same two UIDs to exchanged names.
    // Stored rank keys do not change, but cached Fleet rows must rebuild.
    let vault = session.vault.as_mut().unwrap();
    let mut a = vault.get("A").unwrap().clone();
    let mut b = vault.get("B").unwrap().clone();
    a.username = "B".into();
    b.username = "A".into();
    vault.upsert(a).unwrap();
    vault.upsert(b).unwrap();
    session.order_reconciled_at = None;
    let generation = session.profiles_generation;
    session.reconcile_profile_order();
    assert_eq!(session.profile_order().names(), ["B", "A"]);
    assert_ne!(
        session.profiles_generation, generation,
        "publish the changed sequence"
    );
    assert!(fleet.refresh(&session));
    assert_eq!(
        fleet
            .rows()
            .iter()
            .map(|row| row.name())
            .collect::<Vec<_>>(),
        ["B", "A"]
    );
    assert!(session.order_writes.is_empty());
}

/// T5.3-S3: a quiet Load-order write (the reconcile after the vault gained
/// a profile) that fails is reported at the Load order setting. The shown
/// order keeps the vault's truth, the stored order (the file) stays what it
/// was, the error expires, and a later reorder saves the new order.
#[test]
fn a_failed_quiet_load_order_write_is_reported_and_the_stored_order_stays() {
    use std::sync::atomic::AtomicBool;
    let iso = crate::test_support::IsolatedStores::enter("s3-quiet-order");
    let root = iso.dir().join("state");
    {
        let preferences = crate::UiPreferences::open_for_test(root.clone());
        preferences
            .set(
                crate::PROFILE_ORDER_KEY,
                serde_json::json!({"1": 10, "2": 20}),
            )
            .unwrap();
    }
    let service = host_play::local_state::LocalStateService::open_process(
        host_play::local_state::StateRoot::new(root),
    );
    let fail = Arc::new(AtomicBool::new(true));
    let refusing = Arc::clone(&fail);
    service.set_write_fault_for_test(Arc::new(move || refusing.load(Ordering::SeqCst)));
    let mut session = OperatorSession::<u32>::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.bind_local_state(service);
    session.preferences().unwrap().reload().unwrap();
    session.adopt_operator_preferences();
    session.start(
        vault_with("s3-quiet-order", &[("A", 1, false), ("B", 2, false)]),
        empty_play(),
    );
    assert_eq!(session.profile_names(), ["A", "B"]);
    let path = session.preferences().unwrap().path();
    let stored = std::fs::read(&path).unwrap();
    session.watch_setting_events(true);

    // The vault gains C: the reconcile appends it and writes the order.
    session
        .vault
        .as_mut()
        .unwrap()
        .upsert(Profile {
            username: "C".into(),
            password: "pw".into(),
            uid: 3,
            settings: ProfileSettings::default(),
        })
        .unwrap();
    session.order_reconciled_at = None;
    session.reconcile_profile_order();
    assert!(
        !session.order_writes.is_empty(),
        "the quiet write was queued"
    );
    assert!(matches!(
        session.setting_status(crate::Setting::ProfileOrder),
        crate::SettingStatus::Pending
    ));
    let (events, incomplete) = session.take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [crate::SettingEventState::Pending]
    );
    assert_eq!(events[0].setting(), "profile order");
    let mut surface = Recorder::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while session
        .setting_failure(crate::Setting::ProfileOrder)
        .is_none()
    {
        session.poll_preferences(&mut surface);
        assert!(
            Instant::now() < deadline,
            "the failed write is never reported"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session
        .setting_failure(crate::Setting::ProfileOrder)
        .is_some_and(|text| text.starts_with("Load order not saved: ")));
    let (events, incomplete) = session.take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [crate::SettingEventState::Failed]
    );
    assert_eq!(events[0].setting(), "profile order");
    assert!(
        events[0].error().is_some_and(|error| error.contains("busy")),
        "{events:?}"
    );
    session.poll_preferences_at(
        &mut surface,
        Instant::now() + crate::ERROR_MESSAGE_TIMEOUT,
    );
    assert_eq!(
        session.setting_failure(crate::Setting::ProfileOrder),
        None,
        "the error timeout clears the failure"
    );
    let (events, incomplete) = session.take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [crate::SettingEventState::Cleared]
    );
    assert_eq!(events[0].setting(), "profile order");
    assert_eq!(
        session.profile_order().names(),
        ["A", "B", "C"],
        "the shown order keeps the vault's truth"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        stored,
        "the stored order is what it was"
    );
    assert!(session.order_writes.is_empty());

    // The disk recovers: a reorder is the action that resolves the failure.
    fail.store(false, Ordering::SeqCst);
    let (c, a) = (
        session.profile_identity("C").unwrap(),
        session.profile_identity("A").unwrap(),
    );
    session.stage_reorder(crate::ReorderOp::Move {
        moving: c,
        to: crate::Placement::Before(a),
    });
    struct FleetLend<'a>(&'a mut crate::FleetModel);
    impl SlotSurface for FleetLend<'_> {
        type Io = u32;
        fn attach(
            &mut self,
            _name: &str,
            _profile: &mut crate::SpawnProfile<'_>,
            _retained: Option<u32>,
        ) -> SlotAttach<u32> {
            SlotAttach::new(0, None, None)
        }
        fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}
        fn released(&mut self, _name: &str) {}
        fn models(&mut self) -> crate::Models<'_> {
            crate::Models::none().with_fleet(&mut *self.0)
        }
        fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {}
    }
    let mut fleet = crate::FleetModel::new();
    fleet.refresh(&session);
    let submitted = session
        .submit(
            OperatorCommand::ReorderProfiles,
            crate::session::Targets::Application,
            &mut FleetLend(&mut fleet),
        )
        .expect("the reorder is admitted");
    assert_eq!(
        session.command_failure(submitted.request()),
        None,
        "the reorder is not refused"
    );
    assert_eq!(
        session.setting_failure(crate::Setting::ProfileOrder),
        None,
        "the new write clears the failure it resolves"
    );
    let (events, incomplete) = session.take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [crate::SettingEventState::Pending]
    );
    assert_eq!(events[0].setting(), "profile order");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !matches!(
        session.setting_status(crate::Setting::ProfileOrder),
        crate::SettingStatus::Saved
    ) {
        session.poll_preferences(&mut surface);
        assert!(Instant::now() < deadline, "the retry never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    let (events, incomplete) = session.take_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [crate::SettingEventState::Saved]
    );
    assert_eq!(events[0].setting(), "profile order");
    let file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let order = file[crate::PROFILE_ORDER_KEY].as_object().unwrap();
    assert_eq!(order.len(), 3, "the stored order now holds every profile");
}
