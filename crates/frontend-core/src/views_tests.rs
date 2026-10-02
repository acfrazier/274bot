//! Projection proofs through the real poll: a real [`host_play::Play`] (no
//! server contact), real arms, host rows published the way slot workers
//! publish them. Slot names are unique per test: the log is process-wide.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use host_play::{InstancePermit, Play, PlayOptions, SlotArm, SlotStatus, StartupPhase};
use vault::{Profile, ProfileSettings, Vault};

use super::*;
use crate::log::{LogScope, LogView};
use crate::resources::{Metric, ProcessProbe, ResourceView};
use crate::session::OperatorSession;
use crate::surface::HeadlessSurface;

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

/// Members loaded from a fresh vault (arms attached, no worker threads).
/// `auto_login` decides whether a parked member wants a login.
fn fleet(test: &str, members: &[&str], auto_login: bool) -> OperatorSession<()> {
    let dir = std::env::temp_dir().join(format!(
        "274bot-frontend-core-views-{}-{test}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "test-passphrase-01").unwrap();
    for (i, name) in members.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*name).into(),
                password: "pw".into(),
                uid: 1 + i as i32,
                settings: ProfileSettings {
                    auto_login,
                    ..ProfileSettings::default()
                },
            })
            .unwrap();
    }
    let mut session = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.start(vault, empty_play());
    let mut surface = HeadlessSurface::new();
    for name in members {
        session.load(name, &mut surface);
    }
    session
}

/// Publish (or edit) `name`'s host row, as its slot worker would.
fn publish(s: &OperatorSession<()>, name: &str, edit: impl FnOnce(&mut SlotStatus)) {
    let play = s.play().unwrap();
    let mut rows = play.statuses.lock().unwrap();
    match rows.iter_mut().find(|row| row.username == name) {
        Some(row) => edit(row),
        None => {
            let mut row = SlotStatus {
                username: name.into(),
                ..SlotStatus::default()
            };
            edit(&mut row);
            rows.push(row);
        }
    }
}

fn row<'a>(s: &'a OperatorSession<()>, name: &str) -> &'a FleetRow {
    s.fleet_view().row(name).expect(name)
}

fn detail(s: &OperatorSession<()>) -> &SlotDetail {
    s.fleet_view().detail().expect("a selected slot")
}

fn phase_to(row: &mut SlotStatus, phase: StartupPhase) {
    row.startup_phase = phase;
    row.startup_phase_started = Instant::now();
}

/// Every step of one member's way from loaded to logged out, observed only
/// through the poll (the slot has no observe of its own while offline).
#[test]
fn a_member_row_follows_every_offline_transition() {
    let mut s = fleet("transitions", &["vt-alice"], true);
    s.select("vt-alice");
    s.poll();
    let mut last = s.fleet_view().generation();
    let mut step = |s: &mut OperatorSession<()>, edit: &dyn Fn(&mut SlotStatus)| {
        publish(s, "vt-alice", edit);
        s.poll();
        let now = s.fleet_view().generation();
        assert!(now > last, "a visible change moves the generation");
        last = now;
    };
    assert_eq!(
        row(&s, "vt-alice").phase,
        Phase::Offline,
        "no row published"
    );
    assert_eq!(detail(&s).state, "offline");

    step(&mut s, &|r| {
        phase_to(r, StartupPhase::Preparing);
        r.startup_progress_message = "Loading sprites".into();
        r.startup_progress_percent = Some(40);
    });
    assert_eq!(row(&s, "vt-alice").brief, "starting");
    assert_eq!(detail(&s).state, "Loading sprites (40%)");
    assert!(detail(&s).since.is_some(), "startup shows a timer");

    step(&mut s, &|r| {
        phase_to(r, StartupPhase::Queueing);
        r.startup_progress_message.clear();
        r.startup_progress_percent = None;
        (r.queue_position, r.queue_total) = (2, 3);
    });
    let queued = row(&s, "vt-alice");
    assert_eq!(queued.phase, Phase::Queued);
    assert_eq!(
        queued.queue,
        Some(QueuePlace {
            position: 2,
            total: 3
        })
    );
    assert_eq!(queued.queue.unwrap().ahead(), 1);
    assert_eq!(queued.brief, "queued 2/3");
    assert_eq!(queued.light(), Light::Grey);
    assert_eq!(detail(&s).state, "waiting in login queue (2/3)");
    assert!(detail(&s).since.is_some(), "the queue wait shows a timer");
    assert_eq!(s.fleet_view().counts().queued, 1);

    // A place outside its total is not a place (the old TUI showed "3 of 2"),
    // so this auto-login member just waits to connect (the old rail said
    // "logged out").
    step(&mut s, &|r| (r.queue_position, r.queue_total) = (3, 2));
    assert_eq!(row(&s, "vt-alice").queue, None);
    assert_eq!(row(&s, "vt-alice").phase, Phase::Waiting);
    assert_eq!(detail(&s).state, "waiting to connect");
    assert!(detail(&s).since.is_some());

    step(&mut s, &|r| {
        phase_to(r, StartupPhase::Connecting);
        (r.queue_position, r.queue_total) = (-1, -1);
    });
    assert_eq!(row(&s, "vt-alice").brief, "logging in");
    assert!(detail(&s).since.is_some());

    step(&mut s, &|r| {
        r.startup_progress_message = "Your profile will be transferred in: 3 seconds".into()
    });
    assert_eq!(
        detail(&s).state,
        "Your profile will be transferred in: 3 seconds"
    );
    assert_eq!(
        detail(&s).since,
        None,
        "a server countdown gets no rising timer beside it"
    );

    step(&mut s, &|r| {
        phase_to(r, StartupPhase::LoadingScene);
        r.startup_progress_message.clear();
        r.connected = true;
    });
    let loading = row(&s, "vt-alice");
    assert_eq!(
        (loading.phase, loading.light()),
        (Phase::Loading, Light::Yellow)
    );
    assert_eq!(detail(&s).state, "loading scene…");

    step(&mut s, &|r| {
        phase_to(r, StartupPhase::Ready);
        r.ingame = true;
        r.scene_state = 2;
        r.player = "Alice".into();
        (r.tile_x, r.tile_z, r.tile_level) = (3200, 3201, 0);
    });
    let ready = row(&s, "vt-alice");
    assert_eq!((ready.phase, ready.brief.as_str()), (Phase::Ready, "idle"));
    assert_eq!(detail(&s).state, "ingame scene 2");
    assert_eq!(detail(&s).player, "Alice");
    assert_eq!(detail(&s).ready_tile, Some((3200, 3201, 0)));
    assert_eq!(detail(&s).since, None);
    assert_eq!(s.fleet_view().counts().ready, 1);

    step(&mut s, &|r| r.walk_x = 3210);
    assert_eq!(row(&s, "vt-alice").brief, "running");
    assert_eq!(row(&s, "vt-alice").light(), Light::Green);

    // The connection drops: the host parks it for the retry.
    step(&mut s, &|r| {
        phase_to(r, StartupPhase::Queueing);
        (r.ingame, r.connected, r.walk_x) = (false, false, -1);
    });
    assert_eq!(row(&s, "vt-alice").phase, Phase::Waiting);
    assert_eq!(detail(&s).ready_tile, None, "no routing origin offline");

    let logout = s.logout("vt-alice");
    step(&mut s, &|_| {});
    let parked = row(&s, "vt-alice");
    assert_eq!(parked.phase, Phase::LoggedOut);
    assert_eq!(detail(&s).state, "logged out (Log in to connect)");
    assert_eq!(detail(&s).since, None, "nothing is in flight");
    assert_eq!(
        parked.last_op.as_ref().map(|op| (op.id, op.action)),
        Some((logout, ActionKind::Logout))
    );
    assert_eq!(
        parked.last_op.as_ref().unwrap().outcome,
        Outcome::Completed,
        "an offline logout settles through the poll"
    );
}

#[test]
fn a_parked_member_without_login_intent_is_logged_out_until_log_in() {
    let mut s = fleet("parked", &["vp-carol"], false);
    publish(&s, "vp-carol", |r| phase_to(r, StartupPhase::Queueing));
    s.poll();
    assert_eq!(
        row(&s, "vp-carol").phase,
        Phase::LoggedOut,
        "auto-login off: parked on the title, not waiting"
    );
    s.login("vp-carol", &mut HeadlessSurface::new());
    s.poll();
    assert_eq!(row(&s, "vp-carol").phase, Phase::Waiting);
}

#[test]
fn repeat_logout_guard_reason_is_visible_in_slot_status() {
    let row = FleetRow::fixture("alice", Phase::LoggedOut, None, None);
    let status = SlotStatus {
        username: "alice".into(),
        login_latched: true,
        login_latch_reason: Some(host_play::LoginLatchReason::RepeatedUnexpectedLogouts {
            count: 3,
            window_seconds: 600,
        }),
        ..SlotStatus::default()
    };
    let mut text = String::new();

    write_state(&mut text, &row, Some(&status), false);

    assert_eq!(
        text,
        "logged out (repeat guard: 3 unexpected logouts in 600s; Log in to retry)"
    );
}

#[test]
fn manual_walk_cancellation_status_replaces_the_scene_detail() {
    let row = FleetRow::fixture("alice", Phase::Ready, None, None);
    let status = SlotStatus {
        scene_state: 2,
        ..SlotStatus::default()
    };
    let mut text = String::new();

    write_state(&mut text, &row, Some(&status), true);

    assert_eq!(text, "cancelled by user input");
}

/// Turning Auto login off while a failed login waits for its retry parks
/// the slot (the host stops retrying): logged out, with the error kept as
/// history. An error that itself withdrew the login stays a failure.
#[test]
fn auto_login_off_during_a_retry_wait_parks_the_slot_with_the_error_as_history() {
    let mut s = fleet("autologin-off", &["va-gus"], true);
    s.select("va-gus");
    publish(&s, "va-gus", |r| {
        phase_to(r, StartupPhase::Error);
        r.error = Some("code 5: retry later".into());
    });
    s.poll();
    assert_eq!(row(&s, "va-gus").phase, Phase::LoginError);

    s.set_auto_login("va-gus", false).unwrap();
    s.flush_writes();
    s.poll();
    let parked = row(&s, "va-gus");
    assert_eq!(
        (parked.phase, parked.light()),
        (Phase::LoggedOut, Light::Grey)
    );
    assert_eq!(parked.error.as_deref(), Some("code 5: retry later"));
    assert!(!parked.has_failure());
    assert_eq!(s.fleet_view().counts().failed, 0);
    assert_eq!(detail(&s).state, "logged out (Log in to connect)");

    s.set_auto_login("va-gus", true).unwrap();
    s.flush_writes();
    s.poll();
    assert_eq!(
        row(&s, "va-gus").phase,
        Phase::LoginError,
        "wanted again: the retry is due"
    );

    // A public world preference error withdraws the login itself.
    s.play()
        .unwrap()
        .arm("va-gus")
        .unwrap()
        .hold_login_on_error_for_test();
    s.poll();
    assert_eq!(row(&s, "va-gus").phase, Phase::LoginError);
    assert!(row(&s, "va-gus").has_failure());
}

/// A failed login stays visible until the slot is in game. While the slot
/// still wants the login the error is a retry wait: the header counts it
/// as waiting for a login as well as failed. An error that withdrew the
/// login waits for the operator, not for a login.
#[test]
fn a_login_error_stays_visible_while_retrying_until_ready() {
    let mut s = fleet("error-retained", &["ve-bob"], true);
    publish(&s, "ve-bob", |r| {
        phase_to(r, StartupPhase::Error);
        r.error = Some("code 5: account already logged in".into());
    });
    s.poll();
    let failed = row(&s, "ve-bob");
    assert_eq!(
        (failed.phase, failed.light(), failed.retrying),
        (Phase::LoginError, Light::Red, true),
        "auto-login still wants it: the host retries after its backoff"
    );
    assert!(failed.has_failure());
    let counts = s.fleet_view().counts();
    assert_eq!((counts.queued, counts.failed), (1, 1), "{counts:?}");

    // A public world preference error withdraws the login itself.
    s.play()
        .unwrap()
        .arm("ve-bob")
        .unwrap()
        .hold_login_on_error_for_test();
    s.poll();
    let held = row(&s, "ve-bob");
    assert_eq!((held.phase, held.retrying), (Phase::LoginError, false));
    let counts = s.fleet_view().counts();
    assert_eq!((counts.queued, counts.failed), (0, 1), "{counts:?}");

    // An explicit logout outranks the stale error, which stays as history.
    s.logout("ve-bob");
    s.poll();
    assert_eq!(row(&s, "ve-bob").phase, Phase::LoggedOut);
    assert!(row(&s, "ve-bob").error.is_some());
    assert_eq!(row(&s, "ve-bob").light(), Light::Grey);
    assert_eq!(s.fleet_view().counts().queued, 0);
    s.login("ve-bob", &mut HeadlessSurface::new());
    s.poll();
    let wanted = row(&s, "ve-bob");
    assert_eq!(
        (wanted.phase, wanted.retrying),
        (Phase::LoginError, true),
        "Log in wants it again: the next attempt is a retry"
    );
    assert_eq!(s.fleet_view().counts().queued, 1);

    // The host clears its error when the retry handshake starts.
    publish(&s, "ve-bob", |r| {
        phase_to(r, StartupPhase::Connecting);
        r.error = None;
    });
    s.poll();
    let retrying = row(&s, "ve-bob");
    assert_eq!(retrying.phase, Phase::Connecting);
    assert_eq!(
        retrying.error.as_deref(),
        Some("code 5: account already logged in"),
        "the last error stays visible through the retry"
    );
    assert_eq!(
        retrying.light(),
        Light::Grey,
        "history, not a current error"
    );
    assert!(!retrying.has_failure());

    publish(&s, "ve-bob", |r| {
        phase_to(r, StartupPhase::Queueing);
        (r.queue_position, r.queue_total) = (1, 2);
    });
    s.poll();
    assert_eq!(row(&s, "ve-bob").phase, Phase::Queued);
    assert!(row(&s, "ve-bob").error.is_some(), "still not recovered");

    publish(&s, "ve-bob", |r| {
        phase_to(r, StartupPhase::Ready);
        (r.queue_position, r.queue_total) = (-1, -1);
        (r.ingame, r.connected, r.scene_state) = (true, true, 2);
    });
    s.poll();
    assert_eq!(row(&s, "ve-bob").error, None, "recovery clears it");

    // A new worker lifetime starts clean.
    publish(&s, "ve-bob", |r| {
        phase_to(r, StartupPhase::Error);
        (r.ingame, r.connected) = (false, false);
        r.error = Some("code 3: invalid username or password".into());
    });
    s.poll();
    s.play_mut()
        .unwrap()
        .attach_arm("ve-bob", SlotArm::new(1, true));
    publish(&s, "ve-bob", |r| {
        phase_to(r, StartupPhase::Preparing);
        r.error = None;
    });
    s.poll();
    assert_eq!(row(&s, "ve-bob").phase, Phase::Preparing);
    assert_eq!(row(&s, "ve-bob").error, None);
}

#[test]
fn a_terminal_worker_is_failed_and_an_ended_one_offline() {
    let mut s = fleet("terminal", &["vf-dave", "vf-erin"], true);
    publish(&s, "vf-dave", |r| {
        phase_to(r, StartupPhase::Error);
        r.worker_terminal = Some(host_play::WorkerTerminal::Panicked);
        r.error = Some("slot worker panicked: synthetic".into());
    });
    // erin's worker ended cleanly; its last row is history.
    publish(&s, "vf-erin", |r| {
        phase_to(r, StartupPhase::Ready);
        (r.ingame, r.connected) = (true, true);
    });
    s.play_mut().unwrap().begin_stop_slot("vf-erin");
    s.poll();
    let dave = row(&s, "vf-dave");
    assert_eq!((dave.phase, dave.brief.as_str()), (Phase::Failed, "failed"));
    assert!(dave.has_failure());
    assert_eq!(row(&s, "vf-erin").phase, Phase::Offline);
    s.select("vf-dave");
    s.poll();
    assert_eq!(detail(&s).state, "failed: slot worker panicked: synthetic");
}

/// A failed operation stays on its slot's row after the bounded operation
/// history has rolled past it, until a newer operation on that slot.
#[test]
fn an_operation_failure_outlives_the_operation_history() {
    let mut s = fleet("op-retained", &["vo-frank"], true);
    s.fleet_mut().add("vo-ghost");
    let mut surface = HeadlessSurface::new();
    let failed = s.login("vo-ghost", &mut surface);
    s.poll();
    let ghost = row(&s, "vo-ghost");
    assert_eq!(
        ghost.last_op,
        Some(OpBrief {
            id: failed,
            action: ActionKind::Login,
            outcome: Outcome::Failed("no profile vo-ghost".into()),
        })
    );
    assert!(ghost.has_failure());

    for _ in 0..100 {
        s.load("vo-frank", &mut surface);
    }
    s.poll();
    assert!(s.operation(failed).is_none(), "the history rolled");
    assert_eq!(
        row(&s, "vo-ghost").last_op.as_ref().map(|op| op.id),
        Some(failed),
        "the row keeps it"
    );
    assert_eq!(s.fleet_view().counts().failed, 1);

    let later = s.logout("vo-ghost");
    s.poll();
    assert_eq!(
        row(&s, "vo-ghost").last_op.as_ref().map(|op| op.id),
        Some(later)
    );
    assert!(!row(&s, "vo-ghost").has_failure());
}

#[test]
fn operation_outcomes_reach_the_slot_log_with_their_id() {
    let mut s = fleet("op-log", &["vl-gina"], false);
    let mut surface = HeadlessSurface::new();
    let login = s.login("vl-gina", &mut surface);
    s.poll();
    publish(&s, "vl-gina", |r| {
        phase_to(r, StartupPhase::Ready);
        (r.ingame, r.connected, r.scene_state) = (true, true, 2);
    });
    s.poll();
    let mut view = LogView::new(LogScope::Slot("vl-gina".into()));
    crate::log::global().refresh(&mut view);
    let lines: Vec<&str> = view.rows().iter().map(|e| &*e.message).collect();
    let accepted = format!("op#{} Log in accepted", login.0);
    let completed = format!("op#{} Log in completed", login.0);
    let at = |text: &str| lines.iter().position(|line| *line == text);
    assert!(
        at(&accepted).is_some() && at(&accepted) < at(&completed),
        "{lines:?}"
    );
}

/// A poll that sees no change a front end shows rebuilds nothing and keeps
/// every generation, however much else the host published.
#[test]
fn an_unchanged_fleet_is_not_rederived() {
    let names: Vec<String> = (0..30).map(|i| format!("vg-{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut s = fleet("generation", &refs, true);
    s.select("vg-00");
    for name in &names {
        publish(&s, name, |r| {
            phase_to(r, StartupPhase::Ready);
            (r.ingame, r.connected, r.scene_state) = (true, true, 2);
        });
    }
    s.poll();
    let (generation, rows_generation) = (
        s.fleet_view().generation(),
        s.fleet_view().rows_generation(),
    );
    let rebuilt = s.rows_rebuilt();

    for tick in 0..20u64 {
        for name in &names[1..] {
            publish(&s, name, |r| {
                r.bytes_in += 100 * tick;
                r.runenergy = tick as i32;
                r.tile_x = 3200 + tick as i32;
                r.chat_head = format!("tick {tick}");
            });
        }
        s.poll();
    }
    assert_eq!(s.fleet_view().generation(), generation);
    assert_eq!(s.rows_rebuilt(), rebuilt, "no row re-derived");

    publish(&s, "vg-00", |r| r.tile_x = 3300);
    s.poll();
    assert_eq!(
        s.fleet_view().generation(),
        generation + 1,
        "the selected tile is shown"
    );
    assert_eq!(s.fleet_view().rows_generation(), rows_generation);
    assert_eq!(s.rows_rebuilt(), rebuilt);

    publish(&s, "vg-07", |r| {
        phase_to(r, StartupPhase::Queueing);
        (r.ingame, r.connected) = (false, false);
    });
    s.poll();
    assert_eq!(s.fleet_view().rows_generation(), rows_generation + 1);
    assert_eq!(s.rows_rebuilt(), rebuilt + 1, "only the changed row");
    assert_eq!(row(&s, "vg-07").phase, Phase::Waiting);
}

/// Per slot the projection keeps one row with capped text and one newest
/// operation, and only the selected slot has a detail.
#[test]
fn a_rows_memory_is_bounded_whatever_the_slot_publishes() {
    let mut s = fleet("bounded", &["vb-hana", "vb-ivan"], true);
    let mut surface = HeadlessSurface::new();
    let long = "é".repeat(4_000);
    publish(&s, "vb-hana", |r| {
        phase_to(r, StartupPhase::Error);
        r.error = Some(long.clone());
    });
    for _ in 0..300 {
        s.logout("vb-hana");
        s.login("vb-hana", &mut surface);
    }
    s.poll();
    let hana = row(&s, "vb-hana");
    let error = hana.error.as_deref().unwrap();
    assert!(
        error.len() <= REASON_CAP && error.ends_with('…'),
        "{}",
        error.len()
    );
    assert!(long.starts_with(error.trim_end_matches('…')));
    assert_eq!(
        hana.last_op.as_ref().map(|op| op.action),
        Some(ActionKind::Login),
        "one newest operation, not a history"
    );
    // Re-publishing the same long error is not a change.
    let generation = s.fleet_view().generation();
    s.poll();
    assert_eq!(s.fleet_view().generation(), generation);

    assert!(s.fleet_view().detail().is_none(), "nothing selected");
    s.remove("vb-ivan", Instant::now(), &mut surface);
    s.poll();
    let names: Vec<&str> = s
        .fleet_view()
        .rows()
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["vb-hana"], "a removed member's row is dropped");
}

#[test]
fn the_detail_follows_the_selection_to_a_slot_outside_the_fleet() {
    let mut s = fleet("detail", &["vd-jo"], true);
    s.select("vd-jo");
    publish(&s, "vd-jo", |r| {
        phase_to(r, StartupPhase::Ready);
        (r.ingame, r.connected) = (true, true);
        r.random = host_play::RandomStatus {
            kind: Some(api::RandomKind::Lamp),
            name: Some("genie".into()),
            hold: true,
            toggle: false,
            ..host_play::RandomStatus::default()
        };
        r.welcome_hold = true;
    });
    s.poll();
    let jo = detail(&s);
    assert_eq!(
        jo.row,
        *row(&s, "vd-jo"),
        "a member's detail shares its row"
    );
    assert_eq!(jo.random.as_deref(), Some("lamp: genie (hold) (off)"));
    assert_eq!(jo.welcome.as_deref(), Some("holding"));

    // A running slot that is not a member (single-bot mode).
    s.play_mut()
        .unwrap()
        .attach_arm("vd-kim", SlotArm::new(9, true));
    publish(&s, "vd-kim", |r| phase_to(r, StartupPhase::Connecting));
    s.select("vd-kim");
    s.poll();
    assert_eq!(detail(&s).row.name, "vd-kim");
    assert_eq!(detail(&s).row.phase, Phase::Connecting);
    assert_eq!(s.fleet_view().rows().len(), 1, "not added to the fleet");
    assert_eq!(
        s.fleet_view().row("vd-kim").map(|r| r.phase),
        Some(Phase::Connecting),
        "reachable by name while selected"
    );
}

static PROBES: AtomicUsize = AtomicUsize::new(0);

fn counting_probe() -> ProcessProbe {
    let n = PROBES.fetch_add(1, Ordering::SeqCst) + 1;
    ProcessProbe::Sampled {
        cpu_seconds: n as f64 * 0.25,
        resident: Some(64 << 20),
        peak: 80 << 20,
    }
}

/// Keep polling at `at` (no new sample is due) until the meter shows what
/// `done` waits for from its probe thread, or fail after a few seconds.
fn settle_meter(s: &mut OperatorSession<()>, at: Instant, done: impl Fn(&ResourceView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(s.resources()) {
        assert!(Instant::now() < deadline, "{:?}", s.resources());
        std::thread::sleep(Duration::from_millis(1));
        s.poll_at(at);
    }
}

/// One process sample per second for the whole fleet: polling many
/// members many times never samples per row, per bot or per frame.
#[test]
fn the_meter_samples_once_a_second_for_the_whole_fleet() {
    let names: Vec<String> = (0..40).map(|i| format!("vm-{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut s = fleet("meter", &refs, true);
    s.set_resource_probe(counting_probe);
    s.select("vm-00");
    let start = Instant::now();
    let last_frame = start + Duration::from_millis(59 * 15);
    for frame in 0..60u64 {
        s.poll_at(start + Duration::from_millis(frame * 15));
    }
    settle_meter(&mut s, last_frame, |view| view.ram != Metric::Measuring);
    assert_eq!(PROBES.load(Ordering::SeqCst), 1, "one sample in 885 ms");
    let first = s.resources().clone();
    assert_eq!((first.bots, first.background), (40, 39));
    assert_eq!(first.cpu, Metric::Measuring, "a rate needs two samples");
    assert_eq!(
        first.ram,
        Metric::Available("64.0 MB process, peak 80.0 MB".into())
    );

    let generation = s.resource_generation();
    let second = start + Duration::from_secs(1);
    s.poll_at(second);
    settle_meter(&mut s, second, |view| view.cpu != Metric::Measuring);
    assert_eq!(PROBES.load(Ordering::SeqCst), 2);
    assert!(matches!(s.resources().cpu, Metric::Available(_)));
    assert!(s.resource_generation() > generation);
}

/// World / login is the login phase, not the script brief: a logged-in idle
/// bot is `ready`, not `idle`.
#[test]
fn world_login_uses_the_login_phase_not_script_idle() {
    let local = FleetRow::fixture("alice", Phase::Ready, None, None);
    assert_eq!(local.brief, "idle");
    let mut out = String::new();
    local.write_world_login(&mut out);
    assert_eq!(out, "ready");
    let public = FleetRow::fixture("bob", Phase::Ready, Some(301), None);
    out.clear();
    public.write_world_login(&mut out);
    assert_eq!(out, "w301 ready");
    let logging = FleetRow::fixture("carol", Phase::Connecting, None, None);
    out.clear();
    logging.write_world_login(&mut out);
    assert_eq!(out, "logging in");
}
