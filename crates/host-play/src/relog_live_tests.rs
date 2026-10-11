//! Ignored live proofs (LEDGER-0202/RELOG) that a bot comes back to a world
//! after a protocol error and after the dead-server deadline, through the
//! real slot pump, relog decision and login queue against a local R289
//! engine.
//!
//! - `live_protocol_error_relogs_fresh_into_a_world`: the per-frame hook
//!   dispatches an unknown opcode (74, zero length) through the client's own
//!   dispatch, the T1 path DIAG-PROTO-74 observed.
//! - `live_dead_server_deadline_reconnects_into_a_world`: a test-only TCP
//!   relay between the client and the engine stops forwarding the server's
//!   bytes, so the client's 15 s deadline fires on real silence.
//!
//! Env: `LIVE=1`, `BOT_EVIDENCE_DIR`, `RELOG_ENGINE_DIR`, `RELOG_CACHE_DIR`,
//! `RELOG_UNPACK_DIR`, `RELOG_NAV_PACK` (absolute), optional
//! `RELOG_GAME_PORT` / `RELOG_HTTP_PORT` (44594 / 1080). Run each test in
//! its own process (it installs the host log sink).

use super::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vault::{Profile, ProfileSettings};

const READY_TIMEOUT: Duration = Duration::from_secs(180);
const RELOG_TIMEOUT: Duration = Duration::from_secs(300);
/// A rebuilt scene must hold steady this long before it is counted.
const SETTLE: Duration = Duration::from_secs(4);

struct Capture(Mutex<Vec<String>>);

impl api::hostlog::Sink for Capture {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let message = record.message;
        if message.starts_with("session exit")
            || message.starts_with("relog decision")
            || message.starts_with("unexpected logout")
            || message.starts_with("handshake ok")
            || message.starts_with("login code")
            || message.starts_with("scene ")
        {
            self.0.lock().unwrap().push(format!(
                "{:?} {} {}",
                Instant::now(),
                record.slot.unwrap_or("-"),
                message
            ));
        }
    }
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    ProtocolError,
    DeadServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitReady,
    Faulted,
    WaitRelog,
    Done,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct World {
    session: u64,
    zone: (i32, i32),
    scene_state: i32,
    npcs: i32,
    locs: usize,
}

impl World {
    fn of(client: &Client) -> Self {
        let mut locs = 0;
        for level in 0..4 {
            for x in 0..104 {
                for z in 0..104 {
                    if client.world.get_scene(level, x, z).is_some()
                        || client.world.get_wall(level, x, z).is_some()
                    {
                        locs += 1;
                    }
                }
            }
        }
        Self {
            session: client.gens.session,
            zone: (
                client.map_build_centre_zone_x,
                client.map_build_centre_zone_z,
            ),
            scene_state: client.scene_state,
            npcs: client.npc_count,
            locs,
        }
    }

    fn populated(self) -> bool {
        self.scene_state == 2 && self.npcs > 0 && self.locs > 0
    }

    fn json(self) -> Value {
        json!({
            "session": self.session,
            "zone": [self.zone.0, self.zone.1],
            "scene_state": self.scene_state,
            "npcs": self.npcs,
            "locs": self.locs,
        })
    }
}

struct LiveState {
    fault: Fault,
    phase: Phase,
    ready_since: Option<Instant>,
    before: Option<World>,
    /// Client `scene_state` values seen after the fault, deduplicated.
    scenes_after: Vec<i32>,
    last_scene: i32,
    relog_since: Option<Instant>,
    after: Option<World>,
    pause_requested: bool,
}

impl LiveState {
    fn new(fault: Fault) -> Self {
        Self {
            fault,
            phase: Phase::WaitReady,
            ready_since: None,
            before: None,
            scenes_after: Vec::new(),
            last_scene: -1,
            relog_since: None,
            after: None,
            pause_requested: false,
        }
    }
}

fn frame(client: &mut Client, state: &Mutex<LiveState>) {
    let mut state = state.lock().unwrap();
    match state.phase {
        Phase::WaitReady => {
            if !(client.ingame && client.scene_state == 2) {
                state.ready_since = None;
                return;
            }
            let since = *state.ready_since.get_or_insert_with(Instant::now);
            if since.elapsed() < SETTLE {
                return;
            }
            let world = World::of(client);
            if !world.populated() {
                return;
            }
            state.before = Some(world);
            state.last_scene = client.scene_state;
            state.phase = Phase::Faulted;
            match state.fault {
                Fault::ProtocolError => {
                    // The decoded opcode DIAG-PROTO-74 saw: 289 size 0, no
                    // handler. Dispatched through the client's own seam.
                    let mut payload = client::io::Packet::new(vec![]);
                    client.ptype = 74;
                    client.psize = 0;
                    client.handle_packet(74, &mut payload);
                }
                Fault::DeadServer => state.pause_requested = true,
            }
        }
        Phase::Faulted | Phase::WaitRelog => {
            let scene = if client.ingame {
                client.scene_state
            } else {
                -1
            };
            if scene != state.last_scene {
                state.scenes_after.push(scene);
                state.last_scene = scene;
            }
            let before = state.before.expect("baseline").session;
            if client.ingame && client.gens.session > before {
                state.phase = Phase::WaitRelog;
                if client.scene_state != 2 {
                    state.relog_since = None;
                    return;
                }
                let since = *state.relog_since.get_or_insert_with(Instant::now);
                if since.elapsed() < SETTLE {
                    return;
                }
                let world = World::of(client);
                if world.populated() {
                    state.after = Some(world);
                    state.phase = Phase::Done;
                }
            }
        }
        Phase::Done => {}
    }
}

/// Test-only relay between the client and the engine. `pause` stops the
/// server-to-client direction of the connection that is live when it is
/// called; later connections (the reconnect) pass through.
struct Relay {
    port: u16,
    latest: Arc<AtomicUsize>,
    paused: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl Relay {
    fn start(upstream: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind relay");
        let port = listener.local_addr().unwrap().port();
        let latest = Arc::new(AtomicUsize::new(0));
        let paused = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (latest_accept, paused_accept, stop_accept) =
            (Arc::clone(&latest), Arc::clone(&paused), Arc::clone(&stop));
        thread::spawn(move || {
            let mut next = 0;
            for incoming in listener.incoming() {
                if stop_accept.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(client) = incoming else { continue };
                let Ok(server) = TcpStream::connect(("127.0.0.1", upstream)) else {
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                };
                next += 1;
                let id = next;
                latest_accept.store(id, Ordering::Relaxed);
                pump(
                    client.try_clone().unwrap(),
                    server.try_clone().unwrap(),
                    None,
                );
                pump(server, client, Some((id, Arc::clone(&paused_accept))));
            }
        });
        Self {
            port,
            latest,
            paused,
            stop,
        }
    }

    fn pause_live_connection(&self) -> usize {
        let id = self.latest.load(Ordering::Relaxed);
        self.paused.store(id, Ordering::Relaxed);
        id
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// Copy `from` to `to`. With `pause`, bytes are dropped while the paused id
/// is this connection's: the client hears nothing until it gives up, and its
/// close ends the other direction, which shuts this one down. (No socket
/// option is touched here: the client socket's clones share one descriptor
/// with the other direction's blocking read.)
fn pump(mut from: TcpStream, mut to: TcpStream, pause: Option<(usize, Arc<AtomicUsize>)>) {
    thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if pause
                .as_ref()
                .is_some_and(|(id, paused)| paused.load(Ordering::Relaxed) == *id)
            {
                continue;
            }
            if to.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = from.shutdown(Shutdown::Both);
        let _ = to.shutdown(Shutdown::Both);
    });
}

fn env_path(name: &str) -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} must be supplied")));
    assert!(path.is_absolute(), "{name} must be absolute");
    path
}

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn run_live(fault: Fault, label: &str) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert!(
        api::hostlog::install_sink(&CAPTURE),
        "run this live proof in its own test process"
    );
    let evidence = env_path("BOT_EVIDENCE_DIR");
    std::fs::create_dir_all(&evidence).unwrap();
    let home = PathBuf::from(std::env::var_os("HOME").expect("isohome HOME"));
    let game_port = env_port("RELOG_GAME_PORT", 44594);
    let relay = (fault == Fault::DeadServer).then(|| Relay::start(game_port));
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(relay.as_ref().map_or(game_port, |relay| relay.port)),
        http_port: Some(env_port("RELOG_HTTP_PORT", 1080)),
        engine_dir: Some(env_path("RELOG_ENGINE_DIR")),
        cache_dir: Some(env_path("RELOG_CACHE_DIR")),
        unpack_dir: Some(env_path("RELOG_UNPACK_DIR")),
        nav_pack: Some(env_path("RELOG_NAV_PACK")),
        vault_path: Some(home.join(format!("vault-relog-{label}"))),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .prepare_template()
        .expect("prepare local-289 template");
    let names = mint_live_names(1);
    let (account, password) = mint_live_entries(&names).into_iter().next().unwrap();
    let state = Arc::new(Mutex::new(LiveState::new(fault)));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let started = Instant::now();
    let play = run_with_template(
        template,
        false,
        vec![Profile {
            username: account.clone(),
            password: password.into(),
            uid: 274_202_002,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, username, _| {
            if username == frame_account {
                frame(client, &frame_state);
            }
        },
    )
    .expect("start one-slot live Play");

    let mut paused_connection = None;
    let deadline = started + READY_TIMEOUT + RELOG_TIMEOUT;
    loop {
        let (phase, pause_requested) = {
            let state = state.lock().unwrap();
            (state.phase, state.pause_requested)
        };
        if pause_requested && paused_connection.is_none() {
            paused_connection = relay.as_ref().map(Relay::pause_live_connection);
        }
        if phase == Phase::Done || Instant::now() >= deadline {
            break;
        }
        if phase == Phase::WaitReady {
            assert!(
                started.elapsed() < READY_TIMEOUT,
                "{account} never reached a populated scene"
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
    let elapsed = started.elapsed();

    if let Some(arm) = play.arm(&account) {
        arm.request_logout();
        play.wake(&account);
    }
    let logout_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < logout_deadline
        && play
            .statuses()
            .iter()
            .any(|status| status.username == account && status.ingame)
    {
        thread::sleep(Duration::from_millis(100));
    }
    drop(play);
    drop(relay);

    let state = state.lock().unwrap();
    let log: Vec<String> = CAPTURE.0.lock().unwrap().clone();
    let receipt = json!({
        "label": label,
        "account": account,
        "fault": format!("{fault:?}"),
        "phase": format!("{:?}", state.phase),
        "paused_connection": paused_connection,
        "elapsed_s": elapsed.as_secs_f64(),
        "before": state.before.map(World::json),
        "scenes_after_fault": state.scenes_after,
        "after": state.after.map(World::json),
        "host_log": log,
    });
    write_receipt(&evidence, label, &receipt);
    println!("{}", serde_json::to_string_pretty(&receipt).unwrap());

    assert_eq!(
        state.phase,
        Phase::Done,
        "{account} did not come back: {receipt}"
    );
    let after = state.after.expect("relogged world");
    assert!(after.populated(), "{receipt}");
    let attempts: Vec<&String> = log
        .iter()
        .filter(|line| line.contains("relog decision attempt"))
        .collect();
    let exit = log
        .iter()
        .find(|line| line.contains("session exit reason="))
        .expect("a logged session exit");
    match fault {
        Fault::ProtocolError => {
            assert!(
                exit.contains("protocol_error class=T1 last_opcode=74 size=0 session="),
                "{exit}"
            );
            assert!(exit.contains("previous=["), "{exit}");
            // The first attempt is the initial login; every relog after the
            // T1 is a fresh one.
            assert!(
                attempts.len() >= 2
                    && attempts[1..]
                        .iter()
                        .all(|line| line.ends_with("reconnect=false")),
                "{attempts:?}"
            );
            assert!(
                state.scenes_after.contains(&1),
                "the fresh login rebuilds the scene: {:?}",
                state.scenes_after
            );
        }
        Fault::DeadServer => {
            assert!(exit.contains("dead_server_deadline"), "{exit}");
            assert!(
                attempts.len() == 2 && attempts[1].ends_with("reconnect=true"),
                "{attempts:?}"
            );
        }
    }
}

fn write_receipt(dir: &Path, label: &str, receipt: &Value) {
    let path = dir.join(format!("live-{label}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(receipt).unwrap() + "\n")
        .expect("write live receipt");
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_protocol_error_relogs_fresh_into_a_world() {
    run_live(Fault::ProtocolError, "protocol-error");
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_dead_server_deadline_reconnects_into_a_world() {
    run_live(Fault::DeadServer, "dead-server");
}
