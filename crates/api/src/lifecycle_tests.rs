use super::*;
use crate::identity::{IdentityError, IdentityKind, SlotKey};
use client::client::client::SessionExitReason;
use std::num::NonZeroU64;

fn armed() -> IntentMailbox {
    IntentMailbox::new(true, WorldChoice::Automatic)
}

fn parked() -> IntentMailbox {
    IntentMailbox::new(false, WorldChoice::Automatic)
}

fn online(mailbox: &mut IntentMailbox) -> IntentCommand {
    let command = mailbox.login_command(false).unwrap();
    mailbox.observe(LifecycleFact::LoginSucceeded);
    assert_eq!(mailbox.acknowledge_login(command), Acknowledgement::Applied);
    command
}

fn session(epoch: u64) -> SessionKey {
    SessionKey::new(SlotKey::from_incarnation(5).unwrap(), epoch)
}

#[test]
fn every_row_has_a_meaning_and_its_cancellation_column_matches_the_reducer() {
    for intent in Intent::ALL {
        let meaning = intent.meaning();
        // Auto-derived attempt: spawn with auto-login on, read a login command.
        let mut auto = armed();
        let command = auto.login_command(false).unwrap();
        let world = auto.snapshot().world_generation();
        assert!(auto.attempt_is_current(command, world));
        let receipt = auto.post(intent);
        let cancels_auto = receipt.cancelled_attempt();
        // Explicit attempt: spawn parked, arm an explicit login.
        let mut explicit = parked();
        explicit.post(Intent::Login);
        let command = explicit.login_command(false).unwrap();
        let receipt = explicit.post(intent);
        let cancels_explicit = receipt.cancelled_attempt();
        match meaning.attempt {
            AttemptCancellation::Always => {
                assert!(cancels_auto && cancels_explicit, "{intent:?} must cancel");
                assert!(
                    !explicit.attempt_is_current(command, 0),
                    "{intent:?}: the in-flight attempt is no longer current"
                );
            }
            AttemptCancellation::Never => {
                assert!(
                    !cancels_auto && !cancels_explicit,
                    "{intent:?} must not cancel"
                );
            }
            AttemptCancellation::IfAutoDerived => {
                assert!(cancels_auto, "{intent:?} cancels an auto-derived attempt");
                assert!(!cancels_explicit, "{intent:?} keeps an explicit attempt");
            }
        }
    }
    for fact in LifecycleFact::ALL {
        let mut auto = armed();
        auto.login_command(false).unwrap();
        let receipt = auto.observe(fact);
        assert_eq!(
            receipt.cancelled_attempt(),
            fact.meaning().attempt == AttemptCancellation::Always,
            "{fact:?}"
        );
    }
}

#[test]
fn only_stop_and_removal_terminate_a_run_and_logout_holds_it() {
    let terminating: Vec<Intent> = Intent::ALL
        .into_iter()
        .filter(|intent| intent.meaning().run == RunEffect::Terminate)
        .collect();
    assert_eq!(
        terminating,
        [
            Intent::StopScript,
            Intent::RemoveSlot,
            Intent::ProcessShutdown
        ]
    );
    assert_eq!(Intent::Logout.meaning().run, RunEffect::HoldWork);
    assert_eq!(Intent::Login.meaning().run, RunEffect::ResumeHeld);
    assert_eq!(
        Intent::WatchdogRestart.meaning().run,
        RunEffect::RecreateExecution
    );
    assert_eq!(
        Intent::WatchdogRestart.meaning().session,
        SessionEffect::None
    );
    assert_eq!(Intent::StopScript.meaning().session, SessionEffect::None);
    assert_eq!(
        Intent::WorldPreference(WorldChoice::World(3))
            .meaning()
            .session,
        SessionEffect::NextHandshakeOnly
    );
    for fact in LifecycleFact::ALL {
        assert_ne!(
            fact.meaning().run,
            RunEffect::Terminate,
            "{fact:?}: no fact ends a run"
        );
    }
}

#[test]
fn every_command_advances_the_generation_but_only_cancelling_rows_move_the_fence() {
    let mut mailbox = armed();
    let start = mailbox.snapshot();
    let login = mailbox.post(Intent::Login);
    assert!(login.generation() > start.generation());
    assert_eq!(mailbox.snapshot().cancellation(), start.cancellation());

    let logout = mailbox.post(Intent::Logout);
    assert!(logout.generation() > login.generation());
    assert!(mailbox.snapshot().cancellation() > start.cancellation());
    assert!(logout.cancelled_attempt());

    let before = mailbox.snapshot();
    mailbox.post(Intent::WorldPreference(WorldChoice::World(2)));
    assert_eq!(
        mailbox.snapshot().generation(),
        before.generation(),
        "a world save while nothing is held is not a command"
    );
    assert_eq!(
        mailbox.snapshot().world_generation(),
        before.world_generation() + 1
    );
    mailbox.post(Intent::WorldPreference(WorldChoice::World(2)));
    assert_eq!(
        mailbox.snapshot().world_generation(),
        before.world_generation() + 1,
        "saving the same world again moves nothing"
    );
}

#[test]
fn explicit_login_arms_once_and_lifts_every_latch_and_hold() {
    let mut mailbox = parked();
    assert!(!mailbox.snapshot().login_wanted());
    assert_eq!(mailbox.snapshot().park_reason(), Some(ParkReason::NoIntent));
    mailbox.post(Intent::Logout);
    mailbox.observe(LifecycleFact::WorldPreferenceFailed);
    assert_eq!(
        mailbox.snapshot().park_reason(),
        Some(ParkReason::OperatorLogout)
    );

    mailbox.post(Intent::Login);
    let snapshot = mailbox.snapshot();
    assert_eq!(snapshot.login(), LoginIntent::Once);
    assert_eq!(snapshot.latch(), None);
    assert!(!snapshot.error_hold());
    assert!(!snapshot.logout_wanted());
    assert!(snapshot.login_wanted());
    assert_eq!(snapshot.park_reason(), None);
    assert!(
        !snapshot.auto_login(),
        "an explicit login changes no preference"
    );

    let command = online(&mut mailbox);
    assert_eq!(
        mailbox.snapshot().login(),
        LoginIntent::Withdrawn,
        "a one-shot is consumed by its acknowledgement when auto-login is off"
    );
    assert_eq!(
        mailbox.acknowledge_login(command),
        Acknowledgement::Applied,
        "an agreeing acknowledgement at the same generation is harmless"
    );
    assert_eq!(mailbox.snapshot().login(), LoginIntent::Withdrawn);
}

#[test]
fn agreeing_refreshes_do_not_create_a_second_one_shot() {
    let mut mailbox = armed();
    assert_eq!(mailbox.snapshot().login(), LoginIntent::Auto);
    let first = mailbox.login_command(false).unwrap();
    mailbox.post(Intent::AutoLogin(true));
    let second = mailbox.login_command(false).unwrap();
    assert!(second.generation() > first.generation());
    assert_eq!(
        second.cancellation(),
        first.cancellation(),
        "re-affirming auto-login does not cancel the queued attempt"
    );
    assert!(mailbox.attempt_is_current(first, 0));
    mailbox.observe(LifecycleFact::LoginSucceeded);
    assert_eq!(
        mailbox.acknowledge_login(first),
        Acknowledgement::Applied,
        "the older command still asks for the same outcome"
    );
    assert_eq!(
        mailbox.snapshot().login(),
        LoginIntent::Auto,
        "the want survives as the preference, not as another one-shot"
    );
}

#[test]
fn a_stale_acknowledgement_cannot_settle_a_newer_opposite_intent() {
    let mut mailbox = armed();
    let login = mailbox.login_command(false).unwrap();
    mailbox.post(Intent::Logout);
    assert_eq!(
        mailbox.acknowledge_login(login),
        Acknowledgement::Stale,
        "a login finished under an older generation cannot clear a Logout"
    );
    assert_eq!(mailbox.snapshot().latch(), Some(LoginLatch::OperatorLogout));
    assert!(mailbox.snapshot().logout_wanted());

    let mut mailbox = armed();
    online(&mut mailbox);
    mailbox.post(Intent::Logout);
    let logout = mailbox.logout_command(true).unwrap();
    mailbox.post(Intent::Login);
    assert_eq!(
        mailbox.acknowledge_logout(logout),
        Acknowledgement::Stale,
        "a Login that cancelled an ineffective Logout keeps its want"
    );
    assert_eq!(mailbox.snapshot().login(), LoginIntent::Once);
    assert_eq!(mailbox.snapshot().latch(), None);
}

#[test]
fn clean_logout_latches_until_an_explicit_rearm_and_keeps_run_work_only_for_drops() {
    let mut mailbox = armed();
    mailbox.post(Intent::ScriptNeedsSession(true));
    online(&mut mailbox);
    assert!(mailbox.keeps_run_work_after_exit());
    assert!(mailbox.relogs_after_drop());

    mailbox.post(Intent::Logout);
    assert!(
        !mailbox.relogs_after_drop(),
        "a pending logout never relogs"
    );
    assert!(
        !mailbox.keeps_run_work_after_exit(),
        "deliberate logout ends session work"
    );
    let logout = mailbox.logout_command(true).unwrap();
    assert_eq!(mailbox.acknowledge_logout(logout), Acknowledgement::Applied);
    mailbox.observe(LifecycleFact::SessionExited {
        clean: true,
        resumable: false,
    });
    assert!(
        !mailbox.reconnects(),
        "a clean logout always leads to a fresh login"
    );
    assert_eq!(mailbox.snapshot().latch(), Some(LoginLatch::OperatorLogout));
    assert_eq!(
        mailbox.snapshot().park_reason(),
        Some(ParkReason::OperatorLogout)
    );
    assert!(
        !mailbox.snapshot().login_wanted(),
        "the active script's need is latched off too"
    );
    mailbox.post(Intent::AutoLogin(true));
    assert!(
        !mailbox.snapshot().login_wanted(),
        "a preference toggle does not lift the latch"
    );
    mailbox.post(Intent::WorldPreference(WorldChoice::World(7)));
    assert!(
        !mailbox.snapshot().login_wanted(),
        "neither does a world save"
    );
    assert!(mailbox.login_command(false).is_none());

    mailbox.post(Intent::Login);
    assert!(mailbox.snapshot().login_wanted());
    assert!(mailbox.login_command(false).is_some());
}

#[test]
fn an_offline_logout_owes_one_held_work_reset_and_a_connected_one_none() {
    let mut mailbox = armed();
    mailbox.post(Intent::Logout);
    assert!(
        mailbox.take_work_reset(),
        "offline Logout still ends held work once"
    );
    assert!(
        !mailbox.take_work_reset(),
        "consumed at the next boundary only once"
    );

    let mut mailbox = armed();
    online(&mut mailbox);
    mailbox.post(Intent::Logout);
    assert!(
        !mailbox.take_work_reset(),
        "a connected exit reads OperatorLogout from the latch instead"
    );

    let mut mailbox = armed();
    mailbox.post(Intent::Logout);
    mailbox.post(Intent::Login);
    assert!(
        mailbox.take_work_reset(),
        "a later Login does not clear an older unconsumed offline reset"
    );
}

#[test]
fn unexpected_drop_reconnects_only_while_wanted_and_the_repeat_guard_parks_it() {
    let mut mailbox = parked();
    mailbox.post(Intent::ScriptNeedsSession(true));
    assert!(
        mailbox.snapshot().login_wanted(),
        "an active script wants the login whatever the preference says"
    );
    online(&mut mailbox);
    assert!(
        mailbox.reconnects(),
        "after a grant the next handshake reconnects"
    );
    mailbox.observe(LifecycleFact::SessionExited {
        clean: false,
        resumable: true,
    });
    assert!(mailbox.reconnects());
    assert!(mailbox.relogs_after_drop());
    assert!(mailbox.keeps_run_work_after_exit());
    assert!(
        !mailbox.login_applies_memory(),
        "a dirty reconnect preserves the server session's memory mode"
    );

    let receipt = mailbox.observe(LifecycleFact::RepeatGuardTripped {
        count: 3,
        window_seconds: 600,
    });
    assert!(receipt.cancelled_attempt());
    assert_eq!(
        mailbox.snapshot().latch(),
        Some(LoginLatch::RepeatGuard {
            count: 3,
            window_seconds: 600
        })
    );
    assert_eq!(
        mailbox.snapshot().park_reason(),
        Some(ParkReason::RepeatGuard)
    );
    assert!(!mailbox.relogs_after_drop());
    assert!(
        mailbox.keeps_run_work_after_exit(),
        "the repeat guard parks the login but keeps recoverable run work"
    );
    let again = mailbox.observe(LifecycleFact::RepeatGuardTripped {
        count: 3,
        window_seconds: 600,
    });
    assert!(
        !again.cancelled_attempt(),
        "an already latched slot records nothing new"
    );

    mailbox.post(Intent::Login);
    assert_eq!(
        mailbox.snapshot().latch(),
        None,
        "explicit Login clears the guard"
    );
    assert!(mailbox.relogs_after_drop());

    let mut stopped = parked();
    stopped.post(Intent::ScriptNeedsSession(true));
    online(&mut stopped);
    stopped.post(Intent::ScriptNeedsSession(false));
    assert!(
        !stopped.relogs_after_drop(),
        "a stopped script stops the relog"
    );
    assert!(!stopped.keeps_run_work_after_exit());
}

#[test]
fn a_world_error_holds_the_login_until_the_operator_changes_something() {
    let mut mailbox = armed();
    mailbox.post(Intent::ScriptNeedsSession(true));
    let command = mailbox.login_command(false).unwrap();
    let receipt = mailbox.observe(LifecycleFact::WorldPreferenceFailed);
    assert!(receipt.cancelled_attempt());
    assert!(!mailbox.attempt_is_current(command, 0));
    let snapshot = mailbox.snapshot();
    assert!(snapshot.error_hold());
    assert!(
        !snapshot.login_wanted(),
        "the hold suppresses the script's need as well"
    );
    assert_eq!(snapshot.park_reason(), Some(ParkReason::WorldError));

    mailbox.post(Intent::WorldPreference(WorldChoice::World(12)));
    let snapshot = mailbox.snapshot();
    assert!(!snapshot.error_hold(), "a world change lifts the hold");
    assert_eq!(
        snapshot.login(),
        LoginIntent::Auto,
        "and re-derives the want from the still-enabled preference"
    );

    let mut toggled = armed();
    toggled.observe(LifecycleFact::WorldPreferenceFailed);
    toggled.post(Intent::AutoLogin(false));
    assert!(!toggled.snapshot().error_hold());
    assert_eq!(toggled.snapshot().login(), LoginIntent::Withdrawn);
    toggled.post(Intent::AutoLogin(true));
    assert_eq!(toggled.snapshot().login(), LoginIntent::Auto);
}

#[test]
fn a_world_save_never_cancels_but_makes_the_attempts_outcome_stale() {
    let mut mailbox = armed();
    let command = mailbox.login_command(false).unwrap();
    let world = mailbox.snapshot().world_generation();
    let receipt = mailbox.post(Intent::WorldPreference(WorldChoice::World(9)));
    assert!(!receipt.cancelled_attempt());
    assert!(
        !mailbox.attempt_is_current(command, world),
        "a stale attempt cannot install backoff on the new world"
    );
    let fresh = mailbox.login_command(false).unwrap();
    assert!(mailbox.attempt_is_current(fresh, mailbox.snapshot().world_generation()));
    assert_eq!(mailbox.snapshot().world(), WorldChoice::World(9));
}

#[test]
fn withdraw_and_auto_off_cancel_only_what_they_own() {
    let mut mailbox = armed();
    let auto = mailbox.login_command(false).unwrap();
    let receipt = mailbox.post(Intent::AutoLogin(false));
    assert!(receipt.cancelled_attempt());
    assert!(!mailbox.attempt_is_current(auto, 0));
    assert!(!mailbox.snapshot().login_wanted());
    assert!(!mailbox.snapshot().auto_login());

    let mut explicit = parked();
    explicit.post(Intent::Login);
    let command = explicit.login_command(false).unwrap();
    assert!(!explicit.post(Intent::AutoLogin(false)).cancelled_attempt());
    assert!(
        explicit.attempt_is_current(command, 0),
        "an explicit login outlives the toggle"
    );
    assert!(explicit.post(Intent::WithdrawLogin).cancelled_attempt());
    assert!(!explicit.attempt_is_current(command, 0));
    assert!(!explicit.snapshot().login_wanted());
}

#[test]
fn relog_now_is_a_turnover_not_a_logout_latch() {
    let mut mailbox = armed();
    online(&mut mailbox);
    let receipt = mailbox.post(Intent::RelogNow);
    assert!(!receipt.cancelled_attempt());
    let logout = mailbox.logout_command(true).unwrap();
    assert_eq!(mailbox.acknowledge_logout(logout), Acknowledgement::Applied);
    assert_eq!(mailbox.snapshot().latch(), None, "no park latch");
    assert_eq!(
        mailbox.snapshot().login(),
        LoginIntent::Once,
        "the login half stays armed"
    );
    mailbox.observe(LifecycleFact::SessionExited {
        clean: true,
        resumable: false,
    });
    assert!(!mailbox.reconnects(), "the turnover's login is fresh");
    assert!(mailbox.login_command(false).is_some());
    assert!(
        mailbox.login_applies_memory(),
        "a fresh handshake applies the queued memory mode"
    );

    let mut plain = armed();
    online(&mut plain);
    plain.post(Intent::Logout);
    let logout = plain.logout_command(true).unwrap();
    plain.acknowledge_logout(logout);
    assert_eq!(plain.snapshot().latch(), Some(LoginLatch::OperatorLogout));
    assert!(plain.login_command(false).is_none());
}

#[test]
fn switch_world_cancels_or_exits_then_logs_in_to_the_requested_world() {
    let mut queued = armed();
    let command = queued.login_command(false).unwrap();
    let receipt = queued.post(Intent::SwitchWorld(4));
    assert!(receipt.cancelled_attempt());
    assert!(!queued.attempt_is_current(command, 0));
    assert_eq!(queued.snapshot().world(), WorldChoice::World(4));
    assert!(
        queued.login_command(false).is_some(),
        "a new login to the requested world"
    );
    assert!(
        queued.logout_command(true).is_none(),
        "nothing to log out of"
    );

    let mut live = armed();
    online(&mut live);
    live.post(Intent::SwitchWorld(5));
    let logout = live.logout_command(true).unwrap();
    assert_eq!(live.acknowledge_logout(logout), Acknowledgement::Applied);
    assert_eq!(live.snapshot().latch(), None);
    assert_eq!(live.snapshot().login(), LoginIntent::Once);
    assert_eq!(live.snapshot().world(), WorldChoice::World(5));
}

#[test]
fn removal_withdraws_everything_and_cancels_the_attempt() {
    for intent in [Intent::RemoveSlot, Intent::ProcessShutdown] {
        let mut mailbox = armed();
        mailbox.post(Intent::ScriptNeedsSession(true));
        let command = mailbox.login_command(false).unwrap();
        let receipt = mailbox.post(intent);
        assert!(receipt.cancelled_attempt());
        assert!(!mailbox.attempt_is_current(command, 0));
        assert!(!mailbox.snapshot().login_wanted(), "{intent:?}");
        assert!(!mailbox.snapshot().script_needs_session());
    }
}

#[test]
fn memory_mode_queues_for_the_next_fresh_handshake_only() {
    let mut mailbox = armed();
    assert_eq!(mailbox.snapshot().memory(), None);
    let before = mailbox.snapshot().generation();
    mailbox.post(Intent::MemoryMode(MemoryMode::Low));
    assert_eq!(mailbox.snapshot().memory(), Some(MemoryMode::Low));
    assert_eq!(
        mailbox.snapshot().generation(),
        before,
        "not a login command"
    );
    assert!(mailbox.login_applies_memory(), "the first login is fresh");
    online(&mut mailbox);
    mailbox.observe(LifecycleFact::SessionExited {
        clean: false,
        resumable: true,
    });
    assert!(
        !mailbox.login_applies_memory(),
        "a reconnect keeps the applied mode"
    );
    mailbox.observe(LifecycleFact::SessionExited {
        clean: true,
        resumable: false,
    });
    assert!(mailbox.login_applies_memory());
}

#[test]
fn session_end_classification_is_clean_only_for_a_server_logout() {
    let clean = SessionExitReason::ServerLogout {
        local_idle_request_pending: false,
    };
    assert_eq!(
        SessionEndCause::classify(&clean),
        SessionEndCause::ServerLogout
    );
    assert!(SessionEndCause::classify(&clean).is_clean());
    for reason in [
        SessionExitReason::ReadEof,
        SessionExitReason::ConnectionLost,
        SessionExitReason::DeadServerDeadline {
            elapsed: Duration::from_secs(30),
        },
        SessionExitReason::WebSocketClose {
            code: Some(1006),
            reason: String::new(),
        },
        SessionExitReason::TlsError {
            message: String::new(),
        },
        SessionExitReason::ReadError {
            kind: std::io::ErrorKind::ConnectionReset,
            message: String::new(),
        },
        SessionExitReason::WriteError {
            kind: std::io::ErrorKind::BrokenPipe,
            message: String::new(),
        },
    ] {
        let cause = SessionEndCause::classify(&reason);
        assert!(!cause.is_clean(), "{reason:?} is an unexpected exit");
        assert_ne!(cause, SessionEndCause::ServerLogout);
    }
    assert!(SessionEndCause::Removed.is_clean());
}

#[test]
fn scripts_tick_only_between_scene_ready_and_session_ended_of_the_same_session() {
    let mut gate = SessionGate::new();
    assert_eq!(gate.ready(), None);
    let first = session(1);
    let second = session(2);

    gate.observe(first, &SessionEventOut::SceneReady);
    assert_eq!(gate.ready(), Some(first));

    gate.observe(
        second,
        &SessionEventOut::SessionEnded {
            cause: SessionEndCause::ConnectionLost,
            resumable: true,
        },
    );
    assert_eq!(
        gate.ready(),
        Some(first),
        "an end stamped with another session key is stale and changes nothing"
    );
    gate.observe(
        second,
        &SessionEventOut::Held {
            why: HoldReason::Guardian,
        },
    );
    assert_eq!(gate.ready(), Some(first), "a stale hold is ignored");

    gate.observe(
        first,
        &SessionEventOut::Held {
            why: HoldReason::Guardian,
        },
    );
    assert_eq!(gate.ready(), None);
    assert!(gate.held());
    gate.observe(
        first,
        &SessionEventOut::Held {
            why: HoldReason::WelcomeModal,
        },
    );
    gate.observe(first, &SessionEventOut::Released);
    assert_eq!(gate.ready(), None, "two holds need two releases");
    gate.observe(first, &SessionEventOut::Released);
    assert_eq!(gate.ready(), Some(first));
    gate.observe(first, &SessionEventOut::Released);
    assert_eq!(
        gate.ready(),
        Some(first),
        "an extra release does not go negative"
    );

    gate.observe(
        first,
        &SessionEventOut::SessionEnded {
            cause: SessionEndCause::ServerLogout,
            resumable: false,
        },
    );
    assert_eq!(gate.ready(), None);
    assert!(!gate.held());
    gate.observe(first, &SessionEventOut::Released);
    assert_eq!(
        gate.ready(),
        None,
        "a release after the end cannot resurrect the session"
    );

    gate.observe(
        second,
        &SessionEventOut::HandshakeOk {
            kind: HandshakeKind::Reconnect,
        },
    );
    assert_eq!(gate.ready(), None, "a handshake is not scene readiness");
    gate.observe(second, &SessionEventOut::SceneReady);
    assert_eq!(gate.ready(), Some(second));
    gate.observe(
        first,
        &SessionEventOut::SessionEnded {
            cause: SessionEndCause::ConnectionLost,
            resumable: true,
        },
    );
    assert_eq!(
        gate.ready(),
        Some(second),
        "a late end of the old session cannot close the new one"
    );
}

#[test]
fn session_held_is_not_pause_and_only_stop_ends_the_run() {
    let mut run = RunHold::idle();
    assert!(!run.may_act());
    run.apply(RunEffect::Start, None).unwrap();
    assert!(run.may_act());
    let generation = run.execution_generation();

    run.apply(RunEffect::HoldWork, Some(session(1))).unwrap();
    assert_eq!(run.phase(), RunPhase::Running, "Running stays Running");
    assert_eq!(
        run.session_held(),
        Some(SessionHeld {
            since: Some(session(1))
        })
    );
    assert!(!run.may_act());
    run.apply(RunEffect::ResumeHeld, None).unwrap();
    assert!(
        !run.may_act(),
        "an explicit Login resumes nothing before the next Ready"
    );
    run.apply(RunEffect::RevalidateUnderNewSession, Some(session(2)))
        .unwrap();
    assert!(run.may_act());

    run.apply(RunEffect::Pause, None).unwrap();
    assert_eq!(run.phase(), RunPhase::Paused);
    run.apply(RunEffect::HoldWork, Some(session(2))).unwrap();
    assert_eq!(
        run.phase(),
        RunPhase::Paused,
        "Paused stays Paused plus SessionHeld"
    );
    run.apply(RunEffect::RevalidateUnderNewSession, Some(session(3)))
        .unwrap();
    assert_eq!(
        run.phase(),
        RunPhase::Paused,
        "a new session does not lift an operator pause"
    );
    assert!(!run.may_act());
    run.apply(RunEffect::Resume, None).unwrap();
    assert!(run.may_act());

    run.apply(RunEffect::RecreateExecution, None).unwrap();
    assert_eq!(
        run.phase(),
        RunPhase::Running,
        "a watchdog restart keeps the run"
    );
    assert!(
        run.execution_generation() > generation,
        "with a new execution generation"
    );

    run.apply(RunEffect::Terminate, None).unwrap();
    assert_eq!(run.phase(), RunPhase::Idle);
    assert_eq!(run.session_held(), None);
    assert!(!run.may_act());
    run.apply(RunEffect::HoldWork, Some(session(3))).unwrap();
    assert_eq!(
        run.session_held(),
        None,
        "nothing to hold once the run ended"
    );

    let mut exhausted = RunHold::idle();
    exhausted.apply(RunEffect::Start, None).unwrap();
    exhausted.execution_generation = NonZeroU64::MAX;
    assert_eq!(
        exhausted.apply(RunEffect::RecreateExecution, None),
        Err(IdentityError::Exhausted(IdentityKind::ExecutionGeneration))
    );
    assert_eq!(
        exhausted.execution_generation(),
        NonZeroU64::MAX,
        "a refused recreate reuses nothing"
    );
}
