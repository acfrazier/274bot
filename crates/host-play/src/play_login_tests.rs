use super::*;

const HOST: &str = "world.example";
const PORT: u16 = 43594;

fn clean_exit(now: Instant) -> Arc<SlotArm> {
    let arm = SlotArm::new(7, true);
    arm.record_session_exit_at(true, HOST, PORT, now);
    arm
}

fn busy() -> LoginError {
    LoginError::response(5, "", "account in use")
}

fn queue_attempt(queue: &mut LoginQueue, arm: &SlotArm, now: Instant) {
    queue.enqueue_owner(arm.queue_owner, 7);
    if let Some(until) = arm.clean_release_admission_at(HOST, PORT, now) {
        queue.defer_owner_until(arm.queue_owner, until);
    }
    assert_eq!(queue.poll_owner(arm.queue_owner, 7, now), Permit::Grant);
    assert!(queue
        .start_attempt(arm.queue_owner, now, LoginCancel::new())
        .is_some());
}

#[test]
fn clean_exit_paces_from_exit_not_old_handshake_and_release_then_admits() {
    let start = Instant::now();
    let arm = clean_exit(start);
    let mut queue = LoginQueue::default();
    queue.enqueue_owner(arm.queue_owner, 7);
    let until = arm.clean_release_admission_at(HOST, PORT, start).unwrap();
    assert_eq!(until, start + host::login_queue::LOGIN_START_SPACING);
    queue.defer_owner_until(arm.queue_owner, until);
    assert!(matches!(
        queue.poll_owner(arm.queue_owner, 7, start),
        Permit::Wait(_)
    ));
    assert_eq!(queue.status_owner(arm.queue_owner).unwrap().position, 1);
    queue_attempt(&mut queue, &arm, until);
    let command = arm.login_command(false).unwrap();
    assert!(queue.acknowledge_login_return(arm.queue_owner, until));
    assert_eq!(
        arm.defer_clean_release_at(&busy(), command, HOST, PORT, until),
        Some(CLEAN_RELEASE_CADENCE)
    );
    let retry = until + CLEAN_RELEASE_CADENCE;
    queue_attempt(&mut queue, &arm, retry);
    assert!(queue.acknowledge_login_return(arm.queue_owner, retry));
    on_login_success(&arm, command);
    assert!(arm.session_online());
    assert!(arm.clean_release_admission_at(HOST, PORT, retry).is_none());
    assert_eq!(
        arm.defer_clean_release_at(&busy(), command, HOST, PORT, retry),
        None
    );
}

#[test]
fn never_released_exhausts_absolute_budget_then_uses_generic_code_five() {
    let start = Instant::now();
    let arm = clean_exit(start);
    let command = arm.login_command(false).unwrap();
    for seconds in [1, 6, 11, 16, 21] {
        assert_eq!(
            arm.defer_clean_release_at(
                &busy(),
                command,
                HOST,
                PORT,
                start + Duration::from_secs(seconds)
            ),
            Some(CLEAN_RELEASE_CADENCE)
        );
    }
    assert_eq!(
        arm.defer_clean_release_at(
            &busy(),
            command,
            HOST,
            PORT,
            start + Duration::from_secs(26)
        ),
        None
    );
    assert_eq!(
        login_retry_wait(&mut LoginBackoff::new(), &busy(), 0),
        Duration::from_secs(60)
    );
    arm.arm_explicit_login();
    assert!(arm
        .clean_release_admission_at(HOST, PORT, start + CLEAN_RELEASE_BUDGET)
        .is_none());
}

#[test]
fn dirty_other_world_and_no_evidence_keep_generic_code_five() {
    let now = Instant::now();
    let dirty = clean_exit(now);
    let command = dirty.login_command(false).unwrap();
    dirty.record_session_exit_at(false, HOST, PORT, now);
    assert_eq!(
        dirty.defer_clean_release_at(&busy(), command, HOST, PORT, now),
        None
    );
    for (host, port) in [("other.example", PORT), (HOST, PORT + 1)] {
        let arm = clean_exit(now);
        let command = arm.login_command(false).unwrap();
        assert_eq!(
            arm.defer_clean_release_at(&busy(), command, host, port, now),
            None
        );
        assert_eq!(
            arm.defer_clean_release_at(&busy(), command, HOST, PORT, now),
            None,
            "world mismatch consumes evidence"
        );
    }
    let arm = SlotArm::new(7, true);
    let command = arm.login_command(false).unwrap();
    assert_eq!(
        arm.defer_clean_release_at(&busy(), command, HOST, PORT, now),
        None
    );
    assert_eq!(
        login_retry_wait(&mut LoginBackoff::new(), &busy(), 0),
        Duration::from_secs(60)
    );
}

#[test]
fn code_sixteen_keeps_shared_hold_and_does_not_extend_release_budget() {
    let start = Instant::now();
    let arm = clean_exit(start);
    let command = arm.login_command(false).unwrap();
    assert!(arm
        .defer_clean_release_at(&busy(), command, HOST, PORT, start)
        .is_some());
    let throttle = LoginError::response(16, "", "too many attempts");
    let mut backoff = LoginBackoff::new();
    let hold = server_hold::<()>(
        &Err(LoginError::response(16, "", "too many attempts")),
        &backoff,
    )
    .unwrap();
    assert_eq!(
        arm.defer_clean_release_at(&throttle, command, HOST, PORT, start),
        None
    );
    assert_eq!(login_retry_wait(&mut backoff, &throttle, 0), hold);
    let mut queue = LoginQueue::default();
    queue.hold_for(start, hold);
    queue.enqueue_owner(arm.queue_owner, 7);
    queue.defer_owner_until(arm.queue_owner, start + CLEAN_RELEASE_CADENCE);
    assert!(matches!(
        queue.poll_owner(arm.queue_owner, 7, start),
        Permit::Wait(_)
    ));
    assert!(arm
        .clean_release_admission_at(HOST, PORT, start + CLEAN_RELEASE_BUDGET)
        .is_none());
}

#[test]
fn stop_logout_withdraw_world_and_identity_invalidate_release_wait() {
    let now = Instant::now();
    for action in 0..7 {
        let arm = clean_exit(now);
        let command = arm.login_command(false).unwrap();
        assert!(arm
            .defer_clean_release_at(&busy(), command, HOST, PORT, now)
            .is_some());
        match action {
            0 => arm.stop.store(true, Ordering::Relaxed),
            1 => arm.request_logout(),
            2 => arm.withdraw_login(),
            3 => {
                arm.world_generation.fetch_add(1, Ordering::Relaxed);
            }
            4 => {
                arm.uid.store(8, Ordering::Relaxed);
            }
            5 => arm.invalidate_login_identity(),
            6 => arm.set_auto_login(false),
            _ => unreachable!(),
        }
        assert_eq!(
            arm.defer_clean_release_at(&busy(), command, HOST, PORT, now),
            None,
            "action {action}"
        );
        arm.arm_explicit_login();
        assert!(
            arm.clean_release_admission_at(HOST, PORT, now).is_none(),
            "cannot revive invalidated evidence {action}"
        );
    }
}

#[test]
fn explicit_login_can_claim_unbound_clean_logout_but_not_cancelled_retry() {
    let now = Instant::now();
    let arm = SlotArm::new(7, false);
    arm.request_logout();
    arm.record_session_exit_at(true, HOST, PORT, now);
    arm.arm_explicit_login();
    assert!(arm.clean_release_admission_at(HOST, PORT, now).is_some());
    arm.request_logout();
    arm.arm_explicit_login();
    assert!(arm.clean_release_admission_at(HOST, PORT, now).is_none());
}

#[test]
fn release_wait_owns_fifo_place_not_handshake_and_never_overlaps_attempts() {
    let now = Instant::now();
    let arm = clean_exit(now);
    let command = arm.login_command(false).unwrap();
    let mut queue = LoginQueue::new(Duration::ZERO, 29, Duration::from_secs(60));
    assert!(arm
        .defer_clean_release_at(&busy(), command, HOST, PORT, now)
        .is_some());
    // A sibling already queued is admitted while the relog is release-waiting.
    queue.enqueue_owner(u64::MAX, 9);
    queue.enqueue_owner(arm.queue_owner, 7);
    queue.defer_owner_until(arm.queue_owner, now + CLEAN_RELEASE_CADENCE);
    assert_eq!(queue.poll_owner(u64::MAX, 9, now), Permit::Grant);
    queue
        .start_attempt(u64::MAX, now, LoginCancel::new())
        .unwrap();
    let later = now + CLEAN_RELEASE_CADENCE;
    assert!(
        matches!(queue.poll_owner(arm.queue_owner, 7, later), Permit::Wait(_)),
        "still one handshake even after release deadline"
    );
    assert!(queue.acknowledge_login_return(u64::MAX, later));
    assert_eq!(queue.poll_owner(arm.queue_owner, 7, later), Permit::Grant);
    queue
        .start_attempt(arm.queue_owner, later, LoginCancel::new())
        .unwrap();
    queue.enqueue_owner(u64::MAX - 1, 10);
    assert!(matches!(
        queue.poll_owner(u64::MAX - 1, 10, later),
        Permit::Wait(_)
    ));
    assert!(queue.acknowledge_login_return(arm.queue_owner, later));
    assert_eq!(queue.poll_owner(u64::MAX - 1, 10, later), Permit::Grant);
}

#[test]
fn queued_release_cancellation_retires_place_without_withdrawing_replacement() {
    for action in 0..4 {
        let arm = clean_exit(Instant::now());
        let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        queue.lock().enqueue_owner(arm.queue_owner, 7);
        queue
            .lock()
            .defer_owner_until(arm.queue_owner, Instant::now() + CLEAN_RELEASE_BUDGET);
        let mut changed = false;
        let result = wait_for_permit(&queue, &statuses, "alice", 7, &arm, &mut || {
            assert!(!changed, "the old wait must not survive its cancellation");
            changed = true;
            match action {
                0 => arm.stop.store(true, Ordering::Relaxed),
                1 => arm.request_logout(),
                2 => {
                    arm.world_generation.fetch_add(1, Ordering::Relaxed);
                }
                3 => {
                    arm.uid.store(8, Ordering::Relaxed);
                }
                _ => unreachable!(),
            }
            Duration::ZERO
        });
        assert!(matches!(result, PermitWait::Cancelled));
        assert!(queue.lock().status_owner(arm.queue_owner).is_none());
        assert!(changed);
        if action >= 2 {
            assert!(
                arm.login_wanted(),
                "replacement login intent remains current"
            );
        }
        queue.lock().enqueue_owner(u64::MAX, 9);
        assert_eq!(
            queue.lock().poll_owner(u64::MAX, 9, Instant::now()),
            Permit::Grant
        );
    }
}

#[test]
fn stale_attempt_cannot_install_generic_backoff_on_new_login_intent() {
    let arm = SlotArm::new(7, true);
    let command = arm.login_command(false).unwrap();
    arm.request_logout();
    arm.arm_explicit_login();
    assert!(!arm.login_attempt_is_current(command, 0));
    let mut frames = 0;
    assert!(
        arm.wait_for_retry(Duration::from_secs(60), Some((command, 0)), &mut || {
            frames += 1;
            assert_eq!(frames, 1, "the stale backoff must return before waiting");
            Duration::ZERO
        },)
    );
    assert!(arm.login_wanted());
}

#[test]
fn reaffirming_login_and_load_all_keep_bound_release_evidence() {
    for load_all in [false, true] {
        let now = Instant::now();
        let arm = clean_exit(now);
        let command = arm.login_command(false).unwrap();
        let until = arm.clean_release_admission_at(HOST, PORT, now).unwrap();
        if load_all {
            arm.set_auto_login(true);
        } else {
            arm.arm_explicit_login();
        }
        assert_eq!(arm.clean_release_admission_at(HOST, PORT, now), Some(until));
        assert!(arm.login_attempt_is_current(command, 0));
        assert_eq!(
            arm.defer_clean_release_at(&busy(), command, HOST, PORT, until),
            Some(CLEAN_RELEASE_CADENCE)
        );
    }
}

#[test]
fn agreeing_commands_keep_fifo_place_with_and_without_release_wait() {
    for release in [false, true] {
        for action in 0..3 {
            let arm = SlotArm::new(7, true);
            if release {
                arm.record_session_exit_at(true, HOST, PORT, Instant::now());
            }
            let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
            let statuses = Arc::new(Mutex::new(Vec::new()));
            queue.lock().enqueue_owner(u64::MAX, 9);
            queue.lock().enqueue_owner(arm.queue_owner, 7);
            queue.lock().enqueue_owner(u64::MAX - 1, 10);
            if release {
                queue
                    .lock()
                    .defer_owner_until(arm.queue_owner, Instant::now() + CLEAN_RELEASE_CADENCE);
            }
            let mut changed = false;
            let result = wait_for_permit(&queue, &statuses, "alice", 7, &arm, &mut || {
                if changed {
                    return Duration::from_millis(20);
                }
                changed = true;
                match action {
                    0 => arm.arm_explicit_login(),
                    1 => arm.set_auto_login(true),
                    2 => {
                        let profile = Profile {
                            username: "alice".into(),
                            password: "new-password".into(),
                            uid: 7,
                            settings: vault::ProfileSettings::default(),
                        };
                        sync_profile_arm(&arm, &profile);
                    }
                    _ => unreachable!(),
                }
                assert_eq!(
                    queue.lock().status_owner(arm.queue_owner).unwrap().position,
                    2
                );
                if release {
                    assert!(arm
                        .clean_release_admission_at(HOST, PORT, Instant::now())
                        .is_some());
                }
                queue.lock().leave_owner(u64::MAX);
                queue.lock().clear_owner_defer(arm.queue_owner);
                Duration::from_millis(20)
            });
            assert!(changed);
            assert!(matches!(result, PermitWait::Granted(_)));
            assert_eq!(queue.lock().status_owner(u64::MAX - 1).unwrap().position, 1);
        }
    }
}

#[test]
fn plain_login_does_not_shorten_an_established_generic_backoff() {
    let arm = SlotArm::new(7, true);
    let command = arm.login_command(false).unwrap();
    arm.arm_explicit_login();
    assert!(arm.login_attempt_is_current(command, 0));
    let began = Instant::now();
    let mut clicked = false;
    let mut frames = 0;
    let result = arm.wait_for_retry(Duration::from_secs(60), Some((command, 0)), &mut || {
        frames += 1;
        if !clicked {
            clicked = true;
            arm.arm_explicit_login();
        } else {
            // Two real frames prove the agreeing command did not end the
            // production 60 s wait; Stop terminates it without sleeping 60 s.
            arm.stop.store(true, Ordering::Relaxed);
        }
        Duration::ZERO
    });
    assert!(!result);
    assert_eq!(frames, 2);
    assert!(began.elapsed() < Duration::from_secs(1));
}

#[test]
fn logout_then_login_clears_release_delay_without_losing_fifo_place() {
    let arm = clean_exit(Instant::now());
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    queue.lock().enqueue_owner(arm.queue_owner, 7);
    queue
        .lock()
        .defer_owner_until(arm.queue_owner, Instant::now() + CLEAN_RELEASE_CADENCE);
    let mut changed = false;
    let result = wait_for_permit(&queue, &statuses, "alice", 7, &arm, &mut || {
        assert!(
            !changed,
            "cancelled pacing must be cleared before another frame"
        );
        changed = true;
        arm.request_logout();
        arm.arm_explicit_login();
        Duration::ZERO
    });
    assert!(changed);
    assert!(matches!(result, PermitWait::Granted(_)));
    assert!(arm
        .clean_release_admission_at(HOST, PORT, Instant::now())
        .is_none());
}
