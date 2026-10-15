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

fn tcp_client(port: u16) -> Client {
    Client::new_with_revision(
        client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        },
        client::client::ClientRevision::R289,
    )
}

fn granted_for_arm(queue: &SharedLoginQueue, arm: &SlotArm, uid: i32) -> GrantedReservation {
    let mut q = queue.lock();
    q.enqueue_owner(arm.queue_owner, uid);
    assert_eq!(
        poll_permit(&mut q, arm.queue_owner, uid, Instant::now()),
        Permit::Grant
    );
    drop(q);
    GrantedReservation::new(queue, arm.queue_owner)
}

#[test]
fn logout_after_grant_before_publication_does_not_charge_spacing_or_settlement() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    arm.request_logout();
    let mut network = false;
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |deadline, cancel| {
            network = true;
            arm.finish_attempt(command, world, deadline, &cancel, &Ok(()));
            Ok::<_, ()>(true)
        },
        |_| None,
    );
    assert_eq!(result, None);
    assert!(!network);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert_eq!(q.head_blocker(99, Instant::now()), None);
}

#[test]
fn old_authorization_cannot_be_refreshed_after_logout_then_login() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    arm.request_logout();
    arm.arm_explicit_login();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let mut connected = false;
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |_, _| {
            connected = true;
            Ok::<_, ()>(())
        },
        |_| None,
    );
    assert_eq!(result, None);
    assert!(!connected);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert_eq!(q.head_blocker(99, Instant::now()), None);
}

#[test]
fn cancelling_published_token_before_start_settles_once_without_connect() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let mut connected = false;
    let result = permit.attempt(
        |cancel| {
            assert!(arm.begin_attempt(command, world, cancel.clone()));
            arm.request_logout();
            assert!(cancel.is_cancelled());
            true
        },
        |deadline, cancel| {
            if cancel.is_cancelled() {
                assert!(!arm.finish_attempt(command, world, deadline, &cancel, &Ok(())));
                return Ok::<_, ()>(false);
            }
            connected = true;
            Ok(true)
        },
        |_| None,
    );
    assert_eq!(result, Some(Ok(false)));
    assert!(!connected);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert!(matches!(
        q.head_blocker(99, Instant::now()),
        Some(AdmitBlocker::Spacing { .. })
    ));
}

#[test]
fn cancelling_after_queue_start_before_connect_does_not_open_socket() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let mut client = tcp_client(listener.local_addr().unwrap().port());
    let backoff = LoginBackoff::new();
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |deadline, cancel| {
            arm.request_logout();
            assert!(cancel.is_cancelled());
            SlotLogin {
                client: &mut client,
                arm: &arm,
                command,
                world_generation: world,
                username: "alice",
                password: "secret",
                reconnect: false,
                backoff: &backoff,
            }
            .run(deadline, cancel, |_| {
                panic!("cancelled before Client::login")
            })
            .0
        },
        |_| None,
    );
    assert!(matches!(result, Some(Ok(false))), "{result:?}");
    assert!(listener.accept().is_err(), "cancelled attempt connected");
    assert_eq!(
        arm.latest_lifecycle_event().unwrap().payload(),
        &SessionEventOut::HandshakeAborted {
            reason: AbortedAttempt::Cancelled
        }
    );
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert!(matches!(
        q.head_blocker(99, Instant::now()),
        Some(AdmitBlocker::Spacing { .. })
    ));
}

#[test]
fn switch_world_after_capture_refuses_unused_grant_without_charge() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    arm.switch_world(5);
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |_, _| panic!("stale world reached Connect"),
        |_: &Result<(), ()>| None,
    );
    assert_eq!(result, None);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert_eq!(q.head_blocker(99, Instant::now()), None);
}

#[test]
fn password_read_does_not_acquire_slot_lock_in_reverse_order() {
    let arm = SlotArm::new(7, true);
    let lifecycle = arm.lifecycle.lock();
    let (sent, received) = std::sync::mpsc::channel();
    let reader = {
        let arm = Arc::clone(&arm);
        thread::spawn(move || sent.send(arm.login_password()).unwrap())
    };
    let result = received.recv_timeout(Duration::from_secs(2));
    drop(lifecycle);
    reader.join().unwrap();
    assert!(
        result.is_ok(),
        "password read waited for K4 under password lock"
    );
}

#[test]
fn logout_interrupts_blocked_login_read_and_closes_before_queue_return() {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (eof_tx, eof_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        accepted_tx.send(()).unwrap();
        let mut byte = [0; 1];
        eof_tx.send(socket.read(&mut byte).unwrap()).unwrap();
    });
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let mut client = tcp_client(port);
    let poster = {
        let arm = Arc::clone(&arm);
        thread::spawn(move || {
            accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            arm.request_logout();
        })
    };
    let began = Instant::now();
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |deadline, cancel| {
            SlotLogin {
                client: &mut client,
                arm: &arm,
                command,
                world_generation: world,
                username: "alice",
                password: "secret",
                reconnect: false,
                backoff: &LoginBackoff::new(),
            }
            .run(deadline, cancel, |_| {})
            .0
        },
        |_| {
            assert_eq!(eof_rx.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
            None
        },
    );
    poster.join().unwrap();
    server.join().unwrap();
    assert!(matches!(result, Some(Ok(false))), "{result:?}");
    assert!(began.elapsed() < Duration::from_secs(5));
    assert_eq!(arm.session_key(), None);
    assert!(client.stream.is_none());
    assert_eq!(
        arm.login_latch_reason(),
        Some(LoginLatchReason::OperatorLogout)
    );
}

#[test]
fn password_revision_change_refuses_old_authorization_without_charging_queue() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let initial = Profile {
        username: "alice".into(),
        password: "first".into(),
        uid: 7,
        settings: vault::ProfileSettings::default(),
    };
    sync_profile_arm(&arm, &initial);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let revised = Profile {
        password: "second".into(),
        ..initial
    };
    sync_profile_arm(&arm, &revised);
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |_, _| panic!("stale credentials cannot reach the network"),
        |_: &Result<(), ()>| None,
    );
    assert_eq!(result, None);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert_eq!(q.head_blocker(99, Instant::now()), None);
}

#[test]
fn non_cancelling_intents_preserve_attempt_currency_and_token() {
    for row in [
        Intent::WorldPreference(IntentWorldChoice::Automatic),
        Intent::StopScript,
        Intent::PauseScript,
        Intent::ResumeScript,
        Intent::StartScript,
        Intent::WatchdogRestart,
        Intent::MemoryMode(MemoryMode::Low),
        Intent::AutoLogin(true),
        Intent::ScriptNeedsSession(true),
        Intent::Login,
        Intent::RelogNow,
    ] {
        let arm = SlotArm::new(7, true);
        let (command, world) = capture_login_authorization(&arm, false).unwrap();
        let cancel = LoginCancel::new();
        assert!(arm.begin_attempt(command, world, cancel.clone()));
        arm.post_intent(row);
        assert!(!cancel.is_cancelled(), "{row:?} fired the token");
        assert!(
            arm.finish_attempt(
                command,
                world,
                Instant::now() + Duration::from_secs(30),
                &cancel,
                &Ok(())
            ),
            "{row:?} changed the currency"
        );
        assert!(arm.session_key().is_some(), "{row:?}");
    }
}

#[test]
fn cancelling_rows_after_unused_grant_abandon_without_settlement() {
    for row in [
        Intent::Logout,
        Intent::HoldLoggedOut,
        Intent::WithdrawLogin,
        Intent::RemoveSlot,
        Intent::ProcessShutdown,
    ] {
        let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
        let arm = SlotArm::new(7, true);
        let (command, world) = capture_login_authorization(&arm, false).unwrap();
        let mut permit = granted_for_arm(&queue, &arm, 7);
        arm.post_intent(row);
        let result = permit.attempt(
            |cancel| arm.begin_attempt(command, world, cancel),
            |_, _| panic!("no socket work for {row:?}"),
            |_: &Result<(), ()>| None,
        );
        assert_eq!(result, None, "{row:?}");
        drop(permit);
        let mut q = queue.lock();
        q.enqueue_owner(99, 8);
        assert_eq!(q.head_blocker(99, Instant::now()), None, "{row:?}");
        assert!(!arm.login_wanted(), "{row:?}");
    }
}

#[test]
fn changed_world_preference_fences_without_cancelling_transport() {
    let arm = SlotArm::new(7, true);
    let command = arm.login_command(false).unwrap();
    let world = arm.world_generation();
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt(command, world, cancel.clone()));
    arm.post_intent(Intent::WorldPreference(IntentWorldChoice::World(5)));
    assert!(
        !cancel.is_cancelled(),
        "preference is not a cancellation row"
    );
    assert!(!arm.finish_attempt(
        command,
        world,
        Instant::now() + Duration::from_secs(30),
        &cancel,
        &Ok(()),
    ));
}

#[test]
fn success_at_deadline_without_queue_poll_does_not_publish_session() {
    let arm = SlotArm::new(7, true);
    let command = arm.login_command(false).unwrap();
    let world = arm.world_generation();
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt(command, world, cancel.clone()));
    let deadline = Instant::now();
    assert!(!arm.finish_attempt(command, world, deadline, &cancel, &Ok(())));
    assert_eq!(arm.session_key(), None);
    assert!(matches!(
        arm.latest_lifecycle_event().unwrap().payload(),
        SessionEventOut::HandshakeAborted {
            reason: AbortedAttempt::LateSuccess
        }
    ));
}

#[test]
fn response_sixteen_after_logout_keeps_server_hold() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        let mut reply = [0; 9];
        reply[8] = 16;
        socket.write_all(&reply).unwrap();
    });
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let command = arm.login_command(false).unwrap();
    let world = arm.world_generation();
    let backoff = LoginBackoff::new();
    let hold = std::cell::Cell::new(None);
    let mut client = tcp_client(port);
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |deadline, cancel| {
            let (result, server_hold) = SlotLogin {
                client: &mut client,
                arm: &arm,
                command,
                world_generation: world,
                username: "alice",
                password: "secret",
                reconnect: false,
                backoff: &backoff,
            }
            .run(deadline, cancel, |_| arm.request_logout());
            hold.set(server_hold);
            result
        },
        |_| hold.get(),
    );
    server.join().unwrap();
    assert!(matches!(result, Some(Ok(false))));
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert!(matches!(
        q.head_blocker(99, Instant::now()),
        Some(AdmitBlocker::ServerHold { .. })
    ));
}

#[test]
fn refused_client_stream_closes_before_queue_retirement() {
    use std::io::{Read, Write};
    for code in [3, 5, 7, 16, 21, -1] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (eof_tx, eof_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut hello = [0; 2];
            socket.read_exact(&mut hello).unwrap();
            let mut reply = [0; 10];
            if code >= 0 {
                reply[8] = code as u8;
                socket
                    .write_all(&reply[..if code == 21 { 10 } else { 9 }])
                    .unwrap();
            } else {
                socket.write_all(&reply[..8]).unwrap();
                socket.shutdown(std::net::Shutdown::Write).unwrap();
            }
            let mut byte = [0; 1];
            eof_tx.send(socket.read(&mut byte).unwrap()).unwrap();
        });
        let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
        let arm = SlotArm::new(7, true);
        let mut permit = granted_for_arm(&queue, &arm, 7);
        let command = arm.login_command(false).unwrap();
        let world = arm.world_generation();
        let mut client = tcp_client(port);
        client.set_external_reconnect_owner(true);
        let backoff = LoginBackoff::new();
        let hold = std::cell::Cell::new(None);
        let result = permit.attempt(
            |cancel| arm.begin_attempt(command, world, cancel),
            |deadline, cancel| {
                let (result, server_hold) = SlotLogin {
                    client: &mut client,
                    arm: &arm,
                    command,
                    world_generation: world,
                    username: "alice",
                    password: "secret",
                    reconnect: false,
                    backoff: &backoff,
                }
                .run(deadline, cancel, |_| {});
                hold.set(server_hold);
                result
            },
            |_| {
                assert_eq!(
                    eof_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
                    0,
                    "socket must close before queue return acknowledgement"
                );
                hold.get()
            },
        );
        assert!(matches!(&result, Some(Err(error)) if error.code == code));
        assert!(!client.ingame);
        assert!(client.stream.is_none());
        server.join().unwrap();
    }
}

#[test]
fn cancelled_late_success_logs_out_and_closes_real_socket_before_retirement() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (eof_tx, eof_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        socket.write_all(&[0; 9 + 8]).unwrap();
        let mut header = [0; 2];
        socket.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 16);
        let mut login = vec![0; header[1] as usize];
        socket.read_exact(&mut login).unwrap();
        socket.write_all(&[2, 0, 0]).unwrap();
        let mut byte = [0; 1];
        eof_tx.send(socket.read(&mut byte).unwrap()).unwrap();
    });
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let command = arm.login_command(false).unwrap();
    let world = arm.world_generation();
    let mut client = tcp_client(port);
    let backoff = LoginBackoff::new();
    let hold = std::cell::Cell::new(None);
    let result = permit.attempt(
        |cancel| arm.begin_attempt(command, world, cancel),
        |deadline, cancel| {
            let (result, server_hold) = SlotLogin {
                client: &mut client,
                arm: &arm,
                command,
                world_generation: world,
                username: "alice",
                password: "secret",
                reconnect: false,
                backoff: &backoff,
            }
            .run(deadline, cancel, |returned| {
                assert!(returned.ingame && returned.stream.is_some());
                arm.request_logout();
            });
            hold.set(server_hold);
            result
        },
        |_| {
            assert_eq!(
                eof_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
                0,
                "late-success close must precede retirement"
            );
            hold.get()
        },
    );
    assert!(matches!(result, Some(Ok(false))));
    assert!(!client.ingame);
    assert!(client.stream.is_none());
    assert_eq!(arm.session_key(), None);
    server.join().unwrap();
}

#[test]
fn session_ended_survives_later_intent_until_consumed() {
    let arm = SlotArm::new(7, true);
    on_login_success(&arm, arm.login_command(false).unwrap());
    let key = arm.session_key().unwrap();
    arm.observe_session_exit(SessionEndCause::ServerLogout, false);
    arm.arm_explicit_login();
    let event = arm.take_lifecycle_event().0.unwrap();
    assert_eq!(event.scope(), Scope::Session(key));
    assert!(matches!(
        event.payload(),
        SessionEventOut::SessionEnded {
            cause: SessionEndCause::ServerLogout,
            ..
        }
    ));
    assert!(matches!(
        arm.latest_lifecycle_event().unwrap().payload(),
        SessionEventOut::IntentApplied { .. }
    ));
}

#[test]
fn newer_session_exit_stays_latest_while_older_boundary_awaits_consumption() {
    let arm = SlotArm::new(7, true);
    on_login_success(&arm, arm.login_command(false).unwrap());
    let first = arm.session_key().unwrap();
    arm.observe_session_exit(SessionEndCause::ServerLogout, false);
    arm.complete_login_for_test();
    let second = arm.session_key().unwrap();
    assert_ne!(first, second);
    arm.observe_session_exit(SessionEndCause::ServerLogout, false);
    assert_eq!(
        arm.latest_lifecycle_event().unwrap().scope(),
        Scope::Session(second)
    );
    assert_eq!(
        arm.take_lifecycle_event().0.unwrap().scope(),
        Scope::Session(first)
    );
    assert_eq!(
        arm.take_lifecycle_event().0.unwrap().scope(),
        Scope::Session(second)
    );
}

#[test]
fn two_unconsumed_session_boundaries_keep_order_and_report_overflow() {
    let arm = SlotArm::new(7, true);
    on_login_success(&arm, arm.login_command(false).unwrap());
    let first = arm.session_key().unwrap();
    arm.observe_session_exit(SessionEndCause::ServerLogout, false);
    arm.complete_login_for_test();
    let second = arm.session_key().unwrap();
    arm.observe_session_exit(SessionEndCause::ReadEof, true);
    arm.post_intent(Intent::StopScript);
    assert_eq!(
        arm.take_lifecycle_event().0.unwrap().scope(),
        Scope::Session(first)
    );
    assert_eq!(
        arm.take_lifecycle_event().0.unwrap().scope(),
        Scope::Session(second)
    );

    for _ in 0..SESSION_BOUNDARY_LIMIT + 2 {
        arm.complete_login_for_test();
        arm.observe_session_exit(SessionEndCause::ReadEof, true);
    }
    let (first_retained, missed) = arm.take_lifecycle_event();
    assert!(matches!(
        first_retained.unwrap().payload(),
        SessionEventOut::SessionEnded { .. }
    ));
    assert_eq!(missed, 2);
    for _ in 1..SESSION_BOUNDARY_LIMIT {
        let (event, missed) = arm.take_lifecycle_event();
        assert!(matches!(
            event.unwrap().payload(),
            SessionEventOut::SessionEnded { .. }
        ));
        assert_eq!(missed, 0);
    }
}

fn assert_transport_event(
    error: LoginError,
    stage: client::client::LoginStage,
    kind: std::io::ErrorKind,
    expected: TransportFailure,
) {
    let transport = error
        .transport
        .as_ref()
        .expect("real client transport error");
    assert_eq!(transport.stage, stage);
    assert_eq!(transport.kind, kind);
    let arm = SlotArm::new(7, true);
    let command = arm.login_command(false).unwrap();
    let world_generation = arm.world_generation();
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt(command, world_generation, cancel.clone()));
    let result = Err(error);
    assert!(arm.finish_attempt(
        command,
        world_generation,
        Instant::now() + Duration::from_secs(1),
        &cancel,
        &result,
    ));
    let event = arm.latest_lifecycle_event().unwrap();
    assert_eq!(
        event.scope(),
        api::identity::Scope::Slot(arm.lifecycle.lock().session.slot())
    );
    assert_eq!(
        event.payload(),
        &SessionEventOut::HandshakeAborted {
            reason: AbortedAttempt::Transport(expected),
        }
    );
    assert_eq!(arm.session_key(), None);
}

#[test]
fn real_client_connect_refusal_reports_connect_transport_event() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let error = tcp_client(port)
        .login("alice", "secret", false)
        .unwrap_err();
    assert_transport_event(
        error,
        client::client::LoginStage::Connect,
        std::io::ErrorKind::ConnectionRefused,
        TransportFailure::Connect,
    );
}

#[test]
fn real_client_partial_seed_timeout_reports_read_transport_event() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, held) = std::sync::mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        socket.write_all(&[0; 9]).unwrap();
        held.recv_timeout(Duration::from_secs(2)).unwrap();
    });
    let mut client = tcp_client(port);
    client.set_login_deadline(Instant::now() + Duration::from_millis(150));
    let error = client.login("alice", "secret", false).unwrap_err();
    release.send(()).unwrap();
    server.join().unwrap();
    assert_transport_event(
        error,
        client::client::LoginStage::ReadSeed,
        std::io::ErrorKind::TimedOut,
        TransportFailure::Read,
    );
}

#[test]
fn real_client_partial_seed_eof_reports_closed_transport_event() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        socket.write_all(&[0; 9 + 2]).unwrap();
    });
    let error = tcp_client(port)
        .login("alice", "secret", false)
        .unwrap_err();
    server.join().unwrap();
    assert_transport_event(
        error,
        client::client::LoginStage::ReadSeed,
        std::io::ErrorKind::UnexpectedEof,
        TransportFailure::Closed,
    );
}

#[cfg(unix)]
#[test]
fn real_client_adopted_reset_socket_reports_write_transport_event() {
    use client::io::client_stream::ClientStream;
    use std::net::TcpListener;
    use std::os::fd::AsRawFd;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut stream = ClientStream::connect("127.0.0.1", port).unwrap();
    let (accepted, _) = listener.accept().unwrap();
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    // SAFETY: `accepted` owns a live socket for the duration of this call,
    // and `linger` is initialized with the exact SO_LINGER layout and size.
    // The result and the peer's observed reset below verify the invariant.
    let set = unsafe {
        libc::setsockopt(
            accepted.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&linger as *const libc::linger).cast(),
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        )
    };
    assert_eq!(set, 0, "SO_LINGER: {}", std::io::Error::last_os_error());
    drop(accepted);
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match stream.available() {
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Ok(0) if Instant::now() < until => thread::yield_now(),
            result => panic!("expected local server reset before the login write: {result:?}"),
        }
    }
    let mut client = tcp_client(port);
    client.stream = Some(stream);
    client.baton = true;
    client.set_login_deadline(Instant::now() + Duration::from_secs(1));
    let error = client.login("alice", "secret", true).unwrap_err();
    let transport = error.transport.as_ref().unwrap();
    assert!(matches!(
        transport.kind,
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
    ));
    let kind = transport.kind;
    assert_transport_event(
        error,
        client::client::LoginStage::SendHello,
        kind,
        TransportFailure::Write,
    );
}

#[test]
fn real_response_sixteen_publishes_server_hold_with_exact_deadline() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        let mut reply = [0; 9];
        reply[8] = 16;
        socket.write_all(&reply).unwrap();
    });
    let error = tcp_client(port)
        .login("alice", "secret", false)
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(error.code, 16, "the real client read response 16");
    let backoff = LoginBackoff::new();
    let hold = server_hold(&Err::<(), _>(error), &backoff).unwrap();
    let at = Instant::now();
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    queue.lock().hold_for(at, hold);
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
    let arm = SlotArm::new(7, true);
    arm.prepared();
    enqueue_queue_place(&queue, &statuses, "alice", 7, &arm);
    let event = arm.latest_lifecycle_event().unwrap();
    assert_eq!(
        event.scope(),
        Scope::Slot(arm.lifecycle.lock().session.slot())
    );
    assert_eq!(
        event.payload(),
        &SessionEventOut::Queued {
            place: 1,
            total: 1,
            blocker: Some(QueueBlocker::ServerHold {
                until: Deadline::at(at + hold),
            }),
        }
    );
    assert_eq!(
        statuses.lock().unwrap()[0].queue_blocker,
        Some(AdmitBlocker::ServerHold { until: at + hold })
    );
}

fn poll_permit(queue: &mut LoginQueue, owner: u64, uid: i32, now: Instant) -> Permit {
    let polled = queue.poll_owner(owner, uid, now);
    assert!(polled.cancels.is_empty());
    polled.permit
}

fn queue_attempt(queue: &mut LoginQueue, arm: &SlotArm, now: Instant) {
    queue.enqueue_owner(arm.queue_owner, 7);
    if let Some(until) = arm.clean_release_admission_at(HOST, PORT, now) {
        queue.defer_owner_until(arm.queue_owner, until);
    }
    assert_eq!(poll_permit(queue, arm.queue_owner, 7, now), Permit::Grant);
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
        poll_permit(&mut queue, arm.queue_owner, 7, start),
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
fn skipped_device_owner_publishes_its_device_limit_reason() {
    let now = Instant::now();
    let mut queue = LoginQueue::new(Duration::ZERO, 29, Duration::from_secs(60));
    let owner = 1;
    for _ in 0..4 {
        queue.enqueue_owner(owner, 7);
        assert_eq!(poll_permit(&mut queue, owner, 7, now), Permit::Grant);
        assert!(queue
            .start_attempt(owner, now, LoginCancel::new())
            .is_some());
        assert!(queue.acknowledge_login_return(owner, now));
    }

    queue.enqueue_owner(owner, 7);
    queue.enqueue_owner(2, 8);
    assert_eq!(poll_permit(&mut queue, 2, 8, now), Permit::Grant);
    assert!(queue.start_attempt(2, now, LoginCancel::new()).is_some());
    queue.enqueue_owner(3, 9);

    let mut rows = vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }];
    let event = publish_queue_place(&queue, &mut rows, "alice", owner).unwrap();
    assert_eq!(
        (rows[0].queue_position, rows[0].queue_total),
        (1, 2),
        "the device-blocked owner keeps its queue place"
    );
    assert!(
        matches!(
            rows[0].queue_blocker,
            Some(host::login_queue::AdmitBlocker::DeviceBudget {
                until: Some(until)
            }) if until > Instant::now()
        ),
        "the skipped owner publishes the device cooldown reason"
    );
    assert!(matches!(
        event,
        SessionEventOut::Queued {
            place: 1,
            total: 2,
            blocker: Some(QueueBlocker::DeviceBudget { until: Some(_) }),
        }
    ));
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
        poll_permit(&mut queue, arm.queue_owner, 7, start),
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
            3 => arm.switch_world(2),
            4 => {
                arm.uid.store(8, Ordering::Relaxed);
            }
            5 => {
                arm.post_intent(Intent::LoginIdentityChanged);
            }
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
    assert_eq!(poll_permit(&mut queue, u64::MAX, 9, now), Permit::Grant);
    queue
        .start_attempt(u64::MAX, now, LoginCancel::new())
        .unwrap();
    let later = now + CLEAN_RELEASE_CADENCE;
    assert!(
        matches!(
            poll_permit(&mut queue, arm.queue_owner, 7, later),
            Permit::Wait(_)
        ),
        "still one handshake even after release deadline"
    );
    assert!(queue.acknowledge_login_return(u64::MAX, later));
    assert_eq!(
        poll_permit(&mut queue, arm.queue_owner, 7, later),
        Permit::Grant
    );
    queue
        .start_attempt(arm.queue_owner, later, LoginCancel::new())
        .unwrap();
    queue.enqueue_owner(u64::MAX - 1, 10);
    assert!(matches!(
        poll_permit(&mut queue, u64::MAX - 1, 10, later),
        Permit::Wait(_)
    ));
    assert!(queue.acknowledge_login_return(arm.queue_owner, later));
    assert_eq!(
        poll_permit(&mut queue, u64::MAX - 1, 10, later),
        Permit::Grant
    );
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
                2 => arm.switch_world(2),
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
            poll_permit(&mut queue.lock(), u64::MAX, 9, Instant::now()),
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
fn agreeing_commands_keep_fifo_place_until_grant_with_and_without_release_wait() {
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
                    2 => sync_profile_arm(
                        &arm,
                        &Profile {
                            username: "alice".into(),
                            password: "new-password".into(),
                            uid: 7,
                            settings: vault::ProfileSettings::default(),
                        },
                    ),
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
fn password_save_during_fifo_wait_refuses_grant_and_rejoins_at_back() {
    for release in [false, true] {
        let arm = SlotArm::new(7, true);
        if release {
            arm.record_session_exit_at(true, HOST, PORT, Instant::now());
        }
        let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        // The worker captures its authorization before entering the FIFO.
        let (command, world) = capture_login_authorization(&arm, false).unwrap();
        queue.lock().enqueue_owner(u64::MAX, 9);
        enqueue_queue_place(&queue, &statuses, "alice", 7, &arm);
        queue.lock().enqueue_owner(u64::MAX - 1, 10);
        let mut changed = false;
        let wait = wait_for_permit(&queue, &statuses, "alice", 7, &arm, &mut || {
            if !changed {
                changed = true;
                sync_profile_arm(
                    &arm,
                    &Profile {
                        username: "alice".into(),
                        password: "new-password".into(),
                        uid: 7,
                        settings: vault::ProfileSettings::default(),
                    },
                );
                queue.lock().leave_owner(u64::MAX);
                queue.lock().clear_owner_defer(arm.queue_owner);
            }
            Duration::from_millis(20)
        });
        assert!(changed);
        let PermitWait::Granted(mut permit) = wait else {
            panic!("release={release}: expected the original FIFO grant");
        };
        assert!(
            permit
                .attempt(
                    |cancel| arm.begin_attempt(command, world, cancel),
                    |_, _| Ok::<bool, ()>(true),
                    |_| None,
                )
                .is_none(),
            "release={release}: old password authorization must be refused"
        );
        arm.clear_unused_attempt();
        drop(permit);
        if release {
            assert!(
                arm.clean_release_admission_at(HOST, PORT, Instant::now())
                    .is_some(),
                "a password save does not count as a clean logout"
            );
        }
        let (_command, _world) = capture_login_authorization(&arm, false).unwrap();
        enqueue_queue_place(&queue, &statuses, "alice", 7, &arm);
        assert_eq!(
            queue.lock().status_owner(u64::MAX - 1).unwrap().position,
            1,
            "release={release}: the third owner keeps its place"
        );
        assert_eq!(
            queue.lock().status_owner(arm.queue_owner).unwrap().position,
            2,
            "release={release}: the slot rejoins behind the third owner"
        );
    }
}

#[test]
fn intent_cancellation_fires_only_after_k4_unlock() {
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt(command, world, cancel.clone()));
    arm.assert_cancel_order.store(true, Ordering::Relaxed);
    arm.request_logout();
    assert!(cancel.is_cancelled());
}

#[test]
fn identity_cancellation_fires_only_after_k4_unlock() {
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt(command, world, cancel.clone()));
    arm.assert_cancel_order.store(true, Ordering::Relaxed);
    arm.sync_login_identity(7, "new-password");
    assert!(cancel.is_cancelled());
}

#[test]
fn world_error_cancellation_fires_only_after_k4_unlock() {
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt(command, world, cancel.clone()));
    arm.assert_cancel_order.store(true, Ordering::Relaxed);
    arm.hold_login_on_error();
    assert!(cancel.is_cancelled());
}

#[test]
fn queue_expiry_cancellation_fires_only_after_k1_unlock() {
    let arm = SlotArm::new(7, true);
    let queue = Arc::new(QueueMutex::new(
        LoginQueue::new(Duration::ZERO, 29, Duration::from_secs(60))
            .with_handshake_hold(Duration::from_millis(1)),
    ));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let expired_at = Instant::now() - Duration::from_secs(2);
    let cancel = LoginCancel::new();
    {
        let mut q = queue.lock();
        q.enqueue_owner(u64::MAX, 9);
        assert_eq!(q.poll_owner(u64::MAX, 9, expired_at).permit, Permit::Grant);
        assert!(q
            .start_attempt(u64::MAX, expired_at, cancel.clone())
            .is_some());
        q.enqueue_owner(arm.queue_owner, 7);
    }
    arm.assert_cancel_order.store(true, Ordering::Relaxed);
    let mut retired = false;
    let wait = wait_for_permit(&queue, &statuses, "alice", 7, &arm, &mut || {
        if !retired {
            assert!(cancel.is_cancelled());
            retired = queue
                .lock()
                .acknowledge_login_return(u64::MAX, Instant::now());
        }
        Duration::from_millis(1)
    });
    assert!(matches!(wait, PermitWait::Granted(_)));
    assert!(retired);
    assert!(cancel.is_cancelled());
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
