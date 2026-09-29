//! Live BankBudget Open through a packed NPC teller (not a booth).
//!
//! `HOME` should be a throwaway directory. Point cache at the engine with
//! `OPERATOR_HOME` / `BOT_CACHE_DIR`. Game port defaults to the orchestrator
//! tunnel `45594`.
//!
//! `LIVE=1 cargo test -p host-play --lib live_bankbudget_opens_via_npc_teller -- --ignored --nocapture --test-threads=1`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use nav::collision::{pack_walk, WorldCollision};
use nav::pack::{BankAccess, BankStand};
use nav::transport::TransportGraph;
use nav::world::NavWorld;
use parking_lot::Mutex;
use vault::{Profile, ProfileSettings};

use super::open_bank_at_here;
use super::{run_with_template, tele_args, ProfileOptions, SharedClientTemplate};

const VARROCK_WEST: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};

#[derive(Debug, Clone)]
enum Phase {
    WaitReady,
    TutSkip,
    Tele,
    WaitTile,
    Open,
    Pass { banker: String, tile: WorldTile },
    Fail(String),
}

struct Live {
    phase: Phase,
    last_cheat: Instant,
    opened_at: Option<Instant>,
}

fn live() -> bool {
    std::env::var("LIVE").as_deref() == Ok("1")
}

fn live_profile() -> ProfileOptions {
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2080);
    let scratch = std::env::temp_dir().join(format!("274bot-m279-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
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

fn npc_only_world(tile: WorldTile) -> NavWorld {
    let flags = vec![0u32; 25];
    let (walk, blocked) = pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: tile.x - 2,
                z: tile.z - 2,
                level: tile.level,
            },
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![BankStand {
            name: "Banker".into(),
            tile,
            access: BankAccess::Npc {
                name: "Banker".into(),
                op: 3,
                choose: None,
            },
        }],
    )
}

fn near(tile: Option<(i32, i32, i32)>, dest: WorldTile, radius: i32) -> bool {
    tile.is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs().max((z - dest.z).abs()) <= radius
    })
}

fn drive(client: &mut Client, live: &Mutex<Live>) {
    let mut snap = GameSnapshot::new();
    snap.rebuild(client);
    let mut g = live.lock();
    let now = Instant::now();
    match g.phase.clone() {
        Phase::WaitReady => {
            if client.ingame && client.scene_state == 2 && snap.local_player().is_some() {
                println!(
                    "live_npc_bank: ready tile={:?} scene={}",
                    snap.tile(),
                    snap.scene_state()
                );
                g.phase = Phase::TutSkip;
            }
        }
        Phase::TutSkip if now.duration_since(g.last_cheat) > Duration::from_millis(400) => {
            let _ = interact::cheat(client, "setvar tutorial 1000");
            g.last_cheat = now;
            g.phase = Phase::Tele;
        }
        Phase::Tele if now.duration_since(g.last_cheat) > Duration::from_millis(400) => {
            let _ = interact::cheat(client, &tele_args(VARROCK_WEST));
            g.last_cheat = now;
            g.phase = Phase::WaitTile;
        }
        Phase::WaitTile => {
            if !near(snap.tile(), VARROCK_WEST, 8) {
                if now.duration_since(g.last_cheat) > Duration::from_secs(2) {
                    let _ = interact::cheat(client, &tele_args(VARROCK_WEST));
                    g.last_cheat = now;
                }
                return;
            }
            let banker = snap
                .npcs()
                .iter()
                .find(|npc| npc.name.as_deref() == Some("Banker"));
            let Some(banker) = banker else {
                return;
            };
            if snap.bank_loaded() {
                return;
            }
            println!(
                "live_npc_bank: at {:?} banker@{:?} index={} actions={:?}",
                snap.tile(),
                banker.tile,
                banker.index,
                banker.actions
            );
            if !near(snap.tile(), banker.tile, 1) {
                if now.duration_since(g.last_cheat) > Duration::from_secs(1) {
                    let _ = interact::cheat(client, &tele_args(banker.tile));
                    g.last_cheat = now;
                }
                return;
            }
            g.phase = Phase::Open;
        }
        Phase::Open => {
            if snap.bank_loaded() {
                let banker = snap
                    .npcs()
                    .iter()
                    .find(|npc| npc.name.as_deref() == Some("Banker"));
                g.phase = Phase::Pass {
                    banker: banker
                        .and_then(|n| n.name.clone())
                        .unwrap_or_else(|| "Banker".into()),
                    tile: banker.map(|n| n.tile).unwrap_or(VARROCK_WEST),
                };
                return;
            }
            if now.duration_since(g.last_cheat) < Duration::from_millis(1200) {
                return;
            }
            let Some(banker) = snap
                .npcs()
                .iter()
                .find(|npc| npc.name.as_deref() == Some("Banker"))
            else {
                return;
            };
            let world = npc_only_world(banker.tile);
            let here = snap.tile();
            let sent = open_bank_at_here(client, &snap, here, Some(&world));
            g.last_cheat = now;
            if g.opened_at.is_none() {
                g.opened_at = Some(now);
            }
            if sent {
                println!(
                    "live_npc_bank: Open sent packed NPC access at {:?}",
                    banker.tile
                );
            }
            if g.opened_at
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(20))
            {
                g.phase = Phase::Fail("bank never loaded after packed NPC Open".into());
            }
        }
        Phase::TutSkip | Phase::Tele | Phase::Pass { .. } | Phase::Fail(_) => {}
    }
}

#[test]
#[ignore = "requires LIVE=1 and the tunnelled local 289 engine"]
fn live_bankbudget_opens_via_npc_teller() {
    if !live() {
        return;
    }
    let options = live_profile();
    println!(
        "live_npc_bank: port={:?} http={:?} cache={:?}",
        options.port, options.http_port, options.cache_dir
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
        uid: 274_279_001,
        settings: ProfileSettings::default(),
    }];
    let live = Arc::new(Mutex::new(Live {
        phase: Phase::WaitReady,
        last_cheat: Instant::now() - Duration::from_secs(1),
        opened_at: None,
    }));
    let hook_state = Arc::clone(&live);
    let play = run_with_template(
        template,
        true,
        profiles,
        |_| (None, None),
        move |c, _, _| drive(c, &hook_state),
    )
    .expect("live play starts");
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        let phase = live.lock().phase.clone();
        match phase {
            Phase::Pass { banker, tile } => {
                let s = play.statuses();
                assert!(
                    s.iter().any(|row| row.ingame && row.scene_state == 2),
                    "live NPC bank open without ingame scene 2"
                );
                println!(
                    "PASS: live_bankbudget_opens_via_npc_teller banker={banker} tile={tile:?}"
                );
                return;
            }
            Phase::Fail(msg) => panic!("FAIL: live_bankbudget_opens_via_npc_teller: {msg}"),
            _ => {
                if Instant::now() >= deadline {
                    panic!(
                        "FAIL: live_bankbudget_opens_via_npc_teller timeout in {:?}; statuses={:?}",
                        phase,
                        play.statuses()
                    );
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}
