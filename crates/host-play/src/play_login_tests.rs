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
            cache_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            members: true,
            lowmem: false,
        },
        client::client::ClientRevision::R289,
    )
}

// The host hook is process-scoped; isolate hosted cases from sibling native-client tests.
fn run_hosted_test_in_child(test_name: &str, marker: &str) -> bool {
    if std::env::var_os(marker).is_some() {
        return false;
    }
    let exact_name = format!("play_login::{test_name}");
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", &exact_name, "--nocapture", "--test-threads=1"])
        .env(marker, "1")
        .output()
        .expect("start isolated hosted login test");
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("running 1 test\n"),
        "isolated hosted login test {exact_name} failed or ran zero tests\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

struct HostedNetworkFixture {
    service: Arc<client_adapter::service::NetworkService>,
    stop_maintenance: Arc<std::sync::atomic::AtomicBool>,
    maintenance: Option<thread::JoinHandle<()>>,
}

impl HostedNetworkFixture {
    fn start() -> Self {
        let driver = client_adapter::service::NetworkService::start_composition()
            .expect("start hosted test network service");
        let service = Arc::clone(driver.service());
        let stop_maintenance = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = Arc::clone(&stop_maintenance);
        let maintenance = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                driver.maintain(Instant::now());
                thread::park_timeout(Duration::from_millis(10));
            }
            driver.maintain(Instant::now());
        });
        Self {
            service,
            stop_maintenance,
            maintenance: Some(maintenance),
        }
    }

    fn prepared_attempt(&self, arm: &SlotArm, port: u16) -> PreparedGameAttempt {
        let park = arm.park();
        let source = client_adapter::cancel::CancelSource::new(Arc::clone(&park));
        let token = source.child().expect("test attempt generation");
        let retirement = Arc::new(AttemptRetire::new(token.generation()));
        let mut addresses = client_adapter::resolver::ResolvedAddresses::new();
        assert!(addresses.push(std::net::SocketAddr::from(([127, 0, 0, 1], port))));
        PreparedGameAttempt::new(
            Arc::clone(self.service.reactor()),
            park,
            token,
            retirement,
            client_adapter::host_network::GameTransport::Tcp,
            "127.0.0.1",
            port,
            addresses,
        )
        .expect("prepare hosted test transport")
    }

    fn run_login(
        &self,
        permit: &mut GrantedReservation,
        arm: &SlotArm,
        client: &mut Client,
        command: IntentCommand,
        world_generation: u64,
        after_return: impl FnOnce(&Client),
    ) -> Option<LoginAttemptReturn<(), LoginError>> {
        let backoff = LoginBackoff::new();
        let backoff_ref = &backoff;
        let prepared = self.prepared_attempt(arm, client.config.port);
        permit.attempt(
            prepared,
            || false,
            |cancel, generation| {
                arm.begin_attempt_with_generation(command, world_generation, cancel, generation)
            },
            move |deadline, cancel| {
                SlotLogin {
                    client,
                    arm,
                    command,
                    world_generation,
                    username: "alice",
                    password: "secret",
                    reconnect: false,
                    backoff: backoff_ref,
                }
                .run(deadline, cancel, after_return)
                .0
            },
            |result| server_hold(result, &backoff),
        )
    }
}

impl Drop for HostedNetworkFixture {
    fn drop(&mut self) {
        self.stop_maintenance.store(true, Ordering::Release);
        if let Some(maintenance) = self.maintenance.take() {
            maintenance.thread().unpark();
            maintenance.join().expect("hosted service maintainer");
        }
    }
}

fn run_hosted_client_login(
    network: &HostedNetworkFixture,
    arm: &SlotArm,
    client: &mut Client,
    port: u16,
    reconnect: bool,
) -> Result<(), LoginError> {
    let prepared = network.prepared_attempt(arm, port);
    let cancel = client::io::client_stream::LoginCancel::new();
    cancel.set_host_attempt_identity(prepared.hook_identity(arm.queue_owner, None));
    client.set_login_cancel(cancel);
    client.set_login_deadline(Instant::now() + Duration::from_secs(3));
    client_adapter::host_network::with_prepared_game_attempt(prepared, || {
        client.login("alice", "secret", reconnect)
    })
}

// Bound accept so a failed host fence cannot strand a test server indefinitely.
fn accept_local_with_timeout(
    listener: &std::net::TcpListener,
) -> std::io::Result<(std::net::TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::park_timeout(Duration::from_millis(10));
            }
            Ok((stream, address)) => {
                // BSD and macOS accepted sockets inherit the listener's
                // non-blocking mode; the fixtures read them blocking.
                stream.set_nonblocking(false)?;
                return Ok((stream, address));
            }
            result => return result,
        }
    }
}

fn spawn_local_login_server(
    listener: std::net::TcpListener,
    response: u8,
    release: std::sync::mpsc::Receiver<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        use std::io::{Read, Write};

        let (mut socket, _) = accept_local_with_timeout(&listener).expect("accept login client");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        assert_eq!(hello[0], 14, "client sent the login-server hello");
        socket.write_all(&[0; 9]).unwrap();
        socket.write_all(&[0; 8]).unwrap();

        let mut login_header = [0; 2];
        socket.read_exact(&mut login_header).unwrap();
        assert_eq!(login_header[0], 16, "fresh login uses opcode 16");
        socket.write_all(&[response]).unwrap();
        if response == 2 {
            socket.write_all(&[0, 0]).unwrap();
        }
        release
            .recv_timeout(Duration::from_secs(5))
            .expect("test releases accepted client socket");
    })
}

#[test]
fn direct_client_login_uses_production_admission_and_commit() {
    if run_hosted_test_in_child(
        "release_tests::direct_client_login_uses_production_admission_and_commit",
        "HOST_PLAY_DIRECT_LOGIN_CHILD",
    ) {
        return;
    }
    let _network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = spawn_local_login_server(listener, 2, release_rx);
    let mut client = tcp_client(port);
    let mut login = DirectClientLogin::new(&client);

    login
        .login(&mut client, "alice", "secret", false)
        .expect("hosted direct login succeeds");
    assert!(client.ingame, "accepted hosted login commits the session");
    assert!(
        client.has_stream(),
        "committed client retains its hosted stream"
    );

    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn direct_client_login_preserves_typed_server_refusal() {
    if run_hosted_test_in_child(
        "release_tests::direct_client_login_preserves_typed_server_refusal",
        "HOST_PLAY_DIRECT_LOGIN_CHILD",
    ) {
        return;
    }
    let _network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = spawn_local_login_server(listener, 5, release_rx);
    let mut client = tcp_client(port);
    let mut login = DirectClientLogin::new(&client);

    let error = login
        .login(&mut client, "alice", "secret", false)
        .expect_err("server refusal remains an error");
    assert_eq!(error.code, 5);
    assert!(error.transport.is_none(), "the server response stays typed");

    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn direct_client_login_refuses_unprepared_hosted_client_call() {
    if run_hosted_test_in_child(
        "release_tests::direct_client_login_refuses_unprepared_hosted_client_call",
        "HOST_PLAY_DIRECT_LOGIN_CHILD",
    ) {
        return;
    }
    let _network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = tcp_client(port);

    let error = client
        .login("alice", "secret", false)
        .expect_err("installed host network refuses a login with no attempt identity");
    let transport = error.transport.expect("host login failure is typed");
    assert_eq!(error.code, -1);
    assert_eq!(transport.stage, client::client::LoginStage::Connect);
    assert_eq!(transport.kind, std::io::ErrorKind::NotConnected);
    assert!(
        transport
            .detail
            .contains("hosted login requires an attempt identity"),
        "unexpected diagnostic: {}",
        transport.detail
    );
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
fn test_cells(permit: &GrantedReservation, binding: u64) -> QueueAttemptCells {
    synthetic_queue_attempt(permit.owner, binding)
}

fn finish_test_login(
    arm: &SlotArm,
    client: &mut Client,
    command: IntentCommand,
    world_generation: u64,
    outcome: LoginAttemptReturn<(), LoginError>,
) -> Result<SlotLoginOutcome, LoginError> {
    let backoff = LoginBackoff::new();
    SlotLogin {
        client,
        arm,
        command,
        world_generation,
        username: "alice",
        password: "secret",
        reconnect: false,
        backoff: &backoff,
    }
    .finish_after_q(outcome)
}

#[test]
fn invalidating_intent_cancels_active_key_fetch_outside_k4() {
    let arm = SlotArm::new(7, true);
    let cancel = LoginCancel::new();
    let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_unlocked = Arc::new(AtomicBool::new(false));
    let callback_arm = Arc::clone(&arm);
    let callback_count_ref = Arc::clone(&callback_count);
    let callback_unlocked_ref = Arc::clone(&callback_unlocked);
    cancel.register_cancel_callback(move || {
        callback_count_ref.fetch_add(1, Ordering::Relaxed);
        callback_unlocked_ref.store(
            callback_arm.lifecycle.try_lock().is_some(),
            Ordering::Relaxed,
        );
    });

    assert!(arm.publish_key_fetch_cancel(&cancel));
    let receipt = arm.post_intent(Intent::Logout).expect("slot remains open");
    assert!(receipt.invalidated_attempt());
    assert!(cancel.is_cancelled());
    assert_eq!(callback_count.load(Ordering::Relaxed), 1);
    assert!(callback_unlocked.load(Ordering::Relaxed));
    arm.clear_key_fetch_cancel();
    assert!(arm.key_fetch_cancel.lock().is_none());
}

#[test]
fn slot_stop_cancels_active_key_fetch_once_outside_k4() {
    let arm = SlotArm::new(7, true);
    let cancel = LoginCancel::new();
    let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_unlocked = Arc::new(AtomicBool::new(false));
    let callback_arm = Arc::clone(&arm);
    let callback_count_ref = Arc::clone(&callback_count);
    let callback_unlocked_ref = Arc::clone(&callback_unlocked);
    cancel.register_cancel_callback(move || {
        callback_count_ref.fetch_add(1, Ordering::Relaxed);
        callback_unlocked_ref.store(
            callback_arm.lifecycle.try_lock().is_some(),
            Ordering::Relaxed,
        );
    });

    assert!(arm.publish_key_fetch_cancel(&cancel));
    arm.post_intent(Intent::RemoveSlot)
        .expect("remove intent is accepted");
    assert!(arm.stop.load(Ordering::Acquire));
    assert!(cancel.is_cancelled());
    assert_eq!(callback_count.load(Ordering::Relaxed), 1);
    assert!(callback_unlocked.load(Ordering::Relaxed));
    arm.clear_key_fetch_cancel();
    assert!(arm.key_fetch_cancel.lock().is_none());
}

#[test]
fn prequeue_cancel_is_exact_and_invalidating_intents_fire_outside_k4() {
    let arm = SlotArm::new(7, true);
    let (command, world_generation) = capture_login_authorization(&arm, false).unwrap();
    let first = arm.new_cancel_token().expect("first prequeue token");
    assert!(arm.publish_prequeue_cancel(first.clone(), command, world_generation));
    arm.clear_prequeue_cancel(&first);

    let current = arm.new_cancel_token().expect("current prequeue token");
    assert!(arm.publish_prequeue_cancel(current.clone(), command, world_generation,));
    arm.clear_prequeue_cancel(&first);
    assert!(
        arm.lifecycle
            .lock()
            .prequeue_cancel
            .as_ref()
            .is_some_and(|pending| pending.same_attempt(&current)),
        "clearing an older prequeue attempt must not remove the current one"
    );

    arm.assert_cancel_order.store(true, Ordering::Relaxed);
    let receipt = arm.post_intent(Intent::Logout).expect("slot remains open");
    assert!(receipt.invalidated_attempt());
    assert!(current.is_cancelled());
    assert!(arm.lifecycle.lock().prequeue_cancel.is_none());

    let stopped = SlotArm::new(8, true);
    let (command, world_generation) = capture_login_authorization(&stopped, false).unwrap();
    let pending = stopped.new_cancel_token().expect("stop prequeue token");
    assert!(stopped.publish_prequeue_cancel(pending.clone(), command, world_generation));
    stopped.assert_cancel_order.store(true, Ordering::Relaxed);
    stopped
        .post_intent(Intent::RemoveSlot)
        .expect("remove intent is accepted");
    assert!(stopped.stop.load(Ordering::Acquire));
    assert!(pending.is_cancelled());
    assert!(stopped.lifecycle.lock().prequeue_cancel.is_none());
}

#[test]
fn identity_change_cancels_prequeue_dns_token_outside_k4() {
    let arm = SlotArm::new(7, true);
    let (command, world_generation) = capture_login_authorization(&arm, false).unwrap();
    let token = arm.new_cancel_token().expect("prequeue cancellation token");
    assert!(arm.publish_prequeue_cancel(token.clone(), command, world_generation));
    arm.assert_cancel_order.store(true, Ordering::Relaxed);

    arm.sync_login_identity(8, "updated-password");

    assert!(token.is_cancelled());
    assert!(arm.lifecycle.lock().prequeue_cancel.is_none());
    assert_eq!(arm.uid.load(Ordering::Acquire), 8);
}

#[test]
fn prestart_handshake_abort_requires_the_exact_active_generation() {
    let arm = SlotArm::new(7, true);
    let (command, world_generation) = capture_login_authorization(&arm, false).unwrap();
    let token = arm.new_cancel_token().expect("prequeue cancellation token");
    let generation = token.generation();
    assert!(arm.publish_prequeue_cancel(token, command, world_generation));
    let cancel = LoginCancel::new();
    assert!(arm.begin_attempt_with_generation(command, world_generation, cancel, generation,));

    assert!(arm.lifecycle.lock().prequeue_cancel.is_none());

    arm.publish_handshake_aborted(
        AbortedAttempt::Deadline,
        command,
        world_generation,
        generation + 1,
    );
    assert!(matches!(
        arm.latest_lifecycle_event().unwrap().payload(),
        SessionEventOut::AttemptStarted { .. }
    ));

    arm.publish_handshake_aborted(
        AbortedAttempt::Deadline,
        command,
        world_generation,
        generation,
    );
    assert_eq!(
        arm.latest_lifecycle_event().unwrap().payload(),
        &SessionEventOut::HandshakeAborted {
            reason: AbortedAttempt::Deadline,
        }
    );
    assert!(arm.lifecycle.lock().attempt_generation.is_none());
}

#[test]
fn stop_cancels_prequeue_wait_and_withdraws_its_fifo_place() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let arm = SlotArm::new(7, true);
    let (command, world_generation) = capture_login_authorization(&arm, false).unwrap();
    let token = arm.new_cancel_token().expect("prequeue cancellation token");
    assert!(arm.publish_prequeue_cancel(token.clone(), command, world_generation));
    enqueue_queue_place(&queue, &statuses, "alice", 7, &arm);

    arm.post_intent(Intent::RemoveSlot)
        .expect("slot removal is accepted");
    assert!(token.is_cancelled());
    let result =
        wait_for_permit_with_cancel(&queue, &statuses, "alice", 7, &arm, &token, &mut || {
            Duration::from_millis(1)
        });

    assert!(matches!(result, PermitWait::Cancelled));
    assert!(queue.lock().status_owner(arm.queue_owner).is_none());
}

/// The ready session carries the key, the readiness and the world of one
/// session: absent until that session's scene is ready, bound to the world
/// configured for its handshake (not to a world configured later), and
/// absent again once it ends.
#[test]
fn the_ready_session_binds_its_key_and_world_to_one_session() {
    let arm = SlotArm::new(7, true);
    arm.configure_world_for_test(1);
    arm.complete_login_for_test();
    let first = arm.session_key().expect("a session");
    assert_eq!(arm.ready_session(), None, "not ready before its scene");
    arm.scene_ready_for_test();
    assert_eq!(arm.ready_session(), Some((first, Some(1))));
    // The next login's world is configured while this session runs.
    arm.configure_world_for_test(2);
    assert_eq!(arm.ready_session(), Some((first, Some(1))));

    arm.switch_world(2);
    arm.complete_login_for_test();
    let second = arm.session_key().expect("a new session");
    assert_ne!(second, first);
    assert_eq!(arm.ready_session(), None, "the new session is not ready");
    arm.scene_ready_for_test();
    assert_eq!(arm.ready_session(), Some((second, Some(2))));

    arm.observe_session_exit(SessionEndCause::ServerLogout, false);
    assert_eq!(arm.ready_session(), None, "no session");
}

#[test]
fn logout_after_grant_before_publication_does_not_charge_spacing_or_settlement() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    arm.request_logout();
    let mut network = false;
    let cells = test_cells(&permit, 1);
    let result = permit.attempt_with_test_cells(
        cells,
        |cancel, generation| arm.begin_attempt_with_generation(command, world, cancel, generation),
        |deadline, cancel| {
            network = true;
            arm.finish_attempt(command, world, deadline, &cancel, &Ok(()));
            Ok::<_, ()>(true)
        },
        |_| None,
    );
    assert!(result.is_none());
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
    let cells = test_cells(&permit, 1);
    let result = permit.attempt_with_test_cells(
        cells,
        |cancel, generation| arm.begin_attempt_with_generation(command, world, cancel, generation),
        |_, _| {
            connected = true;
            Ok::<_, ()>(())
        },
        |_| None,
    );
    assert!(result.is_none());
    assert!(!connected);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert_eq!(q.head_blocker(99, Instant::now()), None);
}

#[test]
fn cancelling_published_attempt_before_k1_start_does_not_charge_queue_or_connect() {
    let queue: SharedLoginQueue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let arm = SlotArm::new(7, true);
    let (command, world) = capture_login_authorization(&arm, false).unwrap();
    let mut permit = granted_for_arm(&queue, &arm, 7);
    let mut login_called = false;
    let cells = test_cells(&permit, 1);
    let result = permit.attempt_with_test_cells(
        cells,
        |cancel, generation| {
            assert!(arm.begin_attempt_with_generation(command, world, cancel.clone(), generation));
            arm.request_logout();
            assert!(cancel.is_cancelled());
            true
        },
        |_, cancel| {
            login_called = true;
            assert!(cancel.is_cancelled());
            Ok::<_, ()>(false)
        },
        |_| None,
    );
    assert!(result.is_none());
    assert!(!login_called);
    drop(permit);
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert_eq!(q.head_blocker(99, Instant::now()), None);
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
    let cells = test_cells(&permit, 1);
    let result = permit.attempt_with_test_cells(
        cells,
        |cancel, generation| arm.begin_attempt_with_generation(command, world, cancel, generation),
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
    let outcome = result.expect("started attempt returns exact Q facts");
    assert!(
        outcome.acknowledge_after_physical_release(Instant::now()),
        "synthetic attempt opened no socket, so its physical release is immediate"
    );
    assert!(matches!(
        finish_test_login(&arm, &mut client, command, world, outcome),
        Ok(SlotLoginOutcome::Aborted(AbortedAttempt::Cancelled))
    ));
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
    let cells = test_cells(&permit, 1);
    let result = permit.attempt_with_test_cells(
        cells,
        |cancel, generation| arm.begin_attempt_with_generation(command, world, cancel, generation),
        |_, _| panic!("stale world reached Connect"),
        |_: &Result<(), ()>| None,
    );
    assert!(result.is_none());
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
    let result = received.recv_timeout(Duration::from_secs(60));
    drop(lifecycle);
    reader.join().unwrap();
    assert!(
        result.is_ok(),
        "password read waited for K4 under password lock"
    );
}

#[test]
fn logout_interrupts_blocked_login_read_and_closes_before_retirement_ack() {
    if run_hosted_test_in_child(
        "release_tests::logout_interrupts_blocked_login_read_and_closes_before_retirement_ack",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::Read;
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (eof_tx, eof_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(60)))
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
            accepted_rx.recv_timeout(Duration::from_secs(60)).unwrap();
            arm.request_logout();
        })
    };
    let began = Instant::now();
    let result = network
        .run_login(&mut permit, &arm, &mut client, command, world, |_| {})
        .expect("started attempt returns exact Q facts");
    assert!(matches!(
        finish_test_login(&arm, &mut client, command, world, result),
        Ok(SlotLoginOutcome::Aborted(AbortedAttempt::Cancelled))
    ));
    assert_eq!(eof_rx.recv_timeout(Duration::from_secs(60)).unwrap(), 0);
    poster.join().unwrap();
    server.join().unwrap();
    assert!(began.elapsed() < Duration::from_secs(60));
    assert_eq!(arm.session_key(), None);
    assert!(!client.has_stream());
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
    let cells = test_cells(&permit, 1);
    let result = permit.attempt_with_test_cells(
        cells,
        |cancel, generation| arm.begin_attempt_with_generation(command, world, cancel, generation),
        |_, _| panic!("stale credentials cannot reach the network"),
        |_: &Result<(), ()>| None,
    );
    assert!(result.is_none());
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
        let cells = test_cells(&permit, 1);
        let result = permit.attempt_with_test_cells(
            cells,
            |cancel, generation| {
                arm.begin_attempt_with_generation(command, world, cancel, generation)
            },
            |_, _| panic!("no socket work for {row:?}"),
            |_: &Result<(), ()>| None,
        );
        assert!(result.is_none(), "{row:?}");
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
fn revocation_precedes_deadline_for_success_and_failure() {
    for result in [Ok(()), Err(LoginError::response(16, "", ""))] {
        let arm = SlotArm::new(7, true);
        let command = arm.login_command(false).unwrap();
        let world = arm.world_generation();
        let cancel = LoginCancel::new();
        assert!(arm.begin_attempt(command, world, cancel.clone()));
        cancel.cancel();
        assert!(!arm.finish_attempt(command, world, Instant::now(), &cancel, &result,));
        assert_eq!(arm.session_key(), None);
        assert!(matches!(
            arm.latest_lifecycle_event().unwrap().payload(),
            SessionEventOut::HandshakeAborted {
                reason: AbortedAttempt::Cancelled
            }
        ));
    }
}

#[test]
fn response_sixteen_after_logout_keeps_server_hold() {
    if run_hosted_test_in_child(
        "release_tests::response_sixteen_after_logout_keeps_server_hold",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::{Read, Write};
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
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
    let mut client = tcp_client(port);
    let outcome = network
        .run_login(&mut permit, &arm, &mut client, command, world, |_| {
            arm.request_logout()
        })
        .expect("started attempt returns exact Q facts");
    assert!(matches!(
        finish_test_login(&arm, &mut client, command, world, outcome),
        Ok(SlotLoginOutcome::Aborted(AbortedAttempt::Cancelled))
    ));
    server.join().unwrap();
    let mut q = queue.lock();
    q.enqueue_owner(99, 8);
    assert!(matches!(
        q.head_blocker(99, Instant::now()),
        Some(AdmitBlocker::ServerHold { .. })
    ));
}

#[test]
fn refused_client_stream_closes_before_queue_retirement() {
    if run_hosted_test_in_child(
        "release_tests::refused_client_stream_closes_before_queue_retirement",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::{Read, Write};
    let network = HostedNetworkFixture::start();
    for code in [3, 5, 7, 16, 21, -1] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (eof_tx, eof_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
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
        let outcome = network
            .run_login(&mut permit, &arm, &mut client, command, world, |_| {})
            .expect("started attempt returns exact Q facts");
        assert!(matches!(
            outcome.result(),
            Err(error) if error.code == code
        ));
        {
            let mut q = queue.lock();
            q.enqueue_owner(99, 8);
            assert!(
                matches!(
                    q.head_blocker(99, Instant::now()),
                    Some(
                        AdmitBlocker::Retiring
                            | AdmitBlocker::Spacing { .. }
                            | AdmitBlocker::ServerHold { .. }
                    )
                ),
                "Q cannot grant the next owner: the socket still retires, or an early close leaves ordinary spacing/server hold"
            );
        }
        let error = finish_test_login(&arm, &mut client, command, world, outcome).unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(
            eof_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            0,
            "socket must close before exact retirement acknowledgement"
        );
        assert!(!client.ingame);
        assert!(!client.has_stream());
        server.join().unwrap();
    }
}

#[test]
fn cancelled_late_success_logs_out_and_closes_real_socket_before_retirement() {
    if run_hosted_test_in_child(
        "release_tests::cancelled_late_success_logs_out_and_closes_real_socket_before_retirement",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::{Read, Write};
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (eof_tx, eof_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
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
    let outcome = network
        .run_login(&mut permit, &arm, &mut client, command, world, |returned| {
            assert!(returned.ingame && returned.has_stream());
            arm.request_logout();
        })
        .expect("started attempt returns exact Q facts");
    assert!(matches!(
        finish_test_login(&arm, &mut client, command, world, outcome),
        Ok(SlotLoginOutcome::Aborted(AbortedAttempt::Cancelled))
    ));
    assert_eq!(
        eof_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        0,
        "late-success close must precede retirement"
    );
    assert!(!client.ingame);
    assert!(!client.has_stream());
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
        .expect("Client::login preserves the real transport error");
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
fn os_connect_refusal_maps_to_connect_transport_event() {
    if run_hosted_test_in_child(
        "release_tests::os_connect_refusal_maps_to_connect_transport_event",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    let network = HostedNetworkFixture::start();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let arm = SlotArm::new(7, true);
    let mut client = tcp_client(port);
    let error = run_hosted_client_login(&network, &arm, &mut client, port, false).unwrap_err();
    let transport = error.transport.as_ref().expect("login transport error");
    assert!(
        transport.errno.is_some(),
        "the refused socket's OS error number must survive Client::login: {transport:?}"
    );
    assert_transport_event(
        error,
        client::client::LoginStage::Connect,
        std::io::ErrorKind::ConnectionRefused,
        TransportFailure::Connect,
    );
}

#[test]
fn os_partial_seed_timeout_maps_to_read_transport_event() {
    if run_hosted_test_in_child(
        "release_tests::os_partial_seed_timeout_maps_to_read_transport_event",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::{Read, Write};
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, held) = std::sync::mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        socket.write_all(&[0; 9]).unwrap();
        held.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    let arm = SlotArm::new(7, true);
    let mut client = tcp_client(port);
    let prepared = network.prepared_attempt(&arm, port);
    let cancel = client::io::client_stream::LoginCancel::new();
    cancel.set_host_attempt_identity(prepared.hook_identity(arm.queue_owner, None));
    client.set_login_cancel(cancel);
    client.set_login_deadline(Instant::now() + Duration::from_millis(150));
    let error = client_adapter::host_network::with_prepared_game_attempt(prepared, || {
        client.login("alice", "secret", false)
    })
    .unwrap_err();
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
fn os_partial_seed_eof_maps_to_closed_transport_event() {
    if run_hosted_test_in_child(
        "release_tests::os_partial_seed_eof_maps_to_closed_transport_event",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::{Read, Write};
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        socket.write_all(&[0; 9 + 2]).unwrap();
    });
    let arm = SlotArm::new(7, true);
    let mut client = tcp_client(port);
    let error = run_hosted_client_login(&network, &arm, &mut client, port, false).unwrap_err();
    server.join().unwrap();
    assert_transport_event(
        error,
        client::client::LoginStage::ReadSeed,
        std::io::ErrorKind::UnexpectedEof,
        TransportFailure::Closed,
    );
}

/// End the server's side of an accepted connection with a TCP reset (RST),
/// not a FIN: an abortive close, `SO_LINGER` on with a zero timeout. `socket2`
/// sets it the same way on Linux, macOS and Windows.
fn reset_peer(socket: std::net::TcpStream) {
    socket2::SockRef::from(&socket)
        .set_linger(Some(Duration::ZERO))
        .expect("set SO_LINGER to zero before the abortive close");
    drop(socket);
}

/// What each system reports for an established socket whose peer reset it.
/// Linux and macOS report `ECONNRESET`, or `EPIPE` for a write that finds the
/// connection already reset. Windows reports `WSAECONNRESET`, or
/// `WSAECONNABORTED` for a write after the reset. `InvalidInput` (the macOS
/// `set_write_timeout` failure of the standalone transport) is never one.
fn is_peer_reset_kind(kind: std::io::ErrorKind) -> bool {
    match kind {
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => true,
        std::io::ErrorKind::ConnectionAborted => cfg!(windows),
        _ => false,
    }
}

/// The OS error number must be the real one: std derives the same kind from it.
fn assert_errno_matches_kind(errno: Option<i32>, kind: std::io::ErrorKind) {
    let errno = errno.expect("the OS error number of the reset must survive the adapter");
    assert_eq!(std::io::Error::from_raw_os_error(errno).kind(), kind);
}

#[test]
fn os_reset_after_hello_maps_to_read_transport_event() {
    if run_hosted_test_in_child(
        "release_tests::os_reset_after_hello_maps_to_read_transport_event",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::Read;
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        reset_peer(socket);
    });
    let arm = SlotArm::new(7, true);
    let mut client = tcp_client(port);
    let error = run_hosted_client_login(&network, &arm, &mut client, port, false).unwrap_err();
    server.join().unwrap();
    // The server read the hello before it reset, so the write is complete and
    // the reset can only be found by the read that waits for the reply.
    let transport = error.transport.as_ref().expect("login transport error");
    assert_eq!(transport.kind, std::io::ErrorKind::ConnectionReset);
    assert_errno_matches_kind(transport.errno, transport.kind);
    assert_transport_event(
        error,
        client::client::LoginStage::ReadHello,
        std::io::ErrorKind::ConnectionReset,
        TransportFailure::Read,
    );
}

#[test]
fn os_reset_before_the_hello_maps_to_a_reset_transport_event() {
    if run_hosted_test_in_child(
        "release_tests::os_reset_before_the_hello_maps_to_a_reset_transport_event",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (socket, _) = accept_local_with_timeout(&listener).unwrap();
        reset_peer(socket);
    });
    let arm = SlotArm::new(7, true);
    let mut client = tcp_client(port);
    let error = run_hosted_client_login(&network, &arm, &mut client, port, false).unwrap_err();
    server.join().unwrap();
    // The reactor and the login thread race: the reactor either has latched
    // the reset when the hello is queued (send_hello) or finds it after the
    // hello was queued (read_hello). Both keep the stage, kind and errno.
    let transport = error.transport.as_ref().expect("login transport error");
    let (stage, kind) = (transport.stage, transport.kind);
    assert!(
        is_peer_reset_kind(kind),
        "reset error kind on {}: {kind:?}",
        std::env::consts::OS
    );
    assert_errno_matches_kind(transport.errno, kind);
    let expected = match stage {
        client::client::LoginStage::SendHello => TransportFailure::Write,
        client::client::LoginStage::ReadHello => TransportFailure::Read,
        other => {
            panic!("a reset before the hello is a send_hello or read_hello failure: {other:?}")
        }
    };
    assert_transport_event(error, stage, kind, expected);
}

/// The write itself: the adapter has observed the peer's reset, and the next
/// handshake write through the hosted stream reports that reset with the
/// OS error number the reactor saw, not a bare "closed" or `InvalidInput`.
#[test]
fn os_reset_stream_write_reports_the_reset_with_its_errno() {
    if run_hosted_test_in_child(
        "release_tests::os_reset_stream_write_reports_the_reset_with_its_errno",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use client::io::client_stream::{ClientStream, LoginCancel};
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (socket, _) = accept_local_with_timeout(&listener).unwrap();
        reset_peer(socket);
    });
    let arm = SlotArm::new(7, true);
    let prepared = network.prepared_attempt(&arm, port);
    let cancel = LoginCancel::new();
    cancel.set_host_attempt_identity(prepared.hook_identity(arm.queue_owner, None));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = client_adapter::host_network::with_prepared_game_attempt(prepared, || {
        ClientStream::connect_until(
            client::Transport::Tcp,
            "127.0.0.1",
            port,
            deadline,
            Some(&cancel),
        )
    })
    .expect("connect through the adapter");
    server.join().unwrap();
    // The adapter's reactor owns the socket; wait until it reports the reset.
    let seen = loop {
        match stream.available() {
            Err(error) => break error,
            Ok(0) if Instant::now() < deadline => thread::park_timeout(Duration::from_millis(2)),
            other => panic!("expected the adapter to observe the peer reset: {other:?}"),
        }
    };
    assert!(
        is_peer_reset_kind(seen.kind()),
        "observed reset kind on {}: {:?}",
        std::env::consts::OS,
        seen.kind()
    );
    let write = stream
        .write(&[14, 0], 2)
        .expect_err("a write after the reset must fail");
    assert!(
        is_peer_reset_kind(write.kind()),
        "reset write kind on {}: {:?}",
        std::env::consts::OS,
        write.kind()
    );
    assert_errno_matches_kind(write.raw_os_error(), write.kind());
    assert_eq!(write.raw_os_error(), seen.raw_os_error());
}

/// A handshake write to a peer that accepts and never reads: the adapter's
/// outbound ring and both kernel buffers fill, the write that blocks returns
/// at the login deadline, and its outcome is the typed handshake-deadline
/// error rather than a raw OS timeout (`os error 10060` on Windows).
#[test]
fn handshake_write_to_a_peer_that_never_reads_ends_at_the_typed_login_deadline() {
    if run_hosted_test_in_child(
        "release_tests::handshake_write_to_a_peer_that_never_reads_ends_at_the_typed_login_deadline",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use client::io::client_stream::{ClientStream, LoginCancel};
    let network = HostedNetworkFixture::start();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, held) = std::sync::mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (socket, _) = accept_local_with_timeout(&listener).unwrap();
        held.recv_timeout(Duration::from_secs(15)).unwrap();
        drop(socket);
    });
    let arm = SlotArm::new(7, true);
    let prepared = network.prepared_attempt(&arm, port);
    let cancel = LoginCancel::new();
    cancel.set_host_attempt_identity(prepared.hook_identity(arm.queue_owner, None));
    let window = Duration::from_millis(1500);
    let started = Instant::now();
    let deadline = started + window;
    let mut stream = client_adapter::host_network::with_prepared_game_attempt(prepared, || {
        ClientStream::connect_until(
            client::Transport::Tcp,
            "127.0.0.1",
            port,
            deadline,
            Some(&cancel),
        )
    })
    .expect("connect through the adapter");
    // One record must fit the adapter's outbound ring; several fill it.
    let chunk = vec![0_u8; client_adapter::ring::OUT_RING_BYTES / 2];
    let (error, blocked_for) = loop {
        let call = Instant::now();
        if let Err(error) = stream.write(&chunk, chunk.len()) {
            break (error, call.elapsed());
        }
        assert!(
            started.elapsed() < window + Duration::from_secs(4),
            "the writes neither blocked nor reached the login deadline"
        );
    };
    let took = started.elapsed();
    release.send(()).unwrap();
    server.join().unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error:?}");
    assert_eq!(
        error.raw_os_error(),
        None,
        "not a raw OS timeout: {error:?}"
    );
    assert_eq!(error.to_string(), "login handshake deadline passed");
    assert!(
        took >= window && took < window + Duration::from_millis(1000),
        "the blocked write returned {took:?} after the start, not at its {window:?} deadline"
    );
    assert!(
        blocked_for >= Duration::from_millis(100),
        "the failing write returned after {blocked_for:?}; it never waited on a full queue"
    );
    assert!(
        stream.bytes_out() > 0,
        "the handshake wrote before it blocked"
    );
}

#[test]
fn hosted_client_response_sixteen_publishes_server_hold_with_exact_deadline() {
    if run_hosted_test_in_child(
        "release_tests::hosted_client_response_sixteen_publishes_server_hold_with_exact_deadline",
        "HOST_PLAY_HOSTED_LOGIN_CHILD",
    ) {
        return;
    }
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let network = HostedNetworkFixture::start();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut socket, _) = accept_local_with_timeout(&listener).unwrap();
        let mut hello = [0; 2];
        socket.read_exact(&mut hello).unwrap();
        let mut reply = [0; 9];
        reply[8] = 16;
        socket.write_all(&reply).unwrap();
    });
    let arm = SlotArm::new(7, true);
    let mut client = tcp_client(port);
    let error = run_hosted_client_login(&network, &arm, &mut client, port, false).unwrap_err();
    server.join().unwrap();
    assert_eq!(error.code, 16, "the hosted client read response 16");
    let backoff = LoginBackoff::new();
    let hold = server_hold(&Err::<(), _>(error), &backoff).unwrap();
    let at = Instant::now();
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    queue.lock().hold_for(at, hold);
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
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

fn queue_attempt(
    queue: &mut LoginQueue,
    arm: &SlotArm,
    now: Instant,
    binding: u64,
) -> QueueAttemptCells {
    queue.enqueue_owner(arm.queue_owner, 7);
    if let Some(until) = arm.clean_release_admission_at(HOST, PORT, now) {
        queue.defer_owner_until(arm.queue_owner, until);
    }
    assert_eq!(poll_permit(queue, arm.queue_owner, 7, now), Permit::Grant);
    let cells = synthetic_queue_attempt(arm.queue_owner, binding);
    assert!(cells.start(queue, now).is_some());
    cells
}

fn return_and_release_queue_attempt(
    queue: &mut LoginQueue,
    cells: &QueueAttemptCells,
    now: Instant,
    hold: Option<Duration>,
) -> bool {
    let returned = queue.return_attempt(cells.identity(), now, hold);
    for cancel in returned.cancels {
        cancel.cancel();
    }
    cells.mark_physical_release();
    assert!(cells.acknowledge_after_physical_release(queue, now));
    returned.accepted
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
    let first = queue_attempt(&mut queue, &arm, until, 1);
    let command = arm.login_command(false).unwrap();
    assert!(return_and_release_queue_attempt(
        &mut queue, &first, until, None
    ));
    assert_eq!(
        arm.defer_clean_release_at(&busy(), command, HOST, PORT, until),
        Some(CLEAN_RELEASE_CADENCE)
    );
    let retry = until + CLEAN_RELEASE_CADENCE;
    let second = queue_attempt(&mut queue, &arm, retry, 2);
    assert!(return_and_release_queue_attempt(
        &mut queue, &second, retry, None
    ));
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
    for binding in 1..=4 {
        queue.enqueue_owner(owner, 7);
        assert_eq!(poll_permit(&mut queue, owner, 7, now), Permit::Grant);
        let cells = synthetic_queue_attempt(owner, binding);
        assert!(cells.start(&mut queue, now).is_some());
        assert!(return_and_release_queue_attempt(
            &mut queue, &cells, now, None
        ));
    }

    queue.enqueue_owner(owner, 7);
    queue.enqueue_owner(2, 8);
    assert_eq!(poll_permit(&mut queue, 2, 8, now), Permit::Grant);
    let cells = synthetic_queue_attempt(2, 5);
    assert!(cells.start(&mut queue, now).is_some());
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
    let sibling = synthetic_queue_attempt(u64::MAX, 1);
    assert!(sibling.start(&mut queue, now).is_some());
    let later = now + CLEAN_RELEASE_CADENCE;
    assert!(
        matches!(
            poll_permit(&mut queue, arm.queue_owner, 7, later),
            Permit::Wait(_)
        ),
        "still one handshake even after release deadline"
    );
    assert!(return_and_release_queue_attempt(
        &mut queue, &sibling, later, None
    ));
    assert_eq!(
        poll_permit(&mut queue, arm.queue_owner, 7, later),
        Permit::Grant
    );
    let relog = synthetic_queue_attempt(arm.queue_owner, 2);
    assert!(relog.start(&mut queue, later).is_some());
    queue.enqueue_owner(u64::MAX - 1, 10);
    assert!(matches!(
        poll_permit(&mut queue, u64::MAX - 1, 10, later),
        Permit::Wait(_)
    ));
    assert!(return_and_release_queue_attempt(
        &mut queue, &relog, later, None
    ));
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
        let cells = test_cells(&permit, 1);
        let result = permit.attempt_with_test_cells(
            cells,
            |cancel, generation| {
                arm.begin_attempt_with_generation(command, world, cancel, generation)
            },
            |_, _| Ok::<bool, ()>(true),
            |_| None,
        );
        assert!(
            result.is_none(),
            "release={release}: old password authorization must be refused"
        );
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
    let cells = synthetic_queue_attempt(u64::MAX, 1);
    {
        let mut q = queue.lock();
        q.enqueue_owner(u64::MAX, 9);
        assert_eq!(q.poll_owner(u64::MAX, 9, expired_at).permit, Permit::Grant);
        assert!(cells.start(&mut q, expired_at).is_some());
        q.enqueue_owner(arm.queue_owner, 7);
    }
    arm.assert_cancel_order.store(true, Ordering::Relaxed);
    let mut physically_released = false;
    let wait = wait_for_permit(&queue, &statuses, "alice", 7, &arm, &mut || {
        if !physically_released {
            assert!(cells.cancel().is_cancelled());
            cells.mark_physical_release();
            physically_released = true;
            let mut q = queue.lock();
            assert!(
                !cells.acknowledge_after_physical_release(&mut q, Instant::now()),
                "an early physical ack must preserve the flight until Phase Q"
            );
        }
        Duration::from_millis(1)
    });
    assert!(matches!(wait, PermitWait::Granted(_)));
    assert!(physically_released);
    assert!(cells.cancel().is_cancelled());
    let returned_at = Instant::now();
    let mut q = queue.lock();
    assert!(
        !q.return_attempt(cells.identity(), returned_at, None)
            .accepted
    );
    assert!(cells.acknowledge_after_physical_release(&mut q, returned_at));
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
