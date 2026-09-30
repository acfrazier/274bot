//! Live BankBudget fetch at real packed banks, from the street.
//!
//! Every leg runs a real planned session — Walk, Open, DepositAll,
//! Withdraw, Close, Wear — through the production pump: the script pump
//! ([`super::step_nav_bot`]) or the panel/TUI pump
//! ([`super::step_walk_arm_follow`]), once per player tick. Legs cover
//! Varrock West, Draynor, Falador East and Al Kharid, each with the booth
//! (a booth-only stand table, so Open can only click a booth) and with the
//! banker (the real table, whose Open tries the tellers first), through
//! both owners. A leg passes only when the bronze dagger seeded into the
//! bank ends up worn, the seeded bones are deposited, and Open used the
//! expected access: the booth next to the player, or a banker.
//!
//! Sets a throwaway `HOME` it removes on the way out, points cache at the
//! engine with `BOT_CACHE_DIR`, and defaults the game port to the
//! orchestrator tunnel `45594`.
//!
//! `LIVE=1 cargo test -p host-play --lib live_bankbudget_fetch_from_the_street -- --ignored --nocapture --test-threads=1`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, MiniMenuAction};
use nav::bank_fetch::{plan_bank_fetch, BankStep};
use nav::pack::BankAccess;
use nav::router::{FindOptions, Leg, MissingReq, Route};
use nav::world::NavWorld;
use nav::WorldState;
use parking_lot::Mutex;
use vault::{Profile, ProfileSettings};

use super::{
    run_with_template, step_nav_bot, step_walk_arm_follow, tele_args, NavBot, PendingBankFetch,
    ProfileOptions, SharedClientTemplate, SlotStatus, WalkArm,
};

/// Bronze dagger: wieldable, seeded into the bank each leg.
const DAGGER: i32 = 1205;
/// Bones: the backpack junk the DepositAll must clear.
const BONES: i32 = 526;
const SEED: [&str; 4] = [
    "~clearinv",
    "~clearinv worn",
    "givebank bronze_dagger 1",
    "give bones 3",
];
const LEG_TIMEOUT: Duration = Duration::from_secs(90);

/// Street tiles outside each bank, never in a bankers' aisle.
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
            x: 3105,
            z: 3250,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Booth,
    Banker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// The script `walk_with` pump (`NavBot`).
    Script,
    /// The panel/TUI WalkTo pump (`WalkArm`).
    Panel,
}

#[derive(Debug, Clone, Copy)]
struct LegSpec {
    bank: usize,
    access: Access,
    owner: Owner,
}

fn legs() -> Vec<LegSpec> {
    let mut legs = Vec::new();
    for bank in 0..BANKS.len() {
        for access in [Access::Booth, Access::Banker] {
            for owner in [Owner::Script, Owner::Panel] {
                legs.push(LegSpec {
                    bank,
                    access,
                    owner,
                });
            }
        }
    }
    legs
}

#[derive(Debug, Clone)]
enum Phase {
    Relog,
    WaitRelog,
    WaitReady,
    TutSkip,
    Tele { leg: usize },
    WaitStreet { leg: usize },
    Seed { leg: usize, next: usize },
    WaitSeeded { leg: usize },
    Run { leg: usize },
    Pass,
    Fail(String),
}

/// The Open pump's menu send `(action, a, b, c)` and the tile it was sent
/// from.
type OpenSend = ((i32, i32, i32, i32), (i32, i32, i32));

/// Per-leg observations.
#[derive(Default)]
struct LegRun {
    started: Option<Instant>,
    latch: Option<(u64, (i32, i32, i32))>,
    pumps: u32,
    front: Option<BankStep>,
    open: Option<OpenSend>,
    bank_facts_logged: bool,
    empty_facts_logged: bool,
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

/// Point `HOME` at a fresh temp directory for the whole run; the guard
/// restores it and removes the directory.
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
    ScratchHome { path, previous }
}

struct Live {
    phase: Phase,
    last_cheat: Instant,
    full: Arc<NavWorld>,
    booths: Arc<NavWorld>,
    snap: GameSnapshot,
    navs: Arc<std::sync::Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<std::sync::Mutex<Vec<SlotStatus>>>,
    arm: WalkArm,
    run: LegRun,
    name: String,
    passed: Vec<String>,
}

impl Live {
    fn world(&self, spec: LegSpec) -> Arc<NavWorld> {
        match spec.access {
            Access::Booth => Arc::clone(&self.booths),
            Access::Banker => Arc::clone(&self.full),
        }
    }

    fn session_front(&self, owner: Owner) -> Option<Option<BankStep>> {
        match owner {
            Owner::Script => self
                .navs
                .lock()
                .unwrap()
                .get(&self.name)
                .and_then(|bot| bot.bank_fetch.as_ref())
                .map(|p| p.steps.front().cloned()),
            Owner::Panel => self
                .arm
                .bank_fetch
                .as_ref()
                .map(|p| p.steps.front().cloned()),
        }
    }
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

fn is_loc_op(action: i32) -> bool {
    [
        MiniMenuAction::OP_LOC1,
        MiniMenuAction::OP_LOC2,
        MiniMenuAction::OP_LOC3,
        MiniMenuAction::OP_LOC4,
        MiniMenuAction::OP_LOC5,
    ]
    .contains(&action)
}

fn is_npc_op(action: i32) -> bool {
    [
        MiniMenuAction::OP_NPC1,
        MiniMenuAction::OP_NPC2,
        MiniMenuAction::OP_NPC3,
        MiniMenuAction::OP_NPC4,
        MiniMenuAction::OP_NPC5,
    ]
    .contains(&action)
}

fn rows(items: &[api::snapshot::ItemView]) -> Vec<(i32, i32)> {
    items.iter().map(|it| (it.def.id, it.count)).collect()
}

fn wearing(snap: &GameSnapshot, id: i32) -> bool {
    snap.equipment()
        .iter()
        .any(|it| it.def.id == id && it.count >= 1)
}

fn holding(snap: &GameSnapshot, id: i32) -> bool {
    snap.inventory()
        .iter()
        .any(|it| it.def.id == id && it.count >= 1)
}

fn drive(client: &mut Client, live: &Mutex<Live>) {
    let mut g = live.lock();
    g.snap.rebuild(client);
    let now = Instant::now();
    let here = g.snap.tile();
    let legs = legs();
    match g.phase.clone() {
        Phase::WaitReady => {
            if client.ingame && client.scene_state == 2 && g.snap.local_player().is_some() {
                println!(
                    "live_bank_fetch: ready tile={:?} scene={}",
                    g.snap.tile(),
                    g.snap.scene_state()
                );
                g.phase = Phase::TutSkip;
            }
        }
        Phase::TutSkip if now.duration_since(g.last_cheat) > Duration::from_millis(400) => {
            let _ = interact::cheat(client, "setvar tutorial 1000");
            g.last_cheat = now;
            g.phase = Phase::Relog;
        }
        // A tutorial-skipped account binds its side tabs (the inv tab
        // among them) only at the next login.
        Phase::Relog if now.duration_since(g.last_cheat) > Duration::from_secs(2) => {
            let ifaces = Arc::clone(&client.ifaces);
            if !interact::logout(client, &ifaces) {
                g.phase = Phase::Fail("logout iface missing".into());
                return;
            }
            g.last_cheat = now;
            g.snap = GameSnapshot::new();
            g.phase = Phase::WaitRelog;
        }
        Phase::WaitRelog => {
            if client.ingame
                && client.scene_state == 2
                && g.snap.local_player().is_some()
                && g.snap.inventory_size() > 0
            {
                println!(
                    "live_bank_fetch: relogged tile={:?} side_icon[3]={} inventory_size={}",
                    g.snap.tile(),
                    client.side_icon[3],
                    g.snap.inventory_size()
                );
                g.phase = Phase::Tele { leg: 0 };
            } else if now.duration_since(g.last_cheat) > Duration::from_secs(60) {
                g.phase = Phase::Fail("no inv tab 60s after the relog".into());
            }
        }
        Phase::Tele { leg } if now.duration_since(g.last_cheat) > Duration::from_millis(400) => {
            let street = BANKS[legs[leg].bank].1;
            let _ = interact::cheat(client, &tele_args(street));
            g.last_cheat = now;
            g.run = LegRun {
                started: Some(now),
                ..LegRun::default()
            };
            g.phase = Phase::WaitStreet { leg };
        }
        Phase::WaitStreet { leg } => {
            if g.run
                .started
                .is_some_and(|t| now.duration_since(t) > Duration::from_secs(60))
            {
                g.phase = Phase::Fail(format!(
                    "leg {leg}: never settled on the street at {here:?}"
                ));
                return;
            }
            let spec = legs[leg];
            let street = BANKS[spec.bank].1;
            let off_street = on_packed_stand(&g.full, here) || !near(here, street, 4);
            if off_street || g.snap.bank_component_id() >= 0 {
                if now.duration_since(g.last_cheat) > Duration::from_secs(2) {
                    if g.snap.bank_component_id() >= 0 {
                        let mut ix = interact::Interactions::new(&g.snap, client);
                        let _ = ix.close_modal();
                    } else {
                        let _ = interact::cheat(client, &tele_args(street));
                    }
                    g.last_cheat = now;
                }
                return;
            }
            g.phase = Phase::Seed { leg, next: 0 };
        }
        Phase::Seed { leg, next }
            if now.duration_since(g.last_cheat) > Duration::from_millis(600) =>
        {
            let _ = interact::cheat(client, SEED[next]);
            g.last_cheat = now;
            g.phase = if next + 1 < SEED.len() {
                Phase::Seed {
                    leg,
                    next: next + 1,
                }
            } else {
                Phase::WaitSeeded { leg }
            };
        }
        Phase::WaitSeeded { leg } => {
            let seeded = g
                .snap
                .inventory()
                .iter()
                .filter(|it| it.def.id == BONES)
                .map(|it| it.count)
                .sum::<i32>()
                >= 3
                && !wearing(&g.snap, DAGGER)
                && !holding(&g.snap, DAGGER);
            if !seeded {
                if now.duration_since(g.last_cheat) > Duration::from_secs(10) {
                    g.phase = Phase::Fail(format!(
                        "leg {leg}: seeding never showed: inventory={:?} equipment={:?} inv()={:?}",
                        rows(g.snap.inventory()),
                        rows(g.snap.equipment()),
                        g.snap.inv()
                    ));
                }
                return;
            }
            let spec = legs[leg];
            let world = g.world(spec);
            let (x, z, level) = here.expect("seeded player tile");
            let from = WorldTile { x, z, level };
            let state = WorldState::from_snapshot(&g.snap).with_map_members(true);
            let Some(fetch) = plan_bank_fetch(
                &[MissingReq::WearAny { ids: vec![DAGGER] }],
                &state,
                &[(DAGGER, 1)],
                world.banks(),
                from,
                &world.collision,
            ) else {
                g.phase = Phase::Fail(format!("leg {leg} {spec:?}: no plan from {from:?}"));
                return;
            };
            let BankStep::Walk {
                x: wx,
                z: wz,
                level: wl,
            } = fetch.steps[0]
            else {
                g.phase = Phase::Fail(format!("leg {leg}: plan starts {:?}", fetch.steps));
                return;
            };
            let expected = [
                BankStep::Open,
                BankStep::DepositAll,
                BankStep::Withdraw {
                    id: DAGGER,
                    count: 1,
                },
                BankStep::Close,
                BankStep::Wear { id: DAGGER },
            ];
            if fetch.steps[1..] != expected {
                g.phase = Phase::Fail(format!("leg {leg}: plan order {:?}", fetch.steps));
                return;
            }
            // Judged against the leg's own table: a booth-only leg may plan
            // the booth's aisle-side tile, which is a banker's spawn in the
            // full table but no stand of this one (the Walk then falls back
            // to a routable access tile).
            if on_packed_stand(&world, Some((wx, wz, wl))) {
                g.phase = Phase::Fail(format!("leg {leg}: Walk planned onto a stand"));
                return;
            }
            let pending = PendingBankFetch {
                steps: fetch.steps.clone().into(),
                dest: from,
                opts: FindOptions::default(),
                final_route: Route {
                    legs: vec![Leg::Walk { tiles: vec![from] }],
                    dest: from,
                    ticks: 1.0,
                },
                avoid: Vec::new(),
                progress: Default::default(),
            };
            println!(
                "live_bank_fetch: leg {leg} {} {:?} {:?} from={from:?} plan={:?}",
                BANKS[spec.bank].0, spec.access, spec.owner, fetch.steps
            );
            match spec.owner {
                Owner::Script => {
                    let name = g.name.clone();
                    g.navs.lock().unwrap().insert(
                        name,
                        NavBot {
                            bank_fetch: Some(pending),
                            ..Default::default()
                        },
                    );
                }
                Owner::Panel => {
                    g.arm = WalkArm {
                        bank_fetch: Some(pending),
                        ..Default::default()
                    };
                }
            }
            g.run = LegRun {
                started: Some(now),
                front: fetch.steps.first().cloned(),
                ..LegRun::default()
            };
            g.phase = Phase::Run { leg };
        }
        Phase::Run { leg } => run_leg(client, &mut g, leg, legs[leg], now),
        Phase::TutSkip
        | Phase::Relog
        | Phase::Tele { .. }
        | Phase::Seed { .. }
        | Phase::Pass
        | Phase::Fail(_) => {}
    }
}

fn run_leg(client: &mut Client, g: &mut Live, leg: usize, spec: LegSpec, now: Instant) {
    let label = format!(
        "leg {leg} {} {:?} {:?}",
        BANKS[spec.bank].0, spec.access, spec.owner
    );
    if g.run
        .started
        .is_some_and(|t| now.duration_since(t) > LEG_TIMEOUT)
    {
        g.phase = Phase::Fail(format!(
            "{label}: no finish in {LEG_TIMEOUT:?}, front={:?}",
            g.run.front
        ));
        return;
    }
    let Some(here) = g.snap.tile() else {
        return;
    };
    let world = g.world(spec);
    // Production pumps once per player tick: (gens.player, here) moved.
    let key = (client.gens.player, here);
    if g.run.latch != Some(key) {
        g.run.latch = Some(key);
        let front_before = g.session_front(spec.owner).flatten();
        if front_before == Some(BankStep::Open) {
            client.menu_action[0] = -1;
        }
        match spec.owner {
            Owner::Script => {
                let name = g.name.clone();
                step_nav_bot(
                    client,
                    &name,
                    Some(here),
                    &g.snap,
                    &g.navs,
                    &g.statuses,
                    Some(&world),
                    false,
                    true,
                    || unreachable!("the harness never arms a radius walk"),
                );
            }
            Owner::Panel => {
                step_walk_arm_follow(
                    client,
                    &g.snap,
                    &mut g.arm,
                    Some(&world),
                    here,
                    true,
                    Some(&g.name),
                );
            }
        }
        g.run.pumps += 1;
        if front_before == Some(BankStep::Open) && client.menu_action[0] != -1 {
            g.run.open = Some((
                (
                    client.menu_action[0],
                    client.menu_param_a[0],
                    client.menu_param_b[0],
                    client.menu_param_c[0],
                ),
                here,
            ));
        }
        let front_after = g.session_front(spec.owner).flatten();
        if front_after != g.run.front {
            println!(
                "live_bank_fetch: {label}: {:?} -> {front_after:?} here={here:?} pumps={} bank_com={} inventory={:?} bank_side={:?} worn={:?}",
                g.run.front,
                g.run.pumps,
                g.snap.bank_component_id(),
                rows(g.snap.inventory()),
                rows(g.snap.bank_side()),
                rows(g.snap.equipment()),
            );
            if matches!(g.run.front, Some(BankStep::Walk { .. }))
                && on_packed_stand(&g.full, Some(here))
            {
                g.phase = Phase::Fail(format!("{label}: Walk arrived on a stand {here:?}"));
                return;
            }
            g.run.front = front_after;
        }
    }
    if g.snap.bank_loaded() && !g.run.bank_facts_logged {
        g.run.bank_facts_logged = true;
        println!(
            "live_bank_fetch: {label}: bank open: side_icon[3]={} inventory={:?} inv()={:?} bank_side={:?}",
            client.side_icon[3],
            rows(g.snap.inventory()),
            g.snap.inv(),
            rows(g.snap.bank_side()),
        );
    }
    if g.snap.bank_loaded() && g.snap.bank_side().is_empty() && !g.run.empty_facts_logged {
        g.run.empty_facts_logged = true;
        println!(
            "live_bank_fetch: {label}: pack deposited: inventory={:?} inv()={:?} bank_side={:?} bank has dagger={}",
            rows(g.snap.inventory()),
            g.snap.inv(),
            rows(g.snap.bank_side()),
            g.snap.bank().iter().any(|it| it.def.id == DAGGER),
        );
    }
    if g.session_front(spec.owner).is_some() {
        return;
    }
    // The session ended: it must have fetched and worn the dagger.
    if !wearing(&g.snap, DAGGER) {
        g.phase = Phase::Fail(format!(
            "{label}: session ended at {:?} without the dagger worn; equipment={:?} inventory={:?}",
            g.run.front,
            rows(g.snap.equipment()),
            rows(g.snap.inventory())
        ));
        return;
    }
    if holding(&g.snap, BONES) {
        g.phase = Phase::Fail(format!("{label}: the bones were never deposited"));
        return;
    }
    if g.snap.bank_component_id() >= 0 {
        g.phase = Phase::Fail(format!("{label}: the bank is still open"));
        return;
    }
    let Some(((action, a, b, c), from)) = g.run.open else {
        g.phase = Phase::Fail(format!("{label}: no Open send observed"));
        return;
    };
    let used = match spec.access {
        Access::Booth => {
            let (bx, bz) = client_scene_to_world(client, b, c);
            let own = (bx - from.0).abs().max((bz - from.1).abs()) == 1;
            let booth = world.banks().iter().any(|s| {
                s.tile.x == bx && s.tile.z == bz && matches!(s.access, BankAccess::Booth { .. })
            });
            if !is_loc_op(action) || !own || !booth {
                g.phase = Phase::Fail(format!(
                    "{label}: Open was not the player's own booth: action={action} loc=({bx},{bz}) from={from:?}"
                ));
                return;
            }
            format!("booth ({bx},{bz}) from {from:?}")
        }
        Access::Banker => {
            let npc = g.snap.npcs().iter().find(|n| n.index == a as usize);
            if !is_npc_op(action) {
                g.phase = Phase::Fail(format!(
                    "{label}: Open did not use a banker: action={action}"
                ));
                return;
            }
            format!(
                "banker {:?} at {:?} from {from:?}",
                npc.and_then(|n| n.name.clone()),
                npc.map(|n| n.tile)
            )
        }
    };
    let line = format!(
        "{label}: worn dagger, bones deposited, bank closed; Open used {used}; {} pumps, {:.1}s",
        g.run.pumps,
        g.run
            .started
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0)
    );
    println!("live_bank_fetch: PASS {line}");
    g.passed.push(line);
    g.arm = WalkArm::default();
    g.navs.lock().unwrap().clear();
    g.phase = if leg + 1 < legs().len() {
        Phase::Tele { leg: leg + 1 }
    } else {
        Phase::Pass
    };
}

fn client_scene_to_world(client: &Client, sx: i32, sz: i32) -> (i32, i32) {
    (client.map_build_base_x + sx, client.map_build_base_z + sz)
}

#[test]
#[ignore = "requires LIVE=1 and the tunnelled local 289 engine"]
fn live_bankbudget_fetch_from_the_street() {
    assert!(
        live(),
        "live_bankbudget_fetch_from_the_street requires LIVE=1"
    );
    let scratch = install_throwaway_home();
    let options = live_profile(&scratch.path);
    let pack = options
        .nav_pack
        .clone()
        .expect("baked 289 navpack for live BankBudget");
    let bytes = std::fs::read(&pack).expect("read baked pack");
    let full = Arc::new(NavWorld::from_bytes(&bytes).expect("load baked pack"));
    let (collision, graph, stands) = nav::pack::decode(&bytes).expect("decode baked pack");
    drop(bytes);
    let booths = Arc::new(NavWorld::from_parts(
        collision,
        graph,
        stands
            .into_iter()
            .filter(|stand| matches!(stand.access, BankAccess::Booth { .. }))
            .collect(),
    ));
    assert!(
        full.banks().len() >= 64 && booths.banks().len() < full.banks().len(),
        "live proof needs the real packed stand table, got {} ({} booths)",
        full.banks().len(),
        booths.banks().len()
    );
    println!(
        "live_bank_fetch: port={:?} http={:?} cache={:?} stands={} booths={}",
        options.port,
        options.http_port,
        options.cache_dir,
        full.banks().len(),
        booths.banks().len()
    );
    let profile = options
        .resolve(None)
        .expect("live profile resolve")
        .bind()
        .expect("live profile bind");
    let template =
        SharedClientTemplate::load(Arc::clone(&profile)).expect("live template/world load");
    let name = format!(
        "f{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
            % 1_000_000_000
    );
    let profiles = vec![Profile {
        username: name.clone(),
        password: name.clone().into(),
        uid: 274_279_003,
        settings: ProfileSettings::default(),
    }];
    let live_state = Arc::new(Mutex::new(Live {
        phase: Phase::WaitReady,
        last_cheat: Instant::now() - Duration::from_secs(1),
        full,
        booths,
        snap: GameSnapshot::new(),
        navs: Arc::default(),
        statuses: Arc::default(),
        arm: WalkArm::default(),
        run: LegRun::default(),
        name,
        passed: Vec::new(),
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
    let deadline = Instant::now() + LEG_TIMEOUT * legs().len() as u32 + Duration::from_secs(120);
    loop {
        let phase = live_state.lock().phase.clone();
        match phase {
            Phase::Pass => {
                let s = play.statuses();
                assert!(
                    s.iter().any(|row| row.ingame && row.scene_state == 2),
                    "live bank fetch without ingame scene 2"
                );
                for line in &live_state.lock().passed {
                    println!("live_bank_fetch: {line}");
                }
                println!("PASS: live_bankbudget_fetch_from_the_street");
                return;
            }
            Phase::Fail(msg) => panic!("FAIL: live_bankbudget_fetch_from_the_street: {msg}"),
            _ => {
                if Instant::now() >= deadline {
                    panic!(
                        "FAIL: live_bankbudget_fetch_from_the_street timeout in {:?}; statuses={:?}",
                        phase,
                        play.statuses()
                    );
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}
