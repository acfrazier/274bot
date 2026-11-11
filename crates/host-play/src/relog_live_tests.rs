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
/// A parked bot stays offline this long before the operator logs it in: far
/// longer than a relog takes (about 10 s from the exit, queue included).
const PARK_HOLD: Duration = Duration::from_secs(30);
/// How long a script's bot stays in the world past the idle threshold.
const HELD_FOR: Duration = Duration::from_secs(30);
/// An idle count just below the Java threshold (4500): a bot without a
/// script would send the idle request within a few cycles.
const IDLE_SEED: u32 = 4490;

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
    /// The client's idle timer fires and the server logs the bot out.
    IdleLogout,
    /// The idle counter is raised past the threshold of a bot whose script
    /// holds the timer off.
    IdleHeld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitReady,
    Faulted,
    /// A bot without a script lost its session: nothing may reconnect it
    /// until [`PARK_HOLD`] has passed.
    Parked,
    /// [`Fault::IdleHeld`]: the bot must stay in the world for [`HELD_FOR`].
    Held,
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
    scripted: bool,
    phase: Phase,
    /// When the current `Parked` or `Held` phase began.
    phase_since: Option<Instant>,
    ready_since: Option<Instant>,
    before: Option<World>,
    /// Client `scene_state` values seen after the fault, deduplicated.
    scenes_after: Vec<i32>,
    last_scene: i32,
    relog_since: Option<Instant>,
    after: Option<World>,
    pause_requested: bool,
    /// The client was in game at some frame while the bot should be parked.
    online_while_parked: bool,
    /// The client's idle counter at the latest frame of the held phase.
    idle_cycles_after: Option<u32>,
    /// The client's idle request went out (its logout countdown started).
    idle_request_seen: bool,
}

impl LiveState {
    fn new(fault: Fault, scripted: bool) -> Self {
        Self {
            fault,
            scripted,
            phase: Phase::WaitReady,
            phase_since: None,
            ready_since: None,
            before: None,
            scenes_after: Vec::new(),
            last_scene: -1,
            relog_since: None,
            after: None,
            pause_requested: false,
            online_while_parked: false,
            idle_cycles_after: None,
            idle_request_seen: false,
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
                Fault::IdleLogout => {
                    // Nothing to inject: the bot has no script, so the
                    // client's own idle timer must reach its 4500-cycle
                    // threshold (90 s at the Java client's 20 ms cycle,
                    // replayed by the host through the parked passes).
                }
                Fault::IdleHeld => {
                    // A script's bot: raise the idle count to just below the
                    // threshold. Its timer is held off, so the count falls
                    // to zero and no request goes out.
                    client.shell.idle_cycles = IDLE_SEED;
                    state.phase = Phase::Held;
                    state.phase_since = Some(Instant::now());
                }
            }
        }
        Phase::Held => {
            state.idle_cycles_after = Some(client.shell.idle_cycles);
            if client.logout_timer > 0 {
                state.idle_request_seen = true;
            }
        }
        Phase::Parked => {
            if client.ingame {
                state.online_while_parked = true;
            }
        }
        Phase::Faulted | Phase::WaitRelog => {
            if state.fault == Fault::IdleLogout
                && !state.idle_request_seen
                && client.ingame
                && client.logout_timer > 0
            {
                // The client's own idle timer sent the idle request (it
                // starts the logout countdown as it writes). The local
                // engines run with `debug: true` (`WorldConfig.ts:101`), and
                // `IdleTimerHandler.ts:7` ignores the request there, so the
                // server's reply is the client's own handler for the Logout
                // packet a production server sends.
                state.idle_request_seen = true;
                let mut payload = client::io::Packet::new(vec![]);
                client.psize = 0;
                client.handle_packet(client::io::ServerProt289::LOGOUT, &mut payload);
            }
            if !state.scripted && state.phase == Phase::Faulted && !client.ingame {
                // The session ended and nothing will bring it back: the
                // hold starts now.
                state.phase = Phase::Parked;
                state.phase_since = Some(Instant::now());
                return;
            }
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
fn asset_path(name: &str, description: &str) -> Option<PathBuf> {
    let path = api::test_support::local_asset_path(name, "", description)?;
    assert!(
        path.is_absolute(),
        "{name} must be absolute: {}",
        path.display()
    );
    Some(path)
}

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn run_live(fault: Fault, scripted: bool, label: &str) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    if !api::test_support::game_data_available() {
        return;
    }
    let Some(engine_dir) = asset_path("RELOG_ENGINE_DIR", "local R289 engine content") else {
        return;
    };
    let Some(cache_dir) = asset_path("RELOG_CACHE_DIR", "copied local R289 cache snapshot") else {
        return;
    };
    let Some(unpack_dir) = asset_path("RELOG_UNPACK_DIR", "local R289 cache snapshot") else {
        return;
    };
    let Some(nav_pack) = asset_path("RELOG_NAV_PACK", "local R289 navigation pack") else {
        return;
    };
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
        engine_dir: Some(engine_dir),
        cache_dir: Some(cache_dir),
        unpack_dir: Some(unpack_dir),
        nav_pack: Some(nav_pack),
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
    let state = Arc::new(Mutex::new(LiveState::new(fault, scripted)));
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
    if scripted {
        // The script owns the bot's session from the start: it logs in on
        // its need, and every later exit is a script's reconnect.
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .expect("selected R289 game data");
        play.script_start_handle()
            .start_test_script(&account, Box::new(IdleScript), Some(selected))
            .expect("start the idle script");
    }

    let mut paused_connection = None;
    let mut parked_facts = None;
    let deadline = started + READY_TIMEOUT + RELOG_TIMEOUT + PARK_HOLD + HELD_FOR;
    loop {
        let (phase, pause_requested, since) = {
            let state = state.lock().unwrap();
            (state.phase, state.pause_requested, state.phase_since)
        };
        if pause_requested && paused_connection.is_none() {
            paused_connection = relay.as_ref().map(Relay::pause_live_connection);
        }
        if phase == Phase::Parked && since.is_some_and(|at| at.elapsed() >= PARK_HOLD) {
            // Parked long enough that any automatic relog would have
            // started: record what the host says, then log the bot in.
            let arm = play.arm(&account).expect("the slot's arm");
            let log = CAPTURE.0.lock().unwrap().clone();
            parked_facts = Some(json!({
                "latch_reason": arm.login_latch_reason().map(|reason| reason.to_string()),
                "park_reason": arm.relog_park_reason().to_string(),
                "attempts": log.iter().filter(|l| l.contains("relog decision attempt")).count(),
                "park_lines": log.iter().filter(|l| l.contains("relog decision park reason=idle_")).count(),
                "ingame_status": play.statuses().iter().any(|s| s.username == account && s.ingame),
            }));
            let mut state = state.lock().unwrap();
            state.phase = Phase::WaitRelog;
            state.phase_since = None;
            drop(state);
            arm.arm_explicit_login();
            play.wake(&account);
        }
        if phase == Phase::Held && since.is_some_and(|at| at.elapsed() >= HELD_FOR) {
            state.lock().unwrap().phase = Phase::Done;
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
        "scripted": scripted,
        "phase": format!("{:?}", state.phase),
        "paused_connection": paused_connection,
        "elapsed_s": elapsed.as_secs_f64(),
        "before": state.before.map(World::json),
        "scenes_after_fault": state.scenes_after,
        "after": state.after.map(World::json),
        "online_while_parked": state.online_while_parked,
        "idle_cycles_seen_after_fault": state.idle_cycles_after,
        "idle_request_seen": state.idle_request_seen,
        "parked_facts": parked_facts,
        "host_log": log,
    });
    write_receipt(&evidence, label, &receipt);
    println!("{}", serde_json::to_string_pretty(&receipt).unwrap());

    assert_eq!(
        state.phase,
        Phase::Done,
        "{account} did not finish the proof: {receipt}"
    );
    let attempts: Vec<&String> = log
        .iter()
        .filter(|line| line.contains("relog decision attempt"))
        .collect();
    if fault == Fault::IdleHeld {
        assert!(
            !log.iter().any(|line| {
                line.contains("session exit reason=") && !line.contains("operator_logout")
            }),
            "a running script keeps the bot in the world: {receipt}"
        );
        assert!(
            state
                .idle_cycles_after
                .is_some_and(|cycles| cycles < IDLE_SEED),
            "the script's bot never counts idle cycles: {receipt}"
        );
        assert!(
            !state.idle_request_seen,
            "the script's bot sends no idle request: {receipt}"
        );
        return;
    }
    let after = state.after.expect("relogged world");
    assert!(after.populated(), "{receipt}");
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
        }
        Fault::DeadServer => assert!(exit.contains("dead_server_deadline"), "{exit}"),
        Fault::IdleLogout => {
            assert!(
                state.idle_request_seen,
                "the client's own idle timer sent the idle request: {receipt}"
            );
            assert!(exit.contains("server_logout"), "{exit}");
            assert!(
                exit.contains("local_idle_request_pending=true"),
                "the exit follows the client's own idle request: {exit}"
            );
        }
        Fault::IdleHeld => unreachable!("returned above"),
    }
    if scripted {
        match fault {
            Fault::ProtocolError => {
                // The first attempt is the initial login; every relog after
                // the T1 is a fresh one.
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
            Fault::DeadServer => assert!(
                attempts.len() == 2 && attempts[1].ends_with("reconnect=true"),
                "{attempts:?}"
            ),
            Fault::IdleLogout | Fault::IdleHeld => unreachable!("not a scripted cell"),
        }
        return;
    }
    // No script: the bot parked at the exit and stayed logged out for the
    // whole hold; the operator's Login then connected it once.
    let facts = parked_facts.expect("the bot parked and was logged in by the operator");
    let expected = match fault {
        Fault::IdleLogout => "idle_logout",
        _ => "idle_disconnect",
    };
    assert_eq!(facts["park_reason"], expected, "{receipt}");
    assert_eq!(
        facts["attempts"], 1,
        "only the initial login before the park: {receipt}"
    );
    assert_eq!(
        facts["park_lines"], 1,
        "one park line with its reason: {receipt}"
    );
    assert_eq!(facts["ingame_status"], false, "{receipt}");
    assert!(
        !state.online_while_parked,
        "the bot relogged by itself: {receipt}"
    );
    // Only the operator's Login started a handshake after the park. The
    // login itself may retry: the local engine ignores the idle request or
    // the silence, so the old session can linger and refuse the account
    // (code 5) until it times out.
    assert!(
        attempts.len() >= 2,
        "the operator's Login starts the next handshake: {attempts:?}"
    );
}

fn write_receipt(dir: &Path, label: &str, receipt: &Value) {
    let path = dir.join(format!("live-{label}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(receipt).unwrap() + "\n")
        .expect("write live receipt");
}

/// A script that does nothing: the slot's script state is Running, so the
/// bot is "scripted" for the idle timer and the reconnect policy.
struct IdleScript;

impl script::native::Script for IdleScript {
    fn tick(
        &mut self,
        _tick: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        Ok(script::native::ScriptFlow::Continue)
    }
}

/// Scripted bots: unchanged policy, the bot comes back to a world.
#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_protocol_error_relogs_fresh_into_a_world() {
    run_live(Fault::ProtocolError, true, "protocol-error");
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_dead_server_deadline_reconnects_into_a_world() {
    run_live(Fault::DeadServer, true, "dead-server");
}

/// A running script holds the Java idle timer off: past the idle threshold
/// the bot is still in the world and no idle request has logged it out.
#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_scripted_bot_never_sends_the_idle_request() {
    run_live(Fault::IdleHeld, true, "idle-held");
}

/// Bots without a script: any unexpected exit parks the bot, which stays
/// logged out (no relog) until the operator logs it in.
#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_scriptless_protocol_error_stays_parked_until_login() {
    run_live(Fault::ProtocolError, false, "park-protocol-error");
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_scriptless_dead_server_stays_parked_until_login() {
    run_live(Fault::DeadServer, false, "park-dead-server");
}

/// The Java idle timer runs for a bot without a script: the client sends the
/// idle request, the server logs the bot out, and the bot stays logged out.
#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1 and the local R289 engine"]
fn live_scriptless_idle_logout_stays_parked_until_login() {
    run_live(Fault::IdleLogout, false, "park-idle-logout");
}
