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
    cheat, close_modal, op_loc, tele_args, Driver, Interactions, SendResult, MAXME_SETSTATS,
};
use api::snapshot::{GameSnapshot, ReadContext, WorldTile};
use client::client::Client;
use serde_json::{Map, Value};

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

/// rs2b0t-style `BUDGET_S` (seconds). Unset/empty/0 → `None` (use the
/// scenario's own deadline). When `Some`, live harnesses also keep the
/// window open after PASS until that budget elapses.
pub fn budget_s_from_env() -> Option<Duration> {
    budget_s_from(std::env::var("BUDGET_S").ok().as_deref())
}

pub fn budget_s_from(raw: Option<&str>) -> Option<Duration> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    let n: u64 = s.parse().ok()?;
    (n > 0).then_some(Duration::from_secs(n))
}

/// Boot defaults for a scenario. View knobs are headed-only; deadline /
/// terminal shot / mainland-base gate are consumed by `ScenarioRunner`.
/// `nav` is the session-only nav overlay (paints, camera, tickrate, find
/// flags) — applied for `--live` without writing panel prefs.
#[derive(Debug, Clone, PartialEq)]
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
    /// A JS/TS card (a `$RS2B0T` catalog name) the host **selects** when
    /// the live runner is installed, then Starts on [`StepKind::StartScript`]
    /// after the last seed wait — the scenario's later steps observe the
    /// running script's evidence. The host fills the catalog from `$RS2B0T`
    /// (register/Load) and dispatches `script_start_load` on that step.
    /// `None` for host-driven scenarios.
    pub start_script: Option<&'static str>,
    /// Scenario-only parameter overrides merged last at script Start (never
    /// written to operator `script-settings.json`).
    pub script_settings_inject: Option<&'static [ScriptSettingInject]>,
    /// When set (e.g. `"partner"`), live_prepare inserts the minted
    /// companion username (`names[1]`) into the script settings bag under
    /// this key after [`script_settings_inject`].
    pub inject_companion_as: Option<&'static str>,
}

/// One injected script setting for live gold scenarios.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScriptSettingInject {
    pub id: &'static str,
    pub value: ScriptInjectValue,
}

/// Typed values for [`ScenarioSettings::script_settings_inject`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScriptInjectValue {
    Str(&'static str),
    Num(f64),
    Bool(bool),
    StrList(&'static [&'static str]),
}

/// Build the JSON bag panel/TUI merge last when starting a live script card.
pub fn settings_inject_map(
    inject: Option<&'static [ScriptSettingInject]>,
) -> Option<Map<String, Value>> {
    let rows = inject?;
    let mut map = Map::new();
    for row in rows {
        let value = match row.value {
            ScriptInjectValue::Str(v) => Value::String(v.to_string()),
            ScriptInjectValue::Num(v) => {
                Value::Number(serde_json::Number::from_f64(v).unwrap_or_else(|| 0.into()))
            }
            ScriptInjectValue::Bool(v) => Value::Bool(v),
            ScriptInjectValue::StrList(v) => {
                Value::Array(v.iter().map(|s| Value::String((*s).to_string())).collect())
            }
        };
        map.insert(row.id.to_string(), value);
    }
    Some(map)
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
            start_script: None,
            script_settings_inject: None,
            inject_companion_as: None,
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
    /// `wait.arm` holds — continue a `BUTTON_CONTINUE` chat IF first
    /// (`advancestat` level-up), else answer the chat modal's `choice`-th
    /// button (1-based) when a `p_choiceN` dialog is up (the
    /// `~completequests` debugproc's Arrav-gang and Ikov-side prompts),
    /// else close the modal when one is up (the quest-completion quest
    /// scrolls, whose open main modal stalls the engine's script queue).
    /// A no-op send is harmless, so the re-send drains each dialog as it
    /// appears.
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
    /// Host starts the catalog isolate (`script_start_load`) once when
    /// the live pump sees this step. No-op on the client. The wait is an
    /// immediate / one-tick arm that does not require XP.
    StartScript,
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
        "chicken_killer" => Some(chicken_killer_scenario()),
        "chicken_killer_bank" => Some(chicken_killer_bank_scenario()),
        "thiever" => Some(thiever_scenario()),
        "alcher" => Some(alcher_scenario()),
        "alcher_custom" => Some(alcher_custom_scenario()),
        "alcher_custom_alias" => Some(alcher_custom_alias_scenario()),
        "alcher_custom_name" => Some(alcher_custom_name_scenario()),
        "alcher_ordered" => Some(alcher_ordered_scenario()),
        "alcher_large_batch" => Some(alcher_large_batch_scenario()),
        "bank_fletcher" => Some(bank_fletcher_scenario()),
        "bank_fletcher_string" => Some(bank_fletcher_string_scenario()),
        "bank_fletcher_cut_string" => Some(bank_fletcher_cut_string_scenario()),
        "dart_fletcher" => Some(dart_fletcher_scenario()),
        "dart_fletcher_iron" => Some(dart_fletcher_iron_scenario()),
        "herb_cleaner" => Some(herb_cleaner_scenario()),
        "herb_cleaner_named" => Some(herb_cleaner_named_scenario()),
        "gem_cutter" => Some(gem_cutter_scenario()),
        "gem_cutter_named" => Some(gem_cutter_named_scenario()),
        "door_opener" => Some(door_opener_scenario()),
        "door_opener_gate" => Some(door_opener_gate_scenario()),
        "gnome_course" => Some(gnome_course_scenario()),
        "gnome_course_radius" => Some(gnome_course_radius_scenario()),
        "flax_picker" => Some(flax_picker_scenario()),
        "superheater" => Some(superheater_scenario()),
        "superheater_steel" => Some(superheater_steel_scenario()),
        "superheater_fire_battlestaff" => Some(superheater_fire_battlestaff_scenario()),
        "vial_filler" => Some(vial_filler_scenario()),
        "vial_filler_east" => Some(vial_filler_east_scenario()),
        "potion_maker" => Some(potion_maker_scenario()),
        "potion_maker_named" => Some(potion_maker_named_scenario()),
        "tanner_bot" => Some(tanner_bot_scenario()),
        "tanner_bot_hard" => Some(tanner_bot_hard_scenario()),
        "script_trade" => Some(script_trade_scenario()),
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
        "chicken_killer",
        "chicken_killer_bank",
        "thiever",
        "alcher",
        "alcher_custom",
        "alcher_custom_alias",
        "alcher_custom_name",
        "alcher_ordered",
        "alcher_large_batch",
        "bank_fletcher",
        "bank_fletcher_string",
        "bank_fletcher_cut_string",
        "dart_fletcher",
        "dart_fletcher_iron",
        "herb_cleaner",
        "herb_cleaner_named",
        "gem_cutter",
        "gem_cutter_named",
        "door_opener",
        "door_opener_gate",
        "gnome_course",
        "gnome_course_radius",
        "flax_picker",
        "superheater",
        "superheater_steel",
        "superheater_fire_battlestaff",
        "vial_filler",
        "vial_filler_east",
        "potion_maker",
        "potion_maker_named",
        "tanner_bot",
        "tanner_bot_hard",
        "script_trade",
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

/// Run the real catalog BoneBurier through inventory depletion, off-scene
/// banking, withdrawal and another burial. All stock is prepared before Start;
/// the harness only observes after that boundary.
fn bone_burier_scenario() -> Scenario {
    let watch = |name, arm| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            // The off-scene trip may legitimately outlast the ordinary
            // 150-tick action observation. Observe the script's existing
            // 120-second travel timeout instead of ending the proof first.
            budget_ticks: if matches!(arm, Proof::BankItem { .. }) {
                240
            } else {
                SCRIPT_GOLD_WATCH_TICKS
            },
            arm,
        },
    };
    Scenario {
        name: "bone_burier",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "prepare five carried bones and twenty-eight banked bones",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give bones 5");
                        cheat(c, "givebank bones 28");
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
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            start_catalog_step(),
            watch(
                "watch the first five burials",
                Proof::StatXpGain { id: 5, min: 22 },
            ),
            watch(
                "watch the carried bones run out",
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch(
                "watch the script reach and open its stocked bank",
                Proof::BankItem {
                    name: "Bones",
                    count: 28,
                },
            ),
            watch(
                "watch the script withdraw a full pack",
                Proof::Item {
                    name: "Bones",
                    count: 28,
                },
            ),
            watch(
                "watch the bank stock decrease",
                Proof::BankItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch("watch the script close its bank", Proof::BankClosed),
            // XP baselines are retained for the scenario: six normal bones
            // yield 27 integer XP. Five seed bones alone can yield only 22.
            watch(
                "watch a burial from the withdrawn pack",
                Proof::StatXpGain { id: 5, min: 27 },
            ),
        ],
        proof: Proof::StatXpGain { id: 5, min: 27 },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BoneBurier"),
            terminal_shot: Some("bone_burier terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Shared live-script seed: stick `tutorial=1000`, relog so side tab 3 binds.
fn script_live_seed_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip",
            kind: StepKind::Perform {
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
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
    ]
}

/// Explicit `speed 600` so a leftover nav `speed 300` is not inherited.
fn gold_script_nav() -> ScenarioNav {
    ScenarioNav::default().with_tick_ms(600)
}

/// Watch window after Start (~90s at 600ms ticks). Seed/tele/relog stay longer.
const SCRIPT_GOLD_WATCH_TICKS: u32 = 150;
/// Seed + tele + drain + the short watch. Not a 6-minute soak.
const SCRIPT_GOLD_DEADLINE: Duration = Duration::from_secs(180);

/// Janitor after `advancestat`: click the level-up continue until the chat IF is gone.
fn drain_advancestat() -> Step {
    Step {
        name: "drain advancestat level-up dialogs",
        kind: StepKind::DrainDialogs { choice: 1 },
        wait: Wait {
            arm: Proof::ChatClosed,
            budget_ticks: 60,
        },
    }
}

/// Catalog Start after the last seed wait: live pumps call `script_start_load`
/// once, then this one-tick arm (run energy, not XP) succeeds.
fn start_catalog_step() -> Step {
    Step {
        name: "start the catalog card",
        kind: StepKind::StartScript,
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 1,
        },
    }
}

/// Lumbridge chicken pen (east of the castle). ChickenKiller anchors at
/// Start — host tele lands here after mainland/tutskip/relog, then Start.
const LUMBRIDGE_CHICKENS: WorldTile = WorldTile {
    x: 3235,
    z: 3295,
    level: 0,
};

/// The `chicken_killer` scenario: live ChickenKiller gold — script anchors
/// at Start (no camp nav). Seed = mainland + tutskip + relog + tele to
/// the pen `(3235,3295)`, then Start. Melee defaults, DeathRecovery idle.
/// Proof is strength XP ≥ 1. Remaining miss is combat, not walk.
fn chicken_killer_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 2, min: 1 };
    let tele = LUMBRIDGE_CHICKENS;
    Scenario {
        name: "chicken_killer",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "tele to Lumbridge chickens",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(tele.level, tele.x, tele.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: tele.x,
                        z: tele.z,
                        level: tele.level,
                        radius: 8,
                    },
                    budget_ticks: 120,
                },
            });
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script fight chickens",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ChickenKiller"),
            terminal_shot: Some("chicken_killer terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Falador south chicken pen, immediately south of the host cow-field pin
/// `(3029,3305,0)`. Default Lumbridge pen `(3235,3295,0)` cannot reach a
/// same-plane Use-quickly booth inside PeriodicBank's 60s walk: castle
/// booths are upstairs, and Draynor/Al Kharid/Varrock West are ≥128
/// Chebyshev. This interior is Chebyshev 61 from Falador East vs 64 from
/// Draynor, so nearest packed booth stays Falador East. ChickenKiller
/// `destination()` is null; return radius is the reviewed service 6.
const FALADOR_CHICKENS: WorldTile = WorldTile {
    x: 3029,
    z: 3294,
    level: 0,
};

const STRENGTH_STAT: i32 = 2;

const CHICKEN_KILLER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "lootMatch",
        value: ScriptInjectValue::Str("feather"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
];

/// ChickenKiller loot-count trip: melee, `bankEveryItems=1`, Feather 314.
/// Bones stay the bury keep-list default; melee `afterDeposit` is a no-op.
/// Prepare only before Start. Proof is combat, exact loot, fresh deposit,
/// return to the original anchor, then new exact feathers after the pack
/// was emptied. Same-id `StatXpGain { min: 1 }` cannot witness further
/// work: the runner keeps the first XP baseline per skill id.
fn chicken_killer_bank_scenario() -> Scenario {
    let tele = FALADOR_CHICKENS;
    let first_strength = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let further_feathers = Proof::ItemId {
        id: FEATHER_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare melee stats, empty pack and Falador chicken anchor before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "setstat attack 30");
                cheat(c, "setstat strength 30");
                cheat(c, "~clearinv");
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (name, id) in [
        ("acknowledge prepared Attack 30", 0),
        ("acknowledge prepared Strength 30", STRENGTH_STAT),
    ] {
        steps.push(bank_fletcher_watch(name, Proof::Stat { id, min: 30 }));
    }
    steps.push(bank_fletcher_watch(
        "confirm no seeded feathers in pack before Start",
        Proof::ItemIdAtMost {
            id: FEATHER_ID,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Strength XP from melee chicken combat",
            first_strength,
        ),
        (
            "watch exact Feather 314 looted after Start",
            Proof::ItemId {
                id: FEATHER_ID,
                count: 1,
            },
        ),
        (
            "watch script-looted feathers enter a fresh bank",
            Proof::BankItemId {
                id: FEATHER_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of feathers after deposit",
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: 0,
            },
        ),
        (
            "watch return to the Falador chicken anchor within radius 6",
            Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius: 6,
            },
        ),
        (
            "watch the periodic bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch new exact Feather 314 after return", further_feathers),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "chicken_killer_bank",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_feathers,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ChickenKiller"),
            script_settings_inject: Some(CHICKEN_KILLER_BANK_INJECT),
            terminal_shot: Some("chicken_killer_bank"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// East Ardougne market guard tile (rs2b0t Thiever live gold).
const ARDOUGNE_GUARD: WorldTile = WorldTile {
    x: 2661,
    z: 3306,
    level: 0,
};

const THIEVER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::Str(""),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
];

/// The `thiever` scenario: live Thiever gold — Guard pickpocket at the
/// Ardougne tile, food via `give`, loot off. Proof is thieving XP delta.
fn thiever_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 17, min: 1 };
    let tele = ARDOUGNE_GUARD;
    Scenario {
        name: "thiever",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed stats, food, and tele to the guard stand",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "advancestat thieving 50");
                        cheat(c, "advancestat hitpoints 50");
                        cheat(c, "give lobster 10");
                        cheat(c, &tele_args(tele.level, tele.x, tele.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: tele.x,
                        z: tele.z,
                        level: tele.level,
                        radius: 10,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(drain_advancestat());
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script pickpocket guards",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Thiever"),
            script_settings_inject: Some(THIEVER_INJECT),
            nav: gold_script_nav(),
            terminal_shot: Some("thiever paint"),
            ..Default::default()
        },
    }
}

/// Diagnostic prerequisite workload: bank stock is seeded once, then the
/// unchanged catalog script must eat, restock, and return using host APIs.
pub fn thiever_sustained_scenario() -> Scenario {
    let mut scenario = thiever_scenario();
    let step = scenario
        .steps
        .iter_mut()
        .find(|s| s.name == "seed stats, food, and tele to the guard stand")
        .expect("Thiever seed");
    step.kind = StepKind::Perform {
        send: Box::new(|c, _| {
            cheat(c, "advancestat thieving 50");
            cheat(c, "advancestat hitpoints 50");
            cheat(c, "give lobster 4");
            cheat(c, "givebank lobster 2000");
            cheat(
                c,
                &tele_args(ARDOUGNE_GUARD.level, ARDOUGNE_GUARD.x, ARDOUGNE_GUARD.z),
            );
            true
        }),
    };
    scenario
}

/// Varrock West bank stand (Alcher / BankFletcher gold).
const VARROCK_WEST_BANK: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};

const ALCHER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "items",
    value: ScriptInjectValue::StrList(&["rune_chainbody"]),
}];

const ALCHER_CUSTOM_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["custom"]),
    },
    ScriptSettingInject {
        id: "customItem",
        value: ScriptInjectValue::Str("rune_chainbody"),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_CUSTOM_ALIAS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["custom"]),
    },
    ScriptSettingInject {
        id: "customItem",
        value: ScriptInjectValue::Str("adamant_scimitar"),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_CUSTOM_NAME_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["custom"]),
    },
    ScriptSettingInject {
        id: "customItem",
        value: ScriptInjectValue::Str("Adamant scimitar"),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_ORDERED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_platebody", "rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_LARGE_BATCH_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1000.0),
    },
];

fn alcher_variant_scenario(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    fodder: &'static str,
    fodder_count: i32,
    runes: i32,
    required_stack: Option<(&'static str, i32)>,
) -> Scenario {
    let xp = Proof::StatXpGain { id: 6, min: 1 };
    let bank = VARROCK_WEST_BANK;
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed magic 55 and bounded Alcher stock before Start",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "setstat magic 55");
                        cheat(c, &format!("givebank {fodder} {fodder_count}"));
                        if name == "alcher_ordered" {
                            cheat(c, "givebank rune_chainbody 1");
                        }
                        cheat(c, &format!("givebank naturerune {runes}"));
                        cheat(c, "givebank staff_of_fire 1");
                        cheat(c, &tele_args(bank.level, bank.x, bank.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: bank.x,
                        z: bank.z,
                        level: bank.level,
                        radius: 6,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(Step {
                name: "confirm Magic 55 before Start",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 6, min: 55 },
                    budget_ticks: 200,
                },
            });
            steps.push(start_catalog_step());
            if let Some((item, count)) = required_stack {
                steps.push(Step {
                    name: "watch selected noted stack",
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: Wait {
                        arm: Proof::Item { name: item, count },
                        budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                    },
                });
            }
            steps.push(Step {
                name: "watch the Alcher option variant cast",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn alcher_custom_scenario() -> Scenario {
    alcher_variant_scenario(
        "alcher_custom",
        ALCHER_CUSTOM_INJECT,
        "rune_chainbody",
        2,
        4,
        Some(("Rune chainbody", 1)),
    )
}

fn alcher_ordered_scenario() -> Scenario {
    alcher_variant_scenario(
        "alcher_ordered",
        ALCHER_ORDERED_INJECT,
        "rune_platebody",
        1,
        4,
        Some(("Rune platebody", 1)),
    )
}

fn alcher_large_batch_scenario() -> Scenario {
    alcher_variant_scenario(
        "alcher_large_batch",
        ALCHER_LARGE_BATCH_INJECT,
        "rune_chainbody",
        1000,
        1000,
        Some(("Rune chainbody", 1000)),
    )
}

const ADAMANT_SCIMITAR_ID: i32 = 1331;
const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
const NATURE_RUNE_ID: i32 = 561;
const COINS_ID: i32 = 995;
const STAFF_OF_FIRE_ID: i32 = 1387;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
const HIGH_ALCH_MAGIC_XP: i32 = 65;

fn alcher_custom_alias_scenario() -> Scenario {
    alcher_generated_custom_scenario("alcher_custom_alias", ALCHER_CUSTOM_ALIAS_INJECT)
}

fn alcher_custom_name_scenario() -> Scenario {
    alcher_generated_custom_scenario("alcher_custom_name", ALCHER_CUSTOM_NAME_INJECT)
}

/// Select the frozen Alcher Custom sentinel for a generated item that was
/// never in the handwritten ITEM_DB. The target is banked unnoted and must
/// be withdrawn as certificate id 1332 before the cast.
fn alcher_generated_custom_scenario(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: 6,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 55 and one banked generated custom target before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 55");
                cheat(c, "givebank adamant_scimitar 1");
                cheat(c, "givebank naturerune 1");
                cheat(c, "givebank staff_of_fire 1");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Magic 55 before Start",
            Proof::Stat { id: 6, min: 55 },
        ),
        (
            "confirm no seeded unnoted custom target before Start",
            Proof::ItemIdAtMost {
                id: ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted custom target before Start",
            Proof::ItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Nature rune outcome before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unnoted custom seed bank",
        Proof::BankItemId {
            id: ADAMANT_SCIMITAR_ID,
            count: 1,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded noted custom target in bank",
            Proof::BankItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch the generated custom item land as the noted id",
            Proof::ItemId {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 1,
            },
        ),
        ("watch Magic XP from one High Level Alchemy cast", xp),
        (
            "watch the noted custom target consumed",
            Proof::ItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
        (
            "watch the Nature rune consumed",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "watch the exact High Alchemy coin increase",
            Proof::ItemId {
                id: COINS_ID,
                count: ADAMANT_SCIMITAR_ALCH_COINS,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// The `alcher` scenario: live Alcher gold — Varrock West, noted fodder +
/// natures + fire staff in bank, magic 55+. Proof is magic XP or coin gain
/// from alchs (magic XP delta here).
fn alcher_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 6, min: 1 };
    let bank = VARROCK_WEST_BANK;
    Scenario {
        name: "alcher",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed magic 55 before Start",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setstat magic 55");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 6, min: 55 },
                    budget_ticks: 200,
                },
            });
            steps.push(Step {
                name: "seed bank stock and tele to Varrock West",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "givebank rune_chainbody 30");
                        cheat(c, "givebank naturerune 200");
                        cheat(c, "givebank staff_of_fire 1");
                        cheat(c, &tele_args(bank.level, bank.x, bank.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: bank.x,
                        z: bank.z,
                        level: bank.level,
                        radius: 6,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script alch noted stock",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_INJECT),
            terminal_shot: Some("alcher terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const BANK_FLETCHER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Willow logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Short bow"),
    },
];

const BANK_FLETCHER_STRING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Willow logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("String short bow"),
    },
];

const BANK_FLETCHER_CUT_STRING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("cut+string"),
    },
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Willow logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Short bow"),
    },
];

const WILLOW_LOGS_ID: i32 = 1519;
const UNSTRUNG_WILLOW_SHORTBOW_ID: i32 = 60;
const STRUNG_WILLOW_SHORTBOW_ID: i32 = 849;
const BOW_STRING_ID: i32 = 1777;
const VARROCK_WEST_BANK_BOOTH_ID: i32 = 2213;

fn bank_fletcher_watch(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn bank_fletcher_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                // Use (OP_LOC1) starts banker dialogue. Resolve Use-quickly
                // from the selected booth's published actions instead.
                let booth = WorldTile {
                    x: VARROCK_WEST_BANK.x + 1,
                    z: VARROCK_WEST_BANK.z,
                    level: VARROCK_WEST_BANK.level,
                };
                matches!(
                    Interactions::new(snapshot, c).open_booth_at(booth, VARROCK_WEST_BANK_BOOTH_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn bank_fletcher_close_seed_bank() -> Step {
    Step {
        name: "close the acknowledged seed bank before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| close_modal(c)),
        },
        wait: Wait {
            arm: Proof::BankClosed,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

/// Run BankFletcher through a full pack, product deposit, log withdrawal and
/// another product. All bank stock is prepared before the script starts.
fn bank_fletcher_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 9, min: 1 };
    // Selected content awards 33.3 XP per willow shortbow: the 27 carried logs
    // alone can produce at most 899 integer XP. This requires a banked log.
    let second_batch_xp = Proof::StatXpGain { id: 9, min: 900 };
    let bank = VARROCK_WEST_BANK;
    Scenario {
        name: "bank_fletcher",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed knife, logs, and tele to Varrock West",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "advancestat fletching 35");
                        cheat(c, "give knife 1");
                        cheat(c, "give willow_logs 27");
                        cheat(c, "givebank willow_logs 54");
                        cheat(c, &tele_args(bank.level, bank.x, bank.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Item {
                        name: "Knife",
                        count: 1,
                    },
                    budget_ticks: 120,
                },
            });
            steps.push(drain_advancestat());
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script fletch willow logs",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            for (name, arm) in [
                (
                    "watch the script deposit its first pack of bows",
                    Proof::BankItem {
                        name: "Willow shortbow",
                        count: 27,
                    },
                ),
                (
                    "watch the script withdraw its next pack of logs",
                    Proof::Item {
                        name: "Willow logs",
                        count: 27,
                    },
                ),
                (
                    "watch the banked logs decrease",
                    Proof::BankItemAtMost {
                        name: "Willow logs",
                        count: 27,
                    },
                ),
                ("watch the script close its bank", Proof::BankClosed),
                ("watch the script fletch a withdrawn log", second_batch_xp),
            ] {
                steps.push(Step {
                    name,
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: Wait {
                        arm,
                        budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                    },
                });
            }
            steps
        },
        proof: second_batch_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(BANK_FLETCHER_INJECT),
            terminal_shot: Some("bank_fletcher terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// String two carried pairs, bank the exact id-849 products, withdraw a fresh
/// unstacked 14+14 load, and string beyond the seeded pair. The old catalog has
/// no `mode`, so product selection is the shared old/new stringing contract.
fn bank_fletcher_string_scenario() -> Scenario {
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed exact willow stringing pairs and bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "advancestat fletching 35");
                cheat(c, "give unstrung_willow_shortbow 2");
                cheat(c, "give bow_string 2");
                cheat(c, "givebank unstrung_willow_shortbow 28");
                cheat(c, "givebank bow_string 28");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm Fletching 35 before Start",
            Proof::Stat { id: 9, min: 35 },
        ),
        (
            "confirm two exact unstrung willow shortbows before Start",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "confirm two exact bow strings before Start",
            Proof::ItemId {
                id: BOW_STRING_ID,
                count: 2,
            },
        ),
        (
            "confirm no seeded strung willow shortbow before Start",
            Proof::ItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unstrung seed bank",
        Proof::BankItemId {
            id: UNSTRUNG_WILLOW_SHORTBOW_ID,
            count: 28,
        },
    ));
    for (name, arm) in [
        (
            "acknowledge the exact bow-string seed bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 28,
            },
        ),
        (
            "acknowledge no seeded strung bow in bank",
            Proof::BankItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch XP from both seeded stringing pairs",
            Proof::StatXpGain { id: 9, min: 66 },
        ),
        (
            "watch both seeded pairs become exact strung willow shortbows",
            Proof::ItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch both seeded unstrung bows consumed",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "watch both seeded bow strings consumed",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch the exact strung pair enter a fresh bank",
            Proof::BankItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch a fresh fourteen-bow withdrawal",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 14,
            },
        ),
        (
            "watch a fresh fourteen-string withdrawal",
            Proof::ItemId {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch exact unstrung bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 14,
            },
        ),
        (
            "watch exact bow-string bank stock decrease",
            Proof::BankItemIdAtMost {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch the script close its stringing bank",
            Proof::BankClosed,
        ),
        (
            "watch a newly withdrawn bow become exact id 849",
            Proof::ItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "watch stringing XP beyond the two seeded pairs",
            Proof::StatXpGain { id: 9, min: 67 },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "bank_fletcher_string",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::StatXpGain { id: 9, min: 67 },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(BANK_FLETCHER_STRING_INJECT),
            terminal_shot: Some("bank_fletcher_string terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// New-catalog combined mode: cut only the two seeded logs, observe their exact
/// id-60 products in bank, then withdraw those products with strings and finish
/// both as id 849. No unstrung or strung outcome is seeded.
fn bank_fletcher_cut_string_scenario() -> Scenario {
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed two willow logs and only banked strings before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "advancestat fletching 35");
                cheat(c, "give knife 1");
                cheat(c, "give willow_logs 2");
                cheat(c, "givebank bow_string 28");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm Fletching 35 before Start",
            Proof::Stat { id: 9, min: 35 },
        ),
        (
            "confirm the knife before Start",
            Proof::Item {
                name: "Knife",
                count: 1,
            },
        ),
        (
            "confirm two exact willow logs before Start",
            Proof::ItemId {
                id: WILLOW_LOGS_ID,
                count: 2,
            },
        ),
        (
            "confirm no seeded unstrung willow shortbow before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung willow shortbow before Start",
            Proof::ItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact bow-string seed bank",
        Proof::BankItemId {
            id: BOW_STRING_ID,
            count: 28,
        },
    ));
    for (name, arm) in [
        (
            "acknowledge no extra willow logs in bank",
            Proof::BankItemIdAtMost {
                id: WILLOW_LOGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded unstrung bow in bank",
            Proof::BankItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded strung bow in bank",
            Proof::BankItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch cut XP for both seeded logs",
            Proof::StatXpGain { id: 9, min: 66 },
        ),
        (
            "watch both willow logs become exact unstrung bows",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch both willow logs consumed in the cut phase",
            Proof::ItemIdAtMost {
                id: WILLOW_LOGS_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created unstrung pair enter a fresh bank",
            Proof::BankItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch the same unstrung pair leave that bank",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch the combined string load arrive",
            Proof::ItemId {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch exact unstrung bank stock empty",
            Proof::BankItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "watch exact bow-string bank stock decrease",
            Proof::BankItemIdAtMost {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch the combined script close its bank",
            Proof::BankClosed,
        ),
        (
            "watch XP advance from cutting into stringing",
            Proof::StatXpGain { id: 9, min: 100 },
        ),
        (
            "watch both script-created bows become exact id 849",
            Proof::ItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch both script-created id-60 bows consumed",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "bank_fletcher_cut_string",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: STRUNG_WILLOW_SHORTBOW_ID,
            count: 2,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(BANK_FLETCHER_CUT_STRING_INJECT),
            terminal_shot: Some("bank_fletcher_cut_string terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const BRONZE_DART_TIP_ID: i32 = 819;
const BRONZE_DART_ID: i32 = 806;
const IRON_DART_TIP_ID: i32 = 820;
const IRON_DART_ID: i32 = 807;
const FEATHER_ID: i32 = 314;
const UNIDENTIFIED_GUAM_ID: i32 = 199;
const GUAM_LEAF_ID: i32 = 249;
const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
const MARRENTILL_ID: i32 = 251;
const UNCUT_SAPPHIRE_ID: i32 = 1623;
const SAPPHIRE_ID: i32 = 1607;
const UNCUT_OPAL_ID: i32 = 1625;
const CHISEL_ID: i32 = 1755;
const CRUSHED_GEMSTONE_ID: i32 = 1633;
const FLETCHING_STAT: i32 = 9;
const CRAFTING_STAT: i32 = 12;
const HERBLORE_STAT: i32 = 15;
const LUMBRIDGE_COURTYARD: WorldTile = WorldTile {
    x: 3220,
    z: 3212,
    level: 0,
};

const DART_FLETCHER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "tier",
    value: ScriptInjectValue::Str("Bronze"),
}];

const DART_FLETCHER_IRON_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "tier",
    value: ScriptInjectValue::Str("Iron"),
}];

const HERB_CLEANER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&[]),
}];

const HERB_CLEANER_NAMED_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&["Guam leaf"]),
}];

const GEM_CUTTER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "gems",
    value: ScriptInjectValue::StrList(&[]),
}];

const GEM_CUTTER_NAMED_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "gems",
    value: ScriptInjectValue::StrList(&["Sapphire"]),
}];

fn dart_fletcher_scenario() -> Scenario {
    dart_fletcher_variant(
        "dart_fletcher",
        DART_FLETCHER_INJECT,
        1,
        "bronze_dart_tip",
        BRONZE_DART_TIP_ID,
        BRONZE_DART_ID,
        IRON_DART_ID,
    )
}

fn dart_fletcher_iron_scenario() -> Scenario {
    dart_fletcher_variant(
        "dart_fletcher_iron",
        DART_FLETCHER_IRON_INJECT,
        22,
        "iron_dart_tip",
        IRON_DART_TIP_ID,
        IRON_DART_ID,
        BRONZE_DART_ID,
    )
}

/// No bank. Spam Feather on the selected tip; one action is 10 darts.
fn dart_fletcher_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    level: i32,
    tip_alias: &'static str,
    tip_id: i32,
    product_id: i32,
    wrong_product_id: i32,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 2,
    };
    let courtyard = LUMBRIDGE_COURTYARD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Fletching and exact dart stacks before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat fletching {level}"));
                cheat(c, &format!("give {tip_alias} 100"));
                cheat(c, "give feather 100");
                cheat(c, &tele_args(courtyard.level, courtyard.x, courtyard.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: courtyard.x,
                z: courtyard.z,
                level: courtyard.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Fletching before Start",
            Proof::Stat {
                id: FLETCHING_STAT,
                min: level,
            },
        ),
        (
            "confirm exact dart tips before Start",
            Proof::ItemId {
                id: tip_id,
                count: 100,
            },
        ),
        (
            "confirm exact feathers before Start",
            Proof::ItemId {
                id: FEATHER_ID,
                count: 100,
            },
        ),
        (
            "confirm no seeded dart product before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong-tier dart product before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Fletching XP from dart fletching", first_xp),
        (
            "watch at least one ten-dart action by exact product id",
            Proof::ItemId {
                id: product_id,
                count: 10,
            },
        ),
        (
            "watch exact dart tips consumed",
            Proof::ItemIdAtMost {
                id: tip_id,
                count: 90,
            },
        ),
        (
            "watch exact feathers consumed",
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: 90,
            },
        ),
        (
            "watch further exact product progress",
            Proof::ItemId {
                id: product_id,
                count: 20,
            },
        ),
        (
            "watch no wrong-tier dart product",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
        ("watch Fletching XP beyond one dart action", further_xp),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("DartFletcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn herb_cleaner_scenario() -> Scenario {
    herb_cleaner_variant("herb_cleaner", HERB_CLEANER_INJECT, 3, false)
}

fn herb_cleaner_named_scenario() -> Scenario {
    herb_cleaner_variant("herb_cleaner_named", HERB_CLEANER_NAMED_INJECT, 5, true)
}

/// Empty pack, banked unidentified guam. Named also banks marrentill unids
/// that must stay put. Identify fills the pack, then deposit-all restocks.
fn herb_cleaner_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    level: i32,
    named: bool,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 2,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore and unidentified bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat herblore {level}"));
                cheat(c, "givebank unidentified_guam 30");
                if named {
                    cheat(c, "givebank unidentified_marentill 4");
                }
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Herblore before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: level,
            },
        ),
        (
            "confirm no seeded unidentified guam in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded clean guam before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unidentified marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded clean marrentill before Start",
            Proof::ItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unidentified guam seed bank",
        Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 30,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded clean guam in bank",
        Proof::BankItemIdAtMost {
            id: GUAM_LEAF_ID,
            count: 0,
        },
    ));
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact untouched marrentill seed bank",
            Proof::BankItemId {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 4,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge no seeded clean marrentill in bank",
            Proof::BankItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Herblore XP from identifying guam", first_xp),
        (
            "watch a full pack of exact clean guam before the bank cycle",
            Proof::ItemId {
                id: GUAM_LEAF_ID,
                count: 28,
            },
        ),
        (
            "watch exact unidentified guam consumed",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created clean guam enter a fresh bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 28,
            },
        ),
        (
            "watch a restock of exact unidentified guam",
            Proof::ItemId {
                id: UNIDENTIFIED_GUAM_ID,
                count: 1,
            },
        ),
        (
            "watch exact unidentified guam bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 2,
            },
        ),
    ];
    if named {
        watch.push((
            "watch the filtered marrentill unids stay in bank",
            Proof::BankItemId {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 4,
            },
        ));
        watch.push((
            "watch no filtered marrentill enter the pack",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        ("watch the script close its herb bank", Proof::BankClosed),
        (
            "watch another exact clean guam after restock",
            Proof::ItemId {
                id: GUAM_LEAF_ID,
                count: 1,
            },
        ),
        ("watch Herblore XP beyond the first pack", further_xp),
    ]);
    for (step_name, arm) in watch {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("HerbCleaner"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn gem_cutter_scenario() -> Scenario {
    gem_cutter_variant("gem_cutter", GEM_CUTTER_INJECT, false)
}

fn gem_cutter_named_scenario() -> Scenario {
    gem_cutter_variant("gem_cutter_named", GEM_CUTTER_NAMED_INJECT, true)
}

/// Empty pack, banked chisel plus uncut sapphires. Named also banks uncut
/// opal that must stay put. Deposit keeps the chisel.
fn gem_cutter_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    named: bool,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 2,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Crafting and uncut sapphire bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 20");
                cheat(c, "givebank chisel 1");
                cheat(c, "givebank uncut_sapphire 28");
                if named {
                    cheat(c, "givebank uncut_opal 4");
                }
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Crafting 20 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 20,
            },
        ),
        (
            "confirm no seeded chisel in pack before Start",
            Proof::ItemIdAtMost {
                id: CHISEL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded uncut sapphire in pack before Start",
            Proof::ItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cut sapphire before Start",
            Proof::ItemIdAtMost {
                id: SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no crushed gemstone before Start",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded uncut opal in pack before Start",
            Proof::ItemIdAtMost {
                id: UNCUT_OPAL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact uncut sapphire seed bank",
        Proof::BankItemId {
            id: UNCUT_SAPPHIRE_ID,
            count: 28,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact chisel seed bank",
            Proof::BankItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded cut sapphire in bank",
            Proof::BankItemIdAtMost {
                id: SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no crushed gemstone in bank",
            Proof::BankItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact untouched uncut opal seed bank",
            Proof::BankItemId {
                id: UNCUT_OPAL_ID,
                count: 4,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Crafting XP from cutting sapphire", first_xp),
        (
            "watch a chisel-kept pack of exact cut sapphires",
            Proof::ItemId {
                id: SAPPHIRE_ID,
                count: 27,
            },
        ),
        (
            "watch the chisel remain in pack",
            Proof::ItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "watch exact uncut sapphires consumed",
            Proof::ItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "watch no crushed gemstone from sapphire",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created sapphires enter a fresh bank",
            Proof::BankItemId {
                id: SAPPHIRE_ID,
                count: 27,
            },
        ),
        (
            "watch a restock of exact uncut sapphire",
            Proof::ItemId {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            },
        ),
        (
            "watch the chisel stay out of the deposit",
            Proof::ItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "watch exact uncut sapphire bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            },
        ),
    ];
    if named {
        watch.push((
            "watch the filtered uncut opal stay in bank",
            Proof::BankItemId {
                id: UNCUT_OPAL_ID,
                count: 4,
            },
        ));
        watch.push((
            "watch no filtered uncut opal enter the pack",
            Proof::ItemIdAtMost {
                id: UNCUT_OPAL_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        ("watch the script close its gem bank", Proof::BankClosed),
        (
            "watch another exact cut sapphire after restock",
            Proof::ItemId {
                id: SAPPHIRE_ID,
                count: 1,
            },
        ),
        ("watch Crafting XP beyond the first pack", further_xp),
        (
            "watch crushed gemstone stay empty",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
    ]);
    for (step_name, arm) in watch {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GemCutter"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const AGILITY_STAT: i32 = 16;
const FLAX_ID: i32 = 1779;
const GATE_CLOSED_ID: i32 = 1551;
const GATE_OPEN_ID: i32 = 1552;

/// Packed closed wooden door 1530 on selected 274/289 m50_50, adjacent to
/// DoorOpener's default Lumbridge stand.
const LUMBRIDGE_DOOR: WorldTile = WorldTile {
    x: 3208,
    z: 3211,
    level: 0,
};
const LUMBRIDGE_DOOR_STAND: WorldTile = WorldTile {
    x: 3208,
    z: 3212,
    level: 0,
};

/// Packed closed wooden gate 1551 on selected 274/289 m50_50.
const LUMBRIDGE_GATE: WorldTile = WorldTile {
    x: 3213,
    z: 3261,
    level: 0,
};
const LUMBRIDGE_GATE_STAND: WorldTile = WorldTile {
    x: 3213,
    z: 3260,
    level: 0,
};

const GNOME_START: WorldTile = WorldTile {
    x: 2474,
    z: 3436,
    level: 0,
};
/// Log Walk-across dest is coord z-7 from selected gnome_course.rs2.
const GNOME_AFTER_LOG: WorldTile = WorldTile {
    x: 2474,
    z: 3429,
    level: 0,
};
/// Climb-down lands at packed 0_38_53_55_28.
const GNOME_GROUND_RETURN: WorldTile = WorldTile {
    x: 2487,
    z: 3420,
    level: 0,
};
const GNOME_PIPE: WorldTile = WorldTile {
    x: 2484,
    z: 3431,
    level: 0,
};

const FLAX_FIELD: WorldTile = WorldTile {
    x: 2741,
    z: 3444,
    level: 0,
};

const DOOR_OPENER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "stand",
    value: ScriptInjectValue::Str("3208,3212,0"),
}];

const DOOR_OPENER_GATE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "stand",
        value: ScriptInjectValue::Str("3213,3260,0"),
    },
    ScriptSettingInject {
        id: "obstacle",
        value: ScriptInjectValue::Str("gate"),
    },
];

const GNOME_COURSE_RADIUS_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "searchRadius",
    value: ScriptInjectValue::Num(8.0),
}];

fn door_opener_scenario() -> Scenario {
    door_opener_variant(
        "door_opener",
        DOOR_OPENER_INJECT,
        LUMBRIDGE_DOOR,
        LUMBRIDGE_DOOR_STAND,
        CLOSED_ID,
        OPEN_ID,
    )
}

fn door_opener_gate_scenario() -> Scenario {
    door_opener_variant(
        "door_opener_gate",
        DOOR_OPENER_GATE_INJECT,
        LUMBRIDGE_GATE,
        LUMBRIDGE_GATE_STAND,
        GATE_CLOSED_ID,
        GATE_OPEN_ID,
    )
}

/// Walk to an adjacent stand, Close any open leaf before Start, then require
/// the selected shut loc to become the open id through a same-session world
/// change. Queued Open or script counters are not this proof.
fn door_opener_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    packed: WorldTile,
    stand: WorldTile,
    closed_id: i32,
    open_id: i32,
) -> Scenario {
    let shut = Proof::LocActionNear {
        id: closed_id,
        x: packed.x,
        z: packed.z,
        level: packed.level,
        radius: 1,
        action: "Open",
        present: true,
    };
    let opened = Proof::LocIdNear {
        id: open_id,
        x: packed.x,
        z: packed.z,
        level: packed.level,
        radius: 3,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele adjacent to the selected shut loc",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "close the selected loc if it is still open",
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                if let Some(loc) = snapshot.locs().iter().find(|loc| {
                    loc.id == open_id
                        && loc.tile.level == packed.level
                        && (loc.tile.x - packed.x)
                            .abs()
                            .max((loc.tile.z - packed.z).abs())
                            <= 3
                }) {
                    op_loc(c, loc.tile.x, loc.tile.z, loc.id);
                }
                true
            }),
        },
        wait: Wait {
            arm: shut,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch the selected loc become open after Start",
        opened,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: opened,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("DoorOpener"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn gnome_course_scenario() -> Scenario {
    gnome_course_variant("gnome_course", None)
}

fn gnome_course_radius_scenario() -> Scenario {
    gnome_course_variant("gnome_course_radius", Some(GNOME_COURSE_RADIUS_INJECT))
}

/// Complete a natural gnome lap, then cross the log again. Selected 274/289
/// content grants 86.5 XP per lap plus 7.5 for the next log. Stored milestones
/// are on the ground; snapshot levels follow the actual client plane.
fn gnome_course_variant(
    name: &'static str,
    inject: Option<&'static [ScriptSettingInject]>,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 94,
    };
    let start = GNOME_START;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele to the gnome course start before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(start.level, start.x, start.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: start.x,
                z: start.z,
                level: start.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Agility XP from the first obstacle", first_xp),
        (
            "watch the log dest tile after Walk-across",
            Proof::ArrivedNear {
                x: GNOME_AFTER_LOG.x,
                z: GNOME_AFTER_LOG.z,
                level: GNOME_AFTER_LOG.level,
                radius: 3,
            },
        ),
        (
            "watch the selected climb-down ground return",
            Proof::ArrivedNear {
                x: GNOME_GROUND_RETURN.x,
                z: GNOME_GROUND_RETURN.z,
                level: GNOME_GROUND_RETURN.level,
                radius: 3,
            },
        ),
        (
            "watch the obstacle pipe after the ground nets",
            Proof::ArrivedNear {
                x: GNOME_PIPE.x,
                z: GNOME_PIPE.z,
                level: GNOME_PIPE.level,
                radius: 6,
            },
        ),
        (
            "watch further Agility XP at the start of a second lap",
            further_xp,
        ),
        (
            "watch the log dest after second-lap progress",
            Proof::ArrivedNear {
                x: GNOME_AFTER_LOG.x,
                z: GNOME_AFTER_LOG.z,
                level: GNOME_AFTER_LOG.level,
                radius: 3,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GnomeCourse"),
            script_settings_inject: inject,
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn flax_picker_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the pack and tele to the default Seers flax field",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm no seeded flax in pack before Start",
        Proof::ItemIdAtMost {
            id: FLAX_ID,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch a full pack of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch script-created flax enter a fresh Seers bank",
            Proof::BankItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        ("watch the flax bank close after deposit", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        (
            "watch further exact flax after return",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_picker",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxPicker"),
            terminal_shot: Some("flax_picker"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const MAGIC_STAT: i32 = 6;
const SMITHING_STAT: i32 = 13;
const COPPER_ORE_ID: i32 = 436;
const TIN_ORE_ID: i32 = 438;
const IRON_ORE_ID: i32 = 440;
const COAL_ID: i32 = 453;
const FIRE_BATTLESTAFF_ID: i32 = 1393;
const BRONZE_BAR_ID: i32 = 2349;
const IRON_BAR_ID: i32 = 2351;
const STEEL_BAR_ID: i32 = 2353;
const SUPERHEAT_MAGIC: i32 = 43;
const BRONZE_SMITHING: i32 = 1;
const STEEL_SMITHING: i32 = 30;
const SUPERHEATER_NATURES_SEED: i32 = 200;
const SUPERHEATER_ORE_SEED: i32 = 100;
const SUPERHEATER_COAL_SEED: i32 = 200;

const SUPERHEATER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

const SUPERHEATER_STEEL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Steel"),
}];

const SUPERHEATER_FIRE_BATTLESTAFF_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

#[derive(Clone, Copy)]
enum SuperheaterStaff {
    Fire,
    FireBattlestaff,
}

#[derive(Clone, Copy)]
enum SuperheaterRecipe {
    Bronze,
    Steel,
}

fn superheater_scenario() -> Scenario {
    superheater_variant(
        "superheater",
        SUPERHEATER_INJECT,
        SuperheaterRecipe::Bronze,
        SuperheaterStaff::Fire,
    )
}

fn superheater_steel_scenario() -> Scenario {
    superheater_variant(
        "superheater_steel",
        SUPERHEATER_STEEL_INJECT,
        SuperheaterRecipe::Steel,
        SuperheaterStaff::Fire,
    )
}

fn superheater_fire_battlestaff_scenario() -> Scenario {
    superheater_variant(
        "superheater_fire_battlestaff",
        SUPERHEATER_FIRE_BATTLESTAFF_INJECT,
        SuperheaterRecipe::Bronze,
        SuperheaterStaff::FireBattlestaff,
    )
}

/// Empty pack at Varrock West. Banked staff, natures and recipe ores.
/// Script withdraws/equips the staff, casts Superheat Item on the primary
/// ore, deposits bars except natures, restocks and smelts again.
fn superheater_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    recipe: SuperheaterRecipe,
    staff: SuperheaterStaff,
) -> Scenario {
    let first_magic = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let first_smithing = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let further_magic = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 2,
    };
    let further_smithing = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 2,
    };
    let (bar_id, primary_id, secondary_id, smithing, staff_id, staff_alias) = match (recipe, staff)
    {
        (SuperheaterRecipe::Bronze, SuperheaterStaff::Fire) => (
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            TIN_ORE_ID,
            BRONZE_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Steel, SuperheaterStaff::Fire) => (
            STEEL_BAR_ID,
            IRON_ORE_ID,
            COAL_ID,
            STEEL_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Bronze, SuperheaterStaff::FireBattlestaff) => (
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            TIN_ORE_ID,
            BRONZE_SMITHING,
            FIRE_BATTLESTAFF_ID,
            "fire_battlestaff",
        ),
        (SuperheaterRecipe::Steel, SuperheaterStaff::FireBattlestaff) => {
            unreachable!("steel is a recipe split, not a staff split")
        }
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic, Smithing, bank stock and tele to Varrock West before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat magic {SUPERHEAT_MAGIC}"));
                cheat(c, &format!("setstat smithing {smithing}"));
                if matches!(staff, SuperheaterStaff::FireBattlestaff) {
                    // Both selected content revisions require Attack 30 to
                    // wield a Fire battlestaff (Magic 43 is seeded above).
                    cheat(c, "setstat attack 30");
                }
                cheat(c, &format!("givebank {staff_alias} 1"));
                cheat(
                    c,
                    &format!("givebank naturerune {SUPERHEATER_NATURES_SEED}"),
                );
                match recipe {
                    SuperheaterRecipe::Bronze => {
                        cheat(c, &format!("givebank copper_ore {SUPERHEATER_ORE_SEED}"));
                        cheat(c, &format!("givebank tin_ore {SUPERHEATER_ORE_SEED}"));
                    }
                    SuperheaterRecipe::Steel => {
                        cheat(c, &format!("givebank iron_ore {SUPERHEATER_ORE_SEED}"));
                        cheat(c, &format!("givebank coal {SUPERHEATER_COAL_SEED}"));
                    }
                }
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    let mut before_start = vec![
        (
            "confirm Magic 43 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: SUPERHEAT_MAGIC,
            },
        ),
        (
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
            },
        ),
        (
            "confirm no seeded staff in pack before Start",
            Proof::ItemIdAtMost {
                id: staff_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded natures in pack before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded primary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded secondary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded bars in pack before Start",
            Proof::ItemIdAtMost {
                id: bar_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded iron bars in pack before Start",
            Proof::ItemIdAtMost {
                id: IRON_BAR_ID,
                count: 0,
            },
        ),
    ];
    if matches!(staff, SuperheaterStaff::FireBattlestaff) {
        before_start.push((
            "confirm Attack 30 for Fire battlestaff before Start",
            Proof::Stat { id: 0, min: 30 },
        ));
        before_start.push((
            "confirm default Staff of fire is absent from pack",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ));
    }
    if matches!(recipe, SuperheaterRecipe::Bronze) {
        before_start.push((
            "confirm no seeded steel bars in pack before Start",
            Proof::ItemIdAtMost {
                id: STEEL_BAR_ID,
                count: 0,
            },
        ));
    } else {
        before_start.push((
            "confirm no seeded bronze bars in pack before Start",
            Proof::ItemIdAtMost {
                id: BRONZE_BAR_ID,
                count: 0,
            },
        ));
    }
    for (step_name, arm) in before_start {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact fire staff seed bank",
        Proof::BankItemId {
            id: staff_id,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact nature-rune seed bank",
        Proof::BankItemId {
            id: NATURE_RUNE_ID,
            count: SUPERHEATER_NATURES_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact primary ore seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: SUPERHEATER_ORE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary ore seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: match recipe {
                SuperheaterRecipe::Bronze => SUPERHEATER_ORE_SEED,
                SuperheaterRecipe::Steel => SUPERHEATER_COAL_SEED,
            },
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bars in bank",
        Proof::BankItemIdAtMost {
            id: bar_id,
            count: 0,
        },
    ));
    if matches!(staff, SuperheaterStaff::FireBattlestaff) {
        steps.push(bank_fletcher_watch(
            "acknowledge Staff of fire is absent from the alternative-staff bank",
            Proof::BankItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Magic XP from Superheat Item", first_magic),
        ("watch Smithing XP from the produced bar", first_smithing),
        (
            "watch the exact bar id after Start",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        (
            "watch at least one nature rune consumed",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 49,
            },
        ),
        (
            "watch script-created bars enter a fresh bank",
            Proof::BankItemId {
                id: bar_id,
                count: 1,
            },
        ),
        (
            "watch natures kept in pack across deposit",
            Proof::ItemId {
                id: NATURE_RUNE_ID,
                count: 1,
            },
        ),
        (
            "watch a restock of the exact primary ore",
            Proof::ItemId {
                id: primary_id,
                count: 1,
            },
        ),
        (
            "watch a restock of the exact secondary ore",
            Proof::ItemId {
                id: secondary_id,
                count: match recipe {
                    SuperheaterRecipe::Bronze => 1,
                    SuperheaterRecipe::Steel => 2,
                },
            },
        ),
    ];
    if matches!(staff, SuperheaterStaff::FireBattlestaff) {
        watch.push((
            "watch Staff of fire never enter the pack",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        (
            "watch no iron bar from a partial recipe",
            Proof::ItemIdAtMost {
                id: IRON_BAR_ID,
                count: 0,
            },
        ),
        (
            "watch the script close its superheat bank",
            Proof::BankClosed,
        ),
        (
            "watch another exact bar after restock",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        ("watch Magic XP beyond the first trip", further_magic),
        ("watch Smithing XP beyond the first trip", further_smithing),
    ]);
    for (step_name, arm) in watch {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_smithing,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Superheater"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const EMPTY_VIAL_ID: i32 = 229;
const VIAL_OF_WATER_ID: i32 = 227;
const EYE_OF_NEWT_ID: i32 = 221;
const GUAM_UNF_ID: i32 = 91;
const ATTACK_POTION_3_ID: i32 = 121;
const RANARR_WEED_ID: i32 = 257;
const RANARR_UNF_ID: i32 = 99;
const SNAPE_GRASS_ID: i32 = 231;
const PRAYER_POTION_3_ID: i32 = 139;
const POTION_BATCH_SEED: i32 = 42;
const VIAL_EMPTY_SEED: i32 = 56;
const GUAM_HERBLORE: i32 = 3;
/// Source has no herblore field. Seed high enough that Ranarr mixing is
/// not refused; this is not a generated-data requirement.
const RANARR_HERBLORE: i32 = 38;

// The seed opener performs once: start adjacent so Sent means bank operation.
const FALADOR_WEST_BANK: WorldTile = WorldTile {
    x: 2946,
    z: 3368,
    level: 0,
};
const FALADOR_WEST_BOOTH: WorldTile = WorldTile {
    x: 2946,
    z: 3367,
    level: 0,
};
const FALADOR_EAST_BANK: WorldTile = WorldTile {
    x: 3013,
    z: 3355,
    level: 0,
};
const FALADOR_EAST_BOOTH: WorldTile = WorldTile {
    x: 3013,
    z: 3354,
    level: 0,
};
const FALADOR_FOUNTAIN: WorldTile = WorldTile {
    x: 2949,
    z: 3381,
    level: 0,
};

const VIAL_FILLER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bank",
        value: ScriptInjectValue::Str("Falador West"),
    },
    ScriptSettingInject {
        id: "buyVials",
        value: ScriptInjectValue::Bool(false),
    },
];

const VIAL_FILLER_EAST_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bank",
        value: ScriptInjectValue::Str("Falador East"),
    },
    ScriptSettingInject {
        id: "buyVials",
        value: ScriptInjectValue::Bool(false),
    },
];

const POTION_MAKER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "herb",
        value: ScriptInjectValue::Str("Custom"),
    },
    ScriptSettingInject {
        id: "herbCustom",
        value: ScriptInjectValue::Str("Guam leaf"),
    },
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Custom"),
    },
    ScriptSettingInject {
        id: "secondaryCustom",
        value: ScriptInjectValue::Str("Eye of newt"),
    },
];

const POTION_MAKER_NAMED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "herb",
        value: ScriptInjectValue::Str("Ranarr weed"),
    },
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Snape grass"),
    },
];

const COW_HIDE_ID: i32 = 1739;
const SOFT_LEATHER_ID: i32 = 1741;
const HARD_LEATHER_ID: i32 = 1743;
const TANNER_HIDE_SEED: i32 = 28;
const TANNER_COIN_SEED: i32 = 5000;

const AL_KHARID_BANK: WorldTile = WorldTile {
    x: 3269,
    z: 3167,
    level: 0,
};
const TANNER_STAND: WorldTile = WorldTile {
    x: 3277,
    z: 3191,
    level: 0,
};

const TANNER_BOT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "hideType",
        value: ScriptInjectValue::Str("Soft leather"),
    },
    ScriptSettingInject {
        id: "buyThread",
        value: ScriptInjectValue::Bool(false),
    },
];

const TANNER_BOT_HARD_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "hideType",
        value: ScriptInjectValue::Str("Hard leather"),
    },
    ScriptSettingInject {
        id: "buyThread",
        value: ScriptInjectValue::Bool(false),
    },
];

fn open_seed_booth(name: &'static str, booth: WorldTile, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).open_booth_at(booth, VARROCK_WEST_BANK_BOOTH_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn vial_filler_scenario() -> Scenario {
    vial_filler_variant(
        "vial_filler",
        VIAL_FILLER_INJECT,
        FALADOR_WEST_BANK,
        FALADOR_WEST_BOOTH,
    )
}

fn vial_filler_east_scenario() -> Scenario {
    vial_filler_variant(
        "vial_filler_east",
        VIAL_FILLER_EAST_INJECT,
        FALADOR_EAST_BANK,
        FALADOR_EAST_BOOTH,
    )
}

/// Empty pack at the selected Falador bank. Banked empty vials, no water.
/// Script withdraws, fills at the west fountain, deposits produced water
/// vials, empties the pack of water, restocks empties, returns and fills
/// again. Shop-buy stays pending.
fn vial_filler_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    bank: WorldTile,
    booth: WorldTile,
) -> Scenario {
    let fountain = Proof::ArrivedNear {
        x: FALADOR_FOUNTAIN.x,
        z: FALADOR_FOUNTAIN.z,
        level: FALADOR_FOUNTAIN.level,
        radius: 4,
    };
    let filled = Proof::ItemId {
        id: VIAL_OF_WATER_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed empty vials and tele to the selected Falador bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank vial_empty {VIAL_EMPTY_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded empty vials in pack before Start",
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded water vials in pack before Start",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(open_seed_booth(
        "open and acknowledge the exact empty-vial seed bank",
        booth,
        Proof::BankItemId {
            id: EMPTY_VIAL_ID,
            count: VIAL_EMPTY_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded water vials in bank",
        Proof::BankItemIdAtMost {
            id: VIAL_OF_WATER_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the Falador fountain after Start",
            fountain,
        ),
        (
            "watch empty vials become water vials at the fountain",
            filled,
        ),
        (
            "watch the withdrawn empty vials finish filling",
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
                count: 0,
            },
        ),
        (
            "watch script-created water vials enter a fresh bank",
            Proof::BankItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of water vials after deposit",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact empty vials",
            Proof::ItemId {
                id: EMPTY_VIAL_ID,
                count: 1,
            },
        ),
        ("watch the script close its vial bank", Proof::BankClosed),
        (
            "watch return to the Falador fountain after restock",
            fountain,
        ),
        ("watch another exact water vial after restock", filled),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: filled,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("VialFiller"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn potion_maker_scenario() -> Scenario {
    potion_maker_variant(
        "potion_maker",
        POTION_MAKER_INJECT,
        PotionMakerRecipe {
            herb_id: GUAM_LEAF_ID,
            unf_id: GUAM_UNF_ID,
            secondary_id: EYE_OF_NEWT_ID,
            finished_id: ATTACK_POTION_3_ID,
            wrong_unf_id: RANARR_UNF_ID,
            wrong_finished_id: PRAYER_POTION_3_ID,
            herb_alias: "guam_leaf",
            secondary_alias: "eye_of_newt",
            herblore: GUAM_HERBLORE,
            named: false,
        },
    )
}

fn potion_maker_named_scenario() -> Scenario {
    potion_maker_variant(
        "potion_maker_named",
        POTION_MAKER_NAMED_INJECT,
        PotionMakerRecipe {
            herb_id: RANARR_WEED_ID,
            unf_id: RANARR_UNF_ID,
            secondary_id: SNAPE_GRASS_ID,
            finished_id: PRAYER_POTION_3_ID,
            wrong_unf_id: GUAM_UNF_ID,
            wrong_finished_id: ATTACK_POTION_3_ID,
            herb_alias: "ranarr_weed",
            secondary_alias: "snape_grass",
            herblore: RANARR_HERBLORE,
            named: true,
        },
    )
}

fn potion_maker_live_seed_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip and complete Druidic Ritual",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, "setvar tutorial 1000");
                    // Frozen quest def: druidquest is the server-only
                    // permanent varp; COMPLETE = 4. Relog paints the journal.
                    cheat(c, "setvar druidquest 4");
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
            name: "relog so the inv tab binds and journal refreshes",
            kind: StepKind::Relog,
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
        bank_fletcher_watch(
            "acknowledge Druidic Ritual before Start",
            Proof::QuestDone {
                name: "Druidic Ritual",
            },
        ),
    ]
}

/// Empty pack at Varrock West. Banked herb, water and secondary. Script
/// withdraws a batch, spam-uses herb onto water, withdraws the secondary,
/// finishes, deposits, restocks and makes further product. Named selector
/// also banks leftover Guam that must stay put.
#[derive(Clone, Copy)]
struct PotionMakerRecipe {
    herb_id: i32,
    unf_id: i32,
    secondary_id: i32,
    finished_id: i32,
    wrong_unf_id: i32,
    wrong_finished_id: i32,
    herb_alias: &'static str,
    secondary_alias: &'static str,
    herblore: i32,
    named: bool,
}

fn potion_maker_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    recipe: PotionMakerRecipe,
) -> Scenario {
    let PotionMakerRecipe {
        herb_id,
        unf_id,
        secondary_id,
        finished_id,
        wrong_unf_id,
        wrong_finished_id,
        herb_alias,
        secondary_alias,
        herblore,
        named,
    } = recipe;
    let first_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let finished = Proof::ItemId {
        id: finished_id,
        count: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = potion_maker_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore and potion ingredients before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat herblore {herblore}"));
                cheat(c, &format!("givebank {herb_alias} {POTION_BATCH_SEED}"));
                cheat(c, &format!("givebank vial_water {POTION_BATCH_SEED}"));
                cheat(
                    c,
                    &format!("givebank {secondary_alias} {POTION_BATCH_SEED}"),
                );
                if named {
                    cheat(c, "givebank guam_leaf 14");
                }
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    let mut before_start = vec![
        (
            "confirm Herblore before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: herblore,
            },
        ),
        (
            "confirm no seeded herb in pack before Start",
            Proof::ItemIdAtMost {
                id: herb_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded water vials in pack before Start",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded secondary in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded unfinished potion before Start",
            Proof::ItemIdAtMost {
                id: unf_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded finished potion before Start",
            Proof::ItemIdAtMost {
                id: finished_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong unfinished potion before Start",
            Proof::ItemIdAtMost {
                id: wrong_unf_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong finished potion before Start",
            Proof::ItemIdAtMost {
                id: wrong_finished_id,
                count: 0,
            },
        ),
    ];
    if named {
        before_start.push((
            "confirm no seeded leftover Guam in pack before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ));
    }
    for (step_name, arm) in before_start {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(open_seed_booth(
        "open and acknowledge the exact herb seed bank",
        WorldTile {
            x: VARROCK_WEST_BANK.x + 1,
            z: VARROCK_WEST_BANK.z,
            level: VARROCK_WEST_BANK.level,
        },
        Proof::BankItemId {
            id: herb_id,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact water-vial seed bank",
        Proof::BankItemId {
            id: VIAL_OF_WATER_ID,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded unfinished potion in bank",
        Proof::BankItemIdAtMost {
            id: unf_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded finished potion in bank",
        Proof::BankItemIdAtMost {
            id: finished_id,
            count: 0,
        },
    ));
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact leftover Guam seed bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 14,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        (
            "watch herb plus water become the exact unfinished potion",
            Proof::ItemId {
                id: unf_id,
                count: 1,
            },
        ),
        (
            "watch the withdrawn herb consumed into unfinished potions",
            Proof::ItemIdAtMost {
                id: herb_id,
                count: 0,
            },
        ),
        (
            "watch the withdrawn water vials consumed into unfinished potions",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "watch the unfinished potions become the exact finished potion",
            finished,
        ),
        (
            "watch unfinished potions consumed into the finished product",
            Proof::ItemIdAtMost {
                id: unf_id,
                count: 0,
            },
        ),
        ("watch Herblore XP from finishing potions", first_xp),
        (
            "watch no wrong unfinished potion",
            Proof::ItemIdAtMost {
                id: wrong_unf_id,
                count: 0,
            },
        ),
        (
            "watch no wrong finished potion",
            Proof::ItemIdAtMost {
                id: wrong_finished_id,
                count: 0,
            },
        ),
        (
            "watch script-created finished potions enter a fresh bank",
            Proof::BankItemId {
                id: finished_id,
                count: 1,
            },
        ),
        (
            "watch a restock of exact water vials",
            Proof::ItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
        ),
        (
            "watch a restock of the exact herb",
            Proof::ItemId {
                id: herb_id,
                count: 1,
            },
        ),
        (
            "watch finished potions leave the pack across deposit/restock",
            Proof::ItemIdAtMost {
                id: finished_id,
                count: 0,
            },
        ),
    ];
    if named {
        watch.push((
            "watch leftover Guam stay in bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 14,
            },
        ));
        watch.push((
            "watch leftover Guam never enter the pack",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        (
            "watch the script close its potion restock bank",
            Proof::BankClosed,
        ),
        (
            "watch another exact unfinished potion after restock",
            Proof::ItemId {
                id: unf_id,
                count: 1,
            },
        ),
        (
            "watch another exact finished potion after restock",
            finished,
        ),
    ]);
    for (step_name, arm) in watch {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: finished,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("PotionMaker"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn tanner_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).open_nearest_booth(),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn tanner_bot_scenario() -> Scenario {
    tanner_bot_variant(
        "tanner_bot",
        TANNER_BOT_INJECT,
        SOFT_LEATHER_ID,
        HARD_LEATHER_ID,
    )
}

fn tanner_bot_hard_scenario() -> Scenario {
    tanner_bot_variant(
        "tanner_bot_hard",
        TANNER_BOT_HARD_INJECT,
        HARD_LEATHER_ID,
        SOFT_LEATHER_ID,
    )
}

/// Empty pack at Al-Kharid bank. Banked cowhides 1739 and coins, never leather.
/// Script withdraws, tans at the Tanner widget (not a shop), deposits produced
/// leather, empties the pack of leather, restocks hides, returns and tans again.
/// Dommik thread-buy stays pending.
fn tanner_bot_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    product_id: i32,
    wrong_product_id: i32,
) -> Scenario {
    let tanner = Proof::ArrivedNear {
        x: TANNER_STAND.x,
        z: TANNER_STAND.z,
        level: TANNER_STAND.level,
        radius: 4,
    };
    let leather = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed cowhides, coins, and tele to Al-Kharid bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank cow_hide {TANNER_HIDE_SEED}"));
                cheat(c, &format!("givebank coins {TANNER_COIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded cowhides in pack before Start",
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded soft leather in pack before Start",
            Proof::ItemIdAtMost {
                id: SOFT_LEATHER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded hard leather in pack before Start",
            Proof::ItemIdAtMost {
                id: HARD_LEATHER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact cowhide seed bank",
        Proof::BankItemId {
            id: COW_HIDE_ID,
            count: TANNER_HIDE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact coin seed bank",
        Proof::BankItemId {
            id: COINS_ID,
            count: TANNER_COIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded leather in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded wrong leather in bank",
        Proof::BankItemIdAtMost {
            id: wrong_product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Tanner after Start", tanner),
        (
            "watch cowhides become the selected leather at the Tanner",
            leather,
        ),
        (
            "watch the withdrawn cowhides finish converting",
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
        ),
        (
            "watch script-created leather enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of leather after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact cowhides",
            Proof::ItemId {
                id: COW_HIDE_ID,
                count: 1,
            },
        ),
        ("watch the script close its tanner bank", Proof::BankClosed),
        ("watch return to the Tanner after restock", tanner),
        ("watch another exact leather after restock", leather),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: leather,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("TannerBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Lumbridge courtyard stand where the two-bot trade meets.
const TRADE_COURTYARD: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};

/// Per-frame state for the trade-accept companion (profile 1).
#[derive(Default)]
struct TradeAcceptSlot {
    scene2_seen: bool,
    tele_sent: bool,
}

/// The `script_trade` scenario: a two-bot fleet — both profiles Start the
/// in-tree `TradeBot` fixture (Compat `Trade.request` / `offerAll` /
/// `accept`) with reciprocal `partner` inject; profile 1 also rust-teles
/// beside the driven bot after the mainland hop and presses Accept on
/// both trade screens. Proof: the driven slot holds zero Coins after
/// offering the seeded stack of twenty-five.
fn script_trade_scenario() -> Scenario {
    let courtyard = TRADE_COURTYARD;
    Scenario {
        name: "script_trade",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "stick tutorial skip and seed twenty-five coins",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give coins 25");
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
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to Lumbridge courtyard",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(courtyard.level, courtyard.x, courtyard.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: courtyard.x,
                        z: courtyard.z,
                        level: courtyard.level,
                    },
                    budget_ticks: 120,
                },
            },
            start_catalog_step(),
            Step {
                name: "watch the trade consume coins",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ItemAtMost {
                        name: "Coins",
                        count: 0,
                    },
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            },
        ],
        proof: Proof::ItemAtMost {
            name: "Coins",
            count: 0,
        },
        companions: vec![Companion {
            profile: 1,
            per_frame: {
                let mut slot = TradeAcceptSlot::default();
                Box::new(move |c| trade_acceptor_frame(c, &mut slot))
            },
        }],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("TradeBot"),
            inject_companion_as: Some("partner"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn trade_acceptor_frame(c: &mut Client, s: &mut TradeAcceptSlot) {
    let Some(lp) = &c.local_player else {
        if debug_enabled() {
            eprintln!("[trade-companion] no local_player scene={}", c.scene_state);
        }
        return;
    };
    let here = WorldTile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    if stage_trade_companion_tele(c, here, s) {
        return;
    }
    press_trade_accept(c);
}

/// Host-play queues `mainland_hop` after the first `scene_state == 2`
/// companion frame; cheat-tele beside the driven bot once ingame on a
/// mainland build base (the runner seed gate), not only when `here.x > 3100`.
fn stage_trade_companion_tele(c: &mut Client, here: WorldTile, s: &mut TradeAcceptSlot) -> bool {
    if at_trade_courtyard(here) || s.tele_sent {
        return false;
    }
    if c.scene_state != 2 {
        return false;
    }
    if !s.scene2_seen {
        s.scene2_seen = true;
        return false;
    }
    if c.map_build_base_x < 3000 {
        if debug_enabled() {
            eprintln!(
                "[trade-companion] waiting mainland base, here={here:?} base_x={}",
                c.map_build_base_x
            );
        }
        return false;
    }
    if debug_enabled() {
        eprintln!(
            "[trade-companion] tele {} from {here:?}",
            tele_args(TRADE_COURTYARD.level, TRADE_COURTYARD.x, TRADE_COURTYARD.z)
        );
    }
    cheat(
        c,
        &tele_args(TRADE_COURTYARD.level, TRADE_COURTYARD.x, TRADE_COURTYARD.z),
    );
    s.tele_sent = true;
    true
}

fn at_trade_courtyard(here: WorldTile) -> bool {
    here.x == TRADE_COURTYARD.x
        && here.z == TRADE_COURTYARD.z
        && here.level == TRADE_COURTYARD.level
}

/// Fail-closed when the trade screen is open but the posted accept
/// component id is absent (companion cannot press Accept).
fn trade_accept_missing_block(trade: &api::snapshot::TradeView) -> Option<&'static str> {
    if (trade.offer_open || trade.confirm_open) && trade.accept_component_id < 0 {
        Some("BLOCKED: missing trade accept com")
    } else {
        None
    }
}

fn press_trade_accept(c: &mut Client) {
    let mut snap = GameSnapshot::default();
    snap.rebuild(c);
    let trade = snap.trade();
    if let Some(msg) = trade_accept_missing_block(trade) {
        fail(msg);
    }
    if !trade.offer_open && !trade.confirm_open {
        return;
    }
    let mut ix = Interactions::new(&snap, c);
    let ctx = ReadContext::new(&snap);
    let Some(widget) = ctx.component(trade.accept_component_id) else {
        return;
    };
    match ix.press(widget) {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
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
        Err(_) => match client::operator_home() {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => PathBuf::from(".274bot/274bot.navpack"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_s_from_parses_rs2b0t_style_seconds() {
        assert_eq!(budget_s_from(None), None);
        assert_eq!(budget_s_from(Some("")), None);
        assert_eq!(budget_s_from(Some("0")), None);
        assert_eq!(budget_s_from(Some("nope")), None);
        assert_eq!(budget_s_from(Some("300")), Some(Duration::from_secs(300)));
        assert_eq!(budget_s_from(Some(" 60 ")), Some(Duration::from_secs(60)));
    }

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
                "chicken_killer",
                "chicken_killer_bank",
                "thiever",
                "alcher",
                "alcher_custom",
                "alcher_custom_alias",
                "alcher_custom_name",
                "alcher_ordered",
                "alcher_large_batch",
                "bank_fletcher",
                "bank_fletcher_string",
                "bank_fletcher_cut_string",
                "dart_fletcher",
                "dart_fletcher_iron",
                "herb_cleaner",
                "herb_cleaner_named",
                "gem_cutter",
                "gem_cutter_named",
                "door_opener",
                "door_opener_gate",
                "gnome_course",
                "gnome_course_radius",
                "flax_picker",
                "superheater",
                "superheater_steel",
                "superheater_fire_battlestaff",
                "vial_filler",
                "vial_filler_east",
                "potion_maker",
                "potion_maker_named",
                "tanner_bot",
                "tanner_bot_hard",
                "script_trade",
            ]
        );
    }

    #[test]
    fn script_trade_is_a_two_profile_fleet_with_a_rust_acceptor_companion() {
        let s = get("script_trade").expect("script_trade is registered");
        assert_eq!(s.name, "script_trade");
        assert_eq!(s.seed.profiles, [("test", "test"), ("test2", "test2")]);
        assert!(
            s.seed.mainland,
            "the hop lands both bots before the trade tele"
        );
        assert_eq!(
            s.steps.len(),
            5,
            "seed coins, relog, tele, StartScript, watch coins"
        );
        assert!(matches!(s.steps[0].kind, StepKind::Perform { .. }));
        assert!(matches!(s.steps[1].kind, StepKind::Relog));
        assert_eq!(s.steps[1].wait.arm, Proof::SideTabAvailable { index: 3 });
        assert!(matches!(s.steps[2].kind, StepKind::Perform { .. }));
        assert!(matches!(s.steps[3].kind, StepKind::StartScript));
        assert_eq!(
            s.steps[4].wait.arm,
            Proof::ItemAtMost {
                name: "Coins",
                count: 0
            }
        );
        assert_eq!(
            s.proof.name(),
            "has_item(Coins)<=0",
            "terminal proof is zero coins on the driven slot"
        );
        assert_eq!(
            s.companions.len(),
            1,
            "profile 1 rust-teles and rust-accepts"
        );
        assert_eq!(s.companions[0].profile, 1);
        assert_eq!(s.settings.start_script, Some("TradeBot"));
        assert_eq!(s.settings.inject_companion_as, Some("partner"));
        assert!(s.settings.full_rate);
        assert!(s.settings.require_mainland_base);
        assert_eq!(s.settings.nav.engine_speed_ms, Some(600));
        assert!(names().contains(&"script_trade"));
    }

    #[test]
    fn script_trade_starts_tradebot_after_relog_and_courtyard_tele() {
        let s = get("script_trade").unwrap();
        let i = s
            .steps
            .iter()
            .position(|st| matches!(st.kind, StepKind::StartScript))
            .expect("StartScript step");
        assert!(
            matches!(s.steps[i - 1].kind, StepKind::Perform { .. }),
            "StartScript follows the courtyard tele"
        );
        assert_eq!(
            s.steps[i - 1].wait.arm,
            Proof::Arrived {
                x: 3220,
                z: 3220,
                level: 0,
            }
        );
        assert!(
            matches!(s.steps[i - 2].kind, StepKind::Relog),
            "StartScript follows the relog"
        );
        assert_eq!(s.steps[i].wait.arm, Proof::Stat { id: 16, min: 0 });
        assert_eq!(s.steps[i].wait.budget_ticks, 1);
    }

    #[test]
    fn script_trade_companion_blocks_when_trade_open_without_accept_id() {
        use api::snapshot::GameSnapshot;
        use client::client::{Client, ClientConfig};
        use client::dash3d::ClientPlayer;
        use client::io::ServerProt;

        let mut c = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        c.ingame = true;
        c.scene_state = 2;
        c.local_player = Some(ClientPlayer::at(20, 20));
        // TRADEMAIN without Accept/Decline buttons in the modal tree.
        c.main_modal_id = 3323;
        c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);

        let mut snap = GameSnapshot::default();
        snap.rebuild(&c);
        let trade = snap.trade();
        assert!(trade.offer_open, "offer screen must be open");
        assert!(
            trade.accept_component_id < 0,
            "accept id must be missing without iface buttons"
        );
        assert_eq!(
            trade_accept_missing_block(trade),
            Some("BLOCKED: missing trade accept com")
        );

        c.main_modal_id = 3443; // TRADECONFIRM
        c.bump_gens(ServerProt::IF_OPENMAIN_SIDE);
        snap.rebuild(&c);
        let trade = snap.trade();
        assert!(trade.confirm_open, "confirm screen must be open");
        assert!(trade.accept_component_id < 0);
        assert_eq!(
            trade_accept_missing_block(trade),
            Some("BLOCKED: missing trade accept com")
        );
    }

    #[test]
    fn gold_script_scenarios_register_start_script_names() {
        let cases = [
            ("bone_burier", "BoneBurier"),
            ("chicken_killer", "ChickenKiller"),
            ("thiever", "Thiever"),
            ("alcher", "Alcher"),
            ("bank_fletcher", "BankFletcher"),
            ("script_trade", "TradeBot"),
        ];
        for (name, card) in cases {
            let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
            assert_eq!(
                s.settings.start_script,
                Some(card),
                "{name} must start the {card} catalog card"
            );
            assert!(
                s.settings.require_mainland_base,
                "{name} waits for mainland scene 2"
            );
            assert_eq!(
                s.settings.nav.engine_speed_ms,
                Some(600),
                "{name} cheats speed 600 so a leftover nav speed 300 is not inherited"
            );
        }
        for name in ["thiever", "bank_fletcher"] {
            let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
            assert!(
                s.steps
                    .iter()
                    .any(|st| matches!(st.kind, StepKind::DrainDialogs { .. })),
                "{name} drains advancestat level-up IFs with DrainDialogs"
            );
        }
        let alcher = get("alcher").expect("alcher");
        assert_eq!(
            alcher.settings.start_script,
            Some("Alcher"),
            "get(\"alcher\") has start_script: Some(\"Alcher\")"
        );
        assert!(
            alcher.settings.script_settings_inject.is_some(),
            "Alcher injects the items bag"
        );
        assert!(
            alcher
                .steps
                .iter()
                .any(|st| matches!(st.wait.arm, Proof::Stat { id: 6, min: 55 })),
            "alcher seed waits for Magic ≥ 55 before Start/watch, not arrival alone"
        );
        let thiever = get("thiever").expect("thiever");
        assert!(
            thiever
                .settings
                .script_settings_inject
                .is_some_and(|rows| rows.iter().any(|r| r.id == "loot")),
            "Thiever injects loot off"
        );
        assert!(names().contains(&"chicken_killer"));
        assert!(names().contains(&"bank_fletcher"));
    }

    #[test]
    fn gold_scripts_start_the_catalog_after_the_last_seed_wait() {
        fn start_idx(name: &str) -> usize {
            let s = get(name).unwrap_or_else(|| panic!("{name} is registered"));
            s.steps
                .iter()
                .position(|st| matches!(st.kind, StepKind::StartScript))
                .unwrap_or_else(|| panic!("{name} has a StartScript step after the last seed wait"))
        }

        // bone_burier: after relog SideTabAvailable, before watch bury chat
        let bone = get("bone_burier").unwrap();
        let i = start_idx("bone_burier");
        assert!(
            matches!(bone.steps[i - 1].kind, StepKind::Relog),
            "bone_burier StartScript follows the relog"
        );
        assert_eq!(
            bone.steps[i - 1].wait.arm,
            Proof::SideTabAvailable { index: 3 }
        );
        assert_eq!(
            bone.steps[i + 1].wait.arm,
            Proof::StatXpGain { id: 5, min: 22 }
        );
        assert_eq!(bone.steps[i].wait.arm, Proof::Stat { id: 16, min: 0 });
        assert_eq!(bone.steps[i].wait.budget_ticks, 1);

        // chicken_killer: anchors at Start — host tele to pen after seed, before watch XP
        let chickens = get("chicken_killer").unwrap();
        let i = start_idx("chicken_killer");
        assert_eq!(
            chickens.steps[i - 1].wait.arm,
            Proof::ArrivedNear {
                x: 3235,
                z: 3295,
                level: 0,
                radius: 8,
            },
            "chicken_killer StartScript follows pen tele (3235,3295)"
        );
        assert_eq!(
            chickens.steps[i + 1].wait.arm,
            Proof::StatXpGain { id: 2, min: 1 }
        );
        assert!(
            chickens
                .settings
                .script_settings_inject
                .as_ref()
                .map(|rows| !rows.iter().any(|r| r.id == "banking"))
                .unwrap_or(true),
            "chicken_killer banking stays off (no inject)"
        );
        assert_eq!(chickens.settings.start_script, Some("ChickenKiller"));
        assert!(chickens.settings.require_mainland_base);

        // chicken_killer_bank: Falador pen after seed, loot-count inject, then
        // combat / exact feather / fresh deposit / return r6 / new feathers.
        let bank = get("chicken_killer_bank").unwrap();
        let i = start_idx("chicken_killer_bank");
        assert_eq!(
            bank.steps[i - 1].wait.arm,
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: 0
            },
            "chicken_killer_bank StartScript follows empty-feather ack"
        );
        assert_eq!(
            bank.steps[i - 4].wait.arm,
            Proof::ArrivedNear {
                x: 3029,
                z: 3294,
                level: 0,
                radius: 8,
            },
            "chicken_killer_bank teles to Falador south chickens"
        );
        assert_eq!(bank.settings.start_script, Some("ChickenKiller"));
        assert!(bank.settings.require_mainland_base);

        // thiever: after tele + DrainDialogs, before watch XP
        let thiever = get("thiever").unwrap();
        let i = start_idx("thiever");
        assert!(
            matches!(thiever.steps[i - 1].kind, StepKind::DrainDialogs { .. }),
            "thiever StartScript follows DrainDialogs"
        );
        assert_eq!(
            thiever.steps[i + 1].wait.arm,
            Proof::StatXpGain { id: 17, min: 1 }
        );

        // alcher: after Magic ≥55 and tele ArrivedNear West bank, before watch XP
        let alcher = get("alcher").unwrap();
        let i = start_idx("alcher");
        assert!(
            alcher.steps[..i]
                .iter()
                .any(|st| matches!(st.wait.arm, Proof::Stat { id: 6, min: 55 })),
            "alcher StartScript is after Magic ≥55"
        );
        assert_eq!(
            alcher.steps[i - 1].wait.arm,
            Proof::ArrivedNear {
                x: 3185,
                z: 3440,
                level: 0,
                radius: 6,
            }
        );
        assert_eq!(
            alcher.steps[i + 1].wait.arm,
            Proof::StatXpGain { id: 6, min: 1 }
        );

        // bank_fletcher: after last seed wait (tele + drain), before watch XP
        let fletcher = get("bank_fletcher").unwrap();
        let i = start_idx("bank_fletcher");
        assert!(
            matches!(fletcher.steps[i - 1].kind, StepKind::DrainDialogs { .. }),
            "bank_fletcher StartScript follows the last seed wait"
        );
        assert_eq!(
            fletcher.steps[i + 1].wait.arm,
            Proof::StatXpGain { id: 9, min: 1 }
        );
    }

    #[test]
    fn bank_fletcher_string_modes_are_registered_with_id_strict_proofs() {
        let string = get("bank_fletcher_string").expect("string scenario registered");
        assert_eq!(string.settings.start_script, Some("BankFletcher"));
        assert_eq!(string.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(string.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("material"),
            Some(&Value::String("Willow logs".into()))
        );
        assert_eq!(
            inject.get("product"),
            Some(&Value::String("String short bow".into()))
        );
        assert!(
            !inject.contains_key("mode"),
            "old catalog has no mode setting"
        );
        let string_start = string
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(
            string.steps[string_start + 1].wait.arm,
            Proof::StatXpGain { id: 9, min: 66 },
            "the XP baseline must be armed before the first seeded pair finishes"
        );
        let string_seed_arms = string.steps[..string_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(string_seed_arms.contains(&Proof::BankItemId { id: 60, count: 28 }));
        assert!(string_seed_arms.contains(&Proof::BankItemId {
            id: 1777,
            count: 28,
        }));
        assert!(string_seed_arms.contains(&Proof::BankItemIdAtMost { id: 849, count: 0 }));
        assert_eq!(string.steps[string_start - 1].wait.arm, Proof::BankClosed);
        let string_arms = string
            .steps
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(string_arms.contains(&Proof::ItemId { id: 849, count: 2 }));
        assert!(string_arms.contains(&Proof::BankItemId { id: 849, count: 2 }));
        assert!(string_arms.contains(&Proof::ItemId { id: 60, count: 14 }));
        assert_eq!(string.proof, Proof::StatXpGain { id: 9, min: 67 });

        let combined = get("bank_fletcher_cut_string").expect("combined scenario registered");
        assert_eq!(combined.settings.start_script, Some("BankFletcher"));
        assert_eq!(combined.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(combined.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("mode"),
            Some(&Value::String("cut+string".into()))
        );
        assert_eq!(
            inject.get("material"),
            Some(&Value::String("Willow logs".into()))
        );
        assert_eq!(
            inject.get("product"),
            Some(&Value::String("Short bow".into()))
        );
        let combined_start = combined
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(
            combined.steps[combined_start + 1].wait.arm,
            Proof::StatXpGain { id: 9, min: 66 },
            "the XP baseline must be armed before the cut pair finishes"
        );
        let combined_seed_arms = combined.steps[..combined_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(combined_seed_arms.contains(&Proof::BankItemId {
            id: 1777,
            count: 28,
        }));
        assert!(combined_seed_arms.contains(&Proof::BankItemIdAtMost { id: 1519, count: 0 }));
        assert!(combined_seed_arms.contains(&Proof::BankItemIdAtMost { id: 60, count: 0 }));
        assert!(combined_seed_arms.contains(&Proof::BankItemIdAtMost { id: 849, count: 0 }));
        assert_eq!(
            combined.steps[combined_start - 1].wait.arm,
            Proof::BankClosed
        );
        let combined_arms = combined
            .steps
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(combined_arms.contains(&Proof::ItemId { id: 60, count: 2 }));
        assert!(combined_arms.contains(&Proof::BankItemId { id: 60, count: 2 }));
        assert!(combined_arms.contains(&Proof::ItemId { id: 849, count: 2 }));
        let close = combined_arms
            .iter()
            .rposition(|proof| *proof == Proof::BankClosed)
            .unwrap();
        assert_eq!(
            combined_arms[close + 1..close + 4],
            [
                Proof::StatXpGain { id: 9, min: 100 },
                Proof::ItemId { id: 849, count: 2 },
                Proof::ItemIdAtMost { id: 60, count: 0 },
            ],
            "stringing XP must be observed before the exact inputs are exhausted"
        );
        assert_eq!(combined.proof, Proof::ItemId { id: 849, count: 2 });
        assert!(names().contains(&"bank_fletcher_string"));
        assert!(names().contains(&"bank_fletcher_cut_string"));
    }

    #[test]
    fn alcher_generated_custom_cases_select_custom_and_ack_exact_ids() {
        for (name, custom_item) in [
            ("alcher_custom_alias", "adamant_scimitar"),
            ("alcher_custom_name", "Adamant scimitar"),
        ] {
            let scenario = get(name).unwrap_or_else(|| panic!("{name} is registered"));
            assert_eq!(scenario.settings.start_script, Some("Alcher"));
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
            assert_eq!(scenario.settings.terminal_shot, Some(name));
            let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
            assert_eq!(
                inject.get("items"),
                Some(&Value::Array(vec![Value::String("custom".into())]))
            );
            assert_eq!(
                inject.get("customItem"),
                Some(&Value::String(custom_item.into()))
            );
            assert_eq!(inject.get("alchs"), Some(&Value::from(1.0)));
            let start = scenario
                .steps
                .iter()
                .position(|step| matches!(step.kind, StepKind::StartScript))
                .unwrap();
            assert_eq!(scenario.steps[start - 1].wait.arm, Proof::BankClosed);
            let seed_arms = scenario.steps[..start]
                .iter()
                .map(|step| step.wait.arm)
                .collect::<Vec<_>>();
            assert!(seed_arms.contains(&Proof::BankItemId {
                id: ADAMANT_SCIMITAR_ID,
                count: 1,
            }));
            assert!(seed_arms.contains(&Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 1,
            }));
            assert!(seed_arms.contains(&Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            }));
            assert!(seed_arms.contains(&Proof::BankItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            }));
            assert!(seed_arms.contains(&Proof::ItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            }));
            assert!(seed_arms.contains(&Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            }));
            let watch_arms = scenario.steps[start + 1..]
                .iter()
                .map(|step| step.wait.arm)
                .collect::<Vec<_>>();
            assert_eq!(
                watch_arms[0],
                Proof::ItemId {
                    id: CERT_ADAMANT_SCIMITAR_ID,
                    count: 1,
                },
                "{name} must observe the noted id before XP"
            );
            assert!(watch_arms.contains(&Proof::StatXpGain {
                id: 6,
                min: HIGH_ALCH_MAGIC_XP,
            }));
            assert!(watch_arms.contains(&Proof::ItemId {
                id: COINS_ID,
                count: ADAMANT_SCIMITAR_ALCH_COINS,
            }));
            assert_eq!(
                scenario.proof,
                Proof::StatXpGain {
                    id: 6,
                    min: HIGH_ALCH_MAGIC_XP,
                }
            );
            assert!(names().contains(&name));
        }

        let historical = get("alcher_custom").expect("historical custom case preserved");
        let inject = settings_inject_map(historical.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("customItem"),
            Some(&Value::String("rune_chainbody".into()))
        );
    }

    #[test]
    fn inventory_production_cases_register_exact_ids_and_bank_cycles() {
        let bronze = get("dart_fletcher").expect("dart_fletcher");
        assert_eq!(bronze.settings.start_script, Some("DartFletcher"));
        assert_eq!(bronze.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(bronze.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("tier"), Some(&Value::String("Bronze".into())));
        let bronze_start = bronze
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_ne!(bronze.steps[bronze_start - 1].wait.arm, Proof::BankClosed);
        let bronze_seed = bronze.steps[..bronze_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(bronze_seed.contains(&Proof::ItemId {
            id: BRONZE_DART_TIP_ID,
            count: 100,
        }));
        assert!(bronze_seed.contains(&Proof::ItemId {
            id: FEATHER_ID,
            count: 100,
        }));
        assert!(bronze_seed.contains(&Proof::ItemIdAtMost {
            id: BRONZE_DART_ID,
            count: 0,
        }));
        let bronze_watch = bronze.steps[bronze_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            bronze_watch[0],
            Proof::StatXpGain {
                id: FLETCHING_STAT,
                min: 1
            }
        );
        assert!(bronze_watch.contains(&Proof::ItemId {
            id: BRONZE_DART_ID,
            count: 10,
        }));
        assert!(bronze_watch.contains(&Proof::ItemId {
            id: BRONZE_DART_ID,
            count: 20,
        }));
        assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
            id: BRONZE_DART_TIP_ID,
            count: 90,
        }));
        assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
            id: FEATHER_ID,
            count: 90,
        }));
        assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
            id: IRON_DART_ID,
            count: 0,
        }));
        assert_eq!(
            bronze.proof,
            Proof::StatXpGain {
                id: FLETCHING_STAT,
                min: 2
            }
        );

        let iron = get("dart_fletcher_iron").expect("dart_fletcher_iron");
        let inject = settings_inject_map(iron.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("tier"), Some(&Value::String("Iron".into())));
        let iron_start = iron
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let iron_seed = iron.steps[..iron_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(iron_seed.contains(&Proof::Stat {
            id: FLETCHING_STAT,
            min: 22,
        }));
        assert!(iron_seed.contains(&Proof::ItemId {
            id: IRON_DART_TIP_ID,
            count: 100,
        }));
        let iron_watch = iron.steps[iron_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(iron_watch.contains(&Proof::ItemId {
            id: IRON_DART_ID,
            count: 10,
        }));
        assert!(iron_watch.contains(&Proof::ItemIdAtMost {
            id: BRONZE_DART_ID,
            count: 0,
        }));

        let herb = get("herb_cleaner").expect("herb_cleaner");
        assert_eq!(herb.settings.start_script, Some("HerbCleaner"));
        let inject = settings_inject_map(herb.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("herbs"), Some(&Value::Array(vec![])));
        let herb_start = herb
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(herb.steps[herb_start - 1].wait.arm, Proof::BankClosed);
        let herb_seed = herb.steps[..herb_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(herb_seed.contains(&Proof::Stat {
            id: HERBLORE_STAT,
            min: 3,
        }));
        assert!(herb_seed.contains(&Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 30,
        }));
        assert!(herb_seed.contains(&Proof::ItemIdAtMost {
            id: GUAM_LEAF_ID,
            count: 0,
        }));
        let herb_watch = herb.steps[herb_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            herb_watch[0],
            Proof::StatXpGain {
                id: HERBLORE_STAT,
                min: 1
            }
        );
        assert!(herb_watch.contains(&Proof::ItemId {
            id: GUAM_LEAF_ID,
            count: 28,
        }));
        assert!(herb_watch.contains(&Proof::BankItemId {
            id: GUAM_LEAF_ID,
            count: 28,
        }));
        assert!(herb_watch.contains(&Proof::ItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 1,
        }));
        assert_eq!(
            herb.proof,
            Proof::StatXpGain {
                id: HERBLORE_STAT,
                min: 2
            }
        );

        let named_herb = get("herb_cleaner_named").expect("herb_cleaner_named");
        let inject = settings_inject_map(named_herb.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("herbs"),
            Some(&Value::Array(vec![Value::String("Guam leaf".into())]))
        );
        let named_herb_start = named_herb
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let named_herb_seed = named_herb.steps[..named_herb_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(named_herb_seed.contains(&Proof::Stat {
            id: HERBLORE_STAT,
            min: 5,
        }));
        assert!(named_herb_seed.contains(&Proof::BankItemId {
            id: UNIDENTIFIED_MARENTILL_ID,
            count: 4,
        }));
        let named_herb_watch = named_herb.steps[named_herb_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(named_herb_watch.contains(&Proof::BankItemId {
            id: UNIDENTIFIED_MARENTILL_ID,
            count: 4,
        }));
        assert!(named_herb_watch.contains(&Proof::ItemIdAtMost {
            id: UNIDENTIFIED_MARENTILL_ID,
            count: 0,
        }));

        let gems = get("gem_cutter").expect("gem_cutter");
        assert_eq!(gems.settings.start_script, Some("GemCutter"));
        let inject = settings_inject_map(gems.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("gems"), Some(&Value::Array(vec![])));
        let gems_start = gems
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(gems.steps[gems_start - 1].wait.arm, Proof::BankClosed);
        let gems_seed = gems.steps[..gems_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(gems_seed.contains(&Proof::Stat {
            id: CRAFTING_STAT,
            min: 20,
        }));
        assert!(gems_seed.contains(&Proof::BankItemId {
            id: UNCUT_SAPPHIRE_ID,
            count: 28,
        }));
        assert!(gems_seed.contains(&Proof::BankItemId {
            id: CHISEL_ID,
            count: 1,
        }));
        let gems_watch = gems.steps[gems_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(gems_watch.contains(&Proof::ItemId {
            id: SAPPHIRE_ID,
            count: 27,
        }));
        assert!(gems_watch.contains(&Proof::BankItemId {
            id: SAPPHIRE_ID,
            count: 27,
        }));
        assert!(gems_watch.contains(&Proof::ItemId {
            id: CHISEL_ID,
            count: 1,
        }));
        assert!(gems_watch.contains(&Proof::ItemIdAtMost {
            id: CRUSHED_GEMSTONE_ID,
            count: 0,
        }));
        assert_eq!(
            gems.proof,
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 2
            }
        );

        let named_gems = get("gem_cutter_named").expect("gem_cutter_named");
        let inject = settings_inject_map(named_gems.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("gems"),
            Some(&Value::Array(vec![Value::String("Sapphire".into())]))
        );
        let named_gems_start = named_gems
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let named_gems_seed = named_gems.steps[..named_gems_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(named_gems_seed.contains(&Proof::BankItemId {
            id: UNCUT_OPAL_ID,
            count: 4,
        }));
        let named_gems_watch = named_gems.steps[named_gems_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(named_gems_watch.contains(&Proof::BankItemId {
            id: UNCUT_OPAL_ID,
            count: 4,
        }));
        assert!(named_gems_watch.contains(&Proof::ItemIdAtMost {
            id: UNCUT_OPAL_ID,
            count: 0,
        }));

        for name in [
            "dart_fletcher",
            "dart_fletcher_iron",
            "herb_cleaner",
            "herb_cleaner_named",
            "gem_cutter",
            "gem_cutter_named",
        ] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn location_world_cases_register_exact_settings_and_witnesses() {
        let door = get("door_opener").expect("door_opener");
        assert_eq!(door.settings.start_script, Some("DoorOpener"));
        assert_eq!(door.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(door.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("stand"),
            Some(&Value::String("3208,3212,0".into()))
        );
        let door_start = door
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let door_seed = door.steps[..door_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(door_seed.contains(&Proof::LocActionNear {
            id: CLOSED_ID,
            x: 3208,
            z: 3211,
            level: 0,
            radius: 1,
            action: "Open",
            present: true,
        }));
        assert_eq!(
            door.steps[door_start + 1].wait.arm,
            Proof::LocIdNear {
                id: OPEN_ID,
                x: 3208,
                z: 3211,
                level: 0,
                radius: 3,
            }
        );

        let gate = get("door_opener_gate").expect("door_opener_gate");
        let inject = settings_inject_map(gate.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("obstacle"), Some(&Value::String("gate".into())));
        assert_eq!(
            inject.get("stand"),
            Some(&Value::String("3213,3260,0".into()))
        );
        let gate_start = gate
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let gate_seed = gate.steps[..gate_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(gate_seed.contains(&Proof::LocActionNear {
            id: GATE_CLOSED_ID,
            x: 3213,
            z: 3261,
            level: 0,
            radius: 1,
            action: "Open",
            present: true,
        }));
        assert_eq!(
            gate.steps[gate_start + 1].wait.arm,
            Proof::LocIdNear {
                id: GATE_OPEN_ID,
                x: 3213,
                z: 3261,
                level: 0,
                radius: 3,
            }
        );

        let gnome = get("gnome_course").expect("gnome_course");
        assert_eq!(gnome.settings.start_script, Some("GnomeCourse"));
        assert!(gnome.settings.script_settings_inject.is_none());
        let gnome_start = gnome
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let gnome_watch = gnome.steps[gnome_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            gnome_watch[0],
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 1
            }
        );
        assert!(gnome_watch.contains(&Proof::ArrivedNear {
            x: 2474,
            z: 3429,
            level: 0,
            radius: 3,
        }));
        assert!(gnome_watch.contains(&Proof::ArrivedNear {
            x: 2487,
            z: 3420,
            level: 0,
            radius: 3,
        }));
        assert!(gnome_watch.contains(&Proof::ArrivedNear {
            x: 2484,
            z: 3431,
            level: 0,
            radius: 6,
        }));
        assert_eq!(
            gnome.proof,
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 94
            }
        );

        let radius = get("gnome_course_radius").expect("gnome_course_radius");
        let inject = settings_inject_map(radius.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("searchRadius").and_then(Value::as_f64),
            Some(8.0)
        );

        let flax = get("flax_picker").expect("flax_picker");
        assert_eq!(flax.settings.start_script, Some("FlaxPicker"));
        let flax_start = flax
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let flax_seed = flax.steps[..flax_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(flax_seed.contains(&Proof::ItemIdAtMost {
            id: FLAX_ID,
            count: 0,
        }));
        let flax_watch = flax.steps[flax_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(flax_watch.contains(&Proof::ItemId {
            id: FLAX_ID,
            count: 28,
        }));
        assert!(flax_watch.contains(&Proof::BankItemId {
            id: FLAX_ID,
            count: 28,
        }));
        assert!(flax_watch.contains(&Proof::BankClosed));
        assert!(flax_watch.contains(&Proof::ArrivedNear {
            x: 2741,
            z: 3444,
            level: 0,
            radius: 12,
        }));
        assert_eq!(
            flax.proof,
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            }
        );

        for name in [
            "door_opener",
            "door_opener_gate",
            "gnome_course",
            "gnome_course_radius",
            "flax_picker",
        ] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn superheater_cases_register_exact_ids_and_bank_cycles() {
        let bronze = get("superheater").expect("superheater");
        assert_eq!(bronze.settings.start_script, Some("Superheater"));
        assert_eq!(bronze.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(bronze.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("bar"), Some(&Value::String("Bronze".into())));
        let bronze_start = bronze
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(bronze.steps[bronze_start - 1].wait.arm, Proof::BankClosed);
        let bronze_seed = bronze.steps[..bronze_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(bronze_seed.contains(&Proof::Stat {
            id: MAGIC_STAT,
            min: SUPERHEAT_MAGIC,
        }));
        assert!(bronze_seed.contains(&Proof::Stat {
            id: SMITHING_STAT,
            min: BRONZE_SMITHING,
        }));
        assert!(bronze_seed.contains(&Proof::BankItemId {
            id: STAFF_OF_FIRE_ID,
            count: 1,
        }));
        assert!(bronze_seed.contains(&Proof::BankItemId {
            id: NATURE_RUNE_ID,
            count: SUPERHEATER_NATURES_SEED,
        }));
        assert!(bronze_seed.contains(&Proof::BankItemId {
            id: COPPER_ORE_ID,
            count: SUPERHEATER_ORE_SEED,
        }));
        assert!(bronze_seed.contains(&Proof::BankItemId {
            id: TIN_ORE_ID,
            count: SUPERHEATER_ORE_SEED,
        }));
        assert!(bronze_seed.contains(&Proof::ItemIdAtMost {
            id: BRONZE_BAR_ID,
            count: 0,
        }));
        let bronze_watch = bronze.steps[bronze_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            bronze_watch[0],
            Proof::StatXpGain {
                id: MAGIC_STAT,
                min: 1
            }
        );
        assert!(bronze_watch.contains(&Proof::StatXpGain {
            id: SMITHING_STAT,
            min: 1
        }));
        assert!(bronze_watch.contains(&Proof::ItemId {
            id: BRONZE_BAR_ID,
            count: 1,
        }));
        assert!(bronze_watch.contains(&Proof::BankItemId {
            id: BRONZE_BAR_ID,
            count: 1,
        }));
        assert!(bronze_watch.contains(&Proof::ItemId {
            id: COPPER_ORE_ID,
            count: 1,
        }));
        assert!(bronze_watch.contains(&Proof::ItemId {
            id: TIN_ORE_ID,
            count: 1,
        }));
        assert!(bronze_watch.contains(&Proof::ItemIdAtMost {
            id: NATURE_RUNE_ID,
            count: 49,
        }));
        assert!(bronze_watch.contains(&Proof::BankClosed));
        assert_eq!(
            bronze.proof,
            Proof::StatXpGain {
                id: SMITHING_STAT,
                min: 2
            }
        );

        let steel = get("superheater_steel").expect("superheater_steel");
        let inject = settings_inject_map(steel.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("bar"), Some(&Value::String("Steel".into())));
        let steel_start = steel
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let steel_seed = steel.steps[..steel_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(steel_seed.contains(&Proof::Stat {
            id: SMITHING_STAT,
            min: STEEL_SMITHING,
        }));
        assert!(steel_seed.contains(&Proof::BankItemId {
            id: IRON_ORE_ID,
            count: SUPERHEATER_ORE_SEED,
        }));
        assert!(steel_seed.contains(&Proof::BankItemId {
            id: COAL_ID,
            count: SUPERHEATER_COAL_SEED,
        }));
        let steel_watch = steel.steps[steel_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(steel_watch.contains(&Proof::ItemId {
            id: STEEL_BAR_ID,
            count: 1,
        }));
        assert!(steel_watch.contains(&Proof::ItemId {
            id: COAL_ID,
            count: 2,
        }));
        assert!(steel_watch.contains(&Proof::ItemIdAtMost {
            id: IRON_BAR_ID,
            count: 0,
        }));

        let alt = get("superheater_fire_battlestaff").expect("superheater_fire_battlestaff");
        let inject = settings_inject_map(alt.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("bar"), Some(&Value::String("Bronze".into())));
        let alt_start = alt
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(alt.steps[alt_start - 1].wait.arm, Proof::BankClosed);
        let alt_seed = alt.steps[..alt_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(alt_seed.contains(&Proof::BankItemId {
            id: FIRE_BATTLESTAFF_ID,
            count: 1,
        }));
        assert!(alt_seed.contains(&Proof::BankItemIdAtMost {
            id: STAFF_OF_FIRE_ID,
            count: 0,
        }));
        let alt_watch = alt.steps[alt_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(alt_watch.contains(&Proof::ItemId {
            id: BRONZE_BAR_ID,
            count: 1,
        }));
        assert!(alt_watch.contains(&Proof::ItemIdAtMost {
            id: STAFF_OF_FIRE_ID,
            count: 0,
        }));

        for name in [
            "superheater",
            "superheater_steel",
            "superheater_fire_battlestaff",
        ] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn chicken_killer_bank_registers_loot_count_feather_trip() {
        let bank = get("chicken_killer_bank").expect("chicken_killer_bank");
        assert_eq!(bank.settings.start_script, Some("ChickenKiller"));
        assert_eq!(bank.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(bank.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("bankStrategy"),
            Some(&Value::String("Loot count".into()))
        );
        assert_eq!(inject.get("bankEveryItems"), Some(&Value::from(1.0)));
        assert_eq!(
            inject.get("lootMatch"),
            Some(&Value::String("feather".into()))
        );
        assert_eq!(
            inject.get("combatStyle"),
            Some(&Value::String("melee".into()))
        );
        assert!(inject.get("buryBones").is_none());
        let start = bank
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = bank.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 3029,
            z: 3294,
            level: 0,
            radius: 8,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: FEATHER_ID,
            count: 0,
        }));
        let watch = bank.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            watch,
            vec![
                Proof::StatXpGain {
                    id: STRENGTH_STAT,
                    min: 1
                },
                Proof::ItemId {
                    id: FEATHER_ID,
                    count: 1
                },
                Proof::BankItemId {
                    id: FEATHER_ID,
                    count: 1
                },
                Proof::ItemIdAtMost {
                    id: FEATHER_ID,
                    count: 0
                },
                Proof::ArrivedNear {
                    x: 3029,
                    z: 3294,
                    level: 0,
                    radius: 6,
                },
                Proof::BankClosed,
                Proof::ItemId {
                    id: FEATHER_ID,
                    count: 1
                },
            ]
        );
        assert_eq!(
            bank.proof,
            Proof::ItemId {
                id: FEATHER_ID,
                count: 1
            }
        );
        let core = get("chicken_killer").unwrap();
        assert!(
            core.settings
                .script_settings_inject
                .as_ref()
                .map(|rows| !rows.iter().any(|row| row.id == "bankStrategy"))
                .unwrap_or(true),
            "default chicken_killer banking stays off"
        );
        assert!(names().contains(&"chicken_killer_bank"));
    }

    #[test]
    fn vial_filler_cases_register_fountain_fill_and_bank_cycles() {
        let west = get("vial_filler").expect("vial_filler");
        assert_eq!(west.settings.start_script, Some("VialFiller"));
        assert_eq!(west.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(west.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("bank"),
            Some(&Value::String("Falador West".into()))
        );
        assert_eq!(inject.get("buyVials"), Some(&Value::Bool(false)));
        let start = west
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(west.steps[start - 1].wait.arm, Proof::BankClosed);
        let seed = west.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2946,
            z: 3369,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: EMPTY_VIAL_ID,
            count: VIAL_EMPTY_SEED,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: VIAL_OF_WATER_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemIdAtMost {
            id: VIAL_OF_WATER_ID,
            count: 0,
        }));
        let watch = west.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let fountain = Proof::ArrivedNear {
            x: 2949,
            z: 3381,
            level: 0,
            radius: 4,
        };
        let water = Proof::ItemId {
            id: VIAL_OF_WATER_ID,
            count: 1,
        };
        let pack_empty_water = Proof::ItemIdAtMost {
            id: VIAL_OF_WATER_ID,
            count: 0,
        };
        assert_eq!(
            watch,
            vec![
                fountain,
                water,
                Proof::ItemIdAtMost {
                    id: EMPTY_VIAL_ID,
                    count: 0,
                },
                Proof::BankItemId {
                    id: VIAL_OF_WATER_ID,
                    count: 1,
                },
                pack_empty_water,
                Proof::ItemId {
                    id: EMPTY_VIAL_ID,
                    count: 1,
                },
                Proof::BankClosed,
                fountain,
                water,
            ]
        );
        let first_water = watch.iter().position(|arm| *arm == water).unwrap();
        let bank_water = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: VIAL_OF_WATER_ID,
                    count: 1,
                }
            })
            .unwrap();
        let pack_empty = watch
            .iter()
            .position(|arm| *arm == pack_empty_water)
            .unwrap();
        let further_water = watch.iter().rposition(|arm| *arm == water).unwrap();
        assert!(first_water < bank_water);
        assert!(bank_water < pack_empty);
        assert!(pack_empty < further_water);
        assert_ne!(first_water, further_water);
        assert_eq!(west.proof, water);

        let east = get("vial_filler_east").expect("vial_filler_east");
        let inject = settings_inject_map(east.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("bank"),
            Some(&Value::String("Falador East".into()))
        );
        assert_eq!(inject.get("buyVials"), Some(&Value::Bool(false)));
        let east_start = east
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let east_seed = east.steps[..east_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(east_seed.contains(&Proof::ArrivedNear {
            x: 3013,
            z: 3355,
            level: 0,
            radius: 6,
        }));
        let east_watch = east.steps[east_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(east_watch, watch);

        for name in ["vial_filler", "vial_filler_east"] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn potion_maker_cases_register_staged_unf_finished_and_bank_cycles() {
        let custom = get("potion_maker").expect("potion_maker");
        assert_eq!(custom.settings.start_script, Some("PotionMaker"));
        assert_eq!(custom.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(custom.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("herb"), Some(&Value::String("Custom".into())));
        assert_eq!(
            inject.get("herbCustom"),
            Some(&Value::String("Guam leaf".into()))
        );
        assert_eq!(
            inject.get("secondary"),
            Some(&Value::String("Custom".into()))
        );
        assert_eq!(
            inject.get("secondaryCustom"),
            Some(&Value::String("Eye of newt".into()))
        );
        let start = custom
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(custom.steps[start - 1].wait.arm, Proof::BankClosed);
        let seed = custom.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::QuestDone {
            name: "Druidic Ritual",
        }));
        assert!(seed.contains(&Proof::Stat {
            id: HERBLORE_STAT,
            min: GUAM_HERBLORE,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: GUAM_LEAF_ID,
            count: POTION_BATCH_SEED,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: VIAL_OF_WATER_ID,
            count: POTION_BATCH_SEED,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: EYE_OF_NEWT_ID,
            count: POTION_BATCH_SEED,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: GUAM_UNF_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: ATTACK_POTION_3_ID,
            count: 0,
        }));
        let watch = custom.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            watch[0],
            Proof::ItemId {
                id: GUAM_UNF_ID,
                count: 1,
            }
        );
        assert!(watch.contains(&Proof::ItemId {
            id: ATTACK_POTION_3_ID,
            count: 1,
        }));
        assert!(watch.contains(&Proof::ItemIdAtMost {
            id: GUAM_UNF_ID,
            count: 0,
        }));
        assert!(watch.contains(&Proof::StatXpGain {
            id: HERBLORE_STAT,
            min: 1,
        }));
        assert!(watch.contains(&Proof::BankItemId {
            id: ATTACK_POTION_3_ID,
            count: 1,
        }));
        assert!(watch.contains(&Proof::ItemId {
            id: VIAL_OF_WATER_ID,
            count: 1,
        }));
        assert!(watch.contains(&Proof::BankClosed));
        assert_eq!(
            custom.proof,
            Proof::ItemId {
                id: ATTACK_POTION_3_ID,
                count: 1,
            }
        );
        let unf_idx = watch
            .iter()
            .position(|arm| {
                *arm == Proof::ItemId {
                    id: GUAM_UNF_ID,
                    count: 1,
                }
            })
            .unwrap();
        let finished_idx = watch
            .iter()
            .position(|arm| {
                *arm == Proof::ItemId {
                    id: ATTACK_POTION_3_ID,
                    count: 1,
                }
            })
            .unwrap();
        let restock_empty = watch
            .iter()
            .position(|arm| {
                *arm == Proof::ItemIdAtMost {
                    id: ATTACK_POTION_3_ID,
                    count: 0,
                }
            })
            .unwrap();
        assert!(unf_idx < finished_idx);
        assert!(finished_idx < restock_empty);
        assert!(restock_empty < watch.len() - 1);

        let named = get("potion_maker_named").expect("potion_maker_named");
        let inject = settings_inject_map(named.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("herb"),
            Some(&Value::String("Ranarr weed".into()))
        );
        assert_eq!(
            inject.get("secondary"),
            Some(&Value::String("Snape grass".into()))
        );
        let named_start = named
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let named_seed = named.steps[..named_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(named_seed.contains(&Proof::Stat {
            id: HERBLORE_STAT,
            min: RANARR_HERBLORE,
        }));
        assert!(named_seed.contains(&Proof::BankItemId {
            id: RANARR_WEED_ID,
            count: POTION_BATCH_SEED,
        }));
        assert!(named_seed.contains(&Proof::BankItemId {
            id: SNAPE_GRASS_ID,
            count: POTION_BATCH_SEED,
        }));
        assert!(named_seed.contains(&Proof::BankItemId {
            id: GUAM_LEAF_ID,
            count: 14,
        }));
        let named_watch = named.steps[named_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(named_watch.contains(&Proof::ItemId {
            id: RANARR_UNF_ID,
            count: 1,
        }));
        assert!(named_watch.contains(&Proof::ItemId {
            id: PRAYER_POTION_3_ID,
            count: 1,
        }));
        assert!(named_watch.contains(&Proof::ItemIdAtMost {
            id: GUAM_UNF_ID,
            count: 0,
        }));
        assert!(named_watch.contains(&Proof::ItemIdAtMost {
            id: ATTACK_POTION_3_ID,
            count: 0,
        }));
        assert!(named_watch.contains(&Proof::BankItemId {
            id: GUAM_LEAF_ID,
            count: 14,
        }));

        for name in ["potion_maker", "potion_maker_named"] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn tanner_bot_cases_register_conversion_and_bank_cycles() {
        let soft = get("tanner_bot").expect("tanner_bot");
        assert_eq!(soft.settings.start_script, Some("TannerBot"));
        assert_eq!(soft.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(soft.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("hideType"),
            Some(&Value::String("Soft leather".into()))
        );
        assert_eq!(inject.get("buyThread"), Some(&Value::Bool(false)));
        let start = soft
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(soft.steps[start - 1].wait.arm, Proof::BankClosed);
        let seed = soft.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 3269,
            z: 3167,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: COW_HIDE_ID,
            count: TANNER_HIDE_SEED,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: COINS_ID,
            count: TANNER_COIN_SEED,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: SOFT_LEATHER_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemIdAtMost {
            id: SOFT_LEATHER_ID,
            count: 0,
        }));
        let watch = soft.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let tanner = Proof::ArrivedNear {
            x: 3277,
            z: 3191,
            level: 0,
            radius: 4,
        };
        let leather = Proof::ItemId {
            id: SOFT_LEATHER_ID,
            count: 1,
        };
        let pack_empty_leather = Proof::ItemIdAtMost {
            id: SOFT_LEATHER_ID,
            count: 0,
        };
        assert_eq!(
            watch,
            vec![
                tanner,
                leather,
                Proof::ItemIdAtMost {
                    id: COW_HIDE_ID,
                    count: 0,
                },
                Proof::BankItemId {
                    id: SOFT_LEATHER_ID,
                    count: 1,
                },
                pack_empty_leather,
                Proof::ItemId {
                    id: COW_HIDE_ID,
                    count: 1,
                },
                Proof::BankClosed,
                tanner,
                leather,
            ]
        );
        let first_leather = watch.iter().position(|arm| *arm == leather).unwrap();
        let bank_leather = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: SOFT_LEATHER_ID,
                    count: 1,
                }
            })
            .unwrap();
        let pack_empty = watch
            .iter()
            .position(|arm| *arm == pack_empty_leather)
            .unwrap();
        let further_leather = watch.iter().rposition(|arm| *arm == leather).unwrap();
        assert!(first_leather < bank_leather);
        assert!(bank_leather < pack_empty);
        assert!(pack_empty < further_leather);
        assert_ne!(first_leather, further_leather);
        assert_eq!(soft.proof, leather);

        let hard = get("tanner_bot_hard").expect("tanner_bot_hard");
        let inject = settings_inject_map(hard.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("hideType"),
            Some(&Value::String("Hard leather".into()))
        );
        assert_eq!(inject.get("buyThread"), Some(&Value::Bool(false)));
        let hard_start = hard
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let hard_watch = hard.steps[hard_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let hard_leather = Proof::ItemId {
            id: HARD_LEATHER_ID,
            count: 1,
        };
        assert_eq!(hard_watch[1], hard_leather);
        assert!(hard_watch.contains(&Proof::BankItemId {
            id: HARD_LEATHER_ID,
            count: 1,
        }));
        assert!(hard_watch.contains(&Proof::ItemIdAtMost {
            id: HARD_LEATHER_ID,
            count: 0,
        }));
        assert!(!hard_watch.contains(&leather));
        assert_eq!(hard.proof, hard_leather);

        for name in ["tanner_bot", "tanner_bot_hard"] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn bone_burier_requires_banking_between_burial_cycles() {
        let s = get("bone_burier").unwrap();
        assert_eq!(s.settings.start_script, Some("BoneBurier"));
        let start = s
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(s.steps[..start]
            .iter()
            .any(|step| matches!(step.kind, StepKind::Relog)));
        let arms: Vec<_> = s.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect();
        assert_eq!(
            arms,
            vec![
                Proof::StatXpGain { id: 5, min: 22 },
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0
                },
                Proof::BankItem {
                    name: "Bones",
                    count: 28
                },
                Proof::Item {
                    name: "Bones",
                    count: 28
                },
                Proof::BankItemAtMost {
                    name: "Bones",
                    count: 0
                },
                Proof::BankClosed,
                Proof::StatXpGain { id: 5, min: 27 },
            ]
        );
        assert_eq!(s.proof, Proof::StatXpGain { id: 5, min: 27 });
    }

    #[test]
    fn script_gold_watch_is_a_short_agentic_budget() {
        for name in [
            "bone_burier",
            "chicken_killer",
            "chicken_killer_bank",
            "thiever",
            "alcher",
            "alcher_custom_alias",
            "alcher_custom_name",
            "bank_fletcher",
            "bank_fletcher_string",
            "bank_fletcher_cut_string",
            "dart_fletcher",
            "dart_fletcher_iron",
            "herb_cleaner",
            "herb_cleaner_named",
            "gem_cutter",
            "gem_cutter_named",
            "door_opener",
            "door_opener_gate",
            "gnome_course",
            "gnome_course_radius",
            "flax_picker",
            "superheater",
            "superheater_steel",
            "superheater_fire_battlestaff",
            "vial_filler",
            "vial_filler_east",
            "potion_maker",
            "potion_maker_named",
            "tanner_bot",
            "tanner_bot_hard",
            "script_trade",
        ] {
            let s = get(name).unwrap_or_else(|| panic!("{name} registered"));
            assert_eq!(s.settings.deadline, SCRIPT_GOLD_DEADLINE, "{name} deadline");
            let start = s
                .steps
                .iter()
                .position(|st| matches!(st.kind, StepKind::StartScript))
                .unwrap_or_else(|| panic!("{name} StartScript"));
            let watch = &s.steps[start + 1];
            assert_eq!(
                watch.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS,
                "{name} watch ticks"
            );
            assert!(
                watch.name.starts_with("watch "),
                "{name} step after Start is the watch, got {}",
                watch.name
            );
        }
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
        assert_eq!(d.start_script, None);
        assert_eq!(d.script_settings_inject, None);
        assert_eq!(d.inject_companion_as, None);
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
