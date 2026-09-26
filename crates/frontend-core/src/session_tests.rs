//! Lifecycle proofs through the real host seam: a real [`host_play::Play`]
//! (no server contact), real [`SlotArm`] control flags, real script slots.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use host_play::{InstancePermit, Play, PlayOptions, SlotArm, SlotStatus, StartupPhase};
use vault::{Profile, ProfileSettings, Vault};

use super::*;
use crate::surface::{SlotAttach, SlotSurface};

/// Records lifecycle callbacks; IO values count attaches so reuse is visible.
#[derive(Default)]
struct Recorder {
    attached: u32,
    resets: Vec<String>,
    released: Vec<String>,
    rasters: Vec<vault::RasterMode>,
}

impl SlotSurface for Recorder {
    type Io = u32;

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut Profile,
        retained: Option<u32>,
    ) -> SlotAttach<u32> {
        self.rasters.push(profile.settings.raster);
        let io = retained.unwrap_or_else(|| {
            self.attached += 1;
            self.attached
        });
        SlotAttach {
            io,
            input: None,
            mailbox: None,
        }
    }

    fn lifetime_reset(&mut self, name: &str) {
        self.resets.push(name.to_string());
    }

    fn released(&mut self, name: &str) {
        self.released.push(name.to_string());
    }
}

fn empty_play() -> Play {
    host_play::run_with_io(
        &PlayOptions {
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

fn vault_with(test: &str, profiles: &[(&str, i32, bool)]) -> Vault {
    let dir = std::env::temp_dir().join(format!(
        "274bot-frontend-core-{}-{test}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "bot").unwrap();
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

fn arm(session: &OperatorSession<u32>, name: &str) -> Arc<SlotArm> {
    session.play().unwrap().arm(name).expect("live arm")
}

fn push_status(session: &OperatorSession<u32>, row: SlotStatus) {
    session.play().unwrap().statuses.lock().unwrap().push(row);
}

fn row(name: &str) -> SlotStatus {
    SlotStatus {
        username: name.into(),
        ..SlotStatus::default()
    }
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
    assert_eq!(s.members(), ["manual".to_string(), "auto".to_string()]);
    assert_eq!(s.selected(), None, "Load is not selection");

    let login = s.login("manual", &mut surface);
    assert!(arm(&s, "manual").wants_login());
    assert!(!arm(&s, "manual").auto_login.load(Ordering::Relaxed));
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

fn looping_bot(dir: &std::path::Path) -> (String, script::LoadShape) {
    let path = dir.join("looping.ts");
    std::fs::write(
        &path,
        "export default class T extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    let mut library = script::JsLibrary::new(dir.join("js-scripts.json"));
    let card = library.load(&path).unwrap();
    (card.js, card.shape)
}

fn settle_start(s: &mut OperatorSession<u32>) -> Vec<StartSettled> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.poll();
        let settled = s.take_settled_starts();
        if !settled.is_empty() || Instant::now() > deadline {
            return settled;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

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
            ScriptStart::Load {
                js,
                shape,
                bag: None,
                siblings: Vec::new(),
            },
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
        s.operation(resume).unwrap().action,
        ActionKind::ScriptResume
    );

    let stop = s.stop_script("alice");
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.operation(stop).unwrap().outcome("alice") == Some(&Outcome::Pending)
        && Instant::now() < deadline
    {
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        s.operation(stop).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert_eq!(
        s.play().unwrap().script_state("alice"),
        script::RunState::Idle
    );
}

#[test]
fn a_start_refused_by_the_host_is_an_error_not_an_operation() {
    let mut s = session("start-refused", &[]);
    let refused = s.start_script(
        "ghost",
        ScriptStart::Load {
            js: String::new(),
            shape: script::LoadShape::Reject,
            bag: None,
            siblings: Vec::new(),
        },
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
            .map(|t| t.transition.clone())
            .collect::<Vec<_>>(),
        [Transition::Ingame, Transition::Scene(2)]
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
