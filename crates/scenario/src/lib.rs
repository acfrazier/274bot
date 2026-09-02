//! Shared scenario layer: one scenario, two runners.
//!
//! A scenario is a seed (which profiles to log in, mainland hop), a
//! sequence of run steps (each an action send plus the evidence it must
//! produce within a tick budget), and a proof predicate over the terminal
//! `GameSnapshot` — the same observable state scripts and the host read.
//! The headed runner (`panel-play --live script_<name>`) and the headless
//! runner (`crates/e2e` under `LIVE=1`) drive the same
//! [`ScenarioRunner`], so both pass/fail identically. The PASS/FAIL
//! contract replaces the old "5 Running polls" `LiveScript` stub: a
//! scenario proves real game state, not that a poll loop ran.
//!
//! Waiting names evidence, not sleeps (the 377 harness pattern): a step
//! sends once, then waits `budget_ticks` game ticks for a named predicate
//! on the evolving snapshot. Each runner pumps the machine from its own
//! per-frame hook (the panel slot thread headed, the `run_with_io` hook
//! headless), so no thread sleeps inside the shared layer.

pub mod evidence;
pub mod proof;
mod runner;
pub mod shot;

use std::path::PathBuf;
use std::time::Duration;

use api::interact::{
    cheat, op_loc, tele_args, ActionSpec, Driver, Interactions, OpTarget, MAXME_SETSTATS,
};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;

pub use evidence::{Evidence, InvRow, StatRow};
pub use proof::Proof;
pub use runner::{RunnerStatus, ScenarioRunner};

/// Verbose scenario/closer dumps (`BOT_DEBUG=1`). Cached once per process.
pub fn debug_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BOT_DEBUG").is_ok_and(|v| v == "1"))
}

/// The default wall-clock deadline for a whole scenario run (seed + steps
/// + proof). The headless twin uses its own outer timeout.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(180);

/// Boot defaults for a scenario. View knobs are headed-only; deadline /
/// terminal shot / mainland-base gate are consumed by `ScenarioRunner`.
/// `nav` is the session-only nav overlay (paints, camera, tickrate, find
/// flags) — applied for `--live` without writing panel prefs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioSettings {
    pub renderer: bool,
    pub only_render_selected: bool,
    pub capture: bool,
    pub full_rate: bool,
    pub nav: ScenarioNav,
    pub deadline: Duration,
    pub terminal_shot: Option<&'static str>,
    pub require_mainland_base: bool,
    /// Background cheats the runner fires while the scenario is running
    /// (377 sustain: energy, HP, stats). Empty for most scenarios.
    pub sustains: Vec<Sustain>,
}

/// Session-only nav overlay a scenario applies (never persisted prefs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioNav {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub show_nav_path: bool,
    pub hop_labels: bool,
    pub hop_label_px: i32,
    pub collision_fill: bool,
    pub nsew_labels: bool,
    pub client_trail: bool,
    pub component_flood: bool,
    pub camera_follow: bool,
    /// `speed N` cheat once the seed releases. `None` keeps the engine
    /// at 600ms (door-troll / timing cards).
    pub engine_speed_ms: Option<u32>,
}

impl Default for ScenarioNav {
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            show_nav_path: false,
            hop_labels: true,
            hop_label_px: 11,
            collision_fill: false,
            nsew_labels: false,
            client_trail: false,
            component_flood: false,
            camera_follow: false,
            engine_speed_ms: None,
        }
    }
}

/// Visual + camera preset for headed nav tests. Routing flags stay off;
/// tickrate is opt-in via [`ScenarioNav::with_tick_ms`].
pub fn nav_test_paints() -> ScenarioNav {
    ScenarioNav {
        show_nav_path: true,
        hop_labels: true,
        hop_label_px: 11,
        collision_fill: true,
        client_trail: true,
        camera_follow: true,
        nsew_labels: false,
        component_flood: false,
        allow_teleports: false,
        allow_wilderness: false,
        engine_speed_ms: None,
    }
}

impl ScenarioNav {
    pub fn with_tick_ms(mut self, ms: u32) -> Self {
        self.engine_speed_ms = Some(ms);
        self
    }
}

/// A poll or per-leg cheat. `when` is a [`Proof`] that must hold for
/// [`SustainWhen::Poll`]; [`SustainWhen::EachNavStep`] fires at the start
/// of every Walk/Follow/FollowTele (rs2b0t `restoreRunEnergy` each OD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sustain {
    pub when: SustainWhen,
    pub cheat: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SustainWhen {
    /// Fire once when a nav step begins.
    EachNavStep,
    /// Fire every running tick while `proof` holds (e.g. energy ≤ 25).
    Poll(Proof),
}

impl Sustain {
    pub fn each_nav_step(cheat: &'static str) -> Self {
        Self {
            when: SustainWhen::EachNavStep,
            cheat,
        }
    }

    pub fn poll(when: Proof, cheat: &'static str) -> Self {
        Self {
            when: SustainWhen::Poll(when),
            cheat,
        }
    }
}

/// rs2b0t nav energy: `~energy` each WalkTo leg, and whenever run energy
/// is at or below 25 (content debugproc, not engine `energy`).
pub fn nav_energy_sustains() -> Vec<Sustain> {
    vec![
        Sustain::each_nav_step("~energy"),
        Sustain::poll(Proof::StatAtMost { id: 16, max: 25 }, "~energy"),
    ]
}

impl Default for ScenarioSettings {
    fn default() -> Self {
        Self {
            renderer: true,
            only_render_selected: true,
            capture: false,
            full_rate: false,
            nav: ScenarioNav::default(),
            deadline: DEFAULT_DEADLINE,
            terminal_shot: None,
            require_mainland_base: false,
            sustains: Vec::new(),
        }
    }
}

/// Runner-independent seed: login credentials and whether the mainland hop
/// is queued after `scene_state == 2`.
pub struct Seed {
    /// `(username, password)` pairs the runner logs in.
    pub profiles: Vec<(&'static str, &'static str)>,
    /// Queue rs2b0t `mainlandAccount` (tele + setvar) after scene 2.
    pub mainland: bool,
}

/// One run step: an action send plus the evidence it must produce.
pub struct Step {
    pub name: &'static str,
    pub kind: StepKind,
    pub wait: Wait,
}

/// What a step sends through the client's driver.
pub enum StepKind {
    /// 377 `perform(step, { arms, budgetTicks })`: run the closure once,
    /// then wait for `wait.arm` within `wait.budget_ticks`.
    Perform {
        #[allow(clippy::type_complexity)]
        send: Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>,
    },
    /// Nav walk: arm `nav::router::find` over the collision + transport
    /// graph derived from the baked pack and drive `Traveller::follow`
    /// one step per tick until arrival. Identical to `Follow`; the wait
    /// arm is `arrived(dest)`, the proof mirrors it, so a walk that ends
    /// anywhere but the destination fails the step with the terminal
    /// outcome's message.
    Walk { dest: WorldTile },
    /// Whole-world nav: arm `nav::router::find` over the collision +
    /// transport graph derived from the baked pack and drive
    /// `Traveller::follow` one step per tick until the route terminates.
    /// The wait arm is `arrived(dest)`; the proof mirrors it, so a follow
    /// that ends anywhere but the destination fails the step with the
    /// terminal outcome's message.
    Follow { dest: WorldTile },
    /// Whole-world nav with the any-tile teleport layer on: like
    /// [`StepKind::Follow`], but the route arms with
    /// `FindOptions::allow_teleports`, so a destination only the packed
    /// spell/jewellery teleport edges reach routes (and the traveller
    /// executes the packed op — never the WalkTo `::tele` cheat).
    FollowTele { dest: WorldTile },
    /// Whole-window shot at the moment `wait.arm` holds: nothing is sent,
    /// then the runner fires the shot sink (headed: the panel captures
    /// the window; headless: a no-op) with the label + the terminal
    /// snapshot. The shot lands under `~/.274bot/smoke/<runId>/` as
    /// `<stamp>_<safeLabel>.png` + a `.json` sidecar.
    Shot { label: &'static str },
    /// Dialog janitor: drain the seed's dialogs **every tick** until
    /// `wait.arm` holds — answer the chat modal's `choice`-th button
    /// (1-based) when a `p_choiceN` dialog is up (the `~completequests`
    /// debugproc's Arrav-gang and Ikov-side prompts), else close the
    /// modal when one is up (the quest-completion quest scrolls, whose
    /// open main modal stalls the engine's script queue). A no-op send is
    /// harmless, so the re-send drains each dialog as it appears.
    DrainDialogs { choice: i32 },
    /// Clean IF_BUTTON logout (CC_LOGOUT), pressed once. The slot thread
    /// leaves `run_client` when `!ingame` and the host-play login FIFO
    /// handshakes again — do not call blocking `Client::login` from the
    /// runner (that freezes the panel on "logging in…"). Side icons and
    /// quest-list colour refresh from that login payload.
    Relog,
    /// Like [`StepKind::Perform`], but re-sends every tick until `wait.arm`
    /// holds (`getvar` polls, sticky `setvar tutorial`).
    Repeat {
        #[allow(clippy::type_complexity)]
        send: Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>,
    },
}

/// Evidence wait for a step: a named predicate and a tick budget.
pub struct Wait {
    /// The named evidence arm the wait ends on.
    pub arm: Proof,
    /// Game ticks (delivered server frames) the wait may take.
    pub budget_ticks: u32,
}

/// A whole scenario: seed + run steps + a proof predicate.
pub struct Scenario {
    pub name: &'static str,
    pub seed: Seed,
    pub steps: Vec<Step>,
    /// The terminal proof: asserted on the snapshot once the run steps
    /// complete (replaces the "is the script polling Running" stub).
    pub proof: Proof,
    /// Extra profile slots with their own per-frame hooks: the scenario
    /// runs as a fleet. Profile 0 is the driven scenario slot; each
    /// companion names another `seed.profiles` index (so `>= 1`) the
    /// runner ticks through [`ScenarioRunner::companion_tick`] exactly
    /// like the driven slot's `tick`. Empty for single-bot scenarios.
    pub companions: Vec<Companion>,
    /// Boot settings for the scenario: view knobs for the headed panel and
    /// runner-consumed deadline / terminal shot / mainland-base gate.
    pub settings: ScenarioSettings,
}

/// One extra profile slot in a scenario fleet: `profile` is an index into
/// `seed.profiles` (0 is the driven scenario slot, so companions are
/// `>= 1`) and `per_frame` is the slot's per-frame hook, run once per
/// delivered frame by the same per-frame machinery that ticks the driven
/// slot.
pub struct Companion {
    pub profile: usize,
    pub per_frame: Box<dyn FnMut(&mut Client) + Send>,
}

/// The registered scenario with this name, `None` when unknown.
pub fn get(name: &str) -> Option<Scenario> {
    match name {
        "walk" => Some(walk_scenario()),
        "render_smoke" => Some(render_smoke_scenario()),
        "nav_full" => Some(nav_full_scenario()),
        "nav_door" => Some(nav_door_scenario()),
        "nav_cart" => Some(nav_cart_scenario()),
        "nav_essence" => Some(nav_essence_scenario()),
        "nav_elkoy" => Some(nav_elkoy_scenario()),
        "nav_tele" => Some(nav_tele_scenario()),
        "nav_shantay" => Some(nav_shantay_scenario()),
        "nav_routes" => Some(nav_routes_scenario()),
        "nav_paint_path" => Some(nav_paint_path_scenario()),
        "bone_burier" => Some(bone_burier_scenario()),
        _ => None,
    }
}

/// Every registered scenario name (for the `--live script_<name>` usage).
pub fn names() -> Vec<&'static str> {
    vec![
        "walk",
        "render_smoke",
        "nav_full",
        "nav_door",
        "nav_cart",
        "nav_essence",
        "nav_elkoy",
        "nav_tele",
        "nav_shantay",
        "nav_routes",
        "nav_paint_path",
        "bone_burier",
    ]
}

/// The `render_smoke` scenario: log in `test`/`test`, do nothing, and
/// fire one whole-window shot the tick the seed gate releases. The
/// scenario's own settings carry the relaxed mainland-base seed gate
/// (`require_mainland_base = false`), so the capture lands the tick the
/// focused slot first reaches `ingame && scene_state == 2`; the
/// `stat(16) >= 0` arm always holds on a rebuilt snapshot, so the shot
/// fires immediately. The proof mirrors the arm so the run PASSes once
/// the capture is requested — the panel exits 0 when the PNG is written,
/// not on the runner status.
fn render_smoke_scenario() -> Scenario {
    Scenario {
        name: "render_smoke",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "capture scene 2",
            kind: StepKind::Shot { label: "scene2" },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 0 },
                budget_ticks: 1,
            },
        }],
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings {
            deadline: Duration::from_secs(300),
            ..Default::default()
        },
    }
}

/// The `walk` scenario: log in `test`/`test`, mainland-hop into the
/// Lumbridge courtyard, walk south across the open courtyard in two steps,
/// and prove the player is standing at (3220, 3212, 0). The landing tile
/// after the mainland tele is (3220, 3220) or (3220, 3222); both steps
/// route through open, walkable tiles on the whole-world `NavWorld`.
fn walk_scenario() -> Scenario {
    let mid = WorldTile {
        x: 3220,
        z: 3216,
        level: 0,
    };
    let dest = WorldTile {
        x: 3220,
        z: 3212,
        level: 0,
    };
    Scenario {
        name: "walk",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "walk to courtyard mid",
                kind: StepKind::Walk { dest: mid },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: mid.x,
                        z: mid.z,
                        level: 0,
                    },
                    budget_ticks: 90,
                },
            },
            Step {
                name: "walk to courtyard south",
                kind: StepKind::Walk { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: 0,
                    },
                    budget_ticks: 90,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

/// The `nav_full` scenario: log in `test`/`test`, mainland-hop into the
/// Lumbridge courtyard, then drive the whole-world nav proof. `find` runs
/// the Dijkstra router over the **live scene's** collision map (so the
/// route is one the client can actually walk — the baked pack's boolean
/// walk grid can diverge from the live collision) plus the transport
/// graph derived from the baked whole-world pack, and `Traveller::follow`
/// drives the route one step per tick until arrival. The destination is a
/// concrete Lumbridge tile from the pack — (3220, 3264, 0), 44 chebyshev
/// tiles north of the tele landing — in mapsquare (50,51), which the
/// pre-bake 2-square pack (m50_50 + m44_53) never covered, so the walk
/// crosses the z=3264 square boundary. It is walk-only (no boat/teleport:
/// those have no content-derivable origin tile); the route is checked at
/// arm time, and the run fails with a clear message if no walk path
/// exists. The step budget is sized for a ~100-tile walk plus re-routing.
pub fn nav_full_scenario() -> Scenario {
    let dest = WorldTile {
        x: 3220,
        z: 3264,
        level: 0,
    };
    Scenario {
        name: "nav_full",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![Step {
            name: "follow the whole-world route",
            kind: StepKind::Follow { dest },
            wait: Wait {
                arm: Proof::Arrived {
                    x: dest.x,
                    z: dest.z,
                    level: dest.level,
                },
                budget_ticks: 600,
            },
        }],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_full terminal"),
            nav: nav_test_paints().with_tick_ms(300),
            ..Default::default()
        },
    }
}

/// The `nav_door` scenario: the two-bot door slam as a scenario fleet.
/// Profile 0 (`test`) is the driven walker: after the mainland seed it
/// cheat-teles to the Catherby range-house `OUTSIDE` stand, then `Follow`s
/// a whole-world route through the door to `DEST` inside. Profile 1
/// (`test2`) is the closer companion: it cheat-teles inside, then
/// `op_loc`s the door shut on every player-info tick the instant the door
/// reads open, so the walker's follow is stressed against a tick-perfect
/// closer. The follow's terminal outcome at the door is the diagnostic
/// target (the closer slams the door shut as the walker approaches, which
/// surfaces as `Refused Unreachable` in the traveller's settle). The
/// companion's gating mirrors the old harness: the first `scene_state ==
/// 2` frame is skipped (host-play queues `mainland_hop` after the hook),
/// and the Catherby tele waits until `here` is the mainland courtyard.
fn nav_door_scenario() -> Scenario {
    let outside = WorldTile {
        x: OUTSIDE.x,
        z: OUTSIDE.z,
        level: 0,
    };
    let dest = WorldTile {
        x: DEST.x,
        z: DEST.z,
        level: 0,
    };
    Scenario {
        name: "nav_door",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "tele the walker to the Catherby outside stand",
                kind: StepKind::Perform {
                    // Return true on send: the cheat is queued through the
                    // ISAAC sink, the arm waits for the tele to land.
                    send: Box::new(|c, _| cheat(c, WALKER_TELE)),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: outside.x,
                        z: outside.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow through the range-house door",
                kind: StepKind::Follow { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![Companion {
            profile: 1,
            per_frame: {
                let mut slot = CloserSlot::default();
                Box::new(move |c| closer_frame(c, &mut slot))
            },
        }],
        settings: ScenarioSettings {
            full_rate: true,
            only_render_selected: false,
            nav: nav_test_paints(),
            ..Default::default()
        },
    }
}

/// Closed Catherby range-house door (loc 1530) the closer slams.
const DOOR: WorldTile = WorldTile {
    x: 2816,
    z: 3438,
    level: 0,
};
const CLOSED_ID: i32 = 1530;
const OPEN_ID: i32 = 1531;
/// Briefed outside stand (west of pack origin 2816; on-pack fallback is
/// (2816,3436), the walkable tile south of the door).
const OUTSIDE: WorldTile = WorldTile {
    x: 2813,
    z: 3436,
    level: 0,
};
/// Inside stand, north of the door.
const DEST: WorldTile = WorldTile {
    x: 2817,
    z: 3443,
    level: 0,
};
/// `::tele` to OUTSIDE (level, mx, mz, lx, lz).
const WALKER_TELE: &str = "tele 0,43,53,61,44";
/// Inside, diagonal to the door (2817,3439) — off the 2816 corridor.
const CLOSER_TELE: &str = "tele 0,44,53,1,47";

/// Per-frame state of the closer companion (profile 1), owned by the
/// companion's closure. All plain fields, so the closure is `Send`.
#[derive(Default)]
struct CloserSlot {
    /// First `scene_state == 2` was observed; that frame host-play still
    /// queues `mainland_hop` after `per_frame`, so the Catherby tele
    /// waits a tick.
    scene2_seen: bool,
    tele_sent: bool,
    last_gen: u64,
}

/// The closer companion's per-frame hook: gate the Catherby tele behind
/// the mainland hop, then on every player-info tick `op_loc` the live
/// open leaf (id 1531, often a tile off packed `at`) so it slams shut.
fn closer_frame(c: &mut Client, s: &mut CloserSlot) {
    let Some(lp) = &c.local_player else {
        if debug_enabled() {
            eprintln!("[nav-closer] no local_player scene={}", c.scene_state);
        }
        return;
    };
    let here = WorldTile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    if stage_closer_tele(c, here, s) {
        return;
    }
    if c.gens.player == s.last_gen {
        return;
    }
    s.last_gen = c.gens.player;
    let loc = wall_loc(c, DOOR);
    if debug_enabled() {
        eprintln!(
            "[nav-closer] here={here:?} loc={loc:?} tele_sent={} scene={}",
            s.tele_sent, c.scene_state
        );
    }
    let Some((loc_tile, loc_id)) = loc else {
        return;
    };
    if loc_id == CLOSED_ID {
        return;
    }
    // OP_LOC1 on the live open leaf is Close. The leaf often sits a tile
    // off packed `at`; slamming packed `at` looks up an empty typecode.
    if debug_enabled() {
        eprintln!(
            "[nav-closer] SLAM op_loc id={loc_id} at ({},{}) packed=({},{})",
            loc_tile.x, loc_tile.z, DOOR.x, DOOR.z
        );
    }
    op_loc(c, loc_tile.x, loc_tile.z, loc_id);
}

/// Host-play queues `mainland_hop` *after* `per_frame` on the first
/// `scene_state == 2`. Skip that frame; Catherby-tele only once `here` is
/// the Lumbridge courtyard (or `x > 3100` and not already at Catherby).
fn stage_closer_tele(c: &mut Client, here: WorldTile, s: &mut CloserSlot) -> bool {
    if at_catherby(here) || s.tele_sent {
        return false;
    }
    if c.scene_state != 2 {
        return false;
    }
    if !s.scene2_seen {
        s.scene2_seen = true;
        return false;
    }
    if !at_lumbridge(here) && here.x <= 3100 {
        if debug_enabled() {
            eprintln!(
                "[nav-closer] waiting mainland, here={here:?} scene={}",
                c.scene_state
            );
        }
        return false;
    }
    if debug_enabled() {
        eprintln!("[nav-closer] tele {CLOSER_TELE} from {here:?}");
    }
    cheat(c, CLOSER_TELE);
    s.tele_sent = true;
    true
}

fn wall_loc(c: &Client, tile: WorldTile) -> Option<(WorldTile, i32)> {
    let (bx, bz) = c.build_base();
    nearest_door_loc(tile, |x, z| {
        c.loc_typecode(x - bx, z - bz).map(|tc| (tc >> 14) & 0x7fff)
    })
}

/// Nearest 1530/1531 within chebyshev 3 of packed `at`. Returns the live
/// tile — the Catherby open leaf sits a tile off that origin, and
/// `op_loc` must click the leaf, not packed `at`.
fn nearest_door_loc(
    packed: WorldTile,
    lookup: impl Fn(i32, i32) -> Option<i32>,
) -> Option<(WorldTile, i32)> {
    let mut best: Option<(i32, WorldTile, i32)> = None;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let gap = dx.abs().max(dz.abs());
            if gap > 3 {
                continue;
            }
            let x = packed.x + dx;
            let z = packed.z + dz;
            let Some(id) = lookup(x, z) else {
                continue;
            };
            if id != CLOSED_ID && id != OPEN_ID {
                continue;
            }
            if best.map(|(g, _, _)| gap < g).unwrap_or(true) {
                best = Some((
                    gap,
                    WorldTile {
                        x,
                        z,
                        level: packed.level,
                    },
                    id,
                ));
            }
        }
    }
    best.map(|(_, tile, id)| (tile, id))
}

fn at_lumbridge(here: WorldTile) -> bool {
    here.x >= 3200 && here.x < 3264 && here.z >= 3200 && here.z < 3264
}

fn at_catherby(here: WorldTile) -> bool {
    here.x >= 2800 && here.x < 2860 && here.z >= 3420 && here.z < 3460
}

/// Vigroy's Shilo Village cart driver tile (npc 511, m44_46 local
/// (18,10)): the `at` of the packed Shilo→Brimhaven Npc edge.
const SHILO_DRIVER: WorldTile = WorldTile {
    x: 2834,
    z: 2954,
    level: 0,
};
/// The Shilo→Brimhaven cart's landing tile (m43_50 local (24,14)): the
/// packed edge's `to`.
const BRIM_CART: WorldTile = WorldTile {
    x: 2776,
    z: 3214,
    level: 0,
};

/// The `nav_cart` scenario: the first OP_NPC execute follow. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, cheat-give
/// the fare (the packed Shilo→Brimhaven Npc edge carries a 200-coin
/// `item_req`), cheat-tele to the Shilo Village cart driver, then
/// `Follow` a whole-world route to the Brimhaven cart landing — a
/// destination that **requires** the cart hop (the only way across the
/// sea to the Brimhaven side). The traveller interacts the driver
/// (`OpTarget::Npc` + option 1), answers the driver's fare dialog (the
/// "Yes please…" choice), and PASSes on `TravelOutcome::Arrived` at the
/// landing.
fn nav_cart_scenario() -> Scenario {
    let driver = SHILO_DRIVER;
    let dest = BRIM_CART;
    Scenario {
        name: "nav_cart",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "give the fare and tele to the Shilo cart driver",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "give coins 500");
                        cheat(c, &tele_args(driver.level, driver.x, driver.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: driver.x,
                        z: driver.z,
                        level: driver.level,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow the cart to Brimhaven",
                kind: StepKind::Follow { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_cart terminal"),
            ..Default::default()
        },
    }
}

/// Aubury's Varrock rune-shop anchor (3253,3401): `^essence_mine_to_aubury`
/// = `0_50_53_53_9` — the tile the mine exit portal returns to after
/// entering through Aubury.
const AUBURY_ANCHOR: WorldTile = WorldTile {
    x: 3253,
    z: 3401,
    level: 0,
};
/// The Rune Essence mine pad (m45_75 local (32,33)): the packed entry
/// edge's landing anchor (the real landing is randomised in the mine).
const MINE_PAD: WorldTile = WorldTile {
    x: 2912,
    z: 4833,
    level: 0,
};

/// The `nav_essence` scenario: the EssenceSession execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, then seed
/// Rune Mysteries by painting the quest journal green: the
/// `~completequests` debugproc completes every quest and calls
/// `~update_questlist` (a bare `setvar runemysteries 6` leaves the
/// client's journal colours stale, so `WorldState::from_snapshot` would
/// still gate the packed entry edges closed), but it first opens two
/// `p_choice` dialogs — a `ChatAnswer` janitor answers them as they
/// appear and the step waits on the journal going green. Then cheat-tele
/// to Aubury's shop, `Follow` into the mine (the entry hop latches the
/// session on any mine landing), and `Follow` back out through the exit
/// portal — the session-gated return may only land near Aubury, never
/// another wizard. PASSes when the player stands within the exit landing
/// radius of Aubury's anchor.
fn nav_essence_scenario() -> Scenario {
    Scenario {
        name: "nav_essence",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "complete Rune Mysteries",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~completequests");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ChatChoice,
                    budget_ticks: 20,
                },
            },
            Step {
                name: "answer the quest-seed dialogs until the journal is green",
                kind: StepKind::DrainDialogs { choice: 1 },
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Rune Mysteries Quest",
                    },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to Aubury's shop",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(
                            c,
                            &tele_args(AUBURY_ANCHOR.level, AUBURY_ANCHOR.x, AUBURY_ANCHOR.z),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: AUBURY_ANCHOR.x,
                        z: AUBURY_ANCHOR.z,
                        level: AUBURY_ANCHOR.level,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow into the essence mine",
                kind: StepKind::Follow { dest: MINE_PAD },
                wait: Wait {
                    arm: Proof::EssenceMine,
                    budget_ticks: 600,
                },
            },
            Step {
                name: "follow out to Aubury through the exit portal",
                kind: StepKind::Follow {
                    dest: AUBURY_ANCHOR,
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: AUBURY_ANCHOR.x,
                        z: AUBURY_ANCHOR.z,
                        level: AUBURY_ANCHOR.level,
                        radius: 2,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::ArrivedNear {
            x: AUBURY_ANCHOR.x,
            z: AUBURY_ANCHOR.z,
            level: AUBURY_ANCHOR.level,
            radius: 2,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_essence terminal"),
            ..Default::default()
        },
    }
}

/// The maze-side Elkoy (npc 473, m39_49 local (8,55)): the `at` of the
/// packed maze→village escort edge, one tile south of the entrance coord
/// (2504,3192).
const ELKOY_MAZE_SIDE: WorldTile = WorldTile {
    x: 2504,
    z: 3191,
    level: 0,
};
/// The village maze coord (`^elkoy_maze_coord = 0_39_49_19_23`): the
/// packed escort edge's `to` — the exact tile the maze-side Elkoy's
/// `p_telejump(` lands on (the script's own landing, never a snap).
const ELKOY_MAZE_COORD: WorldTile = WorldTile {
    x: 2515,
    z: 3159,
    level: 0,
};

/// The `nav_elkoy` scenario: the Elkoy OP_NPC execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, then seed
/// Tree Gnome Village by painting the quest journal green: the
/// `~completequests` debugproc completes every quest and calls
/// `~update_questlist` (a bare `setvar treequest …` leaves the client's
/// journal colours stale, so `WorldState::from_snapshot` would still gate
/// the packed escort edges closed), but it first opens two `p_choice`
/// dialogs — a `ChatAnswer` janitor answers them as they appear and the
/// step waits on the journal going green. Then cheat-tele to the maze-side
/// Elkoy, and `Follow` a whole-world route into the village — across the
/// hedge maze, whose 1-tick escort hop beats the maze walk in the router,
/// so the route executes the packed `TransportKind::Npc` edge. The
/// traveller talks to Elkoy
/// (`OpTarget::Npc` + option 1), answers the escort dialog's "Yes please."
/// choice (the chat modal's first), and PASSes on `TravelOutcome::Arrived`
/// at the packed `edge.to` (2515,3159).
fn nav_elkoy_scenario() -> Scenario {
    let driver = ELKOY_MAZE_SIDE;
    let dest = ELKOY_MAZE_COORD;
    Scenario {
        name: "nav_elkoy",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "complete Tree Gnome Village",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~completequests");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ChatChoice,
                    budget_ticks: 20,
                },
            },
            Step {
                name: "answer the quest-seed dialogs until the journal is green",
                kind: StepKind::DrainDialogs { choice: 1 },
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Tree Gnome Village",
                    },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to the maze-side Elkoy",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(driver.level, driver.x, driver.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: driver.x,
                        z: driver.z,
                        level: driver.level,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow Elkoy into the village",
                kind: StepKind::Follow { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_elkoy terminal"),
            ..Default::default()
        },
    }
}

/// The packed dueling-ring landing: the Al Kharid Duel Arena (m51_50
/// local (51,35)) — the `to` of the packed `ring_of_dueling_8` rub edge
/// (obj 2552, `opheld4`), a random standable tile within the
/// `map_findsquare` scatter, never the tile exactly.
const DUEL_ARENA: WorldTile = WorldTile {
    x: 3315,
    z: 3235,
    level: 0,
};

/// The `nav_tele` scenario: the packed Teleport execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, clear the
/// persistent slot's backpack (`~clearinv`) and cheat-give a charged
/// dueling ring (the packed jewellery rub edge's `item_req`),
/// then `Follow` with `allow_teleports` on to the Al Kharid Duel Arena —
/// a destination the 2-tick rub edge always beats (the walk is toll-
/// gated and over a hundred ticks, so the packed leg only ever routes
/// when the layer is on and the item is held). The traveller rubs the
/// held ring (`OpTarget::Item` + option 4 — never the WalkTo `::tele`
/// cheat), answers the destination choice the rub opens, and PASSes on
/// `TravelOutcome::Arrived` within the packed landing's scatter radius.
fn nav_tele_scenario() -> Scenario {
    let dest = DUEL_ARENA;
    Scenario {
        name: "nav_tele",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "clear the backpack and give the charged dueling ring",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        // The persistent `test` slot's backpack fills up
                        // across prior live twins, so the give silently
                        // drops when there is no room — clear the default
                        // inventory first (`[debugproc,clearinv]`).
                        cheat(c, "~clearinv");
                        cheat(c, "give ring_of_dueling_8 1");
                        true
                    }),
                },
                // The arm waits for the ring to actually land in the
                // inventory: the WorldState of the follow step then proves
                // the packed rub edge's `item_req`, or the router falls
                // back to the walk.
                wait: Wait {
                    arm: Proof::Item {
                        name: "Ring of dueling(8)",
                        count: 1,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "follow the packed ring rub to the Duel Arena",
                kind: StepKind::FollowTele { dest },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                        radius: 2,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::ArrivedNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius: 2,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_tele terminal"),
            ..Default::default()
        },
    }
}

/// The Shantay henge desert stand (m51_48 local (38,38) = (3302,3110),
/// south of the gate on the desert side `coordz <= loc z`).
const SHANTAY_DESERT_START: WorldTile = WorldTile {
    x: 3302,
    z: 3110,
    level: 0,
};
/// The Al Kharid-side stand (m51_48 local (40,47) = (3304,3119), north
/// of the gate).
const SHANTAY_PASS_START: WorldTile = WorldTile {
    x: 3304,
    z: 3119,
    level: 0,
};
/// The Al Kharid-side follow dest (m51_48 local (36,48) = (3300,3120)).
const SHANTAY_PASS_DEST: WorldTile = WorldTile {
    x: 3300,
    z: 3120,
    level: 0,
};
/// The desert follow dest (m51_48 local (39,36) = (3303,3108)).
const SHANTAY_DESERT_DEST: WorldTile = WorldTile {
    x: 3303,
    z: 3108,
    level: 0,
};

/// The `nav_shantay` scenario: both directions through the Shantay henge
/// (loc 4031, `shantay_pass.rs2` `oploc1`), driven by `Traveller::follow`
/// like every nav twin. The desert → pass leg follows with an empty
/// inventory — the free desert exit edge (`coordz(coord) <=
/// coordz(loc_coord)` telejump, no `item_req`) is the only Shantay edge
/// the fail-closed WorldState relaxes. The pass → desert leg clears the
/// backpack (`[debugproc,clearinv]` — the shared `test` slot may carry
/// junk from prior live twins), `give`s one Shantay pass, and follows
/// through the gated hop (consume pass + `[queue,shantay_pass_enter]`),
/// the only edge into the desert. PASS is standing on the desert dest.
fn nav_shantay_scenario() -> Scenario {
    Scenario {
        name: "nav_shantay",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "clear the backpack and tele to the desert stand",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        cheat(c, "tele 0,51,48,38,38");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_DESERT_START.x,
                        z: SHANTAY_DESERT_START.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow the free desert exit to Al Kharid",
                kind: StepKind::Follow {
                    dest: SHANTAY_PASS_DEST,
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_PASS_DEST.x,
                        z: SHANTAY_PASS_DEST.z,
                        level: 0,
                    },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "clear the backpack and give the Shantay pass",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        // The gated branch shows its first-crossing
                        // disclaimer dialog (three mesboxes + a "Go into
                        // Desert?" choice) when the player lacks the
                        // disclaimer item — give the disclaimer the script
                        // hands out after any prior crossing so the branch
                        // goes straight to the pass-handover chat the
                        // traveller drives for the loc-4031 hop.
                        cheat(c, "give thshantaydisc 1");
                        cheat(c, "give shantay_pass 1");
                        true
                    }),
                },
                // The arm waits for the pass to actually land: the
                // WorldState of the follow step then proves the packed
                // gated edge's `item_req`, or the router fails closed.
                wait: Wait {
                    arm: Proof::Item {
                        name: "Shantay pass",
                        count: 1,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "tele to the Al Kharid stand",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| cheat(c, "tele 0,51,48,40,47")),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_PASS_START.x,
                        z: SHANTAY_PASS_START.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow the pass-gated hop into the desert",
                kind: StepKind::Follow {
                    dest: SHANTAY_DESERT_DEST,
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_DESERT_DEST.x,
                        z: SHANTAY_DESERT_DEST.z,
                        level: 0,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: SHANTAY_DESERT_DEST.x,
            z: SHANTAY_DESERT_DEST.z,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(420),
            terminal_shot: Some("nav_shantay terminal"),
            ..Default::default()
        },
    }
}

/// Borrowed OD pairs: rs2b0t `script-routes.hardest.json` plus
/// `transport-heavy.routes.json` / boat table so we hit walk, stairs,
/// doors, Karamja fare, slashable web, and gnome glider. Teleports off.
const NAV_ROUTES: &[(&str, WorldTile, WorldTile)] = &[
    // HARD COMMUTE-14-R
    (
        "Seers bank → RockCrab field",
        WorldTile {
            x: 2725,
            z: 3491,
            level: 0,
        },
        WorldTile {
            x: 2710,
            z: 3720,
            level: 0,
        },
    ),
    // HARD WALK-5-4
    (
        "Taverley → Rellekka",
        WorldTile {
            x: 2895,
            z: 3435,
            level: 0,
        },
        WorldTile {
            x: 2668,
            z: 3660,
            level: 0,
        },
    ),
    // HARD WALK-3-10
    (
        "Ardougne → Yanille",
        WorldTile {
            x: 2661,
            z: 3301,
            level: 0,
        },
        WorldTile {
            x: 2612,
            z: 3092,
            level: 0,
        },
    ),
    // HARD BOT-ClueSolver-8-6 (stairs, upstairs houses)
    (
        "Falador house → Rimmington house",
        WorldTile {
            x: 3040,
            z: 3364,
            level: 1,
        },
        WorldTile {
            x: 2970,
            z: 3215,
            level: 1,
        },
    ),
    // Boat: Port Sarim seaman (npc 378 @ 3026,3217) 30-coin fare → Musa dock.
    (
        "Port Sarim → Musa Point",
        WorldTile {
            x: 3029,
            z: 3217,
            level: 0,
        },
        WorldTile {
            x: 2956,
            z: 3146,
            level: 0,
        },
    ),
    // Boat back: customs officer @ 2955,3146.
    (
        "Musa Point → Port Sarim",
        WorldTile {
            x: 2954,
            z: 3146,
            level: 0,
        },
        WorldTile {
            x: 3029,
            z: 3217,
            level: 0,
        },
    ),
    // rs2b0t two-route smoke: Yanille bank → chaos-druid field (web + stairs + ledge).
    (
        "Yanille bank → dungeon warriors",
        WorldTile {
            x: 2612,
            z: 3092,
            level: 0,
        },
        WorldTile {
            x: 2580,
            z: 9501,
            level: 0,
        },
    ),
    // TH-glider-gandius-hub (varp grandtree >= 160).
    (
        "Gandius glider → Grand Tree hub",
        WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        },
        WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
    ),
    // TH-glider-hub-karhewo.
    (
        "Grand Tree hub → Kar-Hewo",
        WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
        WorldTile {
            x: 3284,
            z: 3211,
            level: 0,
        },
    ),
    // Live door we just proved.
    (
        "Catherby range-house door",
        WorldTile {
            x: 2813,
            z: 3436,
            level: 0,
        },
        WorldTile {
            x: 2817,
            z: 3443,
            level: 0,
        },
    ),
];

/// Headed corpus smoke: mainland hop (unique live accounts spawn on
/// tutorial), `setvar` the transport-quest perm varps, clean logout +
/// login so the quest list and side icons refresh from the login payload
/// (no `~completequests` dialog pile), then the rs2b0t item kit (knife,
/// coins) and tele+Follow each borrowed OD pair. Proof is arrival at the
/// last dest. Teleports off. Auto-run is the host bothost feature.
fn nav_routes_scenario() -> Scenario {
    let last = NAV_ROUTES[NAV_ROUTES.len() - 1].2;
    let mut steps = nav_kit_steps();
    for (note, from, to) in NAV_ROUTES {
        steps.push(tele_step(note, *from));
        steps.push(follow_step(note, *to));
    }
    Scenario {
        name: "nav_routes",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::Arrived {
            x: last.x,
            z: last.z,
            level: last.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            require_mainland_base: true,
            deadline: Duration::from_secs(3600),
            sustains: nav_energy_sustains(),
            ..Default::default()
        },
    }
}

/// The `nav_paint_path` scenario: log in `test`/`test`, mainland-hop into
/// the Lumbridge courtyard, and walk ~8 tiles south in one `Walk` step.
/// `nav` arms the panel's live overlay, so the headed runner shows
/// the red baked path clipped to the viewport, the cyan client trail, and
/// (with run on) the two-tone run-alt trail. No closer, no transport — the
/// whole point is a plain courtyard path the camera can hold.
fn nav_paint_path_scenario() -> Scenario {
    let dest = WorldTile {
        x: 3220,
        z: 3212,
        level: 0,
    };
    Scenario {
        name: "nav_paint_path",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![Step {
            name: "walk to courtyard south",
            kind: StepKind::Walk { dest },
            wait: Wait {
                arm: Proof::Arrived {
                    x: dest.x,
                    z: dest.z,
                    level: 0,
                },
                budget_ticks: 90,
            },
        }],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            require_mainland_base: true,
            ..Default::default()
        },
    }
}

/// The `bone_burier` scenario: the live BoneBurier twin (rs2b0t's listed
/// script, exercised as a scenario). Log in a unique minted account,
/// mainland-hop into the Lumbridge courtyard, stick `tutorial=1000`, then
/// relog so the tutorial lock releases the side icons (the inv tab's
/// TYPE_INV widget only binds on a clean login payload — a bare setvar
/// leaves the inv view empty). Then seed five Bones with the `give` cheat
/// and bury them until at most three remain — the server's
/// `bury_bone.rs2` deletes the slot, advances prayer, and prints "You
/// bury the bones." Proof is the bury message in the chat ring: a burial
/// actually happened, not just a count read.
fn bone_burier_scenario() -> Scenario {
    Scenario {
        name: "bone_burier",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "stick tutorial skip",
                kind: StepKind::Repeat {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get tutorial: 1000",
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "relog so the inv tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    // Same arm as the nav kit: after the clean logout the
                    // login payload binds side tab 3 (inventory).
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "seed five bones",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "give bones 5");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Item {
                        name: "Bones",
                        count: 5,
                    },
                    budget_ticks: 80,
                },
            },
            Step {
                name: "bury bones until at most three remain",
                kind: StepKind::Repeat {
                    send: Box::new(|c, snap| {
                        // Held-item Bury (opheld1 on the obj): the label
                        // resolves through the same snapshot the closure
                        // reads. A refusal never fails the run — the arm
                        // needs the server-side count drop.
                        let Some(item) = snap.inventory().iter().find(|it| it.def.id == 526) else {
                            return true;
                        };
                        let mut ix = Interactions::new(snap, c);
                        let _ = ix.interact(OpTarget::Item(item), ActionSpec::Label("Bury".into()));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ItemAtMost {
                        name: "Bones",
                        count: 3,
                    },
                    budget_ticks: 200,
                },
            },
        ],
        proof: Proof::Chat {
            needle: "bury the bones",
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(300),
            ..Default::default()
        },
    }
}
/// Journal colour is login-time `~update_questlist`; the Relog step after
/// these cheats is what actually opens packed quest-gated edges.
const TRANSPORT_QUEST_SETVARS: &[&str] = &[
    "setvar runemysteries 6",
    "setvar grandtree 160",
    "setvar treequest 9",
    "setvar zombiequeen 15",
    "setvar priestperil 60",
    "setvar elenaquest 30",
    "setvar itwatchtower 14",
    "setvar eadgar_quest 110",
    "setvar waterfall_quest 10",
    "setvar dragonquest 10",
    "setvar desertrescue 30",
];

/// Live nav kit. Mainland hop, stick `tutorial=1000`, quest `setvar`s,
/// clean Relog (side tab 3 bound), then rs2b0t `seedTeleKit`: knife, coins,
/// runes, charged jewellery. Auto-run is host bothost.
fn nav_kit_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip",
            kind: StepKind::Repeat {
                send: Box::new(|c, _| {
                    cheat(c, "setvar tutorial 1000");
                    cheat(c, "getvar tutorial");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Chat {
                    needle: "get tutorial: 1000",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "setvar transport quests",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    for cmd in TRANSPORT_QUEST_SETVARS {
                        cheat(c, cmd);
                    }
                    true
                }),
            },
            wait: Wait {
                // Already on the mainland courtyard after the seed hop;
                // this only sequences the cheats before Relog.
                arm: Proof::Arrived {
                    x: 3220,
                    z: 3220,
                    level: 0,
                },
                budget_ticks: 30,
            },
        },
        Step {
            name: "relog so journal, side icons, and tutorial lock refresh",
            kind: StepKind::Relog,
            wait: Wait {
                // rs2b0t mainlandAccount: sideIcon[3] bound after relog.
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
        Step {
            name: "journal shows Grand Tree complete",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::QuestDone {
                    name: "The Grand Tree",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "maxme setstat 99 (debug heading, not ~maxme)",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    for cmd in MAXME_SETSTATS {
                        cheat(c, cmd);
                    }
                    true
                }),
            },
            wait: Wait {
                // Attack is skill 0; Stat id 16 is run energy, not agility.
                arm: Proof::Stat { id: 0, min: 99 },
                budget_ticks: 80,
            },
        },
        seed_give("seed knife", "knife", 1, "Knife", 1),
        seed_give("seed coins for fares", "coins", 5000, "Coins", 5000),
        // Lean tele kit: one Varrock-shaped hop (nav-tele-smoke), not
        // RUNE_SEEDS 80/200/80/80/80. Jewellery is one charged copy each.
        seed_give("seed law runes", "lawrune", 10, "Law rune", 1),
        seed_give("seed air runes", "airrune", 30, "Air rune", 1),
        seed_give("seed fire runes", "firerune", 10, "Fire rune", 1),
        seed_give(
            "seed dueling ring",
            "ring_of_dueling_8",
            1,
            "Ring of dueling(8)",
            1,
        ),
        seed_give(
            "seed glory",
            "amulet_of_glory_4",
            1,
            "Amulet of glory(4)",
            1,
        ),
        seed_give(
            "seed games necklace",
            "necklace_of_minigames_8",
            1,
            "Games necklace(8)",
            1,
        ),
    ]
}

/// One `give <debug> <qty>`, then wait until that display name is in the
/// pack (rs2b0t `seedItem`: give once, poll presence — not re-give every
/// tick).
fn seed_give(
    step: &'static str,
    debug: &'static str,
    qty: i32,
    display: &'static str,
    want: i32,
) -> Step {
    Step {
        name: step,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("give {debug} {qty}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Item {
                name: display,
                count: want,
            },
            budget_ticks: 80,
        },
    }
}

fn tele_step(note: &'static str, tile: WorldTile) -> Step {
    Step {
        name: note,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| cheat(c, &tele_args(tile.level, tile.x, tile.z))),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            },
            budget_ticks: 120,
        },
    }
}

fn follow_step(note: &'static str, dest: WorldTile) -> Step {
    let arm = if glider_landing(dest) {
        // `map_findsquare($dest, 0, 1, lineofwalk)` — live Kar-Hewo
        // landed (3285,3211) for packed (3284,3211).
        Proof::ArrivedNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius: 1,
        }
    } else {
        Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        }
    };
    Step {
        name: note,
        kind: StepKind::Follow { dest },
        wait: Wait {
            arm,
            budget_ticks: 600,
        },
    }
}

/// Packed Gnome Air pads + hub (`gnome_glider.rs2` / `glider.constant`).
fn glider_landing(tile: WorldTile) -> bool {
    matches!(
        (tile.x, tile.z, tile.level),
        (2465, 3501, 3) | (2971, 2969, 0) | (2850, 3497, 0) | (3320, 3430, 0) | (3284, 3211, 0)
    )
}

/// 377 `fail()`: print and exit 1. The headed runner calls this on a
/// `Failed` status; the headless twin maps the same status through its own
/// `common::fail` (same exit-1 contract).
pub fn fail(msg: &str) -> ! {
    eprintln!("FAIL: {msg}");
    std::process::exit(1);
}

/// Nav pack path: `$NAV_PACK`, else `~/.274bot/274bot.navpack` (same rule
/// as host-play and the panel picker; the scenario crate must not depend
/// on host-play).
pub fn default_pack_path() -> PathBuf {
    match std::env::var("NAV_PACK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => PathBuf::from(".274bot/274bot.navpack"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_smoke_registered_as_a_one_shot_scene2_capture() {
        let s = get("render_smoke").expect("render_smoke is registered");
        assert_eq!(s.name, "render_smoke");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(!s.seed.mainland, "no hop needed for a smoke capture");
        assert_eq!(s.steps.len(), 1, "one capture step");
        assert!(matches!(
            s.steps[0].kind,
            StepKind::Shot { label: "scene2" }
        ));
        assert_eq!(s.proof.name(), "stat(16)>=0");
    }

    #[test]
    fn nav_full_is_a_mainland_follow_to_a_cross_square_destination() {
        let s = get("nav_full").expect("nav_full is registered");
        assert_eq!(s.name, "nav_full");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland, "the mainland hop lands the Lumbridge tele");
        assert_eq!(s.steps.len(), 1, "one follow step");
        let (dest, arm) = match &s.steps[0].kind {
            StepKind::Follow { dest } => (
                *dest,
                match &s.steps[0].wait.arm {
                    Proof::Arrived { x, z, level } => (*x, *z, *level),
                    other => panic!("follow arm must be arrived, got {other:?}"),
                },
            ),
            _ => panic!("nav_full step must be Follow"),
        };
        // The destination is a concrete pack tile ~44 tiles north of the
        // mainland landing, crossing the z=3264 mapsquare boundary into
        // (50,51) — a square the old 2-square pack never baked.
        assert_eq!(
            dest,
            WorldTile {
                x: 3220,
                z: 3264,
                level: 0
            }
        );
        assert_eq!(arm, (3220, 3264, 0));
        assert_eq!(s.proof.name(), "arrived(3220,3264,0)");
        assert_eq!(
            names(),
            [
                "walk",
                "render_smoke",
                "nav_full",
                "nav_door",
                "nav_cart",
                "nav_essence",
                "nav_elkoy",
                "nav_tele",
                "nav_shantay",
                "nav_routes",
                "nav_paint_path",
                "bone_burier",
            ]
        );
    }

    #[test]
    fn bone_burier_seeds_bones_and_buries_until_three_remain() {
        let s = get("bone_burier").expect("bone_burier is registered");
        assert_eq!(s.name, "bone_burier");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland, "unique live accounts spawn on tutorial");
        assert_eq!(s.steps.len(), 4, "skip, relog, seed, bury");
        // Step 2 relogs so the inv tab binds (bare setvar leaves the
        // side icons tutorial-locked).
        assert!(matches!(s.steps[1].kind, StepKind::Relog));
        assert_eq!(s.steps[1].wait.arm, Proof::SideTabAvailable { index: 3 });
        // Step 3 cheats five Bones into the pack.
        assert!(matches!(s.steps[2].kind, StepKind::Perform { .. }));
        assert_eq!(
            s.steps[2].wait.arm,
            Proof::Item {
                name: "Bones",
                count: 5
            }
        );
        // Step 4 buries the held Bones until at most three remain (the
        // server's own count drop proves each burial, not a chat read).
        assert!(matches!(s.steps[3].kind, StepKind::Repeat { .. }));
        assert_eq!(
            s.steps[3].wait.arm,
            Proof::ItemAtMost {
                name: "Bones",
                count: 3
            }
        );
        // The terminal proof is the server's bury message: a burial
        // actually happened.
        assert_eq!(s.proof.name(), "chat(contains \"bury the bones\")");
        assert!(s.companions.is_empty());
        assert!(s.settings.require_mainland_base);
    }

    #[test]
    fn nearest_door_loc_returns_the_offset_open_leaf_not_the_packed_at() {
        // Live Catherby: packed closed 1530 is (2816,3438); the open leaf
        // 1531 sits a tile off that `at`. The closer used to find the id
        // in radius 3 then still slam packed `at`, so interact_with_loc
        // looked up a typecode on an empty tile.
        let packed = WorldTile {
            x: 2816,
            z: 3438,
            level: 0,
        };
        let found = nearest_door_loc(packed, |x, z| (x == 2816 && z == 3439).then_some(OPEN_ID));
        assert_eq!(
            found,
            Some((
                WorldTile {
                    x: 2816,
                    z: 3439,
                    level: 0
                },
                OPEN_ID
            )),
            "slam target is the live leaf tile, not packed at"
        );
    }

    #[test]
    fn nearest_door_loc_keeps_the_packed_closed_leaf() {
        let packed = WorldTile {
            x: 2816,
            z: 3438,
            level: 0,
        };
        let found = nearest_door_loc(packed, |x, z| {
            (x == packed.x && z == packed.z).then_some(CLOSED_ID)
        });
        assert_eq!(found, Some((packed, CLOSED_ID)));
    }

    #[test]
    fn nav_door_is_a_two_profile_fleet_with_a_door_closer_companion() {
        let s = get("nav_door").expect("nav_door is registered");
        assert_eq!(s.name, "nav_door");
        assert_eq!(s.seed.profiles, [("test", "test"), ("test2", "test2")]);
        assert!(
            s.seed.mainland,
            "the hop lands the walker before the Catherby tele"
        );
        assert_eq!(s.steps.len(), 2, "tele the walker, then follow the route");
        assert!(
            matches!(s.steps[0].kind, StepKind::Perform { .. }),
            "step 1 is the Catherby cheat-tele"
        );
        let (dest, arm) = match &s.steps[1].kind {
            StepKind::Follow { dest } => (
                *dest,
                match &s.steps[1].wait.arm {
                    Proof::Arrived { x, z, level } => (*x, *z, *level),
                    other => panic!("follow arm must be arrived, got {other:?}"),
                },
            ),
            _ => panic!("nav_door step 2 must be Follow"),
        };
        assert_eq!(
            dest,
            WorldTile {
                x: 2817,
                z: 3443,
                level: 0
            }
        );
        assert_eq!(arm, (2817, 3443, 0));
        assert_eq!(s.proof.name(), "arrived(2817,3443,0)");
        assert_eq!(s.companions.len(), 1, "the closer is the one companion");
        assert_eq!(
            s.companions[0].profile, 1,
            "profile 1 (test2) is the closer"
        );
        assert!(names().contains(&"nav_door"));
    }

    #[test]
    fn nav_cart_is_a_mainland_follow_that_needs_the_cart_hop() {
        let s = get("nav_cart").expect("nav_cart is registered");
        assert_eq!(s.name, "nav_cart");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(
            s.seed.mainland,
            "the hop lands the walker before the Shilo tele"
        );
        assert_eq!(s.steps.len(), 2, "give the fare + tele, then follow");
        let (tele, arm) = match &s.steps[0].kind {
            StepKind::Perform { .. } => (
                "perform",
                match &s.steps[0].wait.arm {
                    Proof::Arrived { x, z, level } => (*x, *z, *level),
                    other => panic!("tele arm must be arrived, got {other:?}"),
                },
            ),
            _ => panic!("nav_cart step 1 must be Perform"),
        };
        assert_eq!(tele, "perform");
        assert_eq!(arm, (2834, 2954, 0), "the tele targets the Shilo driver");
        let (dest, arm) = match &s.steps[1].kind {
            StepKind::Follow { dest } => (
                *dest,
                match &s.steps[1].wait.arm {
                    Proof::Arrived { x, z, level } => (*x, *z, *level),
                    other => panic!("follow arm must be arrived, got {other:?}"),
                },
            ),
            _ => panic!("nav_cart step 2 must be Follow"),
        };
        assert_eq!(
            dest,
            WorldTile {
                x: 2776,
                z: 3214,
                level: 0
            },
            "the destination is the Brimhaven cart landing"
        );
        assert_eq!(arm, (2776, 3214, 0));
        assert_eq!(s.proof.name(), "arrived(2776,3214,0)");
        assert!(s.companions.is_empty());
        assert!(names().contains(&"nav_cart"));
    }

    #[test]
    fn nav_tele_gives_the_ring_and_follows_the_packed_rub_with_teleports_on() {
        let s = get("nav_tele").expect("nav_tele is registered");
        assert_eq!(s.name, "nav_tele");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland);
        assert_eq!(
            s.steps.len(),
            2,
            "give the ring + follow with allow_teleports"
        );
        // Step 1 clears the backpack and cheats the charged ring (the
        // packed rub edge's item_req); the arm waits for the ring to land
        // in the inventory.
        match &s.steps[0].kind {
            StepKind::Perform { .. } => {}
            _ => panic!("nav_tele step 1 must be Perform"),
        }
        assert!(matches!(
            s.steps[0].wait.arm,
            Proof::Item {
                name: "Ring of dueling(8)",
                count: 1
            }
        ));
        // Step 2 is the teleport-layer follow: only a `FollowTele` step
        // arms `allow_teleports`, so the destination must be the packed
        // dueling-ring landing and the arm its scatter radius.
        let (dest, arm) = match &s.steps[1].kind {
            StepKind::FollowTele { dest } => (
                *dest,
                match &s.steps[1].wait.arm {
                    Proof::ArrivedNear {
                        x,
                        z,
                        level,
                        radius,
                    } => (*x, *z, *level, *radius),
                    other => panic!("follow-tele arm must be arrivedNear, got {other:?}"),
                },
            ),
            _ => panic!("nav_tele step 2 must be FollowTele"),
        };
        assert_eq!(
            dest,
            WorldTile {
                x: 3315,
                z: 3235,
                level: 0
            },
            "the destination is the Al Kharid Duel Arena"
        );
        assert_eq!(arm, (3315, 3235, 0, 2));
        assert_eq!(s.proof.name(), "arrived_near(3315,3235,0,2)");
        assert!(s.companions.is_empty());
        assert!(names().contains(&"nav_tele"));
    }

    #[test]
    fn nav_essence_follows_in_and_back_out_to_aubury() {
        let s = get("nav_essence").expect("nav_essence is registered");
        assert_eq!(s.name, "nav_essence");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland);
        assert_eq!(
            s.steps.len(),
            5,
            "quest + dialog janitor + tele + entry follow + exit follow"
        );
        // Step 1 sends `~completequests`; the wait is the first
        // `p_choice` dialog it opens (never a dummy stat wait).
        assert!(matches!(s.steps[0].kind, StepKind::Perform { .. }));
        assert!(matches!(s.steps[0].wait.arm, Proof::ChatChoice));
        // Step 2 is the choice-answering janitor, waiting for the quest
        // journal to paint Rune Mysteries green.
        match &s.steps[1].kind {
            StepKind::DrainDialogs { choice } => assert_eq!(*choice, 1),
            _ => panic!("nav_essence step 2 must be DrainDialogs"),
        }
        assert!(matches!(
            s.steps[1].wait.arm,
            Proof::QuestDone {
                name: "Rune Mysteries Quest"
            }
        ));
        // Step 4 arms the entry follow to the mine pad and waits for any
        // mine landing (the landing is randomised, never the pad).
        let (dest, arm) = match &s.steps[3].kind {
            StepKind::Follow { dest } => (
                *dest,
                match &s.steps[3].wait.arm {
                    Proof::EssenceMine => "in_essence_mine",
                    other => panic!("entry arm must be EssenceMine, got {other:?}"),
                },
            ),
            _ => panic!("nav_essence step 4 must be Follow"),
        };
        assert_eq!(
            dest,
            WorldTile {
                x: 2912,
                z: 4833,
                level: 0
            },
            "the entry follow targets the mine pad"
        );
        assert_eq!(arm, "in_essence_mine");
        // Step 5 follows out through the exit portal to Aubury's anchor
        // (within the portal's randomised landing radius of 2).
        let (dest, arm) = match &s.steps[4].kind {
            StepKind::Follow { dest } => (
                *dest,
                match &s.steps[4].wait.arm {
                    Proof::ArrivedNear {
                        x,
                        z,
                        level,
                        radius,
                    } => (*x, *z, *level, *radius),
                    other => panic!("exit arm must be ArrivedNear, got {other:?}"),
                },
            ),
            _ => panic!("nav_essence step 5 must be Follow"),
        };
        assert_eq!(
            dest,
            WorldTile {
                x: 3253,
                z: 3401,
                level: 0
            },
            "the exit follow targets Aubury's anchor"
        );
        assert_eq!(arm, (3253, 3401, 0, 2));
        assert_eq!(s.proof.name(), "arrived_near(3253,3401,0,2)");
        assert!(s.companions.is_empty());
        assert!(names().contains(&"nav_essence"));
    }

    #[test]
    fn nav_elkoy_follows_into_the_village_through_the_maze_escort() {
        let s = get("nav_elkoy").expect("nav_elkoy is registered");
        assert_eq!(s.name, "nav_elkoy");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland);
        assert_eq!(
            s.steps.len(),
            4,
            "quest-seed + dialog janitor + tele + escort follow"
        );
        // Step 1 sends `~completequests`; the wait is the first `p_choice`
        // dialog it opens (never a dummy stat wait).
        assert!(matches!(s.steps[0].kind, StepKind::Perform { .. }));
        assert!(matches!(s.steps[0].wait.arm, Proof::ChatChoice));
        // Step 2 is the choice-answering janitor, waiting for the journal
        // to paint Tree Gnome Village green.
        match &s.steps[1].kind {
            StepKind::DrainDialogs { choice } => assert_eq!(*choice, 1),
            _ => panic!("nav_elkoy step 2 must be DrainDialogs"),
        }
        assert!(matches!(
            s.steps[1].wait.arm,
            Proof::QuestDone {
                name: "Tree Gnome Village"
            }
        ));
        // Step 3 cheat-teles onto the maze-side Elkoy's tile (the packed
        // escort edge's `at`, one tile south of the entrance coord).
        match &s.steps[2].wait.arm {
            Proof::Arrived { x, z, level } => assert_eq!((*x, *z, *level), (2504, 3191, 0)),
            other => panic!("tele arm must be arrived, got {other:?}"),
        }
        // Step 4 follows into the village: the packed escort edge's `to`,
        // the hop the hedge maze forces.
        let (dest, arm) = match &s.steps[3].kind {
            StepKind::Follow { dest } => (
                *dest,
                match &s.steps[3].wait.arm {
                    Proof::Arrived { x, z, level } => (*x, *z, *level),
                    other => panic!("follow arm must be arrived, got {other:?}"),
                },
            ),
            _ => panic!("nav_elkoy step 4 must be Follow"),
        };
        assert_eq!(
            dest,
            WorldTile {
                x: 2515,
                z: 3159,
                level: 0
            },
            "the destination is the packed maze coord"
        );
        assert_eq!(arm, (2515, 3159, 0));
        assert_eq!(s.proof.name(), "arrived(2515,3159,0)");
        assert!(s.companions.is_empty());
        assert!(names().contains(&"nav_elkoy"));
    }

    #[test]
    fn every_nav_scenario_uses_the_paint_preset() {
        for name in names() {
            if !name.starts_with("nav_") {
                continue;
            }
            let s = get(name).expect(name);
            let n = &s.settings.nav;
            assert!(
                n.show_nav_path
                    && n.collision_fill
                    && n.hop_labels
                    && n.client_trail
                    && n.camera_follow,
                "{name} missing a nav-test paint layer"
            );
            assert!(
                !n.nsew_labels && !n.component_flood,
                "{name} must not force NSEW / flood"
            );
            if name == "nav_door" {
                assert!(n.engine_speed_ms.is_none(), "door-troll stays 600ms ticks");
            } else {
                assert_eq!(n.engine_speed_ms, Some(300), "{name} halves the tickrate");
            }
        }
    }

    #[test]
    fn scenario_settings_default_matches_the_bag() {
        let d = ScenarioSettings::default();
        assert!(d.renderer);
        assert!(d.only_render_selected);
        assert!(!d.capture);
        assert!(!d.full_rate);
        assert_eq!(
            d.nav,
            ScenarioNav::default(),
            "paint layers are opt-in per scenario"
        );
        assert_eq!(d.deadline, DEFAULT_DEADLINE);
        assert_eq!(d.terminal_shot, None);
        assert!(
            !d.require_mainland_base,
            "gate is opt-in for brand-new tutorial accounts"
        );
        assert!(d.sustains.is_empty());
    }

    #[test]
    fn nav_door_settings_are_full_rate_without_capture_or_sidecar() {
        let s = get("nav_door").expect("nav_door");
        assert!(s.settings.full_rate);
        assert!(!s.settings.only_render_selected);
        assert!(!s.settings.capture);
        assert!(s.settings.renderer);
        assert_eq!(s.settings.deadline, DEFAULT_DEADLINE);
        assert!(!s.settings.require_mainland_base);
    }

    #[test]
    fn nav_door_settings_use_the_paint_preset_without_halved_ticks() {
        let s = nav_door_scenario();
        assert_eq!(s.settings.nav, nav_test_paints());
        assert!(s.settings.nav.engine_speed_ms.is_none());
        assert!(s.settings.full_rate);
    }

    #[test]
    fn nav_paint_path_is_a_short_courtyard_walk_with_live_paint_layers() {
        let s = get("nav_paint_path").expect("nav_paint_path is registered");
        assert_eq!(s.name, "nav_paint_path");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland, "the hop lands the courtyard walk");
        assert!(s.settings.require_mainland_base);
        assert_eq!(s.steps.len(), 1, "one short WalkTo step");
        let (dest, arm) = match &s.steps[0].kind {
            StepKind::Walk { dest } => (
                *dest,
                match &s.steps[0].wait.arm {
                    Proof::Arrived { x, z, level } => (*x, *z, *level),
                    other => panic!("walk arm must be arrived, got {other:?}"),
                },
            ),
            _ => panic!("nav_paint_path step must be Walk"),
        };
        // The courtyard walk spans ~8 tiles from the mainland landing
        // (3220,3220) or (3220,3222) down to (3220,3212).
        assert_eq!(
            dest,
            WorldTile {
                x: 3220,
                z: 3212,
                level: 0
            }
        );
        assert_eq!(arm, (3220, 3212, 0));
        assert_eq!(s.proof.name(), "arrived(3220,3212,0)");
        assert!(s.companions.is_empty(), "no closer for the paint path");
        assert!(s.settings.full_rate);
        assert_eq!(
            s.settings.nav,
            nav_test_paints().with_tick_ms(300),
            "nav tests use the paint preset and speed 300"
        );
        assert!(
            names().contains(&"nav_paint_path"),
            "registered for --live script_nav_paint_path"
        );
    }

    #[test]
    fn nav_full_settings_carry_deadline_and_terminal_shot() {
        let s = get("nav_full").expect("nav_full");
        assert_eq!(s.settings.deadline, Duration::from_secs(360));
        assert_eq!(s.settings.terminal_shot, Some("nav_full terminal"));
        assert!(!s.settings.full_rate);
    }

    #[test]
    fn render_smoke_settings_are_300s_and_gate_off() {
        let s = get("render_smoke").expect("render_smoke");
        assert_eq!(s.settings.deadline, Duration::from_secs(300));
        assert!(!s.settings.require_mainland_base);
    }

    #[test]
    fn walk_settings_are_defaults() {
        let s = get("walk").expect("walk");
        assert_eq!(s.settings, ScenarioSettings::default());
    }

    #[test]
    fn nav_routes_is_ten_borrowed_ods_with_item_seed() {
        let s = get("nav_routes").expect("nav_routes is registered");
        assert_eq!(s.name, "nav_routes");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(s.seed.mainland, "unique live accounts spawn on tutorial");
        assert!(s.settings.require_mainland_base);
        assert_eq!(NAV_ROUTES.len(), 10);
        // tutorial + quest setvars + relog + journal + maxme + knife/coins/lean tele kit,
        // then tele+follow per OD.
        assert_eq!(s.steps.len(), 13 + NAV_ROUTES.len() * 2);
        assert!(matches!(s.steps[0].kind, StepKind::Repeat { .. }));
        assert!(matches!(s.steps[2].kind, StepKind::Relog));
        assert!(matches!(
            s.steps[2].wait.arm,
            Proof::SideTabAvailable { index: 3 }
        ));
        assert!(matches!(
            s.steps[4].wait.arm,
            Proof::Stat { id: 0, min: 99 }
        ));
        assert!(matches!(
            s.steps[5].wait.arm,
            Proof::Item {
                name: "Knife",
                count: 1
            }
        ));
        assert!(matches!(
            s.steps[6].wait.arm,
            Proof::Item {
                name: "Coins",
                count: 5000
            }
        ));
        assert!(matches!(
            s.steps[12].wait.arm,
            Proof::Item {
                name: "Games necklace(8)",
                count: 1
            }
        ));
        let kar_hewo = s
            .steps
            .iter()
            .find(|st| {
                matches!(
                    st.kind,
                    StepKind::Follow {
                        dest: WorldTile {
                            x: 3284,
                            z: 3211,
                            level: 0
                        }
                    }
                )
            })
            .expect("Kar-Hewo follow");
        assert!(
            matches!(kar_hewo.wait.arm, Proof::ArrivedNear { radius: 1, .. }),
            "glider map_findsquare radius 1, not exact pad"
        );
        let last = NAV_ROUTES[9].2;
        let dest = match &s.steps[s.steps.len() - 1].kind {
            StepKind::Follow { dest } => *dest,
            _ => panic!("last step must be Follow"),
        };
        assert_eq!(dest, last);
        assert_eq!(s.proof.name(), "arrived(2817,3443,0)");
        assert!(s.settings.full_rate);
        assert_eq!(
            s.settings.nav,
            nav_test_paints().with_tick_ms(300),
            "seam run uses the paint preset and speed 300"
        );
        assert_eq!(s.settings.deadline, Duration::from_secs(3600));
        assert_eq!(s.settings.sustains, nav_energy_sustains());
        assert!(names().contains(&"nav_routes"));
    }
}
