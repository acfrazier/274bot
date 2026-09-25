use super::*;
use client::config::if_type::ComponentType;
use client::{BotTarget, ClientSessionConfig, ClientSessionProfile};
use host::Guardian;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait_for_permit(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
) -> PermitWait {
    enqueue_queue_place(queue, statuses, username, uid, arm);
    super::wait_for_permit(queue, statuses, username, uid, arm)
}

const TEST_QUEUE_OWNER_NAMESPACE: u64 = 1 << 63;

fn test_queue_owner(uid: i32) -> u64 {
    TEST_QUEUE_OWNER_NAMESPACE | u64::from(uid as u32)
}

fn request_test_owner(queue: &mut LoginQueue, uid: i32, now: Instant) -> Permit {
    let owner = test_queue_owner(uid);
    queue.enqueue_owner(owner, uid);
    queue.poll_owner(owner, uid, now)
}

fn request_shared(queue: &SharedLoginQueue, uid: i32, now: Instant) -> Permit {
    request_test_owner(&mut queue.lock(), uid, now)
}

fn wait_for_permit_bounded(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &Arc<SlotArm>,
) -> PermitWait {
    let queue = Arc::clone(queue);
    let statuses = Arc::clone(statuses);
    let username = username.to_string();
    let arm = Arc::clone(arm);
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(wait_for_permit(&queue, &statuses, &username, uid, &arm));
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("permit waiter exceeded bounded timeout")
}

fn native_requested(
    to: WorldTile,
    radius: i32,
    allow_teleports: bool,
) -> (WorldTile, i32, bool, bool, bool) {
    (to, radius, allow_teleports, false, false)
}

/// No script slot: the follow's arrival probe reads an unavailable view.
fn no_reach() -> Arc<api::query::ReachQueryView> {
    Arc::new(api::query::ReachQueryView::unavailable())
}

fn wait_script_state(play: &Play, name: &str, want: script::RunState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while play.script_state(name) != want && Instant::now() < deadline {
        play.pump_script_lifecycle(name);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(play.script_state(name), want);
}

#[test]
fn terminal_startup_error_is_only_the_producer_asset_failure() {
    let mut status = SlotStatus {
        username: "alice".into(),
        startup_phase: StartupPhase::Error,
        error: Some("profile asset initialization failed: Cache identity mismatch".into()),
        ..SlotStatus::default()
    };
    assert_eq!(
        status.terminal_startup_error(),
        Some("profile asset initialization failed: Cache identity mismatch")
    );

    status.error = Some("code 16: login rejected".into());
    assert_eq!(status.terminal_startup_error(), None);
    status.startup_phase = StartupPhase::Preparing;
    status.error = Some("profile asset initialization failed: stale row".into());
    assert_eq!(status.terminal_startup_error(), None);

    let statuses = vec![
        SlotStatus {
            username: "unrelated".into(),
            startup_phase: StartupPhase::Error,
            error: Some("profile asset initialization failed: foreign".into()),
            ..SlotStatus::default()
        },
        SlotStatus {
            username: "owned".into(),
            startup_phase: StartupPhase::Preparing,
            ..SlotStatus::default()
        },
    ];
    assert_eq!(
        owned_terminal_startup_error(&statuses, &["owned".into()]),
        None,
        "unrelated or healthy slots do not fail the run"
    );
    assert_eq!(
        owned_terminal_startup_error(
            &[
                statuses[1].clone(),
                SlotStatus {
                    username: "owned-2".into(),
                    startup_phase: StartupPhase::Error,
                    error: Some("profile asset initialization failed: mismatch".into()),
                    ..SlotStatus::default()
                },
            ],
            &["owned".into(), "owned-2".into()]
        ),
        Some("owned-2: profile asset initialization failed: mismatch".into())
    );
}

#[test]
fn startup_progress_is_latest_only_and_clears_on_completion() {
    let statuses = Arc::new(Mutex::new(
        ["alice", "bob"]
            .into_iter()
            .map(|username| SlotStatus {
                username: username.into(),
                ..SlotStatus::default()
            })
            .collect(),
    ));

    publish_startup_phase(&statuses, "alice", "Preparing client");
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_progress_percent, None);
        assert_eq!(rows[0].startup_progress_message, "Preparing client");
        assert_eq!(rows[0].startup_phase, StartupPhase::Preparing);
        assert!(rows[1].startup_progress_message.is_empty());
    }
    publish_startup_progress(&statuses, "alice", "Requesting models", 70);
    publish_startup_progress(&statuses, "alice", "Preparing game engine", 100);
    {
        let rows = statuses.lock().unwrap();
        let row = &rows[0];
        assert_eq!(row.startup_progress_percent, Some(100));
        assert_eq!(row.startup_progress_message, "Preparing game engine");
        assert!(rows[1].startup_progress_message.is_empty());
    }

    clear_startup_progress(&statuses, "alice");
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_progress_percent, None);
        assert!(rows[0].startup_progress_message.is_empty());
    }

    set_startup_phase(&statuses, "alice", StartupPhase::Queueing);
    set_startup_phase(&statuses, "alice", StartupPhase::Connecting);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::Connecting);
        assert_eq!(rows[1].startup_phase, StartupPhase::Preparing);
    }
    record_login_error(
        &statuses,
        "alice",
        &LoginError {
            code: 16,
            mes1: "busy".into(),
            mes2: "busy".into(),
            retry_after: None,
        },
    );
    let rows = statuses.lock().unwrap();
    assert_eq!(rows[0].startup_phase, StartupPhase::Error);
    assert_eq!(rows[1].startup_phase, StartupPhase::Preparing);
}

fn drive_startup_observe(
    pump: &mut Pump,
    snapshot: &mut GameSnapshot,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    client: &Client,
) {
    let drain = pump.drain_client(client);
    let _session_boundary = publish_session_boundary_status(
        statuses,
        name,
        drain.session_changed,
        client.ingame,
        snapshot.ingame(),
    );
    host::publish_snapshot(snapshot, client, drain);
    let ready = client.ingame && client.scene_state == 2 && snapshot.local_player().is_some();
    let mut all = statuses.lock().unwrap();
    let s = all.iter_mut().find(|s| s.username == name).unwrap();
    s.ingame = ready;
    s.scene_state = snapshot.scene_state();
    apply_startup_phase(s, name, ready, client.ingame);
}

#[test]
fn startup_observe_keeps_loading_scene_across_initial_session_generation() {
    assert_eq!(
        startup_phase_after_observation(StartupPhase::Queueing, false, false),
        Some(StartupPhase::Queueing)
    );
    assert_eq!(
        startup_phase_after_observation(StartupPhase::Queueing, false, true),
        Some(StartupPhase::LoadingScene)
    );
    assert_eq!(
        startup_phase_after_observation(StartupPhase::Preparing, true, true),
        None
    );
    assert_eq!(
        startup_phase_after_observation(StartupPhase::Error, true, true),
        None
    );

    let statuses = Arc::new(Mutex::new(
        ["alice", "bob"]
            .into_iter()
            .map(|username| SlotStatus {
                username: username.into(),
                ..SlotStatus::default()
            })
            .collect(),
    ));
    set_startup_phase(&statuses, "alice", StartupPhase::Queueing);
    mark_login_started(&statuses, "alice");
    set_startup_phase(&statuses, "alice", StartupPhase::LoadingScene);

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 1;
    client.gens.session = 1;

    let mut pump = Pump::new();
    let mut snapshot = GameSnapshot::new();

    drive_startup_observe(&mut pump, &mut snapshot, &statuses, "alice", &client);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::LoadingScene);
        assert!(rows[0].connected, "authenticated session is connected");
        assert!(
            !rows[0].ingame,
            "producer gate stays closed until a current player"
        );
        assert!(rows[0].login_started.is_some());
        assert_eq!(rows[1].startup_phase, StartupPhase::Preparing);
        assert!(snapshot.local_player().is_none());
    }

    client.local_player = Some(client::client::ClientPlayer::at(10, 10));
    client.scene_state = 2;
    drive_startup_observe(&mut pump, &mut snapshot, &statuses, "alice", &client);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(
            rows[0].startup_phase,
            StartupPhase::LoadingScene,
            "stale actor tables must not authorize Ready"
        );
        assert!(!rows[0].ingame);
        assert!(rows[0].connected);
        assert!(snapshot.local_player().is_none());
    }

    client.gens.player += 1;
    client.gens.player_info += 1;
    drive_startup_observe(&mut pump, &mut snapshot, &statuses, "alice", &client);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::Ready);
        assert!(rows[0].ingame);
        assert!(rows[0].connected);
        assert!(snapshot.local_player().is_some());
        assert_eq!(rows[1].startup_phase, StartupPhase::Preparing);
    }

    client.ingame = false;
    drive_startup_observe(&mut pump, &mut snapshot, &statuses, "alice", &client);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::Queueing);
        assert!(!rows[0].ingame);
        assert!(!rows[0].connected);
        assert!(rows[0].login_started.is_none());
    }

    mark_login_started(&statuses, "alice");
    set_startup_phase(&statuses, "alice", StartupPhase::LoadingScene);
    client.ingame = true;
    client.scene_state = 1;
    client.local_player = None;
    client.gens.session += 1;
    drive_startup_observe(&mut pump, &mut snapshot, &statuses, "alice", &client);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::LoadingScene);
        assert!(!rows[0].ingame);
        assert!(rows[0].connected);
        assert!(rows[0].login_started.is_some());
    }

    record_login_error(
        &statuses,
        "alice",
        &LoginError {
            code: 5,
            mes1: "invalid".into(),
            mes2: "invalid".into(),
            retry_after: None,
        },
    );
    client.ingame = false;
    drive_startup_observe(&mut pump, &mut snapshot, &statuses, "alice", &client);
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::Error);
        assert!(rows[0].error.is_some());
        assert_eq!(rows[1].startup_phase, StartupPhase::Preparing);
    }
}

#[test]
fn connected_session_boundary_keeps_the_connection_published() {
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        connected: true,
        ingame: true,
        bytes_in: 123,
        ..SlotStatus::default()
    }]));

    assert!(publish_session_boundary_status(
        &statuses, "alice", true, true, true
    ));

    let row = &statuses.lock().unwrap()[0];
    assert!(
        row.connected,
        "a new authenticated session must not transiently publish disconnected"
    );
    assert!(
        !row.ingame,
        "the new session still closes the producer gate"
    );
    assert_eq!(row.bytes_in, 0);
}

#[test]
fn disconnected_row_drops_session_counters_but_retains_script_paint() {
    let paint = Arc::new(script::shim::ScriptPaint::default());
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        connected: true,
        ingame: true,
        bytes_in: 123,
        bytes_out: 456,
        run_sends: 7,
        script_paint: Some(Arc::clone(&paint)),
        ..SlotStatus::default()
    }]));

    crate::play_status::publish_slot_disconnected(&statuses, "alice");

    let rows = statuses.lock().unwrap();
    assert_eq!((rows[0].bytes_in, rows[0].bytes_out), (0, 0));
    assert_eq!(rows[0].run_sends, 0);
    assert!(
        rows[0]
            .script_paint
            .as_ref()
            .is_some_and(|retained| Arc::ptr_eq(retained, &paint)),
        "disconnect retains the paused script's shared paint frame"
    );
}

#[test]
fn projected_npc_boxes_follow_the_live_clients_bounded_npc_list() {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.cam_x = 6400;
    client.cam_z = 5000;
    let mut npc = client::dash3d::ClientNpc::at(49, 46);
    npc.r#type = Some(0);
    npc.entity.x = 6400;
    npc.entity.z = 6000;
    npc.entity.size = 1;
    npc.entity.height = 100;
    client.npc[7] = Some(Box::new(npc));
    client.npc_ids[0] = 7;
    client.npc_count = 1;

    let boxes = projected_npc_boxes(&client).expect("ready scene");
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].index, 7);
    assert_eq!(boxes[0].points[0], (225, 171));
    let bytes = with_script_snapshot_input(
        1,
        None,
        true,
        None,
        None,
        None,
        None,
        Some(&boxes),
        false,
        false,
        false,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        PostedInspect::default(),
        script::isolate_fb::encode_snapshot_with_native,
    );
    let posted = script::isolate_fb::decode_snapshot(&bytes).expect("snapshot decodes");
    assert!(posted.npc_boxes_available());
    assert_eq!(posted.npc_boxes()[0].points()[0], (225, 171));

    client.npc_count = 0;
    assert_eq!(projected_npc_boxes(&client), Some(Vec::new()));
    client.scene_state = 1;
    assert_eq!(projected_npc_boxes(&client), None);
}
use std::thread;

use client::client::ClientConfig;
use client::client::MiniMenuAction;
use client::config::if_type::ButtonType;
use client::config::Cache;
use client::io::{Packet, ServerProt};
use nav::collision::WorldCollision;
use nav::router::Leg;
use nav::tile::Tile;
use nav::transport::{TransportEdge, TransportGraph, TransportKind};
use vault::ProfileSettings;

#[test]
fn world_host_live_is_rs2b2t_everything_else_is_loopback() {
    assert_eq!(world_host_for_bot_target(Some("live")), "w1.rs2b2t.com");
    assert_eq!(world_host_for_bot_target(Some("prod")), "w1.rs2b2t.com");
    assert_eq!(world_host_for_bot_target(Some("local")), "127.0.0.1");
    assert_eq!(world_host_for_bot_target(None), "127.0.0.1");
}

#[test]
fn mint_live_names_are_unique_and_never_test() {
    let mut all = std::collections::HashSet::new();
    // Repeated fleet sizes catch truncation that drops the invocation
    // serial, including its low digits after it grows past one digit.
    for _ in 0..24 {
        for n in [1, 2, 32, 128] {
            let names = mint_live_names(n);
            assert_eq!(names.len(), n);
            for name in names {
                assert_ne!(name, "test", "a live boot must never log in `test`");
                assert!(name.starts_with("live"), "minted name: {name}");
                assert!(
                    name.len() <= 12,
                    "the engine enforces the 12-char username limit: {name}"
                );
                assert!(
                    all.insert(name.clone()),
                    "every minted name must be unique per invocation: {name}"
                );
            }
        }
    }
}

#[test]
fn mint_live_names_keep_the_12_char_limit_at_scale() {
    for n in [1, 2, 10, 50] {
        for name in mint_live_names(n) {
            assert!(
                name.len() <= 12,
                "{name} (n={n}) exceeds the 12-char username limit"
            );
        }
    }
}

#[test]
fn mint_live_names_zero_is_empty() {
    assert!(mint_live_names(0).is_empty());
}

#[test]
fn profile_password_local_is_username() {
    assert_eq!(
        profile_password_for("alice", client::BotTarget::Local),
        "alice"
    );
}

#[test]
fn default_vault_path_prod_is_not_the_local_file() {
    assert_eq!(
        default_vault_path_for(client::BotTarget::Local),
        script::bot_file("vault")
    );
    assert_eq!(
        default_vault_path_for(client::BotTarget::Prod),
        script::bot_file("vault-prod")
    );
    assert_ne!(
        default_vault_path_for(client::BotTarget::Local),
        default_vault_path_for(client::BotTarget::Prod),
        "prod must not reuse the local vault blob"
    );
}

#[test]
fn profile_password_prod_is_not_username() {
    let pass = profile_password_for("alice", client::BotTarget::Prod);
    assert_ne!(pass, "alice");
    assert!(
        pass.len() >= 16,
        "prod password should be high-entropy: {pass}"
    );
}

#[test]
fn mint_live_entries_prod_refuses_username_as_password() {
    let names = mint_live_names(2);
    let entries = mint_live_entries_for_target(&names, client::BotTarget::Prod);
    for (u, p) in &entries {
        assert_ne!(u, p, "prod must not use username-as-password");
    }
    let local = mint_live_entries_for_target(&names, client::BotTarget::Local);
    for (u, p) in &local {
        assert_eq!(u, p, "local keeps username-as-password");
    }
}

#[test]
fn live_vault_passphrase_local_is_bot() {
    assert_eq!(live_vault_passphrase_for(client::BotTarget::Local), "bot");
}

#[test]
fn live_vault_passphrase_prod_is_not_bot() {
    let pass = live_vault_passphrase_for(client::BotTarget::Prod);
    assert_ne!(pass, "bot");
    assert!(pass.len() >= 16, "prod temp vault passphrase: {pass}");
}

#[test]
fn full_world_round_uses_existing_backoff() {
    let worlds = public_worlds::PublicWorlds::default();
    let mut round = public_worlds::WorldRound::new(&worlds, None, None).unwrap();
    let mut backoff = LoginBackoff::new();
    assert_eq!(
        round.on_login_error(7, worlds.worlds.len()),
        public_worlds::WorldErrorStep::SwitchNow
    );
    assert_eq!(
        round.on_login_error(7, worlds.worlds.len()),
        public_worlds::WorldErrorStep::SwitchAfterWait
    );
    assert_eq!(login_retry_wait(&mut backoff, 7), Duration::from_secs(5));
    let mut pinned = public_worlds::WorldRound::new(&worlds, Some(2), None).unwrap();
    assert_eq!(
        pinned.on_login_error(7, worlds.worlds.len()),
        public_worlds::WorldErrorStep::Stay
    );
    assert_eq!(login_retry_wait(&mut backoff, 7), Duration::from_secs(5));
}

#[test]
fn response_one_returns_to_fifo_without_publishing_error() {
    let mut backoff = LoginBackoff::new();
    assert_eq!(login_retry_wait(&mut backoff, 1), Duration::from_secs(2));

    let statuses = rows(&["alice"]);
    set_startup_phase(&statuses, "alice", StartupPhase::Connecting);
    record_login_error(
        &statuses,
        "alice",
        &LoginError {
            code: 1,
            mes1: "retry".into(),
            mes2: "retry".into(),
            retry_after: None,
        },
    );
    let rows = statuses.lock().unwrap();
    assert_eq!(rows[0].startup_phase, StartupPhase::Connecting);
    assert!(rows[0].error.is_none());
}

#[test]
fn response_21_ignores_world_change_until_server_delay_expires() {
    let statuses = rows(&["alice"]);
    set_startup_phase(&statuses, "alice", StartupPhase::Connecting);
    let arm = SlotArm::new(7, true);
    let started = Instant::now();
    let waiter = {
        let statuses = Arc::clone(&statuses);
        let arm = Arc::clone(&arm);
        thread::spawn(move || {
            wait_for_transfer_response(
                &LoginError {
                    code: 21,
                    mes1: "You have only just left another world".into(),
                    mes2: "Your profile will be transferred in: 1 seconds".into(),
                    retry_after: Some(Duration::from_secs(1)),
                },
                &arm,
                &statuses,
                "alice",
            )
        })
    };
    assert!(wait_until(500, || {
        statuses.lock().unwrap()[0]
            .startup_progress_message
            .contains("transferred in: 1 seconds")
    }));
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].startup_phase, StartupPhase::Connecting);
        assert!(rows[0].error.is_none());
    }
    *arm.world.lock() = Some(2);
    arm.world_generation.fetch_add(1, Ordering::Relaxed);
    arm.notify_retry_wait();
    assert!(wait_until(1_500, || {
        statuses.lock().unwrap()[0]
            .startup_progress_message
            .contains("transferred in: 0 seconds")
    }));

    assert_eq!(waiter.join().unwrap(), Some(true));
    assert!(started.elapsed() >= Duration::from_millis(1_900));
    assert_eq!(*arm.world.lock(), Some(2));
}

#[test]
fn spawned_worker_response_21_retries_same_endpoint_without_fifo_ownership() {
    use std::sync::mpsc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut login_attempt = 0;
        loop {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut preface = [0; 2];
            if socket.read_exact(&mut preface).is_err() || preface[0] != 14 {
                continue;
            }
            login_attempt += 1;
            let accepted = Instant::now();
            match login_attempt {
                1 => socket.write_all(&[0, 0, 0, 0, 0, 0, 0, 0, 21, 0]).unwrap(),
                2 => socket
                    .write_all(&[0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0])
                    .unwrap(),
                attempt => panic!("unexpected login attempt {attempt}"),
            }
            socket.flush().unwrap();
            attempt_tx
                .send((login_attempt, accepted, socket.local_addr().unwrap()))
                .unwrap();
            if login_attempt == 2 {
                release_rx.recv().unwrap();
                return;
            }
        }
    });

    let mut play = run_with_io(
        &PlayOptions {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let arm = SlotArm::new(42, true);
    arm.bypass_asset_startup_for_test();
    play.spawn_slot(profile("alice", 42), None, None, Some(Arc::clone(&arm)));

    let (first_number, first_attempt, first_endpoint) =
        attempt_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(first_number, 1);
    assert_eq!(first_endpoint, endpoint);
    assert!(wait_until(500, || {
        let statuses = play.statuses.lock().unwrap();
        let row = statuses
            .iter()
            .find(|status| status.username == "alice")
            .unwrap();
        row.startup_progress_message
            .contains("transferred in: 0 seconds")
            && row.startup_phase == StartupPhase::Connecting
            && row.error.is_none()
    }));
    assert!(
        play.queue.lock().status_owner(arm.queue_owner).is_none(),
        "the transfer countdown must hold no FIFO place"
    );
    assert!(
        !play.queue.lock().abandon_permit(42),
        "the transfer countdown must hold no handshake reservation"
    );
    assert_eq!(
        request_shared(&play.queue, 43, Instant::now()),
        Permit::Grant,
        "a follower can enter while the transfer countdown runs"
    );
    assert!(play
        .queue
        .lock()
        .acknowledge_login_return(43, Instant::now()));

    let (second_number, second_attempt, second_endpoint) =
        attempt_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    assert_eq!(second_number, 2);
    assert_eq!(second_endpoint, endpoint);
    assert!(
        second_attempt.duration_since(first_attempt) >= Duration::from_millis(900),
        "response 21 with a zero byte still waits one full second"
    );
    assert!(
        second_attempt.duration_since(first_attempt) < Duration::from_secs(4),
        "response 21 must not enter generic retry backoff"
    );
    assert!(wait_until(1_000, || {
        let statuses = play.statuses.lock().unwrap();
        let row = statuses
            .iter()
            .find(|status| status.username == "alice")
            .unwrap();
        row.startup_phase == StartupPhase::LoadingScene && row.error.is_none()
    }));

    arm.stop.store(true, Ordering::Relaxed);
    release_tx.send(()).unwrap();
    play.stop_slot("alice");
    server.join().unwrap();
}

#[test]
fn running_slot_profile_world_change_reseats_next_login_handshake() {
    let w1_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let w1_port = w1_listener.local_addr().unwrap().port();
    let w2_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let w2_port = w2_listener.local_addr().unwrap().port();
    let worlds = public_worlds::PublicWorlds {
        schema_version: 1,
        worlds: vec![
            public_worlds::PublicWorld {
                number: 1,
                host: "127.0.0.1".into(),
                port: w1_port,
                node_id: 10,
            },
            public_worlds::PublicWorld {
                number: 2,
                host: "localhost".into(),
                port: w2_port,
                node_id: 11,
            },
        ],
    };
    // Keep this proof entirely local: pre-seat the key cache so production
    // world configuration never attempts an HTTPS fetch.
    for world in &worlds.worlds {
        public_worlds::modulus_for(world, false, |_, _| {
            Some(client::PROD_LOGIN_RSAN.to_string())
        });
    }

    let session_profile = Arc::new(
        ClientSessionProfile::new(ClientSessionConfig {
            revision: client::client::ClientRevision::R289,
            target: BotTarget::Prod,
            game_host: worlds.worlds[0].host.clone(),
            game_port: w1_port,
            asset_host: "127.0.0.1".into(),
            asset_port: 1,
            cache_dir: "/tmp".into(),
            unpack_dir: "/tmp".into(),
            rsa_modulus: client::PROD_LOGIN_RSAN.into(),
            rsa_exponent: client::PROD_LOGIN_RSAE.into(),
            expected_crc: Some([0; 9]),
            content_id: "world-edit-login-fixture".into(),
        })
        .unwrap(),
    );
    let config = session_profile.client_config(true, true);
    let mut client = Client::from_shared_with_profile(
        config,
        Arc::new(Cache::default()),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        session_profile,
    )
    .unwrap();
    let arm = SlotArm::new(42, false);
    let mut account = profile("alice", 42);
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", Arc::clone(&arm));
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        world: Some(1),
        ..SlotStatus::default()
    });
    play.remember_profile(account.clone());

    let mut round = public_worlds::WorldRound::new(&worlds, None, Some(1)).unwrap();
    assert!(!refresh_slot_world_preference(&mut round, &worlds, &arm).unwrap());
    configure_slot_world(&mut client, &worlds.worlds[round.index], false, &arm.stop).unwrap();
    let w1_server = thread::spawn(move || w1_listener.accept().unwrap());
    let socket =
        std::net::TcpStream::connect((client.config.host.as_str(), client.config.port)).unwrap();
    drop(socket);
    drop(w1_server.join().unwrap());

    account.settings.world = Some(2);
    play.remember_profile(account);
    assert_eq!(
        play.statuses()[0].world,
        Some(1),
        "editing a live slot must not relabel its current connection"
    );
    assert!(matches!(
        request_shared(&play.queue, 42, Instant::now()),
        Permit::Grant
    ));
    assert!(
        !granted_permit_world_is_current(&play.queue, 42, Some(&round), &arm),
        "a world edit while queued must defer the grant before any handshake"
    );
    {
        let mut queue = play.queue.lock();
        assert!(matches!(
            request_test_owner(&mut queue, 43, Instant::now()),
            Permit::Grant
        ));
        assert!(queue.abandon_permit(43), "the stale grant was released");
    }
    assert!(refresh_slot_world_preference(&mut round, &worlds, &arm).unwrap());
    configure_slot_world(&mut client, &worlds.worlds[round.index], false, &arm.stop).unwrap();
    assert_eq!(client.config.host, "localhost");
    assert_eq!(client.config.port, w2_port);
    assert_eq!(client.node_id, 11);

    let w2_server = thread::spawn(move || w2_listener.accept().unwrap());
    let socket =
        std::net::TcpStream::connect((client.config.host.as_str(), client.config.port)).unwrap();
    drop(socket);
    drop(w2_server.join().unwrap());
}

#[test]
fn validate_play_host_loopback_ok_with_local_rsa() {
    assert!(validate_play_host("127.0.0.1", BotTarget::Local).is_ok());
    assert!(validate_play_host("localhost", BotTarget::Local).is_ok());
    assert!(validate_play_host("::1", BotTarget::Local).is_ok());
}

#[test]
fn validate_play_host_non_loopback_err_with_local_rsa() {
    assert!(validate_play_host("attacker.example", BotTarget::Local).is_err());
    assert!(validate_play_host("w1.rs2b2t.com", BotTarget::Local).is_err());
}

#[test]
fn play_endpoint_for_prod_is_wss_443() {
    assert_eq!(
        play_endpoint_for(BotTarget::Prod),
        ("w1.rs2b2t.com".into(), 443)
    );
    assert_eq!(
        play_endpoint_for(BotTarget::Local),
        ("127.0.0.1".into(), 43594)
    );
}

#[test]
fn validate_play_host_non_loopback_ok_with_prod_rsa() {
    assert!(validate_play_host("attacker.example", BotTarget::Prod).is_ok());
    assert!(validate_play_host("w1.rs2b2t.com", BotTarget::Prod).is_ok());
}

fn tmp_vault(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-host-play-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("nested").join("vault")
}

#[test]
fn spawn_config_follows_profile_lowmem() {
    let opt = PlayOptions {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    };
    let quiet = Profile {
        username: "a".into(),
        password: "a".into(),
        uid: 1,
        settings: ProfileSettings {
            lowmem: true,
            auto_login: false,
            tutorial_skipped: None,
            raster: vault::RasterMode::Gpu,
            random_events: true,
            lamp_skill: "strength".into(),
            lamp_auto: true,
            ..ProfileSettings::default()
        },
    };
    let loud = Profile {
        username: "b".into(),
        password: "b".into(),
        uid: 2,
        settings: ProfileSettings {
            lowmem: false,
            auto_login: false,
            tutorial_skipped: None,
            raster: vault::RasterMode::Gpu,
            random_events: true,
            lamp_skill: "strength".into(),
            lamp_auto: true,
            ..ProfileSettings::default()
        },
    };
    assert!(bot_client_config(&opt, &quiet).lowmem);
    assert!(!bot_client_config(&opt, &loud).lowmem);
}

#[test]
fn open_vault_creates_missing_parent_dirs() {
    let path = tmp_vault("create");
    assert!(!path.exists());
    let v = open_vault(&path, "bot").unwrap();
    drop(v);
    assert!(path.exists());
}

#[test]
fn open_vault_wrong_pass_is_not_already_exists() {
    let path = tmp_vault("wrong");
    open_vault(&path, "bot").unwrap();
    match open_vault(&path, "nope") {
        Err(VaultError::WrongPassphrase) => {}
        Err(e) => panic!("expected WrongPassphrase, got {e}"),
        Ok(_) => panic!("expected WrongPassphrase, unlocked"),
    }
}

#[test]
fn run_with_io_empty_profiles_starts_no_slots() {
    let play = run_with_io(
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
    );
    assert!(play.statuses().is_empty());
}

#[test]
fn obj_names_getter_shares_the_play_table() {
    // A cache-less temp dir falls back to `Cache::default()` (empty
    // objs), so the table is empty but still shared and queryable.
    let dir = std::env::temp_dir().join(format!("274bot-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let play = run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: dir.display().to_string(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    assert!(play.obj_names().name(526).is_none());
    assert!(play.obj_names().by_name("Bones").is_none());
}

#[test]
fn run_channels_empty_profiles_starts_no_slots() {
    let play = run_channels(
        &PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        1,
    );
    assert!(play.statuses().is_empty());
}

#[test]
fn stop_slot_sets_stop_and_forgets_name() {
    let mut play = run_with_io(
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
    );
    // No real client: fake arm (uid 7) and a handle that exits only
    // once `stop_slot` flags it.
    let arm = SlotArm::new(7, false);
    play.arms.insert("alice".into(), Arc::clone(&arm));
    let watchdog = {
        let arm = Arc::clone(&arm);
        thread::spawn(move || {
            while !arm.stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1));
            }
        })
    };
    play.handles.insert("alice".into(), watchdog);
    play.spawned.insert("alice".into());
    // uid 7 sits on the FIFO behind a full 29-grant address TTL;
    // stop_slot must drop it even though the thread is still running.
    {
        let mut q = play.queue.lock();
        let now = Instant::now();
        for i in 0..29 {
            assert!(matches!(
                request_test_owner(&mut q, 1000 + i, now),
                Permit::Grant
            ));
            assert!(q.acknowledge_login_return(1000 + i, now));
        }
        q.enqueue_owner(arm.queue_owner, 7);
        assert!(matches!(
            q.poll_owner(arm.queue_owner, 7, now),
            Permit::Wait(_)
        ));
    }

    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    });

    play.stop_slot("alice");

    assert!(arm.stop.load(Ordering::Relaxed));
    assert!(!play.spawned.contains("alice"));
    assert!(play.handles.is_empty());
    assert!(!play.arms.contains_key("alice"), "stop_slot drops the arm");
    assert!(
        play.statuses().iter().all(|s| s.username != "alice"),
        "stop_slot drops the status row"
    );
    assert!(play.queue.lock().status_owner(arm.queue_owner).is_none());
}

#[test]
fn stop_before_worker_start_retires_entries_and_respawn_has_one_row() {
    let mut play = run_with_io(
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
    );
    let profile = Profile {
        username: "alice".into(),
        password: "pw".into(),
        uid: 7,
        settings: ProfileSettings::default(),
    };
    let arm = SlotArm::new(7, true);
    arm.bypass_asset_startup_for_test();
    let (entered, release, published) = arm.hold_worker_start_for_test();
    let cleaned = arm.hold_stop_cleanup_for_test();
    play.spawn_slot(profile.clone(), None, None, Some(Arc::clone(&arm)));
    entered
        .recv_timeout(Duration::from_secs(2))
        .expect("worker did not reach the startup gate");

    let stopper = thread::spawn(move || {
        play.stop_slot("alice");
        play
    });
    cleaned
        .recv_timeout(Duration::from_secs(2))
        .expect("stop did not retire lifetime entries before joining");
    release.send(()).unwrap();
    published
        .recv_timeout(Duration::from_secs(2))
        .expect("worker did not pass lifetime-entry publication");
    let mut play = stopper.join().unwrap();

    assert!(!play.spawned.contains("alice"));
    assert!(!play.arms.contains_key("alice"));
    assert!(!play.handles.contains_key("alice"));
    assert!(!play.scripts.lock().unwrap().contains_key("alice"));
    assert!(!play.cheats.lock().unwrap().contains_key("alice"));
    assert!(!play.wires.lock().unwrap().contains_key("alice"));
    assert!(
        play.statuses()
            .iter()
            .all(|status| status.username != "alice"),
        "a stopped worker must not publish a ghost row after cleanup"
    );

    let replacement = SlotArm::new(8, false);
    replacement.bypass_asset_startup_for_test();
    let (entered, release, published) = replacement.hold_worker_start_for_test();
    play.spawn_slot(profile, None, None, Some(Arc::clone(&replacement)));
    entered
        .recv_timeout(Duration::from_secs(2))
        .expect("replacement worker did not reach the startup gate");
    release.send(()).unwrap();
    published
        .recv_timeout(Duration::from_secs(2))
        .expect("replacement did not publish its lifetime entries");
    assert_eq!(
        play.statuses()
            .iter()
            .filter(|status| status.username == "alice")
            .count(),
        1,
        "a replacement lifetime owns exactly one status row"
    );
    play.stop_slot("alice");
}

#[test]
fn begin_stop_slot_retires_without_joining_live_worker() {
    let mut play = run_with_io(
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
    );
    let arm = SlotArm::new(7, false);
    play.arms.insert("alice".into(), Arc::clone(&arm));
    play.spawned.insert("alice".into());
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    });
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    play.handles.insert(
        "alice".into(),
        thread::spawn(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }),
    );
    entered_rx.recv().unwrap();

    play.begin_stop_slot("alice");

    assert!(arm.stop.load(Ordering::Relaxed));
    assert!(!play.handles.contains_key("alice"));
    assert!(
        play.retiring.contains_key("alice"),
        "the live join is deferred for a finished-only reap"
    );
    assert!(play.statuses().is_empty());
    release_tx.send(()).unwrap();
    play.stop_slot("alice");
    assert!(play.retiring.is_empty());
}

#[test]
fn stop_slot_wakes_a_parked_thread_before_joining() {
    let mut play = run_with_io(
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
    );
    // A fake parked slot: blocks on the control park fd with a long
    // poll (like the real idle scheduler) and only exits once `stop`
    // is set *and* the wake fires it out of the poll.
    let (wake, park) = wake_channel();
    play.wakes.insert("bob".into(), wake);
    let arm = SlotArm::new(9, false);
    play.arms.insert("bob".into(), Arc::clone(&arm));
    play.spawned.insert("bob".into());
    let stop = Arc::clone(&arm.stop);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let watchdog = thread::spawn(move || {
        assert!(
            park.wait_readable(Duration::from_secs(5)),
            "stop_slot's wake must fire the parked wait"
        );
        park.drain();
        assert!(stop.load(Ordering::Relaxed), "woken because stop was set");
        done_tx.send(()).unwrap();
    });
    play.handles.insert("bob".into(), watchdog);

    thread::sleep(Duration::from_millis(50));
    let start = Instant::now();
    play.stop_slot("bob");
    // Without the wake the join would wait out the 5 s poll; the wake
    // must return the rail ✕ within a frame.
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "stop_slot must wake a parked thread, not wait for its poll"
    );
    done_rx
        .recv_timeout(Duration::from_millis(200))
        .expect("parked thread never acknowledged the wake");
    assert!(play.wakes.is_empty(), "stop_slot drops the wake end");
}

#[test]
fn stop_slot_interrupts_login_backoff() {
    let mut play = run_with_io(
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
    );
    let arm = SlotArm::new(9, true);
    play.arms.insert("bob".into(), Arc::clone(&arm));
    play.spawned.insert("bob".into());
    let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let waiter = thread::spawn(move || {
        waiting_tx.send(()).unwrap();
        done_tx
            .send(arm.wait_for_retry(Duration::from_secs(60)))
            .unwrap();
    });
    play.handles.insert("bob".into(), waiter);
    waiting_rx.recv().unwrap();

    let start = Instant::now();
    play.stop_slot("bob");
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "Stop must not join through the remaining login backoff"
    );
    assert!(
        !done_rx.recv_timeout(Duration::from_millis(100)).unwrap(),
        "Stop cancels, rather than completes, the retry wait"
    );
}

#[test]
fn retry_notification_serializes_with_waiter_park() {
    let arm = SlotArm::new(9, true);
    let (waiter_entered, release_waiter) = arm.hold_retry_wait_before_park_for_test();
    let (notify_entered, release_notify) = arm.hold_retry_notify_before_lock_for_test();

    let waiting_arm = Arc::clone(&arm);
    let waiter = thread::spawn(move || waiting_arm.wait_for_retry(Duration::from_millis(40)));
    waiter_entered.recv().unwrap();

    let notifying_arm = Arc::clone(&arm);
    let (notify_returned, observe_notify_returned) = std::sync::mpsc::channel();
    let notifier = thread::spawn(move || {
        notifying_arm.stop.store(true, Ordering::Relaxed);
        notifying_arm.notify_retry_wait();
        notify_returned.send(()).unwrap();
    });
    notify_entered.recv().unwrap();
    release_notify.send(()).unwrap();

    let returned_while_waiter_held = observe_notify_returned
        .recv_timeout(Duration::from_millis(100))
        .is_ok();
    release_waiter.send(()).unwrap();
    assert!(!waiter.join().unwrap(), "Stop cancels the retry wait");
    notifier.join().unwrap();
    assert!(
        !returned_while_waiter_held,
        "the notifier must serialize with the predicate lock until the waiter parks"
    );
}

#[test]
fn generic_wake_does_not_shorten_login_backoff() {
    let mut play = run_with_io(
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
    );
    let arm = SlotArm::new(9, true);
    play.attach_arm("bob", Arc::clone(&arm));
    let started = Instant::now();
    let waiter = thread::spawn(move || arm.wait_for_retry(Duration::from_millis(120)));
    thread::sleep(Duration::from_millis(20));
    play.wake("bob");

    assert!(waiter.join().unwrap());
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "focus/UI wakes must not spend another login attempt early"
    );
}

#[test]
fn stop_slot_during_unresponsive_public_key_fetch_is_bounded() {
    use std::sync::mpsc;

    let mut play = run_with_io(
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
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted, waiting) = mpsc::channel();
    let (release, keep_open) = mpsc::channel();
    let server = thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        accepted.send(()).unwrap();
        keep_open.recv().unwrap();
    });
    let arm = SlotArm::new(44, true);
    play.arms.insert("alice".into(), Arc::clone(&arm));
    let slot = thread::spawn(move || {
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port,
            cache_dir: "/tmp".into(),
            members: false,
            lowmem: true,
        });
        let world = public_worlds::PublicWorld {
            number: 13,
            host: "127.0.0.1".into(),
            port,
            node_id: 42,
        };
        configure_slot_world(&mut client, &world, true, &arm.stop).unwrap();
        assert!(arm.stop.load(Ordering::Relaxed));
    });
    play.handles.insert("alice".into(), slot);
    waiting.recv_timeout(Duration::from_secs(3)).unwrap();
    let started = Instant::now();
    play.stop_slot("alice");
    release.send(()).unwrap();
    server.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn stop_slot_leaves_profile_uid_when_arm_shared_at_spawn() {
    // A caller that retains its own clone makes the arm shared before
    // spawn; the uid must still be forced from the profile (an
    // `Arc::get_mut` fixup would silently no-op here).
    let arm = SlotArm::new(0, false);
    let _caller_clone = Arc::clone(&arm);
    let mut play = run_with_io(
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
    );
    // The profile uid 42 sits queued behind a full address window;
    // stopping must drop 42 from the FIFO, not the arm's stale uid 0.
    {
        let mut q = play.queue.lock();
        let now = Instant::now();
        for i in 0..29 {
            assert!(matches!(
                request_test_owner(&mut q, 1000 + i, now),
                Permit::Grant
            ));
            assert!(q.acknowledge_login_return(1000 + i, now));
        }
        q.enqueue_owner(arm.queue_owner, 42);
        assert!(matches!(
            q.poll_owner(arm.queue_owner, 42, now),
            Permit::Wait(_)
        ));
    }
    play.spawn_slot(
        Profile {
            username: "alice".into(),
            password: "pw".into(),
            uid: 42,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        Some(Arc::clone(&arm)),
    );

    assert_eq!(arm.uid.load(Ordering::Relaxed), 42);
    play.stop_slot("alice");

    assert!(arm.stop.load(Ordering::Relaxed));
    assert!(play.queue.lock().status_owner(arm.queue_owner).is_none());
    assert!(!play.spawned.contains("alice"));
    assert!(play.handles.is_empty());
}

#[test]
fn prepare_client_from_shared_template() {
    let cache = Arc::new(Cache::default());
    let cfg = ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    };
    let a = prepare_client(cfg, 1, Arc::clone(&cache), Arc::new(vec![]), Vec::new());
    assert!(Arc::ptr_eq(&a.cache, &cache));
    assert!(!a.error_loading);
}

#[test]
fn slot_status_walk_defaults_cleared() {
    let s = SlotStatus::default();
    assert_eq!((s.walk_x, s.walk_z, s.walk_level), (-1, -1, -1));
}

#[test]
fn slot_status_is_up_requires_scene_2() {
    let loading = SlotStatus {
        username: "s01".into(),
        ingame: true,
        scene_state: 1,
        ..SlotStatus::default()
    };
    assert!(!loading.is_up(), "still loading is not up");
    let mut ready = SlotStatus {
        username: "s01".into(),
        ingame: true,
        scene_state: 2,
        ..SlotStatus::default()
    };
    assert!(ready.is_up());
    ready.ingame = false;
    assert!(!ready.is_up(), "logged out is not up");
}

#[test]
fn copy_stream_bytes_zeros_without_stream() {
    let c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut s = SlotStatus {
        username: "t".into(),
        ..SlotStatus::default()
    };
    copy_stream_bytes(&c, &mut s);
    assert_eq!(s.bytes_in, 0);
    assert_eq!(s.bytes_out, 0);
}

/// The flat slot row mirrors `Client.stream`'s payload byte counters;
/// a completed handshake proves both directions count.
#[test]
fn copy_stream_bytes_mirrors_stream_counters() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        grant_login(&mut s, &log, 2);
        // Keep the socket open briefly so the writer thread flushes the
        // login block before the client drops it.
        thread::sleep(Duration::from_millis(50));
    });
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: addr.port(),
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.login("a", "pw", false).unwrap();
    let mut s = SlotStatus {
        username: "a".into(),
        ..SlotStatus::default()
    };
    copy_stream_bytes(&c, &mut s);
    assert!(s.bytes_in > 0, "handshake reads count as bytes_in");
    assert!(s.bytes_out > 0, "handshake writes count as bytes_out");
    server.join().unwrap();
}

#[test]
fn apply_queue_wait_writes_k_of_n_and_grant_clears() {
    let mut rows = vec![
        SlotStatus {
            username: "a".into(),
            queue_position: -1,
            queue_total: -1,
            ..SlotStatus::default()
        },
        SlotStatus {
            username: "b".into(),
            queue_position: -1,
            queue_total: -1,
            ..SlotStatus::default()
        },
    ];
    apply_queue_wait(
        &mut rows,
        "b",
        Some(host::login_queue::QueuePos {
            position: 2,
            total: 2,
        }),
    );
    assert_eq!(rows[1].queue_position, 2);
    assert_eq!(rows[1].queue_total, 2);
    apply_queue_wait(&mut rows, "b", None);
    assert_eq!(rows[1].queue_position, -1);
    assert_eq!(rows[1].queue_total, -1);
    apply_queue_wait(
        &mut rows,
        "b",
        Some(host::login_queue::QueuePos {
            position: 3,
            total: 0,
        }),
    );
    assert_eq!(
        (rows[1].queue_position, rows[1].queue_total),
        (-1, -1),
        "an invalid producer tuple is cleared at publication"
    );
}

#[test]
fn publish_login_latched_projects_arm_latch_onto_slot_row() {
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        startup_phase: StartupPhase::Queueing,
        ..Default::default()
    }]));
    publish_login_latched(&statuses, "alice", true);
    {
        let rows = statuses.lock().unwrap();
        assert!(rows[0].login_latched);
        assert_eq!(rows[0].startup_phase, StartupPhase::Queueing);
    }
    publish_login_latched(&statuses, "alice", false);
    assert!(!statuses.lock().unwrap()[0].login_latched);
}

#[test]
fn explicit_login_rearm_clears_latch_and_allows_handshake() {
    let arm = SlotArm::new(0, true);
    arm.request_logout();
    let logout = arm.logout_command(true).unwrap();
    arm.acknowledge_logout(logout);
    assert!(!should_handshake(&arm, false));
    arm.arm_explicit_login();
    assert!(should_handshake(&arm, false));
}

#[test]
fn newer_login_survives_older_logout_completion() {
    let arm = SlotArm::new(0, false);
    arm.request_logout();

    // Pause the worker after consuming Logout, then issue the newer command.
    let worker_logout = arm.logout_command(true).unwrap();
    arm.arm_explicit_login();

    // Acknowledging the consumed generation must not overwrite newer intent.
    arm.acknowledge_logout(worker_logout);

    assert!(
        should_handshake(&arm, false),
        "the latest explicit Login must remain armed after an old Logout completes"
    );
    assert!(!arm.login_latched());
    assert!(!arm.wants_logout());
}

#[test]
fn logout_latches_before_worker_tick_and_clears_retry_activity() {
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        startup_phase: StartupPhase::Queueing,
        error: Some("old retry error".into()),
        ..Default::default()
    }]));
    let arm = SlotArm::new(0, false);

    arm.request_logout();
    publish_login_latched_from_arm(&statuses, "alice", &arm);

    assert!(
        arm.login_latched(),
        "controller command owns the latch before any worker tick"
    );
    let rows = statuses.lock().unwrap();
    assert!(rows[0].login_latched);
    assert_eq!(rows[0].error, None, "latched park is current activity");
}

#[test]
fn title_handshake_boundary_publishes_cleared_latch_before_queue_wait() {
    let statuses = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        startup_phase: StartupPhase::Queueing,
        login_latched: true,
        ..Default::default()
    }]));
    let arm = SlotArm::new(0, false);
    arm.request_logout();
    let logout = arm.logout_command(true).unwrap();
    arm.acknowledge_logout(logout);
    publish_login_latched_from_arm(&statuses, "alice", &arm);
    assert!(statuses.lock().unwrap()[0].login_latched);

    arm.arm_explicit_login();
    assert!(should_handshake(&arm, false));
    publish_login_latched_from_arm(&statuses, "alice", &arm);

    let rows = statuses.lock().unwrap();
    assert!(
        !rows[0].login_latched,
        "stale park TRUE must not survive into queue wait"
    );
    assert_eq!(rows[0].startup_phase, StartupPhase::Queueing);
}

#[test]
fn spawn_without_auto_login_does_not_handshake() {
    let arm = SlotArm::new(0, false);
    assert!(!should_handshake(&arm, false));
    arm.arm_explicit_login();
    assert!(should_handshake(&arm, false));
    arm.request_logout();
    assert!(!should_handshake(&arm, false));
    arm.arm_explicit_login();
    assert!(should_handshake(&arm, false));
    assert!(!should_handshake(&arm, true));
}

#[test]
fn login_success_keeps_auto_login_armed_but_disarms_one_shot() {
    // CLI: `new(uid, true)` (auto_login true) stays armed so an unexpected
    // DC re-handshakes.
    let arm = SlotArm::new(0, true);
    let login = arm.login_command(false).unwrap();
    on_login_success(&arm, login);
    assert!(should_handshake(&arm, false));

    // Panel Log in / Login all: armed explicitly, then disarmed after
    // the handshake — a DC sits on the title.
    let arm = SlotArm::new(0, false);
    arm.arm_explicit_login();
    let login = arm.login_command(false).unwrap();
    on_login_success(&arm, login);
    assert!(!should_handshake(&arm, false));

    // A newer intentional logout wins over completion of an old login.
    let arm = SlotArm::new(0, true);
    let login = arm.login_command(false).unwrap();
    arm.request_logout();
    let logout = arm.logout_command(true).unwrap();
    arm.acknowledge_logout(logout);
    on_login_success(&arm, login);
    assert!(!should_handshake(&arm, false));
}

#[test]
fn successful_login_consumes_one_shot_after_non_conflicting_updates() {
    let arm = SlotArm::new(0, false);
    arm.arm_explicit_login();
    let claimed = arm.login_command(false).unwrap();

    // Repeated Log in / Login all and a profile auto-policy refresh agree
    // with the claimed handshake; neither creates a second one-shot login.
    arm.arm_explicit_login();
    arm.set_auto_login(false);
    on_login_success(&arm, claimed);

    assert!(
        !arm.wants_login(),
        "the successful handshake must consume the explicit one-shot"
    );
    assert!(!should_handshake(&arm, false));
}

fn client_after_observed_idle_logout() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut client = Client::new_with_revision(
        ClientConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        },
        client::client::ClientRevision::R289,
    );
    client.ingame = true;
    client.ptype = -1;
    client.stream =
        Some(client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap());
    let (mut server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.shell.idle_cycles = 4500;
    client.game_loop();
    let mut opcode = [0; 1];
    server.read_exact(&mut opcode).unwrap();
    assert_eq!(opcode, [145]);

    let mut packet = client::io::Packet::new(vec![]);
    client.psize = 0;
    client.handle_packet(client::io::ServerProt289::LOGOUT, &mut packet);
    client
}

#[test]
fn tick_flags_latches_an_observed_idle_logout_without_changing_saved_intent() {
    let mut client = client_after_observed_idle_logout();
    let arm = SlotArm::new(7, true);
    arm.reconnect.store(true, Ordering::Relaxed);
    arm.request_logout();

    assert!(!tick_flags(&mut client, &[], &arm));
    assert!(arm.login_latched());
    assert!(!arm.wants_login());
    assert!(arm.auto_login.load(Ordering::Relaxed));
    assert!(arm.reconnect.load(Ordering::Relaxed));
    assert!(arm.wants_logout());
    assert!(!should_handshake(&arm, false));
    assert_eq!(client.take_session_exit_observation(), None);
}

#[test]
fn unclassified_server_logout_leaves_auto_login_armed() {
    let cfg = ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    };
    let mut client = Client::new_with_revision(cfg, client::client::ClientRevision::R289);
    client.ingame = true;
    let mut packet = client::io::Packet::new(vec![]);
    client.psize = 0;
    client.handle_packet(client::io::ServerProt289::LOGOUT, &mut packet);
    let arm = SlotArm::new(7, true);

    assert!(!tick_flags(&mut client, &[], &arm));
    assert!(!arm.login_latched());
    assert!(arm.wants_login());
    assert!(arm.auto_login.load(Ordering::Relaxed));
    assert!(should_handshake(&arm, false));
}

#[test]
fn tick_flags_presses_logout_when_ingame_and_reports_stop() {
    let cfg = ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    };
    let mut ifaces = vec![None; 10];
    let com = IfType {
        client_code: api::interact::CC_LOGOUT,
        ..Default::default()
    };
    ifaces[7] = Some(Box::new(com));
    let mut client = prepare_client(
        cfg,
        1,
        Arc::new(Cache::default()),
        Arc::new(ifaces.clone()),
        Vec::new(),
    );
    client.ingame = true;
    let arm = SlotArm::new(0, false);
    arm.request_logout();
    // Even with stop already set, the logout probe must return false so
    // the body keeps running until !ingame (no dirty disconnect).
    arm.stop.store(true, Ordering::Relaxed);

    assert!(!tick_flags(&mut client, &ifaces, &arm));
    assert!(!arm.wants_logout());
    assert!(arm.login_latched());
    assert!(!arm.wants_login());
    assert_eq!(
        client.out.data()[0],
        client::io::ClientProt::IF_BUTTON.id as u8
    );

    // After the logout press, a later probe honors stop.
    assert!(tick_flags(&mut client, &ifaces, &arm));

    // A title slot never presses; `stop` still reports.
    client.ingame = false;
    arm.request_logout();
    assert!(tick_flags(&mut client, &ifaces, &arm));
    assert!(
        arm.wants_logout(),
        "no CC_LOGOUT press on the title; the flag stays for the panel"
    );
}

#[test]
fn refused_logout_stays_pending_with_command_latch() {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    });
    client.ingame = true;
    let arm = SlotArm::new(0, false);
    arm.request_logout();

    assert!(!tick_flags(&mut client, &[], &arm));
    assert!(
        arm.wants_logout(),
        "missing logout interface must leave the request pending"
    );
    assert!(
        arm.login_latched(),
        "the controller latch records intent before the packet succeeds"
    );
}

/// Fill the default 29-grant / 60 s idle address limit so the next request has
/// to wait instead of being granted on arrival.
fn fill_address_window(queue: &SharedLoginQueue) -> Instant {
    let now = Instant::now();
    let mut q = queue.lock();
    for i in 0..29 {
        assert!(matches!(
            request_test_owner(&mut q, 1000 + i, now),
            Permit::Grant
        ));
        assert!(q.acknowledge_login_return(1000 + i, now));
    }
    now
}

/// `(queue_position, queue_total)` of `name`'s published row.
fn row_queue(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) -> (i32, i32) {
    let all = statuses.lock().unwrap();
    all.iter()
        .find(|s| s.username == name)
        .map(|s| (s.queue_position, s.queue_total))
        .expect("slot status row")
}

fn rows(names: &[&str]) -> Arc<Mutex<Vec<SlotStatus>>> {
    Arc::new(Mutex::new(
        names
            .iter()
            .map(|n| SlotStatus {
                username: (*n).into(),
                ..SlotStatus::default()
            })
            .collect(),
    ))
}

#[test]
fn wait_for_permit_returns_without_reenqueue_when_stop_set() {
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = rows(&["alice"]);
    let arm = SlotArm::new(7, true);
    // Fill the 29-grant address TTL so alice waits on the FIFO.
    {
        let now = fill_address_window(&queue);
        let mut queue = queue.lock();
        queue.enqueue_owner(arm.queue_owner, 7);
        assert!(matches!(
            queue.poll_owner(arm.queue_owner, 7, now),
            Permit::Wait(_)
        ));
    }
    // Simulate stop_slot: leave then set stop; the waiter must not recreate
    // the worker owner's place.
    queue.lock().leave_owner(arm.queue_owner);
    arm.stop.store(true, Ordering::Relaxed);
    assert_eq!(
        wait_for_permit(&queue, &statuses, "alice", 7, &arm),
        PermitWait::Cancelled
    );
    assert!(
        queue.lock().status_owner(arm.queue_owner).is_none(),
        "stop must not re-enqueue after leave"
    );
}

#[test]
fn wait_for_permit_grant_clears_the_published_place() {
    // Grant: the accepted handshake pops the FIFO place, so the card and
    // any queue position must be gone (no pending login anywhere).
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = rows(&["alice"]);
    let arm = SlotArm::new(7, true);
    assert_eq!(
        wait_for_permit(&queue, &statuses, "alice", 7, &arm),
        PermitWait::Granted
    );
    assert!(queue.lock().acknowledge_login_return(7, Instant::now()));
    assert!(queue.lock().status_owner(arm.queue_owner).is_none());
    assert_eq!(row_queue(&statuses, "alice"), (-1, -1));
}

#[test]
fn late_prefer_snapshot_cannot_survive_title_cleanup() {
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = rows(&["alice"]);
    let stale = {
        let mut q = queue.lock();
        q.prefer_owner(test_queue_owner(7));
        q.status_owner(test_queue_owner(7))
    };
    assert_eq!(request_shared(&queue, 7, Instant::now()), Permit::Grant);
    apply_queue_wait(&mut statuses.lock().unwrap(), "alice", None);
    apply_queue_wait(&mut statuses.lock().unwrap(), "alice", stale);
    drop_queue_place(&queue, &statuses, "alice", test_queue_owner(7));
    assert_eq!(row_queue(&statuses, "alice"), (-1, -1));
}

#[test]
fn cancellation_after_grant_before_login_abandons_the_unused_permit() {
    let queue = Arc::new(QueueMutex::new(LoginQueue::new(
        Duration::ZERO,
        1,
        Duration::from_secs(60),
    )));
    let statuses = rows(&["alice"]);
    let arm = SlotArm::new(7, true);
    assert_eq!(
        wait_for_permit(&queue, &statuses, "alice", 7, &arm),
        PermitWait::Granted
    );

    arm.withdraw_login();
    assert!(granted_permit_may_start_login(&queue, 7, &arm, false).is_none());
    assert_eq!(
        request_shared(&queue, 8, Instant::now()),
        Permit::Grant,
        "an unused grant must not spend the address attempt"
    );
}

#[test]
fn stop_after_grant_before_login_abandons_the_unused_permit() {
    let queue = Arc::new(QueueMutex::new(LoginQueue::new(
        Duration::ZERO,
        1,
        Duration::from_secs(60),
    )));
    let statuses = rows(&["alice"]);
    let arm = SlotArm::new(7, true);
    assert_eq!(
        wait_for_permit(&queue, &statuses, "alice", 7, &arm),
        PermitWait::Granted
    );

    arm.stop.store(true, Ordering::Relaxed);
    assert!(granted_permit_may_start_login(&queue, 7, &arm, false).is_none());
    assert_eq!(
        request_shared(&queue, 8, Instant::now()),
        Permit::Grant,
        "a stopped slot must release a grant it never used"
    );
}

#[test]
fn login_error_return_acknowledges_the_reserved_attempt() {
    let base = Instant::now();
    let queue = Arc::new(QueueMutex::new(LoginQueue::new(
        Duration::ZERO,
        1,
        Duration::from_secs(60),
    )));
    assert_eq!(request_shared(&queue, 7, base), Permit::Grant);
    let mut permit = GrantedReservation::new(&queue, 7);
    let login: Result<(), &str> =
        login_and_acknowledge_permit(&mut permit, || Err("connect failed"));
    assert_eq!(login, Err("connect failed"));
    assert!(
        !queue.lock().abandon_permit(7),
        "an error return spent and acknowledged the permit"
    );
    assert_eq!(
        request_shared(&queue, 8, base + Duration::from_secs(61)),
        Permit::Grant,
        "the conservative completion clock eventually expires"
    );
}

#[test]
fn panic_in_login_attempt_resolves_the_reservation() {
    let base = Instant::now();
    let queue = Arc::new(QueueMutex::new(LoginQueue::new(
        Duration::ZERO,
        1,
        Duration::from_secs(60),
    )));
    assert_eq!(request_shared(&queue, 7, base), Permit::Grant);

    let unwind = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut permit = GrantedReservation::new(&queue, 7);
        let _: Result<(), ()> =
            login_and_acknowledge_permit(&mut permit, || panic!("synthetic client.login unwind"));
    }));
    assert!(unwind.is_err());
    assert!(
        !queue.lock().abandon_permit(7),
        "the unwind guard resolved the pending reservation"
    );
    assert_eq!(
        request_shared(&queue, 8, base + Duration::from_secs(61)),
        Permit::Grant
    );
}

#[test]
fn login_queue_mutex_survives_panicking_owner() {
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let unwind = std::panic::catch_unwind(AssertUnwindSafe({
        let queue = Arc::clone(&queue);
        move || {
            let _guard = queue.lock();
            panic!("synthetic queue owner unwind");
        }
    }));
    assert!(unwind.is_err());
    assert_eq!(request_shared(&queue, 7, Instant::now()), Permit::Grant);
}

#[test]
fn status_readers_recover_before_terminal_publication() {
    let play = run_with_io(
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
    );
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    });

    let statuses = Arc::clone(&play.statuses);
    let poisoner = thread::spawn(move || {
        let _rows = statuses.lock().unwrap();
        panic!("synthetic in-flight status panic");
    });
    assert!(poisoner.join().is_err());
    assert!(play.statuses.is_poisoned());

    let snapshot = play.statuses();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].username, "alice");
    assert!(
        !play.statuses.is_poisoned(),
        "the first reader repairs poison before another worker can cascade"
    );

    set_startup_phase(&play.statuses, "alice", StartupPhase::Connecting);
    assert_eq!(play.statuses()[0].startup_phase, StartupPhase::Connecting);
}

#[test]
fn panicking_spawned_worker_retires_its_place_and_unblocks_follower() {
    let mut play = run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let window_started = fill_address_window(&play.queue);
    let dead = SlotArm::new(7, true);
    dead.bypass_asset_startup_for_test();
    let (queued, panic_worker) = dead.hold_worker_queue_panic_for_test();
    let dead_owner = dead.queue_owner;
    play.spawn_slot(profile("dead", 7), None, None, Some(Arc::clone(&dead)));
    queued
        .recv_timeout(Duration::from_secs(2))
        .expect("the spawned worker must publish its blocked FIFO place");
    assert!(play.queue.lock().status_owner(dead_owner).is_some());

    let script = play
        .scripts
        .lock()
        .unwrap()
        .get("dead")
        .cloned()
        .expect("spawn registers its script slot");
    let script_poisoner = thread::spawn(move || {
        let _script = script.lock().unwrap();
        panic!("synthetic script slot panic");
    });
    assert!(script_poisoner.join().is_err());

    let statuses = Arc::clone(&play.statuses);
    let poisoner = thread::spawn(move || {
        let _statuses = statuses.lock().unwrap();
        panic!("synthetic status publisher panic");
    });
    assert!(poisoner.join().is_err());
    panic_worker.send(()).unwrap();
    while !play
        .handles
        .get("dead")
        .is_some_and(thread::JoinHandle::is_finished)
    {
        thread::yield_now();
    }
    assert!(play.queue.lock().status_owner(dead_owner).is_none());
    {
        let rows = play.statuses();
        let dead_row = rows.iter().find(|row| row.username == "dead").unwrap();
        assert_eq!((dead_row.queue_position, dead_row.queue_total), (-1, -1));
        assert_eq!(dead_row.worker_terminal, Some(WorkerTerminal::Panicked));
        assert!(
            dead_row
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("slot worker panicked:")),
            "caught unwind must leave a terminal operator message"
        );
    }
    assert_eq!(
        request_shared(&play.queue, 8, window_started + Duration::from_secs(61)),
        Permit::Grant
    );
    assert!(play
        .queue
        .lock()
        .acknowledge_login_return(8, window_started + Duration::from_secs(61)));

    assert_eq!(play.reap_finished_workers(), vec!["dead"]);
    assert!(play.arm("dead").is_none());
    assert!(!play.spawned.contains("dead"));
    assert!(!play.handles.contains_key("dead"));
    assert!(!play.scripts.lock().unwrap().contains_key("dead"));
}

#[test]
fn finished_worker_is_reaped_before_explicit_respawn() {
    let mut play = run_with_io(
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
    );
    let stale = SlotArm::new(7, false);
    play.arms.insert("dead".into(), Arc::clone(&stale));
    play.spawned.insert("dead".into());
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "dead".into(),
        startup_phase: StartupPhase::Error,
        error: Some("bound preparation failed".into()),
        ..SlotStatus::default()
    });
    let (exited, observe_exit) = std::sync::mpsc::channel();
    play.handles.insert(
        "dead".into(),
        thread::spawn(move || {
            // Last action in the worker: the receiver is the explicit
            // lifecycle handshake, not a scheduler-yield budget.
            exited.send(()).unwrap();
        }),
    );
    observe_exit.recv().unwrap();
    // The send is the worker's final action, but JoinHandle publishes
    // `is_finished` only after the closure and captures are fully dropped.
    // Wait without an arbitrary iteration/time budget: this worker has no
    // remaining blocking operation after the explicit exit handshake.
    while !play.handles["dead"].is_finished() {
        thread::yield_now();
    }

    let replacement = SlotArm::new(8, false);
    replacement.bypass_asset_startup_for_test();
    let (entered, release, _) = replacement.hold_worker_start_for_test();
    play.try_spawn_slot(
        profile("dead", 8),
        None,
        None,
        Some(Arc::clone(&replacement)),
    )
    .unwrap();

    assert!(
        Arc::ptr_eq(&play.arm("dead").unwrap(), &replacement),
        "an explicit restart must replace the finished worker's stale arm"
    );
    entered
        .recv_timeout(Duration::from_secs(2))
        .expect("replacement worker did not start");
    release.send(()).unwrap();
    play.stop_slot("dead");
}

#[test]
fn auto_off_before_first_wait_withdraws_auto_intent() {
    let arm = SlotArm::new(7, true);
    arm.set_auto_login(false);
    assert!(permit_wait_cancelled(&arm));
    assert!(!arm.wants_login());
}

#[test]
fn auto_on_arms_an_unlatched_parked_slot() {
    let arm = SlotArm::new(7, false);
    arm.set_auto_login(true);
    assert!(arm.wants_login());
    assert!(!permit_wait_cancelled(&arm));
}

#[test]
fn waiting_slot_withdraws_when_auto_login_is_cleared() {
    // Auto-login armed the intent (`SlotArm::new(uid, true)`). Clearing
    // the checkbox while the slot waits must withdraw the request: no
    // FIFO place, no published k of n, and no handshake afterwards.
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = rows(&["alice"]);
    let arm = SlotArm::new(7, true);
    fill_address_window(&queue);
    let waiter = {
        let queue = Arc::clone(&queue);
        let statuses = Arc::clone(&statuses);
        let arm = Arc::clone(&arm);
        thread::spawn(move || wait_for_permit(&queue, &statuses, "alice", 7, &arm))
    };
    assert!(
        wait_until(2000, || row_queue(&statuses, "alice") == (1, 1)),
        "a waiting slot publishes k of n, got {:?}",
        row_queue(&statuses, "alice")
    );

    arm.set_auto_login(false);

    assert_eq!(waiter.join().unwrap(), PermitWait::Cancelled);
    assert!(queue.lock().status_owner(arm.queue_owner).is_none());
    assert!(
        !arm.wants_login(),
        "a withdrawn intent must not handshake on the next loop"
    );
    assert_eq!(row_queue(&statuses, "alice"), (-1, -1));
}

#[test]
fn explicit_login_intent_survives_auto_on_then_off() {
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = rows(&["alice"]);
    let arm = SlotArm::new(7, false);
    arm.arm_explicit_login();
    arm.set_auto_login(true);
    arm.set_auto_login(false);
    assert!(
        arm.wants_login(),
        "auto toggles must not relabel and withdraw explicit intent"
    );
    assert_eq!(
        wait_for_permit(&queue, &statuses, "alice", 7, &arm),
        PermitWait::Granted
    );
    assert!(queue.lock().abandon_permit(7));
}

#[test]
fn waiting_slot_withdraws_on_intentional_logout_and_frees_the_head() {
    // `Session::logout` clears the intent and latches. The waiting slot
    // must abandon the request without consuming the head, so the member
    // that queued behind it takes position 1 and the next grant.
    let queue = Arc::new(QueueMutex::new(LoginQueue::default()));
    let statuses = rows(&["alice", "bob"]);
    let alice = SlotArm::new(7, true);
    let now = fill_address_window(&queue);
    let waiter = {
        let queue = Arc::clone(&queue);
        let statuses = Arc::clone(&statuses);
        let arm = Arc::clone(&alice);
        thread::spawn(move || wait_for_permit(&queue, &statuses, "alice", 7, &arm))
    };
    assert!(
        wait_until(2000, || row_queue(&statuses, "alice") == (1, 1)),
        "alice queues first, got {:?}",
        row_queue(&statuses, "alice")
    );
    // bob asks while she waits, so he lands behind her.
    assert!(matches!(
        request_shared(&queue, 8, Instant::now()),
        Permit::Wait(_)
    ));
    assert!(
        wait_until(2000, || row_queue(&statuses, "alice") == (1, 2)),
        "alice waits ahead of bob, got {:?}",
        row_queue(&statuses, "alice")
    );

    // Credentials Logout withdraws login and arms the IF logout atomically.
    alice.request_logout();

    assert_eq!(waiter.join().unwrap(), PermitWait::Cancelled);
    assert!(queue.lock().status_owner(alice.queue_owner).is_none());
    assert_eq!(row_queue(&statuses, "alice"), (-1, -1));
    let bob = queue
        .lock()
        .status_owner(test_queue_owner(8))
        .expect("bob keeps his place");
    assert_eq!((bob.position, bob.total), (1, 1));
    // The withdrawn request must not have spent a grant: once the 60 s
    // window elapses, bob's handshake is next.
    let later = now + Duration::from_secs(61);
    assert_eq!(request_shared(&queue, 8, later), Permit::Grant);
}

#[test]
fn retried_login_re_enters_at_the_fifo_tail() {
    // Backoff/reconnect: a rejected handshake leaves nothing behind, and
    // the retry joins the FIFO tail instead of jumping the members that
    // queued while it slept.
    let queue = Arc::new(QueueMutex::new(LoginQueue::new(
        Duration::from_secs(60),
        30,
        Duration::from_secs(60),
    )));
    let statuses = rows(&["alice", "bob"]);
    let alice = SlotArm::new(7, true);
    assert_eq!(
        wait_for_permit(&queue, &statuses, "alice", 7, &alice),
        PermitWait::Granted
    );
    assert!(queue.lock().acknowledge_login_return(7, Instant::now()));
    assert!(
        queue.lock().status_owner(alice.queue_owner).is_none(),
        "a granted login holds no place while it backs off"
    );
    assert_eq!(row_queue(&statuses, "alice"), (-1, -1));

    // bob asks first; alice's retry must land behind him.
    let now = Instant::now();
    assert!(matches!(request_shared(&queue, 8, now), Permit::Wait(_)));
    let retry = {
        let queue = Arc::clone(&queue);
        let statuses = Arc::clone(&statuses);
        let arm = Arc::clone(&alice);
        thread::spawn(move || wait_for_permit(&queue, &statuses, "alice", 7, &arm))
    };
    assert!(
        wait_until(2000, || row_queue(&statuses, "alice") == (2, 2)),
        "the retry queues behind bob, got {:?}",
        row_queue(&statuses, "alice")
    );
    assert_eq!(queue.lock().queued_uids(), vec![8, 7]);

    alice.stop.store(true, Ordering::Relaxed);
    assert_eq!(retry.join().unwrap(), PermitWait::Cancelled);
    queue.lock().leave_owner(test_queue_owner(8));
}

#[test]
fn login_all_during_loading_scene_grants_every_parked_owner() {
    let mut play = run_with_io(
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
    );
    let alice = SlotArm::new(1, true);
    let bob = SlotArm::new(2, false);
    // Duplicate device UIDs are legal throttle identities but distinct slots.
    let carol = SlotArm::new(2, false);
    for (name, arm) in [
        ("alice", Arc::clone(&alice)),
        ("bob", Arc::clone(&bob)),
        ("carol", Arc::clone(&carol)),
    ] {
        play.attach_arm(name, arm);
    }
    play.statuses.lock().unwrap().extend([
        SlotStatus {
            username: "alice".into(),
            startup_phase: StartupPhase::Connecting,
            ..SlotStatus::default()
        },
        SlotStatus {
            username: "bob".into(),
            ..SlotStatus::default()
        },
        SlotStatus {
            username: "carol".into(),
            ..SlotStatus::default()
        },
    ]);

    // Alice owns a granted reservation and is between client.login and the
    // first ready observation when Login all lands.
    let alice_login = alice.login_command(false).unwrap();
    assert_eq!(
        wait_for_permit(&play.queue, &play.statuses, "alice", 1, &alice),
        PermitWait::Granted
    );
    assert!(play
        .queue
        .lock()
        .acknowledge_login_return(1, Instant::now()));
    on_login_success(&alice, alice_login);
    set_startup_phase(&play.statuses, "alice", StartupPhase::LoadingScene);

    play.prefer_login("alice");
    for arm in [&alice, &bob, &carol] {
        arm.arm_explicit_login();
    }
    assert!(
        play.login_queue_uids().is_empty(),
        "arming intent never creates control-thread membership"
    );
    assert_eq!(row_queue(&play.statuses, "alice"), (-1, -1));

    assert_eq!(
        wait_for_permit_bounded(&play.queue, &play.statuses, "bob", 2, &bob),
        PermitWait::Granted
    );
    assert!(play
        .queue
        .lock()
        .acknowledge_login_return(2, Instant::now()));
    assert_eq!(
        wait_for_permit_bounded(&play.queue, &play.statuses, "carol", 2, &carol),
        PermitWait::Granted
    );
    assert!(play
        .queue
        .lock()
        .acknowledge_login_return(2, Instant::now()));
    assert!(play.login_queue_uids().is_empty());
}

#[test]
fn focus_selects_the_sampled_slot() {
    let mut play = run_with_io(
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
    );
    assert_eq!(play.focused(), None, "no slot is focused before focus()");
    let b = SlotArm::new(11, false);
    let c = SlotArm::new(12, false);
    play.arms.insert("b".into(), Arc::clone(&b));
    play.arms.insert("c".into(), Arc::clone(&c));
    *play.queue.lock() = LoginQueue::new(Duration::from_secs(1), 30, Duration::from_secs(60));
    play.focus("b");
    assert_eq!(play.focused().as_deref(), Some("b"));
    let now = Instant::now();
    {
        let mut q = play.queue.lock();
        assert!(q.queued_uids().is_empty(), "focus must not reserve a login");
        q.enqueue_owner(b.queue_owner, 11);
        assert_eq!(q.poll_owner(b.queue_owner, 11, now), Permit::Grant);
        q.enqueue_owner(c.queue_owner, 12);
        assert!(matches!(
            q.poll_owner(c.queue_owner, 12, now),
            Permit::Wait(_)
        ));
        q.enqueue_owner(b.queue_owner, 11);
        assert!(matches!(
            q.poll_owner(b.queue_owner, 11, now),
            Permit::Wait(_)
        ));
        assert_eq!(
            q.queued_uids(),
            vec![11, 12],
            "focused reconnect has priority"
        );
    }
    play.focus("c");
    assert_eq!(play.login_queue_uids(), vec![12, 11]);
    assert_eq!(
        play.focused().as_deref(),
        Some("c"),
        "focus only records the sampled slot — no socket is touched"
    );
}

#[test]
fn stop_slot_clears_focus_on_the_stopped_name() {
    let mut play = run_with_io(
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
    );
    // No real client: fake arm + a handle that exits only once
    // `stop_slot` flags it.
    let arm = SlotArm::new(7, false);
    play.arms.insert("alice".into(), Arc::clone(&arm));
    let watchdog = {
        let arm = Arc::clone(&arm);
        thread::spawn(move || {
            while !arm.stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1));
            }
        })
    };
    play.handles.insert("alice".into(), watchdog);
    play.spawned.insert("alice".into());
    play.focus("alice");
    assert_eq!(play.focused().as_deref(), Some("alice"));

    play.stop_slot("alice");

    assert_eq!(
        play.focused(),
        None,
        "focus must not dangle on a stopped slot"
    );
}

#[test]
fn player_world_tile_adds_build_base_to_route_head() {
    // Lumbridge courtyard: base 3200,3200 + route 22,20 → 3222,3220.
    assert_eq!(player_world_tile(3200, 3200, 22, 20), (3222, 3220));
    // Catherby range door from: base 2752,3392 + route 64,45 → 2816,3437.
    assert_eq!(player_world_tile(2752, 3392, 64, 45), (2816, 3437));
    assert_ne!(
        player_world_tile(3200, 3200, 22, 20),
        (22 * 128, 20 * 128),
        "must not report scene pixels"
    );
}

fn open_world(w: usize, h: usize) -> NavWorld {
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk: vec![0u8; w * h],
            blocked: vec![0u64; (w * h).div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

#[test]
fn route_publication_rejects_stale_results_and_preserves_route_on_failure() {
    let old = Route {
        legs: vec![],
        dest: WorldTile {
            x: 1,
            z: 1,
            level: 0,
        },
        ticks: 0.0,
    };
    let next = Route {
        legs: vec![],
        dest: WorldTile {
            x: 2,
            z: 2,
            level: 0,
        },
        ticks: 0.0,
    };
    let mut bot = NavBot {
        route: Some(old.clone()),
        route_generation: 2,
        requested_route: Some((
            WorldTile {
                x: 2,
                z: 2,
                level: 0,
            },
            0,
            true,
            false,
            false,
        )),
        ..Default::default()
    };
    bot.publish_route(1, 0, true, RouteOutcome::Routed(next.clone()));
    assert_eq!(bot.route, Some(old.clone()));
    assert!(!bot.walk_outcome_failed);
    bot.publish_route(2, 0, true, RouteOutcome::NoPath);
    assert_eq!(bot.route, Some(old));
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_seq, 1);
    assert_eq!(bot.walk_outcome_generation, 2);
    assert_eq!(bot.walk_outcome_x, 2);
    assert_eq!(bot.walk_outcome_z, 2);
    assert_eq!(bot.walk_outcome_radius, 0);
    assert!(bot.walk_outcome_allow_teleports);
    assert!(bot.requested_route.is_none());
    bot.publish_route(2, 0, true, RouteOutcome::Routed(next.clone()));
    assert_eq!(bot.route, Some(next));
    assert!(bot.allow_teleports);
}

#[test]
fn stale_generation_same_target_nopath_does_not_publish() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let mut bot = NavBot {
        route_generation: 2,
        requested_route: Some(native_requested(dest, 1, false)),
        ..Default::default()
    };
    bot.publish_route(1, 0, false, RouteOutcome::NoPath);
    assert!(
        !bot.walk_outcome_failed,
        "superseded same-target NoPath must not publish"
    );
    assert_eq!(bot.walk_outcome_seq, 0);
    assert_eq!(bot.requested_route, Some(native_requested(dest, 1, false)));
    bot.publish_route(2, 0, false, RouteOutcome::NoPath);
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_generation, 2);
    assert_eq!(bot.walk_outcome_x, dest.x);
    assert_eq!(bot.walk_outcome_z, dest.z);
    assert!(bot.requested_route.is_none());
}

#[test]
fn failed_radius_search_can_retry_same_destination_with_old_route_retained() {
    let old = Route {
        legs: vec![],
        dest: WorldTile {
            x: 1,
            z: 1,
            level: 0,
        },
        ticks: 0.0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "retry".to_string(),
        NavBot {
            route: Some(old.clone()),
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(3, 3))),
        navs: Arc::clone(&navs),
        name: "retry".into(),
        state: None,
        bank: vec![],
    };
    // Both searches have no in-world approach tile. Each call must actually
    // run a new search, while retaining the unrelated route on failure.
    for expected_generation in 1..=2 {
        assert!(arm.route_with_radius(100, 100, 0, FindOptions::default(), 1));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let all = navs.lock().unwrap();
            let bot = &all["retry"];
            assert_eq!(bot.route_generation, expected_generation);
            assert_eq!(bot.route, Some(old.clone()));
            if bot.route_worker.is_none() {
                break;
            }
            drop(all);
            assert!(
                std::time::Instant::now() < deadline,
                "route worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[test]
fn exact_walk_near_retargets_active_nearby_route_and_rejects_stale_worker() {
    let nearby = WorldTile {
        x: 3,
        z: 3,
        level: 0,
    };
    let exact = WorldTile {
        x: 2,
        z: 2,
        level: 0,
    };
    let old = Route {
        legs: vec![],
        dest: nearby,
        ticks: 0.0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "bank".to_string(),
        NavBot {
            route: Some(old.clone()),
            route_generation: 1,
            route_worker: Some(Arc::new(())),
            requested_route: Some(native_requested(nearby, 1, false)),
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };

    assert!(
        arm.route_with_radius(exact.x, exact.z, exact.level, FindOptions::default(), 0),
        "explicit WalkNear radius 0 must retarget while a nearby route is active"
    );
    assert!(
        !arm.route(4, 4, 0, FindOptions::default()),
        "ordinary Walk stays non-retargeting while a route or worker is busy"
    );

    let pending = {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("bank").expect("nav bot");
        assert_eq!(bot.route_generation, 2);
        assert_eq!(bot.requested_route, Some(native_requested(exact, 0, false)));
        assert_eq!(bot.route.as_ref().map(|r| r.dest), Some(nearby));
        assert!(bot.route_worker.is_some());
        let pending = bot
            .pending_route
            .take()
            .expect("exact request coalesces onto the in-flight worker");
        assert_eq!(pending.generation, 2);
        assert_eq!(pending.radius, 0);
        assert_eq!(pending.to, exact);
        pending
    };

    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("bank").expect("nav bot");
        bot.publish_route(1, 0, false, RouteOutcome::Routed(old.clone()));
        assert_eq!(
            bot.route.as_ref().map(|r| r.dest),
            Some(nearby),
            "stale nearby worker result must not replace the exact request"
        );
        bot.publish_route(2, 0, false, pending.calculate());
        assert_eq!(bot.route.as_ref().map(|r| r.dest), Some(exact));
    }

    assert!(arm.route_with_radius(exact.x, exact.z, exact.level, FindOptions::default(), 0));
    assert_eq!(navs.lock().unwrap()["bank"].route_generation, 2);
}

/// R2-1: WalkNear B retargets while walk A's route is still followed. A's
/// route ending inside that window is not B's route end: B's wait must not
/// settle there. Once B's own route is installed its end publishes for B.
#[test]
fn route_end_of_a_retargeted_walk_does_not_settle_the_new_walk() {
    let a = WorldTile {
        x: 6,
        z: 6,
        level: 0,
    };
    let b = WorldTile {
        x: 1,
        z: 1,
        level: 0,
    };
    let route_to = |dest| Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "bank".to_string(),
        NavBot {
            route_generation: 1,
            requested_route: Some(native_requested(a, 2, false)),
            walk_request_id: 7,
            ..Default::default()
        },
    )])));
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("bank").unwrap();
        bot.publish_route(1, 7, false, RouteOutcome::Routed(route_to(a)));
        // A find is still in flight, so B's retarget coalesces onto it and
        // B's route is not installed yet: the retarget window.
        bot.route_worker = Some(Arc::new(()));
    }
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.queue_route(b.x, b.z, b.level, FindOptions::default(), 1, true, 8));

    let mut all = navs.lock().unwrap();
    let bot = all.get_mut("bank").unwrap();
    assert_eq!(bot.walk_request_id, 8, "B is the armed walk");
    assert_eq!(
        bot.route.as_ref().map(|r| r.dest),
        Some(a),
        "A still followed"
    );
    apply_nav_follow_outcome(
        bot,
        Some(nav::traveller::TravelOutcome::Arrived { at: a }),
        false,
    );
    assert!(bot.route.is_none());
    assert_eq!(
        bot.walk_outcome_seq, 0,
        "A's route end must not settle B's wait"
    );

    bot.publish_route(2, 8, false, RouteOutcome::Routed(route_to(b)));
    apply_nav_follow_outcome(
        bot,
        Some(nav::traveller::TravelOutcome::Arrived { at: b }),
        false,
    );
    assert_eq!(bot.walk_outcome_seq, 1, "B's own route end publishes");
    assert!(!bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_request_id, 8);
    assert_eq!((bot.walk_outcome_x, bot.walk_outcome_z), (b.x, b.z));
    assert_eq!(bot.walk_outcome_radius, 1);
}

/// The failure side of R2-1: walk A's route stalling inside B's retarget
/// window must not fail B's wait. B's own route stalling still does.
#[test]
fn stall_of_a_retargeted_walk_does_not_fail_the_new_walk() {
    let a = WorldTile {
        x: 6,
        z: 6,
        level: 0,
    };
    let b = WorldTile {
        x: 1,
        z: 1,
        level: 0,
    };
    let route_to = |dest| Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let gave_up = |at| Some(nav::traveller::TravelOutcome::GaveUp { at, hops: 40 });
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "bank".to_string(),
        NavBot {
            route_generation: 1,
            requested_route: Some(native_requested(a, 2, false)),
            walk_request_id: 7,
            ..Default::default()
        },
    )])));
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("bank").unwrap();
        bot.publish_route(1, 7, false, RouteOutcome::Routed(route_to(a)));
        bot.route_worker = Some(Arc::new(()));
    }
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.queue_route(b.x, b.z, b.level, FindOptions::default(), 1, true, 8));

    let mut all = navs.lock().unwrap();
    let bot = all.get_mut("bank").unwrap();
    assert_eq!(bot.walk_request_id, 8, "B is the armed walk");
    assert_eq!(
        bot.route.as_ref().map(|r| r.dest),
        Some(a),
        "A still followed"
    );
    apply_nav_follow_outcome(bot, gave_up(a), false);
    assert!(bot.route.is_none(), "the stalled route is dropped");
    assert_eq!(bot.walk_outcome_seq, 0, "A's stall must not fail B's wait");
    assert!(bot.pending_route.is_some(), "B's find is still owed");

    bot.publish_route(2, 8, false, RouteOutcome::Routed(route_to(b)));
    apply_nav_follow_outcome(bot, gave_up(b), false);
    assert_eq!(bot.walk_outcome_seq, 1, "B's own stall publishes");
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_request_id, 8);
    assert_eq!((bot.walk_outcome_x, bot.walk_outcome_z), (b.x, b.z));
}

/// Request id 0 (ctx.walk, old buffers) never settles a wait: its route end
/// publishes nothing, so a success still leaves `walk_outcome_seq` unmoved.
#[test]
fn route_end_of_request_id_zero_publishes_nothing() {
    let dest = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let mut bot = NavBot {
        route_generation: 1,
        requested_route: Some(native_requested(dest, 2, false)),
        ..Default::default()
    };
    bot.publish_route(
        1,
        0,
        false,
        RouteOutcome::Routed(Route {
            legs: vec![],
            dest,
            ticks: 0.0,
        }),
    );
    apply_nav_follow_outcome(
        &mut bot,
        Some(nav::traveller::TravelOutcome::Arrived { at: dest }),
        false,
    );
    assert!(bot.route.is_none(), "the route ended");
    assert_eq!(bot.walk_outcome_seq, 0, "request id 0 publishes nothing");
}

#[test]
fn bank_fetch_session_refuses_exact_walk_near() {
    let dest = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "bank".to_string(),
        NavBot {
            bank_fetch: Some(PendingBankFetch {
                steps: VecDeque::new(),
                dest,
                opts: FindOptions::default(),
                final_route: Route {
                    legs: vec![],
                    dest,
                    ticks: 0.0,
                },
            }),
            route_generation: 1,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };
    assert!(
        !arm.route_with_radius(2, 2, 0, FindOptions::default(), 0),
        "a latched bank-fetch session must refuse exact WalkNear"
    );
    let bot = &navs.lock().unwrap()["bank"];
    assert_eq!(bot.route_generation, 1);
    assert!(bot.pending_route.is_none());
    assert!(bot.requested_route.is_none());
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_generation, 1);
    assert_eq!(bot.walk_outcome_x, 2);
    assert_eq!(bot.walk_outcome_z, 2);
    assert_eq!(bot.walk_outcome_radius, 0);
    assert!(bot.bank_fetch.is_some());
}

#[test]
fn missing_nav_world_publishes_a_failed_walk_outcome() {
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: None,
        navs: Arc::clone(&navs),
        name: "noworld".into(),
        state: None,
        bank: vec![],
    };
    assert!(!arm.route_with_radius(2820, 3556, 0, FindOptions::default(), 1));
    let bot = &navs.lock().unwrap()["noworld"];
    assert!(bot.walk_outcome_failed);
    assert_eq!(
        bot.route_generation, 0,
        "refuse must not bump route_generation"
    );
    assert_eq!(bot.walk_outcome_seq, 1);
    assert_eq!(bot.walk_outcome_generation, 0);
    assert_eq!(bot.walk_outcome_request_id, 0);
    assert_eq!(bot.walk_outcome_x, 2820);
    assert_eq!(bot.walk_outcome_z, 3556);
    assert_eq!(bot.walk_outcome_radius, 1);
    assert!(bot.route.is_none());
    assert!(bot.requested_route.is_none());
}

#[test]
fn abort_clears_walk_outcome_without_settling_a_stale_generation() {
    let dest = WorldTile {
        x: 2,
        z: 2,
        level: 0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "abort".to_string(),
        NavBot {
            route_generation: 4,
            requested_route: Some(native_requested(dest, 0, false)),
            walk_outcome_seq: 3,
            walk_outcome_failed: true,
            walk_outcome_generation: 4,
            walk_outcome_x: dest.x,
            walk_outcome_z: dest.z,
            ..Default::default()
        },
    )])));
    abort_script_walk(&navs, "abort");
    let bot = &navs.lock().unwrap()["abort"];
    assert_eq!(bot.route_generation, 5);
    assert!(!bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_seq, 4);
    bot_clone_generation_guard(bot);
}

fn bot_clone_generation_guard(bot: &NavBot) {
    let mut bot = NavBot {
        route_generation: bot.route_generation,
        walk_outcome_seq: bot.walk_outcome_seq,
        walk_outcome_failed: bot.walk_outcome_failed,
        requested_route: Some((
            WorldTile {
                x: 2,
                z: 2,
                level: 0,
            },
            0,
            false,
            false,
            false,
        )),
        ..Default::default()
    };
    bot.publish_route(4, 0, false, RouteOutcome::NoPath);
    assert!(
        !bot.walk_outcome_failed,
        "stale generation after abort must not republish"
    );
}

fn posted_from_bot(bot: &NavBot) -> PostedWalkOutcome {
    PostedWalkOutcome {
        seq: bot.walk_outcome_seq,
        generation: bot.walk_outcome_generation,
        request_id: bot.walk_outcome_request_id,
        failed: bot.walk_outcome_failed,
        x: bot.walk_outcome_x,
        z: bot.walk_outcome_z,
        level: bot.walk_outcome_level,
        radius: bot.walk_outcome_radius,
        allow_teleports: bot.walk_outcome_allow_teleports,
    }
}

fn encode_walk_snapshot(tick: u64, here: (i32, i32, i32), outcome: PostedWalkOutcome) -> Vec<u8> {
    with_script_snapshot_input(
        tick,
        Some(here),
        true,
        None,
        None,
        None,
        None,
        None,
        false,
        false,
        false,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        outcome,
        PostedInspect::default(),
        script::isolate_fb::encode_snapshot_with_native,
    )
}

fn walk_resilient_src(x: i32, z: i32, radius: i32) -> String {
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    async loop() {{
        globalThis.__rs_ok = null;
        globalThis.__rs_ok = await Traversal.walkTo(
            {{ x: {x}, z: {z}, level: 0 }},
            {{ radius: {radius}, timeoutMs: 300000 }},
        );

    }}
}}
"#
    )
}

fn overlapping_walk_src(x: i32, z: i32, radius: i32) -> String {
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__rs_done) return;
        globalThis.__rs_a = null;
        globalThis.__rs_b = null;
        Traversal.walkTo(
            {{ x: {x}, z: {z}, level: 0 }},
            {{ radius: {radius}, timeoutMs: 300000 }},
        ).then(v => {{ globalThis.__rs_a = v; }});
        globalThis.__rs_b = await Traversal.walkTo(
            {{ x: {x}, z: {z}, level: 0 }},
            {{ radius: {radius}, timeoutMs: 300000 }},
        );

        globalThis.__rs_done = true;
    }}
}}
"#
    )
}

fn park_walk_isolate(
    iso: &script::LoadIsolate,
    x: i32,
    z: i32,
    radius: i32,
    prior: PostedWalkOutcome,
) -> u64 {
    iso.post_snapshot(encode_walk_snapshot(1, (0, 0, 0), prior));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    let drained = iso.drain_interacts();
    let request_id = match &drained[..] {
        [script::shim::InteractReq::WalkNear {
            x: dx,
            z: dz,
            level: 0,
            radius: dr,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id,
        }] if *dx == x && *dz == z && *dr == radius => *request_id,
        other => panic!("unexpected interacts: {other:?}"),
    };
    assert_ne!(request_id, 0);
    request_id
}

fn park_two_walks(iso: &script::LoadIsolate, x: i32, z: i32, radius: i32) -> (u64, u64) {
    iso.post_snapshot(encode_walk_snapshot(
        1,
        (0, 0, 0),
        PostedWalkOutcome::default(),
    ));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.probe("__rs_b").unwrap(), serde_json::Value::Null);
    let drained = iso.drain_interacts();
    match &drained[..] {
        [script::shim::InteractReq::WalkNear {
            x: dx0,
            z: dz0,
            level: 0,
            radius: dr0,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: first,
        }, script::shim::InteractReq::WalkNear {
            x: dx1,
            z: dz1,
            level: 0,
            radius: dr1,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: second,
        }] if *dx0 == x
            && *dz0 == z
            && *dr0 == radius
            && *dx1 == x
            && *dz1 == z
            && *dr1 == radius =>
        {
            assert_ne!(*first, 0);
            assert_ne!(*second, 0);
            assert_ne!(first, second);
            (*first, *second)
        }
        other => panic!("unexpected interacts: {other:?}"),
    }
}

#[test]
fn bank_fetch_refuse_echoes_request_id_without_bumping_route() {
    let dest = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "bank".to_string(),
        NavBot {
            bank_fetch: Some(PendingBankFetch {
                steps: VecDeque::new(),
                dest,
                opts: FindOptions::default(),
                final_route: Route {
                    legs: vec![],
                    dest,
                    ticks: 0.0,
                },
            }),
            route_generation: 1,
            walk_outcome_seq: 3,
            walk_outcome_generation: 1,
            walk_outcome_failed: true,
            walk_outcome_x: dest.x,
            walk_outcome_z: dest.z,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };
    assert!(!arm.queue_route(2820, 3556, 0, FindOptions::default(), 1, true, 42));
    let bot = &navs.lock().unwrap()["bank"];
    assert_eq!(bot.route_generation, 1);
    assert!(bot.bank_fetch.is_some());
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_seq, 4);
    assert_eq!(bot.walk_outcome_generation, 1);
    assert_eq!(bot.walk_outcome_request_id, 42);
    assert_eq!(bot.walk_outcome_x, 2820);
    assert_eq!(bot.walk_outcome_z, 3556);
    assert_eq!(bot.walk_outcome_radius, 1);
}

#[test]
fn late_same_target_cannot_publish_when_outcome_generation_lags_route() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let mut bot = NavBot {
        route_generation: 4,
        requested_route: Some(native_requested(dest, 1, false)),
        walk_outcome_seq: 3,
        walk_outcome_generation: 2,
        walk_outcome_failed: true,
        walk_outcome_x: dest.x,
        walk_outcome_z: dest.z,
        walk_outcome_radius: 1,
        ..Default::default()
    };
    bot.publish_route(2, 9, false, RouteOutcome::NoPath);
    assert_eq!(
        bot.walk_outcome_seq, 3,
        "lagging same-target worker must not publish"
    );
    assert_eq!(bot.walk_outcome_generation, 2);
    assert_eq!(bot.requested_route, Some(native_requested(dest, 1, false)));
    bot.publish_route(4, 11, false, RouteOutcome::NoPath);
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_generation, 4);
    assert_eq!(bot.walk_outcome_request_id, 11);
    assert_eq!(bot.walk_outcome_seq, 4);
    assert!(bot.requested_route.is_none());
}

#[test]
fn mid_follow_terminals_publish_the_armed_request_id() {
    let dest = WorldTile {
        x: 2,
        z: 2,
        level: 0,
    };
    let route = Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let mut bot = NavBot {
        route: Some(route.clone()),
        route_request_id: 7,
        route_generation: 4,
        walk_request_id: 7,
        requested_route: Some(native_requested(dest, 0, false)),
        walk_outcome_seq: 3,
        walk_outcome_generation: 2,
        walk_outcome_failed: false,
        bank_fetch: Some(PendingBankFetch {
            steps: VecDeque::from([BankStep::Walk {
                x: dest.x,
                z: dest.z,
                level: dest.level,
            }]),
            dest,
            opts: FindOptions::default(),
            final_route: route.clone(),
        }),
        ..Default::default()
    };
    apply_nav_follow_outcome(
        &mut bot,
        Some(nav::traveller::TravelOutcome::Refused {
            at: dest,
            reason: api::interact::SendReason::NotIngame,
        }),
        false,
    );
    assert!(bot.route.is_none());
    assert!(
        bot.bank_fetch.is_some(),
        "non-stand Refused must not drop an unrelated bank-fetch"
    );
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_request_id, 7);
    assert_eq!(bot.walk_outcome_seq, 4);

    bot.route = Some(route);
    bot.walk_request_id = 8;
    bot.route_request_id = 8;
    apply_nav_follow_outcome(
        &mut bot,
        Some(nav::traveller::TravelOutcome::Blocked {
            at: dest,
            leg: 0,
            detail: "missing loc".into(),
        }),
        true,
    );
    assert!(bot.route.is_none());
    assert!(bot.bank_fetch.is_none());
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_request_id, 8);
    assert_eq!(bot.walk_outcome_seq, 5);
}

#[test]
fn missing_nav_world_refusal_settles_the_matching_isolate_wait() {
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: None,
        navs: Arc::clone(&navs),
        name: "noworld".into(),
        state: None,
        bank: vec![],
    };
    let iso = script::LoadIsolate::spawn(
        walk_resilient_src(2820, 3556, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let request_id = park_walk_isolate(&iso, 2820, 3556, 1, PostedWalkOutcome::default());
    assert!(!arm.queue_route(2820, 3556, 0, FindOptions::default(), 1, true, request_id));
    let posted = posted_from_bot(&navs.lock().unwrap()["noworld"]);
    assert_eq!(posted.generation, 0);
    assert_eq!(posted.request_id, request_id);
    assert_eq!(posted.seq, 1);
    iso.post_snapshot(encode_walk_snapshot(2, (0, 0, 0), posted));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn two_same_target_requests_delayed_old_outcome_does_not_settle() {
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: None,
        navs: Arc::clone(&navs),
        name: "noworld".into(),
        state: None,
        bank: vec![],
    };
    let iso = script::LoadIsolate::spawn(
        walk_resilient_src(2820, 3556, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let first_id = park_walk_isolate(&iso, 2820, 3556, 1, PostedWalkOutcome::default());
    assert!(!arm.queue_route(2820, 3556, 0, FindOptions::default(), 1, true, first_id));
    let first = posted_from_bot(&navs.lock().unwrap()["noworld"]);
    assert_eq!(first.request_id, first_id);
    // First wait still pending. Delayed old outcome for a different id
    // must not settle; the actual first_id refusal does.
    iso.post_snapshot(encode_walk_snapshot(
        2,
        (0, 0, 0),
        PostedWalkOutcome {
            seq: 99,
            generation: 0,
            request_id: first_id.wrapping_add(3),
            failed: true,
            x: 2820,
            z: 3556,
            level: 0,
            radius: 1,
            allow_teleports: false,
        },
    ));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    iso.post_snapshot(encode_walk_snapshot(3, (0, 0, 0), first));
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn same_key_walk_near_refuses_distinct_id_and_keeps_inflight() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let old = Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "coal".to_string(),
        NavBot {
            route: Some(old.clone()),
            route_generation: 1,
            route_worker: Some(Arc::new(())),
            requested_route: Some(native_requested(dest, 1, false)),
            walk_request_id: 7,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((2823, 3555, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "coal".into(),
        state: None,
        bank: vec![],
    };
    assert!(
        arm.queue_route(
            dest.x,
            dest.z,
            dest.level,
            FindOptions::default(),
            1,
            true,
            7
        ),
        "exact retransmission of the armed id must coalesce"
    );
    assert!(
        arm.queue_route(
            dest.x,
            dest.z,
            dest.level,
            FindOptions::default(),
            1,
            true,
            0
        ),
        "legacy request_id 0 must keep the in-flight route"
    );
    assert!(
        !arm.queue_route(
            dest.x,
            dest.z,
            dest.level,
            FindOptions::default(),
            1,
            true,
            9
        ),
        "a distinct wait must fail-close while the first route stays"
    );
    let bot = &navs.lock().unwrap()["coal"];
    assert_eq!(bot.route_generation, 1);
    assert_eq!(bot.walk_request_id, 7);
    assert_eq!(bot.requested_route, Some(native_requested(dest, 1, false)));
    assert_eq!(bot.route.as_ref().map(|r| r.dest), Some(dest));
    assert!(bot.route_worker.is_some());
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_request_id, 9);
    assert_eq!(bot.walk_outcome_x, dest.x);
    assert_eq!(bot.walk_outcome_z, dest.z);
    assert_eq!(bot.walk_outcome_radius, 1);
}

#[test]
fn unpublished_wait_refusal_survives_legacy_zero_and_yields_to_newer_wait() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let old = Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "coal".to_string(),
        NavBot {
            route: Some(old.clone()),
            route_generation: 1,
            route_worker: Some(Arc::new(())),
            requested_route: Some(native_requested(dest, 1, false)),
            walk_request_id: 7,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((2823, 3555, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "coal".into(),
        state: None,
        bank: vec![],
    };
    assert!(!arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        9
    ));
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("coal").expect("nav bot");
        assert_eq!(bot.walk_outcome_request_id, 9);
        bot.note_failure(bot.route_generation, 0, dest, 1, false);
        assert_eq!(
            bot.walk_outcome_request_id, 9,
            "legacy id 0 must not replace an unpublished current wait refusal"
        );
        bot.publish_route(1, 7, false, RouteOutcome::NoPath);
        assert_eq!(bot.walk_outcome_request_id, 9);
        assert_eq!(bot.route.as_ref().map(|r| r.dest), Some(dest));
        assert_eq!(bot.walk_request_id, 7);
        assert!(bot.requested_route.is_none());
        bot.requested_route = Some(native_requested(dest, 1, false));
    }
    assert!(!arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        11
    ));
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("coal").expect("nav bot");
        assert_eq!(
            bot.walk_outcome_request_id, 11,
            "a newer wait failure must still be publishable"
        );
        bot.publish_route(1, 7, false, RouteOutcome::NoPath);
        assert_eq!(bot.walk_outcome_request_id, 11);
        bot.mark_walk_outcome_posted(bot.walk_outcome_seq);
        bot.requested_route = Some(native_requested(dest, 1, false));
        bot.publish_route(1, 7, false, RouteOutcome::NoPath);
        assert_eq!(
            bot.walk_outcome_request_id, 7,
            "after snapshot post the armed NoPath may occupy the outcome slot"
        );
        bot.clear_walk_outcome();
        assert_eq!(bot.walk_outcome_request_id, 0);
        assert!(!bot.walk_outcome_failed);
        assert_eq!(bot.walk_live_refusal_id, 0);
    }
}

#[test]
fn same_key_pending_route_refuses_distinct_id() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let world = Arc::new(open_world(7, 7));
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "pend".to_string(),
        NavBot {
            route_generation: 1,
            pending_route: Some(ScriptRouteRequest {
                generation: 1,
                request_id: 7,
                world: Arc::clone(&world),
                from: WorldTile {
                    x: 2823,
                    z: 3555,
                    level: 0,
                },
                to: dest,
                radius: 1,
                opts: FindOptions::default(),
                state: None,
                bank: vec![],
                live_candidates: None,
                completion: Default::default(),
            }),
            requested_route: Some(native_requested(dest, 1, false)),
            walk_request_id: 7,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((2823, 3555, 0)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "pend".into(),
        state: None,
        bank: vec![],
    };
    assert!(!arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        9
    ));
    let bot = &navs.lock().unwrap()["pend"];
    assert_eq!(bot.route_generation, 1);
    assert_eq!(bot.walk_request_id, 7);
    assert!(bot.pending_route.is_some());
    assert_eq!(bot.pending_route.as_ref().map(|p| p.request_id), Some(7));
    assert!(bot.route_worker.is_none());
    assert!(bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_request_id, 9);
}

#[test]
fn two_same_key_walk_near_refuses_later_wait_and_old_nopath_does_not_settle_it() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let old = Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let iso = script::LoadIsolate::spawn(
        overlapping_walk_src(dest.x, dest.z, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let (first_id, second_id) = park_two_walks(&iso, dest.x, dest.z, 1);
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "coal".to_string(),
        NavBot {
            route: Some(old.clone()),
            route_generation: 1,
            route_worker: Some(Arc::new(())),
            requested_route: Some(native_requested(dest, 1, false)),
            walk_request_id: first_id,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((2823, 3555, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "coal".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        first_id
    ));
    assert!(!arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        second_id
    ));
    let posted_second = posted_from_bot(&navs.lock().unwrap()["coal"]);
    assert_eq!(posted_second.request_id, second_id);
    assert!(posted_second.failed);
    iso.post_snapshot(encode_walk_snapshot(2, (2823, 3555, 0), posted_second));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_b").unwrap(), false);
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);

    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("coal").expect("nav bot");
        assert_eq!(bot.walk_request_id, first_id);
        assert_eq!(bot.route.as_ref().map(|r| r.dest), Some(dest));
        // The isolate already observed the current wait refusal. A later
        // coalesced NoPath may now occupy the single outcome slot.
        bot.mark_walk_outcome_posted(bot.walk_outcome_seq);
        bot.publish_route(1, first_id, false, RouteOutcome::NoPath);
        assert_eq!(
            bot.route.as_ref().map(|r| r.dest),
            Some(dest),
            "coalesced NoPath must retain the armed route"
        );
        assert!(bot.requested_route.is_none());
        assert_eq!(bot.walk_outcome_request_id, first_id);
    }
    let posted_first = posted_from_bot(&navs.lock().unwrap()["coal"]);
    iso.post_snapshot(encode_walk_snapshot(3, (2823, 3555, 0), posted_first));
    iso.on_game_tick(3);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        false,
        "delayed first NoPath must not unset the later wait"
    );
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);
    iso.join();
}

#[test]
fn two_same_key_old_nopath_before_snapshot_keeps_later_refusal() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let old = Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let iso = script::LoadIsolate::spawn(
        overlapping_walk_src(dest.x, dest.z, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let (first_id, second_id) = park_two_walks(&iso, dest.x, dest.z, 1);
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "coal".to_string(),
        NavBot {
            route: Some(old.clone()),
            route_generation: 1,
            route_worker: Some(Arc::new(())),
            requested_route: Some(native_requested(dest, 1, false)),
            walk_request_id: first_id,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((2823, 3555, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "coal".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        first_id
    ));
    assert!(!arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        second_id
    ));
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("coal").expect("nav bot");
        assert_eq!(bot.walk_outcome_request_id, second_id);
        assert_eq!(bot.walk_request_id, first_id);
        bot.publish_route(1, first_id, false, RouteOutcome::NoPath);
        assert_eq!(
            bot.walk_outcome_request_id, second_id,
            "older coalesced NoPath must not overwrite an unpublished current wait refusal"
        );
        assert!(bot.walk_outcome_failed);
        assert_eq!(bot.walk_request_id, first_id);
        assert_eq!(bot.route_generation, 1);
        assert_eq!(bot.route.as_ref().map(|r| r.dest), Some(dest));
        assert!(bot.route_worker.is_some());
        assert!(bot.requested_route.is_none());
    }
    let posted = posted_from_bot(&navs.lock().unwrap()["coal"]);
    assert_eq!(posted.request_id, second_id);
    assert!(posted.failed);
    iso.post_snapshot(encode_walk_snapshot(2, (2823, 3555, 0), posted));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        false,
        "later wait must settle false from its own refusal after pre-snapshot NoPath"
    );
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);
    iso.join();
}

#[test]
fn two_same_key_old_mid_follow_terminal_before_snapshot_keeps_later_refusal() {
    let dest = WorldTile {
        x: 2820,
        z: 3556,
        level: 0,
    };
    let old = Route {
        legs: vec![],
        dest,
        ticks: 0.0,
    };
    let iso = script::LoadIsolate::spawn(
        overlapping_walk_src(dest.x, dest.z, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let (first_id, second_id) = park_two_walks(&iso, dest.x, dest.z, 1);
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "coal".to_string(),
        NavBot {
            route: Some(old.clone()),
            route_generation: 1,
            route_worker: Some(Arc::new(())),
            requested_route: Some(native_requested(dest, 1, false)),
            walk_request_id: first_id,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((2823, 3555, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "coal".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        first_id
    ));
    assert!(!arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        second_id
    ));
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("coal").expect("nav bot");
        assert_eq!(bot.walk_outcome_request_id, second_id);
        apply_nav_follow_outcome(
            bot,
            Some(nav::traveller::TravelOutcome::GaveUp { at: dest, hops: 3 }),
            false,
        );
        assert_eq!(
            bot.walk_outcome_request_id, second_id,
            "older mid-follow terminal must not overwrite an unpublished current wait refusal"
        );
        assert!(bot.walk_outcome_failed);
        assert_eq!(bot.walk_request_id, first_id);
        assert_eq!(bot.route_generation, 1);
        assert!(bot.route.is_none());
        assert!(bot.route_worker.is_some());
    }
    let posted = posted_from_bot(&navs.lock().unwrap()["coal"]);
    assert_eq!(posted.request_id, second_id);
    assert!(posted.failed);
    iso.post_snapshot(encode_walk_snapshot(2, (2823, 3555, 0), posted));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        false,
        "later wait must settle false from its own refusal after pre-snapshot terminal"
    );
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);
    iso.join();
}

#[test]
fn new_isolate_does_not_consume_prior_host_outcome() {
    let iso1 = script::LoadIsolate::spawn(
        walk_resilient_src(2820, 3556, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let old_id = park_walk_isolate(&iso1, 2820, 3556, 1, PostedWalkOutcome::default());
    iso1.join();
    let leftover = PostedWalkOutcome {
        seq: 4,
        generation: 1,
        request_id: old_id,
        failed: true,
        x: 2820,
        z: 3556,
        level: 0,
        radius: 1,
        allow_teleports: false,
    };
    let iso2 = script::LoadIsolate::spawn(
        walk_resilient_src(2820, 3556, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let new_id = park_walk_isolate(&iso2, 2820, 3556, 1, leftover);
    assert_ne!(
        new_id, old_id,
        "new isolate must not reuse the old wait token"
    );
    assert_eq!(
        iso2.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "leftover host outcome must not settle the new isolate wait"
    );
    iso2.post_snapshot(encode_walk_snapshot(
        2,
        (2823, 3555, 0),
        PostedWalkOutcome { seq: 5, ..leftover },
    ));
    iso2.on_game_tick(2);
    assert_eq!(
        iso2.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "late old-worker request id must not settle the new isolate wait"
    );
    iso2.join();
}

#[test]
fn bank_fetch_refusal_echoes_isolate_request_id() {
    let dest = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let prior = PostedWalkOutcome {
        seq: 3,
        generation: 1,
        request_id: 3,
        failed: true,
        x: dest.x,
        z: dest.z,
        level: 0,
        radius: 0,
        allow_teleports: false,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "bank".to_string(),
        NavBot {
            bank_fetch: Some(PendingBankFetch {
                steps: VecDeque::new(),
                dest,
                opts: FindOptions::default(),
                final_route: Route {
                    legs: vec![],
                    dest,
                    ticks: 0.0,
                },
            }),
            route_generation: 1,
            walk_outcome_seq: prior.seq,
            walk_outcome_generation: prior.generation,
            walk_outcome_request_id: prior.request_id,
            walk_outcome_failed: prior.failed,
            walk_outcome_x: prior.x,
            walk_outcome_z: prior.z,
            walk_outcome_level: prior.level,
            walk_outcome_radius: prior.radius,
            ..Default::default()
        },
    )])));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };
    let iso = script::LoadIsolate::spawn(
        walk_resilient_src(2820, 3556, 1),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let request_id = park_walk_isolate(&iso, 2820, 3556, 1, prior);
    assert!(!arm.queue_route(2820, 3556, 0, FindOptions::default(), 1, true, request_id));
    let posted = posted_from_bot(&navs.lock().unwrap()["bank"]);
    assert_eq!(posted.generation, 1);
    assert_eq!(posted.request_id, request_id);
    assert_eq!(posted.seq, 4);
    iso.post_snapshot(encode_walk_snapshot(2, (0, 0, 0), posted));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn exact_walk_near_replaces_published_nearby_route() {
    let exact = WorldTile {
        x: 1,
        z: 1,
        level: 0,
    };
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "bank".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.route_with_radius(6, 6, 0, FindOptions::default(), 1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let all = navs.lock().unwrap();
        let bot = all.get("bank").expect("nav bot");
        if bot.route_worker.is_none() && bot.route.is_some() {
            break;
        }
        drop(all);
        assert!(
            std::time::Instant::now() < deadline,
            "nearby worker did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let nearby_dest = navs.lock().unwrap()["bank"]
        .route
        .as_ref()
        .map(|route| route.dest);
    assert_ne!(nearby_dest, Some(exact));

    assert!(arm.route_with_radius(exact.x, exact.z, exact.level, FindOptions::default(), 0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let all = navs.lock().unwrap();
        let bot = all.get("bank").expect("nav bot");
        if bot.route_worker.is_none() && bot.route.as_ref().map(|route| route.dest) == Some(exact) {
            break;
        }
        drop(all);
        assert!(
            std::time::Instant::now() < deadline,
            "exact worker did not publish dest"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn approach_candidates_avoid_occupied_target_and_stay_in_radius() {
    let mut world = open_world(7, 7);
    world.collision.blocked[0] |= 1 << (3 * 7 + 3);
    world.collision.walk[3 * 7 + 2] = 1; // A face wall does not occupy its floor tile.
    let target = WorldTile {
        x: 3,
        z: 3,
        level: 0,
    };
    let candidates = approach_tiles(
        &world,
        WorldTile {
            x: 0,
            z: 3,
            level: 0,
        },
        target,
        1,
    );
    assert_eq!(candidates.len(), 8);
    assert!(candidates
        .iter()
        .all(|t| *t != target && (t.x - 3).abs() <= 1 && (t.z - 3).abs() <= 1));
    assert_eq!(candidates[0].x, 2);
    assert!(approach_tiles(&world, target, target, 0).is_empty());
}

#[test]
fn radius_calculate_keeps_first_connected_open_floor_approach() {
    let world = Arc::new(open_world(7, 7));
    let request = ScriptRouteRequest {
        generation: 0,
        request_id: 0,
        world,
        from: WorldTile {
            x: 2,
            z: 3,
            level: 0,
        },
        to: WorldTile {
            x: 3,
            z: 3,
            level: 0,
        },
        radius: 1,
        opts: FindOptions::default(),
        state: None,
        bank: vec![],
        live_candidates: None,
        completion: Default::default(),
    };
    let RouteOutcome::Routed(route) = request.calculate() else {
        panic!("open floor should route");
    };
    assert_eq!(
        route.dest,
        WorldTile {
            x: 2,
            z: 3,
            level: 0
        }
    );
}

#[test]
fn radius_calculate_drops_wall_separated_candidate() {
    let mut world = open_world(7, 7);
    let mut flags = vec![0u32; 49];
    for z in 2..=4 {
        flags[z * 7 + 3] |= client::dash3d::CollisionFlag::W_E as u32;
        flags[z * 7 + 4] |= client::dash3d::CollisionFlag::W_W as u32;
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    world.collision.walk = walk;
    world.collision.blocked = blocked;
    let request = ScriptRouteRequest {
        generation: 0,
        request_id: 0,
        world: Arc::new(world),
        from: WorldTile {
            x: 4,
            z: 3,
            level: 0,
        },
        to: WorldTile {
            x: 3,
            z: 3,
            level: 0,
        },
        radius: 1,
        opts: FindOptions::default(),
        state: None,
        bank: vec![],
        live_candidates: None,
        completion: Default::default(),
    };
    let RouteOutcome::Routed(route) = request.calculate() else {
        panic!("same-room approach should route");
    };
    assert_ne!(
        route.dest,
        WorldTile {
            x: 4,
            z: 3,
            level: 0
        }
    );
    assert!(
        (route.dest.x - request.to.x)
            .abs()
            .max((route.dest.z - request.to.z).abs())
            <= 1
    );
    assert!(
        nav::router::local_step_component(&request.world.collision, request.to, 1)
            .contains(&route.dest),
        "selected destination must be step-connected to the target"
    );
}

#[test]
fn radius_calculate_uses_occupied_target_approach_candidates() {
    let mut world = open_world(7, 7);
    let target = WorldTile {
        x: 3,
        z: 3,
        level: 0,
    };
    world.collision.blocked[0] |= 1 << (target.z as usize * 7 + target.x as usize);
    let request = ScriptRouteRequest {
        generation: 0,
        request_id: 0,
        world: Arc::new(world),
        from: WorldTile {
            x: 2,
            z: 3,
            level: 0,
        },
        to: target,
        radius: 1,
        opts: FindOptions::default(),
        state: None,
        bank: vec![],
        live_candidates: None,
        completion: Default::default(),
    };
    let RouteOutcome::Routed(route) = request.calculate() else {
        panic!("occupied target should route to a neighbour");
    };
    assert_ne!(route.dest, target);
    assert!(
        (route.dest.x - target.x)
            .abs()
            .max((route.dest.z - target.z).abs())
            <= 1
    );
    assert!(request.world.collision.standable(route.dest));
}

#[test]
fn radius_calculate_respects_wall_l_diagonal_geometry() {
    let mut world = open_world(7, 7);
    let mut flags = vec![0u32; 49];
    let target = WorldTile {
        x: 3,
        z: 3,
        level: 0,
    };
    flags[3 * 7 + 3] =
        client::dash3d::CollisionFlag::W_S as u32 | client::dash3d::CollisionFlag::W_E as u32;
    // Close the east-side pocket: the east neighbour's W_W face blocks
    // both direct and diagonal entry, while its north/south faces keep
    // a route from walking around the pocket inside radius one.
    flags[3 * 7 + 4] |= client::dash3d::CollisionFlag::W_W as u32
        | client::dash3d::CollisionFlag::W_N as u32
        | client::dash3d::CollisionFlag::W_S as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    world.collision.walk = walk;
    world.collision.blocked = blocked;
    let request = ScriptRouteRequest {
        generation: 0,
        request_id: 0,
        world: Arc::new(world),
        from: WorldTile {
            x: 0,
            z: 3,
            level: 0,
        },
        to: target,
        radius: 1,
        opts: FindOptions::default(),
        state: None,
        bank: vec![],
        live_candidates: None,
        completion: Default::default(),
    };
    let component = nav::router::local_step_component(&request.world.collision, target, 1);
    let same_side = WorldTile {
        x: 2,
        z: 3,
        level: 0,
    };
    let far_side = WorldTile {
        x: 4,
        z: 3,
        level: 0,
    };
    assert!(component.contains(&same_side));
    assert!(!component.contains(&far_side), "component={component:?}");
    let RouteOutcome::Routed(route) = request.calculate() else {
        panic!("same-side WALL_L approach should route");
    };
    assert!(component.contains(&route.dest));
    assert_ne!(route.dest, far_side);
}

#[test]
fn radius_calculate_drops_detour_outside_radius() {
    let mut world = open_world(7, 7);
    let mut flags = vec![0u32; 49];
    for z in 1..=5 {
        flags[z * 7 + 3] |= client::dash3d::CollisionFlag::W_E as u32;
        flags[z * 7 + 4] |= client::dash3d::CollisionFlag::W_W as u32;
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    world.collision.walk = walk;
    world.collision.blocked = blocked;
    let target = WorldTile {
        x: 3,
        z: 3,
        level: 0,
    };
    let request = ScriptRouteRequest {
        generation: 0,
        request_id: 0,
        world: Arc::new(world),
        from: WorldTile {
            x: 4,
            z: 3,
            level: 0,
        },
        to: target,
        radius: 1,
        opts: FindOptions::default(),
        state: None,
        bank: vec![],
        live_candidates: None,
        completion: Default::default(),
    };
    let component = nav::router::local_step_component(&request.world.collision, target, 1);
    assert!(!component.contains(&WorldTile {
        x: 4,
        z: 3,
        level: 0
    }));
    let RouteOutcome::Routed(route) = request.calculate() else {
        panic!("global detour should still leave a local approach");
    };
    assert!(component.contains(&route.dest));
    assert_ne!(
        route.dest,
        WorldTile {
            x: 4,
            z: 3,
            level: 0
        }
    );
}

#[test]
#[ignore = "requires the locally generated actual 289 navpack"]
fn actual_289_radius_arrival_stays_out_of_horvik() {
    let world = Arc::new(
        NavWorld::load_pack(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/debug/nav/289/274bot.navpack"
        )))
        .expect("actual 289 navpack"),
    );
    let request = ScriptRouteRequest {
        generation: 0,
        request_id: 0,
        world: Arc::clone(&world),
        from: WorldTile {
            x: 3252,
            z: 3420,
            level: 0,
        },
        to: WorldTile {
            x: 3253,
            z: 3401,
            level: 0,
        },
        radius: 2,
        opts: FindOptions::default(),
        state: None,
        bank: vec![],
        live_candidates: None,
        completion: Default::default(),
    };
    let RouteOutcome::Routed(route) = request.calculate() else {
        panic!("actual 289 route should exist");
    };
    assert_ne!(
        route.dest,
        WorldTile {
            x: 3251,
            z: 3403,
            level: 0,
        }
    );
    assert!(
        route.legs.iter().all(|leg| !matches!(
            leg,
            Leg::Transport { edge } if edge.loc_id == 1530
        )),
        "radius arrival must not use Horvik door 1530"
    );
    assert!(
        nav::router::local_step_component(&world.collision, request.to, 2).contains(&route.dest)
    );
}

#[test]
fn walk_arm_may_follow_freezes_under_hold() {
    assert!(WalkArm::may_follow(false), "unheld follow may poll");
    assert!(!WalkArm::may_follow(true), "hold freezes WalkArm follow");
}

/// The shared walk arm (panel `Session::arm_walk_on` is a thin
/// wrapper over this; the TUI calls it directly) routes over the
/// world and latches the route on the focused uid's arm.
#[test]
fn arm_walk_on_routes_and_latches_the_focused_arm() {
    let world = open_world(3, 3);
    let travellers: Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    let route = arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        dest,
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        &travellers,
        Some("alice"),
    )
    .expect("the open 3x3 world routes");
    assert_eq!((route.dest.x, route.dest.z, route.dest.level), (2, 2, 0));
    let all = travellers.lock().unwrap();
    let arm = all
        .get("alice")
        .expect("the focused slot's walk arm exists");
    assert_eq!(
        arm.lock().unwrap().queued_tile(),
        Some(dest),
        "the arm latches the routed dest"
    );
}

#[test]
fn arm_walk_on_without_focus_latches_no_arm() {
    let world = open_world(3, 3);
    let travellers: Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 2,
            z: 2,
            level: 0,
        },
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        &travellers,
        None,
    )
    .expect("the open 3x3 world routes without a focused slot");
    assert!(
        travellers.lock().unwrap().is_empty(),
        "no focused name to key a walk arm"
    );
}

fn profile(name: &str, uid: i32) -> Profile {
    Profile {
        username: name.into(),
        password: "pw".into(),
        uid,
        settings: ProfileSettings {
            lowmem: true,
            auto_login: false,
            tutorial_skipped: None,
            raster: vault::RasterMode::Gpu,
            random_events: true,
            lamp_skill: "strength".into(),
            lamp_auto: true,
            ..ProfileSettings::default()
        },
    }
}

/// Flat model: spawning two profiles gives two full `Client` slots —
/// one status row and one control arm each, no lean channel bookkeeping.
#[test]
fn two_profiles_spawn_two_client_slots() {
    let mut play = run_with_io(
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
    );
    // `auto_login = false` arms sit on the title (no TCP), so both
    // threads idle on a 20 ms sleep until `stop_slot` joins them.
    play.spawn_slot(profile("a", 1), None, None, Some(SlotArm::new(1, false)));
    play.spawn_slot(profile("b", 2), None, None, Some(SlotArm::new(2, false)));

    assert_eq!(play.arms.len(), 2, "one control arm per profile");
    // The slot threads publish their status rows asynchronously.
    assert!(
        wait_until(500, || {
            let names: Vec<String> = play.statuses().into_iter().map(|s| s.username).collect();
            names.contains(&"a".into()) && names.contains(&"b".into())
        }),
        "each profile's slot thread publishes one status row"
    );
    assert_eq!(play.statuses().len(), 2, "one status row per profile");

    play.stop_slot("a");
    play.stop_slot("b");
    assert_eq!(play.statuses().len(), 0, "both slots stopped");
}

fn grant_login(s: &mut std::net::TcpStream, log: &Mutex<Vec<u8>>, code: u8) {
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).unwrap();
    assert_eq!(hdr[0], 14);
    for _ in 0..8 {
        s.write_all(&[0]).unwrap();
    }
    s.write_all(&[0]).unwrap();
    s.write_all(&[0u8; 8]).unwrap();
    let mut buf = [0u8; 512];
    let n = s.read(&mut buf).unwrap();
    assert!(n > 0);
    log.lock().unwrap().push(buf[0]);
    s.write_all(&[code]).unwrap();
    if code == 2 {
        s.write_all(&[0, 0]).unwrap(); // staff + mouse after grant 2
    }
}

// --- Task 5: per-uid compiled scripts ---

#[test]
fn script_stop_clears_offline_published_paint() {
    let play = run_with_io(
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
    );
    script_slot_or_insert(&play.scripts, "alice");
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        script_paint: Some(Arc::new(script::shim::ScriptPaint::default())),
        ..SlotStatus::default()
    });

    play.script_pause("alice");
    assert!(
        play.statuses.lock().unwrap()[0].script_paint.is_some(),
        "Pause retains the offline paint frame"
    );

    play.script_stop("alice");

    assert!(
        play.statuses.lock().unwrap()[0].script_paint.is_none(),
        "script lifecycle owner clears paint without an online observe"
    );
}

#[test]
fn fenced_script_stop_requires_same_identity_and_generation() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    play.script_start_load(
        "alice",
        "export function tick(api) {}".into(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    play.script_attach_identity("alice", "card:a");
    let generation = play.script_runtime_generation("alice").unwrap();

    assert!(!play.script_stop_if_identity_generation("alice", "card:b", generation));
    assert!(!play.script_stop_if_identity_generation(
        "alice",
        "card:a",
        generation.wrapping_add(1)
    ));
    assert_eq!(play.script_state("alice"), script::RunState::Running);

    assert!(play.script_stop_if_identity_generation("alice", "card:a", generation));
    wait_script_state(&play, "alice", script::RunState::Idle);
}

#[test]
fn script_start_unknown_compiled_id_errors_without_v8() {
    // `script::factory` returns `None` for every picker id until the
    // script is ported (WalkTo is the first port; BoneBurier is not
    // yet); Start must surface that, never a dummy.
    let _play = run_with_io(
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
    );
    // alice is a real (armed) slot, so the error is about the picker id.
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    let err = play
        .script_start("alice", script::CompiledId("BoneBurier"))
        .unwrap_err();
    assert!(err.contains("not ported"), "err was {err}");
}

#[test]
fn script_start_load_spawns_isolate_only_on_start_and_refuses_when_active() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    play.script_start_load(
        "alice",
        src.clone(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);

    let err = play
        .script_start_load(
            "alice",
            src.clone(),
            script::LoadShape::NativeTick,
            None,
            vec![],
        )
        .unwrap_err();
    assert!(err.contains("active"), "err was {err}");

    play.script_stop("alice");
    wait_script_state(&play, "alice", script::RunState::Idle);

    // Unknown slot: never creates an entry, and never a V8 runtime.
    let err = play
        .script_start_load("ghost", src, script::LoadShape::NativeTick, None, vec![])
        .unwrap_err();
    assert!(err.contains("no slot"), "err was {err}");
    assert!(
        !play.scripts.lock().unwrap().contains_key("ghost"),
        "an unknown uid must never get a SlotScript entry"
    );
}

#[test]
fn script_start_returns_before_setup_and_stop_before_reap() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    let t0 = Instant::now();
    play.script_start_load(
        "alice",
        src.clone(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "Start blocked: {:?}",
        t0.elapsed()
    );
    assert_eq!(play.script_state("alice"), script::RunState::Starting);
    wait_script_state(&play, "alice", script::RunState::Running);
    let t1 = Instant::now();
    play.script_stop("alice");
    assert!(
        t1.elapsed() < Duration::from_millis(200),
        "Stop blocked: {:?}",
        t1.elapsed()
    );
    assert_eq!(play.script_state("alice"), script::RunState::Stopping);
    wait_script_state(&play, "alice", script::RunState::Idle);
}

/// The slot thread observes every frame while the UI polls its Starts. A
/// poll that read the outcome and the in-flight state under two locks lost
/// the outcome whenever the slot thread settled it in between; every Start
/// here must be reported exactly once, Ready or Failed, never "not owed".
#[test]
fn script_poll_start_never_loses_an_outcome_to_the_slot_threads_observe() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    let done = Arc::new(AtomicBool::new(false));
    let observer = {
        let scripts = Arc::clone(&play.scripts);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if let Some(slot) = script_slot(&scripts, "alice") {
                    slot.lock().unwrap().observe_lifecycle();
                }
            }
        })
    };
    let good = "export function tick(api) {}";
    let bad = "throw new Error('race-proof');\nexport function tick(api) {}";
    let (mut ready, mut failed) = (0, 0);
    for i in 0..100 {
        let src = if i % 2 == 0 { good } else { bad };
        play.script_start_load(
            "alice",
            src.into(),
            script::LoadShape::NativeTick,
            None,
            vec![],
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match play.script_poll_start("alice") {
                script::StartPoll::Pending => assert!(Instant::now() < deadline, "Start {i}"),
                script::StartPoll::Settled(script::StartOutcome::Ready) => {
                    ready += 1;
                    break;
                }
                script::StartPoll::Settled(script::StartOutcome::Failed(e)) => {
                    assert!(e.contains("race-proof"), "{e}");
                    failed += 1;
                    break;
                }
                other => panic!(
                    "Start {i}: outcome lost ({other:?}) in state {:?}",
                    play.script_state("alice")
                ),
            }
        }
        play.script_stop("alice");
        wait_script_state(&play, "alice", script::RunState::Idle);
    }
    done.store(true, Ordering::Relaxed);
    observer.join().unwrap();
    assert_eq!((ready, failed), (50, 50));
}

#[test]
fn script_start_handle_explicit_loadouts_starts() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    let mut bag = serde_json::Map::new();
    bag.insert("loadout".into(), serde_json::json!("Memory food"));
    bag.insert("banking".into(), serde_json::json!("Auto"));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    play.script_start_handle()
        .start_load_with_loadouts(
            "alice",
            src,
            script::LoadShape::NativeTick,
            Some(bag),
            vec![],
            &[script::Loadout::new("Memory food").with_carry("Lobster", 1)],
        )
        .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    play.script_stop("alice");
    wait_script_state(&play, "alice", script::RunState::Idle);
}

#[test]
fn script_paint_click_is_noop_when_idle_paused_or_unadvertised() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    play.script_paint_click("alice", "gobank", 0);
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    play.script_start_load("alice", src, script::LoadShape::NativeTick, None, vec![])
        .unwrap();
    play.script_paint_click("alice", "", 0);
    play.script_paint_click("alice", "gobank", 0);
    play.script_pause("alice");
    play.script_paint_click("alice", "gobank", 0);
    assert_eq!(play.script_state("alice"), script::RunState::Paused);
    play.script_stop("alice");
    play.script_paint_click("alice", "gobank", 0);
    wait_script_state(&play, "alice", script::RunState::Idle);
}

#[test]
fn script_paint_select_is_noop_when_idle_paused_or_unadvertised() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    play.script_paint_select("alice", "strip:k", "Options", 0);
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    play.script_start_load("alice", src, script::LoadShape::NativeTick, None, vec![])
        .unwrap();
    play.script_paint_select("alice", "strip:k", "", 0);
    play.script_paint_select("alice", "strip:k", "Options", 0);
    play.script_paint_select("alice", "strip:k", "Nope", 0);
    play.script_pause("alice");
    play.script_paint_select("alice", "strip:k", "Options", 0);
    assert_eq!(play.script_state("alice"), script::RunState::Paused);
    play.script_stop("alice");
    play.script_paint_select("alice", "strip:k", "Options", 0);
    wait_script_state(&play, "alice", script::RunState::Idle);
}

#[test]
fn script_paint_select_adverts_fixture_chrome() {
    use script::shim::{PaintChromeBand, ScriptPaint};
    let paint = ScriptPaint {
        generation: 7,
        strip: Some(PaintChromeBand {
            id: "k".into(),
            names: vec!["Statistics".into(), "Options".into()],
            selected: "Statistics".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(script_runtime::script_paint_select_advertised(
        &paint, "strip:k", "Options"
    ));
    assert!(!script_runtime::script_paint_select_advertised(
        &paint, "strip:k", "Nope"
    ));
    assert!(!script_runtime::script_paint_select_advertised(
        &paint, "tabs:mg", "Loot"
    ));
    assert_eq!(paint.generation, 7);
}

#[test]
fn script_start_unknown_slot_errors_without_phantom_entry() {
    let play = run_with_io(
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
    );
    let err = play
        .script_start("ghost", script::CompiledId("WalkTo"))
        .unwrap_err();
    assert!(err.contains("no slot"), "err was {err}");
    assert_eq!(play.script_state("ghost"), script::RunState::Idle);
    assert!(
        !play.scripts.lock().unwrap().contains_key("ghost"),
        "an unknown uid must never get a SlotScript entry"
    );
}

#[test]
fn script_control_is_noop_for_unknown_slot_and_state_defaults_idle() {
    let play = run_with_io(
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
    );
    assert_eq!(play.script_state("ghost"), script::RunState::Idle);
    assert_eq!(play.script_last_error("ghost"), None);
    play.script_pause("ghost");
    play.script_resume("ghost");
    play.script_stop("ghost");
    play.script_paint_click("ghost", "gobank", 0);
    assert_eq!(play.script_state("ghost"), script::RunState::Idle);
    play.cheat("ghost", "tele 0,50,50,20,20");
    assert!(
        play.cheats.lock().unwrap().is_empty(),
        "unknown uid cheat is a no-op"
    );
    play.queue_wire("ghost", WireCmd::Continue);
    play.queue_wire(
        "ghost",
        WireCmd::Walk {
            x: 3220,
            z: 3221,
            level: 0,
        },
    );
    assert!(
        play.wires.lock().unwrap().is_empty(),
        "unknown uid wire is a no-op"
    );
}

#[test]
fn queue_wire_lands_on_the_named_slots_queue() {
    let mut play = run_with_io(
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
    );
    // `auto_login = false` arm sits on the title (no TCP).
    play.spawn_slot(profile("a", 1), None, None, Some(SlotArm::new(1, false)));
    assert!(
        wait_until(500, || play.wires.lock().unwrap().contains_key("a")),
        "the slot thread registers its wire queue at spawn"
    );
    // Queue producers accept work only for a connected session. This
    // test does not run a server, so mark the published row connected to
    // exercise ordering independently of the login harness.
    play.statuses
        .lock()
        .unwrap()
        .iter_mut()
        .find(|status| status.username == "a")
        .unwrap()
        .ingame = true;
    play.queue_wire("a", WireCmd::Continue);
    play.queue_wire("a", WireCmd::Answer(2));
    let queued = play.wires.lock().unwrap().get("a").unwrap().clone();
    assert_eq!(
        queued,
        VecDeque::from([WireCmd::Continue, WireCmd::Answer(2)]),
        "queued wires keep their order"
    );
    play.stop_slot("a");
    assert!(
        !play.wires.lock().unwrap().contains_key("a"),
        "stop_slot drops the slot's wire queue"
    );
}

#[test]
fn disconnected_slot_rejects_new_wire_and_cheat_work() {
    let play = run_with_io(
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
    );
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        ingame: false,
        scene_state: 0,
        ..SlotStatus::default()
    });
    play.cheats
        .lock()
        .unwrap()
        .insert("alice".into(), VecDeque::new());
    play.wires
        .lock()
        .unwrap()
        .insert("alice".into(), VecDeque::new());

    play.cheat("alice", "setvar tutorial 1000");
    play.queue_wire("alice", WireCmd::Continue);

    assert!(play.cheats.lock().unwrap()["alice"].is_empty());
    assert!(play.wires.lock().unwrap()["alice"].is_empty());

    play.statuses.lock().unwrap()[0].ingame = true;
    play.cheat("alice", "setvar tutorial 1000");
    play.queue_wire("alice", WireCmd::Continue);
    assert_eq!(play.cheats.lock().unwrap()["alice"].len(), 1);
    assert_eq!(play.wires.lock().unwrap()["alice"].len(), 1);
}

#[test]
fn disconnect_reset_discards_queued_work_and_pauses_session_state() {
    let ScriptWiring {
        scripts, cheats, ..
    } = script_wiring();
    let wires = Arc::new(Mutex::new(HashMap::from([(
        "alice".to_string(),
        VecDeque::from([WireCmd::Continue]),
    )])));
    cheats
        .lock()
        .unwrap()
        .get_mut("alice")
        .unwrap()
        .push_back("setvar tutorial 1000".into());
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "alice".to_string(),
        NavBot {
            route_generation: 7,
            route_worker: Some(Arc::new(())),
            requested_route: Some((
                WorldTile {
                    x: 1,
                    z: 2,
                    level: 0,
                },
                3,
                false,
                false,
                false,
            )),
            ..NavBot::default()
        },
    )])));

    reset_slot_session_work("alice", &scripts, &cheats, &wires, &navs);

    let script = script_slot(&scripts, "alice").unwrap();
    let script = script.lock().unwrap();
    assert_eq!(script.state(), script::RunState::Paused);
    assert!(script.want_run, "disconnect pause preserves Start intent");
    assert!(cheats.lock().unwrap()["alice"].is_empty());
    assert!(wires.lock().unwrap()["alice"].is_empty());
    let navs = navs.lock().unwrap();
    let nav = &navs["alice"];
    assert_eq!(nav.route_generation, 8);
    assert!(nav.route_worker.is_none());
    assert!(nav.requested_route.is_none());
    assert!(nav.route.is_none());
    assert!(nav.bank_fetch.is_none());
}

/// `dispatch_wires` is safe on a snapshot with nothing open: the
/// Continue/Answer sends refuse (no chat modal) and the driver's out
/// buffer stays untouched.
#[test]
fn dispatch_wires_with_no_modal_stays_quiet() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let snap = GameSnapshot::new();
    dispatch_wires(
        &mut c,
        &snap,
        vec![WireCmd::Continue, WireCmd::Answer(1)],
        false,
    );
    assert_eq!(c.out.pos, 0, "no chat modal → nothing dispatched");
}

/// A guardian hold drops WASD walks (the follow is frozen too) but
/// still lets chat Continue/Answer through.
#[test]
fn dispatch_wires_drops_walk_while_hold_but_keeps_chat() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let snap = GameSnapshot::new();
    dispatch_wires(
        &mut c,
        &snap,
        vec![WireCmd::Walk {
            x: 3220,
            z: 3221,
            level: 0,
        }],
        true,
    );
    assert_eq!(c.out.pos, 0, "hold drops the walk send");
}

/// A client seeded for the shim interact dispatch: ingame with a ready
/// scene at base (3200, 3200), the player at scene (5, 5), a Bank
/// booth loc at scene (5, 6) whose def's second op is `Use-quickly`,
/// an open bank (main modal 600 wrapping withdraw component 601
/// holding Bones × 20 with a `Withdraw 1` op) and its deposit side
/// modal (700 wrapping 701 holding Bones × 3 with a `Deposit All` op).
fn bank_client() -> Client {
    use client::client::ClientPlayer;
    use client::config::{LocType, ObjType};
    // A real (loopback) stream so the snapshot passes `Interactions`'
    // attached precondition, like the nav-client fixture.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
    std::mem::forget(listener);
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 0;
    c.local_player = Some(ClientPlayer::at(5, 5));
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.resize(3, ObjType::default());
        cache.objs[1].id = 1;
        cache.objs[1].name = "Bones".into();
        cache.objs[2].id = 2;
        cache.objs[2].name = "Lobster".into();
        cache
            .locs
            .extend((0..(2214usize.saturating_sub(cache.locs.len()))).map(|_| LocType::default()));
        cache.locs[2213].id = 2213;
        cache.locs[2213].name = "Bank booth".into();
        cache.locs[2213].op = vec![None, Some("Use-quickly".into()), None, None, None];
    }
    // The booth: a wall at scene (5, 6) whose typecode encodes loc 2213.
    // typecode2 is the snapshot loc `info` byte: shape lives in the low
    // 5 bits. OpenBooth readiness (`loc_approach::can_operate_from`) only
    // models footprint shapes 10/11/22 — a 0 shape is not operable, so
    // the fixture must plant shape 10 (same contract as api interact booth
    // fixtures and script_snapshot_fb_projects_authoritative_bank_approach).
    let booth_typecode = 0x4000_0000 + (2213 << 14) + 1 + (2 << 7);
    c.world
        .set_wall(0, 5, 6, 0, 0, 0, booth_typecode, 10, 0, 0, 0, 0);
    // Bank: main modal 600 wrapping the withdraw component 601
    // (Bones × 20, ops `[Withdraw 1, ...]`).
    c.main_modal_id = 600;
    c.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    c.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [
                Some("Withdraw 1".into()),
                Some("Withdraw 5".into()),
                Some("Withdraw 10".into()),
                Some("Withdraw All".into()),
                Some("Withdraw X".into()),
            ],
            ..Default::default()
        },
    );
    c.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![2, 0]),
            link_obj_number: Some(vec![20, 0]),
            ..Default::default()
        },
    );
    // Deposit side modal: 700 wrapping 701 (Bones × 3, `Deposit All`).
    c.side_modal_id = 700;
    c.set_iface(
        700,
        IfType {
            id: 700,
            layer_id: 700,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![701]),
            ..Default::default()
        },
    );
    c.set_iface(
        701,
        IfType {
            id: 701,
            layer_id: 700,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Deposit All".into()), None, None, None, None],
            ..Default::default()
        },
    );
    c.set_iface_mut(
        701,
        IfTypeMut {
            link_obj_type: Some(vec![2, 0]),
            link_obj_number: Some(vec![3, 0]),
            ..Default::default()
        },
    );
    for prot in [
        ServerProt::IF_OPENMAIN,
        ServerProt::IF_OPENCHAT,
        ServerProt::UPDATE_INV_FULL,
        ServerProt::REBUILD_NORMAL,
        ServerProt::PLAYER_INFO,
    ] {
        c.bump_gens(prot);
    }
    c
}

/// Task 7 — the shim's interact requests dispatch through the slot
/// Driver: the booth Use-quickly op at the loc tile, the bank-side
/// Deposit-All for a matching name, the bank withdraw op, and close.
/// A request whose target the snapshot lacks fails closed (nothing is
/// sent).
#[test]
fn dispatch_script_interact_sends_open_deposit_withdraw() {
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let names = api::obj_names::ObjNames::from_objs(&{
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.clone()
    });
    let (navs, world) = empty_nav();
    assert_eq!(
        snap.locs()
            .iter()
            .find(|l| l.tile.x == 3205 && l.tile.z == 3206)
            .map(|l| l.actions.clone()),
        Some(vec![None, Some("Use-quickly".into()), None, None, None]),
        "the seeded booth loc carries the Use-quickly op"
    );
    let out_before = c.out.pos;
    // Each op must send on its own, not be masked by the others:
    // open-booth (Use-quickly), bank-side Deposit-All, a withdraw by
    // name whose hyphenated label resolves to the seeded "Withdraw All"
    // op, and close. `dispatch_script_interact` returns whether the
    // driver's out buffer was written.
    for (label, req) in [
        (
            "open-booth",
            script::shim::InteractReq::OpenBooth {
                x: 3205,
                z: 3206,
                level: 0,
                id: 2213,
                name: None,
                action: None,
            },
        ),
        (
            "deposit",
            script::shim::InteractReq::Deposit {
                name: "Bones".into(),
            },
        ),
        (
            "withdraw",
            script::shim::InteractReq::Withdraw {
                name: "Bones".into(),
                action: "Withdraw-All".into(),
            },
        ),
        ("close", script::shim::InteractReq::Close),
    ] {
        let before = c.out.pos;
        assert!(
            dispatch_script_interact(
                &mut c,
                &snap,
                Some(&names),
                Some((3205, 3205, 0)),
                &navs,
                &world,
                None,
                "alice",
                vec![req],
            ),
            "{label} must dispatch"
        );
        assert!(
            c.out.pos > before,
            "{label} must write the driver (masked by the other ops?)"
        );
    }
    assert!(
        c.out.pos > out_before,
        "open + deposit + withdraw + close wrote to the driver"
    );
    // A request with no matching target sends nothing.
    let out_before = c.out.pos;
    assert!(!dispatch_script_interact(
        &mut c,
        &snap,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![
            script::shim::InteractReq::Deposit {
                name: "Lobster".into()
            },
            script::shim::InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw 1".into()
            },
        ],
    ));
    assert_eq!(
        c.out.pos, out_before,
        "missing targets fail closed: nothing is sent"
    );
}

#[test]
fn accepted_open_booth_cancels_only_the_requesting_slot_walk() {
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (navs, world) = empty_nav();
    let route = Route {
        legs: vec![],
        dest: WorldTile {
            x: 3204,
            z: 3206,
            level: 0,
        },
        ticks: 1.0,
    };
    let requested = Some((
        WorldTile {
            x: 3204,
            z: 3206,
            level: 0,
        },
        0,
        false,
        false,
        false,
    ));
    navs.lock().unwrap().extend([
        (
            "alice".into(),
            NavBot {
                route: Some(route.clone()),
                route_worker: Some(Arc::new(())),
                requested_route: requested,
                ..Default::default()
            },
        ),
        (
            "bob".into(),
            NavBot {
                route: Some(route.clone()),
                route_worker: Some(Arc::new(())),
                requested_route: requested,
                ..Default::default()
            },
        ),
    ]);

    let open = || script::shim::InteractReq::OpenBooth {
        x: 3205,
        z: 3206,
        level: 0,
        id: 2213,
        name: None,
        action: None,
    };
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![open()],
    ));
    let all = navs.lock().unwrap();
    let alice = &all["alice"];
    assert!(alice.route.is_none());
    assert!(alice.route_worker.is_none());
    assert!(alice.pending_route.is_none());
    assert!(alice.requested_route.is_none());
    assert_eq!(all["bob"].route, Some(route));
    assert!(all["bob"].route_worker.is_some());
    assert_eq!(all["bob"].requested_route, requested);
}

#[test]
fn rejected_open_booth_does_not_cancel_the_requesting_slot_walk() {
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (navs, world) = empty_nav();
    let route = Route {
        legs: vec![],
        dest: WorldTile {
            x: 3204,
            z: 3206,
            level: 0,
        },
        ticks: 1.0,
    };
    let requested = Some((
        WorldTile {
            x: 3204,
            z: 3206,
            level: 0,
        },
        0,
        false,
        false,
        false,
    ));
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            route: Some(route.clone()),
            route_worker: Some(Arc::new(())),
            requested_route: requested,
            ..Default::default()
        },
    );

    assert!(!dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::OpenBooth {
            x: 3205,
            z: 3206,
            level: 0,
            id: 9999,
            name: None,
            action: None,
        }],
    ));
    let all = navs.lock().unwrap();
    assert_eq!(all["alice"].route, Some(route));
    assert!(all["alice"].route_worker.is_some());
    assert_eq!(all["alice"].requested_route, requested);
}

#[test]
fn dispatch_open_stand_booth_honors_name_and_operation_slot() {
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (navs, world) = empty_nav();
    let request = |name: &str, stand_op| script::shim::InteractReq::OpenStand {
        x: 3205,
        z: 3206,
        level: 0,
        kind: "booth".into(),
        name: Some(name.into()),
        stand_op,
        choose: None,
    };

    let before = c.out.pos;
    assert!(!dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![request("Wrong chest", Some(2))],
    ));
    assert_eq!(c.out.pos, before, "a wrong name cannot retarget");

    assert!(!dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![request("Bank booth", Some(1))],
    ));
    assert_eq!(c.out.pos, before, "a missing requested op cannot fallback");

    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![request("Bank booth", Some(2))],
    ));
    assert!(
        c.out.pos > before,
        "the exact named operation is dispatched"
    );
}

#[test]
fn withdraw_x_composes_script_host_dialog_and_posted_inventory_result() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (!globalThis.__started) {
            globalThis.__started = true;
            globalThis.__clock = 0;
            globalThis.performance.now = () => globalThis.__clock;
            globalThis.__posted_result = await Bank.withdrawX('Knife', 7);
            return;
        }
        if (globalThis.__posted_result === true && !globalThis.__noted_started) {
            globalThis.__noted_started = true;
            globalThis.__noted_result = await Bank.withdrawXById(2, 5, 3);
        }
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("withdraw-X isolate starts");

    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    assert!(snap.bank_loaded());
    let empty_inv: [(i32, i32); 0] = [];

    let before_x = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&empty_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .expect("isolate finishes the queued tick");
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&empty_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let pending = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_withdraw_x();
    assert!(
        c.out.pos > before_x,
        "the X menu action reaches the driver; pending={pending:?}"
    );
    assert!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .pending_withdraw_x()
            .is_some(),
        "a successful X action arms one host-owned continuation"
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__clock = 9001")
        .expect("advance the isolate clock beyond the former JS timeout");

    snap.rebuild(&c);
    let before_wait = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&empty_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        true,
        false,
    );
    assert_eq!(
        c.out.pos, before_wait,
        "the host sends no amount before the dialog is posted"
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("typeof globalThis.__posted_result")
            .unwrap(),
        "undefined",
        "the held frame cannot consume the pending JavaScript result"
    );

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&empty_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("typeof globalThis.__posted_result")
            .unwrap(),
        "undefined",
        "resume without host outcome keeps the promise pending"
    );
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_withdraw_x()
        .is_some());

    c.apply_p_countdialog();
    c.bump_gens(ServerProt::P_COUNTDIALOG);
    snap.rebuild(&c);
    let before_count = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        3,
        Some((3205, 3205, 0)),
        Some(&empty_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(
        c.out.pos > before_count,
        "the matching posted dialog authorizes one amount response"
    );
    assert!(matches!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .pending_withdraw_x()
            .map(|pending| pending.phase),
        Some(script::slot::PendingWithdrawXPhase::Settlement)
    ));

    let posted_inv = [(2, 7)];
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        4,
        Some((3205, 3205, 0)),
        Some(&posted_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let result = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__posted_result")
        .expect("posted inventory resolves the promise");
    assert_eq!(result, true);
    assert!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .pending_withdraw_x()
            .is_none(),
        "the posted outcome consumes the operation exactly once"
    );

    c.dialog_input_open = false;
    c.bump_gens(ServerProt::P_COUNTDIALOG);
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        5,
        Some((3205, 3205, 0)),
        Some(&posted_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .expect("the next loop queues the noted fixed withdraw");
    let before_fixed = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        5,
        Some((3205, 3205, 0)),
        Some(&posted_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(c.out.pos > before_fixed, "the fixed Withdraw 5 is sent");
    assert!(matches!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .pending_withdraw_x()
            .map(|pending| pending.phase),
        Some(script::slot::PendingWithdrawXPhase::Settlement)
    ));

    let posted_noted = [(2, 7), (3, 5)];
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        6,
        Some((3205, 3205, 0)),
        Some(&posted_noted),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let noted_result = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__noted_result")
        .expect("the posted noted destination resolves the fixed operation");
    assert_eq!(noted_result, true);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn withdraw_load_selects_exact_then_all_then_x() {
    let actions = vec![
        None,
        Some("Withdraw 5".into()),
        Some("Withdraw All".into()),
        Some("Withdraw X".into()),
    ];
    assert_eq!(fill_withdraw_action(&actions, 5, 20), Some((2, false)));
    assert_eq!(fill_withdraw_action(&actions, 20, 20), Some((3, false)));
    assert_eq!(fill_withdraw_action(&actions, 7, 20), Some((4, true)));
    assert_eq!(fill_withdraw_action(&[None], 7, 20), None);
}

#[test]
fn deposit_and_ordinary_withdraw_wait_for_observed_progress() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        await Bank.depositInventory();
        globalThis.__deposit_done = true;
        globalThis.__withdraw_result = await Bank.withdraw('Knife', 'Withdraw All');
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("deposit isolate starts");

    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let posted_inv = [(1, 3)];

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&posted_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let before_send = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&posted_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(c.out.pos > before_send, "Deposit-All reaches the driver");
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_bank_op()
        .is_some());
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("typeof globalThis.__deposit_done")
            .unwrap(),
        "undefined"
    );

    let mut empty_side = Packet::new(vec![2, 189, 0, 0]);
    c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut empty_side);
    snap.rebuild(&c);
    assert!(
        snap.bank_side().is_empty(),
        "settlement removed the side row"
    );
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_bank_op()
        .is_none());
    // The emptied side view is waited on for the frozen 1.2 s (a Rust
    // deadline): let it lapse.
    std::thread::sleep(std::time::Duration::from_millis(1_250));
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        3,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("globalThis.__deposit_done")
            .unwrap(),
        true
    );
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_bank_op()
        .is_none());
    let before_withdraw = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        3,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(
        c.out.pos > before_withdraw,
        "Withdraw-All reaches the driver"
    );
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_bank_op()
        .is_some());

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        4,
        Some((3205, 3205, 0)),
        Some(&[(2, 20)]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("globalThis.__withdraw_result")
            .unwrap(),
        true
    );
}

/// Raw `Bank.withdraw(name, 'Withdraw X')` is the open-only amount
/// action: the frozen caller awaits the accepted click and then types
/// the amount + Enter itself, so the acknowledgment must arrive with
/// the inventory still unchanged (the pre-repair path waited for a
/// delta that cannot exist and posted false after the 4s bound).
#[test]
fn raw_withdraw_x_acknowledges_the_sent_action_with_unchanged_inventory() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__withdraw_result = await Bank.withdraw('Knife', 'Withdraw X');
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("withdraw-X isolate starts");
    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let before_send = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(c.out.pos > before_send, "Withdraw X reaches the driver");
    let before_ack = {
        let slot = script_slot(&scripts, "alice").unwrap();
        let slot = slot.lock().unwrap();
        assert_eq!(
            slot.pending_bank_op().map(|pending| pending.kind),
            Some(script::slot::PendingBankOpKind::WithdrawXAction),
            "the raw X action arms the open-only acknowledgment"
        );
        slot.bank_op_result()
    };

    // The bank is still open and the inventory still holds no Knife:
    // the accepted action alone acknowledges the wait.
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert!(slot.pending_bank_op().is_none(), "the acknowledgment lands");
    assert_eq!(
        slot.bank_op_result(),
        (before_ack.0.wrapping_add(1), true),
        "sent open-only X acknowledges true with unchanged inventory"
    );
    assert_eq!(
        slot.probe("globalThis.__withdraw_result").unwrap(),
        true,
        "the frozen caller can type the amount after the acknowledgment"
    );
}

/// The open-only acknowledgment is bound to the accepted bank session:
/// closing the bank before the acknowledgment pass posts false instead.
#[test]
fn raw_withdraw_x_does_not_acknowledge_a_stale_bank_session() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__withdraw_result = await Bank.withdraw('Knife', 'Withdraw X');
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("withdraw-X isolate starts");
    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let before_ack = {
        let slot = script_slot(&scripts, "alice").unwrap();
        let slot = slot.lock().unwrap();
        assert_eq!(
            slot.pending_bank_op().map(|pending| pending.kind),
            Some(script::slot::PendingBankOpKind::WithdrawXAction)
        );
        slot.bank_op_result()
    };

    // The session ended before the acknowledgment pass.
    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert!(slot.pending_bank_op().is_none());
    assert_eq!(
        slot.bank_op_result(),
        (before_ack.0.wrapping_add(1), false),
        "a closed session never acknowledges the raw X action"
    );
}

fn key_req(down: bool, key: &str) -> script::shim::InteractReq {
    script::shim::InteractReq::Key {
        down,
        key: key.into(),
        code: key.into(),
    }
}

fn type_amount(chars: &str) -> Vec<script::shim::InteractReq> {
    let mut reqs = Vec::new();
    for ch in chars.chars() {
        let key = ch.to_string();
        reqs.push(key_req(true, &key));
        reqs.push(key_req(false, &key));
    }
    reqs
}

fn dispatch_keys(
    client: &mut Client,
    snapshot: &GameSnapshot,
    reqs: Vec<script::shim::InteractReq>,
) -> bool {
    let (navs, world) = empty_nav();
    dispatch_script_interact(
        client,
        snapshot,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        reqs,
    )
}

fn count_packet(client: &Client) -> Option<i32> {
    let opcode = client::io::ClientProt::RESUME_P_COUNTDIALOG.id as u8;
    let data = &client.out.data()[..client.out.pos];
    if data.len() < 5 || data[0] != opcode {
        return None;
    }
    Some(i32::from_be_bytes([data[1], data[2], data[3], data[4]]))
}

/// Real JS document/KeyboardEvent producer through FB dispatch into a
/// real Client amount prompt. Digits are visible before Enter; keyup
/// does not duplicate; inventory is not fabricated by the key path.
#[test]
fn canvas_keyboard_types_visible_200_then_enter_on_real_client() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        const n = (globalThis.__step = (globalThis.__step || 0) + 1);
        if (n === 1) {
            for (const ch of ['2', '0', '0']) {
                canvas.dispatchEvent(new KeyboardEvent('keydown', {key: ch, code: ch}));
                canvas.dispatchEvent(new KeyboardEvent('keyup', {key: ch, code: ch}));
            }
        } else if (n === 2) {
            canvas.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', code: 'Enter'}));
            canvas.dispatchEvent(new KeyboardEvent('keyup', {key: 'Enter', code: 'Enter'}));
        }
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("canvas keyboard isolate starts");
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    c.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let inv_before = c.out.pos;

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(c.dialog_input, "200", "digits are visible in the prompt");
    assert!(c.dialog_input_open);
    assert_eq!(
        c.out.pos, inv_before,
        "keys do not fabricate a count packet"
    );
    assert!(c.chat_input.is_empty());

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(count_packet(&c), Some(200));
    assert!(!c.dialog_input_open);
    assert!(c.chat_input.is_empty());
    assert_eq!(
        c.shell.key_held[b'2' as usize], 0,
        "keyup must not leave synthetic held state"
    );
}

#[test]
fn canvas_keyboard_preserves_pending_user_enter_without_chat_leak() {
    let mut c = bank_client();
    c.apply_p_countdialog();
    c.dialog_input = "7".into();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    c.shell.apply_key(true, 10, 10);
    let before = (c.shell.key_queue_read, c.shell.key_queue_write);
    dispatch_keys(&mut c, &snap, type_amount("2"));
    assert!(
        c.chat_input.is_empty(),
        "script digit leaked into chat: {:?}",
        c.chat_input
    );
    assert_eq!(
        (c.shell.key_queue_read, c.shell.key_queue_write),
        before,
        "script input must not drain pending user input"
    );
    assert_eq!(c.dialog_input, "7");
    assert!(c.dialog_input_open);
    assert_eq!(c.shell.poll_key(), 10);
}

#[test]
fn canvas_keyboard_preserves_user_held_digit_on_script_keyup() {
    let mut c = bank_client();
    c.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    c.shell.apply_key(true, 0, b'2' as i32);
    assert_eq!(c.shell.poll_key(), b'2' as i32);
    assert_eq!(c.shell.key_held[b'2' as usize], 1);
    dispatch_keys(
        &mut c,
        &snap,
        vec![key_req(true, "5"), key_req(false, "5"), key_req(false, "2")],
    );
    assert_eq!(c.dialog_input, "5");
    assert_eq!(
        c.shell.key_held[b'2' as usize], 1,
        "script keyup must not clear a user-held digit"
    );
    assert_eq!(c.shell.key_queue_write, c.shell.key_queue_read);
    assert!(c.chat_input.is_empty());
}

#[test]
fn canvas_keyboard_enter_then_digit_does_not_leak_into_chat() {
    let mut c = bank_client();
    c.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let mut reqs = type_amount("200");
    reqs.push(key_req(true, "Enter"));
    reqs.push(key_req(false, "Enter"));
    reqs.push(key_req(true, "3"));
    reqs.push(key_req(false, "3"));
    dispatch_keys(&mut c, &snap, reqs);
    assert_eq!(count_packet(&c), Some(200));
    assert!(!c.dialog_input_open);
    assert!(
        !c.chat_input.contains('3'),
        "digit after Enter in the same batch must not enter chat, got {:?}",
        c.chat_input
    );
    assert!(c.social_input.is_empty());
}

#[test]
fn canvas_keyboard_drops_closed_social_and_replaced_prompts() {
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    dispatch_keys(&mut c, &snap, type_amount("2"));
    assert!(c.dialog_input.is_empty());
    assert!(
        !c.chat_input.contains('2'),
        "closed prompt must not leak into chat: {:?}",
        c.chat_input
    );

    c.social_input_open = true;
    c.dialog_input_open = true;
    c.dialog_input.clear();
    c.social_input.clear();
    dispatch_keys(&mut c, &snap, type_amount("7"));
    assert!(
        c.social_input.is_empty(),
        "social recipient must not take amount keys: {:?}",
        c.social_input
    );
    assert_ne!(c.dialog_input, "7");

    c.social_input_open = false;
    c.apply_p_countdialog();
    dispatch_keys(&mut c, &snap, type_amount("2"));
    assert_eq!(c.dialog_input, "2");
    c.shell.apply_key(true, 0, b'9' as i32);
    c.apply_p_countdialog();
    let queued = (c.shell.key_queue_read, c.shell.key_queue_write);
    let mut reqs = type_amount("00");
    reqs.push(key_req(true, "Enter"));
    reqs.push(key_req(false, "Enter"));
    dispatch_keys(&mut c, &snap, reqs);
    assert!(
        c.dialog_input.is_empty(),
        "script must not type over unread user keys, got {:?}",
        c.dialog_input
    );
    assert_eq!(
        (c.shell.key_queue_read, c.shell.key_queue_write),
        queued,
        "replaced prompt must leave unread user work in the ring"
    );
    assert_eq!(c.shell.poll_key(), b'9' as i32);
    assert!(c.chat_input.is_empty());
    assert_eq!(count_packet(&c), None);
    assert!(c.dialog_input_open);
}

#[test]
fn canvas_keyboard_respects_ring_capacity_and_allowlist() {
    let mut c = bank_client();
    c.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    for _ in 0..127 {
        c.shell.apply_key(true, 0, b'a' as i32);
    }
    assert!(!Driver::can_enqueue_key(&c));
    dispatch_keys(&mut c, &snap, type_amount("9"));
    assert!(
        c.dialog_input.is_empty(),
        "a full ring must not overwrite unread user keys with a script digit"
    );
    assert_eq!(c.shell.poll_key(), b'a' as i32);

    let mut c = bank_client();
    c.apply_p_countdialog();
    snap.rebuild(&c);
    let mut overflow = Vec::new();
    for _ in 0..80 {
        overflow.push(key_req(true, "1"));
        overflow.push(key_req(false, "1"));
    }
    dispatch_keys(&mut c, &snap, overflow);
    assert_eq!(c.dialog_input, "1111111111");
    assert_eq!(c.shell.key_queue_write, c.shell.key_queue_read);

    let mut c = bank_client();
    c.apply_p_countdialog();
    snap.rebuild(&c);
    dispatch_keys(
        &mut c,
        &snap,
        vec![
            key_req(true, "a"),
            key_req(false, "a"),
            key_req(true, ":"),
            key_req(false, ":"),
            key_req(true, "F1"),
            key_req(false, "F1"),
        ],
    );
    assert!(c.dialog_input.is_empty());
    assert!(c.chat_input.is_empty());
    assert_eq!(c.shell.key_queue_write, c.shell.key_queue_read);
}

#[test]
fn canvas_keyboard_keyup_does_not_duplicate_and_second_slot_is_untouched() {
    let mut alice = bank_client();
    let mut bob = bank_client();
    alice.apply_p_countdialog();
    bob.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&alice);
    dispatch_keys(
        &mut alice,
        &snap,
        vec![key_req(true, "2"), key_req(false, "2"), key_req(false, "2")],
    );
    assert_eq!(alice.dialog_input, "2");
    assert_eq!(bob.dialog_input, "");
    assert_eq!(bob.shell.key_queue_write, bob.shell.key_queue_read);
    assert!(bob.dialog_input_open);
}

#[test]
fn canvas_keyboard_pause_and_hold_use_existing_interact_freeze() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        const n = (globalThis.__step = (globalThis.__step || 0) + 1);
        if (n === 1) {
            canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
            canvas.dispatchEvent(new KeyboardEvent('keyup', {key: '2', code: '2'}));
        } else if (n === 2) {
            canvas.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', code: 'Enter'}));
            canvas.dispatchEvent(new KeyboardEvent('keyup', {key: 'Enter', code: 'Enter'}));
        }
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("pause isolate starts");
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    c.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(c.dialog_input, "2");
    assert_eq!(c.out.pos, 0);

    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pause();
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(c.dialog_input, "2");
    assert_eq!(c.out.pos, 0, "pause must not dispatch Enter");

    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .resume();
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(count_packet(&c), Some(2));

    let mut held = bank_client();
    held.apply_p_countdialog();
    snap.rebuild(&held);
    let scripts_hold: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    script_slot_or_insert(&scripts_hold, "bob")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("hold isolate starts");
    script_observe(
        &mut held,
        "bob",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts_hold,
        &cheats,
        &navs,
        &world,
        true,
        false,
    );
    script_slot(&scripts_hold, "bob")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut held,
        "bob",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts_hold,
        &cheats,
        &navs,
        &world,
        true,
        false,
    );
    assert!(
        held.dialog_input.is_empty(),
        "guardian hold must not apply canvas keys, got {:?}",
        held.dialog_input
    );
    assert_eq!(held.out.pos, 0);
    assert_eq!(held.shell.key_held[b'2' as usize], 0);
}

#[test]
fn canvas_keyboard_producer_leaves_pending_user_enter_untouched() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        canvas.dispatchEvent(new KeyboardEvent('keydown', {key: '2', code: '2'}));
        canvas.dispatchEvent(new KeyboardEvent('keyup', {key: '2', code: '2'}));
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("pending-enter isolate starts");
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    c.apply_p_countdialog();
    c.dialog_input = "7".into();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    c.shell.apply_key(true, 10, 10);
    let before = (c.shell.key_queue_read, c.shell.key_queue_write);

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(
        c.chat_input.is_empty(),
        "producer digit leaked into chat: {:?}",
        c.chat_input
    );
    assert_eq!(
        (c.shell.key_queue_read, c.shell.key_queue_write),
        before,
        "producer must not drain pending user Enter"
    );
    assert_eq!(c.dialog_input, "7");
    assert_eq!(c.shell.poll_key(), 10);
}

#[test]
fn canvas_keyboard_script_digits_do_not_advance_revision289_cyclelogic4() {
    let mut c = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        client::client::ClientRevision::R289,
    );
    c.ingame = true;
    c.apply_p_countdialog();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let mut ones = Vec::new();
    for _ in 0..200 {
        ones.push(key_req(true, "1"));
        ones.push(key_req(false, "1"));
    }
    dispatch_keys(&mut c, &snap, ones);
    assert_eq!(c.dialog_input, "1111111111");
    assert_eq!(
        c.out.pos, 0,
        "script digits must not emit ANTICHEAT_CYCLELOGIC4"
    );
    for _ in 0..192 {
        c.handle_chat_input();
    }
    assert_eq!(c.out.pos, 0, "192 empty polls must not emit yet");
    c.handle_chat_input();
    assert_eq!(
        &c.out.data()[..c.out.pos],
        &[137, 232],
        "the authentic 193rd frame poll still emits cyclelogic4"
    );
}

#[test]
fn canvas_mouse_producer_to_native_frame_center_click() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let inp = SlotInput::new();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        const r = canvas.getBoundingClientRect();
        canvas.dispatchEvent(new MouseEvent('mousedown', {
            clientX: r.left + r.width / 2,
            clientY: r.top + r.height / 2,
        }));
    }
}
"#;
    {
        let slot = script_slot_or_insert(&scripts, "alice");
        let mut slot = slot.lock().unwrap();
        slot.bind_native_input(inp.authority());
        slot.start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
            .expect("canvas mouse isolate starts");
    }
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let before_modal = c.main_modal_id;
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(
        (
            c.shell.mouse_click_button,
            c.shell.mouse_click_x,
            c.shell.mouse_click_y,
            c.shell.mouse_button
        ),
        (1, 382, 251, 1)
    );
    c.mainloop();
    assert_eq!(c.main_modal_id, before_modal, "center click must not close");
}

#[test]
fn canvas_mouse_289_down_does_not_move_pointer() {
    let inp = SlotInput::new();
    inp.authority().publish_live();
    let mut c = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        client::client::ClientRevision::R289,
    );
    let before_x = c.shell.mouse_x;
    let before_y = c.shell.mouse_y;
    inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_click_button, 1);
    assert_eq!(c.shell.mouse_x, before_x);
    assert_eq!(c.shell.mouse_y, before_y);
    assert_eq!(c.shell.mouse_button, 1);
}

#[test]
fn canvas_mouse_pause_before_consume_does_not_latch() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let inp = SlotInput::new();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    }
}
"#;
    {
        let slot = script_slot_or_insert(&scripts, "alice");
        let mut slot = slot.lock().unwrap();
        slot.bind_native_input(inp.authority());
        slot.start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
            .expect("pause mouse isolate starts");
    }
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pause();
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_click_button, 0);
    assert_eq!(c.shell.mouse_button, 0);
}

#[test]
fn canvas_mouse_slot_pause_resume_before_frame_releases_held() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let inp = SlotInput::new();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    }
}
"#;
    {
        let slot = script_slot_or_insert(&scripts, "alice");
        let mut slot = slot.lock().unwrap();
        slot.bind_native_input(inp.authority());
        slot.start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
            .expect("pause/resume mouse isolate starts");
    }
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_button, 1);
    {
        let slot = script_slot(&scripts, "alice").unwrap();
        let mut slot = slot.lock().unwrap();
        slot.pause();
        slot.resume();
    }
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_button, 0);
    assert_eq!(c.shell.mouse_click_button, 0);
}

#[test]
fn canvas_mouse_slot_stop_start_before_frame_releases_held() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let inp = SlotInput::new();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
    }
}
"#;
    {
        let slot = script_slot_or_insert(&scripts, "alice");
        let mut slot = slot.lock().unwrap();
        slot.bind_native_input(inp.authority());
        slot.start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
            .expect("stop/start mouse isolate starts");
    }
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        Some(&inp),
    );
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_button, 1);
    {
        let slot = script_slot(&scripts, "alice").unwrap();
        let mut slot = slot.lock().unwrap();
        slot.stop();
        slot.bind_native_input(inp.authority());
        slot.start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
            .expect("replacement start");
    }
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_button, 0);
    assert_eq!(c.shell.mouse_click_button, 0);
}

#[test]
fn canvas_mouse_delayed_old_up_does_not_release_new_hold() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let inp = SlotInput::new();
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        const canvas = document.getElementById('canvas');
        canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
        canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 100, clientY: 100}));
    }
}
"#;
    {
        let slot = script_slot_or_insert(&scripts, "alice");
        let mut slot = slot.lock().unwrap();
        slot.bind_native_input(inp.authority());
        slot.start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
            .expect("delayed up isolate starts");
    }
    let mut c = bank_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    script_observe_with_npc_boxes(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        None,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let reqs = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .drain_interacts();
    let bytes = script::isolate_fb::encode_interact_batch(&reqs);
    let decoded = script::isolate_fb::decode_interact_batch(&bytes).expect("fb mouse batch");
    let old_down = decoded
        .iter()
        .find(|req| matches!(req, script::shim::InteractReq::Mouse { down: true, .. }))
        .cloned()
        .expect("produced down");
    let old_up = decoded
        .iter()
        .find(|req| matches!(req, script::shim::InteractReq::Mouse { down: false, .. }))
        .cloned()
        .expect("produced up");
    let script::shim::InteractReq::Mouse {
        down,
        x,
        y,
        button,
        identity,
    } = old_down
    else {
        panic!("down");
    };
    inp.enqueue_script_mouse_at(identity, down, x, y, button);
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_button, 1);
    {
        let slot = script_slot(&scripts, "alice").unwrap();
        let mut slot = slot.lock().unwrap();
        slot.pause();
        slot.resume();
    }
    inp.enqueue_script_mouse(true, 50.0, 50.0, 0);
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_button, 1);
    assert_eq!(c.shell.mouse_click_x, 50);
    let script::shim::InteractReq::Mouse {
        down,
        x,
        y,
        button,
        identity,
    } = old_up
    else {
        panic!("up");
    };
    inp.enqueue_script_mouse_at(identity, down, x, y, button);
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(
        c.shell.mouse_button, 1,
        "stale up must not release the newer hold"
    );
    assert_eq!(c.shell.mouse_click_x, 50);
}

#[test]
fn canvas_mouse_parked_old_up_preserves_fresh_producer_hold() {
    let inp = SlotInput::new();
    let authority = inp.authority();
    let old_identity = authority.publish_live();
    let iso = script::LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        const canvas = document.getElementById('canvas');
        canvas.dispatchEvent(new MouseEvent('mousedown', {clientX: 100, clientY: 100}));
        await new Promise((resolve) => {
            globalThis.__release = resolve;
        });
        canvas.dispatchEvent(new MouseEvent('mouseup', {clientX: 100, clientY: 100}));
    }
}
"#
        .into(),
        script::LoadShape::CompatClass,
        vec![],
    )
    .expect("parked mouse isolate starts");
    let mut shell = client::client::GameShell::new();

    iso.on_game_tick_at(1, old_identity);
    iso.probe("true").unwrap();
    let old_down = iso.drain_interacts();
    assert!(
        old_down.iter().any(|req| matches!(
            req,
            script::shim::InteractReq::Mouse {
                down: true,
                identity,
                ..
            } if *identity == old_identity
        )),
        "{old_down:?}"
    );
    assert!(take_script_interacts(old_down, Some(&inp)).is_empty());
    inp.consume_native_frame(&mut shell);
    assert_eq!(shell.mouse_button, 1);
    assert_eq!(shell.mouse_click_x, 100);

    iso.pause();
    authority.revoke();
    authority.resume();
    iso.resume();
    let fresh_identity = authority.lock().identity();
    assert_ne!(fresh_identity, old_identity);

    iso.probe(
            "(() => { document.getElementById('canvas').dispatchEvent(new MouseEvent('mousedown', {clientX: 50, clientY: 50})); return true; })()",
        )
        .unwrap();
    iso.on_game_tick_at(2, fresh_identity);
    iso.probe("true").unwrap();
    let fresh_down = iso.drain_interacts();
    assert!(
        fresh_down.iter().any(|req| matches!(
            req,
            script::shim::InteractReq::Mouse {
                down: true,
                identity,
                ..
            } if *identity == fresh_identity
        )),
        "{fresh_down:?}"
    );
    assert!(take_script_interacts(fresh_down, Some(&inp)).is_empty());
    inp.consume_native_frame(&mut shell);
    assert_eq!(shell.mouse_button, 1);
    assert_eq!(shell.mouse_click_x, 50);

    iso.probe("globalThis.__release(); true").unwrap();
    iso.on_game_tick_at(3, fresh_identity);
    iso.probe("true").unwrap();
    let old_up = iso.drain_interacts();
    assert!(
        old_up.iter().any(|req| matches!(
            req,
            script::shim::InteractReq::Mouse {
                down: false,
                identity,
                ..
            } if *identity == old_identity
        )),
        "{old_up:?}"
    );
    assert!(take_script_interacts(old_up, Some(&inp)).is_empty());
    inp.consume_native_frame(&mut shell);
    assert_eq!(
        shell.mouse_button, 1,
        "the parked old up must not release the fresh hold"
    );
    assert_eq!(shell.mouse_click_x, 50);

    iso.probe(
            "(() => { document.getElementById('canvas').dispatchEvent(new MouseEvent('mouseup', {clientX: 50, clientY: 50})); return true; })()",
        )
        .unwrap();
    iso.on_game_tick_at(4, fresh_identity);
    iso.probe("true").unwrap();
    let fresh_up = iso.drain_interacts();
    assert!(
        fresh_up.iter().any(|req| matches!(
            req,
            script::shim::InteractReq::Mouse {
                down: false,
                identity,
                ..
            } if *identity == fresh_identity
        )),
        "{fresh_up:?}"
    );
    assert!(take_script_interacts(fresh_up, Some(&inp)).is_empty());
    inp.consume_native_frame(&mut shell);
    assert_eq!(shell.mouse_button, 0);
    iso.join();
}

#[test]
fn canvas_mouse_logout_before_frame_releases_script_hold() {
    let inp = SlotInput::new();
    inp.authority().publish_live();
    inp.enqueue_script_mouse(true, 100.0, 100.0, 0);
    let mut shell = client::client::GameShell::new();
    inp.consume_native_frame(&mut shell);
    assert_eq!(shell.mouse_button, 1);
    inp.set_host_consume_allowed(false);
    inp.consume_native_frame(&mut shell);
    assert_eq!(shell.mouse_button, 0);
}

#[test]
fn canvas_mouse_289_sample_after_down_does_not_publish_click_coords() {
    let inp = SlotInput::new();
    inp.authority().publish_live();
    let mut c = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        client::client::ClientRevision::R289,
    );
    let before_x = c.shell.mouse_x;
    let before_y = c.shell.mouse_y;
    inp.enqueue_script_mouse(true, 382.5, 251.5, 0);
    inp.consume_native_frame(&mut c.shell);
    assert_eq!(c.shell.mouse_click_button, 1);
    c.shell.sample_mouse();
    assert_eq!(c.shell.mouse_x, before_x);
    assert_eq!(c.shell.mouse_y, before_y);
    assert_eq!(c.shell.mouse_button, 1);
}

#[test]
fn ordinary_withdraw_refusal_posts_false_to_the_isolate() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__withdraw_result = await Bank.withdraw('Knife', 'Withdraw 42');
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .unwrap();
    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("globalThis.__withdraw_result")
            .unwrap(),
        false
    );
}

#[test]
fn withdraw_load_composes_all_send_and_observed_fill_result() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__fill_result = await Bank.withdrawLoad('Knife');
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("withdraw-load isolate starts");

    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let before_inv = [(1, 3)];

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&before_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let before_send = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&before_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(c.out.pos > before_send, "Withdraw-All reaches the driver");

    let mut empty_bank = Packet::new(vec![2, 89, 0, 0]);
    c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut empty_bank);
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&before_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("typeof globalThis.__fill_result")
            .unwrap(),
        "undefined",
        "a vanished stock row alone cannot claim fill progress"
    );
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_withdraw_x()
        .is_some());

    let filled_inv = [(1, 3), (2, 20)];
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        3,
        Some((3205, 3205, 0)),
        Some(&filled_inv),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("globalThis.__fill_result")
            .unwrap(),
        true
    );
}

#[test]
fn withdraw_x_expired_dialog_posts_failure_without_answer() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let slot = script_slot_or_insert(&scripts, "alice");
    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    {
        let mut slot = slot.lock().unwrap();
        slot.start_compiled(Box::new(TickCounter(Arc::new(Mutex::new(0)))), None)
            .unwrap();
        let mut pending = script::slot::PendingWithdrawX::waiting_dialog(
            2,
            7,
            0,
            7,
            snap.bank_session_generation(),
        );
        pending.deadline = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(1))
                .unwrap(),
        );
        slot.set_pending_withdraw_x(Some(pending));
    }

    let before = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let mut slot = slot.lock().unwrap();
    assert_eq!(c.out.pos, before, "an expired dialog sends no count");
    assert!(slot.pending_withdraw_x().is_none());
    assert_eq!(slot.withdraw_x_result(), (1, false));
    slot.set_pending_withdraw_x(Some(script::slot::PendingWithdrawX::waiting_dialog(
        2,
        7,
        0,
        7,
        snap.bank_session_generation().wrapping_add(1),
    )));
    drop(slot);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert!(slot.pending_withdraw_x().is_none());
    assert_eq!(slot.withdraw_x_result(), (2, false));
    assert_eq!(c.out.pos, before, "a changed bank session sends no count");
}

#[test]
fn withdraw_x_same_generation_rejection_posts_false_to_the_isolate() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        Bank.withdrawX('Knife', 7).then((ok) => { globalThis.__rejected = ok; });
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(source.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("withdraw-X isolate starts");

    let mut c = bank_fetch_client();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let generation = snap.bank_session_generation();
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .expect("isolate queues the request");

    let mut empty_bank = Packet::new(vec![2, 89, 0, 0]);
    c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut empty_bank);
    snap.rebuild(&c);
    assert_eq!(
        snap.bank_session_generation(),
        generation,
        "the host rejection is not explained by a changed bank session"
    );
    let before = c.out.pos;
    script_observe(
        &mut c,
        "alice",
        true,
        false,
        1,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(c.out.pos, before, "the vanished op sends nothing");

    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3205, 3205, 0)),
        Some(&[]),
        None,
        Some(&snap),
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let result = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__rejected")
        .expect("the explicit failure result settles the same-generation request");
    assert_eq!(result, false);
}

/// The shim `Item.interact` arm: a `Held { name, action }` request
/// resolves the held item through ObjNames and dispatches its menu op
/// by label (Bones → Bury). The `bank_fetch_client` inv tab holds
/// Bones (obj 1) × 3.
#[test]
fn dispatch_script_interact_sends_held_item_bury() {
    let mut c = bank_fetch_client();
    // Bones (obj 1) has the `Bury` held op (`[opheld1,_bones]`).
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs[1].iop = [Some("Bury".into()), None, None, None, None];
    }
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let names = api::obj_names::ObjNames::from_objs(&{
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.clone()
    });
    let (navs, world) = empty_nav();
    assert_eq!(
        snap.inventory()
            .iter()
            .find(|it| it.def.id == 1)
            .map(|it| (it.def.id, it.count)),
        Some((1, 3)),
        "the inv tab holds Bones"
    );
    let before = c.out.pos;
    assert!(
        dispatch_script_interact(
            &mut c,
            &snap,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![script::shim::InteractReq::Held {
                name: "Bones".into(),
                action: "Bury".into()
            }],
        ),
        "held Bury must dispatch"
    );
    assert!(c.out.pos > before, "held Bury must write the driver");
    // An action the item has no op for sends nothing (Bones has no
    // "Wear"), and an unknown name matches no held item.
    let before = c.out.pos;
    assert!(!dispatch_script_interact(
        &mut c,
        &snap,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![
            script::shim::InteractReq::Held {
                name: "Bones".into(),
                action: "Wear".into()
            },
            script::shim::InteractReq::Held {
                name: "Lobster".into(),
                action: "Bury".into()
            },
        ],
    ));
    assert_eq!(
        c.out.pos, before,
        "a label no held op resolves and an unknown name send nothing"
    );
}

/// The shim `Equipment.unequip` arm: an `Unequip { name }` request
/// resolves the worn row by case-insensitive ObjNames name and sends the
/// worn component's `Remove` op (INV_BUTTON1 at the worn row's
/// id/slot/component). A held-but-not-worn name and an unknown name
/// send nothing.
#[test]
fn dispatch_script_interact_unequip_removes_the_worn_row() {
    use client::client::MiniMenuAction;

    let mut c = bank_fetch_client();
    // Worn-items tab 4: the Knife (obj 2, stored 3) worn in slot 1 of a
    // TYPE_INV component whose own menu is `Remove`.
    c.side_icon[4] = 710;
    c.set_iface(
        710,
        IfType {
            id: 710,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Remove".into()), None, None, None, None],
            ..Default::default()
        },
    );
    c.set_iface_mut(
        710,
        IfTypeMut {
            link_obj_type: Some(vec![0, 3]),
            link_obj_number: Some(vec![0, 1]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let names = api::obj_names::ObjNames::from_objs(&{
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.clone()
    });
    let (navs, world) = empty_nav();
    assert_eq!(
        snap.equipment()
            .iter()
            .map(|it| (it.def.id, it.slot, it.component_id))
            .collect::<Vec<_>>(),
        vec![(2, 1, 710)],
        "the worn tab holds the Knife"
    );

    let mut rec = GuardRec::default();
    assert!(!dispatch_script_interact(
        &mut rec,
        &snap,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![
            script::shim::InteractReq::Unequip {
                name: "Bones".into()
            },
            script::shim::InteractReq::Unequip {
                name: "Lobster".into()
            },
        ],
    ));
    assert!(
        rec.menus.is_empty() && rec.actions.is_empty(),
        "a held-but-not-worn name and an unknown name send nothing"
    );

    assert!(
        dispatch_script_interact(
            &mut rec,
            &snap,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![script::shim::InteractReq::Unequip {
                name: "knife".into()
            }],
        ),
        "unequip of a worn item must dispatch"
    );
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::INV_BUTTON1, 2, 1, 710)],
        "Remove on the worn component row"
    );
}

#[test]
fn dispatch_use_on_honors_exact_inventory_identity_and_rejects_stale_slots() {
    use client::config::ObjType;

    let mut c = bank_fetch_client();
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.resize(1779, ObjType::default());
        for (id, name) in [
            (60, "Willow shortbow"),
            (849, "Willow shortbow"),
            (1777, "Bow string"),
            (1778, "Bow string"),
        ] {
            cache.objs[id].id = id as i32;
            cache.objs[id].name = name.into();
        }
    }
    c.set_iface_mut(
        500,
        IfTypeMut {
            // Stored obj ids are real ids + 1. The selected rows sit
            // after earlier same-name decoys and two empty slots.
            link_obj_type: Some(vec![850, 1778, 0, 0, 61, 0, 1779]),
            link_obj_number: Some(vec![1, 1, 0, 0, 1, 0, 1]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::UPDATE_INV_FULL);

    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    assert_eq!(
        snap.inventory()
            .iter()
            .map(|item| (item.def.id, item.slot))
            .collect::<Vec<_>>(),
        vec![(849, 0), (1777, 1), (60, 4), (1778, 6)]
    );
    let (navs, world) = empty_nav();
    let request = |source_item_id, source_item_slot, target_item_id, target_item_slot| {
        script::shim::InteractReq::UseOn {
            name: "Willow shortbow".into(),
            kind: "inv".into(),
            target_name: Some("Bow string".into()),
            x: 0,
            z: 0,
            level: 0,
            index: None,
            source_item_id,
            source_item_slot,
            target_item_id,
            target_item_slot,
        }
    };

    let mut rec = GuardRec::default();
    assert!(dispatch_script_interact(
        &mut rec,
        &snap,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![request(Some(60), Some(4), Some(1778), Some(6))],
    ));
    assert_eq!(
        rec.menus,
        vec![
            (0, MiniMenuAction::USEHELD_START, 60, 4, 500),
            (0, MiniMenuAction::USEHELD_ONHELD, 1778, 6, 500),
        ],
        "selected ids and slots beat earlier same-name rows"
    );

    for stale in [
        request(Some(60), Some(0), Some(1778), Some(6)),
        request(Some(60), Some(4), Some(1778), Some(1)),
        request(Some(60), None, Some(1778), Some(6)),
    ] {
        let mut rec = GuardRec::default();
        assert!(!dispatch_script_interact(
            &mut rec,
            &snap,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![stale],
        ));
        assert!(
            rec.menus.is_empty(),
            "stale or partial identity sends nothing"
        );
    }

    let mut legacy = GuardRec::default();
    assert!(dispatch_script_interact(
        &mut legacy,
        &snap,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![request(None, None, None, None)],
    ));
    assert_eq!(
        legacy.menus,
        vec![
            (0, MiniMenuAction::USEHELD_START, 849, 0, 500),
            (0, MiniMenuAction::USEHELD_ONHELD, 1777, 1, 500),
        ],
        "legacy name-only requests retain first-name behavior"
    );
}

fn loc_typecode(id: i32) -> i32 {
    0x4000_0000 + (id << 14)
}

fn scene_typecode(id: i32) -> i32 {
    loc_typecode(id) + 7 + (8 << 7)
}

fn seed_wall_and_flax_defs(c: &mut Client) {
    use client::config::LocType;
    let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
    while cache.locs.len() <= 2646 {
        cache.locs.push(LocType::default());
    }
    cache.locs[980] = LocType {
        id: 980,
        name: String::new(),
        op: vec![],
        width: 1,
        length: 1,
        ..Default::default()
    };
    cache.locs[2646] = LocType {
        id: 2646,
        name: "Flax".into(),
        op: vec![None, Some("Pick".into())],
        width: 1,
        length: 1,
        ..Default::default()
    };
}

/// Wall 980 and Ground Flax 2646 share scene tile (5,6). `wall_first`
/// places 980 on the wall layer (native LIVE order); otherwise Flax is
/// the wall-layer row so it is first in the snapshot sweep.
fn colocated_wall_flax(wall_first: bool) -> (Client, GameSnapshot) {
    let mut c = nav_client();
    c.map_build_base_x = 0;
    c.map_build_base_z = 0;
    c.minusedlevel = 0;
    c.local_player = Some(client::dash3d::ClientPlayer::at(4, 5));
    seed_wall_and_flax_defs(&mut c);
    let (wall_layer_id, ground_id) = if wall_first { (980, 2646) } else { (2646, 980) };
    c.world.set_wall(
        0,
        5,
        6,
        0,
        0,
        0,
        loc_typecode(wall_layer_id),
        1 << 6,
        0,
        0,
        0,
        0,
    );
    assert!(c.world.add_scenery(
        0,
        5,
        6,
        0,
        scene_typecode(ground_id),
        (3 << 6) + 10,
        1,
        1,
        0,
        0,
        0,
        0,
        0
    ));
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    (c, snap)
}

fn loc_req(id: Option<i32>, action: &str) -> script::shim::InteractReq {
    script::shim::InteractReq::Loc {
        x: 5,
        z: 6,
        level: 0,
        action: action.into(),
        id,
    }
}

fn dispatch_loc(snap: &GameSnapshot, req: script::shim::InteractReq) -> GuardRec {
    let (navs, world) = empty_nav();
    let mut rec = GuardRec::default();
    dispatch_script_interact(
        &mut rec,
        snap,
        None,
        Some((4, 5, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![req],
    );
    rec
}

fn flax_typecode(snap: &GameSnapshot) -> i32 {
    snap.locs()
        .iter()
        .find(|loc| loc.id == 2646)
        .expect("flax row")
        .typecode
}

#[test]
fn dispatch_loc_preserves_selected_id_when_layers_share_a_tile() {
    for wall_first in [true, false] {
        let (_c, snap) = colocated_wall_flax(wall_first);
        let ids: Vec<i32> = snap.locs().iter().map(|loc| loc.id).collect();
        if wall_first {
            assert_eq!(ids, vec![980, 2646], "native sweep is wall then ground");
        } else {
            assert_eq!(ids, vec![2646, 980], "flax on wall layer is first");
        }
        let rec = dispatch_loc(&snap, loc_req(Some(2646), "Pick"));
        assert_eq!(
            rec.menus,
            vec![(0, MiniMenuAction::OP_LOC2, flax_typecode(&snap), 5, 6)],
            "selected Flax 2646 must win in either row order (wall_first={wall_first})"
        );
    }
}

#[test]
fn dispatch_loc_refuses_stale_missing_forged_and_invalid_action() {
    let (mut c, snap) = colocated_wall_flax(true);
    let before = dispatch_loc(&snap, loc_req(Some(2646), "Open"));
    assert!(before.menus.is_empty(), "invalid flax action sends nothing");

    let forged = dispatch_loc(&snap, loc_req(Some(9999), "Pick"));
    assert!(forged.menus.is_empty(), "forged id sends nothing");

    let wall_pick = dispatch_loc(&snap, loc_req(Some(980), "Pick"));
    assert!(wall_pick.menus.is_empty(), "wall 980 has no Pick");

    c.world.del_loc(0, 5, 6);
    c.bump_gens(ServerProt::LOC_DEL);
    let mut gone = GameSnapshot::new();
    gone.rebuild(&c);
    assert!(
        gone.locs().iter().all(|loc| loc.id != 2646),
        "deleted flax must leave the snapshot"
    );
    let missing = dispatch_loc(&gone, loc_req(Some(2646), "Pick"));
    assert!(
        missing.menus.is_empty(),
        "missing selected flax must not fall back to wall 980"
    );
}

#[test]
fn dispatch_loc_legacy_without_id_keeps_first_row_behavior() {
    let (_c, shared) = colocated_wall_flax(true);
    let legacy_shared = dispatch_loc(&shared, loc_req(None, "Pick"));
    assert!(
        legacy_shared.menus.is_empty(),
        "legacy first-row wall 980 still refuses Pick"
    );

    let mut c = nav_client();
    c.map_build_base_x = 0;
    c.map_build_base_z = 0;
    c.minusedlevel = 0;
    c.local_player = Some(client::dash3d::ClientPlayer::at(4, 5));
    seed_wall_and_flax_defs(&mut c);
    assert!(c.world.add_scenery(
        0,
        5,
        6,
        0,
        scene_typecode(2646),
        (3 << 6) + 10,
        1,
        1,
        0,
        0,
        0,
        0,
        0
    ));
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut only = GameSnapshot::new();
    only.rebuild(&c);
    assert_eq!(
        only.locs().iter().map(|loc| loc.id).collect::<Vec<_>>(),
        vec![2646]
    );
    let rec = dispatch_loc(&only, loc_req(None, "Pick"));
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_LOC2, flax_typecode(&only), 5, 6)],
        "legacy coordinate match still dispatches a lone flax"
    );
}

#[test]
fn dispatch_loc_selected_id_survives_flatbuffer_round_trip() {
    let (_c, snap) = colocated_wall_flax(true);
    let reqs = vec![loc_req(Some(2646), "Pick")];
    let bytes = script::isolate_fb::encode_interact_batch(&reqs);
    let decoded = script::isolate_fb::decode_interact_batch(&bytes).expect("loc batch");
    assert_eq!(decoded, reqs);
    let rec = dispatch_loc(&snap, decoded.into_iter().next().unwrap());
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_LOC2, flax_typecode(&snap), 5, 6)]
    );
}

fn same_name_bank_client() -> Client {
    use client::config::ObjType;
    let mut c = bank_client();
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.resize(12, ObjType::default());
        cache.objs[10].id = 10;
        cache.objs[10].name = "Dragonhide".into();
        cache.objs[11].id = 11;
        cache.objs[11].name = "Dragonhide".into();
    }
    c.handle_packet(
        ServerProt::UPDATE_INV_FULL,
        &mut Packet::new(vec![2, 89, 0]),
    );
    c.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![11, 12]),
            link_obj_number: Some(vec![5, 7]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    c
}

fn inv_button_req(
    id: i32,
    slot: i32,
    component: i32,
    operation: i32,
    bank_generation: u64,
) -> script::shim::InteractReq {
    script::shim::InteractReq::InvButton {
        id,
        slot,
        component,
        operation,
        bank_generation,
    }
}

fn dispatch_inv_button(snap: &GameSnapshot, req: script::shim::InteractReq) -> GuardRec {
    let (navs, world) = empty_nav();
    let mut rec = GuardRec::default();
    dispatch_script_interact(
        &mut rec,
        snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![req],
    );
    rec
}

#[test]
fn dispatch_inv_button_preserves_selected_same_name_bank_id() {
    let mut c = same_name_bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let ids: Vec<i32> = snap.bank().iter().map(|item| item.def.id).collect();
    assert_eq!(ids, vec![10, 11], "two same-name bank rows");
    assert!(snap
        .bank()
        .iter()
        .all(|item| item.def.name.as_deref() == Some("Dragonhide")));
    let selected = snap
        .bank()
        .iter()
        .find(|item| item.def.id == 11)
        .expect("id 11");
    let rec = dispatch_inv_button(
        &snap,
        inv_button_req(
            selected.def.id,
            selected.slot,
            selected.component_id,
            5,
            snap.bank_session_generation(),
        ),
    );
    assert_eq!(
        rec.menus,
        vec![(
            0,
            MiniMenuAction::INV_BUTTON5,
            11,
            selected.slot,
            selected.component_id
        )],
        "selected id 11 must not fall back to same-name id 10"
    );
    assert_eq!(rec.actions, vec![0], "queued op is not a count answer");
    let first = snap
        .bank()
        .iter()
        .find(|item| item.def.id == 10)
        .expect("id 10");
    let first_rec = dispatch_inv_button(
        &snap,
        inv_button_req(
            first.def.id,
            first.slot,
            first.component_id,
            5,
            snap.bank_session_generation(),
        ),
    );
    assert_eq!(
        first_rec.menus,
        vec![(
            0,
            MiniMenuAction::INV_BUTTON5,
            10,
            first.slot,
            first.component_id
        )]
    );
    let _ = &mut c;
}

#[test]
fn dispatch_inv_button_refuses_stale_closed_missing_forged_and_invalid_op() {
    let mut c = same_name_bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let selected = snap
        .bank()
        .iter()
        .find(|item| item.def.id == 11)
        .expect("id 11");
    let gen = snap.bank_session_generation();
    let invalid = dispatch_inv_button(
        &snap,
        inv_button_req(11, selected.slot, selected.component_id, 9, gen),
    );
    assert!(invalid.menus.is_empty(), "invalid op sends nothing");

    let forged = dispatch_inv_button(
        &snap,
        inv_button_req(9999, selected.slot, selected.component_id, 5, gen),
    );
    assert!(forged.menus.is_empty(), "forged id sends nothing");

    let stale = dispatch_inv_button(
        &snap,
        inv_button_req(
            11,
            selected.slot,
            selected.component_id,
            5,
            gen.wrapping_add(1),
        ),
    );
    assert!(
        stale.menus.is_empty(),
        "stale bank generation sends nothing"
    );

    let wrong_slot = dispatch_inv_button(
        &snap,
        inv_button_req(
            11,
            selected.slot.wrapping_add(1),
            selected.component_id,
            5,
            gen,
        ),
    );
    assert!(
        wrong_slot.menus.is_empty(),
        "same id on another slot is not a fallback"
    );

    c.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![11, 11]),
            link_obj_number: Some(vec![5, 7]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    let mut replaced = GameSnapshot::new();
    replaced.rebuild(&c);
    assert!(replaced.bank().iter().all(|item| item.def.id == 10));
    let missing = dispatch_inv_button(
        &replaced,
        inv_button_req(
            11,
            selected.slot,
            selected.component_id,
            5,
            replaced.bank_session_generation(),
        ),
    );
    assert!(
        missing.menus.is_empty(),
        "replaced identity must not send the old id"
    );

    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut closed = GameSnapshot::new();
    closed.rebuild(&c);
    assert!(closed.bank_component_id() < 0 || closed.bank().is_empty());
    let shut = dispatch_inv_button(
        &closed,
        inv_button_req(
            11,
            selected.slot,
            selected.component_id,
            5,
            closed.bank_session_generation(),
        ),
    );
    assert!(shut.menus.is_empty(), "closed bank sends nothing");
}

#[test]
fn dispatch_inv_button_survives_flatbuffer_round_trip() {
    let mut c = same_name_bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let selected = snap
        .bank()
        .iter()
        .find(|item| item.def.id == 11)
        .expect("id 11");
    let reqs = vec![inv_button_req(
        selected.def.id,
        selected.slot,
        selected.component_id,
        5,
        snap.bank_session_generation(),
    )];
    let bytes = script::isolate_fb::encode_interact_batch(&reqs);
    let decoded = script::isolate_fb::decode_interact_batch(&bytes).expect("inv-button batch");
    assert_eq!(decoded, reqs);
    let rec = dispatch_inv_button(&snap, decoded.into_iter().next().unwrap());
    assert_eq!(
        rec.menus,
        vec![(
            0,
            MiniMenuAction::INV_BUTTON5,
            11,
            selected.slot,
            selected.component_id
        )]
    );
    let _ = &mut c;
}

#[test]
fn dispatch_inv_button_sends_bank_side_deposit_all_on_exact_row() {
    let mut c = bank_client();
    c.handle_packet(
        ServerProt::UPDATE_INV_FULL,
        &mut Packet::new(vec![2, 89, 0]),
    );
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let side = snap
        .bank_side()
        .iter()
        .find(|item| item.def.id == 1)
        .expect("bank-side bones id 1");
    assert_eq!(side.component_id, 701);
    assert_eq!(side.actions[0].as_deref(), Some("Deposit All"));
    let gen = snap.bank_session_generation();
    let rec = dispatch_inv_button(
        &snap,
        inv_button_req(side.def.id, side.slot, side.component_id, 1, gen),
    );
    assert_eq!(
        rec.menus,
        vec![(
            0,
            MiniMenuAction::INV_BUTTON1,
            1,
            side.slot,
            side.component_id
        )],
        "bank-side Deposit All must dispatch exact component 701"
    );
    let _ = &mut c;
}

#[test]
fn dispatch_inv_button_bank_side_refuses_stale_wrong_closed_and_invalid_op() {
    let mut c = bank_client();
    c.handle_packet(
        ServerProt::UPDATE_INV_FULL,
        &mut Packet::new(vec![2, 89, 0]),
    );
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let side = snap
        .bank_side()
        .iter()
        .find(|item| item.def.id == 1)
        .expect("bank-side id 1");
    let gen = snap.bank_session_generation();
    assert!(
        snap.bank_loaded(),
        "identity rejections must exercise a loaded bank"
    );
    assert_eq!(
        dispatch_inv_button(
            &snap,
            inv_button_req(1, side.slot, side.component_id, 1, gen)
        )
        .menus
        .len(),
        1,
        "the valid bank-side row must dispatch before checking rejected variants"
    );
    assert!(
        dispatch_inv_button(
            &snap,
            inv_button_req(1, side.slot, side.component_id, 9, gen)
        )
        .menus
        .is_empty(),
        "invalid bank-side op sends nothing"
    );
    assert!(
        dispatch_inv_button(
            &snap,
            inv_button_req(9999, side.slot, side.component_id, 1, gen)
        )
        .menus
        .is_empty(),
        "forged bank-side id sends nothing"
    );
    assert!(
        dispatch_inv_button(
            &snap,
            inv_button_req(1, side.slot, side.component_id, 1, gen.wrapping_add(1))
        )
        .menus
        .is_empty(),
        "stale bank generation sends nothing for bank-side"
    );
    assert!(
        dispatch_inv_button(
            &snap,
            inv_button_req(1, side.slot.wrapping_add(1), side.component_id, 1, gen)
        )
        .menus
        .is_empty(),
        "wrong bank-side slot is not a fallback"
    );
    assert!(
        dispatch_inv_button(
            &snap,
            inv_button_req(1, side.slot, side.component_id.wrapping_add(1), 1, gen)
        )
        .menus
        .is_empty(),
        "wrong bank-side component sends nothing"
    );
    c.main_modal_id = -1;
    c.side_modal_id = -1;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut closed = GameSnapshot::new();
    closed.rebuild(&c);
    assert!(closed.bank_component_id() < 0 || closed.bank_side().is_empty());
    assert!(
        dispatch_inv_button(
            &closed,
            inv_button_req(
                1,
                side.slot,
                side.component_id,
                1,
                closed.bank_session_generation()
            )
        )
        .menus
        .is_empty(),
        "closed/unloaded bank sends nothing for bank-side"
    );
}

#[test]
fn post_snapshot_bank_side_carries_real_component_id() {
    let mut c = bank_client();
    c.handle_packet(
        ServerProt::UPDATE_INV_FULL,
        &mut Packet::new(vec![2, 89, 0]),
    );
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let side = snap
        .bank_side()
        .iter()
        .find(|item| item.def.id == 1)
        .expect("bank-side id 1");
    assert_eq!(side.component_id, 701);
    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        1,
        None,
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("posted snap");
    let posted = view.bank_side();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].id(), 1);
    assert_eq!(
        posted[0].component_id(),
        701,
        "posted bank_side must keep deposit component for Input.invButton"
    );
    let _ = &mut c;
}

fn trade_offer_client() -> Client {
    use client::client::ClientPlayer;
    use client::config::ObjType;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
    std::mem::forget(listener);
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 0;
    c.local_player = Some(ClientPlayer::at(5, 5));
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs.resize(12, ObjType::default());
        cache.objs[6].id = 6;
        cache.objs[6].name = "Rune essence".into();
        cache.objs[7].id = 7;
        cache.objs[7].name = "Rune essence".into();
        cache.objs[7].certlink = 6;
    }
    c.main_modal_id = 3323;
    c.set_iface(
        3323,
        IfType {
            id: 3323,
            layer_id: 3323,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![3415, 3416, 3417]),
            ..Default::default()
        },
    );
    c.set_iface(
        3415,
        IfType {
            id: 3415,
            layer_id: 3323,
            r#type: ComponentType::TYPE_INV,
            iop: [
                Some("Remove 1".into()),
                Some("Remove 5".into()),
                Some("Remove 10".into()),
                Some("Remove All".into()),
                Some("Remove X".into()),
            ],
            ..Default::default()
        },
    );
    c.set_iface_mut(
        3415,
        IfTypeMut {
            link_obj_type: Some(vec![7, 0]),
            link_obj_number: Some(vec![10, 0]),
            ..Default::default()
        },
    );
    c.set_iface(
        3321,
        IfType {
            id: 3321,
            layer_id: 3321,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![3322]),
            ..Default::default()
        },
    );
    c.set_iface(
        3322,
        IfType {
            id: 3322,
            layer_id: 3321,
            r#type: ComponentType::TYPE_INV,
            iop: [
                Some("Offer".into()),
                Some("Offer 5".into()),
                Some("Offer 10".into()),
                Some("Offer All".into()),
                Some("Offer X".into()),
            ],
            ..Default::default()
        },
    );
    c.set_iface_mut(
        3322,
        IfTypeMut {
            link_obj_type: Some(vec![7, 8]),
            link_obj_number: Some(vec![25, 27]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
    c
}

#[test]
fn dispatch_inv_button_preserves_selected_trade_side_id() {
    let mut c = trade_offer_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    assert!(snap.trade().offer_open);
    let selected = snap
        .trade()
        .side_pack
        .iter()
        .find(|item| item.def.id == 7)
        .expect("id 7");
    let rec = dispatch_inv_button(
        &snap,
        inv_button_req(selected.def.id, selected.slot, selected.component_id, 4, 0),
    );
    assert_eq!(
        rec.menus,
        vec![(
            0,
            MiniMenuAction::INV_BUTTON4,
            7,
            selected.slot,
            selected.component_id
        )],
        "selected trade side id 7 must not fall back to same-name id 6"
    );
    let forged = dispatch_inv_button(
        &snap,
        inv_button_req(9999, selected.slot, selected.component_id, 4, 0),
    );
    assert!(forged.menus.is_empty(), "forged trade id sends nothing");
    let wrong_slot = dispatch_inv_button(
        &snap,
        inv_button_req(
            7,
            selected.slot.wrapping_add(1),
            selected.component_id,
            4,
            0,
        ),
    );
    assert!(
        wrong_slot.menus.is_empty(),
        "same id on another trade slot is not a fallback"
    );
    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut closed = GameSnapshot::new();
    closed.rebuild(&c);
    assert!(!closed.trade().offer_open);
    let shut = dispatch_inv_button(
        &closed,
        inv_button_req(7, selected.slot, selected.component_id, 4, 0),
    );
    assert!(shut.menus.is_empty(), "closed trade sends nothing");
}

#[test]
fn dispatch_script_interact_walk_to_uses_nearest_scene_packet_for_solid_target() {
    let mut c = bank_client();
    c.collision[0].flags[5][7] |= client::dash3d::CollisionFlag::SQ_BLOCKED;
    c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (navs, world) = empty_nav();
    let before = c.out.pos;
    assert!(
        dispatch_script_interact(
            &mut c,
            &snap,
            None,
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![script::shim::InteractReq::WalkTo {
                x: 3205,
                z: 3207,
                level: 0,
            }],
        ),
        "DirectNavigator walk-to must accept the client's nearest stand for a solid target"
    );
    assert!(
        c.out.pos > before,
        "solid-target walk-to must write the try_nearest packet, not idle"
    );
    assert!(
        navs.lock()
            .unwrap()
            .get("alice")
            .is_none_or(|b| b.route.is_none()),
        "walk-to must not arm packed nav"
    );
}

#[test]
fn dispatch_script_interact_walk_forwards_allow_teleports() {
    let mut flags = vec![0u32; 25];
    for z in 0..5 {
        flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
    }
    let dest = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let mut graph = TransportGraph::default();
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: dest,
        loc_id: 0,
        option: 0,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    });
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Some(Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    )));
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    let navs_off = Arc::new(Mutex::new(HashMap::new()));
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((0, 0, 0)),
        &navs_off,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Walk {
            x: 4,
            z: 4,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
    ));
    assert!(
        !wait_until(100, || queued(&navs_off).is_some()),
        "walk-only find must not use the teleport edge"
    );

    let navs_on = Arc::new(Mutex::new(HashMap::new()));
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((0, 0, 0)),
        &navs_on,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Walk {
            x: 4,
            z: 4,
            level: 0,
            allow_teleports: true,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
    ));
    assert!(
        wait_until(200, || queued(&navs_on) == Some(dest)),
        "allow_teleports routes the teleport"
    );
}

/// A 5×12 open strip straddling the surface wilderness edge (z 3520),
/// with the content zone packed on the graph: routing gates wilderness
/// entry on `graph.wilderness`, so a fixture without zones gates nothing.
fn wilderness_entry_world() -> NavWorld {
    let flags = vec![0u32; 5 * 12];
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let graph = TransportGraph {
        wilderness: nav::transport::WildernessRules {
            zones: vec![nav::transport::WildernessZone {
                x1: 2944,
                z1: 3520,
                x2: 3391,
                z2: 6399,
                level1: 0,
                level2: 3,
                origin_z: 3520,
            }],
            divisor: 8,
            offset: 1,
        },
        ..TransportGraph::default()
    };
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 3099,
                z: 3518,
                level: 0,
            },
            width: 5,
            height: 12,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    )
}

#[test]
fn dispatch_script_interact_walk_honors_wilderness_and_bank_fetch_bits() {
    let dest = WorldTile {
        x: 3100,
        z: 3525,
        level: 0,
    };
    let world = Some(Arc::new(wilderness_entry_world()));
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    let navs_off = Arc::new(Mutex::new(HashMap::new()));
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3100, 3519, 0)),
        &navs_off,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Walk {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 1,
        }],
    ));
    assert!(
        !wait_until(100, || queued(&navs_off).is_some()),
        "default-false wilderness must not enter the zone"
    );

    let navs_on = Arc::new(Mutex::new(HashMap::new()));
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((3100, 3519, 0)),
        &navs_on,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Walk {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: 2,
        }],
    ));
    assert!(
        wait_until(200, || queued(&navs_on) == Some(dest)),
        "explicit wilderness must route into the zone"
    );
    let bot = &navs_on.lock().unwrap()["alice"];
    assert_eq!(
        bot.requested_route,
        Some((dest, 0, false, true, true)),
        "dispatch must copy both newly carried FindOptions bits"
    );
}

#[test]
fn changed_wilderness_or_bank_fetch_does_not_coalesce() {
    let dest = WorldTile {
        x: 3,
        z: 3,
        level: 0,
    };
    let navs = Arc::new(Mutex::new(HashMap::from([(
        "flags".to_string(),
        NavBot::default(),
    )])));
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::new(open_world(7, 7))),
        navs: Arc::clone(&navs),
        name: "flags".into(),
        state: None,
        bank: vec![],
    };
    assert!(arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions::default(),
        1,
        true,
        11,
    ));
    assert!(wait_until(200, || {
        navs.lock().unwrap()["flags"].requested_route == Some(native_requested(dest, 1, false))
    }));
    assert!(arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions {
            allow_wilderness: true,
            ..FindOptions::default()
        },
        1,
        true,
        12,
    ));
    assert!(
        wait_until(200, || {
            let bot = &navs.lock().unwrap()["flags"];
            bot.walk_request_id == 12 && bot.requested_route == Some((dest, 1, false, true, false))
        }),
        "changed wilderness must not reuse the prior permissioned route"
    );
    assert!(arm.queue_route(
        dest.x,
        dest.z,
        dest.level,
        FindOptions {
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        },
        1,
        true,
        13,
    ));
    assert!(
        wait_until(200, || {
            let bot = &navs.lock().unwrap()["flags"];
            bot.walk_request_id == 13 && bot.requested_route == Some((dest, 1, false, true, true))
        }),
        "changed bank-fetch must not reuse the prior permissioned route"
    );
}

#[test]
fn dispatch_v2_walk_near_forwards_plane_radius_and_request_id() {
    let dest = WorldTile {
        x: 2,
        z: 2,
        level: 0,
    };
    let flags = vec![0u32; 7 * 7];
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Some(Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 7,
            height: 7,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )));
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((0, 0, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::WalkNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 77,
        }],
    ));
    let bot = &navs.lock().unwrap()["alice"];
    assert_eq!(bot.walk_request_id, 77);
    assert_eq!(bot.requested_route, Some(native_requested(dest, 1, false)));
    assert_eq!(
        bot.requested_route
            .map(|(to, radius, ..)| (to.level, radius)),
        Some((dest.level, 1))
    );
}

/// `Inventory.first` / one `{op:'held',...}` queue entry target a single
/// inv row. Two Bones slots must still write one bury op, not one per
/// name match (Withdraw already `.find`s; Held must match).
#[test]
fn dispatch_script_interact_held_first_match_only() {
    let mut one = bank_fetch_client();
    {
        let cache = Arc::get_mut(&mut one.cache).expect("sole cache owner");
        cache.objs[1].iop = [Some("Bury".into()), None, None, None, None];
    }
    let mut snap_one = GameSnapshot::new();
    snap_one.rebuild(&one);
    let names = api::obj_names::ObjNames::from_objs(&{
        let cache = Arc::get_mut(&mut one.cache).expect("sole cache owner");
        cache.objs.clone()
    });
    let (navs, world) = empty_nav();
    let before_one = one.out.pos;
    assert!(dispatch_script_interact(
        &mut one,
        &snap_one,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Held {
            name: "Bones".into(),
            action: "Bury".into()
        }],
    ));
    let one_op = one.out.pos - before_one;
    assert!(one_op > 0, "control bury must write");

    let mut two = bank_fetch_client();
    {
        let cache = Arc::get_mut(&mut two.cache).expect("sole cache owner");
        cache.objs[1].iop = [Some("Bury".into()), None, None, None, None];
    }
    // Two separate Bones rows (stored 2 = obj 1), matching live multi-slot
    // inv the shim's Inventory.first would pick from once.
    two.set_iface_mut(
        500,
        IfTypeMut {
            link_obj_type: Some(vec![2, 2]),
            link_obj_number: Some(vec![1, 1]),
            ..Default::default()
        },
    );
    let mut snap_two = GameSnapshot::new();
    snap_two.rebuild(&two);
    assert_eq!(
        snap_two
            .inventory()
            .iter()
            .filter(|it| it.def.id == 1)
            .count(),
        2,
        "fixture must expose two Bones slots"
    );
    let before_two = two.out.pos;
    assert!(dispatch_script_interact(
        &mut two,
        &snap_two,
        Some(&names),
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Held {
            name: "Bones".into(),
            action: "Bury".into()
        }],
    ));
    assert_eq!(
        two.out.pos - before_two,
        one_op,
        "one Held request must bury Inventory.first only, not every Bones slot"
    );
}

/// Task 8 — the shim's `Npc` interact request resolves by name and
/// records the matching menu op on the driver (`Pick` → OP_NPC1).
#[test]
fn dispatch_script_interact_sends_npc_pick() {
    use client::client::MiniMenuAction;
    use client::config::NpcType;
    use client::dash3d::ClientNpc;

    let mut c = nav_client();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(client::dash3d::ClientPlayer::at(5, 5));
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.npcs.len() <= 9 {
            cache.npcs.push(NpcType::default());
        }
        cache.npcs[9] = NpcType {
            id: 9,
            name: "Goblin".into(),
            op: vec![Some("Pick".into()), Some("Examine".into())],
            ..Default::default()
        };
    }
    let mut npc = ClientNpc::at(6, 6);
    npc.r#type = Some(9);
    c.npc[7] = Some(Box::new(npc));
    c.npc_ids = vec![7];
    c.npc_count = 1;
    c.bump_gens(ServerProt::REBUILD_NORMAL);

    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    assert!(
        snap.npcs()
            .iter()
            .any(|n| n.name.as_deref() == Some("Goblin")),
        "seeded npc must appear in snapshot"
    );

    let (navs, world) = empty_nav();
    let mut rec = GuardRec::default();
    assert!(dispatch_script_interact(
        &mut rec,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Npc {
            name: "Goblin".into(),
            action: "Pick".into(),
            index: None,
        }],
    ));
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 7, 0, 0)],
        "Pick is the first npc op"
    );
}

/// Task 8 fix — when `index` is supplied, match that slot only even if
/// an earlier same-named NPC would win by name.
#[test]
fn dispatch_script_interact_npc_index_beats_same_name() {
    use client::client::MiniMenuAction;
    use client::config::NpcType;
    use client::dash3d::ClientNpc;

    let mut c = nav_client();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(client::dash3d::ClientPlayer::at(5, 5));
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.npcs.len() <= 9 {
            cache.npcs.push(NpcType::default());
        }
        cache.npcs[9] = NpcType {
            id: 9,
            name: "Goblin".into(),
            op: vec![Some("Pick".into()), Some("Examine".into())],
            ..Default::default()
        };
    }
    let mut first = ClientNpc::at(6, 6);
    first.r#type = Some(9);
    c.npc[3] = Some(Box::new(first));
    let mut second = ClientNpc::at(7, 7);
    second.r#type = Some(9);
    c.npc[7] = Some(Box::new(second));
    c.npc_ids = vec![3, 7];
    c.npc_count = 2;
    c.bump_gens(ServerProt::REBUILD_NORMAL);

    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let goblins: Vec<_> = snap
        .npcs()
        .iter()
        .filter(|n| n.name.as_deref() == Some("Goblin"))
        .collect();
    assert_eq!(goblins.len(), 2, "two same-named npcs seeded");

    let (navs, world) = empty_nav();
    let mut rec = GuardRec::default();
    assert!(dispatch_script_interact(
        &mut rec,
        &snap,
        None,
        Some((3205, 3205, 0)),
        &navs,
        &world,
        None,
        "alice",
        vec![script::shim::InteractReq::Npc {
            name: "Goblin".into(),
            action: "Pick".into(),
            index: Some(7),
        }],
    ));
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 7, 0, 0)],
        "indexed npc wins over earlier same name"
    );
}

/// The BankBudget session fixture: [`bank_client`] plus a junk
/// inventory (Bones × 3 on side tab 3), the bank's obj 2 renamed
/// "Knife", and the bank's withdraw component holding the knife
/// (stored 3 = obj 2) — the worn-req item the session must fetch.
fn bank_fetch_client() -> Client {
    let mut c = bank_client();
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.objs[2].name = "Knife".into();
    }
    c.side_icon[3] = 500;
    c.set_iface(
        500,
        IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        500,
        IfTypeMut {
            link_obj_type: Some(vec![2, 0]), // stored 2 = obj 1 (Bones)
            link_obj_number: Some(vec![3, 0]),
            ..Default::default()
        },
    );
    // The bank's withdraw component (601) holds the knife, not Bones.
    c.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![3, 0]), // stored 3 = obj 2 (Knife)
            link_obj_number: Some(vec![20, 0]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut full = Packet::new(vec![2, 89, 2, 0, 3, 20, 0, 0, 0]);
    c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut full);
    c
}

#[test]
fn player_here_tile_uses_minusedlevel_not_hardcoded_zero() {
    let mut c = bank_fetch_client();
    c.minusedlevel = 1;
    c.local_player = Some(client::dash3d::ClientPlayer::at(7, 8));
    c.map_build_base_x = 100;
    c.map_build_base_z = 200;
    assert_eq!(player_here_tile(&c), Some((107, 208, 1)));
}

#[test]
fn step_bank_fetch_walk_stand_matches_player_plane() {
    use nav::bank_fetch::BankStep;
    use nav::router::{Leg, Route};
    use std::collections::VecDeque;

    let final_route = Route {
        legs: vec![Leg::Walk {
            tiles: vec![WorldTile {
                x: 99,
                z: 99,
                level: 0,
            }],
        }],
        dest: WorldTile {
            x: 99,
            z: 99,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut bot = NavBot {
        bank_fetch: Some(PendingBankFetch {
            steps: VecDeque::from([BankStep::Walk {
                x: 10,
                z: 20,
                level: 1,
            }]),
            dest: WorldTile {
                x: 99,
                z: 99,
                level: 0,
            },
            opts: FindOptions::default(),
            final_route: final_route.clone(),
        }),
        ..Default::default()
    };
    let snap = GameSnapshot::new();
    let mut driver = bank_fetch_client();
    step_bank_fetch_on_bot(&mut driver, &snap, &mut bot, None, Some((10, 20, 1)), false);
    assert_eq!(
        bot.route,
        Some(final_route.clone()),
        "stand Walk completes when here matches the stand plane"
    );

    bot.bank_fetch = Some(PendingBankFetch {
        steps: VecDeque::from([BankStep::Walk {
            x: 10,
            z: 20,
            level: 1,
        }]),
        dest: WorldTile {
            x: 99,
            z: 99,
            level: 0,
        },
        opts: FindOptions::default(),
        final_route: final_route.clone(),
    });
    bot.route = None;
    step_bank_fetch_on_bot(&mut driver, &snap, &mut bot, None, Some((10, 20, 0)), false);
    assert!(
        bot.route.is_none(),
        "ground-plane here must not complete an upstairs stand Walk"
    );
}

/// A 5×5 world walled between x=1 and x=2, crossed only by a door
/// gated on wearing a knife (obj `knife_id`), with a bank booth
/// stand at (0, 4).
fn knife_nav_world(knife_id: i32) -> NavWorld {
    let mut flags = vec![0u32; 25];
    for z in 0..5 {
        flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
    }
    let edge = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 1,
            z: 2,
            level: 0,
        },
        to: WorldTile {
            x: 2,
            z: 2,
            level: 0,
        },
        loc_id: 2882,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![knife_id],
        members_req: false,
        wildy_cap: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(edge.at).or_default().push(0);
    graph.edges.push(edge);
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        },
        graph,
        vec![nav::pack::BankStand {
            name: "Bank booth".into(),
            tile: WorldTile {
                x: 0,
                z: 4,
                level: 0,
            },
            access: nav::pack::BankAccess::Booth { op: 2 },
        }],
    )
}

/// Task 8 — the BankBudget session unit: the inventory is full of
/// junk and the knife is in the **bank snapshot**. The strict
/// `find_with` stays fail-closed (no knife worn); the diagnosis
/// names only the worn knife; the session plans walk → open →
/// deposit the backpack → withdraw the knife → wear → close; and the
/// post-session strict re-find crosses. `find` itself never fetches.
#[test]
fn bank_fetch_session_deposits_withdraws_wears_then_finds() {
    let c = bank_fetch_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    // The junk backpack and the open bank's knife row come from the
    // snapshot, exactly as the pump would read them.
    assert_eq!(snap.inv(), &[(1, 3)], "the junk backpack");
    assert!(
        snap.bank().iter().any(|it| it.def.id == 2 && it.count >= 1),
        "the knife is in the open bank"
    );
    let world = knife_nav_world(2);
    let from = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let to = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let state = WorldState::from_snapshot(&snap);
    assert!(
        matches!(
            find_with(
                &world.collision,
                &world.graph,
                from,
                to,
                FindOptions::default(),
                &state,
            ),
            Err(nav::router::RouteError::NoPath)
        ),
        "junk but no knife stays fail-closed even for the session"
    );
    let missing = nav::router::find_missing_item_reqs(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &state,
    )
    .expect("only the worn knife is missing");
    assert_eq!(
        missing,
        vec![nav::router::MissingReq::WearAny { ids: vec![2] }]
    );
    let bank_rows: Vec<(i32, i32)> = snap.bank().iter().map(|it| (it.def.id, it.count)).collect();
    let fetch = nav::bank_fetch::plan_bank_fetch(&missing, &state, &bank_rows, world.banks(), from)
        .expect("the banked knife plans a trip");
    assert_eq!(
        fetch.steps,
        vec![
            nav::bank_fetch::BankStep::Walk {
                x: 0,
                z: 4,
                level: 0
            },
            nav::bank_fetch::BankStep::Open,
            nav::bank_fetch::BankStep::DepositAll,
            nav::bank_fetch::BankStep::Withdraw { id: 2, count: 1 },
            nav::bank_fetch::BankStep::Wear { id: 2 },
            nav::bank_fetch::BankStep::Close,
        ],
        "deposit the junk, withdraw the knife, wear it, close"
    );
    let r = find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions::default(),
        &fetch.state,
    )
    .expect("the post-session strict re-find crosses");
    assert_eq!(r.dest, to);
}

/// Fix round — BankBudget execute on the live walk arm: `allow_bank_fetch`
/// on + junk inv + knife in the open bank snapshot must latch a session
/// on [`ScriptWalkArm::route`] and actually drive Deposit/Withdraw on
/// the Driver (not only `plan_bank_fetch` in isolation). Start on the
/// packed bank stand so Walk completes in place; the client's bank is
/// already open so Open is a no-op; DepositAll + Withdraw must write.
#[test]
fn allow_bank_fetch_on_script_walk_arm_drives_deposit_withdraw() {
    let mut c = bank_fetch_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let world = Arc::new(knife_nav_world(2));
    let state = WorldState::from_snapshot(&snap);
    let bank_rows: Vec<(i32, i32)> = snap.bank().iter().map(|it| (it.def.id, it.count)).collect();
    assert!(
        bank_rows.iter().any(|&(id, n)| id == 2 && n >= 1),
        "knife is in the open bank"
    );
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    // Stand on the packed bank booth so the session's Walk completes
    // without a follow hop; Open sees the already-open bank.
    let arm = ScriptWalkArm {
        here: Some((0, 4, 0)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: Some(state),
        bank: bank_rows,
    };
    assert!(
        arm.route(
            4,
            4,
            0,
            FindOptions {
                allow_bank_fetch: true,
                ..FindOptions::default()
            },
        ),
        "the walk arm must accept the bank-fetch route"
    );
    // The worker latches the session; wait briefly for it.
    let mut latched = false;
    for _ in 0..200 {
        if navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|b| b.bank_fetch.is_some())
        {
            latched = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(latched, "allow_bank_fetch must latch a BankFetch session");
    let out_before = c.out.pos;
    // Pump until DepositAll + Withdraw have had a chance to write (Walk
    // and Open complete immediately on this fixture).
    for _ in 0..16 {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("alice").expect("nav bot");
        if bot.bank_fetch.is_none() {
            break;
        }
        step_bank_fetch_on_bot(
            &mut c,
            &snap,
            bot,
            Some(world.as_ref()),
            Some((0, 4, 0)),
            false,
        );
    }
    assert!(
        c.out.pos > out_before,
        "BankBudget execute must drive deposit/withdraw on the Driver (pos {} → {})",
        out_before,
        c.out.pos
    );
}

/// Fix round 2 — BankBudget Walk from off the stand must poll
/// [`Traveller::follow`] on the stand sub-route. Freeze-all-follow
/// while `bank_fetch` is Some stalls Walk forever; following
/// `final_route` would skip the booth. Start at (0,0), stand at
/// (0,4), final dest (4,4).
#[test]
fn allow_bank_fetch_off_stand_walk_follows_stand_sub_route() {
    let mut c = bank_fetch_client();
    c.local_player = Some(client::dash3d::ClientPlayer::at(0, 0));
    c.bump_gens(client::io::ServerProt::PLAYER_INFO);
    c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let world = Arc::new(knife_nav_world(2));
    let state = WorldState::from_snapshot(&snap);
    let bank_rows: Vec<(i32, i32)> = snap.bank().iter().map(|it| (it.def.id, it.count)).collect();
    assert!(
        bank_rows.iter().any(|&(id, n)| id == 2 && n >= 1),
        "knife is in the open bank"
    );
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
    // Off the packed booth: Walk must arm a stand sub-route and follow
    // it — not stall, not jump to the knife-gated final dest.
    let arm = ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: Some(state),
        bank: bank_rows,
    };
    assert!(
        arm.route(
            4,
            4,
            0,
            FindOptions {
                allow_bank_fetch: true,
                ..FindOptions::default()
            },
        ),
        "the walk arm must accept the bank-fetch route"
    );
    let mut latched = false;
    for _ in 0..200 {
        if navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|b| b.bank_fetch.is_some())
        {
            latched = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(latched, "allow_bank_fetch must latch a BankFetch session");
    // Live pump: session Walk + follow (not step_bank_fetch alone).
    step_nav_bot(
        &mut c,
        "alice",
        Some((0, 0, 0)),
        &snap,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        no_reach,
    );
    let all = navs.lock().unwrap();
    let bot = all.get("alice").expect("nav bot");
    assert!(
        bot.bank_fetch
            .as_ref()
            .is_some_and(|p| matches!(p.steps.front(), Some(BankStep::Walk { .. }))),
        "Walk must still be the front step (not skipped to Open/final)"
    );
    let dest = bot.route.as_ref().map(|r| r.dest);
    assert_eq!(
        dest,
        Some(WorldTile {
            x: 0,
            z: 4,
            level: 0
        }),
        "armed route must be the stand sub-route, not final (4,4)"
    );
    assert!(
        bot.traveller.current_aim().is_some(),
        "Walk must poll Traveller::follow on the stand sub-route (freeze-all-follow stalls)"
    );
}

/// Test script that counts ticks into a shared cell (the panel cannot
/// read a running script's internals, so the wiring tests observe the
/// side effect instead).
#[derive(Default)]
struct TickCounter(Arc<Mutex<u32>>);

impl script::Script for TickCounter {
    fn name(&self) -> &str {
        "TickCounter"
    }
    fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
        *self.0.lock().unwrap() += 1;
    }
}

/// One `script_observe` wiring rig: a started slot script for
/// "alice" plus its (empty) cheat queue.
struct ScriptWiring {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    count: Arc<Mutex<u32>>,
}

fn script_wiring() -> ScriptWiring {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let count = Arc::new(Mutex::new(0));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(TickCounter(Arc::clone(&count))), None)
        .unwrap();
    cheats
        .lock()
        .unwrap()
        .insert("alice".into(), VecDeque::new());
    ScriptWiring {
        scripts,
        cheats,
        count,
    }
}

/// Empty nav rig for observe tests that never touch `ctx.walk`: no
/// nav bots and no nav world (the walk hook would refuse anyway).
type EmptyNav = (Arc<Mutex<HashMap<String, NavBot>>>, Option<Arc<NavWorld>>);
fn empty_nav() -> EmptyNav {
    (Arc::new(Mutex::new(HashMap::new())), None)
}

/// Poll `cond` for up to `ms` milliseconds. The route-arming worker is
/// detached, so tests wait on the effect instead of joining it.
fn wait_until(ms: u64, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(1));
    }
    cond()
}

#[test]
fn script_observe_ticks_only_on_player_edge_while_up() {
    let ScriptWiring {
        scripts,
        cheats,
        count,
    } = script_wiring();
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    // Not up: the edge must not dispatch (the is_up pause gate).
    script_observe(
        &mut c, "alice", false, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert_eq!(*count.lock().unwrap(), 0);
    // Up + edge: exactly one tick.
    script_observe(
        &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert_eq!(*count.lock().unwrap(), 1);
    // Up but no edge: nothing.
    script_observe(
        &mut c, "alice", true, false, 2, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert_eq!(*count.lock().unwrap(), 1);
    // A dispatched tick wrote the driver's out buffer (the slot's own
    // `Client` sends it on the next mainloop pass).
    assert!(script_observe(
        &mut c, "alice", true, true, 3, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false
    ));
    assert_eq!(*count.lock().unwrap(), 2);
}

#[test]
fn script_run_policy_override_reaches_host_auto_run_and_stop_clears_it() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    cheats
        .lock()
        .unwrap()
        .insert("alice".into(), VecDeque::new());
    let (navs, world) = empty_nav();
    let slot = script_slot_or_insert(&scripts, "alice");
    let run_policy_override = slot.lock().unwrap().run_policy_override_cell();
    {
        let mut slot = slot.lock().unwrap();
        slot.start_load_settled(
            r#"
import { RunManager } from '../../runtime/RunManager.js';
export default class T extends LoopingBot {
    loop() {
        RunManager.override({ energyMin: 80 });
    }
}
"#
            .into(),
            script::LoadShape::CompatClass,
            vec![],
        )
        .expect("run-policy script starts");
    }

    let mut client = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    client.ingame = true;
    client.scene_state = 2;
    client.local_player = Some(client::client::ClientPlayer::at(10, 10));
    client.gens.player = 1;
    client.gens.player_info = 1;
    client.runenergy = 20;

    script_observe(
        &mut client,
        "alice",
        true,
        true,
        1,
        Some((10, 10, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    slot.lock()
        .unwrap()
        .probe("true")
        .expect("the script's policy override tick completes");
    assert_eq!(
        slot.lock().unwrap().run_policy_override(),
        Some(api::run_policy::RunPolicyOverride {
            run_auto: None,
            energy_min: Some(api::run_policy::RunEnergyMin::Floor(80)),
        })
    );

    let drive_one_host_frame = |client: &mut Client| {
        let done = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&done);
        Host::run_client(
            client,
            "alice",
            vault::ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            None,
            Arc::clone(&run_policy_override),
            move |_, _, _, _| {
                observed.store(true, Ordering::Relaxed);
                false
            },
            move |_| done.load(Ordering::Relaxed),
            |_| RandomClaim::Host,
        );
    };
    let run_button_sent = |client: &Client| {
        client.out.data()[..client.out.pos]
            .windows(3)
            .any(|packet| {
                packet[0] == client::io::ClientProt::IF_BUTTON.id as u8
                    && u16::from_be_bytes([packet[1], packet[2]])
                        == api::interact::RUN_ORB_IFACE as u16
            })
    };

    drive_one_host_frame(&mut client);
    assert!(
        !run_button_sent(&client),
        "the script's energyMin override suppresses auto-run below 80 energy"
    );

    slot.lock().unwrap().stop();
    assert_eq!(
        run_policy_override.get(),
        None,
        "Stop clears the shared cell"
    );
    client.out.pos = 0;
    drive_one_host_frame(&mut client);
    assert!(
        run_button_sent(&client),
        "after Stop, host auto-run falls back to the host's 20-energy default"
    );
}

#[test]
fn script_observe_idle_slot_publishes_nothing_on_tick_edge() {
    // Task 12: an Idle SlotScript must not publish a script snapshot —
    // no dispatch and no driver write, so the slot has nothing to send
    // on the next mainloop pass.
    let scripts = Arc::new(Mutex::new(HashMap::new()));
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    let count = Arc::new(Mutex::new(0));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    // Never started: no SlotScript entry (Idle). Edge + up publishes
    // nothing — the driver's out buffer stays empty.
    assert!(!script_observe(
        &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false
    ));
    assert_eq!(c.out.pos, 0, "no script bytes on the driver");
    // Started then stopped: Idle again, same skip.
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(TickCounter(Arc::clone(&count))), None)
        .unwrap();
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
    assert_eq!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .state(),
        script::RunState::Idle
    );
    assert!(!script_observe(
        &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false
    ));
    assert_eq!(*count.lock().unwrap(), 0, "Idle must not dispatch tick");
}

#[test]
fn nav_world_state_for_observe_skips_when_idle_and_nav_unarmed() {
    let c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let here = Some((3200, 3200, 0));
    assert!(
        nav_world_state_for_observe(here, &snap, false, false, false).is_none(),
        "idle slot with no armed nav must not build WorldState"
    );
    assert!(
        nav_world_state_for_observe(here, &snap, true, false, false).is_some(),
        "Running script must get WorldState for the walk arm"
    );
    assert!(
        nav_world_state_for_observe(here, &snap, false, true, false).is_some(),
        "armed nav bot must get WorldState for interact walks"
    );
}

#[test]
fn script_observe_compiled_running_skips_isolate_snapshot_encode() {
    let ScriptWiring {
        scripts,
        cheats,
        count: _,
    } = script_wiring();
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let state = WorldState::from_snapshot(&snap);
    assert!(!script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .load_active());
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3200, 3200, 0)),
        None,
        Some(state),
        Some(&snap),
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(
        !script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .has_snapshot_fingerprint(),
        "compiled-only Running must not encode isolate snapshot delta"
    );
}

#[test]
fn projected_npc_boxes_are_lazy_for_the_isolate_snapshot_gate() {
    let ScriptWiring {
        scripts: compiled_scripts,
        cheats: _,
        count: _,
    } = script_wiring();
    let mut projections = 0;
    assert!(
        project_npc_boxes_for_isolate_snapshot(&compiled_scripts, "alice", true, || {
            projections += 1;
            Some(Vec::new())
        },)
        .is_none()
    );
    assert_eq!(projections, 0, "compiled scripts consume no snapshots");

    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let slot = script_slot_or_insert(&scripts, "alice");
    slot.lock()
        .unwrap()
        .start_load_settled(
            "export default class T extends LoopingBot { loop() {} }".to_string(),
            script::LoadShape::CompatClass,
            vec![],
        )
        .expect("load isolate starts");

    assert!(
        project_npc_boxes_for_isolate_snapshot(&scripts, "alice", false, || {
            projections += 1;
            Some(Vec::new())
        })
        .is_none()
    );
    assert_eq!(projections, 0, "non-tick frames consume no snapshots");

    assert_eq!(
        project_npc_boxes_for_isolate_snapshot(&scripts, "alice", true, || {
            projections += 1;
            Some(Vec::new())
        }),
        Some(Vec::new()),
        "a running isolate consumes the tick-edge snapshot"
    );
    assert_eq!(projections, 1);

    slot.lock().unwrap().pause();
    assert!(
        project_npc_boxes_for_isolate_snapshot(&scripts, "alice", true, || {
            projections += 1;
            Some(Vec::new())
        })
        .is_none()
    );
    assert_eq!(projections, 1, "paused isolates consume no snapshots");
    slot.lock().unwrap().stop();
}

#[test]
fn script_observe_load_isolate_still_encodes_snapshot_on_tick_edge() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    cheats
        .lock()
        .unwrap()
        .insert("alice".into(), VecDeque::new());
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = "export default class T extends LoopingBot { loop() {} }";
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .load_active());
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3200, 3200, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .has_snapshot_fingerprint(),
        "Load isolate must still encode snapshot delta on tick edge"
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn script_observe_drains_queued_cheat_onto_driver() {
    let ScriptWiring {
        scripts,
        cheats,
        count,
    } = script_wiring();
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    cheats
        .lock()
        .unwrap()
        .get_mut("alice")
        .unwrap()
        .push_back("setvar tutorial 1000".into());
    let wrote = script_observe(
        &mut c, "alice", true, false, 0, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert!(wrote, "the cheat wrote the driver's out buffer");
    assert_eq!(
        c.out.data()[0],
        client::io::ClientProt::CLIENT_CHEAT.id as u8
    );
    assert!(
        cheats.lock().unwrap().get("alice").unwrap().is_empty(),
        "a drained queue stays for the next panel push"
    );
    assert_eq!(
        *count.lock().unwrap(),
        0,
        "no tick edge → the script must not run"
    );
}

/// Records what a dispatched tick's ctx exposed: whether the inventory
/// view and the shared name table reached the script, and the resolved
/// `has_item` answer for "Bones".
#[derive(Default)]
struct InvProbe(Arc<Mutex<Option<(bool, bool, bool)>>>);

impl script::Script for InvProbe {
    fn name(&self) -> &str {
        "InvProbe"
    }
    fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        *self.0.lock().unwrap() = Some((
            ctx.inv.is_some(),
            ctx.obj_names.is_some(),
            ctx.has_item("Bones"),
        ));
    }
}

#[test]
fn script_observe_passes_inventory_when_running() {
    let mut objs = vec![client::config::ObjType::default(); 2];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let seen = Arc::new(Mutex::new(None));
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(InvProbe(Arc::clone(&seen))), None)
        .unwrap();
    let inv: Vec<(i32, i32)> = vec![(1, 3), (0, 0)];
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        None,
        Some(&inv),
        None,
        None,
        Some(&names),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        *seen.lock().unwrap(),
        Some((true, true, true)),
        "a Running script sees the inventory view and resolves names"
    );
}

/// Records whether the per-tick snapshot reached the script ctx and
/// what the `varp` getter read through it.
type SnapProbeSeen = Option<(bool, Option<i32>)>;

#[derive(Default)]
struct SnapProbe(Arc<Mutex<SnapProbeSeen>>);

impl script::Script for SnapProbe {
    fn name(&self) -> &str {
        "SnapProbe"
    }
    fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        *self.0.lock().unwrap() = Some((ctx.snapshot.is_some(), ctx.varp(101)));
    }
}

#[test]
fn script_observe_passes_the_tick_snapshot_to_the_ctx() {
    let seen = Arc::new(Mutex::new(None));
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(SnapProbe(Arc::clone(&seen))), None)
        .unwrap();
    // A transmitted varp table so the probe's `varp(101)` has a value
    // to read (the snapshot only lists transmitted definitions).
    let cache = Cache {
        varps: (0..102)
            .map(|_| client::config::VarpType::default())
            .collect(),
        ..Default::default()
    };
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(cache),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.var = vec![0; 102];
    c.var[101] = 5;
    c.bump_gens(ServerProt::VARP_SYNC);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        None,
        None,
        None,
        Some(&snap),
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        *seen.lock().unwrap(),
        Some((true, Some(5))),
        "the observe snapshot reaches the ctx and the varp getter reads it"
    );
}

// Task 9b — the posted blob is FlatBuffers (schema:
// crates/script/schema/isolate.fbs) and carries exactly the fields the
// shim Game/Inventory/Skills/EventSignal read: inv rows carry resolved
// obj names (None when the table has none), stats rows the stat
// index/name/xp/base/effective, bank flags from the snapshot, and
// hold/ours pass through for EventSignal.pending(). No World clone —
// only these fields. Round-trips through the script crate's decoder.
#[test]
fn script_snapshot_fb_carries_observed_fields_only() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.runenergy = 42;
    c.stat_effective_level[7] = 40;
    c.stat_base_level[7] = 35;
    c.stat_xp[7] = 1300;
    c.bump_gens(ServerProt::UPDATE_STAT);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let mut objs = vec![client::config::ObjType::default(); 2];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let inv = vec![(1, 2), (99, 5)];
    let (bytes, _fp) = script_snapshot_fb(
        None,
        false,
        7,
        Some((3200, 3200, 0)),
        true,
        Some(&inv),
        Some(&snap),
        Some(&names),
        None,
        true,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
    assert_eq!(view.tick(), 7);
    assert!(view.has_here(), "keyframe carries here");
    let here = view.here().expect("here posted");
    assert_eq!((here.x(), here.z(), here.level()), (3200, 3200, 0));
    assert!(view.ingame());
    assert!(view.has_inv(), "keyframe carries inv");
    let inv = view.inv();
    assert_eq!(inv.len(), 2);
    assert_eq!((inv[0].name(), inv[0].count()), (Some("Bones"), 2));
    assert_eq!(
        (inv[1].name(), inv[1].count()),
        (None, 5),
        "an obj the table does not know posts a null name, never invented"
    );
    assert!(view.has_stats(), "keyframe carries stats");
    let stats = view.stats();
    assert_eq!(
        (
            stats[7].index(),
            stats[7].name(),
            stats[7].xp(),
            stats[7].base(),
            stats[7].effective()
        ),
        (7, "cooking", 1300, 35, 40)
    );
    assert!(!view.bank_open(), "no bank component in the fixture");
    assert!(!view.bank_loaded());
    assert!(
        view.booths().is_empty(),
        "no Use-quickly scene locs in the fixture"
    );
    assert!(
        view.banks().is_empty(),
        "no nav world: no packed stands posted"
    );
    assert!(view.bank().is_empty());
    assert!(view.bank_side().is_empty());
    assert!(view.hold());
    assert!(!view.ours());
    assert!(
        view.has_attacked_by_player(),
        "keyframe carries attacked_by_player"
    );
    assert!(!view.attacked_by_player());
    assert!(view.has_widgets(), "keyframe carries widgets vector");
    assert!(view.widgets().is_empty());
    assert!(
        view.has_bank_approaches(),
        "keyframe posts bank_approaches even when empty"
    );
    assert!(view.bank_approaches().is_empty());

    // No tile / no snapshot: fail-closed nulls and flags.
    let (bare_bytes, _) = script_snapshot_fb(
        None, false, 1, None, false, None, None, None, None, false, true, false,
    );
    let bare = script::isolate_fb::decode_snapshot(&bare_bytes).expect("bare blob decodes");
    assert!(bare.here().is_none());
    assert!(bare.inv().is_empty());
    assert!(bare.stats().is_empty());
    assert!(!bare.bank_open());
    assert!(bare.ours(), "ours rides the blob for EventSignal");
}

#[test]
fn script_snapshot_fb_projects_authoritative_bank_approach() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 0;
    c.local_player = Some(client::client::ClientPlayer::at(5, 5));
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.locs.extend(
            (0..(2214usize.saturating_sub(cache.locs.len())))
                .map(|_| client::config::LocType::default()),
        );
        cache.locs[2213].id = 2213;
        cache.locs[2213].name = "Bank booth".into();
        cache.locs[2213].op = vec![None, Some("Use-quickly".into()), None, None, None];
    }
    let booth_typecode = 0x4000_0000 + (2213 << 14) + 1 + (2 << 7);
    c.world
        .set_wall(0, 6, 6, 0, 0, 0, booth_typecode, 10, 0, 0, 0, 0);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        1,
        Some((3205, 3205, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
    assert!(view.has_bank_approaches());
    let rows = view.bank_approaches();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].loc_id(), 2213);
    assert_eq!((rows[0].x(), rows[0].z()), (3206, 3206));
    assert!(!rows[0].can_operate(), "diagonal is not test_loc ready");
    assert!(rows[0].dest_ok());
    assert_ne!((rows[0].dest_x(), rows[0].dest_z()), (3206, 3206));
    assert_ne!(
        (rows[0].dest_x(), rows[0].dest_z()),
        (3205, 3205),
        "dest is not the signum stay-put tile"
    );
}

#[test]
fn native_trade_controls_reach_script_accept_through_snapshot() {
    for (root, button, hidden) in [(3323, 3420, false), (3443, 3546, false), (3323, 3420, true)] {
        let mut c = trade_offer_client();
        c.main_modal_id = root;
        c.set_iface(
            root as usize,
            IfType {
                id: root,
                layer_id: root,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![button, button + 1]),
                ..Default::default()
            },
        );
        c.set_iface(
            button as usize,
            IfType {
                id: button,
                layer_id: root,
                r#type: ComponentType::TYPE_RECT,
                button_text: "Ok".into(),
                ..Default::default()
            },
        );
        c.set_iface_mut(
            button as usize,
            IfTypeMut {
                button_type: 1,
                hide: hidden,
                ..Default::default()
            },
        );
        c.set_iface(
            (button + 1) as usize,
            IfType {
                id: button + 1,
                layer_id: root,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            (button + 1) as usize,
            IfTypeMut {
                text: "Accept".into(),
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&c);
        let (bytes, _) = script_snapshot_fb(
            None,
            false,
            1,
            Some((3205, 3205, 0)),
            true,
            None,
            Some(&snapshot),
            None,
            None,
            false,
            false,
            false,
        );
        let iso = script::LoadIsolate::spawn(
            r#"import { Trade } from '../../api/trade/Trade.js';
                export default class T extends LoopingBot {
                    async loop() {
                        if (globalThis.__did) return;
                        globalThis.__did = true;
                        globalThis.__accepted = await Trade.accept();
                    }
                }"#
            .into(),
            script::LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        iso.post_snapshot(bytes);
        iso.on_game_tick(1);
        let accepted = iso.probe("globalThis.__accepted").unwrap();
        let interactions = iso.drain_interacts();
        iso.join();
        assert_eq!(accepted, serde_json::Value::Bool(!hidden));
        let expected = if hidden {
            vec![]
        } else {
            vec![script::shim::InteractReq::IfButton {
                component_id: button,
            }]
        };
        assert_eq!(interactions, expected);
    }
}

#[test]
fn script_snapshot_player_actions_preserve_native_slots() {
    fn emitted_actions(player_op: [Option<&str>; 5]) -> Vec<String> {
        let mut c = prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.ingame = true;
        c.player_count = 1;
        c.player_ids[0] = 0;
        c.players[0] = Some(Box::new(client::client::ClientPlayer {
            name: Some("partner".into()),
            ..Default::default()
        }));
        c.player_op = player_op.map(|action| action.map(str::to_owned));
        c.bump_gens(ServerProt::PLAYER_INFO);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&c);
        let (bytes, _) = script_snapshot_fb(
            None,
            false,
            1,
            None,
            true,
            None,
            Some(&snapshot),
            None,
            None,
            false,
            false,
            false,
        );
        let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
        view.players()
            .first()
            .expect("native player emitted")
            .actions()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    // The production snapshot producer must retain the op4 identity;
    // dropping holes would move Trade with from native slot 4 to slot 2.
    assert_eq!(
        emitted_actions([None, None, Some("Follow"), Some("Trade with"), None]),
        vec!["", "", "Follow", "Trade with", ""]
    );
    // Missing and hidden native slots remain unavailable rather than
    // being synthesized as a Trade action.
    assert_eq!(
        emitted_actions([None, None, Some("Follow"), None, None]),
        vec!["", "", "Follow", "", ""]
    );
    assert_eq!(
        emitted_actions([None, None, Some("Follow"), Some("hidden"), None]),
        vec!["", "", "Follow", "hidden", ""]
    );
}

#[test]
fn script_snapshot_crops_shared_canlight_from_profile_plane() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 1;
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    assert!(snap.scene().available, "scene must materialize for crop");
    let origin_x = 3100;
    let origin_z = 3100;
    let world_w = 200usize;
    let world_h = 200usize;
    let cells = 4 * world_w * world_h;
    let collision = WorldCollision {
        origin: WorldTile {
            x: origin_x,
            z: origin_z,
            level: 0,
        },
        width: world_w,
        height: world_h,
        walk: vec![0; cells],
        blocked: vec![0; cells.div_ceil(64)],
        flags: None,
    };
    let world = NavWorld::from_parts(collision, nav::transport::TransportGraph::default(), vec![]);
    let mut bits = vec![0u64; cells.div_ceil(64)];
    let lit = WorldTile {
        x: 3205,
        z: 3206,
        level: 1,
    };
    let idx =
        world_w * world_h + (lit.z - origin_z) as usize * world_w + (lit.x - origin_x) as usize;
    bits[idx / 64] |= 1u64 << (idx % 64);
    let bytes = with_script_snapshot_input(
        1,
        Some((3205, 3205, 1)),
        true,
        None,
        Some(&snap),
        None,
        Some(&world),
        None,
        false,
        false,
        false,
        0,
        false,
        0,
        false,
        0,
        false,
        Some(bits.as_slice()),
        PostedWalkOutcome::default(),
        PostedInspect::default(),
        script::isolate_fb::encode_snapshot_with_native,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("snapshot decodes");
    let reach = view.reach().expect("reach posted");
    assert!(reach.available());
    assert_eq!(reach.level(), 1);
    assert!(!reach.canlight().is_empty(), "profile plane must publish");
    assert!(
        api::query::ReachQueryView::bit_at(
            &reach.canlight(),
            reach.width(),
            reach.height(),
            reach.base_x(),
            reach.base_z(),
            reach.level(),
            lit,
        ),
        "host packing must crop the shared plane, not omit it"
    );
    let missing = with_script_snapshot_input(
        1,
        Some((3205, 3205, 1)),
        true,
        None,
        Some(&snap),
        None,
        Some(&world),
        None,
        false,
        false,
        false,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        PostedInspect::default(),
        script::isolate_fb::encode_snapshot_with_native,
    );
    let missing = script::isolate_fb::decode_snapshot(&missing).expect("snapshot decodes");
    assert!(
        missing.reach().expect("reach").canlight().is_empty(),
        "missing profile plane posts empty canlight, not all-allowed"
    );
}

#[test]
fn script_snapshot_fb_posts_collision_and_los_identity() {
    use client::dash3d::CollisionFlag;
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 0;
    c.collision[0].add_wall(5, 1, 0, 0, true);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    assert!(snap.scene().available);
    assert_eq!(snap.scene().width, 104);
    assert_eq!(snap.scene().collision_flags.len(), 104 * 104);
    assert_ne!(
        snap.scene().collision_flags[5 * 104 + 1] & CollisionFlag::V_W,
        0
    );

    let (bytes, fp) = script_snapshot_fb(
        None,
        false,
        1,
        Some((3201, 3201, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("snapshot decodes");
    let packed = view.collision().expect("production packer posts collision");
    assert!(packed.available());
    assert_eq!(
        (packed.base_x(), packed.base_z(), packed.level()),
        (3200, 3200, 0)
    );
    assert_eq!((packed.width(), packed.height()), (104, 104));
    let flags = packed.flags();
    assert_eq!(flags.len(), 104 * 104);
    assert_ne!(flags[5 * 104 + 1] & CollisionFlag::V_W, 0);
    assert_eq!(flags[104 + 1], CollisionFlag::_OPEN);

    script::observed::on_reset();
    script::observed::apply(&view);
    let here = api::snapshot::WorldTile {
        x: 3201,
        z: 3201,
        level: 0,
    };
    let open_to = api::snapshot::WorldTile {
        x: 3203,
        z: 3201,
        level: 0,
    };
    let blocked_to = api::snapshot::WorldTile {
        x: 3208,
        z: 3201,
        level: 0,
    };
    assert_eq!(
        script::line_of_sight::query_v2(here, open_to, Some(1)),
        Ok(true)
    );
    assert_eq!(
        script::line_of_sight::query_v2(here, blocked_to, Some(1)),
        Ok(false)
    );
    assert!(script::line_of_sight::query_v1(here, open_to, None));
    assert!(!script::line_of_sight::query_v1(here, blocked_to, None));
    assert_eq!(
        script::line_of_sight::raw_flag_at(104 + 1),
        Some(CollisionFlag::_OPEN)
    );
    assert_eq!(script::line_of_sight::raw_flag_at(-1), None);

    let iso = script::LoadIsolate::spawn(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const c = api.snapshot.collision;
  const here = { x: 3201, z: 3201, level: 0 };
  globalThis.__probe = {
    available: c.available,
    length: c.flags.length,
    atSelf: c.flags.at(1 * c.height + 1),
    atVis: c.flags.at(5 * c.height + 1),
    open: api.lineOfSight({ from: here, to: { x: 3203, z: 3201, level: 0 }, size: 1 }),
    blocked: api.lineOfSight({ from: here, to: { x: 3208, z: 3201, level: 0 }, size: 1 }),
  };
}
"#
        .into(),
        script::LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(value["available"], true);
    assert_eq!(value["length"], serde_json::json!(104 * 104));
    assert_eq!(value["atSelf"], serde_json::json!(CollisionFlag::_OPEN));
    assert_ne!(
        value["atVis"].as_i64().unwrap() & CollisionFlag::V_W as i64,
        0
    );
    assert_eq!(value["open"]["ok"], true);
    assert_eq!(value["open"]["value"], true);
    assert_eq!(value["blocked"]["ok"], true);
    assert_eq!(value["blocked"]["value"], false);
    iso.join();

    let (delta, fp2) = script_snapshot_fb(
        Some(&fp),
        false,
        2,
        Some((3201, 3201, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let delta = script::isolate_fb::decode_snapshot(&delta).expect("delta");
    assert!(
        !delta.has_collision(),
        "unchanged production collision must be omitted"
    );
    assert!(
        std::sync::Arc::ptr_eq(&fp.collision.flags, &fp2.collision.flags),
        "unchanged tick reuses the last flags Arc"
    );

    c.scene_state = 1;
    snap.rebuild(&c);
    assert!(!snap.scene().available);
    let (cleared, _) = script_snapshot_fb(
        Some(&fp2),
        false,
        3,
        Some((3201, 3201, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let cleared = script::isolate_fb::decode_snapshot(&cleared).expect("cleared");
    let gone = cleared.collision().expect("unpublish posts a clear");
    assert!(!gone.available());
    assert!(gone.flags().is_empty());
}

#[test]
fn native_chat_producer_reaches_isolate_with_request_identity() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.local_player = Some(client::dash3d::ClientPlayer::at(0, 0));
    let iso = script::LoadIsolate::spawn(
        r#"export default class T extends LoopingBot {
                onStart() {
                    globalThis.__events = [];
                    globalThis.__requests = 0;
                    this.on('chat.message', (e) => {
                        globalThis.__events.push(e);
                        // A consumer needs all three native fields to distinguish
                        // a partner trade request from the same body elsewhere.
                        if (e.type === 4 && e.username === 'Partner' &&
                            e.text === 'wishes to trade with you.') {
                            globalThis.__requests++;
                        }
                    });
                }
                loop() {}
            }"#
        .into(),
        script::LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    let mut fingerprint = None;
    for (tick, seq, kind, sender, events, requests) in [
        (1, 11, 0, "Partner", 1, 0),
        (2, 12, 4, "Stranger", 2, 0),
        (3, 13, 4, "Partner", 3, 1),
        (4, 13, 4, "Partner", 3, 1),
        (5, 14, 4, "Partner", 4, 2),
    ] {
        c.chat_text[0] = "wishes to trade with you.".into();
        c.chat_type[0] = kind;
        c.chat_username[0] = sender.into();
        c.chat_seq = seq;
        c.bump_gens(ServerProt::MESSAGE_GAME);
        snapshot.rebuild(&c);
        let (bytes, next) = script_snapshot_fb(
            fingerprint.as_ref(),
            false,
            tick,
            Some((3200, 3200, 0)),
            true,
            None,
            Some(&snapshot),
            None,
            None,
            false,
            false,
            false,
        );
        fingerprint = Some(next);
        iso.post_snapshot(bytes);
        iso.on_game_tick(tick);
        assert_eq!(iso.probe("__events.length").unwrap(), events);
        assert_eq!(iso.probe("__requests").unwrap(), requests);
    }
    assert_eq!(
        iso.probe("__events[2]").unwrap(),
        serde_json::json!({
            "type": 4, "username": "Partner", "text": "wishes to trade with you."
        }),
    );
    iso.join();
}

#[test]
fn script_snapshot_posts_current_local_overhead_and_coordinate_hint() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    let mut local = client::dash3d::ClientPlayer::at(0, 0);
    local.entity.chat_message = Some("FIGHT!".into());
    c.local_player = Some(local);
    c.chat_text[0] = "latest ring line".into();
    c.chat_type[0] = 4;
    c.chat_username[0] = "Partner".into();
    c.chat_seq = 11;
    c.hint_type = 2;
    c.hint_tile_x = 2761;
    c.hint_tile_z = 9546;
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::MESSAGE_GAME);
    c.bump_gens(ServerProt::REBUILD_NORMAL);

    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (first_bytes, first_fp) = script_snapshot_fb(
        None,
        false,
        1,
        Some((3200, 3200, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let first = script::isolate_fb::decode_snapshot(&first_bytes).expect("first snapshot");
    assert_eq!(first.self_chat(), Some("FIGHT!"));
    assert_eq!(first.hint_tile(), Some((2761, 9546)));
    assert_eq!(first.chat_text(), Some("latest ring line"));
    let chat = first.chat_lines();
    assert_eq!(chat[0].seq(), 11);
    assert_eq!(chat[0].type_(), 4);
    assert_eq!(chat[0].username(), Some("Partner"));

    c.local_player.as_mut().unwrap().entity.chat_message = None;
    c.hint_type = 0;
    assert!(!snap.rebuild(&c));
    let (second_bytes, _) = script_snapshot_fb(
        Some(&first_fp),
        false,
        2,
        Some((3200, 3200, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let second = script::isolate_fb::decode_snapshot(&second_bytes).expect("second snapshot");
    assert!(second.has_self_chat(), "clear must be present in the delta");
    assert_eq!(second.self_chat(), Some(""));
    assert!(second.has_hint_tile(), "clear must be present in the delta");
    assert_eq!(second.hint_tile(), None);
}

#[test]
fn script_snapshot_posts_attacked_by_player_from_local_face() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.self_slot = 0;
    let mut lp = client::dash3d::ClientPlayer::at(0, 0);
    lp.name = Some("Bot".into());
    lp.entity.face_entity = 32768;
    c.local_player = Some(lp);
    c.bump_gens(ServerProt::PLAYER_INFO);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        3,
        Some((3200, 3200, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
    assert!(view.attacked_by_player(), "player face >= 32768");

    c.local_player.as_mut().unwrap().entity.face_entity = 3;
    c.bump_gens(ServerProt::PLAYER_INFO);
    snap.rebuild(&c);
    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        4,
        Some((3200, 3200, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("npc face decodes");
    assert!(
        !view.attacked_by_player(),
        "NPC face is not attackedByPlayer"
    );
}

/// Tab 0 combat IF (274 unarmed layout) round-trips through the isolate
/// blob as Aggressive, not Punch/Kick — the shim matches the style name.
#[test]
fn script_snapshot_fb_posts_tab0_aggressive_combat_style() {
    use api::snapshot::Family;
    use client::config::if_type::{IfType, IfTypeMut};

    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.set_iface(
        2000,
        IfType {
            id: 2000,
            layer_id: 2000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2001]),
            child_x: Some(vec![4]),
            child_y: Some(vec![60]),
            ..Default::default()
        },
    );
    c.set_iface(
        2001,
        IfType {
            id: 2001,
            layer_id: 2000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2010, 2011, 2012, 2020, 2021, 2022, 2023, 2024, 2025]),
            child_x: Some(vec![5, 5, 5, 78, 78, 78, 78, 78, 78]),
            child_y: Some(vec![5, 51, 97, 5, 51, 97, 18, 64, 110]),
            ..Default::default()
        },
    );
    for (id, mode) in [(2010, 0), (2011, 1), (2012, 2)] {
        c.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: 2001,
                r#type: ComponentType::TYPE_GRAPHIC,
                width: 72,
                height: 36,
                scripts: Some(vec![vec![5, 43, 0]]),
                script_operand: Some(vec![mode]),
                script_comparator: Some(vec![0]),
                ..Default::default()
            },
        );
        c.set_iface_mut(
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_SELECT,
                ..Default::default()
            },
        );
    }
    for (id, text) in [
        (2020, "Punch"),
        (2021, "Kick"),
        (2022, "Block"),
        (2023, "(Accurate)"),
        (2024, "(Aggressive)"),
        (2025, "(Defensive)"),
    ] {
        c.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: 2001,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            id,
            IfTypeMut {
                text: text.into(),
                ..Default::default()
            },
        );
    }
    c.side_icon[0] = 2000;
    c.bump_gens(ServerProt::IF_SETICON);
    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild_family(&c, Family::SideTabs));

    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        1,
        Some((3235, 3295, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
    let labels: Vec<String> = view
        .combat_styles()
        .iter()
        .map(|s| s.label().to_string())
        .collect();
    assert!(
        labels
            .iter()
            .any(|l| l.to_ascii_lowercase().contains("aggressive")),
        "isolate combat_styles must carry the IF style name, got {labels:?}"
    );
    let aggressive = view
        .combat_styles()
        .into_iter()
        .find(|s| s.label().to_ascii_lowercase().contains("aggressive"))
        .expect("Aggressive row");
    assert_eq!(aggressive.mode(), 1);
    assert_eq!(aggressive.component_id(), 2011);
}

/// Hop 2 — an npc behind a wall posts `reachable: false` from SceneQuery.
#[test]
fn script_snapshot_npc_behind_wall_posts_reachable_false() {
    use client::dash3d::{ClientNpc, ClientPlayer, CollisionFlag};

    let mut c = nav_client();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(5, 5));
    c.collision[0].flags[5][6] |= CollisionFlag::SQ_BLOCKED;

    let slot = 0usize;
    let type_id = 501;
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.npcs.len() <= type_id {
            cache.npcs.push(client::config::NpcType::default());
        }
        cache.npcs[type_id] = client::config::NpcType {
            id: type_id as i32,
            name: "Wall guard".to_string(),
            op: vec![Some("Attack".to_string())],
            ..Default::default()
        };
    }
    let mut npc = ClientNpc::at(5, 6);
    npc.r#type = Some(type_id);
    while c.npc.len() <= slot {
        c.npc.push(None);
    }
    c.npc[slot] = Some(Box::new(npc));
    c.npc_ids[0] = slot as i32;
    c.npc_count = 1;

    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert_eq!(snap.npcs().len(), 1, "fixture carries one npc");

    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        1,
        Some((3205, 3205, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
    let npcs = view.npcs();
    assert_eq!(npcs.len(), 1);
    assert!(
        !npcs[0].reachable(),
        "npc behind SQ_BLOCKED tile is not reachable"
    );
    let reach = view.reach().expect("native reach metadata");
    assert_eq!(reach.exact_rank().len(), 104 * 104);
    assert_eq!(reach.adjacent_rank().len(), 104 * 104);
    assert_eq!(reach.exact_rank()[5 * 104 + 5], 0, "origin dequeues first");
    assert_eq!(
        reach.exact_rank()[5 * 104 + 6],
        u16::MAX,
        "blocked npc tile has no exact dequeue rank"
    );
    assert_eq!(
        reach.adjacent_rank()[5 * 104 + 6],
        0,
        "blocked npc tile is adjacent from the origin before expansion"
    );
}

/// A loc on a blocked tile uses the same SceneQuery reach as npcs —
/// never a hardcoded `reachable: false`.
#[test]
fn script_snapshot_loc_behind_wall_posts_reachable_false() {
    use client::config::LocType;
    use client::dash3d::{ClientPlayer, CollisionFlag};

    let mut c = nav_client();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(5, 5));
    c.collision[0].flags[5][6] |= CollisionFlag::SQ_BLOCKED;

    let (blocked_id, here_id) = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let blocked_id = cache.locs.len() as i32;
        cache.locs.push(LocType {
            id: blocked_id,
            name: "Wall booth".into(),
            op: vec![Some("Use-quickly".into()), None],
            ..Default::default()
        });
        let here_id = cache.locs.len() as i32;
        cache.locs.push(LocType {
            id: here_id,
            name: "Open crate".into(),
            op: vec![Some("Search".into()), None],
            ..Default::default()
        });
        (blocked_id, here_id)
    };
    let blocked_typecode = 0x4000_0000 + (blocked_id << 14) + 5 + (6 << 7);
    let here_typecode = 0x4000_0000 + (here_id << 14) + 5 + (5 << 7);
    c.world
        .set_wall(0, 5, 6, 0, 0, 0, blocked_typecode, 0, 0, 0, 0, 0);
    c.world
        .set_wall(0, 5, 5, 0, 0, 0, here_typecode, 0, 0, 0, 0, 0);

    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    assert_eq!(snap.locs().len(), 2, "fixture carries two locs");

    let (bytes, _) = script_snapshot_fb(
        None,
        false,
        1,
        Some((3205, 3205, 0)),
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
    let locs = view.locs();
    assert_eq!(locs.len(), 2);
    let blocked = locs
        .iter()
        .find(|l| l.name() == Some("Wall booth"))
        .expect("blocked loc posted");
    let here = locs
        .iter()
        .find(|l| l.name() == Some("Open crate"))
        .expect("here loc posted");
    assert!(
        !blocked.reachable(),
        "loc on SQ_BLOCKED tile is not reachable"
    );
    assert!(
        here.reachable(),
        "loc on the player's tile must use entity_reach, not a hardcoded false"
    );
}

/// Classic varp 172: 0 is auto-retaliate on. The posted field must
/// match that polarity (not invert it).
#[test]
fn script_snapshot_retaliate_enabled_is_varp_172_zero() {
    let cache = Cache {
        varps: (0..173)
            .map(|_| client::config::VarpType::default())
            .collect(),
        ..Default::default()
    };
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(cache),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.var = vec![0; 173];
    c.var[172] = 0;
    c.bump_gens(client::io::ServerProt::VARP_SYNC);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let on_bytes = script_snapshot_fb(
        None,
        false,
        1,
        None,
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    )
    .0;
    let on = script::isolate_fb::decode_snapshot(&on_bytes).expect("on blob");
    assert!(on.retaliate_enabled(), "varp(172)==0 is auto-retaliate on");

    c.var[172] = 1;
    c.bump_gens(client::io::ServerProt::VARP_SYNC);
    snap.rebuild(&c);
    let off_bytes = script_snapshot_fb(
        None,
        false,
        2,
        None,
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    )
    .0;
    let off = script::isolate_fb::decode_snapshot(&off_bytes).expect("off blob");
    assert!(
        !off.retaliate_enabled(),
        "varp(172)!=0 is auto-retaliate off"
    );
}

fn prayer_overlay_game_snapshot(melee: i32, extra_nonzero: usize) -> GameSnapshot {
    const VARP_LEN: usize = 360;
    let cache = Cache {
        varps: (0..VARP_LEN)
            .map(|_| client::config::VarpType::default())
            .collect(),
        ..Default::default()
    };
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(cache),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.var = vec![0; VARP_LEN];
    for index in 0..api::prayer::PRAYER_COUNT {
        c.var[api::prayer::PRAYER_VARP0 as usize + index] = 0;
    }
    c.var[97] = melee;
    let mut filled = 0;
    for index in 0..VARP_LEN {
        if index == 108 || index == 300 || index == 301 {
            continue;
        }
        if (api::prayer::PRAYER_VARP0 as usize
            ..=api::prayer::PRAYER_VARP0 as usize + api::prayer::PRAYER_COUNT - 1)
            .contains(&index)
        {
            continue;
        }
        if filled < extra_nonzero {
            c.var[index] = 2;
            filled += 1;
        }
    }
    c.stat_base_level[5] = 43;
    c.stat_effective_level[5] = 43;
    c.bump_gens(client::io::ServerProt::VARP_SYNC);
    c.bump_gens(client::io::ServerProt::UPDATE_STAT);
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&c);
    snapshot
}

fn publish_script_snapshot(
    last: Option<&script::isolate_fb::SnapshotFingerprint>,
    tick: u64,
    snapshot: &GameSnapshot,
) -> (Vec<u8>, script::isolate_fb::SnapshotFingerprint) {
    script_snapshot_fb(
        last,
        false,
        tick,
        Some((3220, 3220, 0)),
        true,
        None,
        Some(snapshot),
        None,
        None,
        false,
        false,
        false,
    )
}

fn posted_varp(view: &script::isolate_fb::SnapshotReader<'_>, index: i32) -> Option<i32> {
    view.varps()
        .iter()
        .find(|row| row.index() == index)
        .map(|row| row.value())
}

/// Production `script_snapshot_fb` → FlatBuffer → helper must carry the
/// selected 15 prayer overlays including 0, and every other nonzero varp,
/// however many there are.
#[test]
fn script_snapshot_posts_prayer_overlay_zeros_through_isolate_under_extra_pressure() {
    let extras = 40;
    let off_snap = prayer_overlay_game_snapshot(0, extras);
    let on_snap = prayer_overlay_game_snapshot(1, extras);
    let (off_bytes, off_fp) = publish_script_snapshot(None, 1, &off_snap);
    let off_view = script::isolate_fb::decode_snapshot(&off_bytes).expect("off keyframe");
    for index in
        api::prayer::PRAYER_VARP0..api::prayer::PRAYER_VARP0 + api::prayer::PRAYER_COUNT as i32
    {
        assert_eq!(
            posted_varp(&off_view, index),
            Some(0),
            "selected prayer {index} including 0 must be on the wire"
        );
    }
    assert_eq!(posted_varp(&off_view, 108), Some(0));
    assert_eq!(posted_varp(&off_view, 300), Some(0));
    assert_eq!(posted_varp(&off_view, 301), Some(0));
    let extra_rows = off_view
        .varps()
        .iter()
        .filter(|row| {
            let index = row.index();
            row.value() != 0
                && index != 108
                && index != 300
                && index != 301
                && !(api::prayer::PRAYER_VARP0
                    ..api::prayer::PRAYER_VARP0 + api::prayer::PRAYER_COUNT as i32)
                    .contains(&index)
        })
        .count();
    assert_eq!(
        extra_rows, extras,
        "every non-prayer nonzero varp is posted, not a capped subset"
    );

    let game_data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let iso = script::LoadIsolate::spawn_with_game_data(
        r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__active = api.prayerActive({ name: 'Protect from Melee' }).value;
}
"#
        .into(),
        script::LoadShape::NativeTick,
        vec![],
        game_data,
    )
    .unwrap();
    iso.post_snapshot(off_bytes);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__active").unwrap(), false);

    let (on_bytes, on_fp) = publish_script_snapshot(Some(&off_fp), 2, &on_snap);
    let on_view = script::isolate_fb::decode_snapshot(&on_bytes).expect("on delta");
    assert_eq!(
        posted_varp(&on_view, 97),
        Some(1),
        "97=1 must not lose the reserved overlay slot to extra pressure"
    );
    iso.post_snapshot(on_bytes);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("globalThis.__active").unwrap(), true);

    let (held_bytes, held_fp) = publish_script_snapshot(Some(&on_fp), 3, &on_snap);
    let held = script::isolate_fb::decode_snapshot(&held_bytes).expect("unchanged delta");
    assert!(
        !held.has_varps(),
        "unchanged varps stay omitted; helper must retain the last overlay"
    );
    iso.post_snapshot(held_bytes);
    iso.on_game_tick(3);
    assert_eq!(iso.probe("globalThis.__active").unwrap(), true);

    let (back_bytes, _) = publish_script_snapshot(Some(&held_fp), 4, &off_snap);
    let back = script::isolate_fb::decode_snapshot(&back_bytes).expect("off delta");
    assert_eq!(
        posted_varp(&back, 97),
        Some(0),
        "host 97=0 must be published; omitting it leaves the helper ON"
    );
    iso.post_snapshot(back_bytes);
    iso.on_game_tick(4);
    assert_eq!(
        iso.probe("globalThis.__active").unwrap(),
        false,
        "0->1->0 through production publication must observe off"
    );
    iso.join();
}

#[test]
fn script_snapshot_posts_native_quest_rows_and_clears_them_without_a_snapshot() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.set_iface(
        2200,
        IfType {
            id: 2200,
            layer_id: 2200,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2201, 2202, 2203, 2204]),
            ..Default::default()
        },
    );
    for id in 2201..=2204 {
        c.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: 2200,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
    }
    for (id, text, colour) in [
        (2201, "Quest Journal", 0xFFFF00),
        (2202, "Waterfall Quest", 0xF80000),
        (2203, "Lost City", 0xF8F800),
        (2204, "Dragon Slayer", 0x00F800),
    ] {
        c.set_iface_mut(
            id,
            IfTypeMut {
                text: text.into(),
                colour,
                ..Default::default()
            },
        );
    }
    c.side_icon[2] = 2200;
    c.bump_gens(ServerProt::IF_SETTEXT);

    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (bytes, fingerprint) = script_snapshot_fb(
        None,
        false,
        1,
        None,
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    );
    let posted = script::isolate_fb::decode_snapshot(&bytes).expect("snapshot decodes");
    assert!(posted.has_quest_statuses_update());
    assert!(posted.quest_statuses_available());
    let rows = posted.quest_statuses();
    let got = rows
        .iter()
        .map(|row| (row.name(), row.status()))
        .collect::<Vec<_>>();
    assert_eq!(
        got,
        vec![
            ("Quest Journal", "unknown"),
            ("Waterfall Quest", "notStarted"),
            ("Lost City", "inProgress"),
            ("Dragon Slayer", "complete"),
        ]
    );

    let (clear_bytes, _) = script_snapshot_fb(
        Some(&fingerprint),
        false,
        2,
        None,
        false,
        None,
        None,
        None,
        None,
        false,
        false,
        false,
    );
    let clear = script::isolate_fb::decode_snapshot(&clear_bytes).expect("clear decodes");
    assert!(
        clear.has_quest_statuses_update(),
        "clear update must be present"
    );
    assert!(!clear.quest_statuses_available());
    assert!(!clear.has_quest_statuses());
    assert!(clear.quest_statuses().is_empty());
}

#[test]
fn script_snapshot_posts_native_retaliate_control_identity() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.set_iface(
        100,
        IfType {
            id: 100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![101, 102, 103, 104, 105, 106]),
            ..Default::default()
        },
    );
    for id in 101..=106 {
        c.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: 100,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
    }
    c.set_iface_mut(
        101,
        IfTypeMut {
            text: "Auto retaliate".into(),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_SETTEXT);

    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let bytes = script_snapshot_fb(
        None,
        false,
        1,
        None,
        true,
        None,
        Some(&snap),
        None,
        None,
        false,
        false,
        false,
    )
    .0;
    let posted = script::isolate_fb::decode_snapshot(&bytes).expect("snapshot decodes");
    assert_eq!(posted.retaliate_controls(), Some((103, 104)));
}

// Task 9c — delta posts: the keyframe carries every field; a later
// post carries only the fields that changed (plus tick), so a 50+
// isolate wall never resends unchanged inv/bank/stats/booths/packed
// banks. The returned fingerprint is the per-slot last-post state the
// observe stores; banks are re-included on `force_banks` even when the
// stand list did not change (NavWorld identity change).
#[test]
fn script_snapshot_fb_posts_only_changed_tables() {
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.stat_effective_level[7] = 40;
    c.stat_xp[7] = 1300;
    c.bump_gens(ServerProt::UPDATE_STAT);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let mut objs = vec![client::config::ObjType::default(); 2];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let inv = vec![(1, 2)];
    let world = Some(Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 2,
            height: 1,
            walk: vec![0u8; 2],
            blocked: vec![0u64; 2usize.div_ceil(64)],
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        vec![nav::pack::BankStand {
            name: "Bank booth".into(),
            tile: WorldTile {
                x: 1,
                z: 0,
                level: 0,
            },
            access: nav::pack::BankAccess::Booth { op: 2 },
        }],
    )));

    // Keyframe (no last post): every observed field is present.
    let (keyframe, fp1) = script_snapshot_fb(
        None,
        false,
        7,
        Some((3200, 3200, 0)),
        true,
        Some(&inv),
        Some(&snap),
        Some(&names),
        world.as_deref(),
        true,
        false,
        false,
    );
    let kf = script::isolate_fb::decode_snapshot(&keyframe).expect("keyframe decodes");
    assert!(kf.has_here());
    assert!(kf.has_ingame());
    assert!(kf.has_inv());
    assert!(kf.has_stats());
    assert!(kf.has_banks(), "keyframe carries the packed banks");

    // Same observed data again: only tick is carried.
    let (delta, fp2) = script_snapshot_fb(
        Some(&fp1),
        false,
        8,
        Some((3200, 3200, 0)),
        true,
        Some(&inv),
        Some(&snap),
        Some(&names),
        world.as_deref(),
        true,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
    assert_eq!(view.tick(), 8, "tick is always present");
    assert!(!view.has_here(), "unchanged here omitted");
    assert!(!view.has_ingame(), "unchanged ingame omitted");
    assert!(!view.has_inv(), "unchanged inv omitted");
    assert!(!view.has_stats(), "unchanged stats omitted");
    assert!(!view.has_banks(), "unchanged packed banks omitted");
    assert!(!view.has_bank());
    assert!(!view.has_bank_side());
    assert!(view.has_hold(), "hold is re-posted every tick (SEC-004)");
    assert!(view.hold(), "unchanged hold still true");

    // inv changed -> the inv table comes back; banks stay omitted even
    // when force_banks is false.
    let inv2 = vec![(1, 2), (99, 5)];
    let (delta, _fp3) = script_snapshot_fb(
        Some(&fp2),
        false,
        9,
        Some((3200, 3200, 0)),
        true,
        Some(&inv2),
        Some(&snap),
        Some(&names),
        world.as_deref(),
        true,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
    assert!(view.has_inv(), "changed inv is carried");
    assert_eq!(view.inv().len(), 2);
    assert!(!view.has_banks(), "unchanged banks still omitted");

    // NavWorld identity change (force_banks): the packed banks are
    // re-posted even though the stand list is unchanged.
    let (delta, _) = script_snapshot_fb(
        Some(&fp2),
        true,
        10,
        Some((3200, 3200, 0)),
        true,
        Some(&inv),
        Some(&snap),
        Some(&names),
        world.as_deref(),
        true,
        false,
        false,
    );
    let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
    assert!(view.has_banks(), "force_banks re-posts the packed banks");
    assert!(!view.has_inv(), "unchanged inv stays omitted");
}

#[test]
fn inventory_from_ifaces_maps_1_based_ids_to_0_based() {
    // The TYPE_INV iface stores `obj_id + 1` (0 = empty slot); scripts
    // resolve `has_item` against the 0-based ObjNames table, so the
    // view must carry `id - 1` and drop the empties.
    let mut ifaces = vec![None; 3];
    ifaces[1] = Some(Box::new(IfType {
        r#type: ComponentType::TYPE_INV,
        obj_ops: true,
        ..Default::default()
    }));
    let mut ifaces_mut = vec![None; 3];
    ifaces_mut[1] = Some(Arc::new(IfTypeMut {
        link_obj_type: Some(vec![2, 0, 1]),
        link_obj_number: Some(vec![3, 0, 1]),
        ..Default::default()
    }));
    let mut client = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(ifaces),
        ifaces_mut,
    );
    // The inv tab (side 3) binds this root; a TYPE_INV that is not the
    // backpack (no `obj_ops`, or under another side tab) must not
    // satisfy the read.
    client.side_icon[3] = 1;
    let inv = inventory_from_ifaces(&client).expect("TYPE_INV iface present");
    assert_eq!(
        inv,
        vec![(1, 3), (0, 1)],
        "1-based ids map down by one and empty slots drop"
    );

    // End-to-end: the mapped id-0 slot must resolve via has_item.
    let mut objs = vec![client::config::ObjType::default(); 1];
    objs[0].id = 0;
    objs[0].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let mut rec = NavRec {
        walked: None,
        held_ops: 0,
        if_button_components: Vec::new(),
        sink: Sink,
    };
    let ctx = ScriptCtx {
        driver: &mut rec,
        tick: 0,
        here: None,
        walk: None,
        walk_with: None,
        inv: Some(&inv),
        snapshot: None,
        obj_names: Some(&names),
        compiled: script::CompiledTick::default(),
    };
    assert!(ctx.has_item("Bones"));
    assert!(!ctx.has_item("Vial"));
}

#[test]
fn observe_script_inv_reads_only_the_current_snapshot_on_running_ticks() {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: String::new(),
        members: true,
        lowmem: true,
    });
    client.ingame = true;
    let mut snap = GameSnapshot::new();
    client.gens.inv += 1;
    snap.rebuild(&client);
    assert!(observe_script_inv(true, false, &snap).is_none());
    assert!(observe_script_inv(false, true, &snap).is_none());
    assert_eq!(observe_script_inv(true, true, &snap), Some(snap.inv()));
    snap.reset_session(client.gens);
    assert_eq!(observe_script_inv(true, true, &snap), Some(&[][..]));
}

// --- Task 5: guardian hold + knock plumbing over `host::Guardian` ---

/// Recording driver for the guardian tests: captures every
/// menu/action/try_move send (the host crate's fake driver shape;
/// `walks` is the flee/ground-walk trace).
#[derive(Default)]
struct GuardRec {
    menus: Vec<(i32, i32, i32, i32, i32)>,
    actions: Vec<i32>,
    walks: Vec<(i32, i32)>,
    sink: Sink,
}

impl Driver for GuardRec {
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
        self.menus.push((slot, action, a, b, c));
    }
    fn do_action(&mut self, slot: i32) -> bool {
        self.actions.push(slot);
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        dx: i32,
        dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _t: i32,
    ) -> bool {
        self.walks.push((dx, dz));
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        // (0,0) with build_base (0,0): absolute world tiles equal the
        // recorded `try_move` targets (the flee/ground walks land).
        Some((0, 0))
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.sink
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

/// An attached ingame scene-2 client with a named local player at
/// world (0,0) (base 0), ready for the guardian's detect to see.
fn guardian_client() -> Client {
    let mut c = nav_client();
    c.map_build_base_x = 0;
    c.map_build_base_z = 0;
    c.self_slot = 0;
    let mut lp = client::dash3d::ClientPlayer::at(0, 0);
    lp.name = Some("Test".to_string());
    c.local_player = Some(lp);
    c
}

/// Plant NPC `name` in client table slot `slot` with an overhead line
/// (the snapshot `NpcView` the guardian's detect reads).
fn plant_npc(c: &mut Client, slot: usize, name: &str, overhead: Option<&str>) {
    plant_npc_with_face(c, slot, name, -1, overhead);
}

/// Plant NPC `name` in client table slot `slot` facing the local
/// player (`face_entity` >= 32768 decodes as Player kind; + self_slot
/// 0 targets us — the host-owned evade shape).
fn plant_attacking_npc(c: &mut Client, slot: usize, name: &str) {
    plant_npc_with_face(c, slot, name, 32768, None);
}

fn plant_npc_with_face(
    c: &mut Client,
    slot: usize,
    name: &str,
    face_entity: i32,
    overhead: Option<&str>,
) {
    let type_id = 500 + slot;
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.npcs.len() <= type_id {
            cache.npcs.push(client::config::NpcType::default());
        }
        cache.npcs[type_id] = client::config::NpcType {
            id: type_id as i32,
            name: name.to_string(),
            op: vec![Some("Talk-to".to_string())],
            ..Default::default()
        };
    }
    let mut npc = client::dash3d::ClientNpc::at(0, 0);
    npc.r#type = Some(type_id);
    npc.entity.face_entity = face_entity;
    npc.entity.chat_message = overhead.map(str::to_string);
    while c.npc.len() <= slot {
        c.npc.push(None);
    }
    c.npc[slot] = Some(Box::new(npc));
    c.npc_ids[c.npc_count as usize] = slot as i32;
    c.npc_count += 1;
}

/// Active positive type-1 combat hit on the local player (value 5, type 1).
fn plant_positive_hit(c: &mut Client) {
    let until = c.loop_cycle + 70;
    let lp = c.local_player.as_mut().expect("local player");
    lp.entity.damage_values[0] = 5;
    lp.entity.damage_types[0] = 1;
    lp.entity.damage_cycles[0] = until;
}

/// Advance every packet family and rebuild the **persistent**
/// snapshot (one call per game tick, so `snap.tick()` climbs).
fn tick_at(c: &mut Client, snap: &mut GameSnapshot) {
    c.gens.npc = c.gens.npc.wrapping_add(1);
    c.gens.player = c.gens.player.wrapping_add(1);
    c.gens.inv = c.gens.inv.wrapping_add(1);
    c.gens.scene = c.gens.scene.wrapping_add(1);
    c.gens.iface = c.gens.iface.wrapping_add(1);
    c.gens.chat = c.gens.chat.wrapping_add(1);
    snap.rebuild(c);
}

/// A guardian hold still posts the blob and dispatches the isolate
/// tick (onPaint only); compiled scripts stay frozen. The unheld edge
/// dispatches fully.
#[test]
fn script_observe_skips_the_tick_dispatch_while_hold() {
    let ScriptWiring {
        scripts,
        cheats,
        count,
    } = script_wiring();
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    // Up + edge but held by the guardian: compiled tick stays frozen.
    script_observe(
        &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, true, false,
    );
    assert_eq!(
        *count.lock().unwrap(),
        0,
        "hold freezes compiled on_game_tick"
    );
    // The same edge unheld dispatches.
    script_observe(
        &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert_eq!(*count.lock().unwrap(), 1, "an unheld edge dispatches");
}

#[test]
fn welcome_hold_skips_compiled_tick_until_observed_close() {
    let ScriptWiring {
        scripts,
        cheats,
        count,
    } = script_wiring();
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let mut welcome = login_readiness::LoginReadiness::default();
    welcome.on_session_boundary();
    let open = login_readiness::WelcomeObservation {
        session_epoch: welcome.session_epoch(),
        tick: 1,
        now: Instant::now(),
        ingame: true,
        scene_state: 2,
        welcome_interface_id: 42,
        main_modal_id: 42,
        allow_close: true,
    };
    let step = welcome.step(&open, || login_readiness::CloseAttempt::Sent);
    assert!(step.hold);
    script_observe(
        &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, step.hold, false,
    );
    assert_eq!(
        *count.lock().unwrap(),
        0,
        "welcome hold blocks script work before ack"
    );
    let closed = login_readiness::WelcomeObservation {
        main_modal_id: -1,
        tick: 2,
        ..open
    };
    let step = welcome.step(&closed, || panic!("no close after ack"));
    assert!(!step.hold);
    script_observe(
        &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, step.hold, false,
    );
    assert_eq!(
        *count.lock().unwrap(),
        1,
        "script work resumes only after observed close"
    );
}

#[test]
fn script_observe_posts_blob_while_held_and_skips_dispatch() {
    // Fix-round: the snapshot blob must post on the held tick edge, and
    // the isolate tick still dispatches for onPaint (loop frozen inside
    // V8). Compiled-path skip is covered by
    // `script_observe_skips_the_tick_dispatch_while_hold`.
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src =
            "export default class T extends LoopingBot { loop() { globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1; } }";
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    // Held edge: blob posts + isolate tick (paint-only); loop must not
    // advance.
    assert!(script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3200, 3200, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        true,
        false
    ));
    let loops = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__rs_loops || 0")
        .unwrap();
    assert_eq!(loops, 0, "held isolate tick must not run loop()");
    // Unheld edge: the same slot dispatches a full tick (loop runs).
    assert!(script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3200, 3200, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false
    ));
    let loops = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__rs_loops || 0")
        .unwrap();
    assert_eq!(loops, 1, "unheld isolate tick runs loop()");
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

fn force_watchdog_sampling(slot: &mut script::SlotScript, here: (i32, i32, i32)) {
    let t = Instant::now();
    slot.feed_watchdog(t, Some(here), &[], false, true, &[]);
    assert_eq!(
        slot.feed_watchdog(
            t + script::watchdog::WEDGE,
            Some(here),
            &[],
            false,
            true,
            &[]
        ),
        script::WatchdogAction::RequestAnchor
    );
    assert!(matches!(
        slot.watchdog().state(),
        script::WatchdogState::SamplingAnchor
    ));
    assert!(slot.watchdog().holds_script_actions());
}

fn force_watchdog_recovering(slot: &mut script::SlotScript, here: (i32, i32, i32)) {
    force_watchdog_sampling(slot, here);
    assert!(matches!(
        slot.feed_watchdog(
            Instant::now(),
            Some(here),
            &[],
            false,
            true,
            &[script::shim::InteractReq::RecoveryAnchor {
                x: 3200,
                z: 3200,
                level: 0
            }]
        ),
        script::WatchdogAction::ArmWalk { .. }
    ));
    assert!(slot.watchdog().recovering_anchor().is_some());
}

fn finish_watchdog_recovery(slot: &mut script::SlotScript, anchor: (i32, i32, i32)) {
    slot.feed_watchdog(Instant::now(), Some(anchor), &[], false, true, &[]);
    assert!(
        slot.watchdog().recovering_anchor().is_none(),
        "near-anchor tile must exit Recovering"
    );
    assert!(!slot.watchdog().holds_script_actions());
}

#[test]
fn recovery_hold_drops_script_actions_and_keeps_paint() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = r#"
export default class T extends LoopingBot {
    onStart() { globalThis.__starts = (globalThis.__starts || 0) + 1; }
    onPaint() { globalThis.__paints = (globalThis.__paints || 0) + 1; }
    loop() {
        globalThis.__loops = (globalThis.__loops || 0) + 1;
        globalThis.__rs2b0t_host.interact = globalThis.__rs2b0t_host.interact || [];
        globalThis.__rs2b0t_host.interact.push({ op: 'set-camera-yaw', yaw: 1234 });
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    c.ingame = true;
    c.scene_state = 2;
    // Stats loaded: a compat card paints only then (rs2b0t paintBot).
    c.stat_base_level.fill(1);
    c.bump_gens(ServerProt::UPDATE_STAT);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let observe = |c: &mut Client, tick_edge: bool, tick: u64| {
        script_observe(
            c,
            "alice",
            true,
            tick_edge,
            tick,
            Some((100, 100, 0)),
            None,
            None,
            Some(&snap),
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        );
    };
    observe(&mut c, true, 1);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    observe(&mut c, false, 1);
    assert_eq!(
        c.orbit_camera_yaw, 1234,
        "ordinary dispatch writes camera yaw"
    );

    force_watchdog_recovering(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            route_worker: Some(Arc::new(())),
            ..NavBot::default()
        },
    );
    c.orbit_camera_yaw = 0;
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    let loops_before = slot
        .probe("globalThis.__loops || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    let starts_before = slot
        .probe("globalThis.__starts || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    let paints_before = slot
        .probe("globalThis.__paints || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    drop(slot);
    assert_eq!(loops_before, 1, "baseline loop ran once before recovery");
    assert_eq!(starts_before, 1, "onStart ran once before recovery");

    for tick in 2..=4 {
        observe(&mut c, true, tick);
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("true")
            .unwrap();
        observe(&mut c, false, tick);
    }
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    let loops = slot
        .probe("globalThis.__loops || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    let starts = slot
        .probe("globalThis.__starts || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    let paints = slot
        .probe("globalThis.__paints || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    assert!(
        slot.watchdog().recovering_anchor().is_some(),
        "internal recovery hold must not cancel owned Recovering"
    );
    drop(slot);
    assert_eq!(
        c.orbit_camera_yaw, 0,
        "recovery must not dispatch ordinary script actions"
    );
    assert_eq!(
        loops, loops_before,
        "recovery hold must freeze loop(), not merely drop actions"
    );
    assert_eq!(starts, starts_before, "recovery must not re-run onStart");
    assert!(
        paints > paints_before,
        "onPaint continues during recovery: before={paints_before} after={paints}"
    );

    finish_watchdog_recovery(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (3200, 3200, 0),
    );
    observe(&mut c, true, 5);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    observe(&mut c, false, 5);
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    let loops_after = slot
        .probe("globalThis.__loops || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    let starts_after = slot
        .probe("globalThis.__starts || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    drop(slot);
    assert_eq!(
        loops_after,
        loops_before + 1,
        "loop resumes exactly once after recovery hold lifts"
    );
    assert_eq!(starts_after, 1, "release must not re-register onStart");
    assert_eq!(
        c.orbit_camera_yaw, 1234,
        "ordinary actions dispatch again after release, not replayed from hold"
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn recovery_hold_freezes_parked_wait_and_resumes_once() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    onStart() { globalThis.__starts = (globalThis.__starts || 0) + 1; }
    onPaint() { globalThis.__paints = (globalThis.__paints || 0) + 1; }
    async loop() {
        globalThis.__loops = (globalThis.__loops || 0) + 1;
        if (globalThis.__loops === 1) {
            await Execution.delayUntil(() => Game.tick() >= 4, 60000);
            globalThis.__settled = 1;
        }
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    c.ingame = true;
    c.scene_state = 2;
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let observe = |c: &mut Client, tick_edge: bool, tick: u64| {
        script_observe(
            c,
            "alice",
            true,
            tick_edge,
            tick,
            Some((100, 100, 0)),
            None,
            None,
            Some(&snap),
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        );
    };
    observe(&mut c, true, 1);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert_eq!(
        slot.probe("globalThis.__loops || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        1,
        "first loop parks"
    );
    assert_eq!(
        slot.probe("globalThis.__settled || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        0,
        "wait must still be parked"
    );
    drop(slot);

    force_watchdog_recovering(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            route_worker: Some(Arc::new(())),
            ..NavBot::default()
        },
    );
    for tick in 2..=4 {
        observe(&mut c, true, tick);
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("true")
            .unwrap();
    }
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert!(
        slot.watchdog().recovering_anchor().is_some(),
        "parked-wait ticks must stay Recovering: {:?}",
        slot.watchdog().state()
    );
    assert_eq!(
        slot.probe("globalThis.__rs2b0t_host.hold").unwrap(),
        true,
        "recovery must post isolate host_hold"
    );
    assert_eq!(
        slot.probe("globalThis.__loops || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        1,
        "recovery hold must not re-enter loop"
    );
    assert_eq!(
        slot.probe("globalThis.__settled || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        0,
        "parked Execution wait must not settle during recovery hold"
    );
    assert_eq!(
        slot.probe("globalThis.__starts || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        1
    );
    drop(slot);

    finish_watchdog_recovery(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (3200, 3200, 0),
    );
    observe(&mut c, true, 5);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert_eq!(
        slot.probe("globalThis.__settled || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        1,
        "wait settles after recovery hold lifts"
    );
    assert_eq!(
        slot.probe("globalThis.__loops || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        1,
        "settling the parked wait must not re-enter loop on the same tick"
    );
    drop(slot);
    observe(&mut c, true, 6);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert_eq!(
        slot.probe("globalThis.__loops || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        2,
        "loop resumes exactly once after the parked wait"
    );
    assert_eq!(
        slot.probe("globalThis.__starts || 0")
            .unwrap()
            .as_i64()
            .unwrap(),
        1,
        "release must not duplicate onStart"
    );
    drop(slot);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn sampling_anchor_hold_still_returns_async_anchor() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = r#"
export default class T extends LoopingBot {
    onPaint() { globalThis.__paints = (globalThis.__paints || 0) + 1; }
    recoveryAnchor() {
        globalThis.__sampled = (globalThis.__sampled || 0) + 1;
        return { x: 3200, z: 3200, level: 0 };
    }
    loop() { globalThis.__loops = (globalThis.__loops || 0) + 1; }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    c.ingame = true;
    c.scene_state = 2;
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let observe = |c: &mut Client, tick_edge: bool, tick: u64, here: (i32, i32, i32)| {
        script_observe(
            c,
            "alice",
            true,
            tick_edge,
            tick,
            Some(here),
            None,
            None,
            Some(&snap),
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        );
    };
    observe(&mut c, true, 1, (100, 100, 0));
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    force_watchdog_sampling(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    let loops_before = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__loops || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    observe(&mut c, true, 2, (100, 100, 0));
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .request_recovery_anchor();
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    observe(&mut c, true, 3, (100, 100, 0));
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    slot.probe("true").unwrap();
    let loops = slot
        .probe("globalThis.__loops || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    let sampled = slot
        .probe("globalThis.__sampled || 0")
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(
        loops, loops_before,
        "SamplingAnchor hold must freeze loop()"
    );
    assert_eq!(
        sampled, 1,
        "asynchronous recoveryAnchor must still run while recovery-held"
    );
    assert!(
        slot.watchdog().recovering_anchor().is_some(),
        "anchor reply must enter Recovering, not stick in SamplingAnchor: {:?}",
        slot.watchdog().state()
    );
    drop(slot);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn guardian_hold_aborts_recovery_unlike_internal_hold() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = "export default class T extends LoopingBot { loop() {} }";
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    c.ingame = true;
    c.scene_state = 2;
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((100, 100, 0)),
        None,
        None,
        Some(&snap),
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    force_watchdog_recovering(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            route_worker: Some(Arc::new(())),
            ..NavBot::default()
        },
    );
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((100, 100, 0)),
        None,
        None,
        Some(&snap),
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .watchdog()
            .recovering_anchor()
            .is_some(),
        "internal recovery hold must keep Recovering"
    );
    assert!(
        navs.lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .route_worker
            .is_some(),
        "owned recovery nav continues during internal hold"
    );
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        3,
        Some((100, 100, 0)),
        None,
        None,
        Some(&snap),
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        true,
        false,
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    assert!(
        slot.watchdog().recovering_anchor().is_none(),
        "external guardian hold must defer/abort owned recovery"
    );
    assert!(
        !matches!(
            slot.watchdog().state(),
            script::WatchdogState::RestartPending { .. }
        ),
        "guardian hold must not restart: {:?}",
        slot.watchdog().state()
    );
    drop(slot);
    assert!(
        navs.lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .route_worker
            .is_none(),
        "guardian hold aborts recovery nav"
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn pause_clears_live_recovery_walk() {
    let mut play = run_with_io(
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
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    play.script_start_load(
        "alice",
        "export default class T extends LoopingBot { loop() {} }".into(),
        script::LoadShape::CompatClass,
        None,
        vec![],
    )
    .unwrap();
    force_watchdog_recovering(
        &mut script_slot(&play.scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    play.navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            route_worker: Some(Arc::new(())),
            ..NavBot::default()
        },
    );
    play.script_pause("alice");
    assert_eq!(play.script_state("alice"), script::RunState::Paused);
    let navs = play.navs.lock().unwrap();
    let bot = navs.get("alice").expect("nav bot");
    assert!(bot.route_worker.is_none(), "Pause must abort recovery nav");
    assert!(bot.route.is_none());
    assert!(bot.pending_route.is_none());
    drop(navs);
    play.script_stop("alice");
}

#[test]
fn session_reset_clears_live_recovery_walk() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, _world) = empty_nav();
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(
            "export default class T extends LoopingBot { loop() {} }".into(),
            script::LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
    force_watchdog_recovering(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            route_worker: Some(Arc::new(())),
            ..NavBot::default()
        },
    );
    reset_slot_session_work("alice", &scripts, &cheats, &wires, &navs);
    assert!(script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .watchdog()
        .recovering_anchor()
        .is_none());
    let bot = navs.lock().unwrap();
    let bot = bot.get("alice").unwrap();
    assert!(bot.route_worker.is_none());
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn watchdog_restart_does_not_override_not_ready_freeze() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = r#"
export default class T extends LoopingBot {
    onStart() { globalThis.__alive = 1; }
    loop() { globalThis.__alive = 1; }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .unwrap();
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((100, 100, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("true")
        .unwrap();
    force_watchdog_recovering(
        &mut script_slot(&scripts, "alice").unwrap().lock().unwrap(),
        (100, 100, 0),
    );
    // !ready (no here): freeze first; idle recovery must not recreate.
    script_observe(
        &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    let slot = slot.lock().unwrap();
    let alive = slot
        .probe("globalThis.__alive || 0")
        .expect("isolate must not have been recreated");
    assert_eq!(alive, 1, "restart must not override !ready freeze");
    assert!(
        !matches!(
            slot.watchdog().state(),
            script::WatchdogState::RestartPending { .. }
        ),
        "frozen fail branch must not pend restart: {:?}",
        slot.watchdog().state()
    );
    drop(slot);
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn identical_script_paint_skips_status_clone() {
    use script::shim::ScriptPaint;

    let frame = ScriptPaint {
        title: Some("probe".into()),
        accent: None,
        lines: vec!["same".into()],
        buttons: Vec::new(),
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    };
    let mut status = SlotStatus {
        username: "alice".into(),
        script_paint: Some(std::sync::Arc::new(frame.clone())),
        ..SlotStatus::default()
    };
    let lines_ptr = status.script_paint.as_ref().unwrap().lines.as_ptr();

    let shared = std::sync::Arc::clone(status.script_paint.as_ref().unwrap());
    publish_script_paint(&mut status, Some(shared));
    assert_eq!(
        status.script_paint.as_ref().unwrap().lines.as_ptr(),
        lines_ptr,
        "identical paint must not replace status.script_paint"
    );

    let changed = ScriptPaint {
        title: Some("probe".into()),
        accent: None,
        lines: vec!["different".into()],
        buttons: Vec::new(),
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    };
    publish_script_paint(&mut status, Some(std::sync::Arc::new(changed)));
    assert_ne!(
        status.script_paint.as_ref().unwrap().lines.as_ptr(),
        lines_ptr,
        "changed lines must publish a new frame"
    );
    assert_eq!(status.script_paint.as_ref().unwrap().lines[0], "different");

    let relabel = ScriptPaint {
        title: Some("probe".into()),
        accent: None,
        lines: vec!["different".into()],
        buttons: vec![script::shim::ScriptPaintButton {
            id: "gobank".into(),
            label: "Resume".into(),
        }],
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    };
    publish_script_paint(&mut status, Some(std::sync::Arc::new(relabel)));
    assert_eq!(
        status.script_paint.as_ref().unwrap().buttons[0].label,
        "Resume",
        "button label toggle must publish"
    );
}

#[test]
fn isolate_identical_paint_skips_status_clone_on_two_ticks() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    cheats
        .lock()
        .unwrap()
        .insert("alice".into(), VecDeque::new());
    let (navs, world) = empty_nav();
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    loop() {}
    onPaint() {
        const p = Paint.begin();
        p.row('same');
        p.end();
    }
}
"#;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    let mut status = SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    };
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3200, 3200, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        true,
        false,
    );
    assert!(
        wait_until(500, || script_paint_of(&scripts, "alice").is_some()),
        "first tick paints"
    );
    publish_script_paint(&mut status, script_paint_of(&scripts, "alice"));
    let lines_ptr = status
        .script_paint
        .as_ref()
        .expect("first tick paints")
        .lines
        .as_ptr();
    script_observe(
        &mut c,
        "alice",
        true,
        true,
        2,
        Some((3200, 3200, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        true,
        false,
    );
    assert!(
        wait_until(500, || script_paint_of(&scripts, "alice").is_some()),
        "second tick still has paint"
    );
    publish_script_paint(&mut status, script_paint_of(&scripts, "alice"));
    assert_eq!(
        status.script_paint.as_ref().unwrap().lines.as_ptr(),
        lines_ptr,
        "second identical tick must not clone paint onto status"
    );
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn hold_freezes_follow_and_keeps_the_armed_route() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        statuses,
        ..
    } = nav_rig();
    let mut d = NavRec::default();
    let mut c = nav_client();
    assert!(script_observe(
        &mut d,
        "alice",
        true,
        true,
        1,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    ));
    assert!(
        wait_until(100, || queued(&navs).is_some()),
        "the worker armed the route"
    );

    // Held: no follow step, and the armed route is not consumed.
    let mut snap = GameSnapshot::new();
    nav_snapshot_at(&mut c, &mut snap, 0, 0);
    step_nav_bot(
        &mut d,
        "alice",
        Some((0, 0, 0)),
        &snap,
        &navs,
        &statuses,
        world.as_deref(),
        true,
        false,
        no_reach,
    );
    assert_eq!(d.walked, None, "hold freezes the follow");
    assert!(
        queued(&navs).is_some(),
        "the route stays latched under hold"
    );

    // Hold lifted: the next pump step resumes the latched route.
    step_nav_bot(
        &mut d,
        "alice",
        Some((0, 0, 0)),
        &snap,
        &navs,
        &statuses,
        world.as_deref(),
        false,
        false,
        no_reach,
    );
    assert_eq!(d.walked, Some((4, 0)), "the hop resumes after the hold");
}

/// A script that counts ticks and claims every detected event for
/// itself (the Task 5 `Handle` override).
struct ClaimHandle(Arc<Mutex<u32>>);

impl script::Script for ClaimHandle {
    fn name(&self) -> &str {
        "ClaimHandle"
    }
    fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
        *self.0.lock().unwrap() += 1;
    }
    fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
        RandomClaim::Handle
    }
}

#[test]
fn handle_claim_keeps_ticks_and_blocks_host_talk() {
    let count = Arc::new(Mutex::new(0u32));
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(ClaimHandle(Arc::clone(&count))), None)
        .unwrap();
    // The production knock arm: ask the running slot script.
    let knock_scripts = Arc::clone(&scripts);
    let knock_name = "alice".to_string();
    let mut knock = move |ev: &DetectedRandom| -> RandomClaim {
        let Some(slot) = script_slot(&knock_scripts, &knock_name) else {
            return RandomClaim::Host;
        };
        let mut slot = slot.lock().unwrap();
        slot.on_random(ev)
    };

    // The guardian's rising edge knocks; the script claims the event,
    // so no Talk-to goes out and the slot is not held.
    let mut c = guardian_client();
    plant_npc(&mut c, 0, "Genie", Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = GuardRec::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
    assert_eq!(status.claim, RandomClaim::Handle);
    assert!(!status.hold, "a Handle claim never holds");
    assert!(drv.menus.is_empty(), "no Talk-to under a Handle claim");
    assert!(drv.actions.is_empty());

    // And the unheld slot still gets its game tick (the script's
    // `Handle` claim means the host leaves the random to it).
    let (navs, world) = empty_nav();
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    script_observe(
        &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert_eq!(*count.lock().unwrap(), 1, "a Handle claim still ticks");
}

#[test]
fn knock_reaches_peer_slot_while_other_slot_lock_held() {
    // OPT-006: knock takes a brief wall lookup then the peer slot lock —
    // never the wall lock for the whole on_random call.
    use api::random::{DetectedRandom, RandomKind};

    struct ClaimHandle;
    impl script::Script for ClaimHandle {
        fn name(&self) -> &str {
            "claim-handle"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
        fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
            RandomClaim::Handle
        }
    }

    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(TickCounter(Arc::new(Mutex::new(0)))), None)
        .unwrap();
    script_slot_or_insert(&scripts, "bob")
        .lock()
        .unwrap()
        .start_compiled(Box::new(ClaimHandle), None)
        .unwrap();

    let alice = script_slot(&scripts, "alice").unwrap();
    let _alice_guard = alice.lock().unwrap();

    let knock_scripts = Arc::clone(&scripts);
    let knock_name = "bob".to_string();
    let ev = DetectedRandom {
        kind: RandomKind::Dialog,
        name: "genie".to_string(),
        ours: true,
        npc_index: Some(0),
    };
    let claim = {
        let Some(slot) = script_slot(&knock_scripts, &knock_name) else {
            return;
        };
        let mut slot = slot.lock().unwrap();
        slot.on_random(&ev)
    };
    assert_eq!(claim, RandomClaim::Handle);
}

#[test]
fn ignored_randoms_skips_flee_but_detect_still_publishes() {
    // SEC-003: `ignoredRandoms()` remains readable from JS (EventSignal)
    // but the host knock must not honor it — Load isolates cannot
    // decline the guardian. Detect still publishes the kind.
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let src = "export default class T extends LoopingBot { ignoredRandoms() { return ['swarm']; } loop() {} }";
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");
    // The production knock arm (see the slot thread): always ask the
    // running slot script; ignore-list is not consulted here.
    let knock_scripts = Arc::clone(&scripts);
    let knock_name = "alice".to_string();
    let mut knock = move |ev: &DetectedRandom| -> RandomClaim {
        let Some(slot) = script_slot(&knock_scripts, &knock_name) else {
            return RandomClaim::Host;
        };
        let mut slot = slot.lock().unwrap();
        slot.on_random(ev)
    };

    let mut c = guardian_client();
    plant_attacking_npc(&mut c, 0, "Swarm");
    plant_positive_hit(&mut c);
    let mut g = Guardian::new();
    let mut drv = GuardRec::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
    assert_eq!(status.kind, Some(api::random::RandomKind::Evade));
    assert_eq!(status.name.as_deref(), Some("swarm"));
    assert!(status.ours, "detect still publishes the event");
    assert_eq!(
        status.claim,
        RandomClaim::Host,
        "ignoredRandoms cannot decline the guardian"
    );
    assert!(
        !drv.walks.is_empty(),
        "the guardian flees even when the script lists swarm as ignored"
    );
}

#[test]
fn event_signal_pending_reads_true_during_dialog_hold() {
    // Task 12: the guardian's dialog hold is posted into the isolate
    // (`hold: true` on the blob), so `EventSignal.pending()` reads
    // true while the slot is frozen by the in-flight dialog. The
    // shim's `pending()` mapping is pinned by the script crate's
    // load_isolate tests; here the held edge drives the full
    // guardian -> observe -> isolate chain.
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let src = "export default class T extends LoopingBot { loop() {} }";
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(src.to_string(), script::LoadShape::CompatClass, vec![])
        .expect("load isolate starts");

    // The guardian talks to the old man and holds the slot.
    let mut c = guardian_client();
    plant_npc(&mut c, 0, "Mysterious old man", Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = GuardRec::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.handling);
    assert!(status.hold, "the in-flight dialog holds the slot");

    // The held edge posts the blob and dispatches the isolate tick
    // (onPaint only); EventSignal reads the freeze from the blob.
    let (navs, world) = empty_nav();
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    assert!(script_observe(
        &mut c,
        "alice",
        true,
        true,
        1,
        Some((3200, 3200, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        status.hold,
        status.ours
    ));
    let hold = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__rs2b0t_host.snapshot.hold")
        .expect("posted snapshot reads back");
    assert_eq!(hold, true, "the dialog hold is what pending() reads");
    script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .stop();
}

#[test]
fn after_genie_gone_lamp_auto_off_in_inv_detects_without_hold() {
    let mut c = guardian_client();
    plant_npc(&mut c, 0, "Genie", Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = GuardRec::default();
    let settings = ProfileSettings {
        lamp_auto: false,
        ..ProfileSettings::default()
    };
    let mut snap = GameSnapshot::new();

    // Tick 1: the host talks to the genie and holds the slot.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.handling);
    assert!(status.hold);
    assert_eq!(drv.actions, vec![0]);

    // Tick 2: the genie is gone and the lamp sits in the inventory:
    // the handle lifts and the lamp is inert XP — no hold.
    drv.menus.clear();
    drv.actions.clear();
    c.npc[0] = None;
    c.npc_count = 0;
    plant_inv_item(&mut c, 2528); // Lamp (the genie lamp) obj id
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(api::random::RandomKind::Lamp));
    assert_eq!(status.name.as_deref(), Some("lamp"));
    assert!(!status.hold, "leftover lamp must not keep the slot held");
    assert!(
        !status.ours,
        "lamp auto off: leftover lamp must not latch EventSignal.pending via ours"
    );
    assert!(!status.handling, "inert lamp must not latch the handler");
    assert!(drv.menus.is_empty(), "a lamp is never talked to");
}

/// Test script that queues one walk to a (mutable) target each tick and
/// records what `ctx.walk` returned.
struct WalkProbe(Arc<Mutex<Option<bool>>>, Arc<Mutex<(i32, i32, i32)>>);

impl script::Script for WalkProbe {
    fn name(&self) -> &str {
        "WalkProbe"
    }
    fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        let (x, z, level) = *self.1.lock().unwrap();
        let ok = match ctx.walk.as_mut() {
            Some(w) => w(x, z, level),
            None => false,
        };
        *self.0.lock().unwrap() = Some(ok);
    }
}

/// Recording driver: captures the last accepted walk target. `route`
/// is `(0,0)` and `build_base` `(0,0)`, so absolute world tiles equal
/// scene tiles and `api::walk` resolves a route origin.
#[derive(Default)]
struct NavRec {
    walked: Option<(i32, i32)>,
    /// Held-item ops (OP_HELD1..=5): the jewellery rub arm's press.
    held_ops: usize,
    /// The component ids pressed via IF_BUTTON, in order (the follow's
    /// dialog-ride arm asserts *which* choice was answered).
    if_button_components: Vec<i32>,
    sink: Sink,
}

impl Driver for NavRec {
    fn set_menu(&mut self, _slot: i32, action: i32, _a: i32, _b: i32, c: i32) {
        match action {
            MiniMenuAction::OP_HELD1
            | MiniMenuAction::OP_HELD2
            | MiniMenuAction::OP_HELD3
            | MiniMenuAction::OP_HELD4
            | MiniMenuAction::OP_HELD5 => self.held_ops += 1,
            MiniMenuAction::IF_BUTTON => self.if_button_components.push(c),
            _ => {}
        }
    }
    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        dx: i32,
        dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _ty: i32,
    ) -> bool {
        self.walked = Some((dx, dz));
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        Some((0, 0))
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.sink
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

/// Minimal outbound sink: the recording driver never writes packets.
#[derive(Default)]
struct Sink;

impl api::prot::Out for Sink {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}

/// Walk-hook rig: a started `WalkProbe` for "alice" (target `(4,0,0)`),
/// an empty nav-bot map, the open-1×40 fixture world (x in 0..40 at
/// z=0), and a status row.
struct NavRig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    world: Option<Arc<NavWorld>>,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    walk_ret: Arc<Mutex<Option<bool>>>,
    walk_target: Arc<Mutex<(i32, i32, i32)>>,
}

fn nav_rig() -> NavRig {
    // An all-walkable 40×1 world at (0,0): x in 0..40 at z=0, no
    // transport edges — the collision+graph shape `find` consumes, built
    // directly (no pack file on disk in unit tests).
    nav_rig_with(Some(Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 40,
            height: 1,
            walk: vec![0u8; 40],
            blocked: vec![0u64; 40usize.div_ceil(64)],
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ))))
}

/// [`nav_rig`] on the given world (the walk-probe target stays
/// `(4,0,0)`; callers that walk elsewhere override `walk_target`).
fn nav_rig_with(world: Option<Arc<NavWorld>>) -> NavRig {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(Vec::new()));
    let walk_ret = Arc::new(Mutex::new(None));
    let walk_target = Arc::new(Mutex::new((4, 0, 0)));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(
            Box::new(WalkProbe(Arc::clone(&walk_ret), Arc::clone(&walk_target))),
            None,
        )
        .unwrap();
    statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    });
    NavRig {
        scripts,
        cheats,
        navs,
        world,
        statuses,
        walk_ret,
        walk_target,
    }
}

/// The armed route's dest, `None` when the uid has no route. The pump
/// and the walk-refusal gate read the same field.
fn queued(navs: &Arc<Mutex<HashMap<String, NavBot>>>) -> Option<WorldTile> {
    navs.lock()
        .unwrap()
        .get("alice")
        .and_then(|b| b.route.as_ref())
        .map(|r| r.dest)
}

/// A connected, ingame, scene-ready client on build base (0,0): world
/// tiles equal scene tiles, so `find`/`follow` and the pump's `here`
/// agree, and the snapshot passes `Interactions`' attached/ingame/
/// scene preconditions (the scenario runner's follow client does the
/// same).
fn nav_client() -> Client {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
    // Keep the listener alive so the connect stays established.
    std::mem::forget(listener);
    let mut c = prepare_client(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c
}

/// Rebuild the slot's nav snapshot with the player at scene/world
/// `(x, z)` (the body has no level decode, so level stays 0).
fn nav_snapshot_at(c: &mut Client, snap: &mut GameSnapshot, x: i32, z: i32) {
    c.local_player = Some(client::dash3d::ClientPlayer::at(x, z));
    c.bump_gens(client::io::ServerProt::PLAYER_INFO);
    c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    snap.rebuild(c);
}

#[test]
fn raw_bank_walk_reaches_resolved_stand_before_native_v2_opens_booth() {
    let mut c = bank_client();
    c.map_build_base_x = 3250;
    c.map_build_base_z = 3416;
    c.main_modal_id = -1;
    c.side_modal_id = -1;
    c.world.set_wall(
        0,
        3,
        3,
        0,
        0,
        0,
        0x4000_0000 + (2213 << 14) + 1 + (2 << 7),
        10,
        0,
        0,
        0,
        0,
    );
    let start = WorldTile {
        x: 3253,
        z: 3421,
        level: 0,
    };
    let stand = WorldTile {
        x: 3253,
        z: 3420,
        level: 0,
    };
    let booth = WorldTile {
        x: 3253,
        z: 3419,
        level: 0,
    };
    let mut snap = GameSnapshot::new();
    nav_snapshot_at(&mut c, &mut snap, 3, 5);

    let (walk, blocked) = nav::collision::pack_walk(&vec![0u32; 4 * 16 * 16]);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 3250,
                z: 3416,
                level: 0,
            },
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        vec![nav::pack::BankStand {
            name: "Bank booth".into(),
            tile: booth,
            access: nav::pack::BankAccess::Booth { op: 2 },
        }],
    ));
    let game_data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    world.bind_named_bank_facts(&game_data).unwrap();
    let bot = NavBot::default();
    let navs = Arc::new(Mutex::new(HashMap::from([("test".to_string(), bot)])));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let world_opt = Some(Arc::clone(&world));

    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../script/examples/bone_burier_v2.js"),
    )
    .unwrap();
    let iso = script::LoadIsolate::spawn(source, script::LoadShape::NativeTick, vec![]).unwrap();
    let filler = [(999, 1)];
    let post = |tick: u64, here: (i32, i32, i32), snapshot: &GameSnapshot| {
        with_script_snapshot_input(
            tick,
            Some(here),
            true,
            Some(&filler),
            Some(snapshot),
            None,
            Some(world.as_ref()),
            None,
            false,
            false,
            false,
            0,
            false,
            0,
            false,
            0,
            false,
            None,
            Default::default(),
            Default::default(),
            |input, native| {
                iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
                    input, native,
                ));
            },
        );
        iso.on_game_tick(tick);
        iso.probe("true").unwrap();
    };

    post(1, (start.x, start.z, start.level), &snap);
    let requests = iso.drain_interacts();
    assert_eq!(
        requests,
        vec![script::shim::InteractReq::WalkNearestBank],
        "native v2 must select the raw bank-walk path from the initial scene",
    );
    let out_before = c.out.pos;
    assert!(
        dispatch_script_interact_cached(
            &mut c,
            &snap,
            None,
            Some((start.x, start.z, start.level)),
            &navs,
            &world_opt,
            Some(WorldState::empty()),
            "test",
            requests,
            None,
            None,
        ),
        "raw bank selection must be accepted without a movement packet",
    );
    assert_eq!(
        c.out.pos, out_before,
        "raw bank selection itself must not write a movement packet",
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while navs
        .lock()
        .unwrap()
        .get("test")
        .and_then(|bot| bot.route.as_ref())
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "bank worker did not publish the selected stand route",
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    let flood = api::query::SceneQuery::new(snap.scene(), Some(start)).flood_reach();
    let reach = Arc::new(api::query::pack_reach_query(snap.scene(), flood.as_ref()));
    let out_before = c.out.pos;
    step_nav_bot(
        &mut c,
        "test",
        Some((start.x, start.z, start.level)),
        &snap,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        || Arc::clone(&reach),
    );
    assert!(
        c.out.pos > out_before,
        "the first follow pump must dispatch movement toward the resolved stand",
    );

    nav_snapshot_at(&mut c, &mut snap, 3, 4);
    let flood = api::query::SceneQuery::new(snap.scene(), Some(stand)).flood_reach();
    let reach = Arc::new(api::query::pack_reach_query(snap.scene(), flood.as_ref()));
    step_nav_bot(
        &mut c,
        "test",
        Some((stand.x, stand.z, stand.level)),
        &snap,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        || Arc::clone(&reach),
    );
    assert!(
        navs.lock().unwrap()["test"].route.is_none(),
        "arrival on the resolved stand must settle the host route",
    );

    post(2, (stand.x, stand.z, stand.level), &snap);
    assert!(
        iso.drain_interacts().is_empty(),
        "native v2 observes the operable stand before opening it",
    );
    post(3, (stand.x, stand.z, stand.level), &snap);
    let requests = iso.drain_interacts();
    assert!(
        matches!(
            requests.as_slice(),
            [script::shim::InteractReq::OpenStand {
                x,
                z,
                level,
                kind,
                name,
                stand_op,
                choose: None,
            }] if *x == booth.x
                && *z == booth.z
                && *level == booth.level
                && kind == "booth"
                && name.as_deref() == Some("Bank booth")
                && *stand_op == Some(2)
        ),
        "operable bank approach must produce OpenStand for the booth: {requests:?}",
    );
    let out_before = c.out.pos;
    assert!(
        dispatch_script_interact_cached(
            &mut c,
            &snap,
            None,
            Some((stand.x, stand.z, stand.level)),
            &navs,
            &world_opt,
            Some(WorldState::empty()),
            "test",
            requests,
            None,
            None,
        ),
        "OpenStand must dispatch through the real interaction boundary",
    );
    assert!(
        c.out.pos > out_before,
        "OpenStand must write the booth interaction packet",
    );
    iso.join();
}

#[test]
fn native_v2_open_stand_aborts_armed_walk_after_operable_neighbor() {
    let mut c = bank_client();
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    let start = WorldTile {
        x: 3206,
        z: 3204,
        level: 0,
    };
    let neighbor = WorldTile {
        x: 3205,
        z: 3205,
        level: 0,
    };
    let stand = WorldTile {
        x: 3204,
        z: 3205,
        level: 0,
    };
    let booth = WorldTile {
        x: 3205,
        z: 3206,
        level: 0,
    };
    let mut snap = GameSnapshot::new();
    nav_snapshot_at(&mut c, &mut snap, 6, 4);

    let (walk, blocked) = nav::collision::pack_walk(&vec![0u32; 4 * 16 * 16]);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));
    let world_opt = Some(Arc::clone(&world));
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let iso = script::LoadIsolate::spawn(
        r#"
export const apiVersion = 2;
let phase = 0;
export function tick(api) {
  if (phase === 0) {
    phase = 1;
    api.request({
      op: 'walk',
      x: 3204,
      z: 3205,
      level: 0,
      allow_teleports: false,
      allow_wilderness: false,
      allow_bank_fetch: false,
    });
  } else if (phase === 1
             && api.snapshot.here.x === 3205
             && api.snapshot.here.z === 3205) {
    phase = 2;
    api.request({
      op: 'open-stand',
      x: 3205,
      z: 3206,
      level: 0,
      kind: 'booth',
      name: 'Bank booth',
      stand_op: 2,
    });
  }
}
"#
        .into(),
        script::LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    let post = |tick: u64, here: WorldTile, snapshot: &GameSnapshot| {
        with_script_snapshot_input(
            tick,
            Some((here.x, here.z, here.level)),
            true,
            Some(&[]),
            Some(snapshot),
            None,
            Some(world.as_ref()),
            None,
            false,
            false,
            false,
            0,
            false,
            0,
            false,
            0,
            false,
            None,
            Default::default(),
            Default::default(),
            |input, native| {
                iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
                    input, native,
                ));
            },
        );
        iso.on_game_tick(tick);
        iso.probe("true").unwrap();
    };

    post(1, start, &snap);
    let requests = iso.drain_interacts();
    assert_eq!(
        requests,
        vec![script::shim::InteractReq::Walk {
            x: stand.x,
            z: stand.z,
            level: stand.level,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "native v2 must arm the walk before the approach tile is reached",
    );
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((start.x, start.z, start.level)),
        &navs,
        &world_opt,
        Some(WorldState::empty()),
        "native-v2",
        requests,
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while navs
        .lock()
        .unwrap()
        .get("native-v2")
        .and_then(|bot| bot.route.as_ref())
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "native v2 walk did not publish a route",
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    let reach = {
        let flood = api::query::SceneQuery::new(snap.scene(), Some(start)).flood_reach();
        Arc::new(api::query::pack_reach_query(snap.scene(), flood.as_ref()))
    };
    let out_before = c.out.pos;
    step_nav_bot(
        &mut c,
        "native-v2",
        Some((start.x, start.z, start.level)),
        &snap,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        || Arc::clone(&reach),
    );
    assert!(
        c.out.pos > out_before,
        "the armed v2 walk must dispatch movement before the neighbor is reached",
    );

    nav_snapshot_at(&mut c, &mut snap, 5, 5);
    let reach = {
        let flood = api::query::SceneQuery::new(snap.scene(), Some(neighbor)).flood_reach();
        Arc::new(api::query::pack_reach_query(snap.scene(), flood.as_ref()))
    };
    step_nav_bot(
        &mut c,
        "native-v2",
        Some((neighbor.x, neighbor.z, neighbor.level)),
        &snap,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        || Arc::clone(&reach),
    );

    let out_before = c.out.pos;
    assert!(!dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((neighbor.x, neighbor.z, neighbor.level)),
        &navs,
        &world_opt,
        Some(WorldState::empty()),
        "native-v2",
        vec![script::shim::InteractReq::OpenStand {
            x: booth.x,
            z: booth.z,
            level: booth.level,
            kind: "booth".into(),
            name: Some("Wrong booth".into()),
            stand_op: Some(2),
            choose: None,
        }],
    ));
    assert_eq!(
        c.out.pos, out_before,
        "a refused OpenStand must not write a booth packet",
    );
    assert!(
        navs.lock().unwrap()["native-v2"].route.is_some(),
        "a refused OpenStand must keep the armed v2 walk",
    );

    post(2, neighbor, &snap);
    let requests = iso.drain_interacts();
    assert!(
        matches!(
            requests.as_slice(),
            [script::shim::InteractReq::OpenStand {
                x,
                z,
                level,
                kind,
                name,
                stand_op: Some(2),
                choose: None,
            }] if *x == booth.x
                && *z == booth.z
                && *level == booth.level
                && kind == "booth"
                && name.as_deref() == Some("Bank booth")
        ),
        "v2 must open the booth from the operable neighboring tile: {requests:?}",
    );
    let out_before = c.out.pos;
    assert!(dispatch_script_interact(
        &mut c,
        &snap,
        None,
        Some((neighbor.x, neighbor.z, neighbor.level)),
        &navs,
        &world_opt,
        Some(WorldState::empty()),
        "native-v2",
        requests,
    ));
    assert!(
        c.out.pos > out_before,
        "the accepted OpenStand must dispatch the booth operation",
    );

    let out_after_open = c.out.pos;
    let reach = {
        let flood = api::query::SceneQuery::new(snap.scene(), Some(neighbor)).flood_reach();
        Arc::new(api::query::pack_reach_query(snap.scene(), flood.as_ref()))
    };
    step_nav_bot(
        &mut c,
        "native-v2",
        Some((neighbor.x, neighbor.z, neighbor.level)),
        &snap,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        || Arc::clone(&reach),
    );
    assert_eq!(
        c.out.pos, out_after_open,
        "accepted OpenStand must abort the armed v2 walk before a follow re-issues movement",
    );
    assert!(
        navs.lock().unwrap()["native-v2"].route.is_none(),
        "accepted OpenStand must clear the v2 route",
    );
    iso.join();
}

#[test]
fn script_observe_walk_arms_route_and_pump_steps_follow() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        statuses,
        walk_ret,
        ..
    } = nav_rig();
    let mut d = NavRec::default();
    let mut c = nav_client();

    // The observe dispatches the script tick with the walk hook; the
    // hook queues the request from the observed `here` and the worker
    // arms the uid's nav bot off-pump.
    assert!(script_observe(
        &mut d,
        "alice",
        true,
        true,
        1,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    ));
    assert_eq!(
        *walk_ret.lock().unwrap(),
        Some(true),
        "ctx.walk queued the route request"
    );
    assert!(
        wait_until(5_000, || queued(&navs)
            == Some(WorldTile {
                x: 4,
                z: 0,
                level: 0
            })),
        "the worker armed the route"
    );

    // The pump's per-uid nav step polls follow once, sending one hop
    // toward the dest and mirroring the armed dest into the status row.
    let mut snap = GameSnapshot::new();
    nav_snapshot_at(&mut c, &mut snap, 0, 0);
    step_nav_bot(
        &mut d,
        "alice",
        Some((0, 0, 0)),
        &snap,
        &navs,
        &statuses,
        world.as_deref(),
        false,
        false,
        no_reach,
    );
    assert_eq!(d.walked, Some((4, 0)), "the hop targets the dest tile");
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].walk_x, 4, "status mirrors the armed dest");
        assert_eq!(rows[0].walk_z, 0);
        assert_eq!(rows[0].walk_level, 0);
    }

    // Standing on the dest, the next pump poll reports Arrived and
    // clears the route; the status flips back to idle.
    nav_snapshot_at(&mut c, &mut snap, 4, 0);
    step_nav_bot(
        &mut d,
        "alice",
        Some((4, 0, 0)),
        &snap,
        &navs,
        &statuses,
        world.as_deref(),
        false,
        false,
        no_reach,
    );
    assert_eq!(queued(&navs), None, "arrival clears the armed route");
    {
        let rows = statuses.lock().unwrap();
        assert_eq!(rows[0].walk_x, -1, "idle bot reports no target");
        assert_eq!(rows[0].walk_z, -1);
        assert_eq!(rows[0].walk_level, -1);
    }
}

/// A WalkNear settles in the isolate once `here` is within its radius, even
/// short of the approach tile the route aimed at (the live chaos druid card
/// stood at (3114,9932), four tiles from its dest, while the route still
/// aimed at (3110,9932)). The host follow must end there too: a later stall
/// re-send walked the player out of the fight the card had just started.
#[test]
fn walk_near_follow_ends_when_here_is_within_the_requested_radius() {
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 20,
            height: 20,
            walk: vec![0u8; 400],
            blocked: vec![0u64; 400usize.div_ceil(64)],
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
    let arm = ScriptWalkArm {
        here: Some((10, 0, 0)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    };
    assert!(arm.route_with_radius(10, 16, 0, FindOptions::default(), 4));
    // The approach enumeration picks the lowest-x ring tile among the ties.
    let approach = WorldTile {
        x: 6,
        z: 12,
        level: 0,
    };
    assert!(
        wait_until(500, || queued(&navs) == Some(approach)),
        "the worker armed the approach route"
    );
    let mut d = NavRec::default();
    let mut c = nav_client();
    let mut snap = GameSnapshot::new();
    let step = |d: &mut NavRec, snap: &GameSnapshot, here: (i32, i32, i32)| {
        step_nav_bot(
            d,
            "alice",
            Some(here),
            snap,
            &navs,
            &statuses,
            Some(world.as_ref()),
            false,
            false,
            no_reach,
        )
    };
    nav_snapshot_at(&mut c, &mut snap, 10, 0);
    step(&mut d, &snap, (10, 0, 0));
    assert!(d.walked.is_some(), "the follow sends its first hop");

    // One tile outside the radius: the walk is still owed.
    nav_snapshot_at(&mut c, &mut snap, 10, 11);
    step(&mut d, &snap, (10, 11, 0));
    assert_eq!(
        queued(&navs),
        Some(approach),
        "outside the radius the route stays"
    );

    // Inside the radius, four tiles east of the approach tile: done.
    d.walked = None;
    nav_snapshot_at(&mut c, &mut snap, 10, 12);
    step(&mut d, &snap, (10, 12, 0));
    assert_eq!(d.walked, None, "no hop after the card's walk settled");
    assert_eq!(queued(&navs), None, "radius arrival clears the route");
    let rows = statuses.lock().unwrap();
    assert_eq!(
        (rows[0].walk_x, rows[0].walk_z),
        (-1, -1),
        "status reports idle"
    );
}

/// The slot's reach view for `here` on a 20x20 scene split by a closed
/// wall between rows z=12 and z=13 (a shut door the full width across).
fn walled_reach(x: i32, z: i32) -> Arc<api::query::ReachQueryView> {
    use client::dash3d::CollisionFlag;
    let mut scene = api::snapshot::SceneView {
        available: true,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: 20,
        height: 20,
        collision_flags: vec![0; 400],
    };
    for lx in 0..20 {
        scene.collision_flags[lx * 20 + 12] |= CollisionFlag::W_N;
        scene.collision_flags[lx * 20 + 13] |= CollisionFlag::W_S;
    }
    let here = WorldTile { x, z, level: 0 };
    let flood = api::query::SceneQuery::new(&scene, Some(here)).flood_reach();
    Arc::new(api::query::pack_reach_query(&scene, flood.as_ref()))
}

/// Frozen `isArrived` (WF-2): a radius that reaches through a closed wall
/// is not arrival, so the follow keeps the route on the wrong side; the
/// same radius from the dest's side of the wall ends it.
#[test]
fn walk_near_follow_does_not_end_through_a_closed_wall() {
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 20,
            height: 20,
            walk: vec![0u8; 400],
            blocked: vec![0u64; 400usize.div_ceil(64)],
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
    let arm = ScriptWalkArm {
        here: Some((10, 0, 0)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    };
    assert!(arm.route_with_radius(10, 16, 0, FindOptions::default(), 4));
    assert!(wait_until(500, || queued(&navs).is_some()), "route armed");
    let armed = queued(&navs);
    let mut d = NavRec::default();
    let mut c = nav_client();
    let mut snap = GameSnapshot::new();
    let step = |d: &mut NavRec,
                snap: &GameSnapshot,
                here: (i32, i32, i32),
                reach: &dyn Fn() -> Arc<api::query::ReachQueryView>| {
        step_nav_bot(
            d,
            "alice",
            Some(here),
            snap,
            &navs,
            &statuses,
            Some(world.as_ref()),
            false,
            false,
            reach,
        )
    };

    // Outside the radius the rule needs no probe: the view is never asked.
    nav_snapshot_at(&mut c, &mut snap, 10, 11);
    step(&mut d, &snap, (10, 11, 0), &|| {
        panic!("out-of-radius arrival must not read reach")
    });
    assert_eq!(queued(&navs), armed);

    // Chebyshev 4 <= radius 4, but the dest is behind the shut wall.
    nav_snapshot_at(&mut c, &mut snap, 10, 12);
    step(&mut d, &snap, (10, 12, 0), &|| walled_reach(10, 12));
    assert_eq!(
        queued(&navs),
        armed,
        "a radius through a closed wall is not arrival"
    );

    // Same radius from the dest's side of the wall: arrived.
    d.walked = None;
    nav_snapshot_at(&mut c, &mut snap, 10, 13);
    step(&mut d, &snap, (10, 13, 0), &|| walled_reach(10, 13));
    assert_eq!(d.walked, None, "no hop after arrival");
    assert_eq!(queued(&navs), None, "reachable in-radius arrival clears");
}

#[test]
fn walk_near_blocked_target_routes_to_an_arrival_capable_stand() {
    use client::dash3d::CollisionFlag;

    const SIZE: usize = 64;
    let mut flags = vec![0u32; SIZE * SIZE];
    for z in 8..=55usize {
        flags[z * SIZE + 31] |= CollisionFlag::W_E as u32;
        flags[z * SIZE + 32] |= CollisionFlag::W_W as u32;
    }
    let dest = WorldTile {
        x: 34,
        z: 32,
        level: 0,
    };
    let outside = WorldTile {
        x: 31,
        z: 32,
        level: 0,
    };
    let inside = WorldTile {
        x: 34,
        z: 33,
        level: 0,
    };
    flags[dest.z as usize * SIZE + dest.x as usize] |= CollisionFlag::SQ_BLOCKED as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));

    let mut client = nav_client();
    for z in 8..=55usize {
        client.collision[0].flags[31][z] |= CollisionFlag::W_E;
        client.collision[0].flags[32][z] |= CollisionFlag::W_W;
    }
    client.collision[0].flags[dest.x as usize][dest.z as usize] |= CollisionFlag::SQ_BLOCKED;
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, outside.x, outside.z);

    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
    let arm = ScriptWalkArm {
        here: Some((outside.x, outside.z, outside.level)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    };
    let worker_done = arm
        .queue_route_in_snapshot_synced(
            &snapshot,
            dest.x,
            dest.z,
            dest.level,
            FindOptions::default(),
            3,
            true,
            17,
        )
        .expect("route worker spawned");
    worker_done
        .recv_timeout(Duration::from_secs(2))
        .expect("route worker completed");
    let route = navs.lock().unwrap()["alice"].route.clone().expect("route");
    assert_eq!(
        route.dest, inside,
        "the route crosses around the wall to the legal destination-side stand"
    );
    assert!(
        route.ticks > 0.0,
        "the route must not settle where it started"
    );

    let outside_flood = api::query::SceneQuery::new(snapshot.scene(), Some(outside))
        .flood_reach()
        .expect("fresh outside flood");
    let outside_view = api::query::pack_reach_query(snapshot.scene(), Some(&outside_flood));
    assert!(
        !api::query::is_arrived(outside, dest, 3, || &outside_view),
        "the premature in-radius stand is not arrival-capable"
    );

    nav_snapshot_at(&mut client, &mut snapshot, inside.x, inside.z);
    let inside_flood = api::query::SceneQuery::new(snapshot.scene(), Some(inside))
        .flood_reach()
        .expect("fresh inside flood");
    let inside_view = Arc::new(api::query::pack_reach_query(
        snapshot.scene(),
        Some(&inside_flood),
    ));
    assert!(api::query::is_arrived(inside, dest, 3, || {
        Arc::clone(&inside_view)
    }));

    let mut driver = NavRec::default();
    step_nav_bot(
        &mut driver,
        "alice",
        Some((inside.x, inside.z, inside.level)),
        &snapshot,
        &navs,
        &statuses,
        Some(world.as_ref()),
        false,
        false,
        || Arc::clone(&inside_view),
    );
    let all = navs.lock().unwrap();
    let bot = &all["alice"];
    assert!(bot.route.is_none(), "fresh arrival clears the route");
    assert_eq!(
        bot.walk_outcome_seq, 0,
        "arrival, not route-terminal settlement, completed this walk"
    );
}

fn plant_nav_footprint_loc(client: &mut Client, x: i32, z: i32, width: i32, length: i32) {
    use client::config::LocType;

    let cache = Arc::get_mut(&mut client.cache).expect("nav test owns its cache");
    let id = cache.locs.len() as i32;
    cache.locs.push(LocType {
        id,
        name: "Bank booth".into(),
        width,
        length,
        op: vec![
            Some("Use".into()),
            Some("Use-quickly".into()),
            None,
            None,
            None,
        ],
        ..LocType::default()
    });
    let typecode = 0x4000_0000 + (id << 14) + x + (z << 7);
    client
        .world
        .set_wall(0, x, z, 0, 0, 0, typecode, 10, 0, 0, 0, 0);
}

fn arm_snapshot_route(
    world: Arc<NavWorld>,
    snapshot: &GameSnapshot,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
) -> nav::router::Route {
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let arm = ScriptWalkArm {
        here: Some((from.x, from.z, from.level)),
        world: Some(world),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    };
    let worker_done = arm
        .queue_route_in_snapshot_synced(
            snapshot,
            to.x,
            to.z,
            to.level,
            FindOptions::default(),
            radius,
            true,
            29,
        )
        .expect("route worker spawned");
    worker_done
        .recv_timeout(Duration::from_secs(2))
        .expect("route worker completed");
    let route = {
        let all = navs.lock().unwrap();
        all.get("alice").and_then(|bot| bot.route.clone())
    };
    route.expect("route")
}

#[test]
fn modeled_booth_behind_closed_door_routes_with_the_baked_graph() {
    use client::dash3d::CollisionFlag;
    use nav::transport::{TransportEdge, TransportKind};

    const SIZE: usize = 64;
    let booth = WorldTile {
        x: 32,
        z: 32,
        level: 0,
    };
    let from = WorldTile {
        x: 26,
        z: 32,
        level: 0,
    };
    let stand = WorldTile {
        x: 31,
        z: 32,
        level: 0,
    };
    let mut flags = vec![0u32; SIZE * SIZE];
    let mut client = nav_client();
    {
        let mut mark = |x: usize, z: usize, flag: i32| {
            flags[z * SIZE + x] |= flag as u32;
            client.collision[0].flags[x][z] |= flag;
        };
        for i in 30..=34 {
            mark(29, i, CollisionFlag::W_E);
            mark(30, i, CollisionFlag::W_W);
            mark(34, i, CollisionFlag::W_E);
            mark(35, i, CollisionFlag::W_W);
            mark(i, 29, CollisionFlag::W_N);
            mark(i, 30, CollisionFlag::W_S);
            mark(i, 34, CollisionFlag::W_N);
            mark(i, 35, CollisionFlag::W_S);
        }
        mark(
            booth.x as usize,
            booth.z as usize,
            CollisionFlag::SQ_BLOCKED,
        );
    }
    plant_nav_footprint_loc(&mut client, booth.x, booth.z, 1, 1);

    let edge = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 29,
            z: 32,
            level: 0,
        },
        to: WorldTile {
            x: 30,
            z: 32,
            level: 0,
        },
        loc_id: 1530,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(edge.at).or_default().push(0);
    graph.edges.push(edge);
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    ));
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, from.x, from.z);

    let query = api::query::SceneQuery::new(snapshot.scene(), Some(from));
    let flood = query.flood_reach().expect("outside flood");
    let loc = snapshot
        .locs()
        .iter()
        .find(|loc| loc.tile == booth)
        .expect("modeled booth");
    assert_eq!(
        query.booth_approach(loc, &flood).expect("modeled").dest,
        None,
        "the live flood cannot cross the shut door"
    );

    let route = arm_snapshot_route(world, &snapshot, from, booth, 1);
    assert_eq!(route.dest, stand);
    assert!(
        route.legs.iter().any(
            |leg| matches!(leg, nav::router::Leg::Transport { edge } if edge.kind == TransportKind::Door)
        ),
        "the baked multi-goal route must cross the closed door"
    );
    let stand_flood = api::query::SceneQuery::new(snapshot.scene(), Some(stand))
        .flood_reach()
        .expect("stand flood");
    let stand_view = api::query::pack_reach_query(snapshot.scene(), Some(&stand_flood));
    assert!(api::query::is_arrived(stand, booth, 1, || &stand_view));
}

#[test]
fn modeled_two_by_two_loc_routes_to_a_target_cardinal_arrival_stand() {
    use client::dash3d::CollisionFlag;

    const SIZE: usize = 32;
    let target = WorldTile {
        x: 10,
        z: 10,
        level: 0,
    };
    let from = WorldTile {
        x: 15,
        z: 15,
        level: 0,
    };
    let mut flags = vec![0u32; SIZE * SIZE];
    let mut client = nav_client();
    for x in 10..=11usize {
        for z in 10..=11usize {
            flags[z * SIZE + x] |= CollisionFlag::SQ_BLOCKED as u32;
            client.collision[0].flags[x][z] |= CollisionFlag::SQ_BLOCKED;
        }
    }
    plant_nav_footprint_loc(&mut client, target.x, target.z, 2, 2);
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    ));
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, from.x, from.z);

    let route = arm_snapshot_route(world, &snapshot, from, target, 1);
    assert!(
        [
            WorldTile {
                x: 9,
                z: 10,
                level: 0,
            },
            WorldTile {
                x: 10,
                z: 9,
                level: 0,
            },
        ]
        .contains(&route.dest),
        "the route must prefer the target tile's own cardinal sides, got {:?}",
        route.dest
    );
    let flood = api::query::SceneQuery::new(snapshot.scene(), Some(route.dest))
        .flood_reach()
        .expect("route-end flood");
    let view = api::query::pack_reach_query(snapshot.scene(), Some(&flood));
    assert!(
        api::query::is_arrived(route.dest, target, 1, || &view),
        "the chosen endpoint must satisfy the walk's own arrival rule"
    );
}

#[test]
fn empty_or_unroutable_solid_target_goals_fall_back_to_radius_policy() {
    use client::dash3d::CollisionFlag;

    const SIZE: usize = 32;
    let target = WorldTile {
        x: 16,
        z: 16,
        level: 0,
    };

    // No target-cardinal stand: the target sits inside a 3x3 solid block.
    let from = WorldTile {
        x: 12,
        z: 16,
        level: 0,
    };
    let mut flags = vec![0u32; SIZE * SIZE];
    let mut client = nav_client();
    for x in 15..=17usize {
        for z in 15..=17usize {
            flags[z * SIZE + x] |= CollisionFlag::SQ_BLOCKED as u32;
            client.collision[0].flags[x][z] |= CollisionFlag::SQ_BLOCKED;
        }
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    ));
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, from.x, from.z);
    let route = arm_snapshot_route(world, &snapshot, from, target, 4);
    assert_eq!(
        route.dest, from,
        "empty corrected goals fall back to the old in-radius route"
    );

    // Legal target-cardinal stands exist, but a full-height wall makes all
    // of them unroutable. The old radius policy can still settle on this side.
    let from = WorldTile {
        x: 13,
        z: 16,
        level: 0,
    };
    let mut flags = vec![0u32; SIZE * SIZE];
    let mut client = nav_client();
    for z in 0..SIZE {
        flags[z * SIZE + 13] |= CollisionFlag::W_E as u32;
        flags[z * SIZE + 14] |= CollisionFlag::W_W as u32;
        client.collision[0].flags[13][z] |= CollisionFlag::W_E;
        client.collision[0].flags[14][z] |= CollisionFlag::W_W;
    }
    flags[target.z as usize * SIZE + target.x as usize] |= CollisionFlag::SQ_BLOCKED as u32;
    client.collision[0].flags[target.x as usize][target.z as usize] |= CollisionFlag::SQ_BLOCKED;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    ));
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, from.x, from.z);
    let route = arm_snapshot_route(world, &snapshot, from, target, 3);
    assert_eq!(
        route.dest, from,
        "unroutable corrected goals fall back to the old in-radius route"
    );
}

/// AR-1 / frozen `'closest'` (`WalkExecutor.ts:316-325`): an r=12 WalkNear
/// in open terrain routes to an approach tile 12 tiles from the dest, where
/// the BFS rank of the dest is past the 512 arrival budget, so `is_arrived`
/// is false. When the follow reaches that route end it must publish a
/// settled (not failed) outcome for the armed request id, or the isolate
/// wait hangs to the caller timeout.
#[test]
fn radius_walk_route_end_publishes_a_settled_outcome() {
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 40,
            height: 40,
            walk: vec![0u8; 1600],
            blocked: vec![0u64; 1600usize.div_ceil(64)],
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(vec![SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    }]));
    let arm = ScriptWalkArm {
        here: Some((2, 20, 0)),
        world: Some(Arc::clone(&world)),
        navs: Arc::clone(&navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    };
    let dest = WorldTile {
        x: 30,
        z: 20,
        level: 0,
    };
    assert!(arm.queue_route(dest.x, dest.z, 0, FindOptions::default(), 12, true, 7));
    assert!(wait_until(500, || queued(&navs).is_some()), "route armed");
    let approach = queued(&navs).unwrap();
    assert_eq!(
        (approach.x - dest.x).abs().max((approach.z - dest.z).abs()),
        12,
        "the route ends on the near edge of the radius box"
    );
    // Open reach around the approach tile, wide enough that ring 12 is a
    // full BFS ring: the reach-aware rule says not arrived there.
    let open_reach = move || {
        let scene = api::snapshot::SceneView {
            available: true,
            base_x: -30,
            base_z: -30,
            level: 0,
            width: 100,
            height: 100,
            collision_flags: vec![0; 100 * 100],
        };
        let flood = api::query::SceneQuery::new(&scene, Some(approach)).flood_reach();
        Arc::new(api::query::pack_reach_query(&scene, flood.as_ref()))
    };
    assert!(!api::query::is_arrived(approach, dest, 12, open_reach));

    let mut d = NavRec::default();
    let mut c = nav_client();
    let mut snap = GameSnapshot::new();
    let step = |d: &mut NavRec,
                snap: &GameSnapshot,
                here: (i32, i32, i32),
                reach: &dyn Fn() -> Arc<api::query::ReachQueryView>| {
        step_nav_bot(
            d,
            "alice",
            Some(here),
            snap,
            &navs,
            &statuses,
            Some(world.as_ref()),
            false,
            false,
            reach,
        )
    };
    nav_snapshot_at(&mut c, &mut snap, 2, 20);
    step(&mut d, &snap, (2, 20, 0), &no_reach);
    assert!(d.walked.is_some(), "the follow sends its first hop");
    assert_eq!(navs.lock().unwrap()["alice"].walk_outcome_seq, 0);

    nav_snapshot_at(&mut c, &mut snap, approach.x, approach.z);
    step(&mut d, &snap, (approach.x, approach.z, 0), &open_reach);
    assert_eq!(queued(&navs), None, "the route ended");
    let all = navs.lock().unwrap();
    let bot = &all["alice"];
    assert_eq!(bot.walk_outcome_seq, 1, "the route end is published");
    assert!(!bot.walk_outcome_failed, "a route end settles true");
    assert_eq!(bot.walk_outcome_request_id, 7);
    assert_eq!(
        (
            bot.walk_outcome_x,
            bot.walk_outcome_z,
            bot.walk_outcome_level
        ),
        (dest.x, dest.z, dest.level),
        "keyed on the requested dest, as the isolate wait matches it"
    );
    assert_eq!(bot.walk_outcome_radius, 12);
}

/// A packed glory-style jewellery edge (obj 1712, `opheld4` Rub): the
/// shape every dest of the multi-location glory group shares. The
/// `to` names the landing (default Edgeville, `switch_int($choice)`
/// case 1); the group's sibling edges share `loc_id` + option and
/// differ only in `to`, exactly as the bake emits them.
fn glory_edge() -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Teleport,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3087,
            z: 3496,
            level: 0, // Edgeville (case 1)
        },
        loc_id: 1712,
        option: 4, // Rub (opheld4)
        ticks: 2,  // OP_BASE + the rub p_delay(1)
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![(1712, 1)],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    }
}

/// The charged glory in the inv tab (side 3) TYPE_INV container: the
/// shape `teleport_send` reads the rub's held item from.
fn plant_inv_item(c: &mut Client, obj_id: i32) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.objs.len() <= obj_id as usize {
            cache.objs.push(client::config::ObjType::default());
        }
        cache.objs[obj_id as usize] = client::config::ObjType {
            id: obj_id,
            iop: [None, None, None, Some("Rub".into()), None],
            ..Default::default()
        };
    }
    c.side_icon[3] = 300;
    c.set_iface(
        300,
        IfType {
            id: 300,
            layer_id: 300,
            children: Some(vec![301]),
            ..Default::default()
        },
    );
    c.set_iface(
        301,
        IfType {
            id: 301,
            layer_id: 300,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![obj_id + 1]),
            link_obj_number: Some(vec![1]),
            ..Default::default()
        },
    );
}

/// A chat destination dialog (root 100 with one BUTTON_OK choice
/// button per option, at components 101..): the shape a jewellery
/// rub's "Where would you like to teleport to?" opens.
fn plant_choice_dialog(c: &mut Client, options: &[&str]) {
    let root = 100;
    let children: Vec<i32> = (0..options.len()).map(|i| (101 + i) as i32).collect();
    for (i, text) in options.iter().enumerate() {
        let id = 101 + i;
        c.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: root,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                text: (*text).to_string(),
                ..Default::default()
            },
        );
    }
    c.set_iface(
        root as usize,
        IfType {
            id: root,
            layer_id: root,
            children: Some(children),
            ..Default::default()
        },
    );
    c.chat_modal_id = root;
    c.bump_gens(ServerProt::IF_OPENCHAT);
}

/// Bump every gen and rebuild into the existing snapshot (tick + 1).
fn bump_rebuild(c: &mut Client, snap: &mut GameSnapshot) {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    snap.rebuild(c);
}

/// The pump's per-uid nav step must hand the follow the packed any-tile
/// teleport list: a multi-destination jewellery rub (two glory
/// siblings, same `loc_id`, differing `to`) executes the SECOND
/// landing only when the traveller answers dialog choice 2 — the
/// 1-based index of the followed edge's `to` among the packed
/// same-`loc_id` rub edges. Without `world.graph.teleports` the
/// follow falls back to the modal's FIRST choice and rubs to the
/// wrong place (the same pass-through the panel and scenario follow
/// make).
#[test]
fn step_nav_bot_passes_graph_teleports_for_a_multi_dest_jewellery_rub() {
    let karamja = WorldTile {
        x: 2918,
        z: 3176,
        level: 0, // the packed glory case-2 landing
    };
    let edgeville = WorldTile {
        x: 3087,
        z: 3496,
        level: 0, // case 1
    };
    let glory = [
        TransportEdge {
            to: edgeville,
            ..glory_edge()
        },
        TransportEdge {
            to: karamja,
            ..glory_edge()
        },
    ];
    let world = Some(Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 1,
            height: 1,
            walk: vec![0u8; 1],
            blocked: vec![0u64; 1],
            flags: None,
        },
        TransportGraph {
            teleports: glory.to_vec(),
            ..TransportGraph::default()
        },
        Vec::new(),
    )));
    let NavRig {
        navs,
        world,
        statuses,
        ..
    } = nav_rig_with(world);

    let mut c = nav_client();
    plant_inv_item(&mut c, 1712);
    let mut snap = GameSnapshot::new();
    nav_snapshot_at(&mut c, &mut snap, 0, 0);
    let mut d = NavRec::default();

    // Arm the route for the SECOND landing directly in the uid's bot
    // (the arm is the router's job; this test pins the follow's
    // dialog answer).
    navs.lock()
        .unwrap()
        .entry("alice".into())
        .or_default()
        .route = Some(Route {
        legs: vec![Leg::Transport {
            edge: glory[1].clone(),
        }],
        dest: karamja,
        ticks: 2.0,
    });

    // Poll 1: the hop rubs the charged item.
    step_nav_bot(
        &mut d,
        "alice",
        Some((0, 0, 0)),
        &snap,
        &navs,
        &statuses,
        world.as_deref(),
        false,
        false,
        no_reach,
    );
    assert_eq!(d.held_ops, 1, "one OP_HELD4 rub sent");
    assert!(queued(&navs).is_some(), "the route stays armed");

    // The rub opens the destination choice: the next pump poll answers
    // the SECOND option (Karamja), never the constant first.
    plant_choice_dialog(&mut c, &["Edgeville.", "Karamja."]);
    bump_rebuild(&mut c, &mut snap);
    step_nav_bot(
        &mut d,
        "alice",
        Some((0, 0, 0)),
        &snap,
        &navs,
        &statuses,
        world.as_deref(),
        false,
        false,
        no_reach,
    );
    assert_eq!(
        d.if_button_components,
        vec![102],
        "the second destination answers choice 2, not the modal's first"
    );
}

/// A 5×5 world walled between x=1 and x=2, crossed only by a 10-coin
/// toll door (the `toll_edges` shape: loc 2882, `item_req` coins 10).
fn toll_nav_world() -> NavWorld {
    let mut flags = vec![0u32; 25];
    for z in 0..5 {
        flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
    }
    let edge = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 1,
            z: 2,
            level: 0,
        },
        to: WorldTile {
            x: 2,
            z: 2,
            level: 0,
        },
        loc_id: 2882,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![(995, 10)],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(edge.at).or_default().push(0);
    graph.edges.push(edge);
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    )
}

/// The script walk arm's facts gate the route: with 10 coins in the
/// slot's state, `ctx.walk` crosses the toll; the armed route carries
/// the toll Transport leg.
#[test]
fn script_observe_walk_uses_slot_state_across_a_toll() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        walk_ret,
        walk_target,
        ..
    } = nav_rig_with(Some(Arc::new(toll_nav_world())));
    *walk_target.lock().unwrap() = (4, 4, 0);
    let mut d = NavRec::default();
    let state = Some(WorldState {
        inv: std::collections::HashMap::from([(995, 10)]),
        ..WorldState::default()
    });
    assert!(script_observe(
        &mut d,
        "alice",
        true,
        true,
        1,
        Some((0, 0, 0)),
        None,
        state,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    ));
    assert_eq!(*walk_ret.lock().unwrap(), Some(true));
    assert!(
        wait_until(100, || queued(&navs)
            == Some(WorldTile {
                x: 4,
                z: 4,
                level: 0
            })),
        "the worker armed the toll route"
    );
    let route = navs
        .lock()
        .unwrap()
        .get("alice")
        .and_then(|b| b.route.clone())
        .expect("armed route");
    assert!(
        route.legs.iter().any(|l| matches!(
            l,
            nav::router::Leg::Transport { edge } if edge.item_req == vec![(995, 10)]
        )),
        "the route must cross the toll"
    );
}

/// The Rune Essence mine mapsquare (m45_75) as a sealed 64×64
/// all-walkable bake at (2880,4800): the pad and the four exit portal
/// placements inside, nothing packed — the session return hop is
/// synthesized by the router, so a script walk out only arms with a
/// latch.
fn mine_nav_world() -> NavWorld {
    NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 2880,
                z: 4800,
                level: 0,
            },
            width: 64,
            height: 64,
            walk: vec![0u8; 64 * 64],
            blocked: vec![0u64; (64usize * 64).div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

/// `ctx.walk` feeds the uid's latched essence session: a bot whose
/// traveller already latched the mine (entered via Aubury) can walk
/// out through the exit portal's return hop to the wizard's anchor.
#[test]
fn script_observe_walk_uses_the_latched_essence_session() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        walk_ret,
        walk_target,
        ..
    } = nav_rig_with(Some(Arc::new(mine_nav_world())));
    // Seed the bot's traveller with the latched session (the entry-hop
    // latch path itself is the traveller's own test).
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            traveller: {
                let mut t = Traveller::new();
                t.set_essence(nav::essence::essence_session_for_wizard(553));
                t
            },
            route: None,
            bank_fetch: None,
            allow_teleports: false,
            ..Default::default()
        },
    );
    // The walk target is Aubury's anchor; the origin is the mine pad.
    *walk_target.lock().unwrap() = (3253, 3401, 0);
    let mut d = NavRec::default();
    assert!(script_observe(
        &mut d,
        "alice",
        true,
        true,
        1,
        Some((2912, 4833, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    ));
    assert_eq!(*walk_ret.lock().unwrap(), Some(true));
    assert!(
        wait_until(100, || queued(&navs)
            == Some(WorldTile {
                x: 3253,
                z: 3401,
                level: 0
            })),
        "the worker armed the exit route with the latched session"
    );
}

/// No latch: the session return hop is never relaxed — the sealed
/// mine stays NoPath for `ctx.walk` (fail-closed remains correct).
#[test]
fn script_observe_walk_without_a_latch_keeps_the_mine_sealed() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        walk_ret,
        walk_target,
        ..
    } = nav_rig_with(Some(Arc::new(mine_nav_world())));
    *walk_target.lock().unwrap() = (3253, 3401, 0);
    let mut d = NavRec::default();
    assert!(script_observe(
        &mut d,
        "alice",
        true,
        true,
        1,
        Some((2912, 4833, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    ));
    assert_eq!(
        *walk_ret.lock().unwrap(),
        Some(true),
        "a no-path request is queued, not found synchronously"
    );
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        queued(&navs),
        None,
        "no latch -> the mine is sealed (NoPath never arms a route)"
    );
}

/// No slot state (no player decoded): the walk arm fails closed — the
/// toll stays unusable and no route arms.
#[test]
fn script_observe_walk_falls_back_to_empty_when_slot_has_no_state() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        walk_ret,
        walk_target,
        ..
    } = nav_rig_with(Some(Arc::new(toll_nav_world())));
    *walk_target.lock().unwrap() = (4, 4, 0);
    let mut d = NavRec::default();
    assert!(script_observe(
        &mut d,
        "alice",
        true,
        true,
        1,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    ));
    assert_eq!(*walk_ret.lock().unwrap(), Some(true), "request queued");
    thread::sleep(Duration::from_millis(20));
    assert_eq!(
        queued(&navs),
        None,
        "no facts -> the toll stays unusable, no route arms"
    );
}

#[test]
fn script_observe_walk_queues_off_pump_and_refuses_when_unarmable() {
    let NavRig {
        scripts,
        cheats,
        navs,
        world,
        walk_ret,
        walk_target,
        ..
    } = nav_rig();
    let no_world: Option<Arc<NavWorld>> = None;
    let mut d = NavRec::default();

    // No observed tile: synchronous refusal before any world lookup.
    script_observe(
        &mut d, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
        &world, false, false,
    );
    assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no here → refuse");

    // No nav world: synchronous refusal, no worker.
    script_observe(
        &mut d,
        "alice",
        true,
        true,
        2,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &no_world,
        false,
        false,
    );
    assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no world → refuse");

    // A request the world cannot satisfy is still queued (true) but
    // never arms: the worker's find fails and it exits without
    // touching the map.
    *walk_target.lock().unwrap() = (5, 5, 0);
    script_observe(
        &mut d,
        "alice",
        true,
        true,
        3,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        *walk_ret.lock().unwrap(),
        Some(true),
        "a no-path request is queued, not found synchronously"
    );
    thread::sleep(Duration::from_millis(20));
    assert_eq!(queued(&navs), None, "NoPath never arms a route");

    // A reachable request arms asynchronously on the worker.
    *walk_target.lock().unwrap() = (2, 0, 0);
    script_observe(
        &mut d,
        "alice",
        true,
        true,
        4,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(*walk_ret.lock().unwrap(), Some(true));
    assert!(
        wait_until(100, || queued(&navs)
            == Some(WorldTile {
                x: 2,
                z: 0,
                level: 0
            })),
        "the worker armed the queued route"
    );

    // A second walk while a route is queued refuses synchronously, so
    // a script spamming walk every tick spawns no worker per tick.
    *walk_target.lock().unwrap() = (1, 0, 0);
    script_observe(
        &mut d,
        "alice",
        true,
        true,
        5,
        Some((0, 0, 0)),
        None,
        None,
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    assert_eq!(
        *walk_ret.lock().unwrap(),
        Some(false),
        "already-queued → refuse, no worker spawn"
    );
    assert_eq!(
        queued(&navs),
        Some(WorldTile {
            x: 2,
            z: 0,
            level: 0
        }),
        "the armed route is untouched"
    );
}

fn thiever_watch_observation(account: &str) -> catalog_core::Observation {
    let mut observation = catalog_core::Observation {
        ingame: true,
        scene_state: 2,
        player: Some(account.into()),
        tile: Some((2661, 3306, 0)),
        ..catalog_core::Observation::default()
    };
    observation.levels.insert("thieving".into(), 50);
    observation.levels.insert("hitpoints".into(), 50);
    observation.effective_levels.insert("thieving".into(), 50);
    observation.effective_levels.insert("hitpoints".into(), 50);
    observation.items.insert("Lobster".into(), 10);
    observation
}

#[test]
fn observer_pump_inactive_watches_skip_lifecycle_and_guardian_producers() {
    use std::cell::Cell;

    let lifecycle_called = Cell::new(false);
    let guardian_called = Cell::new(false);
    let catalog = catalog_core::CoreWatch::default();
    let paired = paired_core::PairWatch::default();
    let snapshot = GameSnapshot::new();
    let names = api::obj_names::ObjNames::default();

    observe_slot_catalog_and_paired(
        &catalog,
        &paired,
        "alice",
        &snapshot,
        &names,
        false,
        || {
            lifecycle_called.set(true);
            None
        },
        || {
            guardian_called.set(true);
            catalog_core::BoundedGuardian::default()
        },
        None,
        None,
        None,
    );

    assert!(
        !lifecycle_called.get(),
        "disabled catalog must not read the script lifecycle receipt"
    );
    assert!(
        !guardian_called.get(),
        "disabled catalog must not build a guardian fact"
    );
}

#[test]
fn observer_pump_configured_catalog_runs_producers_and_clear_stops_them() {
    use std::cell::Cell;

    let lifecycle_called = Cell::new(false);
    let guardian_called = Cell::new(false);
    let catalog = catalog_core::CoreWatch::default();
    let paired = paired_core::PairWatch::default();
    catalog.configure(catalog_core::CoreCase::Thiever, "alice");
    let snapshot = GameSnapshot::new();
    let names = api::obj_names::ObjNames::default();

    let run = |lifecycle_called: &Cell<bool>, guardian_called: &Cell<bool>| {
        observe_slot_catalog_and_paired(
            &catalog,
            &paired,
            "alice",
            &snapshot,
            &names,
            false,
            || {
                lifecycle_called.set(true);
                None
            },
            || {
                guardian_called.set(true);
                catalog_core::BoundedGuardian {
                    kind: Some("lamp".into()),
                    hold: true,
                    ..catalog_core::BoundedGuardian::default()
                }
            },
            None,
            None,
            None,
        );
    };

    run(&lifecycle_called, &guardian_called);
    assert!(lifecycle_called.get());
    assert!(guardian_called.get());

    lifecycle_called.set(false);
    guardian_called.set(false);
    catalog.clear();
    run(&lifecycle_called, &guardian_called);
    assert!(!lifecycle_called.get());
    assert!(!guardian_called.get());
}

#[test]
fn observer_pump_paired_only_skips_catalog_fact_producers() {
    use std::cell::Cell;

    let lifecycle_called = Cell::new(false);
    let catalog = catalog_core::CoreWatch::default();
    let paired = paired_core::PairWatch::default();
    paired.configure(paired_core::PairCase::Flax, "runner", "spinner");
    let snapshot = GameSnapshot::new();
    let names = api::obj_names::ObjNames::default();

    observe_slot_catalog_and_paired(
        &catalog,
        &paired,
        "runner",
        &snapshot,
        &names,
        false,
        || {
            lifecycle_called.set(true);
            None
        },
        catalog_core::BoundedGuardian::default,
        None,
        None,
        None,
    );

    assert!(
        !lifecycle_called.get(),
        "paired-only slots must not touch catalog lifecycle/guardian producers"
    );
    assert!(paired.configured());
    assert_eq!(paired.status(), paired_core::PairWatchStatus::Ready);
}

#[test]
fn observer_pump_session_boundary_skips_guardian_producer_but_keeps_catalog_lifecycle() {
    use std::cell::Cell;

    let lifecycle_called = Cell::new(false);
    let guardian_called = Cell::new(false);
    let catalog = catalog_core::CoreWatch::default();
    let paired = paired_core::PairWatch::default();
    catalog.configure(catalog_core::CoreCase::Thiever, "alice");
    catalog.observe("alice", thiever_watch_observation("alice"), false);
    assert!(
        catalog.begin_start("alice").is_ok(),
        "pre-start baseline must arm Start"
    );

    observe_slot_catalog_and_paired(
        &catalog,
        &paired,
        "alice",
        &GameSnapshot::new(),
        &api::obj_names::ObjNames::default(),
        true,
        || {
            lifecycle_called.set(true);
            None
        },
        || {
            guardian_called.set(true);
            unreachable!("session boundary must not read prior-frame random status")
        },
        None,
        None,
        None,
    );

    assert!(
        lifecycle_called.get(),
        "active catalog still attaches lifecycle across session boundaries"
    );
    assert!(
        !guardian_called.get(),
        "session boundary uses a default guardian without prior-frame facts"
    );
    assert!(catalog
        .failure()
        .expect("post-Start session boundary is terminal")
        .contains("session boundary after Start"));
}

/// A machine's timed-out walk sends `abort-walk` with its own walk token:
/// the armed follow stops only while that walk is the armed one (as after
/// OpenBooth), and no game packet is written. A script walk armed since
/// then survives it.
#[test]
fn abort_walk_request_stops_only_the_machines_own_follow() {
    let mut c = bank_client();
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let (navs, world) = empty_nav();
    let armed = |walk_request_id| NavBot {
        route_generation: 3,
        walk_request_id,
        requested_route: Some(native_requested(
            WorldTile {
                x: 3210,
                z: 3210,
                level: 0,
            },
            1,
            false,
        )),
        ..Default::default()
    };
    let mut abort = |request_id| {
        let out_before = c.out.pos;
        assert!(!dispatch_script_interact(
            &mut c,
            &snap,
            None,
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![script::shim::InteractReq::AbortWalk { request_id }],
        ));
        assert_eq!(c.out.pos, out_before, "abort-walk writes no packet");
    };

    // A script walk (token 9) replaced the machine's walk (token 7).
    navs.lock().unwrap().insert("alice".to_string(), armed(9));
    abort(7);
    {
        let bot = &navs.lock().unwrap()["alice"];
        assert_eq!(bot.route_generation, 3, "the script's walk survives");
        assert_eq!(bot.walk_request_id, 9);
        assert!(bot.requested_route.is_some());
    }
    abort(0);
    assert_eq!(
        navs.lock().unwrap()["alice"].route_generation,
        3,
        "an unscoped abort stops nothing"
    );

    // The machine's own walk is still armed: it stops.
    navs.lock().unwrap().insert("alice".to_string(), armed(7));
    abort(7);
    let bot = &navs.lock().unwrap()["alice"];
    assert_eq!(bot.route_generation, 4, "the follow's route is superseded");
    assert!(bot.route.is_none());
    assert!(bot.requested_route.is_none());
}
