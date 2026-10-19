//! The TUI event loop's per-iteration functions (`headless_step`, the
//! no-terminal loop, and `headed_frame`, the headed loop's pump and draw).
//! Each test runs the iteration the loop runs and never calls
//! `TuiSession::pump` itself, so an iteration that stops pumping the session
//! (the call the loops make before everything else) fails here. The headed
//! iteration draws onto a `TestBackend`.

use std::sync::atomic::Ordering;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use host_play::{SlotArm, SlotStatus};

/// Which loop iteration a scenario runs.
#[derive(Clone, Copy, Debug)]
enum Loop {
    Headless,
    Headed,
}

struct Rig {
    mode: Loop,
    session: TuiSession,
    app: TuiApp,
    terminal: Terminal<TestBackend>,
    deferred: Vec<ProofLine>,
}

impl Rig {
    fn new(mode: Loop, session: TuiSession) -> Self {
        Self {
            mode,
            session,
            app: TuiApp::new("274bot loop"),
            terminal: Terminal::new(TestBackend::new(120, 40)).expect("a test terminal"),
            deferred: Vec::new(),
        }
    }

    /// One loop iteration; the live run's exit code once it decided.
    fn iterate(&mut self) -> Option<i32> {
        match self.mode {
            Loop::Headless => headless_step(&mut self.session, &mut self.app),
            Loop::Headed => headed_frame(
                &mut self.session,
                &mut self.app,
                &mut self.terminal,
                &mut self.deferred,
            )
            .expect("the test terminal draws"),
        }
    }

    /// What the headed iteration drew last.
    fn screen(&self) -> String {
        crate::test_support::text(&crate::test_support::rows(self.terminal.backend().buffer()))
    }
}

fn options() -> PlayOptions {
    PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    }
}

/// A session over a vault holding `alice` and a `Play` with her arm, her
/// fleet row and the selection.
fn session_with_alice() -> (TuiSession, Arc<SlotArm>) {
    let mut session = TuiSession::new(options());
    session.core.set_spawn_workers(false);
    let path = frontend_core::scripts::paths::bot_home().join("loop-vault");
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(1));
    install_vault(&mut session.core, vault);
    let arm = SlotArm::new(1, false);
    let mut play = frontend_core::test_support::empty_play(&options());
    play.attach_arm("alice", Arc::clone(&arm));
    session.core.set_play(Some(play));
    session.core.fleet_mut().add("alice");
    session
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    (session, arm)
}

fn publish(session: &TuiSession, connected: bool) {
    let mut rows = session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap();
    rows.clear();
    rows.push(SlotStatus {
        username: "alice".into(),
        connected,
        ..SlotStatus::default()
    });
}

macro_rules! scenario {
    ($name:ident, $body:ident) => {
        mod $name {
            use super::*;

            #[test]
            fn headless() {
                $body(Loop::Headless);
            }

            #[test]
            fn headed() {
                $body(Loop::Headed);
            }
        }
    };
}

/// Every iteration polls the host: a status the slot published is on the
/// fleet row after one iteration, and the headed iteration draws it.
fn iteration_polls_the_host(mode: Loop) {
    let _iso = IsolatedStores::enter("tui-loop-host");
    let (session, _arm) = session_with_alice();
    publish(&session, true);
    let mut rig = Rig::new(mode, session);
    assert!(rig.app.fleet.is_empty(), "nothing is polled before a pass");
    assert_eq!(rig.iterate(), None);
    assert!(
        rig.app.fleet.first().is_some_and(|row| row.connected()),
        "one iteration must poll the host and project the fleet row"
    );
    if matches!(mode, Loop::Headed) {
        assert!(
            rig.screen().contains("alice"),
            "the draw follows the pump in the same iteration:\n{}",
            rig.screen()
        );
    }
}

scenario!(host_poll, iteration_polls_the_host);

/// Every iteration advances a pending removal: the iteration after the slot
/// disconnected stops the removed lifetime.
fn iteration_advances_a_pending_removal(mode: Loop) {
    let _iso = IsolatedStores::enter("tui-loop-removal");
    let (session, arm) = session_with_alice();
    publish(&session, true);
    let mut rig = Rig::new(mode, session);
    rig.iterate();
    rig.session.remove(&mut rig.app, "alice");
    assert_eq!(rig.app.error, None);
    assert!(rig.session.core.removal_pending("alice"));
    assert!(
        !arm.stop.load(Ordering::Relaxed),
        "a connected slot gets a clean logout first"
    );
    publish(&rig.session, false);
    rig.iterate();
    assert!(
        arm.stop.load(Ordering::Relaxed),
        "the iteration after the disconnect must stop the removed slot"
    );
}

scenario!(pending_removal, iteration_advances_a_pending_removal);

/// The saved manual-walk pause, once the preferences file holds one.
fn saved_pause(rig: &Rig) -> Option<bool> {
    let bytes = std::fs::read(rig.session.preferences.path()).ok()?;
    let saved: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    saved["nav"]["pause_script_on_manual_walk_abort"].as_bool()
}

/// Iterations until the preferences file holds `want`. A write the disk
/// holds past the bounded wait stays queued and the following iterations
/// settle it; [`HANG_GUARD`] only ends a test whose writer hung.
fn iterate_until_saved(rig: &mut Rig, want: bool) {
    let start = std::time::Instant::now();
    while saved_pause(rig) != Some(want) {
        assert!(
            start.elapsed() < HANG_GUARD,
            "no iteration settled the preference write before the hang guard"
        );
        assert_eq!(rig.iterate(), None);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Every iteration persists what the operator changed: the pump submits
/// the preference write in the same pass, and the iterations settle it
/// when the disk confirms it.
fn iteration_settles_a_preference_write(mode: Loop) {
    let _iso = IsolatedStores::enter("tui-loop-preference");
    let (session, _arm) = session_with_alice();
    let mut rig = Rig::new(mode, session);
    crate::bin::seam::Seam::restore_preferences(&mut rig.app, rig.session.preferences.share());
    rig.app.pause_script_on_manual_walk_abort = true;
    rig.app.pause_script_on_manual_walk_abort_dirty = true;
    rig.iterate();
    assert!(
        !rig.app.pause_script_on_manual_walk_abort_dirty,
        "one iteration must submit the changed preference"
    );
    assert_eq!(rig.app.error, None);
    iterate_until_saved(&mut rig, true);

    rig.app.pause_script_on_manual_walk_abort = false;
    rig.app.pause_script_on_manual_walk_abort_dirty = true;
    rig.iterate();
    assert!(
        !rig.app.pause_script_on_manual_walk_abort_dirty,
        "one iteration must submit the changed preference"
    );
    assert_eq!(rig.app.error, None);
    iterate_until_saved(&mut rig, false);
}

scenario!(preference_write, iteration_settles_a_preference_write);

/// The 289 game data a bound profile selects, prepared on the shared
/// worker as a profile bind does.
fn selected_game_data() -> Arc<api::game_data::SelectedGameData> {
    api::selected::FamilyPreparation::run(|_| {
        api::game_data::for_revision(api::selected::ClientRevision::R289)
            .expect("selected 289 game data")
    })
    .expect("spawn selected data worker")
    .join()
    .expect("selected data worker")
}

/// Hang guard for a worker (the preference writer, the Path reload); the
/// pass condition is the product's own signal.
const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(600);

/// Every iteration advances a Quester Paths reload (T3.3 r5, review
/// finding 9), before the headed iteration draws: the iteration that
/// settles the reload draws the settings popup with the reload finished.
/// An iteration that stops advancing it never settles it; one that
/// advances it after the draw shows the finished reload as running.
fn iteration_settles_a_quester_paths_reload(mode: Loop) {
    let _iso = IsolatedStores::enter("tui-loop-quester-paths");
    let _paths = crate::bin::seam::lock_quester_paths_test_state();
    let (session, _arm) = session_with_alice();
    let mut rig = Rig::new(mode, session);
    rig.app.settings_state.open = true;
    // The popup scrolls to the selected row: Reload Paths.
    rig.app.settings_state.row = 14;
    rig.session
        .core
        .stage_quester_reload(Some(selected_game_data()));
    let request = rig
        .session
        .submit_with(
            &mut rig.app,
            OperatorCommand::ReloadQuesterPaths,
            Targets::Application,
        )
        .expect("the reload is admitted")
        .request();
    assert!(
        rig.app.quester_paths_controller.is_running(),
        "the reload runs on the shared worker"
    );
    let start = std::time::Instant::now();
    while !rig
        .session
        .core
        .commands()
        .report(request)
        .and_then(|report| report.member(frontend_core::command::Subject::Application))
        .is_some_and(|member| member.terminal().is_some())
    {
        assert!(
            start.elapsed() < HANG_GUARD,
            "no iteration settled the Path reload before the hang guard"
        );
        assert_eq!(rig.iterate(), None);
        std::thread::yield_now();
    }
    assert!(!rig.app.quester_paths_controller.is_running());
    assert!(rig.app.quester_paths_controller.notice_text().is_some());
    if matches!(mode, Loop::Headed) {
        let screen = rig.screen();
        assert!(
            screen.contains(frontend_core::quester_paths::RELOAD_PATHS_LABEL),
            "the popup shows Reload Paths:\n{screen}"
        );
        assert!(
            !screen.contains("(reloading…)"),
            "the iteration that settled the reload drew it as running:\n{screen}"
        );
    }
    frontend_core::test_support::scripts::reset_quester_registry();
}

scenario!(
    quester_paths_reload,
    iteration_settles_a_quester_paths_reload
);
