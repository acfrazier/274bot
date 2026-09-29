//! Live BankBudget walk-to-access at real packed banks (booth and teller).
//!
//! Forces a throwaway `HOME` (refuses the operator home), points cache at
//! the engine with `OPERATOR_HOME` / `BOT_CACHE_DIR`, and removes its
//! scratch dir on the way out. Game port defaults to the orchestrator
//! tunnel `45594`.
//!
//! `LIVE=1 cargo test -p host-play --lib live_bankbudget_access_from_the_street -- --ignored --nocapture --test-threads=1`

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use nav::bank_fetch::{nearest_bank_access, BankStep};
use nav::router::{FindOptions, Leg, Route};
use nav::world::NavWorld;
use parking_lot::Mutex;
use vault::{Profile, ProfileSettings};

use super::open_bank_at_here;
use super::{
    run_with_template, tele_args, NavBot, PendingBankFetch, ProfileOptions, SharedClientTemplate,
};

const BANKS: [(&str, WorldTile); 4] = [
    (
        "Varrock West",
        WorldTile {
            x: 3180,
            z: 3430,
            level: 0,
        },
    ),
    (
        "Draynor",
        WorldTile {
            x: 3088,
            z: 3240,
            level: 0,
        },
    ),
    (
        "Falador East",
        WorldTile {
            x: 3008,
            z: 3352,
            level: 0,
        },
    ),
    (
        "Al Kharid",
        WorldTile {
            x: 3264,
            z: 3163,
            level: 0,
        },
    ),
];

#[derive(Debug, Clone)]
enum Phase {
    WaitReady,
    TutSkip,
    Tele { index: usize },
    WaitStreet { index: usize },
    Walk { index: usize },
    OpenBooth { index: usize },
    CloseBooth { index: usize },
    OpenNpc { index: usize },
    CloseNpc { index: usize },
    Pass,
    Fail(String),
}

struct ScratchHome {
    path: PathBuf,
    previous: Option<String>,
}

impl Drop for ScratchHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn home_is_throwaway(home: &Path, scratch: &Path) -> bool {
    home == scratch
        && (home.starts_with(std::env::temp_dir()) || home.starts_with(Path::new("/tmp")))
}

fn install_throwaway_home() -> ScratchHome {
    let previous = std::env::var("HOME").ok();
    let path = std::env::temp_dir().join(format!(
        "274bot-m279-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&path).expect("throwaway HOME");
    std::env::set_var("HOME", &path);
    let now = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    assert!(
        home_is_throwaway(&now, &path),
        "live harness refuses to run against HOME={}; need a throwaway under {}",
        now.display(),
        std::env::temp_dir().display()
    );
    ScratchHome { path, previous }
}

struct Live {
    phase: Phase,
    last_cheat: Instant,
    opened_at: Option<Instant>,
    world: Arc<NavWorld>,
    bot: NavBot,
}

fn live() -> bool {
    std::env::var("LIVE").as_deref() == Ok("1")
}

fn live_profile(scratch: &Path) -> ProfileOptions {
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2080);
    let nav_pack =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/nav/289/274bot.navpack");
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(scratch.join("vault")),
        unpack_dir: Some(scratch.join("unpack")),
        nav_pack: nav_pack.exists().then_some(nav_pack),
        cache_dir: std::env::var_os("BOT_CACHE_DIR").map(PathBuf::from),
        engine_dir: std::env::var_os("BOT_ENGINE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    }
}

fn dummy_route(dest: WorldTile) -> Route {
    Route {
        legs: vec![Leg::Walk { tiles: vec![dest] }],
        dest,
        ticks: 1.0,
    }
}

fn near(tile: Option<(i32, i32, i32)>, dest: WorldTile, radius: i32) -> bool {
    tile.is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs().max((z - dest.z).abs()) <= radius
    })
}

fn on_packed_stand(world: &NavWorld, here: Option<(i32, i32, i32)>) -> bool {
    let Some((x, z, level)) = here else {
        return false;
    };
    world
        .banks()
        .iter()
        .any(|stand| stand.tile.x == x && stand.tile.z == z && stand.tile.level == level)
}

fn action_slot(actions: &[Option<String>], want: &str) -> Option<i32> {
    let want = want.to_lowercase();
    actions.iter().enumerate().find_map(|(i, action)| {
        action.as_deref().and_then(|label| {
            label
                .to_lowercase()
                .eq_ignore_ascii_case(&want)
                .then_some(i as i32 + 1)
        })
    })
}

fn drive(client: &mut Client, live: &Mutex<Live>) {
    let mut snap = GameSnapshot::new();
    snap.rebuild(client);
    let mut g = live.lock();
    let now = Instant::now();
    let here = snap.tile();
    match g.phase.clone() {
        Phase::WaitReady => {
            if client.ingame && client.scene_state == 2 && snap.local_player().is_some() {
                println!(
                    "live_bank_access: ready tile={:?} scene={}",
                    snap.tile(),
                    snap.scene_state()
                );
                g.phase = Phase::TutSkip;
            }
        }
        Phase::TutSkip if now.duration_since(g.last_cheat) > Duration::from_millis(400) => {
            let _ = interact::cheat(client, "setvar tutorial 1000");
            g.last_cheat = now;
            g.phase = Phase::Tele { index: 0 };
        }
        Phase::Tele { index } if now.duration_since(g.last_cheat) > Duration::from_millis(400) => {
            let street = BANKS[index].1;
            let _ = interact::cheat(client, &tele_args(street));
            g.last_cheat = now;
            g.opened_at = None;
            g.bot = NavBot::default();
            g.phase = Phase::WaitStreet { index };
        }
        Phase::WaitStreet { index } => {
            let street = BANKS[index].1;
            if on_packed_stand(&g.world, here) {
                if now.duration_since(g.last_cheat) > Duration::from_secs(1) {
                    let _ = interact::cheat(client, &tele_args(street));
                    g.last_cheat = now;
                }
                return;
            }
            if !near(here, street, 8) {
                if now.duration_since(g.last_cheat) > Duration::from_secs(2) {
                    let _ = interact::cheat(client, &tele_args(street));
                    g.last_cheat = now;
                }
                return;
            }
            println!(
                "live_bank_access: {} street here={:?} (not on a packed stand)",
                BANKS[index].0, here
            );
            let from = here
                .map(|(x, z, level)| WorldTile { x, z, level })
                .unwrap_or(street);
            let access =
                nearest_bank_access(&g.world.collision, g.world.banks(), from).unwrap_or(street);
            g.bot.bank_fetch = Some(PendingBankFetch {
                steps: VecDeque::from([BankStep::Walk {
                    x: access.x,
                    z: access.z,
                    level: access.level,
                }]),
                dest: access,
                opts: FindOptions::default(),
                final_route: dummy_route(access),
                avoid: Vec::new(),
            });
            g.opened_at = Some(now);
            g.phase = Phase::Walk { index };
        }
        Phase::Walk { index } => {
            if on_packed_stand(&g.world, here) {
                g.phase = Phase::Fail(format!(
                    "{} Walk landed on a packed stand tile {:?}",
                    BANKS[index].0, here
                ));
                return;
            }
            let world = Arc::clone(&g.world);
            super::step_bank_fetch_on_bot(client, &snap, &mut g.bot, Some(&world), here, false);
            if let Some(route) = g.bot.route.clone() {
                let mut options = nav::traveller::TravelOptions {
                    close_enough: 0,
                    teleports: Some(world.graph.teleports.as_slice()),
                    edges: Some(world.graph.edges.as_slice()),
                    ..nav::traveller::TravelOptions::default()
                };
                let _ = g.bot.traveller.follow(client, &snap, route, &mut options);
            }
            if g.bot.bank_fetch.is_none() {
                println!(
                    "live_bank_access: {} Walk arrived here={:?}",
                    BANKS[index].0, here
                );
                g.opened_at = Some(now);
                g.last_cheat = now - Duration::from_secs(2);
                g.phase = Phase::OpenBooth { index };
                return;
            }
            if g.opened_at
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(45))
            {
                g.phase = Phase::Fail(format!(
                    "{} Walk never reached an access tile",
                    BANKS[index].0
                ));
            }
        }
        Phase::OpenBooth { index } => {
            if snap.bank_loaded() {
                g.phase = Phase::CloseBooth { index };
                g.last_cheat = now - Duration::from_secs(2);
                return;
            }
            if now.duration_since(g.last_cheat) < Duration::from_millis(800) {
                return;
            }
            let sent = snap.locs().iter().find_map(|loc| {
                action_slot(&loc.actions, "Use-quickly").map(|slot| {
                    let mut ix = Interactions::new(&snap, client);
                    matches!(
                        ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(slot)),
                        SendResult::Sent { .. }
                    )
                })
            });
            g.last_cheat = now;
            if sent == Some(true) {
                println!(
                    "live_bank_access: {} booth Use-quickly from {:?}",
                    BANKS[index].0, here
                );
            }
            if g.opened_at
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(25))
            {
                g.phase = Phase::Fail(format!("{} booth never opened", BANKS[index].0));
            }
        }
        Phase::CloseBooth { index } => {
            if snap.bank_component_id() < 0 {
                g.opened_at = Some(now);
                g.last_cheat = now - Duration::from_secs(2);
                g.phase = Phase::OpenNpc { index };
                return;
            }
            if now.duration_since(g.last_cheat) < Duration::from_millis(600) {
                return;
            }
            let mut ix = Interactions::new(&snap, client);
            let _ = ix.close_modal();
            g.last_cheat = now;
        }
        Phase::OpenNpc { index } => {
            if snap.bank_loaded() {
                g.phase = Phase::CloseNpc { index };
                g.last_cheat = now - Duration::from_secs(2);
                return;
            }
            if now.duration_since(g.last_cheat) < Duration::from_millis(800) {
                return;
            }
            let sent = open_bank_at_here(client, &snap, here, Some(&g.world));
            g.last_cheat = now;
            if sent {
                println!(
                    "live_bank_access: {} packed NPC/booth Open from {:?}",
                    BANKS[index].0, here
                );
            }
            if g.opened_at
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(25))
            {
                g.phase = Phase::Fail(format!("{} teller never opened", BANKS[index].0));
            }
        }
        Phase::CloseNpc { index } => {
            if snap.bank_component_id() < 0 {
                let next = index + 1;
                if next >= BANKS.len() {
                    g.phase = Phase::Pass;
                } else {
                    g.phase = Phase::Tele { index: next };
                    g.last_cheat = now - Duration::from_secs(1);
                }
                return;
            }
            if now.duration_since(g.last_cheat) < Duration::from_millis(600) {
                return;
            }
            let mut ix = Interactions::new(&snap, client);
            let _ = ix.close_modal();
            g.last_cheat = now;
        }
        Phase::TutSkip | Phase::Tele { .. } | Phase::Pass | Phase::Fail(_) => {}
    }
}

#[test]
fn live_harness_refuses_operator_home() {
    let scratch = std::env::temp_dir().join("274bot-m279-scratch");
    assert!(!home_is_throwaway(Path::new("/var/empty"), &scratch));
    assert!(home_is_throwaway(&scratch, &scratch));
}

#[test]
#[ignore = "requires LIVE=1 and the tunnelled local 289 engine"]
fn live_bankbudget_access_from_the_street() {
    assert!(
        live(),
        "live_bankbudget_access_from_the_street requires LIVE=1"
    );
    let scratch = install_throwaway_home();
    let options = live_profile(&scratch.path);
    let pack = options
        .nav_pack
        .clone()
        .expect("baked 289 navpack for live BankBudget");
    let world = Arc::new(NavWorld::load_pack(&pack).expect("load baked pack"));
    assert!(
        world.banks().len() >= 64,
        "live proof needs the real packed stand table, got {}",
        world.banks().len()
    );
    println!(
        "live_bank_access: port={:?} http={:?} cache={:?} stands={}",
        options.port,
        options.http_port,
        options.cache_dir,
        world.banks().len()
    );
    let profile = options
        .resolve(None)
        .expect("live profile resolve")
        .bind()
        .expect("live profile bind");
    let template =
        SharedClientTemplate::load(Arc::clone(&profile)).expect("live template/world load");
    let name = format!(
        "n{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
            % 1_000_000_000
    );
    let profiles = vec![Profile {
        username: name.clone(),
        password: name.clone().into(),
        uid: 274_279_002,
        settings: ProfileSettings::default(),
    }];
    let live_state = Arc::new(Mutex::new(Live {
        phase: Phase::WaitReady,
        last_cheat: Instant::now() - Duration::from_secs(1),
        opened_at: None,
        world,
        bot: NavBot::default(),
    }));
    let hook_state = Arc::clone(&live_state);
    let play = run_with_template(
        template,
        true,
        profiles,
        |_| (None, None),
        move |c, _, _| drive(c, &hook_state),
    )
    .expect("live play starts");
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let phase = live_state.lock().phase.clone();
        match phase {
            Phase::Pass => {
                let s = play.statuses();
                assert!(
                    s.iter().any(|row| row.ingame && row.scene_state == 2),
                    "live bank access without ingame scene 2"
                );
                println!("PASS: live_bankbudget_access_from_the_street banks={BANKS:?}");
                return;
            }
            Phase::Fail(msg) => panic!("FAIL: live_bankbudget_access_from_the_street: {msg}"),
            _ => {
                if Instant::now() >= deadline {
                    panic!(
                        "FAIL: live_bankbudget_access_from_the_street timeout in {:?}; statuses={:?}",
                        phase,
                        play.statuses()
                    );
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}
