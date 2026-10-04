//! Live BankBudget fetch at real packed banks, from the street.
//!
//! Every leg runs a real planned session — Walk, Open, Withdraw, Close, Wear —
//! through the production pump: the script pump
//! ([`super::step_nav_bot`]) or the panel/TUI pump
//! ([`super::step_walk_arm_follow`]), once per player tick. Legs cover
//! Varrock West, Draynor, Falador East and Al Kharid, each with the booth
//! (a booth-only stand table, so Open can only click a booth) and with the
//! banker (the real table, whose Open tries the tellers first), through
//! both owners. A leg passes only when the bronze dagger seeded into the
//! bank ends up worn, the seeded bones remain carried, and Open used the
//! expected access: the booth next to the player, or a banker.
//!
//! Sets a throwaway `HOME` it removes on the way out, points cache at the
//! engine with `BOT_CACHE_DIR`, and defaults the game port to the
//! orchestrator tunnel `45594`.
//!
//! `LIVE=1 cargo test -p host-play --lib live_bankbudget_fetch_from_the_street -- --ignored --nocapture --test-threads=1`
//!
//! The Mage Arena ignored proof runs a real-Play native Select/Open/Deposit/
//! Withdraw/Close script at the cellar without manufacturing a packed stand.
//! It saves JSON and a CPU-rendered PNG below `LIVE_EVIDENCE_DIR`.
//!
//! `LIVE=1 GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> GATHERER_GAME_PORT=<port> GATHERER_HTTP_PORT=<port> LIVE_EVIDENCE_DIR=<evidence-root> cargo test -p host-play --lib live_mage_teller_native_real_play_receipt -- --ignored --nocapture --test-threads=1`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::named_banks::BankPreferences;
use script::bank::{
    AccessKind, Close, Deposit, DepositArgs, Open, OpenArgs, Select, SelectArgs, Withdraw,
    WithdrawArgs, Withdrawal,
};
use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};

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
/// Bones: unrelated backpack items a fetch must retain.
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
    if g.snap
        .inv()
        .iter()
        .filter(|&&(id, _)| id == BONES)
        .map(|&(_, count)| count)
        .sum::<i32>()
        != 3
    {
        g.phase = Phase::Fail(format!("{label}: the fetch changed the carried bones"));
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
        "{label}: worn dagger, bones retained, bank closed; Open used {used}; {} pumps, {:.1}s",
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
const LAW_RUNE_ID: i32 = 563;
const MAGE_CELLAR: WorldTile = WorldTile {
    x: 2535,
    z: 4713,
    level: 0,
};
const MAGE_BANK_NAME: &str = "Mage Arena";
const MAGE_NPC_NAME: &str = "Gundai";
const MAGE_BANK_CHOICE: &str = "I'd like to access my bank account";

#[derive(Debug, Clone, PartialEq, Eq)]
enum TellerOutcome {
    Passed,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
struct TellerTrace {
    started_at: Option<[i32; 3]>,
    selected_bank: Option<String>,
    selected_kind: Option<String>,
    access_kind: Option<String>,
    access_npc: Option<String>,
    access_choice: Option<String>,
    access_operation: Option<String>,
    access_op_code: Option<i32>,
    access_tile: Option<[i32; 3]>,
    open_loaded: bool,
    inventory_law_before: Option<i32>,
    bank_law_before: Option<i32>,
    deposited: Option<u32>,
    inventory_law_after_deposit: Option<i32>,
    bank_law_after_deposit: Option<i32>,
    withdrew_complete: Option<bool>,
    inventory_law_after_withdraw: Option<i32>,
    bank_law_after_withdraw: Option<i32>,
    closed_observed: bool,
    final_tile: Option<[i32; 3]>,
    final_bank_open: Option<bool>,
    final_inventory_law: Option<i32>,
    live_tile: Option<[i32; 3]>,
    live_bank_open: Option<bool>,
    live_inventory_law: Option<i32>,
    failure: Option<String>,
    outcome: Option<TellerOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TellerPrep {
    WaitLogin,
    SkipTutorial,
    Logout,
    WaitRelog,
    Teleport,
    WaitCellar,
    ClearInventory,
    WaitClear,
    SeedLawRune,
    WaitSeed,
    Ready,
    Failed,
}

struct TellerLive {
    phase: TellerPrep,
    last_action: Instant,
    started: Instant,
    offline_seen: bool,
    trace: TellerTrace,
    evidence_dir: PathBuf,
    terminal_at: Option<Instant>,
    capture_started: bool,
    capture_written: bool,
    capture_error: Option<String>,
}

impl TellerLive {
    fn new(evidence_dir: PathBuf) -> Self {
        Self {
            phase: TellerPrep::WaitLogin,
            last_action: Instant::now() - Duration::from_secs(1),
            started: Instant::now(),
            offline_seen: false,
            trace: TellerTrace::default(),
            evidence_dir,
            terminal_at: None,
            capture_started: false,
            capture_written: false,
            capture_error: None,
        }
    }

    fn fail(&mut self, message: String) {
        if !self.capture_started {
            self.trace.failure = Some(message.clone());
            self.trace.outcome = Some(TellerOutcome::Failed(message));
            self.terminal_at = Some(Instant::now());
        }
        self.phase = TellerPrep::Failed;
    }

    fn pass(&mut self) {
        self.trace.outcome = Some(TellerOutcome::Passed);
        self.terminal_at = Some(Instant::now());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TellerStep {
    Select,
    Open,
    Deposit,
    Withdraw,
    Close,
    Done,
}

struct TellerScript {
    facts: Arc<api::named_banks::NamedBankFacts>,
    live: Arc<Mutex<TellerLive>>,
    step: TellerStep,
    select: Option<ActionHandle<Select>>,
    open: Option<ActionHandle<Open>>,
    deposit: Option<ActionHandle<Deposit>>,
    withdraw: Option<ActionHandle<Withdraw>>,
    close: Option<ActionHandle<Close>>,
    access: Option<Arc<script::bank::BankStandAccess>>,
}

impl TellerScript {
    fn new(facts: Arc<api::named_banks::NamedBankFacts>, live: Arc<Mutex<TellerLive>>) -> Self {
        Self {
            facts,
            live,
            step: TellerStep::Select,
            select: None,
            open: None,
            deposit: None,
            withdraw: None,
            close: None,
            access: None,
        }
    }

    fn blocked(&self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        let mut live = self.live.lock();
        if live.trace.outcome.is_none() {
            live.trace.failure = Some(message.clone());
            live.trace.outcome = Some(TellerOutcome::Failed(message.clone()));
            live.terminal_at = Some(Instant::now());
        }
        ScriptFlow::Blocked(ScriptFailure {
            code: "mage-bank-live-proof".into(),
            message: message.into(),
        })
    }
}

fn count_item(items: &[api::snapshot::ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn tile_tuple(tile: WorldTile) -> [i32; 3] {
    [tile.x, tile.z, tile.level]
}
fn observed_item_count(items: Option<&[api::snapshot::ItemView]>, id: i32) -> Option<i32> {
    items.map(|items| count_item(items, id))
}

impl Script for TellerScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.step == TellerStep::Done {
            return Ok(ScriptFlow::Complete);
        }
        let Some(here) = tick.cx.snapshot().here().map(|observed| observed.value) else {
            return Ok(ScriptFlow::Continue);
        };
        if self.live.lock().trace.started_at.is_none() {
            if here != MAGE_CELLAR {
                return Ok(self.blocked(format!(
                    "native teller proof started outside Mage cellar: {here:?}"
                )));
            }
            self.live.lock().trace.started_at = Some(tile_tuple(here));
        }

        match self.step {
            TellerStep::Select => {
                if self.select.is_none() {
                    let args = SelectArgs {
                        facts: Arc::clone(&self.facts),
                        from: MAGE_CELLAR,
                        preferences: BankPreferences {
                            use_mage_bank: true,
                            use_zanaris_bank: false,
                        },
                        allow_wilderness: true,
                        explicit: Some(Arc::from(MAGE_BANK_NAME)),
                    };
                    match tick.actions.begin::<Select>(args, &mut tick.cx) {
                        Ok(handle) => self.select = Some(handle),
                        Err(error) => {
                            return Ok(
                                self.blocked(format!("native Select begin failed: {error:?}"))
                            );
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                let result = tick
                    .actions
                    .poll(self.select.as_ref().expect("Select handle"), &mut tick.cx);
                match result {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => {
                        self.select = None;
                        Ok(self.blocked(format!("native Select failed: {error:?}")))
                    }
                    Poll::Ready(Ok(selected)) => {
                        self.select = None;
                        let Some(bank) = self.facts.banks().get(selected.bank_index as usize)
                        else {
                            return Ok(self.blocked(format!(
                                "native Select returned invalid bank index {}",
                                selected.bank_index
                            )));
                        };
                        let Some(access) = selected.access else {
                            return Ok(self.blocked(format!(
                                "native Select returned no production access metadata for {}",
                                bank.name
                            )));
                        };
                        let access_kind = access.kind.as_str().to_owned();
                        let access_npc = access.name.as_deref().map(str::to_owned);
                        let access_choice = access.choose.as_deref().map(str::to_owned);
                        let access_operation = bank
                            .definition
                            .and_then(|definition| definition.npc)
                            .map(|operation| operation.op.to_owned());
                        let access_op_code = access.stand_op;
                        let access_tile = tile_tuple(selected.access_tile);
                        {
                            let mut live = self.live.lock();
                            live.trace.selected_bank = Some(bank.name.to_owned());
                            live.trace.selected_kind = Some(format!("{:?}", selected.kind));
                            live.trace.access_kind = Some(access_kind.clone());
                            live.trace.access_npc = access_npc.clone();
                            live.trace.access_choice = access_choice.clone();
                            live.trace.access_operation = access_operation.clone();
                            live.trace.access_op_code = Some(access_op_code);
                            live.trace.access_tile = Some(access_tile);
                        }
                        if bank.name != MAGE_BANK_NAME
                            || selected.kind == script::bank::PickKind::NoCandidate
                            || access.kind != AccessKind::Teller
                            || access_npc.as_deref() != Some(MAGE_NPC_NAME)
                            || access_choice.as_deref() != Some(MAGE_BANK_CHOICE)
                            || access_operation.as_deref() != Some("Talk-to")
                        {
                            return Ok(self.blocked(format!(
                                "native Select did not resolve the Mage Arena teller metadata: bank={} kind={:?} access={access_kind:?} npc={access_npc:?} op={access_operation:?}/{access_op_code} choose={access_choice:?}",
                                bank.name, selected.kind
                            )));
                        }
                        self.access = Some(access);
                        self.step = TellerStep::Open;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            TellerStep::Open => {
                if self.open.is_none() {
                    let snapshot = tick.cx.snapshot();
                    let inventory = observed_item_count(
                        snapshot.inventory().map(|observed| observed.value),
                        LAW_RUNE_ID,
                    );
                    if inventory != Some(1) {
                        return Ok(self.blocked(format!(
                            "Mage cellar Start lacks exactly one seeded law rune: inventory={inventory:?}"
                        )));
                    }
                    self.live.lock().trace.inventory_law_before = inventory;
                    let Some(access) = self.access.as_ref().cloned() else {
                        return Ok(self.blocked("native Open has no selected Teller access"));
                    };
                    match tick
                        .actions
                        .begin::<Open>(OpenArgs { access }, &mut tick.cx)
                    {
                        Ok(handle) => self.open = Some(handle),
                        Err(error) => {
                            return Ok(self.blocked(format!("native Open begin failed: {error:?}")));
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                let result = tick
                    .actions
                    .poll(self.open.as_ref().expect("Open handle"), &mut tick.cx);
                match result {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => {
                        self.open = None;
                        Ok(self.blocked(format!("native Open failed: {error:?}")))
                    }
                    Poll::Ready(Ok(())) => {
                        self.open = None;
                        let snapshot = tick.cx.snapshot();
                        let loaded = snapshot.bank().is_some();
                        let bank = observed_item_count(
                            snapshot.bank().map(|observed| observed.value),
                            LAW_RUNE_ID,
                        );
                        let open = snapshot
                            .bank_session()
                            .is_some_and(|observed| observed.value.open);
                        {
                            let mut live = self.live.lock();
                            live.trace.open_loaded = loaded && open;
                            live.trace.bank_law_before = bank;
                        }
                        if !loaded || !open || bank.is_none() {
                            return Ok(self.blocked(format!(
                                "native Open returned without an observed loaded bank: loaded={loaded} open={open} law_rune={bank:?}"
                            )));
                        }
                        self.step = TellerStep::Deposit;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            TellerStep::Deposit => {
                if self.deposit.is_none() {
                    match tick.actions.begin::<Deposit>(
                        DepositArgs {
                            products: Arc::from([LAW_RUNE_ID]),
                            keep: Arc::from([]),
                        },
                        &mut tick.cx,
                    ) {
                        Ok(handle) => self.deposit = Some(handle),
                        Err(error) => {
                            return Ok(
                                self.blocked(format!("native Deposit begin failed: {error:?}"))
                            );
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                let result = tick
                    .actions
                    .poll(self.deposit.as_ref().expect("Deposit handle"), &mut tick.cx);
                match result {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => {
                        self.deposit = None;
                        Ok(self.blocked(format!("native Deposit failed: {error:?}")))
                    }
                    Poll::Ready(Ok(deposited)) => {
                        self.deposit = None;
                        let snapshot = tick.cx.snapshot();
                        let inventory = observed_item_count(
                            snapshot.inventory().map(|observed| observed.value),
                            LAW_RUNE_ID,
                        );
                        let bank = observed_item_count(
                            snapshot.bank().map(|observed| observed.value),
                            LAW_RUNE_ID,
                        );
                        let before = self.live.lock().trace.bank_law_before;
                        {
                            let mut live = self.live.lock();
                            live.trace.deposited = Some(deposited);
                            live.trace.inventory_law_after_deposit = inventory;
                            live.trace.bank_law_after_deposit = bank;
                        }
                        let expected_bank = before.and_then(|count| count.checked_add(1));
                        if deposited != 1 || inventory != Some(0) || bank != expected_bank {
                            return Ok(self.blocked(format!(
                                "native Deposit transfer was not observed: deposited={deposited} inventory={inventory:?} bank={bank:?} expected_bank={expected_bank:?}"
                            )));
                        }
                        self.step = TellerStep::Withdraw;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            TellerStep::Withdraw => {
                if self.withdraw.is_none() {
                    match tick.actions.begin::<Withdraw>(
                        WithdrawArgs {
                            withdrawals: Arc::from([Withdrawal {
                                id: LAW_RUNE_ID,
                                name: Arc::from("Law rune"),
                                target: 1,
                            }]),
                        },
                        &mut tick.cx,
                    ) {
                        Ok(handle) => self.withdraw = Some(handle),
                        Err(error) => {
                            return Ok(
                                self.blocked(format!("native Withdraw begin failed: {error:?}"))
                            );
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                let result = tick.actions.poll(
                    self.withdraw.as_ref().expect("Withdraw handle"),
                    &mut tick.cx,
                );
                match result {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => {
                        self.withdraw = None;
                        Ok(self.blocked(format!("native Withdraw failed: {error:?}")))
                    }
                    Poll::Ready(Ok(complete)) => {
                        self.withdraw = None;
                        let snapshot = tick.cx.snapshot();
                        let inventory = observed_item_count(
                            snapshot.inventory().map(|observed| observed.value),
                            LAW_RUNE_ID,
                        );
                        let bank = observed_item_count(
                            snapshot.bank().map(|observed| observed.value),
                            LAW_RUNE_ID,
                        );
                        let before = self.live.lock().trace.bank_law_before;
                        {
                            let mut live = self.live.lock();
                            live.trace.withdrew_complete = Some(complete);
                            live.trace.inventory_law_after_withdraw = inventory;
                            live.trace.bank_law_after_withdraw = bank;
                        }
                        if !complete || inventory != Some(1) || bank != before {
                            return Ok(self.blocked(format!(
                                "native Withdraw transfer was not observed: complete={complete} inventory={inventory:?} bank={bank:?} expected_bank={before:?}"
                            )));
                        }
                        self.step = TellerStep::Close;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            TellerStep::Close => {
                if self.close.is_none() {
                    match tick.actions.begin::<Close>((), &mut tick.cx) {
                        Ok(handle) => self.close = Some(handle),
                        Err(error) => {
                            return Ok(
                                self.blocked(format!("native Close begin failed: {error:?}"))
                            );
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                let result = tick
                    .actions
                    .poll(self.close.as_ref().expect("Close handle"), &mut tick.cx);
                match result {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => {
                        self.close = None;
                        Ok(self.blocked(format!("native Close failed: {error:?}")))
                    }
                    Poll::Ready(Ok(())) => {
                        self.close = None;
                        let snapshot = tick.cx.snapshot();
                        let final_open =
                            snapshot.bank_session().map(|observed| observed.value.open);
                        let final_tile = snapshot.here().map(|observed| tile_tuple(observed.value));
                        let inventory = observed_item_count(
                            snapshot.inventory().map(|observed| observed.value),
                            LAW_RUNE_ID,
                        );
                        {
                            let mut live = self.live.lock();
                            live.trace.closed_observed = final_open == Some(false);
                            live.trace.final_bank_open = final_open;
                            live.trace.final_tile = final_tile;
                            live.trace.final_inventory_law = inventory;
                        }
                        let validation = validate_teller_trace(&self.live.lock().trace);
                        if let Err(error) = validation {
                            return Ok(self.blocked(error));
                        }
                        self.live.lock().pass();
                        self.step = TellerStep::Done;
                        Ok(ScriptFlow::Complete)
                    }
                }
            }
            TellerStep::Done => Ok(ScriptFlow::Complete),
        }
    }
}
fn validate_teller_trace(trace: &TellerTrace) -> Result<(), String> {
    let expected_deposit_bank = trace.bank_law_before.and_then(|count| count.checked_add(1));
    let valid = trace.started_at == Some(tile_tuple(MAGE_CELLAR))
        && trace.selected_bank.as_deref() == Some(MAGE_BANK_NAME)
        && trace.selected_kind.as_deref() != Some("NoCandidate")
        && trace.access_kind.as_deref() == Some("npc")
        && trace.access_npc.as_deref() == Some(MAGE_NPC_NAME)
        && trace.access_choice.as_deref() == Some(MAGE_BANK_CHOICE)
        && trace.access_operation.as_deref() == Some("Talk-to")
        && trace.open_loaded
        && trace.inventory_law_before == Some(1)
        && trace.bank_law_before.is_some()
        && trace.deposited == Some(1)
        && trace.inventory_law_after_deposit == Some(0)
        && trace.bank_law_after_deposit == expected_deposit_bank
        && trace.withdrew_complete == Some(true)
        && trace.inventory_law_after_withdraw == Some(1)
        && trace.bank_law_after_withdraw == trace.bank_law_before
        && trace.closed_observed
        && trace.final_tile.is_some()
        && trace.final_bank_open == Some(false)
        && trace.final_inventory_law == Some(1);
    if valid {
        Ok(())
    } else {
        Err(format!(
            "native Mage Arena bank receipt did not prove Select/Open/Deposit/Withdraw/Close: {trace:#?}"
        ))
    }
}
type TellerProfile = (
    Arc<super::ServerProfile>,
    Arc<SharedClientTemplate>,
    Arc<api::game_data::SelectedGameData>,
    Arc<api::named_banks::NamedBankFacts>,
);

fn live_teller_profile(scratch: &Path) -> Result<TellerProfile, String> {
    let required_path = |key: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{key} must point at the local 289 fixture"))
    };
    let nav_pack = required_path("GATHERER_NAV_PACK")?;
    let engine_dir = required_path("GATHERER_ENGINE_DIR")?;
    let catalog_root = required_path("GATHERER_CATALOG_ROOT")?;
    for (name, path) in [
        ("GATHERER_NAV_PACK", &nav_pack),
        ("GATHERER_ENGINE_DIR", &engine_dir),
        ("GATHERER_CATALOG_ROOT", &catalog_root),
    ] {
        if !path.is_absolute() {
            return Err(format!("{name} must be absolute: {}", path.display()));
        }
    }
    let port = std::env::var("GATHERER_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("GATHERER_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2080);
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("GATHERER_NAV_FLAGS").map(PathBuf::from),
        engine_dir: Some(engine_dir),
        vault_path: Some(scratch.join("vault")),
        unpack_dir: Some(scratch.join("unpack")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.profile_class() != super::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("native teller proof requires a loopback local engine".into());
    }
    let selected = profile
        .game_data()
        .ok_or("native teller proof needs selected 289 game data")?;
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    let world = template
        .world()
        .ok_or("native teller proof needs the selected navigation world")?;
    if world.named_bank_facts().is_none() {
        world.bind_named_bank_facts(&selected)?;
    }
    let facts = Arc::clone(
        world
            .named_bank_facts()
            .ok_or("selected template has no bound named-bank facts")?,
    );
    let mage = facts
        .banks()
        .iter()
        .find(|bank| bank.name == MAGE_BANK_NAME)
        .ok_or("selected game data has no Mage Arena bank definition")?;
    let definition = mage
        .definition
        .ok_or("Mage Arena bank has no selected definition")?;
    let npc = definition
        .npc
        .ok_or("Mage Arena bank definition has no teller NPC")?;
    if npc.name != MAGE_NPC_NAME
        || npc.op != "Talk-to"
        || definition.choose != Some(MAGE_BANK_CHOICE)
    {
        return Err(format!(
            "unexpected Mage Arena teller metadata: npc={npc:?} choose={:?}",
            definition.choose
        ));
    }
    Ok((profile, template, selected, facts))
}

fn teller_evidence_dir(account: &str) -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR")
            .expect("live_mage_teller_native_real_play_receipt requires LIVE_EVIDENCE_DIR"),
    );
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    root.join(format!("gatherer_mage_teller_{account}_utc-{epoch}Z"))
}

pub(super) fn write_teller_png(client: &mut Client, path: &Path) -> Result<(), String> {
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("real Client render did not return a CPU PixMap".into());
    };
    let mut rgba = Vec::with_capacity(pixels.pixels.len() * 4);
    for pixel in &pixels.pixels {
        let pixel = *pixel;
        rgba.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            u8::MAX,
        ]);
    }
    let file = std::fs::File::create(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("write {} header: {error}", path.display()))?;
    writer
        .write_image_data(&rgba)
        .map_err(|error| format!("write {} pixels: {error}", path.display()))
}

fn save_teller_evidence(
    client: &mut Client,
    directory: &Path,
    account: &str,
    trace: &TellerTrace,
) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let step = if matches!(trace.outcome, Some(TellerOutcome::Passed)) {
        "01-final"
    } else {
        "FAIL-final"
    };
    let png_path = directory.join(format!("{epoch}Z_{step}.png"));
    let json_path = directory.join(format!("{epoch}Z_{step}.json"));
    let png_result = write_teller_png(client, &png_path);
    let outcome = match trace.outcome.as_ref() {
        Some(TellerOutcome::Passed) => "passed",
        Some(TellerOutcome::Failed(_)) => "failed",
        None => "incomplete",
    };
    let receipt = serde_json::json!({
        "scenario": "gatherer_mage_teller_native_real_play",
        "account": account,
        "origin": tile_tuple(MAGE_CELLAR),
        "seed": {
            "id": LAW_RUNE_ID,
            "count": 1,
            "classification": "unrelated consumable; not a gathered product",
        },
        "started_at": trace.started_at,
        "selection": {
            "bank": trace.selected_bank,
            "kind": trace.selected_kind,
            "access_kind": trace.access_kind,
            "npc": trace.access_npc,
            "choose": trace.access_choice,
            "operation": trace.access_operation,
            "op_code": trace.access_op_code,
            "access_tile": trace.access_tile,
        },
        "open_loaded": trace.open_loaded,
        "deposit": {
            "observed_count": trace.deposited,
            "inventory_law_runes_after": trace.inventory_law_after_deposit,
            "bank_law_runes_after": trace.bank_law_after_deposit,
            "bank_law_runes_before": trace.bank_law_before,
        },
        "withdraw": {
            "complete": trace.withdrew_complete,
            "inventory_law_runes_after": trace.inventory_law_after_withdraw,
            "bank_law_runes_after": trace.bank_law_after_withdraw,
        },
        "close": {
            "observed": trace.closed_observed,
            "bank_open_after": trace.final_bank_open,
        },
        "final": {
            "tile": trace.final_tile,
            "inventory_law_runes": trace.final_inventory_law,
            "live_tile": trace.live_tile,
            "live_bank_open": trace.live_bank_open,
            "live_inventory_law_runes": trace.live_inventory_law,
        },
        "outcome": outcome,
        "failure": trace.failure,
        "image": {
            "format": "PNG",
            "path": png_path,
            "renderer": "real Client CpuPix3D framebuffer",
            "error": png_result.as_ref().err(),
        },
    });
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| format!("encode {}: {error}", json_path.display()))?;
    std::fs::write(&json_path, bytes)
        .map_err(|error| format!("write {}: {error}", json_path.display()))?;
    png_result
}
fn teller_frame(client: &mut Client, shared: &Mutex<TellerLive>, account: &str) {
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    let now = Instant::now();
    let capture = {
        let mut live = shared.lock();
        if client.ingame && client.scene_state == 2 {
            live.trace.live_tile = snapshot.tile().map(|(x, z, level)| [x, z, level]);
            live.trace.live_bank_open = Some(snapshot.bank_component_id() >= 0);
            live.trace.live_inventory_law = Some(count_item(snapshot.inventory(), LAW_RUNE_ID));
        }
        if live.trace.outcome.is_none()
            && now.duration_since(live.started) > Duration::from_secs(180)
        {
            let message = format!(
                "Mage cellar setup timed out in {:?}; tile={:?}",
                live.phase,
                snapshot.tile()
            );
            live.fail(message);
        }

        match live.phase {
            TellerPrep::WaitLogin => {
                if client.ingame && client.scene_state == 2 && snapshot.local_player().is_some() {
                    live.phase = TellerPrep::SkipTutorial;
                    live.last_action = now;
                }
            }
            TellerPrep::SkipTutorial
                if now.duration_since(live.last_action) >= Duration::from_millis(400) =>
            {
                let _ = interact::cheat(client, "setvar tutorial 1000");
                live.last_action = now;
                live.phase = TellerPrep::Logout;
            }
            TellerPrep::Logout
                if now.duration_since(live.last_action) >= Duration::from_secs(2) =>
            {
                let ifaces = Arc::clone(&client.ifaces);
                if interact::logout(client, &ifaces) {
                    live.offline_seen = false;
                    live.phase = TellerPrep::WaitRelog;
                    live.last_action = now;
                } else {
                    live.fail("tutorial relog logout interface was unavailable".into());
                }
            }
            TellerPrep::WaitRelog => {
                if !client.ingame {
                    live.offline_seen = true;
                } else if live.offline_seen
                    && client.scene_state == 2
                    && snapshot.local_player().is_some()
                    && snapshot.inventory_size() > 0
                {
                    live.phase = TellerPrep::Teleport;
                    live.last_action = now;
                }
            }
            TellerPrep::Teleport
                if now.duration_since(live.last_action) >= Duration::from_millis(400) =>
            {
                let _ = interact::cheat(client, &tele_args(MAGE_CELLAR));
                live.last_action = now;
                live.phase = TellerPrep::WaitCellar;
            }
            TellerPrep::WaitCellar => {
                if snapshot.tile() == Some((MAGE_CELLAR.x, MAGE_CELLAR.z, MAGE_CELLAR.level))
                    && snapshot.bank_component_id() < 0
                {
                    live.phase = TellerPrep::ClearInventory;
                    live.last_action = now;
                } else if snapshot.bank_component_id() >= 0
                    && now.duration_since(live.last_action) >= Duration::from_secs(2)
                {
                    let _ = interact::Interactions::new(&snapshot, client).close_modal();
                    live.last_action = now;
                }
            }
            TellerPrep::ClearInventory
                if now.duration_since(live.last_action) >= Duration::from_millis(400) =>
            {
                let _ = interact::cheat(client, "~clearinv");
                live.last_action = now;
                live.phase = TellerPrep::WaitClear;
            }
            TellerPrep::WaitClear => {
                if snapshot.inventory().is_empty() {
                    live.phase = TellerPrep::SeedLawRune;
                    live.last_action = now;
                }
            }
            TellerPrep::SeedLawRune
                if now.duration_since(live.last_action) >= Duration::from_millis(400) =>
            {
                let _ = interact::cheat(client, "give lawrune 1");
                live.last_action = now;
                live.phase = TellerPrep::WaitSeed;
            }
            TellerPrep::WaitSeed => {
                let law_runes = count_item(snapshot.inventory(), LAW_RUNE_ID);
                let only_law_runes = snapshot
                    .inventory()
                    .iter()
                    .all(|item| item.def.id == LAW_RUNE_ID);
                if snapshot.tile() == Some((MAGE_CELLAR.x, MAGE_CELLAR.z, MAGE_CELLAR.level))
                    && law_runes == 1
                    && only_law_runes
                    && snapshot.bank_component_id() < 0
                {
                    live.phase = TellerPrep::Ready;
                }
            }
            TellerPrep::Ready | TellerPrep::Failed => {}
            TellerPrep::SkipTutorial
            | TellerPrep::Logout
            | TellerPrep::Teleport
            | TellerPrep::ClearInventory
            | TellerPrep::SeedLawRune => {}
        }

        if matches!(live.trace.outcome.as_ref(), Some(TellerOutcome::Passed))
            && live
                .terminal_at
                .is_some_and(|at| now.duration_since(at) >= Duration::from_secs(8))
            && !(live.trace.live_tile == live.trace.final_tile
                && live.trace.live_bank_open == Some(false)
                && live.trace.live_inventory_law == Some(1))
        {
            let message = format!(
                "final live frame did not show closed Mage bank at the cellar with one law rune: {:?}",
                live.trace
            );
            live.fail(message);
        }
        let failure_ready = matches!(live.trace.outcome.as_ref(), Some(TellerOutcome::Failed(_)));
        let pass_ready = matches!(live.trace.outcome.as_ref(), Some(TellerOutcome::Passed))
            && live.trace.live_tile == live.trace.final_tile
            && live.trace.live_bank_open == Some(false)
            && live.trace.live_inventory_law == Some(1);
        if !live.capture_started && (failure_ready || pass_ready) {
            live.capture_started = true;
            Some((live.evidence_dir.clone(), live.trace.clone()))
        } else {
            None
        }
    };
    if let Some((directory, trace)) = capture {
        let result = save_teller_evidence(client, &directory, account, &trace);
        let mut live = shared.lock();
        live.capture_written = true;
        if let Err(error) = result {
            live.capture_error = Some(error);
        }
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT and a local 289 engine"]
fn live_mage_teller_native_real_play_receipt() {
    assert!(
        live(),
        "live_mage_teller_native_real_play_receipt requires LIVE=1"
    );
    let home = install_throwaway_home();
    let (_profile, template, selected, facts) =
        live_teller_profile(&home.path).expect("load selected local 289 teller facts");
    let account = super::mint_live_names(1)
        .pop()
        .expect("mint one disposable Mage Arena teller account");
    let evidence_dir = teller_evidence_dir(&account);
    std::fs::create_dir_all(&evidence_dir).expect("create Mage Arena evidence directory");
    let state = Arc::new(Mutex::new(TellerLive::new(evidence_dir.clone())));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_004,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, _, _| teller_frame(client, &frame_state, &frame_account),
    )
    .expect("start real Play for Mage Arena teller proof");
    assert_eq!(
        play.named_banks().banks(),
        facts.banks(),
        "Play and the native test script must use the same selected named-bank facts"
    );
    let start = play.script_start_handle();
    let deadline = Instant::now() + Duration::from_secs(210);
    let capture_deadline = deadline + Duration::from_secs(20);
    let mut script_started = false;

    loop {
        let now = Instant::now();
        let should_start = {
            let mut live = state.lock();
            if live.trace.outcome.is_none() && now >= deadline {
                let message = format!(
                    "Mage cellar preparation/native action timed out in {:?}",
                    live.phase
                );
                live.fail(message);
            }
            live.phase == TellerPrep::Ready && !script_started
        };
        if should_start {
            script_started = true;
            match start.start_test_script(
                &account,
                Box::new(TellerScript::new(Arc::clone(&facts), Arc::clone(&state))),
                Some(Arc::clone(&selected)),
            ) {
                Ok(run) => {
                    if play.script_native_run(&account) == Some(run) {
                        play.wake(&account);
                    } else {
                        state
                            .lock()
                            .fail("real Play did not publish the started native teller run".into());
                    }
                }
                Err(error) => state.lock().fail(format!(
                    "real Play rejected the native teller script: {error}"
                )),
            }
        }

        let completed_capture = {
            let live = state.lock();
            live.capture_written
                .then(|| (live.trace.clone(), live.capture_error.clone()))
        };
        if let Some((trace, capture_error)) = completed_capture {
            if let Some(error) = capture_error {
                panic!("Mage Arena live evidence capture failed: {error}; trace={trace:#?}");
            }
            match trace.outcome.clone() {
                Some(TellerOutcome::Passed) => {
                    validate_teller_trace(&trace)
                        .unwrap_or_else(|error| panic!("Mage Arena receipt failed: {error}"));
                    assert!(
                        script_started,
                        "native Teller receipt completed without starting its test Script"
                    );
                    println!(
                        "PASS: live_mage_teller_native_real_play_receipt evidence={}",
                        evidence_dir.display()
                    );
                    return;
                }
                Some(TellerOutcome::Failed(message)) => {
                    panic!(
                        "FAIL: live_mage_teller_native_real_play_receipt: {message}; evidence={}",
                        evidence_dir.display()
                    );
                }
                None => panic!(
                    "FAIL: Mage Arena evidence was saved without a terminal result: {trace:#?}"
                ),
            }
        }
        if now >= capture_deadline {
            let live = state.lock();
            panic!(
                "FAIL: Mage Arena teller proof did not save final/failure evidence; phase={:?} trace={:#?}",
                live.phase, live.trace
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
