//! Live BankBudget fetch at real packed banks, from the street.
//!
//! Every leg runs a real planned session — Walk, Open, Withdraw, Wear (which
//! closes the bank on 289) — through the production pump: the script pump
//! ([`super::step_nav_bot`]) or the panel/TUI pump
//! ([`super::step_walk_arm_follow`]), once per player tick. Legs cover
//! Varrock West, Draynor, Falador East and Al Kharid, each with the booth
//! (a booth-only stand table, so Open can only click a booth) and banker
//! (the real table, whose Open tries the tellers first), through both owners.
//! A leg passes only when the bronze dagger seeded into the bank ends up worn,
//! the seeded bones remain carried, and Open used the expected access: the
//! booth next to the player, or a banker.
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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::named_banks::BankPreferences;
use script::bank::{
    AccessKind, Close, Deposit, DepositArgs, Open, OpenArgs, Select, SelectArgs, Withdraw,
    WithdrawArgs, Withdrawal,
};
use script::native::walk::Walk;
use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};

use super::script_runtime::BankPickWorkObservation;
use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, MiniMenuAction};
use nav::bank_fetch::{plan_bank_fetch, BankStep};
use nav::pack::BankAccess;
use nav::router::{FindOptions, Leg, MissingReq, Route};
use nav::world::NavWorld;
use nav::WorldState;
use parking_lot::Mutex;
use script::work_admission::WorkService;
use vault::{Profile, ProfileSettings};

use super::evidence_writer::{
    write_failure_sidecar, EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
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
    TutorialReseed,
    WaitTutorialReseed,
    Tele { leg: usize },
    WaitStreet { leg: usize },
    Seed { leg: usize, next: usize },
    WaitSeeded { leg: usize },
    Run { leg: usize },
    Pass,
    Fail(String),
}

/// The Open pump's actual menu send and the player tick that emitted it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenSend {
    action: (i32, i32, i32, i32),
    from: (i32, i32, i32),
    tick: u64,
}

fn open_send_after_pump(
    open_step_active: bool,
    access: Access,
    action: (i32, i32, i32, i32),
    from: (i32, i32, i32),
    tick: u64,
) -> Option<OpenSend> {
    (open_step_active && is_open_action(action.0, access)).then_some(OpenSend {
        action,
        from,
        tick,
    })
}
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
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
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
fn is_open_action(action: i32, access: Access) -> bool {
    match access {
        Access::Booth => is_loc_op(action),
        Access::Banker => is_npc_op(action),
    }
}

#[cfg(test)]
mod open_witness_tests {
    use super::*;

    #[test]
    fn expected_open_send_is_captured_for_each_access() {
        let from = (3127, 8758, 0);
        for (access, action_code) in [
            (Access::Booth, MiniMenuAction::OP_LOC1),
            (Access::Banker, MiniMenuAction::OP_NPC1),
        ] {
            let action = (action_code, 0, 3128, 8758);
            let witness = open_send_after_pump(true, access, action, from, 17)
                .expect("the player's expected Open send");
            assert_eq!(
                witness,
                OpenSend {
                    action,
                    from,
                    tick: 17
                }
            );
        }
    }

    #[test]
    fn missing_or_stale_open_send_is_not_a_witness() {
        assert!(
            open_send_after_pump(
                false,
                Access::Booth,
                (MiniMenuAction::OP_LOC1, 0, 3128, 8758),
                (0, 0, 0),
                17,
            )
            .is_none(),
            "an Open from before this leg cannot witness its Open"
        );
        assert!(
            open_send_after_pump(true, Access::Booth, (-1, 0, 0, 0), (0, 0, 0), 17).is_none(),
            "a missing menu send cannot witness Open"
        );
        assert!(
            open_send_after_pump(
                true,
                Access::Booth,
                (MiniMenuAction::INV_BUTTON1, 0, 0, 0),
                (3185, 3436, 0),
                18,
            )
            .is_none(),
            "Withdraw observed while Open is the old front is not an Open send"
        );
        assert!(
            open_send_after_pump(
                true,
                Access::Banker,
                (MiniMenuAction::OP_LOC1, 0, 3128, 8758),
                (0, 0, 0),
                18,
            )
            .is_none(),
            "a booth action cannot witness a banker Open"
        );
    }
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
            let _ = interact::cheat(client, scenario::tutorial::TUTORIAL_SETVAR);
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
                g.phase = Phase::TutorialReseed;
            } else if now.duration_since(g.last_cheat) > Duration::from_secs(60) {
                g.phase = Phase::Fail("no inv tab 60s after the relog".into());
            }
        }
        Phase::TutorialReseed => {
            // Fresh-account post-relog reseed: the kit-close queue has
            // already run in the new session, so this `setvar` sticks. The
            // dagger wear below needs `tutorial > 400` for the combat tab.
            let baseline = scenario::tutorial::chat_baseline(&g.snap);
            let reseed = scenario::tutorial::PostRelogTutorial::new(baseline);
            reseed.send_reseed(client);
            g.tutorial_reseed = Some(reseed);
            g.last_cheat = now;
            g.phase = Phase::WaitTutorialReseed;
        }
        Phase::WaitTutorialReseed => {
            let reseed = g
                .tutorial_reseed
                .as_ref()
                .expect("tutorial reseed armed before waiting");
            match reseed.check(&g.snap) {
                Ok(true) => {
                    println!("{}", scenario::tutorial::confirmation_log());
                    g.phase = Phase::Tele { leg: 0 };
                    g.last_cheat = now;
                }
                Ok(false) => {}
                Err(error) => {
                    g.phase = Phase::Fail(error);
                }
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
            let Ok(fetch) = plan_bank_fetch(
                &[MissingReq::WearAny { ids: vec![DAGGER] }],
                &state,
                &[(DAGGER, 1)],
                world.banks(),
                from,
                &world.collision,
                &world.graph,
                FindOptions::default(),
                &[],
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
                final_route: crate::walk_plan::RoutePath::legacy(Arc::new(Route {
                    legs: vec![Leg::Walk { tiles: vec![from] }],
                    dest: from,
                    ticks: 1.0,
                })),
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
        // Menu actions persist across client ticks. Clear the action slot so
        // the witness below can only use an interaction emitted by this pump.
        client.menu_action[0] = -1;
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
                    None,
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
        let front_after = g.session_front(spec.owner).flatten();
        let action = (
            client.menu_action[0],
            client.menu_param_a[0],
            client.menu_param_b[0],
            client.menu_param_c[0],
        );
        if let Some(open) = open_send_after_pump(
            front_before == Some(BankStep::Open) || front_after == Some(BankStep::Open),
            spec.access,
            action,
            here,
            key.0,
        ) {
            println!(
                "live_bank_fetch: {label}: Open send witness player_tick={} action={} args=({},{},{}) from={:?}",
                open.tick,
                open.action.0,
                open.action.1,
                open.action.2,
                open.action.3,
                open.from,
            );
            g.run.open = Some(open);
        } else if front_before == Some(BankStep::Open) && action.0 != -1 {
            println!(
                "live_bank_fetch: {label}: ignored non-Open witness player_tick={} action={}",
                key.0, action.0
            );
        } else if front_after == Some(BankStep::Open) {
            println!(
                "live_bank_fetch: {label}: no expected Open action player_tick={} action={}",
                key.0, action.0
            );
        }
        if front_after != g.run.front {
            println!(
                "live_bank_fetch: {label}: {:?} -> {front_after:?} here={here:?} pumps={} player_tick={} bank_com={} inventory={:?} bank_side={:?} worn={:?}",
                g.run.front,
                g.run.pumps,
                key.0,
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
    // The equipment observation can precede the bank-close observation by a
    // player tick. Keep the close gate active until it lands or LEG_TIMEOUT.
    if g.snap.bank_component_id() >= 0 {
        println!(
            "live_bank_fetch: {label}: waiting for bank-close observation player_tick={}",
            client.gens.player
        );
        return;
    }
    let Some(open) = g.run.open else {
        g.phase = Phase::Fail(format!("{label}: no Open send observed"));
        return;
    };
    let (action, a, b, c) = open.action;
    let from = open.from;
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
        "{label}: worn dagger, bones retained, bank closed; Open tick={} used {used}; {} pumps, {:.1}s",
        open.tick,
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

#[cfg_attr(test, test)]
#[cfg_attr(test, ignore = "requires LIVE=1 and the tunnelled local 289 engine")]
pub fn live_bankbudget_fetch_from_the_street() {
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
        tutorial_reseed: None,
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
    /// Shared background evidence writer; the hook submits the rendered
    /// frame and marks the capture written only once the files are durable.
    writer: Arc<EvidenceWriter>,
    capture_job: Option<EvidenceJob>,
}

impl TellerLive {
    fn new(evidence_dir: PathBuf, writer: Arc<EvidenceWriter>) -> Self {
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
            writer,
            capture_job: None,
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
                        options: script::native::WalkOptions {
                            allow_wilderness: script::native::WalkBit::Allow,
                            ..Default::default()
                        },
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
                            access: None,
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
                            access: None,
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

/// The slot-thread half of a teller capture: render (needs the client) into
/// native pixmap words. Encode and file writes happen on the shared
/// background evidence writer, so this hook keeps only the render hitch.
pub(super) struct TellerFrame {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: Vec<i32>,
}

pub(super) fn render_teller_frame(client: &mut Client) -> Result<TellerFrame, String> {
    let render_start = Instant::now();
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("real Client render did not return a CPU PixMap".into());
    };
    let render_ms = render_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "capture-render site=teller {}x{} render_ms={:.1}",
        pixels.width, pixels.height, render_ms,
    );
    Ok(TellerFrame {
        width: pixels.width as u32,
        height: pixels.height as u32,
        pixels: pixels.pixels,
    })
}

fn save_teller_evidence(
    client: &mut Client,
    directory: &Path,
    account: &str,
    trace: &TellerTrace,
    writer: &EvidenceWriter,
) -> Result<EvidenceJob, String> {
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
            "error": serde_json::Value::Null,
        },
    });
    let sidecar = EvidenceSidecar {
        path: json_path,
        receipt,
        patch_error: Some(Box::new(|receipt, error| {
            receipt["image"]["error"] = serde_json::Value::String(error.to_owned());
        })),
    };
    match render_teller_frame(client) {
        Ok(frame) => Ok(writer.submit(EvidenceRequest {
            png_path,
            width: frame.width,
            height: frame.height,
            pixels: frame.pixels,
            color: PngColor::Rgba,
            sidecar: Some(sidecar),
        })),
        // Render needs the client, so its failure is reported inline; the
        // sidecar still lands with the error recorded, as before.
        Err(error) => Err(write_failure_sidecar(sidecar, &error)),
    }
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
                let _ = interact::cheat(client, scenario::tutorial::TUTORIAL_SETVAR);
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
        let writer = shared.lock().writer.clone();
        match save_teller_evidence(client, &directory, account, &trace, &writer) {
            Ok(job) => {
                shared.lock().capture_job = Some(job);
            }
            Err(error) => {
                let mut live = shared.lock();
                live.capture_written = true;
                live.capture_error = Some(error);
            }
        }
    }
    // The driver loop already polls `capture_written` under the cell
    // deadline; this marks it only once the background writer has the PNG
    // and sidecar durable, which is the bounded cell-end flush.
    let completed = {
        let live = shared.lock();
        if live.capture_written {
            None
        } else {
            live.capture_job
                .clone()
                .and_then(|job| live.writer.poll(&job))
        }
    };
    if let Some(outcome) = completed {
        let mut live = shared.lock();
        live.capture_written = true;
        live.capture_error = outcome.error;
    }
}

#[cfg_attr(test, test)]
#[cfg_attr(
    test,
    ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT and a local 289 engine"
)]
pub fn live_mage_teller_native_real_play_receipt() {
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
    let writer = Arc::new(EvidenceWriter::new(
        super::evidence_writer::DEFAULT_QUEUE_BOUND,
    ));
    let state = Arc::new(Mutex::new(TellerLive::new(
        evidence_dir.clone(),
        Arc::clone(&writer),
    )));
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
const BANK_PICK_BURST_GUESTS: usize = 10;
const BANK_PICK_BURST_FULL_JOBS: usize = 64;
const BANK_PICK_BURST_HOLDERS: usize = BANK_PICK_BURST_FULL_JOBS - 2;
const BANK_PICK_BURST_BANK: &str = "Varrock West";
const BANK_PICK_BURST_ORIGIN: WorldTile = WorldTile {
    x: 3105,
    z: 3250,
    level: 0,
};
const BANK_PICK_BURST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BankPickBurstPrep {
    WaitLogin,
    SkipTutorial,
    Logout,
    WaitRelog,
    Teleport,
    WaitStreet,
    ClearInventory,
    WaitClear,
    SeedLawRune,
    WaitSeed,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Default)]
struct BankPickBurstTrace {
    initial_scene2_at_ms: Option<u128>,
    ready_scene2_at_ms: Option<u128>,
    ready_tile: Option<[i32; 3]>,
    ready_law_runes: Option<i32>,
    query_started_at_ms: Option<u128>,
    query_submitted_at_ms: Option<u128>,
    query_submitted_at: Option<Instant>,
    query_completed_at_ms: Option<u128>,
    query_from: Option<[i32; 3]>,
    selected_index: Option<u16>,
    walk_started_at_ms: Option<u128>,
    walk_completed_at_ms: Option<u128>,
    walk_end: Option<String>,
    terminal_at_ms: Option<u128>,
}

#[derive(Clone)]
struct BankPickBurstGuest {
    phase: BankPickBurstPrep,
    last_action: Instant,
    offline_seen: bool,
    trace: TellerTrace,
    burst: BankPickBurstTrace,
}

impl BankPickBurstGuest {
    fn new() -> Self {
        Self {
            phase: BankPickBurstPrep::WaitLogin,
            last_action: Instant::now() - Duration::from_secs(1),
            offline_seen: false,
            trace: TellerTrace::default(),
            burst: BankPickBurstTrace::default(),
        }
    }

    fn fail(&mut self, message: String, started: Instant) {
        if self.trace.outcome.is_none() {
            self.trace.failure = Some(message.clone());
            self.trace.outcome = Some(TellerOutcome::Failed(message));
            self.burst.terminal_at_ms = Some(started.elapsed().as_millis());
        }
        self.phase = BankPickBurstPrep::Failed;
    }

    fn pass(&mut self, started: Instant) {
        self.trace.outcome = Some(TellerOutcome::Passed);
        self.burst.terminal_at_ms = Some(started.elapsed().as_millis());
    }
}

fn send_bank_pick_burst_cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture cheat was not sent: {command}"))
    }
}

fn bank_pick_burst_frame(
    client: &mut Client,
    account: &str,
    guests: &Mutex<HashMap<String, Arc<Mutex<BankPickBurstGuest>>>>,
    started: Instant,
) {
    let Some(guest) = guests.lock().get(account).cloned() else {
        return;
    };
    let now = Instant::now();
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    let mut guest = guest.lock();
    if guest.trace.outcome.is_some() {
        return;
    }
    if client.ingame && client.scene_state == 2 {
        guest
            .burst
            .initial_scene2_at_ms
            .get_or_insert_with(|| started.elapsed().as_millis());
    }
    if now.duration_since(started) >= BANK_PICK_BURST_TIMEOUT {
        guest.fail(
            format!("ten-guest run exceeded {BANK_PICK_BURST_TIMEOUT:?}"),
            started,
        );
        return;
    }
    let result = match guest.phase {
        BankPickBurstPrep::WaitLogin => {
            if client.ingame && client.scene_state == 2 && snapshot.local_player().is_some() {
                guest.phase = BankPickBurstPrep::SkipTutorial;
                guest.last_action = now;
            }
            Ok(())
        }
        BankPickBurstPrep::SkipTutorial
            if now.duration_since(guest.last_action) >= Duration::from_millis(400) =>
        {
            send_bank_pick_burst_cheat(client, scenario::tutorial::TUTORIAL_SETVAR).map(|()| {
                guest.phase = BankPickBurstPrep::Logout;
                guest.last_action = now;
            })
        }
        BankPickBurstPrep::Logout
            if now.duration_since(guest.last_action) >= Duration::from_secs(2) =>
        {
            let ifaces = Arc::clone(&client.ifaces);
            if interact::logout(client, &ifaces) {
                guest.offline_seen = false;
                guest.phase = BankPickBurstPrep::WaitRelog;
                guest.last_action = now;
                Ok(())
            } else {
                Err("tutorial relog logout interface was unavailable".into())
            }
        }
        BankPickBurstPrep::WaitRelog => {
            if !client.ingame {
                guest.offline_seen = true;
            } else if guest.offline_seen
                && client.scene_state == 2
                && snapshot.local_player().is_some()
                && snapshot.inventory_size() > 0
            {
                guest.phase = BankPickBurstPrep::Teleport;
                guest.last_action = now;
            }
            Ok(())
        }
        BankPickBurstPrep::Teleport
            if now.duration_since(guest.last_action) >= Duration::from_millis(400) =>
        {
            send_bank_pick_burst_cheat(client, &tele_args(BANK_PICK_BURST_ORIGIN)).map(|()| {
                guest.phase = BankPickBurstPrep::WaitStreet;
                guest.last_action = now;
            })
        }
        BankPickBurstPrep::WaitStreet => {
            if !client.ingame || client.scene_state != 2 {
                Ok(())
            } else if snapshot.tile()
                == Some((
                    BANK_PICK_BURST_ORIGIN.x,
                    BANK_PICK_BURST_ORIGIN.z,
                    BANK_PICK_BURST_ORIGIN.level,
                ))
                && snapshot.bank_component_id() < 0
            {
                guest.phase = BankPickBurstPrep::ClearInventory;
                guest.last_action = now;
                Ok(())
            } else if snapshot.bank_component_id() >= 0 {
                Err("Draynor street preparation unexpectedly opened a bank".into())
            } else {
                Ok(())
            }
        }
        BankPickBurstPrep::ClearInventory
            if now.duration_since(guest.last_action) >= Duration::from_millis(400) =>
        {
            send_bank_pick_burst_cheat(client, "~clearinv").map(|()| {
                guest.phase = BankPickBurstPrep::WaitClear;
                guest.last_action = now;
            })
        }
        BankPickBurstPrep::WaitClear => {
            if snapshot.inventory().is_empty() {
                guest.phase = BankPickBurstPrep::SeedLawRune;
                guest.last_action = now;
            }
            Ok(())
        }
        BankPickBurstPrep::SeedLawRune
            if now.duration_since(guest.last_action) >= Duration::from_millis(400) =>
        {
            send_bank_pick_burst_cheat(client, "give lawrune 1").map(|()| {
                guest.phase = BankPickBurstPrep::WaitSeed;
                guest.last_action = now;
            })
        }
        BankPickBurstPrep::WaitSeed => {
            let law_runes = count_item(snapshot.inventory(), LAW_RUNE_ID);
            let only_law_runes = snapshot
                .inventory()
                .iter()
                .all(|item| item.def.id == LAW_RUNE_ID);
            if client.ingame
                && client.scene_state == 2
                && snapshot.tile()
                    == Some((
                        BANK_PICK_BURST_ORIGIN.x,
                        BANK_PICK_BURST_ORIGIN.z,
                        BANK_PICK_BURST_ORIGIN.level,
                    ))
                && law_runes == 1
                && only_law_runes
                && snapshot.bank_component_id() < 0
            {
                guest.phase = BankPickBurstPrep::Ready;
                guest.burst.ready_scene2_at_ms = Some(started.elapsed().as_millis());
                guest.burst.ready_tile = snapshot.tile().map(|(x, z, level)| [x, z, level]);
                guest.burst.ready_law_runes = Some(law_runes);
            }
            Ok(())
        }
        BankPickBurstPrep::Ready | BankPickBurstPrep::Failed => Ok(()),
        BankPickBurstPrep::SkipTutorial
        | BankPickBurstPrep::Logout
        | BankPickBurstPrep::Teleport
        | BankPickBurstPrep::ClearInventory
        | BankPickBurstPrep::SeedLawRune => Ok(()),
    };
    if let Err(error) = result {
        guest.fail(error, started);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BankPickBurstStep {
    Gate,
    Select,
    Walk,
    Open,
    Deposit,
    Withdraw,
    Close,
    Done,
}

struct BankPickBurstScript {
    account: String,
    interaction_account: String,
    facts: Arc<api::named_banks::NamedBankFacts>,
    guest: Arc<Mutex<BankPickBurstGuest>>,
    gate: Arc<AtomicBool>,
    started: Instant,
    step: BankPickBurstStep,
    select: Option<ActionHandle<Select>>,
    walk: Option<ActionHandle<Walk>>,
    open: Option<ActionHandle<Open>>,
    deposit: Option<ActionHandle<Deposit>>,
    withdraw: Option<ActionHandle<Withdraw>>,
    close: Option<ActionHandle<Close>>,
    access: Option<Arc<script::bank::BankStandAccess>>,
    access_tile: Option<WorldTile>,
}

impl BankPickBurstScript {
    fn new(
        account: String,
        interaction_account: String,
        facts: Arc<api::named_banks::NamedBankFacts>,
        guest: Arc<Mutex<BankPickBurstGuest>>,
        gate: Arc<AtomicBool>,
        started: Instant,
    ) -> Self {
        Self {
            account,
            interaction_account,
            facts,
            guest,
            gate,
            started,
            step: BankPickBurstStep::Gate,
            select: None,
            walk: None,
            open: None,
            deposit: None,
            withdraw: None,
            close: None,
            access: None,
            access_tile: None,
        }
    }

    fn blocked(&self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        self.guest.lock().fail(message.clone(), self.started);
        ScriptFlow::Blocked(ScriptFailure {
            code: "bank-pick-ten-guest-live-proof".into(),
            message: message.into(),
        })
    }

    fn query_complete(&self) {
        self.guest.lock().pass(self.started);
    }
}

impl Script for BankPickBurstScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.step == BankPickBurstStep::Done {
            return Ok(ScriptFlow::Complete);
        }
        if self.step == BankPickBurstStep::Gate {
            if !self.gate.load(Ordering::Acquire) {
                return Ok(ScriptFlow::Continue);
            }
            self.step = BankPickBurstStep::Select;
        }
        let Some(here) = tick.cx.snapshot().here().map(|observed| observed.value) else {
            return Ok(ScriptFlow::Continue);
        };
        match self.step {
            BankPickBurstStep::Select => {
                if self.select.is_none() {
                    let args = SelectArgs {
                        facts: Arc::clone(&self.facts),
                        from: here,
                        preferences: BankPreferences::default(),
                        options: script::native::WalkOptions::default(),
                        explicit: Some(Arc::from(BANK_PICK_BURST_BANK)),
                    };
                    {
                        let mut guest = self.guest.lock();
                        guest.burst.query_started_at_ms = Some(self.started.elapsed().as_millis());
                        guest.burst.query_from = Some(tile_tuple(here));
                        guest.trace.started_at = Some(tile_tuple(here));
                    }
                    match tick.actions.begin::<Select>(args, &mut tick.cx) {
                        Ok(handle) => self.select = Some(handle),
                        Err(error) => {
                            return Ok(self.blocked(format!(
                                "native Select begin failed for {}: {error:?}",
                                self.account
                            )));
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.select.as_ref().expect("Select handle"), &mut tick.cx)
                {
                    Poll::Pending => {
                        let mut guest = self.guest.lock();
                        guest
                            .burst
                            .query_submitted_at
                            .get_or_insert_with(Instant::now);
                        guest
                            .burst
                            .query_submitted_at_ms
                            .get_or_insert_with(|| self.started.elapsed().as_millis());
                        Ok(ScriptFlow::Continue)
                    }
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
                        {
                            let mut guest = self.guest.lock();
                            guest.burst.query_completed_at_ms =
                                Some(self.started.elapsed().as_millis());
                            guest.burst.selected_index = Some(selected.bank_index);
                            guest.trace.selected_bank = Some(bank.name.to_owned());
                            guest.trace.selected_kind = Some(format!("{:?}", selected.kind));
                            guest.trace.access_kind = Some(access.kind.as_str().to_owned());
                            guest.trace.access_npc = access.name.as_deref().map(str::to_owned);
                            guest.trace.access_choice = access.choose.as_deref().map(str::to_owned);
                            guest.trace.access_operation = bank
                                .definition
                                .and_then(|definition| definition.npc)
                                .map(|operation| operation.op.to_owned());
                            guest.trace.access_op_code = Some(access.stand_op);
                            guest.trace.access_tile = Some(tile_tuple(selected.access_tile));
                        }
                        if bank.name != BANK_PICK_BURST_BANK
                            || selected.kind != script::bank::PickKind::Reachable
                        {
                            return Ok(self.blocked(format!(
                                "native Select did not resolve a reachable {BANK_PICK_BURST_BANK} bank: bank={} kind={:?}",
                                bank.name, selected.kind
                            )));
                        }
                        if self.account != self.interaction_account {
                            self.query_complete();
                            self.step = BankPickBurstStep::Done;
                            return Ok(ScriptFlow::Complete);
                        }
                        self.access_tile = Some(selected.access_tile);
                        self.access = Some(access);
                        self.step = BankPickBurstStep::Walk;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            BankPickBurstStep::Walk => {
                if self.walk.is_none() {
                    let Some(target) = self.access_tile else {
                        return Ok(self.blocked("native Walk has no selected bank access tile"));
                    };
                    let request = script::native::WalkRequest {
                        target,
                        loc_id: None,
                        radius: 0,
                        arrival: nav::arrival::ArrivalKind::Reach,
                        options: script::native::WalkOptions::default(),
                        required_after: tick.cx.evidence(),
                        evidence: None,
                        cross: Box::default(),
                        protect: false,
                        food_guard: false,
                        allow: Default::default(),
                    };
                    self.guest.lock().burst.walk_started_at_ms =
                        Some(self.started.elapsed().as_millis());
                    match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                        Ok(handle) => self.walk = Some(handle),
                        Err(error) => {
                            return Ok(self.blocked(format!("native Walk begin failed: {error:?}")));
                        }
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.walk.as_ref().expect("Walk handle"), &mut tick.cx)
                {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => {
                        self.walk = None;
                        Ok(self.blocked(format!("native Walk failed: {error:?}")))
                    }
                    Poll::Ready(Ok(receipt)) => {
                        self.walk = None;
                        let mut guest = self.guest.lock();
                        guest.burst.walk_completed_at_ms = Some(self.started.elapsed().as_millis());
                        guest.burst.walk_end = Some(format!("{:?}", receipt.end));
                        guest.trace.final_tile =
                            tick.cx.snapshot().here().map(|here| tile_tuple(here.value));
                        drop(guest);
                        if receipt.end != script::native::WalkEnd::Arrived {
                            return Ok(self.blocked(format!(
                                "native Walk did not arrive at the selected access tile: {:?}",
                                receipt.end
                            )));
                        }
                        self.step = BankPickBurstStep::Open;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            BankPickBurstStep::Open => {
                if self.open.is_none() {
                    let snapshot = tick.cx.snapshot();
                    let inventory = observed_item_count(
                        snapshot.inventory().map(|observed| observed.value),
                        LAW_RUNE_ID,
                    );
                    if inventory != Some(1) {
                        return Ok(self.blocked(format!(
                            "selected bank interaction lacks its one seeded law rune: inventory={inventory:?}"
                        )));
                    }
                    self.guest.lock().trace.inventory_law_before = inventory;
                    let Some(access) = self.access.as_ref().cloned() else {
                        return Ok(self.blocked("native Open has no selected bank access"));
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
                match tick
                    .actions
                    .poll(self.open.as_ref().expect("Open handle"), &mut tick.cx)
                {
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
                            let mut guest = self.guest.lock();
                            guest.trace.open_loaded = loaded && open;
                            guest.trace.bank_law_before = bank;
                        }
                        if !loaded || !open || bank.is_none() {
                            return Ok(self.blocked(format!(
                                "native Open returned without an observed loaded bank: loaded={loaded} open={open} law_rune={bank:?}"
                            )));
                        }
                        self.step = BankPickBurstStep::Deposit;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            BankPickBurstStep::Deposit => {
                if self.deposit.is_none() {
                    match tick.actions.begin::<Deposit>(
                        DepositArgs {
                            products: Arc::from([LAW_RUNE_ID]),
                            keep: Arc::from([]),
                            access: None,
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
                match tick
                    .actions
                    .poll(self.deposit.as_ref().expect("Deposit handle"), &mut tick.cx)
                {
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
                        let before = self.guest.lock().trace.bank_law_before;
                        {
                            let mut guest = self.guest.lock();
                            guest.trace.deposited = Some(deposited);
                            guest.trace.inventory_law_after_deposit = inventory;
                            guest.trace.bank_law_after_deposit = bank;
                        }
                        let expected_bank = before.and_then(|count| count.checked_add(1));
                        if deposited != 1 || inventory != Some(0) || bank != expected_bank {
                            return Ok(self.blocked(format!(
                                "native Deposit transfer was not observed: deposited={deposited} inventory={inventory:?} bank={bank:?} expected_bank={expected_bank:?}"
                            )));
                        }
                        self.step = BankPickBurstStep::Withdraw;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            BankPickBurstStep::Withdraw => {
                if self.withdraw.is_none() {
                    match tick.actions.begin::<Withdraw>(
                        WithdrawArgs {
                            withdrawals: Arc::from([Withdrawal {
                                id: LAW_RUNE_ID,
                                name: Arc::from("Law rune"),
                                target: 1,
                            }]),
                            access: None,
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
                match tick.actions.poll(
                    self.withdraw.as_ref().expect("Withdraw handle"),
                    &mut tick.cx,
                ) {
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
                        let before = self.guest.lock().trace.bank_law_before;
                        {
                            let mut guest = self.guest.lock();
                            guest.trace.withdrew_complete = Some(complete);
                            guest.trace.inventory_law_after_withdraw = inventory;
                            guest.trace.bank_law_after_withdraw = bank;
                        }
                        if !complete || inventory != Some(1) || bank != before {
                            return Ok(self.blocked(format!(
                                "native Withdraw transfer was not observed: complete={complete} inventory={inventory:?} bank={bank:?} expected_bank={before:?}"
                            )));
                        }
                        self.step = BankPickBurstStep::Close;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            BankPickBurstStep::Close => {
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
                match tick
                    .actions
                    .poll(self.close.as_ref().expect("Close handle"), &mut tick.cx)
                {
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
                            let mut guest = self.guest.lock();
                            guest.trace.closed_observed = final_open == Some(false);
                            guest.trace.final_bank_open = final_open;
                            guest.trace.final_tile = final_tile;
                            guest.trace.final_inventory_law = inventory;
                        }
                        let guest = self.guest.lock();
                        if let Err(error) =
                            validate_bank_pick_burst_interaction(&guest.trace, &guest.burst)
                        {
                            drop(guest);
                            return Ok(self.blocked(error));
                        }
                        drop(guest);
                        self.query_complete();
                        self.step = BankPickBurstStep::Done;
                        Ok(ScriptFlow::Complete)
                    }
                }
            }
            BankPickBurstStep::Gate | BankPickBurstStep::Done => {
                unreachable!("bank-pick burst gate and terminal steps are handled above")
            }
        }
    }
}

fn validate_bank_pick_burst_interaction(
    trace: &TellerTrace,
    burst: &BankPickBurstTrace,
) -> Result<(), String> {
    let expected_bank = trace.bank_law_before.and_then(|count| count.checked_add(1));
    let movement = trace
        .started_at
        .zip(trace.final_tile)
        .is_some_and(|(start, end)| start != end);
    if trace.started_at != Some(tile_tuple(BANK_PICK_BURST_ORIGIN))
        || burst.initial_scene2_at_ms.is_none()
        || burst.ready_tile != Some(tile_tuple(BANK_PICK_BURST_ORIGIN))
        || burst.ready_law_runes != Some(1)
        || burst.query_from != Some(tile_tuple(BANK_PICK_BURST_ORIGIN))
        || burst.query_started_at_ms.is_none()
        || burst.query_submitted_at.is_none()
        || burst.query_completed_at_ms.is_none()
        || trace.selected_bank.as_deref() != Some(BANK_PICK_BURST_BANK)
        || trace.selected_kind.as_deref() != Some("Reachable")
        || trace.access_tile.is_none()
        || trace.final_tile != trace.access_tile
        || burst.walk_started_at_ms.is_none()
        || burst.walk_completed_at_ms.is_none()
        || burst.walk_end.as_deref() != Some("Arrived")
        || !movement
        || !trace.open_loaded
        || trace.inventory_law_before != Some(1)
        || trace.bank_law_before.is_none()
        || trace.deposited != Some(1)
        || trace.inventory_law_after_deposit != Some(0)
        || trace.bank_law_after_deposit != expected_bank
        || trace.withdrew_complete != Some(true)
        || trace.inventory_law_after_withdraw != Some(1)
        || trace.bank_law_after_withdraw != trace.bank_law_before
        || !trace.closed_observed
        || trace.final_bank_open != Some(false)
        || trace.final_inventory_law != Some(1)
    {
        return Err(format!(
            "native bank selection did not cause an observed route, session, deposit, withdrawal and close: {trace:#?}; burst={burst:#?}"
        ));
    }
    Ok(())
}

#[derive(Default)]
struct BankPickBurstAdmission {
    capacity_snapshots: Vec<serde_json::Value>,
    work_events: Vec<serde_json::Value>,
    work_identities: HashSet<api::identity::WorkIdentity>,
    last_capacity_sequence: Option<u64>,
    last_progress_sequence: Option<u64>,
    last_terminal_sequence: Option<u64>,
    owner_observations: Vec<(String, BankPickWorkObservation)>,
    full_sized_query_inputs_identical: bool,
    full_witness: Option<BankPickBurstFullWitness>,
    first_release_after_full: Option<serde_json::Value>,
    admitted_after_full: HashSet<String>,
}

#[derive(Clone)]
struct BankPickBurstFullWitness {
    account: String,
    elapsed_ms: u128,
    capacity_sequence: u64,
    capacity: api::work::WorkCapacity,
    accepted_accounts: Vec<String>,
    accepted_identity_accounts: Vec<String>,
    pending_accounts: Vec<String>,
    retained_revision: api::work::InputRevision,
    retained_charge: api::work::WorkCharge,
    full_sized_query_inputs_identical: bool,
}

fn account_for_work_identity(
    identity: api::identity::WorkIdentity,
    runs: &[(api::selected::RunKey, String)],
) -> Option<String> {
    runs.iter()
        .find(|(candidate, _)| match identity.scope() {
            api::identity::Scope::Slot(slot) => candidate.slot_key() == Some(slot),
            api::identity::Scope::Run(run) => *candidate == run,
            api::identity::Scope::Execution(execution) => *candidate == execution.run(),
            api::identity::Scope::Process | api::identity::Scope::Session(_) => false,
        })
        .map(|(_, account)| account.clone())
}

fn record_burst_work_identity(
    admission: &mut BankPickBurstAdmission,
    identity: api::identity::WorkIdentity,
    kind: &str,
    publication_sequence: u64,
    elapsed_ms: u128,
    runs: &[(api::selected::RunKey, String)],
) {
    if !admission.work_identities.insert(identity) {
        return;
    }
    let account = account_for_work_identity(identity, runs);
    if let (Some(witness), Some(account)) = (admission.full_witness.as_ref(), account.as_ref()) {
        if account == &witness.account && publication_sequence > witness.capacity_sequence {
            admission.admitted_after_full.insert(account.clone());
        }
    }
    admission.work_events.push(serde_json::json!({
        "kind": kind,
        "elapsed_ms": elapsed_ms,
        "capacity_publication_sequence": publication_sequence,
        "account": account,
        "job_id": identity.job().get(),
        "scope": format!("{:?}", identity.scope()),
        "identity": format!("{identity:?}"),
    }));
}

fn sample_bank_pick_burst_owners(
    play: &super::Play,
    admission: &mut BankPickBurstAdmission,
    capacity_sequence: u64,
    started: Instant,
    runs: &[(api::selected::RunKey, String)],
) {
    let observations: Vec<_> = {
        let navs = play.navs.lock().unwrap();
        let observations: Vec<_> = runs
            .iter()
            .filter_map(|(_, account)| {
                let owner = navs.get(account)?.bank_pick.work_observation_for_test()?;
                Some((account.clone(), owner))
            })
            .collect();
        admission.full_sized_query_inputs_identical = runs.len() == BANK_PICK_BURST_GUESTS
            && observations.len() == BANK_PICK_BURST_GUESTS
            && observations.first().is_some_and(|(account, _)| {
                let first = &navs[account].bank_pick;
                observations
                    .iter()
                    .all(|(account, _)| first.query_inputs_match_for_test(&navs[account].bank_pick))
            });
        observations
    };
    for (_, owner) in &observations {
        if let BankPickWorkObservation::Accepted { identity, .. } = owner {
            record_burst_work_identity(
                admission,
                *identity,
                "accepted_owner",
                capacity_sequence,
                started.elapsed().as_millis(),
                runs,
            );
        }
    }
    admission.owner_observations = observations;
}

/// One reading of the shared work service, taken under one lock.
struct BankPickBurstServiceSample {
    capacity_sequence: u64,
    capacity: api::work::WorkCapacity,
    worker_available: bool,
    health: Option<String>,
}

fn sample_bank_pick_burst_work(
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    admission: &mut BankPickBurstAdmission,
    started: Instant,
    runs: &[(api::selected::RunKey, String)],
) -> BankPickBurstServiceSample {
    let service = shared.lock();
    let (capacity_sequence, published_capacity) = service
        .publications()
        .capacity()
        .read(|sequence, capacity| (sequence.get(), *capacity));
    let service_capacity = service.capacity();
    let worker_available = service.worker_available();
    let health = service.health().map(|condition| format!("{condition:?}"));
    let elapsed_ms = started.elapsed().as_millis();
    if admission.last_capacity_sequence != Some(capacity_sequence)
        || admission.capacity_snapshots.is_empty()
    {
        admission.last_capacity_sequence = Some(capacity_sequence);
        admission.capacity_snapshots.push(serde_json::json!({
            "elapsed_ms": elapsed_ms,
            "publication_sequence": capacity_sequence,
            "published": {
                "jobs": published_capacity.jobs(),
                "bytes": published_capacity.bytes(),
                "handles": published_capacity.handles(),
            },
            "service": {
                "jobs": service_capacity.jobs(),
                "bytes": service_capacity.bytes(),
                "handles": service_capacity.handles(),
            },
            "health": health.clone(),
        }));
    }

    let (progress_sequence, progress) = service
        .publications()
        .progress()
        .read(|sequence, progress| (sequence.get(), progress.clone()));
    if admission.last_progress_sequence != Some(progress_sequence) {
        admission.last_progress_sequence = Some(progress_sequence);
        if let Some(progress) = progress {
            record_burst_work_identity(
                admission,
                progress.identity(),
                "progress",
                capacity_sequence,
                elapsed_ms,
                runs,
            );
            admission.work_events.push(serde_json::json!({
                "kind": "progress_snapshot",
                "elapsed_ms": elapsed_ms,
                "publication_sequence": progress_sequence,
                "phase": progress.phase(),
                "completed": progress.completed(),
                "total": progress.total(),
                "identity": format!("{:?}", progress.identity()),
                "account": account_for_work_identity(progress.identity(), runs),
            }));
        }
    }
    let (terminal_sequence, terminal) = service
        .publications()
        .terminal()
        .read(|sequence, terminal| (sequence.get(), *terminal));
    if admission.last_terminal_sequence != Some(terminal_sequence) {
        admission.last_terminal_sequence = Some(terminal_sequence);
        if let Some(identity) = terminal {
            record_burst_work_identity(
                admission,
                identity,
                "terminal",
                capacity_sequence,
                elapsed_ms,
                runs,
            );
        }
    }
    if admission.first_release_after_full.is_none() {
        if let Some(witness) = admission.full_witness.as_ref() {
            let capacity_advanced = capacity_sequence > witness.capacity_sequence;
            let release_visible = published_capacity.jobs() < witness.capacity.jobs();
            let retry_observed = admission.admitted_after_full.contains(&witness.account);
            if capacity_advanced && (release_visible || retry_observed) {
                admission.first_release_after_full = Some(serde_json::json!({
                    "elapsed_ms": elapsed_ms,
                    "publication_sequence": capacity_sequence,
                    "jobs_at_observation": published_capacity.jobs(),
                    "bytes_at_observation": published_capacity.bytes(),
                    "handles_at_observation": published_capacity.handles(),
                    "release_publication_observed_directly": release_visible,
                    "release_then_reaccept_inferred_from_capacity_sequence_and_new_work_identity":
                        retry_observed && !release_visible,
                }));
            }
        }
    }
    BankPickBurstServiceSample {
        capacity_sequence,
        capacity: published_capacity,
        worker_available,
        health,
    }
}

fn bank_pick_burst_account_snapshots(
    names: &[String],
    guests: &Mutex<HashMap<String, Arc<Mutex<BankPickBurstGuest>>>>,
) -> Vec<(String, BankPickBurstGuest)> {
    let map = guests.lock();
    names
        .iter()
        .filter_map(|name| {
            map.get(name)
                .map(|guest| (name.clone(), guest.lock().clone()))
        })
        .collect()
}

fn observe_bank_pick_burst_full(
    admission: &mut BankPickBurstAdmission,
    sample: &BankPickBurstServiceSample,
    snapshots: &[(String, BankPickBurstGuest)],
    runs: &[(api::selected::RunKey, String)],
    started: Instant,
) {
    if admission.full_witness.is_some() {
        return;
    }
    let capacity_sequence = sample.capacity_sequence;
    let capacity = sample.capacity;
    if !sample.worker_available
        || sample.health.is_some()
        || capacity.jobs() != BANK_PICK_BURST_FULL_JOBS
        || capacity.bytes() == 0
    {
        return;
    }
    let pending = |account: &str| {
        snapshots.iter().any(|(name, guest)| {
            name == account
                && guest.burst.query_submitted_at.is_some()
                && guest.burst.query_completed_at_ms.is_none()
                && guest.trace.outcome.is_none()
        })
    };
    let mut accepted_accounts = Vec::new();
    for (account, owner) in &admission.owner_observations {
        if let BankPickWorkObservation::Accepted { identity, .. } = owner {
            if !pending(account)
                || account_for_work_identity(*identity, runs).as_ref() != Some(account)
            {
                return;
            }
            accepted_accounts.push(account.clone());
        }
    }
    accepted_accounts.sort();
    accepted_accounts.dedup();
    if accepted_accounts.len() != 2 {
        return;
    }
    let mut refused = admission
        .owner_observations
        .iter()
        .filter_map(|(account, owner)| {
            let BankPickWorkObservation::Full {
                revision,
                capacity_sequence: refused_at,
                charge,
            } = owner
            else {
                return None;
            };
            (refused_at.get() == capacity_sequence && pending(account))
                .then_some((account, *revision, *charge))
        })
        .collect::<Vec<_>>();
    refused.sort_by_key(|(account, _, _)| *account);
    let Some((account, retained_revision, retained_charge)) = refused.first() else {
        return;
    };
    admission.full_witness = Some(BankPickBurstFullWitness {
        account: (*account).clone(),
        elapsed_ms: started.elapsed().as_millis(),
        capacity_sequence,
        capacity,
        accepted_identity_accounts: accepted_accounts.clone(),
        accepted_accounts,
        pending_accounts: refused
            .iter()
            .map(|(account, _, _)| (*account).clone())
            .collect(),
        retained_revision: *retained_revision,
        retained_charge: *retained_charge,
        full_sized_query_inputs_identical: admission.full_sized_query_inputs_identical,
    });
}

#[cfg(test)]
mod bank_pick_burst_oracle_tests {
    use super::*;
    use api::identity::{ProcessIncarnation, Publication, RequestIdMinter, Scope, WorkIdentity};
    use api::work::{InputRevision, WorkCapacity, WorkCharge};

    struct Receipt {
        admission: BankPickBurstAdmission,
        guests: Vec<(String, BankPickBurstGuest)>,
        runs: Vec<(api::selected::RunKey, String)>,
        capacity: WorkCapacity,
    }

    impl Receipt {
        fn correct() -> Self {
            let incarnation = ProcessIncarnation::from_start_nonce(std::num::NonZeroU64::MIN);
            let ids = RequestIdMinter::new();
            let runs: Vec<_> = (1..=3)
                .map(|slot| {
                    (
                        api::selected::RunKey {
                            slot,
                            session: 1,
                            run: 1,
                        },
                        format!("guest-{slot}"),
                    )
                })
                .collect();
            let owners = runs
                .iter()
                .take(2)
                .map(|(run, account)| {
                    (
                        account.clone(),
                        BankPickWorkObservation::Accepted {
                            identity: WorkIdentity::new(
                                ids.mint().unwrap(),
                                incarnation,
                                Scope::Slot(run.slot_key().unwrap()),
                                None,
                            )
                            .unwrap(),
                            revision: InputRevision::FIRST,
                        },
                    )
                })
                .chain(std::iter::once((
                    runs[2].1.clone(),
                    BankPickWorkObservation::Full {
                        revision: InputRevision::FIRST,
                        capacity_sequence: Publication::new(Scope::Process, ()).sequence(),
                        charge: WorkCharge::new(0, 192 * 1024 * 1024, 0, 0),
                    },
                )))
                .collect();
            let guests = runs
                .iter()
                .map(|(_, account)| {
                    let mut guest = BankPickBurstGuest::new();
                    guest.burst.query_submitted_at = Some(Instant::now());
                    (account.clone(), guest)
                })
                .collect();
            Self {
                admission: BankPickBurstAdmission {
                    owner_observations: owners,
                    ..Default::default()
                },
                guests,
                runs,
                capacity: WorkCapacity::new(BANK_PICK_BURST_FULL_JOBS, 384 * 1024 * 1024, 0),
            }
        }

        fn evaluate(&mut self) -> bool {
            observe_bank_pick_burst_full(
                &mut self.admission,
                &BankPickBurstServiceSample {
                    capacity_sequence: 0,
                    capacity: self.capacity,
                    worker_available: true,
                    health: None,
                },
                &self.guests,
                &self.runs,
                Instant::now(),
            );
            self.admission.full_witness.is_some()
        }
    }

    #[test]
    fn bank_pick_burst_oracle_accepts_slot_tickets_and_typed_full_custody() {
        let mut receipt = Receipt::correct();
        assert!(receipt.evaluate());
        let witness = receipt.admission.full_witness.unwrap();
        assert_eq!(witness.accepted_identity_accounts.len(), 2);
        assert_eq!(witness.account, "guest-3");
        assert_eq!(witness.retained_revision, InputRevision::FIRST);
    }

    #[test]
    fn bank_pick_burst_oracle_rejects_null_accounts() {
        let mut receipt = Receipt::correct();
        let (_, BankPickWorkObservation::Accepted { identity, .. }) =
            &mut receipt.admission.owner_observations[0]
        else {
            unreachable!()
        };
        *identity = WorkIdentity::new(identity.job(), identity.incarnation(), Scope::Process, None)
            .unwrap();
        assert!(!receipt.evaluate());
    }

    #[test]
    fn bank_pick_burst_oracle_rejects_missing_full_custody() {
        let mut receipt = Receipt::correct();
        receipt.admission.owner_observations.truncate(2);
        assert!(!receipt.evaluate());
    }

    #[test]
    fn bank_pick_burst_oracle_rejects_an_unfilled_job_limit() {
        for jobs in [2, BANK_PICK_BURST_FULL_JOBS - 1] {
            let mut receipt = Receipt::correct();
            receipt.capacity = WorkCapacity::new(jobs, receipt.capacity.bytes(), 0);
            assert!(!receipt.evaluate());
        }
    }
}

fn bank_pick_burst_guest_json(
    account: &str,
    guest: &BankPickBurstGuest,
    interaction_account: &str,
    admitted_accounts: &HashSet<String>,
    full_witness: Option<&BankPickBurstFullWitness>,
    released_at_ms: Option<u128>,
    retried_accounts: &HashSet<String>,
) -> serde_json::Value {
    let query_latency_ms = guest
        .burst
        .query_started_at_ms
        .zip(guest.burst.query_completed_at_ms)
        .map(|(start, end)| end.saturating_sub(start));
    let admission = if full_witness.is_some_and(|witness| witness.account == account) {
        "Full, retained owner demand, retried after publication"
    } else if full_witness
        .is_some_and(|witness| witness.pending_accounts.iter().any(|row| row == account))
    {
        "Full while capacity remained occupied, then settled"
    } else if admitted_accounts.contains(account) {
        "accepted and settled"
    } else {
        "no WorkAdmission identity observed"
    };
    serde_json::json!({
        "account": account,
        "preparation": {
            "final_phase": format!("{:?}", guest.phase),
            "initial_ingame_scene2_at_ms": guest.burst.initial_scene2_at_ms,
            "ready_ingame_scene2_at_ms": guest.burst.ready_scene2_at_ms,
            "ready_tile": guest.burst.ready_tile,
            "seeded_law_runes": guest.burst.ready_law_runes,
        },
        "query_input": {
            "api": "production native script::bank::Select",
            "from": guest.burst.query_from,
            "explicit_bank": BANK_PICK_BURST_BANK,
            "preferences": "default",
            "walk_options": "default",
            "production_search_settle_budget": nav::router::NODE_BUDGET,
        },
        "query": {
            "started_at_ms": guest.burst.query_started_at_ms,
            "owner_demand_submitted_at_ms": guest.burst.query_submitted_at_ms,
            "selected_at_ms": guest.burst.query_completed_at_ms,
            "end_to_end_latency_ms": query_latency_ms,
        },
        "admission": {
            "observation": admission,
            "accepted_identity_observed": admitted_accounts.contains(account),
            "retry_identity_observed_after_full": retried_accounts.contains(account),
            "capacity_release_elapsed_ms": released_at_ms,
        },
        "selection": {
            "bank_index": guest.burst.selected_index,
            "bank": guest.trace.selected_bank,
            "kind": guest.trace.selected_kind,
            "access_kind": guest.trace.access_kind,
            "access_name": guest.trace.access_npc,
            "access_choice": guest.trace.access_choice,
            "access_tile": guest.trace.access_tile,
        },
        "interaction": {
            "script_caused": account == interaction_account,
            "walk_started_at_ms": guest.burst.walk_started_at_ms,
            "walk_completed_at_ms": guest.burst.walk_completed_at_ms,
            "walk_end": guest.burst.walk_end,
            "start_tile": guest.trace.started_at,
            "final_tile": guest.trace.final_tile,
            "bank_open_loaded": guest.trace.open_loaded,
            "inventory_law_runes_before": guest.trace.inventory_law_before,
            "bank_law_runes_before": guest.trace.bank_law_before,
            "deposit": {
                "complete": guest.trace.deposited,
                "inventory_after": guest.trace.inventory_law_after_deposit,
                "bank_after": guest.trace.bank_law_after_deposit,
            },
            "withdraw": {
                "complete": guest.trace.withdrew_complete,
                "inventory_after": guest.trace.inventory_law_after_withdraw,
                "bank_after": guest.trace.bank_law_after_withdraw,
            },
            "bank_closed": guest.trace.closed_observed,
            "final_bank_open": guest.trace.final_bank_open,
            "final_inventory_law_runes": guest.trace.final_inventory_law,
        },
        "terminal": {
            "outcome": match guest.trace.outcome {
                Some(TellerOutcome::Passed) => "passed",
                Some(TellerOutcome::Failed(_)) => "failed",
                None => "pending",
            },
            "at_ms": guest.burst.terminal_at_ms,
            "failure": guest.trace.failure,
        },
    })
}

fn bank_pick_burst_percentile(values: &[u128], percentile: usize) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * percentile).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

#[cfg_attr(test, test)]
#[cfg_attr(
    test,
    ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX, LIVE_EVIDENCE_DIR, local 289 assets and Engine A on 127.0.0.1:44594/1080"
)]
pub fn live_ten_guest_shared_bank_pick_full_retry_burst() {
    assert!(
        live(),
        "live_ten_guest_shared_bank_pick_full_retry_burst requires LIVE=1"
    );
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .expect("ten-guest bank-pick burst requires BOT_LIVE_NAME_PREFIX");
    assert!(
        !prefix.is_empty() && prefix.len() <= 4 && prefix.is_ascii(),
        "BOT_LIVE_NAME_PREFIX must be 1–4 ASCII bytes"
    );
    assert_eq!(
        std::env::var("GATHERER_GAME_PORT").unwrap_or_default(),
        "44594",
        "ten-guest bank-pick burst must use Engine A game port 44594"
    );
    assert_eq!(
        std::env::var("GATHERER_HTTP_PORT").unwrap_or_default(),
        "1080",
        "ten-guest bank-pick burst must use Engine A HTTP port 1080"
    );
    let evidence_root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR")
            .expect("ten-guest bank-pick burst requires LIVE_EVIDENCE_DIR"),
    );
    std::fs::create_dir_all(&evidence_root).expect("create ten-guest evidence root");
    let home = install_throwaway_home();
    let (_profile, template, selected, facts) =
        live_teller_profile(&home.path).expect("load selected local 289 bank-pick facts");
    let world = template
        .world()
        .expect("ten-guest native bank selection requires the selected navigation world");
    assert!(
        world.collision.standable(BANK_PICK_BURST_ORIGIN),
        "real Draynor street origin is not standable in the selected navigation pack"
    );
    let target = facts
        .banks()
        .iter()
        .find(|bank| bank.name == BANK_PICK_BURST_BANK)
        .expect("selected facts must contain Varrock West");
    assert!(
        target.routable,
        "Varrock West must have selected-content routing geometry"
    );
    let names = super::mint_live_names(BANK_PICK_BURST_GUESTS);
    assert_eq!(names.len(), BANK_PICK_BURST_GUESTS);
    assert!(
        names.iter().all(|name| name.starts_with(&prefix)),
        "minted guest account names must use BOT_LIVE_NAME_PREFIX"
    );
    assert_eq!(
        names.iter().collect::<HashSet<_>>().len(),
        BANK_PICK_BURST_GUESTS,
        "ten fresh guest names must be unique"
    );
    let interaction_account = names[0].clone();
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let evidence_file = evidence_root.join(format!(
        "bank_pick_ten_guest_{}_utc-{epoch}Z.json",
        names[0]
    ));
    let service =
        WorkService::shared().expect("shared WorkAdmission must be available before the burst");
    {
        let service = service.lock();
        assert!(
            service.worker_available() && service.health().is_none(),
            "shared WorkAdmission must be healthy before the burst"
        );
        let capacity = service.capacity();
        assert_eq!(
            capacity,
            api::work::WorkCapacity::EMPTY,
            "ten-guest burst must start with no charged WorkAdmission jobs"
        );
    }

    let started = Instant::now();
    let guest_map = Arc::new(Mutex::new(
        names
            .iter()
            .cloned()
            .map(|name| (name, Arc::new(Mutex::new(BankPickBurstGuest::new()))))
            .collect::<HashMap<_, _>>(),
    ));
    let start_gate = Arc::new(AtomicBool::new(false));
    let frame_guests = Arc::clone(&guest_map);
    let frame_started = started;
    let entries = super::mint_live_entries(&names);
    assert_eq!(
        entries.len(),
        BANK_PICK_BURST_GUESTS,
        "mint ten unique login entries for the ten fresh accounts"
    );
    let play = run_with_template(
        template,
        true,
        entries
            .into_iter()
            .enumerate()
            .map(|(index, (username, password))| Profile {
                username,
                password: password.into(),
                uid: 274_279_200 + index as i32,
                settings: ProfileSettings::default(),
            })
            .collect(),
        |_| (None, None),
        move |client, account, _| {
            bank_pick_burst_frame(client, account, &frame_guests, frame_started)
        },
    )
    .expect("start one real Play process with ten guest slots");
    assert_eq!(
        play.named_banks().banks(),
        facts.banks(),
        "Play and all native scripts must use the same selected bank roster"
    );

    let start = play.script_start_handle();
    let mut run_keys = Vec::<(api::selected::RunKey, String)>::with_capacity(names.len());
    let mut attempted_start = false;
    let mut gate_released = false;
    let mut admission = BankPickBurstAdmission::default();
    let mut saturation_holders = Vec::with_capacity(BANK_PICK_BURST_HOLDERS);
    let mut holder_receipts = Vec::with_capacity(BANK_PICK_BURST_HOLDERS);
    let mut holder_capacity = api::work::WorkCapacity::EMPTY;
    let mut holders_released_at_ms = None;
    let deadline = started + BANK_PICK_BURST_TIMEOUT;
    loop {
        let sample = sample_bank_pick_burst_work(&service, &mut admission, started, &run_keys);
        sample_bank_pick_burst_owners(
            &play,
            &mut admission,
            sample.capacity_sequence,
            started,
            &run_keys,
        );
        let snapshots = bank_pick_burst_account_snapshots(&names, &guest_map);

        if !attempted_start {
            let all_ready = snapshots.len() == BANK_PICK_BURST_GUESTS
                && snapshots
                    .iter()
                    .all(|(_, guest)| guest.phase == BankPickBurstPrep::Ready);
            let prep_failed = snapshots
                .iter()
                .any(|(_, guest)| guest.phase == BankPickBurstPrep::Failed);
            if all_ready {
                attempted_start = true;
                for account in &names {
                    let guest = guest_map
                        .lock()
                        .get(account)
                        .cloned()
                        .expect("every minted account has a preparation owner");
                    let script = BankPickBurstScript::new(
                        account.clone(),
                        interaction_account.clone(),
                        Arc::clone(&facts),
                        guest.clone(),
                        Arc::clone(&start_gate),
                        started,
                    );
                    match start.start_test_script(
                        account,
                        Box::new(script),
                        Some(Arc::clone(&selected)),
                    ) {
                        Ok(run) if play.script_native_run(account) == Some(run) => {
                            run_keys.push((run, account.clone()));
                            play.wake(account);
                        }
                        Ok(_) => guest.lock().fail(
                            "real Play did not publish the started bank-pick native run".into(),
                            started,
                        ),
                        Err(error) => guest.lock().fail(
                            format!("real Play rejected the native bank-pick script: {error}"),
                            started,
                        ),
                    }
                }
            } else if prep_failed {
                break;
            }
        }

        if attempted_start && !gate_released && run_keys.len() == BANK_PICK_BURST_GUESTS {
            if saturation_holders.is_empty() {
                let bag = Arc::new(script::native::SettingsBag::new());
                for revision in 1..=BANK_PICK_BURST_HOLDERS as u64 {
                    saturation_holders.push(
                        play.script_prepare_config(
                            script::CompiledId("Sherlock"),
                            revision,
                            Arc::clone(&bag),
                        )
                        .expect("admit an actual native configuration preparation"),
                    );
                }
            }
            if saturation_holders.iter().all(|holder| holder.is_finished()) {
                holder_capacity = service.lock().capacity();
                assert_eq!(holder_capacity.jobs(), BANK_PICK_BURST_HOLDERS);
                assert_eq!(holder_capacity.handles(), 0);
                for holder in &saturation_holders {
                    let identity = holder.identity();
                    holder_receipts.push(serde_json::json!({
                        "api": "Play::script_prepare_config",
                        "card": "Sherlock",
                        "family": "ProfilePreparation",
                        "job_id": identity.job().get(),
                        "identity": format!("{identity:?}"),
                        "terminal_published_unconsumed_before_queries": true,
                    }));
                }
                start_gate.store(true, Ordering::Release);
                gate_released = true;
            }
        }

        let snapshots = bank_pick_burst_account_snapshots(&names, &guest_map);
        if gate_released {
            observe_bank_pick_burst_full(&mut admission, &sample, &snapshots, &run_keys, started);
            if admission.full_witness.is_some() && !saturation_holders.is_empty() {
                for holder in saturation_holders.drain(..) {
                    let _prepared = holder
                        .join()
                        .expect("consume the actual configuration preparation terminal")
                        .expect("the native configuration preparation must succeed");
                }
                holders_released_at_ms = Some(started.elapsed().as_millis());
            }
            let all_terminal = snapshots.len() == BANK_PICK_BURST_GUESTS
                && snapshots
                    .iter()
                    .all(|(_, guest)| guest.trace.outcome.is_some());
            let capacity = sample.capacity;
            let capacity_empty =
                capacity.jobs() == 0 && capacity.bytes() == 0 && capacity.handles() == 0;
            if all_terminal && capacity_empty {
                break;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(saturation_holders);

    if !gate_released {
        start_gate.store(true, Ordering::Release);
    }
    let (final_capacity, final_worker_available, final_health) = {
        let service = service.lock();
        (
            service.capacity(),
            service.worker_available(),
            service.health().map(|condition| format!("{condition:?}")),
        )
    };
    let snapshots = bank_pick_burst_account_snapshots(&names, &guest_map);
    let account_runs = run_keys.clone();
    let admitted_accounts: HashSet<String> = admission
        .work_identities
        .iter()
        .filter_map(|identity| account_for_work_identity(*identity, &account_runs))
        .chain(
            snapshots
                .iter()
                .filter(|(_, guest)| guest.burst.query_completed_at_ms.is_some())
                .map(|(account, _)| account.clone()),
        )
        .collect();
    let released_at_ms = admission
        .first_release_after_full
        .as_ref()
        .and_then(|release| release.get("elapsed_ms"))
        .and_then(serde_json::Value::as_u64)
        .map(u128::from);
    let retried_accounts = admission.admitted_after_full.clone();
    let query_latencies: Vec<_> = snapshots
        .iter()
        .filter_map(|(_, guest)| {
            guest
                .burst
                .query_started_at_ms
                .zip(guest.burst.query_completed_at_ms)
                .map(|(query_start, query_end)| query_end.saturating_sub(query_start))
        })
        .collect();
    let guest_evidence: Vec<_> = snapshots
        .iter()
        .map(|(account, guest)| {
            bank_pick_burst_guest_json(
                account,
                guest,
                &interaction_account,
                &admitted_accounts,
                admission.full_witness.as_ref(),
                released_at_ms,
                &retried_accounts,
            )
        })
        .collect();
    let full_witness = admission.full_witness.as_ref().map(|witness| {
        serde_json::json!({
            "refusal": "Full",
            "refusal_observation":
                "two accepted Slot-scoped owner tickets plus the exact SubmissionRefusal::Full retained job at the same capacity publication",
            "completed_product_terminal_holders": holder_receipts.len(),
            "account": witness.account,
            "elapsed_ms": witness.elapsed_ms,
            "capacity_publication_sequence": witness.capacity_sequence,
            "capacity": {
                "jobs": witness.capacity.jobs(),
                "bytes": witness.capacity.bytes(),
                "handles": witness.capacity.handles(),
            },
            "accepted_accounts": witness.accepted_accounts,
            "accepted_identity_accounts": witness.accepted_identity_accounts,
            "pending_owner_accounts": witness.pending_accounts,
            "retained_input_revision": witness.retained_revision.get(),
            "retained_job_charge_bytes": witness.retained_charge.total_bytes(),
        })
    });
    let retried_after_release = admission.full_witness.as_ref().is_some_and(|witness| {
        admission.first_release_after_full.is_some()
            && retried_accounts.contains(&witness.account)
            && snapshots.iter().any(|(account, guest)| {
                account == &witness.account
                    && guest.burst.query_completed_at_ms.is_some()
                    && guest.trace.selected_kind.as_deref() == Some("Reachable")
            })
    });
    let all_prepared = snapshots.len() == BANK_PICK_BURST_GUESTS
        && snapshots.iter().all(|(_, guest)| {
            guest.burst.initial_scene2_at_ms.is_some()
                && guest.burst.ready_scene2_at_ms.is_some()
                && guest.burst.ready_tile == Some(tile_tuple(BANK_PICK_BURST_ORIGIN))
                && guest.burst.ready_law_runes == Some(1)
        });
    let all_scripts_started = run_keys.len() == BANK_PICK_BURST_GUESTS;
    let all_selected = snapshots.len() == BANK_PICK_BURST_GUESTS
        && snapshots.iter().all(|(_, guest)| {
            guest.burst.query_started_at_ms.is_some()
                && guest.burst.query_submitted_at.is_some()
                && guest.burst.query_completed_at_ms.is_some()
                && guest.burst.query_from == Some(tile_tuple(BANK_PICK_BURST_ORIGIN))
                && guest.trace.selected_bank.as_deref() == Some(BANK_PICK_BURST_BANK)
                && guest.trace.selected_kind.as_deref() == Some("Reachable")
        });
    let interaction_guest = snapshots
        .iter()
        .find(|(account, _)| account == &interaction_account);
    let interaction_passed = interaction_guest.is_some_and(|(_, guest)| {
        validate_bank_pick_burst_interaction(&guest.trace, &guest.burst).is_ok()
    });
    let all_terminal = snapshots.len() == BANK_PICK_BURST_GUESTS
        && snapshots
            .iter()
            .all(|(_, guest)| matches!(guest.trace.outcome, Some(TellerOutcome::Passed)));
    let no_capacity_leaks = final_capacity == api::work::WorkCapacity::EMPTY;
    let final_service_healthy = final_worker_available && final_health.is_none();
    let reserve_evidence = admission.full_witness.as_ref().map(|witness| {
        serde_json::json!({
            "aggregate_reserved_bytes_for_two_actual_queries":
                witness.capacity.bytes().checked_sub(holder_capacity.bytes()),
            "per_query_reserved_bytes":
                witness.capacity.bytes().checked_sub(holder_capacity.bytes()).map(|bytes| bytes / 2),
            "total_declared_bytes_at_full": witness.capacity.bytes(),
            "full_sized_query_inputs_identical": witness.full_sized_query_inputs_identical,
            "actual_result_bytes": serde_json::Value::Null,
            "actual_result_bytes_unavailable_via": "public WorkAdmission and bank owner publications",
        })
    });
    let evidence = serde_json::json!({
        "scenario": "live_ten_guest_shared_bank_pick_full_retry_burst",
        "timestamp_unix_seconds": epoch,
        "engine": {
            "host": "127.0.0.1",
            "game_port": 44594,
            "http_port": 1080,
            "profile": "local-289",
        },
        "required_environment": {
            "LIVE": "1",
            "BOT_LIVE_NAME_PREFIX": prefix,
            "GATHERER_NAV_PACK": std::env::var("GATHERER_NAV_PACK").ok(),
            "GATHERER_ENGINE_DIR": std::env::var("GATHERER_ENGINE_DIR").ok(),
            "GATHERER_CATALOG_ROOT": std::env::var("GATHERER_CATALOG_ROOT").ok(),
            "GATHERER_GAME_PORT": "44594",
            "GATHERER_HTTP_PORT": "1080",
            "LIVE_EVIDENCE_DIR": evidence_root.display().to_string(),
        },
        "account_names": names,
        "common_readiness_gate_released": gate_released,
        "all_ten_ready_ingame_scene2_before_query": all_prepared,
        "all_ten_native_scripts_started": all_scripts_started,
        "workload": {
            "api": "production native script::bank::Select",
            "explicit_bank": BANK_PICK_BURST_BANK,
            "real_origin": tile_tuple(BANK_PICK_BURST_ORIGIN),
            "query_settle_budget": nav::router::NODE_BUDGET,
            "result_policy": "all ten must return the selected bank with Reachable",
        },
        "selection_outcomes_observed": {
            "Reachable": snapshots.iter().filter(|(_, guest)| {
                guest.trace.selected_kind.as_deref() == Some("Reachable")
            }).count(),
            "AirFallback": snapshots.iter().filter(|(_, guest)| {
                guest.trace.selected_kind.as_deref() == Some("AirFallback")
            }).count(),
        },
        "admission": {
            "job_limit": BANK_PICK_BURST_FULL_JOBS,
            "real_product_terminal_holders": {
                "count": holder_receipts.len(),
                "started_after_all_ten_native_scripts": run_keys.len() == BANK_PICK_BURST_GUESTS,
                "jobs": holder_receipts,
                "capacity_before_queries": {
                    "jobs": holder_capacity.jobs(),
                    "bytes": holder_capacity.bytes(),
                    "handles": holder_capacity.handles(),
                },
                "released_after_full_at_ms": holders_released_at_ms,
                "successful_results_validated_on_release": holders_released_at_ms.is_some(),
            },
            "first_two_accepted": admission
                .full_witness
                .as_ref()
                .is_some_and(|witness| witness.accepted_accounts.len() == 2),
            "third_full_owner_witness": full_witness,
            "first_capacity_release_after_full": admission.first_release_after_full,
            "retry_after_capacity_publication": retried_after_release,
            "work_identity_events": admission.work_events,
            "capacity_snapshots": admission.capacity_snapshots,
            "reservations": reserve_evidence,
            "all_ten_selected": all_selected,
            "final_capacity": {
                "jobs": final_capacity.jobs(),
                "bytes": final_capacity.bytes(),
                "handles": final_capacity.handles(),
            },
            "final_service_healthy": final_service_healthy,
            "final_health": final_health,
            "no_leaked_charges_or_handles": no_capacity_leaks,
            "ticket_retirement": "all ten native Select owners settled; zero charged WorkCapacity",
        },
        "timing_ms": {
            "query_p50": bank_pick_burst_percentile(&query_latencies, 50),
            "query_p99": bank_pick_burst_percentile(&query_latencies, 99),
            "query_max": query_latencies.iter().copied().max(),
            "wall_time": started.elapsed().as_millis(),
        },
        "script_caused_game_behavior": {
            "account": interaction_account,
            "passed": interaction_passed,
            "initial_seed_is_not_success": true,
            "required_observations": [
                "native reachable selection",
                "real movement to the selected bank access tile",
                "loaded open bank session",
                "law rune inventory changed from 1 to 0 on deposit",
                "law rune inventory changed from 0 to 1 on withdrawal",
                "bank session closed",
            ],
        },
        "all_ten_terminal_outcomes_passed": all_terminal,
        "guests": guest_evidence,
        "limits": {
            "actual_route_result_bytes": "not exposed by existing public WorkAdmission or bank owner publications",
            "ticket_count": "not exposed; checked by all ten settled native Select owners and zero charged capacity",
        },
        "outcome": if all_prepared
            && all_scripts_started
            && admission.full_witness.is_some()
            && retried_after_release
            && all_selected
            && interaction_passed
            && all_terminal
            && final_service_healthy
        {
            "passed"
        } else {
            "failed"
        },
    });
    std::fs::write(
        &evidence_file,
        serde_json::to_vec_pretty(&evidence).expect("serialize ten-guest evidence JSON"),
    )
    .unwrap_or_else(|error| panic!("write {}: {error}", evidence_file.display()));
    println!(
        "ten-guest bank-pick evidence={} wall_ms={}",
        evidence_file.display(),
        started.elapsed().as_millis()
    );

    assert!(
        all_prepared,
        "not all ten guests reached scene-2 preparation: {evidence}"
    );
    assert!(
        all_scripts_started,
        "real Play did not start ten native script slots: {evidence}"
    );
    assert!(
        admission.full_witness.is_some(),
        "did not observe two accepted full-sized native bank-pick queries and the third Full refusal with its owner demand retained: {evidence}"
    );
    assert!(
        retried_after_release,
        "the retained third owner demand did not settle after a capacity publication: {evidence}"
    );
    assert!(
        all_selected,
        "not all ten real native Select demands returned the reachable Varrock West bank: {evidence}"
    );
    assert!(
        interaction_passed,
        "selected route did not cause the expected real movement and bank session/inventory changes: {evidence}"
    );
    assert!(
        all_terminal,
        "not all ten native scripts completed successfully: {evidence}"
    );
    assert!(
        no_capacity_leaks,
        "WorkAdmission capacity did not return to zero after all ten query terminals: {evidence}"
    );
    assert!(
        final_service_healthy,
        "shared WorkAdmission became unhealthy during the ten-guest burst: {evidence}"
    );
}
