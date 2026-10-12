use super::*;
use crate::identity::{ProcessIncarnation, SlotKey};
use crate::process_condition::ProcessServiceCode;
use client::client::client::SessionExitReason;
use std::num::NonZeroU64;

fn armed() -> IntentMailbox {
    IntentMailbox::new(true, WorldChoice::Automatic)
}

fn parked() -> IntentMailbox {
    IntentMailbox::new(false, WorldChoice::Automatic)
}

/// A parked slot whose only login want is its script's need.
fn script_needed() -> IntentMailbox {
    let mut mailbox = parked();
    mailbox.post(Intent::ScriptNeedsSession(true)).unwrap();
    mailbox
}

fn online(mailbox: &mut IntentMailbox) -> IntentCommand {
    let command = mailbox.login_command(false).unwrap();
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    assert_eq!(mailbox.acknowledge_login(command), Acknowledgement::Applied);
    command
}

fn slot() -> SlotKey {
    SlotKey::from_incarnation(5).unwrap()
}

fn session(epoch: u64) -> SessionKey {
    SessionKey::new(slot(), epoch)
}

const DROP: LifecycleFact = LifecycleFact::SessionExited {
    clean: false,
    resumable: true,
};

const CLEAN_EXIT: LifecycleFact = LifecycleFact::SessionExited {
    clean: true,
    resumable: false,
};

#[test]
fn every_row_has_a_meaning_and_its_attempt_column_matches_the_reducer() {
    for intent in Intent::ALL {
        let meaning = intent.meaning();
        // Auto-derived attempt: spawn with auto-login on, read a login command.
        let mut auto = armed();
        let command = auto.login_command(false).unwrap();
        let world = auto.snapshot().world_generation();
        assert!(auto.attempt_is_current(command, world));
        let cancels_auto = auto.post(intent).unwrap().invalidated_attempt();
        // Explicit attempt: spawn parked, arm an explicit login.
        let mut explicit = parked();
        explicit.post(Intent::Login).unwrap();
        let command = explicit.login_command(false).unwrap();
        let cancels_explicit = explicit.post(intent).unwrap().invalidated_attempt();
        // Script-derived attempt: parked, wanted only by the script's need.
        let mut script = script_needed();
        let script_command = script.login_command(false).unwrap();
        let cancels_script = script.post(intent).unwrap().invalidated_attempt();
        let expected = match meaning.attempt() {
            AttemptCancellation::Always => (true, true, true),
            AttemptCancellation::Never => (false, false, false),
            AttemptCancellation::IfAutoDerived => (true, false, false),
            AttemptCancellation::IfScriptNeedEnds => (false, false, true),
            AttemptCancellation::Settled => {
                unreachable!("{intent:?}: no intent settles an attempt")
            }
        };
        assert_eq!(
            (cancels_auto, cancels_explicit, cancels_script),
            expected,
            "{intent:?} (auto-derived, explicit, script-derived)"
        );
        if meaning.attempt() == AttemptCancellation::Always {
            assert!(
                !explicit.attempt_is_current(command, 0),
                "{intent:?}: the in-flight attempt is no longer current"
            );
        }
        if cancels_script {
            assert!(
                !script.attempt_is_current(script_command, 0),
                "{intent:?}: the script-derived attempt is no longer current"
            );
        }
    }
    for fact in LifecycleFact::ALL {
        let mut auto = armed();
        auto.login_command(false).unwrap();
        let receipt = auto.observe(fact).unwrap();
        assert_eq!(
            receipt.invalidated_attempt(),
            matches!(
                fact.meaning().attempt(),
                AttemptCancellation::Always | AttemptCancellation::Settled
            ),
            "{fact:?}"
        );
    }
    assert_eq!(
        LifecycleFact::LoginSucceeded.meaning().attempt(),
        AttemptCancellation::Settled
    );
}

#[test]
fn only_stop_and_removal_terminate_a_run_and_logout_holds_it() {
    let terminating: Vec<Intent> = Intent::ALL
        .into_iter()
        .filter(|intent| intent.meaning().run() == RunEffect::Terminate)
        .collect();
    assert_eq!(
        terminating,
        [
            Intent::StopScript,
            Intent::RemoveSlot,
            Intent::ProcessShutdown
        ]
    );
    assert_eq!(Intent::Logout.meaning().run(), RunEffect::HoldWork);
    assert_eq!(Intent::Login.meaning().run(), RunEffect::ResumeHeld);
    assert_eq!(
        Intent::WatchdogRestart.meaning().run(),
        RunEffect::RecreateExecution
    );
    assert_eq!(
        Intent::WatchdogRestart.meaning().session(),
        SessionEffect::None
    );
    assert_eq!(Intent::StopScript.meaning().session(), SessionEffect::None);
    assert_eq!(
        Intent::WorldPreference(WorldChoice::World(3))
            .meaning()
            .session(),
        SessionEffect::NextHandshakeOnly
    );
    assert_eq!(
        Intent::LoginIdentityChanged.meaning().session(),
        SessionEffect::NextHandshakeOnly
    );
    for fact in LifecycleFact::ALL {
        assert_ne!(
            fact.meaning().run(),
            RunEffect::Terminate,
            "{fact:?}: no fact ends a run"
        );
    }
    let held = SessionHeld::new(Some(session(1)));
    assert_eq!(held.since(), Some(session(1)));
    assert_eq!(SessionHeld::new(None).since(), None);
}

#[test]
fn every_command_advances_the_generation_but_only_cancelling_rows_move_the_fence() {
    let mut mailbox = armed();
    let start = mailbox.snapshot();
    let login = mailbox.post(Intent::Login).unwrap();
    assert!(login.generation() > start.generation());
    assert_eq!(mailbox.snapshot().cancellation(), start.cancellation());

    let logout = mailbox.post(Intent::Logout).unwrap();
    assert!(logout.generation() > login.generation());
    assert!(mailbox.snapshot().cancellation() > start.cancellation());
    assert!(logout.invalidated_attempt());

    let before = mailbox.snapshot();
    mailbox
        .post(Intent::WorldPreference(WorldChoice::World(2)))
        .unwrap();
    assert_eq!(
        mailbox.snapshot().generation(),
        before.generation(),
        "a world save while nothing is held is not a command"
    );
    assert_eq!(
        mailbox.snapshot().world_generation(),
        before.world_generation() + 1
    );
    mailbox
        .post(Intent::WorldPreference(WorldChoice::World(2)))
        .unwrap();
    assert_eq!(
        mailbox.snapshot().world_generation(),
        before.world_generation() + 1,
        "saving the same world again moves nothing"
    );
}

#[test]
fn mailbox_generations_refuse_instead_of_wrapping_and_leave_the_state_intact() {
    let mut mailbox = armed();
    mailbox.generation = IntentGeneration(u64::MAX);
    let before = mailbox.snapshot();
    for _ in 0..2 {
        assert_eq!(
            mailbox.post(Intent::Login),
            Err(MailboxError::Exhausted(MailboxGeneration::Command)),
            "the refusal is sticky, never a wrap to 0"
        );
        assert_eq!(mailbox.snapshot(), before, "a refused row changes nothing");
    }
    assert_eq!(
        mailbox.post(Intent::MemoryMode(MemoryMode::Low)),
        Ok(PostReceipt {
            generation: IntentGeneration(u64::MAX),
            invalidated_attempt: false
        }),
        "a row that is not a command still applies"
    );

    let mut mailbox = armed();
    mailbox.cancellation = IntentGeneration(u64::MAX);
    let before = mailbox.snapshot();
    assert_eq!(
        mailbox.post(Intent::Logout),
        Err(MailboxError::Exhausted(MailboxGeneration::Cancellation))
    );
    assert_eq!(
        mailbox.snapshot(),
        before,
        "the command generation is not advanced when the fence cannot be"
    );
    assert_eq!(
        mailbox.post(Intent::AutoLogin(true)),
        Ok(PostReceipt {
            generation: IntentGeneration(1),
            invalidated_attempt: false
        }),
        "a non-cancelling command still advances"
    );
    let mut script = script_needed();
    script.cancellation = IntentGeneration(u64::MAX);
    assert_eq!(
        script.post(Intent::ScriptNeedsSession(false)),
        Err(MailboxError::Exhausted(MailboxGeneration::Cancellation))
    );
    assert!(
        script.snapshot().script_needs_session(),
        "the need is not recorded when its invalidation is refused"
    );
    assert_eq!(
        script.observe(LifecycleFact::LoginSucceeded),
        Err(MailboxError::Exhausted(MailboxGeneration::Cancellation))
    );
    assert!(!script.session_online(), "a refused fact records nothing");

    let mut mailbox = armed();
    mailbox.world_generation = u64::MAX;
    let before = mailbox.snapshot();
    assert_eq!(
        mailbox.post(Intent::WorldPreference(WorldChoice::World(2))),
        Err(MailboxError::Exhausted(MailboxGeneration::World))
    );
    assert_eq!(mailbox.snapshot(), before);
    assert_eq!(
        mailbox.post(Intent::SwitchWorld(2)),
        Err(MailboxError::Exhausted(MailboxGeneration::World))
    );
    assert_eq!(
        mailbox.snapshot(),
        before,
        "neither generation moves when the world fence cannot"
    );
    assert_eq!(
        mailbox.snapshot().world(),
        WorldChoice::Automatic,
        "a refused switch selects no world"
    );
}

#[test]
fn explicit_login_arms_once_and_lifts_every_latch_and_hold() {
    let mut mailbox = parked();
    assert!(!mailbox.snapshot().login_wanted());
    assert_eq!(mailbox.snapshot().park_reason(), Some(ParkReason::NoIntent));
    mailbox.post(Intent::Logout).unwrap();
    mailbox
        .observe(LifecycleFact::WorldPreferenceFailed)
        .unwrap();
    assert_eq!(
        mailbox.snapshot().park_reason(),
        Some(ParkReason::OperatorLogout)
    );

    mailbox.post(Intent::Login).unwrap();
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
    mailbox.post(Intent::AutoLogin(true)).unwrap();
    let second = mailbox.login_command(false).unwrap();
    assert!(second.generation() > first.generation());
    assert_eq!(
        second.cancellation(),
        first.cancellation(),
        "re-affirming auto-login does not cancel the queued attempt"
    );
    assert!(mailbox.attempt_is_current(first, 0));
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
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
fn login_success_retires_the_attempt_and_identity_or_script_changes_invalidate_it() {
    // Success: the completed attempt's stamp is retired, so after a later
    // exit a late outcome under it is stale (today `invalidate_clean_exit`
    // in `on_login_success`).
    let mut mailbox = armed();
    let command = mailbox.login_command(false).unwrap();
    let receipt = mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    assert!(receipt.invalidated_attempt());
    mailbox.acknowledge_login(command);
    mailbox.observe(DROP).unwrap();
    assert!(
        mailbox.snapshot().login_wanted(),
        "the preference still wants the relog"
    );
    assert!(
        !mailbox.attempt_is_current(command, 0),
        "the completed attempt cannot install backoff on the relog"
    );
    assert!(mailbox.login_command(false).is_some());

    // A script's need ending invalidates the attempt even when the
    // preference still wants it; re-affirming the need does not.
    let mut mailbox = armed();
    mailbox.post(Intent::ScriptNeedsSession(true)).unwrap();
    let command = mailbox.login_command(false).unwrap();
    assert!(!mailbox
        .post(Intent::ScriptNeedsSession(true))
        .unwrap()
        .invalidated_attempt());
    assert!(mailbox.attempt_is_current(command, 0));
    let receipt = mailbox.post(Intent::ScriptNeedsSession(false)).unwrap();
    assert!(receipt.invalidated_attempt());
    assert!(!mailbox.attempt_is_current(command, 0));
    assert!(
        mailbox.login_command(false).is_some(),
        "the preference's login is re-read fresh"
    );
    assert!(
        !mailbox
            .post(Intent::ScriptNeedsSession(false))
            .unwrap()
            .invalidated_attempt(),
        "an already ended need moves nothing"
    );

    // The login identity changing under a queued attempt.
    let mut mailbox = armed();
    let command = mailbox.login_command(false).unwrap();
    let before = mailbox.snapshot();
    let receipt = mailbox.post(Intent::LoginIdentityChanged).unwrap();
    assert!(receipt.invalidated_attempt());
    assert!(receipt.generation() > before.generation());
    assert!(!mailbox.attempt_is_current(command, 0));
    assert!(
        mailbox.snapshot().login_wanted(),
        "the next handshake uses the new identity; nothing is latched"
    );
    assert_eq!(mailbox.snapshot().latch(), None);
}

#[test]
fn a_stale_acknowledgement_cannot_settle_a_newer_opposite_intent() {
    let mut mailbox = armed();
    let login = mailbox.login_command(false).unwrap();
    mailbox.post(Intent::Logout).unwrap();
    assert_eq!(
        mailbox.acknowledge_login(login),
        Acknowledgement::Stale,
        "a login finished under an older generation cannot clear a Logout"
    );
    assert_eq!(mailbox.snapshot().latch(), Some(LoginLatch::OperatorLogout));
    assert!(mailbox.snapshot().logout_wanted());

    let mut mailbox = armed();
    online(&mut mailbox);
    mailbox.post(Intent::Logout).unwrap();
    let logout = mailbox.logout_command(true).unwrap();
    mailbox.post(Intent::Login).unwrap();
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
    mailbox.post(Intent::ScriptNeedsSession(true)).unwrap();
    online(&mut mailbox);
    assert!(mailbox.keeps_run_work_after_exit());
    assert!(mailbox.relogs_after_drop());

    mailbox.post(Intent::Logout).unwrap();
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
    mailbox.observe(CLEAN_EXIT).unwrap();
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
    mailbox.post(Intent::AutoLogin(true)).unwrap();
    assert!(
        !mailbox.snapshot().login_wanted(),
        "a preference toggle does not lift the latch"
    );
    mailbox
        .post(Intent::WorldPreference(WorldChoice::World(7)))
        .unwrap();
    assert!(
        !mailbox.snapshot().login_wanted(),
        "neither does a world save"
    );
    assert!(mailbox.login_command(false).is_none());

    mailbox.post(Intent::Login).unwrap();
    assert!(mailbox.snapshot().login_wanted());
    assert!(mailbox.login_command(false).is_some());
}

#[test]
fn an_offline_logout_owes_one_held_work_reset_and_a_connected_one_none() {
    let mut mailbox = armed();
    mailbox.post(Intent::Logout).unwrap();
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
    mailbox.post(Intent::Logout).unwrap();
    assert!(
        !mailbox.take_work_reset(),
        "a connected exit reads OperatorLogout from the latch instead"
    );

    let mut mailbox = armed();
    mailbox.post(Intent::Logout).unwrap();
    mailbox.post(Intent::Login).unwrap();
    assert!(
        mailbox.take_work_reset(),
        "a later Login does not clear an older unconsumed offline reset"
    );
}

#[test]
fn unexpected_drop_reconnects_only_while_wanted_and_the_repeat_guard_parks_it() {
    let mut mailbox = script_needed();
    assert!(
        mailbox.snapshot().login_wanted(),
        "an active script wants the login whatever the preference says"
    );
    online(&mut mailbox);
    assert!(
        mailbox.reconnects(),
        "after a grant the next handshake reconnects"
    );
    mailbox.observe(DROP).unwrap();
    assert!(mailbox.reconnects());
    assert!(mailbox.relogs_after_drop());
    assert!(mailbox.keeps_run_work_after_exit());
    assert!(
        !mailbox.login_applies_memory(),
        "a dirty reconnect preserves the server session's memory mode"
    );

    let receipt = mailbox
        .observe(LifecycleFact::RepeatGuardTripped {
            count: 3,
            window_seconds: 600,
        })
        .unwrap();
    assert!(receipt.invalidated_attempt());
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
    let again = mailbox
        .observe(LifecycleFact::RepeatGuardTripped {
            count: 3,
            window_seconds: 600,
        })
        .unwrap();
    assert!(
        !again.invalidated_attempt(),
        "an already latched slot records nothing new"
    );

    mailbox.post(Intent::Login).unwrap();
    assert_eq!(
        mailbox.snapshot().latch(),
        None,
        "explicit Login clears the guard"
    );
    assert!(mailbox.relogs_after_drop());

    let mut stopped = script_needed();
    online(&mut stopped);
    stopped.post(Intent::ScriptNeedsSession(false)).unwrap();
    assert!(
        !stopped.relogs_after_drop(),
        "a stopped script stops the relog"
    );
    assert!(!stopped.keeps_run_work_after_exit());
}

#[test]
fn a_world_error_holds_the_login_until_the_operator_changes_something() {
    let mut mailbox = armed();
    mailbox.post(Intent::ScriptNeedsSession(true)).unwrap();
    let command = mailbox.login_command(false).unwrap();
    let receipt = mailbox
        .observe(LifecycleFact::WorldPreferenceFailed)
        .unwrap();
    assert!(receipt.invalidated_attempt());
    assert!(!mailbox.attempt_is_current(command, 0));
    let snapshot = mailbox.snapshot();
    assert!(snapshot.error_hold());
    assert!(
        !snapshot.login_wanted(),
        "the hold suppresses the script's need as well"
    );
    assert_eq!(snapshot.park_reason(), Some(ParkReason::WorldError));

    mailbox
        .post(Intent::WorldPreference(WorldChoice::World(12)))
        .unwrap();
    let snapshot = mailbox.snapshot();
    assert!(!snapshot.error_hold(), "a world change lifts the hold");
    assert_eq!(
        snapshot.login(),
        LoginIntent::Auto,
        "and re-derives the want from the still-enabled preference"
    );

    let mut toggled = armed();
    toggled
        .observe(LifecycleFact::WorldPreferenceFailed)
        .unwrap();
    toggled.post(Intent::AutoLogin(false)).unwrap();
    assert!(!toggled.snapshot().error_hold());
    assert_eq!(toggled.snapshot().login(), LoginIntent::Withdrawn);
    toggled.post(Intent::AutoLogin(true)).unwrap();
    assert_eq!(toggled.snapshot().login(), LoginIntent::Auto);
}

#[test]
fn a_world_save_never_cancels_but_makes_the_attempts_outcome_stale() {
    let mut mailbox = armed();
    let command = mailbox.login_command(false).unwrap();
    let world = mailbox.snapshot().world_generation();
    let receipt = mailbox
        .post(Intent::WorldPreference(WorldChoice::World(9)))
        .unwrap();
    assert!(!receipt.invalidated_attempt());
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
    let receipt = mailbox.post(Intent::AutoLogin(false)).unwrap();
    assert!(receipt.invalidated_attempt());
    assert!(!mailbox.attempt_is_current(auto, 0));
    assert!(!mailbox.snapshot().login_wanted());
    assert!(!mailbox.snapshot().auto_login());

    let mut explicit = parked();
    explicit.post(Intent::Login).unwrap();
    let command = explicit.login_command(false).unwrap();
    assert!(!explicit
        .post(Intent::AutoLogin(false))
        .unwrap()
        .invalidated_attempt());
    assert!(
        explicit.attempt_is_current(command, 0),
        "an explicit login outlives the toggle"
    );
    assert!(explicit
        .post(Intent::WithdrawLogin)
        .unwrap()
        .invalidated_attempt());
    assert!(!explicit.attempt_is_current(command, 0));
    assert!(!explicit.snapshot().login_wanted());
}

#[test]
fn relog_now_is_a_turnover_not_a_logout_latch() {
    let mut mailbox = armed();
    online(&mut mailbox);
    let receipt = mailbox.post(Intent::RelogNow).unwrap();
    assert!(!receipt.invalidated_attempt());
    assert!(mailbox.snapshot().turnover_pending());
    let logout = mailbox.logout_command(true).unwrap();
    assert_eq!(mailbox.acknowledge_logout(logout), Acknowledgement::Applied);
    assert!(!mailbox.snapshot().turnover_pending());
    assert_eq!(mailbox.snapshot().latch(), None, "no park latch");
    assert_eq!(
        mailbox.snapshot().login(),
        LoginIntent::Once,
        "the login half stays armed"
    );
    mailbox.observe(CLEAN_EXIT).unwrap();
    assert!(!mailbox.reconnects(), "the turnover's login is fresh");
    let login = mailbox.login_command(false).unwrap();
    assert!(
        mailbox.login_applies_memory(),
        "a fresh handshake applies the queued memory mode"
    );
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    assert_eq!(mailbox.acknowledge_login(login), Acknowledgement::Applied);
    assert!(
        mailbox.logout_command(true).is_none(),
        "the new session owes no logout"
    );

    let mut plain = armed();
    online(&mut plain);
    plain.post(Intent::Logout).unwrap();
    let logout = plain.logout_command(true).unwrap();
    plain.acknowledge_logout(logout);
    assert_eq!(plain.snapshot().latch(), Some(LoginLatch::OperatorLogout));
    assert!(plain.login_command(false).is_none());
}

#[test]
fn an_offline_relog_is_a_login_and_owes_the_new_session_no_logout() {
    for begin in [Intent::RelogNow, Intent::SwitchWorld(3)] {
        let mut mailbox = parked();
        mailbox.post(Intent::Logout).unwrap();
        mailbox.post(begin).unwrap();
        let snapshot = mailbox.snapshot();
        assert!(
            !snapshot.logout_wanted(),
            "{begin:?}: nothing to log out of"
        );
        assert!(!snapshot.turnover_pending(), "{begin:?}");
        assert_eq!(snapshot.latch(), None, "{begin:?}: the latch is lifted");
        assert_eq!(snapshot.login(), LoginIntent::Once, "{begin:?}");
        let login = mailbox.login_command(false).unwrap();
        mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
        assert_eq!(mailbox.acknowledge_login(login), Acknowledgement::Applied);
        assert!(
            mailbox.logout_command(true).is_none(),
            "{begin:?}: the new session owes no logout"
        );
        assert!(
            !mailbox.snapshot().logout_wanted(),
            "{begin:?}: no logout loop"
        );
    }
}

#[test]
fn a_stronger_row_supersedes_a_pending_turnover() {
    for begin in [Intent::RelogNow, Intent::SwitchWorld(4)] {
        for stronger in [
            Intent::Login,
            Intent::Logout,
            Intent::HoldLoggedOut,
            Intent::WithdrawLogin,
            Intent::RemoveSlot,
            Intent::ProcessShutdown,
        ] {
            let mut mailbox = armed();
            online(&mut mailbox);
            mailbox.post(begin).unwrap();
            let logout = mailbox.logout_command(true).unwrap();
            mailbox.post(stronger).unwrap();
            assert!(
                !mailbox.snapshot().turnover_pending(),
                "{begin:?} then {stronger:?}: the login half is superseded"
            );
            // The old logout press reaches the mailbox late: whatever it
            // settles, it cannot re-arm the turnover's login.
            mailbox.acknowledge_logout(logout);
            assert!(
                !mailbox.snapshot().turnover_pending(),
                "{begin:?} then {stronger:?} then late ack"
            );
            match stronger {
                Intent::Login => {
                    assert_eq!(
                        mailbox.snapshot().login(),
                        LoginIntent::Once,
                        "the explicit login is its own want"
                    );
                    assert!(!mailbox.snapshot().logout_wanted());
                }
                Intent::RemoveSlot | Intent::ProcessShutdown => {
                    assert!(mailbox.snapshot().closed());
                    assert!(
                        mailbox.login_command(false).is_none(),
                        "{stronger:?}: a closed lifetime re-arms nothing"
                    );
                }
                _ => {
                    assert!(
                        mailbox.login_command(false).is_none(),
                        "{begin:?} then {stronger:?}: no login demand"
                    );
                    assert_eq!(mailbox.snapshot().login(), LoginIntent::Withdrawn);
                }
            }
        }

        // The world-error hold ends the turnover too.
        let mut mailbox = armed();
        online(&mut mailbox);
        mailbox.post(begin).unwrap();
        let logout = mailbox.logout_command(true).unwrap();
        mailbox
            .observe(LifecycleFact::WorldPreferenceFailed)
            .unwrap();
        assert!(!mailbox.snapshot().turnover_pending());
        mailbox.acknowledge_logout(logout);
        assert!(mailbox.login_command(false).is_none(), "{begin:?}: held");
        assert_eq!(
            mailbox.snapshot().park_reason(),
            Some(ParkReason::OperatorLogout),
            "the completed logout half latches like a Logout once its login half is gone"
        );

        // An agreeing refresh keeps the turnover.
        let mut mailbox = armed();
        online(&mut mailbox);
        mailbox.post(begin).unwrap();
        let logout = mailbox.logout_command(true).unwrap();
        mailbox.post(Intent::AutoLogin(true)).unwrap();
        mailbox.post(begin).unwrap();
        assert!(mailbox.snapshot().turnover_pending());
        assert_eq!(mailbox.acknowledge_logout(logout), Acknowledgement::Applied);
        assert_eq!(mailbox.snapshot().login(), LoginIntent::Once, "{begin:?}");
        assert_eq!(mailbox.snapshot().latch(), None);
    }
}

#[test]
fn a_session_exit_settles_a_pending_turnovers_logout_half() {
    let mut mailbox = armed();
    mailbox.post(Intent::ScriptNeedsSession(true)).unwrap();
    online(&mut mailbox);
    mailbox.post(Intent::RelogNow).unwrap();
    assert!(
        !mailbox.keeps_run_work_after_exit(),
        "the turnover clears session work like a clean logout"
    );
    // The connection drops before the logout press is acknowledged.
    mailbox.observe(DROP).unwrap();
    let snapshot = mailbox.snapshot();
    assert!(
        !snapshot.logout_wanted(),
        "the session is gone: the logout half is done"
    );
    assert!(!snapshot.turnover_pending());
    assert_eq!(
        snapshot.login(),
        LoginIntent::Once,
        "the login half is armed"
    );
    assert_eq!(snapshot.latch(), None);
    assert!(mailbox.relogs_after_drop());
    let login = mailbox.login_command(false).unwrap();
    mailbox.observe(LifecycleFact::LoginSucceeded).unwrap();
    assert_eq!(mailbox.acknowledge_login(login), Acknowledgement::Applied);
    assert!(
        mailbox.logout_command(true).is_none(),
        "no logout follows the relogged session"
    );
}

#[test]
fn switch_world_cancels_or_exits_then_logs_in_to_the_requested_world() {
    let mut queued = armed();
    let command = queued.login_command(false).unwrap();
    let receipt = queued.post(Intent::SwitchWorld(4)).unwrap();
    assert!(receipt.invalidated_attempt());
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
    live.post(Intent::SwitchWorld(5)).unwrap();
    let logout = live.logout_command(true).unwrap();
    assert_eq!(live.acknowledge_logout(logout), Acknowledgement::Applied);
    assert_eq!(live.snapshot().latch(), None);
    assert_eq!(live.snapshot().login(), LoginIntent::Once);
    assert_eq!(live.snapshot().world(), WorldChoice::World(5));
}

#[test]
fn remove_and_shutdown_close_the_lifetime_for_good() {
    for close in [Intent::RemoveSlot, Intent::ProcessShutdown] {
        // Queued: the attempt is cancelled and nothing re-arms the lifetime.
        let mut mailbox = armed();
        mailbox.post(Intent::ScriptNeedsSession(true)).unwrap();
        let command = mailbox.login_command(false).unwrap();
        let receipt = mailbox.post(close).unwrap();
        assert!(receipt.invalidated_attempt());
        assert!(!mailbox.attempt_is_current(command, 0));
        let snapshot = mailbox.snapshot();
        assert!(snapshot.closed(), "{close:?}");
        assert!(!snapshot.login_wanted(), "{close:?}");
        assert!(!snapshot.script_needs_session());
        assert!(!snapshot.logout_wanted(), "offline: nothing to log out of");
        for rearm in [
            Intent::Login,
            Intent::AutoLogin(true),
            Intent::ScriptNeedsSession(true),
            Intent::RelogNow,
            Intent::SwitchWorld(2),
            Intent::WorldPreference(WorldChoice::World(2)),
            Intent::StartScript,
        ] {
            assert_eq!(
                mailbox.post(rearm),
                Err(MailboxError::Closed),
                "{close:?} then {rearm:?}"
            );
            assert!(
                mailbox.login_command(false).is_none(),
                "{close:?} then {rearm:?}: no new login command"
            );
            assert!(!mailbox.snapshot().login_wanted());
        }
        assert_eq!(
            mailbox.acknowledge_login(command),
            Acknowledgement::Stale,
            "a late completion for the closed lifetime records nothing"
        );
        assert!(
            mailbox.observe(LifecycleFact::LoginSucceeded).is_ok(),
            "a late handshake is still a fact the teardown reads"
        );
        assert!(mailbox.session_online());
        assert!(
            mailbox.logout_command(true).is_none(),
            "an attempt that completed after the close owes no clean logout"
        );
        assert!(
            mailbox
                .observe(LifecycleFact::RepeatGuardTripped {
                    count: 3,
                    window_seconds: 600
                })
                .is_ok_and(|receipt| !receipt.invalidated_attempt()),
            "guard facts no longer change a closed lifetime"
        );
        assert_eq!(mailbox.snapshot().latch(), None);
        let after = mailbox.snapshot();
        assert!(
            !mailbox.post(close).unwrap().invalidated_attempt(),
            "a second close is idempotent"
        );
        assert_eq!(mailbox.snapshot(), after);

        // Connected: the clean logout goes out first, then the lifetime ends
        // with the exit; the old login command cannot re-arm it.
        let mut live = armed();
        let command = online(&mut live);
        live.post(close).unwrap();
        let logout = live
            .logout_command(true)
            .expect("a connected slot gets the clean logout first");
        assert!(live.login_command(false).is_none());
        assert_eq!(live.acknowledge_logout(logout), Acknowledgement::Applied);
        assert!(live.logout_command(true).is_none());
        live.observe(CLEAN_EXIT).unwrap();
        assert!(live.login_command(false).is_none(), "{close:?}: no relog");
        assert!(!live.relogs_after_drop());
        assert_eq!(live.acknowledge_login(command), Acknowledgement::Stale);
        assert_eq!(live.post(Intent::Login), Err(MailboxError::Closed));
    }
}

#[test]
fn memory_mode_queues_for_the_next_fresh_handshake_only() {
    let mut mailbox = armed();
    assert_eq!(mailbox.snapshot().memory(), None);
    let before = mailbox.snapshot().generation();
    mailbox.post(Intent::MemoryMode(MemoryMode::Low)).unwrap();
    assert_eq!(mailbox.snapshot().memory(), Some(MemoryMode::Low));
    assert_eq!(
        mailbox.snapshot().generation(),
        before,
        "not a login command"
    );
    assert!(mailbox.login_applies_memory(), "the first login is fresh");
    online(&mut mailbox);
    mailbox.observe(DROP).unwrap();
    assert!(
        !mailbox.login_applies_memory(),
        "a reconnect keeps the applied mode"
    );
    mailbox.observe(CLEAN_EXIT).unwrap();
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
    assert!(!SessionEndCause::ServerLogout.counts_toward_repeat_guard());
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
        assert!(cause.counts_toward_repeat_guard(), "{reason:?}");
        assert_ne!(cause, SessionEndCause::ServerLogout);
    }
    assert!(SessionEndCause::Removed.is_clean());
    assert!(!SessionEndCause::Removed.counts_toward_repeat_guard());

    let outage = SessionEndCause::ServiceUnavailable {
        condition: crate::process_condition::ConditionId::new(
            ProcessIncarnation::from_start_nonce(NonZeroU64::MIN),
            ProcessServiceCode::ReactorUnavailable,
        ),
    };
    assert!(!outage.is_clean(), "an outage is not a clean end");
    assert!(
        !outage.counts_toward_repeat_guard(),
        "and never a repeat-guard strike: the retry is refused at admission"
    );
}

#[test]
fn scripts_tick_only_between_scene_ready_and_session_ended_of_the_begun_session() {
    let mut gate = SessionGate::new(slot());
    let first = session(1);
    let second = session(2);
    let third = session(3);
    assert_eq!(gate.ready(), None);
    assert_eq!(gate.current(), None);

    // Readiness before any authorization is stale, not an authorization.
    assert_eq!(
        gate.observe(first, &SessionEventOut::SceneReady),
        Err(SessionGateError::StaleSession {
            current: None,
            stamped: first
        })
    );
    assert_eq!(gate.ready(), None, "no session begins by being ready");

    let other = SessionKey::new(SlotKey::from_incarnation(6).unwrap(), 1);
    assert_eq!(
        gate.begin(other),
        Err(SessionGateError::WrongSlot {
            expected: slot(),
            offered: other
        })
    );
    gate.begin(first).unwrap();
    assert_eq!(gate.current(), Some(first));
    assert_eq!(gate.ready(), None, "begun is not ready");
    assert_eq!(
        gate.observe(other, &SessionEventOut::SceneReady),
        Err(SessionGateError::StaleSession {
            current: Some(first),
            stamped: other
        }),
        "another slot's session never applies"
    );

    gate.observe(first, &SessionEventOut::SceneReady).unwrap();
    assert_eq!(gate.ready(), Some(first));

    assert!(gate
        .observe(
            second,
            &SessionEventOut::SessionEnded {
                cause: SessionEndCause::ConnectionLost,
                resumable: true,
            },
        )
        .is_err());
    assert_eq!(
        gate.ready(),
        Some(first),
        "an end stamped with another session key is stale and changes nothing"
    );
    assert!(gate
        .observe(
            second,
            &SessionEventOut::Held {
                why: HoldReason::Guardian,
            },
        )
        .is_err());
    assert_eq!(gate.ready(), Some(first), "a stale hold is ignored");

    gate.observe(
        first,
        &SessionEventOut::Held {
            why: HoldReason::Guardian,
        },
    )
    .unwrap();
    assert_eq!(gate.ready(), None);
    assert!(gate.held());
    assert!(gate.held_by(HoldReason::Guardian));
    assert!(!gate.held_by(HoldReason::WelcomeModal));
    gate.observe(first, &SessionEventOut::SceneReady).unwrap();
    assert_eq!(
        gate.ready(),
        None,
        "a duplicate readiness does not release the guardian hold"
    );
    assert!(gate.held_by(HoldReason::Guardian));
    gate.observe(
        first,
        &SessionEventOut::Held {
            why: HoldReason::WelcomeModal,
        },
    )
    .unwrap();
    gate.observe(
        first,
        &SessionEventOut::Released {
            why: HoldReason::WelcomeModal,
        },
    )
    .unwrap();
    assert_eq!(
        gate.ready(),
        None,
        "the modal's release does not release the guardian"
    );
    assert!(gate.held_by(HoldReason::Guardian));
    gate.observe(
        first,
        &SessionEventOut::Released {
            why: HoldReason::Guardian,
        },
    )
    .unwrap();
    assert_eq!(gate.ready(), Some(first));
    assert!(!gate.held());
    gate.observe(
        first,
        &SessionEventOut::Released {
            why: HoldReason::Guardian,
        },
    )
    .unwrap();
    assert_eq!(
        gate.ready(),
        Some(first),
        "an extra release of a reason not held changes nothing"
    );
    for why in HoldReason::ALL {
        gate.observe(first, &SessionEventOut::Held { why }).unwrap();
        gate.observe(first, &SessionEventOut::Held { why }).unwrap();
    }
    assert!(gate.held());
    for why in HoldReason::ALL {
        gate.observe(first, &SessionEventOut::Released { why })
            .unwrap();
    }
    assert_eq!(
        gate.ready(),
        Some(first),
        "a hold set twice is one hold, released by its one release"
    );

    gate.observe(
        first,
        &SessionEventOut::SessionEnded {
            cause: SessionEndCause::ServerLogout,
            resumable: false,
        },
    )
    .unwrap();
    assert_eq!(gate.ready(), None);
    assert_eq!(gate.current(), None);
    assert!(!gate.held());
    assert!(gate
        .observe(
            first,
            &SessionEventOut::Released {
                why: HoldReason::Guardian
            }
        )
        .is_err());
    assert_eq!(
        gate.observe(first, &SessionEventOut::SceneReady),
        Err(SessionGateError::StaleSession {
            current: None,
            stamped: first
        }),
        "a late readiness cannot resurrect the ended session"
    );
    assert_eq!(gate.ready(), None);
    assert_eq!(
        gate.begin(first),
        Err(SessionGateError::ObsoleteEpoch {
            latest: first,
            offered: first
        }),
        "an ended epoch never begins again"
    );

    gate.begin(second).unwrap();
    gate.observe(
        second,
        &SessionEventOut::HandshakeOk {
            kind: HandshakeKind::Reconnect,
        },
    )
    .unwrap();
    assert_eq!(gate.ready(), None, "a handshake is not scene readiness");
    gate.observe(second, &SessionEventOut::SceneReady).unwrap();
    assert_eq!(gate.ready(), Some(second));
    assert!(
        gate.observe(first, &SessionEventOut::SceneReady).is_err(),
        "a late readiness of the old session is stale"
    );
    assert_eq!(
        gate.ready(),
        Some(second),
        "and cannot replace the newer session"
    );
    assert!(gate
        .observe(
            first,
            &SessionEventOut::SessionEnded {
                cause: SessionEndCause::ConnectionLost,
                resumable: true,
            },
        )
        .is_err());
    assert_eq!(
        gate.ready(),
        Some(second),
        "a late end of the old session cannot close the new one"
    );

    gate.begin(third).unwrap();
    assert_eq!(gate.current(), Some(third));
    assert_eq!(
        gate.ready(),
        None,
        "beginning a newer session supersedes the current one"
    );
    assert!(gate.observe(second, &SessionEventOut::SceneReady).is_err());
    assert_eq!(gate.ready(), None);
    assert_eq!(
        gate.begin(second),
        Err(SessionGateError::ObsoleteEpoch {
            latest: third,
            offered: second
        })
    );
}
